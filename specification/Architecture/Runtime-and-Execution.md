# Runtime and Execution Model

Status: M4.5 persistent host and ABI 2 module execution, M5 OOP descriptors, M6 scheduler, run loop, file-backed state, I/O, and control-channel work, M6.5 debug/monitoring, and M7 release packaging/hardening implemented (2026-10-28)  
Profile: `stc-2013-oop-utf8-v1`  
Related: [Architecture](Architecture.md), [Online Change and GOT](Online-Change-and-GOT.md), [Debug and Monitoring](Debug-and-Monitoring.md), [Conformance](../01-Conformance-Profile.md), [Decisions](../Decisions.md)

This document defines how generated code executes, what the host runtime provides, and how
module, executable, and embedding-library artifacts differ.

> **M4.5 status (2026-10-25):** `crates/stc-runtime` is the persistent host. It owns the GOT,
> root allocator and quarantine, module loader, manifest store, process image, swap controller,
> generation pins, and the load/prepare/activate/commit/rollback/unload state machine. Generated
> code executes as an ABI 2 module: persistent scalars, program fields, direct `AT` variables,
> retained leaves, and task guards use GOT cells; user calls are indirect through function cells;
> host services use import cells; and the module exports the seven entry-point set. The host reads
> `<name>.stc-meta.json` to configure the process image and bind direct cells, reconfigures or
> remaps the image at activation, and preserves direct values by `GotId`. `--emit=exe` emits the
> thin launcher; `--emit=lib` emits the embedding archive with the module separate.
>
> The trap service performs a non-local exit to the host boundary (`__intrinsic_setjmpex` on
> Windows, `_setjmp` on POSIX; generated functions are `uwtable`; an assembly shim owns the
> second `setjmp` return), enabling rollback-window recovery and exit code 70 outside it. The
> embedding C ABI exposes `stc_host_load_module`, `stc_host_cycle`, `stc_host_shutdown`,
> `stc_host_symbol`, `stc_host_export_manifest`, `stc_metadata_json`, `stc_rt_execute`, and the
> M6 phase-4 `stc_host_run` with its `stc_host_options` struct. The launcher and embedding hosts
> share one run loop.
>
> M6 phase 4 added the cooperative task scheduler and the project run workflow: the host owns
> task run state keyed by task `GotId`, writes each `TaskGuard` before every scan, applies the
> interval/`SINGLE`/no-catch-up rules, preserves state across compatible swaps, and resets it on
> a cold install. The launcher parses the run options, sleeps to the cycle deadline, installs
> stop handlers, and returns 0/1/2/3/4/130; `stc run` builds `exe,module,metadata` and returns
> the launcher's exit code.
>
> The runtime uses `std` and is the only crate permitted unsafe code (D-077). The ABI 2 state
> services and the legacy `stc_rt_state_*` symbols are file-backed from M6 phase 5.
> `stc_rt_assert` remains available through the host API for the M6 test
> runner and M5 hosted checks but is unused by M4.5-generated code. Full typed debug/monitoring
> is implemented in M6.5; the interface it builds on is recorded in
> [Debug and Monitoring](Debug-and-Monitoring.md) §12 (historical boundary).

## 1. Execution model

IEC 61131-3 programs execute cyclically in the **persistent host**. The host owns the cycle loop,
GOT, persistent roots, process image, retained state, module lifetime, and services. A loadable
module exports the versioned entry points:

```text
stc_module_attach(host_api, slots, slot_count, generation_id)  receive host services and cells
stc_module_prepare(mode, generation_id, fresh_mask, mask_size) validate/build candidate roots
stc_module_cycle(generation_id)                                execute one scan
stc_module_shutdown(mode)                                      flush state/release resources
stc_module_detach(generation_id)                               release resources before unload
```

`stc_module_descriptor` and `stc_module_functions` expose the versioned descriptor and code
address table without host services. Exact signatures, descriptor bytes, slot-table rules, and
the host API table are fixed by [GOT Artifacts and Schemas](GOT-Artifacts.md) §7–§8. `stc_init`
remains the generated initialization entry used during prepare.

### 1.1 Module and executable mode

`stc build --emit=module` emits `<name>-<build-id>.dll`/`.so` plus its GOT manifest. The module
has no `main` and no persistent storage of its own.

