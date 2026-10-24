# Architecture

Status: M4 implemented; online-change architecture resolved (2026-10-24)  
Profile: `stc-2013-oop-utf8-v1`  
Related: [Project Plan](../00-Project-Plan.md), [Runtime](Runtime-and-Execution.md), [Online Change and GOT](Online-Change-and-GOT.md), [Debug and Monitoring](Debug-and-Monitoring.md), [CLI and Manifest](CLI-and-Manifest.md), [Testing](Testing-and-Verification.md)

## 1. Overview

`stc` is a Rust workspace that compiles Structured Text to textual LLVM IR. The IR is the
compiler's primary output; `clang` is the backend driver that assembles, optimizes, and links it
for the selected target. This keeps the compiler independent of the LLVM C++/C API and its
version, and makes the generated IR inspectable and testable.

```text
+--------+   +--------+   +----------+   +--------+   +----------------------+
| source | ->| lexer  | ->| parser   | ->| sema   | ->| HIR + identity model |
| (UTF-8)|   | tokens |   | CST/AST  |   | names/ |   | typed, resolved      |
+--------+   +--------+   +----------+   | types  |   +----------------------+
                                          +--------+             |
                                                                 v
                                              +-----------------------------------+
                                              | textual IR builder + GOT manifest |
                                              +-----------------------------------+
                                                                 |
                                                       +------------------+
                                                       | clang (module)   |
                                                       +------------------+
                                                                 |
                                    module (.dll/.so) + manifest (.json/.bin)
                                                                 |
                                              +-----------------------------------+
                                              | persistent host (stc-runtime)     |
                                              | cycle | GOT | roots | loader      |
                                              | manifest store | state/IO | debug  |
                                              +-----------------------------------+
```

## 2. Workspace layout

```text
stc/
  Cargo.toml                 workspace manifest
  rust-toolchain.toml        pinned stable toolchain
  crates/
    stc-span/                FileId, Span, SourceMap, Location, line index
    stc-diag/                Diagnostic, severity, codes, renderer, JSON
    stc-lexer/               tokenizer, trivia, literals, pragmas
    stc-syntax/              CST kinds, tree, typed AST wrappers
    stc-parser/              recursive descent + Pratt expressions, recovery
    stc-fmt/                 fixed-style formatter over the CST
    stc-hir/                 arenas, DefId/HirId, types, HIR nodes
    stc-sema/                name resolution, type checking, const eval, lowering
    stc-codegen/             IR builder, GOT/manifest model, ABI, mangling, module emission
    stc-manifest/            stc.toml schema, local dependency graph, source discovery
    stc-runtime/             persistent host, loader/GOT/root allocator, staticlib/cdylib
    stc-test/                test discovery, runner, reporting
    stc-driver/              CLI binary `stc`
  stdlib/                    standard function blocks in ST
  lean/                      Lake project for exact-function models/proofs
  tests/                     conformance, UI, codegen, execution, projects
  examples/
  Specification/
```

Crate dependency direction is acyclic:

```text
stc-span <- stc-diag
stc-span <- stc-lexer <- stc-syntax <- stc-parser
stc-span <- stc-hir <- stc-sema <- stc-parser
stc-hir <- stc-codegen
stc-manifest <- stc-driver
stc-test <- stc-driver
stc-runtime is built independently and linked by the driver
```

`stc-codegen` must not depend on `stc-parser` or `stc-syntax`; it consumes HIR only. The GOT
manifest model is produced by codegen from HIR identity metadata, not by the runtime. The runtime
does not depend on any compiler crate; the manifest parser lives behind the manifest boundary and
is fuzzed independently.

## 3. Front end

### 3.1 Lexer

- Hand-written, byte-oriented over a UTF-8 `&str`.
- Produces tokens and trivia with exact spans; see the tokenization spec.
- No parser callbacks and no global state.
- Interning is done by the lexer or a shared `Interner` owned by the compiler session.

### 3.2 Parser

