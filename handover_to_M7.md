# Handover to M7 — Release Candidate

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-28  
Prepared after M6.5 (debug and monitoring) completion

Read this together with the project plan (`Specification/00-Project-Plan.md` §8, M7–M8),
`Specification/Architecture/Architecture.md` §4–§8,
`Specification/Architecture/Debug-and-Monitoring.md` §1–§18,
`Specification/Architecture/CLI-and-Manifest.md` §1–§4,
`Specification/Architecture/GOT-Artifacts.md` §1–§9,
`Specification/Architecture/Online-Change-and-GOT.md` §2–§11,
`Specification/Architecture/Runtime-and-Execution.md` §1–§8,
`Specification/Architecture/Testing-and-Verification.md` §1–§6,
`Specification/Decisions.md` D-173–D-183, `M6.5-Plan.md`, and `handover_to_M5.md` (the M4.5→M5
input, retained for history).

## 0. Debug and monitoring addendum — final M6.5 handoff (2026-10-28)

**M6.5 is complete and M7 starts on the M6.5 implementation, not on the M6 control surface.**
The debug and monitoring specification is implemented; this document records the state, the
contracts that must not regress, and the work M7 owns.

- Debug is the **default build form**. `--release` selects a separately identified optimised,
  stripped, hook-free module and manifest; the two modes never mix artifacts in one out-dir.
- Debug access is **typed and identity-based**. Clients name roots and fields by stable key or
  `GotId`; no protocol operation exposes a raw address, raw stack pointer, or arbitrary
  memory read/write. Locals are reachable only through the stopped-frame snapshots.
- The debug session is **local and authenticated** (`stc-debug/1`, pipe by default, local
  socket optional). There is no remote/network debugging, no unauthenticated mode, and no
  raw-memory API.
- Every **swap, rollback, cold install, shutdown, or target exit ends the session**; queued
  writes/forces are rejected rather than carried into another generation. `stc` clears forces,
  breakpoints, stop state, and session state before a generation change completes.
- A stopped cycle is bounded by the **stop/idle budget** (`[debug].session-timeout`, default
  five minutes); expiry ends the session and resumes the cycle with a fresh deadline. There is
  no M6 watchdog; the bound is the documented pause policy (D-175).
- Debug output is **deterministic and address-free**: no timestamps, absolute paths, host names,
  user names, or raw addresses in rendered values, split views, sidecars, or manifests. Audit
  records carry protocol time fields by design and are runtime state, not artifacts.
- `stc` remains **not a safety PLC**. Debug can hold outputs indefinitely; no fail-safe output
  timeout, interlock, or SIL claim is provided. M7 documentation must keep that stance explicit.

### 0.1 Implemented M6.5 contracts

M6.5 phases 0–10 are complete. The implementation is the contract M7 extends:

| Crate | Implemented contract |
| --- | --- |
| `stc-got` | GOT manifest format 2 (`MANIFEST_FORMAT = 2`, `stc:manifest:v2`/`stc:catalog:v2` domains) with `type_metadata` and `aggregates`; descriptor format 1 carrying `manifest_format = 2`, with host ABI up to minor 3 (production 2.1, test 2.2, hook-bearing debug 2.3); debug-object format 1 in the metadata sidecar; deterministic binary/JSON manifests and build IDs |
| `stc-codegen` | Deterministic DWARF (ELF) and CodeView-in-object (COFF) debug metadata; source lines, functions, parameters, locals, types, and declaration-only persistent globals; inline `(checkpoint_id, span_id)` checkpoint calls; forced frame pointers and disabled tail calls only in hook-bearing debug builds; ABI 2.3 service 13 import; frame identity/type hints for the stopped-frame sidecar |
| `stc-runtime` | Typed engine (normalization, key/`GotId` resolution, strict reads/writes, enum/subrange validation, split views); queue and generation pinning; breakpoints, stepping, immutable stop snapshots, stopped-frame table loading, x86-64 frame walk; local authenticated server, endpoint file, audit, scopes, rate limits, stop/idle bound; session teardown on swap/shutdown/exit |
| `stc-driver` | `--debug`/`--release` build modes and out-dirs; metadata-sidecar debug object and stopped-frame sidecar extraction; release stripping; `stc debug` launch/attach/batch/interactive CLI; `stc run --debug-server`; debug flag validation |
| `stc-manifest` | `[debug]` schema (`default`, `protocol`, `force-sessions-end-on-swap`, `session-timeout`, `allow-write`, `allow-force`) with `[build].debug` agreement |
| `xtask` / CI | `cargo xtask test --suite debug` (twelve targets); Windows and Ubuntu debug jobs with clang preflight; informational overhead measurement; `Debug overhead` workflow |

