/-
Tokens, token kinds, trivia, and spans for the core surface language (P0.1).

The definitions mirror [Tokenization](../../../Specification/Syntax/Tokenization.md)
for the productions listed in `Plan-Phase0.md` §3.3. The full rowan CST remains
a contract-level abstraction; the token stream and spans are the proof surface.
-/
namespace Stc.Syntax

/-- A source file identifier. Phase 0 core units are single files, but the
surface model keeps the identifier so spans stay file-relative. -/
structure FileId where
  /-- Implementation-defined file index. -/
  id : Nat
deriving Repr, DecidableEq

/-- A byte range `[start, stop)` in a source file. -/
structure Span where
  /-- The source file that contains the range. -/
  file : FileId
  /-- Inclusive byte offset of the first byte. -/
  start : Nat
  /-- Exclusive byte offset one past the last byte. -/
  stop : Nat
deriving Repr, DecidableEq

namespace Span

/-- A span is well formed when its bounds are ordered. -/
def WellFormed (s : Span) : Prop :=
  s.start ≤ s.stop

/-- The byte length of a span. -/
def length (s : Span) : Nat :=
  s.stop - s.start

/-- `outer` contains `inner` when both are in the same file and the inner
bounds lie inside the outer bounds. -/
def contains (outer inner : Span) : Prop :=
  outer.file = inner.file ∧ outer.start ≤ inner.start ∧ inner.stop ≤ outer.stop

/-- The smallest span covering both arguments. In the (unreachable for a
well-formed CST) cross-file case it conservatively returns the first span. -/
def union (a b : Span) : Span :=
  if a.file = b.file then
    { file := a.file, start := min a.start b.start, stop := max a.stop b.stop }
  else
    a

/-- Containment is reflexive. -/
theorem contains_refl (s : Span) : s.contains s :=
  ⟨rfl, Nat.le_refl _, Nat.le_refl _⟩

/-- Containment is transitive. -/
theorem contains_trans {a b c : Span} (hab : a.contains b) (hbc : b.contains c) :
    a.contains c :=
  ⟨hab.1.trans hbc.1, Nat.le_trans hab.2.1 hbc.2.1, Nat.le_trans hbc.2.2 hab.2.2⟩

/-- A containing span contains the union of two spans it contains. -/
theorem contains_union {outer a b : Span} (ha : outer.contains a) (hb : outer.contains b) :
    outer.contains (a.union b) := by
  unfold Span.union
  split
  · unfold Span.contains at *
    refine ⟨ha.1, ?_, ?_⟩
    · exact Nat.le_min.mpr ⟨ha.2.1, hb.2.1⟩
    · exact Nat.max_le.mpr ⟨ha.2.2, hb.2.2⟩
  · exact ha

/-- The union of two well-formed spans is well formed. -/
theorem union_wellFormed {a b : Span} (ha : a.WellFormed) :
    (a.union b).WellFormed := by
  unfold Span.union
  split
  · unfold Span.WellFormed at *
    exact Nat.le_trans (Nat.min_le_left _ _) (Nat.le_trans ha (Nat.le_max_left _ _))
  · exact ha

end Span

/-- A character layout: `CHAR` and `WCHAR` are 8- and 16-bit code units,
`UCHAR` is a 32-bit Unicode scalar value. -/
inductive CharType where
  | char
  | wchar
  | uchar
deriving Repr, DecidableEq

/-- The elementary type names in the Phase 0 core. The surface model keeps the
spelling; widths and signedness are fixed in P0.2 against `Stc.Exact`. -/
inductive ElementaryType where
  | bool
  | sint
  | int
  | dint
  | lint
  | usint
  | uint
  | udint
  | ulint
  | byte
  | word
  | dword
  | lword
  | char
  | wchar
  | uchar
deriving Repr, DecidableEq

namespace ElementaryType

/-- The character layout of a character type name. -/
def charType? : ElementaryType → Option CharType
  | .char => some .char
  | .wchar => some .wchar
  | .uchar => some .uchar
  | _ => none

/-- Whether the type name is a character type. -/
def isChar (ty : ElementaryType) : Bool :=
  ty.charType?.isSome

/-- Whether the type name is an integer or bit-string type. -/
def isNumeric (ty : ElementaryType) : Bool :=
  match ty with
  | .bool => false
  | .char | .wchar | .uchar => false
  | _ => true

end ElementaryType