- Recursive descent for declarations and statements; Pratt/precedence climbing for expressions.
- Lossless CST (rowan-style) plus typed AST wrappers.
- Error recovery:
  - declaration-level sync at `;`, `END_VAR`, `END_FUNCTION`, `END_FUNCTION_BLOCK`,
    `END_PROGRAM`, `END_CLASS`, `END_INTERFACE`, `END_TYPE`, `END_NAMESPACE`;
  - statement-level sync at `;`, `END_IF`, `END_CASE`, `END_FOR`, `END_WHILE`, `END_REPEAT`,
    `ELSE`, `ELSIF`;
  - expression recovery at `)`, `]`, `,`, `;`, and block delimiters.
- The parser never panics on arbitrary input; fuzzed. Expression, statement, and type nesting is
  limited to 256 levels (`STC-E-GRAM-109`), and the ordinary recovery codes are
  `STC-E-GRAM-105`–`111` (Grammar §13.1, D-047).
- Missing tokens are zero-width leaves with a diagnostic. A mismatched `END_*` reports the
  unexpected terminator and a label at the opening keyword; the unexpected terminator is left for
  the enclosing construct (D-053).
- It records pragmas and attaches leading pragmas to the following item and a trailing pragma to
  the preceding variable declaration. `{stc:test}` and `{stc:export}` become typed annotations;
  `stc:` payloads are parsed as strict JSON by `stc-syntax` (serde_json), with duplicate keys,
  nesting deeper than 32, and bodies over 64 KiB rejected, and unknown keys preserved where the
  schema permits (D-050).

### 3.3 Semantic analysis

Passes:

1. **Collection**: collect namespaces, types, POUs, methods, globals, configurations; detect
   duplicate definitions and invalid nesting.
2. **Name resolution**: resolve type references, POU references, variable references, namespaces,
   `USING`, `THIS`/`SUPER`; report unknown/ambiguous names.
3. **Type checking**: check assignments, calls, operators, conditions, loop variables, array
   bounds, subranges, initializers, interface implementations, inheritance, access control.
4. **Constant evaluation**: evaluate constant expressions for array bounds, enum values,
   initializers, case labels, task intervals, and `AT` addresses.
5. **Lowering**: produce HIR with explicit conversions, resolved callees, and resolved lvalues.

The analyzer uses a `Ty::Error` sentinel to suppress cascades. It reports all independent errors
in a file, not just the first.

### 3.4 HIR

Defined in [AST](../AST/AST.md). Codegen consumes only HIR. A HIR body is a list of statements and
expressions with resolved places, callees, and explicit casts.

## 4. Code generation

> **M4 status:** §4.1–§4.12 are implemented for the elementary scalar subset (D-076–D-093).
> Functions, programs, globals, scalar operators, checked arithmetic, control flow, the process
> image, scalar retained leaves, executable/library units, and `{stc:export}` wrappers are
> emitted and tested. OOP lowering (§4.6), aggregates, strings, references, and standard
> functions remain M5; codegen rejects them with `STC-E-CODEGEN-001`. `stc_abi_version` is
> supplied by the runtime, not by the generated module, to avoid a duplicate symbol.
>
> **Post-M4 architecture (resolved 2026-10-24):** the direct-reference, code-linked artifact
> model is superseded by M4.5's persistent host + loadable module model. §4.3–§4.9 below are
> revised so M5 OOP/aggregate/string/reference features are built on GOT roots, runtime
> descriptors, handles, and place descriptors rather than module-owned contiguous objects. The
> existing M4 implementation remains the current scalar artifact; the new model begins in M4.5.

### 4.1 IR builder

`stc-codegen` contains a small typed IR builder that emits **textual LLVM IR**. It does not link
against LLVM.

Design:

- A `Module` owns functions, globals, type definitions, constants, and metadata.
- Builders validate arity/types as far as practical; the final validator is `clang`/`opt`.
- Emission is deterministic: functions and globals are sorted by mangled name (or emitted in a
  stable source order with explicit ordering), and temporary names are allocated from counters.
