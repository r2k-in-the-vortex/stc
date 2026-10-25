# Online Change and GOT Architecture — Update Plan

Status: Resolved; documentation applied (revision 36)  
Date: 2026-10-24  
Profile: `stc-2013-oop-utf8-v1`  
Scope: planning only. This document proposes no compiler or runtime code changes and does
not change any completed milestone.

Revision 2 adds the persistent GOT manifest, compiler-driven rename/migration planning, the
simplification analysis in §14, and questions Q-096–Q-116. Revision 3 records the accepted
fail-safe policy: a missing, invalid, unpaired, or non-current manifest forces a cold download
rather than an online-change recovery attempt (Q-117–Q-119). Revision 4 records Q-032: host-only
GOT writes plus an internal host allocation/rebinding service; user-visible `__NEW`/`__DELETE`
is a v2 extension (Q-120–Q-122). Revision 5 records the reviewed answers: Q-018, Q-019, Q-020,
Q-026, Q-030, Q-031, Q-032, Q-035, and Q-120 resolved at their recommended defaults; Q-121
deferred pending the later reference-based review. Revision 6 records Q-036/Q-037/Q-038, Q-096,
Q-097/Q-098/Q-099, Q-101, Q-108, and Q-117 as resolved; Q-049 and the M6 portion of Q-110 are
deferred until the pre-M4.5 revisit. Revision 7 records the non-default Q-042 decision (recursive
per-field roots with handle-based objects) and its resolved edges Q-123–Q-126; Q-108 was reopened
and Q-127/Q-130 remain open. Revision 8 records the remaining P0 answers (Q-027, Q-046–Q-048,
Q-054, Q-056, Q-057, Q-064, Q-070–Q-072, Q-084, Q-086, Q-089–Q-091, Q-095, Q-127). Q-091 pauses
implementation and Q-095 requires all P0s, including deferred ones, before code. Revision 9
closes Q-128–Q-130; all non-deferred P0s are now resolved. The remaining P0 blockers are Q-049,
the M6 portion of Q-110, and Q-121. Revision 10 adds §6.14, which defines the three-scope
persistent-reference policy and the `REF_TO` vs `REFERENCE TO`/`POINTER TO` scope (Q-131–Q-134).
Revision 11 keeps Q-131 under discussion, adds the §6.15 `POINTER TO` analysis, and adds
Q-135–Q-142 to decide its v1 scope. Revision 12 resolves Q-131/Q-135–Q-138: `POINTER TO` is a
distinct transient-only type with full arithmetic/indexing/SIZEOF and no persistent pointer
fields; Q-143–Q-150 define the safety envelope. Revision 13 closes Q-143–Q-150: contiguous
targets only, typed unchecked arithmetic, no persistent pointers, no integer casts, M5/profile
`STC-POINTER`; Q-151–Q-169 define pointer→reference runtime validation, reference→interface
conversion, and the place-descriptor reference model. Revision 17 resolves Q-162–Q-165: uniform
descriptors for persistent-capable REF_TO, Stack descriptors for call-duration locals,
DirectId-based AT references, and Object{handle, descriptor} references. Revision 18 resolves
Q-159/Q-160/Q-167/Q-168: exact type matching, persistent pointer-derived references under
descriptor rules, no state-file reference persistence in v1, and identity/offset remapping.
Revision 19 adds §6.18 and Q-170–Q-173 for nullability and REF_TO→interface conversion.
Revision 20 resolves them: interfaces are nullable, null/incompatible conversions yield NULL,
no mandatory `__ISVALIDREF`, and pointer→object/interface is disallowed. Revision 21 adds §6.19
and Q-174–Q-182 for double-buffered GOT activation and the real-time reload boundary. Revision 22
resolves Q-174–Q-177: normal/temp GOT generations, background loader thread, whole-module
function binding, and shared preserved-root storage. Revision 23 resolves Q-178–Q-181:
loader-thread prepare with a temp GOT context, background copy-back, one-successful-cycle
rollback window, and off-thread unload after grace. Revision 24 resolves Q-182: bounded
maintenance jitter with an O(1) cycle-boundary switch and all heavy work off-thread.
Revision 25 adds §6.20 and Q-183–Q-190 for non-RT service/monitor threads, typed variable/debug
access, diagnostics/status, and generation pinning. Revision 26 records the non-default Q-183/
Q-186 decision to put full variable and debug access in v1, flags the non-goal conflict, and adds
Q-191–Q-200 for the debug scope/mechanism. Revision 27 resolves Q-191–Q-194: full debug surface,
codegen checkpoint breakpoints, stop-the-world stepping, and boundary-applied forcing.
Revision 28 records the non-default hold-last-outputs and full-DWARF/CodeView decisions and adds
Q-201–Q-206 for watchdog/session safety, debug-info emission, split-object representation, and
debug info retention. Revision 29 records the non-default Q-201/Q-202 decisions: the project is
not a safety PLC and debug holds outputs indefinitely; debug info is the default build behavior
with `--release` as the optional stripped/optimized mode. Revision 30 records Q-199's
non-default "end the debug session on every online swap" and adds Q-211 for force/breakpoint
teardown at session end. Revision 31 resolves Q-204/Q-205/Q-210/Q-211: target-derived
DWARF/CodeView, reproducible debug info with sidecar symbols, M6.5 debug milestone, and session-end
teardown. Revision 32 closes the last P0s: no field migration in v1, transient internal
allocations, active-generation debug info, epoch pinning, host API versioning, and the resolver
details. All P0 questions are resolved; only P1/P2 remain. Revision 33 resolves the first P1
batches: delivery/artifacts/hosts/profile, read-only data/helpers, per-variable direct-variable
cells (non-default Q-033) with per-generation remapping Q-212, function registration, keys, and
removed-reference quarantine. Revision 34 syncs the P1 answers through Q-119; only Q-188–Q-190
and the P2 set remain open. Revision 35 resolves the final P1/P2 batches: non-RT status/API/
isolation (Q-188–Q-190), direct-call performance (Q-062), `THIS`/`SUPER` (Q-063), per-task state
(Q-077), load-time diagnostics (Q-082), host introspection (Q-083), and the manifest P2 items
(Q-105, Q-113, Q-114, Q-116). All questions Q-018–Q-212 are now resolved; §16 lists the
documentation and planning work that follows. Revision 36 applies the §12 documentation
checklist: decisions A-022–A-030 and D-094–D-108 are recorded, the project plan, conformance
profile, architecture/runtime/state/I/O/CLI/testing/AST documents are updated, the new
online-change and debug specifications are added, and `M4.5-Plan.md` / `M6.5-Plan.md` are
written. No compiler or runtime code has changed.

Read this together with:

- [Project Plan](Specification/00-Project-Plan.md) §3–§11
- [Conformance Profile](Specification/01-Conformance-Profile.md)
- [Architecture](Specification/Architecture/Architecture.md) §4–§8
- [Runtime and Execution](Specification/Architecture/Runtime-and-Execution.md) §1–§4
- [State and RETAIN](Specification/Architecture/State-and-Retain.md) §2–§6
- [I/O and Devices](Specification/Architecture/IO-and-Devices.md) §2–§6
- [CLI and Manifest](Specification/Architecture/CLI-and-Manifest.md) §1–§4
- [Testing and Verification](Specification/Architecture/Testing-and-Verification.md) §1–§6
- [AST/AST](Specification/AST/AST.md) §5–§8
- [Decisions](Specification/Decisions.md) D-076–D-093
- [M4-Plan.md](M4-Plan.md)
- [handover_to_M5.md](handover_to_M5.md)

## 1. Why this plan exists

M4 completed the first code-generating path: `stc-codegen` lowers the scalar HIR subset to
textual LLVM IR, and `stc build` links that IR with the Rust runtime into an executable or a
static library. The generated module owns its globals, its program instances, its process-image
pointer, and its retained-state buffer. Every call to a user function is a direct LLVM call, and
every global access is a direct `@symbol` reference. The code and the data it uses are welded
into one artifact for the lifetime of the process.

A PLC must support **online change**: an operator downloads a new program version while the
controller is running, and the new code starts executing at a scan-cycle boundary without
stopping the controller or losing state. The project now treats this as a platform requirement,
not a later convenience. The architectural implications are:

1. **Persistent data must outlive any one code version.** All persistent memory — globals,
   program instances, FB/class instance state, process-image references, retained buffers, and
   runtime bookkeeping — must be owned by the runtime, not by the compiler-generated module.
2. **Code must be replaceable.** The compiler must emit a loadable code module. The runtime must
   load a module, validate it, attach it to existing storage, switch execution at a cycle
   boundary, and unload the previous module safely.
3. **All persistent references must be indirect.** Generated code must access every persistent
   data root and every callable entry point through a **global offset table (GOT)** so that a
   new module can be bound to the same storage and callable slots without patching old code.
4. **Compatibility must be explicit.** A new module may match, add, remove, or change roots,
   functions, types, vtables, tasks, and exports. The runtime must accept, preserve, initialize,
   reset, or reject each case deterministically, and must not leave the controller in a
   half-updated state.
5. **Identity must be persistent and portable.** Source paths, arena IDs, and mangled symbols are
   not enough to survive renames, moves, and cross-developer builds. The compiler must emit and
   consume a persistent GOT manifest that records the identity, layout, and history of every
   tracked instance, and the runtime must store that manifest and make it available as a build
   baseline.

The build artifact in item 5 is the main simplification opportunity examined in §14. It moves
type/layout comparison, rename tracking, and migration planning into the compiler, where that
knowledge already exists, and turns the runtime into a validator/executor of an explicit plan.
It also enables a workflow that runtime-only matching cannot provide: a developer can download
the active manifest from the PLC and build the next version against the running layout.

This plan is the impact analysis and migration plan for those changes. It reviews the completed
M1–M4 code, the planned M5–M8 work, and the specification set. It ends with a numbered question
backlog. No implementation starts until the blocking questions are resolved.

## 2. Recommended direction at a glance

| Area | Recommendation |
| --- | --- |
| Execution model | Persistent runtime host process plus loadable code modules (shared libraries) |
| Artifacts | Add `--emit=module`; each module is paired with a persistent GOT manifest; `exe` becomes a host launcher that loads an adjacent module; `lib` remains the embedding form |
| GOT location | Runtime-owned stable cells; each module receives a slot table of cell pointers at attach |
| GOT contents | Every persistent root, every user/standard function call, every stored function pointer, every vtable/type descriptor; stack, locals, compiler helpers, intrinsics, and module-owned read-only data stay direct |
| Memory ownership | Runtime allocates each persistent root; storage never moves; removed roots are quarantined, not freed, in v1 |
| Root granularity | Recursive per-field roots with handle-based objects; scalar/string arrays are one root; aggregate arrays use per-element handles; `OVERLAP` is atomic |
| Stable identity | Persistent `GotId` recorded in a compiler-emitted GOT manifest; the current qualified path/signature key locates the identity; type identity from qualified name plus structural layout hash |
| Build history | The compiler emits a versioned **GOT manifest** of every tracked root, function, type, instance, task, and export. The next build consumes it to preserve identities, track renames, and emit an online-change plan |
| Preservation | All matching roots are preserved, not only `RETAIN`; initializers run only for new roots; compiler-generated migration plans handle approved structural changes |
| Compatibility | Code-only and additive changes are online-compatible; exact-layout changes preserve state; declared migration operations handle approved field/type changes; everything else is rejected or requires a cold download |
| Runtime handoff | The active manifest is stored by the host, passed with the module, and downloadable so another developer can build the next version against the running layout |
| Cold failure mode | A missing, invalid, unpaired, or non-current manifest selects cold download; no online-change recovery, merge, or best-effort rebase |
| OOP | Runtime-owned type descriptors and vtables built from cell pointers, never raw code pointers in persistent memory |
| Runtime services | Host API table passed at attach; modules import no `stc_rt_*` symbols |
| Swap point | Between complete cycles, after output publication, before the next input latch |
| Failure handling | All-or-nothing prepare; old module stays active on failure; rollback keeps the old cell snapshot |
| Sequencing | Insert **M4.5 — Online-change foundation** before M5; do not build M5 features on the direct-reference model |
| Compatibility mode | Do not keep a second non-GOT codegen mode; one tested path |

If the project rejects online change as a v1 requirement, this plan should be closed and the
project can proceed to M5 unchanged. If it accepts online change, the recommended sequencing in
§9 applies: the M4 scalar surface should be converted before M5 adds FBs, OOP, aggregates, and
strings, because those features would otherwise be implemented twice.

## 3. Terminology

| Term | Meaning |
| --- | --- |
| **Host** | The long-lived runtime process or embedding library. It owns the cycle loop, clock, I/O, state files, memory registry, GOT, and module loader. |
| **Module** | A compiler-generated loadable shared library containing one program version: code, descriptors, read-only data, and module-lifetime binding metadata. A module has no persistent program state of its own. |
| **Version / build** | One module produced by one compiler invocation, identified by a content-derived build ID. |
| **Data root** | A persistent storage object allocated by the host: a global, a program instance, or another top-level persistent instance. Nested aggregate fields live inside their root. |
| **GOT cell** | A host-owned, stable-address pointer slot whose value is either a data-root storage pointer or a function entry pointer. |
| **Slot table** | The module-local array of GOT cell pointers supplied by the host at attach, indexed by compiler-assigned slot numbers. |
| **Catalog / descriptor** | The module-exported tables describing roots, functions, types, vtables, instances, tasks, and exports, with stable keys and layout hashes. |
| **Attach** | The host-to-module call that supplies the host API table and the slot table before any generated code runs. |
| **Prepare** | The pre-activation phase: allocate/bind new roots, initialize new roots, and validate runtime descriptors. Prepare runs with the cycle stopped, does not call through shared function cells, and does not replace any function cell. |
| **Activate / swap** | The atomic commit that makes the new module the active version for the next cycle. |
| **Online change** | A swap that preserves compatible state and continues execution without a cold restart. |
| **Cold download** | A stop-and-reload: state is initialized from defaults and/or the retained-state file, not preserved from the running version. |
| **Quarantine** | Retaining the storage of a removed root so existing references remain memory-safe, without exposing the root to the new module. Reclaimed only by host restart or an explicit reset. |
| **GotId** | The persistent identity of a GOT-tracked instance. It is recorded in the GOT manifest, survives renames and moves, and is what the runtime matches across builds. It is not an arena ID, a mangled symbol, or a source path. |
| **GOT manifest** | The compiler-emitted, versioned build artifact that lists every GOT-tracked instance with its `GotId`, current key, history, type/layout identity, slot kind, and migration information. It is both a build input for the next version and the runtime's active layout record. |
| **Runtime layout record** | The host-side record of active `GotId`s, cells, allocated storage, states, and build lineage. It is derived from the manifest plus runtime allocation and is what the host persists and exposes for download. |
| **Tracked rename / move** | An explicit compiler operation or rename map that keeps a `GotId` while changing the declaration's current key. Without tracking, a rename looks like a remove plus an add and loses state. |
| **Online-change plan** | The compiler-generated description of how a new module relates to a baseline manifest: preserve, allocate, migrate, reset, or drop each `GotId`, with compatibility diagnostics. The host validates and executes it. |
| **Baseline artifact** | A GOT manifest (usually the active manifest downloaded from the PLC) used as the input to the next build so identities and layout compatibility are computed against the running system. |
| **Compatibility class** | The classification of a change as seamless, additive, migration-required, reset-required, or rejected. |

## 4. Requirements and non-goals

### 4.1 Functional requirements

| ID | Requirement |
| --- | --- |
| OC-R-001 | A new module can be loaded and validated while the current version continues to run; binding, preparation, and activation occupy a bounded boundary window with no cycle in flight. |
| OC-R-002 | Activation happens only between complete scan cycles, never in the middle of a cycle. |
| OC-R-003 | Compatible persistent roots keep their values and storage addresses across a swap. |
| OC-R-004 | Every persistent data access and every stored function pointer is reached through the GOT. |
| OC-R-005 | The runtime owns allocation, lifetime, and address stability of all persistent roots. |
| OC-R-006 | A new module is accepted or rejected atomically. A rejected module leaves the old version running. |
| OC-R-007 | The old module can be unloaded after the swap without invalidating host-held data or call targets. |
| OC-R-008 | Module IR, descriptors, slot assignment, and build IDs are deterministic and reproducible. |
| OC-R-009 | The host and module path work on Windows MSVC x86-64 and Linux GNU x86-64; Linux AArch64 follows with M8. |
| OC-R-010 | Unsupported or incompatible changes produce explicit diagnostics naming the affected root, function, type, or task. |
| OC-R-011 | The same host API supports the standalone host binary and library embedding. |
| OC-R-012 | Online change is testable from two independently built modules and a deterministic cycle harness. |
| OC-R-013 | Indirect access is measured; performance effects are documented rather than assumed. |
| OC-R-014 | The Structured Text language surface and semantic rules do not change. |
| OC-R-015 | The feature is documented as a runtime/platform capability, not as an IEC 61131-3 conformance claim. |
| OC-R-016 | The compiler emits a versioned GOT manifest covering every GOT-tracked root, function, type, instance, task, and export. |
| OC-R-017 | A later build can consume a baseline manifest and preserve `GotId`s for unchanged declarations without relying on source order or arena IDs. |
| OC-R-018 | The compiler can perform explicit tracked rename/move operations that keep a `GotId` across a key change. |
| OC-R-019 | The manifest is delivered with the module to the runtime, stored by the host, and exportable for another developer to use as a build baseline. |
| OC-R-020 | The compiler can emit an online-change plan against a baseline manifest, including compatibility diagnostics and permitted migration operations; the host validates and executes it. |
| OC-R-021 | Manifest and module are paired by content digest; a stale, corrupted, or mismatched manifest is rejected rather than silently applied. |
| OC-R-022 | A cold build without a baseline still emits a complete manifest, so the runtime never operates an unmanifested layout. |
| OC-R-023 | A missing, invalid, unpaired, or non-current baseline manifest forces a cold download; the runtime does not attempt an online-change recovery, merge, or best-effort rebase. |
| OC-R-024 | A cold download may restore retained state only according to the existing state-file compatibility policy; it never preserves non-retained running values by accident. |

### 4.2 Proposed non-goals for v1

- Mid-cycle, per-statement, or per-task-subset swapping.
- Hot-patching individual functions without replacing a module.
- Concurrent execution of two program versions.
- Arbitrary field-by-field data migration when a type layout changes. V1 may support explicitly
  scoped compiler-generated migration operations, but not arbitrary layout rewriting.
- Heuristic rename detection. Renames and moves are tracked only when the compiler is told.
- Putting runtime values into the GOT manifest. The manifest carries identities, layout, and
  migration information; values stay in host memory and the retained-state file.
- Hidden build state. A build must not silently depend on a manifest the user did not supply or
  whose digest is not recorded.
- Automatic recovery, merging, or rebasing from a missing, stale, or incompatible baseline.
  Those conditions select a cold download.
- Garbage collection or compaction of persistent storage.
- A user-visible dynamic heap, dynamically sized locals, globals, or results, or ST-visible
  `__NEW`/`__DELETE`. V1 may expose a host-internal allocation/rebinding service to generated
  code; the source-level vendor extension is a v2 item.
- Distributed or network deployment, remote runtime control, and authentication.
- Online debugging, online monitoring, forcing, and tracing beyond what runtime diagnostics
  already require. **Superseded by Q-183/Q-186:** full variable and debug access is now a v1
  scope decision. The project plan, conformance profile, and milestones must be revised.
- **Safety PLC behavior.** The runtime is not a safety PLC and makes no safety-integrity claim.
  Debug sessions may hold outputs indefinitely; no automatic fail-safe output policy is required.
- Running untrusted native modules in a sandbox.
- Preserving raw pointers held outside the runtime's known roots.
- Retaining old modules indefinitely for symbolication by default.

### 4.3 Assumptions

- One host process owns one controller state; there is one cycle thread and one active module.
- A module selects one configuration and one task topology; multi-configuration switching is a
  cold operation.
- Root initializers are constant expressions, as required by D-067.
- Online-compatible modules are produced by the same compiler profile and target as the host.
- Module code is trusted native code supplied by the operator.
- The process image, retained-state file, clock, and task guards are host-owned resources.
- The host has persistent storage for the active GOT manifest/runtime layout record and retained
  state, or an embedding host supplies equivalent storage.
- A GOT manifest may be a build input as well as an output; online-change builds record its
  digest and never consume a manifest silently.
- Migrations that contain persistent references, interface pointers, or raw object pointers are
  restricted; identity-preserving renames are always safe, layout migration is policy-scoped.
- The host is not required to support concurrent external calls during a swap; callers serialize
  through the host API.

## 5. Current architecture and conflicts

### 5.1 What M4 does today

| Location | Current behavior | Consequence for online change |
| --- | --- | --- |
| `crates/stc-codegen/src/unit.rs:128` | Global definitions get direct `@stc_g_*` symbols | Storage belongs to the module and dies with it |
| `crates/stc-codegen/src/unit.rs:135` | Program instances get direct `@stc_pi_*` globals | Instance state is module-owned |
| `crates/stc-codegen/src/unit.rs:273` | Types and globals are emitted into the module | No runtime root registry or catalog |
| `crates/stc-codegen/src/expr.rs:375` | `PlaceHir::Global` reads/writes use the direct symbol | Every global access must become indirect |
| `crates/stc-codegen/src/expr.rs:480` | `place_ptr` returns `Value::symbol` for globals | Address-taking binds code to module storage |
| `crates/stc-codegen/src/expr.rs:521` | `InstanceField` paths start from the instance global | Instance base pointer must come from the GOT |
| `crates/stc-codegen/src/expr.rs:754` | User function calls are direct `call @stc_fn_*` | Call targets are fixed in the module text |
| `crates/stc-codegen/src/expr.rs:847` | Call statements use the same direct path | Same |
| `crates/stc-codegen/src/unit.rs:846` | Program bodies are called directly | Program entry is a raw code pointer |
| `crates/stc-codegen/src/unit.rs:594` | `stc_init` calls the runtime and initializes module-owned globals | Init semantics assume fresh module state |
| `crates/stc-codegen/src/unit.rs:718` | `stc_init` always attempts retained-state load | A swap must not overwrite preserved values with an old file |
| `crates/stc-codegen/src/unit.rs:330` | `stc_io_base` and `stc_io_len` are module globals | Image pointer is not runtime-owned across versions |
| `crates/stc-codegen/src/unit.rs:346` | `stc_retain_buffer` is a module global | Retained-state buffer is module-owned |
| `crates/stc-codegen/src/unit.rs:355` | `SINGLE` task guards are module globals | Task execution state is module-owned |
| `crates/stc-codegen/src/unit.rs:1029` | Executable mode emits `main` calling `stc_rt_execute` | Code lifetime equals process lifetime |
| `crates/stc-runtime/src/lib.rs:100` | `stc_rt_execute` calls `init`, `cycle`, `shutdown` function pointers | No module loader, version registry, or swap |
| `crates/stc-runtime/src/lib.rs:44` | Global runtime state is a single image/clock structure | No per-root allocation registry |
| `crates/stc-driver/src/build.rs:192` | `exe` links the module object plus the runtime staticlib | Cannot replace code while the process runs |
| `crates/stc-codegen/src/abi.rs:4` | Runtime ABI version 1 | No module/host ABI or GOT contract |

### 5.2 The six structural conflicts

1. **Code lifetime equals process lifetime.** User code is linked into the executable or static
   library. There is no loadable unit to replace and no loader boundary.
2. **Data lifetime equals code lifetime.** Globals, program instances, process-image pointers,
   retained buffers, and task guards are module symbols. A new module would get fresh storage.
3. **No stable identity.** HIR uses arena IDs (`DefId`, `FieldId`) and codegen uses mangled
   symbols. Neither is a persistent identity that can map an old root to a new root across
   builds, renames, moves, or changes in source order.
4. **No swap protocol.** The runtime has no catalog validation, compatibility decision, staged
   binding, activation boundary, rollback, or module unload path.
5. **No module ABI.** There is no descriptor format, no attach/prepare/activate interface, no
   host API table, and no rule preventing modules from importing runtime symbols directly.
6. **No build-history artifact.** Nothing records the identity and layout of the running
   version for the next build, for rename tracking, or for another developer or workstation to
   use as a baseline. The only current record is the generated module itself, which is not a
   compiler input and cannot express history, migrations, or downloadable identity.

## 6. Target architecture

### 6.1 Host and module split

```text
+------------------------------- host process --------------------------------+
| stc-runtime host                                                          |
|   cycle loop | latched clock | I/O backends | state file | diagnostics    |
|   root registry and allocator                                             |
|   GOT: stable cells [data | function | descriptor]                        |
|   module registry: active, prepared, retired                              |
+-----------------------------------|---------------------------------------+
                                    | attach(cells), prepare(mode), cycle
                         +----------v-----------+
                         | module (.dll/.so)     |
                         | code                  |
                         | catalog and descriptors|
                         | slot table (cell ptrs)|
                         +-----------------------+
```

- The host owns the cycle loop. At the top of each cycle it checks for a pending module and
  processes load/validate/classify work; bind, prepare, and activation happen under the cycle
  lock with no cycle in flight. `stc_rt_execute` becomes a host-loop convenience for embedding
  hosts; it no longer describes code linked into the process.
- A module is a shared library built by clang from generated IR. It contains code, read-only
  constants, the module catalog, exported entry functions, and module-lifetime binding metadata
  (the slot table and bootstrap pointers). It contains no persistent program state.
- The host loads a module, calls its descriptor entry, validates it and its paired GOT manifest,
  allocates/binds roots, calls `prepare`, and later `activate`. Old modules remain loaded until
  the new version has completed a successful cycle or until rollback is resolved.
- The module's manifest is both the runtime's active identity/layout record and the downloadable
  baseline for the next build. A module without a valid paired manifest is not an online-change
  candidate.

### 6.2 GOT model

The GOT in this plan is a **project-defined runtime table**, not the ELF/PE dynamic-linker GOT.
The dynamic linker still resolves the host executable and the module's own imports; the project
GOT is the stable indirection layer that lets a new module bind to old storage and old callable
slots.

#### 6.2.1 Cells and slot tables

The GOT is a host-owned collection of stable-address cells. A cell holds one pointer:

- a **data cell** holds the address of a runtime-allocated root; or
- a **function cell** holds the address of a callable entry point; or
- a **descriptor cell** holds the address of a runtime-owned type descriptor or vtable.

A module's compiler-assigned slot number is local to that module. The module catalog records the
`GotId` and current key for each slot. At attach, the host builds a `slot table` in module-index
order: an array of pointers to the host cells. The module stores the slot-table pointer in a
single module-local bootstrap global. Generated code loads a cell pointer from the slot table and
then loads the current value from the cell.

Conceptual host layout:

