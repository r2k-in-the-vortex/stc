/-
The source dynamic semantics and its soundness results (P0.3).

This module defines:

* the configuration typing judgment `ConfigTyped`, including the inductive
  continuation-stack typing `StackTyped`;
* determinism, progress, and preservation for `Stc.Sem.Step`;
* type soundness, trap soundness, and exact-arithmetic trap agreement.

The configuration invariant records the types of stored values, the typing of
the focus and of every suspended continuation, and the shape of the call stack.
Typing facts about the AST carried by a continuation live directly in the
`StackTyped` rule that admits it, so the machine itself needs no type
annotations.
-/
import Stc.Sem.Machine

namespace Stc.Sem

open Stc.Hir
open Stc.Syntax

/-! ## Inversion of call typing -/

/-- The two shapes of a `CallHasType` derivation. Stated with the result as a
variable so it can be inverted without dependent-elimination problems. -/
theorem callHasType_cases {ctx : Ctx} {callee : Callee} {args : List Expr}
    {result : Option Ty} (h : CallHasType ctx callee args result) :
    (∃ index span sig, callee = .function index span ∧
        ctx.functions[index]? = some sig ∧ result = sig.result ∧
        Forall2 (ExprHasType ctx) args sig.params) ∨
      (∃ fn span ty, callee = .std fn span ∧ result = some ty ∧
        StdFnHasType ctx fn args ty) := by
  cases h with
  | function hsig hargs =>
      exact Or.inl ⟨_, _, _, rfl, hsig, rfl, hargs⟩
  | std hstd =>
      cases hstd with
      | mk hargs hres =>
          exact Or.inr ⟨_, _, _, rfl, rfl, .mk hargs hres⟩

/-- The standard-function typing inversion. -/
theorem stdFnHasType_cases {ctx : Ctx} {fn : StdFn} {args : List Expr} {ty : Ty}
    (h : StdFnHasType ctx fn args ty) :
    ∃ argTys, Forall2 (ExprHasType ctx) args argTys ∧ stdResult? fn argTys = some ty := by
  cases h with
  | mk hargs hres => exact ⟨_, hargs, hres⟩

/-! ## Frame typing -/

/-- Global roots agree with their declarations and are well formed. -/
def GlobalsTyped (u : HirUnit) (globals : List Value) : Prop :=
  globals.length = u.globals.length ∧
    ∀ (i : Nat) (v : Value), globals[i]? = some v →
      ∃ g, u.globals[i]? = some g ∧ v.ty = g.ty ∧ v.WellFormed

/-- A frame's parameters, locals, and result agree with its POU. The
statement is pointwise and records lengths, so both lookup and in-place update
are easy to reason about. -/
def FrameMatches (u : HirUnit) (f : Frame) : Prop :=
  match f.kind with
  | .function index =>
      ∃ fn, u.functions[index]? = some fn ∧
        f.params.length = fn.params.length ∧
        (∀ (i : Nat) (v : Value), f.params[i]? = some v →
          ∃ p, fn.params[i]? = some p ∧ v.ty = p.ty ∧ v.WellFormed) ∧
        f.locals.length = fn.locals.length ∧
        (∀ (i : Nat) (v : Value), f.locals[i]? = some v →
          ∃ l, fn.locals[i]? = some l ∧ v.ty = l.ty ∧ v.WellFormed) ∧
        (match fn.resultType, f.result with
          | none, none => True
          | some ty, some v => v.ty = ty ∧ v.WellFormed
          | _, _ => False)
  | .program =>
      f.params = [] ∧ f.result = none ∧
        f.locals.length = u.program.locals.length ∧
        (∀ (i : Nat) (v : Value), f.locals[i]? = some v →
          ∃ l, u.program.locals[i]? = some l ∧ v.ty = l.ty ∧ v.WellFormed)

/-- The shape of a call stack: function frames above a single program frame. -/
inductive FramesShape : List Frame → Prop where
  /-- The bottom program frame. -/
  | last {f : Frame} (h : f.kind = .program) : FramesShape [f]
  /-- A function frame above the rest of the stack. -/
  | cons {f : Frame} {rest : List Frame} (h : ∃ i, f.kind = .function i)
      (htail : FramesShape rest) : FramesShape (f :: rest)

/-- A shaped stack is nonempty. -/
theorem FramesShape.ne_nil {fs : List Frame} (h : FramesShape fs) : fs ≠ [] := by
  cases h <;> simp

/-- A shaped stack has a head. -/
theorem FramesShape.exists_head {fs : List Frame} (h : FramesShape fs) :
    ∃ f rest, fs = f :: rest := by
  cases h with
  | last => exact ⟨_, [], rfl⟩
  | cons _ _ => exact ⟨_, _, rfl⟩

/-- A shaped stack with a function head has a nonempty tail. -/
theorem FramesShape.tail_ne_nil {f : Frame} {rest : List Frame}
    (h : FramesShape (f :: rest)) (hf : ∃ i, f.kind = .function i) : rest ≠ [] := by
  intro hnil
  rw [hnil] at h
  cases h with
  | last hk => rcases hf with ⟨i, hi⟩; rw [hi] at hk; cases hk
  | cons _ htail => cases htail

/-- The expectation named by an optional result type. -/
def Expect.ofOption : Option Ty → Expect
  | some ty => .value ty
  | none => .unit

/-- The call's signature at a list of argument types, with already-evaluated
arguments carrying their runtime types. -/
def CalleeSignatureV (ctx : Ctx) (callee : Callee) (done : List Value)
    (remainingTys : List Ty) (result : Option Ty) : Prop :=
  match callee with
  | .function index _ =>
      ∃ sig, ctx.functions[index]? = some sig ∧
        sig.params = done.map Value.ty ++ remainingTys ∧ sig.result = result
  | .std fn _ => stdResult? fn (done.map Value.ty ++ remainingTys) = result

/-! ## Stack typing -/

/-- Whether the stack contains a loop marker. -/
def StackHasLoop : List Cont → Prop
  | [] => False
  | c :: rest => c.isLoopMarker = true ∨ StackHasLoop rest

/-- The continuation stack: each continuation consumes the current focus's
expectation, types the AST it carries, and leaves the next expectation. -/
inductive StackTyped (ctx : Ctx) : List Cont → Expect → LoopCtx → Prop where
  /-- The frame body completed. -/
  | nil {lc : LoopCtx} (hdepth : lc.depth = 0) (hctrl : lc.controlled = []) :
      StackTyped ctx [] .unit lc
  /-- Store a value into a place. -/
  | assign {place : Place} {ty : Ty} {lc : LoopCtx} {span : Span} {rest : List Cont}
      (hlookup : ctx.lookup place.id = some ty)
      (hrest : StackTyped ctx rest .unit lc) :
      StackTyped ctx
        ({ kind := .assign place, before := lc, after := lc, span := span } :: rest)
        (.value ty) lc
  /-- Apply a unary operation. -/
  | unary {op : UnaryOp} {ty : Ty} {lc : LoopCtx} {span : Span} {rest : List Cont}
      (hop : unaryWellTyped op ty = true)
      (hrest : StackTyped ctx rest (.value ty) lc) :
      StackTyped ctx
        ({ kind := .unary op, before := lc, after := lc, span := span } :: rest)
        (.value ty) lc
  /-- Evaluate the right operand of a binary operation. -/
  | binaryL {op : BinaryOp} {rhs : Expr} {ty res : Ty} {lc : LoopCtx} {span : Span}
      {rest : List Cont}
      (hrhs : ExprHasType ctx rhs ty)
      (hop : binaryWellTyped op ty ty = true)
      (hres : res = binaryResultTy op ty ty)
      (hrest : StackTyped ctx rest (.value res) lc) :
      StackTyped ctx
        ({ kind := .binaryL op rhs, before := lc, after := lc, span := span } :: rest)
        (.value ty) lc
  /-- Apply a binary operation to the evaluated left operand. -/
  | binaryR {op : BinaryOp} {lhs : Value} {ty res : Ty} {lc : LoopCtx}
      {span : Span} {rest : List Cont}
      (hlhs : lhs.WellFormed)
      (hty : lhs.ty = ty)
      (hop : binaryWellTyped op ty ty = true)
      (hres : res = binaryResultTy op ty ty)
      (hrest : StackTyped ctx rest (.value res) lc) :
      StackTyped ctx
        ({ kind := .binaryR op lhs, before := lc, after := lc, span := span } :: rest)
        (.value ty) lc
  /-- Apply a conversion. -/
  | cast {to src : Ty} {lc : LoopCtx} {span : Span} {rest : List Cont}
      (hconv : ConversionLegal src to)
      (hcore : to.isCore = true)
      (hrest : StackTyped ctx rest (.value to) lc) :
      StackTyped ctx
        ({ kind := .cast to, before := lc, after := lc, span := span } :: rest)
        (.value src) lc
  /-- Evaluate the next argument of a call. -/
  | callArgs {callee : Callee} {done : List Value} {remaining : List Expr}
      {isStmt : Bool} {t : Ty} {ts : List Ty} {result : Option Ty}
      {lc : LoopCtx} {span : Span} {rest : List Cont}
      (hne : remaining ≠ [])
      (htyped : Forall2 (ExprHasType ctx) remaining (t :: ts))
      (hdone : ∀ v ∈ done, v.WellFormed)
      (hsig : CalleeSignatureV ctx callee done (t :: ts) result)
      (hresult : isStmt = false → result ≠ none)
      (hrest : StackTyped ctx rest (if isStmt then .unit else .ofOption result) lc) :
      StackTyped ctx
        ({ kind := .callArgs callee done remaining isStmt, before := lc, after := lc,
           span := span } :: rest)
        (.value t) lc
  /-- Choose an `IF` arm. -/
  | ifCond {arms : List IfArm} {elseBody : List Stmt} {lc : LoopCtx} {span : Span}
      {rest : List Cont}
      (harms : ∀ arm ∈ arms, ExprHasType ctx arm.cond .bool ∧
        StmtListHasType ctx lc.depth lc.controlled arm.body)
      (helse : StmtListHasType ctx lc.depth lc.controlled elseBody)
      (hrest : StackTyped ctx rest .unit lc) :
      StackTyped ctx
        ({ kind := .ifCond arms elseBody, before := lc, after := lc, span := span } :: rest)
        (.value .bool) lc
  /-- Choose a `CASE` arm. -/
  | caseSel {selector : Expr} {selTy : Ty} {arms : List CaseArm}
      {elseBody : List Stmt} {lc : LoopCtx} {span : Span} {rest : List Cont}
      (hsel : ExprHasType ctx selector selTy)
      (hint : selTy.isInt = true)
      (hlabels : CaseLabelsValid selTy (allCaseLabels arms))
      (hbodies : ∀ arm ∈ arms, StmtListHasType ctx lc.depth lc.controlled arm.body)
      (helse : StmtListHasType ctx lc.depth lc.controlled elseBody)
      (hrest : StackTyped ctx rest .unit lc) :
      StackTyped ctx
        ({ kind := .caseSel selector arms elseBody, before := lc, after := lc,
           span := span } :: rest)
        (.value selTy) lc
  /-- Continue a `WHILE` after its condition. -/
  | whileCond {cond : Expr} {body : List Stmt} {lc : LoopCtx} {span : Span}
      {rest : List Cont}
      (hcond : ExprHasType ctx cond .bool)
      (hbody : StmtListHasType ctx (lc.depth + 1) lc.controlled body)
      (hrest : StackTyped ctx rest .unit lc) :
      StackTyped ctx
        ({ kind := .whileCond cond body, before := lc, after := lc, span := span } :: rest)
        (.value .bool) lc
  /-- Re-evaluate a `WHILE` condition after its body. -/
  | whileLoop {cond : Expr} {body : List Stmt} {before after : LoopCtx}
      {span : Span} {rest : List Cont}
      (hdepth : before.depth = after.depth + 1)
      (hctrl : before.controlled = after.controlled)
      (hcond : ExprHasType ctx cond .bool)
      (hbody : StmtListHasType ctx before.depth before.controlled body)
      (hloop : after.depth > 0 → StackHasLoop rest)
      (hrest : StackTyped ctx rest .unit after) :
      StackTyped ctx
        ({ kind := .whileLoop cond body, before := before, after := after,
           span := span } :: rest)
        .unit before
  /-- Evaluate a `REPEAT` condition after its body. -/
  | repeatBody {body : List Stmt} {cond : Expr} {before after : LoopCtx}
      {span : Span} {rest : List Cont}
      (hdepth : before.depth = after.depth + 1)
      (hctrl : before.controlled = after.controlled)
      (hcond : ExprHasType ctx cond .bool)
      (hbody : StmtListHasType ctx before.depth before.controlled body)
      (hloop : after.depth > 0 → StackHasLoop rest)
      (hrest : StackTyped ctx rest .unit after) :
      StackTyped ctx
        ({ kind := .repeatBody body cond, before := before, after := after,
           span := span } :: rest)
        .unit before
  /-- Continue a `REPEAT` after its condition. -/
  | repeatCond {body : List Stmt} {cond : Expr} {lc : LoopCtx} {span : Span}
      {rest : List Cont}
      (hcond : ExprHasType ctx cond .bool)
      (hbody : StmtListHasType ctx (lc.depth + 1) lc.controlled body)
      (hrest : StackTyped ctx rest .unit lc) :
      StackTyped ctx
        ({ kind := .repeatCond body cond, before := lc, after := lc, span := span } :: rest)
        (.value .bool) lc
  /-- Continue a `FOR` after the start value. -/
  | forInit {var : Place} {stop : Expr} {step : Option Expr} {body : List Stmt}
      {ty : Ty} {lc : LoopCtx} {span : Span} {rest : List Cont}
      (hvar : ctx.lookup var.id = some ty)
      (hint : ty.isInt = true)
      (hnot : var.id ∉ lc.controlled)
      (hstop : ExprHasType ctx stop ty)
      (hstep : ∀ e, step = some e → ExprHasType ctx e ty)
      (hzero : ∀ e, step = some e → ∀ v, Expr.constInt? e = some v → v ≠ 0)
      (hbody : StmtListHasType ctx (lc.depth + 1) (var.id :: lc.controlled) body)
      (hrest : StackTyped ctx rest .unit lc) :
      StackTyped ctx
        ({ kind := .forInit var stop step body, before := lc, after := lc,
           span := span } :: rest)
        (.value ty) lc
  /-- Continue a `FOR` after the stop bound. -/
  | forStop {var : Place} {step : Option Expr} {body : List Stmt} {ty : Ty}
      {lc : LoopCtx} {span : Span} {rest : List Cont}
      (hvar : ctx.lookup var.id = some ty)
      (hint : ty.isInt = true)
      (hnot : var.id ∉ lc.controlled)
      (hstep : ∀ e, step = some e → ExprHasType ctx e ty)
      (hzero : ∀ e, step = some e → ∀ v, Expr.constInt? e = some v → v ≠ 0)
      (hbody : StmtListHasType ctx (lc.depth + 1) (var.id :: lc.controlled) body)
      (hrest : StackTyped ctx rest .unit lc) :
      StackTyped ctx
        ({ kind := .forStop var step body, before := lc, after := lc, span := span } :: rest)
        (.value ty) lc
  /-- Continue a `FOR` after the step, checking the first iteration. -/
  | forStep {var : Place} {stopVal : Value} {body : List Stmt} {ty : Ty}
      {lc : LoopCtx} {span : Span} {rest : List Cont}
      (hvar : ctx.lookup var.id = some ty)
      (hint : ty.isInt = true)
      (hnot : var.id ∉ lc.controlled)
      (hstop : stopVal.WellFormed ∧ stopVal.ty = ty)
      (hbody : StmtListHasType ctx (lc.depth + 1) (var.id :: lc.controlled) body)
      (hrest : StackTyped ctx rest .unit lc) :
      StackTyped ctx
        ({ kind := .forStep var stopVal body, before := lc, after := lc,
           span := span } :: rest)
        (.value ty) lc
  /-- Advance a `FOR` after its body. -/
  | forBody {var : Place} {stopVal stepVal : Value} {body : List Stmt} {ty : Ty}
      {before after : LoopCtx} {span : Span} {rest : List Cont}
      (hdepth : before.depth = after.depth + 1)
      (hctrl : before.controlled = var.id :: after.controlled)
      (hvar : ctx.lookup var.id = some ty)
      (hint : ty.isInt = true)
      (hstop : stopVal.WellFormed ∧ stopVal.ty = ty)
      (hstep : stepVal.WellFormed ∧ stepVal.ty = ty)
      (hbody : StmtListHasType ctx before.depth before.controlled body)
      (hloop : after.depth > 0 → StackHasLoop rest)
      (hrest : StackTyped ctx rest .unit after) :
      StackTyped ctx
        ({ kind := .forBody var stopVal stepVal body, before := before, after := after,
           span := span } :: rest)
        .unit before
  /-- Continue the statement list after a statement. The remaining list must
  stay typed so resuming the continuation reconstructs a typed statement
  focus. -/
  | seq {restStmts : List Stmt} {lc : LoopCtx} {span : Span} {rest : List Cont}
      (hstmts : StmtListHasType ctx lc.depth lc.controlled restStmts)
      (hrest : StackTyped ctx rest .unit lc) :
      StackTyped ctx
        ({ kind := .seq restStmts, before := lc, after := lc, span := span } :: rest)
        .unit lc

/-! ## Focus and configuration typing -/

/-- The focus of a running frame is well typed. -/
def FocusTyped (u : HirUnit) (f : Frame) : Focus → Prop
  | .expr e =>
      ∃ T, ExprHasType (frameCtx u f.kind) e T ∧
        StackTyped (frameCtx u f.kind) f.k (.value T) f.loopCtx
  | .value v =>
      v.WellFormed ∧ ∃ T, v.ty = T ∧
        StackTyped (frameCtx u f.kind) f.k (.value T) f.loopCtx
  | .stmts ss lc =>
      f.loopCtx = lc ∧ StackTyped (frameCtx u f.kind) f.k .unit lc ∧
        StmtListHasType (frameCtx u f.kind) lc.depth lc.controlled ss ∧
        (lc.depth > 0 → StackHasLoop f.k)
  | .resume =>
      StackTyped (frameCtx u f.kind) f.k .unit f.loopCtx
  | .returned none =>
      StackTyped (frameCtx u f.kind) f.k .unit f.loopCtx
  | .returned (some v) =>
      v.WellFormed ∧
        (StackTyped (frameCtx u f.kind) f.k (.value v.ty) f.loopCtx ∨
          StackTyped (frameCtx u f.kind) f.k .unit f.loopCtx)
  | .trapped _ _ => True
  | .halted => True

/-- A suspended frame: its stored values match its declarations, its
continuation stack is typed under the expectation the callee will meet, and a
positive loop depth has a loop marker in the stack. -/
def FrameTyped (u : HirUnit) (f : Frame) : Prop :=
  FrameMatches u f ∧
    StackTyped (frameCtx u f.kind) f.k (.ofOption f.expect) f.loopCtx ∧
    (f.loopCtx.depth > 0 → StackHasLoop f.k)

/-- A suspended caller's expectation is the callee's declared result type, or
`none` for a statement call. The callee's current result value is deliberately
not used, so a write to the result variable does not disturb the link. -/
def CallerExpects (u : HirUnit) (f g : Frame) : Prop :=
  g.expect = none ∨
    ∃ index fn ty, f.kind = .function index ∧ u.functions[index]? = some fn ∧
      fn.resultType = some ty ∧ g.expect = some ty

/-- Every adjacent suspended-frame pair has a matching caller expectation. -/
def FramesCallerExpects (u : HirUnit) : List Frame → Prop
  | [] => True
  | [_] => True
  | f :: g :: rest => CallerExpects u f g ∧ FramesCallerExpects u (g :: rest)

/-- Every suspended frame is well typed. -/
def FramesTailTyped (u : HirUnit) : List Frame → Prop
  | [] => True
  | f :: rest => FrameTyped u f ∧ FramesTailTyped u rest

/-- The configuration typing judgment. -/
structure ConfigTyped (u : HirUnit) (s : State) : Prop where
  /-- Global roots are well typed. -/
  globals : GlobalsTyped u s.globals
  /-- The call stack has function frames above one program frame. -/
  shape : FramesShape s.frames
  /-- The running frame's stored values match its declarations. -/
  headMatch : ∀ f rest, s.frames = f :: rest → FrameMatches u f
  /-- The running frame's focus is well typed. -/
  headFocus : ∀ f rest, s.frames = f :: rest → FocusTyped u f s.focus
  /-- The suspended frames are well typed. -/
  tailTyped : ∀ f rest, s.frames = f :: rest → FramesTailTyped u rest
  /-- A positive loop depth has a loop marker in the running frame's stack. -/
  headLoop : ∀ f rest, s.frames = f :: rest → f.loopCtx.depth > 0 → StackHasLoop f.k
  /-- Every suspended caller expectation matches the callee's declared result. -/
  callers : FramesCallerExpects u s.frames

/-! ## Basic projections -/

/-- The head frame exists. -/
theorem ConfigTyped.head_exists {u : HirUnit} {s : State} (h : ConfigTyped u s) :
    ∃ f rest, s.frames = f :: rest :=
  h.shape.exists_head

/-- The focus is typed. -/
theorem ConfigTyped.head_focus {u : HirUnit} {s : State} {f : Frame} {rest : List Frame}
    (h : ConfigTyped u s) (hf : s.frames = f :: rest) : FocusTyped u f s.focus :=
  h.headFocus f rest hf

/-- A value in focus is well formed. -/
theorem FocusTyped.value_wellFormed {u : HirUnit} {f : Frame} {v : Value}
    (h : FocusTyped u f (.value v)) : v.WellFormed := h.1

/-- A statement focus is typed at its loop context. -/
theorem FocusTyped.stmts_typed {u : HirUnit} {f : Frame} {ss : List Stmt} {lc : LoopCtx}
    (h : FocusTyped u f (.stmts ss lc)) :
    StmtListHasType (frameCtx u f.kind) lc.depth lc.controlled ss := h.2.2.1

/-- A statement focus at positive depth has a loop marker below it. -/
theorem FocusTyped.stmts_loop {u : HirUnit} {f : Frame} {ss : List Stmt} {lc : LoopCtx}
    (h : FocusTyped u f (.stmts ss lc)) (hd : lc.depth > 0) : StackHasLoop f.k :=
  h.2.2.2 hd

/-- A value-expecting stack is nonempty. -/
theorem StackTyped.value_ne_nil {ctx : Ctx} {k : List Cont} {ty : Ty} {lc : LoopCtx}
    (h : StackTyped ctx k (.value ty) lc) : k ≠ [] := by
  intro hnil
  rw [hnil] at h
  cases h

/-- A unit-expecting stack either is empty or its head does not want a value. -/
theorem StackTyped.unit_head {ctx : Ctx} {c : Cont} {rest : List Cont} {lc : LoopCtx}
    (h : StackTyped ctx (c :: rest) .unit lc) : c.wantsValue = false := by
  cases h <;> simp [Cont.wantsValue, ContKind.wantsValue]

/-- A value-expecting stack's head wants a value. -/
theorem StackTyped.value_head {ctx : Ctx} {c : Cont} {rest : List Cont} {ty : Ty}
    {lc : LoopCtx} (h : StackTyped ctx (c :: rest) (.value ty) lc) :
    c.wantsValue = true := by
  cases h <;> simp [Cont.wantsValue, ContKind.wantsValue]

/-- A stack with a loop marker satisfies `StackHasLoop`. -/
theorem StackTyped.hasLoop {ctx : Ctx} {c : Cont} {rest : List Cont} {input : Expect}
    {lc : LoopCtx} (h : StackTyped ctx (c :: rest) input lc) (hm : c.isLoopMarker = true) :
    StackHasLoop (c :: rest) :=
  Or.inl hm

/-- A stack with a loop marker has a `findLoop` result. -/
theorem StackHasLoop.findLoop_some {k : List Cont} (h : StackHasLoop k) :
    ∃ cont rest, findLoop k = some (cont, rest) := by
  induction k with
  | nil => cases h
  | cons c rest ih =>
      by_cases hm : c.isLoopMarker = true
      · exact ⟨c, rest, by simp [findLoop, hm]⟩
      · have htail : StackHasLoop rest := by
          simpa [StackHasLoop, hm] using h
        rcases ih htail with ⟨cont, rest', hk'⟩
        exact ⟨cont, rest', by simp [findLoop, hm, hk']⟩

