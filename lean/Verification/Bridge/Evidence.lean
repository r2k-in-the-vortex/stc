/-
  Verification.Bridge.Evidence — Phase 1 bridge evidence values.

  This module is hand-written, core Lean only, and part of the default `Stc`
  package. It carries the pinned tool versions, source hashes, translation
  counts, external-axiom inventory, and planned checker interfaces recorded by
  the M1–M3 bridge spikes and the M4 architecture decision.

  These values are evidence, not theorems. The registry entries P1-E01 through
  P1-E07 point here; every one is V0, and none claims a refinement or a proved
  bridge. The authoritative pins are in `lean/bridge-toolchain.toml`, and the
  drift check is `tools/ci/check_bridge_toolchain.py`.
-/

namespace Verification.Bridge

namespace Evidence

/-- The pinned Aeneas/Charon bridge environment (P1-E06). -/
structure ToolchainPin where
  leanToolchain : String
  rustChannel : String
  mathlibVersion : String
  aeneasVersion : String
  aeneasCommit : String
  charonVersion : String
  charonCommit : String
  ocamlOpam : String
  ocamlCompiler : String
  gmpVersion : String
  deriving Repr

def toolchain : ToolchainPin :=
  { leanToolchain := "leanprover/lean4:v4.35.0"
    rustChannel := "1.99.1"
    mathlibVersion := "v4.35.0"
    aeneasVersion := "nightly-2026.11.02-ab34bd2"
    aeneasCommit := "ab34bd2"
    charonVersion := "0.1.304"
    charonCommit := "7b462d9"
    ocamlOpam := "2.6.1"
    ocamlCompiler := "5.3.0"
    gmpVersion := "6.3.0" }

/-- One bridge source pin (P1-E03). -/
structure SourcePin where
  target : String
  path : String
  sha256 : String
  generated : Bool
  generatedPath : String
  deriving Repr

def exactSource : SourcePin :=
  { target := "exact"
    path := "crates/stc-codegen/src/exact.rs"
    sha256 := "85f3fbfc5263c918f80fc52b46322510de6645bef66c3fc078c7ba4ee826cba4"
    generated := true
    generatedPath := "lean/aeneas/StcGenerated/Exact/Model.lean" }

def keysSource : SourcePin :=
  { target := "keys"
    path := "crates/stc-got/src/keys.rs"
    sha256 := "af80f2d87d4c9f9d6f59310210d5020daccbeb7ef0b02af03d91bf2aa9ad34bc"
    generated := false
    generatedPath := "" }

def wireSource : SourcePin :=
  { target := "wire"
    path := "crates/stc-got/src/wire.rs"
    sha256 := "ae840ab12704434b162013fafc85facc2883275dcaefccd2951fd2d09b7810fd"
    generated := true
    generatedPath := "lean/aeneas/StcGenerated/Wire/Model.lean" }

def sourcePins : List SourcePin := [exactSource, keysSource, wireSource]

/-- The recorded result of one Aeneas/Charon spike (P1-E01, P1-E04, P1-E05). -/
structure TranslationResult where
  target : String
  sourcePath : String
  sourceSha256 : String
  exitCode : Int
  transparentFunctions : Option Nat
  opaqueFunctions : Option Nat
  outcome : String
  note : String
  deriving Repr

def exactModelTypecheck : TranslationResult :=
  { target := "exact"
    sourcePath := exactSource.path
    sourceSha256 := exactSource.sha256
    exitCode := 0
    transparentFunctions := none
    opaqueFunctions := none
    outcome := "filtered pure-kernel model typechecks under the pinned environment"
    note :=
      "Aeneas produced the filtered Exact.lean and `lake env lean` returned rc=0; " ++
      "the whole-file output (unsupported helper bodies and REGISTRY emitted without " ++
      "proofs) is rejected evidence and is not used." }

def keysTranslation : TranslationResult :=
  { target := "keys"
    sourcePath := keysSource.path
    sourceSha256 := keysSource.sha256
    exitCode := 2
    transparentFunctions := none
    opaqueFunctions := none
    outcome := "translation failed"
    note :=
      "Aeneas produced no Lean file: early returns inside loops, unsupported " ++
      "string signatures, and a higher-ranked lifetime constraint. The proved " ++
      "Lean checker/contract is the chosen bridge (M4-D5); this is not a claim." }

def wireTranslation : TranslationResult :=
  { target := "wire"
    sourcePath := wireSource.path
    sourceSha256 := wireSource.sha256
    exitCode := 0
    transparentFunctions := some 54
    opaqueFunctions := some 65
    outcome := "translation succeeded and the generated file typechecks"
    note :=
      "Fallback evidence only (M4-D6). External definitions and the " ++
      "Aeneas/Mathlib proof-obligation cost are unmeasured; no V1/V2 claim." }

/-- One external operation left opaque by the filtered exact model (P1-E02). -/
structure ExternalAxiom where
  name : String
  domain : String
  trust : String
  deriving Repr

def externalAxioms : List ExternalAxiom :=
  [ { name := "I64::from(u8)"
      domain := "total widening conversion used by IntType.count"
      trust := "model as the natural embedding; discharge or carry as a named TCB hypothesis" },
    { name := "I64::rem_euclid"
      domain := "positive divisor in shifts and rotates; zero-divisor panic unreachable"
      trust := "model as Euclidean remainder under 0 < b; discharge from the guard or carry as a named hypothesis" },
    { name := "I128::checked_neg"
      domain := "neg on Option<i128>; none exactly at i128::MIN"
      trust := "model by definition replacement or carry as a named hypothesis" },
    { name := "I128::abs"
      domain := "abs called only after the signed MIN guard; panic unreachable"
      trust := "model on the guarded domain or discharge with the guard lemma" } ]

def externalAxiomInventory : List ExternalAxiom := externalAxioms

/-- A checker interface that Phase 3 implements (P1-E07). -/
structure CheckerInterface where
  module : String
  entryPoints : List String
  status : String
  deriving Repr

def checkerInterfaces : List CheckerInterface :=
  [ { module := "Stc.Bridge.Keys.Contract"
      entryPoints := ["normalize_sound", "normalize_idempotent", "validate_sound"]
      status := "planned; implementation and proofs are Phase 3" },
    { module := "Stc.Bridge.Wire.Contract"
      entryPoints := ["decode_sound", "decode_encode", "count_bounded", "finish_sound"]
      status := "planned; implementation and proofs are Phase 3" } ]

end Evidence

end Verification.Bridge
