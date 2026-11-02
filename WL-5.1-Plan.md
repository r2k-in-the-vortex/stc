# WL-5.1 Plan — Modbus TCP server I/O

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-11-01; revised 2026-11-02 (owner decisions accepted; phase 0 contracts gate closed)  
Branch: `WL-5.1` (sub-plan of `v2.0.0-planning`)  
Status: Phases 0–3 complete 2026-11-02; phase 4 in progress. WL-5.2 is complete and its
multi-backend I/O system is the baseline. The owner accepted M-1–M-15 and resolved Q-1–Q-22 on
2026-11-02; the decisions in §5 are the locked contract. Phases are handed one at a time to
subagents; each phase ends with a commit, push, and green CI, and this agent updates §7.1 before
the next handover (§7.4).  
Parent: [V2.0.0-PLAN.md](V2.0.0-PLAN.md) §4 (WL-5.1)  
Baseline: `WL-5.1` at `a25bf69` (merge of WL-5.2 into `v2.0.0-planning`); the WL-5.2 working plan
is retired and its decisions are D-211–D-228.  
Inputs: [FutureWishlist.md](FutureWishlist.md) WL-5.1,
[I/O Routing and Backends](specification/Architecture/IO-Routing-and-Backends.md),
[I/O and Devices](specification/Architecture/IO-and-Devices.md),
[CLI and Manifest](specification/Architecture/CLI-and-Manifest.md),
[Runtime and Execution](specification/Architecture/Runtime-and-Execution.md),
[Decisions](specification/Decisions.md) §2 (D-211–D-228),
[EtherCAT-Host.md](specification/Verification/EtherCAT-Host.md),
[KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md)

## 1. Objective

Implement a Modbus TCP **server** backend in `stc-runtime` and incorporate it into the
multi-backend I/O coordinator delivered by WL-5.2. The backend exposes the bound process-image
ranges as coils, discrete inputs, holding registers, and input registers, with 1-, 8-, 16-, 32-,
and 64-bit channel widths; services client requests on dedicated network threads; commits client
writes at the input boundary and PLC outputs at the publish boundary; and reports health and
counters through the existing backend status surface and an optional PLC-visible status device.
A server device may listen on one or more explicit socket addresses, all sharing the same register
bank and mapping.

The integration is additive. The coordinator, the `DeviceBackend` trait, the backend registry, the
route graph, the configuration hash, and the EtherCAT backend are unchanged. Modbus is selected by
a device's `kind` exactly like EtherCAT, so a project can run Modbus and EtherCAT devices together
over one shared process image. No ST language, codegen, route, or host-ABI change is planned; the
only compiler-side change is structural validation of the new profile keys.

The backend is non-real-time and best-effort. It is not a safety function, makes no hard real-time
claim, and has no authentication or encryption. Evidence is cross-host functional and error-path
testing, a simulator/client test harness, cycle-boundary overhead measurements, and specification,
known-limitations, and operating-note updates.

## 2. Locked scope and boundaries

In scope:

- A pure-Rust Modbus TCP server backend in `crates/stc-runtime/src/io_modbus_tcp.rs`, registered as the
  `modbus-tcp` kind and the optional `modbus-tcp-status` status kind. It uses `std::net` and `std::thread`
  only; no new crate, dependency, optional artifact, or host prerequisite.
- Server-side mapping from declared device channels to the four Modbus data tables through the
  selected machine profile. Existing `[[io.devices.channels]]`, `[[io.bindings]]`, and routes
  remain the process-image binding mechanism. Channel widths are 1 bit on the bit tables and 8,
  16, 32, or 64 bits on the register tables.
- One or more explicit socket binds per server device, written as a non-empty TOML array of
  address strings in the profile; all binds share the server's register bank, mapping, counters,
  and connection limit.
- Function codes 1, 2, 3, 4, 5, 6, 15, and 16; standard Modbus TCP framing and exception
  responses; connection limits, idle timeouts, byte/word order, and diagnostics.
- Dedicated listener/connection threads and a mutex-protected register bank. The cycle thread does
  only short bank-to-image copies at the input and publish boundaries; it never performs socket
  I/O.
- An optional PLC-visible `modbus-tcp-status` device with a versioned layout, and per-server
  diagnostics in the existing verbose host status output (`stc run -v`).
- Unit, integration, error-path, concurrency, cross-host, and end-to-end tests on Windows x86-64
  MSVC, Linux x86-64 GNU, and Linux AArch64 GNU; an in-repo test client/simulator; cycle-boundary
  overhead measurements.
- Specification, CLI/manifest, I/O, known-limitations, README/user-guide, decisions, and evidence
  updates.

Out of scope:

- Modbus TCP client/master capability. That is the deferred WL-11 item and is not pulled in by
  this plan.
- Modbus RTU, serial transport, TLS, authentication, user accounts, and gateway/routing to other
  Modbus units.
- Floating point, strings, and arbitrary byte-swapped encodings. Supported widths are 1-bit
  `BOOL` on the bit tables and 8-, 16-, 32-, and 64-bit values on the register tables; 32- and
  64-bit values span 2 and 4 consecutive registers. A `BOOL` in a register is represented by a
  16-bit channel.
- Automatic derivation of Modbus addresses from existing bindings or image offsets. The mapping is
  explicit and source-controlled.
- `%M` route endpoints and general ST dataflow. The v2.0.0 route graph is unchanged.
- Hard real-time guarantees, timing certification, safety functions, and safety-rated output
  states. Modbus is non-real-time and must not be used as an interpolated motion channel.
- Changing the coordinator cycle order, the `DeviceBackend` trait, the configuration hash, or the
  EtherCAT backend and shim.
- A new global `[io].backend` spelling, `--io` value, `[test] io` value, host-ABI enum value, or
  C-ABI change for Modbus. Device `kind` dispatch is the contract (D-213); `[io].backend` remains
  the unbound-area default.
- A mixed real EtherCAT + Modbus hardware-in-the-loop campaign as a release gate. A mixed
  coordinator test with the EtherCAT mock is required; a real mixed bench run with both backends
  active is planned phase-4 evidence and does not connect Modbus signals to EtherCAT signals.
- Building, installing, or modifying a Modbus simulator product. The in-repo test client is the
  simulator; an independent third-party client check is optional evidence.

## 3. Verified baseline (2026-11-01)

### 3.1 The multi-backend I/O system delivered by WL-5.2

WL-5.2 is complete. The relevant implementation is:

- `crates/stc-runtime/src/coordinator.rs`: one `ProcessImage`; `BackendRegistry` maps device
  `kind` to a named backend instance; `Coordinator::cold_start` groups devices by instance,
  validates bindings, routes, fan-in, and image ranges, and calls each instance's
  `DeviceBackend::cold_start` in device declaration order. `input_boundary` and `after_scan`
  implement the deterministic latch/route/publish order. `reports` aggregates per-backend status
  and diagnostics, and `configuration_hash` covers the profile, device tree, kinds, configs,
  channels, bindings, and routes.
- `crates/stc-runtime/src/backend_defaults.rs`: the default registry registers `ethercat` and
  `ethercat-status` as one instance. It registers no other kind.
- `crates/stc-runtime/src/io_mock.rs`: a protocol-neutral mock backend used by coordinator tests.
  The mock can be registered under a second kind to prove the plural interface.
- `crates/stc-runtime/src/io_ethercat.rs`: the Linux IgH backend, including the status-device
  pattern (`ethercat-status`, layout 1) that the Modbus status device should mirror.
- `crates/stc-runtime/src/host.rs`: a generation creates the coordinator when any declared device
  `kind` is registered. A project with no registered hardware kinds keeps the pre-coordinator
  `null`/`memory`/`file` behavior.
- `crates/stc-runtime/src/lib.rs`: `print_host_status` prints one `io-backend` line per instance,
  including `name`, device count, state, exchanges, faults, hash, last fault, and every key/value
  in `diagnostics()`.

No protocol type crosses `DeviceBackend`, and the coordinator applies routes, never a backend.
That is the seam WL-5.1 builds against.

### 3.2 Configuration and metadata

- One source-controlled machine profile is selected by `[io].config`. The compiler reads it,
  checks that it stays inside the project root, parses it with
  `stc_manifest::parse_profile`, embeds the raw source text and its digest in metadata, and merges
  each profile device table field by field over inline `[io.devices].config`
  (`crates/stc-driver/src/build.rs`).
- `crates/stc-manifest/src/profile.rs` currently validates the EtherCAT profile shape:
  `master`/`alias`/`position`/identity/`startup-timeout`/`recovery-cycles`/`syncs`/
  `pdo-channels`/`dc` and the route endpoints. Its `DEVICE_KEYS` list rejects every other device
  key, so Modbus keys need an explicit schema extension.
- Metadata carries each device's merged configuration as opaque JSON text
  (`crates/stc-runtime/src/module_info.rs`). The runtime backend interprets it; codegen does not.
- `[io].backend` and `--io` select the global default for unbound image areas and for projects
  without hardware devices. They do not constrain device dispatch (D-213). `IoBackend::Ethercat`
  is the only hardware spelling and has the same `null` behavior as `IoBackend::Null`.
- The documented `--io-config` diagnostic override and `--io-ethercat-lib` flag are not exposed by
  the shipped CLI: only `--io` and `--io-file` are parsed, and the shim path is environment-only
  (`STC_ETHERCAT_LIBRARY`). This is a documentation/CLI gap. WL-5.1 does not depend on either
  override; the embedded profile is authoritative. Phase 5 reconciles the documentation.

### 3.3 Supported hosts, dependencies, and CI

- Native hosts are Windows x86-64 MSVC, Linux x86-64 GNU, and Linux AArch64 GNU. The CI workflow
  runs the Rust test, project, execution, and online-change suites on `windows-2025`,
  `ubuntu-24.04`, and `ubuntu-24.04-arm`.
- `stc-runtime` depends only on `stc-got`; `std::net` and `std::thread` are available on every
  supported host. A Modbus server needs no kernel module, C shim, privileged library, or optional
  package artifact.
- Existing integration patterns: coordinator mock tests in `coordinator.rs`, the Linux shim smoke
  test in `crates/stc-runtime/tests/ethercat_shim.rs`, project-mode CLI tests in
  `crates/stc-driver/tests/project_cli.rs`, and the DIEWU fixture in
  [tests/ethercat/diewu/](tests/ethercat/diewu/).

### 3.4 What does not exist for Modbus

- No `modbus-tcp` or `modbus-tcp-status` registration in `backend_defaults.rs`.
- No Modbus module, codec, register bank, listener, connection handling, or status device.
- No Modbus profile keys in `stc-manifest/src/profile.rs`; a Modbus `[device.<name>]` table with a
  `bind` key is rejected as an unknown key today.
- No Modbus mapping validation, diagnostics codes, tests, fixture, or evidence document.
- No dedicated Modbus backend section in the I/O specification beyond the statements that
  WL-5.1 adds a backend to the same contract and that the deferred client is WL-11.

### 3.5 Test surfaces the plan will reuse

- The coordinator and mock backend already prove dispatch, declaration order, routes, fan-in
  rejection, fault isolation, and online-change hashing. Modbus tests attach to those interfaces
  rather than introducing a parallel runtime path.
- The project CLI test harness builds a temporary project, runs `stc`, and inspects metadata and
  exit codes. The Modbus end-to-end tests extend that pattern with a real TCP client.
- The CI matrix already covers all three native hosts. No new workflow job is required; the
  existing `cargo test --workspace` and project suites run the new tests on every host.

## 4. Findings that shape the plan

1. **The plural seam is complete; WL-5.1 is an attach, not a redesign.** The coordinator already
   dispatches by `kind`, orders instances deterministically, isolates faults, and hashes the
   configuration. The Modbus backend implements `DeviceBackend` and registers two kinds. No
   coordinator, trait, route, or EtherCAT change is planned.
2. **Registration is a one-function change.** `BackendRegistry::with_defaults` currently calls
   only `register_ethercat`. Adding `register_modbus_tcp` for `modbus-tcp` and `modbus-tcp-status` makes
   `host.rs` create a coordinator for Modbus devices automatically and makes the existing status
   surface list the instance.
