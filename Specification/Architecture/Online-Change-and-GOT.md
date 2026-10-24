# Online Change and GOT

Status: Resolved specification (2026-10-24)  
Profile: `stc-2013-oop-utf8-v1`  
Scope: v1 runtime/platform capability; not an IEC 61131-3 language feature.

This document specifies the host/module execution model, the global offset table (GOT), persistent
memory, the module descriptor, the persistent GOT manifest, and the online-change protocol. It
is the normative companion to
[Runtime and Execution](Runtime-and-Execution.md),
[State and RETAIN](State-and-Retain.md),
[I/O and Devices](IO-and-Devices.md), and
[Debug and Monitoring](Debug-and-Monitoring.md). The resolved planning rationale and the full
question disposition are in `update_plan_got.md`.

## 1. Terms

| Term | Meaning |
| --- | --- |
| Host | The persistent runtime process that owns the cycle loop, GOT, memory, modules, state, I/O, and services |
| Module | A loadable shared library containing one whole generated program version |
| Generation | One loaded module version and its per-generation GOT slot table |
| GOT | Host-owned table of stable cells; generated code reads/writes persistent state through it |
| Cell | A stable host-owned pointer-sized slot holding a storage pointer, function address, or descriptor address |
| Root | A host-owned persistent storage allocation tracked by identity |
| `GotId` | A persisted 128-bit identity for a root, function, type, instance, task, or export |
| Manifest | The persistent identity/layout/lineage artifact paired with a module |
| Activation | The O(1) cycle-boundary switch from the current generation to the prepared generation |
| Cold download | Stop/reinitialize the program and install a new module without preserving running values |
| Online change | Install a compatible new module while preserving persistent state across a cycle boundary |

## 2. Execution model

### 2.1 Host and module split

The v1 deployment model is a persistent host plus a loadable whole-program module. The host is a
long-lived process. A module is a platform shared library (`.dll` on Windows, `.so` on Linux)
loaded by the host. Generated code never owns persistent storage and never loads another module.

The host owns:

- the cycle loop and cycle clock;
- the GOT cell table and root allocator;
- the process image and retained-state buffer;
- module loading, descriptors, manifests, and the active baseline;
- task and configuration state;
- the export registry;
- diagnostics, counters, and service/monitor threads;
- the debug/monitoring protocol.

The module owns only code and version-local read-only data. It may allocate transient memory
through the host service; that memory is freed when the version fails, retires, or unloads.

### 2.2 Thread classes

| Thread | Responsibility |
| --- | --- |
| Cycle | Latches inputs, executes ST, publishes outputs; the only thread that executes ST or switches generations |
| Loader/prepare | Loads the candidate module, validates it, prepares roots/state, and builds the temp GOT |
| Service/monitor | Typed variable access, debug/monitoring, status, diagnostics, and manifest export at lower priority |

The loader and service threads never block the cycle with unbounded locks. They use generation
pinning/epoch refcounts so a module cannot unload while a service reader holds a descriptor.
Status publication uses lock-free counters, double-buffered snapshots, and bounded rings.

### 2.3 Artifacts

| Artifact | Contents |
| --- | --- |
| `module` | Loadable code, module descriptor, entry points, embedded metadata, no persistent storage |
| `manifest` | Canonical JSON (`.stc-got.json`) plus compact binary (`.stc-got.bin`), generated from one model |
| `exe` | Thin host launcher that loads an adjacent module and manifest |
| `lib` | Embedding host library; generated code remains in the separate module (M4.5: static host archive) |

A module and its manifest are a pair. The module descriptor is a small binding structure for code
addresses, sizes, entry points, and the paired digest. The manifest is the identity/history
artifact. Both are projections of one compiler model; the manifest is not the sole source of code
addresses.

## 3. Global offset table

### 3.1 Scope

Every access that can outlive or cross a module version is GOT-indirected:

- persistent roots (including per-field roots);
- user and standard function calls;
- stored function pointers;
- vtable entries and type/object descriptors;
- direct `AT` variable cells (one per variable or field, per generation).

