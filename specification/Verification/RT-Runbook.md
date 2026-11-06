# PREEMPT_RT verification runbook — WL-4

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-30  
Status: Operational runbook for the untuned v2.0.0 protocol. Results are host- and
configuration-specific; this is not a hard real-time guarantee and not a safety function.  
Parent: [V2.0.0-PLAN.md](../../V2.0.0-PLAN.md) §4 (WL-4)  
Report: [RT-Host.md](RT-Host.md)  
Evidence: [RT-Run-Manifest.md](RT-Run-Manifest.md)

This runbook repeats the WL-4 verification on the reference host or on another supported Linux
PREEMPT_RT host. The target host runs artifacts only; every build happens in CI or on a separate
Linux build host.

## 1. Protocol and scope

The v2.0.0 protocol is an **untuned** campaign:

- Leave the boot command line, governor, C-states, IRQ affinity, services, and swap as they are;
  record them for every run.
- Run the prebuilt launcher with the opt-in RT mode: `--rt`, a 1 ms or 10 ms cycle, FIFO
  priority 90 on a chosen CPU, and `--rt-metrics` pointing at a per-run JSONL file.
- Measure R0 (quiet), R1 (load on other CPUs), and R2 (load on the cycle CPU), the 10 ms secondary,
  the light/medium/heavy workload levels, online change under load, the control/status cost A/B,
  and the system noise floor.
- Treat the results as host-specific evidence. They do not establish a timing budget, a hard
  real-time guarantee, or a safety claim.

The deferred tuned boot profile is future work, not part of v2.0.0; its trigger and parameters
are in [RT-Host.md §6](RT-Host.md). Adopting it later is a separate scope decision and requires a
reboot and maintenance window.

## 2. Target host requirements

| Item | Requirement |
| --- | --- |
| Architecture | Linux x86-64 GNU or Linux AArch64 GNU, matching the prebuilt artifacts |
| Kernel | PREEMPT_RT; record `uname -r`, the kernel config, and `/sys/kernel/debug/sched/preempt` |
| Runtime | glibc at least as new as the build host's; the reference artifacts are built on Ubuntu 24.04 |
| Access | The login user can authenticate with sudo for the one-time wrapper install; no stored credential |
| Tools | `python3` 3.11+ (standard library only), `stress-ng`, `taskset`, `cyclictest`, `rtla`, `hwlatdetect`, `perf` |
| Build rule | The target never builds `stc`, the bundle, or the harness; use CI or a separate Linux build host |
| Capacity | Enough free disk for the result archive and enough CPUs for the cycle thread plus load |

The reference host baseline is in §6. A different host will differ in CPU, kernel configuration,
power management, services, and firmware; record every difference.

## 3. Build and transfer

### 3.1 Build in CI

The `rt-build` workflow builds the Linux package with `cargo xtask package` and the bundle with
`tools/rt/build-bundle.sh`, then uploads two artifacts:

- `stc-package-ubuntu-24.04`
- `rt-bench-bundle`

The current workflow targets `ubuntu-24.04` x86-64. For an AArch64 target, add an
`ubuntu-24.04-arm` matrix entry with distinct artifact names (for example
`stc-package-ubuntu-24.04-arm` and `rt-bench-bundle-arm`), or run the same two commands in a
matching Linux container or build host:

```sh
cargo xtask package --locked --out dist
bash tools/rt/build-bundle.sh dist/stc-<version>-<host> dist/rt-bench
```

Do not mix an x86-64 package or bundle with an AArch64 target. Build against the oldest glibc you
need to support; a binary built on the older glibc runs on a newer one.

### 3.2 Bundle layout

```text
stc-<version>-<host>/            the packaged compiler and runtime library
  bin/stc
  lib/
rt-bench/
  rt-bench                       base launcher (medium workload)
  rt-bench-<build-id>.so         base module
  rt-bench.stc-got.json|bin      base manifest
  rt-bench.stc-meta.json
  workloads/light/               cold-start light launcher
  workloads/heavy/               cold-start heavy launcher
  candidates/code-only/          compatible candidates for the control channel
  candidates/field-add-remove/
  candidates/incompatible/
  candidates/trapping/
  runtime/libstc_runtime.so      present when the launcher needs a shared runtime
  SHA256SUMS
```

