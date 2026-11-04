/-
Refinement proofs for the filtered Aeneas exact model.

This module is compiled only in the non-default `lean/aeneas` package. It
proves the refinement propositions stated in `Bridge/Exact/Refinement.lean`,
operation by operation, against the generated model and the exact-side models.

Trust boundary (TCB): the generated model leaves the four external operations
(`i64::from(u8)`, `i64::rem_euclid`, `i128::checked_neg`, `i128::abs`) as Lean
axioms with no generated body. The exact-side models and guard facts live in
`Stc.Bridge.Exact.Externals`; the four equations between the generated axioms
and those models are declared below as TCB assumptions. They cannot be proved
inside Lean because the generated functions are opaque. Everything else in this
module is proved.

No proof holes are used; the proof-hole grep covers this directory.
-/

import Bridge.Exact.Refinement
import Aeneas.Tactic.Solver.Arith.Lemmas

open Aeneas Aeneas.Std

namespace Stc.Bridge.Exact

/-! ## Turning weakest-precondition specs into result refinements -/

/-- A `Result Std.I128` spec yields the integer result refinement. -/
theorem ResultRefinesInt.of_spec {r : Result Std.I128} {expected : Int}
    (h : r ⦃ v => expected = v.val ⦄) : ResultRefinesInt r expected := by
  unfold ResultRefinesInt
  cases r using Result.cases with
  | ret v => simpa using h
  | vis i k => simp at h
  | div => simp at h

/-- A checked-result spec yields the exact-result refinement. -/
theorem ResultRefinesExact.of_spec
    {r : Result (core.result.Result Std.I128 exact.Trap)} {expected : Stc.Exact.Res}
    (h : r ⦃ g => CoreResultRefines g expected ⦄) : ResultRefinesExact r expected := by
  unfold ResultRefinesExact
  cases r using Result.cases with
  | ret g => simpa using h
  | vis i k => simp at h
  | div => simp at h

/-- A `Result Std.U128` spec yields the natural-number result refinement. -/
theorem ResultRefinesNat.of_spec {r : Result Std.U128} {expected : Nat}
    (h : r ⦃ v => expected = v.val ⦄) : ResultRefinesNat r expected := by
  unfold ResultRefinesNat
  cases r using Result.cases with
  | ret v => simpa using h
  | vis i k => simp at h
  | div => simp at h

/-- A `Result Std.U32` spec yields the natural-number result refinement. -/
theorem ResultRefinesU32.of_spec {r : Result Std.U32} {expected : Nat}
    (h : r ⦃ v => expected = v.val ⦄) : ResultRefinesU32 r expected := by
  unfold ResultRefinesU32
  cases r using Result.cases with
  | ret v => simpa using h
  | vis i k => simp at h
  | div => simp at h

/-- A `Result Bool` spec yields the decision refinement. -/
theorem ResultRefinesBool.of_spec {r : Result Bool} {expected : Bool}
    (h : r ⦃ v => expected = v ⦄) : ResultRefinesBool r expected := by
  unfold ResultRefinesBool
  cases r using Result.cases with
  | ret v => simpa using h
  | vis i k => simp at h
  | div => simp at h

/-- A `Result Std.I64` spec yields the `i64` result refinement. -/
theorem ResultRefinesI64.of_spec {r : Result Std.I64} {expected : Int}
    (h : r ⦃ v => expected = v.val ⦄) : ResultRefinesI64 r expected := by
  unfold ResultRefinesI64
  cases r using Result.cases with
  | ret v => simpa using h
  | vis i k => simp at h
  | div => simp at h

/-! ## Trust-boundary assumptions for the four external operations -/

/-- TCB assumption: the generated `i64::from(u8)` axiom computes the natural
embedding modelled in `Stc.Bridge.Exact.fromU8`. -/
axiom fromU8_refines : fromU8Refines

/-- TCB assumption: the generated `i64::rem_euclid` axiom computes Euclidean
remainder on the nonzero, non-`MIN / -1` domain modelled in
`Stc.Bridge.Exact.remEuclid`. -/
axiom remEuclid_refines : remEuclidRefines

/-- TCB assumption: the generated `i128::checked_neg` axiom matches the optional
negation modelled in `Stc.Bridge.Exact.checkedNeg`. -/
axiom checkedNeg_refines : checkedNegRefines

/-- TCB assumption: the generated `i128::abs` axiom matches the mathematical
absolute value on the guarded domain modelled in `Stc.Bridge.Exact.iabs`. -/
axiom i128Abs_refines : i128AbsRefines

/-! ## Helper operations -/

/-- `2 ^ n` is unchanged by the `i128` bounded-modulus normal form for the
widths used by the kernel. -/
theorem bmod_two_pow {n : Nat} (hn : n ≤ 126) :
    Int.bmod ((2 : Int) ^ n) I128.size = ((2 : Int) ^ n) := by
  rw [show I128.size = 2 ^ 128 by simp [I128.size, I128.numBits]]
  apply Aeneas.Arith.Int.bmod_pow2_eq_of_inBounds' 128 _ (by norm_num)
  · have hle : ((2 : Int) ^ n) ≤ (2 : Int) ^ 126 := by gcongr; omega
    have hlt : ((2 : Int) ^ 126) < (2 : Int) ^ 127 := by norm_num
    have hpos : (0 : Int) < (2 : Int) ^ n := by positivity
    omega
  · have hle : ((2 : Int) ^ n) ≤ (2 : Int) ^ 126 := by gcongr; omega
    have hlt : ((2 : Int) ^ 126) < (2 : Int) ^ 127 := by norm_num
    omega

/-- `modulus` refines `2 ^ width`. -/
@[step] theorem modulus_spec (ty : exact.IntType) (hw : WellFormed ty) :
    exact.IntType.modulus ty ⦃ v => (((decodeIntType ty).modulus : Nat) : Int) = v.val ⦄ := by
  rcases hw with ⟨hv0, hv64⟩
  unfold exact.IntType.modulus
  step as ⟨ i, hi ⟩
  have hshift : ((1 : Int) <<< ty.width.val) = ((2 ^ ty.width.val : Nat) : Int) := by
    rw [Int.shiftLeft_eq]; simp
  have hbmod : Int.bmod ((2 ^ ty.width.val : Nat) : Int) I128.size =
      ((2 ^ ty.width.val : Nat) : Int) := by
    rw [show I128.size = 2 ^ 128 by simp [I128.size, I128.numBits]]
    apply Aeneas.Arith.Int.bmod_pow2_eq_of_inBounds' 128 _ (by norm_num)
    · have hle : ((2 ^ ty.width.val : Nat) : Int) ≤ ((2 ^ 64 : Nat) : Int) := by
        exact_mod_cast Nat.pow_le_pow_right (by norm_num) (by omega)
      have h63 : ((2 ^ 64 : Nat) : Int) < ((2 ^ 127 : Nat) : Int) := by norm_num
      have hpos : (0 : Int) < ((2 ^ ty.width.val : Nat) : Int) := by
        exact_mod_cast Nat.two_pow_pos ty.width.val
      omega
    · have hle : ((2 ^ ty.width.val : Nat) : Int) ≤ ((2 ^ 64 : Nat) : Int) := by
        exact_mod_cast Nat.pow_le_pow_right (by norm_num) (by omega)
      have h63 : ((2 ^ 64 : Nat) : Int) < ((2 ^ 127 : Nat) : Int) := by norm_num
      omega
  rw [hi, hshift, hbmod]
  simp [decodeIntType, IntTypeEncoding.toExact, Stc.Exact.IntType.modulus]