Stack slots, locals, parameters, compiler helpers, intrinsics, and module-owned read-only data are
direct. A module must never store a pointer into its own code or read-only data in a persistent
root.

### 3.2 Cell kinds

| Cell kind | Holds | Written by |
| --- | --- | --- |
| Data | Pointer to host-owned root storage | Host |
| Function | Address of a function implementation in the active module | Host |
| Descriptor | Address of a runtime-owned type/object descriptor or vtable | Host |
| Direct variable | Pointer to the process-image location for an `AT` variable/field | Host |
| Import/helper | Address of a host service entry point | Host |

Generated code reads cells but never writes them. Only the host writes cells under the control
lock during prepare/activation/rollback.

### 3.3 Slot tables

Each module has a per-generation slot table mapping local slot numbers to host cells. Slot numbers
are local and are not stable across builds; `GotId` is the persistent identity. The compiler emits
sorted slot tables so double builds are byte-identical. Generated code may cache the slot-table
pointer and a root storage pointer within a function, but must not cache a cell value across a
cycle boundary.

### 3.4 Host-only writes and allocation

The host provides a narrow internal allocation service (alloc/free) for transient module needs.
It returns stable, zeroed, host-owned blocks. There is no module-requested rebinding of a GOT
cell. User-visible `__NEW`/`__DELETE` is a v2 vendor extension with a separate design.

## 4. Persistent memory

### 4.1 Root allocation

The host allocates persistent roots at load/prepare time. Storage is stable and never moves.
Removed roots are quarantined until restart or explicit reset; their storage is not freed while
any preserved value may refer to it.

### 4.2 Per-field roots and objects

Persistent aggregates are rooted recursively per field. Every persistent field has its own
`GotId`, cell, storage, type identity, and manifest record. A persistent object (program, FB, or
class instance) is represented by a stable opaque handle plus a runtime object record and
descriptor. `THIS`, `SUPER`, interfaces, and object references carry the handle, not a contiguous
struct pointer.

- **Arrays:** arrays of scalars/strings are one root with GEP indexing; arrays of aggregates use
  per-element handles whose fields are rooted recursively.
- **Strings:** one root per string value, with length and data internal to the root; no
  per-character roots.
- **`OVERLAP`:** one atomic root with aliasing typed views; per-field online change of overlap
  internals requires reset or cold.
- **Value assignment:** per-field recursive copy into destination roots, preserving destination
  object identity; handles are never copied as storage pointers.

### 4.3 Identity and keys

The current key is the normalized qualified path/signature used to find a declaration in source.
The `GotId` is the persistent identity recorded in the manifest and matched by the runtime.
Tracked renames and moves change the current key while retaining the `GotId`. New declarations
get new IDs; removed entries retire; a reintroduced retired key gets a new ID unless explicitly
reused. Data/type IDs exclude the current type/layout so layout changes remain trackable;
function IDs include the signature.

## 5. Module lifecycle

### 5.1 States

| State | Meaning |
| --- | --- |
| Loaded | Platform loader mapped the module; descriptor not yet trusted |
| Validated | Descriptor, ABI, target, metadata, and manifest pairing passed |
| Attached | Module received the host API table and its GOT context |
| Prepared | Candidate roots/state prepared on the loader thread; temp GOT built |
| Active | Module is the current generation at a cycle boundary |
| Retired | Replaced, inside the rollback/grace window |
| Unloaded | Platform unload completed off-thread |

### 5.2 Entry points

A module exports attach, prepare, cycle, shutdown, and detach entry points plus its descriptor.
The exact names and signatures are part of host ABI 2. A module that fails validation is never
attached. Load failures and rejected modules are non-fatal to the host.

### 5.3 Prepare

The loader thread loads the candidate, validates it, checks the manifest baseline, allocates new
roots, copies preserved values under the preservation rules, builds the temp GOT, and records the
candidate plan. Prepared code may run initialization logic on the loader thread with its own temp
GOT context, but must not publish outputs or mutate the active generation.

