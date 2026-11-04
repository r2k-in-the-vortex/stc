/-
`lake exe semantics-conformance`: Phase 0 core-corpus differential harness
entry point.

See `Verification.Semantics.Conformance` for the corpus, comparison, report,
and fail-closed empty-corpus behavior.
-/
import Verification.Semantics.Conformance

open Verification.Semantics

def main (args : List String) : IO UInt32 :=
  run args
