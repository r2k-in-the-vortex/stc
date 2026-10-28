# M7 Implementation Plan — Release Candidate

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-28  
Status: Implemented (2026-10-28); release-checklist sign-off pending the both-host CI run. Phase 0
fixed the release identity and packaging contracts; phases 1–8 delivered packaging, cross-target,
performance, examples, documentation, and hardening (D-188–D-199).
Inputs: [Project Plan](Specification/00-Project-Plan.md) §8 (M7–M8), `handover_to_M7.md`,
[Architecture](Specification/Architecture/Architecture.md),
[CLI and Manifest](Specification/Architecture/CLI-and-Manifest.md),
[Testing and Verification](Specification/Architecture/Testing-and-Verification.md),
[Online Change and GOT](Specification/Architecture/Online-Change-and-GOT.md),
[GOT Artifacts](Specification/Architecture/GOT-Artifacts.md),
[Debug and Monitoring](Specification/Architecture/Debug-and-Monitoring.md),
and Decisions D-042, D-173–D-187.

M7 turns the implemented M6.5 host into a releasable product. It adds no language surface and
changes no module, descriptor, state, metadata, debug-object, stopped-frame, manifest, ABI, or
`stc-debug/1` contract. Every sample, package, document, and test is an addition over the
M6.5 implementation; any output change is treated as an identity decision recorded in
`Specification/Decisions.md`.

## 1. Objective

Publish a release candidate that a new user can install, build against, run, debug, test, and
update online, with reproducible artifacts, an explicit support matrix, an honest limitation
list, and the documented non-safety stance.

## 2. Scope

### 2.1 In scope

| Area | Work |
| --- | --- |
| Release identity | Version `1.0.0-rc.1`; version-bearing snapshots regenerated once; compatibility note for pre-RC baselines |
| Packaging | `cargo xtask package`; versioned release layout; driver and runtime archives; checksums; package manifest; adjacency lookup |
| Fresh checkout | Cold build from a clean clone with the pinned toolchain; documented clang discovery |
| Cross-target | `clang --target` IR/object smoke matrix; explicit rejection of cross-target executable/library builds; support matrix |
| Performance | Host/regime-labelled measurements for build, cycle, switch, and debug overhead; informational, not CI-gating |
| Examples | Multi-package `examples/motor` and a focused `examples/online-change`; hello kept; example smoke in CI |
| Documentation | `RELEASE-NOTES.md`, `KNOWN-LIMITATIONS.md`, `NON-SAFETY.md`, `USER-GUIDE.md`, `RELEASE-CHECKLIST.md`; README and spec updates |
| Online hardening | Format-1 baseline cold path, debug↔release online behaviour, tampered/truncated download rejection, baseline pairing checks |
| Debug hardening | Object-only CodeView contract, archive cleanliness, resource-limit documentation and tests, release-artifact assertions |
| Sign-off | Checklist evidence from both hosts; handover to M8 |

### 2.2 Out of scope

- New language features, new standard functions, or conformance beyond
  `stc-2013-oop-utf8-v1`.
- Changes to module/descriptor/state/metadata/debug-object/frame/manifest formats, host ABI,
  service IDs, build-ID domains, or the `stc-debug/1` wire schema.
- Remote/network debugging, raw-memory access, multi-user sessions, or a privileged mode.
- Linux x86-64/AArch64 native host builds, AArch64 frame rules, and AArch64 execution
  (M8, D-042).
- Installers, package-manager integration, a registry, or a version solver.
- Safety functions or certification; the non-safety stance is documented, never softened.

## 3. Exit criteria

- No silent acceptance of unsupported features: every requested artifact/command either works
  or reports a documented diagnostic; cross-target execution, format-1 manifests, debug↔release
  swaps, and unsupported frame targets are explicit.
- Documented artifacts and ABI; reproducible IR and manifests; clean build from a fresh
  checkout with the pinned toolchain and documented clang discovery.
- Hot-swap, cold-download, manifest-download, debug, and monitoring suites pass on both hosts.
- Release checklist signed off with recorded evidence.