## 6. Activation, commit, rollback, unload

### 6.1 Double-buffered GOT

The host maintains a normal GOT and a temp GOT. The temp GOT contains:

- shared preserved-root cells (same storage as the normal GOT);
- new-root cells for the candidate;
- all function cells bound to the candidate module;
- descriptor/vtable cells for the candidate.

The active generation points at one GOT. The switch is an O(1) pointer swap at a cycle boundary.

### 6.2 Boundary rule

Activation occurs after the old generation completes its cycle and publishes outputs, before the
next input latch. The old version must complete whole cycles only. The new version starts at the
next cycle start with fresh inputs. The clock is continuous across the swap.

### 6.3 Commit and rollback

The previous generation is retained for one successful cycle. If the new generation traps or
fails during that cycle, the host switches back to the previous GOT and module, records the
failure, and keeps the old generation active. A successful cycle commits the new generation;
rollback after commit is another online change or a cold download. The switch and rollback are
bounded; validation and copying are off-thread.

### 6.4 Unload

After commit and a grace period that covers in-flight service readers and debug pins, the host
unloads the retired module off-thread. Cells remain host-owned. A module is never unloaded while
a generation-pinned reader holds a reference to it.

## 7. Persistent GOT manifest

### 7.1 Format

The manifest has two projections generated from one model:

- `.stc-got.json` — canonical JSON for tooling, download, inspection, and cross-developer use;
- `.stc-got.bin` — compact binary for runtime loading.

Both carry a format version and integrity digest. The runtime caches the parsed active record and
does not reparse on every cycle. Artifact size and load cost are measured on a representative
large program; there is no hard size cap in v1.

### 7.2 Content

- header: format version, profile, target triple, ABI version, build ID, parent build ID/digest;
- identity records: `GotId`, current key, kind, scope, owner, signature/type/layout references;
- layout records: per-field paths, offsets, sizes, type keys, array/string/`OVERLAP` shape;
- function/method/FB-body/standard-FB records;
- type/descriptor/vtable records;
- instance/task/export records;
- current slot mappings and cell kinds;
- online-change plan and compatibility classification;
- quarantine/retirement history;
- integrity digests.

The manifest contains no runtime values. Values remain in host memory and the retained-state
file.

### 7.3 Identity and history

`GotId`s are content-derived 128-bit identities from scope/name/kind, with collision checking.
The manifest records lineage and tracked renames/moves. A build without the active baseline, or
with a baseline that is not exactly current, is a cold-layout build and the runtime requires a
cold download.

### 7.4 Storage, download, and security

The runtime stores the active manifest in its state directory and beside the installed module.
Immutable versioned module files carry a manifest sidecar; a current pointer selects the active
pair. The active copy is authoritative. A downloaded manifest is untrusted parsed data: validate
every field and digest, access-control the export because layout data reveals program structure,
and never execute manifest content. V1 download is local host API/CLI file export; there is no
network deployment protocol.

## 8. Compatibility and preservation

### 8.1 Preservation rules

A root is preserved when its `GotId` is present in the active manifest and its type key, layout
hash, size, and target match. New roots are initialized exactly once. Removed roots are
quarantined and recorded as retired. Initializers apply only to new roots; changed initializers
on preserved roots are ignored.

### 8.2 Changes in v1

| Change | Result |
| --- | --- |
| Code-only change | Online change |
| Field add/remove/reorder | Per-field preserve/drop |
| Tracked field/namespace/package rename or move | Preserve under the old `GotId` |
| Untracked field removal/rename | Diagnostic; explicit allow or cold |
| Field type change, string capacity, array/`OVERLAP` layout | Reset affected root or cold; no field-layout migration |
| Task/resource topology change | Cold download |
| Profile/target/ABI change | Cold download |
| Missing/invalid/unpaired/stale baseline | Cold download |

### 8.3 OOP and interfaces

