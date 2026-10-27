/-
Core HIR type and value-shape definitions (P0.2).

The types are the scalar core of `Plan-Phase0.md` §3.3: `BOOL`, signed and
unsigned integers, bit strings, and the character types. Widths and conversion
rules stay in step with `Stc.Exact`. Every type has a canonical `IntType`
layout: `BOOL` is the one-bit unsigned layout, integers carry their declared
signedness, bit strings are unsigned, and characters are unsigned code-unit or
scalar layouts.

HIR types are canonical and resolved. The surface `ElementaryType` names map
to them through `Ty.ofElementary`; the source spelling is not part of the HIR
identity.
-/
import Stc.Exact.Conv
import Stc.Syntax.Tokens

namespace Stc.Hir

open Stc.Syntax

/-- A canonical core type.

`BOOL` is one bit. `int` covers `SINT`/`INT`/`DINT`/`LINT` and
`USINT`/`UINT`/`UDINT`/`ULINT`; `bits` covers `BYTE`/`WORD`/`DWORD`/`LWORD`;
`char` covers `CHAR`/`WCHAR`/`UCHAR`. The constructors stay distinct even when
their layouts coincide, because conversions between integer, bit-string, and
character types are restricted. -/
inductive Ty where
  /-- `BOOL`: a one-bit unsigned layout. -/
  | bool
  /-- A signed or unsigned integer of the given width. -/
  | int (width : Nat) (signed : Bool)
  /-- A bit string of the given width. -/
  | bits (width : Nat)
  /-- A character layout. -/
  | char (kind : CharType)
deriving Repr, DecidableEq

namespace Ty

/-- Whether the type is `BOOL`. -/
def isBool : Ty → Bool
  | .bool => true
  | _ => false

/-- Whether the type is an integer. -/
def isInt : Ty → Bool
  | .int _ _ => true
  | _ => false

/-- Whether the type is a bit string. -/
def isBits : Ty → Bool
  | .bits _ => true
  | _ => false

/-- Whether the type is a character type. -/
def isChar : Ty → Bool
  | .char _ => true
  | _ => false

/-- Whether the type supports bitwise `AND`/`OR`/`XOR`/`NOT`: `BOOL` or a bit
string. -/
def isBitwise (ty : Ty) : Bool :=
  ty.isBool || ty.isBits

/-- Whether the type is an ordered magnitude for `MIN`, `MAX`, and `LIMIT`. -/
def isMagnitude (ty : Ty) : Bool :=
  ty.isInt || ty.isBits || ty.isChar

/-- The type's width in bits. -/
def width : Ty → Nat
  | .bool => 1
  | .int w _ => w
  | .bits w => w
  | .char .char => 8
  | .char .wchar => 16
  | .char .uchar => 32

/-- The canonical `Stc.Exact` layout of a type. `BOOL` is the one-bit unsigned
layout; `UCHAR` is a 32-bit unsigned scalar layout. -/
def layout : Ty → Exact.IntType
  | .bool => ⟨1, false⟩
  | .int w s => ⟨w, s⟩
  | .bits w => ⟨w, false⟩
  | .char .char => ⟨8, false⟩
  | .char .wchar => ⟨16, false⟩
  | .char .uchar => ⟨32, false⟩

/-- Whether `v` is representable in the type. -/
def holdsB (ty : Ty) (v : Int) : Bool :=
  ty.layout.fits v

/-- The representability relation. -/
abbrev Holds (ty : Ty) (v : Int) : Prop :=
  ty.holdsB v = true

/-- A width is one of the core widths. -/
def coreWidth (w : Nat) : Bool :=
  w == 8 || w == 16 || w == 32 || w == 64

/-- Whether the type is one of the frozen core types. Arithmetic types are
restricted to the declared core widths; `BOOL` and the character layouts are
fixed. -/
def isCore : Ty → Bool
  | .bool => true
  | .int w _ => coreWidth w
  | .bits w => coreWidth w
  | .char _ => true

/-- The core-type predicate used by declarations and casts. -/
abbrev WellFormed (ty : Ty) : Prop :=
  ty.isCore = true

