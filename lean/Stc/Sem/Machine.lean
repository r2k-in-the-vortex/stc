/-
The small-step source machine for the core subset (P0.3).

A configuration is the persistent global root map, a stack of call frames, and
the current focus plus the continuation stack of the innermost frame. Frames
carry their own continuation stacks, so a call suspends the caller without
copying. The control stack is a list of `Cont` values; loops, `IF`, `CASE`,
`FOR`, calls, and expression evaluation each have an explicit continuation.

`step` is a total function from configurations to an `Outcome`: it either
steps to a configuration, reports that the configuration is terminal, or gets
stuck. Well-typed configurations never get stuck; `progress` in
`Stc.Sem.Eval` proves that. The machine is deterministic by construction, which
is the content of `eval_deterministic`.

The machine carries no typing annotations: continuations store the AST nodes
and runtime values they resume, and all static facts live in the inductive
configuration typing `Stc.Sem.StackTyped`/`ConfigTyped`. Ill-typed
configurations may get stuck; typing rules them out.
-/
import Stc.Sem.Values

namespace Stc.Sem

open Stc.Hir
open Stc.Syntax

/-! ## Loop context and continuation expectations -/

/-- The lexical loop context of a statement list: the enclosing loop depth and
the active `FOR` control variables. Statement typing depends only on these. -/
structure LoopCtx where
  /-- The number of enclosing loops. -/
  depth : Nat
  /-- Active `FOR` control variables, innermost first. -/
  controlled : List VarId
deriving Repr, DecidableEq

/-- The top-level loop context of a frame body. -/
def LoopCtx.top : LoopCtx := ⟨0, []⟩

/-- The loop context of a loop body. -/
def LoopCtx.body (lc : LoopCtx) (vars : List VarId) : LoopCtx := ⟨lc.depth + 1, vars⟩

/-- What a continuation expects from the current focus. -/
inductive Expect where
  /-- A value of the given type. -/
  | value (ty : Ty)
  /-- Completion of the current statement. -/
  | unit
deriving Repr, DecidableEq

/-- Whether the expectation is a value. -/
def Expect.isValue : Expect → Bool
  | .value _ => true
  | .unit => false

/-! ## Continuations -/

/-- The continuation stack of one frame. Each constructor stores the AST it
resumes; the static types live in `Stc.Sem.StackTyped`. -/
inductive ContKind where
  /-- Store the value into a place, then complete the assignment. -/
  | assign (place : Place)
  /-- Apply a unary operation. -/
  | unary (op : UnaryOp)
  /-- Evaluate the right operand of a binary operation. -/
  | binaryL (op : BinaryOp) (rhs : Expr)
  /-- Apply a binary operation to the evaluated left operand. -/
  | binaryR (op : BinaryOp) (lhs : Value)
  /-- Apply a conversion. -/
  | cast (to : Ty)
  /-- Evaluate the remaining arguments of a call, then perform it. -/
  | callArgs (callee : Callee) (done : List Value) (remaining : List Expr)
      (isStmt : Bool)
  /-- Choose an `IF` arm from the condition value. -/
  | ifCond (arms : List IfArm) (elseBody : List Stmt)
  /-- Choose a `CASE` arm from the selector value. -/
  | caseSel (selector : Expr) (arms : List CaseArm) (elseBody : List Stmt)
  /-- Continue a `WHILE` after its condition. -/
  | whileCond (cond : Expr) (body : List Stmt)
  /-- Re-evaluate a `WHILE` condition after its body. -/
  | whileLoop (cond : Expr) (body : List Stmt)
  /-- Evaluate a `REPEAT` condition after its body. -/
  | repeatBody (body : List Stmt) (cond : Expr)
  /-- Continue a `REPEAT` after its condition. -/
  | repeatCond (body : List Stmt) (cond : Expr)
  /-- Continue a `FOR` after the start value, evaluating the stop bound. -/
  | forInit (var : Place) (stop : Expr) (step : Option Expr) (body : List Stmt)
  /-- Continue a `FOR` after the stop bound, evaluating the step. -/
  | forStop (var : Place) (step : Option Expr) (body : List Stmt)
  /-- Continue a `FOR` after the step, checking the first iteration. -/
  | forStep (var : Place) (stopVal : Value) (body : List Stmt)
  /-- Advance a `FOR` after its body. -/
  | forBody (var : Place) (stopVal : Value) (stepVal : Value) (body : List Stmt)
  /-- Continue the statement list after a statement. -/
  | seq (rest : List Stmt)

