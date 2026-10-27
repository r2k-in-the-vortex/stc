# M6 Implementation Plan — Project System, CLI, and Verification Hardening

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-26  
Status: Phases 0–9 complete (2026-10-27); phases 10–11 ready to start. M5 (phases 1–7 and 4b) is
complete and M6.5 is planned, so M6 is the bridge from the completed compiler/runtime surface to a
usable v1 project tool.  
Inputs: [Project Plan](Specification/00-Project-Plan.md) §8 M6,
[CLI and Manifest](Specification/Architecture/CLI-and-Manifest.md) §1–§6,
[State and RETAIN](Specification/Architecture/State-and-Retain.md),
[I/O and Devices](Specification/Architecture/IO-and-Devices.md),
[Testing and Verification](Specification/Architecture/Testing-and-Verification.md),
[Online Change and GOT](Specification/Architecture/Online-Change-and-GOT.md),
[Runtime and Execution](Specification/Architecture/Runtime-and-Execution.md),
[Architecture](Specification/Architecture/Architecture.md) §2/§5,
[GOT Artifacts and Schemas](Specification/Architecture/GOT-Artifacts.md),
[Conformance Profile](Specification/01-Conformance-Profile.md), `M5-Plan.md`, `M6.5-Plan.md`, and
Decisions D-051, D-060, D-068, D-071, D-079, D-084, D-085, D-087, D-108–D-110, D-134, D-136, and
D-153–D-164.

M6 follows M5 and precedes M6.5. It turns the finished language/runtime surface into a project
tool: `stc.toml`, local path dependencies, a committed lockfile, package-aware namespaces,
configurations and task scheduling, file-backed retained state, I/O device/binding metadata,
`stc run/test/fmt/clean/new`, the online-change developer workflow, Lean vectors, the conformance
corpus, traceability, and the CI matrix. It does not implement debug/monitoring; DWARF/CodeView,
checkpoint hooks, typed access, forcing, breakpoints, and the debug protocol remain M6.5.

## 1. Objective

Deliver the v1 user workflow on the M5 host/module model:

- a complete `stc.toml` project schema, local path dependency graph, `stc.lock`, and
  package-aware namespace/visibility rules;
- configurations, resources, multiple tasks, priorities, intervals, `SINGLE`, and a deterministic
  cooperative scheduler;
- file-backed retained state with warm/cold restart and the online attach/cold-download policy;
- I/O device-tree, project-binding, `VAR_ACCESS`, and input-default metadata plus the v1
  `null`/`memory`/`file` runtime backends;
- `stc new/check/build/run/test/fmt/clean/metadata/compliance` with JSON diagnostics, verbose
  output, lint levels, colors, and stable exit codes;
- a test-only unit/assertion/runner model with process isolation, timeouts, mock clocks, and
  expected-trap support;
- a minimal fixed-style formatter over the lossless CST;
- the online-change workflow: build against the active baseline, deploy/reload online with
  `stc run --watch`, review a compatibility report, `stc online download/diff`, source-aware
  `stc rename`, and cold recovery from a missing/stale baseline;
- Lean exact-function models, proofs, and committed vectors consumed by Rust tests;
- a conformance corpus, UI diagnostic snapshots, traceability, `fuzz_manifest`, and active CI
  jobs for unit/snapshot, conformance, execution, projects, fuzz-smoke, and Lean.

## 2. Scope

### 2.1 In scope

| Area | Work |
| --- | --- |
| Project system | Full `stc.toml` schema, source discovery, local path dependency graph, `stc.lock`, `--locked`, package-aware namespaces and `INTERNAL` visibility |
| CLI | `new`, `check`, `build`, `run`, `test`, `fmt`, `clean`, `metadata`, `compliance`, `online download/diff`, `rename`, `run --watch`; JSON/short diagnostics, `-v`/`-vv`, `-q`, color, lint levels |
| Configurations | Configuration/resource/task selection; multi-task scheduling; intervals, priorities, `SINGLE`, cycle-latched clocks, mock clocks |
| State | File-backed retained state, atomic save, warm/cold restart, mismatch policy, `--state-file`/`--reset-retain`/`--no-retain`, online attach vs. cold download |
| I/O | Device tree/channels, project bindings, `{stc:io}` precedence, `VAR_ACCESS`, input defaults, `null`/`memory`/`file` backends, per-cycle latch/publish |
| Build | Project defaults, `target/stc/<profile>`, `module`/`exe`/`lib`/`ir`/`obj`/`metadata`, shared host libraries, deterministic artifacts |
| Tests | `{stc:test}` discovery, `STC_ASSERT`, test-only units, test prelude helpers, isolated runner, filters/list/no-run/timeouts/JSON, separate test project |
| Format | Fixed style over the CST, `--check`, comments preserved, strict semicolons, idempotence |
| Online workflow | Active-baseline builds, compatibility reports, `stc online download/diff`, `stc rename` history, `stc run --watch` local deploy control, cold recovery |
| Verification | Lean models/proofs/vectors, Rust vector tests, conformance corpus, UI tests, traceability report, `fuzz_manifest`, CI matrix |
| Docs | README, project plan, conformance profile, architecture/runtime/state/I/O/CLI/testing/online-change specs, decisions, this plan |

### 2.2 Out of scope

- Full debug/monitoring (M6.5): DWARF/CodeView, checkpoint hooks, typed access, watch/force,
  breakpoints, stepping, call stacks, the debug session protocol and authentication, and the
  manifest-assisted split-object view.
- `--release`, stripped/optimised artifacts, debug/release build-ID separation, and debug
  information emission. M6 parses the manifest keys and keeps the existing descriptor flags, but
  builds the unoptimised debug form without DWARF/CodeView or checkpoint hooks (M6.5).
- Field-layout migration, resizing, `GotId`-keyed retained-state migration, and record/handle
  migration across restarts.
- Remote/network deployment, multi-user scheduling, a debugger UI, and a network package
  registry/version solver.
- Hardware I/O backends (EtherCAT, Modbus, MQTT), their schedules, and runtime override
  configuration beyond the v1 backend selection.
- Multi-threaded ST execution. M6 adds host control/service threads for deployment and status;
  only the cycle thread executes ST.
- Migrating the synthesized standard FB bodies to a textual `stdlib/` package. The package graph
  makes that possible, but D-134's synthesized-HIR implementation remains v1 behavior.
- `stc lsp`, `stc doc`, and a configurable/range formatter.
- Native Linux AArch64 host support (M8). M6 CI keeps Windows x64 and Ubuntu x64.

## 3. Architecture summary

```text
stc.toml graph
   -> stc-manifest (schema, sources, dependencies, lockfile)
   -> ordered packages
   -> lexer/parser per file -> one merged HIR with package-qualified namespaces
   -> codegen -> module + manifest + metadata (unchanged ABI 2 formats; 2.2 only for test builds)
   -> host: cycle thread | task scheduler | state/IO | control thread
```

- **One build, one merged HIR.** Packages are compiled together into one HIR crate with globally
  unique `DefId`s. `Def.package` records the owning package; each package has a root namespace
  derived from its manifest name (or dependency alias). Cross-package references resolve through
  package-qualified names and `USING`; `INTERNAL` is checked against package identity, not alias.
- **Package identity is stable.** Catalog keys and `GotId` scopes use the dependency's
  `[package] name`, not its alias, so the same package keeps its identity when a parent renames an
  alias. Mangled symbols and GOT keys include the package name.
- **The host schedules tasks.** The manifest already carries `TaskRecord { single, interval_ns,
  priority, guard_got_id }`. The host owns task run state keyed by task `GotId`; before each scan
  it sets each task's host-owned `TaskGuard` root to "run this scan" or "skip", and the generated
  `stc_module_cycle` runs the instances whose guard is set in plan order. `SINGLE` completion,
  deadlines, and task topology preservation live in the host; a task/resource topology change
  requires a cold download.
- **State and I/O are host-owned.** The ABI 2 `state_load`/`state_save` services become real
  file-backed services with atomic replace. The host applies input defaults and backend reads at
  the scan boundary, runs due tasks, and publishes outputs once.
- **Online deploy is a local control channel.** `stc run --watch` starts the launcher with a
  local-only control endpoint (named pipe on Windows, Unix socket on Linux, nonce handshake) and
  sends module/manifest pairs. The host queues the request, validates/loads/prepares the candidate
  between cycles, and activates at the next boundary. Debug/monitoring has a separate, authenticated
  protocol in M6.5; M6's control channel is for deployment/status only.
- **Determinism is unchanged.** Package order is topological with deterministic tie-breaking;
  source order is package order then lexicographic path; lockfiles use forward slashes and no
  absolute paths; double builds remain byte-identical; all new metadata is canonical JSON.

## 4. Exit criteria

1. `stc new/build/check/run/test/clean` work end-to-end on Windows x64.
2. The full `stc.toml` schema is implemented: package, sources, path dependencies, `stc.lock`,
   `--locked`, build/run/retain/I/O/online/lint/test settings, and unknown-key diagnostics.
3. Package-aware namespaces and `INTERNAL` visibility work across a dependency graph; a separate
   test project can depend on the code under test.
4. A configuration with multiple resources/tasks/intervals/priorities/`SINGLE` runs
   deterministically; all timers in one scan see one latched cycle time; task state survives a
   compatible swap and resets on cold download.
5. File-backed retained state round-trips across restart; layout/profile/format mismatch,
   corruption, atomic-save failure, `--reset-retain`, and `--no-retain` follow the documented
   policy; online attach preserves matching roots and cold download applies defaults and only
   compatible `RETAIN` values.
6. `stc.toml` device trees, project bindings, `VAR_ACCESS`, and input defaults are emitted in
   metadata; `null`/`memory`/`file` backends behave as specified; binding precedence is enforced;
   unbound inputs receive defaults each cycle.
7. `stc test` discovers `{stc:test}` units, compiles test-only sources, resolves test-only
   `STC_ASSERT`, isolates each test in a process, supports cycles/timeouts/mock clock/I/O/trap
   expectations/filters/list/no-run/JSON, and excludes test code from production artifacts (the
   release bind M6.5 will build keeps the same exclusion).
8. `stc fmt` is idempotent, preserves comments and line breaks, emits strict semicolons, supports
   `--check`, and does not rewrite tokens it does not understand.
9. A user can build against the active manifest, deploy/reload online, review the compatibility
   report, download the active manifest, build a compatible change from it, and recover from a
   missing/stale/cold baseline through the documented cold download.
10. `stc rename` rewrites source and records a rename history that preserves `GotId`s across the
    next build; tracked field/method/type/POU moves work.
11. Lean models/proofs build without `sorry`/`admit`; `lake exe vectors` is deterministic and
    committed; Rust vector tests and C-harness samples pass.
12. The conformance corpus and UI snapshots cover implemented rule categories; the traceability
    report shows every implemented normative rule has a test or an explicit "not implemented"
    entry; CI runs unit/snapshot, conformance, execution, projects, fuzz-smoke, and Lean jobs on
    the documented matrix.
