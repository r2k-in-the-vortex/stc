# EtherCAT host integration report — WL-5.2

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-11-01  
Status: Final for WL-5.2; the phase-6 powered loopback and fault matrix passed on the fixed package.  
Origin: v2.0.0 target WL-5.2, released 2026-11-04 ([RELEASE-NOTES.md](../../RELEASE-NOTES.md))  
Evidence: [EtherCAT-Runbook.md](EtherCAT-Runbook.md) §8–§13,
[PHASE6.md](../../tests/ethercat/diewu/PHASE6.md) §8

## 1. Result

`stc` now has a Linux EtherCAT I/O backend on the IgH EtherCAT master, integrated through the same
multi-backend coordinator as the existing `null`, `memory`, and `file` backends. The base package
does not link IgH; the backend is an optional CI-built C shim loaded with `dlopen` when an
EtherCAT device is configured. The device tree, PDO/channel map, and routes live in a
source-controlled machine profile; the runtime embeds the profile and hashes it for online
change.

The evidence covers one host and one slave: `ecat-host` and the DIEWU 16I/16O digital I/O module.
Frame-level success (phases 3–5) is met. The phase-6 powered all-points matrix passed 16/16, and
the powered physical fault/recovery cases A and B passed on the fixed package. An earlier
supply-limited attempt exposed a coordinator stale-input defect; the fix was verified on hardware
with the final package. All phase-6 acceptance items are met.

The fastest maintainable cycle time is **150 µs** (60 s screening and 600 s confirmation). The
first failing step is **100 µs**, where the host failed to wake the cycle thread on time (283
skipped periods; wake lateness 53.4–133.3 µs) while the EtherCAT frame path stayed healthy.
75/50/25 µs were not run because the sweep stops at the first failure. The result is
host-specific and is not a hard real-time guarantee.

Not a safety function. During a fieldbus fault the backend keeps sending the PLC output values
unchanged, presents the last received inputs as stale, exposes master/slave status to the PLC,
and recovers when communication returns. It does not zero outputs, substitute inputs, or exit;
the PLC logic decides. Safety must be provided by a separate system and machine design.

## 2. Host and hardware baseline

| Item | Value |
| --- | --- |
| Host | `ecat-host`, Dell Wyse 5070, Intel Celeron J4105, 8 GiB RAM, bare metal |
| OS | Ubuntu Server 24.04.5 LTS, x86-64 |
| Kernel | `6.12.115-ecat-rt`, `CONFIG_PREEMPT_RT=y` |
| CPU isolation | CPU 3 reserved with `isolcpus`; the cycle thread is pinned there for timing runs |
| IgH master | 1.6.13, commit `61cc654f5b721ddd54df0f58bdd34106d91c5359`, at `/usr/local/etherlab` |
| IgH userspace | `libethercat.so.1.2.0`, `ecrt.h` 1.6 |
| Master device | `/dev/EtherCAT0`, `crw-rw---- root:ethercat`; login user in group `ethercat` |
| Network device | `00:be:43:3f:31:05` (main), link UP |
| Slave | DIEWU `DIEWU_EtherCAT_DIO_16`, vendor `0x00000b95`, product `0x00010200`, revision 1 |
| Slave position | master 0, alias 0, position 0 |
| Distributed clocks | Not used; DIEWU free-run by owner decision. DC is implemented and unit-tested, but no DC-capable slave was available |
| Compiler | `clang` 18.1.3 on the host for the fixture's user-project build |
| Deployed package | `stc 1.0.0`, fixed package `bin/stc` SHA-256 `f5c50648…`; `lib/libstc_runtime.so` SHA-256 `3e73b73d…`; shim SHA-256 `bceeb601…` (unchanged) |
| Shim source | [tools/ethercat](../../tools/ethercat/), built in CI against pinned IgH 1.6.13 |

The host installation, kernel/IgH pairing, and service state are operator prerequisites documented
in [EtherCAT-Runbook.md](EtherCAT-Runbook.md) §2. The repository does not install IgH or kernel
modules on the host.

## 3. Architecture and configuration

- **Backend contract.** `stc-runtime` exposes a Rust `DeviceBackend` trait and a coordinator over
  one process image. Each device's `kind` selects its backend; the coordinator latches inputs,
  applies the route graph, runs the scan, publishes outputs, and aggregates per-backend status.
  EtherCAT and a mock second backend run over one image in the CI tests; WL-5.1 Modbus TCP can
  attach as a second real backend without changing the EtherCAT backend.
