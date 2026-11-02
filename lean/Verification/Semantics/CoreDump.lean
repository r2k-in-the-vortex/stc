/-
The Lean reader for the frozen `stc-core-hir` `0.1.0` dump (P0.6).

The reader is deliberately strict. It accepts exactly one document shape: the
format family, dump version, semantics version, and profile must match the
constants below; the dump must be accepted (`status: "ok"` with an empty
feature array); every required field must be present; every type, span, place,
expression, statement, and callee must fit the schema; declaration-scoped
variable positions and names must resolve; function-call targets must resolve
and their arity must match the target signature. A rejected dump is refused
rather than interpreted, and an unknown version or an unmodeled construct is an
error rather than a best-effort reading.

The schema is specified in `specification/Verification/HIR-Dump.md`. The reader
returns the HIR unit plus the source-file table used to report trap spans; the
table is provenance, not identity.
-/
import Lean.Data.Json
import Stc.Hir

namespace Verification.Semantics

open Lean (Json)
open Stc.Hir
open Stc.Syntax

/-! ## Format constants -/

/-- The dump format family understood by this reader. -/
def coreDumpFormat : String := "stc-core-hir"

/-- The exact dump format version understood by this reader. -/
def coreDumpFormatVersion : String := "0.1.0"

/-- The exact semantics version understood by this reader. -/
def coreDumpSemanticsVersion : String := "1.0-draft"

/-- The exact conformance profile understood by this reader. -/
def coreDumpProfile : String := "stc-2013-oop-utf8-v1"

/-- The maximum nesting depth accepted by the reader. -/
def coreDumpMaxDepth : Nat := 512

/-! ## Result -/

/-- A successfully read core unit and its source-file table. -/
structure CoreUnit where
  /-- The parsed core HIR unit. -/
  unit : HirUnit
  /-- The GOT key of each global, in the same order as `unit.globals`. -/
  globalKeys : Array String
  /-- Source paths by `FileId.id`; the spans in `unit` index this array. -/
  files : Array String

/-- The source path of a span, or `""` when the table has no entry. -/
def CoreUnit.fileOf (core : CoreUnit) (span : Span) : String :=
  core.files[span.file.id]?.getD ""

/-! ## Parser monad and JSON helpers -/

/-- Parse errors are strings; the state is the source-file interning table. -/
private abbrev ParseM := StateT (Array String) (Except String)

private def fail (message : String) : ParseM α := throw message

private def jsonField (j : Json) (name ctx : String) : ParseM Json := do
  match j.getObjVal? name with
  | .ok value => pure value
  | .error _ => fail s!"{ctx}.{name}: required field is missing"

private def jsonString (j : Json) (name ctx : String) : ParseM String := do
  let value ← jsonField j name ctx
  match value.getStr? with
  | .ok text => pure text
  | .error _ => fail s!"{ctx}.{name}: expected a string"

private def jsonNat (j : Json) (name ctx : String) : ParseM Nat := do
  let value ← jsonField j name ctx
  match value.getNat? with
  | .ok number => pure number
  | .error _ => fail s!"{ctx}.{name}: expected a non-negative integer"

private def jsonInt (j : Json) (name ctx : String) : ParseM Int := do
  let value ← jsonField j name ctx
  match value.getInt? with
  | .ok number => pure number
  | .error _ => fail s!"{ctx}.{name}: expected an integer"

private def jsonBool (j : Json) (name ctx : String) : ParseM Bool := do
  let value ← jsonField j name ctx
  match value.getBool? with
  | .ok flag => pure flag
  | .error _ => fail s!"{ctx}.{name}: expected a boolean"

private def jsonArray (j : Json) (name ctx : String) : ParseM (Array Json) := do
  let value ← jsonField j name ctx
  match value.getArr? with
  | .ok items => pure items
  | .error _ => fail s!"{ctx}.{name}: expected an array"

private def valueInt (j : Json) (ctx : String) : ParseM Int := do
  match j.getInt? with
  | .ok number => pure number
  | .error _ => fail s!"{ctx}: expected an integer"

