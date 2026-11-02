# Mixed EtherCAT + Modbus TCP bench result (WL-5.1 phase 4)

Date: 2026-11-02, host time. The timed run started at 10:37:51 and completed
60000 requested 1 ms cycles. Both hardware backends ran in one coordinator
while a Modbus TCP client on the bench network drove the server.

## Environment

| Item | Value |
| --- | --- |
| Host | `ecat-host` (Dell Wyse 5070), Linux `6.12.115-ecat-rt` PREEMPT_RT, x86_64 |
| EtherCAT master | IgH EtherCAT master 1.6.13, `/usr/local/etherlab`, master 0 |
| Slave | DIEWU EtherCAT-IO 16I/16O, position 0, alias 0, vendor `0x00000b95`, product `0x00010200`, revision 1 |
| Slave state before/after | PREOP |
| `stc` package | CI run `41488899175` (phase-3 commit `97c592d`), version 1.0.0, `x86_64-unknown-linux-gnu`, host ABI 2.3 |
| `stc --version` | `stc 1.0.0` |
| clang on host | Ubuntu clang 18.1.3 |
| Deployment | CI package unpacked under `~/stc-wl51/`; shim copied to `lib/`; no stc, runtime, or shim build on the host |

Phase 4 changes only tests and documentation, so the phase-3 package contains
the same runtime and driver code as this branch.

## Artifact hashes

```
aad20ee49a21dc488d553e94ede3a6e278666fc916ef6257ce9a893f7b41a3b6  bin/stc
3f32538dd076da0d07b8469fe93c086316df74295ee2ee622972e3bb9fb249dc  lib/libstc_runtime.a
bceeb6014076f41958b0ae4442f744fc3035af3544bcd5a046e278089e5df448  lib/libstc_io_ethercat.so
```

`package.json` reports `"version": "1.0.0"`, `"host": "x86_64-unknown-linux-gnu"`,
and `"host_abi": {"major": 2, "minor": 3}`. `ldd lib/libstc_io_ethercat.so`
resolves `libethercat.so.1` from `/usr/local/etherlab/lib`.

## Commands

Preflight:

```sh
ethercat master
ethercat slaves
```

Build with the deployed package (no rebuild of stc on the host):

```sh
cd ~/stc-wl51/mixed
STC=$HOME/stc-wl51/bin/stc bash build.sh
```

Run 60000 cycles at 1 ms with both backends, keeping the log on the host:

```sh
cd ~/stc-wl51/mixed
PATH=$HOME/stc-wl51/bin:$PATH \
STC_ETHERCAT_LIBRARY=$HOME/stc-wl51/lib/libstc_io_ethercat.so \
CYCLES=60000 nohup bash run.sh > ~/stc-wl51/logs/mixed-20261102T103751.log 2>&1 &
```

While the run was active, from the build workstation:

```sh
python -X utf8 tests/modbus/mixed/client.py \
  --host 192.168.1.90 --port 15030 --timeout 30
```

After the run:

```sh
ethercat master
ethercat slaves
```

## Backend status (verbatim)

```
stc: status: active 0xbef9f6f98901a6a0439b553fbbf8f51b
stc: status: cycles 60000
stc: status: io ethercat
stc: status: io-backend ethercat devices=2 state=running exchanges=60000 faults=0 hash=0xf077ede5bd3bbaafc5eec900fcf15e8442c6363a0c24822a697268890006e5d0 domain.wc_state=2 domain.working_counter=3 input.stale=0 master.last_error=0 master.link_up=1 master.recoveries=0 master.slaves_responding=1
stc: status: io-backend modbus-tcp devices=2 state=running exchanges=60000 faults=0 hash=0xf077ede5bd3bbaafc5eec900fcf15e8442c6363a0c24822a697268890006e5d0 server0.bind=0.0.0.0:15030 server0.bound_listeners=1 server0.connections=0 server0.exceptions=0 server0.last_error=0 server0.listener0.bind=0.0.0.0:15030 server0.listener0.connections=0 server0.listener0.last_error=0 server0.listener0.state=listening server0.listeners=1 server0.malformed=0 server0.peak_connections=1 server0.reads=8 server0.requests=12 server0.unbound=0 server0.writes=4
stc: status: retain disabled policy error
```

