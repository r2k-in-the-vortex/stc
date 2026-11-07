# Known limitations — `stc` 2.0.0

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-29; revised 2026-11-04

This list is part of the release contract. Each entry is either an explicit
profile boundary or a tested limitation with a documented workaround.
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
| Hardware I/O backends: the EtherCAT backend and the Modbus TCP server are implemented in v2.0.0; MQTT remains deferred | The v2 package includes the Modbus TCP server and the optional Linux EtherCAT shim; see §8 and §10 for the hardware boundaries |
| Multi-threaded ST task execution is not provided | One cooperative cycle thread executes ST; the control and debug service threads never execute ST |
| `ARRAY [*]` is a conformant-parameter form only | Dynamic locals, globals, and function results are rejected |
| `stc fmt` has one fixed style and no configuration | `--check` reports a diff; post-v1 work adds configuration |
| `VAR_CONFIG` `AT` address overrides are rejected (`STC-E-CODEGEN-001`) | `VAR_CONFIG` initial values for scalar `AT` program fields are applied; declare the address on the variable itself instead of in `VAR_CONFIG` |

## 2. Hosts, targets, and packaging

| Limitation | Status / workaround |
| --- | --- |
| Native hosts are Windows 11 x86-64 MSVC (`x86_64-pc-windows-msvc`), Linux x86-64 GNU (`x86_64-unknown-linux-gnu`), and Linux AArch64 GNU (`aarch64-unknown-linux-gnu`) | Each has its own `stc-2.0.0-<triple>` package; other triples are smoke-tested for IR/object emission only |
| Module loading, execution, and frame extraction are supported on the three native hosts | Cross-target artifacts are not loadable or debuggable off-host; link and run on the target host |
| macOS, Windows ARM64, 32-bit, and big-endian hosts are not supported | Use a supported host, or emit IR/objects only; no support claim is made |
| CPU affinity is not pinned; control and debug service threads run at below-normal priority (`THREAD_PRIORITY_BELOW_NORMAL` on Windows, nice +10 on Linux) as an advisory policy | CPU placement belongs to the deployment's cpuset/scheduler policy; this is not a real-time guarantee |
| There is no installer or package-manager integration | Unpack `stc-<version>-<triple>` and use `bin/stc`; the runtime archives are in `lib/` |
| Cross-target debug information follows the requested target (DWARF for ELF/Mach-O, CodeView for MSVC) | Compile with the same `--target` used to build the IR; do not reuse host debug IR for a foreign target |

## 3. Debug and monitoring

