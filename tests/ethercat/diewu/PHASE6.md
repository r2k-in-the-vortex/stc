# DIEWU loopback and physical fault procedure (WL-5.2 phase 6)

This is the manual procedure for the phase-6 loopback matrix on `ecat-host`.
It is written to be executed by the owner or technician; the automated
preparation commit adds the test program, harness, and this checklist. It was
executed on 2026-11-01 (host log time) with a second 24 V supply on the output
stage: §4 passed 16/16, Faults A and B were executed on the fixed package,
Fault C is covered by Fault A, and the restore state was captured. An earlier
supply-limited attempt found a coordinator stale-input defect; the fix and the
final results are in §8. The fault-session counters are cumulative and the
final run recorded one extra, unobserved fault episode before the first held
baseline; the deviation is recorded in §8.2.

The procedure covers:

1. Wiring confirmation and terminal/channel mapping (§2).
2. Deployment, preflight, and the automated all-points matrix (§3-§4).
3. Physical fault injection: cable unplug/replug, DIEWU power cycle, and
   configured-slave reconnect (§5-§6).
4. Evidence capture (§7-§8) and restore/cleanup (§9).

Nothing in this procedure is a safety function. The loopback verifies only
that each DIEWU output driver can turn on the corresponding input opto. It
does not verify field devices, output load capacity, input threshold margins,
isolation, cable/connector integrity beyond the bench harness, or a fail-safe
output state. Physical output behavior is verified only through the loopback;
do not connect field loads to this bench.

## 1. Preconditions

- `ecat-host` runs the baseline host in
  [EtherCAT-Runbook.md](../../../specification/Verification/EtherCAT-Runbook.md)
  §2: IgH EtherCAT master 1.6.13, PREEMPT_RT kernel `6.12.115-ecat-rt`, the
  DIEWU 16I/16O slave at position 0/alias 0, and the login user `r2k` able to
  open `/dev/EtherCAT0` without sudo.
- The CI-built `stc` package and `libstc_io_ethercat.so` are deployed under
  `~/stc-wl52` as in [README.md](README.md). Do not rebuild `stc` on the
  host. `clang` must be available for the user-project build.
- The loopback directory `tests/ethercat/diewu/loopback/` is copied to the
  host, for example `~/stc-wl52/diewu/loopback/`, with no `target/` directory.
- The bench is unloaded except for the loopback harness. No field device,
  relay, solenoid, or meter is connected.
- Use one SSH session for each run or debug session. A second terminal may be
  used only for the read-only `ethercat master`/`ethercat slaves` samples
  listed below, and it must not run other commands while a run is active.
- Do not use root, `sudo`, `--rt`, or a rebuilt compiler/runtime. Stop and
  report if any of these appear to be required.

Record before starting: package version, package/shim hashes, host kernel,
IgH version, and the supply voltage selected for the harness.

## 2. Wiring confirmation and terminal/channel mapping

### 2.1 Terms

- **Fixture channel N** is the software channel: `IN0`/`OUT0` through
  `IN15`/`OUT15` in `stc.toml`. Channel 0 is terminal 1 and channel 15 is
  terminal 16.
- The DIEWU terminals are labelled `IN1..IN16` and `OUT1..OUT16`.
- The DIEWU outputs are NPN sinking. An output that is ON pulls its terminal
  low. The input opto is the load between the input common (S) and the input
  terminal (IN). With S tied to the positive output supply, current flows from
  S through the opto into a sinking output when that output is ON, so a
  driven output reads back as a set input.

### 2.2 Terminal/channel table

Wire each output to the input with the same terminal number:

| Fixture channel | Output terminal | Input terminal | Wire |
| --- | --- | --- | --- |
| 0 | OUT1 | IN1 | OUT1 -> IN1 |
| 1 | OUT2 | IN2 | OUT2 -> IN2 |
| 2 | OUT3 | IN3 | OUT3 -> IN3 |
| 3 | OUT4 | IN4 | OUT4 -> IN4 |
| 4 | OUT5 | IN5 | OUT5 -> IN5 |
| 5 | OUT6 | IN6 | OUT6 -> IN6 |
| 6 | OUT7 | IN7 | OUT7 -> IN7 |
| 7 | OUT8 | IN8 | OUT8 -> IN8 |
| 8 | OUT9 | IN9 | OUT9 -> IN9 |
| 9 | OUT10 | IN10 | OUT10 -> IN10 |
| 10 | OUT11 | IN11 | OUT11 -> IN11 |
| 11 | OUT12 | IN12 | OUT12 -> IN12 |
| 12 | OUT13 | IN13 | OUT13 -> IN13 |
| 13 | OUT14 | IN14 | OUT14 -> IN14 |
| 14 | OUT15 | IN15 | OUT15 -> IN15 |
| 15 | OUT16 | IN16 | OUT16 -> IN16 |

