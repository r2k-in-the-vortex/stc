import Lake
open Lake DSL

package stc

/-- The exact-function models and their proofs (Testing and Verification §5.2). -/
@[default_target]
lean_lib Stc where
  globs := #[Glob.submodules `Stc]

/-- Executable verification tooling for the Phase 0 semantics. -/
@[default_target]
lean_lib Verification where
  globs := #[Glob.submodules `Verification]

/-- `lake exe vectors` regenerates the committed exact-function vectors. -/
lean_exe vectors where
  root := `VectorCli

/--
`lake exe semantics-conformance` runs the Phase 0 core-corpus harness. At P0.0
it is a fail-closed placeholder; see `Verification.Semantics.Conformance`.
-/
@[default_target]
lean_exe «semantics-conformance» where
  root := `SemanticsConformance
