# I/O and Device Linking

Status: Draft  
Profile: `stc-2013-oop-utf8-v1`  
Related: [Architecture](Architecture.md), [Runtime](Runtime-and-Execution.md), [CLI/Manifest](CLI-and-Manifest.md), [Tokenization](../Syntax/Tokenization.md)

## 1. Responsibility split

**Compiler (`stc`)**:

- parses and type-checks `AT %I`/`%Q`/`%M` declarations against the process image,
- assigns deterministic process-image offsets,
- parses `{stc:io ...}` JSON hints on `AT` declarations,
- copies project device declarations and bindings into `<name>.stc-meta.json`,
- validates the shape of the metadata; it does **not** open devices or schedule I/O.

**Runtime**:

- owns I/O backends and device scheduling (v2),
- reads the compiler metadata plus its own runtime configuration,
- binds process-image ranges to device channels,
- may override a pragma hint or a project-level binding,
- latches inputs at cycle start and writes outputs at cycle end.

**v1 scope**: no hardware backends. The executable/library runtime uses `null` (unbound inputs
receive type defaults, unbound outputs are discarded), `memory` (tests), or optionally `file`.
EtherCAT, Modbus, and MQTT are planned for v2 and are noted here but not designed in v1.

## 2. Process image

Direct variables map into a contiguous process image:

```text
+-------------------+-------------------+-------------------+
| %I input image    | %Q output image   | %M marker image   |
+-------------------+-------------------+-------------------+
```

- `%I` is input; `%Q` is output; `%M` is marker (read/write, retained across cycles).
- The compiler computes byte sizes from declarations and alignment rules; the runtime allocates
  the buffers.
- Inputs are latched once at cycle start; outputs are written once at cycle end; `%M` persists.
- `%I` variables are read-only in ST; writing one is `STC-E-IO-002`. `%Q` and `%M` are
  read/write.
- Direct variables in functions/FB bodies are deprecated by 2013; `stc` accepts them with
  `STC-W-RUN-001` and maps them to the same process image.

### 2.1 Address forms

| Form | Meaning |
| --- | --- |
| `%IX5` | input bit 5 |
| `%IX0.0` | input byte 0, bit 0 |
| `%IB0` | input byte 0 |
| `%IW0` | input word 0 (2 bytes) |
| `%ID0` | input dword 0 (4 bytes) |
| `%IL0` | input lword 0 (8 bytes) |
| `%QB0`, `%QW0`, `%QD0`, `%QL0` | output byte/word/dword/lword |
| `%MB0`, `%MW0`, `%MD0`, `%ML0` | marker byte/word/dword/lword |
| `%I*`, `%Q*`, `%M*` | wildcard in declarations/`VAR_ACCESS`; not executable |

- A missing size with a two-part address is a bit (`X`); with a one-part address it is a byte
  (`B`). Example: `%I0.0` is a bit; `%I0` is a byte.
- The declared type must fit the addressed size. `BOOL` uses `X`; `BYTE`/`SINT`/`USINT` use `B`;
  `WORD`/`INT`/`UINT` use `W`; `DWORD`/`DINT`/`UDINT`/`REAL` use `D`; `LWORD`/`LINT`/`ULINT`/
  `LREAL`/`TIME`/`LTIME`/`DATE`/`TOD`/`DT` use `L`.
- Overlapping addresses are allowed for aliasing/`OVERLAP`; the analyzer checks that the declared
  types fit and reports conflicting writes where it can.
- `AT` variables are not ordinary storage; they have no separate allocation.
- `RETAIN` on a direct `AT` variable is accepted but has no file-backed effect
  (`STC-W-RETAIN-001`): the runtime/backend owns the process-image value. See
  [State and RETAIN](State-and-Retain.md).

### 2.2 Unbound variables

A direct variable is **bound** if a runtime override, a `[[io.bindings]]` entry, or an `{stc:io}`
pragma applies. Otherwise:

