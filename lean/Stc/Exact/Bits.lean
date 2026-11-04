/-
Exact bitwise, shift, and rotate model over `Nat` bit patterns.
-/
import Stc.Exact.Int

namespace Stc.Exact

/-- The bit mask with every bit of the type set. -/
def IntType.mask (ty : IntType) : Nat :=
  2 ^ ty.width - 1

/-- The masked shift/rotate amount: the low `width` bits of `n`. -/
def IntType.count (ty : IntType) (n : Int) : Nat :=
  (n.emod (ty.width : Int)).toNat

/-- Bitwise AND on the low `width` bits. -/
def IntType.bitAnd (ty : IntType) (a b : Nat) : Nat :=
  (a % ty.modulus) &&& (b % ty.modulus)

/-- Bitwise OR on the low `width` bits. -/
def IntType.bitOr (ty : IntType) (a b : Nat) : Nat :=
  (a % ty.modulus) ||| (b % ty.modulus)

/-- Bitwise XOR on the low `width` bits. -/
def IntType.bitXor (ty : IntType) (a b : Nat) : Nat :=
  (a % ty.modulus) ^^^ (b % ty.modulus)

/-- Bitwise NOT on the low `width` bits. -/
def IntType.bitNot (ty : IntType) (a : Nat) : Nat :=
  (a % ty.modulus) ^^^ ty.mask

/-- Logical left shift by `n mod width`. -/
def IntType.shl (ty : IntType) (a : Nat) (n : Int) : Nat :=
  ((a % ty.modulus) <<< ty.count n) % ty.modulus

/-- Logical right shift by `n mod width`. -/
def IntType.shr (ty : IntType) (a : Nat) (n : Int) : Nat :=
  (a % ty.modulus) >>> ty.count n

/-- Rotate left by `n mod width`. -/
def IntType.rol (ty : IntType) (a : Nat) (n : Int) : Nat :=
  let r := ty.count n
  let x := a % ty.modulus
  ((x <<< r) ||| (x >>> ((ty.width - r) % ty.width))) % ty.modulus

/-- Rotate right by `n mod width`. -/
def IntType.ror (ty : IntType) (a : Nat) (n : Int) : Nat :=
  let r := ty.count n
  let x := a % ty.modulus
  ((x >>> r) ||| (x <<< ((ty.width - r) % ty.width))) % ty.modulus

/-- The type's modulus is positive. -/
theorem IntType.modulus_pos (ty : IntType) : 0 < ty.modulus :=
  Nat.two_pow_pos ty.width

/-- The mask is below the modulus. -/
theorem IntType.mask_lt (ty : IntType) : ty.mask < ty.modulus := by
  unfold IntType.mask IntType.modulus
  exact Nat.sub_one_lt_of_lt (Nat.two_pow_pos ty.width)

/-- A shift amount is zero for a zero count. -/
theorem IntType.count_zero (ty : IntType) (hw : 0 < ty.width) :
    ty.count 0 = 0 := by
  unfold IntType.count
  have h : (0 : Int).emod (ty.width : Int) = 0 := by
    apply Int.emod_eq_of_lt (by omega)
    exact_mod_cast hw
  rw [h]
  rfl

/-- A shift amount of the width is zero. -/
theorem IntType.count_width (ty : IntType) :
    ty.count (ty.width : Int) = 0 := by
  unfold IntType.count
  have h : (ty.width : Int).emod (ty.width : Int) = 0 := Int.emod_self
  rw [h]
  rfl

/-- A shift amount is below the width. -/
theorem IntType.count_lt (ty : IntType) (n : Int) (hw : 0 < ty.width) :
    ty.count n < ty.width := by
  unfold IntType.count
  have hpos : (0 : Int) < (ty.width : Int) := by exact_mod_cast hw
  have hlt : n.emod (ty.width : Int) < (ty.width : Int) :=
    Int.emod_lt_of_pos n hpos
  have hnonneg : 0 ≤ n.emod (ty.width : Int) :=
    Int.emod_nonneg n (by omega)
  exact (Int.toNat_lt hnonneg).mpr hlt

/-- Bitwise AND stays in range. -/
theorem IntType.bitAnd_lt (ty : IntType) (a b : Nat) :
    ty.bitAnd a b < ty.modulus := by
  have hb : b % ty.modulus < 2 ^ ty.width := by
    simpa [IntType.modulus] using Nat.mod_lt b ty.modulus_pos
  have h := Nat.and_lt_two_pow (a % ty.modulus) hb
  simpa [IntType.bitAnd, IntType.modulus] using h

