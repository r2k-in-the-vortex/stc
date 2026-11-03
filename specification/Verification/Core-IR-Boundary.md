# Emitted IR Boundary — Phase 1 Freeze

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-11-03
Boundary version: `1` (Phase 1 M5)
Status: Frozen by Phase 1 M5 under the owner decision of 2026-11-02. This document fixes the
boundary of the LLVM IR that `stc-codegen` may emit into a production release module, and fixes
the D-V6 first end-to-end subset. It is a specification record at level S: it defines syntax and
scope and makes no semantic, refinement, or preservation claim. The reviewed semantics remain
`Semantics.md` version `1.0`; the reviewed target fragment remains the P0.4 record in
[Core IR Coverage](Core-IR-Coverage.md), which is the core slice of this boundary. Phase 5 owns the
Lean `IRFrag`/`WellFormed` extension to the full boundary; Phase 7 owns codegen preservation; Phase
9 owns the D-V6 end-to-end theorem. No component is claimed above V0 in Phase 1.
Parent: [Verification Program Plan](VerificationProgramPlan.md) Phase 1
Semantics reference: [stc Semantics — Phase 0 Core](Semantics.md)
Core target fragment: [Core IR Coverage](Core-IR-Coverage.md)
Source coverage: [Core Syntax Coverage](Core-Coverage.md)

## 1. Boundary definition and authority

A construct is **inside the boundary** only when the `stc-codegen` emitter in this tree can produce
it in a production release module. The boundary is not "any valid LLVM IR", not "anything clang
accepts", and not anything a future lowering might want. A construct that is not listed here is
outside the boundary and must be rejected by the Phase 5 textual checker and by the Phase 7
preservation proof until this document is revised.

The boundary was derived from the emitter, not from LLVM documentation: the instruction renderers
and value constructors in [`ir.rs`](../../crates/stc-codegen/src/ir.rs), the expression, statement,
and place lowerings in [`expr.rs`](../../crates/stc-codegen/src/expr.rs),
[`stmt.rs`](../../crates/stc-codegen/src/stmt.rs), [`place.rs`](../../crates/stc-codegen/src/place.rs),
and [`call.rs`](../../crates/stc-codegen/src/call.rs), the standard function-block bodies in
[`stdlib.rs`](../../crates/stc-codegen/src/stdlib.rs), the OOP/reference lowerings in
[`oop.rs`](../../crates/stc-codegen/src/oop.rs) and [`reference.rs`](../../crates/stc-codegen/src/reference.rs),
the module assembly in [`unit.rs`](../../crates/stc-codegen/src/unit.rs), the ABI constants in
[`abi.rs`](../../crates/stc-codegen/src/abi.rs), and the structural verifier in
[`verify.rs`](../../crates/stc-codegen/src/verify.rs). The committed snapshot
[`library.ll`](../../crates/stc-codegen/tests/snapshots/library.ll) and the verifier tests in
[`module_ir.rs`](../../crates/stc-codegen/tests/module_ir.rs) are executable evidence for the
production forms.

The Phase 1 executable enforcement of the GOT, entry-point, descriptor, and function-table rules is
the Rust structural verifier `stc_codegen::verify::verify` (V0 evidence). It is not a proved
checker. The Phase 5 Lean checker is the proved counterpart; until it exists, this document is the
normative scope statement, not a claim that emitted modules are well formed.

The production release module is the default `ModuleOptions` module artifact: no test-only
definitions, no coverage instrumentation, no debug information, and no debug checkpoint hooks. The
launcher executable produced by [`launcher.rs`](../../crates/stc-codegen/src/launcher.rs) is a
separate artifact and is outside this boundary (§12).

## 2. Relationship to the Phase 0 core and later phases

| Record | Scope | Status and owner |
| --- | --- | --- |
| [Semantics.md](Semantics.md) §1 | Frozen core source subset and target fragment | Reviewed and frozen at `1.0` (P0.7, 2026-11-02); the only S-level claim |
| [Core IR Coverage](Core-IR-Coverage.md) | P0.4 definition-to-emission map for the core module | Reviewed 2026-10-31; core slice of this boundary |
| This document | Full production-module boundary across all source features the emitter supports, plus the D-V6 subset | Frozen by Phase 1 M5, 2026-11-03; level S, no claim |
| D-V6 subset (§11) | Phase 0 core source subset plus the core target fragment and GOT globals | Owner-approved 2026-11-02; scope fixed here, theorem owned by Phase 9 |
| Lean `IRFrag`/`WellFormed` extension | Proved structural model of this boundary | Phase 5 |
| Codegen preservation | Source-to-IR refinement over the agreed subset | Phase 7, extended subset by subset |
| `pipeline_correct` | End-to-end preservation for D-V6 | Phase 9, target V4 |

