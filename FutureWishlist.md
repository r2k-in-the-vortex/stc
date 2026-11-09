# Future Wishlist — post-v2.0.0

Status: `stc` 2.0.0 is released (2026-11-04). The v3.0.0 target set is locked in
[v3.0.0-plan.md](v3.0.0-plan.md): WL-1.2, WL-7, WL-14, WL-15, WL-16, WL-17, and WL-18 (F-19).
The remaining candidates are WL-6, WL-8, WL-9, WL-11, WL-12, and WL-13; they stay here for a
later release. This file is deliberately not a roadmap, a commitment, or a release plan. The
delivered v2.0.0 scope and evidence are in [RELEASE-NOTES.md](RELEASE-NOTES.md) and
[RELEASE-CHECKLIST.md](RELEASE-CHECKLIST.md).

- Date: 2026-10-29; revised 2026-11-04 (v3 planning start)
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
| WL-14 | macOS native support and Windows/macOS soft-real-time scheduling for tests | Test platform/RT | M/L | WL-4 harness; CI runners; macOS parity | In progress (v3.0.0) |
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
| WL-19 | Cross-target object emission and runtime-side linking for constrained targets | Tooling/runtime | M/L | Object ABI; runtime linker/loader; cold/online artifact flow | Deferred |

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

### WL-19 — Cross-target object emission and runtime-side linking for constrained targets

**Intent.** Let a development host compile a project to target objects and let a constrained
runtime link or load them, so targets without a full clang or a full `stc` compiler can still run
artifacts produced on another host. Review the cold-download and online-change artifact flow so
it can carry objects/binaries instead of sources where the runtime cannot build.

**Status.** Deferred. Not part of v3.0.0 and not part of WL-14; recorded 2026-11-08 from the
WL-14 cross-platform discussion.

**Why it matters.** Planned microcontroller targets may not have a full clang/LLVM toolchain or
the `stc` compiler. The current cross-target surface is IR/object emission; executable and
embedding-library emission are host-only, and the documented rule is to link and run on the
target host. The WL-1.2 remote flow sends sources and builds natively on the runtime host. For a
constrained runtime, the development host must emit the target object and the runtime must only
link or load it. Cold download and online change need an artifact model that matches that split.

**Candidate scope.**

- Define the supported development-host → runtime target pairs and the object ABI, relocation,
  section, and symbol contract.
- Emit a target object plus manifest/GOT metadata on the development host; keep `stc` and clang
  off the runtime where required.
- Add runtime-side linking or loading of that object, including target, build-ID, and
  compatibility validation.
- Review cold download and online change: carry objects/binaries and metadata rather than
  sources when the runtime cannot build; define candidate compatibility, state migration, and
  rollback for that form.
- Evidence: cross-target object emission, runtime link/load tests, cold-download and
  online-change evidence, security/integrity checks, and documentation.

**Constraints and current state.**

- IR/object emission is supported for the cross-target matrix; `--emit=exe` and `--emit=lib`
  are host-only, and module linking is documented as target-host work.
- The runtime validates the module target; a host-default artifact is not portable.
- Microcontroller targets are future work; their object formats, memory models, and linkers
  differ from the current native hosts.
- WL-14 adds macOS ARM64/x86-64 as supported native hosts; it does not add a runtime or change
  the language profile.

**Open questions.**

- Which microcontroller targets and object formats come first; ELF/COFF or a custom image?
- Does the runtime link the object with a target linker, or does a host-side link produce a
  loadable image for the runtime to load?
- How are build ID, source fingerprint, GOT/manifest compatibility, and retained state defined
  for object-based cold/online deployment?
- What is the security and integrity model for objects crossing the development/runtime
  boundary?
- How are debug/frame sidecars produced when the target cannot run a full debug toolchain?

**Related.** [CLI and Manifest](specification/Architecture/CLI-and-Manifest.md),
[Online Change and GOT](specification/Architecture/Online-Change-and-GOT.md),
[Runtime and Execution](specification/Architecture/Runtime-and-Execution.md),
[KNOWN-LIMITATIONS §2](KNOWN-LIMITATIONS.md), WL-1.2 (delivered remote flow), WL-7, WL-15,
WL-14.

**Tentative size.** M/L, dominated by the object/link contract, runtime loading, and
cold/online artifact-flow changes.

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
- The remaining items are additive around a stable compiler. None changes the ST language or the
  core compiler architecture unless a selected item explicitly says so.

