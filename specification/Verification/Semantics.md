# stc Semantics — Phase 0 Core

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-10-27
Semantics version: `1.0-draft` (frozen at `1.0` in P0.7)
HIR dump format version: `0.1-draft` (frozen with the core dump in P0.6)
Status: Skeleton approved 2026-10-27. P0.1 is complete (review approved 2026-10-27): the token
model, lossless CST contract, surface AST, and `Derives` relation are defined, and
`ast_span_wellformed` is proved. P0.2 is complete (2026-10-27): the typed HIR, the declarative
typing judgment, the core checker `checkCore`, the conversion relation, HIR well-formedness, and
the soundness and rejection-characterization theorems are defined and proved. P0.3 is in progress
(2026-10-27): the value and trap model, the small-step machine, the configuration typing, the
determinism, trap-soundness, and guarded exact-arithmetic results, and the fuel interpreter's
soundness and completeness are in place; `progress` is proved, while `preservation`,
`type_soundness`, and initial-state typing remain. The M6.5 compiler drift that paused P0.3 has
stabilized at `stc`
1.0.0; the drift register was rechecked on 2026-10-31 and still holds. P0.3 resumed on 2026-10-31
and its open questions are resolved in [Plan-Phase0.md](Plan-Phase0.md) §19.8. P0.4 is complete
(review approved 2026-10-31, WL-3): `IRFrag`, `IRContract`, `WellFormed`, and the structural
lemmas are defined and proved. P0.5 definitions and proofs follow and are reviewed in P0.7. The open P0.3
semantic questions and the drift register are recorded in [Plan-Phase0.md](Plan-Phase0.md) §19;
the P0.4 scope decisions are in §20. This document does not yet make the full Phase 0 soundness
claim.
Charter: [Semantics Charter](Semantics-Charter.md)
Plan: [Phase 0 Implementation Plan](Plan-Phase0.md) · [Verification Program Plan](Plan.md)

## 1. Scope, core subset, and exclusions

The semantics covers the frozen core in [Plan-Phase0.md](Plan-Phase0.md) §3.3 and §3.4. The
source subset is scalar: `BOOL`, signed and unsigned integers, bit strings, and character types;
scalar variables, globals, functions, and one program; exact operators and standard functions;
structured control flow without `VAR_OUTPUT`, `VAR_IN_OUT`, or aliasing; and the `OVERFLOW`,
`DIV_ZERO`, and `RANGE` traps. The target subset is the emitted-IR fragment for that source
subset: integer and pointer types, the overflow-intrinsic result and fixed arrays, integer and
bitwise instructions including the rotate intrinsics, casts, `select`, `phi`, branches, calls,
stack slots, loads and stores, `getelementptr` for stack slots, root-pointer arrays, and the slot
table, checked arithmetic intrinsics with `extractvalue`, and trap calls, with GOT cells modeled
as typed indices. `switch` is not emitted by the current compiler and is not part of the
fragment.