- Text formatting is stable and tested with snapshots.
- A verifier step runs `clang -x ir -c -o NUL` (or `opt -verify`) in tests and CI; in production a
  failure is reported as an internal compiler error with the IR attached.

Module-shaped IR (`stc_codegen::ir`) adds the GOT primitives from
[Online Change and GOT](Online-Change-and-GOT.md) §3. The per-generation slot table is the
module's only view of persistent state: generated code loads a cell with
`getelementptr ptr, ptr %slots, i64 N` followed by a pointer-sized load, and treats every cell
value as an opaque `ptr`. User and host-service calls are indirect through `Function`/`Import`
cell values. Entry points are defined `dso_local dllexport` with the ABI 2.0 signatures; the
descriptor blob and function-address table are private/internal constants. `stc_codegen::verify`
checks the build contract before rendering: no body names a persistent root or user callable
directly, every cell use matches its kind, direct global access is limited to an allowlisted
module-local set, and the function-address table is the only initializer that names a user
function. Tests compile the module IR with clang for the host and both x64 targets.

### 4.2 Target model

- By default, the emitted module omits `target triple` and `target datalayout`; `clang` selects
  the host target and computes the layout.
- With `--target <triple>`, the module emits `target triple = "<triple>"` but still omits
  `target datalayout`; `clang` derives the layout from the triple.
- Codegen never hard-codes pointer sizes, alignment, or struct offsets. It uses LLVM types,
  `getelementptr`, and explicit length fields. This is what makes the same IR usable for x86-64,
  AArch64, or another clang target.
- C ABI calls use `ptr`, `i8`, `i16`, `i32`, `i64`, `float`, and `double`; no target-specific
  calling conventions are emitted.
- Host platforms do not change the emitted IR. The same module is valid for Windows MSVC and
  Linux GNU targets; only clang's target triple and the runtime staticlib differ (D-042).
- Targets whose C ABI differs in fundamental ways (for example exotic embedded ABIs) are out of
  scope until a target-specific test exists.

### 4.3 Type lowering

| ST type | LLVM representation |
| --- | --- |
| `BOOL` | `i8` in memory; `i1` in SSA; normalize with `icmp ne 0` |
| `SINT`/`USINT` | `i8` |
| `INT`/`UINT` | `i16` |
| `DINT`/`UDINT` | `i32` |
| `LINT`/`ULINT` | `i64` |
| `BYTE`/`WORD`/`DWORD`/`LWORD` | `i8`/`i16`/`i32`/`i64` (unsigned operations) |
| `REAL`/`LREAL` | `float`/`double` |
| `TIME`/`LTIME` | `i64` nanoseconds |
| `DATE`/`LDATE` | `i64` days or nanoseconds (profile-defined) |
| `TOD`/`LTOD` | `i64` nanoseconds since midnight |
| `DT`/`LDT` | `i64` nanoseconds since epoch |
| `CHAR` | `i8` |
| `WCHAR` | `i16` |
| `UCHAR` | `i32` (Unicode scalar) |
| `STRING[n]` | `%stc.string.N = type { i32 len, [N x i8] }` |
| `WSTRING[n]` | `%stc.wstring.N = type { i32 len, [N x i16] }` |
| `USTRING[n]` | `%stc.ustring.N = type { i32 len, [N x i32] }` (4 bytes per character) |
| Enum | underlying integer type |
| Subrange | base integer type; bounds checked at assignment |
| Static array | `[N x T]`; multi-dimensional arrays are nested arrays |
| Variable-length array (conformant parameters only) | `%stc.array.ref = type { ptr, i64, [rank x %stc.bound] }`; `%stc.bound = type { i64 lower, i64 upper }` |
| Struct | named LLVM struct for transient values; persistent structs are per-field roots |
| `OVERLAP` struct | one atomic host root with aliasing typed views at declared offsets |
| `REF_TO T` | host-resolvable place descriptor (not a raw pointer) |
| `POINTER TO T` | transient raw pointer (`ptr`), `STC-POINTER`, never persisted |
| Class/FB instance | stable host handle plus runtime object record/descriptor |
| Interface variable | `{handle, descriptor}`; `NULL` is a null handle/descriptor |
| Function | LLVM function reached through a host-owned function cell |
| Program | LLVM function plus its host-rooted instance fields |