This document does not reinterpret `semantics_version` `1.0`. Where this boundary is wider than
the Phase 0 core, the extra forms are recorded as later-phase scope; they carry no Phase 0 theorem
and no new claim. Where the two overlap, [Core IR Coverage](Core-IR-Coverage.md) remains the
definition-to-emission map.

## 3. Module envelope

A production module is rendered in a fixed order with deterministic ordering: header, optional
types, globals, declarations, functions. The header is:

```llvm
; stc <crate-version> - profile stc-2013-oop-utf8-v1
source_filename = "<artifact-name>"
target triple = "<triple>"        ; optional; present only when the driver selects a target
```

The module envelope has these properties:

- No named type definitions are emitted today. The builder exposes `Module::add_type`, but no
  emitter calls it; aggregate types are rendered inline. Named type definitions are outside
  version `1` of the boundary and require a revision if a lowering starts using them.
- No module-level inline assembly, comdat, alias, ifunc, global annotations, module flags,
  personality function, garbage-collector name, partition, or source-level metadata is emitted in
  a production module.
- Every function is rendered as `define {attrs}{ret} @{name}({params}) uwtable { ... }`. The
  `uwtable` attribute is emitted unconditionally as an ABI marker. No calling convention, address
  space, `personality`, `gc`, `prefix`, `prologue`, or unwind attribute is emitted.
- Module entry points use `external dso_local dllexport` attributes. Internal helper functions and
  function bodies use `internal`. Globals use `private`, `internal`, or `external dso_local
  dllexport` as listed in §7.1. No `hidden` or `protected` visibility is emitted.
- SSA temporaries are named `%tN`; parameters are named `%aN` except the explicitly named accessor
  and entry-point parameters (`%out_size`, `%out_count`, and the ABI parameter names). Basic-block
  labels are `bbN`.
- Global and function order is deterministic (ordered maps), so two compilations of the same input
  produce byte-identical IR except for header version text.

## 4. Types

| Type form | Emitted for | Boundary status |
| --- | --- | --- |
| `void` | Procedure and entry-point returns | Included |
| `i1` | `BOOL` in SSA, comparison results, conditions | Included |
| `i8`, `i16`, `i32`, `i64` | Scalar storage and arithmetic; `BOOL` is `i8` in memory | Included |
| `ptr` | Opaque pointers: locals, GOT cell addresses and values, object references, calls | Included |
| `float`, `double` | `REAL`, `LREAL`, floating-point temporaries | Included (outside D-V6) |
| `{ iN, i1 }` | Result of `llvm.*.with.overflow.iN` | Included |
| `[N x T]` | Fixed arrays, string buffers, descriptor/metadata bytes, function table | Included |
| `{ T, ... }` and `{}` | Struct literals; the empty struct for empty aggregate values | Included (outside D-V6) |
| `{ i32, i32, i64, i64, i64, ptr, ptr }` | Place descriptor (`PLACE_DESCRIPTOR_LLVM`) | Included (outside D-V6) |
| `{ ptr, ptr }` | Interface value: handle and dispatch pointer | Included (outside D-V6) |
| `{ ptr, i64, [rank x { i64, i64 }] }` | Conformant array: data, length, bounds | Included (outside D-V6) |
| `{ i32, [capacity x iN] }` | String value: length and fixed capacity buffer | Included (outside D-V6) |
| `[N x ptr]` object record | Object instance record: header plus field cells | Included (outside D-V6) |

The boundary uses opaque pointers exclusively. It excludes typed pointers, pointer address spaces
other than the default, vector and scalable-vector types, `x86_mmx`, `token`, `metadata` as a value
type (debug-only metadata attachments are outside the boundary), `half`, `bfloat`, `fp128`,
`x86_fp80`, `ppc_fp128`, function types used as first-class values, and named struct types. Integer
widths outside `1`, `8`, `16`, `32`, and `64` are not emitted.

`BOOL` is stored as `i8` and normalized to `i1` in SSA. `CHAR`, `WCHAR`, and `UCHAR` use unsigned
integer layouts (`i8`, `i16`, `i32`). `TIME` and `DATE` use `i64`. The Phase 0 core covers the
integer, bit-string, and character layouts only; `REAL`, `LREAL`, `TIME`, and `DATE` are inside the
full boundary but outside D-V6.

