# M5 Implementation Plan — FBs, OOP, Aggregates, Strings, and Standard Library

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-25  
Status: Phase 0 written. Phases 1–7 and the `STC-POINTER` surface are implemented; phase 4b
(interfaces, runtime descriptors, `?=`) is specified in D-147–D-152 and §4b. Steps 4b.0
(contracts and documentation), 4b.1 (catalog, descriptors, and hidden roots), 4b.2 (host
registry and ABI 2.1), 4b.3 (object records and virtual dispatch), 4b.4 (interface values and
upcasts), and 4b.5 (`?=`, object references, and `REF_TO`-to-interface) are complete; 4b.6
(verification, online-change coverage, and documentation) remains.

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
  parameters, results, contiguous leaves, array elements, strings, and `OVERLAP` views. Pointer
  arithmetic/indexing is unchecked within the target object; null dereference traps. Persistent
  pointer fields, pointer↔integer conversions, and pointer targets that are not contiguous
  storage are rejected.
- Pointer→`REF_TO` builds a `Stack` place descriptor from the raw address (a null pointer yields
  a null descriptor); `REF_TO`→`POINTER` extracts the descriptor's storage pointer, resolves
  `Direct` descriptors through `io_image`, and yields `NULL` for null, object, `%I` input, or
  invalid descriptors. Pointer→object/interface is not allowed.

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
| 3 | References, `NULL`, deref, `RefAssign`, `?=`, output/`VAR_IN_OUT` params, `STC-POINTER` | References, `NULL`, deref, `RefAssign`, aggregate/out/in-out ABI, conformant `ARRAY[*]`, `STC-POINTER`, and the phase-4b reference extensions (`REF(object)`, object-reference attempts, `REF_TO`-to-interface) are complete |
| 4 | Classes, FBs, methods, inheritance, interfaces, descriptors/vtables, `THIS`/`SUPER` | Class/FB fields, object records, class handle/FB state assignment, FB bodies/invocation, methods, virtual dispatch, inheritance, `THIS`/`SUPER`, the object-table ABI, interface values/upcasts/dispatch, `?=`, object references, and NULL tests are complete |
| 5 | Standard functions, standard FB bodies, cycle-latched timers | Complete: all `StdFnKind` variants (including `LEFT`/`RIGHT`/`MID`/`INSERT`/`DELETE`/`REPLACE`/`FIND`), standard FB bodies, and deterministic clock tests; the HIR surface defines no date/time functions |
| 6 | Aggregate/string/FB retained-state leaves and metadata | Complete: per-leaf flattening, packed offsets, GOT-root save/load, and metadata paths |
| 7 | Execution/OOP/stdlib harnesses, clang coverage, fuzz, docs, full checks | Aggregate/OOP/standard-FB/retained-state/pointer/interface/attempt-reference harnesses, M5 hot-swap coverage, M5 fuzz seeds, and status documentation are in place; the final phase-4b verification pass (4b.6) remains |

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
  Complete: `POINTER TO`, `ADR`, `SIZEOF`, `REF_TO T(expr)`/`POINTER TO T(expr)`, pointer
  fields/parameters/results, typed arithmetic and indexing, `SIZEOF` constants, null traps, and
  the transient-storage/contiguous-target rules are implemented and covered by a C harness and a
  null-deref trap executable. `SIZEOF` is folded during code generation, so it is a constant in
  emitted IR but is not accepted in analyzer constant-expression positions.

### Phase 4 — OOP and FBs

- Emit object type records, type-info/vtable/object descriptor records, and their GOT slots.
- Lower class/FB field access through root paths and object records; build instance root tables.
- Lower methods, inheritance, overrides, `SUPER`, interface upcasts/`?=`, interface dispatch
  through descriptor/vtable cells, `THIS`, nested FB instances, and FB input/output binding.
- Extend the host to construct/validate descriptors, vtables, and object records and to enforce
  vtable-shape compatibility.

### Phase 4b — interfaces, descriptors, and `?=`

Status: steps 4b.0–4b.5 complete (2026-10-25); step 4b.6 planned. Decisions
D-147–D-152. Land the steps in order; each step keeps the manifest and descriptor formats at
version 1 and adds its tests with the code.

#### 4b.0 — Contracts and documentation

