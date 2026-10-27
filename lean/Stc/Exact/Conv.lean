/-
Exact conversion model between fixed-width integer layouts.
-/
import Stc.Exact.Int

namespace Stc.Exact

/-- Two raised to a smaller exponent is no larger. -/
theorem two_pow_le_two_pow {a b : Nat} (h : a ≤ b) : (2 : Int) ^ a ≤ (2 : Int) ^ b := by
  have hn : 2 ^ a ≤ 2 ^ b := Nat.pow_le_pow_right (by omega) h
  exact_mod_cast hn

/-- Converts `v` from `src` to `to`.
Widening sign- or zero-extends the source bit pattern, equal widths
reinterpret the bit pattern, and narrowing reports `range` when the value is
not representable. This mirrors code generation (`int_convert`) and is stated
over the source value; callers supply values that fit the source type. -/
def IntType.convert (src to : IntType) (v : Int) : Res :=
  if to.width == 1 then
    -- `BOOL` is the only one-bit layout: the generator tests `v != 0`.
    .ok (if v == 0 then 0 else 1)
  else if src.width < to.width then
    if src.signed && v < 0 then
      if to.signed then .ok v else .ok (v + ((2 : Int) ^ to.width))
    else
      .ok v
  else if src.width > to.width then
    if to.fits v then .ok v else .error .range
  else if src.signed == to.signed then
    .ok v
  else if to.signed then
    .ok (if v ≥ (2 : Int) ^ (to.width - 1) then v - ((2 : Int) ^ to.width) else v)
  else
    .ok (if v < 0 then v + ((2 : Int) ^ to.width) else v)

/-- Conversion to `BOOL` tests the value against zero (code generation's
`Int -> BOOL` rule). -/
theorem IntType.convert_to_bool (src : IntType) (v : Int) :
    src.convert ⟨1, false⟩ v = .ok (if v == 0 then 0 else 1) := by
  unfold IntType.convert
  rw [ite_eq_left (by decide)]

/-- A narrowing conversion fails exactly when the value does not fit. -/
theorem IntType.convert_narrow_error {src to : IntType} {v : Int}
    (hbool : to.width ≠ 1) (h : to.width < src.width) :
    src.convert to v = .error .range ↔ to.fits v = false := by
  unfold IntType.convert
  rw [ite_eq_right (by simp [show to.width ≠ 1 by omega])]
  rw [ite_eq_right (by omega), ite_eq_left h]
  by_cases hf : to.fits v <;> simp [hf]

/-- A narrowing conversion of a representable value returns it unchanged. -/
theorem IntType.convert_narrow_ok {src to : IntType} {v : Int}
    (hbool : to.width ≠ 1) (h : to.width < src.width) (hf : to.fits v = true) :
    src.convert to v = .ok v := by
  unfold IntType.convert
  rw [ite_eq_right (by simp [show to.width ≠ 1 by omega])]
  rw [ite_eq_right (by omega), ite_eq_left h, ite_eq_left hf]

/-- Widening a non-negative value or an unsigned source preserves the value. -/
theorem IntType.convert_widen_value {src to : IntType} {v : Int}
    (hbool : to.width ≠ 1) (h : src.width < to.width)
    (hv : src.signed = false ∨ 0 ≤ v) :
    src.convert to v = .ok v := by
  unfold IntType.convert
  rw [ite_eq_right (by simp [show to.width ≠ 1 by omega])]
  rw [ite_eq_left h]
  rcases hv with hu | hv
  · rw [ite_eq_right (by simp [hu])]
  · rw [ite_eq_right (by simp [show ¬ v < 0 by omega])]

/-- Widening a negative signed value to a signed target preserves the value. -/
theorem IntType.convert_widen_signed {src to : IntType} {v : Int}
    (hbool : to.width ≠ 1) (h : src.width < to.width)
    (hs : src.signed = true) (ht : to.signed = true) (hv : v < 0) : src.convert to v = .ok v := by
  unfold IntType.convert
  rw [ite_eq_right (by simp [show to.width ≠ 1 by omega])]
  rw [ite_eq_left h]
  rw [ite_eq_left (by simp [hs, hv])]
  rw [ite_eq_left (by simp [ht])]

/-- Widening a negative signed value to an unsigned target adds the target
modulus, preserving the source bit pattern. -/
theorem IntType.convert_widen_neg_unsigned {src to : IntType} {v : Int}
    (hbool : to.width ≠ 1) (h : src.width < to.width)
    (hs : src.signed = true) (ht : to.signed = false) (hv : v < 0) : src.convert to v = .ok (v + ((2 : Int) ^ to.width)) := by
  unfold IntType.convert
  rw [ite_eq_right (by simp [show to.width ≠ 1 by omega])]
  rw [ite_eq_left h]
  rw [ite_eq_left (by simp [hs, hv])]
  rw [ite_eq_right (by simp [ht])]

