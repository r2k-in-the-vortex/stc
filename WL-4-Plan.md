# WL-4 Plan — PREEMPT_RT Verification on x86-64

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-29  
Branch: `WL-4` (sub-plan of `v2.0.0-planning`)  
Status: Decisions locked 2026-10-29; implementation not started. Host access is verified and the
untuned baseline is captured. The tuned boot profile is deferred future work (§6.2).  
Parent: [V2.0.0-PLAN.md](V2.0.0-PLAN.md) §4 (WL-4)  
Inputs: [FutureWishlist.md](FutureWishlist.md) WL-4, [Performance.md](specification/Verification/Performance.md),
[Runtime and Execution](specification/Architecture/Runtime-and-Execution.md),
[Online Change and GOT](specification/Architecture/Online-Change-and-GOT.md),
[CLI and Manifest](specification/Architecture/CLI-and-Manifest.md),
[KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md)

## 1. Objective

Produce reproducible evidence of how the released runtime behaves on the available x86-64
PREEMPT_RT host as configured (untuned): cycle wake-up jitter, cycle execution time, missed
deadlines, steady-state page faults and allocations, control/status-thread cost, and
online-change prepare, activation, and rollback under load.

The host is a run target. All builds happen on development or CI hosts, and the results describe
only the host and configuration measured. WL-4 makes no hard real-time or safety claim.

## 2. Locked scope and boundaries

In scope:

- `stc run` through the prebuilt launcher, with a fixed cycle time.
- Online change prepare/activation and one-cycle rollback, driven by prebuilt candidates.
- A benchmark corpus with compatible, incompatible, and trapping candidates.
- A machine-readable harness, raw results, a report, and a host runbook.
- The smallest runtime instrumentation needed to measure the above; no language or compiler
  feature.

Out of scope:

- EtherCAT, Modbus, motion, debug, and WebUI work owned by other targets.
- Building `stc` or project artifacts on the RT host.
- Affinity, priority, or real-time scheduling defaults in normal `stc run` behavior.
- Tuned boot profiles and kernel command-line changes. The deferred profile is recorded in §6.2
  for future work; v2.0.0 measures the untuned host.
- Hard real-time guarantees, certification, or safety functions.
- Portable timing claims; every result is host- and configuration-specific.

## 3. Host access and baseline (verified 2026-10-29)

The host note in `.env/RT_Host.md` (local only, gitignored) defines `RT_HOST_SSH_TARGET`, used
here as `$RT_HOST`. Key login works without a password, and the host is reachable from the
Windows development machine. The login user is a sudoer; sudo requires interactive
authentication (`sudo-rs`) and no passwordless rule is configured. The campaign must not store
credentials (§5, R-2).

### 3.1 Identity

| Item | Value |
| --- | --- |
| Access | Key login to `$RT_HOST`, verified 2026-10-29 |
| OS | Ubuntu 25.10 (Questing Quokka), x86-64 |
| Hardware | TianBei N1 PRO desktop, firmware `HCN150_MI3_0.10` (2025-04-18) |
| Virtualization | Bare metal (`systemd-detect-virt` reports `none`) |
| Network | LAN address from the local note on `enp3s0` (DHCP), default route via the local gateway; Docker bridges `172.17.0.0/16` and `172.18.0.0/16` |
| Uptime at probe | 21 days; load average 0.08 / 0.04 / 0.01 |

### 3.2 Kernel and PREEMPT_RT configuration

| Item | Value |
| --- | --- |
| Kernel | `Linux 6.17.999-8-generic #8 SMP PREEMPT_RT`, built 2026-01-26 |
| Source | Ubuntu `6.17.0-8.8` source plus the RT patch, built locally; source tree and packages remain in `~/linux-6.17.0` |
| Preemption | `/sys/kernel/debug/sched/preempt` reports `(full) lazy`; `CONFIG_PREEMPT_RT=y`, `CONFIG_PREEMPTION=y`, `CONFIG_PREEMPT_DYNAMIC=y` |
| Timers and RCU | `CONFIG_HZ_1000=y`, `CONFIG_NO_HZ=y`, `CONFIG_NO_HZ_FULL=y`, `CONFIG_RCU_NOCB_CPU=y`, `CONFIG_HIGH_RES_TIMERS=y` |
| Interrupts | `CONFIG_IRQ_FORCED_THREADING=y`; `sched_rt_runtime_us=950000`, `sched_rt_period_us=1000000`; `CONFIG_RT_GROUP_SCHED` is not set |
| Clocksource | `tsc` (available: `tsc`, `acpi_pm`); `constant_tsc`, `nonstop_tsc`, `tsc_known_freq` |
| Tracers | `timerlat`, `osnoise`, `hwlat`, `blk`, `mmiotrace`, `function_graph`, `wakeup_dl`, `wakeup_rt`, `wakeup`, `function`, `nop` |
| Security | AppArmor, lockdown, IMA/EVM, Yama; `sshd` active |
| Boot command line | `BOOT_IMAGE=/boot/vmlinuz-6.17.999-8-generic root=UUID=... ro crashkernel=...`; no isolation or RT tuning parameters |

### 3.3 CPU, memory, storage, and power

