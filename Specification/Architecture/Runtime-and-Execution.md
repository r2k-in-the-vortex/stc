# Runtime and Execution Model

Status: M4 runtime implemented; online-change architecture resolved (2026-10-24)  
Profile: `stc-2013-oop-utf8-v1`  
Related: [Architecture](Architecture.md), [Online Change and GOT](Online-Change-and-GOT.md), [Debug and Monitoring](Debug-and-Monitoring.md), [Conformance](../01-Conformance-Profile.md), [Decisions](../Decisions.md)

This document defines how generated code executes, what the host runtime provides, and how
module, executable, and embedding-library artifacts differ.

> **M4 status:** `crates/stc-runtime` implements the C ABI, the `--cycles N` executable loop,
> the latched cycle clock, the process image (`stc_rt_io_configure`/`stc_rt_io_image`), trap
> reporting (exit 70), and retained-state stubs that return 5 (disabled). It uses `std` and is
> the only crate permitted unsafe code (D-077). `stc_rt_assert`, the memory/file I/O backends,
> vtable/type descriptors, and file-backed state are later milestones. `stc_abi_version` is
> runtime-owned; generated library modules export `stc_init`, `stc_cycle`, `stc_shutdown`,
> `stc_metadata_json`, export wrappers, and the retain functions, but not `stc_abi_version`.
>
> **Post-M4 architecture (resolved):** the runtime becomes a persistent host that owns the GOT,
> root allocator, module loader, manifest store, swap controller, and service threads. Host ABI 2
> adds attach/prepare/detach, GOT/cell access, the reference resolver, and versioned host services.
> The M4 `staticlib` remains as the embedding/host library; the direct-reference executable model
> is superseded by a thin launcher plus loadable module.

## 1. Execution model

IEC 61131-3 programs execute cyclically in the **persistent host**. The host owns the cycle loop,
GOT, persistent roots, process image, retained state, module lifetime, and services. A loadable
module exports the versioned entry points:

```text
stc_attach(host_api, got_context, descriptor)   receive host services and the GOT context
stc_prepare(mode, old_manifest, new_manifest)   validate/build candidate roots and temp GOT
stc_cycle()                                     execute one scan of the configured tasks/programs
stc_shutdown()                                  flush state and release module-local resources
stc_detach()                                    release module-local resources before unload
```

Exact signatures and the host API table are fixed by ABI 2 in §2.3. `stc_init` remains the
generated initialization entry used during prepare.

### 1.1 Module and executable mode

`stc build --emit=module` emits `<name>-<build-id>.dll`/`.so` plus its GOT manifest. The module
has no `main` and no persistent storage of its own.

`stc build --emit=exe` emits a thin host launcher whose `main` calls
`stc_rt_execute(argc, argv, descriptor_path)`, together with the adjacent module and manifest
pair it loads. The host:

1. Parses runtime arguments (`--cycles`, `--cycle-time`, `--io`, `--trace`, `--module`).
2. Loads and validates the active module and manifest pair.
3. Initializes the process image, clock, GOT cells, and root allocator.
4. Attaches the module and runs its preparation (cold start or online attach).
5. Repeats until the cycle limit is reached or a stop signal is received:
   - latch inputs into the input image,
   - apply queued debug writes/forces at the boundary,
   - check for a prepared candidate and switch generations if ready,
   - call `stc_cycle`,
   - write outputs from the output image,
   - sleep until the next cycle deadline if a cycle time is configured,
   - check the watchdog and service queues.
6. Runs shutdown, saves retained state, and unloads generations.
7. Returns exit code 0 on normal completion, non-zero on a trap or configuration error.

Default behavior without `--cycles`: run until interrupted. Tests always pass a finite cycle
count or use library mode.

### 1.2 Embedding library mode

`stc build --emit=lib` emits an embedding host library with no `main`. It exports the host API
plus any `{stc:export}` wrappers:

```c
uint32_t stc_abi_version(void);
const char* stc_metadata_json(void);
int32_t stc_host_run(stc_host_options* options);
int32_t stc_host_load_module(const char* path, stc_load_mode mode);
int32_t stc_host_export_manifest(char* buffer, int64_t* len);
```

