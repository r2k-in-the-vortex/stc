# State and RETAIN Persistence

Status: Draft (Q-007 resolved: file-backed state)  
Profile: `stc-2013-oop-utf8-v1`  
Related: [Runtime](Runtime-and-Execution.md), [CLI/Manifest](CLI-and-Manifest.md), [Architecture](Architecture.md), [Decisions](../Decisions.md)

## 1. Scope

IEC 61131-3:2013 defines `RETAIN` and `NON_RETAIN` for `VAR`, `VAR_INPUT`, `VAR_OUTPUT`, and
`VAR_GLOBAL` sections, including FB/class/program instances. `stc` implements file-backed retained
state for executable mode and exposes the same primitives to library hosts.

A start with a valid state file is a **warm restart**; a start without one (or with
`--reset-retain`) is a **cold restart**. Warm restart restores retained values; cold restart
initializes declared/type defaults.

> **M4 status:** scalar-leaf `RetainLayout` is computed (retained globals and program fields,
> `AT` excluded), emitted as metadata, and exposed through generated
> `stc_retain_size/layout_hash/save/load`. `stc_init` calls `stc_rt_state_load` and restores on
> success; `stc_shutdown` saves through `stc_rt_state_save`. The runtime state functions are
> stubs returning 5 (retention disabled) until M6. Aggregate/string/FB leaves and the
> file-backed atomic format are M5/M6.

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
- `STRING`, `WSTRING`, `USTRING` (the fixed-capacity struct is copied);
- arrays and structs (recursively);
- FB/class instance data fields.

Not persistable:

- `REF_TO` values, interface references, `VAR_IN_OUT` parameters;
- vtable/type-descriptor pointers and other runtime pointers (these are reinitialized by
  `stc_init` before state is loaded; they are excluded from the payload);
- variables containing any non-persistable field (`STC-E-RETAIN-001`).

The compiler emits a **retained layout**: an ordered list of leaf fields with path, type, byte
offset, and size. The layout is the contract between generated code and the runtime.

## 3. State file format

Little-endian binary:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 8 | magic `STCRET01` |
| 8 | 4 | format version (`1`) |
| 12 | 4 | runtime ABI version (`1`) |
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
abi=1
size=<payload length>
leaf=<path>|<type>|<offset>|<size>
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
6. `stc_cycle` runs normally; retained values are ordinary storage.
7. `stc_shutdown` calls `stc_retain_save` to fill the buffer and `stc_rt_state_save` to write it
   atomically.

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

The module also emits an internal global buffer of `stc_retain_size()` bytes. `stc_init` and
`stc_shutdown` use the runtime helpers:

```c
int32_t stc_rt_state_load(void* buffer, int64_t len, int64_t layout_hash);
int32_t stc_rt_state_save(const void* buffer, int64_t len, int64_t layout_hash);
void    stc_rt_state_configure(const char* path, int32_t policy, int32_t enabled);
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

Generated `stc_init`:

1. initialize defaults and vtables;
2. if `stc_rt_state_load` returns OK, call `stc_retain_load`;
3. otherwise leave defaults (missing) or apply runtime policy (mismatch/corrupt).

Generated `stc_shutdown`:

1. call `stc_retain_save` into the buffer;
2. call `stc_rt_state_save`.

Library hosts may bypass the file helpers and use `stc_retain_size/save/load` with their own
storage. The host owns path/atomicity/policy in that case.

## 6. Metadata

`<name>.stc-meta.json` includes:

```json
{
  "retain": {
    "size": 128,
    "layout_hash": "0x9f2c...",
    "file": "motor.stcstate",
    "on_mismatch": "error",
    "leaves": [
      { "path": "Main.Counter", "type": "DINT", "offset": 0, "size": 4 },
      { "path": "Main.Timer.Elapsed", "type": "TIME", "offset": 8, "size": 8 }
    ]
  }
}
```

- `leaves` is sorted by path/offset and is the human-readable form of the layout hash input.
- The runtime uses `size`/`layout_hash` with the generated functions; it does not interpret leaf
  types.

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

## 9. Open questions

- Whether to support periodic/checkpointed saves (v1 saves only at shutdown).
- Whether to add checksums stronger than CRC32 or optional signing.
- Whether `RETAIN` direct variables should persist the process image in a later version.
- Whether partial retention inside a retained FB/class instance should be exposed in metadata as
  leaf fields only (current design) or as whole-instance ranges for performance.