/-- `minimum` refines the exact minimum. -/
@[step] theorem minimum_spec (ty : exact.IntType) (hw : WellFormed ty) :
    exact.IntType.minimum ty ⦃ v => (decodeIntType ty).minimum = v.val ⦄ := by
  rcases hw with ⟨hw0, hw64⟩
  unfold exact.IntType.minimum
  by_cases hs : ty.signed
  · simp only [hs, ite_true]
    step as ⟨ i, hi ⟩
    step as ⟨ i1, h1 ⟩
    have hpos : (0 : Int) < i1.val := by
      rw [h1, Int.shiftLeft_eq, one_mul]
      rw [bmod_two_pow (by omega)]
      positivity
    have hne : i1 ≠ IScalar.min IScalarTy.I128 := by
      intro hmin
      rw [hmin] at hpos
      norm_num [I128.min, I128.numBits] at hpos
    step as ⟨ v, hv ⟩
    rw [hv, h1, Int.shiftLeft_eq, one_mul]
    rw [bmod_two_pow (by omega)]
    have hi' : i.val = ty.width.val - 1 := by simpa using hi
    rw [hi']
    simp [decodeIntType, IntTypeEncoding.toExact, Stc.Exact.IntType.minimum, hs]
  · rw [ite_eq_right hs]
    simp [decodeIntType, IntTypeEncoding.toExact, Stc.Exact.IntType.minimum, hs]

/-- `maximum` refines the exact maximum. -/
@[step] theorem maximum_spec (ty : exact.IntType) (hw : WellFormed ty) :
    exact.IntType.maximum ty ⦃ v => (decodeIntType ty).maximum = v.val ⦄ := by
  have hw0 : 0 < ty.width.val := hw.1
  have hw64 : ty.width.val ≤ 64 := hw.2
  unfold exact.IntType.maximum
  by_cases hs : ty.signed
  · simp only [hs, ite_true]
    step as ⟨ i, hi ⟩
    step as ⟨ i1, h1 ⟩
    have hval : i1.val = (2 : Int) ^ (ty.width.val - 1) := by
      rw [h1, Int.shiftLeft_eq, one_mul]
      rw [bmod_two_pow (by omega)]
      have hi' : i.val = ty.width.val - 1 := by simpa using hi
      rw [hi']
    have hge1 : (1 : Int) ≤ (2 : Int) ^ (ty.width.val - 1) := by
      exact_mod_cast Nat.one_le_two_pow (n := ty.width.val - 1)
    have hlo : IScalar.min IScalarTy.I128 ≤ i1.val - 1 := by
      simp [I128.min, I128.numBits]
      rw [hval]
      omega
    have hhi : i1.val - 1 ≤ IScalar.max IScalarTy.I128 := by
      have hpow : (2 : Int) ^ (ty.width.val - 1) ≤ (2 : Int) ^ 63 := by
        exact_mod_cast Nat.pow_le_pow_right (by norm_num) (by omega : ty.width.val - 1 ≤ 63)
      have hnum : (2 : Int) ^ 63 ≤ (2 : Int) ^ 127 - 1 := by norm_num
      simp [I128.max, I128.numBits]
      rw [hval]
      omega
    step as ⟨ v, hv ⟩
    rw [hv, hval]
    simp [decodeIntType, IntTypeEncoding.toExact, Stc.Exact.IntType.maximum, hs]
  · have hs0 : ty.signed = false := by simpa using hs
    rw [ite_eq_right hs]
    step as ⟨ i, hi ⟩
    step as ⟨ v, hv ⟩
    rw [hv, ← hi]
    simp [decodeIntType, IntTypeEncoding.toExact, Stc.Exact.IntType.maximum,
      Stc.Exact.IntType.modulus, hs0]

/-- `fits` refines the exact range predicate. -/
@[step] theorem fits_spec (ty : exact.IntType) (value : Std.I128) (hw : WellFormed ty) :
    exact.IntType.fits ty value ⦃ b => ((decodeIntType ty).fits value.val) = b ⦄ := by
  unfold exact.IntType.fits
  step* <;> simp_all [Stc.Exact.IntType.fits]

/-! ## Range bounds of production layouts

`WellFormed` keeps every layout strictly inside the shared `i128` kernel: the
minimum is above `i128::MIN` and the maximum is below `i128::MAX`. Those two
facts turn a failed `i128` intermediate into an exact-side overflow, which is
what lets `checked`/`add`/`sub`/`mul` refine through their `i128` guards. -/

/-- The exact minimum of a production layout lies strictly above `i128::MIN`. -/
theorem i128Min_lt_minimum {ty : exact.IntType} (hw : WellFormed ty) :
    Stc.Bridge.Exact.i128Min < (decodeIntType ty).minimum := by
  rcases hw with ⟨hw0, hw64⟩
  have hmin : (decodeIntType ty).minimum =
      if ty.signed then -((2 : Int) ^ (ty.width.val - 1)) else 0 := rfl
  rw [hmin]
  unfold Stc.Bridge.Exact.i128Min
  by_cases hs : ty.signed
  · rw [ite_eq_left hs]
    have hpow : (2 : Int) ^ (ty.width.val - 1) ≤ (2 : Int) ^ 63 := by
      exact_mod_cast Nat.pow_le_pow_right (by norm_num) (by omega : ty.width.val - 1 ≤ 63)
    have h63 : (2 : Int) ^ 63 < (2 : Int) ^ 127 := by norm_num
    omega
  · rw [ite_eq_right hs]
    norm_num

/-- The exact maximum of a production layout lies strictly below `i128::MAX`. -/
theorem maximum_lt_i128Max {ty : exact.IntType} (hw : WellFormed ty) :
    (decodeIntType ty).maximum < Stc.Bridge.Exact.i128Max := by
  rcases hw with ⟨hw0, hw64⟩
  have hmax : (decodeIntType ty).maximum =
      if ty.signed then (2 : Int) ^ (ty.width.val - 1) - 1
      else (2 : Int) ^ ty.width.val - 1 := rfl
  rw [hmax]
  unfold Stc.Bridge.Exact.i128Max
  by_cases hs : ty.signed
  · rw [ite_eq_left hs]
    have hpow : (2 : Int) ^ (ty.width.val - 1) ≤ (2 : Int) ^ 63 := by
      exact_mod_cast Nat.pow_le_pow_right (by norm_num) (by omega : ty.width.val - 1 ≤ 63)
    have h63 : (2 : Int) ^ 63 < (2 : Int) ^ 127 := by norm_num
    omega
  · rw [ite_eq_right hs]
    have hpow : (2 : Int) ^ ty.width.val ≤ (2 : Int) ^ 64 := by
      exact_mod_cast Nat.pow_le_pow_right (by norm_num) hw64
    have h64 : (2 : Int) ^ 64 < (2 : Int) ^ 127 := by norm_num
    omega

/-- `Stc.Bridge.Exact.i128Min` is the generated `IScalar.min` at `I128`. -/
theorem i128Min_eq_IScalar_min : Stc.Bridge.Exact.i128Min = IScalar.min IScalarTy.I128 := by
  unfold Stc.Bridge.Exact.i128Min
  rw [IScalar.min_IScalarTy_I128_eq, I128.min_eq]
  norm_num

/-- `Stc.Bridge.Exact.i128Max` is the generated `IScalar.max` at `I128`. -/
theorem i128Max_eq_IScalar_max : Stc.Bridge.Exact.i128Max = IScalar.max IScalarTy.I128 := by
  unfold Stc.Bridge.Exact.i128Max
  rw [IScalar.max_IScalarTy_I128_eq, I128.max_eq]
  norm_num

/-- A value outside the `i128` range is outside every production layout, so the
exact-side range predicate is false and the exact operation overflows. -/
theorem not_fits_of_not_i128 {ty : exact.IntType} (hw : WellFormed ty) {v : Int}
    (h : ¬ (Stc.Bridge.Exact.i128Min ≤ v ∧ v ≤ Stc.Bridge.Exact.i128Max)) :
    (decodeIntType ty).fits v = false := by
  by_cases hf : (decodeIntType ty).fits v = true
  · rw [Stc.Exact.IntType.fits_iff] at hf
    exact absurd
      ⟨le_of_lt (lt_of_lt_of_le (i128Min_lt_minimum hw) hf.1),
        le_of_lt (lt_of_le_of_lt hf.2 (maximum_lt_i128Max hw))⟩ h
  · simpa using hf

/-- The same fact stated with the generated `IScalar` bounds, which is the form
the `i128` checked-arithmetic step specs produce. -/
theorem not_fits_of_out_of_i128 {ty : exact.IntType} (hw : WellFormed ty) {v : Int}
    (h : ¬ (IScalar.min IScalarTy.I128 ≤ v ∧ v ≤ IScalar.max IScalarTy.I128)) :
    (decodeIntType ty).fits v = false := by
  apply not_fits_of_not_i128 hw
  rwa [i128Min_eq_IScalar_min, i128Max_eq_IScalar_max]

/-- `i128::MIN` negated is above every production maximum, so exact negation of
the shared minimum overflows every production layout. -/
theorem not_fits_neg_i128Min {ty : exact.IntType} (hw : WellFormed ty) :
    (decodeIntType ty).fits (-Stc.Bridge.Exact.i128Min) = false := by
  apply not_fits_of_not_i128 hw
  intro ⟨_, hhi⟩
  have hlt : Stc.Bridge.Exact.i128Max < -Stc.Bridge.Exact.i128Min := by
    unfold Stc.Bridge.Exact.i128Min Stc.Bridge.Exact.i128Max
    norm_num
  omega

/-- The shared `I128.min` used by the division guards is
`Stc.Bridge.Exact.i128Min`. -/
theorem i128Min_eq_I128_min : Stc.Bridge.Exact.i128Min = I128.min := by
  rw [i128Min_eq_IScalar_min, IScalar.min_IScalarTy_I128_eq]

/-- A value that fits a production layout lies strictly above `i128::MIN`, so
it can never trigger the shared `i128` division overflow guard. -/
theorem ne_i128Min_of_fits {ty : exact.IntType} (hw : WellFormed ty) {v : Int}
    (hf : (decodeIntType ty).fits v = true) : v ≠ I128.min := by
  rw [Stc.Exact.IntType.fits_iff] at hf
  have hlt : I128.min < (decodeIntType ty).minimum := by
    rw [← i128Min_eq_I128_min]
    exact i128Min_lt_minimum hw
  omega

/-- The same guard fact stated against the mathematical `i128::MIN` model used
by the external `i128::abs` specification. -/
theorem ne_externals_i128Min_of_fits {ty : exact.IntType} (hw : WellFormed ty) {v : Int}
    (hf : (decodeIntType ty).fits v = true) : v ≠ Stc.Bridge.Exact.i128Min := by
  rw [Stc.Exact.IntType.fits_iff] at hf
  have hlt : Stc.Bridge.Exact.i128Min < (decodeIntType ty).minimum := i128Min_lt_minimum hw
  omega

/-! ## Checked-result construction and checked arithmetic -/

/-- `checked` refines the exact checked-result construction: the range test is
the exact `fits`, so the success branch returns the value and the failure branch
returns `overflow`. -/
@[step] theorem checked_spec (ty : exact.IntType) (value : Std.I128) (hw : WellFormed ty) :
    exact.checked ty value ⦃ g => CoreResultRefines g ((decodeIntType ty).checked value.val) ⦄ := by
  unfold exact.checked
  step* <;> simp_all [CoreResultRefines, Stc.Exact.IntType.checked, decodeTrap]

/-- `add` refines exact checked addition. A shared-`i128` success feeds the
range check; a shared-`i128` overflow is already outside every production
layout, so it refines the exact overflow. -/
@[step] theorem add_spec (ty : exact.IntType) (a b : Std.I128) (hw : WellFormed ty) :
    exact.add ty a b ⦃ g => CoreResultRefines g ((decodeIntType ty).add a.val b.val) ⦄ := by
  unfold exact.add
  cases h : I128.checked_add a b with
  | none =>
      change core.num.checked_add_IScalar a b = none at h
      have hs := I128.checked_add_bv_spec (x := a) (y := b)
      rw [h] at hs
      simp only [lift, core.option.Option.ok_or,
        core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual, bind_ok]
      simp_all [CoreResultRefines, Stc.Exact.IntType.add, Stc.Exact.IntType.checked,
        not_fits_of_out_of_i128 hw, decodeTrap]
  | some z =>
      change core.num.checked_add_IScalar a b = some z at h
      have hs := I128.checked_add_bv_spec (x := a) (y := b)
      rw [h] at hs
      obtain ⟨_, _, hz, _⟩ := hs
      simp only [lift, core.option.Option.ok_or,
        core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual, bind_ok]
      simpa [Stc.Exact.IntType.add, hz] using checked_spec ty z hw

/-- `sub` refines exact checked subtraction, by the same argument as `add`. -/
@[step] theorem sub_spec (ty : exact.IntType) (a b : Std.I128) (hw : WellFormed ty) :
    exact.sub ty a b ⦃ g => CoreResultRefines g ((decodeIntType ty).sub a.val b.val) ⦄ := by
  unfold exact.sub
  cases h : I128.checked_sub a b with
  | none =>
      change core.num.checked_sub_IScalar a b = none at h
      have hs := I128.checked_sub_bv_spec (x := a) (y := b)
      rw [h] at hs
      simp only [lift, core.option.Option.ok_or,
        core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual, bind_ok]
      simp_all [CoreResultRefines, Stc.Exact.IntType.sub, Stc.Exact.IntType.checked,
        not_fits_of_out_of_i128 hw, decodeTrap]
  | some z =>
      change core.num.checked_sub_IScalar a b = some z at h
      have hs := I128.checked_sub_bv_spec (x := a) (y := b)
      rw [h] at hs
      obtain ⟨_, _, hz, _⟩ := hs
      simp only [lift, core.option.Option.ok_or,
        core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual, bind_ok]
      simpa [Stc.Exact.IntType.sub, hz] using checked_spec ty z hw

/-- `mul` refines exact checked multiplication, by the same argument as `add`. -/
@[step] theorem mul_spec (ty : exact.IntType) (a b : Std.I128) (hw : WellFormed ty) :
    exact.mul ty a b ⦃ g => CoreResultRefines g ((decodeIntType ty).mul a.val b.val) ⦄ := by
  unfold exact.mul
  cases h : I128.checked_mul a b with
  | none =>
      change core.num.checked_mul_IScalar a b = none at h
      have hs := I128.checked_mul_bv_spec (x := a) (y := b)
      rw [h] at hs
      simp only [lift, core.option.Option.ok_or,
        core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual, bind_ok]
      simp_all [CoreResultRefines, Stc.Exact.IntType.mul, Stc.Exact.IntType.checked,
        not_fits_of_out_of_i128 hw, decodeTrap]
  | some z =>
      change core.num.checked_mul_IScalar a b = some z at h
      have hs := I128.checked_mul_bv_spec (x := a) (y := b)
      rw [h] at hs
      obtain ⟨_, _, hz, _⟩ := hs
      simp only [lift, core.option.Option.ok_or,
        core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual, bind_ok]
      simpa [Stc.Exact.IntType.mul, hz] using checked_spec ty z hw

/-- `div` refines exact checked division. The dividend fits its layout, so the
shared `i128` division stays away from `i128::MIN / -1`, where Rust would
panic; every generated branch (zero divisor, exact minimum over `-1`, and the
plain quotient) matches the exact model. -/
@[step] theorem div_spec (ty : exact.IntType) (a b : Std.I128) (hw : WellFormed ty)
    (hfits : (decodeIntType ty).fits a.val = true) :
    exact.div ty a b ⦃ g => CoreResultRefines g ((decodeIntType ty).div a.val b.val) ⦄ := by
  unfold exact.div
  have hne : a.val ≠ I128.min := ne_i128Min_of_fits hw hfits
  step*
  all_goals simp_all [CoreResultRefines, decodeTrap, Stc.Exact.IntType.div,
    IScalar.eq_equiv, decodeIntType, IntTypeEncoding.toExact]

/-- `modulo` refines exact checked remainder. The generated `i128` remainder
has no overflow guard, but the model checks the exact minimum over `-1` before
calling it, so every branch matches the exact model. -/
@[step] theorem modulo_spec (ty : exact.IntType) (a b : Std.I128) (hw : WellFormed ty) :
    exact.modulo ty a b ⦃ g => CoreResultRefines g ((decodeIntType ty).mod a.val b.val) ⦄ := by
  unfold exact.modulo
  step*
  all_goals simp_all [CoreResultRefines, decodeTrap, Stc.Exact.IntType.mod,
    IScalar.eq_equiv, decodeIntType, IntTypeEncoding.toExact]

/-- `abs` refines exact absolute value. The operand fits its layout, so the
external `i128::abs` call stays away from `i128::MIN`, where Rust panics; the
signed exact minimum overflows before the call, as in the model. -/
@[step] theorem abs_spec (ty : exact.IntType) (a : Std.I128) (hw : WellFormed ty)
    (hfits : (decodeIntType ty).fits a.val = true) :
    exact.abs ty a ⦃ g => CoreResultRefines g ((decodeIntType ty).abs a.val) ⦄ := by
  unfold exact.abs
  have hne : a.val ≠ Stc.Bridge.Exact.i128Min := ne_externals_i128Min_of_fits hw hfits
  have hab := i128Abs_refines a hne
  cases h : exact.core.num.I128.abs a using Result.cases with
  | ret v =>
      rw [h] at hab
      simp only [ResultRefinesInt, Result.match.ok] at hab
      by_cases hs : ty.signed
      · have hs' : (decodeIntType ty).signed = true := by
          simpa [decodeIntType, IntTypeEncoding.toExact] using hs
        rw [ite_eq_left hs]
        step as ⟨ i, hi ⟩
        by_cases hai : a = i
        · rw [ite_eq_left hai]
          have hamin : a.val = (decodeIntType ty).minimum := by
            rw [hai, ← hi]
          simp [CoreResultRefines, decodeTrap, Stc.Exact.IntType.abs, hs', hamin]
        · rw [ite_eq_right hai]
          have hmin_ne : a.val ≠ (decodeIntType ty).minimum := by
            intro hv
            apply hai
            exact (IScalar.eq_equiv a i).mpr (by rw [hv, hi])
          simp [CoreResultRefines, decodeTrap, Stc.Exact.IntType.abs, hs', hmin_ne,
            Stc.Bridge.Exact.iabs, ← hab]
      · have hs0 : ty.signed = false := by simpa using hs
        have hs0' : (decodeIntType ty).signed = false := by
          simpa [decodeIntType, IntTypeEncoding.toExact] using hs0
        rw [ite_eq_right hs]
        simp [CoreResultRefines, decodeTrap, Stc.Exact.IntType.abs, hs0',
          Stc.Bridge.Exact.iabs, ← hab]
  | vis i k =>
      rw [h] at hab
      simp [ResultRefinesInt] at hab
  | div =>
      rw [h] at hab
      simp [ResultRefinesInt] at hab

/-- `neg` refines exact checked negation. The external `i128::checked_neg` call
is a TCB assumption; it fails exactly at `i128::MIN`, which lies below every
production minimum and so is an exact overflow. -/
@[step] theorem neg_spec (ty : exact.IntType) (a : Std.I128) (hw : WellFormed ty) :
    exact.neg ty a ⦃ g => CoreResultRefines g ((decodeIntType ty).neg a.val) ⦄ := by
  unfold exact.neg
  have hcn := checkedNeg_refines a
  cases h : exact.core.num.I128.checked_neg a using Result.cases with
  | ret o =>
      rw [h] at hcn
      simp only [ResultRefinesOptionInt] at hcn
      cases o with
      | none =>
          simp only [core.option.Option.ok_or,
            core.result.Result.Insts.CoreOpsTry.branch,
            core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual, bind_ok]
          simp_all [CoreResultRefines, Stc.Exact.IntType.neg, Stc.Exact.IntType.checked,
            Stc.Bridge.Exact.checkedNeg_none_iff, not_fits_neg_i128Min hw, decodeTrap]
      | some z =>
          simp only [core.option.Option.ok_or,
            core.result.Result.Insts.CoreOpsTry.branch,
            core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual, bind_ok]
          have hz : z.val = -a.val := (Stc.Bridge.Exact.checkedNeg_some_iff.mp hcn).2
          simpa [Stc.Exact.IntType.neg, hz] using checked_spec ty z hw
  | vis i k =>
      rw [h] at hcn
      simp [ResultRefinesOptionInt] at hcn
  | div =>
      rw [h] at hcn
      simp [ResultRefinesOptionInt] at hcn

/-! ## Bitwise, shift, and rotate operations

The generated helpers operate on the shared `i128`/`u128` kernel: `modulus` is
narrowed to `u128` with a checked cast, operands are reduced modulo it, and the
bitwise or shift operation runs on `u128`. On the `WellFormed` invariant the
modulus stays below `2 ^ 128`, so the cast returns it unchanged; the shift
results stay below `2 ^ 128` (their inputs are below `2 ^ 64` and the counts
below `64`), so the generated `u128` wrap is the identity. That is exactly the
exact-side model in `Stc.Exact.Bits`. -/

/-- The modulus of any production layout is below `2 ^ 128`. -/
theorem modulus_lt_two_pow_128 {ty : exact.IntType} (hw : WellFormed ty) :
    (decodeIntType ty).modulus < 2 ^ (128 : Nat) := by
  have h : (decodeIntType ty).modulus = 2 ^ ty.width.val := rfl
  rw [h]
  have hle : 2 ^ ty.width.val ≤ 2 ^ 64 := Nat.pow_le_pow_right (by norm_num) hw.2
  have hlt : (2 : Nat) ^ 64 < 2 ^ 128 := by norm_num
  omega

/-- A checked cast of the modulus to `u128` is the identity, because the
modulus is below `2 ^ 128`. -/
theorem hcast_modulus_val {ty : exact.IntType} (hw : WellFormed ty) {i : Std.I128}
    (hi : (((decodeIntType ty).modulus : Nat) : Int) = i.val) :
    (IScalar.hcast UScalarTy.U128 i).val = (decodeIntType ty).modulus := by
  rw [IScalar.hcast_val_eq, ← hi]
  rw [show UScalarTy.U128.numBits = 128 from rfl]
  have h0 : (0 : Int) ≤ ((decodeIntType ty).modulus : Int) := Int.natCast_nonneg _
  have hlt : ((decodeIntType ty).modulus : Int) < (2 : Int) ^ 128 := by
    exact_mod_cast modulus_lt_two_pow_128 hw
  rw [Int.emod_eq_of_lt h0 hlt, Int.toNat_natCast]

/-- `mask` refines the exact bit mask. -/
@[step] theorem mask_spec (ty : exact.IntType) (hw : WellFormed ty) :
    exact.IntType.mask ty ⦃ v => ((decodeIntType ty).mask : Nat) = v.val ⦄ := by
  unfold exact.IntType.mask
  step*
  · rw [i_post]
    have hlt : (2 : Nat) ^ ty.width.val < 2 ^ 128 := by
      have hle : 2 ^ ty.width.val ≤ 2 ^ 64 := Nat.pow_le_pow_right (by norm_num) hw.2
      have hlt64 : (2 : Nat) ^ 64 < 2 ^ 128 := by norm_num
      omega
    have hval : (1 : Nat) <<< ty.width.val % U128.size = 2 ^ ty.width.val := by
      rw [Nat.shiftLeft_eq, one_mul,
        show U128.size = 2 ^ 128 by simp [U128.size, U128.numBits]]
      exact Nat.mod_eq_of_lt hlt
    rw [hval]
    simp
  · rw [v_post, i_post]
    have hlt : (2 : Nat) ^ ty.width.val < 2 ^ 128 := by
      have hle : 2 ^ ty.width.val ≤ 2 ^ 64 := Nat.pow_le_pow_right (by norm_num) hw.2
      have hlt64 : (2 : Nat) ^ 64 < 2 ^ 128 := by norm_num
      omega
    have hval : (1 : Nat) <<< ty.width.val % U128.size = 2 ^ ty.width.val := by
      rw [Nat.shiftLeft_eq, one_mul,
        show U128.size = 2 ^ 128 by simp [U128.size, U128.numBits]]
      exact Nat.mod_eq_of_lt hlt
    rw [hval]
    simp [decodeIntType, IntTypeEncoding.toExact, Stc.Exact.IntType.mask]

/-- `count` refines the exact masked shift count. The generated code converts
the width to `i64` (TCB assumption), takes `rem_euclid`, and narrows to `u32`;
every step is exact on the `WellFormed` invariant. -/
@[step] theorem count_spec (ty : exact.IntType) (n : Std.I64) (hw : WellFormed ty) :
    exact.IntType.count ty n ⦃ v => ((decodeIntType ty).count n.val) = v.val ⦄ := by
  unfold exact.IntType.count
  have hf := fromU8_refines ty.width
  cases hfw : exact.I64.Insts.CoreConvertFromU8.from ty.width using Result.cases with
  | ret w =>
      rw [hfw] at hf
      simp only [ResultRefinesI64, Result.match.ok] at hf
      have hw_val : w.val = (ty.width.val : Int) := by
        rw [← hf]
        simp [Stc.Bridge.Exact.fromU8]
      have hnz : w.val ≠ 0 := by
        rw [hw_val]
        have : (0 : Int) < (ty.width.val : Int) := by exact_mod_cast hw.1
        omega
      have hnov : ¬ (n.val = Stc.Bridge.Exact.i64Min ∧ w.val = -1) := by
        intro ⟨_, hneg⟩
        rw [hw_val] at hneg
        have h0 : (0 : Int) ≤ (ty.width.val : Int) := by exact_mod_cast Nat.zero_le _
        omega
      have hre := remEuclid_refines n w hnz hnov
      simp only [bind_ok]
      cases hrem : exact.core.num.I64.rem_euclid n w using Result.cases with
      | ret r =>
          rw [hrem] at hre
          simp only [ResultRefinesI64, Result.match.ok] at hre
          have hr_val : r.val = n.val.emod (ty.width.val : Int) := by
            rw [← hre]
            simp [Stc.Bridge.Exact.remEuclid, hw_val]
          have h0 : 0 ≤ n.val.emod (ty.width.val : Int) :=
            Int.emod_nonneg _ (by exact_mod_cast hw.1.ne')
          have hlt : n.val.emod (ty.width.val : Int) < 2 ^ 32 := by
            have hltw : n.val.emod (ty.width.val : Int) < (ty.width.val : Int) :=
              Int.emod_lt_of_pos _ (by exact_mod_cast hw.1)
            have h64 : (ty.width.val : Int) ≤ 64 := by exact_mod_cast hw.2
            have h32 : (2 : Int) ^ 32 = 4294967296 := by norm_num
            omega
          simp only [bind_ok, Aeneas.Std.WP.spec_ok]
          rw [IScalar.hcast_val_eq, show UScalarTy.U32.numBits = 32 from rfl, hr_val,
            Int.emod_eq_of_lt h0 hlt]
          simp [Stc.Exact.IntType.count, decodeIntType, IntTypeEncoding.toExact]
      | vis i k =>
          rw [hrem] at hre
          simp [ResultRefinesI64] at hre
      | div =>
          rw [hrem] at hre
          simp [ResultRefinesI64] at hre
  | vis i k =>
      rw [hfw] at hf
      simp [ResultRefinesI64] at hf
  | div =>
      rw [hfw] at hf
      simp [ResultRefinesI64] at hf

/-- `bit_and` refines the exact masked bitwise and. -/
@[step] theorem bit_and_spec (ty : exact.IntType) (a b : Std.U128) (hw : WellFormed ty) :
    exact.bit_and ty a b ⦃ v => ((decodeIntType ty).bitAnd a.val b.val) = v.val ⦄ := by
  unfold exact.bit_and
  step*
  · have hv := hcast_modulus_val hw i_post
    intro hzero
    rw [i1_post] at hzero
    rw [hv] at hzero
    have hpos : 0 < (decodeIntType ty).modulus := by
      show 0 < 2 ^ ty.width.val
      exact Nat.two_pow_pos _
    omega
  · have hv := hcast_modulus_val hw i_post
    intro hzero
    rw [i3_post] at hzero
    rw [hv] at hzero
    have hpos : 0 < (decodeIntType ty).modulus := by
      show 0 < 2 ^ ty.width.val
      exact Nat.two_pow_pos _
    omega
  · have hv := hcast_modulus_val hw i_post
    simp [Stc.Exact.IntType.bitAnd, UScalar.val_and, i1_post, i3_post, i2_post, i4_post, hv]

/-- `bit_or` refines the exact masked bitwise or. -/
@[step] theorem bit_or_spec (ty : exact.IntType) (a b : Std.U128) (hw : WellFormed ty) :
    exact.bit_or ty a b ⦃ v => ((decodeIntType ty).bitOr a.val b.val) = v.val ⦄ := by
  unfold exact.bit_or
  step*
  · have hv := hcast_modulus_val hw i_post
    intro hzero
    rw [i1_post] at hzero
    rw [hv] at hzero
    have hpos : 0 < (decodeIntType ty).modulus := by
      show 0 < 2 ^ ty.width.val
      exact Nat.two_pow_pos _
    omega
  · have hv := hcast_modulus_val hw i_post
    intro hzero
    rw [i3_post] at hzero
    rw [hv] at hzero
    have hpos : 0 < (decodeIntType ty).modulus := by
      show 0 < 2 ^ ty.width.val
      exact Nat.two_pow_pos _
    omega
  · have hv := hcast_modulus_val hw i_post
    simp [Stc.Exact.IntType.bitOr, UScalar.val_or, i1_post, i3_post, i2_post, i4_post, hv]

/-- `bit_xor` refines the exact masked bitwise xor. -/
@[step] theorem bit_xor_spec (ty : exact.IntType) (a b : Std.U128) (hw : WellFormed ty) :
    exact.bit_xor ty a b ⦃ v => ((decodeIntType ty).bitXor a.val b.val) = v.val ⦄ := by
  unfold exact.bit_xor
  step*
  · have hv := hcast_modulus_val hw i_post
    intro hzero
    rw [i1_post] at hzero
    rw [hv] at hzero
    have hpos : 0 < (decodeIntType ty).modulus := by
      show 0 < 2 ^ ty.width.val
      exact Nat.two_pow_pos _
    omega
  · have hv := hcast_modulus_val hw i_post
    intro hzero
    rw [i3_post] at hzero
    rw [hv] at hzero
    have hpos : 0 < (decodeIntType ty).modulus := by
      show 0 < 2 ^ ty.width.val
      exact Nat.two_pow_pos _
    omega
  · have hv := hcast_modulus_val hw i_post
    simp [Stc.Exact.IntType.bitXor, UScalar.val_xor, i1_post, i3_post, i2_post, i4_post, hv]

/-- `bit_not` refines the exact masked bitwise complement. -/
@[step] theorem bit_not_spec (ty : exact.IntType) (a : Std.U128) (hw : WellFormed ty) :
    exact.bit_not ty a ⦃ v => ((decodeIntType ty).bitNot a.val) = v.val ⦄ := by
  unfold exact.bit_not
  step*
  · have hv := hcast_modulus_val hw i_post
    intro hzero
    rw [i1_post] at hzero
    rw [hv] at hzero
    have hpos : 0 < (decodeIntType ty).modulus := by
      show 0 < 2 ^ ty.width.val
      exact Nat.two_pow_pos _
    omega
  · have hv := hcast_modulus_val hw i_post
    simp [Stc.Exact.IntType.bitNot, UScalar.val_xor, i1_post, i2_post, i3_post, hv]

/-- `shl` refines the exact masked left shift. -/
@[step] theorem shl_spec (ty : exact.IntType) (a : Std.U128) (n : Std.I64) (hw : WellFormed ty) :
    exact.shl ty a n ⦃ v => ((decodeIntType ty).shl a.val n.val) = v.val ⦄ := by
  unfold exact.shl
  step*
  · have hv := hcast_modulus_val hw i_post
    intro hzero
    rw [i1_post] at hzero
    rw [hv] at hzero
    have hpos : 0 < (decodeIntType ty).modulus := by
      show 0 < 2 ^ ty.width.val
      exact Nat.two_pow_pos _
    omega
  · rw [← i3_post]
    have hlt := Stc.Exact.IntType.count_lt (decodeIntType ty) n.val hw.1
    have hwidth : (decodeIntType ty).width = ty.width.val := rfl
    rw [hwidth] at hlt
    have h64 : ty.width.val ≤ 64 := hw.2
    omega
  · have hv := hcast_modulus_val hw i_post
    intro hzero
    rw [i5_post] at hzero
    rw [hv] at hzero
    have hpos : 0 < (decodeIntType ty).modulus := by
      show 0 < 2 ^ ty.width.val
      exact Nat.two_pow_pos _
    omega
  · have hv := hcast_modulus_val hw i_post
    rw [v_post, i4_post, i5_post, hv, i2_post, i1_post, hv, ← i3_post]
    have hmod : (decodeIntType ty).modulus = 2 ^ ty.width.val := rfl
    have hsize : U128.size = 2 ^ 128 := by simp [U128.size, U128.numBits]
    have h128 : ty.width.val ≤ 128 := by
      have := hw.2
      omega
    unfold Stc.Exact.IntType.shl
    rw [hmod, hsize]
    rw [Nat.mod_mod_of_dvd _ (Nat.pow_dvd_pow 2 h128)]

/-- `shr` refines the exact masked right shift. -/
@[step] theorem shr_spec (ty : exact.IntType) (a : Std.U128) (n : Std.I64) (hw : WellFormed ty) :
    exact.shr ty a n ⦃ v => ((decodeIntType ty).shr a.val n.val) = v.val ⦄ := by
  unfold exact.shr
  step*
  · have hv := hcast_modulus_val hw i_post
    intro hzero
    rw [i1_post] at hzero
    rw [hv] at hzero
    have hpos : 0 < (decodeIntType ty).modulus := by
      show 0 < 2 ^ ty.width.val
      exact Nat.two_pow_pos _
    omega
  · rw [← i3_post]
    have hlt := Stc.Exact.IntType.count_lt (decodeIntType ty) n.val hw.1
    have hwidth : (decodeIntType ty).width = ty.width.val := rfl
    rw [hwidth] at hlt
    have h64 : ty.width.val ≤ 64 := hw.2
    omega
  · have hv := hcast_modulus_val hw i_post
    rw [v_post, i2_post, i1_post, hv, ← i3_post]
    unfold Stc.Exact.IntType.shr
    rfl

/-- Shifting a 64-bit value by a count below 64 stays inside `u128`. -/
theorem shl_lt_u128_size {x c : Nat} (hx : x < 2 ^ 64) (hc : c < 64) :
    x <<< c < U128.size := by
  rw [Nat.shiftLeft_eq, show U128.size = 2 ^ 128 by simp [U128.size, U128.numBits]]
  have h2 : 2 ^ c < 2 ^ 64 := Nat.pow_lt_pow_right (by norm_num) hc
  have h := Nat.mul_lt_mul_of_lt_of_lt hx h2
  have h64 : (2 : Nat) ^ 64 * 2 ^ 64 = 2 ^ 128 := by
    rw [← Nat.pow_add]
  omega

/-- Step spec for the Aeneas `u8 → u32` conversion used inside the rotate
helpers. It is the standard widening embedding and returns `ok`. -/
@[step] theorem fromU32U8_spec (x : Std.U8) :
    lift (core.convert.num.FromU32U8.from x) ⦃ w => w.val = x.val ⦄ := by
  unfold lift
  simp

/-- `rol` refines the exact masked left rotate. Both rotated halves stay below
`2 ^ 64`, so neither `u128` shift wraps. -/
@[step] theorem rol_spec (ty : exact.IntType) (a : Std.U128) (n : Std.I64) (hw : WellFormed ty) :
    exact.rol ty a n ⦃ v => ((decodeIntType ty).rol a.val n.val) = v.val ⦄ := by
  unfold exact.rol
  step*
  · have hv := hcast_modulus_val hw i_post
    intro hzero
    rw [i1_post] at hzero
    rw [hv] at hzero
    have hpos : 0 < (decodeIntType ty).modulus := by
      show 0 < 2 ^ ty.width.val
      exact Nat.two_pow_pos _
    omega
  · rw [← r_post]
    have hlt := Stc.Exact.IntType.count_lt (decodeIntType ty) n.val hw.1
    have hwidth : (decodeIntType ty).width = ty.width.val := rfl
    rw [hwidth] at hlt
    have h64 : ty.width.val ≤ 64 := hw.2
    omega
  · rw [← r_post, width_post]
    have hlt := Stc.Exact.IntType.count_lt (decodeIntType ty) n.val hw.1
    have hwidth : (decodeIntType ty).width = ty.width.val := rfl
    rw [hwidth] at hlt
    omega
  · have hv7 := hcast_modulus_val hw i7_post
    intro hzero
    rw [i8_post] at hzero
    rw [hv7] at hzero
    have hpos : 0 < (decodeIntType ty).modulus := by
      show 0 < 2 ^ ty.width.val
      exact Nat.two_pow_pos _
    omega
  · have hv := hcast_modulus_val hw i_post
    have hv7 := hcast_modulus_val hw i7_post
    have hM : (decodeIntType ty).modulus = 2 ^ ty.width.val := rfl
    have hwidth : (decodeIntType ty).width = ty.width.val := rfl
    have h64 : ty.width.val ≤ 64 := hw.2
    have hnowrap :
        (↑a % (decodeIntType ty).modulus) <<< (decodeIntType ty).count ↑n < U128.size := by
      apply shl_lt_u128_size
      · have haM : ↑a % (decodeIntType ty).modulus < (decodeIntType ty).modulus :=
          Nat.mod_lt _ (decodeIntType ty).modulus_pos
        rw [hM] at haM ⊢
        have hle : 2 ^ ty.width.val ≤ 2 ^ 64 := Nat.pow_le_pow_right (by norm_num) h64
        omega
      · have hlt := Stc.Exact.IntType.count_lt (decodeIntType ty) n.val hw.1
        rw [hwidth] at hlt
        omega
    rw [v_post, i8_post, hv7, i6_post, UScalar.val_or, i2_post, i5_post, i4_post, i3_post,
      width_post, x_post, i1_post, hv, ← r_post, Nat.mod_eq_of_lt hnowrap]
    unfold Stc.Exact.IntType.rol
    rfl

/-- `ror` refines the exact masked right rotate. -/
@[step] theorem ror_spec (ty : exact.IntType) (a : Std.U128) (n : Std.I64) (hw : WellFormed ty) :
    exact.ror ty a n ⦃ v => ((decodeIntType ty).ror a.val n.val) = v.val ⦄ := by
  unfold exact.ror
  step*
  · have hv := hcast_modulus_val hw i_post
    intro hzero
    rw [i1_post] at hzero
    rw [hv] at hzero
    have hpos : 0 < (decodeIntType ty).modulus := by
      show 0 < 2 ^ ty.width.val
      exact Nat.two_pow_pos _
    omega
  · rw [← r_post]
    have hlt := Stc.Exact.IntType.count_lt (decodeIntType ty) n.val hw.1
    have hwidth : (decodeIntType ty).width = ty.width.val := rfl
    rw [hwidth] at hlt
    have h64 : ty.width.val ≤ 64 := hw.2
    omega
  · rw [← r_post, width_post]
    have hlt := Stc.Exact.IntType.count_lt (decodeIntType ty) n.val hw.1
    have hwidth : (decodeIntType ty).width = ty.width.val := rfl
    rw [hwidth] at hlt
    omega
  · have hv7 := hcast_modulus_val hw i7_post
    intro hzero
    rw [i8_post] at hzero
    rw [hv7] at hzero
    have hpos : 0 < (decodeIntType ty).modulus := by
      show 0 < 2 ^ ty.width.val
      exact Nat.two_pow_pos _
    omega
  · have hv := hcast_modulus_val hw i_post
    have hv7 := hcast_modulus_val hw i7_post
    have hM : (decodeIntType ty).modulus = 2 ^ ty.width.val := rfl
    have hwidth : (decodeIntType ty).width = ty.width.val := rfl
    have h64 : ty.width.val ≤ 64 := hw.2
    have hnowrap :
        (↑a % (decodeIntType ty).modulus) <<<
          ((↑ty.width - (decodeIntType ty).count ↑n) % ↑ty.width) < U128.size := by
      apply shl_lt_u128_size
      · have haM : ↑a % (decodeIntType ty).modulus < (decodeIntType ty).modulus :=
          Nat.mod_lt _ (decodeIntType ty).modulus_pos
        rw [hM] at haM ⊢
        have hle : 2 ^ ty.width.val ≤ 2 ^ 64 := Nat.pow_le_pow_right (by norm_num) h64
        omega
      · have hlt := Nat.mod_lt (↑ty.width - (decodeIntType ty).count ↑n) hw.1
        omega
    rw [v_post, i8_post, hv7, i6_post, UScalar.val_or, i5_post, i2_post, i4_post, i3_post,
      width_post, x_post, i1_post, hv, ← r_post, Nat.mod_eq_of_lt hnowrap]
    unfold Stc.Exact.IntType.ror
    rfl

/-- `modulus_refines`. -/
theorem modulus_refines : modulusRefines := by
  intro ty hw
  exact ResultRefinesInt.of_spec (modulus_spec ty hw)

/-- `minimum_refines`. -/
theorem minimum_refines : minimumRefines := by
  intro ty hw
  exact ResultRefinesInt.of_spec (minimum_spec ty hw)

/-- `maximum_refines`. -/
theorem maximum_refines : maximumRefines := by
  intro ty hw
  exact ResultRefinesInt.of_spec (maximum_spec ty hw)

/-- `fits_refines`. -/
theorem fits_refines : fitsRefines := by
  intro ty value hw
  exact ResultRefinesBool.of_spec (fits_spec ty value hw)

/-- `checked_refines`. -/
theorem checked_refines : checkedRefines := by
  intro ty value hw
  exact ResultRefinesExact.of_spec (checked_spec ty value hw)

/-- `add_refines`. -/
theorem add_refines : addRefines := by
  intro ty a b hw
  exact ResultRefinesExact.of_spec (add_spec ty a b hw)

/-- `sub_refines`. -/
theorem sub_refines : subRefines := by
  intro ty a b hw
  exact ResultRefinesExact.of_spec (sub_spec ty a b hw)

/-- `mul_refines`. -/
theorem mul_refines : mulRefines := by
  intro ty a b hw
  exact ResultRefinesExact.of_spec (mul_spec ty a b hw)

/-- `neg_refines`. -/
theorem neg_refines : negRefines := by
  intro ty a hw
  exact ResultRefinesExact.of_spec (neg_spec ty a hw)

/-- `div_refines`. -/
theorem div_refines : divRefines := by
  intro ty a b hw hfits
  exact ResultRefinesExact.of_spec (div_spec ty a b hw hfits)

/-- `modulo_refines`. -/
theorem modulo_refines : moduloRefines := by
  intro ty a b hw
  exact ResultRefinesExact.of_spec (modulo_spec ty a b hw)

/-- `abs_refines`. -/
theorem abs_refines : absRefines := by
  intro ty a hw hfits
  exact ResultRefinesExact.of_spec (abs_spec ty a hw hfits)

/-- `mask_refines`. -/
theorem mask_refines : maskRefines := by
  intro ty hw
  exact ResultRefinesNat.of_spec (mask_spec ty hw)

/-- `count_refines`. -/
theorem count_refines : countRefines := by
  intro ty n hw
  exact ResultRefinesU32.of_spec (count_spec ty n hw)

/-- `bitAnd_refines`. -/
theorem bitAnd_refines : bitAndRefines := by
  intro ty a b hw
  exact ResultRefinesNat.of_spec (bit_and_spec ty a b hw)

/-- `bitOr_refines`. -/
theorem bitOr_refines : bitOrRefines := by
  intro ty a b hw
  exact ResultRefinesNat.of_spec (bit_or_spec ty a b hw)

/-- `bitXor_refines`. -/
theorem bitXor_refines : bitXorRefines := by
  intro ty a b hw
  exact ResultRefinesNat.of_spec (bit_xor_spec ty a b hw)

/-- `bitNot_refines`. -/
theorem bitNot_refines : bitNotRefines := by
  intro ty a hw
  exact ResultRefinesNat.of_spec (bit_not_spec ty a hw)

/-- `shl_refines`. -/
theorem shl_refines : shlRefines := by
  intro ty a n hw
  exact ResultRefinesNat.of_spec (shl_spec ty a n hw)

/-- `shr_refines`. -/
theorem shr_refines : shrRefines := by
  intro ty a n hw
  exact ResultRefinesNat.of_spec (shr_spec ty a n hw)

/-- `rol_refines`. -/
theorem rol_refines : rolRefines := by
  intro ty a n hw
  exact ResultRefinesNat.of_spec (rol_spec ty a n hw)

/-- `ror_refines`. -/
theorem ror_refines : rorRefines := by
  intro ty a n hw
  exact ResultRefinesNat.of_spec (ror_spec ty a n hw)

end Stc.Bridge.Exact