### 0.2 What M7 must extend, and what it must not break

M7 is a release candidate: packaging, examples, release notes, cross-target smoke tests, a
performance pass, and hardening. It is not a language or format milestone. Unless a new
decision record says otherwise:

- Do not change module, descriptor, state, metadata, debug-object, or stopped-frame formats,
  manifest format 2, build-ID domains, service IDs, the host ABI, or the `stc-debug/1` wire
  schema. Hardening must preserve them or add a documented compatibility path.
- Do not weaken release cleanliness: `--release` remains stripped, hook-free, and separately
  identified; debug sidecars and checkpoints remain absent from release artifacts.
- Do not weaken the session rules: local authentication, scope grants, audit, pause bound,
  swap teardown, and non-safety notice stay mandatory.
- Do not expose raw memory, addresses, or caller-frame guesses to make stack views look
  complete. Honest `optimized_out`/`available:false` is the contract.
- Extend by adding tests and documentation first; treat every output or artifact change as an
  identity/compatibility decision, not a cosmetic fix.

## 1. State at handover

M6.5 is implemented and complete as of 2026-10-28. Phase 9 closed at `dc84907`
(`M6.5 phase 9 Linux measurements`) on branch `M6.5`; its final CI run is 40698937369, green
on both hosts. The phase-10 documentation and test-only wait are the tip of this handover.
`main` was not advanced by this milestone; `origin/main` remains `324488d`.

| Item | State |
| --- | --- |
| Branch | Local `M6.5`, pushed as `origin/M6.5`; phase 9 tip `dc84907`; phase 10 is this documentation commit |
| CI | `ci.yml` at `dc84907` is green (run 40698937369); the `debug` job runs on windows-2025 and ubuntu-24.04 |
| Workspace | `stc-span`, `stc-diag`, `stc-lexer`, `stc-syntax`, `stc-parser`, `stc-hir`, `stc-sema`, `stc-codegen`, `stc-got`, `stc-manifest`, `stc-runtime`, `stc-test`, `stc-fmt`, `stc-driver`, `xtask`; `fuzz/` is separate |
| Toolchain | Rust 1.99.1, edition 2024; workspace `unsafe_code = "forbid"`; `stc-runtime` is the documented exception |
| Tests | Full workspace suite: 1,052 passed, 0 failed, 2 ignored performance measurements (`debug_overhead`, `switch_latency`); the debug suite is twelve targets and 115 tests. CI also runs online, UI, conformance, execution, project, traceability, Lean, fuzz-smoke, and cross-target jobs |
| Checks | clippy `-D warnings`, `cargo fmt --check`, `check_docs.py`, Markdown lint, and `cargo xtask traceability --check` pass |
| Traceability | 225 rules, 219 implemented, 219 covered, 0 uncovered; four allowlisted IDs are non-debug (`STC-E-ONLINE-STATE-001`, `STC-W-ONLINE-STATE-001`, `STC-W-ONLINE-004`, `STC-W-RUN-001`) |
| Fuzzing | `fuzz_lexer`, `fuzz_parser`, `fuzz_analyzer`, `fuzz_codegen`, `fuzz_got_manifest`, `fuzz_manifest`, `fuzz_module_descriptor`; manifest/descriptor targets never load native code |
| Host tools | Local Windows host: clang/LLVM 23.1.4, `llvm-ar`, `llvm-readobj`, host triple `x86_64-pc-windows-msvc`; CI uses its platform clang and validates the reader preflight |
| Artifacts | `.ll`, `.o`, `<name>-<build-id>.dll`/`.so` module, `.stc-got.json`/`.stc-got.bin` manifest pair, `.stc-meta.json` (including debug object for debug forms), optional `<name>.stc-frames.json`, thin `.exe` launcher, embedding `.lib`/`.a` host |
| Build modes | Debug default (`target/stc/debug`, debug information, checkpoints); `--release` (`target/stc/release`, optimised, stripped, hook-free, distinct IDs/manifests); `stc test` builds hook-free debug at ABI 2.2 |
| Debug interface | `stc debug` (`--attach`, `--endpoint`, `--batch`, `--break`, `--watch`, `--allow-write`, `--allow-force`, `--session-timeout`, `--debug-protocol`) and `stc run --debug-server`; protocol `stc-debug/1` |
| Formats | GOT manifest format 2; descriptor/module/state/metadata formats 1; debug object format 1; stopped-frame sidecar format 1; host ABI major 2 minor 3; build-ID domain `stc:build:v1` |
| Working tree | Phase-10 documentation plus one test-only wait in `debug_cli.rs`; no product behavior, format, ABI, or protocol change |

