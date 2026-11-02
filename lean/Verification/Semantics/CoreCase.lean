/-
The `stc-core-case` `0.1.0` descriptor reader (P0.6).

A case names one program entry point, a requested cycle count, a fuel bound for
the reference interpreter, coverage feature tags, and the expected observable
result. The descriptor is specified in
`specification/Verification/Semantics-Cases.md`; corpus descriptors live under
`tests/verification/semantics/<case>/case.json`.

The schema is deliberately narrow. `inputs` is required but must be empty in
`0.1.0` because the only entry kind is a parameterless program; a non-empty
list is an error rather than silently ignored. Expected statuses are `halted`
and `trap`; an `outOfFuel` or `stuck` reference result is always a harness
failure, not an expected outcome.
-/
import Verification.Semantics.CoreDump
import Stc.Exact.Int

namespace Verification.Semantics

open Lean (Json)
open Stc.Exact (Trap)

/-! ## Format constants -/

/-- The case descriptor format family. -/
def coreCaseFormat : String := "stc-core-case"

/-- The exact case descriptor version understood by this reader. -/
def coreCaseFormatVersion : String := "0.1.0"

/-! ## Descriptor -/

/-- The entry point named by a case. -/
structure CaseEntry where
  /-- The entry kind; `program` is the only `0.1.0` kind. -/
  kind : String
  /-- The canonical program name. -/
  name : String

/-- The expected observable result of a case. -/
structure CaseExpected where
  /-- `halted` or `trap`. -/
  status : String
  /-- The expected trap for `status = "trap"`; `none` for `halted`. -/
  trap : Option Trap
  /-- Expected global values, in unit declaration order. -/
  globals : List Int
  /-- Expected completed cycles. -/
  cycles : Nat

/-- A parsed `stc-core-case` `0.1.0` descriptor. -/
structure CoreCase where
  /-- The case name. -/
  name : String
  /-- The entry point. -/
  entry : CaseEntry
  /-- The reference-interpreter fuel bound. -/
  fuel : Nat
  /-- The requested number of program cycles. -/
  cycles : Nat
  /-- Coverage feature tags, strictly sorted. -/
  features : List String
  /-- The expected result. -/
  expected : CaseExpected

/-! ## JSON helpers -/

private def fail (message : String) : Except String α := throw message

private def jsonField (j : Json) (name ctx : String) : Except String Json := do
  match j.getObjVal? name with
  | .ok value => pure value
  | .error _ => fail s!"{ctx}.{name}: required field is missing"

private def jsonString (j : Json) (name ctx : String) : Except String String := do
  let value ← jsonField j name ctx
  match value.getStr? with
  | .ok text => pure text
  | .error _ => fail s!"{ctx}.{name}: expected a string"

private def jsonNat (j : Json) (name ctx : String) : Except String Nat := do
  let value ← jsonField j name ctx
  match value.getNat? with
  | .ok number => pure number
  | .error _ => fail s!"{ctx}.{name}: expected a non-negative integer"

private def jsonArray (j : Json) (name ctx : String) : Except String (Array Json) := do
  let value ← jsonField j name ctx
  match value.getArr? with
  | .ok items => pure items
  | .error _ => fail s!"{ctx}.{name}: expected an array"

private def valueInt (j : Json) (ctx : String) : Except String Int := do
  match j.getInt? with
  | .ok number => pure number
  | .error _ => fail s!"{ctx}: expected an integer"

private def mapJsonArray (items : Array Json) (parse : Nat → Json → Except String α) :
    Except String (List α) := do
  let mut result : Array α := #[]
  for index in [0:items.size] do
    result := result.push (← parse index items[index]!)
  pure result.toList

/-! ## Validation helpers -/

private def isUpperAscii (c : Char) : Bool := 'A' ≤ c && c ≤ 'Z'

private def isLowerAscii (c : Char) : Bool := 'a' ≤ c && c ≤ 'z'

private def isDigitAscii (c : Char) : Bool := '0' ≤ c && c ≤ '9'

private def isIdentStart (c : Char) : Bool := isUpperAscii c || c == '_'

private def isIdentRest (c : Char) : Bool := isIdentStart c || isDigitAscii c

private def checkCanonicalName (name : String) (ctx : String) : Except String Unit := do
  for part in name.splitOn "." do
    match part.toList with
    | [] => fail s!"{ctx}: name `{name}` has an empty segment"
    | first :: rest =>
        if !isIdentStart first then
          fail s!"{ctx}: name `{name}` is not uppercase canonical"
        if !rest.all isIdentRest then
          fail s!"{ctx}: name `{name}` is not uppercase canonical"

private def checkCaseName (name : String) (ctx : String) : Except String Unit := do
  match name.toList with
  | [] => fail s!"{ctx}: case name is empty"
  | first :: rest =>
      let start := isUpperAscii first || isLowerAscii first || isDigitAscii first
      let body := fun c =>
        isUpperAscii c || isLowerAscii c || isDigitAscii c || c == '.' || c == '-' || c == '_'
      if !start then
        fail s!"{ctx}: case name `{name}` must start with a letter or digit"
      if !rest.all body then
        fail s!"{ctx}: case name `{name}` contains a character outside [A-Za-z0-9._-]"