The base and all candidates in the reference bundle use the same workload constant
(`WORK=10000`); light and heavy use 2000 and 18000. The workload constants are calibrated for
the reference host; another host must label or recalibrate its workload levels.

### 3.3 Transfer and verify

Set `RT_HOST` from your local access note. The checked-in `.env/RT_Host.md` is local-only and is
not part of the repository.

```sh
gh run download <rt-build-run-id> -n stc-package-ubuntu-24.04 -D dist/rt
gh run download <rt-build-run-id> -n rt-bench-bundle -D dist/rt
scp -r dist/rt "$RT_HOST":~/stc-rt/
ssh "$RT_HOST" 'cd ~/stc-rt && (cd stc-<version>-<host> && sha256sum -c SHA256SUMS) && (cd rt-bench && sha256sum -c SHA256SUMS)'
```

Artifact downloads drop the executable bit. Restore it where needed:

```sh
chmod +x ~/stc-rt/stc-<version>-<host>/bin/stc ~/stc-rt/rt-bench/rt-bench \
  ~/stc-rt/rt-bench/workloads/light/rt-bench ~/stc-rt/rt-bench/workloads/heavy/rt-bench
```

Verify the package and the launcher before measuring:

```sh
~/stc-rt/stc-<version>-<host>/bin/stc --version
ldd ~/stc-rt/rt-bench/rt-bench
```

If `ldd` reports `libstc_runtime.so` missing, run with
`LD_LIBRARY_PATH="$PWD/runtime:${LD_LIBRARY_PATH:-}"` or use the static launcher form.

## 4. Privileged setup

### 4.1 Wrapper and sudoers

The campaign needs FIFO scheduling, `mlockall`, perf, and tracefs. Install the checked-in wrapper
and a narrow sudoers rule once, as root:

```sh
sudo install -o root -g root -m 0755 tools/rt/stc-wl4-run /usr/local/sbin/stc-wl4-run
sudo install -o root -g root -m 0440 tools/rt/stc-wl4.sudoers /etc/sudoers.d/stc-wl4
sudo visudo -cf /etc/sudoers.d/stc-wl4
```

The sudoers rule must name the host login user:

```text
<login-user> ALL=(root) NOPASSWD: /usr/local/sbin/stc-wl4-run
```

The wrapper accepts only fixed subcommands: `cyclictest`, `timerlat`, `osnoise`, `hwlat`, `perf`,
and `campaign`; every argument is validated. Do not add a generic command passthrough or a broad
NOPASSWD rule.

### 4.2 Harness

Install the single-file harness root-owned at the path the wrapper expects:

```sh
sudo install -d -o root -g root -m 0755 /home/<login-user>/stc-rt/tools/rt
sudo install -o root -g root -m 0755 tools/rt/campaign.py \
  /home/<login-user>/stc-rt/tools/rt/campaign.py
```

The wrapper checks that the harness is owned by root and is not group- or other-writable. The
harness uses only the Python standard library; it does not need pip.

### 4.3 Adapting to another user or host

The checked-in wrapper and sudoers are configured for the reference user `r2k` and the path
`/home/r2k/stc-rt/tools/rt/campaign.py`. On another host:

1. Replace `r2k` in `tools/rt/stc-wl4.sudoers` with the target login user.
2. Replace the wrapper's `CAMPAIGN` constant with that user's harness path, or install the
   harness at `/home/r2k/stc-rt/tools/rt/campaign.py` and use `r2k` as the login user.
3. Install both files root-owned, validate with `visudo -cf`, and confirm the harness is
   root-owned and not group/other-writable.
4. Pass the matching bundle path to the harness with `--bundle` if it is not
   `/home/r2k/stc-rt/rt-bench`.

Verify the setup before a campaign:

```sh
sudo -n /usr/local/sbin/stc-wl4-run cyclictest 2
sudo -n /usr/local/sbin/stc-wl4-run campaign --help
```