| Limitation | Status / workaround |
| --- | --- |
| Debug is local by default (`pipe`, or loopback socket); remote access is an SSH tunnel to that loopback `socket` endpoint, not a new listener | Use `stc debug --remote <user@host>` or `stc serve` with an SSH client/agent and a `known_hosts` entry. The remote login account must own a packaged `stc` and a dedicated work directory. No TLS beyond SSH; no password prompt or unknown-host acceptance |
| Remote debugging requires OpenSSH on the development host and the same `stc` version on both ends; it has no automatic reconnect | Install a matching packaged `stc` on the host; a version mismatch is `STC-E-REMOTE-002`. After tunnel/host loss, start a new attach and re-apply breakpoints |
| One remote `stc serve` owns a work directory; launch and attach both use it | Stop the existing serve before starting another. Attach leaves it running; launch stops its own serve at teardown |
| The remote source fingerprint covers production source files, not build settings or manifest text | A build-setting change is detected by the manifest comparison/build result, not the source hash alone; `remoteDeploy=ask` still prompts on source drift |
| Remote online/cold deployment ends any active debug session | Reattach after deployment; online preserves compatible retained state, cold follows the runtime policy |
| Each remote candidate build writes a unique `candidate-<nonce>` directory so a later status/build cannot overwrite the active baseline; old candidate directories are not garbage-collected | Remove stale `~/stc-wl12/candidate-*` directories when reclaiming disk. The descriptor names the active build directory, so do not remove that one while its serve is running |
| There is no raw-memory read/write, arbitrary register write, or unauthenticated mode | Use typed identity-based reads/writes and the manifest-assisted split view |
| One debug session per host | Stop the current session before starting another |
| Starting a session does not stop the cycle; a stop/step/breakpoint holds outputs | The opt-in stop/idle bound (`[debug].session-timeout`, disabled by default) resumes the cycle and ends the session when configured; this is not a fail-safe timeout |
| A launch is start-gated only until cycle 0; attach and a host started by `stc run --debug-server` cannot be paused before they run | Use a launch for deterministic cycle-0 stops; attach breakpoints bind at the next checkpoint, and cycles before the attach are not observed |
| Every swap, rollback, cold install, shutdown, or target exit ends the session and clears forces/breakpoints; queued writes/forces are rejected | Reconnect after the generation change |
| Release builds are monitoring-only | Use a debug build for breakpoints, stepping, and full locals; monitor live Globals from the Debug Console |
| Frame extraction covers x86-64 Windows MSVC, x86-64 Linux, and AArch64 Linux and needs `llvm-readobj` or `readelf` at build time | Without a usable sidecar, `stack.list`/`frame.read` report `STC-E-DEBUG-017`; functions compiled without a frame pointer or without a source line map are reported as optimized-out callers or with an absent source, never a fabricated location; module loading and typed monitoring still work |
| Windows ships object-level CodeView (COFF `.debug$S`/`.debug$T`); there is no PDB and no DLL-level CodeView claim | Use the object (or external inspection) for source-level analysis; the runtime uses build-time sidecars, not DWARF/CodeView |
| Session limits: 256 watches, 1024 breakpoints, 1024 queued writes/forces (1 MiB), 1024 aggregate leaves per read, 1024 stack frames per stop, 100 requests/s with burst 200, 5 s connect/request deadlines, 1 MiB request and 4 MiB response/event frames | These are product limits; changing them is a protocol decision. A stopped one-shot write applied immediately on `stc-debug/1.1` does not enter the queue or consume a queue entry |
| Embedded hosts use the `pipe` transport; the embedding ABI has no transport selector | `socket` is available through `DebugServerConfig` and the CLI |
| Unix audit records have `local_date_ns = 0` because no local-time conversion is available without a new platform dependency | `monotonic_ns` is process-relative and always present |
| Persistent globals and program fields use declaration-only debug-info entries because their storage is host-owned GOT memory | DWARF reports their name, type, and source declaration but no storage location; CodeView omits them. Use typed reads and the manifest-assisted view for live values; function, parameter, local, and type debugging is unaffected |

## 4. IDE integration