- **Inputs (`%I`)**: at the start of each cycle the runtime writes the type's default value into
  the input location. Defaults follow the 2013 default initial values: `BOOL` FALSE, numeric types
  0, `REAL`/`LREAL` 0.0, strings empty, arrays/structs recursively defaulted, enums/derived types
  use their declared default. If the declaration has an explicit initializer, that initializer is
  used instead of the type default. The compiler emits an `input_defaults_hex` blob for defaults
  that are not all-zero; the runtime copies it before each cycle. No device read occurs for an
  unbound input.
- **Outputs (`%Q`)**: writes update the output image during the cycle, but there is no external
  sink. The image is discarded at the end of the cycle. Reads later in the same cycle see what the
  program wrote. Externally, an unbound output "does nothing".
- **Markers (`%M`)**: always ordinary process-image memory, read/write, retained across cycles;
  no binding is required.
- If `stc.toml` has no `[io]` section, no devices, and no bindings, every input is unbound and
  every output is discarded. This is the default `null` runtime behavior.
- A bound input is overwritten by its backend after the defaults are applied. If a backend does
  not provide a value (missing/failed device), the runtime may leave the default; this fallback
  policy is runtime-defined.

## 3. Project device model

`stc.toml` carries the device tree. The compiler treats device `config` as opaque TOML and copies
it into metadata; the runtime interprets it.

```toml
[io]
backend = "null"          # v1: null | memory | file; v2 adds hardware backends
input-bytes = 16          # optional; compiler computes if omitted
output-bytes = 16
marker-bytes = 64

[[io.devices]]
name = "drive1"
kind = "ethercat"         # opaque identifier; runtime-defined
config = { vendor = "0x1234", revision = 3 }  # opaque to the compiler

  [[io.devices.channels]]
  name = "DI5"
  direction = "input"     # input | output | inout
  width = 1               # bits

[[io.bindings]]
variable = "Main.MyBool"  # fully qualified ST variable
device = "drive1"
channel = 5
direction = "input"
```

- Device names are unique. `kind` is a string; the compiler does not interpret it.
- `config` may contain any TOML value. It is serialized to JSON metadata deterministically.
- `channels` is optional. If present, the compiler can validate a pragma/project binding's
  `channel` and `direction`; otherwise the runtime validates.
- `[[io.bindings]]` is the project-level binding. It is optional; the `AT` pragma is the local
  hint.
- The compiler does not require a binding for every direct variable. Unbound variables remain
  plain process-image locations.

## 4. `AT` pragma hints

An `AT` declaration may carry a single `{stc:io ...}` pragma with a JSON object:

```st
VAR_GLOBAL
  MyBool AT %IX5 : BOOL {stc:io {"device": "drive1", "channel": 5, "direction": "input"}};
  MyWord AT %QW2 : WORD {stc:io {"device": "drive1", "channel": 12, "format": "u16"}};
END_VAR
```

Rules:

- The payload must be a JSON object. It may contain arbitrary keys for future I/O types.
- Recognized hint keys for validation: `device` (string), `channel` (non-negative integer),
  `direction` (`input`/`output`/`inout`). Unknown keys are preserved.
- If `device` is present and no matching `[[io.devices]]` entry exists, emit `STC-W-IO-001`
  (warning, not error; the runtime may add devices).
- `stc:io` on a declaration without `AT` is `STC-W-IO-002` and has no effect.
- Duplicate `stc:io` pragmas on one declaration are `STC-E-IO-003`.
- The pragma is a **hint**. The runtime may ignore or override it.
- The compiler copies the JSON into metadata; it does not link hardware.

Binding precedence (highest to lowest):

1. Runtime override (runtime configuration/CLI).
2. `[[io.bindings]]` in `stc.toml`.
3. `{stc:io ...}` pragma on the `AT` declaration.
4. Default: the raw `%I`/`%Q`/`%M` process-image address.

A runtime override may bind a variable that has no pragma, or rebind one that does.

## 5. Metadata contract

`<name>.stc-meta.json` includes:

```json
{
  "io": {
    "backend": "null",
    "image": { "inputs_bytes": 16, "outputs_bytes": 16, "markers_bytes": 64 },
    "devices": [
      { "name": "drive1", "kind": "ethercat",
        "config": { "revision": 3, "vendor": "0x1234" },
        "channels": [ { "name": "DI5", "direction": "input", "width": 1 } ] }
    ],
    "input_defaults_hex": "00000000...",
    "output_defaults_hex": "00000000...",
    "bindings": [
      { "variable": "Main.MyBool", "area": "I", "address": "IX5",
        "offset_bits": 5, "type": "BOOL",
        "hint": { "device": "drive1", "channel": 5, "direction": "input" },
        "source": "pragma" }
    ]
  }
}
```

- JSON objects in metadata are canonicalized (sorted keys) for deterministic builds.
- `source` is `pragma` or `project`; runtime bindings are not in compiler metadata.
- `offset_bits` is relative to the start of the area.
- `input_defaults_hex` is a hex string with the same byte length as the input image; it contains
  the type-default values for every input location. The runtime applies it to unbound inputs and
  may use it as a fallback for bound inputs whose backend supplies no value.
  `output_defaults_hex` is analogous and is used only if a runtime chooses to preload outputs;
  v1 discards unbound outputs.
- The runtime uses this metadata plus its own configuration; it may ignore `hint`/`source`.

## 6. Runtime behavior

### 6.1 v1 backends

- `null`: at the start of each cycle the input image is filled from `input_defaults_hex` (or
  zeroed if absent); no device is read. The output image is discarded after the cycle. This is
  the "no linking defined" behavior: inputs see type defaults, outputs do nothing externally.
- `memory`: explicit input/output byte arrays for tests and host embedding. Unbound input
  locations still receive defaults unless the test writes them.
- `file`: input image read from a file at startup, output image written at shutdown or each cycle
  (implementation-defined; tests use memory).
- The runtime API exposes the process image (`stc_rt_io_image`) and an optional backend vtable.

### 6.2 v2 device backends (not designed yet)

Planned primary backends:

- **EtherCAT** — cyclic process data; device tree maps to channels.
- **Modbus** — register/coil mapping; read/write scheduling.
- **MQTT** — topic/payload mapping; asynchronous I/O and caching.

These require runtime-side scheduling, discovery, and configuration. The compiler metadata and
pragma JSON are designed to carry opaque, future-proof binding information; no protocol-specific
fields are added to the language or compiler.

### 6.3 Cycle behavior

- At cycle start, unbound input locations are reset to their type defaults, then bound inputs are
  read from their backends. Inputs are latched once before `stc_cycle` runs program code.
- Outputs are published once at cycle end (after `stc_cycle`). Unbound outputs have no sink and
  are discarded.
- The cycle time is latched at the same boundary (see Runtime §2.5); I/O latches and timer time
  share the cycle boundary.
- A runtime backend may fail to read/write; it reports a trap/status to the host. Exact error
  codes are runtime-defined in v2.

## 7. Diagnostics

| Code | Condition |
| --- | --- |
| `STC-E-IO-001` | `AT` address/size does not match the declared type |
| `STC-E-IO-002` | write to a read-only `%I` variable |
| `STC-E-IO-003` | duplicate `stc:io` pragma on one declaration |
| `STC-E-IO-004` | `stc:io` payload is not a JSON object or is malformed |
| `STC-W-IO-001` | pragma names a device not declared in `stc.toml` |
| `STC-W-IO-002` | `stc:io` on a declaration without `AT` |
| `STC-W-IO-003` | project binding overrides a pragma hint (informational) |

## 8. Open questions

- The exact runtime override format (CLI flags vs. a runtime config file) is v2; v1 exposes only
  the backend selection and test hooks.
- Whether `[[io.bindings]]` should also support `fb_instance.member` paths and struct fields;
  v1 accepts fully qualified variable paths and the analyzer resolves them.
- Whether device channel descriptors should become normative for every backend; v1 keeps them
  optional hints.
