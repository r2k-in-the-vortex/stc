# Release checklist — `stc` 1.0.0

Owner: project maintainer (`r2k`)  
Release date: 2026-10-29  
Profile: `stc-2013-oop-utf8-v1`  
Milestone: v1.0.0 final; M8 — Linux host portability

Sign-off requires recorded evidence for every item. An item that cannot be
verified is marked **unsupported** and added to
[`KNOWN-LIMITATIONS.md`](KNOWN-LIMITATIONS.md); it is never left blank.

## 1. Contract freeze

- [x] Version is `1.0.0` in the workspace, driver, descriptors, manifests,
      and package.
- [x] No module, descriptor, state, metadata, debug-object, stopped-frame,
      manifest-format-2, host-ABI 2.3, service-ID, build-ID-domain, or
      `stc-debug/1` change was made after the freeze except D-191's
      archive-only strip step (no format/ABI change).
- [x] Debug is the default build form; release is stripped, hook-free, and
      separately identified; test builds are hook-free at ABI 2.2.
- [x] Version-bearing snapshots were regenerated once and committed.
- [x] The four non-debug traceability allowlist entries remain documented as
      not implemented; no debug rule is allowlisted.

## 2. Local verification (Windows 11 x86-64 MSVC)

Record the command, result, and date. Re-run after any source change.

| # | Check | Command | Result |
| --- | --- | --- | --- |
| 1 | Formatting | `cargo fmt --all --check` | `[x]` passed 2026-10-29 |
| 2 | Clippy | `cargo clippy --workspace --all-targets --locked -- -D warnings` | `[x]` passed 2026-10-29 |
| 3 | Workspace tests | `STC_REQUIRE_CLANG=1 cargo test --workspace --locked` | `[x]` 1078 passed, 0 failed, 3 ignored (2026-10-29) |
| 4 | Debug suites | `cargo xtask test --suite debug --locked` | `[x]` passed 2026-10-29 |
| 5 | Projects, examples, hardening | `cargo xtask test --suite projects --locked` | `[x]` passed 2026-10-29 |
| 6 | Cross-target matrix | `cargo xtask test --suite cross-target --locked` | `[x]` passed 2026-10-29 |
| 7 | UI, conformance, execution | `cargo xtask test --suite ui|conformance|execution --locked` | `[x]` passed 2026-10-29 |
| 8 | Traceability | `cargo xtask traceability --check` | `[x]` 253 rules, 249 implemented/covered, 4 allowed not-implemented |
| 9 | Specification links | `python tools/ci/check_docs.py` | `[x]` 22 documents, 328 links, OK |
| 10 | Markdown lint | `npx --yes markdownlint-cli2@0.24.0` | `[x]` 0 issues |
| 11 | Performance | `cargo test -p stc-driver --test performance -- --ignored --nocapture` (informational) | `[x]` recorded 2026-10-28 in `Specification/Verification/Performance.md` (rc.1 build; not re-run for the version-only change) |
| 12 | Debug overhead | `cargo test -p stc-driver --test debug_overhead -- --ignored --nocapture` (informational) | `[x]` recorded 2026-10-28 in `Specification/Verification/Performance.md` (rc.1 build; not re-run for the version-only change) |
| 13 | Switch latency | `cargo test -p stc-runtime --test switch_latency -- --ignored --nocapture` (informational) | `[x]` recorded 2026-10-28 in `Specification/Verification/Performance.md` (rc.1 build; not re-run for the version-only change) |

## 3. Package verification

| # | Check | Evidence |
| --- | --- | --- |
| 1 | `cargo xtask package --locked --out dist` succeeds | `[x]` passed 2026-10-29 |
| 2 | Package directory is `stc-1.0.0-<host-triple>` with `bin`, `lib`, `docs`, `package.json`, `SHA256SUMS` | `[x]` verified |
| 3 | `sha256sum -c SHA256SUMS` passes for every file | `[x]` all 9 files OK |
| 4 | `package.json` reports `schema 1`, version, profile, host, formats (manifest/catalog 2; descriptor/module/state/metadata/debug object/frame 1), host ABI 2.3, `non_safety: not-a-safety-plc` | `[x]` verified 2026-10-29 |
| 5 | Packaged `bin/stc --version` prints `stc 1.0.0`; `stc compliance` prints the non-safety stance | `[x]` verified 2026-10-29 |
| 6 | Packaged layout resolves the runtime from `lib/` without `--runtime`/`STC_RUNTIME` | `[x]` `package.rs` and fresh-project exercise 2026-10-29 |
| 7 | `stc new`, `build`, `build --release`, `run --cycles 2`, and `test` pass in a fresh project with the packaged driver | `[x]` passed 2026-10-29 |
| 8 | Static archive and shared library contain no `.debug*` sections after the archive-aware strip step, and the stripped archive still links an executable | `[x]` `debug_release` and package verification |
| 9 | Debug and release builds emit no PDB on Windows; generated DLLs are object-CodeView-only | `[x]` `windows_debug_and_release_builds_emit_no_pdb` |
| 10 | Double builds produce byte-identical IR/manifests and distinct debug/release build IDs (workspace tests) | `[x]` workspace suite |