The exclusions and the phase that adds each one are recorded in the
[charter's exclusion register](Semantics-Charter.md#6-frozen-core-and-exclusion-register).
Excluded constructs are rejected by the reference tools and never interpreted silently.

## 2. Grammar, AST, HIR, and IR mapping

### 2.1 Tokens, trivia, spans, and the CST contract

P0.1 defines the token and trivia model, span coverage, and the lossless CST contract in
`Stc.Syntax.Tokens` and `Stc.Syntax.Cst`, following
[Tokenization](../Syntax/Tokenization.md). The contract states that token and trivia spans cover
the source exactly, in order, with no gaps or overlaps. `Cst.leaves` lists the leaf spans,
`CoversFrom` is the ordered gap-free coverage relation, `Lossless` is `CoversFrom 0 sourceLength`,
and `Cst.WellFormedNodes` records that every node span contains its children. The full
production-to-definition map is the [Core Syntax Coverage](Core-Coverage.md) table.

### 2.2 Surface AST and the `Derives` relation

P0.1 defines the surface AST in `Stc.Syntax.Ast` and the declarative relation
`Derives : Cst → Ast → Prop` in `Stc.Grammar.Derives`, mirroring the core productions in
[Grammar](../Syntax/Grammar.md). Every core production receives a constructor or relation rule;
the [Core Syntax Coverage](Core-Coverage.md) table maps productions to Lean rules. `Derives`
consumes the trivia-free projection `Cst.stripTrivia`; the helper relations `ListRel`, `ListRel1`,
`OptionalNode`, `CommaSeparated`, `StatementListRel`, and `DerivesChained` cover repetition,
optionality, comma lists, semicolon-terminated statement lists, and left-associative operator
chains.

### 2.3 AST-to-CST and source-span mapping

P0.1 defines the AST↔CST mapping and the source-span mapping used by Phase 4's parser
contracts. `MapsTo` relates a full CST to an AST through `Cst.stripTrivia`; `MapsToWithSpan` adds
root-span equality. `Derives.unit_span_eq` proves a derived source unit carries the source-file
node's span, `Derives.unit_span_wellFormed` proves that span is well formed,
`Derives.unit_span_bounded` proves it lies within the declared source file and length, and
`ast_span_wellformed` proves every descendant of a span-nested AST node is span-nested and
contained in the root. The per-node `Derives`-to-CST containment lemma is left to Phase 4's
parser-soundness proof; it is not part of the P0.1 exit gate.

### 2.4 Typed HIR

P0.2 defines the typed HIR in `Stc.Hir.Types` and `Stc.Hir.Core`: canonical `Ty` values
(`BOOL`, signed/unsigned integers, bit strings, and character layouts), typed literals, places,
expressions, statements, declarations, and a `HirUnit` with globals, functions, and exactly one
program. HIR is resolved and lowered: variables are `VarId` indices, callees are function or
standard-function targets, implicit conversions are explicit `Cast` nodes, call arguments are in
formal-parameter order, and `CASE` labels are folded constant integers. The family aligns with
the Rust `stc-hir` structures and the typed views in [AST, CST, and HIR](../AST/AST.md), but the
Lean family is the semantic reference. The dump alignment and canonical names are P0.6.

`Ty.layout` maps each core type to its `Stc.Exact.IntType` layout: `BOOL` is the one-bit
unsigned layout, integers carry their signedness, bit strings are unsigned, and `CHAR`/`WCHAR`/
`UCHAR` are 8/16/32-bit unsigned layouts. `Ty.isCore` fixes the frozen widths.

### 2.5 HIR-to-IR mapping and the covered fragment

P0.4 defines `IRFrag`, `IRContract`, and `WellFormed` in `Stc.Codegen.Ir`; P0.5 defines the
operational semantics in `Stc.Codegen.Semantics`. The mapping covers the production module the
compiler emits for the core source subset: scalar types, the overflow-intrinsic result, fixed
pointer and byte arrays, the emitted instructions and terminators, the generated entry-point
wrappers, and the module globals. `WellFormed` is defined over the whole module; P0.5 interprets
the covered user functions and the cycle body, with attach/prepare/shutdown as the initialization
boundary. The textual printer and parser and the debug/test build forms are out of scope, so the
semantics is defined over the in-memory fragment. The definition-to-emission map is the
[Core IR Coverage](Core-IR-Coverage.md) table.

## 3. Static semantics

### 3.1 Types, values, and widths

P0.2 defines the canonical `Ty` family and its widths. `BOOL` is a one-bit unsigned layout, an
`i8` in memory and an `i1` normalized with `icmp ne 0` in SSA. Integer and bit-string operations
reuse the existing `Stc.Exact` models, including checked overflow, truncating division, masked
shift and rotate counts, and conversion rules. `CHAR`, `WCHAR`, and `UCHAR` are unsigned integer
layouts; `UCHAR` uses a 32-bit unsigned layout. Literals are typed HIR values with an integer
payload; `Literal.WellFormed` requires the payload to fit the literal's core type.

**Known gap (Q3).** The P0.3 character model is layout-only. [UTF8-Strings.md](../UTF8-Strings.md)
requires `WCHAR_TO_UCHAR` to trap on lone surrogates and `UCHAR_TO_WCHAR` to trap above the BMP,
and defines `UCHAR` as a Unicode scalar. P0.3 does not enforce scalar validity and does not model
those conversion traps; the conversions are integer layout conversions. This gap is deferred to
the later string phase and excluded from P0.6 conformance until it is modeled. The source type
soundness result is therefore relative to the layout-only character model.

HIR typing is monomorphic. The declarative operator rules require arithmetic operands to be the
same integer type, bitwise operands to be the same `BOOL`/bit-string type, and comparison
operands to be the same type; comparisons produce `BOOL`. Assignment, argument passing, and
initializers require exact type equality because lowering has already inserted the source
analyzer's implicit conversions as explicit `Cast` nodes. The exact standard functions are
`ABS`, `SEL`, `MIN`, `MAX`, `LIMIT`, and `MUX`; arithmetic, bitwise, and comparison standard
functions are represented by the operator nodes.

### 3.2 Declarative typing judgment `HasType`

P0.2 defines the judgment over the lowered HIR in `Stc.Sem.Typing`:

- `ExprHasType ctx e T` synthesizes an expression's type;
- `CallHasType ctx callee args result` types direct function and standard-function calls;
- `StmtHasType ctx depth controlled s` types statements, tracking loop depth and the active
  `FOR` control variables;
- `GlobalTyped`, `FunctionTyped`, `ProgramTyped`, and whole-unit `HasType p env` cover
  declarations, initializers, and bodies.

`CASE` requires an integer selector, label bounds that fit the selector type, ordered ranges,
distinct labels, and no overlap between distinct labels. `FOR` requires an integer control
variable that is not already controlled, matching start/stop/step types, a non-zero constant
step, and a body that does not assign the control variable. `EXIT` and `CONTINUE` require an
enclosing loop. `RETURN` is available in functions and cycle bodies.

Deliberate differences from the Rust analyzer, recorded for P0.6: HIR is post-lowering, so
the checker does not re-insert implicit conversions; `CASE` selectors are restricted to integer
types (the analyzer also accepts character, bit-string, and enum selectors); and the P0.2
checker over-rejects `FOR` (see §3.3).

### 3.3 Core checker `checkCore`

`checkCore : HirUnit → Except Rejection Env` checks declaration types, global and local
initializers, and every body, and returns the declaration-derived environment. `Rejection`
carries the named rule and the exact failing construct and context.

Proved:

- `core_check_sound`: `checkCore p = .ok env → HasType p env`.
- `core_rejection_characterized`: `checkCore p = .error r → FailsRule r`, where `FailsRule`
  states the named rule's failure condition for the construct carried by `r`.
- `hir_wellformed`: `checkCore p = .ok env → HirWellFormed p`.

`core_check_complete` is deliberately descoped and registered as such. The P0.2 checker
over-rejects `FOR` with the named rule `StmtRule.forUnsupported`; the declarative judgment still
defines `FOR`, and P0.6's interpreter consumes the judgment. Full completeness is deferred to
Phase 2, where lowering connects the source checker to this judgment. Label validity is
characterized by the checker's label-problem function; the success path is proved equivalent to
`CaseLabelsValid` by `checkCaseLabels_sound`.

### 3.4 Conversions

`ConversionLegal` accepts exactly the conversions the analyzer's `can_convert` accepts for the
core scalar types: identity; integer-to-integer; bit-string-to-bit-string; character-to-
character; integer/bit-string cross-conversions; and `BOOL` to/from integers and bit strings.
Character-to-integer conversions are excluded. Character conversions use the layout-only model
described in §3.1; the Unicode scalar traps are the recorded Q3 gap. `conversion_legal_iff`
proves the relation is decided by `conversionLegalB`, the checker's executable form. `ImplicitConversion` records the
smaller source-level assignability relation the lowering must realize, and
`implicit_conversion_legal` proves it is contained in `ConversionLegal`. The behavioural
agreement between legal conversions and `Stc.Exact.IntType.convert` (success with a destination
value or a `RANGE` trap) is the conversion half of P0.3's `checked_arith_traps_exactly`.

### 3.5 Well-formedness

`HirWellFormed` requires every declared global, parameter, local, and result type to be a core
type. `hir_wellformed` proves accepted HIR satisfies it. Expression- and statement-level
restrictions (literal representability, cast legality, bound identifiers, loop depth, `CASE`
labels, `FOR` control) are part of the typing judgment. Target well-formedness `WellFormed` is
defined in P0.4 and is the precondition for the target soundness results.

## 4. Dynamic semantics

### 4.1 Configurations and the source control stack

P0.3 defines a configuration in `Stc.Sem.Machine`: persistent global roots, a stack of call
frames, the running frame's focus (`expr`, `value`, `stmts`, `resume`, `returned`, `trapped`,
or `halted`), and per-frame continuation stacks. A frame carries its parameters, locals,
result variable, continuation stack, and loop context. `EXIT` and `CONTINUE` target the
innermost loop marker in the continuation stack; `CONTINUE` in a `FOR` performs the checked
increment and bound test. A signed `FOR` uses the ascending bound test exactly when the runtime
step is positive; an unsigned loop always ascends. This matches `lower_for` and resolves Q4.
`RETURN` completes the current frame and delivers its result to the caller. The configuration-typing judgment `ConfigTyped` is defined in `Stc.Sem.Eval`;
`progress` and `preservation` are proved; type soundness follows once initial-state typing is established.

