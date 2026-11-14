# MQTT client verification report — WL-15

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-11-14  
Status: Phase 5 automated evidence complete; the manual two-host Mosquitto run is
procedural evidence that only the owner can execute on two real hosts.  
Origin: v3.0.0 target WL-15 ([WL-15-Plan.md](../../WL-15-Plan.md))  
Contract: [I/O Routing and Backends](../Architecture/IO-Routing-and-Backends.md) §4.6  
Tooling: [tools/mqtt/README.md](../../tools/mqtt/README.md)

## 1. Result

The MQTT backend is a project-owned MQTT 5.0 **client** integrated through the
multi-backend I/O coordinator. A project declares one or more devices with
`kind = "mqtt"`, maps each declared channel to a subscription filter or publish
topic in the selected machine profile, and optionally adds a `kind = "mqtt-status"`
device exposing layout 1. TLS uses the approved `rustls` stack with a
user-supplied CA file, optional client-certificate authentication, and file
credentials. The broker stays external: the base package has no broker
dependency, no broker binary, and no credential.

Phase 5 adds the deterministic broker simulator, the fault-injection and
cross-host suites, and the cycle-overhead harness:

- A test-only scripted MQTT 5.0 broker (`crates/stc-runtime/tests/mqtt_sim`)
  speaks loopback MQTT 5.0 with ephemeral ports, records every client packet, and
  injects drop, delay, malformed frame, oversized frame, duplicate delivery,
  retained replay, rejected CONNACK, and absent CONNACK.
- Six fault-injection tests exercise protocol, oversize, payload-decode,
  duplicate-QoS-2, slow-CONNACK, and capture behavior. They run in the existing
  CI matrix on every supported host, so they are the cross-host end-to-end
  fixture.
- A release-mode overhead harness measures the `DeviceBackend` cycle boundary
  with an idle, a busy, and a faulted broker.
- A manual two-host harness (`tools/mqtt/`) provides the Python MQTT 5 peer and a
  Mosquitto TLS configuration. It is test tooling only and never enters the
  build, the test suite, CI, or the release package.

Automated evidence is complete and cross-host. The manual two-host run is
documented and reproducible but was **not executed in this environment**: it
needs a second physical host, a broker installation, and operator-owned TLS
material. It must not be reported as a CI result.

## 2. Environment

| Item | Value |
| --- | --- |
| CI hosts | GitHub-hosted `windows-2025` (x86_64 MSVC), `ubuntu-24.04` (x86_64 GNU), `ubuntu-24.04-arm` (AArch64 GNU), `macos-15` and `macos-15-intel` |
| Toolchain | Rust pinned by `rust-toolchain.toml`, edition 2024; `RUSTFLAGS=-D warnings` in CI |
| Functional build | workspace debug, `cargo test --workspace --locked` |
| Overhead build | `cargo test -p stc-runtime --release --test mqtt_overhead` |
| Overhead host | Windows 11 x86_64, 13th Gen Intel Core i7-13850HX, 16 GiB, rustc release profile |
| Network | Automated tests use loopback and ephemeral ports only; no broker, DNS name, or external service is involved |
| Broker for manual evidence | Mosquitto with TLS on the primary host; MQTT 5 peer host on a second machine |

Timing numbers are single-host and are not portable performance claims.

## 3. Automated evidence

| Suite | Tests | Coverage |
| --- | --- | --- |
| `crates/stc-runtime/src/io_mqtt` unit tests | 105 | MQTT 5.0 codec, property table, payload encodings, topic rules, session machine, profile validation, TLS/credential loading |
| `crates/stc-runtime/tests/mqtt_backend.rs` | 16 | Loopback exchange, stale/cache inputs, unreachable and silent brokers, mixed Modbus and mock-EtherCAT isolation, clean shutdown, verified TLS, insecure TLS, wrong CA, mutual TLS, credential CONNECT capture, retained replay, reconnect ladder |
| `crates/stc-runtime/tests/mqtt_faults.rs` | 6 | Malformed frame, oversized frame, undecodable payload, duplicate QoS 2 delivery, slow CONNACK, CONNECT/SUBSCRIBE/PUBLISH capture |
| `crates/stc-runtime/tests/mqtt_online_change.rs` | 3 | Unchanged hash keeps session and counters, changed mapping returns `STC-E-ONLINE-IO-001`, cold download restarts the session |
| `crates/stc-runtime/tests/mqtt_overhead.rs` | 1 (ignored) | Cycle-boundary overhead with idle, busy, and faulted brokers |