## 4. Work breakdown

| Phase | Work | Acceptance |
| --- | --- | --- |
| 0 | Freeze contract, fix release identity/decision records, record baseline | Decisions D-188–D-199; plan reviewed; clean baseline run |
| 1 | Release identity, packaging, runtime adjacency, checksums, fresh-checkout verification | `cargo xtask package` produces a verified layout twice; installed layout builds and runs |
| 2 | Cross-target IR/object smoke matrix and support matrix | `cross-target` suite passes on both hosts; exe/lib rejection tested |
| 3 | Performance pass: build, cycle, switch, debug overhead; recorded host/regime | `performance` suite runs; numbers recorded, non-gating |
| 4 | Examples: `motor` multi-package, `online-change`, hello refresh, example smoke | Every example checks/builds/runs/tests/fmt-checks; CI covers them |
| 5 | Release notes, known limitations, non-safety, user guide, README/spec updates | `check_docs.py` and Markdown lint clean; docs consistent |
| 6 | Manifest/online-change hardening | Format-1, debug↔release, tampered-download, and baseline-pairing tests pass |
| 7 | Debug/monitoring hardening | Object-only CodeView contract, archive cleanliness, limit tests, release artifact assertions |
| 8 | Release checklist, CI release job, sign-off, M8 handover | Fresh-checkout and both-host evidence recorded; checklist signed |

## 5. Phase 0 — release decisions

These decisions fix M7 behaviour; later phases implement and test them. Formats and ABI are
unchanged unless a new decision explicitly says otherwise.

| Decision | Choice | Reason |
| --- | --- | --- |
| D-188 | M7 is a release-candidate milestone at version `1.0.0-rc.1`; no language/format change | The profile is v1; the RC marker distinguishes it from the final v1 and from post-1.0 work |
| D-189 | `cargo xtask package` owns release packaging; layout `stc-<version>-<host>/bin`, `lib`, `docs`, `SHA256SUMS`, `package.json` | Repository task, no product CLI surface; deterministic names and no timestamps |
| D-190 | Windows release contract is object-level CodeView; no PDB and no DLL-level CodeView claim | The runtime consumes sidecars, not DWARF/CodeView; PDBs add nondeterministic identity |
| D-191 | Release static archives are unpacked, each object stripped with `llvm-objcopy --strip-debug`, and the stripped objects replace the originals in place with `llvm-ar rD`; import-library members and the symbol table are preserved | Precompiled Rust standard-library members carry debug sections even in a release build, and a fresh archive repack would drop duplicated Windows import-library symbols |
| D-192 | Format-1 manifests are cold-download only; no migration tool | Reinterpreting pre-M6.5 baselines would risk preserved-value safety |
| D-193 | Performance numbers are informational, host/regime-labelled, and not CI gates | Host variance would make a gate noisy; the plan records measurements instead |
| D-194 | Remote debug, raw memory, and unauthenticated modes remain out of scope | Security and architecture decision, not a hardening tweak |
| D-195 | Cross-target support means IR/object emission only; AArch64 frames and execution are M8 | No sysroots or execution hosts are part of v1 packaging |
| D-196 | Embedded hosts use the pipe transport in v1; socket remains a CLI/API option | The embedding ABI has no transport selector; adding one is an ABI change |
| D-197 | The release checklist owner is the project maintainer; artifacts are the two-host package, checksums, suites, notes, and docs | One named owner and an explicit artifact list make sign-off auditable |
| D-198 | Flaky process/timing tests are bugs; capture logs and diagnose before changing timeouts | Prevents the debug suite from passing by widening deadlines |
| D-199 | The four non-debug traceability allowlist entries stay documented as not-implemented v1 policy | Closing them would change state/I/O behaviour, which M7 does not do |

## 6. Phase 1 — release identity and packaging

### 6.1 Version

