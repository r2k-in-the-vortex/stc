import Lake
open Lake DSL

package stc

/-- The exact-function models and their proofs (Testing and Verification §5.2). -/
@[default_target]
lean_lib Stc where
  globs := #[Glob.submodules `Stc]

/-- `lake exe vectors` regenerates the committed exact-function vectors. -/
lean_exe vectors where
  root := `VectorCli