### 2.3 Supply and commons

1. Power the module from a single supply that feeds the module's output
   supply terminals. The owner-confirmed harness uses 12 V or 24 V; both are
   inside the output DC5-48 V range and the input 9-30 V range.
2. Tie input commons `S1`, `S2`, `S3`, and `S4` to the same supply positive
   as the output supply (the +V that feeds the module). Do not use a second
   supply with a different common.
3. Do not connect any external load to the outputs. The input opto is the only
   load in this harness.
4. With the supply off, check each of the 16 wires for the terminal pair in
   §2.2 and check that S1-S4 go only to +V. Check for loose strands and
   shorts. Then apply power.

Record in §8: supply voltage, commons tied to +V (yes/no), and each wire
checked. If the harness is reused from the phase-5 work, record that it was
inspected again before this procedure.

### 2.4 Wiring limitations to state in the report

- The harness does not test output load capacity, input threshold margins,
  isolation, or any field device.
- The DIEWU output watchdog is enabled. During a link or power fault the
  slave turns its physical outputs off even though the runtime keeps the last
  PLC output image. The physical terminals are therefore not evidence of the
  runtime's output behavior during the fault; the runtime image is observed
  through `var.read` in §6.

## 3. Deployment and preflight

On `ecat-host` as the login user:

```sh
export STC_ETHERCAT_LIBRARY="$HOME/stc-wl52/lib/libstc_io_ethercat.so"
export LD_LIBRARY_PATH="/usr/local/etherlab/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PATH="$HOME/stc-wl52/bin:$PATH"
export LOGS="$HOME/stc-wl52/logs"
mkdir -p "$LOGS"

"$STC" --version
ethercat master > "$LOGS/master-phase6-before.txt"
ethercat slaves
```

Stop if `ethercat master` does not show `Phase: Idle` before the run: another
application holds the master. Stop if the DIEWU slave is not listed at
position 0. Do not kill another application.

Build the loopback project once (the harness also builds, but this separates
a compile error from the run):

```sh
bash "$HOME/stc-wl52/diewu/loopback/build.sh"
sha256sum "$HOME/stc-wl52/diewu/loopback/stc.toml" \
          "$HOME/stc-wl52/diewu/loopback/io/bench.toml" \
          "$HOME/stc-wl52/diewu/loopback/src/main.st" \
          "$HOME/stc-wl52/diewu/loopback/loopback_test.py" \
          "$HOME/stc-wl52/diewu/loopback/build.sh" \
          "$HOME/stc-wl52/diewu/loopback/run-loopback.sh" \
          "$HOME/stc-wl52/diewu/loopback/fault-session.sh" \
          > "$LOGS/loopback-phase6-files.sha256"
```

## 4. Automated all-points matrix

### 4.1 Command

From one SSH session, with no other host commands running:

```sh
set -o pipefail
bash "$HOME/stc-wl52/diewu/loopback/run-loopback.sh" \
  2>&1 | tee "$LOGS/loopback-$(date +%Y%m%dT%H%M%S).log"
```

The harness prints the `stc` version, the shim path, the build/session
identity, the 16-channel matrix, the masks, and the status snapshot. It exits
0 only when all 16 channels passed, `FailMask=0`, `faults=0`,
`input.stale=false`, `wc_state=2`, the link is up, and the slave is
operational.

### 4.2 Expected output

On a correctly wired and healthy bench the matrix is all `PASS`, the
`ObservedMask` is `0xFFFF`, and the summary is:

```
Pass:          True
FailMask:      0x0000
ObservedMask:  0xFFFF
OutputMask:    0x0000 (outputs are off after the walk)
Channel:       16 (16 means every channel was sampled)
Status:        faults=0 stale=false recoveries=0 wc_state=2 ... link_up=true
               master_phase=2 ... slave_al_state=8 slave_online=true
               slave_operational=true ...
Acceptance:    PASS (16/16 channels; fieldbus healthy)
```

`slave_al_state=8` is OP, `wc_state=2` is WC complete, and `master_phase=2`
is IgH Operation phase.

### 4.3 Reading Pass and FailMask

- `Pass` is the final result: true only when `FailMask` is zero after all 16
  channels were sampled.
