# Phase 2 Report — scalar semantic kernel, exact operations, and constant evaluation

Profile: `stc-2013-oop-utf8-v1`

Status: Phase 2 implementation is substantially complete, but the phase is **not
owner-accepted and does not meet its V1 exit criterion**. This report records the
work completed in WL-3, the evidence produced, the parts that remain model-level
or V0, and the owner decisions that block a V1 claim. It is durable; the
temporary execution plan remains `WL-3-Phase-2.md` at the repository root until
owner acceptance.

Starting commit: `c82e521` (Phase 1 owner-accepted 2026-11-03).
Current Phase 2 commits: `7520ae1` (exact property lemmas), `5785011`
(constant-evaluation model and proofs), `3d12a46` (bridge package and generated
model), `6a91ba4` (Rust differential certificate).

## 1. Scope and claim discipline

The Phase 2 scope in
[VerificationProgramPlan.md](../VerificationProgramPlan.md#phase-2-scalar-semantic-kernel-exact-operations-and-constant-evaluation)
is the exact scalar kernel, its constant-evaluation model, the exact bridge, and
the certificate or unification of the three Rust implementations of the same
rules. The claim ladder in §3 of the program plan is unchanged:

- the Phase 0 S-level semantics remains the only owner-reviewed semantic claim;
- Phase 2 model theorems are proved facts about the Lean model, recorded at level
  V0 (model-verified; implementation tested against the model);
- no Phase 2 record claims V1, no external Aeneas axiom is discharged, and the
  Rust production paths remain duplicated;
- no `sorry` or `admit` exists in the default package or in the checked-in
  bridge modules, and the default `lean/` build imports neither Aeneas nor
  Mathlib.

## 2. Milestones

| ID | Milestone | Status | Evidence |
| --- | --- | --- | --- |
| P2-M1 | Working plan | Done | `WL-3-Phase-2.md` and this report |
| P2-M2 | Exact property lemmas | Done | `Stc.Exact.{Int,Bits,Conv,Select,Strings}` extensions; `lake build` clean |
| P2-M3 | Constant-evaluation model | Done for the modeled subset | `Stc/ConstEval.lean`; `constEval_sound`, `constEval_type_preserving`; limitations in §4 |
| P2-M4 | Exact bridge package | Done | `lean/aeneas/` package, pinned manifest, checked-in generated model; host build `1806` jobs |
| P2-M5 | Bridge obligations | Partial, honestly recorded | 24 obligations, 2 proved, 22 open; four external axioms open; no V1 |
| P2-M6 | Rust kernel certificate | Done for the shared subset | `crates/stc-codegen/tests/exact_differential.rs`; existing vector agreement test |
| P2-M7 | Registry and CI | Done | P2 records, checker policy, sorry-grep coverage |
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

`lean/Stc/Exact/Conv.lean` adds `BOOL` outcomes, narrow and widen fit lemmas, and
a round-trip theorem for widening conversions. A single `convert_ok_fits`
covering equal-width signedness reinterpretation is still not proved; the
individual equal-width cases are proved and the general fit statement remains a
recorded gap.

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
differential certificate in §6 is V0 test evidence for the shared scalar
subset.

## 5. Exact bridge

The non-default package is `lean/aeneas/`. Its `lakefile.lean` requires Aeneas
from git at `ab34bd28458a08c150b7cbf1873e59af74588138`, subdirectory
`backends/lean`, plus the default `stc` package from `..`. The checked-in
`lake-manifest.json` pins Mathlib `v4.35.0` and all transitive dependencies.
Fast CI never builds this package.

`lean/aeneas/StcGenerated/Exact/Model.lean` is the filtered generated model. It
is `409` lines: a `12`-line provenance header naming the source path
`crates/stc-codegen/src/exact.rs`, source SHA-256
`85f3fbfc5263c918f80fc52b46322510de6645bef66c3fc078c7ba4ee826cba4`,
Aeneas `nightly-2026.11.02-ab34bd2`, Charon `0.1.304 (7b462d9)`, and the
generation recipe, followed by the `397`-line generated body. The body SHA-256
is `95a0ba5c896b7c768921577d79384e0a583cb6a7c550673115fbb01c7552553d`,
byte-identical to the accepted Phase 1 spike. The generated body is never
hand-edited.

`Stc.Bridge.Exact.Encode` and `Stc.Bridge.Exact.Externals` are core Lean and do
not import Aeneas. `Bridge/Exact/Refinement.lean` states the per-operation
propositions over the generated surface; `compareRefines` and `selRefines` are
proved (`compare_refines`, `sel_refines`). `Bridge/Exact/Audit.lean` inventories
the four external axioms and all 24 obligations: 2 proved, 22 open.

The four external axioms remain open: `I64::from(u8)`, `I64::rem_euclid`,
`I128::checked_neg`, and `I128::abs`. The mathematical models and their guard
lemmas are stated in `Externals.lean`, but the generated equations are not
discharged.

Bridge scope limitation: the filtered model does not contain `convert`,
`min_list`, `max_list`, `limit`, `mux`, `len`, `len_max`, or `len_code_unit`.
A wider regeneration was tested on the pinned host; it adds string/iterator
external axioms and emits a generated `sorry` for the `mux` closure, so it is
not accepted. Those operations keep model, theorem, and vector evidence, but
their refinement is remaining work and a wider model is an owner bridge-scope
decision.

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

The existing `constant_folding_agrees_with_exact_vectors` test in
`crates/stc-codegen/tests/lean_vectors.rs` continues to compare the analyzer's
folding with the committed generated vectors, which are themselves checked
against `exact.rs`. Shifts, rotates, `ABS`, and the selection functions have no
constant-folding syntax in `consteval.rs`, so they remain model-plus-vector
evidence rather than differential evidence.

This is V0 agreement evidence, not a refinement proof, and it does not unify
the three Rust paths. `exact.rs`, `consteval.rs`, and codegen
`cast_value`/`int_convert` remain duplicated implementations.

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
  crates, including the new differential test.

Bridge package check on the pinned Ubuntu host: `lake build` in `lean/aeneas`
completed `1806` jobs. This is manual/nightly evidence and is not part of fast
CI.

Fast CI on the final WL-3 HEAD is recorded in the final handoff for this task.

## 8. Trusted computing base

- The Lean 4.35.0 kernel and standard library, plus Batteries for executable
  models.
- The Phase 0 definitions and the frozen `semantics_version` `1.0`.
- The generated Aeneas model and its provenance pins; Aeneas, Charon, and
  Mathlib are trusted for the bridge package only.
- The four external axioms remain assumptions; they are not discharged.
- The Rust differential tests use the Rust standard library and the analyzer's
  own constant folder as the implementation side of the comparison.
- Rust `i128` intermediates: the reference model and the production paths trap
  when an intermediate exceeds `i128`, while the Lean model is unbounded; the
  boundary tests cover the shared scalar widths, and the residual difference is
  recorded rather than assumed away.

No theorem in Phase 2 constrains the Rust compiler. The Rust implementation
remains V0/model-tested for these kernels.

## 9. Remaining work and owner decisions

Remaining Phase 2 work before the phase can claim V1:

1. Discharge or explicitly accept the four external Aeneas axioms.
2. Prove the 22 open refinement obligations, including the generated helper
   arithmetic and bitwise operations.
3. Extend the generated model to `convert`, `min_list`, `max_list`, `limit`,
   `mux`, and the string-length helpers, or record an owner-approved scope
   exclusion; the tested wider regeneration is not acceptable because it emits
   a generated `sorry`.
4. Compose the per-operation obligations into a statement about the Rust
   functions that run in the build, and add the constant-certificate path or
   proved-equivalent kernel required by the program plan.
5. Prove `consteval_agrees_with_codegen` for the shared scalar subset or replace
   the three Rust paths with one verified kernel.
6. Close the equal-width conversion fit gap if the planned `convert_ok_fits`
   statement is required.

Owner decisions, in the order they block progress:

- whether to record a V1 claim only after the obligations and axioms are
  reviewed;
- whether to unify or refactor the three production Rust paths, and in which
  direction;
- whether to widen the bridge scope and accept the additional external axioms
  and regeneration recipe;
- whether any toolchain pin or accepted Phase 1 architecture decision changes.

The implementing agent does not make any of these decisions. Until the owner
responds, the correct status is V0/model-verified and implementation-tested, and
the working plan stays in the repository.

## 10. Retirement status

This report carries the durable Phase 2 content: milestone status, theorem
inventory, bridge scope limits, TCB, residual risks, and the owner gate list. The
temporary `WL-3-Phase-2.md` is not deleted and this report does not claim owner
acceptance. On acceptance, the working plan's operational rows can be removed
after confirming that this report, `Semantics.md` §7, and
`lean/verification-status.toml` carry the same evidence and levels.
