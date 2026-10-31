# EtherCAT HIL verification runbook — WL-5.2

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-31  
Status: Draft. The phase-3 open-ended procedure plus the phase-4 software-fault, status-register,
and online-change evidence are recorded here; phases 5–6 extend this runbook when they are
executed. Results are host- and hardware-specific; this is not a hard real-time guarantee and not
a safety function.  
Parent: [WL-5.2-Plan.md](../../WL-5.2-Plan.md) §7 (phases 3–4)  
Inputs: [I/O Routing and Backends](../Architecture/IO-Routing-and-Backends.md),
[I/O and Devices](../Architecture/IO-and-Devices.md),
[DIEWU fixture](../../tests/ethercat/diewu/README.md)

## 1. Protocol and scope

The phase-3 procedure brings up the delivered DIEWU 16I/16O EtherCAT slave on `ecat-host` with
open-ended (unwired/unloaded) I/O and runs a non-CPU-bound PLC program for 600000 cycles at 1 ms.
The target is **frame-level** success only:

- the slave is operational (OP) and the domain working counter is complete;
- `ecrt_master_send` accepts every cycle's frame and the receive/process path sees the reply;
- the runtime reports no send/receive errors and no unexplained fault episodes;
- the IgH master's lost-frame and Tx-error counters do not increase across the run.

Physical input values and measured output behaviour are **not** verified in phase 3. The terminal
blocks stay unwired/unloaded, and the procedure does not toggle, measure, unplug, power-cycle, or
inject faults. The all-points matrix, loopback harness, and physical fault injection are phase 6.
Phases 3–5 use this same open-ended, frame-level protocol.

The run is not a hard real-time measurement. It does not pass `--rt`, and the host is not changed.
The reference host, its tuned boot profile, and its IgH installation are inputs to this runbook;
this repository does not install or modify IgH on the host.

## 2. Host and prerequisites

| Item | Reference value |
| --- | --- |
| Host | `ecat-host`, Dell Wyse 5070, Intel Celeron J4105, 8 GiB RAM, bare metal |
| OS | Ubuntu Server 24.04.5 LTS, x86-64 |
| Kernel | `6.12.115-ecat-rt`, `CONFIG_PREEMPT_RT=y` |
| IgH master | 1.6.13, commit `61cc654f5b721ddd54df0f58bdd34106d91c5359`, at `/usr/local/etherlab` |
| IgH userspace | `libethercat.so.1.2.0`, `ecrt.h` version 1.6 |
| Device | `/dev/EtherCAT0`, `crw-rw---- root:ethercat`; the login user is in `ethercat` |
| Slave | DIEWU `DIEWU_EtherCAT_DIO_16`, vendor `0x00000b95`, product `0x00010200`, revision `1` |
| Slave position | master 0, alias 0, position 0; free-run, no distributed clocks |
| Compiler | `clang` on the host for the fixture's user-project build |
| Artifact source | CI package and shim built on `ubuntu-24.04`; never build `stc` on the host |

The login user opens `/dev/EtherCAT0` directly. Do not use `sudo` to work around a permission
error: if the device is not accessible to the login user, stop and report. Do not read, copy, or
store credentials or local access notes in the repository.

Before a run, record the host state and confirm that no other application owns the master:

```sh
uname -a
cat /etc/os-release
ethercat version
clang --version
ethercat master
ethercat slaves
```

`ethercat master` must show `Phase: Idle`, link up, one responding slave, and zero Tx errors and
lost frames. `ethercat slaves` must show the DIEWU slave in PREOP. If another application is
holding the master, stop and report; do not kill or restart it.

The host reserves CPU 3 with `isolcpus`, but this non-RT procedure does not pin the run or the IgH
threads, and the observed `EtherCAT-IDLE` thread has scheduling class `TS`, not a real-time class.
An SSH login or a burst of unrelated CPU work on CPUs 0–2 can therefore delay a frame. Run the
timed procedure from the single SSH session that starts it, and do not open other sessions, poll
from another terminal, or run background commands on the host while the run is in progress.
§8.1 records what happened when the run was monitored from separate SSH sessions.

## 3. Build and transfer

The base `stc` package has no IgH dependency. The optional shim artifact is loaded only when an
EtherCAT device is configured. CI builds both artifacts; the host receives the finished files.

### 3.1 Trigger the manual CI run

The workflow is manually dispatched on the `WL-5.2` branch. Trigger it and wait for the run whose
head is the commit under test:

```sh
gh workflow run ci.yml --ref WL-5.2
gh run list --workflow ci.yml --branch WL-5.2 --limit 5
gh run watch <run-id> --exit-status
```

The run must be green before its artifacts are used. The two artifacts are
`stc-package-ubuntu-24.04` and `stc-io-ethercat-linux-x86_64`.

### 3.2 Download and verify

On the build workstation, download both artifacts into an empty staging directory:

```sh
gh run download <run-id> -n stc-package-ubuntu-24.04 -D artifacts/package
gh run download <run-id> -n stc-io-ethercat-linux-x86_64 -D artifacts/shim
```

The package artifact expands to
`artifacts/package/dist/stc-<version>-x86_64-unknown-linux-gnu/` containing `bin/`, `lib/`,
`docs/`, `package.json`, and `SHA256SUMS`; the shim artifact contains `libstc_io_ethercat.so`.
Verify the package checksums before transferring it:

```sh
package="$(cd artifacts/package/dist/stc-* && pwd)"
(cd "$package" && sha256sum -c SHA256SUMS)
```

Transfer the package contents, the shim, and the fixture directory (without `target/`) to the
host:

