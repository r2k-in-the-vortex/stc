# Modbus TCP bench fixture (WL-5.1 phase 4)

This fixture exercises the Modbus TCP **server** backend with a real network
client. The PLC program copies every Modbus input channel to an output
channel, so a client can write all four tables and read the results back at
known zero-based addresses. The fixture covers 1-, 8-, 16-, 32-, and 64-bit
channels, both bit tables and both register tables, a low/high 8-bit pair
sharing one register, device-level and per-channel word orders, and two
listeners serving one register bank.

The automated cross-host end-to-end test is
[`crates/stc-driver/tests/modbus_e2e.rs`](../../../crates/stc-driver/tests/modbus_e2e.rs).
It copies this directory to a temporary project, replaces the two loopback
ports with free ones, builds and runs the fixture with the test-built `stc`,
drives the same functional vector through a std-only TCP client, and asserts
the status line on clean exit. The concurrent EtherCAT + Modbus TCP bench
fixture and its recorded result are in [mixed/](mixed/).

## Files

| Path | Purpose |
| --- | --- |
| `stc.toml` | Manifest: one `modbus-tcp` device with 17 data channels, one optional `modbus-tcp-status` device (layout 1), and project bindings for every channel. `[io]` has no global `backend` spelling; the device `kind` selects the backend. |
| `io/bench.toml` | Machine profile: two loopback binds, unit 1, connection and idle limits, device `word-order = "low-first"`, and the zero-based channel map. |
| `src/main.st` | Copies the input image to the output image for coils, bytes, words, dwords, and lwords; 1 ms task. |
| `client.py` | Standard-library-only Modbus TCP client. Writes the input tables, reads the output tables, and checks both binds share one bank. Manual/host runs only. |
| `build.sh` | Builds the fixture with an existing `stc` (`STC=/path/to/stc`). |
| `run.sh` | Runs the fixture (default: until interrupted) with verbose status. |
| `stc.lock` | Lock file for the fixture project. |
| `mixed/` | Concurrent DIEWU EtherCAT + Modbus TCP bench fixture and recorded phase-4 result. |

## Fixture ports

The profile serves two listeners on loopback:

```
127.0.0.1:15020   first bind
127.0.0.1:15021   second bind
```

Both listeners share one register bank, one channel map, and one unit id
(unit 1). A write through either bind is visible through the other, and a
read through either bind returns the same bank contents. The fixture only
binds loopback addresses so it is safe on a shared build workstation. The
mixed bench fixture binds the bench host's LAN address and a separate port;
see [mixed/README.md](mixed/README.md).

## Zero-based addressing

Addresses in `io/bench.toml` and on the wire are **zero-based**: address 0 is
the first element of the table. Vendor documentation usually prints
one-based reference numbers (00001, 10001, 30001, 40001) and they are
converted by subtracting one:

| Table | Wire address 0 | Vendor reference | Functions |
| --- | --- | --- | --- |
| coil | first coil | 00001 | FC1 read, FC5/FC15 write |
| discrete-input | first input | 10001 | FC2 read |
| input-register | first register | 30001 | FC4 read |
| holding-register | first register | 40001 | FC3 read, FC6/FC16 write |

The fixture never uses vendor notation. For example `CoilOut` at zero-based
address 1024 is vendor reference 01025.

## Table and width rules

- Coil and holding-register channels may be inputs or outputs.
- Discrete-input and input-register channels may only be outputs; they are
  read-only over Modbus.
- 1-bit channels map to the bit tables.
- 8-, 16-, 32-, and 64-bit channels map to the register tables.
- 32-bit channels span 2 registers; 64-bit channels span 4 registers. Writes
  that cover only part of a wider channel are rejected with exception 0x02.
- Two 8-bit channels may share one register as the `byte = "low"` and
  `byte = "high"` pair. All other channels need distinct, non-overlapping
  register spans.

## Byte and word order

A 16-bit register is always transmitted most significant byte first, as the
Modbus specification requires. For 32- and 64-bit channels, `word-order`
selects how the 16-bit registers are laid out from the lowest register
address upward. Letters A..H are the value's bytes from most significant to
least significant; the device-level setting applies unless the channel
overrides it.

| `word-order` | 32-bit wire order | 64-bit wire order |
| --- | --- | --- |
| `high-first` / `abcd` | A B C D | A B C D E F G H |
| `low-first` (device) | C D A B | G H E F C D A B |
| `cdab` | C D A B | C D A B G H E F |
| `dcba` | D C B A | D C B A H G F E |
| `badc` | B A D C | B A D C F E H G |
| integer list | explicit permutation, MSB positions in wire order | same, eight entries |