- `FailMask` is a 16-bit mask. Bit N (0 = `0x0001`, 15 = `0x8000`) is set when
  channel N failed. Channel N is terminal OUT(N+1)/IN(N+1).
- `ObservedMask` bit N is set when channel N's own input read 1 at its sample.
  A dead output shows `ObservedMask` bit N clear; a stuck-high or miswired
  input can fail a different channel's sample and shows in the `InputMask` of
  that sample.
- The matrix prints the same information per channel. A failing row names the
  output and input terminals to check.

### 4.4 If the run fails

Do not change wiring while a run is active. Keep the log. Stop, power the
bench off if a wiring check is needed, and inspect the named terminal pair.
A fail with `faults != 0`, `input.stale=true`, or `wc_state != 2` is a
fieldbus event, not a channel failure: recheck the master and slave state and
the deployed shim before rerunning. Record the first failure; do not silently
rerun until it passes.

After the run:

```sh
ethercat master > "$LOGS/master-phase6-after-loopback.txt"
ethercat slaves
```

The master must return to `Phase: Idle` and the slave to PREOP after release.
The harness closes the session and shuts the host down; do not kill processes.

Record the log path, exit code, `Pass`, `FailMask`, `ObservedMask`, and the
status values in §8.

> **Supply-limited variant (2026-11-01).** If the output-stage supply is
> unavailable, skip §4. The input loop is unpowered, so `INPUTMASK` is `0` at
> the baseline and during faults; use `OUTPUTMASK`, the status fields, and
> `SCANCOUNT` as the evidence. Record the skipped all-points matrix and the
> unverified physical I/O in §8. Do not treat this variant as a pass of §4.
> The final 2026-11-01 run used a second 24 V supply on the output stage and
> §4 passed 16/16; this variant is retained for benches without that supply.

## 5. Physical fault injection

All three cases use the same observation method and the same `Hold` mode.
Run them after the all-points matrix passed with `faults=0`; the final
2026-11-01 run used the powered harness and expected `INPUTMASK=65535` while
held. With no output-stage supply, use the supply-limited variant above and
expect `INPUTMASK=0`.

### 5.1 Observation method

Start one interactive debug session and keep it for all three faults:

```sh
bash "$HOME/stc-wl52/diewu/loopback/fault-session.sh" \
  2>&1 | tee "$LOGS/fault-session-$(date +%Y%m%dT%H%M%S).log"
```

`fault-session.sh` starts `stc debug` on the loopback project with
`--io ethercat --cycles 0 --cycle-time 1ms --allow-write`. The write scope is
used only for the `Hold` flag. The session prints the non-safety notice and a
`stc debug>` prompt; type the commands below at that prompt.

Useful commands:

| Command | Meaning |
| --- | --- |
| `print DIEWU-LOOPBACK::DONE` | True after the all-points walk finished. |
| `print DIEWU-LOOPBACK::STATUSFAULTS` | Fault episodes since start. |
| `print DIEWU-LOOPBACK::STATUSINPUTSTALE` | Inputs are the last received image. |
| `print DIEWU-LOOPBACK::STATUSRECOVERIES` | Successful recovery count. |
| `print DIEWU-LOOPBACK::STATUSWCSTATE` | Domain working-counter state (2 = complete). |
| `print DIEWU-LOOPBACK::STATUSWORKINGCOUNTER` | Last working counter (3 expected). |
| `print DIEWU-LOOPBACK::STATUSLINKUP` | Master link state. |
| `print DIEWU-LOOPBACK::STATUSPHASE` | IgH master phase (2 = Operation). |
| `print DIEWU-LOOPBACK::STATUSSLAVEALSTATE` | Slave AL state (8 = OP when healthy). |
| `print DIEWU-LOOPBACK::STATUSSLAVEONLINE` | Slave online flag. |
| `print DIEWU-LOOPBACK::STATUSSLAVEOPERATIONAL` | Slave operational flag. |
| `print DIEWU-LOOPBACK::SCANCOUNT` | The PLC scan count; it must keep advancing. |
| `print DIEWU-LOOPBACK::OUTPUTMASK` | 16 outputs the PLC writes (0xFFFF in Hold). |
| `print DIEWU-LOOPBACK::INPUTMASK` | Last received input image. |
| `set DIEWU-LOOPBACK::HOLD TRUE` | Enter fault-hold mode (outputs all on). |
| `set DIEWU-LOOPBACK::HOLD FALSE` | Leave hold mode (outputs all off). |
| `quit` | End the session and release the master. |

`set` first prints the current value, then a queued `var.write` status; the
write is applied at the next cycle boundary and a queue-result event follows.
Wait about one second before printing the result. Values are printed as
`stc: NAME = value`.