/-- Core keywords. Standard function names such as `ABS` and `SEL` are
predeclared identifiers, not keywords. -/
inductive Keyword where
  | kFunction
  | kEndFunction
  | kProgram
  | kEndProgram
  | kVar
  | kVarInput
  | kVarGlobal
  | kEndVar
  | kIf
  | kThen
  | kElsif
  | kElse
  | kEndIf
  | kCase
  | kOf
  | kEndCase
  | kFor
  | kTo
  | kBy
  | kDo
  | kEndFor
  | kWhile
  | kEndWhile
  | kRepeat
  | kUntil
  | kEndRepeat
  | kExit
  | kContinue
  | kReturn
  | kTrue
  | kFalse
  | kAnd
  | kOr
  | kXor
  | kNot
  | kMod
  | kBool
  | kSint
  | kInt
  | kDint
  | kLint
  | kUsint
  | kUint
  | kUdint
  | kUlint
  | kByte
  | kWord
  | kDword
  | kLword
  | kChar
  | kWchar
  | kUchar
deriving Repr, DecidableEq

namespace ElementaryType

/-- The elementary type named by a core keyword. -/
def ofKeyword? : Keyword → Option ElementaryType
  | .kBool => some .bool
  | .kSint => some .sint
  | .kInt => some .int
  | .kDint => some .dint
  | .kLint => some .lint
  | .kUsint => some .usint
  | .kUint => some .uint
  | .kUdint => some .udint
  | .kUlint => some .ulint
  | .kByte => some .byte
  | .kWord => some .word
  | .kDword => some .dword
  | .kLword => some .lword
  | .kChar => some .char
  | .kWchar => some .wchar
  | .kUchar => some .uchar
  | _ => none

end ElementaryType

/-- Core punctuation tokens. -/
inductive Punct where
  | lparen
  | rparen
  | semicolon
  | comma
  | colon
  | assign
  | range
  | plus
  | minus
  | star
  | slash
  | power
  | equal
  | notEqual
  | less
  | greater
  | lessEqual
  | greaterEqual
  | ampersand
deriving Repr, DecidableEq

/-- A lexical token kind. Literal payloads are decoded by the lexer; the AST
uses `Literal.ofToken?` to recover them. -/
inductive TokenKind where
  /-- An identifier with its original spelling (case-insensitive lookup). -/
  | ident (text : String)
  /-- A reserved keyword. -/
  | keyword (value : Keyword)
  /-- A decimal integer literal value. -/
  | intLit (value : Int)
  /-- A based literal (`2#...`, `8#...`, `16#...`) as a bit pattern. -/
  | basedLit (value : Nat)
  /-- A typed integer or bit-string literal. -/
  | typedInt (ty : ElementaryType) (value : Int)
  /-- A typed boolean literal (`BOOL#TRUE`, `BOOL#1`). -/
  | typedBool (value : Bool)
  /-- A typed character literal with its decoded scalar value. -/
  | charLit (ty : CharType) (scalar : Nat)
  /-- A punctuation token. -/
  | punct (value : Punct)
  /-- The end-of-file marker. -/
  | eof
  /-- A lexical error token produced during recovery. -/
  | error
deriving Repr, DecidableEq

/-- A token with its source span. The token text is `source[span]`; it is not
stored twice. -/
structure Token where
  kind : TokenKind
  span : Span
deriving Repr, DecidableEq

namespace Token

/-- Whether the token is the end-of-file marker. -/
def isEof (t : Token) : Bool :=
  match t.kind with
  | .eof => true
  | _ => false

end Token

/-- Trivia kinds. Comments and whitespace are preserved in the lossless CST. -/
inductive TriviaKind where
  | whitespace
  | lineComment
  | blockComment
  | docComment
  | bom
deriving Repr, DecidableEq

/-- A trivia item with its source span. -/
structure Trivia where
  kind : TriviaKind
  span : Span
deriving Repr, DecidableEq

/-- An identifier occurrence with its original spelling and span. -/
structure Name where
  text : String
  span : Span
deriving Repr, DecidableEq

namespace Name

/-- The normalized lookup key: ASCII uppercase. Identifiers are ASCII-only in
the profile, so this matches the lexer's normalized key. -/
def key (n : Name) : String :=
  n.text.toUpper

/-- Recover a name from an identifier token. -/
def ofToken? (t : Token) : Option Name :=
  match t.kind with
  | .ident text => some { text := text, span := t.span }
  | _ => none

/-- A recovered name carries the token's span. -/
theorem ofToken?_span {t : Token} {n : Name} (h : ofToken? t = some n) :
    n.span = t.span := by
  cases t with
  | mk kind span =>
      cases kind <;> simp [ofToken?] at h
      all_goals (rw [← h])

end Name

end Stc.Syntax
