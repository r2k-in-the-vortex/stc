/-
Constant evaluation for the Phase 2 folded expression subset.

The model is deliberately smaller than the compiler's `consteval.rs`: it covers
literals, constants visible in a `ConstWorld`, unary and binary operators,
explicit casts, and the exact standard functions. Function calls, real values,
date/time values, and source-syntax folding are outside the modeled subset and
return `unsupported` or are handled by a later phase.

`constEval` is the executable evaluator. `ConstEvalRel` is the declarative
relation over the same expression grammar and Phase 0 evaluators. Soundness
shows the algorithm's successful result is admitted by the relation; type
preservation shows the result has the expression's declared type and is well
formed.
-/
import Stc.Sem.Typing
import Stc.Sem.Values

namespace Stc.ConstEval

open Stc.Hir
open Stc.Sem
open Stc.Syntax

/-- A failure of constant evaluation. Traps are the Phase 0 exact traps. -/
inductive ConstError where
  /-- A variable is not bound in the typing context. -/
  | unbound (id : VarId)
  /-- A variable is bound but has no known constant value. -/
  | notConstant (id : VarId)
  /-- Two operands of a binary expression do not have a common type. -/
  | typeMismatch
  /-- A literal or variable payload is malformed. -/
  | malformed
  /-- An exact operation trapped. -/
  | trap (trap : Trap)
  /-- A user-function call is not part of the folded subset. -/
  | unsupportedCall
deriving Repr, DecidableEq

/-- The constant world visible to the evaluator: the typing context plus the
constant values of variables. `value_typed` is the invariant that every known
constant has the context's declared type and is well formed. -/
structure ConstWorld where
  /-- The typing context of the expression. -/
  ctx : Ctx
  /-- Known constant values, by resolved variable. -/
  value : VarId → Option Value
  /-- Every known constant has its declared type and is well formed. -/
  value_typed : ∀ {id ty v}, ctx.lookup id = some ty → value id = some v →
    v.ty = ty ∧ v.WellFormed

/- Evaluate an expression and an argument list mutually, left to right. -/
mutual
  /-- Evaluate one expression in a constant world. -/
  def constEval (w : ConstWorld) : Expr → Except ConstError Value
    | .literal lit _ =>
        if lit.WellFormed then .ok ⟨lit.ty, lit.value⟩
        else .error .malformed
    | .var id _ =>
        match w.ctx.lookup id with
        | none => .error (.unbound id)
        | some _ =>
            match w.value id with
            | none => .error (.notConstant id)
            | some v => .ok v
    | .unary op operand _ =>
        match constEval w operand with
        | .error e => .error e
        | .ok v =>
            match evalUnary op v.ty v.value with
            | .error trap => .error (.trap trap)
            | .ok result => .ok result
    | .binary op lhs rhs _ =>
        match constEval w lhs with
        | .error e => .error e
        | .ok a =>
            match constEval w rhs with
            | .error e => .error e
            | .ok b =>
                if a.ty = b.ty then
                  match evalBinary op a.ty a.value b.value with
                  | .error trap => .error (.trap trap)
                  | .ok result => .ok result
                else .error .typeMismatch
    | .cast operand to _ _ =>
        match constEval w operand with
        | .error e => .error e
        | .ok v =>
            match evalCast v.ty to v.value with
            | .error trap => .error (.trap trap)
            | .ok result => .ok result
    | .call (.std fn _) args _ =>
        match evalArgs w args with
        | .error e => .error e
        | .ok values =>
            match evalStd fn values with
            | .error trap => .error (.trap trap)
            | .ok result => .ok result
    | .call (.function _ _) _ _ => .error .unsupportedCall

  /-- Evaluate an argument list left to right. -/
  def evalArgs (w : ConstWorld) : List Expr → Except ConstError (List Value)
    | [] => .ok []
    | arg :: rest =>
        match constEval w arg with
        | .error e => .error e
        | .ok value =>
            match evalArgs w rest with
            | .error e => .error e
            | .ok values => .ok (value :: values)
end