This fixture sets `word-order = "low-first"` at the device level and then
overrides two channels:

| Channel | Value example | Wire registers |
| --- | --- | --- |
| `DwordIn` (device low-first, 32-bit) | `0xAABBCCDD` | `0xCCDD`, `0xAABB` |
| `DwordSwapIn` (`dcba`, 32-bit) | `0x11223344` | `0x4433`, `0x2211` |
| `LwordIn` (list `[4,5,6,7,0,1,2,3]`, 64-bit) | `0x0102030405060708` | `0x0506`, `0x0708`, `0x0102`, `0x0304` |

The explicit 64-bit list is deliberately different from the device low-first
order (`0x0708`, `0x0506`, `0x0304`, `0x0102` for the same value), so a test
that accidentally applies the device default fails. The output channels
`DwordOut`, `DwordSwapOut`, and `LwordOut` carry the same settings, so the
program's copy round-trips the wire layout unchanged.

The 8-bit pair at holding register 0 and 8 uses the register's low and high
bytes: `0x1234` gives `ByteInLo = 0x34` and `ByteInHi = 0x12`.

## Process-image map

`%IBn` is byte n, `%IWn` is bytes 2n..2n+1, `%IDn` is bytes 4n..4n+3, and
`%ILn` is bytes 8n..8n+7.

```
Inputs
  %IX0.0..%IX0.2   CoilIn0..2          coils 0..2
  %IB1, %IB2       ByteInLo/ByteInHi   holding register 0 low/high
  %IW2             WordIn              holding register 2
  %ID2             DwordIn             holding registers 4..5
  %ID3             DwordSwapIn         holding registers 6..7
  %IL2             LwordIn             holding registers 12..15
  %IX24.0          MbListening         server0.listening
  %IW13..%IW15     MbListeners..       server0.listeners/bound_listeners/connections
  %ID8..%ID12      MbRequests..        server0.requests/writes/exceptions/malformed/last_error

Outputs
  %QX0.0, %QX0.1   CoilOut, DiOut      coil 1024, discrete input 0
  %QB1, %QB2       ByteOutLo/ByteOutHi holding register 8 low/high
  %QW2             WordOut             holding register 10
  %QD2             DwordOut            input registers 0..1
  %QD3             DwordSwapOut        input registers 2..3
  %QL2             LwordOut            input registers 4..7
```

The status device is read-only and is visible to the PLC and to
`stc run -v`; it is not addressable over Modbus. Its values are filled at the
input boundary.

## Manual run

Build and start the server in one terminal:

```sh
STC=/path/to/stc bash build.sh
CYCLES=0 bash run.sh
```

Then, in another terminal, run the client:

```sh
python3 client.py
```

The client prints one `ok` line per check and exits 0. It writes the input
tables through the first bind, polls the output tables through the second
bind (the PLC needs a scan to copy them), and then writes through the second
bind and reads through the first to prove the shared bank. A 10-second
connection/poll timeout is the default; use `--timeout` to extend it.

To terminate a cycling run early, interrupt `stc` with Ctrl-C; the verbose
status line is printed before exit. To make the run self-terminating, set
`CYCLES` to a positive count, for example `CYCLES=10000 bash run.sh`.

Expected backend status on exit (values vary with traffic):

```
stc: status: io-backend modbus-tcp devices=2 state=running exchanges=... faults=0 hash=0x... server0.bind=127.0.0.1:15020,127.0.0.1:15021 server0.listeners=2 server0.bound_listeners=2 server0.connections=... server0.peak_connections=... server0.requests=... server0.reads=... server0.writes=... server0.exceptions=0 server0.malformed=0 server0.unbound=0 server0.listener0.bind=127.0.0.1:15020 server0.listener0.state=listening ...
```

`server0.exceptions` counts Modbus exception responses; `server0.malformed`
counts rejected frames; `server0.unbound` counts requests for addresses that
are not mapped. A healthy client run leaves `exceptions`, `malformed`, and
`unbound` at 0.

## Automated test

```sh
cargo test -p stc-driver --test modbus_e2e --locked
```

The test needs `clang`; when it is missing the test skips, following the
other project tests. Set `STC_REQUIRE_CLANG=1` (as CI does) to turn the skip
into a failure. It never uses the fixture's fixed ports: it writes two free
loopback ports into its temporary copy of `io/bench.toml`.
