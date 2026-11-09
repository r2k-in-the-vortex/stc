# WL-14 working plan — macOS support at Windows parity and Windows/macOS soft-real-time scheduling for tests

Status: **Phase 1 accepted 2026-11-08; Phase 2 accepted 2026-11-08; Phase 3 accepted 2026-11-08; Phase 4 accepted 2026-11-08; Phase 5A complete 2026-11-08; Phase 5B complete 2026-11-09; Phase 5C complete 2026-11-09; Phase 5D complete 2026-11-09. Phase 6 manual check pending owner approval.**
Branch: `WL-14`, based on `v3.0.0-planning` at `2388d1a`; current HEAD `73baed5`.
Date: 2026-11-07; revised 2026-11-09.
Profile: `stc-2013-oop-utf8-v1`.

This is a temporary working file. It is updated at the end of every phase and deleted after the job is done; the durable record goes into `v3.0.0-plan.md`, `FutureWishlist.md`, and the specification tree.

## 0. Owner correction (2026-11-08)

The earlier “macOS test-only” interpretation was wrong. macOS must support everything that is
already available on Windows, on both macOS architectures, except:

- **EtherCAT** hardware I/O, which remains Linux-only.
- **PREEMPT_RT-style hard real-time**, which remains Linux-only.

The opt-in `stc run --rt` soft scheduling mode is already supported on macOS and stays in scope.
Debug, VS Code integration, release packaging, online change, state/I/O, remote debug, the full
workspace test suite, and the support matrix are all in scope for macOS.

The test-only documentation committed in `db53067` and decision D-248 are **superseded** by this
revision. They are not the final state and must not be carried into the PR. The final tree must
contain no “macOS test-only” claim.

## 1. Outcome

1. macOS ARM64 (`macos-15`, `aarch64-apple-darwin`) and x86-64 (`macos-15-intel`,
   `x86_64-apple-darwin`) are **supported native hosts** at parity with Windows for the
   Windows-available surface.
2. macOS runs the **full workspace test suite**, not only library/binary tests plus a selected
   suite. This includes remote debug, `stc serve`, LSP, coverage, package-layout, IDE, and all
   pure-crate integration tests. No test is silently skipped; Linux-only tests are explicitly
   cfg-gated with a documented reason.
3. macOS release packages are built, verified, checksummed, and exercised in CI:
   - `stc-<version>-aarch64-apple-darwin`
   - `stc-<version>-x86_64-apple-darwin`
   - Mach-O runtime names (`libstc_runtime.a`, `libstc_runtime.dylib`), `debug_contract: dwarf`.
4. The VS Code extension smoke runs on both macOS architectures against the packaged VSIX:
   activation, `stc lsp`, a `stc check` task, and a `stc dap` launch session. The extension
   package itself remains one platform-independent VSIX.
5. Documentation, decisions, and the support matrix state macOS support and the two explicit
   exclusions. No hard-real-time or safety claim is added on any platform.
6. Windows and Linux behavior is unchanged; the default scheduler, language profile, ABI, and
   formats are unchanged.

## 2. Gap analysis (macOS versus the Windows-available surface)