Complete (2026-10-25). Host ABI 2.1 is recorded in the constants and in GOT Artifacts §1/§7/§8,
Runtime and Execution §2, Architecture §4.6/§7, and Online Change and GOT §4.2/§8.3/§9; D-133
and D-131 are amended. The layouts, key convention, prototype/stub rules, and inline access
rules are fixed in GOT Artifacts §8.5 and Runtime and Execution §2.6. The conformance-profile
status for interfaces and `?=` stays deferred to 4b.6, when the code lands.

- Document the TYPEINFO, VTABLE, and object-record byte layouts, the hidden record-root key
  convention, interface-prototype function records, trap-stub cells, and inline access rules in
  GOT Artifacts §8 and Runtime and Execution §2. Descriptor metadata keeps the owning type's type
  key, layout hash, size, and alignment; the runtime record layout is separate and versioned.
- Record host ABI 2.1 (`HOST_ABI_MINOR = 1`) and update the ABI table constants and C-facing
  documentation. No new services are appended.
- Amend D-133, Architecture §4.6, and Online Change and GOT §4.2/§8.3 for resolved storage
  pointers, class handle assignment versus FB value copy, the class value/instance layout split
  and `PTR_TO(T:<class>)` field projection, non-retained class handle roots with retained
  owned-instance fields, unprojected `Object` descriptors, per-pair interface vtables, virtual
  class/FB dispatch, and the stack-record escape rule.
- Update the conformance profile's interface and `?=` status when the code lands.

#### 4b.1 — Catalog, descriptors, and hidden roots

Complete (2026-10-25). `types` projects the class value/instance split, `PTR_TO` handle keys,
object-record shapes, the base-first interface map, and per-interface vtable order; `module`
projects TYPEINFO/VTABLE descriptors, interface-prototype method records with trap-stub symbols,
and hidden `HANDLE`/`OBJREC` roots; `unit` emits trap stubs and fills fresh records and handles
from the slot table; `stc-got` accepts the hidden markers without a format change. Class-owned
instance fields remain ordinary retained leaves. Local class values still use the phase-4a inline
instance until 4b.3 introduces stack handles and records; class-typed parameters/results and
whole-value assignments remain explicit 4b.3 diagnostics rather than half-implemented handle
semantics. The 4b.1 runtime binds descriptor cells to zeroed placeholder storage until the 4b.2
registry lands.

- `stc-codegen::types`: compute the object-record layout (`ptr typeinfo` plus N leaf storage
  pointers), the base-first interface-map order (inherited indices fixed, new interfaces appended
  sorted by `GotId`), per-interface vtable slot order, class vtable cell order, and trap-stub
  signatures for interface prototypes.
- `stc-codegen::module`: project interface prototypes as `FunctionKind::Method` records with
  trap-stub symbols; project one TYPEINFO `DescriptorDecl` per class/FB and one VTABLE when the
  type has vtable entries (interface types need no descriptor); emit one hidden non-retained
  record root per persistent object
  (program fields, globals, nested instances, array elements) with a deterministic `#OBJREC`-style
  key and the declaration's source span. For class-typed persistent declarations also emit a
  non-retained handle root (internal `PTR_TO(T:<id>)` key) holding the reference value; FB
  declarations get no handle root. Set required-feature bits 2 and 3.
- Class reference layout: distinguish the class **value** layout (pointer-sized handle) from the
  class **instance** layout (field struct) in `TypeFacts`/`NamedType`/`llvm_storage`. Class-typed
  fields, array elements, locals, parameters, and results use the handle layout; the owned instance
  uses the instance layout and its own record/roots. Project class-typed `FieldRecord`s with
  `PTR_TO(T:<class id>)` keys while the source type stays the class, and keep `TypeRecord`
  size/align/layout hash as the instance layout. Emit the owned instance's field roots, hidden
  record root, and handle root separately, with arrays of classes getting one owned instance per
  element. FB-typed declarations stay inline. Exclude class handle roots from retained-state
  leaves and metadata payloads. Document the projection.
- Define the hidden record-root and class-handle key/root-kind convention for global objects,
  program-instance objects, nested instances, and array elements so it passes existing
  catalog/engine key validation without a schema change; add collision and validation tests.