The generated module remains a separate loadable artifact; M4.5 emits the embedding host as a
static library, and shared host libraries are M6. The embedding host owns the loop, I/O, clock,
GOT, root allocator, manifest store, loader/prepare thread, and service/monitor threads.
`stc_cycle` is called only by the host's cycle thread; external callers must serialize all host
API calls. The runtime is not a general-purpose thread-safe library: service access goes through
the typed generation-pinned API, and the cycle thread is the only ST executor.

### 1.3 Task model

- The configuration declares resources, tasks, and program instances.
- `stc_cycle` executes the selected configuration's programs according to the task schedule.
- **v1 implementation**: one task, one interval, programs executed in declaration order. A
  configuration with multiple tasks or priorities is diagnosed with `STC-E-RUN-001` at build time
  unless `--allow-multi-task` is given (which runs them sequentially in priority order for
  testing only).
- The HIR keeps the full task/program mapping, so a real scheduler can be added without changing
  the language front end.
- Task execution state (`SINGLE` completion, deadlines, per-task flags) is **host-owned and keyed
  by task identity**. It is preserved across an online change when the task/resource topology is
  unchanged and reset on a cold download.
- `SINGLE` tasks (run once) execute on the first cycle and are marked complete. The completion
  flag survives a compatible swap.
- `INTERVAL` tasks execute when their deadline is reached; v1 checks deadlines once per scan. A
  task/resource topology change requires a cold download; it is never introduced mid-schedule.

## 2. Runtime library

### 2.1 Implementation

- `crates/stc-runtime` is a Rust crate built as `staticlib` and optionally `cdylib`. It is the
  persistent host as well as the embedding library.
- It exposes a C ABI (`extern "C"`, `#[no_mangle]`, `#[repr(C)]`) through a versioned host API
  table, not a flat set of mandatory exported symbols.
- No Rust types cross the ABI except `u8`/`i32`/`i64`/`f32`/`f64`/pointers/opaque handles.
- The runtime is built with the toolchain that matches the target C ABI: MSVC for
  `*-pc-windows-msvc`, GNU for `*-unknown-linux-gnu`. Windows x86-64 is the primary development
  host; Linux x86-64 and AArch64 follow basic development (D-042).
- Panics are caught at the ABI boundary and converted to `stc_rt_trap` where possible; the
  compiler-generated code does not rely on unwinding.
- The host owns the GOT cell table, root allocator, module loader, manifest store, swap
  controller, process image, retained state, and service/monitor threads. Module code owns only
  code and version-local read-only data.
- Host state is protected by a control lock used only by host control paths, never held across
  ST execution. Service readers use generation pinning/epoch refcounts.

### 2.2 ABI versioning

- `STC_HOST_ABI_MAJOR = 2`, `STC_HOST_ABI_MINOR = 0` for v1's online-change host.
- The runtime exposes `stc_abi_version()` and a host API struct with `abi_version` and
  `struct_size`. A module rejects a host whose struct is shorter than required.
- Generated modules embed the expected ABI version in metadata and their descriptor; the loader
  rejects mismatched modules before attach.
- Additive changes bump the minor version; breaking signature/struct changes bump the major
  version. M4 ABI 1 artifacts are not loadable.

### 2.3 Core ABI

```c
typedef void (*stc_fn)(void);

int32_t stc_rt_execute(int32_t argc, char** argv, const char* module_descriptor_path);

typedef enum {
    STC_TRAP_OVERFLOW = 1,
    STC_TRAP_DIV_ZERO = 2,
    STC_TRAP_RANGE = 3,
    STC_TRAP_BOUNDS = 4,
    STC_TRAP_NULL_REF = 5,
    STC_TRAP_INVALID_UTF8 = 6,
    STC_TRAP_CONVERSION = 7,
    STC_TRAP_WATCHDOG = 8,
    STC_TRAP_ASSERT = 9,
    STC_TRAP_UNIMPLEMENTED = 10
} stc_trap_code;

void stc_rt_trap(int32_t code, uint32_t span_id);
void stc_rt_assert(int32_t condition, uint32_t span_id, const char* message); /* test builds only */

int64_t  stc_rt_cycle_now_ns(void);      /* latched at stc_cycle entry */
uint64_t stc_rt_monotonic_now_ns(void);  /* live clock; runtime/host only */
void     stc_rt_set_cycle_time_ns(int64_t ns); /* override next cycle */
int64_t  stc_rt_local_date_ns(void);

int32_t stc_rt_state_load(void* buffer, int64_t len, int64_t layout_hash);
int32_t stc_rt_state_save(const void* buffer, int64_t len, int64_t layout_hash);
void    stc_rt_state_configure(const char* path, int32_t policy, int32_t enabled);

void stc_rt_io_image(uint8_t** base, uint64_t* len);
void stc_rt_set_io_backend(const stc_io_backend* backend);
```

