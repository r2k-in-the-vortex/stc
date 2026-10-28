# stc Project Plan

Status: M4.5 online-change foundation, M5 lowering (phases 1–7 and 4b), M6 project system,
CLI, and verification hardening (phases 0–11), and M6.5 debug/monitoring (phases 0–10)
implemented (2026-10-28); M7 release candidate is next  
Date: 2026-10-24  
Profile: `stc-2013-oop-utf8-v1`  
Standard reference: IEC 61131-3:2013 (Ed. 3.0), local copy under `Specification/Reference/`

## 1. Purpose

`stc` is a command-line compiler for IEC 61131-3 Structured Text. It will:

- parse and semantically analyse ST source written against the IEC 61131-3:2013 language,
- include the 2013 object-oriented features (classes, interfaces, inheritance, methods),
  namespaces, references, and configuration/task declarations,
- add a project-defined UTF-8 string extension (the 4th edition is not available and will not
  be purchased),
- emit textual LLVM IR,
- invoke `clang` to produce objects, loadable modules, executables, or libraries, and
- support PLC **online change**: replacing the executing module between scan cycles while the
  host and persistent memory survive.

The compiler is written in Rust. Specifications are written before compiler code, as required by
the project README. Mathematical functions with exact discrete semantics are specified and
proved in Lean before implementation. Online-change, GOT, module, manifest, and debug behaviour
is specified in
[Online Change and GOT](Architecture/Online-Change-and-GOT.md) and
[Debug and Monitoring](Architecture/Debug-and-Monitoring.md).

## 2. Goals

1. A correct, well-diagnosed ST front end with a lossless syntax tree.
2. A typed middle end that resolves names, overloads, inheritance, interfaces, and conversions
   according to a written profile.
3. Deterministic textual LLVM IR with a documented ABI and name-mangling scheme.
4. A small Rust runtime with a stable C ABI, usable by both generated executables and embedding
   hosts.
5. Both artifact modes:
   - executable: a thin host launcher that loads an adjacent module/manifest pair;
   - library: no `main`; an embedding host that exports the `stc_host_*`/`stc_rt_execute`
     boundary, with the generated `stc_module_*` code kept in the separate module.
6. A Cargo-like CLI with human and JSON diagnostics, verbose phase output, and a built-in test
   runner.
7. Lean models and proofs for exact functions (integer, bitwise, conversion, selection, and
   related discrete operations), plus generated test vectors consumed by Rust tests.
8. A project/dependency model: packages, local path dependencies, a dependency graph, and
   package-aware namespace visibility, enough for separate test projects.
9. A conformance corpus and traceability from specification rules to tests.
10. PLC online change in v1: a persistent host runtime, loadable whole-program modules, a
    project-defined global offset table (GOT), host-owned persistent memory, and an
    O(1) cycle-boundary activation.
11. A persistent GOT manifest emitted with every module, consumed by the next build as the
    identity baseline, retained by the runtime, and downloadable as a build baseline for
    another developer.
12. Typed variable access, forcing, watch, breakpoints, stepping, and call-stack inspection in
    v1, with debug information as the default build output and `--release` as the optional
    stripped/optimised mode.
13. Explicit non-safety positioning: `stc` and its runtime are not a safety PLC and are not
    responsible for safety in any way.

## 3. Non-goals for v1

- IEC 61131-3:2025 (Ed. 4.0) conformance. The project targets the 2013 language plus documented
  extensions and makes no Ed. 4 claim.
- LD, FBD, SFC, and IL as input languages. Only ST is parsed.
- The IEC communication model and HMI. Online change, monitoring, and debugging are project
  runtime capabilities in v1, not IEC conformance features.
- A safety claim. `stc` is not a safety PLC and is not responsible for safety in any way; debug
  sessions may hold outputs indefinitely.
- Field-layout migration in v1. Per-field add/remove/rename is supported; type changes,
  resizing, and array/string/`OVERLAP` layout changes reset the affected root or require a cold
  download.
- Remote/network deployment, multi-user scheduling, and a package registry. V1 online change is
  a local host API/CLI capability.
- Hardware I/O backends (EtherCAT, Modbus, MQTT) and their schedules in v1. They are planned for
  v2 and are noted in the I/O spec; v1 uses null/memory/file backends.
- Vendor project import (TIA Portal, CODESYS, etc.).
- The SIMATIC AX preprocessor and vendor-specific pragma semantics. AX documentation is used
  only as syntax inspiration.
- A network package registry and version-solving resolver in v1. Local path dependencies are in
  v1; a registry/version solver is a later extension (A-009).
- Formal verification of IEEE-754 transcendental functions. These remain test-based.
- Multi-threaded ST task execution in v1 (architecture must not preclude it). The host runtime
  has a cycle thread, an M6 control/status thread, and M6.5 service/monitor threads; fully
  off-thread loader/prepare is M7. Only the cycle thread executes ST code.
- `PERSISTENT` (not a 2013 keyword) and periodic/checkpointed state saves. v1 persists retained
  variables at normal shutdown; crash-time saves are a later extension.
- Native Linux host support during initial development. Linux x86-64 and Linux AArch64 host
  builds are the first planned post-basic-development deliverable (M8, D-042).

## 4. Constraints and assumptions