/-- A continuation with the lexical loop contexts before and after it. Loop
markers raise the depth (`before = after + 1`); all other continuations keep
it. -/
structure Cont where
  /-- The continuation. -/
  kind : ContKind
  /-- The loop context of the focus this continuation consumes. -/
  before : LoopCtx
  /-- The loop context of the focus that follows. -/
  after : LoopCtx
  /-- The source span for traps raised here. -/
  span : Span

/-- A continuation that keeps the loop context. -/
def Cont.same (kind : ContKind) (lc : LoopCtx) (span : Span) : Cont :=
  { kind := kind, before := lc, after := lc, span := span }

namespace ContKind

/-- Whether the continuation consumes a value. -/
def wantsValue : ContKind → Bool
  | .assign .. => true
  | .unary .. => true
  | .binaryL .. => true
  | .binaryR .. => true
  | .cast .. => true
  | .callArgs .. => true
  | .ifCond .. => true
  | .caseSel .. => true
  | .whileCond .. => true
  | .repeatCond .. => true
  | .forInit .. => true
  | .forStop .. => true
  | .forStep .. => true
  | .whileLoop .. => false
  | .repeatBody .. => false
  | .forBody .. => false
  | .seq .. => false

/-- Whether the continuation is a loop marker. -/
def isLoopMarker : ContKind → Bool
  | .whileLoop .. => true
  | .repeatBody .. => true
  | .forBody .. => true
  | _ => false

end ContKind

namespace Cont

/-- Whether the continuation consumes a value. -/
def wantsValue (c : Cont) : Bool := c.kind.wantsValue

/-- Whether the continuation is a loop marker. -/
def isLoopMarker (c : Cont) : Bool := c.kind.isLoopMarker

end Cont

/-! ## Frames, focus, and configurations -/

/-- The frame's declaring POU. -/
inductive FrameKind where
  /-- A function, by index into the unit's function list. -/
  | function (index : Nat)
  /-- The single program. -/
  | program
deriving Repr, DecidableEq

/-- One call frame. The head frame is the running one; later frames are
suspended callers. `expect` is the expectation of the frame's continuation
stack while the frame is suspended by a call. -/
structure Frame where
  /-- The declaring POU. -/
  kind : FrameKind
  /-- Parameter values. -/
  params : List Value
  /-- Local values. -/
  locals : List Value
  /-- The current value of the result variable, when the POU has one. -/
  result : Option Value
  /-- The continuation stack. -/
  k : List Cont
  /-- The expectation of `k` while the frame is suspended. -/
  expect : Option Ty
  /-- The loop context of the current focus. -/
  loopCtx : LoopCtx

/-- The current focus of the running frame. -/
inductive Focus where
  /-- Evaluate an expression. -/
  | expr (e : Expr)
  /-- An evaluated value awaiting its continuation. -/
  | value (v : Value)
  /-- Execute a statement list at a loop context. -/
  | stmts (ss : List Stmt) (lc : LoopCtx)
  /-- A statement completed; resume the enclosing continuation. -/
  | resume
  /-- A callee returned an optional value to the caller. -/
  | returned (rv : Option Value)
  /-- A trap terminated the run. -/
  | trapped (trap : Trap) (span : Span)
  /-- The requested cycles completed. -/
  | halted

/-- A machine configuration. -/
structure State where
  /-- Persistent global roots. -/
  globals : List Value
  /-- The call stack; the head frame runs. -/
  frames : List Frame
  /-- The focus of the running frame. -/
  focus : Focus
  /-- Completed cycles. -/
  cycles : Nat
  /-- Remaining cycles, including the current one. -/
  cyclesLeft : Nat

/-- The result of one step. -/
inductive Outcome where
  /-- The configuration steps to a successor. -/
  | step (c : State)
  /-- The configuration is terminal. -/
  | halt
  /-- The configuration is ill-typed or otherwise stuck. -/
  | stuck

/-! ## Pure helpers -/

/-- Replace the `n`th element of a list. -/
def List.set? : List α → Nat → α → Option (List α)
  | [], _, _ => none
  | _ :: xs, 0, v => some (v :: xs)
  | x :: xs, n + 1, v => (List.set? xs n v).map (fun xs' => x :: xs')

