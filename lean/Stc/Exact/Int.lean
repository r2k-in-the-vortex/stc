/-
Exact integer model: fixed-width checked arithmetic, division, and `ABS`.
-/
import Init.Data.Nat.Bitwise

namespace Stc.Exact

/-- A fixed-width integer layout. `width` is a multiple of 8 up to 64. -/
structure IntType where
  /-- Width in bits. -/
  width : Nat
  /-- `true` for `SINT`/`INT`/`DINT`/`LINT`, `false` for unsigned types. -/
  signed : Bool
deriving Repr, DecidableEq

/-- The number of distinct bit patterns. -/
def IntType.modulus (ty : IntType) : Nat :=
  2 ^ ty.width

/-- The smallest representable value. -/
def IntType.minimum (ty : IntType) : Int :=
  if ty.signed then -((2 : Int) ^ (ty.width - 1)) else 0

/-- The largest representable value. -/
def IntType.maximum (ty : IntType) : Int :=
  if ty.signed then (2 : Int) ^ (ty.width - 1) - 1 else (2 : Int) ^ ty.width - 1

/-- Whether `v` is representable in `ty`. -/
def IntType.fits (ty : IntType) (v : Int) : Bool :=
  decide (ty.minimum ≤ v ∧ v ≤ ty.maximum)

/-- The two's-complement bit pattern of `v` at the type's width. -/
def IntType.toBits (ty : IntType) (v : Int) : Nat :=
  (v.emod (ty.modulus : Int)).toNat

/-- Interprets a bit pattern in the type, wrapping modulo the width. -/
def IntType.fromBits (ty : IntType) (bits : Nat) : Int :=
  let wrapped := bits % ty.modulus
  if ty.signed && wrapped ≥ 2 ^ (ty.width - 1) then
    (wrapped : Int) - (ty.modulus : Int)
  else
    (wrapped : Int)

/-- A checked-operation failure. -/
inductive Trap where
  /-- The exact result is outside the type's range. -/
  | overflow
  /-- The divisor is zero. -/
  | divZero
  /-- A conversion result is not representable. -/
  | range
deriving Repr, DecidableEq

/-- A checked exact result. -/
abbrev Res := Except Trap Int

/-- Wraps an exact integer result in an overflow check. -/
def IntType.checked (ty : IntType) (v : Int) : Res :=
  if ty.fits v then .ok v else .error .overflow

/-- Checked addition. -/
def IntType.add (ty : IntType) (a b : Int) : Res :=
  ty.checked (a + b)

/-- Checked subtraction. -/
def IntType.sub (ty : IntType) (a b : Int) : Res :=
  ty.checked (a - b)

/-- Checked multiplication. -/
def IntType.mul (ty : IntType) (a b : Int) : Res :=
  ty.checked (a * b)

/-- Checked negation (`0 - a`). -/
def IntType.neg (ty : IntType) (a : Int) : Res :=
  ty.checked (-a)

/-- Checked truncating division (`DIV` and the `/` operator). -/
def IntType.div (ty : IntType) (a b : Int) : Res :=
  if b == 0 then
    .error .divZero
  else if ty.signed && a == ty.minimum && b == -1 then
    .error .overflow
  else
    .ok (Int.tdiv a b)

/-- Checked truncating remainder (`MOD`). -/
def IntType.mod (ty : IntType) (a b : Int) : Res :=
  if b == 0 then
    .error .divZero
  else if ty.signed && a == ty.minimum && b == -1 then
    .error .overflow
  else
    .ok (Int.tmod a b)

/-- Checked absolute value: the most-negative signed value overflows. -/
def IntType.abs (ty : IntType) (a : Int) : Res :=
  if ty.signed && a == ty.minimum then
    .error .overflow
  else
    .ok (if a < 0 then -a else a)

/-- `fits` is the range predicate. -/
theorem IntType.fits_iff {ty : IntType} {v : Int} :
    ty.fits v = true ↔ ty.minimum ≤ v ∧ v ≤ ty.maximum := by
  simp [IntType.fits, decide_eq_true_eq]

/-- A checked operation succeeds exactly when its result fits. -/
theorem IntType.checked_ok {ty : IntType} {v r : Int} :
    ty.checked v = .ok r ↔ ty.fits v = true ∧ v = r := by
  unfold IntType.checked
  by_cases h : ty.fits v <;> simp [h]