`stc build --emit=exe` emits a thin host launcher whose `main` calls
`stc_rt_execute(argc, argv, module_path)`, together with the adjacent module and manifest
pair it loads. The host:

1. Parses runtime arguments: `--cycles <n>` (absent or `0` runs until interrupted),
   `--cycle-time <duration>`, `--io null|memory|file`, `--io-file <path>`,
   `--state-file <path>`, `--reset-retain`, `--no-retain`, `--mock-clock <duration>`,
   `--status`, and `--module <path>`. `--trace` is rejected with a clear error. `stc run`
   resolves `[run].cycles` and `[run].cycle-time` into these options.
2. Loads and validates the active module and manifest pair.
3. Initializes the process image, clock, GOT cells, and root allocator.
4. Attaches the module and runs its preparation (cold start or online attach).
5. Repeats until the cycle limit is reached or a stop signal is received:
   - consume a pending mock-clock override, or read the live clock, and latch `cycle_now_ns`,
   - set each task's guard from the host scheduler (§1.3),
   - apply queued debug writes/forces at the boundary (implemented in M6.5),
   - check for a prepared candidate and switch generations if ready,
   - call `stc_module_cycle`,
   - write outputs from the output image (phase 5),
   - sleep until the next cycle deadline if a cycle time is configured; a mock-clock run
     advances the mock time by the cycle time instead of sleeping,
   - mark due `SINGLE` tasks complete and advance interval deadlines after a successful scan,
   - check the watchdog and service queues.

   The launcher cold-installs the adjacent pair and runs the host scheduler and cycle loop.
   Online prepare/activation is driven through the local deploy/status control channel
   (`stc run --watch`), the Rust `Host` API, and the embedding host. Validation, load, and
   prepare run between cycles on the cycle thread; fully off-thread prepare remains M7.
6. Runs shutdown, saves retained state (phase 5), and unloads generations.
7. Returns exit code `0` on normal completion, `1` for a load or cycle failure, `2` for an
   invalid runtime option, `3` for an internal panic, `4` for a trap, and `130` when
   interrupted by Ctrl-C (Ctrl-Break on Windows). The raw `stc_rt_trap` service keeps exit code
   70 when it fires outside a host boundary.

The launcher defaults to running until interrupted when `--cycles` is omitted or `0`. Stop
handlers are installed at loop start; the handler sets an atomic flag and the loop shuts down
normally before returning 130.

### 1.2 Embedding library mode

`stc build --emit=lib` emits an embedding host library with no `main`. It exports the host API,
and `{stc:export}` wrappers are resolved through the host export registry because the generated
code stays in the separate module:

```c
uint32_t stc_abi_version(void);
const char* stc_metadata_json(void);            /* metadata sidecar, after a load */
int32_t stc_host_load_module(const char* path, stc_load_mode mode);
int32_t stc_host_cycle(void);
int32_t stc_host_shutdown(void);
void* stc_host_symbol(const char* name);        /* exported module symbol registry */
int32_t stc_host_export_manifest(char* buffer, int64_t* len);
int32_t stc_host_run(const stc_host_options* options);

typedef struct {
    uint32_t struct_size;       /* sizeof(stc_host_options), or 0 */
    uint32_t abi_version;       /* host ABI major (2), or 0 */
    uint64_t cycles;            /* 0 = until interrupted or *stop */
    int64_t  cycle_time_ns;     /* negative = no fixed cycle time */
    int32_t  io_backend;        /* -1 = metadata, 0 null, 1 memory, 2 file */
    int32_t  reset_retain;
    int32_t  no_retain;
    int32_t  status;            /* print the status summary at exit */
    int64_t  mock_clock_ns;     /* negative = live clock */
    const char* module_path;    /* null = run the already-loaded module */
    const char* io_file;
    const char* state_file;
    const char* control_endpoint; /* local named pipe/socket, or null */
    const int32_t* stop;        /* nonzero stops before the next scan */
    const char* control_nonce;    /* per-run nonce when control_endpoint is set */
    /* Implemented in M6.5 (ABI 2.3), appended; ignored by a shorter struct */
    const char* debug_endpoint;   /* local debug endpoint file, or null */
    int32_t  debug_allow_write;   /* grant the debug write scope */
    int32_t  debug_allow_force;   /* grant the debug force scope */
    int64_t  debug_session_timeout_ns; /* 0 = disabled */
} stc_host_options;
```

