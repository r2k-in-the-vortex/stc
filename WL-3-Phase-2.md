# WL-3 Phase 2 working plan

Temporary execution plan for Phase 2 of the verification program
(`specification/Verification/VerificationProgramPlan.md` §Phase 2). This file is
not an enduring specification. The durable report is
`specification/Verification/Reports/Phase2-Report.md`; theorem claims live in
`lean/verification-status.toml` and `specification/Verification/Semantics.md`.

Profile: `stc-2013-oop-utf8-v1`.

## 1. Starting point and constraints

- Starting commit: `c82e521` on `WL-3` (Phase 1 owner-accepted 2026-11-03).
- Phase 0 S-level semantics is the only reviewed semantic claim. Phase 1 bridge
  evidence is V0.
- Phase 2 targets V1 evidence, but no record is marked V1/proved until the
  refinement theorem (or proved-equivalent implementation) exists and has owner
  review.
- No `sorry` or `admit`. The default `lean/` package never imports Aeneas or
  Mathlib. Generated and Mathlib-bearing content stays in `lean/aeneas/`.
- The pinned bridge environment is the Ubuntu host described only in the
  gitignored `.env/RT_Host.md`. Host addresses, credentials, and `.env` content
  are never committed. Regeneration is manual/reviewed and never runs in fast
  CI.
- `lean/bridge-toolchain.toml` is authoritative for pins and source hashes.
  A pin change requires owner review.

## 2. Milestones

| ID | Milestone | Deliverable | Exit condition |
| --- | --- | --- | --- |
| P2-M1 | Working plan | This file | Plan committed; program-plan status updated as batches land |
| P2-M2 | Exact property lemmas | `Stc.Exact.{Int,Bits,Conv,Select,Strings}` extensions | `lake build`; no `sorry`; named lemmas cover arithmetic, shifts/rotates, conversions, selection, strings |
| P2-M3 | Constant-evaluation model | `Stc.ConstEval` model and relation | Executable `constEval`, declarative `ConstEvalRel`, `constEval_sound`, conditional `constEval_type_preserving`, or precisely recorded partial results |
| P2-M4 | Exact bridge package | `lean/aeneas/` Lake package, filtered generated model, header | Generated core model checked in byte-identically to the pinned spike; typechecks on the pinned host; four external axioms enumerated |
| P2-M5 | Bridge obligations | `Stc.Bridge.Exact.{Encode,Externals}` and `aeneas/Bridge/Exact/{Refinement,Audit}` | External models and equations stated; per-operation refinement propositions named; no V1 claim |
| P2-M6 | Rust kernel certificate | Differential tests across `exact.rs`/`consteval.rs`/codegen conversion | Tests pass for the shared scalar subset; production unification deferred to owner |
| P2-M7 | Registry and CI | P2 records, checker phase policy, sorry-grep coverage | Registry audit passes; fast CI stays free of Aeneas/Mathlib |
| P2-M8 | Durable report and fast CI | `Phase2-Report.md`, plan/status reconciliation | Report records evidence, gaps, and TCB; final WL-3 HEAD passes fast CI |

## 3. Theorem inventory

The inventory maps each exact operation family to the model, the property
lemmas, and the evidence level. A name in this table is added only when the
lemma exists; planned names are not registry claims.

### 3.1 Integer arithmetic (`Stc.Exact.Int`)

| Operation | Existing model | Property lemmas |
| --- | --- | --- |
| `add` | `IntType.add`/`checked` | `add_ok`, `add_overflow`, `add_ok_iff`, `add_overflow_iff`, `add_comm`, `add_zero` |
| `sub` | `IntType.sub` | `sub_ok`, `sub_overflow`, `sub_zero` |
| `mul` | `IntType.mul` | `mul_ok`, `mul_overflow`, `mul_comm` |
| `neg` | `IntType.neg` | `neg_ok`, `neg_overflow`, `neg_neg` |
| `abs` | `IntType.abs` | `abs_ok`, `abs_overflow`, `abs_nonneg`, `abs_of_nonneg` |
| `div` | `IntType.div` | `div_ok`, `div_zero`, `div_overflow_iff`, `div_mod_identity` |
| `mod` | `IntType.mod` | `mod_ok`, `mod_zero`, `mod_overflow_iff`, `div_mod_identity` |
| bounds | `minimum`/`maximum`/`fits` | `fits_iff`, `fits_minimum`, `fits_maximum`, `minimum_le_maximum` |