String representation is a **length-prefixed fixed-capacity host root** (one root per string
value; no per-character roots):

- `STRING[n]`: `{ i32 len; [n x i8] data; }`, `len` in bytes/characters, no NUL requirement.
- `WSTRING[n]`: `{ i32 len; [n x i16] data; }`, `len` in 16-bit units.
- `USTRING[n]`: `{ i32 len; [n x i32] data; }`, `len` in characters (Unicode scalar values).
  Each character occupies 4 bytes, so the data area is exactly `4 * n` bytes and indexing is
  O(1). `LEN` reads `len`; `LEN_MAX` returns `n`; `LEN_CODE_UNIT` encodes/scans to count UTF-8
  code units for interchange. The in-memory size is `4 + 4 * n` bytes. UTF-8 is the
  source/interchange encoding, not the storage encoding.
- Copying a string copies the root value; `VAR_IN_OUT` passes a descriptor/pointer to the root.
- The runtime owns bounds checks and encoding validation.

`STRING`/`WSTRING` default capacities are 255 units; `USTRING` default capacity is 255 characters
(1020 bytes of data). Explicit `[n]` capacities are honored; the v1 maximum is 65535 characters
(262140 bytes of data).

Conformant arrays (`ARRAY [*]` in `VAR_IN_OUT`/FB input/output parameters) use a descriptor
`{ ptr data, i64 len, [rank x { i64 lower, i64 upper }] }`. The descriptor points at caller-owned
storage; v1 allocates nothing and rejects dynamically sized locals, globals, and function results
(D-023). Every index is bounds-checked against the descriptor's bounds. The callee never owns the
storage and must not retain the pointer beyond the call.

### 4.4 Memory layout

- Struct fields are laid out in declaration order using LLVM's default target layout. No implicit
  packing.
- `OVERLAP` structs are laid out as a byte array of the computed size; each field is accessed by a
  typed pointer at its declared offset. The analyzer verifies that fields fit and that
  overlap-addressed fields do not conflict unless they share the same `AT` range.
- `AT` direct variables are not laid out as ordinary storage. The compiler computes their
  process-image offsets and records them in metadata; it does not link them to hardware. Binding
  process-image ranges to devices is runtime-owned (see [I/O and Devices](IO-and-Devices.md)).
  Unbound inputs receive type defaults each cycle; unbound outputs are discarded.
- Persistent aggregates do not use one contiguous parent allocation. Each persistent field is a
  separate host root with its own cell, identity, and storage; field access lowers to a field-cell
  lookup. A runtime object record links the fields of an object and holds its descriptor.
- FB/class inheritance is represented by a base-chain in the object record/descriptor, not by a
  contiguous prefix struct. Upcasting adjusts the descriptor/handle view, not a raw pointer.
- Interface values are `{handle, descriptor}`. `NULL` is a null handle/descriptor. Preserved
  interface values survive compatible swaps because descriptors are runtime-owned.
- Validated reference values (`REF_TO`) are place descriptors (`Root`/`Direct`/`Stack`/`Object`);
  `NULL` is the null descriptor. `POINTER TO` remains a transient raw pointer and cannot appear
  in a persistent root.

### 4.5 Functions and calls

- A function lowers to an LLVM function with the target's C calling convention (the default).
  A call to a user or standard function goes through a host-owned function cell, so a swap can
  rebind it without rewriting code. Compiler helpers/intrinsics are direct.
- A function with a result uses the declared return type; a function without a result returns
  `void`. Assignment to the function name writes a dedicated result local; `RETURN` returns it.
- FB instances are host-rooted per-field objects with a stable handle and runtime object record.
  An FB body is a function called with the handle/descriptor; inputs are written to the instance
  fields before the body runs; outputs are read after the call.
