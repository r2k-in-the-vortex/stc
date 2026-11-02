/-
`lake exe semantics-conformance`: the P0.6 core-corpus differential harness.

The harness is the V0 differential-evidence producer for the core subset. It
does not prove a refinement: it runs three independently implemented paths on
each corpus case and reports whether their observable results agree.

For one case directory `tests/verification/semantics/<case>/` containing
`case.json` and one or more `.st` sources, the harness:

1. runs `stc hir-dump` over the sources and reads the frozen `stc-core-hir`
   `0.1.0` dump with the strict reader;
2. runs `stc-sem --dump ... --case ...` and reads the `stc-sem-result` `0.1.0`
   result;
3. builds a runnable artifact with `stc build --emit=exe`, runs it with the
   case's cycle count and the test-host `--snapshot` surface, and reads the
   `stc-runtime-snapshot` document;
4. maps the dump's globals, in declaration order, to their GOT roots in the
   snapshot and compares `status`, `trap`, `cycles`, and the global values of
   the reference result and the artifact against the descriptor's `expected`;
5. emits one `stc-sem-conformance-report` `0.1.0` object on stdout (and to
   `--report <path>` when given) with the semantics, dump, case, result, and
   snapshot versions and each case's feature tags.

The report is machine-readable; diagnostics and per-case failures go to
stderr. Exit statuses are `0` for agreement, `1` for a case failure, `2` for a
usage or tool failure, and `3` for an empty corpus. An empty corpus is never
reported as agreement: the harness fails closed, so a case can never be added
to a harness that does not run it.

# Tool discovery

`--stc`, `--stc-sem`, `--runtime`, and `--clang` select the Rust compiler, the
reference interpreter, the static runtime library, and the C toolchain.
Without an option, the harness uses `STC`, `STC_SEM`, `STC_RUNTIME`, and
`STC_CLANG`, then known build paths relative to the `lean/` directory, then a
`PATH` probe for the two programs. The runtime library has no `PATH` probe. On
Windows the clang fallback is the LLVM install under `ProgramFiles`, because a
`lake exe` run puts the Lean toolchain's MinGW-targeting clang first on `PATH`,
and that clang cannot link the MSVC runtime library.

# Test-only restriction

`--snapshot` is the opt-in test-host surface. It reads every root in the
active manifest through `Host::debug_read`, including protected `RETAIN` roots
and `AT` direct variables, without the audited `observe` authorization the
debug protocol requires. It is therefore only reachable by passing the flag to
a compiled artifact, never exposed as a production debug service, and it never
writes, forces, or retargets a root.
-/
import Lean.Data.Json
import Verification.Semantics.Reference

namespace Verification.Semantics

open Lean (Json JsonNumber)
open Stc.Hir

/-! ## Format and contract constants -/

/-- Default corpus location, relative to the `lean/` directory. -/
def defaultCorpus : System.FilePath := "../tests/verification/semantics"

/-- The conformance-report format family. -/
def conformanceReportFormat : String := "stc-sem-conformance-report"

/-- The exact conformance-report version emitted by this harness. -/
def conformanceReportFormatVersion : String := "0.1.0"

/-- The snapshot format family emitted by the compiled artifact's test host. -/
def snapshotFormat : String := "stc-runtime-snapshot"

/-- The exact snapshot version understood by this harness. -/
def snapshotFormatVersion : String := "0.1.0"

/-- The evidence label; agreement is differential evidence, not a refinement proof. -/
def conformanceEvidence : String :=
  "V0 differential evidence: the frozen core dump, the reference interpreter, and the compiled artifact agree on every case; this is not a refinement proof"

/-- Exit status used while the corpus is empty: the harness is fail-closed. -/
def emptyCorpusExit : UInt32 := 3

/-- Exit status for an invalid command line or a missing tool. -/
def usageExit : UInt32 := 2

/-- Exit status when at least one case disagrees. -/
def failureExit : UInt32 := 1

/-- The command-line usage line. -/
def usageLine : String :=
  "usage: semantics-conformance [--corpus DIR] [--stc PATH] [--stc-sem PATH]\n" ++
  "                             [--runtime PATH] [--clang PATH] [--report PATH]"

/-! ## Corpus layout -/

