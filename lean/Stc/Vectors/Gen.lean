/-
Deterministic exact-function vector generator (Testing and Verification §5.2–§5.3).

Every vector file records the format version, the Lean model version, and an
FNV-1a 64 digest of the generated cases. The Rust harness recomputes the digest
from the parsed cases, so a hand-edited or stale vector fails the tests.
-/
import Stc.Exact.Int
import Stc.Exact.Bits
import Stc.Exact.Conv
import Stc.Exact.Select
import Stc.Exact.Bool
import Stc.Exact.Strings

namespace Stc.Vectors

open Stc.Exact

/-- The vector format version (§5.3). -/
def formatVersion : Nat := 1

/-- The Lean model version. Bump this when a model's semantics change. -/
def modelVersion : Nat := 1

/-- A minimal ordered JSON tree. -/
inductive J where
  /-- A signed integer. -/
  | int : Int → J
  /-- A natural number (unsigned values and lengths). -/
  | nat : Nat → J
  /-- A string. -/
  | str : String → J
  /-- A boolean. -/
  | bool : Bool → J
  /-- An array. -/
  | arr : List J → J
deriving Repr

/-- One vector case: ordered keys and their values. -/
abbrev Case := List (String × J)

/-- One vector file. -/
structure Vector where
  /-- The exact function name. -/
  name : String
  /-- The generated cases. -/
  cases : List Case

private def hexDigit (n : Nat) : Char :=
  if n < 10 then Char.ofNat (48 + n) else Char.ofNat (87 + n)

private def hex (digits : Nat) (n : Nat) : String :=
  let rendered := String.ofList (Nat.toDigits 16 n)
  let padded := String.ofList (List.replicate (digits - rendered.length) '0')
  padded ++ rendered

private def escapeString (s : String) : String :=
  String.join (s.toList.map fun c =>
    if c == '"' then "\\\""
    else if c == '\\' then "\\\\"
    else if c == '\n' then "\\n"
    else if c == '\r' then "\\r"
    else if c == '\t' then "\\t"
    else if c.toNat < 0x20 then "\\u" ++ hex 4 c.toNat
    else String.singleton c)

/-- Renders a compact JSON value. -/
def render (value : J) : String :=
  match value with
  | .int n => toString n
  | .nat n => toString n
  | .str s => "\"" ++ escapeString s ++ "\""
  | .bool b => if b then "true" else "false"
  | .arr values => "[" ++ String.intercalate "," (values.map render) ++ "]"

/-- Renders a case as a JSON object with spaces after the separators. -/
def renderCase (fields : Case) : String :=
  "{" ++ String.intercalate ", "
    (fields.map fun (key, value) => "\"" ++ key ++ "\": " ++ render value) ++ "}"

/-- The canonical bytes hashed for the generator field. -/
def canonical (name : String) (cases : List Case) : String :=
  name ++ "\n" ++ String.join (cases.map fun fields =>
    String.intercalate "," (fields.map fun (key, value) => key ++ "=" ++ render value)
      ++ "\n")

/-- FNV-1a 64 over the canonical case text. -/
def generatorHash (name : String) (cases : List Case) : Nat :=
  let bytes := (canonical name cases).toUTF8.toList
  bytes.foldl (fun h b => ((h ^^^ b.toNat) * 0x100000001b3) %
    0x10000000000000000) 0xcbf29ce484222325

private def renderCases (cases : List Case) : String :=
  match cases with
  | [] => ""
  | [single] => "    " ++ renderCase single
  | single :: rest => "    " ++ renderCase single ++ ",\n" ++ renderCases rest

/-- Renders one complete vector file. -/
def renderVector (vector : Vector) : String :=
  let hash := generatorHash vector.name vector.cases
  "{\n" ++
    "  \"function\": \"" ++ vector.name ++ "\",\n" ++
    "  \"version\": " ++ toString formatVersion ++ ",\n" ++
    "  \"model_version\": " ++ toString modelVersion ++ ",\n" ++
    "  \"generator\": \"0x" ++ hex 16 hash ++ "\",\n" ++
    "  \"cases\": [\n" ++ renderCases vector.cases ++ "\n  ]\n}\n"

/-- The file name for a vector. -/
def vectorFile (vector : Vector) : String :=
  vector.name.toLower ++ ".json"

/-! ## Types and result encoding -/

