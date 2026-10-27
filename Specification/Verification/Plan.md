# Verification Program Plan — Lean Verification of stc

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-10-27
Status: Draft for review. Phase 0 has not started; no verification claim beyond the existing
exact-function models is made by this document.
Inputs: [Testing and Verification](../Architecture/Testing-and-Verification.md),
[Architecture](../Architecture/Architecture.md),
[GOT Artifacts and Schemas](../Architecture/GOT-Artifacts.md),
[Online Change and GOT](../Architecture/Online-Change-and-GOT.md),
[Runtime and Execution](../Architecture/Runtime-and-Execution.md),
[AST, CST, and HIR](../AST/AST.md),
[Grammar](../Syntax/Grammar.md),
[Tokenization](../Syntax/Tokenization.md),
[Decisions](../Decisions.md).

## 1. Purpose

The goal is to verify as much of `stc` as possible in Lean 4.35.0, starting with the targets
where a silent wrong answer is most damaging and where a proof is most tractable. "As much as
possible" is not "all 131k lines of Rust": the plan maximizes proved decision logic and
artifact validation under an explicit trust boundary, and it states the verification level of
every target rather than implying uniform coverage.

The plan is a verification layer over the existing Rust compiler, not a Lean rewrite. A
Lean-first compiler is a separate, larger decision (D-V10); the recommended path keeps the
product architecture and moves the highest-risk kernels and artifact boundaries into Lean's
checked world.

The honest end claim, if the program succeeds, is:

> A machine-checked proof that the covered language subset's translation from source to textual
> LLVM IR preserves the source semantics, with proved kernels and artifact validators for the
> remaining critical paths; clang/LLVM, the textual printer boundary, the operating system, and
> hardware remain in the trusted computing base.

This is deliberately weaker than "the compiler is formally verified". LLVM is not verified, the
project is not a safety PLC, and verification is not certification. The claim ladder in §3 makes
the boundary explicit.

## 2. Starting point

The repository already has real assets and one important gap.

Assets:

- `lean/Stc/Exact/` models checked integer arithmetic, bitwise operations, shifts and rotates,
  conversions, selection, boolean operations, and string-length operations.
- `lake exe vectors` generates deterministic JSON vectors consumed by
  `crates/stc-codegen/tests/lean_vectors.rs`; 30 vector files are committed.
- The Lean project builds cleanly (`lake build`, 9 jobs) and CI has a `lean` job.
- GOT format 1 is precisely specified in [GOT Artifacts](../Architecture/GOT-Artifacts.md),
  with canonical encodings, digests, validation rules, slot assignment, and plan semantics.
- `stc-codegen` already has a structural module verifier, and `stc-got` already has plan
  validation and manifest/descriptor codecs.
- The repository has extensive snapshot, conformance, execution, fuzz, and online-change tests,
  which are the right differential oracle for Lean models.

The gap:

- Proofs currently cover the Lean model, and the Rust implementation is tested against generated
  vectors (A-006). That is a model-level claim, not a proof about the compiler.
- `crates/stc-codegen/src/exact.rs` is used by the vector test only. The production paths in
  `crates/stc-sema/src/consteval.rs` and `crates/stc-codegen/src/expr.rs` implement the same
  arithmetic and conversion rules again. The comment on `convert_int_const` says it "mirrors
  code generation (`cast_value`/`int_convert`)", which is exactly the duplication that a
  verification program should eliminate or prove equal.
- No proof currently connects a Lean model to a Rust function, an emitted artifact, or a runtime
  transition. The first priority is therefore not "more Lean models"; it is a bridge from Lean
  evidence to the compiler's actual behavior.

The gap is the central risk of the whole program: a large library of correct proofs about
models that the compiler does not use would not reduce the chance of a miscompiled PLC program.
The new Phase 0 gives the program a precise, checked semantics to aim at; Phase 1 closes the
implementation gap before the proof effort scales.

## 3. Verification levels and claim language

Every target is assigned one level. Claims in documentation, pull requests, and release notes
must name the level and the trusted boundary.

| Level | Evidence | Allowed claim |
| --- | --- | --- |
| S | Formal semantics: AST/HIR/IR definitions, static and dynamic judgments, meta-theoretic soundness theorems, and a reference interpreter; the Rust implementation is compared by conformance only | "semantics defined and proved sound; implementation tested against it, not proved" |
| V0 | Lean model plus generated vectors; Rust compared by tests | "model-verified; implementation tested against the model" |
| V1 | Lean model plus a refinement proof for the Rust implementation (Aeneas/Charon) or a proved-equivalent extracted implementation | "implementation proved equivalent to the model" |
| V2 | A proved Lean checker validates the actual production artifact or input | "artifact validated by a proved checker" (translation validation) |
| V3 | Lean-extracted code is the implementation used on the production path | "verified code in the trusted path" |
| V4 | Machine-checked semantic-preservation theorem for a defined subset, composed across phases | "verified compiler for the defined subset up to the stated boundary" |

Rules:

- S is the semantic reference, not an implementation level. A phase may report S for the
  semantics artifact while the Rust compiler remains V0 for that component. S does not imply
  V1; a conformance harness is evidence, not refinement.
- A V0 result may be reported as a model result, never as a compiler-correctness result.
- V1 requires a proof about the Rust function that runs in the build, or a build that uses the
  extracted function. A proof about a hand-port that is not executed is V0.
- V2 requires the checker to run on the artifact that is actually shipped or on every module the
  compiler emits in CI; a checker that only inspects test fixtures is V0/V1.
- V4 requires a named subset, a named source semantics, a named target semantics, and a stated
  TCB. Coverage outside the subset remains V0/V1/V2.
- No level is implied by another. A V4 codegen theorem does not make the parser V4; a V1 kernel
  does not make the surrounding compiler V1.

## 4. Prioritization

Targets are ranked by four criteria:

- Severity: a silent wrong value in a running PLC is worse than a crash, which is worse than a
  diagnostic or CLI defect.
- Likelihood and bug density: arithmetic and conversions, layout/identity, constant folding, and
  code generation are the highest-risk areas; the parser is heavily tested and its failures are
  usually loud.
- Tractability: pure, discrete, well-specified kernels are far cheaper to prove than an entire
  front end or a general LLVM semantics.
- Leverage: shared semantic kernels and artifact boundaries support several later phases;
  isolated feature proofs do not.

Pipeline order is the dependency order, not the value order. The natural pipeline sequence
(tokenizer, parser, semantic analysis, codegen) and the composition/TCB framing are kept, but
the work is sequenced by risk and tractability. The plan front-loads semantic kernels and
artifact validity because that is where silent failures live and where proofs are tractable.
The parser and the full type system come later even though they occur earlier in the pipeline.

### 4.1 Priority ranking

| Rank | Target | Severity | Tractability | Leverage | Phase | Target level |
| --- | --- | --- | --- | --- | --- | --- |
| 0 | Formal AST/HIR/IR and sound source/target semantics | Foundation | High | Very high | 0 | S |
| 1 | Exact scalar semantics and constant evaluation | High | High | High | 2 | V1/V3 |
| 2 | GOT identity, layout, plan, and artifact codecs | High | High | High | 3 | V1/V2 |
| 3 | Lexer, CST, formatter, and expression round-trip | Medium-high | High | Medium | 4 | V1 |
| 4 | Emitted-IR validation (translation validation) | High | Medium | High | 5 | V2 |
| 5 | Core type system, name resolution, and HIR lowering | High | Medium | High | 6 | V1/V4 |
| 6 | Codegen semantic preservation | Very high | Low | High | 7 | V4 |
| 7 | Runtime swap, preservation, and scheduler invariants | High | Medium | Medium | 8 | V1/V2 |
| 8 | Full grammar soundness and completeness | Medium | Low | Medium | 4b | V1/V4 |
| 9 | Remaining language features (OOP, strings, references) | Medium | Low | Medium | 6/7/8 | V4 |
| 10 | End-to-end composition and published claims | Very high | Low | High | 9 | V4 |