3. **The profile parser is EtherCAT-shaped.** It validates the EtherCAT device table and rejects
   unknown keys. Modbus needs explicit `bind` (a TOML array), `unit`, `max-connections`,
   `idle-timeout`, `word-order`, and channel-map keys with structural validation. The parser can
   accept both key families because it does not know device kinds; the runtime validates the keys
   for the kind it serves. The compiler itself stays protocol-neutral apart from structural
   profile validation.
4. **No new global backend spelling is needed.** A Modbus-only project declares
   `kind = "modbus-tcp"` and omits `[io].backend` or leaves it `null`; the coordinator is created from
   the registered kind and the unbound areas keep the `null` behavior. Adding `IoBackend::Modbus`
   would touch the manifest enum, metadata parser, runtime enum, CLI, host ABI integer values, and
   their tests for no behavioral gain. D-213 says `[io].backend` does not constrain device
   backends.
5. **The mapping must be explicit and channel-based.** Deriving Modbus addresses from image
   offsets would make the server layout depend on compiler allocation and would change silently
   when variables move. The profile maps each declared channel to one table and zero-based
   address, mirroring the EtherCAT `pdo-channels` map. The existing bindings and routes decide
   which process-image range the channel carries. Register channels are 8, 16, 32, or 64 bits; a
   wider channel occupies consecutive registers and uses a device-level word-order default with
   optional per-channel named or index-list overrides (and, for 8-bit channels, a low/high byte
   placement). Addresses are zero-based: the first element of every
   table is address 0, while vendor `00001`/`10001`/`30001`/`40001` notation is one-based and must
   be converted by subtracting one.
6. **Request handling must not run on the cycle thread.** A blocking or slow client must never
   delay a PLC scan. Listener and connection threads own sockets; a mutex-protected register bank
   is the only shared state. `latch_inputs` and `publish_outputs` perform short copies at the
   cycle boundary. This is the Modbus form of “request scheduling at the cycle boundary”: the
   boundary defines visibility, not socket work.
7. **Direction is the core correctness property.** A client write may only change a channel that
   the project declared as a physical input. PLC outputs are read-only to clients. Discrete inputs
   and input registers are read-only tables by protocol and therefore can carry only output
   channels. Every violation is an explicit exception, never a silent write.
8. **Modbus fault policy is not EtherCAT policy.** There is no fieldbus watchdog and no “stale
   input” concept. A client disconnect is normal; the last committed input values stay in the
   image and the PLC continues. Any bind/listen failure at cold start is fatal. A runtime listener
   failure is a per-listener fault that is reported and retried without stopping the scan or the
   server's other listeners. This is defined in §6.7 and is not imposed on EtherCAT.
9. **The status surface already exists, with one control-surface gap.** `BackendReport` carries
   name, device count, state, exchanges, faults, hash, and a diagnostic key/value map, and
   `print_host_status` prints all of it. The driver exposes that launcher `--status` output as
   `stc run -v`; the control `Status` JSON does not include `io_reports` today, so the
   specification's “control status surface” wording is broader than that implementation. The
   optional `modbus-tcp-status` device mirrors the existing `ethercat-status` pattern and needs no
   coordinator change.
10. **Online change is already configuration-hashed.** The hash covers the raw profile digest,
    device kinds/configs/channels, bindings, and routes. A code-only swap keeps the backend and
    its listeners; a change to the bind array, unit, word order, byte placement, table,
    address/span, channel, or binding requires a cold download (`STC-E-ONLINE-IO-001`). No
    Modbus-specific remap exists or is needed.
11. **Pure `std` is sufficient and portable.** A Modbus TCP server is a set of `TcpListener`s,
    blocking socket reads with timeouts, and a small byte-level codec. No async runtime, TLS stack,
    or external crate is required. All three native hosts support the needed APIs.
12. **Tests need a real client and deterministic ports.** The project has no Modbus client. A
    small test client/simulator in the test tree will drive all function codes, widths, orders,
    and malformed cases. In-process tests bind `127.0.0.1:0` entries and read the effective list
    from diagnostics to avoid races; subprocess tests probe free ports before writing the profile.
13. **The server is network-facing and unauthenticated.** Modbus TCP has no standard
    authentication or encryption. The profile must list one or more explicit bind addresses; there
    is no wildcard default, and the exposure is the union of every entry. The operator is told to
    use a trusted or segmented network and to treat the exposed coils/registers as writable by any
    reachable client. This is not a safety boundary.
14. **Overhead evidence is host-specific and not a CI gate.** The cycle-thread cost is the bank
    copy plus mutex acquisition, not network work. Measure latch/publish distributions with
    0/1/4/8 connected clients and a request load, and report request latency and throughput.
    Results are indicative and not a real-time claim.
15. **The documentation has three pre-existing gaps.** IO-Routing-and-Backends §2.3 and D-212 name
    a `--io-config` override that the CLI does not expose; D-218 names a `--io-ethercat-lib` flag
    while the loader uses `STC_ETHERCAT_LIBRARY` only; and the “control status surface” wording is
    broader than the implementation (the launcher `--status` output is reachable through
    `stc run -v`, but the control `Status` JSON has no I/O reports). WL-5.1 phase 5 corrects the
    documentation or records the gaps; WL-5.1 does not add a runtime override or change the control
    JSON.

## 5. Decisions (accepted 2026-11-02)

`Accepted 2026-11-02 (owner)` means the owner confirmed the decision; the table is the locked
contract. Any later change requires an explicit amendment. The decisions will be recorded in
[Decisions](specification/Decisions.md) §2 as D-229 onward during phase 5.

All owner questions (Q-1–Q-22) are resolved. Phase 0 closed on 2026-11-02; phases are handed one
at a time under §7.4.

| ID | Decision | Status |
| --- | --- | --- |
| M-1 | Implement Modbus TCP as `crates/stc-runtime/src/io_modbus_tcp.rs`, registered as the `modbus-tcp` kind and the `modbus-tcp-status` status kind, both served by one instance named `modbus-tcp`. The kind includes the transport so a future Modbus RTU kind and the deferred WL-11 Modbus TCP client cannot collide with this server. The coordinator, `DeviceBackend` trait, cycle order, route graph, configuration hash, EtherCAT backend, and shim are unchanged. No new crate, optional artifact, dependency, or host prerequisite. | Accepted 2026-11-02 (owner). |
| M-2 | Map declared channels to Modbus addresses explicitly in the selected machine profile with `[[device.<name>.channels]]` entries containing `name`, `table`, `address`, an optional `byte` for 8-bit register channels, and an optional per-channel `word-order` override for 32/64-bit channels. `name` must match a declared `[[io.devices.channels]]` channel; direction and width come from `stc.toml`. **Addresses are zero-based: the first coil, discrete input, input register, or holding register has address 0.** A channel's `address` is its first table address; 32- and 64-bit register channels occupy 2 and 4 consecutive registers. Existing `[[io.bindings]]` and routes carry the process-image range. Do not derive addresses from image offsets. | Accepted 2026-11-02 (owner). |
| M-3 | Direction and width semantics are fixed. `coil` and `holding-register` accept `input` channels (client write → PLC input) and `output` channels (PLC output → client read). `discrete-input` and `input-register` accept only `output` channels because Modbus provides no write function for those tables. `coil` and `discrete-input` carry 1-bit `BOOL` values only. `holding-register` and `input-register` carry 8-, 16-, 32-, or 64-bit channels; a boolean in a register is represented by a 16-bit channel. An 8-bit channel occupies one selected byte (`byte = "low"` default or `"high"`) of one register; two 8-bit channels may share one register only as a low/high pair. A 32/64-bit channel occupies 2/4 consecutive registers starting at `address`. A write must cover complete input-mapped channels: FC6 may write a register containing an input-mapped 8- or 16-bit channel; FC16 may write only whole-channel spans; a request that partially covers a 32/64-bit channel returns 0x02. Writes to output-mapped, unmapped, or read-only addresses return 0x02 and apply nothing; reads of unmapped ranges return 0x02. Reads of input channels return the last written bank value. Reads may address any mapped register, including part of a wider channel; the selected words are returned in the configured order. | Accepted 2026-11-02 (owner): BOOL on bit tables only; partial 32/64-bit writes rejected; unmapped ranges fail the whole request. |
| M-4 | Profile schema: `bind` is required and is a non-empty TOML array of literal IPv4/IPv6 address-and-port strings; each entry is trimmed and empty entries are rejected. `unit` is optional and, when present, is 1..=247; `max-connections` is optional with default 8 and range 1..=128, counted across all binds of the server; `idle-timeout` is optional with default 60 s and must be positive; `word-order` is optional at the device level, `high-first` (default) or `low-first`, with an optional per-channel `word-order` override (`high-first`, `low-first`, `abcd`, `cdab`, `dcba`, `badc`, or an integer index list) for 32/64-bit channels; each channel map entry has a unique channel name, a table, a **zero-based `address` in 0..=65535 where address 0 is the first element of the table**, and an optional `byte` (`low` default, `high`) valid only for 8-bit register channels. Register spans must fit in 0..=65535 and must not overlap, except a low/high 8-bit pair in one register. A concrete duplicate `ip:port` within or across devices is rejected; port-0 entries are allowed and get distinct ephemeral ports. `word-order` and `byte` changes are configuration changes. | Accepted 2026-11-02 (owner): `unit` optional with accept-any when absent; full schema accepted. |
| M-5 | Request scheduling: one listener thread per configured bind (a server device may have several) and one thread per accepted connection, with `max-connections` as the total across the server's listeners. All listeners share the server's register bank and mapping. At `latch_inputs` the backend copies input-channel bank values into the bound `%I` ranges; at `publish_outputs` it copies the bound `%Q` values into the bank. A client write is visible to the PLC at the next input boundary; a PLC output is visible to clients after the publish boundary. Multi-address writes hold the bank lock for the whole request, so a latch sees all or none. Last writer wins. Never-written input channels read zero. No socket call runs on the cycle thread. | Accepted 2026-11-02 (owner). |
| M-6 | Protocol surface: Modbus TCP MBAP framing; function codes 1 (read coils), 2 (read discrete inputs), 3 (read holding registers), 4 (read input registers), 5 (write single coil), 6 (write single register), 15 (write multiple coils), and 16 (write multiple registers). Standard quantity limits (1..=2000 bits for FC1/2, 1..=125 registers for FC3/4, 1..=1968 bits for FC15, 1..=123 registers for FC16). Exception responses 0x01 (illegal function), 0x02 (illegal data address), 0x03 (illegal data value), 0x04 (server device failure), and 0x0B (wrong unit id when `unit` is configured). Pipelined requests in one connection are processed in order. Invalid protocol id, invalid MBAP length, oversized ADU, and truncated framing close the connection; semantic PDU errors return an exception and keep it open. FC8, FC43, FC22, FC23, broadcast, and RTU are not implemented. | Accepted 2026-11-02 (owner). |
| M-7 | Byte and word order: each 16-bit register is big-endian on the wire; the process image is little-endian. Device-level `word-order` sets the default for 32/64-bit channels (`high-first` default or `low-first`). A 32/64-bit channel may override it with `high-first`, `low-first`, `abcd`, `cdab`, `dcba`, `badc`, or an explicit integer permutation of length `width/8`. Named forms are 4-byte patterns applied per 32-bit half for 64-bit values; a list gives byte positions from most significant (0) to least significant (`n-1`) in wire order from the lowest address upward. Device-level `low-first` for 64-bit reverses all four 16-bit words (GHEFCDAB), distinct from per-channel `cdab` (CDABGHEF). `byte = "low"`/`"high"` selects the byte for an 8-bit register channel; unclaimed bytes read zero and ignore writes. Coils and discrete-input bits are packed LSB-first in responses. | Accepted 2026-11-02 (owner): device default plus per-channel named/index permutation override. |
| M-8 | Connection policy: `max-connections` is the total across all binds of a server; accept-and-close at the limit; per-connection idle timeout that closes a connection after the configured interval without a complete request; per-connection read/write timeouts that also let shutdown stop promptly; `TCP_NODELAY` on accepted connections. One client's error or disconnect never affects another connection, another listener, another server device, or the PLC image. No authentication, authorization, or encryption; every bind is explicit and the trusted-network assumption is documented. | Accepted 2026-11-02 (owner). |
| M-9 | Fault policy: any cold-start bind/listen failure is all-or-nothing and fatal (`STC-E-IO-MODTCP-002`); the whole server is not started and the run never begins. At runtime each listener is isolated: a failed listener is marked `Faulted` and retried with backoff, other listeners of the same server keep serving, the server's `listening` is false until every listener is accepting, and the backend state is `Faulted` while any listener is faulted. Client malformed requests, unsupported functions, illegal addresses, timeouts, and disconnects are normal server events: they return an exception where possible, update counters, and do not fault the backend. Client disconnects never change the process image and there is no “stale input” flag. This is not a safety function. | Accepted 2026-11-02 (owner): all-or-nothing cold-start binds; runtime per-listener fault isolation. |
| M-10 | Status and diagnostics: the verbose host status (`stc run -v`) reports per-server diagnostics including the effective bind list, configured listener count, currently bound listener count, total connections, requests, reads, writes, exceptions, malformed frames, unbound channels, and last error; per-listener `server<N>.listener<M>.*` keys report bind, state, connections, and last error. Accept the optional read-only `modbus-tcp-status` device (`config = { layout = 1 }`) with per-server channels `listening`, `listeners`, `bound_listeners`, `connections`, `requests`, `writes`, `exceptions`, `malformed`, and `last_error`; `<N>` indexes the configured Modbus data devices in declaration order. Status devices have no profile entry, unknown layouts/channels are rejected, and unbound fields cost no image bytes. | Accepted 2026-11-02 (owner): status device optional; full diagnostics accepted. |
| M-11 | Diagnostic codes: `STC-E-IO-MODTCP-001` invalid device/channel configuration; `STC-E-IO-MODTCP-002` bind/listen failure at cold start; `STC-E-IO-MODTCP-003` a bound channel does not resolve or its direction/width/image range disagrees; `STC-W-IO-MODTCP-001` a declared Modbus channel is unbound (recorded in diagnostics and the traceability allowlist; no new notice path); `STC-W-IO-MODTCP-002` a runtime listener failed and is retrying. Per-request Modbus exceptions are counters, not diagnostics. | Accepted 2026-11-02 (owner). |
| M-12 | Online change: the existing configuration hash already covers the profile digest, device kind/config, channels, bindings, and routes. A code-only or same-configuration swap keeps the Modbus instance, listeners, connections, and counters. Any change to the bind array, unit, word order, byte placement, table, address, width/span, channel, or binding changes the hash and returns `STC-E-ONLINE-IO-001`; a cold download is required. No Modbus-specific remap is added. | Accepted 2026-11-02 (owner). |
| M-13 | No new global `[io].backend` spelling, `--io` value, `[test] io` value, metadata backend value, or host-ABI enum value for Modbus. A Modbus-only project omits `[io].backend` or sets `null`; `kind = "modbus-tcp"` selects the backend and unbound areas keep the `null` behavior. This preserves D-213 and avoids touching the stable host ABI for a cosmetic selector. | Accepted 2026-11-02 (owner): kind only. |
| M-14 | Evidence and platform: pure `std` on Windows x86-64 MSVC, Linux x86-64 GNU, and Linux AArch64 GNU. Use the existing CI matrix for functional and error-path coverage; add no new workflow or timing gate. Provide an in-repo Modbus test client/simulator, an end-to-end fixture under `tests/modbus/`, and an overhead harness that measures latch/publish with 0/1/4/8 clients and request latency/throughput. Cover 1/8/16/32/64-bit channels, low/high byte placement, device and per-channel word orders, single- and multi-bind servers, and **zero-based address vectors (`address 0` ↔ vendor 00001/10001/30001/40001)**. A real mixed bench run with EtherCAT and Modbus TCP active concurrently is planned on `ecat-host`, but no external Modbus↔EtherCAT signal connection is available; cross-backend routing is proven by the mixed mock test and the bench limitation is recorded. Results are host-specific and not a real-time claim. | Accepted 2026-11-02 (owner): mixed mock required; real mixed bench without external cross-wiring; independent client check optional. |
| M-15 | No runtime bind override in v1. The embedded profile is the only configuration path; tests generate a profile with a free port or use port 0 and diagnostics. Phase 5 reconciles the documented `--io-config`, `--io-ethercat-lib`, and control-status wording gaps; implementing a flag or extending the control JSON is not part of WL-5.1. | Accepted 2026-11-02 (owner): no runtime bind override; documentation-gap cleanup remains in phase 5. |