/-- The `case.json` descriptors directly under the corpus root, sorted by path. -/
def corpusCases (root : System.FilePath) : IO (Array System.FilePath) := do
  unless ← root.pathExists do
    return #[]
  let entries ← root.readDir
  let mut cases := #[]
  for entry in entries do
    let descriptor := entry.path / "case.json"
    if ← descriptor.pathExists then
      cases := cases.push descriptor
  return cases.qsort (fun left right => left.toString < right.toString)

/-- The `.st` sources of a case directory, sorted by path. -/
private def caseSources (directory : System.FilePath) : IO (Array System.FilePath) := do
  let entries ← directory.readDir
  let mut sources := #[]
  for entry in entries do
    if entry.path.extension == some "st" then
      sources := sources.push entry.path
  return sources.qsort (fun left right => left.toString < right.toString)

/-- Makes a relative path absolute against the process working directory. -/
private def absolutize (path : System.FilePath) : IO System.FilePath := do
  if path.isAbsolute then
    pure path
  else
    pure ((← IO.currentDir) / path)

/-! ## Command line -/

/-- Parsed command-line options. -/
private structure Options where
  corpus : Option String := none
  stc : Option String := none
  stcSem : Option String := none
  runtime : Option String := none
  clang : Option String := none
  report : Option String := none
  help : Bool := false

private def setOnce (current : Option String) (value : String) (flag : String) :
    Except String (Option String) := do
  if current.isSome then
    throw s!"{flag} is given more than once"
  pure (some value)

private partial def parseOptionsFrom (options : Options) : List String → Except String Options
  | [] => pure options
  | "--corpus" :: value :: rest => do
      let corpus ← setOnce options.corpus value "--corpus"
      parseOptionsFrom { options with corpus } rest
  | ["--corpus"] => throw "--corpus requires a path"
  | "--stc" :: value :: rest => do
      let stc ← setOnce options.stc value "--stc"
      parseOptionsFrom { options with stc } rest
  | ["--stc"] => throw "--stc requires a path"
  | "--stc-sem" :: value :: rest => do
      let stcSem ← setOnce options.stcSem value "--stc-sem"
      parseOptionsFrom { options with stcSem } rest
  | ["--stc-sem"] => throw "--stc-sem requires a path"
  | "--runtime" :: value :: rest => do
      let runtime ← setOnce options.runtime value "--runtime"
      parseOptionsFrom { options with runtime } rest
  | ["--runtime"] => throw "--runtime requires a path"
  | "--clang" :: value :: rest => do
      let clang ← setOnce options.clang value "--clang"
      parseOptionsFrom { options with clang } rest
  | ["--clang"] => throw "--clang requires a path"
  | "--report" :: value :: rest => do
      let report ← setOnce options.report value "--report"
      parseOptionsFrom { options with report } rest
  | ["--report"] => throw "--report requires a path"
  | "--help" :: _ => pure { options with help := true }
  | "-h" :: _ => pure { options with help := true }
  | argument :: _ => throw s!"unknown argument: {argument}"

private def parseOptions (args : List String) : Except String Options :=
  parseOptionsFrom {} args

/-! ## Observations -/

/-- The four compared observations: status, trap, globals, and completed cycles. -/
private structure ObservedResult where
  /-- `halted` or `trap`. -/
  status : String
  /-- The trap tag, for `status = "trap"`. -/
  trap : Option String
  /-- Global values, in unit declaration order. -/
  globals : List Int
  /-- Completed cycles. -/
  cycles : Nat

/-- One root in a `stc-runtime-snapshot` document. -/
private structure SnapshotRoot where
  /-- The resolved GOT key. -/
  key : String
  /-- The canonical type key. -/
  typeKey : String
  /-- Whether the read was consistent. -/
  consistent : Bool
  /-- The value as the runtime encoded it. -/
  value : Json

/-- The parsed parts of a `stc-runtime-snapshot` document. -/
private structure Snapshot where
  /-- The active generation. -/
  generation : Nat
  /-- Completed cycles reported by the host. -/
  cycles : Nat
  /-- Every manifest root, in manifest order. -/
  roots : List SnapshotRoot

