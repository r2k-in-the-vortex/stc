# WL-14 working plan — macOS CI and Windows/macOS soft-real-time scheduling for tests

Status: **Phase 1 accepted 2026-11-08; Phase 2 accepted 2026-11-08; Phase 3 accepted 2026-11-08.**
Branch: `WL-14`, created from `v3.0.0-planning` at `2388d1a`.

Owner clarification 2026-11-08: cross-target object emission and runtime-side linking for
constrained targets (future microcontroller targets; no full clang or `stc` compiler on the
runtime; review the cold-download and online-change artifact flow) is **not WL-14 scope**. It is
recorded as WL-19 in `FutureWishlist.md`.
Date: 2026-11-07.
Profile: `stc-2013-oop-utf8-v1`.

This is a temporary working file. It is updated at the end of every phase and deleted after the
job is done; the durable record goes into `v3.0.0-plan.md`, `FutureWishlist.md`, and the
specification tree.

## 1. Outcome

1. Both macOS architectures (ARM64 and x86-64) run a meaningful test suite on GitHub-hosted
   runners. macOS stays **test-only**: no supported-host claim, no hard-real-time or safety claim.
   1.1. macOS release packaging is **not** part of WL-14. It is planned for the v3 release effort
   after the v3 plan is complete, so the release matrix can add macOS packages deliberately. WL-14
   leaves release packaging and the supported-host claim unchanged.
2. The debug and frame suites also run on macOS. Mach-O DWARF extraction is implemented as a
   separate stage rather than excluded:
   - `stc build` writes the stopped-frame sidecar on macOS x86-64 and ARM64.
   - The debug/frame targets are added to the macOS CI suite.
3. `stc run --rt` becomes the opt-in test/benchmark scheduling mode on Windows and macOS:
   - Windows: `THREAD_PRIORITY_TIME_CRITICAL`, plus MMCSS (`AvSetMmThreadCharacteristicsW`)
     when available, with explicit revert.
   - macOS: Mach `THREAD_TIME_CONSTRAINT_POLICY`, with a QoS fallback, with explicit revert.
   - Both use absolute-deadline pacing and the existing `stc-rt-metrics/1` JSONL.
   - The default run loop and the production scheduler are unchanged.
   - CLI help and documentation explain the behavior **per host**, not one generic sentence.
4. A portable measurement script and a non-gating `workflow_dispatch` measurement workflow compare
   baseline against `--rt` on `windows-2025`, `macos-15` (ARM64), and `macos-15-intel` (x86-64).
   Results are recorded with host, OS, build form, metric, and sample.
5. Documentation states the macOS test-only status, the measurement limits, and the absence of a
   hard-real-time or safety claim.
6. PR targets `v3.0.0-planning`; merging stays manual.

## 2. Verified starting facts

- The branch `WL-14` exists and is clean; `v3.0.0-planning` is at `2388d1a`.
- The repository is public (`r2k-in-the-vortex/stc`), so standard GitHub-hosted runners are free
  for this project. This removes the CI-budget question from the original WL-14 outline.
- Current runner images from the `actions/runner-images` manifest (checked 2026-11-07):
  - `macos-15` = macOS 15 Arm64.
  - `macos-15-intel` = macOS 15 x86-64.
  - `macos-14` was retired from GitHub Actions on 2026-11-02 and must not be used.
- The workspace already anticipates macOS in several places:
  - `crates/stc-driver/src/toolchain.rs` returns no extra link arguments on macOS.
  - `crates/stc-driver/src/build.rs` names `lib<name>.dylib` / `libstc_runtime.dylib` on macOS.
  - `crates/stc-driver/tests/common/mod.rs` names `libstc_runtime.dylib` on macOS.
  - `crates/stc-runtime/src/transport.rs` handles the macOS `nfds_t`.
  - `crates/stc-driver/tests/cross_target.rs` already covers `x86_64-apple-darwin` and
    `aarch64-apple-darwin` object emission.
- Feasibility check performed on this branch (temporary edit, reverted):
  - Extending only the `trap_boundary.rs` target list makes
    `cargo check --workspace --all-targets` pass for both `aarch64-apple-darwin` and
    `x86_64-apple-darwin`.
  - This is a compile check only; it does not assemble the global assembly or link a launcher.
    Phase 1 proves the real build and test path on the macOS runners.
