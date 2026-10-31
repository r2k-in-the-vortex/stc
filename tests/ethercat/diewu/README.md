# DIEWU open-ended EtherCAT HIL fixture (WL-5.2 phase 3)

This fixture brings up a DIEWU EtherCAT-IO 16I/16O slave in OP on the
`ecat-host` bench and confirms at frame level that IgH accepts and replies to
every process-data frame. It is deliberately open-ended: the terminal blocks
are unwired/unloaded, the program only copies inputs to outputs and advances a
counter, and no physical input value or output measurement is asserted.

Phase 3 does **not** cover cable unplug, power-cycle, loopback harnesses, fault
injection, or any manual toggling/measuring. Those belong to phase 6.

## Bench

- Host: `ecat-host` (Dell Wyse 5070), Ubuntu 24.04, `6.12.115-ecat-rt`
  PREEMPT_RT kernel.
- Master: IgH EtherCAT master 1.6.13 at `/usr/local/etherlab`, device
  `/dev/EtherCAT0`, master 0; user `r2k` can open the device without sudo.
- Slave: DIEWU 16I/16O at position 0, alias 0.
  Vendor `0x00000b95`, product `0x00010200`, revision 1.
- DC: not configured. The SII has no `<Dc>` section; free-run is the owner
  decision (WL-5.2-Plan.md E-17).

## Files

| Path | Purpose |
| --- | --- |
| `stc.toml` | Manifest: 16 BOOL inputs, 16 BOOL outputs, `status` device (`ethercat-status`, layout 1), direct bindings, `[io].config = "io/bench.toml"`. |
| `io/bench.toml` | Machine profile: DIEWU identity, SM0/SM1 output PDOs `0x1a00`/`0x1a01` on `0x3101:01`/`0x3101:02`, SM2 input PDOs `0x1600`/`0x1601` on `0x3001:01`/`0x3001:02`, the 32-bit channel map, and three routes. |
| `src/main.st` | Non-CPU-bound program: `Out0..Out13 := In0..In13`, `In14` toggles a state bit, `Counter` increments; 1 ms task. |
| `build.sh` | Builds the fixture with the deployed `stc` package. |
| `run-open-ended.sh` | Runs 600000 cycles at 1 ms (10 minutes) with `-v` status output. |

## Process-image map

```
%IX0.0..%IX0.7  In0..In7     0x3001:01 bits 0..7
%IX1.0..%IX1.7  In8..In15    0x3001:02 bits 0..7
%IX2.0          Shadow1      sink of the buffered PLC -> PLC route
%QX0.0..%QX0.7  Out0..Out7   0x3101:01 bits 0..7
%QX1.0..%QX1.5  Out8..Out13  0x3101:02 bits 0..5
OUT14 (0x3101:02 bit 6)      route Out0 -> OUT14
OUT15 (0x3101:02 bit 7)      route IN15 -> OUT15 (physical pass-through)
```

Status variables occupy `%I` bytes 4..38 (`%IWn` is word n at byte 2n, `%IDn`
is dword n at byte 4n). The `ethercat-status` device fills them at the input
boundary from the live master/domain state; they are read-only.

## Deploying with the CI-built package

Never build `stc` on the bench host. Download the CI artifacts
`stc-package-ubuntu-24.04` and `stc-io-ethercat-linux-x86_64`, then install
them under a dedicated directory:

```sh
install -d "$HOME/stc-wl52/bin" "$HOME/stc-wl52/lib" "$HOME/stc-wl52/logs"
tar -C "$HOME/stc-wl52" -xzf stc-package-ubuntu-24.04.tar.gz
install -m 0755 libstc_io_ethercat.so "$HOME/stc-wl52/lib/"
"$HOME/stc-wl52/bin/stc" --version
```

Copy this fixture directory (no `target/`) to the host, for example
`~/stc-wl52/diewu`.

`clang` must be available on the host for the fixture's user-project build
(`clang --version`). If it is missing, stop: do not install packages. The
fixture itself is built with the deployed `stc`, not by rebuilding the
compiler or runtime.

## Procedure

1. Confirm the master is idle and the slave is present:

   ```sh
   ethercat master
   ethercat slaves
   ```

   `ethercat master` must show `Phase: Idle` before the run; if another
   application holds the master, stop and report.

2. Export the shim and IgH library paths:

   ```sh
   export STC_ETHERCAT_LIBRARY="$HOME/stc-wl52/lib/libstc_io_ethercat.so"
   export LD_LIBRARY_PATH="/usr/local/etherlab/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
   export PATH="$HOME/stc-wl52/bin:$PATH"
   ```

3. Build the fixture once:

   ```sh
   bash ~/stc-wl52/diewu/build.sh
   ```

4. Sample the master counters, then run the 10-minute open-ended cycle in the
   foreground and keep the log on the host:

   ```sh
   ethercat master > ~/stc-wl52/logs/master-before.txt
   bash ~/stc-wl52/diewu/run-open-ended.sh \
     2>&1 | tee ~/stc-wl52/logs/open-ended-$(date +%Y%m%dT%H%M%S).log
   ethercat master > ~/stc-wl52/logs/master-after.txt
   ```

   The script passes `--io ethercat`, `--cycles 600000`, `--cycle-time 1ms`,
   and `-v` (exit status summary). It does not pass `--rt`.

## Frame-level success criteria

The run succeeds when all of the following hold; physical I/O values are not
verified:

- The slave reaches OP (`ethercat slaves` during the run, and
  `slave0.operational=1`, `slave0.al_state=8` in the status output).
- The runtime stays healthy: `state=running`, `faults=0`,
  `domain.wc_state=2` (WC complete), and `exchanges=600000` at exit.
- No send/receive failures or startup errors in the log
  (`STC-E-ETH-*`, `send`, `receive`); no recovery is triggered
  (`master.recoveries=0`, `input.stale=0`).
- `ethercat master` after the run shows the same lost-frame and Tx-error
  counters as before (frame-level health; sample both, do not reset).
- The process exits after the cycle count and release does not rewrite the
  output image (the backend never writes outputs during shutdown).

The `-v` summary line looks like:

```
stc: status: io-backend ethercat devices=2 state=running exchanges=600000 faults=0 hash=0x... master.slaves_responding=1 master.link_up=1 domain.wc_state=2 domain.working_counter=... input.stale=0 master.recoveries=0 master.last_error=0
```

Keep the raw logs in `~/stc-wl52/logs/` for the run report; do not commit
them. Commit only the concise summary and the runbook procedure, with log
paths and hashes.

## Troubleshooting

- `STC_ETHERCAT_LIBRARY` unset or wrong: the runtime fails with an EtherCAT
  backend activation error; check `stc: status: io-backend ... last-fault=`.
- Shim cannot find `ecrt_*`: add `/usr/local/etherlab/lib` to
  `LD_LIBRARY_PATH`.
- `/dev/EtherCAT0` permission denied: stop and report; do not use sudo.
- `ethercat master` cannot open the master: another application is using it;
  stop and report instead of killing it.
- The slave never reaches OP or frames are rejected: capture the full log and
  `ethercat master`/`ethercat slaves` output, then diagnose the fixture or
  runtime. Do not power-cycle, unplug cables, or toggle I/O.
