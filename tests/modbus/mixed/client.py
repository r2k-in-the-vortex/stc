#!/usr/bin/env python3
"""Modbus TCP network-client check for the mixed EtherCAT + Modbus bench.

Connects to the mixed fixture's Modbus server, writes command word 0 and
coil 0, and confirms the PLC scan published word 1 = word 0 + 1 and coil 1 =
coil 0 while the DIEWU EtherCAT slave is being served by the same
coordinator. The check runs two value rounds so the result is not a stale
first-scan value.

All addresses are zero-based (holding register 0 = vendor 40001, coil 0 =
00001). Standard library only.

Usage:
  python3 client.py [--host 127.0.0.1] [--port 15030] [--timeout 10]

Typical bench use from the build workstation:
  python3 client.py --host 192.168.1.90

Exit codes:
  0  every check passed
  1  a value did not match
  2  connection or framing failure
"""

from __future__ import annotations

import argparse
import socket
import struct
import sys
import time

UNIT_ID = 1
RENDEZVOUS = 0


class ProtocolError(Exception):
    """Connection or framing failure (exit 2)."""


class Mismatch(Exception):
    """Value mismatch (exit 1)."""


class Client:
    def __init__(self, host: str, port: int, timeout: float) -> None:
        try:
            self.sock = socket.create_connection((host, port), timeout=timeout)
        except OSError as error:
            raise ProtocolError(f"cannot connect to {host}:{port}: {error}") from error
        self.sock.settimeout(timeout)
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.transaction = 0

    def close(self) -> None:
        self.sock.close()

    def _recv_exact(self, count: int) -> bytes:
        data = b""
        while len(data) < count:
            chunk = self.sock.recv(count - len(data))
            if not chunk:
                raise ProtocolError("connection closed mid-response")
            data += chunk
        return data

    def _exchange(self, pdu: bytes) -> bytes:
        self.transaction = (self.transaction + 1) & 0xFFFF or 1
        adu = struct.pack(">HHHB", self.transaction, 0, len(pdu) + 1, UNIT_ID) + pdu
        try:
            self.sock.sendall(adu)
            transaction, protocol, length, unit = struct.unpack(
                ">HHHB", self._recv_exact(7)
            )
            if (transaction, protocol, unit) != (self.transaction, 0, UNIT_ID):
                raise ProtocolError(
                    f"bad MBAP echo {transaction}/{protocol}/{unit}"
                )
            if not 2 <= length <= 254:
                raise ProtocolError(f"MBAP length {length} out of range")
            return self._recv_exact(length - 1)
        except OSError as error:
            raise ProtocolError(f"socket exchange failed: {error}") from error

    def _check(self, pdu: bytes, function: int) -> None:
        if pdu and pdu[0] == function | 0x80:
            raise Mismatch(f"FC{function} exception 0x{pdu[1]:02X}")
        if not pdu or pdu[0] != function:
            raise ProtocolError(f"FC{function} response shape is invalid")

    def write_register(self, address: int, value: int) -> None:
        request = struct.pack(">BHH", 6, address, value)
        response = self._exchange(request)
        self._check(response, 6)
        if response != request:
            raise Mismatch(f"FC6 echo at register {address} did not match")

    def read_registers(self, address: int, quantity: int) -> list[int]:
        response = self._exchange(struct.pack(">BHH", 3, address, quantity))
        self._check(response, 3)
        if response[1] != quantity * 2 or len(response) != 2 + quantity * 2:
            raise ProtocolError("FC3 byte count is invalid")
        return list(struct.unpack(f">{quantity}H", response[2:]))

    def write_coil(self, address: int, value: bool) -> None:
        request = struct.pack(">BHH", 5, address, 0xFF00 if value else 0)
        response = self._exchange(request)
        self._check(response, 5)
        if response != request:
            raise Mismatch(f"FC5 echo at coil {address} did not match")

    def read_coil(self, address: int) -> bool:
        response = self._exchange(struct.pack(">BHH", 1, address, 1))
        self._check(response, 1)
        if response[1] != 1 or len(response) != 3:
            raise ProtocolError("FC1 byte count is invalid")
        return bool(response[2] & 1)


def connect_with_retry(host: str, port: int, timeout: float) -> Client:
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            return Client(host, port, timeout)
        except ProtocolError as error:
            last = error
            time.sleep(0.1)
    raise ProtocolError(f"no server on {host}:{port} within {timeout}s: {last}")


def wait_for(client: Client, label: str, read, expected, timeout: float):
    deadline = time.monotonic() + timeout
    while True:
        value = read()
        if value == expected:
            return value
        if time.monotonic() >= deadline:
            raise Mismatch(f"{label}: got {value!r}, expected {expected!r}")
        time.sleep(0.02)


def check_round(client: Client, command: int, bit: bool, timeout: float) -> None:
    client.write_register(0, command)
    client.write_coil(0, bit)
    seen = wait_for(
        client,
        f"holding register 1 for command {command:#06x}",
        lambda: client.read_registers(1, 1)[0],
        (command + 1) & 0xFFFF,
        timeout,
    )
    out = wait_for(
        client,
        f"coil 1 for bit {bit}",
        lambda: client.read_coil(1),
        bit,
        timeout,
    )
    back = client.read_registers(0, 1)[0]
    back_bit = client.read_coil(0)
    if back != command or back_bit != bit:
        raise Mismatch(
            f"input readback mismatch: word {back:#06x}, coil {back_bit}"
        )
    print(
        f"ok   command {command:#06x} -> indicator {seen:#06x}; "
        f"bit {int(bit)} -> coil {int(out)}"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=15030)
    parser.add_argument("--timeout", type=float, default=10.0)
    args = parser.parse_args()
    client = None
    try:
        client = connect_with_retry(args.host, args.port, args.timeout)
        check_round(client, RENDEZVOUS, True, args.timeout)
        check_round(client, 0x1357, False, args.timeout)
    except Mismatch as error:
        print(f"FAIL {error}", file=sys.stderr)
        return 1
    except ProtocolError as error:
        print(f"ERROR {error}", file=sys.stderr)
        return 2
    finally:
        if client is not None:
            client.close()
    print("PASS mixed EtherCAT + Modbus TCP client check")
    return 0


if __name__ == "__main__":
    sys.exit(main())