/-- The canonical type named by a core elementary type. -/
def ofElementary : ElementaryType → Ty
  | .bool => .bool
  | .sint => .int 8 true
  | .int => .int 16 true
  | .dint => .int 32 true
  | .lint => .int 64 true
  | .usint => .int 8 false
  | .uint => .int 16 false
  | .udint => .int 32 false
  | .ulint => .int 64 false
  | .byte => .bits 8
  | .word => .bits 16
  | .dword => .bits 32
  | .lword => .bits 64
  | .char => .char .char
  | .wchar => .char .wchar
  | .uchar => .char .uchar

/-- The elementary type name of a core canonical type, when one exists. -/
def toElementary? : Ty → Option ElementaryType
  | .bool => some .bool
  | .int 8 true => some .sint
  | .int 16 true => some .int
  | .int 32 true => some .dint
  | .int 64 true => some .lint
  | .int 8 false => some .usint
  | .int 16 false => some .uint
  | .int 32 false => some .udint
  | .int 64 false => some .ulint
  | .bits 8 => some .byte
  | .bits 16 => some .word
  | .bits 32 => some .dword
  | .bits 64 => some .lword
  | .char .char => some .char
  | .char .wchar => some .wchar
  | .char .uchar => some .uchar
  | _ => none

/-- Every elementary type maps back to itself. -/
theorem toElementary?_ofElementary (ty : ElementaryType) :
    (ofElementary ty).toElementary? = some ty := by
  cases ty <;> rfl

/-- Every elementary type maps to a core canonical type. -/
theorem ofElementary_isCore (ty : ElementaryType) :
    (ofElementary ty).isCore = true := by
  cases ty <;> rfl

/-- A core type's layout has a positive width. -/
theorem layout_width_pos {ty : Ty} (h : ty.isCore = true) : 0 < ty.layout.width := by
  cases ty with
  | bool => simp [Ty.layout]
  | int w s =>
      have hw : Ty.coreWidth w = true := by simpa [Ty.isCore] using h
      simp [Ty.layout]
      simp [Ty.coreWidth] at hw
      omega
  | bits w =>
      have hw : Ty.coreWidth w = true := by simpa [Ty.isCore] using h
      simp [Ty.layout]
      simp [Ty.coreWidth] at hw
      omega
  | char k => cases k <;> simp [Ty.layout]

end Ty

/-- A typed HIR literal. The value is an integer payload: `BOOL` uses `0` and
`1`, characters use their scalar value, and integer and bit-string literals use
their numeric value. `Literal.WellFormed` requires the payload to be
representable in the literal's type. -/
structure Literal where
  /-- The canonical literal type. -/
  ty : Ty
  /-- The integer payload. -/
  value : Int
deriving Repr, DecidableEq

namespace Literal

/-- Whether the literal has a core type and a representable payload. -/
def wellFormedB (lit : Literal) : Bool :=
  lit.ty.isCore && lit.ty.holdsB lit.value

/-- The literal well-formedness predicate. -/
abbrev WellFormed (lit : Literal) : Prop :=
  lit.wellFormedB = true

/-- A well-formed literal has a core type. -/
theorem isCore_of_wellFormed {lit : Literal} (h : lit.WellFormed) :
    lit.ty.isCore = true := by
  have h' : (lit.ty.isCore && lit.ty.holdsB lit.value) = true := h
  exact (Bool.and_eq_true_iff.mp h').1

/-- A well-formed literal's payload is representable. -/
theorem holds_of_wellFormed {lit : Literal} (h : lit.WellFormed) :
    lit.ty.Holds lit.value := by
  have h' : (lit.ty.isCore && lit.ty.holdsB lit.value) = true := h
  exact (Bool.and_eq_true_iff.mp h').2

end Literal

/-- A resolved variable identifier. HIR has no names in typing: parameters,
locals, globals, and the function result are indexed by position. -/
inductive VarId where
  /-- A function parameter. -/
  | param (index : Nat)
  /-- A local variable. -/
  | local (index : Nat)
  /-- A global variable root. -/
  | global (index : Nat)
  /-- The current function's result variable (the function's own name). -/
  | result
deriving Repr, DecidableEq

end Stc.Hir