## 6. Architecture

### 6.1 Components

```text
stc.toml ── compiler ──> <name>.stc-meta.json ──> runtime I/O coordinator
                                                        │
                                      ┌─────────────────┴─────────────────┐
                                      │                                   │
                               EtherCAT backend                  Modbus backend
                               (WL-5.2, unchanged)               (WL-5.1)
                                      │                                   │
                               IgH shim/libethercat              std::net TCP listeners
                                      │                                   │
                               EtherCAT slaves                   Modbus TCP clients

                    one shared %I / %Q / %M process image
```

- **Compiler:** parses and validates the profile shape (including the new Modbus keys), resolves
  routes, and embeds the profile and merged device configuration. It does not open sockets, assign
  Modbus addresses, or interpret function codes.
- **Coordinator:** unchanged. It groups devices by `kind`, validates bindings/routes, calls
  `cold_start`, `latch_inputs`, `publish_outputs`, `status`, `diagnostics`, and `shutdown` in the
  documented order, and applies routes.
- **Modbus backend:** one instance serves every `modbus-tcp` and `modbus-tcp-status` device. It parses its
  opaque device configs, validates the channel map, register spans, and image ranges, binds and
  serves the configured endpoints, and moves values between the register bank and the process
  image at the cycle boundary.
- **Register bank:** per-server table/address storage and per-channel direction, width, span,
  byte placement, and image range. Network threads and the cycle thread share it only through a
  mutex.
- **Network threads:** one listener per configured bind, one thread per accepted connection,
  bounded by the server's total connection limit and stopped/joined on shutdown.

### 6.2 Configuration model

The logical device tree and channel descriptors stay in `stc.toml`. The selected machine profile
carries the Modbus TCP address map. The kind spelling is `modbus-tcp`, not `modbus`, so Modbus RTU
and the deferred Modbus TCP client (WL-11) remain distinct kinds. Example, reduced to one channel
per table and direction:

```toml
# stc.toml
[io]
# Optional: device kind selects the backend. A Modbus-only project may omit
# `backend` or leave it null; unbound areas keep the null behavior.
backend = "null"
# Source-controlled machine profile, resolved and embedded at build time.
config = "io/bench.toml"

[[io.devices]]
name = "modbus0"
kind = "modbus-tcp"

  [[io.devices.channels]]
  name = "CoilIn0"
  direction = "input"
  width = 1

  [[io.devices.channels]]
  name = "CoilOut0"
  direction = "output"
  width = 1

  [[io.devices.channels]]
  name = "DiOut0"
  direction = "output"
  width = 1

  [[io.devices.channels]]
  name = "ByteIn0"
  direction = "input"
  width = 8

  [[io.devices.channels]]
  name = "ByteIn1"
  direction = "input"
  width = 8

  [[io.devices.channels]]
  name = "RegIn0"
  direction = "input"
  width = 16

  [[io.devices.channels]]
  name = "RegOut0"
  direction = "output"
  width = 16

  [[io.devices.channels]]
  name = "DwordIn0"
  direction = "input"
  width = 32

  [[io.devices.channels]]
  name = "LwordOut0"
  direction = "output"
  width = 64

  [[io.devices.channels]]
  name = "IrOut0"
  direction = "output"
  width = 16

[[io.bindings]]
variable = "Main.CoilIn0"
device = "modbus0"
channel = "CoilIn0"
direction = "input"

[[io.bindings]]
variable = "Main.CoilOut0"
device = "modbus0"
channel = "CoilOut0"
direction = "output"

# The remaining channels follow the same binding pattern.
```

```toml
# io/bench.toml (selected by [io].config; embedded at build time)
[device.modbus0]
# One or more literal address:port entries in a TOML array. There is no
# wildcard default. Port 0 is allowed for tests; the effective addresses are
# reported in diagnostics. All binds share one register bank and mapping.
bind = ["192.168.1.7:15020", "192.168.2.7:567"]
unit = 1
max-connections = 8
idle-timeout = "60s"
# Optional; high-first is the default for 32- and 64-bit register channels.
word-order = "high-first"

# Addresses are zero-based: address 0 is the first element of the table
# (vendor notation 00001 / 10001 / 30001 / 40001). Vendor one-based
# reference numbers must be converted by subtracting one.

[[device.modbus0.channels]]
name = "CoilIn0"
table = "coil"
# First coil: zero-based address 0, vendor 00001.
address = 0

[[device.modbus0.channels]]
name = "CoilOut0"
table = "coil"
# Zero-based address 1024, vendor 01025.
address = 1024

[[device.modbus0.channels]]
name = "DiOut0"
table = "discrete-input"
# First discrete input: zero-based address 0, vendor 10001.
address = 0

# Two 8-bit channels sharing the first holding register (vendor 40001):
# low and high bytes.
[[device.modbus0.channels]]
name = "ByteIn0"
table = "holding-register"
address = 0
byte = "low"

[[device.modbus0.channels]]
name = "ByteIn1"
table = "holding-register"
address = 0
byte = "high"

[[device.modbus0.channels]]
name = "RegIn0"
table = "holding-register"
# Zero-based address 10, vendor 40011.
address = 10

[[device.modbus0.channels]]
name = "RegOut0"
table = "holding-register"
# Zero-based address 1024, vendor 41025.
address = 1024

# A 32-bit channel occupies zero-based addresses 20..21
# (vendor holding registers 40021..40022). The per-channel word-order
# overrides the device default; "dcba" writes the value little-endian.
[[device.modbus0.channels]]
name = "DwordIn0"
table = "holding-register"
address = 20
word-order = "dcba"

# A 64-bit channel occupies zero-based addresses 30..33
# (vendor input registers 30031..30034). The index list is an explicit
# permutation; this one is the same as device-level "low-first" for 64 bits.
[[device.modbus0.channels]]
name = "LwordOut0"
table = "input-register"
address = 30
word-order = [6, 7, 4, 5, 2, 3, 0, 1]

[[device.modbus0.channels]]
name = "IrOut0"
table = "input-register"
# First input register: zero-based address 0, vendor 30001.
address = 0
```

Rules:

- `bind` is required for a `modbus-tcp` data device and is a non-empty TOML array of
  `"address:port"` strings. Each entry must be a literal IPv4 or IPv6 socket address (IPv6 uses
  the bracketed `[address]:port` form); a hostname is rejected rather than resolved at build or
  cold start. Entries are trimmed and empty entries are rejected. All entries bind the same
  register bank and mapping; one listener is created per
  entry. If any entry fails to bind at cold start, the whole backend instance fails to start and
  no listener is left running.
- `unit` is optional. When present, requests for a different unit id receive exception 0x0B. When
  absent, the unit id is accepted and echoed, so ordinary clients that use 0 or 1 work without
  extra configuration. One `unit` applies to every bind of the server; devices that need different
  unit ids are declared as separate server devices.
- `max-connections` and `idle-timeout` are optional and apply per server device. Defaults are 8
  and 60 s. `max-connections` is the total across all of the server's binds.
- `word-order` is optional at the device level: `"high-first"` (the default) or `"low-first"`.
  A 32/64-bit channel may set its own `word-order` to `"high-first"`, `"low-first"`, `"abcd"`,
  `"cdab"`, `"dcba"`, `"badc"`, or an integer index list. See §6.3 for the exact wire orders and
  the 64-bit expansion.
- Every declared channel of a `modbus-tcp` device must have exactly one profile mapping, and every
  mapping must name a declared channel. Duplicate channel names, undeclared names, width/table
  mismatches, and overlapping register spans are rejected at cold start. A low/high 8-bit pair may
  share one register; no other overlap is allowed.