M6.5 delivered the typed debug and monitoring surface on top of M6: debug/release build modes
and artifact separation; deterministic DWARF/CodeView metadata and checkpoint hooks; manifest
format 2 with type and aggregate metadata; the typed read/write/force/watch engine; breakpoint,
step, and call-stack sessions; the local authenticated server with audit and policy; the
manifest-assisted split-object view; stopped-frame sidecar extraction; the `stc debug` CLI and
`stc run --debug-server`; the two-host debug matrix; and the recorded overhead measurements and
exit-criteria sweep.

## 2. M7 scope and exit criteria

From the project plan:

> Deliverables: performance pass, cross-target smoke tests via `clang --target`, packaging,
> examples, release notes, known-limitations list, manifest/online-change hardening, debug and
> monitoring hardening, and the documented non-safety stance.
>
> Exit criteria:
>
> - No silent acceptance of unsupported features.
> - Documented artifacts and ABI; reproducible IR and manifests; clean build from a fresh checkout.
> - Hot-swap, cold-download, manifest-download, debug, and monitoring suites pass on both hosts.
> - Release checklist signed off.

M7 turns the implemented host into a releasable product. It should not add language surface or
change the artifact model. The recommended boundary:

| Area | M7 | Deferred |
| --- | --- | --- |
| Packaging | Reproducible release layout, debug/release runtime archive, install/version docs, fresh-checkout build | Installer, package-manager integration |
| Performance | Real-time boundary measurements, overhead acceptance discussion, regression capture | Real-time guarantees; hard deadlines |
| Cross-target | `clang --target` smoke tests for IR/object emission; document unsupported execution targets | Linux AArch64 execution and host portability (M8) |
| Examples | End-to-end examples using `new/build/check/run/test/online/debug` | New language features |
| Hardening | Manifest/online-change failure modes, debug/monitoring limits, packaging checks, flake removal | Format changes without a decision record |
| Documentation | Release notes, consolidated known limitations, non-safety statement, user guide completion | Conformance claims beyond the profile |

Anything outside this column should fail with an explicit diagnostic rather than silently
produce a partial release.

## 3. Contracts M6.5 provides and M7 must preserve

### 3.1 Build modes, identity, and artifact separation

- Debug is the default; `--debug` forces it. `--release` selects an optimised, stripped,
  hook-free form. The two modes have distinct build IDs, manifests, and default out-dirs
  (`target/stc/debug`, `target/stc/release`, D-178).