/-- The observation of one case by one path. -/
private structure Observation where
  /-- The reference interpreter's result. -/
  reference : ObservedResult
  /-- The compiled artifact's result. -/
  artifact : ObservedResult
  /-- The compiled artifact's launcher exit status. -/
  artifactExit : UInt32

/-- The three external tools the harness needs. -/
private structure Toolchain where
  /-- The Rust compiler binary. -/
  stc : String
  /-- The reference interpreter binary. -/
  stcSem : String
  /-- The static runtime library the artifact links. -/
  runtime : String
  /-- The C compiler `stc build` uses, when one is selected or found. -/
  clang : Option String

/-- A harness failure is a string; the harness runs in `IO`. -/
private abbrev CaseM := ExceptT String IO

/-! ## Small helpers -/

private def trimText (text : String) : String :=
  text.trimAscii.toString

private def jsonParse (text : String) (what : String) : CaseM Json := do
  match Json.parse text with
  | .ok json => pure json
  | .error message => throw s!"{what}: malformed JSON: {message}"

private def jsonField (j : Json) (name ctx : String) : CaseM Json := do
  match j.getObjVal? name with
  | .ok value => pure value
  | .error _ => throw s!"{ctx}.{name}: required field is missing"

private def jsonString (j : Json) (name ctx : String) : CaseM String := do
  let value ← jsonField j name ctx
  match value.getStr? with
  | .ok text => pure text
  | .error _ => throw s!"{ctx}.{name}: expected a string"

private def jsonNat (j : Json) (name ctx : String) : CaseM Nat := do
  let value ← jsonField j name ctx
  match value.getNat? with
  | .ok number => pure number
  | .error _ => throw s!"{ctx}.{name}: expected a non-negative integer"

private def jsonBool (j : Json) (name ctx : String) : CaseM Bool := do
  let value ← jsonField j name ctx
  match value.getBool? with
  | .ok flag => pure flag
  | .error _ => throw s!"{ctx}.{name}: expected a boolean"

private def jsonArray (j : Json) (name ctx : String) : CaseM (Array Json) := do
  let value ← jsonField j name ctx
  match value.getArr? with
  | .ok items => pure items
  | .error _ => throw s!"{ctx}.{name}: expected an array"

private def readText (path : System.FilePath) (what : String) : CaseM String := do
  try
    pure (← IO.FS.readFile path)
  catch error =>
    throw s!"cannot read {what} `{path}`: {error}"

private def readTextOption (path : System.FilePath) : IO (Option String) := do
  try
    pure (some (← IO.FS.readFile path))
  catch _ =>
    pure none

private def fileExists (path : System.FilePath) : IO Bool :=
  path.pathExists

/-! ## Result and snapshot readers -/

/-- Parses one `stc-sem-result` `0.1.0` object and checks its header. -/
private def parseResult (text : String) : CaseM ObservedResult := do
  let json ← jsonParse text "stc-sem-result"
  let format ← jsonString json "format" "result"
  if format != coreResultFormat then
    throw s!"result.format: `{format}` is not `{coreResultFormat}`"
  let formatVersion ← jsonString json "formatVersion" "result"
  if formatVersion != coreResultFormatVersion then
    throw s!"result.formatVersion: `{formatVersion}` is not `{coreResultFormatVersion}`"
  let semanticsVersion ← jsonString json "semanticsVersion" "result"
  if semanticsVersion != coreDumpSemanticsVersion then
    throw s!"result.semanticsVersion: `{semanticsVersion}` is not `{coreDumpSemanticsVersion}`"
  let dumpFormatVersion ← jsonString json "dumpFormatVersion" "result"
  if dumpFormatVersion != coreDumpFormatVersion then
    throw s!"result.dumpFormatVersion: `{dumpFormatVersion}` is not `{coreDumpFormatVersion}`"
  let caseFormatVersion ← jsonString json "caseFormatVersion" "result"
  if caseFormatVersion != coreCaseFormatVersion then
    throw s!"result.caseFormatVersion: `{caseFormatVersion}` is not `{coreCaseFormatVersion}`"
  let profile ← jsonString json "profile" "result"
  if profile != coreDumpProfile then
    throw s!"result.profile: `{profile}` is not `{coreDumpProfile}`"
  let status ← jsonString json "status" "result"
  if status != "halted" && status != "trap" then
    throw s!"result.status: the reference interpreter is not terminal (`{status}`)"
  let trapJson ← jsonField json "trap" "result"
  let trap ←
    if trapJson.isNull then
      pure none
    else
      match trapJson.getStr? with
      | .ok tag => pure (some tag)
      | .error _ => throw "result.trap: expected a trap tag or null"
  if status == "halted" && trap.isSome then
    throw "result.trap: a halted result has a trap"
  if status == "trap" && trap.isNone then
    throw "result.trap: a trapping result has no trap tag"
  let globalsJson ← jsonArray json "globals" "result"
  let mut globals : List Int := []
  for index in [0:globalsJson.size] do
    let entry := globalsJson[index]!
    let valueJson ← jsonField entry "value" s!"result.globals[{index}]"
    match valueJson.getInt? with
    | .ok value => globals := globals ++ [value]
    | .error _ => throw s!"result.globals[{index}].value: expected an integer"
  let cycles ← jsonNat json "cycles" "result"
  pure { status, trap, globals, cycles }

