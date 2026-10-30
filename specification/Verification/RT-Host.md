# PREEMPT_RT host verification — WL-4

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-30  
Status: Informational and host-specific. Not a hard real-time guarantee, not a safety function,
and not a CI gate.  
Parent: [WL-4-Plan.md](../../WL-4-Plan.md)  
Evidence: [RT-Run-Manifest.md](RT-Run-Manifest.md)

This report records the untuned v2.0.0 PREEMPT_RT campaign on the available x86-64 host. It is
the evidence for WL-4. It does not establish a hard real-time guarantee, does not define a
timing budget, and does not change the safety stance in [NON-SAFETY.md](../../NON-SAFETY.md).

## 1. Summary

- On the untuned host, the 1 ms medium workload runs with execution p50 0.66–0.78 ms and p99
  0.93–0.95 ms in the quiet regime (R0). Nearly all 1 ms budget is consumed by the workload.
- A heavy mixed load on the other CPUs (R1) raises execution p50 to 2.58–2.68 ms; the host does
  not sustain a 1 ms cycle in that stress case. The same load on the cycle CPU (R2) leaves
  execution p50 at 0.65–0.66 ms because the FIFO:90 cycle thread preempts the lower-class
  stress workers.
- The 10 ms secondary runs meet the period in all three regimes, with no skipped periods.
- The control/status thread adds about 116 µs to execution p50 when idle and about 240 µs with
  100 Hz status polling, compared with a control-disabled run.
- All 24 online-change cases passed under load. Compatible changes activated; the incompatible
  candidate returned `cold_required` with the base generation active; the trapping candidate
  activated and rolled back on the next cycle in every repetition.
- The untuned noise floor measured cyclictest maxima of 27–40 µs, timerlat maxima of 69 µs IRQ /
  84 µs thread / 93 µs user, and osnoise maxima of 8.5–22.8 ms. hwlat recorded a 13 µs maximum.
- The untuned R1 stress result and the osnoise maxima meet the deferred tuned-profile trigger in
  [WL-4-Plan.md](../../WL-4-Plan.md) §6.2. The tuned profile remains future work and was not
  executed in v2.0.0.

## 2. Environment

| Item | Value |
| --- | --- |
| Host | TianBei N1 PRO desktop, Intel N150 (4 cores / 4 threads, no SMT, 1 NUMA node), 10 GiB RAM |
| OS | Ubuntu 25.10 (Questing Quokka), x86-64, bare metal |
| Kernel | `Linux 6.17.999-8-generic #8 SMP PREEMPT_RT`, built 2026-01-26 |
| Preemption | PREEMPT_RT full/lazy (`/sys/kernel/debug/sched/preempt` reports `(full) lazy`) |
| Power | `powersave` governor on all CPUs; `intel_pstate` active; deep idle states C1E/C6/C8/C10 |
| Isolation | None: no `isolcpus`, `nohz_full`, or `rcu_nocbs`; IRQs can land on any CPU |
| Storage | 336 GB root, 4 GiB swap file |
| Background | Normal desktop services, including Docker, Samba, chrony, ModemManager, and the desktop session |
| Campaign window | 2026-10-29 21:34:41 UTC to 2026-10-30 01:12:41 UTC |
| Host changes | None: no boot parameter, governor, C-state, IRQ-affinity, service, or swap change; no reboot |

The campaign was untuned by design. The host snapshot at the start of the noise floor showed a
1-minute load average of 2.57, immediately after the calibration checks; the main campaign ran
after the noise floor with load averages around 1.3–1.9. The background services and the desktop
session are part of the R0 regime.

The full host baseline, including the kernel configuration and the original boot command line,
is in [WL-4-Plan.md](../../WL-4-Plan.md) §3. The per-run host snapshots are in
[RT-Run-Manifest.md](RT-Run-Manifest.md) and the raw archive.

## 3. Method

### 3.1 Regimes and load

| Regime | Load | Intended use |
| --- | --- | --- |
| R0 | None | Quiet untuned baseline |
| R1 | `mixed` load on CPUs 0–2 | Heavy external-load stress case, not a light background load |
| R2 | `mixed` load pinned to the cycle CPU (3) | Same-CPU lower-class load; the cycle thread remains FIFO:90 |

