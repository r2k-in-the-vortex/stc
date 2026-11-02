/-
`lake exe stc-sem`: the reference-interpreter driver (P0.6).

See `Verification.Semantics.ReferenceCli` for the command contract.
-/
import Verification.Semantics.ReferenceCli

def main (args : List String) : IO UInt32 :=
  Verification.Semantics.ReferenceCli.run args
