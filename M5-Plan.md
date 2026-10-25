# M5 Implementation Plan — FBs, OOP, Aggregates, Strings, and Standard Library

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-25  
Status: Phase 0 written. Phases 1–7 are implemented incrementally; the status table records
the state of each phase.

Inputs: [Project Plan](Specification/00-Project-Plan.md) §8, [Architecture](Specification/Architecture/Architecture.md)
§4, [Runtime and Execution](Specification/Architecture/Runtime-and-Execution.md) §1–§4,
[Online Change and GOT](Specification/Architecture/Online-Change-and-GOT.md),
[GOT Artifacts and Schemas](Specification/Architecture/GOT-Artifacts.md),
[State and RETAIN](Specification/Architecture/State-and-Retain.md) §2–§7,
[Testing and Verification](Specification/Architecture/Testing-and-Verification.md) §1–§4 and §6,
`handover_to_M5.md`, and Decisions D-128–D-136.

M5 starts on the M4.5 host/module model. Persistent values are host-owned per-field roots reached
through GOT cells; the module owns code and version-local read-only data only. M5 does not
reintroduce module-owned persistent storage, contiguous persistent parent objects, raw `this`
pointers, or persisted references.

## 1. Objective

Lower the full M3/M4 HIR surface that M4 deferred: enums, subranges, named-value types, structs,
static and conformant arrays, strings, references, classes, FBs, interfaces, methods, standard
functions, and the standard FB library. Add the host/runtime substrate those values need
(place descriptors, type/object descriptors, vtable cells, string and math helpers) while keeping
the M4.5 identity, manifest, swap, and rollback contracts unchanged.

Exit criteria (from the project plan):

- OOP conformance cases (inheritance, overriding, interface dispatch, assignment attempt) pass.
- Static arrays and conformant `ARRAY [*]` parameters pass bounds/aliasing tests; dynamic
  locals/globals/returns are rejected with explicit diagnostics.
- UTF-8 vectors (valid/invalid sequences, counts, byte lengths, 4-byte scalar indexing, slicing,
  emoji/supplementary scalars) pass.
- Standard timer/counter/edge/bistable FBs pass deterministic tests with a mock clock; all timers
  in one scan see the same latched cycle time.
- Online-change tests cover FB/class state, aggregate roots, interfaces, and preserved
  descriptors; `POINTER TO` tests cover arithmetic, indexing, `SIZEOF`, null traps, and the
  rejection of persistent pointer fields.

## 2. Scope

### 2.1 In scope

| Area | Work |
| --- | --- |
| Types | Enums, subranges, named-value types, structs, static arrays, conformant arrays, strings (`STRING`/`WSTRING`/`USTRING`), references, classes, FBs, interfaces, transient `POINTER TO` (`STC-POINTER`) |
| Expressions | Standard functions, FB/method/interface calls, `SUPER`, `REF`/`NULL`/deref, `?=`, array/struct/string initializers, aggregate places |
| Statements | `RefAssign`, `Attempt`, all statements on aggregate places |
| Parameters | Output and `VAR_IN_OUT` parameters, conformant array parameters, pointer parameters/results |
| OOP/FB | FB instance state, nested instances, methods, class instances, inheritance, interface values, runtime-owned descriptors/vtables, `THIS`/`SUPER` |
| Globals | Aggregate/string/reference globals and constant initializers |
| ABI | Place-descriptor services (append-only ABI 2.x), attempt helper, string/math/date helpers, aggregate retain leaves |
| Stdlib | All standard functions; `SR`, `RS`, `R_TRIG`, `F_TRIG`, `CTU`, `CTD`, `CTUD`, `TP`, `TON`, `TOF`; cycle-latched clock |
| State | Aggregate/string/FB retained leaves; file-backed state remains M6 |
| Build | Projected catalog/metadata extended with M5 roots, types, and descriptors; same artifact pipeline |
| Tests | Layout/type snapshots, clang acceptance, execution harnesses, hot-swap coverage, fuzz corpus |

### 2.2 Out of scope

- File-backed retained state, retained-state migration, and `stc_rt_state_*` enablement (M6).
- `stc.toml` project system, dependency graph, `stc run`, `stc test`, `stc fmt` (M6).
- Full task scheduling, priorities, and the mock-clock CLI (M6); M5 keeps one task per resource.
- Debug information, checkpoint hooks, typed debug access, forcing (M6.5).
- `__NEW`/`__DELETE`, dynamic allocation, dynamically sized locals/globals/results.
- Device tree, `VAR_ACCESS` metadata, and non-null I/O backends (M6).