/-- Parses one `stc-runtime-snapshot` `0.1.0` document. -/
private def parseSnapshot (text : String) : CaseM Snapshot := do
  let json ← jsonParse text "runtime snapshot"
  let format ← jsonString json "format" "snapshot"
  if format != snapshotFormat then
    throw s!"snapshot.format: `{format}` is not `{snapshotFormat}`"
  let formatVersion ← jsonString json "formatVersion" "snapshot"
  if formatVersion != snapshotFormatVersion then
    throw s!"snapshot.formatVersion: `{formatVersion}` is not `{snapshotFormatVersion}`"
  let generation ← jsonNat json "generation" "snapshot"
  let cycles ← jsonNat json "cycles" "snapshot"
  let rootsJson ← jsonArray json "roots" "snapshot"
  let mut roots : List SnapshotRoot := []
  for index in [0:rootsJson.size] do
    let root := rootsJson[index]!
    let ctx := s!"snapshot.roots[{index}]"
    let key ← jsonString root "key" ctx
    let typeKey ← jsonString root "type" ctx
    let consistent ← jsonBool root "consistent" ctx
    let value ← jsonField root "value" ctx
    roots := roots ++ [(⟨key, typeKey, consistent, value⟩ : SnapshotRoot)]
  pure { generation, cycles, roots }

/-! ## Mapping dump globals to snapshot roots -/

private def decodeGlobalValue (root : SnapshotRoot) (ty : Ty) : CaseM Int := do
  if !root.consistent then
    throw s!"snapshot root `{root.key}` is not a consistent read"
  match ty with
  | .bool =>
      match root.value.getBool? with
      | .ok true => pure 1
      | .ok false => pure 0
      | .error _ => throw s!"snapshot root `{root.key}` has a non-boolean value"
  | .int _ _ | .bits _ | .char _ =>
      match root.value.getStr? with
      | .ok text =>
          match text.toInt? with
          | some value => pure value
          | none => throw s!"snapshot root `{root.key}` has a non-integer value `{text}`"
      | .error _ =>
          match root.value.getInt? with
          | .ok value => pure value
          | .error _ => throw s!"snapshot root `{root.key}` has no scalar value"

/-- Collects global values in unit declaration order from the snapshot. -/
private def collectGlobals (core : CoreUnit) (snapshot : Snapshot) : CaseM (List Int) := do
  let mut values : List Int := []
  for (global, key) in core.unit.globals.zip core.globalKeys.toList do
    match snapshot.roots.find? (fun root => root.key == key) with
    | none => throw s!"snapshot has no root for global `{key}`"
    | some root =>
        values := values ++ [← decodeGlobalValue root global.ty]
  pure values

/-- The trap tag the compiled artifact printed on stderr, if any. -/
private def trapFromStderr (text : String) : Option String :=
  let rec go : List String → Option String
    | [] => none
    | line :: rest =>
        let line := line.trimAscii.toString
        if line.startsWith "stc: runtime trap: " then
          let after := (line.drop "stc: runtime trap: ".length).toString
          match after.splitOn " " with
          | tag :: _ => some tag
          | [] => none
        else
          go rest
  go (text.splitOn "\n")