#### 2.3.1 Host API table

The exact ABI is a versioned `stc_host_api` struct. Its entries include trap/assert, clocks,
state, I/O, transient alloc/free, string/math helpers, GOT cell access, reference resolution,
object handle/descriptor access, typed variable access, and generation pin/unpin. Modules receive
the table in `stc_attach`; they do not import platform symbols from the host executable.

```c
typedef struct stc_host_api {
    uint32_t abi_version;   /* major/minor packed */
    uint32_t struct_size;   /* bytes; modules reject a too-small table */
    void (*trap)(int32_t code, uint32_t span_id);
    int64_t (*cycle_now_ns)(void);
    uint64_t (*monotonic_now_ns)(void);
    void (*set_cycle_time_ns)(int64_t ns);
    void* (*alloc)(uint64_t size, uint64_t align);
    void (*free)(void* ptr);
    void* (*got_cell)(uint32_t slot);          /* active generation's cell */
    int32_t (*resolve_ref)(const void* ptr, uint32_t type_key,
                           stc_place_descriptor* out);
    int32_t (*attempt_cast)(const stc_object_ref* src,
                            const stc_type_descriptor* dst,
                            stc_object_ref* out);
    /* state/IO/string/math/typed-access entries follow */
} stc_host_api;
```

#### 2.3.2 Module descriptor and GOT context

The module descriptor records the module format version, host ABI, profile, target, build ID,
paired manifest digest, entry-point addresses, sizes, and the slot-table count. `stc_attach`
receives the host API and a GOT context that maps local slots to host cells. Generated code
resolves a slot through the context or a cached storage pointer; it never writes a cell.

```c
typedef struct stc_module_descriptor {
    uint32_t module_format;
    uint32_t host_abi;
    uint32_t profile_hash;
    uint32_t target_hash;
    uint64_t build_id[2];
    uint64_t manifest_digest[2];
    uint32_t slot_count;
    stc_fn attach, prepare, cycle, shutdown, detach;
} stc_module_descriptor;
```

#### 2.3.3 Generation pinning and services

Service/monitor threads read typed variables through generation-pinned descriptors. A pin is an
epoch/refcount that delays unload until the reader releases it. Service threads never call
`stc_cycle`, never write GOT cells, and never take a lock held by the cycle thread. Debug writes
and forcing are queued for cycle-boundary application.

Traps are fatal for the current run or, during an online-change rollback window, cause the host
to restore the previous generation. In executable mode the runtime prints
`<file>:<line>:<column>: error[STC-RUN-xxx]: message` using the source map; in library mode it
returns a non-zero status to the host and records the last trap.

### 2.4 Process image and direct variables

Direct variables (`%I`, `%Q`, `%M`) map into a contiguous process image:

- `%I` input bytes at the start, then `%Q` output bytes, then `%M` marker bytes.
- The compiler computes offsets and sizes from the declarations; the layout is recorded in
  `<name>.stc-meta.json` and the manifest.
- Each direct `AT` variable/field has a per-generation GOT cell pointing into the process image.
  Generated code accesses the variable through the cell, not a cached raw image pointer, so an
  online change can remap the location.
- On an online change the host reconfigures the image, re-evaluates bindings, preserves `%M` and
  same-address variables, and warns about remapped variables. A binding/topology change that
  cannot be preserved requires a cold download.
- The runtime provides default input/output image buffers and a `stc_io_backend` vtable:

  ```c
  typedef struct {
      void* user;
      int32_t (*read_inputs)(void* user, uint8_t* dst, uint64_t len);
      int32_t (*write_outputs)(void* user, const uint8_t* src, uint64_t len);
  } stc_io_backend;
  ```

