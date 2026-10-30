# Known limitations — `stc` 1.0.0

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-29

This list is part of the release contract. Each entry is either an explicit
v1 profile boundary or a tested limitation with a documented workaround.
Anything not listed here and not described in
[`specification/01-Conformance-Profile.md`](specification/01-Conformance-Profile.md)
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
| `VAR_CONFIG` `AT` address overrides are rejected (`STC-E-CODEGEN-001`) | `VAR_CONFIG` initial values for scalar `AT` program fields are applied; declare the address on the variable itself instead of in `VAR_CONFIG` |

## 2. Hosts, targets, and packaging

| Limitation | Status / workaround |
| --- | --- |
| Native hosts are Windows 11 x86-64 MSVC (`x86_64-pc-windows-msvc`), Linux x86-64 GNU (`x86_64-unknown-linux-gnu`), and Linux AArch64 GNU (`aarch64-unknown-linux-gnu`) | Each has its own `stc-1.0.0-<triple>` package; other triples are smoke-tested for IR/object emission only |
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
| Persistent globals and program fields use declaration-only debug-info entries because their storage is host-owned GOT memory | DWARF reports their name, type, and source declaration but no storage location; CodeView omits them. Use typed reads and the manifest-assisted view for live values; function, parameter, local, and type debugging is unaffected |

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
| Performance figures are informational and host-dependent, not real-time guarantees | See [`specification/Verification/Performance.md`](specification/Verification/Performance.md) and record the host/regime when comparing |

## 6. Traceability and evidence

| Limitation | Status |
| --- | --- |
| Four non-debug rules are documented as not implemented: `STC-E-ONLINE-STATE-001`, `STC-W-ONLINE-STATE-001`, `STC-W-ONLINE-004`, `STC-W-RUN-001` | In `tools/ci/traceability-allowlist.toml`; no debug rule is allowlisted |
| Timing measurements are not CI gates | The `performance` and `debug_overhead` tests are ignored by default and run explicitly |
| `Debug overhead` workflow dispatch becomes available only after the workflow exists on the default branch | Run the same ignored test locally or in a dispatched `debug` job meanwhile |
| Flaky process/timing tests are treated as bugs, not fixed by widening deadlines | See D-198 and [`RELEASE-CHECKLIST.md`](RELEASE-CHECKLIST.md) |

## 7. Real-time measurement (WL-4)

Added 2026-10-30. The full evidence is in
[`specification/Verification/RT-Host.md`](specification/Verification/RT-Host.md) and the per-run
detail is in
[`specification/Verification/RT-Run-Manifest.md`](specification/Verification/RT-Run-Manifest.md).

| Limitation | Status / workaround |
| --- | --- |
| The WL-4 figures describe one untuned host and configuration; they are not a hard real-time guarantee and not a safety function | Use them as host-specific evidence; rerun the campaign per host and configuration |
| The untuned host does not sustain a 1 ms cycle under the heavy R1 mixed external stress load (execution p50 about 2.6 ms) | The same-CPU lower-class load case is protected by FIFO:90; the tuned profile is deferred future work ([WL-4-Plan.md](WL-4-Plan.md) §6.2) |
| The strict `missed_deadline` counter counts any positive wake lateness, so it reports nearly every cycle | Use the execution and wake-lateness percentiles and the skipped-period count; the counter is not a hard-miss count |
| The max wake-lateness columns contain isolated multi-hundred-millisecond samples | The raw data retains the per-cycle records; p99.9 is the tail metric; the maxima are not bounds |
| The untuned noise floor shows osnoise maxima of 8.5–22.8 ms and a 13 µs hwlat maximum | The tuned profile remains deferred future work; the trigger is recorded in RT-Host.md §6 and [WL-4-Plan.md](WL-4-Plan.md) §6.2 |
| Deploy prepare/load/activation runs on the cycle thread; online-change deployment latency is 1.8–4.3 ms in R2 and 14.9–126.3 ms in R1 | This is maintenance latency, not cycle jitter; fully off-thread prepare is post-v1 work |
| RT metrics are buffered and flushed at exit; a hard crash can lose buffered records | Keep runs bounded or accept the loss; the launcher writes a summary on normal exit, stop, or interrupt |
| The control-disabled control-cost leg estimates its warm-up cycle boundary from the cycle count | The error is negligible over a 10 minute run; the control-enabled legs use fixed status boundaries |
| The tuned boot profile was not executed in v2.0.0 | It remains a separate future-work scope decision; no tuned result is claimed |
