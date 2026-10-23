# Architecture

Status: Draft  
Profile: `stc-2013-oop-utf8-v1`  
Related: [Project Plan](../00-Project-Plan.md), [Runtime](Runtime-and-Execution.md), [CLI and Manifest](CLI-and-Manifest.md), [Testing](Testing-and-Verification.md)

## 1. Overview

`stc` is a Rust workspace that compiles Structured Text to textual LLVM IR. The IR is the
compiler's primary output; `clang` is the backend driver that assembles, optimizes, and links it
for the selected target. This keeps the compiler independent of the LLVM C++/C API and its
version, and makes the generated IR inspectable and testable.

```
+--------+   +--------+   +----------+   +--------+   +-----------+   +--------+
| source | ->| lexer  | ->| parser   | ->| sema   | ->| HIR       | ->| IR     |
| (UTF-8)|   | tokens |   | CST/AST  |   | names/ |   | typed,    |   | text   |
+--------+   +--------+   +----------+   | types  |   | resolved  |   +--------+
                                          +--------+   +-----------+        |
                                                                            v
                                                                    +---------------+
                                                                    | clang         |
                                                                    | (target/link) |
                                                                    +---------------+
                                                                            |
                                              .ll / .o / exe / .a / .dll / .so
```

## 2. Workspace layout

```
stc/
  Cargo.toml                 workspace manifest
  rust-toolchain.toml        pinned stable toolchain
  crates/
    stc-span/                FileId, Span, SourceMap, Location, line index
    stc-diag/                Diagnostic, severity, codes, renderer, JSON
    stc-lexer/               tokenizer, trivia, literals, pragmas
    stc-syntax/              CST kinds, tree, typed AST wrappers
    stc-parser/              recursive descent + Pratt expressions, recovery
    stc-hir/                 arenas, DefId/HirId, types, HIR nodes
    stc-sema/                name resolution, type checking, const eval, lowering
    stc-codegen/             IR builder, ABI, mangling, module emission
    stc-manifest/            stc.toml schema, local dependency graph, source discovery
    stc-runtime/             Rust staticlib/cdylib with C ABI
    stc-test/                test discovery, runner, reporting
    stc-driver/              CLI binary `stc`
  stdlib/                    standard function blocks in ST
  lean/                      Lake project for exact-function models/proofs
  tests/                     conformance, UI, codegen, execution, projects
  examples/
  Specification/
```

Crate dependency direction is acyclic:

```
stc-span <- stc-diag
stc-span <- stc-lexer <- stc-syntax <- stc-parser
stc-span <- stc-hir <- stc-sema <- stc-parser
stc-hir <- stc-codegen
stc-manifest <- stc-driver
stc-test <- stc-driver
stc-runtime is built independently and linked by the driver
```

`stc-codegen` must not depend on `stc-parser` or `stc-syntax`; it consumes HIR only.

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
- The parser never panics on arbitrary input; fuzzed.
- It records pragmas and attaches them to the following item; `{stc:test}` and `{stc:export}`
  become typed annotations.

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
| Struct | named LLVM struct, fields in declaration order |
| `OVERLAP` struct | byte buffer with typed accessors at checked offsets |
| `REF_TO T` | `ptr` |
| Class/FB instance | named LLVM struct |
| Interface variable | `%stc.iface = type { ptr, ptr }` (data, vtable) |
| Function | LLVM function |
| Program | LLVM function plus its instance body |

String representation is a **length-prefixed fixed-capacity buffer**:

- `STRING[n]`: `{ i32 len; [n x i8] data; }`, `len` in bytes/characters, no NUL requirement.
- `WSTRING[n]`: `{ i32 len; [n x i16] data; }`, `len` in 16-bit units.
- `USTRING[n]`: `{ i32 len; [n x i32] data; }`, `len` in characters (Unicode scalar values).
  Each character occupies 4 bytes, so the data area is exactly `4 * n` bytes and indexing is
  O(1). `LEN` reads `len`; `LEN_MAX` returns `n`; `LEN_CODE_UNIT` encodes/scans to count UTF-8
  code units for interchange. The in-memory size is `4 + 4 * n` bytes. UTF-8 is the
  source/interchange encoding, not the storage encoding.
