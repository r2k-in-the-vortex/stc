# Phase 2 Report — scalar semantic kernel, exact operations, and constant evaluation

Profile: `stc-2013-oop-utf8-v1`

Status: Phase 2 implementation is complete against the four owner decisions
recorded in `WL-3-Phase-2.md` §5.1, but the phase is **not owner-accepted and
does not claim V1**. This report records the work completed in WL-3, the
evidence produced, the parts that remain model-level or V0, and the review still
required for a V1 claim. It is durable; the temporary execution plan remains
`WL-3-Phase-2.md` at the repository root until owner acceptance.

Starting commit: `c82e521` (Phase 1 owner-accepted 2026-11-03).
Phase 2 commits through this report: `7520ae1` (exact property lemmas),
`5785011` (constant-evaluation model and proofs), `3d12a46` (bridge package and
generated model), `6a91ba4` (Rust differential certificate), `c790a40` and
`3ad3419` (owner decisions and fast-CI evidence), `ed8097e` (single kernel and
widened model), and `a75a9cb` (24 exact-model refinement proofs). The final
certificate, registry, and report commit is recorded in the task handoff.

## 1. Scope and claim discipline

The Phase 2 scope in
[VerificationProgramPlan.md](../VerificationProgramPlan.md#phase-2-scalar-semantic-kernel-exact-operations-and-constant-evaluation)
is the exact scalar kernel, its constant-evaluation model, the exact bridge, and
the certificate or unification of the three Rust implementations of the same
rules. The claim ladder in §3 of the program plan is unchanged:

- the Phase 0 S-level semantics remains the only owner-reviewed semantic claim;
- Phase 2 model theorems are proved facts about the Lean model, recorded at level
  V0 (model-verified; implementation tested against the model);
- no Phase 2 record claims V1 and no external Aeneas axiom is discharged; the
  Rust production paths now delegate to one shared kernel, but the refinement
  statements and their axiom equations remain subject to owner review;
- no `sorry` or `admit` exists in the default package or in the checked-in
  bridge modules, and the default `lean/` build imports neither Aeneas nor
  Mathlib.

## 2. Milestones

| ID | Milestone | Status | Evidence |
| --- | --- | --- | --- |
| P2-M1 | Working plan | Done | `WL-3-Phase-2.md` and this report |
| P2-M2 | Exact property lemmas | Done | `Stc.Exact.{Int,Bits,Conv,Select,Strings}` extensions; `lake build` clean |
| P2-M3 | Constant-evaluation model | Done for the modeled subset | `Stc/ConstEval.lean`; `constEval_sound`, `constEval_type_preserving`; limitations in §4 |
| P2-M4 | Exact bridge package | Done | `lean/aeneas/` package, pinned manifest, widened checked-in generated model; host build of `Bridge.Exact.Audit` green |
| P2-M5 | Bridge obligations | Done | 32 obligations, 32 proved, 0 open; twelve generated axiom declarations and the bridge string equations recorded in the TCB; no V1 |
| P2-M6 | Rust kernel certificate | Done | one shared `exact.rs` kernel; constant certificates verified before IR emission; differential tests and vector tests |
| P2-M7 | Registry and CI | Done | P2 records, checker policy, sorry-grep coverage; certificate tests run in `cargo test --workspace` |
| P2-M8 | Durable report and fast CI | Report done; fast CI result recorded by the final WL-3 run | This report and the CI run cited in the final handoff |

## 3. Exact property lemmas

`lean/Stc/Exact/Int.lean` now proves success/overflow characterizations for
`add`, `sub`, `mul`, `neg`, `abs`, `div`, and `mod`, including the
most-negative-divided-by-minus-one overflow guard, the truncating
`div_mod_identity`, and range extrema (`fits_iff`, `fits_minimum`,
`fits_maximum`, `minimum_le_maximum`). `Int.lean` also records that the Rust
implementation uses `i128` intermediates while the Lean model is unbounded.

`lean/Stc/Exact/Bits.lean` adds count masking (`count_zero`, `count_width`,
`count_lt`), range lemmas for every bitwise/shift/rotate operation, count-zero
and count-width identities, `ror_width` and `rol_width`, and bitwise value
identities. A complete `rol`/`ror` inverse is not claimed.

`lean/Stc/Exact/Conv.lean` adds `BOOL` outcomes, narrow and widen fit lemmas, a
round-trip theorem for widening conversions, `convert_equal_ok_fits` for
equal-width identity and signedness reinterpretation, and the single
`convert_ok_fits` statement over the generated cast-plan order (`BOOL`,
widening, narrowing, equal width). The equal-width fit gap is closed; one-bit
targets are required to be unsigned, matching the production layouts.

`lean/Stc/Exact/Select.lean` adds membership and range properties for `SEL`,
`MIN`/`MAX`, `LIMIT`, and in-range `MUX`; tie-breaking is represented by the
bound and membership lemmas, not by a first-index tie theorem.

`lean/Stc/Exact/Strings.lean` adds supplementary-scalar lengths, aggregate
UTF-8 byte bounds, UTF-16 bounds, append identities, and per-kind
length/code-unit bounds. The full lemma list is the module inventory; this
report and the registry name the central results.

## 4. Constant-evaluation model

`lean/Stc/ConstEval.lean` defines:

- `ConstError`, including unbound and non-constant variables, type mismatch,
  malformed payloads, exact traps, and unsupported user-function calls;
- `ConstWorld`, carrying the typing context, known constant values, and the
  `value_typed` invariant;
- the executable left-to-right `constEval` and `evalArgs`;
- the declarative `ConstEvalRel` and `ConstEvalRelArgs`;
- `constEval_sound` and `constEval_type_preserving`, proved through
  `ConstEvalRel_typed`.

The modeled subset is literals, paths to constants, unary and binary operators,
casts, and the exact standard functions. User-function calls, real and
date/time values, string folding beyond literals, and source-syntax folding are
outside the model. The model is not proved equal to `consteval.rs`; the
production folder delegates to the same `exact.rs` kernel used by code
generation and records verifiable constant certificates, and the differential
certificate in §6 is V0 test evidence for that shared scalar subset.

## 5. Exact bridge

The non-default package is `lean/aeneas/`. Its `lakefile.lean` requires Aeneas
from git at `ab34bd28458a08c150b7cbf1873e59af74588138`, subdirectory
`backends/lean`, plus the default `stc` package from `..`. The checked-in
`lake-manifest.json` pins Mathlib `v4.35.0` and all transitive dependencies.
Fast CI never builds this package.

`lean/aeneas/StcGenerated/Exact/Model.lean` is the widened generated model. It
is `764` lines: a `20`-line provenance header naming the source path
`crates/stc-codegen/src/exact.rs`, source SHA-256
`5ecf5d604bb7d545927bc9626876f6fcedfead405623378c4800b3f65b377c44`,
Aeneas `nightly-2026.11.02-ab34bd2`, Charon `0.1.304 (7b462d9)`, and the
generation recipe, followed by the `744`-line generated body. The body SHA-256
is `fa6628af1041f27db8fae1d5cca9ab318d14b515d72465addc950f73d271c0bf`,
byte-identical to the accepted wide spike. The generated body is never
hand-edited, and it contains no generated `sorry`. The generation roots are
`add`, `sub`, `mul`, `neg`, `div`, `modulo`, `abs`, the bitwise/shift/rotate
operations, `compare`, `sel`, `convert`, `min_list`, `max_list`, `limit`, `mux`,
`len`, `len_max`, and `len_code_unit`.

The generated model declares twelve axioms. Nine are operation axioms: the four
external scalar operations `I64::from(u8)`, `I64::rem_euclid`,
`I128::checked_neg`, and `I128::abs`, modeled in `Stc.Bridge.Exact.Externals`,
and five string operations whose equations are stated in
`Bridge/Exact/WideExternals.lean`. The remaining three declarations (two
`Display::fmt` axioms and the opaque `EncodeUtf16` type) lie outside the proved
obligation closure. The shared decoding function `strScalars` is an opaque TCB
function declared in `Bridge/Exact/StrModel.lean`. `Bridge/Exact/Audit.lean`
inventories every generated declaration, the five string equations, and all
32 obligations.

`Stc.Bridge.Exact.Encode` and `Stc.Bridge.Exact.Externals` are core Lean and do
not import Aeneas. `Bridge/Exact/Refinement.lean` states the original
per-operation propositions, `Bridge/Exact/WideRefinement.lean` states the eight
widened ones, and all 32 have completed proofs in `Soundness.lean`,
`Refinement.lean`, and `WideSoundness.lean`. `Bridge/Exact/Kernel.lean`
composes them into `exact_kernel_refines`, re-exported by
`Audit.kernelStatement_proved`. The bridge records stay `tested` because the
package is not built in fast CI and every TCB declaration remains an
assumption; the proof completion does not by itself create a V1 claim.

The four external operations are modeled, not discharged. Their equations
remain TCB assumptions recorded with the exact-side model; the guard lemmas
describe the domain on which each model applies. Similarly, the five string
equations and the UTF-8 byte-length invariant are TCB assumptions about the
opaque generated operations and the shared string content model.

## 6. Rust differential certificate

`crates/stc-codegen/tests/exact_differential.rs` is the Phase 2 certificate for
the shared scalar subset:

- `constant_arithmetic_matches_the_exact_reference` folds `+`, `-`, `*`, `/`,
  `MOD`, unary `-`, bitwise `AND`/`OR`/`XOR`/`NOT`, and the six comparisons on
  every integer/bit-string type and boundary values, and compares each result
  or constant failure with `stc_codegen::exact`.
- `constant_conversions_match_the_exact_reference` folds every ordered
  conversion pair accepted by the front end (178 of 256 `CONV_TYPES` pairs,
  842 boundary cases) and compares the value, `CHAR` payload, or range failure
  with `exact::convert`.
- `runtime_conversions_emit_the_exact_model_shape` checks emitted IR for each
  conversion class: widening is `sext`/`zext`, narrowing is a range check plus
  `trunc`, equal widths are a trap-free reinterpretation, and `BOOL` targets
  test against zero.
- `constant_folding_records_verified_certificates` folds arithmetic, bitwise,
  unary, and conversion constants, requires the recorded certificate classes,
  verifies every certificate through the exact kernel, and generates the module.
- `corrupted_constant_certificate_is_rejected` changes one recorded result and
  requires generation to fail with the internal certificate diagnostic before
  emitting IR.

The production paths now share one kernel. `crates/stc-codegen/src/exact.rs` is
the single source of the scalar rules: `consteval.rs` calls `exact::add`,
`sub`, `mul`, `div`, `modulo`, `neg`, the bitwise helpers, and `exact::convert`;
codegen dispatches integer conversions on `exact::cast_plan` and takes range
bounds from `exact::IntType`. The certificate path closes the loop: the folder
records a `ConstCertificate` (operation, layouts, operands, result) for every
arithmetic, bitwise, and conversion value it produces, and
`crates/stc-codegen/src/certificate.rs` recomputes each one through the same
kernel before `generate` projects or emits any IR. A mismatch is an internal
codegen error. The verifier is Rust, not a proved Lean checker, and the
repository has no Lean model of HIR-to-IR lowering, so
`consteval_agrees_with_codegen` is established by construction (one kernel used
by both paths) plus this build-time checker, not by a named Lean theorem. The
certificate module is deliberately outside `exact.rs` so the bridge source pin
stays byte-identical.

The existing `constant_folding_agrees_with_exact_vectors` test in
`crates/stc-codegen/tests/lean_vectors.rs` continues to compare the analyzer's
folding with the committed generated vectors, which are themselves checked
against `exact.rs`. Shifts, rotates, `ABS`, and the selection functions have no
constant-folding syntax in `consteval.rs`, so they remain model-plus-vector
evidence rather than differential evidence.

This is V0 agreement and build-time enforcement evidence, not a Lean refinement
proof of the compiler. The 32 refinement proofs constrain the generated model
of the shared kernel; they do not constrain the Rust verifier or the lowering
pipeline.

## 7. Verification evidence

Local default-package checks on the final Phase 2 content:

- `lake build` in `lean/` (default package, no Aeneas/Mathlib);
- `lake exe stc-sem-tests` (`OK`, 11 test groups);
- `python tools/ci/check_verification_registry.py`;
- `python tools/ci/check_docs.py` and the pinned markdownlint command;
- `python tools/ci/check_bridge_toolchain.py` (plain guard; the
  `--require-generated` form still fails because the Phase 1 `wire` model is not
  checked in);
- sorry/admit grep over `Stc/Exact`, `Stc/ConstEval`, `Stc/Bridge`,
  `aeneas/Bridge`, and `aeneas/StcGenerated`;
- `cargo fmt --check`, `cargo clippy`, and `cargo test` for the changed Rust
  crates, including the differential and certificate tests;
- `cargo test --workspace` in fast CI, which runs `exact_differential`.

Bridge package check on the pinned Ubuntu host: `lake build` in `lean/aeneas`
completed `1812` jobs, building the generated model, all bridge proof modules,
and `Bridge.Exact.Audit`. This is manual/nightly evidence and is not part of
fast CI.

Fast CI on the final WL-3 HEAD is recorded in the final handoff for this task.

## 8. Trusted computing base

- The Lean 4.35.0 kernel and standard library, plus Batteries for executable
  models.
- The Phase 0 definitions and the frozen `semantics_version` `1.0`.
- The generated Aeneas model and its provenance pins; Aeneas, Charon, and
  Mathlib are trusted for the bridge package only.
- The generated model's twelve axiom declarations. Nine are operation axioms:
  the four external scalar operations are modeled in `Externals` and remain
  assumptions, and the five string operations are covered by the TCB equations
  in `WideExternals` together with the UTF-8 byte-length invariant. The other
  three (two `Display::fmt` axioms and the opaque `EncodeUtf16` type) are
  outside the proved obligation closure.
- `strScalars`, the shared opaque decoding function, and the four string
  operation equations are TCB assumptions; they are not discharged.
- The Rust certificate verifier (`crates/stc-codegen/src/certificate.rs`) is
  trusted test/enforcement code, not a proved Lean checker. It recomputes
  folded values through the shared kernel and rejects mismatches at build
  time.
- The Rust differential tests use the Rust standard library and the analyzer's
  own constant folder as the implementation side of the comparison.
- Rust `i128` intermediates: the reference model and the production paths trap
  when an intermediate exceeds `i128`, while the Lean model is unbounded; the
  boundary tests cover the shared scalar widths, and the residual difference is
  recorded rather than assumed away.

No theorem in Phase 2 constrains the Rust compiler. The 32 bridge proofs
constrain the generated model of the shared kernel; the Rust implementation
remains V0/model-tested, and the certificate checker is the build-time tie
between the two.

## 9. Remaining work and owner decisions

The work items from the previous Phase 2 status are complete: the 32
refinement obligations are proved, the widened model is accepted with no
generated `sorry`, the equal-width conversion fit gap is closed, the Rust paths
share one kernel, and the constant-certificate path is implemented and tested.
What remains before a V1 claim is owner review, not open implementation work:

1. Review the 32 refinement proofs and the bundled `exact_kernel_refines`
   statement, and decide whether the completed proofs support a V1 record.
2. Discharge or explicitly accept the external operation axioms and the string
   TCB equations (`strScalars`, the five recorded equations, and the three
   out-of-closure generated declarations). The audit records them; none is
   discharged.
3. Decide whether `consteval_agrees_with_codegen` must be a Lean theorem. A
   non-vacuous statement over the compiler requires a Lean model of HIR-to-IR
   lowering, which the repository does not have; the current realization is the
   single shared kernel plus the Rust certificate checker. Building the
   lowering model is new scope.
4. Decide whether the non-default bridge package must run in fast CI. It is
   currently manual/nightly evidence, consistent with the 2026-11-03 owner
   decision that fast CI never installs or builds Aeneas/Mathlib.

Owner decisions, in the order they block progress:

- whether to record a V1 claim after reviewing the proofs and the TCB;
- whether to accept the modeled external axioms and string equations as the
  phase's trusted boundary, or require discharge;
- whether to fund a Lean HIR-to-IR lowering model and a proved certificate
  checker for the literal `consteval_agrees_with_codegen` statement;
- whether any toolchain pin or accepted Phase 1 architecture decision changes.

The implementing agent does not make any of these decisions. Until the owner
responds, the correct status is V0/model-verified and implementation-tested, and
the working plan stays in the repository.

## 10. Retirement status

This report carries the durable Phase 2 content: milestone status, theorem
inventory, bridge scope and TCB, residual risks, and the owner gate list. The
temporary `WL-3-Phase-2.md` is not deleted and this report does not claim owner
acceptance. On acceptance, the working plan's operational rows can be removed
after confirming that this report, `Semantics.md` §7, and
`lean/verification-status.toml` carry the same evidence and levels.
