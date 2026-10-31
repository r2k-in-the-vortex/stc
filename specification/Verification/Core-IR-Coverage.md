# Core IR Coverage — P0.4

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-10-31
Status: Approved for P0.4 review (2026-10-31). This table is the traceability record from the
emitted
production-module fragment to the Lean definitions in [`Stc.Codegen.Ir`](../../lean/Stc/Codegen/Ir.lean).
It makes no parser or refinement claim; Phase 5 owns the textual printer/parser and the proved
checker. The table is updated when the Lean definitions or the compiler's emitted instruction set
change.
Parent: [Phase 0 Implementation Plan](Plan-Phase0.md) §6 (P0.4)
Semantics reference: [stc Semantics — Phase 0 Core](Semantics.md)

## 1. Scope

The covered target is the production module the current `stc-codegen` emits for the frozen core
source subset in [Plan-Phase0.md](Plan-Phase0.md) §3.3. `WellFormed` is defined over the whole
module, including the generated entry-point wrappers and module globals. The P0.5 target
semantics interprets the covered user functions and the cycle body; attach/prepare/shutdown are
the initialization boundary.

Out of scope: debug-info builds, test builds and coverage instrumentation, debug checkpoint
hooks, the textual printer/parser, the full LLVM memory model, OOP descriptors/vtables, the
process image, and raw pointer arithmetic. Build-form exclusions are listed in §8.

The Lean module is the umbrella [`Stc.Codegen.Ir`](../../lean/Stc/Codegen/Ir.lean). The
definitions sit in the `Stc.Codegen` namespace:

| Definition | Meaning |
| --- | --- |
| `IrType` | Core IR types: `void`, integer widths, `ptr`, the overflow-intrinsic result, and fixed arrays |
| `Operand` | SSA local, integer literal, null pointer, or symbol reference |
| `Instr` | Core instruction forms |
| `Terminator` | `ret`, `br`, conditional `br`, `unreachable` |
| `Block`, `Function`, `Global`, `Declaration` | Module structure |
| `IRFrag` | The whole module plus the slot and function tables |
| `CellKind`, `SlotTable` | The GOT cell-kind model and its canonical group order |
| `IRContract` | The per-build symbols, entry points, and descriptor/function-table specs |
| `FunctionFacts` | The builder-recorded GOT facts checked by `WellFormed` |
| `WellFormed` | The well-formedness predicate |

## 2. Module shapes

| Emitted shape | Lean definition | Specification |
| --- | --- | --- |
| Named type definitions | `TypeDef`; core production modules carry none | Plan-Phase0 §3.4 |
| Module globals | `Global` with `GlobalInit` | GOT Artifacts §7.3; Runtime and Execution §2.3 |
| External declarations | `Declaration` (LLVM intrinsics) | Plan-Phase0 §3.4 |
| Function definitions | `Function` with linkage, return type, parameters, blocks | Plan-Phase0 §3.4 |
| Slot table | `IRFrag.slots : SlotTable` (one `CellKind` per host cell) | Online Change and GOT §3.3; GOT Artifacts §7.1 |
| Function-address table | `IRFrag.functionTable : List String` and the emitted `[N x ptr]` global | GOT Artifacts §7.1, §7.3 |

## 3. Types

| LLVM spelling | Lean type | Emitted for | Specification |
| --- | --- | --- | --- |
| `void` | `IrType.void` | Procedure returns | Plan-Phase0 §3.4 |
| `i1` | `IrType.int 1` | `BOOL` in SSA; comparison results | Semantics §3.1 |
| `i8` | `IrType.int 8` | `SINT`, `USINT`, `BYTE`, `CHAR` | Semantics §3.1 |
| `i16` | `IrType.int 16` | `INT`, `UINT`, `WORD`, `WCHAR` | Semantics §3.1 |
| `i32` | `IrType.int 32` | `DINT`, `UDINT`, `DWORD`, `UCHAR` | Semantics §3.1 |
| `i64` | `IrType.int 64` | `LINT`, `ULINT`, `LWORD` | Semantics §3.1 |
| `ptr` | `IrType.ptr` | Locals, GOT cell values, calls | Plan-Phase0 §3.4 |
| `{ iN, i1 }` | `IrType.overflow N` | `llvm.*.with.overflow` result | Semantics §6 |
| `[N x ptr]` | `IrType.array IrType.ptr N` | Function table, root-pointer arrays | GOT Artifacts §7.1; Plan-Phase0 §3.4 |
| `[N x i8]` | `IrType.array (IrType.int 8) N` | Descriptor blob, metadata blob, retain buffer | GOT Artifacts §7.1; Plan-Phase0 §3.4 |

