# Phase 0 Report — Core Semantics Foundation and Soundness

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-11-02
Semantics version: `1.0` (frozen by P0.7; freeze commit
`7aad70d41e1dc4b47606f13cd7818b638fd45927`)
Status: P0.7 was approved on 2026-11-02; Phase 0 is complete. P0.0–P0.7 are owner-approved.
This report records what Phase 0 established, the conformance evidence, the known gaps, the
trusted computing base, the version freeze, and the handoff to later phases. It does not claim a
refinement proof; the review and sign-off are recorded in §9.
Companion: [Semantics](Semantics.md) · [Semantics Charter](Semantics-Charter.md) ·
[Core HIR Dump](HIR-Dump.md) · [Core Semantics Cases](Semantics-Cases.md) ·
[Phase 0 Implementation Plan](Plan-Phase0.md) · [Verification Program Plan](Plan.md)

## 1. Purpose and scope

Phase 0 is the standalone S-level semantics foundation of the verification program. It defines a
frozen core subset of stc and the emitted-IR fragment it compiles to, proves the core semantics
deterministic, type-sound, trap-sound, memory-safe, GOT-sound, and free of modeled undefined
behavior, provides executable reference models proved equivalent to the declarative judgments,
and compares the Rust compiler against the reference on a versioned corpus as V0 differential
evidence. The normative definitions are in [Semantics.md](Semantics.md); the authoritative
theorem status is [`lean/verification-status.toml`](../../lean/verification-status.toml).

Phase 0 does not claim that the Rust parser, analyzer, or code generator refines the semantics.
The conformance result is evidence that the reference semantics and the compiler agree on the
committed corpus; it is not a refinement proof, and no theorem in Phase 0 constrains the Rust
implementation.

## 2. The Phase 0 claim

At level S, Phase 0 claims:

> The stc core language subset and the emitted-IR fragment it compiles to are defined by a
> machine-checked semantics. The core source semantics is deterministic, type-sound, and
> trap-sound; the core checker accepts only well-typed programs; the emitted-IR semantics is
> deterministic and memory-, GOT-, and trap-sound on the covered fragment; and the executable
> reference models are equivalent to the declarative judgments. The Rust compiler is compared
> with the reference semantics on the core corpus as V0 evidence.

The claim is relative to the trusted computing base in §6 and the gaps recorded in §5. It is not a
safety-certification claim, a general LLVM claim, or a compiler-correctness claim. Those
non-claims are fixed in [Semantics-Charter.md](Semantics-Charter.md) §2.

## 3. What was proved and established (P0.0–P0.6)

The registry holds 45 records: 36 proved theorems, 6 proved definitions, 1 descoped theorem, and 2
tested evidence items. The counts and statuses are audited by
`tools/ci/check_verification_registry.py`.

| Sub-phase | Delivered | Status |
| --- | --- | --- |
| P0.0 | Scope and claim freeze, TCB, exclusion register, versioning policy, module skeleton, registry, and fail-closed CI checks | Approved 2026-10-27 |
| P0.1 | Token/trivia/span model, lossless CST contract, surface AST, `Derives`, AST/CST and source-span mapping, `ast_span_wellformed`, and the [Core Syntax Coverage](Core-Coverage.md) table | Approved |
| P0.2 | Typed HIR, `HasType`, `checkCore`, `HirWellFormed`, `ConversionLegal`; proved `core_check_sound`, `core_rejection_characterized`, `hir_wellformed`, and `conversion_legal_iff`; descoped `core_check_complete` with the `FOR` over-rejection recorded | Approved |
| P0.3 | Values and traps, source small-step machine, `ConfigTyped`, fuel interpreter; proved `eval_deterministic`, `progress`, `preservation`, `type_soundness`, `trap_soundness`, guarded `checked_arith_traps_exactly`, `interpreter_sound`, `interpreter_complete`, and initial-state typing | Approved 2026-10-31 |
| P0.4 | Production-module fragment `IRFrag`/`IRContract`, `WellFormed`, and the structural definition/lemma inventory, including the GOT cell kinds, slot table, overflow result, fixed arrays, and entry-point contract | Approved 2026-10-31 |
| P0.5 | `IRState`/`IRStep`, checked arithmetic, memory, GOT, calls, traps, `irExecRec`/`irRun`; proved `ir_deterministic`, `ir_progress`, `ir_memory_safe`, `ir_got_sound`, `ir_trap_sound`, `ir_checked_arith_refines_exact`, `ir_recursive_model_equiv`, `ir_no_ub`, `initState?_wf`, `irStep_wf`/`irStepStar_wf`, the reachability corollaries, and `irRun_halt_wf` | Approved 2026-11-02 |
| P0.6 | Frozen core-HIR dump `0.1.0`, strict Lean reader, `stc-core-case` and `stc-sem-result` `0.1.0`, `lake exe stc-sem`, opt-in `--snapshot`, differential harness, `stc-sem-conformance-report` `0.1.0`, 74-case corpus, committed byte-stable report, and the fail-closed corpus gate | Approved 2026-11-02; V0 evidence |