### 4.2 Source small-step relation `Step`

P0.3 defines `Step` as the graph of the total step function `Stc.Sem.step`, with a
prefix-oriented reflexive-transitive closure `Steps`, and `Terminal` as `step u c = .halt`.
The relation is the reference for source soundness and does not require termination; functions
may be recursive. `eval_deterministic` and `progress` are proved; `preservation` is pending.

### 4.3 Recursive reference model `stcSem`

P0.3 defines the fuel-based executable model `runFuel` and its entry `stcSem` in
`Verification.Semantics.Interpreter`, over the same configuration type. The interpreter threads
explicit state and encodes control flow in the continuation stack and the focus, not as a host
exception. `interpreter_sound` (every reported result is reachable by `Steps` and terminal) and
`interpreter_complete` (every terminating run is reproduced with enough fuel) are proved.

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

The source semantics covers one program and one sequential cycle. `initState u cycles` models
the initialization boundary: global roots and program fields start at their zero defaults, the
first statement list runs global initializers, then program-field initializers, then the first
cycle body in declaration order, and `completeFrame` ends a cycle, decrements `cyclesLeft`, and
starts the next cycle from the persisted program frame. `RETURN` in a program body ends the
cycle. The target semantics covers the ABI 2 entry points and the cycle body; attach, prepare,
activate, commit, rollback, and unload beyond the initialization boundary are Phase 8.