/-- The current frame, if any. -/
def State.head? (s : State) : Option Frame := s.frames.head?

/-- The running frame's continuation stack. -/
def State.k (s : State) : List Cont := s.frames.head?.map Frame.k |>.getD []

/-- Look up a resolved variable. -/
def State.lookupVar? (s : State) : VarId → Option Value
  | .param i => s.head?.bind (fun f => f.params[i]?)
  | .local i => s.head?.bind (fun f => f.locals[i]?)
  | .global i => s.globals[i]?
  | .result => s.head?.bind (fun f => f.result)

/-- Write a resolved variable. The write is total; an out-of-range index
leaves the store unchanged, which is unreachable from a well-typed program
because the typing context bounds every variable. -/
def State.setVar (s : State) (id : VarId) (v : Value) : State :=
  match s.frames with
  | [] => s
  | f :: rest =>
      match id with
      | .param i => { s with frames := { f with params := f.params.set i v } :: rest }
      | .local i => { s with frames := { f with locals := f.locals.set i v } :: rest }
      | .global i => { s with globals := s.globals.set i v }
      | .result => { s with frames := { f with result := some v } :: rest }

/-- Replace the running frame. -/
def State.updateHead (s : State) (f : Frame) : State :=
  match s.frames with
  | [] => s
  | _ :: rest => { s with frames := f :: rest }

/-- Push a continuation onto the running frame. -/
def State.pushCont (s : State) (cont : Cont) : State :=
  let f := s.head?.getD ⟨.program, [], [], none, [], none, LoopCtx.top⟩
  s.updateHead { f with k := cont :: f.k }

/-- The declaration-derived typing context of a frame. -/
def frameCtx (u : HirUnit) : FrameKind → Ctx
  | .function i =>
      match u.functions[i]? with
      | some fn => Ctx.ofFunction (Env.ofUnit u) fn
      | none =>
          { globals := [], functions := [], params := [], locals := [], result := none }
  | .program => Ctx.ofProgram (Env.ofUnit u) u.program

/-- Find the innermost loop marker and the stack below it. -/
def findLoop : List Cont → Option (Cont × List Cont)
  | [] => none
  | c :: rest => if c.isLoopMarker then some (c, rest) else findLoop rest

/-- Whether a `CASE` label contains a value. -/
def caseLabelContains (label : CaseLabel) (value : Int) : Bool :=
  let (lo, hi) := label.bounds
  decide (lo ≤ value) && decide (value ≤ hi)

/-- The body of the first `CASE` arm whose labels contain the value. -/
def caseArmBody? : List CaseArm → Int → Option (List Stmt)
  | [], _ => none
  | arm :: rest, value =>
      if arm.labels.any (fun label => caseLabelContains label value) then some arm.body
      else caseArmBody? rest value

/-- Statements that initialize a list of locals, starting at `index`. -/
def localInitStmtsFrom (index : Nat) : List Local → List Stmt
  | [] => []
  | loc :: rest =>
      (match loc.init with
        | some e => [Stmt.assign ⟨.local index, loc.span⟩ e loc.span]
        | none => []) ++ localInitStmtsFrom (index + 1) rest

/-- Statements that initialize a list of locals. -/
def localInitStmts (locals : List Local) : List Stmt :=
  localInitStmtsFrom 0 locals

/-- Statements that initialize a list of global roots, starting at `index`. -/
def globalInitStmtsFrom (index : Nat) : List Global → List Stmt
  | [] => []
  | glob :: rest =>
      (match glob.init with
        | some e => [Stmt.assign ⟨.global index, glob.span⟩ e glob.span]
        | none => []) ++ globalInitStmtsFrom (index + 1) rest

/-- Statements that initialize global roots. -/
def globalInitStmts (globals : List Global) : List Stmt :=
  globalInitStmtsFrom 0 globals

/-- The initialization sequence followed by the first cycle body. -/
def initBody (u : HirUnit) : List Stmt :=
  globalInitStmts u.globals ++ localInitStmts u.program.locals ++ u.program.body

/-- The initialization boundary: default global roots, a program frame with
default fields, and the initialization sequence followed by the first cycle.
A run with zero cycles is terminal immediately. -/
def initState (u : HirUnit) (cycles : Nat) : State :=
  { globals := u.globals.map (fun g => Value.default g.ty)
    frames := [{ kind := .program, params := [], locals := u.program.locals.map (fun l => Value.default l.ty), result := none, k := [], expect := none, loopCtx := LoopCtx.top }]
    focus := if cycles = 0 then .halted else .stmts (initBody u) LoopCtx.top
    cycles := 0
    cyclesLeft := cycles }