## 5. Instructions

| Instruction form | Emitted for | Boundary status |
| --- | --- | --- |
| `alloca <ty>` | Scalar and aggregate stack temporaries, root-pointer arrays, field-pointer arrays | Included; static type only, never a dynamic element count |
| `load <ty>, ptr <p>[, align N]` | Locals, GOT cell addresses/values, root storage, object fields | Included |
| `store <ty> <v>, ptr <p>[, align N]` | Locals, root storage, object fields, function-table setup | Included |
| `getelementptr [inbounds] <pointee>, ptr <b>, <indices>` | Slot-table entries, array elements, struct fields, descriptor/vtable fields, byte offsets | Included; `inbounds` only where the aggregate path is known in range |
| `add`, `sub`, `mul`, `and`, `or`, `xor`, `shl`, `lshr`, `ashr` | Integer and bit-string arithmetic, masks, shifts | Included |
| `sdiv`, `udiv`, `srem`, `urem` | Exact integer division and remainder | Included |
| `fadd`, `fsub`, `fmul`, `fdiv`, `frem`, `fneg` | Floating-point arithmetic | Included (outside D-V6) |
| `icmp eq`, `ne`, `slt`, `sle`, `sgt`, `sge`, `ult`, `ule`, `ugt`, `uge` | Scalar comparisons, null and range checks | Included |
| `fcmp oeq`, `une`, `olt`, `ole`, `ogt`, `oge`, `ord` | Floating-point comparisons and NaN checks | Included (outside D-V6) |
| `select i1 <c>, <a>, <b>` | `SEL`, `MIN`, `MAX`, `LIMIT`, `MUX`, null/range selection | Included |
| `zext`, `sext`, `trunc` | Integer and character conversions, shift-count narrowing | Included |
| `ptrtoint` | Raw-pointer comparison in the reference/pointer subset | Included (outside D-V6) |
| `sitofp`, `uitofp`, `fptosi`, `fptoui`, `fpext`, `fptrunc` | Numeric conversions | Included (outside D-V6) |
| `phi <ty> [ v, %pred ], ...` | Short-circuit `AND`/`OR` merges, standard FB state | Included; emitted at block start by the lowering |
| `extractvalue <agg>, <idx>` | Checked-arithmetic result and overflow flag; aggregate fields | Included |
| `insertvalue <agg>, <v>, <idx>` | Interface/descriptor pairs, object and reference pairs | Included (outside D-V6) |
| `call <ret> @name(<args>)` | Intrinsics, `memcmp`, internal helpers, retain helpers | Included; direct callee must be a known intrinsic, `memcmp`, or an internal helper |
| `call <ret> <ptr>(<args>)` | User calls and program bodies through `Function` cells; host services through `Import` cells; vtable code addresses | Included; callee must originate from a `Function`, `Import`, or vtable code cell |
| `call void @llvm.memcpy.p0.p0.i64(ptr, ptr, i64, i1 false)` | Aggregate and string copies | Included (outside D-V6) |
| `extractelement`, `insertelement`, `shufflevector` | None | Excluded: no vector lowering |
| `bitcast`, `inttoptr`, `addrspacecast` | None | Excluded |
| `freeze`, `undef`, poison-producing operations | None | Excluded |
| Atomic operations, `fence`, volatile loads/stores | None | Excluded |
| `invoke`, `landingpad`, `resume`, `catchswitch`, `catchpad`, `cleanuppad` | None | Excluded |
| `indirectbr`, `callbr` | None | Excluded |
| Inline `asm`, `va_arg`, `llvm.va_*` | None | Excluded |
| `llvm.assume`, lifetime/launder/experimental intrinsics, `llvm.stacksave`/`stackrestore` | None | Excluded |
| `switch` | None: `CASE` lowers to comparison branches | Excluded; builder-only, no emitter call site |
| Floating min/max and other libm-like intrinsics not listed in §6 | None | Excluded |

Binary operations carry no `nsw`, `nuw`, `exact`, or fast-math flags. Floating-point operations
carry no fast-math flags. Loads and stores carry `align` only when the emitter knows the
alignment. All calls use the default calling convention; no `tail`, `musttail`, `notail`,
operand bundles, or call-site attributes are emitted.

## 6. Calls, intrinsics, host services, and traps

### 6.1 Direct callees

