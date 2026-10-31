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
      (hrest : StackTyped ctx rest (.value ty) lc) :
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
      (hrest : StackTyped ctx rest (.value ty) lc) :
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
      (hrest : StackTyped ctx rest .unit after) :
      StackTyped ctx
        ({ kind := .forBody var stopVal stepVal body, before := before, after := after,
           span := span } :: rest)
        .unit before
  /-- Continue the statement list after a statement. -/
  | seq {restStmts : List Stmt} {lc : LoopCtx} {span : Span} {rest : List Cont}
      (hrest : StackTyped ctx rest .unit lc) :
      StackTyped ctx
        ({ kind := .seq restStmts, before := lc, after := lc, span := span } :: rest)
        .unit lc

/-- Whether the stack contains a loop marker. -/
def StackHasLoop : List Cont → Prop
  | [] => False
  | c :: rest => c.isLoopMarker = true ∨ StackHasLoop rest

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

/-- A suspended frame: its stored values match its declarations and its
continuation stack is typed under the expectation the callee will meet. -/
def FrameTyped (u : HirUnit) (f : Frame) : Prop :=
  FrameMatches u f ∧
    StackTyped (frameCtx u f.kind) f.k (.ofOption f.expect) f.loopCtx

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

/-- A stack with a loop marker has a `dropToLoop` result. -/
theorem StackHasLoop.dropToLoop_some {k : List Cont} (h : StackHasLoop k) :
    ∃ k', dropToLoop k = some k' := by
  induction k with
  | nil => cases h
  | cons c rest ih =>
      by_cases hm : c.isLoopMarker = true
      · exact ⟨rest, by simp [dropToLoop, hm]⟩
      · have htail : StackHasLoop rest := by
          simpa [StackHasLoop, hm] using h
        rcases ih htail with ⟨k', hk'⟩
        exact ⟨k', by simp [dropToLoop, hm, hk']⟩

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

/-- A typed variable is present in the runtime store. -/
theorem ConfigTyped.lookupVar?_some {u : HirUnit} {s : State} {f : Frame}
    {rest : List Frame} (hc : ConfigTyped u s) (hf : s.frames = f :: rest)
    {id : VarId} {ty : Ty} (h : (frameCtx u f.kind).lookup id = some ty) :
    ∃ v, s.lookupVar? id = some v := by
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
          rcases List.getElem?_map_some h with ⟨p, hp⟩
          have hi := List.getElem?_isSome hp
          have hi' : i < f.params.length := by omega
          exact ⟨f.params[i], by simp [State.lookupVar?, State.head?, hf, List.getElem?_eq_getElem hi']⟩
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
          rcases List.getElem?_map_some h with ⟨l, hl⟩
          have hi := List.getElem?_isSome hl
          have hi' : i < f.locals.length := by omega
          exact ⟨f.locals[i], by simp [State.lookupVar?, State.head?, hf, List.getElem?_eq_getElem hi']⟩
      | program =>
          rw [hk, frameCtx_program] at h
          simp only [Ctx.ofProgram, Ctx.lookup] at h
          simp only [FrameMatches, hk] at hm
          rcases hm with ⟨_, _, hlen, hloc⟩
          rcases List.getElem?_map_some h with ⟨l, hl⟩
          have hi := List.getElem?_isSome hl
          have hi' : i < f.locals.length := by omega
          exact ⟨f.locals[i], by simp [State.lookupVar?, State.head?, hf, List.getElem?_eq_getElem hi']⟩
  | «global» i =>
      cases hk : f.kind with
      | function index =>
          simp only [FrameMatches, hk] at hm
          rw [hk] at h
          rcases hm with ⟨fn, hfn, _⟩
          rw [frameCtx_function hfn] at h
          simp only [Ctx.ofFunction, Ctx.lookup] at h
          rcases hc.globals with ⟨glen, gok⟩
          rcases List.getElem?_map_some h with ⟨g, hg⟩
          have hi := List.getElem?_isSome hg
          have hi' : i < s.globals.length := by omega
          exact ⟨s.globals[i], by simp [State.lookupVar?, State.head?, List.getElem?_eq_getElem hi']⟩
      | program =>
          rw [hk, frameCtx_program] at h
          simp only [Ctx.ofProgram, Ctx.lookup] at h
          rcases hc.globals with ⟨glen, gok⟩
          rcases List.getElem?_map_some h with ⟨g, hg⟩
          have hi := List.getElem?_isSome hg
          have hi' : i < s.globals.length := by omega
          exact ⟨s.globals[i], by simp [State.lookupVar?, State.head?, List.getElem?_eq_getElem hi']⟩
  | result =>
      cases hk : f.kind with
      | function index =>
          simp only [FrameMatches, hk] at hm
          rw [hk] at h
          rcases hm with ⟨fn, hfn, hplen, hpar, hllen, hloc, hres⟩
          rw [frameCtx_function hfn] at h
          simp only [Ctx.ofFunction, Ctx.lookup] at h
          -- h : fn.resultType = some ty
          cases hrt : fn.resultType with
          | none => rw [hrt] at h; cases h
          | some rty =>
              rw [hrt] at hres
              cases hr : f.result with
              | none => rw [hr] at hres; cases hres
              | some v =>
                  exact ⟨v, by simp [State.lookupVar?, State.head?, hf, hr]⟩
      | program =>
          rw [hk, frameCtx_program] at h
          simp only [Ctx.ofProgram, Ctx.lookup] at h
          cases h

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
          simp [stepValue, hf, hk]
      | forStop hvar hint hnot hstep hzero hbody hrest =>
          simp only [stepValue, hf, hk, forContinue]
          repeat' split
          all_goals simp
      | forStep hvar hint hnot hstop hbody hrest =>
          simp only [stepValue, hf, hk, forContinue]
          repeat' split
          all_goals simp

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