private def mapJsonArray (items : Array Json) (parse : Nat → Json → ParseM α) :
    ParseM (List α) := do
  let mut result : Array α := #[]
  for index in [0:items.size] do
    result := result.push (← parse index items[index]!)
  pure result.toList

/-! ## Canonical names and packages -/

private def isUpperAscii (c : Char) : Bool := 'A' ≤ c && c ≤ 'Z'

private def isDigitAscii (c : Char) : Bool := '0' ≤ c && c ≤ '9'

private def isIdentStart (c : Char) : Bool := isUpperAscii c || c == '_'

private def isIdentRest (c : Char) : Bool := isIdentStart c || isDigitAscii c

private def checkName (name : String) (ctx : String) (allowPath : Bool) : ParseM Unit := do
  let parts := if allowPath then name.splitOn "." else [name]
  for part in parts do
    match part.toList with
    | [] => fail s!"{ctx}: name `{name}` has an empty segment"
    | first :: rest =>
        if !isIdentStart first then
          fail s!"{ctx}: name `{name}` is not uppercase canonical"
        if !rest.all isIdentRest then
          fail s!"{ctx}: name `{name}` is not uppercase canonical"
  if parts.isEmpty then
    fail s!"{ctx}: name is empty"

private def checkPackage (package : String) (ctx : String) : ParseM Unit := do
  match package.toList with
  | [] => fail s!"{ctx}: package is empty"
  | first :: rest =>
      if !isIdentStart first then
        fail s!"{ctx}: package `{package}` is not canonical"
      if !rest.all (fun c => isIdentRest c || c == '-') then
        fail s!"{ctx}: package `{package}` is not canonical"

/-! ## Source files and spans -/

private def internFile (file : String) : ParseM FileId := do
  let files ← get
  match files.findIdx? (fun existing => existing == file) with
  | some index => pure ⟨index⟩
  | none =>
      set (files.push file)
      pure ⟨files.size⟩

private def parseSpan (j : Json) (ctx : String) : ParseM Span := do
  let file ← jsonString j "file" ctx
  let start ← jsonNat j "start" ctx
  let stop ← jsonNat j "end" ctx
  if start > stop then
    fail s!"{ctx}: span start {start} is after end {stop}"
  let file ← internFile file
  pure { file := file, start := start, stop := stop }

private def parseSpanField (j : Json) (name ctx : String) : ParseM Span := do
  parseSpan (← jsonField j name ctx) s!"{ctx}.{name}"

/-! ## Types -/

private def parseTy (j : Json) (ctx : String) : ParseM Ty := do
  let kind ← jsonString j "kind" ctx
  match kind with
  | "bool" => pure .bool
  | "int" =>
      let width ← jsonNat j "width" ctx
      let signed ← jsonBool j "signed" ctx
      if !(width == 8 || width == 16 || width == 32 || width == 64) then
        fail s!"{ctx}: non-core integer width {width}"
      pure (.int width signed)
  | "bits" =>
      let width ← jsonNat j "width" ctx
      if !(width == 8 || width == 16 || width == 32 || width == 64) then
        fail s!"{ctx}: non-core bit-string width {width}"
      pure (.bits width)
  | "char" =>
      let charKind ← jsonString j "charKind" ctx
      match charKind with
      | "char" => pure (.char .char)
      | "wchar" => pure (.char .wchar)
      | "uchar" => pure (.char .uchar)
      | other => fail s!"{ctx}: unknown character kind `{other}`"
  | "unsupported" =>
      let feature ← jsonString j "feature" ctx
      fail s!"{ctx}: unsupported type node (feature `{feature}`)"
  | other => fail s!"{ctx}: unknown type kind `{other}`"

private def parseTyField (j : Json) (name ctx : String) : ParseM Ty := do
  parseTy (← jsonField j name ctx) s!"{ctx}.{name}"

/-! ## Declaration scope -/

/-- The declaration names visible from a function, program, or initializer. -/
private structure Scope where
  package : String
  params : Array String
  locals : Array String
  globals : Array String
  result : Option String

