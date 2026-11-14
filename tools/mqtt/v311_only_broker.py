#!/usr/bin/env python3
"""Test-only MQTT 3.1.1-only broker stub for the manual WL-15 evidence run.

Mosquitto accepts MQTT 3.1.1 and MQTT 5 on the same listener, so it cannot show
what an MQTT 3.1.1-only broker does with an MQTT 5 CONNECT. This stub does
exactly what such a broker must: it reads the protocol level, accepts level 4,
and answers a level 5 CONNECT with CONNACK reason 0x01 (unacceptable protocol
version) before closing.

It is test tooling only: not part of the Rust build, the CI matrix, the test
suite, or the release package.

    python3 v311_only_broker.py 1886 connect.log

Then point a plaintext MQTT profile at 127.0.0.1:1886; the runtime records
`client<N>.last_error` = 4 (CONNACK rejected) and `client<N>.reason` = 1.
"""

from __future__ import annotations

import socket
import sys
from pathlib import Path

MQTT_3_1_1_LEVEL = 4
CONNACK_ACCEPTED = bytes([0x20, 0x03, 0x00, 0x00, 0x00])
CONNACK_UNACCEPTABLE_VERSION = bytes([0x20, 0x03, 0x00, 0x01, 0x00])


def protocol_level(connect: bytes) -> int:
    """Return the CONNECT protocol level, or -1 when the frame is too short."""
    # Fixed header (2 bytes), then a 2-byte protocol-name length, the name
    # itself, and the protocol level byte.
    if len(connect) < 9:
        return -1
    name_length = int.from_bytes(connect[2:4], "big")
    return connect[4 + name_length]


def main() -> int:
    if len(sys.argv) != 3:
        sys.exit("usage: v311_only_broker.py <port> <log-file>")
    port, log_path = int(sys.argv[1]), Path(sys.argv[2])
    with log_path.open("a", encoding="utf-8", buffering=1) as log:
        with socket.socket() as server:
            server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            server.bind(("127.0.0.1", port))
            server.listen(4)
            while True:
                with server.accept()[0] as client:
                    connect = client.recv(4096)
                    if not connect:
                        continue
                    level = protocol_level(connect)
                    log.write(f"CONNECT protocol level {level}\n")
                    accepted = level == MQTT_3_1_1_LEVEL
                    client.sendall(
                        CONNACK_ACCEPTED if accepted else CONNACK_UNACCEPTABLE_VERSION
                    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
