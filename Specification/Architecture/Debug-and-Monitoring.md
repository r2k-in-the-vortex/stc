# Debug and Monitoring

Status: Resolved specification (2026-10-24); M4.5 interface recorded; M6 complete with the M6.5 boundary at host ABI 2.2 service ID 12 (2026-10-27); M6.5 phase 0 fixes the protocol, session, access, checkpoint, and build contracts (2026-10-27); M6.5 implementation complete (2026-10-28)  
Profile: `stc-2013-oop-utf8-v1`  
Scope: v1 runtime/platform capability; not an IEC 61131-3 language feature.

This document specifies typed variable access, monitoring, forcing, breakpoints, stepping, call
stacks, debug information, the local debug protocol, and the interaction between debug sessions
and online change. It complements
[Online Change and GOT](Online-Change-and-GOT.md) and
[Runtime and Execution](Runtime-and-Execution.md). The full debugger is implemented in M6.5; the
phase decisions are D-173–D-183.

## 1. Scope and safety stance

V1 provides **full typed debug and monitoring**. `stc` can control real machinery, but the
project is **not a safety PLC and is not responsible for safety in any way**; safety must be
provided by a separate safety system and machine design as required by law and regulation. Debug
sessions may:

- stop the cycle (a future watchdog deadline is paused or extended while stopped);
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
| Service/monitor | Queues typed reads and watches and serves debug control, status, and manifest export; the cycle thread applies reads and watch batches at a boundary |

A debug session is a service-side session with an authenticated connection. Normal monitoring
does not stop the cycle. Breakpoint stops and stepping suspend the cycle thread at defined
checkpoints. A stop does not run ST on the service thread and does not move persistent storage.

## 3. Typed variable access

### 3.1 Addressing

Variables are addressed by typed path or `GotId`, never by a raw address supplied by the client.
A path resolves through the active manifest's instance/field records to a root or direct-variable
cell; a `GotId` addresses the same identity directly. Local and parameter access additionally
requires a stopped frame; a pinned generation keeps the manifest and storage alive but does not
grant access to a running frame's locals.

### 3.2 Reads

A read is performed against a generation-pinned view:

- the service thread submits the read; the cycle thread applies it at the next boundary, pinning
  the active generation with an epoch/refcount;
- the cycle thread resolves the descriptor and copies the typed value (see §15.2);
- the generation cannot unload until the pin is released;
- no cycle lock is taken and the service thread never dereferences host storage.

A read may observe a value from a single cycle or a torn aggregate only if the type is explicitly
declared as non-atomic; scalar roots and the documented atomic roots (`OVERLAP`) are copied as a
unit. The debug API reports the generation/build ID and cycle counter associated with a read.

### 3.3 Writes

Writes are queued and applied at a cycle boundary, in the same phase as forced values. A write:

- targets a field/root by `GotId` or typed path;
- is type-checked and range-checked against the active manifest;
- is rejected if the target was removed or its layout changed;
- is not applied while the cycle thread is executing ST.

A write queued across an online swap never migrates: the swap ends the session and the queued
write is discarded, and a client that reconnects re-resolves and re-issues it against the new
generation. A write is rejected at the boundary if its target was removed or its layout changed
(`STC-E-DEBUG-010`) and is never retargeted. Debug writes are not persisted to the retained-state
file unless the target is a `RETAIN` root and the normal state-save policy runs.

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
- Forces are not persisted as a force table. A force held on a `RETAIN` root can still affect the
  value saved by the normal shutdown state policy; that interaction is reported as
  `STC-W-DEBUG-005` when the force is created. The force itself is always cleared at session end
  or swap and is never restored by the state file.
- Every force is recorded in the debug audit log with time, session, target, value, and release.

## 5. Breakpoints, stepping, and call stacks

### 5.1 Breakpoints

Breakpoints are source-level locations resolved to compiler-emitted checkpoints. Codegen (from
M6.5) emits a checkpoint table and a call to a host check at each eligible statement/block
boundary; M4.5 and M6 did not emit either. The host does not patch machine code and does not
require writable text pages. A breakpoint:

- is set/cleared by source location or checkpoint ID;
- is matched against the active generation's table;
- stops the cycle thread before the checkpoint's statement executes;
- reports the generation, frame, source location, and cycle number.

If a source location has no checkpoint in the active generation, the breakpoint is reported as
unbound rather than silently moved to an unrelated statement.

### 5.2 Stepping

Stepping is stop-the-world: the cycle thread is suspended at a checkpoint. Step-over, step-into,
step-out, and continue are supported. A step that would cross an online swap ends the session
instead of continuing into a different generation. While the cycle is stopped, the bounded stop
budget of §14.3 (`[debug].session-timeout`, default five minutes) applies; if the bound is
exceeded the session is terminated and the cycle resumes with a fresh deadline. M6 has no
watchdog; if one is added later it is paused or extended while the cycle is stopped, and the stop
budget still terminates the session.

### 5.3 Call stack

The runtime unwinds the stopped call stack using debug-frame information and returns frames with
function identity, source location, and typed locals/parameters where available. In optimized
builds, frames or variables may be unavailable; the report marks them as optimized out rather
than inventing values. `SUPER`/interface dispatch reports the dynamic function identity.

## 6. Debug information

### 6.1 Default and release

Debug information is the default build output. `--release` selects the optional stripped and
optimised mode. Debug and release builds have distinct build IDs and manifests. Debug hooks and
checkpoint calls are absent from release artifacts. M6.5 implements this; M4.5 and M6 emitted
no debug information and rejected `--release` (§12 records that historical boundary).

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
as manifest-assisted so a user does not mistake it for a raw struct dump. The host produces this
view from the same typed read that returns the value (`var.read` and `frame.read`), and both the
JSON and text presentations are deterministic and contain no address, absolute path, or
timestamp.

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

The CLI provides `stc debug` (build and launch a session, or attach with `--attach`) and
debug-related build options; the command surface is fixed in
[CLI and Manifest](CLI-and-Manifest.md) §1.7. Diagnostics use the stable
`STC-E-DEBUG-*` / `STC-W-DEBUG-*` family defined in §18 for protocol, session, breakpoint, force,
and debug-information errors. Online-change diagnostics use their own
`MODULE`/`MANIFEST`/`ONLINE` families.

## 9. Verification

- Typed read/write/force/watch tests for scalars, aggregates, strings, interfaces, and direct
  variables, including removed-field and layout-change rejection.
- Breakpoint, step, call-stack, and optimized-out reporting tests.
- DWARF/CodeView inspection with platform tools on Windows and Linux.
- Session teardown, force clearing, pause-bound expiry/resume, and swap-end tests.
- Debug/release build-ID, manifest, stripping, and hook-absence tests.
- Concurrency tests for generation pinning, service contention, and bounded queues.

## 10. Non-goals

- Safety functions, safety certification, automatic fail-safe outputs, or a safety watchdog.
- Remote/network debugging in v1.
- Raw memory editing in the standard API.
- Persistent breakpoints/forces across online swaps or restarts.
- Reconstructing missing source or debug information from a manifest.
- A general-purpose profiler or trace analyzer; the debug service collects no profiles or
  execution traces.

## 11. Related documents

- [Online Change and GOT](Online-Change-and-GOT.md) — generations, swaps, manifest identity.
- [Runtime and Execution](Runtime-and-Execution.md) — cycle loop, services, traps.
- [State and RETAIN](State-and-Retain.md) — state-file interaction.
- [Testing and Verification](Testing-and-Verification.md) — debug test levels.
- [CLI and Manifest](CLI-and-Manifest.md) — debug CLI and `[debug]` table.

## 12. M4.5/M6 interface notes (2026-10-27)

**Completion note (2026-10-28):** M6.5 implements the surface described in §5–§10 and
§13–§18. This section is kept as the historical M4.5/M6 boundary record; §12.1–§12.3
describe the pre-M6.5 state and the rules M6.5 followed, not current gaps. The GOT manifest was
raised to format 2 by M6.5 phase 4 (D-180); descriptor, module, state, and metadata-sidecar
formats remain 1, the metadata sidecar gains only the optional `debug` object, M6.5 adds the
format-1 stopped-frame sidecar, and the host ABI appends service 13 at minor 2.3.

M4.5 and M6 do not implement the debugger. They fix the host/module substrate and leave the debug
surface to M6.5. This section records the boundary so M6.5 can build on it without changing
the descriptor or module format 1 or the ABI 2 service signatures; the GOT manifest was later
raised to format 2 by M6.5 phase 4 (D-180).

### 12.1 Available before M6.5 (historical)

- **Identity, layout, and storage.** Every persistent root, function, instance, task, and export
  has a stable `GotId`, normalized key, and kind-specific identity (type/layout/signature) in the
  active manifest; roots additionally carry size and source location.
  `Host::active_manifest()`, `Host::roots()`, `Host::root_storage(got_id)`, and
  `Host::export_manifest*()` expose the active baseline and stable root bytes without copying or
  moving storage. The metadata sidecar carries the process-image layout and direct bindings for
  the split-object view.
- **Generation pinning.** `Host::pin_active()` and `Host::pin(generation)` return a
  `GenerationPin`; the pin keeps a retired generation and its module loaded, and a cold install
  fails while any pin is held. This is the epoch/refcount primitive for typed reads, watch
  snapshots, and debug-info lifetime. M4.5 and M6 have no service/monitor thread yet, so pins are
  used by tests, embedding hosts, and the control/status snapshot.