| Item | Value |
| --- | --- |
| CPU | Intel N150 (GenuineIntel, family 6 model 190), 4 cores / 4 threads, 1 socket, no SMT, 1 NUMA node |
| Cache | L1d 128 KiB, L1i 256 KiB, L2 2 MiB, L3 6 MiB |
| Frequency | 700 MHz to 3.60 GHz; boost supported and active; `no_turbo=0` |
| Current governor | `powersave` on all CPUs; `intel_pstate` active; EPP `balance_performance`; 1.57 GHz at probe |
| Available governors | `performance`, `powersave` |
| Idle states | POLL 0 µs, C1E 2 µs, C6 195 µs, C8 260 µs, C10 660 µs |
| Isolation | None: `isolated` is empty, `nohz_full` is null, CPUs 0-3 online, IRQ default affinity `f` |
| NIC IRQs | 141-145 (`enp3s0`), affinity `0-3` |
| Memory | 10 GiB (11.1 GB reported), 4 GiB swap file (65 MB used), no hugepages |
| Storage | 336 GB root, 229 GB free; `/tmp` is a 5.5 GB tmpfs |
| Thermal | Throttle counters at probe were zero |

### 3.4 Tooling

Present: `cyclictest` 2.60, `rtla` (timerlat and osnoise), `hwlatdetect`, `stress-ng`, `perf` 6.17.2,
`trace-cmd`, `taskset`, `chrt`, `numactl`, `cpupower`, `turbostat`, `rsync`, `scp`, `git`,
`make`, `gcc` 15.2.0, `clang`, `python3` 3.13.7, `jq`, `systemd-run`, `flock`, `timeout`,
`setsid`, `ionice`.

Missing: `pip`, `schedtool`, `hyperfine`, `powertop`, and the Python `psutil` module. The harness
must use the Python standard library only.

Rust: `rustc` 1.85.1 from the Ubuntu package and a rustup tree exist, but `cargo` is not on the
non-interactive PATH and the workspace pins Rust 1.99.1. The host is not a build environment.

### 3.5 Privileges and measurement capability

| Item | Value |
| --- | --- |
| User | `r2k`, uid 1000, member of `sudo`, `adm`, `cdrom`, `dip`, `plugdev`, `users` |
| sudo | Interactive authentication required; no NOPASSWD rule; `sudo-rs` |
| RT priority | `ulimit -r` is 0; `chrt -f 10 true` fails with `EPERM`; unprivileged `cyclictest` refuses to start |
| Memory lock | `ulimit -l` is 8192 KiB |
| perf | `kernel.perf_event_paranoid=4`; unprivileged perf events are unavailable |
| Tracing | tracefs is not writable by `r2k`; `/sys/kernel/debug` is root-only |
| cgroups | cgroup v2, session scope under `user.slice` |
| Services | 27 running services, including `chrony`, `docker`, `containerd`, `smbd`, `nmbd`, `ModemManager`, `multipathd`, `systemd-networkd`, `systemd-resolved`, `ssh`, `rsyslog` |

A meaningful campaign needs a privileged path for FIFO scheduling, `mlockall`, perf, and tracefs.
The operator can authenticate interactively; no credential is stored in the repository or
harness. R-2 fixes the mechanism.

### 3.6 Build and artifact state

- No `stc` installation or staged `stc` artifacts were found in the searched paths on the host.
- The released `stc-1.0.0-x86_64-unknown-linux-gnu.tar.gz` was transferred and smoke-tested:
  `bin/stc --version` reports `stc 1.0.0`, and `ldd` shows only `libgcc_s`, `libm`, `libc`, and
  the dynamic loader. The host glibc is 2.42, so the CI-built package (ubuntu-24.04, glibc 2.39)
  runs unchanged.
- `stc compliance --message-format json` reports profile `stc-2013-oop-utf8-v1`, runtime
  capabilities `online-change`, `got-manifest`, `debug-monitoring`, and `safety: not-a-safety-plc`.
- CI publishes `stc-package-ubuntu-24.04` (about 11 MB) for every green run; release `v1.0.0` also
  carries the Linux x86-64 package. These are the artifact sources.
- The Windows development host has Rust 1.99.1, clang 23.1.4 (MSVC), and Docker Desktop. WSL has
  only the `docker-desktop` distribution and no Linux userland, so it is not a build fallback as
  configured.
- `stc build --target` off-host emits IR and objects; `--emit=exe` and `--emit=lib` are rejected
  for a non-host target. `--emit=module` is not rejected, but the Windows host has no Linux
  sysroot: a test build produced the ELF object and manifests, then failed at `ld.lld` with
  missing `crti.o`, `-lgcc`, `-lgcc_s`, and `-lc`. CI (or a Linux container) is therefore the
  build path for complete module and launcher artifacts.
- Prior art on the host (`~/online_change_poc`, `~/plc_runtime`, `~/rusty`, `~/ethercat`) is
  unrelated to `stc` and must not be mixed into the campaign directory.

### 3.7 Preliminary measurements (untuned, informational)

Measured 2026-10-29 on the current untuned regime: `powersave` governor, no CPU isolation, all
background services running, 15-second cyclictest and 5-second rtla samples. These numbers prove
the tooling works and are not campaign evidence.

`cyclictest -m -S -p90 -i200 -h1000 -D15 -q` (one FIFO:90 thread per CPU, 75,000 samples per CPU):

