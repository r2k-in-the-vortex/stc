# Phase 0 Design and Decision Record

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-11-02
Semantics version: `1.0`
Status: Durable Phase 0 design record. This document preserves the decisions, target-state
invariant, proof architecture, exit-criteria assessment, and proof-engineering notes from the
retired Phase 0 working plan. The normative semantics is [Semantics.md](../Semantics.md); results,
conformance evidence, gaps, TCB, and the Phase 1–8 handoff are in
[Phase0-Report.md](Phase0-Report.md); the active program plan is
[VerificationProgramPlan.md](../VerificationProgramPlan.md).

## 1. Phase 0 decisions

| ID | Decision | Resolution |
| --- | --- | --- |
| P0-D1 | Exact core language subset | Use the frozen §3.3 subset; exclude floats, strings, aggregates, OOP, references, FBs, tasks, I/O, and online change. |
| P0-D2 | Exact target IR fragment and memory model | Use the frozen §3.4 fragment; abstract GOT cells and stack frames; exclude general LLVM memory and UB. |
| P0-D3 | Small-step versus functional semantics | Define both the recursive executable model and the loop-shaped small-step relation; prove them equivalent and expose both as refinement targets. |
| P0-D4 | HIR dump format and version | Stable, versioned JSON with schema validation; no Rust internals; see [HIR-Dump.md](../HIR-Dump.md). |
| P0-D5 | Mathlib/Batteries policy | Core Lean plus Batteries for executable models; Mathlib proof-only after a build-time spike. |
| P0-D6 | Floats, time, strings, and aggregates | Excluded by P0-D1; later phases own them. |
| P0-D7 | Conformance oracle | Compare the compiled artifact for execution; keep a no-clang pure-function path. |
| P0-D8 | Semantics document location | [Semantics.md](../Semantics.md), versioned with the plan. |
| P0-D9 | Recursion and fuel | Include recursion, fuel the interpreter, and do not require a source termination proof. |
| P0-D10 | Conformance acceptance threshold | Every core case agrees or has a written triage exclusion; no silent exclusions. |

## 2. P0.4 target scope decisions

- `IRFrag`/`WellFormed` cover the whole emitted production module, including generated
  entry-point wrappers and module globals; P0.5 interprets the cycle body and the covered user
  functions.
- All five GOT cell kinds are modeled: `Data`, `Function`, `Descriptor`, `Direct`, `Import`.
- The slot table is grouped in the canonical kind order; identity sorting remains Phase 3's
  `slots_canonical`.
- `WellFormed` includes every `stc-codegen` structural rule applicable to the core fragment,
  with the definition-to-emitter mapping in [Core-IR-Coverage.md](../Core-IR-Coverage.md).
- `switch` was removed from the fragment because the compiler lowers `CASE` to branches.
- Descriptor/function-table content pairing remains Phase 5 work.

## 3. Open questions Q1–Q7

| Question | Resolution |
| --- | --- |
| Q1 — completion path | Keep the current machine and `ConfigTyped` invariant and finish the proofs. |
| Q2 — exact-model link | Keep the guarded `evalCast`/`evalStd` form; the unguarded equality is a Phase 2 follow-up. |
| Q3 — character validity | Keep the layout-only character model; Unicode scalar validity and the `WCHAR_TO_UCHAR`/`UCHAR_TO_WCHAR` traps remain a documented gap. |
| Q4 — `FOR` zero step | Signed loops use the ascending test only when `step > 0`; unsigned loops always ascend. |
| Q5 — initialization boundary | Global initializers, then program-field/local initializers, then the cycle body; `RETURN` ends the cycle and `cyclesLeft` persists program state. |
| Q6 — negative signed exponent | `evalPow` traps `RANGE`; constant evaluation rejects the constant case. |
| Q7 — checkpoint policy | Commit compiling work as WIP checkpoints on `WL-3`. |

## 4. Target-state invariant

P0.5 separates module-level builder facts from per-state facts. `WFProgram contract frag` is the
P0.4 `WellFormed` predicate plus the builder-recorded facts that Phase 5 derives:

`allocsNonEmpty`, `localTypesRepresentable`, `originsTrusted`, `stackOriginsTrusted`,
`callPtrOrigins`, `trapCodesSpecified`, `unreachableGuarded`, `returnNonStack`,
`slotsGlobalReadOnly`, `slotBasesNonParam`, `slotBasesLoadOnly`, `cellOriginsSlotEntry`,
`storesNonPointer`, `allocaRepresentable`, `originGlobals`, `originSlotKinds`,
`slotsSymbolsLoadOnly`, `intrinsicNamesReserved`.