private def checkFeatureTag (tag : String) (ctx : String) : Except String Unit := do
  match tag.toList with
  | [] => fail s!"{ctx}: feature tag is empty"
  | first :: rest =>
      let start := isLowerAscii first || isDigitAscii first
      let body := fun c =>
        isLowerAscii c || isDigitAscii c || c == '.' || c == '-' || c == '_'
      if !start then
        fail s!"{ctx}: feature tag `{tag}` must start with a lowercase letter or digit"
      if !rest.all body then
        fail s!"{ctx}: feature tag `{tag}` contains a character outside [a-z0-9._-]"

private def strictlySorted (tags : List String) : Bool :=
  let rec go : List String → Bool
    | [] => true
    | [_] => true
    | first :: second :: rest => first < second && go (second :: rest)
  go tags

/-! ## Parser -/

private def parseTrap (j : Json) (ctx : String) : Except String (Option Trap) := do
  if j.isNull then
    pure none
  else
    let name ←
      match j.getStr? with
      | .ok text => pure text
      | .error _ => fail s!"{ctx}: expected a trap name or null"
    match name with
    | "OVERFLOW" => pure (some .overflow)
    | "DIV_ZERO" => pure (some .divZero)
    | "RANGE" => pure (some .range)
    | other => fail s!"{ctx}: unknown trap `{other}`"

private def parseExpected (j : Json) (ctx : String) : Except String CaseExpected := do
  let status ← jsonString j "status" ctx
  if status != "halted" && status != "trap" then
    fail s!"{ctx}.status: unknown expected status `{status}` (expected `halted` or `trap`)"
  let trapJson ← jsonField j "trap" ctx
  let trap ← parseTrap trapJson s!"{ctx}.trap"
  if status == "halted" && trap.isSome then
    fail s!"{ctx}.trap: a halted case must have a null trap"
  if status == "trap" && trap.isNone then
    fail s!"{ctx}.trap: a trapping case must name its trap"
  let globalsJson ← jsonArray j "globals" ctx
  let globals ← mapJsonArray globalsJson fun index value =>
    valueInt value s!"{ctx}.globals[{index}]"
  let cycles ← jsonNat j "cycles" ctx
  pure { status, trap, globals, cycles }

private def parseEntry (j : Json) (ctx : String) : Except String CaseEntry := do
  let kind ← jsonString j "kind" ctx
  if kind != "program" then
    fail s!"{ctx}.kind: unknown entry kind `{kind}` (the core subset has only `program`)"
  let name ← jsonString j "name" ctx
  checkCanonicalName name s!"{ctx}.name"
  pure { kind, name }

/-- Parse and validate a `stc-core-case` `0.1.0` descriptor. -/
def parseCoreCase (text : String) : Except String CoreCase := do
  let root ← Json.parse text
  let format ← jsonString root "format" "case"
  if format != coreCaseFormat then
    fail s!"case.format: unknown format `{format}` (expected `{coreCaseFormat}`)"
  let formatVersion ← jsonString root "formatVersion" "case"
  if formatVersion != coreCaseFormatVersion then
    fail s!"case.formatVersion: unknown version `{formatVersion}` (expected `{coreCaseFormatVersion}`)"
  let semanticsVersion ← jsonString root "semanticsVersion" "case"
  if semanticsVersion != coreDumpSemanticsVersion then
    fail s!"case.semanticsVersion: unknown version `{semanticsVersion}` (expected `{coreDumpSemanticsVersion}`)"
  let profile ← jsonString root "profile" "case"
  if profile != coreDumpProfile then
    fail s!"case.profile: unknown profile `{profile}` (expected `{coreDumpProfile}`)"
  let name ← jsonString root "name" "case"
  checkCaseName name "case.name"
  let entry ← parseEntry (← jsonField root "entry" "case") "case.entry"
  let inputs ← jsonArray root "inputs" "case"
  if !inputs.isEmpty then
    fail "case.inputs: the program entry point takes no inputs in version 0.1.0"
  let cycles ← jsonNat root "cycles" "case"
  let fuel ← jsonNat root "fuel" "case"
  if fuel == 0 then
    fail "case.fuel: the reference interpreter needs a positive fuel bound"
  let featuresJson ← jsonArray root "features" "case"
  let features ← mapJsonArray featuresJson fun index tag =>
    match tag.getStr? with
    | .ok text => pure text
    | .error _ => fail s!"case.features[{index}]: expected a string"
  for index in [0:features.length] do
    checkFeatureTag features[index]! s!"case.features[{index}]"
  if !strictlySorted features then
    fail "case.features: feature tags must be strictly sorted and duplicate-free"
  let expected ← parseExpected (← jsonField root "expected" "case") "case.expected"
  pure { name, entry, fuel, cycles, features, expected }

end Verification.Semantics