The proof inventory and the exact theorem shapes are in [Semantics.md](Semantics.md) §7. The
P0.3 proof found and fixed three machine bugs before the results were accepted: the call-argument
continuation advanced past the wrong argument, `EXIT` kept the loop body's loop context, and
`CONTINUE` on a `FOR` entered the increment in the body's loop context. The P0.4 target scope was
revised to remove `switch`; `CASE` lowers to branches, and no other inclusion or exclusion
changed. These are recorded in the sub-phase completion records in
[Plan-Phase0.md](Plan-Phase0.md) §19–§23.

## 4. Conformance evidence

### 4.1 Result

The frozen corpus contains 74 cases under `tests/verification/semantics/`. On the P0.7 freeze
toolchain, all 74 cases pass across the descriptor, the reference interpreter, and the compiled
artifact: the committed report's summary is `{"total":74,"passed":74,"failed":0}`. The report is
`tests/verification/semantics/conformance-report.json` and is the artifact of record. It is
V0 differential evidence, not a theorem and not a refinement proof.

### 4.2 Versions and schema

The report records the exact inputs of the run:

| Artifact | Format | Version |
| --- | --- | --- |
| HIR dump | `stc-core-hir` | `0.1.0` |
| Case descriptor | `stc-core-case` | `0.1.0` |
| Reference result | `stc-sem-result` | `0.1.0` |
| Conformance report | `stc-sem-conformance-report` | `0.1.0` |
| Runtime snapshot | `stc-runtime-snapshot` | `0.1.0` |
| Semantics | `semantics_version` | `1.0` |

The format versions are unchanged by the P0.7 freeze. The schemas, exit codes, comparison rule,
and gating contract are normative in [Semantics-Cases.md](Semantics-Cases.md) §8 and
[HIR-Dump.md](HIR-Dump.md).

### 4.3 Corpus and coverage

The coverage inventory in [Semantics-Cases.md](Semantics-Cases.md) §7.1 maps every claimed core
type, literal and expression form, operator, exact standard function, statement form, conversion,
trap, GOT access, and program/execution shape to at least one case, or records a justified
exclusion. The corpus exercises one program and one sequential cycle plus multi-cycle persistence,
recursion under the fuel bound, global initializers and GOT access, and all three core traps.

### 4.4 Gating and reproducibility

CI builds the Rust compiler and the uninstrumented test runtime, runs the harness with `--report`
into a temporary path, and `tools/ci/check_semantics_placeholder.py` compares the bytes with the
committed report. A stale report or a disagreement fails the gate; an empty corpus exits `3` and
is never reported as agreement. A passing report carries no timestamp, path, or build identifier,
so repeated passing runs are byte-stable for the same corpus and toolchain.

### 4.5 Platform-independence caveat

The Lean target semantics is target-independent: it models GOT cells, slot kinds, stack frames,
and exact integer operations abstractly, without host endianness or layout choices. The
conformance evidence is not platform-independent. The compiled artifact is produced by the local
Rust compiler, clang/LLVM, runtime library, operating system, and hardware, and those components
are the oracle. Byte-stability of the committed report is claimed only for the fixed corpus and
toolchain used to produce it; a different toolchain or host may produce different artifact bytes
and must be treated as a separate evidence run. Source paths in dumps are provenance, not
identity, so dump bytes differ across checkouts even when the program is the same. Agreement on
this corpus supports confidence but does not establish agreement on another platform.