The most-negative-divided-by-minus-one case is the `div`/`mod` overflow guard;
the truncating identity is `div_mod_identity`. Rust `i128` intermediate limits
are recorded as a bridge risk, not assumed away.

### 3.2 Bitwise, shifts, and rotates (`Stc.Exact.Bits`)

Range lemmas already exist for `bitAnd`, `bitOr`, `bitXor`, `bitNot`, `shl`,
`shr`, `rol`, and `ror`; commutativity/associativity exist for the bitwise
operators. Phase 2 adds count-masking lemmas (`shl`/`shr` at count zero and at
the width), the missing `ror_width`, and value identities (`bitAnd_self`,
`bitOr_self`, `bitXor_self`, `bitXor_zero`, `bitNot_zero`). A full
`rol`/`ror` inverse is not claimed unless a complete proof lands.

### 3.3 Conversions (`Stc.Exact.Conv`)

Widening, narrowing, equal-width signedness changes, BOOL conversion, and one
round-trip already have lemmas. Phase 2 adds BOOL zero/nonzero outcomes,
`convert_narrow_ok_fits`, `convert_widen_ok_fits`, and the corresponding
`evalCast` connections. A single `convert_ok_fits` covering equal-width
signedness reinterpretation is not yet proved; the equal-width cases remain a
recorded gap, not a claim.

### 3.4 Selection (`Stc.Exact.Select`)

`SEL` projection and identity, `MIN`/`MAX` bounds, `LIMIT` clamp/identity, and
`MUX` empty/zero/negative/past-end already exist. Phase 2 adds membership
(`sel_mem`, `minList_mem`, `maxList_mem`), in-range `MUX` selection, `LIMIT`
below/above cases, and range properties for `SEL`/`MIN`/`MAX`/`MUX`. Tie
breaking is recorded through the bound and membership lemmas; a first-index
tie-breaking theorem is not claimed unless proved.

### 3.5 Strings (`Stc.Exact.Strings`)

Byte-count range and ASCII, UTF-16 bounds, and the Unicode length/code-unit
inequality exist. Phase 2 adds supplementary-scalar lengths, aggregate UTF-8
byte bounds, per-kind length/code-unit bounds, and append/element identities
needed by a future string constant folder.

### 3.6 Constant evaluation (`Stc.ConstEval`)

Target surface: literals, constant variables, unary/binary operators, casts,
and the exact standard functions, over the Phase 0 `Value` and
`evalUnary`/`evalBinary`/`evalCast`/`evalStd` evaluators.

- `ConstError : Type` enumerates unbound/non-constant variables, type mismatch,
  malformed payloads, exact traps, and unsupported user-function calls.
- `ConstWorld` carries the typing context, the known constant values, and the
  `value_typed` invariant that every known constant has its context type and is
  well formed.
- `constEval : ConstWorld → Expr → Except ConstError Value` (with the
  left-to-right `evalArgs` companion);
- declarative `ConstEvalRel : ConstWorld → Expr → Value → Prop` and
  `ConstEvalRelArgs` for argument lists;
- `constEval_sound : constEval w e = .ok v → ConstEvalRel w e v`;
- `constEval_type_preserving : ExprHasType w.ctx e ty → constEval w e = .ok v →
  v.ty = ty ∧ v.WellFormed`, proved through `ConstEvalRel_typed` and the
  `ConstWorld.value_typed` invariant.