/-! ## Running the tools -/

private def runCommand (program : String) (args : Array String)
    (cwd : System.FilePath) : CaseM IO.Process.Output := do
  try
    pure (← IO.Process.output { cmd := program, args := args, cwd := some cwd })
  catch error =>
    throw s!"cannot run `{program}`: {error}"

private def expectSuccess (what : String) (output : IO.Process.Output) : CaseM Unit := do
  if output.exitCode != 0 then
    let stderr := trimText output.stderr
    let stdout := trimText output.stdout
    throw s!"{what} failed with exit {output.exitCode}\nstdout: {stdout}\nstderr: {stderr}"

/-- Runs the three paths for one case and returns their observations. -/
private def observeCase (tools : Toolchain) (casePath : System.FilePath)
    (descriptor : CoreCase) (sources : Array System.FilePath) : CaseM Observation :=
  IO.FS.withTempDir (m := CaseM) fun directory => do
    let dumpPath := directory / "dump.json"
    let resultPath := directory / "result.json"
    let snapshotPath := directory / "snapshot.json"
    let dumpArgs := #["hir-dump"] ++ sources.map (fun path => path.toString) ++
      #["--output", dumpPath.toString]
    let dumpOutput ← runCommand tools.stc dumpArgs directory
    expectSuccess "stc hir-dump" dumpOutput
    let dumpText ← readText dumpPath "core dump"
    let core ←
      match parseCoreDump dumpText with
      | .ok core => pure core
      | .error message => throw s!"core dump: {message}"
    let semArgs : Array String := #[
      "--dump", dumpPath.toString,
      "--case", casePath.toString,
      "--output", resultPath.toString
    ]
    let semOutput ← runCommand tools.stcSem semArgs directory
    expectSuccess "stc-sem" semOutput
    let resultText ← readText resultPath "reference result"
    let reference ← parseResult resultText
    if reference.globals.length != core.unit.globals.length then
      throw s!"reference result has {reference.globals.length} global(s) for {core.unit.globals.length} dump global(s)"
    let mut buildArgs := #["build"] ++ sources.map (fun path => path.toString) ++ #[
      "--emit=exe",
      "--name", "core-case",
      "--out-dir", directory.toString,
      "--runtime", tools.runtime
    ]
    if let some clang := tools.clang then
      buildArgs := buildArgs ++ #["--clang", clang]
    let buildOutput ← runCommand tools.stc buildArgs directory
    expectSuccess "stc build" buildOutput
    let exeName := if System.Platform.isWindows then "core-case.exe" else "core-case"
    let exePath := directory / exeName
    let present ← liftM (m := IO) (fileExists exePath)
    if !present then
      throw s!"stc build did not produce `{exePath}`"
    let runArgs : Array String := #[
      "--cycles", toString descriptor.cycles,
      "--snapshot", snapshotPath.toString
    ]
    let runOutput ← runCommand exePath.toString runArgs directory
    let status ←
      if runOutput.exitCode == 0 then
        pure "halted"
      else if runOutput.exitCode == 4 then
        pure "trap"
      else
        throw s!"compiled artifact exited {runOutput.exitCode} (expected 0 or 4): {trimText runOutput.stderr}"
    let trap ←
      if status == "trap" then
        match trapFromStderr runOutput.stderr with
        | some tag => pure (some tag)
        | none => throw "compiled artifact trapped without a runtime trap report"
      else
        pure none
    let snapshotText ← readText snapshotPath "runtime snapshot"
    let snapshot ← parseSnapshot snapshotText
    let globals ← collectGlobals core snapshot
    pure {
      reference
      artifact := { status, trap, globals, cycles := snapshot.cycles }
      artifactExit := runOutput.exitCode
    }

/-! ## Comparison and report -/

private def describeTrap : Option String → String
  | none => "no trap"
  | some tag => tag

private def showValues (values : List Int) : String :=
  "[" ++ ",".intercalate (values.map toString) ++ "]"

