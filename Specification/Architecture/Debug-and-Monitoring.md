# Debug and Monitoring

Status: Resolved specification (2026-10-24)  
Profile: `stc-2013-oop-utf8-v1`  
Scope: v1 runtime/platform capability; not an IEC 61131-3 language feature.

This document specifies typed variable access, monitoring, forcing, breakpoints, stepping, call
stacks, debug information, the local debug protocol, and the interaction between debug sessions
and online change. It complements
[Online Change and GOT](Online-Change-and-GOT.md) and
[Runtime and Execution](Runtime-and-Execution.md). The full debugger is the M6.5 milestone; the
resolved rationale is in `update_plan_got.md`.

## 1. Scope and safety stance

V1 provides **full typed debug and monitoring**. The project is **not a safety PLC and is not
responsible for safety in any way**. Debug sessions may:

- pause or disable the watchdog;
- stop the cycle;
- hold outputs at their last or forced values indefinitely;
- change persistent variables and force I/O.

No automatic fail-safe, output timeout, or safety interlock is provided. A deployment that needs
those properties must implement them independently and must not rely on `stc`.

## 2. Thread and session model

The runtime has three thread classes:

| Thread | Role in debug |
| --- | --- |
| Cycle | Executes ST, applies queued writes/forces at boundaries, and services breakpoint checkpoints |
| Loader/prepare | Loads/prepares a candidate module during online change |
| Service/monitor | Serves typed reads, watch streams, debug control, status, and manifest export |

A debug session is a service-side session with an authenticated connection. Normal monitoring
does not stop the cycle. Breakpoint stops and stepping suspend the cycle thread at defined
checkpoints. A stop does not run ST on the service thread and does not move persistent storage.

## 3. Typed variable access

### 3.1 Addressing

Variables are addressed by typed path or `GotId`, never by a raw address supplied by the client.
A path resolves through the active manifest's instance/field records to a root or direct-variable
cell; a `GotId` addresses the same identity directly. Local and parameter access additionally
requires a stopped frame or a pinned generation.

### 3.2 Reads

A read is performed against a generation-pinned view:

- the service thread pins the active generation with an epoch/refcount;
- the host resolves the descriptor and copies the typed value;
- the generation cannot unload until the pin is released;
- no cycle lock is taken.

A read may observe a value from a single cycle or a torn aggregate only if the type is explicitly
declared as non-atomic; scalar roots and the documented atomic roots (`OVERLAP`) are copied as a
unit. The debug API reports the generation/build ID and cycle counter associated with a read.

### 3.3 Writes

Writes are queued and applied at a cycle boundary, in the same phase as forced values. A write:

- targets a field/root by `GotId` or typed path;
- is type-checked and range-checked against the active manifest;
- is rejected if the target was removed or its layout changed;
- is not applied while the cycle thread is executing ST.

A write queued across an online swap is rejected or retried against the new generation; it never
targets a removed field. Debug writes are not persisted to the retained-state file unless the
target is a `RETAIN` root and the normal state-save policy runs.

### 3.4 Watch and polling

A watch is a repeated typed read of a set of paths/`GotId`s. The service thread may coalesce
watches and rate-limit them. Watch data is tagged with generation and cycle number. Watches never
block the cycle and are dropped or coalesced under pressure.

## 4. Forcing

Forcing overrides a variable's value at the cycle boundary until explicitly released. Rules:

- A force targets a typed path/`GotId` and carries a typed value.
- The host applies forces after the cycle's normal writes and before outputs are published.
- A forced value persists across cycles until released, session end, or an online swap.
- Forcing an input affects the value seen by the next cycle; forcing an output holds the published
  value, subject to the I/O backend.
- Forces are not persisted to the retained-state file.
- Every force is recorded in the debug audit log with time, session, target, value, and release.

## 5. Breakpoints, stepping, and call stacks

### 5.1 Breakpoints

Breakpoints are source-level locations resolved to compiler-emitted checkpoints. Codegen emits a
checkpoint table and a call to a host check at each eligible statement/block boundary. The host
does not patch machine code and does not require writable text pages. A breakpoint:

- is set/cleared by source location or checkpoint ID;
- is matched against the active generation's table;
- stops the cycle thread before the checkpoint's statement executes;
- reports the generation, frame, source location, and cycle number.