13. `stc new/build/check/run/test/clean`, the project manifest, `stc.lock`, state, I/O, online
    workflow, and testing docs describe the implemented behavior truthfully; no M6.5 debug
    capability is claimed.

## 5. Work breakdown

| Phase | Work | Acceptance |
| --- | --- | --- |
| 0 | Resolve the M6 decision questions; fix the manifest schema, diagnostics, JSON report shapes, and M6/M6.5 boundary | Reviewed decisions; stable names/codes |
| 1 | `stc-manifest`: full `stc.toml`, source discovery, dependency graph, `stc.lock`, `--locked` | Schema/graph/lock/source tests; deterministic lockfile; `fuzz_manifest` |
| 2 | Multi-package front end: package root namespaces, cross-package resolution, `INTERNAL`, configuration selection | Dependency projects resolve/build; access and alias tests |
| 3 | Project CLI: `new/check/build/clean/metadata/compliance`, build settings, artifacts, JSON/verbose/lints/color | CLI integration for every verb and exit code; scaffold builds |
| 4 | Configurations/tasks: host scheduler, guards, task state, `stc run`, mock clock, cycle-time, signals | Multi-task scheduling and run-loop tests; online task preservation |
| 5 | Retained state and I/O: file-backed state, policies, device tree/bindings/VAR_ACCESS/defaults, backends | State round-trip/corruption/atomic tests; I/O metadata and backend tests |
| 6 | `stc-test`: test build mode, `STC_ASSERT`, test prelude, process-isolated runner, filters/JSON | Function/FB/program tests; timeout/trap; production exclusion |
| 7 | `stc-fmt`: fixed style, `--check`, comments, strict semicolons, idempotence | Formatter tests; CLI integration |
| 8 | Online workflow: reports, `online download/diff`, `rename` history, `run --watch` control/deploy, cold recovery | End-to-end watch swap, download/rebuild, diff, rename, cold tests |
| 9 | Lean: Lake project, exact models/proofs, vector generator, Rust/C-harness vector tests | `lake build` clean; vectors committed; tests pass |
| 10 | Conformance/UI/traceability/`fuzz_manifest`/xtask/CI matrix | CI jobs active; traceability report complete |
| 11 | Documentation and M6.5 handover notes | Specs/README/plan/decisions consistent; checks pass |

Phase status: phases 0–9 are complete (2026-10-27); phases 10–11 are pending. Phase 0 fixed
the manifest spelling, diagnostic families, JSON report shapes, M6/M6.5 boundary, and the
test-build, scheduler, and rename-history interfaces. Phase 1 added `crates/stc-manifest`, the
resolved project graph and lockfile API, and `fuzz_manifest`. Phase 2 added the merged
multi-package analyzer, package-aware visibility and configuration selection, and package
identities in codegen/GOT artifacts and source fingerprints; the driver now compiles `stc.toml`
projects. Phase 3 added the project CLI (`new`/`check`/`build`/`clean`/`metadata`/`compliance`),
manifest build settings and toolchain precedence, human/short/JSON diagnostics with colors and
lint levels, verbose phase timings, shared host libraries, and the baseline compatibility
summary; the later verbs are parsed but return a usage error until their phase. Phase 4 removed
the one-task-per-resource restriction, added the cooperative host scheduler and guarded cycle,
the rewritten launcher loop with mock clocks and stop signals, `stc run`, `stc_host_run`, and
the host status surface. Phase 5 added the `STCRET01` state file with CRC32 and atomic replace,
host `state_load`/`state_save` wiring and the policy overrides, project device/binding/
`VAR_ACCESS` metadata with `source`/`direct_id`/effective binding fields, and the `null`/
`memory`/`file` host backends with per-scan input defaults and one output publish. Phase 6 added
`crates/stc-test`, the `stc test` command and test-build mode, the test-only prelude, host ABI
2.2 service 12, and the process-isolated runner with filters, list/no-run, timeouts, traps, mock
clocks, I/O, and human/JSON reports; production builds exclude test units and assert calls.
Phase 7 added `crates/stc-fmt`, the fixed-style lossless-CST formatter, and `stc fmt`
(`--check`, manifest-free paths, and root-package source formatting) with strict-semicolon
insertion, preserved comments/blank lines/BOM/per-break line endings, and stable idempotent
output. Phase 8 added the online-change workflow: `stc-runtime` is a regular driver dependency;
`stc build` reads `[online].baseline = "active"` from `[runtime].manifest-dir` (or an explicit
manifest) and merges `stc.renames.toml` with CLI `--rename` entries (chains composed, conflicts
`STC-E-PROJECT-013`); `stc rename` rewrites sema-resolved declarations and references and appends
the history; `stc online download` exports a validated store or live-host download envelope, and
`stc online diff` uses the manifest-only runtime comparison; `stc run --watch` launches the host
with a user-only named-pipe/Unix-socket control endpoint (versioned protocol, per-run nonce),
watches sources/manifests/history, rebuilds against the active baseline, deploys, reports the
compatibility result and prepare time, retries once on a stale baseline, keeps the old generation
on build/deploy failure, and exits 130 on Ctrl-C. The host queues deploy/status/download/stop
requests for the cycle thread and activates prepared candidates between scans. Phase 9 added the
Lake project under `lean/` with the `Stc.Exact.Int/Bits/Conv/Select/Bool/Strings` models and their
range/overflow/round-trip proofs (no `sorry`/`admit`), the deterministic `Stc.Vectors.Gen`
generator and `lake exe vectors` entry point, 30 committed `tests/vectors/exact/*.json` files
recording the format/model versions and an FNV-1a generator digest, the `stc_codegen::exact`
reference module, `crates/stc-codegen/tests/lean_vectors.rs` (registry coverage, per-case model
agreement, digest check, and a constant-folding agreement run over the foldable operations), and
the `crates/stc-driver/tests/exact_vectors_cli.rs` C-harness sample that executes arithmetic,
shifts/rotates, selection, and string lengths through generated code and checks an overflow trap
through a module cycle. Vectors group one operation across its type instantiations with a per-case
`type` field (the conversion file carries the full 16-by-16 `X_TO_Y` matrix); the registry requires
every implemented operation/type pair. The agreement run surfaced and fixed three latent
implementation bugs: sub-64-bit `SHL`/`SHR` emitted mismatched LLVM operand types, `SEL` fetched the
selector type from `IN0`, and constant folding evaluated integer arithmetic on the REAL path and
accepted `MIN DIV/MOD -1` where generated code traps. The CI `lean` job is active through the
existing `lean/lakefile.lean` detection and regenerates the vectors to fail on drift.
Phases may proceed in parallel only where the dependency direction allows (for example, Lean and
the conformance harness can start once phase 0 fixes the vector and corpus contracts).

## 6. Phase details

### Phase 0 — Contracts and decision questions (complete 2026-10-26)

- [x] Resolve Q6-1–Q6-15 and record them as D-153–D-163 (§13).
- [x] Fix the `stc.toml` spelling and schema: canonical keys are kebab-case as in
  [CLI and Manifest](Specification/Architecture/CLI-and-Manifest.md) §2 (`opt-level`, `out-dir`,
  `lib-type`, `on-mismatch`, `module-dir`, `manifest-dir`, `max-retired-generations`,
  `cold-policy`, `allow-untracked-removal`, `force-sessions-end-on-swap`, `cycle-time`,
  `default-cycles`, `mock-clock`, `expect-trap`, `input-bytes`, `output-bytes`). Snake_case
  spellings are unknown keys, not aliases. The README example's snake_case keys are stale and
  are corrected in phase 11.
- [x] Fix new diagnostic families and JSON report shapes (§8,
  [Conformance Profile](Specification/01-Conformance-Profile.md) §10, and
  [CLI and Manifest](Specification/Architecture/CLI-and-Manifest.md) §1.6). Codes are unique
  across the existing families; `STC-W-ONLINE-002` keeps its initializer meaning and the
  compatibility-report warning is `STC-W-ONLINE-007`.
- [x] Fix the M6/M6.5 boundary: M6 keeps the host ABI 2 major and the existing descriptor and
  manifest formats; it appends only the test-clock service (host ABI 2.2, service ID 12,
  `set_cycle_time_ns(int64_t)`, Q6-15). It does not append debug services or emit checkpoint
  hooks. The control channel is deployment/status only, not the debug protocol (D-155/D-158).
- [x] Sketch the test-build mode, the task scheduler, and the `stc.renames.toml` history format
  in §14 and review them with the GOT, state, I/O, and ABI contracts. The sketches fix the
  phase-1 schema extensions, phase-4 scheduler order, phase-6 test-artifact interface, and the
  phase-8 rename-history file, so later phases do not reopen them.

Acceptance: decisions recorded; no code; the phase-1 schema and the phase-8 workflow have stable
interfaces. Phase 0 also records D-164 for the applied contract fixes.

### Phase 1 — Project manifest, source discovery, and dependency graph (complete 2026-10-26)

Add `crates/stc-manifest` (workspace member):

- Parse `stc.toml` with a span-aware TOML reader (`toml_edit` is the proposed dependency) into an
  explicit model; reject unknown keys with a "did you mean" suggestion; validate `schema = 1`,
  `[package]` name/version/language, and all table types.
- Implement source discovery from `[project].sources` and `[test].sources`: `*`, `**`, `?`, and
  character classes; hidden directories and `target/` excluded; symlinked directories not
  followed; symlinked files accepted only inside the project; a glob matching nothing is an
  error; `.st` files only; deterministic path ordering.
- Resolve `[dependencies]` local path entries: canonicalize, deduplicate by canonical path, reject
  cycles, build a topological order, and support an explicit alias (`name = { path = "...",
  package = "real-name" }`). A dependency without `path` is an error in v1. Package identity is
  the dependency's `[package].name`; the alias is its root namespace.
- Parse and validate `[project]`, `[build]`, `[io]`, `[[io.devices]]`,
  `[[io.devices.channels]]`, `[[io.bindings]]`, `[retain]`, `[runtime]`, `[online]`, `[debug]`,
  `[lints]`, `[run]`, `[test]`, and `[[test.case]]` against the documented schema.
- Write/check `stc.lock`: schema 1, packages sorted by `(name, source)`, dependency lists sorted,
  root `source = "root"`, dependency paths normalized forward-slash relative paths, no absolute
  paths. `--locked` fails on a missing or changed lockfile; normal builds update it.
- Define the graph API the driver consumes: ordered packages, each with manifest path, root
  directory, sources, test sources, dependencies, alias/root namespace, and build settings.
- Add `fuzz_manifest` to `fuzz/` and seed it with valid/invalid manifests.

Tests: schema positives/negatives (unknown keys, wrong types, missing fields, bad names/versions),
path containment and forward-slash rules, glob discovery (case, hidden, target, symlink, empty),
dependency aliases/dedup/cycles, lockfile determinism and `--locked`, and manifest fuzz smoke.

