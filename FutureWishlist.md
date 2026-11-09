# Future Wishlist — post-v2.0.0

Status: `stc` 2.0.0 is released (2026-11-04). The v3.0.0 target set is locked in
[v3.0.0-plan.md](v3.0.0-plan.md): WL-1.2, WL-7, WL-14, WL-15, WL-16, WL-17, and WL-18 (F-19).
The remaining candidates are WL-6, WL-8, WL-9, WL-11, WL-12, WL-13, WL-19, WL-20, and WL-21;
they stay here for a later release. This file is deliberately not a roadmap, a commitment, or a
release plan. The delivered v2.0.0 scope and evidence are in [RELEASE-NOTES.md](RELEASE-NOTES.md) and
[RELEASE-CHECKLIST.md](RELEASE-CHECKLIST.md).

- Date: 2026-10-29; revised 2026-11-09 (compiler AI friendliness candidate added)
- Branch: `v3.0.0-planning`
- Baseline: `stc` 2.0.0, released 2026-11-04; profile `stc-2013-oop-utf8-v1`

## 1. Purpose

This file records candidate directions for releases after v2.0.0, while they are still ideas.
It is deliberately not a roadmap, a commitment, or a release plan.

The v3.0.0 target set was picked from this list and locked in
[v3.0.0-plan.md](v3.0.0-plan.md) on 2026-11-04. Items that were not picked stay here for a
later release. Append new entries with the next `WL-` id and change an entry's status rather
than deleting it.

## 2. Ground rules

These apply to every candidate until a future target set is locked.

- **The released v2.0.0 artifact and language profile are the compatibility baseline.** Future
  work is additive around the released compiler, runtime, artifact formats, and language profile
  unless a selected item explicitly says otherwise.
- **No core language or compiler feature work.** This wishlist contains no new ST language
  features, no codegen redesign, and no runtime architecture rewrite. Compiler changes are
  limited to bug fixes and to instrumentation required by a selected item (for example a stable
  IDE API or a conformance HIR dump); any such change is specified and reviewed as part of that
  item.
- **Evidence over assertion.** Verification claims use the V0–V4 ladder in the
  [Verification Program Plan](specification/Verification/VerificationProgramPlan.md). Performance claims record
  host, kernel, build form, metric, and sample. `stc` remains not a safety PLC and makes no
  safety claim; see [NON-SAFETY.md](NON-SAFETY.md).
- **The PREEMPT_RT host is a run target, not a build host.** Artifacts are built on development
  or CI hosts and copied to it. No builds, package installs, or repository checkouts that
  compile happen there.
- **Wishlist entries are not commitments.** Scope sketch, size, and open questions are for
  comparison, not promises.

## 3. Selection and locking

All entries start as `Candidate`. The v3.0.0 target set was locked on 2026-11-04 in
[v3.0.0-plan.md](v3.0.0-plan.md). The completed v2.0.0 target set and its evidence remain in
[RELEASE-NOTES.md](RELEASE-NOTES.md) and [RELEASE-CHECKLIST.md](RELEASE-CHECKLIST.md).

| Status | Meaning |
| --- | --- |
| `Candidate` | Recorded and comparable; no release commitment |
| `Selected` | Chosen for a future release; not yet locked |
| `Locked` | Committed as a target; the target set is fixed and detailed planning can begin |
| `Deferred` | Considered but not selected; remains on the list |
| `Dropped` | Not a direction for this project; keep the rationale in the entry |

Selection criteria:

1. **Product value.** Does it move `stc` closer to a practical controller or development
   environment, rather than improving something that already works?
2. **Risk retired.** Does it reduce the chance of a silent wrong answer, an unsafe operating
   assumption, or an unsupported deployment?
3. **Dependency fit.** Does it use existing contracts (process image, metadata, debug protocol,
   verification registry) instead of duplicating them? Does it unblock other items?
4. **Verifiability.** Can the item state exit criteria and evidence that are testable, measurable,
   or machine-checked?
5. **Effort.** Is the proposed slice bounded, or is the item an open-ended program that needs a
   selected phase rather than a whole?

Locking process:

1. Pick one item or a coherent group and mark it `Selected`.
2. When target selection is complete, lock the target set and mark each chosen item `Locked`.
3. After the target set is locked, write and review the detailed plans: goal, scope and
   non-goals, work breakdown, exit criteria, evidence, risks, effort, and dependencies.
4. After the target set is locked, adding or removing a target is an explicit re-plan, not an
   edit to the locked set.

Tentative size classes follow the [Verification Program Plan](specification/Verification/VerificationProgramPlan.md):
S is up to one engineer-month, M is one to three, L is three to six, and XL is six to twelve or
more. They are order-of-magnitude estimates for comparison only.

## 4. Summary

The delivered v2.0.0 targets are intentionally absent from this table; their scope and evidence
are in [RELEASE-NOTES.md](RELEASE-NOTES.md). Entries marked `Locked` are the v3.0.0 target set
in [v3.0.0-plan.md](v3.0.0-plan.md); the remaining entries are deferred candidates for a later
release. Dependencies that name WL-1.1, WL-3, WL-4, WL-5.1, or WL-5.2 refer to delivered v2.0.0
components.

| ID | Candidate | Area | Tentative size | Key dependency | Status |
| --- | --- | --- | --- | --- | --- |
| WL-1.2 | Remote IDE debugging | Tooling/network | M/L | WL-1.1; network security | Complete (v3.0.0, 2026-11-07) |
| WL-7 | ESI-based EtherCAT device configuration | Runtime/IO | M/L | WL-5.2 | Locked |
| WL-14 | macOS native support and Windows/macOS soft-real-time scheduling for tests | Test platform/RT | M/L | WL-4 harness; CI runners; macOS parity | Complete (v3.0.0, 2026-11-09) |
| WL-15 | MQTT client for process-image I/O | Runtime/IO | M/L | WL-5.1/WL-5.2 backend contract; external broker | Locked |
| WL-16 | Debug session lifecycle: start-paused binding and exit status | Tooling/debug | S/M | WL-1.1 debug protocol; runtime launch | Complete |
| WL-17 | Manifest and task follow-ups from v2.0.0 | Tooling/IDE | S/M | WL-1.1 delivered; manifest/task contracts | Locked |
| WL-18 | Language-server rename (F-19) | Tooling/IDE | M | WL-1.1 delivered; reference resolution | Locked |
| WL-6 | Motion controller: coordinated multi-axis interpolated motion | Motion | L/XL | WL-5.2, WL-7, WL-4; robot-sim reuse | Deferred |
| WL-8 | Tracing UI to watch and graph PLC variables | Tooling/observability | M/L | WL-9 WebUI | Deferred |
| WL-9 | WebUI for basic runtime control | Tooling/operations | M/L | Runtime control API; network security | Deferred |
| WL-11 | Modbus TCP client capability; separate from WL-5.1 server | Runtime/IO | M/L | WL-5.1 backend contract | Deferred |
| WL-12 | Reduce cycle-thread wake-up jitter at sub-millisecond cycle times | Runtime/RT | M/L | WL-5.2 CT-sweep evidence; RT host | Deferred |
| WL-13 | Lean security verification for the Modbus TCP server boundary | Verification/security | L/XL | WL-3 Lean infrastructure; WL-5.1 server | Deferred |
| WL-19 | Microcontroller support and a tiny runtime variant | Runtime/platform | L/XL | Existing IR/object cross-target matrix; bare-metal toolchain | Candidate |
| WL-20 | Optional C backend for microcontrollers outside LLVM | Compiler/backend | XL | WL-19 tiny-runtime contract; vendor C toolchains; ground-rule exception | Candidate |
| WL-21 | Compiler AI friendliness: benchmark-first MCP surface for ST | Tooling/AI | XL | Existing CLI, diagnostics, lossless CST, IDE queries, I/O and online-change contracts; benchmark harness | Candidate |

## 5. Candidate items

### WL-1.2 — Remote IDE debugging

**Intent.** Extend the delivered local IDE debugging path (WL-1.1) to a runtime on another
host: connect VS Code to a remote `stc` runtime over the network with authentication, transport
security, and source mapping. This is a separate task.

