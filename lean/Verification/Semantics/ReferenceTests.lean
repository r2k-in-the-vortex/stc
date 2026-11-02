/-
Unit tests for the `stc-sem` reference path (P0.6).

The fixtures are temporary: `lake exe stc-sem-tests` builds JSON documents in
memory or writes them to a temporary directory. No `case.json` is committed, so
the fail-closed `semantics-conformance` placeholder stays green.
-/
import Lean.Data.Json
import Verification.Semantics.ReferenceCli

namespace Verification.Semantics.ReferenceTests

open Lean (Json)
open Verification.Semantics
open Verification.Semantics.ReferenceCli

/-! ## Fixtures -/

/-- A comprehensive accepted dump emitted by `stc hir-dump` for a program that
exercises a function call, globals, locals, `FOR`, `IF`, `CASE`, `WHILE`,
`REPEAT`, unary negation, and the `ABS` standard function. -/
private def runtimeDumpText : String :=
  r##"{"features":[],"format":"stc-core-hir","formatVersion":"0.1.0","package":"_ROOT","profile":"stc-2013-oop-utf8-v1","semanticsVersion":"1.0-draft","status":"ok","unit":{"functions":[{"body":[{"kind":"assign","place":{"kind":"var","name":"T","position":0,"span":{"end":131,"file":"runtime.st","start":130},"varKind":"local"},"span":{"end":141,"file":"runtime.st","start":130},"value":{"kind":"binary","lhs":{"kind":"var","name":"A","position":0,"span":{"end":136,"file":"runtime.st","start":135},"varKind":"param"},"op":"add","rhs":{"kind":"var","name":"B","position":1,"span":{"end":140,"file":"runtime.st","start":139},"varKind":"param"},"span":{"end":138,"file":"runtime.st","start":137},"type":{"kind":"int","signed":true,"width":32}}},{"kind":"assign","place":{"kind":"var","name":"ADD","span":{"end":149,"file":"runtime.st","start":146},"varKind":"result"},"span":{"end":155,"file":"runtime.st","start":146},"value":{"kind":"var","name":"T","position":0,"span":{"end":154,"file":"runtime.st","start":153},"varKind":"local"}}],"locals":[{"init":null,"name":"T","span":{"end":109,"file":"runtime.st","start":108},"type":{"kind":"int","signed":true,"width":32}}],"name":"ADD","params":[{"name":"A","span":{"end":69,"file":"runtime.st","start":68},"type":{"kind":"int","signed":true,"width":32}},{"name":"B","span":{"end":83,"file":"runtime.st","start":82},"type":{"kind":"int","signed":true,"width":32}}],"result":{"kind":"int","signed":true,"width":32},"span":{"end":168,"file":"runtime.st","start":34}}],"globals":[{"gotKey":"_ROOT::G","init":null,"name":"G","span":{"end":16,"file":"runtime.st","start":15},"type":{"kind":"int","signed":true,"width":32}}],"program":{"body":[{"kind":"assign","place":{"gotKey":"_ROOT::G","kind":"var","name":"G","position":0,"span":{"end":253,"file":"runtime.st","start":252},"varKind":"global"},"span":{"end":267,"file":"runtime.st","start":252},"value":{"args":[{"kind":"literal","span":{"end":262,"file":"runtime.st","start":261},"type":{"kind":"int","signed":true,"width":32},"value":1},{"kind":"literal","span":{"end":265,"file":"runtime.st","start":264},"type":{"kind":"int","signed":true,"width":32},"value":2}],"callee":{"kind":"function","name":"ADD","span":{"end":266,"file":"runtime.st","start":257}},"kind":"call","span":{"end":266,"file":"runtime.st","start":257},"type":{"kind":"int","signed":true,"width":32}}},{"kind":"assign","place":{"kind":"var","name":"X","position":0,"span":{"end":273,"file":"runtime.st","start":272},"varKind":"local"},"span":{"end":283,"file":"runtime.st","start":272},"value":{"kind":"binary","lhs":{"gotKey":"_ROOT::G","kind":"var","name":"G","position":0,"span":{"end":278,"file":"runtime.st","start":277},"varKind":"global"},"op":"mul","rhs":{"kind":"literal","span":{"end":282,"file":"runtime.st","start":281},"type":{"kind":"int","signed":true,"width":32},"value":2},"span":{"end":280,"file":"runtime.st","start":279},"type":{"kind":"int","signed":true,"width":32}}},{"kind":"assign","place":{"kind":"var","name":"X","position":0,"span":{"end":289,"file":"runtime.st","start":288},"varKind":"local"},"span":{"end":296,"file":"runtime.st","start":288},"value":{"kind":"unary","op":"neg","operand":{"kind":"var","name":"X","position":0,"span":{"end":295,"file":"runtime.st","start":294},"varKind":"local"},"span":{"end":295,"file":"runtime.st","start":293},"type":{"kind":"int","signed":true,"width":32}}},{"kind":"assign","place":{"kind":"var","name":"X","position":0,"span":{"end":302,"file":"runtime.st","start":301},"varKind":"local"},"span":{"end":313,"file":"runtime.st","start":301},"value":{"args":[{"kind":"var","name":"X","position":0,"span":{"end":311,"file":"runtime.st","start":310},"varKind":"local"}],"callee":{"kind":"std","name":"ABS","span":{"end":312,"file":"runtime.st","start":306}},"kind":"call","span":{"end":312,"file":"runtime.st","start":306},"type":{"kind":"int","signed":true,"width":32}}},{"kind":"assign","place":{"kind":"var","name":"Y","position":1,"span":{"end":319,"file":"runtime.st","start":318},"varKind":"local"},"span":{"end":325,"file":"runtime.st","start":318},"value":{"kind":"literal","span":{"end":324,"file":"runtime.st","start":323},"type":{"kind":"int","signed":true,"width":32},"value":0}},{"body":[{"kind":"assign","place":{"kind":"var","name":"Y","position":1,"span":{"end":358,"file":"runtime.st","start":357},"varKind":"local"},"span":{"end":368,"file":"runtime.st","start":357},"value":{"kind":"binary","lhs":{"kind":"var","name":"Y","position":1,"span":{"end":363,"file":"runtime.st","start":362},"varKind":"local"},"op":"add","rhs":{"kind":"var","name":"I","position":2,"span":{"end":367,"file":"runtime.st","start":366},"varKind":"local"},"span":{"end":365,"file":"runtime.st","start":364},"type":{"kind":"int","signed":true,"width":32}}}],"kind":"for","span":{"end":381,"file":"runtime.st","start":330},"start":{"kind":"literal","span":{"end":340,"file":"runtime.st","start":339},"type":{"kind":"int","signed":true,"width":32},"value":0},"step":{"kind":"literal","span":{"end":381,"file":"runtime.st","start":330},"type":{"kind":"int","signed":true,"width":32},"value":1},"stop":{"kind":"literal","span":{"end":345,"file":"runtime.st","start":344},"type":{"kind":"int","signed":true,"width":32},"value":2},"var":{"kind":"var","name":"I","position":2,"span":{"end":335,"file":"runtime.st","start":334},"varKind":"local"}},{"arms":[{"body":[{"kind":"assign","place":{"kind":"var","name":"C","position":3,"span":{"end":409,"file":"runtime.st","start":408},"varKind":"local"},"span":{"end":418,"file":"runtime.st","start":408},"value":{"kind":"literal","span":{"end":417,"file":"runtime.st","start":413},"type":{"kind":"bool"},"value":1}}],"cond":{"kind":"binary","lhs":{"kind":"var","name":"X","position":0,"span":{"end":390,"file":"runtime.st","start":389},"varKind":"local"},"op":"gt","rhs":{"kind":"literal","span":{"end":394,"file":"runtime.st","start":393},"type":{"kind":"int","signed":true,"width":32},"value":4},"span":{"end":392,"file":"runtime.st","start":391},"type":{"kind":"bool"}},"span":{"end":459,"file":"runtime.st","start":386}}],"else":[{"kind":"assign","place":{"kind":"var","name":"C","position":3,"span":{"end":437,"file":"runtime.st","start":436},"varKind":"local"},"span":{"end":447,"file":"runtime.st","start":436},"value":{"kind":"literal","span":{"end":446,"file":"runtime.st","start":441},"type":{"kind":"bool"},"value":0}}],"kind":"if","span":{"end":459,"file":"runtime.st","start":386}},{"arms":[{"body":[{"kind":"assign","place":{"kind":"var","name":"Y","position":1,"span":{"end":486,"file":"runtime.st","start":485},"varKind":"local"},"span":{"end":493,"file":"runtime.st","start":485},"value":{"kind":"literal","span":{"end":492,"file":"runtime.st","start":490},"type":{"kind":"int","signed":true,"width":32},"value":10}}],"labels":[{"high":null,"low":0,"span":{"end":483,"file":"runtime.st","start":482}}],"span":{"end":561,"file":"runtime.st","start":464}},{"body":[{"kind":"assign","place":{"kind":"var","name":"Y","position":1,"span":{"end":509,"file":"runtime.st","start":508},"varKind":"local"},"span":{"end":521,"file":"runtime.st","start":508},"value":{"kind":"binary","lhs":{"kind":"var","name":"Y","position":1,"span":{"end":514,"file":"runtime.st","start":513},"varKind":"local"},"op":"add","rhs":{"kind":"literal","span":{"end":520,"file":"runtime.st","start":517},"type":{"kind":"int","signed":true,"width":32},"value":100},"span":{"end":516,"file":"runtime.st","start":515},"type":{"kind":"int","signed":true,"width":32}}}],"labels":[{"high":3,"low":1,"span":{"end":506,"file":"runtime.st","start":502}}],"span":{"end":561,"file":"runtime.st","start":464}}],"else":[{"kind":"assign","place":{"kind":"var","name":"Y","position":1,"span":{"end":540,"file":"runtime.st","start":539},"varKind":"local"},"span":{"end":547,"file":"runtime.st","start":539},"value":{"kind":"unary","op":"neg","operand":{"kind":"literal","span":{"end":546,"file":"runtime.st","start":545},"type":{"kind":"int","signed":true,"width":32},"value":1},"span":{"end":546,"file":"runtime.st","start":544},"type":{"kind":"int","signed":true,"width":32}}}],"kind":"case","selector":{"kind":"var","name":"Y","position":1,"span":{"end":470,"file":"runtime.st","start":469},"varKind":"local"},"span":{"end":561,"file":"runtime.st","start":464}},{"body":[{"kind":"assign","place":{"kind":"var","name":"Y","position":1,"span":{"end":592,"file":"runtime.st","start":591},"varKind":"local"},"span":{"end":602,"file":"runtime.st","start":591},"value":{"kind":"binary","lhs":{"kind":"var","name":"Y","position":1,"span":{"end":597,"file":"runtime.st","start":596},"varKind":"local"},"op":"add","rhs":{"kind":"literal","span":{"end":601,"file":"runtime.st","start":600},"type":{"kind":"int","signed":true,"width":32},"value":1},"span":{"end":599,"file":"runtime.st","start":598},"type":{"kind":"int","signed":true,"width":32}}}],"cond":{"kind":"binary","lhs":{"kind":"var","name":"Y","position":1,"span":{"end":573,"file":"runtime.st","start":572},"varKind":"local"},"op":"lt","rhs":{"kind":"literal","span":{"end":579,"file":"runtime.st","start":576},"type":{"kind":"int","signed":true,"width":32},"value":110},"span":{"end":575,"file":"runtime.st","start":574},"type":{"kind":"bool"}},"kind":"while","span":{"end":617,"file":"runtime.st","start":566}},{"body":[{"kind":"assign","place":{"kind":"var","name":"Y","position":1,"span":{"end":638,"file":"runtime.st","start":637},"varKind":"local"},"span":{"end":648,"file":"runtime.st","start":637},"value":{"kind":"binary","lhs":{"kind":"var","name":"Y","position":1,"span":{"end":643,"file":"runtime.st","start":642},"varKind":"local"},"op":"sub","rhs":{"kind":"literal","span":{"end":647,"file":"runtime.st","start":646},"type":{"kind":"int","signed":true,"width":32},"value":1},"span":{"end":645,"file":"runtime.st","start":644},"type":{"kind":"int","signed":true,"width":32}}}],"cond":{"kind":"binary","lhs":{"kind":"var","name":"Y","position":1,"span":{"end":660,"file":"runtime.st","start":659},"varKind":"local"},"op":"le","rhs":{"kind":"literal","span":{"end":667,"file":"runtime.st","start":664},"type":{"kind":"int","signed":true,"width":32},"value":105},"span":{"end":663,"file":"runtime.st","start":661},"type":{"kind":"bool"}},"kind":"repeat","span":{"end":683,"file":"runtime.st","start":622}},{"kind":"assign","place":{"gotKey":"_ROOT::G","kind":"var","name":"G","position":0,"span":{"end":689,"file":"runtime.st","start":688},"varKind":"global"},"span":{"end":695,"file":"runtime.st","start":688},"value":{"kind":"var","name":"Y","position":1,"span":{"end":694,"file":"runtime.st","start":693},"varKind":"local"}}],"locals":[{"init":null,"name":"X","span":{"end":189,"file":"runtime.st","start":188},"type":{"kind":"int","signed":true,"width":32}},{"init":null,"name":"Y","span":{"end":203,"file":"runtime.st","start":202},"type":{"kind":"int","signed":true,"width":32}},{"init":null,"name":"I","span":{"end":217,"file":"runtime.st","start":216},"type":{"kind":"int","signed":true,"width":32}},{"init":null,"name":"C","span":{"end":231,"file":"runtime.st","start":230},"type":{"kind":"bool"}}],"name":"P","span":{"end":707,"file":"runtime.st","start":170}},"span":{"end":707,"file":"runtime.st","start":170}}}"##

