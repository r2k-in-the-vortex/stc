# I/O Routing and Backends

Status: WL-5.2 phase 0 contract lock (2026-10-31); v2.0.0 hardware-backend implementation not
started  
Profile: `stc-2013-oop-utf8-v1`  
Related: [I/O and Devices](IO-and-Devices.md), [Runtime and Execution](Runtime-and-Execution.md),
[CLI and Manifest](CLI-and-Manifest.md), [Online Change and GOT](Online-Change-and-GOT.md),
[Architecture](Architecture.md), [WL-5.2 plan](../../WL-5.2-Plan.md),
[v2.0.0 plan](../../V2.0.0-PLAN.md)

## 1. Scope and authority

This document fixes the v2.0.0 hardware-I/O contract: the machine-profile schema and selection,
the route graph, the multi-backend coordinator, the EtherCAT status device, transparent fault and
recovery behavior, online-change compatibility, and the evidence sequencing for the backend
campaign. WL-5.2 implements the EtherCAT backend; WL-5.1 adds a Modbus backend to the same
contract.

The accepted decisions and their rationale are in the [WL-5.2 plan](../../WL-5.2-Plan.md) §5
(E-1–E-16), §6.3–§6.3.1, and §6.4–§6.7. This document records the resulting contract; where the
plan gives only the shape, this document fixes the schema. If implementation finds a conflict,
the plan is amended first and this document follows.

The v1 backends (`null`, `memory`, `file`) and the v1 binding precedence remain as specified in
[I/O and Devices](IO-and-Devices.md) and [CLI and Manifest](CLI-and-Manifest.md). The contract
here is additive: a project without hardware devices behaves exactly as in v1.

Protocol details stay out of the language and compiler. The compiler validates the manifest and
the selected profile only; it does not interpret EtherCAT PDOs, sync managers, IgH calls, or
Modbus function codes.

## 2. Machine profiles and `[io].config`

### 2.1 Selection and embedding

- There is one source-controlled machine profile per machine, for example `io/bench.toml` or
  `io/line1.toml`. The profile contains the backend device configuration and PDO map for every
  data device on that machine, plus the `[[routes]]` table. Read-only status devices
  (`ethercat-status`) have no profile entry.
- The logical device tree and channel descriptors stay in `stc.toml`: device names, `kind`,
  `[[io.devices.channels]]` names, directions, and widths. The profile references devices by
  name; it does not redefine the logical tree.
- `[io].config` selects exactly one profile per build. The value is a path relative to the
  project root and must stay inside the project root. A missing file, a path escape, or a TOML
  parse error fails the build.
- The compiler canonicalizes the selected profile, resolves its route endpoints against the
  program and the device tree, and embeds the profile and the effective routes in
  `<name>.stc-meta.json`. The artifact is self-contained and reproducible; the runtime does not
  read the profile file at run time.
- The compiler emits the effective device tree, the PDO/channel map, and the resolved routes.
  Route validation covers endpoint resolution, fan-in, direction, and width (§3).
- `[io].config` and the route `kind` spellings are fixed by this contract. Metadata JSON key
  names and Rust type names are phase-1 implementation defaults; they do not change this
  contract.

Example selection in `stc.toml`:

```toml
[io]
# A single-backend project may name the hardware backend; a mixed project
# omits backend or leaves it "null" and lets each device's kind select one.
backend = "ethercat"
# Source-controlled machine profile, resolved and embedded at build time.
config = "io/bench.toml"

[[io.devices]]
name = "diewu"
kind = "ethercat"

[[io.devices.channels]]
name = "OUT0"
direction = "output"
width = 1
```

### 2.2 Profile schema

A profile is a TOML document with one `[device.<name>]` table per data device and one top-level
`[[routes]]` array. The profile references devices by their declared name; it does not repeat
the logical tree or the device `kind`. The runtime rejects a profile device name that is not
declared in `stc.toml`, and a declared data device of a hardware backend kind that has neither a
profile entry nor inline `config`. Status devices have no profile entry.