- Default backend (`null`): unbound inputs are filled with the type's default value each cycle
  (from `input_defaults_hex`), bound inputs are read from their backend, and unbound outputs are
  discarded at cycle end. Inputs with no linking therefore see type defaults; outputs with no
  linking do nothing externally. `%M` markers persist.
- Tests use a memory backend with explicit input/output images; unbound inputs still receive
  defaults unless the test overrides them.
- Hardware backends (EtherCAT, Modbus, MQTT) and their schedules are v2; the compiler only emits
  process-image metadata and binding hints. See [I/O and Devices](IO-and-Devices.md).
- Direct variables inside functions/FB bodies are deprecated by the 2013 standard; `stc` accepts
  them with a warning (`STC-W-RUN-001`) and documents that they refer to the same process image.

### 2.5 Clock and cycle-latched time

- `stc_cycle` reads the time source **once at entry** and stores `cycle_now_ns` for the whole
  scan. Every ST-visible time function and standard FB uses this latched value. Two timers
  evaluated in the same scan cannot observe different times (D-027).
- `stc_rt_cycle_now_ns()` returns the latched time as signed 64-bit nanoseconds. This is what
  generated ST code calls.
- `stc_rt_monotonic_now_ns()` returns the live monotonic clock and is for the runtime/host only.
  Generated ST code does not call it. The runtime reads it through a host abstraction
  (`QueryPerformanceCounter` on Windows, `clock_gettime(CLOCK_MONOTONIC)` on Linux) and converts
  to nanoseconds; the ST-visible semantics are identical on every host (D-042).
- The clock remains continuous across an online change. The host latches the next cycle's time
  normally; the swap does not reset or re-latch it, so timers observe elapsed wall/monotonic time.
- The watchdog is paused or extended with a documented bound during prepare/activation and while
  a debugger has the cycle stopped. If the bound is exceeded, the normal watchdog policy resumes.
- In executable mode, the runtime latches the host monotonic clock at the start of each cycle.
  `--cycle-time` controls the loop deadline but does not change the value latched for the scan.
- In library mode, the host owns the loop. By default `stc_cycle` latches the live monotonic
  clock; the host can call `stc_rt_set_cycle_time_ns(t)` before `stc_cycle` to supply the next
  cycle's time. The override is consumed by the next `stc_cycle` call.
- Tests use a mock clock with the same semantics: the harness sets the next cycle time, calls
  `stc_cycle`, and advances the mock clock between cycles. `STC_CLOCK_ADVANCE(T#10ms)` schedules
  the next cycle's time; it does not change the time already latched in the current scan.
- `TIME`/`LTIME` are signed 64-bit nanoseconds. `TON`/`TOF`/`TP` compute elapsed time from
  `cycle_now_ns` and their captured start time. Arithmetic overflow traps.
- Date/time functions use `cycle_now_ns` converted to the profile's epoch. Within a scan,
  `NOW()`/`TIME()` are constant.
- Time-zone handling is profile-defined; v1 uses local time only and records the choice.

## 3. Standard library

### 3.1 Standard functions

Standard functions are available through the predeclared `STC_STD` namespace, implicitly
imported. They are resolved by the analyzer and emitted as:

- inline IR for trivial exact operations (`ABS`, `MIN`, `MAX`, `SEL`, bit operations),
- runtime calls for string operations, conversions with checks, date/time, and math,
- `llvm.*` intrinsics where semantics match (`llvm.sqrt`, `llvm.fabs`, `llvm.fshl`),
- calls to user/standard functions through host-owned function cells under `STC_STD` with stable
  keys, so an online change can rebind them without rewriting code.

Categories and ownership:

| Category | Implementation |
| --- | --- |
| Type conversions | Runtime helpers + inline checks |
| Integer arithmetic | Inline checked IR (`*.with.overflow`) + traps |
| Bitwise/shift/rotate | Inline IR, `llvm.fshl`/`llvm.fshr` |
| Selection/comparison | Inline IR |
| String functions | Runtime (`stc_rt_str_*`), UTF-8 aware for `USTRING` |
| Date/time | Runtime (`stc_rt_time_*`) |
| Math (float) | Runtime calling the platform libm; documented edge cases |
| Exact math | Runtime or inline, backed by Lean models and vectors |

### 3.2 Standard function blocks

