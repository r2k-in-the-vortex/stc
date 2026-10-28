# Handover to M8 — Linux Host Portability

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-28  
Prepared after M7 (release candidate, `1.0.0-rc.1`)

Read this with the project plan (`Specification/00-Project-Plan.md` §8, M8–M9),
`M7-Plan.md`, `RELEASE-NOTES.md`, `KNOWN-LIMITATIONS.md`, `USER-GUIDE.md`,
`RELEASE-CHECKLIST.md`, `Specification/Architecture/Architecture.md`,
`Runtime-and-Execution.md`, `State-and-Retain.md`, `CLI-and-Manifest.md`, and
`Testing-and-Verification.md`, plus Decisions D-042 and D-188–D-199.

## 1. State at handover

M7 is implemented as `1.0.0-rc.1`. M8 is the next milestone: native Linux
x86-64 and Linux AArch64 builds of `stc` and the runtime, with full suite and
end-to-end CLI coverage. M7 did not add language surface or change module,
descriptor, state, metadata, sidecar, manifest-format-2, host-ABI 2.3,
service-ID, build-ID, or `stc-debug/1` contracts.

| Item | State |
| --- | --- |
| Version | `1.0.0-rc.1` (workspace, driver, descriptors, manifests, package) |
| Branch | M7 work on `M7`; `main` remains the release baseline |
| CI | `ci.yml` adds `package` and runs the `cross-target` suite on Windows and Ubuntu; green run 40777754963 (commit 8dbcbf4) covers the `debug`, `online`, `projects`, `test`, `ui`, `conformance`, `execution`, `traceability`, `lean`, `fuzz-smoke`, `cross-target`, and `package` jobs |
| Tests | Workspace: 1068 passed, 0 failed, 3 ignored (performance, debug overhead, switch latency) at the M7 local baseline; project/online/example/hardening and debug suites are separate CI jobs |
| Package | `cargo xtask package --locked` writes `dist/stc-1.0.0-rc.1-<host>` with `bin`, `lib`, `docs`, `package.json`, and `SHA256SUMS`; the unpacked layout builds/runs/tests a fresh project |
| Formats | Manifest/catalog 2; descriptor/module/state/metadata/debug-object/frames 1; host ABI 2.3 (test 2.2, production 2.1); protocol `stc-debug/1` |
| Release docs | `RELEASE-NOTES.md`, `USER-GUIDE.md`, `KNOWN-LIMITATIONS.md`, `NON-SAFETY.md`, `RELEASE-CHECKLIST.md`, `Specification/Verification/Performance.md` |
| Safety | `stc` can control real machinery; it is not a safety PLC and safety must be provided by a separate system and machine design as required by law and regulation |

## 2. M8 scope and exit criteria

From the project plan:

> Deliverables: host abstraction audit; native builds of `stc` and the runtime for Linux x86-64
> and Linux AArch64; clang/linker discovery and artifact naming on Linux; state-file atomicity on
> Linux; module loading, thread priority/affinity, and debug-info generation on Linux; CI jobs on
> both hosts; host-support documentation.
>
> Exit criteria:
>
> - `cargo test --workspace` passes natively on Linux x86-64 and Linux AArch64.
> - `stc new/build/check/run/test/clean` work end-to-end on both hosts.
> - The execution suite, conformance corpus, hot-swap suite, manifest tests, and debug suite pass
>   on both hosts; retained state round-trips.
> - No host-specific conditionals remain in `stc-lexer`, `stc-parser`, `stc-sema`, `stc-codegen`,
>   or the module/manifest model.

## 3. Contracts M7 provides and M8 must preserve

- Package layout and runtime adjacency search: `bin/stc` plus `lib/<runtime>` (D-189). The
  explicit `--runtime`/`STC_RUNTIME`/manifest precedence is unchanged.
- Version and identity: `1.0.0-rc.1`; debug/release/test forms have distinct build IDs and
  manifests; debug is the default; release is stripped and hook-free.
- Archive stripping: unpack, `llvm-objcopy --strip-debug` each object, replace in place with
  `llvm-ar rD`, preserving import-library members and symbols (D-191). On Linux the same rule
  applies to `libstc_runtime.a`; the shared library is stripped directly.
- Format-1 manifests are cold-download only (D-192); cross-mode debug/release swaps are not a
  supported hot-swap pair; keep stores separate.
- Object-only Windows CodeView, no PDB (D-190). Linux uses DWARF; M8 must verify AArch64 debug
  info and frame rules rather than assuming x86-64 behaviour.
- Debug remains local, authenticated, typed, and raw-memory-free (D-194); one session per host;
  swap/rollback/cold/shutdown/exit ends the session; the stop/idle bound is mandatory.
- Performance numbers are informational (D-193); do not turn them into gates without a decision.
- Flaky timing tests are bugs (D-198); diagnose before widening deadlines.
- The four non-debug traceability rules remain documented as not implemented (D-199); no debug
  rule is allowlisted.

## 4. Known limitations M8 inherits or owns

1. **Linux host support is CI-only until M8.** `test`, `online`, `debug`, `projects`,
   `cross-target`, and `package` jobs run on `ubuntu-24.04`; native AArch64 is untested.