`stc_host_run` loads `module_path` cold when it is non-null and otherwise runs the module
already loaded through `stc_host_load_module`. It shares one loop implementation with the
launcher: the same latched clock, scheduler, cycle-time deadline, mock clock, status output, and
0/1/2/3/4/130 exit codes. A non-empty control endpoint requires the matching per-run nonce and
serves the local deploy/status channel; the I/O and retained-state paths configure the host
before a cold load and apply to the final shutdown.

The four debug fields are appended and `struct_size`-gated; an older embedding host that passes a
shorter struct gets no debug server. A non-null `debug_endpoint` starts the local debug service
and writes the endpoint file there; it is a separate endpoint, token, and protocol from
`control_endpoint` (D-155, D-173). `debug_allow_write`/`debug_allow_force` grant the corresponding
scopes; `debug_session_timeout_ns` of 0 disables the pause/idle bound, and a positive value sets
it (D-212). An embedding host that does not set `debug_endpoint` never starts the service.

Embedding hosts configure the process-image backend and retained state before loading a module
with `stc_host_configure_io(backend, file)` (`-1` keeps the metadata backend, `0`/`1`/`2` select
`null`/`memory`/`file`) and `stc_host_configure_retain(path, policy, reset)` (a null path
disables retention). A `stc_host_run` options struct may configure the same values for the
module it loads; an already-loaded module keeps the host's current configuration.

The generated module remains a separate loadable artifact; M6 emits the embedding host as a
static library or, with `lib-type = "shared"`, a shared host library. `stc_host_load_module`
takes mode `0` (cold), `1` (online prepare), or `2` (auto).
`stc_module_cycle` is called only by the host's cycle thread; external callers must serialize
all host API calls. The embedding host owns the loop, I/O, clock, GOT, root allocator, manifest
store, and control; M6 runs candidate validation/load/prepare between cycles on the cycle thread,
and M6.5 provides the service/monitor thread class. The runtime is not a general-purpose
thread-safe library: service access goes through the typed generation-pinned API, and the cycle
thread is the only ST executor.

### 1.3 Task model

- The configuration declares resources, tasks, and program instances. The compiler projects
  every task with its `interval_ns`, `priority`, `SINGLE` flag, and guard identity into the
  existing `TaskRecord`/instance records; no manifest format change is needed.
- `stc_module_cycle` executes the selected configuration's programs according to the task
  schedule. The generated cycle checks each task-bound instance's `TaskGuard` root and skips the
  instance when the guard is clear; it does not initialize or clear guards. Task-less instances
  are unguarded and run every scan.
- **Scheduler order.** The compiler emits task-bound instances in scheduler rank order: tasks
  with an explicit priority first, ascending; tasks without a priority after them; ties by
  resource declaration order, then task declaration order; within a task, program declaration
  order. Task-less instances run first, in declaration order. The host does not reorder
  instances; it only decides which guards are set.
- **Host state.** The host owns task run state keyed by task `GotId`: `SINGLE` completion and the
  next interval deadline. The launcher/host loop latches the scan time at the boundary and the
  scheduler reads it as `now = cycle_now_ns` (the same value the module sees); the scheduler then
  computes each task's due state and writes the task's `TaskGuard` BOOL root. The guard root is
  reached through the `SlotKind::Data` slot whose `got_id` equals the manifest task's
  `guard_got_id`; the cell value is the root's storage address, so the write needs no GOT or
  manifest format change. Every
  scheduled task is projected with a guard; a manifest task without one is `STC-E-RUN-003`.
- A task without an interval is due every scan. An interval task is due when
  `now >= next_deadline_ns`; its first deadline is the first scan's latched time plus the
  interval, and after a due scan the next deadline is `now + interval` (no catch-up). A `SINGLE`
  task runs on its first due scan and is marked complete after that cycle returns normally; it
  does not run again. All due tasks share one scan, one latched clock, one input latch, and one
  output publish.