### 4.2 Why this order

- Formal semantics first because it is a standalone deliverable: it fixes the meaning of the
  AST, HIR, and IR fragment, proves the core semantics sound, and gives the compiler a reference
  interpreter and conformance oracle. Every later proof cites these definitions.
- Exact semantics second because the compiler already folds constants and emits checked
  arithmetic, the Lean models exist, and a mismatch is silent. This phase also produces the
  implementation-level vocabulary (values, widths, traps, conversions) reused everywhere else.
- GOT format and plan third because online change is the product differentiator, the format is
  already specified precisely, the code is pure and bounded, and a wrong identity or layout can
  preserve the wrong value across a swap.
- Front-end round-trip fourth because it is cheap, catches real parser/formatter bugs, and
  establishes the grammar/CST vocabulary needed before a full parser-soundness proof.
- IR validation fifth because it is the cheapest backend assurance available: a proved checker
  can reject malformed, mistyped, non-SSA, or wrongly wired modules without a full LLVM
  semantics.
- The type system and full semantic preservation are the largest efforts. They are staged so
  each stage covers a real, useful subset and can stop with a defensible claim.
- Runtime invariants come after the GOT plan and can run in parallel with the type system and
  codegen work.

## 5. Phase overview

| Phase | Objective | Effort class | Depends on | Milestone claim |
| --- | --- | --- | --- | --- |
| 0 | Formal AST/HIR/IR, source and target semantics, soundness proofs, reference interpreter | L/XL | none | S-level checked semantics of the core language and IR fragment |
| 1 | Verification architecture, bridge decision, CI and registry | S/M | Phase 0 | Approved levels, bridge spike, no new claims |
| 2 | Scalar semantic kernel, exact operations, constant evaluation | L | Phases 0 and 1 | V1 exact kernel used by folding |
| 3 | GOT keys, layout, GotId, slots, codecs, plan | L | Phase 1 | V1/V2 format-1 and plan |
| 4 | Lexer, CST, formatter, round-trip, parser soundness | M/L | Phases 0 and 1 | V1 front-end and parser contracts |
| 5 | Emitted-IR validation | L | Phases 0 and 3 | V2 every emitted module checked |
| 6 | Type system, name resolution, HIR lowering | XL | Phases 0, 2, and 4 | V1/V4 core language |
| 7 | Codegen semantic preservation | XL | Phases 0, 2, 3, 5, and 6 | V4 core subset to IR |
| 8 | Runtime, swap, scheduler invariants | L | Phases 3 and 5 | V1/V2 swap protocol |
| 9 | Composition, coverage widening, claims | M ongoing | all | V4 for the defined subset |

Effort classes: S is up to one engineer-month, M is one to three, L is three to six, and XL is
six to twelve or more. These are order-of-magnitude estimates for an experienced Lean and Rust
engineer; Phase 0 and Phase 1 should replace them with a measured plan. The full program is
multi-year. Each phase is independently useful, and the program is designed so it can stop after
any phase and still have a true, published claim.

## 6. Phase details

### Phase 0 — Formal semantics foundation and soundness

**Goal.** Deliver a standalone, machine-checked formal semantics of the stc core language and
the emitted IR fragment, with the soundness results that make the semantics trustworthy. This
phase does not depend on the Rust bridge or on a codegen correctness proof. It gives the project
a precise, reviewable answer to "what does an stc program mean?" and "what does the emitted IR
mean?", and it proves the core semantics is well-defined, deterministic, type-sound, and
trap-explicit. In the claim ladder this is an S-level deliverable: it is the semantic
reference, not an implementation-refinement claim.

**Why this is a deliverable on its own.** A semantics that is merely a pile of definitions is
not verification. Phase 0 produces a checked semantic reference: the AST, HIR, and IR types;
the static and dynamic judgments; the meta-theoretic soundness results; an executable reference
interpreter; and a conformance report against the compiler's own corpus. That artifact is useful
even if no later phase runs: it pins down the language and target meaning, it is the oracle for
compiler tests, and it is the vocabulary every later theorem cites. It also prevents the program
from proving the wrong thing: a parser or codegen proof is only as meaningful as the semantics
it preserves.

**Scope.**

- `Stc.Syntax`: tokens, the lossless CST contract, and the surface AST. Define the AST as an
  inductive family and relate it to the parser's CST through the lossless and round-trip
  relations. The full rowan CST can remain a contract-level abstraction; the AST and its
  mapping are the priority.
- `Stc.Grammar`: a declarative grammar relation `Derives` for the core surface language, shared
  with Phase 4's parser contracts. Phase 0 defines the relation and the AST mapping; full parser
  soundness (`parse_sound`) and completeness (`parse_complete`) are Phase 4b, while Phase 4a
  proves the round-trip contracts.
- `Stc.Hir`: the typed HIR as an inductive family: types, expressions, statements, places,
  declarations, and programs. Align it with `stc-hir` and the typing rules in the specification.
- `Stc.Sem`: values, widths, memory, traps, the core declarative typing judgment, the core
  algorithmic checker (a Lean reference; the Rust checker is Phase 6), and a declarative
  evaluation relation for the core HIR subset: scalar
  types, expressions, assignments, control flow, functions, programs, GOT globals, and traps.
  Define the core subset boundary explicitly. Aggregates, strings, OOP, references, and the
  standard library are later phases.
- `Stc.Codegen.Ir`: the `IRFrag` syntax and the `WellFormed` predicate for the emitted IR
  fragment. This is the target syntax that Phase 5's checker and Phase 7's simulation consume.
- `Stc.Codegen.Semantics`: an operational semantics for `IRFrag`: integer and floating
  operations, memory, GOT cell loads and stores, indirect calls, descriptor/vtable dispatch,
  traps, and the host ABI boundary, for the same core subset.
- A reference interpreter for the core HIR subset, executable in Lean and usable as an oracle.
- A semantics-conformance harness: run the Lean reference interpreter and the Rust compiler's
  observable behavior over the existing conformance and execution corpus, and record agreement.
  The harness consumes a canonical HIR dump for the core subset, so Phase 0 does not need a full
  Lean front end. This is differential evidence, not a refinement proof; label it V0. The
  refinement bridge is Phase 1, and the exact/constant-evaluation refinement is Phase 2.
- A semantics document and version: the grammar/AST/HIR/IR mapping, the judgments, the core
  subset, the theorem list, and a changelog. The semantics is a product artifact, not an
  incidental proof helper.

**Soundness theorems to establish.**

- `eval_deterministic`: evaluation is a partial function; at most one result for a given
  expression and state.
- `progress`: a well-typed, well-formed core program either reaches a terminal state or takes a
  step, except for the documented trap and cycle boundaries.
- `preservation`: evaluation preserves typing for the core.
- `type_soundness`: a well-typed expression evaluates to a value of its type or traps for a
  documented reason.
- `core_check_sound`: the core algorithmic checker accepts only programs with a derivation in
  the core declarative typing judgment.
- `core_check_complete` where the core checker is intended to be complete; otherwise
  `core_rejection_characterized`, an explicit statement of what the core checker may
  over-reject.
- `trap_soundness`: every trap maps to a specified condition (overflow, divide by zero, range,
  bounds, null, conversion), and no unspecified trap occurs.
- `checked_arith_traps_exactly`: checked arithmetic traps exactly when the exact model overflows
  or divides by zero.
- `ir_deterministic`: the target semantics is deterministic on `IRFrag`.
- `ir_checked_arith_refines_exact`: the IR checked operations plus trap equal the existing
  `Stc.Exact` kernel.
