# Runtime and Execution Model

Status: Draft  
Profile: `stc-2013-oop-utf8-v1`  
Related: [Architecture](Architecture.md), [Conformance](../01-Conformance-Profile.md), [Decisions](../Decisions.md)

This document defines how generated code executes, what the runtime library provides, and how
executable and library artifacts differ.

## 1. Execution model

IEC 61131-3 programs execute cyclically. The generated code has three entry points:

```
stc_init()      initialize runtime, globals, FB/class instances, process image
stc_cycle()     execute one scan of the configured tasks/programs
stc_shutdown()  flush state and release resources
```

### 1.1 Executable mode

`stc build --emit=exe` emits an executable whose generated `main` calls
`stc_rt_execute(argc, argv, init, cycle, shutdown)`. The runtime owns the loop:

1. Parse runtime arguments (`--cycles`, `--cycle-time`, `--io`, `--trace`).
2. Initialize the process image and clock.
3. Call `stc_init`.
4. Repeat until the cycle limit is reached or a stop signal is received:
   - latch inputs into the input image,
   - call `stc_cycle`,
   - write outputs from the output image,
   - sleep until the next cycle deadline if a cycle time is configured,
   - check the watchdog.
5. Call `stc_shutdown`.
6. Return exit code 0 on normal completion, non-zero on a trap or configuration error.

Default behavior without `--cycles`: run until interrupted. Tests always pass a finite cycle
count or use library mode.

### 1.2 Library mode

`stc build --emit=lib` emits a static or shared library with no `main`. It exports:

```c
void stc_init(void);
void stc_cycle(void);
void stc_shutdown(void);
uint32_t stc_abi_version(void);
const char* stc_metadata_json(void);
```

plus any `{stc:export}` wrappers. The host owns the loop, I/O, and clock. `stc_cycle` may be
called from a host thread; v1 is not thread-safe and requires external serialization.

### 1.3 Task model

- The configuration declares resources, tasks, and program instances.
- `stc_cycle` executes the selected configuration's programs according to the task schedule.
- **v1 implementation**: one task, one interval, programs executed in declaration order. A
  configuration with multiple tasks or priorities is diagnosed with `STC-E-RUN-001` at build time
  unless `--allow-multi-task` is given (which runs them sequentially in priority order for
  testing only).
- The HIR keeps the full task/program mapping, so a real scheduler can be added without changing
  the language front end.
- `SINGLE` tasks (run once) execute on the first cycle and are marked complete.
- `INTERVAL` tasks execute when their deadline is reached; v1 checks deadlines once per scan.

## 2. Runtime library

### 2.1 Implementation

- `crates/stc-runtime` is a Rust crate built as `staticlib` and optionally `cdylib`.
- It exposes a C ABI (`extern "C"`, `#[no_mangle]`, `#[repr(C)]`).
- No Rust types cross the ABI except `u8`/`i32`/`i64`/`f32`/`f64`/pointers.
- The runtime is built with the same MSVC toolchain as the target on Windows.
- Panics are caught at the ABI boundary and converted to `stc_rt_trap` where possible; the
  compiler-generated code does not rely on unwinding.

### 2.2 ABI versioning

- `STC_RUNTIME_ABI = 1` for v1.
- The runtime exposes `stc_abi_version()`.
- Generated modules embed the expected ABI version in metadata; the driver refuses to link
  mismatched runtime versions.
- Additive changes bump the minor version; breaking signature changes bump the major version.

### 2.3 Core ABI

```c
typedef void (*stc_fn)(void);

int32_t stc_rt_execute(int32_t argc, char** argv,
                       stc_fn init, stc_fn cycle, stc_fn shutdown);

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

Traps are fatal for the current run. In executable mode the runtime prints
`<file>:<line>:<column>: error[STC-RUN-xxx]: message` using the source map; in library mode it
returns a non-zero status to the host and records the last trap.

### 2.4 Process image and direct variables

Direct variables (`%I`, `%Q`, `%M`) map into a contiguous process image:

- `%I` input bytes at the start, then `%Q` output bytes, then `%M` marker bytes.
- The compiler computes offsets and sizes from the declarations; the layout is recorded in
  `<name>.stc-meta.json`.
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
  Generated ST code does not call it.
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
- `llvm.*` intrinsics where semantics match (`llvm.sqrt`, `llvm.fabs`, `llvm.fshl`).

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

- The stdlib is injected as a prelude; its POUs live in a reserved namespace (`STC_STD`).
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

See [Architecture](Architecture.md) §4.3. Strings are fixed-capacity, length-prefixed structs.

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
- **v1 executable mode**: retained values are persisted to a versioned state file. A valid file
  is a warm restart; a missing file or `--reset-retain` is a cold restart. Layout/profile/version
  mismatches and CRC failures follow `on-mismatch = error|reset` (default `error`). Saves are
  atomic and happen at normal shutdown.
- **v1 library mode**: the same behavior is available through `stc_retain_size/save/load` and
  `stc_rt_state_*`; the host may use the file helpers or its own storage and policy.
- The state file contains only persistable leaves; references/interfaces/pointers and vtable
  pointers are excluded (vtable/type pointers are reinitialized before load). Direct `AT`
  variables are excluded with `STC-W-RETAIN-001`.
- `PERSISTENT` is not a 2013 keyword; it is out of profile unless added as a documented extension.
- Full format, hashing, ABI, and diagnostics: [State and RETAIN](State-and-Retain.md).

## 6. Traps and diagnostics

### 6.1 Trap behavior

- Traps are deterministic and fatal for the current cycle/run.
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

## 9. Open runtime questions

None currently. New runtime questions are appended here or in [Decisions](../Decisions.md).