```sh
package="$(cd artifacts/package/dist/stc-* && pwd)"
ssh <user>@ecat-host 'install -d "$HOME/stc-wl52" "$HOME/stc-wl52/logs"'
scp -r "$package/." <user>@ecat-host:~/stc-wl52/
scp artifacts/shim/libstc_io_ethercat.so <user>@ecat-host:~/stc-wl52/lib/
ssh <user>@ecat-host 'rm -rf "$HOME/stc-wl52/diewu"'
scp -r tests/ethercat/diewu <user>@ecat-host:~/stc-wl52/
ssh <user>@ecat-host 'cd "$HOME/stc-wl52" && sha256sum -c SHA256SUMS'
ssh <user>@ecat-host 'chmod 0755 "$HOME/stc-wl52/bin/stc"'
```

Artifact downloads do not preserve the executable bit; the `chmod` restores it for the package
binary. The fixture scripts are invoked with `bash`, so their mode is not significant.

The working directories on the host are `~/stc-wl52/bin`, `~/stc-wl52/lib`, and
`~/stc-wl52/diewu`. Raw logs stay under `~/stc-wl52/logs`. Do not commit the artifacts, logs, or
the fixture's `target/` directory. The `rm -rf` target is only the fixture copy; it never touches
`~/stc-wl52/logs` or the package files, and the fixture can be rebuilt from source.

### 3.3 Verify the deployed files on the host

```sh
~/stc-wl52/bin/stc --version
ldd ~/stc-wl52/lib/libstc_io_ethercat.so | grep ethercat
```

`stc --version` must report the packaged version. `ldd` must resolve `libethercat.so.1` through
`/usr/local/etherlab/lib`; if it does not, add that directory to `LD_LIBRARY_PATH` for the run.
Do not rebuild `stc`, the runtime, or the shim on the host. Building the user-project fixture with
the packaged `stc` is the intended use of the package.

If `clang` is missing on the host, stop and report. Do not install packages without
authorization.

## 4. Fixture

The fixture is source-controlled at `tests/ethercat/diewu/`. Its manifest declares sixteen BOOL
inputs and sixteen BOOL outputs on the EtherCAT device, a read-only `ethercat-status` device with
layout 1, the direct `%I`/`%Q` bindings, and `[io].config = "io/bench.toml"`. The selected machine
profile supplies the DIEWU identity, the sync manager and PDO map, the 32-channel map, and the
routes. The program copies `In0..In13` to `Out0..Out13`, toggles a state bit from `In14`, and
increments a counter; it is not CPU-bound.

The profile is selected by `[io].config` in the manifest and embedded in the built artifact. The
`--io-config` runtime option is a diagnostic path override; it does not change the profile's
routes or bindings. The source-controlled `io/bench.toml` is the profile under test.

Build the fixture with the deployed package:

```sh
export STC="$HOME/stc-wl52/bin/stc"
bash "$HOME/stc-wl52/diewu/build.sh"
```

The build emits the module and metadata under `diewu/target/stc/`. The package's runtime library
is used from the package `lib/`; set `STC_ETHERCAT_LIBRARY` and `LD_LIBRARY_PATH` when running.

## 5. Open-ended run

The run uses `stc run --io ethercat` without `--rt`. The script sets 600000 cycles at `1ms`
(`--cycles 600000 --cycle-time 1ms`), passes `-v`, and stops after the cycle count. It does not
write outputs during shutdown; the backend releases the master and the slave's process-data
watchdog handles the stopped frames.

1. Open one SSH session and keep it open for the whole run. Do not open another session while
   the run is active. The one exception is the initial session setup, which finishes before the
   run starts.

2. In that session, sample the master and the slave state, then run the script in the foreground:

   ```sh
   cd "$HOME/stc-wl52"
   mkdir -p logs
   ethercat master > logs/master-before.txt
   ethercat slaves > logs/slaves-before.txt
   export STC="$HOME/stc-wl52/bin/stc"
   export STC_ETHERCAT_LIBRARY="$HOME/stc-wl52/lib/libstc_io_ethercat.so"
   export LD_LIBRARY_PATH="/usr/local/etherlab/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
   bash diewu/run-open-ended.sh > "logs/open-ended-$(date +%Y%m%dT%H%M%S).log" 2>&1
   ethercat master > logs/master-after.txt
   ethercat slaves > logs/slaves-after.txt
   ```

   The script exits when the 600000 cycles complete. If it fails before the cycle count, keep
   the log and the master/slave outputs and diagnose the cause; do not start a second run while
   another process owns the master.

3. Keep the raw log, both master samples, and both slave samples on the host. Record their paths
   and SHA256 values in the run summary. Do not commit the raw logs, and do not reset the master
   counters.

4. After the run, `ethercat master` must be back in `Phase: Idle` and `ethercat slaves` must show
   the slave in PREOP. The slave's output watchdog handles the stopped frames; the runtime does
   not write the output image on shutdown.

## 6. Frame-level success criteria

The run is accepted when all of the following hold. Physical I/O values are not part of this
check.

| Check | Evidence |
| --- | --- |
| Slave reaches and stays operational | The backend health gate requires every configured slave to be online and operational; `faults=0`, `domain.wc_state=2`, and `master.slaves_responding=1` reflect OP. The kernel log shows the slave-state transition, and `ethercat slaves` after release shows PREOP. |
| Domain working counter complete | `domain.wc_state=2` and a stable working counter in the status line |
| Every cycle's frame accepted and replied | `exchanges=600000`; no `STC-E-IO-ETH-*` send/receive error; no startup timeout |
| No unexplained runtime fault | `faults=0`, `master.recoveries=0`, `input.stale=0`, `master.last_error=0` at exit |
| Master counters unchanged | `Tx errors` and `Lost frames` in `logs/master-before.txt` and `logs/master-after.txt` are equal |
| Kernel log clean during the run | no `Working counter changed to 0/3`, no `SKIPPED`, and no `UNMATCHED` after the activation phase |
| Shutdown is non-writing | master released; slave PREOP; output image not rewritten by the runtime |