- The known gaps found by inspection:
  - `crates/stc-runtime/src/trap_boundary.rs` has a `compile_error!` for macOS on both
    architectures.
  - `crates/stc-codegen/src/host.rs` falls back to a Linux triple on macOS.
  - `crates/stc-runtime/src/lib.rs` (`host_platform`) falls back to a Linux triple on macOS;
    `crates/stc-runtime/src/catalog.rs` (`HostPlatform::current`) rejects macOS.
  - `crates/stc-driver/tests/cli_project.rs` has its own host-triple helper that falls back to
    Linux.
  - `crates/stc-driver/src/frames.rs` supports only Windows MSVC CodeView (`llvm-readobj`) and
    Linux DWARF (`readelf`). **Phase 2 implements the Mach-O DWARF path**; it is not excluded.
  - `crates/stc-runtime/src/stop.rs` enables the AArch64 service-13 trampoline only on Linux; the
    fallback is used on macOS ARM64. Phase 2 enables the macOS AArch64 hook if the debug tests
    need it.
  - `crates/stc-runtime/src/rt.rs` is Linux-only; Windows and macOS return
    "`--rt` is only supported on Linux".

## 3. Scope

### In scope

- macOS ARM64/x86-64 CI jobs on the selected functional, project, execution, and I/O tests;
  then the debug/frame tests after Mach-O extraction is implemented.
- Mach-O DWARF frame-sidecar extraction for macOS x86-64 and ARM64.
- The opt-in `--rt` test/benchmark scheduling mode on Windows and macOS, with per-host help and
  documentation.
- Measurements of cycle timing, wake lateness, and skipped periods, baseline versus `--rt`.
- Documentation of test-only macOS and of the limits of soft-real-time scheduling on Windows and
  macOS.
- The existing three supported native hosts and the release-package matrix are unchanged.

### Out of scope (non-goals)

- macOS as a supported production host or a macOS support claim.
- macOS release packages; deferred to the v3 release effort (1.1).
- A hard-real-time or safety claim on any platform.
- A production scheduler change, a default priority change, or a new real-time budget.
- WL-12 sub-millisecond wake-jitter work, the deferred tuned boot profile, or EtherCAT changes.
- Changing `stc-debug`, the GOT, the language profile, or release packaging.

## 4. Decisions requested from the owner

1. **Extend `--rt` instead of adding a second flag.** Approved in review.
   - Keep `--rt` as the single opt-in test/benchmark switch. On Windows/macOS it selects the
     platform's soft scheduling facility; on Linux its FIFO behavior is unchanged.
   - `--rt-cpu` and `--rt-priority` stay Linux-only and return a clear error elsewhere.
   - CLI help and the user guide must describe the actual behavior on the current host (Linux:
     `SCHED_FIFO` and optional CPU pinning; Windows: time-critical priority + MMCSS; macOS: Mach
     time-constraint policy + QoS fallback).
2. **Implement Mach-O DWARF extraction; do not exclude the debug/frame tests.** Agreed in review.
   - This becomes Phase 2, before the scheduling work.
   - The macOS suite adds the debug/frame targets after the sidecar path is green on both
     architectures.
   - No test is silently skipped. Any remaining exclusion is explicit and justified.
3. **No numeric pass/fail threshold for the scheduling mode.** This is the existing policy, not a
   new standard:
   - D-193 says performance numbers are informational, not CI gates.
   - `Performance.md` is marked "Informational; not a CI gate (D-193)".
   - `RT-Host.md`/`RT-Runbook.md` (WL-4) are explicitly not CI gates; the strict
     `missed_deadline` counter is not treated as a hard-miss count.
   - Windows timing tests (`performance`, `debug_overhead`) are `#[ignore]`d and informational.
   - A numeric threshold is only meaningful with a fixed host, OS image, build form, sample size,
     and load regime. GitHub-hosted runners are shared VMs, so a fixed number would be flaky or
     misleading. The project gates on correctness, determinism, and documented behavior.
   - If the owner wants a threshold anyway, the plan can add a **comparative** rule on the same
     runner (for example, `--rt` p99 wake lateness must not be worse than baseline), with a fixed
     repetition count and image. That would be a separate owner decision and is not assumed here.
