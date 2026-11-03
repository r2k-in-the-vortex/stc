/-
Refinement propositions for the filtered Aeneas exact model.

This module is compiled only in the non-default `lean/aeneas` package. It
imports the checked-in generated model and the hand-written exact-side models.
Every proposition below is a `def` returning `Prop`: none is asserted as a
theorem unless a proof is completed, and none is a V1 claim. The two proved
facts (`sel_refines`, `compare_refines`) are recorded explicitly; everything
else remains an open obligation in `Bridge/Exact/Audit.lean`.
-/

import StcGenerated.Exact.Model
import Stc.Bridge.Exact.Encode
import Stc.Bridge.Exact.Externals
import Stc.Exact.Bits
import Stc.Exact.Select

namespace Stc.Bridge.Exact

open Aeneas Aeneas.Std

/-! ## Decoding the generated representations -/

/-- Decode the generated `IntType` (a `u8` width and a signed flag). -/
def decodeIntType (ty : exact.IntType) : Stc.Exact.IntType :=
  (IntTypeEncoding.mk ty.width.val ty.signed).toExact

/-- Decode a generated trap. -/
def decodeTrap : exact.Trap → Stc.Exact.Trap
  | .Overflow => .overflow
  | .DivZero => .divZero
  | .Range => .range

/-- Decode a generated comparison selector. -/
def decodeCompare : exact.Compare → Int → Int → Bool
  | .Eq, a, b => decide (a = b)
  | .Ne, a, b => decide (a ≠ b)
  | .Lt, a, b => decide (a < b)
  | .Le, a, b => decide (a ≤ b)
  | .Gt, a, b => decide (a > b)
  | .Ge, a, b => decide (a ≥ b)

/-! ## Result-refinement predicates -/

/-- A generated checked result matches an exact checked result: the success
value is the decoded integer and each generated trap is the decoded exact trap. -/
def CoreResultRefines (g : core.result.Result Std.I128 exact.Trap)
    (expected : Stc.Exact.Res) : Prop :=
  match g with
  | .Ok value => expected = .ok value.val
  | .Err trap => expected = .error (decodeTrap trap)

/-- A generated `Result` computation returns a checked exact result. -/
def ResultRefinesExact
    (r : Result (core.result.Result Std.I128 exact.Trap))
    (expected : Stc.Exact.Res) : Prop :=
  match r.match with
  | .ok g => CoreResultRefines g expected
  | _ => False

/-- A generated `Result Std.I128` returns the expected exact integer. -/
def ResultRefinesInt (r : Result Std.I128) (expected : Int) : Prop :=
  match r.match with
  | .ok value => expected = value.val
  | _ => False

/-- A generated `Result Std.I64` returns the expected exact integer. -/
def ResultRefinesI64 (r : Result Std.I64) (expected : Int) : Prop :=
  match r.match with
  | .ok value => expected = value.val
  | _ => False

/-- A generated `Result Std.U128` returns the expected bit pattern. -/
def ResultRefinesNat (r : Result Std.U128) (expected : Nat) : Prop :=
  match r.match with
  | .ok value => expected = value.val
  | _ => False

/-- A generated `Result Std.U32` returns the expected natural number. -/
def ResultRefinesU32 (r : Result Std.U32) (expected : Nat) : Prop :=
  match r.match with
  | .ok value => expected = value.val
  | _ => False

/-- A generated `Result Bool` returns the expected decision. -/
def ResultRefinesBool (r : Result Bool) (expected : Bool) : Prop :=
  match r.match with
  | .ok value => expected = value
  | _ => False

/-- A generated `Result (Option Std.I128)` returns the expected optional
integer. -/
def ResultRefinesOptionInt (r : Result (Option Std.I128))
    (expected : Option Int) : Prop :=
  match r.match with
  | .ok none => expected = none
  | .ok (some value) => expected = some value.val
  | _ => False

/-! ## External operations -/

/-- The generated `i64::from(u8)` axiom computes the natural embedding. -/
def fromU8Refines : Prop :=
  ∀ n : Std.U8,
    ResultRefinesI64 (exact.I64.Insts.CoreConvertFromU8.from n)
      (Stc.Bridge.Exact.fromU8 n.val)

/-- The generated `i64::rem_euclid` axiom computes Euclidean remainder on its
used domain: nonzero divisor and no `MIN / -1` overflow. -/
def remEuclidRefines : Prop :=
  ∀ a b : Std.I64,
    b.val ≠ 0 →
      ¬ (a.val = Stc.Bridge.Exact.i64Min ∧ b.val = -1) →
        ResultRefinesI64 (exact.core.num.I64.rem_euclid a b)
          (Stc.Bridge.Exact.remEuclid a.val b.val)