- `ir_memory_safe`: well-formed IR accesses only valid stack storage, GOT cells of the correct
  kind, and valid descriptors; no uninitialized or dangling access on the covered fragment.
- `ir_got_sound`: a GOT cell load returns the value of the root recorded by the slot table; a
  function-cell call resolves to the recorded callable.
- `ir_trap_sound`: IR traps occur exactly on the specified trap paths; no undefined behavior on
  the covered fragment.
- `interpreter_sound` and, where feasible, `interpreter_complete`: the executable reference
  interpreter agrees with the declarative evaluation relation.

The conformance harness result is recorded but is not a theorem; it is V0 evidence that the
Rust compiler and the reference semantics currently agree on the corpus.

**Deliverables.**

- Lean modules for `Stc.Syntax`, `Stc.Hir`, `Stc.Sem`, `Stc.Codegen.Ir`, and
  `Stc.Codegen.Semantics`.
- `lake exe stc-sem`, the reference interpreter for the core subset.
- `lake exe semantics-conformance`, the corpus harness and report.
- The semantics document and a `semantics_version` changelog.
- Registry entries for every semantics definition and theorem, with specification references.

**Exit criteria.** The AST, HIR, and IR fragment are Lean inductive types; the source and target
semantics are defined for the named core subset; the soundness theorems above are proved with no
`sorry`; the reference interpreter runs the core corpus; the conformance report is committed;
the semantics document is reviewed and versioned; later phases can cite exact definitions and
theorem names. The result is recorded at level S; the Rust compiler remains V0 for these
components until Phase 1 and Phase 2 connect it.

**Bridge.** Phase 0 uses no Aeneas/Charon and does not require a Rust refinement. The Lean
semantics is the reference; the Rust compiler is compared through the conformance harness. The
implementation bridge is Phase 1, and the exact/constant-evaluation refinement is Phase 2. This
keeps Phase 0 standalone and prevents toolchain work from blocking the semantics deliverable.

**Risks.** The formal semantics may drift from the Rust implementation; the core subset may be
too small to be useful; progress and preservation are awkward for an imperative PLC language
with cycles, traps, and persistent state; the surface AST/CST representation may be hard to
model faithfully. Mitigate by defining the subset and its boundary explicitly, keeping the
reference interpreter executable and the conformance harness current, and treating the
semantics document as the review artifact.

**Effort.** L/XL. Defining and proving a core language semantics is the riskiest estimate in
the plan; Phase 0 should start with a timeboxed semantics skeleton and refine the estimate
before committing to the full theorem list.

### Phase 1 — Verification architecture and bridge

**Goal.** Decide how Lean evidence constrains the Rust compiler, build the infrastructure that
every later phase uses, and connect the Phase 0 reference semantics to the implementation.
Phase 1 produces no large correctness proofs; it produces the bridge and the tooling.

**Work.**

- Run a bridge spike on three representative Rust targets:
  - `crates/stc-codegen/src/exact.rs` (pure arithmetic and conversions),
  - `crates/stc-got/src/keys.rs` (string normalization and key construction),
  - one `stc-got` codec path (`wire.rs` primitive encode/decode or one descriptor section).
  Measure Aeneas/Charon translation coverage, toolchain pinning, proof obligations, and
  generated-code readability. Record the result in a short spike report.
- Choose the bridge per target class, using the decision matrix in §7.
- Define the Lean module layout:

  ```text
  lean/
    Stc/
      Syntax/        tokens, CST contract, surface AST
      Grammar/       core grammar relation shared with the parser proofs
      Hir/           typed HIR types, places, expressions, statements
      Sem/           values, widths, memory, traps, evaluation
      Exact/         existing exact models and proofs
      ConstEval/     constant-expression semantics and soundness
      Got/           keys, layout, GotId, slots, codecs, plan
      Frontend/      lexer, parser, formatter, grammar
      Sema/          typing, resolution, lowering
      Codegen/       IR fragment, validation, semantics, simulation
      Runtime/       host state machine, swap, scheduler
      Proofs/        composition, TCB, claim registry
    Vectors/         existing executable vector generator
    Verification/    executable checkers, oracle, registry tooling
  ```

  Phase 0 creates the `Syntax`, `Hir`, `Sem`, and core `Codegen` modules; Phase 1 adds the
  bridge and tooling around them.

- Define proof-engineering policy: no `sorry` or `admit`; top-level theorems audited with
  `#print axioms`; a documented tactic set; a pinned Lean toolchain; and a policy for Mathlib
  and Batteries (D-V2).
- Add a machine-readable verification registry, for example `lean/verification-status.toml`,
  with one record per target: phase, level, theorem name, specification section or rule ID,
  status, owner, and TCB note. Add an `xtask` check that every record resolves to a theorem and
  every top-level theorem is registered.
- Extend the Phase 0 `IRFrag` to the full emitted LLVM IR fragment for Phase 5 and Phase 7: the
  exact instructions, types, control-flow constructs, GOT access patterns, calls, traps,
  globals, and descriptor/function table shape that codegen may emit. Anything outside the
  fragment is out of scope until the fragment is extended.
- Fix the first end-to-end subset (D-V6), building on the Phase 0 core subset: recommended as
  scalar types, expressions, assignments, control flow, functions, programs, and GOT globals,
  without OOP, references, strings, or aggregates.
- Write the TCB and claim-language document (D-V4).

**Exit criteria.** Bridge spike report exists; D-V1 through D-V10 are resolved; the Phase 0
semantics is the reference for the bridge; the registry, no-sorry check, vector-regeneration
check, and oracle harness run in CI; the full IR fragment and first end-to-end subset are
written down; no component is claimed above V0 except the S-level Phase 0 semantics.

**Effort.** S/M.

### Phase 2 — Scalar semantic kernel, exact operations, and constant evaluation

**Goal.** Make the exact scalar semantics the single source of truth for folding and codegen
checks, and prove that the compiler's constant evaluator agrees with the Phase 0 source
semantics on the folded subset. Phase 0 defines the semantics; this phase connects the
compiler's folding path to it.

**Scope.**

- Confirm and complete coverage of the exact functions in
  [Testing and Verification §5.1](../Architecture/Testing-and-Verification.md): checked integer
  arithmetic, division and modulo, bitwise operations, shifts and rotates, conversions,
  comparisons, `SEL`/`MIN`/`MAX`/`LIMIT`/`MUX`, boolean operations, and string length/UTF-8
  operations. The existing models cover the operation list; the new work is the missing
  property lemmas, the implementation bridge, and the constant-evaluation soundness proof.
- Use the Phase 0 HIR expression model and evaluation relation; extend them with the constant
  forms and exact standard functions needed for folding: literals, paths to constants, unary
  and binary operators, conversions, and selection.
- Prove the algorithmic constant evaluator sound against the Phase 0 declarative relation.
- Unify the three Rust implementations of the scalar rules: the vector-test module
  (`exact.rs`), the analyzer's constant folder (`consteval.rs`), and codegen's
  `cast_value`/`int_convert`. The preferred end state is one kernel used by all three and proved
  equal to the Lean model (V1), or the Lean kernel extracted into the build (V3).
- Add a constant-certificate path: for every folded value that reaches the emitted IR, the
  compiler records the operand identity and the Lean-evaluated result; a proved checker can
  recompute the result from the operands and reject a mismatch.

**Theorems to establish.**

```lean
theorem add_ok_iff {ty : IntType} {a b r : Int} (ha : ty.fits a = true) (hb : ty.fits b = true) :
    ty.add a b = .ok r ↔ r = a + b ∧ ty.fits r = true

theorem add_overflow_iff {ty : IntType} {a b : Int} (ha : ty.fits a = true) (hb : ty.fits b = true) :
    ty.add a b = .error .overflow ↔ ty.fits (a + b) = false
```

