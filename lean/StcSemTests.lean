/-
`lake exe stc-sem-tests`: unit tests for the P0.6 reference path.

The tests use in-memory and temporary-file fixtures; no corpus `case.json` is
committed.
-/
import Verification.Semantics.ReferenceTests

def main : IO UInt32 :=
  Verification.Semantics.ReferenceTests.runReferenceTests