Status (2026-10-26): implemented in `crates/stc-manifest` with `parse_manifest`,
`parse_lockfile`, and `load_project` → `ProjectGraph`. The crate parses the full schema with
span-mapped `STC-E-PROJECT-*`/`STC-E-LOCK-*` diagnostics and did-you-mean suggestions, discovers
production and test sources with the documented exclusion/symlink rules, resolves deduplicated
path dependencies in deterministic topological order with alias/name identity separation, and
writes/checks the canonical `stc.lock` (including `--locked`). `fuzz_manifest` and committed
valid/invalid seeds are in `fuzz/`; the stable-toolchain seed smoke test runs with the crate tests.
The driver is not wired to the new crate yet, and no analyzer changes were made.

Acceptance: `stc-manifest` resolves a project graph and lockfile deterministically; no analyzer
changes yet.

### Phase 2 — Package-aware front end

- Extend `stc_sema` with a multi-package entry point: the driver passes ordered package sessions
  and the files belonging to each package. Allocate one `PackageInfo` and root namespace per
  package, assign `Def.package` during collection, and keep `DefId`s globally unique.
- Resolve cross-package names through package root namespaces, dependency aliases, and `USING`;
  keep `PRIVATE` POU/class visibility and make `INTERNAL` package-private. Duplicate top-level
  names in different packages are allowed; duplicates in one package remain errors.
- Restrict `[project].configuration` to the root package's configurations (or the synthesized
  `_DEFAULT` when none exists); dependency configurations are declarations, not build entries.
  Support `[project].stdlib = false`: standard FB synthesis is suppressed, standard functions
  remain, and the project's own definitions are unaffected.
- Thread package names through `stc-codegen`/`stc-got`: catalog keys, `GotId` scopes, mangled
  symbols, source paths (package-relative), and the source fingerprint (all package sources in
  order) include the package graph. No new artifact format is introduced.
- Driver: load the graph, lex/parse files in package-topological order, run the merged analysis,
  and report package-qualified diagnostics. The manifest-free `stc <command> <file>...` bridge is
  retained for tests and one-off checks, but project commands use `stc.toml`.

Tests: cross-package function/FB/type/global references, alias-vs-name identity, `INTERNAL`
rejection, duplicate names across packages, `USING` across packages, configuration selection,
`stdlib = false`, deterministic file/package order, and a separate test project dependency.

Status (2026-10-26): implemented. `stc-sema` exposes `ProjectSession`/`PackageSession` and
`analyze_project`, which allocate one `PackageInfo` and root namespace per package in the merged
HIR, stamp `Def.package` during collection, link dependency aliases into dependent root scopes,
resolve qualified and `USING` names across packages, and enforce `INTERNAL` as package-private
while keeping `PRIVATE` member visibility. Only root-package `CONFIGURATION`s are build entries;
an explicit `[project].configuration` that names a dependency configuration or no configuration
reports `STC-E-PROJECT-007`. `[project].stdlib = false` suppresses standard FB synthesis but
keeps standard functions. `stc-codegen`/`stc-got` now include the package identity in mangled
symbols, use package-relative source paths and a package-qualified source fingerprint, and select
configurations/programs only from the root package. The driver compiles a project when
`stc check`/`stc build` has no input files or `--manifest-path` is given: it lexes/parses sources
in package-topological order, runs the merged analysis, renders `<package>/<relative>` source
paths, and defaults project builds to `--emit module` under `<project>/target/stc/debug`. The
manifest-free `<file>...` bridge is unchanged and still defaults to `--emit ir`. Artifact formats
are unchanged; the full project CLI/build settings remain phase 3.

Acceptance: a two-package project builds a module whose GOT keys use package identities and whose
cross-package calls resolve.

### Phase 3 — Project CLI, build settings, and artifacts

- Expand the driver parser: commands `new`, `check`, `build`, `run`, `test`, `fmt`, `clean`,
  `metadata`, `compliance`, `online download`, `online diff`, `rename`, and the `run --watch`
  flag. Keep `parse`; make file-based `check`/`build` the explicit manifest-free bridge.
- Add `--manifest-path`, `--locked`, `--message-format human|json|short`, `--color auto|always|
  never`, `-q`, `-v`/`-vv` (phase timings; `-vv` adds per-file phases, the exact clang command,
  and retained IR paths), `-D`/`-W`/`-A`, `--deny warnings`, and the
  manifest/CLI/environment precedence from CLI-and-Manifest §1.2.
- Implement project defaults: default emit `module`, out-dir `target/stc/debug`, name from
  `[package].name`, target/clang/runtime discovery with CLI > `STC_*` > manifest > defaults,
  `[build].opt-level` to clang, `syntax`, `lib-type` (`static` or shared host library), and
  source fingerprinting across packages. The manifest-free `stc build <file>...` bridge keeps its
  M4.5 defaults (`ir`, sanitized file stem, `target/stc/debug`).
- Implement `stc new` (scaffold `stc.toml`, `src/main.st`, `tests/smoke.st`, `.gitignore`),
  `stc clean` (remove the configured out-dir; never remove `stc.lock` or state files),
  `stc metadata` (human and JSON summary of configuration, ABI, tasks, exports, retain, I/O,
  artifacts), and `stc compliance` (profile ID, feature matrix, extensions, runtime capabilities,
  non-safety statement).
- Emit `module`/`exe`/`lib`/`ir`/`obj`/`metadata` under the project defaults; add the shared host
  library form for `lib-type = "shared"` (runtime `cdylib` or equivalent shared link) while
  keeping generated code in the separate module.
- Print the compatibility summary when a baseline is used: preserved/allocated/dropped/rejected
  counts and warnings, with a stable JSON object.
- Add the project-manifest diagnostics and lockfile diagnostics; `-D warnings`/`--deny warnings`
  map warnings to errors without hiding the original code.

Tests: every verb and exit code, `new` scaffold build/run/test, manifest defaults/overrides,
target/clang/runtime precedence, `--locked`, out-dir containment, shared/static `lib`, short/JSON
diagnostics, color off/on, `-q`, `-v`/`-vv`, lint overrides/unknown codes, and `metadata`/
`compliance` shapes.

Status (2026-10-26): implemented. The driver parses the full M6 command surface; `new`, `check`,
`build`, `clean`, `metadata`, and `compliance` are implemented, while `run`/`test`/`fmt`/
`online`/`rename` are parsed and return a usage error until phases 4–8. Project builds use the
manifest emit list (default `module`), `target/stc/debug`, `[package].name`, `[build].opt-level`,
`syntax`, and `lib-type`; target/clang/runtime discovery follows CLI > `STC_TARGET`/`STC_CLANG`/
`STC_RUNTIME` > manifest > default, and the manifest-free `<file>...` bridge keeps `ir`, the
sanitized file stem, and `target/stc/debug`. `stc build` emits `module`/`exe`/`lib`/`ir`/`obj`/
`metadata`, including `stc-runtime`'s cdylib for `lib-type = "shared"` while generated code stays
in the separate module. Diagnostics support human/short/JSON, `--color auto|always|never`, `-q`,
`-v`/`-vv` phase timings (per-file lex/parse, clang commands, and IR paths at `-vv`), and
`-D`/`-W`/`-A`/`--deny warnings` with manifest `[lints]` beneath CLI overrides; a warning
promoted to an error keeps its original code. Project/lockfile failures exit 2, compile errors
exit 1, and usage/toolchain errors exit 2. `stc new` writes the documented scaffold, `stc clean`
removes the configured out-dir while preserving `stc.lock` and retained-state files, `stc
metadata` and `stc compliance` print the §1.6 summaries, and a baseline build prints the
compatibility verdict, counts, warnings, and (with `-v`) per-record actions.

Acceptance: `stc new/build/check/clean/metadata/compliance` work end-to-end on Windows; the
manifest-free bridge still passes its existing tests.

### Phase 4 — Configurations, tasks, and `stc run` (complete 2026-10-26)

- Remove the one-task-per-resource restriction in `stc-codegen::config`; project every task with
  its interval/priority/`SINGLE` and guard identity. Update the conformance profile and runtime
  docs for multi-task scheduling.
- Host scheduler in `stc-runtime` (§14.5):
  - task run state keyed by task `GotId` (interval deadline, `SINGLE` completion);
  - before each scan, set each task's `TaskGuard` root to run/skip;
  - tasks without an interval run every scan; interval tasks first run after one interval, then
    no catch-up (`next = now + interval`); `SINGLE` runs once and completes;
  - priority: lower numeric value is higher priority; an absent priority sorts after explicit
    priorities; ties by resource/task declaration order;
  - due tasks run in one scan; inputs are latched once and outputs published once;
  - task state is preserved across a compatible swap by `GotId`; a task/resource topology change
    is rejected or cold; cold download resets completion/deadlines.
- Generated `stc_module_cycle` runs instances whose task guard is set, in scheduler order; it no
  longer clears `SINGLE` guards. The compiler emits task-bound instances in scheduler rank order
  (priority, then resource/task declaration order, then program declaration order); task-less
  instances run unguarded in declaration order before the scheduled instances. Unbound instances
  continue to run every scan.
- Rewrite the launcher loop: default cycles from `[run].cycles` (absent or `0` means until
  interrupted), `--cycles`, `--cycle-time`, `--io`, `--state-file`, `--reset-retain`, `--no-retain`,
  stop signals, and exit codes 0/1/2/3/4/130. Sleep to the next scan deadline when a cycle time is
  configured; mock-clock runs advance deterministically without sleeping.
- Implement `stc run`: build `exe,module` (and metadata), start the launcher, pass options, and
  return its exit code. Keep `--watch` as a phase-8 workflow.
- Add the embedding-host run surface the runtime spec defers to M6: `stc_host_run` and an options
  struct carrying cycle count/time, I/O backend/paths, state policy, stop conditions, and the
  optional control endpoint. The launcher and embedding hosts share one loop implementation.
- Add a host status surface (active/pending/retired build IDs, cycles, last rejection, warnings)
  for `stc run -v` and the control channel.

Tests: multi-task ordering/priorities/intervals/`SINGLE`, latched clock for timers across tasks,
task guard preservation across a warm swap, cold reset, run-loop cycle/timeout/signal/exit codes,
mock clock, and `[run]` defaults.

Status (2026-10-26): implemented. `stc-codegen::config` projects every task with its
interval/priority/`SINGLE`/guard identity, removes the one-task-per-resource restriction, and
orders the generated cycle with task-less instances first and task-bound instances in scheduler
rank order (priority, resource/task declaration, then program declaration). The generated
`stc_module_cycle` reads every task-bound instance's guard in that order and never initializes or
clears a guard. The new `stc-runtime::scheduler` owns task state keyed by task `GotId` (interval
deadline, `SINGLE` completion), reconciles it across compatible swaps, resets it on cold installs,
and writes each `TaskGuard` value before every scan; `Host::cycle_at` runs one scan at an explicit
latched time, and `Host::status` exposes active/pending/retired builds, cycles, last rejection,
warnings, and per-task due/completed/deadline state. `Host::load` rejects a task without a guard
as `STC-E-RUN-003`; a changed task schedule is rejected or requires a cold download. The shared
`run` module drives both the rewritten launcher and `stc_host_run`: `--cycles` (absent or `0`
runs until interrupted), `--cycle-time`, `--io`, `--io-file`, `--state-file`, `--reset-retain`,
`--no-retain`, `--mock-clock`, `--status`, and `--module`; it sleeps to the cycle deadline, runs
mock-clock scans without sleeping, installs Ctrl-C/Ctrl-Break handlers, and returns
0/1/2/3/4/130 (a trap maps to 4; raw `stc_rt_trap` keeps 70 outside a host boundary). `stc run`
builds `exe,module,metadata`, resolves `[run]` defaults and CLI overrides, starts the launcher,
and returns its exit code. `stc_host_run` and `stc_host_options` add the embedding surface; the
control endpoint is reserved for phase 8, and the I/O and retained-state paths are carried for
phase 5. `--trace` remains a clear runtime option error.