| Limitation | Status / workaround |
| --- | --- |
| The VS Code extension and both servers are local processes; remote debugging is driven by the Rust driver over SSH | `remote`, `remoteDir`, `remoteStc`, `remoteDeploy`, and `remoteSession` select a remote launch or attach; the extension itself opens no network connection and needs an OpenSSH client, agent, and `known_hosts` entry |
| A remote attach with `remoteDeploy=ask` runs `stc remote status` (sync plus candidate build) before showing the Online/Cold/Cancel prompt | The prompt appears only when sources drifted. Cancel starts no session and changes no running host. Online is offered only when the candidate plan is compatible; otherwise cold is the only deploy choice |
| There is one debug session per host and one adapter session per launch | Stop the current session before starting another; a second session fails honestly |
| Every online swap, rollback, cold install, shutdown, or target exit ends the editor session; breakpoints, forces, and watches do not migrate | Reconnect after the generation change and re-resolve targets |
| Release and hook-free builds are monitoring-only | No breakpoints, stepping, or Locals scope; use the Debug Console for live Globals and target paths, and a debug build for a full session |
| Local names appear in the frame table's uppercase normalized spelling; the source spelling is not retained | Read the uppercase name shown in the Locals scope |
| `evaluate` and watches accept a stopped-frame local name or a manifest target path, not an ST expression | Use a target path such as `MAIN.COUNTER`; `1 + 2` and conditions are rejected |
| Locals requires a `stc-debug/1.1` host with stopped-frame information | A `stc-debug/1`-only host or a build without frame metadata has no Locals scope; Globals and typed monitoring still work |
| Program and FB body frames have no frame-table locals; their fields are instance roots in Globals, so their Locals scope is empty | Expand the program instance in Globals; function frames still list their locals. A frame-to-instance mapping for Locals is not implemented (F-13) |
| The language server reanalyzes the whole project after a 200 ms debounce on open/change/save; there is no incremental analysis | Expect a short delay on large projects; the parse diagnostics appear first and semantic diagnostics replace them |
| An invalid `stc.toml` prevents `stc lsp` from initializing, so no language features are available until the manifest loads | The extension treats this deterministic failure as a configuration error: it does not restart the client, logs the `STC-E-PROJECT-*` detail to the **stc Language Server** output channel, and shows one error notification with **Open Output** and **Retry**. Fix the manifest, then choose **Retry** (or reload the window). Degraded initialization that publishes manifest diagnostics and recovers automatically is not implemented. Unexpected server crashes are restarted up to four times in three minutes, separately from this configuration error |
| A declared source glob that matches no files is `STC-E-PROJECT-004` at project load, including `[test] sources` before any test exists | A project without tests must currently omit `[test]` or add a matching test source. Deferred follow-up F-23 covers allowing a project without tests and auditing other optional project-file items that fail at load |
| The LSP capability set is closed: no semantic tokens, rename, code actions, inlay hints, range formatting, call hierarchy, or workspace symbols | Highlighting comes from the TextMate grammar; use the compiler and `stc rename` outside the editor |
| Highlighting is lexical: declarations and uses are not resolved. User call names and named arguments are colored as variables | Only prelude names receive built-in scopes; pragma-name and direct-variable colors are guaranteed only in the shipped `stc Dark`/`stc Light` themes, and built-in themes may color the pragma name as a keyword |
| Debug writes are disabled by default; `setVariable` works only for writable live roots (globals, program-instance fields, and live split fields) and only when `allowWrite` and `[debug].allow-write` are enabled | Stopped-frame locals are read-only snapshots. `allowForce` requests the force scope, but DAP has no force request, so forcing remains a `stc debug` operation. A write while running takes effect at the next cycle boundary; on `stc-debug/1.1` a write while stopped is applied before the response, while `stc-debug/1` keeps the queued behavior |
| The adapter advertises no conditional, hit-count, log, function, data, or instruction breakpoints; no exception breakpoints, restart, reverse debugging, multi-thread debugging, or `stopOnEntry` | Use source breakpoints and over/into/out stepping; a launch is start-gated and binds breakpoints received before `configurationDone` before cycle 0, while attach binds at the next checkpoint; traps end the session |
| A launch binds before cycle 0, but attach (`stc dap` attach, `stc debug --attach`, or a host started by `stc run --debug-server`) cannot be paused before it runs | Use a launch configuration for first-cycle stops; an attached host may already have executed cycles |
| When `[debug].session-timeout` is enabled, the session names the bound at startup but shows no countdown or pre-expiry warning | Expiry ends the session with `STC-E-DEBUG-018` and resumes the cycle; keep the bound generous or leave it disabled |
| VS Code 1.82 or newer is required, distribution is VSIX-only, and v2.0.0 does not publish to the marketplace or sign the VSIX | Install the VSIX from the release artifacts; there is no telemetry or network access |
| The compiler is not bundled with the extension | Install `stc` and put it on `PATH` or set `stc.path`; the extension logs a missing compiler instead of starting a server |
| The online-change surface runs one session per editor window and has no cross-window coordination | Two windows can start two watches on the same project; the CLI does not lock a store. Stop one before starting another |
| The IDE online-change surface presents the watch only: no cold-download, rollback, migration, write, or force action | Use `stc run --watch`/`stc online` for cold and recovery operations; the extension shows the active generation/build and surfaces the existing swap teardown |
| An online-change stop closes the watch's stdin and waits five seconds; a watch that does not exit is killed as a process tree | A forced stop bypasses normal shutdown, so retained state may not be saved; the extension warns in the channel. Closing the window stops an owned watch the same way |
| The extension reads the online-change generation/build from a private per-session debug endpoint file under extension storage; that file's token, endpoint, and audit path are never read or logged | This is the runtime's existing local user-only contract. Do not expect the file under `target/` or a configurable path; the endpoint path is internal |

## 5. Online change and manifests

| Limitation | Status / workaround |
| --- | --- |
| Online change replaces the whole module and does not migrate field layouts | Add/remove/rename is supported; type, size, array, and string layout changes reset the affected root or require a cold download |
| Candidate validation/load/prepare runs on the cycle thread between scans; fully off-thread preparation is v2 work | The switch itself remains O(1); preparation jitter is bounded and measured |
| GOT manifest format 1 is rejected | The documented cold download is the only upgrade path; there is no migration tool (D-192) |
| Debug and release forms have distinct identities and are not a supported hot-swap pair | Keep separate stores; a release candidate against a debug baseline selects a cold layout, and the reverse direction is classified compatible but is not a supported deployment path |
| M4-era code-linked artifacts are not loadable | Rebuild with the current module model |
| There is no general remote/network deployment UI, multi-user scheduling, or rollback UI; the IDE surface starts/stops the local watch and offers Online/Cold/Cancel only for a remote attach | Online change is a host API/CLI capability; remote attach deployment uses the SSH driver and the host control channel. Use `stc run --watch` and `stc online` for local watch operations |
| `stc run --watch` and the local IDE online-change surface are local; remote control is available only through the WL-1.2 SSH driver | The debug protocol remains a separate loopback endpoint; the extension adds no network path of its own |
| A missing, stale, or corrupt baseline forces a cold download | The runtime exports and downloads the active manifest; never execute manifest content |