- **Online change and cold download.** Task state survives a compatible swap by `GotId`. A task
  record whose resource, name, `SINGLE`, interval, or priority changed is a topology change and
  is rejected by the existing migration check (`STC-E-ONLINE-004`), so a cold download is
  required. A cold install or cold download starts with no completion and deadlines computed
  from the first scan's time. Task state is host memory, not a manifest field or state-file
  content; guard roots are `TaskGuard` roots and are never retained.
- The M4.5 one-task-per-resource restriction and its `--allow-multi-task` testing escape are
  historical; M6 phase 4 implements the scheduler above. Codegen keeps `STC-E-RUN-001/002` for
  genuinely ambiguous or unsupported configurations, and the runtime uses `STC-E-RUN-003` for
  invalid run options and runtime scheduling configuration. The HIR keeps the full task/program
  mapping, so the schedule is checked in codegen and executed by the host.
- **Status surface.** `Host::status()` reports the active, pending, and retired build IDs, the
  active cycle count, the last rejection, activation warnings, and per-task
  `due`/`completed`/`next_deadline_ns` state. The launcher prints it under `--status` (used by
  `stc run -v`), and the M6 control channel consumes the same snapshot.

## 2. Runtime library

### 2.1 Implementation

- `crates/stc-runtime` is a Rust crate built as `staticlib` (the embedding/host artifact) with
  an `rlib` target for Rust integration tests, and as a shared host library when
  `lib-type = "shared"` is selected. It is the persistent host as well as the embedding library.
- It exposes a C ABI (`extern "C"`, `#[no_mangle]`, `#[repr(C)]`) through a versioned host API
  table, not a flat set of mandatory exported symbols.
- No Rust types cross the ABI except `u8`/`i32`/`i64`/`f32`/`f64`/pointers/opaque handles.
- The runtime is built with the toolchain that matches the target C ABI: MSVC for
  `*-pc-windows-msvc`, GNU for `*-unknown-linux-gnu`. Windows x86-64 is the primary development
  host; Linux x86-64 and Linux AArch64 are supported native hosts from M8 (D-042, D-200).
- Panics are caught at the ABI boundary and converted to `stc_rt_trap` where possible; the
  compiler-generated code does not rely on unwinding.
- The host owns the GOT cell table, root allocator, module loader, manifest store, swap
  controller, process image, retained state, and the M6 control/status thread; M6.5 provides the
  service/monitor thread class. Module code owns only code and version-local read-only data.
- Host state is protected by a control lock used only by host control paths, never held across
  ST execution. Service readers use generation pinning/epoch refcounts.

### 2.2 ABI versioning

- `STC_HOST_ABI_MAJOR = 2`; the v1 persistent host implements minor **3** (2.3).
  Minor 1 appends no service and changes no signature; it records the inline TYPEINFO, VTABLE,
  and object-record layout of §2.6 that generated OOP code reads without a host call (D-151). M6
  phase 6 implemented minor 2; M6.5 appended minor 3 for the debug checkpoint service.
- Minor **2.2** appends exactly one service, ID 12, `void (*set_cycle_time_ns)(int64_t ns)`,
  after `assert` in `stc_host_api`. It sets the next cycle's latched time and does not change the
  current scan; `STC_CLOCK_ADVANCE` lowers to `set(cycle_now_ns() + duration)`. Only test builds
  import it; production modules keep required minor 2.1 (D-157). The table and service IDs are
  fixed in [GOT Artifacts and Schemas](GOT-Artifacts.md) §8.
- Minor **2.3** (implemented in M6.5) appends exactly one service, ID 13,
  `void (*debug_check)(uint32_t checkpoint_id, uint32_t span_id)`, after service 12. Only debug
  builds that emit checkpoint hooks import it; they require minor 2.3. Test builds remain at 2.2;
  hook-free debug and release builds remain at 2.1 (D-177). The checkpoint table itself is in the
  metadata sidecar, not the host API.
- The runtime exposes `stc_abi_version()` and a host API struct with `abi_version` and
  `struct_size`. A module rejects a host whose struct is shorter than required.
- Generated modules embed the expected ABI version in metadata and their descriptor; the loader
  rejects mismatched modules before attach.
- Additive changes bump the minor version; breaking signature/struct changes bump the major
  version. M4 ABI 1 artifacts are not loadable.

### 2.3 Core ABI

