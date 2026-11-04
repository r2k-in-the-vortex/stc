# Performance measurements — 1.0.0-rc.1

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-28  
Status: Informational; not a CI gate (D-193)

These numbers are measured, not expected. They describe one host in one
regime and must not be read as real-time guarantees. Timing is not part of the
release contract; the release contract is correctness, determinism, and the
documented failure behaviour.

## Environment

| Item | Value |
| --- | --- |
| Host | Windows 11 Pro, x86-64 (`x86_64-pc-windows-msvc`) |
| CPU | Intel64 Family 6 Model 183 Stepping 1 (GenuineIntel) |
| Rust | rustc 1.99.1 (58cf8f9c3 2026-10-13), cargo 1.99.1 |
| clang | clang 23.1.4 (`llvm-project 7a941dfb2877a0b8cf30323fefa85ad019e0b0f5`) |
| Build | workspace test profile for the CLI timings; generated modules as noted |
| Measurement | best of N, with warm-up; wall clock; shared-host noise applies |

## Build and workflow timings

Measured by `crates/stc-driver/tests/performance.rs` on a copy of
`examples/hello` (five production POUs including a TON, a task configuration,
and three tests). Each command is a full process invocation.

| Command | Best | Runs |
| --- | --- | --- |
| `stc check` | 9.208 ms | 10 |
| `stc build --emit=ir` | 28.025 ms | 5 |
| `stc build` (debug module) | 237.635 ms | 5 |
| `stc build --release` | 263.697 ms | 5 |
| `stc run` (100 cycles) | 697.029 ms | 5 |
| `stc run` (1100 cycles) | 711.055 ms | 5 |
| `stc run` marginal cycle cost | 14.026 µs/cycle | derived |
| `stc test` | 4198.963 ms | 3 |

The `stc run` marginal figure removes process start, build, and linking by
comparing 100 and 1100 cycles; it is wall-clock scheduling and I/O, not the
generated code's instruction cost.

Reproduce:

```sh
cargo test -p stc-driver --test performance --locked -- --ignored --nocapture
```

## Runtime overhead

Measured by `crates/stc-driver/tests/debug_overhead.rs`: the same generated
program built through the real driver pipeline, 50,000 cycles per run, best and
median of 15 runs after 2,000 warm-up cycles.

| Build form | Best | Median |
| --- | --- | --- |
| Release (`-O2`, stripped) | 1126.52 ns/cycle | 1150.04 ns/cycle |
| Debug hook-free (`-O0 -g`, test form) | 1312.45 ns/cycle | 1335.58 ns/cycle |
| Debug hooks (`-O0 -g`, 8 checkpoints) | 1551.28 ns/cycle | 1565.65 ns/cycle |
| Debug hooks + active observe session | 2573.39 ns/cycle | 2761.03 ns/cycle |
| Typed read through the host API | 34309.42 ns/read | 35592.11 ns/read |

Derived: hook cost 29.85 ns/hook; debug-hooks/release 1.38x; hook-free/release
1.17x; checkpoints are 15.4% of the debug cycle; an active observe session adds
1022.11 ns/cycle (127.76 ns/hook), including the per-cycle server pump. The
typed read resolves one `Acc` DINT through the in-process API, including
manifest target resolution and value decoding.

The hook-free comparison is the documented test form (§17.7), not a product
configuration; the CLI has no hook-free production debug mode. Release is the
product form. Hook-bearing debug is the default `stc build` form and is
intended for development, not for a timing-sensitive deployment.

Reproduce:

```sh
cargo test -p stc-driver --test debug_overhead --locked -- --ignored --nocapture
```

## AArch64 debug overhead (one ARM64 runner measurement)

This is one measurement from a single GitHub-hosted `ubuntu-24.04-arm` runner, taken from the
phase-4 CI run
[40792824763](https://github.com/r2k-in-the-vortex/stc/actions/runs/40792824763) (commit
`edd6fae`). It is one ARM64 host in one regime, informational and non-gating (D-193); do not
compare it with the Windows figures above without recording the host and toolchain difference.

| Item | Value |
| --- | --- |
| Host | Ubuntu 24.04 ARM64, `aarch64-unknown-linux-gnu`, runner `ubuntu-24.04-arm` |
| CPU | not reported by the runner (`unknown`) |
| Rust | rustc 1.99.1 (58cf8f9c3 2026-10-13) |
| clang | Ubuntu clang 18.1.3 (1ubuntu1) |
| Regime | same generator source; 50,000 cycles/run, best and median of 15 runs, 2,000 warm-up cycles |

| Build form | Best | Median |
| --- | --- | --- |
| Release (`-O2`, stripped) | 2189.56 ns/cycle | 2191.17 ns/cycle |
| Debug hook-free (`-O0 -g`, test form) | 2194.01 ns/cycle | 2196.99 ns/cycle |
| Debug hooks (`-O0 -g`, 8 checkpoints) | 2871.19 ns/cycle | 2872.73 ns/cycle |
| Debug hooks + active observe session | 4913.53 ns/cycle | 4916.48 ns/cycle |
| Typed read through the host API | 7289.83 ns/read | 7302.38 ns/read |

The log derives a hook cost of 84.65 ns/hook, debug-hooks/release 1.31x, hook-free/release
1.00x, and a checkpoint share of 23.6% of the debug cycle; an active observe session adds
2042.33 ns/cycle (255.29 ns/hook), including the per-cycle server pump. The hook-free row is the
documented test form, not a product configuration. The Ubuntu runner uses clang 18.1.3, unlike
the clang 23.1.4 used for the Windows figures above, so cross-host comparison is not meaningful.

