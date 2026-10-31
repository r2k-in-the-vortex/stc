# Phase 0 Implementation Plan — Formal Semantics Foundation and Soundness

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-10-27
Status: Draft for review. Phase 0 decisions P0-D1 through P0-D10 are resolved (2026-10-27).
P0.0 is complete (review approved 2026-10-27): the charter, module skeleton, registry, CI checks,
and semantics document skeleton are in place. P0.1 is complete (review approved 2026-10-27):
the token model, lossless CST contract, surface AST, `Derives` relation, coverage table, and
`ast_span_wellformed` are in place. P0.2 is complete (2026-10-27): the typed HIR, declarative
typing judgment, `checkCore`, conversion relation, HIR well-formedness, and the soundness and
rejection-characterization theorems are in place. P0.3 is complete (review approved 2026-10-31):
the value and trap model, the small-step machine, the configuration typing, determinism, trap
soundness, exact arithmetic agreement, the sound/completeness results for the fuel interpreter,
`progress`, `preservation`, initial-state typing, and `type_soundness` are in place. Work was
paused for alignment with M6.5; P0.3 resumed on 2026-10-31. The compiler drift that prompted the
pause has stabilized at `stc` 1.0.0, the §19.4 register was rechecked on 2026-10-31 and still
holds, and the open questions were resolved in §19.8. P0.4 is complete (review approved 2026-10-31)
as WL-3 of [V2.0.0-PLAN.md](../../V2.0.0-PLAN.md); its scope decisions and execution record are in
§20. The P0.3 completion record and sign-off are in §19.12; the drift register is in §19.
Parent: [Verification Program Plan](Plan.md)
Inputs: [Testing and Verification](../Architecture/Testing-and-Verification.md),
[Architecture](../Architecture/Architecture.md),
[AST, CST, and HIR](../AST/AST.md),
[Grammar](../Syntax/Grammar.md),
[Tokenization](../Syntax/Tokenization.md),
[GOT Artifacts and Schemas](../Architecture/GOT-Artifacts.md),
[Runtime and Execution](../Architecture/Runtime-and-Execution.md),
[Decisions](../Decisions.md),
[UTF-8 Strings](../UTF8-Strings.md).

## 1. Objective

Phase 0 delivers a standalone, machine-checked formal semantics of a defined core of the stc
language and of the LLVM IR fragment the compiler emits for that core. The deliverable is not a
set of definitions alone: it includes the static and dynamic judgments, the meta-theoretic
soundness results, an executable reference interpreter, and a conformance report against the
Rust compiler's own corpus. The result is recorded at level S in the program plan: the semantics
is defined and proved sound, while the Rust implementation is tested against it and is not yet
proved to refine it.

Phase 0 must be useful even if no later phase runs. It fixes what the core language and the
emitted IR mean, gives later proofs a reviewed vocabulary, and gives compiler tests an oracle.
It also prevents the program from proving the wrong theorem: a parser, type-checker, or codegen
proof is only meaningful relative to a semantics someone has reviewed.

This document is the detailed execution plan for Phase 0. The [Verification Program
Plan](Plan.md) remains authoritative for the overall program; this plan may refine Phase 0's
work breakdown but does not change its level, scope, or exit claim without updating the parent.

## 2. Relationship to the program plan

The parent plan assigns Phase 0 the following S-level deliverables:

- `Stc.Syntax`, `Stc.Grammar`, `Stc.Hir`, `Stc.Sem`, `Stc.Codegen.Ir`, and
  `Stc.Codegen.Semantics`.
- A reference interpreter (`lake exe stc-sem`) and a conformance harness
  (`lake exe semantics-conformance`).
- Source soundness: determinism, progress, preservation, type soundness, core checker
  soundness, trap soundness, and exact-arithmetic trap agreement.
- Target soundness: determinism, checked-arithmetic refinement, memory safety, GOT soundness,
  trap soundness, and no undefined behavior on the covered fragment.
- A reviewed and versioned semantics document.

This plan decomposes that work into sub-phases P0.0 through P0.7, each with its own deliverables
and exit gate. The sub-phases are ordered so that every later proof depends only on reviewed
definitions, not on unfinished work.

## 3. Scope

### 3.1 In scope

- The core language subset defined in §3.3, including its surface AST, typed HIR, static
  semantics, and dynamic semantics.
- The core emitted-IR fragment defined in §3.4, including its syntax, well-formedness predicate,
  and operational semantics.
- The relationship between the source and target semantics at the level of shared values,
  traps, and exact arithmetic. Full compiler correctness is Phase 7, not Phase 0.
- An executable reference interpreter for the core HIR subset.
- A conformance harness comparing the reference interpreter with the Rust compiler's observable
  behavior on the core corpus.
- The semantics document, versioning, theorem registry entries, and handoff notes.

### 3.2 Out of scope

- Proving that the Rust parser, analyzer, or code generator refines the semantics. That is
  Phase 1 and the later V-level phases.
- The full language: floating point, strings, arrays, structs, enums, subranges, OOP,
  interfaces, references, pointers, standard function blocks, tasks, I/O, retained state, online
  change, and debug.
- Full LLVM semantics. Phase 0 defines only the emitted fragment; it does not model LLVM's
  optimizer, poison/undef generality, threads, atomics, or the full memory model.
- Proving the textual IR printer or parser. The target semantics is defined over the in-memory
  `IRFrag`; the printer/parser round-trip is Phase 5.
- Proving a compiler-correctness theorem. That is Phase 7.
- Performance, safety certification, and production integration of the reference interpreter.

### 3.3 Core language subset

The core subset is deliberately small. It is the smallest subset that exercises the compiler's
scalar semantics, control flow, functions, persistent globals, and traps.

| Area | Included in Phase 0 | Excluded until later phases |
| --- | --- | --- |
| Types | `BOOL`, `SINT`/`INT`/`DINT`/`LINT`, `USINT`/`UINT`/`UDINT`/`ULINT`, `BYTE`/`WORD`/`DWORD`/`LWORD`, `CHAR`/`WCHAR`/`UCHAR` | `REAL`/`LREAL`, `TIME`/date types, `STRING`/`WSTRING`/`USTRING`, arrays, structs, enums, subranges, references, pointers, class/FB/interface |
| Declarations | Scalar `VAR`, function `VAR_INPUT`, scalar `VAR_GLOBAL` roots, `FUNCTION`, `PROGRAM` | `VAR_OUTPUT`, `VAR_IN_OUT`, function blocks, classes, interfaces, methods, namespaces, configurations, resources, tasks |
| Expressions | Literals, variable access, unary and binary exact operators, comparisons, integer/bit/char conversions, `SEL`/`MIN`/`MAX`/`LIMIT`/`MUX`, calls to core functions and exact standard functions | Array/field access, method/interface calls, references, pointers, string operations, floating-point operations |
| Statements | Assignment, `IF`/`ELSIF`/`ELSE`, `CASE` over integer literals and ranges, `FOR`, `WHILE`, `REPEAT`, `EXIT`, `CONTINUE`, `RETURN`, empty | I/O `AT`, `RETAIN`, online change, test pragmas, debug hooks |
| Execution | One program, one task, one sequential cycle; scalar program fields and globals as GOT roots | Multi-task scheduling, I/O backends, retained state, online change, debug sessions |
| Standard library | Exact standard functions | Standard function blocks, timers, string/math functions, floating-point functions |
| Traps | `OVERFLOW`, `DIV_ZERO`, `RANGE` | Bounds, null-reference, conversion-to-real traps, and any trap that depends on excluded types |

Semantics decisions for the core:

- `BOOL` is a one-bit unsigned layout. In memory it is an `i8`; in SSA it is an `i1` normalized
  with `icmp ne 0`.
- Integer and bit-string operations follow the existing `Stc.Exact` models, including checked
  overflow, truncating division, masking of shift/rotate counts, and conversion rules.
- `CHAR`/`WCHAR`/`UCHAR` are unsigned integer layouts; `UCHAR` is a 32-bit Unicode scalar.
- Functions may be recursive. The reference interpreter carries fuel; the declarative semantics
  does not require termination.
- A program has scalar local variables and a single cycle body. Persistent scalar globals and
  scalar program fields are GOT roots. There is no heap, no aliasing beyond GOT cells, and no
  `VAR_IN_OUT`.
- `CASE` is restricted to integer selectors and integer literal/range labels. The source
  semantics does not require a particular lowering. `stc-codegen` currently lowers `CASE` to
  comparison branches, so `switch` is not part of the P0.4 fragment; Phase 5 adds it if lowering
  changes.
- `EXIT` and `CONTINUE` target the innermost enclosing loop. Their semantics is expressed with a
  structured control stack in the small-step relation.

### 3.4 Core IR fragment

The target fragment is the subset of textual LLVM IR that the compiler emits for the core
language. Phase 0 models this fragment natively in Lean; it does not import or port Vellvm.

| Area | Included in Phase 0 | Excluded until later phases |
| --- | --- | --- |
| Types | `void`, `i1`, `i8`, `i16`, `i32`, `i64`, `ptr`; the `{ iN, i1 }` overflow-intrinsic result; fixed arrays `[N x ptr]` (function table and root-pointer arrays) and `[N x i8]` (descriptor and metadata blobs) | `float`, `double`, general structs and arrays, vector types |
| Instructions | Integer `add`/`sub`/`mul`/`sdiv`/`udiv`/`srem`/`urem`, `and`/`or`/`xor`/`shl`/`lshr`/`ashr`, the `llvm.fshl`/`llvm.fshr` rotate intrinsics, `icmp`, `zext`/`sext`/`trunc`, `select`, `phi`, `br`, `ret`, `unreachable`, direct and indirect `call`, `alloca`, `load`, `store`, `getelementptr` for stack slots, root-pointer arrays, and the slot table, the `llvm.{s,u}{add,sub,mul}.with.overflow.iN` checked arithmetic intrinsics, `extractvalue` of their `{ iN, i1 }` result, and trap calls | `fadd`/`fsub`/`fmul`/`fdiv`/`frem`, vector ops, atomics, volatile access, `invoke`/`landingpad`, dynamic `alloca`, `insertvalue`, `llvm.memcpy`, `ptrtoint`/`inttoptr`, `switch`, descriptor/vtable/TYPEINFO blobs, process-image access |
| Globals | The slot table, the function-address table, the module descriptor blob, and other module-local read-only constants such as the metadata blob; module-local mutable pointers to the host API and slot table; the retain buffer | Runtime descriptor/vtable cells, process image, source string data, debug metadata |
| Entry points | Host ABI 2 entry-point signatures (production modules require minor 2.1; the current host minor is 2.3) and the cycle body; attach/prepare/shutdown are modeled as the initialization boundary | Full host lifecycle, swap, rollback, unload |
| Memory | Stack frames, GOT cells as opaque pointer-sized values, and the root-pointer array passed to a program body; no heap | Object records, process image, I/O memory, raw pointer arithmetic |
| Undefined behavior | No poison/undef, no out-of-bounds stack access, no misaligned access on covered programs; prove no UB under `WellFormed` | Full LLVM memory model, integer-pointer casts, provenance |