/-! ## The step function -/

mutual

/-- Complete the running frame: a function returns its result to the caller, a
program ends its cycle. -/
def completeFrame (u : HirUnit) (c : State) : Outcome :=
  match c.frames with
  | [] => .stuck
  | f :: rest =>
      match f.kind with
      | .function _ =>
          match rest with
          | [] => .stuck
          | _ :: _ => .step { c with frames := rest, focus := .returned f.result }
      | .program =>
          if c.cyclesLeft ≤ 1 then
            .step { c with cycles := c.cycles + 1, cyclesLeft := 0, focus := .halted }
          else
            .step { c with
              cycles := c.cycles + 1
              cyclesLeft := c.cyclesLeft - 1
              frames := { f with k := [], expect := none, loopCtx := LoopCtx.top } :: rest
              focus := .stmts u.program.body LoopCtx.top }

/-- Whether a `FOR` loop continues after its bound is evaluated. -/
def forKeep (current stopVal stepVal : Value) : Bool :=
  if stepVal.ty.layout.signed then
    if 0 < stepVal.value then decide (current.value ≤ stopVal.value)
    else decide (stopVal.value ≤ current.value)
  else decide (current.value ≤ stopVal.value)

/-- Push a `FOR` body or complete the loop, depending on the bound check. -/
def forContinue (u : HirUnit) (c : State) (cont : Cont) (rest : List Cont)
    (var : Place) (stopVal stepVal : Value) (body : List Stmt) : Outcome :=
  let current := (c.lookupVar? var.id).getD stopVal
  let keep := forKeep current stopVal stepVal
  let bodyLc := LoopCtx.body cont.after (var.id :: cont.after.controlled)
  if keep then
    .step { c with
      focus := .stmts body bodyLc
      frames := { c.head?.getD ⟨.program, [], [], none, [], none, LoopCtx.top⟩ with
        k := { kind := .forBody var stopVal stepVal body, before := bodyLc, after := cont.after, span := cont.span } :: rest
        loopCtx := bodyLc } :: c.frames.tail }
  else
    .step { c with
      focus := .resume
      frames := { c.head?.getD ⟨.program, [], [], none, [], none, LoopCtx.top⟩ with
        k := rest, loopCtx := cont.after } :: c.frames.tail }

/-- Increment a `FOR` control variable and continue or finish the loop. -/
def forAdvance (u : HirUnit) (c : State) (cont : Cont) (rest : List Cont)
    (var : Place) (stopVal stepVal : Value) (body : List Stmt) : Outcome :=
  let current := (c.lookupVar? var.id).getD stepVal
  match current.ty.layout.add current.value stepVal.value with
  | .error trap => .step { c with focus := .trapped trap cont.span }
  | .ok next => forContinue u (c.setVar var.id (Value.ofInt current.ty next)) cont rest var stopVal stepVal body

/-- Perform a call after its arguments are evaluated. -/
def performCall (u : HirUnit) (c : State) (callee : Callee) (args : List Value)
    (isStmt : Bool) (span : Span) : Outcome :=
  match callee with
  | .std fn _ =>
      match evalStd fn args with
      | .error trap => .step { c with focus := .trapped trap span }
      | .ok v =>
          if isStmt then .step { c with focus := .resume }
          else .step { c with focus := .value v }
  | .function index _ =>
      match u.functions[index]? with
      | none => .stuck
      | some fn =>
          match c.frames with
          | [] => .stuck
          | caller :: rest =>
              let calleeFrame : Frame :=
                { kind := .function index, params := args, locals := fn.locals.map (fun l => Value.default l.ty), result := fn.resultType.map Value.default, k := [], expect := none, loopCtx := LoopCtx.top }
              .step { c with
                -- A statement call discards the result, so the suspended caller's
                -- expectation is `unit`; an expression call expects the result.
                frames := calleeFrame ::
                  { caller with expect := if isStmt then none else fn.resultType } :: rest
                focus := .stmts (localInitStmts fn.locals ++ fn.body) LoopCtx.top }

