/-
`lake exe stc-sem`: run one core case through the reference interpreter (P0.6).

Usage:
  stc-sem --dump <dump.json> --case <case.json> [--trace] [--output <path>]

The command reads a `stc-core-hir` `0.1.0` dump and a `stc-core-case` `0.1.0`
descriptor, runs the reference interpreter, and writes one compressed JSON
`stc-sem-result` `0.1.0` object followed by a newline. A produced result exits
`0` even when it is a trap, `outOfFuel`, or `stuck`; those are observations for
the harness to compare. Invalid input exits `2` for an unreadable or unwritable
path and `3` for a format, version, schema, or entry mismatch.
-/
import Lean.Data.Json
import Verification.Semantics.Reference

namespace Verification.Semantics.ReferenceCli

open Lean (Json)
open Verification.Semantics

/-- Exit status for an invalid command line or an I/O failure. -/
def usageExit : UInt32 := 2

/-- Exit status for an invalid dump, case, or entry mismatch. -/
def validationExit : UInt32 := 3

/-- The command-line usage line. -/
def usage : String :=
  "usage: stc-sem --dump <dump.json> --case <case.json> [--trace] [--output <path>]"

/-- Parsed command-line options. -/
private structure Options where
  dump : Option String := none
  caseFile : Option String := none
  output : Option String := none
  trace : Bool := false
  help : Bool := false

private def setOnce (current : Option String) (value : String) (flag : String) :
    Except String (Option String) := do
  if current.isSome then
    throw s!"{flag} is given more than once"
  pure (some value)

private partial def parseArgsFrom (options : Options) : List String → Except String Options
  | [] => pure options
  | "--dump" :: path :: rest => do
      let dump ← setOnce options.dump path "--dump"
      parseArgsFrom { options with dump } rest
  | ["--dump"] => throw "--dump requires a path"
  | "--case" :: path :: rest => do
      let caseFile ← setOnce options.caseFile path "--case"
      parseArgsFrom { options with caseFile } rest
  | ["--case"] => throw "--case requires a path"
  | "--output" :: path :: rest => do
      let output ← setOnce options.output path "--output"
      parseArgsFrom { options with output } rest
  | ["--output"] => throw "--output requires a path"
  | "--trace" :: rest => parseArgsFrom { options with trace := true } rest
  | "--help" :: _ => pure { options with help := true }
  | "-h" :: _ => pure { options with help := true }
  | argument :: _ => throw s!"unknown argument: {argument}"

private def parseArgs (args : List String) : Except String Options :=
  parseArgsFrom {} args

private def readInput (path what : String) : IO (Option String) := do
  try
    pure (some (← IO.FS.readFile path))
  catch error =>
    IO.eprintln s!"stc-sem: cannot read {what} `{path}`: {error}"
    pure none

private def writeOutput (path text : String) : IO Bool := do
  try
    IO.FS.writeFile path text
    pure true
  catch error =>
    IO.eprintln s!"stc-sem: cannot write `{path}`: {error}"
    pure false

private def emit (options : Options) (core : CoreUnit) (result : ReferenceResult) :
    IO UInt32 := do
  let text := (encodeResult core result).compress ++ "\n"
  match options.output with
  | none =>
      IO.print text
      pure 0
  | some path =>
      if ← writeOutput path text then pure 0 else pure usageExit

private def execute (options : Options) (dumpPath casePath : String) : IO UInt32 := do
  match ← readInput dumpPath "dump" with
  | none => pure usageExit
  | some dumpText =>
      match ← readInput casePath "case" with
      | none => pure usageExit
      | some caseText =>
          match parseCoreDump dumpText with
          | .error message =>
              IO.eprintln s!"stc-sem: dump: {message}"
              pure validationExit
          | .ok core =>
              match parseCoreCase caseText with
              | .error message =>
                  IO.eprintln s!"stc-sem: case: {message}"
                  pure validationExit
              | .ok descriptor =>
                  match runReference core descriptor options.trace with
                  | .error message =>
                      IO.eprintln s!"stc-sem: {message}"
                      pure validationExit
                  | .ok result => emit options core result

/-- Run the `stc-sem` command. -/
def run (args : List String) : IO UInt32 := do
  match parseArgs args with
  | .error message =>
      IO.eprintln s!"stc-sem: {message}"
      IO.eprintln usage
      pure usageExit
  | .ok options =>
      if options.help then
        IO.println usage
        pure 0
      else
        match options.dump, options.caseFile with
        | some dumpPath, some casePath => execute options dumpPath casePath
        | none, _ =>
            IO.eprintln "stc-sem: --dump is required"
            IO.eprintln usage
            pure usageExit
        | _, none =>
            IO.eprintln "stc-sem: --case is required"
            IO.eprintln usage
            pure usageExit

end Verification.Semantics.ReferenceCli