The fragment is the production module shape: debug-info builds, test instrumentation, coverage
counters, and debug checkpoint hooks are excluded. `WellFormed` is defined over the whole module,
including the generated entry-point wrappers and module globals. The P0.5 target semantics
interprets the covered user functions and the cycle body and treats attach/prepare/shutdown as
the initialization boundary.

The target semantics treats a GOT cell as an indexed, typed cell, not as an arbitrary pointer.
`WellFormed` records the slot table and each cell's kind; the semantics checks that a load or
store uses the matching kind. The cell kinds are `Data`, `Function`, `Descriptor`, `Direct`, and
`Import`, matching [GOT Artifacts §7.1](../Architecture/GOT-Artifacts.md) and
[Online Change and GOT §3.2](../Architecture/Online-Change-and-GOT.md). `Descriptor` cells are
structurally representable but semantically out of the core because OOP is excluded. Function
cells are loaded and called indirectly; the semantics relates a function cell to its `GotId` and
records the callable. This is enough for the core and leaves descriptors, vtables, and the
process image to Phase 5 and Phase 8.

The slot table is canonical in the sense of the five contiguous kind groups (`Data`, `Function`,
`Descriptor`, `Direct`, `Import`). Sorting identities within a group by `GotId` bytes, and imports
by service ID, is the Phase 3 theorem `slots_canonical`; it is not a P0.4 proof obligation.

### 3.5 Semantics versioning

The semantics is a product artifact with a version.

- `semantics_version` starts at `1.0` for the core subset.
- Definitions that later phases extend must be in separate modules or sections so a version
  bump is explicit.
- A change to an existing definition or theorem invalidates downstream proofs and requires a
  version bump and a registry update.
- The conformance harness records the semantics version and the HIR dump format version in every
  report.

## 4. Deliverables and repository layout

```text
lean/
  Stc/
    Syntax/          tokens, spans, CST contract, surface AST
    Grammar/         Derives relation, AST mapping
    Hir/             typed HIR types, places, expressions, statements
    Sem/             values, memory, traps, typing, evaluation, soundness
    Codegen/
      Ir.lean        IRFrag and WellFormed
      Semantics.lean operational semantics and target soundness
  Verification/
    Semantics/       reference interpreter and conformance harness
    Registry/        theorem registry and status generation

specification/Verification/
  Plan.md            authoritative program plan
  Plan-Phase0.md     this document
  Semantics.md       reviewed semantics reference (Phase 0 deliverable)
  Core-Coverage.md   core production-to-definition coverage (P0.1)
  Core-IR-Coverage.md core emitted-IR-to-definition coverage (P0.4)
  Phase0-Report.md   results, conformance report, gaps, handoff

tests/verification/semantics/
  <case>/case.json   entry point, inputs, cycles, expected result
```

The exact file names are provisional; the module boundaries and the separation between
executable definitions and proof-only lemmas are not.

## 5. Work breakdown

| Sub-phase | Work | Effort | Depends on | Exit gate |
| --- | --- | --- | --- | --- |
| P0.0 | Charter, core subset, interfaces, scaffolding, CI | S | none | Decisions recorded; skeleton builds; checks active |
| P0.1 | Surface syntax, CST contract, surface AST, `Derives` | M | P0.0 | Definitions and basic lemmas complete |
| P0.2 | HIR, core typing judgment, core checker | M/L | P0.0, P0.1 | Checker soundness and rejection characterization |
| P0.3 | Source dynamic semantics and soundness | L | P0.1, P0.2 | Determinism, progress, preservation, type/trap soundness |
| P0.4 | Target syntax and `WellFormed` | M | P0.0, P0.1 | `IRFrag` and well-formedness defined; structural lemmas |
| P0.5 | Target operational semantics and soundness | L | P0.4, P0.2 | Target determinism, memory/GOT/trap soundness, no UB |
| P0.6 | Reference interpreter and conformance harness | M | P0.2, P0.3, P0.5 | Core corpus runs; disagreements triaged |
| P0.7 | Semantics document, review, versioning, handoff | S/M | all | Document reviewed; version 1.0; handoff accepted |

Effort classes are inherited from the parent plan: S is up to one engineer-month, M is one to
three, L is three to six, and XL is six to twelve or more. Phase 0 as a whole is L/XL. The
sub-phase estimates are not additive commitments; P0.1 and P0.2 can overlap, and P0.4 can start
once P0.0 and P0.1 have fixed the vocabulary.

The conservative sub-phase estimates sum above the parent plan's L/XL midpoint. The planning
number is the critical path, P0.0 → P0.1 → P0.2 → P0.3 → P0.6 → P0.7, with P0.4 and P0.5 in
parallel. If the schedule tightens, trim the target fragment in P0.4/P0.5 or move the remaining
extensions to a later phase; do not weaken the source semantics or the conformance gate.

Dependency shape:

```text
P0.0 ──> P0.1 ──> P0.2 ──> P0.3 ──> P0.6 ──> P0.7
  │         │         │
  │         └──> P0.4 ──> P0.5 ──────┘
  └──────────────────────────────────┘
```

## 6. Sub-phase details

### P0.0 — Charter, scope, and scaffolding

**Goal.** Freeze the Phase 0 scope and build the environment so every later sub-phase starts
from agreed definitions and checks.

**Work.**

- Approve the core subset in §3.3 and the target fragment in §3.4. Record every exclusion with
  the phase that will add it.
- Write the semantics charter: purpose, S-level claim, TCB boundary, relationship to the program
  plan, versioning, and the rule that later phases extend rather than silently change the core.
- Confirm and record the resolved Phase 0 decisions in §14: core subset, target fragment,
  recursive-plus-small-step semantics, JSON HIR dump, dependency policy, conformance oracle,
  document location, recursion/fuel, and acceptance threshold.
- Create the Lean module skeletons and the `lean/Verification/Semantics` tree. Ensure `lake
  build` passes with empty or stub modules.
- Create the verification registry entries for the Phase 0 theorem inventory, with each entry
  naming its specification section and status. A checked-in TOML table plus a small validation
  script is enough; the full registry tooling is Phase 1.
- Add CI checks: no `sorry`/`admit` in Phase 0 modules, `#print axioms` audit for top-level
  theorems, registry resolution, and a semantics-conformance placeholder that fails until it
  runs real cases.
- Write the `specification/Verification/Semantics.md` skeleton with the section headings from
  §9.

**Deliverables.** Approved charter; module skeleton; registry entries; CI checks; Semantics.md
skeleton.

**Exit criteria.** The core subset and target fragment are frozen; decisions P0-D1 through
P0-D10 are recorded; `lake build` passes; the no-sorry, axiom, registry, and placeholder checks
run in CI; the semantics document skeleton is reviewed.

**Risks.** Scope negotiation can stall the phase. Use timeboxes and default to excluding a
feature; adding it later is cheaper than proving the wrong core.

### P0.1 — Surface syntax and grammar relation

**Goal.** Define the surface syntax and the declarative grammar that later parser proofs and the
AST mapping cite.

**Work.**

- Define tokens, token kinds, trivia, and spans for the core subset.
- Define the lossless CST contract: token/trivia spans cover the source exactly, in order, with
  no gaps or overlaps. The full rowan representation may remain a contract-level abstraction.
- Define the surface AST as an inductive family for the core declarations, expressions,
  statements, and programs.
- Define the grammar relation `Derives : Grammar → Ast → Prop` (or an equivalent indexed
  relation) mirroring the core EBNF productions.
- Define the AST↔CST mapping relation and the source-span mapping. This is the relation Phase 4
  uses for `parse_lossless`, `parse_sound`, and `parse_complete`.
- Record the grammar rules and rule IDs covered by the core subset, with links to
  [Grammar](../Syntax/Grammar.md) and [Tokenization](../Syntax/Tokenization.md).

**Deliverables.** `Stc.Syntax` and `Stc.Grammar` definitions; basic lemmas such as
`ast_span_wellformed`; a coverage table mapping core grammar productions to Lean definitions.

**Exit criteria.** The core surface AST and grammar relation are complete for §3.3; every core
production has a Lean constructor or relation rule; the coverage table is reviewed; no parser
correctness claim is made yet.

**Risks.** Modeling the rowan CST too literally will consume the phase. Keep the CST as a
contract and put the proof weight on the AST and grammar relation. If a construct is hard to
model, exclude it from the core and record it.

### P0.2 — HIR and core static semantics

**Goal.** Define the typed HIR and prove the core static semantics sound and the core checker
sound.

**Work.**

- Define the typed HIR inductive family for the core: types, expressions, statements, places,
  declarations, and programs. Align it with `stc-hir` and the typed-AST views in
  [AST, CST, and HIR](../AST/AST.md).
- Define the declarative typing judgment `HasType` (or `Typing`) for the core, including
  assignment, conditions, calls, conversions, and the exact standard functions.
- Define the core algorithmic checker `checkCore` and prove it returns a typing derivation or a
  characterized rejection.