- **Descriptor feature gates.** The module descriptor has `entry_flags` bit 4 (debug build form)
  and bit 6 (debug hooks active), plus `required_features` bit 4 (debug checkpoint hooks). M4.5
  sets the debug-build form bit but neither debug-hook bit, and it requires no host debug
  support, so M6.5 can introduce checkpoints behind the feature bit without a descriptor format
  change.
- **Append-only host API.** ABI 2 services are appended within major 2 and gated by
  `struct_size`/minor version. Debug services can be added without breaking M4.5/M6 modules; a
  module only requires the services it uses. M6 fixed the next append at 2.2, service ID 12
  (`set_cycle_time_ns`, test builds only); M6.5 debug services therefore start at ID 13 and must
  not change service 12 or any earlier signature (D-157). M6 adds no checkpoint hook, debug
  information, or debug service, and its `stc run --watch` control channel is deployment/status
  only — it is not this document's debug protocol (D-155).
- **Trap records and boundary.** `TrapRecord { code, span_id, generation }` records the last
  module trap, and the non-local trap boundary turns a module trap into a failed generation (in
  the rollback window) or exit code 70. M6.5 can report trap state and stop/continue around it.
- **Swap semantics.** Activation is explicit (`Host::activate`), commit follows one successful
  cycle, rollback is bounded to that window, and retired-module unload waits for pins. M6.5 must
  end a debug session at activation/swap and drop the old generation's debug information after
  unload.
- **Direct variables.** The metadata bindings, `Host::warnings`, and `image::DirectRemap`
  identify direct variables that moved during an online change for the split-object view.

### 12.2 Not present before M6.5 (historical)

- No checkpoint table or host-check call is emitted by codegen; `entry_flags` bit 6 and
  `required_features` bit 4 stay clear in M4.5 and M6.
- No DWARF/CodeView emission. M4.5 and M6 artifacts use the default debug build form but contain
  no debug information. M6 parses `--debug`/`[build].debug` but does not emit debug information,
  and rejects `--release` as a usage error until M6.5 implements it (D-158).
- No typed read/write/force/watch API, write/force queue, or boundary application in the host
  API.
- No debug session, local transport, authentication, authorization, rate limits, audit log,
  watchdog pause, or output hold.
- No `stc debug` command and no `STC-E-DEBUG-*`/`STC-W-DEBUG-*` diagnostics.
- No service/monitor thread driving typed access or watch streams.
- No manifest-assisted split-object presentation; the manifest data it needs is available.

### 12.3 Rules for M6.5 (historical; followed)

- Add the checkpoint host check as an appended ABI 2.x entry starting after M6's service ID 12;
  typed access, reference resolution, and generation pins are host-side and need no new
  module-facing service. Do not change any existing service signature, service 12, or the
  descriptor format.
- Set `required_features` bit 4 and `entry_flags` bit 6 only when a module actually emits
  checkpoint hooks; release artifacts must not set them.
- Keep `--release` artifacts hook-free and give debug/release builds distinct build IDs and
  manifests.
- Never take a cycle-blocking lock: pin a generation for reads, and queue writes/forces for the
  cycle boundary.
- End every debug session on an online swap; never keep a raw code or descriptor pointer across
  unload.
- Preserve the non-safety stance in CLI output and documentation.

## 13. Debug protocol

This section fixes the v1 debug wire contract implemented in M6.5. The debug service is separate
from the M6 control/status channel (D-155): a control endpoint, token, or message is never
accepted as a debug endpoint, and the debug service performs no deployment or status operation.
The protocol is local-only; remote debugging remains a non-goal (§10). The externally binding
choices are recorded as D-173–D-179.

### 13.1 Transport and endpoint

| `[debug].protocol` | Transport |
| --- | --- |
| `pipe` (default) | Windows named pipe created with a user-only DACL and acceptance of local clients only. Linux Unix domain socket in a user-private directory (mode 0700) with socket mode 0600. |
| `socket` | Loopback TCP on `127.0.0.1` with an ephemeral port. The host never binds a non-loopback address. The token is the only peer authentication; no TLS or remote-client authentication is claimed. |

The configured kind is reported as `transport` even when the OS primitive is a Unix socket.
`[debug].protocol` selects the default; `--debug-protocol` overrides it for a launched server.

The host binds, creates the audit log, and then atomically writes a user-only endpoint file (mode
0600 on Unix; a user-only DACL on Windows). The file is format 1 JSON with sorted keys:

```json
{
  "audit_log": "target/stc/debug/stc-debug-0123456789abcdef0123456789abcdef.audit.jsonl",
  "build_id": "0x0123456789abcdef0123456789abcdef",
  "endpoint": "<transport-specific endpoint>",
  "format": 1,
  "generation": 3,
  "mode": "debug",
  "pid": 1234,
  "protocol": "stc-debug/1",
  "token": "<64 lowercase hex characters>",
  "transport": "pipe"
}
```

- `token` is 32 bytes from the OS cryptographic random source, encoded as 64 lowercase hex
  characters. It is unique per host process run. The host compares it in constant time and never
  prints, logs, or stores it in a build artifact.
- `build_id`, `generation`, and `mode` describe the active generation when the file is written.
  They are refreshed atomically at each activation. `mode` is `debug` or `release`.
- The endpoint file is created exclusively and without following symlinks. If user-only
  permissions cannot be established, the server refuses to start (`STC-E-DEBUG-020`). A stale
  file from a crashed run is never reused; each launch uses a new random suffix.
- The audit path is `<endpoint base>.audit.jsonl` in the same directory. The endpoint directory
  must be user-controlled; the server refuses a symlinked endpoint or audit path.
- The file is deleted on clean shutdown. It is runtime state, not a build artifact, and contains
  no timestamps.

The driver chooses the default endpoint path under the active out-dir as
`stc-debug-<32 hex>.endpoint`; `--endpoint` (`stc debug`) or `--debug-endpoint` (`stc run`)
selects an explicit path whose parent directory must already exist. The host prints the endpoint
path in server and launch modes; the token is never printed. A client reads the endpoint file
only after the host reports it, or polls for it within the connect deadline.

### 13.2 Framing and version negotiation

- The stream is UTF-8 without a BOM. Every message is one compact JSON object terminated by a
  single LF. A literal LF inside a string is escaped, not embedded. Indentation, trailing commas,
  comments, and duplicate keys are invalid.
- A request frame is at most 1 MiB; a response or event is at most 4 MiB. A frame that exceeds
  its limit, or a partial frame that does not terminate within the request deadline, is
  `STC-E-DEBUG-004` and closes the connection. Three malformed frames in one connection also
  close it.
- Every message carries `"protocol":"stc-debug/1"`. Version negotiation is exact and happens on
  the first request. A first request carrying another version receives `STC-E-DEBUG-002` with
  `detail.supported = ["stc-debug/1"]`; the connection may retry once with a supported version.
  There is no unauthenticated banner, hello, or capability probe.
- Message shapes (wire keys are snake_case; `stc.toml` remains kebab-case):

  ```json
  {"kind":"request","protocol":"stc-debug/1","id":1,"token":"<hex>","op":"var.read","params":{}}
  {"kind":"response","protocol":"stc-debug/1","id":1,"status":"ok","result":{},"warnings":[]}
  {"kind":"response","protocol":"stc-debug/1","id":1,"status":"error","error":{"code":"STC-E-DEBUG-008","message":"unknown target","retryable":false,"detail":{}}}
  {"kind":"event","protocol":"stc-debug/1","session":"<id>","seq":7,"event":"stopped","data":{},"warnings":[]}
  ```

- `id` is a client-assigned integer in `[1, 2^53-1]`, strictly increasing on one connection; the
  response echoes it. At most one request is outstanding per connection. The server processes
  requests in order and may interleave events. A duplicated, decreasing, or out-of-range `id` is
  `STC-E-DEBUG-003`.
- `seq` is server-assigned per session, starts at 1, and strictly increases. Events are never
  reordered. State events (`stopped`, `resumed`, `session-ended`) are never dropped; `watch`
  events may be coalesced or dropped with `STC-W-DEBUG-003`.
- An unknown key, an unknown `kind`, a missing required key, a field for another operation, or an
  invalid parameter type is `STC-E-DEBUG-003`. Error messages are human-readable, contain no host
  absolute paths or token material, and never include a memory address.
- A success response and an event may carry `warnings`, an array of `{code,message,detail?}`
  objects from §18.2. Warnings never change the response status.

### 13.3 Authentication, authorization, and audit

Every request is checked in this order: framing and protocol, token, session state, operation
authorization. Authentication precedes every operation, including `session.start`.

- A missing, malformed, or wrong token is `STC-E-DEBUG-005`; the connection closes and the failure
  is audited. Because every authentication failure closes its connection, failures are counted per
  endpoint, not per connection. Twenty authentication failures on one endpoint within 60 seconds
  make the endpoint refuse new connections for 60 seconds, audited as `transport-error` with
  `detail.reason:"auth-refused"`. Failures are rate-limited before the constant-time comparison.
