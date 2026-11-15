# mqtt — process-image I/O over an MQTT 5.0 broker

A runnable example of the WL-15 MQTT client backend. It maps two broker topics
to PLC inputs (`Start` and `Setpoint`), publishes two outputs (`Running` and
`Speed`), and mirrors the optional `mqtt-status` diagnostics to retained output
topics so a browser client can watch the session.

> `stc` is **not a safety PLC** and is not responsible for safety. It can
> control real machinery, but safety must be provided by a separate safety
> system and machine design as required by law and regulation. MQTT is
> asynchronous, non-real-time, and carries untrusted network input. The broker
> connection, retained messages, last will, and stale flags are diagnostics, not
> safety signals.

## Files

| Path | Purpose |
| --- | --- |
| `stc.toml` | Device tree, channels, and bindings for one MQTT data device and one `mqtt-status` device |
| `io/broker.toml` | Broker address, QoS/retain/stale policy, last will, and topic/payload map |
| `broker/mosquitto.conf` | Example-only Mosquitto listeners: MQTT on 1883 and WebSocket on 9001 for MQTTX Web |
| `src/main.st` | PLC program: start/speed logic and status mirrors |
| `tests/mqtt_test.st` | Pure `{stc:test}` tests for the speed clamp |

`broker/mosquitto.conf` is a test configuration, not a supported broker
product. The repository ships no broker and no credential.

## 1. Start the broker

Use any MQTT 5.0 broker, or run the example Mosquitto configuration in Docker.
A browser cannot open a raw MQTT TCP socket, so MQTTX Web connects to the
WebSocket listener. This example maps container port 9001 to host port 9002;
use any free host port.

From this directory:

```sh
docker run --rm --name stc-mqtt-broker \
  -p 1883:1883 \
  -p 9002:9001 \
  -v "$PWD/broker/mosquitto.conf:/mosquitto/config/mosquitto.conf:ro" \
  eclipse-mosquitto:2.0.18
```

On PowerShell, replace `$PWD` with `${PWD}`. On a remote broker host, open the
same two ports in the host firewall and use that host's address below.

The runtime connects to `127.0.0.1:1883`, the plaintext MQTT listener, by
default. MQTTX Web connects to `ws://127.0.0.1:9002/mqtt`.

## 2. Start MQTTX Web

```sh
docker run --rm --name stc-mqttx-web -p 8080:80 emqx/mqttx-web:latest
```

Open <http://localhost:8080>, choose **New Connection**, and use:

| Field | Value |
| --- | --- |
| Name | `stc-mqtt-example` |
| Host | `ws://127.0.0.1:9002/mqtt` (or the broker host and chosen port) |
| Port | left as the URL supplies it |
| MQTT Version | `5.0` |
| Client ID | `stc-mqttx-web` |
| Username / Password | empty for the example listener |
| Clean Start | on |

Connect, then subscribe to `stc/example/#` with QoS 1.

## 3. Build, test, and run the project

From this directory:

```sh
stc check
stc test
stc run -v
```

`stc run` uses `stc.toml`'s `[run]` policy and stays active until interrupted.
`stc run -v` prints the final `io-backend mqtt` diagnostics line on exit, so
stop with Ctrl+C to capture it. A healthy run reports `client0.connected=true`,
`client0.state=connected`, `client0.tls=off` (or `verified` with TLS), and
`client0.messages_out` advancing as the retained outputs are published.

The runtime publishes the output values once when the session connects and then
again only when a value changes. All output topics in this example are retained,
so a later subscriber receives the current values immediately.

## 4. Topics and payloads

Payloads are big-endian. `bool` is exactly one byte (`00` or `01`); `float` is
the 4-byte IEEE-754 representation. MQTTX Web's **Hex** payload type is the
easiest way to send and read binary values.

| Channel | Direction | Topic | Encoding | Bytes | Example |
| --- | --- | --- | --- | --- | --- |
| `Start` | input | `stc/example/start` | `bool` | 1 | `01` = running |
| `Setpoint` | input | `stc/example/setpoint` | `float` | 4 | `422A0000` = 42.5 |
| `Running` | output | `stc/example/running` | `bool` | 1 | `01` |
| `Speed` | output | `stc/example/speed` | `float` | 4 | `422A0000` = 42.5 |
| `StatusConnected` | output | `stc/example/connected` | `bool` | 1 | `01` |
| `StatusReconnects` | output | `stc/example/reconnects` | `uint` | 4 | `00000004` |
| `StatusMessagesIn` | output | `stc/example/messages-in` | `uint` | 4 | `00000002` |
| `StatusStale` | output | `stc/example/stale` | `bool` | 1 | `00` |
| last will | broker-published | `stc/example/status` | UTF-8 | — | `offline` |

The `Connected`, `Reconnects`, `MessagesIn`, and `Stale` variables come from the
`BrokerStatus` device (`kind = "mqtt-status"`, layout 1). `src/main.st` copies
them to ordinary output channels so MQTTX Web can display them. They are
diagnostics; a real project can bind the status channels directly instead.

## 5. Verify subscribe, publish, retain, and last will

### Subscribe and retained replay

With the runtime already running, subscribe to `stc/example/#`. MQTTX Web
receives the retained outputs immediately, for example:

```text
stc/example/running       01
stc/example/speed         00000000
stc/example/connected     01
stc/example/reconnects    00000000
stc/example/messages-in   00000000
stc/example/stale         01
```

`stale` is `01` until the first input message arrives, because no input value
has been received yet. It is a last-value cache flag, not a fault.

