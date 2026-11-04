#!/usr/bin/env python3
"""Modbus TCP client check for the tests/modbus fixture (WL-5.1 phase 4).

The script drives a real TCP connection to the fixture's two loopback binds
and verifies the full functional vector documented in README.md:

  * FC5/FC15 writes on the coil table (1-bit inputs);
  * FC6/FC16 writes on the holding-register table (8/16/32/64-bit inputs,
    including the low/high 8-bit pair, the device-level low-first order, the
    per-channel "dcba" order, and an explicit integer word-order list);
  * FC1/FC2 reads on both bit tables and FC3/FC4 reads on both register
    tables (8/16/32/64-bit outputs);
  * one register bank shared by both configured binds.

All addresses on the wire are zero-based: address 0 is the first element of
every table. The script never uses vendor 00001/10001/30001/40001 notation on
the wire.

Usage:
  python3 client.py [--host 127.0.0.1] [--port 15020] [--port-b 15021]
                    [--timeout 10]

Exit codes:
  0  every check passed
  1  a value or exception response did not match
  2  connection, framing, or protocol failure

Standard library only; this is a manual/host-run client, not a CI driver.
"""

from __future__ import annotations

import argparse
import socket
import struct
import sys
import time

UNIT_ID = 1

# Zero-based protocol addresses from io/bench.toml. The comments give the
# one-based vendor reference number for reference only.
COIL_IN_START = 0       # vendor 00001
COIL_OUT_ADDR = 1024    # vendor 01025
DISCRETE_OUT_ADDR = 0   # vendor 10001
HOLDING_BYTES = 0       # vendor 40001 (low/high 8-bit pair)
HOLDING_WORD_IN = 2     # vendor 40003
HOLDING_DWORD_IN = 4    # vendor 40005..40006 (device low-first)
HOLDING_DWORD_SWAP = 6  # vendor 40007..40008 ("dcba")
HOLDING_LWORD_IN = 12   # vendor 40013..40016 (integer list)
HOLDING_BYTES_OUT = 8   # vendor 40009 (low/high 8-bit pair)
HOLDING_WORD_OUT = 10   # vendor 40011
INPUT_DWORD_OUT = 0     # vendor 30001..30002 (device low-first)
INPUT_DWORD_SWAP = 2    # vendor 30003..30004 ("dcba")
INPUT_LWORD_OUT = 4     # vendor 30005..30008 (integer list)


class ProtocolError(Exception):
    """A connection or framing failure (exit 2)."""


