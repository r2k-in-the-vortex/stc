# Phase 2 Report — scalar semantic kernel, exact operations, and constant evaluation

Profile: `stc-2013-oop-utf8-v1`

Status: Phase 2 is complete and owner-accepted on 2026-11-03. The owner
accepted the exact scalar kernel at V1, the documented TCB, the single-kernel
construction plus certificate checker for the folding path, and the retirement
of the temporary working plan. This report records the work completed in WL-3,
the evidence produced, the V1 scope and its trust boundary, the parts that
remain model-level or V0, and the residual follow-ups.

Starting commit: `c82e521` (Phase 1 owner-accepted 2026-11-03).
Phase 2 commits through this report: `7520ae1` (exact property lemmas),
`5785011` (constant-evaluation model and proofs), `3d12a46` (bridge package and
generated model), `6a91ba4` (Rust differential certificate), `c790a40` and
`3ad3419` (owner decisions and fast-CI evidence), `ed8097e` (single kernel and
widened model), `a75a9cb` (24 exact-model refinement proofs), `bbba1e6` (wide
model, 32 proofs, bundled kernel), `813b148` (constant certificates), `5d28598`
(registry, report, and TCB), and `3d53cdd` (final CI evidence). The acceptance
commit is recorded in the task handoff.

## 1. Scope and claim discipline