| Item | Value |
| --- | --- |
| Compiler language | Rust stable (workspace; toolchain pinned in `rust-toolchain.toml`) |
| IR | Textual LLVM IR (`.ll`); no LLVM FFI dependency required |
| Backend driver | `clang` (and its linker) lowers IR to the selected target |
| Dev toolchain | clang/LLVM 23.1.4, cargo 1.99.1, Lean 4.35.0 (Lake 5.0.0) |
| Primary host | Windows 11, `x86_64-pc-windows-msvc` (development host) |
| Additional hosts | After basic development: Linux x86-64 and Linux AArch64 (ARM64), native builds and execution (D-042) |
| Target selection | Host by default; `--target <triple>` passed to `clang` |
| Source encoding | UTF-8 (optional BOM), case-insensitive language keywords/identifiers |
| Project dependencies | Local path dependencies plus committed `stc.lock` in v1; package graph is acyclic; `--locked` enforced; registry/version solver later (A-009) |
| Standard FBs / time | Standard FB bodies are synthesized HIR in `STC_STD` (D-134); the host clock is latched once at `stc_module_cycle` entry and shared by all timers in that scan (A-010) |
| I/O | Runtime-owned linking; compiler emits process-image/device/binding metadata; `{stc:io}` JSON hints; v1 null/memory/file; EtherCAT/Modbus/MQTT v2 (A-011) |
| Retained state | File-backed, versioned, layout-hashed, CRC-checked, atomic save; warm/cold restart; v1 saves at shutdown (A-012) |
| Standard text | Local IEC 61131-3:2013 PDF only; not committed or redistributed |
| UTF-8 strings | Project-defined extension; `USTRING[n]` stores `n` characters as 32-bit scalars (4 bytes each), static storage, UTF-8 source/interchange |
| Lean scope | Proofs for exact discrete functions cover the Lean model; Rust is tested against generated vectors (A-006); float/transcendental math is test-based |
| Execution model | Persistent host runtime plus a loadable whole-program module (DLL/so); no per-function patching, no code-linked deployment path for online change |
| Persistent memory | Host-owned GOT roots with stable storage; per-field roots and handle-based objects; generated code accesses cells through the GOT |
| Online change | Whole-module load/prepare, O(1) activation at a cycle boundary, one-successful-cycle rollback window, and off-thread unload; M6 validates/loads/prepares between cycles on the cycle thread, and fully off-thread prepare is M7 |
| Identity baseline | Persistent GOT manifest (canonical JSON + compact binary) paired with every module; missing/stale/invalid baseline forces a cold download |
| Runtime ABI | Host ABI 2; M4 code-linked artifacts are not loadable; module descriptor + manifest are versioned together |
| Debug and monitoring | Full typed debug in v1; debug information is the default build output; `--release` is the optional stripped/optimised mode; swaps end debug sessions |
| Safety | Not a safety PLC and not responsible for safety in any way; no automatic fail-safe; debug may hold outputs indefinitely |

Assumption: the compiler host runs a single compilation job at a time. The generated runtime is
single-threaded for ST execution; M6 adds a control/status thread and M6.5 adds service/monitor
threads, none of which execute ST or block the cycle. Fully off-thread candidate preparation is
M7 work.

Host-specific code is confined to the driver and runtime. The front end, HIR, and IR builder are
portable Rust with no host conditionals, so the planned Linux x86-64/AArch64 port is additive
(D-042).

## 5. Deliverables

### 5.1 Specification

- `Specification/01-Conformance-Profile.md` — exactly what is implemented and claimed.
- `Specification/Syntax/Tokenization.md` — source model, tokens, literals, trivia, errors.
- `Specification/Syntax/Grammar.md` — EBNF grammar for the supported ST surface.
- `Specification/AST/AST.md` — CST, AST, HIR, symbols, spans, diagnostics.
- `Specification/Architecture/Architecture.md` — pipeline, crates, IR generation, ABI.
- `Specification/Architecture/Runtime-and-Execution.md` — runtime, artifact modes, tasks,
  standard functions/FBs, trap model.
- `Specification/Architecture/IO-and-Devices.md` — process image, `AT` linking, device tree,
  pragma hints, runtime override, unbound defaults.
- `Specification/Architecture/State-and-Retain.md` — file-backed retained state, warm/cold
  restart, layout hashing, atomic saves, ABI.
- `Specification/Architecture/CLI-and-Manifest.md` — CLI verbs, diagnostics, `stc.toml`.
- `Specification/Architecture/Testing-and-Verification.md` — test framework, conformance,
  Lean vector bridge, fuzzing, CI.
- `Specification/Architecture/Online-Change-and-GOT.md` — host/module model, GOT, root
  allocation, module descriptor, manifest, identity/history, load/prepare/activate/rollback/
  unload, compatibility, cold download.
- `Specification/Architecture/GOT-Artifacts.md` — versioned stable keys, `GotId`, catalog,
  module descriptor, host ABI 2.0, GOT manifest, online-change plan, and build-ID formats.
- `Specification/Architecture/Debug-and-Monitoring.md` — typed access, watch, forcing,
  breakpoints, stepping, call stack, debug protocol/security, DWARF/CodeView, swap interaction.