```text
cell[data, GotId=G-001]      -> bytes for Main.Counter (never moves)
cell[fn,   GotId=F-017]      -> active code address for Main.Compute
cell[desc, GotId=T-004]      -> runtime type descriptor for Main.Motor
slot_table[module A][0]      -> &cell[data, G-001]
slot_table[module A][1]      -> &cell[fn, F-017]
slot_table[module B][0]      -> &cell[data, G-001]   (same cell)
slot_table[module B][1]      -> &cell[fn, F-017]     (same cell)
```

A new module can use a different local slot order; the catalog and attach mapping absorb the
difference. Stable keys, not slot numbers, define identity.

#### 6.2.2 Emitted access shapes

Data access (conceptual LLVM IR; exact naming is an implementation decision):

```llvm
; %slot.base is the slot-table pointer stored at module attach
%cell.ptr = getelementptr ptr, ptr %slot.base, i64 <slot-index>
%cell     = load ptr, ptr %cell.ptr
%storage  = load ptr, ptr %cell
%field    = getelementptr %RootType, ptr %storage, i32 0, i32 <field-index>
%value    = load i32, ptr %field
```

Function call:

```llvm
%cell.ptr = getelementptr ptr, ptr %slot.base, i64 <slot-index>
%cell     = load ptr, ptr %cell.ptr
%fn       = load ptr, ptr %cell
%result   = call i32 %fn(i32 %a0, i32 %a1)
```

The builder needs a typed indirect-call operation so the signature is explicit in textual IR.
Opaque pointers are retained; the compatibility check enforces that the key's signature matches
the call-site signature.

A module may cache the slot-table pointer in its entry block or in a module-local bootstrap
global set by attach. It must not cache the value of a data cell across a swap because cells may
change between cycles; caching within one function invocation is safe, since no swap occurs
inside a cycle.

Bootstrap exceptions to "all memory through the GOT":

- the module-local pointer to the host slot table;
- module-owned read-only constants and string literals that never escape into persistent storage;
- stack, local, temporary, parameter, and return storage;
- compiler-generated helper functions and LLVM intrinsics that are version-local.

The compiler must not emit any other writable module-level state. Any value with program
lifetime is a root, a host-owned resource, or a host cell. In particular, compiler helper caches,
task flags, image pointers, and retained buffers must not become hidden module globals.

Everything else is GOT-indirected. The plan recommends making this a structural IR invariant
tested by codegen and fuzzing, not only a convention.

#### 6.2.3 Data-root granularity

Q-042 selected **recursive per-field roots with handle-based objects**; Q-123–Q-126 resolved the
aggregate edges:

- Every persistent scalar or value field of a global, program instance, FB, class, or struct is
  its own root with its own `GotId`, GOT cell, storage, and manifest record. Nested fields
  recursively follow the same rule.
- A persistent object instance is represented by a stable host handle plus a runtime object
  record. `THIS`, `SUPER`, interface values, and aggregate references carry the handle, not a
  contiguous object pointer. Field access resolves the field root directly, not by GEP from the
  object.
- Arrays of scalars or strings are one root with contiguous storage and GEP indexing. Arrays of
  aggregates use per-element handles whose fields are rooted recursively (Q-123).
- A string value is one root with length/data internal to that root; characters are not separate
  roots (Q-124).
- An `OVERLAP` struct is one atomic root with aliasing typed views; its fields are not separate
  roots (Q-125).
- A direct (`AT`) variable is not an independent root; it has a GOT cell pointing into the
  runtime-owned process image (Q-033), with a stable direct-variable identity for remapping.
- Locals, `VAR_TEMP`, parameters, function results, and conformant-array descriptors are
  transient and not roots.
- `REF_TO`/`VAR_IN_OUT` use raw pointers for stable leaf storage and handles/place descriptors
  for aggregates and objects (Q-126).

The consequence is deliberate: adding, removing, or reordering a persistent field is a per-field
preserve/drop operation, not a whole-root layout change. Field type changes and the
array/string/`OVERLAP` exceptions still require an explicit reset, migration, or rejection.
Value-copy semantics are Q-127. See Q-042/Q-046–Q-050/Q-123–Q-127.

#### 6.2.4 Direct variables and the process image

The process image remains runtime-owned. Q-033 selected **one GOT cell per direct variable**,
pointing at its location in the image:

```llvm
%cell  = load ptr, ptr <direct-variable-slot>
%ptr   = load ptr, ptr %cell
%value = load i32, ptr %ptr
```

- Each `AT` variable (or direct field) has a stable direct-variable identity and a cell. The cell
  points into the image; it does not own storage.
- Cells are **per GOT generation** (Q-212). On image reconfiguration, the host rebinds the new
  generation's cells by identity/address and preserves values where possible; old code never sees
  the new image.
- The compiler still emits per-variable metadata (path, area, address, type, offset, hint) so the
  host can reconfigure bindings and remap cells.
- The per-variable lookup cost is accepted for uniform binding/remapping and typed debug access.

#### 6.2.5 Function identity and dispatch

- User functions, FB bodies, methods, interface prototypes, standard FBs, and exported functions
  each get a persistent `GotId`, a current key, and a GOT function cell.
- The current key is derived from the existing mangled name and signature hash, with package and
  namespace qualification. The `GotId` is resolved through the GOT manifest and survives a
  tracked rename or move.
- A function body change keeps the same `GotId`/cell; activation replaces the cell value with the
  new module's address.
- A signature change produces a different current key and normally a new `GotId`. If the old
  function is reachable from an export, a live vtable, or a stored descriptor, the change is
  incompatible unless an explicit mapping and ABI rule permit it.
- Methods are bound through runtime-owned vtables whose entries are cell pointers, not raw code
  pointers. A swap updates function cells; a vtable with an unchanged shape keeps working.
- Direct calls inside a module are allowed only for version-local compiler helpers and
  intrinsics. Calls to user code, methods, standard FBs, and runtime services go through GOT
  cells in the v1 foundation. Q-062 tracks whether an optimized direct-call mode is acceptable
  later.

### 6.3 Runtime memory manager

The host owns a root registry:

```text
RootRecord {
    got_id: GotId         // persistent identity from the manifest
    key: StableKey        // current locator, kept for diagnostics/history
    storage: *mut u8      // never moves while registered
    size: u64
    align: u64
    layout_hash: u64
    type_key: StableKey
    state: Active | Quarantined | Retired
    generation: u64
}
```

Rules:

- The host allocates each new root; generated code never calls `malloc`. A zero-sized
  declaration still receives a one-byte allocation so every cell has a unique non-null address.
- Under Q-042, each persistent field is allocated as its own root. A persistent object also has a
  stable host handle and object record; the record identifies the instance and its field roots,
  but does not make the object contiguous. Arrays/strings/`OVERLAP` follow Q-123–Q-125.
- Matching roots are matched by field-level `GotId`; a new object instance creates a new set of
  field roots and a new handle. Quarantined fields keep their storage; handles to removed
  instances remain opaque to new code.
- Storage is zeroed on allocation. New-root initializers run during prepare.
- Storage never moves. This keeps data cells, references, and object pointers valid.
- The host uses a conservative alignment (16 bytes on the supported targets) and a size supplied
  by the module descriptor. The module computes sizes with LLVM constant expressions, so the
  compiler does not need to hard-code target layout; if a constant-expression size proves
  unportable across the accepted LLVM versions, the descriptor provides a size function that the
  host calls once at attach. Q-041 tracks this.
- Matching roots reuse the existing storage pointer and are not reinitialized.
- Removed roots are quarantined in v1: the record and storage stay allocated, the host warns,
  and no new module can name them. The storage is reclaimed only on host restart or an explicit
  reset. This prevents dangling `REF_TO`, interface data pointers, and `VAR_IN_OUT` storage
  addresses that were retained across cycles.
- Reset roots allocate a fresh record and cell value; the old storage is quarantined because
  other preserved data may still point into it.
- There is no compaction or moving collector in v1.
- The host may expose a narrow internal allocation/rebinding service to generated code
  (`alloc(size, align)`, `free`). These allocations are host-owned, stable, zeroed, and aligned,
  but they are **not** manifest roots, do not get `GotId`s, and are not preserved across module
  versions. Q-121 is resolved: they are transient module-version resources, freed on failed
  prepare, retirement, or unload; they must never be stored in persistent roots or carried
  across a swap.
- The process image, retained payload buffer, task guards, clock state, runtime counters, and
  module registry are host-owned. Generated code reaches them through cells or the host API.

### 6.4 Module catalog and descriptors

A module exports a descriptor containing:

1. **Header**: magic, module ABI version, host ABI version, profile ID, compiler version,
   target triple, endianness, pointer width, build ID, generator flags.
2. **Data roots**: `GotId`, current key and previous keys, root kind (global/program instance),
   type key, layout hash, LLVM-computed size, alignment, retention policy, initializer
   reference, and source location.
3. **Functions**: `GotId`, current key and previous keys, signature key, pointer, flags
   (user/method/FB body/export/runtime shim).
4. **Types**: `GotId`/type identity, current key and previous keys, structural layout hash, size,
   base type, interfaces, vtable shape, method keys in slot order.
5. **Instances and tasks**: configuration name, instance path, program type key, resource, task,
   bindings, task guard identity, and persistent `GotId`.
6. **Exports**: C name, signature key, function cell key/`GotId`, `no_mangle` flag.
7. **Initialization metadata**: which roots need declarations/defaults, and the entry points for
   new-root initialization, cycle, shutdown, retain save/load, and metadata.
8. **Source and span tables**: per-module, retained by the host for diagnostics after unload.

The binary descriptor is the loading contract for code addresses and sizes. The GOT manifest is
the identity/history contract described in §6.4.1. They are two deterministic projections of one
catalog model and are paired by digests; neither is maintained as an independent schema. Phase 1
finalizes the pairing as a descriptor digest (computed with the descriptor's manifest-digest field
zeroed) plus the manifest digest, with descriptor/manifest slot and function sets required to be
equal; the v1 pairing does not hash the complete platform image
([GOT Artifacts and Schemas](Specification/Architecture/GOT-Artifacts.md) §11). The
existing `<name>.stc-meta.json` remains the human/tooling sidecar for module metadata, spans,
I/O, and retained-state details; the GOT manifest is a separate canonical artifact with its own
format version.

The host copies every descriptor byte, key string, and table entry it needs during validation.
It must not retain pointers into module memory after the module is unloaded; the catalog,
compatibility record, source map, and build ID become host-owned data at load time.

#### 6.4.1 Persistent GOT manifest

The compiler emits a persistent **GOT manifest** with every module. It is the build-history
artifact for all GOT-tracked instances, the input to the next build, the runtime's record of the
active layout, and the file a different developer can download from the PLC as a baseline. The
working file names are `<name>.stc-got.json` for the canonical tooling/download artifact and
`<name>.stc-got.bin` for the compact runtime projection; final naming and encoding are Q-097.

The manifest is versioned, deterministic, and self-describing:

| Section | Contents |
| --- | --- |
| Header | Manifest format version, profile ID, target triple, endianness, pointer width, compiler version, module/host ABI version, build ID, descriptor digest, parent manifest build ID, active configuration, source fingerprint |
| Identity table | One record per GOT-tracked instance: `GotId`, kind (data root/function/type/descriptor/instance/task/export), current key, previous keys, owner/root path, source location (relative path and offsets), current module slot(s), runtime cell kind, status |
| Layout table | For each data root and type: canonical type key, structural layout hash, target size/alignment, field paths/types/offsets, base/interface links, vtable method keys and slots, pointer-containing flags, retention policy, initializer fingerprint |
| Function table | Function/method/FB-body `GotId`, signature key, current key and previous keys, owner type, vtable slot, export C name, flags |
| Instance/task table | Configuration, resource, task, program-instance paths, task attributes, bindings, task-guard identity |
| Online-change section | Parent build ID, per-`GotId` action (preserve/allocate/migrate/reset/drop), migration operation list or migration-function `GotId`s, compatibility verdicts, warnings, and the rationale for every rejection |
| Integrity | Content digest of the manifest itself, digest of the paired descriptor (manifest-digest field zeroed), optional build signature/hash pin |

Field offsets, sizes, and vtable layouts in the manifest are target-specific. A manifest is a
valid online-change baseline only for the same target triple, endianness, pointer width, profile,
and ABI. A cold build on a different target may reuse identity keys for matching, but it must
recompute all layout and migration data; it must not treat the old target's offsets as valid.

The manifest tracks **GOT-tracked instances**, not every leaf. Roots, functions, types,
descriptors, configured instances, tasks, and exports have `GotId`s. Nested struct/array/string
fields and nested FB/class state are described by the parent root's layout table and by field
paths; they do not get separate cells unless they are themselves independently addressable
persistent roots. This keeps the manifest proportional to the program surface rather than to the
number of scalar leaves. Slot numbers in a manifest describe the build that produced it; they are
per-build binding details, not identities, and a consumer re-derives its own slot table from the
`GotId` records.

The manifest is not a value store. It contains identities, layout, history, and migration
instructions. Runtime values remain in host memory and the retained-state file.

#### 6.4.2 Identity assignment and build workflow

- A `GotId` is a persistent identifier recorded by the first manifest that introduces the
  instance. It is not a `DefId`, `FieldId`, source path, or mangled symbol.
- New identities are assigned deterministically. The recommended scheme is a content-derived
  128-bit identity from the declaration's first-seen scope/name/kind, with a checked collision
  policy. For data roots and types, the identity excludes the current type/layout, so a layout
  change can be tracked and migrated; the type/layout hash is compared separately. For
  functions, the signature is part of the identity; a signature change is a new callable unless
  an explicit mapping says otherwise. A sequential allocator recorded in the manifest is an
  acceptable alternative if concurrent development is not supported.
- An unchanged declaration keeps its `GotId` because its current key still matches the manifest.
- A renamed or moved declaration keeps its `GotId` only when the build expresses the rename or
  move explicitly. The manifest records the old key in the instance's history.
- A new declaration gets a new `GotId`. A removed declaration's entry becomes retired and its
  storage is quarantined by the host. If a later build reintroduces a retired key, it gets a new
  `GotId` unless an explicit reuse mapping selects the old one; this prevents accidental
  resurrection of quarantined values.
- Matching is case-insensitive according to the language rules, but keys are stored in their
  normalized uppercase form plus the original spelling for diagnostics. Keys include the package
  identity from the manifest, not a local dependency alias or absolute path. A different type
  under the same data-root name is the same logical identity with a layout change, not a new
  variable; to create a fresh identity with an old name, use an explicit retire/rename policy.
- A build without a valid baseline manifest is a **cold layout build**: it assigns identities
  from scratch and still emits a complete manifest. It is marked as having no online-change
  parent, and the runtime will require a cold download if an operator attempts to deploy it over
  a running program. Coincidentally matching `GotId`s do not make it an online change.
- An online-change build must supply or auto-discover a structurally valid baseline manifest,
  record its digest in the output, and emit the new manifest plus an online-change plan. A
  missing, unreadable, invalid, or target/profile-mismatched baseline selects the cold-layout
  path rather than an online attempt.
- Up-to-date-ness is a runtime decision: the manifest records its parent build ID, and the
  runtime accepts the plan only when that parent equals the active build ID and the module pairs
  with the manifest. Otherwise the only permitted action is a cold download. The compiler never
  claims that an online change will be accepted based on a local baseline.
- The build does not need a separate online-change mode: a valid baseline produces an
  online-candidate plan, no baseline produces a cold layout, and `--cold` forces cold. The
  runtime's currentness check is the final authority.

Build flow:

```text
cold build:     source --------------------------> module V1 + manifest M1 (no parent)
online build:   source + baseline manifest M1 ---> module V2 + manifest M2 + plan M1->M2
runtime:        module V2 + manifest M2 ---------> if parent == active: apply plan, activate
                                                   else: cold download only
download:       PLC active manifest M2 ----------> next developer's baseline
```

The manifest is an explicit build input. It is not hidden in a compiler cache. The compiler
records its digest in the new manifest and in module metadata, so a build can be reproduced by
supplying the same source and baseline. A missing, stale, target-mismatched, or schema-invalid
baseline is a diagnostic, never a silent reset.

#### 6.4.3 Tracked rename and move

Renames and moves are the first complex online change enabled by the manifest:

- A build may accept an explicit rename/move map, for example `old.path -> new.path`, or a
  compiler command that performs a source-aware rename and records the mapping.
- The compiler resolves the new declaration, verifies its kind/type/signature compatibility,
  carries the old `GotId` forward, and records the previous key in the manifest history.
- The runtime sees the same `GotId` and reuses the storage. No data is copied.
- Tracked renames cover variables, globals, program-instance declarations, struct/FB/class types
  and their fields, methods, functions, tasks, and exports. A field rename is recorded in the
  parent type's field-path history while the type keeps its `GotId`. Export renames additionally
  require the C ABI name/signature rules in §6.6.
- Changes of namespace or package are moves and use the same mechanism. A namespace/package
  rename can be expressed as a bulk move mapping, so every contained identity keeps its `GotId`.
  A path change is not silently treated as a rename.
- Without an explicit mapping, a rename is an untracked remove plus add. The compiler emits a
  warning or error that names the removed `GotId` and the new declaration, and requires
  `--allow-untracked-removal` or a cold build before it will discard the old identity.
- Heuristic rename detection by name similarity, type equality, or source diff is deliberately
  not part of v1. It is too easy to bind the wrong storage and silently corrupt state.

A source-aware rename command can be added after the identity plumbing works. The minimum v1
mechanism is an explicit rename map or `--rename old=new` build option; the build option assumes
the source already uses the new name, while `stc rename` performs the source rewrite and records
the mapping in one operation.

#### 6.4.4 Online-change plan and migration scope

The compiler compares the new source against the baseline manifest and emits an explicit plan:

| Plan action | Meaning |
| --- | --- |
| `Preserve` | The `GotId` exists and its layout hash/size matches. The host reuses storage. |
| `Allocate` | The `GotId` is new. The host allocates and the module initializes it. |
| `Migrate` | The `GotId` exists with a compatible identity but a different layout/type. The plan describes how to carry values forward. |
| `Reset` | The `GotId` exists but the change is only acceptable by reinitializing it under the selected policy. Old storage is quarantined. |
| `Drop` | The `GotId` is gone. The host quarantines storage and warns if references may remain. |
| `Reject` | The change is unsafe or outside policy. The module is rejected atomically. |

Migration operations are compiler-generated because the compiler knows both layouts. A plan may
contain field copies, widening/lossless conversions, initialization of new fields, dropping of
removed fields, and runtime-pointer repair instructions. The host validates indices, offsets,
sizes, and digests and executes the plan; it does not infer ST types on its own.

Recommended staging:

- **M4.5:** manifest emission/consumption, `GotId` identity, preserve/allocate/drop, tracked
  rename, and exact-layout compatibility. No field-level layout migration.
- **M5:** aggregate/string/FB roots participate in the same identity and layout records; exact
  layout preservation and reset/reject rules apply. Pointer-containing roots remain strict.
- **M6 and later (not v1):** field-level migration is not in v1. Per-field roots already make
  field add/remove/reorder preserve/drop operations; field type changes, arrays/strings/OVERLAP
  changes, and resizing reset or require a cold download. A future migration design for
  pointer-free cases would need a new decision; pointer translation remains a later design.

Migration functions live in the new module, are version-local, and run during prepare. They must
not call through shared function cells; they may use host services and direct helpers. The host
calls them with old and new root pointers, or with a staged old-value snapshot, depending on theresizing policy. In-place migration must be preceded by an old-value snapshot or performed on
staged storage so a failed prepare can roll back without corrupting the preserved root. The
manifest identifies the function and the `GotId`s it touches.

#### 6.4.5 Runtime layout record and download

The host persists two related records:

1. **Active manifest**: the compiler manifest of the active module, stored as the authoritative
   identity/layout baseline and paired with the active build ID/module digest.
2. **Runtime layout record**: the host's map from `GotId` to cell, storage pointer, size, state
   (active/quarantined/retired), allocation generation, and the active build lineage.

The downloadable artifact combines the active manifest, the runtime layout summary (no values),
and the module metadata needed to identify the deployment. It lets another developer or
workstation build the next version against the running PLC layout. It does not transfer runtime
values; state stays on the PLC. The active module binary may optionally be offered for download
under the same access control, for diagnostics or source-independent reuse, but it is not
required for the manifest workflow.

The host exports the artifact through the host API and a local CLI/file operation in v1. The
runtime never silently creates a new identity baseline when a stored manifest is missing or
corrupt; it reports the condition and requires an explicit cold start. Active-manifest
replacement is atomic and paired with the commit; a failed write leaves the previous manifest
active. If no valid active manifest remains, the next deployment is a cold download.

A later optional command, `stc online scaffold`, may emit ST declaration stubs from a
downloaded manifest when the original source is unavailable. It cannot recreate statement
bodies or comments, and it is not a substitute for source control; its only purpose is to let a
new project reuse the running identities and layout deliberately.

If the runtime must operate a legacy module that predates the manifest format, that deployment is
a **cold installation**. The host may synthesize a manifest from the module descriptor during
that cold install and store it as the active baseline; the legacy module cannot participate in an
online change before that cold installation.

The fail-safe rule is simple: if the active manifest is missing, invalid, unpaired with the
module, or its parent build ID is not the active build ID, the runtime does not attempt an online
change. It reports the reason and offers the cold-download path.

### 6.5 Online-change protocol

```text
IDLE
  | loader thread: load(module, manifest)
  v
LOADED ----------- manifest/module invalid or unpaired --> COLD REQUIRED (old version stays active)
  | validate header, digests, pairing, catalog, keys
  v
VALIDATED -------- parent build != active build ---------> COLD REQUIRED (old version stays active)
  | classify the plan against the active layout
  v
CLASSIFIED ------- incompatibility / rejected plan ------> REJECTED (old version stays active)
  | build temp GOT generation, allocate/bind new roots
  v
BOUND ------------ allocation or plan failure ----------> FAILED (drop temp; old stays active)
  | loader thread prepare with temp GOT context
  v
PREPARED --------- prepare/migration failure ------------> FAILED (drop temp; old stays active)
  | atomic temp GOT publish at a cycle boundary (O(1))
  v
ACTIVATED -------- first cycle traps --------------------> TRAP / rollback to normal
  | one successful cycle
  v
COMMITTED -------- copy temp->normal off-thread; switch normal at later boundary; unload old
```

Detailed steps:

1. **Build (offline).** The compiler emits IR, a module, a binary descriptor, JSON metadata, a
   content-derived build ID, and the GOT manifest. When a baseline manifest was supplied, the
   output also contains an online-change plan and compatibility report.
2. **Load.** The loader thread loads the shared library and its manifest while the cycle
   continues. The cycle thread is not blocked by module loading.
3. **Validate.** Check magic, ABI versions, profile, target, endianness, pointer width, build ID
   uniqueness, manifest/module digest pairing, duplicate keys/`GotId`s, catalog bounds, and
   descriptor consistency. A missing, unreadable, invalid, or unpaired manifest selects the
   **cold-download** path; it is not an online-change error to recover from.
4. **Check lineage.** The manifest's parent build ID must equal the active build ID. If it does
   not, the baseline is stale or belongs to another deployment: select the **cold-download**
   path. No best-effort rebase, merge, or partial online attempt is performed.
5. **Classify.** Compare the new manifest against the active runtime layout record. If the
   compiler supplied an online-change plan, validate every action against the actual active
   `GotId`s, layouts, and build lineage; do not trust the plan blindly. Without a plan, apply
   the strict key/layout matching rules.
6. **Bind into a temporary generation.** Copy the active GOT/slot table into a temp generation.
   Add new cells; point preserved data cells at the same stable storage; allocate new roots and
   quarantine removed roots. Every function slot in the temp generation binds to the new module
   (Q-176). Build/validate host-side vtables/descriptors for new types. For `Migrate` actions,
   stage old values/pointers as required; in-place migration snapshots the affected roots.
7. **Prepare on the loader thread.** Call `stc_module_prepare` with the temp GOT context
   (Q-178). The module initializes only fresh roots and must not write preserved roots or shared
   host state. I/O/task staging is host-owned. Root initializers are constant expressions
   (D-067), so this restriction is satisfiable in v1.
8. **Activate.** At the cycle boundary, publish the temp GOT generation and active-module pointer
   with an atomic release/acquire switch; the next cycle runs the new module. This is O(1); no
   allocation, loader call, or O(N) copy occurs in the cycle thread. The old module and normal
   GOT remain resident but inactive.
9. **Observe.** Run at least one full cycle. On recoverable failure, switch back to the normal
   GOT and old module, restore descriptors/process-image configuration/active manifest, and
   quarantine or free new roots. A trap follows the existing fatal-trap policy unless a host
   rollback policy is selected.
10. **Commit.** After one successful cycle, copy the temp GOT to the normal GOT on the loader
    thread while the cycle runs on temp (Q-179). Switch the active pointer back to normal at a
    later boundary. Store the new manifest as the active baseline, update the runtime layout
    record, and unload the old module off-thread after a grace period when no active GOT entry,
    reference, or descriptor points into it (Q-181). Rollback after commit requires the previous
    module/manifest pair; otherwise it is a cold download.
11. **Cold download.** A stop-and-reload path initializes all roots and may load retained state
    from the state file under the existing layout/profile compatibility policy. It uses the same
    module ABI but a different init mode. A cold build still emits and installs a manifest, so
    the next online change has a baseline. Cold download is the required outcome whenever the
    online preconditions above fail.

### 6.6 Compatibility classes