- Methods are functions with an explicit handle/descriptor first:
  `void @stc_m_Class_Method(ptr %handle, ptr %descriptor, ...)`.
- Class/FB constructors are not separate user-visible functions; prepare/init allocates roots and
  applies declared initializers for new fields only.
- Function calls with output parameters (`=>`) pass references to the destination; input
  parameters are passed by value unless declared `VAR_IN_OUT`.
- `VAR_IN_OUT` parameters use place descriptors/pointers to the actual variable's storage;
  aliasing is the user's responsibility per the ST semantics. Conformant `ARRAY [*]` parameters
  use the descriptor described above instead of a bare pointer.

### 4.6 OOP lowering

- Each class/FB type gets a runtime-owned type descriptor and a vtable whose entries are
  host-owned function cells, not raw code addresses. Descriptors are allocated/validated by the
  host and survive module unload.
- Vtable slots are assigned deterministically: methods sorted by `(name, signature)` across the
  type's interface set; overrides reuse the base slot.
- Interface method calls use the interface descriptor/vtable slot; class/FB calls through a base
  reference use the class descriptor. Direct calls on a concrete instance still go through the
  function cell for swap safety.
- `OVERRIDE` rebinds the derived type's vtable cell to the new implementation.
- `SUPER.method()` resolves the base type's implementation through the base descriptor/cell.
- `THIS` is the stable handle/object record; passing `THIS` to an interface builds
  `{handle, descriptor}`.
- `?=` (assignment attempt) calls a runtime helper that checks the source descriptor against the
  destination interface/class descriptor and returns `{null, null}` or a valid
  `{handle, descriptor}`.
- `FINAL`/`ABSTRACT` are enforced statically; runtime checks cover descriptor compatibility.
- A vtable shape change is rejected during online change while any live instance or interface
  depends on the old shape.

### 4.7 Control flow

- `IF`/`ELSIF`/`ELSE` -> basic blocks with `br` and `phi` where needed.
- `CASE` -> a chain of comparisons or a `switch` when the labels are dense integer constants.
  Ranges and non-integer selectors use comparison chains.
- `FOR` -> initialization, header block, body, increment, and exit. The loop variable is checked
  against subrange/overflow rules. `BY` defaults to 1; negative steps terminate when the control
  variable passes the final value.
- `WHILE` -> condition block + body + back edge.
- `REPEAT` -> body executes once, then condition; back edge to body.
- `EXIT`/`CONTINUE` -> branches to the innermost loop's exit/continue blocks; the parser/analyzer
  records the target loop.
- `RETURN` -> branch to the function's return block; the result local is returned.
- Empty statements emit no code.

### 4.8 Arithmetic and checks

- Integer addition/subtraction/multiplication use LLVM's `llvm.sadd.with.overflow`,
  `llvm.uadd.with.overflow`, etc., or the corresponding checked helpers. On overflow, call
  `stc_rt_trap(STC_TRAP_OVERFLOW, span_id)`.
- Division/modulo by zero calls `stc_rt_trap(STC_TRAP_DIV_ZERO, span_id)`.
- `MOD` follows truncating division (sign of dividend), matching C/LLVM `srem`.
- Conversions insert range checks where the value may not fit (for example `DINT -> SINT`).
  Explicit conversion functions use the same checks.
- Float operations use `fadd`, `fsub`, `fmul`, `fdiv`, `frem` without `fast-math` flags. NaN and
  infinity behavior follows IEC 60559; transcendental functions call the runtime.
- Comparisons produce `i1`; boolean values are normalized before storage.
- Subrange assignments and array bounds are checked in hosted builds. A future `--release`
  policy may disable bounds checks; it is documented and tested separately.
- Bitwise operations on `BYTE`/`WORD`/`DWORD`/`LWORD` are unsigned integer ops of the same width.
  Shifts mask the count to the type width; rotates use `llvm.fshl`/`llvm.fshr`.

### 4.9 Globals, initialization, and main