These are implemented in ST under `stdlib/` and compiled with the user project:

`SR`, `RS`, `R_TRIG`, `F_TRIG`, `CTU`, `CTD`, `CTUD`, `TP`, `TON`, `TOF`.

- The stdlib is injected as a prelude; its POUs live in a reserved namespace (`STC_STD`) and are
  compiled into each module with stable GOT keys. The host does not require a separately loaded
  stdlib module in v1.
- User code can use the standard names directly; the analyzer resolves them after local/global
  scopes, so a user-defined `TON` shadows the stdlib (with a warning).
- The stdlib depends only on `stc_rt_cycle_now_ns()` and exact arithmetic. It must not read the
  live monotonic clock.
- All timers in one scan use the same latched cycle time, so two `TON`/`TOF`/`TP` instances
  evaluated in the same cycle cannot produce different elapsed times (D-027).
- Timer behavior is defined in `Specification/Stdlib/Timers.md` (future document) and tested with
  a mock clock and deterministic step sequences.

### 3.3 Prelude and name collisions

- The prelude is not textual; it is a set of HIR definitions injected by `stc-sema`.
- Standard function names are ordinary identifiers and may be shadowed. Keywords remain reserved.
- The compiler warns when a user POU shadows a standard FB or function (`STC-W-NAME-002`).

## 4. Strings

### 4.1 Layout

See [Architecture](Architecture.md) §4.3. Strings are fixed-capacity, length-prefixed **host
roots**: one root per string value, owned by the host, accessed through the GOT. String
assignment copies the root's value; the destination root identity is preserved.

### 4.2 Operations

- Copy: memcpy of the struct; the runtime normalizes the length to the destination capacity.
- Concatenation: runtime checks the destination capacity; overflow is a trap (`STC_TRAP_RANGE`).
- `LEN` (characters for `USTRING`, units for `STRING`/`WSTRING`).
- `LEN_CODE_UNIT` (UTF-8 code units when encoded for interchange), `LEN_MAX` (declared capacity in
  characters for `USTRING`).
- Indexing is 1-based and O(1) for `USTRING` because storage is a 32-bit scalar array; bounds are
  checked.
- `LEFT`/`RIGHT`/`MID`/`INSERT`/`DELETE`/`REPLACE`/`FIND` use the appropriate unit (code points
  for `USTRING`, characters for `STRING`, 16-bit units for `WSTRING`).
- Invalid UTF-8 cannot be constructed through the language; `BYTES_TO_USTRING` validates and
  traps on invalid sequences. Raw memory manipulation is not exposed.

### 4.3 Encoding rules

- `STRING` is 2013 single-byte; raw source characters are ASCII-only, escapes produce bytes.
- `WSTRING` is UTF-16 code units; surrogate pairs are valid; lone surrogates are rejected by
  conversions and string functions.
- `USTRING` stores 32-bit Unicode scalars; the stored length is characters, and the data area is
  `4 * capacity` bytes. UTF-8 is the source/interchange encoding; `BYTES_TO_USTRING` decodes and
  validates, and `USTRING_TO_BYTES` encodes. No Unicode normalization is performed.

### 4.4 Conformant arrays

- `ARRAY [*]` is supported for `VAR_IN_OUT` and FB input/output parameters (A-007).
- The parameter is passed as a descriptor `{ ptr data, i64 len, [rank x bound] }`; the descriptor
  points at caller-owned static storage. The runtime allocates nothing.
- Bounds are checked on every access; the callee must not retain the pointer past the call.
- Dynamically sized locals, globals, and function results are rejected at analysis time with
  `STC-E-TYPE-*`; there is no runtime allocation path for them in v1.

## 5. RETAIN and PERSISTENT

- `RETAIN`/`NON_RETAIN` are parsed and analysed; the compiler computes an effective policy per
  storage leaf (explicit qualifier, section qualifier, instance inheritance, struct/array
  inheritance).
- **Cold start**: retained values are persisted to a versioned state file. A valid file is a warm
  restart; a missing file or `--reset-retain` is a cold restart. Layout/profile/version
  mismatches and CRC failures follow `on-mismatch = error|reset` (default `error`). Saves are
  atomic and happen at normal shutdown.