| Callee | Signature | Emitted for |
| --- | --- | --- |
| `llvm.{s,u}{add,sub,mul}.with.overflow.iN` | `{ iN, i1 } (iN, iN)` | Checked arithmetic; `N` in `8`, `16`, `32`, `64` |
| `llvm.fshl.iN`, `llvm.fshr.iN` | `iN (iN, iN, iN)` | `ROL`/`ROR` with masked rotate count |
| `llvm.{pow,fabs,sqrt,log,log10,exp,sin,cos,tan,asin,acos,atan,atan2}.f32/.f64` | floating signature of the same arity | Floating-point standard functions |
| `llvm.memcpy.p0.p0.i64` | `void (ptr, ptr, i64, i1)` | Aggregate and string copies |
| `memcmp` | `i32 (ptr, ptr, i64)` | String comparison |
| `stc_pow_sN`, `stc_pow_uN` | `iN (iN, iN, i32)` | Integer exponentiation with overflow/range traps |
| `stc_retain_size`, `stc_retain_layout_hash`, `stc_retain_save`, `stc_retain_load` | compiler-defined helper signatures | Retained-state support |

The direct callee must be one of these forms or another function defined in the same module with
internal linkage. A direct call to a user function, a program body, a host service, or a root
symbol is outside the boundary.

### 6.2 Indirect callees and host services

Indirect calls go through a GOT cell value or a vtable code address, never through a root symbol or
the function-address table.

| Source of the callee | Cell kind or origin | Emitted for |
| --- | --- | --- |
| Function cell value | `Function` | User functions, program bodies, methods, interface trap stubs |
| Import cell value | `Import` | Host services |
| Vtable code address | `PointerOrigin::VtableCode` | OOP dynamic dispatch |

Host services are never declared by name in the module and never called directly. The import slot's
cell value is the host service address. The service table is ABI 2, with these IDs:

| ID | Service | Boundary status |
| --- | --- | --- |
| 1 | `trap` | Core; required for every trap |
| 2 | `cycle_now_ns` | Timers and standard function blocks |
| 3 | `monotonic_now_ns` | ABI-reserved; not imported by the current emitter |
| 4 | `local_date_ns` | ABI-reserved; not imported by the current emitter |
| 5, 6 | `alloc`, `free` | ABI-reserved; not imported by the current emitter |
| 7, 8 | `io_image`, `io_size` | Process image and `AT` bindings (outside D-V6) |
| 9, 10 | `state_load`, `state_save` | Retained state (outside D-V6) |
| 11 | `assert` | Test-only `STC_ASSERT`; outside the boundary |
| 12 | `set_cycle_time_ns` | Test build only; outside the boundary |
| 13 | `debug_check` | Debug build with checkpoints; outside the boundary |

In the production boundary, services 1, 2, 7, 8, 9, and 10 may be imported; services 3–6 are
ABI-reserved and are not imported by the current emitter; services 11–13 belong to test or debug
builds outside the boundary (§12). A production module still validates the `8 + 11 * 8`-byte
production prefix, which includes the slot for service 11 even though a production module does not
import it. Test and debug builds require larger prefixes and are outside the boundary.

### 6.3 Trap pattern

A trap is the imported `trap` service called with two `i32` arguments, the trap code and the
source span identifier, followed by `unreachable`:

```llvm
%trap = call ptr ...            ; the Import cell value for service 1
call void %trap(i32 <code>, i32 <span-id>)
unreachable
```

The trap service may receive codes `1` (`OVERFLOW`), `2` (`DIV_ZERO`), `3` (`RANGE`), `4`
(`BOUNDS`), `5` (`NULL_REF`), `7` (`CONVERSION`), and `10` (`UNIMPLEMENTED`). Code `9` (`ASSERT`)
is carried by the separate `assert` service, and codes `6` (`INVALID_UTF8`) and `8` (`WATCHDOG`)
are runtime-side codes that the current compiler does not emit. The core subset uses only
`OVERFLOW`, `DIV_ZERO`, and `RANGE`. The `assert` service has the shape `void (i32, i32, i32)`
(condition, code `9`, span) and is not followed by `unreachable`.

## 7. Globals and GOT access

### 7.1 Module globals