private def parseVarRef (j : Json) (scope : Scope) (ctx : String) : ParseM (VarId × Span) := do
  let kind ← jsonString j "varKind" ctx
  let name ← jsonString j "name" ctx
  checkName name s!"{ctx}.name" true
  let id ← match kind with
    | "param" =>
        let position ← jsonNat j "position" ctx
        match scope.params[position]? with
        | some expected =>
            if expected != name then
              fail s!"{ctx}: parameter {position} is `{expected}`, not `{name}`"
            pure (.param position)
        | none =>
            fail s!"{ctx}: parameter {position} is outside the {scope.params.size} parameter(s)"
    | "local" =>
        let position ← jsonNat j "position" ctx
        match scope.locals[position]? with
        | some expected =>
            if expected != name then
              fail s!"{ctx}: local {position} is `{expected}`, not `{name}`"
            pure (.local position)
        | none =>
            fail s!"{ctx}: local {position} is outside the {scope.locals.size} local(s)"
    | "global" =>
        let position ← jsonNat j "position" ctx
        match scope.globals[position]? with
        | some expected =>
            if expected != name then
              fail s!"{ctx}: global {position} is `{expected}`, not `{name}`"
            let key ← jsonString j "gotKey" ctx
            let expectedKey := scope.package ++ "::" ++ name
            if key != expectedKey then
              fail s!"{ctx}: GOT key `{key}` is not `{expectedKey}`"
            pure (.global position)
        | none =>
            fail s!"{ctx}: global {position} is outside the {scope.globals.size} global(s)"
    | "result" =>
        match scope.result with
        | some expected =>
            if expected != name then
              fail s!"{ctx}: result variable is `{expected}`, not `{name}`"
            pure .result
        | none => fail s!"{ctx}: result variable `{name}` does not exist in this scope"
    | other => fail s!"{ctx}: unknown variable kind `{other}`"
  let span ← parseSpanField j "span" ctx
  pure (id, span)

private def parsePlace (j : Json) (scope : Scope) (ctx : String) : ParseM Place := do
  let kind ← jsonString j "kind" ctx
  match kind with
  | "var" =>
      let (id, span) ← parseVarRef j scope ctx
      pure ⟨id, span⟩
  | "unsupported" =>
      let feature ← jsonString j "feature" ctx
      fail s!"{ctx}: unsupported place (feature `{feature}`)"
  | other => fail s!"{ctx}: unknown place kind `{other}`"

/-! ## Function signatures -/

/-- The part of a function declaration needed to resolve call targets. -/
private structure FunctionSig where
  name : String
  arity : Nat
  hasResult : Bool

private def functionIndex? (functions : Array FunctionSig) (name : String) : Option Nat :=
  functions.findIdx? (fun function => function.name == name)

/-! ## Expressions -/

private def parseUnaryOp (name : String) (ctx : String) : ParseM UnaryOp :=
  match name with
  | "neg" => pure .neg
  | "pos" => pure .pos
  | "not" => pure .not
  | other => fail s!"{ctx}: unknown unary operator `{other}`"

private def parseBinaryOp (name : String) (ctx : String) : ParseM BinaryOp :=
  match name with
  | "add" => pure .add
  | "sub" => pure .sub
  | "mul" => pure .mul
  | "div" => pure .div
  | "mod" => pure .mod
  | "pow" => pure .pow
  | "and" => pure .and
  | "or" => pure .or
  | "xor" => pure .xor
  | "eq" => pure .eq
  | "ne" => pure .ne
  | "lt" => pure .lt
  | "le" => pure .le
  | "gt" => pure .gt
  | "ge" => pure .ge
  | other => fail s!"{ctx}: unknown binary operator `{other}`"

private def parseCastKind (name : String) (ctx : String) : ParseM CastKind :=
  match name with
  | "implicit" => pure .implicit
  | "explicit" => pure .explicit
  | other => fail s!"{ctx}: unknown cast kind `{other}`"

private def stdArityAccepts (fn : StdFn) (arity : Nat) : Bool :=
  match fn with
  | .abs => arity == 1
  | .sel => arity == 3
  | .min => arity == 2
  | .max => arity == 2
  | .limit => arity == 3
  | .mux => arity >= 2