No other aggregate, floating-point, or vector type is part of the fragment.

## 4. Instructions

| LLVM instruction | Lean constructor | Emitted for | Specification |
| --- | --- | --- | --- |
| `alloca T` | `Instr.alloca` | Scalar locals, root-pointer arrays | Semantics §4.1, §5.1 |
| `load T, ptr P` | `Instr.load` | Locals, GOT cell addresses/values, roots | Semantics §5.1–5.2 |
| `store T V, ptr P` | `Instr.store` | Locals, root storage through `Data`/`Direct` cells | Semantics §5.2 |
| `getelementptr T, ptr B, ...` | `Instr.gep` | Root-pointer arrays, slot-table entries | GOT Artifacts §7.3 |
| `add`/`sub`/`mul` | `Instr.binop` | Exact integer arithmetic | Semantics §3.1, §6 |
| `sdiv`/`udiv`/`srem`/`urem` | `Instr.binop` | Exact division and remainder | Semantics §3.1, §6 |
| `and`/`or`/`xor` | `Instr.binop` | Bitwise operations and boolean conditions | Semantics §3.1 |
| `shl`/`lshr`/`ashr` | `Instr.binop` | Masked shifts | Semantics §3.1, §6 |
| `llvm.fshl.iN`/`llvm.fshr.iN` | `Instr.call` to the named intrinsic | `ROL`/`ROR` | Semantics §3.1, §6 |
| `icmp P T A, B` | `Instr.icmp` | Comparisons, `BOOL` normalization, loop bounds | Semantics §3.1 |
| `select i1 C, T A, B` | `Instr.select` | `FOR` bound selection; `SEL`/`MIN`/`MAX`/`LIMIT`/`MUX` lowering | Semantics §3.1 |
| `zext`/`sext`/`trunc` | `Instr.cast` | Integer and character conversions; shift-count narrowing | Semantics §3.4 |
| `phi T [V, %P], ...` | `Instr.phi` | `&&`/`||` short-circuit merges | Semantics §4.2 |
| `call T @f(args)` | `Instr.call` | Intrinsics, compiler helpers, pre-attach accessor bodies | Plan-Phase0 §3.4 |
| `call T %p(args)` | `Instr.callPtr` | User calls through `Function`/`Import` cells; trap calls | Online Change and GOT §3.1 |
| `extractvalue { iN, i1 } V, 0/1` | `Instr.extractValue` | Checked-arithmetic result and overflow flag | Semantics §6 |
| `unreachable` | `Terminator.unreachable` | After a trap call | Semantics §5.3 |

`insertvalue`, `llvm.memcpy`, `ptrtoint`/`inttoptr`, `switch`, floating-point operations, atomics,
volatile access, `invoke`/`landingpad`, and dynamic `alloca` are not part of the fragment.

## 5. Terminators

| LLVM terminator | Lean constructor | Emitted for | Specification |
| --- | --- | --- | --- |
| `ret void` / `ret T V` | `Terminator.ret` | Function and program-body returns | Semantics §4.2 |
| `br label %L` | `Terminator.br` | Sequential and loop control flow | Semantics §4.2 |
| `br i1 C, label %T, label %F` | `Terminator.condBr` | `IF`, `CASE`, loop conditions, trap guards | Semantics §4.2 |
| `unreachable` | `Terminator.unreachable` | The block after a checked-arithmetic trap | Semantics §5.3 |

Every well-formed block has exactly one of these after its instructions.

## 6. Globals and entry points

| Emitted symbol | Lean shape | Rule | Specification |
| --- | --- | --- | --- |
| `stc_module_descriptor_data` | `Global` constant `[N x i8]`, bytes | Contract descriptor symbol and size | GOT Artifacts §7.1, §7.3 |
| `stc_module_function_table` | `Global` constant `[N x ptr]`, symbol array | Contract table symbol and count; entries match `IRFrag.functionTable` | GOT Artifacts §7.1, §7.3 |
| `stc_module_api`, `stc_module_slots` | `Global` mutable `ptr`, `null` | Module-local, not a root | Online Change and GOT §3.3 |
| `stc_retain_buffer` | `Global` mutable `[1 x i8]`, zeroinitializer | Module-local | State and Retain §2 |
| `stc_metadata_json_data` | `Global` constant `[N x i8]`, bytes | Opaque module-local metadata | Plan-Phase0 §3.4 |