| CPU | Min | Avg | Max |
| --- | --- | --- | --- |
| 0 | 1 µs | 4 µs | 16 µs |
| 1 | 2 µs | 4 µs | 14 µs |
| 2 | 2 µs | 4 µs | 21 µs |
| 3 | 2 µs | 4 µs | 17 µs |

No histogram overflows at 1000 µs.

`rtla timerlat hist -d 5s` (all CPUs):

| Class | Min | Avg | Max |
| --- | --- | --- | --- |
| IRQ | 1 µs | 3 µs | 17 µs |
| Thread | 2 µs | 8 µs | 33 µs |
| User | 3 µs | 12 µs | 41 µs |

`rtla osnoise hist -d 5s`:

| CPU | Min | Avg | Max | Overflows |
| --- | --- | --- | --- | --- |
| 0 | 1 µs | 3.89 µs | 1024 µs | 6 |
| 1 | 1 µs | 3.07 µs | 941 µs | 2 |
| 2 | 1 µs | 3.58 µs | 1033 µs | 2 |
| 3 | 1 µs | 5.17 µs | 3010 µs | 9 |

The multi-millisecond osnoise maxima are the main unexplained signal. They may come from firmware
SMI, deep C-state exits, background services, or the short sample; the campaign must resolve this
with longer runs, a quiet regime, `hwlatdetect`, and a C-state-limited boot profile.

## 4. Findings that shape the plan

1. **The kernel is capable.** PREEMPT_RT full/lazy preemption is active, high-resolution timers
   and the full RT tracer set are built in, and the host is bare metal. This is a valid RT run
   target.
2. **The current regime is untuned.** Deep C-states reach 660 µs exit latency, the governor is
   `powersave`, IRQs can land on any CPU, and there is no isolation. Busy-loop cyclictest looks
   good, but event-driven wake-up and background noise are not characterised. v2.0.0 measures
   this regime as configured; tuning is deferred (§6.2).
3. **Privilege is the setup gate.** Without FIFO scheduling, `mlockall`, perf, and tracefs, the
   harness cannot produce RT evidence. This is a host configuration task, not a product change.
4. **The runtime has no RT measurement mode.** `run_loop` uses a relative `std::thread::sleep(period - elapsed)`,
   has no absolute deadline tracking, no per-cycle timestamps, no missed-deadline counter, no
   cycle-thread priority or affinity control, and no page-fault or allocation counters. The cycle
   thread runs at the process default; the control/status thread is lowered by nice +10 only.
   WL-4 cannot meet its evidence list without a small opt-in instrumentation path (R-3).
5. **Online change can be driven without building on the host.** The control channel is a
   user-only local socket or pipe carrying newline-delimited JSON (`deploy`, `status`, `download`,
   `stop`) with a nonce. `deploy` takes module and manifest paths and runs load/prepare/activate
   between scans on the cycle thread. `stc run --watch` must not be used on the host because it
   rebuilds there.
6. **Prepare and activation need separate measurement.** The `deploy` response's `prepare_ns`
   covers candidate open and the compatibility report, not full load/prepare, and the response
   carries no activation duration. The harness needs cycle-boundary markers or the R-3 metrics.
7. **The control channel is itself a jitter source.** Deploy work runs on the cycle thread in v1.
   The campaign must report it as maintenance latency, not hide it.
8. **Artifact transfer is proven at the compiler level.** The Linux package runs on the host with
   only base glibc symbols, and the package is available from CI and the release. Project bundles
   still need a Linux build step, which CI provides.
9. **Four cores are shared in the untuned campaign.** The cycle thread, services, the harness,
   and load all compete for the same CPUs. A future tuned profile would dedicate CPU 3 and leave
   three CPUs for housekeeping (§6.2); the untuned load cases must be sized accordingly and
   recorded.

## 5. Decisions

Decisions were locked on 2026-10-29. **Accepted** means the owner confirmed the choice;
**Default** means the choice follows from the host and tooling evidence and may be adjusted
during implementation without a scope change; **Deferred** means the work is out of v2.0.0.