## 5. Values, memory, GOT, traps, and undefined behavior

### 5.1 Values and memory model

The core has stack frames and persistent GOT cells. There is no heap, no general pointer
arithmetic, and no aliasing beyond GOT cells. Target values are integers, pointers, and the
typed cell contents described by `WellFormed`.

### 5.2 GOT root and slot model

A GOT cell is an index, not an arbitrary pointer. The target state carries a root map
`GotId → Value`, a slot table `Slot → CellKind`, and a function table `GotId → Callable`.
`WellFormed` records each cell's kind, and the semantics checks kind agreement on every load,
store, and call. The cell kinds are `Data`, `Function`, `Descriptor`, `Direct`, and `Import`;
`Descriptor` cells are structurally representable but semantically out of the core because OOP is
excluded. The slot table is grouped in the canonical kind order; sorting within a group by
`GotId`/service ID is Phase 3's `slots_canonical`. `ir_got_sound` states that a load returns the
root value recorded by the slot table.

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
`checked_arith_traps_exactly` states that source checked arithmetic agrees with the `Stc.Exact`
models, including checked conversions and the exact standard functions. `evalCast` and
`evalStd` re-check their results with the exact checked wrapper. P0.3 keeps this guarded form
(Q2 resolved in [Plan-Phase0.md](Plan-Phase0.md) §19.8); the guard is redundant for representable
inputs but the unguarded equality needs the `convert_ok_fits` and standard-function fit lemmas,
which are recorded as a P0.5/Phase 2 follow-up. `ir_checked_arith_refines_exact` states that the IR
instruction sequence plus its trap path produces the same result or trap as the exact model.