- Expose descriptor slot bindings (`TYPEINFO`/`VTABLE` per type) alongside root and function
  targets so codegen can load typeinfo cells when constructing records, initializing handles, and
  upcasting; keep the descriptor metadata's type key/layout hash/size/align equal to the owning
  type's so engine classification is unchanged.
- `stc-codegen::unit`: emit trap-stub bodies for prototypes and extend the prepare entry to fill
  fresh record roots (typeinfo pointer plus leaf storage pointers) and fresh class handle roots
  (pointing at their owned instance record) from the active slot table.
- `stc-got`: verify engine classification and slot assignment cover the new records with no format
  change; add catalog, descriptor, and hidden-root tests and double-build determinism.

#### 4b.2 — Host registry and ABI 2.1

Complete (2026-10-25). ABI 2.1 was already recorded by 4b.0; this step implements the
`stc-runtime::descriptors` registry. TYPEINFO and class VTABLE records are keyed by their catalog
descriptor `GotId`; per-(object type, interface) tables are keyed by the two type identities
because they have no catalog identity (D-148). Prepare validates descriptor metadata, object
leaf counts against the hidden `OBJREC` roots, class slot counts, contiguous interface prototype
slots, base-first interface maps, and prototype trap-stub cells; it reuses a record's stable
address while its shape is compatible and quarantines removed or reshaped records. Shadow words
are built during prepare and patched at activation, and rollback restores previous words and
releases fresh records. `stc_module_prepare` now runs after descriptor cells are bound, so fresh
object records store real TYPEINFO addresses. Interface dispatch and `?=` remain 4b.3–4b.5 work.

- `stc-got::descriptor`, `stc-codegen::abi`, and `stc-runtime::host_api`: bump the minor to 2.1 and
  update the version constants; the service table and `REQUIRED_SIZE` stay unchanged because no
  services are appended.
- `stc-runtime::host` and `got`: allocate a descriptor registry keyed by `GotId`; build TYPEINFO,
  VTABLE, and per-(type, interface) vtable records from manifest function and type records at
  prepare; bind `SlotKind::Descriptor` cells through `Got::bind_descriptors`; validate descriptor
  size, alignment, type/layout identity, slot counts, base-first interface maps, and trap-stub
  cells.
- Order the candidate lifecycle so generated prepare can read descriptor cells: build the
  descriptor registry, bind GOT descriptor/function/root cells, then call `stc_module_prepare`
  (which fills fresh records and class handles), then activate and patch descriptor entries.
- Preserve stable descriptor addresses for compatible shapes; build shadow contents during
  prepare and patch entries and maps at activation; reject shape changes while a live object or
  interface depends on them; release fresh records on rollback.
- Runtime tests: descriptor-cell binding, malformed or missing descriptors, preserved interface
  values across a warm swap, vtable-shape rejection, feature-bit rejection by older hosts, and
  object-record preserve/allocate/rollback.

#### 4b.3 — Object records and virtual dispatch

Complete (2026-10-25). Object values are record handles. `unit` and `place` allocate local
class/FB instances, handle slots, and records on the stack, read and write persistent objects
through their hidden `HANDLE`/`OBJREC` roots, treat `THIS` as the record handle, and apply the
handle/record parameter rules. Class `:=` copies the handle, FB `:=` copies state, and method/FB
bodies receive `(handle, table, ...)`. `oop`/`call` dispatch every class/FB method through
TYPEINFO and the class VTABLE; `SUPER` calls the base function cell directly. `sema` rejects
class handles that refer to activation-owned records from escaping through results, persistent
storage, or `OUT`/`VAR_IN_OUT` parameters. The C harness covers base-typed virtual overrides,
handle aliasing/orphaning, class arrays, nested local/persistent objects, recursion,
`THIS`/`SUPER`, class/FB parameters and copy isolation, and null dispatch traps; the existing
C harness covers FB fields and counters.

- `stc-codegen::unit` and `place`: object values become record pointers; local class instances
  (owned instance, handle slot, record) and local FB instances and records are stack allocas;
  persistent records and class handles come from hidden roots; class field/array-element access
  loads the handle and then uses the referenced record and root table; FB fields stay inline.
  `THIS` is the record handle; class `OUT`/`VAR_IN_OUT` parameters receive a pointer to the
  caller's handle slot; FB `OUT`/`VAR_IN_OUT` parameters receive the instance record/state and use
  its tail table.