- Set the workspace version to `1.0.0-rc.1`.
- Regenerate the version-bearing snapshots once (`STC_UPDATE_SNAPSHOTS=1` where supported) and
  record the resulting identity change. Pre-RC manifests remain valid inputs for a baseline
  check only where formats match; their build IDs are not reused.
- `stc --version` and `stc metadata` report the RC version.

### 6.2 Package layout

```text
stc-1.0.0-rc.1-<host-triple>/
  bin/stc(.exe)
  lib/stc_runtime.lib | libstc_runtime.a
  lib/stc_runtime.dll | libstc_runtime.so
  docs/README.md
  docs/USER-GUIDE.md
  docs/RELEASE-NOTES.md
  docs/KNOWN-LIMITATIONS.md
  docs/NON-SAFETY.md
  package.json
  SHA256SUMS
```

- `package.json` records `schema`, `version`, `profile`, `host`, the supported artifact formats
  and ABI minor, the artifact list, and the non-safety identifier. It contains no timestamp,
  path, or user name.
- `SHA256SUMS` is sorted and covers every other file in the package; `sha256sum -c`-compatible.
- The driver searches for the runtime next to its executable and in `<exe>/../lib`, so the
  packaged `bin`/`lib` layout works without environment variables. The `--runtime`,
  `STC_RUNTIME`, and manifest settings keep precedence.
- Packaging refuses to run without the pinned toolchain; it builds with `--locked`.
- The release profile sets `strip = "debuginfo"`. Static archives are additionally unpacked, stripped
  member-by-member with `llvm-objcopy --strip-debug`, and updated in place with `llvm-ar rD` so the
  archive stays linkable; packaging rejects an archive, shared library, module, or executable that
  still has debug sections.

### 6.3 Fresh checkout