- `Specification/Decisions.md` — decision log and open questions.
- `M4.5-Plan.md` — online-change foundation implementation plan.
- `M5-Plan.md` — FBs, OOP, aggregates, and UTF-8 strings implementation plan.
- `M6-Plan.md` — project system, CLI, and verification hardening implementation plan.
- `M6.5-Plan.md` — debug/monitoring implementation plan.
- This plan.

### 5.2 Implementation

- Rust workspace under `crates/`.
- `crates/stc-runtime` — persistent host runtime with a C ABI: cycle loop, scheduler, GOT, root
  allocator, module loader, manifest store, swap controller, control/status thread, state, and
  I/O; debug services are implemented in M6.5. Also the `staticlib` (with an `rlib` for Rust
  integration tests) used by embedding hosts.
- `crates/stc-manifest` — `stc.toml` schema, source discovery, local dependency graph, and
  `stc.lock`.
- `crates/stc-test` — `{stc:test}` discovery, wrapper generation, and the process-isolated
  runner; `crates/stc-fmt` — the fixed-style formatter over the lossless CST.
- Loadable `module` artifacts (`.dll`/`.so`) generated from ST, each paired with a GOT
  manifest; `exe` is a thin host launcher for an adjacent module; `lib` is the embedding host
  library, static or shared.
- Standard library — the 2013 standard functions lower inline or through runtime helpers, and
  the standard FB bodies are synthesized HIR in the reserved `STC_STD` scope (D-134); there is
  no textual `stdlib/` package.
- `lean/` — Lake project for exact-function models, proofs, and vector generation.
- `tests/` — conformance corpus, UI snapshots, committed Lean vectors, and cross-target fixtures;
  crate integration tests cover the compiler, runtime, and end-to-end project workflow.
- `examples/` — small ST projects.
- `crates/xtask` — CI suite selection (`cargo xtask test --suite ...`) and rule traceability
  (`cargo xtask traceability`); vector regeneration is `lake exe vectors` in `lean/`.

### 5.3 Documentation

- Updated top-level `README.md` with the confirmed profile, decisions, and question log.
- The CLI's `--help` output and the architecture specifications are the current user guide; a
  dedicated user guide and expanded examples are post-v1 documentation work.

## 6. Architecture at a glance

```text
ST source (UTF-8)
   |
   v
lexer -> lossless CST -> typed AST -> name resolution -> type checking / HIR
   |                                                              |
   +-- diagnostics <----------------------------------------------+
                                                                  |
                                                                  v
                                        textual LLVM IR builder + GOT/manifest model
                                                                  |
                                        clang (assemble/optimize/link module)
                                                                  |
                                 module (.dll/.so) + manifest (.json/.bin)
                                                                  |
                        +-----------------------------------------+
                        |  persistent host runtime (stc-runtime)   |
                        |  cycle loop | GOT cells | root allocator |
                        |  loader/prepare | swap controller        |
                        |  manifest store | state/IO | debug       |
                        +-----------------------------------------+
                                                                  |
                                   outputs, retained state, debug/monitoring services
```

Key choices:

- Hand-written lexer and recursive-descent parser with a Pratt-style expression parser.
- Lossless CST (rowan-style) plus a typed AST/HIR; every node carries a byte span.
- Identifier interning with case-insensitive lookup and original-spelling preservation.
- A typed HIR after resolution; codegen never walks the raw AST.
- Textual IR is emitted deterministically by a small typed IR builder; `clang` validates and
  lowers it. This avoids binding the project to a specific LLVM C++/C API version.
- Runtime ABI is a narrow, versioned C ABI; generated modules call host services through a
  versioned function table and access persistent state through GOT cells.
- Persistent roots are per-field recursively; objects use stable handles plus runtime-owned
  descriptors; interfaces and object references are `{handle, descriptor}`. `REF_TO` values use
  place descriptors, and `POINTER TO` is a distinct transient-only extension (`STC-POINTER`).
- `VAR_IN_OUT` and `REF_TO` lower through host-resolvable descriptors; value parameters are
  passed according to the C ABI for the target.
- Executable mode generates a thin host launcher that loads an adjacent module; library mode
  embeds the same host. The module exports attach/prepare/cycle/shutdown/detach and a descriptor.
- Every module is paired with a persistent GOT manifest that carries identity, layout, and
  lineage; the next build consumes the active manifest as its baseline.
- `stc-manifest` resolves local path dependencies into an acyclic package graph, source files are
  compiled in topological order, and package boundaries participate in namespace/visibility
  checks. A separate test project is just another package that depends on the code under test.

## 7. Specification process

1. Every normative rule gets a stable ID in the form `STC-<AREA>-<NNN>` (for example
   `STC-LEX-014`, `STC-TYPE-031`, `STC-RUN-006`).
2. Rule IDs are referenced from implementation code comments and from tests.
3. A traceability table maps each rule to at least one test, a conformance case, or a marked
   "not implemented" state.
4. Implementation-defined choices are recorded in the conformance profile and the decision log;
   they are never left implicit.
5. Ambiguities in the 2013 text are resolved explicitly, with the chosen interpretation and a
   test. The standard's Annex A and prose are both consulted where they disagree.
6. Unsupported syntax or semantics produce an explicit diagnostic. Silent acceptance is a bug.

## 8. Milestones

### M0 — Specification baseline and repository scaffolding