- Assignment: class `:=` copies the handle, orphaning the destination's owned instance but
  leaving it rooted; FB `:=` copies fields. Class `IN` parameters are handles, so assigning the
  parameter only rebinds the callee handle; FB `IN` parameters use caller copies. Assignment
  through a dereferenced `Object` reference copies state into the referenced object.
- `stc-codegen::oop`, `call`, and `expr`: method and FB body ABI `(handle, root table, ...)`;
  virtual class/FB method calls use TYPEINFO, VTABLE, and the slot cell; `SUPER` calls the direct
  base cell; interface calls use the per-pair vtable.
- `stc-sema`: extend the pointer-escape rule to class handles and interfaces that refer to stack
  records: reject returning them, storing them persistently, or writing them through `OUT`/
  `VAR_IN_OUT` parameters; keep the `THIS` type and assignability rules consistent.
- Tests: virtual override through base-typed values, class reference assignment/aliasing and
  downcasts through handles, class handle layout/arrays with per-element owned instances,
  retention of owned-instance fields after handle reassignment, local/persistent/nested objects,
  recursion, `THIS`/`SUPER`, class and FB parameters and copy isolation, arrays of objects, and
  null-receiver dispatch traps.

#### 4b.4 — Interface values and upcasts

Complete (2026-10-25). Interface values are 16-byte `{record, per-(object type, interface)
vtable}` pairs passed and returned by value; interface-typed fields, globals, program fields, and
arrays are single roots. Class/FB-to-interface upcasts use the compile-time base-first map index
and its runtime base offset (a null class handle yields the null interface), interface-to-interface
upcasts search the runtime TYPEINFO interface map by interface `GotId`, and `= NULL`/`<> NULL`
compare the record pointer while identity comparison stays an explicit diagnostic. `oop`
dispatches interface calls through the pair's vtable with `(record, record + 8, ...)`, and `sema`
rejects interfaces that refer to activation-owned records escaping through results, persistent
storage, or `OUT`/`VAR_IN_OUT` parameters. The runtime's object-record leaf counting now follows
the compiler's single-root classification for array fields, so arrays of interfaces, handles, or
scalars count once. The C harness covers dispatch across implementors, `EXTENDS`, interface
parameters/results/fields/globals/arrays, FB implementors, `OUT`/`VAR_IN_OUT` bindings, and NULL
behavior; clang accepts the generated interface IR.

- `stc-codegen::types` and `call`: interface ABI by value `{ ptr, ptr }`; stop classifying
  interfaces as contiguous aggregates for parameter passing.
- `stc-codegen::place` and `expr`: class/FB-to-interface upcasts use the compile-time base-first
  map index (valid for derived runtime objects because inherited entries keep their indices);
  interface-to-interface upcasts use a bounded search; implement `= NULL` and `<> NULL`.
- Cover interface parameters, results, fields, globals, and arrays, plus FB-implemented
  interfaces.
- Tests: dispatch across implementors, `EXTENDS`, interface parameters/results/arrays, NULL
  behavior, clang acceptance, and snapshots.

#### 4b.5 — `?=`, object references, and `REF_TO`-to-interface

Complete (2026-10-25). `stmt`/`oop` lower `?=` inline for class, interface, and reference
destinations: class destinations compare the source object TYPEINFO's identity and base chain,
interface destinations search the base-first interface map (including `EXTENDS`), class
destinations from interface values compare the record TYPEINFO, and reference destinations
compare the referenced object's descriptor; a null source or referenced object fails without
trapping and every failure stores the all-zero `NULL` value. `REF(object place)` builds an
`Object` descriptor whose data field is the record handle and whose descriptor field is the
TYPEINFO pointer; `^` yields the class handle or FB state, field and array access through it walk
the record's leaf table, whole-object assignment copies state through the record and never
rebinds the source variable's handle, and a null class value yields the null descriptor.
`REF_TO` class/FB values convert to interfaces by searching the referenced object's interface
map, yielding the null interface for null or incompatible objects; the same descriptor check
backs reference-to-reference attempts. Sema allows `REF_TO` class/FB-to-interface assignment and
interface attempts, and pointer-to-object/interface conversions and `ADR` of objects stay
rejected. The C harness covers attempt success/failure in both directions, null sources,
`REF(object)` dereference/field writes/state copy, and `REF_TO`-to-interface success and
failure; clang accepts the generated attempt/reference IR.