If `sudo -n` fails, the sudoers rule or its mode is wrong. Do not weaken the rule to debug it.

## 5. Host baseline and regime choice

Before the first run, record the target's state. This is also what the harness writes to
`host.json` for every run.

```sh
uname -a
cat /etc/os-release
cat /proc/cmdline
cat /sys/kernel/debug/sched/preempt
grep -E 'CONFIG_PREEMPT_RT|CONFIG_PREEMPTION|CONFIG_HZ|CONFIG_NO_HZ|CONFIG_RCU_NOCB|CONFIG_IRQ_FORCED_THREADING' /boot/config-$(uname -r)
lscpu
free -h
swapon --show
for c in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do echo "$c $(cat "$c")"; done
for d in /sys/devices/system/cpu/cpu0/cpuidle/state*; do echo "$d $(cat "$d/name") $(cat "$d/latency")"; done
cat /sys/devices/system/cpu/isolated
cat /sys/devices/system/cpu/nohz_full
grep -E 'enp|eth|wlp' /proc/interrupts
systemctl list-units --type=service --state=running --no-legend | wc -l
df -h /
```

For the untuned v2.0.0 protocol, do not change any of these values. If the host is already tuned by
its owner, record that regime and do not compare it with the untuned reference report. The deferred
tuned profile is a future-work comparison, not part of this protocol.

## 6. Reference host baseline

This is the host measured in [RT-Host.md](RT-Host.md). It is a reference point, not a requirement.

| Item | Value |
| --- | --- |
| Host | TianBei N1 PRO desktop; firmware `HCN150_MI3_0.10` (2025-04-18); bare metal |
| OS | Ubuntu 25.10 (Questing Quokka), x86-64 |
| Kernel | `6.17.999-8-generic #8 SMP PREEMPT_RT`, built 2026-01-26; Ubuntu 6.17.0-8.8 source plus the RT patch |
| Preemption | `(full) lazy`; `CONFIG_PREEMPT_RT=y`, `CONFIG_PREEMPTION=y`, `CONFIG_PREEMPT_DYNAMIC=y` |
| Time/RCU | `CONFIG_HZ_1000=y`, `CONFIG_NO_HZ=y`, `CONFIG_NO_HZ_FULL=y`, `CONFIG_RCU_NOCB_CPU=y`, `CONFIG_HIGH_RES_TIMERS=y` |
| Interrupts | `CONFIG_IRQ_FORCED_THREADING=y`; `sched_rt_runtime_us=950000`, `sched_rt_period_us=1000000`; `CONFIG_RT_GROUP_SCHED` not set |
| Clocksource | `tsc`; `constant_tsc`, `nonstop_tsc`, `tsc_known_freq` |
| Tracers | `timerlat`, `osnoise`, `hwlat`, `blk`, `mmiotrace`, `function_graph`, `wakeup_dl`, `wakeup_rt`, `wakeup`, `function`, `nop` |
| Boot command line | No `isolcpus`, `nohz_full`, `rcu_nocbs`, `irqaffinity`, or `threadirqs` parameters |
| CPU | Intel N150, 4 cores / 4 threads, no SMT, 1 NUMA node; L1d 128 KiB, L1i 256 KiB, L2 2 MiB, L3 6 MiB |
| Frequency | 700 MHz to 3.60 GHz; `intel_pstate`; `powersave`; EPP `balance_performance`; turbo enabled |
| Idle states | POLL 0 µs, C1E 2 µs, C6 195 µs, C8 260 µs, C10 660 µs |
| Isolation | None; NIC IRQs 141–145 had affinity 0–3 |
| Memory/storage | 10 GiB RAM, 4 GiB swap file, no hugepages; 336 GB root filesystem |
| Services | 27 running, including Docker, Samba, chrony, ModemManager, multipathd, networkd, resolved, ssh, rsyslog |
| Tooling | cyclictest 2.60, rtla, hwlatdetect, stress-ng, perf 6.17.2, trace-cmd, taskset, chrt, numactl, cpupower, turbostat, python3 3.13.7, jq |
| Build state | glibc 2.42; systemd 257; cgroup v2; rustc 1.85.1 without cargo; not a build host |
| Original privileges | Login user was a sudoer with interactive authentication; `ulimit -r` 0; `perf_event_paranoid` 4; tracefs root-only; memlock 8 MiB |