```toml
# io/bench.toml (selected by [io].config; embedded at build time)
[device.diewu]
master = 0
alias = 0
position = 0
vendor = "0x00000b95"
product = "0x00010200"
revision = "0x00000001"
startup-timeout = "5s"
recovery-cycles = 3

[[device.diewu.syncs]]
index = 0
direction = "output"
watchdog = "enable"

[[device.diewu.syncs.pdos]]
index = "0x1a00"

[[device.diewu.syncs.pdos.entries]]
index = "0x3101"
subindex = 1
bits = 8

[[device.diewu.pdo-channels]]
name = "OUT0"
entry = { index = "0x3101", subindex = 1 }
bit = 0

# OUT1..OUT7 use bits 1..7 of 0x3101:01; OUT8..OUT15 use 0x3101:02 bits 0..7.
# IN0..IN15 mirror this on 0x3001:01 (IN0..IN7) and 0x3001:02 (IN8..IN15).

# IN0 fans out to the PLC input and to physical OUT1. OUT0 is driven by
# Main.Out0 through the single-sink binding shorthand in stc.toml.
[[routes]]
source = { kind = "physical-input", device = "diewu", channel = "IN0" }
sinks = [
  { kind = "plc-input", variable = "Main.In0" },
  { kind = "physical-output", device = "diewu", channel = "OUT1" },
]

[[routes]]
source = { kind = "plc-output", variable = "TaskA.Out" }
sinks = [{ kind = "plc-input", variable = "TaskB.In" }]
```

Device table rules:

| Key | Type | Rule |
| --- | --- | --- |
| `master` | integer | Non-negative master index. |
| `alias` | integer | Non-negative slave alias. |
| `position` | integer | Non-negative ring position; distinct per device. |
| `vendor`, `product`, `revision` | integer or hex string | Slave identity; string spellings are accepted for convenience and normalized before comparison. |
| `startup-timeout` | duration string | Bound on reaching operational state and a healthy working counter. |
| `recovery-cycles` | integer | Consecutive healthy exchanges required before the stale/fault state clears. |
| `syncs` | array | Sync-manager, PDO, and entry tree, declared in order. |
| `pdo-channels` | array | Logical channel name to PDO entry and start bit. |
| `dc` | table | Optional distributed-clock configuration. |

`[[routes]]` is a top-level array in the same profile, not a key of `[device.<name>]`; §3
defines it.

- `syncs` mirrors the IgH slave PDO configuration: each `syncs` entry has `index`,
  `direction` (`input` or `output`), and `watchdog` (`enable` or `disable`); each `pdos` entry
  adds a PDO `index`; each `entries` entry adds an `index`, a `subindex`, and a `bits` width.
  Entry order inside a PDO defines the concatenated bit ranges.
- A `pdo-channels` entry resolves through an entry already declared in the same device's
  sync/PDO tree; it cannot name an undeclared entry. Its `bit` must lie inside the entry width.
  For a selected device, the runtime cross-checks that every declared `[[io.devices.channels]]`
  channel maps to exactly one `pdo-channels` entry and every `pdo-channels` entry maps back to a
  declared channel; duplicate or missing mappings are rejected at cold start.
- Device names and channel names are unique in `stc.toml`; that is the compiler's validation
  surface. Slave positions and entry/index/subindex combinations must be unique, and
  overlapping PDO entries or channel bit ranges are rejected at cold start.
- The optional `dc` table carries `assign-activate`, `sync0-cycle`, `sync0-shift`,
  `sync1-cycle`, and `sync1-shift` in the same units as the IgH distributed-clock API.
  Distributed clocks are configured only when `assign-activate` is present; without it the
  device runs free-run. A `dc` table that requests synchronization but omits `assign-activate`
  is `STC-W-IO-ETH-002`. The DIEWU profile omits `dc` by owner decision.
- The V2.0.0 profile model is static: an ESI parser, dynamic PDO discovery, and SDO-based device
  configuration are deferred (WL-7). The hand-maintained map is the contract.

### 2.3 Precedence

The effective device configuration is resolved field by field with this precedence, highest
first:

1. Runtime override (`--io-config <path>`).
2. The selected source profile (`[io].config`).
3. Inline `[io.devices].config` in `stc.toml`.