`WFState contract c` holds `WFProgram contract c.module` plus the state fields:

`frameIdsNodup`, `frameIdsBound`, `frameFunction`, `frameBlock`, `framePrev`,
`noBareUnreachable`, `localsInitialized`, `localsTyped`, `originsMatch`, `allocsWellFormed`,
`stackPtrsBounded`, `stackPtrsOrdered`, `memTyped`, `memNoPtr`, `storagePtrsLive`, `slotsBase`,
`slotsStorage`, `storageValuesLive`, `storageDomain`, `noSlotsGlobalPointers`,
`functionBindings`, `callTargets`, `returnLink`, `bottomRetDest`, `stackReturnsSafe`,
`payloadsTyped`.

## 5. P0.5 decisions D1–D4

### D1 — offset-aware persistent storage

Option 2 was adopted. `IRState` carries host-supplied `storage : List (StoreId × List IRValue)`,
with `lookupStorage`/`writeStorage`; `.root` and `.global` addresses carry offsets;
`loadValue`/`storeValue` read and write persistent storage rather than the GOT cell.
`IRAddr.slotEntry` models the first load of the compiler's two-load slot access: the slot-table
GEP produces `.slotEntry slot`, and a load from it yields `.ptr (.cell slot)`. `.cell` remains
non-writable and host-owned. `initState?`/`IRSem`/`irRun` take the host storage and globals.

### D2 — bounds-checked memory and undefined behavior

Option 1 was adopted. `evalGepAddr` checks allocation/storage extents; successful `.gep` and
load/store steps re-check extent and element type; out-of-range accesses are defined `.stuck`
faults, not undefined behavior. `StackPtrsBounded` and `AllocsWellFormed` encode the extent side,
and `AccessBounded` carries the source-check obligation for dynamic indices. The Q3 character
path needs `CharIndexBounded` or exclusion until the character model is resolved. `ir_no_ub`
remains about `unreachable`, unknown trap codes, host imports, and cell stores.

### D3 — indirect-call origins

P0.4 is kept frozen. `OriginsTrusted`, `CallPtrOrigins`, `TrapCodesSpecified`, and
`UnreachableGuarded` are builder-trusted `WFProgram` facts. `OriginHolds` ties every recorded
local to the value shape and slot kind at run time; `FunctionBindingsTyped` and
`CallTargetsTyped` connect callable cell values to module functions or the trap service.

### D4 — return linkage

`IRState` carries host-supplied `functionSignatures`, with `lookupFunctionSignature`.
`FunctionBindingsTyped` requires a Function-cell binding to name a module function with exactly
that signature. `CallTargetsTyped` checks `ret` and argument operand types against the cell's
signature, and requires Import-cell calls to be the trap service. `ReturnLink` holds for every
adjacent frame pair and, for `retDest = some dest`, requires the destination type to match the
callee return type, no recorded origin, non-parameter status, and `IsCallDest caller.fn dest`.

## 6. Resolved invariant gaps

The Phase 0 proof pass closed the following gap groups; the resulting facts are in the field
lists above. No gap remains open.

1. The initial inventory: `ReturnLink` scope, storage-domain liveness, slots-global read-only,
   alloca representability, origin `.global` linkage, and descriptor-cell origins.
2. Module-wide slots storage at calls.
3. Slot-base parameters at calls.
4. Return-destination origins.
5. Non-head bare-unreachable frames.
6. Return destination as the slots parameter.
7. Stack pointers into a popped frame.
8. Pointer stores into another frame's memory.
9. Slot-base destinations and the untracked call destination.
10. Storage values as frame pointers.
11. Cell-origin loads and slot-entry GEPs.
12. `extractvalue` pair-payload typing.
13. Direct calls versus intrinsic signatures.
14. Overflow-typed literals.
15. Jump-target return safety.
16. Callee `%slots` parameter forwarding.
17. `CallTargetsTyped` quantifier scoping.

The builder slots ABI was removed (`8194013`), the Lean slots fields were dropped (`04cd781`),
and `CallTargetsTyped` was restated universally (`4cfb238`, `7c0dac1`).

## 7. Proof architecture