## 3. Architecture summary

### 3.1 Type and layout layer

`stc-codegen::types` becomes the single canonical type/layout source:

- Canonical type keys follow GOT Artifacts §4.1 (`BOOL`…, `STRING[n]`, `ARRAY[l..u]OF(key)`,
  `REF_TO(key)`, `PTR_TO(key)`, `T:<type GotId>`).
- Named types derive their `GotId` from `(IdentityKind::Type, package, qualified path, "")`;
  layout is a separate value so tracked renames and compatible code-only changes keep identity.
- Layout is computed with natural alignment from the target pointer width; no LLVM
  `target datalayout`, struct offset, or pointer size is hard-coded in emitted IR.
- The same computed sizes/offsets feed the catalog, metadata, and retained-state metadata; they
  never appear as constants in generated IR, which uses LLVM types and GEP only.

`StorageType` distinguishes scalars, enums/subranges (base integer), structs, static conformant
arrays, strings, references/pointers, interfaces, objects, and `OVERLAP`. `IrType` gains
`Agg(Rc<str>)` for rendered LLVM aggregate types; `Value` stays cheap and cloneable.

### 3.2 Persistent roots

Root enumeration follows Q-042/Q-123–Q-129:

- scalars, enums, subranges, strings, scalar/string arrays, and `OVERLAP` values are one root;
- struct/FB/class fields recurse per field (`RootKind::NestedField`);
- arrays of aggregates recurse per element; every element field is a root;
- an object instance also gets per-object descriptor records and, where a handle is required, a
  host-owned object record;
- reference/interface values are roots in memory but are excluded from retained-state leaves.

Codegen resolves a persistent place to a **root path** (instance path + field/array segment
chain) and uses the canonical slot for that root. A program/FB/method body receives a
per-instance **root table** (`[N x ptr]`) indexed by the type's static leaf ordinal; callers
build the table from the active instance's slots. Struct values that span several roots are
materialized into a transient alloca on read and split back into roots on write.

### 3.3 Calls and objects

- All aggregate parameters and results pass by reference to caller-owned storage; the callee
  receives a pointer. Scalar, reference, and interface values pass by value.
- `VAR_IN_OUT` and `VAR_OUTPUT` parameters receive stable place descriptors or pointers as the
  type requires. A split persistent aggregate is copied to a temp, passed, and copied back.
- FB bodies are function cells called with the instance root table; inputs are written before
  the call and outputs read after it.
- Methods receive the object record handle/descriptor first.
- Class/FB/interface type descriptors, vtables, and object descriptors are runtime-owned records
  projected from the catalog. Vtable slots come from HIR `vtable` order and are not recomputed.
- Interface values are `{object-record pointer, descriptor pointer}`; `NULL` is all-zero.
- `?=` uses a descriptor-compatibility helper; failed attempts yield `NULL`.

### 3.4 References and pointers

- `REF_TO` values are place descriptors (`Null`, `Root`, `Direct`, `Stack`, `Object`) with a
  fixed layout defined in Decisions and GOT Artifacts.
- `REF(place)` constructs a descriptor; dereference calls the host resolver, which validates the
  descriptor (generation, null, type/layout) and returns storage. Null or invalid dereference
  traps `STC-TRAP_NULL_REF`/`STC-TRAP_RANGE`.
- `POINTER TO` is the transient `STC-POINTER` extension: a raw pointer used for locals,
  parameters, contiguous leaves, array elements, strings, and `OVERLAP` views. Pointer
  arithmetic/indexing is unchecked within the target object; null dereference traps. Persistent
  pointer fields and pointer↔integer conversions are rejected.
- Pointer→`REF_TO` goes through the host resolver and yields `NULL` on failure; pointer→
  object/interface is not allowed.

### 3.5 Standard library

- Standard functions are inline IR where semantics are exact (`ABS`, `MIN`, `MAX`, `LIMIT`,
  `SEL`, `MUX`, shifts/rotates, comparisons, `MOVE`) and host-service calls otherwise (strings,
  math, date/time, conversions).
- Standard FBs are compiled as production HIR from synthesized bodies in the reserved `STC_STD`
  scope; they have stable keys and hidden state fields. They read the latched `cycle_now_ns`
  service and never the live clock.