The same shape covers `sub`, `mul`, `neg`, `abs`, `div`, and `mod`, including the
most-negative-divided-by-minus-one overflow and the truncating remainder identity. Additional
obligations:

- bitwise and shift/rotate range and masking lemmas; `rol`/`ror` inverses;
- conversion widening, narrowing, equal-width reinterpretation, `BOOL` conversion, and the
  round-trip conditions;
- `SEL`/`MIN`/`MAX`/`LIMIT`/`MUX` tie-breaking and range properties;
- string character count versus UTF-8 byte count, including supplementary scalars;
- `constEval_sound : constEval e = some v → Eval e v` for the modeled expression subset;
- `constEval_type_preserving`: a folded value fits its declared type;
- initializer, array-bound, case-label, and task-interval folding are instances of the same
  soundness theorem;
- `consteval_agrees_with_codegen` for the shared scalar subset.

**Bridge.** Aeneas/Charon refinement for `exact.rs` and the core of `consteval.rs`, if the
Phase 1 spike supports it; otherwise extract the Lean kernel to C/Rust and use it in the
compiler. If neither is practical, the kernel remains V0 and the claim stays "model-verified,
implementation-tested", which is not sufficient for Phase 2's exit.

**Exit criteria.** Every exact function has a model, a theorem set, and V1 evidence; the
constant-folding subset is proved sound; the compiler's folding path uses the verified kernel or
a proved-equivalent implementation; generated vectors cover every operation and type; CI has no
`sorry`; the registry records the level and TCB.

**Risks.** Rust `i128` intermediate arithmetic versus Lean's unbounded `Int`; `checked_*`
overflow behavior, including 64-bit multiplication where the exact product exceeds `i128` and
Rust's `checked_mul` returns `None`; floating-point, date/time, and platform-dependent
operations must remain out of the proved subset and be documented as such; string-encoding
scope may grow.

**Effort.** L.

### Phase 3 — Persistent identity, layout, plan, and artifact codecs

**Goal.** Prove the format-1 identity and online-change foundation: keys, type and signature
keys, layout hashes, `GotId` derivation, catalog and slot assignment, descriptor/manifest
codecs, build IDs, and plan classification and validation.

**Scope.**

- Model [GOT Artifacts](../Architecture/GOT-Artifacts.md) format 1: primitive encoding,
  canonical JSON, keys, type keys, signature keys, layout hashes, `GotId`, catalog records,
  descriptor format 1, manifest format 1, plan actions, build IDs, and pairing checks.
- Prove canonicalization and determinism, round-trip for the codecs, validation soundness, and
  the plan invariants that the runtime relies on.
- Prove the stable FNV-1a name and signature hash is deterministic and matches the specified
  byte sequence, and that mangled names are injective modulo the documented hash collision
  assumption.
- Prove the SHA-256 implementation (`stc-got/src/sha256.rs`) matches FIPS 180-4, or replace it
  with a verified implementation. Keep collision resistance as a separate cryptographic
  assumption; implementation correctness and collision resistance are different obligations.
- Prove artifact determinism: identical inputs produce identical catalog, descriptor, manifest,
  and metadata bytes, with canonical ordering and no time, path, or map-iteration inputs.
- Prove the Rust `keys.rs`, `layout.rs`, classification, and slot-assignment functions equal to
  the Lean models (V1), and build a proved decoder/validator for untrusted manifests and
  descriptors (V2). Run it over the fuzz corpus and every artifact produced in CI.

**Theorems to establish.**

- `normalize_idempotent`: a valid key normalizes to itself.
- `normalize_canonical`: equal logical identities have one canonical spelling; case and
  separator variants collapse to the same key.
- `normalize_injective`: distinct logical identities have distinct canonical keys.
- `layout_hash_deterministic`: the same target, type, and layout bytes produce the same hash.
- `preserve_iff_shape`: `classify_root` returns `Preserve` exactly when root kind, type key,
  layout hash, size, and alignment match the baseline; any mismatch returns `Reject`.
- `field_list_hash_permutation_invariant`: field order does not change the hash.
- `got_id_deterministic`; `tracked_rename_preserves_got_id`; `got_id_collision_detected`.
- `sha256_spec`: the SHA-256 implementation matches the FIPS 180-4 compression and padding
  functions on every input length used by the artifact formats.
- `fnv1a_deterministic` and `signature_hash_sound`: the stable name/signature hash depends only
  on the specified byte sequence and matches the Lean reference.
- `artifact_deterministic`: identical inputs produce identical artifact bytes, with canonical
  ordering and no ambient inputs.
- `slots_canonical`: assignment is grouped, contiguous, and injective; identity groups are
  sorted by `GotId` and import slots by service ID; input order does not matter.
- `slot_table_valid_iff_canonical`: a slot table is accepted exactly when it is canonical.
- `decode_encode (m : Manifest) : decode (encode m) = .ok m`.
- `encode_decode_canonical`: for canonical bytes, decoding then encoding returns the same bytes.
- `decode_total`: decoding never panics and returns a bounded error on malformed input.
- `validate_sound`: an accepted manifest or descriptor satisfies every format-1 invariant,
  including digest pairing and previous-key rules.
- `classify_total` and `classify_deterministic`.
- `preserve_sound`: a `Preserve` action implies compatible identity, type key, layout hash,
  size, and alignment.
- `reject_sound`: a `Reject` action implies the baseline and candidate are incompatible.
- `plan_valid_sound`: every action in an accepted plan is justified by the baseline and the
  candidate catalog.
- `build_id_deterministic` and `pairing_check_sound`.

**Bridge.** Aeneas/Charon for the pure key, layout, and classification functions; a Lean
reference codec and validator with an equivalence proof for `wire.rs`, `descriptor.rs`,
`manifest.rs`, and the canonical JSON projection. The codec is large, so prove validation and
round-trip for format 1 first, then widen to the canonical JSON and binary projections.

**Exit criteria.** Format-1 encode/decode round-trip and validation are proved; key, layout,
`GotId`, slot, and plan invariants are proved; the Rust pure kernels are V1; the proved
validator processes the committed fuzz corpus and all CI artifacts; format versioning is
explicit and a format change starts a new proof module.

**Risks.** SHA-256 collision resistance is a cryptographic assumption; the implementation's
correctness is a separate proved obligation. Canonical JSON ordering and number formatting are
easy to get subtly wrong; decoder completeness is a large surface; format evolution can
invalidate proofs. State the cryptographic assumption in the TCB and version the proofs with
the format.

**Effort.** L.

### Phase 4 — Front-end exactness, round-trip, and parser soundness

**Goal.** Prove the front-end contracts that are cheap to state and catch the most bugs, then
close full parser soundness and completeness. Stage 4a covers tokenization, literals, CST
losslessness, the formatter, and Annex A precedence. Stage 4b proves the parser against the
Phase 0 `Derives` relation.

**Scope.**

Stage 4a proves the cheap contracts:

- Model tokens, trivia, spans, and the lossless CST contract for the constructs used by the
  formatter and expression grammar.
- Prove span coverage and ordering, literal decoding, keyword case-insensitivity, and the UTF-8
  string rules.
- Prove the formatter contract: idempotence, parse-preservation, comment preservation, and
  encoding preservation.
- Prove the expression precedence and associativity rules, including the Annex A split where
  relational operators bind tighter than equality.
- Use the corpus and UI tests as the differential oracle; every proof target has a generated
  vector set or a corpus replay.

Stage 4b extends the Phase 0 `Derives` relation to the full grammar and proves parser soundness,
completeness, termination, and the error-recovery characterization.

**Theorems to establish.**

