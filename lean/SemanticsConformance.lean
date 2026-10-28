/-
`lake exe semantics-conformance`: Phase 0 conformance harness entry point.

See `Verification.Semantics.Conformance` for the fail-closed placeholder
behavior installed at P0.0.
-/
import Verification.Semantics.Conformance

open Verification.Semantics

def main (args : List String) : IO UInt32 :=
  run args