private def parseStdFn (name : String) (ctx : String) : ParseM StdFn :=
  match name with
  | "ABS" => pure .abs
  | "SEL" => pure .sel
  | "MIN" => pure .min
  | "MAX" => pure .max
  | "LIMIT" => pure .limit
  | "MUX" => pure .mux
  | other => fail s!"{ctx}: non-core standard function `{other}`"

private def parseCallee (j : Json) (functions : Array FunctionSig) (ctx : String) :
    ParseM Callee := do
  let kind ← jsonString j "kind" ctx
  let span ← parseSpanField j "span" ctx
  match kind with
  | "function" =>
      let name ← jsonString j "name" ctx
      checkName name s!"{ctx}.name" true
      match functionIndex? functions name with
      | some index => pure (.function index span)
      | none => fail s!"{ctx}: call target `{name}` is not declared in the dump"
  | "std" =>
      let name ← jsonString j "name" ctx
      pure (.std (← parseStdFn name s!"{ctx}.name") span)
  | other => fail s!"{ctx}: unknown callee kind `{other}`"

private def checkDepth (depth : Nat) (ctx : String) : ParseM Unit :=
  if depth > coreDumpMaxDepth then
    fail s!"{ctx}: exceeds the nesting limit of {coreDumpMaxDepth}"
  else
    pure ()

mutual
  /-- Parse an expression. -/
  private partial def parseExpr (j : Json) (scope : Scope) (functions : Array FunctionSig)
      (depth : Nat) (ctx : String) : ParseM Expr := do
    checkDepth depth ctx
    let kind ← jsonString j "kind" ctx
    match kind with
    | "literal" =>
        let ty ← parseTyField j "type" ctx
        let value ← jsonInt j "value" ctx
        let literal : Literal := ⟨ty, value⟩
        if !literal.wellFormedB then
          fail s!"{ctx}: literal value {value} is not representable in its declared type"
        let span ← parseSpanField j "span" ctx
        pure (.literal literal span)
    | "var" =>
        let (id, span) ← parseVarRef j scope ctx
        pure (.var id span)
    | "unary" =>
        let op ← parseUnaryOp (← jsonString j "op" ctx) s!"{ctx}.op"
        let operand ← parseExpr (← jsonField j "operand" ctx) scope functions
          (depth + 1) s!"{ctx}.operand"
        let _ ← parseTyField j "type" ctx
        let span ← parseSpanField j "span" ctx
        pure (.unary op operand span)
    | "binary" =>
        let op ← parseBinaryOp (← jsonString j "op" ctx) s!"{ctx}.op"
        let lhs ← parseExpr (← jsonField j "lhs" ctx) scope functions
          (depth + 1) s!"{ctx}.lhs"
        let rhs ← parseExpr (← jsonField j "rhs" ctx) scope functions
          (depth + 1) s!"{ctx}.rhs"
        let _ ← parseTyField j "type" ctx
        let span ← parseSpanField j "span" ctx
        pure (.binary op lhs rhs span)
    | "call" =>
        let callee ← parseCallee (← jsonField j "callee" ctx) functions s!"{ctx}.callee"
        let argsJson ← jsonArray j "args" ctx
        let args ← mapJsonArray argsJson fun index argument =>
          parseExpr argument scope functions (depth + 1) s!"{ctx}.args[{index}]"
        match callee with
        | .function index _ =>
            match functions[index]? with
            | some signature =>
                if args.length != signature.arity then
                  fail s!"{ctx}: call passes {args.length} argument(s) to `{signature.name}`, which has {signature.arity}"
            | none => fail s!"{ctx}: function index {index} is outside the function table"
        | .std fn _ =>
            unless stdArityAccepts fn args.length do
              fail s!"{ctx}: standard call has an unsupported arity {args.length}"
        let _ ← parseTyField j "type" ctx
        let span ← parseSpanField j "span" ctx
        pure (.call callee args span)
    | "cast" =>
        let operand ← parseExpr (← jsonField j "operand" ctx) scope functions
          (depth + 1) s!"{ctx}.operand"
        let to ← parseTyField j "to" ctx
        let castKind ← parseCastKind (← jsonString j "castKind" ctx) s!"{ctx}.castKind"
        let _ ← parseTyField j "type" ctx
        let span ← parseSpanField j "span" ctx
        pure (.cast operand to castKind span)
    | "unsupported" =>
        let feature ← jsonString j "feature" ctx
        fail s!"{ctx}: unsupported expression (feature `{feature}`)"
    | other => fail s!"{ctx}: unknown expression kind `{other}`"