The required entry points are the seven host ABI 2 production symbols from
[GOT Artifacts §7.3](../Architecture/GOT-Artifacts.md): `stc_module_descriptor`,
`stc_module_functions`, `stc_module_attach`, `stc_module_prepare`, `stc_module_cycle`,
`stc_module_shutdown`, and `stc_module_detach`. `IRContract.entryPoints` carries their exact
return and parameter types; `wellFormed_entry_points_match_contract` states that each exists with
external linkage.

## 7. Well-formedness rules

| Rule | Lean conjunct | Rust verifier counterpart |
| --- | --- | --- |
| Type correctness | `WellFormed.typesCorrect` | Builder-only; the verifier does not re-check |
| Module names and global/declaration types | `WellFormed.moduleShapes` | Builder-only; the verifier does not re-check |
| SSA uniqueness and dominance | `WellFormed.usesDefined` | Builder-only; the verifier does not re-check |
| Block terminators | `WellFormed.blocksHaveTerminators` | `Function::check` |
| `phi` firstness and predecessor agreement | `WellFormed.phisFirst`, `WellFormed.phiPredecessorsMatch` | Builder-only |
| Slot references in range | `WellFormed.slotRefsInRange` | `slot-range` |
| Slot kind agreement | `WellFormed.slotRefsKindCorrect` | `slot-kind`, `indirect-call` |
| Grouped slot table | `WellFormed.slotTableCanonical` | Catalog assignment; not re-checked by the verifier |
| Entry-point signatures and linkage | `WellFormed.entryPointsMatchContract` | `entry-point` |
| No direct root or callable references | `WellFormed.noDirectRootOrCallable` | `direct-root-reference`, `direct-function-reference`, `direct-user-call`, `root-global` |
| Pre-attach accessors | `WellFormed.preAttachClean` | `pre-attach` |
| Local-global allowlist | `WellFormed.localGlobalsAllowed` | `global-access` |
| No cell writes | `WellFormed.cellsNotWritten` | `cell-write` |
| Descriptor and function-table shape | `WellFormed.descriptorAndTable` | `descriptor`, `function-table` |

`FunctionFacts` mirrors the builder's recorded facts (`origins`, `directCalls`, `indirectCalls`,
`memoryAccesses`, `cellWrites`, `memoryGlobals`, `symbolRefs`). Deriving those facts from the
textual IR is Phase 5's checker obligation; P0.4 checks them against the contract. The structural
lemmas are listed in [Plan-Phase0 §7.1](Plan-Phase0.md#71-p04-structural-lemmas).

## 8. Exclusions and build forms

| Excluded | Reason | Added by |
| --- | --- | --- |
| Debug-info builds and `llvm.dbg.*` | Out of the production fragment; debug is excluded from the core | Phase 5/7 if debug is ever verified |
| Test builds, coverage counters, `set_cycle_time_ns` | Test-only instrumentation | Not planned for the verified fragment |
| Debug checkpoint hooks (`debug_check`, service 13) | Debug-only ABI 2.3 feature | Not planned for the verified fragment |
| `switch` | `CASE` currently lowers to comparison branches | Phase 5 if lowering changes |
| `insertvalue`, `llvm.memcpy` | Aggregate/parameter copies outside the scalar core | Phase 5/7 |
| `ptrtoint`/`inttoptr`, `fadd`/`fsub`/`fmul`/`fdiv`/`frem` | References and floating point are excluded from the source core | Phase 5–7 |
| Descriptor/vtable/TYPEINFO blobs, process image | OOP, I/O, and online change are later phases | Phase 5/7/8 |

## 9. Change control

The fragment is defined by the emitted production module, not by the full textual IR. When
`stc-codegen` changes an emitted instruction, type, global, or entry-point shape, update this
table and the Lean definition in the same change. A construct that appears only in debug, test,
or OOP builds is rejected by the fragment rather than modeled silently.
