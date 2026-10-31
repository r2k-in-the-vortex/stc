# Future Wishlist — v2.0.0

Status: v2.0.0 target set locked 2026-10-29 and re-planned 2026-10-31. Locked: WL-1.1, WL-2, WL-3 (P0.4), WL-4 (x86-64), WL-5.1, WL-5.2, and WL-10. Deferred: WL-1.2, WL-6, WL-7, WL-8, WL-9, and WL-11. Completed: WL-4 (2026-10-30) and WL-10 (2026-10-31). In progress: WL-3 (started 2026-10-31).  
Date: 2026-10-29; revised 2026-10-31  
Branch: `v2.0.0-planning`  
Baseline: `stc` 1.0.0, released 2026-10-29  
Profile: `stc-2013-oop-utf8-v1`

## 1. Purpose

`stc` 1.0.0 is released. This file starts the v2.0.0 wishlist: candidate directions for the next
major release, recorded while they are still ideas. It is deliberately not a roadmap, a
commitment, or the v2.0.0 plan.

When v2 planning starts, the v2.0.0 target is picked from this list and locked in a dedicated
plan with scope, non-goals, work breakdown, exit criteria, evidence, risks, and effort. Items
that are not picked stay here for a later release. The list is expected to grow: append new
entries with the next `WL-` id and change an entry's status rather than deleting it.

## 2. Ground rules

These apply to every candidate until a v2.0.0 target is locked.

- **v1.0.0 is the compatibility baseline.** v2 work is additive around the released compiler,
  runtime, artifact formats, and language profile unless a selected item explicitly says
  otherwise.
- **No core language or compiler feature work.** This wishlist contains no new ST language
  features, no codegen redesign, and no runtime architecture rewrite. Compiler changes are
  limited to bug fixes and to instrumentation required by a selected item (for example a stable
  IDE API or a conformance HIR dump); any such change is specified and reviewed as part of that
  item.
- **Evidence over assertion.** Verification claims use the V0–V4 ladder in the
  [Verification Program Plan](specification/Verification/Plan.md). Performance claims record
  host, kernel, build form, metric, and sample. `stc` remains not a safety PLC and makes no
  safety claim; see [NON-SAFETY.md](NON-SAFETY.md).
- **The PREEMPT_RT host is a run target, not a build host.** Artifacts are built on development
  or CI hosts and copied to it. No builds, package installs, or repository checkouts that
  compile happen there.
- **Wishlist entries are not commitments.** Scope sketch, size, and open questions are for
  comparison, not promises.

## 3. Selection and locking

All entries start as `Candidate`. The v2.0.0 targets are selected from this table; once the
target set is locked, detailed planning begins. The v2.0.0 target set was locked on 2026-10-29
and re-planned on 2026-10-31: WL-9 was removed and WL-5.2 was added. The locked targets and
their plans are in [V2.0.0-PLAN.md](V2.0.0-PLAN.md).

| Status | Meaning |
| --- | --- |
| `Candidate` | Recorded and comparable; no v2 commitment |
| `Selected` | Chosen for v2.0.0; not yet locked |
| `Locked` | Committed as a v2.0.0 target; the target set is fixed and detailed planning can begin |
| `Deferred` | Considered but not for v2.0.0; remains on the list |
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
5. **Effort.** Is the v2 slice bounded, or is the item an open-ended program that needs a
   selected phase rather than a whole?

Locking process:

1. Pick one item or a coherent group and mark it `Selected`.
2. When target selection is complete, lock the target set and mark each chosen item `Locked`.
3. After the target set is locked, write and review the detailed plans: goal, scope and
   non-goals, work breakdown, exit criteria, evidence, risks, effort, and dependencies.
4. After the target set is locked, adding or removing a target is an explicit re-plan, not an
   edit to the locked set.

Tentative size classes follow the [Verification Program Plan](specification/Verification/Plan.md):
S is up to one engineer-month, M is one to three, L is three to six, and XL is six to twelve or
more. They are order-of-magnitude estimates for comparison only.

## 4. Summary

| ID | Candidate | Area | Tentative size | Key dependency | Status |
| --- | --- | --- | --- | --- | --- |
| WL-1.1 | Local VS Code integration and debugging | Tooling | L (staged) | Stable IDE/debug API | Locked |
| WL-1.2 | Remote IDE debugging | Tooling/network | M/L | WL-1.1; network security | Deferred |
| WL-2 | Test coverage to 85% as reported by the existing CI coverage badge; CI pass gate | Quality | M/L | Existing coverage job/badge | Locked |
| WL-3 | Expand the Lean specification per the verification plan; P0.4 complete, continuing with P0.3 | Verification | M for P0.4 | Phase 0 P0.4 | In progress; P0.4 done 2026-10-31 |
| WL-4 | Run and verify RT performance on PREEMPT_RT Linux x86-64 | Runtime evidence | M | x86-64 RT run host and transferable artifacts | Done 2026-10-30 |
| WL-5.1 | Modbus TCP server I/O on all supported hosts; non-real-time | Runtime/IO | M/L | None | Locked |
| WL-5.2 | EtherCAT I/O on Linux using IgH | Runtime/IO | L | IgH; Linux host; EtherCAT hardware | Locked |
| WL-6 | Motion controller: coordinated multi-axis interpolated motion | Motion | L/XL | WL-5.2, WL-7, WL-4; robot-sim reuse | Deferred |
| WL-7 | ESI-based EtherCAT device configuration | Runtime/IO | M/L | WL-5.2 | Deferred |
| WL-8 | Tracing UI to watch and graph PLC variables | Tooling/observability | M/L | WL-9 WebUI | Deferred |
| WL-9 | WebUI for basic runtime control | Tooling/operations | M/L | Runtime control API; network security | Deferred |
| WL-10 | Coverage emission for the ST test suite | Test tooling | M/L | Test runner; span mapping | Done 2026-10-31 |
| WL-11 | Modbus TCP client capability; separate from WL-5.1 server | Runtime/IO | M/L | WL-5.1 backend contract | Deferred |

