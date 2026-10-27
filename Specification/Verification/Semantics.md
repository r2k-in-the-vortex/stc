# stc Semantics — Phase 0 Core

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-10-27
Semantics version: `1.0-draft` (frozen at `1.0` in P0.7)
HIR dump format version: `0.1-draft` (frozen with the core dump in P0.6)
Status: Skeleton approved 2026-10-27. Definitions and proofs are added in P0.1–P0.5 and reviewed
in P0.7. Nothing in this document is a current verification claim.
Charter: [Semantics Charter](Semantics-Charter.md)
Plan: [Phase 0 Implementation Plan](Plan-Phase0.md) · [Verification Program Plan](Plan.md)

## 1. Scope, core subset, and exclusions

The semantics covers the frozen core in [Plan-Phase0.md](Plan-Phase0.md) §3.3 and §3.4. The
source subset is scalar: `BOOL`, signed and unsigned integers, bit strings, and character types;
scalar variables, globals, functions, and one program; exact operators and standard functions;
structured control flow without `VAR_OUTPUT`, `VAR_IN_OUT`, or aliasing; and the `OVERFLOW`,
`DIV_ZERO`, and `RANGE` traps. The target subset is the emitted-IR fragment for that source
subset: integer and pointer types, integer and bitwise instructions, casts, `select`, `phi`,
branches, `switch`, calls, stack slots, loads and stores, slot-table `getelementptr`, checked
arithmetic intrinsics, and trap calls, with GOT cells modeled as typed indices.