Acceptance: `stc run` executes a multi-task configuration deterministically on Windows; existing
single-task tests keep passing.

### Phase 5 — Retained state and I/O

Retained state (`stc-runtime`, driver, metadata):

- Implement a state module with the State-and-RETAIN §3 file format: magic, version, ABI, profile
  hash, layout hash, payload length, payload, CRC32; little-endian; unknown versions rejected.
- Wire the ABI 2 `state_load`/`state_save` services to the host's configured state file. Return
  0/1/2/3/4/5 per the spec. Load before prepare; save atomically at normal shutdown.
- Atomic save: `<state-file>.tmp` in the same directory, flush/sync, replace; a failed replace
  leaves the previous file intact and reports `STC-E-RUN-RETAIN-003`.
- Policy: `[retain].enabled`, `file`, `on-mismatch = "error" | "reset"`; `--state-file`,
  `--reset-retain`, `--no-retain` overrides. `stc run` resolves project-relative paths to
  absolute paths; a standalone launcher resolves a relative `file` against the module directory
  and documents it.
- Metadata carries `retain.enabled/file/on_mismatch` for the runtime; layout/size/hash/leaves stay
  as today.
- Online attach: no state load; matching roots preserved; the state file is not rewritten merely
  because of a swap; normal shutdown save still applies. Cold download: stop/reinitialize,
  defaults plus compatible `RETAIN` only, transients not preserved. Baseline problems select cold
  download.
- Tests: scalar/aggregate/string/FB round-trips, warm/cold, `NON_RETAIN`, layout change under both
  policies, corrupt magic/version/length/CRC, atomic failure, `--no-retain`, `--reset-retain`,
  online attach/cold, and layout-hash stability.

I/O (`stc-codegen` metadata, `stc-runtime`, driver):

- Emit the project device tree (`name`, opaque `kind`, opaque canonical `config`, optional
  channels), project bindings, `VAR_ACCESS` declarations, effective binding source
  (`runtime` is not compiler metadata; compiler emits `pragma` or `project`), `offset_bits`,
  `width_bits`, `got_id`/`direct_id` where applicable, and `input_defaults_hex`/
  `output_defaults_hex`.
- Resolve `[[io.bindings]].variable` against HIR; validate declared devices/channels/directions;
  apply precedence runtime override > project binding > pragma > raw address; emit
  `STC-W-IO-003` when a project binding overrides a pragma; unknown `{stc:io}` devices stay
  `STC-W-IO-001`.
- Runtime backends: `null` (unbound inputs receive defaults each scan; outputs discarded),
  `memory` (embedding/test input/output bytes), `file` (input image read at cold start, output
  image written atomically at shutdown; `--io-file` overrides). `--io` overrides metadata.
- Host applies input defaults for unbound locations at scan start and overlays bound values from
  the backend; outputs publish after all due tasks. `%M` persists; `%I` writes remain rejected;
  direct cells are rebound through the existing image path.
- Emit `VAR_ACCESS` metadata for runtime clients; the runtime may expose the metadata text through
  the existing metadata accessor.
- Tests: metadata snapshots for devices/bindings/var-access/defaults; precedence and validation;
  null/memory/file behavior; unbound defaults each cycle; `%M` persistence; online image
  reconfiguration with the new metadata.

Status (2026-10-26): implemented. The new `stc-runtime::state` module owns the `STCRET01`
format (little-endian header, profile/layout hashes, CRC32), the ABI 2 result codes
(0/1/2/3/4/5), `error`/`reset` mismatch policies, and atomic `<file>.tmp` saves with a
replace-on-rename that leaves the previous file intact (`STC-E-RUN-RETAIN-003`). `Host`
installs the effective `RetainConfig` before prepare, permits only the active generation's
normal shutdown to save, refuses saves from retired generations during an online swap, records
`STC-W-RUN-RETAIN-001/002`, and fails a cold load on `STC-E-RUN-RETAIN-001/002` under `error`.
The generated retain layout hash now covers the documented profile/ABI/size header and each
leaf's stable root identity, type, offset, and size. The driver resolves `[retain].file` and
`[io].file` against the project root and CLI state/I/O paths against the current directory
before launching; a standalone launcher or embedding host resolves relative metadata paths
against the module directory. `stc-runtime::io` implements the `null`, `memory`, and `file`
backends: inputs are latched once per scan (defaults for unbound locations, host bytes for
`memory`, the cold-read image for `file`), unbound outputs are discarded or published once, `%M`
persists, and the file backend writes its output image atomically at shutdown with a missing
input reported as `STC-W-IO-004`. Codegen emits declared devices (opaque canonical config),
channels, resolved project bindings with `source`, `direct_id`, `got_id`, and effective
device/channel/direction, `VAR_ACCESS` paths/types/direct locations, and the `retain`
`enabled`/`file`/`on_mismatch` policy; `STC-W-IO-001` and `STC-W-IO-003` are reported during
code generation and unresolved project bindings are `STC-E-PROJECT-009`. `stc metadata` reports
the resolved bindings/devices/`VAR_ACCESS` and the metadata `retain` policy, and `stc run`
passes the resolved absolute paths and the retain overrides to the launcher. Tests cover the
state format and codes, scalar/aggregate/string/FB round-trips, `NON_RETAIN`, warm/cold and
layout-change policies, corruption, atomic failure, `--state-file`/`--reset-retain`/
`--no-retain`, online attach/cold download, layout-hash stability, metadata resolution and
precedence, and `null`/`memory`/`file` behavior including per-scan defaults and `%M`
persistence.

Acceptance: state round-trips and the documented policies pass on Windows; I/O metadata and
backends match IO-and-Devices; online attach/cold behavior matches State-and-RETAIN §4.1.1.

### Phase 6 — Test runner (complete 2026-10-26)

- Add `crates/stc-test` (discovery/reporting/runner) and a codegen test-build mode.
- Discovery: `{stc:test}` on `FUNCTION`, `FUNCTION_BLOCK`, and `PROGRAM`; test `FUNCTION` has no
  inputs and returns `BOOL` and passes when it returns TRUE; `FUNCTION_BLOCK`/`PROGRAM` tests run
  `cycles` scans, and an optional `PASSED : BOOL` output on an FB test must be TRUE at the end;
  pragma payload `cycles`, `timeout`, `expect-trap`; `[[test.case]]` overrides by POU name;
  conflicts are errors; duplicate test names in one package are errors.
- Test-only units: `[test].sources` are parsed only for `stc test`; `{stc:test}` POUs are included
  in test builds and excluded from production builds and release artifacts. A production POU that
  uses `STC_ASSERT` or references a test-only POU is an error (`STC-E-ASSERT-001`,
  `STC-E-TEST-001`).
- Test prelude (test-only units only, §14.4): `STC_ASSERT(condition, message)` lowers to host ABI
  service ID 11 (`assert`) with the call-site span; the test host records failures and continues
  instead of trapping. `STC_CLOCK_ADVANCE(T#...)` uses host ABI 2.2 service ID 12
  (`set_cycle_time_ns(int64_t)`, Q6-15/D-157) to set the next cycle's latched time.
  `STC_IO_SET_INPUT(...)`/`STC_IO_GET_OUTPUT(...)` read/write the process image through the
  existing I/O services.
- Build one test artifact per test case in v1 (a synthetic wrapper for function/FB tests, the
  test program for program tests), with a synthetic single-task configuration, no retained state,
  the memory I/O backend (unless `[test].io` overrides), a mock clock starting at
  `[test].mock-clock` (default `0ns`), and the selected test body. This keeps process isolation and
  trap semantics simple; a shared test registry can replace it later without changing the CLI.
- Runner: each test in a separate process; default timeout 5 s; kill on expiry; parse the test
  host's structured result (pass/fail/trap, assertions, duration); filters by substring;
  `--list`, `--no-run`, `--timeout`, `-- --nocapture`; human and JSON (`name`, `status`,
  `duration_ms`, `failures`) output. Tests run sequentially in v1.
- Separate test project: `stc test` compiles dependencies but discovers/runs only the current
  package's tests.
- Tests: function/FB/program pass and fail, assertion collection, cycles/clock/timers, timeout,
  expected trap, filters/list/no-run/JSON, duplicate/invalid declarations, production exclusion,
  separate project, and no test code or assert calls in production artifacts.

Acceptance: `stc test` runs the scaffold's smoke test and the suite end-to-end on Windows.

Status (2026-10-26): implemented. The new `crates/stc-test` crate owns syntax-level discovery
(`{stc:test}` on FUNCTION/FUNCTION_BLOCK/PROGRAM, signature validation, duplicate detection,
`[[test.case]]` merging and conflict diagnostics), wrapper generation, the process-isolated
runner (timeout, kill, structured result parsing), and human/JSON reports. `stc test` parses the
production sources plus the root package's `[test].sources`, discovers only the current
package's tests, and builds one artifact per test under `target/stc/test/<artifact>` with a
synthetic `STC_TEST` configuration (resource `STC_TEST_RES`, task `STC_TEST_TASK`, one
instance). The synthetic names are compiler-reserved and use lexable single-underscore
spellings (`STC_TEST_WRAPPER`, `STC_TEST_INSTANCE`, ...) because the lexer rejects leading or
trailing double underscores. Function tests get a one-shot wrapper that asserts the `BOOL`
result; FB tests instantiate the block and the host reads the `PASSED` leaf directly (FB fields
are `PROTECTED` by default); program tests configure the program itself. Test builds are always
cold, disable retention, default to the `memory` I/O backend, start the mock clock at
`[test].mock-clock`, and set the task guard through the normal host scheduler. The test host is
a generated launcher around the runtime's `stc_rt_test_main`; ABI 2.2 service 12 is imported by
test modules only, and production modules keep required minor 2.1. The test-only prelude lowers
`STC_ASSERT` to the assert service (the host records failures and continues, with the driver's
span-keyed sidecar supplying messages/locations), `STC_CLOCK_ADVANCE(T#…)` to
`set_cycle_time_ns(now + duration)`, and `STC_IO_SET_INPUT(offset, value)` /
`STC_IO_GET_OUTPUT(offset)` to the process image. Production analysis excludes `{stc:test}` and
test-source units; `STC_ASSERT` or a test-only reference in production reports
`STC-E-ASSERT-001`/`STC-E-TEST-001`, including inside test builds. `--list`, `--no-run`,
substring filters, `--timeout`, `-- --nocapture`, per-test timeouts and expected traps work, and
tests run sequentially with exit codes 0/1/4 (4 for an unexpected trap). Integration tests
cover function/FB/program pass and fail, assertion collection, cycles/clock/timers, I/O, timeout,
expected/mismatched traps, filters/list/no-run/JSON, duplicate/invalid conflict diagnostics,
production exclusion and assert calls, a user configuration, and a separate test project.