| Change | v1 classification | Default action |
| --- | --- | --- |
| Function/method body change, same signature | Seamless | Preserve all roots; update function cells |
| New function/method/type not reachable from live exports, vtables, or roots | Seamless/additive | Add cells/descriptors |
| New global variable or persistent field | Additive | Allocate the new field root(s); initialize defaults |
| New program/FB/class instance | Additive | Allocate the instance handle and its field roots; initialize defaults |
| Constant initializer change for a new field root | Additive | Apply at allocation |
| Constant initializer change for a preserved field root | Ignored | Preserve value; warn |
| Removed global/field/instance not referenced by preserved state | Removed | Quarantine the field storage; warn |
| Removed global/field/instance reachable from preserved state | Removed | Quarantine; v1 keeps references memory-safe but stale |
| Tracked rename or move, same type/layout | Seamless | Preserve the field/instance `GotId` and storage; update keys/history |
| Untracked rename (old key removed, new key added) | Removed + additive | Quarantine old storage; warn/error and require explicit untracked-removal or cold policy |
| Added/removed/reordered field under per-field roots | Additive/removed | Add or drop the affected field roots; reordering is identity-neutral |
| Type/layout change of a preserved field | Migration-required or incompatible | Reset/migrate/reject; arrays/strings/OVERLAP follow Q-123–Q-125 |
| Object handle for a preserved instance | Seamless if the instance and descriptor are preserved | Reuse the handle; new instances get new handles |
| Manifest missing, stale, unpaired, or digest-mismatched | Cold required | Cold download; no online attempt and no recovery/merge |
| Plan parent build ID differs from the active build ID | Cold required | Cold download; the baseline is stale or belongs to another deployment |
| Compiler plan conflicts with the active runtime `GotId`/layout | Incompatible | Reject atomically |
| Migration plan not permitted by policy or touches unsafe pointer-bearing state | Incompatible or reset | Reject by default; reset only with explicit policy |
| Constant global value change, inlined by the compiler | Code-only | New code uses the new value; no persistent root is involved |
| Address-taken constant value change | Incompatible | Reject by default; the old storage is quarantined if an explicit reset is chosen |
| Enum/subrange value or bound change | Code-only layout | Allow; document that stored values are not revalidated |
| Export signature change or removed export | Incompatible | Reject |
| Vtable/interface shape change for a type with live instances or interface values | Incompatible | Reject or require a cold download |
| New method on a type with no live instances/interface values | Additive | Allow |
| Configuration, resource, task, or task-priority topology change | Incompatible | Cold download |
| `AT` address/layout change | Additive with remap | Reconfigure image; preserve by identity where possible; warn |
| Retained-state layout change | Incompatible for cold start | Online swap keeps memory; next state save uses the new layout hash |
| Target triple, profile, endianness, pointer width, or host ABI change | Incompatible | Reject |

The default `on-incompatible` policy is `reject`. A per-root or manifest-level `reset` policy may
be added for development convenience; it allocates fresh storage for the affected roots and warns
about stale references. A `stop`/`cold` policy performs a cold download. Missing, invalid,
unpaired, or non-current manifests always select cold download rather than a compatibility
decision. Migration is never implied by a type change alone: the compiler must emit an explicit
plan, the manifest must declare the operation, and the host must validate it against the active
runtime layout. If any part of the plan is unsafe or unsupported, the whole module is rejected
unless `reset` is explicitly selected for the affected roots.

### 6.7 OOP, interfaces, and `?=`

- Each class/FB/interface gets a runtime-owned type descriptor with a stable manifest `GotId`
  (or type identity derived from it), not merely a source name.
- Descriptors contain base/interface links and a vtable of **cell pointers**, not raw code
  addresses. Dotted method dispatch loads the cell pointer and then the function pointer.
- Under Q-042, a persistent object is identified by a stable host handle and object record, not a
  contiguous data pointer. `THIS` and `SUPER` carry the handle; field access resolves per-field
  roots. Interface values are `{ handle, descriptor pointer }`.
- `SUPER.method()` dispatches through the base descriptor/function cell; `THIS.field` and
  `SUPER.field` resolve field roots directly.
- `?=` consults runtime descriptors. A type whose manifest identity and layout hash match is
  considered the same type across versions. A changed descriptor shape requires a migration plan
  or a reset/reject decision.
- The module must never write a raw code pointer or a persistent object pointer into module-owned
  storage. Compiler-generated tables that contain function addresses are read-only module data
  used only during attach; the host copies the addresses into cells and descriptors.
- Vtable and interface-shape changes are compared against the manifest's recorded shape. Adding
  or reordering slots is allowed only when no live instance or interface value depends on the old
  shape, or when an explicit descriptor migration is planned.

### 6.8 Process image, clock, tasks, and retained state

- The host owns the process image and reconfigures it during prepare when metadata changes.
  Inputs and outputs are latched/published only around complete cycles.
- The clock is host-owned and continuous across swaps. The latched cycle time is refreshed by
  the host at the start of every cycle.
- Retained-state save runs at host shutdown or an explicit stop, not at a module swap. The old
  module is retired without saving; the new module's retain functions save the in-memory values
  when the host eventually stops.
- Task topology is immutable in online mode in v1. `SINGLE` guards, last-execution timestamps,
  and watchdog counters are host-owned and keyed by resource/task identity.
- Retained-state load is skipped in online-attach mode. The active module's retain functions
  save the current in-memory values at shutdown. A layout change makes the next cold start see a
  layout-hash mismatch and follow the existing `on-mismatch` policy. The state-file header's
  format and runtime/ABI version fields are updated with the host ABI; files from the old model
  follow the same explicit mismatch policy rather than being reinterpreted.
- The active manifest is the identity baseline for the state file. A later state-format revision
  can key retained leaves by `GotId` plus path, which lets a retained value survive a tracked
  rename or a permitted layout migration. V1 online change preserves values in memory; the state
  file continues to use the existing layout-hash policy until that revision is designed.
- `AT` values are process-image values, not file-retained values; `%M` markers persist across
  swaps when their area/address is unchanged.

### 6.9 Module ABI sketch

The final ABI is Q-064. The plan proposes:

```c
/* Stable host services, versioned. Additive fields only within a major version. */
typedef struct stc_host_api {
    uint32_t abi_version;
    uint32_t struct_size;
    void   (*trap)(int32_t code, uint32_t span_id);
    int64_t (*cycle_now_ns)(void);
    void*  (*alloc)(uint64_t size, uint64_t align);   /* internal; lifetime Q-121 */
    void   (*free)(void* pointer);                    /* internal; lifetime Q-121 */
    /* clock, I/O image, state file, string/math helpers, diagnostics */
} stc_host_api;

/* Module-exported entry points. */
const stc_module_desc* stc_module_descriptor(void);
int32_t stc_module_attach(const stc_host_api* host, const void* const* slots);
int32_t stc_module_prepare(uint32_t mode, const uint8_t* fresh_root_mask);
void    stc_cycle(void);
int32_t stc_module_shutdown(uint32_t mode);
void    stc_module_detach(void);
```

- The host API table is a host-owned function-pointer table; it is the runtime-service portion
  of the GOT. Generated code stores the table pointer once at attach and calls through indexed
  slots.
- Modules call host services through the host API table; they do not import `stc_rt_*` symbols.
  This avoids Windows import-library and symbol-export problems and makes the runtime ABI a
  single versioned struct.
- The host API table includes allocator/registry services only where generated code needs them;
  persistent allocation itself is host-driven from the catalog.
- `mode` distinguishes cold start, online attach, and explicit reset.
- `fresh_root_mask` lets generated initialization skip preserved roots without the host
  snapshotting and restoring memory.
- The descriptor records the paired manifest digest and size; the module may also export a
  read-only pointer to the canonical manifest bytes. Migration functions listed by the manifest
  are exposed through the descriptor and called by the host with old/new root pointers or staged
  old values.
- The descriptor is versioned independently of the host API so tooling can read future modules
  without loading them.

### 6.10 Diagnostics

Proposed new code families (final allocation is Q-080):

| Code | Meaning |
| --- | --- |
| `STC-E-MODULE-001` | malformed module descriptor, catalog, or entry point |
| `STC-E-MODULE-002` | module target/profile/endianness/pointer-width mismatch |
| `STC-E-MODULE-003` | duplicate or malformed stable key |
| `STC-E-ABI-001` | host ABI version mismatch |
| `STC-E-MANIFEST-001` | GOT manifest missing, stale, or not paired with the module |
| `STC-E-MANIFEST-002` | invalid manifest schema, digest, `GotId`, or duplicate key |
| `STC-E-MANIFEST-003` | untracked removal/rename requires an explicit policy |
| `STC-E-MANIFEST-004` | online-change plan invalid, unsafe, or inconsistent with the active layout |
| `STC-E-ONLINE-001` | incompatible root type/layout |
| `STC-E-ONLINE-002` | incompatible export signature or removed export |
| `STC-E-ONLINE-003` | incompatible vtable/interface shape |
| `STC-E-ONLINE-004` | incompatible configuration/task topology |
| `STC-E-ONLINE-005` | module prepare or activation failure |
| `STC-E-ONLINE-006` | slot table or cell binding failure |
| `STC-W-ONLINE-001` | removed root quarantined |
| `STC-W-ONLINE-002` | initializer ignored on a preserved root |
| `STC-W-ONLINE-003` | direct-variable address remapped during swap |
| `STC-W-ONLINE-004` | stale reference may point at quarantined storage |
| `STC-W-ONLINE-005` | baseline manifest is not current; cold download required |
| `STC-W-ONLINE-006` | manifest missing or invalid; cold download required |
| `STC-W-MANIFEST-001` | no baseline manifest; build uses a cold layout |
| `STC-W-MANIFEST-002` | untracked rename/removal will discard state |
| `STC-W-MANIFEST-003` | baseline manifest source fingerprint differs from the current source tree |

Human diagnostics name the root path, old and new type/layout hashes, and the active build ID.
JSON diagnostics add structured fields so deployment tooling does not parse prose.

### 6.11 Determinism

- GOT slot assignment sorts roots and functions by stable key, not arena order. `GotId`s are
  persistent and are not reassigned by sorting a later build.
- Catalog and manifest arrays are sorted by key/`GotId`; size/layout hashes are content-derived.
- Build IDs derive from profile, target, module ABI, catalog, manifest content, and IR content,
  with no timestamps or absolute paths.
- Two builds of the same source and the same baseline manifest must produce byte-identical IR,
  descriptor bytes, manifest bytes, JSON metadata, online-change plan, and build ID.
- A build without a baseline is reproducible from source alone; a build with a baseline is
  reproducible only when the baseline digest is supplied and recorded.
- The existing M4 determinism test extends to descriptor, manifest, slot-table, and plan
  emission.

### 6.12 Security and robustness

- Native modules execute with full process privileges. V1 trusts modules produced by the same
  compiler and supplied by the operator; it does not sandbox them.
- The loader validates all offsets, counts, string lengths, and keys before use. Malformed
  catalogs must produce a diagnostic, not a panic or out-of-bounds access.
- An optional build-ID/hash pin prevents accidental deployment of a stale or wrong module.
- A downloaded GOT manifest is untrusted input to the compiler and runtime. It is parsed and
  validated structurally, never executed. Its layout data can reveal program structure, so
  export/download is an access-controlled operation in a deployed environment.
- The manifest and module are paired by digest; a manifest that does not match the module or the
  active build lineage is rejected.
- No network deployment or remote control is in v1.
- Module paths are explicit; the host does not scan arbitrary directories.
- Unload happens only when no code can be executing and no raw code pointer remains outside the
  host's function cells and module registry.

### 6.13 Per-field root model (Q-042)

Q-042 resolved that, for persistent aggregates, **every field is a separate root recursively**,
with handle-based objects. This supersedes the earlier "aggregate fields live inside one parent
root" assumption wherever it appears in this document. Q-123–Q-126 resolved the remaining
array/string/`OVERLAP`/reference details.

Working model:

- A persistent object (program/FB/class instance) is represented by a stable host **handle**
  plus a runtime object record. `THIS`, `SUPER`, interface values, and aggregate references carry
  that handle, not a contiguous struct pointer.
- Each persistent field has its own `GotId`, GOT cell, storage allocation, type identity, and
  manifest record. The field key is the instance path plus the field path, including base-type
  and nested-aggregate segments.
- Field access lowers to a field cell lookup, not `getelementptr` from a parent object. GEP is
  still used inside an individual field root, for example an array's elements if an array is a
  single root.
- Object initialization, copies, `VAR_IN_OUT`, `REF_TO`, interface upcasts/downcasts, `?=`, and
  `SUPER` all need handle/place rules. Raw pointers remain valid only for stable leaf storage
  that is never moved.
- Adding/removing a field becomes a per-field preserve/drop operation rather than a whole-root
  layout change. This reduces the need for field-layout migration, but type changes, arrays,
  strings, `OVERLAP`, and value-copy semantics still need explicit rules.
- The manifest grows to one record per persistent field. GOT slot count and field-access
  indirection grow accordingly. Q-123–Q-126 and Q-130 (performance) bound this.

Resolved edges:

- **Arrays (Q-123):** arrays of scalars/strings are one root with GEP indexing; arrays of
  aggregates use per-element handles with recursively rooted fields.
- **Strings (Q-124):** one root per string value with internal length/data; no per-character
  roots.
- **`OVERLAP` (Q-125):** one atomic overlap root with aliasing typed views; internals require
  reset/cold for online change.
- **Handles and references (Q-126/Q-162–Q-165):** stable opaque handle plus runtime object
  record for objects; validated `REF_TO` values use place descriptors (no persisted raw
  pointers); object references use `{handle, descriptor}`. `SUPER`, interfaces, `REF_TO`, and
  `VAR_IN_OUT` follow the same rule.
- **Value semantics (Q-127):** aggregate assignment/copy is a per-field recursive copy into the
  destination roots, preserving destination object identity; handles are not copied as storage
  pointers.
- **Per-field keys/paths (Q-128):** configuration instance + declaring type/base chain + field
  names, normalized; `GotId`s survive tracked renames/moves.
- **Uniform scope (Q-129):** uniform for persistent program/FB/class/struct fields, with the
  Q-123–Q-125 array/string/`OVERLAP` exceptions.
- **Performance/size (Q-130):** measure field-access indirection, cell count, manifest size, and
  load time; cache slot/cell pointers; no hard cap yet.

Remaining consequences:

- **Retained state and reporting:** the v1 state file keeps its layout-hash policy and does not
  migrate per-field leaves; a later state format may key leaves by `GotId` plus path (Q-113).
- **M4.5 scope:** the M4 scalar surface has only scalar program/global fields, so it can
  implement per-field roots without nested aggregates. M5 must implement the handle-based
  object/aggregate model; this is a major M5 design change, not an incremental tweak.
- **Status:** all questions are resolved. Field migration (Q-049/Q-110) is not in v1; internal
  allocation lifetime (Q-121) is transient/free-on-retire.

The rest of this document uses "root" in the per-field sense where Q-042 applies. Statements
about aggregate fields living inside a parent root are obsolete for persistent aggregates and
must be read with this section.

### 6.14 Persistent reference policy — what "no persistent references" means

The plan uses "no persistent references" as shorthand for a lifetime rule, not as a ban on
`REF_TO`. It has three distinct scopes:

| Scope | Allowed | Not allowed |
| --- | --- | --- |
| Across scan cycles (in-memory) | `REF_TO` fields pointing at host-owned stable leaf roots; handles/place descriptors for aggregate/object targets; interfaces with stable descriptors; `VAR_IN_OUT` during a call | References into module-owned code/read-only data, transient host allocations, stack/locals, or unregistered memory |
| Across warm restart (state file) | Nothing reference-shaped is currently persisted. References/interfaces/vtable/descriptor pointers are excluded from the RETAIN payload (A-012/D-032) and are reset or reconstructed | Raw pointers, handles, or vtable/descriptor pointers in the state file |
| Across online change / module unload | References whose target `GotId`, type, layout, and storage identity are preserved; interface/object handles whose descriptor is preserved | References to removed roots (quarantined, memory-safe but stale), migrated/moved targets without translation, and anything pointing into the retiring module |

Consequences:

- A persistent `REF_TO` field is in scope. It is stored in its field root and may point to a
  stable host root. It must never point to a module-owned object or to a transient allocation
  that is freed on retire/unload.
- Removed roots are quarantined, so a stored pointer remains memory-safe. It is still "stale" in
  the sense that new code cannot name the removed root and the value may no longer be meaningful.
- A field whose type/layout is migrated or reset may invalidate references into it. Pointer-bearing
  roots are therefore excluded from migration unless a pointer-translation design is approved.
- Interface values are `{ handle, descriptor }`; the descriptor must be a stable runtime object,
  not a module-owned table.
- `VAR_IN_OUT` is a call-duration parameter, not a persistent reference. Passing an aggregate
  carries a handle; passing a scalar/string/array leaf may carry a raw pointer to stable storage.
- The state file continues to exclude reference/interface/pointer-shaped leaves. A later format
  may persist stable identities and reconstruct them, but that is not v1.

Spelling scope:

- IEC 61131-3:2013 uses **`REF_TO`**; the current profile supports `REF_TO`, `REF()`, `^`,
  `NULL`, `?=`, and `VAR_IN_OUT`.
- **`REFERENCE TO` is not the 2013 spelling** and is not in the current grammar/profile.
- **`POINTER TO` is a vendor extension** (TwinCAT/CODESYS-style) and is not in the current
  grammar/profile. Adding it would be a separate profile/vendor-extension decision, likely v2,
  with stricter rules than `REF_TO`.
- Q-131–Q-134 track these choices and the migration/state rules.

### 6.15 `POINTER TO` extension analysis (Q-131)

Q-131 confirmed `REF_TO` as the expected spelling and asked whether CODESYS-style `POINTER TO`
should also be in v1. It is **under discussion**, not approved. This section records the
implications and the decision questions.

What `POINTER TO` would add:

- A distinct raw-address type: `p : POINTER TO T`, `ADR(x)`, `p^`, `NULL`; CODESYS also has
  arithmetic/indexing, casts, and `SIZEOF`.
- Unlike `REF_TO`, it carries no target identity, type-shape, or lifetime guarantee. It may point
  into the middle of a root, an array element, a field, or storage later removed.
- CODESYS/TwinCAT commonly pair it with `__NEW`/`__DELETE`; those are already deferred to v2.

Implications if added to v1:

| Area | Impact |
| --- | --- |
| Language/profile | New keyword/type/operators; new vendor extension ID; no IEC 2013 claim for the extension; grammar, AST/HIR, sema, docs |
| Type system | Nullability, casts, aliasing, pointer arithmetic/bounds, conversions to/from `REF_TO`/interfaces/VAR_IN_OUT |
| Memory/roots | Pointers to split per-field objects are not meaningful; pointers into arrays/structs need rules; handles/place descriptors remain the aggregate representation |
| Online change | No `GotId`/layout identity for raw pointers; removed targets quarantine but are stale; migrated/resized targets dangle; pointer arithmetic makes target tracking impossible |
| Retained state | Cannot persist raw pointers; reset/reconstruct on warm start |
| Module unload/ABI | Must never point into module code/data or transient allocations; only stable host storage |
| Dynamic allocation | `__NEW`/`__DELETE` remain v2; v1 pointers could only target static/locals/params unless allocation is pulled forward |
| Testing | Parser/type/codegen/execution/negative tests plus online-change/state tests for any persistent pointer use |

Scope options:

1. **REF_TO only (current profile).** Safest for online change; no CODESYS pointer compatibility.
2. **Transient-only POINTER TO in v1.** Locals/parameters only, `ADR`/`^`/`NULL`, no persistent
   pointer fields, no arithmetic/indexing/casts, no dynamic allocation. Bounded risk and useful
   for porting helper code.
3. **Full POINTER TO in v1.** Persistent fields, arithmetic, casts, allocation. Large
   language/runtime/online-change scope; weak identity and migration guarantees.
4. **Alias POINTER TO to REF_TO.** Cheap but misleading; not true raw-pointer semantics.

Recommendation: if `POINTER TO` must be in v1, adopt option 2 (transient-only, no persistent
fields) and defer full pointer semantics to v2. If CODESYS compatibility is not a v1 necessity,
keep option 1 and defer the extension. Option 3 is not recommended for v1.

Resolved v1 scope (Q-131/Q-135–Q-138):

- `POINTER TO` is a **distinct type**, not an alias for `REF_TO`.
- It is **transient-only**: no pointer-typed globals, persistent fields, RETAIN leaves, or
  manifest roots. It may appear in locals, parameters, and (subject to Q-147) function results.
- It supports **full arithmetic, indexing, and `SIZEOF`**, not just `ADR`/`^`/`NULL`.
- It does **not** bring `__NEW`/`__DELETE`; dynamic allocation remains v2.
- Pointers are cycle-local: they cannot be stored in persistent roots, cannot enter the state
  file, and cannot cross an online swap or module unload.

Safety envelope (resolved unless noted):

- **Contiguous targets only (Q-143):** scalars, strings, scalar/string arrays, and `OVERLAP`
  roots. Split structs/FBs/classes have no contiguous object representation and are not raw
  pointer targets.
- **`ADR` scope (Q-144):** locals/parameters, leaf field roots, scalar/string array elements,
  string internals, and `OVERLAP` regions. Not split aggregate objects or arithmetic across field
  roots.
- **Arithmetic semantics (Q-145):** typed stride, unchecked like CODESYS, one-past-end allowed,
  crossing a root/array boundary undefined, null dereference traps. No bounds metadata.
- **Escape prevention (Q-146):** no implicit pointer conversions; explicit `POINTER TO` ↔
  `REF_TO` only; no pointer↔integer; sema rejects pointer-typed persistent declarations.
- **Functions (Q-147):** pointer parameters and results are allowed with documented lifetime;
  pointers to locals/transients must not escape the call.
- **`SIZEOF` (Q-148):** storage size of contiguous values only (elementary, string, array,
  `OVERLAP`). Split aggregate objects are rejected.
- **Milestone/profile (Q-149):** `STC-POINTER` vendor extension, implemented in M5; no IEC 2013
  claim for the extension.
- **Diagnostics (Q-150):** compile-time rejection of persistent pointers, null-deref trap, and
  documented undefined behavior for dangling/out-of-root pointers. No runtime registry.
- **Interop (Q-151–Q-161):** pointer→`REF_TO` may be allowed through a runtime resolver backed
  by GOT/layout metadata (NULL on failure); pointer→interface/handle remains disallowed;
  `REF_TO`→interface is allowed only for class/FB object references with the existing
  descriptor/`?=` check. See §6.16.

### 6.16 Pointer→`REF_TO` runtime validation (proposed)

The proposal: a cast from `POINTER TO T` to `REF_TO T` calls a host resolver, for example
`stc_rt_resolve_reference(pointer, target_type_key, target_layout)`.

Resolver checks:

1. The pointer is non-null.
2. The address lies inside an **active host-owned persistent root**; module code/data, stack
   locals, parameters, and transient allocations do not resolve.
3. For an interior address, the manifest's layout descriptors map it to a declared array
   element, string character, or `OVERLAP` typed view; an undeclared offset does not resolve.
4. The resolved object's type/layout matches the reference target type/layout. No silent
   reinterpretation; explicit reinterpret rules would need a separate decision.
5. The root is active, not retired or quarantined.

Result: a validated reference, or `NULL` when any check fails. Dereferencing `NULL` traps.

Remaining design consequences:

- **Validity is point-in-time.** A reference validated now can later target a root that is
  removed, migrated, or resized. Removed targets quarantine and stay memory-safe but stale;
  pointer-bearing roots remain reset/reject during migration.
- **Provenance.** A raw reference value does not remember its target `GotId`. This is acceptable
  under conservative pointer-bearing rules, but precise preservation/remapping would need a
  `{GotId, offset}` reference representation or runtime side table (Q-158/Q-160).
- **Transient targets.** Stack/locals/transient allocations do not resolve. If the user wants
  them to resolve, a separate transient-pointer registry and lifetime tag are required (Q-155).
- **Direct `AT` variables.** They are process-image metadata, not GOT roots. Whether the resolver
  can map them to references is a separate choice (Q-155).
- **Cost.** The runtime builds a range/type index from active roots and manifest layout
  descriptors; each conversion performs a lookup. No per-dereference cost if the resulting
  reference is a raw pointer/handle.
- **Safety.** The resolver never returns a pointer outside a registered active root, so a
  successful cast cannot fabricate arbitrary memory. Invalid input yields `NULL`, not a trap.

Resolved by Q-155–Q-161 (see §13.0). This replaces the earlier blanket "disallow" recommendation.

### 6.17 Place-descriptor reference model (Q-155/Q-158)

Q-155 resolved that the resolver also accepts direct `AT`/process-image variables. Q-158 resolved
that validated references carry a **place descriptor** (`{GotId, offset}`-style), not just a raw
pointer. This supersedes Q-126's "raw pointers for stable leaf storage" for validated references
and must be reconciled with ordinary `REF(place)` references.

Proposed tagged descriptor:

```text
Reference = Null
          | Root   { got_id, offset, type_key }
          | Direct { direct_id | area, offset, type_key }
          | Stack  { frame_id, offset, type_key }      // if locals/params are allowed
          | Object { object_handle, descriptor }       // if REF_TO FB/class uses handles
```

Implications:

- **Dereference** resolves the descriptor through the GOT/process-image/object registry, then
  applies the offset. This costs a lookup per dereference unless a generation-checked cache is
  stored in the reference.
- **Pointer→REF_TO** returns a `Root` or `Direct` descriptor when the resolver finds an active
  target; otherwise `Null`. `^` on `Null` traps.
- **`REF(place)`** can produce the same descriptor directly from compiler-known place identity;
  ordinary references and pointer-derived references then share one representation.
- **Online change** can remap `Root`/`Direct` descriptors by identity/offset instead of treating
  all reference-bearing roots as opaque. Removed targets resolve to null or quarantined storage;
  type/layout changes resolve to null/reset/reject.
- **Stack references** are the hard case: a local/parameter has no `GotId`. The options are to
  disallow `REF_TO` to stack storage, add a `Stack` descriptor valid only during the call, or
  keep a raw pointer tag for stack-only references.
- **Object/interface references** need either an `Object` kind or a `Root` descriptor plus
  runtime handle/descriptor lookup. `REF_TO`→interface conversion then uses the object
  descriptor; pointer→interface remains disallowed.
- **State file:** descriptors with stable `GotId`s are *potentially* persistable/reconstructible,
  unlike raw pointers, but Q-134 currently excludes all references. A later state-format
  revision may persist `Root`/`Direct` descriptors.
- **ABI/performance:** the reference value grows from a pointer to a descriptor; dereference and
  assignment become host-resolvable operations. Reference copies/assignments are descriptor
  copies. The host resolver returns descriptors, not raw pointers.

Resolved details (see §13.0):

- Q-162: one uniform place descriptor for all persistent-capable `REF_TO`.
- Q-163: `Stack` descriptors are valid only during the call.
- Q-164: stable `DirectId` in the manifest plus area/offset/type for `AT` references.
- Q-165: `Object { handle, descriptor }`; `REF_TO`→interface uses the descriptor check.
- Q-166: dereference resolves the descriptor; an optional generation-checked cache is allowed.
- Q-167: references are not persisted in the v1 state file; a later state format may persist
  `Root`/`Direct`.
- Q-168: remap `Root`/`Direct`; never `Stack`; `Object` follows its descriptor.
- Q-169: host resolver API/index with exact type matching (with Q-159/Q-161).

### 6.18 Nullability and `REF_TO`→interface conversion

The current profile keeps `NULL` assignable to references, interfaces, and classes (D-069;
Architecture §4.4 represents a null interface as `{null, null}`). `?=` is an assignment attempt
and can yield a null interface. Therefore:

- An interface value **can be null**. A null `REF_TO`/object reference converting to an interface
  yields a null interface; it is not a type error merely because the source is null.