The exclusions and the phase that adds each one are recorded in the
[charter's exclusion register](Semantics-Charter.md#6-frozen-core-and-exclusion-register).
Excluded constructs are rejected by the reference tools and never interpreted silently.

## 2. Grammar, AST, HIR, and IR mapping

### 2.1 Tokens, trivia, spans, and the CST contract

P0.1 defines the token and trivia model, span coverage, and the lossless CST contract in
`Stc.Syntax.Tokens` and `Stc.Syntax.Cst`, following
[Tokenization](../Syntax/Tokenization.md). The contract states that token and trivia spans cover
the source exactly, in order, with no gaps or overlaps.

### 2.2 Surface AST and the `Derives` relation

P0.1 defines the surface AST in `Stc.Syntax.Ast` and the declarative relation
`Derives : Grammar → Ast → Prop` in `Stc.Grammar.Derives`, mirroring the core productions in
[Grammar](../Syntax/Grammar.md). Every core production receives a constructor or relation rule;
the P0.1 coverage table maps productions to Lean rules.

### 2.3 AST-to-CST and source-span mapping

P0.1 defines the AST↔CST mapping and the source-span mapping used by Phase 4's parser
contracts. `ast_span_wellformed` is the structural result for the AST side.

### 2.4 Typed HIR

P0.2 defines the typed HIR in `Stc.Hir.Types` and `Stc.Hir.Core`: types, places, expressions,
statements, declarations, and programs. It aligns with the Rust `stc-hir` structures and the
typed views in [AST, CST, and HIR](../AST/AST.md), but the Lean family is the semantic
reference.

### 2.5 HIR-to-IR mapping and the covered fragment

P0.4 defines `IRFrag` and `WellFormed` in `Stc.Codegen.Ir`; P0.5 defines the operational
semantics in `Stc.Codegen.Semantics`. The mapping covers the emitted IR for the core source
subset only. The textual printer and parser are out of scope, so the semantics is defined over
the in-memory fragment.

## 3. Static semantics

### 3.1 Types, values, and widths

P0.2 defines the core type and value families and their widths. `BOOL` is a one-bit unsigned
layout, an `i8` in memory and an `i1` normalized with `icmp ne 0` in SSA. Integer and bit-string
operations reuse the existing `Stc.Exact` models, including checked overflow, truncating
division, masked shift and rotate counts, and conversion rules. `CHAR`, `WCHAR`, and `UCHAR` are
unsigned integer layouts; `UCHAR` is a 32-bit Unicode scalar.

### 3.2 Declarative typing judgment `HasType`

P0.2 defines `HasType` for expressions, statements, places, declarations, and programs,
including assignment, conditions, calls, conversions, and the exact standard functions. The
judgment is intentionally small and is reviewed against the specification rather than inferred
from the Rust analyzer.

### 3.3 Core checker `checkCore`

P0.2 defines the algorithmic checker `checkCore` and proves `core_check_sound`. It proves
`core_check_complete` where the checker is intended to be complete, and
`core_rejection_characterized` for the over-rejection that remains deliberate. Known differences
from the Rust analyzer's diagnostics are recorded with a reason.

### 3.4 Conversions

P0.2 defines the core conversion relation and proves `conversion_legal_iff`: the relation
accepts exactly the conversions specified for the included types, using the `Stc.Exact`
conversion model.

### 3.5 Well-formedness

P0.2 defines `hir_wellformed` and proves that accepted HIR satisfies it. Target well-formedness
`WellFormed` is defined in P0.4 and is the precondition for the target soundness results.

## 4. Dynamic semantics

### 4.1 Configurations and the source control stack

P0.3 defines a configuration as the expression or statement being evaluated, a local store, a
global root map, a call stack, and a structured control stack. Terminal states are values,
halts, and traps. `EXIT` and `CONTINUE` target the innermost enclosing loop through the control
stack; `RETURN` pops a call frame.

### 4.2 Source small-step relation `Step`

P0.3 defines `Step` over configurations. The relation is the declarative reference for source
soundness. It does not require termination; functions may be recursive.

### 4.3 Recursive reference model `stcSem`

P0.3 defines the fuel-based executable model `stcSem` over the same configuration type. The
interpreter threads explicit state and encodes control flow as an explicit result
(`continue`/`break`/`return`/`trap`), not as a host exception. `interpreter_sound` and
`interpreter_complete` relate it to the reflexive-transitive closure of `Step`.

### 4.4 Target small-step relation `IRStep`

P0.5 defines `IRStep` for the covered instructions, including checked arithmetic, trap calls,
loads and stores through GOT cells, and direct and indirect calls. `IRState` records the program
counter, stack frame, local memory, GOT root map, slot table, function cells, call stack, and
trap state.

### 4.5 Recursive target executor `irExecRec`

P0.5 defines the recursive executor `irExecRec` and proves `ir_recursive_model_equiv` against
the reflexive-transitive closure of `IRStep`. The recursive model is the preferred Phase 1
refinement target; the step relation remains the reference for progress and memory soundness.

### 4.6 Cycle and initialization boundary

The source semantics covers one program and one sequential cycle. The target semantics covers
the ABI 2 entry points and the cycle body; attach, prepare, activate, commit, rollback, and
unload beyond the initialization boundary are Phase 8. The initialization boundary and its
trusted assumptions are documented with the target semantics.

## 5. Values, memory, GOT, traps, and undefined behavior

### 5.1 Values and memory model

The core has stack frames and persistent GOT cells. There is no heap, no general pointer
arithmetic, and no aliasing beyond GOT cells. Target values are integers, pointers, and the
typed cell contents described by `WellFormed`.

### 5.2 GOT root and slot model

A GOT cell is an index, not an arbitrary pointer. The target state carries a root map
`GotId → Value`, a slot table `Slot → CellKind`, and a function table `GotId → Callable`.
`WellFormed` records each cell's kind, and the semantics checks kind agreement on every load,
store, and call. `ir_got_sound` states that a load returns the root value recorded by the slot
table.

### 5.3 Traps

Traps are explicit terminal states with a trap code and a source span. The core trap set is
`OVERFLOW`, `DIV_ZERO`, and `RANGE`. `trap_soundness` and `ir_trap_sound` state that every
terminal trap is in the specified set and occurs only on the specified path.

### 5.4 Undefined-behavior policy

The covered fragment excludes poison and `undef`, out-of-bounds stack access, misaligned access,
and raw pointer arithmetic. `ir_no_ub` states that `WellFormed` IR cannot reach an undefined
state on the fragment. This is a property of the modeled fragment, not a general LLVM result.

## 6. Exact arithmetic connection

The source and target integer operations are defined against the existing `Stc.Exact` models.
`checked_arith_traps_exactly` states that source checked arithmetic traps exactly on overflow or
division by zero. `ir_checked_arith_refines_exact` states that the IR instruction sequence plus
its trap path produces the same result or trap as the exact model. This bridge keeps the
semantics and the proved exact functions on one definition of the scalar kernel.

## 7. Theorem inventory and proof status

The authoritative status of every entry is [`lean/verification-status.toml`](../../lean/verification-status.toml);
`tools/ci/check_verification_registry.py` validates it and audits proved theorems. This section
mirrors the registry at P0.0 and is regenerated from it at P0.7. All entries are `planned`.

### 7.1 Source soundness

| Theorem | Module | Sub-phase | Status |
| --- | --- | --- | --- |
| `Stc.Syntax.ast_span_wellformed` | `Stc.Syntax.Ast` | P0.1 | planned |
| `Stc.Sem.core_check_sound` | `Stc.Sem.Typing` | P0.2 | planned |
| `Stc.Sem.core_check_complete` | `Stc.Sem.Typing` | P0.2 | planned |
| `Stc.Sem.core_rejection_characterized` | `Stc.Sem.Typing` | P0.2 | planned |
| `Stc.Sem.hir_wellformed` | `Stc.Sem.Typing` | P0.2 | planned |
| `Stc.Sem.conversion_legal_iff` | `Stc.Sem.Typing` | P0.2 | planned |
| `Stc.Sem.eval_deterministic` | `Stc.Sem.Eval` | P0.3 | planned |
| `Stc.Sem.progress` | `Stc.Sem.Eval` | P0.3 | planned |
| `Stc.Sem.preservation` | `Stc.Sem.Eval` | P0.3 | planned |
| `Stc.Sem.type_soundness` | `Stc.Sem.Eval` | P0.3 | planned |
| `Stc.Sem.trap_soundness` | `Stc.Sem.Eval` | P0.3 | planned |
| `Stc.Sem.checked_arith_traps_exactly` | `Stc.Sem.Eval` | P0.3 | planned |

### 7.2 Target soundness

| Theorem | Module | Sub-phase | Status |
| --- | --- | --- | --- |
| `Stc.Codegen.ir_deterministic` | `Stc.Codegen.Semantics` | P0.5 | planned |
| `Stc.Codegen.ir_progress` | `Stc.Codegen.Semantics` | P0.5 | planned |
| `Stc.Codegen.ir_memory_safe` | `Stc.Codegen.Semantics` | P0.5 | planned |
| `Stc.Codegen.ir_got_sound` | `Stc.Codegen.Semantics` | P0.5 | planned |
| `Stc.Codegen.ir_trap_sound` | `Stc.Codegen.Semantics` | P0.5 | planned |
| `Stc.Codegen.ir_checked_arith_refines_exact` | `Stc.Codegen.Semantics` | P0.5 | planned |
| `Stc.Codegen.ir_recursive_model_equiv` | `Stc.Codegen.Semantics` | P0.5 | planned |
| `Stc.Codegen.ir_no_ub` | `Stc.Codegen.Semantics` | P0.5 | planned |

### 7.3 Interpreter equivalence and harness properties

| Theorem | Module | Sub-phase | Status |
| --- | --- | --- | --- |
| `Verification.Semantics.interpreter_sound` | `Verification.Semantics.Interpreter` | P0.3 | planned |
| `Verification.Semantics.interpreter_complete` | `Verification.Semantics.Interpreter` | P0.3 | planned |
| `Verification.Semantics.semantics_versioned` | `Verification.Semantics.Conformance` | P0.6 | planned |

### 7.4 Conformance evidence

| Evidence | Artifact | Sub-phase | Status |
| --- | --- | --- | --- |
| `Verification.Semantics.semantics_conformance` | `lake exe semantics-conformance` | P0.6 | planned |

The conformance result is evidence, not a theorem. The report must not describe it as a proof
that the Rust compiler refines the semantics.

## 8. Trusted computing base and assumptions

The trusted base is recorded in the [charter](Semantics-Charter.md#3-trusted-computing-base).
In full: the Lean 4.35.0 kernel and standard library; the definitions in this repository;
Batteries for executable models and Mathlib only in proof libraries if P0-D5 admits it; the
versioned HIR dump, case descriptors, and report schema as harness inputs; and the Rust compiler,
clang/LLVM, operating system, and hardware as the conformance oracle. No proof in Phase 0
constrains the Rust compiler. The printer/parser boundary, LLVM, and the host runtime outside
the modeled initialization boundary remain outside the semantics.

Assumptions that must be stated with any published claim:

- the compiled program's observable behavior is represented faithfully by its HIR dump;
- the case descriptor describes the intended inputs and cycle count;
- no excluded construct reaches the reference tools; unknown versions and unmodeled features
  are rejected rather than interpreted;
- the conformance corpus covers every core feature and trap it claims to cover.

## 9. Version and changelog

### 9.1 Semantics version

`semantics_version` is `1.0-draft` during Phase 0 and becomes `1.0` at the P0.7 freeze. The
freeze commit is recorded in the registry and `Phase0-Report.md`. A change to any core definition
or theorem after the freeze requires a version bump and a registry update.

### 9.2 HIR dump format version

The dump format is `0.1-draft` until P0.6 defines and validates the core schema; P0.6 freezes it
for the core subset and every report records it. A dump with an unknown version or an unmodeled
construct is rejected.

### 9.3 Changelog

| Date | Version | Phase | Change |
| --- | --- | --- | --- |
| 2026-10-27 | 1.0-draft | P0.0 | Created the skeleton; froze the core scope and exclusions in the charter |

## 10. Rust/Lean interface

### 10.1 HIR dump

The reference interpreter consumes a stable, versioned JSON dump of the core HIR, not source
text. P0.6 defines the schema. It records the dump-format version and the semantics version; the
core module with functions, programs, globals, and types; signatures and parameter/local
declarations; statements and expressions with resolved places, explicit casts, call targets, and
constant values; GOT root keys and scalar program fields; source spans for traps; and a feature
tag for every construct outside the core. It exposes no Rust `DefId`s, arena indices, or debug
text as stable identity. Unknown versions and unmodeled constructs are rejected, not
interpreted.

### 10.2 Reference interpreter

`lake exe stc-sem` reads a dump and a case descriptor, runs the core evaluator, and writes a
machine-readable result: values, traps, global snapshot, cycle count, and an optional trace. It
is a verification tool, not a production interpreter; performance is not a goal. P0.6 builds it
on the recursive model in §4.3.

### 10.3 Conformance harness

`lake exe semantics-conformance` orchestrates the comparison: it builds the dump and a runnable
artifact with the Rust compiler, runs `stc-sem`, runs the compiled artifact or test host,
compares values, traps, and global snapshots, and emits a report keyed by case and feature with
the semantics and dump-format versions. The corpus starts from the existing conformance and
execution cases that fit the core, plus targeted cases for every core type, operator, statement,
conversion, trap, and GOT access. Until the corpus exists, the P0.0 placeholder exits nonzero so
an empty run can never be reported as agreement.

### 10.4 Disagreement triage

Every disagreement is classified as a Lean semantics bug, a Rust compiler bug, a dump bug, or an
unmodeled or excluded feature. The case is fixed or excluded with a written reason; a silent
exclusion is a Phase 0 exit failure. The triage log is part of `Phase0-Report.md`.

## 11. References

- [Semantics Charter](Semantics-Charter.md)
- [Phase 0 Implementation Plan](Plan-Phase0.md)
- [Verification Program Plan](Plan.md)
- [Testing and Verification](../Architecture/Testing-and-Verification.md)
- [Architecture](../Architecture/Architecture.md)
- [AST, CST, and HIR](../AST/AST.md)
- [Grammar](../Syntax/Grammar.md)
- [Tokenization](../Syntax/Tokenization.md)
- [GOT Artifacts and Schemas](../Architecture/GOT-Artifacts.md)
- [Runtime and Execution](../Architecture/Runtime-and-Execution.md)
- [UTF-8 Strings](../UTF8-Strings.md)
- [Decisions](../Decisions.md)