- A channel's `address` is zero-based. For `coil` and `discrete-input`, address 0 is the first bit
  (vendor `00001`/`10001`). For `holding-register` and `input-register`, address 0 is the first
  register (vendor `40001`/`30001`), and the channel occupies one register for 8/16 bits, two for
  32 bits, and four for 64 bits. The span must fit in 0..=65535. An 8-bit channel's `byte` is
  `"low"` (the default) or `"high"`; a `byte` key on any other width is rejected at cold start.
- A concrete duplicate `ip:port` is rejected within a device and across `modbus-tcp` devices.
  Port-0 entries are allowed and receive distinct ephemeral ports; the effective list appears in
  diagnostics.
- `pdo-channels` and the EtherCAT keys do not apply to a Modbus device. The Modbus backend rejects
  `syncs`, `pdo-channels`, or `dc` on a `modbus-tcp` device with `STC-E-IO-MODTCP-001`; the EtherCAT
  backend's handling of Modbus keys is unchanged. The compiler's profile parser validates only
  structure; the backend validates semantics.
- Inline `[io.devices].config` remains valid as a field-level default for fields the profile
  omits; profile fields win field by field, as in D-212. For a `modbus-tcp` data device the
  selected profile is the configuration path (M-15) and must supply the channel map (M-2); there
  is no profile-less or runtime-override bind path.
- A `modbus-tcp-status` device has no profile entry and uses inline `config = { layout = 1 }`.

Structural profile errors (wrong TOML type, unknown key, malformed bind entry, an unknown
`word-order` spelling or non-integer list element, an invalid `byte` value, zero-based address
outside 0..=65535) use the existing `STC-E-PROJECT-009`. Runtime semantic errors (undeclared
channel, width/direction mismatch, duplicate runtime mapping, overlapping span, a `word-order`
permutation of the wrong length, range, or duplicates, `byte` on a non-8-bit channel, bind
failure) use the `STC-E-IO-MODTCP-*` codes; §6.3 assigns the permutation error to
`STC-E-IO-MODTCP-001`.

### 6.3 Mapping model

The process-image side is unchanged: each bound `BackendChannel` carries a direction, a width, and
an optional `ImageRange`; the coordinator validates and groups it. The protocol side is the new
profile map.

**Zero-based addressing.**

The four data tables are independent address spaces. The profile and runtime use **zero-based
protocol addresses**. The first coil, the first discrete input, the first input register, and the
first holding register all have **address 0**. The profile never uses a vendor one-based reference
number; the value in `address` is the number placed in the Modbus PDU.

| Table | Vendor notation of the first element | `address = 0` | `address = 1` | `address = 1024` |
| --- | --- | --- | --- | --- |
| Coil | 00001 | 00001 | 00002 | 01025 |
| Discrete input | 10001 | 10001 | 10002 | 11025 |
| Input register | 30001 | 30001 | 30002 | 31025 |
| Holding register | 40001 | 40001 | 40002 | 41025 |

Rules:

- **The first register has address 0.** So does the first coil, discrete input, and input register.
  `address = 1` is the second element; `address = 1024` is the 1025th element.
- The `0xxxx`, `1xxxx`, `3xxxx`, and `4xxxx` reference numbers in vendor documentation are
  one-based. Convert them by subtracting one before writing the profile: vendor holding register
  `40021` is `address = 20`, and a 32-bit channel at `address = 20` covers vendor `40021..40022`.
- A vendor document that says "register 1" is ambiguous: it may mean the first register (address 0)
  or the second (address 1). Confirm against a known value or the device's PDU trace. The server
  only accepts zero-based addresses and never guesses.
- A multi-register channel's `address` is its first zero-based register. A 64-bit channel at
  `address = 30` covers registers 30, 31, 32, and 33 (vendor `40031..40034` on a holding-register
  table). An 8-bit low/high pair at `address = 0` is the first register (vendor `40001`).
- The Modbus PDU is already zero-based. The backend writes the configured `address` directly into
  the PDU and performs no off-by-one conversion at any layer.
- Diagnostics, the evidence report, and every example use zero-based values. Documentation must
  label one-based values explicitly as "vendor reference numbers" or "vendor notation", never as
  `address`.
- Tests pin both notations: address 0 ↔ `00001`/`10001`/`30001`/`40001`, address 1 ↔
  `00002`/`10002`/`30002`/`40002`, and address 1024 ↔ `01025`/`11025`/`31025`/`41025`. A test that
  maps the first register to `address = 1` must fail the golden vector.
- The report and [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) state the zero-based convention
  explicitly because vendor off-by-one differences are a common integration fault.

Table and direction matrix:

| Declared channel direction | `coil` | `discrete-input` | `holding-register` | `input-register` |
| --- | --- | --- | --- | --- |
| `input` (client → PLC) | Writable by FC5/15; readable by FC1; bank value latched to `%I` | Rejected: the table has no write function | Writable by FC6/16; readable by FC3; bank value latched to `%I` | Rejected: the table has no write function |
| `output` (PLC → client) | Readable by FC1 from the last published `%Q`; writes return exception 0x02 | Readable by FC2 from the last published `%Q`; no write function exists | Readable by FC3 from the last published `%Q`; writes return exception 0x02 | Readable by FC4 from the last published `%Q`; no write function exists |

Width and occupancy rules:

- The supported width set is `BOOL` (1 bit) on coils and discrete inputs, and `BYTE`, `WORD`,
  `DWORD`, and `LWORD` (8, 16, 32, and 64 bits) on holding and input registers.
- `coil` and `discrete-input` channels must be 1 bit (`BOOL`).
- `holding-register` and `input-register` channels must be 8, 16, 32, or 64 bits. A `BOOL` is not
  mapped to a register table; use a 16-bit channel for a 0/1 flag.
- A register channel's `address` is its first zero-based register (address 0 is vendor `40001` on
  holding registers and `30001` on input registers). It occupies `ceil(width / 16)` registers:
  1 for an 8- or 16-bit channel, 2 for a 32-bit channel, and 4 for a 64-bit channel. The span must
  fit in 0..=65535 and must not overlap another channel's span, except that one low and one high
  8-bit channel may share the same register.
- An 8-bit register channel uses `byte = "low"` (default) or `"high"` to select the byte within
  the register. Two 8-bit channels at the same address must use opposite byte positions. An 8-bit
  channel may not share a register with a 16-, 32-, or 64-bit channel.
- An 8-, 16-, 32-, or 64-bit channel's image range must be byte-aligned; a 1-bit channel may use
  any bit. A range that leaves its area or disagrees with the declared width is
  `STC-E-IO-MODTCP-003`.

Word and byte order:

- Every 16-bit register is big-endian on the wire. The process image is little-endian; the
  backend converts the numeric value, not the raw image bytes.
- Device-level `word-order` sets the default for 32/64-bit channels: `high-first` (default) or
  `low-first`. A 32/64-bit channel may override it with `high-first`, `low-first`, `abcd`, `cdab`,
  `dcba`, `badc`, or an explicit integer permutation. `high-first` and `low-first` order whole
  16-bit words; the four letter forms are 4-byte patterns and, for a 64-bit value, apply to each
  32-bit half. `low-first` reverses all four 16-bit words at 64 bits (GHEFCDAB), whether it is the
  device default or a per-channel override; per-channel `cdab` is the per-half pattern
  (CDABGHEF).
- Letters A..H denote the value bytes from most significant (A) to least significant (H). The
  table lists wire order from the lowest address to the highest; the last column is the equivalent
  index list for the 32-bit form.

  | `word-order` | 32-bit wire order | 64-bit wire order | 32-bit index list |
  | --- | --- | --- | --- |
  | `high-first` | A B C D | A B C D E F G H | `[0, 1, 2, 3]` |
  | `low-first` | C D A B | G H E F C D A B | `[2, 3, 0, 1]` |
  | `abcd` | A B C D | A B C D E F G H | `[0, 1, 2, 3]` |
  | `cdab` | C D A B | C D A B G H E F | `[2, 3, 0, 1]` |
  | `dcba` | D C B A | D C B A H G F E | `[3, 2, 1, 0]` |
  | `badc` | B A D C | B A D C F E H G | `[1, 0, 3, 2]` |

- An explicit integer list is a permutation of `0..n-1`, where `0` is the most significant value
  byte and `n-1` the least; the list gives the value-byte positions in wire order from the lowest
  address upward. The length is 4 for a 32-bit channel and 8 for a 64-bit channel. Duplicates,
  out-of-range values, and the wrong length are rejected with `STC-E-IO-MODTCP-001`.
- Example: the 32-bit value `0xAABBCCDD` is written as `AA BB CC DD` with `abcd`, `CC DD AA BB`
  with `cdab`, `DD CC BB AA` with `dcba`, and `BB AA DD CC` with `badc` at addresses `N..N+1`.
  Each 16-bit register keeps its big-endian wire order.
- For an 8-bit channel, `byte = "low"` selects the least significant byte of the register and
  `"high"` the most significant. For register value `0x1234`, low is `0x34` and high is `0x12`.
- The default `high-first`/`low-first` modes order whole 16-bit words; a per-channel named or
  index-list override may also reorder bytes within the value.

Value and write rules:

- Reads of `input` channels return the last client-written bank value. PLC code cannot write
  `%I`, and fan-in is rejected, so the bank value and the PLC input are the same value after the
  latch.
- Never-written input channels read zero until the first write. Input defaults are not applied to
  bound bits (D-228); the backend's zero is the initial bound value.
- Write behavior by channel width (the channel must also be input-mapped; a write to an address
  mapped only to output channels is 0x02):

  | Width and table | FC5/15 | FC6 | FC16 |
  | --- | --- | --- | --- |
  | 1-bit coil | Allowed | Not applicable | Not applicable |
  | 1-bit discrete input | Not applicable (read-only table) | Not applicable | Not applicable |
  | 8-bit register | Not applicable | Allowed; updates each input-claimed byte in the register | Allowed; updates each input-claimed byte in the written registers |
  | 16-bit register | Not applicable | Allowed | Allowed |
  | 32-bit register | Not applicable | Rejected: partial channel (0x02) | Allowed only when the request covers its 2-register span |
  | 64-bit register | Not applicable | Rejected: partial channel (0x02) | Allowed only when the request covers its 4-register span |

- An FC16 request must cover complete input-mapped channels. If its range partially covers a 32-
  or 64-bit channel, the request returns 0x02 and applies no part of the request. A write request
  returns 0x02 and applies nothing if any address in its range is unmapped, read-only, or mapped
  only to output channels. An address that carries an input-mapped 8- or 16-bit channel is
  writable; for an 8-bit low/high pair that mixes an input byte with an output byte, only the
  input byte is updated and the output byte is preserved.
- An 8-bit register write updates its selected byte and any other input-claimed byte in the
  register. A byte claimed by an output-mapped 8-bit channel is preserved; an unclaimed byte is
  ignored. Reads of the register return the claimed bytes, with unclaimed bytes read as zero.
- Reads may address any mapped register, including part of a 32- or 64-bit channel; the selected
  words are returned in the configured word order. A read range that contains an unmapped address
  returns 0x02 with no partial data.
- Multi-address writes are atomic with respect to the cycle boundary: the whole request is applied
  under one bank lock. Two clients writing the same channel produce last-writer-wins.
- Unbound declared channels are counted in diagnostics and reserve their addresses; their values
  have no process-image effect. The stable code `STC-W-IO-MODTCP-001` names the condition. Emission
  follows the existing backend-warning policy: the trait has no notice channel today, so the
  condition is recorded in diagnostics and the traceability allowlist rather than emitted as a
  new warning path.

### 6.4 Request scheduling and the cycle boundary

The coordinator calls the backend synchronously at two points; the backend must not block on the
network there.

1. **Cold start.** Parse and validate every device config; validate channel maps, widths, spans,
   byte pairs, directions, zero-based address bounds, and image ranges; bind every configured
   address; only after every bind succeeds, spawn the listener threads and seed the bank to zero.
   A failure closes every listener already bound in this instance and returns an error; the
   coordinator then shuts down previously started instances in reverse order. A partial server is
   never left accepting requests. `cold_start` is idempotent: a previously started instance is
   shut down first.
2. **Input boundary (`latch_inputs`).** Lock the bank; copy each bound input channel's bank value
   into its image range; fill the `modbus-tcp-status` ranges; unlock. This is the commit point for
   client writes. Requests racing with the lock are either fully before or fully after the copy.