4. **Runner images.** Approved in review: pin `macos-15` (ARM64) and `macos-15-intel` (x86-64).
   Do not use `macos-latest` or `macos-14`.

## 5. Phases

The owner rule applies: **do not start the next phase until the previous phase's CI run is green.
If CI fails twice in a row, stop and ask the owner.** Each phase ends with a commit and a
`workflow_dispatch` CI run on `WL-14`.

### Phase 1 — macOS portability and core CI jobs

**Goal.** Both macOS architectures build the workspace and pass the functional/project/execution/
I/O suite; the aggregate `ci` gate requires the new jobs.

**Work.**

1. Extend the trap boundary:
   - `crates/stc-runtime/src/trap_boundary.rs`: allow `x86_64` macOS and `aarch64` macOS in the
     target list. The existing System V x86-64 and AAPCS64 assembly blocks are reused; verify the
     Mach-O symbol spelling (`_setjmp`, `longjmp`) during the real build.
   - Do **not** enable the AArch64 service-13 trampoline on macOS in this phase; Phase 2 does that
     with the debug suite.
2. Add host triples:
   - `crates/stc-codegen/src/host.rs`: `x86_64-apple-darwin` / `aarch64-apple-darwin`.
   - `crates/stc-runtime/src/lib.rs::host_platform`: the same triples.
   - `crates/stc-runtime/src/catalog.rs::HostPlatform::current`: accept the same triples.
   - `crates/stc-driver/tests/cli_project.rs::host_triple`: the test helper follows the host.
3. Stabilize macOS linking and execution:
   - Run the selected suite on the runners first; if Rust `staticlib`/`cdylib` linking needs
     Darwin arguments, add them in `crates/stc-driver/src/toolchain.rs` (for example the
     framework/library set the runner requires) rather than in tests.
   - Do not change the Windows or Linux link arguments.
4. Add the reusable suite:
   - `crates/xtask/src/main.rs`: add `"macos"` to `suite_targets` and to the usage/unknown-suite
     text. Initial targets:
     - `stc-driver`: `cli`, `cli_project`, `conformance`, `cross_target`, `examples`, `execution`,
       `exact_vectors_cli`, `fmt_cli`, `hir_dump`, `manifest_hardening`, `modbus_e2e`,
       `module_cli`, `online_cli`, `project_cli`, `projects`, `run_cli`, `state_io_cli`,
       `test_cli`, `ui`.
     - `stc-runtime`: `descriptor_lifecycle`, `embedding_run`, `host_lifecycle`, `image_remap`,
       `modbus_online_change`, `scheduler`.
   - Review the other targets (debug/frame, remote/serve, coverage, LSP) and add every one that
     passes; Phase 2 adds the debug/frame targets once Mach-O extraction exists. The only
     permanent exclusion planned is `package`, because macOS release packaging is deferred to the
     v3 release (1.1).
5. Add the CI job in `.github/workflows/ci.yml`:
   - Job `macos`, name `macOS tests (${{ matrix.os }})`.
   - `needs: detect`, `if: needs.detect.outputs.rust == 'true'`.
   - Matrix `os: [macos-15, macos-15-intel]`, `fail-fast: false`, `timeout-minutes: 40`.
   - `env: RUSTFLAGS: "-D warnings"`, `STC_REQUIRE_CLANG: "1"`.
   - Steps: checkout, print `clang`/`rustc` versions, `Swatinem/rust-cache@v2`,
     `cargo test --workspace --lib --bins --locked`,
     `cargo xtask test --suite macos --locked`.
   - Add `macos` to the `ci` aggregate job's `needs` list.
   - Add a comment that macOS is test-only and that the selected suite is explicit.
6. Keep `docs`, `fmt`, `clippy`, `coverage`, `online`, `debug`, `package`, and the rest unchanged.

**Exit gate.**