- `session.start` requests scopes from this closed set:
  - `observe`: `var.read`, `watch.*`, `session.status`, `stack.list`, `frame.read`,
    `manifest.get`.
  - `control`: `breakpoint.*`, `exec.*`, `session.stop`.
  - `write`: `var.write`.
  - `force`: `var.force`, `var.release`, `var.release-all`.

  The server grants a subset from host policy and returns it in `result.capabilities.scopes`. An
  unknown scope name is `STC-E-DEBUG-003`; a known but ungranted scope on an operation is
  `STC-E-DEBUG-006`. `observe` is always granted. `write` and `force` also require
  `[debug].allow-write`/`[debug].allow-force` or the matching launch flag. `session.stop` is
  always allowed with any granted scope; it is the unconditional teardown action.
- Protected data: reads of `RETAIN`/`PERSISTENT` roots, `AT` direct variables, and `manifest.get`
  require `observe` and are audited once per request. A watch over a protected target is audited
  at `watch.add` and `watch.remove`, not per batch. Writes, forces, releases, breakpoint changes,
  stops/resumes, authentication failures, and policy denials are audited.
- No operation accepts or returns a raw address, and the standard API has no raw-memory read,
  write, poke, or disassembly operation. References, pointers, class/interface identities, and
  object handles are reported as opaque identities (§15.5). A request that supplies an address
  field is an invalid request (`STC-E-DEBUG-003`).
- The audit log is format 1 JSON Lines, user-only, append-only, and opened before the endpoint
  accepts connections:

  ```json
  {"build_id":"0x0123456789abcdef0123456789abcdef","cycle":42,"detail":{},"event":"force","format":1,"generation":3,"local_date_ns":0,"monotonic_ns":123456,"outcome":"ok","scope":"force","seq":7,"session":"<id>","target":{"key":"MAIN.COUNTER","kind":"root"},"value":{"type":"DINT","value":"1"}}
  ```

  Fields are `format`, `seq`, `monotonic_ns`, optional `local_date_ns`, `session`, `event`,
  `outcome` (`ok`/`denied`/`error`), `scope`, `target`, `value`, `generation`, `build_id`,
  `cycle`, and `detail`. Every record has `format`, `seq`, `monotonic_ns`, `event`, and `outcome`;
  `session`, `scope`, `target`, `value`, `generation`, `build_id`, and `cycle` are present with
  null when not applicable; `detail` is always an object. `seq` starts at 1 per log file.
  `monotonic_ns` is the host monotonic clock; `local_date_ns` is the host local-date clock or 0
  when it is unavailable. Paths are relative or package keys, never absolute host paths. For
  protected reads and `manifest.get`, `value` is null. Values larger than 4 KiB are recorded as
  `{"truncated":true,"size":<bytes>}`. A write/force `value` is the §15.5 typed envelope,
  including typed element envelopes for arrays and aggregates; finite `REAL`/`LREAL` values are
  JSON numbers and non-finite values use the canonical strings. Audit events are
  `server-start`, `server-stop`,
  `connect`, `disconnect`, `auth-fail`, `session-start`, `session-end`, `read-protected`,
  `manifest-export`, `write`, `force`, `force-release`, `breakpoint-set`, `breakpoint-clear`,
  `stop`, `resume`, `step`, `swap-end`, `timeout`, `policy-deny`, and `transport-error`.
- An audit file that cannot be created or secured prevents server start (`STC-E-DEBUG-021`). An
  append failure refuses protected reads and every `write`/`force` operation
  (`STC-E-DEBUG-021`, fail closed); ordinary reads may continue. The audit log is runtime state,
  so it may contain time; build artifacts do not.

### 13.4 Operations and result shapes

The operation set is closed for `stc-debug/1`. An unknown operation is `STC-E-DEBUG-003`.

| Operation | Scope | Result |
| --- | --- | --- |
| `session.start` | token | `session`, `capabilities`, `generation`, `build_id`, `cycle`, `mode`, optional `warnings` |
| `session.status` | any | state, session, generation, `build_id`, cycle, stop information, queue/force/breakpoint/watch counts, `pause_remaining_ms` |
| `session.stop` | any | `ended:true`; the response is sent, then the connection closes |
| `var.read` | `observe` | typed value plus generation, `build_id`, cycle, `consistent`, `atomic`, `stopped`, and an optional `view` (§15.5) |
| `var.write` | `write` | `status:"queued"`, `queue_id` |
| `var.force` | `force` | `status:"queued"`, `queue_id` |
| `var.release` | `force` | `released:true`, `effective:"next-boundary"` |
| `var.release-all` | `force` | `released:<count>` |
| `watch.add` | `observe` | `watch_id`, `interval_ms`, resolved targets |
| `watch.remove` | `observe` | `removed:true` |
| `watch.list` | `observe` | `watches:[...]` |
| `breakpoint.set` | `control` | `breakpoint_id`, `bound`, bound checkpoint IDs and source |
| `breakpoint.clear` | `control` | `cleared:true` |
| `breakpoint.list` | `control` | `breakpoints:[...]` |
| `exec.continue` | `control` | `status:"running"`, previous `stop_id` |
| `exec.step` | `control` | `status:"stepping"`, `stop_id`, `mode` |
| `exec.pause` | `control` | `status:"pause-requested"` or `"stopped"` |
| `stack.list` | `observe` | `frames:[...]`, `truncated` |
| `frame.read` | `observe` | typed local/parameter value or availability reason, plus an optional `view` (§15.5) |
| `manifest.get` | `observe` | `format`, `digest`, and JSON data or base64 binary data |

Request parameters:

- `session.start`: `{"client":{"name":...,"version":...},"scopes":["observe",...]}`. A second
  `session.start` on the host while a session is active is `STC-E-DEBUG-007`.
- Target objects (§15.1): `{"kind":"got_id","got_id":"0x..."}`,
  `{"kind":"path","key":"MAIN.COUNTER"}`, `{"kind":"local","frame":0,"name":"x"}`. A `local`
  target is valid only for `frame.read` and requires the current `stop_id`.
- `var.read`:
  `{"target":<target>,"expected_generation":3,"expected_build_id":"0x...","range":{"start":0,"count":16}}`.
  `expected_*` are optional; when supplied they must match (`STC-E-DEBUG-010` otherwise).
  `range` is valid only for a single-root string, `WSTRING`/`USTRING` unit array, or single-root
  array. A read without `range` carries the whole-value `view`; a sliced read omits `view` so the
  presentation cannot disagree with the sliced `value`.
- `var.write`/`var.force`:
  `{"target":<target>,"value":<typed value>,"expected_generation":3,"expected_build_id":"0x...","expected_type":"DINT","expected_layout":"0x..."}`.
  Expected fields are optional and checked before enqueue and again at application.
- `watch.add`: `{"targets":[<target>...],"interval_ms":100}`; the interval is clamped to the
  server minimum and maximum and returned.
- `breakpoint.set`: `{"target":{"kind":"source","file":"src/main.st","line":12,"column":5}}` or
  `{"target":{"kind":"checkpoint","checkpoint_id":17}}`. `breakpoint.clear` uses
  `{"breakpoint_id":1}`.
- `exec.step`: `{"mode":"over"|"into"|"out","stop_id":4}`. A stale `stop_id` is
  `STC-E-DEBUG-015`. `stack.list` and `frame.read` also require the current `stop_id`.
- `manifest.get`: `{"format":"json"|"binary"}`. `json` returns the canonical manifest object in
  `data`; `binary` returns base64 in `data`. The response is limited to 4 MiB; a larger manifest
  is `STC-E-DEBUG-011` and may be read from the artifact by the client.

`session.start` returns a closed `capabilities` object:

```json
{
  "scopes": ["observe"],
  "hooks": true,
  "debug_info": "dwarf",
  "checkpoints": true,
  "stepping": true,
  "max_watch_points": 256,
  "max_breakpoints": 1024
}
```

`scopes` is the granted subset of §13.3. `hooks` is the descriptor hook capability;
`debug_info` is `dwarf`, `codeview`, or `none`; `checkpoints` and `stepping` are true only when a
valid checkpoint table and frame information are available; the two maxima repeat the §13.5
limits. A release or hook-free artifact reports `hooks:false`, `checkpoints:false`, and
`stepping:false` (and `debug_info:"none"` for a stripped release) with `STC-W-DEBUG-004`. No other
capability key is valid in `stc-debug/1`.

Events:

| Event | Data |
| --- | --- |
| `stopped` | `stop_id`, `reason` (`breakpoint`/`step`/`pause`), `checkpoint_id`, source, generation, `build_id`, cycle, optional `breakpoint_id` |
| `resumed` | `stop_id`, cycle |
| `watch` | `watch_id`, generation, `build_id`, cycle, `consistent`, `values:[{target,value?,available,code?}]` |
| `queue-result` | `queue_id`, `operation` (`write`/`force`), `outcome` (`applied`/`rejected`), cycle, optional `code`, optional `message` |
| `session-ended` | `reason` (`client`, `disconnect`, `pause-timeout`, `idle-timeout`, `swap`, `shutdown`, `target-exited`, `protocol-error`, `internal`), optional `code`, generation, `build_id`, cycle |

A `stopped` event is the only event that changes `stop_id`. `stack.list`/`frame.read` values
belong to that stop snapshot; a stale `stop_id` is rejected. A `queue-result` with
`outcome:"rejected"` carries the rejection code from §18.1 and does not close the session.

### 13.5 Limits, timeouts, and rate limits