### Phase 7 — Formatter (complete 2026-10-26)

- Add `crates/stc-fmt` over the lossless CST.
- Fixed style: keywords uppercase; identifiers keep spelling; 2-space indentation; one statement
  per line; one space around binary operators, after commas, and around `:=`/`=>`; no space before
  a call's `(`; `name : TYPE`; `END_*` on its own line; strict semicolons in output even for
  `--relaxed` input; comments preserved; existing line breaks preserved; no wrapping; no
  configuration; idempotent.
- `stc fmt [paths...] [--check]`; no paths formats the project sources; paths format the given
  files (manifest-free); syntax errors prevent formatting (`STC-E-FMT-001`); unreadable/writable
  files report `STC-E-FMT-002`. Preserve BOM/line-ending encoding when rewriting bytes.
- Tests: idempotence, comment/trivia preservation, relaxed-to-strict semicolons, spacing,
  indentation, CRLF/BOM, unknown tokens untouched, `--check` exit code, and a project format run.

Acceptance: `stc fmt` followed by `stc fmt --check` succeeds on the examples and corpus.

Status (2026-10-26): implemented. `crates/stc-fmt` re-emits the lossless CST with the fixed style:
uppercase keywords, original identifier/literal spelling, two-space indentation, one
statement/declaration per line, normalized operator/comma/`:=`/`=>` spacing, `name : TYPE`
colons, `END_*` on its own line, and strict semicolons (a terminator is inserted when relaxed
input omitted one after a block statement). Comments and pragmas are preserved verbatim, blank
lines and existing line breaks are kept, and each file's BOM and per-break line-ending encoding
survive a rewrite; inserted breaks use the dominant ending. The driver's `stc fmt [paths...]
[--check]` formats the root package's production and test sources or the given manifest-free
files, writes in place, and makes `--check` write nothing and exit 1 on a difference; lexer or
parser errors prevent formatting (`STC-E-FMT-001`), and read/write failures report
`STC-E-FMT-002`. Project discovery uses a read-only graph load, so `stc fmt` never creates or
updates `stc.lock` (a `--locked` project still fails on a missing or stale lockfile). The
`stc new` scaffold is already in the fixed style, so `stc new` followed by
`stc fmt --check` succeeds. Tests cover spacing/indentation, relaxed-to-strict semicolons,
comment/blank-line preservation, one declaration per line, CRLF/BOM and mixed line endings,
unknown-token spelling, syntax errors, `--check`/idempotence, and a project run over production
and test sources.

### Phase 8 — Online-change workflow

- **Driver/runtime boundary.** The `stc online` commands, the control client, and `run --watch`
  use the runtime's store/download/diff/control APIs. Add `stc-runtime` as a regular driver
  dependency (it is currently a dev-dependency); the runtime keeps its one-way dependency on
  `stc-got` only.
- **Compatibility report.** `stc build` prints a human/JSON summary when a baseline is used:
  verdict, preserved/allocated/dropped/retired counts, warnings, and rejection codes. `-v` adds
  per-record actions. The report is derived from the existing plan/manifest data.
- **`stc online download`.** Read the active manifest from the runtime store
  (`[runtime].manifest-dir`, `--store`, or a live `--host` control request) and write the download
  envelope (canonical JSON, optional binary, deployment/layout summary). Validate the store before
  exporting; refuse to synthesize a baseline from a missing/corrupt store.
- **`stc online diff`.** Compare a candidate `.stc-got.json`/`.bin` with the active baseline or an
  explicit `--baseline`; print verdict/actions/diagnostics and exit 0 for compatible, 1 for
  incompatible/rejected, 2 for usage/parse errors. Add a manifest-only runtime comparison entry
  point so diff never executes a module.
- **`stc rename`.** Resolve a qualified key in the current package, use sema resolutions to find
  the declaration and every reference, rewrite source files without touching comments/strings, and
  record `old -> new` in a committed `stc.renames.toml` history using the schema in §14.6 and
  [CLI and Manifest](Specification/Architecture/CLI-and-Manifest.md) §3.1. Builds merge CLI
  `--rename` with the history; conflicts are errors; chains resolve oldest-first. `--dry-run`
  prints edits. The history preserves identity only when a baseline is present; a cold build is
  warned with `STC-W-PROJECT-002` that the new key receives a fresh `GotId`. Renaming a
  dependency's declaration or a package name is out of scope.
- **`stc run --watch`.** Build against the host's active baseline, start the launcher with a
  local control endpoint, watch package sources/manifests, rebuild on change, and deploy the
  module/manifest pair. The control endpoint is a named pipe (Windows) or Unix socket (Linux),
  user-only, with a per-run nonce; it is not the M6.5 debug protocol. The host queues deploy
  requests, loads/validates/prepares between cycles, replies with the compatibility report, and
  activates at the next boundary. Build errors, incompatible changes, or prepare failures leave
  the old generation active. On stale baseline, re-download and rebuild once. Ctrl-C stops the
  host, saves state, and exits 130.
- **State policy.** Online attach skips the state file; cold download applies the documented
  restart policy; a missing/stale/corrupt baseline selects cold download. `stc run --watch`
  reports which path it took.
- **Threading scope.** M6 adds a host control/status thread and queues deploy work; preparation
  runs between cycles on the cycle thread in v1 so ST is never re-entered. Fully off-thread
  prepare with bounded jitter remains an M7 hardening item; the maintenance pause is measured and
  documented. M6.5 may add service/monitor threads for debug on the same control boundary.
- Tests: download/rebuild consumes the active baseline and reproduces identities; diff compatible/
  incompatible; rename preserves values and `GotId`s across a build and a swap; watch deploys a
  code-only change, add/remove/rename, and rejects an incompatible change; build failure and
  prepare failure keep the old generation; stale/missing baseline cold recovery; control-channel
  nonce/ACL and protocol-version rejection.

Acceptance: the project-plan M6 online workflow exit criteria pass on Windows; the manifest-free
baseline paths keep working.

Status (2026-10-26): implemented. `stc-runtime` is a regular driver dependency; the driver uses
`ManifestStore`, `Download`, `compare_manifests`, and the control client/server rather than
reimplementing manifest or plan logic. Project builds resolve `[online].baseline = "active"` from
`[runtime].manifest-dir` (a missing or corrupt store warns `STC-W-ONLINE-007` and selects a cold
layout) or `[online].baseline = "explicit"`; the committed `stc.renames.toml` history is merged with
CLI `--rename` entries, chains compose oldest-first, duplicate mappings and cycles are
`STC-E-PROJECT-013`, and a cold build with renames warns `STC-W-PROJECT-002`. `stc rename` resolves
a dotted key in the root package through sema resolutions, rewrites the declaration and every
resolved reference (including `THIS`/`SUPER` members and qualified POU/type prefixes) in place,
preserves BOM/line endings/comments/strings, supports `--dry-run`, and appends the history only
after all writes succeed. `stc online download` validates the store (or requests the active
envelope from a live endpoint with the per-run nonce), writes canonical JSON and an optional binary
projection, and refuses a missing or corrupt store with `STC-E-ONLINE-009`; `stc online diff`
compares a JSON/binary candidate with `--baseline`, the store, or a live host through the runtime's
manifest-only entry point and exits 0/1/2. `stc run --watch` builds against the store's active
manifest, launches the host with a user-only Windows named pipe or Unix socket, protocol version 1,
and a per-run nonce, watches production/test sources, package manifests, the lockfile, and the
rename history, debounces changes, rebuilds without relinking the running launcher, deploys the
module/binary-manifest pair, prints the compatibility report and prepare time, retries once after a
stale-baseline rejection, keeps the active generation on build/incompatible/prepare failure, and
stops the host through the control channel on Ctrl-C (exit 130). The control thread queues
deploy/status/download/stop requests; candidate load, validation, prepare, and activation run on
the cycle thread between scans, so ST is never re-entered. Tests cover download/rebuild from the
store, download and diff through a live host, diff compatible/incompatible/parse failures,
rename identity and field/THIS rewrites with a baseline, chained history, values and `GotId`s
across an online swap, history conflicts, BOM/CRLF preservation, watch code-only/rename/new-file
deploys, build-failure recovery, Ctrl-C 130, and control nonce, protocol-version, user-only socket
permission, concurrent clients, status, stop, and download messages.

### Phase 9 — Lean vector integration

- Create the Lake project (`lean/lakefile.lean`, pinned `lean/lean-toolchain` already exists) and
  the exact modules from Testing-and-Verification §5.2: `Int`, `Bits`, `Conv`, `Select`, `Bool`,
  and `Strings`, with named models, the properties that matter, and no `sorry`/`admit`.
- Implement `Vectors/Gen.lean` and `VectorCli.lean`; `lake exe vectors` writes deterministic
  `tests/vectors/exact/*.json` in the §5.3 format, records the model version/generator hash, and
  is committed. CI regenerates and fails on drift.
- Add a Rust exact-operation reference module and a vector test harness; add
  `crates/stc-codegen/tests/lean_vectors.rs` that fails when an implemented exact function has no
  vector or a vector mismatches. Add a constant-folding agreement test where the same operation
  is folded in `stc-sema`, and a C-harness sample that executes selected vectors through generated
  code (checked arithmetic, shifts/rotates, selection, string length).
- Scope: exact integer arithmetic/overflow, bitwise, shifts/rotates, conversions, comparisons,
  selection, boolean, and exact string character/UTF-8-byte length. Floating-point/transcendental
  math remains test-based.
- CI: activate the existing `lean` job; it checks `Stc/Exact`, rejects `sorry`/`admit`, runs
  `lake build`, regenerates vectors, and fails on uncommitted drift.

Status (2026-10-27): implemented. The `lean/` Lake project builds `Stc.Exact.Int` (checked
add/sub/mul/neg/div/mod/abs with `Trap` results and range/overflow/round-trip theorems),
`Stc.Exact.Bits` (AND/OR/XOR/NOT, logical shifts, rotates over `Nat` bit patterns with range and
identity theorems), `Stc.Exact.Conv` (sign/zero extension, bit-pattern reinterpretation, checked
narrowing, and BOOL's test-against-zero rule, with narrowing/round-trip/fits theorems),
`Stc.Exact.Select` (SEL/MIN/MAX/LIMIT/MUX with bound and selection theorems), `Stc.Exact.Bool`
(standard boolean identities and De Morgan), and `Stc.Exact.Strings` (LEN/LEN_MAX/LEN_CODE_UNIT
over SBCS/UTF-16/UTF-8 with length-range theorems) without `sorry`/`admit` and without Mathlib.
`Stc.Vectors.Gen` renders a minimal deterministic JSON tree, groups one operation across its IEC
type instantiations with a per-case `type` field, and records the format version (`version: 1`), the
Lean model version (`model_version: 1`), and an FNV-1a 64 digest of the canonical case text
(`generator`); `VectorCli.lean` (`lake exe vectors`) writes the 30 committed
`tests/vectors/exact/*.json` files, and rerunning it is byte-identical. The conversion vector file
covers the full 16-by-16 ordered `X_TO_Y` matrix over BOOL, the integer types, the bit strings, and
CHAR/WCHAR/UCHAR. `stc_codegen::exact` mirrors the implementation's exact semantics (checked
arithmetic with i128 overflow detection, truncating division with the `MIN/-1` trap, masked
shifts/rotates, the codegen conversion rules including `v != 0` for BOOL targets, comparisons,
selection, and the string-length units) and exposes the exact-function registry and required type
matrix. `crates/stc-codegen/tests/lean_vectors.rs` requires every registered operation file and
type/kind/pair, validates every case against the model, verifies the generator digest, and folds the
same arithmetic/bitwise/comparison/conversion cases through `stc-sema`; the folding run exposed and
fixed three implementation bugs: `SHL`/`SHR` emitted a 64-bit shift count against narrower LLVM
operands (an internal compiler error), `SEL` read the selector type from `IN0` instead of `G`, and
constant folding evaluated integer arithmetic as REAL and accepted `MIN DIV/MOD -1` where generated
code traps. `crates/stc-driver/tests/exact_vectors_cli.rs` generates wrappers from the vectors,
builds the library, and runs `tests/harness/exact_vectors.c` over the non-trapping arithmetic,
shift/rotate, SEL, and LEN/LEN_CODE_UNIT cases; a second module checks that checked overflow traps
through `stc_host_cycle`. The CI `lean` job is active via the existing `lean/lakefile.lean`
detection: it builds the package, rejects `sorry`/`admit` under `Stc/Exact`, regenerates the
vectors, and fails on uncommitted drift. Floating-point and transcendental math remain vector- and
tolerance-tested outside the Lean models.

Acceptance: `lake build` and `lake exe vectors` pass; Rust vector tests pass; CI Lean job is
active.

### Phase 10 — Conformance, UI, traceability, fuzzing, and CI

- **Conformance corpus.** Add `tests/conformance/<area>/<case>.st` plus `.meta.toml`
  (`profile`, `kind = compile-pass|compile-fail|run-pass|run-fail|trap`, expected exit/stdout/
  trap, `cycles`, `rules`). Areas: lexical, types, conversions/overloads, statements/control
  flow, expression precedence, syntax modes, functions/FBs/programs, OOP, namespaces/USING,
  configuration/tasks, I/O, retained state, strings, standard functions/FBs, traps/diagnostics,
  and project/online cases. Every implemented rule category maps to at least one case.
- **UI tests.** `tests/ui/*.st` with expected `.stderr` and `//~ ERROR CODE` annotations; the
  harness invokes `stc check` and fails on missing/unexpected diagnostics or changed spans.
- **Harness and xtask.** Add `crates/xtask` and a `.cargo/config.toml` alias; implement
  `cargo xtask test --suite ui|conformance|execution|projects --locked` and
  `cargo xtask traceability [--check]`. The conformance/UI/projects harnesses live in driver
  integration tests and use the `stc` binary/exit codes.
- **Traceability.** Scan `Specification/**/*.md` for `STC-*` rule IDs and the tests/corpus for
  rule annotations; emit covered/uncovered/not-implemented; fail CI when an implemented normative
  rule has no test. Add an explicit allowlist for rules marked "not implemented".
- **Fuzzing.** Add `fuzz_manifest` for arbitrary TOML; keep the other targets; seed and cap input
  size; no native module execution in fuzzers.
- **CI.** Activate `ui`, `conformance`, `execution`, `lean`, and `fuzz-smoke`; fix the execution
  detect flag; add a `projects` job (Windows + Ubuntu) that runs `stc new/build/check/run/test/
  clean` and the online workflow; keep the `test` and `online` jobs and both hosts. CI uses
  `--locked`, denies warnings, pins runner images, and prints clang versions.
- Tests: the corpus/UI suites themselves, traceability report, fuzz smoke, and the project e2e.

Acceptance: CI shows unit/snapshot, conformance, execution, projects, fuzz-smoke, and Lean proof
jobs passing; the traceability report covers every implemented rule.

### Phase 11 — Documentation and handover

- Update README status, CLI list, project-manifest example (kebab-case), repo layout, CI table,
  and decision summary.
- Update the project plan's M6 status note, immediate next actions, and deliverables list (add
  this plan); update the conformance profile status, implementation-defined choices (multi-task
  scheduling, assertions, project model, state/IO), and out-of-scope list.
- Update CLI-and-Manifest, State-and-RETAIN, I/O-and-Devices, Runtime-and-Execution,
  Online-Change-and-GOT, Testing-and-Verification, Architecture, and GOT Artifacts for the
  implemented behavior; remove stale "M6 pending" notes.
- Record M6 decisions D-153 onward and the phase records; add the M6 completion note to this
  plan; update the M6.5 handover boundary if the control/service-thread interface changed, and
  the host ABI minor if Q6-15 appends the test-clock service.
- Correct stale statements (for example, the conformance profile's "standard FBs implemented in
  ST under stdlib/" versus D-134, and README snake_case keys) and ensure no TODO/TBD placeholders.

Acceptance: `check_docs.py`, Markdown lint, and the link/profile checks pass; docs and code agree.

## 7. Determinism and artifact compatibility

- Package order: topological; ties sorted by package name then canonical path. File order:
  package order, then lexicographic normalized path.
- `stc.lock` is deterministic and uses forward slashes; no absolute paths; `stc clean` never
  deletes it.
- `GotId` scopes use the dependency's manifest name; aliases affect namespaces only. Mangled
  symbols/catalog keys include the package name.
- Source fingerprints include all production package sources (and test sources only in test
  builds), in deterministic order.
- Double builds of the same graph/baseline remain byte-identical for module, IR, metadata,
  manifest, plan, and build IDs. New metadata is canonical JSON.
- Host ABI stays major version 2; manifest/descriptor formats remain version 1. M6 bumps the
  minor to 2.2 when phase 6 emits test builds and appends only service ID 12
  (`set_cycle_time_ns(int64_t)`); no existing service ID/signature changes and no debug services
  are added. Production modules keep required minor 2.1. `stc_host_run` and its options struct
  are host exports (not module ABI) and are added in phase 4 for the launcher/embedding host.
- Task scheduling state is host memory keyed by task `GotId`, not a manifest field; task records
  already carry the schedule and guard identity.

## 8. Diagnostics

Existing families stay stable. The M6 additions below are confirmed in phase 0 and indexed in
[Conformance Profile](Specification/01-Conformance-Profile.md) §10; each keeps its owning
specification as the normative definition.

| Code | Meaning |
| --- | --- |
| `STC-E-PROJECT-001` | `stc.toml` cannot be read or parsed (I/O, TOML syntax, duplicate key) |
| `STC-E-PROJECT-002` | schema/type/unknown-key/invalid-value error with span and did-you-mean suggestion; snake_case spellings are unknown keys |
| `STC-E-PROJECT-003` | package name/version/language missing or invalid |
| `STC-E-PROJECT-004` | source glob/path invalid, outside the project, ambiguous, or matching nothing |
| `STC-E-PROJECT-005` | dependency path/alias/name invalid, missing `path`, or alias/name mismatch |
| `STC-E-PROJECT-006` | dependency cycle or duplicate canonical package path |
| `STC-E-PROJECT-007` | configuration selection missing or ambiguous |
| `STC-E-PROJECT-008` | `{stc:test}`/`[[test.case]]` conflict or invalid test metadata |
| `STC-E-PROJECT-009` | I/O device/channel/binding invalid |
| `STC-E-PROJECT-010` | build/run/retain/online/debug setting invalid (enum, duration, range, or combination) |
| `STC-E-PROJECT-011` | lint configuration invalid, unknown code, or tries to downgrade an error |
| `STC-E-PROJECT-012` | `stc.renames.toml` cannot be read/parsed or has an invalid schema |
| `STC-E-PROJECT-013` | rename history conflict, duplicate mapping, or cycle |
| `STC-E-LOCK-001` | lockfile missing under `--locked` |
| `STC-E-LOCK-002` | lockfile out of date (normal builds rewrite it; `--locked` fails) |
| `STC-E-LOCK-003` | lockfile malformed or unsupported schema |
| `STC-E-ASSERT-001` | production POU uses `STC_ASSERT` |
| `STC-E-ASSERT-002` | `STC_ASSERT` used outside a test-only unit or with an invalid form |
| `STC-E-TEST-001` | production POU references a test-only POU (existing code) |
| `STC-E-TEST-002` | duplicate test name or duplicate `[[test.case]]` entry |
| `STC-E-TEST-003` | invalid test signature: function inputs/return, FB/program shape, or `PASSED` type |
| `STC-E-TEST-004` | test-source conflict: same file in project and test globs, outside-project test source, or invalid test-only reference |
| `STC-E-FMT-001` | syntax errors prevent formatting |
| `STC-E-FMT-002` | file cannot be read or written |
| `STC-E-ONLINE-007` | control channel unavailable or unauthorized |
| `STC-E-ONLINE-008` | deploy rejected (incompatible candidate or prepare failure) |
| `STC-E-ONLINE-009` | active manifest unavailable for download/diff (missing or corrupt store) |
| `STC-E-RUN-003` | invalid `stc run` option (cycle count/time, I/O, state, stop condition) or runtime scheduling configuration (for example a task without a guard); codegen configuration errors stay `STC-E-RUN-001/002` |
| `STC-E-RUN-IO-001` | the file I/O backend cannot write its output image at shutdown; the previous file is left intact |
| `STC-W-IO-004` | the file I/O backend input image is missing or unreadable; the run starts with defaults |
| `STC-W-PROJECT-001` | setting parsed but not honored until a later milestone (for example `debug = false`) |
| `STC-W-PROJECT-002` | rename history has no baseline; the new key will receive a fresh `GotId` |
| `STC-W-ONLINE-007` | compatibility-report warning not covered by an existing online code |

`STC-W-ONLINE-002` remains the existing initializer warning; the compatibility-report generic
warning was moved to `STC-W-ONLINE-007` in phase 0 to keep codes unique. Existing
`STC-E-RUN-RETAIN-*`, `STC-E-ONLINE-STATE-*`, and `STC-W-ONLINE-STATE-*` codes are unchanged.

Rules: one definition/call-site error explains one unsupported setting; no per-type/per-function
diagnostic expansion. New compiler codes require a decision entry and a conformance/test mapping.
A code is never reused for a different condition; a retired code stays documented.

## 9. Verification

- `cargo test --workspace --locked` (including clang acceptance with `STC_REQUIRE_CLANG=1`).
- `cargo xtask test --suite ui --locked`, `--suite conformance`, `--suite execution`,
  `--suite projects`.
- `cargo xtask traceability --check`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo fmt --all --check`, `python tools/ci/check_docs.py`, Markdown lint.
- `lake build` and `lake exe vectors` (committed-output check) in `lean/`.
- `fuzz_manifest` plus the existing six targets for 60 s each with a 1 MiB cap.
- End-to-end `stc new/build/check/run/test/clean`, a multi-task project, a state restart, an I/O
  metadata project, a separate test project, an online watch swap, `online download/diff`, and a
  tracked rename on Windows x64 and Ubuntu x64.