## 5. Candidate items

### WL-1.1 — Local VS Code integration and debugging

**Intent.** Make VS Code the practical local development environment for `stc`: edit, navigate,
build, test, and debug ST without falling back to the CLI for routine work. Everything in this
entry runs locally against a local compiler and runtime.

**Selection.** Selected and locked for v2.0.0 on 2026-10-29. Detailed planning is in
[V2.0.0-PLAN.md](V2.0.0-PLAN.md).

**Why it matters.** The product is a compiler plus runtime, but developers spend their time in an
editor. Rust development already has rust-analyzer, a test explorer, coverage gutters, and
coverage tasks configured. ST has no equivalent: no syntax highlighting, no go-to-definition, no
Problems-panel integration, and no debug adapter. The M6.5 debugger, the lossless CST, and the
stable append-only `SyntaxKind` are the raw material.

**Selected scope.**

- Local workspace: tasks and problem matchers for `stc check`, `build`, `run`, `test`, and
  `fmt`; diagnostics in the Problems panel; launch configurations for `stc run` and `stc test`.
- Language support: a TextMate grammar for syntax highlighting; a language server (`stc-lsp`)
  providing diagnostics, document symbols, go-to-definition, references, hover, completion, and
  formatting through `stc fmt`. The server needs an IDE-facing API over the CST/HIR/sema crates;
  the public CST/IDE API is currently planned for v2/v3 (D-041), so this item either lands that
  API or wraps the crates behind a stable boundary.
- Debug: a Debug Adapter Protocol adapter (`stc-dap`) translating DAP to the local `stc-debug/1`
  protocol: launch/attach, breakpoints, stepping, variables/watch, and call stack. Reuse the
  typed access and checkpoint contracts; do not invent a second debugger protocol.
- Packaging: an extension manifest with recommended tasks and settings, distributed as a VSIX or
  through the marketplace; a sample ST workspace for smoke testing.
- Evidence: extension smoke tests for diagnostics, navigation, and a local debug session, plus
  DAP mapping tests for supported and honestly unsupported capabilities.

**Constraints and current state.**

- Debug is local-only, one session per host; release builds are monitoring-only; every swap,
  rollback, cold install, or shutdown ends the session and clears forces and breakpoints.
- `SyntaxKind` is stable and append-only; the full public CST/IDE API is not yet committed.
- Extension packaging and distribution (marketplace, signed VSIX, or repo-local extension) and
  the minimum VS Code version are undecided.
- The extension must not become a second source of truth for diagnostics or type information;
  `stc` remains authoritative.
- This entry explicitly excludes remote or network access; remote debugging is WL-1.2.

**Open questions.**

- Is the language server a new crate in this workspace, or part of `stc-driver`?
- Which IDE API is stable enough to expose, and does it require compiler changes beyond adding
  public views?
- Which DAP features map cleanly to `stc-debug/1`, and which are honest unsupported capabilities?
- What is the first supported editor surface: VS Code only, or an LSP/DAP pair usable by other
  editors?
- Should the local language server use stdio or a local socket, and how does that interact with
  the local debug transport?
- What is the extension distribution and versioning policy?

**Related.** [Debug and Monitoring](specification/Architecture/Debug-and-Monitoring.md),
[CLI and Manifest](specification/Architecture/CLI-and-Manifest.md),
[Known limitations §3](KNOWN-LIMITATIONS.md), D-041, [.vscode/](.vscode/), WL-1.2.

**Tentative size.** L, staged: S/M for highlighting, tasks, and diagnostics; L for the language
server and debug adapter.

### WL-1.2 — Remote IDE debugging

**Intent.** Extend the local IDE debugging path (WL-1.1) to a runtime on another host: connect
VS Code to a remote `stc` runtime over the network with authentication, transport security, and
source mapping. This is a separate task and depends on local IDE debugging first.

**Status.** Deferred to a future version (2026-10-29).

**Why it matters.** The PREEMPT_RT host (WL-4) and deployed controllers are run-only targets, not
development machines. A developer needs to set breakpoints, step, inspect variables, and read
call stacks on the target from the development host. The v1 debug protocol is local-only (`pipe`
by default, loopback socket optional), so remote debugging is a product and security change, not
just an extension setting. It is split from WL-1.1 because network transport, authentication,
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
- Use the PREEMPT_RT host (WL-4) as the first target; direct-host and Remote-SSH or remote
  workspace paths are deployment options.
- Evidence: remote session integration tests, authentication/TLS negative tests, reconnection
  tests, a threat model, and a runbook.

**Constraints and current state.**

- Depends on WL-1.1: the local DAP adapter and local debug protocol must exist first.
- v1 debug is local-only and one session per host; release builds are monitoring-only; every
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
- What happens to a remote session on network loss, swap, rollback, or runtime restart?
- Is one remote session per host still the limit, or do deployments need multiple concurrent
  sessions?
- Should remote debugging work from other DAP clients, or only VS Code?
- Which host is first: the PREEMPT_RT host (WL-4) or a deployed controller?

**Related.** [Debug and Monitoring](specification/Architecture/Debug-and-Monitoring.md),
[Known limitations §3 and §4](KNOWN-LIMITATIONS.md), WL-1.1, WL-4, WL-8, WL-9.