- Textual `stdlib/*.st` injection is not used in M5 because the M6 package graph is what makes a
  separate standard package resolvable and shadowable without duplicate-definition errors. The
  synthesized-HIR deviation is recorded in D-134.

### 3.6 Retained state

- Leaves flatten recursively through structs, arrays, strings, enums/subranges (as base
  integers), and FB/class instance state over host roots.
- Paths are readable (`Main.Timer.ET`), sorted; payload offsets are packed sizes, never LLVM
  struct offsets; saved/loaded through GEP chains with `align 1`.
- References, interfaces, pointers, `VAR_IN_OUT`, and descriptor/vtable pointers are excluded.
  Non-persistable leaves keep `STC-E-RETAIN-001`.
- The layout hash format is unchanged; the M5 leaf set and paths are the new content.

## 4. Phases

| Phase | Deliverable | Status |
| --- | --- | --- |
| 0 | Decisions D-128–D-136 and this plan | Complete |
| 1 | Type/layout substrate, aggregate IR builder, type/descriptor catalog records | Complete |
| 2 | Structs, enums, subranges, arrays, strings: values, roots, initializers, traps | Complete for values, roots, initializers, and traps; string search functions are phase 5 |
| 3 | References, `NULL`, deref, `RefAssign`, `?=`, output/`VAR_IN_OUT` params, `STC-POINTER` | References, `NULL`, deref, `RefAssign`, aggregate/out/in-out ABI, and conformant `ARRAY[*]` complete; `?=` needs phase 4 and `STC-POINTER` remains |
| 4 | Classes, FBs, methods, inheritance, interfaces, descriptors/vtables, `THIS`/`SUPER` | Class/FB fields, object copy, FB bodies/invocation, methods, inheritance, `THIS`/`SUPER`, and the object-table ABI are complete; interfaces, `?=`, and runtime descriptors remain |
| 5 | Standard functions, standard FB bodies, cycle-latched timers | Complete: all `StdFnKind` variants (including `LEFT`/`RIGHT`/`MID`/`INSERT`/`DELETE`/`REPLACE`/`FIND`), standard FB bodies, and deterministic clock tests; the HIR surface defines no date/time functions |
| 6 | Aggregate/string/FB retained-state leaves and metadata | Complete: per-leaf flattening, packed offsets, GOT-root save/load, and metadata paths |
| 7 | Execution/OOP/stdlib harnesses, clang coverage, fuzz, docs, full checks | Aggregate/OOP/standard-FB/retained-state harnesses, M5 hot-swap coverage, and M5 fuzz seeds are in place; final docs and the closing check pass remain |

### Phase 1 — type and layout substrate

- Add `types.rs`: canonical keys, named-type IDs, structural classification, sizes/alignments,
  layout hashes, LLVM type rendering, field trees, and root-leaf enumeration.
- Extend `ir.rs`: aggregate type values, aggregate load/store, multi-index `extractvalue`/
  `insertvalue`, `memcpy`, `switch`, and aggregate argument handling with structural checks.
- Project `Catalog.types`, `Catalog.descriptors`, and field records for every named M5 type;
  keep identity derivation in one place.
- Unit tests for keys, layout sizes/alignments, field offsets, layout hashes, and determinism;
  clang acceptance for aggregate IR.

### Phase 2 — aggregates and strings

- Preflight accepts M5 types; unsupported constructs narrow to M6-only features.
- Locals/params/globals/fields support aggregate and string storage; initializers and default
  values initialize all leaves exactly once.
- Lower aggregate assignment (per-field/root copy), `ArrayInit`, `StructInit`, string literals
  and copies; add bounds/capacity/UTF-8 traps.
- Add `stc_rt_str_*` helpers through the host API for `LEN`, `LEN_MAX`, `LEN_CODE_UNIT`,
  `LEFT`/`RIGHT`/`MID`, `CONCAT`, `INSERT`/`DELETE`/`REPLACE`/`FIND`, and UTF-8 validation.
- Extend the process image/metadata/retain projection for aggregate roots.

### Phase 3 — references, pointers, parameters

- Define the place-descriptor layout and append host services (`place_deref`, `place_make`,
  `attempt`, `resolve`); bump the ABI 2 minor version.
- Lower `REF`, `NULL`, dereference, `RefAssign`, `?=`, and reference parameters.
- Implement output bindings (`=>`, `NOT =>`), `VAR_IN_OUT` copies, and conformant array
  descriptors with rank bounds.
