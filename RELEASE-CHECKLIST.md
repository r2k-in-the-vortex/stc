# Release checklist — `stc` 1.0.0-rc.1

Owner: project maintainer (`r2k`)  
Release date: 2026-10-28  
Profile: `stc-2013-oop-utf8-v1`  
Milestone: M7 — release candidate

Sign-off requires recorded evidence for every item. An item that cannot be
verified is marked **unsupported** and added to
[`KNOWN-LIMITATIONS.md`](KNOWN-LIMITATIONS.md); it is never left blank.

## 1. Contract freeze

- [x] Version is `1.0.0-rc.1` in the workspace, driver, descriptors, manifests,
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
| 1 | Formatting | `cargo fmt --all --check` | `[x]` passed 2026-10-28 |
| 2 | Clippy | `cargo clippy --workspace --all-targets --locked -- -D warnings` | `[x]` passed 2026-10-28 |
| 3 | Workspace tests | `STC_REQUIRE_CLANG=1 cargo test --workspace --locked` | `[x]` 1068 passed, 0 failed, 3 ignored |
| 4 | Debug suites | `cargo xtask test --suite debug --locked` | `[x]` passed 2026-10-28 |
| 5 | Projects, examples, hardening | `cargo xtask test --suite projects --locked` | `[x]` passed 2026-10-28 |
| 6 | Cross-target matrix | `cargo xtask test --suite cross-target --locked` | `[x]` passed 2026-10-28 |
| 7 | UI, conformance, execution | `cargo xtask test --suite ui|conformance|execution --locked` | `[x]` passed 2026-10-28 |
| 8 | Traceability | `cargo xtask traceability --check` | `[x]` 225 rules, 219 implemented/covered, 4 allowed not-implemented |
| 9 | Specification links | `python tools/ci/check_docs.py` | `[x]` 23 documents, 322 links, OK |
| 10 | Markdown lint | `npx --yes markdownlint-cli2@0.24.0` | `[x]` 0 issues |
| 11 | Performance | `cargo test -p stc-driver --test performance -- --ignored --nocapture` (informational) | `[x]` recorded in `Specification/Verification/Performance.md` |
| 12 | Debug overhead | `cargo test -p stc-driver --test debug_overhead -- --ignored --nocapture` (informational) | `[x]` recorded in `Specification/Verification/Performance.md` |
| 13 | Switch latency | `cargo test -p stc-runtime --test switch_latency -- --ignored --nocapture` (informational) | `[x]` recorded in `Specification/Verification/Performance.md` |

## 3. Package verification

| # | Check | Evidence |
| --- | --- | --- |
| 1 | `cargo xtask package --locked --out dist` succeeds | `[x]` passed 2026-10-28 |
| 2 | Package directory is `stc-1.0.0-rc.1-<host-triple>` with `bin`, `lib`, `docs`, `package.json`, `SHA256SUMS` | `[x]` verified |
| 3 | `sha256sum -c SHA256SUMS` passes for every file | `[x]` all OK |
| 4 | `package.json` reports `schema 1`, version, profile, host, formats (manifest/catalog 2; descriptor/module/state/metadata/debug object/frame 1), host ABI 2.3, `non_safety: not-a-safety-plc` | `[x]` verified |
| 5 | Packaged `bin/stc --version` prints `stc 1.0.0-rc.1`; `stc compliance` prints the non-safety stance | `[x]` verified |
| 6 | Packaged layout resolves the runtime from `lib/` without `--runtime`/`STC_RUNTIME` | `[x]` `package.rs` and fresh-project exercise |
| 7 | `stc new`, `build`, `build --release`, `run --cycles 2`, and `test` pass in a fresh project with the packaged driver | `[x]` passed 2026-10-28 |
| 8 | Static archive and shared library contain no `.debug*` sections after the archive-aware strip step, and the stripped archive still links an executable | `[x]` `debug_release` and package verification |
| 9 | Debug and release builds emit no PDB on Windows; generated DLLs are object-CodeView-only | `[x]` `windows_debug_and_release_builds_emit_no_pdb` |
| 10 | Double builds produce byte-identical IR/manifests and distinct debug/release build IDs (workspace tests) | `[x]` workspace suite |

## 4. Both-host CI evidence

CI is the record for the second host. A green `ci.yml` run is required before
sign-off; the job summary must show `test`, `online`, `debug`, `projects`,
`cross-target`, `package`, `conformance`, `execution`, `ui`, `traceability`,
`lean`, and `fuzz-smoke` passing (or skipped only by the documented detect
conditions).

| Check | Windows | Ubuntu |
| --- | --- | --- |
| Workspace tests | `[x]` | `[x]` |
| Online change | `[x]` | `[x]` |
| Debug matrix | `[x]` | `[x]` |
| Projects/examples/hardening | `[x]` | `[x]` |
| Cross-target matrix | `[x]` | `[x]` |
| Package build + unpacked exercise | `[x]` | `[x]` |
| Conformance/execution/UI | `[x]` | `[x]` |

Fuzz smoke (Ubuntu): `[x]`

CI run URL / ID: https://github.com/r2k-in-the-vortex/stc/actions/runs/40777754963  
Result: success on `windows-2025` and `ubuntu-24.04` (workflow_dispatch, commit `8dbcbf4`)

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
- [x] README, project plan, conformance profile, CLI/testing specifications,
      and the decision log reflect the release candidate.

## 6. Known limitations accepted for this release

- [x] Format-1 manifests: cold download only (D-192).
- [x] Debug/release cross-mode swaps: not a supported pair; keep stores
      separate.
- [x] Windows: object-level CodeView, no PDB, no DLL-level claim (D-190).
- [x] Frame extraction: x86-64 only; AArch64 rules in M8 (D-195).
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
| Release owner | project maintainer (`r2k`) | 2026-10-28 | green CI run 40777754963 |
| Build/CI evidence reviewed | project maintainer (`r2k`) | 2026-10-28 | green on windows-2025 and ubuntu-24.04 |
| Documentation reviewed | project maintainer (`r2k`) | 2026-10-28 | local pass |
| Non-safety statement reviewed | project maintainer (`r2k`) | 2026-10-28 | local pass |

**Status 2026-10-28:** local Windows checks, package verification, the unpacked fresh-project
exercise, and both-host CI run 40777754963 pass. The release candidate is signed off.

The release candidate is signed off only when:

1. every local check above is checked with its command output recorded;
2. the both-host CI run is green and its ID is filled in;
3. the package checksums and unpacked exercise pass on both hosts;
4. no item was quietly removed from this list.

If a check fails, fix the cause or record it as a limitation; do not widen a
timeout or delete an assertion to make the suite pass (D-198).