- **Online attach**: the state file is not loaded; in-memory values win. Matching per-field roots
  are preserved, new roots are initialized, and removed roots are quarantined. V1 does not migrate
  retained leaves by `GotId` across restarts; the state file keeps its layout-hash policy. A
  later state-format revision may key leaves by `GotId` plus path.
- **Cold download**: non-retained state is initialized, `RETAIN` values are restored only under
  the existing compatibility policy, and transient allocations are not preserved.
- **Embedding host**: the same behavior is available through `stc_retain_size/save/load` and
  `stc_rt_state_*`; the host may use the file helpers or its own storage and policy.
- The state file contains only persistable leaves; references/interfaces/pointers and vtable
  pointers are excluded (vtable/type pointers are reinitialized before load). Direct `AT`
  variables are excluded with `STC-W-RETAIN-001`.
- `PERSISTENT` is not a 2013 keyword; it is out of profile unless added as a documented extension.
- Full format, hashing, ABI, and diagnostics: [State and RETAIN](State-and-Retain.md).

## 6. Traps and diagnostics

### 6.1 Trap behavior

- Traps are deterministic and fatal for the current cycle/run. If the trap occurs in a new
  generation during the one-successful-cycle rollback window, the host restores the previous
  generation and records the failure; after commit, a trap follows the normal fatal policy.
- The runtime prints a diagnostic with the source span (from the embedded span table), the trap
  code, and a short message.
- In library mode, `stc_rt_last_trap()` returns a struct with code, span, and message; the host
  decides whether to abort or continue.
- Traps are not catchable in ST. `ASSERT` is not an Ed. 4 feature in this profile; the runtime
  helper exists for tests only (`{stc:test}` harness).

### 6.2 Span table

- The compiler emits a compact `span_id -> (file, start, end)` table and file path table into
  `.stc-meta.json` and, optionally, an embedded section.
- Executable mode embeds the table; library mode exposes it via `stc_metadata_json`.
- Span IDs are stable within a build, not across builds; the metadata includes a build hash.

## 7. Test execution

- `stc test` compiles the project plus test sources and links a test runner.
- Each test runs in a separate process with a timeout (default 5 s) to isolate hangs and traps.
- The runner provides:
  - a fresh runtime (`stc_init`) per test,
  - a mock clock,
  - a memory I/O backend,
  - assertion helpers that record failures without aborting immediately.
- Test functions return `BOOL` (TRUE = pass) and may call `STC_ASSERT`.
- Program/FB tests may run a configured number of cycles (`cycles = N` in the test metadata) and
  inspect outputs/globals.
- Results are reported as human-readable text or JSON; exit code 0 if all tests pass.

Detailed test discovery and pragma syntax are in [Testing](Testing-and-Verification.md).

## 8. Error handling policy

| Situation | Behavior |
| --- | --- |
| Compile-time type/name error | Diagnostic, no codegen |
| Unsupported language construct | Diagnostic with the profile reference |
| Runtime overflow | Trap `STC_TRAP_OVERFLOW` |
| Division/modulo by zero | Trap `STC_TRAP_DIV_ZERO` |
| Array/string bounds | Trap `STC_TRAP_BOUNDS` |
| Invalid UTF-8 conversion | Trap `STC_TRAP_INVALID_UTF8` |
| Null dereference | Trap `STC_TRAP_NULL_REF` |
| Watchdog timeout | Trap `STC_TRAP_WATCHDOG` |
| Missing runtime symbol at link | Build error with the missing symbol and the IR path |
| Module descriptor/ABI/target mismatch | Reject the candidate before attach; keep the active generation; non-fatal |
| Missing/invalid/stale manifest baseline | Cold download; no merge or partial recovery |
| Candidate prepare failure | Discard temp GOT/module; active generation continues |
| Trap in the rollback window | Restore the previous generation; record the failure |
| Debug session / forced value | Documented hold/force behavior; not a safety function |
| Service request for a removed field/generation | Reject or retry against the active generation; never target removed storage |

## 9. Open runtime questions

None as of 2026-10-24. The online-change host/GOT/manifest/debug architecture is resolved in
[Online Change and GOT](Online-Change-and-GOT.md),
[Debug and Monitoring](Debug-and-Monitoring.md), and `update_plan_got.md` §13.0. New runtime
questions are appended here or in [Decisions](../Decisions.md).