- `tokenize_total`: tokenization never panics and returns a token stream or diagnostics.
- `spans_cover`: the token and trivia spans exactly cover the source, with no gaps or overlaps.
- `spans_ordered`: spans are monotone in source order.
- `literal_value_correct`: numeric, string, UTF-8, duration, date, and time literals decode to
  the specified values.
- `literal_roundtrip`: where a canonical spelling is defined, printing then parsing returns the
  same value.
- `parse_lossless`: the CST text equals the source text.
- `parse_deterministic`: one input produces at most one CST.
- `parse_sound`: `parse s = .ok ast → Derives grammar ast`.
- `parse_complete`: `Derives grammar ast → ∃ s, parse s = .ok ast`, or the practical form
  "every source in the grammar's language parses".
- `parse_terminates`: parsing terminates within the documented depth and fuel bounds, and the
  256-level nesting limit is enforced.
- `parse_error_sound` where feasible: a reported syntax error is genuine; otherwise the recovery
  cases are characterized rather than proved complete.
- `format_idempotent`: formatting a formatted tree produces no change.
- `format_preserves_parse`: `parse (format t)` is the same AST as `parse t` modulo trivia,
  keyword case, semicolon strictness, and line endings.
- `format_preserves_comments`: every comment appears exactly once and in order.
- `precedence_matches_annex_a`, `associativity_correct`, and `short_circuit_correct`.

**Bridge.** The Rust CST is rowan-style and large; model the contract rather than the whole
data structure. Use Aeneas/Charon for the pure lexer and formatter helpers where practical, and
otherwise prove the Lean reference and replay the corpus through an oracle. Stage 4a proves the
round-trip, termination, and lexical contracts that catch the most bugs per unit of effort;
Stage 4b proves full parser soundness and completeness against the Phase 0 grammar.

**Exit criteria.** Stage 4a: lexer span and literal theorems; formatter idempotence and
parse-preservation on the corpus; precedence theorem; the oracle replays the lexer, parser, UI,
and formatter corpora. Stage 4b: `parse_sound` and `parse_complete` for the full grammar, or a
documented gap with a stated reason. The registry records which functions are V1 and which
remain V0.

**Risks.** The CST representation may not match the Lean model closely enough for a V1 claim;
trivia, encoding, and relaxed semicolon mode add cases; error recovery is intentionally
permissive and may be hard to model exactly; full grammar soundness must handle the parser's
depth limit, left-recursion-free structure, and termination argument.

**Effort.** M/L.

### Phase 5 — Emitted-IR validation (translation validation)

**Goal.** Build a proved checker for the LLVM IR fragment the compiler emits, and run it on every
emitted module. This is the practical backend assurance step before the full Phase 7 codegen
semantics theorem.

**Scope.**

- Extend the Phase 0 `IRFrag` syntax from the core subset to the full emitted textual IR:
  modules, type definitions, globals, constants, functions, basic blocks, integer and floating
  operations, comparisons, casts, memory operations, `getelementptr`, `phi`, branches,
  `switch`, `return`, direct and indirect calls, checked arithmetic intrinsics, trap calls, and
  the GOT slot access pattern.
- Extend the Phase 0 `WellFormed` predicate with the full contract: type correctness, SSA form
  and dominance, block terminators, `phi` arity, GOT cell kind consistency, canonical slot
  table, entry-point signatures and linkage, descriptor and function-table pairing, no direct
  persistent-root or user-callable references, pre-attach restrictions, and the local-global
  allowlist.
- Implement `checkIR : IRFrag → Except Error Unit` in Lean and prove
  `checkIR_sound : checkIR ir = .ok () → WellFormed ir`.
- Prove the textual printer/parser round-trip `parseIR (printIR ir) = .ok ir` so the checker
  applies to the artifact that clang receives, not only to the in-memory builder.
- Connect the Rust verifier in `crates/stc-codegen/src/verify.rs` to the Lean checker: either
  prove the Rust contract refines `WellFormed` (V1) or replace the Rust path with the extracted
  checker (V3). The Rust verifier is already a structural contract, so this is a tractable
  bridge.
- Run the checker in CI over IR snapshots, the conformance corpus, examples, and execution-test
  artifacts; add negative fixtures for every rule.

**Theorems to establish.**

- `checkIR_sound : checkIR ir = .ok () → WellFormed ir`.
- `print_parse_roundtrip : parseIR (printIR ir) = .ok ir`.
- `verify_contract_sound`: if the Rust verifier accepts, the module satisfies `WellFormed`.
- `slot_access_well_formed`: every cell use has the kind recorded by the slot table.
- `no_direct_root_or_callable`: no body names a persistent root or user callable directly.
- `entry_points_match_contract`: the required ABI 2 entry points have the specified signatures
  and linkage.
- `descriptor_and_table_pair`: the descriptor blob and function table match the catalog and
  manifest identities.

**Exit criteria.** Every emitted module in CI passes the proved checker; the checker agrees
with clang acceptance on positive and negative fixtures; every rule has a negative test; the
printer/parser round-trip is proved or explicitly listed in the TCB; the production integration
decision (CI-only versus in-compiler) is recorded.

**Risks.** The textual IR is an external format; a printer/parser mismatch can create a false
sense of validation. Keep the emitted fragment small and disciplined, and treat clang acceptance
as necessary but not sufficient. A proved checker does not prove semantic preservation; the
claim is V2, not V4.

**Effort.** L.

### Phase 6 — Type system, name resolution, and HIR lowering

**Goal.** Extend the Phase 0 core static and dynamic semantics to the full language and prove
the Rust checker sound: accepted programs are well-typed, names resolve to the intended
bindings, conversions are legal, and lowering to HIR preserves meaning. Phase 0 defines the
AST, HIR, core typing judgment, and core evaluation relation; this phase extends them rather
than redefining them.

**Scope, staged.**

- Stage 6a: extend the Phase 0 core typing judgment to the complete scalar, expression,
  statement, function, and program language, including calls and constant expressions.
- Stage 6b: arrays, structs, subranges, enums, strings, conformant parameters, and bounds.
- Stage 6c: namespaces and `USING`, classes, interfaces, inheritance, methods, access control,
  `THIS`/`SUPER`, references, `POINTER TO`, and escape rules.

**Theorems to establish.**

- `check_sound : check p = .ok env → HasType p env`.
- `check_complete` where the checker is intended to be complete; otherwise
  `rejection_characterized`, an explicit statement of what the checker may over-reject.
- `call_arity_sound`: every analyzed call has the declared arity and parameter form.
- `definite_assignment_sound`: every read is assigned on every path that reaches it, according
  to the language rule.
- `resolve_sound` and `resolve_unique`: every free identifier resolves to a unique, correct
  binding; `no_use_before_decl`.
- `conversion_legal_iff`: the conversion relation accepts exactly the conversions the language
  defines, including the sign and width cases already modeled in Phase 2.
- `lower_preserves_semantics`: lowering inserts only valid explicit casts and resolves places
  and callees without changing meaning.
- `hir_wellformed`: every lowered HIR node satisfies the HIR invariants.
- `no_stack_escape`, `retention_sound`, `access_control_sound`, `override_sound`, and
  `interface_sound` as the relevant stages land.

**Bridge.** The Lean checker is the reference. Prove the pure decision functions (conversion
tables, name normalization, overload ordering, access checks) equal to Rust with Aeneas/Charon
where the spike supports it; otherwise differential-test the Rust checker against the Lean
reference and treat the Rust checker as V0 until refined. For a V4 end-to-end claim, the core
lowering path needs V1 evidence or an extracted implementation.

**Exit criteria.** The extended checker soundness is proved for the covered stages; resolution
and lowering theorems are proved; the Phase 0 core semantics is reused rather than re-proved;
incompleteness is documented rather than hidden; the Rust core paths are V1 or explicitly V0
with a stated gap; the feature matrix records which stages are covered.

