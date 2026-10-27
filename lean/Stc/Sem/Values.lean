/-
Core values, widths, locals, globals, GOT roots, memory, and traps (P0.3).

The model has no heap and no aliasing beyond GOT cells, so a configuration is a
value store plus a root map plus explicit call and control stacks. Exact
operations reuse `Stc.Exact`; this module supplies the configuration-level
wrappers and the value-shaped exact operations.

Values are typed pairs `⟨ty, value⟩` with an integer payload. The payload of a
well-formed value is representable in its type (`Value.WellFormed`). `BOOL` uses
`0` and `1`, characters use their scalar value, and integer and bit-string
values use their numeric value, exactly as typed HIR literals do.

Operations that the `Stc.Exact` models already range-check (`add`, `sub`,
`mul`, `neg`, `div`, `mod`, `abs`) reuse the model directly. Conversions and
the exact standard functions additionally pass their result through the exact
model's checked wrapper. That check is redundant for representable inputs —
`Stc.Exact` never yields an out-of-range success — but it keeps every runtime
value well formed by construction.
-/
import Stc.Exact.Select
import Stc.Exact.Bits
import Stc.Sem.Typing

namespace Stc.Sem

open Stc.Hir
open Stc.Exact

/-- The core trap set: `OVERFLOW`, `DIV_ZERO`, and `RANGE`. The constructors
are exactly the specified traps; P0.3 adds no others. -/
abbrev Trap := Stc.Exact.Trap

/-- A mapped result is a successful result exactly when the original result
was successful and the mapping preserves the value. -/
theorem Except.map_ok {ε α β : Type} {f : α → β} {r : Except ε α} {v : β}
    (h : r.map f = .ok v) : ∃ a, r = .ok a ∧ f a = v := by
  cases r with
  | error e => simp [Except.map] at h
  | ok a =>
      simp only [Except.map, Except.ok.injEq] at h
      exact ⟨a, rfl, h⟩

/-! ## Values -/

/-- A runtime value: a canonical type and an integer payload. -/
structure Value where
  /-- The value's canonical type. -/
  ty : Ty
  /-- The integer payload. -/
  value : Int
deriving Repr, DecidableEq

namespace Value

/-- A value with the given type and payload. -/
def ofInt (ty : Ty) (value : Int) : Value := ⟨ty, value⟩

/-- The default value of a type: payload `0`. Zero is representable in every
core type. -/
def default (ty : Ty) : Value := ⟨ty, 0⟩

/-- Whether the value has a core type and a representable payload. -/
def wellFormedB (v : Value) : Bool :=
  v.ty.isCore && v.ty.holdsB v.value

/-- The value well-formedness predicate. -/
abbrev WellFormed (v : Value) : Prop :=
  v.wellFormedB = true

/-- A well-formed value has a core type. -/
theorem isCore_of_wellFormed {v : Value} (h : v.WellFormed) :
    v.ty.isCore = true :=
  (Bool.and_eq_true_iff.mp h).1

/-- A well-formed value's payload is representable. -/
theorem holds_of_wellFormed {v : Value} (h : v.WellFormed) :
    v.ty.Holds v.value :=
  (Bool.and_eq_true_iff.mp h).2

/-- Constructing a well-formed value from a core type and a representable
payload. -/
theorem wellFormed_of {ty : Ty} {value : Int}
    (hcore : ty.isCore = true) (hfit : ty.Holds value) :
    (ofInt ty value).WellFormed := by
  simp [WellFormed, wellFormedB, ofInt, hcore, hfit]

