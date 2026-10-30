# Phase 4 RT campaign run manifest

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-30  
Status: Phase 4 evidence for the untuned v2.0.0 campaign. Raw samples remain in the
local campaign archive; the report is the phase 5 deliverable.  
Parent: [WL-4-Plan.md](../../WL-4-Plan.md)  
Schema: `stc-rt-summary/1` and `stc-rt-metrics/1`

## 1. Environment

The campaign ran on the same untuned host recorded in the parent plan: Ubuntu 25.10, Linux
`6.17.999-8-generic #8 SMP PREEMPT_RT`, Intel N150 (4 cores), 10 GiB RAM. No boot parameter,
governor, C-state, IRQ affinity, service, or swap setting was changed.

Host snapshot before the campaign:

```text
2026-10-29T21:34:41+00:00
Linux r2k 6.17.999-8-generic #8 SMP PREEMPT_RT Mon Jan 26 06:59:24 UTC 2026 x86_64 GNU/Linux
BOOT_IMAGE=/boot/vmlinuz-6.17.999-8-generic root=UUID=14365534-cc30-4b24-a7df-650f18e3a50b ro crashkernel=2G-4G:320M,4G-32G:512M,32G-64G:1024M,64G-128G:2048M,128G-:4096M

(null)
/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor powersave
/sys/devices/system/cpu/cpu1/cpufreq/scaling_governor powersave
/sys/devices/system/cpu/cpu2/cpufreq/scaling_governor powersave
/sys/devices/system/cpu/cpu3/cpufreq/scaling_governor powersave
/sys/devices/system/cpu/cpu0/cpuidle/state0 POLL 0
/sys/devices/system/cpu/cpu0/cpuidle/state1 C1E 2
/sys/devices/system/cpu/cpu0/cpuidle/state2 C6 195
/sys/devices/system/cpu/cpu0/cpuidle/state3 C8 260
/sys/devices/system/cpu/cpu0/cpuidle/state4 C10 660
 141:          0          0          0          3 IR-PCI-MSIX-0000:03:00.0    0-edge      enp3s0
 142:   37718975          0          0          0 IR-PCI-MSIX-0000:03:00.0    1-edge      enp3s0-TxRx-0
 143:          0   40612348          0          0 IR-PCI-MSIX-0000:03:00.0    2-edge      enp3s0-TxRx-1
 144:          0          0   35072043          0 IR-PCI-MSIX-0000:03:00.0    3-edge      enp3s0-TxRx-2
 145:          0          0          0   20990136 IR-PCI-MSIX-0000:03:00.0    4-edge      enp3s0-TxRx-3
NAME      TYPE SIZE  USED PRIO
/swap.img file   4G 69.2M   -2
27
 21:34:41 up 21 days, 15:51,  5 users,  load average: 2.57, 1.29, 0.66
# rt-bench — WL-4 benchmark bundle

Built from 2be4325 with stc 1.0.0.

## Layout

6107e867b303c897ab5e940eeea06b96aa7912bd69b454bf241620238dbbcf50  /home/r2k/stc-rt/rt-bench/SHA256SUMS
-rwxr-xr-x 1 r2k r2k 5363352 Oct 29 21:32 /home/r2k/stc-rt/rt-bench/rt-bench
```

## 2. Artifacts and harness

