# WL-15 Plan — MQTT client for process-image I/O

Status: Phase 0 is closed and the plan was approved by the owner on 2026-11-13. Phase 1 (MQTT
5.0 packet codec, payload encodings, topic rules, and session state machine) is implemented and
locally green; the phase commit and CI run are pending. This is a temporary working document; it
is deleted at closeout.

- Date: 2026-11-13
- Branch: `WL-15`, branched from `v3.0.0-planning` at `3aaf898`
- Parent planning item: [v3.0.0-plan.md](v3.0.0-plan.md) §4 WL-15 (locked 2026-11-04)
- Wishlist entry: [FutureWishlist.md](FutureWishlist.md) WL-15 (status `Locked`)
- Target release: v3.0.0, after the delivered WL-1.2, WL-14, WL-16, WL-17, and WL-18
- Profile: `stc-2013-oop-utf8-v1`

## 1. Goal

Add an MQTT **client** I/O backend that connects to an external broker and maps topic/payload
traffic to the `stc` process image, as a third hardware backend beside the delivered EtherCAT and
Modbus TCP backends. The backend is additive: it adds the `mqtt` and `mqtt-status` device kinds,
one machine-profile device schema, one `DeviceBackend` implementation, diagnostics, tests, and
documentation. It does not change the language, the compiler's protocol neutrality, the
`DeviceBackend` trait, the coordinator cycle order, the route graph, or the host ABI.

The broker is external and out of scope. The base package gains no broker and no broker
dependency.

## 2. Locked scope

Taken from [v3.0.0-plan.md](v3.0.0-plan.md) §4 WL-15; not editable without an owner re-plan.

- Client role only: connect to one or more configured MQTT brokers; no broker, MQTT server,
  MQTT-SN, or cloud-specific connector.
- Map channels to topics and payloads in the machine profile: subscription topics for inputs,
  publish topics for outputs, payload encoding, retain and last-will, QoS, keepalive, reconnect
  and backoff, and stale/cache behavior.
- Integrate as an `mqtt`/`mqtt-status` kind through `DeviceBackend`; connection lifecycle,
  per-backend fault/status, and optional PLC-visible status fields.
- Support broker authentication and TLS where the broker requires it; document the trust
  boundary and credential handling.
- Provide a deterministic local broker or simulator for tests; do not make the base package
  depend on a broker.
- Record cross-host tests, reconnect and malformed-payload tests, and cycle-overhead
  measurements. No hard-real-time or safety claim.

## 3. Exclusions

- Implementing a broker, MQTT server, MQTT-SN, or a cloud connector (AWS IoT, Azure IoT, HiveMQ
  extensions, and similar). The test broker is a test fixture, not a product surface.
- Hard-real-time behavior, motion over MQTT, or any timing guarantee. MQTT is asynchronous and
  non-real-time by construction.
- Changing the process-image model, the device-binding model, the route graph, the coordinator
  cycle order, or the `DeviceBackend` trait beyond registering two more kinds.
- New core language or compiler features. The compiler keeps validating manifest/profile
  structure only and stays protocol-agnostic; it does not parse MQTT packets or topics.
- Making a broker a prerequisite of the driver, the runtime artifact, the CI matrix, or the
  release package.
- WL-11 (Modbus TCP client), WL-7 (ESI), WL-6 (motion), WL-8/WL-9 (UI). None is a prerequisite.
- A bundled CA store, a system trust-store integration, or client-certificate provisioning
  tooling. TLS material is supplied by the user.

## 4. Starting point: the contracts this backend must fit

These are delivered and are treated as fixed inputs, not open design:

- **Coordinator and backend interface.** `crates/stc-runtime/src/coordinator.rs` owns one shared
  process image, resolves each device `kind` to a registered instance, groups bindings per
  device, and calls `cold_start`, `latch_inputs`, `publish_outputs`, `status`, `diagnostics`, and
  `shutdown` in the documented order
  ([I/O Routing and Backends](specification/Architecture/IO-Routing-and-Backends.md) §4.2–§4.3).
  No protocol type crosses the trait; device configuration is opaque JSON text interpreted by its
  backend.
- **Kind registration.** `crates/stc-runtime/src/backend_defaults.rs` registers the default
  registry. `mqtt` and `mqtt-status` will register as one instance named `mqtt`, mirroring
  `modbus-tcp`/`modbus-tcp-status`. A `-status` device has no profile entry and read-only `%I`
  channels only.