The debug session's default stop/idle budget is five minutes; the cycle is
not stopped during these reads, so an active session does not consume it. If
the session ends, relaunch it: the host restarts and the fault counters reset,
so redo the baseline and treat the interrupted attempt as incomplete.

### 5.2 Baseline (before any fault)

Wait for the all-points walk to finish, then hold all outputs on:

```text
print DIEWU-LOOPBACK::DONE
set DIEWU-LOOPBACK::HOLD TRUE
print DIEWU-LOOPBACK::OUTPUTMASK
print DIEWU-LOOPBACK::INPUTMASK
print DIEWU-LOOPBACK::STATUSFAULTS
print DIEWU-LOOPBACK::STATUSINPUTSTALE
print DIEWU-LOOPBACK::STATUSRECOVERIES
print DIEWU-LOOPBACK::STATUSWCSTATE
print DIEWU-LOOPBACK::STATUSLINKUP
print DIEWU-LOOPBACK::STATUSSLAVEALSTATE
print DIEWU-LOOPBACK::STATUSSLAVEOPERATIONAL
print DIEWU-LOOPBACK::SCANCOUNT
```

Expected baseline: `DONE=true`, `OUTPUTMASK=65535`, `INPUTMASK=65535`
(`INPUTMASK=0` in the supply-limited variant), `STATUSFAULTS=0`,
`STATUSINPUTSTALE=false`, `STATUSRECOVERIES=0`,
`STATUSWCSTATE=2`, `STATUSLINKUP=true`, `STATUSSLAVEALSTATE=8`,
`STATUSSLAVEOPERATIONAL=true`, and `SCANCOUNT` advancing. If any value differs,
do not inject a fault: record the state and stop.

### 5.3 Fault A: cable unplug/replug (host-side link loss)

1. With Hold on and the baseline recorded, unplug the EtherCAT cable from the
   host NIC (the cable between the host and the DIEWU IN port). Leave the
   slave powered.
2. Wait 5 seconds, then type:

   ```text
   print DIEWU-LOOPBACK::STATUSFAULTS
   print DIEWU-LOOPBACK::STATUSINPUTSTALE
   print DIEWU-LOOPBACK::STATUSRECOVERIES
   print DIEWU-LOOPBACK::STATUSWCSTATE
   print DIEWU-LOOPBACK::STATUSLINKUP
   print DIEWU-LOOPBACK::STATUSSLAVEALSTATE
   print DIEWU-LOOPBACK::STATUSSLAVEOPERATIONAL
   print DIEWU-LOOPBACK::SCANCOUNT
   print DIEWU-LOOPBACK::OUTPUTMASK
   print DIEWU-LOOPBACK::INPUTMASK
   ```

   Expected during the fault: `STATUSFAULTS` is at least 1 (one episode),
   `STATUSINPUTSTALE=true`, `STATUSRECOVERIES` unchanged, `STATUSWCSTATE`
   not 2 (record 0 or 1), `STATUSLINKUP=false`, `STATUSSLAVEALSTATE` not 8
   (record the observed value), `STATUSSLAVEOPERATIONAL=false`, `SCANCOUNT`
   has advanced since the baseline (the PLC keeps running), `OUTPUTMASK=65535`
   (the runtime image is unchanged), and `INPUTMASK=65535` (`0` when the
   input loop is unpowered; the stale flag is then the input evidence).

3. While still faulted, demonstrate the stale input image: type
   `set DIEWU-LOOPBACK::HOLD FALSE`, wait one second, then print
   `OUTPUTMASK` and `INPUTMASK`. Expect `OUTPUTMASK=0` (the PLC now drives all
   outputs off) while `INPUTMASK` is still `65535` (`0` in the supply-limited
   variant; the slave cannot receive the new outputs and the runtime presents
   the last received image).
4. Replug the cable. Wait up to 10 seconds, then print the same status
   fields. Expected after recovery: `STATUSLINKUP=true`,
   `STATUSSLAVEOPERATIONAL=true`, `STATUSSLAVEALSTATE=8`,
   `STATUSWCSTATE=2`, `STATUSINPUTSTALE=false`, `STATUSRECOVERIES` increased
   by one, `STATUSFAULTS` unchanged at its fault-time value, `SCANCOUNT`
   still advancing, `OUTPUTMASK=0`, and `INPUTMASK=0` (the off outputs reached
   the slave after recovery).
5. If the link or slave has not recovered within 30 seconds, stop and report;
   do not power-cycle the host or kill the runtime.