**Tentative size.** M/L, dominated by transport/security and remote source handling, not by the
DAP adapter itself.

### WL-2 — Raise test coverage to at least 85%, highest-value code first

**Intent.** Raise the line coverage reported by the existing CI coverage badge to at least 85%.
This is the single headline coverage target. The measurement mechanism, badge, and reporting job
do not change; the coverage job simply fails when the reported figure is below 85%.

**Selection.** Selected and locked for v2.0.0 on 2026-10-29. Detailed planning is in
[V2.0.0-PLAN.md](V2.0.0-PLAN.md); the scope and gate rules below are fixed.

**Why it matters.** The CI badge currently reports 79.3% line coverage (fetched 2026-10-29).
Coverage is reporting-only: the job writes a summary, HTML report, and badge, but there is no
floor. The gaps are concentrated in the same areas the v1 correctness claims depend on: semantic
analysis, code generation, GOT identity and codecs, runtime swap and retained state, and the
process image. An 85% target is a floor, not a goal; tests must exercise behavior and error
paths, not merely execute lines.

**Selected scope.**

- Record the baseline by file from the existing CI coverage report and rank primarily by
  **uncovered line count**. Apply a small criticality boost to core pipeline files (lexer,
  parser, HIR, sema, codegen) and security-adjacent files (debug access and forcing, manifest and
  descriptor decoding, runtime loader), but do not let tooling such as `xtask` or test
  infrastructure outrank product code just because it has many uncovered lines. The simple
  objective is to reduce total uncovered lines.
- Add focused unit, property, differential, and negative tests. Reuse existing fuzz corpora, the
  conformance/execution suites, and cross-host jobs where they already cover the behavior.
- Make the existing CI coverage job a pass/fail gate at 85% line coverage. Use the same LCOV
  figure that produces the badge, with no separate metric and no change to the measurement
  mechanism.
- Keep the reporting output (summary, HTML, badge) unchanged; the gate only adds the failure
  condition.
- The gate prevents regressions below 85% once the target is reached.

**Initial gap estimate (2026-10-29).** The latest CI coverage artifact (run `40917236322`)
reports 84,503 workspace lines, 66,996 covered (79.3%), and 17,507 uncovered. Reaching 85%
requires about 4,832 more covered lines if the total stays stable. The largest per-file gaps are:

| File | Uncovered lines | Line coverage | Class |
| --- | ---: | ---: | --- |
| `crates/stc-runtime/src/debug.rs` | 1,975 | 69.0% | Runtime debug, security-adjacent |
| `crates/stc-sema/src/check.rs` | 1,415 | 72.3% | Core pipeline |
| `crates/stc-codegen/src/place.rs` | 1,214 | 70.8% | Core pipeline |
| `crates/xtask/src/main.rs` | 974 | 12.3% | Tooling, lower criticality |
| `crates/stc-driver/src/debug.rs` | 662 | 56.1% | Debug tooling |
| `crates/stc-runtime/src/lib.rs` | 564 | 41.1% | Runtime core |
| `crates/stc-sema/src/types.rs` | 510 | 75.9% | Core pipeline |
| `crates/stc-runtime/src/debug_server.rs` | 481 | 84.4% | Runtime debug, security-adjacent |
| `crates/stc-sema/src/consteval.rs` | 477 | 59.8% | Core pipeline |
| `crates/stc-got/src/manifest.rs` | 463 | 68.9% | Artifact codec, security-adjacent |
| `crates/stc-codegen/src/module.rs` | 444 | 79.9% | Core pipeline |
| `crates/stc-codegen/src/expr.rs` | 409 | 74.3% | Core pipeline |

The core pipeline crates (lexer, parser, HIR, sema, codegen) account for about 6,958 uncovered
lines, or 40% of the total. The runtime/debug block is the single largest cluster. This is a
snapshot for triage, not a fixed list; re-derive it from the next CI run as tests land.

**Constraints and current state.**

- The existing coverage job runs the workspace tests under instrumentation with clang required
  and a 45-minute timeout; that measurement is authoritative.
- Line coverage is not rule coverage; semantic traceability is a separate check in
  [Testing and Verification](specification/Architecture/Testing-and-Verification.md).
- There is one headline target: 85% workspace line coverage on the existing Ubuntu x64 CI run.
  Do not add per-crate, per-host, region, or other secondary targets.
- The gate is a CI pass requirement: a run below 85% fails, and v2.0.0 is not released with a
  failing coverage job.
- Tests must not be inflated to hit the number; the highest-value gaps are closed first.

**Resolved decisions (2026-10-29).**

- **Effort.** No effort estimate now. Detailed planning, including the test breakdown and effort,
  happens after all v2.0.0 targets are selected and locked.
- **Gate enforcement.** The coverage job fails on every CI trigger that runs it (pull request,
  `main`, and manual dispatch) when the badge figure is below 85%. Make the `coverage` job a
  required branch-protection check for pull requests and `main`; no separate gate job or metric.
- **Runtime/debug test approach.** Start with unit tests for isolatable state-machine and protocol
  logic, and add integration-test harness work only where a debug session or transport boundary
  is required. Apply the same split to `crates/stc-driver/src/debug.rs` and
  `crates/stc-runtime/src/debug_server.rs`.

**Related.** [Testing and Verification](specification/Architecture/Testing-and-Verification.md),
[Verification Program Plan §4](specification/Verification/Plan.md),
[README coverage section](README.md), [tools/ci/coverage_summary.py](tools/ci/coverage_summary.py),
[tools/ci/coverage_badge.py](tools/ci/coverage_badge.py), [.github/workflows/ci.yml](.github/workflows/ci.yml).