/-- A checked operation overflows exactly when its result does not fit. -/
theorem IntType.checked_overflow {ty : IntType} {v : Int} :
    ty.checked v = .error .overflow ↔ ty.fits v = false := by
  unfold IntType.checked
  by_cases h : ty.fits v <;> simp [h]

/-- Addition succeeds with the exact sum exactly when the sum fits. -/
theorem IntType.add_ok {ty : IntType} {a b r : Int} :
    ty.add a b = .ok r ↔ ty.fits (a + b) = true ∧ a + b = r :=
  IntType.checked_ok

/-- Addition overflows exactly when the exact sum is out of range. -/
theorem IntType.add_overflow {ty : IntType} {a b : Int} :
    ty.add a b = .error .overflow ↔ ty.fits (a + b) = false :=
  IntType.checked_overflow

/-- Subtraction succeeds with the exact difference exactly when it fits. -/
theorem IntType.sub_ok {ty : IntType} {a b r : Int} :
    ty.sub a b = .ok r ↔ ty.fits (a - b) = true ∧ a - b = r :=
  IntType.checked_ok

/-- Multiplication succeeds with the exact product exactly when it fits. -/
theorem IntType.mul_ok {ty : IntType} {a b r : Int} :
    ty.mul a b = .ok r ↔ ty.fits (a * b) = true ∧ a * b = r :=
  IntType.checked_ok

/-- A successful checked arithmetic result is in range. -/
theorem IntType.ok_range {ty : IntType} {v r : Int}
    (h : ty.checked v = .ok r) : ty.minimum ≤ r ∧ r ≤ ty.maximum := by
  rw [IntType.checked_ok] at h
  have hr : ty.fits r = true := by simpa [h.2] using h.1
  exact IntType.fits_iff.mp hr

/-- Addition is commutative, including its failure mode. -/
theorem IntType.add_comm (ty : IntType) (a b : Int) :
    ty.add a b = ty.add b a := by
  simp [IntType.add, Int.add_comm]

/-- Multiplication is commutative, including its failure mode. -/
theorem IntType.mul_comm (ty : IntType) (a b : Int) :
    ty.mul a b = ty.mul b a := by
  simp [IntType.mul, Int.mul_comm]

/-- Addition with zero succeeds exactly when `a` fits and returns `a`. -/
theorem IntType.add_zero {ty : IntType} {a : Int} (h : ty.fits a = true) :
    ty.add a 0 = .ok a := by
  rw [IntType.add_ok, Int.add_zero]
  exact ⟨h, rfl⟩

/-- Double negation round-trips when both the value and its negation fit. -/
theorem IntType.neg_neg {ty : IntType} {a b : Int} (ha : ty.fits a = true)
    (h : ty.neg a = .ok b) : ty.neg b = .ok a := by
  rw [IntType.neg, IntType.checked_ok] at h
  rw [IntType.neg, IntType.checked_ok]
  refine ⟨?_, by omega⟩
  have hb : b = -a := h.2.symm
  rw [hb]
  simpa using ha

/-- Negation is subtraction from zero. -/
theorem IntType.sub_zero (ty : IntType) (a : Int) :
    ty.sub 0 a = ty.neg a := by
  simp only [IntType.sub, IntType.neg]
  rw [Int.zero_sub]

/-- Division and remainder reconstruct the dividend when both succeed. -/
theorem IntType.div_mod_identity {ty : IntType} {a b q r : Int}
    (hq : ty.div a b = .ok q) (hr : ty.mod a b = .ok r) :
    q * b + r = a := by
  simp only [IntType.div] at hq
  simp only [IntType.mod] at hr
  by_cases hb0 : b == 0
  · simp [hb0] at hq
  · by_cases hov : ty.signed && a == ty.minimum && b == -1
    · simp [hb0, hov] at hq
    · simp [hb0, hov] at hq
      simp [hb0, hov] at hr
      have hq' : q = Int.tdiv a b := by simpa using hq.symm
      have hr' : r = Int.tmod a b := by simpa using hr.symm
      rw [hq', hr']
      exact Int.tdiv_mul_add_tmod a b

/-- `ABS` of a successful result is non-negative. -/
theorem IntType.abs_nonneg {ty : IntType} {a r : Int} (h : ty.abs a = .ok r) :
    0 ≤ r := by
  unfold IntType.abs at h
  split at h
  · simp at h
  · split at h <;> simp only [Except.ok.injEq] at h <;> omega