**Risks.** The language is large; OOP and references are semantically rich; the checker is
deliberately incomplete in places; specification ambiguity can produce a proof of the wrong
property. Each stage needs a written declarative judgment before implementation proofs start.

**Effort.** XL, staged.

### Phase 7 — Codegen semantic preservation

**Goal.** Prove that the emitted IR refines the Phase 0 source semantics for a defined language
subset, and extend the subset feature by feature. Phase 0 defines the core source and target
semantics; this phase proves the compiler's translation implements them.

**Scope.**

- Extend the Phase 0 source semantics from the core subset to the full HIR subset: values,
  places, memory, traps, calls, GOT roots, function cells, and the task/cycle boundary.
- Extend the Phase 0 target semantics to the full emitted `IRFrag`: integer and floating
  operations, memory, GOT cell loads and stores, indirect calls, descriptor/vtable dispatch,
  traps, and the host ABI boundary. Consult Vellvm for LLVM behavior, but formalize the fragment
  natively in Lean. Do not attempt a Vellvm port: there is no complete Lean LLVM semantics and a
  wholesale port is a research project. Lean-MLIR, VeIR, and Trivet are references for patterns,
  not dependencies; the fragment is defined natively and validated against the emitted artifact.
- Define the simulation relation between source and target states, including a relation for
  errors and traps.
- Prove the top-level theorem for each stage.

**Top-level shape.**

```lean
theorem codegen_preserves
    {p : HirProgram} {ir : Module} {s : SourceState} :
    codegen p = .ok ir →
    SourceSem p s →
    ∃ s', TargetSem ir s' ∧ Rel s s'
```

Additional obligations:

- `trap_preserved`: a source trap corresponds to a target trap with the same code and span, and
  no source trap becomes a silent target value.
- `checked_arith_refines_exact`: the IR's checked add/sub/mul/div plus trap behavior equals the
  Phase 2 exact semantics.
- `got_access_refines`: a slot load or store observes the root value model used by Phase 3 and
  Phase 8.
- `call_refines`: a function-cell call resolves to the intended callee for the active
  generation.
- `bounds_and_range_checks_refine`: subrange and array checks trap exactly when the source
  semantics requires.

Stage the proof:

- 7a: scalar expressions, assignments, control flow, functions, programs, GOT globals, traps.
- 7b: arrays, structs, strings, subranges, enums, bounds and range checks. Prove the
  type-layout computation matches LLVM's layout for the covered types, including struct field
  order, alignment, array strides, string headers, and GEP offsets; a wrong offset is as
  damaging as a wrong instruction.
- 7c: classes, interfaces, inheritance, method dispatch, references, pointers, `VAR_IN_OUT`,
  conformant arrays, and escape rules.
- 7d: standard functions and function blocks, timers, the process image, tasks, and the
  scheduler boundary.

The textual printer and clang/LLVM remain outside this theorem unless the printer round-trip is
proved and a verified backend is adopted. The claim is "verified to the emitted IR boundary",
not "verified to machine code".

**Exit criteria.** The end-to-end theorem for the defined subset is proved with no `sorry`; the
feature matrix states exactly which constructs are covered; trap and UB behavior is documented;
the TCB lists the printer, clang/LLVM, the runtime, and the hardware; the Phase 5 checker runs
on every artifact used in the proof.

**Risks.** This is the largest phase. LLVM's `undef`, poison, and `nsw` behavior is subtle; the
project should emit a disciplined fragment, avoid undefined behavior on covered inputs, and
document any assumptions. GOT indirection, target-specific layout, OOP dispatch, and the runtime
ABI all add proof obligations. Mitigate by fixing the fragment, staging by feature, and stopping
at V2 if V4 coverage becomes uneconomic.

**Effort.** XL, multi-stage.

### Phase 8 — Runtime, swap, and scheduler invariants

**Goal.** Prove the safety and preservation properties of the persistent host and online-change
state machine that the rest of the design relies on.

**Scope.**

- Model the host lifecycle: cold install, load, prepare, activate, commit, rollback, unload;
  pending, active, and retired generations; generation pins; quarantine; the root registry; the
  descriptor registry; the process image; and the cooperative task scheduler.
- Prove atomicity, rollback, preservation, initialization, descriptor stability, and
  no-use-after-unload.
- Model the cycle boundary: inputs latched once, task guards set and cleared, the clock latched
  once per scan, tasks run in canonical order, outputs published after the scan, and traps
  handled by the documented policy.
- Model the `STCRET01` retained-state file: the layout hash, CRC, atomic save, warm/cold
  restore, and mismatch policy. Prove that a valid file restores exactly the retained values,
  and that corruption or a layout mismatch follows the documented policy rather than restoring
  partial state.
- Model the process-image span computation and direct-address decoding (`image_span` and the
  byte/bit addressing rules), and prove that a binding addresses the intended bits and bytes
  with the declared widths.
- Model the standard function-block state machines, especially the cycle-latched `TON`/`TOF`/
  `TP` timers, and prove their transitions match the standard's timing rules and the
  once-per-scan clock rule.
- Model the host ABI 2.2 service table and prove its indices, signatures, and trap boundary
  match the ABI contract.
- Replay the runtime and online-change test suites as model-based tests against the Lean state
  machine.

**Theorems to establish.**

- `activate_atomic`: activation changes the active generation in one cycle-boundary step; no
  cycle observes a mixture of old and new generations.
- `rollback_restores`: a rollback restores the pre-activation active generation and process
  image.
- `preserve_values`: for every preserved root, the value after activation equals the value
  before it.
- `allocate_init_once`: every new root is initialized exactly once and never reinitialized while
  preserved.
- `drop_quarantine`: removed roots are quarantined and not reused as fresh storage.
- `descriptor_stability`: compatible TYPEINFO, VTABLE, record, and interface descriptors keep
  their addresses across a warm swap; incompatible shape changes are rejected while live.
- `no_use_after_unload`: no retained pin or interface can reference an unloaded generation.
- `process_image_remap_sound`: direct-variable bindings map to the correct offsets, and unbound
  inputs receive defaults.
- `image_span_sound`: byte and bit addresses decode to the intended process-image range with
  the declared width and alignment.
- `state_file_roundtrip` and `state_restore_preserves`: a valid state file restores exactly the
  retained leaves; `state_corruption_rejected` covers CRC and layout-hash mismatches.
- `timer_semantics_matches_standard`: `TON`, `TOF`, and `TP` transitions match the standard's
  timing rules under the once-per-scan clock.
- `host_abi_table_sound`: service indices, signatures, and the non-returning trap boundary match
  the ABI 2.2 contract.
- `scheduler_deterministic`, `clock_latched_once`, and `trap_boundary_sound`.

**Bridge.** The state machine is the reference. Use Aeneas/Charon for the core transition
functions where practical; otherwise prove the model invariants and treat the Rust runtime as
V0 with model-based tests, explicitly stating that the implementation is not yet refined. The
runtime is concurrent (cycle thread plus control/status thread), so model the documented
synchronization boundary and test the Rust implementation against it; do not claim a proof of
unbounded concurrent behavior without a concurrency model.

**Exit criteria.** The invariants are proved for the model; the runtime test suite maps to model
transitions; the core Rust paths are V1 or explicitly V0; the claim states "swap protocol
model-verified, implementation tested/refined for the core paths"; format-1 plan proofs from
Phase 3 are used, not duplicated.

**Risks.** Model fidelity, concurrency, platform I/O, and unbounded state. Keep the model
abstract at the hardware and OS boundary; use the Phase 3 plan semantics as the shared
vocabulary; treat the control thread as a documented event source with explicit invariants.