## 7. Backlog

All recorded items are elaborated above. Add new candidates below with the next `WL-` id, a
one-line intent, and the same structure when they are elaborated.

## 8. Change log

| Date | Change |
| --- | --- |
| 2026-11-08 | WL-14 scope revised: macOS ARM64/x86-64 become supported native hosts at Windows parity, excluding EtherCAT and PREEMPT_RT-style hard real time; the full workspace, packages, and extension smoke are part of the claim. |
| 2026-11-08 | Added WL-19 (cross-target object emission and runtime-side linking for constrained targets) from the WL-14 cross-platform discussion; it is not in v3.0.0 or WL-14. |
| 2026-11-07 | WL-1.2 merged into `v3.0.0-planning` (PR #25, merge `d74bc89`); remote and local `WL-1.2` branches deleted. |
| 2026-11-07 | WL-1.2 complete; manual check approved; PR #25 merged (`d74bc89`). Final CI run [42397941737](https://github.com/r2k-in-the-vortex/stc/actions/runs/42397941737), 39/39. Remote launch/attach, online/cold deployment, QuickPick, HIL evidence, threat model, and documentation are complete. |
| 2026-11-07 | WL-1.2 phase 6 HIL complete and CI-green (run [42343615644](https://github.com/r2k-in-the-vortex/stc/actions/runs/42343615644), 39/39): launch, cycle-0 values, stepping, stack, function locals, globals, online/cold deployment, tunnel drop, and reattach verified on `r2k@ecat`; evidence in [Remote-Debug-Runbook.md](specification/Verification/Remote-Debug-Runbook.md). |
| 2026-11-07 | WL-1.2 phases 3–5 complete and CI-green (run [42341503422](https://github.com/r2k-in-the-vortex/stc/actions/runs/42341503422), 39/39): remote launch/attach over the SSH tunnel, online/cold deployment with the extension QuickPick, security negative tests, threat model, and runbook skeleton. |
| 2026-11-06 | Started WL-1.2 on branch `WL-1.2`; selected the SSH-tunnel transport to the remote loopback debug socket and recorded the phased plan in [v3.0.0-plan.md](v3.0.0-plan.md). |
| 2026-11-05 | WL-16 complete: launches are start-gated before cycle 0 and session teardown/exit is reported in the Debug Console; manual check approved. |
| 2026-11-04 | Locked the v3.0.0 target set (WL-1.2, WL-7, WL-14, WL-15, WL-16, WL-17, and WL-18) and created [v3.0.0-plan.md](v3.0.0-plan.md) as the working plan. |
| 2026-11-04 | Selected WL-16 (F-9, F-15) and WL-17 (F-23, F-24) for v3 (not yet locked). |
| 2026-11-04 | Fixed F-22 (`277f752`): deterministic project-load errors no longer restart the language server and now show the manifest error with Open Output/Retry. Split F-19 out of WL-17 as WL-18 and selected WL-1.2, WL-7, WL-14, WL-15, and WL-18 for v3 (not yet locked); WL-17 now covers F-23 and F-24. |
| 2026-11-04 | v2.0.0 released. Started v3 planning: retired the delivered v2.0.0 plan, moved its open follow-ups here as WL-16 (F-9, F-15) and WL-17 (F-19, F-22, F-23, F-24), folded F-21 into WL-1.2 and the deferred tuned boot profile into WL-12, and set the compatibility baseline to 2.0.0. |
| 2026-11-04 | v2.0.0 target set complete. Removed completed targets WL-1.1, WL-2, WL-3, WL-4, WL-5.1, WL-5.2, and WL-10; retained the deferred candidates below. |
| 2026-11-03 | Added WL-15 (MQTT client for process-image I/O) as deferred beyond v2.0.0. |
| 2026-11-02 | Added WL-13 (Lean security verification for the Modbus TCP server boundary) and WL-14 (macOS CI and Windows/macOS soft-real-time scheduling) as deferred beyond v2.0.0. |
| 2026-11-01 | Added WL-12 (cycle-thread wake-up jitter at sub-millisecond cycle times) as deferred. |
| 2026-10-29 | Created the post-v1 wishlist; added WL-1.2, WL-6, WL-7, WL-8, WL-9, and WL-11. |
