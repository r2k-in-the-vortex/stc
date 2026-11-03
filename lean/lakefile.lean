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
`lake exe semantics-conformance` runs the Phase 0 core-corpus differential
harness; see `Verification.Semantics.Conformance`.
-/
@[default_target]
lean_exe «semantics-conformance» where
  root := `SemanticsConformance

/--
`lake exe stc-sem` runs one core case through the reference interpreter; see
`Verification.Semantics.ReferenceCli`.
-/
@[default_target]
lean_exe «stc-sem» where
  root := `StcSem

/--
`lake exe stc-sem-tests` runs the reader, case-parser, result-encoding, and CLI
tests with temporary fixtures.
-/
@[default_target]
lean_exe «stc-sem-tests» where
  root := `StcSemTests