/- The declarative constant-evaluation relation. Each rule is one Phase 0
evaluation step; the relation does not prescribe an evaluation order. -/
mutual
inductive ConstEvalRel (w : ConstWorld) : Expr → Value → Prop where
  /-- A well-formed literal evaluates to its payload. -/
  | literal {lit : Literal} {span : Span} (h : lit.WellFormed) :
      ConstEvalRel w (.literal lit span) ⟨lit.ty, lit.value⟩
  /-- A variable evaluates to its known constant. -/
  | var {id : VarId} {span : Span} {ty : Ty} {v : Value}
      (hlook : w.ctx.lookup id = some ty) (hvalue : w.value id = some v) :
      ConstEvalRel w (.var id span) v
  /-- A unary operation evaluates its operand and then the Phase 0 unary
  evaluator. -/
  | unary {op : UnaryOp} {operand : Expr} {span : Span} {v result : Value}
      (hoperand : ConstEvalRel w operand v)
      (hstep : evalUnary op v.ty v.value = .ok result) :
      ConstEvalRel w (.unary op operand span) result
  /-- A binary operation evaluates both operands of a common type and then the
  Phase 0 binary evaluator. -/
  | binary {op : BinaryOp} {lhs rhs : Expr} {span : Span} {a b result : Value}
      (hlhs : ConstEvalRel w lhs a) (hrhs : ConstEvalRel w rhs b)
      (hty : a.ty = b.ty)
      (hstep : evalBinary op a.ty a.value b.value = .ok result) :
      ConstEvalRel w (.binary op lhs rhs span) result
  /-- A cast evaluates its operand and then the Phase 0 cast evaluator. -/
  | cast {operand : Expr} {to : Ty} {kind : CastKind} {span : Span}
      {v result : Value}
      (hoperand : ConstEvalRel w operand v)
      (hstep : evalCast v.ty to v.value = .ok result) :
      ConstEvalRel w (.cast operand to kind span) result
  /-- A standard-function call evaluates its arguments and then the Phase 0
  standard-function evaluator. -/
  | std {fn : StdFn} {cspan : Span} {args : List Expr} {span : Span}
      {values : List Value} {result : Value}
      (hargs : ConstEvalRelArgs w args values)
      (hstep : evalStd fn values = .ok result) :
      ConstEvalRel w (.call (.std fn cspan) args span) result

/- The argument-list companion of `ConstEvalRel`. -/
inductive ConstEvalRelArgs (w : ConstWorld) : List Expr → List Value → Prop where
  /-- The empty argument list. -/
  | nil : ConstEvalRelArgs w [] []
  /-- A head and tail argument list. -/
  | cons {arg : Expr} {args : List Expr} {value : Value} {values : List Value}
      (harg : ConstEvalRel w arg value)
      (hargs : ConstEvalRelArgs w args values) :
      ConstEvalRelArgs w (arg :: args) (value :: values)
end

/-! ## Helpers for type preservation -/

/-- A well-typed binary operation's two operand types are equal. -/
theorem binaryWellTyped_rhs_eq {op : BinaryOp} {lhs rhs : Ty}
    (h : binaryWellTyped op lhs rhs = true) : rhs = lhs := by
  cases op with
  | add | sub | mul | div | mod | pow =>
      simp [binaryWellTyped] at h
      exact h.1.symm
  | and | or | xor =>
      simp [binaryWellTyped] at h
      exact h.1.symm
  | eq | ne | lt | le | gt | ge =>
      simp [binaryWellTyped] at h
      exact h.symm

/-- A type appearing in a standard-function result is an argument type. -/
theorem stdResult?_mem {fn : StdFn} {argTys : List Ty} {ty : Ty}
    (h : stdResult? fn argTys = some ty) : ty ∈ argTys := by
  unfold stdResult? at h
  split at h <;> simp_all

/-- A standard-function result type is an argument type, hence core whenever
every argument type is core. -/
theorem stdResult?_core {fn : StdFn} {argTys : List Ty} {ty : Ty}
    (h : stdResult? fn argTys = some ty)
    (hcore : ∀ t ∈ argTys, t.isCore = true) : ty.isCore = true :=
  hcore ty (stdResult?_mem h)

/-- Every type in a well-formed value list is core. -/
theorem Forall2.types_core {values : List Value} {tys : List Ty}
    (h : Forall2 (fun v t => v.ty = t ∧ v.WellFormed) values tys) :
    ∀ t ∈ tys, t.isCore = true := by
  induction h with
  | nil => simp
  | cons hhead htail ih =>
      intro t ht
      rcases List.mem_cons.mp ht with rfl | ht
      · rw [← hhead.1]
        exact Value.isCore_of_wellFormed hhead.2
      · exact ih t ht

/-! ## Soundness of the executable evaluator -/