**Tentative size.** M/L, dominated by the high-value gap analysis and the tests that change
behavior, not by wiring the gate.

### WL-3 — Expand the Lean specification according to the verification plan

**Intent.** Resume and extend the Lean formalization and verification program now that v1.0.0 is
released and the compiler is no longer moving underneath the proofs. This is the only wishlist
area expected to touch the compiler, and only for bug fixes found by the work. There is no
wishlist for new language or core compiler features.

**Selection.** Selected and locked for v2.0.0 on 2026-10-29 with the **P0.4** target (target
syntax and `WellFormed`), and started on branch `WL-3` on 2026-10-31. P0.4 is complete and
review-approved (2026-10-31); WL-3 continues with P0.3, whose open questions are reevaluated after
the P0.4 CI run. Core compiler features and the language are not expected to change, so further
drift is not expected; only bug fixes are in scope. Detailed planning is in
[V2.0.0-PLAN.md](V2.0.0-PLAN.md); the P0.4 scope decisions are in
[Plan-Phase0.md](specification/Verification/Plan-Phase0.md) §20.

**Why it matters.** Phase 0 is paused mid-P0.3 pending alignment with M6.5. The earlier blocker —
ongoing compiler changes — is resolved by the v1.0.0 release. The expensive parts are partly in
place: the token model, lossless CST contract, surface AST, grammar relation, typed HIR, core
checker soundness, the value and trap model, determinism, trap soundness, and the fuel
interpreter. The remaining Phase 0 theorems and the later phases are staged so each phase yields
a defensible claim. Continuing against a released compiler avoids the rework that blocked the
work before. P0.4 depends only on the P0.0/P0.1 vocabulary, so the paused P0.3 work does not
block it.

**Selected scope.** P0.4 only. Define the emitted-IR fragment and the structural invariants the
target semantics will assume:

- Define `IRFrag`: modules, type definitions, globals, constants, functions, basic blocks,
  instructions, terminators, and the slot/function tables.
- Define `WellFormed`: type correctness, SSA/dominance, block terminators, `phi` arity, GOT cell
  kinds, canonical slot table, entry-point signatures, and no direct persistent-root or
  user-callable references in the covered fragment.
- Define the slot table and cell-kind model used by the later target semantics, including the
  canonical ordering rule.
- Prove the structural lemmas listed in
  [Plan-Phase0 §P0.4](specification/Verification/Plan-Phase0.md): well-formed blocks have a
  terminator; `phi` predecessors match; slot references are in range and have the recorded kind;
  entry-point signatures match the ABI 2 contract.

P0.5 target operational semantics, P0.6 conformance, P0.7 review/versioning, and all later
phases are out of scope for v2.0.0. The [program plan](specification/Verification/Plan.md)
remains authoritative for the overall sequence.

**Constraints.**

- Lean 4.35.0 is pinned; no `sorry` or `admit`; the registry, no-sorry check, and vector
  regeneration are already in CI.
- The Rust compiler is the conformance oracle, not the reference. V0, V1, and V2 claims must stay
  distinct.
- Bug fixes found during proofs are in scope. Behavior changes beyond fixes are not.
- Phase 0 is standalone; Phase 1 toolchain work must not block it.
- The verification registry and specification traceability rules still apply to every theorem.

**Open questions.**

- Does P0.4 need any compiler-side IR dump or artifact change, or is the textual IR already
  sufficient?
- Which existing `stc-codegen::verify` contracts should `WellFormed` mirror, and where is the
  Rust verifier stricter or looser than the Lean predicate?
- Are there specification ambiguities in the slot table or cell-kind model that must be resolved
  before the structural proofs?
- Who owns the proof CI budget for P0.4?

**Related.** [Verification Program Plan](specification/Verification/Plan.md),
[Phase 0 Implementation Plan](specification/Verification/Plan-Phase0.md),
[Semantics](specification/Verification/Semantics.md),
[Semantics Charter](specification/Verification/Semantics-Charter.md),
[lean/verification-status.toml](lean/verification-status.toml).

**Tentative size.** M for P0.4, per [Plan-Phase0](specification/Verification/Plan-Phase0.md);
the full program remains XL.

### WL-4 — Run and verify RT performance on PREEMPT_RT Linux

**Status.** Done 2026-10-30. Report: [RT-Host.md](specification/Verification/RT-Host.md);
runbook: [RT-Runbook.md](specification/Verification/RT-Runbook.md); evidence:
[RT-Run-Manifest.md](specification/Verification/RT-Run-Manifest.md).

**Intent.** Measure `stc` runtime behavior on a PREEMPT_RT Linux host and produce evidence about
cycle timing, jitter, and scheduling latency. Use the host only to run artifacts, never as a
build environment.

**Selection.** Selected and locked for v2.0.0 on 2026-10-29, targeting the available x86-64
PREEMPT_RT Linux host. The host remains a run target only. Detailed planning is in
[V2.0.0-PLAN.md](V2.0.0-PLAN.md).

**Why it matters.** The v1 performance figures are Windows-host wall-clock measurements and
explicitly not real-time guarantees. A controller's practical value depends on deterministic
cycle behavior, and an RT host is available now. Using it as a run target buys measurement
evidence and a deployment runbook cheaply. This item produces evidence and guidance, not a hard
real-time guarantee and not a safety claim.

**Selected scope.**

- Define the artifact pipeline: build release artifacts on the development or CI host for the RT
  host's triple, copy the executable or module, runtime libraries, metadata, manifests, and a
  benchmark project to the host, and verify checksums/build IDs before running.