/-- Converting between layouts of equal width and equal signedness returns the
value unchanged. -/
theorem IntType.convert_same_signedness {src to : IntType} {v : Int}
    (hbool : to.width ≠ 1) (hw : src.width = to.width)
    (hs : src.signed = to.signed) :
    src.convert to v = .ok v := by
  unfold IntType.convert
  rw [ite_eq_right (by simp [show to.width ≠ 1 by omega])]
  rw [ite_eq_right (by omega), ite_eq_right (by omega)]
  rw [ite_eq_left (by simpa using hs)]

/-- Same-width signed to unsigned keeps the two's-complement bit pattern: a
negative value gains the target modulus. -/
theorem IntType.convert_signed_to_unsigned {src to : IntType} {v : Int}
    (hbool : to.width ≠ 1) (hw : src.width = to.width)
    (hs : src.signed = true) (ht : to.signed = false) (hv : v < 0) : src.convert to v = .ok (v + ((2 : Int) ^ to.width)) := by
  unfold IntType.convert
  rw [ite_eq_right (by simp [show to.width ≠ 1 by omega])]
  rw [ite_eq_right (by omega), ite_eq_right (by omega)]
  rw [ite_eq_right (by simp [hs, ht])]
  rw [ite_eq_right (by simp [ht])]
  rw [ite_eq_left (by omega)]

/-- Same-width unsigned to signed reinterprets a high bit pattern as negative. -/
theorem IntType.convert_unsigned_to_signed_high {src to : IntType} {v : Int}
    (hbool : to.width ≠ 1) (hw : src.width = to.width)
    (hs : src.signed = false) (ht : to.signed = true)
    (hv : (2 : Int) ^ (to.width - 1) ≤ v) :
    src.convert to v = .ok (v - ((2 : Int) ^ to.width)) := by
  unfold IntType.convert
  rw [ite_eq_right (by simp [show to.width ≠ 1 by omega])]
  rw [ite_eq_right (by omega), ite_eq_right (by omega)]
  rw [ite_eq_right (by simp [hs, ht])]
  rw [ite_eq_left (by simp [ht])]
  rw [ite_eq_left (by omega)]

/-- Same-width unsigned to signed keeps a low bit pattern. -/
theorem IntType.convert_unsigned_to_signed_low {src to : IntType} {v : Int}
    (hbool : to.width ≠ 1) (hw : src.width = to.width)
    (hs : src.signed = false) (ht : to.signed = true)
    (hv : v < (2 : Int) ^ (to.width - 1)) :
    src.convert to v = .ok v := by
  unfold IntType.convert
  rw [ite_eq_right (by simp [show to.width ≠ 1 by omega])]
  rw [ite_eq_right (by omega), ite_eq_right (by omega)]
  rw [ite_eq_right (by simp [hs, ht])]
  rw [ite_eq_left (by simp [ht])]
  rw [ite_eq_right (by omega)]

/-- Two to a natural exponent is positive as an integer. -/
theorem two_pow_pos_int (n : Nat) : (0 : Int) < (2 : Int) ^ n := by
  have h := Nat.two_pow_pos n
  exact_mod_cast h

/-- Two to a natural exponent is at least one. -/
theorem two_pow_one_le (n : Nat) : (1 : Int) ≤ (2 : Int) ^ n := by
  have h := Nat.one_le_two_pow (n := n)
  exact_mod_cast h

/-- Fits gives an upper bound below the modulus. -/
theorem IntType.fits_lt_modulus {ty : IntType} {v : Int} (hw : 0 < ty.width)
    (h : ty.fits v = true) : v < (2 : Int) ^ ty.width := by
  rw [IntType.fits_iff] at h
  rcases h with ⟨_, hhi⟩
  rw [IntType.maximum] at hhi
  by_cases hs : ty.signed
  · rw [ite_eq_left hs] at hhi
    have hp : (2 : Int) ^ (ty.width - 1) ≤ (2 : Int) ^ ty.width :=
      two_pow_le_two_pow (by omega)
    omega
  · rw [ite_eq_right hs] at hhi
    omega