- An `rfl` smoke test checks a constant `DINT` addition through the executable
  evaluator.

Function calls, real/date/time values, and source-syntax folding are out of the
modeled subset and are recorded as such. The model does not claim agreement with
`consteval.rs`; that differential certificate is milestone P2-M6.

### 3.7 Rust differential certificate

`crates/stc-codegen/tests/exact_differential.rs` compares the production Rust
paths with `crates/stc-codegen/src/exact.rs` on the shared scalar subset:

- `constant_arithmetic_matches_the_exact_reference` folds `+`, `-`, `*`, `/`,
  `MOD`, unary `-`, bitwise `AND`/`OR`/`XOR`/`NOT`, and the six comparisons on
  every integer/bit-string type and boundary values, requiring the folded value
  or constant failure to match `exact.rs`.
- `constant_conversions_match_the_exact_reference` folds every ordered
  conversion pair accepted by the front end (178 of the 256
  `CONV_TYPES` pairs, `842` boundary cases) and requires the value, `CHAR`
  payload, or range failure to match `exact::convert`.
- `runtime_conversions_emit_the_exact_model_shape` generates a runtime
  conversion for each conversion class and checks the IR: widening is
  `sext`/`zext`, narrowing is a range check plus `trunc`, equal widths are a
  trap-free reinterpretation, and `BOOL` targets test against zero.

The existing `constant_folding_agrees_with_exact_vectors` test continues to
check the generated vector corpus. Shifts, rotates, `ABS`, and the selection
functions have no constant-folding syntax in `consteval.rs`, so they remain
model-plus-vector evidence, not differential evidence. These tests are V0
implementation agreement; they do not replace a refinement proof, and the three
production paths remain duplicated pending an owner decision.

## 4. Bridge plan

- Regenerate the filtered exact model from the pinned host with the Phase 1
  root set (`add`, `sub`, `mul`, `neg`, `div`, `modulo`, `abs`, bitwise,
  shifts/rotates, `compare`, `sel`). The output is byte-identical to the
  accepted Phase 1 spike (`397` lines, four axioms).
- Check it in at `lean/aeneas/StcGenerated/Exact/Model.lean` with a provenance
  header naming the source path, source SHA-256, Aeneas, Charon, and the
  generation command. The header is added by a reviewed regeneration step; the
  generated body is never hand-edited. Header plus body is `409` lines; the
  body is byte-identical to `.tmp/Exact.lean` (SHA-256
  `95a0ba5c896b7c768921577d79384e0a583cb6a7c550673115fbb01c7552553d`).
- The non-default package is `lean/aeneas/`: `lakefile.lean` requires Aeneas
  from git at `ab34bd28458a08c150b7cbf1873e59af74588138`, subdirectory
  `backends/lean`, and the default `stc` package from `..`. The generated
  manifest pins Mathlib `v4.35.0` and all transitive dependencies. The package
  built successfully on the pinned Ubuntu host (`lake build`, `1806` jobs).
- `Stc.Bridge.Exact.Encode` decodes generated layouts, traps, checked results,
  and unsigned bit patterns to `Stc.Exact` without importing Aeneas.
- `Stc.Bridge.Exact.Externals` models the four external operations:
  `I64::from(u8)`, `I64::rem_euclid`, `I128::checked_neg`, `I128::abs`,
  including the nonzero-divisor and `MIN` guard facts.
- `lean/aeneas/Bridge/Exact/Refinement.lean` states the per-operation
  refinement propositions over the generated surface. `compareRefines` and
  `selRefines` are proved (`compare_refines`, `sel_refines`); the remaining
  `22` obligations are stated `Prop`s with no proof and no V1 claim.
  `Audit.lean` inventories the four generated axioms, all stated obligations,
  and the two proved facts.