/-- The four-observation comparison shared with `Semantics-Cases.md` §6. -/
private def compareObservation (what : String) (expected : CaseExpected)
    (observed : ObservedResult) : List String := Id.run do
  let mut failures : List String := []
  if observed.status != expected.status then
    failures := failures ++
      [s!"{what}.status: expected `{expected.status}`, observed `{observed.status}`"]
  let expectedTrap := expected.trap.map trapTag
  if observed.trap != expectedTrap then
    failures := failures ++
      [s!"{what}.trap: expected {describeTrap expectedTrap}, observed {describeTrap observed.trap}"]
  if observed.globals != expected.globals then
    failures := failures ++
      [s!"{what}.globals: expected {showValues expected.globals}, observed {showValues observed.globals}"]
  if observed.cycles != expected.cycles then
    failures := failures ++
      [s!"{what}.cycles: expected {expected.cycles}, observed {observed.cycles}"]
  failures

private def intJson (value : Int) : Json :=
  Json.num (JsonNumber.fromInt value)

private def stringJson (value : String) : Json :=
  Json.str value

private def encodeExpected (expected : CaseExpected) : Json :=
  Json.mkObj [
    ("status", expected.status),
    ("trap", match expected.trap with | some trap => (trapTag trap : Json) | none => Json.null),
    ("globals", Json.arr (expected.globals.map intJson).toArray),
    ("cycles", expected.cycles)
  ]

private def encodeObservedFields (observed : ObservedResult) : List (String × Json) := [
  ("status", observed.status),
  ("trap", match observed.trap with | some tag => (tag : Json) | none => Json.null),
  ("globals", Json.arr (observed.globals.map intJson).toArray),
  ("cycles", observed.cycles)
]

private def encodeObserved (observed : ObservedResult) : Json :=
  Json.mkObj (encodeObservedFields observed)

/-- One case's report entry. -/
private structure CaseReport where
  name : String
  features : List String
  failures : List String
  expected : Json
  reference : Option ObservedResult := none
  artifact : Option ObservedResult := none
  artifactExit : Option UInt32 := none

private def failedReport (name : String) (features : List String) (expected : Json)
    (message : String) : CaseReport :=
  { name, features, failures := [message], expected }

private def encodeCaseReport (report : CaseReport) : Json :=
  let artifact :=
    match report.artifact with
    | none => Json.null
    | some observed =>
        Json.mkObj (encodeObservedFields observed ++
          [("exitCode", ((report.artifactExit.getD 0).toNat : Json))])
  let reference :=
    match report.reference with
    | none => Json.null
    | some observed => encodeObserved observed
  Json.mkObj [
    ("name", report.name),
    ("features", Json.arr (report.features.map stringJson).toArray),
    ("status", if report.failures.isEmpty then "pass" else "fail"),
    ("failures", Json.arr (report.failures.map stringJson).toArray),
    ("expected", report.expected),
    ("reference", reference),
    ("artifact", artifact)
  ]

private def encodeReport (reports : List CaseReport) : Json :=
  let failed := (reports.filter (fun report => !report.failures.isEmpty)).length
  let passed := reports.length - failed
  Json.mkObj [
    ("format", conformanceReportFormat),
    ("formatVersion", conformanceReportFormatVersion),
    ("semanticsVersion", coreDumpSemanticsVersion),
    ("profile", coreDumpProfile),
    ("dumpFormat", coreDumpFormat),
    ("dumpFormatVersion", coreDumpFormatVersion),
    ("caseFormat", coreCaseFormat),
    ("caseFormatVersion", coreCaseFormatVersion),
    ("resultFormat", coreResultFormat),
    ("resultFormatVersion", coreResultFormatVersion),
    ("snapshotFormat", snapshotFormat),
    ("snapshotFormatVersion", snapshotFormatVersion),
    ("evidence", conformanceEvidence),
    ("cases", Json.arr (reports.map encodeCaseReport).toArray),
    ("summary", Json.mkObj [
      ("total", reports.length),
      ("passed", passed),
      ("failed", failed)
    ])
  ]