class Mismatch(Exception):
    """A value or exception response did not match the expectation (exit 1)."""


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

    def _exchange(self, pdu: bytes) -> bytes:
        self.transaction = (self.transaction + 1) & 0xFFFF
        if self.transaction == 0:
            self.transaction = 1
        adu = struct.pack(">HHHB", self.transaction, 0, len(pdu) + 1, UNIT_ID) + pdu
        try:
            self.sock.sendall(adu)
            header = self._recv_exact(7)
            transaction, protocol, length, unit = struct.unpack(">HHHB", header)
            if transaction != self.transaction:
                raise ProtocolError(
                    f"transaction id {transaction} does not echo {self.transaction}"
                )
            if protocol != 0:
                raise ProtocolError(f"response protocol id {protocol} is not 0")
            if unit != UNIT_ID:
                raise ProtocolError(f"response unit id {unit} is not {UNIT_ID}")
            if not 2 <= length <= 254:
                raise ProtocolError(f"response MBAP length {length} is out of range")
            return self._recv_exact(length - 1)
        except OSError as error:
            raise ProtocolError(f"socket exchange failed: {error}") from error

    def _recv_exact(self, count: int) -> bytes:
        data = b""
        while len(data) < count:
            chunk = self.sock.recv(count - len(data))
            if not chunk:
                raise ProtocolError("connection closed mid-response")
            data += chunk
        return data

    def _check_exception(self, pdu: bytes, function: int) -> None:
        if pdu and pdu[0] == function | 0x80:
            code = pdu[1] if len(pdu) > 1 else -1
            raise Mismatch(f"FC{function} returned exception 0x{code:02X}")

    def read_bits(self, function: int, address: int, quantity: int) -> list[bool]:
        pdu = self._exchange(struct.pack(">BHH", function, address, quantity))
        self._check_exception(pdu, function)
        if len(pdu) < 2 or pdu[0] != function:
            raise ProtocolError(f"FC{function} response shape is invalid")
        byte_count = pdu[1]
        if len(pdu) != 2 + byte_count or byte_count != (quantity + 7) // 8:
            raise ProtocolError(f"FC{function} byte count {byte_count} is invalid")
        bits = []
        for index in range(quantity):
            bits.append(bool(pdu[2 + index // 8] & (1 << (index % 8))))
        return bits

    def read_registers(self, function: int, address: int, quantity: int) -> list[int]:
        pdu = self._exchange(struct.pack(">BHH", function, address, quantity))
        self._check_exception(pdu, function)
        if len(pdu) < 2 or pdu[0] != function:
            raise ProtocolError(f"FC{function} response shape is invalid")
        byte_count = pdu[1]
        if len(pdu) != 2 + byte_count or byte_count != quantity * 2:
            raise ProtocolError(f"FC{function} byte count {byte_count} is invalid")
        return list(struct.unpack(f">{quantity}H", pdu[2:]))

    def write_coil(self, address: int, value: bool) -> None:
        pdu = self._exchange(struct.pack(">BHH", 5, address, 0xFF00 if value else 0x0000))
        self._check_exception(pdu, 5)
        if pdu != struct.pack(">BHH", 5, address, 0xFF00 if value else 0x0000):
            raise Mismatch(f"FC5 echo at address {address} did not match")

    def write_coils(self, address: int, values: list[bool]) -> None:
        quantity = len(values)
        packed = bytearray((quantity + 7) // 8)
        for index, value in enumerate(values):
            if value:
                packed[index // 8] |= 1 << (index % 8)
        pdu = self._exchange(
            struct.pack(">BHHB", 15, address, quantity, len(packed)) + bytes(packed)
        )
        self._check_exception(pdu, 15)
        if pdu != struct.pack(">BHH", 15, address, quantity):
            raise Mismatch(f"FC15 echo at address {address} did not match")

    def write_register(self, address: int, value: int) -> None:
        pdu = self._exchange(struct.pack(">BHH", 6, address, value))
        self._check_exception(pdu, 6)
        if pdu != struct.pack(">BHH", 6, address, value):
            raise Mismatch(f"FC6 echo at address {address} did not match")

    def write_registers(self, address: int, values: list[int]) -> None:
        quantity = len(values)
        body = struct.pack(f">{quantity}H", *values)
        pdu = self._exchange(struct.pack(">BHHB", 16, address, quantity, len(body)) + body)
        self._check_exception(pdu, 16)
        if pdu != struct.pack(">BHH", 16, address, quantity):
            raise Mismatch(f"FC16 echo at address {address} did not match")


def expect(label: str, actual: object, expected: object) -> None:
    if actual != expected:
        raise Mismatch(f"{label}: got {actual!r}, expected {expected!r}")
    print(f"ok   {label} = {actual!r}")


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


def run_check(host: str, port_a: int, port_b: int, timeout: float) -> None:
    first = connect_with_retry(host, port_a, timeout)
    second = connect_with_retry(host, port_b, timeout)
    try:
        # ---- Write every input table through bind A ----------------------
        first.write_coil(0, True)
        first.write_coils(COIL_IN_START, [True, True, False])
        # Holding register 0 carries the 8-bit low/high pair: 0x34/0x12.
        first.write_register(HOLDING_BYTES, 0x1234)
        first.write_register(HOLDING_WORD_IN, 0xA1B2)
        # Device-level low-first: AABBCCDD -> CC DD AA BB on the wire.
        first.write_registers(HOLDING_DWORD_IN, [0xCCDD, 0xAABB])
        # Per-channel "dcba": 11223344 -> 44 33 22 11 on the wire.
        first.write_registers(HOLDING_DWORD_SWAP, [0x4433, 0x2211])
        # Integer list [4,5,6,7,0,1,2,3]: 0102030405060708 -> E F G H A B C D.
        first.write_registers(
            HOLDING_LWORD_IN, [0x0506, 0x0708, 0x0102, 0x0304]
        )

        # ---- Read every output table through bind B ----------------------
        deadline = time.monotonic() + timeout
        expected_coil = [True]
        while True:
            coils = second.read_bits(1, COIL_OUT_ADDR, 1)
            if coils == expected_coil or time.monotonic() >= deadline:
                break
            time.sleep(0.05)
        expect("FC1 coil output 1024", coils, expected_coil)

        expect("FC2 discrete input 0", second.read_bits(2, DISCRETE_OUT_ADDR, 1), [True])

        expected_bytes = [0x1234]
        expected_word = [0xA1B2]
        expected_dword = [0xCCDD, 0xAABB]
        expected_swap = [0x4433, 0x2211]
        expected_lword = [0x0506, 0x0708, 0x0102, 0x0304]

        deadline = time.monotonic() + timeout
        while True:
            values = (
                second.read_registers(3, HOLDING_BYTES_OUT, 1)
                + second.read_registers(3, HOLDING_WORD_OUT, 1)
                + second.read_registers(4, INPUT_DWORD_OUT, 2)
                + second.read_registers(4, INPUT_DWORD_SWAP, 2)
                + second.read_registers(4, INPUT_LWORD_OUT, 4)
            )
            if values == (
                expected_bytes
                + expected_word
                + expected_dword
                + expected_swap
                + expected_lword
            ) or time.monotonic() >= deadline:
                break
            time.sleep(0.05)
        expect("FC3 holding output register 8 (8-bit pair)", values[0:1], expected_bytes)
        expect("FC3 holding output register 10 (16-bit)", values[1:2], expected_word)
        expect("FC4 input registers 0..1 (device low-first)", values[2:4], expected_dword)
        expect("FC4 input registers 2..3 (dcba)", values[4:6], expected_swap)
        expect("FC4 input registers 4..7 (integer list)", values[6:10], expected_lword)

        # The input tables also read back the last written bank value.
        expect(
            "FC1 coil inputs 0..2",
            second.read_bits(1, COIL_IN_START, 3),
            [True, True, False],
        )
        expect(
            "FC3 holding input register 0 (8-bit pair)",
            second.read_registers(3, HOLDING_BYTES, 1),
            [0x1234],
        )
        expect(
            "FC3 holding input registers 4..5 (device low-first)",
            second.read_registers(3, HOLDING_DWORD_IN, 2),
            [0xCCDD, 0xAABB],
        )
        expect(
            "FC3 holding input registers 12..15 (integer list)",
            second.read_registers(3, HOLDING_LWORD_IN, 4),
            [0x0506, 0x0708, 0x0102, 0x0304],
        )

        # ---- Both binds share one register bank --------------------------
        # A write through bind B is read back through bind A and published to
        # the output register through the same bank.
        second.write_register(HOLDING_WORD_IN, 0xBEEF)
        deadline = time.monotonic() + timeout
        while True:
            via_a = first.read_registers(3, HOLDING_WORD_IN, 1)
            published = first.read_registers(3, HOLDING_WORD_OUT, 1)
            if (via_a, published) == ([0xBEEF], [0xBEEF]) or time.monotonic() >= deadline:
                break
            time.sleep(0.05)
        expect("bind B write visible on bind A (input register 2)", via_a, [0xBEEF])
        expect("bind B write published on bind A (output register 10)", published, [0xBEEF])
        expect(
            "bind B reads its own write",
            second.read_registers(3, HOLDING_WORD_IN, 1),
            [0xBEEF],
        )

        # ---- Status device mirror is served through the image ------------
        # The status device is not addressable over Modbus; it is visible to
        # the PLC and to `stc run -v`. The client only asserts that the input
        # register bank still answers after the exchanges above.
        expect(
            "FC3 input registers 6..7 after all writes",
            first.read_registers(3, HOLDING_DWORD_SWAP, 2),
            [0x4433, 0x2211],
        )
    finally:
        first.close()
        second.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=15020, help="first bind")
    parser.add_argument("--port-b", type=int, default=15021, help="second bind")
    parser.add_argument("--timeout", type=float, default=10.0)
    args = parser.parse_args()
    try:
        run_check(args.host, args.port, args.port_b, args.timeout)
    except Mismatch as error:
        print(f"FAIL {error}", file=sys.stderr)
        return 1
    except ProtocolError as error:
        print(f"ERROR {error}", file=sys.stderr)
        return 2
    print("PASS all Modbus TCP fixture checks")
    return 0


if __name__ == "__main__":
    sys.exit(main())
