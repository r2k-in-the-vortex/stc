# Known limitations — `stc` 1.0.0-rc.1

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-29

This list is part of the release contract. Each entry is either an explicit
v1 profile boundary or a tested limitation with a documented workaround.
Anything not listed here and not described in
[`Specification/01-Conformance-Profile.md`](Specification/01-Conformance-Profile.md)
must either work or produce an explicit diagnostic; silent acceptance of an
unsupported feature is a bug.

Read [`NON-SAFETY.md`](NON-SAFETY.md) first: `stc` may control real machinery,
but it is not a safety PLC and safety must be provided by a separate system and
machine design as required by law and regulation.

## 1. Language and profile

| Limitation | Status / workaround |
| --- | --- |
| IEC 61131-3:2025 (Ed. 4) is not implemented or claimed | Profile is 2013 plus the `STC-UTF8`, `STC-PRAGMA`, `STC-TEST`, and `STC-POINTER` extensions. Ed. 4 syntax is rejected with a diagnostic |
| LD, FBD, SFC, and IL are not input languages | Only Structured Text is parsed |
| Vendor project import (TIA Portal, CODESYS), the SIMATIC AX preprocessor, and vendor pragma semantics are not supported | Use plain ST against this profile |
| A registry, version solver, or network package resolution is not provided | Local path dependencies plus a committed `stc.lock`; `--locked` is enforced in CI |
| `PERSISTENT` is not a keyword; periodic or crash-time retained-state saves are not provided | `RETAIN` is saved at normal shutdown only |
| Hardware I/O backends (EtherCAT, Modbus, MQTT) are v2 work | v1 uses `null`, `memory`, and `file` backends |
| Multi-threaded ST task execution is not provided | One cooperative cycle thread executes ST; the control and debug service threads never execute ST |
| `ARRAY [*]` is a conformant-parameter form only | Dynamic locals, globals, and function results are rejected |
| `stc fmt` has one fixed style and no configuration | `--check` reports a diff; post-v1 work adds configuration |

## 2. Hosts, targets, and packaging

| Limitation | Status / workaround |
| --- | --- |
| Native hosts are Windows 11 x86-64 MSVC (`x86_64-pc-windows-msvc`), Linux x86-64 GNU (`x86_64-unknown-linux-gnu`), and Linux AArch64 GNU (`aarch64-unknown-linux-gnu`) | Each has its own `stc-1.0.0-rc.1-<triple>` package; other triples are smoke-tested for IR/object emission only |
| Module loading, execution, and frame extraction are supported on the three native hosts | Cross-target artifacts are not loadable or debuggable off-host; link and run on the target host |
| macOS, Windows ARM64, 32-bit, and big-endian hosts are not supported | Use a supported host, or emit IR/objects only; no support claim is made |
| CPU affinity is not pinned; control and debug service threads run at below-normal priority (`THREAD_PRIORITY_BELOW_NORMAL` on Windows, nice +10 on Linux) as an advisory policy | CPU placement belongs to the deployment's cpuset/scheduler policy; this is not a real-time guarantee |
| There is no installer or package-manager integration | Unpack `stc-<version>-<triple>` and use `bin/stc`; the runtime archives are in `lib/` |
| Cross-target debug information follows the requested target (DWARF for ELF/Mach-O, CodeView for MSVC) | Compile with the same `--target` used to build the IR; do not reuse host debug IR for a foreign target |

## 3. Debug and monitoring