- `stc-codegen::stmt` and `expr`: lower `Attempt` with base-chain and interface-membership checks;
  assign all-zero on failure; never trap.
- `stc-codegen::reference`: `REF(object)` builds `Object {handle, descriptor}`; dereference and
  assignment through it use object semantics; object-reference attempts use descriptor
  compatibility; `REF_TO`-to-interface yields null for null or incompatible sources; keep
  pointer-to-object/interface conversions and `ADR` of objects rejected.
- Tests: upcast and downcast attempts, failed attempts, null sources/destinations (attempt fails
  without trapping), `REF(object)` dereference, whole-object assignment through a dereferenced
  reference (state copy, no handle rebind), `REF_TO`-to-interface success and failure, and
  rejected pointer conversions.

#### 4b.6 — Verification, online change, and documentation

- Update the phase-4 rejection tests in `stc-codegen/tests/codegen.rs` (assignment attempt; the
  interface-dispatch rejection test was replaced by success coverage in 4b.4) and
  `stc-driver/tests/execution.rs` (unsupported constructs) to success coverage; keep explicit
  diagnostics for genuinely unsupported combinations.
- Add C-harness coverage for the full surface, hot-swap tests for preserved interfaces, descriptor
  identity, vtable-shape rejection, fresh-record initialization, and rollback, and fuzz seeds for
  malformed TYPEINFO/VTABLE/interface-map inputs.
- Update README, Decisions, project plan/conformance status, Architecture/GOT/Runtime docs, and
  this plan; run `cargo test --workspace --locked`, clippy, fmt, docs, markdown lint, and the
  clang-required CI job; double builds remain byte-identical.

Exit criteria: interface dispatch, virtual overrides, class reference assignment/aliasing and
downcasts through handles, `?=` success and failure, `REF(object)` and `REF_TO`-to-interface,
arrays of objects, preserved interface values and class handles across a warm swap, and
vtable-shape rejection all pass; no descriptor or vtable is module-owned; generated code contains
no heap allocation or raw persistent object pointer.

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

- Codegen snapshots and clang verification for every new construct. Clang acceptance and
  execution coverage are in place for the implemented surface, including an M5 determinism test.
- Driver execution harnesses for OOP, arrays/strings, conformant arrays, timers, pointers, and
  retained state; online-change coverage for FB/class state, aggregate roots, and descriptors.
  FB and aggregate state preservation is covered; interface/descriptor preservation waits on
  phase 4b.
- Extend `fuzz_codegen` with M5 HIR; keep bounded output/time and determinism. The target already
  generates arbitrary analyzed HIR; local M5 seeds were used during development but the corpus
  directory is gitignored.
- Update README, Decisions, project plan/conformance status, architecture/runtime/IO/state/
  testing docs, and this plan; run all checks and commit. The status docs now distinguish the
  implemented M5 surface from the deferred items below.

### Remaining M5 work

- **Phase 4b — interfaces, descriptors, `?=`.** Step 4b.4 makes interface values 16-byte
  `{record, per-pair vtable}` pairs passed and returned by value, covers interface
  parameters/results/fields/globals/arrays and FB implementors, upcasts class/FB values through
  the compile-time base-first map index, upcasts interface values through a bounded map search,
  implements `= NULL`/`<> NULL`, dispatches interface calls through the pair vtable, and extends
  the stack-record escape rule to interfaces. Step 4b.0 records host ABI 2.1 and the inline
  record contracts (GOT Artifacts §8.5, Runtime and Execution §2.6). Step 4b.1 projects the
  catalog: one TYPEINFO per class/FB, class VTABLES, interface prototype records with trap stubs,
  and hidden `HANDLE`/`OBJREC` roots; prepare fills fresh records and handles. Step 4b.2 builds
  and validates the host-owned descriptor registry before prepare, keeps compatible record
  addresses stable across a warm swap, patches entries at activation, quarantines removed
  descriptors, and releases fresh records on rollback. Step 4b.3 makes object values record
  handles and dispatches class/FB methods virtually through TYPEINFO and the class VTABLE.
  Step 4b.5 lowers `?=` inline for class/interface/reference destinations, implements
  `REF(object)` descriptors with record/state dereference and state-copy assignment, and converts
  `REF_TO` class/FB references to interfaces with null on null or incompatible input. Interface
  identity comparison remains an explicit diagnostic; step 4b.6 updates the remaining rejection
  tests and documentation. The resolved decisions are D-147–D-152.