| Limit | Value | Violation |
| --- | --- | --- |
| Connect and first-request deadline | 5 s | Close; audited |
| Request processing deadline | 5 s | `STC-E-DEBUG-013` |
| Stop/idle session bound | `[debug].session-timeout`, default 5 min | `STC-E-DEBUG-018`, session ends |
| Request frame | 1 MiB | `STC-E-DEBUG-004`, close |
| Response or event frame | 4 MiB | `STC-E-DEBUG-011` |
| Request rate | 100/s, burst 200, per session | `STC-E-DEBUG-012` |
| Rate-error circuit breaker | 20 rate errors in 60 s | Close |
| Watch minimum interval | 50 ms | Clamped; returned in `watch.add` |
| Watch points | 256 per session | `STC-E-DEBUG-011` |
| Watch event queue | 64 batches or 1 MiB | Drop oldest with `STC-W-DEBUG-003` |
| Breakpoints | 1024 per session | `STC-E-DEBUG-011` |
| Queued writes and forces | 1024 entries or 1 MiB | `STC-E-DEBUG-011` |
| Aggregate leaves per read | 1024 | `STC-E-DEBUG-011` |
| Stack frames per stop | 1024 | `truncated:true` |
| Checkpoint table | 1,000,000 entries or 64 MiB metadata | `STC-E-DEBUG-016` |

- A request timeout aborts only the request; the connection and session remain usable. Writes
  and forces are idempotent by `queue_id`: the client may retry the same semantic request and
  must reconcile with `session.status` and `queue-result`. Watch and breakpoint add/remove
  operations are not silently replayed; a retry first reconciles with `watch.list` or
  `breakpoint.list`.
- The stop bound is continuous from the moment the cycle thread parks. The client cannot extend
  it except by resuming and stopping again; `session.status` does not reset it.
- Limits are per host, not per connection; a second connection cannot create a second session.

### 13.6 Error object and close rules

An error response is:

```json
{"kind":"response","protocol":"stc-debug/1","id":12,"status":"error","error":{"code":"STC-E-DEBUG-010","message":"generation is stale","retryable":true,"detail":{"expected_generation":3,"active_generation":4}}}
```

`code` is one of the stable codes in §18.1, `message` is human-readable, `retryable` states
whether the same request may succeed after re-resolution, and `detail` is an object with
operation-specific non-secret data. An error response never includes a token, an absolute host
path, a raw address, or a partial typed value that failed validation.

Errors that close the connection are: authentication failure (E-005), framing/oversize (E-004),
unsupported protocol (E-002), repeated malformed requests, the rate-error circuit breaker, and an
internal error that leaves framing or session state untrusted (E-022). All other errors leave the
connection and session usable.

## 14. Session lifecycle

### 14.1 States

| State | Meaning | Transitions |
| --- | --- | --- |
| created | Connection accepted; no session | `session.start` -> active; auth/framing failure -> closed |
| active | Session authenticated; cycle running or stopping | breakpoint/step/pause -> stopped; disconnect/timeout/stop/swap/shutdown -> ending |
| stopped | Cycle thread parked at a checkpoint; output image held | continue/step -> active; stop/disconnect/timeout/swap/shutdown -> ending |
| ending | Teardown in progress; no new requests accepted | cleanup completes -> closed |
| closed | Connection closed; session resources released | none |

There is exactly one session per host process and one connection per session. A second connection
may authenticate but `session.start` fails with `STC-E-DEBUG-007` until the first session ends.
`session.status` is served only on the session's connection.

### 14.2 Start, stop, and output hold

`session.start` authenticates and negotiates scopes; it does not stop the cycle and does not
pause the watchdog (M6 has no watchdog; §14.3 fixes the future interaction). Breakpoints and
watches may be added while running. A stop occurs when a cycle thread reaches a breakpoint
checkpoint, a step plan completes, or a pause request is observed at the next checkpoint. The
cycle thread parks inside service 13 and does not execute ST until resumed.

While stopped:

- the output image is not re-published; outputs remain at the last published values;
- the I/O backend is not called for output publication, and input latching is suspended;
- normal task execution, state saves, and online activation are suspended;
- typed reads copy root storage and may return `consistent:true, atomic:true` for one root;
- queued writes and forces are accepted but not applied.

The stop is visible to the runtime as a host service call that does not return. The service
thread remains responsive; the loader/prepare thread may prepare a candidate but cannot activate
it without ending the session (§14.5).

### 14.3 Pause bound and watchdog interaction

The documented pause bound is `[debug].session-timeout`, default `5m`, configurable per launch by
`--session-timeout`. The bound covers the continuous time the cycle is stopped, from the parking
checkpoint to the resume. When it expires:

1. the host emits `session-ended` with `reason:"pause-timeout"` and code `STC-E-DEBUG-018`;
2. all breakpoints, step/pause plans, forces, and queued writes are cleared;
3. the cycle thread leaves service 13 and resumes the interrupted cycle;
4. the next cycle deadline is computed fresh, with no catch-up and no accumulated missed-cycle
   count.

An idle session with no watches that sends no successful request for the bound is ended with
`reason:"idle-timeout"`. A delivered watch batch counts as activity, so a watch-only session is
bounded by connection liveness rather than this timer. `session.status` does not extend either
bound.

M6 has no watchdog implementation; it has only a trap code for a watchdog expiry. Phase 6
implements this stop budget as the host-side pause bound. If a watchdog is added later, its
deadline is paused or extended while the cycle is parked, never treated as an elapsed cycle, and
the pause bound above still terminates the session. This is a non-safety behavior: the bound is
an availability control, not a safety function.

### 14.4 Generation, build, and cycle tagging

Every read, watch batch, stop, queue result, and session-ended event reports:

- `generation`: the generation counter of the active process image;
- `build_id`: the build-ID string of the active module/manifest pair;
- `cycle`: the last completed cycle counter, or the cycle in progress at a stop.

A stop snapshot is bound to the generation/build/cycle in its `stopped` event; frame and local
reads must use that snapshot. A read while running reports the generation pinned for the copy.
If the generation changed between resolution and copy, the read is retried once within the pin
and then fails `STC-E-DEBUG-010`. A watch batch reports one generation/build/cycle for the batch
and a per-value `consistent` flag; split aggregates are `consistent:false` when their leaves span
cycles.

### 14.5 Online swap teardown

Every online swap ends the debug session before the new generation becomes active (D-175,
D-104):

- If a candidate activation is requested while the cycle is stopped, the stop is aborted: the
  session ends with `reason:"swap"`, forces/breakpoints/queued writes are cleared, the cycle
  resumes to a boundary, and activation proceeds. The client must reconnect after the swap.
- If the cycle is running, activation proceeds; at the swap the session ends with
  `reason:"swap"` and code `STC-E-DEBUG-019`. The `session-ended` event carries the old
  generation/build ID. The connection then closes.
- The old generation's debug information, checkpoint table, and frame metadata are retained only
  while a read or stop snapshot pin holds them; the pin is dropped at session end and the module
  is unloaded when its last pin is released.
- No breakpoint, force, watch, or queued write migrates to the new generation. A client that
  reconnects starts a new session against the new generation and re-resolves all targets.
- A `session.stop` racing an activation is resolved in favor of the swap: the session ends with
  `reason:"swap"`; activation is never blocked indefinitely by a debugger.

### 14.6 Disconnect, timeout, and reconnect

A connection close, protocol-error close, or idle/pause timeout ends the session, clears
breakpoints, forces, step/pause plans, watches, and queued writes, and resumes the cycle if it
was stopped. The host audits the reason. No session state, stop snapshot, or typed value is
retained after teardown.

Reconnect is a new `session.start` on the same endpoint with the same per-run token (the token
does not rotate during a host run). Reconnect rules:

- no automatic reconnect and no replay of previous breakpoints, forces, watches, or queue entries;
- a client that reconnects after a swap uses the endpoint file's refreshed build ID and
  generation;
- a reconnect while another session is `ending` receives `STC-E-DEBUG-007` until teardown
  completes;
- authentication failure limits apply across reconnects on the same endpoint.

The endpoint remains bound until host shutdown; a failed session does not require restarting the
host. If the host is an executable launched by `stc debug` in launch mode, the CLI owns the host
and normally shuts it down at quit; attach mode leaves it running.

### 14.7 Clearing, host shutdown, and state interaction

- `session.stop`, disconnect, timeout, swap, target exit, and host shutdown clear every force,
  breakpoint, step/pause plan, watch, and queued write. Clearing is not persisted.
- Host shutdown first stops the session and clears forces, then runs the normal shutdown state
  policy. A forced `RETAIN`/`PERSISTENT` root may be saved at its current value; this is the
  `STC-W-DEBUG-005` interaction, not a persisted force table. The `.stc-state` file never records
  a force, breakpoint, or watch.
- A target trap ends the cycle as specified by the runtime. If a session is active and the host
  remains alive, it receives a `session-ended` event with `reason:"target-exited"`; otherwise
  the connection closes. The trap record remains available to runtime diagnostics.
- `force-sessions-end-on-swap` must be `true` in v1; `false` is a configuration error
  (`STC-E-PROJECT-010`), so an implementation cannot silently keep a session across a swap.

## 15. Typed access, write, and force rules

### 15.1 Targets and resolution

A target is one of:

- a stable key path string from the active manifest (`MAIN.COUNTER`, including package/instance
  path normalization), resolved through instance/field records;
- a `GotId` in `0x` + 32 lowercase hex form, resolved directly;
- a stopped-frame local/parameter (`kind:"local"` with `frame` and `name`), resolved through the
  stop snapshot's DWARF/CodeView metadata.