end

/-! ## Statements -/

private def parseCaseLabel (j : Json) (ctx : String) : ParseM CaseLabel := do
  let low ← jsonInt j "low" ctx
  let highJson ← jsonField j "high" ctx
  let high ←
    if highJson.isNull then
      pure none
    else
      pure (some (← valueInt highJson s!"{ctx}.high"))
  let span ← parseSpanField j "span" ctx
  pure ⟨low, high, span⟩

mutual
  /-- Parse one statement. -/
  private partial def parseStmt (j : Json) (scope : Scope) (functions : Array FunctionSig)
      (depth : Nat) (ctx : String) : ParseM Stmt := do
    checkDepth depth ctx
    let kind ← jsonString j "kind" ctx
    match kind with
    | "assign" =>
        let place ← parsePlace (← jsonField j "place" ctx) scope s!"{ctx}.place"
        let value ← parseExpr (← jsonField j "value" ctx) scope functions
          (depth + 1) s!"{ctx}.value"
        let span ← parseSpanField j "span" ctx
        pure (.assign place value span)
    | "call" =>
        let call ← parseExpr (← jsonField j "call" ctx) scope functions
          (depth + 1) s!"{ctx}.call"
        match call with
        | .call .. => pure ()
        | _ => fail s!"{ctx}.call: a statement call must be a call expression"
        let span ← parseSpanField j "span" ctx
        pure (.call call span)
    | "return" =>
        pure (.return_ (← parseSpanField j "span" ctx))
    | "exit" =>
        pure (.exit (← parseSpanField j "span" ctx))
    | "continue" =>
        pure (.continue_ (← parseSpanField j "span" ctx))
    | "empty" =>
        pure (.empty (← parseSpanField j "span" ctx))
    | "if" =>
        let armsJson ← jsonArray j "arms" ctx
        let arms ← mapJsonArray armsJson fun index arm =>
          parseIfArm arm scope functions (depth + 1) s!"{ctx}.arms[{index}]"
        let elseBody ← parseStmtList (← jsonArray j "else" ctx) scope functions
          (depth + 1) s!"{ctx}.else"
        let span ← parseSpanField j "span" ctx
        pure (.ifThenElse arms elseBody span)
    | "case" =>
        let selector ← parseExpr (← jsonField j "selector" ctx) scope functions
          (depth + 1) s!"{ctx}.selector"
        let armsJson ← jsonArray j "arms" ctx
        let arms ← mapJsonArray armsJson fun index arm =>
          parseCaseArm arm scope functions (depth + 1) s!"{ctx}.arms[{index}]"
        let elseBody ← parseStmtList (← jsonArray j "else" ctx) scope functions
          (depth + 1) s!"{ctx}.else"
        let span ← parseSpanField j "span" ctx
        pure (.case selector arms elseBody span)
    | "for" =>
        let var ← parsePlace (← jsonField j "var" ctx) scope s!"{ctx}.var"
        let start ← parseExpr (← jsonField j "start" ctx) scope functions
          (depth + 1) s!"{ctx}.start"
        let stop ← parseExpr (← jsonField j "stop" ctx) scope functions
          (depth + 1) s!"{ctx}.stop"
        let stepJson ← jsonField j "step" ctx
        if stepJson.isNull then
          fail s!"{ctx}.step: the default FOR step must be materialized as an expression"
        let step ← parseExpr stepJson scope functions (depth + 1) s!"{ctx}.step"
        let body ← parseStmtList (← jsonArray j "body" ctx) scope functions
          (depth + 1) s!"{ctx}.body"
        let span ← parseSpanField j "span" ctx
        pure (.forLoop var start stop (some step) body span)
    | "while" =>
        let cond ← parseExpr (← jsonField j "cond" ctx) scope functions
          (depth + 1) s!"{ctx}.cond"
        let body ← parseStmtList (← jsonArray j "body" ctx) scope functions
          (depth + 1) s!"{ctx}.body"
        let span ← parseSpanField j "span" ctx
        pure (.whileLoop cond body span)
    | "repeat" =>
        let body ← parseStmtList (← jsonArray j "body" ctx) scope functions
          (depth + 1) s!"{ctx}.body"
        let cond ← parseExpr (← jsonField j "cond" ctx) scope functions
          (depth + 1) s!"{ctx}.cond"
        let span ← parseSpanField j "span" ctx
        pure (.repeatLoop body cond span)
    | "unsupported" =>
        let feature ← jsonString j "feature" ctx
        fail s!"{ctx}: unsupported statement (feature `{feature}`)"
    | other => fail s!"{ctx}: unknown statement kind `{other}`"

  /-- Parse a statement list. -/
  private partial def parseStmtList (items : Array Json) (scope : Scope)
      (functions : Array FunctionSig) (depth : Nat) (ctx : String) : ParseM (List Stmt) :=
    mapJsonArray items fun index stmt =>
      parseStmt stmt scope functions (depth + 1) s!"{ctx}[{index}]"

  /-- Parse one `IF`/`ELSIF` arm. -/
  private partial def parseIfArm (j : Json) (scope : Scope) (functions : Array FunctionSig)
      (depth : Nat) (ctx : String) : ParseM IfArm := do
    checkDepth depth ctx
    let cond ← parseExpr (← jsonField j "cond" ctx) scope functions
      (depth + 1) s!"{ctx}.cond"
    let body ← parseStmtList (← jsonArray j "body" ctx) scope functions
      (depth + 1) s!"{ctx}.body"
    let span ← parseSpanField j "span" ctx
    pure (IfArm.mk cond body span)

  /-- Parse one `CASE` arm. -/
  private partial def parseCaseArm (j : Json) (scope : Scope) (functions : Array FunctionSig)
      (depth : Nat) (ctx : String) : ParseM CaseArm := do
    checkDepth depth ctx
    let labelsJson ← jsonArray j "labels" ctx
    let labels ← mapJsonArray labelsJson fun index label =>
      parseCaseLabel label s!"{ctx}.labels[{index}]"
    let body ← parseStmtList (← jsonArray j "body" ctx) scope functions
      (depth + 1) s!"{ctx}.body"
    let span ← parseSpanField j "span" ctx
    pure (CaseArm.mk labels body span)
