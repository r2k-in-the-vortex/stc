# MQTT client verification report — WL-15

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-11-14
Status: Complete 2026-11-14; automated evidence and both manual runs are
recorded. The manual two-host Mosquitto run is in §6 and the Phase 6
`examples/mqtt` walkthrough with Mosquitto and MQTTX Web is in §6.1, including
TLS, bad-credential, and unsupported-version observations. Owner approval was
recorded 2026-11-14; both runs remain manual evidence, not CI results.
Origin: v3.0.0 target WL-15 ([v3.0.0-plan.md](../../v3.0.0-plan.md) §4)
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

Automated evidence is complete and cross-host. The manual two-host run was
executed on 2026-11-14 with Mosquitto 2.0.18 on `ecat` and the Python peer on a
second host (§6). A second Phase 6 walkthrough ran the committed
[`examples/mqtt`](../../examples/mqtt/README.md) project against a local
Mosquitto container and MQTTX Web (§6.1). Both are manual evidence, not CI
results. The two-host run found one interoperability defect — a legal MQTT 5
acknowledgement without a property length was rejected — which is fixed and
covered by unit tests.

## 2. Environment

| Item | Value |
| --- | --- |
| CI hosts | GitHub-hosted `windows-2025` (x86_64 MSVC), `ubuntu-24.04` (x86_64 GNU), `ubuntu-24.04-arm` (AArch64 GNU), `macos-15` and `macos-15-intel` |
| Toolchain | Rust pinned by `rust-toolchain.toml`, edition 2024; `RUSTFLAGS=-D warnings` in CI |
| Functional build | workspace debug, `cargo test --workspace --locked` |
| Overhead build | `cargo test -p stc-runtime --release --test mqtt_overhead` |
| Overhead host | Windows 11 x86_64, 13th Gen Intel Core i7-13850HX, 16 GiB, rustc release profile |
| Network | Automated tests use loopback and ephemeral ports only; no broker, DNS name, or external service is involved |
| Broker for manual evidence | Mosquitto 2.0.18 on `ecat` (Ubuntu 24.04.5 LTS, Linux 6.12.115-ecat-rt, x86_64, 3 cores), installed user-locally from the distribution archive; mutual TLS with file credentials |
| Peer host | Windows 11 x86_64, Python 3.11.9, `paho-mqtt` 2.2.0 |

Timing numbers are single-host and are not portable performance claims.

## 3. Automated evidence

| Suite | Tests | Coverage |
| --- | --- | --- |
| `crates/stc-runtime/src/io_mqtt` unit tests | 106 | MQTT 5.0 codec, property table, payload encodings, topic rules, session machine, profile validation, TLS/credential loading |
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

Executed 2026-11-14 against a real third-party broker, with the broker and the
runtime on `ecat` and the peer on the second host. The material was generated
on the broker host and kept outside the repository: a self-signed CA, a server
certificate carrying `DNS:ecat` and `IP:192.168.1.90`, a client certificate for
mutual TLS, and a Mosquitto password file. Only paths and parameters are
recorded here, never key or password contents.

TLS session observed from the peer host with `openssl s_client -brief`:
TLSv1.3, `TLS_AES_256_GCM_SHA384`, verification OK. The listener used
`require_certificate true`, `allow_anonymous false`, and a topic ACL.

| Observation | Evidence from the peer transcript and `stc run -v` status |
| --- | --- |
| Subscribe and publish over mutual TLS | peer `connect rc=Success`, `subscribe result=0`; runtime `state=running`, `messages_in=4`, `messages_out=11`, `faults=0`, `client0.tls=verified` |
| Typed payloads both ways | peer published `line1/plc/cmd/start` = `01` and `line1/plc/speed/setpoint` = `422a0000`; runtime published `line1/plc/running` = `01` and `line1/plc/speed` = `422a0000` (42.5) |
| Retained replay | a subscriber connected after the traffic received the retained `cmd/start`, `running`, `speed`, `stale`, `connected`, and `status` values on subscribe |
| Last will | runtime killed with `SIGKILL`; the subscribed peer received retained `line1/plc/status` = `offline` |
| Reconnect | broker stopped for 5 s and restarted; runtime reached `reconnects=5`, `state=connected`, `connected=true`, `tls=verified`, with the last fault `cannot connect to broker ... Connection refused (os error 111)` |
| Stale inputs | across the outage the runtime reported `input.stale=true` and republished `line1/plc/stale` = `01` on recovery |
| Bad credentials | wrong password file: `last_error=4`, `client0.reason=135` (0x87, not authorised), state `backoff`; no credential byte appeared in a diagnostic |
| Unsupported broker version | the `tools/mqtt/v311_only_broker.py` stub answered the client's protocol level 5 CONNECT with CONNACK 0x01: `last_error=4`, `client0.reason=1`, state `backoff` |

The run exposed one interop defect: Mosquitto sends a legal MQTT 5 PUBACK that
carries a reason code and no property length (`remaining length` 3), and a
DISCONNECT with a reason and no property block (`remaining length` 1). The
decoder required a property block in both cases, reported `last_error=7`
("packet body is truncated") and dropped the session after four publishes. Both
decoder paths were fixed and covered by a unit test in
`crates/stc-runtime/src/io_mqtt/packet.rs`; the observations above are from the
rerun with the fix. No other defect was found.

Reproduce: [tools/mqtt/README.md](../../tools/mqtt/README.md). This run is
manual by design and is not a CI result.