/-- Evaluate an expression. -/
def stepExpr (u : HirUnit) (c : State) (e : Expr) : Outcome :=
  match c.frames with
  | [] => .stuck
  | f :: fr =>
      match e with
      | .literal lit _ => .step { c with focus := .value ⟨lit.ty, lit.value⟩ }
      | .var id _ =>
          match c.lookupVar? id with
          | some v => .step { c with focus := .value v }
          | none => .stuck
      | .unary op operand span =>
          .step { c with
            focus := .expr operand
            frames := { f with k := Cont.same (.unary op) f.loopCtx span :: f.k } :: fr }
      | .binary op lhs rhs span =>
          .step { c with
            focus := .expr lhs
            frames := { f with k := Cont.same (.binaryL op rhs) f.loopCtx span :: f.k } :: fr }
      | .call callee args span =>
          match args with
          | [] => performCall u c callee [] false span
          | arg :: _ =>
              .step { c with
                focus := .expr arg
                frames := { f with k := Cont.same (.callArgs callee [] args false) f.loopCtx span :: f.k } :: fr }
      | .cast operand to _ span =>
          .step { c with
            focus := .expr operand
            frames := { f with k := Cont.same (.cast to) f.loopCtx span :: f.k } :: fr }

/-- Execute a statement list. -/
def stepStmts (u : HirUnit) (c : State) (ss : List Stmt) (lc : LoopCtx) : Outcome :=
  match ss with
  | [] => .step { c with focus := .resume }
  | s :: rest => stepStmt u c s rest lc

/-- Execute one statement. -/
def stepStmt (u : HirUnit) (c : State) (s : Stmt) (rest : List Stmt)
    (lc : LoopCtx) : Outcome :=
  match c.frames with
  | [] => .stuck
  | f :: fr =>
      let seqCont : Cont := Cont.same (.seq rest) lc s.span
      match s with
      | .assign place value span =>
          .step { c with
            focus := .expr value
            frames := { f with k := Cont.same (.assign place) lc span :: seqCont :: f.k } :: fr }
      | .call call callSpan =>
          match call with
          | .call callee args innerSpan =>
              match args with
              | [] =>
                  performCall u (c.updateHead { f with k := seqCont :: f.k })
                    callee [] true innerSpan
              | arg :: _ =>
                  .step { c with
                    focus := .expr arg
                    frames := { f with k := Cont.same (.callArgs callee [] args true) lc innerSpan :: seqCont :: f.k } :: fr }
          | _ => .stuck
      | .return_ _ => completeFrame u c
      | .exit _ =>
          match findLoop f.k with
          | none => .stuck
          -- Exiting a loop resumes in the loop context the marker closes over,
          -- not the body's loop context.
          | some (marker, below) =>
              .step { c with focus := .resume, frames := { f with k := below, loopCtx := marker.after } :: fr }
      | .continue_ _ =>
          match findLoop f.k with
          | none => .stuck
          | some (marker, below) =>
              match marker.kind with
              | .whileLoop cond body =>
                  .step { c with
                    focus := .expr cond
                    frames := { f with k := Cont.same (.whileCond cond body) marker.after marker.span :: below, loopCtx := marker.after } :: fr }
              | .repeatBody body cond =>
                  .step { c with
                    focus := .expr cond
                    frames := { f with k := Cont.same (.repeatCond body cond) marker.after marker.span :: below, loopCtx := marker.after } :: fr }
              | .forBody var stopVal stepVal body =>
                  forAdvance u { c with frames := { f with k := below } :: fr } marker below var stopVal stepVal body
              | _ => .stuck
      | .empty _ => .step { c with focus := .stmts rest lc }
      | .ifThenElse arms elseBody span =>
          match arms with
          | [] => .step { c with focus := .stmts elseBody lc, frames := { f with k := seqCont :: f.k } :: fr }
          | arm :: _ =>
              .step { c with
                focus := .expr arm.cond
                frames := { f with k := Cont.same (.ifCond arms elseBody) lc span :: seqCont :: f.k } :: fr }
      | .case selector arms elseBody span =>
          .step { c with
            focus := .expr selector
            frames := { f with k := Cont.same (.caseSel selector arms elseBody) lc span :: seqCont :: f.k } :: fr }
      | .forLoop var start stop stepE body span =>
          .step { c with
            focus := .expr start
            frames := { f with k := Cont.same (.forInit var stop stepE body) lc span :: seqCont :: f.k } :: fr }
      | .whileLoop cond body span =>
          .step { c with
            focus := .expr cond
            frames := { f with k := Cont.same (.whileCond cond body) lc span :: seqCont :: f.k } :: fr }
      | .repeatLoop body cond span =>
          let bodyLc := LoopCtx.body lc lc.controlled
          .step { c with
            focus := .stmts body bodyLc
            frames := { f with
              k := { kind := .repeatBody body cond, before := bodyLc, after := lc, span := span } :: seqCont :: f.k
              loopCtx := bodyLc } :: fr }