mutual
  /-- A successful constant evaluation is admitted by the declarative
  relation. -/
  theorem constEval_sound (w : ConstWorld) :
      ∀ (e : Expr) {v : Value}, constEval w e = .ok v → ConstEvalRel w e v
    | .literal lit span, v, h => by
        simp only [constEval] at h
        split at h
        · next hwf =>
            cases h
            exact .literal hwf
        · cases h
    | .var id span, v, h => by
        simp only [constEval] at h
        split at h
        · cases h
        · next ty hlook =>
            split at h
            · cases h
            · next found hvalue =>
                cases h
                exact .var hlook hvalue
    | .unary op operand span, v, h => by
        cases hoperand : constEval w operand with
        | error e => simp [constEval, hoperand] at h
        | ok value =>
            simp only [constEval, hoperand] at h
            split at h
            · cases h
            · next result hstep =>
                cases h
                exact .unary (constEval_sound w operand hoperand) hstep
    | .binary op lhs rhs span, v, h => by
        cases hlhs : constEval w lhs with
        | error e => simp [constEval, hlhs] at h
        | ok a =>
            cases hrhs : constEval w rhs with
            | error e => simp [constEval, hlhs, hrhs] at h
            | ok b =>
                simp only [constEval, hlhs, hrhs] at h
                by_cases hty : a.ty = b.ty
                · simp only [hty, if_true] at h
                  split at h
                  · cases h
                  · next result hstep =>
                      cases h
                      rw [← hty] at hstep
                      exact .binary (constEval_sound w lhs hlhs)
                        (constEval_sound w rhs hrhs) hty hstep
                · simp only [hty, ite_false] at h
                  cases h
    | .cast operand to kind span, v, h => by
        cases hoperand : constEval w operand with
        | error e => simp [constEval, hoperand] at h
        | ok value =>
            simp only [constEval, hoperand] at h
            split at h
            · cases h
            · next result hstep =>
                cases h
                exact .cast (constEval_sound w operand hoperand) hstep
    | .call (.std fn cspan) args span, v, h => by
        cases hargs : evalArgs w args with
        | error e => simp [constEval, hargs] at h
        | ok values =>
            simp only [constEval, hargs] at h
            split at h
            · cases h
            · next result hstep =>
                cases h
                exact .std (evalArgs_sound w args hargs) hstep
    | .call (.function index cspan) args span, v, h => by
        simp [constEval] at h

  /-- A successful argument-list evaluation is admitted by the declarative
  argument relation. -/
  theorem evalArgs_sound (w : ConstWorld) :
      ∀ (es : List Expr) {vs : List Value},
        evalArgs w es = .ok vs → ConstEvalRelArgs w es vs
    | [], vs, h => by
        simp [evalArgs] at h
        cases h
        exact .nil
    | arg :: rest, vs, h => by
        cases harg : constEval w arg with
        | error e => simp [evalArgs, harg] at h
        | ok value =>
            cases hrest : evalArgs w rest with
            | error e => simp [evalArgs, harg, hrest] at h
            | ok values =>
                simp [evalArgs, harg, hrest] at h
                cases h
                exact .cons (constEval_sound w arg harg) (evalArgs_sound w rest hrest)
end

/-! ## Type preservation -/

/-- The relation-typing motive for expressions. -/
def RelTyped (w : ConstWorld) (e : Expr) (v : Value)
    (_h : ConstEvalRel w e v) : Prop :=
  ∀ {ty : Ty}, ExprHasType w.ctx e ty → v.ty = ty ∧ v.WellFormed

/-- The relation-typing motive for argument lists. -/
def RelArgsTyped (w : ConstWorld) (es : List Expr) (vs : List Value)
    (_h : ConstEvalRelArgs w es vs) : Prop :=
  ∀ {tys : List Ty}, Forall2 (ExprHasType w.ctx) es tys →
    Forall2 (fun v t => v.ty = t ∧ v.WellFormed) vs tys