**Effort.** L.

### Phase 9 — Composition, coverage widening, and claims

**Goal.** Compose the phase theorems into end-to-end statements, widen coverage, publish an
honest status matrix, and keep the proofs alive as the compiler evolves.

**Work.**

- Compose parser, type system, HIR lowering, codegen, and runtime models into the top-level
  theorem for the defined subset. The theorem is only as strong as its weakest linked phase;
  unlinked phases remain in the TCB and are named.
- Publish a verification status matrix covering every feature: target level, theorem, spec
  reference, status, owner, and TCB note. Keep it generated from the registry rather than
  hand-maintained.
- Add proof-impact checks: a change to a modeled Rust function or specification section must
  update the registry or fail CI.
- Widen coverage from the feature matrix: close gaps in the order of remaining severity, not in
  pipeline order.
- Decide whether to pursue translation validation for LLVM (for example an Alive2-like or
  Trivet-like checker), a verified backend, or a Lean-first implementation. None is in scope for
  the first program; record the decision and its rationale.
- Run an external review of the top-level claims and TCB before any public "verified" statement.

**Exit criteria.** The status matrix has no unlabeled claims; the top-level theorem for the
defined subset is published with its TCB; the critical targets meet their stated levels; no
`sorry` remains in the proved modules; the proof CI and maintenance budget are agreed.

**Effort.** M plus ongoing maintenance.

## 7. Bridge decision matrix

The bridge is the hard part. A proof that does not constrain the running compiler is not
verification. Choose the cheapest bridge that reaches the required level.

| Target class | Preferred bridge | Fallback | Do not claim |
| --- | --- | --- | --- |
| Pure scalar kernel (`exact.rs`, conversion tables) | Aeneas/Charon refinement to Lean | Lean extraction to C/Rust used in the build | V1 from vectors alone |
| Constant evaluator | Lean reference plus Aeneas proof or extraction | Differential oracle plus V0 label | V1 from a hand-port |
| Key/layout/classification kernels | Aeneas/Charon refinement | Lean extraction or proved checker at the artifact boundary | V1 from matching test vectors |
| Artifact codecs and validators | Proved Lean checker run on the artifact (V2) | Aeneas proof of the Rust decoder | V2 for a checker that inspects fixtures only |
| Lexer/formatter/parser contracts | Lean model plus corpus oracle; Aeneas for pure helpers | V0 differential testing | V1 for the whole parser |
| Type checker and lowering | Lean reference plus Aeneas for decision cores | V0 differential testing with an explicit gap | V4 without a linked lowering proof |
| Codegen | Proved IR checker (V2), then simulation theorem (V4) | Translation validation only | V4 from the checker alone |
| Runtime state machine | Lean state machine plus model-based tests; Aeneas for core transitions | V0 with invariants and conformance tests | Unbounded concurrency proof without a model |

Extraction is not free. Lean's executable backend emits C, so using extracted code in a Rust
compiler adds an FFI boundary, a C toolchain to the build, and a marshalling layer that must
itself be tested or proved. Treat extraction as a fallback for a small, stable kernel, not as the
default bridge for large components.

Loop-heavy Rust needs extra care. A proof about a structurally recursive Lean function does not
transfer to a Rust `for` or `while` loop by itself; the loop must be modeled, for example with
Aeneas's loop fixed-point combinator, and its termination measure supplied. Mutable state, early
exits, panics, and integer overflow must be represented explicitly in the model. Prefer pure,
panic-free kernels for Aeneas targets; for codegen and runtime loops, budget a separate loop
model or a model-based test harness rather than assuming the recursive proof carries over.

## 8. Feature coverage matrix

The matrix is the program's scoreboard. It is illustrative at this stage; Phase 0 and Phase 1
turn it into the registry.

| Feature | Phase | Target level | Key theorem | Current status |
| --- | --- | --- | --- | --- |
| AST, HIR, and IR fragment types | 0 | S | definitions plus `WellFormed` | Not started |
| Core source semantics | 0 | S | `eval_deterministic`, `progress`, `preservation`, `type_soundness` | Not started |
| Core typing judgment and checker | 0 | S | `core_check_sound`, `core_rejection_characterized` | Not started |
| Core target semantics | 0 | S | `ir_deterministic`, `ir_memory_safe`, `ir_trap_sound` | Not started |
| Reference interpreter and conformance | 0 | V0 | `lake exe semantics-conformance` report | Not started |
| Checked integer arithmetic | 2 | V1 | `add_ok_iff`, overflow/div-zero lemmas | Model + vectors (V0) |
| Bitwise, shifts, rotates | 2 | V1 | range, masking, inverse lemmas | Model + vectors (V0) |
| Conversions, comparisons, selection | 2 | V1 | conversion and selection lemmas | Model + vectors (V0) |
| String length and UTF-8 | 2 | V1 | `len`/`len_code_unit` theorems | Model + vectors (V0) |
| Constant evaluation | 2 | V1 | `constEval_sound` | Not started |
| Keys, type keys, layout hashes | 3 | V1 | normalization and hash invariants | Rust functions + tests (V0) |
| `GotId`, catalog, slots | 3 | V1 | identity, canonical slot tables | Rust functions + tests (V0) |
| Descriptor/manifest codecs | 3 | V2 | `decode_encode`, `validate_sound` | Rust codecs + fuzz (V0) |
| Online-change plan | 3 | V1 | `preserve_sound`, `plan_valid_sound` | Rust validation + tests (V0) |
| Artifact determinism and hashing | 3 | V1 | `artifact_deterministic`, `sha256_spec` | Rust + double-build tests (V0) |
| Lexer spans and literals | 4 | V1 | `spans_cover`, `literal_value_correct` | Rust lexer + tests (V0) |
| CST losslessness | 4 | V1 | `parse_lossless` | Rust parser + tests (V0) |
| Formatter round-trip | 4 | V1 | `format_idempotent`, `format_preserves_parse` | Rust formatter + tests (V0) |
| Expression precedence | 4 | V1 | `precedence_matches_annex_a` | Not started |
| Full parser soundness/completeness | 4b | V1/V4 | `parse_sound`, `parse_complete` | Not started |
| Extended type checking | 6a | V1/V4 | `check_sound` | Not started |
| Arrays, structs, strings, subranges | 6b | V1/V4 | typing and bounds soundness | Not started |
| OOP, interfaces, references | 6c | V1/V4 | resolution, dispatch, escape lemmas | Not started |
| HIR lowering | 6 | V1 | `lower_preserves_semantics` | Not started |
| Emitted-IR well-formedness | 5 | V2 | `checkIR_sound` | Rust verifier exists (V0) |
| Scalar codegen semantics | 7a | V4 | `codegen_preserves` | Not started |
| Aggregate/string codegen and type layout | 7b | V4 | staged simulation and layout lemmas | Not started |
| OOP/reference codegen | 7c | V4 | staged simulation | Not started |
| Standard library and FBs | 7d | V4 | staged simulation | Not started |
| Swap, preservation, rollback | 8 | V1/V2 | `activate_atomic`, `preserve_values` | Not started |
| Retained-state file | 8 | V1 | `state_file_roundtrip`, `state_restore_preserves` | Not started |
| Scheduler and timers | 8 | V1 | `scheduler_deterministic`, `timer_semantics_matches_standard` | Not started |
| Process image and I/O | 8 | V1 | `process_image_remap_sound`, `image_span_sound` | Not started |
| Host ABI 2.2 table | 8 | V1 | `host_abi_table_sound` | Not started |
| End-to-end subset | 9 | V4 | `pipeline_correct` | Not started |

Not planned: full LLVM/LLVM optimizer/backend semantics, floating-point transcendental
semantics (remains test-based under D-006), the TOML parser and project CLI (unless a security
property demands it), diagnostics rendering, and the operating system, filesystem, I/O devices,
and hardware.