/-- Fits gives a lower bound no smaller than minus half the modulus. -/
theorem IntType.fits_ge_neg_half {ty : IntType} {v : Int}
    (h : ty.fits v = true) : -((2 : Int) ^ (ty.width - 1)) ≤ v := by
  rw [IntType.fits_iff] at h
  have hlo := h.1
  rw [IntType.minimum] at hlo
  have hp := two_pow_pos_int (ty.width - 1)
  by_cases hs : ty.signed
  · rw [ite_eq_left hs] at hlo
    exact hlo
  · rw [ite_eq_right hs] at hlo
    omega

/-- A value-preserving widening conversion fits the target. -/
theorem IntType.convert_widen_value_fits {src to : IntType} {v : Int}
    (hw : 0 < src.width) (h : src.width < to.width)
    (hsrc : src.fits v = true) (hv : src.signed = false ∨ 0 ≤ v) :
    to.fits v = true := by
  have hlt := IntType.fits_lt_modulus hw hsrc
  have hge := IntType.fits_ge_neg_half hsrc
  have hvnonneg : (0 : Int) ≤ v := by
    rcases hv with hu | hv
    · have h0 := (IntType.fits_iff.mp hsrc).1
      rw [IntType.minimum, hu] at h0
      exact h0
    · exact hv
  by_cases ht : to.signed
  · have hp : (2 : Int) ^ src.width ≤ (2 : Int) ^ (to.width - 1) :=
      two_pow_le_two_pow (by omega)
    have hp2 : (2 : Int) ^ (src.width - 1) ≤ (2 : Int) ^ (to.width - 1) :=
      two_pow_le_two_pow (by omega)
    refine IntType.fits_iff.mpr ⟨?_, ?_⟩
    · simp [IntType.minimum, ht]
      omega
    · simp [IntType.maximum, ht]
      omega
  · have hp : (2 : Int) ^ src.width ≤ (2 : Int) ^ to.width :=
      two_pow_le_two_pow (by omega)
    refine IntType.fits_iff.mpr ⟨?_, ?_⟩
    · simp [IntType.minimum, ht]
      omega
    · simp [IntType.maximum, ht]
      omega

/-- A widening sign-preserving conversion to a signed target fits. -/
theorem IntType.convert_widen_signed_fits {src to : IntType} {v : Int}
    (hw : 0 < src.width) (h : src.width < to.width) (ht : to.signed = true)
    (hsrc : src.fits v = true) : to.fits v = true := by
  have hlt := IntType.fits_lt_modulus hw hsrc
  have hge := IntType.fits_ge_neg_half hsrc
  have hp_hi : (2 : Int) ^ src.width ≤ (2 : Int) ^ (to.width - 1) :=
    two_pow_le_two_pow (by omega)
  have hp_lo : (2 : Int) ^ (src.width - 1) ≤ (2 : Int) ^ (to.width - 1) :=
    two_pow_le_two_pow (by omega)
  refine IntType.fits_iff.mpr ⟨?_, ?_⟩
  · simp [IntType.minimum, ht]
    omega
  · simp [IntType.maximum, ht]
    omega

/-- A widening bit-pattern conversion to an unsigned target fits. -/
theorem IntType.convert_widen_neg_unsigned_fits {src to : IntType} {v : Int}
    (hw : 0 < src.width) (h : src.width < to.width) (ht : to.signed = false)
    (hsrc : src.fits v = true) (hv : v < 0) :
    to.fits (v + ((2 : Int) ^ to.width)) = true := by
  have hge := IntType.fits_ge_neg_half hsrc
  have hp : (2 : Int) ^ (src.width - 1) ≤ (2 : Int) ^ to.width :=
    two_pow_le_two_pow (by omega)
  have hpos := two_pow_pos_int to.width
  refine IntType.fits_iff.mpr ⟨?_, ?_⟩
  · simp [IntType.minimum, ht]
    omega
  · simp [IntType.maximum, ht]
    omega

/-- Narrowing to a representable value and widening it back with the same
signedness round-trips. -/
theorem IntType.convert_roundtrip {src to : IntType} {v : Int}
    (hsrcw : src.width ≠ 1) (hgt : to.width < src.width)
    (hs : src.signed = to.signed) (_hf : to.fits v = true) :
    to.convert src v = .ok v := by
  unfold IntType.convert
  rw [ite_eq_right (by simp [show src.width ≠ 1 by omega])]
  rw [ite_eq_left hgt]
  by_cases hneg : to.signed && v < 0
  · rw [ite_eq_left hneg]
    have hsigned : to.signed = true := (Bool.and_eq_true_iff.mp hneg).1
    rw [ite_eq_left (by simpa [hs] using hsigned)]
  · rw [ite_eq_right hneg]

end Stc.Exact