- `cargo xtask fresh-check` (or the package job's checkout step) builds from a clean clone in a
  temporary directory: `cargo build --release --locked -p stc-driver -p stc-runtime`, then
  `stc new`, `stc build`, `stc build --release`, `stc test`, and `stc run --cycles 1` in a new
  project.
- Document clang/LLVM discovery and the required inspection tools (`llvm-ar`, `llvm-readobj`,
  or `readelf`) in the user guide.

## 7. Phase 2 — cross-target smoke matrix

- Add a driver integration target (`cross_target`) that builds the same IR with and without an
  explicit `--target`, then invokes `clang --target=<triple> -x ir -c` for:
  - `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
  - `x86_64-apple-darwin`, `aarch64-apple-darwin`,
  - `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`,
  - `riscv64-unknown-elf`, `thumbv7em-none-eabihf`,
  - one big-endian triple (`powerpc64-unknown-linux-gnu`).
- Assert the emitted object has the requested architecture/endianness through `llvm-readobj`
  (or `readelf` where available), and that the IR remains free of target datalayout and offset
  constants.
- Assert `--emit=exe` and `--emit=lib` with a non-host `--target` fail with the documented
  diagnostic rather than producing a partial artifact.
- Extend the CI `cross-target` job to run the new suite on both hosts; keep the existing raw
  `smoke.ll` check.
- Document the support matrix: object emission across the tested triples; execution and module
  loading only on x86-64 Windows MSVC and x86-64 Linux GNU; AArch64 host/execution is M8.

## 8. Phase 3 — performance pass

- Add a driver integration target (`performance`, ignored by default) that measures, with the
  host and regime printed:
  - `stc check`, IR generation, object emission, and full module link for the examples;
  - cycle time for a release module, a hook-free test module, and a hook-bearing debug module;
  - O(1) switch latency at several root counts and debug-session overhead;
  - typed read latency.
- Record the results in `Specification/Verification/Performance.md` with date, host, clang,
  cargo, root/checkpoint counts, and variance. State that numbers are informational.
- Keep timing out of CI gates; a dispatched job may publish numbers.
- Investigate, but do not gate on, any regression against the M6.5 phase-9 figures.

## 9. Phase 4 — examples

- `examples/motor`: a two-package project. The dependency package exposes a motor/controller
  library with a class, an interface, a function block, arrays, a `USTRING`, subranges/enums,
  and a task configuration; the root package wires I/O, retained state, a test suite, and an
  online-change walkthrough. It replaces the README's stale `examples/motor` references with a
  real project.
- `examples/online-change`: a minimal program used by the documented hot-swap, rename, and cold
  download walkthrough, with commands and expected output in its README.
- `examples/hello`: keep as the starter; refresh links/version statements and the debug sample.
- Every example commits `stc.lock`, ignores build output, and carries a README with the
  non-safety notice.
- Extend the `projects` CI suite (or add an `examples` suite target) to run `check`, `build`,
  `build --release`, `test`, and `fmt --check` for every example.

## 10. Phase 5 — release documentation

- `RELEASE-NOTES.md`: version, date, highlights by milestone, supported hosts and targets,
  artifact/ABI summary, upgrade notes, known limitations link, and the non-safety statement.
- `KNOWN-LIMITATIONS.md`: consolidated handover §4 items and plan non-goals, each with the
  honest workaround or explicit "unsupported" label. Includes object-only CodeView, caller-frame
  coverage, archive stripping, format-1 baselines, debug↔release swaps, transport limits,
  performance variance, and M8 scope.
- `NON-SAFETY.md`: explicit statement that `stc` is not a safety PLC, assumes no safety
  responsibility, and may hold outputs indefinitely while stopped or forced. It can control real
  machinery; safety must be provided by a separate system and machine design as required by law
  and regulation. There is no fail-safe timeout, interlock, redundancy, or SIL claim.
- `USER-GUIDE.md`: install/unpack, clang discovery, project workflow, build modes and
  artifacts, online change, debug/monitoring, tests, performance/limits, support matrix, and
  troubleshooting.
- `README.md`: status, version, install/quickstart, links to the new documents, and the
  non-safety notice; remove stale milestone text.
- Specification updates: artifact/package layout and runtime search order
  (CLI-and-Manifest), release/cross-target/performance evidence (Testing-and-Verification),
  support matrix and non-safety (Conformance Profile), and decision records.
- `tools/ci/check_docs.py` gains the new release documents if they become required.

## 11. Phase 6 — manifest and online-change hardening

- Format-1 baseline: generate/ship a format-1 fixture; assert the descriptor rejection
  diagnostic, the no-silent-upgrade rule, and that `--cold` / a missing baseline produces a
  deployable cold artifact.
- Debug↔release: test a candidate built in the other mode against the active baseline; record
  the compatibility verdict, ensure no incompatible activation is accepted, and document the
  cold-download path.
- Download hardening: reject truncated, bit-flipped, digest-mismatched, wrong-pairing, and
  malformed manifests with stable diagnostics; never execute manifest content.
- Baseline pairing: test a manifest paired with the wrong descriptor/build ID and a stale
  parent baseline; verify the documented cold or reject outcome.
- Keep the existing fuzz targets and double-build determinism coverage; add the new fixtures to
  the online suite and CI.
- Confirm the four allowlisted traceability IDs remain documented v1 policy and are not silently
  claimed.

## 12. Phase 7 — debug and monitoring hardening

- Windows: assert the release contract is object-level CodeView. Check the COFF object has
  `.debug$S`/`.debug$T`, the linked DLL has no CodeView/PDB, and the documented limitation says
  so. Do not add `/DEBUG` or PDB packaging.
- Release archives: assert the packaged static archive and `--emit=lib --release` archive have no
  debug sections after the archive-aware strip step; keep import-library members intact.
- Session limits: add tests for the documented limits that lack coverage (watch count/interval,
  breakpoint count, queued write/force bounds, frame/leaf caps, rate limiting) and state them as
  product limits in the user guide.
- Swap/session teardown: re-run the swap, rollback, cold-install, shutdown, and target-exit
  teardown tests; assert queued forces/writes are rejected and state is cleared.
- Release cleanliness: assert release modules/executables/manifests carry no debug object,
  frame sidecar, or checkpoint hooks, and have distinct build IDs from debug forms.
- Flake policy: diagnose any timing failure; fix the race rather than widening deadlines. Keep
  the endpoint-file wait.
- Do not weaken local authentication, scopes, audit, pause bound, or the non-safety notice.

## 13. Phase 8 — release checklist and sign-off

- `RELEASE-CHECKLIST.md` with owner, date, artifact hashes, and checkboxes for:
  - fresh-checkout build with the pinned toolchain;
  - `cargo fmt --all --check`, clippy `-D warnings`, `cargo test --workspace --locked`,
    `cargo xtask test --suite debug`, `online`, `projects`, `conformance`, `execution`, `ui`,
    `cross-target`, `performance`;
  - both-host CI results;
  - package contents, checksums, stripped/debug-free assertions;
  - release notes, known limitations, user guide, non-safety statement;
  - traceability, `check_docs.py`, Markdown lint;
  - version and build-ID notes.
- Add a CI `release` job on Windows and Ubuntu: `cargo xtask package --locked`, verify the
  package, upload it as an artifact, and run `stc --version`/`stc compliance` from the package.
- Create `handover_to_M8.md`: remaining limitations, the M8 portability work, and the contracts
  M8 must preserve.
- Update the Project Plan M7 status, this plan's status, the decision log, and README.
- The checklist is signed only when every item has recorded evidence; unverified items are
  marked unsupported, not left blank.

## 14. Test and verification plan

| Level | Method |
| --- | --- |
| Packaging | `cargo xtask package` twice; content, checksum, version, ABI, and stripped-archive assertions; installed-layout build/run |
| Fresh checkout | Clean clone build and `new/build/run/test` in CI |
| Cross-target | IR/object emission for the matrix; architecture/endianness inspection; exe/lib rejection |
| Performance | Ignored `performance` test with host/regime; recorded table; no gate |
| Examples | `check/build/build --release/test/fmt --check` for every example; documented walkthroughs |
| Online hardening | Format-1, debug↔release, tampered download, pairing, stale parent, cold path |
| Debug hardening | CodeView/PDB contract, archive cleanliness, limits, teardown, release cleanliness |
| Documentation | `check_docs.py`, Markdown lint, traceability, link checks, non-safety presence |
| Release | Checklist evidence from Windows and Ubuntu CI plus a local package run |

Commands used for the release candidate:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
$env:STC_REQUIRE_CLANG = "1"
cargo test --workspace --locked
cargo xtask test --suite debug --locked
cargo xtask test --suite projects --locked
cargo xtask test --suite cross-target --locked
cargo xtask test --suite ui --locked
cargo xtask test --suite conformance --locked
cargo xtask test --suite execution --locked
cargo test -p stc-driver --test performance --locked -- --ignored --nocapture
cargo xtask package --locked
cargo xtask traceability --check
python tools/ci/check_docs.py
npx --yes markdownlint-cli2@0.24.0
```

Online-change coverage is the `online` CI job (`stc-got`, `stc-runtime`, `module_cli`,
`execution`) plus the `projects` suite's `online_cli` and `manifest_hardening` targets.

## 15. Deliverables checklist

- [x] Version `1.0.0-rc.1` and regenerated snapshots.
- [x] `cargo xtask package` with verified layout and checksums.
- [x] Runtime adjacency search and installed-layout test.
- [x] Cross-target suite and support matrix.
- [x] Performance measurements recorded.
- [x] `examples/motor`, `examples/online-change`, refreshed `examples/hello`, example CI.
- [x] `RELEASE-NOTES.md`, `KNOWN-LIMITATIONS.md`, `NON-SAFETY.md`, `USER-GUIDE.md`,
      `RELEASE-CHECKLIST.md`.
- [x] Online-change and debug hardening tests and documentation.
- [ ] Both-host suite evidence, traceability, docs, lint, and checklist sign-off (local evidence
      complete; both-host CI and signature pending).
- [x] `handover_to_M8.md` and project-plan/decision-log updates.