- **Profile model.** `crates/stc-manifest/src/profile.rs` parses `[device.<name>]` tables into an
  opaque `ConfigValue` tree plus `[[routes]]`; the runtime backend validates semantics and raises
  its own `STC-E-IO-*` codes. A `mqtt` device rejects EtherCAT and Modbus keys explicitly rather
  than ignoring them.
- **Configuration hash and online change.** The coordinator hashes the canonical profile digest,
  device tree, kinds/configs/channels, bindings, and routes. A changed MQTT mapping requires a
  cold download with `STC-E-ONLINE-IO-001`; a code-only or same-configuration swap keeps the live
  session. MQTT follows the Modbus precedent ([§7](specification/Architecture/IO-Routing-and-Backends.md)).
- **Evidence and diagnostic conventions.** Stable `STC-E-IO-<PROTO>-00N` codes, `stc run -v`
  per-backend diagnostic keys, a verification report under `specification/Verification/`, and
  decisions in `specification/Decisions.md` (next `D-254`).
- **Test conventions.** `crates/stc-runtime/tests/*` for backend behavior on every host,
  `crates/stc-driver/tests/*` for CLI/end-to-end fixtures, loopback-only networking, ephemeral
  ports, and no external service in the base test path.

## 5. Interface design (proposed; confirmed in Phase 0)

### 5.1 Kinds and device model

- `kind = "mqtt"` — a data device. One backend instance serves every MQTT data and status device
  in the project, so all devices share one client session per broker connection set.
- `kind = "mqtt-status"` — optional read-only status device, `config = { layout = 1 }`.
- No `[io].backend = "mqtt"` and no `--io mqtt`: the kind selects the backend, and unbound image
  areas keep the null behavior (the Modbus precedent, D-241).
- A data device has no inline addressing; its configuration is the selected machine profile and
  every declared channel must map exactly once.

### 5.2 Profile schema (proposed keys)

```toml
[device.broker0]
host = "broker.example"          # hostname or literal address; resolved off the cycle thread
port = 1883                      # or 8883 with tls
client-id = "line1-plc"         # omitted: broker-assigned, persisted per device
keepalive = "30s"
clean-start = true               # MQTT 5 Clean Start
session-expiry = "0s"            # MQTT 5 Session Expiry Interval
connect-timeout = "5s"
reconnect-backoff = ["250ms", "500ms", "1s", "2s", "5s"]   # capped ladder, last value repeats
stale-timeout = "5s"             # input staleness after the last accepted message
qos = 1                          # default for channels that omit qos
retain = false                   # default for publish channels
# TLS is MQTT 5.0 over the approved `rustls` dependency (Q3); no bundled trust store.
tls = { ca-file = "io/ca.pem", server-name = "broker.example", verify = true }
# Optional client-certificate authentication:
# client-cert-file = "io/client.pem", client-key-file = "io/client.key"
credentials = { username-file = "io/mqtt-user", password-file = "io/mqtt-password" }
last-will = { topic = "line1/plc/status", payload-encoding = "utf8", payload = "offline", qos = 1, retain = true }

[[device.broker0.channels]]
name = "CmdStart"
direction = "input"
topic = "line1/plc/cmd/start"
payload-encoding = "bool"        # bool | uint | int | float | string | raw
qos = 1

[[device.broker0.channels]]
name = "SpeedSetpoint"
direction = "output"
topic = "line1/plc/speed"
payload-encoding = "float"
retain = true
```

Rules to be fixed in Phase 0/2: channel `direction` must agree with the declared channel; each
channel maps one topic and one encoding, width-checked against the declaration; no wildcard in a
publish topic; subscription topics may use `+`/`#` and `$share/<group>/`; a topic may carry at
most one input channel and at most one output channel per instance unless Phase 0 decides
otherwise; `tls` and `credentials` may be inline or file references but never inline secrets.

The wire version is MQTT 5.0 only (Q1). The client encodes and validates the v5 property set it
supports — session expiry, receive maximum, maximum packet size, topic-alias rejection, reason
strings, and user properties — and rejects unknown or out-of-range properties rather than
silently ignoring them. Broker reason codes are surfaced in diagnostics and the verbose status.
Topic aliases are not used in either direction, and request/response metadata (response topic,
correlation data) and subscription identifiers are not surfaced; a profile that would need them
is rejected rather than silently dropped. `$share/<group>/` subscriptions are accepted (Q13).

### 5.3 Runtime shape