/-- `ABS` leaves a non-negative value unchanged. -/
theorem IntType.abs_of_nonneg {ty : IntType} {a : Int} (ha : 0 ≤ a)
    (hmin : (ty.signed && a == ty.minimum) = false) : ty.abs a = .ok a := by
  unfold IntType.abs
  rw [hmin]
  simp [Int.not_lt.mpr ha]

/-! ## Complete success and failure characterizations -/

/-- Subtraction overflows exactly when the exact difference is out of range. -/
theorem IntType.sub_overflow {ty : IntType} {a b : Int} :
    ty.sub a b = .error .overflow ↔ ty.fits (a - b) = false :=
  IntType.checked_overflow

/-- Multiplication overflows exactly when the exact product is out of range. -/
theorem IntType.mul_overflow {ty : IntType} {a b : Int} :
    ty.mul a b = .error .overflow ↔ ty.fits (a * b) = false :=
  IntType.checked_overflow

/-- Negation succeeds with the exact negation exactly when it fits. -/
theorem IntType.neg_ok {ty : IntType} {a r : Int} :
    ty.neg a = .ok r ↔ ty.fits (-a) = true ∧ -a = r :=
  IntType.checked_ok

/-- Negation overflows exactly when the exact negation is out of range. -/
theorem IntType.neg_overflow {ty : IntType} {a : Int} :
    ty.neg a = .error .overflow ↔ ty.fits (-a) = false :=
  IntType.checked_overflow

/-- The program-plan form of addition success: under fitting operands, the
result is exactly the sum and the sum fits. -/
theorem IntType.add_ok_iff {ty : IntType} {a b r : Int}
    (_ha : ty.fits a = true) (_hb : ty.fits b = true) :
    ty.add a b = .ok r ↔ r = a + b ∧ ty.fits r = true := by
  rw [IntType.add_ok]
  constructor
  · rintro ⟨hf, h⟩
    exact ⟨h.symm, by simpa [h] using hf⟩
  · rintro ⟨h, hf⟩
    exact ⟨by simpa [← h] using hf, h.symm⟩

/-- The program-plan form of addition overflow: under fitting operands, the
operation overflows exactly when the exact sum does not fit. -/
theorem IntType.add_overflow_iff {ty : IntType} {a b : Int}
    (_ha : ty.fits a = true) (_hb : ty.fits b = true) :
    ty.add a b = .error .overflow ↔ ty.fits (a + b) = false :=
  IntType.add_overflow

/-- The subtraction form of `add_ok_iff`. -/
theorem IntType.sub_ok_iff {ty : IntType} {a b r : Int}
    (_ha : ty.fits a = true) (_hb : ty.fits b = true) :
    ty.sub a b = .ok r ↔ r = a - b ∧ ty.fits r = true := by
  rw [IntType.sub_ok]
  constructor
  · rintro ⟨hf, h⟩
    exact ⟨h.symm, by simpa [h] using hf⟩
  · rintro ⟨h, hf⟩
    exact ⟨by simpa [← h] using hf, h.symm⟩

/-- The subtraction form of `add_overflow_iff`. -/
theorem IntType.sub_overflow_iff {ty : IntType} {a b : Int}
    (_ha : ty.fits a = true) (_hb : ty.fits b = true) :
    ty.sub a b = .error .overflow ↔ ty.fits (a - b) = false :=
  IntType.sub_overflow

/-- The multiplication form of `add_ok_iff`. -/
theorem IntType.mul_ok_iff {ty : IntType} {a b r : Int}
    (_ha : ty.fits a = true) (_hb : ty.fits b = true) :
    ty.mul a b = .ok r ↔ r = a * b ∧ ty.fits r = true := by
  rw [IntType.mul_ok]
  constructor
  · rintro ⟨hf, h⟩
    exact ⟨h.symm, by simpa [h] using hf⟩
  · rintro ⟨h, hf⟩
    exact ⟨by simpa [← h] using hf, h.symm⟩

/-- The multiplication form of `add_overflow_iff`. -/
theorem IntType.mul_overflow_iff {ty : IntType} {a b : Int}
    (_ha : ty.fits a = true) (_hb : ty.fits b = true) :
    ty.mul a b = .error .overflow ↔ ty.fits (a * b) = false :=
  IntType.mul_overflow