The Phase 2 scope in
[VerificationProgramPlan.md](../VerificationProgramPlan.md#phase-2-scalar-semantic-kernel-exact-operations-and-constant-evaluation)
is the exact scalar kernel, its constant-evaluation model, the exact bridge, and
the certificate or unification of the three Rust implementations of the same
rules. The owner accepted the phase on 2026-11-03 with these decisions:

1. record one V1 claim, for the exact scalar kernel only;
2. accept the documented TCB rather than discharging its axioms in this phase;
3. accept the single-kernel construction plus the build-time certificate
   checker as the realization of `consteval_agrees_with_codegen`, explicitly
   without a literal Lean theorem;
4. retire the temporary `WL-3-Phase-2.md` working plan.

The claim ladder in §3 of the program plan is otherwise unchanged:

- the Phase 0 S-level semantics remains the source-semantics claim;
- exactly one Phase 2 record claims V1: `P2-T33`,
  `Stc.Bridge.Exact.exact_kernel_refines` in `Bridge.Exact.Kernel`. It covers
  the exact scalar kernel generated from `crates/stc-codegen/src/exact.rs`
  only, relative to the TCB listed in §8;
- all other Phase 2 records are proved facts about the Lean models at level V0
  (model-verified; implementation tested against the model), or V0 evidence;
- no external Aeneas axiom is discharged; the V1 claim is relative to the
  modeled operation axioms and the string equations recorded in the TCB;
- the Rust production paths delegate to one shared kernel, and the
  constant-folding path is tied to it by the certificate checker; the rest of
  the compiler remains V0;
- no `sorry` or `admit` exists in the default package or in the checked-in
  bridge modules, and the default `lean/` build imports neither Aeneas nor
  Mathlib.

## 2. Milestones

| ID | Milestone | Status | Evidence |
| --- | --- | --- | --- |
| P2-M1 | Working plan | Done, retired | `WL-3-Phase-2.md` (git history) and this report |
| P2-M2 | Exact property lemmas | Done | `Stc.Exact.{Int,Bits,Conv,Select,Strings}` extensions; `lake build` clean |
| P2-M3 | Constant-evaluation model | Done for the modeled subset | `Stc/ConstEval.lean`; `constEval_sound`, `constEval_type_preserving`; limitations in §4 |
| P2-M4 | Exact bridge package | Done | `lean/aeneas/` package, pinned manifest, widened checked-in generated model; host build of `Bridge.Exact.Audit` green |
| P2-M5 | Bridge obligations | Done, owner-accepted | 32 obligations, 32 proved, 0 open; twelve generated axiom declarations and the bridge string equations recorded in the TCB; `P2-T33` is the single V1 claim |
| P2-M6 | Rust kernel certificate | Done | one shared `exact.rs` kernel; constant certificates verified before IR emission; differential tests and vector tests |
| P2-M7 | Registry and CI | Done | P2 records, the owner-approved V1 exception and textual guard, sorry-grep coverage; certificate tests run in `cargo test --workspace` |
| P2-M8 | Durable report and fast CI | Done; owner-accepted 2026-11-03 | This report, the green fast CI run, and the retired working plan |

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
`Audit.kernelStatement_proved`.

The owner accepted Phase 2 on 2026-11-03 and recorded `P2-T33`,
`Stc.Bridge.Exact.exact_kernel_refines` in `Bridge.Exact.Kernel`, as the single
V1 claim. The claim covers the exact scalar kernel generated from
`crates/stc-codegen/src/exact.rs` only, relative to the documented TCB:
Aeneas/Charon translation, the nine operation axioms, the three non-equation
declarations, the five string equations, the opaque `strScalars` decoding
function, and the unproved Rust certificate verifier described in §6. It makes
no claim about the rest of the compiler. The other bridge records remain V0
`tested` evidence.

Because the bridge package imports Aeneas/Mathlib and is never built in fast
CI, the registry checker allowlists this one bridge-only record: it skips the
default-package `#print axioms` audit and instead requires the module to
declare `exact_kernel_refines` textually. Every default-package proved record
keeps the full axiom audit. The full proof is compiled by the pinned host
build of `Bridge.Exact.Audit`.

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
stays byte-identical. The owner accepted this construction for the folding path
on 2026-11-03 and explicitly did not claim a literal Lean theorem. The Rust
certificate verifier is therefore part of the V1 TCB for the folding path: it
is build-time enforcement code, not a proved checker.

The existing `constant_folding_agrees_with_exact_vectors` test in
`crates/stc-codegen/tests/lean_vectors.rs` continues to compare the analyzer's
folding with the committed generated vectors, which are themselves checked
against `exact.rs`. Shifts, rotates, `ABS`, and the selection functions have no
constant-folding syntax in `consteval.rs`, so they remain model-plus-vector
evidence rather than differential evidence.

This is V0 agreement and build-time enforcement evidence for the folding path,
not a Lean refinement proof of the compiler. The V1 kernel theorem constrains
the generated model of `exact.rs`; it does not constrain the Rust certificate
verifier or the lowering pipeline.

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

Fast CI on the pre-acceptance HEAD `3d53cdd` passed (run `41716346782`, all 37
jobs). The run for the acceptance commit is recorded in the final handoff.

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
- `strScalars`, the shared opaque decoding function, and the five string TCB
  equations (the four operation equations plus the UTF-8 byte-length invariant)
  are TCB assumptions; they are not discharged.
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

The V1 kernel claim is relative to exactly this TCB. One Phase 2 theorem now
constrains the Rust exact kernel: `exact_kernel_refines` is the V1 refinement
claim for the generated model extracted from
`crates/stc-codegen/src/exact.rs`. It does not constrain the rest of the
compiler: HIR-to-IR lowering, IR emission, the Rust certificate verifier, and
the constant-evaluation path remain V0/model-tested, and no Phase 2 theorem
constrains them.

## 9. Residual follow-ups

Phase 2 is owner-accepted; no implementation work or owner decision blocks the
V1 kernel claim. The follow-ups below are optional or belong to later phases
and are not conditions of the V1 record:

1. A future phase could discharge the modeled external operation axioms and
   the string TCB equations instead of accepting them. `P2-T33` is relative to
   them as recorded in §8.
2. A literal Lean `consteval_agrees_with_codegen` theorem would require a Lean
   model of HIR-to-IR lowering and a proved certificate checker. Phase 2
   explicitly claims the single-kernel construction plus the build-time checker
   instead; building the lowering model is new scope.
3. The non-default bridge package remains manual/nightly evidence. Adding it to
   a scheduled job is a CI-cost decision, not a claim change.
4. Toolchain pins and accepted Phase 1 architecture decisions stay fixed;
   changing one is a separate owner decision.

## 10. Retirement status

The owner accepted Phase 2 on 2026-11-03. The durable content is in this
report, `lean/verification-status.toml`, `Semantics.md` §7.8, and the program
plan; the temporary `WL-3-Phase-2.md` working plan was deleted after that
content was transferred and remains in git history. Its four owner decisions
are summarized in §1. The registry records exactly one Phase 2 V1 claim
(`P2-T33`); every other Phase 2 record remains V0.