## 10. Risks and mitigations

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Multi-package analyzer regressions | Wrong resolution or identity | One merged HIR with explicit package order; alias/name/INTERNAL tests; preserve the manifest-free bridge |
| Task scheduling ambiguity | Nondeterministic scans/timers | Host-owned state keyed by task `GotId`; one latched clock; documented priority/interval/no-catch-up rules; mock-clock tests |
| State-file corruption or loss | Failed starts or lost values | Existing magic/version/profile/layout hash + CRC; atomic replace; policy tests; `--reset-retain` escape hatch |
| I/O metadata drift | Wrong bindings/defaults | Compiler emits canonical metadata; precedence tests; runtime override; online image reconfiguration tests |
| Test runner hangs/flakes | CI instability | Per-test process isolation; default timeout; no silent retries; mock clock; sequential v1 |
| Formatter edge cases | Source damage or churn | CST-based rewrite, idempotence/comment/round-trip tests, `--check`, no rewriting unknown tokens |
| Watch/deploy control exposure | Unauthorized module load | Local-only transport, user-only ACL/socket permissions, per-run nonce, protocol versioning; separate from M6.5 debug auth |
| Rename history mistakes | Wrong `GotId` preservation | Explicit committed history, conflict errors, dry-run, chain tests, no heuristic matching |
| Lean/vector drift | Proofs disconnected from implementation | Committed deterministic vectors, CI regeneration check, Rust reference tests, C-harness samples |
| Traceability burden | CI red for documentation gaps | Automated scanning, corpus `rules` metadata, explicit not-implemented allowlist, phase-by-phase coverage |
| Scope creep into M6.5 debug | Unstable ABI/security surface | Keep ABI major 2 with only the 2.2 test-clock append, no debug info/hooks, separate control channel; M6.5 owns the debug protocol |