**Status.** Complete for v3.0.0 (2026-11-07); manually checked and approved; merged into
`v3.0.0-planning` (PR #25, merge `d74bc89`). Final CI run
[42397941737](https://github.com/r2k-in-the-vortex/stc/actions/runs/42397941737), 39/39 jobs. Locked for v3.0.0 (2026-11-04); previously deferred (2026-10-29). The v2.0.0 WL-1.1
follow-up F-21 (always-on runtime and attach-time deployment choice) is folded into this item
rather than tracked separately.

**Why it matters.** The PREEMPT_RT host (WL-4) and deployed controllers are run-only targets, not
development machines. A developer needs to set breakpoints, step, inspect variables, and read
call stacks on the target from the development host. The current debug protocol is local-only
(`pipe` by default, loopback socket optional), so remote debugging is a product and security
change, not just an extension setting. It is split from WL-1.1 because network transport, authentication,
deployment, and remote source handling are different work from the local DAP adapter.

**Candidate scope.**

- Remote transport for the debug path: direct TCP/TLS to the runtime, an authenticated proxy or
  sidecar, or tunneling through SSH or a remote workspace. Define whether the runtime gains a
  network mode or the adapter/proxy owns it.
- Authentication and authorization, TLS, session lifecycle, timeouts, audit, and rate limits.
  Bind to loopback by default and require explicit opt-in for network exposure.
- Source mapping: remote source paths, build IDs, manifests, and generations; locate or fetch
  sources matching the loaded artifact and reject mismatched builds.
- Launch and attach configurations for remote hosts; connection loss, reconnection, and
  swap/rollback behavior.
- Always-on runtime and attach-time deployment: keep the runtime independent of the editor and
  file watcher; when an attach finds changed sources or artifacts, offer an explicit choice
  between a cold download and an online change (with cancel), and define state preservation and
  rollback for each.
- Use the PREEMPT_RT host (WL-4) as the first target; direct-host and Remote-SSH or remote
  workspace paths are deployment options.
- Evidence: remote session integration tests, authentication/TLS negative tests, reconnection
  tests, a threat model, and a runbook.

**Constraints and current state.**

- WL-1.1 is delivered: the local DAP adapter and local debug protocol exist. This item adds
  remote transport, authentication, and source mapping rather than another local adapter.
- The current debug path is local-only and one session per host; release builds are monitoring-only; every
  swap, rollback, cold install, or shutdown ends the session and clears forces and breakpoints.
- Debug can read and write variables, force values, stop the cycle, and hold outputs. Remote
  exposure must be authenticated and encrypted; `stc` is not a safety PLC.
- Source paths and debug information are build-host-specific. Cross-host artifacts need a path
  mapping or source-fetch policy.
- The network model should be shared with WL-8 (tracing through the WebUI) and WL-9 (WebUI,
  deferred), not invented independently.
- A remote language server is not required for remote debugging; it is a separate workspace and
  security question that can be considered later.

**Open questions.**

- Direct network debug, SSH tunnel or Remote-SSH, or both? Which comes first?
- Does the runtime expose a network debug endpoint, or does a proxy or sidecar translate to the
  local protocol?
- Authentication model: token, mTLS, SSH identity, or delegated WebUI authentication? Where does
  TLS terminate?
- How are remote source files mapped or fetched, and how is an artifact/build-ID mismatch
  handled?
- On attach with changed sources, should the IDE always ask, remember a default, or infer
  online change vs cold download from the compatibility result?
- What happens to a remote session on network loss, swap, rollback, or runtime restart?
- Is one remote session per host still the limit, or do deployments need multiple concurrent
  sessions?
- Should remote debugging work from other DAP clients, or only VS Code?
- Which host is first: the PREEMPT_RT host (WL-4) or a deployed controller?

**Related.** [Debug and Monitoring](specification/Architecture/Debug-and-Monitoring.md),
[Known limitations §3 and §4](KNOWN-LIMITATIONS.md), WL-1.1, WL-4, WL-8, WL-9.

**Tentative size.** M/L, dominated by transport/security and remote source handling, not by the
DAP adapter itself.

### WL-6 — Motion controller for coordinated multi-axis interpolated motion

**Intent.** Add a motion-control capability for coordinated multi-axis motion: jerk-limited
joint-space interpolation and Cartesian interpolated moves, running inside the runtime cycle.
Adapt the existing motion implementation and Lean formulations from
[robot-sim](https://github.com/r2k-in-the-vortex/robot-sim) instead of reimplementing the
mathematics.

**Status.** Deferred to a future version (2026-10-29).

**Why it matters.** Motion is a core PLC use case, and the project already has a proven
planner/tracker. robot-sim's MIT-licensed `robot-motion` crate is written against the Lean
specification in `lean/Motion` and covers cubic profiles, feasibility, conservative/optimal/
general planners, stretching, synchronization, composition, `MoveJ`/`MoveL`, and
tracking/supervision, with exact-rational golden data. It also pins the same Lean 4.35.0
toolchain as `stc`. Reuse turns a large control problem into integration, generalization, and
verification work, and its Lean formulations belong in the verification program rather than in
a second, disconnected proof effort.

**Candidate scope.**

- Adapt the `robot-motion` planning, tracking, and supervisor code into a runtime motion service
  or library; keep the proven algorithms and retire simulator-only pieces.
- Generalize from the 6-axis DH arm to a configurable axis model: axis count, joint and axis
  limits, velocity/acceleration/jerk limits, units, gearing, and encoder scaling. Retain the arm
  kinematics and closed-form IK where the target machine is articulated.
- Coordinated interpolation: joint-space synchronized S-curves (`MoveJ`), Cartesian straight-line
  moves with orientation interpolation and IK (`MoveL`), and sampling/streaming setpoints at the
  cycle boundary.
- Axis interfaces: virtual or simulated axes first; bind to the process image and the WL-5.2
  EtherCAT backend for real servo drives, using ESI-described PDOs (WL-7). Modbus TCP (WL-5.1)
  is general I/O, not a motion channel. Expose enable/disable, status, limits, following error,
  stop/abort, and recovery.
- ST-visible API: a minimal standard motion function-block/function set or a runtime service
  contract, with configuration in `stc.toml`/metadata and defined cycle semantics.
- Evidence: reuse the exact-rational golden vectors and Lean proofs; add cycle-level integration
  tests, a simulator, hardware-in-the-loop runs on the PREEMPT_RT host, and timing/jitter
  measurements.
- Decide whether the Lean `Motion` library is imported into `lean/` as a proof library or kept
  as a pinned external dependency, and register its theorems and TCB accordingly.

**Constraints and current state.**

- robot-sim is MIT-licensed, so adaptation is compatible. Its Lean proof library uses Mathlib
  v4.35.0; stc's D-V2 policy allows Mathlib only in proof libraries after a build-time spike,
  while executable models stay core Lean plus Batteries. That policy and the spike still need to
  be resolved.
- The runtime executes ST on one cooperative cycle thread; setpoint generation must fit the cycle
  budget. The delivered WL-4 campaign is host-specific evidence; motion still needs its own
  cycle-budget and jitter evidence.
- The current release has no motion function blocks and no drive integration. Motion does not
  architecturally require EtherCAT or ESI—virtual axes are enough to exercise the controller—but
  practically it uses the delivered WL-5.2 backend and the deferred WL-7 configuration to drive
  real servo axes. Modbus TCP (WL-5.1) is not a viable interpolated motion channel. Without
  EtherCAT and ESI, motion is limited to virtual axes or simulation.
- Motion can move machinery. `stc` is not a safety PLC; limits, watchdogs, and emergency stops
  must come from a separate safety system, and this item must not present a motion stop as a
  safety function.
- Online change and motion state need explicit rules: an active move across a swap should
  likely be rejected or aborted rather than migrated.
- The robot-sim kinematics are arm-specific; a general motion controller may need a configurable
  kinematic model or a simpler coordinated per-axis model for gantries and other machines.

**Open questions.**

- Which machine class comes first: articulated arm, general coordinated axes (gantry/cartesian),
  or both?
- Runtime service, ST function blocks, or both; what is the minimum ST-visible surface?
- Which robot-sim components are in scope: planner, tracker, supervisor, IK, and dynamics? The
  force/torque model may be out of scope.
- How are axes mapped to process-image locations and drive PDOs, and what is the unit/gearing
  contract?
- Which servo drives and ESI profiles must be supported first, and what PDO/distributed-clock
  subset do they require?
- Does motion require a new runtime ABI service or metadata section, or can it be built on the
  existing I/O and cycle contracts?
- How are active moves and axis state handled across online change, stop, and restart?
- Where do the Lean proofs live, and how are they kept in sync with the external repository?
- What evidence demonstrates interpolation correctness: golden trajectories, cycle-accurate
  replay, or hardware measurement?

**Related.** [robot-sim](https://github.com/r2k-in-the-vortex/robot-sim),
[Runtime and Execution](specification/Architecture/Runtime-and-Execution.md),
[I/O and Device Linking](specification/Architecture/IO-and-Devices.md),
[Verification Program Plan](specification/Verification/VerificationProgramPlan.md) (D-V2), WL-3, WL-4, WL-5.2,
WL-7.

**Tentative size.** L/XL. Adapting proven code reduces the math risk; generalization, drive
integration, the ST API, and cycle/online-change behavior dominate.

### WL-7 — ESI-based EtherCAT device configuration

**Intent.** Let the runtime configure EtherCAT devices from vendor `.esi` (EtherCAT Slave
Information) files instead of hand-maintained channel and PDO metadata. This builds on the
delivered WL-5.2 EtherCAT backend.

**Status.** Locked for v3.0.0 (2026-11-04). Previously deferred (2026-10-29).

**Why it matters.** An ESI file describes a slave's vendor/product/revision identity, object
dictionary, PDO mapping, sync managers, distributed clocks, and mailbox behavior. Practical
EtherCAT commissioning requires matching the actual slave revision and PDO layout; hand-written
configuration is error-prone and does not scale across devices. ESI support is what turns WL-5.2
from a static configuration demonstration into deployable I/O, and it is practically required by
WL-6 for real servo drives.

**Candidate scope.**

- Ingest ESI XML from project or runtime configuration, including package-relative paths and a
  vendor file library.
- Parse and validate identity (vendor ID, product code, revision), PDO mappings, sync managers,
  distributed clocks, and mailbox/SDO data. Handle multiple files and revisions with
  deterministic selection and clear diagnostics.
- Map validated slave descriptions to the `stc` device/channel model and to IgH configuration,
  and verify at startup that the discovered slave matches the expected vendor/product/revision
  and PDO layout.
- Decide the compiler/runtime split: the compiler may validate binding widths against ESI at
  build time while keeping the language and metadata protocol-agnostic; the runtime owns IgH
  configuration and live verification.
- Test with synthetic and vendor-supplied ESI files, malformed/ambiguous/mismatched inputs, a
  simulator, and hardware-in-the-loop on the RT host. Document the supported ESI subset and the
  failure policy.
- Ship only a small curated ESI set for tests; users supply vendor files.

**Constraints and current state.**

- ESI is XML with vendor extensions; full coverage is unrealistic. Define a supported subset and
  reject the rest explicitly rather than silently ignoring fields.
- The compiler does not link hardware or interpret protocol details. ESI is I/O configuration,
  not a language feature.
- IgH has its own configuration model and tooling. The integration point (generate IgH config,
  call a library API, or both) must be chosen in planning.
- ESI files are untrusted input; parsing must be bounded, robust, and fuzz-tested.
- Vendor ESI files may have redistribution restrictions. Do not commit vendor files without
  permission; tests use synthetic or minimal descriptions.

**Open questions.**

- What is the minimum ESI subset for the first release: identity and PDO mapping only, or also
  distributed clocks, mailbox, and SDO startup parameters?
- Compile-time validation, runtime-only configuration, or both?
- How are ESI files selected, version-pinned, and resolved for deployment?
- Does IgH consume ESI directly or through generated configuration, and which IgH versions are
  supported?
- How is a device mismatch handled: refuse to start, fall back to defaults, or require a cold
  download?
- Where do ESI files live in a package or manifest, and how are they validated at build time?

**Related.** [I/O and Device Linking](specification/Architecture/IO-and-Devices.md),
[Runtime and Execution](specification/Architecture/Runtime-and-Execution.md),
[CLI and Manifest](specification/Architecture/CLI-and-Manifest.md), WL-5.2, WL-6.

**Tentative size.** M/L, depending on how much of the ESI object dictionary and distributed-clock
behavior is in the first supported subset.

### WL-8 — Tracing UI to watch and graph PLC variables

**Intent.** Provide a tracing UI, delivered as a module of the WebUI (WL-9, deferred), that lets a
developer or commissioner select PLC variables, sample them over time, and see live graphs,
with export and basic analysis.

**Status.** Deferred to a future version (2026-10-29).

**Why it matters.** Typed watches already exist, but there is no visualization or time-series
capture. Commissioning, tuning, and troubleshooting motion, I/O, and control loops need trends
rather than single reads. A tracing UI makes the existing debug and monitoring investment usable
without writing a custom client.

**Candidate scope.**

- Variable selection from the manifest or typed paths, watch sets, sample interval, start/stop,
  value or edge triggers, multiple series, a time axis, pause/zoom, export to CSV/JSON, and
  snapshot capture.
- Data source: the existing debug watch protocol (typed reads, watch batches, generation and
  cycle tags) for low-rate capture; optionally a runtime trace ring buffer sampled at the cycle
  boundary for high-rate or long captures, transferred in batches. Define sample semantics
  (cycle counter, monotonic time, latch point) and behavior across swaps and session end.
- UI form: a tracing module inside the WebUI (WL-9, deferred), using its hosting, authentication,
  packaging, and lifecycle. A VS Code webview could reuse the same component later, but the
  WebUI is the primary surface. Tracing must not run a second unauthenticated network service.
  Local-first; remote access only with the network-security work.
- Evidence: a trace harness, deterministic replay, sampling-overhead measurement, and tests for
  swap/session end, rate limits, and data-size limits.

**Constraints and current state.**

- The tracing UI depends on WL-9 (deferred) for hosting, authentication, and deployment. Its data
  path still uses the debug/watch contracts; the WebUI is only the front end.
- Debug is local-only and one session per host. Release builds are monitoring-only. Session
  limits include 256 watches, 100 requests/s with burst 200, 5 s deadlines, and 4 MiB response
  frames. Every swap, rollback, cold install, or shutdown ends the session.
- Persistent globals and program fields have declaration-only debug-info entries because their
  storage is host-owned; tracing must use typed reads, not raw memory.
- High-rate tracing competes with the cycle. Sampling must be bounded and measured, and a
  runtime-side ring buffer changes the service/debug boundary and needs a specification.
- Tracing is monitoring, not a safety function. Values may be stale or torn according to the
  documented atomicity rules.

**Open questions.**

- What is the split between the WebUI shell (WL-9, deferred) and the tracing module: routing,
  auth scopes, navigation, and packaging?
- Polled watches or a runtime-side ring buffer? What sample rate and capture duration are
  required?
- Where is trace data stored, and what are the retention and export formats?
- How are variables re-resolved after an online change: end the series, or remap by `GotId`?
- Does tracing also need a VS Code surface (WL-1.1), or only the WebUI?
- Is tracing allowed on release builds and in production deployments, subject to the monitoring
  limits?

**Related.** [Debug and Monitoring](specification/Architecture/Debug-and-Monitoring.md),
[Known limitations §3](KNOWN-LIMITATIONS.md),
[Runtime and Execution](specification/Architecture/Runtime-and-Execution.md), WL-9 (deferred),
WL-1.1.

**Tentative size.** M/L, dominated by the sampling architecture and the UI, not by basic watch
plumbing.

### WL-9 — WebUI for basic runtime control

**Intent.** Provide a browser-based UI for basic runtime control: start, stop, restart, and
status, plus a small, explicitly scoped set of operational actions.

**Status.** Deferred to a future version (2026-10-31); removed from the v2.0.0 target set in the
2026-10-31 re-plan. No v2.0.0 target depends on it.

**Why it matters.** Current runtime control is a local CLI and API. There is no remote or network
deployment, no multi-user scheduling, and no deploy or rollback UI. A minimal WebUI is the
natural operations surface, but exposing a PLC runtime on a network is security-critical and
must be designed rather than bolted on.

**Candidate scope.**

- A runtime HTTP/WebSocket server or a sidecar with a versioned API and static assets. Bind to
  loopback by default and require explicit opt-in for network exposure.
- Actions: start/stop/restart the cycle; show status (running, stopped, faulted, generation/build
  ID, cycle time, trap/error); show logs. Optionally online download/activate/rollback and
  manifest export. Keep "basic" explicit: no programming, no arbitrary memory access, and no
  forcing unless separately specified.
- Authentication and authorization: users/roles or an API token, TLS, CSRF protection, audit
  logging, rate limits, and session expiry. Integrate with the existing control/status thread
  and service boundaries rather than creating a parallel control path.
- State machine: map each action to the runtime lifecycle, reject invalid transitions, and define
  stop/restart semantics for retained state, output behavior, active debug sessions, and motion.
  An emergency stop remains a separate, external safety function.
- Packaging: static assets embedded in the artifact, no CDN dependency, headless operation, and
  support on all native hosts.
- Future extension point for the tracing UI (WL-8, deferred): shared routing, authentication
  scopes, navigation, and packaging, without coupling core runtime control to trace data.
- Tests: authentication and authorization matrix, invalid transitions, concurrent sessions,
  network failure, online change, security-negative cases, and browser smoke tests. Deliver a
  threat model and runbook.

**Constraints and current state.**

- The current runtime is local-only for `stc run --watch`, the control channel, and debug. Exposing network
  control is a product and security change; it overlaps WL-1.2's remote debugging work and
  should share a single remote-access security design.
- `stc` is not a safety PLC. A WebUI stop is not an emergency stop and must not be presented as
  one. Debug sessions and motion can hold outputs or move machinery.
- The runtime has a control/status thread and host ABI services; the WebUI must use those
  contracts.
- Multi-user scheduling and roles do not exist in v1. If the WebUI needs roles, that is new
  runtime surface.
- Authentication secrets and certificate management need a deployment story.

**Open questions.**

- In-runtime server or separate sidecar? Which HTTP stack and static-asset strategy?
- Local-only first, or network from day one? What is the default bind and the explicit opt-in?
- Authentication model: single admin token, users and roles, or external identity? Where does
  TLS terminate, and how are certificates rotated?
- Scope of "basic": start/stop/restart/status only, or also download/activate/rollback and logs?
- Does the WebUI use the debug protocol, the CLI control channel, or a new HTTP API?
- How are actions audited, and how do they interact with active debug sessions and online
  changes?
- If WL-8 is revived, how is the tracing module mounted and authorized within the WebUI?

**Related.** [Runtime and Execution](specification/Architecture/Runtime-and-Execution.md),
[CLI and Manifest](specification/Architecture/CLI-and-Manifest.md),
[Debug and Monitoring](specification/Architecture/Debug-and-Monitoring.md),
[Known limitations §3 and §4](KNOWN-LIMITATIONS.md), WL-1.2, WL-8.

**Tentative size.** M/L, dominated by authentication, authorization, the lifecycle state machine,
and security review.

### WL-11 — Modbus TCP client capability

**Intent.** Add Modbus TCP client/master capability, separate from the WL-5.1 server backend:
connect to external Modbus TCP servers and map their coils and registers into the `stc` process
image or access them from ST. It is not a real-time channel.

**Status.** Deferred to a future version (2026-10-29); separate from the delivered WL-5.1
server.

**Why it matters.** Many deployments need both roles: the PLC exposes its own process image
(server) and also reads sensors, meters, remote I/O, or drives that expose a Modbus TCP server.
The server role shipped first because it adds no outbound connection scheduling to the cycle.
The client role is separate because it introduces connection management, polling,
reconnect/backoff, and partial-failure handling with different scheduling implications.

**Candidate scope.**

- Client connections to one or more remote Modbus TCP servers; read and write coils, discrete
  inputs, holding registers, and input registers; schedule requests within the cycle; timeouts,
  reconnects, backoff, byte/word order, and diagnostics.
- Map remote data into the process image or expose explicit read/write operations to ST, with a
  defined freshness rule for cached values.
- Configuration and binding model: endpoint, unit ID, mapping, and direction, expressed through
  the existing device/binding metadata where possible.
- Must run on Windows x86-64 MSVC, Linux x86-64 GNU, and Linux AArch64 GNU.
- Evidence: Modbus server simulator tests, error-path and partial-failure tests, cross-host
  tests, cycle-overhead measurements, and specification updates.

**Constraints and current state.**

- Not real-time and not an interpolated motion channel. Client polling is best-effort; a slow or
  failed server must not silently produce wrong outputs, and the non-safety stance applies.
- Modbus TCP has no standard authentication or encryption. Assume a trusted or segmented network
  and document the risk; transport security is a separate decision.
- The compiler never opens devices; the runtime owns connections, scheduling, and bindings.
- Shares the backend contract with WL-5.1 but has a different scheduling and failure model; do
  not force the server and client implementations into one code path.
- Server and client may need to run simultaneously in one runtime; that interaction is not yet
  designed.

**Open questions.**

- Which function codes, data widths, and byte/word order are required first?
- Polling model: scheduled per cycle, background with a freshness rule, or explicit ST calls?
- One connection per remote device, or a shared connection pool?
- How are client bindings represented relative to the WL-5.1 server mappings?
- What are the timeout, reconnect/backoff, and partial-failure policies?
- Can server and client run concurrently in one runtime, and what does that imply for the
  backend contract?
- Is client capability needed before or after the EtherCAT (WL-5.2) and deferred motion work?

**Related.** [I/O and Device Linking](specification/Architecture/IO-and-Devices.md),
[Runtime and Execution](specification/Architecture/Runtime-and-Execution.md),
[CLI and Manifest](specification/Architecture/CLI-and-Manifest.md),
[Known limitations §5](KNOWN-LIMITATIONS.md), D-028, WL-5.1.

**Tentative size.** M/L, dominated by scheduling, connection management, and failure handling.

### WL-12 — Reduce cycle-thread wake-up jitter at sub-millisecond cycle times

**Intent.** Investigate and reduce the host/runtime wake-up jitter that limits the maintainable
cycle time below 150 µs on the WL-4/WL-5.2 PREEMPT_RT host. This is a scheduling and platform
investigation, not an EtherCAT protocol or backend change.

**Selection.** Deferred on 2026-11-01. It is explicitly out of scope for the delivered WL-5.2
work and the v2.0.0 target set. It is recorded because the WL-5.2 phase-5 CT sweep produced
reproducible evidence that the remaining limit is wake-side scheduling, not the fieldbus. It
also absorbs the tuned boot profile that WL-4 deferred when v2.0.0 shipped.

**Why it matters.** WL-5.2's instrumented sweep maintained 150 µs with zero skipped periods,
while 100 µs failed with 283 skipped periods. The 100 µs events had 53.4–133.3 µs wake lateness
and only 38.3–53.9 µs execution, so the frame path stayed healthy but the cycle thread woke late.
The metrics-writer 1 MiB flush was a separate artifact and has been moved off the cycle thread.
Faster cycle times matter for motion and high-channel-count I/O, but they were not part of the
delivered WL-5.2 scope.

**Candidate scope.**

- Reproduce the 150/100/75/50 µs sweep with the WL-5.2 DIEWU fixture and the installed wrappers.
- Absorb the WL-4 tuned boot profile evaluation: the untuned triggers (R1 heavy mixed load and
  osnoise) were met; measure whether a tuned boot profile removes the remaining stability limit
  and record the configuration as host-specific evidence.
- Instrument wake-up sources with `rtla timerlat`/`osnoise`, `cyclictest`, `perf sched`, and
  tracepoints; correlate wake lateness with the IgH master thread and IRQs, the runtime
  control/status/debug/metrics threads, C-states, isolation, and the timer source.
- Evaluate host and runtime measures: IRQ affinity, `isolcpus`/`nohz_full`/`rcu_nocbs`, C-state
  limits, governor/frequency, RT throttling, `skew_tick`, thread priority/affinity, and moving
  any remaining service work off the cycle CPU.
- Define a wake-lateness budget and a repeatable campaign; report distributions, skipped
  periods, and the first failing cycle time, host- and configuration-specific.
- Keep the EtherCAT backend semantics and the `stc-rt-metrics/1` schema stable; instrumentation
  and scheduling changes are additive.

**Constraints and current state.**

- The WL-5.2 result already separates the metrics-writer artifact from host wake jitter; the
  remaining limit is not a fieldbus correctness problem.
- The host is a fixed 1.5 GHz J4105 with CPU 3 isolated, RT throttling off, and the IgH master
  active. Results must not be generalized to other hardware.
- This item does not create a hard real-time guarantee or a safety function.

**Open questions.**

- What target cycle time or wake-lateness budget is worth pursuing?
- Which runtime threads or IgH IRQs can be moved without changing the I/O contract?
- Is a tuned boot profile acceptable, or should the runtime work within the current profile?
- Can the platform meet the target, or is the 150 µs floor hardware-limited?

**Related.** [EtherCAT Host report](specification/Verification/EtherCAT-Host.md),
[EtherCAT Runbook](specification/Verification/EtherCAT-Runbook.md) §10.7,
[RT-Host.md](specification/Verification/RT-Host.md),
[RT-Runbook.md](specification/Verification/RT-Runbook.md),
[Performance](specification/Verification/Performance.md), WL-4, WL-5.2.

**Tentative size.** M/L, dominated by the measurement campaign, trace correlation, and host
tuning experiments.

### WL-13 — Lean security verification for the Modbus TCP server boundary

**Intent.** Extend the Lean verification program to the WL-5.1 Modbus TCP server's untrusted
network boundary: model the decoder, function dispatch, address/width validation, and
register-bank write rules, and prove security properties for malformed input and client writes.

**Selection.** Deferred on 2026-11-02, beyond v2.0.0. Recorded after WL-5.1 shipped because the
server is the project's first network-facing backend and its tests are not a proof.

**Why it matters.** Any reachable client can send arbitrary MBAP/PDU bytes and write every
input-mapped coil or register. WL-5.1 has unit, property, fuzz-smoke, loopback, and cross-host
tests, but those are samples. A Lean model can prove the bounded properties that matter at this
boundary: the decoder is total on bounded frames, no accepted request can panic or write outside
its table/image range, output and read-only channels are never written, 8-bit byte placement and
wider spans are preserved, and multi-write requests are all-or-none at the latch boundary. The
properties fit the existing WL-3 Lean infrastructure and create a reusable model for WL-11 and
future protocol work.

**Candidate scope.**

- Model the externally reachable server boundary: MBAP framing, PDU decode, function-code
  dispatch, quantity/byte-count checks, table/address/span validation, unit filtering, 8-bit
  low/high placement, device/per-channel word order, and the register bank's read/write rules.
  Abstract sockets, threads, timeouts, and the OS scheduler; this item is about the protocol and
  image transformation, not the network stack.
- Prove, with no `sorry`:
  - decoder totality and boundedness: every accepted frame is at most 260 bytes, has a
    well-formed PDU, and cannot produce an out-of-range slice or arithmetic overflow;
  - write authorization: a write can affect only an input-mapped channel of the addressed table;
    output/read-only/unmapped addresses and partial 32/64-bit spans are rejected; an 8-bit input
    byte is preserved when an output byte shares its register;
  - image isolation: no write leaves the configured channel span or crosses a server/backend
    boundary;
  - atomicity: a multi-address write is all-or-none at the latch boundary;
  - counters and status remain within their bounded domains and are not mistaken for a safety
    signal.
- Connect the model to the Rust implementation by a documented refinement argument, an extracted
  core, or a bounded review; the connection method is an open question, not assumed.
- Add the proofs to the verification registry and report, cross-check against the Rust tests and
  fuzz corpus, and state what the model does not cover. No production behavior change and no
  safety or real-time claim.

**Constraints and current state.**

- WL-3 provides the Lean project, registry, and proof-review pattern. WL-5.1 is complete; its
  permanent contract is [I/O Routing and Backends](specification/Architecture/IO-Routing-and-Backends.md)
  §4.5 and [Decisions](specification/Decisions.md) D-229–D-243.
- The server is unauthenticated by design. The item proves protocol and write-authorization
  invariants, not authentication, TLS, or a trusted-network assumption.
- The runtime is Rust with safe code at this boundary; the proof is about the protocol state
  machine and image mapping, not a general memory-safety proof of the whole runtime.

**Open questions.**

- Model the protocol/bank in Lean, extract a verified core, or prove a refinement of the Rust
  functions?
- Which properties are worth proving first, and what is the exact theorem set?
- How should sockets, threads, timeouts, and fault/retry state be abstracted?
- Should the status device, counters, and unit filtering be in the first slice?
- How is the model kept in sync with future Modbus changes and WL-11?
- Does the proof need a machine-checked link to the Rust code, or is a reviewed refinement
  argument sufficient for the claimed evidence level?

**Related.** [I/O Routing and Backends](specification/Architecture/IO-Routing-and-Backends.md)
§4.5, [Modbus-Server.md](specification/Verification/Modbus-Server.md),
[Decisions](specification/Decisions.md) D-229–D-243, WL-3, WL-5.1, WL-11. Inspiration:
[external discussion](https://chat.deepseek.com/share/7f57f70kv31kawzx5f) (not a project
dependency or evidence).

**Tentative size.** L/XL, dominated by the protocol/image model and the refinement connection.

### WL-14 — macOS native support and Windows/macOS soft-real-time scheduling for tests

**Intent.** Make macOS ARM64 and x86-64 supported native hosts at Windows parity for the
Windows-available surface, and use the operating system's soft-real-time scheduling facilities on
macOS and Windows to improve test and benchmark behavior. EtherCAT and PREEMPT_RT-style hard real
time are excluded; no hard-real-time claim is made.

**Status.** Complete for v3.0.0 (2026-11-09); manually checked and approved; merged into
`v3.0.0-planning` (PR #27, merge `e99f33a`). Final CI run
[42546445894](https://github.com/r2k-in-the-vortex/stc/actions/runs/42546445894), all jobs green,
including both macOS workspace, package, and extension-smoke jobs. Locked for v3.0.0 (2026-11-04);
scope revised 2026-11-08 after owner review.

**Selection.** Locked for v3.0.0 (2026-11-04); scope revised 2026-11-08 after owner review. The
earlier coverage-only interpretation was withdrawn: macOS joins the support matrix, including release
packaging and the VS Code extension smoke. This item is inspired by the audio/pro-audio use of
thread priorities and time-constraint policies on Windows and macOS.

**Why it matters.** macOS is a second soft-real-time-capable Unix and a common development host.
Full-workspace coverage, packages, and the extension smoke catch portability regressions that a
narrower CI subset misses. Windows has `SetThreadPriority`/`THREAD_PRIORITY_TIME_CRITICAL` and MMCSS
for audio-style workloads; macOS has Mach thread policies and QoS classes for audio and
time-constrained work. Using these for tests can reduce cycle jitter and expose scheduling
assumptions. The RT evidence remains host-specific and not a guarantee.

**Scope.**

- Add macOS ARM64 (`macos-15`) and x86-64 (`macos-15-intel`) as supported native hosts: full
  workspace tests, release packages with `debug_contract: dwarf`, and the VS Code extension smoke.
- Keep EtherCAT and PREEMPT_RT-style hard real time Linux-only; document both exclusions.
- Add an opt-in test/benchmark scheduling mode:
  - Windows: raise the cycle/test thread to `THREAD_PRIORITY_TIME_CRITICAL`, and optionally use
    MMCSS (`AvSetMmThreadCharacteristics` with an audio task, then revert) with explicit scope
    and cleanup; avoid requiring administrator privileges.
  - macOS: apply a Mach `THREAD_TIME_CONSTRAINT_POLICY` or appropriate QoS/priority to the
    cycle/test thread; document entitlement or privilege requirements and ARM64/x86-64
    differences.
- Measure cycle timing, wake lateness, and request/scan jitter with the WL-4 harness on Windows
  and macOS; compare baseline against scheduling mode; record host, OS, build form, metric, and
  sample. Reuse the WL-12 harness if that work is selected later, but WL-12 is not a prerequisite.
- Keep production scheduler behavior unchanged unless measured evidence and owner review approve
  an opt-in runtime mode. Define no timing threshold or CI gate.

**Constraints and current state.**

- GitHub-hosted macOS runners are available on both architectures and accept the Mach
  time-constraint policy; the QoS fallback remains covered by host tests.
- The full workspace exposed portability issues (Unix-socket path length, BSD userland in the
  fake remote tests, and the non-Linux EtherCAT mock); they are fixed in the test harness.
- Mach-O packaging uses `llvm-strip --strip-debug`, Mach-O `__debug_*` detection, and ad-hoc
  re-signing; the package matrix includes both macOS runners.
- Windows MMCSS is designed for audio; its behavior for a PLC cycle thread is measured, not
  assumed. Thread priority alone is not a real-time guarantee.
- The item changes the support matrix, not the language profile or safety stance.

**Resolved questions.**

- Which macOS runner images and architectures? `macos-15` (ARM64) and `macos-15-intel` (x86-64).
- Do the Mach time-constraint APIs work in GitHub-hosted macOS runners and in a normal app? Yes on
  both runners; the QoS fallback is tested.
- Is MMCSS appropriate for a cycle thread? It is applied when available and recorded in the
  metrics header; the mode remains advisory.
- Which tests run on macOS? The full workspace, plus the package exercise and extension smoke.
- What metric and threshold justify an opt-in scheduling mode? No threshold; `--rt` is opt-in and
  informational (D-193, D-249).
- Should the scheduling mode reuse WL-12's wake-lateness metrics? WL-12 is not required; the
  existing WL-4 harness supplies the metrics.

**Related.** [RT-Host.md](specification/Verification/RT-Host.md),
[RT-Runbook.md](specification/Verification/RT-Runbook.md),
[EtherCAT-Host.md](specification/Verification/EtherCAT-Host.md),
[Performance.md](specification/Verification/Performance.md),
[USER-GUIDE.md](USER-GUIDE.md), [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md), WL-4, WL-5.1,
WL-5.2, WL-12.
Inspiration: [external discussion](https://chat.deepseek.com/share/7308oh9yx127s4kogv) (not a
project dependency or evidence).

**Tentative size.** M/L, dominated by CI matrix work, platform scheduling experiments, and
measurement.

### WL-15 — MQTT client for process-image I/O

**Intent.** Add an MQTT client I/O backend that connects to an external MQTT broker and maps
topic/payload messages to the `stc` process image. The broker is external; this item does not
implement a broker, an MQTT server, or a cloud connector.

**Selection.** Locked for v3.0.0 (2026-11-04). Previously deferred on 2026-11-03, beyond
v2.0.0. Recorded because the architecture already lists MQTT as a planned backend and because
it complements the Modbus TCP server and EtherCAT backends.

**Why it matters.** MQTT is the common lightweight publish/subscribe transport for sensors,
gateways, dashboards, and IT/cloud integration. A client backend lets `stc` consume remote
sensor and telemetry values into inputs and publish PLC outputs and status without implementing a
broker. It attaches to the existing multi-backend coordinator as another device `kind`, so it can
run alongside Modbus TCP and EtherCAT over the same process image. It is non-real-time,
best-effort, and not suitable for interpolated motion or safety functions.

**Candidate scope.**

- Client role only: connect to one or more configured MQTT brokers; no broker, MQTT server,
  MQTT-SN, or cloud-specific connector.
- Map channels to topics and payloads in the machine profile: subscription topics for inputs,
  publish topics for outputs, payload encoding, retain and last-will, QoS, keepalive, reconnect
  and backoff, and stale/cache behavior.
- Integrate as an `mqtt`/`mqtt-status` kind through `DeviceBackend`; connection lifecycle,
  per-backend fault/status, and optional PLC-visible status fields.
- Support broker authentication and TLS where the broker requires it; document the trust
  boundary and credential handling.
- Evidence: a broker simulator or local test broker, reconnect and malformed-payload tests,
  cross-host tests, and cycle-overhead measurements. No hard real-time or safety claim.

**Constraints and current state.**

- The project does not ship or require a broker; an operator-provided broker is a run target. The
  test strategy must provide a deterministic local broker or simulator.
- MQTT payloads are project-defined; there is no standard PLC register layout. The mapping must
  be explicit and source-controlled, not inferred.
- MQTT is asynchronous and non-real-time. A slow or unavailable broker must not stall the scan;
  the backend needs an explicit stale/cache and reconnect policy.
- The item does not add MQTT to the v2.0.0 release, the default package, or the safety claim.

**Open questions.**

- MQTT 3.1.1 first, MQTT 5.0, or both?
- Use an external MQTT client library (and which license/ABI) or a project-owned minimal client?
- What payload encoding(s) are required: raw bytes, BOOL/int/float/string, or JSON?
- Which QoS levels, retained messages, last-will, clean-session, and keepalive defaults?
- How are credentials and TLS material configured and kept out of source control?
- What is the stale/cache policy when the broker disconnects, and how are failures visible to
  the PLC?
- One broker per device, multiple brokers per run, or one broker shared by several devices?
- How does the test suite provide a broker without making the base package depend on one?

**Related.** [I/O Routing and Backends](specification/Architecture/IO-Routing-and-Backends.md),
[I/O and Devices](specification/Architecture/IO-and-Devices.md), WL-5.1, WL-5.2, WL-11.

**Tentative size.** M/L, dominated by MQTT client integration, mapping schema, security
configuration, and broker-backed tests.

### WL-16 — Debug session lifecycle: start-paused binding and exit status

**Intent.** Close the two WL-1.1 debug-session follow-ups deferred from v2.0.0: bind breakpoints
before the first cycle (F-9) and report session termination and exit in the Debug Console (F-15).
Optionally surface the `[debug].session-timeout` budget as well.

**Status.** Complete for v3.0.0 (2026-11-05); manually checked and approved. Locked 2026-11-04;
previously deferred (2026-11-04); recorded from the v2.0.0 WL-1.1 post-completion follow-ups.

**Why it matters.** Before WL-16, a launch with a breakpoint bound after one or two cycles, so the
first stop could observe cycle-1 or cycle-2 values instead of cycle-0 values; startup logic was
therefore not debugged deterministically. Separately, terminating a launch printed nothing in
the Debug Console even though the host exited, so a normal exit and a failed session looked alike.

**Candidate scope.**

- Add a start-paused mode (or an equivalent ready/bind handshake) so breakpoints bind before the
  first cycle; define launch/attach behavior, the time budget, and the failure path.
- Report terminate and exit in the Debug Console and distinguish a user stop from a host exit,
  trap, or connection loss.
- Optionally surface the `[debug].session-timeout` budget or a warning when the opt-in timeout
  is enabled.
- Evidence: adapter/runtime integration tests with a breakpoint at cycle 0, terminate/exit and
  timeout tests, and an extension smoke update.

**Constraints and current state.**

- WL-1.1 is delivered: the local DAP adapter and the `stc-debug/1` protocol exist, with the
  additive `stc-debug/1.1` Locals revision. `stc-debug/1` must remain valid; start-paused is
  either a protocol revision or an adapter-side handshake.
- Debug is local-only and one session per host. Every swap, rollback, cold install, shutdown, or
  target exit ends the session and clears forces and breakpoints.
- `[debug].session-timeout` is opt-in and disabled by default because an automatic resume must
  not restart machinery unexpectedly.
- A paused session holds outputs. This is not a safety function.

**Open questions.**

- Is start-paused a new protocol message, a launch flag, or an adapter-side wait for a ready
  event?
- Does it apply to attach as well as launch, and to release monitoring?
- Should the timeout warning be a Debug Console line, a notification, or both?
- Does F-15 need a protocol change, or only adapter and extension reporting?

**Related.** [Debug and Monitoring](specification/Architecture/Debug-and-Monitoring.md),
[IDE Integration](specification/Architecture/IDE-Integration.md),
[Known limitations §3 and §4](KNOWN-LIMITATIONS.md), WL-1.1 (delivered), WL-1.2.

**Tentative size.** S/M, dominated by the start-paused protocol/runtime handshake.

### WL-17 — Manifest and task follow-ups from v2.0.0

**Intent.** Close the remaining WL-1.1 close-out follow-ups that are local to the project loader,
task provider, and extension: optional project items and empty source sets (F-23) and
task-argument variable expansion (F-24).

**Status.** Locked for v3.0.0 (2026-11-04). Previously deferred (2026-11-04); recorded from the
v2.0.0 WL-1.1 post-completion follow-ups. F-19 was split out to WL-18 when it was selected for
v3; F-22 was fixed after v2.0.0 (`277f752`).

**Why it matters.** Each is a small but visible developer-experience gap:

- **F-23.** A declared source glob that matches no files is `STC-E-PROJECT-004` at project load,
  so a project without tests must omit `[test]`; other optional project-file items may fail at
  load in the same way.
- **F-24.** The task provider expands `${workspaceFolder}`-style variables in `project` but
  passes user-supplied `args` through unchanged, so expansion is not guaranteed for every task
  type.

**Candidate scope.**

- F-23: audit required versus optional project-file items and empty source sets; allow a project
  without tests and defer errors until the command that needs them. This amends
  `STC-E-PROJECT-004`, the conformance profile, and the CLI and language-server documentation.
- F-24: apply `expandTaskVariables` to each `args` entry and document the expansion so
  configured tasks behave consistently.
- Evidence: project-loader tests for empty and optional sets, task-provider tests, and
  documentation updates.

**Constraints and current state.**

- Project loading is shared by the CLI, language server, and task provider; F-23 changes a
  project error contract and needs a decision.
- The extension is local-only and VSIX-only; no marketplace or remote behavior is implied.

**Open questions.**

- Which project items become lazy, and does that change `stc check` diagnostics?
- Should all task arguments use one variable set, and how are unknown variables reported?

**Related.** [IDE Integration](specification/Architecture/IDE-Integration.md),
[CLI and Manifest](specification/Architecture/CLI-and-Manifest.md),
[Known limitations §4](KNOWN-LIMITATIONS.md), WL-1.1 (delivered), WL-18.

**Tentative size.** S/M.

### WL-18 — Language-server rename (F-19)

**Intent.** Add `textDocument/prepareRename` and `textDocument/rename` to `stc-lsp`, backed by
the compiler's reference resolution and rename history, so Rename (F2) works in VS Code instead
of doing nothing.

**Status.** Locked for v3.0.0 (2026-11-04). Split out of WL-17.

**Why it matters.** Rename (F2) currently does nothing and prints no message, even though the
compiler and runtime carry rename history. It is the most visible missing editor action in the
closed language-server capability set.

**Candidate scope.**

- Advertise `renameProvider` with `prepareProvider`; resolve the symbol at the cursor and return
  a `WorkspaceEdit` that renames every resolved reference.
- Reuse the compiler's reference resolution and rename history; do not build a second symbol
  graph in the language server.
- Handle the symbol kinds the compiler already supports (programs, functions, variables, fields,
  and types as available) and reject unsupported symbols with a clear message rather than a
  silent no-op.
- Evidence: LSP rename tests over a multi-file project, prepare/rename negative cases, and an
  editor smoke check.

**Constraints and current state.**

- WL-1.1 is delivered: the language server, reference resolution, and `stc rename` CLI exist;
  the LSP capability set is closed and advertises no rename support. This item reopens the set
  for rename only.
- Rename across files must respect the project graph and source sets; renaming a symbol used by
  an artifact or generated wrapper needs the same rules the CLI uses.
- The extension is local-only and VSIX-only; no remote behavior is implied.

**Open questions.**

- Does the first slice cover every symbol kind the CLI rename supports, or a bounded subset?
- Does rename apply to files outside the project source set, and how are ambiguous or unresolved
  references reported?
- Should the CLI and LSP share one rename operation, or is the CLI's behavior the reference?
- Is a workspace-edit size limit needed for large projects?

**Related.** [IDE Integration](specification/Architecture/IDE-Integration.md),
[CLI and Manifest](specification/Architecture/CLI-and-Manifest.md),
[Known limitations §4](KNOWN-LIMITATIONS.md), WL-1.1 (delivered), WL-17.

**Tentative size.** M, dominated by reference-backed rename and its edge cases.

### WL-19 — Microcontroller support and a tiny runtime variant

**Intent.** Add a bare-metal execution target for `stc`: a statically linked, single-generation
"tiny" host runtime with a bounded host-API subset, no operating system, no dynamic module
loading, and no online change, so ST programs can run on 32-bit microcontrollers such as ARM
Cortex-M and RISC-V. The MCU is a run target, not a development or build host; the compiler and
editor continue to run on the supported native hosts.

**Status.** Candidate (2026-11-05). New; not part of the v3.0.0 target set.

**Why it matters.** The compiler already emits target-independent IR and clang objects for
`thumbv7em-none-eabihf` and `riscv64-unknown-elf` in the cross-target smoke matrix, but execution
stops at the object. `--emit=exe` and `--emit=lib` reject non-host targets (`STC-E-BUILD-004`),
and the runtime supports Windows x86-64, Linux x86-64, Linux AArch64, macOS ARM64, and
macOS x86-64. A tiny host turns
the existing cross-compile path into an executable one and opens a class of controllers with no
OS, loader, filesystem, or network. It is also the prerequisite for WL-20.

**Candidate scope.**

- **Tiny host profile.** Link the generated module and a small host into one image instead of
  loading a `.dll`/`.so`: one generation, a fixed GOT cell table and root storage in static
  memory, no manifest store, no swap controller, and no control or debug service threads.
- **First target slice.** `thumbv7em-none-eabihf` (Cortex-M4F) with QEMU and at least one board
  as evidence; then a 32-bit RISC-V target or the already-smoke-tested `riscv64-unknown-elf`.
  8- and 16-bit targets are out of scope for this item and belong to WL-20.
- **Runtime subset.** Decide and document the host-API services a tiny module may require (trap,
  cycle and monotonic clocks, process image, alloc/free, assert, and optionally state) and the
  ABI minor it may require. Reject an unsupported service at build time rather than failing at
  run time.
- **Static build model.** Generated object plus tiny host archive plus startup, vector table, and
  linker script. Define who supplies `main`, the timer, the interrupt handlers, and the linker
  script, and how the slot table and root storage are laid out at link time.
- **Execution and scheduling.** Run the cooperative task scheduler from a main loop or a timer
  ISR; define cycle overrun, watchdog, and interrupt-latency behavior. There is no preemption and
  no threading.
- **Memory and traps.** No heap or a fixed allocator; an explicit stack budget; the hosted
  `setjmp`/`longjmp` trap boundary is not available, so a trap halts, resets, or reports a code
  instead of rolling back. Retained state is out of scope unless a user flash/EEPROM hook is
  added.
- **Build and packaging.** An additive emit mode or a documented static-link recipe, target
  triple support, a flash/RAM footprint report, and no host filesystem paths at run time.
- **Diagnostics.** No DWARF or debug service in the first slice. Where the target provides
  semihosting or a UART, report the source span and trap code; a minimal serial trace is a later
  addition.
- **Profile subset.** Determine whether the full `stc-2013-oop-utf8-v1` profile fits the smallest
  target. If it does not, specify an explicit, documented subset (for example strings or OOP)
  rather than changing semantics silently.
- **Evidence.** QEMU execution tests over the shared corpus, at least one hardware-in-the-loop
  board run per target, deterministic mock-clock tests, timer-based cycle and jitter
  measurements, footprint measurements, and negative tests for unsupported services and build
  forms.

**Constraints and current state.**

- The cross-target matrix compiles IR and objects for nine triples, including
  `thumbv7em-none-eabihf` and `riscv64-unknown-elf`, but only inspects object headers. Execution,
  module loading, and frame extraction are native-host-only.
- `crates/stc-runtime` is `std`-based and owns the GOT, root allocator, module loader, manifest
  store, process image, swap controller, retained state, control/status thread, and debug server;
  traps use a `setjmp`/`longjmp` boundary. A tiny host is a new runtime profile, not a
  configuration toggle.
- D-200 supports only 64-bit little-endian native hosts. An MCU target is cross-compiled and
  run-only; the compiler and editor stay on Windows and Linux. The 32-bit pointer width and the
  target-specific layout hash must be tested, not assumed.
- Online change, rollback, debug sessions, file-backed state, and the EtherCAT, Modbus, and MQTT
  backends are out of scope. The tiny host keeps the process-image and host-API contracts, not
  the deployment services.
- `stc` is not a safety PLC. A watchdog, reset, or halt is not a safety function, and this item
  makes no hard-real-time or safety claim.
- The Lean verification program covers the source and IR semantics and the exact scalar kernel.
  It does not prove the tiny host, the timer/ISR path, or the board integration; evidence for
  those is test- and measurement-based.
- Rust `core`/`no_std` support, clang/lld, and the vendor startup and linker files are
  prerequisites. Targets with no Rust support at all are WL-20 territory.

**Open questions.**

- Which target comes first: Cortex-M4F (`thumbv7em-none-eabihf`) or a 32-bit RISC-V, and which
  QEMU/board pair is the reference?
- Is the tiny host a `no_std` feature of `stc-runtime`, a new `stc-runtime-tiny` crate, or a C
  shim? How much GOT, root, and image logic can be shared without destabilizing the hosted
  runtime?
- Does a tiny module reuse host ABI 2 with a documented required subset, or does it get a
  separate `stc-tiny` ABI profile?
- What is the static build artifact: a new `--emit=tiny`, an object-plus-archive recipe, or a
  generated linker/startup bundle? How are the slot table and metadata supplied?
- How is the process image bound on a target with no OS: static RAM buffers, memory-mapped
  addresses, or user glue?
- Which ABI minor must a tiny module require, and can test builds (`{stc:test}` on QEMU or
  hardware) use the mock clock?
- What does a trap do without unwinding: halt, reset, or report and continue? What is the
  watchdog policy?
- Is retained state out of scope, or does the tiny host expose a flash/EEPROM save/load hook?
- Which profile features must fit (strings, OOP, 64-bit time), and what is the flash/RAM budget?
- What evidence makes a target "supported": QEMU only, one HIL board, or a board matrix?
- Does the cycle run in the main loop or a timer ISR, and how is a missed deadline reported?
- How are target-specific C ABI and startup differences kept out of the front end and IR
  builder, consistent with D-042 and D-201?

**Related.** [Architecture](specification/Architecture/Architecture.md) §4.2 and §6.4,
[Runtime and Execution](specification/Architecture/Runtime-and-Execution.md) §1–§2,
[GOT Artifacts](specification/Architecture/GOT-Artifacts.md),
[Online Change and GOT](specification/Architecture/Online-Change-and-GOT.md),
[Known limitations §2](KNOWN-LIMITATIONS.md),
[Verification Program Plan](specification/Verification/VerificationProgramPlan.md), D-042,
D-200, D-201, WL-20.

**Tentative size.** L/XL. The first QEMU/board slice is bounded; the `no_std` host, static link
model, and target evidence dominate.

### WL-20 — Optional C backend for microcontrollers outside LLVM

**Intent.** Record a possible second code-generation backend that emits portable C instead of
textual LLVM IR, so an existing vendor C compiler can target small microcontrollers that LLVM
does not support (for example 8051, PIC16, RL78, or STM8). The intent is to reach parts with no
viable LLVM or Rust backend, not to replace the LLVM path for targets that already work.

**Status.** Candidate (2026-11-05). New; not part of the v3.0.0 target set. It depends on WL-19
and cannot be selected without an explicit amendment to the ground rules' no-codegen-redesign and
no-runtime-architecture-rewrite clauses.

**Why it matters.** LLVM has backends for ARM Cortex-M, RISC-V, AVR, and MSP430, but many of the
smallest and cheapest controllers have only a vendor C toolchain. Emitting C is the
practical route to those parts without writing an LLVM backend or a new Rust target. Recording
the direction keeps the option visible, but the cost is a second output contract, a second
runtime implementation, and a second semantics story; it should be judged against simply
declaring those targets out of scope.

**Candidate scope.**

- **C emitter.** Generate C over HIR as an alternative to the textual IR builder: C99 plus a
  documented vendor-extension boundary, explicit checked-arithmetic, trap, and conversion
  helpers, and no reliance on undefined behavior or compiler-specific intrinsics where a
  portable form exists.
- **Portable tiny runtime.** Implement the tiny host ABI subset from WL-19 in portable C (or a
  C-callable shim over a shared core where the target supports it), with static GOT and root
  storage and no OS, loader, or threads.
- **Per-target adapter.** Vendor compiler invocation and flags, C standard/dialect, memory model
  (near/far, banked memory, 8- and 16-bit pointers), startup and vector table, linker script,
  and library conventions.
- **First bounded spike.** Pick one target family for toolchain availability and CI automation;
  do not build a general multi-vendor backend before that spike succeeds.
- **Differential and target evidence.** Compare generated C behavior against the LLVM backend on
  the host for the shared language subset; use an emulator or HIL where available; commit
  generated-C snapshots and footprint measurements.
- **Trust boundary.** State what is verified (ST semantics, C emitter, runtime), what is assumed
  (vendor C compiler, startup code, hardware), and the evidence level for each claim.
- **Explicit rejection.** Reject constructs outside the supported profile/subset at build time
  rather than risking a silent miscompile.

**Constraints and current state.**

- The compiler's primary output is textual LLVM IR, and the Lean verification program proves
  source and IR semantics and the exact scalar kernel. A C backend leaves that boundary unless a
  C-level model or refinement argument is added; the existing proofs are not evidence for
  emitted C.
- The current ground rules prohibit codegen redesign and runtime architecture rewrites. This
  item is recorded as a possible exception only; selecting it is an explicit re-plan.
- `stc-runtime` is Rust `std`; the smallest targets have no Rust toolchain, so a second runtime
  implementation is implied. It must track the host ABI, GOT, root, image, and descriptor
  contracts or the targets will diverge.
- Vendor C compilers differ in integer promotion, pointer width, code model, extensions,
  licensing, availability, and CI support. Some are Windows-only, GUI-licensed, or not
  redistributable; an automated matrix may not be possible for every family.
- 8- and 16-bit parts stress the type model: small pointers, banked address spaces, limited
  stack, no native 64-bit arithmetic, and small flash. The profile's 64-bit `TIME` types and
  4-byte `UCHAR`/`USTRING` storage may be impractical; a documented subset and build-time checks
  would be required.
- Online change, debug sessions, file-backed state, and the v2 I/O backends remain out of scope.
  A C backend does not make them available.
- A C backend must not weaken the project's verification claims, determinism, or non-safety
  stance, and must not claim parity with the LLVM path.

**Open questions.**

- Is a C backend worth the second semantics and trusted base, or should LLVM-less targets stay
  out of scope?
- Which family is the first spike: 8051/SDCC, PIC16/XC8, RL78, STM8, or another? Which vendor
  toolchain is automatable and licensed for CI?
- Which C standard and subset, and which vendor extensions, are allowed? How are unsupported
  constructs diagnosed?
- How is C-level semantics tied to ST semantics: a formal C model, a refinement argument,
  differential testing, or an explicitly smaller claim?
- Does the tiny runtime exist in C for these targets, or can a C-callable ABI over a Rust core
  be used where Rust supports the target?
- How are 8- and 16-bit pointers, banking, and 64-bit ST types represented without changing the
  language profile?
- What is the evidence plan where no emulator exists and HIL is expensive or unavailable?
- How is the C backend kept from diverging from the IR backend: shared HIR and lowering, golden
  C-versus-IR tests, one conformance corpus?
- Is the C backend only a portability or bootstrap path while LLVM remains primary, or would it
  become a supported backend with its own guarantees?
- What would retire this candidate, for example LLVM gaining the target or a vendor providing a
  Rust toolchain?

**Related.** [Architecture](specification/Architecture/Architecture.md) §4 and §6,
[GOT Artifacts](specification/Architecture/GOT-Artifacts.md),
[Verification Program Plan](specification/Verification/VerificationProgramPlan.md),
[Known limitations §2](KNOWN-LIMITATIONS.md), D-042, D-200, WL-19.

**Tentative size.** XL for a usable backend; a single-target spike is S/M and should be
evaluated before any commitment.

### WL-21 — Compiler AI friendliness: benchmark-first MCP surface for ST

**Intent.** Make `stc` the authoritative tool surface for AI agents that write, navigate, and
repair Structured Text and operate the runtime. The work starts with a deterministic ST
benchmark and a measured raw-text baseline. Only after that baseline is accepted does it expose
the existing compiler and runtime phases as a stateless MCP (Model Context Protocol) service:
parse, check, build/run/test, semantic queries, node-addressed CST editing, and a runtime surface
for I/O discovery and configuration, cold download, and permission-gated online change. Each
capability is then re-measured against the same corpus, so the item produces evidence for the
marginal value of every exposed tool rather than one bundled claim. The ST language, profile,
artifact formats, runtime behavior, and safety stance do not change; the work is instrumentation
and APIs around the existing compiler and runtime.

**Status.** Candidate (2026-11-09). New; not part of the v3.0.0 target set. Benchmark-first: the
corpus, verifiers, harness, and raw-text baseline are Phase 1 and must be committed and accepted
before any MCP implementation starts. Size XL.

**Why it matters.** The dominant failure mode of AI-authored code is not a syntax error the
compiler catches; it is a silent semantic failure: code that compiles and runs but is wrong
because the model guessed at structure, types, or the available API. `stc` already owns the
facts an agent needs: stable diagnostic codes (`STC-E-*`, `STC-W-*`), machine-applicable
suggestions in the JSON diagnostic stream, a lossless CST with trivia and node identity, a
formatter over that CST, and IDE queries for definition, references, hover, and document
symbols. Exposing those facts through MCP replaces speculative text exploration with
authoritative, queryable answers, and node-addressed editing can make syntactic validity a
property of the edit operation rather than a hope. The PLC domain adds semantics that generic
code-graph tools do not model: POUs, function blocks, programs, configurations, tasks, global
variables, process-image channels, GOT roots, retained state, and online change.

The compiler is only half of the PLC loop. An agent that can check and edit ST but cannot see
the device tree, the effective I/O bindings, or the state of a running host is still guessing
about the plant. The runtime already owns the facts and operations that matter: `[io]` devices,
channels, `[[io.bindings]]`, binding precedence, process-image sizes, backend selection, cold
download, online change, compatibility reports, and retained state. Exposing them turns "wire
this variable to that channel and deploy it" into a sequence of inspectable, validated
operations. Online change is different from the other tools because it mutates a running
controller; it is denied to the agent by default and requires an explicit project-level grant.

The external evidence is genuinely mixed. Several studies report large token reductions and
correctness gains for AST editing and compiler-indexed semantic queries, while a 500-task
SWE-bench MCP study reported a lower resolution rate with fewer tokens. The lesson is not that
MCP works or fails; it is that utility is task- and surface-dependent and must be measured as a
two-number trade-off: task resolution and tokens-to-verified-success. A capability that cuts
tokens while reducing correctness has negative utility and should be redesigned or dropped.

**Evidence and design stance.**

- The benchmark is the product. No MCP tool is accepted on intuition; each capability is a
  separate treatment with its own paired measurement against the baseline and against the
  previous capability set.
- External results are context, not evidence for `stc`. The item records the design rationale
  and then measures this compiler, these tasks, and these agent scaffolds.
- MCP does not update model weights. The model learns the tool surface in context from tool
  names, descriptions, and input schemas. Tool-description quality and worked examples are
  therefore part of the implementation, not documentation polish.
- Report correctness and efficiency together. A single score hides the failure mode the item is
  meant to detect.
- Pre-register the corpus, arms, models, scaffolds, prompts, metrics, and stopping rules before
  the first measured run. Results are generated from raw logs, not hand-entered.

**Phase 0 — scope and design decisions.** Resolve the benchmark taxonomy, arm design, agent
scaffolds, model pins, metrics, statistics, MCP transport, statelessness model, artifact-handle
representation, node identity, runtime scope, the online-change permission model, the
cold-download confirmation model, and security posture. Record durable answers as decisions. No
MCP implementation starts in this phase.

**Phase 1 — benchmark first (no MCP).**

- Build a benchmark harness, for example under `tools/ai-bench/`, with a pre-registered
  `DESIGN.md`, task fixtures, verifiers, proxy logging, and analysis scripts. Evaluate an
  off-the-shelf harness first (for example mcpbr, agentrial, or DynamicMCPBench); build a small
  custom runner only where ST project fixtures and the existing `stc` CLI cannot be driven.
- Port an initial corpus of 20–50 programming challenges to ST. Start with classic algorithmic
  tasks (arithmetic, arrays, strings, sorting, searching, state machines, recursion where the
  profile supports it), then add ST/PLC tasks (timers and counters, edge detection, PID and
  ramps, alarms and sequencing, conveyor and traffic-light control, I/O mapping, retained state,
  recipes, filtering, unit conversion) and OOP tasks (function blocks, interfaces, methods,
  inheritance, polymorphism).
- Add project-scale tasks that exercise the surfaces the MCP tools will expose: add or rename a
  symbol, change a type across files, fix a failing test, repair a semantic diagnostic, repair a
  runtime trap, add a source set or task to `stc.toml`, and write a missing test.
- Add runtime tasks: discover the device tree and effective bindings, add or change a binding,
  diagnose a missing device or channel, plan a compatible change, cold download, and online
  change. In the MCP arm, online change without a project grant must fail, cold download
  without confirmation must fail, and a task that grants itself the permission must be scored
  as a failure. The baseline arm keeps the existing CLI behavior; the harness reports MCP
  permission violations separately from task resolution.
- Include "doomed" tasks that cannot succeed as specified, to measure failure cost rather than
  only success cost.
- Each task ships as a self-contained project with a prompt, a starting commit, hidden tests,
  and a deterministic verifier. Score by reading the repository state back and running
  `stc check`, `stc build`, `stc test`, and behavioral checks; never accept the agent's own
  claim of success.
- Measure the raw-text baseline arm: file read/write/edit, search, shell, and the existing
  `stc` CLI. This arm must not use MCP. Record resolution rate, tokens, tool calls, wall-clock
  time, corrective turns, and cost per resolved task.
- Hold out an evaluation split that is not shown during harness development, and record task
  provenance and licensing. Port challenges as original ST implementations; do not copy
  copyrighted prompts or fixtures.
- Exit gate: the corpus, verifiers, harness, pre-registration, and baseline report are committed
  and accepted by the owner. No MCP code is written before this gate.

**Phase 2 — MCP core: parse and check.**

- Add a stdio MCP server, preferably `stc mcp --stdio` in the existing driver so the host points
  at a transparent compiler binary rather than an opaque wrapper. JSON-RPC 2.0, no network
  listener, no hidden session state.
- Expose `stc_parse` (syntax only) and `stc_check` (parse plus semantic analysis). Reuse the
  existing `--message-format=json` diagnostic schema: code, severity, message, file, span,
  labels, notes, and suggestions. Add a versioned response envelope and, where available, a
  stable repair kind or `repair` object derived from the existing suggestions; do not rename the
  existing diagnostic codes.
- Keep the stages separable so the agent can choose the cheapest sufficient check: parse,
  check, build, run, test.
- Preserve labels and notes so the first diagnostic carries the cause chain. The benefit lands
  on the agent's first hypothesis, not on iterative retries, so a detailed first message is
  worth more than a terse one.
- Re-run the benchmark. Report the marginal delta versus the baseline. Keep the capability only
  if it improves the two-number trade-off or produces a documented correctness gain.

**Phase 3 — build, run, and test with artifact handles.**

- Expose `stc_build`, `stc_run`, and `stc_test`. `stc_build` returns an opaque artifact handle
  and metadata; `stc_run` and `stc_test` accept the handle. Compiled bytes never enter the
  model's context window.
- Scope handles to the workspace and make them explicit, content-addressed, or build-id-based
  values; a handle is not hidden server state. Reject stale or foreign handles.
- Return runtime output, traps, and test results in structured JSON, with source spans where
  available.
- Re-run the benchmark and report the marginal delta.

**Phase 4 — semantic queries (the highest-value surface).**

- Expose `stc_context` (project summary: packages, configurations, tasks, POUs, function
  blocks, programs, global variables, process-image channels, GOT roots, dependencies),
  `stc_find_symbol`, `stc_get_type`, `stc_get_callers`, `stc_get_references`,
  `stc_get_dependencies`, and `stc_list_symbols`.
- Reuse the `stc-ide` query layer rather than forking resolution or type checking. Return
  definitions, resolved types, visibility, owners, and edges; return bodies only when the caller
  asks for them, so a symbol lookup does not dump an entire file.
- Use typed input schemas and worked examples. Test whether the agent actually calls the tool
  and whether the answer changes its behavior; a tool that the agent ignores is not a benefit.
- Re-run the benchmark and report the marginal delta.

**Phase 5 — CST inspection and node-addressed editing.**

- Expose `stc_inspect_node`, `stc_update_node`, `stc_insert_node`, `stc_delete_node`, and
  `stc_fmt`.
- Address nodes by a stable symbolic path plus a content hash, for example
  `file::POU::symbol` plus the file hash, rather than exposing the internal `NodeId` across
  calls. Reject a stale path/hash instead of editing the wrong node.
- Use the lossless CST and the existing formatter so trivia, comments, and formatting outside
  the edited node survive. After each mutation, re-parse and re-check the file; reject an edit
  that does not produce a valid tree. This gives syntactic validity by construction where the
  CST and formatter support it, and by verification everywhere else.
- Add incremental re-analysis or a content-addressed parse/check cache so the edit-verify loop
  stays interactive. The cache is an optimization, not observable session state.
- Re-run the benchmark and report the marginal delta.

**Phase 6 — runtime surface: I/O and deployment.**

- Expose read-only runtime facts first: `stc_runtime_status` (host state, active generation and
  build, compatibility, retained-state summary), `stc_io_discover` (devices, kinds, channels,
  directions, widths, bindings, resolved variables, bound/unbound status, backend, process-image
  sizes), and `stc_io_get_config` (effective configuration and binding precedence: runtime
  override > `[[io.bindings]]` > `{stc:io}` pragma > raw `%` address).
- Add `stc_io_validate` for a proposed device or binding change. It returns diagnostics and a
  compatibility result, including whether the change requires a cold download. Add
  `stc_io_apply` only if the benchmark shows value; it edits `stc.toml` through the same
  validated path, never talks to hardware directly, and never mutates a running host by itself.
- Expose `stc_online_diff` read-only: compare a candidate build against the active baseline and
  report compatibility, moved variables, layout changes, and the cases that require a cold
  download.
- Expose `stc_deploy_cold` and `stc_deploy_online`. Cold download stops and reinitializes the
  program and follows the existing `[online] cold-policy = "require-confirmation"`; the MCP
  server must not silently confirm. Online change is denied by default and is available only
  when the project explicitly grants it in `stc.toml` (for example
  `[mcp] allow-online-change = true`; the exact section, key, and schema are resolved in
  Phase 0). The agent cannot grant the permission, the MCP server must not write it, and a
  project without the grant gets a structured permission error rather than a hidden fallback.
- Keep runtime tools local by default. Remote deployment stays behind the existing SSH/remote
  contract and is either out of scope or a separate, separately gated surface.
- Include negative tasks in the benchmark: online change without the grant must be refused;
  cold download without confirmation must be refused; a stale or foreign handle must be
  rejected; a topology change must report cold-download-required rather than attempting a swap.
- Re-run the benchmark and report the marginal delta for each runtime capability.

**Phase 7 — discovery and scaffolding.**

- Have `stc new` write project-local discovery files: `.mcp.json`, `.cursor/mcp.json`,
  `.vscode/mcp.json`, and `AGENTS.md` (optionally `CLAUDE.md`), pointing at
  `stc mcp --stdio`. Do not overwrite existing files; warn and skip instead.
- The host application launches the server from its configuration; the model never starts it.
  A `--help` message is not a discovery mechanism.
- Point the generated entry at the compiler binary with an explicit subcommand, never a shell
  script or opaque launcher, so the trust prompt shows exactly what will run. Document the trust
  prompt.
- Measure the scaffold's effect separately from the tool surface; discovery and tool quality are
  different variables.

**Tool surface (tentative).**

| Tool | Purpose | Returns |
| --- | --- | --- |
| `stc_parse` | Syntax-only validation | Lossless CST or typed AST summary, syntax diagnostics |
| `stc_check` | Parse plus full semantic analysis | Structured JSON diagnostics with stable codes and repairs |
| `stc_build` | Check plus code generation | Opaque artifact handle and build metadata |
| `stc_run` | Execute a built artifact | Output, traps, and runtime result |
| `stc_test` | Run project tests | Per-test results and failure diagnostics |
| `stc_context` | Summarize a project | Packages, configurations, tasks, POUs, globals, I/O, GOT roots, dependencies |
| `stc_find_symbol` | Resolve a name | Definition location, kind, type, visibility, owner |
| `stc_get_type` | Resolve an expression or identifier | Resolved type and layout information |
| `stc_get_callers` | Call-edge query | Callers and call sites |
| `stc_get_references` | Reference query | Definitions and references with spans |
| `stc_get_dependencies` | Dependency query | Module/package dependency edges |
| `stc_list_symbols` | Symbol table for a file or project | Symbols with kinds and locations |
| `stc_inspect_node` | Inspect a CST node | Kind, span, children, resolved type, symbol |
| `stc_update_node` | Replace a node | Updated file or edit result |
| `stc_insert_node` | Insert a node | Updated file or edit result |
| `stc_delete_node` | Delete a node | Updated file or edit result |
| `stc_fmt` | Format a file or node | Formatted source or edit result |
| `stc_runtime_status` | Inspect a running host | State, active generation/build, compatibility, retained-state summary |
| `stc_io_discover` | Discover the device tree | Devices, channels, bindings, resolved variables, bound/unbound, backend, image sizes |
| `stc_io_get_config` | Read effective I/O configuration | `[io]` config, binding precedence, runtime overrides |
| `stc_io_validate` | Validate a proposed I/O change | Diagnostics, compatibility, cold-download requirement |
| `stc_io_apply` | Apply a validated I/O change | Updated `stc.toml` and edit result; never touches a host |
| `stc_online_diff` | Compare a candidate with the active baseline | Compatibility, moved variables, layout changes, cold-download cases |
| `stc_deploy_cold` | Cold download | Stop/reinitialize result, installed generation, retained-state outcome |
| `stc_deploy_online` | Online change | Swap result; denied unless the project grants permission |

**Benchmark design.**

- **Paired arms, identical conditions.** Arm A is the raw-text baseline (read, search, edit,
  shell, and the existing CLI). Arm B adds MCP tools. Hold the model, scaffold, prompts,
  starting commit, and time budget fixed.
- **Per-capability ablation.** Add one tool category at a time to the baseline and measure its
  marginal effect. The final bundled MCP arm is reported, but it is not the only comparison.
- **Runtime tasks and permissions.** Treat the permission boundary as part of the treatment:
  measure the allowed path and the denied path. A runtime tool that can be talked into a
  forbidden online change, or that lets the agent add its own grant, is a failure, not a
  capability.
- **Information parity and its limit.** Both arms can read the same repository and run the same
  compiler. The MCP arm additionally receives semantic and runtime facts; that is the treatment,
  not a hidden information advantage. For each capability, record exactly which facts the tool
  exposes so the delta is attributable.
- **At least two agent scaffolds.** The optimal surface depends on the harness and model.
  Pin scaffold versions and model identifiers; randomize task order; keep prompts identical.
- **Metrics.** Task resolution rate (primary correctness), tokens-to-success (total input and
  output, including retries, with cached and uncached tokens separate), tool calls, wall-clock
  time, corrective-turn ratio, cache-adjusted cost per resolved task, and failure cost on
  doomed tasks. Log every tool call through a proxy so a scaffold cannot hide usage in a
  sub-agent.
- **Statistics.** Minimum three runs per task per arm; five or more for a formal comparison.
  Report Wilson 95% confidence intervals for pass rates, paired Wilcoxon signed-rank tests for
  continuous metrics, and Benjamini-Hochberg correction when comparing many surfaces.
- **Minimum credible benchmark.** Thirty tasks with deterministic verifiers; three trials per
  task; two paired arms with the same model and scaffold; every tool call proxied; and three
  reported numbers per arm: resolution rate with a 95% Wilson interval, median
  tokens-to-success, and cache-adjusted cost per resolved task. Pre-register the task set and
  metrics before running.
- **Artifacts.** Commit the pre-registration, task hashes, raw JSONL logs, analysis scripts, and
  generated summary. Recompute every published number from the raw logs.
- **Decision rule.** Keep a capability when resolution does not regress and tokens-to-success
  improves, or when a documented correctness gain justifies a cost. Redesign or drop it
  otherwise. Record the decision and the evidence in the item's working plan.

**Constraints and current state.**

- The language profile, artifact formats, runtime contracts, and safety stance do not change.
  The compiler and runtime work is instrumentation: a stable MCP-facing API, a versioned
  diagnostic envelope, optional repair metadata, node-addressed editing over the existing CST,
  and read/validate/deploy wrappers over the existing runtime and online-change contracts. No
  codegen redesign and no runtime architecture rewrite.
- ST source remains the canonical program form. There is no JSON-AST or other alternate
  canonical representation; the MCP surface queries and edits the existing lossless CST and
  source files.
- The assets already exist and should be reused: stable `STC-E-*`/`STC-W-*` codes, JSON
  diagnostics with spans and suggestions, the lossless CST, the `stc-fmt` formatter, the
  `stc-ide` definition/reference/hover/document-symbol layer, `stc.toml` device and binding
  metadata, `stc online download`/`diff`, and the online-change compatibility report. The LSP
  and DAP remain the human IDE surfaces; MCP is a complementary agent surface and must not fork
  semantic analysis.
- `stc-lsp` does not yet provide rename, pull diagnostics, code actions, semantic tokens, or
  range formatting. WL-18 (rename) is a natural reuse when delivered, not a hard prerequisite.
- Statelessness is about hidden session state, not about ignoring the workspace. Each MCP call
  is a pure function of the explicit arguments and the workspace on disk; artifact handles are
  explicit, workspace-scoped values. Passing the full source set on every call, as a small
  stateless service might, is impractical for a project with `stc.toml`, `stc.lock`, multiple
  source sets, and generated artifacts. Document the chosen model and test it for determinism.
- Security posture: stdio only, no network listener, project-root confinement, no arbitrary
  shell execution, no writes outside the workspace, and a visible launch command. Project-local
  MCP configuration is untrusted by default; expect the host's trust prompt.
- Runtime mutations are a separate trust class. Online change is denied by default and requires
  an explicit, committed project-level grant in `stc.toml`; the agent cannot grant it, and the
  MCP server must not write the grant. A diff that adds or widens the grant is rejected. Cold
  download follows the existing `[online] cold-policy = "require-confirmation"` and must not be
  silently confirmed. The permission and confirmation surfaces are resolved in Phase 0.
- I/O configuration is project data, not hardware access. `stc_io_apply` edits `stc.toml`
  through the validated manifest path; it does not open devices or schedule I/O. Topology and
  binding changes that cannot be preserved are reported as cold-download-required, and the
  existing binding precedence (runtime override > `[[io.bindings]]` > `{stc:io}` pragma > raw
  `%` address) is preserved.
- Runtime tools are local by default. Remote deployment remains the existing SSH/remote contract
  and is not silently widened by the MCP surface.
- Benchmark validity risks: task overfitting, a corpus that favors the tools, model and scaffold
  drift, incomplete token accounting, and publishing a single run. Mitigate with a held-out
  split, pinned versions, proxy logs, multiple scaffolds, and pre-registration.
- Static semantic queries describe what the compiler can resolve. For behavior that depends on
  runtime values, dynamic dispatch, or external I/O, keep `run`/`test` and a focused reproduction
  in the loop; the static map is a navigation aid, not a proof of behavior.
- AI-authored ST still requires tests and human review. MCP improves the tool surface; it does
  not make the output correct, and it does not change the project's non-safety stance.
- New code targets 95% line coverage; the project coverage gate is 85%.

**Open questions.**

- Which agent scaffolds and models are the reference set, how are they pinned, and how is
  reproducibility handled when a hosted model changes?
- What is the initial task count and category mix, and what is the public/held-out split? Which
  classic challenges port cleanly to ST, and which ST/PLC tasks best exercise the compiler?
- Is the MCP server a mode of the existing driver (`stc mcp --stdio`), a separate
  `stc-mcp` crate, or a thin wrapper? Which JSON-RPC/MCP library, and which protocol version?
- What is the exact statelessness contract: workspace mode, explicit source-set mode, or both?
  How are artifact handles represented, invalidated, and scoped?
- How granular should the tools be: separate parse/check/build/run/test, or fewer composite
  tools? Which shape produces better tool selection and fewer wasted calls?
- Should diagnostics gain an explicit stable repair kind, or are the existing suggestions
  sufficient? What is the JSON schema version and compatibility policy?
- How are node paths and content hashes defined across files and edits, and what happens when a
  file changes between inspect and update?
- Which semantic queries matter most for ST: symbols and types, call edges, task/config
  relationships, I/O channel mapping, GOT roots, retained-state ownership, or online-change
  compatibility?
- Which runtime facts come from the compiler metadata, which from the live host, and which from
  `stc.toml`? How are they reconciled when they disagree?
- What is the exact project-level permission key and scope for online change, and how is it
  reviewed and committed? Is it per project, per host, or per task? How does the MCP server
  prove the grant was made by a human/owner rather than by the agent?
- Does default-deny apply only to the MCP surface, or must the CLI also refuse an online change
  when it is driven by an agent? If the agent has shell access, how is that boundary enforced
  and measured?
- How is cold-download confirmation surfaced through an MCP host: elicitation, an explicit tool
  argument, a one-time approval token, or a CLI action outside MCP?
- Is `stc_io_apply` in scope, or is I/O configuration read-only with edits going through normal
  file editing? Which I/O changes are safe online and which force a cold download?
- Are remote hosts in scope for the runtime surface, and if so, how does the existing SSH/remote
  contract compose with the MCP permission model?
- Should online change expose cancel/rollback, or only the forward swap, with recovery left to
  `stc online` or cold download?
- How should the benchmark score partial progress, test-writing tasks, and doomed tasks? How are
  failures attributed to syntax, semantics, runtime, tests, or specification misunderstanding?
- Is `stc new` scaffolding on by default or opt-in? Which host config files are written, and how
  are existing files preserved?
- What improvement justifies keeping a capability, and what evidence retires the item? Is a
  token reduction with equal resolution enough, or is a correctness gain required?
- Does the MCP surface duplicate the LSP enough to justify sharing a query API, or should the
  MCP server depend on `stc-ide` directly?
- Does this item need an explicit ground-rule exception, or is it covered by the existing
  "instrumentation required by a selected item" clause?

**Related.** [IDE Integration](specification/Architecture/IDE-Integration.md),
[CLI and Manifest](specification/Architecture/CLI-and-Manifest.md),
[Architecture](specification/Architecture/Architecture.md),
[Runtime and Execution](specification/Architecture/Runtime-and-Execution.md),
[I/O and Devices](specification/Architecture/IO-and-Devices.md),
[I/O Routing and Backends](specification/Architecture/IO-Routing-and-Backends.md),
[Online Change and GOT](specification/Architecture/Online-Change-and-GOT.md),
[State and RETAIN](specification/Architecture/State-and-Retain.md),
[Testing and Verification](specification/Architecture/Testing-and-Verification.md),
[Decisions](specification/Decisions.md), [USER-GUIDE.md](USER-GUIDE.md),
[KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md), WL-1.1 (delivered IDE), WL-17 (manifest and task
follow-ups), WL-18 (rename), and the feature-development workflow in
[workflows/FEATURE-DEVELOPMENT.md](workflows/FEATURE-DEVELOPMENT.md).

**Tentative size.** XL. The benchmark corpus and harness are M/L and come first; the MCP core is
S/M; semantic queries are M; CST editing and incremental re-analysis are M/L; the runtime surface
(I/O discovery and configuration, cold download, gated online change) is M/L; repeated
measurement and analysis dominate the schedule. A benchmark-only spike (Phase 1) is worthwhile
before committing to the full item.

## 6. Cross-cutting notes

- The v2.0.0 release delivered the local IDE, RT evidence, Modbus TCP server, and EtherCAT
  backend. The candidates and selected items build on those contracts rather than re-planning
  them.
- WL-1.2 remote debugging builds on the delivered WL-1.1 local debugger and shares its remote
  authentication design with the deferred WL-8 tracing UI and WL-9 WebUI.
- WL-6 motion uses the delivered WL-5.2 EtherCAT backend and the deferred WL-7 ESI
  configuration. Modbus TCP is general I/O, not an interpolated motion channel.
- WL-11 is the Modbus TCP client counterpart to the delivered WL-5.1 server. It has different
  outbound scheduling and failure behavior and should not be forced into the server code path.
- WL-12 is a host/runtime scheduling investigation that reuses the delivered WL-5.2 CT-sweep
  fixture and WL-4 tooling; it absorbs the deferred WL-4 tuned boot profile evaluation and does
  not change the EtherCAT backend or the safety stance.
- WL-13 extends the delivered WL-3 Lean program to the WL-5.1 protocol/register-bank boundary.
  It verifies protocol and write-authorization invariants, not authentication or TLS, and does
  not make the server a safety function.
- WL-14 adds macOS ARM64/x86-64 as supported native hosts at Windows parity, excluding EtherCAT
  and PREEMPT_RT-style hard real time. It is selected for v3 and uses the existing WL-4 harness
  for opt-in scheduling experiments; WL-12 is not a prerequisite. No hard-real-time or safety
  claim is made.
- WL-15 is an MQTT client for an external broker, not a broker or server. It shares the delivered
  multi-backend coordinator and needs an explicit payload, stale/cache, and security model.
- WL-8 is a module of the deferred WL-9 WebUI, not a separate front end; both use the WebUI's
  authentication, packaging, and deployment and must not bypass the runtime control or debug
  contracts.
- WL-16 and WL-17 cover the remaining WL-1.1 follow-ups deferred when v2.0.0 shipped: F-9 and
  F-15 in WL-16, F-23 and F-24 in WL-17; F-19 is WL-18, and F-21 is part of WL-1.2. F-22 was
  fixed after v2.0.0.
- The v3.0.0 target set is locked: WL-1.2, WL-7, WL-14, WL-15, WL-16, WL-17, and WL-18
  (2026-11-04); see [v3.0.0-plan.md](v3.0.0-plan.md).
- WL-19 is a bare-metal run target, not a new supported development host, and does not change
  the native host matrix, the language profile, or the safety stance. It reuses the existing
  cross-target IR/object work and keeps the hosted runtime's process-image and host-API
  contracts where they make sense.
- WL-20 is a research direction that would add a second code-generation backend; it depends on
  WL-19's tiny-runtime contract and requires an explicit ground-rule exception. It is not a
  second supported compiler path until selected, specified, and evidenced.
- WL-21 is an additive instrumentation item: it exposes the existing parse, check, build, run,
  test, IDE-query, I/O, and deployment layers through MCP and adds a benchmark to measure each
  capability. It does not change the ST language, code generation, artifact formats, or runtime
  behavior, and it complements the LSP and DAP rather than replacing them. Online change is
  denied by default and requires an explicit project-level grant; the agent cannot grant it.
  Benchmark-first ordering is part of the item: no MCP implementation before the raw-text
  baseline is measured.
- The remaining items are additive around a stable compiler. None changes the ST language or the
  core compiler architecture unless a selected item explicitly says so; WL-20 is recorded as a
  possible future exception and would require an explicit re-plan.

## 7. Backlog

All recorded items are elaborated above. Add new candidates below with the next `WL-` id, a
one-line intent, and the same structure when they are elaborated.

## 8. Change log

| Date | Change |
| --- | --- |
| 2026-11-09 | Added WL-21 (compiler AI friendliness): benchmark-first ST challenge corpus and raw-text baseline, then a stateless MCP surface for parse/check/build/run/test, semantic queries, node-addressed CST editing, and runtime I/O discovery/configuration, cold download, and permission-gated online change, re-measured after each capability. ST remains the canonical program form; there is no JSON-AST endpoint. Online change is denied by default unless the project grants it. Not part of v3.0.0. |
| 2026-11-09 | WL-14 merged into `v3.0.0-planning` (PR #27, merge `e99f33a`); remote and local `WL-14` branches deleted. |
| 2026-11-09 | WL-14 complete; manual check approved. Final CI run [42546445894](https://github.com/r2k-in-the-vortex/stc/actions/runs/42546445894) green on all jobs, including both macOS workspace, package, and extension-smoke jobs. |
| 2026-11-08 | WL-14 scope revised: macOS ARM64/x86-64 become supported native hosts at Windows parity, excluding EtherCAT and PREEMPT_RT-style hard real time; the full workspace, packages, and extension smoke are part of the claim. |
| 2026-11-08 | WL-19 was redefined on `v3.0.0-planning` as microcontroller support with a tiny runtime variant, with WL-20 as the optional C backend; the cross-target object-emission notes from the WL-14 discussion are folded into that direction. Neither item is in v3.0.0 or WL-14. |
| 2026-11-07 | WL-1.2 merged into `v3.0.0-planning` (PR #25, merge `d74bc89`); remote and local `WL-1.2` branches deleted. |
| 2026-11-07 | WL-1.2 complete; manual check approved; PR #25 merged (`d74bc89`). Final CI run [42397941737](https://github.com/r2k-in-the-vortex/stc/actions/runs/42397941737), 39/39. Remote launch/attach, online/cold deployment, QuickPick, HIL evidence, threat model, and documentation are complete. |
| 2026-11-07 | WL-1.2 phase 6 HIL complete and CI-green (run [42343615644](https://github.com/r2k-in-the-vortex/stc/actions/runs/42343615644), 39/39): launch, cycle-0 values, stepping, stack, function locals, globals, online/cold deployment, tunnel drop, and reattach verified on `r2k@ecat`; evidence in [Remote-Debug-Runbook.md](specification/Verification/Remote-Debug-Runbook.md). |
| 2026-11-07 | WL-1.2 phases 3–5 complete and CI-green (run [42341503422](https://github.com/r2k-in-the-vortex/stc/actions/runs/42341503422), 39/39): remote launch/attach over the SSH tunnel, online/cold deployment with the extension QuickPick, security negative tests, threat model, and runbook skeleton. |
| 2026-11-06 | Started WL-1.2 on branch `WL-1.2`; selected the SSH-tunnel transport to the remote loopback debug socket and recorded the phased plan in [v3.0.0-plan.md](v3.0.0-plan.md). |
| 2026-11-05 | WL-16 complete: launches are start-gated before cycle 0 and session teardown/exit is reported in the Debug Console; manual check approved. |
| 2026-11-05 | Added WL-19 (microcontroller support and a tiny runtime variant) and WL-20 (optional C backend for microcontrollers outside LLVM) as candidates. |
| 2026-11-04 | Locked the v3.0.0 target set (WL-1.2, WL-7, WL-14, WL-15, WL-16, WL-17, and WL-18) and created [v3.0.0-plan.md](v3.0.0-plan.md) as the working plan. |
| 2026-11-04 | Selected WL-16 (F-9, F-15) and WL-17 (F-23, F-24) for v3 (not yet locked). |
| 2026-11-04 | Fixed F-22 (`277f752`): deterministic project-load errors no longer restart the language server and now show the manifest error with Open Output/Retry. Split F-19 out of WL-17 as WL-18 and selected WL-1.2, WL-7, WL-14, WL-15, and WL-18 for v3 (not yet locked); WL-17 now covers F-23 and F-24. |
| 2026-11-04 | v2.0.0 released. Started v3 planning: retired the delivered v2.0.0 plan, moved its open follow-ups here as WL-16 (F-9, F-15) and WL-17 (F-19, F-22, F-23, F-24), folded F-21 into WL-1.2 and the deferred tuned boot profile into WL-12, and set the compatibility baseline to 2.0.0. |
| 2026-11-04 | v2.0.0 target set complete. Removed completed targets WL-1.1, WL-2, WL-3, WL-4, WL-5.1, WL-5.2, and WL-10; retained the deferred candidates below. |
| 2026-11-03 | Added WL-15 (MQTT client for process-image I/O) as deferred beyond v2.0.0. |
| 2026-11-02 | Added WL-13 (Lean security verification for the Modbus TCP server boundary) and WL-14 (macOS CI and Windows/macOS soft-real-time scheduling) as deferred beyond v2.0.0. |
| 2026-11-01 | Added WL-12 (cycle-thread wake-up jitter at sub-millisecond cycle times) as deferred. |
| 2026-10-29 | Created the post-v1 wishlist; added WL-1.2, WL-6, WL-7, WL-8, WL-9, and WL-11. |