Deliverables: this plan, conformance profile, tokenization, grammar, AST, architecture, runtime,
CLI/manifest, testing/verification, decision log; README updated; toolchain pins; CI skeleton.

Exit criteria:

- Documents reviewed and no unresolved blocking questions for M1–M2.
- Terminology and profile are consistent across documents.
- The repository has no stray placeholders and the reference PDF is excluded from version
  control.

### M1 — Lexer

Deliverables: complete token set for the profile; source map and spans; comments/pragmas/literals;
UTF-8 string literals; diagnostics; fuzz target; lexer snapshots and property tests.

Exit criteria:

- Every lexical rule has a rule ID and tests.
- Valid/invalid UTF-8, CRLF/LF, BOM, nested comments, based/typed/time/date literals, and
  string escapes are covered.
- Fuzzing finds no panic or hang on arbitrary bytes.

### M2 — Parser and CST/AST

Deliverables: full ST grammar (declarations, statements, expressions, OOP, namespaces,
configurations); error recovery; CST and typed AST; `stc parse`/`stc check` skeleton; parser
snapshots and negative tests.

Exit criteria:

- All grammar productions have positive and negative tests.
- Error recovery produces multiple useful diagnostics and never panics.
- CST can round-trip source text exactly (including trivia).

### M3 — Name resolution and type checking

Deliverables: symbol tables, scopes, namespaces/USING, class/interface/FB inheritance, method
resolution, overloads, implicit/explicit conversions, constant expressions, initializers, access
control, reference/interface rules, assignment attempt, diagnostic codes; `stc check`.

Exit criteria:

- Positive and negative conformance cases for every rule category.
- No codegen yet; diagnostics are stable and snapshot-tested.
- Type errors include actionable spans and notes.

### M4 — LLVM IR core codegen

Deliverables: typed HIR -> textual IR; scalar types and operators; control flow; functions and
programs; globals and initialization; process-image metadata for `AT`; runtime ABI stubs; clang
invocation; `--emit=ir,obj,exe`; execution tests; deterministic IR snapshots.

Exit criteria:

- A set of small ST programs produces IR accepted by `clang` and runs correctly.
- Both executable and library mode link and execute a no-op cycle.
- IR is deterministic and free of target-specific offsets.

> **M4 status (2026-10-24):** implemented. `stc-codegen` lowers the elementary scalar HIR
> subset to deterministic textual IR; `stc-runtime` supplies the C ABI, cycle loop, process
> image, clock, traps, and retained-state stubs; `stc build` emits `ir`, `obj`, `exe`, static
> `lib`, and `metadata` with clang/runtime discovery on Windows MSVC. Codegen, clang
> verification, execution, and C-harness tests pass, including library mode, process-image
> access, bindings, casts, short-circuit logic, and scalar retained-state round-trips. M5
> constructs (FBs, OOP, aggregates, strings, references, standard functions) produce explicit
> `STC-E-CODEGEN-001` diagnostics. File-backed retained state, the manifest, `stc run`,
> `stc test`, and the cooperative multi-task scheduler were delivered in M6. M5 kept one task
> per resource.

### M4.5 — Online-change foundation

Deliverables: host/module artifact model; module descriptor and host ABI 2; GOT with shared
host-owned cells and per-module slot tables; host root allocator; per-field scalar roots;
per-generation direct-variable cells; module attach/prepare/cycle/shutdown/detach; background
loader/prepare thread; normal/temp GOT double buffering; O(1) cycle-boundary activation;
one-successful-cycle rollback; off-thread unload; immutable module files, manifest sidecar, and
current pointer; persistent GOT manifest in canonical JSON and compact binary; exact-parent
baseline rules; tracked rename/move maps; preserve/allocate/drop; compatibility classification
and diagnostics; the cold-download path; CLI `--emit=module`, `--got-manifest`, `--cold`, and
`--rename`; a thin `exe` launcher and an embedding `lib` host with generated code in the separate
module (the two emit kinds may be combined); and the host API/export path for manifest
download/diff. The user-facing `stc online download/diff` commands were delivered in M6.

Exit criteria:

- Hot-swap suite passes: code-only swap, field add/remove/rename, tracked move, incompatible
  change rejection, missing/stale manifest to cold download, load/prepare failure rollback, and
  unload after grace.
- `--emit=exe` produces a thin launcher for an adjacent module pair; `--emit=lib` produces the
  embedding host library with the generated module kept separate; the two emit kinds may be
  combined.
- Double builds produce byte-identical modules and manifests; the runtime exports the active
  manifest and a second build consumes it as an exact baseline.
- Generated code contains no persistent raw pointers; every persistent root/function call goes
  through the GOT.
- The cycle-boundary switch is O(1), heavy work is off-thread, and maintenance jitter is bounded
  and measured.
- M4 scalar semantics are unchanged; M4 code-linked artifacts are explicitly non-loadable.