## 5. Known gaps, exclusions, and triage

### 5.1 Exclusions E1–E7

The corpus exclusions are recorded with their reasons in
[Semantics-Cases.md](Semantics-Cases.md) §7.2. None is a silent omission.

| ID | Exclusion | Reason and disposition |
| --- | --- | --- |
| E1 | Result-less (procedure-style) function calls | Source-legal and supported by codegen, but the frozen dump requires a `type` on every call expression; a result-less call has none and is rejected with `type.error`. Needs a dump-format change (also T1). |
| E2 | Direct self-recursion | The function name inside its body denotes the result variable, so direct self-recursion is not expressible. Mutual recursion is covered by `mutual-recursion`. |
| E3 | Shift and rotate in the source corpus | The core HIR has no shift/rotate operator or standard function; shift and rotate remain in the exact target model and target IR. |
| E4 | Direct program-field root observation | The snapshot compares manifest global roots; program-field persistence is observed indirectly by mirroring the field into a global each cycle. |
| E5 | Cross-global initializer ordering (Q5) | Global initializers must be constant, so no initializer reads another global and the order is not observable through the source surface. Q5 remains a model decision. |
| E6 | Q3 character-validity traps | The P0.3 character model is layout-only; Unicode scalar validity and the `WCHAR_TO_UCHAR`/`UCHAR_TO_WCHAR` traps are deferred to the string phase. Corpus cases use valid character values. |
| E7 | Constructs outside the frozen core | Floats, time/date, strings, aggregates, OOP, references, function blocks, tasks, I/O, retained state, other standard functions, and traps outside `OVERFLOW`, `DIV_ZERO`, and `RANGE` are excluded by the charter. |

### 5.2 Model gaps and descoped obligations

- **Q3, layout-only characters.** `CHAR`, `WCHAR`, and `UCHAR` are modeled as unsigned integer
  layouts. Unicode scalar validity and the specified conversion traps are not modeled. Source
  type soundness is relative to this model, and `ir_progress` carries the `AccessBounded`
  source-check premise on the character-index path until Q3 is resolved. This is a recorded
  semantics gap, not a scope reduction.
- **Q2, guarded exact arithmetic.** `checked_arith_traps_exactly` is proved in a guarded form
  whose guard is redundant for representable inputs. The unguarded equality needs the
  `convert_ok_fits` and standard-function fit lemmas, recorded as a P0.5/Phase 2 follow-up.
- **Q5, cross-global initializer ordering.** Constant initializers make the ordering
  unobservable through the source surface; the model decision is recorded and is not a
  conformance claim.
- **`core_check_complete` descoped.** The P0.2 checker over-rejects `FOR` with the named rule
  `StmtRule.forUnsupported`. The declarative judgment still defines `FOR`, and the interpreter
  consumes the judgment. Full completeness is deferred to Phase 2, where lowering connects the
  source checker to the judgment.
- **Hypothesis-relative target results.** The target soundness results are relative to
  `WellFormed`, `WFRunning`, and the trusted trap-code/guarded-unreachable facts. They are not a
  general LLVM result.

### 5.3 Triage log and the T2 fix

The full triage log is [Semantics-Cases.md](Semantics-Cases.md) §7.3. No reference-versus-artifact
disagreement remains in the committed corpus.

| ID | Classification | Resolution |
| --- | --- | --- |
| T1 | Dump-format limitation | Result-less calls are excluded (E1); `stmt-call-discard` covers a result-bearing call in statement position. |
| T2 | Rust source-analyzer bug | `MIN(X, Y)` over two `WORD` values failed with `cannot convert WORD to USINT`, and `MIN`/`MAX` over `CHAR` failed with `argument of MIN must be ANY_MAGNITUDE, found CHAR`, although the Lean model's `isMagnitude` includes `CHAR` and the bit strings. Fixed: `AnyMagnitude` now includes `CHAR`, and `std_unify` chooses a bit-string or character result for `MIN`/`MAX` instead of integer promotion. Regression test `magnitude_standard_functions_accept_bit_strings_and_characters` and corpus case `std-min-max-bits-char` pass; `LIMIT` over the same categories also passes. |
| T3 | Source-language rule | Direct self-recursion is not expressible (E2); mutual recursion is covered. |
| T4 | Case-authoring error | Two early hand-written expectations were wrong; both were corrected from the Lean semantics, not from one implementation. The reference and artifact had agreed with each other; only the descriptors were wrong. No semantics or compiler behavior changed. |
| T5 | Source-language rule | A global initializer that reads another global fails as non-constant; recorded as E5. |