end

/-! ## Declarations -/

private def parseParam (j : Json) (ctx : String) : ParseM Param := do
  let name ← jsonString j "name" ctx
  checkName name s!"{ctx}.name" false
  let ty ← parseTyField j "type" ctx
  let span ← parseSpanField j "span" ctx
  pure ⟨name, ty, span⟩

private def parseLocal (j : Json) (scope : Scope) (functions : Array FunctionSig)
    (ctx : String) : ParseM Local := do
  let name ← jsonString j "name" ctx
  checkName name s!"{ctx}.name" false
  let ty ← parseTyField j "type" ctx
  let initJson ← jsonField j "init" ctx
  let init ←
    if initJson.isNull then
      pure none
    else
      pure (some (← parseExpr initJson scope functions 1 s!"{ctx}.init"))
  let span ← parseSpanField j "span" ctx
  pure ⟨name, ty, init, span⟩

private def parseGlobal (j : Json) (scope : Scope) (functions : Array FunctionSig)
    (ctx : String) : ParseM (Global × String) := do
  let name ← jsonString j "name" ctx
  checkName name s!"{ctx}.name" true
  let key ← jsonString j "gotKey" ctx
  let expectedKey := scope.package ++ "::" ++ name
  if key != expectedKey then
    fail s!"{ctx}.gotKey: `{key}` is not `{expectedKey}`"
  let ty ← parseTyField j "type" ctx
  let initJson ← jsonField j "init" ctx
  let init ←
    if initJson.isNull then
      pure none
    else
      pure (some (← parseExpr initJson scope functions 1 s!"{ctx}.init"))
  let span ← parseSpanField j "span" ctx
  pure (⟨name, ty, init, span⟩, key)