- Define `hir_wellformed` and prove that accepted HIR satisfies it.
- Define the core conversion relation and prove `conversion_legal_iff` for the included types,
  using the Phase 0 exact models.
- Align the checker's accepted subset with the Rust analyzer's diagnostics for the core; record
  known deliberate differences.

**Deliverables.** `Stc.Hir`; `Stc.Sem.Typing`; `checkCore`; soundness, completeness or
rejection-characterization, and well-formedness theorems.

**Exit criteria.** `core_check_sound` is proved; `core_check_complete` or
`core_rejection_characterized` is proved; HIR well-formedness is defined and preserved by the
checker; the conversion relation matches `Stc.Exact` on the included types.

**Risks.** The declarative judgment can drift from the Rust analyzer. Keep the judgment small and
review it against the specification; do not try to encode every later language feature.

### P0.3 — Source dynamic semantics and soundness

**Goal.** Define what core programs do and prove the source semantics sound.

**Work.**

- Define values, types, widths, locals, globals, GOT roots, memory, and traps for the core.
- Define a small-step operational semantics for expressions, statements, and program cycles,
  including `EXIT`, `CONTINUE`, `RETURN`, recursion, and traps.
- Define `HasType` for configurations and prove progress and preservation for the core.
- Prove determinism, type soundness, trap soundness, and exact-arithmetic trap agreement.
- Define the executable reference interpreter as a fuel-based function and prove it sound with
  respect to the declarative semantics; prove completeness for terminating runs within fuel.
- Shape the interpreter and the step relation for later refinement: thread explicit state,
  encode control flow (`continue`/`break`/`return`/`trap`) as an explicit result, and keep the
  recursive model structurally close to the imperative implementation. The recursive model is
  the preferred Phase 1 refinement target; the step relation remains the reference for
  soundness.
- Record the semantics of the cycle boundary: one program, one cycle, deterministic scheduling.

**Theorems.** `eval_deterministic`, `progress`, `preservation`, `type_soundness`,
`trap_soundness`, `checked_arith_traps_exactly`, `interpreter_sound`,
`interpreter_complete` where feasible.

**Deliverables.** `Stc.Sem` modules; source soundness proofs; executable interpreter prototype.

**Exit criteria.** All source soundness theorems are proved with no `sorry`; the interpreter is
sound and handles the core corpus; the cycle boundary is defined; the semantics document's
source-semantics sections are complete.

**Risks.** Progress and preservation for an imperative language with loops and traps are the
main proof-engineering risk. Use a structured control stack and explicit terminal states; state
the progress theorem as "not stuck except at a documented trap or halt," not as "always steps."
Keep the core free of aliasing and heap to make the memory model tractable.

### P0.4 — Target syntax and well-formedness

**Goal.** Define the emitted-IR fragment and the structural invariants the target semantics
assumes.

**Work.**

- Define `IRFrag` over the whole emitted production module: type definitions, globals and
  constants, functions, basic blocks, instructions, terminators, the slot table, the
  function-address table, and the generated entry-point wrappers. Debug-info builds, test
  instrumentation, coverage counters, and debug checkpoint hooks are out of scope.
- Define the cell-kind model with all five GOT kinds (`Data`, `Function`, `Descriptor`, `Direct`,
  `Import`) and a slot table grouped in the canonical kind order. Sorting within a group by
  `GotId`/service ID remains Phase 3's `slots_canonical`.
- Define `IRContract`, mirroring the Rust verifier's contract: persistent roots, user callables,
  the module-local global allowlist, the slot table, the required entry points, and the
  descriptor/function-table shapes.
- Define `WellFormed contract frag`: type correctness, SSA/dominance and definedness, block
  terminators, `phi` firstness and predecessor/arity agreement, slot references in range and
  kind-correct, the grouped slot table, the host ABI 2 production entry-point signatures and
  linkage, no direct persistent-root or user-callable references (the function table is the
  documented exception), no cell writes, pre-attach entry points that do not use cells, and the
  local-global allowlist.
- Prove the structural lemmas in §7.1. The predicate is the contract; each lemma is a projection
  consumed by P0.5 and Phase 5.
- Record the rule-by-rule alignment with `crates/stc-codegen/src/verify.rs` and the intentional
  differences: Lean checks type correctness, SSA/dominance, and `phi` shape intrinsically, while
  the Rust verifier relies on the builder and checks structural facts; Rust does not check
  canonical slot ordering because the catalog owns it; descriptor/function-table content pairing
  is Phase 5.
- Update the semantics document where P0.4 fixes vocabulary or the fragment.
- Write the target fragment coverage table in `Core-IR-Coverage.md`, mapping every included
  instruction, terminator, type, global shape, and entry point to its Lean definition and
  specification section.

**Deliverables.** `Stc.Codegen.Ir`; `IRFrag`, `IRContract`, `WellFormed`, the cell-kind and slot
model; the §7.1 structural lemmas; [Core-IR-Coverage.md](Core-IR-Coverage.md); registry entries;
the verifier alignment record.

**Exit criteria.** `IRFrag` covers the emitted production module; every included construct has a
Lean constructor and a coverage-table row; `WellFormed` is defined; every structural lemma in
§7.1 is proved with no `sorry`; the Lean build and registry checks pass; the P0.4 review is
approved and recorded.

**Risks.** The textual IR is large and evolving. Define only the emitted fragment and reject
everything else rather than modeling it. Keep the printer/parser, the full LLVM memory model, and
OOP/process-image constructs out of scope. The wrapper bodies are generated, so re-run the
fragment coverage table when `stc-codegen` changes.

### P0.5 — Target operational semantics and soundness

**Goal.** Define what the emitted IR does and prove the target semantics sound for the core
fragment.

**Work.**

- Define `IRState`: program counter, stack frame, local memory, GOT root map, slot table,
  function cells, call stack, and trap state.
- Define the small-step relation `IRStep` for the instructions in §3.4, including checked
  arithmetic, trap calls, loads/stores through GOT cells, and direct/indirect calls.
- Define `IRSem` for a module's cycle entry and the initialization boundary.
- Define a recursive executor for the core IR fragment and prove it equivalent to the small-step
  relation. The recursive executor is the preferred Phase 1 refinement target; the small-step
  relation is the reference for progress and memory soundness.
- Prove target determinism, progress for well-formed states, memory safety, GOT soundness,
  trap soundness, checked-arithmetic refinement, and no undefined behavior on covered programs.
- Define the GOT model: a root map from `GotId` to value, a slot table from slot to cell kind,
  and the invariant that every cell access matches its kind.
- Document the UB policy: the core fragment excludes poison/undef, out-of-bounds stack access,
  misaligned access, and raw pointer arithmetic; `WellFormed` plus the source-level checks
  establish no UB for covered programs.

**Theorems.** `ir_deterministic`, `ir_progress`, `ir_memory_safe`, `ir_got_sound`,
`ir_trap_sound`, `ir_checked_arith_refines_exact`, `ir_no_ub`.

**Deliverables.** `Stc.Codegen.Semantics`; target soundness proofs; target fragment coverage
table.

**Exit criteria.** All target soundness theorems are proved with no `sorry`; every instruction
in the core fragment has a semantics rule; the GOT and trap models are documented; the target
semantics is connected to `Stc.Exact` for checked arithmetic.

**Risks.** LLVM's memory model and UB are the main hazards. Keep the fragment restricted and
the memory model abstract; do not attempt to model general LLVM. If a construct cannot be given
a sound, small semantics, exclude it and record the gap.

### P0.6 — Reference interpreter and conformance harness

**Goal.** Make the semantics executable and compare it with the Rust compiler on the core corpus.

**Work.**

- Define a stable, versioned HIR dump for the core subset. Start from
  `crates/stc-hir/src/dump.rs`, but do not depend on a debug text format; define the exact
  fields and version.
- Implement `lake exe stc-sem` to read the dump, run the reference interpreter with case inputs,
  and emit a machine-readable result: value, trap, global snapshot, cycles, and traces.
- Define the case format under `tests/verification/semantics/<case>/case.json`: entry point,
  inputs, cycle count, expected result, and feature tags.
- Implement `lake exe semantics-conformance` to:
  - build the HIR dump and a runnable artifact with the Rust compiler,
  - run the reference interpreter,
  - run the compiled artifact or test host,
  - compare observable results and traps,
  - emit a report with the semantics version and dump format version.
- Seed the corpus from existing conformance and execution tests that fit the core subset, plus
  targeted cases for every core feature and trap.
- Define disagreement triage: Lean semantics bug, Rust compiler bug, dump bug, or unmodeled
  feature. Every disagreement is fixed or explicitly excluded with a written reason before
  Phase 0 exits.

**Deliverables.** HIR dump specification and emitter; `stc-sem`; conformance harness; core
corpus; committed report.

**Exit criteria.** The harness runs the full core corpus deterministically; all cases agree or
have a documented exclusion; the report is committed; a disagreement is filed with a
reproduction; the harness is part of CI.

**Risks.** The dump can become an unstable compiler-internals leak. Version it, validate it, and
keep it independent of Rust `DefId`s and arena indices. The compiled artifact path depends on
clang; keep a pure-function comparison path that does not, so semantics work can continue when
the toolchain is unavailable.

### P0.7 — Semantics document, review, versioning, and handoff

**Goal.** Turn the definitions and proofs into a reviewed, versioned artifact that later phases
can cite.

**Work.**

- Complete `specification/Verification/Semantics.md`: scope, core subset, grammar/AST/HIR/IR
  mapping, static and dynamic judgments, memory/GOT model, trap/UB policy, theorem inventory,
  TCB, and changelog.
- Complete `specification/Verification/Phase0-Report.md`: what was proved, the conformance
  results, known gaps and exclusions, and the handoff.
- Run the review: internal review of the semantics document, specification traceability, proof
  audit (no `sorry`, axiom audit), and a registry check.