The wrapper in §4 is what makes FIFO, `mlockall`, perf, and tracefs available without a broad
privilege change.

## 7. Validation and campaign

### 7.1 Short validation

Run these before the full campaign and check that the metrics and summary files exist and use
`stc-rt-metrics/1` and `stc-rt-summary/1`:

```sh
sudo -n /usr/local/sbin/stc-wl4-run cyclictest 2
sudo -n /usr/local/sbin/stc-wl4-run timerlat 2s
sudo -n /usr/local/sbin/stc-wl4-run campaign --regime r0 --duration 10s --cycle-time 1ms \
  --workload base --load-profile none --rt-cpu <cpu> --rt-priority 90 \
  --out /home/<login-user>/stc-rt/results/validate-r0
sudo -n /usr/local/sbin/stc-wl4-run campaign --mode online --online-case all --duration 5s \
  --cycle-time 1ms --regime r1 --load-profile mixed --rt-cpu <cpu> --rt-priority 90 \
  --out /home/<login-user>/stc-rt/results/validate-online
```

The online validation must show compatible activation, `cold_required` for the incompatible
candidate, and activation followed by rollback for the trapping candidate. If rollback is not
observed, stop and investigate before the campaign.

### 7.2 Campaign matrix

| Work | Runs | Cycle | Workload | Duration |
| --- | --- | --- | --- | --- |
| Main steady R0/R1/R2 | 3 repetitions each | 1 ms | medium | 10 min each |
| Secondary R0/R1/R2 | 1 each | 10 ms | medium | 10 min each |
| Light/heavy supplementary | 1 each, R0 | 1 ms | light, heavy | 10 min each |
| Online change R1/R2 | 3 repetitions, four cases each | 1 ms | medium | 5 s each, 2 s warm-up |
| Control cost R0 | 3 legs | 1 ms | medium | 10 min each |
| Noise floor R0 | cyclictest, timerlat, osnoise, hwlat | system | system | 600 s / 10 min each |

Every steady run has a 2 s warm-up; the summary excludes cycles before the measurement boundary.

### 7.3 Commands

```sh
# Main steady. R0: --regime r0 --load-profile none.
# R1: --regime r1 --load-profile mixed (load on the other CPUs).
# R2: --regime r2 --load-profile mixed (load on the cycle CPU).
sudo -n /usr/local/sbin/stc-wl4-run campaign --regime r0 --duration 10m --cycle-time 1ms \
  --workload base --load-profile none --rt-cpu <cpu> --rt-priority 90 \
  --out /home/<login-user>/stc-rt/results/<run>

# Secondary 10 ms.
sudo -n /usr/local/sbin/stc-wl4-run campaign --regime r0 --duration 10m --cycle-time 10ms \
  --workload base --load-profile none --rt-cpu <cpu> --rt-priority 90 \
  --out /home/<login-user>/stc-rt/results/<run>

# Light or heavy.
sudo -n /usr/local/sbin/stc-wl4-run campaign --regime r0 --duration 10m --cycle-time 1ms \
  --workload light --load-profile none --rt-cpu <cpu> --rt-priority 90 \
  --out /home/<login-user>/stc-rt/results/<run>

# Online change under load; all four cases in one invocation.
sudo -n /usr/local/sbin/stc-wl4-run campaign --mode online --online-case all \
  --regime r1 --load-profile mixed --duration 5s --cycle-time 1ms \
  --rt-cpu <cpu> --rt-priority 90 --out /home/<login-user>/stc-rt/results/<run>

# Control cost: --control-mode none, then socket with --status-hz 0, then 100.
sudo -n /usr/local/sbin/stc-wl4-run campaign --regime r0 --duration 10m --cycle-time 1ms \
  --workload base --load-profile none --control-mode none \
  --out /home/<login-user>/stc-rt/results/<run>

# Noise floor.
sudo -n /usr/local/sbin/stc-wl4-run cyclictest 600
sudo -n /usr/local/sbin/stc-wl4-run timerlat 10m
sudo -n /usr/local/sbin/stc-wl4-run osnoise 10m
sudo -n /usr/local/sbin/stc-wl4-run hwlat 10m
```