| ID | Decision | Status |
| --- | --- | --- |
| R-1 | Build the Linux GNU compiler package and the benchmark bundle in CI on `ubuntu-24.04`; transfer with `scp`/`rsync`; never build on the RT host. Docker Desktop is an optional local Linux build fallback; WSL is not usable as configured. | Default |
| R-2 | Install one root-owned wrapper (or systemd unit) for the campaign and a narrow NOPASSWD sudoers entry for that wrapper only. Do not store credentials in the repository, harness, or data. Do not permanently relax `perf_event_paranoid` or tracefs permissions. | Accepted |
| R-3 | Add an opt-in RT measurement mode to `stc run`/the launcher: absolute `clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME)` pacing, per-cycle metrics (deadline, wake, scan start/end, execution, lateness, missed deadlines, page faults, allocations), a configurable cycle-thread FIFO priority, and a cycle-thread CPU affinity. Default behavior and normal artifacts are unchanged. | Accepted |
| R-4 | With R-3 active, keep control/status and debug threads at `SCHED_OTHER` with nice +10; only the cycle thread gets FIFO. | Accepted |
| R-5 | v2.0.0 measures the untuned host only. The tuned boot profile is deferred future work, recorded in §6.2. It is revisited only if an untuned run misses a deadline or the report review judges jitter or latency unsatisfactory. | Accepted (deferred) |
| R-6 | Implement the harness in Python 3 standard library under `tools/rt/`, emitting gzip JSONL raw samples plus a summary JSON and a Markdown summary. | Default |
| R-7 | Add a deterministic benchmark project under `tests/rt/` with a configurable workload and four online-change candidates: code-only compatible, field add/remove compatible, incompatible (cold required), and trapping (automatic rollback). Build it in CI and transfer the complete output directory. | Accepted |
| R-8 | Commit the report and summary to `specification/Verification/RT-Host.md` and keep raw samples as campaign artifacts with SHA256. Update `Performance.md` and `KNOWN-LIMITATIONS.md` with the host-specific result and limits. | Accepted |
| R-9 | Three repetitions per regime at 1 ms, plus one 10 ms secondary run per regime; each run is 10 minutes. Report min, p50, p90, p99, p99.9, max, counts, overflows, and missed deadlines. No timing threshold is a pass/fail gate. | Accepted |
| R-10 | No affinity, priority, or RT pacing defaults in production; no hard real-time or safety claim; no timing gate in CI. | Accepted |

## 6. Measurement matrix

| Measurement | Method | Regime | Evidence |
| --- | --- | --- | --- |
| System noise floor | `cyclictest` 10 min, one FIFO:90 thread per CPU, 1000 µs histogram; `rtla timerlat hist`; `rtla osnoise hist`; `hwlatdetect` | R0 | Raw histograms, min/avg/max, overflows, host conditions |
| Cycle wake-up jitter | R-3 metrics: `wake - deadline` per cycle; cross-check with `rtla timerlat` | R0, R1, R2 | JSONL samples, percentiles, max |
| Cycle execution time | R-3 metrics: scan start/end and duration; `perf stat` for cycles/instructions | R0, R1, R2 | Percentiles, distribution, workload description |
| Missed deadlines | R-3 metrics: wake after deadline or scan end after the next deadline | R0, R1, R2 | Count, lateness distribution, first occurrence |
| Page faults and allocations | `/proc/<pid>/stat` and `/proc/<pid>/status` deltas; `perf stat` page faults; host transient-allocator counters | R0, R1, R2 | Steady-state counts per cycle, RSS |
| Control/status-thread cost | A/B run with control disabled, enabled idle, enabled with status polling at 100 Hz | R0 | Cycle-time delta, sample counts |
| Online-change prepare | Control `deploy` response plus R-3 prepare markers | R0, R1, R2 | Prepare duration, candidate size, root count |
| Activation | R-3 cycle-boundary markers around the activate call | R0, R1, R2 | Activation duration, cycle where it occurred |
| Rollback under load | Deploy the trapping candidate; R-3 markers plus status transition | R1, R2 | Time to restore, restored generation, preserved values |
| Incompatible change | Deploy the layout-changing candidate | R1, R2 | `cold_required` response, no activation, old generation continues |
| Load | `stress-ng` CPU/memory/I/O and a network generator; R1 confines load to other CPUs, R2 pins it to the cycle thread's CPU; exact commands recorded | R1, R2 | Load PIDs, CPU utilization, interference results |

### 6.1 Regimes (v2.0.0)

- **R0 — untuned, quiet.** Current boot: `powersave`, no isolation, default IRQ affinity, normal
  background services, swap as configured.
- **R1 — untuned, other-CPU load.** R0 plus CPU, memory, I/O, and network load confined to CPUs
  other than the cycle thread's CPU.
- **R2 — untuned, same-CPU load.** R0 plus a labelled case with load pinned to the cycle thread's
  CPU.

The cycle thread's CPU is set with the R-3 affinity option and recorded for every run. If the
affinity option is not used, the observed CPU is recorded and R1/R2 are defined relative to it.
No boot parameter, governor, C-state limit, IRQ affinity, service set, or swap setting is changed
in v2.0.0.

Every run records the kernel, command line, firmware, governor, C-state limit, IRQ affinities,
service list, swap state, load commands, `stc` package version and SHA256, launcher build ID,
manifest build IDs, cycle time, workload, and run duration.

### 6.2 Deferred tuned profile (future work, not v2.0.0)

The tuned profile is recorded so a future version can revisit it without re-planning from
scratch. It is out of scope for v2.0.0 and must not be applied during the campaign.

- Boot: `isolcpus=3 nohz_full=3 rcu_nocbs=3 irqaffinity=0-2 intel_idle.max_cstate=1`.
- Power: `performance` governor and performance EPP; IRQ affinity on CPUs 0-2.
- Services: reduce background services to the documented minimum; swap off.
- If the profile is adopted later, verify after its reboot: `cat /proc/cmdline`,
  `/sys/devices/system/cpu/isolated`, `nohz_full`, the governor, the idle-state latencies, and
  the IRQ affinities.
- Revisit trigger: any missed deadline in the v2.0.0 untuned campaign, or a report review that
  judges jitter or latency unsatisfactory.
- Cost and risk: a reboot and maintenance window, and effects on other host users. Adopting it
  later is a separate scope decision, not a silent extension of WL-4.

### 6.3 Harness and data

