/-
Mathematical models for the four external operations left opaque by the
filtered Aeneas model:

* `i64::from(u8)` (`I64::from(u8)` in the generated model),
* `i64::rem_euclid`,
* `i128::checked_neg`,
* `i128::abs`.

This module is hand-written, core Lean only, and part of the default `Stc`
package. It states total mathematical models and the guard facts that make
them match the Rust operations on their used domains. The generated axioms
remain axioms; `lean/aeneas/Bridge/Exact/Refinement.lean` states the equations
between each axiom and the model here. No equation is proved in this batch,
and none is a V1 claim.
-/

import Stc.Exact.Int

namespace Stc.Bridge.Exact

/-- The smallest `i64` value. -/
def i64Min : Int := -((2 : Int) ^ 63)

/-- The largest `i64` value. -/
def i64Max : Int := (2 : Int) ^ 63 - 1

/-- The smallest `i128` value. -/
def i128Min : Int := -((2 : Int) ^ 127)

/-- The largest `i128` value. -/
def i128Max : Int := (2 : Int) ^ 127 - 1

/-- `i64::from(u8)`: the natural embedding of a byte into `i64`. -/
def fromU8 (n : Nat) : Int := (n : Int)

/-- `i64::rem_euclid`: Euclidean remainder. The Rust method panics on a zero
divisor, so callers carry `b ≠ 0`. -/
def remEuclid (a b : Int) : Int := a.emod b

/-- `i128::checked_neg`: negation fails exactly at `i128::MIN`. -/
def checkedNeg (a : Int) : Option Int :=
  if a = i128Min then none else some (-a)

/-- `i128::abs` as a total mathematical function. Rust `i128::abs` panics at
`i128::MIN`; the generated `exact.abs` guards that value before the call. -/
def iabs (a : Int) : Int :=
  if a < 0 then -a else a

/-- The `from(u8)` model is the natural embedding. -/
@[simp]
theorem fromU8_eq (n : Nat) : fromU8 n = (n : Int) := rfl

/-- Every byte embeds inside the `i64` range. -/
theorem fromU8_fits {n : Nat} (h : n ≤ 255) :
    i64Min ≤ fromU8 n ∧ fromU8 n ≤ i64Max := by
  have hmin : i64Min ≤ 0 := by decide
  have hmax : (255 : Int) ≤ i64Max := by decide
  constructor
  · simp only [fromU8]
    omega
  · simp only [fromU8]
    omega

/-- Euclidean remainder is non-negative for a nonzero divisor. -/
theorem remEuclid_nonneg (a b : Int) (hb : b ≠ 0) : 0 ≤ remEuclid a b :=
  Int.emod_nonneg a hb

/-- Euclidean remainder is below a positive divisor. -/
theorem remEuclid_lt (a b : Int) (hb : 0 < b) : remEuclid a b < b :=
  Int.emod_lt_of_pos a hb

/-- The model agrees with Euclidean remainder on the generated predicate's
domain: positive divisor. -/
@[simp]
theorem remEuclid_eq_emod (a b : Int) : remEuclid a b = a.emod b := rfl

/-- `checked_neg` returns `none` exactly at `i128::MIN`. -/
@[simp]
theorem checkedNeg_min : checkedNeg i128Min = none := by
  simp [checkedNeg]

/-- `checked_neg` fails exactly at `i128::MIN`. -/
theorem checkedNeg_none_iff {a : Int} : checkedNeg a = none ↔ a = i128Min := by
  unfold checkedNeg
  by_cases h : a = i128Min <;> simp [h]

/-- A successful `checked_neg` returns the exact negation. -/
theorem checkedNeg_some_iff {a r : Int} :
    checkedNeg a = some r ↔ a ≠ i128Min ∧ r = -a := by
  unfold checkedNeg
  by_cases h : a = i128Min
  · simp [h]
  · have hne : ¬ a = i128Min := h
    rw [ite_eq_right hne]
    simp only [Option.some.injEq]
    constructor
    · intro hr
      exact ⟨hne, hr.symm⟩
    · rintro ⟨_, hr⟩
      exact hr.symm

/-- The absolute-value model is non-negative. -/
@[simp]
theorem iabs_nonneg (a : Int) : 0 ≤ iabs a := by
  unfold iabs
  split <;> omega

/-- The absolute-value model leaves a non-negative value unchanged. -/
theorem iabs_of_nonneg {a : Int} (h : 0 ≤ a) : iabs a = a := by
  simp [iabs, Int.not_lt.mpr h]

/-- The absolute-value model negates a negative value. -/
theorem iabs_of_neg {a : Int} (h : a < 0) : iabs a = -a := by
  simp [iabs, h]

/-- At `i128::MIN`, the mathematical absolute value is one past `i128::MAX`.
This is exactly why the Rust function panics there and why the generated
`exact.abs` guards the value before calling it. -/
theorem iabs_i128Min : iabs i128Min = (2 : Int) ^ 127 := by
  have hneg : i128Min < 0 := by
    unfold i128Min
    decide
  rw [iabs_of_neg hneg]
  simp [i128Min]

/-- The absolute-value model stays within the `i128` range away from
`i128::MIN`. -/
theorem iabs_i128_range {a : Int} (hmin : a ≠ i128Min)
    (hlo : i128Min ≤ a) (hhi : a ≤ i128Max) :
    i128Min ≤ iabs a ∧ iabs a ≤ i128Max := by
  have hminLe : i128Min ≤ 0 := by decide
  have hmax : i128Max = -i128Min - 1 := by
    unfold i128Min i128Max
    simp
  by_cases hneg : a < 0
  · rw [iabs_of_neg hneg]
    constructor
    · omega
    · rw [hmax]
      omega
  · rw [iabs_of_nonneg (by omega)]
    exact ⟨hlo, hhi⟩

end Stc.Bridge.Exact