Record the observed values and the recovery time in §8.

### 5.4 Fault B: DIEWU power cycle

1. Return to the baseline with the link healthy: confirm `STATUSINPUTSTALE`
   is false, `STATUSSLAVEOPERATIONAL` is true, and `STATUSWCSTATE=2`.
   Type `set DIEWU-LOOPBACK::HOLD TRUE`, wait one second, and confirm
   `OUTPUTMASK=65535` and `INPUTMASK=65535` (`0` in the supply-limited
   variant).
2. Switch off the DIEWU supply. Wait 5 seconds, then print the status fields
   in §5.3 step 2. Expected: a new fault episode (`STATUSFAULTS` increases by
   one), `STATUSINPUTSTALE=true`, `STATUSLINKUP=false`, the slave not
   operational, `SCANCOUNT` advancing, `OUTPUTMASK=65535`, `INPUTMASK=65535`
   stale. `STATUSRECOVERIES` is unchanged until recovery.
3. Switch the supply back on. The slave boots and IgH must reconfigure it as
   the same configured slave; no cold download and no host restart is allowed.
   Wait up to 30 seconds and print the status fields. Expected after recovery:
   link up, slave online and operational with AL state 8, `STATUSWCSTATE=2`,
   `STATUSINPUTSTALE=false`, `STATUSRECOVERIES` increased by one,
   `SCANCOUNT` advancing, and `OUTPUTMASK=65535` with `INPUTMASK` returning to
   `65535` (`0` in the supply-limited variant; the held outputs resumed at the
   slave, observed through the output LEDs).
4. Type `set DIEWU-LOOPBACK::HOLD FALSE`, wait one second, and confirm
   `OUTPUTMASK=0` and `INPUTMASK=0`.

Record the observed values and the recovery time. A slave that stays
non-operational or a master that stays faulted after 30 seconds is a failure:
stop, keep the log, and report.

### 5.5 Fault C: configured-slave reconnect (slave-side link loss)

This case disconnects the same configured slave at the DIEWU IN port while the
slave stays powered, then reconnects it. It is the hot-swap path D-216 allows;
adding or removing a device would be a topology change and is out of scope.

> **2026-11-01 execution note.** Fault C was not run separately. With a single
> direct cable between the host NIC and the DIEWU IN port, unplugging either
> end produces the same slave-side link loss, and Fault A (host-end unplug)
> already reconnected the same configured slave while it stayed powered. The
> §8.2 Fault C column records that this case is covered by Fault A.

1. Return to the healthy baseline with Hold on (`OUTPUTMASK=65535`,
   `INPUTMASK=65535`, `STATUSINPUTSTALE=false`, `STATUSWCSTATE=2`).
2. Unplug the EtherCAT cable at the DIEWU IN port. Wait 5 seconds and print
   the status fields in §5.3 step 2. Expected: a new fault episode,
   `STATUSINPUTSTALE=true`, link down, slave not operational, `SCANCOUNT`
   advancing, masks held at `65535`.
3. Replug the cable at the DIEWU IN port. Wait up to 30 seconds and print the
   status fields. Expected after recovery: link up, slave online and
   operational with AL state 8, `STATUSWCSTATE=2`, `STATUSINPUTSTALE=false`,
   `STATUSRECOVERIES` increased by one, `SCANCOUNT` advancing, masks
   `65535`.
4. Type `set DIEWU-LOOPBACK::HOLD FALSE`, wait one second, confirm
   `OUTPUTMASK=0` and `INPUTMASK=0`, then type `quit`.

Record the observed values and the recovery time.

### 5.6 What "outputs unchanged" means here

D-216 says the master never rewrites the data it exchanges: on a fault it keeps
sending the PLC's output values unchanged and keeps presenting the last
received inputs as stale. In this procedure the runtime image is observed
through `OUTPUTMASK` (the PLC values the program drives) and the status
registers; `SCANCOUNT` must keep advancing, proving the cycle and the
application did not exit. The physical output terminals may turn off while
the slave has no frames (its output watchdog is enabled), so do not use a
meter on the terminals as evidence that the runtime held the image. The
loopback observation after recovery (inputs returning to the held pattern, or
to zero after `HOLD FALSE`, without a cold download or a program restart)
confirms that the same output image was sent again when communication
returned.

## 6. Restore the master, slave, and service state

After the fault session:

```sh
ethercat master > "$LOGS/master-phase6-after-faults.txt"
ethercat slaves
```

Expected restore state:

- `ethercat master`: `Phase: Idle`, `Active: no`; the Tx-error and lost-frame
  counters must be recorded before and after; the harness does not reset them.