## 4. Three-host CI evidence

CI is the record for the Linux hosts. A green `ci.yml` run is required before
sign-off; the job summary must show `test`, `online`, `debug`, `projects`,
`cross-target`, `package`, `conformance`, `execution`, `ui`, `traceability`,
`lean`, and `fuzz-smoke` passing (or skipped only by the documented detect
conditions).

| Check | Windows | Ubuntu x86-64 | Ubuntu AArch64 |
| --- | --- | --- | --- |
| Workspace tests | `[x]` | `[x]` | `[x]` |
| Online change | `[x]` | `[x]` | `[x]` |
| Debug matrix | `[x]` | `[x]` | `[x]` |
| Projects/examples/hardening | `[x]` | `[x]` | `[x]` |
| Cross-target matrix | `[x]` | `[x]` | `[x]` |
| Package build + unpacked exercise | `[x]` | `[x]` | `[x]` |
| Conformance/execution/UI | `[x]` | `[x]` | `[x]` |

Fuzz smoke (Ubuntu x86-64): `[x]`

CI run URL / ID: https://github.com/r2k-in-the-vortex/stc/actions/runs/40821738218  
Result: success on `windows-2025`, `ubuntu-24.04`, and `ubuntu-24.04-arm` (commit `ba97c5a`, all 36 jobs).
The `1.0.0` change is release-identity metadata only (D-210); this run remains the qualification
evidence, and the release PR's CI run is the final gate for the version commit.

## 5. Documentation

- [x] [`RELEASE-NOTES.md`](RELEASE-NOTES.md) — version, date, highlights,
      artifacts, formats/ABI, upgrade notes, support matrix, verification.
- [x] [`KNOWN-LIMITATIONS.md`](KNOWN-LIMITATIONS.md) — consolidated v1 limits
      and workarounds, including object-only CodeView, caller frames, format-1
      baselines, cross-mode swaps, local-only debug, limits, and M8 scope.
- [x] [`USER-GUIDE.md`](USER-GUIDE.md) — install, quick start, manifest, build
      modes, commands, online change, debug, tests, cross-target, performance,
      troubleshooting, support matrix.
- [x] [`NON-SAFETY.md`](NON-SAFETY.md) — `stc` can control real machinery but
      is not a safety PLC; safety must be provided by a separate safety system
      and machine design as required by law and regulation.
- [x] Every example README repeats the non-safety stance.
- [x] [`Specification/Verification/Performance.md`](Specification/Verification/Performance.md)
      records host, regime, numbers, and the non-gating policy.
- [x] README, conformance profile, CLI/testing specifications, and the decision
      log reflect the release.

## 6. Known limitations accepted for this release

- [x] Format-1 manifests: cold download only (D-192).
- [x] Debug/release cross-mode swaps: not a supported pair; keep stores
      separate.
- [x] Windows: object-level CodeView, no PDB, no DLL-level claim (D-190).
- [x] Frame extraction: x86-64 and AArch64 Linux; AArch64 rules delivered in
      M8 (D-202, D-209).
- [x] Debug: local only, typed only, no raw memory (D-194).
- [x] Prepare remains on the cycle thread between scans; fully off-thread
      preparation is post-v1.
- [x] Performance numbers are informational (D-193).
- [x] Four non-debug traceability rules remain not implemented and documented
      (D-199).
- [x] No installers, registry, hardware I/O, or remote deployment in v1.

## 7. Sign-off

| Role | Name | Date | Signature / result |
| --- | --- | --- | --- |
| Release owner | project maintainer (`r2k`) | 2026-10-29 | green CI run 40821738218 |
| Build/CI evidence reviewed | project maintainer (`r2k`) | 2026-10-29 | green on windows-2025, ubuntu-24.04, and ubuntu-24.04-arm |
| Documentation reviewed | project maintainer (`r2k`) | 2026-10-29 | local pass |
| Non-safety statement reviewed | project maintainer (`r2k`) | 2026-10-29 | local pass |

**Status 2026-10-29:** local Windows checks (1078 workspace tests, all suites, docs, traceability,
lint), `stc-1.0.0` package verification (9/9 checksums), the unpacked fresh-project exercise, and
three-host CI run 40821738218 pass. The release is signed off; the release PR CI run is the final
confirmation for the version-only commit.

The release is signed off only when:

1. every local check above is checked with its command output recorded;
2. the both-host CI run is green and its ID is filled in;
3. the package checksums and unpacked exercise pass on both hosts;
4. no item was quietly removed from this list.

If a check fails, fix the cause or record it as a limitation; do not widen a
timeout or delete an assertion to make the suite pass (D-198).