## 6. State, I/O, and tests

| Limitation | Status / workaround |
| --- | --- |
| Retained state is saved at normal shutdown; crash-time saves and periodic checkpoints are not provided | Use the documented shutdown path; a cold start reinitializes |
| Per-field retained-state reset policy during online change is not implemented | `STC-E-ONLINE-STATE-001`, `STC-W-ONLINE-STATE-001`, and `STC-W-ONLINE-004` are allowlisted as not implemented; the runtime preserves host memory and rejects incompatible layouts |
| Direct variables in function/FB bodies are rejected as `STC-E-IO-001` rather than accepted with a deprecation warning | `STC-W-RUN-001` is allowlisted as not implemented |
| The test runner is sequential and process-isolated; parallel test threads are not provided | Deterministic output and resource use are deliberate |
| `STC_ASSERT` and `{stc:test}` units are excluded from production builds; production use is an error | Keep test code in `[test].sources` or a separate test package |
| I/O bindings in `stc.toml` are validated against the selected production configuration; a test build uses the synthetic test configuration, so binding paths that name production instances are not available in `stc test` | Use `{stc:io}` pragmas on the direct variable when the project must also test cleanly, or keep bindings in a production-only configuration project |
| Performance figures are informational and host-dependent, not real-time guarantees | See [`specification/Verification/Performance.md`](specification/Verification/Performance.md) and record the host/regime when comparing |

## 7. Traceability and evidence

| Limitation | Status |
| --- | --- |
| Four online-change/state rules and three hardware-warning rules are documented as not implemented: `STC-E-ONLINE-STATE-001`, `STC-W-ONLINE-STATE-001`, `STC-W-ONLINE-004`, `STC-W-RUN-001`, `STC-W-IO-ETH-001`, `STC-W-IO-ETH-002`, `STC-W-IO-MODTCP-001` | In `tools/ci/traceability-allowlist.toml`; no debug rule is allowlisted |
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
| The untuned host does not sustain a 1 ms cycle under the heavy R1 mixed external stress load (execution p50 about 2.6 ms) | The same-CPU lower-class load case is protected by FIFO:90; the tuned profile is deferred future work ([RT-Host.md](specification/Verification/RT-Host.md) §6) |
| The strict `missed_deadline` counter counts any positive wake lateness, so it reports nearly every cycle | Use the execution and wake-lateness percentiles and the skipped-period count; the counter is not a hard-miss count |
| The max wake-lateness columns contain isolated multi-hundred-millisecond samples | The raw data retains the per-cycle records; p99.9 is the tail metric; the maxima are not bounds |
| The untuned noise floor shows osnoise maxima of 8.5–22.8 ms and a 13 µs hwlat maximum | The tuned profile remains deferred future work; the trigger is recorded in [RT-Host.md](specification/Verification/RT-Host.md) §6 |
| Deploy prepare/load/activation runs on the cycle thread; online-change deployment latency is 1.8–4.3 ms in R2 and 14.9–126.3 ms in R1 | This is maintenance latency, not cycle jitter; fully off-thread prepare is post-v1 work |
| RT metrics are buffered and flushed at exit; a hard crash can lose buffered records | Keep runs bounded or accept the loss; the launcher writes a summary on normal exit, stop, or interrupt |
| The control-disabled control-cost leg estimates its warm-up cycle boundary from the cycle count | The error is negligible over a 10 minute run; the control-enabled legs use fixed status boundaries |
| The tuned boot profile was not executed in v2.0.0 | It remains a separate future-work scope decision; no tuned result is claimed |

## 8. EtherCAT I/O (WL-5.2, v2.0.0)