- `ethercat slaves`: the DIEWU is present and PREOP after release, not OP.
- No `stc` process remains; the runtime released the master when the session
  ended. Do not kill processes or stop the `ethercat` service as part of this
  procedure.
- The EtherCAT master service remains running and enabled, as it was before
  the procedure. Phase 5 already exercised the service stop/start path; phase
  6 does not repeat it.
- Switch off the bench supply if the bench is being left unattended. Record
  whether the harness remains connected for the next session.

If the master is not Idle or the slave is not PREOP, keep the session log and
the master output, stop, and report. Do not use sudo or restart the service
without the owner's decision.

## 7. Evidence capture

Keep raw logs on `ecat-host`; do not commit them. Commit only the concise
summary and hashes in the runbook/report.

| Evidence | Path on `ecat-host` |
| --- | --- |
| All-points matrix log | `~/stc-wl52/logs/loopback-<timestamp>.log` |
| Fault session transcript | `~/stc-wl52/logs/fault-session-<timestamp>.log` |
| Master before the runs | `~/stc-wl52/logs/master-phase6-before.txt` |
| Master after the all-points run | `~/stc-wl52/logs/master-phase6-after-loopback.txt` |
| Master after the fault session | `~/stc-wl52/logs/master-phase6-after-faults.txt` |
| Loopback project file hashes | `~/stc-wl52/logs/loopback-phase6-files.sha256` |
| Log hashes | `sha256sum` of each log, recorded in the summary |

Also record the package `stc --version`, the package/shim SHA256 values from
the deployment, `uname -r`, and the IgH version (`ethercat version`). The
all-points log contains the build ID and generation printed by the harness;
the fault session log contains the status values in command order.

## 8. Results

Final execution: 2026-11-01 host log time on `ecat-host`, with the module
logic on the 24 V barrel jack and a second 24 V supply on the output stage.
The all-points matrix passed 16/16 and the powered fault matrix passed on the
fixed package. An earlier supply-limited attempt (without the output-stage
supply) found a coordinator stale-input defect, fixed in `d694266`; the final
run verifies the fix. Raw logs remain on the host; only the summary and hashes
are recorded here.

### 8.1 Bench and wiring

| Item | Record |
| --- | --- |
| Date/time and operator | 2026-11-01 host time (all-points log 17:49:24; fault session 17:49:58–~17:53:25); owner/operator at login `r2k` |
| Host, kernel, IgH version | `ecat-host`, kernel `6.12.115-ecat-rt` PREEMPT_RT, IgH 1.6.13, CPU 3 isolated |
| Package version and package/shim SHA256 | `stc 1.0.0`; fixed package: `bin/stc` `f5c506487f996cb44c4807ba5c36f333d7bc192425611a65ceec79caf2ffc644`, `lib/libstc_runtime.so` `3e73b73d244f9489f2bd1751bfa97004763c2d2c5a5a8212b27b54f72e7b2dab`, `lib/libstc_io_ethercat.so` `bceeb6014076f41958b0ae4442f744fc3035af3544bcd5a046e278089e5df448`. Debug session build `0x672c20bf1bc31fc0c76cbb939183e310`. |
| Supply voltage (12 V or 24 V) | Module logic from the 24 V barrel jack; output stage from a second 24 V supply, with S1–S4 tied to its `VCC`. Both final runs used this. |
| S1-S4 tied to supply positive | Yes, to the output-stage `VCC`; the second supply was powered. |
| All 16 wires checked against §2.2 | Owner confirmed the 16 OUTn→INn wires and S1–S4 commons; all 16 channels passed the automated matrix. |
| Faults=0 and stale=false at the held baseline | Initial pre-HOLD block: `STATUSFAULTS=0`, `STATUSINPUTSTALE=false`, `STATUSRECOVERIES=0`, WC state 2, working counter 3, link up, AL 8, `OUTPUTMASK=0`, `INPUTMASK=0`. The first post-HOLD block showed `faults=1`/`recoveries=1` after an unobserved episode; see §8.2. |
| All-points log path and SHA256 | `/home/r2k/stc-wl52/logs/loopback-fix-20261101T174924.log` — `06d295c5f45c4fcaf468d365bc98873e3164a5c1e4da1818aa3c7fc131e85b41` |
| All-points exit code | 0 |
| All-points `Pass` / `FailMask` / `ObservedMask` | `True` / `0x0000` / `0xFFFF`; `OutputMask=0`, `Channel=16`, `Scans=390`, `exchanges=401` |
| All-points `faults` / `stale` / `wc_state` / `slave_al_state` | 0 / false / 2 / 8 |
| All-points master counters | Tx errors 0; lost frames 18856 → 18856 (delta 0) |
| Fault-session log and master counters | `/home/r2k/stc-wl52/logs/fault-session-fix-20261101T174958.log` — `14d7689bed749aa12770196722ed8c6e696e0cff7d9afe3e305717cd00b3de6e`; Tx errors 0; lost frames 18856 → 24426 (delta 5570) |