Inline device `config` remains valid as a default for projects without a profile and for fields
the profile omits. `--io-config <path>` is an addressing-only diagnostic override: it may
override master/slave addressing for the selected devices, but it cannot add or remove devices,
change channel bindings, or change routes in v2.0.0. Routes always come from the profile
embedded in the artifact. The runtime override is recorded in status output so a diagnostic run
is distinguishable from the committed configuration.

## 3. Route graph

### 3.1 Schema

Routes are a directed graph over the shared process image. The table lives in the selected
machine profile under `[[routes]]`, alongside the PDO map; it is not accepted in `stc.toml`. The
existing `[[io.bindings]]` in `stc.toml` remains the single-sink shorthand (§3.3).

A route has exactly one `source` object and a non-empty `sinks` array:

| Object | Kind | Required keys | Meaning |
| --- | --- | --- | --- |
| Source | `physical-input` | `device`, `channel` | A declared physical input channel. |
| Source | `plc-output` | `variable` | A fully qualified direct `%Q` variable. |
| Sink | `plc-input` | `variable` | A fully qualified direct `%I` variable. |
| Sink | `physical-output` | `device`, `channel` | A declared physical output channel. |

The four allowed source-to-sink combinations are:

| Source | Sink | Applied at |
| --- | --- | --- |
| `physical-input` | `plc-input` | Input boundary, after the backend latch. |
| `physical-input` | `physical-output` | Input boundary, after the backend latch (pass-through). |
| `plc-output` | `physical-output` | After the scan, before the backend publish. |
| `plc-output` | `plc-input` | Input boundary of the next scan, with one scan of latency. |

Any other source/sink pairing is rejected. A source object must use `device`/`channel` for a
physical kind and `variable` for a PLC kind; mixing the two spellings, omitting a required key,
or adding an unknown key is invalid.

### 3.2 Direction, width, and mapping rules

- Physical inputs are sources only; physical outputs are sinks only. `%I` variables are sinks
  only; `%Q` variables are sources only. `%M` markers are not route endpoints in v2.0.0.
- A `plc-input` sink must be a direct `%I` variable and a `plc-output` source must be a direct
  `%Q` variable. A non-direct variable, an address in another area, or a `%M` address is
  rejected with `STC-E-IO-BINDING-002`.
- Source and sink widths must be equal. A physical endpoint uses its declared channel `width`
  in bits; a PLC endpoint uses its variable type width. A width mismatch is
  `STC-E-IO-BINDING-002`.
- A physical endpoint must fit inside one PDO entry. Split or multi-entry mappings are out of
  scope for v2.0.0. An 8-, 16-, or 32-bit value must be byte-aligned in the entry and in the
  process image; a 1-bit `BOOL` may use any entry bit.
- Every endpoint must resolve: the device must be declared, the channel must be declared with
  the matching direction, and the variable must exist with a direct address. An unresolved
  endpoint is `STC-E-IO-BINDING-002`. A route that names a device whose `kind` has no registered
  backend is rejected at cold start with `STC-E-IO-BINDING-003`.
- The compiler validates the effective route graph against the program and the declared device
  tree. The runtime revalidates physical route source/sink representation and the bound image
  ranges at cold start and fails closed; it never substitutes an unrouted value for a physical
  endpoint.

### 3.3 Fan-out, fan-in, and bindings

- A source may fan out to many sinks, either as multiple entries in one `sinks` array or across
  routes. A physical input may drive both a PLC input and a physical output in the same scan.
- Each sink has exactly one source. Two sources claiming the same PLC input or physical output
  are fan-in and are rejected with `STC-E-IO-BINDING-001`. The compiler rejects it at build
  time; the runtime revalidates and rejects it at cold start.
- The compiler emits the effective graph, and the runtime also rejects duplicate sink claims
  across bindings and routes. Binding and route claims are one namespace: a project binding and
  a route that write the same physical output conflict.
- `[[io.bindings]]` is the single-sink shorthand: `variable` + `device`/`channel` + `direction`
  is equivalent to a route with one source and one sink. Its precedence over pragmas and raw
  addresses is unchanged ([I/O and Devices](IO-and-Devices.md) §4).