/-- The typing facts a loop marker exposes: its body is typed in the marker's
outer loop context. -/
def MarkerTyped (ctx : Ctx) (marker : Cont) : Prop :=
  marker.isLoopMarker = true ∧
    match marker.kind with
    | .whileLoop cond body =>
        ExprHasType ctx cond .bool ∧
          StmtListHasType ctx (marker.after.depth + 1) marker.after.controlled body
    | .repeatBody body cond =>
        ExprHasType ctx cond .bool ∧
          StmtListHasType ctx (marker.after.depth + 1) marker.after.controlled body
    | .forBody var stopVal stepVal body =>
        ∃ ty, ctx.lookup var.id = some ty ∧ ty.isInt = true ∧
          stopVal.WellFormed ∧ stopVal.ty = ty ∧
          stepVal.WellFormed ∧ stepVal.ty = ty ∧
          StmtListHasType ctx (marker.after.depth + 1)
            (var.id :: marker.after.controlled) body
    | _ => True

/-- The innermost loop marker found by `findLoop` is typed, and the stack below
it is typed in the marker's outer loop context. -/
theorem StackTyped.findLoop_marker {ctx : Ctx} {k below : List Cont} {marker : Cont}
    {lc : LoopCtx} (h : StackTyped ctx k .unit lc)
    (hfind : findLoop k = some (marker, below)) :
    StackTyped ctx below .unit marker.after ∧
      (marker.after.depth > 0 → StackHasLoop below) ∧
      MarkerTyped ctx marker := by
  induction k generalizing lc with
  | nil => simp [findLoop] at hfind
  | cons cont rest ih =>
      cases h with
      | whileLoop hdepth hctrl hcond hbody hloop hrest =>
          simp only [findLoop, Cont.isLoopMarker, ContKind.isLoopMarker] at hfind
          injection hfind with h1
          injection h1 with h2 h3
          subst marker
          subst below
          exact ⟨hrest, hloop, by
            refine ⟨by simp [Cont.isLoopMarker, ContKind.isLoopMarker], ?_⟩
            exact ⟨hcond, by simpa [hdepth, hctrl] using hbody⟩⟩
      | repeatBody hdepth hctrl hcond hbody hloop hrest =>
          simp only [findLoop, Cont.isLoopMarker, ContKind.isLoopMarker] at hfind
          injection hfind with h1
          injection h1 with h2 h3
          subst marker
          subst below
          exact ⟨hrest, hloop, by
            refine ⟨by simp [Cont.isLoopMarker, ContKind.isLoopMarker], ?_⟩
            exact ⟨hcond, by simpa [hdepth, hctrl] using hbody⟩⟩
      | forBody hdepth hctrl hvar hint hstop hstep hbody hloop hrest =>
          simp only [findLoop, Cont.isLoopMarker, ContKind.isLoopMarker] at hfind
          injection hfind with h1
          injection h1 with h2 h3
          subst marker
          subst below
          exact ⟨hrest, hloop, by
            refine ⟨by simp [Cont.isLoopMarker, ContKind.isLoopMarker], ?_⟩
            exact ⟨_, hvar, hint, hstop.1, hstop.2, hstep.1, hstep.2,
              by simpa [hdepth, hctrl] using hbody⟩⟩
      | seq hstmts hrest =>
          simp only [findLoop, Cont.isLoopMarker, ContKind.isLoopMarker] at hfind
          exact ih hrest hfind

/-- The unit-wide function table of a function frame's context. -/
theorem frameCtx_functions_ofFunction {u : HirUnit} {index : Nat} {fn : Function}
    (h : u.functions[index]? = some fn) :
    (frameCtx u (.function index)).functions = (Env.ofUnit u).functions := by
  simp [frameCtx, h, Ctx.ofFunction]

/-- The unit-wide function table of the program context. -/
theorem frameCtx_functions_program (u : HirUnit) :
    (frameCtx u .program).functions = (Env.ofUnit u).functions := rfl

/-- A function signature in the unit environment comes from the unit's function
list. -/
theorem Env.ofUnit_functions_getElem? {u : HirUnit} {index : Nat} {sig : Signature}
    (h : (Env.ofUnit u).functions[index]? = some sig) :
    ∃ fn, u.functions[index]? = some fn := by
  simp only [Env.ofUnit] at h
  rw [List.getElem?_map] at h
  cases hl : u.functions[index]? with
  | none => rw [hl] at h; cases h
  | some fn => exact ⟨fn, rfl⟩

/-- A shaped stack whose head is a function frame has a nonempty tail. -/
theorem FramesShape.function_tail_ne_nil {f : Frame} {rest : List Frame}
    (h : FramesShape (f :: rest)) (hf : ∃ index, f.kind = .function index) :
    rest ≠ [] :=
  FramesShape.tail_ne_nil h hf

/-! ## Runtime store operations -/

/-- A store write never empties the call stack. -/
theorem State.setVar_frames_ne_nil {s : State} {id : VarId} {v : Value}
    (h : s.frames ≠ []) : (s.setVar id v).frames ≠ [] := by
  cases hs : s.frames with
  | nil => exact absurd hs h
  | cons f fr =>
      cases id <;> simp [State.setVar, hs]

/-- A `Forall2` relation relates lists of equal length. -/
theorem Forall2.length_eq {α β : Type} {R : α → β → Prop} {as : List α} {bs : List β}
    (h : Forall2 R as bs) : as.length = bs.length := by
  induction h with
  | nil => rfl
  | cons _ _ ih => simp [ih]

/-- A mapped list lookup success comes from a source lookup success. -/
theorem List.getElem?_map_some {α β : Type} {f : α → β} {l : List α} {i : Nat}
    {b : β} (h : (l.map f)[i]? = some b) : ∃ a, l[i]? = some a := by
  rw [List.getElem?_map] at h
  cases hl : l[i]? with
  | none => rw [hl] at h; cases h
  | some a => exact ⟨a, rfl⟩

/-- A lookup success is an in-range index. -/
theorem List.getElem?_isSome {α : Type} {l : List α} {i : Nat} {a : α}
    (h : l[i]? = some a) : i < l.length :=
  (List.getElem?_eq_some_iff.mp h).1

/-- The context of a program frame. -/
theorem frameCtx_program (u : HirUnit) :
    frameCtx u .program = Ctx.ofProgram (Env.ofUnit u) u.program := rfl

/-- The context of a function frame. -/
theorem frameCtx_function {u : HirUnit} {index : Nat} {fn : Function}
    (h : u.functions[index]? = some fn) :
    frameCtx u (.function index) = Ctx.ofFunction (Env.ofUnit u) fn := by
  simp [frameCtx, h]

/-- A mapped-list lookup success identifies its source and mapping. -/
theorem List.getElem?_map_eq_some {α β : Type} {f : α → β} {l : List α} {i : Nat}
    {b : β} (h : (l.map f)[i]? = some b) : ∃ a, l[i]? = some a ∧ f a = b := by
  rw [List.getElem?_map] at h
  cases hl : l[i]? with
  | none => rw [hl] at h; cases h
  | some a =>
      rw [hl] at h
      have h' : f a = b := by simpa using h
      exact ⟨a, rfl, h'⟩

