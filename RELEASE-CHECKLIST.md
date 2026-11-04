# Release checklist — `stc` 2.0.0

- Owner: project maintainer (`r2k`)
- Target release date: 2026-11-04
- Profile: `stc-2013-oop-utf8-v1`
- Baseline: `stc` 1.0.0 (2026-10-29)
- Branch: `v2.0.0-planning`

Sign-off requires recorded evidence for every item. An item that cannot be
verified is marked **unsupported** and added to
[`KNOWN-LIMITATIONS.md`](KNOWN-LIMITATIONS.md); it is never left blank.

**Current status:** the workspace and version-bearing snapshots are now
`2.0.0`. The final `2.0.0` CI run, all three package builds, and the
Aeneas/Charon build are green and verified. Artifact upload and owner sign-off
are still open.

## 1. Release identity and contract freeze

| # | Check | Evidence | Result |
| --- | --- | --- | --- |
| 1 | Workspace, driver, descriptors, manifests, and package report `2.0.0` | `Cargo.toml`, `Cargo.lock`, generated descriptors, `package.json` | `[x]` `Cargo.toml` and `Cargo.lock` report `2.0.0`; final descriptors/package are produced at release build |
| 2 | Version-bearing snapshots are regenerated once and committed | `STC_UPDATE_SNAPSHOTS=1` codegen snapshot; release-identity diff | `[x]` `library.ll` regenerated with `stc 2.0.0` and new deterministic build IDs |
| 3 | Profile, language surface, artifact formats, and host ABI are unchanged from 1.0.0 except the documented revisions | [USER-GUIDE §4](USER-GUIDE.md#4-build-modes-and-artifacts) | `[x]` format 2 GOT/catalog; format 1 descriptor/module/state/metadata/debug object; stopped-frame sidecar 2; host ABI 2.3 |
| 4 | Debug protocol is additive: `stc-debug/1` remains valid; `stc-debug/1.1` adds Locals and stopped writes | [Debug and Monitoring](specification/Architecture/Debug-and-Monitoring.md) | `[x]` |
| 5 | Release notes, checklist, README, user guide, known limitations, and non-safety statement describe the release | this file and [RELEASE-NOTES.md](RELEASE-NOTES.md) | `[x]` |
| 6 | Extension version and VSIX contents are recorded | `editors/vscode/package.json` → `0.1.0`; CI `VS Code extension package` | `[x]` |

## 2. Full CI verification

The full CI matrix at the final release commit `0f3bf3c` passed:
[run 41882102341](https://github.com/r2k-in-the-vortex/stc/actions/runs/41882102341),
**39/39 jobs**, 0 skipped, 0 failed, on 2026-11-04. The code commit `e604435`
also passed [run 41879601977](https://github.com/r2k-in-the-vortex/stc/actions/runs/41879601977)
with the same 39/39 result.

| # | Check | Evidence | Result |
| --- | --- | --- | --- |
| 1 | Full CI matrix | run `41882102341`; `windows-2025`, `ubuntu-24.04`, `ubuntu-24.04-arm` | `[x]` 39/39 |
| 2 | Workspace and driver tests | Rust test jobs on all three hosts | `[x]` |
| 3 | Online change, debug, UI, conformance, execution, projects, cross-target | CI jobs | `[x]` |
| 4 | Packaging and fresh-project exercise at release identity | Release package jobs on all three hosts | `[x]` all three jobs passed at `2.0.0` |
| 5 | VS Code extension | typecheck, grammar/problem-matcher tests, VSIX contents, headless smoke | `[x]` |
| 6 | EtherCAT shim and DIEWU fixture | `EtherCAT shim (Linux)` job | `[x]` |
| 7 | Fuzz smoke and traceability | `Fuzz smoke`, `Rule traceability` jobs | `[x]` |
| 8 | Docs, specification links, Markdown lint | `Docs and specification` job | `[x]` |
| 9 | Lean proofs | `Lean proofs` job, 2m 33s | `[x]` |
| 10 | Coverage gate | `Rust coverage` job; threshold 85%; final run records 85.28% (99,696/116,909) | `[x]` |
| 11 | Local release-identity checks | `cargo fmt --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; `cargo test --workspace --locked`; `cargo test -p xtask`; Windows package build; traceability; host audit | `[x]` passed at `e604435` |

## 3. Aeneas/Charon bridge verification

This is deliberately separate from fast CI and is not run on a GitHub-hosted
runner. It was re-run on the pinned host `ecat-host` at the final version
commit `e604435` on 2026-11-04.

| # | Check | Evidence | Result |
| --- | --- | --- | --- |
| 1 | Pin and source-hash guard | `python3 tools/ci/check_bridge_toolchain.py --check-environment`; three source pins match; checked-in exact model present | `[x]` OK |
| 2 | Aeneas and Charon versions | `aeneas -version` → `ab34bd2`; Charon pin/commit → `7b462d9` | `[x]` match pins |
| 3 | Bridge package build | release worktree at `e604435`; `cd lean/aeneas && lake build`; log `/home/r2k/stc-release-aeneas.log` on `ecat-host` | `[x]` 1814 jobs, exit 0 |
| 4 | Exact and wide bridge modules | generated exact model; `Bridge.Exact`/`Wide*` targets | `[x]` built |
| 5 | No proof failures or `sorry`/`admit` regressions | build succeeded; `grep -RInw -e sorry -e admit lean/aeneas/Bridge lean/aeneas/StcGenerated` found no matches | `[x]` |
| 6 | Claim boundary recorded | [Phase 2 report](specification/Verification/Reports/Phase2-Report.md) | `[x]` one V1 record for the exact scalar kernel; other records remain V0 |

Note: the checked-in wire model is still absent, so the guard reports that
notice; this is the existing Phase 1 limitation, not a v2 regression.

Regeneration is **not** part of this run. The exact model is current by drift
evidence rather than a fresh regeneration: the current `exact.rs` SHA-256
matches the pin and generated header, the checked-in body's SHA-256 is the
recorded accepted generated hash `fa6628af…` (744 lines), and the matching
`exact-wide-3` spike has the same generated file hash. A fresh regeneration
diff remains available on the pinned host if release policy requires it.

## 4. Target evidence

| # | Target | Evidence | Result |
| --- | --- | --- | --- |
| 1 | WL-1.1 — VS Code integration and debugging | PR #18; owner-confirmed manual acceptance 2026-11-03; final branch run `41732616607`; current extension jobs in `41882102341` | `[x]` |
| 2 | WL-2 — 85% compile coverage | coverage job passed; 85.28% recorded on final run `41882102341`; gate active in CI | `[x]` |
| 3 | WL-3 — Lean and bridge verification | [Phase 0](specification/Verification/Reports/Phase0-Report.md), [Phase 1](specification/Verification/Reports/Phase1-Report.md), [Phase 2](specification/Verification/Reports/Phase2-Report.md); CI Lean job; Aeneas run above | `[x]` |
| 4 | WL-4 — RT verification | [RT-Host.md](specification/Verification/RT-Host.md), [RT-Runbook.md](specification/Verification/RT-Runbook.md), [RT-Run-Manifest.md](specification/Verification/RT-Run-Manifest.md); host-specific | `[x]` |
| 5 | WL-5.1 — Modbus TCP server | [Modbus-Server.md](specification/Verification/Modbus-Server.md); [tests/modbus](tests/modbus/README.md); mixed bench 60,000 cycles | `[x]` |
| 6 | WL-5.2 — EtherCAT on Linux | [EtherCAT-Host.md](specification/Verification/EtherCAT-Host.md); [EtherCAT-Runbook.md](specification/Verification/EtherCAT-Runbook.md); 16/16 all-points and fault matrix | `[x]` |
| 7 | WL-10 — ST test coverage | [Testing and Verification](specification/Architecture/Testing-and-Verification.md) §2.6; [examples/coverage](examples/coverage/README.md); golden reports | `[x]` |

## 5. Packaging and release artifacts

| # | Check | Evidence | Result |
| --- | --- | --- | --- |
| 1 | `cargo xtask package --locked` succeeds for all three native hosts at `2.0.0` | CI artifacts from run `41882102341` | `[x]` Windows, Linux x86-64, and Linux AArch64 |
| 2 | Package layout, `package.json`, and `SHA256SUMS` verify | all three package checksums verified; each reports `2.0.0`, host ABI 2.3, `frame_sidecar` 2 | `[x]` |
| 3 | Unpacked fresh project: `new`, `check`, `build`, `build --release`, `build --emit=lib`, `run`, `test`, `clean`, rebuild | package jobs in run `41882102341` | `[x]` |
| 4 | VSIX installs against the packaged `stc`; extension smoke passes | local VSIX build + extension/smoke jobs in run `41882102341` | `[x]` |
| 5 | Release artifacts uploaded and URLs/checksums recorded | GitHub release | `[ ]` pending |

## 6. Documentation and limitations

| # | Check | Evidence | Result |
| --- | --- | --- | --- |
| 1 | README, user guide, release notes, checklist, limitations, and non-safety statement are current | full CI `Docs and specification` job | `[x]` |
| 2 | Specification links and Markdown lint pass | `check_docs.py` and `markdownlint-cli2` in CI | `[x]` |
| 3 | Every release limitation is recorded with a workaround or boundary | [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) | `[x]` |
| 4 | Deferred scope is explicit: MQTT, Modbus client, ESI, motion, remote IDE, WebUI | [FutureWishlist.md](FutureWishlist.md), release notes | `[x]` |
| 5 | Non-safety statement is present in docs and debug-facing material | [NON-SAFETY.md](NON-SAFETY.md) | `[x]` |

## 7. Known limitations accepted for this release

- **VS Code is local-only.** No remote IDE, remote source mapping, or remote
  debug attachment; one session per host; release builds are monitoring-only.
- **Modbus TCP is server-only, unauthenticated, and non-real-time.** No client
  or master, no RTU/TLS, no third-party client interoperability claim, and no
  external Modbus-to-EtherCAT cross-wiring evidence.
- **EtherCAT evidence is Linux x86-64 and DIEWU-only.** IgH is an operator
  prerequisite; static configuration only; no servo hardware; no AArch64
  hardware verification; no hard real-time or safety claim.
- **RT results are host-specific.** The untuned campaign is evidence, not a
  guarantee; the tuned boot profile was not executed.
- **Coverage is statement coverage only.** It is not correctness evidence.
- **Formal verification is scoped.** The exact-kernel V1 claim is relative to
  the documented TCB; the rest of the compiler and bridge are not claimed as
  fully verified.
- **No MQTT in 2.0.0.** MQTT, Modbus client, ESI configuration, motion control,
  remote IDE debugging, and the WebUI remain deferred.

See [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) for the full contract and
workarounds.

## 8. Sign-off

| Role | Name | Date | Signature / result |
| --- | --- | --- | --- |
| Release owner | project maintainer (`r2k`) | — | `[ ]` pending |
| Build/CI evidence reviewed | project maintainer (`r2k`) | — | Run `41882102341` (39/39), Aeneas build, three package checksums, and VSIX recorded; owner review pending |
| Documentation reviewed | project maintainer (`r2k`) | — | `[ ]` pending |
| Non-safety statement reviewed | project maintainer (`r2k`) | — | `[ ]` pending |

**Release is not signed off until:**

1. the version is set to `2.0.0` and version-bearing snapshots are regenerated;
2. the final package and extension artifacts are built at that commit and
   verified;
3. the final CI run for the version commit is green;
4. the Aeneas/Charon evidence above is re-run or confirmed against the final
   version commit;
5. every remaining `[ ]` item above is checked with its output recorded.