- Persistent globals and program/FB/class fields are **host roots**, not ordinary module globals.
  Codegen emits a per-generation slot table and accesses storage through the GOT; the manifest
  records each root's `GotId`, type/layout, and current slot.
- Version-local read-only data and compiler helpers may remain module globals with deterministic
  mangled names (`@stc_g_<ns>_<name>` for non-persistent constants).
- Constant and non-constant initializers are recorded for new roots; prepare/init allocates the
  root and applies the initializer exactly once. Preserved roots are never reinitialized.
- The module entry points are `stc_module_attach` (receive the host API table and slot table),
  `stc_module_prepare` (validate/build the candidate and new roots), `stc_module_cycle`
  (execute one scan), `stc_module_shutdown`, and `stc_module_detach`; descriptor and function
  table getters expose the versioned metadata without host services. Exact symbol names,
  signatures, descriptor bytes, and slot-table rules are fixed in
  [GOT Artifacts and Schemas](GOT-Artifacts.md) §7–§8; `stc_init` remains the generated
  initialization entry used by prepare.
- `stc_module_cycle` executes one scan of the configured programs/tasks. The host, not the
  module, latches inputs, publishes outputs, and decides when a swap may occur.
- `stc_module_shutdown` saves retained state and releases module-local resources; the host retains
  GOT cells and quarantined roots.
- Executable mode generates a thin host launcher:

  ```llvm
  define i32 @main(i32 %argc, ptr %argv) {
  entry:
    %rc = call i32 @stc_rt_execute(i32 %argc, ptr %argv, ptr @stc_module_descriptor)
    ret i32 %rc
  }
  ```

  `stc_rt_execute` loads the adjacent module, owns the cycle loop, time base, cycle limit,
  signal handling, error reporting, and the online-change controller.
- Library mode exports the host embedding entry points; `{stc:export}` wrappers are served
  through the host export registry for the active module, which remains a separate loadable
  artifact. The embedding host owns the loop, I/O backend, manifest store, and debug services.

### 4.10 Name mangling

Mangled names are stable and C-identifier-safe:

```text
stc_fn_<ns>_<name>_<sig_hash>       functions
stc_fb_<ns>_<name>                  FB type
stc_fb_body_<ns>_<name>_<sig_hash>  FB body function
stc_m_<type>_<method>_<sig_hash>    methods
stc_g_<ns>_<name>                   globals
stc_iface_<ns>_<name>               interfaces
stc_vtbl_<type>                     vtables
stc_desc_<type>                     type descriptors
stc_export_<name>                   {stc:export} wrappers
stc_rt_*                            runtime C ABI (fixed names)
```

Namespace segments are joined with `_n_`; underscores in source names are preserved. The
signature hash is a stable 64-bit FNV-1a of the canonical type string, not a random hash.
`{stc:no_mangle}` uses the exact source name after validating it is a C identifier and unique.

### 4.11 Debug information

- Debug information is the default v1 build output; `--release` selects the optional stripped,
  optimised mode. Debug and release builds have distinct build IDs and manifests.
- Codegen emits full DWARF/CodeView variable/type/location information through clang, plus a
  checkpoint table and host check calls for breakpoints. It does not patch machine code.
- The IR builder retains source-span IDs and manifest-linked field/type identities so the
  debugger can present both standard debug info and a manifest-assisted split-object view.
- Debug hooks and checkpoint calls are absent from `--release` artifacts; a sidecar symbol file
  may retain symbols for post-mortem analysis without changing runtime behaviour.
- Runtime traps report the source span ID and the runtime formats it using a source map emitted
  into the binary or a sidecar JSON file.
- Every online swap ends the debug session and drops the retired generation's debug information
  after unload.

### 4.12 Determinism and reproducibility

- No hash-map iteration in emission paths; use sorted vectors or `BTreeMap`.
- Temporary names use a monotonic counter.
- Name hashes are deterministic.
- The module header includes the compiler version and profile ID, but not timestamps or paths.
- GOT slots are sorted deterministically; `GotId`s are content-derived; manifests have canonical
  JSON and byte-identical binary projections for identical inputs.