The loopback project blobs at `58719ac` used by the session were:

| File | SHA256 |
| --- | --- |
| `stc.toml` | `6c85941e902d254652b4f34d412a9f585e66d4d365fd3c6a10bd742e05afd98d` |
| `io/bench.toml` | `8208ffaf3b581adcd3cbe5444bcaeae5165cbbb522c0e21bf0c934d204a2aa85` |
| `src/main.st` | `7dccdabd61492b70f5ded14d16b7e3c3839f9846fde85e48ecc7bf10d135a049` |
| `loopback_test.py` | `4d77830dd15b5669cab11f9380d9f76525778137034c7ce7bf2c9f2170c5ee50` |
| `build.sh` | `67ad1076fe49825d5e61e2cfeb738edcc4a07cceae7046f72d1a9626f18969b7` |
| `run-loopback.sh` | `5bee642b69369f67367d3ea9df46baa5ed4df01e3f703b9afa5d53f9e391665c` |
| `fault-session.sh` | `3dcb9e82cfc0acf64f41c44d4480c1b4691cfc137580c10a8e5274bac2f17578` |

### 8.2 Physical fault matrix

The session recorded one extra, unobserved fault/recovery episode between
`HOLD TRUE` and the first post-HOLD block. The recorded matrix therefore starts
at `faults=1`/`recoveries=1`; the recorded Fault A is episode 2 and Fault B is
episode 3. This is a manual-sequence deviation, accepted by the owner, not a
product failure. `STATUSFAULTS` is cumulative and `STATUSRECOVERIES` increments
only when the link and slave return. `STATUSPHASE` stayed 2 (IgH Operation)
during both faults; only the slave link was down.

| Observation | Fault A: cable unplug/replug (episode 2) | Fault B: DIEWU power cycle (episode 3) | Fault C: slave-side reconnect |
| --- | --- | --- | --- |
| Fault start | session cycle 70582 | session cycle 183087 | Not run — covered by A |
| `STATUSFAULTS` before / during / after | 1 / 2 / 2 | 2 / 3 / 3 | Covered by A |
| `STATUSINPUTSTALE` during / after | true / false | true / false | Covered by A |
| `STATUSRECOVERIES` before / after | 1 / 2 | 2 / 3 | Covered by A |
| `STATUSWCSTATE` during / after | 0 / 2 | 0 / 2 | Covered by A |
| `STATUSLINKUP` during / after | false / true | false / true | Covered by A |
| `STATUSSLAVEALSTATE` during / after | 0 / 8 | 0 / 8 | Covered by A |
| `STATUSSLAVEOPERATIONAL` during / after | false / true | false / true | Covered by A |
| `SCANCOUNT` advancing during the fault (yes/no) | yes: 51872 → 70591 → 119703 | yes: 133860 → 183096 → 196078 | Covered by A |
| `OUTPUTMASK` during / after | 65535 / 0 (HOLD false after the stale check) | 65535 / 65535 (HOLD true) | Covered by A |
| `INPUTMASK` during / after | **65535** / 0 | **65535** / 65535 | Covered by A |
| `INPUTMASK` after `HOLD FALSE` during the fault | **65535** | Not performed (final HOLD FALSE after recovery) | Not performed |
| Recovery time (s) | Values read within the 10 s window after replug; exact time not measured | Values read within the 30 s window after repower; exact time not measured | — |
| Result (pass/fail) and notes | Pass. Stale input held at 65535; output held at 65535; `HOLD FALSE` gave 0/65535. Physical output LEDs cleared during link loss and returned on replug. | Pass. Stale input held at 65535; output held at 65535; recovery returned 65535/65535. Physical output LEDs cleared during the power loss and returned on repower. | Covered by A: one direct cable, so host-end and slave-end unplug are the same link-loss path; the slave stayed powered in A, which is the configured-slave reconnect case. |