- Record the host baseline: CPU, kernel version and PREEMPT_RT configuration, isolation and CPU
  affinity, governor, and background load. Include a `cyclictest` baseline.
- Measure cycle wake-up jitter, cycle execution time distribution (best, median, p99, max),
  missed deadlines, page faults and allocations in steady state, and the cost of the control and
  status threads. Cover `stc run`, online change prepare/activation, and rollback. State the
  sample count and method.
- Stress at the deployment cycle time and shorter; vary root count, task count, and I/O backend;
  run a swap and rollback under load.
- Deliver a report with method, environment, raw data, and limits, plus a repeatable harness
  (machine-readable output) and a runbook for the host.
- Decide whether the harness is an ignored benchmark like the existing performance tests, a
  manual runbook, or a dispatched job. The RT host is not a CI runner.

**Constraints and current state.**

- The host is not a development machine. Do not build, install toolchains, or compile there.
- Artifacts must match the host architecture and libc; cross-host transfer is part of the
  procedure.
- v1 has no CPU affinity pinning; control and debug threads run below normal priority as advisory
  policy. RT setup (cpuset, priorities, isolation) belongs to deployment policy unless
  measurement shows a product defect.
- Debug stops may hold the cycle indefinitely; use release artifacts and no debug session unless
  the measurement is specifically about debug overhead.
- Results remain host- and configuration-specific; no hard real-time guarantee is claimed.

**Open questions (resolved 2026-10-30; see the report for the answers).**

- What does "verified" mean here: a reproducible report, a threshold on max jitter or missed
  deadlines, or comparison against the `cyclictest` baseline?
- Which cycle times, task counts, and program shapes are representative for the first report?
- Which kernel, distribution, and RT patch level is the x86-64 host running?
- How are artifacts transferred and verified, and is the RT host reachable from CI or only
  manually?
- Should the harness include WL-5.1 Modbus server runs, or stay pure compute for the first
  report? EtherCAT (WL-5.2) is a separate v2.0.0 target and was not part of this campaign.
- If measurement exposes a runtime or compiler defect, fixing it is in scope as a bug fix.

**Related.** [Performance](specification/Verification/Performance.md),
[Runtime and Execution](specification/Architecture/Runtime-and-Execution.md),
[Known limitations §2](KNOWN-LIMITATIONS.md),
[crates/stc-driver/tests/performance.rs](crates/stc-driver/tests/performance.rs),
[crates/stc-driver/tests/debug_overhead.rs](crates/stc-driver/tests/debug_overhead.rs).

**Tentative size.** M, dominated by the harness, measurement campaign, and report; host
provisioning is outside the estimate.

### WL-5.1 — Modbus TCP server I/O on all supported hosts

**Intent.** Add a Modbus TCP server I/O backend to the runtime on every supported host. The
runtime owns device scheduling and binding; the compiler continues to emit opaque metadata and
`{stc:io}` hints.

**Selection.** Selected and locked for v2.0.0 on 2026-10-29. The first capability is the
**server** role, exposing the `stc` process image to Modbus clients. It is not a real-time
channel. Client/master capability is a separate item (WL-11, deferred) and is not part of this
selection. Detailed planning is in [V2.0.0-PLAN.md](V2.0.0-PLAN.md).

**Why it matters.** Modbus TCP is the lowest-friction hardware I/O path: it runs over ordinary TCP
on Windows and Linux, needs no kernel modules or special RT setup, and has a large installed base
with broad controller, HMI, remote I/O, and test-tool support. A server backend lets those
clients read and write the `stc` process image without custom integration. It is a good first
real I/O backend and exercises the v1 device tree, binding precedence, metadata sidecar, and
per-generation direct-variable cells without the EtherCAT dependency. It is best-effort, not real-time, and is not an interpolated motion channel; polling
latency and the lack of distributed clocks make it unsuitable for coordinated servo motion.
Motion (WL-6) is deferred and would depend on EtherCAT (WL-5.2, locked for v2.0.0) and ESI
(WL-7, deferred).

**Selected scope.**

- Backend framework shared with the other backends: runtime configuration and override format (a
  v1 open question), device and channel binding resolution, health/status reporting, failure
  policy, deterministic latch/publish at the cycle boundary, and simulator-backed tests.
- Modbus TCP server exposing process-image ranges as coils, discrete inputs, holding registers,
  and input registers; handling client requests and scheduling responses within the cycle;
  connection limits, timeouts, byte/word order, and diagnostics. Must run on Windows x86-64 MSVC,
  Linux x86-64 GNU, and Linux AArch64 GNU.
- Configuration: extend `[io]`, `[[io.devices]]`, and `[[io.bindings]]` handling only where the
  existing opaque design is insufficient. Keep protocol-specific fields out of the language and
  compiler.
- Evidence: Modbus simulator tests, error-path tests, cross-host tests, I/O-cycle overhead
  measurements, and updates to the I/O specification and
  [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md).

**Constraints and current state.**

- The compiler parses `AT`, assigns process-image offsets, emits metadata, and validates
  pragma/binding shape; it never opens devices. The runtime owns backend scheduling and may
  override hints.
- Binding precedence and the `null`/`memory`/`file` semantics are specified. The runtime override
  format is still an open question in
  [I/O and Device Linking](specification/Architecture/IO-and-Devices.md).
- Modbus TCP has no standard authentication or encryption. Assume a trusted or segmented network
  and document the risk; transport security is a separate decision, not part of this backend.
- Server request latency and connection handling are part of the cycle behavior. The backend is
  best-effort, not real-time; a slow or failed client must not silently produce wrong outputs,
  and the non-safety stance still applies.