- Tag `semantics_version = 1.0` and freeze the core definitions. Record the exact commit.
- Write the handoff note: what Phase 1, 2, 4, 5, 6, 7, and 8 consume, what they may extend, and
  what requires a version bump. Record the refinement handoff explicitly: recursive models,
  loop-shaped step relations, explicit state and control-flow encoding, termination/fuel
  obligations, and the intended Phase 1 bridge points.
- Update the parent plan's Phase 0 status and the verification registry.

**Deliverables.** Semantics document; Phase 0 report; handoff note; version tag; registry
update.

**Exit criteria.** The semantics document and report are reviewed; all Phase 0 theorems are
registered with specification references; no `sorry` remains; the handoff is accepted; the
parent plan reflects the completed state.

**Risks.** Documentation can lag the proofs. Draft the semantics document incrementally in
P0.1–P0.5 rather than writing it at the end.

## 7. Theorem inventory

The inventory is the contract for Phase 0. Statements are indicative; the semantics document
fixes the final preconditions and notation.

| Theorem | Shape | Sub-phase | Depends on |
| --- | --- | --- | --- |
| `ast_span_wellformed` | AST nodes carry spans that map to the CST/source | P0.1 | syntax, grammar |
| `core_check_sound` | `checkCore p = .ok env → HasType p env` | P0.2 | HIR, typing |
| `core_check_complete` | `HasType p env → ∃ env', checkCore p = .ok env'` where intended | P0.2 | HIR, typing |
| `core_rejection_characterized` | Every rejected core program fails a named typing rule | P0.2 | HIR, typing |
| `hir_wellformed` | `checkCore p = .ok env → HirWellFormed p` | P0.2 | HIR |
| `conversion_legal_iff` | The core conversion relation accepts exactly the specified conversions | P0.2 | exact models |
| `eval_deterministic` | `Eval e s v₁ → Eval e s v₂ → v₁ = v₂` | P0.3 | source semantics |
| `progress` | Well-typed, well-formed configurations are terminal, trap, or step | P0.3 | source semantics |
| `preservation` | `ConfigTyped u c → Step u c c' → ConfigTyped u c'` | P0.3 | source semantics |
| `type_soundness` | The initial configuration is typed; every reachable configuration is typed and not stuck, with trap documentation carried by `trap_soundness` | P0.3 | source semantics |
| `trap_soundness` | Every terminal trap is in the specified trap set | P0.3 | source semantics |
| `checked_arith_traps_exactly` | Checked arithmetic traps exactly on overflow or divide by zero | P0.3 | exact models |
| `interpreter_sound` | `stcSem fuel e s = .ok v → EvalStar e s v` | P0.3 | interpreter |
| `interpreter_complete` | Terminating runs within fuel are reproduced by the interpreter | P0.3 | interpreter |
| `ir_deterministic` | `IRStep ir s s₁ → IRStep ir s s₂ → s₁ = s₂` | P0.5 | target semantics |
| `ir_progress` | Well-formed non-terminal IR states step or trap | P0.5 | target semantics |
| `ir_memory_safe` | Well-formed IR accesses only valid stack/GOT storage of the right kind | P0.5 | target semantics |
| `ir_got_sound` | GOT loads return the root value recorded by the slot table | P0.5 | target semantics, GOT model |
| `ir_trap_sound` | Every IR trap is on a specified trap path | P0.5 | target semantics |
| `ir_checked_arith_refines_exact` | IR checked arithmetic plus trap equals `Stc.Exact` | P0.5 | exact models |
| `ir_recursive_model_equiv` | `irExecRec ir s = r ↔ IRStepStar ir s r` | P0.5 | target semantics |
| `ir_no_ub` | `WellFormed ir → ¬ UB ir` on the covered fragment | P0.5 | target semantics |
| `semantics_conformance` | The harness reports agreement for every core corpus case or a documented exclusion | P0.6 | interpreter, harness |
| `semantics_versioned` | Every report and dump carries the semantics and format versions | P0.6 | harness |

### 7.1 P0.4 structural lemmas

The P0.4 predicate is `Stc.Codegen.WellFormed`; it takes an `IRContract` and an `IRFrag`. The
lemmas below are its projections. They are the structural contract P0.5 and Phase 5 consume.
Names are fixed once registered in [`lean/verification-status.toml`](../../lean/verification-status.toml).

| Lemma | Shape | Depends on |
| --- | --- | --- |
| `wellFormed_blocks_have_terminators` | Every block in a well-formed function has exactly one terminator after its instructions | IRFrag, WellFormed |
| `wellFormed_phis_first` | Every `phi` precedes all non-`phi` instructions in its block | IRFrag, WellFormed |
| `wellFormed_phi_predecessors_match` | A `phi` has exactly one incoming pair per CFG predecessor of its block, and no other labels | IRFrag, WellFormed |
| `wellFormed_types_correct` | Every instruction and terminator is type-correct for the core IR types, and `ret` matches the function signature | IRFrag, WellFormed |
| `wellFormed_uses_defined` | Every local operand is a parameter or a dominating definition, and parameter/definition names are unique | IRFrag, WellFormed |
| `wellFormed_module_shapes` | Type-definition, global, declaration, and function names are unique; type definitions, global initializers, and declarations are well-typed; declarations are known intrinsics | IRFrag, WellFormed |
| `wellFormed_slot_refs_in_range` | Every slot index is below the slot-table length | IRFrag, IRContract, WellFormed |
| `wellFormed_slot_refs_kind_correct` | Calls use `Function`/`Import` cells, loads and stores use `Data`/`Direct` cells, and cell addresses are never stored | IRFrag, IRContract, WellFormed |
| `wellFormed_slot_table_canonical` | The slot table's cell kinds are non-decreasing in the canonical `Data`, `Function`, `Descriptor`, `Direct`, `Import` group order | IRFrag, WellFormed |
| `wellFormed_entry_points_match_contract` | The required entry points exist with the host ABI 2 production signatures and external linkage | IRFrag, IRContract, WellFormed |
| `wellFormed_no_direct_root_or_callable` | No body or global initializer names a persistent root or user callable directly; the function table is the only exception | IRFrag, IRContract, WellFormed |
| `wellFormed_pre_attach_clean` | The descriptor and function accessors do not use the slot table or make indirect calls | IRFrag, IRContract, WellFormed |
| `wellFormed_local_globals_allowed` | Direct global loads and stores target only the contract's local-global allowlist | IRFrag, IRContract, WellFormed |
| `wellFormed_cells_not_written` | No instruction stores through a cell address | IRFrag, IRContract, WellFormed |
| `wellFormed_descriptor_and_table` | The descriptor blob and function table have the contract's constant/internal array shapes, and the table entries match the fragment's function-table list | IRFrag, IRContract, WellFormed |

`slots_canonical` and `slot_table_valid_iff_canonical` remain Phase 3 theorems; P0.4 defines only
the grouped kind order. Descriptor/function-table content pairing remains Phase 5's
`descriptor_and_table_pair`.

The source recursive-model equivalence is the pair `interpreter_sound` and
`interpreter_complete`; the target recursive-model equivalence is `ir_recursive_model_equiv`.
These are the internal Lean equivalences that Phase 1 will use as refinement targets.

The conformance harness result is evidence, not a theorem. The report must not describe it as a
proof that the Rust compiler refines the semantics.

## 8. Proof architecture

### 8.1 Declarative judgments and algorithmic checkers

Follow the pattern the program plan requires: define the declarative judgment first, then the
algorithmic checker, then prove soundness and completeness or characterize rejection. This
applies to the core typing judgment (`HasType` versus `checkCore`) and to the source semantics
(`Step`/`Eval` versus `stcSem`).

### 8.2 Small-step source semantics

Define a configuration as the expression or statement being evaluated, a local store, a global
root map, a call stack, and a control stack for structured statements. `Step` is a relation over
configurations. Terminal configurations are values, halts, and traps. This shape makes progress
and preservation expressible and keeps `EXIT`/`CONTINUE`/`RETURN` explicit.

The executable interpreter is a fuel-based total function over the same configuration type. Prove
`interpreter_sound` against the reflexive-transitive closure of `Step`, and prove a completeness
statement only for runs that terminate within the supplied fuel.

### 8.3 Progress and preservation in an imperative language

State progress as: a well-typed, well-formed, non-terminal configuration either steps or is a
documented trap/halt. Do not claim that every well-typed program terminates. Preservation is
stated for the configuration typing judgment, including the control stack. Loops are handled
by the control stack and by induction on the step relation, not by a global termination
argument.

### 8.4 Target memory and GOT model

The target state has:

- a stack frame with typed slots;
- a root map `GotId → Value` and a slot table `Slot → CellKind`;
- a function table `GotId → Callable`;
- a call stack and a program counter.

A GOT cell is an index, not an arbitrary pointer. `WellFormed` records cell kinds and the slot
table; the semantics checks kind agreement on every load/store and call. This avoids modeling
LLVM pointer provenance while still modeling the compiler's actual access pattern.

### 8.5 Traps and undefined behavior

Traps are explicit terminal states with a trap code and span. `trap_soundness` says every trap
is in the specified set. `ir_no_ub` says the covered fragment does not reach an undefined state:
no poison/undef, no out-of-bounds stack access, no misaligned access, and no raw pointer
arithmetic. The UB policy is part of the semantics document and the target fragment definition.

### 8.6 Exact arithmetic refinement

Define the target checked operations in terms of `Stc.Exact` where possible, and prove
`ir_checked_arith_refines_exact`: the IR instruction sequence plus its trap path has the same
result or trap as the exact model. This is the bridge between Phase 0's target semantics and the
existing exact-function proofs.

### 8.7 Recursive model, loop-shaped semantics, and the Phase 1 handoff

A structurally recursive Lean function is usually easier to prove correct than an imperative
loop. The practical pattern, used by L4YAML and supported by Aeneas, is:

1. Define a recursive model of the computation.
2. Prove its properties and its agreement with the declarative semantics.
3. Prove the Rust loop implementation equivalent to the model (Phase 1), so the loop inherits
   the properties.

Phase 0 applies the first two steps internally and prepares the third:

- Source: `Eval`/`Step` is the declarative reference; `stcSem` is the recursive executable
  model; `interpreter_sound` and `interpreter_complete` are the equivalence.
- Target: `IRStep` is the loop-shaped small-step relation; `irExecRec` is the recursive
  executor; `ir_recursive_model_equiv` proves they agree.
- State: both models thread explicit locals, globals, GOT cells, memory, call stack, and control
  stack. Control flow is an explicit result (`continue`/`break`/`return`/`trap`), not a host
  exception. This is the shape Aeneas can translate and the shape a manual loop proof can relate
  to.
- Termination: the source executable model is fuel-based; the target recursive executor should
  have a structural or lexicographic termination argument where possible. Phase 1 supplies the
  Rust loop's termination measure.

Phase 0 does not prove any Rust function equivalent to these models, and it does not choose
Aeneas/Charon versus extraction. It ends with the recursive models, the loop-shaped relations,
their equivalence theorems, and a written handoff that names the intended refinement points.
This is the concrete answer to the "recursive Lean function plus Rust loop equivalence" insight:
Phase 0 builds and proves the recursive half and the internal equivalence, and Phase 1 performs
the Rust-loop half.

## 9. Rust/Lean interface

### 9.1 HIR dump

The reference interpreter consumes a stable HIR dump, not source text. This keeps Phase 0
independent of parser completeness and avoids duplicating the front end in Lean.

The dump must include:

- a format version and the semantics version;
- the core module: functions, programs, globals, and types;
- function signatures and parameter/local declarations;
- statements and expressions with resolved places, explicit casts, call targets, and constant
  values;
- GOT root keys and scalar program fields;
- source spans for traps;
- a feature tag for every construct outside the core, so the harness rejects rather than
  silently misinterprets.

The dump must not expose Rust `DefId`s, arena indices, or debug text as stable identity. Use
canonical names and keys. The dump is validated against a schema and rejected on an unknown
version or an unmodeled construct.

### 9.2 Reference interpreter

`lake exe stc-sem` reads a dump and a case descriptor, runs the core evaluator, and writes a
machine-readable result. It reports values, traps, global snapshots, cycle counts, and an
optional trace. It is a verification tool, not a production interpreter; performance is not a
goal.

### 9.3 Conformance harness

`lake exe semantics-conformance` orchestrates the comparison:

1. Run the Rust compiler to produce the core HIR dump and a runnable artifact.
2. Run `stc-sem` on the dump and case inputs.
3. Run the compiled artifact or the existing test host and capture its observable result.
4. Compare values, traps, and global snapshots.
5. Emit a report keyed by case and feature, with the semantics and format versions.

The corpus starts from existing conformance and execution cases that fit the core subset, plus
targeted cases for every core type, operator, statement, conversion, trap, and GOT access.
Cases outside the core are tagged and excluded explicitly.

### 9.4 Disagreement triage

Every disagreement is classified as one of:

- Lean semantics bug;
- Rust compiler bug;
- HIR dump bug;
- unmodeled or excluded feature.

The case is fixed or excluded with a written reason. A silent exclusion is a Phase 0 exit
failure. The triage log is part of `Phase0-Report.md`.

## 10. Semantics document

`specification/Verification/Semantics.md` is the reviewed reference. It contains:

- scope, core subset, and exclusions;
- grammar/AST/HIR/IR mapping;
- static semantics: typing judgment and checker;
- dynamic semantics: source small-step rules and target rules;
- values, memory, GOT, traps, and UB policy;
- exact-arithmetic connection;
- theorem inventory with proof status;
- TCB and assumptions;
- version and changelog.

The document is written incrementally with P0.1–P0.5 and reviewed at P0.7. Later phases cite it
by version and section; they do not redefine its concepts informally.

## 11. CI and proof engineering

- `lake build` must pass for every Phase 0 module.
- CI fails on `sorry` or `admit` in Phase 0 modules.
- Top-level theorems are audited with `#print axioms`; any non-standard axiom is listed in the
  TCB.
- The verification registry resolves every Phase 0 theorem to a Lean name and a specification
  section.
- `lake exe stc-sem` has unit and corpus tests.
- `lake exe semantics-conformance` runs on the core corpus and commits or checks the report.
- The docs and Markdown checks cover `Semantics.md` and `Phase0-Report.md`; the profile
  identifier is present.
- Mathlib is not used in executable models. If it is adopted for proofs, it is confined to a
  proof-only library and pinned; the executable kernel and interpreter remain core Lean plus
  Batteries.
- `native_decide` is avoided in trusted theorems unless the trust cost is documented and
  accepted.

## 12. Risks and mitigations

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Core subset too large | Phase 0 never finishes | Freeze §3.3; exclude aggressively; add features in later phases |
| Semantics/implementation drift | The S-level reference is wrong or unused | Conformance harness; versioning; triage log |
| Progress/preservation proof is expensive | Schedule slip | Small-step semantics with explicit terminal states; restrict the core; state honest claims |
| LLVM memory model/UB explodes | Target semantics stalls | Abstract GOT cells and stack frames; exclude poison/undef and raw pointers; prove no UB only on the fragment |
| HIR dump becomes unstable | Harness breaks; false confidence | Version and schema the dump; validate; avoid compiler internals |
| Proof-engineering time | Phase 0 misses its gate | Timebox P0.0; prioritize definitions and key theorems; use the interpreter as an early deliverable |
| Spec ambiguity | Proves the wrong property | Semantics document review; decisions recorded; traceability to specification sections |
| Corpus coverage is thin | Conformance gives false assurance | Seed from existing conformance/execution suites; add a case for every core feature and trap |
| Mathlib or Lean upgrade churn | Proof maintenance | Pin the toolchain; keep Mathlib out of executable models; nightly upgrade checks |

## 13. Exit criteria and definition of done

Phase 0 is done when:

- The core language subset and target fragment are frozen and documented.
- `Stc.Syntax`, `Stc.Grammar`, `Stc.Hir`, `Stc.Sem`, `Stc.Codegen.Ir`, and
  `Stc.Codegen.Semantics` are defined for the core.
- The theorem inventory in §7 is proved with no `sorry`, or a theorem is explicitly descoped
  with a written justification and a registry entry.
- The recursive and loop-shaped models agree (`interpreter_sound`/`interpreter_complete` and
  `ir_recursive_model_equiv`), and the semantics document records the Phase 1 refinement points.
- `lake exe stc-sem` runs the core corpus.
- `lake exe semantics-conformance` reports agreement for every core corpus case, or a documented
  exclusion with a triage reason.
- `specification/Verification/Semantics.md` is reviewed, versioned at `1.0`, and traceable to
  the specification.
- `specification/Verification/Phase0-Report.md` records results, gaps, exclusions, and the
  handoff.
- The registry and parent plan reflect the completed S-level claim: the semantics is defined
  and proved sound; the Rust implementation is tested against it, not proved equivalent.

## 14. Decisions

| ID | Decision | Resolution |
| --- | --- | --- |
| P0-D1 | Exact core language subset | **Decided 2026-10-27:** use §3.3 as written; exclude floats, strings, aggregates, OOP, references, FBs, tasks, I/O, and online change |
| P0-D2 | Exact target IR fragment and memory model | **Decided 2026-10-27:** use §3.4 as written; abstract GOT cells and stack frames; exclude general LLVM memory and UB |
| P0-D3 | Small-step versus functional semantics | **Decided 2026-10-27:** define both the recursive executable model and the loop-shaped small-step relation; prove them equivalent and expose both as refinement targets |
| P0-D4 | HIR dump format and version | **Decided 2026-10-27:** stable, versioned JSON with schema validation; no Rust internals; start from the `stc-hir` dump |
| P0-D5 | Mathlib/Batteries policy | **Decided 2026-10-27:** core Lean plus Batteries for executable models; Mathlib proof-only after a build-time spike |
| P0-D6 | Floats, time, strings, and aggregates | **Decided by P0-D1 2026-10-27:** excluded from Phase 0; later phases own them |
| P0-D7 | Conformance oracle | **Decided 2026-10-27:** compare the compiled artifact for execution and `stc test` for function cases; keep a no-clang pure-function path |
| P0-D8 | Semantics document location | **Decided 2026-10-27:** `specification/Verification/Semantics.md`, versioned with the plan |
| P0-D9 | Recursion and fuel | **Decided 2026-10-27:** include recursion, fuel the interpreter, and do not require a source termination proof |
| P0-D10 | Conformance acceptance threshold | **Decided 2026-10-27:** every core case agrees or has a written triage exclusion; no silent exclusions |

## 15. Immediate next steps

1. Review this plan; decisions P0-D1 through P0-D10 are resolved.
2. Freeze §3.3 and §3.4 and open the registry entries.
3. Create the Lean module skeleton and CI checks from P0.0.
4. Write the `Semantics.md` skeleton and the core grammar/AST mapping table.
5. P0.1 is complete; start P0.2 and prototype the HIR dump in parallel once P0.2's types are
   stable.
6. Build `stc-sem` against a small hand-written core dump before the full conformance harness.
7. Add the core corpus and run the first conformance report.
8. Schedule the P0.7 review once the source and target soundness theorems are proved.
9. P0.3 is complete and review-approved (2026-10-31); see §19.12. P0.4 (WL-3) is complete;
   its scope decisions and execution record are in §20.

## 16. P0.0 execution record

Date: 2026-10-27.

| Deliverable | Location | State |
| --- | --- | --- |
| Scope freeze, claim, TCB, versioning, exclusion register, decision confirmation | [Semantics Charter](Semantics-Charter.md) | Approved 2026-10-27 |
| Semantics document skeleton | [Semantics.md](Semantics.md) | Approved 2026-10-27 |
| Lean module skeletons | `lean/Stc/Syntax`, `Stc/Grammar`, `Stc/Hir`, `Stc/Sem`, `Stc/Codegen`, `lean/Verification/Semantics` | `lake build` passes |
| Theorem registry | `lean/verification-status.toml` | 24 inventory records, all `planned` |
| Registry validation and axiom audit | `tools/ci/check_verification_registry.py` | CI check active |
| Fail-closed conformance placeholder | `lake exe semantics-conformance` | Exits 3 while the corpus is empty |
| Placeholder check | `tools/ci/check_semantics_placeholder.py` | CI check active |
| No-`sorry`/`admit` and axiom checks | `.github/workflows/ci.yml` (`lean` job) | CI check active |
| Required-document coverage | `tools/ci/check_docs.py` | Includes the Phase 0 plan, charter, and semantics document |