- **References to split persistent aggregates** remain rejected because they have no single
  stable storage location; contiguous transient aggregates and single-root persistent values are
  supported (D-144). A multi-root descriptor with host-side resolution is recorded as a possible
  v2 extension in the [project plan](Specification/00-Project-Plan.md) §13. Pointer targets obey
  the same rule: `ADR` of a split persistent aggregate is rejected, while leaves, single-root
  values, and transient aggregates are supported.

#### Phase 4b decision questions

All questions below were resolved on 2026-10-25. Record the resolutions as decision entries
(D-147 onward) before implementing the affected part. Q4B-1–Q4B-4 were the blocking set; the
remaining questions follow in the listed order.

**Object representation (blocking)**

- **Q4B-1 — What is an object value at runtime?** (a) Full D-133 record
  `{typeinfo, leaf storage pointers…}` with record-pointer values; (b) a record whose tail is the
  existing `[N x ptr]` root table, keeping the body ABI; (c) transient root-table pointers with no
  dynamic type checks. Also decide whether records hold storage pointers (swap-stable) or GOT cell
  pointers (literal D-133 wording). **Resolved:** full D-133 record with resolved storage
  pointers; D-133's "field-root cell pointers" wording is clarified accordingly.
- **Q4B-2 — Is dynamic type identity required on every object value?** Full `?=` semantics
  (class downcasts, interface-to-interface attempts) need typeinfo on locals and parameters too;
  otherwise only statically safe attempts are implemented and dynamic cases are rejected.
  **Resolved:** full dynamic checks; every class/FB value carries typeinfo.
- **Q4B-3 — Object-record ownership and escape policy.** Persistent instances are host-owned and
  stable across swaps; locals/parameters need stack or transient records. Decide how interface or
  handle values referencing a local record are prevented from escaping into persistent storage,
  and define rollback/removal behavior. **Resolved:** persistent instances use host-owned records
  allocated once at prepare; locals use per-activation stack records and object parameters are
  caller handles; generated code performs no heap allocation; handles or interfaces to stack
  records are rejected from escaping; recursion is allowed.
- **Q4B-4 — Method/FB body ABI, `THIS`, `SUPER`, object parameters.** Pass record + root table,
  record only (table at a fixed offset), or keep table-only; make `THIS` handle-valued and
  `SUPER` use the base view. Resolve object input parameters (they pass as `ptr` but cannot be
  read today) and the by-value/by-pointer rule for object, reference, and interface values.
  **Resolved:** handle + root-table arguments; `THIS` is the handle; object `IN` parameters are
  handles, with the caller copying when IN isolation is required.

**Descriptor model (blocking before catalog/host work)**

- **Q4B-5 — Descriptor kinds and runtime layouts.** Define TYPEINFO, VTABLE, and OBJECT record
  layouts, versioning, and whether OBJECT is per type (record shape) or per instance; catalog
  descriptor records carry metadata only today. **Resolved:** hidden non-retained record roots for
  persistent objects; the host allocates their storage and generated prepare fills typeinfo and
  leaf storage pointers for fresh objects.
- **Q4B-6 — How do vtable entries and interface maps reach the host?** Extend
  `DescriptorRecord`/`DescriptorDecl` with entries (manifest format impact), reconstruct in the
  host from function/type records, add a prototype/method-map record, or use method-key lookup.
  **Resolved:** interface prototypes are projected as function records with trap-stub cells; the
  host matches prototype name+signature to implementors and embeds per-(object type, interface)
  vtables in the type's TYPEINFO. No manifest format change.
- **Q4B-7 — Interface slot model.** Sema assigns interface-prototype and class/FB slots in
  separate spaces. Choose per-(object type, interface) vtables in interface slot order, a unified
  sema slot space, host-built interface vtables, or runtime key lookup; define `vtable_shape`.
  **Resolved:** per-(object type, interface) vtables in interface slot order; sema slot assignment
  and `vtable_shape` remain per type.
