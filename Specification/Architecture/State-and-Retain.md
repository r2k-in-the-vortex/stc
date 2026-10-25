# State and RETAIN Persistence

Status: M4.5 scalar roots and online preservation implemented (2026-10-25); file-backed state is M6  
Profile: `stc-2013-oop-utf8-v1`  
Related: [Runtime](Runtime-and-Execution.md), [Online Change and GOT](Online-Change-and-GOT.md), [CLI/Manifest](CLI-and-Manifest.md), [Architecture](Architecture.md), [Decisions](../Decisions.md)

## 1. Scope

IEC 61131-3:2013 defines `RETAIN` and `NON_RETAIN` for `VAR`, `VAR_INPUT`, `VAR_OUTPUT`, and
`VAR_GLOBAL` sections, including FB/class/program instances. `stc` implements file-backed retained
state for executable mode and exposes the same primitives to library hosts.

A start with a valid state file is a **warm restart**; a start without one (or with
`--reset-retain`) is a **cold restart**. Warm restart restores retained values; cold restart
initializes declared/type defaults.

An **online attach** is neither: the host is already running, so in-memory values win and the
state file is not loaded. Matching per-field roots are preserved, new roots are initialized, and
removed roots are quarantined. A **cold download** stops/reinitializes the program and applies
the restart policy above. See [Online Change and GOT](Online-Change-and-GOT.md) §8.

> **M4.5 status (2026-10-25):** scalar-leaf `RetainLayout` is computed (retained globals and
> program fields, `AT` excluded), emitted as metadata, and exposed through generated
> `stc_retain_size/layout_hash/save/load`. Retained leaves are host-owned per-field roots.
> `stc_init` calls the ABI 2 `state_load` service during prepare and restores on success;
> shutdown saves through `state_save`. The runtime state services are stubs returning 5
> (retention disabled) until M6. Online prepare preserves matching roots and initializes only
> fresh roots; cold installs skip the retained-state file. Aggregate/string/FB leaves are M5;
> the file-backed atomic format is M6.
>
> **Online-change policy (resolved):** v1 preserves values in host memory across an online
> change; it does not migrate retained leaves by `GotId` across restarts. The state-file format
> keeps its layout-hash policy. A later format revision may key leaves by `GotId` plus path, which
> the per-field manifest already records.

## 2. Which variables are retained

### 2.1 Effective retain policy

For every storage leaf the compiler computes an effective policy:

1. An explicit `RETAIN`/`NON_RETAIN` on the variable declaration wins.
2. Otherwise the enclosing section qualifier applies (`VAR RETAIN`, `VAR NON_RETAIN`, etc.).
3. For an FB/class/program instance variable with a qualifier, the qualifier applies to all
   instance fields, except fields with an explicit qualifier and nested FB/class instances, which
   use their own type declarations.
4. Arrays and structs: all elements inherit the effective policy.
5. If no qualifier applies, the v1 default is **NON_RETAIN** (the 2013 standard leaves this
   implementer-specific).

### 2.2 Allowed locations

- `VAR`, `VAR_INPUT`, `VAR_OUTPUT`, `VAR_GLOBAL` in programs, FBs, classes, and configurations.
- FB/class/program instance variables.
- `VAR_TEMP` cannot be retained (`STC-E-RETAIN-002`).
- `VAR_IN_OUT` and `VAR_EXTERNAL` aliases cannot be retained (`STC-E-RETAIN-002`).
- Function-local `VAR` cannot be retained; functions have no persistent storage
  (`STC-E-RETAIN-002`).
- Direct (`AT`) variables are accepted syntactically but are **excluded** from file-backed state
  (`STC-W-RETAIN-001`). The runtime/backend owns their values.

### 2.3 Persistable leaves

Retained storage is copied as raw bytes of its data fields:

- elementary types, bit strings, enums, subranges, date/time types;
- `STRING`, `WSTRING`, `USTRING` (the fixed-capacity root value is copied);
- arrays and structs (recursively);
- FB instance data fields and class owned-instance data fields (recursively).

Not persistable:

- `REF_TO` values, interface references, `VAR_IN_OUT` parameters;
- class handle roots and object-record roots (hidden `HANDLE`/`OBJREC` roots; fresh ones are
  initialized by prepare and none are saved, while an owned instance's field roots are ordinary
  retained leaves);
- vtable/type-descriptor pointers and other runtime pointers (these are reinitialized by
  `stc_init` before state is loaded; they are excluded from the payload);
- variables containing any non-persistable field (`STC-E-RETAIN-001`).

The compiler emits a **retained layout**: an ordered list of leaf fields with path, type, byte
offset within its root, and size. The layout is the contract between generated code and the
runtime. Under the per-field root model, each retained leaf is copied from its host root; a leaf
cannot contain a persisted raw reference/pointer. The owning root's `GotId` is recorded in the
GOT manifest (and is available for diagnostics and a possible later state-format migration); v1
still matches the whole layout hash.

