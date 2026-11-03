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
`convert_ok_fits` (a successful non-BOOL conversion fits the destination), and
the `evalCast` fit connection. Conversion totality on source-fitting values is
stated where the proof is complete.

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

- `constEval : ConstWorld → Expr → Except ConstEvalError Value`;
- declarative `ConstEvalRel : ConstWorld → Expr → Value → Prop`;
- `constEval_sound : constEval w e = .ok v → ConstEvalRel w e v`;
- `constEval_type_preserving : ExprHasType ctx e ty → constEval w e = .ok v →
  v.ty = ty ∧ v.WellFormed`, under a world-constant well-formedness invariant.

Function calls, real/date/time values, and source-syntax folding are out of the
modeled subset and are recorded as such.

## 4. Bridge plan

- Regenerate the filtered exact model from the pinned host with the Phase 1
  root set (`add`, `sub`, `mul`, `neg`, `div`, `modulo`, `abs`, bitwise,
  shifts/rotates, `compare`, `sel`). The output is byte-identical to the
  accepted Phase 1 spike (`397` lines, four axioms).
- Check it in at `lean/aeneas/StcGenerated/Exact/Model.lean` with a provenance
  header naming the source path, source SHA-256, Aeneas, Charon, and the
  generation command. The header is added by a reviewed regeneration step; the
  generated body is never hand-edited.
- `Stc.Bridge.Exact.Encode` decodes generated/primitive representations to
  `Stc.Exact` without importing Aeneas.
- `Stc.Bridge.Exact.Externals` models the four external operations:
  `I64::from(u8)`, `I64::rem_euclid`, `I128::checked_neg`, `I128::abs`.
- `lean/aeneas/Bridge/Exact/Refinement.lean` states the per-operation
  refinement propositions (`exact_add_refines`, `exact_div_refines`,
  `exact_sel_refines`, and the rest of the generated surface). No proposition
  is marked proved until the equation is completed. `Audit.lean` inventories
  the axioms and the unresolved obligations.
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
- `python tools/ci/check_bridge_toolchain.py` (no Aeneas needed).
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
| Exact integer property lemmas | Planned |
| Bitwise/shift/rotate lemmas | Planned |
| Conversion lemmas | Planned |
| Selection lemmas | Planned |
| String-length lemmas | Planned |
| Constant-evaluation model | Planned |
| Exact bridge package and generated model | Planned |
| External-axiom models and refinement propositions | Planned |
| Rust differential certificate | Planned |
| Registry/docs/CI updates | Planned |
| Phase 2 report | Planned |
| Fast CI on final WL-3 HEAD | Planned |