/-- Bitwise OR stays in range. -/
theorem IntType.bitOr_lt (ty : IntType) (a b : Nat) :
    ty.bitOr a b < ty.modulus := by
  have ha : a % ty.modulus < 2 ^ ty.width := by
    simpa [IntType.modulus] using Nat.mod_lt a ty.modulus_pos
  have hb : b % ty.modulus < 2 ^ ty.width := by
    simpa [IntType.modulus] using Nat.mod_lt b ty.modulus_pos
  have h := Nat.or_lt_two_pow ha hb
  simpa [IntType.bitOr, IntType.modulus] using h

/-- Bitwise XOR stays in range. -/
theorem IntType.bitXor_lt (ty : IntType) (a b : Nat) :
    ty.bitXor a b < ty.modulus := by
  have ha : a % ty.modulus < 2 ^ ty.width := by
    simpa [IntType.modulus] using Nat.mod_lt a ty.modulus_pos
  have hb : b % ty.modulus < 2 ^ ty.width := by
    simpa [IntType.modulus] using Nat.mod_lt b ty.modulus_pos
  have h := Nat.xor_lt_two_pow ha hb
  simpa [IntType.bitXor, IntType.modulus] using h

/-- Bitwise NOT stays in range. -/
theorem IntType.bitNot_lt (ty : IntType) (a : Nat) :
    ty.bitNot a < ty.modulus := by
  have ha : a % ty.modulus < 2 ^ ty.width := by
    simpa [IntType.modulus] using Nat.mod_lt a ty.modulus_pos
  have hm : ty.mask < 2 ^ ty.width := by
    simpa [IntType.modulus] using ty.mask_lt
  have h := Nat.xor_lt_two_pow ha hm
  simpa [IntType.bitNot, IntType.modulus] using h

/-- AND is commutative. -/
theorem IntType.bitAnd_comm (ty : IntType) (a b : Nat) :
    ty.bitAnd a b = ty.bitAnd b a := by
  simp [IntType.bitAnd, Nat.and_comm]

/-- OR is commutative. -/
theorem IntType.bitOr_comm (ty : IntType) (a b : Nat) :
    ty.bitOr a b = ty.bitOr b a := by
  simp [IntType.bitOr, Nat.or_comm]

/-- XOR is commutative. -/
theorem IntType.bitXor_comm (ty : IntType) (a b : Nat) :
    ty.bitXor a b = ty.bitXor b a := by
  simp [IntType.bitXor, Nat.xor_comm]

/-- XOR is associative. -/
theorem IntType.bitXor_assoc (ty : IntType) (a b c : Nat) :
    ty.bitXor (ty.bitXor a b) c = ty.bitXor a (ty.bitXor b c) := by
  have ha : a % ty.modulus < ty.modulus := Nat.mod_lt _ ty.modulus_pos
  have hb : b % ty.modulus < ty.modulus := Nat.mod_lt _ ty.modulus_pos
  have hc : c % ty.modulus < ty.modulus := Nat.mod_lt _ ty.modulus_pos
  have h1 : (a % ty.modulus ^^^ b % ty.modulus) < ty.modulus :=
    Nat.xor_lt_two_pow
      (by simpa [IntType.modulus] using ha)
      (by simpa [IntType.modulus] using hb)
  have h2 : (b % ty.modulus ^^^ c % ty.modulus) < ty.modulus :=
    Nat.xor_lt_two_pow
      (by simpa [IntType.modulus] using hb)
      (by simpa [IntType.modulus] using hc)
  simp only [IntType.bitXor]
  rw [Nat.mod_eq_of_lt h1, Nat.mod_eq_of_lt h2]
  exact Nat.xor_assoc _ _ _

/-- Double bitwise negation is the identity on the type's value. -/
theorem IntType.bitNot_involutive (ty : IntType) (a : Nat) :
    ty.bitNot (ty.bitNot a) = a % ty.modulus := by
  have ha : a % ty.modulus < ty.modulus := Nat.mod_lt _ ty.modulus_pos
  have hm : ty.mask < ty.modulus := ty.mask_lt
  have h1 : (a % ty.modulus ^^^ ty.mask) < ty.modulus :=
    Nat.xor_lt_two_pow ha hm
  unfold IntType.bitNot
  rw [Nat.mod_eq_of_lt h1]
  rw [Nat.xor_assoc, Nat.xor_self, Nat.xor_zero]