- `[build].debug` is canonical; an explicit `[debug].default` must agree. CLI flags override
  both; both flags together are `STC-E-PROJECT-010`. `stc test` always builds the hook-free
  debug form and rejects `--release`, `--debug-server`, `--debug-protocol`, and server grants.
- Debug-form artifacts carry the optional `debug` object and, for hook-bearing builds, the
  stopped-frame sidecar. Release artifacts carry neither. Hook-free debug (the test form)
  remains ABI 2.2; hook-bearing debug requires 2.3; production modules require 2.1.
- Three distinct manifests/build IDs exist for the same source across release, hook-free
  debug/test, and hook-bearing debug. Do not deduplicate them by name.

### 3.2 Artifact and wire formats

- GOT manifest format 2 is required by descriptor format 1. A descriptor declaring another
  manifest format is rejected (`STC-E-MODULE-001`, "descriptor manifest format ... is not 2"),
  not silently upgraded. Pre-M6.5 baselines therefore require the documented cold download.
- Sidecars are optional, deterministic, format-1 JSON, paired by `build_id`, excluded from
  identity, and never required for module load. A missing stopped-frame table disables
  `stack.list`/`frame.read` with `STC-E-DEBUG-017`; module loading succeeds.
- The debug protocol is `stc-debug/1`: JSON messages, one authenticated session per host,
  local pipe or local socket, 5 s connect/request deadlines, 1 MiB request and 4 MiB
  response/event frames, 100 req/s with burst 200, 256 watches, 1024 breakpoints, 1024 queued
  writes/forces, 1024 aggregate leaves per read, 1024 stack frames per stop.
- The endpoint file is user-only, created exclusively without following symlinks, and is the
  only place the token appears. The audit log is append-only format-1 JSON Lines, user-only,
  fail-closed, and is runtime state.

### 3.3 Session lifecycle and policy

- Scopes are `observe`, `control`, `write`, and `force`; `write`/`force` are never granted by
  default. Protected reads, writes, forces, releases, authentication failures, stops/resumes,
  and policy denials are audited.
- Writes and forces are queued and applied at a cycle boundary, revalidated for generation,
  build, root, key/type/layout, expected value, writability, size, and direct binding. One
  force per `GotId`; a second force replaces and is audited.
- Every generation change ends the session. The host clears forces, breakpoints, stop state,
  and session state before the swap/rollback/cold install/shutdown completes. Reconnect uses
  the endpoint file's refreshed build ID and generation; no state is replayed.
- Stopping is stop-the-world for the cycle thread. Outputs are held structurally because the
  cycle does not return; there is no copied output buffer.
- The stop/idle budget defaults to five minutes and is configurable. Expiry ends the session
  and resumes the cycle with a fresh deadline. There is no watchdog thread; M7 must not assume
  one exists.

### 3.4 Typed access and honest unavailability

- Reads/writes resolve normalized stable keys or `GotId` paths through manifest type metadata.
  Enum and subrange values are validated; inaccessible/optimized-out values are reported as
  unavailable with a diagnostic, never reconstructed from registers or raw memory.
- The split-object view is manifest-assisted: per-field roots, handles, and stable identities;
  no raw struct dump and no address.
- Caller frames report function identity, symbol, and `optimized_out` but no source location;
  only the top frame has a checkpoint source binding. Caller locals are described only where
  the sidecar has a usable whole-function live range.
- Frame extraction covers x86-64 Windows MSVC and Linux only. AArch64 frame rules remain
  deferred and must not be claimed.

### 3.5 Determinism and presentation

- IR, manifests, build IDs, debug metadata, sidecars, and rendered views are deterministic
  functions of source, options, and toolchain. No timestamps, absolute paths, host/user names,
  random IDs, or addresses are embedded.
- JSON/text debug presentations are deterministic and address-free. Audit records are excluded
  from that rule: they carry `monotonic_ns` and an optional `local_date_ns` by design.