The expected `-v` status line is:

```text
stc: status: io-backend ethercat devices=2 state=running exchanges=600000 faults=0 hash=0x... domain.wc_state=2 domain.working_counter=3 input.stale=0 master.last_error=0 master.link_up=1 master.recoveries=0 master.slaves_responding=1
```

The `hash` value is the configuration hash for the selected profile and bindings. It must match
the committed fixture and profile; record it in the run summary.

IgH may print activation-phase messages such as `1 datagram UNMATCHED!` and
`No application time received up to now` before the first complete domain cycle. These occur
during `ecrt_master_activate` and the PREOP/SAFEOP/OP transition, not during the steady-state
process-data run. Record them in the run summary; a warning in the steady-state window is a
frame-level fault and must be diagnosed.

## 7. Troubleshooting

- `STC_ETHERCAT_LIBRARY` unset or wrong: the runtime fails to load the shim. Check the status
  line's `last-fault` and the log; do not rebuild the shim on the host.
- `libethercat.so.1` not found: set `LD_LIBRARY_PATH=/usr/local/etherlab/lib` for the run.
- `/dev/EtherCAT0` permission denied: stop and report; do not use `sudo` or change device
  permissions.
- `ethercat master` cannot open the master: another application owns it. Stop and report; do not
  kill the other process.
- Startup timeout or identity mismatch: compare the fixture's vendor, product, revision, alias,
  and position with `ethercat slaves -v`; fix the fixture, not the host.
- A single skipped/unmatched datagram while another SSH session is active: finish that session,
  confirm the master counters did not increase, and repeat the 10-minute run in one session. The
  master threads are `SCHED_OTHER`; login CPU bursts can exceed one cycle.
- Any steady-state WC or AL fault: keep the logs and the master/slave samples, and stop before
  changing the host. Do not power-cycle, unplug cables, or toggle I/O in phase 3.

## 8. Phase 3 evidence

Two 10-minute runs were made on 2026-10-31 with the same package, shim, and fixture. The first was
monitored from separate SSH sessions and recorded one transient working-counter fault during a
login CPU burst; it is retained as a diagnostic run but is not the acceptance run. The second ran
from a single SSH session with no other host activity and is the phase-3 acceptance run. The slave
was open-ended (unwired/unloaded); physical input values and measured output behaviour were not
tested. Raw logs stay on `ecat-host` under `~/stc-wl52/logs/`; this section is the committed
summary and hash record.

### 8.1 First diagnostic run (fault under concurrent SSH activity)

The command, environment, and cycle count were the same as §5: `bash diewu/run-open-ended.sh`
with `STC=~/stc-wl52/bin/stc`,
`STC_ETHERCAT_LIBRARY=~/stc-wl52/lib/libstc_io_ethercat.so`, and
`LD_LIBRARY_PATH=/usr/local/etherlab/lib`, 600000 cycles at 1 ms, debug module. The run occupied
09:11:40–09:22:29 (-06:00) and exited 0 with log `logs/open-ended-20261031T091140.log`.

| Field | Observed |
| --- | --- |
| Exchanges | 600000 |
| Runtime state at exit | `state=running`, `faults=1`, `recoveries=1`, `input.stale=0` |
| Domain at exit | `domain.wc_state=2`, `domain.working_counter=3` |
| Last fault | `STC-W-IO-ETH-003: fieldbus is not operational` |
| Master before | Phase Idle, 1 slave, Tx frames 60951966, Rx frames 60951965, Tx errors 0, Lost frames 0 |
| Master after | Phase Idle, 1 slave, Tx frames 61556753, Rx frames 61556752, Tx errors 0, Lost frames 0 |
| Slave state | OP after activation; no state change in the fault window; PREOP after release at 09:22:29 |
| Kernel fault window | 09:15:27 WC changed to 0/3; 09:15:28 `1 datagram UNMATCHED!`, `domain0-0-main` SKIPPED 1 time; WC changed to 3/3 |

The fault window coincides with a separate monitoring login: journal session 201 opened at
09:15:26 and closed at 09:15:28, and the session scope consumed 1.092 s CPU time. The observed
`EtherCAT-IDLE` thread is `SCHED_OTHER` (`TS`), and this non-RT procedure does not pin the run or
the IgH threads, so a login CPU burst can delay a reply past one cycle. The runtime's health gate
saw the incomplete working counter, entered fault, kept the cycle running, and recovered after
three consecutive healthy samples; the slave stayed OP, and the master's Tx-error and lost-frame
counters stayed at zero. This is consistent with host scheduling, not a runtime send/receive
error. Because the run was monitored from separate sessions, it is not used as the acceptance
run; the clean run in §8.2 repeats the procedure in isolation.

### 8.2 Clean acceptance run

The acceptance run was started and left alone in a single SSH session. It occupied
09:26:09–09:36:57 (-06:00), exited 0, and wrote `logs/open-ended-clean-20261031T092609.log`.
No other host login or background command ran during the window. The status line was:

```text
stc: status: io-backend ethercat devices=2 state=running exchanges=600000 faults=0 hash=0x9325a55375832b4fb39ed3bbfd3cc7a0d38dbd1b7ecb44dfaf52ab3f6174681c domain.wc_state=2 domain.working_counter=3 input.stale=0 master.last_error=0 master.link_up=1 master.recoveries=0 master.slaves_responding=1
```

