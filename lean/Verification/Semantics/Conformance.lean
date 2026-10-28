/-
`lake exe semantics-conformance`: the Phase 0 core-corpus harness (P0.6).

P0.0 installs a fail-closed placeholder. While `tests/verification/semantics`
contains no `case.json` descriptors, the executable exits nonzero so CI cannot
mistake an empty corpus for corpus agreement. Once cases exist, the executable
grows into the differential harness specified in `Plan-Phase0.md` §9.3:
it runs the Rust compiler, the reference interpreter, and the compiled
artifact or test host, then compares values, traps, global snapshots, and the
semantics and dump-format versions.
-/
import Std

namespace Verification.Semantics

/-- Default corpus location, relative to the `lean/` directory. -/
def defaultCorpus : System.FilePath := "../tests/verification/semantics"

/-- Exit status used while the corpus is empty: the harness is fail-closed. -/
def emptyCorpusExit : UInt32 := 3

/-- Exit status used when cases exist but the differential harness is absent. -/
def unimplementedExit : UInt32 := 4

/-- Exit status for an invalid command line. -/
def usageExit : UInt32 := 2

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

/-- Runs the harness with an optional `--corpus DIR` override. -/
def run (args : List String) : IO UInt32 := do
  let (root, rest) ← match args with
    | "--corpus" :: dir :: rest => pure (System.FilePath.mk dir, rest)
    | _ => pure (defaultCorpus, args)
  if let some arg := rest.head? then
    IO.eprintln s!"semantics-conformance: unknown argument: {arg}"
    IO.eprintln "usage: semantics-conformance [--corpus DIR]"
    return usageExit
  let cases ← corpusCases root
  if cases.isEmpty then
    IO.eprintln s!"semantics-conformance: no case.json descriptors under {root.toString}"
    IO.eprintln "semantics-conformance: fail-closed placeholder; the core corpus and differential harness arrive in P0.6"
    return emptyCorpusExit
  IO.eprintln s!"semantics-conformance: found {cases.size} case(s) under {root.toString}, but the differential harness is not implemented until P0.6"
  return unimplementedExit

end Verification.Semantics