| Limitation | Status / workaround |
| --- | --- |
| Debug is local only (`pipe` by default, loopback socket optional); there is no remote/network mode | Run the debugger on the host; local authentication, scopes, and audit are mandatory |
| There is no raw-memory read/write, arbitrary register write, or unauthenticated mode | Use typed identity-based reads/writes and the manifest-assisted split view |
| One debug session per host | Stop the current session before starting another |
| Starting a session does not stop the cycle; a stop/step/breakpoint holds outputs | The 5-minute default stop/idle bound resumes the cycle and ends the session; this is not a fail-safe timeout |
| Every swap, rollback, cold install, shutdown, or target exit ends the session and clears forces/breakpoints; queued writes/forces are rejected | Reconnect after the generation change |
| Release builds are monitoring-only | Use a debug build for breakpoints, stepping, and full locals |
| Caller frames have no source location; caller locals appear only where the stopped-frame sidecar has a whole-function live range | Only the top frame has a checkpoint source binding; unavailable values are reported honestly, never reconstructed |
| Frame extraction covers x86-64 Windows MSVC, x86-64 Linux, and AArch64 Linux and needs `llvm-readobj` or `readelf` at build time | Without a usable sidecar, `stack.list`/`frame.read` report `STC-E-DEBUG-017`; functions compiled without a frame pointer are reported as optimized-out callers; module loading and typed monitoring still work |
| Windows ships object-level CodeView (COFF `.debug$S`/`.debug$T`); there is no PDB and no DLL-level CodeView claim | Use the object (or external inspection) for source-level analysis; the runtime uses build-time sidecars, not DWARF/CodeView |
| Session limits: 256 watches, 1024 breakpoints, 1024 queued writes/forces (1 MiB), 1024 aggregate leaves per read, 1024 stack frames per stop, 100 requests/s with burst 200, 5 s connect/request deadlines, 1 MiB request and 4 MiB response/event frames | These are product limits; changing them is a protocol decision |
| Embedded hosts use the `pipe` transport; the embedding ABI has no transport selector | `socket` is available through `DebugServerConfig` and the CLI |
| Unix audit records have `local_date_ns = 0` because no local-time conversion is available without a new platform dependency | `monotonic_ns` is process-relative and always present |

## 4. Online change and manifests

| Limitation | Status / workaround |
| --- | --- |
| Online change replaces the whole module and does not migrate field layouts | Add/remove/rename is supported; type, size, array, and string layout changes reset the affected root or require a cold download |
| Candidate validation/load/prepare runs on the cycle thread between scans; fully off-thread preparation is v2 work | The switch itself remains O(1); preparation jitter is bounded and measured |
| GOT manifest format 1 is rejected | The documented cold download is the only upgrade path; there is no migration tool (D-192) |
| Debug and release forms have distinct identities and are not a supported hot-swap pair | Keep separate stores; a release candidate against a debug baseline selects a cold layout, and the reverse direction is classified compatible but is not a supported deployment path |
| M4-era code-linked artifacts are not loadable | Rebuild with the current module model |
| There is no remote/network deployment, multi-user scheduling, or deploy/rollback UI | v1 online change is a local host API/CLI capability |
| `stc run --watch` and the control channel are local only | The debug protocol is a separate local endpoint |
| A missing, stale, or corrupt baseline forces a cold download | The runtime exports and downloads the active manifest; never execute manifest content |

## 5. State, I/O, and tests

| Limitation | Status / workaround |
| --- | --- |
| Retained state is saved at normal shutdown; crash-time saves and periodic checkpoints are not provided | Use the documented shutdown path; a cold start reinitializes |
| Per-field retained-state reset policy during online change is not implemented | `STC-E-ONLINE-STATE-001`, `STC-W-ONLINE-STATE-001`, and `STC-W-ONLINE-004` are allowlisted as not implemented; v1 preserves host memory and rejects incompatible layouts |
| Direct variables in function/FB bodies are rejected as `STC-E-IO-001` rather than accepted with a deprecation warning | `STC-W-RUN-001` is allowlisted as not implemented |
| The test runner is sequential and process-isolated; parallel test threads are not provided | Deterministic output and resource use are deliberate |
| `STC_ASSERT` and `{stc:test}` units are excluded from production builds; production use is an error | Keep test code in `[test].sources` or a separate test package |
| I/O bindings in `stc.toml` are validated against the selected production configuration; a test build uses the synthetic test configuration, so binding paths that name production instances are not available in `stc test` | Use `{stc:io}` pragmas on the direct variable when the project must also test cleanly, or keep bindings in a production-only configuration project |
| Performance figures are informational and host-dependent, not real-time guarantees | See [`Specification/Verification/Performance.md`](Specification/Verification/Performance.md) and record the host/regime when comparing |

## 6. Traceability and evidence

| Limitation | Status |
| --- | --- |
| Four non-debug rules are documented as not implemented: `STC-E-ONLINE-STATE-001`, `STC-W-ONLINE-STATE-001`, `STC-W-ONLINE-004`, `STC-W-RUN-001` | In `tools/ci/traceability-allowlist.toml`; no debug rule is allowlisted |
| Timing measurements are not CI gates | The `performance` and `debug_overhead` tests are ignored by default and run explicitly |
| `Debug overhead` workflow dispatch becomes available only after the workflow exists on the default branch | Run the same ignored test locally or in a dispatched `debug` job meanwhile |
| Flaky process/timing tests are treated as bugs, not fixed by widening deadlines | See D-198 and [`RELEASE-CHECKLIST.md`](RELEASE-CHECKLIST.md) |