/-- Runs one case; an unexpected I/O failure becomes a case failure. -/
private def runCase (tools : Toolchain) (casePath : System.FilePath) : IO CaseReport := do
  let directory := casePath.parent.getD defaultCorpus
  let fallbackName := directory.fileName.getD casePath.toString
  try
    match ← readTextOption casePath with
    | none =>
        pure (failedReport fallbackName [] Json.null s!"cannot read `{casePath}`")
    | some caseText =>
        match parseCoreCase caseText with
        | .error message =>
            pure (failedReport fallbackName [] Json.null s!"case descriptor: {message}")
        | .ok descriptor =>
            let expected := encodeExpected descriptor.expected
            let sources ← caseSources directory
            if sources.isEmpty then
              pure (failedReport descriptor.name descriptor.features expected
                "the case directory has no `.st` source files")
            else if descriptor.cycles == 0 then
              pure (failedReport descriptor.name descriptor.features expected
                "case.cycles: the differential harness needs at least one cycle")
            else
              let sources ← sources.mapM absolutize
              match ← ExceptT.run (observeCase tools casePath descriptor sources) with
              | .error message =>
                  pure (failedReport descriptor.name descriptor.features expected message)
              | .ok observation =>
                  let failures :=
                    compareObservation "reference" descriptor.expected observation.reference ++
                    compareObservation "artifact" descriptor.expected observation.artifact
                  pure {
                    name := descriptor.name
                    features := descriptor.features
                    failures
                    expected
                    reference := some observation.reference
                    artifact := some observation.artifact
                    artifactExit := some observation.artifactExit
                  }
  catch error =>
    pure (failedReport fallbackName [] Json.null s!"harness I/O failure: {error}")

/-! ## Tool discovery -/

private def envOrNone (name : String) : IO (Option String) := do
  match ← IO.getEnv name with
  | some value => if value.isEmpty then pure none else pure (some value)
  | none => pure none

private def firstExisting (candidates : List System.FilePath) : IO (Option System.FilePath) := do
  for candidate in candidates do
    if ← fileExists candidate then
      return some candidate
  pure none

private def probeCommand (name : String) (args : Array String) : IO Bool := do
  try
    let output ← IO.Process.output { cmd := name, args := args }
    pure (output.exitCode == 0)
  catch _ =>
    pure false

/-- Makes an explicit tool path absolute; bare command names stay for `PATH`. -/
private def normalizeToolPath (value : String) : IO String := do
  let hasSeparator := value.toList.any (fun c => c == '/' || c == '\\')
  if hasSeparator then
    pure (← absolutize (System.FilePath.mk value)).toString
  else
    pure value

private def resolveTool (label : String) (option : String) (envName : String)
    (explicit : Option String) (candidates : List System.FilePath)
    (probeArgs : Array String) (description : String) : IO (Except String String) := do
  match explicit with
  | some value => pure (.ok (← normalizeToolPath value))
  | none =>
      match ← envOrNone envName with
      | some value => pure (.ok (← normalizeToolPath value))
      | none =>
          match ← firstExisting candidates with
          | some path => pure (.ok path.toString)
          | none =>
              if ← probeCommand label probeArgs then
                pure (.ok label)
              else
                pure (.error s!"cannot find {description}; pass `{option} <path>` or set `{envName}`")

private def resolveRuntime (explicit : Option String)
    (candidates : List System.FilePath) : IO (Except String String) := do
  match explicit with
  | some value => pure (.ok (← normalizeToolPath value))
  | none =>
      match ← envOrNone "STC_RUNTIME" with
      | some value => pure (.ok (← normalizeToolPath value))
      | none =>
          match ← firstExisting candidates with
          | some path => pure (.ok path.toString)
          | none =>
              pure (.error "cannot find the runtime static library; pass `--runtime <path>` or set `STC_RUNTIME`")

/-- Resolves the C compiler for `stc build`, when one is selected or found. -/
private def resolveClang (explicit : Option String) : IO (Option String) := do
  match explicit with
  | some value => pure (some (← normalizeToolPath value))
  | none =>
      match ← envOrNone "STC_CLANG" with
      | some value => pure (some (← normalizeToolPath value))
      | none =>
          if System.Platform.isWindows then
            let programFiles ← envOrNone "ProgramFiles"
            let programFilesX86 ← envOrNone "ProgramFiles(x86)"
            let candidates := (programFiles.toList ++ programFilesX86.toList).map
              (fun root => System.FilePath.mk root / "LLVM" / "bin" / "clang.exe")
            pure ((← firstExisting candidates).map (fun path => path.toString))
          else
            pure none