- Modbus TCP is not suitable for interpolated motion; do not treat it as a fallback motion
  channel.
- Client/master capability is not part of this backend; it is WL-11.
- Device and channel bindings must be validated when the device tree is optional and runtime
  overrides can rebind.

**Open questions.**

- Which server function codes, data widths, and byte/word order are required first?
- How many concurrent client connections are supported, and what are the timeout, idle, and
  failure policies?
- How are incoming requests queued and served relative to the input latch and output publish?
- Do server-exposed ranges use the existing `[[io.bindings]]` model, or a separate server mapping?
- Which simulator and cross-host CI strategy prove the backend?
- Should MQTT remain a v2 candidate? It is neither Modbus nor EtherCAT and is not covered here.
- How are bindings validated across optional device trees and runtime rebinding?

**Related.** [I/O and Device Linking](specification/Architecture/IO-and-Devices.md),
[Runtime and Execution](specification/Architecture/Runtime-and-Execution.md),
[CLI and Manifest](specification/Architecture/CLI-and-Manifest.md),
[Known limitations §5](KNOWN-LIMITATIONS.md), D-028. WL-11 and WL-6 are deferred; WL-5.2 is
locked for v2.0.0.

**Tentative size.** M/L: backend framework M, Modbus TCP server M/L, plus cross-host test and
evidence work.

### WL-5.2 — EtherCAT I/O on Linux using IgH

**Intent.** Add an EtherCAT I/O backend on Linux hosts using the IgH EtherCAT master library,
including cyclic process data and servo-drive integration. Implementing an EtherCAT master, a
slave stack, or a raw EtherCAT protocol engine is explicitly out of scope.

**Selection.** Selected and locked for v2.0.0 on 2026-10-31; WL-4 is complete and the EtherCAT
hardware has arrived. Detailed planning is in [V2.0.0-PLAN.md](V2.0.0-PLAN.md).

**Why it matters.** EtherCAT provides cyclic, deterministic process data and is the practical
fieldbus for interpolated multi-axis motion. IgH is a kernel-module plus userspace-library stack,
so this backend is Linux-only in this project. It is the hardware path that makes WL-6 motion
practical; without it, motion is limited to virtual axes or simulation. ESI descriptions (WL-7)
supply the device identity and PDO mapping that real slaves require.

**Candidate scope.**

- Integrate IgH and map the `stc.toml` device tree and channel descriptors to IgH configuration
  and process data; run cyclic process data at the cycle boundary; handle startup, faults, and
  recovery; use distributed clocks where IgH provides them.
- Use static configuration for the v2.0.0 path; ESI (WL-7, deferred) can supply identity and PDO
  mapping when it lands.
- Runtime configuration and override, binding resolution, health/status reporting, deterministic
  latch/publish, error paths, and simulator or loopback tests where possible.
- Evidence: an IgH setup and runbook, hardware-in-the-loop tests on the PREEMPT_RT host (WL-4),
  I/O-cycle timing and jitter measurements, and specification updates.
- Review IgH licensing and packaging before implementation.

**Constraints and current state.**

- EtherCAT requires Linux, kernel support, and privileged or raw network access. IgH kernel
  modules constrain the kernel/RT configuration; the WL-4 host is the natural first target.
- IgH licensing and distribution (GPL implications for the runtime and packages) must be reviewed
  before implementation. That review is a planning input, not a reason to write a master.
- Static configuration is the v2.0.0 path; ESI-based device configuration (WL-7) is deferred.
- When WL-6 is revived, it depends on this backend and WL-7 in practice. Modbus TCP (WL-5.1) is
  not a substitute.
- Fieldbus failure behavior must not silently produce wrong outputs; the non-safety stance still
  applies.

**Open questions.**

- IgH version, kernel modules, distributed-clock support, and the link model (shared library,
  static library, or daemon interface) are planning inputs.
- What is the minimum static configuration subset needed for the first hardware-in-the-loop run?
- Which slaves and servo drives does the delivered hardware cover, and which are supported first?
- What PDO and distributed-clock subset does motion (WL-6) require?
- How is IgH configured: generated configuration, a library API, or both?
- How do online change and rollback handle topology and binding changes that cannot be
  preserved?
- What are the startup and shutdown ordering rules between IgH and the runtime host?

**Related.** [I/O and Device Linking](specification/Architecture/IO-and-Devices.md),
[Runtime and Execution](specification/Architecture/Runtime-and-Execution.md),
[CLI and Manifest](specification/Architecture/CLI-and-Manifest.md),
[Known limitations §5](KNOWN-LIMITATIONS.md), D-028, WL-4, WL-6, WL-7.

**Tentative size.** L, including IgH integration, hardware bring-up, and evidence.

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
  budget. Hard real-time behavior is unproven and is measured by WL-4, not assumed.
- v1 has no motion function blocks and no drive integration. Motion does not architecturally
  require EtherCAT or ESI—virtual axes are enough to exercise the controller—but practically it
  depends on WL-5.2 and WL-7 to drive real servo axes. Modbus TCP (WL-5.1) is not a viable
  interpolated motion channel. Without EtherCAT and ESI, motion is limited to virtual axes or
  simulation.
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
[Verification Program Plan](specification/Verification/Plan.md) (D-V2), WL-3, WL-4, WL-5.2,
WL-7.

**Tentative size.** L/XL. Adapting proven code reduces the math risk; generalization, drive
integration, the ST API, and cycle/online-change behavior dominate.

### WL-7 — ESI-based EtherCAT device configuration

**Intent.** Let the runtime configure EtherCAT devices from vendor `.esi` (EtherCAT Slave
Information) files instead of hand-maintained channel and PDO metadata. This depends on
integrating EtherCAT at all (WL-5.2).