The `mixed` profile uses stress-ng CPU, VM, HDD, and socket stressors. The exact command,
affinity, and PIDs are recorded per run in `load.log` in the raw archive. The cycle thread runs
at SCHED_FIFO priority 90 on CPU 3 in every harness run. The host is untuned: there is no CPU
isolation, and the service/control threads are explicitly returned to SCHED_OTHER with nice +10
when the cycle thread is FIFO.

### 3.2 Campaign matrix

| Work | Runs | Cycle | Workload | Duration |
| --- | --- | --- | --- | --- |
| Calibration | 7 short checks | 1 ms | base/light/heavy | 10 s |
| Noise floor, R0 | cyclictest, timerlat, osnoise, hwlat | system | system | 600 s / 10 min each |
| Main steady, R0 | 3 repetitions | 1 ms | medium (base) | 10 min each |
| Main steady, R1 | 3 repetitions | 1 ms | medium (base) | 10 min each |
| Main steady, R2 | 3 repetitions | 1 ms | medium (base) | 10 min each |
| Secondary | 1 per regime | 10 ms | medium (base) | 10 min each |
| Supplementary | 1 per workload, R0 | 1 ms | light and heavy | 10 min each |
| Online change | R1 and R2, 3 repetitions, four cases | 1 ms | medium (base) | 5 s each, 2 s warm-up |
| Control cost | 3 legs, R0 | 1 ms | medium (base) | 10 min each |

Every steady run uses a 2 s warm-up; the measurement window starts at the cycle count reported
by the control status after the warm-up. The percentiles in this report are computed over the
measured window only.

### 3.3 Harness, schemas, and artifacts

- Harness: `tools/rt/campaign.py`, Python 3 standard library, installed root-owned on the host.
  It starts the bundled launcher, drives the local control socket, starts and stops the load,
  samples `/proc`, and writes the summary.
- Metrics: `stc-rt-metrics/1` from the launcher, one record per cycle plus a header and summary.
- Summary: `stc-rt-summary/1`, with min/p50/p90/p99/p99.9/max for execution and wake lateness,
  counts, faults, and allocator deltas.
- Control: protocol 1, newline-delimited JSON over a user-only Unix socket with an ephemeral
  nonce. The harness never logs the nonce.
- Launcher build: base `0x638faa446167b23b679c92aef25430e0`; light
  `0x943c00c72947547d2096d6b49a75cc8a`; heavy `0x21e8cd3977f5f84a07501fdc75f24cac`.
- Candidate builds: code-only `0xdd995f1178173eed496aa6e953e5a194`; field-add-remove
  `0xc141df556c3e4b52917d67b18759f287`; incompatible `0x063d745e8db613e698e0449e63d6acc1`;
  trapping `0xfcf725d76618be2e4bd78983f748d42a`.

The campaign artifacts are listed in §8. The raw archive is local and gitignored; the committed
manifest is [RT-Run-Manifest.md](RT-Run-Manifest.md).

### 3.4 Metric definitions

- `deadline_ns` is the scheduled start of the cycle on `CLOCK_MONOTONIC`.
- `wake_lateness_ns` is `wake - deadline`. It is signed; positive values mean the cycle thread
  woke after the scheduled start.
- `execution_ns` is the cycle execution time from scan start to scan end.
- `scan_overrun_ns` is the completion after `deadline + cycle_time`.
- `missed_deadline` is true when either the wake or the completion is late by any amount. This is
  a strict counter; see §5.
- `skipped_periods` is the number of whole periods skipped when the next deadline had already
  passed.
- Fault and allocator fields are sampled at cycle 1 and every 1000 cycles; the summary reports
  the first and last sample and the delta.

## 4. Results

### 4.1 Medium workload, 1 ms

The values are ranges across the three repetitions; the per-run values are in
[RT-Run-Manifest.md](RT-Run-Manifest.md).