Resolution checks, in order: target shape, existence in the active manifest, expected
generation/build, expected type/layout, and operation writability. A raw address, offset, or
pointer expression is not a target. An unknown target is `STC-E-DEBUG-008`; a target that existed
in an expected generation but is removed or has a changed layout is `STC-E-DEBUG-010`. A removed
field is never silently retargeted to a same-named field in another instance.

Enum membership, subrange bounds, and split-storage parent shapes come from the manifest-format-2
`type_metadata` and `aggregates` records (GOT Artifacts §9). The host rejects a format-1 manifest
at load, so a resolved writable enum or subrange always has its declared member set or bounds,
and a whole-program instance or array-of-aggregate parent resolves through an aggregate record
instead of being guessed. A target absent from the type/root/aggregate tables remains
`STC-E-DEBUG-008`. No resolution path accepts a raw address, offset, or pointer expression.

### 15.2 Reads and consistency

- The service thread submits the read and waits for the reply; the cycle thread applies it at the
  next boundary in `Host::debug_pump_at`, pinning the active generation (`Host::pin_active`),
  resolving the target, copying the typed value from root/direct storage, and releasing the pin.
  The service thread never dereferences host, module, root, or process-image storage, takes no
  lock held by the cycle, and never blocks the cycle. `Host::debug_read` is the synchronous
  embedding API and performs the same pinned copy on the calling thread. `Host::cycle_at` pumps
  automatically; an embedding host that drives the cycle itself calls `Host::debug_pump_at` at a
  boundary.
- A scalar root, enum, subrange, time/date value, single-root string, or single-root scalar array
  is copied as one unit and reported `atomic:true`. An `OVERLAP` root is copied as its declared
  bytes.
- A split aggregate, multi-root object, interface, or reference is resolved leaf by leaf. A
  whole-program instance aggregate and an array-of-aggregate element resolve their parent shape
  through the format-2 aggregate records; their fields are still copied individually and the
  value is not presented as one atomic struct. Each leaf carries its own generation; the result
  is `consistent:false` if any leaf was copied from a different cycle than the reported cycle.
  The debugger labels this view manifest-assisted (§6.2); it is not a raw struct dump.
- Every unsliced read also returns the presentation `view` (§15.5). A live split target marks
  each node `manifest_assisted:true` and `contiguous:false`; a field with its own manifest root
  carries that root's `GotId`, key, and root kind, and a class, FB, interface, reference, or
  pointer node carries opaque identity metadata and hidden `HANDLE`/`OBJREC` roots instead of a
  raw handle value. A single-root target is one contiguous copy and is marked
  `contiguous:true, manifest_assisted:false`, so callers can distinguish it from split storage.
  A stopped-local view comes from one immutable snapshot copy: it is marked
  `origin:"stop_snapshot"`, `contiguous:true`, and has no live roots. Unavailable leaves stay
  `available:false` with `STC-W-DEBUG-002`; the view never invents a root, handle, or value.
- Locals and parameters are read only from the current stopped-frame snapshot. They are never
  read from a running frame. A stale `stop_id` is `STC-E-DEBUG-015`; an unavailable or
  optimized-out local is reported with `available:false` and `STC-W-DEBUG-002`, not
  reconstructed.
- A read is never repeated automatically across a swap. The pin prevents unload during one read;
  the generation tag lets the client detect a boundary between reads.

### 15.3 Write queue

A write request:

- is validated for target kind, type, shape, range, encoding, and write permission before
  enqueue;
- is stored with a server-assigned `queue_id`, the resolved `GotId`, expected
  generation/build/type/layout, and a typed copy of the value;
- is drained by the cycle thread at the next boundary, after normal ST writes and before output
  publication;
- is re-resolved and re-validated at application; a stale generation/layout or a removed target
  rejects it with `queue-result` `outcome:"rejected"` and `STC-E-DEBUG-010`;
- is applied in queue order; multiple writes are not a transaction and may be observed
  individually by a subsequent read;
- never targets a local, parameter, class/interface identity, `REF_TO`/`POINTER` identity, or a
  field that has no persistent root of its own. A field with its own manifest root is an ordinary
  scalar target; a field reached only through a split parent is not writable as a parent value;
- is limited with the force queue to 1024 entries or 1 MiB; over either limit is
  `STC-E-DEBUG-011` at enqueue, never backpressure on the cycle thread;
- must match the target's exact declared type. An actual value type mismatch is
  `STC-E-DEBUG-009`; an expected generation/build/type/layout mismatch is `STC-E-DEBUG-010`.
  Enum membership and subrange bounds are checked from the format-2 metadata, and arrays/strings
  must match their declared count, element type, capacity, and unit range. There is no implicit
  conversion.

A write queued while stopped remains queued until the cycle resumes and reaches the boundary. A
write to an input is overwritten by the next input latch unless an active force holds it. A write
to an output is visible in the same cycle's publication. A write to a `RETAIN` root follows the
normal state-save policy.

A write to a 1-bit direct variable (`%IX`/`%QX`/`%MX` bit form) updates only that bit in the
shared process-image byte and preserves the other bits; the active process-image binding supplies
the byte offset and bit position.

### 15.4 Force queue and release

A force is a write that is re-applied at every boundary until released. Rules:

- `var.force` validates and enqueues like a write; it returns `status:"queued"` and the
  `queue-result` reports `applied` when the force becomes active at a boundary;
- there is one force per target `GotId`; a second force replaces the value and is audited as a
  new force;
- active forces are applied at the boundary after queued writes, so a force wins over a write to
  the same target in that cycle;
- an active force on an input is also substituted at the input latch of the next scan so the
  module sees it; an active force on an output is applied before publication;
- `var.release` removes the force entry; the value in storage is no longer overridden and the
  program's subsequent normal writes determine it. `var.release-all` releases every force in the
  session;
- releasing is effective at the next boundary; a release while stopped takes effect when the
  cycle resumes;
- session end, disconnect, timeout, swap, and host shutdown clear all forces. Forces are never
  written to the state file and never restored. A host-level clear releases every active force
  and discards every queued write/force even when no session is attached; queued entries produce
  rejected queue results with `STC-E-DEBUG-010` and are never migrated to another generation;
- every force and release is audited with target, typed value, scope, generation, and cycle. A
  force on a `RETAIN`/`PERSISTENT` root carries `STC-W-DEBUG-005`. Releasing a target that has no
  active force is a successful no-op.

### 15.5 Value and identity encoding

The typed value envelope is `{"type":"<canonical type key>","value":...}`. The type key uses the
manifest's normalized type spelling (for example `DINT`, `REAL`, `STRING[80]`,
`ARRAY[1..4] OF INT`). The host rejects a mismatched declared type unless an expected type was
supplied and matches.

| Type class | Encoding |
| --- | --- |
| `BOOL` | JSON boolean |
| Integer, enum, subrange, `TIME`, `DATE`, `DT`, `TOD`, `LTIME` | Decimal string (avoids JSON precision loss) |
| `REAL`, `LREAL` | JSON number when finite; `"NaN"`, `"INF"`, `"-INF"` strings otherwise |
| `STRING` | `{"length":N,"units":[<code points>]}`; `WSTRING`/`USTRING` use the same unit array with code points |
| Single-root array | `{"elements":[<typed value>...]}` in declared index order |
| `OVERLAP` | `{"bytes":"<base64>"}`; read-only |
| `REF_TO`, `POINTER` | `{"identity":"0x<16 hex>","present":<bool>,"target_type":...}`; no address |
| Class/interface object | `{"identity":"0x<16 hex>","class":...,"interface":...,"fields":[...]}`; opaque identity |
| Split aggregate | `{"aggregate":true,"atomic":false,"fields":[{"name":...,"value":...}]}` |
| Optimized-out/unavailable | `{"available":false,"code":"STC-W-DEBUG-002"}` |

Strings are validated for capacity and valid code points; an out-of-range integer, invalid enum
value, out-of-bounds subrange value, wrong element count, or over-capacity string is
`STC-E-DEBUG-009`. There is no implicit conversion between types. A 64-bit integer is transported
as a decimal string so no precision is lost. A reference/pointer identity is stable only within
the active generation; it is not an address and cannot be dereferenced. A class/interface value
is never written or forced; only its scalar fields are individually writable if the manifest
marks them as persistent roots. The artifact JSON projection is integer-only and is never used
for audit or protocol values; a finite `REAL`/`LREAL` audit value is written as a JSON number
token and non-finite values as `NaN`, `INF`, or `-INF`.

#### 15.5.1 Manifest-assisted view (D-182)

`var.read` and `frame.read` add one optional `view` key; this is an additive result extension,
not a new operation, and every pre-existing key keeps its meaning. The view is omitted when the
request supplies `range`. It describes the same copy as `value`:

| Key | Meaning |
| --- | --- |
| `target` | normalized request target; for `frame.read`, `"<frame>:<name>"` |
| `resolved_key` | canonical manifest key, or the normalized local path |
| `generation`, `build_id`, `cycle` | tags of the copy |
| `origin` | `"live"` (root/aggregate storage) or `"stop_snapshot"` (immutable captured local) |
| `atomic`, `consistent` | read-level tags, unchanged from the typed result |
| `manifest_assisted`, `contiguous` | storage labels of the root node |
| `view` | root node (below) |