- Copying a string copies the struct; `VAR_IN_OUT` passes a pointer.
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
- `AT` direct variables are not laid out as ordinary storage; they map to process-image offsets
  computed by the runtime descriptor.
- FB/class inheritance uses a **prefix layout**: a derived struct's first field is the base
  struct. Field access walks the prefix chain. This makes upcasting a no-op pointer adjustment.
- Interface references are fat pointers `{ptr data, ptr vtable}`. `NULL` is `{null, null}`.
- Reference values (`REF_TO`) are plain pointers; `NULL` is `null`.

### 4.5 Functions and calls

- A function lowers to an LLVM function with the target's C calling convention (the default).
- A function with a result uses the declared return type; a function without a result returns
  `void`. Assignment to the function name writes a dedicated result local; `RETURN` returns it.
- FB instances are structs. An FB body is a function:
  `void @stc_fb_TON(ptr %this, <inputs...>)` or, more generally, a call with the instance pointer
  first. Inputs are assigned to instance fields before the body runs; outputs are read from the
  instance after the call.
- Methods are functions with an explicit `this` pointer first:
  `void @stc_m_Class_Method(ptr %this, ...)`.
- Class/FB constructors are not separate user-visible functions; `stc_init` zero-initializes and
  applies declared initializers.
- Function calls with output parameters (`=>`) pass pointers to the destination; input
  parameters are passed by value unless declared `VAR_IN_OUT`.
- `VAR_IN_OUT` parameters are pointers to the actual variable's storage; aliasing is the user's
  responsibility per the ST semantics. Conformant `ARRAY [*]` parameters use the descriptor
  described above instead of a bare pointer.

### 4.6 OOP lowering

- Each class/FB type gets a named LLVM struct, a vtable global, and a type descriptor.
- Vtable slots are assigned deterministically: methods sorted by `(name, signature)` across the
  type's interface set; overrides reuse the base slot.
- Interface method calls use the interface vtable slot; class/FB calls through a base reference
  use the class vtable slot. Direct calls on a concrete instance use a direct function call unless
  the method is virtual and the reference is polymorphic.
- `OVERRIDE` replaces the implementation in the derived type's vtable.
- `SUPER.method()` emits a direct call to the base type's implementation (no vtable).
- `THIS` is the `this` pointer; `THIS` passed to an interface variable builds a fat pointer using
  the current type descriptor.
- `?=` (assignment attempt) calls a runtime helper that checks the source type descriptor against
  the destination interface/class descriptor:
  `ptr @stc_rt_attempt_cast(ptr %data, ptr %src_desc, ptr %dst_desc)` returning `{null,null}` or
  a valid fat pointer.
- `FINAL`/`ABSTRACT` are enforced statically; no runtime representation is needed beyond the
  vtable.

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

- Globals are emitted as internal globals with deterministic mangled names:
  `@stc_g_<ns>_<name>`.
- Constant initializers go in the global initializer; non-constant initializers are applied by
  `stc_init`.
- FB/class instance state is part of the global/local struct; `stc_init` zero-initializes and
  applies declared initial values.
- `stc_init` initializes the runtime process image and all global/static state.
- `stc_cycle` executes one scan of the configured programs/tasks.
- `stc_shutdown` flushes runtime state and releases resources.
- Executable mode generates:
  ```llvm
  define i32 @main(i32 %argc, ptr %argv) {
  entry:
    %rc = call i32 @stc_rt_execute(i32 %argc, ptr %argv, ptr @stc_init,
                                  ptr @stc_cycle, ptr @stc_shutdown)
    ret i32 %rc
  }
  ```
  `stc_rt_execute` owns the cycle loop, time base, cycle limit, signal handling, and error
  reporting. This keeps `main` tiny and the loop portable.
- Library mode exports:
  ```llvm
  define void @stc_init()
  define void @stc_cycle()
  define void @stc_shutdown()
  ```
  plus any `{stc:export}` wrappers and a metadata symbol. The host owns the loop and the I/O
  backend.

### 4.10 Name mangling

Mangled names are stable and C-identifier-safe:

```
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

- v1 emits source-level line locations (`!dbg`) for statements and calls when `--debug` is set.
- Full variable/type debug info (DWARF/CodeView) is a later milestone; the IR builder keeps
  source-span IDs so it can be added without changing HIR.
- Runtime traps report the source span ID and the runtime formats it using a source map emitted
  into the binary or a sidecar JSON file.

### 4.12 Determinism and reproducibility

- No hash-map iteration in emission paths; use sorted vectors or `BTreeMap`.
- Temporary names use a monotonic counter.
- Name hashes are deterministic.
- The module header includes the compiler version and profile ID, but not timestamps or paths.
- IR snapshots normalize target-specific differences; execution tests run on the host triple.

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

### 5.1 clang discovery

Order:

1. `--clang <path>` CLI option.
2. `STC_CLANG` environment variable.
3. `clang` on `PATH`.
4. A configured path in `stc.toml` (`[build] clang = "..."`).

`stc` runs `clang --version` and `clang -print-target-triple` once per build and reports a
diagnostic if clang is missing or too old. The minimum tested version is recorded in CI; the
project targets IR syntax accepted by LLVM 15 and later.

### 5.2 Commands

| Artifact | Command shape |
| --- | --- |
| IR only | `stc build --emit=ir` writes `<name>.ll`; no clang call |
| Object | `clang --target=<triple> -x ir <name>.ll -c -o <name>.o -O<level> [-g]` |
| Executable | `clang --target=<triple> <name>.ll <runtime_staticlib> -o <name>.exe -O<level> [-g]` |
| Static library | `clang ... -c` then `llvm-ar rcs <name>.a <objects>` plus the runtime object |
| Shared library | `clang --target=<triple> -shared <name>.ll <runtime> -o <name>.dll/.so` |

- On Windows MSVC targets, clang uses `link.exe` or `lld-link`; the Rust runtime staticlib is
  built with the same MSVC toolchain (`*-pc-windows-msvc`).
- The runtime staticlib path is discovered from the workspace build output or `STC_RUNTIME`.
- Linking errors are surfaced as build errors with the clang command (verbosity-gated) and the
  generated IR retained for inspection.

### 5.3 Artifact layout

```
target/stc/<profile>/
  <name>.ll
  <name>.o
  <name>.exe | <name>.dll/.so | <name>.a
  <name>.stc-meta.json     runtime descriptor, exports, source map
  build.log                verbose log (if -v)
```

The `.stc-meta.json` includes the selected configuration, task list, exported symbols, string
capacities, and source-span table for runtime diagnostics.

## 7. Runtime boundary

The generated module calls a narrow C ABI:

| Symbol | Purpose |
| --- | --- |
| `stc_rt_execute(argc, argv, init, cycle, shutdown)` | Run the executable loop |
| `stc_rt_trap(code, span_id)` | Trap with a diagnostic |
| `stc_rt_assert(cond, span_id)` | User assertion (test/development) |
| `stc_rt_cycle_now_ns()` | Cycle time latched at `stc_cycle` entry |
| `stc_rt_monotonic_now_ns()` | Live monotonic clock (runtime/host only) |
| `stc_rt_set_cycle_time_ns()` | Override the next cycle's time (host/tests) |
| `stc_rt_io_image(ptr*, len*)` | Process-image access |
| `stc_rt_attempt_cast(data, src_desc, dst_desc)` | Interface/reference downcast |
| `stc_rt_len_*`, `stc_rt_str_*` | String helpers |
| `stc_rt_math_*` | Math helpers (where not inlined) |
| `stc_rt_trace(...)` | Optional tracing |

The full list and signatures are in [Runtime and Execution](Runtime-and-Execution.md).

## 8. Security and robustness

- The compiler treats source as untrusted input: no panics, bounded recursion, bounded memory.
- Parser/analyzer recursion depth is limited; deeply nested expressions produce a diagnostic
  rather than a stack overflow.
- Generated code is native and unsandboxed; `stc test` runs untrusted test code in a separate
  process with a timeout. This is a development tool, not a security boundary.
- Paths in `stc.toml` are resolved relative to the manifest; path traversal outside the project is
  allowed only for explicit user paths and is reported in verbose output.
- The runtime does not load plugins or execute ST at runtime.

## 9. Open architecture questions

- Q-006: I/O backend for direct variables (memory, file, callbacks) in executable mode.
- Q-007: RETAIN/PERSISTENT storage in executable mode.
- Q-010: Whether to support a `--strict` mode that disables the semicolon relaxation and other
  supersets.