> **M4.5 status (2026-10-25):** phases 0–10 are complete. Phase 1
> fixes the versioned artifact contracts in [GOT Artifacts and Schemas](Architecture/GOT-Artifacts.md):
> stable keys, `GotId`, catalog, module descriptor, host ABI 2.0, manifest/plan formats, build IDs,
> and pairing. Phase 2 implements the identity/manifest layer in `stc-got`; decisions
> D-113–D-118 record the choices and phase-2 clarifications. Phase 3 adds the module IR builder
> and structural verifier in `stc-codegen`; D-119 records the IR-level choices. Phase 4 adds the
> descriptor codec and pairing validation plus the `stc-codegen::module`
> catalog/slot/descriptor/manifest/plan projection; D-120–D-121 record the choices. Phase 5 adds
> the host GOT, roots, quarantine, transient allocation, and artifact validation (D-122); phase 6
> adds the loader, ABI 2 services, manifest store, and swap controller (D-123). Phase 7 converts
> generated execution to the module model: persistent state, direct variables, retain leaves, and
> task guards use GOT cells; calls are indirect through function cells; the launcher and
> embedding host load compiler-produced module pairs; and a non-local trap boundary enables
> rollback-window recovery (D-124). Existing M4 execution tests run through modules. Phase 8
> completes the scalar compatibility policy: activation reconfigures the process image and
> remaps direct cells, values are preserved by `GotId`, moved variables and dropped roots are
> reported as runtime warnings, rollback restores the image with the generation, cold installs
> reinitialize roots and the image, incompatible layout/type changes require a cold rebuild, and
> real compiler-produced modules cover hot-swap, rename, reject/cold, and direct-remap paths
> (D-125). Phase 9 adds artifact/metadata fuzz targets, end-to-end byte-identical double builds
> with deterministic Windows PE timestamps, a combined launcher/embedding artifact test,
> malformed-baseline and metadata negatives, concurrent generation-pin coverage, a recorded
> O(1) switch measurement, and an explicit Windows/Linux `online` CI job (D-126). Phase 10
> completes the documentation and handover pass: the specification set, README, decisions, and
> `handover_to_M5.md` describe the implemented host/module model, and
> [Debug and Monitoring](Architecture/Debug-and-Monitoring.md) §12 records the M4.5 interface
> that M6.5 extends (D-127). M5 is implemented.

### M5 — FBs, OOP, aggregates, and UTF-8 strings

Status: Complete (2026-10-25). Phases 1–7, the `STC-POINTER` surface, and phase 4b
(interfaces, runtime descriptors, `?=`, object references, and their verification) are
implemented; the exit criteria below pass in the codegen, execution, online-change, and
clang-required suites.

Deliverables: FB instances/state, methods, inheritance, interfaces/vtables, `THIS`/`SUPER`,
references, `VAR_IN_OUT`; static arrays, conformant `ARRAY [*]` parameters, structs, enums,
subranges; `STRING`/`WSTRING`/`USTRING`; standard functions; standard FB stdlib; cycle-latched
clock; single-task cyclic runtime; test runner MVP. All persistent aggregates follow the
per-field/handle model: runtime-owned descriptors, vtable cells, place descriptors for `REF_TO`,
nullable interfaces, and the transient-only `STC-POINTER` extension.

Exit criteria:

- OOP conformance cases (inheritance, overriding, interface dispatch, assignment attempt) pass.
- Static arrays and conformant `ARRAY [*]` parameters pass bounds/aliasing tests; dynamic
  locals/globals/returns are rejected with explicit diagnostics.
- UTF-8 vectors (valid/invalid sequences, character counts, UTF-8 byte lengths, 4-byte scalar
  indexing, slicing, emoji/supplementary scalars) pass.
- Standard timer/counter/edge/bistable FBs pass deterministic tests with a mock clock; all timers
  in one scan see the same latched cycle time.
- Online-change tests cover FB/class state, aggregate roots, interfaces, and preserved
  descriptors; `POINTER TO` tests cover arithmetic, indexing, `SIZEOF`, null traps, and the
  rejection of persistent pointer fields.

### M6 — Project system, CLI, and verification hardening

Deliverables: full `stc.toml`, local path dependency graph, `stc.lock`, package-aware namespace
visibility, configurations/tasks, I/O device tree/bindings/defaults metadata, file-backed
retained state, build artifacts, JSON diagnostics, verbose output, `stc test`, minimal `stc fmt`,
Lean vector integration, conformance corpus, fuzzing, CI matrix, docs. Online-change workflow:
`stc run --watch`, `stc online download/diff`, source-aware `stc rename`, compatibility reports,
manifest/baseline CLI and host API, and the state-file policy for online attach/cold start.

Exit criteria:

- `stc new/build/check/run/test/clean` work end-to-end on Windows.
- A user can build against the active manifest, deploy/reload online, review the compatibility
  report, download the active manifest and build a compatible change from it, and recover from a
  missing/stale baseline via a documented cold download.
- CI runs unit, snapshot, conformance, execution, fuzz-smoke, and Lean proof jobs.
- Traceability report shows every implemented rule has a test.