- **Configuration.** One source-controlled machine profile per machine is selected by
  `[io].config`. It contains the device tree, PDO map, channel map, and routes. `--io-config` is an
  addressing-only diagnostic override and does not change routes. The profile is embedded in the
  artifact and hashed.
- **PDO mapping.** The mapping is bit-exact and bounds-checked. Each sink has one source; fan-out
  is allowed and fan-in is rejected. Unbound output PDOs are zeroed at configuration.
- **Routing.** The route graph supports physical input → PLC input, PLC output → physical output,
  physical input → physical output, and PLC → PLC. PLC → PLC routes have a one-scan delay; cycles
  are allowed without a warning.
- **Fault and online change.** The backend never rewrites sent data. A fault keeps the cycle
  running, presents stale inputs, and publishes status. Recovery requires the configured slave;
  adding or removing a device is a topology change and requires a cold download. The
  configuration hash covers the profile, device tree, backend kinds, PDO/channel map, and
  effective bindings/routes; a code-only swap preserves the backend, a changed hash returns
  `STC-E-ONLINE-IO-001`.
- **Packaging and licensing.** The base `stc` package has no IgH dependency. CI builds the C shim
  against pinned IgH 1.6.13 userspace and ships it in the optional `stc-io-ethercat` artifact.
  The shim source is committed; the library is dynamically linked LGPL userspace and no kernel
  modules are redistributed.
- **Status surface.** The `ethercat-status` device exposes 14 read-only `%I` channels; the PLC
  binds the ones it needs. Master frame counters are sampled at run boundaries with
  `ethercat master`, not exposed as status channels.

The locked contract is
[IO-Routing-and-Backends.md](../Architecture/IO-Routing-and-Backends.md); the decisions are
recorded in [Decisions.md](../Decisions.md) §2, D-211–D-228.

## 4. Frame-level evidence (phases 3–5)

### 4.1 Phase 3: open-ended bring-up

A 600000-cycle, 1 ms run with open-ended (unwired/unloaded) I/O completed with
`exchanges=600000`, `faults=0`, `domain.wc_state=2`, `domain.working_counter=3`,
`master.slaves_responding=1`, `master.link_up=1`, and no send/receive diagnostic. IgH accepted
and replied to every frame; the master Tx-error and lost-frame counters were 0 before and after;
the slave reached OP; after release the slave returned to PREOP and the master to `Phase: Idle`.
The run took 648 s of wall time for 600000 requested 1 ms cycles (about 1.08 ms effective), which
is consistent with the non-RT pacing and the debug module. Shutdown does not write outputs; the
slave watchdog handles stopped frames. Physical I/O values were not verified in this phase. Full
detail and hashes: runbook §8.

### 4.2 Phase 4: software fault, status, and online change

The software-fault matrix passed fail-closed:

| Case | Result |
| --- | --- |
| Master already owned | Runtime exits 1 with `STC-E-IO-ETH-002`; no run, no outputs; master returns to Idle when the holder exits |
| Wrong identity | Startup timeout `STC-E-IO-ETH-008`; exit 1; Idle/PREOP |
| Wrong position | `STC-E-IO-ETH-003`; exit 1; Idle/PREOP |
| Startup timeout | `STC-E-IO-ETH-008`; exit 1; Idle/PREOP |
| Service stop/start, idle master | Stop/start wrapper works; master returns Idle with one PREOP slave |
| Service stop during an active run | Module cannot unload while the runtime holds the device; unit ends `failed`, runtime continues healthily, `start` restores the unit |
| Code-only online change | Backend and image preserved; `STATUSEXCHANGES` 260 → 1060 across the swap; all 14 status values healthy |
| Changed configuration | `STC-E-ONLINE-IO-001`, `cold_required=true`; active generation continues |

The 14 PLC-visible status channels were read live and matched the master/domain/slave state:
`STATUSPHASE=2`, `STATUSMASTERACTIVE=true`, `STATUSSLAVESRESPONDING=1`, `STATUSLINKUP=true`,
`STATUSLASTERROR=0`, `STATUSRECOVERIES=0`, `STATUSWCSTATE=2`, `STATUSWORKINGCOUNTER=3`,
`STATUSINPUTSTALE=false`, `STATUSEXCHANGES` advancing, `STATUSFAULTS=0`,
`STATUSSLAVEALSTATE=8`, `STATUSSLAVEONLINE=true`, `STATUSSLAVEOPERATIONAL=true`. The runtime
unit tests cover a non-fatal fault keeping the cycle running, unchanged outputs, stale inputs,
recovery after the window, and one backend's fault not touching another backend's data. No
physical fault was injected in this phase. Full detail and hashes: runbook §9–§10.6.