Every suite runs in the CI matrix, including the AArch64 and both macOS jobs, so
the functional evidence is cross-host. The tests prove:

- a broker that never answers, rejects CONNECT, or disappears leaves the run
  alive with cached inputs and a per-device fault;
- a malformed or oversized frame, a bad payload, and a duplicate delivery update
  counters and diagnostics without stopping the scan or corrupting the image;
- the cycle thread never blocks on DNS, connect, handshake, or read: 200 cycle
  boundaries complete in under 250 ms while the broker delays CONNACK by 600 ms;
- shutdown sends DISCONNECT with the normal reason (0), and TLS shutdown sends
  `close_notify`;
- TLS fails closed on a wrong CA and on a mismatched client certificate/key,
  and no credential or key byte appears in a diagnostic or fault message.

## 4. Fault injection

The simulator (`tests/mqtt_sim/mod.rs`) is a test-only broker. It decodes every
client packet into an event, so the tests assert the wire contract rather than
re-parsing frames:

| Injection | Observed behavior |
| --- | --- |
| Reserved packet type after SUBACK | `client0.last_error` = 7 (protocol), transport dropped, reconnect on the ladder, session recovers |
| 320 KiB PUBLISH | `client0.last_error` = 8 (oversize), transport dropped, session recovers |
| 3-byte payload for a 16-bit channel | `client0.dropped` advances, `last_error` = 11 (payload decode), the last decodable value stays cached |
| Duplicate QoS 2 PUBLISH with the same identifier | applied once: `client0.messages_in` = 1 |
| 600 ms CONNACK delay | no cycle-boundary stall; `client0.connected` = false and `input.stale` = true |
| Retained PUBLISH after SUBACK | replayed value latches; subscription requests retain handling 0 |
| Transport close after CONNECT | reconnect counter advances, session recovers |

## 5. Cycle-overhead measurement

Release build, Windows x86_64, 20 000 boundary iterations per scenario, 32 bound
channels (16 inputs, 16 outputs, 16-bit each), measured at the `DeviceBackend`
seam (`latch_inputs` + `publish_outputs`):

| Scenario | latch p50 | latch p99 | latch max | publish p50 | publish p99 | publish max | cycle mean | client |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Idle broker | 0.40 µs | 1.10 µs | 22.4 µs | 0.30 µs | 1.00 µs | 146.4 µs | 1.07 µs | connected |
| Busy broker (1 ms publishes) | 0.40 µs | 0.80 µs | 185.2 µs | 0.30 µs | 0.80 µs | 512.7 µs | 0.94 µs | connected, 64 messages |
| Faulted broker | 1.90 µs | 2.00 µs | 1321.2 µs | 1.40 µs | 1.50 µs | 1542.9 µs | 3.56 µs | connecting, 4 reconnects |

The seam cost is sub-microsecond at the median in every scenario. A repeat run
on the same host kept every p50 at 0.3–0.4 µs (faulted latch 1.9 µs) while the
maxima moved with host scheduling: idle 22–133 µs, busy 105–185 µs, faulted
0.6–1.8 ms. The maxima are not a bound; the faulted scenario is wider because
the session thread retries while the boundary loop runs. There is no threshold and no timing gate: the numbers are diagnostic
evidence that the cycle thread does not perform network I/O.

Reproduce:

```text
cargo test -p stc-runtime --release --test mqtt_overhead -- --ignored --nocapture
```

`STC_MQTT_OVERHEAD_ITERATIONS` overrides the iteration count.

## 6. Manual two-host run

The peer host and broker configuration live in
[tools/mqtt/README.md](../../tools/mqtt/README.md). The run covers subscribe,
publish, retained replay, last will, reconnect, stale inputs, the TLS session,
an MQTT 3.1.1-only broker rejection, and a bad-credential failure, and records
broker and TLS versions, material paths (never contents), the peer-script
revision, and the command output.

Status: procedure documented; execution needs a second host, Mosquitto, and
operator-owned certificate material, so it is recorded as manual evidence owned
by the owner and is not part of this automated result.

## 7. Limitations

- MQTT is asynchronous and non-real-time. The backend caches inputs, never
  interpolates motion, and provides no fail-safe output state; see
  [NON-SAFETY.md](../../NON-SAFETY.md).
- The automated broker is a test fixture, not a supported broker. No third-party
  broker is a build, test, or release dependency.
- The overhead figures come from one Windows host in release mode. They are not
  a portability or real-time claim.
- The manual two-host run has not been executed here, so the Q11 evidence still
  needs an owner-run transcript before closeout.