If a source location has no checkpoint in the active generation, the breakpoint is reported as
unbound rather than silently moved to an unrelated statement.

### 5.2 Stepping

Stepping is stop-the-world: the cycle thread is suspended at a checkpoint. Step-over, step-into,
step-out, and continue are supported. A step that would cross an online swap ends the session
instead of continuing into a different generation. The watchdog is paused or extended while the
cycle is stopped, with a documented bound; if the bound is exceeded the session is terminated and
the normal watchdog policy resumes.

### 5.3 Call stack

The runtime unwinds the stopped call stack using debug-frame information and returns frames with
function identity, source location, and typed locals/parameters where available. In optimized
builds, frames or variables may be unavailable; the report marks them as optimized out rather
than inventing values. `SUPER`/interface dispatch reports the dynamic function identity.

## 6. Debug information

### 6.1 Default and release

Debug information is the default build output. `--release` selects the optional stripped and
optimised mode. Debug and release builds have distinct build IDs and manifests. Debug hooks and
checkpoint calls are absent from release artifacts.

- Debug: full DWARF/CodeView, checkpoint table, frame information, and typed location metadata.
- Release: optimised code, no debug hooks, optionally a sidecar symbol file; the sidecar does not
  change the module's runtime behaviour.

Debug-info emission is derived from the clang target triple and verified by toolchain tests.
Reproducible builds use deterministic paths, stable type/field ordering, and no timestamps.
Stripping rules are explicit: the manifest and build ID remain; symbol/debug sections are removed
or moved to the sidecar.

### 6.2 Split-object view

The debugger may present an object/aggregate as split fields by combining DWARF/CodeView with the
manifest's per-field layout records. This view shows the runtime's actual per-field roots and
handles; it does not claim that the object is contiguous memory. The debugger must label the view
as manifest-assisted so a user does not mistake it for a raw struct dump.

### 6.3 Active generation and swap

The host retains debug information for the active generation and any pinned retired generation.
Every online swap ends the debug session: breakpoints and forces are cleared, the cycle resumes,
and the client must reconnect. The old generation's debug information is dropped after unload; a
new session resolves against the new manifest.

## 7. Protocol and security

The v1 debug protocol is local (host API or local socket/pipe) with:

- authentication and authorization for debug and force operations;
- rate limits and request timeouts;
- an audit log for connect/disconnect, reads of protected data, writes, forces, breakpoint
  changes, stops, and session termination;
- no raw-memory read/write in the standard API; a privileged raw-memory mode, if ever added, is a
  separate product decision;
- access control for manifest/variable downloads because layout data reveals program structure.

Production deployments may disable debug entirely with a build/runtime policy. When enabled, the
documented behavior above applies; there is no hidden safety guarantee.

## 8. CLI and diagnostics

The CLI provides `stc debug` (start a session) and debug-related build options. Diagnostics use a
stable `STC-E-DEBUG-*` / `STC-W-DEBUG-*` family for protocol, session, breakpoint, force, and
debug-info errors. Online-change diagnostics use their own `MODULE`/`MANIFEST`/`ONLINE` families.

## 9. Verification

- Typed read/write/force/watch tests for scalars, aggregates, strings, interfaces, and direct
  variables, including removed-field and layout-change rejection.
- Breakpoint, step, call-stack, and optimized-out reporting tests.
- DWARF/CodeView inspection with platform tools on Windows and Linux.
- Session teardown, force clearing, watchdog resume, and swap-end tests.
- Debug/release build-ID, manifest, stripping, and hook-absence tests.
- Concurrency tests for generation pinning, service contention, and bounded queues.

## 10. Non-goals

- Safety functions, safety certification, automatic fail-safe outputs, or a safety watchdog.
- Remote/network debugging in v1.
- Raw memory editing in the standard API.
- Persistent breakpoints/forces across online swaps or restarts.
- Reconstructing missing source or debug information from a manifest.

## 11. Related documents

- [Online Change and GOT](Online-Change-and-GOT.md) — generations, swaps, manifest identity.
- [Runtime and Execution](Runtime-and-Execution.md) — cycle loop, services, traps.
- [State and RETAIN](State-and-Retain.md) — state-file interaction.
- [Testing and Verification](Testing-and-Verification.md) — debug test levels.
- [CLI and Manifest](CLI-and-Manifest.md) — debug CLI and `[debug]` table.