> **M6 status (2026-10-27):** phases 0–11 are complete. Phase 0 fixed the kebab-case `stc.toml`
> schema, diagnostic/report shapes, the host ABI 2.2 test-clock append, and the M6/M6.5 boundary
> (D-153–D-164). Phase 1 added `crates/stc-manifest` with schema, source discovery, the path
> dependency graph, `stc.lock`, and `--locked`; phase 2 added package-aware namespaces,
> `INTERNAL` visibility, and merged multi-package analysis; phase 3 added the project CLI
> (`new`/`check`/`build`/`clean`/`metadata`/`compliance`), build settings, artifacts, and
> human/short/JSON diagnostics. Phase 4 added cooperative multi-task scheduling, `stc run`, the
> rewritten launcher, mock clocks, and `stc_host_run` (D-165). Phase 5 added the `STCRET01`
> state file, mismatch policies, atomic saves, device/binding/`VAR_ACCESS` metadata, and the
> `null`/`memory`/`file` backends (D-166). Phase 6 added `crates/stc-test`, test-build mode,
> the test prelude, ABI 2.2 service 12, and the process-isolated runner (D-167). Phase 7 added
> `crates/stc-fmt` and `stc fmt` (D-168). Phase 8 added the online workflow: active-baseline
> builds, compatibility reports, `online download/diff`, source-aware `rename` with
> `stc.renames.toml`, and `run --watch` with the local control channel (D-169). Phase 9 added
> the Lake exact models/proofs, the deterministic vector generator, 30 committed vector files,
> the Rust reference harness, and the C-harness samples (D-170). Phase 10 added the conformance
> corpus, UI snapshots, `xtask`, traceability, `fuzz_manifest`, and the active CI matrix
> (D-171). Phase 11 completed the documentation and handover pass (D-172). All M6 exit criteria
> pass on Windows x64 and the documented Ubuntu x64 CI jobs.

### M6.5 — Full debug and monitoring

Deliverables: typed read/write/watch of persistent and local variables, forcing applied at cycle
boundaries, codegen breakpoint checkpoints, stop-the-world stepping, call-stack unwinding,
DWARF/CodeView debug information with sidecar symbols for release builds, a local debug protocol
with authentication/audit, the manifest-assisted split-object view, and debug/release build-ID
and manifest separation.

Exit criteria:

- Read, write, force, watch, breakpoint, step, and call-stack tests pass on both hosts.
- Debug information resolves variables and source locations in optimised and unoptimised builds.
- Starting a debug session pauses the watchdog and holds outputs; stopping or disconnecting
  clears forces/breakpoints and resumes normal execution.
- Every online swap ends the debug session cleanly, then unloads the old generation.
- Debug hooks are absent from `--release` artifacts; measured cycle overhead with debug disabled
  is documented.

> **M6.5 status (2026-10-28):** phases 0–10 are complete and all exit criteria pass on Windows
> x64 and the documented Ubuntu x64 CI jobs. Phase 0 fixed the protocol, session, access,
> checkpoint, and build contracts (D-173–D-179, `M6.5-Plan.md`); phases 1–8 implemented
> DWARF/CodeView, the checkpoint table and host service 13 at ABI 2.3, typed read/write/force/
> watch, the debug session protocol with audit and the stop/idle bound, breakpoints/stepping/
> call stacks with the stopped-frame sidecar, the manifest-assisted split view, and the
> `stc debug`/`stc run --debug-server` CLI with swap teardown. Phase 4 raised only the GOT
> manifest to format 2 (D-180); descriptor, module, state, metadata-sidecar, frame-sidecar, and
> host-ABI formats are otherwise unchanged from M6. Phase 9 added the Windows/Linux `debug` CI
> job, the twelve-target debug suite (115 tests), the informational overhead measurement, and
> the exit-criteria sweep; phase 10 closed the documentation and M7 handover (D-183). The exit
> criterion that names a watchdog is satisfied by the documented stop/idle budget: M6 has no
> watchdog implementation, and the M6.5 session bound (`[debug].session-timeout`, default five
> minutes) ends the session and resumes the cycle with a fresh deadline. M7 (release candidate)
> is next.

### M7 — Release candidate

M6.5 is complete; M7 is the next milestone.

Deliverables: performance pass, cross-target smoke tests via `clang --target`, packaging,
examples, release notes, known-limitations list, manifest/online-change hardening, debug and
monitoring hardening, and the documented non-safety stance.

Exit criteria:

- No silent acceptance of unsupported features.
- Documented artifacts and ABI; reproducible IR and manifests; clean build from a fresh checkout.
- Hot-swap, cold-download, manifest-download, debug, and monitoring suites pass on both hosts.
- Release checklist signed off.

### M8 — Linux host portability

Deliverables: host abstraction audit; native builds of `stc` and the runtime for Linux x86-64
and Linux AArch64; clang/linker discovery and artifact naming on Linux; state-file atomicity on
Linux; module loading, thread priority/affinity, and debug-info generation on Linux; CI jobs on
both hosts; host-support documentation.

Exit criteria:

- `cargo test --workspace` passes natively on Linux x86-64 and Linux AArch64.
- `stc new/build/check/run/test/clean` work end-to-end on both hosts.
- The execution suite, conformance corpus, hot-swap suite, manifest tests, and debug suite pass
  on both hosts; retained state round-trips.
- No host-specific conditionals remain in `stc-lexer`, `stc-parser`, `stc-sema`, `stc-codegen`,
  or the module/manifest model.

## 9. Verification strategy