### 4.3 Phase 5: cycle-time sweep

The sweep ran the release fixture with `SCHED_FIFO` 90 on CPU 3, a non-CPU-bound 16I/16O program,
60 s per screening step, and a 600 s confirmation at the fastest passing step. The master
counters were not reset between steps; every passing row reported `state=running`, `faults=0`,
WC state 2, current inputs, link up, one responding slave, no master error, and no recovery.

| CT | Window | Result | Cycles | Skipped | Exec p99 / max (µs) | Wake p99.9 / max (µs) | Master Tx / lost |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 ms | 60 s | pass | 118302 | 0 | 49.3 / 60.7 | 27.4 / 30.8 | 0 / 1953 → 1953 |
| 500 µs | 60 s | pass | 236589 | 0 | 48.8 / 61.7 | 27.9 / 31.9 | 0 / 1953 → 1953 |
| 250 µs | 60 s | pass | 473195 | 0 | 47.9 / 63.9 | 28.2 / 32.1 | 0 / 1953 → 1953 |
| 200 µs | 60 s | pass | 591527 | 0 | 47.1 / 61.4 | 28.0 / 32.0 | 0 / 1953 → 1953 |
| 150 µs | 60 s | pass | 788678 | 0 | 47.8 / 68.3 | 28.3 / 96.8 | 0 / 1953 → 1953 |
| 100 µs | 60 s | **fail** | 1182738 | **283** | 48.8 / 60.6 | 45.8 / 133.3 | 0 / 1953 → 1953 |
| 150 µs | 600 s | pass | 4388666 | 0 | 49.0 / 68.4 | 28.2 / 75.1 | 0 / 1953 → 1953 |

The 100 µs failure is host wake-side jitter: the 283 skipped periods were one-period events with
wake lateness 53.4–133.3 µs, irregularly spaced; the execution distribution stayed below the
half-period gate; the frame path completed all 1182738 exchanges with WC state 2, no faults,
current inputs, unchanged master Tx/lost counters, and no recovery. The metrics writer was moved
off the cycle thread in `77cab14` (dedicated `stc-rt-metrics` thread, bounded channel, drop
accounting); every final run reports `dropped_records:0`. The earlier 500 µs failure was the
1 MiB flush on the cycle thread and is superseded. The strict `missed_deadline` counter equals
the cycle count by design (any positive wake lateness), so it is not a pass gate. Full
distributions and hashes: runbook §10.7–§10.8.

The fastest maintainable CT is 150 µs. The first failing CT is 100 µs, and the failure mode is
host wake jitter, not the EtherCAT frame path. 75/50/25 µs were not run because the sweep stops
at the first failure.

## 5. Physical I/O evidence (phase 6)

### 5.1 Harness and supply limitation

The bench harness wires OUT1→IN1 through OUT16→IN16 and ties input commons S1–S4 to the
output-stage positive supply. DIEWU outputs are NPN sinking; an ON output pulls its terminal low,
and the input opto between S and IN is the load, so a driven output reads back as a set input.
No external load is allowed on the bench.

The module logic is powered from the 24 V barrel jack. The output stage requires a separate
`DC5-48V` supply; the 2-pin connector is not a power output. The final 2026-11-01 run used a second
24 V supply on the output stage, with S1–S4 tied to its `VCC`. The automated all-points matrix then
passed 16/16: every output driver turned on its corresponding input opto (`FailMask=0`,
`ObservedMask=0xFFFF`), and the 16 physical input paths are verified. An earlier attempt without
the second supply is retained only as the defect history below.

### 5.2 Fault and recovery

One `stc debug` session (1 ms, `--cycles 0`, `HOLD TRUE`) injected Fault A (host-end cable
unplug/replug, slave powered) and Fault B (DIEWU power cycle). Fault C (slave-end unplug) was not
run separately: with one direct cable, host-end and slave-end unplug are the same link-loss path,
and Fault A reconnected the same configured slave while it stayed powered.