A node carries `kind` (`scalar`, `overlap`, `aggregate`, `program`, `array`, `class`,
`function_block`, `interface`, `reference`, `unavailable`), `type` (canonical type key),
`layout_hash` (hex, or `null` when the manifest has none), `available`, `manifest_assisted`,
`contiguous`, `atomic`, `consistent`, optional `root`, optional `identity`, `fields`, and
`elements`. `root` is `{got_id,key,kind,marker}` for a persistent backing root; `marker` is
`"HANDLE"` or `"OBJREC"` for a hidden object root and `null` otherwise. `identity` is
`{kind,identity,present,class,interface,target_type,roots}` with the same opaque identity as the
typed value and no raw handle or address. Fields are ordered by the manifest/type declaration and
elements by declared index; a field object adds its `name` and normalized `path`. Composite kinds
represent their content only through `fields`/`elements`; `scalar`, `overlap`, `reference`, and
`unavailable` nodes also carry the typed `value`. Split nodes carry `manifest_assisted:true` and
`contiguous:false`; a single-root or snapshot node is `contiguous:true` with
`manifest_assisted:false`. Unavailable nodes leave `available:false`, carry the `STC-W-DEBUG-002`
warning in `value`, and are not assigned roots. The JSON object is key-sorted, and the text
presentation escapes control characters, so repeated rendering of one view is byte-identical and
safe for a terminal.

### 15.6 Boundary application order

At each completed scan boundary the host performs, in order:

1. latch backend inputs, substituting active input forces after the latch;
2. evaluate task guards and execute the module cycle (normal ST writes);
3. apply queued debug writes in queue order;
4. apply active forces in ascending `GotId` order;
5. publish outputs;
6. complete the cycle counter and release the boundary.

This is the only place debug writes and forces modify values. The service thread never writes a
root cell. The cycle thread never waits for a service request; a queue that exceeds its limit is
rejected at enqueue (`STC-E-DEBUG-011`), not by blocking the cycle. Split-aggregate leaves are
written only as complete root values; per-field writes resolve to the field's own root where the
manifest provides one. `OVERLAP` roots are read-only (see §15.5) and are rejected as write or
force targets with `STC-E-DEBUG-009`.

### 15.7 Watches

- A watch is a repeated read of up to 256 targets at an interval of at least 50 ms; the service
  thread coalesces missed periods into one batch, never a backlog of identical batches.
- A new watch is due immediately: the first batch (the current values) is emitted at the next
  boundary pump, and later batches follow at the watch interval. A watch batch may therefore
  arrive before a teardown event on the same connection; this is within the coalescing rules.
- Each batch is tagged with generation, build ID, cycle, and per-value `consistent`; values use
  §15.5. Unavailable values carry their warning code per value.
- If the event queue is full (64 batches or 1 MiB), the oldest watch batch is dropped and the
  next batch carries `STC-W-DEBUG-003`; state events are never dropped.
- Watch reads pin the generation for the copy and never block the cycle. A watch that spans a
  swap ends with the session; no watch continues into the new generation.
- A watch over protected data is audited at add/remove, not per batch. A watch may not target a
  local outside a stop snapshot.

### 15.8 Rejection and stale-target rules

| Condition | Code |
| --- | --- |
| Unknown path/`GotId`/local/frame/checkpoint/breakpoint/watch ID | `STC-E-DEBUG-008` |
| Wrong type, not writable, wrong shape, out of range, invalid enum/code point, capacity overflow | `STC-E-DEBUG-009` |
| Expected generation/build/type/layout differs; target removed; layout changed | `STC-E-DEBUG-010` |
| Queue/breakpoint/watch/leaf/response limit exceeded | `STC-E-DEBUG-011` |
| Rate limit exceeded | `STC-E-DEBUG-012` |
| Request timeout | `STC-E-DEBUG-013` |
| No hooks/breakpoint/frame information available | `STC-E-DEBUG-014` |
| Invalid step/continue or stale stop frame | `STC-E-DEBUG-015` |
| Invalid/missing/inconsistent debug metadata or checkpoint table | `STC-E-DEBUG-016` |
| Source/locals/stack debug information unavailable | `STC-E-DEBUG-017` |

A stale target is never retried against a different generation automatically; the client
re-resolves against the reported active generation and issues a new request. A write or force
that is already queued when the generation changes is rejected by the `queue-result`, not
retargeted.

## 16. Checkpoint table and host check

### 16.1 Location and artifact set

The checkpoint table is emitted in the `<name>.stc-meta.json` metadata sidecar under a `debug`
object. It is **not** a new module export, a descriptor field, a manifest section, or a member of
the loadable module's data. The loadable module contains only the per-site call immediates passed
to service 13. This preserves descriptor format 1, the seven module entry points, and all
existing ABI signatures (D-177). The GOT manifest and its binary/JSON projections are format 2
from M6.5 phase 4 (D-180); that manifest change is independent of the checkpoint table and does
not change the descriptor or module format.

The metadata sidecar is already part of the module artifact set: the host reads it for the
process-image layout and direct bindings. The debug object is paired with the module/manifest by
`build_id`; it is not part of build-ID derivation, manifest identity, or online-change
compatibility. A host may load and run a hook-free module without the object; a debug session
that needs it validates it at `session.start`.

### 16.2 Table encoding

Format 1 in the sidecar:

```json
{
  "build_id": "0x0123456789abcdef0123456789abcdef",
  "checkpoints": [
    {"column": 3, "flags": 1, "function": "0x11111111111111111111111111111111", "id": 1, "line": 6, "span": 12}
  ],
  "format": 1,
  "hooks": true,
  "info": "dwarf"
}
```

- `format` is 1.
- `build_id` is the build ID of the paired module/manifest; a mismatch is
  `STC-E-DEBUG-016`.
- `hooks` is true only when the module imports service 13 and at least one checkpoint is emitted.
  It must equal the descriptor's `required_features` bit 4 and `entry_flags` bit 6.