3. **After the scan (`publish_outputs`).** Lock the bank; copy each bound output channel's image
   value into the bank; unlock; increment the exchange counter. This is the commit point for PLC
   outputs that clients can read.
4. **Shutdown.** Set the stop flag; stop accepting; close listeners; wake and join connection
   threads; set `Stopped`. Shutdown is idempotent and is also called from `Drop`, so a failed or
   dropped generation cannot leak a listener or a thread. Shutdown never rewrites image values.

Threads and shared state:

- Each configured bind has one listener thread. All `modbus-tcp` devices share one
  `ModbusTcpBackend` and one aggregate status; each server device has its own bank, listener set,
  connection limit, and counters.
- Each accepted connection gets one thread. The number of live connection threads per server is
  bounded by `max-connections` across all of its listeners; an accepted connection above the limit
  is closed immediately and counted. A connection-slot guard releases the slot on every exit path,
  including a connection thread panic; the listener keeps serving after an isolated
  connection-thread failure.
- Listener and connection threads check the stop flag and a short socket timeout, so shutdown does
  not wait for an idle client. Socket errors are reported to the shared state, not unwrapped.
- The only cycle-thread work is the bank/image copy and the status fill. No DNS, `accept`, `read`,
  `write`, or flush runs on the cycle thread. PDU decoding and response encoding happen on the
  connection thread before or after the lock; only the bank apply is under the lock.
- The bank is a `std::sync::Mutex`; all request handlers and the two boundary copies take it for
  short, bounded sections. The copy cost is proportional to the number of bound channels, not to
  connection count or request rate.

Visibility and ordering:

- A client write completed before the input-boundary lock is visible to the scan that follows;
  a write that arrives after the latch is visible to the next scan.
- A client read completed after the publish-boundary unlock sees the scan's output values; a read
  that races with the copy sees either the previous or the new values, never a partial publish,
  because the whole bank copy is performed under one lock.
- Multi-address writes are atomic: a latch cannot observe half of an FC15/FC16 request.
- Last writer wins between clients. There is no optimistic concurrency or ownership.
- Never-written input channels read zero. Client disconnects do not clear or stale them.

### 6.5 Protocol surface

Framing (Modbus TCP):

- MBAP header: transaction id (2 bytes, echoed), protocol id (2 bytes, must be 0), length (2 bytes,
  unit id + PDU), unit id (1 byte).
- Maximum ADU is 260 bytes (7-byte header + 253-byte PDU). A length below 2 or above 254, an
  unsupported protocol id, or an oversized frame closes the connection.
- A connection may pipeline multiple ADUs; they are processed in order. A partial ADU is
  accumulated until complete or the idle timeout expires.
- Responses echo the transaction id and unit id and carry protocol id 0.

Function codes:

| FC | Request | Success response | Quantity / size |
| --- | --- | --- | --- |
| 1 | Read Coils: start, quantity | byte count, packed bits | 1..=2000 bits |
| 2 | Read Discrete Inputs: start, quantity | byte count, packed bits | 1..=2000 bits |
| 3 | Read Holding Registers: start, quantity | byte count, big-endian registers | 1..=125 registers |
| 4 | Read Input Registers: start, quantity | byte count, big-endian registers | 1..=125 registers |
| 5 | Write Single Coil: address, 0xFF00/0x0000 | echo request | one coil |
| 6 | Write Single Register: address, value | echo request | one register |
| 15 | Write Multiple Coils: start, quantity, byte count, packed bits | echo start/quantity | 1..=1968 bits |
| 16 | Write Multiple Registers: start, quantity, byte count, values | echo start/quantity | 1..=123 registers |

All start and address fields in these PDUs are zero-based protocol addresses. Vendor one-based
reference numbers (`00001`/`40001`) never appear on the wire; the server uses the configured
zero-based `address` directly.

Unsupported function codes return exception 0x01. FC8 (diagnostics), FC22 (mask write), FC23
(read/write multiple), FC43 (device identification), broadcast, and RTU are not implemented.

Exception responses:

| Code | Condition |
| --- | --- |
| 0x01 | Unsupported function code. |
| 0x02 | An address is unmapped, write-protected, or the requested range leaves the 16-bit address space, or a write partially covers a 32/64-bit channel. |
| 0x03 | A quantity, byte count, or coil value is invalid for the function code. |
| 0x04 | An internal backend failure while serving a valid request. |
| 0x0B | The request names a unit id different from the configured `unit` (only when `unit` is configured). |

Error handling boundary:

- Invalid protocol id, invalid MBAP length, an ADU larger than 260 bytes, and a truncated frame
  whose connection closes are framing errors: close the connection and count a malformed frame.
- An unsupported function, illegal address, illegal value, or partial write of a 32/64-bit
  channel is a PDU error: return the exception and keep the connection open.
- A write to an output-mapped address is an illegal data address (0x02), not an internal error.
- A failed socket write closes the connection; it never faults the backend.

### 6.6 Connection and lifecycle policy

- `bind` is explicit and required. It is a non-empty TOML array of literal `"address:port"`
  entries; there is no wildcard default. All entries serve the same register bank and mapping.
  A loopback bind is the recommended local HMI/test configuration; a production remote-I/O
  deployment must choose its interfaces deliberately.