```c
typedef void (*stc_fn)(void);

/* Launcher/embedding-host exports. These are host symbols, not module imports. */
int32_t  stc_rt_execute(int32_t argc, char** argv, const char* module_path);
uint32_t stc_abi_version(void);

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

/* Host-only configuration; generated modules call the host API table instead. */
void stc_rt_set_cycle_time_ns(int64_t ns);
void stc_rt_set_io_backend(const stc_io_backend* backend);
void stc_rt_state_configure(const char* path, int32_t policy, int32_t enabled);
```

Generated modules do not import `stc_rt_*` symbols. They call traps, clocks, state, I/O, and
allocation through the host API table (ABI 2.2 implemented; production modules require 2.1, test
modules 2.2, and debug-hook modules 2.3) defined below and in
[GOT Artifacts and Schemas](GOT-Artifacts.md) §8. The flat `stc_rt_*` module ABI is M4 ABI 1 and
is not loadable by the persistent host. `stc_rt_set_cycle_time_ns` is a host-only export for the
launcher and embedding hosts; generated test code advances the next cycle's time through service
ID 12 instead. `stc_rt_test_main` is the test-host entry point linked by a generated test
launcher; production modules never call it.

#### 2.3.1 Host API table

The exact ABI is a versioned `stc_host_api` struct, fixed for ABI 2.1 in
[GOT Artifacts and Schemas](GOT-Artifacts.md) §8, with the 2.2 append recorded there. The table
is additive within major version 2: new services are appended and bump the minor version; a
module rejects a host whose table is shorter than it requires. Modules receive the table in
`stc_module_attach` and do not import platform symbols from the host executable. Minor 1 appends
no service and adds no field: it records the inline OOP record layout of §2.6, so the minor gate
rejects a 2.1 module on a 2.0 host before generated code can read a record. Minor 2 appends only
the test-clock service (ID 12); a 2.2 test module is rejected by a 2.1 host, while a production
2.1 module still runs on either. Minor 3 (implemented in M6.5) appends only the debug checkpoint
service (ID 13); only hook-emitting debug builds import it and require 2.3. Reference resolution,
typed variable access, string/math helpers, and generation pin/unpin are host-side capabilities
implemented in M6.5 and need no new module-facing service; the debug protocol and typed access do
not use the host API table.
M4.5-generated code imports trap, I/O image/size, and the retained-state services. Clocks,
transient alloc/free, and assert are in ABI 2.0 but unused by M4.5-generated code; M5/M6
consumers add the corresponding import slots when they first emit those calls.

The trap entry is non-returning by contract. The host calls prepare and cycle from a platform
trap boundary and converts a module trap into a failed generation (or exit code 70 when there is
no rollback target). Generated code never resumes after a trap.

#### 2.3.2 Module descriptor and GOT context

The descriptor is a pointer-free, versioned byte blob fixed in
[GOT Artifacts and Schemas](GOT-Artifacts.md) §7. It records the module format, host ABI, profile,
resolved target, build ID, paired manifest digest, function table, and slot table. It contains no
target addresses; `stc_module_functions` returns the code-address array separately, and the host
copies everything it needs before the module can be unloaded.

`stc_module_attach` receives the host API, a generation-specific array of pointers to host cells,
and the generation ID. The module caches the slot array for its generation; generated code reads
a cell value through the slot and never writes a cell. Storage pointers are loaded from data
cells, code addresses from function cells, and process-image addresses from direct cells.

#### 2.3.3 Generation pinning and services

Service/monitor threads submit typed reads and watches; the cycle thread applies them at the next
boundary in `Host::debug_pump_at`, pinning the active generation. A pin is an epoch/refcount that
delays unload until the copy is complete. The service thread never dereferences host, module,
root, or process-image storage, never calls `stc_module_cycle`, never writes GOT cells, and never
takes a lock held by the cycle thread. Debug writes and forcing are queued for cycle-boundary
application; `Host::debug_read` performs the same pinned copy synchronously on the calling thread
for embedding hosts.