- `/Brepro` keeps PE timestamps content-derived. Release stripping preserves the build-ID note,
  descriptor, manifest metadata, and symbols.

### 3.6 Diagnostics and non-safety

- Debug diagnostics are the stable `STC-E-DEBUG-*`/`STC-W-DEBUG-*` families; every debug rule
  is implemented and covered, and no debug rule is allowlisted. The four allowlisted traceability
  IDs are online/run rules, not debug rules.
- `stc` is not a safety PLC. The non-safety notice is printed at session start and in `--help`.
  A stopped or forced controller may hold outputs indefinitely; there is no fail-safe output
  timeout, interlock, redundancy, or SIL claim. M7 release notes and examples must repeat this.

## 4. Known limitations and technical debt M7 owns

### 4.1 Windows debug DLLs have no CodeView and no PDB

`link_module` in `crates/stc-driver/src/build.rs` invokes clang with `-shared` and
`/NOIMPLIB`/`/Brepro` but no `-Xlinker /DEBUG`; `lld-link` therefore does not place CodeView
into the DLL or produce a PDB. COFF objects do carry `.debug$S`/`.debug$T`, and the phase-9
assertion checks the object on both hosts (`crates/stc-driver/tests/debug_release.rs`). Decide
in M7 whether to add `/DEBUG` plus PDB packaging or to document object-only CodeView as the
release contract. Do not claim DLL-level CodeView until both the link and packaging paths are
tested.

### 4.2 Caller frame source/location and typed-local coverage

Only the top frame has a source binding. Caller frames have no source location; caller locals
are present only for functions with a whole-function live range, and otherwise report
`optimized_out` or `available:false` (`crates/stc-runtime/src/stop.rs`,
`crates/stc-runtime/src/frames.rs`). Extraction is x86-64 only; targets outside x86-64 or hosts
without a working `llvm-readobj`/`readelf` omit the sidecar and disable stack/frame reads
(`crates/stc-driver/src/frames.rs`). AArch64 frame rules are an explicit M8 follow-up. M7 may
improve coverage, but must keep unavailable data honest.

### 4.3 Release static embedding archive is not stripped

`strip_release_artifact` (`crates/stc-driver/src/build.rs`) skips archive containers because
`llvm-objcopy` cannot strip `!<arch>` archives; the static host library is a direct copy of the
runtime toolchain archive. Modules, objects, and executables are stripped. Packaging must use a
runtime archive built without debug information when the embedding library itself must be
debug-free, or add an archive-aware step with its own test.

### 4.4 Format-1 manifest migration and online-change compatibility

Only manifest format 2 is accepted; `crates/stc-got/src/descriptor.rs` rejects any other
`manifest_format`, and format-1 manifests are not migrated in place. Upgrading a pre-M6.5
baseline uses the documented cold download. M7 hardening should test the upgrade path and
decide whether a migration/conversion tool is in scope or explicitly unsupported. Online
change between the debug and release forms is not covered by the M6.5 suite; verify it
explicitly rather than assuming a hot swap between differently identified builds.

### 4.5 Measurement variance and non-gating overhead

Phase-9 numbers are measured, not expected, and are not CI gates; host regime moves them
materially. Windows local release measured 1119.43/1131.96 ns/cycle, hook-free 1287.57/1302.40,
hook-bearing 1516.76/1537.81 (8 checkpoints, 28.65 ns/hook, 1.35x/1.15x, hooks 15.1% of the
debug cycle),
session 2772.00/2813.88 (+1255.24 ns/cycle), typed read 4252.52/4283.30 ns. Linux CI release
measured 1518.36/1530.72, hook-free 1555.24/1564.20, hook-bearing 1862.66/1872.20
(38.43 ns/hook, 1.23x/1.02x, hooks 16.5% of the debug cycle), session 3060.49/3082.64
(+1197.83), read
4540.15/4548.34. The Windows CI cross-check measured release 1339.49/1423.45, hook-free
1487.72/1537.72, hook-bearing 1711.39/1788.00, session 3275.98/3358.36, read 4848.69/5027.30.
The M7 performance pass should record its host and regime, keep timing informational unless a
decision makes it a gate, and compare like with like (`crates/stc-driver/tests/debug_overhead.rs`,
`M6.5-Plan.md` phase 9).