The session recorded one extra, unobserved fault/recovery episode before the first post-HOLD
block. The pre-HOLD block was `faults=0`/`recoveries=0`/`OUTPUTMASK=0`/`INPUTMASK=0`; the
post-HOLD baseline was `faults=1`/`recoveries=1`/65535/65535. The recorded Fault A is episode 2
and Fault B is episode 3. This is a manual-sequence deviation, accepted by the owner, not a
product failure.

| Observation | Baseline (post-HOLD) | A during (ep. 2) | A after | B during (ep. 3) | B after |
| --- | --- | --- | --- | --- | --- |
| `STATUSFAULTS` | 1 | 2 | 2 | 3 | 3 |
| `STATUSINPUTSTALE` | false | true | false | true | false |
| `STATUSRECOVERIES` | 1 | 1 | 2 | 2 | 3 |
| `STATUSWCSTATE` | 2 | 0 | 2 | 0 | 2 |
| `STATUSWORKINGCOUNTER` | 3 | 0 | 3 | 0 | 3 |
| `STATUSLINKUP` | true | false | true | false | true |
| `STATUSSLAVEALSTATE` | 8 | 0 | 8 | 0 | 8 |
| `STATUSSLAVEOPERATIONAL` | true | false | true | false | true |
| `SCANCOUNT` | 51872 | 70591 | 119703 | 183096 | 196078 |
| `OUTPUTMASK` | 65535 | 65535 | 0 | 65535 | 65535 |
| `INPUTMASK` | 65535 | **65535** | 0 | **65535** | 65535 |

`STATUSPHASE` stayed 2 (IgH Operation) during both faults. `SCANCOUNT` advanced through every
interval, so the cycle and PLC kept running through both link-down windows. `OUTPUTMASK` stayed
65535 through both faults: the runtime did not rewrite the PLC output image. `INPUTMASK` stayed
65535 through both recorded faults: the runtime presented the last received image as stale.
`HOLD FALSE` during episode 2 gave `OUTPUTMASK=0`, `INPUTMASK=65535`. `STATUSINPUTSTALE` was true
during each fault and cleared on recovery; `STATUSRECOVERIES` incremented once per recovery; the
same configured slave returned without a cold download or host restart. The physical output LEDs
cleared during each fault and returned on replug/repower, consistent with the slave watchdog
dropping the outputs and the master re-sending the held image on recovery.

**Stale-input defect and fix.** An earlier powered run on the phase-5 package showed
`STATUSINPUTSTALE=true` but `INPUTMASK=0` during both faults. The coordinator applied the global
input defaults to the whole input area for `Null`/`Ethercat` global backends; a faulted backend
skips its input copy, so the bound inputs were zeroed before the stale flag was exposed. The fix
(commit `d694266`, CI run 41367423862) builds a `claimed_input` map and applies defaults only to
unclaimed input bits; the regression test covers a fault under an `Ethercat` global backend. The
final run on the fixed package verifies the corrected behavior.

Restore state after the session: `ethercat master` was `Phase: Idle`, `Active: no`, one slave;
`ethercat slaves` was `0 0:0 PREOP + DIEWU ...`; no `stc` process remained; Tx errors were 0 in
all four boundary samples. Lost frames were 18856 → 18856 for the all-points run (delta 0) and
18856 → 24426 for the fault session (delta 5570); the fault-session increase is the frames sent
during the link-down and power-off windows.

The final all-points log is `/home/r2k/stc-wl52/logs/loopback-fix-20261101T174924.log`, SHA-256
`06d295c5f45c4fcaf468d365bc98873e3164a5c1e4da1818aa3c7fc131e85b41`; the final fault transcript is
`/home/r2k/stc-wl52/logs/fault-session-fix-20261101T174958.log`, SHA-256
`14d7689bed749aa12770196722ed8c6e696e0cff7d9afe3e305717cd00b3de6e`. The earlier
`fault-session-20261101T172040.log` (`b9f11faa…`) is the phase-5-package run that exposed the
defect. The procedure and the filled results tables are
[PHASE6.md](../../tests/ethercat/diewu/PHASE6.md) §8; the runbook summary is
[EtherCAT-Runbook.md](EtherCAT-Runbook.md) §13.

### 5.3 What this proves and what it does not

Proved: the 16 output drivers turn on the 16 input optos (all-points 16/16); the runtime holds the
PLC output image through link loss and slave power loss, keeps scanning, presents the last
received input image as stale, exposes the fault in the status registers, recovers the same
configured slave without a cold download or host restart, and returns the master/slave/service
state to Idle/PREOP with no runtime process left. The physical output LEDs dropped during each
fault and returned on recovery, consistent with the slave watchdog and the held image. The
all-points acceptance criterion is met.