Type descriptors and vtables are runtime-owned. Vtable cells contain cell pointers, not raw code.
Preserved interfaces survive a compatible swap because the descriptor is stable. A vtable shape
change is rejected while any live instance or interface depends on it. `REF_TO` values use place
descriptors and are remapped by identity/offset; `Stack` descriptors never outlive the call.

### 8.4 State and cold download

Online attach skips the retained-state file load because in-memory state wins. The state file
keeps its existing layout-hash policy for restarts. A cold download stops/reinitializes the
program: non-retained state takes defaults, `RETAIN` values are restored only under the existing
compatibility policy, and transient allocations are not preserved. V1 does not migrate retained
state by `GotId`; a later state format may do so.

## 9. References and `POINTER TO`

Persistent-capable `REF_TO` values are place descriptors:

```text
Null
Root   { got_id, offset, type_key }
Direct { direct_id | area, offset, type_key }
Stack  { frame_id, offset, type_key }
Object { handle, descriptor }
```

`REF(place)` produces the same descriptor directly. Pointer→`REF_TO` conversion is validated by
the host resolver against active roots/process-image ranges and manifest layout descriptors; an
invalid conversion yields `NULL`, and dereferencing `NULL` traps. `REF_TO`→interface is allowed
only for class/FB object references with a descriptor check. Interfaces are nullable.

`POINTER TO` is a distinct transient-only `STC-POINTER` extension: full typed arithmetic,
indexing, `SIZEOF` for contiguous values, pointer parameters/results, null-dereference traps, and
documented dangling behavior. Persistent pointer fields are rejected. Pointer→object/interface
and pointer↔integer conversions are not allowed.

## 10. Threads, services, and diagnostics

Service/monitor threads use typed path/`GotId` access, not raw memory. Reads are generation-pinned
and do not lock the cycle. Writes and forcing are queued for cycle-boundary application. A write
queued across a swap is rejected or retried against the new generation; it must not target a
removed field.

The host exposes active/retired build IDs, catalog summary, compatibility state, quarantine list,
and the last rejection. Diagnostics use stable `MODULE`, `MANIFEST`, `ABI`, and `ONLINE` families.
Online-change load-time diagnostics are primary; source warnings are added only for constructs
with no safe runtime rule.

## 11. ABI and versioning

Host ABI 2 is a versioned struct (`abi_version` plus `struct_size`, additive within a major).
M4 code-linked artifacts and ABI 1 are not loadable. The module descriptor, manifest format, and
runtime state layout are versioned together. A failed compatibility check rejects the candidate
before activation.

## 12. Definition of done

- A code-only change, field add/remove/rename, and tracked move deploy online on Windows and
  Linux with preserved compatible state.
- An incompatible change is rejected or requires an explicit reset; a missing/stale/corrupt
  baseline forces a tested cold download.
- The cycle-boundary switch is O(1); prepare, copy, and unload are off-thread; jitter is bounded.
- Generated code contains no persistent raw pointers and no module-owned persistent storage.
- Double builds produce byte-identical modules and manifests; the active manifest can be
  downloaded and used as the next build baseline.
- Hot-swap, manifest, cold-download, rollback, and descriptor tests run in CI on both hosts.

## 13. Non-goals and safety

- No per-function hot patching, ancestor/partial-baseline matching, merge, or rebase.
- No field-layout migration, resizing, or pointer translation in v1.
- No network deployment protocol, multi-user scheduling, or remote UI in v1.
- No safety claim. `stc` is not a safety PLC and is not responsible for safety in any way.
  Debug sessions may hold outputs indefinitely; no automatic fail-safe is provided.

## 14. Related documents

- [Runtime and Execution](Runtime-and-Execution.md) — host loop, ABI, services.
- [State and RETAIN](State-and-Retain.md) — state file and cold/warm policy.
- [I/O and Devices](IO-and-Devices.md) — process image and direct variables.
- [CLI and Manifest](CLI-and-Manifest.md) — commands, options, and project tables.
- [Debug and Monitoring](Debug-and-Monitoring.md) — typed access and debug guarantees.
- [Testing and Verification](Testing-and-Verification.md) — hot-swap and manifest test levels.