### 4.6 Debug CLI endpoint-line test

The phase-8 `debug_cli` launch tests read the endpoint path from stderr; a race between the
endpoint file and the stderr flush could make the test flaky. Phase 10 adds a test-only wait for
the `stc: debug endpoint ` line before reading it (`crates/stc-driver/tests/debug_cli.rs`); no
product behavior changed. The suite still uses real processes and bounded waits, so treat any
future failure as a timing bug to reproduce, not as a reason to widen timeouts blindly.

### 4.7 `Debug overhead` workflow dispatchability

`.github/workflows/debug-overhead.yml` is `workflow_dispatch` only. GitHub requires a workflow
to exist on the default branch before it can be dispatched, so it becomes directly dispatchable
only after merge; meanwhile the same ignored test runs as an informational step in a dispatched
`debug` CI job. Confirm the workflow is listed and dispatchable after M6.5 reaches the default
branch; keep the measurement non-gating.

### 4.8 Local-only debug, no remote/network, no raw memory

The debug server accepts local pipe or local TCP only, authenticates one session per host, and
exposes typed identities. There is no remote/network debug mode, no raw-memory read/write, no
arbitrary register write, and no unauthenticated allowlist. This is a v1 contract, not an
oversight. Any M7 request to add remote access or raw memory is an architecture and security
decision, not a hardening tweak.

### 4.9 Session and resource limits

One session per host; 256 watches per session (50 ms minimum interval); 64 watch batches or
1 MiB of queued watch events (oldest dropped with `STC-W-DEBUG-003`); 1024 breakpoints; 1024
queued writes/forces or 1 MiB; 1024 aggregate leaves per read; 1024 stack frames per stop;
1,000,000 checkpoints or 64 MiB of checkpoint metadata; 100 req/s with burst 200; 20 rate
errors in 60 s closes the endpoint for 60 s. The five-minute default stop/idle bound applies to
both stopped and idle sessions. M7 may document these as product limits; changing them is a
protocol decision.

### 4.10 Remaining phase 0–9 deviations

- The five-authentication-failures-per-connection rule is unreachable because every failure
  closes the connection immediately; the endpoint-wide twenty-failure/60-second refusal is
  implemented and tested. This is a specification ambiguity, not a missing counter.
- The embedding ABI has no debug transport selector, so embedded hosts use `pipe`; `socket` is
  available through `DebugServerConfig` and the CLI. Adding a selector is an ABI decision.
- Unix `local_date_ns` is 0 because no local-time conversion is available without a new
  platform dependency; Windows supplies local time. `monotonic_ns` is process-relative.
- `session.status` is minimal (`active`, `stop:null`, `pause_remaining_ms:null`,
  `breakpoints:0`) and stepping capabilities are advertised by the phase-6 surface; do not
  infer richer status from the phase-5 placeholder.
- A watch added through the service emits its first batch at the next pump, which can precede a
  teardown event on the same connection; this is within the coalescing rules.
- Windows named-pipe connections close with `CloseHandle` only so a final `session-ended` event
  is not discarded; peer-close errors normalize to EOF/disconnect. The audit file uses a DACL
  that shares read access on Windows and mode 0600 on Unix.
- A swap while the cycle is parked cannot be driven through the public `Host` API because
  `cycle_at` holds `&mut Host`; running-swap, client-stop, target-exit, shutdown, and
  pause-timeout paths are tested. The abort wiring exists for hosts that end a generation from
  their own control path.
- The pause-timeout test uses a spin loop to keep a cycle alive; it is a fixture aid, not
  product behavior. No watchdog thread was added.