| Regime | Reps | Cycles | Skipped | Exec p50 (ms) | Exec p90 (ms) | Exec p99 (ms) | Exec p99.9 (ms) | Exec max (ms) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| R0 quiet | 3 | 598,563–599,705 | 300–1,442 | 0.663–0.782 | 0.907–0.933 | 0.929–0.947 | 0.954–1.014 | 1.263–1.325 |
| R1 other-CPU mixed load | 3 | 177,942–188,901 | 411,169–422,166 | 2.580–2.681 | 3.207–3.305 | 3.429–3.476 | 3.512–3.568 | 3.602–3.647 |
| R2 same-CPU mixed load | 3 | 599,470–599,883 | 121–535 | 0.649–0.659 | 0.697–0.706 | 0.758–0.803 | 0.801–0.821 | 0.870–0.882 |

| Regime | Reps | Wake p50 (µs) | Wake p90 (µs) | Wake p99 (µs) | Wake p99.9 (µs) | Wake max (µs) |
| --- | --- | --- | --- | --- | --- | --- |
| R0 quiet | 3 | 6.1–6.9 | 9.7–11.5 | 12.3–12.9 | 14.2–42.9 | 1,266.6–1,709.0 |
| R1 other-CPU mixed load | 3 | 20.9–23.7 | 31.3–35.3 | 47.9–96.8 | 62.4–116.3 | 4,816.4–268,762.5 |
| R2 same-CPU mixed load | 3 | 7.6–7.7 | 9.8–10.4 | 12.8–14.4 | 19.6–24.1 | 1,295.7–306,311.6 |

The max column contains isolated samples, not representative tail behaviour; the p99.9 column is
the tail metric used in the interpretation. The largest wake maxima are 1.7 ms in R0, 268.8 ms
in R1, and 306.3 ms in R2. The cause of the isolated multi-hundred-millisecond samples is not
resolved by this campaign; they are retained in the raw data and are not treated as a guarantee
or a bound.

### 4.2 Secondary 10 ms runs

| Regime | Cycles | Skipped | Exec p50/p90/p99/p99.9/max (ms) | Wake p50/p90/p99/p99.9/max (µs) |
| --- | --- | --- | --- | --- |
| R0 quiet | 60,003 | 0 | 1.242 / 1.387 / 1.421 / 1.432 / 1.548 | 67.0 / 86.5 / 192.4 / 209.7 / 261.6 |
| R1 other-CPU mixed load | 60,003 | 0 | 2.540 / 3.219 / 3.402 / 3.485 / 3.598 | 52.3 / 75.4 / 93.3 / 104.6 / 132.4 |
| R2 same-CPU mixed load | 60,003 | 0 | 0.756 / 0.873 / 0.992 / 1.042 / 1.206 | 13.0 / 16.7 / 22.4 / 28.8 / 37.5 |

The 10 ms period is met in all three regimes, including R1, with no skipped periods and no
material wake-lateness tail.

### 4.3 Workload levels

| Workload | Regime | Cycles | Skipped | Exec p50/p90/p99/p99.9/max (ms) | Wake p50/p90/p99/p99.9/max (µs) |
| --- | --- | --- | --- | --- | --- |
| Light | R0 | 598,461 | 1,543 | 0.249 / 0.250 / 0.256 / 0.282 / 0.291 | 8.7 / 9.4 / 12.7 / 21.1 / 772,436.4 |
| Medium (base) | R0 | 598,563–599,705 | 300–1,442 | 0.663–0.782 / 0.907–0.933 / 0.929–0.947 / 0.954–1.014 / 1.263–1.325 | 6.1–6.9 / 9.7–11.5 / 12.3–12.9 / 14.2–42.9 / 1,266.6–1,709.0 |
| Heavy | R0 | 598,453 | 1,551 | 0.784 / 0.948 / 0.957 / 1.019 / 1.927 | 4.0 / 11.4 / 12.0 / 15.6 / 884.0 |

The calibration measured a fixed cycle cost of about 0.19 ms; the light workload at `WORK=2000`
is about 25% of the 1 ms budget rather than below a 0.2 ms numeric target. The three levels are
still separated as intended: light ≈0.25 ms, medium ≈0.66–0.78 ms, heavy ≈0.78–0.95 ms at p99.
The light wake max of 772.4 ms is one isolated sample; it is not representative.