- **Cold start validates, never blocks on the network.** It parses and validates every device and
  channel, checks image ranges, resolves nothing over DNS, starts one session thread per broker,
  and returns ready. An unreachable broker is a fault state, not a fatal startup error.
- **The cycle thread never does I/O.** `latch_inputs` copies the instance's latest decoded input
  bank under a short mutex; `publish_outputs` copies bound `%Q` ranges into the publish bank under
  the same mutex. Encoding, socket reads/writes, TLS, DNS, reconnect, and backoff all run on the
  session thread.
- **Input staleness.** Each input channel carries a last-accepted value and a receipt stamp. After
  `stale-timeout` without a message, the channel keeps its last value (cache policy) and the
  backend sets `input.stale`; it never substitutes zero and never stalls the scan.
- **Faults are non-fatal and per backend.** A disconnect, protocol error, malformed payload, or
  TLS failure changes state and counters, keeps publishing the PLC's output values unchanged, and
  retries with the configured ladder. The run does not stop.
- **Shutdown** stops the session, sends a clean DISCONNECT when connected, joins the thread, and
  is idempotent and `Drop`-safe.

### 5.4 Status device and diagnostics (proposed layout 1)

`<N>` is the zero-based index of the configured MQTT data device in declaration order:
`client<N>.connected` (`BOOL`), `client<N>.state` (`UINT`), `client<N>.reconnects` (`UDINT`),
`client<N>.messages_in`/`messages_out` (`UDINT`), `client<N>.dropped` (`UDINT`),
`client<N>.last_error` (`UDINT`), `broker<N>.rtt` or a keepalive-derived age (`UDINT`),
`input.stale` (`BOOL`), and `cycle.exchanges`/`cycle.faults` (`UDINT`). The exact set is fixed in
Phase 4; unbound fields cost no image bytes. `stc run -v` prints one `io-backend mqtt` line with
the same counters; the control `Status` JSON stays unchanged, matching v2.0.0.

### 5.5 Security stance (proposed)

- Credentials and TLS material live in user files outside source control, referenced by path
  relative to the project root; inline secrets in `stc.toml` or the profile are rejected with a
  diagnostic. Files must be private on Unix and are never echoed in diagnostics, `stc run -v`,
  or logs.
- TLS is opt-in per device; when enabled, the default is certificate and hostname verification
  against the supplied CA, never `verify = false` silently. Implementation uses the approved
  `rustls` stack (Q3) with a user-supplied CA file and no bundled or system trust store. A
  documented, explicit `verify = false` escape hatch may be offered only with a warning code.
- The threat model is a short section of the backend contract, not a new standalone document:
  the broker is a network peer, MQTT payloads are untrusted field input, and the backend is not a
  safety function and provides no fail-safe output state. It shares the WL-1.2 secret-handling
  stance, not its SSH transport.

## 6. Phases

Each phase follows [Feature Development Workflow](workflows/FEATURE-DEVELOPMENT.md) §4–§6: one
subagent handoff, local checks, pushed commit, green CI, recorded acceptance. Acceptance is
automatic for automatable gates; Phase 4/6 manual surfaces require owner acceptance.

### Phase 0 — Close owner questions

Goal: no open question blocks implementation, and every durable answer is a decision.

Work:

- Ask the owner the questions in §7 and record the answers as `D-254` and following in
  [Decisions](specification/Decisions.md).
- Fix the mapping schema, payload encodings, QoS/retain/last-will and clean-start/session-expiry
  defaults, stale
  policy, credential and TLS file rules, diagnostic codes, status layout, and the test-broker
  strategy in this plan.

Exit criteria: every §7 question has an owner answer; the plan is approved; no implementation
commit exists on the branch before approval.

Evidence: decisions `D-254`–`D-25N`, this plan updated, owner approval recorded in §9.

### Phase 1 — MQTT protocol core and session state machine

Goal: a host-independent MQTT client core with deterministic tests.

Work:

- Implement MQTT 5.0 packet encode/decode for CONNECT/CONNACK, PUBLISH, PUBACK/PUBREC/PUBREL/
  PUBCOMP, SUBSCRIBE/SUBACK, UNSUBSCRIBE/UNSUBACK, PINGREQ/PINGRESP, and DISCONNECT, including
  the v5 property blocks and reason codes. There is no MQTT 3.1.1 path and no protocol version
  negotiation; a broker that does not accept MQTT 5.0 fails the session with a clear diagnostic.
