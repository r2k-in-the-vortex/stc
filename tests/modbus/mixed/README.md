# Mixed DIEWU EtherCAT + Modbus TCP bench fixture (WL-5.1 phase 4)

This fixture runs the DIEWU EtherCAT-IO 16I/16O slave and the Modbus TCP
server **concurrently in one coordinator**. A Modbus TCP client on the bench
network writes a command word and a coil; the PLC program adds one to the
word and copies the coil, and the client reads the results back. At the same
time every DIEWU input is mirrored to its output and the EtherCAT frames are
accepted in OP. Both backends must report `state=running`, `faults=0`, and
zero faults; the cross-backend data path is not wired.

**Limitation:** no external Modbus-to-EtherCAT signal connection device is
available on the bench. The Modbus and EtherCAT data paths therefore run
independently: Modbus traffic proves the server and coordinator stay live
while the slave is served, but it does not move a value from a Modbus client
to a DIEWU output or vice versa. Cross-backend routing over one process image
is proven by the mixed mock test
`coordinator_serves_modbus_and_ethercat_over_one_image` in
`crates/stc-runtime/src/io_modbus_tcp.rs`. This limitation is recorded in
[RESULTS.md](RESULTS.md) and the phase report.

## Files

| Path | Purpose |
| --- | --- |
| `stc.toml` | Manifest: DIEWU `ethercat` device (16 inputs, 16 outputs), `ethercat-status`, Modbus `modbus-tcp` device (word + coil in, word + coil out), `modbus-tcp-status`, and all bindings. |
| `io/bench.toml` | Machine profile: DIEWU identity/PDO map and one Modbus listener on `0.0.0.0:15030`. No cross-backend routes. |
| `src/main.st` | Mirrors DIEWU inputs to outputs; `BenchSeen := BenchCmd + 1`; `BenchOut := BenchBit`; 1 ms task. |
| `client.py` | Network Modbus client: two write/read rounds, verifies the PLC published `cmd + 1` and the coil copy. |
| `build.sh` | Builds with an existing `stc` (the deployed package; never rebuild stc on the host). |
| `run.sh` | Runs 60000 cycles (60 s) at 1 ms with the CI-built EtherCAT shim and verbose status. |
| `RESULTS.md` | Recorded phase-4 bench run: commands, versions, hashes, status output, client observations, limitation. |

## Process-image map

```
%IX0.0..%IX1.7   In0..In15       DIEWU inputs 0x3001:01/02
%QX0.0..%QX1.7   Out0..Out15     DIEWU outputs 0x3101:01/02
%IW2..%ID8       EtherCAT status master/domain/cycle/slave fields (bytes 4..38)
%IW20            BenchCmd        Modbus holding register 0
%IX42.0          BenchBit        Modbus coil 0
%QW20            BenchSeen       Modbus holding register 1
%QX42.0          BenchOut        Modbus coil 1
%IX48.0..%ID18   Modbus status   server0 fields (bytes 48..75)
```

%IWn is bytes 2n..2n+1 and %IDn is bytes 4n..4n+3. The two status blocks and
the data channels do not overlap.

## Bench prerequisites

- `ecat-host` reachable, `ethercat master` shows `Phase: Idle`, and
  `ethercat slaves` shows the DIEWU slave at position 0 in PREOP.
- A deployed CI package under `~/stc-wl51/` (`bin/stc`, `lib/libstc_runtime.a`,
  `lib/libstc_io_ethercat.so`) from a run that includes the Modbus backend,
  and `/usr/local/etherlab/lib/libethercat.so.1` for the shim.
- `clang` on the host for the user-project build. Do not install packages and
  do not build stc, the runtime, or the shim on the host.
- The build workstation can reach the host's LAN address on TCP 15030.

Deployment follows the [EtherCAT runbook](../../../specification/Verification/EtherCAT-Runbook.md):
download `stc-package-ubuntu-24.04` and `stc-io-ethercat-linux-x86_64`,
verify `sha256sum -c SHA256SUMS`, copy them to `~/stc-wl51/`, and check
`stc --version` and `ldd bin/stc`. This phase used the phase-3 package (run
41488899175); phase 4 changes only tests and documentation.

## Procedure

1. Preflight the master and slave on the host:

   ```sh
   ssh ecat 'ethercat master; ethercat slaves'
   ```

   Stop if the phase is not `Idle` or the slave is not PREOP.

2. Copy this directory (without `target/`) to `~/stc-wl51/mixed/`.

3. Build with the deployed package:

   ```sh
   ssh ecat 'cd ~/stc-wl51/mixed && STC=~/stc-wl51/bin/stc bash build.sh'
   ```

4. Start the timed run and keep the log on the host:

   ```sh
   ssh ecat 'cd ~/stc-wl51/mixed && \
     PATH=$HOME/stc-wl51/bin:$PATH \
     STC_ETHERCAT_LIBRARY=$HOME/stc-wl51/lib/libstc_io_ethercat.so \
     CYCLES=60000 nohup bash run.sh > ~/stc-wl51/logs/mixed-$(date +%Y%m%dT%H%M%S).log 2>&1 & echo $!'
   ```

5. While the run is active, drive the Modbus server from the build
   workstation:

   ```sh
   python3 client.py --host 192.168.1.90
   ```

   The client prints two `ok` lines and `PASS`; it proves command word
   `n` became register 1 value `n + 1`, and the coil was copied, across two
   rounds.

6. Wait for the timed run to exit, then sample the master and slave again:

   ```sh
   ssh ecat 'ethercat master; ethercat slaves'
   ```

   The master must be `Idle` with the same frame-error counters after release,
   and the slave must be PREOP.

## Success criteria

- Both backends appear in the verbose status with `state=running`,
  `faults=0`, and `exchanges=60000`.
- The EtherCAT backend reports `domain.wc_state=2`, `input.stale=0`,
  `master.link_up=1`, `master.slaves_responding=1`, `slave0.online=1`, and
  `slave0.operational=1`; `master.recoveries=0` and `master.last_error=0`.
- The Modbus backend reports one bound listener, `server0.exceptions=0`,
  `server0.malformed=0`, `server0.unbound=0`, `server0.last_error=0`, and
  counters consistent with the client's exchanges.
- `client.py` exits 0 with two `ok` rounds and `PASS`.
- No cross-backend value routing is claimed; the mock test is the routing
  evidence.

Example status lines:

```
stc: status: io-backend ethercat devices=2 state=running exchanges=60000 faults=0 hash=0x... domain.wc_state=2 domain.working_counter=3 input.stale=0 master.last_error=0 master.link_up=1 master.recoveries=0 master.slaves_responding=1 slave0.al_state=8 slave0.online=1 slave0.operational=1
stc: status: io-backend modbus-tcp devices=2 state=running exchanges=60000 faults=0 hash=0x... server0.bind=0.0.0.0:15030 server0.listeners=1 server0.bound_listeners=1 server0.connections=0 server0.exceptions=0 server0.last_error=0 server0.listener0.bind=0.0.0.0:15030 server0.listener0.state=listening server0.malformed=0 server0.peak_connections=1 server0.requests=14 server0.reads=10 server0.writes=4 server0.unbound=0
```

Keep raw logs on the host; commit only RESULTS.md with the commands,
versions, hashes, status excerpt, client output, and the limitation.