The cycle thread runs at the process default operating-system priority. Control-server accept and
connection threads and debug-server accept and handler threads run one class lower: Windows
`THREAD_PRIORITY_BELOW_NORMAL` and Linux `setpriority(PRIO_PROCESS, 0, 10)` (nice +10) for the
calling thread. The Linux call is best-effort: a container or sandbox may reject it, and the
runtime never fails a session, swap, or shutdown because priority could not be lowered. v1 does
not pin CPU affinity; placement belongs to the deployment's `cpuset` policy, so the host keeps
its inherited processor mask. The policy is an advisory mitigation for maintenance jitter, not a
real-time guarantee (D-204).

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

- The host reads the time source **once at `stc_module_cycle` entry** and stores `cycle_now_ns`
  for the whole scan. Every ST-visible time function and standard FB uses this latched value.
  Two timers evaluated in the same scan cannot observe different times (D-027).
- The `cycle_now_ns` host-API service returns the latched time as signed 64-bit nanoseconds.
  Generated ST code reads it through its import slot.
- The `monotonic_now_ns` host-API service returns the live monotonic clock and is for the
  runtime/host only. Generated ST code does not call it. The runtime reads it through a host
  abstraction (`QueryPerformanceCounter` on Windows, `clock_gettime(CLOCK_MONOTONIC)` on Linux)
  and converts to nanoseconds; the ST-visible semantics are identical on every host (D-042).
- The clock remains continuous across an online change. The host latches the next cycle's time
  normally; the swap does not reset or re-latch it, so timers observe elapsed wall/monotonic time.
- When a watchdog exists, it is paused or extended with a documented bound during
  prepare/activation and while a debugger has the cycle stopped. The debug stop/idle bound is
  `[debug].session-timeout`, optional and disabled by default (D-212); when configured and
  exceeded, the session ends with `STC-E-DEBUG-018`, forces and breakpoints are cleared, and the
  cycle resumes with a fresh deadline ([Debug and Monitoring](Debug-and-Monitoring.md) §14.3). If
  a bound is exceeded, the normal watchdog policy resumes. M6 has no watchdog implementation;
  M6.5 phase 6 implemented the host-side stop budget, and a future watchdog consults it (D-175).
- In executable mode, the runtime latches the host monotonic clock at the start of each cycle.
  `--cycle-time` controls the loop deadline but does not change the value latched for the scan.
- In library mode, the host owns the loop. By default `stc_module_cycle` latches the live
  monotonic clock; the host can supply the next cycle's time before calling `stc_module_cycle`.
  The override is consumed by the next cycle.
- Tests use a mock clock with the same semantics: the harness sets the next cycle time, calls
  `stc_module_cycle`, and advances the mock clock between cycles. The test prelude's
  `STC_CLOCK_ADVANCE(T#10ms)` lowers to host ABI 2.2 service ID 12
  (`set_cycle_time_ns(int64_t)`), which schedules the next cycle's latched time and does not
  change the time already latched in the current scan (D-157).
- `TIME`/`LTIME` are signed 64-bit nanoseconds. `TON`/`TOF`/`TP` compute elapsed time from
  `cycle_now_ns` and their captured start time. Arithmetic overflow traps.
- Date/time functions use `cycle_now_ns` converted to the profile's epoch. Within a scan,
  `NOW()`/`TIME()` are constant.
- Time-zone handling is profile-defined; v1 uses local time only and records the choice.

### 2.6 Objects, descriptors, and dispatch

The byte layouts that generated OOP code reads inline are fixed in
[GOT Artifacts and Schemas](GOT-Artifacts.md) §8.5. This section fixes the host behavior around
them.

**Ownership and lifetime.** TYPEINFO and VTABLE records are host-owned and allocated once per
type when a generation is prepared; they keep stable addresses across compatible swaps so
preserved class handles and interface values stay valid. An object record is host-owned storage
too, but per declaration rather than per type: a persistent object's record lives in its hidden
`OBJREC` root, and a local object's record is a per-activation stack alloca. A class handle lives
in its hidden `HANDLE` root (persistent) or in a stack slot (local). The host allocates hidden
roots like any data root and zeroes fresh storage; it applies no generic initializer to them.
Generated code never allocates, frees, or writes a TYPEINFO, VTABLE, or record header.