/-- Left shift stays in range. -/
theorem IntType.shl_lt (ty : IntType) (a : Nat) (n : Int) :
    ty.shl a n < ty.modulus := by
  unfold IntType.shl
  exact Nat.mod_lt _ ty.modulus_pos

/-- Logical right shift stays in range. -/
theorem IntType.shr_lt (ty : IntType) (a : Nat) (n : Int) :
    ty.shr a n < ty.modulus := by
  unfold IntType.shr
  rw [Nat.shiftRight_eq_div_pow]
  exact Nat.lt_of_le_of_lt (Nat.div_le_self _ _) (Nat.mod_lt _ ty.modulus_pos)

/-- Rotate left stays in range. -/
theorem IntType.rol_lt (ty : IntType) (a : Nat) (n : Int) :
    ty.rol a n < ty.modulus := by
  unfold IntType.rol
  exact Nat.mod_lt _ ty.modulus_pos

/-- Rotate right stays in range. -/
theorem IntType.ror_lt (ty : IntType) (a : Nat) (n : Int) :
    ty.ror a n < ty.modulus := by
  unfold IntType.ror
  exact Nat.mod_lt _ ty.modulus_pos

/-- A left shift is multiplication by the corresponding power of two. -/
theorem IntType.shl_eq_mul (ty : IntType) (a : Nat) (n : Int) :
    ty.shl a n = (a % ty.modulus) * 2 ^ ty.count n % ty.modulus := by
  simp [IntType.shl, Nat.shiftLeft_eq]

/-- A logical right shift is division by the corresponding power of two. -/
theorem IntType.shr_eq_div (ty : IntType) (a : Nat) (n : Int) :
    ty.shr a n = (a % ty.modulus) / 2 ^ ty.count n := by
  simp [IntType.shr, Nat.shiftRight_eq_div_pow]

/-- Rotating by zero is the identity on the value. -/
theorem IntType.rol_zero (ty : IntType) (a : Nat) (hw : 0 < ty.width) :
    ty.rol a 0 = a % ty.modulus := by
  unfold IntType.rol
  rw [ty.count_zero hw]
  have hx : a % ty.modulus < ty.modulus := Nat.mod_lt _ ty.modulus_pos
  simp only [Nat.shiftLeft_zero, Nat.sub_zero, Nat.mod_self, Nat.shiftRight_zero,
    Nat.or_self]
  exact Nat.mod_eq_of_lt hx

/-- Rotating right by zero is the identity on the value. -/
theorem IntType.ror_zero (ty : IntType) (a : Nat) (hw : 0 < ty.width) :
    ty.ror a 0 = a % ty.modulus := by
  unfold IntType.ror
  rw [ty.count_zero hw]
  have hx : a % ty.modulus < ty.modulus := Nat.mod_lt _ ty.modulus_pos
  simp only [Nat.shiftRight_zero, Nat.sub_zero, Nat.mod_self, Nat.shiftLeft_zero,
    Nat.or_self]
  exact Nat.mod_eq_of_lt hx

/-- Rotating by the width is the identity on the value. -/
theorem IntType.rol_width (ty : IntType) (a : Nat) :
    ty.rol a (ty.width : Int) = a % ty.modulus := by
  unfold IntType.rol
  rw [ty.count_width]
  have hx : a % ty.modulus < ty.modulus := Nat.mod_lt _ ty.modulus_pos
  simp only [Nat.shiftLeft_zero, Nat.sub_zero, Nat.mod_self, Nat.shiftRight_zero,
    Nat.or_self]
  exact Nat.mod_eq_of_lt hx

/-- Rotating right by the width is the identity on the value. -/
theorem IntType.ror_width (ty : IntType) (a : Nat) :
    ty.ror a (ty.width : Int) = a % ty.modulus := by
  unfold IntType.ror
  rw [ty.count_width]
  have hx : a % ty.modulus < ty.modulus := Nat.mod_lt _ ty.modulus_pos
  simp only [Nat.shiftRight_zero, Nat.sub_zero, Nat.mod_self, Nat.shiftLeft_zero,
    Nat.or_self]
  exact Nat.mod_eq_of_lt hx