- Add the `STC-POINTER` front-end surface (lexer/parser/AST/sema/HIR), transient pointer
  lowering, arithmetic/indexing, `SIZEOF`, null traps, and persistent-pointer rejection.

### Phase 4 — OOP and FBs

- Emit object type records, type-info/vtable/object descriptor records, and their GOT slots.
- Lower class/FB field access through root paths and object records; build instance root tables.
- Lower methods, inheritance, overrides, `SUPER`, interface upcasts/`?=`, interface dispatch
  through descriptor/vtable cells, `THIS`, nested FB instances, and FB input/output binding.
- Extend the host to construct/validate descriptors, vtables, and object records and to enforce
  vtable-shape compatibility.

### Phase 5 — standard library

- Lower every `StdFnKind`; inline exact operations and call host helpers otherwise. Complete,
  including `LEFT`, `RIGHT`, `MID`, `INSERT`, `DELETE`, `REPLACE`, and `FIND` over fixed-capacity
  `STRING`/`WSTRING`/`USTRING` values. The current HIR surface has no date/time function kinds;
  date/time literals and conversions are covered by the scalar layer.
- Synthesize standard FB bodies (`SR`, `RS`, `R_TRIG`, `F_TRIG`, `CTU`, `CTD`, `CTUD`, `TP`,
  `TON`, `TOF`) with hidden state fields and latched-clock reads. Complete in `stdlib.rs`.
- Deterministic timer/counter/edge tests with `stc_rt_set_cycle_time_ns`. Complete in the
  `standard_function_blocks_run_under_a_c_harness` driver test; string functions are covered by
  `StringOps`/`WideOps`/`UnicodeOps` and a capacity-range trap case.

### Phase 6 — retained state

- Flatten aggregate/string/FB leaves with recursive paths and packed offsets. Complete in
  `config::retain_layout`, driven by `TypeSystem::root_leaves`.
- Save/load through GEP chains; exclude non-persistable values; keep the metadata schema and
  layout-hash policy. Complete: leaves reach their host roots through `GlobalLeaf`/`InstanceLeaf`
  GOT slots and the retain mover loads/stores whole leaf values at `align 1`.
- Round-trip tests for aggregate/string/FB leaves and cold/online policy tests. Complete for
  structs, arrays of structs, strings, FB state, and program-instance aggregate fields; file
  policy remains M6.

### Phase 7 — verification and documentation

- Codegen snapshots and clang verification for every new construct.
- Driver execution harnesses for OOP, arrays/strings, conformant arrays, timers, and retained
  state; online-change coverage for FB/class state, aggregate roots, and descriptors.
- Extend `fuzz_codegen` with M5 HIR; keep bounded output/time and determinism.
- Update README, Decisions, project plan/conformance status, architecture/runtime/IO/state/
  testing docs, and this plan; run all checks and commit.

## 5. Diagnostics

| Code | Meaning |
| --- | --- |
| `STC-E-CODEGEN-001` | out-of-profile construct or feature deliberately deferred to M6 |
| `STC-E-CODEGEN-002` | internal codegen failure / IR invariant violation |
| `STC-E-RUN-001/002` | unsupported scheduling / no runnable configuration |
| `STC-E-RETAIN-001/002`, `STC-W-RETAIN-001` | existing retain rules |
| `STC-TRAP_BOUNDS`, `STC-TRAP_RANGE`, `STC-TRAP_NULL_REF`, `STC-TRAP_INVALID_UTF8`, `STC-TRAP_CONVERSION` | runtime traps |

One definition/call-site error explains an unsupported feature; no per-type or per-function
diagnostic expansion. New compiler codes require a decision entry.

## 6. Verification

- `cargo test --workspace --locked` (including clang acceptance with `STC_REQUIRE_CLANG=1`).
- `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo fmt --all --check`,
  `python tools/ci/check_docs.py`, Markdown lint.
- Fuzz smoke for parser/analyzer/codegen plus the artifact targets.
- Real module builds and hot swaps on Windows x64 and Ubuntu x64 CI.

## 7. Definition of done

The project-plan M5 exit criteria plus:

- Every new root/callable/type/descriptor/instance/export has a deterministic identity and slot.
- Generated IR contains no persistent raw pointer, no module-owned persistent storage, and no
  target-specific offsets.
- Unsupported M6 constructs remain explicit diagnostics, not silent wrong IR.
- README, Decisions, project plan, conformance profile, and architecture/runtime/state/testing
  specs describe the implemented M5 surface truthfully.