| Criterion | Evidence |
| --- | --- |
| Slave operational | `faults=0`, `domain.wc_state=2`, `master.slaves_responding=1`; kernel `Slave states on main device: OP` at 09:26:11; after release `ethercat slaves` reports `0 0:0 PREOP + DIEWU EtherCAT-IO (Digital 16-Input/16-Output)` |
| Domain working counter complete | `domain.wc_state=2`, `domain.working_counter=3` |
| Frames accepted and replied | `exchanges=600000`; no `STC-E-IO-ETH-*` send/receive error; no startup timeout |
| No runtime fault | `faults=0`, `master.recoveries=0`, `input.stale=0`, `master.last_error=0` |
| Master counters unchanged | Tx errors 0 and Lost frames 0 in both samples below |
| Kernel log clean | activation-phase `UNMATCHED` and `No application time` warnings at 09:26:11 only; WC 3/3 and OP follow; no WC change, `SKIPPED`, or `UNMATCHED` after activation |
| Shutdown non-writing | master Idle, `Active: no`; slave PREOP; the runtime does not write outputs on shutdown; physical outputs were not measured |

| Master sample | Phase | Slaves | Tx frames | Rx frames | Tx errors | Lost frames |
| --- | --- | --- | --- | --- | --- | --- |
| Before (09:26:09) | Idle | 1 | 61768039 | 61768038 | 0 | 0 |
| After (09:36:57) | Idle | 1 | 62369519 | 62369518 | 0 | 0 |

The frame counters advanced by 601480 with no lost frame and no Tx error. The 600000 requested
1 ms cycles took 648 s of wall time (about 1.08 ms effective), consistent with the non-RT pacing
and the debug module; phase 3 checks frame acceptance, not cycle jitter. The run is accepted as
the phase-3 frame-level acceptance run. OP is inferred from the health gate, the kernel state
transition, and the post-release state because the operator status line does not print per-slave
fields. Physical I/O values were not verified.

### 8.3 Artifacts, fixture, and log hashes