Local evidence:

```text
cd lean && lake build
cd lean && lake exe semantics-conformance            # exit 3: no corpus cases (fail-closed)
python tools/ci/check_verification_registry.py       # 24 records: 24 planned
python tools/ci/check_semantics_placeholder.py       # placeholder fails closed as required
python tools/ci/check_docs.py
```

P0.0 exit status: complete. The technical exit criteria pass, and the reviewer approved the
charter, the exclusion register, and the [Semantics.md](Semantics.md) skeleton on 2026-10-27.

## 17. P0.1 execution record

Date: 2026-10-27.

| Deliverable | Location | State |
| --- | --- | --- |
| Token, trivia, keyword, punctuation, and span model | `lean/Stc/Syntax/Tokens.lean` | Defined; token-span lemmas proved |
| Lossless CST contract and coverage predicates | `lean/Stc/Syntax/Cst.lean` | Defined; coverage and node-span lemmas proved |
| Surface AST for core declarations, expressions, statements, and programs | `lean/Stc/Syntax/Ast.lean` | Defined; `Ast.SpansNested` and `ast_span_wellformed` proved |
| Declarative grammar relation | `lean/Stc/Grammar/Derives.lean` | `Derives : Cst → Ast → Prop` with every core production rule; helpers for lists, options, comma lists, statement lists, and operator chains |
| AST↔CST and source-span mapping | `Stc.Grammar.MapsTo`, `MapsToWithSpan`, `Derives.unit_span_eq`, `Derives.unit_span_wellFormed`, `Derives.unit_span_bounded` | Defined and root lemmas proved |
| Production-to-definition coverage table | [Core-Coverage.md](Core-Coverage.md) | Approved 2026-10-27 |
| Checked derivation smoke test | `lean/Stc/Grammar/Examples.lean` | A complete `PROGRAM` derivation builds |
| Registry entry | `lean/verification-status.toml` (`P0-T01`) | `Stc.Syntax.ast_span_wellformed` proved; no axioms |
| Semantics document updates | [Semantics.md](Semantics.md) §2.1–2.3, §7.1, §9.3 | Updated for P0.1 |

Design note: the plan names `Derives : Grammar → Ast → Prop` or an equivalent indexed relation.
P0.1 uses the CST as the grammar carrier: `Cst` nodes are tagged with production names, so a
separate duplicate grammar tree adds no information. `Derives` matches the significant (trivia-free)
CST and is mapped from a full CST by `MapsTo`; the coverage table is the production index.

Local evidence:

```text
cd lean && lake build                                   # 34 jobs, including Stc.Grammar.Examples
cd lean && lake env lean ...                            # type-checked during development
cd lean && #print axioms Stc.Syntax.ast_span_wellformed # does not depend on any axioms
python tools/ci/check_verification_registry.py          # 24 records: 1 proved, 23 planned
python tools/ci/check_docs.py                           # 22 required documents, Core-Coverage.md included
```

P0.1 exit status: complete. The technical exit criteria pass: the core surface AST and grammar
relation cover every included production in §3.3, the coverage table is approved, and no
parser-correctness claim is made. P0.2 starts against these definitions.

## 18. P0.2 execution record

Date: 2026-10-27.

| Deliverable | Location | State |
| --- | --- | --- |
| Canonical core types, layouts, literals, and variable identifiers | `lean/Stc/Hir/Types.lean` | Defined; `Ty.ofElementary`, `Ty.layout`, `Ty.isCore`, `Literal.WellFormed` |
| Typed HIR syntax for expressions, places, statements, declarations, and units | `lean/Stc/Hir/Core.lean` | Defined; `HirUnit` has globals, functions, and one program |
| Typing contexts, environments, operator and standard-function typing | `lean/Stc/Sem/Typing.lean` | Defined; `Ctx`, `Env`, `ExprHasType`, `CallHasType`, `StdFnHasType` |
| Statement and whole-unit declarative judgment | `lean/Stc/Sem/Typing.lean` | Defined; `StmtHasType`, `HasType` |
| Core conversion relation and characterization | `lean/Stc/Sem/Typing.lean` | `ConversionLegal`, `conversion_legal_iff`, `ImplicitConversion` |
| Algorithmic checker and structured rejection | `lean/Stc/Sem/Typing.lean` | `checkCore : HirUnit → Except Rejection Env`; named `ExprRule`, `StmtRule`, `CaseLabelProblem` |
| Soundness | `Stc.Sem.core_check_sound` | Proved; no non-standard axioms |
| Rejection characterization | `Stc.Sem.core_rejection_characterized` | Proved; `FailsRule` states the named rule failure carried by each rejection |
| HIR well-formedness | `Stc.Sem.hir_wellformed` | Proved; accepted HIR has core declared types |
| Completeness | `Stc.Sem.core_check_complete` | **Descoped** with the reason recorded in the registry: the P0.2 checker deliberately over-rejects `FOR` (`StmtRule.forUnsupported`); completeness is deferred to Phase 2 |
| Semantics document updates | [Semantics.md](Semantics.md) §2.4, §3.1–3.5, §7.1, §9.3 | Updated for P0.2 |
| Registry entries | `lean/verification-status.toml` (`P0-T02`, `P0-T04`, `P0-T05`, `P0-T06`) | Proved; `P0-T03` descoped |

Design notes:

- HIR is the lowered form. Variables are `VarId` indices, callees are resolved, call arguments
  are in formal-parameter order, implicit conversions are explicit `Cast` nodes, and `CASE`
  labels are folded integers. The checker therefore requires exact type equality for
  assignment, argument passing, and initializers. This is a deliberate difference from the Rust
  analyzer, which inserts implicit conversions during type checking; lowering owns that step.
- `CASE` selectors are restricted to integer types, as §3.3 specifies. The Rust analyzer also
  accepts character, bit-string, and enum selectors; the restriction is recorded for P0.6.
- The exact standard functions are `ABS`, `SEL`, `MIN`, `MAX`, `LIMIT`, and `MUX`. Arithmetic,
  bitwise, and comparison standard-function forms are represented by operator nodes. Other
  standard functions are excluded until Phase 6.
- `FOR` is fully defined by `StmtHasType.forLoop`, but the P0.2 checker rejects it with the
  named `StmtRule.forUnsupported` rule. This is the explicit over-rejection that
  `core_rejection_characterized` records in place of completeness. The P0.6 interpreter consumes
  the declarative judgment, not `checkCore`.
- Label validity is characterized by the checker's label-problem function; the success path is
  proved equivalent to `CaseLabelsValid` by `checkCaseLabels_sound`.
- The behavioural agreement between `ConversionLegal` and `Stc.Exact.IntType.convert` (a
  destination value or a `RANGE` trap) is stated with P0.3's
  `checked_arith_traps_exactly`, where the dynamic semantics supplies the values.

Local evidence:

```text
cd lean && lake build                                   # 34 jobs
cd lean && #print axioms Stc.Sem.core_check_sound        # no axioms
cd lean && #print axioms Stc.Sem.core_rejection_characterized
cd lean && #print axioms Stc.Sem.hir_wellformed
cd lean && #print axioms Stc.Sem.conversion_legal_iff
python tools/ci/check_verification_registry.py          # 24 records: 5 proved, 1 descoped, 18 planned
python tools/ci/check_docs.py
```

P0.2 exit status: complete. `core_check_sound` is proved; `core_rejection_characterized` is
proved in place of completeness, with the `FOR` over-rejection recorded; HIR well-formedness is
defined and preserved by the checker; and the conversion relation is characterized against the
analyzer's core conversions. The exact-model behaviour link is carried into P0.3.

## 19. P0.3 execution record — complete, review approved

Date: 2026-10-27. Status: **complete 2026-10-31; review approved 2026-10-31.** The definitions and the
soundness work are in place. The work was paused for alignment with M6.5; the drift register was
rechecked on 2026-10-31 (§19.7), the open questions were resolved in §19.8, and the remaining
proofs are recorded in §19.9–§19.12. There is no `sorry` or `admit`; `lake build` and the CI
checks pass. The completion record and review checklist are in §19.12. The owner approved the
P0.3 review on 2026-10-31.

### 19.1 Deliverables

| Deliverable | Location | State |
| --- | --- | --- |
| Values, traps, and exact operation wrappers | [Values.lean](../../lean/Stc/Sem/Values.lean) | Defined; operation well-formedness proved |
| Small-step machine, configurations, `Step`/`Steps`/`Terminal`, cycle boundary | [Machine.lean](../../lean/Stc/Sem/Machine.lean) | Defined; `initState` models the initialization boundary |
| Configuration typing (`StackTyped`, `ConfigTyped`) and projections | [Eval.lean](../../lean/Stc/Sem/Eval.lean) | Defined; runtime variable-lookup lemma proved |
| `eval_deterministic` | `Stc.Sem.Eval` | Proved |
| `trap_soundness` | `Stc.Sem.Eval` | Proved |
| `checked_arith_traps_exactly` | `Stc.Sem.Eval` | Proved in the guarded form described in Q2 |
| Fuel interpreter `runFuel`/`stcSem` | [Interpreter.lean](../../lean/Verification/Semantics/Interpreter.lean) | Defined; `interpreter_sound` and `interpreter_complete` proved |
| `progress` | `Stc.Sem.Eval` | Proved (P0-T08) |
| `preservation` | `Stc.Sem.Eval` | Proved (P0-T09) |
| `type_soundness` | `Stc.Sem.Eval` | Proved (P0-T10) |
| Initial-state typing (`ConfigTyped u (initState u n)` from `HasType u`) | `Stc.Sem.Eval` | Proved (`initState_typed`, `initBody_typed`) |
| Source-semantics sections of [Semantics.md](Semantics.md) | — | Complete for P0.3; completion record in §19.12 |

