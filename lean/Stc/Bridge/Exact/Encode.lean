/-
Encodings between the Aeneas-generated exact model and `Stc.Exact`.

This module is hand-written, core Lean only, and part of the default `Stc`
package. It deliberately does not import Aeneas or Mathlib. The Aeneas side
uses these encodings in `lean/aeneas/Bridge/Exact/Refinement.lean`; the default
build only needs the exact-side definitions and lemmas.

The generated model represents a scalar `IntType` by a `u8` width plus a
`signed` flag, traps by a three-constructor enum, and a checked result by a
generated `Result` enum. `IntTypeEncoding`, `TrapEncoding`, and `ResEncoding`
are the decoded counterparts. `decodeBits` decodes a generated unsigned scalar
(`Std.U128`) to the mathematical bit pattern consumed by `Stc.Exact.Bits`.
-/

import Stc.Exact.Int

namespace Stc.Bridge.Exact

/-- The decoded form of the generated `exact.IntType`. -/
structure IntTypeEncoding where
  /-- Width in bits. -/
  width : Nat
  /-- `true` for signed layouts. -/
  signed : Bool
deriving Repr, DecidableEq

/-- Decode a generated-width layout to the exact model. -/
def IntTypeEncoding.toExact (e : IntTypeEncoding) : Stc.Exact.IntType :=
  { width := e.width, signed := e.signed }

/-- Encode an exact layout for comparison with the generated model. -/
def IntTypeEncoding.ofExact (ty : Stc.Exact.IntType) : IntTypeEncoding :=
  { width := ty.width, signed := ty.signed }

@[simp]
theorem IntTypeEncoding.toExact_ofExact (ty : Stc.Exact.IntType) :
    (IntTypeEncoding.ofExact ty).toExact = ty := by
  cases ty
  rfl

@[simp]
theorem IntTypeEncoding.ofExact_toExact (e : IntTypeEncoding) :
    IntTypeEncoding.ofExact e.toExact = e := by
  cases e
  rfl

/-- The decoded form of the generated `exact.Trap`. -/
inductive TrapEncoding where
  /-- Exact arithmetic overflow. -/
  | overflow
  /-- Division or remainder by zero. -/
  | divZero
  /-- A conversion result outside the destination range. -/
  | range
deriving Repr, DecidableEq

/-- Decode a generated trap to the exact model. -/
def TrapEncoding.toExact : TrapEncoding → Stc.Exact.Trap
  | .overflow => .overflow
  | .divZero => .divZero
  | .range => .range

/-- Encode an exact trap for comparison with the generated model. -/
def TrapEncoding.ofExact : Stc.Exact.Trap → TrapEncoding
  | .overflow => .overflow
  | .divZero => .divZero
  | .range => .range

@[simp]
theorem TrapEncoding.toExact_ofExact (trap : Stc.Exact.Trap) :
    (TrapEncoding.ofExact trap).toExact = trap := by
  cases trap <;> rfl

@[simp]
theorem TrapEncoding.ofExact_toExact (trap : TrapEncoding) :
    TrapEncoding.ofExact trap.toExact = trap := by
  cases trap <;> rfl

/-- The decoded form of the generated checked-result enum. -/
inductive ResEncoding where
  /-- A successful checked value. -/
  | ok (value : Int)
  /-- A trapped checked operation. -/
  | error (trap : TrapEncoding)
deriving Repr, DecidableEq

/-- Decode a checked result to the exact model. -/
def ResEncoding.toExact : ResEncoding → Stc.Exact.Res
  | .ok value => .ok value
  | .error trap => .error trap.toExact

/-- Encode an exact checked result for comparison with the generated model. -/
def ResEncoding.ofExact : Stc.Exact.Res → ResEncoding
  | .ok value => .ok value
  | .error trap => .error (TrapEncoding.ofExact trap)

@[simp]
theorem ResEncoding.toExact_ofExact (result : Stc.Exact.Res) :
    (ResEncoding.ofExact result).toExact = result := by
  cases result with
  | ok value => rfl
  | error trap => simp [ResEncoding.ofExact, ResEncoding.toExact]

@[simp]
theorem ResEncoding.ofExact_toExact (result : ResEncoding) :
    ResEncoding.ofExact result.toExact = result := by
  cases result with
  | ok value => rfl
  | error trap => simp [ResEncoding.ofExact, ResEncoding.toExact]

/-- Decode a generated unsigned scalar to a bit pattern of the given width. -/
def decodeBits (width bits : Nat) : Nat :=
  bits % 2 ^ width

/-- A decoded bit pattern is below the width's modulus. -/
theorem decodeBits_lt (width bits : Nat) : decodeBits width bits < 2 ^ width :=
  Nat.mod_lt _ (Nat.two_pow_pos width)

/-- A decoded bit pattern is below the exact layout's modulus. -/
theorem decodeBits_lt_modulus (ty : Stc.Exact.IntType) (bits : Nat) :
    decodeBits ty.width bits < ty.modulus := by
  simpa [decodeBits, Stc.Exact.IntType.modulus] using decodeBits_lt ty.width bits

/-- A bit pattern already inside the width is unchanged by decoding. -/
theorem decodeBits_of_lt {width bits : Nat} (h : bits < 2 ^ width) :
    decodeBits width bits = bits :=
  Nat.mod_eq_of_lt h

/-- Decoding a bit pattern that fits the exact layout is the identity. -/
theorem decodeBits_of_lt_modulus {ty : Stc.Exact.IntType} {bits : Nat}
    (h : bits < ty.modulus) : decodeBits ty.width bits = bits := by
  apply decodeBits_of_lt
  simpa [Stc.Exact.IntType.modulus] using h

end Stc.Bridge.Exact