The deployed package is CI artifact `stc-package-ubuntu-24.04` from run
[41212625765](https://github.com/r2k-in-the-vortex/stc/actions/runs/41212625765) at head
`021e12f6dbfe8957952f7bb4d09d72acd9eb256a`; the shim is artifact
`stc-io-ethercat-linux-x86_64` from the same run. The package is
`stc-1.0.0-x86_64-unknown-linux-gnu` (`debug_contract: dwarf`, host
`x86_64-unknown-linux-gnu`). The deployed files on the host matched the package:

| Artifact | SHA256 |
| --- | --- |
| `bin/stc` | `b05c2b5bd4b76bb9728fca0ed451b90b127d5838acde5ac8f9453b4771214ff0` |
| `lib/libstc_runtime.so` | `fbe4764f3f608b6fb3663bfb583d378828579a919aed500e66eb41ff3af6a090` |
| `lib/libstc_runtime.a` | `002bf66b3269fe6d874dca2f81d98c906e56fb8f1d45cbb49bbae08d01d61b2d` |
| `lib/libstc_io_ethercat.so` | `bceeb6014076f41958b0ae4442f744fc3035af3544bcd5a046e278089e5df448` |
| `package.json` | `dd3e5d37e936331c5876ccf50e5b5dcc920765212665d3f91c41e80abc405dbb` |
| `SHA256SUMS` | `9a7e878617d2e58ceb92429d6caec9dc80a5d59e85b71f4f0008a8392c00958b` |
| `compliance.json` | `a015ba95f5eecfeca7db724cb8aa18f19593bc2bc79c5de3d08d23f4a77eedad` |

The fixture files used by both runs:

| File | SHA256 |
| --- | --- |
| `diewu/stc.toml` | `c50d43a6df3df9edfca9353512b8b71e8c7b0bc55139029414f48a66d2f36f9b` |
| `diewu/io/bench.toml` | `5638fb28969b6931cec7b19947c35472e16bf0ffa1ad814161de0d2fedcdb20a` |
| `diewu/src/main.st` | `3d70b4d46b391aa754e0dae51a1b03400a8d5a6935da946b2b2f419049ce8522` |
| `diewu/build.sh` | `141b15c77f4f4827b580e96ee77700ab372c2fb44c6f298a087a48b41ccd658d` |
| `diewu/run-open-ended.sh` | `eba6c5244eb86d5446999a983f7542db6a06ce5c7727c61dec017a83deeeba66` |

Raw log hashes, relative to `~/stc-wl52/` on `ecat-host`:

| File | SHA256 |
| --- | --- |
| `logs/open-ended-clean-20261031T092609.log` | `aa77b1b322e591fe6883fb9dd6561cd8a5c6453d314e8be34f894bff441da80d` |
| `logs/kernel-clean.log` | `7272c1e373d1ab50fa1ca4a847652bee6aa380f4f177c93276d80022286f529a` |
| `logs/master-clean-before.txt` | `10693d9b189b064020636c330267e52bee418b3143eb8e82e8cae85917fbce72` |
| `logs/master-clean-after.txt` | `17f66537a94ea6c80d105ea79c7e011c88ceb839280ab6bec824c6b9c362d3b6` |
| `logs/host-clean.txt` | `7976f5a361ee6979ab2e4c7c79b63c68d3a11c0f84b10ab4e8618777d05b7ebd` |
| `logs/run-clean-start.txt` | `2bfff3dceb77316f24076fe61b84e8533cd49b1f753d44a177640977b233bd64` |
| `logs/run-clean-result.txt` | `dc16d4c2e8c3bd20fa4fb2ebb3ee31efc62b3ced964b86bfc574f53e8242373a` |
| `logs/open-ended-20261031T091140.log` | `290139b590321a6d6bdef3803911a17d5462e0530cbf368613c6189a5520ab25` |
| `logs/master-before.txt` | `c173ebbb3c6916d991c545463c067a2946ad150ac499811a5293c1ba8d82ecf6` |
| `logs/master-after.txt` | `c34805366c550dba46d276ac5ead6251e000e61fb25d1049311e9f353b9f4e81` |

The host record also reports kernel `6.12.115-ecat-rt #2 SMP PREEMPT_RT`, `nproc: 3` (the boot
profile isolates CPU 3), `stc 1.0.0`, IgH 1.6.13, and clang 18.1.3. The raw logs and the
deployment are not committed; the hashes above identify the retained evidence.

## 9. Phase 4 evidence

Phase 4 exercises the software-fault matrix, the PLC-visible status device, and online change on
`ecat-host`. No physical fault was injected: no cable was unplugged or reconnected, the slave was
not power-cycled, no loopback harness was used, and no input or output was toggled or measured;
those remain phase 6. The master service was not stopped or restarted (§9.3). Every run used the
prebuilt package; nothing was rebuilt on the host, and no `sudo` command was used.

The package is CI run
[41217639207](https://github.com/r2k-in-the-vortex/stc/actions/runs/41217639207) at head
`938aa937d40b5a277a3c1142390596177b5a5c3e` (`stc-1.0.0-x86_64-unknown-linux-gnu`). The deployed
files matched the run's artifacts:

| Artifact | SHA256 |
| --- | --- |
| `bin/stc` | `c68055ca99876770e49fb1466a5d0576546d33a0e79e0009aed0082a3455640e` |
| `lib/libstc_runtime.so` | `600f7348555c4e7bfb6e0f08f461b1876e395595548168ad6f00652cdcc46e7f` |
| `lib/libstc_runtime.a` | `11070b1f6add9ea8df047b576effec6b851e047196077e98a52c20e96a866ae1` |
| `lib/libstc_io_ethercat.so` | `bceeb6014076f41958b0ae4442f744fc3035af3544bcd5a046e278089e5df448` |
| `package.json` | `dd3e5d37e936331c5876ccf50e5b5dcc920765212665d3f91c41e80abc405dbb` |
| `SHA256SUMS` | `77ff67dcbbcf49afa795f3c87e06d3d6b70b2945ca749797c64c336ddaa1700e` |

The run's `compliance.json` metadata is unchanged from phase 3 (`a015ba95f5eecfeca7db724cb8aa18f19593bc2bc79c5de3d08d23f4a77eedad`).
The shim binary is byte-identical to the phase-3 artifact; phase 4 changed the runtime, not the
shim. The fixture files stayed at their §8.3 hashes: `stc.toml` `c50d43a6...`, `io/bench.toml`
`5638fb28...`, `src/main.st` `3d70b4d4...`, `build.sh` `141b15c7...`, and `run-open-ended.sh`
`eba6c524...`. The master-owned case uses the new helper `diewu/holder/holder.c`
(`1051c8f7f8663c3f7a064f9548687456cdbb21e77abcb87c8a583331c362ab8a`); the host-compiled binary
is `f41dc457ea92ea3875e136530f2806db7c759881240a6119f543d2420941b53d`.

### 9.1 Master already owned

`holder.c` is a small ordinary-user application that calls `ecrt_request_master(0)` and pauses; it
reproduces the master-owned condition without root. The host binary was compiled from the
committed source, and the host source hash equals the repository hash.

Procedure: run the holder as the login user, confirm `ethercat master` shows `Phase: Operation`,
start the fixture with `--cycles 10 --cycle-time 1ms`, then terminate the holder with SIGTERM.
The runtime failed closed:

```text
Failed to reserve master: Device or resource busy
stc: cannot install module ...: STC-E-IO-ETH-002: shim master open failed: cannot request EtherCAT master 0 (code 2)
```

The process exited 1. While the holder owned the master, `ethercat master` reported `Phase:
Operation`, `Active: no`, and one slave; the runtime published no `stc: status: io-backend` line
and never entered the run loop, so no outputs were written. After the holder exited, the master
returned to `Phase: Idle` and the slave remained PREOP. The runtime does not contend for or take
over a master that another application owns. The same result was observed with the phase-3
package before this re-run with the phase-4 package.

### 9.2 Wrong identity and startup timeout

Three copies of the fixture profile changed one field each; the committed fixture was not
modified. Every run used 10 cycles at 1 ms with `--no-retain --status`.

| Case | Profile change | Runtime diagnostic | Master after | Slave after |
| --- | --- | --- | --- | --- |
| Wrong identity | `vendor` `0x00000b95` -> `0x00000b96` | exit 1, `STC-E-IO-ETH-008: EtherCAT startup timed out after 10000 ms: fieldbus is not operational` | Idle | PREOP |
| Wrong position | `position` 0 -> 1 | exit 1, `STC-E-IO-ETH-003: shim slave configure failed: cannot read slave 1 identity (-22) (code 3)` | Idle | PREOP |
| Startup timeout | `startup-timeout` `10s` -> `1us` | exit 1, `STC-E-IO-ETH-008: EtherCAT startup timed out after 0 ms: fieldbus is not operational` | Idle | PREOP |

IgH accepts the slave configuration for a non-matching vendor/product but leaves the slave
unconfigured, so the failure surfaces as the configured startup timeout (ETH-008), not as an
identity error; the 1 us profile prints as `0 ms` because the diagnostic is reported in whole
milliseconds. A position with no slave fails identity lookup (ETH-003; IgH logs
`Failed to get slave info: Invalid argument`). In all three cases the runtime opened no run,
released the master, and left the slave in PREOP; no outputs were published. Each case is
fail-closed and returns to `Phase: Idle`.

### 9.3 Master service stop/start (recorded privilege gap)

The matrix entry for stopping and restarting the EtherCAT master service was not executed.
`sudo -n -l` on the host lists a password-gated `(ALL : ALL) ALL` entry and one NOPASSWD entry,
`(root) NOPASSWD: /usr/local/sbin/stc-wl4-run`, which does not cover `systemctl stop` or
`systemctl start ethercat`. The phase rules forbid privilege escalation and host changes, so this
case is recorded as a privilege gap instead of being tested; no `sudo` command was run. The
runtime-visible behavior it would exercise (backend fault, last inputs presented stale, recovery
after the window, unchanged outputs, and another backend remaining unaffected) is covered by the
Linux CI tests in §9.4. Testing the service transition directly needs a human decision to widen
the NOPASSWD policy.

### 9.4 Runtime fault, stale inputs, and recovery

No physical link fault was injected in phase 4. The runtime semantics are supported by the phase-3
diagnostic run in §8.1, which observed a real working-counter fault under host CPU load
(`faults=1`, `recoveries=1`, `input.stale=0` at exit; the fault cleared after three healthy
exchanges, outputs were not rewritten, and the master counters stayed at zero), and by the Linux
CI job for the phase-4 head. The following tests passed in run 41217639207:

- `io_ethercat::tests::status_device_is_filled_from_master_domain_and_slave_state`
- `io_ethercat::tests::fault_keeps_last_inputs_stale_and_recovers_after_the_window`
- `io_ethercat::tests::faulted_publish_still_sends_plc_outputs_unchanged`
- `io_ethercat::tests::startup_timeout_is_eth_008_and_opens_no_run`
- `io_ethercat::tests::removed_frame_counters_are_rejected_as_status_channels`
- `coordinator::tests::one_backend_fault_does_not_touch_another_backends_data`
- `io_mock::tests::fault_injection_reports_and_clears_a_faulted_backend`

These cover the E-6 policy: a non-fatal runtime fault keeps the cycle running, leaves the output
image unchanged, presents the last input image as stale, publishes the status registers every
cycle, and clears staleness only after `recovery-cycles` consecutive healthy exchanges. The
coordinator test shows that one backend's fault does not touch another backend's data.

### 9.5 PLC-visible status registers

The fixture's `ethercat-status` device (E-14, layout 1) occupies 14 read-only `%I` channels. In a
healthy open-ended run (`--cycles 600000 --cycle-time 1ms`) with the debug endpoint attached, each
channel was read with `var.read` on `DIEWU-HIL::STATUS...`. The values at runtime cycles 46–61:

| Variable | Type | Observed | Meaning |
| --- | --- | --- | --- |
| `STATUSPHASE` | UINT | 2 | master phase Operation |
| `STATUSMASTERACTIVE` | BOOL | true | master active |
| `STATUSSLAVESRESPONDING` | UINT | 1 | one slave responding |
| `STATUSLINKUP` | BOOL | true | link up |
| `STATUSLASTERROR` | UDINT | 0 | no master error |
| `STATUSRECOVERIES` | UDINT | 0 | no recoveries |
| `STATUSWCSTATE` | UINT | 2 | domain working counter complete |
| `STATUSWORKINGCOUNTER` | UINT | 3 | working counter 3 (expected) |
| `STATUSINPUTSTALE` | BOOL | false | input image current |
| `STATUSEXCHANGES` | UDINT | 56 | successful exchanges |
| `STATUSFAULTS` | UDINT | 0 | no faults |
| `STATUSSLAVEALSTATE` | UINT | 8 | slave AL state OP |
| `STATUSSLAVEONLINE` | BOOL | true | slave online |
| `STATUSSLAVEOPERATIONAL` | BOOL | true | slave operational |

The control status endpoint reported `cycles` 144 before and 2093 after the reads (the host cycle
counter is per generation); the run's final status line reported `cycles 2184`,
`exchanges=2184`, `faults=0`, `domain.wc_state=2`, `domain.working_counter=3`, `input.stale=0`,
`master.last_error=0`, `master.link_up=1`, `master.recoveries=0`,
`master.slaves_responding=1`, and
`hash=0xb9a6ddf4daaa2b6e6676a896a1d420c88d8d980c54dde792384187103fc6b8a1`. While running,
`ethercat master` reported `Phase: Operation`, `Active: yes`; after the clean stop it reported
`Phase: Idle`, `Active: no`, with Tx errors and lost frames unchanged at zero, and the slave
returned to PREOP. The `master.lost_frames`, `master.tx_errors`, and `master.frame_loss_permille`
fields are intentionally not status channels; the CLI samples them at run boundaries, and
`ethercat master` is the evidence of record.

The `hash` is `0xb9a6...` rather than the §8.2 `0x9325...` because the phase-4 head refines the
E-7 configuration hash to cover the effective bindings and routes; the phase-3 run predates that
change. The observed status layout and channel set are the committed fixture's.

### 9.6 Online change

Online change used the packaged runtime in watch mode (`stc run --watch -v`) with the fixture's
debug and control endpoints. The fixture was restored to its committed hashes after the test.

**Code-only change.** A source-comment-only edit to `src/main.st` was rebuilt by the packaged
compiler into build `0x6ea2a4622ad0b9bdfc3329d56ddac264` from baseline
`0x536dc3bbd8d3409af91f85898f922dd6`. The watch build was compatible (`preserved 6 allocated 45
dropped 0 retired 0 rejected 0`), deployed online (`deployed build
0x6ea2a4622ad0b9bdfc3329d56ddac264`), and the active generation switched without reopening the
master: status `STATUSEXCHANGES` advanced 260 -> 1060 across the swap while the per-generation
host cycle counter reset 342 -> 308, and all 14 status values stayed healthy (phase 2, active
true, WC state 2, WC 3, stale false, faults 0, slave OP). The EtherCAT backend and its
master/domain were preserved.

**Changed configuration.** A temporary profile edit (`startup-timeout` `10s` -> `9s`, then
restored) produced candidate `0x81e5ad05bb7a6ec93749e1e2499791e9` from baseline
`0x6ea2a4622ad0b9bdfc3329d56ddac264`. The deploy was rejected:

```json
{"code":"STC-E-ONLINE-IO-001","cold_required":true,"id":1,"message":"the I/O configuration changed (active 0xb9a6ddf4daaa2b6e6676a896a1d420c88d8d980c54dde792384187103fc6b8a1, candidate 0x7be1ca1042aa0628639d1bd3749394a54f2e6f717542c8a90d1d3ff0ae31c346); a cold download is required","ok":false,"prepare_ns":4685930,"protocol":1,"report":{"baseline_build_id":"0x6ea2a4622ad0b9bdfc3329d56ddac264","candidate_build_id":"0x81e5ad05bb7a6ec93749e1e2499791e9","counts":{"allocated":45,"dropped":0,"preserved":6,"rejected":0,"retired":0},"rejections":[],"verdict":"compatible"}}
```

The response's report also carried the baseline-source-fingerprint warning that the watch log
shows; it did not affect compatibility. The active generation continued: the watch logged `stale
baseline; rebuilding against the active manifest`, re-prepared against the active manifest, and
then logged `keeping generation
0x6ea2a4622ad0b9bdfc3329d56ddac264; candidate was rejected`. The control status recorded
`last_rejection` as the same `STC-E-ONLINE-IO-001` message with `pending_build: null`, and the
status values after the rejection stayed healthy (`STATUSEXCHANGES` 3292, `STATUSFAULTS` 0,
`STATUSINPUTSTALE` false, slave OP). The watch exited 0 on stop; the master returned to Idle and
the slave to PREOP. The active generation continued across every rejection.

Topology, binding, PDO, and profile mutations are covered by the same configuration hash. The
Linux CI test `online_change_rejects_a_changed_io_configuration` mutates each of those fields and
asserts `STC-E-ONLINE-IO-001`, unchanged active generation and build, no pending generation, an
unchanged manifest, and a continuing cycle. `configuration_hash_covers_backend_config_and_ignores_unbound_remaps`
and `io_configuration_rejection_marks_cold_required` passed in the same run. A same-configuration
reload is still allowed to remap unbound direct cells; the phase-8 image-remap test covers that
path. Cross-backend isolation is covered by
`coordinator::tests::one_backend_fault_does_not_touch_another_backends_data`.

### 9.7 Evidence hashes

Raw logs and samples remain on `ecat-host` under `~/stc-wl52/`; the hashes below identify them.
They are not committed.

| File (relative to `~/stc-wl52/`) | SHA256 |
| --- | --- |
| `logs/phase4-fault-matrix-before-20261031T101338.txt` | `f143b27abed69524b3bfb6eed96f0f605d9ca3fff69e70cd531022fe354d5c49` |
| `logs/phase4-fault-matrix-slaves-before-20261031T101338.txt` | `4f94d5060a0386cc9a684443117919cd69b82ea5aeaf55a45de6cc94c1ccdbe0` |
| `logs/phase4-bad-identity-20261031T101338.log` | `598e8f2a7f7c1f718c7af84ffb4496e300e5becaca77fffca3c19f49d4e5fa44` |
| `logs/phase4-bad-identity-master-after-20261031T101338.txt` | `1537521519ef32132c5a2d84ad6aaffc849d3d6a550c9366f5d91440685a026c` |
| `logs/phase4-bad-position-20261031T101338.log` | `80da868761923d3aa60ec273e101e59d36c5b48bfc73ff6d2c0d95844396e8c1` |
| `logs/phase4-bad-position-master-after-20261031T101338.txt` | `9cd5865fff826f76fbef27ccbc2b021c4b41a3150d195d87f3e6a205eaa2f647` |
| `logs/phase4-startup-timeout-20261031T101338.log` | `b600aa43fa368f595775af02dc04c93a3b8c84a3965c85a5f8f45b2dd5da44bb` |
| `logs/phase4-startup-timeout-master-after-20261031T101338.txt` | `d89f524e76cbcf261d6fbfbee6c81c2852ccbda1db61446d9d4f5455d4be88a7` |
| `logs/phase4-master-owned-run-20261031T101635.log` | `204660cdd27de5f7264de574ac35fed1aad6ffbc7793cc58b3d23cac45ebde3d` |
| `logs/phase4-master-owned-holder-20261031T101635.txt` | `3edf00ec22198959df1d2865a7c4a189e88e57d25e9fb422a0a33b674769e925` |
| `logs/phase4-master-owned-during-20261031T101635.txt` | `418385d94a57df16edb7efb39e3abb8cbc53e56815795597662b4ad2e8b980af` |
| `logs/phase4-master-owned-after-failure-20261031T101635.txt` | `ba13ed10ec02bcc1355ac51120238d7d6fe49a963e3fb0d6e1ee6df1f1a075e4` |
| `logs/phase4-master-owned-after-release-20261031T101635.txt` | `4afa4e1910c2678cf8c5c548f6866906162f1d38f0102ecbe61842a81cd0a6c1` |
| `logs/phase4-status-run-20261031T101417.log` | `9df72b9d367535d984f546653a04c1d2b3b44497a166e4d66b6819d97a4a8803` |
| `logs/phase4-status-values-20261031T101417.json` | `f64e654b089cbc3c9f2b90b3b45a2438ce9a82d427603962adfcf8d0d8d69486` |
| `logs/phase4-status-control-before-20261031T101417.json` | `b206b2f2ae7dea551d77d00e521e5e8118f18025c2c877f6d868365f329c0844` |
| `logs/phase4-status-control-after-20261031T101417.json` | `e11f3a7be8e4207bf65adbdf9574c65f9a96ce9547e7bad136412f25556ac319` |
| `logs/phase4-status-master-during-20261031T101417.txt` | `e13d2cac78e37924bab557fd45f12b2e35a238c1f04b3939f457724718818766` |
| `logs/phase4-status-master-after-20261031T101417.txt` | `cc89569a9d13499ec0a9e5483952705c58c865a2beccc1302ef2f903bcbae3b0` |
| `logs/phase4-status-debug-20261031T101417.err` | `4247b0dd344212746c8257f580623927fba5f88b6c5f087ab0e5c91073ae8d68` |
| `logs/phase4-online-watch-20261031T101515.log` | `5c24e7b3af6b296a48825978e82a4e16a0e205892d6ea19626c4f6cec503c0ed` |
| `logs/phase4-online-control-before-20261031T101515.json` | `454abb441d665d7108c34efdf6b221d0d0bfa20c3cb7f9a67157d55e6c6cd88e` |
| `logs/phase4-online-control-after-code-20261031T101515.json` | `4f07cf1da7bc6564ced8689b0a1df9657477f5ac9acb00ca57ba54b0cec45ab6` |
| `logs/phase4-online-control-after-reject-20261031T101515.json` | `ad453050a05aea4378cabea238c3b279b5d273add0da37f510ece63b8752f61f` |
| `logs/phase4-online-deploy-response-20261031T101515.json` | `93c0266917e9db8f4dcf0a8e41ccda39015e278279895c6b3f4e95da378a9c7a` |
| `logs/phase4-online-before-20261031T101515.json` | `6111f5a2d0d24def562a4b18e70e8ac76d734a17bc4fae58d4f3c421e708ab52` |
| `logs/phase4-online-after-code-20261031T101515.json` | `5a6d23b04fc9bf88ab8ea0b3e8c750e6e0904218ae40f43463d42ac6081b0375` |
| `logs/phase4-online-after-reject-20261031T101515.json` | `6e9ae6b243a537533c38eedde1fc2be5db052f5ac871ea6e424438eaf0869340` |
| `logs/phase4-online-master-after-20261031T101515.txt` | `c091a46a78e31b8b3af29a573c4f0f4ee1ad5014a0cd28f0b890217f15537c85` |
| `logs/phase4-online-slaves-after-20261031T101515.txt` | `4f94d5060a0386cc9a684443117919cd69b82ea5aeaf55a45de6cc94c1ccdbe0` |
| `logs/phase4-status-control-stop-20261031T101417.json` | `758011a4122a9955fe58adbea76db699b9115eeac4de77f8201819780c18e6a8` |
| `logs/phase4-online-stop-20261031T101515.json` | `758011a4122a9955fe58adbea76db699b9115eeac4de77f8201819780c18e6a8` |

The host-side phase-4 helpers are `holder/holder.c` and `holder/holder`, under
`~/stc-wl52/`; the control client and status batch are `phase4/ctl.py`
(`aa32581217ff3e491d4e8dcfc59ef4275cac4bb20bb31134821d3dd37f8e1603`) and
`phase4/status-batch.json`
(`177fd9894a718f8d0d75f61a17f919b6de6fec75271118f7ffe374cac832d162`). They are reproduced by the
committed `holder/holder.c` and the runbook's debug/control protocol; they are not committed.

### 9.8 Limitations

Phase 4 is a software-fault matrix only. It does not inject physical link, slave-power, or
input/output faults, and it does not measure physical I/O; those are phase 6. The healthy status
run verifies the register layout, values, and visibility, not the behavior under a physical
disconnect. The master service stop/start case is blocked by the host privilege policy (§9.3).
The identity and timeout cases used profile copies, and the committed fixture was restored to its
committed hashes afterwards. The online-change test used one code-only edit and one profile edit;
the remaining configuration classes are covered by CI tests on the same hash policy. No timing or
real-time claim is made.

## 10. Cleanup

The run releases the master on exit. Confirm `ethercat master` is `Phase: Idle` and
`ethercat slaves` shows PREOP. Keep the logs under `~/stc-wl52/logs/`; leave the host's IgH
installation, boot profile, and service state unchanged. Do not rewrite the output image or
zero the outputs as part of cleanup.

## 11. Phase 5–6 extensions

Phase 4 is recorded in §9: the software-fault matrix (master already owned, wrong identity,
startup timeout), the PLC-visible status registers, the stale/recovery model, and online change.
The master service stop/start case remains a recorded privilege gap.

Phase 5 adds the cycle-time sweep from 1 ms down to 50 µs (extending below 50 µs if that passes),
60 s screening per step, a 10-minute confirmation at the fastest passing step, release-build
measurements, skipped-period/deadline checks, and the `stc-io-metrics/1` JSONL stream.

Phase 6 adds the loopback harness, the automated all-points matrix, physical fault injection
(cable unplug/replug, slave power cycle, configured-slave reconnect), and the final report. Those
procedures will be recorded in this runbook before they are executed.