| Symbol | Linkage and visibility | Type and initializer | Boundary status |
| --- | --- | --- | --- |
| `stc_module_api` | `internal` | `ptr` `null`, mutable | Included; written by attach, cleared by detach |
| `stc_module_slots` | `internal` | `ptr` `null`, mutable | Included; written by attach, cleared by detach |
| `stc_retain_buffer` | `internal` | `[max(size, 1) x i8]` `zeroinitializer`, mutable | Included; always emitted, one byte when there is no retained state |
| `stc_module_descriptor_data` | `private` | `[N x i8]` byte string, constant, align 8 | Included |
| `stc_module_function_table` | `internal` | `[N x ptr]` symbol array, constant, align 8 | Included |
| `stc_metadata_json_data` | `private` | `[len x i8]` NUL-terminated byte string, constant, align 1 | Included |
| `stc_coverage_counters` | `external dso_local dllexport` | `[N x i64]` `zeroinitializer`, align 8 | Test/coverage build only; outside the boundary |

User `VAR_GLOBAL` roots have **no module storage**. Their storage is host-owned and reachable only
through the GOT. The global symbol names appear in the verification contract's root set so that a
regression which emits a direct reference is rejected, but no `@global` definition or declaration
for a user root appears in the module.

### 7.2 Slot-table access

Every GOT access uses the two-load pattern over the host-provided slot table:

```llvm
%slots = load ptr, ptr @stc_module_slots, align 8
%entry = getelementptr ptr, ptr %slots, i64 <slot>
%cell  = load ptr, ptr %entry, align 8
%value = load ptr, ptr %cell, align 8
```

The load through `%entry` yields the cell address; the load through the cell address yields the cell
value. The slot index is a constant in the core subset. The cell value is then used by kind:

| Cell kind | Cell value means | Permitted uses |
| --- | --- | --- |
| `Data` | Pointer to persistent host-owned root storage | Typed load/store at the root's storage type |
| `Direct` | Pointer to a process-image direct-variable location | Typed load/store; `AT` bindings only, outside D-V6 |
| `Function` | Address of a function implementation in the active module | Indirect callee only |
| `Import` | Address of a host service entry point | Indirect callee only |
| `Descriptor` | Address of a runtime-owned descriptor or vtable | Field pointer reads for objects; vtable source; outside D-V6 |

The boundary forbids: storing through a slot-table entry or a cell address (cell writes); passing a
cell address where a cell value is expected; calling a `Data` or `Direct` cell; loading or storing
through a `Function` or `Import` cell value; and referencing a root, user function, program body, or
host service by `@symbol` in a production module. The attach entry point is the only writer of
`stc_module_api` and `stc_module_slots`; detach clears both; accessor bodies and user code run
after attach.

### 7.3 Slot-table shape

The slot table is contiguous from slot `0`. Groups appear in the order `Data`, `Function`,
`Descriptor`, `Direct`, `Import`. Within the first four groups, entries are ordered by `GotId`;
`Import` entries are ordered by service ID. Each cell kind is fixed by the GOT plan; the module
neither chooses nor reorders slots. The Rust verifier checks the slot count and kind agreement
against the contract.

### 7.4 Process image and vtables

`AT`-bound direct variables reach the process image through `Direct` cells; ad-hoc `%I`, `%Q`, and
`%M` expressions reach it through the `io_image` service and a byte-offset GEP. Object code reads
runtime-owned descriptor/vtable memory through `Descriptor` cells, loads a vtable function cell,
loads its code address, and calls that address indirectly. Both patterns are inside the full
boundary and outside D-V6.

## 8. Entry points and ABI

A production module defines exactly these seven external entry points, with these signatures
(`ptr` is opaque):

| Symbol | Return type | Parameters |
| --- | --- | --- |
| `stc_module_descriptor` | `ptr` | `(ptr %out_size)` |
| `stc_module_functions` | `ptr` | `(ptr %out_count)` |
| `stc_module_attach` | `i32` | `(ptr, ptr, i32, i64)` |
| `stc_module_prepare` | `i32` | `(i32, i64, ptr, i32)` |
| `stc_module_cycle` | `i32` | `(i64)` |
| `stc_module_shutdown` | `i32` | `(i32)` |
| `stc_module_detach` | `void` | `(i64)` |

`stc_module_descriptor` stores the descriptor length as an `i64` through `%out_size` at align 8 and
returns the descriptor symbol. `stc_module_functions` stores the function count as an `i32` through
`%out_count` at align 4 and returns the function-table symbol. `stc_module_attach` validates the
host API major version and table prefix length, validates the slot count, and caches the API and
slot pointers in the module globals; it returns `0` on success and nonzero on rejection.
`stc_module_prepare`, `stc_module_cycle`, and `stc_module_shutdown` return `0` on success and
nonzero for their documented preconditions. `stc_module_detach` clears the cached pointers and
returns `void`.