The initial pre-HOLD block was healthy (`STATUSFAULTS=0`, `STATUSINPUTSTALE=false`,
`OUTPUTMASK=0`, `INPUTMASK=0`); `DONE=true` was printed before `HOLD TRUE`. An
unobserved unplug/replug happened before the first post-HOLD block, which is
why the counters begin at 1/1. The recorded
during-fault blocks show `INPUTMASK=65535` (the last received image) and
`OUTPUTMASK=65535` (the held PLC image). `HOLD FALSE` during episode 2 gave
`OUTPUTMASK=0`, `INPUTMASK=65535`. Recovery cleared the stale flag, restored
link/WC/AL, and incremented recoveries. `SCANCOUNT` advanced through every
interval; the intervals include operator time, not recovery latency. The
physical output LEDs cleared during each fault and returned on replug/repower.

### 8.3 Restore state

| Item | Record |
| --- | --- |
| `ethercat master` after the session (Phase, Active) | `Phase: Idle`, `Active: no`, `Slaves: 1`, link UP |
| Tx errors / lost frames | All-points: 18856 → 18856 (delta 0); fault session: 18856 → 24426 (delta 5570); Tx errors 0 in all four samples. The fault-session increase is the frames sent during the link-down and power-off windows. |
| `ethercat slaves` after release (PREOP) | `0 0:0 PREOP + DIEWU EtherCAT-IO (Digital 16-Input/16-Output)` |
| All-points log path and SHA256 | `/home/r2k/stc-wl52/logs/loopback-fix-20261101T174924.log` — `06d295c5f45c4fcaf468d365bc98873e3164a5c1e4da1818aa3c7fc131e85b41` |
| Fault-session log path and SHA256 | `/home/r2k/stc-wl52/logs/fault-session-fix-20261101T174958.log` — `14d7689bed749aa12770196722ed8c6e696e0cff7d9afe3e305717cd00b3de6e` |
| Fault-session transcript path and SHA256 | Same file; it is the `tee` transcript of the debug session. |
| Remaining `stc` processes (none expected) | None (`pgrep -a stc` empty). |

The earlier logs are retained for the defect history:
`fault-session-20261101T172040.log` (`b9f11faa2a76ef514c2f59d6667d0bde5e03ad42e00594f5cbe484d496529974`)
is the powered run on the phase-5 package that exposed the stale-input
zeroing; `fault-session-20261101T143222.log`
(`b0ca162d1af768805d0710601878847c1bedf468596e5669dfa4d7cc7b117148`) is the
supply-limited run; `loopback-20261101T171644.log`
(`fe3a8d6e72f6215dc4ee38f3b421623790a0a7dcf41731fcd0762b7a19d535ea`) is the
all-points pass on the phase-5 package. They are not the final evidence.

### 8.4 Outcome

- The powered all-points matrix passed 16/16 on the fixed package
  (`FailMask=0`, `ObservedMask=0xFFFF`, `faults=0`, `stale=false`, WC state 2,
  working counter 3, AL 8, exit 0); the master lost frames were unchanged at
  18856 and Tx errors were 0.
- The powered fault matrix passed: each recorded fault showed
  `STATUSINPUTSTALE=true` with `INPUTMASK=65535` (last received image),
  `OUTPUTMASK=65535` (held PLC image), and `SCANCOUNT` advancing. `HOLD FALSE`
  during the first recorded fault gave `OUTPUTMASK=0`, `INPUTMASK=65535`.
  Recovery cleared stale, restored link/WC/AL, and incremented recoveries
  without a cold download or host restart.
- Fault C is covered by Fault A; it was not run separately.
- The phase-6 acceptance items are met. The extra unobserved episode makes the
  cumulative counters 1/1 at the post-HOLD baseline and 3/3 at the end; this
  manual-sequence deviation is recorded above and is not a product failure.
- The earlier supply-limited attempt found a coordinator stale-input defect;
  the fix (commit `d694266`, CI 41367423862) is verified by this run on the
  fixed package.

## 9. Stop conditions

Stop and report, keeping every log, if any of the following occurs:

- `ethercat master` is not Idle before a run, or another application holds the
  master.
- The slave does not reach OP, or the all-points run reports `faults != 0`,
  `stale=true`, or `wc_state != 2` at a healthy baseline.
- The all-points `FailMask` is non-zero and a wiring check does not explain it.
- A fault does not recover within 30 seconds after the physical cause is
  removed.
- The debug session ends unexpectedly, the host process exits, or the master
  is left not Idle.
- Any step would require root, sudo, `--rt`, a host change, a service
  stop/start, or a rebuilt `stc`.

Do not describe a failed or partial run as a phase-6 pass. The final report
must separate frame-level evidence (phases 3-5) from the physical I/O evidence
recorded here, and must repeat the non-safety limitation.