## 3. State file format

Little-endian binary:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 8 | magic `STCRET01` |
| 8 | 4 | format version (`1`) |
| 12 | 4 | runtime ABI version (`2`) |
| 16 | 8 | profile hash (FNV-1a of the profile ID) |
| 24 | 8 | layout hash (FNV-1a of the canonical retained layout) |
| 32 | 8 | payload length |
| 40 | N | payload bytes |
| 40+N | 4 | CRC32 (IEEE) over bytes 0 through 40+N |

- The payload is the concatenation of retained leaf bytes in canonical layout order (paths
  sorted lexicographically, then offset).
- `payload length` must equal `stc_retain_size()`; a mismatch is corruption.
- CRC32 detects corruption/truncation; it is not a security mechanism.
- The format is versioned. A future version may add fields/migration; v1 rejects unknown
  versions.
- The format is little-endian. Windows x86-64, Linux x86-64, and Linux AArch64 are all
  little-endian, so a state file is portable across supported hosts when the profile and layout
  hashes match (D-042).

### 3.1 Layout hash

The layout hash covers:

```text
profile=stc-2013-oop-utf8-v1
abi=2
size=<payload length>
leaf=<path>|<got_id>|<type>|<offset>|<size>
...
```

Paths use `.` for fields/elements and `[i]` for array indices, for example
`Main.Timer.State`, `Main.Buffer[3]`. The hash is stable across builds with the same layout.
Adding/removing/reordering a retained field changes the hash.

## 4. Lifecycle

### 4.1 Cold start / warm start

1. `stc_init` zero-initializes memory and applies declared/type initial values.
2. It initializes vtables/type descriptors and other runtime pointers.
3. If retention is enabled and not reset, it calls `stc_rt_state_load` to read and validate the
   state file. On success it calls `stc_retain_load` to apply the payload.
4. Missing file: cold start; defaults remain.
5. Layout/profile/format mismatch or corruption: apply the configured policy:
   - `error` (default): trap `STC-E-RUN-RETAIN-001` and do not run. `--reset-retain` is the
     escape hatch.
   - `reset`: emit a warning, keep defaults, and overwrite the file on shutdown.
6. `stc_module_cycle` runs normally; retained values are ordinary storage.
7. `stc_module_shutdown` calls `stc_retain_save` to fill the buffer and the host's
   `state_save` service to write it atomically.

#### 4.1.1 Online attach and cold download

- **Online attach:** the host does not call `stc_rt_state_load`; running values win. It preserves
  matching roots, initializes new roots once, and quarantines removed roots. The state file is
  neither read nor rewritten merely because a swap occurred; the normal shutdown save still
  applies.
- **Cold download:** the host stops the program, applies defaults, and may load `RETAIN` values
  only under the existing layout/profile policy. Non-retained running values and transient
  allocations are not preserved.
- A baseline/module mismatch does not attempt state recovery; it selects cold download
  (see [Online Change and GOT](Online-Change-and-GOT.md) §8).
- The online plan can reset an affected retained root only under an explicit per-field policy;
  otherwise an incompatible layout rejects the module.

### 4.2 Atomic save

- Write to `<state-file>.tmp` in the same directory, flush/sync, then rename over the target.
  Keeping the temporary file in the same directory makes the rename a same-filesystem operation.
- Replace-on-rename is provided by the runtime's host platform layer on Windows and Linux; a
  failed replace leaves the previous state file intact and reports `STC-E-RUN-RETAIN-003`
  (D-042).
- v1 saves at normal shutdown (including handled interrupt). A crash or power loss can lose the
  latest cycle's state; periodic saving is a future option.

### 4.3 CLI and manifest controls

```toml
[retain]
enabled = true
file = "motor.stcstate"     # relative to the project root; default <artifact>.stcstate in out-dir
on-mismatch = "error"       # error | reset
```

CLI overrides:

```text
--state-file <path>   use a specific state file
--reset-retain        cold start: ignore existing state, overwrite on shutdown
--no-retain           disable load and save for this run
```

`stc clean` does not delete state files; they are user data.

## 5. Generated ABI

The generated module exports:

```c
int64_t stc_retain_size(void);
int64_t stc_retain_layout_hash(void);
int32_t stc_retain_save(void* buffer);
int32_t stc_retain_load(const void* buffer);
```

The module also emits a retained-root descriptor of `stc_retain_size()` bytes. Under the host
model the buffer is filled from/into host-owned roots; the generated functions are version-local
helpers, not the owner of the storage. Generated `stc_init` and `stc_module_shutdown` use the
host API's `state_load`/`state_save` services (host ABI 2.1; see
[GOT Artifacts and Schemas](GOT-Artifacts.md) §8):