Implemented in `tools/rt/campaign.py` (2026-10-29).

- The harness orchestrates runs, starts the launcher with `--control`, generates an ephemeral
  nonce that is never logged, drives deploy/status over the local socket, starts and stops load,
  samples `/proc`, and writes one gzip JSONL stream per run.
- Raw samples: `cycles.jsonl.gz` (the `stc-rt-metrics/1` stream) and `proc.jsonl.gz`; the run
  metadata, host baseline, artifact digests, regime, workload, and cycle time are in
  `summary.json` and `host.json`.
- Summary: `summary.json` (`stc-rt-summary/1`) with counts, percentiles, missed deadlines, fault
  and allocator deltas, deploy/rollback evidence, and a human-readable `summary.md`.
- The harness never reads or writes credentials; privileged setup is external (R-2).
- The control endpoint stays user-only and is removed after the run.

#### 6.3.1 RT metrics JSONL (`stc-rt-metrics/1`)

The launcher writes one line per record when `--rt-metrics <path>` is set:

```json
{"record":"header","protocol":"stc-rt-metrics/1","build_id":"<hex|null>","cycle_time_ns":1000000,"priority":90,"cpu":3,"pid":1234,"started_monotonic_ns":0,"fault_interval":1000}
{"record":"cycle","cycle":1,"generation":7,"deadline_ns":0,"wake_ns":0,"scan_start_ns":0,"scan_end_ns":0,"wake_lateness_ns":0,"execution_ns":0,"scan_overrun_ns":0,"missed_deadline":false,"skipped_periods":0,"minor_faults":null,"major_faults":null,"allocations":null,"frees":null}
{"record":"summary","cycles":0,"missed_deadlines":0,"skipped_periods":0,"max_wake_lateness_ns":0,"max_execution_ns":0,"max_scan_overrun_ns":0,"last_minor_faults":null,"last_major_faults":null,"last_allocations":null,"last_frees":null,"finished_monotonic_ns":0}
```

- Timestamps are `CLOCK_MONOTONIC` nanoseconds. `deadline_ns` is the scheduled start of the
  cycle; `wake_lateness_ns` is `wake - deadline` (signed); `scan_overrun_ns` is the completion
  after `deadline + cycle_time`; `missed_deadline` is true when either the wake or the completion
  is late.
- `skipped_periods` is the number of whole cycles skipped after this cycle when the next deadline
  had already passed; the next record's `deadline_ns` reflects the jump.
- Fault and allocator fields are sampled at cycle 1 and every `fault_interval` cycles, and are
  `null` on other records. The summary repeats the last sample.
- The header's `pid`, `priority`, and `cpu` describe the cycle thread; `cpu` is `null` when no
  affinity was requested.
- The file is opened before the first deadline and the header is written before the first cycle.
  Records are buffered and flushed at exit; the summary is written on normal exit, stop, or
  interrupt.

### 6.4 Benchmark workload

- A deterministic ST project with a configurable arithmetic and function-block workload, no I/O,
  and a task interval equal to the cycle time.
- Primary cycle time 1 ms, secondary 10 ms. The 1 ms runs are repeated three times per regime;
  the 10 ms run is a single secondary per regime.
- Workload levels: light (well under the budget), medium (about half the budget), and heavy
  (near the budget), each labelled and measured separately.
- Separate candidates for each online-change case in R-7.

## 7. Work breakdown

| Phase | Work | Acceptance |
| --- | --- | --- |
| 0 | Lock R-1–R-10, install the privileged wrapper, and record this baseline; no host tuning | Decisions recorded; wrapper installed and tested; untuned host state recorded |
| 1 | CI artifact pipeline: Linux package plus `rt-bench` bundle with all candidates; transfer and verify procedure | Package and bundle run on the host; SHA256 verified; no build on the host; launcher finds its runtime library |
| 2 | R-3 measurement mode: absolute pacing, metrics schema, priority/affinity, page-fault and allocation counters; unit and ignored timing tests | Metrics schema fixed; default behavior unchanged; workspace tests pass; release artifacts unchanged except opt-in code |
| 3 | Harness: control client, deploy/rollback, load, `/proc` sampling, summaries, tests | Dry run on the host produces a complete JSONL and summary; reruns produce the same schema |
| 4 | Campaign: untuned R0, R1, R2; cyclictest, timerlat, osnoise, hwlat; three 1 ms repetitions plus the 10 ms secondary; online-change and rollback cases | Every matrix row has raw data and a summary; host conditions recorded; failed runs kept and explained |
| 5 | Report and runbook: `specification/Verification/RT-Host.md`, `Performance.md`, `KNOWN-LIMITATIONS.md`; record the deferred tuned profile (§6.2); review | Report states host, method, distributions, and limits; runbook repeats the untuned campaign from a clean boot; no safety claim |

## 8. Exit criteria

- The artifact pipeline is documented and reproducible, and the RT host never builds.
- The harness produces comparable, machine-readable results with a stable schema.
- The report states the host baseline, the untuned regimes, the workload, and the measured
  distributions for wake-up jitter, execution time, missed deadlines, page faults, allocations,
  and control/status cost.
- Online-change prepare, activation, and one-cycle rollback are measured under other-CPU and
  same-CPU load, including an incompatible candidate that is rejected and a trapping candidate
  that rolls back.