private def duplicateName? (names : Array String) : Option String :=
  let rec go (seen : Array String) (index : Nat) : Option String :=
    if index >= names.size then
      none
    else
      let name := names[index]!
      if seen.any (fun existing => existing == name) then
        some name
      else
        go (seen.push name) (index + 1)
  go #[] 0

private def parseFunction (j : Json) (package : String) (globals : Array String)
    (functions : Array FunctionSig) (ctx : String) : ParseM Function := do
  let name ← jsonString j "name" ctx
  checkName name s!"{ctx}.name" true
  let resultJson ← jsonField j "result" ctx
  let resultType ←
    if resultJson.isNull then
      pure none
    else
      pure (some (← parseTy resultJson s!"{ctx}.result"))
  let paramsJson ← jsonArray j "params" ctx
  let paramNames ← mapJsonArray paramsJson fun index param =>
    jsonString param "name" s!"{ctx}.params[{index}]"
  if let some duplicate := duplicateName? paramNames.toArray then
    fail s!"{ctx}.params: duplicate parameter name `{duplicate}`"
  let params ← mapJsonArray paramsJson fun index param =>
    parseParam param s!"{ctx}.params[{index}]"
  let localsJson ← jsonArray j "locals" ctx
  let localNames ← mapJsonArray localsJson fun index localJson =>
    jsonString localJson "name" s!"{ctx}.locals[{index}]"
  if let some duplicate := duplicateName? localNames.toArray then
    fail s!"{ctx}.locals: duplicate local name `{duplicate}`"
  let scope : Scope := {
    package
    params := paramNames.toArray
    locals := localNames.toArray
    globals
    result := if resultType.isSome then some name else none
  }
  let locals ← mapJsonArray localsJson fun index localJson =>
    parseLocal localJson scope functions s!"{ctx}.locals[{index}]"
  let body ← parseStmtList (← jsonArray j "body" ctx) scope functions 1 s!"{ctx}.body"
  let span ← parseSpanField j "span" ctx
  pure ⟨name, resultType, params, locals, body, span⟩

private def parseProgram (j : Json) (package : String) (globals : Array String)
    (functions : Array FunctionSig) (ctx : String) : ParseM Program := do
  let name ← jsonString j "name" ctx
  checkName name s!"{ctx}.name" true
  let localsJson ← jsonArray j "locals" ctx
  let localNames ← mapJsonArray localsJson fun index localJson =>
    jsonString localJson "name" s!"{ctx}.locals[{index}]"
  if let some duplicate := duplicateName? localNames.toArray then
    fail s!"{ctx}.locals: duplicate field name `{duplicate}`"
  let scope : Scope := {
    package
    params := #[]
    locals := localNames.toArray
    globals
    result := none
  }
  let locals ← mapJsonArray localsJson fun index localJson =>
    parseLocal localJson scope functions s!"{ctx}.locals[{index}]"
  let body ← parseStmtList (← jsonArray j "body" ctx) scope functions 1 s!"{ctx}.body"
  let span ← parseSpanField j "span" ctx
  pure ⟨name, locals, body, span⟩

/-! ## Root -/

private def parseFunctionSignatures (functionsJson : Array Json) : ParseM (Array FunctionSig) := do
  let mut signatures : Array FunctionSig := #[]
  for index in [0:functionsJson.size] do
    let function := functionsJson[index]!
    let ctx := s!"unit.functions[{index}]"
    let name ← jsonString function "name" ctx
    checkName name s!"{ctx}.name" true
    if (functionIndex? signatures name).isSome then
      fail s!"{ctx}: duplicate function name `{name}`"
    let paramsJson ← jsonArray function "params" ctx
    let resultJson ← jsonField function "result" ctx
    signatures := signatures.push {
      name
      arity := paramsJson.size
      hasResult := !resultJson.isNull
    }
  pure signatures