/-- The generated `i128::checked_neg` axiom matches the optional exact
negation, failing exactly at `i128::MIN`. -/
def checkedNegRefines : Prop :=
  ∀ a : Std.I128,
    ResultRefinesOptionInt (exact.core.num.I128.checked_neg a)
      (Stc.Bridge.Exact.checkedNeg a.val)

/-- The generated `i128::abs` axiom matches the mathematical absolute value
on the guarded domain used by `exact.abs`. -/
def i128AbsRefines : Prop :=
  ∀ a : Std.I128,
    a.val ≠ Stc.Bridge.Exact.i128Min →
      ResultRefinesInt (exact.core.num.I128.abs a)
        (Stc.Bridge.Exact.iabs a.val)

/-! ## Generated helper operations -/

/-- `IntType.modulus` refines `2 ^ width`, where the generated `i128` shift is
representable. -/
def modulusRefines : Prop :=
  ∀ ty : exact.IntType,
    ty.width.val ≤ 63 →
      ResultRefinesInt (exact.IntType.modulus ty)
        (((decodeIntType ty).modulus : Nat) : Int)

/-- `IntType.minimum` refines the exact minimum. -/
def minimumRefines : Prop :=
  ∀ ty : exact.IntType,
    0 < ty.width.val → ty.width.val ≤ 64 →
      ResultRefinesInt (exact.IntType.minimum ty) (decodeIntType ty).minimum

/-- `IntType.maximum` refines the exact maximum. -/
def maximumRefines : Prop :=
  ∀ ty : exact.IntType,
    0 < ty.width.val → ty.width.val ≤ 64 →
      ResultRefinesInt (exact.IntType.maximum ty) (decodeIntType ty).maximum

/-- `IntType.fits` refines the exact range predicate. -/
def fitsRefines : Prop :=
  ∀ (ty : exact.IntType) (value : Std.I128),
    ResultRefinesBool (exact.IntType.fits ty value)
      ((decodeIntType ty).fits value.val)

/-- `IntType.mask` refines the exact bit mask. -/
def maskRefines : Prop :=
  ∀ ty : exact.IntType,
    ty.width.val ≤ 128 →
      ResultRefinesNat (exact.IntType.mask ty) (decodeIntType ty).mask

/-- `IntType.count` refines the exact masked shift count. -/
def countRefines : Prop :=
  ∀ (ty : exact.IntType) (n : Std.I64),
    0 < ty.width.val →
      ResultRefinesU32 (exact.IntType.count ty n)
        ((decodeIntType ty).count n.val)

/-- `checked` refines the exact checked-result construction. -/
def checkedRefines : Prop :=
  ∀ (ty : exact.IntType) (value : Std.I128),
    ResultRefinesExact (exact.checked ty value)
      ((decodeIntType ty).checked value.val)

/-! ## Checked arithmetic operations -/

/-- `exact.add` refines `Stc.Exact.IntType.add`. -/
def addRefines : Prop :=
  ∀ (ty : exact.IntType) (a b : Std.I128),
    ResultRefinesExact (exact.add ty a b)
      ((decodeIntType ty).add a.val b.val)

/-- `exact.sub` refines `Stc.Exact.IntType.sub`. -/
def subRefines : Prop :=
  ∀ (ty : exact.IntType) (a b : Std.I128),
    ResultRefinesExact (exact.sub ty a b)
      ((decodeIntType ty).sub a.val b.val)

/-- `exact.mul` refines `Stc.Exact.IntType.mul`. -/
def mulRefines : Prop :=
  ∀ (ty : exact.IntType) (a b : Std.I128),
    ResultRefinesExact (exact.mul ty a b)
      ((decodeIntType ty).mul a.val b.val)

/-- `exact.neg` refines `Stc.Exact.IntType.neg`. -/
def negRefines : Prop :=
  ∀ (ty : exact.IntType) (a : Std.I128),
    ResultRefinesExact (exact.neg ty a)
      ((decodeIntType ty).neg a.val)

/-- `exact.div` refines `Stc.Exact.IntType.div`. -/
def divRefines : Prop :=
  ∀ (ty : exact.IntType) (a b : Std.I128),
    ResultRefinesExact (exact.div ty a b)
      ((decodeIntType ty).div a.val b.val)