## 11. Definition of done

- All §4 exit criteria pass on Windows x64; the documented Linux x64 CI jobs pass.
- The project graph, package-aware analysis, configurations/tasks, state, I/O, test runner,
  formatter, and online workflow are implemented and tested.
- `stc new` produces a project that builds, runs, tests, and formats with the new CLI.
- The active-manifest workflow works end-to-end: build, deploy/reload online, review the report,
  download/rebuild, and cold recovery.
- `stc rename` preserves `GotId`s and values through a subsequent build and swap.
- Lean proofs and vectors, the conformance/UI corpus, traceability, and fuzz smoke are part of CI.
- README, project plan, conformance profile, architecture/runtime/state/I/O/CLI/testing/online
  specs, decisions, and this plan describe the implemented M6 surface truthfully.
- No M6.5 debug/monitoring capability is claimed; the M6.5 boundary is unchanged except for the
  control/service-thread interface recorded in the handover notes.

## 12. Commands

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all --check
python tools/ci/check_docs.py
npx --yes markdownlint-cli2@0.24.0 README.md "Specification/**/*.md" "*.md"

cargo xtask test --suite ui --locked
cargo xtask test --suite conformance --locked
cargo xtask test --suite execution --locked
cargo xtask test --suite projects --locked
cargo xtask traceability --check

cargo run -p stc-driver -- new examples/motor
cargo run -p stc-driver -- build
cargo run -p stc-driver -- run --cycles 10
cargo run -p stc-driver -- test
cargo run -p stc-driver -- fmt --check
cargo run -p stc-driver -- online download --out motor.stc-got.json
cargo run -p stc-driver -- online diff target/stc/debug/motor.stc-got.json
cargo run -p stc-driver -- rename Main.Speed Main.Rate
cargo run -p stc-driver -- run --watch