/-- Resolves a type spelling to its exact layout (`BOOL` is a 1-bit unsigned). -/
def typeLayout : String → IntType
  | "BOOL" => ⟨1, false⟩
  | "SINT" => ⟨8, true⟩
  | "INT" => ⟨16, true⟩
  | "DINT" => ⟨32, true⟩
  | "LINT" => ⟨64, true⟩
  | "USINT" => ⟨8, false⟩
  | "UINT" => ⟨16, false⟩
  | "UDINT" => ⟨32, false⟩
  | "ULINT" => ⟨64, false⟩
  | "BYTE" => ⟨8, false⟩
  | "WORD" => ⟨16, false⟩
  | "DWORD" => ⟨32, false⟩
  | "LWORD" => ⟨64, false⟩
  | "CHAR" => ⟨8, false⟩
  | "WCHAR" => ⟨16, false⟩
  | "UCHAR" => ⟨32, false⟩
  | _ => ⟨8, false⟩

def signedTypes : List String := ["SINT", "INT", "DINT", "LINT"]
def unsignedTypes : List String := ["USINT", "UINT", "UDINT", "ULINT"]
def bitTypes : List String := ["BYTE", "WORD", "DWORD", "LWORD"]
def intTypes : List String := signedTypes ++ unsignedTypes
def compareTypes : List String := intTypes ++ bitTypes ++ ["BOOL"]
def selectTypes : List String := ["DINT", "LINT", "UDINT", "ULINT", "DWORD", "LWORD"]
def muxTypes : List String := ["DINT", "UDINT", "LWORD"]
def convTypes : List String :=
  ["BOOL"] ++ intTypes ++ bitTypes ++ ["CHAR", "WCHAR", "UCHAR"]

/-- Encodes a checked result; traps become their vector spellings. -/
def resultJson (result : Res) : J :=
  match result with
  | .ok value => .int value
  | .error .overflow => .str "overflow"
  | .error .divZero => .str "div_zero"
  | .error .range => .str "range"

private def typeField (ty : String) : (String × J) := ("type", .str ty)

private def binaryCase (ty : String) (a b : Int) (result : Res) : Case :=
  [("a", .int a), ("b", .int b), ("result", resultJson result), typeField ty]

private def unaryCase (ty : String) (a : Int) (result : Res) : Case :=
  [("a", .int a), ("result", resultJson result), typeField ty]

/-! ## Case selection -/

private def signedValues (ty : IntType) : List Int :=
  [ty.minimum, ty.minimum + 1, -101, -7, -1, 0, 1, 7, 100,
    ty.maximum - 1, ty.maximum]

private def unsignedValues (ty : IntType) : List Int :=
  [0, 1, 2, 7, 100, 128, 255, ty.maximum / 2, ty.maximum - 1, ty.maximum]

private def valuesOf (ty : IntType) : List Int :=
  if ty.signed then signedValues ty else unsignedValues ty

private def addPairs (ty : IntType) : List (Int × Int) :=
  if ty.signed then
    [(ty.maximum, 1), (ty.minimum, -1), (ty.maximum, -1), (ty.maximum, 0),
      (ty.minimum, 0), (ty.maximum, ty.maximum), (ty.minimum, ty.minimum),
      (0, 0), (7, 3), (-7, 3)]
  else
    [(ty.maximum, 1), (0, 1), (ty.maximum, 0), (ty.maximum, ty.maximum),
      (0, 0), (7, 3), (1, ty.maximum), (ty.maximum / 2, 2)]

private def subPairs (ty : IntType) : List (Int × Int) :=
  if ty.signed then
    [(ty.minimum, 1), (ty.maximum, -1), (ty.maximum, 0), (ty.minimum, 0),
      (0, 1), (0, 0), (7, 3), (-7, -3), (ty.minimum, ty.minimum)]
  else
    [(0, 1), (ty.maximum, 0), (0, 0), (7, 3), (ty.maximum, ty.maximum),
      (ty.maximum / 2, 2)]

private def mulPairs (ty : IntType) : List (Int × Int) :=
  if ty.signed then
    [(ty.maximum, 2), (ty.minimum, -1), (ty.maximum, 0), (ty.minimum, 0),
      (-1, ty.maximum), (2, 3), (-7, 3), (0, 0), (ty.minimum, 1)]
  else
    [(ty.maximum, 2), (ty.maximum, 0), (1, ty.maximum), (2, 3),
      (ty.maximum / 2, 2), (0, 0), (7, 7)]

private def divPairs (ty : IntType) : List (Int × Int) :=
  if ty.signed then
    [(7, 3), (-7, 3), (7, -3), (-7, -3), (ty.minimum, -1), (7, 0),
      (ty.maximum, 1), (ty.minimum, 1), (0, 5)]
  else
    [(7, 3), (ty.maximum, 1), (7, 0), (ty.maximum, 2), (0, 5),
      (ty.maximum, ty.maximum)]

private def bitPatterns (ty : IntType) : List Nat :=
  [0, 1, ty.mask, ty.mask - 1, ty.mask / 2, ty.mask / 3, ty.mask / 5]

private def shiftCounts (ty : IntType) : List Int :=
  [-1, 0, 1, 2, (ty.width / 2 : Nat), ty.width - 1, ty.width, ty.width + 1,
    63, 64, -(ty.width : Int)]