/-- An accepted dump whose program traps with `DIV_ZERO`. -/
private def trapDumpText : String :=
  r##"{"features":[],"format":"stc-core-hir","formatVersion":"0.1.0","package":"_ROOT","profile":"stc-2013-oop-utf8-v1","semanticsVersion":"1.0-draft","status":"ok","unit":{"functions":[],"globals":[{"gotKey":"_ROOT::G","init":null,"name":"G","span":{"end":16,"file":"trap.st","start":15},"type":{"kind":"int","signed":true,"width":32}}],"program":{"body":[{"kind":"assign","place":{"gotKey":"_ROOT::G","kind":"var","name":"G","position":0,"span":{"end":79,"file":"trap.st","start":78},"varKind":"global"},"span":{"end":89,"file":"trap.st","start":78},"value":{"kind":"binary","lhs":{"kind":"literal","span":{"end":84,"file":"trap.st","start":83},"type":{"kind":"int","signed":true,"width":32},"value":1},"op":"div","rhs":{"kind":"literal","span":{"end":88,"file":"trap.st","start":87},"type":{"kind":"int","signed":true,"width":32},"value":0},"span":{"end":86,"file":"trap.st","start":85},"type":{"kind":"int","signed":true,"width":32}}},{"kind":"assign","place":{"kind":"var","name":"X","position":0,"span":{"end":95,"file":"trap.st","start":94},"varKind":"local"},"span":{"end":101,"file":"trap.st","start":94},"value":{"kind":"literal","span":{"end":100,"file":"trap.st","start":99},"type":{"kind":"int","signed":true,"width":32},"value":1}}],"locals":[{"init":null,"name":"X","span":{"end":57,"file":"trap.st","start":56},"type":{"kind":"int","signed":true,"width":32}}],"name":"TRAPP","span":{"end":113,"file":"trap.st","start":34}},"span":{"end":113,"file":"trap.st","start":34}}}"##