| Level | Method |
| --- | --- |
| Lexer | Unit tests, snapshot tokens/trivia, property tests, fuzzing |
| Parser | CST snapshots, AST snapshots, error-recovery snapshots, round-trip |
| Analyzer | Positive/negative conformance cases, diagnostic snapshots, rule-ID traceability |
| IR | Deterministic golden `.ll`, `clang`/`opt` verification, structural checks |
| Execution | Compile and run ST programs; compare stdout/exit/state; mock clock and I/O |
| OOP | Inheritance/dispatch/interface interop cases, assignment-attempt/null cases |
| UTF-8 | Valid/invalid sequences, character count vs UTF-8 byte length, 4-byte scalar storage, O(1) indexing, emoji/supplementary scalars, slicing, conversions |
| Standard library | Function-by-function tests; Lean vectors for exact functions |
| Lean | `lake build` proofs in CI; generated vectors checked in and verified by Rust tests |
| CLI/manifest | Integration tests for every verb, exit code, JSON shape, and error path |
| Fuzzing | Lexer/parser/analyzer fuzz targets; no panics; bounded time/memory |
| Cross-target | `clang --target` smoke tests on IR/object emission (execution where available) |
| Online change | Hot-swap suite across attach/prepare/activate/commit/rollback/unload; incompatible-change rejection; cold-download recovery; double-build determinism |
| GOT/manifest | Identity/rename/move, exact-parent, download/consume, byte-identical manifests, corrupt/malformed parser fuzzing, quarantine/reset policy |
| Real-time boundary | O(1) switch measurement, bounded maintenance jitter (M7 target), between-cycle candidate preparation in M6, service-thread contention and generation pinning |
| Debug/monitoring | Typed read/write/force/watch, breakpoints/step/call stack, DWARF/CodeView, swap/session teardown, debug/release separation |
| Host portability | Full workspace tests and end-to-end CLI on Linux x86-64 and Linux AArch64 |

## 10. Risks and mitigations

| Risk | Impact | Mitigation |
| --- | --- | --- |
| 2013 + OOP + UTF-8 is large | Schedule and coherence | Tier features in the profile; implement in milestones; no silent gaps |
| Ed. 4 unavailable | UTF-8 semantics unverifiable | Define an explicit `stc` UTF-8 extension; do not claim Ed. 4 conformance |
| IEC copyright | Legal exposure | Keep the PDF local and ignored; write original prose and grammar; cite clause numbers |
| Lean proof scope creep | Stalls implementation | Restrict proofs to exact discrete functions; float math is test-based; define the model/implementation boundary |
| Textual IR limitations | Debug info/optimization gaps | Start with a stable IR subset; add metadata/debug info incrementally; validate with clang |
| Windows/LLVM toolchain drift | Build breakage | Pin Rust/Lean; discover clang via `--clang`/`STC_CLANG`; CI matrix with the pinned version |
| I/O metadata/runtime contract drift | Unbound or mis-bound variables | Version the metadata, test binding precedence and defaults, keep protocol details runtime-side |
| Retained-state corruption/layout drift | Failed starts or lost state | Magic/version/profile/layout hash + CRC, atomic save, `on-mismatch` policy, restart tests |
| Case-insensitive names + namespaces | Resolution bugs | Canonical symbol keys; dedicated tests for shadowing and `USING` |
| OOP dispatch/ABI | Hard-to-debug codegen | Vtable layout documented; ABI tests and IR snapshots |
| UTF-8 representation/indexing | Silent data corruption | Fixed 4-byte scalar storage; character/UTF-8-byte APIs kept distinct; exhaustive invalid-sequence tests |
| Runtime ABI churn | Breaks host integration | Version the ABI; keep runtime calls narrow and documented |
| Test runner hangs | CI instability | Per-test process isolation, timeouts, deterministic clocks |
| Windows-only assumptions in compiler/runtime | Linux port requires rework | Confine host calls (paths, clock, linker, module loading, atomic replace) to a platform layer; run Linux CI from M1; no shell or drive-letter assumptions |
| Online-change state corruption | Lost or inconsistent production state | Host-owned stable cells; per-field preserve/drop; exact-parent baseline; one-successful-cycle rollback; atomic snapshots; reject/reset incompatible roots |
| Manifest loss/corruption | Forced cold download or wrong baseline | Immutable versioned files plus active copy; digests and pairing checks; runtime export/download; tests treat the cold path as first-class |
| GOT/descriptor ABI churn | Preserved values become unsafe across versions | Versioned host ABI and descriptor; M4 artifacts non-loadable; structural validation; no module-owned descriptors/vtables |
| Loader/service thread interference | Cycle jitter or missed deadlines | Lower priority/affinity policy; bounded queues; lock-free status; O(1) switch; measure contention and maintenance jitter |
| Debug affects real-time behavior | Unexpectedly held outputs or timing changes | Explicit non-safety stance; watchdog pause documented; outputs held indefinitely; debug hooks absent in release artifacts; session teardown on swap |
| Migration/layout misclassification | Silent data reinterpretation | No field-layout migration in v1; exact type/layout matching; reset/reject/cold policy; compatibility report before deploy |
| Malicious/corrupt downloaded manifest | Code or data compromise | Treat as untrusted data; validate every field/digest; access-controlled download; never execute manifest content |

## 11. Definition of done for v1

- The supported language surface is exactly the documented profile; unsupported constructs are
  rejected with an explicit diagnostic.
- Both executable and library modes work, with a stable C ABI and a documented artifact layout.
- All exact functions have Lean models and proofs; all math functions have defined edge-case
  behavior and tests.
