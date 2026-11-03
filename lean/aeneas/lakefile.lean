import Lake
open Lake DSL

/-
The non-default Aeneas/Mathlib bridge package.

This package is never part of the default `lean/` build and never runs in fast
CI. The pins mirror `lean/bridge-toolchain.toml`; changing them requires owner
review. The generated model lives under `StcGenerated/`; hand-written
refinement propositions and the trust audit live under `Bridge/`.
-/

package stcBridge

require aeneas from git
  "https://github.com/AeneasVerif/aeneas.git" @ "ab34bd28458a08c150b7cbf1873e59af74588138" /
  "backends/lean"

require stc from ".."

/-- The checked-in filtered Aeneas model of the exact kernel. -/
@[default_target]
lean_lib StcGenerated where
  globs := #[Glob.submodules `StcGenerated]

/-- Hand-written refinement propositions and the external-axiom audit. -/
@[default_target]
lean_lib Bridge where
  globs := #[Glob.submodules `Bridge]