private def shiftPairs (ty : IntType) : List (Nat × Int) :=
  let h := ty.mask / 2
  [(0, 0), (1, 1), (ty.mask, ty.width - 1), (h, (ty.width / 2 : Nat)),
    (h, ty.width), (ty.mask / 3, -1), (ty.mask / 5, 63), (h, -(ty.width : Int))]

private def comparePairs (ty : IntType) : List (Int × Int) :=
  if ty.signed then
    [(ty.minimum, ty.maximum), (-1, 0), (-1, 1), (0, 0), (ty.maximum, ty.maximum - 1),
      (ty.minimum, 0)]
  else
    [(ty.maximum, 1), (0, ty.maximum), (ty.maximum, ty.maximum),
      (ty.maximum / 2, ty.maximum / 2 + 1), (1, 0)]

private def convertValues (ty : IntType) : List Int :=
  if ty.signed then [ty.minimum, ty.maximum]
  else [0, ty.maximum]

private def minMaxLists (ty : IntType) : List (List Int) :=
  let v := valuesOf ty
  let a := v.getD 0 0
  let b := ty.maximum
  let c := ty.minimum
  [[a], [b, a], [a, b], [b, b, a], [c, a, b]]

private def limitCases (ty : IntType) : List (Int × Int × Int) :=
  let lo := ty.minimum
  let hi := ty.maximum
  let mid := (lo + hi) / 2
  -- Keep every value inside the 64-bit JSON integer range.
  [(lo, mid, hi), (mid, lo, hi), (lo, hi, mid), (lo, lo, hi),
    (lo, hi, hi), (mid, lo, mid), (hi, mid, lo), (lo, hi - 1, hi)]

private def muxCases (ty : IntType) : List (Int × List Int × Int) :=
  let a := valuesOf ty |>.getD 0 0
  let b := ty.maximum
  let d := ty.minimum
  [(0, [a, b], d), (1, [a, b], d), (2, [a, b], d), (-1, [a, b], d),
    (0, [], d), (0, [a], d)]

/-- The printable test strings, per encoding. -/
private def stringCases : List (String × String) :=
  [("SBCS", ""), ("SBCS", "A"), ("SBCS", "hello"), ("SBCS", "ABCXYZ"),
    ("DBCS", ""), ("DBCS", "hello"), ("DBCS", "héllo"), ("DBCS", "😀"),
    ("DBCS", "a€😀z"),
    ("UNICODE", ""), ("UNICODE", "hello"), ("UNICODE", "héllo"),
    ("UNICODE", "😀"), ("UNICODE", "a€😀z")]

private def stringKindOf : String → StringKind
  | "DBCS" => .dbcs
  | "UNICODE" => .unicode
  | _ => .sbcs

private def scalarsOf (s : String) : List Nat :=
  s.toList.map Char.toNat

/-! ## Vectors -/

private def arithmeticVector (name : String) (op : IntType → Int → Int → Res)
    (pairs : IntType → List (Int × Int)) : Vector :=
  { name := name
    cases := intTypes.flatMap fun ty =>
      let layout := typeLayout ty
      (pairs layout).map fun (a, b) => binaryCase ty a b (op layout a b) }

private def unaryVector (name : String) (op : IntType → Int → Res) : Vector :=
  { name := name
    cases := intTypes.flatMap fun ty =>
      let layout := typeLayout ty
      (valuesOf layout).map fun a => unaryCase ty a (op layout a) }

private def bitwiseVector (name : String) (op : IntType → Nat → Nat → Nat) : Vector :=
  { name := name
    cases := (bitTypes.map fun ty => (ty, false)) ++ [("BOOL", true)] |>.flatMap fun (ty, isBool) =>
      if isBool then
        let combos := [(false, false), (false, true), (true, false), (true, true)]
        combos.map fun (a, b) =>
          let result := op (typeLayout ty) (if a then 1 else 0) (if b then 1 else 0)
          [("a", .bool a), ("b", .bool b), ("result", .bool (result != 0)), typeField ty]
      else
        let layout := typeLayout ty
        let patterns := bitPatterns layout
        patterns.flatMap fun a =>
          patterns.take 3 |>.map fun b =>
            [("a", .nat a), ("b", .nat b), ("result", .nat (op layout a b)), typeField ty] }

private def notVector : Vector :=
  { name := "NOT"
    cases := (bitTypes.map fun ty => (ty, false)) ++ [("BOOL", true)] |>.flatMap fun (ty, isBool) =>
      if isBool then
        [false, true].map fun a =>
          [("a", .bool a), ("result", .bool (!a)), typeField ty]
      else
        let layout := typeLayout ty
        (bitPatterns layout).map fun a =>
          [("a", .nat a), ("result", .nat (layout.bitNot a)), typeField ty] }