- `info` is `dwarf` or `codeview` for a debug build (the platform's emitted debug format). It is
  `none` only for an explicit hook-free artifact. The host uses it for capability reporting, not
  for breakpoint resolution.
- `checkpoints` is sorted by `id` ascending; IDs are dense from 1. `id` 0 is invalid. The array
  length is the checkpoint count.
- `span` is a span-table index in the metadata sidecar; `function` is a function `GotId` from the
  active manifest. `line` and `column` are 1-based source coordinates of the checkpoint; `span`
  identifies the source file and range.
- `flags` bits: 0 statement entry, 1 block entry, 2 call site, 3 loop back-edge. Bits 4–31 are
  reserved and must be zero. A site may set several bits.
- The table has at most 1,000,000 entries and the `debug` object at most 64 MiB; a larger table
  rejects debug sessions with `STC-E-DEBUG-016`.

### 16.3 Checkpoint IDs and ordering

Checkpoints are collected after code generation for one build, before the metadata is written.
Eligible sites are:

- the entry of each executable statement;
- the first statement of each `IF`/`ELSIF`/`ELSE`, `CASE` arm, and loop-body block;
- each call site, before the call;
- each loop back-edge;
- each `RETURN`.

Expression-level, synthetic standard-function-body, empty-span, and unreachable sites are not
eligible. IDs are assigned after sorting sites by function `GotId` bytes, then span start, span
end, flags, and source order; IDs are then dense 1..N. The sort is stable and independent of
hash-map iteration and file-system order. A function with no sites contributes no entries. A
release build assigns none.

### 16.4 Host check signature and service ID

Host ABI 2.3 appends service ID 13 after service ID 12:

```c
void (*debug_check)(uint32_t checkpoint_id, uint32_t span_id);
```

- The service is appended to `stc_host_api`; its field follows `set_cycle_time_ns`. The
  `struct_size`/minor gate is unchanged: a module that imports service 13 requires host minor
  >= 2.3, and a 2.2 host that loads it fails the normal required-feature/minor check.
- `debug_check` is called by generated module code on the cycle thread, immediately before the
  checkpoint's statement/block/call/back-edge executes. It returns normally, may block while the
  session is stopped, and is a no-op that returns immediately when no debug session is active.
- The module's import slot is a normal `Import` slot with service 13; `span_id` is the span-table
  ID used by trap reporting. The host validates that the table entry for `checkpoint_id` exists
  and that its span matches. A mismatch reports `STC-E-DEBUG-016` and disables breakpoints for
  the session, but the module continues to run.
- `debug_check` never runs on the loader/prepare thread, never allocates in the module, and never
  changes the module's observable behavior when no session is active.

### 16.5 Descriptor and feature bits

- `required_features` bit 4 (debug checkpoint hooks) is set exactly when the module emits at
  least one `debug_check` call and imports service 13; it requires `host_abi_minor >= 3`.
- `entry_flags` bit 6 (debug hooks active) equals `required_features` bit 4. A descriptor with
  one set and the other clear is inconsistent and is rejected by the host's normal descriptor
  validation (`STC-E-MODULE-002`).
- `entry_flags` bit 4 (debug build form) is set for every debug-form build, including test builds
  and hook-free debug builds; it is clear for release. It does not by itself require host 2.3.
- Test builds remain hook-free: `required_features` bit 4 clear, `entry_flags` bit 6 clear,
  `debug.hooks=false`, empty checkpoint array, and required host minor 2.2 (service 12). This
  keeps the measured test ABI and M6 test modules unchanged.
- Release builds: bit 4 clear, bit 6 clear, feature bit 4 clear, no `debug` object, required host
  minor 2.1.

### 16.6 Release absence and hook-free artifacts

A release artifact contains no checkpoint table, no `debug` object, no `debug_check` import, no
DWARF/CodeView sections (after stripping), and no debug hooks. Its metadata sidecar is present
for the runtime layout but has no `debug` key. `stc debug` may attach to a release host in
monitoring-only mode; `breakpoint.set` and `exec.*` return `STC-E-DEBUG-014`, and source/stack
reads return `STC-E-DEBUG-017`. A hook-free debug build (`hooks:false`) is treated the same way
for breakpoint/step operations while retaining its typed read/write/force/watch support.

Monitoring-only means typed `var.read`/`var.write`/`var.force`/`var.release`, watches, status,
and `manifest.get` remain available subject to the same token/scope/policy checks; breakpoints,
stepping, pause, `stack.list`, and `frame.read` are unavailable because no checkpoint or frame
metadata exists. Writes and forces still apply only at cycle boundaries.

### 16.7 Validation and breakpoint binding

At `session.start`, the host validates the `debug` object if present: format, build ID, dense
sorted IDs, sparse reserved flags, valid span indexes, manifest function identities, nonzero
line/column, the debug-object/descriptor hook agreement, and the size limits. Failure refuses
debug features with `STC-E-DEBUG-016`; the module keeps running and typed monitoring remains
available regardless. Breakpoints and stepping are refused.

A source breakpoint binds to the table, not to DWARF:

- `{"kind":"source","file":"src/main.st","line":12,"column":5}` matches checkpoints whose span
  file is `src/main.st` and whose `line` is 12;
- with a positive `column`, the host selects the smallest ID whose column is >= the requested
  column; if none, the smallest ID on the line; with no column (0 or absent), the smallest ID on
  the line;
- if no checkpoint matches, the breakpoint is returned with `bound:false` and
  `STC-W-DEBUG-001`; it is never moved to an unrelated line;
- `{"kind":"checkpoint","checkpoint_id":17}` binds directly and rejects an unknown ID with
  `STC-E-DEBUG-008`;
- a breakpoint bound before a swap is cleared by the session end; rebinding after reconnect is
  explicit;
- at most 1024 breakpoints, each with one source or checkpoint binding.

### 16.8 Stepping and call stacks

A stop snapshot captures the checkpoint ID, source location, generation, build ID, cycle, the ST
call chain, and the register/stack context needed for DWARF/CodeView frame and local reads. The
snapshot is bound to `stop_id`; continue/step invalidates it.

- Step over stops at the next checkpoint whose call depth is less than or equal to the current
  depth after at least one checkpoint; step into stops at the next checkpoint at any depth; step
  out stops at the next checkpoint with lower depth. A plan that cannot be satisfied because the
  frame is optimized out or the depth is unavailable is `STC-E-DEBUG-015`.
- A breakpoint hit cancels a pending step plan and is reported with `reason:"breakpoint"`.
- Pause is honored at the next checkpoint after the request; a pause with no remaining checkpoint
  in the scan stops at the first checkpoint of the next scan. If the build has no checkpoints,
  pause is `STC-E-DEBUG-014`.
- Frames unavailable in the unwind are reported as `optimized_out:true` with
  `STC-W-DEBUG-002`; locals unavailable in a frame are reported per value. `SUPER`/interface
  dispatch reports the dynamic function identity from the manifest.
- The stack and register context are captured on the cycle thread at the checkpoint; frame and
  local reads use the snapshot, never a raw stack address exposed to the client.

### 16.9 Stopped-frame sidecar

A hook-emitting debug build emits `<name>.stc-frames.json` alongside the module, manifest,
metadata, and debug object. It is format-1 JSON, deterministic, paired by `build_id`, optional at
load, and excluded from build-ID, manifest, descriptor, module, state, metadata-sidecar, and
host-ABI identity. The front end does not know LLVM's final frame offsets, so the table is
extracted at build time from the module's emitted DWARF/CodeView (or an equivalent deterministic
backend-derived source); the runtime never parses DWARF/CodeView. A build whose debug information
cannot be extracted emits no frame table, and stack/frame reads are then unavailable rather than
guessed.

Format 1:

```json
{
  "build_id": "0x0123456789abcdef0123456789abcdef",
  "format": 1,
  "functions": [
    {
      "cfa": "fp+16",
      "frame_size": 48,
      "function": "0x11111111111111111111111111111111",
      "high_pc": 0,
      "locals": [
        {
          "live": {"first": 3, "last": 9},
          "location": "fp-24",
          "name": "COUNTER",
          "size": 4,
          "type": "DINT"
        }
      ],
      "low_pc": 0,
      "return_address": -8,
      "saved_fp": -16,
      "symbol": "stc_main_cycle"
    }
  ],
  "pointer_width": 8,
  "target": "x86_64-unknown-linux-gnu"
}
```

- `functions` is sorted by manifest function `GotId` bytes; locals are sorted by normalized name.
  `function` is the manifest function `GotId`, `symbol`/`low_pc`/`high_pc` identify the code
  range, and `frame_size`/`saved_fp`/`return_address`/`cfa` describe the frame layout.
- Locals and parameters carry the normalized identifier, canonical type key, size, a simple
  `fp`/`cfa`-relative location, and the live checkpoint/span range. Only simple static locations
  are defined in v1; a value without one is reported unavailable, not reconstructed.
- The table has at most 1,000,000 locals and is at most 64 MiB; records are validated for
  identity, ordering, bounds, and location form. A missing or invalid table makes
  `stack.list`/`frame.read` unavailable with `STC-E-DEBUG-017`; breakpoints and stepping depend
  on the checkpoint table, not this sidecar.
- Debug builds force frame pointers (`frame-pointer="all"`) and disable tail calls in generated
  code so the saved-frame-pointer chain is reliable; release builds are unchanged.
- At `debug_check` the cycle thread captures the frame, stack, and instruction pointers, walks
  the chain, maps return addresses through the function ranges/symbols, copies typed locals from
  their recorded locations, and stores immutable `StopFrame` snapshots behind `stop_id`. The
  protocol thread reads only those snapshots. Values or frames the table cannot describe are
  `available:false`/`optimized_out:true` with `STC-W-DEBUG-002`; no raw address is returned to a
  client.
- The format-1 frame rule covers x86-64 Windows and Linux and AArch64 Linux. On AArch64 the
  service-13 entry point captures `x29` (frame pointer), `sp`, and `x30` (return address) and
  passes them in place of the x86-64 `rbp`/`rsp`/stack return address; the build-time extractor
  recognizes `DW_AT_frame_base` with `DW_OP_reg29 (x29)` in addition to x86-64 `DW_OP_reg6
  (rbp)`, and uses the same `fp+16` CFA, `saved_fp = -16`, and `return_address = -8` layout.
  AArch64 modules are compiled with `-mbranch-protection=none` so the saved link register is the
  raw caller address the walk compares against manifest code ranges. A function compiled without
  a frame pointer has no usable frame base; it is reported as an optimized-out caller
  (`optimized_out:true` with `STC-W-DEBUG-002`) rather than walked with a guessed layout.

## 17. Debug and release build rules

### 17.1 Default mode and selection

Debug is the default build mode. The mode is selected by, in precedence order:

1. `--debug` or `--release` on the command line, which are mutually exclusive; both together is
   `STC-E-PROJECT-010` and exit 2;
2. an explicitly present `[build].debug` boolean;
3. an explicitly present `[debug].default` boolean;
4. the default `true` (debug).

If both `[build].debug` and `[debug].default` are explicitly present and disagree, the build
fails with `STC-E-PROJECT-010`; they are aliases and must agree. `[debug].default` selects the
default build mode; it does not enable a runtime server. A runtime server is enabled only by
`stc debug`, `stc run --debug-server`, or an embedding host's `stc_host_options.debug_endpoint`.

`stc debug` defaults to debug and accepts `--release` for a monitoring-only session. `stc test`
always builds the hook-free debug form and rejects `--release`. `stc build`, `stc run`,
`stc online`, and `stc debug` accept both modes; `stc clean` removes both mode directories and the
test tree by default, and `stc clean --release` removes just the release default directory
(D-184).

### 17.2 Release semantics

`--release`, or a manifest that selects the release form with `[build].debug = false` (or
`[debug].default = false` when `[build].debug` is absent), selects the optimised build:

- default `[build].opt-level = 2` when the key is absent; an explicit `[build].opt-level`
  overrides it;
- debug builds default to `[build].opt-level = 0` when the key is absent; an explicit value is
  honored (the build remains the debug form with debug information);
- no checkpoint table, no `debug_check` import, no `entry_flags` bit 6, no `required_features`
  bit 4; `entry_flags` bit 4 is clear, and required host minor is 2.1;
- full debug information is absent; symbol/debug sections are stripped as in §17.5;
- the build ID and manifest are distinct from a debug build of the same sources because
  `generator_flags` bit 1 differs, so the pairing identity differs.

### 17.3 Debug information

A debug build emits:

- DWARF on ELF targets, including the line table, type/field information, subprogram ranges,
  parameter/local locations, and frame unwind information;
- CodeView on MSVC targets, including line/type/symbol information and frame data;
- a deterministic producer string and no timestamps.

The supported v1 target families are the ones the runtime already ships (Windows MSVC and Linux
GNU). If the selected target's clang cannot emit the required format, the build fails with the
existing toolchain/build diagnostic rather than silently producing an undebuggable binary.
Debug information is derived from the clang target triple; `stc metadata` reports `debug_info`
as `dwarf`, `codeview`, or `none`.

### 17.4 Determinism