### 19.2 Exit criteria assessment

The P0.3 exit criteria are **met**. `progress`, `preservation`, initial-state typing, and
`type_soundness` are proved; there is no `sorry` or `admit`; `lake build` and the local CI checks
pass. The completion record and review checklist are in §19.12. The proof work is committed on
WL-3, and owner review approval was recorded on 2026-10-31.

### 19.3 Open questions

These questions were deliberately open. They are resolved in §19.8; the original statements are
kept for the record.

- **Q1 — Completion path.** Finish the current machine and `ConfigTyped` proofs, redesign the
  configuration invariant to make preservation smaller, or descope `progress`, `preservation`, and
  `type_soundness` with a written justification. The current definitions assume the first path.
- **Q2 — Exact-model link.** `evalCast` and `evalStd` pass their results through the exact checked
  wrapper, and `checked_arith_traps_exactly` is stated as the exact model followed by that guard.
  The alternative is to prove that `Stc.Exact.IntType.convert` never succeeds out of range (and that
  standard-function results fit), remove the guards, and state plain equality with `Stc.Exact`. The
  guarded form is weaker than the conversion-half promise in §3.4 of the semantics document.
- **Q3 — Character and `UCHAR` validity.** [UTF8-Strings.md](../UTF8-Strings.md) requires
  `WCHAR_TO_UCHAR` to trap on lone surrogates and `UCHAR_TO_WCHAR` to trap above the BMP, and defines
  `UCHAR` as a Unicode scalar. The current value model is layout-only: `Literal.WellFormed` checks
  only that the payload fits 32 bits, and character conversions are integer layout conversions. The
  choice is to model scalar validity and the conversion traps now, or to record the layout-only model
  as a known gap for P0.6 dump validation and later string phases.
- **Q4 — `FOR` runtime zero step.** [stmt.rs](../../crates/stc-codegen/src/stmt.rs) evaluates the
  start, stop, and step expressions once; for signed control variables it uses the ascending bound
  test only when `step > 0`, otherwise the descending test; unsigned loops always ascend. The current
  machine uses the ascending test for `step ≥ 0`, so a non-constant step that evaluates to zero
  diverges from emitted code. Constant zero steps are rejected by typing.
- **Q5 — Initialization and cycle boundary.** Confirm the current model: all global initializers,
  then program-field initializers, then the first cycle body, each in declaration order from zero
  defaults; initializers may call functions and read other roots, with later roots reading zero;
  `RETURN` in a program body ends the cycle; `cyclesLeft` runs the requested number of cycles with
  program-field state persistent. [unit.rs](../../crates/stc-codegen/src/unit.rs) emits global
  initializers before instance initializers in cold mode, which matches this order.
- **Q6 — Negative signed exponent.** The runtime power helper traps `RANGE` for a negative signed
  exponent, while constant evaluation rejects the expression at compile time. Confirm `RANGE` as the
  dynamic trap code rather than `OVERFLOW` or a compile-time-only rejection.
- **Q7 — Checkpoint policy.** Whether to commit the current compiling work as a work-in-progress
  checkpoint or leave it uncommitted until P0.3 completes.

Design decisions already taken and to document at completion (not open questions): the machine
stores no typing annotations, and `StackTyped` types the AST carried by each continuation; `setVar`
is total and an out-of-range write is a no-op unreachable from typed programs; conversions and
standard functions pass results through the exact checked wrapper; a malformed standard-function
call returns the default `BOOL` value and is unreachable from typed calls.

### 19.4 Drift register

Checked against the tree on 2026-10-27 and rechecked on 2026-10-31 after M6.5 stabilized. These are
recorded, not reconciled; the P0.3 alignment pass resolves them when that phase resumes.

| Area | Evidence | Current semantics | Resolution |
| --- | --- | --- | --- |
| `FOR` step evaluation and zero step | `lower_for` in `crates/stc-codegen/src/stmt.rs` | Start/stop/step once; signed loops use the ascending test only when `step > 0`, unsigned loops always ascend. The machine was corrected on 2026-10-31 to match. | Q4 resolved |
| Character conversions | `specification/UTF8-Strings.md` §3.1 and the conversion table | Layout-only conversion; runs of surrogates preserved | Q3 |
| Power trap code | `ensure_pow_helper` in `crates/stc-codegen/src/expr.rs`; `consteval.rs` | `RANGE` for a negative signed exponent | Q6 |
| Initialization order | `emit_global_initializers`, `emit_instance_initializers` in `crates/stc-codegen/src/unit.rs` | Globals then program fields, declaration order, once | Q5 |

### 19.5 Resumption plan

P0.3 resumed on 2026-10-31 after the P0.4 review and CI. The drift register was rechecked (§19.7)
and the open questions are resolved (§19.8). The remaining work:

1. `progress` and the supporting no-stuck lemmas (runtime lookup, loop-marker existence, frame
   completion, call performance), using `ConfigTyped.lookupVar?_some`, the `StackTyped`
   projections, and the `FramesShape` lemmas already present. **Done 2026-10-31.**
2. `preservation` and the store-update lemmas for `FrameMatches`/`GlobalsTyped`, including the
   callee-frame construction, loop markers, and the cycle reset.
3. Initial-state typing from `HasType`, including context agreement between `Ctx.ofGlobals` and
   `Ctx.ofProgram` for global initializers.
4. `type_soundness`, then the registry and semantics-document updates, and the P0.3 review.

This sequence is complete as of 2026-10-31; see §19.11 (preservation) and §19.12 (initial-state
typing and type soundness). The definitions in `Stc.Sem.Values`, `Stc.Sem.Machine`, and
`Stc.Sem.Eval` are the basis; the machine and typing design follow Q1 with no redesign.

### 19.6 Local evidence

```text
cd lean && lake build
python tools/ci/check_verification_registry.py
python tools/ci/check_docs.py
python tools/ci/check_semantics_placeholder.py
grep -rnwE 'sorry|admit' lean/Stc/Sem lean/Verification    # no matches
```

The registry marks the eight proved P0.3 theorems as `proved`: determinism, progress,
preservation, type soundness, trap soundness, guarded exact arithmetic, and the interpreter
soundness and completeness results. `type_soundness` is P0-T10; the completion record is in
§19.12.

### 19.7 Drift recheck (2026-10-31)

The compiler drift that paused P0.3 has stabilized in `stc` 1.0.0. The four §19.4 rows were
rechecked against the released tree:

| Area | Recheck result |
| --- | --- |
| `FOR` zero step | Unchanged: `lower_for` uses `sgt step, 0` for the ascending test and the descending test for zero; the Lean machine still uses `step ≥ 0`. Q4 remains open. |
| Character conversions | Unchanged: [UTF8-Strings.md](../UTF8-Strings.md) still requires scalar validity and conversion traps, and the P0.3 value model remains layout-only. Q3 remains open. |
| Power trap code | Unchanged: `ensure_pow_helper` traps `RANGE` for a negative signed exponent. Q6 remains open. |
| Initialization order | Unchanged: global initializers then program-field initializers, declaration order, once. Q5 remains as recorded. |

No new drift was found in the files the register covers.

### 19.8 Reevaluation (2026-10-31)

P0.3 resumed after the P0.4 review and CI. The open questions are resolved as follows:

| Question | Resolution |
| --- | --- |
| Q1 — completion path | Keep the current machine and `ConfigTyped` invariant and finish the proofs. Redesign only if preservation blocks; no descoping. |
| Q2 — exact-model link | Keep the guarded `evalCast`/`evalStd` form for P0.3. The unguarded equality needs `convert_ok_fits` and standard-function fit lemmas; record it as a follow-up for P0.5/Phase 2. The semantics document states the guarded form. |
| Q3 — character validity | Keep the layout-only character model in P0.3 and record Unicode scalar validity and the `WCHAR_TO_UCHAR`/`UCHAR_TO_WCHAR` traps as a known gap for the later string phase and P0.6 dump validation. |
| Q4 — `FOR` zero step | Fixed in `Machine.forContinue`: signed loops use the ascending test only when `step > 0`; unsigned loops always ascend. This matches `lower_for`. |
| Q5 — initialization boundary | Confirmed: global initializers, then program-field and local initializers, then the cycle body; `RETURN` ends the cycle and `cyclesLeft` persists program state. |
| Q6 — negative signed exponent | Confirmed and implemented: `evalPow` traps `RANGE` for a signed negative exponent; constant evaluation rejects the constant case. |
| Q7 — checkpoint policy | Commit the compiling work as WIP checkpoints on WL-3; the P0.4 commit is the first checkpoint. |

`progress` is done. The remaining work is the rest of the §19.5 sequence: `preservation` (with
the value/statement/resume/returned transitions), initial-state typing, then `type_soundness`,
the registry and semantics document updates, and the P0.3 review.

### 19.9 Preservation checkpoint (2026-10-31)

Preservation exposed five invariant gaps, now fixed:

- `StackTyped.seq` carries `StmtListHasType` for the remaining statements, so
  resuming a statement list reconstructs a typed `.stmts` focus.
- `performCall` records a statement caller's expectation as `unit` (and an
  expression caller's as the callee result type), matching the caller's
  `StackTyped` expectation.
- The assignment branch of `stepValue` pops the assignment continuation before
  writing; previously the stack retained a value-consuming head and the next
  `resume` was stuck.
- `StackTyped.forInit` and `StackTyped.forStop` require the stack below them to
  expect a unit; the machine leaves a unit-expecting stack below the `FOR`
  continuations, so the old value expectation was wrong.
- `ConfigTyped.headLoop` and the `FrameTyped` loop field record that a positive
  loop depth has a loop marker in the frame's stack. Choosing an `IF`/`CASE`/loop
  body after an expression step reconstructs a typed `.stmts` focus from it.

