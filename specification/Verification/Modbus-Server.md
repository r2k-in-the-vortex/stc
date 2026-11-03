# Modbus TCP server verification report — WL-5.1

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-11-02  
Status: Final for WL-5.1; all phases complete on `WL-5.1`; final phase-5 CI run
[41500681839](https://github.com/r2k-in-the-vortex/stc/actions/runs/41500681839) succeeded on all
three hosts.  
Parent: [V2.0.0-PLAN.md](../../V2.0.0-PLAN.md) (WL-5.1)  
Contract: [I/O Routing and Backends](../Architecture/IO-Routing-and-Backends.md) §4.5,
[CLI and Manifest](../Architecture/CLI-and-Manifest.md) §2.2.1  
Decisions: [Decisions](../Decisions.md) §2, D-229–D-243  
Fixture: [tests/modbus/README.md](../../tests/modbus/README.md)

## 1. Result

`stc` now has a pure-Rust Modbus TCP **server** backend integrated through the multi-backend I/O
coordinator delivered by WL-5.2. A project declares one or more devices with
`kind = "modbus-tcp"` and maps each declared channel to a Modbus table and zero-based address in
the selected machine profile. The backend serves MBAP-framed requests on one listener per
configured bind, one thread per connection, and commits values only at the coordinator's input
and publish boundaries. An optional `kind = "modbus-tcp-status"` device exposes layout-1 health
and counters to the PLC.

All WL-5.1 exit criteria are met:

- The codec, register bank, mapping, lifecycle, error paths, online-change behavior, and status
  surfaces are implemented and covered by tests on Windows x86-64 MSVC, Linux x86-64 GNU, and
  Linux AArch64 GNU.
- The cross-host end-to-end fixture drives all four tables, all five widths, a low/high 8-bit
  pair, device-default and per-channel word orders, and two listeners sharing one bank through a
  real TCP client.
- A real mixed bench run had the DIEWU EtherCAT slave and the Modbus TCP server active in one
  coordinator for 60000 cycles with zero backend faults. The run did **not** connect a Modbus
  signal to an EtherCAT signal; that limitation is recorded in §10.
- Cycle-boundary overhead was measured in release on one host with 0, 1, 4, and 8 clients. The
  seam cost is microseconds and host-specific; it is not a real-time claim.

The backend is non-real-time, best-effort, unauthenticated, and not a safety function. It must
not be used as an interpolated motion channel, and safety must be provided by a separate system
and machine design. See [NON-SAFETY.md](../../NON-SAFETY.md) and
[KNOWN-LIMITATIONS.md](../../KNOWN-LIMITATIONS.md) §10.

## 2. Environment

| Item | Value |
| --- | --- |
| CI hosts | GitHub-hosted `windows-2025` (x86_64 MSVC), `ubuntu-24.04` (x86_64 GNU), `ubuntu-24.04-arm` (AArch64 GNU) |
| Toolchain | Rust 1.99.1, edition 2024, pinned by `rust-toolchain.toml`; `RUSTFLAGS=-D warnings` in CI |
| Build | workspace release for timing, debug for the functional suites; CI builds from the locked workspace |
| Mixed bench host | `ecat-host`, Dell Wyse 5070, Intel Celeron J4105, 8 GiB, Linux `6.12.115-ecat-rt` PREEMPT_RT, x86_64 |
| Bench master/slave | IgH EtherCAT master 1.6.13; DIEWU EtherCAT-IO 16I/16O at position 0, PREOP before/after |
| Bench package | CI package from run `41488899175` (phase-3 commit `97c592d`), version 1.0.0, host ABI 2.3 |
| Overhead host | Dell Precision 7680, 13th Gen Intel Core i7-13850HX (20 cores, 28 logical), 16 GiB, Windows 11 Pro 10.0.26300, x86_64 |
| Overhead toolchain | rustc 1.99.1 (58cf8f9c3 2026-10-13), cargo 1.99.1, release profile |
| Network | CI and overhead use loopback only; the bench used the host LAN address `192.168.1.90:15030` |

The functional result is cross-host. The timing and real-hardware results are single-host and
are not portable performance claims.

## 3. Architecture and configuration

The backend is `crates/stc-runtime/src/io_modbus_tcp.rs`. It registers the kinds `modbus-tcp`
and `modbus-tcp-status` as one `ModbusTcpBackend` instance through `register_modbus_tcp` in
`backend_defaults.rs`. No coordinator, `DeviceBackend` trait, route graph, configuration-hash,
EtherCAT, or host-ABI change was needed; device `kind` dispatch selects the backend exactly as
for EtherCAT.

- One listener thread per profile `bind` entry; all binds of a server share one register bank,
  channel map, counters, and `max-connections` budget.
- One thread per accepted connection; blocking reads with an idle timeout; `TCP_NODELAY`;
  accept-and-close at the connection limit.
- The cycle thread only copies bank/image values under a short mutex at `latch_inputs` and
  `publish_outputs`. It performs no socket work.
- Cold start validates every device, channel map, span, direction, width, and image range, then
  binds every configured address before spawning threads. Any failure is all-or-nothing and
  fatal.
- A runtime listener failure is isolated per listener, reported, and retried; other listeners
  and the scan continue.

The profile schema and a complete example are in
[CLI and Manifest](../Architecture/CLI-and-Manifest.md) §2.2.1 and the fixture
[tests/modbus/io/bench.toml](../../tests/modbus/io/bench.toml). The fixture declares one data
device with 17 channels and one status device, and binds two loopback listeners.

```toml
[device.modbus0]
bind = ["127.0.0.1:15020", "127.0.0.1:15021"]
unit = 1
max-connections = 8
idle-timeout = "60s"
word-order = "low-first"

[[device.modbus0.channels]]
name = "CoilIn0"
table = "coil"
address = 0
```

There is no global `[io].backend = "modbus-tcp"` spelling and no `--io` value; a Modbus-only
project omits `[io].backend` or leaves it `null`, and unbound areas keep the null behavior
(D-241/M-13). The profile is the only runtime configuration path; there is no bind override
(D-243/M-15).

## 4. Zero-based addressing and vendor conversion

All addresses in the profile, the PDU, diagnostics, and this report are **zero-based**. Address
0 is the first coil, discrete input, input register, or holding register. The four tables are
independent address spaces.

| Table | First element | Wire address 0 | Vendor reference number | Functions |
| --- | --- | --- | --- | --- |
| Coil | first coil | 0 | 00001 | FC1 read; FC5/FC15 write |
| Discrete input | first input | 0 | 10001 | FC2 read only |
| Input register | first register | 0 | 30001 | FC4 read only |
| Holding register | first register | 0 | 40001 | FC3 read; FC6/FC16 write |

Vendor `0xxxx`/`1xxxx`/`3xxxx`/`4xxxx` notation is one-based and must be converted by
subtracting one before it is written as `address`. For example, vendor holding register `40021`
is `address = 20`; a 32-bit channel there covers `address` 20 and 21 (vendor `40021..40022`).
A vendor document that says "register 1" is ambiguous and must be checked against a known value
or a PDU trace; the server never guesses.

Tests pin both notations for every table:

- `address = 0` ↔ `00001`/`10001`/`30001`/`40001`
- `address = 1` ↔ `00002`/`10002`/`30002`/`40002`
- `address = 1024` ↔ `01025`/`11025`/`31025`/`41025`
- `address 0` is explicitly the first register, not the second.

The zero-based rule is implemented in one place: the configured address is placed directly into
the PDU, and no layer adds or subtracts one. Documentation labels one-based values only as
vendor reference numbers.

## 5. Mapping, widths, and byte/word order

Direction is the core correctness rule. A client may write only a channel that the project
declares as a physical input (`%I`); a PLC output (`%Q`) is read-only to clients.

| Table | Input channel | Output channel |
| --- | --- | --- |
| `coil` | Writable by FC5/FC15; readable by FC1; latched to `%I` | Readable by FC1; writes return exception 0x02 |
| `discrete-input` | Rejected: the table has no write function | Readable by FC2; no write function exists |
| `holding-register` | Writable by FC6/FC16; readable by FC3; latched to `%I` | Readable by FC3; writes return exception 0x02 |
| `input-register` | Rejected: the table has no write function | Readable by FC4; no write function exists |

Widths and spans:

- `coil` and `discrete-input` carry 1-bit `BOOL` values only.
- `holding-register` and `input-register` carry 8-, 16-, 32-, or 64-bit channels. A boolean in a
  register is represented by a 16-bit channel, not a 1-bit channel.
- A 32-bit channel spans 2 consecutive registers and a 64-bit channel spans 4. The span must fit
  in the 16-bit address space and must not overlap another channel, except that one low and one
  high 8-bit channel may share one register.
- An 8-bit register channel selects `byte = "low"` (default) or `byte = "high"`. For register
  value `0x1234`, low is `0x34` and high is `0x12`. A write updates input-claimed bytes and
  preserves output-claimed bytes; unclaimed bytes read zero and ignore writes.
- A write must cover complete input-mapped channels. FC6 cannot write part of a 32/64-bit
  channel, and FC16 must cover its whole span; otherwise the request returns 0x02 and applies
  nothing. A write range containing any unmapped or read-only address fails closed.

Byte and word order:

- Every 16-bit register is big-endian on the wire; the process image is little-endian, and the
  backend converts the numeric value rather than raw image bytes.
- Device-level `word-order` defaults to `high-first`; `low-first` is the other whole-word form.
- A 32/64-bit channel can override the device default with `high-first`, `low-first`, `abcd`,
  `cdab`, `dcba`, `badc`, or an explicit integer permutation whose length is `width/8`.
- Named letter forms are 4-byte patterns applied per 32-bit half for 64-bit values. Device
  `low-first` at 64 bits reverses all four 16-bit words (GHEFCDAB), while per-channel `cdab` is
  the per-half pattern (CDABGHEF).
- Coils and discrete-input bits are packed LSB-first in responses.

The fixture covers the device default and both a named `dcba` override and an explicit 64-bit
list that deliberately differs from the device default. Tests also cover `high-first`,
`low-first`, `abcd`, `cdab`, `badc`, the 64-bit variants, and round-trip permutation checks.

## 6. Scheduling, fault policy, and status

Visible points:

1. A client write completed before `latch_inputs` is visible to the following scan; a write
   after the latch is visible to the next scan.
2. A client read after `publish_outputs` sees the scan's output values. A read racing with the
   copy sees the previous or the new bank, never a partial publish.
3. FC15/FC16 apply under one bank lock, so a latch cannot observe half a multi-write. Last
   writer wins; never-written inputs read zero.
4. A disconnect never clears, zeroes, or stales the last committed inputs. There is no
   `input.stale` concept for Modbus.

Fault policy:

| Event | Behavior |
| --- | --- |
| Cold-start bind/listen failure | Fatal and all-or-nothing (`STC-E-IO-MODTCP-002`); no partial listener set is left running and the run never begins. |
| Runtime listener failure | That listener is `Faulted`, reported (`STC-W-IO-MODTCP-002`), and retried with backoff; other listeners and servers keep serving. Aggregate state stays `Faulted` until every listener is accepting. |
| Malformed frame, unsupported function, illegal address/value, timeout, disconnect | Normal server event: exception where possible, counters updated, connection closed for framing errors; never a backend fault. |
| Client write to an output, unmapped, or read-only address | Exception 0x02; nothing is applied. |
| Partial FC16 span over a 32/64-bit channel | Exception 0x02; nothing is applied. |

Diagnostics are printed by `stc run -v` as one `io-backend modbus-tcp` line per instance. Keys
include `server<N>.bind`, `listeners`, `bound_listeners`, `connections`, `peak_connections`,
`requests`, `reads`, `writes`, `exceptions`, `malformed`, `unbound`, `last_error`, and per
listener `server<N>.listener<M>.{bind,state,connections,last_error}`.

The optional status device uses `config = { layout = 1 }` and exposes `server<N>.listening`,
`listeners`, `bound_listeners`, `connections`, `requests`, `writes`, `exceptions`, `malformed`,
and `last_error` as read-only `%I` channels. Unknown layouts or channel names and width or
direction mismatches fail closed. Status is diagnostic, not safety-rated.

## 7. Protocol and error coverage

Implemented function codes and limits:

| FC | Function | Limit |
| --- | --- | --- |
| 1 | Read Coils | 1..=2000 bits |
| 2 | Read Discrete Inputs | 1..=2000 bits |
| 3 | Read Holding Registers | 1..=125 registers |
| 4 | Read Input Registers | 1..=125 registers |
| 5 | Write Single Coil | one 0xFF00/0x0000 value |
| 6 | Write Single Register | one register |
| 15 | Write Multiple Coils | 1..=1968 bits |
| 16 | Write Multiple Registers | 1..=123 registers |

Exception codes: 0x01 illegal function, 0x02 illegal data address, 0x03 illegal data value,
0x04 server device failure, 0x0B wrong unit id when `unit` is configured. FC8, FC22, FC23, FC43,
broadcast, and RTU are not implemented.

Framing behavior: MBAP protocol id must be 0; length must be 2..=254; ADU maximum is 260 bytes;
transaction and unit ids are echoed. Invalid protocol id, invalid length, oversized ADU, and a
truncated frame that ends the connection are framing errors that close the connection and count
one malformed event. Semantic PDU errors return an exception and keep the connection open.
Pipelined ADUs are processed in order. Arbitrary byte streams and random PDUs are fuzzed at the
codec and bank layers and never panic.

The runtime module contains 61 unit tests covering the codec, table/address/span rules, all
widths and orders, all function codes, quantity/byte-count errors, exceptions, partial wide
writes, atomicity, last-writer-wins, listener isolation, connection limits, idle timeout,
shutdown/drop, status layout, and configuration failures. Integration tests add the loopback
client lifecycle, two-server and multiple-listener behavior, and the mixed mock-EtherCAT
coordinator test.

## 8. Cross-host CI evidence

The existing CI matrix runs the workspace tests on all three native hosts; no Modbus-specific
workflow or timing gate was added.

| Phase | Commit | CI run | Result |
| --- | --- | --- | --- |
| 0 — contracts | `3547e58` | [41473961570](https://github.com/r2k-in-the-vortex/stc/actions/runs/41473961570) | Success |
| 1 — protocol core and bank | `ebfbbc6` | [41478485216](https://github.com/r2k-in-the-vortex/stc/actions/runs/41478485216) | Success |
| 2 — backend and coordinator | `6c55414` | [41483247583](https://github.com/r2k-in-the-vortex/stc/actions/runs/41483247583) | Success |
| 3 — error paths, lifecycle, online change | `97c592d` | [41488899175](https://github.com/r2k-in-the-vortex/stc/actions/runs/41488899175) | Success |
| 4 — cross-host end-to-end and mixed bench | `bf3e7fa` | [41493523376](https://github.com/r2k-in-the-vortex/stc/actions/runs/41493523376) | Success |
| 5 — overhead, documentation, PR | `13bbbed`, `5ceb9df` | [41500681839](https://github.com/r2k-in-the-vortex/stc/actions/runs/41500681839) | Success |

Each run includes the `Rust test (windows-2025)`, `Rust test (ubuntu-24.04)`, and
`Rust test (ubuntu-24.04-arm)` jobs, plus formatting, clippy, docs/specification, traceability,
project, execution, online-change, UI diagnostic, and conformance jobs. The end-to-end fixture
test runs in the workspace test job on all three hosts (`STC_REQUIRE_CLANG=1` makes a missing
clang a failure rather than a skip).

## 9. End-to-end fixture

[tests/modbus/](../../tests/modbus/README.md) contains a complete fixture:

- `stc.toml`: one `modbus-tcp` device with 17 channels, one `modbus-tcp-status` device (layout 1),
  and a binding for every channel.
- `io/bench.toml`: two loopback binds, unit 1, connection and idle limits, device `word-order =
  "low-first"`, and the zero-based map.
- `src/main.st`: copies every input channel to its corresponding output channel in a 1 ms task.
- `client.py`: a stdlib-only Modbus TCP client for manual/host runs.
- `build.sh`, `run.sh`, and `stc.lock`.

The automated test
[modbus_e2e.rs](../../crates/stc-driver/tests/modbus_e2e.rs) copies the fixture to a temporary
project, replaces only the two loopback ports with free ports, builds and runs `stc run`, and
uses a std-only TCP client to:

- write coils, the 8-bit low/high pair, 16-bit, device-default 32-bit, `dcba` 32-bit, and an
  explicit 64-bit list through bind A;
- read the mirrored coil, discrete input, holding-register, and input-register outputs through
  bind B, polling for the 1 ms scan;
- verify both binds serve one register bank by writing through B and reading through A;
- verify the clean exit, `state=running`, `faults=0`, listener binds/states, connection counts,
  and zero exception/malformed/unbound/last-error counters;
- verify both listeners are gone after shutdown.

The fixture demonstrates address 0, address 1, address 1024, register spans, and the
input/output direction matrix on real processes rather than only unit tests.

## 10. Real mixed bench result

[tests/modbus/mixed/RESULTS.md](../../tests/modbus/mixed/RESULTS.md) records a 60000-cycle,
1 ms run on `ecat-host` with both the DIEWU EtherCAT backend and the Modbus TCP server active in
one coordinator. Key results:

- Both backend reports show `state=running`, `exchanges=60000`, `faults=0`.
- EtherCAT diagnostics show a complete working counter (`domain.wc_state=2`), fresh input
  (`input.stale=0`), link up, one responding slave, and zero recoveries.
- Modbus counters show 12 requests, 8 reads, 4 writes, one peak connection, and zero exception,
  malformed, unbound, or last-error counts. The client's two rounds passed.
- Master Tx/Rx counters advanced by 61708 frames with zero Tx errors; lost frames were
  unchanged at 24426.
- The master returned to Idle and the slave to PREOP after release.

**No cross-wiring evidence.** No external device connected a Modbus client signal to a DIEWU
channel. The two paths ran independently: the network client exercised the Modbus server, and
the PLC mirrored DIEWU inputs to outputs while IgH accepted frames. This run does **not** prove
end-to-end Modbus-client-to-EtherCAT-output (or EtherCAT-input-to-Modbus-client) value transfer.
Cross-backend routing over one process image is proven by the mixed mock test
`coordinator_serves_modbus_and_ethercat_over_one_image` in `io_modbus_tcp.rs`; the real bench
proves concurrent health, not physical cross-wiring.

The optional independent third-party client check was not run: `mbpoll`, `modpoll`, and
`pymodbus` were not installed on the bench host or the build workstation. All client
observations use the in-repo stdlib client. This report therefore does not claim third-party
interoperability testing.

## 11. Overhead evidence

Harness: [crates/stc-runtime/tests/modbus_overhead.rs](../../crates/stc-runtime/tests/modbus_overhead.rs),
an `#[ignore]` release test. Raw output:
[tests/modbus/overhead/windows-x86_64-2026-11-02.txt](../../tests/modbus/overhead/windows-x86_64-2026-11-02.txt).
Command:

```sh
cargo test -p stc-runtime --release --test modbus_overhead -- --ignored --nocapture
```

Method: the test constructs `ModbusTcpBackend` directly and calls `cold_start`, then times the
public `DeviceBackend::latch_inputs` and `publish_outputs` calls in a tight loop. Client threads
connect over loopback and issue a round-robin mix of FC1 (coils at 1024, 8 bits), FC3 (holding
registers at 1000, 8 registers), FC4 (input registers at 0, 8 registers), and FC16 (holding
registers at 0, 4 registers). The configuration has 56 bound channels across all four tables,
widths 1/16/32/64, a 48-byte input image, and a 64-byte output image. Three runs of 200000
iterations per client count were made on the host in §2.

Columns are microseconds except throughput and CPU. "cycle" is `latch + publish` only; it is not
a PLC scan period. Each row's CPU is process-wide and includes all client and server threads.

| Run | Clients | Latch p50/p99/max | Publish p50/p99/max | Cycle mean | RTT p50/p99/max | Requests/s |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 0 | 1.8 / 1.9 / 23.4 | 2.9 / 3.1 / 71.1 | 4.706 | — | 0 |
| 1 | 1 | 1.9 / 3.2 / 124.0 | 2.9 / 4.2 / 225.5 | 4.975 | 22.4 / 53.9 / 310.6 | 41481 |
| 1 | 4 | 2.0 / 6.5 / 147.9 | 3.2 / 8.0 / 151.7 | 6.015 | 25.5 / 75.5 / 1521.5 | 136098 |
| 1 | 8 | 2.6 / 46.2 / 432.7 | 3.9 / 50.2 / 357.5 | 13.262 | 36.5 / 112.3 / 3844.0 | 182979 |
| 2 | 0 | 2.5 / 3.2 / 139.3 | 4.1 / 5.4 / 131.7 | 6.397 | — | 0 |
| 2 | 1 | 2.6 / 4.4 / 26.8 | 4.2 / 6.3 / 154.7 | 7.092 | 31.7 / 66.2 / 343.7 | 29715 |
| 2 | 4 | 2.8 / 9.1 / 166.9 | 4.4 / 9.8 / 239.0 | 8.284 | 36.0 / 104.2 / 892.7 | 97868 |
| 2 | 8 | 3.6 / 62.8 / 332.0 | 5.2 / 67.7 / 376.8 | 17.242 | 50.2 / 150.9 / 3910.3 | 132858 |
| 3 | 0 | 2.5 / 2.7 / 46.5 | 4.0 / 4.3 / 170.2 | 6.624 | — | 0 |
| 3 | 1 | 2.6 / 4.3 / 121.4 | 4.2 / 5.8 / 196.3 | 7.012 | 31.5 / 67.5 / 364.1 | 29878 |
| 3 | 4 | 2.2 / 6.5 / 166.6 | 3.2 / 8.0 / 250.1 | 6.284 | 26.1 / 82.7 / 718.7 | 128664 |
| 3 | 8 | 3.1 / 51.5 / 447.6 | 4.5 / 56.3 / 1344.6 | 14.460 | 41.5 / 131.0 / 5458.5 | 162454 |

Observations:

- With no clients, the latch/publish seam is about 4.7–6.6 µs mean and 1.8–2.5 µs at p50 for
  latch and 2.9–4.1 µs for publish. This is the bank/image copy plus mutex, not network work.
- One to four clients add little at p50 but increase p99 and max, consistent with loopback load
  and wakeup latency rather than a change in the copy path.
- Eight clients raise latch/publish p99 to roughly 46–63 µs and produce multi-millisecond
  outliers. These are host-level scheduling and contention effects; they are not bounded by the
  backend and are not a real-time guarantee.
- Throughput rises with clients (about 30k requests/s at one client to 98k–183k/s at four to
  eight) but is limited by the synthetic per-connection round-robin loop and the loopback stack.
- Process CPU rises from about one core with no clients to about 10–11 cores at eight clients
  (client threads dominate). The cycle thread is not doing network work.

Limits: one Windows x86-64 host; no Linux or AArch64 timing run on 2026-11-02 because WSL was
not running and `ecat-host` was not reachable from the timing workstation. No CI timing gate.
The harness measures the public boundary, not a full coordinator scan, PLC logic, or kernel
network stack. Results are host-specific and indicative only.

## 12. Evidence index

| Evidence | Location |
| --- | --- |
| Backend implementation | [crates/stc-runtime/src/io_modbus_tcp.rs](../../crates/stc-runtime/src/io_modbus_tcp.rs) |
| Registration | [crates/stc-runtime/src/backend_defaults.rs](../../crates/stc-runtime/src/backend_defaults.rs) |
| Profile schema | [crates/stc-manifest/src/profile.rs](../../crates/stc-manifest/src/profile.rs) |
| Runtime unit and integration tests | `io_modbus_tcp.rs` (61 unit tests plus loopback and mixed-coordinator tests) |
| Online-change tests | [crates/stc-runtime/tests/modbus_online_change.rs](../../crates/stc-runtime/tests/modbus_online_change.rs) |
| Cross-host e2e test | [crates/stc-driver/tests/modbus_e2e.rs](../../crates/stc-driver/tests/modbus_e2e.rs) |
| Fixture | [tests/modbus/README.md](../../tests/modbus/README.md) |
| Mixed bench result | [tests/modbus/mixed/RESULTS.md](../../tests/modbus/mixed/RESULTS.md) |
| Overhead harness | [crates/stc-runtime/tests/modbus_overhead.rs](../../crates/stc-runtime/tests/modbus_overhead.rs) |
| Overhead raw runs | [tests/modbus/overhead/windows-x86_64-2026-11-02.txt](../../tests/modbus/overhead/windows-x86_64-2026-11-02.txt) |
| Contract | [IO Routing and Backends](../Architecture/IO-Routing-and-Backends.md) §4.5 |
| Profile schema documentation | [CLI and Manifest](../Architecture/CLI-and-Manifest.md) §2.2.1 |
| Decisions | [Decisions](../Decisions.md) §2, D-229–D-243 |
| Known limitations | [KNOWN-LIMITATIONS.md](../../KNOWN-LIMITATIONS.md) §10 |
| Working plan | Retired after the WL-5.1 pull request merged; the permanent contract is [IO Routing and Backends](../Architecture/IO-Routing-and-Backends.md) §4.5. |
| Parent plan | [V2.0.0-PLAN.md](../../V2.0.0-PLAN.md) §4 |

## 13. Limitations

- **Non-real-time.** The backend is best-effort and not suitable for interpolated motion, hard
  deadlines, timing certification, or safety functions.
- **No authentication, authorization, or encryption.** Any reachable client can write every
  input-mapped coil/register on every bind. Use a trusted/segmented network or host firewall and
  audit the whole bind array. Connection limits and idle timeouts bound resource use but do not
  prevent determined denial of service.
- **BOOL only on bit tables.** A 0/1 flag in a register must be a 16-bit channel; a 1-bit channel
  cannot map to a register table.
- **Zero-based addresses.** The profile never accepts vendor one-based reference numbers.
  Confusing the two is a common integration fault and is called out in the schema, fixture,
  report, and known limitations.
- **Width and span scope.** Only 1/8/16/32/64-bit values are supported. Floating point, strings,
  arbitrary byte swaps, and writes that partially cover a 32/64-bit channel are rejected.
- **Word-order scope.** `high-first`/`low-first` and the named/index permutations cover the
  documented forms only; there is no arbitrary per-register layout.
- **No external Modbus↔EtherCAT cross-wiring evidence.** The mixed mock test proves routing over
  one image; the real bench proves concurrent health of both backends, not a physical value
  transfer between protocols.
- **Independent client not run.** The optional `mbpoll`/`modpoll`/`pymodbus` check was not
  executed; third-party interoperability is not claimed.
- **Single-host overhead.** The timing evidence is one Windows x86-64 machine and is not a
  cross-host or real-time result.
- **Configuration and status surface gaps.** Only `--io` and `--io-file` are parsed; the
  EtherCAT shim path is environment-only (`STC_ETHERCAT_LIBRARY`); the control `Status` JSON has
  no `io_reports`, while the launcher's verbose `stc run -v` output does. The documentation now
  states the implemented behavior and records these gaps; WL-5.1 does not add flags or extend
  the control JSON.

## 14. Conclusion

WL-5.1 is complete. The Modbus TCP server is implemented, registered, documented, and tested on
all three native hosts; its mapping and error behavior fail closed; its cycle-boundary cost is
microseconds on the measured host; and a real EtherCAT + Modbus run shows both backends healthy
in one coordinator. The evidence does not prove real-time behavior, third-party client
interoperability, or physical Modbus-to-EtherCAT cross-wiring, and the report states those
boundaries. The contract is recorded as D-229–D-243.