- `REF_TO`→interface conversion is valid only when the reference's static target is a class/FB
  object type. The runtime checks the object descriptor implements the interface. Null stays
  null; an incompatible object yields null under attempt semantics (or a trap under a hard-cast
  policy, Q-171).
- No mandatory `IF __ISVALIDREF() THEN` block is required. The program must handle a possibly
  null interface before using it, exactly as for any nullable interface. `__ISVALIDREF` is a
  CODESYS vendor helper, not part of the current profile; `ref <> NULL`, `?=`, and the
  null-dereference trap are the v1 tools. A helper can be added as a vendor extension (Q-172).
- If the profile instead required **non-null interfaces**, a null reference→interface conversion
  could not produce a valid interface: it would have to trap or be statically guarded. That is a
  separate type-system/profile decision (Q-170).
- `POINTER TO`→`REF_TO` of an **object** type is not the same as pointer→`REF_TO` of contiguous
  storage. Objects are handle-based and split into field roots; a pointer to a field root is not
  the object. Pointer→Object should be disallowed or resolve only to an explicit object-handle
  cell, not an arbitrary field (Q-173). `REF_TO Object` is normally produced by `REF(object)`
  or an object expression, not by a raw-pointer cast.

Resolved by Q-170–Q-173 (see §13.0).

### 6.19 Double-buffered GOT activation and real-time impact

The proposed reload sequence is a **double-buffered GOT** activation:

1. Load the new module's functions and bind GOT data.
2. Build a new temporary GOT from the active GOT plus the new module's bindings.
3. At a cycle boundary, switch the active GOT pointer from normal to temporary; the next cycle
   runs the new code.
4. Observe the new version. On failure, switch back to normal, drop the temporary GOT, and unload
   the new module. On success, keep the old module until commit.
5. Copy the temporary GOT to the normal GOT.
6. At the next boundary, switch the active pointer back to normal and retire the old module.

Assessment: this is a valid refinement of the plan's activation. It has important requirements:

- **Whole-module binding (Q-176).** Every function slot in the temporary GOT must point to the
  new module, not to old module code. Otherwise the old module cannot be unloaded safely. A
  delta/reuse model needs refcounts and is not a v1 path.
- **Shared preserved data (Q-177).** Temporary and normal GOTs must point at the same stable
  storage for preserved roots. The temporary GOT adds new roots and leaves removed roots
  quarantined. Prepare must not write preserved roots while old code can still observe them.
- **Concurrent old/new execution (Q-178).** A single global GOT pointer cannot serve old code and
  new prepare code at the same time. If prepare runs new code before the switch, each module
  needs its own GOT context (per-module slot table/pointer), or prepare must run only in a
  stopped boundary window. The plan's per-module slot table supports the former.
- **Copy-back is not free (Q-179).** Copying temp→normal is O(GOT entries). Do it in a
  background/loader thread while the temp GOT is active; then switch to normal at a boundary.
  Serialize a second reload behind the copy.
- **Old/new overlap and unload (Q-181).** Keep the old module and normal GOT until the new
  version completes a successful cycle. Unload off-thread after commit/grace; never unload while
  any active GOT entry or reference points into it.
- **Failure/rollback (Q-180).** Recoverable load/prepare failures switch back to normal. A trap
  after activation follows the existing fatal-trap policy; if the host can recover, roll back to
  normal while the old module is still resident. If the process exits, rollback is moot.
- **Real-time boundary (Q-182).** The switch is one atomic pointer store (release/acquire) at the
  cycle boundary. Module loading, GOT construction, root initialization, copy-back, and unload
  are O(N) and must be off the cycle thread or outside the timed cycle. The first new cycle can
  still see cache/TLB and page-fault effects; "no real-time impact" should mean bounded
  maintenance jitter with heavy work off-thread, not zero cost.
- **I/O/clock/task state.** Stage I/O configuration, task guards, and clock changes in the
  temporary generation. Do not mutate shared host state that the old cycle can observe until
  activation.
- **Memory ordering.** Build the temporary GOT completely, publish it with a release store, and
  have the cycle thread acquire it at the boundary. No allocation, file I/O, or loader calls in
  the cycle path.

This model is adopted (Q-174). It supersedes the plan's earlier in-place function-cell update plus
snapshot for the normal activation path. The loader thread builds a temp GOT generation; the
cycle thread only performs the O(1) boundary switch. The protocol in §6.5 gains an explicit
temporary/normal GOT generation and a background loader/prepare phase.

Resolved details: Q-175 background loader thread; Q-176 whole-module function binding; Q-177
shared preserved roots; Q-178 loader-thread prepare with its own temp context; Q-179 background
copy-back; Q-180 one-successful-cycle rollback window; Q-181 off-thread unload after grace;
Q-182 bounded maintenance jitter with an O(1) switch.

Resolved by Q-174–Q-182 (see §13.0).

### 6.20 Non-real-time services and thread model

The runtime may also run non-RT work on service/monitor threads: variable access, debug access,
diagnostics, status, logging, manifest download, and state-file I/O. The model has three thread
classes:

| Thread | Responsibilities | Must not |
| --- | --- | --- |
| Cycle thread | Execute `stc_cycle`, latch clock/inputs, publish outputs, perform the O(1) GOT generation switch | Block on non-RT locks, allocate, load/unload modules, perform file/network I/O, or wait for service threads |
| Loader/prepare thread | Load/validate modules, build temp GOT generations, prepare new roots, copy back, unload retired modules, build manifest indexes | Mutate preserved roots or shared host state visible to the active cycle |
| Service/monitor threads | Typed variable reads, queued writes/forcing, diagnostics, status snapshots, logging, download/export | Hold raw pointers across a generation change, block the cycle, or access unregistered memory |

Synchronization rules:

- **Generation pinning / RCU.** Service readers pin the active GOT generation (epoch/refcount),
  resolve place descriptors through it, and release. The loader may unload a retired module only
  when no reader pins a generation that references it. Readers retry if the generation changed
  while they held a value.
- **Reads.** Variable/status reads should use typed paths/`GotId`s and pinned snapshots, not raw
  pointers. Aggregates read as a consistent snapshot where possible; otherwise document
  per-field tearing.
- **Writes/forcing.** Direct concurrent writes into cycle storage create data races and torn
  aggregates. Queue write/force requests for application at a cycle boundary, or use an explicit
  stop-the-world debug mode. Do not let service threads write shared roots while the cycle runs.
- **Status/diagnostics.** Use lock-free counters, double-buffered snapshots, or bounded
  ring buffers. Drop-oldest or coalesce under pressure; never block the cycle. Trap/last-error
  state is a host-owned snapshot.
- **Debug access.** Breakpoints, stepping, and forcing need cycle suspension or instrumentation.
  They are a separate debugger feature, not a side effect of variable access. Q-183/Q-186 put
  full variable and debug access in v1.
- **Swap interaction.** Service readers may hold a descriptor across a swap; generation pinning
  keeps the referenced module alive until the read completes. A write queued across a swap is
  rejected or retried against the new generation; it must not target a removed field.
- **Security/ABI.** Expose typed path/`GotId` access, not raw memory. Authenticate/authorize
  external access, rate-limit requests, and enforce timeouts. A debug raw-memory API, if ever
  added, is a separate privileged mode.
- **Real-time isolation.** Non-RT threads run at lower priority and must not cause unbounded
  locks, allocator contention, or memory-bandwidth stalls in the cycle. Use bounded queues,
  CPU affinity/priority policy, and measure contention (Q-190).

Resolved by Q-183–Q-190 (see §13.0).

#### Full debug v1 scope (Q-183/Q-186)