The runtime ABI is 2.3; a production module validates the API major version `2` and the `8 + 11 * 8`
byte production service prefix. Test and debug builds require larger prefixes and are outside the
boundary. The seven entry points are the only symbols the host resolves for the lifecycle; the
module additionally exports `stc_metadata_json` and, when the source declares exports, export
wrappers, which are inside the full boundary.

## 9. Descriptor, function table, and metadata

- **Descriptor.** `stc_module_descriptor_data` is a private constant `[N x i8]` integer array,
  eight-byte aligned, whose bytes are the serialized module descriptor. The descriptor accessor
  reports the same `N`. The byte format is owned by the GOT artifact specification; this boundary
  fixes only the symbol, type, linkage, alignment, and the length agreement.
- **Function table.** `stc_module_function_table` is an internal constant `[N x ptr]`, eight-byte
  aligned, initialized with `ptr @symbol` entries that are code addresses of module functions,
  never GOT cell addresses. `N` equals the descriptor's function count and the module's function
  slot count. Entry order follows the GOT function order. The function table is not called through
  by generated code.
- **Metadata.** `stc_metadata_json_data` is a private constant NUL-terminated `[len x i8]` byte
  array at align 1. The exported `stc_metadata_json` function returns its address. The JSON content
  is metadata, not program state; no generated code reads it.

## 10. Control flow and undefined-behavior policy

Each function has at least one block, unique labels, and exactly one terminator per block. The
emitted terminators are:

| Terminator | Emitted for | Boundary status |
| --- | --- | --- |
| `ret void` / `ret <ty> <v>` | Function and entry-point returns | Included |
| `br label %L` | Sequential flow, loop backedges, merge blocks | Included |
| `br i1 <c>, label %T, label %F` | `IF`, `CASE`, loop conditions, trap and null guards | Included |
| `unreachable` | After a trap call; interface-prototype trap stubs | Included |
| `switch` | None | Excluded |

`phi` nodes are emitted at the start of a block by the lowering. The Rust `Function::check` enforces
unique labels and terminators; SSA dominance, phi firstness, and predecessor agreement are
builder obligations and Phase 0 `WellFormed` conjuncts, not Rust verifier checks. There are no
exception edges, landing pads, coroutines, or indirect branches. `uwtable` is emitted as an ABI
marker even though no unwind path exists.

The boundary excludes `undef` literals and poison-producing operations, overflow flags (`nsw`,
`nuw`), exact division flags, fast-math flags, `freeze`, and any load or store without a known
well-formed pointer at the boundary level. `alloca` is stack storage only and never dynamic. The
`getelementptr inbounds` form is used only for aggregate field paths the builder knows to be in
range; byte-offset and slot-table GEPs are non-`inbounds`. The core subset's no-UB theorem
(`ir_no_ub`) covers only the Phase 0 core fragment; extending it to the rest of this boundary is
Phase 7 work and is not claimed here.

## 11. D-V6 first end-to-end subset

### 11.1 Approved subset

The owner-approved D-V6 subset (2026-11-02) is: **scalar types, expressions, assignments, control
flow, functions, programs, and GOT globals; no OOP, references, strings, or aggregates.** It builds
on the Phase 0 core and does not widen it. This subset is the scope of the Phase 9 end-to-end
obligation recorded as `pipeline_correct`; Phase 1 fixes the scope and makes no theorem.

### 11.2 Included source constructs