Added 2026-11-01; revised for v2.0.0. The v2.0.0 package carries the optional Linux EtherCAT
backend; the base runtime still has no IgH dependency and no kernel modules are redistributed.
The full evidence is in
[EtherCAT-Host.md](specification/Verification/EtherCAT-Host.md); the procedure and per-run data
are in [EtherCAT-Runbook.md](specification/Verification/EtherCAT-Runbook.md) and
[tests/ethercat/diewu/PHASE6.md](tests/ethercat/diewu/PHASE6.md).

| Limitation | Status / workaround |
| --- | --- |
| Hardware-in-the-loop evidence is Linux x86-64 only (`ecat-host`, kernel `6.12.115-ecat-rt`, IgH 1.6.13) | The backend compiles for Linux AArch64 but is not hardware-verified there; no hard-RT claim |
| HIL uses one DIEWU 16I/16O digital slave at master 0/alias 0/position 0 | Multi-slave configurations are covered by configuration/mapping and mock tests, not hardware |
| No servo-drive hardware was available | The generic cyclic-PDO and distributed-clock path is implemented and unit-tested; the DIEWU runs free-run, so DC was not exercised on hardware |
| Fault C (slave-end unplug) was not run separately; it is covered by Fault A (host-end unplug) because a single direct cable has the same link-loss path | Run the separate case if the harness gains an intermediate device; the configured-slave reconnect behavior is recorded through Fault A |
| Recovery time was not measured, only sampled within the procedure windows; the fault-session counters carry one extra unobserved episode | The behavior and stale-image values are recorded; add a timed recovery measurement if a bound is required. The counter deviation is a manual-sequence record, not a product failure |
| The master lost-frame counter is cumulative; the phase-6 samples are 18856 → 18856 for the all-points run and 18856 → 24426 for the fault session (delta 5570) | The fault-session delta is frames sent during the link-down/power-off windows. Record `ethercat master` before and after each future run |
| The optional per-backend `stc-io-metrics/1` stream from the WL-5.2 design is not implemented | Timing evidence uses `stc-rt-metrics/1` plus `ethercat master` boundary samples; a per-backend stream is deferred |
| The fastest maintainable cycle time is 150 µs on this host; 100 µs fails on host wake jitter (283 skipped periods, wake lateness 53.4–133.3 µs), and 75/50/25 µs were not run | Host- and configuration-specific, not a hard real-time guarantee. Release is the timing evidence; the debug build uses most of a 1 ms budget |
| Mixed real backends in one run were not HIL-tested in WL-5.2 | WL-5.1 ran the DIEWU EtherCAT slave and a Modbus TCP server concurrently in one coordinator for 60000 cycles with zero backend faults, but without an external Modbus↔EtherCAT signal connection; cross-backend routing is proven by the mixed mock test, not by that bench run (see §10) |
| IgH is an operator-installed host prerequisite and is coupled to the kernel | Pin the kernel/IgH pairing in the runbook and rebuild the modules after a kernel update; the repository does not redistribute kernel modules |
| The base `stc` package has no IgH dependency; the backend is an optional `stc-io-ethercat` shim loaded with `dlopen` | The shim loads only when an EtherCAT device is configured; the shim source is committed and the library is dynamically linked LGPL userspace |
| Transparent fault behavior is not a safety function and the status block is not a safety signal | The runtime holds outputs, presents stale inputs, and exposes status; the PLC decides. Provide safety separately as required by law and regulation |

## 9. Coverage (WL-10)

Added 2026-10-31. `stc test --coverage` reports statement coverage for the ST code compiled into
the test artifacts; the model and formats are in
[Testing and Verification](specification/Architecture/Testing-and-Verification.md) §2.6.

| Limitation | Status / workaround |
| --- | --- |
| Statement coverage only; branch, condition, and MC/DC coverage are not measured | Use the statement and line figures; the `stc-coverage/1` protocol is versioned so a later format can be added without changing this one |
| Test timeouts are wall-clock and include instrumentation overhead | Raise `--timeout` for a test that runs near its limit under coverage; statuses and exit codes otherwise do not change |
| A timed-out, crashed, or failed-to-load test contributes no coverage data; failed and trapped tests do contribute what they executed | Investigate the failure with the normal test report; coverage is not a substitute for it |
| Production `PROGRAM` bodies that a test artifact does not link are outside the measured universe | Put the code under test in a package the test build links, or accept that unlinked programs are not measured |
| Dependency entries use package-qualified labels (`<package>/<relative>`), not filesystem paths | Use the label to map the report entry back to the dependency source |
| There is no per-package or per-file coverage filter in v2.0.0 | Filter `coverage.json` in external tooling; all production packages are reported |
| There is no fail-under threshold, coverage gate, or HTML report | Consume `coverage.json` or `lcov.info` in external tooling; see Testing and Verification §2.6 |
| Coverage is not correctness evidence and is not rule-level traceability | Correctness is established by tests and by the rule traceability check; coverage only records executed statements |