## 6. Trusted computing base and assumptions

The TCB is fixed by [Semantics-Charter.md](Semantics-Charter.md) §3 and restated in
[Semantics.md](Semantics.md) §8. In full:

- the Lean 4.35.0 kernel, its standard library, and the pinned toolchain in
  `lean/lean-toolchain`;
- the repository definitions: every theorem is true of the Lean definitions, so a wrong
  definition is a wrong semantics even when the proof checks;
- Batteries for executable models; Mathlib is not used in Phase 0 and is permitted only in
  proof-only libraries after a build-time spike (P0-D5); it never enters the interpreter, dump
  reader, or harness;
- the versioned HIR dump, case descriptors, and report schema as harness inputs: they are
  validated, but their content is trusted to describe the compiled program;
- the Rust compiler, clang/LLVM, operating system, and hardware as the conformance oracle. Bugs
  in that stack appear as disagreements or unmodeled features; the semantics does not rule them
  out.

Trusted-theorem policy: no `sorry` or `admit` in proved modules; top-level theorems are audited
with `#print axioms`; the only permitted axioms are `propext`, `Classical.choice`, and
`Quot.sound`. `native_decide` is not used in a trusted theorem.

Assumptions that must be stated with any published claim:

- the compiled program's observable behavior is represented faithfully by its HIR dump;
- the case descriptor describes the intended inputs and cycle count;
- no excluded construct reaches the reference tools; unknown versions and unmodeled features are
  rejected rather than interpreted;
- the conformance corpus covers every core feature and trap it claims to cover;
- the target soundness results are read relative to the frozen fragment and its hypotheses, not
  as general LLVM or compiler-correctness results.

## 7. Version freeze record

P0.7 freezes `semantics_version` at `1.0`. The freeze covers the definitions and theorem
statements in `Stc.Syntax`, `Stc.Grammar`, `Stc.Hir`, `Stc.Sem`, `Stc.Codegen.Ir`,
`Stc.Codegen.Semantics`, and `Verification.Semantics`, and the version metadata in the HIR dump,
case descriptors, reference results, and conformance report. The format versions remain at their
P0.6 values: `stc-core-hir` `0.1.0`, `stc-core-case` `0.1.0`, `stc-sem-result` `0.1.0`,
`stc-sem-conformance-report` `0.1.0`, and `stc-runtime-snapshot` `0.1.0`.

The freeze commit is `7aad70d41e1dc4b47606f13cd7818b638fd45927`, recorded in
[`lean/verification-status.toml`](../../lean/verification-status.toml) and here. The freeze commit
itself carried a marked placeholder because a commit cannot name itself; the immediately following
commit records the exact hash. The committed report was regenerated from the frozen corpus and
toolchain and matches the gate byte-for-byte; the case descriptors and the Rust/Lean version
constants and tests carry `1.0`.

No semantic definition or theorem statement was changed to perform the freeze, and no
incompatibility requiring an owner decision surfaced. The version bump changed only the version
metadata and its tests and fixtures.

## 8. Handoff to later phases

### 8.1 What each phase consumes