private def shiftVector (name : String) (op : IntType → Nat → Int → Nat) : Vector :=
  { name := name
    cases := bitTypes.flatMap fun ty =>
      let layout := typeLayout ty
      (shiftPairs layout).map fun (a, n) =>
        [("a", .nat a), ("n", .int n), ("result", .nat (op layout a n)), typeField ty] }

private def compareVector (name : String) (op : Int → Int → Bool) : Vector :=
  { name := name
    cases := compareTypes.flatMap fun ty =>
      let layout := typeLayout ty
      (comparePairs layout).map fun (a, b) =>
        [("a", .int a), ("b", .int b), ("result", .bool (op a b)), typeField ty] }

private def selectVector : Vector :=
  { name := "SEL"
    cases := selectTypes.flatMap fun ty =>
      let layout := typeLayout ty
      let pairs := [(1, 2), (layout.maximum, layout.minimum), (0, 0)]
      [false, true].flatMap fun g =>
        pairs.map fun (a, b) =>
          [("a", .int a), ("b", .int b), ("g", .bool g),
            ("result", .int (sel g a b)), typeField ty] }

private def minMaxVector (name : String) (op : List Int → Option Int) : Vector :=
  { name := name
    cases := compareTypes.flatMap fun ty =>
      let layout := typeLayout ty
      (minMaxLists layout).map fun values =>
        [("result", .int ((op values).getD 0)), typeField ty,
          ("values", .arr (values.map J.int))] }

private def limitVector : Vector :=
  { name := "LIMIT"
    cases := compareTypes.flatMap fun ty =>
      let layout := typeLayout ty
      (limitCases layout).map fun (mn, x, mx) =>
        [("mn", .int mn), ("mx", .int mx), ("result", .int (limit mn x mx)),
          typeField ty, ("x", .int x)] }

private def muxVector : Vector :=
  { name := "MUX"
    cases := muxTypes.flatMap fun ty =>
      let layout := typeLayout ty
      (muxCases layout).map fun (k, values, d) =>
        [("default", .int d), ("k", .int k), ("result", .int (mux k values d)),
          typeField ty, ("values", .arr (values.map J.int))] }

private def convVector : Vector :=
  { name := "CONV"
    cases := convTypes.flatMap fun src =>
      let srcLayout := typeLayout src
      convTypes.flatMap fun to =>
        (convertValues srcLayout).map fun value =>
          [("from", .str src),
            ("result", resultJson (srcLayout.convert (typeLayout to) value)),
            ("to", .str to), ("value", .int value)] }

private def lenVector (name : String) (op : StringKind → List Nat → Nat) : Vector :=
  { name := name
    cases := stringCases.map fun (kind, s) =>
      [("kind", .str kind), ("result", .nat (op (stringKindOf kind) (scalarsOf s))),
        ("s", .str s)] }

private def lenMaxVector : Vector :=
  { name := "LEN_MAX"
    cases := ["SBCS", "DBCS", "UNICODE"].flatMap fun kind =>
      [1, 80, 255, 65535].map fun capacity =>
        [("capacity", .nat capacity), ("kind", .str kind),
          ("result", .nat (lenMax capacity))] }

/-- Every generated vector, in deterministic order. -/
def vectors : List Vector :=
  [ arithmeticVector "ADD" IntType.add (fun ty => addPairs ty)
  , arithmeticVector "SUB" IntType.sub (fun ty => subPairs ty)
  , arithmeticVector "MUL" IntType.mul (fun ty => mulPairs ty)
  , arithmeticVector "DIV" IntType.div (fun ty => divPairs ty)
  , arithmeticVector "MOD" IntType.mod (fun ty => divPairs ty)
  , unaryVector "NEG" IntType.neg
  , unaryVector "ABS" IntType.abs
  , bitwiseVector "AND" IntType.bitAnd
  , bitwiseVector "OR" IntType.bitOr
  , bitwiseVector "XOR" IntType.bitXor
  , notVector
  , shiftVector "SHL" IntType.shl
  , shiftVector "SHR" IntType.shr
  , shiftVector "ROL" IntType.rol
  , shiftVector "ROR" IntType.ror
  , compareVector "EQ" (fun a b => a == b)
  , compareVector "NE" (fun a b => a != b)
  , compareVector "LT" (fun a b => a < b)
  , compareVector "LE" (fun a b => a ≤ b)
  , compareVector "GT" (fun a b => a > b)
  , compareVector "GE" (fun a b => a ≥ b)
  , selectVector
  , minMaxVector "MIN" minList
  , minMaxVector "MAX" maxList
  , limitVector
  , muxVector
  , convVector
  , lenVector "LEN" len
  , lenMaxVector
  , lenVector "LEN_CODE_UNIT" lenCodeUnit
  ]

end Stc.Vectors