- Implement the session state machine (connecting, connected, backoff, disconnecting, closed),
  keepalive scheduling, clean-start and session-expiry handling, topic matching for `+`/`#` and
  `$share/` groups, and payload encode/decode for the six accepted scalar/raw encodings with
  width checks. Q14 selected no JSON encoding, so this target adds no JSON codec, no struct
  descriptor, and no compiler involvement.
- Bound every length and remaining-length field; reject malformed, truncated, oversized, and
  unexpected packets deterministically. No panics on untrusted input.
- Unit-test the codec against hand-built frames and the OASIS conformance vectors that apply;
  add the parser to the fuzz targets if Phase 0 chooses a project-owned implementation.

Exit criteria: codec and state machine pass unit tests on every CI host; malformed input is
rejected with an error, never a panic; payload encodings round-trip and reject width mismatches.

Evidence: unit tests, fuzz smoke (if applicable), and the dependency/licence decision record for
the TLS stack (`rustls` plus CA/certificate parsing, Q3).

**Phase 1 status (2026-11-13).** Implemented on `WL-15`:

- `crates/stc-runtime/src/io_mqtt/mod.rs` — module docs, `MqttError`, and `ErrorKind`
  (`incomplete`, `malformed`, `unsupported`, `protocol`, `too-large`, `exhausted`).
- `crates/stc-runtime/src/io_mqtt/packet.rs` — MQTT 5.0 fixed/variable headers, remaining-length
  and variable-integer rules (including the minimal-encoding check), the full property table with
  duplicate and range validation, CONNECT (will, credentials, session expiry, receive maximum,
  maximum packet size), PUBLISH QoS 0/1/2, PUBACK/PUBREC/PUBREL/PUBCOMP, SUBSCRIBE with
  no-local/retain-as-published/retain-handling, UNSUBSCRIBE, CONNACK/SUBACK/UNSUBACK, PINGREQ/
  PINGRESP, and DISCONNECT. AUTH, topic aliases, and server-forbidden client packets are rejected
  with a typed error.
- `crates/stc-runtime/src/io_mqtt/payload.rs` — the six Q4 encodings over a `u64` image bit
  pattern, big-endian default with per-channel byte order, exact-length and width validation.
- `crates/stc-runtime/src/io_mqtt/topic.rs` — topic-name and filter validation, `+`/`#` matching,
  the `$` first-level rule, and `$share/<group>/` shared subscriptions.
- `crates/stc-runtime/src/io_mqtt/session.rs` — idle/connecting/connected/backoff/disconnecting/
  closed machine, CONNECT timeout, keepalive PINGREQ and response timeout, non-fatal backoff
  ladder with a repeating last entry, inbound QoS 1/2 acknowledgements, packet-identifier
  allocation with wrap, and clean shutdown. The session never touches a socket or a clock.
- `fuzz/fuzz_targets/fuzz_mqtt_packet.rs` — decoder, payload, and topic fuzz entry point,
  registered in `fuzz/Cargo.toml`.

Local evidence: `cargo test -p stc-runtime --lib` 416 passed (63 of them new `io_mqtt` tests);
`cargo clippy -p stc-runtime --lib --tests --locked -- -D warnings` clean; `cargo check
--manifest-path fuzz/Cargo.toml --bin fuzz_mqtt_packet` clean. The fuzz target has not been run
under `cargo fuzz` (nightly toolchain); CI fuzz smoke is the remaining gate. No TLS code or
dependency is added in this phase.

### Phase 2 — Profile schema, validation, and mapping

Goal: an `mqtt`/`mqtt-status` device is validated at build and cold start, and every channel maps
to one topic and payload.

Work:

- Extend profile validation so `mqtt` devices accept only the MQTT key set and reject EtherCAT
  and Modbus keys with a dedicated code; keep structural errors as `STC-E-PROJECT-009`.
- Implement runtime semantic validation: channel/topic/encoding/direction/width agreement,
  wildcard and duplicate-topic rules, TLS/credential file existence and privacy, QoS and retain
  ranges, backoff ladder shape, status-device layout and read-only bindings.
- Bind decoded input values into the image and encode published `%Q` values, with 1/8/16/32/64-bit
  mappings per the decided encoding table.
- Add fixtures for valid, invalid, and ambiguous profiles, and a driver-level build test proving a
  malformed profile fails with a clear code and no artifact.

Exit criteria: a valid profile builds and cold-starts; every invalid case fails closed with an
actionable diagnostic and no partial start; the compiler remains protocol-agnostic.