/-- Apply the current continuation to a value. -/
def stepValue (u : HirUnit) (c : State) (v : Value) : Outcome :=
  match c.frames with
  | [] => .stuck
  | f :: fr =>
      match f.k with
      | [] => .stuck
      | cont :: rest =>
          match cont.kind with
          | .assign place =>
              -- Pop the assignment continuation, then write the value.
              .step { (c.updateHead { f with k := rest }).setVar place.id v with
                focus := .resume }
          | .unary op =>
              match evalUnary op v.ty v.value with
              | .error trap => .step { c with focus := .trapped trap cont.span }
              | .ok w => .step { c with focus := .value w, frames := { f with k := rest } :: fr }
          | .binaryL op rhs =>
              .step { c with
                focus := .expr rhs
                frames := { f with k := { kind := .binaryR op v, before := cont.before, after := cont.after, span := cont.span } :: rest } :: fr }
          | .binaryR op lhs =>
              match evalBinary op lhs.ty lhs.value v.value with
              | .error trap => .step { c with focus := .trapped trap cont.span }
              | .ok w => .step { c with focus := .value w, frames := { f with k := rest } :: fr }
          | .cast to =>
              match evalCast v.ty to v.value with
              | .error trap => .step { c with focus := .trapped trap cont.span }
              | .ok w => .step { c with focus := .value w, frames := { f with k := rest } :: fr }
          | .callArgs callee done remaining isStmt =>
              match remaining with
              | [] => .stuck
              | [_] =>
                  performCall u { c with frames := { f with k := rest } :: fr }
                    callee (done ++ [v]) isStmt cont.span
              -- The value just consumed belongs to `remaining`'s head, which the
              -- continuation still carries; evaluate the argument after it.
              | _ :: next :: more =>
                  .step { c with
                    focus := .expr next
                    frames := { f with k := { kind := .callArgs callee (done ++ [v]) (next :: more) isStmt, before := cont.before, after := cont.after, span := cont.span } :: rest } :: fr }
          | .ifCond arms elseBody =>
              match arms with
              | [] => .step { c with focus := .stmts elseBody cont.before, frames := { f with k := rest, loopCtx := cont.before } :: fr }
              | arm :: remainingArms =>
                  if v.value == 0 then
                    match remainingArms with
                    | [] => .step { c with focus := .stmts elseBody cont.before, frames := { f with k := rest, loopCtx := cont.before } :: fr }
                    | next :: more =>
                        .step { c with
                          focus := .expr next.cond
                          frames := { f with k := { kind := .ifCond (next :: more) elseBody, before := cont.before, after := cont.after, span := cont.span } :: rest, loopCtx := cont.before } :: fr }
                  else
                    .step { c with focus := .stmts arm.body cont.before, frames := { f with k := rest, loopCtx := cont.before } :: fr }
          | .caseSel _ arms elseBody =>
              match caseArmBody? arms v.value with
              | some body => .step { c with focus := .stmts body cont.before, frames := { f with k := rest, loopCtx := cont.before } :: fr }
              | none => .step { c with focus := .stmts elseBody cont.before, frames := { f with k := rest, loopCtx := cont.before } :: fr }
          | .whileCond cond body =>
              if v.value == 0 then
                .step { c with focus := .resume, frames := { f with k := rest, loopCtx := cont.after } :: fr }
              else
                let bodyLc := LoopCtx.body cont.before cont.before.controlled
                .step { c with
                  focus := .stmts body bodyLc
                  frames := { f with k := { kind := .whileLoop cond body, before := bodyLc, after := cont.before, span := cont.span } :: rest, loopCtx := bodyLc } :: fr }
          | .repeatBody body cond =>
              let condLc := cont.after
              .step { c with
                focus := .expr cond
                frames := { f with k := Cont.same (.repeatCond body cond) condLc cont.span :: rest, loopCtx := condLc } :: fr }
          | .repeatCond body cond =>
              if v.value == 0 then
                let bodyLc := LoopCtx.body cont.before cont.before.controlled
                .step { c with
                  focus := .stmts body bodyLc
                  frames := { f with k := { kind := .repeatBody body cond, before := bodyLc, after := cont.before, span := cont.span } :: rest, loopCtx := bodyLc } :: fr }
              else
                .step { c with focus := .resume, frames := { f with k := rest, loopCtx := cont.after } :: fr }
          | .forInit var stop stepE body =>
              -- Write the start value first; the continuation update must use the
              -- updated head frame, not the original one, or the write is lost.
              let c1 := c.setVar var.id v
              match c1.frames with
              | [] => .stuck
              | f1 :: restFrames =>
                  .step { c1 with
                    focus := .expr stop
                    frames := { f1 with
                      k := Cont.same (.forStop var stepE body) cont.before cont.span :: rest,
                      loopCtx := cont.before } :: restFrames }
          | .forStop var stepE body =>
              match stepE with
              | none =>
                  forContinue u { c with frames := { f with k := rest } :: fr }
                    (Cont.same (.forStop var none body) cont.before cont.span) rest var v (Value.ofInt v.ty 1) body
              | some stepExpr =>
                  .step { c with
                    focus := .expr stepExpr
                    frames := { f with k := { kind := .forStep var v body, before := cont.before, after := cont.after, span := cont.span } :: rest, loopCtx := cont.before } :: fr }
          | .forStep var stopVal body =>
              forContinue u { c with frames := { f with k := rest } :: fr }
                (Cont.same (.forStep var stopVal body) cont.before cont.span) rest var stopVal v body
          | .whileLoop .. => .stuck
          | .forBody .. => .stuck
          | .seq .. => .stuck