**Status.** Deferred to a future version (2026-10-29).

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
2026-10-31 re-plan. Nothing in the locked set depends on it.

**Why it matters.** v1 runtime control is a local CLI and API. There is no remote or network
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

- v1 is local-only for `stc run --watch`, the control channel, and debug. Exposing network
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

### WL-10 — Coverage emission for the ST test suite

**Intent.** Let `stc test` emit coverage for the ST code under test: which statements, and later
which branches, the test suite exercised, attributed to source spans. This is coverage of the
user's ST program, not the compiler's Rust test coverage (WL-2).

**Selection.** Selected and locked for v2.0.0 on 2026-10-29. Detailed planning is in
[V2.0.0-PLAN.md](V2.0.0-PLAN.md).

**Why it matters.** `stc test` already discovers `{stc:test}` POUs, builds one cold artifact per
test, and runs them in isolated processes. It reports pass/fail but says nothing about which
parts of the ST program the tests never touched. PLC projects need coverage evidence for
commissioning, change review, and regulated development. The compiler already has HIR spans, a
span table, and codegen instrumentation points, so coverage is a natural extension of the test
build rather than a new runtime feature.

**Selected scope.**

- Instrument test artifacts, not production builds, with deterministic counters keyed by HIR
  statement or source span; statement coverage first, branch coverage next.
- Map counters back to ST source file, line, and column through the span table; handle multiple
  source files and synthetic test wrappers.
- Collect per-test counters in the test host or runner and aggregate across the suite, keeping
  per-test detail where it is useful.
- CLI: `stc test --coverage` with native JSON first, then LCOV and a human/HTML summary. Decide
  threshold and fail-under behavior separately.
- Exclude test POUs, synthetic wrappers, and compiler-generated helpers by default. Decide
  whether dependency packages and synthesized standard-library or function-block bodies are in
  scope or reported separately.
- Ensure instrumented and uninstrumented tests behave identically (same results, traps, and
  timing semantics) and that coverage instrumentation is absent from production artifacts.
- Evidence: golden reports, aggregation tests, determinism checks, cross-host tests, an example
  project with known covered and uncovered lines, and documentation of the coverage model.

**Constraints and current state.**

- Tests run one artifact per test in a separate process; counters must be per-test and merged by
  the runner. `stc test` is sequential in v1.
- Test builds are cold, hook-free debug form and reject online change and retained state.
  Coverage instrumentation should follow the same policy.
- The span table and debug checkpoint machinery exist, but debug checkpoints are a separate
  mechanism with different semantics. Coverage counters must not require a debug session.
- Only test builds are instrumented; `stc build` and release artifacts remain unchanged.
- Coverage is not a correctness claim. A high percentage does not prove the tests assert the
  right behavior, and ST line coverage is not the same as the rule-level traceability check in
  [Testing and Verification](specification/Architecture/Testing-and-Verification.md).

**Open questions.**

- First granularity: statement or line coverage, branch coverage, condition coverage, or MC/DC?
  Which is useful for the target industries?
- Native JSON only first, or LCOV/HTML at the same time? Can existing coverage tooling and editor
  gutters consume the output?
- Per-test coverage, suite aggregate, or both? How do test filters affect the report?
- Are dependency packages instrumented, or only the package under test?
- Are synthesized standard-library and function-block bodies included, excluded, or reported
  separately?
- How are declarations, empty statements, and compiler-inserted code represented?
- Should `stc test` support a fail-under threshold, or is gating a CI concern?
- Does coverage later surface in WL-1.1/VS Code, or stay CLI/CI only?
- What is the overhead budget for instrumentation in test builds?

**Related.** [Testing and Verification](specification/Architecture/Testing-and-Verification.md)
§2, [CLI and Manifest](specification/Architecture/CLI-and-Manifest.md) §1.6,
[crates/stc-test](crates/stc-test), WL-1.1, WL-2.

**Tentative size.** M/L, dominated by instrumentation and span mapping, not the report format.

### WL-11 — Modbus TCP client capability

**Intent.** Add Modbus TCP client/master capability, separate from the WL-5.1 server backend:
connect to external Modbus TCP servers and map their coils and registers into the `stc` process
image or access them from ST. It is not a real-time channel.

**Status.** Deferred to a future version (2026-10-29); separate from WL-5.1 and not part of
v2.0.0.

**Why it matters.** Many deployments need both roles: the PLC exposes its own process image
(server) and also reads sensors, meters, remote I/O, or drives that expose a Modbus TCP server.
The server role was selected first because it adds no outbound connection scheduling to the v2
cycle. The client role is separate because it introduces connection management, polling,
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

## 6. Cross-cutting notes

- WL-4, WL-5.1, and WL-5.2 are locked v2.0.0 targets: RT timing, Modbus TCP server I/O, and
  EtherCAT I/O using IgH. WL-6 and WL-7 are deferred to a future version and would complete the
  motion/ESI cluster.
- WL-5.1 and WL-5.2 share the runtime backend contract but have different platform, scheduling,
  and failure models. Keep the interface shared and the implementations separate. Modbus TCP is
  general I/O, not an interpolated motion channel; when WL-6 is revived, it depends on WL-5.2 and
  WL-7 in practice.
- WL-11 is deferred. It is the Modbus TCP client counterpart to WL-5.1; the server role is
  selected first, while the client has different outbound scheduling and failure behavior.
- WL-4 and WL-5.2 reinforce each other: EtherCAT cyclic process data is best verified on the
  PREEMPT_RT host, and WL-5.2 should not assume the host exists without a fallback test path.