/-- The negation form of `add_ok_iff`. -/
theorem IntType.neg_ok_iff {ty : IntType} {a r : Int} (_ha : ty.fits a = true) :
    ty.neg a = .ok r ↔ r = -a ∧ ty.fits r = true := by
  rw [IntType.neg_ok]
  constructor
  · rintro ⟨hf, h⟩
    exact ⟨h.symm, by simpa [h] using hf⟩
  · rintro ⟨h, hf⟩
    exact ⟨by simpa [← h] using hf, h.symm⟩

/-- The negation form of `add_overflow_iff`. -/
theorem IntType.neg_overflow_iff {ty : IntType} {a : Int} (_ha : ty.fits a = true) :
    ty.neg a = .error .overflow ↔ ty.fits (-a) = false :=
  IntType.neg_overflow

/-- `ABS` succeeds exactly when the most-negative guard is not triggered; the
result is the exact absolute value. -/
theorem IntType.abs_ok {ty : IntType} {a r : Int} :
    ty.abs a = .ok r ↔
      (ty.signed && a == ty.minimum) = false ∧
        r = (if a < 0 then -a else a) := by
  unfold IntType.abs
  by_cases hmin : (ty.signed && a == ty.minimum) = true
  · simp [hmin]
  · have hmin' : (ty.signed && a == ty.minimum) = false := by
      simpa using hmin
    simp [hmin']
    exact ⟨Eq.symm, Eq.symm⟩

/-- `ABS` overflows exactly at the most-negative signed value. -/
theorem IntType.abs_overflow {ty : IntType} {a : Int} :
    ty.abs a = .error .overflow ↔ (ty.signed && a == ty.minimum) = true := by
  unfold IntType.abs
  by_cases hmin : (ty.signed && a == ty.minimum) = true <;> simp [hmin]

/-- A successful absolute value fits when the operand fits and the layout has a
positive width. -/
theorem IntType.abs_fits {ty : IntType} {a r : Int} (_hw : 0 < ty.width)
    (ha : ty.fits a = true) (h : ty.abs a = .ok r) : ty.fits r = true := by
  rw [IntType.abs_ok] at h
  rcases h with ⟨hmin, rfl⟩
  rw [IntType.fits_iff] at ha ⊢
  rcases ha with ⟨hlo, hhi⟩
  by_cases hs : ty.signed
  · have hlo' : -((2 : Int) ^ (ty.width - 1)) ≤ a := by
      simpa [IntType.minimum, hs] using hlo
    have hhi' : a ≤ (2 : Int) ^ (ty.width - 1) - 1 := by
      simpa [IntType.maximum, hs] using hhi
    have hmin' : (a == ty.minimum) = false := by simpa [hs] using hmin
    have hnotmin : a ≠ ty.minimum := by
      intro heq
      rw [heq] at hmin'
      simp at hmin'
    have hnotmin' : a ≠ -((2 : Int) ^ (ty.width - 1)) := by
      intro heq
      exact hnotmin (by rw [IntType.minimum, ite_eq_left hs]; exact heq)
    have hgt : -((2 : Int) ^ (ty.width - 1)) < a := by omega
    by_cases hneg : a < 0
    · rw [ite_eq_left hneg]
      constructor
      · simp [IntType.minimum, hs]
        omega
      · simp [IntType.maximum, hs]
        omega
    · rw [ite_eq_right hneg]
      constructor
      · simp [IntType.minimum, hs]
        omega
      · simp [IntType.maximum, hs]
        omega
  · have hlo' : (0 : Int) ≤ a := by simpa [IntType.minimum, hs] using hlo
    have hhi' : a ≤ (2 : Int) ^ ty.width - 1 := by
      simpa [IntType.maximum, hs] using hhi
    have hnonneg : ¬ a < 0 := by omega
    rw [ite_eq_right hnonneg]
    constructor
    · simpa [IntType.minimum, hs] using hlo'
    · simpa [IntType.maximum, hs] using hhi'

/-- Division succeeds exactly when the divisor is nonzero and the signed
most-negative/minus-one guard is not triggered; the result is truncating
division. -/
theorem IntType.div_ok {ty : IntType} {a b r : Int} :
    ty.div a b = .ok r ↔
      b ≠ 0 ∧
        (ty.signed && a == ty.minimum && b == -1) = false ∧
          r = Int.tdiv a b := by
  unfold IntType.div
  by_cases hb : b = 0
  · subst b
    simp
  · by_cases hov : (ty.signed && a == ty.minimum && b == -1) = true
    · simp [hb, hov]
    · have hov' : (ty.signed && a == ty.minimum && b == -1) = false := by
        simpa using hov
      simp [hb, hov']
      exact ⟨Eq.symm, Eq.symm⟩

/-- Division by zero traps. -/
theorem IntType.div_zero (ty : IntType) (a : Int) : ty.div a 0 = .error .divZero := by
  simp [IntType.div]

/-- Division overflows exactly at the signed most-negative value divided by
minus one. -/
theorem IntType.div_overflow_iff {ty : IntType} {a b : Int} :
    ty.div a b = .error .overflow ↔
      b ≠ 0 ∧ (ty.signed && a == ty.minimum && b == -1) = true := by
  unfold IntType.div
  by_cases hb : b = 0 <;> by_cases hov : (ty.signed && a == ty.minimum && b == -1) = true
  all_goals simp [hb, hov]

/-- Remainder succeeds exactly when the divisor is nonzero and the signed
most-negative/minus-one guard is not triggered; the result is the truncating
remainder. -/
theorem IntType.mod_ok {ty : IntType} {a b r : Int} :
    ty.mod a b = .ok r ↔
      b ≠ 0 ∧
        (ty.signed && a == ty.minimum && b == -1) = false ∧
          r = Int.tmod a b := by
  unfold IntType.mod
  by_cases hb : b = 0
  · subst b
    simp
  · by_cases hov : (ty.signed && a == ty.minimum && b == -1) = true
    · simp [hb, hov]
    · have hov' : (ty.signed && a == ty.minimum && b == -1) = false := by
        simpa using hov
      simp [hb, hov']
      exact ⟨Eq.symm, Eq.symm⟩

/-- Remainder by zero traps. -/
theorem IntType.mod_zero (ty : IntType) (a : Int) : ty.mod a 0 = .error .divZero := by
  simp [IntType.mod]

/-- Remainder overflows exactly at the signed most-negative value with divisor
minus one. -/
theorem IntType.mod_overflow_iff {ty : IntType} {a b : Int} :
    ty.mod a b = .error .overflow ↔
      b ≠ 0 ∧ (ty.signed && a == ty.minimum && b == -1) = true := by
  unfold IntType.mod
  by_cases hb : b = 0 <;> by_cases hov : (ty.signed && a == ty.minimum && b == -1) = true
  all_goals simp [hb, hov]

/-! ## Range extrema -/

/-- The minimum never exceeds the maximum. -/
theorem IntType.minimum_le_maximum (ty : IntType) : ty.minimum ≤ ty.maximum := by
  unfold IntType.minimum IntType.maximum
  by_cases hs : ty.signed
  · rw [ite_eq_left hs, ite_eq_left hs]
    have hpos : (0 : Int) < (2 : Int) ^ (ty.width - 1) := by
      have h := Nat.two_pow_pos (ty.width - 1)
      exact_mod_cast h
    omega
  · rw [ite_eq_right hs, ite_eq_right hs]
    have hpos : (1 : Int) ≤ (2 : Int) ^ ty.width := by
      have h := Nat.one_le_two_pow (n := ty.width)
      exact_mod_cast h
    omega

/-- The minimum fits its own layout. -/
theorem IntType.fits_minimum (ty : IntType) : ty.fits ty.minimum = true := by
  rw [IntType.fits_iff]
  exact ⟨Int.le_refl _, ty.minimum_le_maximum⟩

/-- The maximum fits its own layout. -/
theorem IntType.fits_maximum (ty : IntType) : ty.fits ty.maximum = true := by
  rw [IntType.fits_iff]
  exact ⟨ty.minimum_le_maximum, Int.le_refl _⟩

/-- Every value between the minimum and maximum fits. -/
theorem IntType.fits_of_between {ty : IntType} {v : Int}
    (hlo : ty.minimum ≤ v) (hhi : v ≤ ty.maximum) : ty.fits v = true := by
  rw [IntType.fits_iff]
  exact ⟨hlo, hhi⟩

end Stc.Exact