Both backends report `running`, `exchanges=60000`, and `faults=0`. The
EtherCAT diagnostics show a complete working counter (`domain.wc_state=2`),
fresh input (`input.stale=0`), link up, one responding slave, no recovery,
and no master error. The Modbus counters match the client: 4 writes, 8 reads,
one peak connection, and zero exception, malformed, unbound, or last-error
counts. No `STC-E-*` error appears; the only message outside the status
block is the expected cold-layout manifest warning for a fixture with no
baseline.

## Client observation

```
ok   command 0x0000 -> indicator 0x0001; bit 1 -> coil 1
ok   command 0x1357 -> indicator 0x1358; bit 0 -> coil 0
PASS mixed EtherCAT + Modbus TCP client check
```

The client connected from the build workstation to `192.168.1.90:15030` over
the bench LAN. Two rounds prove the PLC is scanning: holding register 0 value
`n` was published as register 1 value `n + 1`, and coil 0 was copied to coil
1, with the input values read back unchanged.

## Master counters and release

| Counter | Before | After |
| --- | --- | --- |
| Tx frames | 58757354 | 58819062 |
| Rx frames | 58757353 | 58819061 |
| Tx errors | 0 | 0 |
| Lost frames | 24426 | 24426 |
| Frame loss | 0.0% | 0.0% |

After release the master is `Phase: Idle`, `Active: no`, with link UP, and
the DIEWU slave is PREOP.

## Log files and hashes

Raw logs remain on `ecat-host` under `~/stc-wl51/logs/`:

```
c940556e795355af1ac1a7c3c6c21c27acb64e7f5f57704427479447bbf68085  mixed-20261102T103751.log
c2066fcf0ca4b6fee3961baf1e855fe298701f46c7b19f2c970d1618cc840e9a  mixed-20261102T103751-master-before.txt
e937d8e0b45aef1b72d0de4d235a11376bbe4c9aaec763d858704b643e24da9a  mixed-20261102T103751-master-after.txt
4f94d5060a0386cc9a684443117919cd69b82ea5aeaf55a45de6cc94c1ccdbe0  mixed-20261102T103751-slaves-before.txt
4f94d5060a0386cc9a684443117919cd69b82ea5aeaf55a45de6cc94c1ccdbe0  mixed-20261102T103751-slaves-after.txt
```

## Limitation: no Modbus-to-EtherCAT cross-wiring

No external device connects a Modbus client signal to a DIEWU channel. The
two data paths therefore ran independently: the network client exercised the
Modbus server, and the PLC mirrored DIEWU inputs to outputs while IgH accepted
the frames. This run does not prove an end-to-end Modbus-client-to-EtherCAT-
output (or EtherCAT-input-to-Modbus-client) value transfer.

Cross-backend routing over one process image is proven by the mixed mock test
`coordinator_serves_modbus_and_ethercat_over_one_image` in
`crates/stc-runtime/src/io_modbus_tcp.rs`, as recorded in the phase plan
(M-14, Q-10). This run proves the two real backends are concurrently healthy
in one coordinator.

## Independent third-party client

`mbpoll`, `modpoll`, and `pymodbus` are not installed on the bench host or
the build workstation. The optional independent-client check was therefore
not run; all client observations above use the in-repo stdlib client.

## Conclusion

The phase-4 real mixed bench criterion passes: DIEWU EtherCAT and Modbus TCP
were active concurrently in one coordinator, both backend reports are
healthy, a network Modbus client completed two command/indicator rounds, the
master and slave returned cleanly to Idle/PREOP, and the no-cross-wiring
limitation is recorded.