- On Unix, ports below 1024 (including the standard Modbus port 502 and the example's port 567)
  require `CAP_NET_BIND_SERVICE` or a privileged service manager. The example also uses 15020;
  production documents the privilege prerequisites.
- `max-connections` default 8, range 1..=128, counted across all binds of the server. At the
  limit, a newly accepted connection is closed immediately and counted; existing connections are
  unaffected. `idle-timeout` default 60 s; a connection with no complete request for the interval
  is closed. A partial ADU that stops mid-frame is closed under the same timeout.
- Keep-alive is not required. A client may open, transact, and close a connection for every poll;
  each connection is independent.
- `TCP_NODELAY` is set on accepted connections to avoid Nagle-induced request latency.
- Cold start binds every configured entry before spawning any thread. Any bind failure is fatal
  with `STC-E-IO-MODTCP-002`; the error names the device and address, and every listener already
  bound in the backend instance is closed. No partial bind set is left running.
- Shutdown stops accepting on every listener, closes them, signals connection threads, and joins
  them. It does not write to the process image and does not send a final response to in-flight
  clients.
- A runtime listener error marks that listener `Faulted`, records `last_fault` and the diagnostics
  error, and retries it with a bounded backoff. Other listeners of the same server continue;
  `server<N>.listening` is false until all listeners are accepting. The scan continues throughout;
  the PLC keeps the last committed input values and continues publishing outputs. There is no
  `input.stale` channel.

### 6.7 Fault and recovery policy

| State | Entry | Behavior |
| --- | --- | --- |
| `Ready` | Cold start completed; all configured binds are listening; threads running | Accept requests; values move only at the cycle boundaries. |
| `Running` | First successful latch/publish | Serve requests and publish outputs. |
| `Faulted` | Any listener runtime failure | Record the fault; stop accepting on the failed listener; the server's other listeners keep serving; keep the cycle running; keep the last committed inputs; keep publishing outputs; retry the failed listener. |
| `Recovering` | A retry is in progress after a fault | Attempt to bind/listen again with backoff; other listeners and servers continue; the aggregate backend state stays `Faulted` until every listener is accepting; return to `Running` on success, or stay `Faulted` on failure. |
| `Stopped` | Shutdown | No listeners, no threads, no image rewrite. |

The aggregate backend state is `Faulted` while any listener is faulted, including during a retry
(M-9); `Recovering` names the retry activity, and the per-listener state remains `faulted` until
the listener is accepting again.

- Client-level errors are not backend faults. A malformed request, an unsupported function, an
  illegal address, an idle timeout, and a disconnect are normal events that update counters. They
  never change the process image and never stop the run.
- A cold-start bind/listen failure is fatal: the run never begins and no outputs are published.
- A runtime listener failure is non-fatal and retried indefinitely. It is reported as
  `STC-W-IO-MODTCP-002` and in status. The PLC sees `server<N>.listening = FALSE` and a reduced
  `server<N>.bound_listeners` if those channels are bound. Other listeners of the same server and
  other server devices in the same backend continue serving.
- Fault state is per backend, with per-listener state reported in diagnostics. A Modbus listener
  fault does not alter EtherCAT data, and an EtherCAT fault does not alter Modbus data. The
  coordinator aggregates both reports.
- A client write to a bound input changes the image only at the next latch; a fault or disconnect
  does not zero, default, or stale the last value.
- This behavior is not a safety function. The PLC logic must decide what to do when the server is
  not listening; safety must come from a separate system and machine design.

### 6.8 Status and diagnostics

The verbose host status (`stc run -v`) already prints one `io-backend` line per instance. The
Modbus instance adds these stable diagnostic keys per server, indexed by declaration order:

| Key | Meaning |
| --- | --- |
| `server<N>.bind` | Effective bind list after binding, rendered as a comma-separated string in configuration order (including actual ports for port 0). |
| `server<N>.listeners` | Number of configured listener addresses. |
| `server<N>.bound_listeners` | Number of listeners currently bound and accepting. |
| `server<N>.connections` | Current live connection count across all listeners. |
| `server<N>.peak_connections` | Highest live count since start. |
| `server<N>.requests` | Complete ADUs processed across all listeners. |
| `server<N>.reads` | Successful read functions (FC1/2/3/4). |
| `server<N>.writes` | Successful write functions (FC5/6/15/16). |
| `server<N>.exceptions` | Exception responses sent. |
| `server<N>.malformed` | Framing errors that closed a connection. |
| `server<N>.unbound` | Declared channels with no image binding. |
| `server<N>.last_error` | Last backend/listener error code, 0 when none. |

Per-listener diagnostics are also printed:

| Key | Meaning |
| --- | --- |
| `server<N>.listener<M>.bind` | Effective `ip:port` of listener `M`, in configuration order. |
| `server<N>.listener<M>.state` | `listening`, `faulted`, or `stopped`. |
| `server<N>.listener<M>.connections` | Current live connections on that listener. |
| `server<N>.listener<M>.last_error` | Last error for that listener, 0 when none. |

`last_error` is 0 (none), 1 (bind/listen), 2 (accept), or 3 (internal). The status device exposes
the same code in `server<N>.last_error`.

`BackendReport.exchanges` counts completed publish boundaries; `faults` counts listener fault
episodes plus coordinator-observed errors. Counters are since start and saturate rather than wrap.

The optional PLC-visible status device has `kind = "modbus-tcp-status"` and
`config = { layout = 1 }`. It has no profile entry. Example binding of two fields:

```toml
[[io.devices]]
name = "MBStatus"
kind = "modbus-tcp-status"
config = { layout = 1 }

  [[io.devices.channels]]
  name = "server0.listening"
  direction = "input"
  width = 1

  [[io.devices.channels]]
  name = "server0.requests"
  direction = "input"
  width = 32
```

Layout 1:

| Channel | Type | Meaning |
| --- | --- | --- |
| `server<N>.listening` | `BOOL` | Every configured listener is bound and accepting. |
| `server<N>.listeners` | `UINT` | Number of configured listeners. |
| `server<N>.bound_listeners` | `UINT` | Number of listeners currently accepting. |
| `server<N>.connections` | `UINT` | Current live connections across all listeners. |
| `server<N>.requests` | `UDINT` | Complete ADUs processed. |
| `server<N>.writes` | `UDINT` | Successful writes. |
| `server<N>.exceptions` | `UDINT` | Exception responses. |
| `server<N>.malformed` | `UDINT` | Framing errors. |
| `server<N>.last_error` | `UDINT` | Last backend/listener error code. |

- `<N>` is the zero-based index of the configured Modbus data device in declaration order; status
  devices are not counted.
- Only `AT %I` bindings are allowed. A `%Q` binding, a write, an unknown channel, an unknown
  layout, or a width mismatch is rejected.
- A project binds only the fields it needs; unbound fields cost no image bytes. The backend fills
  the bound ranges at the input boundary before the module runs.
- Status values are diagnostic, not safety-rated.

### 6.9 Online change

The existing `configuration_hash` already covers the profile digest, each device's name, kind,
config text, and channels, and each binding's area/offset/width. The Modbus bind array, unit, word
order, byte placement, table, address/span, and channel map are part of that config text and
channels, so:

- a code-only swap, or a swap whose configuration hash is unchanged, keeps the Modbus instance,
  its listeners, its live connections, and its counters;
- a changed bind array entry, port, unit, word order, byte placement, table, address/span,
  channel direction/width, or binding changes the hash and returns `STC-E-ONLINE-IO-001`; the
  active generation continues and a cold download is required;
- a client connect/disconnect is not a configuration change;
- no Modbus-specific image remap is added. Bound ranges follow the existing host remap rules; a
  range change is a configuration change.

Tests must cover the preserved-connection case and the cold-required case.

### 6.10 Security and trust boundary

Modbus TCP has no standard authentication or encryption. The server exposes writable input
channels to any client that can reach any of its bound addresses.

- Every bind is explicit. Do not use a wildcard or an internet-reachable interface by accident;
  a server may bind several interfaces, so audit the whole array.
- Use a trusted or segmented network, or a host firewall that limits clients on every bind.
- The process image written by clients is ordinary PLC input; a malicious client can change it.
  The PLC logic must treat network input like any other untrusted field input.
- Connection limits and idle timeouts bound resource use but do not prevent a determined denial of
  service from a reachable network.
- The server is not a safety function and provides no fail-safe output state. See
  [NON-SAFETY.md](NON-SAFETY.md).
- Document these limits in [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) and the evidence report.
  WL-11 (client) and any transport-security work remain separate.

### 6.11 Platform, packaging, and performance

- The backend is compiled on every supported host and registered unconditionally. It has no
  platform `cfg`; `std::net` and `std::thread` are the only platform APIs.
- No new dependency, feature, optional artifact, installer step, or privileged helper is added.
  The base package is unchanged in its dependency graph.
- Windows Firewall may prompt when any non-loopback bind is used; the runbook documents the
  explicit-bind rule and the loopback example.
- Timing evidence is collected with the release build. The cycle-thread cost is the bank copy and
  mutex; network work is on other threads. Report latch/publish p50/p99/max, request round-trip
  latency, throughput, and CPU with 0/1/4/8 clients. Results are host-specific and not a hard
  real-time guarantee.
- The server is not suitable for interpolated motion; Modbus latency and polling semantics make
  it unsuitable as a motion channel.

### 6.12 Compiler and runtime boundary

**Compiler:**

- parses `stc.toml`, resolves and embeds `[io].config` as today;
- extends `stc-manifest::parse_profile` with structural validation of the Modbus device keys, the
  bind array, device and per-channel `word-order` (named forms or integer index lists), the channel
  map, and the optional `byte` placement;
- validates device/channel uniqueness and route endpoints as today;
- does not open sockets, resolve the bind array, validate register-span overlaps, assign protocol
  addresses, interpret function codes, or add a protocol-specific field to the language or
  generated code.

**Runtime:**

- registers `modbus-tcp` and `modbus-tcp-status`;
- parses the opaque device config JSON, validates the table/address/width/span/direction map and
  byte placement, resolves channels to image ranges, binds every address, and serves requests;
- delegates no coordinator work to the backend: routes remain the coordinator's responsibility;
- reports status and diagnostics through the existing trait methods.

## 7. Work breakdown

| Phase | Work | Acceptance | Size |
| --- | --- | --- | --- |
| 0 | Lock the contracts: accept or amend M-1–M-15; record the profile schema including the bind array, the width/span/byte/word-order matrix, the zero-based addressing convention, protocol surface, scheduling model, connection/fault policy, status layout, diagnostic codes, and fixture design | Decisions recorded; schema and test matrix reviewed; no implementation started before the mapping and scheduling contracts are settled | S |
| 1 | Protocol core and register bank: MBAP/PDU codec, FC1/2/3/4/5/6/15/16, exceptions, table/address maps, 1/8/16/32/64-bit values, register spans, byte placement, device-level word order and per-channel permutation overrides, channel validation, bank, conversion, **zero-based address vectors**. Unit and property tests; no sockets | Codec and bank tests pass on all three hosts; malformed/truncated inputs cannot panic; table/direction/width/span rules, 8-bit low/high placement, device defaults, per-channel permutation orders, and `address 0 ↔ 00001/10001/30001/40001` vectors are covered; no socket code yet | M |
| 2 | Backend, profile, and coordinator integration: new `io_modbus_tcp` module; `register_modbus_tcp`; profile parser keys, bind-array and device/per-channel word-order validation; `cold_start`/`latch_inputs`/`publish_outputs`/`status`/`diagnostics`/`shutdown`; listener per bind and connection threads; `modbus-tcp-status`; loopback integration tests; mixed Modbus + mock EtherCAT test | `kind = "modbus-tcp"` selects the backend end to end; a real loopback client writes inputs visible at the next latch and reads outputs published after the scan; all four tables and all five widths work; a server with two binds serves both from one bank; status and diagnostics appear; the coordinator serves Modbus and the mock EtherCAT over one image; existing null/memory/file and EtherCAT tests are unchanged | M |
| 3 | Error paths, limits, lifecycle, and online change: malformed MBAP/PDU, protocol id, length, oversized/truncated frames, unsupported functions, quantity/byte-count errors, unmapped/read-only writes, partial 32/64-bit writes, invalid unit, partial frames, idle timeout, connection limit across binds, multiple clients, multiple server devices, duplicate/overlapping addresses and spans, duplicate bind entries, one-bind failure, listener retry, shutdown/drop/trap, atomic multi-writes, last-writer-wins, code-only swap and cold-required change, status device validation | Error matrix passes; no request can silently corrupt the image; all configuration errors fail closed with the documented codes; listener failure and recovery are observable per listener; multi-write atomicity and concurrency tests pass; online-change tests distinguish preserve from cold-required | M |
| 4 | Cross-host and end-to-end evidence: `tests/modbus/` fixture (ST project, profile, client/simulator, scripts, README); `stc run` end-to-end on Windows x86-64, Linux x86-64, Linux AArch64; project-mode and existing `[test] io` tests; mixed mock test; real mixed bench run on `ecat-host` with EtherCAT and Modbus TCP active concurrently, with no external Modbus↔EtherCAT signal connection; optional independent third-party client check | The fixture builds and runs on all three native hosts; the CI matrix is green; the end-to-end test writes all input tables and reads all output tables through a real TCP client, covering 8/16/32/64-bit values, device-default and per-channel permutation orders, and a multi-bind server; the mixed coordinator mock test covers cross-backend routes; the real bench run shows both backends active concurrently, and the no-external-cross-wiring limitation is recorded; any independent-client caveat is recorded | M |
| 5 | Overhead, documentation consolidation, and PR/review: overhead harness and results; consolidate permanent documentation into `specification/` (I/O contract, CLI/manifest, report, known limitations, README/user guide, decisions); remove stale and working-only content from this plan; update V2.0.0-PLAN.md with the implemented work and FutureWishlist; reconcile the `--io-config`, `--io-ethercat-lib`, and control-status gaps; open the `WL-5.1` → `v2.0.0-planning` pull request; final CI and owner review | Overhead distributions and method are reported; permanent documentation is in `specification/`; stale working content is removed; V2.0.0-PLAN.md is updated; the WL-5.1 PR is green and owner-approved; after merge the working plan is retired and its permanent content is in `specification/`; known limitations state non-RT, no-auth, boolean-on-bit-tables, zero-based addressing, 8-bit byte placement, word-order scope, and security boundaries; decisions are recorded as D-229 onward; CI is green; owner accepts | M |

The total is L, matching the parent plan's M/L estimate. The orchestrator assigns phase owners
under §7.4; each phase ends with a commit, a push, and a green CI run. A phase may fix a failing CI
run once; a second
consecutive failure stops the sequence for a human decision and returns control to the
orchestrator (§7.4).

### 7.1 Phase status

| Phase | Status | Commit | CI |
| --- | --- | --- | --- |
| 0 — contracts | Done | `3547e58` | [41473961570](https://github.com/r2k-in-the-vortex/stc/actions/runs/41473961570) — success |
| 1 — protocol core and bank | Done | `ebfbbc6` | [41478485216](https://github.com/r2k-in-the-vortex/stc/actions/runs/41478485216) — success |
| 2 — backend and coordinator integration | Done | `6c55414` | [41483247583](https://github.com/r2k-in-the-vortex/stc/actions/runs/41483247583) — success |
| 3 — error paths, lifecycle, online change | Done | `97c592d` | [41488899175](https://github.com/r2k-in-the-vortex/stc/actions/runs/41488899175) — success |
| 4 — cross-host and end-to-end | In progress | — | — |
| 5 — overhead, documentation consolidation, PR | Not started | — | — |

Status values: Not started, In progress, Done, Blocked. The orchestrator (this agent) updates this
table before handing the next phase to a subagent; subagents do not edit it. The next phase's
commit carries the status update.

### 7.2 Planned file changes

| File | Change |
| --- | --- |
| `crates/stc-runtime/src/io_modbus_tcp.rs` | New backend: codec, register bank, 1/8/16/32/64-bit mapping with spans and order, config/channel validation, listener per bind, connection threads, status, diagnostics, tests. |
| `crates/stc-runtime/src/lib.rs` | Add `pub mod io_modbus_tcp;` and export `ModbusTcpBackend`. |
| `crates/stc-runtime/src/backend_defaults.rs` | Register `modbus-tcp` and `modbus-tcp-status` as one instance. |
| `crates/stc-manifest/src/profile.rs` | Accept and structurally validate the `bind` array, `unit`, `max-connections`, `idle-timeout`, device and per-channel `word-order`, and the channel-map array with `byte`. |
| `crates/stc-driver/src/build.rs` | Only if a compile-time kind/config cross-check is added; no profile embedding or route change is required. |
| `crates/stc-driver/src/cli.rs` | No change: no new global backend spelling. |
| `crates/stc-runtime/src/coordinator.rs` | No behavior change; tests may live here or in `io_modbus_tcp.rs`. |
| `crates/stc-runtime/src/io_ethercat.rs` | No change. |
| `crates/stc-runtime/tests/` | New Modbus integration/error/concurrency tests covering widths, order, and multiple binds, plus a reusable test client. |
| `crates/stc-driver/tests/` | Project-mode end-to-end and manifest/profile tests. |
| `tests/modbus/` | New fixture: `stc.toml`, profile with a multi-address bind array and zero-based address map, ST program with 1/8/16/32/64-bit channels, client/simulator, scripts, README. |
| `tests/modbus/mixed/` | Real mixed bench fixture/scripts: EtherCAT and Modbus TCP active concurrently on `ecat-host`, no external Modbus↔EtherCAT signal connection. |
| `specification/Architecture/IO-Routing-and-Backends.md` | Modbus backend contract: zero-based addressing, mapping, scheduling, fault policy, status layout, codes; registered kinds `modbus-tcp`/`modbus-tcp-status` (replacing the provisional `modbus` spelling in §4.1); the runtime-override and control-status wording gaps. |
| `specification/Architecture/IO-and-Devices.md` | Move Modbus from “planned” to the v2.0.0 backend contract; link the details. |
| `specification/Architecture/CLI-and-Manifest.md` | Profile schema, zero-based address convention, and any documentation correction for the override gap. |
| `specification/Verification/Modbus-Server.md` | New evidence report: zero-based addressing, environment, method, function/error coverage, cross-host results, overhead, limits. |
| `specification/Decisions.md` | Record accepted M-1–M-15 as D-229 onward. |
| `KNOWN-LIMITATIONS.md` | Reword the “Modbus is v2 work” item; add non-RT, no-auth, boolean-on-bit-tables, zero-based addressing, width/span, byte/word-order, security, and evidence boundaries. |
| `README.md`, `USER-GUIDE.md` | Short Modbus server section and profile example. |
| `V2.0.0-PLAN.md`, `FutureWishlist.md` | Status updates when phases complete; final WL-5.1 completion record. |
| `WL-5.1-Plan.md` | Phase status, decisions, and change log during implementation. |

### 7.3 Sequencing and owners

- Phase 0 was the planning gate: the owner resolved §12 (Q-1–Q-22) and accepted M-1–M-15 on
  2026-11-02. No runtime code started before that gate.
- Phases 1–2 are one implementation stream in `stc-runtime`; phase 2 depends on phase 1.
- Phase 3 depends on phase 2 and runs on every host, not only Linux.
- Phase 4 depends on phase 3; the fixture and CI work can begin in parallel with the late phase-3
  error matrix.
- Phase 5 depends on phase 4's results and includes the documentation consolidation and PR
  described in §7.4.
- Phase owners and handovers follow §7.4. The implementation subagent owns the `io_modbus_tcp`
  module and its tests; the documentation work in phase 5 is owned by the phase-5 subagent, with
  the orchestrator retaining decision and status authority.

### 7.4 Execution and handover protocol

Phase 0 ended on 2026-11-02 when the owner resolved every question in §12 (Q-1–Q-22) and accepted
M-1–M-15. No implementation phase starts before that gate.

**Phase 0 outcome (2026-11-02).** The owner accepted M-1–M-15 and resolved Q-1–Q-22. The profile
schema (bind array, zero-based addressing, 1/8/16/32/64-bit widths and register spans, `byte`
placement, device and per-channel word order with named forms and index lists), the protocol
surface, the scheduling/fault/status/diagnostics contracts, the phase plan, the file-change list,
the test matrix, and the evidence plan were reviewed for consistency; no implementation code was
started. The contracts are locked for the phase-1 handover; the orchestrator records the commit
and CI in §7.1 at handover.

After the gate:

- **Orchestrator (this agent):** updates §7.1 with the completed phase's commit and CI, then hands
  exactly one phase to a subagent. It does not start the next phase until the previous phase's CI
  is green and the status row is updated.
- **Phase subagent:** reads this plan and the linked specification and code; implements only the
  assigned phase; updates that phase's deliverables; commits, pushes, and runs CI; reports the
  commit and CI run. It stops and returns control to the orchestrator when (a) an owner decision is
  needed, or (b) CI fails for the second consecutive time. A subagent may fix a failing CI run
  once; a second consecutive failure is a stop condition, not a retry.
- **Owner:** resolves a returned question; the orchestrator records the answer in this plan and,
  when accepted, in [Decisions](specification/Decisions.md), then re-hands the phase.
- **No phase skipping.** No subagent edits §5 decisions, §7.1 status, or the scope of another
  phase. Status and decision changes go through the orchestrator.
- **Completion:** every phase ends with commit, push, green CI, and a §7.1 row update. Phase 5
  ends with the consolidation and PR actions below.

**Stop conditions for a phase subagent**

- A question appears that only the owner can resolve.
- The work would change a decision in §5 or the scope of another phase.
- CI fails for the second consecutive time. The subagent reports the failing run and does not
  start a third attempt.

**Phase 5 cleanup and PR**

- Move permanent content into `specification/`: the Modbus backend contract in
  [IO-Routing-and-Backends](specification/Architecture/IO-Routing-and-Backends.md), the profile and
  CLI wording in [CLI-and-Manifest](specification/Architecture/CLI-and-Manifest.md), the backend
  status in [IO-and-Devices](specification/Architecture/IO-and-Devices.md), the report in
  `specification/Verification/Modbus-Server.md`, the decisions in
  [Decisions](specification/Decisions.md), and the limits in
  [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md).
- Remove stale and working-only content from this plan. The plan is a working document, not the
  permanent contract.
- Update [V2.0.0-PLAN.md](V2.0.0-PLAN.md) with the implemented work and the final WL-5.1 status;
  update [FutureWishlist.md](FutureWishlist.md).
- Open a pull request from `WL-5.1` into `v2.0.0-planning` containing the implementation and the
  final plan state. The PR must be green and owner-approved before merge.
- After merge, retire the working plan as WL-5.2's was: its permanent content is the
  specification, Decisions, and the parent plan. The plan file is not a permanent duplicate.

## 8. Test matrix

| Area | Environment | Cases | Evidence |
| --- | --- | --- | --- |
| MBAP/PDU codec | CI, all hosts | Header fields, transaction echo, protocol id, length, unit id, max ADU, truncated frames, pipelining | Unit and property tests |
| Function codes | CI, all hosts | FC1/2/3/4/5/6/15/16 success, limits, response packing; FC6 rejected on part of a 32/64-bit channel; FC16 accepted only for whole-channel spans | Unit tests with golden bytes |
| Exceptions | CI, all hosts | Unsupported function, unmapped address, read-only address, illegal quantity, bad byte count, bad coil value, invalid unit, partial 32/64-bit write | Unit tests and test-client integration |
| Table/direction mapping | CI, all hosts | Every table/direction combination; 1/8/16/32/64-bit widths; register spans; low/high 8-bit pair, including a mixed input/output pair (input byte updated, output byte preserved); overlapping spans; undeclared/missing channel; byte-aligned image range | Unit tests with golden diagnostics |
| Zero-based addressing | CI and docs review | `address = 0` ↔ `00001`/`10001`/`30001`/`40001`; `address = 1` ↔ `00002`/`10002`/`30002`/`40002`; `address = 1024` ↔ `01025`/`11025`/`31025`/`41025`; 32/64-bit spans; no off-by-one in the PDU or diagnostics | Golden-vector tests and documentation review |
| Byte order | CI, all hosts | 8-bit low/high; 16-bit big-endian; device-level high-first/low-first (including 64-bit `low-first` GHEFCDAB versus per-channel `cdab` CDABGHEF); per-channel named and index-list permutations; 0x0000, 0x0001, 0x00FF, 0x0100, 0x1234, 0xFFFF, 64-bit patterns; coil bit packing LSB-first | Golden-vector tests |
| Cycle-boundary visibility | CI, all hosts | Write before latch visible in the scan; write after latch visible next scan; output visible after publish; no socket work on the cycle thread | Integration tests with a real loopback client |
| Atomicity and concurrency | CI, all hosts | FC15/FC16 all-or-none under a racing latch; whole-span 32/64-bit writes; multiple clients last-writer-wins; no torn values | Stress and integration tests |
| Lifecycle | CI, all hosts | Cold start, any bind failure in a multi-bind server, duplicate bind entries, shutdown, drop without shutdown, trap/stop, restart, partial cold-start teardown | Integration and process tests |
| Connections | CI, all hosts | Connection limit counted across binds, idle timeout, partial frame, disconnect, reconnect, write failure, slow client | Integration tests |
| Multiple binds | CI, all hosts | Two or three loopback binds on one server, all serving one bank; per-listener state; one listener fault leaves the others serving; duplicate concrete addresses rejected; two port-0 entries get distinct ports | Integration tests |
| Multiple servers | CI, all hosts | Two `modbus-tcp` devices, each with one or more binds, distinct addresses, per-server status indexing, one server fault does not affect the other | Integration tests |
| Status device | CI, all hosts | Layout 1, unknown layout/channel, width/direction errors, bound and unbound fields, listener/bound counts, counters update, no data device, multiple server indexes | Unit and integration tests |
| Diagnostics and CLI | CI, all hosts | `stc run -v` output, `server<N>.*` and `server<N>.listener<M>.*` keys, last fault, config hash | Project CLI tests |
| Coordinator/multi-backend | CI, all hosts | Modbus + mock EtherCAT over one image; declaration order; fault isolation; cross-backend routes | Rust coordinator tests |
| Real mixed bench | `ecat-host`, release package | EtherCAT (DIEWU) and Modbus TCP active concurrently; both backends report healthy; no external Modbus↔EtherCAT signal connection; the cross-wiring limitation is recorded | Bench logs, status output, report |
| Online change | CI, all hosts | Code-only swap preserves listeners/connections; bind/word-order/byte/span/channel change returns cold-required; counter continuity | Host online-change tests |
| Profile/manifest | CI, all hosts | Missing/duplicate keys, bad types, bad bind array (empty, non-string, malformed address, duplicate), bad unit/limits, bad word-order/byte, unknown table, zero-based address outside 0..=65535, overlapping/out-of-range spans, inline/profile precedence, missing mapping, status device with a profile entry | Manifest and project tests with golden diagnostics |
| Cross-host end-to-end | Windows x86-64, Linux x86-64, Linux AArch64 CI | `stc run` fixture; multi-bind server; real TCP client writes all input tables and reads all output tables using zero-based addresses, including 8/16/32/64-bit values and device-default/per-channel word orders; clean stop | CI logs and fixture scripts |
| Overhead | Release build, representative hosts | Latch/publish p50/p99/max with 0/1/4/8 clients; request latency/throughput; scan impact | Harness output and report |
| Security/negative | CI and review | Write to output, invalid unit, oversized frame, partial write, connection flood bounded by limit, multi-interface exposure documented, no-auth documented | Tests and documentation review |

## 9. Evidence and deliverables

- **Code:** `io_modbus_tcp.rs` with the codec, bank, 1/8/16/32/64-bit mapping and spans, device
  and per-channel word order, multiple listeners, status, diagnostics, and tests; the registration
  in `backend_defaults.rs`; the profile parser extension.
- **Fixture:** `tests/modbus/` with an ST program, `stc.toml`, selected profile with a multi-address
  `bind` array, a small client script for manual/host runs, run scripts, and a README that explains
  the zero-based address convention, register spans, byte/word order, and table/direction
  semantics. The reusable Rust test client lives with the runtime integration tests and is what CI
  drives.
- **Cross-host evidence:** CI runs on Windows x86-64, Linux x86-64, and Linux AArch64 for the
  functional, error-path, project, and online-change suites.
- **Mixed bench evidence:** a real run on `ecat-host` with the DIEWU EtherCAT slave and a Modbus
  TCP server active concurrently. It does not connect Modbus signals to EtherCAT signals (no
  device is available for that); cross-backend routing is proven by the mixed mock test, and the
  limitation is recorded.
- **Overhead evidence:** a harness and report with the environment, method, distributions, and
  limits. Raw output may be retained with the run; summaries are committed.
- **Report:** `specification/Verification/Modbus-Server.md` covering zero-based addressing and the
  vendor-reference conversion, configuration, multi-address binds, mapping and register spans,
  device and per-channel word order, the mixed real bench and its no-cross-wiring limitation,
  scheduling, protocol surface, error behavior, cross-host results, overhead, security boundary,
  and what the evidence does not prove.
- **Specification updates:** the Modbus contract in IO-Routing-and-Backends, the backend status in
  IO-and-Devices, the profile schema in CLI-and-Manifest, and the limitation entries.
- **Decisions:** M-1–M-15 are recorded as D-229 onward in phase 5.
- **PR and cleanup:** permanent content consolidated in `specification/`, stale working content
  removed, `V2.0.0-PLAN.md` updated, and a `WL-5.1` pull request into `v2.0.0-planning`; the
  working plan is retired after merge.
- **No RT or safety claim.** The report states the non-real-time, best-effort, unauthenticated
  boundary.

## 10. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Network threads perturb the PLC scan on a loaded or RT host | Keep all socket work off the cycle thread; take the bank lock only for bounded copies; measure latch/publish with 0/1/4/8 clients; document that Modbus is non-RT and must not be used for motion |
| A failed cold start or swap leaks a listener or connection thread | Bind every entry before spawning; tear down partial starts; make `shutdown` idempotent and call it from `Drop`; test multi-bind failure, swap, and drop |
| Port collisions or privileged ports break tests or deployment | Require explicit bind entries; use port 0 and diagnostics in tests; use high ports except the documented privileged 567/502 examples; document the privilege requirement |
| A slow or hostile client consumes resources | Per-server connection limit across binds, idle timeout, bounded ADU, per-connection isolation, accept-and-close at the limit; no unbounded buffers |
| Unauthenticated clients can write PLC inputs on any bound interface | Explicit bind arrays, trusted/segmented network guidance, documentation that the image is writable; no safety claim |
| Multiple binds multiply the exposed attack surface | Require every entry explicitly, show the effective list in diagnostics, and document that all binds share one writable bank |
| The real mixed bench cannot prove physical Modbus↔EtherCAT cross-wiring | No external connection device is available; the mixed mock test proves cross-backend routing, and the bench limitation is recorded in the report and known limitations |
| Ambiguous mapping semantics cause silent wrong values | Fixed table/direction/width/span matrix; strict exception on unmapped, read-only, and partial wider writes; all-or-none multi-writes; golden tests |
| Vendor documentation uses one-based reference numbers (`00001`/`40001`) while the profile uses zero-based addresses, causing an off-by-one access to the wrong element | Fix the zero-based convention in the plan, profile comments, specification, report, and known limitations; test `address` 0/1/1024 against vendor notation; review every example for "address" versus "reference number" |
| Byte or word order is wrong for wider values | Test 8-bit low/high, 16-bit big-endian, device defaults, and per-channel permutation orders with 0x0001/0x0100/0x1234/0xFFFF and 64-bit patterns on every host; document the word-order boundary |
| Third-party clients behave differently from the test client | Implement standard function codes and framing; keep the test client protocol-faithful; run an optional independent client check and record any interop caveat |
| Online change drops live connections unexpectedly | Preserve the backend on an unchanged hash; test connection continuity; document that bind/map changes require a cold download |
| Cross-host CI localhost networking is flaky | Bind loopback only; use port 0/free ports; set client timeouts; keep tests single-purpose; the existing matrix already exercises all three hosts, and the new tests add only loopback TCP |
| Scope creep into client, RTU, TLS, or arbitrary register layouts | Locked scope in §2; WL-11 and transport security remain separate; the width, span, byte, and word-order boundaries are in known limitations |
| Documentation drifts from the implementation | Phase 5 reconciles the I/O contract, CLI/manifest, known limitations, README, user guide, decisions, and report; a traceability check lists every new code and channel |
| Overhead measurements are noisy or over-interpreted | Release build, repeated runs, report distributions and environment, state clearly that results are host-specific and not a real-time guarantee |

## 11. Exit criteria

- The Modbus TCP server backend works on Windows x86-64 MSVC, Linux x86-64 GNU, and Linux
  AArch64 GNU; the existing CI matrix is green.
- Coils, discrete inputs, holding registers, and input registers are mapped through the selected
  profile; the table/direction/width/span and byte/word-order rules (including per-channel
  permutation overrides) and the 1/8/16/32/64-bit widths are enforced and tested.
- Addressing is explicitly zero-based: the first element of every table is address 0; tests pin
  address 0/1/1024 against vendor `00001`/`10001`/`30001`/`40001` notation; every document labels
  one-based values as vendor reference numbers, never as `address`.
- FC1/2/3/4/5/6/15/16 and the standard exception behavior are covered by the simulator and
  error-path tests, including partial 32/64-bit write rejection.
- Simulator tests cover reads, writes, malformed requests, unsupported functions, illegal
  addresses, disconnects, reconnects, timeouts, and the connection limit across binds.
- A server can bind multiple addresses from a TOML array; all binds serve one register bank and
  mapping; any bind failure at cold start fails the server; one listener can fail and recover
  while the others continue; status shows listener and bound counts.
- Request handling cannot silently corrupt process-image values: multi-address writes are atomic,
  writes to read-only addresses return an exception, unmapped ranges fail closed, partial wider
  writes are rejected, and last-writer-wins is documented.
- Client writes are visible to the PLC at the next input boundary; PLC outputs are visible to
  clients after the publish boundary; no socket work runs on the cycle thread.
- A Modbus project with an EtherCAT mock runs over one process image; per-backend fault isolation,
  aggregate status, and cross-backend routes are demonstrated by the mixed mock test.
- A real mixed bench run on `ecat-host` shows the DIEWU EtherCAT slave and Modbus TCP active
  concurrently; no external Modbus↔EtherCAT signal connection is available, and that limitation is
  recorded.
- The `modbus-tcp-status` device layout and the verbose status diagnostics are implemented, validated,
  and documented; unknown layouts/channels are rejected.
- Online-change behavior is tested: a code-only swap preserves the server and connections; a
  configuration change returns `STC-E-ONLINE-IO-001`.
- Cycle-boundary overhead is measured and reported; no hard real-time or safety claim is made.
- The I/O specification, IO-and-Devices, CLI-and-Manifest, KNOWN-LIMITATIONS, README, USER-GUIDE,
  decisions, and the evidence report agree with the implementation.
- Permanent documentation is consolidated in `specification/`; stale and working-only content is
  removed from this plan; `V2.0.0-PLAN.md` records the implemented work; the `WL-5.1` pull request
  into `v2.0.0-planning` is green and owner-approved; after merge the working plan is retired.
- The owner reviews and accepts the phase-5 result.

## 12. Owner questions (resolved 2026-11-02)

| ID | Question | Resolution |
| --- | --- | --- |
| Q-1 | Is the explicit profile channel map accepted instead of deriving addresses from image offsets? | Accepted 2026-11-02: explicit map, M-2. |
| Q-2 | Should reads of input-mapped coils/registers return the last client-written value? | Accepted 2026-11-02: the bank value is the PLC input after the latch, M-3. |
| Q-3 | Should an unmapped address in a read/write range fail the whole request? | Owner answer 2026-11-02: yes; exception 0x02, no partial data, M-3. |
| Q-4 | Is `unit` optional and “accept any” when absent? | Owner answer 2026-11-02: yes; enforce 1..=247 only when configured, M-4/M-6. |
| Q-5 | Is `bind` a required non-empty TOML array with no wildcard default? | Accepted 2026-11-02: one listener per entry, all sharing the bank, M-4/M-8. |
| Q-6 | Is the `modbus-tcp-status` device optional? | Owner answer 2026-11-02: yes; verbose status diagnostics always exist, M-10. |
| Q-7 | Are 8/16/32/64-bit values and multi-register spans required in v2.0.0? | Accepted 2026-11-02: 1/8/16/32/64-bit channels, 32/64 spanning 2/4 registers, M-3/M-7. |
| Q-8 | Is a new global `[io].backend = "modbus-tcp"` spelling required? | Owner answer 2026-11-02: no; device `kind` only, M-13. |
| Q-9 | Is a runtime bind override required? | Owner answer 2026-11-02: no; the profile is authoritative, tests use port 0/free ports, M-15. |
| Q-10 | What evidence is required for EtherCAT and Modbus TCP in the same run? | Owner answer 2026-11-02: mixed mock test required; a real mixed bench run with both backends active, but no external Modbus↔EtherCAT signal connection is available. Cross-backend routing is proven by the mock test; the bench limitation is recorded, M-14. |
| Q-11 | Is an independent third-party Modbus client check required? | Accepted 2026-11-02: optional; record the client and result if run, M-14. |
| Q-12 | Should `max-connections` default to 8 and `idle-timeout` to 60 s? | Accepted 2026-11-02: `max-connections` is total across the server's binds, M-4/M-8. |
| Q-13 | Should a write to an output-mapped coil/register return 0x02? | Accepted 2026-11-02: yes; illegal data address, M-3. |
| Q-14 | Should counters saturate rather than wrap? | Accepted 2026-11-02: yes; UDINT status counters saturate, M-10. |
| Q-15 | Should port 0 be accepted for tests and embedded use? | Accepted 2026-11-02: yes; the effective list is reported in diagnostics, M-4. |
| Q-16 | Should any bind failure at cold start fail the whole server? | Owner answer 2026-11-02: yes, all-or-nothing; no partial bind set is left running, M-9. |
| Q-17 | Should 8-bit channels select low/high bytes and be allowed to share a register? | Accepted 2026-11-02: one low/high pair per register, no other overlap, M-3/M-7. |
| Q-18 | What word-order support is required for 32/64-bit channels? | Owner answer 2026-11-02: device-level default plus per-channel named `abcd`/`cdab`/`dcba`/`badc` or explicit byte-index permutation; vocabulary locked in M-7/§6.3. |
| Q-19 | Should `BOOL` be allowed on a register table? | Owner answer 2026-11-02: no; BOOL stays on bit tables, use a 16-bit channel for a 0/1 register value, M-3. |
| Q-20 | Should partial writes of a 32/64-bit channel be rejected? | Owner answer 2026-11-02: yes; 0x02, and FC16 must cover the whole span, M-3. |
| Q-21 | Should `server<N>.listening` mean every configured listener is accepting? | Accepted 2026-11-02: yes; `bound_listeners` reports a partial state, M-9/M-10. |
| Q-22 | Should one failed listener mark the backend `Faulted` while the other listeners keep serving? | Accepted 2026-11-02: yes; aggregate state is faulted, per-listener status shows which, M-9/M-10. |

## 13. Change log

| Date | Change |
| --- | --- |
| 2026-11-01 | Created the WL-5.1 working plan on the post-WL-5.2 baseline. Proposed the explicit profile channel map, table/direction semantics, dedicated-thread scheduling, protocol surface, fault policy, status device, online-change behavior, and evidence plan; no implementation started. |
| 2026-11-02 | Made the transport explicit in the identifiers: `modbus-tcp` and `modbus-tcp-status` kinds, `io_modbus_tcp` module, `ModbusTcpBackend`, `register_modbus_tcp`, and `STC-*-IO-MODTCP-*` diagnostic codes. The kind spelling leaves room for Modbus RTU and the WL-11 Modbus TCP client. |
| 2026-11-02 | Added multi-address binding as a non-empty TOML `bind` array (one listener per entry, one shared bank), 8-, 32-, and 64-bit register channels in addition to 16-bit (with 1/2/4-register spans), low/high byte placement for 8-bit channels, device-level `word-order`, rejection of partial wider writes, and listener-aware status and diagnostics. |
| 2026-11-02 | Documented zero-based addressing explicitly: the first coil, discrete input, input register, and holding register all have `address` 0; vendor `00001`/`10001`/`30001`/`40001` reference numbers are one-based and convert by subtracting one. Profile examples, diagnostics, tests, the report, and known limitations must use and label the convention. |
| 2026-11-02 | Recorded the execution protocol: after the owner resolves the open questions, phases are handed to subagents one at a time; each ends with commit, push, and green CI; the orchestrator updates §7.1 before the next handover. A subagent stops on an owner question or a second consecutive CI failure. Phase 5 consolidates permanent documentation, removes stale working content, updates V2.0.0-PLAN.md, and opens the WL-5.1 → v2.0.0-planning PR. |
| 2026-11-02 | Recorded owner answers: BOOL on bit tables only; partial 32/64-bit writes rejected; word order is a device-level default with a per-channel named ABCD/CDAB/DCBA/BADC or explicit index-list override; `modbus-tcp` is kind-only, with no global backend spelling; cold-start binds are all-or-nothing; the mixed mock test is required and a real mixed bench run is planned with both backends active but no external Modbus↔EtherCAT signal connection; the status device is optional; `unit` is optional with accept-any when absent; unmapped ranges fail the whole request; there is no runtime bind override. |
| 2026-11-02 | All owner questions resolved; M-1–M-15 accepted. Phase 0 handed to a subagent under §7.4; the orchestrator will update §7.1 with the phase commit and CI before handing phase 1. |
| 2026-11-02 | Phase 0 complete: commit `3547e58`, CI run 41473961570 success. The contracts are locked; phase 1 (protocol core and register bank) handed to a subagent. |
| 2026-11-02 | Phase 1 complete: commit `ebfbbc6`, CI run 41478485216 success. The protocol core and register bank have 44 new tests; phase 2 (backend and coordinator integration) handed to a subagent. |
| 2026-11-02 | Phase 2 complete: commit `6c55414`, CI run 41483247583 success. The backend, registration, profile validation, loopback integration, and mixed mock-EtherCAT tests are in place; phase 3 (error paths, lifecycle, online change) handed to a subagent. |
| 2026-11-02 | Phase 3 complete: commit `97c592d`, CI run 41488899175 success. Error/lifecycle and host online-change tests are in place; phase 4 (cross-host end-to-end and real mixed bench) handed to a subagent. |
| 2026-11-02 | Phase 0 contracts gate closed. Verified M-1–M-15 and Q-1–Q-22 against the profile schema, protocol surface, scheduling/fault/status/diagnostics contracts, phase plan, file-change list, test matrix, and evidence plan. Clarified the word-order/code-validation boundary, the 8-bit mixed input/output write rule, the aggregate fault state during a listener retry, the profile-only Modbus data-device configuration path, and the mixed-bench evidence status; no implementation code was started. |