| Consumer | Consumes from the frozen `1.0` artifact |
| --- | --- |
| Phase 1 | The recursive models `stcSem`/`runFuel` and `irRun`/`irExecRec`; the loop-shaped relations `Step`/`Steps` and `IRStep`/`IRStepStar`; the equivalence theorems `interpreter_sound`, `interpreter_complete`, and `ir_recursive_model_equiv`; the registry schema and checker; and the no-`sorry`/axiom policy. The intended bridge points are `crates/stc-codegen/src/exact.rs` against `Stc.Exact`, `crates/stc-got/src/keys.rs` against the GOT key normalization, and one `stc-got` codec path against the cell/codec model. |
| Phase 2 | The core typing judgment and `ConversionLegal` in `Stc.Sem.Typing`; the exact-arithmetic connection `checked_arith_traps_exactly` and `ir_checked_arith_refines_exact`; and the descoped `core_check_complete` as the completeness obligation connecting the source checker to the judgment. |
| Phase 4 | The surface AST in `Stc.Syntax.Ast`, the `Derives` relation in `Stc.Grammar.Derives`, the AST/CST and source-span mappings, the lossless CST contract, and the [Core Syntax Coverage](Core-Coverage.md) table. Phase 4a proves the round-trip contracts; Phase 4b owns `parse_sound` and `parse_complete`. |
| Phase 5 | `IRFrag`, `IRContract`, `WellFormed`, and the structural lemmas in `Stc.Codegen.Ir` as the starting fragment for the full emitted-IR and textual-IR work, plus the grouped slot-table contract. Phase 5 extends the fragment; it does not reinterpret the core. |
| Phase 6 | The core typing judgment and dynamic semantics in `Stc.Sem` as the extension point for the full type system, name resolution, and HIR lowering. |
| Phase 7 | The source semantics and the target `IRFrag`/`IRStep` semantics for the codegen preservation theorem, with `ir_recursive_model_equiv` as the executable bridge. |
| Phase 8 | The GOT cell and slot model (`CellKind`, slot table, root map, function table), the trap model, and the initialization boundary represented by `initState` and the host ABI entry points. |

### 8.2 What may be extended and what requires a version bump

Later phases may extend the semantics in new modules or new sections, add constructors and rules
for new features, and add corpus cases and format-compatible feature tags. They may cite the
frozen `1.0` core for the constructs it defines. They may not silently widen the core, reinterpret
an existing definition, or change a theorem statement.

A semantics version bump is required when:

- an existing `1.0` definition, judgment, transition rule, or theorem statement changes, or its
  meaning changes even if the syntax is unchanged;
- an extension changes the truth of an existing theorem or requires a different hypothesis;
- the frozen core scope in [Semantics-Charter.md](Semantics-Charter.md) §6 changes;
- a dump, case, result, report, or snapshot field name, meaning, shape, ordering rule, status or
  trap vocabulary, or comparison rule changes. Such a change requires the affected format version
  to change as well; the semantics version is bumped if the semantic claim changes, and every
  report records both versions.

Purely additive work in new modules that leaves every `1.0` definition and theorem statement
intact may carry its own registry entries while the core remains cited as `1.0`; the extension is
not part of the frozen core claim until a reviewed version bump says so. The registry is the
single source of truth: every new definition or theorem is registered with a specification
reference and an honest status.

### 8.3 Refinement handoff

The refinement target is explicit and must be consumed as follows.

- **Recursive models.** `stcSem`/`runFuel` and `irRun`/`irExecRec` are total, fuel-indexed step
  functions over explicit configurations. `interpreter_sound` and `interpreter_complete` relate
  the source interpreter to `Steps`; `ir_recursive_model_equiv` relates a halting `irExecRec`
  result to a finite `IRStepStar` run ending in a state whose own step is `.halt`. These are the
  preferred executable targets for a Rust bridge because they expose the state and control flow
  as data.
- **Loop-shaped step relations.** `Step`/`Steps` and `IRStep`/`IRStepStar` are the declarative
  reference. A refinement proof must preserve them, not only the interpreter's final value. The
  closure is prefix-oriented and does not require termination; functions may be recursive.
- **Explicit state and control-flow encoding.** The source configuration carries persistent
  global roots, a frame stack, a focus (`expr`, `value`, `stmts`, `resume`, `returned`,
  `trapped`, `halted`), per-frame continuations, and loop context. The target `IRState` carries
  the module, an `IRFrame` stack (block, remaining instructions, terminator, locals, stack
  memory, allocation table, `phi` predecessor, return destination), GOT roots, bindings,
  globals, cycle counters, and machine status. Control flow is encoded in the focus, the
  continuation stack, block predecessors, and status transitions, not in host-language
  exceptions or an implicit call stack. A Rust bridge must relate the Rust loop and recursion to
  these explicit states; it may not assume the host call stack is the model's frame stack.