### Publish inputs

Publish with QoS 1 and payload type **Hex**:

1. `stc/example/start` = `01`
2. `stc/example/setpoint` = `422A0000`

The runtime publishes:

```text
stc/example/running       01
stc/example/speed         422A0000
stc/example/messages-in   00000002
```

`messages-in` counts accepted inbound messages, so it advances by two. Publish
`stc/example/start` = `00` and the runtime publishes `running=00` and
`speed=00000000`; the setpoint is retained in the PLC's cached input but has no
effect while `Start` is false.

The program clamps `Setpoint` to 0..100. Send `42C80000` (100.0) and then
`42C90000` (100.5) to see the published speed stay at `42C80000`; see
`ClampSpeed` in `src/main.st`.

### Retain

Disconnect MQTTX Web and connect again (or connect a second client), then
subscribe to `stc/example/#`. The broker replays the last retained output
values without asking the runtime to republish them. Input topics are not
retained by this example, so MQTTX Web starts with an empty input history.

### Last will

The runtime configures `stc/example/status` = `offline` as its last will. Stop
the runtime **uncleanly** (kill the `mqtt` process or stop the container/host;
do not press Ctrl+C) and the broker publishes the retained will. A subscriber
then sees `stc/example/status` = `offline`. A clean Ctrl+C sends DISCONNECT, so
the will is not published.

## 6. Verify disconnect, reconnect, and stale inputs

With `stc run -v` active and MQTTX Web subscribed:

1. Stop the broker container:

   ```sh
   docker stop stc-mqtt-broker
   ```

2. The runtime stays alive. After `stale-timeout` (5 s in `io/broker.toml`),
   `stc/example/connected` becomes `00` and `stc/example/stale` becomes `01`.
   The PLC keeps the last accepted `Start` and `Setpoint` values; it does not
   zero them.
3. Start the broker again:

   ```sh
   docker start stc-mqtt-broker
   ```

   The runtime reconnects with the configured backoff ladder, re-subscribes,
   and republishes the retained outputs. `connected` returns to `01`, `stale`
   returns to `00` when fresh inputs arrive (or stays `01` until the first
   input message after the restart), and `reconnects` advances.

During the outage the cycle thread keeps running; DNS, connect, TLS, and packet
I/O stay on the session thread. A broker outage is a per-device fault, never a
reason for the scan to stop.

## 7. Optional: TLS and credentials

TLS and credentials are opt-in and file-based. Never put secrets inline in
`stc.toml` or the profile. Both are loaded at cold start and fail closed if the
material is missing, malformed, or not private on Unix.

For a TLS broker, add to the device in `io/broker.toml`:

```toml
tls = { ca-file = "io/ca.pem", server-name = "broker.example" }
```

The broker certificate chain and `server-name` are verified against the
supplied CA. There is no system or bundled trust store. Optional mutual TLS
adds `client-cert-file` and `client-key-file` to the same table. The
`tools/mqtt/mosquitto-tls.conf` file and `tools/mqtt/README.md` show the broker
side of this setup for the manual evidence run. A verified session reports
`client0.tls=verified` in `stc run -v`; an explicit `tls.verify = false`
escape hatch reports `client0.tls=insecure` and `STC-W-IO-MQTT-003`.

For file credentials, add:

```toml
credentials = { username-file = "io/mqtt-user", password-file = "io/mqtt-password" }
```

The broker must require authentication (`allow_anonymous false` and a password
file). A wrong password produces a CONNACK rejection: the verbose line reports
`client0.last_error=4`, `client0.reason=135` (`0x87`, not authorised),
`client0.state=backoff`, and retries with the ladder. No credential byte is
printed in a diagnostic or log.

## 8. Optional: unsupported broker version

The backend speaks MQTT 5.0 only; there is no 3.1.1 fallback or version
negotiation. An MQTT 3.1.1-only broker answers the protocol level 5 CONNECT with
CONNACK reason `0x01`. Reproduce it with the test stub:

```sh
python tools/mqtt/v311_only_broker.py 1886 v311.log
```

Point a plaintext copy of the profile at `127.0.0.1:1886`. The runtime reports
`client0.last_error=4`, `client0.reason=1`, and `client0.state=backoff`; it
keeps retrying and the scan keeps running. Stop the stub when finished.

## 9. Troubleshooting

| Symptom | Action |
| --- | --- |
| MQTTX Web cannot connect to `1883` | Browsers need the WebSocket listener; use `ws://<host>:9002/mqtt`, not `mqtt://<host>:1883` |
| `client0.connected=false`, `last_error=2` | The broker is not reachable on the profile's host/port; check the container mapping and firewall |
| `last_error=4`, `reason=135` | The broker rejected the credentials; check the username/password files and broker password file |
| `last_error=4`, `reason=1` | The broker does not accept MQTT 5.0; use an MQTT 5.0 broker |
| TLS line says `insecure` | `tls.verify = false` is set; remove it and supply the correct CA and server name |
| Build fails with a DLL/output permission error | An earlier `mqtt` process is still running and holds the artifact; stop it and rebuild |

## Contract and evidence

- Backend contract: [I/O Routing and Backends](../../specification/Architecture/IO-Routing-and-Backends.md) §4.6.
- Profile schema: [CLI and Manifest](../../specification/Architecture/CLI-and-Manifest.md) §2.2.2.
- Verification report: [MQTT-Client.md](../../specification/Verification/MQTT-Client.md).
- Manual two-host harness: [tools/mqtt/README.md](../../tools/mqtt/README.md).