- IR snapshots normalize target-specific differences; execution tests run on the host triple.
- A double-build test compares the module, manifest, and metadata byte-for-byte.

## 5. Project graph and namespaces

A **package** is an `stc.toml` manifest plus its source files. A project build is a graph of
packages. In v1, dependencies are local paths plus a `stc.lock` (A-009):

```toml
[dependencies]
motor-lib = { path = "../motor-lib" }
```

The driver:

1. Loads the root manifest.
2. Resolves each dependency entry to a manifest at the given path, canonicalizes the path, and
   deduplicates packages by canonical path.
3. Detects cycles and builds a topological order.
4. Assigns each package a `PackageId` and a root namespace derived from the manifest name (or the
   dependency alias).
5. Loads each package's sources and builds its item tree.
6. Merges all packages into one HIR crate with globally unique `DefId`s. Cross-package references
   resolve through package-qualified namespaces.

Rules:

- A dependency key is the name the parent uses; the dependency's own `[package] name` must match
  unless the manifest declares an explicit alias. Duplicate names with different canonical paths
  are errors unless aliased.
- `INTERNAL` is package-private; `PRIVATE` is POU/class-private; the default top-level visibility
  is public across packages. Access checks run after name resolution.
- `USING` imports namespaces, not packages. A package's root namespace is its manifest name.
- Mangled symbols include the package name, so two dependencies with the same POU name cannot
  collide in the linked module.
- Source dependencies compile together with the root package; v1 does not require a prebuilt
  binary library. `stc build --emit=lib` is still available for distribution/embedding, and a
  binary-dependency mode can use it later.
- `stc test` in a package compiles its dependencies but discovers and runs only tests in the
  current package. A dependency's tests are not run unless you invoke `stc test` in that package.
- A separate test project is a normal package with `[dependencies]` pointing at the code under
  test. This is the recommended layout for larger projects.
- v1 has no network access or registry. `stc.lock` records the resolved path graph, and `--locked`
  enforces it. Registry/version solving is a later extension (A-009).

## 6. Backend invocation

### 6.1 clang discovery

Order:

1. `--clang <path>` CLI option.
2. `STC_CLANG` environment variable.
3. `clang` on `PATH`.
4. A configured path in `stc.toml` (`[build] clang = "..."`).

`stc` runs `clang --version` and `clang -print-target-triple` once per build and reports a
diagnostic if clang is missing or too old. The minimum tested version is recorded in CI; the
project targets IR syntax accepted by LLVM 15 and later.

### 6.2 Commands

| Artifact | Command shape |
| --- | --- |
| IR only | `stc build --emit=ir` writes `<name>.ll`; no clang call |
| Object | `clang --target=<triple> -x ir <name>.ll -c -o <name>.o -O<level> [-g]` |
| Module | `clang --target=<triple> -shared -fPIC <name>.ll -o <name>.dll/.so`; paired manifest written by the compiler |
| Executable | `clang --target=<triple> <launcher>.ll <runtime_staticlib> -o <name>.exe -O<level> [-g]`; loads an adjacent module |
| Static library | `clang ... -c` then `llvm-ar rcs <name>.a <objects>` plus the runtime object |
| Embedding library | `clang --target=<triple> -shared <name>.ll <runtime> -o <name>.dll/.so` with the host embedded |

- The runtime staticlib is built with the toolchain that matches the target C ABI: the MSVC
  toolchain for `*-pc-windows-msvc` and the GNU toolchain for `*-unknown-linux-gnu`.
- On Windows MSVC targets, clang uses `link.exe` or `lld-link`. On Linux targets it uses the
  system linker (GNU `ld` or `lld`). Artifact suffixes and linker flags are selected from the
  target triple, not hard-coded.
- The runtime staticlib path is discovered from the workspace build output or `STC_RUNTIME`.
- Linking errors are surfaced as build errors with the clang command (verbosity-gated) and the
  generated IR retained for inspection.

### 6.3 Artifact layout