Debug and release builds follow the existing deterministic-build rules:

- absolute source/build paths are remapped with `-ffile-prefix-map` and the debug compilation
  directory is set to a stable relative value; no absolute path appears in DWARF/CodeView;
- no timestamps, random build IDs, host names, or user names are embedded;
- type, field, subprogram, and line-table order is a deterministic function of the source and
  manifest order, not hash-map iteration;
- the producer string is fixed by the toolchain pin;
- two builds of the same inputs emit byte-identical modules, manifests, metadata, and sidecars.

The build ID derivation and manifest canonicalization are unchanged. The debug object and debug
sections are not inputs to the build ID.

### 17.5 Stripping and sidecars

- A debug build keeps full DWARF/CodeView in the module. It may additionally emit a
  `<name>-<build-id>.pdb` (MSVC) or `<name>-<build-id>.debug` (ELF) sidecar; the module remains
  usable for source/line/type debugging without the sidecar, and the sidecar is never loaded by
  the runtime.
- A hook-emitting debug build also emits `<name>.stc-frames.json` (§16.9), the stopped-frame
  table used by the debugger. It is deterministic, paired by `build_id`, excluded from identity,
  optional at load, and never changes runtime behavior; a missing table disables stack/frame
  reads, not module loading.
- A release build strips `.debug_*`, `.zdebug_*`, `.debug$S`, and `.debug$T` sections (and the
  corresponding CodeView records) and emits no PDB. It may emit a build-ID-named public-symbol
  sidecar; the sidecar is optional, has no runtime effect, and is not part of the manifest.
- Sidecars are deterministic, contain no timestamps or absolute paths, are named by the build
  ID, and are excluded from build-ID derivation. Missing sidecars never change runtime behavior.
- Stripping never removes the build-ID note, descriptor, manifest, or metadata. A release module
  has no checkpoint imports to preserve.

### 17.6 Identity, manifests, and output directories

- Debug and release builds have distinct build IDs and manifests. The manifest, metadata, and
  module are written as one pair/set; a set never mixes modes.
- The default debug out-dir remains `target/stc/debug`; the default release out-dir is
  `target/stc/release`. `[build].out-dir` and `--out-dir` override the default verbatim for both
  modes. Building both modes into one explicit directory is not a supported layout because the
  manifest/metadata names collide; use distinct directories.
- `stc clean` removes the debug, release, and test trees and an explicit `[build].out-dir` by
  default; `--debug`, `--release`, or `--test` narrows it to one tree, and `--out-dir` selects
  exactly one directory. It preserves `stc.lock`, retained-state files, and the runtime
  module/manifest store (D-184).
- `stc metadata` appends `build_mode` (`debug`/`release`), `debug_info`
  (`dwarf`/`codeview`/`none`), and `debug_hooks` (boolean) to its summary; the sidecar remains
  authoritative for the checkpoint table.
- The descriptor, module, metadata-sidecar, and report formats remain format 1. The GOT manifest
  is format 2 from M6.5 phase 4 (D-180) and carries the required type-metadata and aggregate
  sections; the audit object itself remains format 1.

### 17.7 Test and hook-free builds

Test builds are debug-form builds for the existing test driver:

- `entry_flags` bit 4 set, bit 6 clear; `required_features` bit 4 clear;
- `debug.hooks=false`, empty checkpoint array, `info` may be `dwarf`/`codeview`;
- required host minor 2.2 because the test ABI uses service 12;
- no debug server, no checkpoint calls, and no watch/force service enabled by `stc test`.

This preserves M6 test timing and module compatibility. A test build can still be read by a
debugger that supports typed access, but it cannot stop at checkpoints. `stc test` rejects
`--release`, `--debug-server`, and `--debug-protocol`.

## 18. Diagnostics and stable codes

### 18.1 Error codes

| Code | Meaning and effect |
| --- | --- |
| `STC-E-DEBUG-001` | Debug service unavailable or disabled: no capable module, no server, or debug support disabled by policy. Session cannot start. |
| `STC-E-DEBUG-002` | Unsupported protocol version or wrong protocol/endpoint (for example a control endpoint). The connection may retry once with `stc-debug/1`. |
| `STC-E-DEBUG-003` | Malformed or invalid request: unknown key/op/field/scope/kind, bad parameter type, duplicate/decreasing/out-of-range `id`, raw-address field. Request rejected; repeated malformed frames close the connection. |
| `STC-E-DEBUG-004` | Oversized or unterminated frame. Connection closes. |
| `STC-E-DEBUG-005` | Authentication failed: missing/malformed/wrong token. Connection closes; repeated failures are limited and audited. |
| `STC-E-DEBUG-006` | Authorization denied: scope not granted or host policy denied a write/force/protected read. Operation rejected and audited. |
| `STC-E-DEBUG-007` | Invalid session state: no session, session already active, request before `session.start`, or an operation that requires a session. |
| `STC-E-DEBUG-008` | Unknown target: path, `GotId`, frame, local, checkpoint, breakpoint, watch, or `stop_id` does not exist. |
| `STC-E-DEBUG-009` | Invalid target or value: wrong type/shape, not writable, out of range, invalid enum or code point, capacity overflow, unsupported encoding. |
| `STC-E-DEBUG-010` | Stale generation/layout: expected generation/build/type/layout differs, target removed, layout changed, or the generation changed during a read. Re-resolve and retry. |
| `STC-E-DEBUG-011` | Limit exceeded: queue, breakpoints, watches, aggregate leaves, response size, or backpressure. |
| `STC-E-DEBUG-012` | Request or watch rate limit exceeded. Request rejected; repeated rate errors may close the connection. |
| `STC-E-DEBUG-013` | Request timeout. The operation may or may not have applied; the idempotency and reconciliation rules in §13.5 apply. |
| `STC-E-DEBUG-014` | Breakpoint/step/pause unavailable: the artifact has no debug hooks or no frame/checkpoint information (release or hook-free build). Monitoring remains available. |
| `STC-E-DEBUG-015` | Invalid step/continue: not stopped, stale `stop_id`, unsatisfiable step, or invalid step mode. |
| `STC-E-DEBUG-016` | Invalid debug metadata: missing/malformed/non-dense checkpoint table, build-ID/span/function mismatch, reserved flags set, debug-object/descriptor hook disagreement, or table too large. Breakpoints/steps are refused; the module keeps running. |
| `STC-E-DEBUG-017` | Debug information unavailable for the requested source, frame, or local (stripped, optimized out, or absent). Reported honestly; values are not reconstructed. |
| `STC-E-DEBUG-018` | Session timeout: the stop or idle bound was exceeded. The session ends, forces/breakpoints/queued writes are cleared, and the cycle resumes with a fresh deadline. |
| `STC-E-DEBUG-019` | Session ended by an online swap. Reconnect and re-resolve against the new generation; no state migrates. |
| `STC-E-DEBUG-020` | Transport/endpoint failure: bind/listen/path/permission/symlink/unsupported transport. The server refuses to start or the connection closes. |
| `STC-E-DEBUG-021` | Audit unavailable: the audit log cannot be created, secured, or appended. Server start or the protected/modify operation is refused (fail closed). |
| `STC-E-DEBUG-022` | Internal debug service error. The session ends if framing or session state is untrusted; otherwise the request is rejected. |

### 18.2 Warning codes

| Code | Meaning and reporting |
| --- | --- |
| `STC-W-DEBUG-001` | Breakpoint is unbound (no checkpoint at the source location). Reported in `breakpoint.set`/`breakpoint.list`; the breakpoint is not moved. |
| `STC-W-DEBUG-002` | Frame, parameter, or local is optimized out or unavailable. Reported per frame/value; the value is not invented. |
| `STC-W-DEBUG-003` | Watch batch was coalesced or dropped because of rate/backpressure. The next batch carries the warning. |
| `STC-W-DEBUG-004` | Debug information or hooks are unavailable; the session is monitoring-only (release or hook-free build). Reported at `session.start` and on breakpoint/step attempts. |
| `STC-W-DEBUG-005` | A force targets a `RETAIN`/`PERSISTENT` root; normal shutdown may save the resulting value even though the force table is not persisted. Reported when the force is created. |

### 18.3 Error semantics and stability

- Codes are stable and are never reused for a different meaning. A new condition gets the next
  unused number in its family.
- A protocol error response uses the code in `error.code`; a warning is carried in `warnings` and
  does not change the response status. A session-ending condition uses the code in
  `session-ended.data.code` when a code is defined (018, 019, 022).
- Build-time debug-setting errors use `STC-E-PROJECT-010` (mutually exclusive flags or a
  `[build].debug`/`[debug].default` disagreement); descriptor inconsistencies use
  `STC-E-MODULE-002`; a transport failure at launch may additionally use an existing
  `STC-E-IO-*` code. The DEBUG family remains the protocol/session authority.
- A rejected queued write/force is reported in `queue-result` with the rejection code, not as a
  response error, because the original request was accepted.
- The stable diagnostic index and cross-references are in
  [Conformance Profile](../01-Conformance-Profile.md) §10.1.

### 18.4 Non-safety statement

The debug service, its pause bound, its authentication, and its audit log are availability and
operability features, not safety functions. A stopped or forced controller may hold outputs
indefinitely. `stc` can control real machinery, but it is not a safety PLC and is not responsible
for safety; safety must be provided by a separate safety system and machine design as required by
law and regulation. Deployments that require fail-safe behavior must implement it independently
and must not rely on `stc`.