## 9. Proof engineering and CI

- Toolchain: Lean 4.35.0 is pinned in `lean/lean-toolchain`; the program keeps that pin until a
  deliberate upgrade with a proof-build validation.
- No `sorry` or `admit` in proved modules. CI fails on either. Top-level theorems are audited
  with `#print axioms`; any extra axiom is listed in the TCB and justified.
- Executable models stay free of proof-only dependencies. If Mathlib is adopted (D-V2), it is
  confined to proof libraries; the executable kernel and vector generator remain core Lean plus
  Batteries so `lake exe vectors` stays fast and deterministic.
- The verification registry is the single source of truth for claims. A generated status matrix
  in this document and in the README replaces hand-written claims.
- Vector and oracle pipeline: keep `lake exe vectors`; add `lake exe oracle` for constant
  evaluation, keys/layout, codecs, and front-end contracts; Rust tests compare against the
  committed JSON. Regenerating a vector is a reviewed change, and a digest mismatch fails CI.
- Aeneas/Charon pipeline: pin the toolchain; check generated Lean models into a separate
  `lean/Stc/Generated/` tree; keep proofs in hand-written modules; record the exact Rust source
  hash the model was generated from and fail CI when it drifts.
- CI jobs: `lean` (build, no-sorry, axiom audit), `lean-vectors` (regeneration diff),
  `lean-oracle` (differential tests), `lean-checkers` (verified IR/GOT checkers on CI
  artifacts), and an optional nightly `aeneas` job. The existing `lean` job becomes the first
  three.
- Time budget: keep the fast proof CI under a documented limit; put long proofs and the Aeneas
  regeneration in a nightly job. A proof that cannot fit the budget is a design smell.
- Traceability: every registry record names the specification section or rule ID it proves.
  A theorem without a specification reference is either a missing specification update or a
  proof of something the product does not need.

## 10. Risks and mitigations

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Model/implementation gap persists | Proofs do not constrain the compiler; false confidence | Phase 1 bridge; V1/V2 required for strong claims; registry records the gap |
| Aeneas/Charon cannot handle the Rust | Schedule slip or V0-only kernels | Spike first; fall back to extraction or a proved checker; keep kernels pure and panic-free |
| Proof scope creep | Program never reaches a usable claim | Stage by feature; stop at V2 where V4 is uneconomic; publish partial claims |
| Specification drift | Proofs about the wrong behavior | Registry links every theorem to a spec section; proof-impact CI; review changes |
| Lean or Mathlib upgrade breaks proofs | Maintenance burden | Pinned toolchain; proof-only dependencies; nightly upgrade validation |
| LLVM `undef`/poison and target layout | Semantic mismatch or false V4 | Restricted emitted fragment; clang verification; translation validation; document assumptions |
| Runtime concurrency | Model does not match the implementation | Model documented transitions; model-based tests; keep the control-thread boundary explicit |
| Windows/Linux differences | Proofs do not cover the shipping target | Target-independent models; target-specific layout in Phase 3/7; CI on both hosts |
| False safety or certification claim | Product and legal risk | Explicit non-safety stance; claim ladder; external review before public statements |
| Maintenance cost | Proofs rot and are ignored | Registry, generated status matrix, proof-impact CI, one owner, quarterly review |

## 11. Non-goals

- A general formal semantics of LLVM, its optimizer, its backend, the linker, or clang.
- Proving floating-point transcendental functions or platform libm behavior; these remain
  test-based under D-006.
- Proving the Rust standard library, the operating system, the filesystem, I/O devices, or
  hardware.
- A Lean rewrite of the 131k-line compiler. This is a separate product decision (D-V10).
- Safety certification or a safety claim. The project is not a safety PLC and assumes no safety
  responsibility.
- Proving CLI ergonomics, diagnostic prose, or the TOML parser unless a security property
  requires it.
- A cost-model proof of the O(1) activation or latency claims. These remain measured and
  documented; a separate cost model can be added later if the claim becomes contractual.
- Proving completeness where the type checker is intentionally incomplete; the program proves
  soundness plus a characterized rejection class.

## 12. Open decisions

| ID | Decision | Recommendation |
| --- | --- | --- |
| D-V1 | Bridge strategy per target class | Aeneas/Charon for existing pure kernels; proved Lean checkers for artifacts; vectors for the rest; extraction only if Aeneas fails |
| D-V2 | Mathlib and Batteries policy | Core Lean plus Batteries for executable models; Mathlib allowed only in proof libraries after a build-time spike |
| D-V3 | Emitted-IR fragment and UB policy | Phase 0 defines the core fragment; Phase 5 extends it. Keep it restricted with no poison/undef on covered paths; no fast-math; clang verification plus the proved checker |
| D-V4 | TCB and claim language | Adopt the V0–V4 ladder; publish the TCB; never say "fully verified" |
| D-V5 | Verification registry and spec traceability | TOML registry plus an `xtask` check; generated status matrix |
| D-V6 | First end-to-end subset | Phase 0 defines the core semantic subset; Phase 1 fixes the first end-to-end theorem subset: scalars, expressions, assignments, control flow, functions, programs, GOT globals; no OOP/strings/aggregates |
| D-V7 | Production versus CI-only checkers | CI first for Phases 3 and 5; production integration after a size and latency evaluation |
| D-V8 | Ownership and maintenance budget | One verification owner; a standing proof-CI budget; quarterly coverage review |
| D-V9 | Format-version proof policy | Prove format 1; start a new proof module for any format change; no implicit migration |
| D-V10 | Lean-first rewrite | Not in this program; revisit after Phase 5 and Phase 7 results |

## 13. Definition of done

The program is done when:

- The verification status matrix records a level, theorem, specification reference, and TCB note
  for every compiler feature, with no unlabeled claims.
- The Phase 0 semantics is reviewed and versioned at level S; the AST, HIR, and IR fragment are
  defined; the core source and target semantics are proved sound; the reference interpreter and
  conformance report are committed.
- The critical targets meet their planned levels: exact scalar semantics and constant evaluation
  at V1/V3, GOT format and plan at V1/V2, lexer/formatter round-trip at V1, emitted-IR
  validation at V2, the extended type system and HIR at V1, and the runtime swap invariants at
  V1/V2.
- The defined end-to-end subset has a V4 semantic-preservation theorem from source semantics to
  the emitted IR boundary, with the TCB documented.
- CI enforces no `sorry`, the registry, vector regeneration, the oracle, and the proved
  checkers; the proof build stays within its budget.
- The public claim is accurate: verified for the defined subset and for the proved kernels and
  validators, with LLVM, the printer boundary, the runtime outside the proved paths, the
  operating system, and hardware in the trusted base.

## 14. Immediate next steps

1. Review this plan and resolve D-V1 through D-V10.
2. Start Phase 0: create `Stc.Syntax`, `Stc.Hir`, `Stc.Sem`, `Stc.Codegen.Ir`, and
   `Stc.Codegen.Semantics`; write the semantics document and fix the core subset boundary.
3. Build `lake exe stc-sem` and `lake exe semantics-conformance`; run the core subset of the
   conformance and execution corpus and commit the report.
4. Run the Phase 1 bridge spike in parallel: Aeneas/Charon on `exact.rs`, `keys.rs`, and one
   codec path; write the spike report with measured effort and coverage.
5. Create the `Stc.ConstEval`, `Stc.Got`, and `Stc.Verification` skeletons, the registry, and
   the no-sorry/axiom/registry CI checks.
6. Extend the exact vectors to every operation and type, then add the constant-evaluation
   oracle.
7. Prototype `lake exe check-ir` on one emitted module and compare it with
   `stc_codegen::verify`.