## 7. Theorem inventory and proof status

The authoritative status of every entry is [`lean/verification-status.toml`](../../lean/verification-status.toml);
`tools/ci/check_verification_registry.py` validates it and audits proved theorems. This section
mirrors the registry while P0.3 is resumed and P0.4 is complete: the P0.1 and P0.2 entries are
proved except `core_check_complete`, which is descoped with the reason recorded in the registry;
the P0.3 entries for determinism, trap soundness, guarded exact arithmetic, and the interpreter
are proved, `progress` is proved, while `preservation` and `type_soundness` remain in progress; the P0.4
definitions and structural lemmas are proved. Open P0.3 questions are recorded in
[Plan-Phase0.md](Plan-Phase0.md) §19 and the P0.4 decisions in §20. The registry is regenerated
into this section at P0.7.

### 7.1 Source soundness

| Theorem | Module | Sub-phase | Status |
| --- | --- | --- | --- |
| `Stc.Syntax.ast_span_wellformed` | `Stc.Syntax.Ast` | P0.1 | proved |
| `Stc.Sem.core_check_sound` | `Stc.Sem.Typing` | P0.2 | proved |
| `Stc.Sem.core_check_complete` | `Stc.Sem.Typing` | P0.2 | descoped (see registry) |
| `Stc.Sem.core_rejection_characterized` | `Stc.Sem.Typing` | P0.2 | proved |
| `Stc.Sem.hir_wellformed` | `Stc.Sem.Typing` | P0.2 | proved |
| `Stc.Sem.conversion_legal_iff` | `Stc.Sem.Typing` | P0.2 | proved |
| `Stc.Sem.eval_deterministic` | `Stc.Sem.Eval` | P0.3 | proved |
| `Stc.Sem.progress` | `Stc.Sem.Eval` | P0.3 | proved |
| `Stc.Sem.preservation` | `Stc.Sem.Eval` | P0.3 | proved |
| `Stc.Sem.type_soundness` | `Stc.Sem.Eval` | P0.3 | in progress |
| `Stc.Sem.trap_soundness` | `Stc.Sem.Eval` | P0.3 | proved |
| `Stc.Sem.checked_arith_traps_exactly` | `Stc.Sem.Eval` | P0.3 | proved (guarded form, Q2) |

### 7.2 Target syntax and well-formedness