/-- Resume after a statement completes. -/
def stepResume (u : HirUnit) (c : State) : Outcome :=
  match c.frames with
  | [] => .stuck
  | f :: fr =>
      match f.k with
      | [] => completeFrame u c
      | cont :: rest =>
          match cont.kind with
          | .seq restStmts => .step { c with focus := .stmts restStmts cont.after, frames := { f with k := rest, loopCtx := cont.after } :: fr }
          | .whileLoop cond body =>
              .step { c with
                focus := .expr cond
                frames := { f with k := Cont.same (.whileCond cond body) cont.after cont.span :: rest, loopCtx := cont.after } :: fr }
          | .repeatBody body cond =>
              .step { c with
                focus := .expr cond
                frames := { f with k := Cont.same (.repeatCond body cond) cont.after cont.span :: rest, loopCtx := cont.after } :: fr }
          | .forBody var stopVal stepVal body => forAdvance u c cont rest var stopVal stepVal body
          | _ => .stuck

/-- Deliver a returned value to the caller. -/
def stepReturned (u : HirUnit) (c : State) (rv : Option Value) : Outcome :=
  match c.frames with
  | [] => .stuck
  | f :: fr =>
      match f.k with
      | [] => .step { c with focus := .resume }
      | cont :: _ =>
          if cont.wantsValue then
            match rv with
            | some v => .step { c with focus := .value v }
            | none => .stuck
          else .step { c with focus := .resume }

/-- One small step. -/
def step (u : HirUnit) (c : State) : Outcome :=
  match c.focus with
  | .expr e => stepExpr u c e
  | .value v => stepValue u c v
  | .stmts ss lc => stepStmts u c ss lc
  | .resume => stepResume u c
  | .returned rv => stepReturned u c rv
  | .trapped _ _ => .halt
  | .halted => .halt

end

/-! ## Step relations -/

/-- A single step. -/
def Step (u : HirUnit) (c c' : State) : Prop := step u c = .step c'

/-- The reflexive-transitive closure of `Step`, prefix-oriented so induction
follows the run from the initial configuration. -/
inductive Steps (u : HirUnit) : State → State → Prop where
  /-- A zero-step run. -/
  | refl (c : State) : Steps u c c
  /-- A step followed by a run. -/
  | step {a b c : State} : Step u a b → Steps u b c → Steps u a c

/-- A terminal configuration: a trap or a completed requested run. -/
def Terminal (u : HirUnit) (c : State) : Prop := step u c = .halt

end Stc.Sem