### 4.4 Control/status-thread cost

| Control leg | Cycles | Skipped | Exec p50/p90/p99/p99.9/max (ms) | Wake p50/p90/p99/p99.9/max (µs) |
| --- | --- | --- | --- | --- |
| Disabled | 600,002 | 2,001 | 0.509 / 0.937 / 0.947 / 1.004 / 1.638 | 7.0 / 11.2 / 11.9 / 97.7 / 649,377.5 |
| Enabled, idle (`--status-hz 0`) | 598,899 | 1,105 | 0.624 / 0.929 / 0.943 / 1.014 / 1.273 | 6.8 / 11.5 / 12.1 / 15.8 / 1,389.9 |
| Enabled, 100 Hz status | 597,864 | 2,142 | 0.748 / 0.929 / 0.959 / 1.013 / 1.287 | 7.0 / 11.6 / 14.6 / 86.7 / 1,316.7 |

The control thread adds about 116 µs to execution p50 when idle and about 240 µs with 100 Hz
status polling, compared with the control-disabled leg. The p90 and p99 values are close because
the medium workload already dominates the tail. The control-disabled leg estimates its warm-up
cycle boundary from the cycle count; over 10 minutes the error is negligible. The 649.4 ms wake
maximum in the disabled leg is an isolated sample; it is not representative.

### 4.5 Online change under load

| Regime | Case | Result | Activation cycles (3 reps) | Rollback cycles (3 reps) | Deploy latency (ms, 3 reps) |
| --- | --- | --- | --- | --- | --- |
| R1 | code-only | Passed, activated | 1009 / 899 / 913 | — | 56.3 / 55.3 / 126.3 |
| R1 | field-add-remove | Passed, activated | 788 / 792 / 780 | — | 46.2 / 74.8 / 53.5 |
| R1 | incompatible | Passed, `cold_required` | — | — | 14.9 / 25.6 / 26.7 |
| R1 | trapping | Passed, rollback | 753 / 759 / 757 | 754 / 760 / 758 | 38.4 / 39.8 / 29.4 |
| R2 | code-only | Passed, activated | 1777 / 2022 / 2019 | — | 2.96 / 3.64 / 3.57 |
| R2 | field-add-remove | Passed, activated | 2024 / 2020 / 2024 | — | 4.33 / 3.76 / 4.13 |
| R2 | incompatible | Passed, `cold_required` | — | — | 1.84 / 3.45 / 1.92 |
| R2 | trapping | Passed, rollback | 2021 / 2019 / 2021 | 2022 / 2020 / 2022 | 3.97 / 3.95 / 3.78 |

All 24 case runs passed. Compatible candidates changed the active generation; the incompatible
candidate kept the base generation active and returned `cold_required`; the trapping candidate
activated and rolled back on the next cycle in every repetition. The deploy latency is the
request round trip and includes candidate open, compatibility reporting, load, prepare, and
activation on the cycle thread. The R2 values (1.8–4.3 ms) are much lower than the R1 values
(14.9–126.3 ms) because R1 carries the heavy external mixed load. Deployment latency is
maintenance latency, not cycle jitter.

### 4.6 System noise floor, R0 untuned

| Recorder | Configuration | Result |
| --- | --- | --- |
| cyclictest | `cyclictest -m -S -p90 -i200 -h1000 -D600 -q` | Min 2/2/2/2 µs; avg 4/4/4/4 µs; max 32/40/27/32 µs; no histogram overflows |
| timerlat | `rtla timerlat hist -d 10m` | Per-CPU max IRQ 41/69/61/62 µs; thread 52/84/74/71 µs; user 59/93/82/81 µs; all max 69 IRQ / 84 thread / 93 user µs; about 2.4M samples per class |
| osnoise | `rtla osnoise hist -d 10m` | Max 8,493 / 10,675 / 10,297 / 22,775 µs on CPUs 0–3; avg 4.49 / 4.65 / 4.14 / 4.89 µs; counts 891,903 / 921,151 / 923,026 / 863,760 |
| hwlat | `hwlatdetect --duration 10m` | Max 13 µs; 12 samples recorded; 12 above threshold; rc=1 is the tool's threshold-exceeded status, not a missing run |

