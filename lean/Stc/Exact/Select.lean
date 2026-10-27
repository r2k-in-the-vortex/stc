/-
Exact selection model: `SEL`, `MIN`, `MAX`, `LIMIT`, and `MUX`.
-/
import Stc.Exact.Int

namespace Stc.Exact

/-- `SEL(G, IN0, IN1)`: the selector chooses `IN1` when true. -/
def sel (g : Bool) (in0 in1 : Int) : Int :=
  if g then in1 else in0

/-- `MIN` over a value list, keeping the earlier value on ties. -/
def minList (values : List Int) : Option Int :=
  values.min?

/-- `MAX` over a value list, keeping the earlier value on ties. -/
def maxList (values : List Int) : Option Int :=
  values.max?

/-- `LIMIT(MN, IN, MX)` clamps to the bounds in declaration order. -/
def limit (mn x mx : Int) : Int :=
  let low := if x < mn then mn else x
  if mx < x then mx else low

/-- `MUX(K, IN0, ..., INn, DEFAULT)`: the last value is the default. -/
def mux (k : Int) (values : List Int) (default : Int) : Int :=
  if 0 ≤ k then
    match values[k.toNat]? with
    | some value => value
    | none => default
  else
    default

/-- `SEL` with a true selector returns the first input value. -/
theorem sel_true (a b : Int) : sel true a b = b := rfl

/-- `SEL` with a false selector returns the second input value. -/
theorem sel_false (a b : Int) : sel false a b = a := rfl

/-- `SEL` with equal inputs is the identity. -/
theorem sel_same (g : Bool) (a : Int) : sel g a a = a := by
  cases g <;> rfl

/-- An empty `MIN` has no result. -/
theorem minList_nil : minList [] = none := rfl

/-- An empty `MAX` has no result. -/
theorem maxList_nil : maxList [] = none := rfl

/-- A successful `MIN` is a lower bound of every value. -/
theorem minList_le {values : List Int} {m : Int} (h : minList values = some m) :
    ∀ v ∈ values, m ≤ v := by
  have h' : values.min? = some m := h
  exact (List.min?_eq_some_iff.mp h').2

/-- A successful `MAX` is an upper bound of every value. -/
theorem maxList_ge {values : List Int} {m : Int} (h : maxList values = some m) :
    ∀ v ∈ values, v ≤ m := by
  have h' : values.max? = some m := h
  exact (List.max?_eq_some_iff.mp h').2

/-- `LIMIT` stays within its bounds when the bounds are ordered. -/
theorem limit_within {mn x mx : Int} (h : mn ≤ mx) :
    mn ≤ limit mn x mx ∧ limit mn x mx ≤ mx := by
  unfold limit
  by_cases h1 : x < mn
  · rw [ite_eq_left h1]
    by_cases h2 : mx < x
    · rw [ite_eq_left h2]
      omega
    · rw [ite_eq_right h2]
      omega
  · rw [ite_eq_right h1]
    by_cases h2 : mx < x
    · rw [ite_eq_left h2]
      omega
    · rw [ite_eq_right h2]
      omega

/-- `LIMIT` leaves an in-range value unchanged. -/
theorem limit_identity {mn x mx : Int} (hlo : mn ≤ x) (hhi : x ≤ mx) :
    limit mn x mx = x := by
  unfold limit
  rw [ite_eq_right (by omega), ite_eq_right (by omega)]

/-- `MUX` on an empty value list returns the default. -/
theorem mux_nil (k d : Int) : mux k [] d = d := by
  simp [mux]

/-- `MUX` with selector zero returns the first value. -/
theorem mux_zero (value : Int) (rest : List Int) (d : Int) :
    mux 0 (value :: rest) d = value := by
  simp [mux]

/-- `MUX` with a negative selector returns the default. -/
theorem mux_negative {k : Int} (hk : k < 0) (values : List Int) (d : Int) :
    mux k values d = d := by
  simp [mux, show ¬ 0 ≤ k by omega]

/-- `MUX` with a selector past the values returns the default. -/
theorem mux_past_end {k : Int} (hk : 0 ≤ k) {values : List Int}
    (hlen : values.length ≤ k.toNat) (d : Int) : mux k values d = d := by
  simp [mux, hk, List.getElem?_eq_none_iff.mpr hlen]

end Stc.Exact