- In a project that selects a machine profile, an unrouted direct `%I` variable is a warning, not
  an error. It keeps its declared/default input value, and the compiler emits
  `STC-W-IO-ROUTE-001` naming the variable and its effective default. The runtime keeps the same
  default in the process image. A `%I` variable is routed when a binding supplies its
  `device`/`channel` shorthand or when it is the `plc-input` sink of a route.
- In the same project, an unrouted direct `%Q` variable is a warning: its value is discarded and
  the compiler emits `STC-W-IO-ROUTE-002` naming the variable. A `%Q` variable is routed when a
  binding supplies its `device`/`channel` shorthand or when it is the `plc-output` source of a
  route. These warnings do not apply to a project without a profile, which has no route graph and
  keeps v1 behavior.
- Fan-in remains `STC-E-IO-BINDING-001`; an unrouted variable is never accepted as a second
  source for an already claimed sink.
- Route cycles are allowed. They are resolved by the one-scan PLC→PLC buffer described in
  §3.4 and produce no warning; a cycle is expected behavior, not a configuration error.

### 3.4 Timing and cross-backend routes

- Physical-input routes are applied after every backend has latched and after the global input
  defaults have been applied to unbound areas. That is the same scan's input image, so a
  pass-through output is published in the same cycle.
- PLC-output→physical-output routes are applied after the scan and before the backend publish,
  so the physical output carries the scan's value in the same cycle.
- PLC→PLC routes are staged from their `%Q` sources after the scan and applied to their `%I`
  sinks at the next input boundary. This adds exactly one scan of latency and makes every route
  cycle well-defined.
- Cross-backend routes are allowed because every route operates on the shared process image. A
  Modbus input may drive an EtherCAT output, or the reverse, without either backend knowing
  about the other. The coordinator, not a backend, applies the route.
- A route carries the values that are in the image. If a backend is faulted, its bound inputs
  are stale and its bound outputs are unchanged (§6); the route does not silently substitute,
  suppress, or rewrite data.

## 4. Multi-backend coordinator

### 4.1 Device dispatch and registry

Hardware backends are selected per device `kind`, not by one global selector:

- Each declared device's `kind` resolves to a registered backend instance at cold start.
  Devices of the same kind share one backend instance; the instance owns only those devices and
  their bindings.
- `[io].backend` and `--io` remain the default for unbound image areas and for projects without
  hardware devices. They do not constrain the number of device backends. A single-backend
  project is a project whose devices share one kind, not a separate runtime path.
- WL-5.2 registers the kinds `ethercat` and `ethercat-status`; both are served by the same
  EtherCAT backend instance, so the status device reads the same master and slave state as the
  data devices. WL-5.1 registers `modbus` as another backend instance.
- A device whose `kind` has no registered backend is a cold-start error; the run never begins.
- A declared device of a registered kind is configured by default even when it has no bindings,
  so its health is visible; its unbound output entries are zeroed at configuration time. A
  backend may define an explicit opt-out for unused devices.
- When `[io].backend` names a hardware backend, the coordinator uses the `null` behavior for
  unbound image areas; the hardware instances handle their bound ranges.

### 4.2 Backend interface

The coordinator-facing interface is a backend-neutral Rust trait in `stc-runtime`. Exact type
names are a phase-1 default; this shape is the contract WL-5.1 builds against:

```rust
// Shape only; no protocol type crosses this interface.
trait DeviceBackend {
    fn cold_start(&mut self, config: &BackendConfig) -> Result<(), Error>;
    fn latch_inputs(&mut self, image: &mut ProcessImage) -> Result<(), Error>;
    fn publish_outputs(&mut self, image: &ProcessImage) -> Result<(), Error>;
    fn status(&self) -> BackendStatus;
    fn shutdown(&mut self) -> Result<(), Error>;
}
```

- No EtherCAT, Modbus, or other protocol type crosses the interface. The coordinator does not
  know what a PDO, a sync manager, or a Modbus register is.
- `latch_inputs` writes only the instance's bound `%I` ranges and its status-device block.
  `publish_outputs` reads only the instance's bound `%Q` ranges. The coordinator applies routes,
  never a backend.