- A `workflow_dispatch` CI run on `WL-14` is green, including both macOS matrix entries and the
  aggregate `ci` job.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo xtask host-audit --check`, `cargo xtask traceability --check`, `python3
  tools/ci/check_docs.py`, and markdown lint are green in that run.
- No macOS package is produced and no supported-host document is changed to claim macOS support.

### Phase 2 — Mach-O frame extraction and the macOS debug suite

**Goal.** `stc build` writes a valid, deterministic stopped-frame sidecar on macOS x86-64 and
ARM64; the debug and frame suites run on macOS.

**Work.**

1. Add the Mach-O extraction format:
   - `crates/stc-driver/src/frames.rs`: add `Format::MachDwarf`, selected when the target triple
     contains `apple-darwin`.
   - `tool_for`: resolve `llvm-dwarfdump` from PATH first (the CI job puts the Xcode toolchain
     `usr/bin` on PATH), then `xcrun --find llvm-dwarfdump`, then the clang sibling directory. If
     no reader is available, keep the existing "omit the sidecar" behavior, but the CI job asserts
     that the reader exists so the tests cannot silently skip.
   - Run `llvm-dwarfdump --debug-info <object>` for DIEs and `llvm-dwarfdump --debug-line <object>`
     for the line table.
2. Parse the LLVM/Mach-O text:
   - `parse_mach_dwarf_info`: DIE depth from the LLVM indentation; `DW_AT_linkage_name`,
     `DW_AT_low_pc`, `DW_AT_high_pc`, `DW_AT_frame_base` (`DW_OP_reg6 RBP` on x86-64,
     `DW_OP_reg29 X29` on AArch64), `DW_AT_location` (`DW_OP_fbreg -N`), and `DW_AT_name`.
   - `parse_mach_dwarf_lines`: the `file_names` table plus the address/line/file-index rows;
     ignore `end_sequence`; normalize the source path with the existing rules.
   - Normalize Mach-O linkage names by removing one leading `_` before matching the front-end
     symbol hints; keep the sidecar symbol unprefixed so the runtime manifest comparison is
     unchanged.
3. Resolve addresses:
   - Prefer the object when `DW_AT_low_pc` values are distinct and the line rows map cleanly.
   - If relocatable-object addresses are ambiguous, extract from the linked Mach-O dylib after
     `link_module` (debug builds keep DWARF). The sidecar remains function-relative (`low_pc` 0,
     `high_pc` code size), so the runtime contract does not change.
4. Enable the AArch64 service-13 hook on macOS if the debug tests need frame rules:
   - `crates/stc-runtime/src/stop.rs`: extend the AArch64 cfg to macOS, keep the `bti c` landing
     pad, and verify the AAPCS64 prologue against the pinned runner.
   - If the runner or the generated prologue makes the hook unsafe, keep the fallback and record
     the exact failing test and the documented limitation; do not fake frame data.
5. Tests and fixtures:
   - Add unit tests in `frames.rs` with captured `llvm-dwarfdump` text for Mach-O x86-64 and
     AArch64, including file-name tables, fbreg locals, and line rows.
   - `crates/stc-driver/tests/frames.rs`: extend `extraction_tool_available()` for macOS
     (`llvm-dwarfdump`/`dwarfdump`) and keep the "tool exists but sidecar missing" assertion.
   - `crates/stc-codegen/tests/debug_info.rs`: use `llvm-dwarfdump` on macOS; update
     `tool_available` to find the Xcode toolchain.
   - Add the debug/frame targets to the `macos` suite:
     - `stc-runtime`: `debug_access`, `debug_write`, `debug_stop`, `debug_view`, `debug_protocol`.
     - `stc-driver`: `debug_cli`, `dap_cli`, `debug_release`, `frames`.
     - `stc-codegen`: `debug_info`, `checkpoints`, `frames`.
     - `stc-got`: `metadata_v2`.
6. Add the Xcode toolchain step to the macOS CI job:
   - Put `$(xcode-select -p)/Toolchains/XcodeDefault.xctoolchain/usr/bin` on `GITHUB_PATH` and
     assert `command -v llvm-dwarfdump` (or the chosen reader) before the tests.

**Exit gate.**

- Both macOS jobs green with the debug/frame targets enabled.
- The sidecar exists, is byte-deterministic, has normalized forward-slash paths, and supports a
  real stopped session with typed locals (`frames` test).
- `debug_info` and `checkpoints` pass on macOS with the pinned Xcode tool.
- No change to the Windows CodeView or Linux DWARF paths.

### Phase 3 — Opt-in scheduling mode on Windows and macOS

**Goal.** `stc run --rt` applies a documented soft scheduling facility on Windows and macOS, emits
the existing metrics, cleans up, and leaves the default run loop untouched.

**Work.**

1. `crates/stc-runtime/src/rt.rs`:
   - Split the platform module into `linux`, `windows`, `macos`, and `other`.
   - Linux: keep the current `SCHED_FIFO` + affinity + `clock_nanosleep` behavior unchanged.
   - Windows:
     - `enter`: reject `--rt-cpu` (Linux-only) and set the cycle thread to
       `THREAD_PRIORITY_TIME_CRITICAL`; attempt `AvSetMmThreadCharacteristicsW("Audio")` and keep
       the handle for cleanup. If MMCSS fails, continue with the priority alone and record the
       fallback.
     - `now_ns`: `QueryPerformanceCounter` / `QueryPerformanceFrequency`.
     - `sleep_until`: a high-resolution waitable timer
       (`CreateWaitableTimerExW` with `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`,
       `SetWaitableTimer`, `WaitForSingleObject`); return `Interrupted` on an aborted wait.
     - `background`: below-normal priority for the metrics writer thread.
     - `Drop`: `AvRevertMmThreadCharacteristics`, restore the previous thread priority, and close
       the timer handle.
   - macOS:
     - `enter`: reject `--rt-cpu`; capture the current QoS, then apply
       `THREAD_TIME_CONSTRAINT_POLICY` with a period/computation/constraint derived from the cycle
       time (for example period 2×, computation ½×, constraint 1×, preemptible). If the policy is
       rejected, fall back to `QOS_CLASS_USER_INTERACTIVE` and record the fallback.
     - `now_ns`: `clock_gettime(CLOCK_MONOTONIC)`.
     - `sleep_until`: `mach_wait_until` with the `mach_timebase_info` conversion; `KERN_ABORTED`
       is `Interrupted`.
     - `background`: standard policy plus nice +10 for the writer thread.
     - `Drop`: restore the captured QoS and set `THREAD_STANDARD_POLICY`.
   - `fault_counts`: remains Linux-only and returns `None` elsewhere; the JSONL fields stay
     `null`.
2. `crates/stc-runtime/src/priority.rs`: add the checked/lower helpers the Windows/macOS writer
   cleanup needs; keep the existing service-thread policy.
3. CLI and launcher help:
   - `crates/stc-driver/src/cli.rs` and `crates/stc-runtime/src/lib.rs`: `--rt` is accepted on
     all three hosts; `--rt-priority` and `--rt-cpu` return a clear "only supported on Linux"
     error on Windows/macOS.
   - Help text is host-specific and is tested:
     - Linux: "`--rt` sets the cycle thread to `SCHED_FIFO` (default priority 90) and uses
       absolute monotonic sleeps; `--rt-cpu` pins it to one CPU."
     - Windows: "`--rt` raises the cycle thread to `THREAD_PRIORITY_TIME_CRITICAL` and registers
       it with MMCSS when available; `--rt-cpu`/`--rt-priority` are Linux-only."
     - macOS: "`--rt` applies a Mach `THREAD_TIME_CONSTRAINT_POLICY`, falling back to an
       interactive QoS class; `--rt-cpu`/`--rt-priority` are Linux-only."
   - `stc run --help` and the generated launcher's `--help` both show the current host's text.
4. Metrics:
   - Keep `stc-rt-metrics/1` records compatible. If a `sched` field is added to the header, make
     it optional and update `tools/rt/campaign.py` and its tests only if necessary; existing
     parsers must keep reading older records.
5. Tests:
   - Add host-run tests in `rt.rs`/`priority.rs` for `enter` (priority/QoS applied), `now_ns`
     monotonicity, `sleep_until` not returning early, and cleanup. These are unprivileged on
     Windows and macOS.
   - Keep the Linux `SCHED_FIFO` test `#[ignore]`d and privileged as it is.
   - Add a runtime test that writes metrics through the non-Linux path and verifies the summary.
   - Add help tests that assert the host-specific wording.