- **Q4B-8 — Descriptor identity.** The key grammar names one descriptor per owner type path and
  kind, so per-interface vtables and per-instance object descriptors cannot be named. Choose an
  embedded map, a grammar/format extension, or host-internal records without catalog identities.
  **Resolved:** TYPEINFO and VTABLE are projected per owner type only; no per-pair or per-instance
  descriptor identities. `DescriptorKind::Object` stays defined but unprojected in M5: object
  records are host storage, not catalog descriptors.
- **Q4B-9 — Descriptor lifecycle across online change.** Preserved interface values need stable
  descriptor addresses while vtable entries must reach the new generation's cells. Choose in-place
  entry patching at activation, per-generation descriptors plus preserved-root rewriting, or
  stable dispatch cells; define shape-change rejection and live-dependency tracking.
  **Resolved:** descriptors keep stable addresses; prepare builds shadow contents and activation
  patches method cells/interface maps under the cycle boundary.
- **Q4B-10 — Descriptor registry and ABI.** Define host registry/validation, descriptor GOT cell
  binding, required-feature bits 2/3, and whether `?=` uses an appended ABI 2.1 service or inline
  descriptor reads. **Resolved:** generated code reads the TYPEINFO/vtable/record layout inline;
  the host ABI minor is bumped to 2.1 for that layout and for the required-feature bits.

**Interface semantics and codegen**

- **Q4B-11 — Interface ABI and comparisons.** Pass `{ptr, ptr}` by value or by pointer; define
  results, `VAR_IN_OUT`/`OUT`, fields, roots; define null tests and whether identity comparison is
  supported (codegen currently rejects interface comparisons). **Resolved:** pass by value
  `{ptr, ptr}`; support NULL tests only for interfaces and class/FB handles; no identity
  comparison in M5.
- **Q4B-12 — Upcast mechanics.** Precompute the interface entry index in the static source type's
  map or look it up at runtime; define interface-to-interface upcasts and any explicit conversions.
  **Resolved:** class/FB-to-interface upcasts use a compile-time map index; interface-to-interface
  upcasts and dynamic attempts use a bounded search over the object type's base-first interface
  map. Inherited interface-map entries keep their indices so the static index stays valid for
  derived runtime objects; newly implemented interfaces are appended sorted by `GotId`.
- **Q4B-13 — `REF` of object places and the `Object` descriptor kind.** Implement
  `Object {handle, descriptor}` and `REF_TO`→interface now, or reject `REF` of object-typed places
  in M5 and record the deviation. **Resolved:** implement `Object {handle, descriptor}`;
  `REF(object)` yields it, dereference yields the object, object attempts use the descriptor, and
  `REF_TO`→interface follows Q-152/Q-171.
- **Q4B-14 — Virtual dispatch for class/FB methods.** Implement vtable dispatch (all methods or
  virtual/abstract only) or keep static resolution for M5 and document that dynamic dispatch is
  interface-only. **Resolved:** virtual dispatch for all class/FB methods via VTABLE; `SUPER`
  stays a direct base call.
- **Q4B-15 — Scope matrix and verification.** Confirm which combinations are in scope (interface
  parameters/results/globals/arrays, FB interfaces, interface `EXTENDS`, attempts in both
  directions, `THIS` as interface, arrays of objects) and the harness, hot-swap, fuzz, and doc
  updates that close the phase. **Resolved:** full documented surface, including interface
  parameters/results/fields/globals/arrays, FB interfaces, interface `EXTENDS`, both attempt
  directions, `THIS` as interface, arrays of objects, and `REF_TO` objects/interfaces.
- **Q4B-16 — How should `:=` between class-typed values behave?** The analyzer classifies class
  assignment as a reference assignment (`RefAssign`, D-069), while phase 4a codegen copies fields.
  **Resolved:** class values are references (handle copy); each class-typed variable or field owns
  an embedded instance whose field roots remain retained; FB values remain embedded and `:=`
  copies state. Assignment through a dereferenced `Object` reference copies state into the
  referenced object.

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
- Generated IR contains no persistent raw object-storage or code pointer, no module-owned
  persistent storage, and no target-specific offsets. Class handles are pointers to host-owned
  records and are the only persistent object references.
- Unsupported M6 constructs remain explicit diagnostics, not silent wrong IR.
- README, Decisions, project plan, conformance profile, and architecture/runtime/state/testing
  specs describe the implemented M5 surface truthfully.