The store/frame lemmas (`ConfigTyped.setVar`, the `FrameMatches` updates,
`GlobalsTyped.setGlobal`), the typed variable lookup, `callArgsPush`, the
`ConfigTyped` transition helpers (`updateFocus`, `updateHeadFocus`,
`updateHeadFrame`, `pushFrame`), the initializer-typing lemmas, the callee-frame
construction, and `stepExpr_preserves` are proved. Preservation itself remains
open: the value/statement/resume/returned transition cases (including the
completed-argument `performCall` frame construction), initial-state typing, and
`type_soundness` remain, followed by the review. `preservation` stays planned
(P0-T09). No `sorry` remains in the tree.

### 19.10 Caller-expectation checkpoint (2026-10-31)

`completeFrame` preservation exposed a missing configuration invariant: the
caller's `expect` was not linked to the callee's declared result, so the
`.returned` focus could not be typed when the callee returned. The fix adds:

- `CallerExpects u f g` and `FramesCallerExpects u frames`, recording that every
  suspended caller's `expect` is `none` (statement call) or the callee's declared
  result type. The link uses the declaration, not the callee's current result
  value, so result-variable writes preserve it.
- `ConfigTyped.callers`, with corresponding proofs in `ConfigTyped.setVar`,
  `updateHeadFocus`, `updateHeadFrame`, `updateFocus`, and `pushFrame`.
- `FrameMatches.result_cases` and `FocusTyped.returned_of_caller`, which derive
  the `.returned` focus typing from the link.
- `ConfigTyped.updateCycles` and `completeFrame_preserves`; frame completion is
  now proved for both function return and program cycle reset.

The same checkpoint adds the loop-marker tail condition to
`StackTyped.whileLoop`, `repeatBody`, and `forBody`, so a completed loop can
reconstruct a typed resume or expression focus when an outer loop remains.
`StackHasLoop` moves before `StackTyped` so the rule can refer to it.

Remaining: `stepValue_preserves` (including the completed-argument
`performCall`), `stepStmt`/`stepStmts` preservation, `stepResume_preserves`
(including `forAdvance`/`forContinue`), `stepReturned_preserves`, assembling
`preservation`, initial-state typing, and `type_soundness`. `preservation` and
`type_soundness` remain planned.

### 19.11 Preservation checkpoint (2026-10-31)

`preservation` is proved (`P0-T09`). The checkpoint adds:

- `stepValue_preserves`, with per-continuation helpers for assignment,
  unary/binary/cast, `IF`/`CASE`, `WHILE`/`REPEAT`, `FOR` start/bound/step,
  and call arguments. Completed-argument calls go through `performCall`
  (standard and user functions) and `CalleeSignatureV.push`.
- `stepStmt_preserves` and `stepStmts_preserves` for every statement form,
  including `EXIT`/`CONTINUE`, and a statement-call `callArgsPushStmt`.
- `stepResume_preserves` and `stepReturned_preserves`.
- `forContinue_preserves` and `forAdvance_preserves` for the `FOR` increment
  path, including the control-variable write, and `MarkerTyped` /
  `StackTyped.findLoop_marker` for the innermost loop marker.

Proving preservation exposed three more machine bugs, now fixed:

- The `.callArgs` continuation re-focused the argument it had just consumed
  instead of the following one, so multi-argument calls would re-evaluate the
  first argument and never evaluate the rest. It now matches
  `_ :: next :: more` and carries `next :: more`, matching the
  `StackTyped.callArgs` invariant.
- `EXIT` kept the loop body's loop context after dropping the marker; it now
  resumes in the marker's outer context via `findLoop`. The now-unused
  `dropToLoop` was removed.
- `CONTINUE` on a `FOR` marker entered `forAdvance` in the body's loop
  context; it now enters in the marker's outer context, matching the typing of
  the stack below the marker.

### 19.12 P0.3 completion and review checklist (2026-10-31)

Initial-state typing and `type_soundness` are proved. The missing piece was
context agreement for global initializers: `GlobalTyped` checks a global
initializer in `Ctx.ofGlobals env`, while the initialization statements run in
the program frame's `Ctx.ofProgram env u.program`. `ExprHasType.transport`
proves that contexts agreeing on variable lookups and the function table type
the same expressions, by structural recursion on the expression using the
mutual recursor `ExprHasType.rec` with one motive per mutually defined
relation (an `induction` on the derivation is rejected because the judgment is
mutually inductive, and mutual-recursion termination is unnecessary once the
recursor supplies the induction hypotheses). The `ofGlobals`/`ofProgram` case
needs only the global and function tables: a source param/local/result lookup
is impossible because the globals context binds none of them.

The remaining pieces are:

- `initBody_typed`: `globalInitStmtsFrom_typed` and
  `localInitStmtsFrom_typed` in the program context, plus
  `StmtListHasType.append`.
- `initState_typed`: `GlobalsTyped` and `FrameMatches` from
  `Value.default_wellFormed` and `HirWellFormed`, with the cycle-zero halt
  focus and the first-cycle statement focus.
- `type_soundness`: initial-state typing composed with `preservation` along
  `Steps`, and `progress` showing that no reachable configuration is stuck.

**Review checklist (approved 2026-10-31).** The reviewer confirmed:

1. `ExprHasType.transport` is stated for arbitrary lookup- and
   function-table-agreeing contexts (not only the initializer pair), and its
   recursor motives cover all four mutually defined relations
   (`ExprHasType`, `CallHasType`, `StdFnHasType`, `Forall2`).
2. `initState_typed` covers both the zero-cycle (`.halted`) and positive-cycle
   (`.stmts (initBody u) LoopCtx.top`) focuses, and `initBody_typed` covers
   global initializers, program-field initializers, and the first cycle body
   in that order.
3. `type_soundness` states and proves exactly the P0.3 intent: the initial
   configuration is typed, every reachable configuration is typed, and no
   reachable configuration is stuck; trap documentation remains
   `trap_soundness`.
4. The registry entries P0-T09/P0-T10 are `proved`, the audit passes, and the
   status rows in this plan, [Semantics.md](Semantics.md), [Plan.md](Plan.md),
   and [V2.0.0-PLAN.md](../../V2.0.0-PLAN.md) agree.
5. The three machine fixes recorded in §19.11 (call-argument advance, `EXIT`
   loop context, `CONTINUE`-on-`FOR` context) match the intended source
   semantics; any objection is a semantics change, not a proof change.

Verification commands:

```bash
cd lean && lake build
python tools/ci/check_verification_registry.py   # 45 records: 34 proved, 10 planned, 1 descoped
python tools/ci/check_docs.py
python tools/ci/check_semantics_placeholder.py
grep -rnwE 'sorry|admit' lean/Stc/Sem               # no matches
```

**Review approved by the owner on 2026-10-31.** The P0.3 exit criteria are met and the sign-off
is recorded. P0.3 is complete; the next Phase 0 work is P0.5–P0.7 outside the v2.0.0 WL-3 scope.

## 20. P0.4 execution record — complete

Date: 2026-10-31. Status: **complete; review approved 2026-10-31.** The scope decisions were
taken before implementation:

| Decision | Resolution |
| --- | --- |
| Module boundary | `IRFrag`/`WellFormed` cover the whole emitted production module, including generated entry-point wrappers and module globals. P0.5 interprets user functions and the cycle body; attach/prepare/shutdown remain the initialization boundary. |
| Cell kinds | All five GOT kinds: `Data`, `Function`, `Descriptor`, `Direct`, `Import`. `Descriptor` cells are structurally representable but semantically out of the core. |
| Slot canonicality | The slot table is grouped in the canonical kind order in P0.4. Identity sorting by `GotId`/service ID remains Phase 3's `slots_canonical` and `slot_table_valid_iff_canonical`. |
| Verifier alignment | `WellFormed` includes every `stc-codegen` structural rule that applies to the core fragment: type/SSA checks intrinsic to Lean, plus the contract rules for roots/callables/local globals, slot kinds, entry points, pre-attach accessors, cell writes, and descriptor/function-table shape. Content pairing is Phase 5. |
| `switch` | Removed from the P0.4 fragment: the current compiler lowers `CASE` to branches. Phase 5 adds `switch` if lowering changes. |
| Fragment coverage table | [Core-IR-Coverage.md](Core-IR-Coverage.md). |

The deliverables and lemmas are in §6 (P0.4) and §7.1. Registry records P0-T25–P0-T45 are
`proved`; `lake build` passes and the registry audit reports only the standard Lean axioms. The
definitions are in [`Stc.Codegen.Ir`](../../lean/Stc/Codegen/Ir.lean); the smoke inhabitant
`minimalFrag` proves `WellFormed` is satisfiable. The P0.4 review was approved on 2026-10-31 with
no definition changes requested.

Local evidence:

```text
cd lean && lake build
python tools/ci/check_verification_registry.py   # 45 records: 31 proved, 13 planned, 1 descoped
python tools/ci/check_docs.py
grep -rnwE 'sorry|admit' lean/Stc/Codegen lean/Verification    # no matches
```

### 20.1 Review record

The P0.4 review was approved by the owner on 2026-10-31 with no definition changes requested. The
review confirmed:

- the five cell kinds and the grouped slot-table rule match [GOT Artifacts §7.1](../Architecture/GOT-Artifacts.md) and [Online Change and GOT §3.2](../Architecture/Online-Change-and-GOT.md);
- the `WellFormed` rule mapping in [Core-IR-Coverage §7](Core-IR-Coverage.md#7-well-formedness-rules) covers `crates/stc-codegen/src/verify.rs`, including the intentional differences (Lean checks type correctness, SSA/dominance, and `phi` shape intrinsically; Rust relies on builder facts; identity-sorted slot canonicality is Phase 3; descriptor/function-table content pairing is Phase 5);
- the fragment boundary is the whole production module, with debug/test builds, coverage counters, checkpoint hooks, `switch`, aggregates, OOP, and the process image excluded;
- the smoke inhabitant `minimalFrag` is an adequate non-vacuity check for `WellFormed`;
- any definition change from the review is reflected in the registry and this record.