## 10. Modbus TCP server I/O (WL-5.1, v2.0.0)

Added 2026-11-02. WL-5.1 implements a Modbus TCP **server** backend in the runtime; it is not a
client, not Modbus RTU, and not a safety function. The contract is in
[I/O Routing and Backends](specification/Architecture/IO-Routing-and-Backends.md) §4.5; the
evidence is in [Modbus-Server.md](specification/Verification/Modbus-Server.md), with the real
mixed bench in [tests/modbus/mixed/RESULTS.md](tests/modbus/mixed/RESULTS.md).

| Limitation | Status / workaround |
| --- | --- |
| Non-real-time and best-effort; no hard deadline, timing certification, or safety claim | Do not use Modbus as an interpolated motion channel or as a safety function; provide safety separately and verify the cycle budget on the target host |
| No authentication, authorization, or encryption; every input-mapped coil/register on every bind is writable by any reachable client | Bind explicit interfaces only, audit the whole `bind` array, and use a trusted/segmented network or host firewall on every bind; connection limits and idle timeouts do not prevent determined denial of service |
| `BOOL` maps only to coils/discrete inputs; a boolean in a register must be a 16-bit channel | Use a 16-bit register channel for 0/1 flags; 1-bit channels on register tables are rejected |
| Addresses are zero-based; vendor `00001`/`10001`/`30001`/`40001` reference numbers are one-based | Subtract one when converting vendor documentation; confirm ambiguous "register 1" wording against a known value or PDU trace. The profile never accepts vendor notation |
| Width and span scope: only 1/8/16/32/64-bit values; 32/64-bit channels span 2/4 registers; partial writes of a wide channel are rejected (0x02) | Use complete-channel writes and the documented byte/word orders; floating point, strings, and arbitrary per-register layouts are not supported |
| Byte/word order support is limited to device `high-first`/`low-first` and per-channel `high-first`/`low-first`/`abcd`/`cdab`/`dcba`/`badc`/index list | Confirm the vendor's register order against a known value; the named 64-bit forms are per-half patterns, and device `low-first` reverses all four words |
| No external Modbus↔EtherCAT cross-wiring evidence. The real mixed bench ran both backends concurrently but did not connect a Modbus client signal to an EtherCAT channel | Cross-backend routing over one process image is proven by the mixed mock test; a physical Modbus-to-EtherCAT (or reverse) transfer is not claimed |
| The optional independent third-party Modbus client check was not run; `mbpoll`, `modpoll`, and `pymodbus` were not installed | All client evidence uses the in-repo stdlib client; third-party interoperability is not claimed |
| Overhead evidence is one Windows x86-64 host with the public `latch_inputs`/`publish_outputs` seam, not a full coordinator scan; no CI timing gate | Results are host-specific and indicative, not bounds; rerun the ignored release harness on the target host |
| A runtime listener failure is retried indefinitely and the backend state stays `Faulted` until every listener is accepting | The PLC must decide what to do when `server<N>.listening` is false; there is no stale-input flag and no fail-safe output state |
| Unbound declared channels are counted in `server<N>.unbound` but `STC-W-IO-MODTCP-001` is not emitted because the `DeviceBackend` interface has no notice channel | Inspect the diagnostics/status; the code is traceability-allowlisted as not implemented |
| The control `Status` JSON has no `io_reports`; only the verbose launcher output (`stc run -v`) prints backend diagnostics | Use `stc run -v` or the optional `modbus-tcp-status` device; extending the control JSON is not part of WL-5.1 |
| On Unix, ports below 1024 (including the standard Modbus port 502 and the example port 567) require `CAP_NET_BIND_SERVICE` or a privileged service manager | Prefer an unprivileged high port or grant the capability deliberately; the standard port is not assumed |
| Modbus TCP client/master, RTU/serial, TLS, gateways, FC8/FC22/FC23/FC43, and broadcast are not implemented | Use the supported server function codes on a TCP network; WL-11 remains the separate client item |
