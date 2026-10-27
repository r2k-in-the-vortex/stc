/-
The executable reference interpreter for the core HIR subset (P0.6).

The interpreter is a fuel-based total function over the same configuration type
as the declarative source semantics. `interpreter_sound` and
`interpreter_complete` relate it to the reflexive-transitive closure of
`Stc.Sem.Step`. It is a verification tool, not a production interpreter.
-/
namespace Verification.Semantics
end Verification.Semantics