| Item | Value |
| --- | --- |
| `rt-build` run | [40958283937](https://github.com/r2k-in-the-vortex/stc/actions/runs/40958283937) on `2be4325` |
| Linux package | `stc-1.0.0-x86_64-unknown-linux-gnu`, 11,080,726 bytes; `SHA256SUMS` SHA256 `415cca1343d6f299703aaad73f346bc8f5d781daf2a8e348a2ec7696f2b098a4` |
| `rt-bench` bundle | 7,093,490 bytes; `SHA256SUMS` SHA256 `6107e867b303c897ab5e940eeea06b96aa7912bd69b454bf241620238dbbcf50` |
| Harness | `tools/rt/campaign.py`, SHA256 `02c3f6b8afd6c70a5a98f500dc5fe7de4140bd34d001290e65249780c021f001`, installed root-owned |
| Base build ID | `0x638faa446167b23b679c92aef25430e0` |
| Light build ID | `0x943c00c72947547d2096d6b49a75cc8a` |
| Heavy build ID | `0x21e8cd3977f5f84a07501fdc75f24cac` |
| code-only candidate | `0xdd995f1178173eed496aa6e953e5a194` |
| field-add-remove candidate | `0xc141df556c3e4b52917d67b18759f287` |
| incompatible candidate | `0x063d745e8db613e698e0449e63d6acc1` |
| trapping candidate | `0xfcf725d76618be2e4bd78983f748d42a` |

Workloads were calibrated from the phase 3 dry runs: base/candidates `WORK=10000`,
light `WORK=2000`, heavy `WORK=18000`. The 10 s calibration checks measured execution
p50 0.497 ms (base), 0.248 ms (light), and 0.797 ms (heavy) at a 1 ms cycle.

## 3. Main steady campaign (medium workload)

| Run | Regime | Workload | Cycle | Control | Load | Result | Cycles (meas/exp) | Missed | Skipped | Execution p50/p99/max | Wake p50/p99/max | Minor/major faults | Alloc/free |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `phase4-main-r0-10ms` | r0 | base | 10.000 ms | socket | none | passed | 60003/60000 | 60003 | 0 | 1.242 ms / 1.421 ms / 1.548 ms | 67.038 µs / 192.358 µs / 261.639 µs | 170/0 | 0/0 |
| `phase4-main-r0-1ms-rep1` | r0 | base | 1.000 ms | socket | none | passed | 599705/600000 | 599705 | 300 | 762.502 µs / 929.370 µs / 1.263 ms | 6.113 µs / 12.937 µs / 1.361 ms | 1/0 | 0/0 |
| `phase4-main-r0-1ms-rep2` | r0 | base | 1.000 ms | socket | none | passed | 598563/600000 | 598563 | 1442 | 662.845 µs / 946.870 µs / 1.325 ms | 6.935 µs / 12.264 µs / 1.267 ms | 1/0 | 0/0 |
| `phase4-main-r0-1ms-rep3` | r0 | base | 1.000 ms | socket | none | passed | 599629/600000 | 599629 | 375 | 781.945 µs / 933.661 µs / 1.280 ms | 6.266 µs / 12.688 µs / 1.709 ms | 1/0 | 0/0 |
| `phase4-main-r1-10ms` | r1 | base | 10.000 ms | socket | mixed | passed | 60003/60000 | 60003 | 0 | 2.540 ms / 3.402 ms / 3.598 ms | 52.331 µs / 93.347 µs / 132.422 µs | 170/0 | 0/0 |
| `phase4-main-r1-1ms-rep1` | r1 | base | 1.000 ms | socket | mixed | passed | 177942/600000 | 177942 | 422166 | 2.681 ms / 3.476 ms / 3.647 ms | 23.697 µs / 47.862 µs / 4.816 ms | 169/0 | 0/0 |
| `phase4-main-r1-1ms-rep2` | r1 | base | 1.000 ms | socket | mixed | passed | 186096/600000 | 186096 | 414039 | 2.603 ms / 3.451 ms / 3.607 ms | 21.361 µs / 48.666 µs / 52.748 ms | 170/0 | 0/0 |
| `phase4-main-r1-1ms-rep3` | r1 | base | 1.000 ms | socket | mixed | passed | 188901/600000 | 188901 | 411169 | 2.580 ms / 3.429 ms / 3.602 ms | 20.943 µs / 96.820 µs / 268.763 ms | 82/0 | 0/0 |
| `phase4-main-r2-10ms` | r2 | base | 10.000 ms | socket | mixed | passed | 60003/60000 | 60003 | 0 | 756.491 µs / 991.502 µs / 1.206 ms | 13.045 µs / 22.443 µs / 37.485 µs | 171/0 | 0/0 |
| `phase4-main-r2-1ms-rep1` | r2 | base | 1.000 ms | socket | mixed | passed | 599470/600000 | 599470 | 535 | 658.846 µs / 757.692 µs / 878.261 µs | 7.726 µs / 12.830 µs / 306.312 ms | 1/0 | 0/0 |
| `phase4-main-r2-1ms-rep2` | r2 | base | 1.000 ms | socket | mixed | passed | 599837/600000 | 599837 | 167 | 649.093 µs / 790.097 µs / 870.030 µs | 7.604 µs / 13.930 µs / 56.339 ms | 1/0 | 0/0 |
| `phase4-main-r2-1ms-rep3` | r2 | base | 1.000 ms | socket | mixed | passed | 599883/600000 | 599883 | 121 | 659.327 µs / 802.753 µs / 882.256 µs | 7.593 µs / 14.353 µs / 1.296 ms | 1/0 | 0/0 |

## 4. Supplementary workload levels (R0)

| Run | Regime | Workload | Cycle | Control | Load | Result | Cycles (meas/exp) | Missed | Skipped | Execution p50/p99/max | Wake p50/p99/max | Minor/major faults | Alloc/free |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `phase4-heavy-r0-1ms` | r0 | heavy | 1.000 ms | socket | none | passed | 598453/600000 | 598453 | 1551 | 784.330 µs / 956.956 µs / 1.927 ms | 4.008 µs / 11.974 µs / 883.966 µs | 86/0 | 0/0 |
| `phase4-light-r0-1ms` | r0 | light | 1.000 ms | socket | none | passed | 598461/600000 | 598461 | 1543 | 248.576 µs / 256.146 µs / 291.411 µs | 8.664 µs / 12.688 µs / 772.436 ms | 1/0 | 0/0 |

## 5. Control/status-thread cost (R0, medium, 1 ms)

| Run | Regime | Workload | Cycle | Control | Load | Result | Cycles (meas/exp) | Missed | Skipped | Execution p50/p99/max | Wake p50/p99/max | Minor/major faults | Alloc/free |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `phase4-control-100hz` | r0 | base | 1.000 ms | socket | none | passed | 597864/600000 | 597864 | 2142 | 748.372 µs / 958.778 µs / 1.287 ms | 7.023 µs / 14.554 µs / 1.317 ms | 115/0 | 0/0 |
| `phase4-control-idle` | r0 | base | 1.000 ms | socket | none | passed | 598899/600000 | 598899 | 1105 | 624.360 µs / 943.403 µs / 1.273 ms | 6.798 µs / 12.081 µs / 1.390 ms | 1/0 | 0/0 |
| `phase4-control-none` | r0 | base | 1.000 ms | none | none | passed | 600002/600000 | 600002 | 2001 | 508.543 µs / 946.508 µs / 1.638 ms | 6.952 µs / 11.916 µs / 649.377 ms | 86/0 | 0/0 |

## 6. Online change under load

| Run | Regime | Case | Result | Cold required | Rollback observed | Activation cycle | Rollback cycle | Deploy response latency | Candidate build |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `phase4-online-r1-rep1` | r1 | code-only | passed | False | False | 1009 | None | 56.322 ms | `0xdd995f1178173eed496aa6e953e5a194` |
| `phase4-online-r1-rep1` | r1 | field-add-remove | passed | False | False | 788 | None | 46.194 ms | `0xc141df556c3e4b52917d67b18759f287` |
| `phase4-online-r1-rep1` | r1 | incompatible | passed | True | False | None | None | 14.859 ms | `0x063d745e8db613e698e0449e63d6acc1` |
| `phase4-online-r1-rep1` | r1 | trapping | passed | False | True | 753 | 754 | 38.412 ms | `0xfcf725d76618be2e4bd78983f748d42a` |
| `phase4-online-r1-rep2` | r1 | code-only | passed | False | False | 899 | None | 55.268 ms | `0xdd995f1178173eed496aa6e953e5a194` |
| `phase4-online-r1-rep2` | r1 | field-add-remove | passed | False | False | 792 | None | 74.804 ms | `0xc141df556c3e4b52917d67b18759f287` |
| `phase4-online-r1-rep2` | r1 | incompatible | passed | True | False | None | None | 25.596 ms | `0x063d745e8db613e698e0449e63d6acc1` |
| `phase4-online-r1-rep2` | r1 | trapping | passed | False | True | 759 | 760 | 39.760 ms | `0xfcf725d76618be2e4bd78983f748d42a` |
| `phase4-online-r1-rep3` | r1 | code-only | passed | False | False | 913 | None | 126.337 ms | `0xdd995f1178173eed496aa6e953e5a194` |
| `phase4-online-r1-rep3` | r1 | field-add-remove | passed | False | False | 780 | None | 53.536 ms | `0xc141df556c3e4b52917d67b18759f287` |
| `phase4-online-r1-rep3` | r1 | incompatible | passed | True | False | None | None | 26.687 ms | `0x063d745e8db613e698e0449e63d6acc1` |
| `phase4-online-r1-rep3` | r1 | trapping | passed | False | True | 757 | 758 | 29.386 ms | `0xfcf725d76618be2e4bd78983f748d42a` |
| `phase4-online-r2-rep1` | r2 | code-only | passed | False | False | 1777 | None | 2.958 ms | `0xdd995f1178173eed496aa6e953e5a194` |
| `phase4-online-r2-rep1` | r2 | field-add-remove | passed | False | False | 2024 | None | 4.334 ms | `0xc141df556c3e4b52917d67b18759f287` |
| `phase4-online-r2-rep1` | r2 | incompatible | passed | True | False | None | None | 1.840 ms | `0x063d745e8db613e698e0449e63d6acc1` |
| `phase4-online-r2-rep1` | r2 | trapping | passed | False | True | 2021 | 2022 | 3.968 ms | `0xfcf725d76618be2e4bd78983f748d42a` |
| `phase4-online-r2-rep2` | r2 | code-only | passed | False | False | 2022 | None | 3.640 ms | `0xdd995f1178173eed496aa6e953e5a194` |
| `phase4-online-r2-rep2` | r2 | field-add-remove | passed | False | False | 2020 | None | 3.757 ms | `0xc141df556c3e4b52917d67b18759f287` |
| `phase4-online-r2-rep2` | r2 | incompatible | passed | True | False | None | None | 3.452 ms | `0x063d745e8db613e698e0449e63d6acc1` |
| `phase4-online-r2-rep2` | r2 | trapping | passed | False | True | 2019 | 2020 | 3.952 ms | `0xfcf725d76618be2e4bd78983f748d42a` |
| `phase4-online-r2-rep3` | r2 | code-only | passed | False | False | 2019 | None | 3.570 ms | `0xdd995f1178173eed496aa6e953e5a194` |
| `phase4-online-r2-rep3` | r2 | field-add-remove | passed | False | False | 2024 | None | 4.133 ms | `0xc141df556c3e4b52917d67b18759f287` |
| `phase4-online-r2-rep3` | r2 | incompatible | passed | True | False | None | None | 1.919 ms | `0x063d745e8db613e698e0449e63d6acc1` |
| `phase4-online-r2-rep3` | r2 | trapping | passed | False | True | 2021 | 2022 | 3.780 ms | `0xfcf725d76618be2e4bd78983f748d42a` |

## 7. System noise floor (R0, untuned)

| Recorder | Key summary lines | Raw file SHA256 |
| --- | --- | --- |
| `cyclictest` (cyclictest -m -S -p90 -i200 -h1000 -D600 -q) | # Min Latencies: 00002 00002 00002 00002; # Avg Latencies: 00004 00004 00004 00004; # Max Latencies: 00032 00040 00027 00032; # Histogram Overflows: 00000 00000 00000 00000 | `6653f9d4f87ebc2f08ee2bbb96df36dc99a58e63815248a5d75d1c6087ac0890` |
| `timerlat` (rtla timerlat hist -d 10m) | min:          0         1         2         0         1         2         0         1         2         0         1         2; avg:          2         7        10         2         7        11         2         7        10         6        16        23; max:         41        52        59        69        84        93        61        74        82        62        71        81; ALL:        IRQ       Thr       Usr; count:  2399977   2399978   2399978; min:          0         1         2; avg:          3         9        14; max:         69        84        93 | `aa99d538e5d85b4bead9db1b6e661eb0d598ac0c9c28e96455cde05585c581ef` |
| `osnoise` (rtla osnoise hist -d 10m) | count:   891903    921151    923026    863760; min:          1         1         1         1; avg:       4.49      4.65      4.14      4.89; max:       8493     10675     10297     22775 | `f4e0ad14949c344011cef0917201e8972055c0c0679379ce52dc0b7c147ea9f0` |
| `hwlat` (hwlatdetect --duration 10m) | Max Latency: 13us; Samples recorded: 12; Samples exceeding threshold: 12 | `2d28046675b9353c6040f4626833a961280c322c675776514478acadab76e504` |

## 8. Failures and notes

- noise-hwlat rc=1

Per-run wrapper exit codes (`logs/status.txt`):

```text
noise-cyclictest rc=0
noise-timerlat rc=0
noise-osnoise rc=0
noise-hwlat rc=1
main-r0-1ms-rep1 rc=0
main-r0-1ms-rep2 rc=0
main-r0-1ms-rep3 rc=0
main-r1-1ms-rep1 rc=0
main-r1-1ms-rep2 rc=0
main-r1-1ms-rep3 rc=0
main-r2-1ms-rep1 rc=0
main-r2-1ms-rep2 rc=0
main-r2-1ms-rep3 rc=0
main-r0-10ms rc=0
main-r1-10ms rc=0
main-r2-10ms rc=0
light-r0-1ms rc=0
heavy-r0-1ms rc=0
online-r1-rep1 rc=0
online-r1-rep2 rc=0
online-r1-rep3 rc=0
online-r2-rep1 rc=0
online-r2-rep2 rc=0
online-r2-rep3 rc=0
control-none rc=0
control-idle rc=0
control-100hz rc=0
```

- The `noise-hwlat` rc=1 is hwlatdetect's threshold-exceeded status, not a missing run: Max Latency: 13us; Samples recorded: 12; Samples exceeding threshold: 12.

The noise floor began immediately after the short calibration checks; the host snapshot's
1-minute load average was 2.57 at the start. The main campaign ran after the noise floor.
The R0 noise floor is untuned and includes normal host background services. The osnoise
maxima in §7 are the untuned noise signal; the parent plan reopens the deferred tuned
profile on any missed deadline or a judgment that jitter or latency is unsatisfactory.
Phase 5 evaluates that trigger. The harness counts a missed deadline when wake or scan
completion is late; the execution and wake-lateness percentiles above are the primary
timing evidence.

Host snapshot after the campaign:

```text
2026-10-30T01:12:41+00:00
 01:12:41 up 21 days, 19:29,  6 users,  load average: 1.33, 1.53, 1.88
```

## 9. Raw data

The raw campaign archive is local and gitignored at `dist/rt/results/phase4`; it contains
420 files. `SHA256SUMS` SHA256: `23f4daa2af10512978cc5fcda52d633dccf012f163b982ea035e3518f1623a3b`.