| D-V6 element | Included source constructs | Phase 0 reference |
| --- | --- | --- |
| Scalar types | `BOOL`; `SINT`/`USINT`/`INT`/`UINT`/`DINT`/`UDINT`/`LINT`/`ULINT`; `BYTE`/`WORD`/`DWORD`/`LWORD`; `CHAR`/`WCHAR`/`UCHAR` | [Semantics §1](Semantics.md#1-scope-core-subset-and-exclusions), [Charter §6](Semantics-Charter.md#6-frozen-core-and-exclusion-register) |
| Expressions | Literals, variable access, exact arithmetic and bitwise operators, comparisons, integer/bit/char conversions, `SEL`, `MIN`, `MAX`, `LIMIT`, `MUX`, core function calls | Semantics §3.1, §3.4, §6 |
| Assignments | Scalar and bit-string assignment with the core conversion rules | Semantics §4.2 |
| Control flow | `IF`, `CASE` (comparison-branch lowering), `FOR`, `WHILE`, `REPEAT`, `EXIT`, `CONTINUE`, `RETURN` | Semantics §4.2 |
| Functions | `FUNCTION` with `VAR_INPUT` parameters and a scalar result | Semantics §1, §3.2 |
| Programs | Exactly one `PROGRAM` and one sequential cycle | Semantics §4.6 |
| GOT globals | Scalar `VAR_GLOBAL` roots accessed through `Data` cells and the two-load pattern | Semantics §5.2 |

### 11.3 Included target forms

D-V6 uses the Phase 0 core fragment in [Core IR Coverage](Core-IR-Coverage.md) §3–§8 without
extension: `void`, `i1`–`i64`, `ptr`, the overflow result, and fixed pointer/byte arrays; integer,
bitwise, comparison, cast, `select`, and `phi` instructions; rotate intrinsics; checked-arithmetic
intrinsics with `extractvalue`; `alloca`, `load`, `store`, and GEP for stack slots, root arrays, and
the slot table; direct calls to intrinsics and helpers; indirect calls through `Function` and
`Import` cells; trap calls with `unreachable`; the five cell kinds; the function table and module
locals; and the seven entry points. Every module carries the descriptor, function table, metadata
symbol, module globals, accessors, and attach/detach even when the program uses none of the wider
features, because those are part of the module envelope.

### 11.4 Deferred from D-V6

| Deferred feature | Boundary part that carries it | Added by |
| --- | --- | --- |
| `REAL`, `LREAL`, floating operators and standard functions | `float`/`double`, floating binops, `fcmp`, float intrinsics | Phase 6 source; Phase 5/7 target |
| `TIME`, `DATE`, `TOD`, `DT` | `i64` date operations, `local_date_ns` | Phase 6 source; Phase 5/7 target |
| Strings and UTF-8 | String aggregates, `memcmp`, `BOUNDS`/`INVALID_UTF8`, retained string data | Phase 6 source; Phase 5/7 IR; Phase 8 state |
| Arrays, structs, subranges, aggregate literals | Aggregate types, `insertvalue`, `llvm.memcpy`, range traps | Phase 6 source; Phase 5/7 IR; Phase 8 state |
| OOP, classes, function blocks, interfaces, methods, `THIS`, `SUPER` | Object records, descriptor/vtable cells, instance globals, vtable code calls, trap stubs | Phase 6 source; Phase 5/7 IR; Phase 8 runtime |
| References, pointers, `VAR_OUTPUT`, `VAR_IN_OUT`, aliasing | `ptrtoint`, null guards, reference pairs | Phase 6 source; Phase 5/7 target |
| Standard function blocks | `stdlib.rs` bodies, `cycle_now_ns`, timer/counter state, `phi` | Phase 6 source; Phase 8 runtime |
| I/O `AT` bindings and the process image | `Direct` cells, `io_image`/`io_size`, byte-offset GEP | Phase 6 source; Phase 8 runtime |
| Retained state | `stc_retain_buffer`, retain helpers, `state_load`/`state_save` | Phase 8 |
| Multiple tasks and guards | Task-guard roots and cycle guards | Phase 6 source; Phase 8 runtime |
| Additional trap and assert codes: `CONVERSION`, `BOUNDS`, `NULL_REF`, `ASSERT`, `UNIMPLEMENTED` | Trap calls with those codes; `assert` service | Phase 6/7 by trap |
| Export wrappers and source-level exports | Export functions and instance field pointers | Phase 6 source; Phase 8 runtime |
| Test build and coverage instrumentation | `stc_coverage_counters`, test services, test-only definitions | Not planned for the verified fragment |
| Debug information and checkpoint hooks | `llvm.dbg.declare`, metadata nodes, `debug_check` | Not planned (release contract excludes debug state) |
| Launcher executable | `launcher.rs` module with `main` | Outside the module boundary; not a Phase 7 target |

## 12. Build-form and construction exclusions

The boundary covers the production release module only. These build forms are outside it, and a
module containing their forms is rejected rather than silently admitted:

- **Test builds.** Host services 11 (`assert`) and 12 (`set_cycle_time_ns`), the larger API prefix,
  test-only source definitions, and coverage counters `stc_coverage_counters` with direct load/store
  instrumentation. The core target fragment excludes them ([Core IR Coverage](Core-IR-Coverage.md) §8).
- **Debug builds.** `llvm.dbg.declare` calls, `!dbg` attachments, `!llvm.dbg.cu` and
  `!llvm.module.flags` anchors, the `!DI*` metadata graph, the `"frame-pointer"="all"` and
  `"disable-tail-calls"="true"` function attributes, and `debug_check` checkpoint instrumentation.
- **Launcher executables.** The launcher is a separate module with a private `stc_module_path`
  string and a `main(i32, ptr)` that calls `stc_rt_execute` or `stc_rt_test_main` directly. It
  contains no generated program code and is outside the module boundary and the Phase 7 target.
- **Host ABI surfaces outside the lifecycle.** The host API table beyond the seven entry points
  belongs to the runtime; generated code reaches services only through `Import` cells and never
  declares or calls a `stc_rt_*` symbol directly in a production module.
- **Constructs never emitted.** The excluded instruction and type forms in §4 and §5 remain outside
  even if a future LLVM version accepts them.

If a later phase needs a test or debug form, this document, the Lean fragment, and the Rust
verifier must be revised together; a form does not enter the boundary because a build option can
enable it.

## 13. Versioning and change control

The boundary version is `1`, tied to profile `stc-2013-oop-utf8-v1`, host ABI major version `2`,
and `semantics_version` `1.0`.

- **Additions.** A new instruction, type, intrinsic, global, entry-point shape, or call pattern
  enters the boundary only by a reviewed change to this document. If the form also belongs to the
  Phase 0 core, [Core IR Coverage](Core-IR-Coverage.md) and the Lean `Stc.Codegen.Ir` definition
  change in the same commit. A change that adds a form without a boundary revision is a defect in
  the emitter change.
- **Removals.** Removing an emitted form is a boundary revision too; it must update the coverage
  table, the verifier, and any Phase 5/7 model that covers the form.
- **Version increments.** The boundary version increments when the set of included forms changes,
  not when prose or traceability links change. The value is independent of `semantics_version`;
  the semantics version changes only through its own freeze procedure.
- **Phase ownership.** Phase 5 extends `IRFrag`/`WellFormed` to exactly this boundary; a mismatch is
  an extension defect, not permission to accept unlisted forms. Phase 7 extends preservation
  subset by subset, starting from D-V6, and does not claim a wider subset before its proof exists.
  Phase 9 owns the D-V6 composition theorem.
- **CI.** This document is a required specification document in
  [`check_docs.py`](../../tools/ci/check_docs.py). The Rust structural verifier and the clang
  acceptance tests are the executable checks for the emitted forms; the Phase 5 proved checker is
  the long-term counterpart. A change to the emitter that adds a form without updating this
  document fails review, not CI, until the Phase 5 checker exists.

## 14. Traceability

| Boundary area | Emitter source | Executable evidence |
| --- | --- | --- |
| Module envelope, globals, descriptor, function table, metadata, entry points | [`ir.rs`](../../crates/stc-codegen/src/ir.rs), [`unit.rs`](../../crates/stc-codegen/src/unit.rs) | [`library.ll`](../../crates/stc-codegen/tests/snapshots/library.ll), [`module_ir.rs`](../../crates/stc-codegen/tests/module_ir.rs) |
| Instructions, types, casts, calls | [`ir.rs`](../../crates/stc-codegen/src/ir.rs), [`expr.rs`](../../crates/stc-codegen/src/expr.rs), [`place.rs`](../../crates/stc-codegen/src/place.rs) | clang acceptance in `module_ir.rs`, snapshot IR |
| GOT access, cell kinds, slot rules | [`unit.rs`](../../crates/stc-codegen/src/unit.rs), [`expr.rs`](../../crates/stc-codegen/src/expr.rs), [`place.rs`](../../crates/stc-codegen/src/place.rs) | [`verify.rs`](../../crates/stc-codegen/src/verify.rs), `module_ir.rs` |
| Intrinsics and host services | [`abi.rs`](../../crates/stc-codegen/src/abi.rs), [`place.rs`](../../crates/stc-codegen/src/place.rs), [`stdlib.rs`](../../crates/stc-codegen/src/stdlib.rs) | snapshot IR and codegen tests |
| Traps | [`expr.rs`](../../crates/stc-codegen/src/expr.rs), [`place.rs`](../../crates/stc-codegen/src/place.rs), [`unit.rs`](../../crates/stc-codegen/src/unit.rs) | trap tests and snapshot IR |
| D-V6 scope | [Semantics.md](Semantics.md) §1, [Core IR Coverage](Core-IR-Coverage.md) | Phase 0 registry and conformance evidence; no new Phase 1 theorem |