**Prepare and activation.** Prepare builds the shadow descriptor registry from the manifest's
type and function records, binds the generation's descriptor, function, root, direct, and import
cells, and then calls `stc_module_prepare`. Generated prepare walks the fresh-root mask: for each
fresh object record it stores the type's TYPEINFO pointer and the resolved storage addresses of
the record's leaves, and for each fresh class handle it stores the owned instance's record
address. Preserved records and handles are not rewritten. `stc_module_cycle` cannot run until
the host activates the generation, at which point it patches descriptor entries and interface
maps under the cycle boundary. A failed prepare or first cycle releases fresh records and
handles with the candidate's roots; a preserved handle to a removed instance is quarantined, not
freed. Phase 4b.2 implements the registry: descriptor cells are bound to the host-owned records
before `stc_module_prepare`, compatible shapes keep their addresses and shadow contents are
patched at activation, shape changes and removals reject while a preserved object or interface
value can depend on them, and removed descriptors quarantine with their roots. Phase 4b.6
verifies the lifecycle end to end with compiler-produced modules: a preserved interface value
and class handle keep their record, handle, TYPEINFO, VTABLE, and per-pair vtable addresses
across a warm swap, activation patches the preserved pair to the new method cells, fresh object
records and handles initialize once at prepare, and rollback releases them while preserved
records retain their address and state.

**Class values and FB values.** A class value is a non-retained handle to an object record; the
declaration owns an embedded instance whose field roots keep their retention policy, and a
reassigned handle only orphans the instance (which stays rooted for a cold start to re-point).
An FB value is an inline instance: its fields are its own roots and `:=` copies them. A
persistent record comes from its hidden `OBJREC` root and a persistent class handle from its
hidden `HANDLE` root; a local class owns a stack instance, a stack handle slot, and a stack
record, and a local FB owns a stack instance and stack record. Class field and array-element
access loads the handle and then uses the referenced record and root table; FB fields stay
inline. Method and FB bodies receive `(handle, root table, ...)`. For an object body the root
table is the record's tail (`record + 8`); `THIS` is the handle, `SUPER.method()` calls the base
implementation directly with the same handle, and `SUPER()` invokes the base FB body on the same
record. Class `IN` parameters are handles, so assigning the parameter only rebinds the callee's
copy; FB `IN` parameters are copied by the caller for IN isolation. `OUT` and `VAR_IN_OUT`
object parameters receive a pointer to the caller's handle slot (class) or to the instance
record/state (FB). Generated code performs no heap allocation.

**Dispatch.** Every class and FB method call is virtual: load the record's TYPEINFO, load the
class VTABLE through it, index the method's slot, load the host function cell, and call the cell
value. Interface calls index the interface value's per-pair VTABLE in the same way. A class/FB
value converts to an interface at a compile-time index into the source type's base-first
interface map; the inherited-index rule keeps that index valid for a derived runtime object, and
the entry's table reflects the object's own overrides. Interface-to-interface upcasts and dynamic
`?=` checks search the map by interface `GotId`. `?=` never traps: a null or incompatible source
yields the all-zero `NULL` value, successful class attempts copy the handle, successful interface
attempts copy the pair, and no attempt calls a host service. `= NULL`/`<> NULL` compare the
handle or the interface's record pointer; identity comparison is not part of M5.

**Traps and escape.** A null handle, object record, TYPEINFO, VTABLE, or interface pointer that a
load or dispatch depends on traps `STC-TRAP_NULL_REF`. `REF(object)` produces an `Object` place
descriptor (`{handle, descriptor}`) whose descriptor is the object's TYPEINFO pointer;
dereference, assignment, and object-reference `?=` use the record and TYPEINFO, never a raw
storage address, and `REF_TO`-to-interface yields null for null or incompatible input. A handle
or interface that refers to a stack record must not outlive its activation: returning one,
storing one persistently, or writing one through an `OUT`/`VAR_IN_OUT` parameter is rejected by
the analyzer, exactly like the pointer-escape rule. Recursion is allowed because each activation
has its own stack records.

**Online change.** Descriptor shape (kind, owner, type key, layout hash, size, align, vtable
shape) is part of compatibility. A shape change rejects while a live object or interface value
depends on it, and removing a type that a live handle or interface depends on rejects the swap,
so dispatch can never reach retired code. Removed descriptors retire and quarantine with their
roots. Hidden `HANDLE`/`OBJREC` roots are non-retained and are excluded from retained-state
leaves and metadata payloads; the owned instance's field roots are their own retained leaves
(see [State and RETAIN](State-and-Retain.md) §2.3).

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