/-- `exact.modulo` refines `Stc.Exact.IntType.mod`. -/
def moduloRefines : Prop :=
  ∀ (ty : exact.IntType) (a b : Std.I128),
    ResultRefinesExact (exact.modulo ty a b)
      ((decodeIntType ty).mod a.val b.val)

/-- `exact.abs` refines `Stc.Exact.IntType.abs`. -/
def absRefines : Prop :=
  ∀ (ty : exact.IntType) (a : Std.I128),
    ResultRefinesExact (exact.abs ty a)
      ((decodeIntType ty).abs a.val)

/-! ## Bitwise, shift, and rotate operations -/

/-- `exact.bit_and` refines `Stc.Exact.IntType.bitAnd`. -/
def bitAndRefines : Prop :=
  ∀ (ty : exact.IntType) (a b : Std.U128),
    ResultRefinesNat (exact.bit_and ty a b)
      ((decodeIntType ty).bitAnd a.val b.val)

/-- `exact.bit_or` refines `Stc.Exact.IntType.bitOr`. -/
def bitOrRefines : Prop :=
  ∀ (ty : exact.IntType) (a b : Std.U128),
    ResultRefinesNat (exact.bit_or ty a b)
      ((decodeIntType ty).bitOr a.val b.val)

/-- `exact.bit_xor` refines `Stc.Exact.IntType.bitXor`. -/
def bitXorRefines : Prop :=
  ∀ (ty : exact.IntType) (a b : Std.U128),
    ResultRefinesNat (exact.bit_xor ty a b)
      ((decodeIntType ty).bitXor a.val b.val)

/-- `exact.bit_not` refines `Stc.Exact.IntType.bitNot`. -/
def bitNotRefines : Prop :=
  ∀ (ty : exact.IntType) (a : Std.U128),
    ResultRefinesNat (exact.bit_not ty a)
      ((decodeIntType ty).bitNot a.val)

/-- `exact.shl` refines `Stc.Exact.IntType.shl`. -/
def shlRefines : Prop :=
  ∀ (ty : exact.IntType) (a : Std.U128) (n : Std.I64),
    ResultRefinesNat (exact.shl ty a n)
      ((decodeIntType ty).shl a.val n.val)

/-- `exact.shr` refines `Stc.Exact.IntType.shr`. -/
def shrRefines : Prop :=
  ∀ (ty : exact.IntType) (a : Std.U128) (n : Std.I64),
    ResultRefinesNat (exact.shr ty a n)
      ((decodeIntType ty).shr a.val n.val)

/-- `exact.rol` refines `Stc.Exact.IntType.rol`. -/
def rolRefines : Prop :=
  ∀ (ty : exact.IntType) (a : Std.U128) (n : Std.I64),
    ResultRefinesNat (exact.rol ty a n)
      ((decodeIntType ty).rol a.val n.val)

/-- `exact.ror` refines `Stc.Exact.IntType.ror`. -/
def rorRefines : Prop :=
  ∀ (ty : exact.IntType) (a : Std.U128) (n : Std.I64),
    ResultRefinesNat (exact.ror ty a n)
      ((decodeIntType ty).ror a.val n.val)

/-! ## Comparison and selection -/

/-- `exact.compare` refines scalar comparison. -/
def compareRefines : Prop :=
  ∀ (op : exact.Compare) (a b : Std.I128),
    ResultRefinesBool (exact.compare op a b)
      (decodeCompare op a.val b.val)

/-- `exact.sel` refines `Stc.Exact.sel`. -/
def selRefines : Prop :=
  ∀ (g : Bool) (a b : Std.I128),
    ResultRefinesInt (exact.sel g a b) (Stc.Exact.sel g a.val b.val)

/-! ## Proved obligations -/

/-- Comparison is a pure generated dispatch and its decoded form is the exact
comparison; the generated model's `Result` is `ok` in every branch. -/
theorem compare_refines : compareRefines := by
  unfold compareRefines
  intro op a b
  cases op <;>
    simp [ResultRefinesBool, decodeCompare, exact.compare]
  · exact (IScalar.eq_equiv a b).symm
  · by_cases h : a.val = b.val
    · have hab : a = b := IScalar.eq_of_val_eq h
      simp [hab]
    · have hne : a ≠ b := fun hab => h (IScalar.val_eq_of_eq hab)
      simp [h, hne]

/-- Selection is a pure generated dispatch and its decoded form is
`Stc.Exact.sel`. -/
theorem sel_refines : selRefines := by
  unfold selRefines
  intro g a b
  cases g <;>
    simp [ResultRefinesInt, exact.sel, Stc.Exact.sel]

end Stc.Bridge.Exact