6. Documentation (phase-local):
   - `specification/Architecture/Runtime-and-Execution.md` §2.3.3: describe the opt-in
     test/benchmark mode per host and the unchanged default.
   - `USER-GUIDE.md`: `--rt` behavior per host and the test-only wording.
   - `KNOWN-LIMITATIONS.md`: Windows/macOS scheduling is advisory, not a hard-real-time
     guarantee; MMCSS/QoS/time-constraint behavior depends on the host and the runner.
   - Decisions: add D-248 (macOS test-only CI and the Mach-O frame path) and D-249 (opt-in
     scheduling mode) after the implementation is verified; numbering is provisional until the
     docs phase.

**Exit gate.**

- CI green on Windows, Linux, and macOS.
- A Windows run and a macOS run produce `stc-rt-metrics/1` output with the scheduling mode and
  exit cleanly; the same runs without `--rt` keep the default behavior.
- `--rt` help on each host names that host's facility and the Linux-only options.
- No production default priority, sleep path, or support claim changed.

### Phase 4 — Measurement campaign and evidence

**Goal.** Baseline versus `--rt` numbers on Windows and both macOS architectures, recorded with
host, OS, build form, metric, and sample, without adding a timing gate.

**Work.**

1. Add `tools/rt/sched_compare.py` (Python 3 standard library only):
   - Inputs: launcher path, cycle time, cycles per run, run count, mode (`baseline`/`rt`), output
     directory.
   - Runs the launcher directly with `--cycle-time`, `--cycles`, and (for the mode) `--rt`
     `--rt-metrics <file>`.
   - Discards a fixed warm-up window (for example the first 2,000 cycles), then computes
     min/p50/p90/p99/p99.9/max for `execution_ns` and `wake_lateness_ns`, plus
     `missed_deadlines`, `skipped_periods`, and `dropped_records`.
   - Records host/OS/CPU/build-form metadata and the launcher build ID.
   - Writes a stable JSON summary and a Markdown table.
   - Adds unit tests under `tools/rt/tests/` in the existing `unittest` style; no root, no real
     control socket.