2. **AArch64 frame rules** are deferred; `stack.list`/`frame.read` are x86-64 only. M8 must add
   AArch64 unwinding or keep the honest `STC-E-DEBUG-017` behaviour and document it.
3. **AArch64 execution** is not claimed. The cross-target matrix proves IR/object emission only.
4. **Thread priority/affinity** and maintenance-thread policy are Windows-shaped or absent; M8
   owns the Linux policy and measurement.
5. **State-file atomicity** is tested on Windows; M8 must verify rename/replace semantics, user-only
   permissions, and state round-trips on Linux (and AArch64).
6. **Module loading** uses the platform layer; M8 verifies `dlopen`/`dlsym`, unload/grace, and
   path handling on Linux and AArch64.
7. **Debug endpoint security** uses Windows DACLs and Unix mode 0600 in a 0700 directory; M8
   verifies the Unix path on AArch64 and container environments.
8. **Prepare remains on the cycle thread**; fully off-thread preparation is post-v1 (not M8 unless
   a decision says otherwise).
9. **Format-1 migration, field-layout migration, remote debug, raw memory, hardware I/O, and a
   registry** remain out of v1 scope.
10. **Release package** is an archive, not an installer; M8 adds Linux package jobs for both
    architectures but not package-manager integration.

## 5. Commands to start M8

```sh
# On each native host:
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
STC_REQUIRE_CLANG=1 cargo test --workspace --locked
cargo xtask test --suite debug --locked
cargo xtask test --suite projects --locked
cargo xtask test --suite cross-target --locked
cargo xtask traceability --check
python3 tools/ci/check_docs.py
cargo xtask package --locked --out dist
```

Then exercise the unpacked package:

```sh
package="$(ls -d dist/stc-*)"
"$package/bin/stc" new fresh
cd fresh
"$package/bin/stc" build
"$package/bin/stc" build --release
"$package/bin/stc" run --cycles 2
"$package/bin/stc" test
"$package/bin/stc" debug --batch /dev/stdin <<'EOF'
quit
EOF
```

## 6. Key files

| Area | Files |
| --- | --- |
| Packaging | `crates/xtask/src/main.rs` (`package`, `strip_runtime_archive`, checksum/JSON writers) |
| Runtime discovery / build | `crates/stc-driver/src/build.rs` (`find_runtime`, `strip_archive`, `link_module`, `link_exe`) |
| Host platform layer | `crates/stc-runtime/src/platform.rs`, `loader.rs`, `transport.rs`, `state.rs`, `control.rs` |
| Debug frames | `crates/stc-driver/src/frames.rs`, `crates/stc-runtime/src/frames.rs`, `stop.rs` |
| CI | `.github/workflows/ci.yml` (`cross-target`, `package`, `debug`, `online`, `projects`), `.github/workflows/debug-overhead.yml` |
| Tests | `crates/stc-driver/tests/{cross_target,package,examples,manifest_hardening,debug_release,performance}.rs`; `crates/stc-runtime/tests/*` |
| Release docs | `RELEASE-NOTES.md`, `USER-GUIDE.md`, `KNOWN-LIMITATIONS.md`, `NON-SAFETY.md`, `RELEASE-CHECKLIST.md` |
| Decisions | `Specification/Decisions.md` D-042, D-188–D-199 |

## 7. Open questions for M8

1. **AArch64 frame rules:** implement unwinding, or keep stack/frame reads unavailable with a
   documented diagnostic? Decide before claiming AArch64 debug parity.
2. **Thread priority/affinity:** which Linux policy (nice, SCHED_FIFO, affinity) is in scope, and
   how is it measured without a real-time claim?
3. **State atomicity:** rename + `fsync` policy on Linux/AArch64; test power-loss-shaped failures?
4. **Module unload grace:** verify `dlclose`/generation pinning and the documented unload after
   grace on both Linux architectures.
5. **Packaging:** one Linux package per architecture, or a combined archive with both runtimes?
   Keep `package.json` host-specific unless a decision changes it.
6. **CI runners:** native ARM64 availability and the pinned runner image policy; do not silently
   move `-latest`.
7. **Cross-compilation:** is building `stc` for the other architecture in scope, or native builds
   only as the plan says?
8. **Debug transport on containers/headless hosts:** confirm the pipe/socket behaviour and the
   user-only endpoint directory rules.

## 8. Definition of done for M8

- [ ] Native Linux x86-64 and Linux AArch64 builds of `stc` and the runtime.
- [ ] `cargo test --workspace --locked` passes on both.
- [ ] `stc new/build/check/run/test/clean` works end-to-end on both.
- [ ] Execution, conformance, hot-swap, manifest, package, and debug suites pass on both.
- [ ] Retained state round-trips; atomic save and permissions are verified.
- [ ] AArch64 debug/frame behaviour is implemented or documented as unavailable.
- [ ] No host-specific conditionals in the portable compiler crates.
- [ ] Host-support documentation, known limitations, and CI jobs are updated.

Where this handover and a specification disagree, the specification and a decision record win.
