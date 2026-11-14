#!/usr/bin/env python3
"""Manual MQTT 5 peer host for WL-15 evidence (Q11).

This script is test tooling, not a product surface: it is not part of the Rust
build, the CI matrix, the test suite, or the release package. It is used on a
second host for the manual two-host run against Mosquitto and the `stc`
runtime, with TLS and optional client-certificate authentication.

Requires `paho-mqtt` 2.x on the peer host only:

    python3 -m pip install --user 'paho-mqtt>=2,<3'

Examples:

    # Subscribe to everything the runtime publishes, over one-way TLS.
    python3 mqtt_peer.py subscribe --host broker.example --port 8883 \
        --ca io/ca.pem --topic 'line1/plc/#'

    # Subscribe with mutual TLS.
    python3 mqtt_peer.py subscribe --host broker.example --port 8883 \
        --ca io/ca.pem --cert io/peer.pem --key io/peer.key \
        --topic 'line1/plc/#'

    # Publish one input value, retained.
    python3 mqtt_peer.py publish --host broker.example --port 8883 \
        --ca io/ca.pem --topic line1/plc/cmd/start --message 1 --retain

    # Publish a value every second for a slow-broker observation.
    python3 mqtt_peer.py publish --host broker.example --port 8883 \
        --ca io/ca.pem --topic line1/plc/speed --message 1.5 --repeat 30

Every run prints one line per event with a UTC timestamp so the transcript can
be pasted into the verification report. Certificate contents are never printed;
only the file paths named on the command line are echoed.
"""

from __future__ import annotations

import argparse
import ssl
import sys
import time
from datetime import datetime, timezone

try:
    import paho.mqtt.client as mqtt
except ImportError:  # pragma: no cover - tooling-only dependency
    sys.exit("paho-mqtt 2.x is required on the peer host: pip install 'paho-mqtt>=2,<3'")


def stamp() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"


def log(message: str) -> None:
    print(f"{stamp()} {message}", flush=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Manual MQTT 5 peer host (WL-15)")
    parser.add_argument("mode", choices=("subscribe", "publish"))
    parser.add_argument("--host", required=True)
    parser.add_argument("--port", type=int, default=8883)
    parser.add_argument("--client-id", default="stc-peer")
    parser.add_argument("--topic", required=True)
    parser.add_argument("--qos", type=int, choices=(0, 1, 2), default=1)
    parser.add_argument("--message", default="1")
    parser.add_argument(
        "--hex",
        action="store_true",
        help="treat --message as hex bytes for typed payloads (float, raw)",
    )
    parser.add_argument("--retain", action="store_true")
    parser.add_argument("--repeat", type=int, default=1)
    parser.add_argument("--interval", type=float, default=1.0)
    parser.add_argument("--timeout", type=float, default=30.0)
    parser.add_argument("--ca", help="CA file used to verify the broker")
    parser.add_argument("--cert", help="client certificate file")
    parser.add_argument("--key", help="client private key file")
    parser.add_argument(
        "--username-file", help="file holding the user name (trailing CR/LF trimmed)"
    )
    parser.add_argument(
        "--password-file", help="file holding the password; requires --username-file"
    )
    parser.add_argument(
        "--insecure",
        action="store_true",
        help="skip certificate and host-name checks (manual diagnostics only)",
    )
    return parser.parse_args()


def build_client(args: argparse.Namespace) -> mqtt.Client:
    client = mqtt.Client(
        mqtt.CallbackAPIVersion.VERSION2,
        client_id=args.client_id,
        protocol=mqtt.MQTTv5,
    )
    if args.ca:
        log(f"tls ca={args.ca} cert={args.cert or '-'} key={args.key or '-'}")
        client.tls_set(
            ca_certs=args.ca,
            certfile=args.cert,
            keyfile=args.key,
            tls_version=ssl.PROTOCOL_TLS_CLIENT,
        )
        if args.insecure:
            client.tls_insecure_set(True)
            log("tls verification disabled (manual diagnostics only)")
    elif args.cert or args.key:
        sys.exit("--cert/--key require --ca")

    if args.username_file:
        with open(args.username_file, encoding="utf-8") as handle:
            username = handle.read().strip()
        password = None
        if args.password_file:
            with open(args.password_file, encoding="utf-8") as handle:
                password = handle.read().strip()
        client.username_pw_set(username, password)
        log(
            f"credentials user-file={args.username_file} "
            f"password-file={args.password_file or '-'}"
        )
    elif args.password_file:
        sys.exit("--password-file requires --username-file")
    return client


def main() -> int:
    args = parse_args()
    client = build_client(args)

    def on_connect(_client, _userdata, _flags, reason_code, _properties):
        log(f"connect rc={reason_code}")

    def on_disconnect(_client, _userdata, _flags, reason_code, _properties):
        log(f"disconnect rc={reason_code}")

    def on_message(_client, _userdata, message):
        log(
            f"publish topic={message.topic} qos={message.qos} "
            f"retain={message.retain} payload={message.payload!r}"
        )

    client.on_connect = on_connect
    client.on_disconnect = on_disconnect
    client.on_message = on_message

    log(f"connecting host={args.host} port={args.port} id={args.client_id}")
    client.connect(args.host, args.port, keepalive=30)
    client.loop_start()
    try:
        if args.mode == "subscribe":
            result, _mid = client.subscribe(args.topic, qos=args.qos)
            log(f"subscribe topic={args.topic} qos={args.qos} result={result}")
            deadline = time.monotonic() + args.timeout
            while time.monotonic() < deadline:
                time.sleep(0.2)
        else:
            payload = bytes.fromhex(args.message) if args.hex else args.message
            for index in range(args.repeat):
                info = client.publish(
                    args.topic,
                    payload=payload,
                    qos=args.qos,
                    retain=args.retain,
                )
                info.wait_for_publish(timeout=5)
                log(
                    f"published topic={args.topic} qos={args.qos} "
                    f"retain={args.retain} payload={args.message!r} "
                    f"({index + 1}/{args.repeat})"
                )
                if index + 1 < args.repeat:
                    time.sleep(args.interval)
    finally:
        client.disconnect()
        client.loop_stop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