2. Add `.github/workflows/sched-measure.yml`:
   - `workflow_dispatch` inputs for cycles, runs, and cycle time; no push/PR trigger.
   - Matrix `windows-2025`, `macos-15`, `macos-15-intel`.
   - Builds `stc` and the runtime, builds `tests/rt/bench` with `--emit=exe,module,metadata`, and
     runs `sched_compare.py` for baseline and `--rt`.
   - Uploads the JSON, Markdown, and raw JSONL files as artifacts.
   - The workflow is informational; it is not in the `ci` gate.
3. Run the workflow on the `WL-14` branch and collect the numbers.
4. Record the results:
   - `specification/Verification/Performance.md`: new section with the host table, percentiles,
     and the comparison. Label the GitHub-hosted runners and their noise.
   - `specification/Verification/RT-Runbook.md`: the portable procedure for Windows/macOS and the
     artifact/checksum steps.
   - `specification/Verification/RT-Host.md`: a short cross-reference section that keeps the
     PREEMPT_RT host report intact and points to `Performance.md`.
   - `KNOWN-LIMITATIONS.md`: the measured limits and the fact that no threshold is defined.
5. If the macOS time-constraint policy is rejected by the runner, record the exact error, the QoS
   fallback, and the limitation. The exit criteria allow a documented platform restriction; they
   do not require the facility to be available.

**Exit gate.**

- CI green on `WL-14` (the normal matrix, not the measurement workflow).
- The `sched-measure` run completes on all three runner labels and its artifacts are available.
- The recorded tables include host, OS, build form, metric, and sample, and compare baseline with
  `--rt`.
- No timing threshold or timing gate was added to `ci.yml`.

### Phase 5 — Documentation, manual check, and PR

**Goal.** Durable documentation, owner manual approval, clean working tree, and a PR to
`v3.0.0-planning`.

**Work.**