The noise floor is untuned and includes normal background services. It began immediately after
the calibration checks, with a 1-minute load average of 2.57; the main campaign ran after the
noise floor. The osnoise maxima are operating-system noise measurements under the tracer's own
workload, not the cycle thread's wake latency. The raw files and SHA256 values are in
[RT-Run-Manifest.md](RT-Run-Manifest.md).

### 4.7 Faults and allocations

| Group | Minor faults | Major faults | Allocations / frees |
| --- | --- | --- | --- |
| 1 ms medium R0 | 1 / 1 / 1 | 0 | 0 / 0 |
| 1 ms medium R1 | 169 / 170 / 82 | 0 | 0 / 0 |
| 1 ms medium R2 | 1 / 1 / 1 | 0 | 0 / 0 |
| 10 ms R0/R1/R2 | 170 / 170 / 171 | 0 | 0 / 0 |
| Light / heavy R0 | 1 / 86 | 0 | 0 / 0 |
| Control disabled / idle / 100 Hz | 86 / 1 / 115 | 0 | 0 / 0 |

The 1 ms medium runs show one minor fault in the quiet and same-CPU cases and 82–170 under the
external mixed load. The 10 ms runs and some control legs show about 170 minor faults; the
per-run first/last samples are in the manifest. Major faults are zero. The generated benchmark
performs no transient allocation in steady state, so the allocation/free deltas are zero in every
run.

## 5. Interpretation

**R0 is close to the 1 ms budget.** The medium workload uses 0.66–0.78 ms at p50 and 0.93–0.95 ms
at p99, leaving little headroom. The measured fixed cycle cost is about 0.19 ms, which is why the
light workload is about 0.25 ms rather than below 0.2 ms. The wake lateness is low in the normal
case (p50 6–7 µs, p99 12–13 µs), but the max column contains isolated 1.3–1.7 ms samples.

**R1 is a heavy external-load stress case, not a light background load.** The mixed stress-ng
profile runs on CPUs 0–2 and contends for memory bandwidth, page cache, block I/O, and interrupt
handling that the cycle thread still needs. The FIFO cycle thread cannot preempt that external
contention, so execution p50 rises to 2.58–2.68 ms and the loop skips 411k–422k of the 600k
expected periods. The 10 ms R1 run meets its period (p99 3.40 ms, no skips), so the host can
sustain 10 ms under this load but not 1 ms. The R1 load is deliberately stronger than ordinary
desktop background work; it is a stress case.

**R2 shows the FIFO policy protecting the cycle thread from same-CPU lower-class load.** The
stress workers are pinned to CPU 3 with the cycle thread, but they run in SCHED_OTHER; the
FIFO:90 cycle thread preempts them. Execution p50 stays at 0.65–0.66 ms and the loop skips only
121–535 periods. The isolated wake maxima up to 306 ms show that a single event can still occur;
they are not representative of the p99.9 tail (19.6–24.1 µs). A different same-CPU load that
holds a kernel lock or blocks in a driver could behave differently; this measurement covers the
mixed stress-ng profile only.

**Control cost is measurable at the 1 ms budget.** The control thread adds about 116 µs to
execution p50 when idle and about 240 µs with 100 Hz status polling. The p50 shift is the useful
signal because the medium workload already occupies most of the tail. The control-disabled leg
estimates its warm-up boundary; the error is negligible over 10 minutes.

**Online change is maintenance work on the cycle thread.** The deploy latency includes candidate
open, compatibility reporting, load, prepare, and activation between scans. R2 absorbs this in
1.8–4.3 ms; R1 takes 14.9–126.3 ms under the external load. Compatible changes activate, the
incompatible candidate is rejected as cold-required with the base generation active, and the
trapping candidate rolls back on the next cycle in every repetition. The rollback window and the
O(1) activation contract hold in this campaign.