### 6.1 `examples/mqtt` walkthrough with Mosquitto and MQTTX Web

Phase 6 adds a second, reproducible manual walkthrough that uses the committed
[`examples/mqtt`](../../examples/mqtt/README.md) project. It ran on 2026-11-14
on a Windows 11 x86-64 host with `stc` from the `WL-15` branch, Mosquitto 2.0.18
in the `eclipse-mosquitto:2.0.18` container, and `emqx/mqttx-web` in a browser
on port 8080. The broker container published `1883` (MQTT) and mapped container
port `9001` (WebSocket) to host port `9002`. This is a local single-host run; it
supplements, and does not replace, the two-host run in §6.

The exact commands are in the example README. The workspace-relative form used
here was:

```text
docker run --rm --name stc-mqtt-broker -p 1883:1883 -p 9002:9001 \
  -v "$PWD/broker/mosquitto.conf:/mosquitto/config/mosquitto.conf:ro" \
  eclipse-mosquitto:2.0.18
docker run --rm --name stc-mqttx-web -p 8080:80 emqx/mqttx-web:latest
stc check
stc test
stc run -v
```

MQTTX Web connected with MQTT 5.0 to `ws://127.0.0.1:9002/mqtt`, client ID
`stc-mqttx-web`, clean start on, and subscribed to `stc/example/#` at QoS 1.
The observations were:

| Observation | Evidence |
| --- | --- |
| Subscribe and retained replay | On subscribe, MQTTX Web received `stc/example/running=00`, `speed=00000000`, `connected=01`, `reconnects=00000000`, `messages-in=00000000`, and `stale=01` without waiting for a new publish |
| Publish and typed payloads | Published `stc/example/start=01` and `stc/example/setpoint=422A0000` (42.5) with QoS 1 and Hex payload type; MQTTX Web then received `running=01`, `speed=422A0000`, and `messages-in=00000002` |
| Retain after reconnect | A MQTTX Web reconnect and fresh subscription replayed the retained outputs; the runtime did not need to republish them for the replay |
| Last will | The `mqtt.exe` runtime process was force-killed; MQTTX Web received the retained `stc/example/status=offline`. A clean Ctrl+C sends DISCONNECT and does not publish the will |
| Broker outage | The broker container was stopped; the runtime stayed alive and kept cycling. After the broker restarted, the runtime reconnected and republished the retained outputs, `connected` returned to `01`, `stale` followed the fresh-input state, and `reconnects` advanced to `00000004` |
| Clean shutdown | Ctrl+C sent DISCONNECT; `stc run -v` printed `client0.connected=true`, `client0.state=connected`, and the expected `messages_in`/`messages_out` counters |

The retained replay also demonstrates the fresh-start `stale=01` value: before
the first input message, no value has been received, so the last-value cache is
marked stale. This is a diagnostic, not a fault or a safety state.

The same example was then run with a local TLS listener and with negative
cases. The TLS profile added:

```toml
tls = { ca-file = "io/ca.pem", server-name = "localhost" }
```

with a local CA and server certificate carrying `DNS:localhost` and
`IP:127.0.0.1`. `stc run --cycles 100 -v` reported:

```text
client0.connected=true client0.dropped=0 client0.host=127.0.0.1
client0.last_error=0 client0.messages_out=7 client0.port=8883
client0.reason=16 client0.reconnects=0 client0.state=connected
client0.tls=verified
```

`openssl s_client -connect 127.0.0.1:8883 -CAfile io/ca.pem -servername
localhost -brief` reported `Protocol version: TLSv1.3`, `Ciphersuite:
TLS_AES_256_GCM_SHA384`, and `Verification: OK`. The broker log recorded the
client as `p5` (MQTT 5.0) on port 8883.

A password-protected listener with the profile's credential files set to the
wrong password produced `client0.last_error=4`, `client0.reason=135` (`0x87`,
not authorised), `client0.state=backoff`, and four fault episodes in 100 cycles.
The broker log recorded `Sending CONNACK to stc-mqtt-example (0, 135)` and
`Client stc-mqtt-example disconnected, not authorised.` No credential byte
appeared in the runtime diagnostic line.

Finally, `tools/mqtt/v311_only_broker.py 1886 v311.log` answered the runtime's
protocol level 5 CONNECT with CONNACK `0x01`. The stub logged `CONNECT protocol
level 5`; the runtime reported `client0.last_error=4`, `client0.reason=1`, and
`client0.state=backoff`, and kept retrying. This is the same
unsupported-version behavior recorded from the Python peer in §6, reproduced
through the committed example.

The owner approved this walkthrough on 2026-11-14. It remains manual evidence
from one local host, one broker version, and one browser client; it is not a CI
result or a broker-support claim.

## 7. Limitations

- MQTT is asynchronous and non-real-time. The backend caches inputs, never
  interpolates motion, and provides no fail-safe output state; see
  [NON-SAFETY.md](../../NON-SAFETY.md).
- The automated broker is a test fixture, not a supported broker. No third-party
  broker is a build, test, or release dependency.
- The overhead figures come from one Windows host in release mode. They are not
  a portability or real-time claim.
- The manual two-host run is single-observation evidence on one broker version
  and one TLS stack; it does not establish support for Mosquitto or any other
  broker. It was executed on `ecat` with a second host as peer, not in CI.
- The Phase 6 MQTTX Web walkthrough is manual evidence from a browser UI plus a
  local Mosquitto container. MQTTX Web is not a supported client surface, and
  the walkthrough is not a CI result or a broker-support claim.