1. Final documentation:
   - `KNOWN-LIMITATIONS.md`: macOS test-only; the macOS suite and the Mach-O frame path;
     Windows/macOS scheduling advisory; MMCSS/time-constraint/QoS limits; no hard-real-time or
     safety claim.
   - `USER-GUIDE.md`: support matrix remains the three native hosts; macOS is test-only; `--rt`
     per-host behavior.
   - `specification/Architecture/Runtime-and-Execution.md`: final wording for the opt-in mode.
   - `specification/Architecture/Architecture.md`: macOS test-only note, no support claim.
   - `specification/Verification/RT-Host.md`, `RT-Runbook.md`, `Performance.md`: final evidence
     sections.
   - `specification/Decisions.md`: D-248 (macOS test-only CI and the Mach-O frame path) and D-249
     (opt-in Windows/macOS scheduling).
   - `v3.0.0-plan.md`: WL-14 status, checklist, and change log.
   - `FutureWishlist.md`: WL-14 status and the deferred macOS release-package item.
2. Manual verification walkthrough for the owner:
   - CI links for the macOS jobs on both architectures and the aggregate `ci` gate, including
     the debug/frame targets.
   - The `sched-measure` artifacts and the recorded table.
   - A local Windows manual check: build `stc`, run the bench baseline and with `--rt`, inspect the
     JSONL header/summary, confirm default behavior is unchanged, and confirm `--rt-cpu` /
     `--rt-priority` errors on Windows.
   - A review of the macOS job output to confirm the selected suite ran and the exclusions are
     explicit.
3. Owner approval gate. Do not mark WL-14 complete or open the PR before approval.
4. After approval:
   - Update the WL-14 status in `v3.0.0-plan.md` and `FutureWishlist.md`.
   - Delete `WL-14-Plan.md` (this file) from the branch.
   - Commit, push `WL-14`, run the final CI on `WL-14`, and open the PR against
     `v3.0.0-planning` (not `main`). Leave merging manual.
   - After the owner merges, delete the local and remote `WL-14` branches and record the merge in
     the planning documents, as with WL-1.2.

**Exit gate.**

- Manual check approved.
- Final CI green on `WL-14`.
- PR open against `v3.0.0-planning`; merge left to the owner.
- No temporary planning file remains in the PR.

## 6. Verification map

| Claim | Evidence |
| --- | --- |
| macOS ARM64 build and selected tests | `macos-15` CI job, `cargo test --workspace --lib --bins`, `cargo xtask test --suite macos` |
| macOS x86-64 build and selected tests | `macos-15-intel` CI job, same commands |
| macOS debug/frame support | Mach-O sidecar unit fixtures; `frames`/`debug_*`/`debug_info`/`checkpoints` targets on both macOS jobs |
| macOS is test-only | CI matrix has no macOS package job; `KNOWN-LIMITATIONS.md` and `USER-GUIDE.md` state it |
| Trap boundary on macOS | `cargo test` execution: trap fixtures in the selected suites; the `a_trap_returns_to_the_boundary` unit test |
| Host identity on macOS | host-triple tests in `cli_project`; runtime `HostPlatform` use in the selected runtime tests |
| Default scheduler unchanged | code inspection plus a test run without `--rt`; existing Linux RT tests unchanged |
| Windows scheduling applied | `rt.rs` unit test reads `GetThreadPriority`; metrics header/records from a Windows run |
| macOS scheduling applied | `rt.rs` unit test reads the applied policy/QoS or the documented fallback; metrics records |
| Per-host help | help tests assert the host-specific wording on each host |
| Cleanup | tests assert the prior priority/QoS is restored after `Drop` |
| Measurements | `sched-measure` artifacts and `Performance.md` tables |
| No timing gate | `ci.yml` has no `sched-measure` job; `Performance.md` states D-193 still applies |

## 7. Risks and fallbacks