**The strict missed-deadline counter is not a hard-miss count.** The schema marks a deadline
missed when the wake or the completion is late by any amount. Wake lateness is measured from the
scheduled start and is almost always positive (p50 6–7 µs in R0), so the counter reports nearly
every cycle. The execution and wake-lateness percentiles, and the skipped-period count, are the
primary evidence. The report does not turn the counter into a pass/fail threshold.

**The noise floor is untuned and conservative.** cyclictest and timerlat show low tens of
microseconds; hwlat shows a 13 µs maximum; osnoise shows 8.5–22.8 ms maxima. The osnoise sample
started with a 2.57 one-minute load average after the calibration checks, and it includes normal
desktop services. The osnoise maxima are not the cycle thread's wake latency, but they are real
untuned noise evidence and they meet the deferred tuned-profile trigger.

## 6. Deferred tuned-profile evaluation

The trigger in [WL-4-Plan.md](../../WL-4-Plan.md) §6.2 is "any missed deadline in the untuned
campaign, or a report review that judges jitter or latency unsatisfactory." The strict counter is
nonzero in every run, and the R1 heavy mixed-load result (p50 2.58–2.68 ms at a 1 ms target) and
the osnoise maxima (8.5–22.8 ms) are unsatisfactory for a 1 ms hard cycle. The trigger is met.

The tuned profile remains deferred future work. It was not executed in v2.0.0 and is not part of
the v2.0.0 scope. Adopting it later is a separate scope decision; the recorded profile,
verification commands, and reboot cost are in [WL-4-Plan.md](../../WL-4-Plan.md) §6.2. No tuned
measurement is claimed in this report.

## 7. Limits

- One untuned host and configuration; the results are not portable to another CPU, kernel,
  firmware, service set, or power profile.
- The R1 mixed load is a heavy external stress case, not a light background load. It is not a
  model of every deployed load.
- The strict `missed_deadline` counter counts any positive wake lateness. Use the execution and
  wake-lateness percentiles and the skipped-period count; the counter is not a hard-miss count.
- The max columns contain isolated outliers, including 772 ms in the light run and 649 ms in the
  control-disabled run. They are retained for honesty but are not representative tails and are
  not bounds. The cause of the multi-hundred-millisecond samples is not resolved.
- The metrics are buffered and flushed at exit; a hard process crash can lose buffered records.
- The control-disabled leg estimates its warm-up cycle boundary from the cycle count. The error
  is negligible over a 10 minute run but is not a fixed sample boundary.
- Deploy prepare/load/activation runs on the cycle thread; deployment latency is maintenance
  latency, not cycle jitter. Fully off-thread prepare is post-v1 work.
- The untuned host has deep idle states, no CPU isolation, and normal background services. The
  tuned profile in the plan was not applied.
- The light workload is about 25% of the 1 ms budget, not below a 0.2 ms numeric target, because
  the host has a measured fixed cycle cost of about 0.19 ms.
- This report makes no hard real-time guarantee, no safety claim, and no timing budget. It is
  informational evidence for the host and configuration measured.

## 8. Reproduce

### 8.1 Artifacts