P0.4 defines the production-module fragment and its well-formedness predicate. The structural
lemmas are projections of `Stc.Codegen.WellFormed`; the full inventory and shapes are in
[Plan-Phase0 §7.1](Plan-Phase0.md#71-p04-structural-lemmas).

| Lemma | Module | Sub-phase | Status |
| --- | --- | --- | --- |
| `Stc.Codegen.wellFormed_blocks_have_terminators` | `Stc.Codegen.Ir` | P0.4 | proved |
| `Stc.Codegen.wellFormed_phis_first` | `Stc.Codegen.Ir` | P0.4 | proved |
| `Stc.Codegen.wellFormed_phi_predecessors_match` | `Stc.Codegen.Ir` | P0.4 | proved |
| `Stc.Codegen.wellFormed_types_correct` | `Stc.Codegen.Ir` | P0.4 | proved |
| `Stc.Codegen.wellFormed_uses_defined` | `Stc.Codegen.Ir` | P0.4 | proved |
| `Stc.Codegen.wellFormed_module_shapes` | `Stc.Codegen.Ir` | P0.4 | proved |
| `Stc.Codegen.wellFormed_slot_refs_in_range` | `Stc.Codegen.Ir` | P0.4 | proved |
| `Stc.Codegen.wellFormed_slot_refs_kind_correct` | `Stc.Codegen.Ir` | P0.4 | proved |
| `Stc.Codegen.wellFormed_slot_table_canonical` | `Stc.Codegen.Ir` | P0.4 | proved |
| `Stc.Codegen.wellFormed_entry_points_match_contract` | `Stc.Codegen.Ir` | P0.4 | proved |
| `Stc.Codegen.wellFormed_no_direct_root_or_callable` | `Stc.Codegen.Ir` | P0.4 | proved |
| `Stc.Codegen.wellFormed_pre_attach_clean` | `Stc.Codegen.Ir` | P0.4 | proved |
| `Stc.Codegen.wellFormed_local_globals_allowed` | `Stc.Codegen.Ir` | P0.4 | proved |
| `Stc.Codegen.wellFormed_cells_not_written` | `Stc.Codegen.Ir` | P0.4 | proved |
| `Stc.Codegen.wellFormed_descriptor_and_table` | `Stc.Codegen.Ir` | P0.4 | proved |

### 7.3 Target soundness

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

### 7.4 Interpreter equivalence and harness properties

| Theorem | Module | Sub-phase | Status |
| --- | --- | --- | --- |
| `Verification.Semantics.interpreter_sound` | `Verification.Semantics.Interpreter` | P0.3 | proved |
| `Verification.Semantics.interpreter_complete` | `Verification.Semantics.Interpreter` | P0.3 | proved |
| `Verification.Semantics.semantics_versioned` | `Verification.Semantics.Conformance` | P0.6 | planned |

### 7.5 Conformance evidence

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
| 2026-10-27 | 1.0-draft | P0.1 | Defined tokens, trivia, spans, the lossless CST contract, the surface AST, and `Derives`; proved `ast_span_wellformed` and the root source-span lemmas; added the [Core Syntax Coverage](Core-Coverage.md) table |
| 2026-10-27 | 1.0-draft | P0.2 | Defined the typed HIR, the declarative typing judgment, `checkCore`, `HirWellFormed`, and the conversion relation; proved `core_check_sound`, `core_rejection_characterized`, `hir_wellformed`, and `conversion_legal_iff`; descoped `core_check_complete` with the `FOR` over-rejection recorded |
| 2026-10-27 | 1.0-draft | P0.3 (in progress) | Defined values and traps, the small-step machine, the configuration typing, and the fuel interpreter; proved `eval_deterministic`, `trap_soundness`, guarded `checked_arith_traps_exactly`, `interpreter_sound`, and `interpreter_complete`; `progress`, `preservation`, `type_soundness`, and initial-state typing remain; open questions Q1–Q7 and the drift register are recorded in Plan-Phase0.md §19 |
| 2026-10-31 | 1.0-draft | P0.4 (complete) | Started WL-3. Fixed the production-module fragment: five GOT cell kinds, a grouped slot table, the overflow-result and fixed-array types, rotate intrinsics, `extractvalue`, and `unreachable`; removed `switch` (CASE lowers to branches). Defined `IRFrag`, `IRContract`, `ModuleShapes`, `WellFormed`, and the structural lemmas, and proved the P0.4 lemmas with the standard Lean axioms; the definition-to-emission map is the [Core IR Coverage](Core-IR-Coverage.md) table. The P0.4 review was approved 2026-10-31 |
| 2026-10-31 | 1.0-draft | P0.3 (resumed) | Reevaluated Q1–Q7: keep the current machine and `ConfigTyped` invariant; keep the guarded exact form; record layout-only characters as a known Q3 gap; correct the `FOR` zero-step test to `step > 0` for signed loops and always-ascending for unsigned; confirm the initialization boundary and the `RANGE` power trap; commit WIP checkpoints. |
| 2026-10-31 | 1.0-draft | P0.3 (progress) | Proved `progress`: a `ConfigTyped` configuration is terminal or takes a step, via the expression, value, statement, resume, and return no-stuck lemmas. `preservation`, initial-state typing, and `type_soundness` remain |
| 2026-10-31 | 1.0-draft | P0.3 (preservation) | Proved `preservation` over every focus form, including the value/statement/resume/returned transitions and the FOR advance path. Fixed three machine bugs found by the proof: the call-argument continuation advanced past the wrong argument, `EXIT` kept the loop body's loop context, and `CONTINUE` on a `FOR` entered the increment in the body's loop context. Initial-state typing and `type_soundness` remain |

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
is a verification tool, not a production interpreter; performance is not a goal. P0.3 provides
the fuel-based core (`runFuel`/`stcSem` in §4.3); P0.6 adds the dump and case front ends.

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
- [Core Syntax Coverage](Core-Coverage.md)
- [Core IR Coverage](Core-IR-Coverage.md)
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