private def parseGlobalNames (globalsJson : Array Json) : ParseM (Array String) := do
  let mut names : Array String := #[]
  for index in [0:globalsJson.size] do
    let ctx := s!"unit.globals[{index}]"
    let name ← jsonString globalsJson[index]! "name" ctx
    checkName name s!"{ctx}.name" true
    if names.any (fun existing => existing == name) then
      fail s!"{ctx}: duplicate global name `{name}`"
    names := names.push name
  pure names

private def parseRoot (root : Json) : ParseM (HirUnit × Array String) := do
  let format ← jsonString root "format" "dump"
  if format != coreDumpFormat then
    fail s!"dump.format: unknown format `{format}` (expected `{coreDumpFormat}`)"
  let formatVersion ← jsonString root "formatVersion" "dump"
  if formatVersion != coreDumpFormatVersion then
    fail s!"dump.formatVersion: unknown version `{formatVersion}` (expected `{coreDumpFormatVersion}`)"
  let semanticsVersion ← jsonString root "semanticsVersion" "dump"
  if semanticsVersion != coreDumpSemanticsVersion then
    fail s!"dump.semanticsVersion: unknown version `{semanticsVersion}` (expected `{coreDumpSemanticsVersion}`)"
  let profile ← jsonString root "profile" "dump"
  if profile != coreDumpProfile then
    fail s!"dump.profile: unknown profile `{profile}` (expected `{coreDumpProfile}`)"
  let package ← jsonString root "package" "dump"
  checkPackage package "dump.package"
  let status ← jsonString root "status" "dump"
  if status != "ok" && status != "rejected" then
    fail s!"dump.status: unknown status `{status}` (expected `ok` or `rejected`)"
  let featuresJson ← jsonArray root "features" "dump"
  let mut featureTags : Array String := #[]
  for index in [0:featuresJson.size] do
    let feature := featuresJson[index]!
    let ctx := s!"dump.features[{index}]"
    let tag ← jsonString feature "tag" ctx
    let _ ← jsonString feature "detail" ctx
    let atJson ← jsonField feature "at" ctx
    if !atJson.isNull then
      let _ ← parseSpan atJson s!"{ctx}.at"
      pure ()
    featureTags := featureTags.push tag
  let expectedStatus := if featureTags.isEmpty then "ok" else "rejected"
  if status != expectedStatus then
    fail s!"dump.status: `{status}` does not match the {featureTags.size} feature tag(s) (expected `{expectedStatus}`)"
  if status == "rejected" then
    fail s!"dump: rejected core dump ({featureTags.toList}); refusing to interpret it"
  let unitJson ← jsonField root "unit" "dump"
  let globalsJson ← jsonArray unitJson "globals" "unit"
  let functionsJson ← jsonArray unitJson "functions" "unit"
  let programJson ← jsonField unitJson "program" "unit"
  if programJson.isNull then
    fail "unit.program: an accepted core unit must contain exactly one program"
  let signatures ← parseFunctionSignatures functionsJson
  let globalNames ← parseGlobalNames globalsJson
  let globalScope : Scope := {
    package
    params := #[]
    locals := #[]
    globals := globalNames
    result := none
  }
  let globals ← mapJsonArray globalsJson fun index global =>
    parseGlobal global globalScope signatures s!"unit.globals[{index}]"
  let functions ← mapJsonArray functionsJson fun index function =>
    parseFunction function package globalNames signatures s!"unit.functions[{index}]"
  let program ← parseProgram programJson package globalNames signatures "unit.program"
  let unitSpan ← parseSpanField unitJson "span" "unit"
  pure ({
    globals := globals.map Prod.fst
    functions
    program
    span := unitSpan
  }, (globals.map Prod.snd).toArray)

/-- Parse and validate a `stc-core-hir` `0.1.0` dump. -/
def parseCoreDump (text : String) : Except String CoreUnit := do
  let json ← Json.parse text
  let ((unit, globalKeys), files) ← (parseRoot json).run #[]
  pure { unit, globalKeys, files }

end Verification.Semantics