| Item | Value |
| --- | --- |
| Campaign `rt-build` | [40958283937](https://github.com/r2k-in-the-vortex/stc/actions/runs/40958283937) on `2be4325` |
| Final reproducibility `rt-build` | [40985821634](https://github.com/r2k-in-the-vortex/stc/actions/runs/40985821634) on `cf45f62`; reproduced base build ID `0x638faa446167b23b679c92aef25430e0` |
| Linux package | `stc-1.0.0-x86_64-unknown-linux-gnu`; `SHA256SUMS` SHA256 `415cca1343d6f299703aaad73f346bc8f5d781daf2a8e348a2ec7696f2b098a4` |
| `rt-bench` bundle | `SHA256SUMS` SHA256 `6107e867b303c897ab5e940eeea06b96aa7912bd69b454bf241620238dbbcf50` |
| Harness | `tools/rt/campaign.py` SHA256 `02c3f6b8afd6c70a5a98f500dc5fe7de4140bd34d001290e65249780c021f001`, installed root-owned |
| Raw archive | Local gitignored `dist/rt/results/phase4`, 420 files; `SHA256SUMS` SHA256 `23f4daa2af10512978cc5fcda52d633dccf012f163b982ea035e3518f1623a3b` |

The package and bundle were built in CI; the RT host never builds. The artifact transfer and
checksum procedure, the privileged wrapper, and the host setup are in
[WL-4-Plan.md](../../WL-4-Plan.md) §9.

### 8.2 Commands

The harness runs as root through the phase 0 wrapper. Replace `<run>` with a unique result
directory name.

```sh
# Main steady, medium workload. Repeat for r1 (--regime r1 --load-profile mixed)
# and r2 (--regime r2 --load-profile mixed).
sudo -n /usr/local/sbin/stc-wl4-run campaign --regime r0 --duration 10m \
  --cycle-time 1ms --workload base --load-profile none --rt-cpu 3 --rt-priority 90 \
  --out /home/r2k/stc-rt/results/<run>

# Secondary 10 ms run.
sudo -n /usr/local/sbin/stc-wl4-run campaign --regime r0 --duration 10m \
  --cycle-time 10ms --workload base --load-profile none --rt-cpu 3 --rt-priority 90 \
  --out /home/r2k/stc-rt/results/<run>

# Light or heavy workload.
sudo -n /usr/local/sbin/stc-wl4-run campaign --regime r0 --duration 10m \
  --cycle-time 1ms --workload light --load-profile none --rt-cpu 3 --rt-priority 90 \
  --out /home/r2k/stc-rt/results/<run>

# Online change under load. Use reg r1 or r2; all four cases run in one invocation.
sudo -n /usr/local/sbin/stc-wl4-run campaign --mode online --online-case all \
  --regime r1 --load-profile mixed --duration 5s --cycle-time 1ms \
  --out /home/r2k/stc-rt/results/<run>

# Control-cost legs: --control-mode none, then socket with --status-hz 0, then 100.
sudo -n /usr/local/sbin/stc-wl4-run campaign --regime r0 --duration 10m \
  --cycle-time 1ms --workload base --load-profile none --control-mode none \
  --out /home/r2k/stc-rt/results/<run>

# Noise floor.
sudo -n /usr/local/sbin/stc-wl4-run cyclictest 600
sudo -n /usr/local/sbin/stc-wl4-run timerlat 10m
sudo -n /usr/local/sbin/stc-wl4-run osnoise 10m
sudo -n /usr/local/sbin/stc-wl4-run hwlat 10m
```

The complete runbook, including the clean-boot check that the untuned host state is unchanged,
is [WL-4-Plan.md](../../WL-4-Plan.md) §9.

### 8.3 Runbook

1. Verify the host is on the untuned boot profile and record the host snapshot; do not change
   boot parameters, governor, C-states, IRQ affinity, services, or swap.
2. Transfer the package and bundle from CI; verify both `SHA256SUMS` files; set executable bits
   where the artifact download dropped them.
3. Install the harness root-owned; run the campaign matrix above; keep the raw result
   directories.
4. Stop the load and launcher; verify no `stress-ng` or `rt-bench` processes remain.
5. Collect the raw results, gzip the JSONL streams if not already gzipped, compute SHA256, and
   keep the archive outside the repository.

## 9. References

- [WL-4-Plan.md](../../WL-4-Plan.md) — scope, decisions, host baseline, runbook, deferred tuned
  profile.
- [RT-Run-Manifest.md](RT-Run-Manifest.md) — per-run tables, host snapshots, raw file SHA256
  values.
- [Performance.md](Performance.md) — the full performance record, including this section.
- [KNOWN-LIMITATIONS.md](../../KNOWN-LIMITATIONS.md) — the documented real-time limits.
- [NON-SAFETY.md](../../NON-SAFETY.md) — the safety stance.
- [Runtime and Execution](../Architecture/Runtime-and-Execution.md) — the cycle loop, thread
  classes, and host ABI.
- [Online Change and GOT](../Architecture/Online-Change-and-GOT.md) — the prepare/activate/
  rollback model.