/-- A checked result of a core layout is a well-formed value. -/
theorem wellFormed_of_checked {ty : Ty} {value r : Int}
    (hcore : ty.isCore = true) (h : ty.layout.checked value = .ok r) :
    (ofInt ty r).WellFormed := by
  have h' := IntType.checked_ok.mp h
  rw [← h'.2]
  exact wellFormed_of hcore h'.1

/-- A `BOOL` value is normalized to `0` and `1`. -/
def boolValue (b : Bool) : Value := ⟨.bool, if b then 1 else 0⟩

/-- A normalized `BOOL` value is well formed. -/
theorem boolValue_wellFormed (b : Bool) : (boolValue b).WellFormed := by
  cases b <;> decide

/-- A `BOOL` value is true exactly when its payload is nonzero. -/
theorem boolValue_true_iff (b : Bool) : (boolValue b).value ≠ 0 ↔ b = true := by
  cases b <;> simp [boolValue]

end Value

/-- The integer payload of a comparison result. -/
def boolPayload (b : Bool) : Int := if b then 1 else 0

/-- A comparison result is a well-formed `BOOL` value. -/
theorem boolPayload_wellFormed (b : Bool) :
    (Value.ofInt .bool (boolPayload b)).WellFormed := by
  cases b <;> decide

/-! ## Zero is representable -/

/-- Zero is representable in every core type. -/
theorem Ty.holdsB_zero {ty : Ty} (h : ty.isCore = true) : ty.holdsB 0 = true := by
  cases ty with
  | bool => rfl
  | int w s =>
      have hw : Ty.coreWidth w = true := by simpa [Ty.isCore] using h
      simp only [Ty.coreWidth, Bool.or_eq_true, beq_iff_eq, or_assoc] at hw
      rcases hw with rfl | rfl | rfl | rfl <;> (cases s <;> decide)
  | bits w =>
      have hw : Ty.coreWidth w = true := by simpa [Ty.isCore] using h
      simp only [Ty.coreWidth, Bool.or_eq_true, beq_iff_eq, or_assoc] at hw
      rcases hw with rfl | rfl | rfl | rfl <;> decide
  | char k => cases k <;> decide

/-- The default value of a core type is well formed. -/
theorem Value.default_wellFormed {ty : Ty} (h : ty.isCore = true) :
    (Value.default ty).WellFormed :=
  Value.wellFormed_of h (Ty.holdsB_zero h)

/-! ## Exact-layout facts -/

/-- Two to the width is even at a positive width. -/
theorem pow_two_succ (w : Nat) (hw : 0 < w) : (2 : Int) ^ w = 2 * 2 ^ (w - 1) := by
  have h : (2 : Int) ^ w = (2 : Int) ^ ((w - 1) + 1) := by
    rw [show (w - 1) + 1 = w by omega]
  rw [h, Int.pow_succ, Int.mul_comm]

/-- Two to any width is positive. -/
theorem pow_two_pos (w : Nat) : (0 : Int) < 2 ^ w := by
  have : (0 : Nat) < 2 ^ w := Nat.two_pow_pos w
  exact_mod_cast this

/-- The negative absolute value is a lower bound. -/
theorem neg_natAbs_le (a : Int) : -((a.natAbs : Int)) ≤ a := by
  by_cases h : 0 ≤ a
  · have h0 : (0 : Int) ≤ a.natAbs := by exact_mod_cast Nat.zero_le a.natAbs
    omega
  · have h' := Int.eq_neg_natAbs_of_nonpos (by omega : a ≤ 0)
    omega

/-- A representable value's magnitude is bounded by the layout's signed
half-width or unsigned maximum. -/
theorem natAbs_le_of_fits {ty : IntType} {v : Int} (hw : 0 < ty.width)
    (h : ty.fits v = true) :
    v.natAbs ≤ if ty.signed then 2 ^ (ty.width - 1) else 2 ^ ty.width - 1 := by
  have hlo : ty.minimum ≤ v := (IntType.fits_iff.mp h).1
  have hhi : v ≤ ty.maximum := (IntType.fits_iff.mp h).2
  by_cases hs : ty.signed
  · have hmin : ty.minimum = -((2 : Int) ^ (ty.width - 1)) := by
      simp [IntType.minimum, hs]
    have hmax : ty.maximum = (2 : Int) ^ (ty.width - 1) - 1 := by
      simp [IntType.maximum, hs]
    rw [hmin] at hlo
    rw [hmax] at hhi
    rw [if_pos hs]
    have hle : (v.natAbs : Int) ≤ 2 ^ (ty.width - 1) := by
      by_cases hv : 0 ≤ v
      · rw [Int.natAbs_of_nonneg hv]; omega
      · rw [Int.eq_neg_natAbs_of_nonpos (by omega : v ≤ 0)]; omega
    exact_mod_cast hle
  · have hmin : ty.minimum = 0 := by simp [IntType.minimum, hs]
    have hmax : ty.maximum = (2 : Int) ^ ty.width - 1 := by
      simp [IntType.maximum, hs]
    rw [hmin] at hlo
    rw [hmax] at hhi
    rw [if_neg hs]
    have hlt : (v.natAbs : Int) < (2 : Int) ^ ty.width := by
      by_cases hv : 0 ≤ v
      · rw [Int.natAbs_of_nonneg hv]; omega
      · rw [Int.eq_neg_natAbs_of_nonpos (by omega : v ≤ 0)]; omega
    have hltn : v.natAbs < 2 ^ ty.width := by exact_mod_cast hlt
    exact Nat.le_sub_one_of_lt hltn

/-- Checked division never leaves the layout's range. -/
theorem IntType.div_fits {ty : IntType} {a b r : Int} (hw : 0 < ty.width)
    (ha : ty.fits a = true) (hb : ty.fits b = true)
    (h : ty.div a b = .ok r) : ty.fits r = true := by
  have hna := natAbs_le_of_fits hw ha
  have hpow : (2 : Int) ^ ty.width = 2 * 2 ^ (ty.width - 1) := pow_two_succ _ hw
  have hp : (0 : Int) < 2 ^ (ty.width - 1) := pow_two_pos _
  have hlo_a : ty.minimum ≤ a := (IntType.fits_iff.mp ha).1
  have hhi_a : a ≤ ty.maximum := (IntType.fits_iff.mp ha).2
  have hlo_b : ty.minimum ≤ b := (IntType.fits_iff.mp hb).1
  have hhi_b : b ≤ ty.maximum := (IntType.fits_iff.mp hb).2
  unfold IntType.div at h
  by_cases hb0 : b == 0
  · simp only [hb0] at h; cases h
  · by_cases hov : ty.signed && a == ty.minimum && b == -1
    · simp only [hb0, hov] at h; cases h
    · simp only [hb0, hov] at h
      have hr : r = a.tdiv b := by simpa using h.symm
      subst hr
      have hbne : b ≠ 0 := by simpa using hb0
      have hbound : (a.tdiv b).natAbs ≤ a.natAbs := Int.natAbs_tdiv_le_natAbs a b
      rw [IntType.fits_iff]
      by_cases hs : ty.signed
      · have hmin : ty.minimum = -((2 : Int) ^ (ty.width - 1)) := by
          simp [IntType.minimum, hs]
        have hmax : ty.maximum = (2 : Int) ^ (ty.width - 1) - 1 := by
          simp [IntType.maximum, hs]
        rw [hmin, hmax]
        rw [if_pos hs] at hna
        constructor
        · have hN : ((a.tdiv b).natAbs : Int) ≤ 2 ^ (ty.width - 1) := by
            have hle : (a.tdiv b).natAbs ≤ 2 ^ (ty.width - 1) := Nat.le_trans hbound hna
            exact_mod_cast hle
          have := neg_natAbs_le (a.tdiv b)
          omega
        · by_cases hp0 : 0 ≤ a.tdiv b
          · have hcast : ((a.tdiv b).natAbs : Int) = a.tdiv b := Int.natAbs_of_nonneg hp0
            by_cases heq : a.tdiv b = (2 : Int) ^ (ty.width - 1)
            · exfalso
              have hnat : (a.tdiv b).natAbs = 2 ^ (ty.width - 1) := by
                rw [heq]
                have h0 : (0 : Int) ≤ (2 : Int) ^ (ty.width - 1) := by omega
                have hc := Int.natAbs_of_nonneg h0
                simpa using hc
              have haN : a.natAbs = 2 ^ (ty.width - 1) := by
                have h1 : (a.tdiv b).natAbs ≤ a.natAbs := hbound
                have h2 : a.natAbs ≤ 2 ^ (ty.width - 1) := hna
                omega
              have hquot : a.natAbs / b.natAbs = 2 ^ (ty.width - 1) :=
                (Int.natAbs_tdiv a b).symm.trans hnat
              have hbabs1 : b.natAbs = 1 := by
                by_cases hz : b.natAbs = 0
                · have hbz : b = 0 := Int.natAbs_eq_zero.mp hz
                  exact absurd hbz hbne
                · rcases Nat.eq_or_lt_of_le (Nat.one_le_iff_ne_zero.mpr hz) with h1 | h1
                  · exact h1.symm
                  · exfalso
                    have hapos : 0 < a.natAbs := by
                      rw [haN]
                      exact Nat.two_pow_pos _
                    have hlt : a.natAbs / b.natAbs < a.natAbs :=
                      Nat.div_lt_self hapos h1
                    rw [hquot, haN] at hlt
                    exact Nat.lt_irrefl _ hlt
              have hb1 : b = 1 ∨ b = -1 := by
                have := Int.natAbs_eq_iff.mp hbabs1
                simpa using this
              rcases hb1 with hb1 | hb1
              · rw [hb1, Int.tdiv_one] at heq
                omega
              · have hminA : a = ty.minimum := by
                  rw [hmin]
                  have haa : (a.natAbs : Int) = 2 ^ (ty.width - 1) := by exact_mod_cast haN
                  have hle : a ≤ (2 : Int) ^ (ty.width - 1) - 1 := by
                    rw [hmax] at hhi_a; exact hhi_a
                  by_cases hav : 0 ≤ a
                  · rw [Int.natAbs_of_nonneg hav] at haa; omega
                  · rw [Int.eq_neg_natAbs_of_nonpos (by omega : a ≤ 0)] at haa; omega
                have htrue : (ty.signed && a == ty.minimum && b == -1) = true := by
                  simp [hs, hminA, hb1]
                exact hov htrue
            · omega
          · omega
      · have hmin : ty.minimum = 0 := by simp [IntType.minimum, hs]
        have hmax : ty.maximum = (2 : Int) ^ ty.width - 1 := by
          simp [IntType.maximum, hs]
        rw [hmin, hmax]
        rw [if_neg hs] at hna
        have hbgt : 0 < b := by rw [hmin] at hlo_b; omega
        have hnonneg : 0 ≤ a.tdiv b :=
          Int.tdiv_nonneg (by rw [hmin] at hlo_a; exact hlo_a) (by omega)
        have hlt : (a.tdiv b).natAbs < 2 ^ ty.width :=
          Nat.lt_of_le_of_lt (Nat.le_trans hbound hna)
            (Nat.sub_one_lt (by have := Nat.two_pow_pos ty.width; omega))
        have hltc : ((a.tdiv b).natAbs : Int) < (2 : Int) ^ ty.width := by exact_mod_cast hlt
        have hcast : ((a.tdiv b).natAbs : Int) = a.tdiv b := Int.natAbs_of_nonneg hnonneg
        constructor <;> omega

/-- Checked remainder never leaves the layout's range. -/
theorem IntType.mod_fits {ty : IntType} {a b r : Int} (hw : 0 < ty.width)
    (ha : ty.fits a = true) (hb : ty.fits b = true)
    (h : ty.mod a b = .ok r) : ty.fits r = true := by
  have hna := natAbs_le_of_fits hw ha
  have hpow : (2 : Int) ^ ty.width = 2 * 2 ^ (ty.width - 1) := pow_two_succ _ hw
  have hp : (0 : Int) < 2 ^ (ty.width - 1) := pow_two_pos _
  have hlo_a : ty.minimum ≤ a := (IntType.fits_iff.mp ha).1
  have hhi_a : a ≤ ty.maximum := (IntType.fits_iff.mp ha).2
  have hlo_b : ty.minimum ≤ b := (IntType.fits_iff.mp hb).1
  have hhi_b : b ≤ ty.maximum := (IntType.fits_iff.mp hb).2
  unfold IntType.mod at h
  by_cases hb0 : b == 0
  · simp only [hb0] at h; cases h
  · by_cases hov : ty.signed && a == ty.minimum && b == -1
    · simp only [hb0, hov] at h; cases h
    · simp only [hb0, hov] at h
      have hr : r = a.tmod b := by simpa using h.symm
      subst hr
      have hbne : b ≠ 0 := by simpa using hb0
      have hbound : (a.tmod b).natAbs ≤ a.natAbs := by
        rw [Int.natAbs_tmod]; exact Nat.mod_le _ _
      rw [IntType.fits_iff]
      by_cases hs : ty.signed
      · have hmin : ty.minimum = -((2 : Int) ^ (ty.width - 1)) := by
          simp [IntType.minimum, hs]
        have hmax : ty.maximum = (2 : Int) ^ (ty.width - 1) - 1 := by
          simp [IntType.maximum, hs]
        rw [hmin, hmax]
        have hbabs : (b.natAbs : Int) ≤ 2 ^ (ty.width - 1) := by
          by_cases hv : 0 ≤ b
          · rw [Int.natAbs_of_nonneg hv]; rw [hmin] at hlo_b; omega
          · rw [Int.eq_neg_natAbs_of_nonpos (by omega : b ≤ 0)]; rw [hmin] at hlo_b; omega
        have hbabsN : b.natAbs ≤ 2 ^ (ty.width - 1) := by exact_mod_cast hbabs
        have hmodlt : a.natAbs % b.natAbs < b.natAbs := by
          apply Nat.mod_lt
          exact Nat.pos_of_ne_zero (by intro hz; exact hbne (Int.natAbs_eq_zero.mp hz))
        have hlt : (a.tmod b).natAbs < 2 ^ (ty.width - 1) := by
          rw [Int.natAbs_tmod]
          exact Nat.lt_of_lt_of_le hmodlt hbabsN
        constructor
        · have hN : ((a.tmod b).natAbs : Int) ≤ 2 ^ (ty.width - 1) := by
            have hle : (a.tmod b).natAbs ≤ 2 ^ (ty.width - 1) := Nat.le_of_lt hlt
            exact_mod_cast hle
          have := neg_natAbs_le (a.tmod b)
          omega
        · by_cases hp0 : 0 ≤ a.tmod b
          · have hcast : ((a.tmod b).natAbs : Int) = a.tmod b := Int.natAbs_of_nonneg hp0
            have hltc : ((a.tmod b).natAbs : Int) < 2 ^ (ty.width - 1) := by exact_mod_cast hlt
            omega
          · omega
      · have hmin : ty.minimum = 0 := by simp [IntType.minimum, hs]
        have hmax : ty.maximum = (2 : Int) ^ ty.width - 1 := by
          simp [IntType.maximum, hs]
        rw [hmin, hmax]
        rw [if_neg hs] at hna
        have hnonneg : 0 ≤ a.tmod b := by
          rw [hmin] at hlo_a
          exact Int.tmod_nonneg b hlo_a
        have hlt : (a.tmod b).natAbs < 2 ^ ty.width :=
          Nat.lt_of_le_of_lt (Nat.le_trans hbound hna)
            (Nat.sub_one_lt (by have := Nat.two_pow_pos ty.width; omega))
        have hltc : ((a.tmod b).natAbs : Int) < (2 : Int) ^ ty.width := by exact_mod_cast hlt
        have hcast : ((a.tmod b).natAbs : Int) = a.tmod b := Int.natAbs_of_nonneg hnonneg
        constructor <;> omega

/-! ## Bit-pattern operations -/

/-- An unsigned layout interprets a bit pattern below its modulus as a
representable value. -/
theorem IntType.fromBits_fits_unsigned {w : Nat} {bits : Nat}
    (h : bits < 2 ^ w) :
    (⟨w, false⟩ : IntType).fits ((⟨w, false⟩ : IntType).fromBits bits) = true := by
  have hfrom : (⟨w, false⟩ : IntType).fromBits bits = (bits : Int) := by
    simp [IntType.fromBits, IntType.modulus, Nat.mod_eq_of_lt h]
  rw [hfrom, IntType.fits_iff]
  constructor
  · simp [IntType.minimum]
  · simp [IntType.maximum]
    have hb : (bits : Int) < (2 : Int) ^ w := by exact_mod_cast h
    omega

/-- Bit-pattern operations at `BOOL` and bit-string types stay representable. -/
theorem Ty.bitPattern_fits {ty : Ty} (hbit : ty.isBitwise = true) (bits : Nat)
    (hbits : bits < ty.layout.modulus) : ty.Holds (ty.layout.fromBits bits) := by
  cases ty with
  | bool =>
      exact IntType.fromBits_fits_unsigned (w := 1) (by
        simpa [Ty.layout, IntType.modulus] using hbits)
  | int w s => rw [Ty.isBitwise] at hbit; cases hbit
  | bits w =>
      exact IntType.fromBits_fits_unsigned (w := w) (by
        simpa [Ty.layout, IntType.modulus] using hbits)
  | char k => rw [Ty.isBitwise] at hbit; cases hbit

/-- Bitwise AND at a type, on the low `width` bits of the payload. -/
def bitAndVal (ty : Ty) (a b : Int) : Int :=
  ty.layout.fromBits (ty.layout.bitAnd (ty.layout.toBits a) (ty.layout.toBits b))

/-- Bitwise OR at a type. -/
def bitOrVal (ty : Ty) (a b : Int) : Int :=
  ty.layout.fromBits (ty.layout.bitOr (ty.layout.toBits a) (ty.layout.toBits b))

/-- Bitwise XOR at a type. -/
def bitXorVal (ty : Ty) (a b : Int) : Int :=
  ty.layout.fromBits (ty.layout.bitXor (ty.layout.toBits a) (ty.layout.toBits b))

/-- Bitwise NOT at a type. -/
def bitNotVal (ty : Ty) (a : Int) : Int :=
  ty.layout.fromBits (ty.layout.bitNot (ty.layout.toBits a))

/-- Bitwise AND of a bitwise type is representable. -/
theorem bitAndVal_fits {ty : Ty} (h : ty.isBitwise = true) (a b : Int) :
    ty.Holds (bitAndVal ty a b) :=
  Ty.bitPattern_fits h _ (IntType.bitAnd_lt _ _ _)

/-- Bitwise OR of a bitwise type is representable. -/
theorem bitOrVal_fits {ty : Ty} (h : ty.isBitwise = true) (a b : Int) :
    ty.Holds (bitOrVal ty a b) :=
  Ty.bitPattern_fits h _ (IntType.bitOr_lt _ _ _)

/-- Bitwise XOR of a bitwise type is representable. -/
theorem bitXorVal_fits {ty : Ty} (h : ty.isBitwise = true) (a b : Int) :
    ty.Holds (bitXorVal ty a b) :=
  Ty.bitPattern_fits h _ (IntType.bitXor_lt _ _ _)

/-- Bitwise NOT of a bitwise type is representable. -/
theorem bitNotVal_fits {ty : Ty} (h : ty.isBitwise = true) (a : Int) :
    ty.Holds (bitNotVal ty a) :=
  Ty.bitPattern_fits h _ (IntType.bitNot_lt _ _)

/-! ## Checked exponentiation -/

/-- Checked exponentiation by squaring, mirroring the code generator's integer
power helper: a `0` exponent yields the accumulator, an odd exponent multiplies
the accumulator by the current base, and the base is squared for the next
round. Every multiplication is checked, so the first overflow traps. -/
def powLoop (layout : IntType) : Nat → Int → Int → Res
  | 0, _, acc => .ok acc
  | e + 1, base, acc =>
      let acc' :=
        if (e + 1) % 2 == 1 then layout.mul acc base else .ok acc
      match acc' with
      | .error trap => .error trap
      | .ok acc'' =>
          if (e + 1) / 2 == 0 then .ok acc''
          else
            match layout.mul base base with
            | .error trap => .error trap
            | .ok base' => powLoop layout ((e + 1) / 2) base' acc''
termination_by e => e

/-- Checked integer exponentiation. A negative exponent in a signed layout is
a `RANGE` trap; the result is checked against the layout, so a representable
power succeeds and any other power traps with `OVERFLOW`. -/
def evalPow (ty : Ty) (base exponent : Int) : Except Trap Value :=
  if ty.layout.signed && exponent < 0 then
    .error .range
  else
    match powLoop ty.layout exponent.toNat base 1 with
    | .error trap => .error trap
    | .ok r => (ty.layout.checked r).map (Value.ofInt ty)

/-! ## Unary operations -/

/-- Evaluate a unary operation at a type. `NOT` is bitwise on `BOOL` and bit
strings, `-` is checked negation, and `+` is the identity. -/
def evalUnary (op : UnaryOp) (ty : Ty) (a : Int) : Except Trap Value :=
  match op with
  | .neg => (ty.layout.neg a).map (Value.ofInt ty)
  | .pos => .ok (Value.ofInt ty a)
  | .not => .ok (Value.ofInt ty (bitNotVal ty a))

/-- A successful unary operation has the operand's type and is well formed,
provided the operand is well formed. -/
theorem evalUnary_ok {op : UnaryOp} {ty : Ty} {a : Int} {v : Value}
    (hcore : ty.isCore = true) (hop : unaryWellTyped op ty = true)
    (ha : ty.Holds a) (h : evalUnary op ty a = .ok v) :
    v.ty = ty ∧ v.WellFormed := by
  cases op with
  | neg =>
      simp only [evalUnary] at h
      rcases Except.map_ok h with ⟨r, hr, hv⟩
      subst hv
      exact ⟨rfl, Value.wellFormed_of_checked hcore hr⟩
  | pos =>
      simp only [evalUnary, Except.ok.injEq] at h
      subst h
      exact ⟨rfl, Value.wellFormed_of hcore ha⟩
  | not =>
      simp only [evalUnary, Except.ok.injEq] at h
      subst h
      have hbit : ty.isBitwise = true := by simpa [unaryWellTyped] using hop
      exact ⟨rfl, Value.wellFormed_of hcore (bitNotVal_fits hbit a)⟩

/-! ## Binary operations -/

/-- Compare two in-range core values. Signed and unsigned payloads are both
stored as their numeric value, so comparison is numeric. -/
def compareValues : BinaryOp → Int → Int → Bool
  | .eq, a, b => a == b
  | .ne, a, b => a != b
  | .lt, a, b => decide (a < b)
  | .le, a, b => decide (a ≤ b)
  | .gt, a, b => decide (b < a)
  | .ge, a, b => decide (b ≤ a)
  | _, _, _ => false

/-- Evaluate a binary operation at a common operand type. Arithmetic is
checked, division and remainder trap on a zero divisor, bitwise operations act
on the type's bit pattern, and comparisons produce `BOOL`. -/
def evalBinary (op : BinaryOp) (ty : Ty) (a b : Int) : Except Trap Value :=
  match op with
  | .add => (ty.layout.add a b).map (Value.ofInt ty)
  | .sub => (ty.layout.sub a b).map (Value.ofInt ty)
  | .mul => (ty.layout.mul a b).map (Value.ofInt ty)
  | .div => (ty.layout.div a b).map (Value.ofInt ty)
  | .mod => (ty.layout.mod a b).map (Value.ofInt ty)
  | .pow => evalPow ty a b
  | .and => .ok (Value.ofInt ty (bitAndVal ty a b))
  | .or => .ok (Value.ofInt ty (bitOrVal ty a b))
  | .xor => .ok (Value.ofInt ty (bitXorVal ty a b))
  | .eq | .ne | .lt | .le | .gt | .ge =>
      .ok (Value.ofInt .bool (boolPayload (compareValues op a b)))

/-- A successful binary operation has the comparison or operand type and is
well formed. -/
theorem evalBinary_ok {op : BinaryOp} {ty : Ty} {a b : Int} {v : Value}
    (hcore : ty.isCore = true) (hop : binaryWellTyped op ty ty = true)
    (ha : ty.Holds a) (hb : ty.Holds b)
    (h : evalBinary op ty a b = .ok v) :
    v.ty = (if op.isComparison then .bool else ty) ∧ v.WellFormed := by
  have hw : 0 < ty.layout.width := Ty.layout_width_pos hcore
  cases op with
  | add =>
      simp only [evalBinary, BinaryOp.isComparison] at h ⊢
      rcases Except.map_ok h with ⟨r, hr, hv⟩
      subst hv
      exact ⟨rfl, Value.wellFormed_of_checked hcore hr⟩
  | sub =>
      simp only [evalBinary, BinaryOp.isComparison] at h ⊢
      rcases Except.map_ok h with ⟨r, hr, hv⟩
      subst hv
      exact ⟨rfl, Value.wellFormed_of_checked hcore hr⟩
  | mul =>
      simp only [evalBinary, BinaryOp.isComparison] at h ⊢
      rcases Except.map_ok h with ⟨r, hr, hv⟩
      subst hv
      exact ⟨rfl, Value.wellFormed_of_checked hcore hr⟩
  | div =>
      simp only [evalBinary, BinaryOp.isComparison] at h ⊢
      rcases Except.map_ok h with ⟨r, hr, hv⟩
      subst hv
      exact ⟨rfl, Value.wellFormed_of hcore (IntType.div_fits hw ha hb hr)⟩
  | mod =>
      simp only [evalBinary, BinaryOp.isComparison] at h ⊢
      rcases Except.map_ok h with ⟨r, hr, hv⟩
      subst hv
      exact ⟨rfl, Value.wellFormed_of hcore (IntType.mod_fits hw ha hb hr)⟩
  | pow =>
      simp only [evalBinary, BinaryOp.isComparison] at h ⊢
      unfold evalPow at h
      split at h
      · cases h
      · split at h
        · cases h
        · next r hr =>
          rcases Except.map_ok h with ⟨r', hr', hv⟩
          subst hv
          exact ⟨rfl, Value.wellFormed_of_checked hcore hr'⟩
  | and =>
      have hbit : ty.isBitwise = true := by
        have hh := hop
        simp only [binaryWellTyped] at hh
        exact (Bool.and_eq_true_iff.mp hh).2
      simp only [evalBinary, BinaryOp.isComparison, Except.ok.injEq] at h
      subst h
      exact ⟨rfl, Value.wellFormed_of hcore (bitAndVal_fits hbit a b)⟩
  | or =>
      have hbit : ty.isBitwise = true := by
        have hh := hop
        simp only [binaryWellTyped] at hh
        exact (Bool.and_eq_true_iff.mp hh).2
      simp only [evalBinary, BinaryOp.isComparison, Except.ok.injEq] at h
      subst h
      exact ⟨rfl, Value.wellFormed_of hcore (bitOrVal_fits hbit a b)⟩
  | xor =>
      have hbit : ty.isBitwise = true := by
        have hh := hop
        simp only [binaryWellTyped] at hh
        exact (Bool.and_eq_true_iff.mp hh).2
      simp only [evalBinary, BinaryOp.isComparison, Except.ok.injEq] at h
      subst h
      exact ⟨rfl, Value.wellFormed_of hcore (bitXorVal_fits hbit a b)⟩
  | eq =>
      simp only [evalBinary, BinaryOp.isComparison, Except.ok.injEq] at h
      subst h
      exact ⟨rfl, boolPayload_wellFormed _⟩
  | ne =>
      simp only [evalBinary, BinaryOp.isComparison, Except.ok.injEq] at h
      subst h
      exact ⟨rfl, boolPayload_wellFormed _⟩
  | lt =>
      simp only [evalBinary, BinaryOp.isComparison, Except.ok.injEq] at h
      subst h
      exact ⟨rfl, boolPayload_wellFormed _⟩
  | le =>
      simp only [evalBinary, BinaryOp.isComparison, Except.ok.injEq] at h
      subst h
      exact ⟨rfl, boolPayload_wellFormed _⟩
  | gt =>
      simp only [evalBinary, BinaryOp.isComparison, Except.ok.injEq] at h
      subst h
      exact ⟨rfl, boolPayload_wellFormed _⟩
  | ge =>
      simp only [evalBinary, BinaryOp.isComparison, Except.ok.injEq] at h
      subst h
      exact ⟨rfl, boolPayload_wellFormed _⟩

/-! ## Conversions -/

/-- Converts a value and re-checks its representability. `Stc.Exact` never
returns an out-of-range success, so the check is a well-formedness guard; a
conversion that leaves the destination range traps with `RANGE`. -/
def evalCast (src to : Ty) (a : Int) : Except Trap Value :=
  match src.layout.convert to.layout a with
  | .error trap => .error trap
  | .ok r => if to.layout.fits r then .ok (Value.ofInt to r) else .error .range

/-- A successful conversion has the destination type and is well formed. -/
theorem evalCast_ok {src to : Ty} {a : Int} {v : Value}
    (hto : to.isCore = true) (h : evalCast src to a = .ok v) :
    v.ty = to ∧ v.WellFormed := by
  unfold evalCast at h
  split at h
  · cases h
  · split at h
    · next hfit =>
      cases h
      exact ⟨rfl, Value.wellFormed_of hto hfit⟩
    · cases h

/-- The conversion result is exactly `Stc.Exact.IntType.convert` followed by
the representability check. -/
theorem evalCast_eq_convert (src to : Ty) (a : Int) :
    evalCast src to a =
      match src.layout.convert to.layout a with
      | .error trap => .error trap
      | .ok r => if to.layout.fits r then .ok (Value.ofInt to r) else .error .range :=
  rfl

/-! ## Exact standard functions -/

/-- The last element of a list, if any. -/
def listLast? : List α → Option α
  | [] => none
  | [x] => some x
  | _ :: rest => listLast? rest

/-- A successful `listLast?` result is a member. -/
theorem listLast?_mem {xs : List α} {a : α} (h : listLast? xs = some a) : a ∈ xs := by
  induction xs with
  | nil => simp [listLast?] at h
  | cons x rest ih =>
      cases rest with
      | nil =>
          simp [listLast?] at h
          subst h
          exact List.mem_cons_self
      | cons y ys =>
          simp only [listLast?] at h
          exact List.mem_cons_of_mem _ (ih h)

/-- `listLast?` is defined on every nonempty list. -/
theorem listLast?_isSome {xs : List α} (h : xs ≠ []) : listLast? xs ≠ none := by
  induction xs with
  | nil => exact absurd rfl h
  | cons x rest ih =>
      cases rest with
      | nil => simp [listLast?]
      | cons y ys =>
          simp only [listLast?]
          exact ih (by simp)

/-- All but the last element of a list. -/
def listDropLast : List α → List α
  | [] => []
  | [_] => []
  | x :: rest => x :: listDropLast rest

/-- Evaluate an exact standard function on an argument list. Malformed
argument lists (not reachable from typed HIR) return the default `BOOL` value;
they cannot occur for a well-typed call. Every result is checked against its
type, which is redundant because the result is always one of the arguments. -/
def evalStd (fn : StdFn) (args : List Value) : Except Trap Value :=
  match fn with
  | .abs =>
      match args with
      | [x] =>
          match x.ty.layout.abs x.value with
          | .error trap => .error trap
          | .ok r => (x.ty.layout.checked r).map (Value.ofInt x.ty)
      | _ => .ok (Value.default .bool)
  | .sel =>
      match args with
      | [g, x, y] =>
          (x.ty.layout.checked (if g.value == 0 then x.value else y.value)).map
            (Value.ofInt x.ty)
      | _ => .ok (Value.default .bool)
  | .min =>
      match args with
      | [x, y] => (x.ty.layout.checked (if y.value < x.value then y.value else x.value)).map
          (Value.ofInt x.ty)
      | _ => .ok (Value.default .bool)
  | .max =>
      match args with
      | [x, y] => (x.ty.layout.checked (if x.value < y.value then y.value else x.value)).map
          (Value.ofInt x.ty)
      | _ => .ok (Value.default .bool)
  | .limit =>
      match args with
      | [mn, x, mx] =>
          (x.ty.layout.checked (limit mn.value x.value mx.value)).map (Value.ofInt x.ty)
      | _ => .ok (Value.default .bool)
  | .mux =>
      match args with
      | k :: rest =>
          match listLast? rest with
          | some d =>
              (d.ty.layout.checked
                (mux k.value (rest.dropLast.map (·.value)) d.value)).map
                (Value.ofInt d.ty)
          | none => .ok (Value.default .bool)
      | [] => .ok (Value.default .bool)

/-- A membership witness in a `Forall2` relation. -/
theorem Forall2.exists_ty_of_mem {R : Value → Ty → Prop} {vs : List Value} {ts : List Ty}
    (h : Forall2 R vs ts) {v : Value} (hv : v ∈ vs) :
    ∃ t, t ∈ ts ∧ R v t := by
  induction h with
  | nil => simp at hv
  | cons hab htail ih =>
      simp only [List.mem_cons] at hv
      rcases hv with rfl | hv
      · exact ⟨_, List.mem_cons_self, hab⟩
      · rcases ih hv with ⟨t, ht, htv⟩
        exact ⟨t, List.mem_cons_of_mem _ ht, htv⟩

/-- Every type in a list equal to `t` is `t`. -/
theorem List.eq_of_all_beq {ts : List Ty} {t : Ty}
    (h : ts.all (fun x => x == t) = true) {x : Ty} (hx : x ∈ ts) : x = t := by
  have := (List.all_eq_true.mp h) x hx
  simpa using this

/-- A successful exact standard function has the standard-function result type
and is well formed, provided every argument is well formed and fits the
standard signature. -/
theorem evalStd_ok {fn : StdFn} {args : List Value} {argTys : List Ty} {ty : Ty}
    (hargs : Forall2 (fun (a : Value) (t : Ty) => a.ty = t ∧ a.WellFormed) args argTys)
    (hres : stdResult? fn argTys = some ty) (hcore : ty.isCore = true)
    (h : evalStd fn args = .ok v) :
    v.ty = ty ∧ v.WellFormed := by
  cases fn with
  | abs =>
      cases argTys with
      | nil => simp [stdResult?] at hres
      | cons t ts =>
          cases ts with
          | cons t2 ts2 => simp [stdResult?] at hres
          | nil =>
              simp only [stdResult?] at hres
              by_cases hti : t.isInt = true
              · simp only [hti, if_true, Option.some.injEq] at hres
                rw [← hres]
                cases hargs with
                | cons hx hnil =>
                    cases hnil
                    simp only [evalStd] at h
                    split at h
                    · cases h
                    · rcases Except.map_ok h with ⟨r, hr, hv⟩
                      subst hv
                      exact ⟨hx.1,
                        Value.wellFormed_of_checked (Value.isCore_of_wellFormed hx.2) hr⟩
              · simp [hti] at hres
  | sel =>
      cases argTys with
      | nil => simp [stdResult?] at hres
      | cons t1 ts1 =>
          cases ts1 with
          | nil => simp [stdResult?] at hres
          | cons t2 ts2 =>
              cases ts2 with
              | nil => simp [stdResult?] at hres
              | cons t3 ts3 =>
                  cases ts3 with
                  | cons t4 ts4 => simp [stdResult?] at hres
                  | nil =>
                      simp only [stdResult?] at hres
                      by_cases hg : t1.isBool = true
                      · by_cases heq : (t2 == t3) = true
                        · simp only [hg, heq, Bool.and_true, if_true, Option.some.injEq] at hres
                          rw [← hres]
                          cases hargs with
                          | cons hgv rest =>
                              cases rest with
                              | cons hx rest2 =>
                                  cases rest2 with
                                  | cons hy hnil =>
                                      cases hnil
                                      simp only [evalStd] at h
                                      rcases Except.map_ok h with ⟨r, hr, hv⟩
                                      subst hv
                                      exact ⟨hx.1,
                                        Value.wellFormed_of_checked (Value.isCore_of_wellFormed hx.2) hr⟩
                        · simp [hg, heq] at hres
                      · simp [hg] at hres
  | min =>
      cases argTys with
      | nil => simp [stdResult?] at hres
      | cons t1 ts1 =>
          cases ts1 with
          | nil => simp [stdResult?] at hres
          | cons t2 ts2 =>
              cases ts2 with
              | cons t3 ts3 => simp [stdResult?] at hres
              | nil =>
                  simp only [stdResult?] at hres
                  by_cases heq : (t1 == t2 && t1.isMagnitude) = true
                  · simp only [heq, if_true, Option.some.injEq] at hres
                    rw [← hres]
                    cases hargs with
                    | cons hx rest =>
                        cases rest with
                        | cons hy hnil =>
                            cases hnil
                            simp only [evalStd] at h
                            rcases Except.map_ok h with ⟨r, hr, hv⟩
                            subst hv
                            exact ⟨hx.1,
                              Value.wellFormed_of_checked (Value.isCore_of_wellFormed hx.2) hr⟩
                  · simp [heq] at hres
  | max =>
      cases argTys with
      | nil => simp [stdResult?] at hres
      | cons t1 ts1 =>
          cases ts1 with
          | nil => simp [stdResult?] at hres
          | cons t2 ts2 =>
              cases ts2 with
              | cons t3 ts3 => simp [stdResult?] at hres
              | nil =>
                  simp only [stdResult?] at hres
                  by_cases heq : (t1 == t2 && t1.isMagnitude) = true
                  · simp only [heq, if_true, Option.some.injEq] at hres
                    rw [← hres]
                    cases hargs with
                    | cons hx rest =>
                        cases rest with
                        | cons hy hnil =>
                            cases hnil
                            simp only [evalStd] at h
                            rcases Except.map_ok h with ⟨r, hr, hv⟩
                            subst hv
                            exact ⟨hx.1,
                              Value.wellFormed_of_checked (Value.isCore_of_wellFormed hx.2) hr⟩
                  · simp [heq] at hres
  | limit =>
      cases argTys with
      | nil => simp [stdResult?] at hres
      | cons t1 ts1 =>
          cases ts1 with
          | nil => simp [stdResult?] at hres
          | cons t2 ts2 =>
              cases ts2 with
              | nil => simp [stdResult?] at hres
              | cons t3 ts3 =>
                  cases ts3 with
                  | cons t4 ts4 => simp [stdResult?] at hres
                  | nil =>
                      simp only [stdResult?] at hres
                      by_cases h1 : (t1 == t2 && t2 == t3 && t2.isMagnitude) = true
                      · simp only [h1, if_true, Option.some.injEq] at hres
                        rw [← hres]
                        cases hargs with
                        | cons hmn rest =>
                            cases rest with
                            | cons hx rest2 =>
                                cases rest2 with
                                | cons hmx hnil =>
                                    cases hnil
                                    simp only [evalStd] at h
                                    rcases Except.map_ok h with ⟨r, hr, hv⟩
                                    subst hv
                                    exact ⟨hx.1,
                                      Value.wellFormed_of_checked (Value.isCore_of_wellFormed hx.2) hr⟩
                      · simp [h1] at hres
  | mux =>
      cases argTys with
      | nil => simp [stdResult?] at hres
      | cons kt sits =>
          cases sits with
          | nil => simp [stdResult?] at hres
          | cons dt dts =>
              simp only [stdResult?] at hres
              by_cases hk : kt.isInt = true
              · by_cases hall : dts.all (fun t => t == dt) = true
                · simp only [hk, hall, Bool.and_true, if_true, Option.some.injEq] at hres
                  rw [← hres]
                  cases args with
                  | nil => cases hargs
                  | cons k rest =>
                      cases hargs with
                      | cons hk_rest hrest =>
                          simp only [evalStd] at h
                          split at h
                          · next last hlast =>
                              rcases Except.map_ok h with ⟨r, hr, hv⟩
                              subst hv
                              have hlast_mem : last ∈ rest := listLast?_mem hlast
                              have hlastTy : last.ty = dt := by
                                rcases Forall2.exists_ty_of_mem hrest hlast_mem with ⟨t, ht, htv⟩
                                rw [htv.1]
                                by_cases hdt : t = dt
                                · exact hdt
                                · have htmem : t ∈ dts := by
                                    simp only [List.mem_cons] at ht
                                    rcases ht with htd | htd
                                    · exact absurd htd hdt
                                    · exact htd
                                  exact List.eq_of_all_beq hall htmem
                              exact ⟨hlastTy,
                                Value.wellFormed_of_checked (by rw [hlastTy, hres]; exact hcore) hr⟩
                          · next hnone =>
                              exact absurd hnone (listLast?_isSome (by
                                intro hnil
                                rw [hnil] at hrest
                                cases hrest))
                · simp [hk, hall] at hres
              · simp [hk] at hres