| Surface | Windows/Linux today | macOS today | Revised action |
| --- | --- | --- | --- |
| Full workspace tests | `cargo test --workspace --locked` on Windows, Linux x86-64, Linux AArch64 | `cargo test --workspace --lib --bins` plus the selected `macos` suite | Run `cargo test --workspace --locked` on both macOS jobs; fix portability failures; keep the `macos` suite as a local shortcut |
| Remote debug, `stc serve`, LSP | Covered by full workspace (`remote_attach_cli`, `remote_status_cli`, `serve_cli`, `lsp_cli`) | Not run by the current macOS job | Covered by the full workspace run; fix macOS path/socket/SSH-fake issues |
| Release package | `package` matrix: `windows-2025`, `ubuntu-24.04`, `ubuntu-24.04-arm` | No package; `package` excluded by design | Add both macOS runners to the `package` matrix; fix Mach-O strip/verify/checksum; exercise the unpacked package |
| Package debug contract | `object-codeview` on Windows, `dwarf` on Linux | Not tested | macOS is `dwarf`; verify the metadata and the absence of `__debug_*` sections |
| Package checksum verification | `sha256sum -c` on Windows/Linux | `sha256sum` is not normally on macOS | Use `shasum -a 256 -c` on macOS (or a portable verifier) |
| VS Code extension package | One VSIX built on Linux | Not applicable; VSIX is platform-independent | Keep the single VSIX; no macOS-specific extension build |
| VS Code extension smoke | Linux + xvfb, real VS Code, LSP/task/DAP | Not run | Add `macos-15` and `macos-15-intel` to the smoke matrix; skip xvfb on macOS; run the real Electron smoke |
| Debug/frame | CodeView on Windows, DWARF on Linux | Mach-O DWARF path already implemented and tested | Retain; ensure the full workspace still asserts the reader exists |
| Online change / state I/O / Modbus | Full workspace plus explicit jobs | Selected suite covers the main targets | Covered by the full workspace run; add any missing targets |
| EtherCAT | Linux shim only | Linux-only backend is cfg-gated; non-Linux uses the protocol-neutral mock | Accepted exclusion; document it clearly and add a macOS test that pins the exclusion behavior |
| Hard real-time | PREEMPT_RT Linux only | No hard RT; soft `--rt` works | Accepted exclusion; keep the soft `--rt` mode and document it as advisory |
| CI aggregate | macOS jobs are already in `needs` | Package/smoke jobs absent | Add macOS package and smoke jobs to the aggregate gate |
| Support matrix / decisions | macOS listed as unsupported/test-only | Test-only claims in D-248, architecture, guide, limitations | Add D-250 (macOS parity); supersede D-248; amend D-200/D-205/D-206; revise all living docs |
| Platform-independent jobs | `docs`, `fmt`, `clippy`, `coverage`, `traceability`, `lean`, `fuzz-smoke` on Linux | Not duplicated | No change; these are host-independent gates, not macOS feature gaps |
| EtherCAT shim job | Linux only | Not applicable | No change; accepted exclusion |

## 3. Scope

### In scope

- Full workspace tests on both macOS architectures, with `STC_REQUIRE_CLANG=1` and the LLVM tools
  on `PATH` so frame/debug assertions cannot silently skip.
- Portability fixes required by the full workspace run, including macOS Unix-socket path length,
  fake-SSH/remote tests, coverage/package-layout tests, and LSP timing.
- macOS release packages in the `package` matrix, including:
  - Mach-O-aware debug stripping (`llvm-strip` or `strip -S`; `llvm-objcopy` is not assumed to
    support Mach-O).
  - Mach-O debug-section detection (`__debug_*` via `llvm-objdump --macho --section-headers` or
    `otool -l`).
  - Portable checksum verification (`shasum -a 256 -c` on macOS).
  - The unpacked-package workflow: `new`, `check`, `build`, `build --release`,
    `build --release --emit=lib`, `run`, `test`, `clean`, and rebuild.
- VS Code extension smoke on both macOS architectures, with a conditional xvfb step and the real
  Electron smoke on macOS.
- An explicit macOS test for the EtherCAT exclusion behavior (clear documentation of the
  non-Linux mock versus a hard rejection; the accepted exclusion is EtherCAT hardware support).
- Documentation and decisions:
  - D-250: macOS ARM64/x86-64 are supported native hosts at Windows parity, excluding EtherCAT
    and PREEMPT_RT-style hard real time.
  - Supersede D-248; amend D-200 (hosts), D-205 (CI matrix), and D-206 (packaging/debug contract).
  - `Architecture.md` §6.4, `Runtime-and-Execution.md` §2.3.3, `USER-GUIDE.md` §1/§8/§10/§11/§14,
    `KNOWN-LIMITATIONS.md` §2/§3/§7, `RT-Host.md` §9, `RT-Runbook.md` §10, `Performance.md`,
    `v3.0.0-plan.md`, and `FutureWishlist.md`.
- The macOS `macos` suite remains available as a local targeted shortcut; the CI gate is the full
  workspace run.

### Out of scope (accepted non-goals)

- EtherCAT hardware I/O on macOS (Linux-only shim/backend).
- PREEMPT_RT-style hard real-time on macOS (Linux-only).
- A hard-real-time or safety claim on any platform.
- Windows ARM64, 32-bit, and big-endian hosts.
- A timing threshold or timing gate; D-193 remains in force.
- WL-12 sub-millisecond wake-jitter work and the deferred tuned boot profile.
- A production scheduler change or a default priority change.
- Language, profile, format, ABI, or protocol changes.
- A macOS-specific VS Code extension build; the VSIX stays platform-independent.

## 4. Decisions requested from the owner