Evidence: `stc-manifest`/codegen tests, runtime config tests, driver build tests, and updated
[CLI and Manifest](specification/Architecture/CLI-and-Manifest.md).

### Phase 3 — Backend through the coordinator

Goal: the backend runs in the delivered cycle without blocking or leaking protocol types.

Work:

- Implement `MqttBackend` in `crates/stc-runtime/src/io_mqtt.rs`, register `mqtt`/`mqtt-status` in
  `backend_defaults.rs`, and keep the coordinator, trait, cycle order, and routes unchanged.
- Split the bank exchange: session thread owns sockets; `latch_inputs`/`publish_outputs` do bounded
  copies under one mutex; no allocation proportional to traffic on the cycle thread.
- Prove coexistence: a mixed Modbus + MQTT (and mock-EtherCAT + MQTT) project latching and
  publishing in one coordinator with per-backend fault isolation, including a faulted MQTT
  backend that leaves the other backend's data untouched.
- Enforce the cold-start rule: unreachable broker starts faulted, never fatal; a failed
  configuration is fatal with no partial start; `shutdown`/`Drop` clean up threads.

Exit criteria: cycle-boundary tests show the scan cannot be stalled by a slow or absent broker;
a broker that never answers leaves the run alive with stale inputs; a second backend is
unaffected by MQTT faults; no protocol type crosses the trait.

Evidence: `crates/stc-runtime/tests/mqtt_backend.rs` (or equivalent), coordinator mixed-backend
tests, and a thread-shutdown test.

### Phase 4 — Lifecycle, TLS credentials, status, and online change

Goal: the operational surface is complete and documented.

Work:

- Reconnect and backoff, stale/cache behavior, keepalive, retained-message handling on subscribe,
  last-will, MQTT 5 clean-start and session-expiry semantics, and clean shutdown (DISCONNECT with
  a normal reason code).
- TLS over the approved `rustls` dependency with a user-supplied CA file and no bundled trust
  store, including optional client-certificate authentication; credential loading from files with
  private-file checks; negative tests for missing, world-readable, or mismatched material; no
  secret in any diagnostic or log.
- Status device layout 1 and the `stc run -v` `io-backend mqtt` line; counters saturate rather
  than wrap.
- Online change: an unchanged hash keeps the client session and counters; any mapping change
  returns `STC-E-ONLINE-IO-001`; a cold download restarts the session.
- Fix the final diagnostic-code table (`STC-E-IO-MQTT-00N`, `STC-W-IO-MQTT-00N`) and record it.

Exit criteria: every lifecycle transition is covered by a test; TLS and credential failures fail
closed with actionable messages and no secret leakage; the status block and verbose line report
the documented fields; online-change behavior matches the Modbus precedent.

Evidence: lifecycle/TLS/credential tests, status and verbose-output tests, online-change tests,
and updated [I/O Routing and Backends](specification/Architecture/IO-Routing-and-Backends.md) and
[I/O and Devices](specification/Architecture/IO-and-Devices.md).

### Phase 5 — Broker simulator, cross-host, and overhead evidence

Goal: deterministic evidence without a product broker dependency.

Work:

- Add a test-only broker simulator (in-process or a test binary under `tests/`) that speaks
  MQTT 5.0 over loopback with ephemeral ports, scripted captures, and fault
  injection (drop, delay, malformed frame, duplicate, retained replay). It is not shipped in the
  release package and is not a supported product surface.
- Add reconnect, malformed-payload, oversized-payload, slow-broker, and partial-failure tests, and
  a cross-host end-to-end fixture run by the existing CI matrix jobs.
- Measure per-cycle backend overhead with the existing `modbus_overhead` harness pattern
  (`crates/stc-runtime/tests/modbus_overhead.rs`) for an idle broker, a busy broker, and a faulted
  broker; record host, OS, build form, metric, and sample. No threshold and no timing gate.
- Add the manual two-host harness (Q11): a small Python MQTT 5 peer host that publishes and
  subscribes, with TLS and optional client-certificate support, plus a Mosquitto configuration
  with TLS on the primary host. It is manual evidence, never CI: no broker, credential, or
  Python dependency enters the workspace build, the test suite, or the release package.
- Run the peer host on a second host against Mosquitto and against the runtime, covering
  subscribe, publish, retain, last-will, reconnect, and TLS; record broker and TLS versions,
  material paths (never contents), and the peer-script revision in the verification report.

