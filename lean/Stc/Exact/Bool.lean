/-
Exact boolean model: `AND`, `OR`, `XOR`, and `NOT` on `BOOL`.
-/
import Stc.Exact.Int

namespace Stc.Exact

/-- Boolean `AND`. -/
def boolAnd (a b : Bool) : Bool := a && b

/-- Boolean `OR`. -/
def boolOr (a b : Bool) : Bool := a || b

/-- Boolean `XOR`. -/
def boolXor (a b : Bool) : Bool := a != b

/-- Boolean `NOT`. -/
def boolNot (a : Bool) : Bool := !a

/-- `AND` is commutative. -/
theorem boolAnd_comm (a b : Bool) : boolAnd a b = boolAnd b a := by
  cases a <;> cases b <;> rfl

/-- `OR` is commutative. -/
theorem boolOr_comm (a b : Bool) : boolOr a b = boolOr b a := by
  cases a <;> cases b <;> rfl

/-- `XOR` is commutative. -/
theorem boolXor_comm (a b : Bool) : boolXor a b = boolXor b a := by
  cases a <;> cases b <;> rfl

/-- `AND` is associative. -/
theorem boolAnd_assoc (a b c : Bool) :
    boolAnd (boolAnd a b) c = boolAnd a (boolAnd b c) := by
  cases a <;> cases b <;> cases c <;> rfl

/-- `OR` is associative. -/
theorem boolOr_assoc (a b c : Bool) :
    boolOr (boolOr a b) c = boolOr a (boolOr b c) := by
  cases a <;> cases b <;> cases c <;> rfl

/-- `XOR` is associative. -/
theorem boolXor_assoc (a b c : Bool) :
    boolXor (boolXor a b) c = boolXor a (boolXor b c) := by
  cases a <;> cases b <;> cases c <;> rfl

/-- `AND` is idempotent. -/
theorem boolAnd_self (a : Bool) : boolAnd a a = a := by
  cases a <;> rfl

/-- `OR` is idempotent. -/
theorem boolOr_self (a : Bool) : boolOr a a = a := by
  cases a <;> rfl

/-- `XOR` with itself is false. -/
theorem boolXor_self (a : Bool) : boolXor a a = false := by
  cases a <;> rfl

/-- Double negation is the identity. -/
theorem boolNot_not (a : Bool) : boolNot (boolNot a) = a := by
  cases a <;> rfl

/-- De Morgan: negating an `AND` is the `OR` of the negations. -/
theorem boolNot_and (a b : Bool) :
    boolNot (boolAnd a b) = boolOr (boolNot a) (boolNot b) := by
  cases a <;> cases b <;> rfl

/-- De Morgan: negating an `OR` is the `AND` of the negations. -/
theorem boolNot_or (a b : Bool) :
    boolNot (boolOr a b) = boolAnd (boolNot a) (boolNot b) := by
  cases a <;> cases b <;> rfl

/-- `AND` with true is the identity. -/
theorem boolAnd_true (a : Bool) : boolAnd a true = a := by
  cases a <;> rfl

/-- `OR` with false is the identity. -/
theorem boolOr_false (a : Bool) : boolOr a false = a := by
  cases a <;> rfl

end Stc.Exact