`<cpu>` is the cycle thread's CPU; the reference campaign used CPU 3. R1 load is placed on the
other CPUs and R2 load on the same CPU; the harness records the exact `stress-ng` command, affinity,
and PIDs in `load.log`.

## 8. Running on other hosts

### 8.1 Architecture and artifacts

- Use a package, bundle, and harness built for the target architecture. The reference `rt-build`
  workflow is x86-64; an AArch64 target needs the arm matrix entry or a matching Linux build host.
- Do not copy an x86-64 bundle to an AArch64 host, or the reverse. Do not copy a build from a
  newer glibc to a host with an older glibc.
- The target is a run host. Build `stc`, the bundle, and the harness elsewhere; install only the
  artifacts and the root-owned harness on the target.

### 8.2 User and paths

- Replace the reference user `r2k` in the sudoers file and the wrapper's `CAMPAIGN` path as
  described in §4.3.
- Use `--bundle` if the bundle is not at `/home/r2k/stc-rt/rt-bench`.
- Keep the harness root-owned and not group/other-writable; the wrapper checks this.
- Set `RT_HOST` from that host's own access note; do not reuse the reference `.env/RT_Host.md`.

### 8.3 Host differences and comparability

- Record the kernel, PREEMPT_RT mode, firmware, CPU, governor, idle states, IRQ affinity, services,
  swap, and build IDs for every run.
- The untuned protocol is per host. A host with a different governor, C-state limit, isolation,
  service set, or kernel configuration is a different regime; label it and do not compare it with
  the reference report as if it were the same host.
- The workload constants are calibrated for the reference host. On another host, label the
  measured execution times and recalibrate the light/medium/heavy constants if the plan's
  qualitative levels are required.
- The reference host used CPU 3 for the cycle thread; choose and record the target's cycle CPU.
  The R1/R2 load placement follows that choice.
- Results are not portable across hosts, kernels, firmware, or power profiles. Do not turn them
  into a cross-host threshold or a CI gate.

### 8.4 Minimal validation

Run the §7.1 short validation on the new host first. Confirm the metrics schema, the control
protocol, FIFO scheduling, the online activation/rollback cases, and the host snapshot. Only then
run the full §7.2 matrix.

## 9. Collect and restore

```sh
gzip -9 ~/stc-rt/results/*/*.jsonl
sha256sum ~/stc-rt/results/*/* > ~/stc-rt/results/SHA256SUMS
scp -r "$RT_HOST":~/stc-rt/results dist/rt/results
```

Stop all load and launcher processes and verify none remain:

```sh
pgrep -af 'stress-ng|rt-bench'
```

The untuned protocol makes no host changes, so no boot, governor, service, or swap restore is
required. If a tuned campaign or any temporary host change was made, restore the original boot
profile and services and record the restoration. Keep the raw archive outside the repository; the
committed evidence is [RT-Run-Manifest.md](RT-Run-Manifest.md).

## 10. References

- [RT-Host.md](RT-Host.md) — the report, method, results, limits, and deferred tuned profile.
- [RT-Run-Manifest.md](RT-Run-Manifest.md) — per-run tables, host snapshots, and raw-file SHA256 values.
- [Performance.md](Performance.md) — the performance record, including the RT summary.
- [KNOWN-LIMITATIONS.md](../../KNOWN-LIMITATIONS.md) — the documented real-time limits.
- [NON-SAFETY.md](../../NON-SAFETY.md) — the safety stance.
- [Runtime and Execution](../Architecture/Runtime-and-Execution.md) — the cycle loop, thread classes, and host ABI.
- [Online Change and GOT](../Architecture/Online-Change-and-GOT.md) — prepare/activate/rollback.
- [V2.0.0-PLAN.md](../../V2.0.0-PLAN.md) — the v2.0.0 target set and the WL-4 completion status.