- The tuned boot profile is recorded as deferred future work and is not applied in v2.0.0.
- The runbook repeats the untuned campaign from a clean boot without using the host as a build
  environment.
- No hard real-time or safety claim is made, and results are labelled host- and
  configuration-specific.

## 9. Runbook (draft)

### 9.1 One-time host setup (operator)

1. Create a root-owned campaign wrapper that runs the launcher, harness, `cyclictest`, `rtla`,
   `hwlatdetect`, `perf`, and `stress-ng` with the required privileges.
2. Add a narrow `/etc/sudoers.d/stc-wl4` entry for that wrapper, for example
   `r2k ALL=(root) NOPASSWD: /usr/local/sbin/stc-wl4-run` (validate with `visudo -c`). Do not add
   broad NOPASSWD rules.
3. Record the current boot command line, governor, C-state configuration, IRQ affinities, swap,
   and service list. Do not change them for v2.0.0.
4. Confirm that no reboot or service change is required for the untuned campaign.

**Installed wrapper (2026-10-29).** The source is [`tools/rt/stc-wl4-run`](tools/rt/stc-wl4-run),
installed root-owned at `/usr/local/sbin/stc-wl4-run` (mode 0755). The sudoers source is
[`tools/rt/stc-wl4.sudoers`](tools/rt/stc-wl4.sudoers), installed at `/etc/sudoers.d/stc-wl4`
(mode 0440) and validated by `visudo -cf`. The wrapper accepts only `cyclictest`, `timerlat`,
`osnoise`, `hwlat`, `perf`, and `campaign`; every argument is validated, and `campaign` execs
only the root-owned `/home/r2k/stc-rt/tools/rt/campaign.py`. Test evidence: `cyclictest 2`
completed with max latencies of 10-14 µs per CPU; `timerlat 2s` completed with all-CPU maxima
of 6 µs IRQ, 22 µs thread, and 30 µs user; `hwlat 2s` reported no samples over threshold; an
invalid duration was rejected with exit 64. No boot, governor, service, or swap change was made.

### 9.2 Build and transfer (Windows development host)

The `rt-build` workflow on `ubuntu-24.04` builds the Linux package and the `rt-bench` bundle and
uploads `stc-package-ubuntu-24.04` and `rt-bench-bundle`. While `WL-4` is not merged to the
default branch, a push to `WL-4` triggers it; `workflow_dispatch` becomes available once the
workflow is on the default branch.
Set `RT_HOST` from the local `.env/RT_Host.md` note; it is not stored in this repository.

The package artifact downloads under a `dist/` prefix, so stage the bundle in its own directory
and copy the package directory directly:

```sh
gh run download <run-id> -n stc-package-ubuntu-24.04 -D dist/wl4
gh run download <run-id> -n rt-bench-bundle -D dist/wl4/rt-bench
ssh "$RT_HOST" 'mkdir -p ~/stc-rt'
scp -r dist/wl4/dist/stc-* "$RT_HOST":~/stc-rt/
scp -r dist/wl4/rt-bench "$RT_HOST":~/stc-rt/
ssh "$RT_HOST" 'chmod +x ~/stc-rt/stc-*/bin/stc ~/stc-rt/rt-bench/rt-bench ~/stc-rt/rt-bench/workloads/*/rt-bench'
ssh "$RT_HOST" 'cd ~/stc-rt/stc-* && sha256sum -c SHA256SUMS'
ssh "$RT_HOST" 'cd ~/stc-rt/rt-bench && sha256sum -c SHA256SUMS'
```

The bundle contains the base launcher and module, the light and heavy workload launchers, and all
four online-change candidates. Artifact downloads drop the executable bit, so the `chmod` is part
of the transfer. Transfer the whole staging directory; do not rebuild on the host.

### 9.3 Verify the runtime layout

Replace `<version>` with the pinned package version.

```sh
ssh "$RT_HOST" '~/stc-rt/stc-<version>-x86_64-unknown-linux-gnu/bin/stc --version'
ssh "$RT_HOST" 'cd ~/stc-rt/rt-bench && ldd rt-bench | head'
ssh "$RT_HOST" 'cd ~/stc-rt/rt-bench && LD_LIBRARY_PATH="$PWD/runtime${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" ./rt-bench --cycles 3 --cycle-time 1ms --status'
```

The light and heavy workload launchers use the same layout:

```sh
ssh "$RT_HOST" 'cd ~/stc-rt/rt-bench/workloads/light && ./rt-bench --cycles 2 --cycle-time 1ms --status'
ssh "$RT_HOST" 'cd ~/stc-rt/rt-bench/workloads/heavy && ./rt-bench --cycles 2 --cycle-time 1ms --status'
```

If `ldd` reports `libstc_runtime.so` as missing, use the bundle `runtime/` directory as shown;
otherwise the launcher is statically linked. Record which form was used.