| Risk | Mitigation / fallback |
| --- | --- |
| macOS linking of the Rust staticlib/cdylib needs extra Darwin arguments | Add them in `toolchain.rs`; keep the same test suite and rerun. Do not special-case tests. |
| `llvm-dwarfdump` output differs across Xcode versions | Pin the runner image, capture the unit fixtures from that image, and keep the parser dialect explicit. |
| Xcode does not expose `llvm-dwarfdump` on PATH | Put the Xcode toolchain `usr/bin` on `GITHUB_PATH`; if it is absent, use Apple `dwarfdump` with a separate parser, or record the limitation with evidence. |
| Mach-O linkage names carry a leading `_` | Normalize one leading underscore before matching hints; keep the sidecar symbol unprefixed. |
| Relocatable Mach-O addresses are ambiguous | Extract from the linked dylib after `link_module`; the sidecar stays function-relative. |
| macOS AArch64 stop hook needs BTI/frame rules | Extend the `stop.rs` cfg and verify against the pinned runner; if unsafe, keep the fallback and document the failing test. |
| macOS runner rejects `THREAD_TIME_CONSTRAINT_POLICY` | Fall back to `QOS_CLASS_USER_INTERACTIVE`, record the error and the fallback, and document the limitation. |
| Windows high-resolution timer floor (~0.5 ms) | Use 1 ms and 10 ms runs; document the timer floor and treat 1 ms results as comparative, not as a bound. |
| GitHub-hosted runner noise, especially macOS VMs | Comparative baseline/mode runs on the same host and run; record the image and host; no threshold. |
| A selected test has an unstated Linux assumption | Fix it if it is a portability bug; otherwise exclude it explicitly with a reason in the plan and `KNOWN-LIMITATIONS.md`. Never use a silent test filter. |
| `macos-15-intel` label changes | Verify against the runner-images manifest; pin the image in the job. |
| Coverage gate | Windows/macOS branches are not compiled in the Ubuntu coverage job; Linux changes stay test-covered. |
| `package` target | Deferred with macOS release packaging (1.1); not part of WL-14. |
| Two consecutive CI failures | Stop and ask the owner, as required. |

## 8. Status log

| Phase | Status | Commit | CI run | Notes |
| --- | --- | --- | --- | --- |
| 0. Plan | Approved 2026-11-07 | — | — | Decisions 1–4 approved; existing no-threshold policy kept |
| 1. macOS portability and CI | **Complete and accepted 2026-11-08** | `e5a6a99` | [42460413226](https://github.com/r2k-in-the-vortex/stc/actions/runs/42460413226) | Both macOS jobs, coverage, and the aggregate gate green. The Phase 1 report was deleted after acceptance. |
| 2. Mach-O frame extraction | **Complete and accepted 2026-11-08** | `3a86b4e`, `e8bbcdc` | [42506252319](https://github.com/r2k-in-the-vortex/stc/actions/runs/42506252319) | Both macOS jobs, coverage, and the aggregate gate green, including the full `macos` suite through the debug/frame targets. The branch also carries `5477ec2`, the separate cross-platform E004 close fix outside WL-14 scope, included for the later merge. |
| 3. Scheduling mode | **Complete and accepted 2026-11-08** | `9526b92`, `c4a5345`, `3733082`, `ce2d4bc` | [42516443734](https://github.com/r2k-in-the-vortex/stc/actions/runs/42516443734) | Both macOS jobs, coverage, the aggregate gate, and the Windows/Linux jobs green on the final acceptance run. Host tests cover Windows priority/MMCSS/sleep/Drop restore and macOS time-constraint/QoS fallback, monotonic sleeps, and Drop restore; `--rt-cpu`/`--rt-priority` reject on non-Linux hosts; the full-path Windows/macOS `run_cli` metrics test passes. Windows observed `sched:"mmcss"` with final priority 13; macOS observed `sched:"time_constraint"` with the QoS fallback covered by code/tests (not exercised on the runners). |
| 4. Measurements | Not started | — | — | |
| 5. Docs, manual check, PR | Not started | — | — | |

## 9. CI procedure

Each phase closes with:

```sh
cd /c/repos/stc3
gh workflow run ci.yml --ref WL-14
gh run list --workflow=ci.yml --branch WL-14 --limit 1
```

The phase is complete only when the run is green and the phase's status row is updated. The next
phase's first commit is made only after that. If a phase's CI fails twice in a row, work stops and
the owner is asked before a third attempt.