- `status` returns a per-backend summary for `--status` and the control/status surface: backend
  name, device count, state, exchange and fault counters, last fault, and the configuration
  hash. A second backend adds its own entry without changing the EtherCAT fields.
- A runtime failure reports a fault status and does not abort the run. Backends are not
  safety-rated and do not implement a fail-safe output state.

### 4.3 Cycle-boundary order

The coordinator owns one shared process image and applies this order deterministically:

1. **Cold start.** Parse metadata; resolve each device `kind` to a backend instance; group
   bindings, physical sinks, and routes by device; call `cold_start` for every instance in
   device declaration order. The run does not begin until every instance reports ready or its
   startup timeout fails the run.
2. **Input boundary.** Apply the global input defaults to unbound areas; call each instance's
   `latch_inputs` in device declaration order; apply physical-input→`plc-input` and
   physical-input→physical-output routes; apply the buffered PLC→PLC sources to their `%I`
   sinks.
3. **After the scan.** Apply `plc-output`→physical-output routes; stage the PLC→PLC route
   sources; call each instance's `publish_outputs` in the same declaration order; apply the
   global output behavior to unbound areas.
4. **Shutdown.** Call `shutdown` for every instance in reverse declaration order.

The order is device declaration order for start, latch, and publish, and its reverse for
shutdown. It is deterministic and is covered by coordinator tests.

### 4.4 Conflicts and isolation

- Bound image ranges are disjoint across devices and backends. Overlapping claims, fan-in, and
  a route or binding that names a sink already claimed by another source are rejected at cold
  start; a conflicting configuration fails closed.
- Fault state is per backend. One backend's fault does not alter another backend's data, and
  the aggregate status reports every backend.
- A partial cold start is never left running: if one backend fails to start, the coordinator
  shuts down the instances already started in reverse order and fails the run. The failure is
  fatal with a clear diagnostic and no PLC outputs are published.
- Output publication runs on every cycle attempt, including a rolled-back or faulted cycle, so
  a fieldbus watchdog is not tripped by a missing frame. The values sent are the PLC's values,
  unchanged.

## 5. EtherCAT status device

### 5.1 Channel layout

A project may declare a read-only status device in `stc.toml` and bind `AT %I` variables to its
channels. The device has no PDO map and no profile addressing:

```toml
[[io.devices]]
name = "ecat-status"
kind = "ethercat-status"
config = { layout = 1 }

[[io.devices.channels]]
name = "input.stale"
direction = "input"
width = 1
```

The runtime implements the full channel set below and fills the bound block at the input
boundary before the module runs, so the PLC sees the current state in the same scan. The project
binds only the fields it needs; unbound fields cost no image bytes. `<N>` is the zero-based index
of the configured EtherCAT data device in device declaration order; status devices are not
counted. For example, `slave0.al_state` is the first configured data device and
`slave1.online` the second.

| Channel | Type | Meaning |
| --- | --- | --- |
| `master.phase` | `UINT` | IgH master phase. |
| `master.active` | `BOOL` | Master has an active application. |
| `master.slaves_responding` | `UINT` | Responding slave count. |
| `master.link_up` | `BOOL` | EtherCAT link state. |
| `master.lost_frames` | `UDINT` | IgH lost-frame counter. |
| `master.tx_errors` | `UDINT` | IgH Tx-error counter. |
| `master.frame_loss_permille` | `UINT` | Frame-loss rate in tenths of a percent. |
| `master.last_error` | `UDINT` | Last backend error code. |
| `master.recoveries` | `UDINT` | Successful recovery count. |
| `domain.wc_state` | `UINT` | Working-counter state. |
| `domain.working_counter` | `UINT` | Last working counter. |
| `slave<N>.al_state` | `UINT` | Per-slave application-layer state. |
| `slave<N>.online` | `BOOL` | Per-slave online flag. |
| `slave<N>.operational` | `BOOL` | Per-slave operational flag. |
| `input.stale` | `BOOL` | Inputs are the last received values, not current. |
| `cycle.exchanges` | `UDINT` | Successful exchanges since start. |
| `cycle.faults` | `UDINT` | Fault episodes since start. |

### 5.2 Binding and versioning

