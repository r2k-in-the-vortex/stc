/-
Exact string-length model for `STRING` (SBCS), `WSTRING` (DBCS), and
`USTRING` (Unicode scalars), following the UTF-8 extension.
-/
import Stc.Exact.Int

namespace Stc.Exact

/-- The storage encoding of a string type. -/
inductive StringKind where
  /-- `STRING`: one byte per character (`SBCS`). -/
  | sbcs
  /-- `WSTRING`: one UTF-16 code unit per unit (`DBCS`). -/
  | dbcs
  /-- `USTRING`: one Unicode scalar per character. -/
  | unicode
deriving Repr, DecidableEq

/-- The number of UTF-8 bytes used by one Unicode scalar value. -/
def utf8Length (scalar : Nat) : Nat :=
  if scalar ≤ 0x7F then 1
  else if scalar ≤ 0x7FF then 2
  else if scalar ≤ 0xFFFF then 3
  else 4

/-- The number of UTF-8 bytes used by a scalar sequence. -/
def utf8ByteLength : List Nat → Nat
  | [] => 0
  | scalar :: rest => utf8Length scalar + utf8ByteLength rest

/-- The number of UTF-16 code units used by one scalar. -/
def utf16Units (scalar : Nat) : Nat :=
  if scalar ≤ 0xFFFF then 1 else 2

/-- The number of UTF-16 code units used by a scalar sequence. -/
def utf16Length : List Nat → Nat
  | [] => 0
  | scalar :: rest => utf16Units scalar + utf16Length rest

/-- `LEN`: the stored character/unit count. -/
def len (kind : StringKind) (scalars : List Nat) : Nat :=
  match kind with
  | .sbcs => scalars.length
  | .dbcs => utf16Length scalars
  | .unicode => scalars.length

/-- `LEN_MAX`: the declared capacity. -/
def lenMax (capacity : Nat) : Nat := capacity

/-- `LEN_CODE_UNIT`: UTF-8 code units for `USTRING`; the stored unit count for
the 2013 encodings. -/
def lenCodeUnit (kind : StringKind) (scalars : List Nat) : Nat :=
  match kind with
  | .sbcs => scalars.length
  | .dbcs => utf16Length scalars
  | .unicode => utf8ByteLength scalars

/-- A UTF-8 scalar uses between one and four bytes. -/
theorem utf8Length_range (scalar : Nat) :
    1 ≤ utf8Length scalar ∧ utf8Length scalar ≤ 4 := by
  unfold utf8Length
  split
  · omega
  · split
    · omega
    · split <;> omega

/-- `LEN_MAX` returns its capacity. -/
theorem lenMax_eq (capacity : Nat) : lenMax capacity = capacity := rfl

/-- A Unicode string's character count never exceeds its UTF-8 byte count. -/
theorem unicode_len_le_code_unit (scalars : List Nat) :
    len .unicode scalars ≤ lenCodeUnit .unicode scalars := by
  induction scalars with
  | nil => simp [len, lenCodeUnit, utf8ByteLength]
  | cons scalar rest ih =>
    have h := (utf8Length_range scalar).1
    have ih' : rest.length ≤ utf8ByteLength rest := ih
    simp only [len, lenCodeUnit, utf8ByteLength, List.length_cons]
    simpa [Nat.add_comm] using Nat.add_le_add ih' h

/-- A DBCS string's code-unit count equals its length. -/
theorem dbcs_len_eq_code_unit (scalars : List Nat) :
    len .dbcs scalars = lenCodeUnit .dbcs scalars := rfl

/-- An SBCS string's code-unit count equals its length. -/
theorem sbcs_len_eq_code_unit (scalars : List Nat) :
    len .sbcs scalars = lenCodeUnit .sbcs scalars := rfl

/-- ASCII scalars each use one UTF-8 byte. -/
theorem utf8_ascii (scalar : Nat) (h : scalar ≤ 0x7F) : utf8Length scalar = 1 := by
  simp [utf8Length, h]

/-- The UTF-16 length is between the scalar count and twice it. -/
theorem utf16Length_bounds (scalars : List Nat) :
    scalars.length ≤ utf16Length scalars ∧ utf16Length scalars ≤ 2 * scalars.length := by
  induction scalars with
  | nil => simp [utf16Length]
  | cons scalar rest ih =>
    have ih' := ih
    simp only [utf16Length, List.length_cons]
    by_cases hsmall : scalar ≤ 0xFFFF
    · have hu : utf16Units scalar = 1 := by simp [utf16Units, hsmall]
      rw [hu]
      omega
    · have hu : utf16Units scalar = 2 := by simp [utf16Units, hsmall]
      rw [hu]
      omega

end Stc.Exact