/-- A shift whose masked count is zero is the identity on the value. -/
theorem IntType.rol_count_zero (ty : IntType) (a : Nat) {n : Int}
    (h : ty.count n = 0) : ty.rol a n = a % ty.modulus := by
  unfold IntType.rol
  rw [h]
  have hx : a % ty.modulus < ty.modulus := Nat.mod_lt _ ty.modulus_pos
  simp only [Nat.shiftLeft_zero, Nat.sub_zero, Nat.mod_self, Nat.shiftRight_zero,
    Nat.or_self]
  exact Nat.mod_eq_of_lt hx

/-- A right shift whose masked count is zero is the identity on the value. -/
theorem IntType.ror_count_zero (ty : IntType) (a : Nat) {n : Int}
    (h : ty.count n = 0) : ty.ror a n = a % ty.modulus := by
  unfold IntType.ror
  rw [h]
  have hx : a % ty.modulus < ty.modulus := Nat.mod_lt _ ty.modulus_pos
  simp only [Nat.shiftRight_zero, Nat.sub_zero, Nat.mod_self, Nat.shiftLeft_zero,
    Nat.or_self]
  exact Nat.mod_eq_of_lt hx

/-- A left shift by a zero count is the identity on the value. -/
theorem IntType.shl_zero (ty : IntType) (a : Nat) (hw : 0 < ty.width) :
    ty.shl a 0 = a % ty.modulus := by
  unfold IntType.shl
  rw [ty.count_zero hw]
  simp [Nat.mod_eq_of_lt (Nat.mod_lt _ ty.modulus_pos)]

/-- A logical right shift by a zero count is the identity on the value. -/
theorem IntType.shr_zero (ty : IntType) (a : Nat) (hw : 0 < ty.width) :
    ty.shr a 0 = a % ty.modulus := by
  unfold IntType.shr
  rw [ty.count_zero hw]
  simp

/-- A left shift by the width is the identity on the value. -/
theorem IntType.shl_width (ty : IntType) (a : Nat) :
    ty.shl a (ty.width : Int) = a % ty.modulus := by
  unfold IntType.shl
  rw [ty.count_width]
  simp [Nat.mod_eq_of_lt (Nat.mod_lt _ ty.modulus_pos)]

/-- A logical right shift by the width is the identity on the value. -/
theorem IntType.shr_width (ty : IntType) (a : Nat) :
    ty.shr a (ty.width : Int) = a % ty.modulus := by
  unfold IntType.shr
  rw [ty.count_width]
  simp

/-- AND with a value is the value on the type's bit pattern. -/
theorem IntType.bitAnd_self (ty : IntType) (a : Nat) :
    ty.bitAnd a a = a % ty.modulus := by
  simp [IntType.bitAnd, Nat.and_self]

/-- OR with a value is the value on the type's bit pattern. -/
theorem IntType.bitOr_self (ty : IntType) (a : Nat) :
    ty.bitOr a a = a % ty.modulus := by
  simp [IntType.bitOr, Nat.or_self]

/-- XOR with a value is zero. -/
theorem IntType.bitXor_self (ty : IntType) (a : Nat) : ty.bitXor a a = 0 := by
  simp [IntType.bitXor, Nat.xor_self]

/-- AND with zero is zero. -/
theorem IntType.bitAnd_zero (ty : IntType) (a : Nat) : ty.bitAnd a 0 = 0 := by
  simp [IntType.bitAnd]

/-- OR with zero keeps the value. -/
theorem IntType.bitOr_zero (ty : IntType) (a : Nat) :
    ty.bitOr a 0 = a % ty.modulus := by
  simp [IntType.bitOr]

/-- XOR with zero keeps the value. -/
theorem IntType.bitXor_zero (ty : IntType) (a : Nat) :
    ty.bitXor a 0 = a % ty.modulus := by
  simp [IntType.bitXor]

/-- Complementing zero gives the mask. -/
theorem IntType.bitNot_zero (ty : IntType) : ty.bitNot 0 = ty.mask := by
  simp [IntType.bitNot]

/-- Complementing the mask gives zero. -/
theorem IntType.bitNot_mask (ty : IntType) : ty.bitNot ty.mask = 0 := by
  simp [IntType.bitNot, Nat.mod_eq_of_lt ty.mask_lt, Nat.xor_self]

end Stc.Exact