Q-183 and Q-186 selected **full variable and debug access in v1**. This is a scope expansion
that conflicts with the current Project Plan v1 non-goal and with Architecture §4.11 ("full
variable/type debug info is a later milestone"). It requires explicit reconciliation.

Full debug implies:

- typed variable read/write/force and watch;
- breakpoints, stepping, call stack, and source-level location mapping;
- a debug control protocol/session model with authentication and defined stop behavior;
- debug information for variable locations/types: full DWARF/CodeView in debug builds, plus
  source maps and manifest-assisted split-object views (Q-196/Q-202/Q-203);
- breakpoint/checkpoint codegen and an execution-control mode (suspend/resume/step);
- forcing semantics across cycles, online change, and state/retain;
- stop behavior: hold last outputs indefinitely (Q-201); the project makes no safety claim and
  the watchdog must be paused/disabled for a stopped session (Q-207);
- interaction with online change: every swap ends the debug session, clears breakpoints and
  forces, and requires the debugger to reconnect to the new generation (Q-199/Q-211);
- new milestones, DoD entries, tests, and documentation; removal or revision of the v1
  online-debugging non-goal.

Build default (Q-202): debug info is part of normal builds; `--release` is the optional
optimized/stripped production mode. Debug and release artifacts need distinct build IDs/manifests
and defined symbol/stripping rules (Q-208/Q-209).

The architecture must remain safe under these additions: the cycle thread cannot block on debug
locks; breakpoint checks must be disableable; forced writes must be boundary-applied; a stopped
cycle holds outputs by design rather than claiming safety; and debug sessions must not prevent
module unload indefinitely.

Questions Q-191–Q-200 define the exact v1 debug surface, breakpoint/stepping mechanism, forcing,
debug info, protocol/security, performance, online-change interaction, and milestone placement.

## 7. Impact on completed code (M1–M4)

### 7.1 Summary by crate

| Crate / area | Impact | Notes |
| --- | --- | --- |
| `stc-span`, `stc-diag`, `stc-lexer`, `stc-syntax`, `stc-parser` | None | No language or syntax change |
| `stc-hir` | Low | Stable-key/qualified-path helpers and catalog-facing identity data; arenas and node shapes mostly unchanged |
| `stc-sema` | Low–medium | Export/signature validation, initializer and retention invariants, online-change-facing diagnostics; no parsing/type-system change |
| `stc-codegen` | High | `GotId`/manifest resolution, GOT slot assignment, indirect calls, descriptor/manifest emission, online-change plan, module mode, all global/instance access, init/migration modes, exports, metadata |
| `stc-runtime` | Very high | Split into host, GOT, allocator, registry, loader, manifest store/download, plan execution, swap state machine, descriptor validation, platform layer, state interaction |
| `stc-driver` | Medium–high | New artifact kinds and link commands, baseline manifest discovery/input/output, rename map, module naming, host/run/download command wiring, metadata |
| Tests and snapshots | High | All IR snapshots and execution harnesses change; new hot-swap, manifest, rename, and migration suites |
| Documentation | High | Architecture, runtime, state, I/O, CLI, testing, project plan, decisions, README |

### 7.2 `stc-hir` and `stc-sema`

No language change. The likely additions are:

- A build-level identity/rename layer. It can live in the driver/codegen rather than HIR, but
  the compiler must be able to correlate a new declaration with a baseline manifest entry by
  its normalized key, kind, signature, and explicit rename map.
- Source-aware rename support needs enough syntax/CST information to identify declaration names
  and references. The M2 CST already provides lossless spans; a `stc rename` command can be
  added without changing the language or the HIR node shapes.
- HIR may need stable accessors for the qualified path and package name used in manifest keys.
  `Def.parent`, `Def.package`, and `PackageInfo` already contain the necessary structure; the
  helper can live in codegen to keep HIR free of output concerns.
- If the catalog needs package names in a form that is not derivable from `PackageInfo`, add a
  package-name field or accessor. Otherwise HIR is read-only.
- Sema should validate export signatures and any online-change-relevant invariants that can be
  checked at compile time. Cross-build compatibility is a compiler comparison against the
  baseline manifest, not a semantic rule.
- Sema must keep initializers constant-foldable for descriptor-driven initialization, or the
  generated `prepare` function must contain the initializer code. The existing D-067 constant
  rule already supports the former.

### 7.3 `stc-codegen`

#### `lib.rs`

- Add `Mode::Module` (or a separate artifact kind) and options for host ABI, build ID, baseline
  manifest, manifest output path, and module identity.
- `Output` gains the binary descriptor, the GOT manifest, and the online-change plan or the
  structured data needed by the driver to write them.
- Keep `generate(&HirCrate, &Options) -> Result<Output, Vec<CodegenError>>` as the public
  contract; extend `Output` rather than adding a second generation entry point.

#### `module.rs`

- Add typed indirect calls with explicit function signatures.
- Add module descriptor globals/arrays and string tables for stable keys.
- Add `dllexport`/visibility control for descriptor and entry functions.
- Keep the structural verifier; extend it to check that forbidden direct `@stc_g_*`,
  `@stc_pi_*`, and user-function references do not appear outside allowed bootstrap/descriptor
  contexts.
- Keep deterministic ordering and temp counters.

#### `mangle.rs`

- Keep the existing function/global symbol mangling for code symbols.
- Add stable key functions: qualified variable path, qualified function key, type key, instance
  key, task key, export key, and build ID.
- Add manifest `GotId` assignment/lookup helpers and history-key normalization.
- Add descriptor, slot-table, manifest, and migration-table symbol names.
- Ensure keys are case-normalized and package-qualified.

#### `layout.rs`

- Keep `ScalarKind` for the M4 surface; M5 extends it to `StorageType`/`TypeLayout`.
- Add root layout descriptors: type key, structural layout hash, size constant, alignment.
- Compute sizes through LLVM constant expressions rather than hard-coded target layout. The
  compiler knows the LLVM type spelling; it does not need target datalayout.
- Extend canonical type names to cover aggregates/strings/references in M5 for key hashing.

#### `config.rs`

- Replace `PlanInstance.global` with stable instance/object handles and a per-field root catalog.
- `ProgramLayout` becomes a field-root catalog plus an object descriptor. There is no contiguous
  program struct to GEP through; field access resolves field `GotId`s. Arrays/strings/`OVERLAP`
  follow Q-123–Q-125.
- `ImageLayout` drives host reconfiguration; the module no longer caches the image base in a
  module-owned global.
- `RetainLayout` keeps per-field stable paths and offsets; root access goes through GOT cells.
  Add compatibility metadata (per-leaf type key/layout hash) if the state format needs it.
- Add catalog assembly: field roots, functions, types, object handles/descriptors, instances,
  tasks, exports, init masks.
- Add manifest assembly and baseline comparison: resolve each field against the baseline
  manifest, carry `GotId`s, apply tracked renames/moves, classify preserve/allocate/migrate/
  reset/drop/reject, and produce the online-change plan.
- Add migration-plan assembly and, when in scope, migration-function selection/data generation.
  Old layout offsets come from the baseline manifest, not from the old module.

#### `expr.rs`

- `read_place`, `write_place`, and `place_ptr` load the field root pointer from the data cell for
  `Global`, `InstanceField`, and `This` places. There is no GEP from a parent object; field
  access is a `GotId` lookup.
- `THIS`/`SUPER` carry object handles; scalar/string/array field references may use raw
  pointers; aggregate/object references use handles/place descriptors (Q-126).
- `global_address`/`field_address` direct-variable paths use the image base cell and image
  offsets; no per-variable module globals remain.
- `lower_call` and `lower_call_statement` resolve calls through function cells and emit indirect
  calls. Output/`VAR_IN_OUT` argument handling (M5) passes pointers or handles as Q-126 requires.
- `trap_if` calls the host trap service through the host API table.
- Compiler helpers (`stc_pow_*`) and LLVM intrinsics may remain direct; they are version-local.
- `Read`/`write` of module-owned constant data may remain direct; no persistent pointer may
  escape from it.

#### `stmt.rs`

- No semantic change. Call statements follow the indirect-call path; all places already route
  through `expr.rs`.

#### `unit.rs`

- `Lowerer` replaces `function_symbols`/`global_symbols`/`instance_globals` with slot maps plus
  catalog keys. Symbols are still needed for descriptor initializers and code definitions.
- `build_symbols` assigns deterministic slots, resolves `GotId`s against the baseline manifest,
  applies tracked renames, and rejects duplicate keys/identities.
- `emit_types_and_globals` no longer emits persistent roots or instance globals. It emits the
  slot table, descriptor, read-only constants, migration entry points, and module entry points.
- `emit_function` keeps internal code symbols; the descriptor exposes addresses. Function bodies
  are otherwise unchanged.
- `emit_program_body` remains internal; calls to it go through a function cell.
- `emit_init_body` becomes `prepare` with a mode and fresh-root mask:
  - online attach: initialize only new roots; do not load retained state;
  - cold start: initialize all roots and apply retained-state load if configured;
  - reset: initialize selected/all roots.
- `instance_field_pointer` is replaced by field-root lookup through the field `GotId`; object
  handles are opaque and do not provide a contiguous base for GEP.
- `emit_cycle_body` calls program bodies through function cells; task guards come from host
  cells or the host API.
- `emit_shutdown` calls host state-saving services with the active module's retain data.
- `emit_retain_functions` accesses roots through cells; the retain buffer is host-owned.
- `emit_export_wrappers` either disappears (host export registry calls function cells) or becomes
  a thin direct-call wrapper for external embedding. The online-change path must use the host
  export registry.
- `emit_main` moves out of the module. Executable mode becomes a host launcher with a module
  path (Q-020).
- `build_metadata` adds the catalog, build ID, target/profile, slot map, compatibility hashes,
  module ABI, manifest digest/path, and online-change plan summary.
- Manifest emission is a separate deterministic projection of the same catalog model; module
  descriptor and manifest must not be maintained as independent schemas.

#### `abi.rs`

- Introduce `STC_HOST_ABI = 2` (or a separate module/host ABI pair) and the module entry names.
- Add host API table indices for trap, clock, I/O, state, math/string helpers, and diagnostics.
- Keep trap code values stable.
- Document that M4 ABI-1 artifacts are not loadable by the host.

#### `metadata.rs`

- Add module header, build ID, slot map, root catalog, function catalog, type/vtable catalog,
  instance/task catalog, export catalog, manifest digest, and compatibility summary.
- Bump the metadata schema version.
- Keep JSON sorted and deterministic; the binary descriptor remains the loading contract and the
  GOT manifest remains the identity/history contract.

### 7.4 `stc-runtime`

The runtime grows from a 319-line cycle loop into the host platform. Proposed modules:

```text
crates/stc-runtime/src/
  lib.rs          host API, ABI version, panic boundary
  got.rs          cells, slot tables, stable cell registry
  memory.rs       root registry, allocation, quarantine; transient module allocations (alloc/free)
  catalog.rs      descriptor and manifest parsing/validation
  loader.rs       platform load/unload abstraction
  module.rs       module lifecycle and entry calls
  manifest.rs     active manifest, runtime layout record, download/export
  migration.rs    online-change plan validation and execution
  swap.rs         compatibility classification, prepare, activate, rollback
  state.rs        process image, clock, retained-state integration
  platform/       Windows and Linux loader/clock/file operations
```

Specific changes:

- `stc_rt_execute` becomes a host-loop API or is replaced by `stc_host_*`; the existing function
  signatures are part of ABI 1 and cannot be silently reinterpreted.
- The global `Mutex<State>` becomes a host state with module registry, GOT, root registry,
  process image, clock, I/O, and state-file configuration.
- `stc_rt_trap` remains the trap entry, but generated calls reach it through the host API table.
- `stc_rt_io_configure`/`stc_rt_io_image` remain host services; generated code uses cells rather
  than module globals.
- Retained-state load/save are host-driven around module init/shutdown, not called blindly by
  every module init.
- `stc_rt_assert` (M6) is a host service.
- Add module load/prepare/activate/rollback/unload and status APIs for the host binary, tests,
  and embedding hosts.
- Add the internal allocation/rebinding service: stable zeroed host-owned blocks, per-module
  ownership tracking, free-on-failure/retire/unload, and validation that dynamic pointers never
  enter persistent roots or cross a swap. No GC and no ST-visible dynamic pointers in v1.
- Add manifest storage and download APIs: persist the active manifest and runtime layout record,
  pair them with the active module digest, export a self-contained baseline artifact, and reject
  missing/stale/corrupt manifests in online mode.
- Add online-change plan validation and execution: preserve/allocate/migrate/reset/drop,
  migration function dispatch, rollback snapshots, and quarantine bookkeeping.
- All loader calls and foreign pointer accesses stay in `stc-runtime`, the only crate allowed
  unsafe code (D-077).

### 7.5 `stc-driver`

- Add `--emit=module` and make the module the primary online-change artifact. A module emission
  always writes its paired GOT manifest; an explicit `manifest` emit kind may be added for
  tooling-only generation.
- Add GOT manifest output and input options: default discovery next to the artifact/source,
  explicit `--got-manifest <path>`, `--cold`/`--no-baseline` for deliberate cold builds, and a
  recorded baseline digest in the new manifest.
- Write the module and manifest as a validated pair only after a successful codegen/link, or
  mark the build incomplete. A failed build must not leave a manifest that looks deployable.
- Add a rename/move option and, later, a source-aware `stc rename` command that rewrites source
  references and records identity history.
- Add a manifest download/export command or host-API route for `stc online download`, plus a
  compatibility report/diff for pre-deployment review.
- Build modules with `clang -shared -fPIC`, correct visibility/export flags, and no link to the
  generated executable's runtime. The module imports host services through the API table, not
  through linker symbols.
- Produce immutable, versioned module filenames and a sidecar metadata file.
- Define `exe` behavior when a module host exists: recommended as a host launcher that loads an
  adjacent module, not as a code-linked executable. This is Q-020/Q-022.
- Keep `lib` for host embedding; clarify whether it contains a module, a host, or both.
- Add `stc run` (M6) options for module path, cycles, clock, I/O, watch/reload, and state.
- Update artifact discovery, link diagnostics, exit codes, and help text.
- Add tests for module naming, descriptor output, load rejection, and CLI behavior.

### 7.6 Tests, snapshots, and fuzzing

- All existing IR snapshots change: direct globals and calls become slot-table/cell sequences.
- `crates/stc-codegen/tests/codegen.rs` assertions that search for `@stc_pi_*`, `@stc_g_*`,
  `call void @stc_fn_*`, `@stc_io_base`, and `@stc_retain_buffer` must be rewritten around
  descriptor/slot checks and the structural no-direct-reference invariant.
- `crates/stc-codegen/tests/snapshots/library.ll` is replaced by module-mode snapshots plus
  descriptor snapshots.
- `crates/stc-driver/tests/execution.rs` and `tests/harness/*.c` must exercise the host API,
  module load, cycle counts, I/O, and exports.
- Add a hot-swap suite: build v1 and v2, load v1, run cycles, load v2, assert preservation,
  activation boundary, old-module unload, and rollback on rejection.
- Add manifest tests: first-build identity assignment, second-build reuse, tracked rename/move,
  untracked-rename warnings, missing/stale/tampered/non-current baseline forcing cold download,
  download/import round-trip, and cross-developer build from a downloaded baseline.
- Add cold-download tests: non-retained state resets, retained state follows the existing
  compatibility policy, the new manifest becomes active, and a subsequent online change works
  from the new baseline.
- Add migration tests when migration lands: field addition/removal, same-size in-place copy,
  lossless widening, pointer-bearing rejection/reset, rollback after migration failure.
- Add negative tests for layout change, export signature change, vtable shape change, task
  topology change, target/ABI mismatch, malformed catalogs, and duplicate keys/`GotId`s.
- Add determinism tests for descriptor bytes, slot assignment, and build IDs.
- Add a structural IR test that fails on a direct persistent-global or user-function reference.
- Extend `fuzz_codegen` to include catalogs/slot assignment; add a loader/catalog fuzz target if
  it can run without a real shared library.
- Keep `STC_REQUIRE_CLANG=1` and the Windows/Linux matrix; add hot-swap jobs to both hosts.

### 7.7 What does not change

- Lexer, parser, CST/AST, tokenization, grammar, and source-level diagnostics.
- Type checking and constant-evaluation semantics.
- HIR expression/statement shapes for the M4 surface.
- Scalar arithmetic, comparison, control-flow, and trap semantics.
- Target selection through clang and the absence of hard-coded datalayout.
- Deterministic-emission principles.
- The M4 limit on one task per resource and one active configuration, until M6 scheduling work.

## 8. Impact on planned work (M5–M8)

### 8.1 M5 — FBs, OOP, aggregates, strings, standard library

M5 must be implemented on the GOT/module model from its first line. The following M5 areas gain
extra requirements:

| M5 area | Added requirement |
| --- | --- |
| Type lowering and layout | Every persistent field root needs a stable type key/layout hash; arrays/strings/`OVERLAP` have explicit representation rules (Q-123–Q-125) |
| Structs/arrays/strings | Persistent fields are separate roots; objects are handle-based; scalar/string arrays are one root; aggregate arrays use per-element handles; `OVERLAP` is atomic |
| References | Raw pointers for stable leaf storage; handles/place descriptors for aggregate/object references; removed roots quarantine |
| `VAR_IN_OUT` | Scalar/string/array parameters point at stable storage; aggregate/object parameters carry handles; legal only for the call duration |
| Classes/FBs/interfaces | Stable handles plus runtime-owned descriptors/vtables of cell pointers; no contiguous object pointer; preserved instances require compatible field/descriptor identity |
| FB invocation | Instance handle plus per-field roots; input/output binding through field roots; nested instances are handle/field trees; state preserved per field |
| `THIS`/`SUPER` | `THIS` is the stable object handle; `SUPER` uses the base descriptor/handle; field access resolves per-field roots |
| `?=` | Runtime descriptor lookup with cross-version identity rules; handles remain stable for preserved instances |
| Standard functions | Inline code stays version-local; runtime helpers are host services accessed through the host API table/GOT |
| Standard FBs | Compiled into each module with stable reserved-namespace keys; timer state lives in preserved field roots |
| Retain | Per-field leaves, packed offsets, host-owned buffer, online-change compatibility rules |
| Exports | Signature-stable export registry through cells; no raw wrapper pointers held by the host |
| Tests | Every M5 execution test runs through the module host; new swap tests cover per-field preservation, handles, arrays/strings/OVERLAP |
| Manifest | Every persistent field gets a `GotId`, path, type record, and owner/handle link; field migration is not in v1 (§6.4.4) |

M5 must not reintroduce module-owned vtables, direct persistent-global references, or a second
init path that reloads retained state during online attach. Those would be rework.

### 8.2 M6 — Project system, CLI, state, I/O, tests

- Manifest gains runtime/deployment configuration: active baseline location, download/import
  route, `on-incompatible`/migration policy, and state-file interaction.
- `stc run`/`stc test` use the host + module path; `stc deploy` or `--watch` triggers reloads.
- File-backed retained state becomes real; online attach must skip state load. A state-format
  revision can key retained leaves by `GotId` for rename/migration continuity.
- I/O device tree and bindings are re-evaluated on module prepare; process-image reconfiguration
  is part of the swap.
- The test runner uses the same host API and can exercise swap scenarios.
- Metadata JSON schema and `stc metadata` expose the module catalog, manifest digest, active
  baseline, and compatibility status.
- `stc online download`/import, `stc online diff`, and the source-aware rename command land
  here if they are not in M4.5.

### 8.3 M6.5 — Debug and online monitoring

- Typed variable read/write/force and watch through generation-pinned descriptors.
- Breakpoints, stop-the-world stepping, call stack, and source-level locations.
- Debug protocol/session lifecycle, authentication/authorization, audit, and production policy.
- DWARF/CodeView debug info; debug is the default build mode; `--release` strips/optimizes;
  sidecar symbols and reproducible debug info.
- Forcing applied at cycle boundaries and held until release; session end clears forces and
  breakpoints.
- Stop behavior: hold last outputs indefinitely and pause/disable the watchdog; the project makes
  no safety claim.
- Online change ends the debug session; the debugger reconnects to the new generation.
- Generation pinning keeps retired modules alive for service readers but does not block unload
  indefinitely.

Exit criteria:

- A debugger can attach, set/clear breakpoints, step, inspect locals/globals/objects, force
  values, and detach on both supported hosts.
- Debug/release artifacts have distinct build IDs/manifests; release is stripped; debug symbols
  are reproducible and available for symbolication.
- A swap while debugging terminates the session cleanly and leaves no forced/held state.
- No debug work blocks the cycle thread; production overhead with debug disabled is measured.
- Type/field/debug info covers split per-field objects via the manifest-assisted debugger view.

### 8.4 M7 — Release candidate

- Performance measurements for GOT data and call indirection.
- Packaging of the host binary, module, manifest, and downloadable baseline.
- Documented deployment/rollback procedure, including tracked rename and migration policy.
- Reproducible module/manifest builds and build-ID/digest verification.
- Manifest format stability and compatibility-report review.
- Release notes and known limitations for online change.

### 8.5 M8 — Linux host portability

- Loader abstraction verified on Linux x86-64 and AArch64.
- Module and manifest naming, visibility, CRT/libm linkage, and unload behavior.
- Cross-build artifacts: a module or manifest built for a different target/profile than the
  host is rejected.
- The same hot-swap, rename, and manifest-download suite runs on both hosts.

### 8.6 Post-v1 — user-visible dynamic allocation

TwinCAT-style `__NEW`/`__DELETE` is a v2 vendor extension, not part of M4.5–M8 or the v1 ST
profile. Before it is scheduled it needs a separate design for:

- source syntax, the extension/profile ID, and conformance claims;
- pointer/reference lifetime rules and null/failure behavior;
- dynamic-object identity, retention, and online-change semantics;
- memory accounting, limits, fragmentation, and teardown;
- interaction with `REF_TO`, interfaces, `VAR_IN_OUT`, and retained state;
- whether dynamic objects can be tracked in the GOT manifest or are explicitly excluded.

The v1 internal allocation service is not a preview of this feature: it has no ST surface and its
allocations cannot escape a module version.

## 9. Milestone and sequencing recommendation

### 9.1 Options

| Option | Description | Consequence |
| --- | --- | --- |
| **A. Insert M4.5 (recommended)** | Add a foundation milestone after M4 and before M5: host/module split, GOT, root allocator, catalog, swap for the M4 scalar surface, tests | Bounded rework of M4 code now; M5 features are built once on the final model |
| B. Fold into M5 | Implement M5 features and GOT together | M5 becomes very large; higher risk of partial/inconsistent architecture |
| C. Defer to M7/M8 | Continue M5 with direct references, then retrofit | M5 OOP/vtables/aggregates/retain would be rewritten; highest total cost |
| D. Dual mode | Keep direct codegen and add an optional GOT mode | Two untested paths, snapshot/test matrix growth; not recommended |

### 9.2 Proposed M4.5 — Online-change foundation

Scope:

- Runtime host and loadable module artifacts for the M4 scalar surface.
- GOT cells, slot tables, root registry, allocation, quarantine.
- Internal host allocation/rebinding service for generated code: stable blocks, per-version
  ownership, free on failure/retire/unload, no persistent references, no ST surface.
- Module descriptor/catalog, host ABI, attach/prepare/activate/rollback/unload.
- Persistent GOT manifest emission/consumption: `GotId` assignment, baseline reuse, tracked
  rename/move maps, preserve/allocate/drop classification, and manifest/module pairing.
- Runtime manifest storage, active-layout record, exact-currentness gate, cold-download path, and
  export/download round-trip.
- Codegen conversion of every M4 global, instance, function call, I/O base, retain buffer, and
  task guard path.
- Compatibility classification for the M4 surface: code-only changes and new/removed scalar
  globals; exact-layout preservation; reject layout/type changes and unsupported migrations.
- Deterministic build IDs, descriptor snapshots, and manifest snapshots.
- Windows MSVC and Linux x86-64 hot-swap tests.
- Documentation updates listed in §12.

Proposed exit criteria:

1. `stc build --emit=module` produces a loadable module, metadata, and a GOT manifest for the M4
   surface; a cold build without a baseline still emits a complete manifest.
2. A second build consumes the first manifest, reuses every unchanged `GotId`, and emits a new
   manifest plus an online-change plan.
3. The host loads a module, runs cycles, loads a second module, preserves matching scalar roots,
   swaps at a cycle boundary, stores the new active manifest, and unloads the old module.
4. A code-only change preserves all matching state; a new global initializes; a removed global
   is quarantined with a warning.
5. A tracked rename preserves the `GotId` and value; an untracked rename is diagnosed and
   requires an explicit policy.
6. A layout/type change is rejected by default; the old version continues.
7. A missing, stale, tampered, unpaired, or non-current manifest selects cold download; no
   online-change recovery, merge, or partial attempt occurs.
8. The active manifest can be exported/downloaded and used as the baseline for another build,
   producing identical `GotId`s for matching declarations.
9. A failed prepare or load leaves the old version active and intact.
10. Internal host allocations requested by a module version are transient: reclaimed on failed
    prepare, retire, or unload; no persistent root or manifest entry may reference them.
11. Generated IR passes a structural test that no persistent root or user function is directly
    referenced.
12. Two builds of the same source and baseline produce identical IR, descriptors, manifest,
    online-change plan, metadata, and build IDs.
13. Existing M4 execution behavior is preserved through the host/module path, including traps,
    process image, exports, and scalar retain stubs.
14. `cargo test --workspace --locked`, clippy, fmt, docs checks, markdown lint, and fuzz smoke
    pass.
15. README, project plan, architecture, runtime, state, I/O, CLI, testing, decisions, and
    handover documents reflect the new model.

### 9.3 M5 exit-criteria additions

Add to the M5 definition of done:

- Every M5 persistent root and function entry is registered in the catalog and reached through a
  cell.
- Every M5 root, type, method, and instance has a stable manifest identity; tracked renames and
  moves preserve it across builds.
- OOP dispatch and `?=` use runtime-owned descriptors; no raw code pointers are stored in
  persistent memory.
- Aggregate/string/FB state survives a compatible online change and is reset, migrated, or
  rejected under the documented policy when its layout changes.
- The M5 execution suite includes at least one online-change case per feature family, including
  a manifest round-trip through the runtime export/download path.

## 10. Work breakdown for M4.5

| Phase | Work | Acceptance |
| --- | --- | --- |
| 0 | Resolve blocking questions in §13; record decisions and update the spec set | No open P0 question; docs checklist complete |
| 1 | Define stable keys, `GotId` scheme, catalog/manifest schema, online-change-plan schema, host API, module ABI, and build-ID rules | Reviewed design; descriptor/manifest/ABI sketches approved |
| 2 | Add compiler identity layer: baseline manifest parsing, `GotId` matching, tracked rename/move maps, preserve/allocate/drop classification, manifest emission | First/second-build identity tests; rename test; deterministic manifest snapshot |
| 3 | Extend the IR builder: indirect calls, slot-table access, descriptor globals, visibility, structural verifier | Codegen unit tests; clang accepts modules |
| 4 | Add codegen slot assignment, root catalog, manifest projection, and online-change plan emission for the M4 scalar surface | Deterministic slots; descriptor/manifest snapshots; plan review |
| 5 | Implement host GOT, root registry, allocator, quarantine, transient module allocation/rebinding service, descriptor/manifest validation, and plan validation | Runtime unit tests; malformed/tampered manifest rejection; allocations freed on failure/retire |
| 6 | Implement module loader, attach/prepare/activate/rollback/unload, active-manifest storage, exact-currentness gate, cold-download path, and export/download | Host loads a no-op module; manifest round-trip; missing/stale manifest forces cold |
| 7 | Convert globals, instances, calls, I/O base, retain buffer, task guards, exports, init modes | Existing execution tests pass through modules |
| 8 | Add compatibility classification and state preservation for scalar roots; plan-execution skeleton for preserve/allocate/drop; no field migration in v1 (type/layout changes reset/cold); verify cold-download state policy | Hot-swap/rename tests pass; rejection/cold paths behave as specified |
| 9 | Add test harnesses, negative cases, determinism, fuzz coverage, Windows/Linux CI | CI green on both hosts |
| 10 | Update all documentation and prepare the M5 handover addendum | Planning and implementation docs consistent |

No M5 feature work starts before phase 7 is stable, because M5 would otherwise multiply the
number of call sites and persistent roots that need conversion.

## 11. Verification plan

### 11.1 Structural codegen invariants

- No direct reference to `@stc_g_*`, `@stc_pi_*`, or a user function symbol outside the module
  descriptor/initializer data.
- Every persistent place lowers to a slot-table load, a cell load, and storage access.
- Every user/standard/method/FB call lowers to an indirect call through a function cell.
- Every emitted call signature matches the catalog signature key.
- Every root has exactly one catalog entry; every catalog entry has a unique stable key and a
  manifest `GotId`; the module descriptor and manifest are paired by digest.
- Every manifest entry is either referenced by the module or explicitly retired; no orphan
  identities are emitted.
- Slot assignment is deterministic and independent of arena order; `GotId`s are stable across
  builds with the same baseline.

### 11.2 Runtime unit tests

- Cell allocation and stable addresses.
- Slot-table construction and module-index remapping.
- Root matching, allocation, initialization, quarantine, reset.
- Descriptor validation for bad magic, version, target, counts, offsets, duplicate keys, and
  truncated data.
- Swap state machine: load, validate, bind, prepare, activate, commit, rollback, unload.
- Function-cell updates and rollback snapshots.
- Vtable/descriptor rebuild and interface-value stability.
- Process-image reconfiguration and clock continuity.
- Manifest parsing/validation: bad schema, digest mismatch, duplicate `GotId`, stale parent
  build ID, target/profile mismatch, and tampered plan.
- Exact-currentness gate: parent != active, missing/invalid/unpaired manifest selects cold and
  leaves the old version untouched.
- Active-manifest storage, export, atomic replacement on commit, and restoration on rollback;
  cold installation sets the new baseline.
- Plan execution: preserve/allocate/drop/reset, migration function dispatch, and quarantine
  bookkeeping.
- Transient allocation service: stable zeroed blocks, alignment, per-version ownership,
  reclamation on failure/retire/unload as resolved by Q-121, double-free/unknown-pointer
  rejection, and refusal to register a dynamic pointer as a persistent root.

### 11.3 Execution and hot-swap tests

Build at least two versions of a small program:

- v1: counter, timer-like state, exported function, `%M` marker.
- v2: changed function body, added global, removed global, added program instance.
- v2-renamed: tracked rename of a variable, a global, a type, and a method, with values
  preserved.
- v2-untracked: rename without a map, expected diagnostic and explicit cold path.
- v2-imported: build from a manifest downloaded from the running host, with matching `GotId`s.
- v2-layout: changed type/layout, export signature, vtable shape, task topology.
- v2-migrate: field migration is not in v1; type/layout changes reset or require a cold
  download. Migration tests are deferred with the feature.

Assertions:

- v1 runs, state reaches a known value.
- v2 activation happens only at a cycle boundary.
- Preserved roots keep values and addresses.
- A tracked rename keeps the same `GotId` and value; an untracked rename does not silently
  discard the old identity.
- A build from a downloaded manifest reuses the same `GotId`s for matching declarations.
- New roots receive defaults/initializers exactly once.
- Removed roots do not appear to new code and generate a warning.
- The old version remains active after a rejected module.
- A missing, stale, tampered, unpaired, or non-current manifest takes the cold-download path;
  no online activation occurs, and the old version remains active until the cold install.
- A cold install initializes non-retained state and applies the retained-state compatibility
  policy; afterward the new manifest is active and a subsequent online change from it works.
- Internal allocations made by a module version are reclaimed on failure/retire/unload and never
  appear in the manifest or a preserved root.
- A failed prepare or migration rolls back cells, descriptors, and the active manifest.
- Double-buffered activation: temp GOT built/bound/prepared off-thread; O(1) boundary switch;
  successful cycle; background copy-back; normal switch; old module unload; recoverable
  rollback switches back without disturbing the old cycle.
- Prepare with the temp GOT context writes only new roots; preserved-root values are unchanged
  until activation. Loader-thread work does not run in the cycle path.
- The old module unloads and no stale code pointer remains.
- Outputs at the boundary come from the completed old cycle; the next cycle is new code.
- Traps report the active module's source spans.

### 11.4 Determinism and fuzzing

- Two builds produce identical module IR, descriptor bytes, manifest bytes, online-change plan,
  slot assignment, metadata, and build ID.
- Build IDs and manifest digests are independent of paths, timestamps, and file order.
- A build with a baseline is reproducible only with the same baseline digest, which is recorded
  in the output.
- `fuzz_codegen` covers slot/catalog/manifest generation and never panics.
- A loader/catalog/manifest fuzz target exercises malformed descriptors and manifests without
  loading native code, or a constrained target with a fixed test module.

### 11.5 Performance

- Benchmark a representative scalar cycle and an OOP/FB cycle with direct calls versus GOT
  calls.
- Record the overhead of data-cell loads and indirect calls.
- Evaluate caching the slot-table pointer and root storage pointers within a function.
- Measure manifest generation/parsing, active-manifest storage, and download size on a
  representative large program.
- Do not trade correctness for inlining; if the overhead is unacceptable, revisit Q-062 rather
  than reintroducing hidden direct persistent access.

### 11.6 CI matrix

- Windows Server 2025 x64 MSVC: build, load, swap, unload.
- Ubuntu 24.04 x64 GNU: build, load, swap, unload.
- Linux AArch64 after M8.
- Cross-target: module/object emission only; host-load tests require a matching target.
- `STC_REQUIRE_CLANG=1` remains mandatory.
- Markdown lint and `check_docs.py` include any new specification document.

### 11.7 Debug and monitoring tests

- Attach/detach, authenticated commands, and unauthorized-command rejection.
- Breakpoint set/clear/hit, statement stepping, call stack, and source-location mapping.
- Typed variable reads/writes/forcing, generation pinning, and retry across a swap.
- Forced values held until release; session end/swap clears forces and breakpoints.
- Hold-last-outputs and paused/disabled watchdog while stopped; resume behavior.
- Debug/release build IDs/manifests, DWARF/PDB verification, stripping, sidecar symbols, and
  reproducible paths.
- Split per-field objects/handles through the manifest-assisted debugger view.
- Loader/service threads never block the cycle; measured production overhead with debug disabled.
- Online swap while debugging terminates the session cleanly and unloads the old module.

## 12. Documentation update checklist

**Status: applied 2026-10-24 (revision 36).** The table is retained as the record of what was
required and changed. Decisions A-022–A-030 and D-094–D-108 record the answers; the new
specifications and plans are linked from `README.md`.

| Document | Sections | Required change |
| --- | --- | --- |
| `README.md` | Status, Compilation model, CLI, Repository layout, Confirmed decisions, Question log, Build examples | Describe the host/module model, `--emit=module`, GOT manifest, online-change status, resolved decisions, and the question log |
| `Specification/00-Project-Plan.md` | §2 goals, §3 non-goals, §4 constraints, §6 architecture, §8 milestones, §9 verification, §10 risks, §11 definition of done, §13 post-v1 | Add online change as a goal, add M4.5 and M6.5, update architecture and risk table, add manifest/build-history requirements, record full debug/online monitoring and the non-safety stance, record the v2 `__NEW`/`__DELETE` extension in post-v1, remove "online debugging" ambiguity |
| `Specification/01-Conformance-Profile.md` | §1 status, §4 language surface, §7 out of scope, §8 implementation-defined choices | State that online change, the GOT manifest, and debug/monitoring are runtime/platform capabilities, not language extensions; record the non-safety claim, debug/release defaults, compatibility, rename, migration, and preserving policies |
| `Specification/Architecture/Architecture.md` | §4 code generation, §4.9 globals/init/main, §4.10 mangling, §5 packages, §6 backend, §7 runtime boundary, §8 security, §9 open questions | Add host/module split, GOT, root allocation, descriptors, manifest identity/history/migration, module ABI, loader, download, compatibility, determinism |
| `Specification/Architecture/Runtime-and-Execution.md` | §1 execution model, §1.1 executable, §1.2 library, §1.3 tasks, §2 runtime, §2.2 ABI, §2.3 core ABI, §2.4 process image, §2.5 clock, §5 retain, §6 traps, §7 tests | Rewrite around the host; add GOT/memory/module/manifest/swap sections; define init modes, plan execution, manifest download, exact-currentness/cold-download gating, transient allocation service, and state interaction |
| `Specification/Architecture/State-and-Retain.md` | §1 scope, §2 leaves, §3 format, §4 lifecycle, §5 ABI, §6 metadata, §8 tests | Add online attach vs cold start, `GotId`-aware identity, layout-hash behavior, quarantine, and retained-root compatibility/migration policy |
| `Specification/Architecture/IO-and-Devices.md` | §1 split, §2 image, §2.2 unbound, §5 metadata, §6 runtime behavior | Add image reconfiguration during prepare, binding re-evaluation, direct-variable preservation, and swap-boundary I/O publication |
| `Specification/Architecture/CLI-and-Manifest.md` | §1 commands/options, §1.3 exit codes, §2 manifest, §4 artifacts/metadata | Add `module`, host/run/deploy/watch options, `--got-manifest`/`--cold`, debug/release defaults, debug protocol options, `stc rename`, `stc online download/diff`, `[online]`/`[runtime]`/`[debug]` schema, manifest metadata, build IDs, and cold-build status output |
| `Specification/Architecture/Testing-and-Verification.md` | §1–§4 test levels, §6 CI | Add hot-swap suite, manifest identity/rename/download tests, loader/catalog fuzzing, determinism, performance, debug/monitoring tests, and both-host matrix |
| `Specification/AST/AST.md` | §5–§8 HIR | Document stable identity/catalog expectations if HIR gains identity helpers; clarify that `GotId`s and manifest history are compiler/runtime contracts, not language semantics |
| `Specification/Decisions.md` | §1 answered, §2 confirmed, §3 open | Add Q-018+ and, after resolution, A-022+ and D-094+ with affected sections |
| `M4-Plan.md` | Top note and status | Mark the direct-reference artifact model as superseded by M4.5 while keeping the historical plan |
| `handover_to_M5.md` | §2 scope, §3 contracts, §5 architecture, §9 decisions, §10 definition of done | Add a GOT/module/manifest addendum: M5 must build on the host model; the existing M5 recommendations about module-owned vtables and globals are superseded |
| `update_plan_got.md` | This document | Update as questions are resolved and decisions are recorded |
| New document | `Specification/Architecture/Debug-and-Monitoring.md` | Recommended dedicated spec for typed variable access, breakpoints/stepping, forcing, debug protocol/security, DWARF/CodeView, stop behavior, and online-change interaction |
| New document | `Specification/Architecture/Online-Change-and-GOT.md` | Recommended dedicated spec for the GOT, GOT manifest, identity/history, host API, module ABI, online-change plan, migration, download, compatibility, and swap protocol |
| `tools/ci/check_docs.py` | `REQUIRED_DOCUMENTS` | Add the new specification document if it is created |
| `.github/workflows/ci.yml` | Hot-swap jobs | Add module/host/manifest tests to Windows and Linux jobs; keep cross-target module emission |
| `README.md` document table | Links | Add the new spec document and this plan if it remains a maintained document |

## 13. Questions and resolved answers

All questions in this section are resolved. §13.0 records the disposition; §13.1–§13.10 retain
the question text and the recommended default that was adopted, unless the status column records
a non-default decision.

Priority meanings:

- **P0** — blocking: must be resolved before M4.5 design or any code change.
- **P1** — must be resolved before the milestone that first depends on it (M5, M6, M7, or M8).
- **P2** — deferred; can be answered after the foundation is working.

### 13.0 Review status

| ID | Answer | Status |
| --- | --- | --- |
| Q-018 | Online change is a v1 requirement | Resolved (recommended) |
| Q-019 | Persistent host + whole modules; no per-function patching | Resolved (recommended) |
| Q-020 | `module` + manifest primary; `exe` launcher; `lib` embedding | Resolved (recommended) |
| Q-026 | Persistent-boundary GOT scope | Resolved (recommended) |
| Q-027 | Locals/temps/stack stay direct, not GOT-managed | Resolved (recommended) |
| Q-030 | Shared host cells + per-module slot table | Resolved (recommended) |
| Q-031 | Stable pointer cells; storage never moves | Resolved (recommended) |
| Q-032 | Host-only cell writes; internal host alloc/free; no cell rebinding; v2 `__NEW`/`__DELETE` | Resolved |
| Q-035 | One GOT codegen path | Resolved (recommended) |
| Q-036 | Current key + persistent `GotId` for data roots | Resolved (recommended) |
| Q-037 | Qualified mangled name + signature + `GotId` for functions | Resolved (recommended) |
| Q-038 | `GotId` + qualified name + structural hash + size + target for types | Resolved (recommended) |
| Q-040 | Host allocates roots; storage never moves | Resolved (recommended) |
| Q-041 | LLVM constant-expression size + 16-byte alignment | Resolved (recommended) |
| Q-042 | Per field recursively; handle-based objects; arrays hybrid; strings one root; `OVERLAP` atomic | Resolved (non-default) |
| Q-043 | Removed roots quarantined until restart/explicit reset | Resolved (recommended) |
| Q-046 | Preserve every matching field/root regardless of RETAIN | Resolved (recommended) |
| Q-047 | Code-only + field add/remove/rename + compatible field changes | Resolved (recommended) |
| Q-048 | Reject whole module by default; explicit per-field reset; cold for baseline issues | Resolved (recommended) |
| Q-054 | Atomic module acceptance; one rejected field/plan rejects the module | Resolved (recommended) |
| Q-056 | Versioned host API: trap/clock/IO/state/string/math/alloc/free/object handles | Resolved (recommended) |
| Q-057 | Runtime-owned descriptors; vtable cells; object record points to descriptor | Resolved (recommended) |
| Q-064 | Descriptor + attach/prepare/cycle/shutdown/detach with field/object metadata | Resolved (recommended) |
| Q-070 | Old/new coexist through prepare/rollback; unload after commit + one successful cycle | Resolved (recommended) |
| Q-071 | Single cycle thread + background loader/prepare thread; atomic boundary publish | Resolved (refined by Q-175) |
| Q-072 | Swap after old outputs publish, before next input latch; complete cycles only | Resolved (recommended) |
| Q-084 | Layered tests incl. hot-swap/manifest/cold/fuzz on Windows+Linux | Resolved (recommended) |
| Q-086 | Sorted keys/slots; content build IDs; byte-identical double builds | Resolved (recommended) |
| Q-089 | DoD spans M4.5–M8, not foundation only | Resolved (recommended) |
| Q-090 | Insert M4.5 before M5; no renumbering | Resolved (recommended) |
| Q-091 | Pause implementation; finish reference review and docs first | Resolved (recommended) |
| Q-095 | All P0s, including deferred ones, must be resolved before code | Resolved (stricter than default) |
| Q-096 | Persistent GOT manifest design approved | Resolved (recommended) |
| Q-097 | Canonical JSON + compact binary manifest projection | Resolved (recommended) |
| Q-098 | Valid baseline for online; otherwise cold | Resolved (recommended) |
| Q-099 | Content-derived 128-bit `GotId` | Resolved (recommended) |
| Q-101 | Compiler emits migration plan; host validates/executes | Resolved (recommended) |
| Q-108 | Per-field `GotId`s; arrays hybrid; strings one root; `OVERLAP` atomic | Resolved by Q-042/Q-123–Q-126 |
| Q-110 | M4.5 identity/rename and M5 aggregates; no field-layout migration in v1 | Resolved |
| Q-117 | Exact parent == active build ID + digest pair + profile/target/ABI | Resolved (recommended) |
| Q-120 | Alloc/free only; no cell rebinding | Resolved (recommended) |
| Q-122 | User-visible `__NEW`/`__DELETE` is a v2 vendor extension | Resolved by Q-032 clarification |
| Q-123 | Hybrid arrays: scalar/string contiguous; aggregate arrays per-element handles | Resolved (recommended) |
| Q-124 | One root per string value; no per-character roots | Resolved (recommended) |
| Q-125 | One atomic OVERLAP root with aliasing views | Resolved (recommended) |
| Q-126 | Opaque handles; uniform place descriptors for REF_TO with Root/Direct/Stack/Object kinds | Resolved with revisions Q-158/Q-162–Q-165 |
| Q-127 | Per-field recursive value copy into destination roots | Resolved (recommended) |
| Q-128 | Config/instance + base chain + field-name paths | Resolved (recommended) |
| Q-129 | Uniform per-field rooting with Q-123–Q-125 exceptions | Resolved (recommended) |
| Q-130 | Measure performance/size first; no hard cap yet | Resolved (recommended) |
| Q-132 | Persistent REF_TO targets: stable host roots/handles; reject transient/module-owned | Resolved (recommended) |
| Q-133 | Pointer-bearing roots reject/reset during migration; no raw copy | Resolved (recommended) |
| Q-134 | References/interfaces/handles are not persisted in the RETAIN state file | Resolved (recommended) |
| Q-131 | `POINTER TO` scope in v1 | Resolved in principle: transient-only, distinct type, full arithmetic; details Q-143–Q-150 |
| Q-135 | POINTER TO is transient-only in v1, with full arithmetic | Resolved (non-default) |
| Q-136 | Allow arithmetic/indexing/SIZEOF | Resolved (non-default) |
| Q-137 | No persistent pointer fields; locals/params only | Resolved (recommended) |
| Q-138 | Distinct POINTER TO type | Resolved (recommended) |
| Q-139 | Explicit POINTER TO↔REF_TO only; no implicit pointer conversions | Resolved (recommended) |
| Q-140 | `__NEW`/`__DELETE` remain v2; v1 pointers target static/locals/params | Resolved |
| Q-141 | `STC-POINTER` vendor extension; no IEC claim | Resolved via Q-149 |
| Q-142 | No persistent pointers in v1; if later allowed, reject/reset and exclude state | Resolved |
| Q-143 | Contiguous pointer targets only | Resolved (recommended) |
| Q-144 | ADR: locals/params, leaf fields, array elements, strings, OVERLAP | Resolved (recommended) |
| Q-145 | Typed unchecked arithmetic; null-deref trap; boundary crossing undefined | Resolved (recommended) |
| Q-146 | No implicit conversions; explicit POINTER TO↔REF_TO only; no pointer↔integer; reject persistent pointers | Resolved (recommended) |
| Q-147 | Pointer parameters and results allowed with documented lifetime | Resolved (recommended) |
| Q-148 | SIZEOF for contiguous values only | Resolved (recommended) |
| Q-149 | STC-POINTER extension, implemented in M5 | Resolved (recommended) |
| Q-150 | Reject persistent pointers; null-deref trap; documented dangling UB | Resolved (recommended) |
| Q-151 | Pointer→REF_TO via runtime GOT/process-image resolver; NULL on failure | Resolved (details Q-161/Q-166/Q-169) |
| Q-152 | REF_TO→interface allowed only for class/FB object references with descriptor check; null→null | Resolved (recommended) |
| Q-153 | Pointer→interface/handle conversion disallowed | Resolved by Q-173 |
| Q-154 | Descriptor checks for REF_TO→interface; resolver for pointer→REF_TO | Resolved by Q-152/Q-161/Q-171 |
| Q-155 | Pointer→REF_TO resolves to active persistent roots and direct AT variables | Resolved (non-default) |
| Q-156 | Validate interior pointers via manifest layout descriptors | Resolved (recommended) |
| Q-157 | Invalid cast yields NULL; deref traps | Resolved (recommended) |
| Q-158 | Validated references use place descriptors | Resolved (non-default) |
| Q-159 | Exact runtime type/layout match; no silent reinterpretation | Resolved (recommended) |
| Q-160 | Pointer-derived REF_TO may persist under descriptor/remap rules | Resolved (recommended) |
| Q-161 | Host resolver service + range/type index; lookup per cast | Resolved (recommended) |
| Q-162 | Uniform place descriptor for all persistent-capable REF_TO | Resolved (recommended) |
| Q-163 | Stack descriptor valid only during the call | Resolved (recommended) |
| Q-164 | Stable DirectId in manifest + area/offset/type for AT refs | Resolved (recommended) |
| Q-165 | Object kind {handle, descriptor}; REF_TO→interface descriptor check | Resolved (recommended) |
| Q-166 | Dereference resolves descriptor; optional generation-checked cache | Resolved (recommended) |
| Q-167 | References not persisted in v1 state file; later Root/Direct format possible | Resolved (recommended) |
| Q-168 | Remap Root/Direct; Stack never; Object by descriptor | Resolved (recommended) |
| Q-169 | Resolver API/index and exact type matching | Resolved (recommended) |
| Q-170 | Interfaces are nullable; NULL is valid | Resolved (recommended) |
| Q-171 | REF_TO→interface null/incompatible yields null (attempt semantics) | Resolved (recommended) |
| Q-172 | Use NULL comparison/?=; no mandatory __ISVALIDREF | Resolved (recommended) |
| Q-173 | Pointer→Object/interface disallowed; object refs via REF(object) | Resolved (recommended) |
| Q-174 | Adopt double-buffered normal/temp GOT activation | Resolved (recommended) |
| Q-175 | Background loader/prepare thread; cycle thread only switches | Resolved (recommended) |
| Q-176 | All function slots in temp GOT bind to new module | Resolved (recommended) |
| Q-177 | Share preserved roots; temp adds new; no preserved writes during prepare | Resolved (recommended) |
| Q-178 | New code may prepare on the loader thread with its own temp GOT context | Resolved (recommended) |
| Q-179 | Background temp→normal copy; switch at later boundary; serialize reloads | Resolved (recommended) |
| Q-180 | Keep old generation until one successful cycle; recoverable rollback | Resolved (recommended) |
| Q-181 | Off-thread unload after commit/grace | Resolved (recommended) |
| Q-182 | Bounded maintenance jitter; O(1) switch; heavy work off-thread | Resolved (recommended) |
| Q-183 | Full variable and debug access in v1 | Resolved (non-default; scope expansion) |
| Q-184 | Generation-pinned typed reads; no cycle lock | Resolved (recommended) |
| Q-185 | Queue writes/forcing for cycle-boundary application | Resolved (recommended) |
| Q-186 | Full debug in v1 | Resolved (non-default; conflicts with prior non-goal) |
| Q-187 | Epoch/refcount pinning; loader waits for pins | Resolved (recommended) |
| Q-188 | Lock-free counters/double-buffer/ring status | Resolved (recommended) |
| Q-189 | Typed path/GotId API with auth/rate limits; no raw memory | Resolved (recommended) |
| Q-190 | Lower-priority non-RT threads, bounded queues, measured contention | Resolved (recommended) |
| Q-191 | Full debug includes read/write/force/watch/breakpoints/step/call stack | Resolved (recommended) |
| Q-192 | Codegen breakpoint checkpoints/table; no runtime patching | Resolved (recommended) |
| Q-193 | Stop-the-world stepping; suspend/resume cycle | Resolved (recommended) |
| Q-194 | Boundary-applied forcing, held until release, not persisted | Resolved (recommended) |
| Q-195 | Hold last outputs indefinitely; project is not a safety PLC | Resolved (non-default) |
| Q-196 | Require full DWARF/CodeView for full debug | Resolved (non-default; split view Q-203) |
| Q-197 | Authenticated debug protocol, audit, production policy | Resolved (recommended) |
| Q-198 | Debug hooks disabled in production; measured overhead | Resolved (recommended) |
| Q-199 | Every online swap ends the debug session; breakpoints/forces cleared; reconnect | Resolved (non-default) |
| Q-200 | Dedicated debug milestone before v1 release | Resolved (recommended) |
| Q-201 | Hold outputs indefinitely; no automatic fail-safe | Resolved (non-default; non-safety stance) |
| Q-202 | Debug default; `--release` optional stripped/optimized mode | Resolved (non-default) |
| Q-203 | Standard DWARF/CodeView + manifest-assisted split-object view | Resolved (recommended) |
| Q-204 | DWARF/CodeView via clang target; toolchain verification | Resolved (recommended) |
| Q-205 | Debug-info determinism/stripping rules | Resolved (recommended) |
| Q-206 | Keep active-generation debug info; swap ends session; drop old after unload | Resolved (recommended) |
| Q-207 | Pause/disable watchdog while stopped; hold outputs | Resolved (recommended) |
| Q-208 | Debug default; release strips/optimizes; sidecar symbols policy | Resolved (recommended) |
| Q-209 | Debug/release have distinct build IDs/manifests | Resolved (recommended) |
| Q-210 | M6.5 dedicated debug milestone between M6 and M7 | Resolved (recommended) |
| Q-211 | Session end clears forces/breakpoints and resumes normal outputs | Resolved (recommended) |
| Q-021 | Host API + local CLI/deploy; no network control | Resolved (recommended) |
| Q-022 | Host + adjacent module only; no code-linked executable | Resolved (recommended) |
| Q-023 | Windows MSVC x64 + Linux GNU x64; AArch64 in M8 | Resolved (recommended) |
| Q-024 | Online change is a runtime/platform capability, not IEC profile | Resolved (recommended) |
| Q-025 | Host API + local CLI reload/watch; remote UI later | Resolved (recommended) |
| Q-028 | Read-only module data direct; no persistent pointers into it | Resolved (recommended) |
| Q-029 | Compiler helpers/intrinsics direct; not GOT-managed | Resolved (recommended) |
| Q-033 | One direct-variable GOT cell per AT variable/field | Resolved (non-default) |
| Q-034 | Register user functions/methods/FB bodies/exports/std FBs; helpers not | Resolved (recommended) |
| Q-039 | Package-qualified keys; duplicate error; tracked moves keep GotId | Resolved (recommended) |
| Q-044 | Quarantine removed storage; references stale but memory-safe | Resolved (recommended) |
| Q-212 | Direct-variable cells are per-generation | Resolved (recommended) |
| Q-045 | Host owns process image, retain buffer, task guards, clock, counters, registry, diagnostics | Resolved (recommended) |
| Q-050 | Per-field add/remove preserves; field type changes reset/reject; array/string/OVERLAP rules | Resolved (recommended) |
| Q-051 | Add/remove instances with init/reset; config/task topology cold | Resolved (recommended) |
| Q-052 | Initializers only for new roots; changed initializers on preserved roots ignored | Resolved (recommended) |
| Q-053 | Online attach skips state load; in-memory wins; v1 layout hash | Resolved (recommended) |
| Q-055 | Rollback via temp/normal GOT for load/prepare; traps fatal unless policy | Resolved (recommended) |
| Q-058 | Stable runtime descriptors; preserved interfaces survive compatible swaps | Resolved (recommended) |
| Q-059 | Host export registry by C name/signature cell | Resolved (recommended) |
| Q-060 | Stdlib compiled per module under STC_STD with stable keys | Resolved (recommended) |
| Q-061 | Vtable shape changes only when no live instance/interface depends | Resolved (recommended) |
| Q-066 | Full structural module validation | Resolved (recommended) |
| Q-067 | Immutable versioned module files + manifest sidecar + current pointer | Resolved (recommended) |
| Q-068 | Internal user code; exported descriptor/entry; no imports; dynamic CRT; shared/PIC | Resolved (recommended) |
| Q-069 | Platform loader; load failures non-fatal | Resolved (recommended) |
| Q-073 | Task/resource topology changes require cold | Resolved (recommended) |
| Q-074 | Clock continuous; watchdog paused/extended with bound | Resolved (recommended) |
| Q-075 | Reconfigure image; preserve %M/same-address values; warn remaps | Resolved (recommended) |
| Q-076 | Old version completes/publishes; new starts next input latch | Resolved (recommended) |
| Q-078 | Module emit, got-manifest/cold, rename, run/watch, online download/diff, debug options | Resolved (recommended) |
| Q-079 | [runtime]/[online]/[debug] manifest tables | Resolved (recommended) |
| Q-080 | Stable MODULE/MANIFEST/ABI/ONLINE/W + debug families | Resolved (recommended) |
| Q-081 | Host retains per-module span/source metadata; active generation reported | Resolved (recommended) |
| Q-085 | Measure scalar/OOP/field access; no hard cap yet | Resolved (recommended) |
| Q-087 | Trusted local modules; strict validation; optional build-ID/hash pin | Resolved (recommended) |
| Q-088 | Extend fuzz_codegen + descriptor/manifest parser; no native execution | Resolved (recommended) |
| Q-092 | Host ABI 2; M4 artifacts not loadable | Resolved (recommended) |
| Q-093 | Dedicated Online-Change/GOT and Debug/Monitoring specs | Resolved (recommended) |
| Q-094 | Online change/manifest are runtime capabilities; POINTER TO/debug separate extensions | Resolved (recommended) |
| Q-100 | Explicit rename/move maps + --rename; stc rename later | Resolved (recommended) |
| Q-102 | Versioned manifest; module + runtime state dir; optional VCS; active authoritative | Resolved (recommended) |
| Q-103 | Host API/CLI export active manifest/layout/controller ID; --got-manifest | Resolved (recommended) |
| Q-104 | Untrusted parsed data; validate/digest/pin; access-controlled | Resolved (recommended) |
| Q-106 | No globally stable slots; GotId + per-module mapping | Resolved (recommended) |
| Q-107 | Manifest has no runtime values | Resolved (recommended) |
| Q-109 | Diagnose untracked changes; explicit allow-untracked-removal or cold | Resolved (recommended) |
| Q-111 | Content GotIds; exact parent; stale -> cold; no merge/rebase | Resolved (recommended) |
| Q-112 | Local host API/CLI file export/copy; no network protocol | Resolved (recommended) |
| Q-115 | Compiler emits pre-deployment compatibility report/diff | Resolved (recommended) |
| Q-118 | Cold initializes non-retained; RETAIN per compatibility; no transient allocations | Resolved (recommended) |
| Q-119 | Manifest parent + compiler status + runtime enforcement + --cold | Resolved (recommended) |
| Q-065 | Host API versioned struct: abi_version + struct_size; additive within major | Resolved (recommended) |
| Q-049 | No field-layout migration in v1; per-field add/remove/rename; type changes reset/cold | Resolved |
| Q-121 | Internal allocations transient; freed on failure/retire/unload | Resolved |
| Q-062 | No direct calls in the M4.5 foundation; revisit after benchmarks | Resolved (recommended) |
| Q-063 | No special `THIS`/`SUPER` mechanism; stable handles/per-field roots | Resolved (recommended) |
| Q-077 | Host-owned task state by task identity; preserved if topology unchanged | Resolved (recommended) |
| Q-082 | Load-time compatibility diagnostics primary; warn only for unsafe constructs | Resolved (recommended) |
| Q-083 | Host introspection of active/retired builds, catalog, compatibility, quarantine | Resolved (recommended) |
| Q-105 | Minimal module descriptor plus manifest; one model, two projections | Resolved (recommended) |
| Q-113 | No retained-state migration in v1; a later state format may key by `GotId` | Resolved (recommended) |
| Q-114 | Measure; JSON for tooling plus binary for runtime; cache active record | Resolved (recommended) |
| Q-116 | Optional later declaration scaffold; not source recovery | Resolved (recommended) |

### 13.1 Scope, artifacts, and process

| ID | Question | Recommended default | Pri |
| --- | --- | --- | --- |
| Q-018 | Is online change a v1 requirement? | Yes. Implement the foundation in M4.5 and the user-facing workflow in M6. | P0 |
| Q-019 | Is the execution model a persistent host plus loadable whole modules, rather than per-function hot patching? | Yes. Load one whole module as a shared library, swap it atomically at a cycle boundary, and retire the whole previous module. Per-function or relocatable-object loading is not a v1 path. | P0 |
| Q-020 | What artifact kinds exist and what does each mean? | Add `module`; each module is paired with a GOT manifest; `exe` is a host launcher that loads an adjacent module; `lib` remains the embedding artifact. No code-linked executable for online-change deployments. | P0 |
| Q-021 | How is a new module delivered and triggered? | Host API plus `stc run --watch`/a local deploy command; no network control in v1. | P1 |
| Q-022 | Is a standalone non-hot-swap executable still required? | Keep it as a host + adjacent module; do not maintain a second code-linked static deployment path. | P1 |
| Q-023 | Which hosts and targets are supported in v1? | Windows MSVC x86-64 and Linux GNU x86-64; Linux AArch64 with M8; a module target must match the host. | P1 |
| Q-024 | Is online change part of the IEC conformance profile? | No. It is a project runtime/platform capability; document it separately from the ST language profile. | P1 |
| Q-025 | What is the minimum online-change UX in v1? | Programmatic host API plus a local CLI reload/watch in M6; remote deploy, scheduling, and rollback UI are v2. | P1 |

### 13.2 GOT semantics

| ID | Question | Recommended default | Pri |
| --- | --- | --- | --- |
| Q-026 | What must be GOT-indirected? | Every persistent root, every user/standard function call, every stored function pointer, and every vtable/type descriptor. Stack, locals, compiler helpers, intrinsics, and module-owned read-only data stay direct. | P0 |
| Q-027 | Are locals, temporaries, parameters, and stack storage GOT-managed? | No. They are transient and cannot outlive a cycle. | P0 |
| Q-028 | Are module-owned string literals and constant tables GOT-managed? | No, provided no persistent variable stores a pointer into them. | P1 |
| Q-029 | Are compiler-generated helpers and LLVM intrinsics GOT-indirected? | No; they are version-local and never stored as persistent pointers. | P1 |
| Q-030 | Where does the GOT live and how is it addressed? | Host-owned stable cells; module gets a slot table of cell pointers at attach; shared cells are reused across versions. | P0 |
| Q-031 | Do cells hold pointers or base-plus-offset values? | Pointers, in stable-address cells; storage never moves. | P0 |
| Q-032 | May generated code write GOT cells? | No direct writes. Only the host writes cells under the control lock. The module may call an internal host allocation/rebinding service; user-visible `__NEW`/`__DELETE` is deferred to a v2 vendor extension. See Q-120/Q-121. | P0 |
| Q-033 | How do direct `%I`/`%Q`/`%M` accesses use the GOT? | One GOT cell per direct variable/field, pointing into the process image; per-variable metadata and DirectIds. Per-variable cells are per GOT generation (Q-212). | P1 |
| Q-212 | Are direct-variable GOT cells per-generation or shared? | Per-generation. Image reconfiguration rebinds cells by DirectId/area+offset and preserves values where possible; old code never sees the new image. | P1 |
| Q-034 | Must all function definitions be registered in the catalog even if called directly? | Yes for user functions, methods, FB bodies, exports, and standard FBs; compiler helpers may be omitted. | P1 |
| Q-035 | Is a non-GOT codegen mode kept? | No. One path, fully tested. | P0 |

### 13.3 Identity and memory

| ID | Question | Recommended default | Pri |
| --- | --- | --- | --- |
| Q-036 | What is the stable key for a data root, and what is its persistent identity? | The current key is the package/config/instance-qualified field path plus root kind/field kind. Persistent identity is a per-field manifest `GotId`, so renames and moves do not lose state. Type identity is recorded separately. | P0 |
| Q-037 | What is the stable key for a function? | Current key is the qualified mangled name plus signature hash; persistent function identity is a manifest `GotId`. Overloads remain distinct. | P0 |
| Q-038 | What is the stable key/layout identity for a type? | Manifest `GotId`/type identity plus qualified name, structural layout hash, size, and target triple; aliases are transparent. | P0 |
| Q-039 | How are renames, collisions, and package moves handled? | Package-qualified keys; a rename is a remove plus add; duplicate keys are a module error. | P1 |
| Q-040 | Who allocates persistent roots, and what is their lifetime? | The host allocates each root; storage never moves; it survives swaps until quarantined or the host restarts. | P0 |
| Q-041 | How does the host learn root size and alignment? | Module descriptor size from an LLVM constant expression; conservative 16-byte alignment; target triple checked. | P0 |
| Q-042 | What is the root granularity? | Per field recursively, with handle-based objects: every persistent struct/FB/class field is a separate root with its own `GotId`. Arrays, strings, and `OVERLAP` granularity are resolved by Q-123–Q-125. Supersedes the earlier root-per-global/program-instance default. | P0 |
| Q-043 | What happens to removed roots? | Quarantine until host restart or explicit reset; warn; never free during a swap in v1. | P0 |
| Q-044 | What happens to references to removed or reset storage? | Quarantine keeps them memory-safe but stale; warn; document that the new code cannot name the removed root. | P1 |
| Q-045 | Which non-root state is host-owned? | Process image, retained buffer, task guards, clock, runtime counters, module registry, and diagnostics. | P1 |

### 13.4 Compatibility and state

| ID | Question | Recommended default | Pri |
| --- | --- | --- | --- |
| Q-046 | What is preserved on a swap? | All per-field roots whose manifest `GotId` is present and compatible, regardless of `RETAIN`. `RETAIN` remains a warm-restart policy, not an online-change subset. | P0 |
| Q-047 | What changes are online-compatible? | Code-only changes, new globals/functions/types not reachable from live state, additive instances, and changed bodies with unchanged signatures. Per-field add/remove is preserve/drop; field type changes, arrays/strings/OVERLAP changes, vtable-shape changes, and task-topology changes are policy decisions. | P0 |
| Q-048 | What is the default policy for an incompatible root? | Reject the whole module; keep the old version active. `reset` is opt-in per field/root or manifest; cold download is the fallback for baseline failures. | P0 |
| Q-049 | Is field-wise layout migration required in v1? | No. Per-field roots turn field add/remove/reorder into preserve/drop; field type changes, resizing, and arrays/strings/`OVERLAP` changes reset the affected root or require a cold download. Migration remains a later design. | P0 |
| Q-050 | How are FB/class instance type changes handled? | Per-field add/remove preserves matching fields; field type changes require reset/migration/reject. Arrays/strings/OVERLAP follow Q-123–Q-125. No whole-object layout migration. | P1 |
| Q-051 | How are program instance and configuration changes handled? | Adding/removing instances is allowed with init/reset; configuration, resource, and task topology changes require a cold download. | P1 |
| Q-052 | Are initializers re-applied to preserved roots? | No. Only new roots are initialized; changing an initializer on a preserved root is ignored with a warning. | P1 |
| Q-053 | How do online change, the manifest, and `RETAIN` state files interact? | Online attach skips state-file load; the in-memory values win. The active manifest is the identity baseline. A later state-format revision can key retained leaves by `GotId` plus path; v1 keeps the layout-hash policy. | P1 |
| Q-054 | Is a module accepted atomically or item by item? | Atomically. Any rejected item rejects the module unless the policy explicitly resets it. | P0 |
| Q-055 | Is rollback supported? | Yes for load/prepare failure using a cell snapshot; after activation, a trap follows the existing fatal policy unless a host rollback policy is selected. Rollback after commit requires the previous module/manifest pair; if it is unavailable or not current, rollback is a cold download. | P1 |

### 13.5 Functions, OOP, and exports

| ID | Question | Recommended default | Pri |
| --- | --- | --- | --- |
| Q-056 | How does a module call runtime services? | Through the host API function table passed at attach; no `stc_rt_*` imports. V1 includes trap, clock, I/O, state, string/math helpers, and the narrow internal allocation/rebinding service (Q-120). | P0 |
| Q-057 | How are vtables and type descriptors represented? | Runtime-owned descriptors containing cell pointers; the host builds/validates them from the module catalog. | P0 |
| Q-058 | How do interface values and `?=` survive a swap? | Stable runtime type descriptors and type keys; compatible types keep descriptors, incompatible shapes reject the module. | P1 |
| Q-059 | How are `{stc:export}` functions dispatched across a swap? | Host export registry keyed by C name and signature cell; unchanged signature required; external hosts call through the host API. | P1 |
| Q-060 | How is standard-library FB identity stabilized? | Compile the stdlib into each module under the reserved `STC_STD` namespace; keys must be identical across compatible builds. | P1 |
| Q-061 | Can methods be added/removed or vtable slots reordered online? | Only when no live instance or interface value depends on the old shape; otherwise reject or cold download. | P1 |
| Q-062 | May direct calls replace GOT calls for performance? | Not in the M4.5 foundation. Revisit after benchmarks; any optimization must preserve the no-raw-persistent-pointer rule. | P2 |
| Q-063 | How are `THIS`, `SUPER`, and object pointers treated? | No special mechanism: `THIS` is the stable root pointer; `SUPER` uses a function cell; preserved object pointers remain valid under quarantine rules. | P2 |

### 13.6 Module ABI and loader

| ID | Question | Recommended default | Pri |
| --- | --- | --- | --- |
| Q-064 | What is the module ABI surface? | Descriptor plus attach/prepare/cycle/shutdown/detach, as sketched in §6.9. Prepare must not call through shared function cells; cells switch only at activation. | P0 |
| Q-065 | What is the host API table shape and versioning rule? | Versioned struct with `abi_version` and `struct_size`; additive fields within a major version; major bump for breaking changes. | P0 |
| Q-066 | How is a module validated before use? | Magic, module/host ABI, profile, target triple, endianness, pointer width, build ID, catalog bounds, duplicate keys, and descriptor consistency. | P1 |
| Q-067 | How are modules packaged, named, and deployed? | Immutable versioned files `<name>-<build-id>.dll/.so` plus sidecar JSON; atomic "current" pointer; no overwrite of a loaded module. | P1 |
| Q-068 | What linkage, visibility, and CRT rules apply? | Internal user code, exported descriptor/entry only; no runtime imports; dynamic CRT on Windows; `-shared -fPIC`; module target equals host. | P1 |
| Q-069 | Which loader APIs and failure semantics? | Platform layer over `LoadLibrary`/`GetProcAddress`/`FreeLibrary` and `dlopen`/`dlsym`/`dlclose`; load failures are non-fatal and keep the old version. | P1 |
| Q-070 | When can old and new modules coexist, and when is unload safe? | Coexist during prepare/rollback; unload after commit and one successful cycle. Raw code pointers exist only in host function cells, never in persistent roots or external host state. | P0 |
| Q-071 | What is the host threading model? | One cycle thread plus an optional background loader/prepare thread. The loader builds a temp GOT generation off-thread; the cycle thread publishes it with an atomic release/acquire switch at a boundary. External calls serialize on the control lock; no concurrent cycle or activation. | P0 |

### 13.7 Execution context

| ID | Question | Recommended default | Pri |
| --- | --- | --- | --- |
| Q-072 | Where exactly is the swap boundary relative to I/O? | After the old cycle publishes outputs, before the next input latch; only complete cycles. | P0 |
| Q-073 | Can task/resource topology change online? | No in v1; cold download. | P1 |
| Q-074 | How do the clock and watchdog behave during a swap? | Clock remains continuous; watchdog is paused or extended during load/prepare with a bounded limit. | P1 |
| Q-075 | How is the process image reconfigured and what values survive? | Host reconfigures from metadata; preserve `%M` and same-address values where possible; warn on remapped addresses. | P1 |
| Q-076 | Which version publishes outputs at the boundary? | The old version completes and publishes; the new version starts with the next input latch. | P1 |
| Q-077 | What happens to `SINGLE` and per-task execution state? | Host-owned and keyed by task; preserved when topology is unchanged, reset on cold download. | P2 |

### 13.8 Tooling, diagnostics, and verification

| ID | Question | Recommended default | Pri |
| --- | --- | --- | --- |
| Q-078 | Which CLI commands and options are added? | `--emit=module` (always emits its manifest); `--got-manifest`, `--cold`, `--rename old=new`; `stc run`/`--watch`; `stc online download/diff`; `stc deploy` in M6; host status/rollback options. | P1 |
| Q-079 | Which `stc.toml` keys are added? | A `[runtime]`/`[online]` table for host, module dir, baseline manifest path, rename map, migration/reset policy, state interaction, and build-ID pinning. | P1 |
| Q-080 | Which diagnostic codes are assigned? | The `STC-E-MODULE-*`, `STC-E-MANIFEST-*`, `STC-E-ABI-*`, `STC-E-ONLINE-*`, `STC-W-MANIFEST-*`, and `STC-W-ONLINE-*` families in §6.10. | P1 |
| Q-081 | How are traps and source maps handled after unload? | Keep per-module span/metadata in the host; report against the active version; optional debug retention of retired modules. | P1 |
| Q-082 | Are compile-time warnings needed for online-hostile constructs? | Not initially; load-time compatibility diagnostics are primary. Add warnings only for constructs with no safe runtime rule. | P2 |
| Q-083 | What host introspection is exposed? | Active/retired build IDs, catalog summary, compatibility state, quarantine list, and last rejection. | P2 |
| Q-084 | What is the test strategy and CI matrix? | Unit, structural IR, descriptor, host lifecycle, hot-swap execution, negative, determinism, fuzz, Windows/Linux. | P0 |
| Q-085 | What performance budget applies? | Measure scalar and OOP cycles; document overhead; no hard budget until benchmarks exist. Optimize cell caching before considering direct calls. | P1 |
| Q-086 | What determinism guarantees apply? | Sorted keys/slots, content-derived build IDs, byte-identical double builds, no paths/timestamps. | P0 |
| Q-087 | What is the loader security stance? | Trusted local modules; strict descriptor validation; optional build-ID pin; no sandbox, network deploy, or untrusted loading in v1. | P1 |
| Q-088 | What is the fuzzing scope? | Extend `fuzz_codegen`; add a descriptor/catalog parser target; avoid executing arbitrary native modules. | P1 |
| Q-089 | What is the v1 definition of done for online change? | Foundation M4.5 criteria plus M5 feature coverage, M6 UX/state, M7 hardening, M8 host parity. | P0 |

### 13.9 Process and documentation

| ID | Question | Recommended default | Pri |
| --- | --- | --- | --- |
| Q-090 | Where does online change sit in the milestones? | Insert M4.5 before M5; do not renumber the existing milestones. | P0 |
| Q-091 | Do we backtrack to update M4 now or proceed to M5? | Pause implementation: finish the reference review and apply the documentation updates first. Do not start M4.5 or M5 code yet. Resolved. | P0 |
| Q-092 | Are old M4 artifacts and ABI kept compatible? | No. Introduce host ABI 2; document that M4 executables/libraries are not loadable. | P1 |
| Q-093 | Is a dedicated specification document created? | Yes: `Specification/Architecture/Online-Change-and-GOT.md`, added to the required-document checker and README table. | P1 |
| Q-094 | Does online change alter the language profile? | No. No new syntax or semantics; document it as a runtime capability and project feature. | P1 |
| Q-095 | What is the planning exit condition? | All P0 questions, including deferred ones, must be resolved; decisions recorded; and the §12 document checklist applied. Only then choose M4.5 implementation. Resolved (stricter than default). | P0 |

### 13.10 Persistent GOT manifest and compiler-driven change

| ID | Question | Recommended default | Pri |
| --- | --- | --- | --- |
| Q-096 | Should the compiler emit and consume a persistent GOT manifest as the identity baseline? | Yes. Every module is paired with a manifest; every online-change build consumes a baseline and records its digest; cold builds still emit a complete manifest. | P0 |
| Q-097 | What is the manifest format and content? | Working names `<name>.stc-got.json` (canonical tooling/download) and `<name>.stc-got.bin` (compact runtime projection), both generated from one in-memory model. It carries header/lineage, identity records (including current slot mappings and runtime cell kinds), layout/field records, function/type/instance/task/export records, the online-change plan, and integrity digests. | P0 |
| Q-098 | Is a manifest required for online-change builds, and how are missing/stale/mismatched baselines handled? | A baseline is required for online-change intent, with explicit `--got-manifest` and a default discovery path. Missing, invalid, unpaired, or non-current baselines select a cold-layout build or a cold download; no recovery/merge is attempted. | P0 |
| Q-099 | How are persistent `GotId`s assigned and retained? | Use a content-derived 128-bit identity from scope/name/kind, with collision checking. Data roots and types exclude the current type/layout so layout changes remain trackable; functions include the signature. Renames/moves keep the old `GotId`; new declarations get new IDs; removed entries retire. A reintroduced retired key gets a new ID unless explicitly reused. Per-module slot numbers remain local and map to `GotId`s. | P0 |
| Q-100 | How are tracked renames and moves expressed? | Explicit rename/move map plus `--rename old=new` in M4.5; support field, namespace, and package bulk maps; source-aware `stc rename` that rewrites references and records history in M6. No heuristic rename detection. | P1 |
| Q-101 | What migration operations exist, who generates them, and who executes them? | Resolved: migration is not in v1. When later added, the compiler emits preserve/allocate/migrate/reset/drop plans and the host validates/executes them with rollback snapshots. M4.5 supports preserve/allocate/drop and tracked rename only. | P0 |
| Q-102 | Where is the manifest stored, versioned, and committed? | Versioned format; stored next to the module and in the runtime's state directory; optionally committed to VCS. The runtime's active copy is authoritative. If it is missing or corrupt, the next deployment is a cold download; `stc clean` must not silently delete the active baseline. | P1 |
| Q-103 | How does another developer download and use the artifact? | Host API/`stc online download` exports the active manifest plus runtime layout summary and a controller/deployment identifier; `stc build --got-manifest <file>` consumes it. If they build without the active manifest or with a stale one, the result is a cold build; the runtime enforces the parent build ID. Target/profile must match. | P1 |
| Q-104 | How is a downloaded manifest trusted and protected? | Treat it as untrusted parsed data; validate every field and digest; optional build hash/signature pin; access-control download because layout data reveals program structure. Never execute manifest data. | P1 |
| Q-105 | Can the manifest reduce or replace the module descriptor/ABI? | Keep a minimal module descriptor for code addresses, sizes, entry points, and the paired digest. Use one model with two projections (descriptor and manifest) rather than making the manifest the sole source of code addresses. | P2 |
| Q-106 | Should slot numbers be globally stable instead of per-build? | No. Keep `GotId` identity plus per-module slot-table mapping. Globally stable sequential slots complicate concurrent builds, removal, and collision handling without removing the need for identity records. | P1 |
| Q-107 | Does the artifact contain runtime values? | No. It contains identities, layout, history, and migration instructions. Values remain in host memory and the retained-state file. | P1 |
| Q-108 | What exactly is a GOT-tracked instance? | Resolved by Q-042/Q-123–Q-125: every persistent field/leaf gets a `GotId` under the recursive per-field model; arrays, strings, and `OVERLAP` follow the resolved hybrid/one-root/atomic rules. Functions/methods/FB bodies, types/descriptors, instances/tasks, and exports remain tracked. Direct `AT` variables remain process-image metadata. | P0 |
| Q-109 | How are untracked removals/renames and retired identities handled? | Diagnose the old identity and new declaration; require `--allow-untracked-removal` or `--cold`. Do not silently discard state. Resurrecting a retired `GotId` requires an explicit option. | P1 |
| Q-110 | Which complex online changes are in scope, and when? | Resolved. M4.5: identity, tracked rename/move, preserve/add/remove, exact layout. M5: aggregate/string/FB roots under the same rules. No field-layout migration in v1; type/layout changes reset the affected root or require a cold download. Resizing and pointer translation are later. | P0 |
| Q-111 | How are concurrent builds from the same baseline handled? | Content-derived `GotId`s avoid accidental collisions. Only the build whose manifest parent equals the active build ID can be an online change; a second build from an older baseline requires a cold download. No merge/rebase is attempted. | P1 |
| Q-112 | How is the artifact downloaded from the PLC in v1? | Local host API and CLI file export/copy; no network deployment protocol in v1. The exported artifact is self-contained and versioned. | P1 |
| Q-113 | Does the manifest enable retained-state migration across restarts? | Not in M4.5. A later state-format revision can key retained leaves by `GotId` plus path; v1 online change preserves in-memory values and keeps the existing layout-hash policy. | P2 |
| Q-114 | What are the artifact size and performance costs? | Manifest size is proportional to roots/functions/types, not leaves. Use canonical JSON for tooling and compact binary for runtime; cache the parsed active record; measure large-program builds and loads. | P2 |
| Q-115 | Should the compiler emit a pre-deployment compatibility report/diff? | Yes. It lists preserved/allocated/dropped/renamed/migrated/rejected instances and warnings, so the operator can review before loading. | P1 |
| Q-116 | Should the compiler generate an ST declaration scaffold from a downloaded manifest? | Optional later. Useful when the original source is unavailable, but it cannot recreate bodies and must be treated as a new source baseline, not a source recovery tool. | P2 |
| Q-117 | What exactly makes a baseline current enough for online change? | Resolved: exact match — manifest parent build ID equals the active build ID, module/manifest digests pair, and profile/target/ABI/layout identity match. A byte-identical module with the active build ID is a no-op. Any other mismatch selects cold download. Ancestor or partial matches are not accepted in v1. | P0 |
| Q-118 | What does a cold download preserve? | It stops/reinitializes the program: non-retained state is initialized from defaults, and `RETAIN` values may be restored from the state file only under the existing layout/profile compatibility policy. Transient module allocations are not preserved. | P1 |
| Q-119 | How does the build and CLI distinguish a cold build from an online-change candidate? | The manifest records no parent for a cold build and a parent build ID/digest for an online build. The compiler emits an explicit status/warning; the runtime enforces the distinction; `--cold` makes the intent explicit. | P1 |
| Q-120 | What exactly does the v1 internal host allocation/rebinding service expose? | Alloc/free only; no module-requested rebinding of GOT cells. Stable, zeroed, host-owned storage; no user-visible pointers or ST syntax; no GC. Resolved. | P0 |
| Q-121 | What are the lifetime rules for internal dynamic allocations across online change? | Allocations are module-version-owned and transient, freed when that version fails, is retired, or is unloaded; they cannot be stored in persistent roots, carried across swaps, or migrated. Resolved. | P0 |
| Q-122 | Where is user-visible `__NEW`/`__DELETE` scheduled and how is it scoped? | A v2 vendor extension with its own profile/design. Not part of v1, M4.5–M8, or the current ST surface; v1 has no syntax for it. | P2 |
| Q-123 | Under recursive per-field roots, how are arrays represented? | Hybrid recommended: arrays of scalars/strings are one root with GEP indexing; arrays of aggregates use per-element handles whose fields are rooted recursively. No per-element GOT lookup for scalar arrays. | P0 |
| Q-124 | How are strings represented under per-field roots? | One root per string value, with length and data internal to that root. No per-character roots. Copy semantics operate on the root; capacity/layout changes are type/layout changes. | P0 |
| Q-125 | How is `OVERLAP` handled under per-field roots? | One atomic overlap root with aliasing field views, not separate field roots. Per-field online change of overlap internals requires reset/cold unless a later overlapping-slot design is approved. | P0 |
| Q-126 | How are `THIS`, object handles, and aggregate references represented? | Opaque handles + object records for objects. `REF_TO` validated references are now place descriptors (Q-158/Q-162), not raw pointers; stack/transient representations remain Q-163. `SUPER`, interfaces, `VAR_IN_OUT` follow the resolved rules. | P0 |
| Q-127 | What are the value-copy/assignment semantics for aggregates under per-field roots? | Per-field recursive copy into the destination roots, preserving destination object identity; nested objects follow the same rule; handles are not copied as storage pointers. No shallow/reference copy for value assignment. | P0 |
| Q-128 | How are per-field keys/paths formed for nested, inherited, and instance fields? | Include the configuration instance, declaring type/base chain, and field names in a normalized path; `GotId`s are recorded per field and survive tracked renames/moves. Root fields and nested fields share the same key scheme. | P1 |
| Q-129 | Does recursive per-field rooting apply uniformly to programs, FBs, classes, and structs? | Yes for persistent declarations, with the Q-123–Q-125 exceptions: scalar/string arrays are one root, aggregate arrays use per-element handles, strings are one root, and `OVERLAP` is atomic. | P0 |
| Q-130 | What are the performance/size budgets for per-field GOT and manifest growth? | Measure field-access indirection, cell count, manifest size, and load time on a representative large PLC program. Cache slot-table/cell pointers within a function; no hard budget until benchmarks exist. | P1 |
| Q-131 | Are the spellings `REFERENCE TO` and `POINTER TO` in scope, or only IEC `REF_TO`? | Resolved. `REF_TO` is the expected spelling; `REFERENCE TO` is not the 2013 spelling. CODESYS-style `POINTER TO` is a distinct transient-only v1 extension; see §6.15 and Q-135–Q-150. | P0 |
| Q-132 | What persistent reference targets are allowed? | `REF_TO` fields may point to stable host leaf roots, or carry handles/place descriptors for aggregate/object targets. References into module-owned data, transient allocations, stack/locals, or unregistered memory are rejected. Removed targets stay quarantined but stale. | P0 |
| Q-133 | How are pointer-bearing roots handled during online change/migration? | Reject or reset pointer-bearing roots in v1; do not raw-copy references. A reference survives only when its target identity/type/layout/storage is preserved. Runtime pointer translation is a later design. | P0 |
| Q-134 | Are references/interfaces/handles persisted in the retained-state file? | No. They remain excluded from the RETAIN payload (A-012/D-032) and are reset or reconstructed on warm start. Persisting stable identities and reconstructing them is a later state-format design, not v1. | P1 |
| Q-135 | Is `POINTER TO` in v1, and in what scope? | Resolved (non-default). `POINTER TO` is a distinct transient-only type in v1 with full arithmetic/indexing/`SIZEOF`; no persistent pointer fields; the safety envelope is Q-143–Q-150. | P0 |
| Q-136 | Which `POINTER TO` operations are allowed? | For a transient subset: `ADR`, `^`, `NULL`, and explicit conversions only; no arithmetic, indexing, `SIZEOF`, or function pointers. Full CODESYS operations remain v2. | P0 |
| Q-137 | May `POINTER TO` appear in persistent fields/manifest-tracked roots? | No for the recommended subset. Persistent pointers cannot be identity-migrated; if later allowed, they require reject/reset and exclusion from state/migration. | P0 |
| Q-138 | Is `POINTER TO` a distinct type or an alias/spelling? | Distinct type if added. Aliasing it to `REF_TO` would misrepresent raw-pointer semantics. `REFERENCE TO` is not the 2013 spelling; it may be an alias only if explicitly approved. | P0 |
| Q-139 | What conversions/interop apply between `POINTER TO`, `REF_TO`, interfaces, and `VAR_IN_OUT`? | Explicit conversions only; no implicit raw-pointer conversion. Interfaces/handles remain the aggregate representation; `VAR_IN_OUT` rules are documented separately. | P1 |
| Q-140 | Does `POINTER TO` bring dynamic allocation (`__NEW`/`__DELETE`) into v1? | No. `__NEW`/`__DELETE` remain v2. V1 pointers, if any, target static/locals/parameters only. | P0 |
| Q-141 | How is `POINTER TO` represented in the profile/conformance claim? | A new vendor extension ID (`STC-POINTER` or similar), documented separately; no IEC 2013 claim for the extension. | P1 |
| Q-142 | What are the online-change/state rules for any persistent pointers if they are later allowed? | Reject/reset pointer-bearing roots, exclude pointers from the state file, and quarantine removed targets. Runtime pointer translation is a later design, not v1. | P0 |
| Q-143 | What types may `POINTER TO` target under transient-only per-field roots? | Contiguous storage only: scalars, strings, scalar/string arrays, and `OVERLAP` roots. Split structs/FBs/classes have no contiguous object representation and are not pointer targets unless a field-view design is approved. | P0 |
| Q-144 | What may `ADR` take the address of? | Locals/parameters, leaf field roots, scalar/string array elements, string internals, and `OVERLAP` regions. Split aggregate objects and arithmetic across field roots are excluded. | P0 |
| Q-145 | What are pointer arithmetic/indexing/bounds semantics? | Typed stride, unchecked like CODESYS, one-past-end permitted, crossing a root/array boundary undefined, null dereference traps. Document the undefined cases and add optional checked access later. | P0 |
| Q-146 | How are pointer conversions and escape prevented? | No implicit pointer conversions; explicit pointers↔`REF_TO` only; no pointer↔integer; sema rejects pointer-typed persistent declarations. Function results/parameters allowed with documented lifetime. | P0 |
| Q-147 | May functions return pointers or take pointer parameters? | Yes for parameters and results, with documented lifetime; a pointer to a local/transient allocation must not escape the call. No persistent storage. | P1 |
| Q-148 | What does `SIZEOF` mean for strings, arrays, and split aggregates? | Size of a contiguous value: elementary, string struct, array storage, or `OVERLAP` region. Split aggregate objects are rejected or return the sum only if an explicit logical-size rule is approved. | P1 |
| Q-149 | Where is `POINTER TO` implemented and how is it claimed in the profile? | New `STC-POINTER` vendor extension, implemented in M5 (not M4.5), with no IEC 2013 claim for the extension. | P1 |
| Q-150 | What diagnostics/traps apply to dangling or escaping pointers? | Compile-time rejection of pointer-typed persistent declarations, a null-dereference trap, and documented undefined behavior for dangling pointers. No general runtime pointer registry/validation in v1. | P1 |
| Q-151 | May `POINTER TO` convert to `REF_TO`? | Yes, via a runtime resolver backed by the GOT and layout descriptors; invalid/null resolution yields `NULL`. Details Q-155–Q-161. | P0 |
| Q-152 | May `REF_TO` convert to an interface? | Allow only when the reference target is a class/FB object type implementing the interface, using the runtime descriptor/`?=` check. Null → null interface; incompatible → null (Q-171). Scalar/string/array/direct references cannot convert. | P0 |
| Q-153 | May `POINTER TO` convert to an interface or object handle? | Disallow. Pointers do not carry a handle/descriptor, and split objects have no contiguous raw-pointer representation. Interfaces/objects stay handle-based. | P0 |
| Q-154 | What runtime validity check applies to any approved reference/interface conversion? | `REF_TO`→interface uses descriptor/`?=` checks. Pointer→`REF_TO` uses the Q-155–Q-161 resolver; no general pointer registry. | P1 |
| Q-155 | What may pointer→`REF_TO` resolve to? | Active host-owned persistent GOT roots **and direct `AT`/process-image variables**; module code/data, stack/locals/parameters, and transient allocations resolve to `NULL`. Direct variables are resolved via process-image metadata. | P0 |
| Q-156 | How are interior pointers validated? | Use manifest layout descriptors to resolve declared array elements, string characters, and `OVERLAP` typed views; undeclared offsets resolve to `NULL`. Exact-root-start-only is the fallback. | P0 |
| Q-157 | What does an invalid pointer→`REF_TO` cast yield? | `NULL`, not a trap. Dereferencing the resulting null reference traps. Provide a validity check/helper for code that needs to distinguish. | P0 |
| Q-158 | How is a validated reference represented for online-change provenance? | Place descriptor (`{GotId, offset}`-style) for validated references, with Root/Direct/Stack/Object kinds as needed. Enables precise identity/remap; changes REF_TO representation and adds indirection. Details Q-162–Q-169. | P0 |
| Q-162 | Is the place descriptor uniform for all `REF_TO` values, or only pointer-derived casts? | Uniform for all persistent-capable `REF_TO`: `REF(place)` and pointer→REF_TO both produce descriptors. Raw pointers remain only inside transient/stack descriptors or as implementation caches. | P0 |
| Q-163 | How are references to locals/parameters/stack handled? | Options: disallow `REF_TO` to stack storage; add a `Stack` descriptor valid only during the call; or keep a tagged raw pointer for stack-only references. Decision required before M5. | P0 |
| Q-164 | How are direct `AT` references identified and remapped? | Give direct variables stable manifest identities; descriptor carries `DirectId`/area+offset+type. Image reconfiguration remaps or nulls; renames/layout changes follow manifest rules. | P0 |
| Q-165 | How are object/interface references represented? | Object `REF_TO` uses an `Object` kind (handle + descriptor) or a `Root` descriptor plus handle lookup. `REF_TO`→interface uses the descriptor check; pointer→interface stays disallowed. | P0 |
| Q-166 | What is the dereference/ABI cost of place descriptors? | Resolve through GOT/process image on dereference; optionally cache a generation-checked raw pointer in the descriptor. Measure before choosing. | P1 |
| Q-167 | Are place descriptors persisted in the state file? | Not in v1 (Q-134). A later state-format revision may persist Root/Direct descriptors with GotIds and reconstruct Stack/Object references; design separately. | P1 |
| Q-168 | How are descriptors remapped on online change? | Root: remap by GotId+offset; removed → null/quarantined; type/layout change → null/reset/reject. Direct: remap image or null. Stack: never persisted. Object: descriptor/handle preserved if compatible. | P0 |
| Q-169 | What resolver API/index and type-matching rule apply? | Host resolver takes pointer + target type/layout, uses an active-root/process-image index, and returns a descriptor or null. Exact type/layout match; no silent reinterpretation. | P1 |
| Q-170 | Are interface values nullable? | Yes, keep the current profile: `NULL` is assignable to interfaces/classes, and `?=` can yield a null interface. A null source `REF_TO` converts to a null interface. | P0 |
| Q-171 | What does `REF_TO`→interface yield for null or incompatible input? | Null under attempt semantics; or trap under a hard-cast policy. Recommended: null for compatibility with `?=`; dereferencing the resulting interface is still the caller's responsibility. | P0 |
| Q-172 | What validity-check syntax is available for references/interfaces? | Use `NULL` comparisons and `?=`; do not require `__ISVALIDREF`. An `__ISVALIDREF`-style helper can be added later as a vendor extension if needed. | P1 |
| Q-173 | May `POINTER TO` become `REF_TO`/interface for object types? | No by default. Objects are handle-based and split into field roots; a pointer to a field is not the object. Allow only an explicit object-handle-cell target if a later design requires it. | P0 |
| Q-174 | Adopt double-buffered GOT activation (normal/temp) instead of in-place cell updates? | Yes. Build a temporary GOT, switch the active pointer at a boundary, observe, copy back, switch back. Keeps the old generation intact for rollback and isolates prepare; costs one extra GOT buffer and an O(N) copy off-thread. | P0 |
| Q-175 | Is a background loader/prepare thread allowed? | Yes: the cycle thread only publishes/switches a fully built generation; load, bind, initialize, copy-back, and unload run off-thread. If the host is strictly single-threaded, the O(N) work adds to cycle time and must be budgeted. | P0 |
| Q-176 | Must every function slot in the temp GOT bind to the new module? | Yes for v1 whole-module loading. Old module code cannot be unloaded while any active temp entry points into it. Incremental reuse with refcounts is not a v1 path. | P0 |
| Q-177 | How do temp/normal GOTs share data roots? | Preserved roots share the same stable storage. The temp GOT adds new roots and leaves removed roots quarantined. Prepare must not write preserved roots until activation. | P0 |
| Q-178 | May new code run during prepare before activation? | Yes, with its own temp GOT context and only for new-root initialization. It must not mutate preserved roots or shared host state the old cycle can observe. Otherwise prepare runs in a stopped boundary window. | P0 |
| Q-179 | How is temp→normal copy-back handled? | Copy in a background/loader thread while temp is active; switch to normal at a later boundary. Serialize a second reload behind the copy. Do not copy synchronously in the cycle thread. | P0 |
| Q-180 | What is the failure/rollback policy? | Keep the old module and normal GOT until the new version completes one successful cycle. Recoverable load/prepare failure switches back; a trap follows the existing fatal-trap policy unless the host has a rollback policy. | P0 |
| Q-181 | When is the old module unloaded? | Off-thread after commit and a grace period, only when no active GOT entry, reference, or descriptor points into it. Optional debug retention for symbolication. | P1 |
| Q-182 | What real-time guarantee applies to reload? | Bounded maintenance jitter, not zero cost: O(1) pointer switch at the boundary; load/bind/init/copy/unload off the cycle thread; measure first-cycle cache/TLB effects and maintenance latency. | P1 |
| Q-183 | Which non-RT services are in v1? | Diagnostics/status plus read-only variable access in v1; variable writes/forcing and full debug/breakpoints in v2. This keeps the current v1 non-goal for online debugging while building the thread/API foundation. | P0 |
| Q-184 | How are non-RT variable reads synchronized? | Generation-pinned typed reads via `GotId`/paths and snapshots; no cycle lock and no raw pointers. Readers retry if the generation changed; aggregates document snapshot/tearing behavior. | P0 |
| Q-185 | How are non-RT variable writes/forcing applied? | Queue requests and apply them at a cycle boundary under host control; or use an explicit stop-the-world debug mode. No concurrent writes into cycle storage. | P0 |
| Q-186 | Is online debugging/breakpoint/forcing in v1? | No; architecture hooks and typed access only. Read-only monitoring can be v1; breakpoints/step/forcing are a later debugger feature. | P0 |
| Q-187 | How do readers pin generations to prevent module unload? | Epoch/refcount pinning: the loader waits for pins before unloading; readers retry when the generation changes. A read lock that blocks swap is not acceptable for RT. | P0 |
| Q-188 | How are diagnostics/status published? | Lock-free counters, double-buffered snapshots, and bounded ring buffers; drop-oldest/coalesce under pressure. Never block the cycle. | P1 |
| Q-189 | What API/security model applies to non-RT access? | Typed path/`GotId` APIs with authentication/authorization, rate limits, and timeouts; no raw memory access in v1. A privileged raw-memory debug API is separate. | P1 |
| Q-190 | How is non-RT work isolated from the cycle? | Lower priority, bounded queues, CPU affinity/priority policy, no cycle-blocking locks, and measured allocator/memory-bandwidth contention. | P1 |
| Q-191 | What does "full debug access in v1" include? | Typed variable read/write/force, watch, breakpoints, stepping, and call stack. If any of these are excluded, it is not "full" debug. | P0 |
| Q-192 | How are breakpoints implemented? | Codegen emits statement/span checkpoints and a breakpoint table; the runtime hook is enabled only in debug mode. No runtime code patching, no separate interpreter. | P0 |
| Q-193 | What are stepping semantics in a cyclic PLC? | Stop-the-world debug session: suspend at a breakpoint, step statement-by-statement, resume the cycle. Outputs/watchdog follow Q-195. Cycle-synchronized and per-task stepping are later. | P0 |
| Q-194 | What are forcing semantics? | Forced values are applied at cycle boundaries, held until explicitly released, excluded from retained state, and invalidated/revalidated on type or online change. | P0 |
| Q-195 | What happens to outputs and the watchdog while stopped? | Stop the cycle, apply configured safe/hold outputs, and pause/extend the watchdog with a maximum debug-session timeout. Do not silently keep outputs live or kill a session unexpectedly. | P0 |
| Q-196 | What debug information is required? | Source maps + manifest field/type/place metadata for typed access; optional DWARF/CodeView for full variable locations and call stacks when built with debug info. Full debug may require the latter. | P0 |
| Q-197 | What debug protocol/security model applies? | Authenticated control channel (local and/or network) with attach/break/step/read/write/force/status commands, audit logging, and a production policy that can disable debug access. | P0 |
| Q-198 | How is debug overhead controlled? | Debug hooks compiled but disabled in production; enable checks only in a debug mode/session, or use a separate debug build. Measure cycle overhead. | P0 |
| Q-201 | While debug holds last outputs, what happens to the watchdog/session? | Hold last outputs, pause/extend the watchdog, impose a maximum debug-session timeout, then apply a configured fail-safe or resume. A lost debugger must not leave outputs held indefinitely. | P0 |
| Q-202 | When are DWARF/CodeView debug sections emitted? | Debug builds or separate debug artifacts by default; production artifacts are stripped. Emitting full debug info in every build is a size/reproducibility cost. | P0 |
| Q-203 | How are split per-field objects/handles represented in DWARF/CodeView? | Standard debug info for contiguous values/locals; split objects/handles via manifest metadata and a debugger extension/pretty-printer. A synthetic contiguous debug mirror or flattened per-field variables are alternatives. | P0 |
| Q-204 | Which debug-info formats/toolchains are supported? | DWARF on Linux and CodeView/PDB on Windows MSVC targets via clang; verify with llvm-dwarfdump/PDB tools. Keep the format target-derived, not hard-coded. | P1 |
| Q-205 | What determinism/stripping rules apply to debug info? | Debug info includes paths and type/field metadata; define reproducible options, separate debug files, and stripping for release artifacts. | P1 |
| Q-206 | How is debug info retained across online change/unload? | Keep debug info for the active generation. A swap ends the debug session; the debugger reconnects to the new generation. Old debug info may be dropped after unload; do not keep old code loaded just for symbolication. | P1 |
| Q-207 | What happens to the watchdog while a debug session holds outputs? | Pause/disable the watchdog for a stopped debug session and hold outputs until the operator resumes. No automatic timeout/fail-safe is required; the project makes no safety claim. | P0 |
| Q-208 | What are the default and `--release` build behaviors? | Debug info is default in normal builds; `--release` is the optional optimized/stripped mode. Define whether release keeps a sidecar symbol artifact and how artifacts/symbols are named. | P0 |
| Q-209 | Are debug and release builds distinct build IDs/manifests? | Yes. Debug info and optimization change the artifact, build ID, digest, and manifest; debug/release must not be treated as the same module. Normalize paths for reproducibility. | P0 |
| Q-210 | What is the dedicated debug milestone called and where is it inserted? | Add a dedicated debug milestone after M5/M6 and before the release candidate, without renumbering existing milestones (for example M6.5), with full debug in the v1 DoD. | P1 |
| Q-199 | How does online change interact with debug sessions? | Every online swap ends the debug session. Breakpoints, holds, and forced values are cleared; the debugger must reconnect to the new generation. A debug session never blocks the swap or module unload. | P0 |
| Q-211 | What happens to forced values and held outputs when a debug session ends (including on swap)? | Release all forces and clear breakpoints at session end; resume normal program writes/cycle. Do not persist forces or carry held outputs into the new generation. | P0 |
| Q-200 | Where does full debug sit in the milestones? | Add a dedicated debug milestone (after the M4.5 foundation and M5 features, before the v1 release) and update the v1 definition of done. Alternatively revert Q-183/Q-186 and defer debug to v2. | P0 |
| Q-159 | What type matching applies to the cast? | Exact runtime target type/layout match with the reference target; no silent reinterpretation. Reinterpret casts need a separate explicit decision. | P0 |
| Q-160 | May a pointer-derived `REF_TO` be stored in persistent fields? | Yes, but the containing root becomes pointer-bearing: no state-file persistence, and migration/reset rules apply. Transient-only is the conservative alternative. | P0 |
| Q-161 | What host API/index supports the resolver, and at what cost? | Add a host resolver service and a range/type index built from active roots and manifest layout descriptors; lookup per cast, no per-dereference cost. Cache/index validation on swap. | P1 |

## 14. Simplification analysis: the persistent GOT manifest

### 14.1 The proposal under review

The clarification proposes a compiler-emitted build artifact that traces and identifies every
GOT-tracked instance. The next build can load that artifact and keep the same identities, which
makes renames, moves, and more complex online changes possible. The artifact is handed to the
runtime, stored there, and can be downloaded by another developer as the baseline for their own
build and online change.

This section asks whether that is a simplification, what it buys, and what it costs. The short
answer: it does **not** remove the GOT/module architecture, and it is not a simplification of
the total system. It is a **reallocation of complexity** from the runtime to the compiler, where
ST type and layout knowledge already lives, and it replaces runtime guesswork with an explicit
compiler-produced plan. It also unlocks workflow capabilities that runtime-only matching cannot
provide. The recommendation is to adopt it, with the scope guardrails in §14.5.

### 14.2 What the manifest simplifies

| Area | Without a persistent manifest | With a persistent manifest |
| --- | --- | --- |
| Identity across builds | Runtime compares current names/type hashes; a rename is a remove plus add | Compiler carries `GotId`s forward; runtime matches by identity |
| Rename/move | State is lost unless the runtime supports a special rename protocol | Compiler records the old key in history; storage is reused; no data copy |
| Complex data changes | Runtime must infer type compatibility; structural change is usually rejected | Compiler knows both layouts and emits an explicit migration plan; runtime validates and executes |
| Cross-developer workflow | Developer needs the exact old source and a matching build; the PLC has no identity artifact | Developer downloads the active manifest, builds against the running layout, and reviews a compatibility report |
| Runtime type knowledge | Runtime needs structural type/layout comparison and domain-specific diagnostics | Compiler emits actions and reasons; runtime performs generic validation and execution |
| Pre-deployment confidence | Incompatibility is discovered when loading the module | Compiler reports preserve/add/drop/rename/migrate/reject before deployment |
| Failure recovery | The runtime needs policy for stale, partial, or merged baselines | Missing/stale/invalid/non-current baselines fail safe to cold download; there is no recovery/merge path |
| State-file evolution | State layout is tied to a build hash with no identity history | A later state format can key retained leaves by `GotId`, surviving renames and approved migrations |
| Module ABI | More metadata must be squeezed into the module descriptor | The descriptor stays minimal (addresses, sizes, digest); the manifest carries identity/history |

The most important simplification is not code size. It is that the **compiler becomes the
authority on what a change means**, and the runtime becomes a generic executor of an explicit,
validated plan. That matches the existing architecture, where sema and codegen already own the
type and layout knowledge.

### 14.3 What the manifest complicates

| Risk | Why it matters | Mitigation |
| --- | --- | --- |
| Non-hermetic builds | Output now depends on a baseline artifact, not only source | Baseline is an explicit input; its digest is recorded; cold builds are reproducible from source alone; CI tests both modes |
| Artifact lifecycle | The manifest can be lost, stale, forked, committed, or overwritten | Versioned format; stored with module; runtime keeps the active copy; missing/stale/corrupt forces cold download; `stc clean` policy; digest/lineage checks |
| Source/manifest divergence | A downloaded manifest may describe a different source tree | If the manifest is not the active baseline, the runtime requires cold download; the compiler still warns about unmatched identities |
| Migration correctness | Compiler-generated field copies/conversions can corrupt state if wrong | Start with preserve/add/drop/rename; pointer-free same-size migration only; full validation and rollback; extensive tests |
| Pointer-bearing roots | Migrating references/interfaces/object pointers can produce dangling references | Reject/reset in v1; pointer translation is a later design item |
| Concurrent builds | Two developers can derive new identities from the same baseline | Content-derived `GotId`s; only the build whose parent equals the active build ID is an online change; stale baselines force cold download |
| Trust and IP exposure | Downloaded manifests are input data and reveal program structure | Parse/validate only; optional hash/signature pin; access-controlled download; never execute manifest data |
| Format evolution | Old/new compiler and runtime versions must interoperate | Versioned manifest, module ABI, and host ABI; reject unknown required features |
| Duplicate schemas | Manifest and module descriptor can drift into two truths | One in-memory model, two deterministic projections, paired by digest; tests enforce equality |
| Tooling surface | More commands, options, and failure modes | Keep v1 CLI minimal: build baseline, cold build, rename map, download; add diff/scaffold later |

### 14.4 Alternative simplifications considered

| Alternative | Benefits | Risks | Verdict |
| --- | --- | --- | --- |
| **S0. Platform dynamic-linker symbols as the GOT** | No custom GOT/catalog; standard loader resolves names | No stable type/layout/rename semantics; host must export every root; Windows import libraries; symbol collisions; unload/rebind still needs a manifest-like layer | Reject as the primary design |
| **S1. One runtime-owned state struct/arena and a base pointer** | Fewer cells and loads; one allocation | Adding/reordering variables shifts offsets; fixed layout across versions; moving/compaction breaks pointers; references and structural changes become harder; no per-root quarantine | Reject for mutable roots; acceptable for immutable tables later |
| **S2. Runtime-only key/layout matching (the previous plan)** | Hermetic builds; fewer artifacts; simpler compiler/driver | No tracked rename, no cross-developer baseline, no complex migration; runtime needs type inference; all structural changes are cold | Keep as fallback if the manifest is rejected |
| **S3. Persistent manifest + compiler-driven plan (recommended)** | Identity across builds, renames/moves, download/handoff, explicit migration, compiler-side diagnostics, runtime simplification | Build-state lifecycle, migration codegen, trust, concurrency, schema pairing | Adopt as proposed; Q-096 is the approval gate |
| **S4. Globally stable numeric slot IDs from the manifest** | No per-module slot mapping; flat runtime array | Concurrent builds collide; holes/growth; manifest becomes mandatory for every build; merges are painful; identity still needed for renames | Reject; keep `GotId` + per-module slot table |
| **S5. Manifest identity/rename only, no migration** | Most workflow benefits with low migration risk; incremental | Structural changes still cold/reset; less than the full proposal | Adopt as the M4.5 scope, with migration phased later |
| **S6. Heuristic rename detection from source diff/similarity** | Less user input | Can bind the wrong storage and silently corrupt state; non-deterministic across tools | Reject; explicit renames only |

The recommended combination is **S3 with S5 as the first delivery increment**: build the
identity/history artifact and tracked renames first, keep runtime validation and strict
preservation. Field migration is not in v1; it would be a later design if the project needs it.
This gives the workflow benefits immediately without betting the foundation on complex migration.

### 14.5 Scope guardrails and approval gate

If the project approves the manifest, it is incorporated under these constraints:

1. The manifest is an explicit build input; no hidden compiler cache state. A build records the
   baseline digest, and a cold build is still possible and reproducible.
2. The runtime never operates without an active manifest. A cold build emits one; a legacy
   module gets a synthesized one at load.
3. The manifest carries identities, layout, history, and migration instructions, never runtime
   values.
4. `GotId` is the identity; current keys are locators. Renames/moves are explicit; there is no
   heuristic rename detection.
5. Per-module slot numbers remain local; code does not depend on globally stable numeric slots.
6. The runtime validates every manifest and plan field against the active layout. A compiler plan
   is not trusted blindly.
7. V1 migration scope is preserve/allocate/drop/rename with exact layout and per-field
   add/remove/reorder. Field type/layout changes, arrays/strings/OVERLAP changes, and resizing
   reset or require a cold download; field migration is a later design.
8. Manifest and module are paired by digest. Missing, stale, invalid, unpaired, or non-current
   pairs select cold download; they are never recovered, merged, or partially applied.
9. Download is a controlled operation because the manifest reveals program structure.
10. The module descriptor remains a minimal runtime binding contract; the manifest and descriptor
    are projections of one model.
11. A cold download is a first-class, tested path, not an error afterthought. It initializes
    non-retained state, applies the retained-state compatibility policy, and installs the new
    manifest as the active baseline.

**Approval:** the recommended answer is to adopt this design (Q-096). If it is rejected, the
plan falls back to S2: runtime-only key/layout matching, no persistent manifest, no tracked
renames, no cross-developer baseline, and structural changes require cold downloads.

### 14.6 Accepted cold-download policy and its effect on the plan

The accepted policy is: **if a valid, paired, currently active baseline manifest is not
available, do not attempt an online change; perform a cold download.** This simplifies the
following parts of the design:

- **Removed:** ancestor/partial-baseline matching, plan rebasing, merge/conflict resolution for
  concurrent builds, the source-fingerprint acceptance gate, best-effort recovery from corrupt
  manifests, legacy modules participating in online change, and the requirement to retain every
  historical manifest for arbitrary future online transitions. Source fingerprints remain useful
  for diagnostics and untracked-change warnings. Rollback still needs the immediately previous
  module/manifest pair if it is to be an online rollback; otherwise it is a cold download.
- **Kept:** active-manifest storage and download, exact parent/build matching, digest pairing,
  migration and rollback when the baseline *is* current, untracked-rename/removal diagnostics,
  and audit/lineage records.
- **Strengthened:** the cold-download path is now a first-class, tested feature rather than an
  error fallback. It must define startup/shutdown behavior, non-retained state reset, retained
  state compatibility, process-image/task reinitialization, and installation of the new
  manifest as the active baseline.
- **Operational tradeoff:** losing or corrupting the active manifest costs a cold restart. That
  is accepted, but it makes manifest persistence and download reliability important even though
  the recovery logic is gone.
- **Questions:** Q-117 (exact currentness), Q-118 (what cold preserves), Q-119 (build/CLI
  distinction).

This policy does not remove the hard parts of a *valid* online change: layout migration,
pointer-bearing roots, rollback snapshots, OOP descriptor updates, and state-file compatibility.
It removes the much broader problem of recovering or reconciling an invalid or stale baseline.

## 15. Notes on the highest-impact questions

### Q-018 / Q-090 / Q-091 — scope and sequencing

The decision is not only whether online change is required, but when the architecture is
introduced. M4's scalar surface has a small number of global, instance, and call sites. M5 adds
methods, FB bodies, interface dispatch, vtables, aggregate roots, string state, references, and
retained leaves. Converting M4 now touches a bounded amount of code and tests; converting after
M5 touches every M5 feature as well. A final decision to defer online change is valid, but it
should be recorded explicitly so M5 is not later described as foundation-ready.

### Q-020 — artifact model

The options are:

1. **Host launcher + adjacent module (recommended).** `exe` is a thin host-like launcher that
   loads `<name>-<build-id>.dll`/`.so`. One code path; online change works from the start.
2. **Standalone executable with linked code.** Familiar M4 behavior, but the process cannot
   replace its own code; online change requires a different host executable.
3. **Hybrid: embedded initial module plus later loadable updates.** Best standalone story, but
   requires extracting or memory-loading the embedded module and keeping two registration paths.

Option 3 is the most user-friendly long term and the most work now. The plan recommends option 1
for v1, with option 3 as a later packaging feature.

### Q-026 — GOT scope

"All memory and function pointers through the GOT" can mean:

- **Strict:** every global access and every call is indirect, including compiler helpers.
- **Persistent-boundary (recommended):** every access that can outlive or cross a module
  version is indirect; stack, version-local helpers, intrinsics, and read-only constants are
  direct.
- **Escaping-only:** only data roots and function pointers that can be stored in persistent state
  are indirect; internal calls and internal roots stay direct.

The escaping-only option is fastest but does not satisfy a literal reading of the requirement and
makes the invariant harder to test. The persistent-boundary option satisfies the safety and
hot-swap goal while keeping compiler internals simple. If the project wants literal strictness,
the foundation can route every call and root access through cells; the cost is primarily
performance and larger IR, not correctness.

### Q-032 / Q-120 / Q-121 — host-only writes and transient allocation

The resolved direction is:

- Generated code never writes a GOT cell directly.
- A module may call a narrow host allocation service for its own transient needs. Q-120 resolved
  the API to alloc/free only; there is no module-requested GOT cell rebinding.
- The service returns stable, zeroed, host-owned blocks and supports explicit free.
- Q-121 resolved the lifetime rule: allocations belong to one module version, are freed on
  failed prepare, retire, or unload, and are not manifest roots or persistent state.
- V1 has no ST-visible dynamic allocation. TwinCAT-style `__NEW`/`__DELETE` is a separate v2
  vendor extension (Q-122) and does not change A-007/D-023 or the v1 ST profile.

Impact on the rest of the plan is bounded: root allocation, manifest identity, preservation, and
migration are unchanged. The host API, module ABI, unload/reclamation, tests, and security limits
gain a transient-allocation category. The v2 extension would require a separate memory/lifetime
design.

### Q-030 / Q-031 / Q-070 — GOT placement, cell shape, and unload

A shared cell table owned by the host is what makes unload safe. If each module owned its own
GOT, a preserved vtable or interface value could point at the old module's table or code. With
host-owned cells:

- preserved data pointers are stable;
- preserved function references are cell pointers and remain valid after a swap;
- unload removes code but not the cells;
- rollback restores cell values.

The cost is an extra load per call/access. The builder can cache the slot-table pointer and, where
safe, a root storage pointer within a function. It must not cache cell values across cycles.

### Q-036 / Q-037 / Q-038 — identity and the GOT manifest

Arena IDs are build-local. Mangled symbols are stable for functions but do not cover variables,
instances, tasks, or types. The runtime needs identities that survive file reordering, unrelated
source edits, declaration reordering, generated/synthetic definitions, and package additions.

The recommended model separates **locator** from **identity**:

- The current key is the normalized qualified path/signature used to find a declaration in the
  source tree.
- The persistent `GotId` is recorded in the GOT manifest and is what the runtime matches.
- A tracked rename or move changes the current key while keeping the `GotId`.
- A type's identity includes its qualified name plus a structural layout hash, size, and target
  triple; a layout change is a migration/reset/reject decision, not a silent new type.

This separation is what makes renames and cross-developer builds possible. Without it, the
runtime can only match by current path and must treat every rename as a remove plus add.

### Q-096 / Q-101 / Q-110 — manifest scope and migration

The manifest is a genuine simplification of the runtime and the operator workflow, but it adds
a compiler-side persistent build artifact and a migration language. The recommended staging is
deliberate:

- M4.5 proves identity, tracked rename/move, preserve/allocate/drop, and manifest
  export/download with scalar roots. It does not attempt field migration.
- M5 applies the same records to aggregates, strings, FB/class state, and OOP descriptors. Exact
  layout preservation remains the default; migration is still not implied.
- M6 and later may add compiler-generated field-level migration for approved, pointer-free
  cases and a compatibility report; field migration is not in v1. Resizing and pointer
  translation remain later work.
- If any migration case is not explicitly planned and validated, the module is rejected or the
  affected root is reset under an explicit policy. There is no automatic "best effort" migration.

This staging gives the workflow benefits early and keeps the highest-risk data transformation
behind an approval gate.

### Q-098 / Q-117 / Q-118 — fail-safe cold policy

Accepting that a missing, invalid, unpaired, or non-current manifest requires a cold download
removes several complication classes:

- no baseline discovery fallback or "online intent" guessing;
- no stale-baseline recovery, merge, or rebase logic;
- no source-fingerprint acceptance workflow for a build that is not actually an online candidate;
- no concurrent-build conflict resolution beyond comparing the parent build ID;
- no legacy-manifest synthesis path for online change (a legacy module is cold-installed);
- no ambiguous runtime state after a partial or mismatched baseline.

What it does **not** remove:

- the need to store, version, validate, pair, and download the active manifest;
- the need to protect it and to treat it as untrusted input;
- the migration machinery and rollback snapshots when the manifest *is* current;
- the need to test the cold path and to define what cold preserves (defaults and compatible
  `RETAIN` state, never non-retained running values).

The recommended rule is exact currentness (Q-117): the manifest parent build ID must equal the
active build ID, and module/manifest digests must pair. Anything else is cold. This is the
simplest defensible policy and turns whole classes of artifact-history problems into one
predictable operation.

### Q-040 / Q-043 / Q-044 — allocation and quarantine

Freeing removed storage is unsafe in the presence of any retained pointer, and the compiler
cannot generally prove that no reference exists. Since ST has `REF_TO`, interfaces, object
pointers, and `VAR_IN_OUT`, and any of those may be stored in a preserved root, v1 should not
free removed root storage. Quarantine is a deliberate, documented memory-growth tradeoff: the
host warns and reclaims only on restart or explicit reset. A later reference registry or
lifetime analysis could allow reclamation, but it is not a v1 requirement.

### Q-046 / Q-047 / Q-048 / Q-049 — preservation and compatibility

The safest v1 rule is:

- preserve every root whose manifest `GotId` is present and whose type key, layout hash, and
  size match;
- initialize every new root exactly once;
- quarantine removed roots and record the retirement in the manifest;
- reject the whole module for any preserved-root layout change unless an explicitly approved
  compiler-generated migration plan covers it;
- provide an explicit reset or cold-download escape hatch.

In v1, adding or removing a field is a per-field preserve/drop operation; renaming without a
tracked rename is diagnosed and requires an explicit choice. Changing a field type, a string
capacity, or an array/`OVERLAP` layout is not an online change: the affected root is reset under
an explicit policy or the module needs a cold download. Field-level migration is a later design;
if it is required for the target application, it must be designed before the milestone that
needs it. The persistent manifest is what makes such migration possible without guessing in the
runtime.

### Q-056 / Q-064 — host services and no imports

The host API table is the cleanest cross-platform boundary:

- Windows does not need an import library or exported runtime symbols from the host executable.
- Linux does not need `-rdynamic` or fragile global symbol lookup.
- The runtime ABI is versioned in one struct.
- Tests can provide mock host services and deterministic clocks.

The tradeoff is that every runtime call is an indirect call through the table; this is the same
order of overhead as the GOT and is acceptable for the scalar runtime services.

### Q-057 / Q-058 / Q-061 — OOP across swaps

A runtime-owned descriptor is the only representation that survives unload safely:

- an instance stores a data pointer and a descriptor pointer;
- interface values store a data pointer and a descriptor pointer;
- vtables contain cell pointers, not raw code;
- a swap updates function cells and leaves descriptors valid when the shape is unchanged;
- a shape change is rejected for live types in v1.

If descriptors were module-owned, preserved instances and interface values would point into the
old module, and unload would be unsafe. That is why the M5 handover's recommendation to emit
module-owned vtables should be superseded.

### Q-072 / Q-073 / Q-074 — swap boundary, tasks, and I/O

The cycle boundary is the only safe point for activation:

- the old version completes its outputs and state updates;
- the host has published outputs;
- the new version is installed;
- the host latches fresh inputs at the next cycle start.

Task topology and scheduling changes are not safe to introduce in the middle of a running
schedule in v1. They require a cold download. Clock continuity and a bounded watchdog pause keep
timer behavior understandable across the swap.

### Q-084 / Q-086 / Q-087 — verification and security

The feature is credible only if tests can observe each phase. The hot-swap suite must be part of
the normal CI matrix, not a manual demo. Determinism must include the descriptor and slot table,
not only the textual IR. The loader must be treated as a parser of untrusted bytes even though
modules are trusted code; malformed metadata is the realistic corruption/fuzz case.

## 16. Status and next steps

All questions Q-018–Q-212 are resolved. The planning and documentation phase is complete:

1. Resolved answers are recorded as A-022–A-030 and implementation decisions as D-094–D-108 in
   `README.md` and `Specification/Decisions.md`, including the non-default Q-032, Q-033, Q-042,
   Q-095, Q-135–Q-138, Q-183/Q-186, Q-195/Q-196, Q-199/Q-201, and Q-202 decisions.
2. The §12 documentation checklist is applied: the project plan, conformance profile, and
   architecture/runtime/state/I/O/CLI/testing/AST documents are updated; the new
   [Online Change and GOT](Specification/Architecture/Online-Change-and-GOT.md) and
   [Debug and Monitoring](Specification/Architecture/Debug-and-Monitoring.md) specifications
   are added; the non-safety stance and debug-by-default/`--release` policy are recorded.
3. `M4.5-Plan.md` is written with the manifest identity/rename scope, per-field scalar roots,
   the M4-surface conversion, and the §9.2 exit criteria/§10 breakdown.
4. `M6.5-Plan.md` is written for the full debug/monitoring surface, with M6.5 between M6 and M7.
5. `handover_to_M5.md` carries an online-change addendum so M5 cannot reintroduce module-owned
   state, direct persistent references, module-owned vtables, contiguous-object assumptions, or
   path-only identities.
6. Markdown lint and `python tools/ci/check_docs.py` pass on the updated documentation set.

**Next decision:** approve `M4.5-Plan.md` and begin the M4.5 implementation, or explicitly defer
online change. Do not start M5 on the M4 direct-reference/contiguous-object model. The existing
M4 branch remains a valid scalar codegen milestone; this plan does not invalidate its semantics,
only its artifact-lifetime, root-granularity, and object-representation model.

**Update 2026-10-24 (M4.5 phase 0/1):** phase 0 is complete. Phase 1 fixes the artifact contracts
in [GOT Artifacts and Schemas](Specification/Architecture/GOT-Artifacts.md) and records decisions
D-113–D-117: normalized stable keys and `GotId`, canonical binary/JSON encodings and digest
chain, pointer-free module descriptor format 1, host ABI 2.0, manifest/plan format 1, build-ID
rules, and artifact pairing.

**Update 2026-10-24 (M4.5 phase 2):** the compiler identity layer is implemented in the
`stc-got` crate: stable-key and `GotId` derivation, catalog records, binary and canonical JSON
manifest codecs, digest and structural validation, baseline matching, tracked rename/move maps,
preserve/allocate/drop/reject classification, canonical slot assignment, source fingerprint and
build-ID derivation, and first/second-build determinism, rename, removal, rejection, and
tamper-test coverage. Decisions D-118 records the phase-2 format clarifications. Phase 2 and
D-118 were reviewed and approved on 2026-10-24.

**Update 2026-10-24 (M4.5 phase 3):** the module IR builder is implemented in `stc-codegen`:
GOT slot-table access (cell address and cell-value loads), indirect calls through function/import
cells, exported `dso_local dllexport` `stc_module_*` entry points, private descriptor blobs,
function-address tables, and linkage/visibility attributes. A structural verifier checks the
build contract (no direct persistent-root/callable references, cell-kind-correct loads/stores and
calls, allowlisted module-local globals, ABI 2.0 entry-point signatures, descriptor/table shapes).
Builder, determinism, rejection, and clang-acceptance tests run on the host and both x64 targets;
D-119 records the IR-level choices.

**Update 2026-10-25 (M4.5 phase 4):** the compiler artifact projection is implemented. The
`stc-got::descriptor` module encodes/decodes descriptor format 1, computes its zeroed-digest
projection, validates canonical function/slot tables, and checks descriptor/manifest pairing
(§11.4). `stc-codegen::module` builds the M4 scalar catalog (global, program-field, direct, and
task-guard roots; user and program-body functions; instances, tasks, and exports), obtains the
canonical slot assignment, exposes declaration-to-slot bindings for the module-shaped lowering,
and emits the descriptor, manifest binary/canonical JSON, and online-change plan from one
deterministic model. Configuration-less builds synthesize `_DEFAULT` configuration/resource/task
identities; imports are trap, I/O image/size, and retained-state load/save. Golden
descriptor/manifest snapshots, double-build byte equality, second-build baseline reuse,
rename/drop/reject/cold paths, and tamper rejection are covered. D-120–D-121 record the choices.
Phase 5 (host GOT, root registry, allocator, and artifact validation) is next; IR execution
lowering and runtime integration have not started.