cd lean && lake build && lake exe vectors
```

## 13. Phase 0 decision questions (resolved 2026-10-26)

All questions below were resolved on 2026-10-26 and recorded as decisions D-153–D-163; the
resolution column is normative. The Q6-9 "emit `\n`" proposal was superseded in favor of the
phase-7 preserve-encoding rule.

| Question | Resolution |
| --- | --- |
| **Q6-1 — TOML parsing and diagnostics.** Use a span-aware TOML library, a typed model with serde, or a hand-written parser? | **Resolved (D-153):** `toml_edit` into an explicit model with span-mapped `STC-E-PROJECT-*` diagnostics and did-you-mean suggestions; `fuzz_manifest` seeds valid/invalid documents. |
| **Q6-2 — Package identity vs. alias.** Does a dependency alias change catalog keys/`GotId`s? | **Resolved (D-153):** identity is `[package].name`; the alias sets only the root namespace; catalog keys, `GotId`s, and symbols use the package name, and `INTERNAL` compares package identity. |
| **Q6-3 — Multi-package analysis.** Compile packages separately and link HIR, or merge into one HIR crate? | **Resolved (D-153):** one merged HIR crate with per-package root namespaces, globally unique `DefId`s, and one analyzer pass; deterministic topological package then lexicographic file order. |
| **Q6-4 — Task scheduling ownership and semantics.** Generated scheduler or host scheduler? Interval/priority/first-run/no-catch-up rules? | **Resolved (D-154):** host-owned scheduler sets `TaskGuard` roots per scan; the generated cycle runs guarded instances in scheduler rank order (§14.5). Lower number = higher priority; absent priority sorts last; no interval = every scan; interval = first run after one interval, then `now + interval`; `SINGLE` once; no catch-up; topology change cold. |
| **Q6-5 — Watch/deploy control transport.** Named pipe/socket, nonce/ACL, and relationship to M6.5 debug. | **Resolved (D-155):** local-only named pipe (Windows) / Unix socket (Linux) with user-only permissions and a per-run nonce; deployment/status only; separate from the M6.5 authenticated debug protocol. |
| **Q6-6 — Rename history.** Where is a source-aware rename's mapping persisted? | **Resolved (D-156):** committed `stc.renames.toml` at the root package; builds merge CLI `--rename` and history; conflicts error; chains resolve oldest-first; `--dry-run` available; identity preservation requires a baseline. |
| **Q6-7 — Test execution model.** One shared test module/registry or one artifact per test? | **Resolved (D-157):** one artifact per test in v1 (a synthetic wrapper for function/FB tests, the test program for program tests), one process per test, sequential runs; a shared registry is a later optimization. |
| **Q6-8 — Debug/release boundary.** Should M6 honor `debug = false`/`--release`? | **Resolved (D-158):** parse the keys but build the unoptimised debug form without debug info/hooks; `debug = false` reports `STC-W-PROJECT-001`; `--release` is a usage error until M6.5. |
| **Q6-9 — Formatter output.** Newline convention, blank-line handling, and semicolon insertion. | **Resolved (D-159):** preserve each file's line-ending encoding (CRLF stays CRLF, LF stays LF; inserted lines use the dominant ending; new files use LF) and BOM; preserve blank lines/comments/line breaks; insert strict semicolons after relaxed block statements; no wrapping or configuration. This supersedes the "emit `\n`" proposal. |
| **Q6-10 — File I/O backend.** When are input/output files read/written? | **Resolved (D-160):** input image read once at cold start (missing file = defaults plus warning); output image written atomically at shutdown; `--io-file` overrides; tests use memory; unbound inputs default each scan. |
| **Q6-11 — Lean/vector contract.** Model scope, format, and Rust linkage. | **Resolved (D-161):** Testing-and-Verification §5.2/§5.3 exactly; no Mathlib; committed deterministic vectors; a Rust `exact` reference module and C-harness samples; CI drift check. |
| **Q6-12 — Traceability mechanism.** How are rules mapped to tests? | **Resolved (D-162):** conformance `.meta.toml` `rules`, test annotations/comments with `STC-*` IDs, an explicit not-implemented allowlist, and `cargo xtask traceability --check`. |
| **Q6-13 — Host threading scope.** Does M6 prepare candidates off-thread? | **Resolved (D-155):** M6 adds a control/status thread and queues deploy work; validation/load/prepare runs between cycles on the cycle thread to avoid re-entering ST; off-thread prepare and jitter measurement remain M7. |
| **Q6-14 — State/I/O relative paths.** Project-relative paths at runtime. | **Resolved (D-163):** `stc run` resolves manifest-relative paths and CLI paths to absolute options; a standalone launcher resolves relative paths against the module directory; embedding hosts set their own paths. |
| **Q6-15 — Test-clock ABI.** How does generated test code advance the mock clock? | **Resolved (D-157):** append host ABI 2.2 service ID 12, `set_cycle_time_ns(int64_t)`, for the test prelude; it sets the next cycle's latched time; no other service and no debug services. |

## 14. Phase 0 interface contracts (2026-10-26, no code)

These sketches are the stable interfaces phases 1–10 consume. Normative details live in the
named specifications; this section records the cross-document decisions and the phase ordering.

### 14.1 Manifest spelling and schema

The canonical `stc.toml` spelling is kebab-case everywhere; snake_case is not an alias. The
keys named in the phase-0 checklist are fixed as follows:

| Table | Keys |
| --- | --- |
| top level | `schema` |
| `[package]` | `name`, `version`, `language` |
| `[dependencies]` | dependency alias entries with `path` and optional `package`; `version` is reserved |
| `[project]` | `sources`, `stdlib`, `configuration` |
| `[build]` | `opt-level`, `out-dir`, `lib-type`, `debug`, `syntax`, `emit`, `target`, `clang`, `runtime` |
| `[io]` | `backend`, `file`, `input-bytes`, `output-bytes`, `marker-bytes` |
| `[[io.devices]]` | `name`, `kind`, `config` |
| `[[io.devices.channels]]` | `name`, `direction`, `width` |
| `[[io.bindings]]` | `variable`, `device`, `channel`, `direction` |
| `[retain]` | `enabled`, `file`, `on-mismatch` |
| `[runtime]` | `module-dir`, `manifest-dir`, `max-retired-generations` |
| `[online]` | `baseline`, `manifest`, `cold-policy`, `allow-untracked-removal` |
| `[debug]` | `default`, `protocol`, `force-sessions-end-on-swap` |
| `[run]` | `cycles`, `cycle-time` |
| `[test]` | `sources`, `timeout`, `default-cycles`, `mock-clock`, `io` |
| `[[test.case]]` | `name`, `cycles`, `timeout`, `expect-trap` |
| `[lints]` | `warnings` and quoted `STC-*` codes |

`[io]` image sizes, `[test].default-cycles`, and `[[test.case]].expect-trap` are
part of the schema from phase 1 even where a later phase first exercises them.
[CLI and Manifest](Specification/Architecture/CLI-and-Manifest.md) §2 is normative; the README
example's snake_case keys are corrected in phase 11.

### 14.2 Diagnostics and JSON report shapes

The M6 code table in §8 is the authoritative addition list; the complete index is
[Conformance Profile](Specification/01-Conformance-Profile.md) §10. `STC-W-ONLINE-002` keeps its
initializer meaning, and the compatibility-report generic warning is `STC-W-ONLINE-007`.
`STC-E-PROJECT-012`/`013` cover rename-history parse/schema and conflict errors;
`STC-E-TEST-001` stays the existing production-reference code.

JSON shapes are fixed in
[CLI and Manifest](Specification/Architecture/CLI-and-Manifest.md) §1.6:

- diagnostics: one object per line (existing shape, unchanged);
- `stc test --message-format json`: one test-result object per line;
- compatibility reports: one object with `verdict`, `counts`, `warnings`, `rejections`, and the
  baseline/candidate build IDs; per-record `actions` only under `-v`;
- `stc metadata` and `stc compliance`: one summary object each.

### 14.3 M6/M6.5 ABI boundary

- The host ABI major stays 2. M6 keeps descriptor format 1 and manifest/binary/JSON format 1.
- M6 appends exactly one host service: ID 12, `void set_cycle_time_ns(int64_t ns)`, and bumps the
  host minor from 2.1 to 2.2 when phase 6 emits test builds. It sets the next cycle's latched
  time; it does not change the current scan. No existing service ID or signature changes.
- Only test builds import service 12. Production modules keep required minor 2.1 and run on a 2.1
  host; a test module requires 2.2 and is rejected by an older host.
- M6 emits no checkpoint hooks, no debug information, no debug services, and no service/monitor
  thread. The `stc run --watch` control channel supports deployment/status only and is not the
  M6.5 debug protocol (D-155/D-158).
- M6.5 appends debug services after ID 12 under the same major and owns `--release`, debug
  information, checkpoint hooks, typed access, forcing, breakpoints, stepping, and sessions.

### 14.4 Test-build mode (phase 6)

**Discovery and build.** `stc test` discovers `{stc:test}` POUs in the current package plus
`[test].sources` (Testing §2.1) and builds one test artifact per test case. A function test gets a
synthetic wrapper `PROGRAM` that calls the function once and asserts its `BOOL` result; an FB test
gets a wrapper that instantiates the FB and calls it once per scan (the host reads the optional
`PASSED : BOOL` leaf root because FB fields are `PROTECTED`); a program test uses the program
itself.

**Synthetic configuration.** Each test artifact is a normal ABI 2 module/manifest pair built with
a synthetic single-task configuration (configuration `STC_TEST`, resource `STC_TEST_RES`, task
`STC_TEST_TASK`, processor `STC_TEST_CPU`, instance `STC_TEST_INSTANCE`, wrapper
`STC_TEST_WRAPPER`). These names are compiler-reserved; a user declaration with one of them is
invalid test metadata. (The spellings use single embedded underscores because the lexer rejects
leading/trailing double underscores; the earlier `__STC_TEST__` sketch was not lexable.) The task
has no interval, priority, or `SINGLE`; the test host sets its guard before each cycle, exactly
as the scheduler would. The build is always cold-layout: it never reads the project's active
baseline, never writes the runtime's active manifest store, and never involves online change.
Test builds are not deployable and are not part of `[build].emit`.

**Runtime settings.** No retained state is loaded or saved; the I/O backend is `memory` unless
`[test].io` overrides it; the mock clock starts at `[test].mock-clock` (default `0ns`), and the
runner advances it cycle by cycle. The test host uses the existing assert service (ID 11), the
appended test-clock service (ID 12), and the existing I/O services; no other ABI addition is
needed. The driver writes a span-keyed assertion sidecar next to each test artifact so the host
can report the source `message`/`file`/`line`/`column` while the ABI assert service keeps its
fixed `(condition, code, span_id)` signature.

**Test host and result protocol.** `stc test` builds a test-only executable that links the
runtime test-host support, loads its module, runs the requested cycle count, and prints exactly
one structured result line to stdout:

```json
{"kind":"stc-test-result","name":"TimerBlink","status":"pass","assertions":[],"trap":null,"cycles":25}
```

`status` is `pass`, `fail`, or `trap`; `assertions` carries recorded failures with
`message`/`file`/`line`/`column`; `trap` is the trap name when the module trapped. The parent
adds `duration_ms`, maps a killed process to `timeout`, and emits the public report shape in
CLI §1.6. Test-body output is captured unless `-- --nocapture`, so the result line stays
parseable; a missing or malformed result line fails the test as an internal test-host error.
Each test runs in its own process; tests run sequentially in v1.

**Exclusion and errors.** Production builds parse test sources only for syntax errors and exclude
`{stc:test}` POUs before analysis/codegen. A production POU using `STC_ASSERT` is
`STC-E-ASSERT-001`; referencing a test-only POU is `STC-E-TEST-001`; invalid test signatures and
duplicates use `STC-E-TEST-002/003/004`. Test-only units never appear in production artifacts.

**Contract review.** GOT: a normal module/manifest with synthetic records; no format change.
State: no state file and no retained roots. I/O: memory backend, no metadata change. ABI: only
service ID 12. Determinism: artifact names derive from package and test name; the discovery order
is package order then test name.

### 14.5 Host task scheduler (phase 4)

**Compiler plan order.** The compiler projects every task with its `interval_ns`, `priority`,
`single`, and guard identity, and emits task-bound instances in scheduler rank order: explicit
priorities first in ascending numeric order, then tasks without a priority; ties by resource
declaration order, then task declaration order; within a task, program declaration order. The
plan also emits task-less instances, which run unguarded in declaration order before the
scheduled instances. The generated `stc_module_cycle` checks the task guard for every task-bound
instance; it does not initialize or clear guards. This makes the priority rule observable inside
one scan while keeping the host's job to due/skip only.

**Host state.** The host keeps `TaskState { got_id, completed, next_deadline_ns }` per manifest
task, keyed by task `GotId`. It resolves each task's guard from the manifest `guard_got_id` to
the `SlotKind::Data` slot with that `GotId` and writes the bound `TaskGuard` BOOL root (`0`/`1`)
before every scan; no manifest or GOT format change is needed.

**Due rule.** At the scan boundary the launcher/host loop latches the scan time, and the
scheduler reads it as `now = cycle_now_ns` (the same value the module will see). A task with no
interval is due every scan. An interval task is due when `now >= next_deadline_ns`; its first
deadline is `start_time + interval`, and after a due scan the deadline becomes `now + interval`
(no catch-up).
A `SINGLE` task runs on its first due scan and is then complete; completion is checked before the
next scan. A task that is not due has its guard cleared.

**Scan sequence.** Latch inputs, set every task guard, call `stc_module_cycle` once, publish
outputs once. All due tasks therefore share one latched clock, one input latch, and one output
publish. The host marks due `SINGLE` tasks complete only after the cycle returns normally.

**Swap and cold.** `TaskState` survives a compatible online swap by `GotId`; the existing
migration already rejects a changed task record (resource, name, `single`, `interval_ns`,
`priority`) as `STC-E-ONLINE-004`, so the surviving state always matches the active schedule. A
cold install or cold download starts with no completion and deadlines computed from the first
scan's time.

**State and I/O review.** Task state is host memory, not a manifest field and not part of the
state file; guard roots are `TaskGuard` roots and are never retained. The scheduler runs between
the input latch and output publish, so it does not change the existing state or I/O contracts.
`stc run -v` and the control channel expose active/pending/retired build IDs, cycle count,
per-task due/completed/deadline status, and the last rejection.

### 14.6 `stc.renames.toml` (phase 8)

The history is a committed TOML file at the root package directory:

```toml
schema = 1

[[rename]]
old = "Main.Speed"
new = "Main.Rate"
```

- `schema` is required and currently `1`; unknown keys are errors (`STC-E-PROJECT-012`).
- Each `[[rename]]` entry has non-empty `old` and `new`, exactly as they would be passed to
  `--rename`; entries are ordered oldest first and the file is rewritten in that order.
- A build merges history entries (oldest first) with CLI `--rename` entries (in command order)
  and resolves chains by composition: `A -> B` followed by `B -> C` passes `A -> C` to the
  identity engine. Duplicate `old` values with different `new` values, or a chain that revisits a
  key, are `STC-E-PROJECT-013`.
- `stc rename` rewrites only sema-resolved identifier tokens (declaration and references), leaves
  comments and strings untouched, preserves formatting and file encoding, and appends the entry
  only after a successful rewrite. `--dry-run` prints edits without writing source or history.
- The history changes `GotId` matching only when a baseline is present; a cold build emits
  `STC-W-PROJECT-002` and gives the new key a fresh `GotId`. Renaming a dependency's declaration,
  a package name, or a key outside the current package is out of scope.
- Contract review: the history feeds the existing `stc-got` rename maps; it does not change the
  manifest format, the state file, or the module ABI.

### 14.7 Cross-contract review

- **GOT/online:** no descriptor, manifest, slot, root, or task-record format change; M6 only
  consumes existing records. Test builds are cold and never touch the manifest store.
- **State:** the state-file format is unchanged; scheduler and test state are host memory, and
  test builds disable retention. The D-163 path rules make the state file absolute before the
  launcher starts.
- **I/O:** no metadata-shape change; the file backend follows D-160 (cold read, atomic shutdown
  write), and test builds use memory unless `[test].io` overrides it.
- **ABI:** only service ID 12 is appended; production modules remain 2.1-compatible, test modules
  require 2.2, and M6.5 debug services start after 12.
- **Determinism:** scheduler rank, synthetic test names, rename-history order, and the JSON report
  shapes are deterministic inputs, so double builds remain byte-identical.

## 15. Post-M6 limitations

- Full debug/monitoring (DWARF/CodeView, checkpoint hooks, typed access, forcing, breakpoints,
  stepping, call stacks, debug sessions/protocol) remains M6.5.
- `--release`, stripped artifacts, and debug/release build-ID separation remain M6.5.
- No field-layout migration, resizing, `GotId`-keyed state migration, or record/handle migration.
- No network deployment, registry/version solving, package binary dependencies, or hardware I/O;
  v1 uses local path dependencies and `null`/`memory`/`file` backends.
- Multi-task execution is cooperative on one cycle thread; preemptive/multi-threaded ST is out of
  scope. Off-thread candidate preparation and bounded maintenance jitter are M7 hardening.
- Standard FB bodies remain synthesized HIR rather than a textual `stdlib/` package (D-134).
- `stc fmt` is the minimal fixed style; no configuration, range formatting, or wrapping.
- `stc test` runs sequentially and builds one artifact per test; a shared registry and parallel
  execution are future work.
- Host portability beyond the CI-tested Windows x64 and Ubuntu x64 matrix is M8.