## Activation latency

Measured by `crates/stc-runtime/tests/switch_latency.rs`: cold install, then
prepare and activate a second generation.

| Roots | Prepare | Cycle-boundary switch |
| --- | --- | --- |
| 16 | 3.1037 ms | 12 µs |
| 1024 | 141.2133 ms | 28.1 µs |

The switch cost grows sublinearly with the root catalog and is independent of
the candidate's code size; preparation dominates. Candidate validation/load/
prepare runs on the cycle thread between scans in v1, so prepare time is
maintenance jitter, not cycle jitter. Fully off-thread preparation is post-v1
work; the switch itself is O(1) in the documented sense (bounded by the
process-image and direct-cell reconfiguration, not the module size).

Reproduce:

```sh
cargo test -p stc-runtime --test switch_latency --locked -- --ignored --nocapture
```

## Service-thread priority and maintenance jitter

Control-server and debug-server accept/handler threads run one OS priority class below the cycle
thread: `THREAD_PRIORITY_BELOW_NORMAL` on Windows and nice +10 per thread on Linux, best effort
(D-204). The cycle thread keeps the process default, and v1 does not pin CPU affinity.

The policy is an advisory mitigation for maintenance jitter, not a real-time guarantee. The
maintenance context is already recorded above: candidate load/validation/prepare runs on the
cycle thread between scans in v1, so the 3.1037 ms (16 roots) and 141.2133 ms (1024 roots)
prepare times are maintenance latency rather than cycle jitter, and an active observe session
adds 1022.11 ns/cycle to the measured debug form. Lowering the service threads lets monitoring
work yield to the scan; it does not bound prepare time, and a container or sandbox may reject the
change without affecting the session. These figures describe the host in Environment, not a
guarantee for another host or regime.

## Comparison with M6.5

M6.5 recorded (Windows local, same class of host): release
1119.43/1131.96 ns/cycle, hook-free 1287.57/1302.40, hook-bearing
1516.76/1537.81 (28.65 ns/hook), session 2772.00/2813.88, typed read
4252.52/4283.30. The M7 run shows the same shape and order of magnitude for
cycle and session costs; the 1.0.0-rc.1 version and M7 test changes invalidate
byte identity with M6.5 artifacts, and the typed-read figure is materially
higher on this run (34 µs versus 4.3 µs). That difference is recorded honestly
rather than treated as a regression gate; it should be re-measured with the
debugging instrumentation that the M6.5 run used before drawing conclusions.

## PREEMPT_RT host (WL-4)

Measured on the untuned x86-64 PREEMPT_RT host in [RT-Host.md](RT-Host.md) on 2026-10-29/30.
These figures describe one host in one regime. They are informational, are not comparable with
the Windows or AArch64 figures above, and are not a hard real-time guarantee.

| 1 ms medium workload | Execution p50 / p99 | Wake p50 / p99 | Cycles / skipped per 10 min |
| --- | --- | --- | --- |
| R0 quiet | 0.663–0.782 ms / 0.929–0.947 ms | 6.1–6.9 µs / 12.3–12.9 µs | 598,563–599,705 / 300–1,442 |
| R1 other-CPU mixed load | 2.580–2.681 ms / 3.429–3.476 ms | 20.9–23.7 µs / 47.9–96.8 µs | 177,942–188,901 / 411,169–422,166 |
| R2 same-CPU mixed load | 0.649–0.659 ms / 0.758–0.803 ms | 7.6–7.7 µs / 12.8–14.4 µs | 599,470–599,883 / 121–535 |

The R1 case is a heavy mixed-load stress case, not a light background load. The report covers the
10 ms secondary runs, the online-change/rollback cases, the control-cost A/B, and the noise
floor. An untuned R1 p50 of about 2.6 ms at a 1 ms target and osnoise maxima of 8.5–22.8 ms meet
the deferred tuned-profile trigger in [RT-Host.md](RT-Host.md) §6; the tuned profile was not
executed in v2.0.0.

## Coverage instrumentation overhead (WL-10)

Measured on the Windows development host with the `examples/coverage` suite (three tests) as a
full `stc test` process invocation. The comparison is informational, host-specific, and
non-gating (WL-10); it is dominated by test-build compilation and host noise, not by the
executed statements.

| Regime | Plain median | Coverage median | Difference |
| --- | --- | --- | --- |
| Five paired runs | 3619 ms | 3664 ms | +45 ms (about 1.2%) |
| Earlier three-run sample | 3331 ms | 3297 ms | -34 ms (noise) |

Reproduce:

```sh
cargo run -p stc-driver -- test --manifest-path examples/coverage/stc.toml --locked
cargo run -p stc-driver -- test --manifest-path examples/coverage/stc.toml --locked --coverage
```

Each invocation rebuilds every per-test artifact and links its host, so build and link time
dominates. Instrumentation adds one GEP/load/add/store sequence per executed site; the earlier
sample moved in the opposite direction, which shows the measurement is noise-dominated. No
overhead budget, threshold, or CI gate is defined.

## Policy

- A performance change is not a release blocker unless it breaks correctness,
  determinism, the O(1) activation contract, or a documented bound.
- If a future decision makes a budget a gate, it must fix the host, regime,
  checkpoint count, run count, and variance policy first (D-193).
- Do not compare figures across hosts, clang versions, build forms, or
  checkpoint counts without recording the difference.
- WL-4 PREEMPT_RT figures are informational and host-specific; they are not a CI gate and do
  not establish a hard real-time guarantee (D-193).