- All status channels are read-only from the PLC's perspective. Only `AT %I` variables may bind
  to them; a `%Q` binding or a write is rejected. A status device is not a physical route
  endpoint; routes name channels of data devices, and the PLC reaches status values through its
  `%I` bindings.
- `config.layout` selects the layout version. Layout 1 is the table above. An unknown layout,
  an unknown channel name, or a channel width mismatch is rejected at build or cold start;
  unknown channels are never silently ignored.
- The backend fills the block from master, domain, and slave state each cycle. A project that
  binds no status channels incurs no image cost.
- `--status` and the control/status surface print the same information for operators. The
  status block and the operator output are diagnostic, not safety-rated, and do not implement a
  fail-safe output state.

## 6. Transparent fault and recovery behavior

The master never rewrites the data it exchanges. Fieldbus failure changes status and counters,
not process-image values.

| State | Entry | Behavior |
| --- | --- | --- |
| `Opening` | Cold start | Request master, configure slaves, activate. Failures are fatal startup errors. |
| `WaitingForOp` | After activation | Exchange frames until all configured slaves are operational and the domain is healthy, or `startup-timeout` expires. |
| `Running` | Healthy exchange | Latch inputs, publish outputs, count exchanges. |
| `Faulted` | WC/AL/link fault | Keep sending the PLC output values unchanged; keep the last received input image and set the stale/fault status registers; keep exchanging and retrying; record the transition. |
| `Recovering` | Healthy exchange observed | Require `recovery-cycles` consecutive healthy exchanges before returning to `Running`; clear the stale flag and increment the recovery counter. |
| `Closed` | Stop, trap, or setup failure | Stop exchanging and release the master; do not rewrite outputs. |

- On a fault the runtime does not zero outputs, default inputs, or exit. The PLC output values
  continue to be sent unchanged, the last received inputs are presented as stale, and
  `input.stale`, the domain working counter, the master counters, and the per-slave state are
  visible every cycle.
- Recovery is automatic when communication returns, after the configured number of consecutive
  healthy exchanges. Connect and disconnect of a configured slave are normal and use this path.
- Adding or removing a device is a topology change; the runtime does not reconfigure the master
  while running, and a cold download is required.
- A runtime fault is non-fatal and retries indefinitely. A startup failure is fatal and the run
  never begins. A fieldbus fault is reported as `STC-W-IO-ETH-003`; the transparent policy
  replaces the removed persistent-fault error.
- Fault state is per backend. This section fixes the EtherCAT policy; WL-5.1 defines the Modbus
  policy. The coordinator keeps one backend's fault from masking or silently changing another
  backend's data.
- This behavior is not a safety function. The PLC logic must use `input.stale` and the status
  registers; safety must come from a separate system and machine design.

## 7. Online change

The coordinator computes a configuration hash over:

- the canonical selected profile contents;
- the device tree (device names, kinds, and channel descriptors);
- the backend kinds and the PDO/channel map;
- the effective bindings and routes, including each bound variable's image area, offset, and
  width.

The hash is carried in the backend status and compared when a candidate generation is prepared.

- A code-only swap or a same-configuration swap whose hash is unchanged keeps the backends
  active, their mappings live, and the last published outputs live. The status device and
  counters continue.
- A candidate whose hash differs is rejected with `STC-E-ONLINE-IO-001`, and the active
  generation continues. The deployment path must use a cold download.
- Adding or removing a device, changing a `kind`, changing the PDO/channel map, changing a
  route, or changing a bound variable's area, offset, or width changes the hash and therefore
  requires a cold download. A configured-slave reconnect is not a configuration change.
- Compatible image remapping of unbound areas follows the existing host rules
  ([Online Change and GOT](Online-Change-and-GOT.md)); a change that affects a bound backend
  range is a configuration change, not a remap.
- The policy is backend-neutral: a mixed project includes every backend's configuration in the
  hash.

## 8. Evidence sequencing

The backend campaign is split so that frame-level evidence is gathered without physical wiring:

- **Phases 3–5** use open-ended (unwired/unloaded) I/O. A cycle succeeds at the frame level:
  the send is accepted, the receive/process path sees the reply, the domain working-counter
  state is healthy, all configured slaves are operational, there are no send/receive errors,
  and the master's lost-frame and Tx-error counters do not increase. For timing runs, skipped
  periods and missed deadlines must also be zero. Physical I/O values are not verified.
- **Phase 6** builds the loopback harness (all 16 outputs to all 16 inputs with the correct NPN
  common), runs the automated all-points matrix, performs the physical fault injection (cable
  unplug/replug, slave power cycle, configured-slave reconnect), and completes all manual and
  physical tests.
- The hardware-in-the-loop evidence is single-slave. Multi-slave configurations are covered by
  configuration, mapping, and mock tests; the boundary is stated in the report.
- Open-ended I/O proves the fieldbus and runtime path, not the field wiring or signal integrity.
  Frame-level success is not physical I/O proof.
- There is no CI timing gate. Timing results are host- and hardware-specific, and no hard
  real-time or safety claim is made.

## 9. Compiler and runtime boundary

**Compiler:**

- parses `stc.toml`, resolves `[io].config` against the project root, requires the file to exist
  and parse as TOML, and embeds its canonical contents;
- validates device and channel uniqueness, route endpoint resolution against the program and
  device tree, the fan-in rule, direction, and width;
- emits the effective device tree, PDO/channel map, and routes in metadata;
- does not interpret PDO indices, sync managers, IgH, or Modbus, and adds no protocol-specific
  field to the language or to generated code.

**Runtime:**

- interprets the embedded profile, PDO map, and routes;
- dispatches devices by `kind`, applies routes at the cycle boundary, and validates sink
  conflicts and binding resolution at cold start;
- delegates all bus and protocol behavior to a backend.

The profile and routes are source-controlled inputs to the build, not runtime files. The
metadata is self-contained, so the artifact can be deployed to the target host without copying
the source profile.

## 10. Diagnostics

The contract uses these stable codes. The phase-2 implementation fixes the full EtherCAT set and
its wording; diagnostics that are defined but not yet emitted are recorded in the traceability
allowlist.

| Code | Condition |
| --- | --- |
| `STC-E-IO-BINDING-001` | A sink has more than one source (fan-in), or two routes/bindings claim the same output. |
| `STC-E-IO-BINDING-002` | A route endpoint does not resolve, or its direction or width is invalid. |
| `STC-E-IO-BINDING-003` | A route or device names a `kind` with no registered backend. |
| `STC-W-IO-ROUTE-001` | An unrouted direct `%I` variable keeps its declared/default value. |
| `STC-W-IO-ROUTE-002` | An unrouted direct `%Q` variable is discarded. |
| `STC-E-IO-ETH-001` | The shim/library is missing, cannot be loaded, or fails the version/symbol handshake. |
| `STC-E-IO-ETH-002` | The IgH master cannot be requested (busy, unavailable, or permission denied). |
| `STC-E-IO-ETH-003` | Slave configuration failed (identity mismatch or invalid position/alias). |
| `STC-E-IO-ETH-004` | PDO/sync configuration or entry registration failed. |
| `STC-E-IO-ETH-005` | Master activation, domain data, or release failed. |
| `STC-E-IO-ETH-006` | The EtherCAT device or channel configuration is invalid. |
| `STC-E-IO-ETH-007` | A bound channel does not resolve, or its direction or width or image range mismatches. |
| `STC-E-IO-ETH-008` | Startup did not reach OP within the startup timeout. |
| `STC-W-IO-ETH-001` | A declared EtherCAT channel is unbound. |
| `STC-W-IO-ETH-002` | Distributed clocks were requested but no `assign-activate` is configured. |
| `STC-W-IO-ETH-003` | Fieldbus fault: outputs unchanged, inputs stale, status registers updated; recovery continues. |
| `STC-E-ONLINE-IO-001` | The I/O configuration changed and a cold download is required. |

## 11. Open questions

No design questions remain as of 2026-10-31; E-1–E-16 in the
[WL-5.2 plan](../../WL-5.2-Plan.md) are accepted. The exact Rust type names and metadata JSON
key spellings are implementation defaults fixed in phase 1; they do not change the contract in
this document.