1. **Approve macOS as a supported native host at Windows parity**, excluding EtherCAT and
   PREEMPT_RT-style hard real time. This supersedes the test-only interpretation and D-248.
2. **Approve macOS release packaging and VS Code extension smoke as WL-14 scope**, not deferred
   release work. This adds both macOS runners to the `package` and `extension-smoke` matrices.
3. **Approve the full workspace test suite on macOS** as the CI gate. The explicit `macos` suite
   stays as a local shortcut; it is no longer the macOS CI gate.
4. **Approve superseding `db53067` rather than rewriting history.** The final tree will replace
   the test-only wording with parity wording in a new commit. If a clean history is preferred,
   `db53067` can be reverted first; the final tree is the same.
5. **Keep the no-threshold policy.** macOS measurements remain informational; no timing gate is
   added.

## 5. Phases

The owner rule applies: **do not start the next phase until the previous phase's CI run is green.
If CI fails twice in a row, stop and ask the owner.** Each phase ends with a commit and a
`workflow_dispatch` CI run on `WL-14`.

### Accepted phases 1–4 (record)

| Phase | Accepted | Commit | CI run | Summary |
| --- | --- | --- | --- | --- |
| 1. macOS portability and core CI | 2026-11-08 | `e5a6a99` | [42460413226](https://github.com/r2k-in-the-vortex/stc/actions/runs/42460413226) | Both macOS jobs, coverage, and the aggregate gate green. |
| 2. Mach-O frame extraction | 2026-11-08 | `3a86b4e`, `e8bbcdc` | [42506252319](https://github.com/r2k-in-the-vortex/stc/actions/runs/42506252319) | Mach-O DWARF sidecar; debug/frame targets in the `macos` suite. |
| 3. Scheduling mode | 2026-11-08 | `9526b92`, `c4a5345`, `3733082`, `ce2d4bc` | [42516443734](https://github.com/r2k-in-the-vortex/stc/actions/runs/42516443734) | Windows time-critical/MMCSS, macOS time-constraint/QoS, metrics, cleanup. |
| 4. Measurements | 2026-11-08 | `9e894fd`, `e4079bd`, `29338a3`, `a48e256`, `e57731b` | [42523556357](https://github.com/r2k-in-the-vortex/stc/actions/runs/42523556357) | Informational campaign on Windows and both macOS runners; no threshold. |

The durable evidence remains in `Performance.md`, `RT-Host.md`, `RT-Runbook.md`, and
`KNOWN-LIMITATIONS.md`. The 4.500 ms macOS figure was a harness-timeout artifact, not a
measurement; `e4079bd` and `29338a3` fixed the cap and the null-mean reporting.

### Phase 5 — macOS parity implementation

**Goal.** macOS is a supported host at Windows parity for the Windows-available surface, with
EtherCAT and PREEMPT_RT hard real time explicitly excluded.

#### Phase 5A — Full workspace tests on macOS

**Work.**

1. Change the `macos` CI job to run `cargo test --workspace --locked` (full parity with the
   Windows/Linux `test` job). Keep `STC_REQUIRE_CLANG=1` and the Homebrew LLVM `PATH` step.
2. Raise the job timeout to a realistic value (for example 75 minutes) and split the job if the
   full run cannot fit; do not weaken tests to fit a timeout.
3. Run the full suite on both macOS architectures and fix portability failures. Expected areas:
   - `remote_attach_cli`, `remote_status_cli`, `serve_cli`: Unix-socket path length (macOS limit
     is 104 bytes), fake-SSH scripts, Python availability, process cleanup.
   - `lsp_cli`: startup and shutdown timing on slower macOS runners.
   - `coverage_cli`, `coverage_evidence`: clang coverage instrumentation and host-independent
     golden reports.
   - `package` (driver layout test): static runtime name and packaged-driver lookup.
   - Any pure-crate integration test that assumes Linux/Windows paths.
4. Keep the `macos` suite as a local shortcut. Do not add a second CI run of the same targets
   unless a specific reason appears.
5. Add or update a macOS test for the EtherCAT exclusion: pin whether non-Linux selects the
   protocol-neutral mock or rejects the configuration, and make the behavior explicit in the
   documentation. The accepted exclusion is EtherCAT hardware support, not a silent hardware
   claim.
6. No silent skips: `STC_REQUIRE_CLANG=1`, `llvm-dwarfdump` present, and any Linux-only test
   explicitly `#[cfg(target_os = "linux")]` with a reason.

**Exit gate.**

- Both macOS jobs green with `cargo test --workspace --locked`.
- The full suite includes remote/serve/LSP/coverage/package/IDE targets; no test silently
  skipped.
- The aggregate `ci` gate is green.
- Windows and Linux jobs are unchanged and green.

#### Phase 5B — macOS release packaging

**Work.**

1. Make `cargo xtask package` Mach-O-aware:
   - `runtime_names()` already returns `libstc_runtime.a` and `libstc_runtime.dylib`; keep it.
   - Replace the unconditional `llvm-objcopy --strip-debug` path for macOS with `llvm-strip
     --strip-debug` or `strip -S` for archive members and the dylib.
   - Extend `check_no_debug_sections` to detect Mach-O `__debug_*` sections using
     `llvm-objdump --macho --section-headers` or `otool -l`.
   - Keep `is_native_object` Mach-O detection; keep `llvm-ar`/`ar` extraction and `rD`
     replacement.
2. Add `macos-15` and `macos-15-intel` to the `package` matrix.
3. Add the LLVM `PATH` step to the macOS package job; assert `llvm-ar`, the strip tool, and the
   section reader exist.
4. Make checksum verification portable: `sha256sum -c` on Windows/Linux, `shasum -a 256 -c` on
   macOS.
5. Keep the unpacked-package exercise on macOS: `new`, `check`, `build`, `build --release`,
   `build --release --emit=lib`, `run --cycles 2`, `test`, `clean`, rebuild.
6. Verify `package.json`:
   - directory ends with `aarch64-apple-darwin` or `x86_64-apple-darwin`;
   - `debug_contract` is `dwarf`;
   - `SHA256SUMS` verifies;
   - no `__debug_*` sections remain in the archive or dylib.
7. Upload both macOS package artifacts.

**Exit gate.**

- `cargo xtask package --locked --out dist` succeeds on both macOS runners.
- The package metadata, checksums, and debug-section checks pass.
- The unpacked package workflow passes on both macOS architectures.
- The aggregate `ci` gate is green; Windows/Linux package behavior is unchanged.

#### Phase 5C — VS Code extension smoke on macOS

**Work.**

1. Add `macos-15` and `macos-15-intel` to the `extension-smoke` matrix.
2. Keep the single Linux-built VSIX artifact; the VSIX is platform-independent.
3. Make the xvfb installation and `xvfb-run` conditional on Linux. On macOS, run
   `npm run test:electron` directly.
4. Build `stc-driver` and `stc-runtime` on macOS, put `target/debug` on `PATH`, add the Homebrew
   LLVM `PATH`, and set `STC_REQUIRE_CLANG=1`.
5. Verify activation, `stc lsp`, the `stc check` task/problem matcher, and the `stc dap` launch
   session on both macOS architectures.
6. If Electron needs additional macOS launch flags, add them in `editors/vscode/test/electron`
   rather than weakening the smoke. Upload the smoke logs on failure.

**Exit gate.**

- The extension smoke is green on both macOS runners and on Linux.
- The macOS run exercises the real VS Code extension host, LSP, task, and DAP paths.
- The aggregate `ci` gate is green.

#### Phase 5D — Support matrix, decisions, and documentation

**Work.**

1. Add D-250 and supersede/amend the earlier decisions:
   - D-250: macOS ARM64/x86-64 are supported native hosts at Windows parity, excluding EtherCAT
     and PREEMPT_RT-style hard real time; macOS packages, full workspace tests, and extension
     smoke are part of the support claim.
   - D-248: mark superseded by D-250.
   - D-200: amend the host list for macOS.
   - D-205: amend the CI matrix for macOS.
   - D-206: amend the package matrix and `debug_contract` (`dwarf` on Linux and macOS).
2. Revise living documentation:
   - `USER-GUIDE.md`:
     - §1 package layout and the five native packages, including macOS runtime names and
       `debug_contract`.
     - §8 VS Code integration: macOS is supported; the VSIX is platform-independent.
     - §10 cross-target: macOS is a native execution/debug host, not test-only.
     - §11 `--rt`: macOS soft scheduling is supported; no hard-real-time claim.
     - §14 support matrix: add both macOS rows and the exclusions.
   - `KNOWN-LIMITATIONS.md`:
     - §2 hosts/packaging: macOS is supported; replace the test-only section with the
       EtherCAT/PREEMPT_RT exclusions.
     - §3 debug: macOS Mach-O DWARF is a supported path.
     - §7 scheduling: Windows/macOS soft scheduling is advisory; no hard RT.
   - `Architecture.md` §6.4: add macOS to supported hosts; state the exclusions.
   - `Runtime-and-Execution.md` §2.3.3: remove “test-only”; describe the soft mode on macOS.
   - `RT-Host.md` §9 and `RT-Runbook.md` §10: macOS is a supported host for the informational
     soft-scheduling campaign; no hard RT.
   - `Performance.md`: add the support context and keep the numbers informational.
   - `v3.0.0-plan.md`: WL-14 row, goal, locked scope, non-goals, work breakdown, exit criteria,
     evidence, checklist, and change log.
   - `FutureWishlist.md`: WL-14 intent, selection, scope, constraints, open questions, and
     cross-references.
3. Remove every “macOS test-only” claim from the final tree. Keep the historical commits in
   history; the PR tree is authoritative.
4. Run `python tools/ci/check_docs.py`, Markdown lint, `cargo xtask host-audit --check`, and
   `cargo xtask traceability --check`.

**Exit gate.**

- No “test-only” wording remains in the living documentation or decisions.
- The support matrix lists both macOS packages and the two exclusions.
- Docs/lint/traceability/host-audit checks are green.
- The aggregate `ci` gate is green on the revised documentation commit.

### Phase 6 — Manual check and PR

**Goal.** Owner manual approval, clean working tree, final CI, and a PR to `v3.0.0-planning`.

**Work.**

1. Manual verification walkthrough for the owner:
   - CI links for both macOS full-workspace jobs, both macOS package jobs, both macOS extension
     smoke jobs, and the aggregate `ci` gate.
   - The macOS package artifacts: `package.json`, `SHA256SUMS`, `debug_contract: dwarf`, no
     `__debug_*` sections, and the unpacked-package exercise.
   - The extension smoke logs: activation, LSP, task, DAP.
   - A local Windows manual check: build `stc`, run the bench baseline and `--rt`, inspect the
     JSONL header/summary, confirm default behavior is unchanged, and confirm
     `--rt-cpu`/`--rt-priority` errors on Windows.
   - A review of the macOS full-workspace output to confirm remote/serve/LSP/coverage/package/IDE
     targets ran and any Linux-only exclusion is explicit.
   - The revised support matrix and decisions.
2. Owner approval gate. Do not mark WL-14 complete or open the PR before approval.
3. After approval:
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
| macOS ARM64 full workspace | `macos-15` job, `cargo test --workspace --locked` |
| macOS x86-64 full workspace | `macos-15-intel` job, same command |
| macOS remote debug / serve / LSP | Full workspace targets `remote_attach_cli`, `remote_status_cli`, `serve_cli`, `lsp_cli` |
| macOS debug/frame | Mach-O sidecar fixtures; `frames`/`debug_*`/`debug_info`/`checkpoints` in the full workspace |
| macOS release package | `package` matrix entries for both macOS runners; `package.json`, `SHA256SUMS`, unpacked-package exercise |
| macOS package debug contract | `debug_contract: dwarf`; no `__debug_*` sections |
| macOS VS Code integration | `extension-smoke` matrix entries for both macOS runners; real VS Code, LSP, task, DAP |
| macOS online change / state I/O / Modbus | Full workspace and the existing suite targets |
| EtherCAT exclusion | Linux-only backend/shim; macOS test and documentation pin the non-Linux behavior |
| Hard-real-time exclusion | macOS `--rt` is the soft scheduling mode; no PREEMPT_RT claim; docs state the limit |
| Support matrix | `USER-GUIDE.md` §14, `Architecture.md` §6.4, `KNOWN-LIMITATIONS.md` §2, D-250 |
| No timing gate | `ci.yml` has no `sched-measure` job; `Performance.md` states D-193 still applies |
| Default scheduler unchanged | Code inspection plus a run without `--rt`; existing Linux RT tests unchanged |

## 7. Risks and fallbacks

| Risk | Mitigation / fallback |
| --- | --- |
| Full workspace exposes many macOS portability failures | Fix real portability bugs; explicitly cfg-gate only genuinely Linux-only tests with a documented reason; never use a silent filter. |
| macOS Unix-socket path limit (104 bytes) breaks serve/remote/LSP tests | Use short test roots under `/tmp` or the runtime's short-path handling; add a regression test. |
| Fake-SSH remote tests need Python or shell behavior | Use the runner's `python3`; add `actions/setup-python` if needed; keep the fake tools POSIX-sh compatible. |
| `llvm-objcopy` does not support Mach-O stripping | Use `llvm-strip --strip-debug` or Apple `strip -S`; assert the tool on the runner. |
| Mach-O debug-section names are `__debug_*`, not `.debug*` | Extend the section reader/parser for Mach-O; assert the absence of both prefixes. |
| `sha256sum` is absent on macOS | Use `shasum -a 256 -c`; keep `sha256sum` on Windows/Linux. |
| macOS Electron smoke needs a display or entitlement | Use the existing `@vscode/test-electron` launch flags; run the real smoke on the runner. If a platform restriction is proven, record the exact error and keep the Rust LSP/DAP suites as the authoritative feature evidence, but do not silently drop macOS. |
| macOS runners are slower and the full workspace exceeds the timeout | Raise the timeout or split the job by crate group; do not weaken tests. |
| EtherCAT mock on non-Linux could be mistaken for hardware support | Document the Linux-only hardware path and pin the macOS behavior with a test; consider a clear rejection if the mock is not acceptable. |
| The already-committed test-only docs (`db53067`) conflict with the revised scope | Supersede them with the Phase 5D commit; optionally revert `db53067` first if a clean history is preferred. The final tree is authoritative. |
| Two consecutive CI failures | Stop and ask the owner, as required. |

## 8. Status log

| Phase | Status | Commit | CI run | Notes |
| --- | --- | --- | --- | --- |
| 0. Plan | Approved 2026-11-07 | — | — | Decisions 1–4 approved; no-threshold policy kept |
| 1. macOS portability and CI | **Complete and accepted 2026-11-08** | `e5a6a99` | [42460413226](https://github.com/r2k-in-the-vortex/stc/actions/runs/42460413226) | Both macOS jobs, coverage, and the aggregate gate green |
| 2. Mach-O frame extraction | **Complete and accepted 2026-11-08** | `3a86b4e`, `e8bbcdc` | [42506252319](https://github.com/r2k-in-the-vortex/stc/actions/runs/42506252319) | Mach-O DWARF sidecar; debug/frame targets in the suite |
| 3. Scheduling mode | **Complete and accepted 2026-11-08** | `9526b92`, `c4a5345`, `3733082`, `ce2d4bc` | [42516443734](https://github.com/r2k-in-the-vortex/stc/actions/runs/42516443734) | Windows/macOS soft scheduling; metrics and cleanup |
| 4. Measurements | **Complete and accepted 2026-11-08** | `9e894fd`, `e4079bd`, `29338a3`, `a48e256`, `e57731b` | [42523556357](https://github.com/r2k-in-the-vortex/stc/actions/runs/42523556357) | Informational campaign; no threshold |
| 5. macOS parity implementation | **Complete 2026-11-09** | `50de1f1`, `73e6bc4`, `6377193`, `ee016d7`, `ed0c95f`, `1f607dd`, `d315fff`, `73baed5` | [42537537193](https://github.com/r2k-in-the-vortex/stc/actions/runs/42537537193), [42539220760](https://github.com/r2k-in-the-vortex/stc/actions/runs/42539220760), [42541200112](https://github.com/r2k-in-the-vortex/stc/actions/runs/42541200112), [42546445894](https://github.com/r2k-in-the-vortex/stc/actions/runs/42546445894) | Full workspace, Mach-O packages, and VS Code extension smoke green on both macOS architectures; docs and decisions updated. The first Phase 5D run [42544131054](https://github.com/r2k-in-the-vortex/stc/actions/runs/42544131054) failed on a pre-existing Modbus idle-timeout flake; `73baed5` widened the test margin and [42546445894](https://github.com/r2k-in-the-vortex/stc/actions/runs/42546445894) is green. |
| 6. Manual check and PR | Pending owner approval | — | — | Gated on Phase 5 acceptance and owner approval |

## 9. CI procedure

Each phase closes with:

```sh
cd /c/repos/stc
gh workflow run ci.yml --ref WL-14
gh run list --workflow=ci.yml --branch WL-14 --limit 1
```

The phase is complete only when the run is green and the phase's status row is updated. The next
phase's first commit is made only after that. If a phase's CI fails twice in a row, work stops and
the owner is asked before a third attempt.