- **Declarative then algorithmic.** Define the declarative judgment first, then the executable
  checker/model, then prove soundness and completeness or characterize rejection. This is used
  for `HasType`/`checkCore` and for `Step`/`Eval`/`stcSem`.
- **Source small step.** A configuration is the focused expression or statement, locals, global
  roots, call stack, and control stack. Terminal configurations are values, halts, and traps.
  `EXIT`, `CONTINUE`, and `RETURN` are explicit, so progress and preservation include the control
  stack. Loops are handled by the control stack and induction on `Step`, not by a global
  termination argument.
- **Progress and preservation.** Progress says a well-typed, well-formed, non-terminal
  configuration steps or is a documented trap/halt; it does not claim termination. Preservation
  is stated for `ConfigTyped`, including the control stack.
- **Target memory and GOT.** The target state has typed stack slots, a root map and slot table, a
  function table, a call stack, and a program counter. A GOT cell is an index, not an arbitrary
  pointer; `WellFormed` records cell kinds and the semantics checks kind agreement. This avoids
  modeling general LLVM provenance while matching the compiler's access pattern.
- **Traps and UB.** Traps are terminal states with a code and span. `trap_soundness` bounds the
  trap set; `ir_no_ub` excludes poison/undef, out-of-bounds stack access, misalignment, and raw
  pointer arithmetic on the covered fragment.
- **Exact arithmetic.** Target checked operations are related to `Stc.Exact` by
  `ir_checked_arith_refines_exact`, connecting the target semantics to the existing exact
  models.
- **Recursive model and refinement handoff.** `Eval`/`Step` and `IRStep` are the declarative
  loop-shaped relations; `stcSem` and `irExecRec` are the recursive executable models;
  `interpreter_sound`/`interpreter_complete` and `ir_recursive_model_equiv` prove the internal
  equivalences. Phase 1 supplies the Rust loop equivalence and termination measure. State and
  control flow are explicit values, not host exceptions; this is the shape Aeneas or a manual
  loop proof can relate to.

## 8. Exit-criteria assessment

| Criterion | Result |
| --- | --- |
| Frozen core language subset and target fragment | Met; [Semantics.md](../Semantics.md) §1 and [Core-IR-Coverage.md](../Core-IR-Coverage.md). |
| Lean modules defined for the core | Met: `Stc.Syntax`, `Stc.Grammar`, `Stc.Hir`, `Stc.Sem`, `Stc.Codegen.Ir`, `Stc.Codegen.Semantics`. |
| Theorem inventory proved or descoped with justification | Met; registry and [Semantics.md](../Semantics.md) §7. |
| Recursive/loop-shaped models agree and Phase 1 points are recorded | Met; equivalence theorems and [Phase0-Report.md](Phase0-Report.md) §8. |
| `lake exe stc-sem` runs the core corpus | Met. |
| Conformance harness agrees on every case or documents an exclusion | Met: 74/74 cases; E1–E7 in [Phase0-Report.md](Phase0-Report.md) §5. |
| Semantics reviewed, versioned `1.0`, and traceable | Met; [Semantics.md](../Semantics.md) and [Phase0-Report.md](Phase0-Report.md) §7. |
| Report records results, gaps, exclusions, and handoff | Met: [Phase0-Report.md](Phase0-Report.md). |
| Registry and parent plan reflect the S-level claim | Met: registry and [VerificationProgramPlan.md](../VerificationProgramPlan.md). |

## 9. CI, proof engineering, and risks

The Phase 0 CI and proof-engineering contract is:

- `lake build` must pass; CI fails on `sorry`/`admit`; top-level theorems are audited with
  `#print axioms`, and any non-standard axiom is listed in the TCB.
- The registry resolves every Phase 0 theorem to a Lean name and a specification section.
- `lake exe stc-sem` has unit and corpus tests; the conformance gate regenerates the committed
  report and rejects a stale copy; the docs and Markdown checks cover the report and semantics
  document.
- Executable models use core Lean plus Batteries; Mathlib is proof-only if adopted; `native_decide`
  is avoided in trusted theorems unless the trust cost is documented.

The principal risks and mitigations were: core-subset size (freeze and exclude aggressively),
semantics/implementation drift (versioned conformance and triage), proof cost (small-step
semantics and restricted core), LLVM memory/UB complexity (abstract GOT cells and stack frames),
dump instability (version and validate), schedule (timebox and prioritize), and corpus thinness
(seed from existing suites and cover every core feature/trap).