/-- A typed variable is present in the runtime store with its declared type
and a well-formed value. -/
theorem ConfigTyped.lookupVar?_typed {u : HirUnit} {s : State} {f : Frame}
    {rest : List Frame} (hc : ConfigTyped u s) (hf : s.frames = f :: rest)
    {id : VarId} {ty : Ty} (h : (frameCtx u f.kind).lookup id = some ty) :
    ∃ v, s.lookupVar? id = some v ∧ v.ty = ty ∧ v.WellFormed := by
  have hm := hc.headMatch f rest hf
  cases id with
  | param i =>
      cases hk : f.kind with
      | function index =>
          simp only [FrameMatches, hk] at hm
          rw [hk] at h
          rcases hm with ⟨fn, hfn, hplen, hpar, hllen, hloc, hres⟩
          rw [frameCtx_function hfn] at h
          simp only [Ctx.ofFunction, Ctx.lookup] at h
          obtain ⟨p, hp, hpt⟩ := List.getElem?_map_eq_some h
          have hi := List.getElem?_isSome hp
          have hi' : i < f.params.length := by omega
          rcases hpar i f.params[i] (List.getElem?_eq_getElem hi') with ⟨p', hp', hty, hwf⟩
          have hpp : p' = p := by rw [hp] at hp'; cases hp'; rfl
          refine ⟨f.params[i],
            by simp [State.lookupVar?, State.head?, hf, List.getElem?_eq_getElem hi'], ?_, hwf⟩
          rw [hty, hpp, hpt]
      | program =>
          rw [hk, frameCtx_program] at h
          simp [Ctx.ofProgram, Ctx.lookup] at h
  | «local» i =>
      cases hk : f.kind with
      | function index =>
          simp only [FrameMatches, hk] at hm
          rw [hk] at h
          rcases hm with ⟨fn, hfn, hplen, hpar, hllen, hloc, hres⟩
          rw [frameCtx_function hfn] at h
          simp only [Ctx.ofFunction, Ctx.lookup] at h
          obtain ⟨l, hl, hlt⟩ := List.getElem?_map_eq_some h
          have hi := List.getElem?_isSome hl
          have hi' : i < f.locals.length := by omega
          rcases hloc i f.locals[i] (List.getElem?_eq_getElem hi') with ⟨l', hl', hty, hwf⟩
          have hll : l' = l := by rw [hl] at hl'; cases hl'; rfl
          refine ⟨f.locals[i],
            by simp [State.lookupVar?, State.head?, hf, List.getElem?_eq_getElem hi'], ?_, hwf⟩
          rw [hty, hll, hlt]
      | program =>
          rw [hk, frameCtx_program] at h
          simp only [Ctx.ofProgram, Ctx.lookup] at h
          simp only [FrameMatches, hk] at hm
          rcases hm with ⟨_, _, hlen, hloc⟩
          obtain ⟨l, hl, hlt⟩ := List.getElem?_map_eq_some h
          have hi := List.getElem?_isSome hl
          have hi' : i < f.locals.length := by omega
          rcases hloc i f.locals[i] (List.getElem?_eq_getElem hi') with ⟨l', hl', hty, hwf⟩
          have hll : l' = l := by rw [hl] at hl'; cases hl'; rfl
          refine ⟨f.locals[i],
            by simp [State.lookupVar?, State.head?, hf, List.getElem?_eq_getElem hi'], ?_, hwf⟩
          rw [hty, hll, hlt]
  | «global» i =>
      cases hk : f.kind with
      | function index =>
          simp only [FrameMatches, hk] at hm
          rw [hk] at h
          rcases hm with ⟨fn, hfn, _⟩
          rw [frameCtx_function hfn] at h
          simp only [Ctx.ofFunction, Ctx.lookup] at h
          simp only [Env.ofUnit] at h
          rcases hc.globals with ⟨glen, gok⟩
          obtain ⟨g, hg, hgt⟩ := List.getElem?_map_eq_some h
          have hi := List.getElem?_isSome hg
          have hi' : i < s.globals.length := by omega
          rcases gok i s.globals[i] (List.getElem?_eq_getElem hi') with ⟨g', hg', hty, hwf⟩
          have hgg : g' = g := by rw [hg] at hg'; cases hg'; rfl
          refine ⟨s.globals[i],
            by simp [State.lookupVar?, List.getElem?_eq_getElem hi'], ?_, hwf⟩
          rw [hty, hgg, hgt]
      | program =>
          rw [hk, frameCtx_program] at h
          simp only [Ctx.ofProgram, Ctx.lookup] at h
          simp only [Env.ofUnit] at h
          rcases hc.globals with ⟨glen, gok⟩
          obtain ⟨g, hg, hgt⟩ := List.getElem?_map_eq_some h
          have hi := List.getElem?_isSome hg
          have hi' : i < s.globals.length := by omega
          rcases gok i s.globals[i] (List.getElem?_eq_getElem hi') with ⟨g', hg', hty, hwf⟩
          have hgg : g' = g := by rw [hg] at hg'; cases hg'; rfl
          refine ⟨s.globals[i],
            by simp [State.lookupVar?, List.getElem?_eq_getElem hi'], ?_, hwf⟩
          rw [hty, hgg, hgt]
  | result =>
      cases hk : f.kind with
      | function index =>
          simp only [FrameMatches, hk] at hm
          rw [hk] at h
          rcases hm with ⟨fn, hfn, hplen, hpar, hllen, hloc, hres⟩
          rw [frameCtx_function hfn] at h
          simp only [Ctx.ofFunction, Ctx.lookup] at h
          cases hrt : fn.resultType with
          | none => rw [hrt] at h; cases h
          | some rty =>
              rw [hrt] at h
              have hrty : rty = ty := by cases h; rfl
              rw [hrt] at hres
              cases hr : f.result with
              | none => rw [hr] at hres; cases hres
              | some v =>
                  rw [hr] at hres
                  rcases hres with ⟨hvty, hvwf⟩
                  refine ⟨v, by simp [State.lookupVar?, State.head?, hf, hr], ?_, hvwf⟩
                  rw [hvty, hrty]
      | program =>
          rw [hk, frameCtx_program] at h
          simp only [Ctx.ofProgram, Ctx.lookup] at h
          cases h

/-- A typed variable is present in the runtime store. -/
theorem ConfigTyped.lookupVar?_some {u : HirUnit} {s : State} {f : Frame}
    {rest : List Frame} (hc : ConfigTyped u s) (hf : s.frames = f :: rest)
    {id : VarId} {ty : Ty} (h : (frameCtx u f.kind).lookup id = some ty) :
    ∃ v, s.lookupVar? id = some v := by
  obtain ⟨v, hv, _, _⟩ := hc.lookupVar?_typed hf h
  exact ⟨v, hv⟩

/-! ## Store and frame preservation -/

/-- The global types of a function frame's context. -/
theorem frameCtx_globals_function {u : HirUnit} {index : Nat} {fn : Function}
    (h : u.functions[index]? = some fn) :
    (frameCtx u (.function index)).globals = (Env.ofUnit u).globalTypes := by
  rw [frameCtx_function h]; rfl

/-- The parameter types of a function frame's context. -/
theorem frameCtx_params_function {u : HirUnit} {index : Nat} {fn : Function}
    (h : u.functions[index]? = some fn) :
    (frameCtx u (.function index)).params = fn.params.map (·.ty) := by
  rw [frameCtx_function h]; rfl

/-- The local types of a function frame's context. -/
theorem frameCtx_locals_function {u : HirUnit} {index : Nat} {fn : Function}
    (h : u.functions[index]? = some fn) :
    (frameCtx u (.function index)).locals = fn.locals.map (·.ty) := by
  rw [frameCtx_function h]; rfl

/-- The result type of a function frame's context. -/
theorem frameCtx_result_function {u : HirUnit} {index : Nat} {fn : Function}
    (h : u.functions[index]? = some fn) :
    (frameCtx u (.function index)).result = fn.resultType := by
  rw [frameCtx_function h]; rfl

/-- The global types of the program context. -/
theorem frameCtx_globals_program (u : HirUnit) :
    (frameCtx u .program).globals = (Env.ofUnit u).globalTypes := rfl

/-- The program context has no parameters. -/
theorem frameCtx_params_program (u : HirUnit) : (frameCtx u .program).params = [] := rfl

/-- The program context's local types. -/
theorem frameCtx_locals_program (u : HirUnit) :
    (frameCtx u .program).locals = u.program.locals.map (·.ty) := rfl

/-- The program context has no result variable. -/
theorem frameCtx_result_program (u : HirUnit) : (frameCtx u .program).result = none := rfl

/-- A frame-shape is preserved when only a head frame's stored values change. -/
theorem FramesShape.updateHead {f f' : Frame} {rest : List Frame}
    (h : FramesShape (f :: rest)) (hk : f'.kind = f.kind) :
    FramesShape (f' :: rest) := by
  cases h with
  | last hkind => exact FramesShape.last (by rw [hk, hkind])
  | cons hfun htail =>
      exact FramesShape.cons
        (by rcases hfun with ⟨i, hi⟩; exact ⟨i, by rw [hk, hi]⟩) htail

/-- Focus typing depends on the frame's POU, continuation stack, and loop
context, not on its stored parameter, local, or result values. -/
theorem FocusTyped.updateFrame {u : HirUnit} {f f' : Frame} {focus : Focus}
    (h : FocusTyped u f focus) (hk : f'.kind = f.kind)
    (hk' : f'.k = f.k) (hlc : f'.loopCtx = f.loopCtx) :
    FocusTyped u f' focus := by
  cases focus with
  | expr e =>
      simp only [FocusTyped] at h ⊢
      rw [hk, hk', hlc]
      exact h
  | value v =>
      simp only [FocusTyped] at h ⊢
      rw [hk, hk', hlc]
      exact h
  | stmts ss lc =>
      simp only [FocusTyped] at h ⊢
      rw [hk, hk', hlc]
      exact h
  | resume =>
      simp only [FocusTyped] at h ⊢
      rw [hk, hk', hlc]
      exact h
  | returned rv =>
      simp only [FocusTyped] at h ⊢
      rw [hk, hk', hlc]
      exact h
  | trapped trap span => trivial
  | halted => trivial

/-- `FrameMatches` is preserved by a parameter write of the context type. -/
theorem FrameMatches.setParams {u : HirUnit} {f : Frame} {i : Nat} {v : Value}
    (hm : FrameMatches u f)
    (hlookup : (frameCtx u f.kind).lookup (.param i) = some v.ty)
    (hvwf : v.WellFormed) :
    FrameMatches u { f with params := f.params.set i v } := by
  cases hk : f.kind with
  | program =>
      simp only [FrameMatches, hk] at hm
      rcases hm with ⟨hparams, _, _, _⟩
      rw [hk] at hlookup
      simp only [Ctx.lookup] at hlookup
      rw [frameCtx_params_program] at hlookup
      cases hlookup
  | function index =>
      simp only [FrameMatches, hk] at hm
      rcases hm with ⟨fn, hfn, hplen, hpar, hllen, hloc, hres⟩
      rw [hk] at hlookup
      simp only [Ctx.lookup] at hlookup
      rw [frameCtx_params_function hfn] at hlookup
      obtain ⟨p, hp, hpt⟩ := List.getElem?_map_eq_some hlookup
      have hiMap : i < (fn.params.map (·.ty)).length := List.getElem?_isSome hlookup
      have hi : i < fn.params.length := by simpa using hiMap
      have hif : i < f.params.length := by omega
      refine ⟨fn, hfn, ?_, ?_, hllen, hloc, hres⟩
      · simp [List.length_set, hplen]
      · intro j w hw
        by_cases hji : i = j
        · subst hji
          rw [List.getElem?_set_self hif] at hw
          simp only [Option.some.injEq] at hw
          subst hw
          exact ⟨p, hp, hpt.symm, hvwf⟩
        · rw [List.getElem?_set_ne hji] at hw
          exact hpar j w hw

/-- `FrameMatches` is preserved by a local write of the context type. -/
theorem FrameMatches.setLocals {u : HirUnit} {f : Frame} {i : Nat} {v : Value}
    (hm : FrameMatches u f)
    (hlookup : (frameCtx u f.kind).lookup (.local i) = some v.ty)
    (hvwf : v.WellFormed) :
    FrameMatches u { f with locals := f.locals.set i v } := by
  cases hk : f.kind with
  | program =>
      simp only [FrameMatches, hk] at hm
      rcases hm with ⟨hparams, hresult, hlen, hloc⟩
      rw [hk] at hlookup
      simp only [Ctx.lookup] at hlookup
      rw [frameCtx_locals_program] at hlookup
      obtain ⟨l, hl, hlt⟩ := List.getElem?_map_eq_some hlookup
      have hiMap : i < (u.program.locals.map (·.ty)).length := List.getElem?_isSome hlookup
      have hi : i < u.program.locals.length := by simpa using hiMap
      have hif : i < f.locals.length := by omega
      refine ⟨hparams, hresult, ?_, ?_⟩
      · simp [List.length_set, hlen]
      · intro j w hw
        by_cases hji : i = j
        · subst hji
          rw [List.getElem?_set_self hif] at hw
          simp only [Option.some.injEq] at hw
          subst hw
          exact ⟨l, hl, hlt.symm, hvwf⟩
        · rw [List.getElem?_set_ne hji] at hw
          exact hloc j w hw
  | function index =>
      simp only [FrameMatches, hk] at hm
      rcases hm with ⟨fn, hfn, hplen, hpar, hllen, hloc, hres⟩
      rw [hk] at hlookup
      simp only [Ctx.lookup] at hlookup
      rw [frameCtx_locals_function hfn] at hlookup
      obtain ⟨l, hl, hlt⟩ := List.getElem?_map_eq_some hlookup
      have hiMap : i < (fn.locals.map (·.ty)).length := List.getElem?_isSome hlookup
      have hi : i < fn.locals.length := by simpa using hiMap
      have hif : i < f.locals.length := by omega
      refine ⟨fn, hfn, hplen, hpar, ?_, ?_, hres⟩
      · simp [List.length_set, hllen]
      · intro j w hw
        by_cases hji : i = j
        · subst hji
          rw [List.getElem?_set_self hif] at hw
          simp only [Option.some.injEq] at hw
          subst hw
          exact ⟨l, hl, hlt.symm, hvwf⟩
        · rw [List.getElem?_set_ne hji] at hw
          exact hloc j w hw

/-- `FrameMatches` is preserved by a result write of the context type. -/
theorem FrameMatches.setResult {u : HirUnit} {f : Frame} {v : Value}
    (hm : FrameMatches u f)
    (hlookup : (frameCtx u f.kind).lookup .result = some v.ty)
    (hvwf : v.WellFormed) :
    FrameMatches u { f with result := some v } := by
  cases hk : f.kind with
  | program =>
      simp only [FrameMatches, hk] at hm
      rw [hk] at hlookup
      simp only [Ctx.lookup] at hlookup
      rw [frameCtx_result_program] at hlookup
      cases hlookup
  | function index =>
      simp only [FrameMatches, hk] at hm
      rcases hm with ⟨fn, hfn, hplen, hpar, hllen, hloc, hres⟩
      rw [hk] at hlookup
      simp only [Ctx.lookup] at hlookup
      rw [frameCtx_result_function hfn] at hlookup
      refine ⟨fn, hfn, hplen, hpar, hllen, hloc, ?_⟩
      rw [hlookup]
      exact ⟨rfl, hvwf⟩

/-- `GlobalsTyped` is preserved by a global write of the declared type. -/
theorem GlobalsTyped.setGlobal {u : HirUnit} {globals : List Value} {i : Nat}
    {v : Value} (hg : GlobalsTyped u globals)
    (hv : ∃ g, u.globals[i]? = some g ∧ v.ty = g.ty)
    (hvwf : v.WellFormed) :
    GlobalsTyped u (globals.set i v) := by
  rcases hg with ⟨hlen, hok⟩
  rcases hv with ⟨g, hgi, hty⟩
  have hi : i < u.globals.length := List.getElem?_isSome hgi
  have hif : i < globals.length := by omega
  refine ⟨by simp [List.length_set, hlen], ?_⟩
  intro j w hw
  rw [List.getElem?_set] at hw
  by_cases hji : i = j
  · rw [if_pos hji, if_pos hif] at hw
    simp only [Option.some.injEq] at hw
    subst hw
    rw [hji] at hgi
    exact ⟨g, hgi, hty, hvwf⟩
  · simp [hji] at hw
    exact hok j w hw

/-- The caller-expectation link is unchanged when the head frame's kind does
not change. -/
theorem CallerExpects.updateFrame {u : HirUnit} {f f' : Frame} {g : Frame}
    (h : CallerExpects u f g) (hk : f'.kind = f.kind) : CallerExpects u f' g := by
  rcases h with hnone | ⟨index, fn, ty, hkind, hfn, hres, hexp⟩
  · exact Or.inl hnone
  · exact Or.inr ⟨index, fn, ty, by rw [hk, hkind], hfn, hres, hexp⟩

/-- Changing only the head frame's non-kind fields preserves the caller links. -/
theorem FramesCallerExpects.updateHead {u : HirUnit} {f f' : Frame} {rest : List Frame}
    (h : FramesCallerExpects u (f :: rest)) (hk : f'.kind = f.kind) :
    FramesCallerExpects u (f' :: rest) := by
  cases rest with
  | nil => simp [FramesCallerExpects]
  | cons g gs =>
      simp only [FramesCallerExpects] at h ⊢
      exact ⟨CallerExpects.updateFrame h.1 hk, h.2⟩

/-- A typed write preserves configuration typing. -/
theorem ConfigTyped.setVar {u : HirUnit} {s : State} {f : Frame} {rest : List Frame}
    (hc : ConfigTyped u s) (hf : s.frames = f :: rest)
    {id : VarId} {v : Value}
    (hlookup : (frameCtx u f.kind).lookup id = some v.ty)
    (hvwf : v.WellFormed) :
    ConfigTyped u (s.setVar id v) := by
  have hshape : FramesShape (f :: rest) := by simpa [hf] using hc.shape
  cases id with
  | param i =>
      have hm' : FrameMatches u { f with params := f.params.set i v } :=
        FrameMatches.setParams (hc.headMatch f rest hf) hlookup hvwf
      have hfr : (s.setVar (.param i) v).frames =
          { f with params := f.params.set i v } :: rest := by simp [State.setVar, hf]
      have hfocus : (s.setVar (.param i) v).focus = s.focus := by simp [State.setVar, hf]
      refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
      · simpa [State.setVar, hf] using hc.globals
      · rw [hfr]; exact FramesShape.updateHead hshape rfl
      · intro g gs hg
        rw [hfr] at hg
        simp only [List.cons.injEq] at hg
        rcases hg with ⟨rfl, rfl⟩
        exact hm'
      · intro g gs hg
        rw [hfr] at hg
        simp only [List.cons.injEq] at hg
        rcases hg with ⟨rfl, rfl⟩
        rw [hfocus]
        exact FocusTyped.updateFrame (hc.headFocus f rest hf) rfl rfl rfl
      · intro g gs hg
        rw [hfr] at hg
        simp only [List.cons.injEq] at hg
        rcases hg with ⟨rfl, rfl⟩
        exact hc.tailTyped f rest hf
      · intro g gs hg hd
        rw [hfr] at hg
        simp only [List.cons.injEq] at hg
        rcases hg with ⟨rfl, rfl⟩
        exact hc.headLoop f rest hf hd
      · have hc' : FramesCallerExpects u (f :: rest) := by simpa [hf] using hc.callers
        rw [hfr]
        exact FramesCallerExpects.updateHead hc' rfl
  | «local» i =>
      have hm' : FrameMatches u { f with locals := f.locals.set i v } :=
        FrameMatches.setLocals (hc.headMatch f rest hf) hlookup hvwf
      have hfr : (s.setVar (.local i) v).frames =
          { f with locals := f.locals.set i v } :: rest := by simp [State.setVar, hf]
      have hfocus : (s.setVar (.local i) v).focus = s.focus := by simp [State.setVar, hf]
      refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
      · simpa [State.setVar, hf] using hc.globals
      · rw [hfr]; exact FramesShape.updateHead hshape rfl
      · intro g gs hg
        rw [hfr] at hg
        simp only [List.cons.injEq] at hg
        rcases hg with ⟨rfl, rfl⟩
        exact hm'
      · intro g gs hg
        rw [hfr] at hg
        simp only [List.cons.injEq] at hg
        rcases hg with ⟨rfl, rfl⟩
        rw [hfocus]
        exact FocusTyped.updateFrame (hc.headFocus f rest hf) rfl rfl rfl
      · intro g gs hg
        rw [hfr] at hg
        simp only [List.cons.injEq] at hg
        rcases hg with ⟨rfl, rfl⟩
        exact hc.tailTyped f rest hf
      · intro g gs hg hd
        rw [hfr] at hg
        simp only [List.cons.injEq] at hg
        rcases hg with ⟨rfl, rfl⟩
        exact hc.headLoop f rest hf hd
      · have hc' : FramesCallerExpects u (f :: rest) := by simpa [hf] using hc.callers
        rw [hfr]
        exact FramesCallerExpects.updateHead hc' rfl
  | result =>
      have hm' : FrameMatches u { f with result := some v } :=
        FrameMatches.setResult (hc.headMatch f rest hf) hlookup hvwf
      have hfr : (s.setVar .result v).frames = { f with result := some v } :: rest := by
        simp [State.setVar, hf]
      have hfocus : (s.setVar .result v).focus = s.focus := by simp [State.setVar, hf]
      refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
      · simpa [State.setVar, hf] using hc.globals
      · rw [hfr]; exact FramesShape.updateHead hshape rfl
      · intro g gs hg
        rw [hfr] at hg
        simp only [List.cons.injEq] at hg
        rcases hg with ⟨rfl, rfl⟩
        exact hm'
      · intro g gs hg
        rw [hfr] at hg
        simp only [List.cons.injEq] at hg
        rcases hg with ⟨rfl, rfl⟩
        rw [hfocus]
        exact FocusTyped.updateFrame (hc.headFocus f rest hf) rfl rfl rfl
      · intro g gs hg
        rw [hfr] at hg
        simp only [List.cons.injEq] at hg
        rcases hg with ⟨rfl, rfl⟩
        exact hc.tailTyped f rest hf
      · intro g gs hg hd
        rw [hfr] at hg
        simp only [List.cons.injEq] at hg
        rcases hg with ⟨rfl, rfl⟩
        exact hc.headLoop f rest hf hd
      · have hc' : FramesCallerExpects u (f :: rest) := by simpa [hf] using hc.callers
        rw [hfr]
        exact FramesCallerExpects.updateHead hc' rfl
  | global i =>
      have hm := hc.headMatch f rest hf
      have hg' : GlobalsTyped u (s.globals.set i v) := by
        cases hk : f.kind with
        | program =>
            rw [hk] at hlookup
            simp only [Ctx.lookup] at hlookup
            rw [frameCtx_globals_program] at hlookup
            simp only [Env.ofUnit] at hlookup
            obtain ⟨g, hglob, hty⟩ := List.getElem?_map_eq_some hlookup
            exact GlobalsTyped.setGlobal hc.globals ⟨g, hglob, hty.symm⟩ hvwf
        | function index =>
            simp only [FrameMatches, hk] at hm
            rcases hm with ⟨fn, hfn, _⟩
            rw [hk] at hlookup
            simp only [Ctx.lookup] at hlookup
            rw [frameCtx_globals_function hfn] at hlookup
            simp only [Env.ofUnit] at hlookup
            obtain ⟨g, hglob, hty⟩ := List.getElem?_map_eq_some hlookup
            exact GlobalsTyped.setGlobal hc.globals ⟨g, hglob, hty.symm⟩ hvwf
      refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
      · simpa [State.setVar, hf] using hg'
      · simpa [State.setVar, hf] using hc.shape
      · intro g gs hg
        have hgold : s.frames = g :: gs := by simpa [State.setVar, hf] using hg
        simpa [State.setVar, hf] using hc.headMatch g gs hgold
      · intro g gs hg
        have hgold : s.frames = g :: gs := by simpa [State.setVar, hf] using hg
        simpa [State.setVar, hf] using hc.headFocus g gs hgold
      · intro g gs hg
        have hgold : s.frames = g :: gs := by simpa [State.setVar, hf] using hg
        simpa [State.setVar, hf] using hc.tailTyped g gs hgold
      · intro g gs hg hd
        have hgold : s.frames = g :: gs := by simpa [State.setVar, hf] using hg
        exact hc.headLoop g gs hgold hd
      · simpa [State.setVar, hf] using hc.callers

/-- A typed global write preserves configuration typing when the written type
matches the declared global type in the given frame's context. -/
theorem ConfigTyped.setGlobal {u : HirUnit} {s : State} {g : Frame} {i : Nat} {v : Value}
    (hc : ConfigTyped u s) (hfm : FrameMatches u g)
    (hlookup : (frameCtx u g.kind).lookup (.global i) = some v.ty)
    (hvwf : v.WellFormed) :
    ConfigTyped u { s with globals := s.globals.set i v } := by
  have hg' : GlobalsTyped u (s.globals.set i v) := by
    cases hk : g.kind with
    | program =>
        rw [hk] at hlookup
        simp only [Ctx.lookup] at hlookup
        rw [frameCtx_globals_program] at hlookup
        simp only [Env.ofUnit] at hlookup
        obtain ⟨glob, hglob, hty⟩ := List.getElem?_map_eq_some hlookup
        exact GlobalsTyped.setGlobal hc.globals ⟨glob, hglob, hty.symm⟩ hvwf
    | function index =>
        simp only [FrameMatches, hk] at hfm
        rcases hfm with ⟨fn, hfn, _⟩
        rw [hk] at hlookup
        simp only [Ctx.lookup] at hlookup
        rw [frameCtx_globals_function hfn] at hlookup
        simp only [Env.ofUnit] at hlookup
        obtain ⟨glob, hglob, hty⟩ := List.getElem?_map_eq_some hlookup
        exact GlobalsTyped.setGlobal hc.globals ⟨glob, hglob, hty.symm⟩ hvwf
  refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
  · simpa using hg'
  · simpa using hc.shape
  · intro f rest hf
    simpa using hc.headMatch f rest hf
  · intro f rest hf
    simpa using hc.headFocus f rest hf
  · intro f rest hf
    simpa using hc.tailTyped f rest hf
  · intro f rest hf hd
    exact hc.headLoop f rest hf hd
  · simpa using hc.callers

/-- A binary operator's well-typedness forces equal operand types. -/
theorem binaryWellTyped_eq {op : BinaryOp} {a b : Ty}
    (h : binaryWellTyped op a b = true) : a = b := by
  cases op <;> simp [binaryWellTyped] at h ⊢
  all_goals first | exact h.1 | exact h

/-- `FrameMatches` ignores the continuation stack. -/
theorem FrameMatches.updateK {u : HirUnit} {f : Frame} {k : List Cont}
    (h : FrameMatches u f) : FrameMatches u { f with k := k } := by
  cases hk : f.kind with
  | program => simpa [FrameMatches, hk] using h
  | function index => simpa [FrameMatches, hk] using h

/-- `FrameMatches` ignores the loop context. -/
theorem FrameMatches.updateLoopCtx {u : HirUnit} {f : Frame} {lc : LoopCtx}
    (h : FrameMatches u f) : FrameMatches u { f with loopCtx := lc } := by
  cases hk : f.kind with
  | program => simpa [FrameMatches, hk] using h
  | function index => simpa [FrameMatches, hk] using h

/-- `FrameMatches` ignores the continuation stack and the loop context. -/
theorem FrameMatches.updateKLoop {u : HirUnit} {f : Frame} {k : List Cont} {lc : LoopCtx}
    (h : FrameMatches u f) : FrameMatches u { f with k := k, loopCtx := lc } := by
  cases hk : f.kind with
  | program => simpa [FrameMatches, hk] using h
  | function index => simpa [FrameMatches, hk] using h

/-- `FrameMatches` ignores the caller expectation and the continuation stack. -/
theorem FrameMatches.updateKExpect {u : HirUnit} {f : Frame} {k : List Cont} {e : Option Ty}
    (h : FrameMatches u f) : FrameMatches u { f with k := k, expect := e } := by
  cases hk : f.kind with
  | program => simpa [FrameMatches, hk] using h
  | function index => simpa [FrameMatches, hk] using h

/-- Replacing the head frame's continuation stack and the focus preserves
configuration typing. -/
theorem ConfigTyped.updateHeadFocus {u : HirUnit} {s : State} {f : Frame}
    {rest : List Frame} {focus : Focus} {k : List Cont}
    (hc : ConfigTyped u s) (hf : s.frames = f :: rest)
    (hfocus : FocusTyped u { f with k := k } focus)
    (hloop : f.loopCtx.depth > 0 → StackHasLoop k) :
    ConfigTyped u { s with frames := { f with k := k } :: rest, focus := focus } := by
  have hshape : FramesShape (f :: rest) := by simpa [hf] using hc.shape
  have hframe : FrameMatches u { f with k := k } :=
    FrameMatches.updateK (hc.headMatch f rest hf)
  refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
  · simpa using hc.globals
  · exact FramesShape.updateHead hshape rfl
  · intro g gs hg
    simp only [List.cons.injEq] at hg
    rcases hg with ⟨rfl, rfl⟩
    exact hframe
  · intro g gs hg
    simp only [List.cons.injEq] at hg
    rcases hg with ⟨rfl, rfl⟩
    exact hfocus
  · intro g gs hg
    simp only [List.cons.injEq] at hg
    rcases hg with ⟨rfl, rfl⟩
    exact hc.tailTyped f rest hf
  · intro g gs hg hd
    simp only [List.cons.injEq] at hg
    rcases hg with ⟨rfl, rfl⟩
    exact hloop hd
  · have hc' : FramesCallerExpects u (f :: rest) := by simpa [hf] using hc.callers
    simpa using FramesCallerExpects.updateHead hc' (f' := { f with k := k }) rfl

/-- Replace the head frame and the focus, preserving configuration typing. -/
theorem ConfigTyped.updateHeadFrame {u : HirUnit} {s : State} {f : Frame} {rest : List Frame}
    {f' : Frame} {focus : Focus} (hc : ConfigTyped u s) (hf : s.frames = f :: rest)
    (hframe : FrameMatches u f') (hfocus : FocusTyped u f' focus)
    (hshape : FramesShape (f' :: rest))
    (hloop : f'.loopCtx.depth > 0 → StackHasLoop f'.k)
    (hkind : f'.kind = f.kind) :
    ConfigTyped u { s with frames := f' :: rest, focus := focus } := by
  refine ⟨?_, hshape, ?_, ?_, ?_, ?_, ?_⟩
  · simpa using hc.globals
  · intro g gs hg
    simp only [List.cons.injEq] at hg
    rcases hg with ⟨rfl, rfl⟩
    exact hframe
  · intro g gs hg
    simp only [List.cons.injEq] at hg
    rcases hg with ⟨rfl, rfl⟩
    exact hfocus
  · intro g gs hg
    simp only [List.cons.injEq] at hg
    rcases hg with ⟨rfl, rfl⟩
    exact hc.tailTyped f rest hf
  · intro g gs hg hd
    simp only [List.cons.injEq] at hg
    rcases hg with ⟨rfl, rfl⟩
    exact hloop hd
  · have hc' : FramesCallerExpects u (f :: rest) := by simpa [hf] using hc.callers
    simpa using FramesCallerExpects.updateHead hc' hkind

/-- Replace the focus while keeping the frames, preserving configuration
typing. -/
theorem ConfigTyped.updateFocus {u : HirUnit} {s : State} {f : Frame} {rest : List Frame}
    {focus : Focus} (hc : ConfigTyped u s) (hf : s.frames = f :: rest)
    (hfocus : FocusTyped u f focus) :
    ConfigTyped u { s with focus := focus } := by
  have hshape : FramesShape (f :: rest) := by simpa [hf] using hc.shape
  refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
  · simpa using hc.globals
  · simpa using hc.shape
  · intro g gs hg
    have hg' : s.frames = g :: gs := by simpa using hg
    simpa using hc.headMatch g gs hg'
  · intro g gs hg
    have hg' : s.frames = g :: gs := by simpa using hg
    rw [hf] at hg'
    simp only [List.cons.injEq] at hg'
    rcases hg' with ⟨rfl, rfl⟩
    simpa using hfocus
  · intro g gs hg
    have hg' : s.frames = g :: gs := by simpa using hg
    simpa using hc.tailTyped g gs hg'
  · intro g gs hg hd
    have hg' : s.frames = g :: gs := by simpa using hg
    exact hc.headLoop g gs hg' hd
  · simpa using hc.callers

/-- Configuration typing is independent of the cycle counters. -/
theorem ConfigTyped.updateCycles {u : HirUnit} {s : State} (hc : ConfigTyped u s)
    (cycles cyclesLeft : Nat) :
    ConfigTyped u { s with cycles := cycles, cyclesLeft := cyclesLeft } := by
  refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
  · simpa using hc.globals
  · simpa using hc.shape
  · intro f rest hf; simpa using hc.headMatch f rest hf
  · intro f rest hf; simpa using hc.headFocus f rest hf
  · intro f rest hf; simpa using hc.tailTyped f rest hf
  · intro f rest hf hd; simpa using hc.headLoop f rest hf hd
  · simpa using hc.callers

/-- `FrameMatches` ignores the caller expectation. -/
theorem FrameMatches.updateExpect {u : HirUnit} {f : Frame} {e : Option Ty}
    (h : FrameMatches u f) : FrameMatches u { f with expect := e } := by
  cases hk : f.kind with
  | program => simpa [FrameMatches, hk] using h
  | function index => simpa [FrameMatches, hk] using h

/-- A `Forall2` with an empty left list has an empty right list. -/
theorem Forall2.nil_left {α β : Type} {R : α → β → Prop} {bs : List β}
    (h : Forall2 R ([] : List α) bs) : bs = [] := by
  cases h
  rfl

/-- Changing only a head frame's continuation stack preserves the shape. -/
theorem FramesShape.updateHeadK {f : Frame} {rest : List Frame}
    (h : FramesShape (f :: rest)) (k : List Cont) :
    FramesShape ({ f with k := k } :: rest) :=
  FramesShape.updateHead h rfl

/-- Changing only a head frame's expectation preserves the shape. -/
theorem FramesShape.updateHeadExpect {f : Frame} {rest : List Frame} {e : Option Ty}
    (h : FramesShape (f :: rest)) :
    FramesShape ({ f with expect := e } :: rest) :=
  FramesShape.updateHead h rfl

/-- Pushing a function frame preserves the call-stack shape. -/
theorem FramesShape.push {callee caller : Frame} {rest : List Frame} {index : Nat}
    (h : FramesShape (caller :: rest)) (hk : callee.kind = .function index) :
    FramesShape (callee :: caller :: rest) :=
  FramesShape.cons ⟨index, hk⟩ h

/-- Push a callee frame above a suspended caller, preserving configuration
typing. -/
theorem ConfigTyped.pushFrame {u : HirUnit} {s : State} {f : Frame} {rest : List Frame}
    {callee caller : Frame} {focus : Focus} (hc : ConfigTyped u s)
    (hf : s.frames = f :: rest) (hcallee : FrameMatches u callee)
    (hcaller : FrameTyped u caller) (hshape : FramesShape (callee :: caller :: rest))
    (hfocus : FocusTyped u callee focus)
    (hcalleeLoop : callee.loopCtx.depth > 0 → StackHasLoop callee.k)
    (hcallerKind : caller.kind = f.kind)
    (hcallerExpect : CallerExpects u callee caller) :
    ConfigTyped u { s with frames := callee :: caller :: rest, focus := focus } := by
  have htail : FramesTailTyped u rest := hc.tailTyped f rest hf
  refine ⟨?_, hshape, ?_, ?_, ?_, ?_, ?_⟩
  · simpa using hc.globals
  · intro g gs hg
    simp only [List.cons.injEq] at hg
    rcases hg with ⟨rfl, rfl⟩
    exact hcallee
  · intro g gs hg
    simp only [List.cons.injEq] at hg
    rcases hg with ⟨rfl, rfl⟩
    exact hfocus
  · intro g gs hg
    simp only [List.cons.injEq] at hg
    rcases hg with ⟨rfl, rfl⟩
    exact ⟨hcaller, htail⟩
  · intro g gs hg hd
    simp only [List.cons.injEq] at hg
    rcases hg with ⟨rfl, rfl⟩
    exact hcalleeLoop hd
  · have hc' : FramesCallerExpects u (f :: rest) := by simpa [hf] using hc.callers
    exact ⟨hcallerExpect, FramesCallerExpects.updateHead hc' hcallerKind⟩

/-- `binaryResultTy` at equal operand types is the comparison result or the
operand type. -/
theorem binaryResultTy_eq (op : BinaryOp) (ty : Ty) :
    binaryResultTy op ty ty = if op.isComparison then .bool else ty := by
  cases op <;> rfl

/-- Local initializer statements are typed when the context's local types match
the declarations. -/
theorem localInitStmtsFrom_typed {ctx : Ctx} (index : Nat) (locals : List Local)
    (hlocals : ctx.locals.drop index = locals.map (·.ty))
    (h : ∀ loc ∈ locals, ∀ expr, loc.init = some expr →
      ExprHasType ctx expr loc.ty) :
    StmtListHasType ctx 0 [] (localInitStmtsFrom index locals) := by
  induction locals generalizing index with
  | nil => simp [localInitStmtsFrom, StmtListHasType]
  | cons loc rest ih =>
      cases hinit : loc.init with
      | none =>
          simp only [localInitStmtsFrom, hinit, List.nil_append]
          apply ih (index + 1)
          · have h0 : ctx.locals.drop index = loc.ty :: rest.map (·.ty) := by
              rw [hlocals, List.map_cons]
            rw [← List.drop_drop (i := 1) (j := index), h0]
            rfl
          · intro loc' hmem expr hexpr
            exact h loc' (List.mem_cons_of_mem _ hmem) expr hexpr
      | some e =>
          simp only [localInitStmtsFrom, hinit, List.singleton_append]
          intro stmt hstmt
          rcases List.mem_cons.mp hstmt with hhead | htail
          · subst hhead
            apply StmtHasType.assign
            · simp
            · simp only [Ctx.lookup]
              have h0 : ctx.locals.drop index = loc.ty :: rest.map (·.ty) := by
                rw [hlocals, List.map_cons]
              have hget : ctx.locals[index]? = some loc.ty := by
                have := congrArg (fun l => l[0]?) h0
                simpa [List.getElem?_drop] using this
              simpa using hget
            · exact h loc (List.mem_cons_self ..) e hinit
          · apply ih (index + 1)
            · have h0 : ctx.locals.drop index = loc.ty :: rest.map (·.ty) := by
                rw [hlocals, List.map_cons]
              rw [← List.drop_drop (i := 1) (j := index), h0]
              rfl
            · intro loc' hmem expr hexpr
              exact h loc' (List.mem_cons_of_mem _ hmem) expr hexpr
            · exact htail

/-- Global initializer statements are typed when the context's global types
match the declarations. -/
theorem globalInitStmtsFrom_typed {ctx : Ctx} (index : Nat) (globals : List Global)
    (hglobals : ctx.globals.drop index = globals.map (·.ty))
    (h : ∀ glob ∈ globals, ∀ expr, glob.init = some expr →
      ExprHasType ctx expr glob.ty) :
    StmtListHasType ctx 0 [] (globalInitStmtsFrom index globals) := by
  induction globals generalizing index with
  | nil => simp [globalInitStmtsFrom, StmtListHasType]
  | cons glob rest ih =>
      cases hinit : glob.init with
      | none =>
          simp only [globalInitStmtsFrom, hinit, List.nil_append]
          apply ih (index + 1)
          · have h0 : ctx.globals.drop index = glob.ty :: rest.map (·.ty) := by
              rw [hglobals, List.map_cons]
            rw [← List.drop_drop (i := 1) (j := index), h0]
            rfl
          · intro glob' hmem expr hexpr
            exact h glob' (List.mem_cons_of_mem _ hmem) expr hexpr
      | some e =>
          simp only [globalInitStmtsFrom, hinit, List.singleton_append]
          intro stmt hstmt
          rcases List.mem_cons.mp hstmt with hhead | htail
          · subst hhead
            apply StmtHasType.assign
            · simp
            · simp only [Ctx.lookup]
              have h0 : ctx.globals.drop index = glob.ty :: rest.map (·.ty) := by
                rw [hglobals, List.map_cons]
              have hget : ctx.globals[index]? = some glob.ty := by
                have := congrArg (fun l => l[0]?) h0
                simpa [List.getElem?_drop] using this
              simpa using hget
            · exact h glob (List.mem_cons_self ..) e hinit
          · apply ih (index + 1)
            · have h0 : ctx.globals.drop index = glob.ty :: rest.map (·.ty) := by
                rw [hglobals, List.map_cons]
              rw [← List.drop_drop (i := 1) (j := index), h0]
              rfl
            · intro glob' hmem expr hexpr
              exact h glob' (List.mem_cons_of_mem _ hmem) expr hexpr
            · exact htail

/-- Pushing the first call argument preserves stack typing. -/
theorem callArgsPush {ctx : Ctx} {callee : Callee} {arg : Expr} {more : List Expr}
    {T : Ty} {k : List Cont} {lc : LoopCtx} {span : Span}
    (hcall : CallHasType ctx callee (arg :: more) (some T))
    (hstack : StackTyped ctx k (.value T) lc) :
    ∃ t, ExprHasType ctx arg t ∧
      StackTyped ctx
        (Cont.same (.callArgs callee [] (arg :: more) false) lc span :: k)
        (.value t) lc := by
  rcases callHasType_cases hcall with
    ⟨idx, cspan, sig, hcallee, hsigfn, hres, hargs⟩ |
    ⟨fn, cspan, ty, hcallee, hresOuter, hstd⟩
  · subst hcallee
    cases hsp : sig.params with
    | nil => rw [hsp] at hargs; cases hargs
    | cons p ps =>
        rw [hsp] at hargs
        cases hargs with
        | cons harg htail =>
            refine ⟨p, harg, ?_⟩
            refine StackTyped.callArgs (ts := ps) (result := some T) (by simp) ?_ (by simp) ?_ (by simp) ?_
            · exact Forall2.cons harg htail
            · refine ⟨sig, hsigfn, ?_, ?_⟩
              · simp [hsp]
              · exact hres.symm
            · simpa [Expect.ofOption] using hstack
  · subst hcallee
    have hty : ty = T := by cases hresOuter; rfl
    cases hstd with
    | @mk _ _ argTys _ hargs hres =>
        cases ht : argTys with
        | nil => rw [ht] at hargs; cases hargs
        | cons p ps =>
            rw [ht] at hargs
            cases hargs with
            | cons harg htail =>
                refine ⟨p, harg, ?_⟩
                refine StackTyped.callArgs (ts := ps) (result := some T) (by simp) ?_ (by simp) ?_ (by simp) ?_
                · exact Forall2.cons harg htail
                · simpa [CalleeSignatureV, ht, hty] using hres
                · simpa [Expect.ofOption] using hstack

/-- Pushing the first call argument of a statement call preserves stack typing. -/
theorem callArgsPushStmt {ctx : Ctx} {callee : Callee} {arg : Expr} {more : List Expr}
    {result : Option Ty} {k : List Cont} {lc : LoopCtx} {span : Span}
    (hcall : CallHasType ctx callee (arg :: more) result)
    (hrest : StackTyped ctx k .unit lc) :
    ∃ t, ExprHasType ctx arg t ∧
      StackTyped ctx
        (Cont.same (.callArgs callee [] (arg :: more) true) lc span :: k) (.value t) lc := by
  rcases callHasType_cases hcall with
    ⟨idx, cspan, sig, hcallee, hsigfn, hres, hargs⟩ |
    ⟨fn, cspan, ty, hcallee, hresOuter, hstd⟩
  · subst hcallee
    cases hsp : sig.params with
    | nil => rw [hsp] at hargs; cases hargs
    | cons p ps =>
        rw [hsp] at hargs
        cases hargs with
        | cons harg htail =>
            refine ⟨p, harg, ?_⟩
            refine StackTyped.callArgs (ts := ps) (result := result) (by simp) ?_ (by simp) ?_ (by simp) ?_
            · exact Forall2.cons harg htail
            · refine ⟨sig, hsigfn, ?_, ?_⟩
              · simp [hsp]
              · exact hres.symm
            · simpa using hrest
  · subst hcallee
    rcases stdFnHasType_cases hstd with ⟨argTys, hargs, hres⟩
    cases ht : argTys with
    | nil => rw [ht] at hargs; cases hargs
    | cons p ps =>
        rw [ht] at hargs
        cases hargs with
        | cons harg htail =>
            refine ⟨p, harg, ?_⟩
            refine StackTyped.callArgs (ts := ps) (result := some ty) (by simp) ?_ (by simp) ?_ (by simp) ?_
            · exact Forall2.cons harg htail
            · simpa [CalleeSignatureV, ht] using hres
            · simpa using hrest

/-! ## Progress support -/

/-- A function signature in a frame's context resolves to a unit function. -/
theorem frameCtx_functions_lookup {u : HirUnit} {f : Frame} {index : Nat}
    {sig : Signature} (hfm : FrameMatches u f)
    (hsig : (frameCtx u f.kind).functions[index]? = some sig) :
    ∃ fn, u.functions[index]? = some fn := by
  have hctxf : (frameCtx u f.kind).functions = (Env.ofUnit u).functions := by
    cases hk : f.kind with
    | program => simpa [hk] using frameCtx_functions_program u
    | function i =>
        simp only [FrameMatches, hk] at hfm
        rcases hfm with ⟨fn, hfn, _⟩
        exact frameCtx_functions_ofFunction hfn
  exact Env.ofUnit_functions_getElem? (by rwa [hctxf] at hsig)

/-- A signature in a frame's context is the environment signature of a unit
function. -/
theorem frameCtx_functions_signature {u : HirUnit} {f : Frame} {index : Nat}
    {sig : Signature} (hfm : FrameMatches u f)
    (hsig : (frameCtx u f.kind).functions[index]? = some sig) :
    ∃ fn, u.functions[index]? = some fn ∧
      sig = ⟨fn.name, fn.params.map (·.ty), fn.resultType, fn.span⟩ := by
  have hctxf : (frameCtx u f.kind).functions = (Env.ofUnit u).functions := by
    cases hk : f.kind with
    | program => simpa [hk] using frameCtx_functions_program u
    | function i =>
        simp only [FrameMatches, hk] at hfm
        rcases hfm with ⟨fn, hfn, _⟩
        exact frameCtx_functions_ofFunction hfn
  have hsig' : (Env.ofUnit u).functions[index]? = some sig := by rwa [hctxf] at hsig
  obtain ⟨fn, hfn⟩ := Env.ofUnit_functions_getElem? hsig'
  have hsigfn : (Env.ofUnit u).functions[index]? =
      some ⟨fn.name, fn.params.map (·.ty), fn.resultType, fn.span⟩ := by
    simp [Env.ofUnit, hfn]
  rw [hsigfn] at hsig'
  cases hsig'
  exact ⟨fn, hfn, rfl⟩

/-- The frame built by `performCall` matches its function declaration. -/
theorem calleeFrameMatches {u : HirUnit} {index : Nat} {fn : Function} {args : List Value}
    (hfn : u.functions[index]? = some fn)
    (hparamsLen : args.length = fn.params.length)
    (hparams : ∀ (i : Nat) (v : Value), args[i]? = some v →
      ∃ p : Param, fn.params[i]? = some p ∧ v.ty = p.ty ∧ v.WellFormed)
    (hcore : FunctionWellFormed fn) :
    FrameMatches u
      { kind := .function index, params := args,
        locals := fn.locals.map (fun l => Value.default l.ty),
        result := fn.resultType.map Value.default, k := [], expect := none,
        loopCtx := LoopCtx.top } := by
  refine ⟨fn, hfn, hparamsLen, hparams, ?_, ?_, ?_⟩
  · simp
  · intro i v hv
    obtain ⟨l, hl, hlv⟩ := List.getElem?_map_eq_some hv
    refine ⟨l, hl, ?_, ?_⟩
    · rw [← hlv]
      rfl
    · rw [← hlv]
      exact Value.default_wellFormed (hcore.locals l (List.mem_of_getElem? hl))
  · cases hrt : fn.resultType with
    | none => simp [hrt]
    | some ty =>
        simp [hrt]
        exact ⟨rfl, Value.default_wellFormed (hcore.result ty hrt)⟩

/-- A typed expression focus never gets stuck. -/
theorem stepExpr_not_stuck {u : HirUnit} {c : State} {f : Frame} {rest : List Frame}
    {e : Expr} (hf : c.frames = f :: rest) (hctx : ConfigTyped u c)
    (hfm : FrameMatches u f)
    (h : ∃ T, ExprHasType (frameCtx u f.kind) e T ∧
        StackTyped (frameCtx u f.kind) f.k (.value T) f.loopCtx) :
    stepExpr u c e ≠ .stuck := by
  simp only [stepExpr, hf]
  cases e with
  | literal lit span => simp
  | var id span =>
      rcases h with ⟨T, ht, _⟩
      cases ht with
      | var hlookup =>
          obtain ⟨v, hv⟩ := ConfigTyped.lookupVar?_some hctx hf hlookup
          simp [hv]
  | unary op operand span => simp
  | binary op lhs rhs span => simp
  | call callee args span =>
      cases args with
      | nil =>
          cases callee with
          | std fn sp =>
              simp only [performCall]
              split <;> simp
          | function index sp =>
              rcases h with ⟨T, ht, _⟩
              cases ht with
              | call hcall =>
                  rcases callHasType_cases hcall with
                    ⟨idx, cspan, sig, hcallee, hsig, hres, hargs⟩ |
                    ⟨fn, cspan, ty, hcallee, hres, hstd⟩
                  · cases hcallee
                    obtain ⟨fn, hfn⟩ := frameCtx_functions_lookup hfm hsig
                    simp [performCall, hfn, hf]
                  · cases hcallee
      | cons arg more => simp
  | cast operand to kind span => simp

/-- A non-marker continuation does not hide a loop marker above it. -/
theorem StackHasLoop.tail_of_not_marker {cont : Cont} {rest : List Cont}
    (h : StackHasLoop (cont :: rest)) (hm : cont.isLoopMarker = false) :
    StackHasLoop rest := by
  cases h with
  | inl hh => rw [hm] at hh; cases hh
  | inr hh => exact hh

/-- A trapped focus is typed. -/
theorem FocusTyped.trapped (u : HirUnit) (f : Frame) (trap : Trap) (span : Span) :
    FocusTyped u f (.trapped trap span) :=
  trivial

/-- A halted focus is typed. -/
theorem FocusTyped.halted (u : HirUnit) (f : Frame) : FocusTyped u f .halted :=
  trivial

/-- A well-formed value with a typed value stack is a typed value focus. -/
theorem FocusTyped.value_of {u : HirUnit} {f : Frame} {v : Value}
    (hstack : StackTyped (frameCtx u f.kind) f.k (.value v.ty) f.loopCtx)
    (hvwf : v.WellFormed) : FocusTyped u f (.value v) :=
  ⟨hvwf, v.ty, rfl, hstack⟩

/-- A typed unit stack is a typed resume focus. -/
theorem FocusTyped.resume_of {u : HirUnit} {f : Frame}
    (hstack : StackTyped (frameCtx u f.kind) f.k .unit f.loopCtx) :
    FocusTyped u f .resume :=
  hstack

/-- The result shape of a frame that matches its declarations. -/
theorem FrameMatches.result_cases {u : HirUnit} {f : Frame} (h : FrameMatches u f) :
    (∃ index fn, f.kind = .function index ∧ u.functions[index]? = some fn ∧
      fn.resultType = none ∧ f.result = none) ∨
    (f.kind = .program ∧ f.result = none) ∨
    (∃ v index fn, f.result = some v ∧ v.WellFormed ∧ f.kind = .function index ∧
      u.functions[index]? = some fn ∧ fn.resultType = some v.ty) := by
  cases hk : f.kind with
  | program =>
      simp only [FrameMatches, hk] at h
      rcases h with ⟨_, hr, _, _⟩
      exact Or.inr (Or.inl ⟨rfl, hr⟩)
  | function index =>
      simp only [FrameMatches, hk] at h
      rcases h with ⟨fn, hfn, _, _, _, _, hres⟩
      cases hrt : fn.resultType with
      | none =>
          cases hr : f.result with
          | none => exact Or.inl ⟨index, fn, rfl, hfn, hrt, rfl⟩
          | some v => simp [hrt, hr] at hres
      | some ty =>
          cases hr : f.result with
          | none => simp [hrt, hr] at hres
          | some v =>
              simp [hrt, hr] at hres
              exact Or.inr (Or.inr ⟨v, index, fn, rfl, hres.2, rfl, hfn,
                by rw [hres.1]; exact hrt⟩)

/-- A returned focus is typed from the callee's declaration and the caller's
expectation link, without knowing the callee's current result value. -/
theorem FocusTyped.returned_of_caller {u : HirUnit} {f g : Frame} {rv : Option Value}
    (hfm : FrameMatches u f) (hft : FrameTyped u g)
    (hlink : CallerExpects u f g) (hrv : f.result = rv) :
    FocusTyped u g (.returned rv) := by
  rcases hft with ⟨_, hstack, _⟩
  rcases FrameMatches.result_cases hfm with
    ⟨index, fn, hkind, hfn, hrtNone, hresNone⟩ |
    ⟨hkindProg, hresNone⟩ |
    ⟨v, index, fn, hresSome, hvwf, hkind, hfn, hrtSome⟩
  · have hrvNone : rv = none := by rw [← hrv, hresNone]
    subst hrvNone
    rcases hlink with hnone | ⟨idx, fn', ty, hkind', hfn', hrt', hexp⟩
    · rw [hnone] at hstack
      simpa [FocusTyped, Expect.ofOption] using hstack
    · exfalso
      rw [hkind] at hkind'
      cases hkind'
      have hfnEq : fn = fn' := by
        rw [hfn] at hfn'
        exact Option.some.inj hfn'
      subst hfnEq
      rw [hrtNone] at hrt'
      cases hrt'
  · have hrvNone : rv = none := by rw [← hrv, hresNone]
    subst hrvNone
    rcases hlink with hnone | ⟨idx, fn', ty, hkind', hfn', hrt', hexp⟩
    · rw [hnone] at hstack
      simpa [FocusTyped, Expect.ofOption] using hstack
    · exfalso
      rw [hkindProg] at hkind'
      cases hkind'
  · have hrvSome : rv = some v := by rw [← hrv, hresSome]
    subst hrvSome
    rcases hlink with hnone | ⟨idx, fn', ty, hkind', hfn', hrt', hexp⟩
    · rw [hnone] at hstack
      refine ⟨hvwf, Or.inr ?_⟩
      simpa [Expect.ofOption] using hstack
    · rw [hkind] at hkind'
      cases hkind'
      have hfnEq : fn = fn' := by
        rw [hfn] at hfn'
        exact Option.some.inj hfn'
      subst hfnEq
      rw [hrtSome] at hrt'
      have htyEq : v.ty = ty := Option.some.inj hrt'
      refine ⟨hvwf, Or.inl ?_⟩
      rw [hexp] at hstack
      simp only [Expect.ofOption] at hstack
      rw [← htyEq] at hstack
      exact hstack

/-- A typed statement list with a loop marker is a typed statement focus. -/
theorem FocusTyped.stmts_of {u : HirUnit} {f : Frame} {ss : List Stmt} {lc : LoopCtx}
    (hlc : f.loopCtx = lc) (hstack : StackTyped (frameCtx u f.kind) f.k .unit lc)
    (hstmts : StmtListHasType (frameCtx u f.kind) lc.depth lc.controlled ss)
    (hloop : lc.depth > 0 → StackHasLoop f.k) : FocusTyped u f (.stmts ss lc) :=
  ⟨hlc, hstack, hstmts, hloop⟩

/-- A standard-function result type is one of the argument types. -/
theorem stdResult?_mem_argTys {fn : StdFn} {argTys : List Ty} {ty : Ty}
    (h : stdResult? fn argTys = some ty) : ty ∈ argTys := by
  revert ty
  cases fn <;> cases argTys with
  | nil => intro ty h; simp [stdResult?] at h
  | cons a rest =>
      cases rest with
      | nil =>
          intro ty h; simp only [stdResult?] at h
          first | simp_all | (split at h <;> simp_all)
      | cons b rest2 =>
          cases rest2 with
          | nil =>
              intro ty h; simp only [stdResult?] at h
              first | simp_all | (split at h <;> simp_all)
          | cons c rest3 =>
              cases rest3 with
              | nil =>
                  intro ty h; simp only [stdResult?] at h
                  first | simp_all | (split at h <;> simp_all)
              | cons d rest4 =>
                  intro ty h; simp only [stdResult?] at h
                  first | simp_all | (split at h <;> simp_all)

/-- A standard-function result type is core when every argument type is. -/
theorem stdResult?_isCore {fn : StdFn} {argTys : List Ty} {ty : Ty}
    (hcore : ∀ t ∈ argTys, t.isCore = true)
    (h : stdResult? fn argTys = some ty) : ty.isCore = true :=
  hcore ty (stdResult?_mem_argTys h)

/-- Evaluated arguments pair with their runtime types. -/
theorem forall2_map_ty_of_wellFormed {args : List Value}
    (h : ∀ v ∈ args, v.WellFormed) :
    Forall2 (fun (a : Value) (t : Ty) => a.ty = t ∧ a.WellFormed) args (args.map Value.ty) := by
  induction args with
  | nil => exact Forall2.nil
  | cons a rest ih =>
      exact Forall2.cons ⟨rfl, h a (by simp)⟩ (ih (by intro v hv; exact h v (by simp [hv])))

/-- Appending an evaluated argument to a call signature. -/
theorem CalleeSignatureV.push {ctx : Ctx} {callee : Callee} {done : List Value}
    {v : Value} {t : Ty} {ts : List Ty} {result : Option Ty}
    (hsig : CalleeSignatureV ctx callee done (t :: ts) result)
    (hvy : v.ty = t) :
    CalleeSignatureV ctx callee (done ++ [v]) ts result := by
  cases callee with
  | function index sp =>
      simp only [CalleeSignatureV, List.append_nil] at hsig ⊢
      rcases hsig with ⟨sig, hsigfn, hparams, hresSig⟩
      refine ⟨sig, hsigfn, ?_, hresSig⟩
      rw [hparams]
      simp [List.map_append, hvy]
  | std fn sp =>
      simp only [CalleeSignatureV, List.append_nil] at hsig ⊢
      simpa [List.map_append, hvy] using hsig

/-- Performing a call after its arguments are evaluated preserves
configuration typing. -/
theorem performCall_preserves {u : HirUnit} {env : Env} {s c' : State} {f : Frame}
    {fr : List Frame} {caller : Frame} {k : List Cont} {callee : Callee}
    {args : List Value} {isStmt : Bool} {span : Span} {result : Option Ty}
    (hunit : HasType u env) (hc : ConfigTyped u s) (hf : s.frames = f :: fr)
    (hkind : caller.kind = f.kind) (hk : caller.k = k)
    (hcallerMatch : FrameMatches u caller)
    (hcallerLoop : caller.loopCtx.depth > 0 → StackHasLoop caller.k)
    (hsig : CalleeSignatureV (frameCtx u caller.kind) callee args [] result)
    (hresult : isStmt = false → result ≠ none)
    (hargs : ∀ v ∈ args, v.WellFormed)
    (hrest : StackTyped (frameCtx u caller.kind) k
      (if isStmt then .unit else .ofOption result) caller.loopCtx)
    (hs : performCall u { s with frames := caller :: fr } callee args isStmt span = .step c') :
    ConfigTyped u c' := by
  have hshape : FramesShape (caller :: fr) :=
    FramesShape.updateHead (by simpa [hf] using hc.shape) hkind
  cases callee with
  | std fn sp =>
      simp only [CalleeSignatureV, List.append_nil] at hsig
      simp only [performCall] at hs
      generalize hev : evalStd fn args = res at hs
      cases res with
      | error trap =>
          simp at hs
          cases hs
          exact ConfigTyped.updateHeadFrame (f' := caller) hc hf hcallerMatch
            (FocusTyped.trapped u caller trap span) hshape hcallerLoop hkind
      | ok w =>
          simp at hs
          cases isStmt with
          | true =>
              simp at hs
              cases hs
              have hrest' : StackTyped (frameCtx u caller.kind) k .unit caller.loopCtx := by
                simpa using hrest
              exact ConfigTyped.updateHeadFrame (f' := caller) hc hf hcallerMatch
                (FocusTyped.resume_of (by simpa [hk] using hrest')) hshape hcallerLoop hkind
          | false =>
              simp at hs
              cases hs
              obtain ⟨ty0, hty0⟩ : ∃ ty0, result = some ty0 := by
                cases hres : result with
                | none => exact absurd hres (hresult rfl)
                | some ty0 => exact ⟨ty0, rfl⟩
              have hstd : stdResult? fn (args.map Value.ty) = some ty0 := by
                rw [hty0] at hsig
                simpa using hsig
              have hcoreArgs : ∀ t ∈ args.map Value.ty, t.isCore = true := by
                intro t ht
                obtain ⟨a, ha, rfl⟩ := List.mem_map.mp ht
                exact Value.isCore_of_wellFormed (hargs a ha)
              have hcore : ty0.isCore = true := stdResult?_isCore hcoreArgs hstd
              obtain ⟨hwty, hwwf⟩ :=
                evalStd_ok (forall2_map_ty_of_wellFormed hargs) hstd hcore hev
              have hstack : StackTyped (frameCtx u caller.kind) caller.k (.value w.ty)
                  caller.loopCtx := by
                rw [hwty]
                have hof : StackTyped (frameCtx u caller.kind) k
                    (.ofOption (some ty0)) caller.loopCtx := by
                  simpa [hty0] using hrest
                simpa [Expect.ofOption, hk] using hof
              exact ConfigTyped.updateHeadFrame (f' := caller) hc hf hcallerMatch
                (FocusTyped.value_of hstack hwwf) hshape hcallerLoop hkind
  | function index cspan =>
      simp only [CalleeSignatureV, List.append_nil] at hsig
      rcases hsig with ⟨sig, hsigfn, hparams, hresSig⟩
      obtain ⟨fn, hfn, hsigEq⟩ := frameCtx_functions_signature hcallerMatch hsigfn
      rw [hsigEq] at hparams hresSig
      simp only at hparams hresSig
      have hfnmem : fn ∈ u.functions := List.mem_of_getElem? hfn
      have hftyped : FunctionTyped (Env.ofUnit u) fn := by
        have := hunit.functions fn hfnmem
        rwa [hunit.env_eq] at this
      have hwell : FunctionWellFormed fn := hunit.wellformed.functions fn hfnmem
      have hparamsLen : args.length = fn.params.length := by
        have := congrArg List.length hparams
        simp only [List.length_map] at this
        omega
      have hargsparams : ∀ (i : Nat) (v : Value), args[i]? = some v →
          ∃ p : Param, fn.params[i]? = some p ∧ v.ty = p.ty ∧ v.WellFormed := by
        intro i v hv
        have hargsTy : (args.map Value.ty)[i]? = some v.ty := by
          rw [List.getElem?_map, hv]
          rfl
        have hfnTy : (fn.params.map (·.ty))[i]? = some v.ty := by
          rw [hparams]
          exact hargsTy
        obtain ⟨p, hp, hpty⟩ := List.getElem?_map_eq_some hfnTy
        exact ⟨p, hp, hpty.symm, hargs v (List.mem_of_getElem? hv)⟩
      let calleeFrame : Frame :=
        { kind := .function index, params := args,
          locals := fn.locals.map (fun l => Value.default l.ty),
          result := fn.resultType.map Value.default, k := [], expect := none,
          loopCtx := LoopCtx.top }
      have hcalleeMatch : FrameMatches u calleeFrame :=
        calleeFrameMatches hfn hparamsLen hargsparams hwell
      have hfocus : FocusTyped u calleeFrame
          (.stmts (localInitStmts fn.locals ++ fn.body) LoopCtx.top) := by
        refine ⟨rfl, StackTyped.nil rfl rfl, ?_, ?_⟩
        · rw [frameCtx_function hfn]
          intro stmt hmem
          rcases List.mem_append.mp hmem with hinit | hbody
          · have hlocals : StmtListHasType (Ctx.ofFunction (Env.ofUnit u) fn)
                0 [] (localInitStmts fn.locals) := by
              simpa [localInitStmts] using
                localInitStmtsFrom_typed 0 fn.locals rfl hftyped.1
            exact hlocals stmt hinit
          · exact hftyped.2 stmt hbody
        · intro hd
          simp [LoopCtx.top] at hd
      cases isStmt with
      | true =>
          have hrest' : StackTyped (frameCtx u caller.kind) k .unit caller.loopCtx := by
            simpa using hrest
          have hcaller : FrameTyped u { caller with expect := none } :=
            ⟨FrameMatches.updateExpect hcallerMatch,
              by simpa [Expect.ofOption, hk] using hrest',
              hcallerLoop⟩
          have hcallerExpect : CallerExpects u calleeFrame { caller with expect := none } :=
            Or.inl rfl
          have hshape' : FramesShape (calleeFrame :: { caller with expect := none } :: fr) :=
            FramesShape.push (index := index)
              (FramesShape.updateHead (f := caller) (f' := { caller with expect := none })
                hshape (by simpa using hkind))
              (by simp [calleeFrame])
          have hs' : performCall u { s with frames := caller :: fr }
              (.function index cspan) args true span = .step c' := by
            simpa using hs
          simp only [performCall, hfn] at hs'
          cases hs'
          exact ConfigTyped.pushFrame (f := f) hc hf hcalleeMatch hcaller hshape' hfocus
            (by intro hd; simp [calleeFrame, LoopCtx.top] at hd)
            (by simpa using hkind) hcallerExpect
      | false =>
          obtain ⟨ty0, hty0⟩ : ∃ ty0, result = some ty0 := by
            cases hres : result with
            | none => exact absurd hres (hresult rfl)
            | some ty0 => exact ⟨ty0, rfl⟩
          have hresTy : fn.resultType = some ty0 := by rw [hresSig]; exact hty0
          have hcaller : FrameTyped u { caller with expect := fn.resultType } := by
            refine ⟨FrameMatches.updateExpect hcallerMatch, ?_, hcallerLoop⟩
            have hof : StackTyped (frameCtx u caller.kind) k
                (.ofOption (some ty0)) caller.loopCtx := by
              simpa [hty0] using hrest
            simpa [hresTy, Expect.ofOption, hk] using hof
          have hcallerExpect : CallerExpects u calleeFrame { caller with expect := fn.resultType } := by
            right
            exact ⟨index, fn, ty0, by simp [calleeFrame], hfn, hresTy, by simpa using hresTy⟩
          have hshape' : FramesShape (calleeFrame :: { caller with expect := fn.resultType } :: fr) :=
            FramesShape.push (index := index)
              (FramesShape.updateHead (f := caller) (f' := { caller with expect := fn.resultType })
                hshape (by simpa using hkind))
              (by simp [calleeFrame])
          have hs' : performCall u { s with frames := caller :: fr }
              (.function index cspan) args false span = .step c' := by
            simpa using hs
          simp only [performCall, hfn] at hs'
          cases hs'
          exact ConfigTyped.pushFrame (f := f) hc hf hcalleeMatch hcaller hshape' hfocus
            (by intro hd; simp [calleeFrame, LoopCtx.top] at hd)
            (by simpa using hkind) hcallerExpect

/-- Evaluating the next call argument preserves configuration typing. -/
theorem stepValue_callArgs_preserves {u : HirUnit} {env : Env} {c c' : State} {f : Frame}
    {fr : List Frame} {k : List Cont} {callee : Callee} {done : List Value}
    {remaining : List Expr} {isStmt : Bool} {t : Ty} {ts : List Ty}
    {result : Option Ty} {span : Span} {v : Value}
    (hunit : HasType u env) (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hk : f.k = Cont.mk (.callArgs callee done remaining isStmt) f.loopCtx f.loopCtx span :: k)
    (hne : remaining ≠ [])
    (htyped : Forall2 (ExprHasType (frameCtx u f.kind)) remaining (t :: ts))
    (hdone : ∀ w ∈ done, w.WellFormed)
    (hsig : CalleeSignatureV (frameCtx u f.kind) callee done (t :: ts) result)
    (hresult : isStmt = false → result ≠ none)
    (hrest : StackTyped (frameCtx u f.kind) k
      (if isStmt then .unit else .ofOption result) f.loopCtx)
    (hvwf : v.WellFormed) (hvy : v.ty = t)
    (hs : stepValue u c v = .step c') : ConfigTyped u c' := by
  have hloop : f.loopCtx.depth > 0 → StackHasLoop k := by
    intro hd
    have h := hc.headLoop f fr hf hd
    rw [hk] at h
    exact StackHasLoop.tail_of_not_marker h
      (by simp [Cont.isLoopMarker, ContKind.isLoopMarker])
  have hdoneAll : ∀ w ∈ done ++ [v], w.WellFormed := by
    intro w hw
    rcases List.mem_append.mp hw with hw' | hw'
    · exact hdone w hw'
    · simp at hw'
      subst hw'
      exact hvwf
  simp only [stepValue, hf, hk] at hs
  cases remaining with
  | nil => exact absurd rfl hne
  | cons next more =>
      cases more with
      | nil =>
          -- The evaluated argument is the last one: perform the call.
          cases htyped with
          | cons hnext htail =>
              cases htail
              have hsig' : CalleeSignatureV (frameCtx u f.kind) callee (done ++ [v]) []
                  result := CalleeSignatureV.push hsig hvy
              simp only at hs
              exact performCall_preserves (s := c) (f := f) (fr := fr)
                (caller := { f with k := k }) (k := k) hunit hc hf rfl rfl
                (FrameMatches.updateK (hc.headMatch f fr hf)) hloop hsig' hresult
                hdoneAll hrest hs
      | cons next' more' =>
          -- More arguments remain: push the next one.
          cases htyped with
          | cons hnext htail =>
              cases htail with
              | cons hnext' htail' =>
                  simp only at hs
                  cases hs
                  let k' : List Cont :=
                    Cont.mk (.callArgs callee (done ++ [v]) (next' :: more') isStmt)
                      f.loopCtx f.loopCtx span :: k
                  have hstack' : StackTyped (frameCtx u f.kind) k' (.value _) f.loopCtx :=
                    StackTyped.callArgs (by simp) (Forall2.cons hnext' htail') hdoneAll
                      (CalleeSignatureV.push hsig hvy) hresult hrest
                  have hfocus : FocusTyped u { f with k := k' } (.expr next') :=
                    ⟨_, hnext', by simpa [k'] using hstack'⟩
                  have hloop' : f.loopCtx.depth > 0 → StackHasLoop k' := by
                    intro hd; exact Or.inr (hloop hd)
                  exact ConfigTyped.updateHeadFrame (f' := { f with k := k' }) hc hf
                    (FrameMatches.updateK (hc.headMatch f fr hf)) hfocus
                    (FramesShape.updateHeadK (by simpa [hf] using hc.shape) k') hloop' rfl
/-- A typed expression step preserves configuration typing. -/
theorem stepExpr_preserves {u : HirUnit} {env : Env} {c c' : State} {f : Frame}
    {rest : List Frame} {e : Expr} (hunit : HasType u env) (hc : ConfigTyped u c)
    (hf : c.frames = f :: rest) (he : FocusTyped u f (.expr e))
    (hs : stepExpr u c e = .step c') : ConfigTyped u c' := by
  have hfm := hc.headMatch f rest hf
  simp only [FocusTyped] at he
  rcases he with ⟨T, ht, hstack⟩
  simp only [stepExpr, hf] at hs
  cases e with
  | literal lit span =>
      cases hs
      cases ht with
      | literal hwf =>
          rw [← hf]
          apply ConfigTyped.updateFocus hc hf
          exact ⟨Value.wellFormed_of (Literal.isCore_of_wellFormed hwf)
              (Literal.holds_of_wellFormed hwf), lit.ty, rfl, hstack⟩
  | var id span =>
      cases ht with
      | var hlookup =>
          obtain ⟨v, hv, hvty, hvwf⟩ := ConfigTyped.lookupVar?_typed hc hf hlookup
          simp only [hv] at hs
          cases hs
          rw [← hf]
          apply ConfigTyped.updateFocus hc hf
          exact ⟨hvwf, T, hvty, hstack⟩
  | unary op operand span =>
      cases hs
      cases ht with
      | unary hop hoperand =>
          apply ConfigTyped.updateHeadFrame hc hf
            (FrameMatches.updateK (hc.headMatch f rest hf))
          · exact ⟨_, hoperand, StackTyped.unary hop hstack⟩
          · exact FramesShape.updateHeadK (by simpa [hf] using hc.shape) _
          · intro hd
            exact Or.inr (hc.headLoop f rest hf hd)
          · rfl
  | binary op lhs rhs span =>
      cases hs
      cases ht with
      | binary hop hlhs hrhs =>
          rename_i lhsTy rhsTy
          have hty : lhsTy = rhsTy := binaryWellTyped_eq hop
          have hrhs' : ExprHasType (frameCtx u f.kind) rhs lhsTy := by
            rw [hty]; exact hrhs
          apply ConfigTyped.updateHeadFrame hc hf
            (FrameMatches.updateK (hc.headMatch f rest hf))
          · change ∃ T, ExprHasType (frameCtx u f.kind) lhs T ∧
                StackTyped (frameCtx u f.kind)
                  (Cont.same (.binaryL op rhs) f.loopCtx span :: f.k) (.value T) f.loopCtx
            refine ⟨lhsTy, hlhs, ?_⟩
            rw [← hty] at hop
            exact StackTyped.binaryL hrhs' hop rfl hstack
          · exact FramesShape.updateHeadK (by simpa [hf] using hc.shape) _
          · intro hd
            exact Or.inr (hc.headLoop f rest hf hd)
          · rfl
  | call callee args span =>
      cases args with
      | nil =>
          cases callee with
          | std fn sp =>
              cases ht with
              | call hcall =>
                  rcases callHasType_cases hcall with
                    ⟨idx, cspan, sig, hcallee, hsig, hres, hargs⟩ |
                    ⟨fn, cspan, ty, hcallee, hres, hstd⟩
                  · cases hcallee
                  · rcases stdFnHasType_cases hstd with ⟨argTys, hargs, hres'⟩
                    have hargTysNil : argTys = [] := Forall2.nil_left hargs
                    rw [hargTysNil] at hres'
                    cases fn <;> simp [stdResult?] at hres'
          | function index sp =>
              cases ht with
              | call hcall =>
                  rcases callHasType_cases hcall with
                    ⟨idx, cspan, sig, hcallee, hsig, hres, hargs⟩ |
                    ⟨fn, cspan, ty, hcallee, hres, hstd⟩
                  · cases hcallee
                    have hsigParamsNil : sig.params = [] := Forall2.nil_left hargs
                    obtain ⟨fn, hfn, hsigEq⟩ :=
                      frameCtx_functions_signature (hc.headMatch f rest hf) hsig
                    rw [hsigEq] at hsigParamsNil
                    have hparams : fn.params = [] := List.map_eq_nil_iff.mp hsigParamsNil
                    have hresult : fn.resultType = some T := by
                      rw [hsigEq] at hres
                      exact hres.symm
                    have hfnmem : fn ∈ u.functions := List.mem_of_getElem? hfn
                    have hftyped : FunctionTyped (Env.ofUnit u) fn := by
                      have := hunit.functions fn hfnmem
                      rwa [hunit.env_eq] at this
                    have hwell : FunctionWellFormed fn := hunit.wellformed.functions fn hfnmem
                    have hcalleeMatch : FrameMatches u
                        { kind := .function index, params := [],
                          locals := fn.locals.map (fun l => Value.default l.ty),
                          result := fn.resultType.map Value.default, k := [], expect := none,
                          loopCtx := LoopCtx.top } := by
                      apply calleeFrameMatches hfn
                      · simp [hparams]
                      · intro i v hv
                        simp at hv
                      · exact hwell
                    have hcaller : FrameTyped u { f with expect := fn.resultType } := by
                      refine ⟨FrameMatches.updateExpect hfm, ?_, hc.headLoop f rest hf⟩
                      rw [hresult]
                      simpa [Expect.ofOption] using hstack
                    have hshape : FramesShape
                        ({ kind := .function index, params := [],
                           locals := fn.locals.map (fun l => Value.default l.ty),
                           result := fn.resultType.map Value.default, k := [], expect := none,
                           loopCtx := LoopCtx.top } :: { f with expect := fn.resultType } :: rest) := by
                      apply FramesShape.push
                      · exact FramesShape.updateHeadExpect (by simpa [hf] using hc.shape)
                      · rfl
                    have hfocus : FocusTyped u
                        { kind := .function index, params := [],
                          locals := fn.locals.map (fun l => Value.default l.ty),
                          result := fn.resultType.map Value.default, k := [], expect := none,
                          loopCtx := LoopCtx.top }
                        (.stmts (localInitStmts fn.locals ++ fn.body) LoopCtx.top) := by
                      refine ⟨rfl, StackTyped.nil rfl rfl, ?_, ?_⟩
                      · rw [frameCtx_function hfn]
                        intro stmt hmem
                        rcases List.mem_append.mp hmem with hinit | hbody
                        · have hlocals : StmtListHasType (Ctx.ofFunction (Env.ofUnit u) fn)
                              0 [] (localInitStmts fn.locals) := by
                            simpa [localInitStmts] using
                              localInitStmtsFrom_typed 0 fn.locals rfl hftyped.1
                          exact hlocals stmt hinit
                        · exact hftyped.2 stmt hbody
                      · intro hd
                        simp [LoopCtx.top] at hd
                    have hcallerExpect : CallerExpects u
                        { kind := .function index, params := [],
                          locals := fn.locals.map (fun l => Value.default l.ty),
                          result := fn.resultType.map Value.default, k := [], expect := none,
                          loopCtx := LoopCtx.top }
                        { f with expect := fn.resultType } := by
                      right
                      exact ⟨index, fn, T, rfl, hfn, hresult, by rw [hresult]⟩
                    simp only [performCall, hfn, hf] at hs
                    cases hs
                    exact ConfigTyped.pushFrame hc hf hcalleeMatch hcaller hshape hfocus
                      (by intro hd; simp [LoopCtx.top] at hd) rfl hcallerExpect
                  · cases hcallee
      | cons arg more =>
          cases hs
          cases ht with
          | call hcall =>
              rcases callArgsPush hcall hstack with ⟨t, harg, hstack'⟩
              apply ConfigTyped.updateHeadFrame hc hf
                (FrameMatches.updateK (hc.headMatch f rest hf))
              · exact ⟨_, harg, hstack'⟩
              · exact FramesShape.updateHeadK (by simpa [hf] using hc.shape) _
              · intro hd
                exact Or.inr (hc.headLoop f rest hf hd)
              · rfl
  | cast operand to kind span =>
      cases hs
      cases ht with
      | cast hoperand hconv hcore =>
          apply ConfigTyped.updateHeadFrame hc hf
            (FrameMatches.updateK (hc.headMatch f rest hf))
          · exact ⟨_, hoperand, StackTyped.cast hconv hcore hstack⟩
          · exact FramesShape.updateHeadK (by simpa [hf] using hc.shape) _
          · intro hd
            exact Or.inr (hc.headLoop f rest hf hd)
          · rfl

/-- Assignment preserves configuration typing. -/
theorem stepValue_assign_preserves {u : HirUnit} {c c' : State} {f : Frame}
    {fr : List Frame} {k : List Cont} {place : Place} {ty : Ty} {span : Span} {v : Value}
    (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hk : f.k = Cont.mk (.assign place) f.loopCtx f.loopCtx span :: k)
    (hvwf : v.WellFormed) (hvy : v.ty = ty)
    (hlookup : (frameCtx u f.kind).lookup place.id = some ty)
    (hrest : StackTyped (frameCtx u f.kind) k .unit f.loopCtx)
    (hs : stepValue u c v = .step c') : ConfigTyped u c' := by
  simp only [stepValue, hf, hk] at hs
  cases hs
  have hloop : f.loopCtx.depth > 0 → StackHasLoop k := by
    intro hd
    have h := hc.headLoop f fr hf hd
    rw [hk] at h
    exact StackHasLoop.tail_of_not_marker h
      (by simp [Cont.isLoopMarker, ContKind.isLoopMarker])
  let s' : State := { c with frames := { f with k := k } :: fr, focus := .resume }
  have hc1 : ConfigTyped u s' :=
    ConfigTyped.updateHeadFocus hc hf (FocusTyped.resume_of hrest) hloop
  have hlookup' : (frameCtx u ({ f with k := k } : Frame).kind).lookup place.id =
      some v.ty := by
    rw [hvy]; simpa using hlookup
  have hc2 : ConfigTyped u (s'.setVar place.id v) :=
    ConfigTyped.setVar hc1 rfl hlookup' hvwf
  cases hid : place.id <;> simpa [s', hf, State.updateHead, State.setVar, hid] using hc2

/-- Unary evaluation preserves configuration typing. -/
theorem stepValue_unary_preserves {u : HirUnit} {c c' : State} {f : Frame}
    {fr : List Frame} {k : List Cont} {op : UnaryOp} {ty : Ty} {span : Span} {v : Value}
    (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hk : f.k = Cont.mk (.unary op) f.loopCtx f.loopCtx span :: k)
    (hvwf : v.WellFormed) (hvy : v.ty = ty)
    (hop : unaryWellTyped op ty = true)
    (hrest : StackTyped (frameCtx u f.kind) k (.value ty) f.loopCtx)
    (hs : stepValue u c v = .step c') : ConfigTyped u c' := by
  simp only [stepValue, hf, hk] at hs
  rw [hvy] at hs
  generalize hev : evalUnary op ty v.value = res at hs
  cases res with
  | error trap =>
      simp at hs
      cases hs
      simpa [hf] using ConfigTyped.updateFocus hc hf (FocusTyped.trapped u f trap span)
  | ok w =>
      simp at hs
      cases hs
      have hcore : ty.isCore = true := by
        have hw := Value.isCore_of_wellFormed hvwf
        rwa [hvy] at hw
      have hholds : ty.Holds v.value := by
        have hw := Value.holds_of_wellFormed hvwf
        rwa [hvy] at hw
      obtain ⟨hwty, hwwf⟩ := evalUnary_ok hcore hop hholds hev
      have hloop : f.loopCtx.depth > 0 → StackHasLoop k := by
        intro hd
        have hloop := hc.headLoop f fr hf hd
        rw [hk] at hloop
        exact StackHasLoop.tail_of_not_marker hloop
          (by simp [Cont.isLoopMarker, ContKind.isLoopMarker])
      have hfocus : FocusTyped u { f with k := k } (.value w) := by
        refine FocusTyped.value_of ?_ hwwf
        rwa [hwty]
      exact ConfigTyped.updateHeadFrame hc hf
        (FrameMatches.updateK (hc.headMatch f fr hf)) hfocus
        (FramesShape.updateHeadK (by simpa [hf] using hc.shape) k) hloop rfl

/-- Evaluating the right operand preserves configuration typing. -/
theorem stepValue_binaryL_preserves {u : HirUnit} {c c' : State} {f : Frame}
    {fr : List Frame} {k : List Cont} {op : BinaryOp} {rhs : Expr} {ty res : Ty}
    {span : Span} {v : Value}
    (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hk : f.k = Cont.mk (.binaryL op rhs) f.loopCtx f.loopCtx span :: k)
    (hvwf : v.WellFormed) (hvy : v.ty = ty)
    (hrhs : ExprHasType (frameCtx u f.kind) rhs ty)
    (hop : binaryWellTyped op ty ty = true)
    (hres : res = binaryResultTy op ty ty)
    (hrest : StackTyped (frameCtx u f.kind) k (.value res) f.loopCtx)
    (hs : stepValue u c v = .step c') : ConfigTyped u c' := by
  simp only [stepValue, hf, hk] at hs
  cases hs
  let k' : List Cont := Cont.mk (.binaryR op v) f.loopCtx f.loopCtx span :: k
  have hloop : f.loopCtx.depth > 0 → StackHasLoop k' := by
    intro hd
    have h := hc.headLoop f fr hf hd
    rw [hk] at h
    exact Or.inr (StackHasLoop.tail_of_not_marker h
      (by simp [Cont.isLoopMarker, ContKind.isLoopMarker]))
  have hstack' : StackTyped (frameCtx u f.kind) k' (.value ty) f.loopCtx :=
    StackTyped.binaryR hvwf hvy hop hres hrest
  have hfocus : FocusTyped u { f with k := k' } (.expr rhs) :=
    ⟨ty, hrhs, hstack'⟩
  exact ConfigTyped.updateHeadFrame hc hf
    (FrameMatches.updateK (hc.headMatch f fr hf)) hfocus
    (FramesShape.updateHeadK (by simpa [hf] using hc.shape) _) hloop rfl

/-- Applying the binary operation preserves configuration typing. -/
theorem stepValue_binaryR_preserves {u : HirUnit} {c c' : State} {f : Frame}
    {fr : List Frame} {k : List Cont} {op : BinaryOp} {lhs : Value} {ty res : Ty}
    {span : Span} {v : Value}
    (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hk : f.k = Cont.mk (.binaryR op lhs) f.loopCtx f.loopCtx span :: k)
    (hlhs : lhs.WellFormed) (hty : lhs.ty = ty) (hvwf : v.WellFormed)
    (hvy : v.ty = ty)
    (hop : binaryWellTyped op ty ty = true)
    (hres : res = binaryResultTy op ty ty)
    (hrest : StackTyped (frameCtx u f.kind) k (.value res) f.loopCtx)
    (hs : stepValue u c v = .step c') : ConfigTyped u c' := by
  simp only [stepValue, hf, hk] at hs
  rw [hty] at hs
  generalize hev : evalBinary op ty lhs.value v.value = res at hs
  cases res with
  | error trap =>
      simp at hs
      cases hs
      simpa [hf] using ConfigTyped.updateFocus hc hf (FocusTyped.trapped u f trap span)
  | ok w =>
      simp at hs
      cases hs
      have hcore : ty.isCore = true := by
        have hw := Value.isCore_of_wellFormed hlhs
        rwa [hty] at hw
      have ha : ty.Holds lhs.value := by
        have hw := Value.holds_of_wellFormed hlhs
        rwa [hty] at hw
      have hb : ty.Holds v.value := by
        have hw := Value.holds_of_wellFormed hvwf
        rwa [hvy] at hw
      obtain ⟨hwty, hwwf⟩ := evalBinary_ok hcore hop ha hb hev
      have hres' : res = (if op.isComparison then Ty.bool else ty) := by
        rw [binaryResultTy_eq] at hres; exact hres
      have hwty' : w.ty = res := hwty.trans hres'.symm
      have hloop : f.loopCtx.depth > 0 → StackHasLoop k := by
        intro hd
        have hloop := hc.headLoop f fr hf hd
        rw [hk] at hloop
        exact StackHasLoop.tail_of_not_marker hloop
          (by simp [Cont.isLoopMarker, ContKind.isLoopMarker])
      have hfocus : FocusTyped u { f with k := k } (.value w) := by
        refine FocusTyped.value_of ?_ hwwf
        rwa [hwty']
      exact ConfigTyped.updateHeadFrame hc hf
        (FrameMatches.updateK (hc.headMatch f fr hf)) hfocus
        (FramesShape.updateHeadK (by simpa [hf] using hc.shape) k) hloop rfl

/-- Applying the conversion preserves configuration typing. -/
theorem stepValue_cast_preserves {u : HirUnit} {c c' : State} {f : Frame}
    {fr : List Frame} {k : List Cont} {to src : Ty} {span : Span} {v : Value}
    (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hk : f.k = Cont.mk (.cast to) f.loopCtx f.loopCtx span :: k)
    (hvwf : v.WellFormed) (hvy : v.ty = src)
    (hconv : ConversionLegal src to) (hcore : to.isCore = true)
    (hrest : StackTyped (frameCtx u f.kind) k (.value to) f.loopCtx)
    (hs : stepValue u c v = .step c') : ConfigTyped u c' := by
  simp only [stepValue, hf, hk] at hs
  rw [hvy] at hs
  generalize hev : evalCast src to v.value = res at hs
  cases res with
  | error trap =>
      simp at hs
      cases hs
      simpa [hf] using ConfigTyped.updateFocus hc hf (FocusTyped.trapped u f trap span)
  | ok w =>
      simp at hs
      cases hs
      obtain ⟨hwty, hwwf⟩ := evalCast_ok hcore hev
      have hloop : f.loopCtx.depth > 0 → StackHasLoop k := by
        intro hd
        have hloop := hc.headLoop f fr hf hd
        rw [hk] at hloop
        exact StackHasLoop.tail_of_not_marker hloop
          (by simp [Cont.isLoopMarker, ContKind.isLoopMarker])
      have hfocus : FocusTyped u { f with k := k } (.value w) := by
        refine FocusTyped.value_of ?_ hwwf
        rwa [hwty]
      exact ConfigTyped.updateHeadFrame hc hf
        (FrameMatches.updateK (hc.headMatch f fr hf)) hfocus
        (FramesShape.updateHeadK (by simpa [hf] using hc.shape) k) hloop rfl

/-- `IF` arm selection preserves configuration typing. -/
theorem stepValue_ifCond_preserves {u : HirUnit} {c c' : State} {f : Frame}
    {fr : List Frame} {k : List Cont} {arms : List IfArm} {elseBody : List Stmt}
    {span : Span} {v : Value}
    (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hk : f.k = Cont.mk (.ifCond arms elseBody) f.loopCtx f.loopCtx span :: k)
    (harms : ∀ arm ∈ arms, ExprHasType (frameCtx u f.kind) arm.cond .bool ∧
      StmtListHasType (frameCtx u f.kind) f.loopCtx.depth f.loopCtx.controlled arm.body)
    (helse : StmtListHasType (frameCtx u f.kind) f.loopCtx.depth f.loopCtx.controlled elseBody)
    (hrest : StackTyped (frameCtx u f.kind) k .unit f.loopCtx)
    (hs : stepValue u c v = .step c') : ConfigTyped u c' := by
  have hloop : f.loopCtx.depth > 0 → StackHasLoop k := by
    intro hd
    have h := hc.headLoop f fr hf hd
    rw [hk] at h
    exact StackHasLoop.tail_of_not_marker h
      (by simp [Cont.isLoopMarker, ContKind.isLoopMarker])
  simp only [stepValue, hf, hk] at hs
  cases arms with
  | nil =>
      simp at hs
      cases hs
      exact ConfigTyped.updateHeadFocus hc hf
        (FocusTyped.stmts_of rfl hrest helse hloop) hloop
  | cons arm remaining =>
      by_cases hzero : (v.value == 0) = true
      · simp [hzero] at hs
        cases remaining with
        | nil =>
            simp at hs
            cases hs
            exact ConfigTyped.updateHeadFocus hc hf
              (FocusTyped.stmts_of rfl hrest helse hloop) hloop
        | cons next more =>
            simp at hs
            cases hs
            have hnext := harms next (by simp)
            let k' : List Cont :=
              Cont.mk (.ifCond (next :: more) elseBody) f.loopCtx f.loopCtx span :: k
            have hstack' : StackTyped (frameCtx u f.kind) k' (.value .bool) f.loopCtx :=
              StackTyped.ifCond (arms := next :: more)
                (by intro a ha; exact harms a (by simp [ha])) helse hrest
            have hfocus : FocusTyped u { f with k := k' } (.expr next.cond) :=
              ⟨.bool, hnext.1, hstack'⟩
            have hloop' : f.loopCtx.depth > 0 → StackHasLoop k' := by
              intro hd; exact Or.inr (hloop hd)
            exact ConfigTyped.updateHeadFrame hc hf
              (FrameMatches.updateK (hc.headMatch f fr hf)) hfocus
              (FramesShape.updateHead (f := f) (rest := fr) (f' := { f with k := k' }) (by simpa [hf] using hc.shape) rfl) hloop' rfl
      · simp [hzero] at hs
        cases hs
        have harm := harms arm (by simp)
        exact ConfigTyped.updateHeadFocus hc hf
          (FocusTyped.stmts_of rfl hrest harm.2 hloop) hloop

/-- A body returned by `caseArmBody?` is one of the arm bodies. -/
theorem caseArmBody?_typed {ctx : Ctx} {depth : Nat} {controlled : List VarId}
    {arms : List CaseArm} {value : Int} {body : List Stmt}
    (hbodies : ∀ arm ∈ arms, StmtListHasType ctx depth controlled arm.body)
    (h : caseArmBody? arms value = some body) :
    StmtListHasType ctx depth controlled body := by
  induction arms with
  | nil => simp [caseArmBody?] at h
  | cons arm rest ih =>
      by_cases hmem : arm.labels.any (fun label => caseLabelContains label value) = true
      · simp [caseArmBody?, hmem] at h
        subst h
        exact hbodies arm (by simp)
      · simp [caseArmBody?, hmem] at h
        exact ih (by intro a ha; exact hbodies a (by simp [ha])) h

/-- `FOR` start-value evaluation preserves configuration typing. -/
theorem stepValue_forInit_preserves {u : HirUnit} {c c' : State} {f : Frame}
    {fr : List Frame} {k : List Cont} {var : Place} {stop : Expr}
    {step : Option Expr} {body : List Stmt} {ty : Ty} {span : Span} {v : Value}
    (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hk : f.k = Cont.mk (.forInit var stop step body) f.loopCtx f.loopCtx span :: k)
    (hvar : (frameCtx u f.kind).lookup var.id = some ty)
    (hint : ty.isInt = true)
    (hnot : var.id ∉ f.loopCtx.controlled)
    (hstop : ExprHasType (frameCtx u f.kind) stop ty)
    (hstep : ∀ e, step = some e → ExprHasType (frameCtx u f.kind) e ty)
    (hzero : ∀ e, step = some e → ∀ v, Expr.constInt? e = some v → v ≠ 0)
    (hbody : StmtListHasType (frameCtx u f.kind) (f.loopCtx.depth + 1)
      (var.id :: f.loopCtx.controlled) body)
    (hrest : StackTyped (frameCtx u f.kind) k .unit f.loopCtx)
    (hvwf : v.WellFormed) (hvy : v.ty = ty)
    (hs : stepValue u c v = .step c') : ConfigTyped u c' := by
  have hfm := hc.headMatch f fr hf
  have hshape : FramesShape (f :: fr) := by simpa [hf] using hc.shape
  have hloop : f.loopCtx.depth > 0 → StackHasLoop k := by
    intro hd
    have h := hc.headLoop f fr hf hd
    rw [hk] at h
    exact StackHasLoop.tail_of_not_marker h
      (by simp [Cont.isLoopMarker, ContKind.isLoopMarker])
  simp only [stepValue, hf, hk] at hs
  let k' : List Cont := Cont.mk (.forStop var step body) f.loopCtx f.loopCtx span :: k
  have hstack' : StackTyped (frameCtx u f.kind) k' (.value ty) f.loopCtx :=
    StackTyped.forStop hvar hint hnot hstep hzero hbody hrest
  have hfocus : FocusTyped u { f with k := k' } (.expr stop) :=
    ⟨ty, hstop, hstack'⟩
  have hloop' : f.loopCtx.depth > 0 → StackHasLoop k' := by
    intro hd; exact Or.inr (hloop hd)
  let s1 : State :=
    { globals := c.globals, frames := { f with k := k' } :: fr, focus := .expr stop,
      cycles := c.cycles, cyclesLeft := c.cyclesLeft }
  have hc1 : ConfigTyped u s1 :=
    ConfigTyped.updateHeadFrame hc hf (FrameMatches.updateK hfm) hfocus
      (FramesShape.updateHeadK (f := f) (rest := fr) hshape k') hloop' rfl
  have hlookup' : (frameCtx u ({ f with k := k' } : Frame).kind).lookup var.id =
      some v.ty := by
    rw [hvy]; simpa using hvar
  have hc2 : ConfigTyped u (s1.setVar var.id v) :=
    ConfigTyped.setVar hc1 rfl hlookup' hvwf
  cases hid : var.id with
  | param i =>
      simp only [s1, State.setVar, hf, hid] at hs
      cases hs
      simpa [s1, k', Cont.same, State.updateHead, State.setVar, hf, hid] using hc2
  | «local» i =>
      simp only [s1, State.setVar, hf, hid] at hs
      cases hs
      simpa [s1, k', Cont.same, State.updateHead, State.setVar, hf, hid] using hc2
  | result =>
      simp only [s1, State.setVar, hf, hid] at hs
      cases hs
      simpa [s1, k', Cont.same, State.updateHead, State.setVar, hf, hid] using hc2
  | global i =>
      simp only [s1, State.setVar, hf, hid] at hs
      cases hs
      simpa [s1, k', Cont.same, State.updateHead, State.setVar, hf, hid] using hc2

/-- `CASE` arm selection preserves configuration typing. -/
theorem stepValue_caseSel_preserves {u : HirUnit} {c c' : State} {f : Frame}
    {fr : List Frame} {k : List Cont} {selector : Expr} {arms : List CaseArm}
    {elseBody : List Stmt} {span : Span} {v : Value}
    (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hk : f.k = Cont.mk (.caseSel selector arms elseBody) f.loopCtx f.loopCtx span :: k)
    (hbodies : ∀ arm ∈ arms,
      StmtListHasType (frameCtx u f.kind) f.loopCtx.depth f.loopCtx.controlled arm.body)
    (helse : StmtListHasType (frameCtx u f.kind) f.loopCtx.depth f.loopCtx.controlled elseBody)
    (hrest : StackTyped (frameCtx u f.kind) k .unit f.loopCtx)
    (hs : stepValue u c v = .step c') : ConfigTyped u c' := by
  have hloop : f.loopCtx.depth > 0 → StackHasLoop k := by
    intro hd
    have h := hc.headLoop f fr hf hd
    rw [hk] at h
    exact StackHasLoop.tail_of_not_marker h
      (by simp [Cont.isLoopMarker, ContKind.isLoopMarker])
  simp only [stepValue, hf, hk] at hs
  generalize hcase : caseArmBody? arms v.value = res at hs
  cases res with
  | none =>
      simp at hs
      cases hs
      exact ConfigTyped.updateHeadFocus hc hf
        (FocusTyped.stmts_of rfl hrest helse hloop) hloop
  | some body =>
      simp at hs
      cases hs
      have hbody := caseArmBody?_typed hbodies hcase
      exact ConfigTyped.updateHeadFocus hc hf
        (FocusTyped.stmts_of rfl hrest hbody hloop) hloop

/-- `WHILE` condition selection preserves configuration typing. -/
theorem stepValue_whileCond_preserves {u : HirUnit} {c c' : State} {f : Frame}
    {fr : List Frame} {k : List Cont} {cond : Expr} {body : List Stmt}
    {span : Span} {v : Value}
    (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hk : f.k = Cont.mk (.whileCond cond body) f.loopCtx f.loopCtx span :: k)
    (hcond : ExprHasType (frameCtx u f.kind) cond .bool)
    (hbody : StmtListHasType (frameCtx u f.kind) (f.loopCtx.depth + 1)
      f.loopCtx.controlled body)
    (hrest : StackTyped (frameCtx u f.kind) k .unit f.loopCtx)
    (hs : stepValue u c v = .step c') : ConfigTyped u c' := by
  have hloop : f.loopCtx.depth > 0 → StackHasLoop k := by
    intro hd
    have h := hc.headLoop f fr hf hd
    rw [hk] at h
    exact StackHasLoop.tail_of_not_marker h
      (by simp [Cont.isLoopMarker, ContKind.isLoopMarker])
  simp only [stepValue, hf, hk] at hs
  by_cases hzero : (v.value == 0) = true
  · simp [hzero] at hs
    cases hs
    exact ConfigTyped.updateHeadFocus hc hf (FocusTyped.resume_of hrest) hloop
  · simp [hzero] at hs
    cases hs
    let bodyLc : LoopCtx := LoopCtx.body f.loopCtx f.loopCtx.controlled
    let k' : List Cont := Cont.mk (.whileLoop cond body) bodyLc f.loopCtx span :: k
    have hstack' : StackTyped (frameCtx u f.kind) k' .unit bodyLc :=
      StackTyped.whileLoop rfl rfl hcond (by simpa [bodyLc, LoopCtx.body] using hbody)
        (by intro hd; exact hloop hd) hrest
    have hfocus : FocusTyped u { f with k := k', loopCtx := bodyLc }
        (.stmts body bodyLc) :=
      FocusTyped.stmts_of rfl hstack' (by simpa [bodyLc, LoopCtx.body] using hbody)
        (by intro _; exact Or.inl rfl)
    exact ConfigTyped.updateHeadFrame hc hf (FrameMatches.updateKLoop
      (hc.headMatch f fr hf)) hfocus
      (FramesShape.updateHead (f := f) (rest := fr) (f' := { f with k := k', loopCtx := bodyLc }) (by simpa [hf] using hc.shape) rfl)
      (by intro _; exact Or.inl rfl) rfl

/-- `REPEAT` condition selection preserves configuration typing. -/
theorem stepValue_repeatCond_preserves {u : HirUnit} {c c' : State} {f : Frame}
    {fr : List Frame} {k : List Cont} {body : List Stmt} {cond : Expr}
    {span : Span} {v : Value}
    (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hk : f.k = Cont.mk (.repeatCond body cond) f.loopCtx f.loopCtx span :: k)
    (hcond : ExprHasType (frameCtx u f.kind) cond .bool)
    (hbody : StmtListHasType (frameCtx u f.kind) (f.loopCtx.depth + 1)
      f.loopCtx.controlled body)
    (hrest : StackTyped (frameCtx u f.kind) k .unit f.loopCtx)
    (hs : stepValue u c v = .step c') : ConfigTyped u c' := by
  have hloop : f.loopCtx.depth > 0 → StackHasLoop k := by
    intro hd
    have h := hc.headLoop f fr hf hd
    rw [hk] at h
    exact StackHasLoop.tail_of_not_marker h
      (by simp [Cont.isLoopMarker, ContKind.isLoopMarker])
  simp only [stepValue, hf, hk] at hs
  by_cases hzero : (v.value == 0) = true
  · simp [hzero] at hs
    cases hs
    let bodyLc : LoopCtx := LoopCtx.body f.loopCtx f.loopCtx.controlled
    let k' : List Cont := Cont.mk (.repeatBody body cond) bodyLc f.loopCtx span :: k
    have hstack' : StackTyped (frameCtx u f.kind) k' .unit bodyLc :=
      StackTyped.repeatBody rfl rfl hcond (by simpa [bodyLc, LoopCtx.body] using hbody)
        (by intro hd; exact hloop hd) hrest
    have hfocus : FocusTyped u { f with k := k', loopCtx := bodyLc }
        (.stmts body bodyLc) :=
      FocusTyped.stmts_of rfl hstack' (by simpa [bodyLc, LoopCtx.body] using hbody)
        (by intro _; exact Or.inl rfl)
    exact ConfigTyped.updateHeadFrame hc hf (FrameMatches.updateKLoop
      (hc.headMatch f fr hf)) hfocus
      (FramesShape.updateHead (f := f) (rest := fr) (f' := { f with k := k', loopCtx := bodyLc }) (by simpa [hf] using hc.shape) rfl)
      (by intro _; exact Or.inl rfl) rfl
  · simp [hzero] at hs
    cases hs
    exact ConfigTyped.updateHeadFocus hc hf (FocusTyped.resume_of hrest) hloop

/-- One is representable in every core integer type. -/
theorem Ty.holdsB_one {ty : Ty} (hcore : ty.isCore = true) (hint : ty.isInt = true) :
    ty.holdsB 1 = true := by
  cases ty with
  | bool => simp [Ty.isInt] at hint
  | int w s =>
      simp only [Ty.isCore, Ty.coreWidth, Bool.or_eq_true, beq_iff_eq, or_assoc] at hcore
      rcases hcore with rfl | rfl | rfl | rfl <;> cases s <;> decide
  | bits w => simp [Ty.isInt] at hint
  | char k => simp [Ty.isInt] at hint

/-- The default `FOR` step value is well formed. -/
theorem Value.one_wellFormed {ty : Ty} (hcore : ty.isCore = true)
    (hint : ty.isInt = true) : (Value.ofInt ty 1).WellFormed :=
  Value.wellFormed_of hcore (Ty.holdsB_one hcore hint)

/-- Choosing whether a `FOR` body runs preserves configuration typing. -/
theorem forContinue_preserves {u : HirUnit} {c c' : State} {f g : Frame}
    {fr : List Frame} {rest : List Cont} {var : Place} {stopVal stepVal : Value}
    {body : List Stmt} {ty : Ty} {lc : LoopCtx} {span : Span} {cont : Cont}
    (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hgKind : g.kind = f.kind) (hgm : FrameMatches u g)
    (hcontAfter : cont.after = lc) (hcontSpan : cont.span = span)
    (hvar : (frameCtx u g.kind).lookup var.id = some ty)
    (hint : ty.isInt = true)
    (hstop : stopVal.WellFormed ∧ stopVal.ty = ty)
    (hstep : stepVal.WellFormed ∧ stepVal.ty = ty)
    (hbody : StmtListHasType (frameCtx u g.kind) (lc.depth + 1)
      (var.id :: lc.controlled) body)
    (hrest : StackTyped (frameCtx u g.kind) rest .unit lc)
    (hloop : lc.depth > 0 → StackHasLoop rest)
    (hs : forContinue u { c with frames := g :: fr } cont rest
            var stopVal stepVal body = .step c') :
    ConfigTyped u c' := by
  have hshape : FramesShape (f :: fr) := by simpa [hf] using hc.shape
  simp only [forContinue, hcontAfter, hcontSpan, State.head?, List.head?_cons,
    Option.getD_some, List.tail_cons] at hs
  by_cases hkeep : forKeep ((({ c with frames := g :: fr }).lookupVar? var.id).getD stopVal) stopVal stepVal = true
  · -- The body runs: push a loop marker and enter the body.
    rw [if_pos hkeep] at hs
    cases hs
    let bodyLc : LoopCtx := LoopCtx.body lc (var.id :: lc.controlled)
    let k' : List Cont := Cont.mk (.forBody var stopVal stepVal body) bodyLc lc span :: rest
    have hbodyLc : StmtListHasType (frameCtx u g.kind) bodyLc.depth bodyLc.controlled body := by
      simpa [bodyLc, LoopCtx.body] using hbody
    have hstack' : StackTyped (frameCtx u g.kind) k' .unit bodyLc :=
      StackTyped.forBody (by simp [bodyLc, LoopCtx.body]) (by simp [bodyLc, LoopCtx.body])
        hvar hint hstop hstep hbodyLc hloop hrest
    have hfocus : FocusTyped u { g with k := k', loopCtx := bodyLc } (.stmts body bodyLc) :=
      FocusTyped.stmts_of rfl hstack' hbodyLc (by intro _; exact Or.inl rfl)
    exact ConfigTyped.updateHeadFrame (f' := { g with k := k', loopCtx := bodyLc }) hc hf
      (FrameMatches.updateKLoop hgm) hfocus
      (FramesShape.updateHead (f := f) (rest := fr) (f' := { g with k := k', loopCtx := bodyLc })
        hshape hgKind)
      (by intro _; exact Or.inl rfl) hgKind
  · -- The loop is finished: resume with the remaining stack.
    rw [if_neg hkeep] at hs
    cases hs
    exact ConfigTyped.updateHeadFrame (f' := { g with k := rest, loopCtx := lc }) hc hf
      (FrameMatches.updateKLoop hgm)
      (FocusTyped.resume_of hrest)
      (FramesShape.updateHead (f := f) (rest := fr)
        (f' := { g with k := rest, loopCtx := lc }) hshape hgKind)
      hloop hgKind

/-- A `FOR` increment preserves configuration typing. -/
theorem forAdvance_preserves {u : HirUnit} {c c' : State} {f g : Frame}
    {fr : List Frame} {below : List Cont} {var : Place} {stopVal stepVal : Value}
    {body : List Stmt} {ty : Ty} {lc : LoopCtx} {span : Span} {cont : Cont}
    (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hgKind : g.kind = f.kind) (hgm : FrameMatches u g)
    (hloopG : g.loopCtx.depth > 0 → StackHasLoop g.k)
    (hcontAfter : cont.after = lc) (hcontSpan : cont.span = span)
    (hvar : (frameCtx u f.kind).lookup var.id = some ty)
    (hint : ty.isInt = true)
    (hstop : stopVal.WellFormed ∧ stopVal.ty = ty)
    (hstep : stepVal.WellFormed ∧ stepVal.ty = ty)
    (hbody : StmtListHasType (frameCtx u f.kind) (lc.depth + 1)
      (var.id :: lc.controlled) body)
    (hbelow : StackTyped (frameCtx u f.kind) below .unit lc)
    (hloop : lc.depth > 0 → StackHasLoop below)
    (hcur : ∀ w, ({ c with frames := g :: fr }).lookupVar? var.id = some w →
      w.ty = ty ∧ w.WellFormed)
    (hs : forAdvance u { c with frames := g :: fr } cont below
            var stopVal stepVal body = .step c') :
    ConfigTyped u c' := by
  simp only [forAdvance, hcontAfter, hcontSpan] at hs
  generalize hcl : (({ c with frames := g :: fr }).lookupVar? var.id).getD stepVal = cur at hs
  have hcurTy : cur.ty = ty := by
    cases hl : ({ c with frames := g :: fr }).lookupVar? var.id with
    | none =>
        rw [hl] at hcl
        simp only [Option.getD_none] at hcl
        rw [← hcl]
        exact hstep.2
    | some w =>
        rw [hl] at hcl
        simp only [Option.getD_some] at hcl
        rw [← hcl]
        exact (hcur w hl).1
  have hcurCore : cur.ty.isCore = true := by
    cases hl : ({ c with frames := g :: fr }).lookupVar? var.id with
    | none =>
        rw [hl] at hcl
        simp only [Option.getD_none] at hcl
        rw [← hcl]
        exact Value.isCore_of_wellFormed hstep.1
    | some w =>
        rw [hl] at hcl
        simp only [Option.getD_some] at hcl
        rw [← hcl]
        exact Value.isCore_of_wellFormed (hcur w hl).2
  cases hm : cur.ty.layout.add cur.value stepVal.value with
  | error trap =>
      simp only [hm] at hs
      cases hs
      exact ConfigTyped.updateHeadFrame (f' := g) hc hf hgm
        (FocusTyped.trapped u g trap span)
        (FramesShape.updateHead (by simpa [hf] using hc.shape) hgKind) hloopG hgKind
  | ok next =>
      simp only [hm] at hs
      have hvfits : cur.ty.layout.fits next = true := by
        have := (Stc.Exact.IntType.add_ok.mp hm).1
        rwa [(Stc.Exact.IntType.add_ok.mp hm).2] at this
      have hvwf : (Value.ofInt cur.ty next).WellFormed :=
        Value.wellFormed_of hcurCore hvfits
      cases hid : var.id with
      | param i =>
          rw [hid] at hs
          simp only [State.setVar] at hs
          have hvar' : (frameCtx u
              ({ g with params := g.params.set i (Value.ofInt cur.ty next) }).kind).lookup
              (.param i) = some (Value.ofInt cur.ty next).ty := by
            rw [← hid]
            rw [hcurTy]
            simpa [Value.ofInt, hgKind] using hvar
          refine forContinue_preserves (c := c) (f := f) (fr := fr) (span := span)
            (var := var)
            (g := { g with params := g.params.set i (Value.ofInt cur.ty next) })
            hc hf (by simpa using hgKind) (FrameMatches.setParams hgm hvar' hvwf)
            hcontAfter hcontSpan
            (by simpa [hgKind] using hvar) hint hstop hstep
            (by simpa [hgKind] using hbody) (by simpa [hgKind] using hbelow) hloop ?_
          simpa [State.setVar] using hs
      | «local» i =>
          rw [hid] at hs
          simp only [State.setVar] at hs
          have hvar' : (frameCtx u
              ({ g with locals := g.locals.set i (Value.ofInt cur.ty next) }).kind).lookup
              (.local i) = some (Value.ofInt cur.ty next).ty := by
            rw [← hid]
            rw [hcurTy]
            simpa [Value.ofInt, hgKind] using hvar
          refine forContinue_preserves (c := c) (f := f) (fr := fr) (span := span)
            (var := var)
            (g := { g with locals := g.locals.set i (Value.ofInt cur.ty next) })
            hc hf (by simpa using hgKind) (FrameMatches.setLocals hgm hvar' hvwf)
            hcontAfter hcontSpan
            (by simpa [hgKind] using hvar) hint hstop hstep
            (by simpa [hgKind] using hbody) (by simpa [hgKind] using hbelow) hloop ?_
          simpa [State.setVar] using hs
      | result =>
          rw [hid] at hs
          simp only [State.setVar] at hs
          have hvar' : (frameCtx u
              ({ g with result := some (Value.ofInt cur.ty next) }).kind).lookup
              .result = some (Value.ofInt cur.ty next).ty := by
            rw [← hid]
            rw [hcurTy]
            simpa [Value.ofInt, hgKind] using hvar
          refine forContinue_preserves (c := c) (f := f) (fr := fr) (span := span)
            (var := var)
            (g := { g with result := some (Value.ofInt cur.ty next) })
            hc hf (by simpa using hgKind) (FrameMatches.setResult hgm hvar' hvwf)
            hcontAfter hcontSpan
            (by simpa [hgKind] using hvar) hint hstop hstep
            (by simpa [hgKind] using hbody) (by simpa [hgKind] using hbelow) hloop ?_
          simpa [State.setVar] using hs
      | global i =>
          rw [hid] at hs
          simp only [State.setVar] at hs
          have hvar' : (frameCtx u g.kind).lookup (.global i) =
              some (Value.ofInt cur.ty next).ty := by
            rw [← hid]
            rw [hcurTy]
            simpa [Value.ofInt, hgKind] using hvar
          have hc' : ConfigTyped u { c with globals := c.globals.set i (Value.ofInt cur.ty next) } :=
            ConfigTyped.setGlobal hc hgm hvar' hvwf
          refine forContinue_preserves (span := span) (var := var)
            (c := { c with globals := c.globals.set i (Value.ofInt cur.ty next) })
            (f := f) (fr := fr) (g := g) hc' (by simpa using hf) hgKind hgm
            hcontAfter hcontSpan
            (by simpa [Value.ofInt, hid, hcurTy] using hvar')
            hint hstop hstep (by simpa [hgKind] using hbody) (by simpa [hgKind] using hbelow) hloop ?_
          simpa [State.setVar] using hs

/-- `FOR` stop-bound evaluation preserves configuration typing. -/
theorem stepValue_forStop_preserves {u : HirUnit} {c c' : State} {f : Frame}
    {fr : List Frame} {k : List Cont} {var : Place} {stepE : Option Expr}
    {body : List Stmt} {ty : Ty} {span : Span} {v : Value}
    (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hk : f.k = Cont.mk (.forStop var stepE body) f.loopCtx f.loopCtx span :: k)
    (hvar : (frameCtx u f.kind).lookup var.id = some ty)
    (hint : ty.isInt = true)
    (hnot : var.id ∉ f.loopCtx.controlled)
    (hstep : ∀ e, stepE = some e → ExprHasType (frameCtx u f.kind) e ty)
    (hzero : ∀ e, stepE = some e → ∀ v, Expr.constInt? e = some v → v ≠ 0)
    (hbody : StmtListHasType (frameCtx u f.kind) (f.loopCtx.depth + 1)
      (var.id :: f.loopCtx.controlled) body)
    (hrest : StackTyped (frameCtx u f.kind) k .unit f.loopCtx)
    (hvwf : v.WellFormed) (hvy : v.ty = ty)
    (hs : stepValue u c v = .step c') : ConfigTyped u c' := by
  have hloop : f.loopCtx.depth > 0 → StackHasLoop k := by
    intro hd
    have h := hc.headLoop f fr hf hd
    rw [hk] at h
    exact StackHasLoop.tail_of_not_marker h
      (by simp [Cont.isLoopMarker, ContKind.isLoopMarker])
  simp only [stepValue, hf, hk] at hs
  cases stepE with
  | none =>
      exact forContinue_preserves (g := { f with k := k })
        (cont := Cont.mk (.forStop var none body) f.loopCtx f.loopCtx span)
        hc hf rfl (FrameMatches.updateK (hc.headMatch f fr hf)) rfl rfl hvar hint ⟨hvwf, hvy⟩
        ⟨Value.one_wellFormed (Value.isCore_of_wellFormed hvwf) (by rw [hvy]; exact hint),
          by simp [Value.ofInt, hvy]⟩ hbody hrest hloop hs
  | some stepExpr =>
      cases hs
      let k' : List Cont := Cont.mk (.forStep var v body) f.loopCtx f.loopCtx span :: k
      have hstack' : StackTyped (frameCtx u f.kind) k' (.value ty) f.loopCtx :=
        StackTyped.forStep hvar hint hnot ⟨hvwf, hvy⟩ hbody hrest
      have hfocus : FocusTyped u { f with k := k' } (.expr stepExpr) :=
        ⟨ty, hstep stepExpr rfl, by simpa [k'] using hstack'⟩
      have hloop' : f.loopCtx.depth > 0 → StackHasLoop k' := by
        intro hd; exact Or.inr (hloop hd)
      exact ConfigTyped.updateHeadFrame hc hf
        (FrameMatches.updateK (hc.headMatch f fr hf)) hfocus
        (FramesShape.updateHead (f := f) (rest := fr) (f' := { f with k := k' })
          (by simpa [hf] using hc.shape) rfl) hloop' rfl

/-- `FOR` step-value evaluation preserves configuration typing. -/
theorem stepValue_forStep_preserves {u : HirUnit} {c c' : State} {f : Frame}
    {fr : List Frame} {k : List Cont} {var : Place} {stopVal : Value}
    {body : List Stmt} {ty : Ty} {span : Span} {v : Value}
    (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hk : f.k = Cont.mk (.forStep var stopVal body) f.loopCtx f.loopCtx span :: k)
    (hvar : (frameCtx u f.kind).lookup var.id = some ty)
    (hint : ty.isInt = true)
    (hnot : var.id ∉ f.loopCtx.controlled)
    (hstop : stopVal.WellFormed ∧ stopVal.ty = ty)
    (hbody : StmtListHasType (frameCtx u f.kind) (f.loopCtx.depth + 1)
      (var.id :: f.loopCtx.controlled) body)
    (hrest : StackTyped (frameCtx u f.kind) k .unit f.loopCtx)
    (hvwf : v.WellFormed) (hvy : v.ty = ty)
    (hs : stepValue u c v = .step c') : ConfigTyped u c' := by
  have hloop : f.loopCtx.depth > 0 → StackHasLoop k := by
    intro hd
    have h := hc.headLoop f fr hf hd
    rw [hk] at h
    exact StackHasLoop.tail_of_not_marker h
      (by simp [Cont.isLoopMarker, ContKind.isLoopMarker])
  simp only [stepValue, hf, hk] at hs
  exact forContinue_preserves (g := { f with k := k })
    (cont := Cont.mk (.forStep var stopVal body) f.loopCtx f.loopCtx span)
    hc hf rfl (FrameMatches.updateK (hc.headMatch f fr hf)) rfl rfl hvar hint hstop ⟨hvwf, hvy⟩
    hbody hrest hloop hs

/-- A value transition preserves configuration typing. -/
theorem stepValue_preserves {u : HirUnit} {env : Env} {c c' : State} {f : Frame}
    {fr : List Frame} {v : Value}
    (hunit : HasType u env) (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hval : FocusTyped u f (.value v))
    (hs : stepValue u c v = .step c') : ConfigTyped u c' := by
  rcases hval with ⟨hvwf, T, hty, hstack⟩
  have hne : f.k ≠ [] := StackTyped.value_ne_nil hstack
  cases hk : f.k with
  | nil => exact absurd hk hne
  | cons cont k =>
      rw [hk] at hstack
      cases hstack with
      | assign hlookup hrest =>
          exact stepValue_assign_preserves hc hf hk hvwf hty hlookup hrest hs
      | unary hop hrest =>
          exact stepValue_unary_preserves hc hf hk hvwf hty hop hrest hs
      | binaryL hrhs hop hres hrest =>
          exact stepValue_binaryL_preserves hc hf hk hvwf hty hrhs hop hres hrest hs
      | binaryR hlhs hlt hop hres hrest =>
          exact stepValue_binaryR_preserves hc hf hk hlhs hlt hvwf hty hop hres hrest hs
      | cast hconv hcore hrest =>
          exact stepValue_cast_preserves hc hf hk hvwf hty hconv hcore hrest hs
      | callArgs hrem htyped hdone hsig hresult hrest =>
          exact stepValue_callArgs_preserves hunit hc hf hk hrem htyped hdone hsig hresult
            hrest hvwf hty hs
      | ifCond harms helse hrest =>
          exact stepValue_ifCond_preserves hc hf hk harms helse hrest hs
      | caseSel hsel hint hlabels hbodies helse hrest =>
          exact stepValue_caseSel_preserves hc hf hk hbodies helse hrest hs
      | whileCond hcond hbody hrest =>
          exact stepValue_whileCond_preserves hc hf hk hcond hbody hrest hs
      | repeatCond hcond hbody hrest =>
          exact stepValue_repeatCond_preserves hc hf hk hcond hbody hrest hs
      | forInit hvar hint hnot hstop hstep hzero hbody hrest =>
          exact stepValue_forInit_preserves hc hf hk hvar hint hnot hstop hstep hzero hbody
            hrest hvwf hty hs
      | forStop hvar hint hnot hstep hzero hbody hrest =>
          exact stepValue_forStop_preserves hc hf hk hvar hint hnot hstep hzero hbody hrest
            hvwf hty hs
      | forStep hvar hint hnot hstop hbody hrest =>
          exact stepValue_forStep_preserves hc hf hk hvar hint hnot hstop hbody hrest
            hvwf hty hs

/-- A frame completion preserves configuration typing. -/
theorem completeFrame_preserves {u : HirUnit} {env : Env} {c c' : State} {f : Frame}
    {rest : List Frame} (hunit : HasType u env) (hc : ConfigTyped u c)
    (hf : c.frames = f :: rest) (hs : completeFrame u c = .step c') :
    ConfigTyped u c' := by
  have hshape : FramesShape (f :: rest) := by simpa [hf] using hc.shape
  have hfm : FrameMatches u f := hc.headMatch f rest hf
  simp only [completeFrame, hf] at hs
  cases hk : f.kind with
  | program =>
      by_cases hle : c.cyclesLeft ≤ 1
      · simp [hk, hle] at hs
        cases hs
        have hc1 := ConfigTyped.updateFocus hc hf (FocusTyped.halted u f)
        simpa [hf] using ConfigTyped.updateCycles hc1 (c.cycles + 1) 0
      · simp [hk, hle] at hs
        cases hs
        have hframe : FrameMatches u
            { f with k := [], expect := none, loopCtx := LoopCtx.top } := by
          simpa [FrameMatches, hk] using hfm
        have hbody : StmtListHasType (frameCtx u .program) 0 [] u.program.body := by
          rw [frameCtx_program, ← hunit.env_eq]
          exact hunit.program.2
        have hbody' : StmtListHasType
            (frameCtx u { f with k := [], expect := none, loopCtx := LoopCtx.top }.kind)
            0 [] u.program.body := by
          simpa [hk] using hbody
        have hfocus : FocusTyped u
            { f with k := [], expect := none, loopCtx := LoopCtx.top }
            (.stmts u.program.body LoopCtx.top) := by
          refine FocusTyped.stmts_of rfl (StackTyped.nil rfl rfl) hbody' ?_
          intro hd
          simp [LoopCtx.top] at hd
        have hc1 := ConfigTyped.updateHeadFrame hc hf hframe hfocus
          (FramesShape.updateHead hshape rfl)
          (by intro hd; simp [LoopCtx.top] at hd) rfl
        simpa [hk] using ConfigTyped.updateCycles hc1 (c.cycles + 1) (c.cyclesLeft - 1)
  | function index =>
      have htail : rest ≠ [] := FramesShape.function_tail_ne_nil hshape ⟨index, hk⟩
      cases rest with
      | nil => exact absurd rfl htail
      | cons g gs =>
          simp [hk] at hs
          cases hs
          have htailTyped : FramesTailTyped u (g :: gs) := hc.tailTyped f (g :: gs) hf
          have hcallers : FramesCallerExpects u (f :: g :: gs) := by
            simpa [hf] using hc.callers
          refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
          · simpa using hc.globals
          · cases hshape with
            | cons _ htailShape => exact htailShape
          · intro g' gs' hg'
            simp only [List.cons.injEq] at hg'
            rcases hg' with ⟨rfl, rfl⟩
            exact htailTyped.1.1
          · intro g' gs' hg'
            simp only [List.cons.injEq] at hg'
            rcases hg' with ⟨rfl, rfl⟩
            exact FocusTyped.returned_of_caller hfm htailTyped.1 hcallers.1 rfl
          · intro g' gs' hg'
            simp only [List.cons.injEq] at hg'
            rcases hg' with ⟨rfl, rfl⟩
            exact htailTyped.2
          · intro g' gs' hg' hd
            simp only [List.cons.injEq] at hg'
            rcases hg' with ⟨rfl, rfl⟩
            exact htailTyped.1.2.2 hd
          · simpa using hcallers.2

/-- A typed value focus never gets stuck. -/
theorem stepValue_not_stuck {u : HirUnit} {c : State} {f : Frame} {rest : List Frame}
    {v : Value} (hf : c.frames = f :: rest) (hfm : FrameMatches u f)
    (h : v.WellFormed ∧ ∃ T, v.ty = T ∧
        StackTyped (frameCtx u f.kind) f.k (.value T) f.loopCtx) :
    stepValue u c v ≠ .stuck := by
  rcases h with ⟨_hvwf, T, _hty, hstack⟩
  have hne : f.k ≠ [] := StackTyped.value_ne_nil hstack
  simp only [stepValue, hf]
  cases hk : f.k with
  | nil => exact absurd hk hne
  | cons cont rest =>
      rw [hk] at hstack
      simp only [hk]
      cases hstack with
      | assign hlookup hrest => simp [stepValue, hf, hk]
      | unary hop hrest =>
          simp only [stepValue, hf, hk]
          split <;> simp
      | binaryL hrhs hop hres hrest => simp [stepValue, hf, hk]
      | binaryR hlhs hty hop hres hrest =>
          simp only [stepValue, hf, hk]
          split <;> simp
      | cast hconv hcore hrest =>
          simp only [stepValue, hf, hk]
          split <;> simp
      | callArgs hrem htyped hdone hsig hresult hrest =>
          simp only [stepValue, hf, hk]
          split
          · exfalso; exact hrem rfl
          · simp only [performCall]
            split
            · split
              · simp
              · split <;> simp
            · simp only [CalleeSignatureV] at hsig
              rcases hsig with ⟨sig, hsigfn, _hargs⟩
              obtain ⟨fn, hfn⟩ := frameCtx_functions_lookup hfm hsigfn
              simp [performCall, hfn, hf]
          · simp
      | ifCond harms helse hrest =>
          simp only [stepValue, hf, hk]
          repeat' split
          all_goals simp
      | caseSel hsel hint hlabels hbodies helse hrest =>
          simp only [stepValue, hf, hk]
          split <;> simp
      | whileCond hcond hbody hrest =>
          simp only [stepValue, hf, hk]
          split <;> simp
      | repeatCond hcond hbody hrest =>
          simp only [stepValue, hf, hk]
          split <;> simp
      | forInit hvar hint hnot hstop hstep hzero hbody hrest =>
          simp only [stepValue, hf, hk]
          split
          · next heq => exfalso; exact State.setVar_frames_ne_nil (by rw [hf]; simp) heq
          · simp
      | forStop hvar hint hnot hstep hzero hbody hrest =>
          simp only [stepValue, hf, hk, forContinue]
          repeat' split
          all_goals simp
      | forStep hvar hint hnot hstop hbody hrest =>
          simp only [stepValue, hf, hk, forContinue]
          repeat' split
          all_goals simp

/-- One statement transition preserves configuration typing. -/
theorem stepStmt_preserves {u : HirUnit} {env : Env} {c c' : State} {f : Frame}
    {fr : List Frame} {s : Stmt} {restStmts : List Stmt} {lc : LoopCtx}
    (hunit : HasType u env) (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hfocus : FocusTyped u f (.stmts (s :: restStmts) lc))
    (hs : stepStmt u c s restStmts lc = .step c') : ConfigTyped u c' := by
  rcases hfocus with ⟨hlc, hstack, hlist, hloopStmts⟩
  subst lc
  have hfm := hc.headMatch f fr hf
  have hshape : FramesShape (f :: fr) := by simpa [hf] using hc.shape
  have hstmt : StmtHasType (frameCtx u f.kind) f.loopCtx.depth f.loopCtx.controlled s :=
    hlist s (by simp)
  have hrestList : StmtListHasType (frameCtx u f.kind) f.loopCtx.depth
      f.loopCtx.controlled restStmts :=
    fun stmt hmem => hlist stmt (by simp [hmem])
  simp only [stepStmt, hf] at hs
  cases s with
  | assign place value span =>
      cases hstmt with
      | assign hnotControlled hplace hvalue =>
          cases hs
          let seqCont : Cont := Cont.same (.seq restStmts) f.loopCtx span
          let k' : List Cont := Cont.same (.assign place) f.loopCtx span :: seqCont :: f.k
          have hseq : StackTyped (frameCtx u f.kind) (seqCont :: f.k) .unit f.loopCtx :=
            StackTyped.seq hrestList (by simpa [seqCont] using hstack)
          have hstack' : StackTyped (frameCtx u f.kind) k' (.value _) f.loopCtx :=
            StackTyped.assign hplace (by simpa [k', seqCont] using hseq)
          have hfocus' : FocusTyped u { f with k := k' } (.expr value) :=
            ⟨_, hvalue, by simpa [k'] using hstack'⟩
          have hloop' : f.loopCtx.depth > 0 → StackHasLoop k' := by
            intro hd; exact Or.inr (Or.inr (hloopStmts hd))
          exact ConfigTyped.updateHeadFrame (f' := { f with k := k' }) hc hf
            (FrameMatches.updateK hfm) hfocus'
            (FramesShape.updateHeadK hshape k') hloop' rfl
  | call call callSpan =>
      cases call with
      | call callee args innerSpan =>
          cases hstmt with
          | call hcall =>
              let seqCont : Cont := Cont.same (.seq restStmts) f.loopCtx callSpan
              have hseq : StackTyped (frameCtx u f.kind) (seqCont :: f.k) .unit f.loopCtx :=
                StackTyped.seq hrestList (by simpa [seqCont] using hstack)
              cases args with
              | nil =>
                  simp only [State.updateHead, hf] at hs
                  obtain ⟨result, hsig⟩ :
                      ∃ result, CalleeSignatureV (frameCtx u f.kind) callee [] [] result := by
                    rcases callHasType_cases hcall with
                      ⟨idx, cspan, sig, hcallee, hsigfn, hres, hargs⟩ |
                      ⟨fn, cspan, ty, hcallee, hresOuter, hstd⟩
                    · subst hcallee
                      have hnil : sig.params = [] := Forall2.nil_left hargs
                      exact ⟨sig.result, sig, hsigfn, by simp [hnil], rfl⟩
                    · subst hcallee
                      rcases stdFnHasType_cases hstd with ⟨argTys, hargs, hres⟩
                      have hnil : argTys = [] := Forall2.nil_left hargs
                      subst hnil
                      exact ⟨some ty, by
                        simp only [CalleeSignatureV, List.append_nil, List.map_nil]
                        exact hres⟩
                  exact performCall_preserves (s := c) (f := f) (fr := fr)
                    (caller := { f with k := seqCont :: f.k }) (k := seqCont :: f.k)
                    (result := result) (isStmt := true)
                    hunit hc hf rfl rfl (FrameMatches.updateK hfm)
                    (by intro hd; exact Or.inr (hloopStmts hd)) hsig
                    (by intro h; cases h) (by intro v hv; cases hv) hseq hs
              | cons arg more =>
                  cases hs
                  let k' : List Cont :=
                    Cont.same (.callArgs callee [] (arg :: more) true) f.loopCtx innerSpan ::
                      seqCont :: f.k
                  rcases callArgsPushStmt hcall hseq with ⟨t, harg, hstackCont⟩
                  have hfocus' : FocusTyped u { f with k := k' } (.expr arg) :=
                    ⟨t, harg, by simpa [k'] using hstackCont⟩
                  have hloop' : f.loopCtx.depth > 0 → StackHasLoop k' := by
                    intro hd; exact Or.inr (Or.inr (hloopStmts hd))
                  exact ConfigTyped.updateHeadFrame (f' := { f with k := k' }) hc hf
                    (FrameMatches.updateK hfm) hfocus'
                    (FramesShape.updateHeadK hshape k') hloop' rfl
      | _ => cases hstmt
  | return_ span => exact completeFrame_preserves hunit hc hf hs
  | exit span =>
      cases hstmt with
      | exit hdepth =>
          have hloopStack : StackHasLoop f.k := hloopStmts (by simpa using hdepth)
          obtain ⟨marker, below, hfind⟩ := StackHasLoop.findLoop_some hloopStack
          simp only [hfind] at hs
          cases hs
          obtain ⟨hbelow, hloopBelow, _hmarker⟩ := StackTyped.findLoop_marker hstack hfind
          exact ConfigTyped.updateHeadFrame (f' := { f with k := below, loopCtx := marker.after })
            hc hf (FrameMatches.updateKLoop hfm) (FocusTyped.resume_of hbelow)
            (FramesShape.updateHead (f := f) (rest := fr)
              (f' := { f with k := below, loopCtx := marker.after }) hshape rfl)
            hloopBelow rfl
  | continue_ span =>
      cases hstmt with
      | continue_ hdepth =>
          have hloopStack : StackHasLoop f.k := hloopStmts (by simpa using hdepth)
          obtain ⟨marker, below, hfind⟩ := StackHasLoop.findLoop_some hloopStack
          obtain ⟨hbelow, hloopBelow, hmarker⟩ := StackTyped.findLoop_marker hstack hfind
          simp only [hfind] at hs
          cases hk : marker.kind with
          | whileLoop cond body =>
              simp only [MarkerTyped, hk] at hmarker
              rcases hmarker with ⟨_hmark, hcond, hbodyk⟩
              simp only [hk] at hs
              cases hs
              let k' : List Cont :=
                Cont.same (.whileCond cond body) marker.after marker.span :: below
              have hstack' : StackTyped (frameCtx u f.kind) k' (.value .bool) marker.after :=
                StackTyped.whileCond hcond hbodyk hbelow
              have hfocus' : FocusTyped u { f with k := k', loopCtx := marker.after }
                  (.expr cond) :=
                ⟨.bool, hcond, by simpa [k'] using hstack'⟩
              exact ConfigTyped.updateHeadFrame
                (f' := { f with k := k', loopCtx := marker.after }) hc hf
                (FrameMatches.updateKLoop hfm) hfocus'
                (FramesShape.updateHead (f := f) (rest := fr)
                  (f' := { f with k := k', loopCtx := marker.after }) hshape rfl)
                (by intro hd; exact Or.inr (hloopBelow hd)) rfl
          | repeatBody body cond =>
              simp only [MarkerTyped, hk] at hmarker
              rcases hmarker with ⟨_hmark, hcond, hbodyk⟩
              simp only [hk] at hs
              cases hs
              let k' : List Cont :=
                Cont.same (.repeatCond body cond) marker.after marker.span :: below
              have hstack' : StackTyped (frameCtx u f.kind) k' (.value .bool) marker.after :=
                StackTyped.repeatCond hcond hbodyk hbelow
              have hfocus' : FocusTyped u { f with k := k', loopCtx := marker.after }
                  (.expr cond) :=
                ⟨.bool, hcond, by simpa [k'] using hstack'⟩
              exact ConfigTyped.updateHeadFrame
                (f' := { f with k := k', loopCtx := marker.after }) hc hf
                (FrameMatches.updateKLoop hfm) hfocus'
                (FramesShape.updateHead (f := f) (rest := fr)
                  (f' := { f with k := k', loopCtx := marker.after }) hshape rfl)
                (by intro hd; exact Or.inr (hloopBelow hd)) rfl
          | forBody var stopVal stepVal body =>
              simp only [MarkerTyped, hk] at hmarker
              rcases hmarker with
                ⟨_hmark, ty, hvar, hint, hstopWF, hstopTy, hstepWF, hstepTy, hbodyk⟩
              simp only [hk] at hs
              have hcur : ∀ w,
                  ({ c with frames := { f with k := below, loopCtx := marker.after } :: fr }).lookupVar?
                      var.id = some w → w.ty = ty ∧ w.WellFormed := by
                intro w hw
                have hlookEq :
                    ({ c with frames := { f with k := below, loopCtx := marker.after } :: fr }).lookupVar?
                        var.id = c.lookupVar? var.id := by
                  cases hid : var.id <;>
                    simp [State.lookupVar?, State.head?, hf, hid]
                rw [hlookEq] at hw
                obtain ⟨v, hv, hty, hwf⟩ := ConfigTyped.lookupVar?_typed hc hf hvar
                rw [hv] at hw
                cases hw
                exact ⟨hty, hwf⟩
              exact forAdvance_preserves (c := c) (f := f) (fr := fr)
                (g := { f with k := below, loopCtx := marker.after })
                (cont := marker) (lc := marker.after) (span := marker.span)
                hc hf rfl (FrameMatches.updateKLoop hfm)
                (by intro hd; exact hloopBelow hd) rfl rfl
                hvar hint ⟨hstopWF, hstopTy⟩ ⟨hstepWF, hstepTy⟩ hbodyk hbelow hloopBelow
                hcur hs
          | _ => simp [hk] at hs
  | empty span =>
      cases hs
      simpa [hf] using ConfigTyped.updateFocus (focus := .stmts restStmts f.loopCtx) hc hf
        ⟨rfl, hstack, hrestList, hloopStmts⟩
  | ifThenElse arms elseBody span =>
      cases hstmt with
      | ifThenElse harmsCond harmsBody helse =>
          let seqCont : Cont := Cont.same (.seq restStmts) f.loopCtx span
          have hseq : StackTyped (frameCtx u f.kind) (seqCont :: f.k) .unit f.loopCtx :=
            StackTyped.seq hrestList (by simpa [seqCont] using hstack)
          have helse' : StmtListHasType (frameCtx u f.kind) f.loopCtx.depth
              f.loopCtx.controlled elseBody := fun stmt hmem => helse stmt hmem
          cases arms with
          | nil =>
              cases hs
              let k' : List Cont := seqCont :: f.k
              have hfocus' : FocusTyped u { f with k := k' } (.stmts elseBody f.loopCtx) :=
                ⟨rfl, by simpa [k'] using hseq, helse', by intro hd; exact Or.inr (hloopStmts hd)⟩
              exact ConfigTyped.updateHeadFrame (f' := { f with k := k' }) hc hf
                (FrameMatches.updateK hfm) hfocus'
                (FramesShape.updateHeadK hshape k')
                (by intro hd; exact Or.inr (hloopStmts hd)) rfl
          | cons arm remaining =>
              cases hs
              let k' : List Cont :=
                Cont.same (.ifCond (arm :: remaining) elseBody) f.loopCtx span :: seqCont :: f.k
              have hcondArm : ExprHasType (frameCtx u f.kind) arm.cond .bool :=
                harmsCond arm (by simp)
              have harms' : ∀ a ∈ arm :: remaining,
                  ExprHasType (frameCtx u f.kind) a.cond .bool ∧
                    StmtListHasType (frameCtx u f.kind) f.loopCtx.depth f.loopCtx.controlled a.body :=
                fun a ha => ⟨harmsCond a (by simp [ha]), fun stmt hmem => harmsBody a (by simp [ha]) stmt hmem⟩
              have hstack' : StackTyped (frameCtx u f.kind) k' (.value .bool) f.loopCtx :=
                StackTyped.ifCond harms' helse' (by simpa [k', seqCont] using hseq)
              have hfocus' : FocusTyped u { f with k := k' } (.expr arm.cond) :=
                ⟨.bool, hcondArm, by simpa [k'] using hstack'⟩
              have hloop' : f.loopCtx.depth > 0 → StackHasLoop k' := by
                intro hd; exact Or.inr (Or.inr (hloopStmts hd))
              exact ConfigTyped.updateHeadFrame (f' := { f with k := k' }) hc hf
                (FrameMatches.updateK hfm) hfocus'
                (FramesShape.updateHeadK hshape k') hloop' rfl
  | case selector arms elseBody span =>
      cases hstmt with
      | case hselector hint hlabels hbodies helse =>
          cases hs
          let seqCont : Cont := Cont.same (.seq restStmts) f.loopCtx span
          have hseq : StackTyped (frameCtx u f.kind) (seqCont :: f.k) .unit f.loopCtx :=
            StackTyped.seq hrestList (by simpa [seqCont] using hstack)
          let k' : List Cont :=
            Cont.same (.caseSel selector arms elseBody) f.loopCtx span :: seqCont :: f.k
          have hbodies' : ∀ a ∈ arms, StmtListHasType (frameCtx u f.kind)
              f.loopCtx.depth f.loopCtx.controlled a.body :=
            fun a ha => fun stmt hmem => hbodies a ha stmt hmem
          have helse' : StmtListHasType (frameCtx u f.kind) f.loopCtx.depth
              f.loopCtx.controlled elseBody := fun stmt hmem => helse stmt hmem
          have hstack' : StackTyped (frameCtx u f.kind) k' (.value _) f.loopCtx :=
            StackTyped.caseSel hselector hint hlabels hbodies' helse'
              (by simpa [k', seqCont] using hseq)
          have hfocus' : FocusTyped u { f with k := k' } (.expr selector) :=
            ⟨_, hselector, by simpa [k'] using hstack'⟩
          have hloop' : f.loopCtx.depth > 0 → StackHasLoop k' := by
            intro hd; exact Or.inr (Or.inr (hloopStmts hd))
          exact ConfigTyped.updateHeadFrame (f' := { f with k := k' }) hc hf
            (FrameMatches.updateK hfm) hfocus'
            (FramesShape.updateHeadK hshape k') hloop' rfl
  | forLoop var start stop stepE body span =>
      cases hstmt with
      | forLoop hnotControlled hvar hint hstart hstop hstep hzero hbodyStmts =>
          cases hs
          let seqCont : Cont := Cont.same (.seq restStmts) f.loopCtx span
          have hseq : StackTyped (frameCtx u f.kind) (seqCont :: f.k) .unit f.loopCtx :=
            StackTyped.seq hrestList (by simpa [seqCont] using hstack)
          let k' : List Cont :=
            Cont.same (.forInit var stop stepE body) f.loopCtx span :: seqCont :: f.k
          have hstack' : StackTyped (frameCtx u f.kind) k' (.value _) f.loopCtx :=
            StackTyped.forInit hvar hint hnotControlled hstop hstep hzero
              (fun stmt hmem => hbodyStmts stmt hmem)
              (by simpa [k', seqCont] using hseq)
          have hfocus' : FocusTyped u { f with k := k' } (.expr start) :=
            ⟨_, hstart, by simpa [k'] using hstack'⟩
          have hloop' : f.loopCtx.depth > 0 → StackHasLoop k' := by
            intro hd; exact Or.inr (Or.inr (hloopStmts hd))
          exact ConfigTyped.updateHeadFrame (f' := { f with k := k' }) hc hf
            (FrameMatches.updateK hfm) hfocus'
            (FramesShape.updateHeadK hshape k') hloop' rfl
  | whileLoop cond body span =>
      cases hstmt with
      | whileLoop hcond hbodyStmts =>
          cases hs
          let seqCont : Cont := Cont.same (.seq restStmts) f.loopCtx span
          have hseq : StackTyped (frameCtx u f.kind) (seqCont :: f.k) .unit f.loopCtx :=
            StackTyped.seq hrestList (by simpa [seqCont] using hstack)
          let k' : List Cont :=
            Cont.same (.whileCond cond body) f.loopCtx span :: seqCont :: f.k
          have hstack' : StackTyped (frameCtx u f.kind) k' (.value .bool) f.loopCtx :=
            StackTyped.whileCond hcond (fun stmt hmem => hbodyStmts stmt hmem)
              (by simpa [k', seqCont] using hseq)
          have hfocus' : FocusTyped u { f with k := k' } (.expr cond) :=
            ⟨.bool, hcond, by simpa [k'] using hstack'⟩
          have hloop' : f.loopCtx.depth > 0 → StackHasLoop k' := by
            intro hd; exact Or.inr (Or.inr (hloopStmts hd))
          exact ConfigTyped.updateHeadFrame (f' := { f with k := k' }) hc hf
            (FrameMatches.updateK hfm) hfocus'
            (FramesShape.updateHeadK hshape k') hloop' rfl
  | repeatLoop body cond span =>
      cases hstmt with
      | repeatLoop hbodyStmts hcond =>
          cases hs
          let seqCont : Cont := Cont.same (.seq restStmts) f.loopCtx span
          have hseq : StackTyped (frameCtx u f.kind) (seqCont :: f.k) .unit f.loopCtx :=
            StackTyped.seq hrestList (by simpa [seqCont] using hstack)
          let bodyLc : LoopCtx := LoopCtx.body f.loopCtx f.loopCtx.controlled
          let k' : List Cont :=
            Cont.mk (.repeatBody body cond) bodyLc f.loopCtx span :: seqCont :: f.k
          have hbody' : StmtListHasType (frameCtx u f.kind) bodyLc.depth bodyLc.controlled body := by
            intro stmt hmem
            simpa [bodyLc, LoopCtx.body] using hbodyStmts stmt hmem
          have hloopSeq : f.loopCtx.depth > 0 → StackHasLoop (seqCont :: f.k) := by
            intro hd; exact Or.inr (hloopStmts hd)
          have hstack' : StackTyped (frameCtx u f.kind) k' .unit bodyLc :=
            StackTyped.repeatBody (by simp [bodyLc, LoopCtx.body])
              (by simp [bodyLc, LoopCtx.body]) hcond hbody' hloopSeq
              (by simpa [k', seqCont] using hseq)
          have hfocus' : FocusTyped u { f with k := k', loopCtx := bodyLc }
              (.stmts body bodyLc) :=
            ⟨rfl, by simpa [k'] using hstack', hbody', by intro _; exact Or.inl rfl⟩
          exact ConfigTyped.updateHeadFrame (f' := { f with k := k', loopCtx := bodyLc }) hc hf
            (FrameMatches.updateKLoop hfm) hfocus'
            (FramesShape.updateHead (f := f) (rest := fr)
              (f' := { f with k := k', loopCtx := bodyLc }) hshape rfl)
            (by intro _; exact Or.inl rfl) rfl

/-- A statement list transition preserves configuration typing. -/
theorem stepStmts_preserves {u : HirUnit} {env : Env} {c c' : State} {f : Frame}
    {fr : List Frame} {ss : List Stmt} {lc : LoopCtx}
    (hunit : HasType u env) (hc : ConfigTyped u c) (hf : c.frames = f :: fr)
    (hfocus : FocusTyped u f (.stmts ss lc))
    (hs : stepStmts u c ss lc = .step c') : ConfigTyped u c' := by
  rcases hfocus with ⟨hlc, hstack, hlist, hloopStmts⟩
  subst lc
  simp only [stepStmts, hf] at hs
  cases ss with
  | nil =>
      cases hs
      simpa [hf] using ConfigTyped.updateFocus (focus := .resume) hc hf hstack
  | cons s restStmts =>
      exact stepStmt_preserves hunit hc hf ⟨rfl, hstack, hlist, hloopStmts⟩ hs

/-! ## Progress support: frame completion and loop markers -/

/-- A frame completion never gets stuck for a shaped stack. -/
theorem completeFrame_not_stuck {u : HirUnit} {c : State} {f : Frame} {rest : List Frame}
    (hf : c.frames = f :: rest) (hshape : FramesShape (f :: rest)) :
    completeFrame u c ≠ .stuck := by
  simp only [completeFrame, hf]
  cases hk : f.kind with
  | program =>
      by_cases hle : c.cyclesLeft ≤ 1 <;> simp [hle]
  | function index =>
      have htail : rest ≠ [] := FramesShape.function_tail_ne_nil hshape ⟨index, hk⟩
      cases rest with
      | nil => exact absurd rfl htail
      | cons g gs => simp

/-- `findLoop` returns a loop marker. -/
theorem findLoop_marker {k : List Cont} {cont : Cont} {rest : List Cont}
    (h : findLoop k = some (cont, rest)) : cont.isLoopMarker = true := by
  induction k with
  | nil => simp [findLoop] at h
  | cons c cs ih =>
      by_cases hm : c.isLoopMarker = true
      · simp [findLoop, hm] at h
        rcases h with ⟨hc, _⟩
        subst hc
        exact hm
      · simp [findLoop, hm] at h
        exact ih h

/-- The marker continuation kinds are the loop forms. -/
theorem ContKind.loopMarker_cases {kind : ContKind} (h : kind.isLoopMarker = true) :
    (∃ cond body, kind = .whileLoop cond body) ∨
    (∃ body cond, kind = .repeatBody body cond) ∨
    (∃ var stopVal stepVal body, kind = .forBody var stopVal stepVal body) := by
  cases kind with
  | whileLoop cond body => exact Or.inl ⟨cond, body, rfl⟩
  | repeatBody body cond => exact Or.inr (Or.inl ⟨body, cond, rfl⟩)
  | forBody var stopVal stepVal body =>
      exact Or.inr (Or.inr ⟨var, stopVal, stepVal, body, rfl⟩)
  | _ => simp [ContKind.isLoopMarker] at h

/-- A typed statement never gets stuck. -/
theorem stepStmt_not_stuck {u : HirUnit} {c : State} {f : Frame} {rest : List Frame}
    {s : Stmt} {restStmts : List Stmt} {lc : LoopCtx}
    (hf : c.frames = f :: rest) (hshape : FramesShape (f :: rest))
    (hfm : FrameMatches u f)
    (h : f.loopCtx = lc ∧ StackTyped (frameCtx u f.kind) f.k .unit lc ∧
        StmtListHasType (frameCtx u f.kind) lc.depth lc.controlled (s :: restStmts) ∧
        (lc.depth > 0 → StackHasLoop f.k)) :
    stepStmt u c s restStmts lc ≠ .stuck := by
  rcases h with ⟨hlc, hstack, hlist, hloop⟩
  have hstmt : StmtHasType (frameCtx u f.kind) lc.depth lc.controlled s :=
    hlist s (by simp)
  simp only [stepStmt, hf]
  cases s with
  | assign place value span => simp
  | call call callSpan =>
      cases call with
      | call callee args innerSpan =>
          cases args with
          | nil =>
              obtain ⟨result, hcall⟩ : ∃ result,
                  CallHasType (frameCtx u f.kind) callee [] result := by
                cases hstmt with | call hc => exact ⟨_, hc⟩
              cases callee with
              | std fn sp =>
                  simp only [performCall]
                  split
                  · simp
                  · split <;> simp
              | function index sp =>
                  rcases callHasType_cases hcall with
                    ⟨idx, cspan, sig, hcallee, hsig, hres, hargs⟩ |
                    ⟨fn, cspan, ty, hcallee, hres, hstd⟩
                  · cases hcallee
                    obtain ⟨fn, hfn⟩ := frameCtx_functions_lookup hfm hsig
                    simp [performCall, State.updateHead, hf, hfn]
                  · cases hcallee
          | cons arg more => simp
      | _ => cases hstmt
  | return_ span => exact completeFrame_not_stuck hf hshape
  | exit span =>
      have hdepth : 0 < lc.depth := by cases hstmt with | exit hd => exact hd
      obtain ⟨marker, below, hfind⟩ := StackHasLoop.findLoop_some (hloop hdepth)
      simp only [hfind]
      simp
  | continue_ span =>
      have hdepth : 0 < lc.depth := by cases hstmt with | continue_ hd => exact hd
      obtain ⟨cont, below, hfind⟩ := StackHasLoop.findLoop_some (hloop hdepth)
      have hmarker : cont.isLoopMarker = true := findLoop_marker hfind
      rcases ContKind.loopMarker_cases hmarker with
        ⟨cond, body, hkind⟩ | ⟨body, cond, hkind⟩ |
        ⟨var, stopVal, stepVal, body, hkind⟩
      · rw [hfind]; simp [hkind]
      · rw [hfind]; simp [hkind]
      · rw [hfind]
        simp only [hkind, forAdvance, forContinue]
        repeat' split
        all_goals simp
  | empty span => simp
  | ifThenElse arms elseBody span =>
      cases arms with
      | nil => simp
      | cons arm remaining => simp
  | case selector arms elseBody span => simp
  | forLoop var start stop stepE body span => simp
  | whileLoop cond body span => simp
  | repeatLoop body cond span => simp

/-- A typed statement list never gets stuck. -/
theorem stepStmts_not_stuck {u : HirUnit} {c : State} {f : Frame} {rest : List Frame}
    {ss : List Stmt} {lc : LoopCtx}
    (hf : c.frames = f :: rest) (hshape : FramesShape (f :: rest))
    (hfm : FrameMatches u f)
    (h : f.loopCtx = lc ∧ StackTyped (frameCtx u f.kind) f.k .unit lc ∧
        StmtListHasType (frameCtx u f.kind) lc.depth lc.controlled ss ∧
        (lc.depth > 0 → StackHasLoop f.k)) :
    stepStmts u c ss lc ≠ .stuck := by
  simp only [stepStmts, hf]
  cases ss with
  | nil => simp
  | cons s restStmts =>
      exact stepStmt_not_stuck hf hshape hfm h

/-- A typed resume focus never gets stuck. -/
theorem stepResume_not_stuck {u : HirUnit} {c : State} {f : Frame} {rest : List Frame}
    (hf : c.frames = f :: rest) (hshape : FramesShape (f :: rest))
    (h : StackTyped (frameCtx u f.kind) f.k .unit f.loopCtx) :
    stepResume u c ≠ .stuck := by
  simp only [stepResume, hf]
  cases hk : f.k with
  | nil => exact completeFrame_not_stuck hf hshape
  | cons cont rest =>
      rw [hk] at h
      simp only [hk]
      cases h with
      | seq hstmts hrest => simp
      | whileLoop hdepth hctrl hcond hbody hrest => simp
      | repeatBody hdepth hctrl hcond hbody hrest => simp
      | forBody hdepth hctrl hvar hint hstop hstep hbody hrest =>
          simp only [forAdvance, forContinue]
          repeat' split
          all_goals simp

/-- A returned focus never gets stuck. -/
theorem stepReturned_not_stuck {u : HirUnit} {c : State} {f : Frame} {rest : List Frame}
    {rv : Option Value} (hf : c.frames = f :: rest)
    (h : FocusTyped u f (.returned rv)) :
    stepReturned u c rv ≠ .stuck := by
  simp only [stepReturned, hf]
  cases hk : f.k with
  | nil => simp
  | cons cont rest =>
      simp only [hk]
      by_cases hw : cont.wantsValue = true
      · cases rv with
        | none =>
            exfalso
            have hu : StackTyped (frameCtx u f.kind) (cont :: rest) .unit f.loopCtx := by
              simpa [FocusTyped, hk] using h
            have hfalse := StackTyped.unit_head hu
            simp [hw] at hfalse
        | some v => simp [hw]
      · simp [hw]

/-! ## Progress -/

/-- A well-typed configuration is terminal or takes a step. -/
theorem progress {u : HirUnit} {c : State} (h : ConfigTyped u c) :
    Terminal u c ∨ ∃ c', Step u c c' := by
  have hnot : step u c ≠ .stuck := by
    obtain ⟨f, rest, hf⟩ := h.head_exists
    have hfm := h.headMatch f rest hf
    have hfocus := h.headFocus f rest hf
    have hshape : FramesShape (f :: rest) := by simpa [hf] using h.shape
    cases hfoc : c.focus with
    | halted => simp [step, hfoc]
    | trapped trap span => simp [step, hfoc]
    | expr e =>
        have he : ∃ T, ExprHasType (frameCtx u f.kind) e T ∧
            StackTyped (frameCtx u f.kind) f.k (.value T) f.loopCtx := by
          simpa [hfoc, FocusTyped] using hfocus
        rw [show step u c = stepExpr u c e by simp [step, hfoc]]
        exact stepExpr_not_stuck hf h hfm he
    | value v =>
        have hv : v.WellFormed ∧ ∃ T, v.ty = T ∧
            StackTyped (frameCtx u f.kind) f.k (.value T) f.loopCtx := by
          simpa [hfoc, FocusTyped] using hfocus
        rw [show step u c = stepValue u c v by simp [step, hfoc]]
        exact stepValue_not_stuck hf hfm hv
    | stmts ss lc =>
        have hs : f.loopCtx = lc ∧ StackTyped (frameCtx u f.kind) f.k .unit lc ∧
            StmtListHasType (frameCtx u f.kind) lc.depth lc.controlled ss ∧
            (lc.depth > 0 → StackHasLoop f.k) := by
          simpa [hfoc, FocusTyped] using hfocus
        rw [show step u c = stepStmts u c ss lc by simp [step, hfoc]]
        exact stepStmts_not_stuck hf hshape hfm hs
    | resume =>
        have hs : StackTyped (frameCtx u f.kind) f.k .unit f.loopCtx := by
          simpa [hfoc, FocusTyped] using hfocus
        rw [show step u c = stepResume u c by simp [step, hfoc]]
        exact stepResume_not_stuck hf hshape hs
    | returned rv =>
        have hs : FocusTyped u f (.returned rv) := by simpa [hfoc, FocusTyped] using hfocus
        rw [show step u c = stepReturned u c rv by simp [step, hfoc]]
        exact stepReturned_not_stuck hf hs
  cases hs : step u c with
  | halt => exact Or.inl hs
  | step c' => exact Or.inr ⟨c', hs⟩
  | stuck => exact False.elim (hnot hs)

/-! ## Determinism -/

/-- A terminal state has no successors. -/
theorem Steps.of_terminal {u : HirUnit} {c c' : State} (t : Terminal u c)
    (h : Steps u c c') : c' = c := by
  induction h with
  | refl => rfl
  | step hs _ _ =>
      unfold Step at hs
      rw [t] at hs
      cases hs

/--  is a function, so its transitive closure is deterministic. -/
theorem eval_deterministic {u : HirUnit} {c c1 c2 : State}
    (h1 : Steps u c c1) (h2 : Steps u c c2)
    (t1 : Terminal u c1) (t2 : Terminal u c2) : c1 = c2 := by
  induction h1 generalizing c2 with
  | refl =>
      exact (Steps.of_terminal t1 h2).symm
  | step hs _ ih =>
      cases h2 with
      | refl =>
          exfalso
          unfold Step at hs
          rw [t2] at hs
          cases hs
      | step hs2 htail2 =>
          unfold Step at hs hs2
          rw [hs] at hs2
          have hb := Outcome.step.inj hs2
          cases hb
          exact ih htail2 t1 t2

/-! ## Trap soundness -/

/-- Every terminal trap is one of the specified traps. -/
theorem trap_soundness {u : HirUnit} {c c' : State} (_h : Steps u c c')
    {trap : Trap} {span : Span} (_hf : c'.focus = .trapped trap span) :
    trap = .overflow ∨ trap = .divZero ∨ trap = .range := by
  cases trap with
  | overflow => exact Or.inl rfl
  | divZero => exact Or.inr (Or.inl rfl)
  | range => exact Or.inr (Or.inr rfl)

/-! ## Exact arithmetic -/

/-- A mapped error is the original error. -/
theorem Except.map_error {ε α β : Type} {f : α → β} {e : Except ε α} {t : ε}
    (h : e.map f = .error t) : e = .error t := by
  cases e with
  | error e' => cases h; rfl
  | ok a => simp [Except.map] at h

/-- Checked source arithmetic and conversion agree exactly with the 
models, including the checked conversion guard. -/
theorem checked_arith_traps_exactly :
    (∀ (ty : Ty) (a b : Int), evalBinary .add ty a b = (ty.layout.add a b).map (Value.ofInt ty)) ∧
    (∀ (ty : Ty) (a b : Int), evalBinary .sub ty a b = (ty.layout.sub a b).map (Value.ofInt ty)) ∧
    (∀ (ty : Ty) (a b : Int), evalBinary .mul ty a b = (ty.layout.mul a b).map (Value.ofInt ty)) ∧
    (∀ (ty : Ty) (a b : Int), evalBinary .div ty a b = (ty.layout.div a b).map (Value.ofInt ty)) ∧
    (∀ (ty : Ty) (a b : Int), evalBinary .mod ty a b = (ty.layout.mod a b).map (Value.ofInt ty)) ∧
    (∀ (ty : Ty) (a : Int), evalUnary .neg ty a = (ty.layout.neg a).map (Value.ofInt ty)) ∧
    (∀ (src to : Ty) (a : Int), evalCast src to a =
      (match src.layout.convert to.layout a with
        | .error trap => .error trap
        | .ok r => if to.layout.fits r then .ok (Value.ofInt to r) else .error .range)) :=
  ⟨fun _ _ _ => rfl, fun _ _ _ => rfl, fun _ _ _ => rfl, fun _ _ _ => rfl,
    fun _ _ _ => rfl, fun _ _ => rfl, fun _ _ _ => rfl⟩

/-- Checked addition traps exactly when the exact sum is out of range. -/
theorem checked_add_traps_iff {ty : Ty} {a b : Int} :
    evalBinary .add ty a b = .error .overflow ↔ ty.layout.fits (a + b) = false := by
  constructor
  · intro h
    have h' : ty.layout.add a b = .error .overflow := Except.map_error h
    exact (Stc.Exact.IntType.checked_overflow).mp h'
  · intro h
    have h' : ty.layout.add a b = .error .overflow := (Stc.Exact.IntType.checked_overflow).mpr h
    show (ty.layout.add a b).map (Value.ofInt ty) = .error .overflow
    rw [h']
    rfl

/-- Checked division traps exactly on a zero divisor or signed minimum overflow. -/
theorem checked_div_traps_iff {ty : Ty} {a b : Int} {trap : Trap} :
    evalBinary .div ty a b = .error trap ↔ ty.layout.div a b = .error trap := by
  constructor
  · intro h; exact Except.map_error h
  · intro h
    show (ty.layout.div a b).map (Value.ofInt ty) = .error trap
    rw [h]; rfl


end Stc.Sem