Not proved: output load capacity, input threshold margins, isolation, field devices, a fail-safe
output state, recovery time (only sampled within the procedure windows), separate Fault C,
multi-slave behavior, or servo/DC hardware. The manual fault-session counters carry the extra
unobserved episode; that is a test-record deviation, not a product limitation.

## 6. Status and diagnostics

The `ethercat-status` device is a read-only `%I` layout with 14 channels: master phase, master
active, slaves responding, link up, last error, recoveries, WC state, working counter, input
stale, exchanges, fault episodes, slave AL state, slave online, and slave operational. The PLC
binds the channels it needs. The removed lost-frame, Tx-error, and frame-loss channels are
intentionally not part of the block; `ethercat master` is the evidence of record at run
boundaries.

On a fault the status device publishes `faults` incrementing, `input.stale=true`, WC state
incomplete, link down, the slave offline/non-operational, and the last error; on recovery it
clears stale, increments recoveries, and restores WC/AL/online/operational. The runtime keeps the
output image unchanged throughout; the PLC decides whether to hold, stop, or drive a safe state.
This is not a safety function and does not replace a safety PLC.

## 7. Limitations

The full list is [KNOWN-LIMITATIONS.md](../../KNOWN-LIMITATIONS.md) §8. The main ones:

- Hardware-in-the-loop evidence is Linux x86-64 only; the AArch64 backend is not hardware-verified.
- One DIEWU 16I/16O slave; multi-slave configurations are covered by configuration/mapping and
  mock tests, not HIL.
- No servo hardware. The generic PDO/DC path is implemented and unit-tested; the DIEWU runs
  free-run, so distributed clocks were not exercised on hardware.
- Fault C is covered by A rather than run separately; recovery time was sampled within the
  procedure windows, not measured. The fault-session counters carry the extra unobserved episode
  (a manual-sequence deviation, not a product failure).
- The fastest maintainable CT is 150 µs on this host; 100 µs fails on host wake jitter; 75/50/25 µs
  were not run. The debug build uses most of a 1 ms budget; release is the timing evidence.
- The master lost-frame counter is cumulative; the phase-6 samples are 18856 → 18856 for the
  all-points run and 18856 → 24426 for the fault session (delta 5570).
- IgH is an operator-installed host prerequisite coupled to the kernel; the repository does not
  distribute kernel modules.
- The backend is not a safety function, not a motion controller, and not a substitute for a
  safety PLC.

## 8. Evidence index

- Phase 3: runbook §8.2–§8.3 (run log, master samples, package/shim and fixture hashes).
- Phase 4: runbook §9.1–§9.7 (fault matrix, status values, online change, samples and hashes).
- Phase 5: runbook §10.6–§10.8 (service stop/start, CT sweep, distributions, per-run hashes).
- Phase 6: runbook §13 and [PHASE6.md](../../tests/ethercat/diewu/PHASE6.md) §8 (all-points 16/16,
  powered fault matrix, restore state, log/transcript hashes, loopback file hashes); the
  stale-input fix is commit `d694266`, CI run 41367423862.
- Packaging: CI-built `stc-io-ethercat` artifact and committed shim source in
  [tools/ethercat](../../tools/ethercat/).

Raw logs remain on `ecat-host` under `~/stc-wl52/logs/`; they are not committed. The hashes in
the runbook and procedure identify the retained evidence.

## 9. Conclusion

WL-5.2 delivers a working IgH EtherCAT backend with the multi-backend seam, the source-controlled
machine profile and route graph, the PLC-visible status device, transparent fault/recovery, and
online-change protection. Frame-level criteria (phases 3–5) are met, including a 600000-cycle
clean run at 1 ms and a 150 µs maintainable cycle time. The phase-6 powered all-points matrix
passed 16/16, and the powered fault/recovery cases A and B passed on the fixed package; the
earlier supply-limited attempt exposed a coordinator stale-input defect that was fixed in
`d694266` and re-verified on hardware. All phase-6 acceptance items are met. The remaining limits
are scope limits: one host and one slave, no servo hardware, DIEWU free-run/DC not exercised,
the 150 µs CT floor, x86-64-only HIL, and the manual counter deviation in the fault session.

Accept WL-5.2 with those scope limits. No safety claim is made.