- **Termination and fuel obligations.** Phase 0 does not require source termination (P0-D9).
  Both executable models are fuel-indexed. `interpreter_complete` reproduces every terminating
  run given enough fuel, and `ir_recursive_model_equiv` is a halting-run equivalence. A Phase 1
  bridge theorem must therefore state its fuel obligation explicitly: either exhibit a fuel bound
  sufficient for the covered run, or prove that the Rust computation terminates within the
  modeled bound. A theorem that holds only under a fuel hypothesis must say so and must not be
  presented as a total-correctness result.
- **Phase 1 bridge points.** The intended first bridge is Aeneas/Charon on
  `crates/stc-codegen/src/exact.rs`, `crates/stc-got/src/keys.rs`, and one `stc-got` codec path.
  The Lean side of that bridge is `Stc.Exact`, the GOT key normalization contract, and the
  cell/codec model, with the Phase 0 recursive models and step relations framing the semantic
  obligation. Phase 1 also adopts the registry format, the no-`sorry` convention, and the axiom
  audit. The first end-to-end subset (D-V6) builds on the Phase 0 core subset; extending
  `IRFrag` to the full emitted fragment is Phase 1 work and is itself a versioned extension, not
  a reinterpretation of the core.

## 9. Review status and checklist

The owner reviewed and approved P0.7 on 2026-11-02. The checklist below is the accepted review
scope and record.

| Area | Check |
| --- | --- |
| Semantics document | `Semantics.md` covers scope/core subset, grammar/AST/HIR/IR mapping, static and dynamic judgments, memory/GOT, trap/UB, theorem inventory with status, TCB, exact-arithmetic connection, conformance evidence, references, and a final changelog row; stale draft/pending text is removed and the current state is stated accurately. |
| Report | `Phase0-Report.md` records what was proved, the 74/74 conformance result and artifact versions, E1–E7, Q2/Q3/Q5, T1 and the T2 fix, the TCB and assumptions, the platform caveat, the freeze record, and the handoff. |
| Registry | `semantics_version` is `1.0`, `dump_format_version` is `0.1.0`, every Phase 0 theorem/definition/evidence item has a specification reference and an honest status, and the freeze commit is recorded (placeholder finalized in the follow-up commit). |
| Proof audit | `lake build` passes; no `sorry` or `admit`; `#print axioms` reports only `propext`, `Classical.choice`, and `Quot.sound`; no unregistered top-level theorem. |
| Versioning | Every dump, case, result, and report header carries `1.0`; the format versions remain `0.1.0`; unknown versions are rejected; the committed report matches a fresh harness run byte-for-byte. |
| Tests and CI | `lake exe stc-sem-tests` passes; the registry, docs, placeholder/conformance gate, Markdown, Rust test/clippy/fmt, and corpus checks pass. |
| Claim boundary | The V0 differential label, the TCB, the exclusions, and the non-refinement statement are explicit in the semantics document, the report, and the dump/case/report schemas. |
| Owner decisions | No semantic definition or theorem change was needed and no version-bump incompatibility surfaced. Open items for owner awareness remain Q3 (layout-only characters), Q2 (guarded exact form), Q5 (initializer ordering), `core_check_complete` (descoped to Phase 2), and E1/T1 (result-less call dump gap). |

**Owner sign-off (2026-11-02).** The owner approved P0.7 and Phase 0 completion at `0cece10`;
this checklist and the documented gaps are accepted.

## 10. References

- [Semantics](Semantics.md)
- [Semantics Charter](Semantics-Charter.md)
- [Core HIR Dump](HIR-Dump.md)
- [Core Semantics Cases](Semantics-Cases.md)
- [Core Syntax Coverage](Core-Coverage.md)
- [Core IR Coverage](Core-IR-Coverage.md)
- [Phase 0 Implementation Plan](Plan-Phase0.md)
- [Verification Program Plan](Plan.md)
- [Testing and Verification](../Architecture/Testing-and-Verification.md)
- [GOT Artifacts and Schemas](../Architecture/GOT-Artifacts.md)
- [Runtime and Execution](../Architecture/Runtime-and-Execution.md)
- [AST, CST, and HIR](../AST/AST.md)
- [Grammar](../Syntax/Grammar.md)
- [UTF-8 Strings](../UTF8-Strings.md)