```text
target/stc/<profile>/
  <name>.ll
  <name>.o
  <name>.exe | <name>.dll/.so | <name>.a
  <name>.stc-meta.json       runtime descriptor, exports, source map
  <name>.stc-got.json        canonical manifest (tooling/download)
  <name>.stc-got.bin         compact runtime manifest projection
  build.log                  verbose log (if -v)
```

The `.stc-meta.json` includes the selected configuration, task list, exported symbols, string
capacities, and source-span table for runtime diagnostics. The module and manifest are installed
under immutable versioned names (`<name>-<build-id>.dll/.so` and its manifest pair); the runtime
state directory holds the active copy and a current pointer.

### 6.4 Host platforms

- Primary development host: Windows 11, x86-64, MSVC (D-007).
- Planned additional hosts after basic development: Linux x86-64 and Linux AArch64 (ARM64), GNU
  toolchain (D-042). Both are little-endian, so the state-file format and IR are unchanged.
- Linux x86-64 and Linux AArch64 are valid clang targets from the start. Cross-target builds
  need a runtime staticlib built for the target.
- Host-specific operations are confined to the driver and runtime: clang/linker discovery,
  executable/shared-library naming, path semantics, the monotonic clock, and atomic file
  replace. The front end, HIR, and IR builder contain no host conditionals.
- Linux host builds must pass the same workspace, execution, and conformance tests as Windows;
  CI covers both hosts (see [Testing and Verification](Testing-and-Verification.md) §6).

## 7. Runtime boundary

The generated module uses a versioned host API table (host ABI 2), not a fixed set of exported
global symbols. Host ABI 2.0 provides:

| Entry | Purpose |
| --- | --- |
| `trap` | Trap with a diagnostic; non-returning to module code |
| `assert` | User assertion (test/development) |
| `cycle_now_ns` | Cycle time latched at cycle entry |
| `monotonic_now_ns` | Live monotonic clock |
| `local_date_ns` | Local date/time latched for the scan |
| `alloc` / `free` | Transient module-version allocation |
| `io_image` / `io_size` | Process-image access |
| `state_load` / `state_save` | Retained-state helpers |

Reference resolution, interface/descriptor checks, typed variable access, string/math helpers,
and tracing are later minor-version additions; they are not part of ABI 2.0 and M4.5 does not
call them. The launch/embedding entry points (`stc_rt_execute`, `stc_host_*`) are host exports,
not module imports. The exact table, service IDs, versions, and signatures are in
[GOT Artifacts and Schemas](GOT-Artifacts.md) §8; the lifecycle is in
[Online Change and GOT](Online-Change-and-GOT.md).

## 8. Security and robustness

- The compiler treats source as untrusted input: no panics, bounded recursion, bounded memory.
- Parser/analyzer recursion depth is limited; deeply nested expressions produce a diagnostic
  rather than a stack overflow.
- Generated code is native and unsandboxed; `stc test` runs untrusted test code in a separate
  process with a timeout. This is a development tool, not a security boundary.
- Paths in `stc.toml` are resolved relative to the manifest; path traversal outside the project is
  allowed only for explicit user paths and is reported in verbose output.
- The runtime loads only local, explicitly deployed modules with strict descriptor validation.
  Modules are trusted native code and are not sandboxed; v1 has no network deployment or
  untrusted-module loading.
- Downloaded GOT manifests are untrusted parsed data: validate every field and digest,
  access-control export because layout data reveals program structure, and never execute manifest
  content. Descriptor/manifest parsers are fuzz targets.
- Debug/monitoring access is authenticated and audited; the standard API is typed, not raw
  memory.

## 9. Open architecture questions

None as of 2026-10-24. The online-change/GOT/manifest/debug architecture is resolved in
[Online Change and GOT](Online-Change-and-GOT.md),
[Debug and Monitoring](Debug-and-Monitoring.md), and `update_plan_got.md` §13.0. Resolved
decisions are tracked in [Decisions](../Decisions.md). New architecture questions are appended
here.