Exit criteria: the full test set passes on Windows x86-64, Linux x86-64, Linux AArch64, macOS
ARM64, and macOS x86-64; the two-host Mosquitto TLS run is recorded with commands and output;
overhead numbers are recorded with context; the base package still has no broker dependency.

Evidence: simulator and fault-injection tests, cross-host CI run links, overhead table, and a new
`specification/Verification/MQTT-Client.md` report modeled on
[Modbus-Server.md](specification/Verification/Modbus-Server.md).

### Phase 6 — Documentation, closeout, and manual verification

Goal: the item is complete, documented, and ready for the PR.

Work:

- Update [I/O Routing and Backends](specification/Architecture/IO-Routing-and-Backends.md) (new
  §4.6 backend contract), [I/O and Devices](specification/Architecture/IO-and-Devices.md),
  [CLI and Manifest](specification/Architecture/CLI-and-Manifest.md), [USER-GUIDE.md](USER-GUIDE.md),
  [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md), and [NON-SAFETY.md](NON-SAFETY.md) if the backend
  merits a line.
- Record decisions, the diagnostics table, and the traceability allowlist entries for defined but
  unemitted codes; run `python tools/ci/check_docs.py`, `markdownlint`, traceability, host audit,
  and `git diff --check`.
- Manual walkthrough: run a real project against Mosquitto (Q11) with TLS, supported by the
  Python peer host on a second host; exercise subscribe/publish, retain, last-will, disconnect
  and reconnect, stale behavior, the TLS session, an unsupported-broker-version rejection, and a
  bad-credential failure; capture commands and output.
- Update the WL-15 row, status, retirement checklist, and change log in
  [v3.0.0-plan.md](v3.0.0-plan.md) and [FutureWishlist.md](FutureWishlist.md); open the PR against
  `v3.0.0-planning` after owner approval of the manual evidence.

Exit criteria: documentation and decisions are current; the manual walkthrough is approved; PR
CI is green; merging is left to the owner.

Evidence: docs diff, local check output, manual walkthrough transcript, PR and CI links.

## 7. Owner questions (Phase 0)

Each question carries a proposed default so implementation can proceed once the owner answers.

1. **MQTT version — answered 2026-11-13 (owner).** MQTT 5.0 only. No 3.1.1 path, no version
   negotiation, and no dual-version tests; a broker that rejects MQTT 5.0 is a diagnosed session
   failure.
2. **Implementation — answered 2026-11-13 (owner).** A project-owned minimal client
   (`std::net` + `std::thread`), matching the Modbus precedent D-229. No external MQTT client
   crate and no generated or vendored third-party protocol code; the base package gains no new
   crate for the MQTT protocol itself.
3. **TLS — answered 2026-11-13 (owner).** An external TLS dependency is acceptable; it is the
   only external dependency this target adds. Implementation uses `rustls` plus CA/certificate
   parsing, server and hostname verification against a user-supplied CA file, no bundled or
   system trust store, and optional client-certificate authentication through the same stack.
   Exact crate versions and the `Cargo.lock` delta are fixed in Phase 1.
4. **Payload encodings — answered 2026-11-13 (owner).** Accepted as proposed: the six
   scalar/raw encodings in §5.2, big-endian by default with a per-channel `byte-order`; string
   channels set payload-format-indicator = UTF-8 with an optional content type. JSON is a
   separate question (Q14).
5. **Defaults — answered 2026-11-13 (owner).** Accepted as proposed: QoS 1 by default, retain off
   by default, clean start on with session-expiry 0, 30 s keepalive, and last-will configured only
   when the profile sets it. Per-channel `qos`, `retain`, and `clean-start`/`session-expiry`
   overrides stay explicit in the profile.
6. **Stale and cache policy — answered 2026-11-13 (owner).** Accepted as proposed: keep the last
   accepted value and set `input.stale` after `stale-timeout`; the PLC reads the stale flag; no
   implicit zeroing or reversion to declared defaults, matching §6 of the backend contract.
7. **Cold-start policy — answered 2026-11-13 (owner).** Accepted as proposed: an unreachable
   broker never fails the run. The instance starts faulted and retries forever with the backoff
   ladder; `startup-timeout` remains a Modbus/EtherCAT-only key and is rejected on an `mqtt`
   device.
8. **Broker topology — answered 2026-11-13 (owner).** Accepted as proposed: one broker per data
   device; several broker devices may exist in one project, each owning its own session. A
   shared-session key that multiplexes devices onto one client connection is out of scope.