- The hook-free comparison uses the test build form because the CLI has no hook-free production
  debug mode. Do not present that figure as a supported product configuration.
- `stc debug` launch uses the existing M6 control endpoint with a fresh per-process nonce for
  clean host shutdown; batch mode is presentation/envelope handling only and cannot add protocol
  fields or operations.

## 5. Tests and commands

Run these before any M7 change and on the release candidate:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
$env:STC_REQUIRE_CLANG = "1"
cargo test --workspace --locked
cargo xtask test --suite debug --locked
cargo xtask traceability --check
python tools/ci/check_docs.py
npx --yes markdownlint-cli2@0.24.0
```

The debug suite is:

| Crate | Target |
| --- | --- |
| `stc-runtime` | `debug_access`, `debug_write`, `debug_stop`, `debug_view`, `debug_protocol` |
| `stc-driver` | `debug_cli`, `debug_release`, `frames` |
| `stc-codegen` | `debug_info`, `checkpoints`, `frames` |
| `stc-got` | `metadata_v2` |

The overhead measurement is informational and ignored by default:

```powershell
cargo test -p stc-driver --test debug_overhead --locked -- --ignored --nocapture
```

End-to-end checks:

```powershell
stc new demo
cd demo
stc build
stc build --release
stc debug --break src/main.st:10
stc run --debug-server --cycles 100
stc test
stc online download --release
```

CI expands these into the `docs`, `fmt`, `clippy`, `test`, `coverage`, `online`, `ui`,
`conformance`, `execution`, `projects`, `debug`, `traceability`, `lean`, `fuzz-smoke`, and
`cross-target` jobs, with `debug` running on windows-2025 and ubuntu-24.04 and the `ci`
summary job requiring it. `.github/workflows/debug-overhead.yml` is manual and non-gating.

## 6. Key files and APIs

| Area | Files |
| --- | --- |
| Wire protocol, limits, diagnostics | `Specification/Architecture/Debug-and-Monitoring.md` §12–§18 |
| CLI and `[debug]` schema | `crates/stc-driver/src/cli.rs`, `crates/stc-driver/src/debug.rs`, `crates/stc-manifest/src/model.rs`, `crates/stc-manifest/src/parse.rs`, `Specification/Architecture/CLI-and-Manifest.md` §1.7 and §2.1 |
| Build modes and artifact emission | `crates/stc-driver/src/build.rs` (`link_module`, `link_exe`, `strip_release_artifact`), `crates/stc-driver/src/toolchain.rs`, `crates/stc-codegen/src/lib.rs` (`Options`/`Mode`) |
| Debug metadata and checkpoints | `crates/stc-codegen/src/debug.rs`, `crates/stc-codegen/src/abi.rs`, `crates/stc-codegen/src/module.rs` (`debug_hooks`, checkpoint input) |
| Typed access, writes, watches | `crates/stc-runtime/src/debug.rs`, `crates/stc-got/src/manifest.rs`, `crates/stc-got/src/engine.rs`, `crates/stc-got/tests/metadata_v2.rs` |
| Server, transport, audit, session | `crates/stc-runtime/src/debug_server.rs`, `crates/stc-runtime/src/transport.rs`, `crates/stc-runtime/src/module_info.rs`, `crates/stc-runtime/src/host.rs` |
| Stops, stepping, frames, sidecar | `crates/stc-runtime/src/stop.rs`, `crates/stc-runtime/src/frames.rs`, `crates/stc-driver/src/frames.rs` |
| Split view | `crates/stc-runtime/src/debug_view.rs`, `crates/stc-runtime/tests/debug_view.rs` |
| Runtime tests | `crates/stc-runtime/tests/debug_access.rs`, `debug_write.rs`, `debug_stop.rs`, `debug_view.rs`, `debug_protocol.rs` |
| Driver tests | `crates/stc-driver/tests/debug_cli.rs`, `debug_release.rs`, `debug_overhead.rs`, `frames.rs` |
| Codegen tests | `crates/stc-codegen/tests/debug_info.rs`, `checkpoints.rs`, `frames.rs`, `tests/snapshots/checkpoints.json` |
| CI and suite selection | `.github/workflows/ci.yml` (`debug` job), `.github/workflows/debug-overhead.yml`, `crates/xtask/src/main.rs` |
| Decisions | `Specification/Decisions.md` D-173–D-183 |

Public entry points M7 should reuse: `Host::start_debug_server` and `DebugServerConfig`/
`DebugServerInfo` for embedded hosts; `DebugClient`/`DebugEndpointFile` for tests and tooling;
`stc_codegen::Options` (`debug`, `release`, `test_build`) and `Mode` for build selection;
`cargo xtask test --suite debug` for the matrix.

## 7. Open questions for M7

1. **CodeView/PDB packaging.** Add `-Xlinker /DEBUG` and ship PDBs, or document object-only
   CodeView? Either way, test the linked artifact and update the limitation.
2. **Release embedding archive.** Build a debug-free runtime archive for packaging, or add an
   archive-aware strip step? Decide before calling the release package fully stripped.
3. **Manifest format-1 users.** Keep cold-download-only, or add a migration tool? This is a
   compatibility commitment; decide explicitly.
4. **Overhead acceptance.** Keep timing informational, or set a budget/regression gate? If a
   gate, fix the host, regime, checkpoint count, and variance policy first.
5. **Remote debug / raw memory.** Confirm they remain out of scope for v1, or record an
   architecture/security decision before any implementation.
6. **AArch64 frames.** Confirm AArch64 frame rules and Linux host portability stay M8, unless
   M7 packaging requires a cross-target smoke extension.
7. **Transport selector in the embedding ABI.** Is `pipe`-only for embedded hosts acceptable
   for v1, or does the release need a documented selector?
8. **Release checklist.** Name the owner and artifacts: fresh-checkout build, both-host suite,
   package contents, versioning, checksums, release notes, non-safety statement.
9. **Traceability allowlist.** Keep the four non-debug IDs allowlisted, or close them during
   hardening? Do not add debug IDs to the allowlist.
10. **Flake policy.** Treat debug CLI/process timing failures as bugs with captured logs; do
    not make the suite pass by increasing deadlines without a diagnosis.

## 8. Definition of done for M7

- [ ] Fresh checkout builds with the pinned toolchain and documented clang discovery.
- [ ] `cargo test --workspace --locked`, the debug suite, and all CI jobs pass on both hosts.
- [ ] `--release` packaging is stripped, hook-free, separately identified, and free of debug
      sidecars; the embedding archive decision is implemented or documented.
- [ ] Manifest/online-change upgrade and cold-download paths are tested and documented.
- [ ] Debug/monitoring hardening leaves the wire protocol, formats, ABI, and identity stable
      unless a decision record changes them.
- [ ] Examples, release notes, known-limitations list, and the non-safety stance are published.
- [ ] `check_docs.py`, Markdown lint, traceability, clippy, and fmt are clean.
- [ ] The release checklist is signed off.

## 9. Suggested order

1. Freeze the contract: run the full suite and record the release baseline; do not change
   formats while packaging.
2. Release engineering: fresh-checkout build, out-dir layout, runtime archive, checksums,
   example projects.
3. Documentation: release notes, known limitations, non-safety statement, user guide.
4. Performance and cross-target smoke: record host/regime, compare like with like, keep timing
   informational unless decided otherwise.
5. Hardening: manifest/online-change upgrade paths, debug limits, CodeView/PDB decision, flake
   fixes.
6. Release checklist sign-off; only then portability work begins in M8.

The phase-9 numbers and the exit-criteria evidence are in `M6.5-Plan.md`; the protocol and
diagnostic contracts are normative in `Specification/Architecture/Debug-and-Monitoring.md`.
Where this handover and a specification disagree, the specification and a decision record win.