- WL-6 is deferred to a future version. If revived, it reuses robot-sim's `robot-motion` and its
  Lean `Motion` library. Those proofs should be registered with stc's verification program
  rather than duplicated, which ties the item to WL-3 and to the Mathlib policy (D-V2) and
  introduces an external-repository maintenance obligation.
- WL-8 is deferred to a future version, as is the WebUI it depends on (WL-9). If revived, it is a
  module of the WebUI, not a separate front end; it uses the WebUI's authentication, packaging,
  and deployment, and neither may bypass the runtime's control or debug contracts.
- WL-9 is deferred. WL-5.1 is the only locked v2 item that exposes a network service. WL-1.2 and
  WL-8 are deferred; if either is revived, a single authenticated remote-access design should
  cover remote debugging, tracing, and the WebUI rather than three independent security models.
- WL-10 is the ST-program counterpart to WL-2's Rust coverage. They may share report formats and
  CI habits, but not instrumentation: WL-2 measures the compiler with `cargo-llvm-cov`, while
  WL-10 instruments generated test artifacts. Keep the two numbers separate. WL-10 can emit LCOV
  for WL-1.1's coverage gutters later, but WL-1.1 is not a dependency.
- WL-1.1's debug adapter depends on the existing local debug protocol. WL-1.2 is deferred; if
  revived, it depends on WL-1.1 and extends the path with network transport and security. It
  overlaps the "debug is local only" limitation and should be planned as a security and product
  change, not only as an extension feature.
- WL-2 is locked for v2.0.0. It is continuous and can run alongside the other locked targets. It
  has one headline target (85% as reported by the existing CI coverage badge) and turns the
  existing coverage job into a pass/fail gate; it does not change the measurement mechanism or
  add secondary targets.
- WL-3 is locked with the bounded P0.4 target; the rest of the verification program remains out
  of scope for v2.0.0. No further compiler or language drift is expected, so the proofs target a
  stable compiler.
- All non-deferred items are largely additive around a stable compiler. WL-3 and WL-10 may drive
  compiler work for instrumentation, validation, or a small runtime/API surface, but none of
  them changes the ST language or the core compiler architecture. WL-6 and WL-7 remain deferred
  and would carry their own compiler/runtime surface if revived.

## 7. Backlog

All recorded items are elaborated above. Add new candidates below with the next `WL-` id, a
one-line intent, and the same structure when they are elaborated.

## 8. Change log

| Date | Change |
| --- | --- |
| 2026-10-29 | Initial wishlist with WL-1 through WL-5. WL-5 records IgH as the recommended EtherCAT foundation and excludes implementing an EtherCAT master. |
| 2026-10-29 | Added WL-6 (motion controller adapted from robot-sim and its Lean `Motion` library), WL-7 (ESI-based EtherCAT configuration, dependent on WL-5), WL-8 (tracing UI), and WL-9 (runtime-control WebUI). |
| 2026-10-29 | Split WL-5 into WL-5.1 (Modbus TCP on all hosts) and WL-5.2 (EtherCAT on Linux using IgH). WL-6 motion now depends on WL-5.2 and WL-7 in practice, not on Modbus TCP. WL-8 tracing UI now depends on WL-9 WebUI and is delivered as its module. |
| 2026-10-29 | Added WL-10 (coverage emission for the ST test suite), distinct from the Rust compiler coverage in WL-2. |
| 2026-10-29 | Split WL-1 into WL-1.1 (local VS Code integration and debugging) and WL-1.2 (remote IDE debugging, dependent on WL-1.1). |
| 2026-10-29 | Clarified WL-2: one headline target of 85% as reported by the existing CI coverage badge; the existing coverage job becomes a CI pass/fail gate with no change to the measurement mechanism. Added the initial per-file gap estimate from CI run 40917236322 and the uncovered-line-first prioritization rule. |
| 2026-10-29 | Selected WL-2 for v2.0.0 and resolved its open questions. Detailed planning is deferred until all v2.0.0 targets are selected and locked; the selection/locking wording now reflects target-lock before detailed planning. |
| 2026-10-29 | Deferred WL-1.2 (remote IDE debugging), WL-6 (motion controller), WL-7 (ESI configuration), and WL-8 (tracing UI) to a future version. |
| 2026-10-29 | Selected WL-1.1, WL-3 with the P0.4 target, WL-4 on the x86-64 PREEMPT_RT host, and WL-5.1 (Modbus TCP server, non-real-time). Deferred WL-5.2. Added WL-11 (Modbus TCP client) as a separate candidate. |
| 2026-10-29 | Selected WL-9 (WebUI) and WL-10 (ST coverage emission); deferred WL-11 (Modbus TCP client). All recorded items are now selected or deferred. |
| 2026-10-29 | Locked the v2.0.0 target set (WL-1.1, WL-2, WL-3 P0.4, WL-4 x86-64, WL-5.1, WL-9, WL-10) and started [V2.0.0-PLAN.md](V2.0.0-PLAN.md). |
| 2026-10-30 | Marked WL-4 done after PR #7 merged into main and main was merged back into v2.0.0-planning; the report, runbook, and evidence are in specification/Verification/. |
| 2026-10-31 | Re-planned the v2.0.0 target set: deferred WL-9 (WebUI) and locked WL-5.2 (EtherCAT using IgH) now that WL-4 is complete and the EtherCAT hardware has arrived. |
| 2026-10-31 | Marked WL-10 done and started WL-3 (P0.4) on branch `WL-3`; the compiler drift that paused P0.3 was rechecked against `stc` 1.0.0 and P0.3 remains paused. |