```c
int32_t state_load(uint8_t* buffer, int64_t len, int64_t layout_hash);
int32_t state_save(const uint8_t* buffer, int64_t len, int64_t layout_hash);
```

Return codes:

| Code | Meaning |
| --- | --- |
| 0 | OK |
| 1 | file not found (cold start) |
| 2 | layout/profile/version mismatch |
| 3 | corrupt/truncated payload or CRC failure |
| 4 | I/O error |
| 5 | retention disabled |

Generated initialization (called from `stc_module_prepare`):

1. initialize defaults and vtables for fresh roots;
2. if the host's `state_load` service returns OK, call `stc_retain_load`;
3. otherwise leave defaults (missing) or apply the runtime policy (mismatch/corrupt).

Generated `stc_module_shutdown`:

1. call `stc_retain_save` into the buffer;
2. call the host's `state_save` service.

Library hosts may bypass the file helpers and use `stc_retain_size/save/load` with their own
storage. The host owns path/atomicity/policy in that case.

## 6. Metadata

`<name>.stc-meta.json` includes:

```json
{
  "retain": {
    "size": 128,
    "layout_hash": "0x9f2c...",
    "leaves": [
      { "path": "Main.Counter", "type": "DINT", "offset": 0, "size": 4 },
      { "path": "Main.Timer.Elapsed", "type": "TIME", "offset": 8, "size": 8 }
    ]
  }
}
```

- `leaves` is sorted by path/offset and is the human-readable form of the layout hash input.
- The runtime uses `size`/`layout_hash` with the generated functions; it does not interpret leaf
  types. Root identities (`GotId`) live in the GOT manifest, not in the leaf list.
- M4.5 emits `size`, `layout_hash`, and `leaves` for scalar programs; `file` and `on_mismatch`
  are part of the M6 file-backed state policy. Aggregate/FB leaf paths such as
  `Main.Timer.Elapsed` are M5.

## 7. Diagnostics

| Code | Condition |
| --- | --- |
| `STC-E-RETAIN-001` | a retained variable/field is not persistable (reference/interface/pointer) |
| `STC-E-RETAIN-002` | invalid RETAIN location (`VAR_TEMP`, `VAR_IN_OUT`, function local, etc.) |
| `STC-W-RETAIN-001` | RETAIN on a direct `AT` variable is ignored (runtime owns it) |
| `STC-E-RUN-RETAIN-001` | state layout/profile/version mismatch under `error` policy |
| `STC-E-RUN-RETAIN-002` | corrupt/truncated state or CRC failure under `error` policy |
| `STC-E-RUN-RETAIN-003` | state file write failure |
| `STC-W-RUN-RETAIN-001` | cold start (state file missing) |
| `STC-W-RUN-RETAIN-002` | state reset due to mismatch/corruption under `reset` policy |
| `STC-E-ONLINE-STATE-001` | retained root layout is incompatible and no explicit reset policy applies; reject/cold |
| `STC-W-ONLINE-STATE-001` | retained root reset under an explicit per-field online-change policy |

## 8. Testing

- Round-trip: scalars, enums, subranges, strings (`STRING`/`WSTRING`/`USTRING`), arrays, structs,
  FB/class instances, partial retention.
- `NON_RETAIN` and unqualified variables are not persisted.
- Warm restart restores values; `--reset-retain` and missing file use defaults.
- Layout change: `error` trap and `reset` behavior.
- Corrupt magic/version/length/CRC.
- Atomic save leaves the old file intact on write failure.
- `--no-retain` disables both directions.
- Reference/interface fields in retained variables are rejected.
- Direct `AT` RETAIN warning.
- Layout hash is stable across identical builds and changes when layout changes.
- Library host snapshot/restore via `stc_retain_*` without file helpers.
- Online attach preserves matching per-field roots, initializes new roots, quarantines removed
  roots, and does not load the state file.
- Cold download applies defaults and only compatible `RETAIN` values; transient allocations are
  not preserved.
- Incompatible retained-root layout rejects the module or resets the root only under an explicit
  policy; baseline problems select cold download.
- A `GotId`-keyed state format is not exercised in v1; the layout hash remains authoritative for
  restarts.

## 9. Open questions

- Whether to support periodic/checkpointed saves (v1 saves only at shutdown).
- Whether to add checksums stronger than CRC32 or optional signing.
- Whether `RETAIN` direct variables should persist the process image in a later version.
- Whether partial retention inside a retained FB/class instance should be exposed in metadata as
  leaf fields only (current design) or as whole-instance ranges for performance.
- Whether a later state-format revision should migrate retained leaves by `GotId` plus path
  across restarts. V1 does not; the manifest already records the identities needed to design it.