/-- The declarative relation is type preserving. -/
theorem ConstEvalRel_typed (w : ConstWorld) {e : Expr} {v : Value}
    (h : ConstEvalRel w e v) : RelTyped w e v h := by
  refine ConstEvalRel.rec (motive_1 := RelTyped w) (motive_2 := RelArgsTyped w)
    ?_ ?_ ?_ ?_ ?_ ?_ ?_ ?_ h
  · intro lit span hwf
    unfold RelTyped
    intro ty' htyTy
    cases htyTy with
    | literal hwf' =>
        exact ⟨rfl, Value.wellFormed_of (Literal.isCore_of_wellFormed hwf')
          (Literal.holds_of_wellFormed hwf')⟩
  · intro id span tyLookup v hlook hvalue
    unfold RelTyped
    intro ty' htyTy
    cases htyTy with
    | var hlookTy => exact w.value_typed hlookTy hvalue
  · intro op operand span v result hrel hstepEq ihRel
    unfold RelTyped
    intro ty' htyTy
    cases htyTy with
    | unary hop hoperandTy =>
        have ih' := ihRel hoperandTy
        have hcore : ty'.isCore = true := by
          rw [← ih'.1]
          exact Value.isCore_of_wellFormed ih'.2
        have hfit : ty'.Holds v.value := by
          rw [← ih'.1]
          exact Value.holds_of_wellFormed ih'.2
        rw [ih'.1] at hstepEq
        exact evalUnary_ok hcore hop hfit hstepEq
  · intro op lhs rhs span a b result hrelL hrelR hcommon hstepEq ihL ihR
    unfold RelTyped
    intro ty' htyTy
    cases htyTy with
    | binary hop hlhsTy hrhsTy =>
        have ihL' := ihL hlhsTy
        have ihR' := ihR hrhsTy
        have hbty : b.ty = a.ty := by
          rw [ihL'.1, ihR'.1]
          exact binaryWellTyped_rhs_eq hop
        have hcore : a.ty.isCore = true := Value.isCore_of_wellFormed ihL'.2
        have ha : a.ty.Holds a.value := Value.holds_of_wellFormed ihL'.2
        have hb : b.ty.Holds b.value := Value.holds_of_wellFormed ihR'.2
        have hb' : a.ty.Holds b.value := by
          simpa [hbty] using hb
        have hop' : binaryWellTyped op a.ty a.ty = true := by
          rw [ihL'.1]
          simpa [binaryWellTyped_rhs_eq hop] using hop
        have hres := evalBinary_ok hcore hop' ha hb' hstepEq
        constructor
        · rw [hres.1]
          simp [binaryResultTy, ihL'.1]
        · exact hres.2
  · intro operand to kind span v result hrel hstepEq ihRel
    unfold RelTyped
    intro ty' htyTy
    cases htyTy with
    | cast hoperandTy hconv hcore => exact evalCast_ok hcore hstepEq
  · intro fn cspan args span values result hrelArgs hstepEq ihArgs
    unfold RelTyped
    intro ty' htyTy
    cases htyTy with
    | call hcall =>
        cases hcall with
        | std hstd =>
            cases hstd with
            | mk hargTys hres =>
                have ihArgs' := ihArgs hargTys
                have htycore : ty'.isCore = true :=
                  stdResult?_core hres (Forall2.types_core ihArgs')
                exact evalStd_ok ihArgs' hres htycore hstepEq
  · unfold RelArgsTyped
    intro tys hforall
    cases hforall with
    | nil => exact .nil
  · intro arg args value values hrel hrels ihRel ihRels
    unfold RelArgsTyped
    intro tys hforall
    cases hforall with
    | cons hhead htail => exact .cons (ihRel hhead) (ihRels htail)

/-- A successful constant evaluation has the expression's declared type and
is a well-formed value. -/
theorem constEval_type_preserving (w : ConstWorld) {e : Expr} {ty : Ty} {v : Value}
    (hty : ExprHasType w.ctx e ty) (heval : constEval w e = .ok v) :
    v.ty = ty ∧ v.WellFormed :=
  ConstEvalRel_typed w (constEval_sound w e heval) hty

/-! ## Executable smoke test -/

/-- A zero-width test span. -/
def testSpan : Span := { file := ⟨0⟩, start := 0, stop := 0 }

/-- A world with an empty typing context and no constants. -/
def emptyWorld : ConstWorld where
  ctx := { globals := [], functions := [], params := [], locals := [], result := none }
  value := fun _ => none
  value_typed := by
    intro id ty v hlook
    cases id <;> simp [Ctx.lookup] at hlook

/-- The executable evaluator computes a constant `DINT` addition. -/
example :
    constEval emptyWorld
      (.binary .add
        (.literal ⟨.int 32 true, 1⟩ testSpan)
        (.literal ⟨.int 32 true, 2⟩ testSpan) testSpan) =
      .ok ⟨.int 32 true, 3⟩ := by
  rfl

end Stc.ConstEval