**Phase 1 artifact evidence (2026-10-29).** `rt-build` run
[40941245512](https://github.com/r2k-in-the-vortex/stc/actions/runs/40941245512) on commit
`19c5bb0` uploaded `stc-package-ubuntu-24.04` (11,070,839 bytes) and `rt-bench-bundle`
(7,090,218 bytes). Local validation with the Windows driver: `code-only` and `trapping` are
compatible; `field-add-remove` is compatible with the `SPARE` quarantine warning; `incompatible`
is cold (`online diff` exits 1); the trapping candidate traps on its first scan. Host
verification: both `SHA256SUMS` files pass; `bin/stc --version` reports `stc 1.0.0`; the base
launcher is statically linked; base/light/heavy ran 3/2/2 cycles with exit 0. Build IDs: base
`0xe4cfdfe067d81174ca8292d2f1333fd4`, light `0x39230aa2b8c40ff6cc7e922216e557ec`, heavy
`0x00496294897a5fd6f5e8b9e1d3ac5524`; candidate modules `f9208c110d714ba394d533bd9202f841`
(code-only), `fbf4c879687fe2cbcbaee5883e46cc7a` (field), `c18805e99dabef777f18329430878681`
(incompatible), and `45673bcf3ae588a7fe9b9789eef7231d` (trapping). Artifact downloads dropped the
executable bit, so the runbook chmods the driver and launchers.

**Phase 3 harness evidence (2026-10-29).** `tools/rt/campaign.py` is installed root-owned at
`/home/r2k/stc-rt/tools/rt/campaign.py`; the bundle and package were refreshed from `rt-build` run
[40952490246](https://github.com/r2k-in-the-vortex/stc/actions/runs/40952490246) on `7ad6a60`,
because the phase 1 bundle predates the phase 2 `--rt` flags. The harness unit tests (27) pass
with `python3 -m unittest discover -s tools/rt/tests -v` and run in `rt-build`.

Dry runs (raw data under `~/stc-rt/results/phase3-*`, not committed):

- R0, base workload, 10 s at 1 ms: 2,503 measured cycles, execution p50 3.073 ms, 2,503 missed
  deadlines, 7,509 skipped periods. The base workload overruns a 1 ms cycle.
- R0, light workload, 10 s at 1 ms: 9,556 measured cycles, execution p50 0.950 ms, 9,556 missed
  deadlines (the schema counts any positive wake lateness), 448 skipped periods. The light
  workload is at the 1 ms budget, so phase 4 must calibrate the workload constants.
- Online, 10 ms cycle: `code-only` and `field-add-remove` activated (`0xf9208c11…`,
  `0xfbf4c879…`); `incompatible` returned `cold_required` and kept the base active; `trapping`
  activated at cycle 205 and rolled back at cycle 206, observed after about 10.6 ms. All four
  cases passed, and the `stc-rt-summary/1` keys were identical across reruns.

No boot, GRUB, governor, C-state, IRQ-affinity, service, or swap change was made; the phase 0
wrapper and sudoers rule were untouched. The only host setup change was the root-owned harness
and the refreshed bundle/package.

**Phase 4 campaign evidence (2026-10-30).** The untuned campaign ran from 2026-10-29T21:34Z to
2026-10-30T01:12Z through the phase 0 wrapper. The benchmark workloads were calibrated to
base/candidates `WORK=10000`, light `WORK=2000`, and heavy `WORK=18000`; 10 s checks measured
execution p50 0.497 ms, 0.248 ms, and 0.797 ms at a 1 ms cycle. Artifacts: `rt-build` run
[40958283937](https://github.com/r2k-in-the-vortex/stc/actions/runs/40958283937) on `2be4325`;
bundle and package `SHA256SUMS` SHA256 `6107e867…` and `415cca13…`; harness
`02c3f6b8…` installed root-owned. The committed run manifest is
[RT-Run-Manifest.md](specification/Verification/RT-Run-Manifest.md); raw samples are local and
gitignored at `dist/rt/results/phase4` (420 files, `SHA256SUMS` SHA256 `23b7f059…`).

- Main medium 1 ms: R0 execution p50 0.663–0.782 ms (p99 about 0.93 ms), 598–600k cycles; R1
  other-CPU mixed load p50 2.58–2.68 ms, 178–189k cycles and 411–422k skipped periods; R2
  same-CPU load p50 0.649–0.659 ms, 599k cycles.
- 10 ms secondary: R0/R1/R2 execution p50 1.242/2.540/0.756 ms; 60k cycles; no skipped periods.
- Supplementary R0: light p50 0.249 ms; heavy p50 0.784 ms.
- Control/status A/B (R0, medium, 1 ms): execution p50 508 µs (control disabled), 624 µs
  (control idle), 748 µs (status at 100 Hz).
- Online change under load: all 24 case runs passed; code-only and field-add-remove activated;
  incompatible returned `cold_required` with the base active; trapping activated and rolled back
  on the next cycle in every R1/R2 repetition.
- R0 noise floor: cyclictest max 27–40 µs; timerlat max 69 µs IRQ / 84 µs thread / 93 µs user;
  osnoise max 8.5–22.8 ms; hwlat max 13 µs with 12 samples above the 10 µs threshold (the tool's
  exit 1 is threshold-exceeded, not a missing run).
- The osnoise maxima meet the parent plan's "unsatisfactory untuned jitter" trigger; phase 5
  evaluates the deferred tuned profile (§6.2). No boot, governor, C-state, IRQ, service, or swap
  change was made; no reboot.

### 9.4 Record the untuned host state

Do not change the boot profile, governor, C-state limit, IRQ affinities, services, or swap.

```sh
cat /proc/cmdline
cat /sys/devices/system/cpu/isolated
cat /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor
for d in /sys/devices/system/cpu/cpu0/cpuidle/state*; do cat "$d/name" "$d/latency"; done
grep -E 'enp3s0' /proc/interrupts
swapon --show
systemctl list-units --type=service --state=running --no-legend | wc -l
```

Record the values in every run header. The deferred tuned profile (§6.2) is not applied.

### 9.5 Run the campaign

```sh
sudo /usr/local/sbin/stc-wl4-run campaign --regime r0 --duration 10m --cycle-time 1ms \
  --workload light --out ~/stc-rt/results/2026-10-29-r0
sudo /usr/local/sbin/stc-wl4-run campaign --regime r1 --load-profile mixed --duration 10m \
  --cycle-time 1ms --workload light --out ~/stc-rt/results/2026-10-29-r1
sudo /usr/local/sbin/stc-wl4-run campaign --mode online --online-case all --duration 10s \
  --cycle-time 10ms --out ~/stc-rt/results/2026-10-29-online
```

`--workload` selects `base`, `light`, or `heavy`; phase 4 must calibrate the workload constants
before the 1 ms primary runs. Repeat for R0, R1, R2 and for the 10 ms secondary runs. R1 starts
the recorded load on CPUs other than the cycle thread's CPU; R2 pins the load to that CPU. Run
the online-change cases with a fresh generation per repetition.

The harness starts the bundled launcher with `--rt --rt-priority 90 --rt-cpu <cycle-cpu>
--rt-metrics <run>/cycles.jsonl`; §6.3.1 fixes the schema. It must not use `stc run --watch` on
the host.

### 9.6 Collect and restore

The harness already writes `cycles.jsonl.gz` and `proc.jsonl.gz`; do not gzip them again.

```sh
find ~/stc-rt/results -type f -print0 | sort -z | xargs -0 sha256sum \
  > ~/stc-rt/results/SHA256SUMS
scp -r "$RT_HOST":~/stc-rt/results dist/rt/results
```

Remove the campaign's temporary files and control sockets. No boot, governor, service, or swap
restore is required because the untuned campaign does not change them. If a future tuned campaign
is run, restore the original boot profile and services and record the restoration.

## 10. Risks and resolved questions

Risks:

- Privilege setup may be blocked or awkward under `sudo-rs`; a systemd unit is the fallback.
- Four cores are shared with the harness, services, and load in the untuned campaign; there is
  no isolation benefit to rely on.
- The preliminary osnoise maxima reach 3 ms; firmware SMI or deep C-states may make the untuned
  results unsatisfactory. The deferred tuned profile is the agreed follow-up if the trigger is
  met, not a v2.0.0 deliverable.
- R-3 touches `run_loop` and the launcher; it must be opt-in, reviewed, and leave normal
  artifacts and default behavior untouched.
- Deploy prepare runs on the cycle thread in v1; it is a real jitter source. Fully off-thread
  prepare is post-v1 and out of scope.
- CI is the only current Linux build path; WSL is not usable as configured.
- Timing results are not portable and must not become CI thresholds.

### 10.1 Resolved questions (2026-10-29)

1. R-3 is approved as an opt-in runtime measurement mode. External tracing alone is not the
   chosen path.
2. The tuned boot profile is deferred future work and out of scope for v2.0.0. v2.0.0 measures
   the untuned host. The profile is recorded in §6.2 and is revisited only if an untuned run
   misses a deadline or the report review judges jitter or latency unsatisfactory.
3. The campaign uses a scoped NOPASSWD wrapper, not per-session interactive sudo.
4. The report and summary are committed under `specification/Verification/`; raw samples are
   retained as campaign artifacts with SHA256.
5. The campaign is three repetitions per regime at 1 ms, plus one 10 ms secondary run per
   regime; each run is 10 minutes.
6. Load cases are quiet, other-CPU load, and same-CPU load, each labelled separately.
7. All four online-change cases are required: code-only, field add/remove, incompatible, and
   trapping rollback.

### 10.2 Remaining implementation defaults

R-1 (CI artifact path), R-6 (Python standard-library harness), and the `tests/rt/` benchmark
location are implementation defaults. They follow from the host and tooling evidence and may be
adjusted during phase 0 without a scope change.

## 11. Change log

| Date | Change |
| --- | --- |
| 2026-10-29 | Started the WL-4 sub-plan. Verified SSH key access, captured the host and kernel baseline, ran a preliminary untuned cyclictest/timerlat/osnoise sample, and smoke-tested the released Linux package on the host. Proposed decisions R-1–R-10. |
| 2026-10-29 | Locked decisions: R-3 opt-in measurement mode, scoped sudo wrapper, spec report with raw artifacts, 10 min × 3 campaign, quiet/other-CPU/same-CPU load, and all four online-change cases. Deferred the tuned boot profile to future work; v2.0.0 measures the untuned host. |
| 2026-10-30 | Completed the WL-4 phase 4 untuned campaign: 23 harness runs, 24 online cases, the R0 noise floor, and the control A/B. Committed the run manifest [RT-Run-Manifest.md](specification/Verification/RT-Run-Manifest.md); raw data remains local and gitignored. |