private def dintJson : Json :=
  Json.mkObj [("kind", "int"), ("width", 32), ("signed", true)]

private def spanJson : Json :=
  Json.mkObj [("file", "fixture.st"), ("start", 0), ("end", 1)]

private def globalPlaceJson : Json :=
  Json.mkObj [
    ("kind", "var"),
    ("varKind", "global"),
    ("name", "G"),
    ("position", 0),
    ("gotKey", "_ROOT::G"),
    ("span", spanJson)
  ]

private def literalJson (value : Int) (ty : Json := dintJson) : Json :=
  Json.mkObj [
    ("kind", "literal"),
    ("type", ty),
    ("value", (value : Json)),
    ("span", spanJson)
  ]

private def assignGlobalJson (value : Json) : Json :=
  Json.mkObj [
    ("kind", "assign"),
    ("place", globalPlaceJson),
    ("value", value),
    ("span", spanJson)
  ]

/-- A minimal accepted dump with one global `G` and a program `P` whose body
assigns `value` to `G`. -/
private def minimalDump (value : Json) (status : String := "ok")
    (features : Json := Json.arr #[]) : Json :=
  Json.mkObj [
    ("format", coreDumpFormat),
    ("formatVersion", coreDumpFormatVersion),
    ("semanticsVersion", coreDumpSemanticsVersion),
    ("profile", coreDumpProfile),
    ("package", "_ROOT"),
    ("status", status),
    ("features", features),
    ("unit", Json.mkObj [
      ("globals", Json.arr #[Json.mkObj [
        ("name", "G"),
        ("gotKey", "_ROOT::G"),
        ("type", dintJson),
        ("init", Json.null),
        ("span", spanJson)
      ]]),
      ("functions", Json.arr #[]),
      ("program", Json.mkObj [
        ("name", "P"),
        ("locals", Json.arr #[]),
        ("body", Json.arr #[assignGlobalJson value]),
        ("span", spanJson)
      ]),
      ("span", spanJson)
    ])
  ]

private def nestedPos : Nat → Json
  | 0 => literalJson 1
  | n + 1 => Json.mkObj [
      ("kind", "unary"),
      ("op", "pos"),
      ("operand", nestedPos n),
      ("type", dintJson),
      ("span", spanJson)
    ]

private def runtimeFeatures : List String := [
  "control.case",
  "control.for",
  "control.if",
  "control.repeat",
  "control.while",
  "function.call",
  "std.abs",
  "type.bool",
  "type.int32"
]

private def runtimeCaseJson : Json :=
  Json.mkObj [
    ("format", coreCaseFormat),
    ("formatVersion", coreCaseFormatVersion),
    ("semanticsVersion", coreDumpSemanticsVersion),
    ("profile", coreDumpProfile),
    ("name", "runtime-p"),
    ("entry", Json.mkObj [("kind", "program"), ("name", "P")]),
    ("inputs", Json.arr #[]),
    ("cycles", 1),
    ("fuel", 10000),
    ("features", Json.arr (runtimeFeatures.map (fun tag => Json.str tag)).toArray),
    ("expected", Json.mkObj [
      ("status", "halted"),
      ("trap", Json.null),
      ("globals", Json.arr #[(105 : Json)]),
      ("cycles", 1)
    ])
  ]

private def trapCaseJson : Json :=
  Json.mkObj [
    ("format", coreCaseFormat),
    ("formatVersion", coreCaseFormatVersion),
    ("semanticsVersion", coreDumpSemanticsVersion),
    ("profile", coreDumpProfile),
    ("name", "trap-p"),
    ("entry", Json.mkObj [("kind", "program"), ("name", "TRAPP")]),
    ("inputs", Json.arr #[]),
    ("cycles", 1),
    ("fuel", 100),
    ("features", Json.arr #[Json.str "trap.div-zero"]),
    ("expected", Json.mkObj [
      ("status", "trap"),
      ("trap", "DIV_ZERO"),
      ("globals", Json.arr #[(0 : Json)]),
      ("cycles", 0)
    ])
  ]

/-! ## Assertions -/

private def contains (haystack needle : String) : Bool :=
  (haystack.splitOn needle).length > 1

private def ensure (label : String) (condition : Bool) : IO Unit := do
  unless condition do
    throw (IO.userError label)

private def expectOk (label : String) (result : Except String α) : IO α :=
  match result with
  | .ok value => pure value
  | .error message => throw (IO.userError s!"{label}: unexpected error: {message}")

private def expectError (label : String) (result : Except String α) (needle : String) :
    IO Unit :=
  match result with
  | .ok _ => throw (IO.userError s!"{label}: expected an error mentioning `{needle}`, got success")
  | .error message =>
      unless contains message needle do
        throw (IO.userError s!"{label}: error does not mention `{needle}`: {message}")

private def strField (j : Json) (name : String) : Except String String :=
  j.getObjVal? name >>= Json.getStr?

private def natField (j : Json) (name : String) : Except String Nat :=
  j.getObjVal? name >>= Json.getNat?

private def intField (j : Json) (name : String) : Except String Int :=
  j.getObjVal? name >>= Json.getInt?

private def arrField (j : Json) (name : String) : Except String (Array Json) :=
  j.getObjVal? name >>= Json.getArr?

private def arrayAt (items : Array Json) (index : Nat) (label : String) : IO Json :=
  match items[index]? with
  | some value => pure value
  | none => throw (IO.userError s!"{label}: index {index} is out of bounds")

private def runTest (name : String) (action : IO Unit) (failures : IO.Ref (Array String)) :
    IO Unit := do
  try
    action
  catch error =>
    failures.modify fun current => current.push s!"{name}: {error}"

/-! ## Reader tests -/

private def testMinimalReader : IO Unit := do
  let core ← expectOk "minimal dump"
    (parseCoreDump (minimalDump (literalJson 1)).compress)
  ensure "minimal global count" (core.unit.globals.length == 1)
  ensure "minimal global name" (core.unit.globals.head?.map (·.name) == some "G")
  ensure "minimal function count" (core.unit.functions.length == 0)
  ensure "minimal program name" (core.unit.program.name == "P")
  ensure "minimal body count" (core.unit.program.body.length == 1)
  ensure "minimal file table" (core.files == #["fixture.st"])

private def testVersionRejection : IO Unit := do
  let dump := minimalDump (literalJson 1)
  expectError "wrong format"
    (parseCoreDump (dump.setObjVal! "format" (Json.str "stc-other-hir")).compress)
    "unknown format"
  expectError "wrong format version"
    (parseCoreDump (dump.setObjVal! "formatVersion" (Json.str "0.1.1")).compress)
    "unknown version"
  expectError "wrong semantics version"
    (parseCoreDump (dump.setObjVal! "semanticsVersion" (Json.str "1.0")).compress)
    "unknown version"
  expectError "wrong profile"
    (parseCoreDump (dump.setObjVal! "profile" (Json.str "other")).compress)
    "unknown profile"

private def testStatusRejection : IO Unit := do
  let dump := minimalDump (literalJson 1)
  let feature : Json := Json.mkObj [
    ("tag", "type.real"),
    ("detail", "test feature"),
    ("at", Json.null)
  ]
  expectError "status/features mismatch"
    (parseCoreDump (dump.setObjVal! "status" (Json.str "rejected")).compress)
    "does not match"
  expectError "rejected status without tags"
    (parseCoreDump (dump.setObjVal! "features" (Json.arr #[feature])).compress)
    "does not match"
  let rejected := (dump.setObjVal! "features" (Json.arr #[feature])).setObjVal!
    "status" (Json.str "rejected")
  expectError "rejected dump is refused" (parseCoreDump rejected.compress) "rejected core dump"
  expectError "rejected tag is reported" (parseCoreDump rejected.compress) "type.real"

private def testSchemaRejection : IO Unit := do
  expectError "missing header" (parseCoreDump r#"{"format":"stc-core-hir"}"#) "required field"
  expectError "malformed json" (parseCoreDump "{") "unexpected end of input"
  let badType := Json.mkObj [("kind", "int"), ("width", 12), ("signed", true)]
  expectError "non-core width"
    (parseCoreDump (minimalDump (literalJson 1 badType)).compress)
    "non-core integer width"
  let unsupported : Json := Json.mkObj [
    ("kind", "unsupported"),
    ("feature", "expr.test"),
    ("span", spanJson)
  ]
  expectError "unsupported expression"
    (parseCoreDump (minimalDump unsupported).compress)
    "unsupported expression"
  let badVariable : Json := Json.mkObj [
    ("kind", "var"),
    ("varKind", "global"),
    ("name", "G"),
    ("position", 5),
    ("gotKey", "_ROOT::G"),
    ("span", spanJson)
  ]
  expectError "variable position"
    (parseCoreDump (minimalDump badVariable).compress)
    "outside the 1 global"
  let wrongVariableName : Json := Json.mkObj [
    ("kind", "var"),
    ("varKind", "global"),
    ("name", "H"),
    ("position", 0),
    ("gotKey", "_ROOT::H"),
    ("span", spanJson)
  ]
  expectError "variable name mismatch"
    (parseCoreDump (minimalDump wrongVariableName).compress)
    "global 0 is `G`"
  let missingTarget : Json := Json.mkObj [
    ("kind", "call"),
    ("callee", Json.mkObj [
      ("kind", "function"),
      ("name", "MISSING"),
      ("span", spanJson)
    ]),
    ("args", Json.arr #[]),
    ("type", dintJson),
    ("span", spanJson)
  ]
  expectError "unresolved call target"
    (parseCoreDump (minimalDump missingTarget).compress)
    "not declared"
  expectError "nesting limit"
    (parseCoreDump (minimalDump (nestedPos 520)).compress)
    "nesting limit"

private def testComprehensiveReader : IO Unit := do
  let core ← expectOk "runtime dump" (parseCoreDump runtimeDumpText)
  ensure "runtime globals" (core.unit.globals.length == 1)
  ensure "runtime global name" (core.unit.globals.head?.map (·.name) == some "G")
  ensure "runtime functions" (core.unit.functions.length == 1)
  ensure "runtime function name" (core.unit.functions.head?.map (·.name) == some "ADD")
  ensure "runtime function params"
    (core.unit.functions.head?.map (fun fn => fn.params.length) == some 2)
  ensure "runtime program" (core.unit.program.name == "P")
  ensure "runtime program locals" (core.unit.program.locals.length == 4)
  ensure "runtime body" (core.unit.program.body.length == 11)
  ensure "runtime files" (core.files.size == 1)
  ensure "runtime file name" (core.files[0]? == some "runtime.st")

/-! ## Case tests -/

private def testCaseParsing : IO Unit := do
  let descriptor ← expectOk "runtime case" (parseCoreCase runtimeCaseJson.compress)
  ensure "case name" (descriptor.name == "runtime-p")
  ensure "case entry" (descriptor.entry.kind == "program" && descriptor.entry.name == "P")
  ensure "case cycles" (descriptor.cycles == 1)
  ensure "case fuel" (descriptor.fuel == 10000)
  ensure "case features" (descriptor.features == runtimeFeatures)
  ensure "case expected status" (descriptor.expected.status == "halted")
  ensure "case expected trap" (descriptor.expected.trap.isNone)
  ensure "case expected globals" (descriptor.expected.globals == [105])
  ensure "case expected cycles" (descriptor.expected.cycles == 1)

private def testCaseRejection : IO Unit := do
  let descriptor := runtimeCaseJson
  expectError "case format version"
    (parseCoreCase (descriptor.setObjVal! "formatVersion" (Json.str "0.2.0")).compress)
    "unknown version"
  expectError "case inputs"
    (parseCoreCase (descriptor.setObjVal! "inputs" (Json.arr #[Json.mkObj []])).compress)
    "takes no inputs"
  expectError "case feature order"
    (parseCoreCase (descriptor.setObjVal! "features" (Json.arr #[Json.str "b", Json.str "a"])).compress)
    "strictly sorted"
  expectError "case zero fuel"
    (parseCoreCase (descriptor.setObjVal! "fuel" (0 : Json)).compress)
    "positive fuel"
  let expected := descriptor.getObjValD "expected"
  let haltedWithTrap := descriptor.setObjVal! "expected"
    (expected.setObjVal! "trap" (Json.str "DIV_ZERO"))
  expectError "halted case with trap"
    (parseCoreCase haltedWithTrap.compress)
    "halted case must have a null trap"
  let trapWithoutTrap := descriptor.setObjVal! "expected"
    (expected.setObjVal! "status" (Json.str "trap"))
  expectError "trap case without trap"
    (parseCoreCase trapWithoutTrap.compress)
    "must name its trap"

/-! ## Result tests -/

private def testRuntimeResult : IO Unit := do
  let core ← expectOk "runtime dump" (parseCoreDump runtimeDumpText)
  let descriptor ← expectOk "runtime case" (parseCoreCase runtimeCaseJson.compress)
  let result ← expectOk "runtime run" (runReference core descriptor false)
  ensure "runtime status" (result.status == "halted")
  ensure "runtime value" (result.value.isNone)
  ensure "runtime trap" (result.trap.isNone)
  ensure "runtime globals" (result.globals.map (fun global => global.value) == [105])
  ensure "runtime cycles" (result.cycles == 1)
  ensure "runtime requested cycles" (result.requestedCycles == 1)
  ensure "runtime fuel" (result.fuel == 10000)
  ensure "runtime matches expected" (result.matchesExpected descriptor.expected)
  let traced ← expectOk "runtime trace" (runReference core descriptor true)
  let trace ← match traced.trace with
    | some trace => pure trace
    | none => throw (IO.userError "runtime trace: trace was not recorded")
  ensure "runtime trace nonempty" (!trace.isEmpty)
  ensure "runtime trace first step" (trace.head?.map (·.step) == some 0)
  ensure "runtime trace last focus" (trace.getLast?.map (·.focus) == some "halted")

private def testTrapResult : IO Unit := do
  let core ← expectOk "trap dump" (parseCoreDump trapDumpText)
  let descriptor ← expectOk "trap case" (parseCoreCase trapCaseJson.compress)
  let result ← expectOk "trap run" (runReference core descriptor false)
  ensure "trap status" (result.status == "trap")
  ensure "trap tag"
    (result.trap.map trapTag == some "DIV_ZERO")
  ensure "trap cycles" (result.cycles == 0)
  ensure "trap globals" (result.globals.map (fun global => global.value) == [0])
  ensure "trap matches expected" (result.matchesExpected descriptor.expected)
  let encoded := encodeResult core result
  let trap ← expectOk "trap encoding" (strField encoded "trap")
  ensure "trap encoding tag" (trap == "DIV_ZERO")
  let span ← expectOk "trap span" (encoded.getObjVal? "trapSpan")
  let spanFile ← expectOk "trap span file" (strField span "file")
  ensure "trap span file name" (spanFile == "trap.st")
  let spanStart ← expectOk "trap span start" (natField span "start")
  ensure "trap span start value" (spanStart == 85)

private def testResultEncoding : IO Unit := do
  let core ← expectOk "runtime dump" (parseCoreDump runtimeDumpText)
  let descriptor ← expectOk "runtime case" (parseCoreCase runtimeCaseJson.compress)
  let result ← expectOk "runtime run" (runReference core descriptor false)
  let encoded := encodeResult core result
  let text := encoded.compress
  let parsed ← expectOk "result JSON" (Json.parse text)
  ensure "result format" ((← expectOk "result format" (strField parsed "format")) == coreResultFormat)
  ensure "result format version"
    ((← expectOk "result format version" (strField parsed "formatVersion")) == coreResultFormatVersion)
  ensure "result semantics version"
    ((← expectOk "result semantics version" (strField parsed "semanticsVersion")) == coreDumpSemanticsVersion)
  ensure "result dump version"
    ((← expectOk "result dump version" (strField parsed "dumpFormatVersion")) == coreDumpFormatVersion)
  ensure "result case version"
    ((← expectOk "result case version" (strField parsed "caseFormatVersion")) == coreCaseFormatVersion)
  ensure "result profile"
    ((← expectOk "result profile" (strField parsed "profile")) == coreDumpProfile)
  ensure "result case name" ((← expectOk "result case name" (strField parsed "case")) == "runtime-p")
  ensure "result status" ((← expectOk "result status" (strField parsed "status")) == "halted")
  ensure "result value"
    ((← expectOk "result value" (parsed.getObjVal? "value")).isNull)
  ensure "result trap"
    ((← expectOk "result trap" (parsed.getObjVal? "trap")).isNull)
  ensure "result cycles" ((← expectOk "result cycles" (natField parsed "cycles")) == 1)
  let globals ← expectOk "result globals" (arrField parsed "globals")
  ensure "result global count" (globals.size == 1)
  let global ← arrayAt globals 0 "result globals"
  ensure "result global name" ((← expectOk "result global name" (strField global "name")) == "G")
  ensure "result global value" ((← expectOk "result global value" (intField global "value")) == 105)
  let ty ← expectOk "result global type" (global.getObjVal? "type")
  ensure "result global type width"
    ((← expectOk "result global type width" (natField ty "width")) == 32)
  match parsed.getObjVal? "trace" with
  | .error _ => pure ()
  | .ok _ => throw (IO.userError "result without --trace must omit the trace field")
  ensure "result encoding is deterministic"
    ((encodeResult core result).compress == text)
  let traced ← expectOk "runtime trace" (runReference core descriptor true)
  let tracedEncoded := encodeResult core traced
  let traceArray ← expectOk "encoded trace" (arrField tracedEncoded "trace")
  let trace ← match traced.trace with
    | some trace => pure trace
    | none => throw (IO.userError "encoded trace: trace was not recorded")
  ensure "encoded trace length" (traceArray.size == trace.length)

/-! ## CLI tests -/

private def testCli : IO Unit := do
  IO.FS.withTempDir fun directory => do
    let dumpPath := directory / "dump.json"
    let casePath := directory / "case.json"
    let outputPath := directory / "result.json"
    let tracePath := directory / "trace.json"
    let badDumpPath := directory / "bad-dump.json"
    let badCasePath := directory / "bad-case.json"
    let missingPath := directory / "missing.json"
    IO.FS.writeFile dumpPath runtimeDumpText
    IO.FS.writeFile casePath runtimeCaseJson.compress
    let exit ← run ["--dump", dumpPath.toString, "--case", casePath.toString,
      "--output", outputPath.toString]
    ensure "cli exit" (exit == 0)
    let output ← IO.FS.readFile outputPath
    let parsed ← expectOk "cli output JSON" (Json.parse output)
    ensure "cli status" ((← expectOk "cli status" (strField parsed "status")) == "halted")
    let globals ← expectOk "cli globals" (arrField parsed "globals")
    let global ← arrayAt globals 0 "cli globals"
    ensure "cli global value" ((← expectOk "cli global value" (intField global "value")) == 105)
    let traceExit ← run ["--dump", dumpPath.toString, "--case", casePath.toString,
      "--trace", "--output", tracePath.toString]
    ensure "cli trace exit" (traceExit == 0)
    let traceText ← IO.FS.readFile tracePath
    let traceJson ← expectOk "cli trace JSON" (Json.parse traceText)
    let traceArray ← expectOk "cli trace array" (arrField traceJson "trace")
    ensure "cli trace nonempty" (!traceArray.isEmpty)
    ensure "cli missing case" ((← run ["--dump", dumpPath.toString]) == 2)
    ensure "cli missing file"
      ((← run ["--dump", missingPath.toString, "--case", casePath.toString]) == 2)
    IO.FS.writeFile badDumpPath
      ((minimalDump (literalJson 1)).setObjVal! "formatVersion" (Json.str "0.1.1") |>.compress)
    ensure "cli bad dump"
      ((← run ["--dump", badDumpPath.toString, "--case", casePath.toString]) == 3)
    IO.FS.writeFile badCasePath
      (runtimeCaseJson.setObjVal! "formatVersion" (Json.str "0.2.0")).compress
    ensure "cli bad case"
      ((← run ["--dump", dumpPath.toString, "--case", badCasePath.toString]) == 3)
    ensure "cli help" ((← run ["--help"]) == 0)

/-! ## Entry point -/

/-- Run all `stc-sem` tests, returning `0` on success and `1` on failure. -/
def runReferenceTests : IO UInt32 := do
  let failures ← IO.mkRef #[]
  runTest "reader-minimal" testMinimalReader failures
  runTest "version-rejection" testVersionRejection failures
  runTest "status-rejection" testStatusRejection failures
  runTest "schema-rejection" testSchemaRejection failures
  runTest "comprehensive-reader" testComprehensiveReader failures
  runTest "case-parsing" testCaseParsing failures
  runTest "case-rejection" testCaseRejection failures
  runTest "runtime-result" testRuntimeResult failures
  runTest "trap-result" testTrapResult failures
  runTest "result-encoding" testResultEncoding failures
  runTest "cli" testCli failures
  let failures ← failures.get
  if failures.isEmpty then
    IO.println "stc-sem-tests: OK (11 test groups)"
    pure 0
  else
    for failure in failures do
      IO.eprintln s!"stc-sem-tests: FAIL: {failure}"
    IO.eprintln s!"stc-sem-tests: {failures.size} group(s) failed"
    pure 1

end Verification.Semantics.ReferenceTests