9. **Mapping cardinality — answered 2026-11-13 (owner).** Accepted as proposed: one topic per
   channel; wildcards are allowed on subscribe topics only and rejected on publish topics; at
   most one channel per (direction, topic) pair, so a topic cannot fan in two inputs.
10. **Test broker — answered 2026-11-13 (owner).** Accepted as proposed: an in-process simulator
    plus a loopback fixture carries the automated tests; no external broker and no CI service
    container. The third-party broker is used only for the manual evidence in Q11.
11. **Non-loopback evidence — answered 2026-11-13 (owner).** Required, not optional. The manual
    walkthrough includes a two-host run: a small Python MQTT 5 peer host on the second host and a
    real third-party broker, both over TLS. [Mosquitto](https://mosquitto.org/) is the accepted
    broker for this evidence. It is a test tool only: not shipped, not a build or CI dependency,
    no broker binary, configuration secret, or credential committed, and no claim that a
    particular broker is supported. The Python peer host lives with the test tooling, is never
    part of the Rust build or the release package, and may use an off-the-shelf MQTT client
    library for its own implementation. The report records broker and TLS versions plus the
    commands and output of the run.
12. **Diagnostic codes and status layout — answered 2026-11-13 (owner).** Accepted: the backend
    uses `STC-E-IO-MQTT-00N` and `STC-W-IO-MQTT-00N` codes and status layout 1 as proposed in
    §5.4. The exact code table and channel list are fixed in Phase 4, before the status device is
    implemented, and no code is reused from another backend.
13. **MQTT 5.0 feature scope — answered 2026-11-13 (owner).** Accepted as proposed: session expiry,
    reason codes, maximum packet size, receive maximum, and shared `$share/<group>/` subscriptions
    are in scope. Topic aliases are not used in either direction; request/response metadata
    (response topic, correlation data) and subscription identifiers are not encoded or surfaced,
    and a profile that would need them is rejected rather than silently dropped.
14. **JSON payloads, and whether whole `STRUCT`s can travel in one message — answered
    2026-11-13 (owner).** A single message
    carrying an entire PLC struct is **not feasible without a language/compiler/binding change**,
    so it is outside the locked scope:
    - `AT` declarations and `[[io.bindings]]` resolve through `layout::scalar_kind_at` in
      `crates/stc-codegen/src/config.rs`, which rejects non-scalar leaves, so a struct or any of
      its members is not a bindable image leaf (the same gap [I/O and Devices](specification/Architecture/IO-and-Devices.md)
      §11 lists as an open question).
    - A device channel binds exactly one contiguous `ImageRange` (`BackendChannel.image`), and the
      coordinator enforces that the channel width equals the binding width, so one channel cannot
      span several struct members.
    - `ProcessImage::read_bits`/`write_bits` are capped at 64 bits, so the image cannot move a
      struct through a single channel even if the binding model allowed it.
    - Struct type identity is not in the metadata the backend receives; it is per-leaf scalar
      widths and image offsets only.

    Feasible **without any language or compiler change**: a profile-level JSON encoding over a
    *group of declared scalar channels* on one topic — the backend already receives each channel's
    `ImageRange`, so it can decode one JSON object into several scalar channels and encode several
    channels into one object. Server-side constraints: canonical key order, strict UTF-8, bounded
    size and nesting depth, duplicate-key and non-finite rejection, and integers beyond 2^53
    carried as strings or rejected.

    The same limit applies to PLC `STRING` values: they are non-scalar leaves too, so a
    `string`-encoded MQTT channel carries the text form of a scalar (BOOL/integer/float) channel,
    not a PLC `STRING`. Carrying PLC strings would need the same aggregate work as structs.

    Options:
    (a) add `payload-encoding = "json"` for a channel group in v3 — *recommended*: it covers the
        real use case (one message carrying a set of related fields) with no compiler or binding
        change and no struct descriptor in the wire contract;
    (b) defer JSON entirely and ship scalar/raw encodings only;
    (c) whole-struct JSON, which is **not** WL-15: it needs a compiler/aggregate-layout change
        and a multi-range or per-leaf struct binding contract, and it contradicts the §2 non-goal
        "changing the process-image or device-binding model". It would be a separate target with
        its own owner re-plan.

    **Decision: option (b).** WL-15 ships the scalar/raw encodings only; no
    JSON codec is implemented, tested, or documented as a supported encoding in this target.
    Option (c) is recorded as [FutureWishlist](FutureWishlist.md) WL-22 (aggregate and struct I/O
    binding), which is not in v3.0.0 and needs its own owner re-plan.

## 8. Dependencies and risks

- The coordinator, profile model, and Modbus/EtherCAT backends are delivered and are the
  integration points; no prerequisite target remains.
- MQTT is asynchronous and non-real-time. A slow broker, a partial TLS handshake, or a burst of
  retained messages must never reach the cycle thread; the bank-copy design is the mitigation
  and needs a test that proves it.
- A project-owned client carries protocol-conformance risk, higher for MQTT 5.0 than for 3.1.1
  because property blocks and reason codes are the largest part of the wire format. The mitigation
  is a bounded codec, conformance-vector tests, and fuzzing; there is no external MQTT crate to
  fall back on (Q2).
- TLS is the one approved external dependency (`rustls` plus CA/certificate parsing, Q3). It is a
  build-time dependency of `stc-runtime`, not a host prerequisite, and it must not make the base
  package depend on a broker or add a runtime service.
- Credential handling is the security-sensitive surface: file-only secrets, private-file checks,
  and no leakage in diagnostics.
- Untrusted payloads are decoded on the session thread; bounds are enforced before they reach the
  image.
- The CI matrix has no broker; all automated evidence runs over loopback with ephemeral ports.
  Non-loopback evidence is manual by design (Q11): Mosquitto plus a Python peer host on a second
  host, with TLS, recorded by version and command in the verification report. The manual run is
  not reproducible in CI and must never be presented as a CI result.

## 9. Acceptance log

| Phase | Accepted | Commit | CI run | Notes |
| --- | --- | --- | --- | --- |
| 0 | 2026-11-13 | | | Questions 1–14 answered 2026-11-13; Phase 0 closed; plan approved by the owner 2026-11-13 |
| 1 | | | | Implemented 2026-11-13; `cargo test -p stc-runtime --lib` 416 passed, clippy clean, fuzz target compiles. Commit and CI pending |
| 2 | | | | |
| 3 | | | | |
| 4 | | | | |
| 5 | | | | |
| 6 | | | | |

## 10. Change log

| Date | Change |
| --- | --- |
| 2026-11-13 | Plan drafted on branch `WL-15` from `v3.0.0-planning` (`3aaf898`); submitted for owner approval. |
| 2026-11-13 | Phase 0 questions 1–3 answered by the owner: MQTT 5.0 only (no 3.1.1), a project-owned client with no external MQTT crate, and an accepted external TLS dependency. Plan updated for 5.0 semantics (clean start, session expiry, v5 properties, reason codes) and a `rustls`-based TLS design. |
| 2026-11-13 | Q4 accepted by the owner (six scalar/raw encodings, big-endian default with per-channel `byte-order`, UTF-8 strings with payload-format-indicator). Added Q14: whole-`STRUCT` JSON is infeasible without a compiler/binding change, while JSON over a group of declared scalar channels is feasible and proposed for v3. |
| 2026-11-13 | Q14 answered by the owner: option (b), scalar/raw encodings only, no JSON in WL-15. Option (c) recorded as [FutureWishlist](FutureWishlist.md) WL-22 (aggregate and struct I/O binding), outside the v3.0.0 target set. |
| 2026-11-13 | Q5–Q10 accepted as proposed by the owner (QoS 1 / retain off / clean start / 30 s keepalive; last-value stale policy; non-fatal cold start; one broker per device; one topic per channel with subscribe-only wildcards; in-process simulator, no CI broker). Q11 answered: a mandatory manual two-host run with a Python MQTT 5 peer host and Mosquitto over TLS, recorded as manual evidence only. |
| 2026-11-13 | Plan approved by the owner; Phase 0 accepted. Phase 1 implemented: MQTT 5.0 codec, six payload encodings, topic rules, session state machine, 63 new tests, and a fuzz target. No TLS dependency yet. |
| 2026-11-13 | Q12 and Q13 accepted as proposed by the owner: `STC-E-IO-MQTT-00N`/`STC-W-IO-MQTT-00N` codes with status layout 1 (exact table fixed in Phase 4), and a v5 scope of session expiry, reason codes, packet-size/receive limits, and `$share` subscriptions, without topic aliases, request/response metadata, or subscription identifiers. **Phase 0 closed**; plan awaiting approval. |