The standard FB bodies are synthesized as production HIR in the reserved `STC_STD` scope and
compiled into each module; there is no textual `stdlib/` package (D-134):

`SR`, `RS`, `R_TRIG`, `F_TRIG`, `CTU`, `CTD`, `CTUD`, `TP`, `TON`, `TOF`.

- The standard FB types are injected as a prelude; their definitions live in `STC_STD` and are
  compiled into each module with stable GOT keys. The host does not require a separately loaded
  stdlib module in v1.
- User code can use the standard names directly; the analyzer resolves them after local/global
  scopes, so a user-defined `TON` shadows the stdlib (with a warning).
- The synthesized bodies depend only on the latched `cycle_now_ns` service and exact arithmetic.
  They must not read the live monotonic clock.
- All timers in one scan use the same latched cycle time, so two `TON`/`TOF`/`TP` instances
  evaluated in the same cycle cannot produce different elapsed times (D-027).
- Timer behavior is defined by the synthesized bodies and tested with a mock clock and
  deterministic step sequences (D-134/D-140).

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
- Span IDs are assigned by sorting the distinct non-empty spans by `(FileId, start, end)`; the
  table is deduplicated, so identical spans share one ID and repeated builds assign identical
  IDs.

## 7. Test execution

- `stc test` compiles the project plus test sources into one test artifact per test and links a
  test host. Each artifact is a cold-layout ABI 2 module/manifest pair with a synthetic
  single-task configuration; test builds never use the project baseline or online change.
- Each test runs in a separate process with a timeout (default 5 s) to isolate hangs and traps.
- The runner provides:
  - a fresh host/module state per test,
  - a mock clock starting at `[test].mock-clock` (default `0ns`),
  - a memory I/O backend unless `[test].io` overrides it,
  - assertion helpers that record failures without aborting immediately,
  - no retained-state load or save.
- Test functions return `BOOL` (TRUE = pass) and may call `STC_ASSERT`; assertions use the
  existing ABI 2 `assert` service with a recording sink (the host records span IDs and the
  driver's sidecar supplies messages/locations), and `STC_CLOCK_ADVANCE` uses the appended ABI
  2.2 service ID 12 as `set(cycle_now_ns() + duration)`. `STC_IO_SET_INPUT(offset, value)` and
  `STC_IO_GET_OUTPUT(offset)` use the existing I/O services. An FB test's optional `PASSED`
  output is read by the host through its GOT leaf root rather than source-level access.
- Program/FB tests may run a configured number of cycles (`cycles = N` in the test metadata) and
  inspect outputs/globals.
- Results are reported as human-readable text or the JSON shape in
  [CLI and Manifest](CLI-and-Manifest.md) §1.6; exit code 0 if all tests pass.

Detailed test discovery, pragma syntax, and the per-test artifact interface are in
[Testing](Testing-and-Verification.md) §2.

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
| Debug session stop / forced value | The opt-in stop/idle bound (`[debug].session-timeout`, disabled by default) ends the session with `STC-E-DEBUG-018` when configured, clears state, and resumes with a fresh deadline; forced values are held until release/session end/swap; not a safety function |
| Debug read/write/force for a removed field or stale generation | Reads pin the generation; a removed/stale/layout-changed target is `STC-E-DEBUG-010` and is never retargeted; queued writes/forces are rejected at the boundary; every swap ends the session (`STC-E-DEBUG-019`) |
| Debug protocol/authentication failure | Reject or close the connection; no cycle effect; audit the failure; `STC-E-DEBUG-*` |
| Debug metadata/checkpoint mismatch | Refuse breakpoints/stepping; the module keeps running; `STC-E-DEBUG-016` |

## 9. Open runtime questions

None as of 2026-10-28. The online-change host/GOT/manifest/debug architecture is resolved in
[Online Change and GOT](Online-Change-and-GOT.md),
[Debug and Monitoring](Debug-and-Monitoring.md), and [Decisions](../Decisions.md); the M6.5
protocol, session, access, checkpoint, and build contracts are recorded as D-173–D-183. New
runtime questions are appended here or in [Decisions](../Decisions.md).