- Bridge scope limitation: the Phase 1 filtered model does not contain
  `convert`, `min_list`, `max_list`, `limit`, `mux`, `len`, `len_max`, or
  `len_code_unit`. A wider regeneration was tested: it adds string/iterator
  external axioms and emits a generated `sorry` for the `mux` closure, so it is
  not accepted. Those functions keep model + theorem + vector evidence; their
  refinement is remaining Phase 2 work and an owner bridge-scope decision if a
  wider model is required.

## 5. Owner gates

Stop and request an owner decision before:

- recording any V1 claim, discharging the four external axioms as proved, or
  changing a Phase 0/Phase 1 claim;
- unifying or refactoring the three production Rust kernel paths (`exact.rs`,
  `consteval.rs`, codegen `cast_value`/`int_convert`);
- widening the bridge scope to new external axioms, a wider generated model,
  or a new pin/regeneration recipe;
- changing `semantics_version`, a toolchain pin, or an accepted Phase 1
  architecture decision.

## 6. Verification per batch

- `lake build` and `lake exe stc-sem-tests` in `lean/`.
- `python tools/ci/check_verification_registry.py`.
- `python tools/ci/check_docs.py` and the pinned markdownlint command.
- Sorry/admit grep over `Stc/Exact`, `Stc/ConstEval`, `Stc/Bridge`, and
  `aeneas/Bridge`.
- `python tools/ci/check_bridge_toolchain.py` (no Aeneas needed). The
  `--require-generated` form is not used yet because the Phase 1 `wire` model
  is still not checked in.
- Bridge package only, on the pinned Ubuntu host: `lake build` in
  `lean/aeneas` (compiles Aeneas/Mathlib and the generated model). This is
  manual/nightly and never part of fast CI.
- `cargo fmt --check`, `cargo clippy`, and targeted `cargo test` for changed
  Rust.

## 7. Retirement protocol

This plan is retired only after Phase 2 owner acceptance. Before deletion:

1. move the milestone status, theorem inventory, bridge scope limits, TCB, and
   residual risks into `specification/Verification/Reports/Phase2-Report.md`;
2. update `VerificationProgramPlan.md` Phase 2 status and exit assessment;
3. update `Semantics.md` §7 and `lean/verification-status.toml` with the same
   evidence and levels;
4. keep the working plan in git history and remove only stale operational
   content from the working tree.

Until acceptance, this file stays in the repository. No deletion and no
"owner accepted" statement is made by the implementing agent.

## 8. Status board

| Item | Status |
| --- | --- |
| Working plan | Created |
| Exact integer property lemmas | Done: success/overflow characterizations, MIN/-1 division and remainder, range extrema |
| Bitwise/shift/rotate lemmas | Done: count-zero/width identities, `ror_width`, bitwise value identities; full rotate inverse remains open |
| Conversion lemmas | Partial: BOOL outcomes, narrow/widen fit; equal-width reinterpret fit remains open |
| Selection lemmas | Done: membership, clamp cases, in-range MUX, range properties |
| String-length lemmas | Done: supplementary scalars, byte/UTF-16 bounds, append, per-kind length bound |
| Constant-evaluation model | Done: executable evaluator, declarative relation, soundness, type preservation over literals/constants/unary/binary/cast/std; user-function calls, real/date/time, and source folding remain out of the modeled subset |
| Exact bridge package and generated model | Done: `lean/aeneas` package, pinned manifest, checked-in 409-line model with provenance header, host build `1806` jobs |
| External-axiom models and refinement propositions | Done: four mathematical models and guard lemmas, 24 stated obligations, 2 proved (`compare_refines`, `sel_refines`), 22 open, no V1 claim |
| Rust differential certificate | Done: `crates/stc-codegen/tests/exact_differential.rs` checks arithmetic/bitwise/comparison folding and all front-end-accepted conversion pairs against `exact.rs`; the existing vector tests cover the generated corpus; production unification deferred to owner |
| Registry/docs/CI updates | Planned |
| Phase 2 report | Planned |
| Fast CI on final WL-3 HEAD | Planned |