- UTF-8 strings are implemented and tested, with character/UTF-8-byte semantics and the 4-byte
  static scalar layout documented.
- OOP, namespaces, references, and configuration/task syntax are implemented to the profile.
- The conformance corpus passes on Windows x64; cross-target object emission is smoke-tested.
- The test runner, JSON diagnostics, and traceability report are usable.
- Local path dependencies compile topologically, `stc.lock` is deterministic and `--locked` is
  enforced in CI, package-aware namespaces resolve, and a separate test project can depend on the
  code under test.
- Unbound I/O has defined behavior (inputs default, outputs discarded), and I/O metadata carries
  devices, bindings, hints, and default values for the runtime.
- Retained state round-trips across restart, rejects layout/profile mismatches and corruption by
  policy, and writes atomically.
- Online change replaces a whole module at a cycle boundary without restarting the host; safe
  changes preserve compatible persistent state; incompatible or untracked changes are rejected
  or require a cold download.
- Every module is paired with a persistent GOT manifest; exact-parent builds work online, the
  runtime exports the active manifest, and a missing/stale/corrupt baseline forces a documented,
  tested cold download.
- Generated code contains no persistent raw pointers; all persistent roots and user/standard
  function calls go through the GOT; objects use host descriptors and handles.
- Full typed debug and monitoring works on both hosts, and every online swap ends the debug
  session cleanly.
- The project documentation states explicitly that `stc` is not a safety PLC and assumes no
  safety responsibility; debug sessions may hold outputs indefinitely.
- `stc fmt` is idempotent, preserves comments, emits strict semicolons, and supports `--check`.
- Production and release artifacts contain no test-only units or `STC_ASSERT` calls; production
  use of either is an error.
- Host-specific code is isolated behind a small platform layer; the planned Linux x86-64/AArch64
  port is additive (D-042).
- No known silent correctness gaps.

## 12. Immediate next actions

1. M6.5 (debug and monitoring) is complete (2026-10-28); its phase records are D-173–D-183,
   `M6.5-Plan.md`, and `handover_to_M7.md`. M7 (release candidate) is the next milestone:
   performance pass, cross-target smoke tests, packaging, examples, release notes, the
   known-limitations list, manifest/online-change hardening, debug/monitoring hardening, and the
   documented non-safety stance.
2. Keep the implemented debug contracts: debug is the default build form; `--release` is
   optimised, stripped, and hook-free with distinct build IDs/manifests; checkpoint hooks use
   host ABI service ID 13 at minor 2.3; the debug protocol is local-only `stc-debug/1`, separate
   from the M6 control channel; every swap ends the session; and forcing is boundary-applied,
   audited, and never a safety function. M7 hardening must not weaken the non-safety stance or
   add raw-memory access.
3. All planning questions Q-001 through Q-212 are resolved (A-005 through A-030, Q6-1–Q6-15);
   the online-change architecture is recorded in
   [Online Change and GOT](Architecture/Online-Change-and-GOT.md), and the resolved question log
   is in `update_plan_got.md` §13.0.
4. Keep the implemented host/module contracts: do not reintroduce the old direct-reference or
   contiguous-object model, and do not reintroduce module-owned state or descriptors
   (Q-090/Q-091/Q-095).
5. Keep host-specific operations (paths, clock, linker discovery, module loading, atomic
   replace) behind a platform layer; Linux x86-64/AArch64 host support follows basic development
   (M8, D-042).

## 13. Post-v1 outlook

- Field-layout migration for approved, pointer-free cases, building on per-field `GotId`s and
  the persistent manifest.
- Remote/network deployment, multi-user online-change scheduling, and a deploy/rollback UI.
- Linux x86-64 and Linux AArch64 host support (M8, D-042), then further hosts only on demand.
- Public CST/traversal API for IDE tools (v2/v3), building on the stable `SyntaxKind`.
- Hardware I/O backends and schedules: EtherCAT, Modbus, MQTT.
- Preemptive/multi-threaded ST execution, task stepping, and off-thread candidate preparation;
  M6 provides the cooperative single-cycle-thread scheduler.
- User-visible `__NEW`/`__DELETE` as a separate v2 vendor extension with its own memory model.
- Multi-root references to split persistent aggregates (v2): a place-descriptor form that names an
  aggregate's constituent per-field roots instead of one storage pointer, with host-side
  resolution that validates each root's identity, layout, and generation and a language-defined
  copy-back window for writes made through the reference. The per-field root model enables
  per-leaf preservation, quarantine, and retained-state flattening, but it leaves persistent
  structs and arrays of aggregates without a single stable address, so `REF`, `ADR`, and `SIZEOF`
  reject them and passing them falls back to whole-value copy-in/copy-out. The extension would let
  `REF_TO` values address those aggregates in place without reintroducing contiguous persistent
  parents; it requires an append-only descriptor/ABI addition plus explicit aliasing, lifetime,
  and validation rules. Raw `ADR` arithmetic would remain contiguous-only.
- Periodic/checkpointed RETAIN saves and stronger state integrity.
- Registry/version solving for dependencies.
- Configurable formatter and additional tooling (`stc lsp`, `stc doc`).
- Safety certification is never claimed by this project. A separate safety-qualified product
  would require an independent design, verification, and certification effort.