private def resolveToolchain (options : Options) : IO (Except String Toolchain) := do
  let cwd ← IO.currentDir
  let exe (stem : String) : String := stem ++ System.FilePath.exeExtension
  let stcCandidates : List System.FilePath := [
    cwd / ".." / "target" / "debug" / exe "stc",
    cwd / "target" / "debug" / exe "stc",
    cwd / ".." / "target" / "release" / exe "stc",
    cwd / "target" / "release" / exe "stc"
  ]
  match ← resolveTool "stc" "--stc" "STC" options.stc stcCandidates
      #["--version"] "the Rust compiler" with
  | .error message => pure (.error message)
  | .ok stc =>
      let semCandidates : List System.FilePath := [
        cwd / ".lake" / "build" / "bin" / exe "stc-sem",
        cwd / ".." / "lean" / ".lake" / "build" / "bin" / exe "stc-sem"
      ]
      match ← resolveTool "stc-sem" "--stc-sem" "STC_SEM" options.stcSem semCandidates
          #["--help"] "the reference interpreter" with
      | .error message => pure (.error message)
      | .ok stcSem =>
          let runtimeName :=
            if System.Platform.isWindows then "stc_runtime.lib" else "libstc_runtime.a"
          let runtimeCandidates : List System.FilePath := [
            cwd / ".." / "target" / "test-runtime" / "debug" / runtimeName,
            cwd / "target" / "test-runtime" / "debug" / runtimeName,
            cwd / ".." / "target" / "test-runtime" / "release" / runtimeName,
            cwd / "target" / "test-runtime" / "release" / runtimeName,
            cwd / ".." / "target" / "debug" / runtimeName,
            cwd / "target" / "debug" / runtimeName,
            cwd / ".." / "target" / "release" / runtimeName,
            cwd / "target" / "release" / runtimeName
          ]
          match ← resolveRuntime options.runtime runtimeCandidates with
          | .error message => pure (.error message)
          | .ok runtime =>
              let clang ← resolveClang options.clang
              pure (.ok { stc, stcSem, runtime, clang })

/-! ## Entry point -/

private def writeReport (path : String) (text : String) : IO (Option String) := do
  try
    IO.FS.writeFile path text
    pure none
  catch error =>
    pure (some s!"cannot write report `{path}`: {error}")

/-- Runs the differential harness. -/
def run (args : List String) : IO UInt32 := do
  match parseOptions args with
  | .error message =>
      IO.eprintln s!"semantics-conformance: {message}"
      IO.eprintln usageLine
      pure usageExit
  | .ok options =>
      if options.help then
        IO.println usageLine
        pure 0
      else
        let root :=
          match options.corpus with
          | some directory => System.FilePath.mk directory
          | none => defaultCorpus
        let root ← absolutize root
        let cases ← corpusCases root
        if cases.isEmpty then
          IO.eprintln s!"semantics-conformance: no case.json descriptors under {root.toString}"
          IO.eprintln "semantics-conformance: the corpus is empty; an empty run is never reported as agreement"
          pure emptyCorpusExit
        else
          match ← resolveToolchain options with
          | .error message =>
              IO.eprintln s!"semantics-conformance: {message}"
              pure usageExit
          | .ok tools =>
              let mut reports : Array CaseReport := #[]
              for casePath in cases do
                reports := reports.push (← runCase tools casePath)
              let reportText := (encodeReport reports.toList).compress ++ "\n"
              match options.report with
              | none => pure ()
              | some path =>
                  match ← writeReport path reportText with
                  | none => pure ()
                  | some message =>
                      IO.eprintln s!"semantics-conformance: {message}"
                      return usageExit
              IO.print reportText
              for report in reports do
                for failure in report.failures do
                  IO.eprintln s!"semantics-conformance: {report.name}: {failure}"
              let failed := (reports.filter (fun report => !report.failures.isEmpty)).size
              let passed := reports.size - failed
              IO.eprintln s!"semantics-conformance: {reports.size} case(s), {passed} passed, {failed} failed"
              pure (if failed == 0 then 0 else failureExit)

end Verification.Semantics
