# stc — Structured Text Compiler

[![CI](https://github.com/r2k-in-the-vortex/stc/actions/workflows/ci.yml/badge.svg)](https://github.com/r2k-in-the-vortex/stc/actions/workflows/ci.yml)

`stc` compiles IEC 61131-3 Structured Text to textual LLVM IR. `clang` takes the IR from there to
x86-64 or any other target clang supports.

## Status

**M4 LLVM IR core codegen complete.** The repository contains the source-map and diagnostic
crates, the full tokenizer, `stc-syntax` (stable `SyntaxKind`, lossless CST, typed AST views,
strict pragma JSON), `stc-parser` (full ST grammar, Annex A precedence, error recovery,
unsupported-construct diagnostics), `stc-hir` (arenas, stable IDs, interner, canonical types,
resolved bodies, `{stc:io}` hints, export names, folded task values), `stc-sema` (collection,
namespaces/`USING`, name resolution, type checking, constant evaluation, initializers, access
control, OOP, references, I/O and retention checks, HIR lowering), `stc-codegen` (deterministic
textual LLVM IR for the elementary scalar subset: expressions, checked arithmetic, casts,
control flow, functions, programs, globals, process image, scalar retained leaves), and
`stc-runtime` (Rust `staticlib` with the C ABI, cycle loop, process image, clock, traps, and
retained-state stubs). The driver provides `stc parse`, `stc check`, and the M4
`stc build <file>...` bridge with `--emit=ir,obj,exe,lib,metadata`; clang lowers and links
`executable` and static-library artifacts.

The online-change architecture is **resolved** (2026-10-24): the v1 runtime is a persistent host
plus loadable whole-program modules, persistent memory is host-owned and accessed through a
project-defined GOT, every module is paired with a persistent GOT manifest, and activation is a
cycle-boundary module swap. The foundation milestone is M4.5 before M5; full debug/monitoring is
M6.5. The resolution is recorded in `update_plan_got.md`; implementation has not started, and the
M4 artifact model remains the current implemented state.

M4 does not implement function blocks, OOP lowering, aggregates, strings, references, standard
functions, the standard FB library, the manifest, `stc run`, or `stc test`; those constructs are
rejected with explicit `STC-E-CODEGEN-001` diagnostics. Retained-state file I/O is an M6 stub
(returns "disabled"), and task scheduling beyond one task per resource is M5. Parser and
semantic snapshots, IR snapshots, clang verification, execution and C-harness tests, a committed
corpus, traceability tables, deterministic property tests, a 20k-term deep-chain test, and
`fuzz_lexer`, `fuzz_parser`, `fuzz_analyzer`, and `fuzz_codegen` targets are in place. The
specification continues to be written before implementation, as required below.

| Document | Purpose |
| --- | --- |
| [Project Plan](Specification/00-Project-Plan.md) | Scope, milestones, risks, definition of done |
| [Conformance Profile](Specification/01-Conformance-Profile.md) | Exactly what is implemented and claimed |
| [Tokenization](Specification/Syntax/Tokenization.md) | Source model, tokens, literals, trivia |
| [Grammar](Specification/Syntax/Grammar.md) | EBNF for the supported ST surface |
| [UTF-8 Strings](Specification/UTF8-Strings.md) | `USTRING`/`UCHAR` extension, encoding, length, ABI |
| [AST/CST/HIR](Specification/AST/AST.md) | Syntax tree and typed middle-end design |
| [Architecture](Specification/Architecture/Architecture.md) | Pipeline, crates, LLVM IR, ABI |
| [Runtime and Execution](Specification/Architecture/Runtime-and-Execution.md) | Execution model, runtime, stdlib |
| [I/O and Devices](Specification/Architecture/IO-and-Devices.md) | Process image, AT linking, device model, JSON pragma hints |
| [State and RETAIN](Specification/Architecture/State-and-Retain.md) | File-backed retained state, warm/cold restart, layout hash |
| [CLI and Manifest](Specification/Architecture/CLI-and-Manifest.md) | Commands, diagnostics, `stc.toml` |
| [Testing and Verification](Specification/Architecture/Testing-and-Verification.md) | Test framework, Lean, CI |
| [Online Change and GOT](Specification/Architecture/Online-Change-and-GOT.md) | Host/module model, GOT, manifest, swap protocol |
| [Debug and Monitoring](Specification/Architecture/Debug-and-Monitoring.md) | Typed access, forcing, breakpoints, debug info |
| [Decisions and Open Questions](Specification/Decisions.md) | Rationale and backlog |

## Goal

A Rust command-line compiler for IEC 61131-3:2013 Structured Text, including the 2013
object-oriented features (classes, interfaces, inheritance, methods), namespaces, references, and
configuration/task declarations, plus a project-defined UTF-8 string extension. It emits LLVM IR
and is designed to feel like Cargo: precise diagnostics, explicit project configuration, a built-in
test runner, and inspectable artifacts.

## Language profile

**Profile ID:** `stc-2013-oop-utf8-v1`

| Aspect | Decision |
| --- | --- |
| Base standard | IEC 61131-3:2013 (Ed. 3.0), Structured Text portions |
| 4th edition | Not available and not purchased; **no Ed. 4 conformance claim** |
| Input language | ST only; LD, FBD, SFC, and IL are not accepted |
| OOP | Classes, interfaces, inheritance, methods, access specifiers, `THIS`/`SUPER`, assignment attempt |
| Namespaces | `NAMESPACE`, `USING`, qualified names, `INTERNAL` |
| References | `REF_TO`, `REF()`, `^`, `NULL`, `VAR_IN_OUT`; persistent-capable `REF_TO` uses place descriptors |
| Pointer extension | `STC-POINTER`: transient-only `POINTER TO` with arithmetic/indexing/`SIZEOF`, no persistent pointer fields |
| Configuration | `CONFIGURATION`, `RESOURCE`, `TASK`, program configuration, `VAR_ACCESS` |
| Execution model | Persistent host runtime plus loadable whole-program modules; no per-function hot patching |
| Online change | Whole-module load/prepare, O(1) cycle-boundary activation, one-cycle rollback, off-thread unload; exact-baseline online change or cold download |
| Persistent memory | Host-owned GOT roots and stable storage; per-field roots, handle-based objects, runtime descriptors |
| Identity baseline | Persistent GOT manifest (canonical JSON + compact binary) paired with every module; tracked renames/moves; runtime export/download |
| Debug/monitoring | Full typed debug in v1; debug info by default; `--release` optional; every swap ends the debug session |
| Safety | Not a safety PLC and not responsible for safety in any way; debug may hold outputs indefinitely |
| UTF-8 strings | Project extension: `USTRING`, `UCHAR`, `U#'...'`, `USTRING#'...'`, `${HEX}` escapes; `USTRING[n]` stores `n` characters as 32-bit scalars (4 bytes each, static) |
| Strings | 2013 `STRING`/`WSTRING`/`CHAR`/`WCHAR` plus the UTF-8 extension |
| Standard library | 2013 standard functions and FBs (`TON`, `TOF`, `TP`, counters, edge, bistable, strings, math); FBs in ST, clock latched once per scan |
| Projects | `stc.toml` packages, local path dependencies, `stc.lock`, dependency graph, package-aware namespaces |
| Testing | `{stc:test}` POUs; tests may live in a separate test project that depends on the code under test |
| I/O | Runtime-owned linking; TwinCAT-like device tree in `stc.toml`; `{stc:io {...}}` hints on `AT`; unbound inputs use type defaults, unbound outputs do nothing |
| I/O backends | v1: null/memory/file; v2 planned: EtherCAT, Modbus, MQTT |
| Persistence | File-backed `RETAIN` state; warm restart restores, cold start initializes; layout hash + CRC |
| Tooling API | Stable append-only `SyntaxKind`; public CST/IDE API planned for v2/v3 |
| AX role | Syntax/UX inspiration only; AX preprocessor and vendor pragmas are out of profile |

The 2013 PDF is a **local reference**, not a redistributable artifact. It is excluded from version
control by `.gitignore`. The specification in `Specification/` is original prose and grammar; it
does not reproduce the standard. The 2013 text is consulted for clause-level behavior.

## Compilation model

```text
ST source (UTF-8)
  -> lexer -> lossless CST -> typed AST -> name resolution/type checking -> HIR
  -> textual LLVM IR (.ll)
  -> clang -> .o / loadable module / executable / static or shared library
```

- The compiler's primary target is **textual LLVM IR**; it does not link against the LLVM C/C++
  API. This keeps `stc` independent of LLVM binding versions and makes IR inspectable.
- `clang` selects the target and performs assembly, optimization, and linking.
- Both executable and library modes are supported through `--emit`:

```sh
stc build --emit=ir                 # <name>.ll
stc build --emit=ir,obj             # .ll + .o
stc build --emit=exe                # M4: code-linked exe; M4.5+: launcher + module
stc build --emit=lib                # M4: static code archive; M4.5+: embedding host lib
stc build --emit=metadata           # <name>.stc-meta.json
stc build --emit=module             # loadable module + GOT manifest (M4.5+)
```

- M4 defaults to `ir`; `stc build --emit=ir` stops after IR. The v1 target default becomes
  `module` (with its paired manifest) for online-change deployments; `exe` is a thin host
  launcher for an adjacent module, and `lib` is the embedding host library with generated code
  kept in the separate module. M4.5 allows `exe` and `lib` in one build; M4 rejects that
  combination because both are code-linked. Shared host libraries are M6.
- A module is paired with `<name>.stc-got.json` (canonical tooling/download) and
  `<name>.stc-got.bin` (compact runtime projection). `--got-manifest <file>` selects the build
  baseline; `--cold` explicitly requests a cold-layout build; `--rename old=new` records a
  tracked rename.
- The host runtime, module descriptor, GOT, and swap protocol are specified in
  [Online Change and GOT](Specification/Architecture/Online-Change-and-GOT.md).
- `--target <triple>` is passed to clang and emitted as the IR triple; without it, clang uses the
  host target.
- Windows 11 x86-64 (MSVC) is the primary development host. After basic development, `stc` and
  its runtime are expected to run natively on Linux x86-64 and Linux AArch64 (ARM64); target
  selection stays with clang, so either host can cross-compile (D-042).
- Executable mode generates a thin launcher that loads an adjacent module; library mode embeds
  the host. The module exports attach/prepare/cycle/shutdown/detach, explicit `{stc:export}`
  wrappers, and its descriptor.
- The host owns the GOT, persistent memory, process image, retained state, module lifetime, and
  debug/monitoring services. Generated module code never owns persistent storage.

The full artifact layout, runtime ABI, and process-image model are in
[Architecture](Specification/Architecture/Architecture.md),
[Runtime](Specification/Architecture/Runtime-and-Execution.md), and
[Online Change and GOT](Specification/Architecture/Online-Change-and-GOT.md).

## Lean

Lean 4.35.0 is the specification/proof tool for **exact** functions:

- integer arithmetic with checked overflow, division/modulo,
- bitwise operations, shifts, rotates,
- conversions, comparisons, selection (`SEL`, `MIN`, `MAX`, `LIMIT`, `MUX`),
- boolean operations and exact string character-count/UTF-8 byte-length operations.

Each exact function gets a Lean model and proofs before its implementation. The Lean generator
emits deterministic test vectors consumed by Rust tests. Proofs cover the Lean reference model;
the Rust implementation is validated against the generated vectors (A-006). Floating-point
transcendentals and platform libm behavior are not proved; they are tested against high-precision
reference vectors.

## CLI (M4 subset implemented)

```text
stc new <path>          scaffold a project
stc check               parse and analyse; no codegen
stc build               emit IR/objects/module/executable/library
stc run                 build and run an executable
stc test                build and run the built-in test suite
stc fmt                 format sources with the fixed style; --check for CI
stc clean               remove the artifact directory
stc metadata            print configuration, ABI, exports, layout
stc compliance          print the profile and feature matrix
stc online download     export the active manifest and layout summary (M6)
stc online diff         compare a candidate manifest with the active baseline (M6)
stc rename              source-aware tracked rename (M6)
stc run --watch         watch, rebuild, and deploy online (M6)
stc debug               start a local debug/monitoring session (M6.5)
```

M4 implements `stc parse`, `stc check`, and `stc build` (with `--name`, `--emit`, `--out-dir`,
`--target`, `--clang`, `--runtime`, `--relaxed`, `--message-format`, and `-v`). `stc new`,
`stc run`, `stc test`, `stc fmt`, `stc clean`, `stc metadata`, and `stc compliance`, the
manifest, verbose `-vv`, color control, and warnings-as-errors arrive in M6. M4.5 adds
`--emit=module`, `--got-manifest`, `--cold`, `--rename`, the thin `exe` launcher, the embedding
`lib` host, and the online-change foundation.
`--debug`/`--release` select debug-info emission; debug is the default and `--release` is the
optional stripped/optimised mode.

Cargo-like diagnostics (`file:line:col: error[STC-E-...]`), source snippets, notes, suggestions,
and JSON output are implemented. Details:
[CLI and Manifest](Specification/Architecture/CLI-and-Manifest.md).

## Project manifest

`stc.toml` describes the package, source globs, selected configuration, build target and
artifacts, runtime options, and tests. Example:

```toml
schema = 1

[package]
name = "motor-control"
version = "0.1.0"
language = "stc-2013-oop-utf8-v1"

[dependencies]
motor-lib = { path = "../motor-lib" }

[project]
sources = ["src/**/*.st"]
configuration = "Main"

[build]
target = "x86_64-pc-windows-msvc"   # optional; default = clang host
opt-level = 2
debug = true
emit = ["exe", "ir"]

[test]
sources = ["tests/**/*.st"]
timeout = "5s"

[runtime]
module_dir = "runtime/modules"
manifest_dir = "runtime/manifests"
max_retired_generations = 1

[online]
baseline = "active"          # exact current baseline; missing/stale -> cold
cold_policy = "require-confirmation"
allow_untracked_removal = false

[debug]
default = true               # debug info is the default build output
protocol = "local"
force_sessions_end_on_swap = true
```

Local path dependencies are resolved into an acyclic package graph and pinned by a committed
`stc.lock`; `--locked` refuses updates. I/O binding is runtime-owned: `[io]` defines devices and
optional bindings, `AT` declarations may carry `{stc:io {...}}` hints, and the runtime may
override them. The full schema, lockfile format, I/O schema, path rules, unknown-key policy, and
environment overrides are in [CLI and Manifest](Specification/Architecture/CLI-and-Manifest.md)
and [I/O and Devices](Specification/Architecture/IO-and-Devices.md). Online-change and debug
schema details are in
[Online Change and GOT](Specification/Architecture/Online-Change-and-GOT.md) and
[Debug and Monitoring](Specification/Architecture/Debug-and-Monitoring.md).

## Repository layout

```text
stc/
  .github/workflows/         GitHub Actions CI
  .markdownlint-cli2.jsonc   Markdown lint configuration
  M4.5-Plan.md               online-change foundation plan
  M6.5-Plan.md               debug/monitoring plan
  update_plan_got.md         resolved online-change planning record
  tools/ci/check_docs.py     specification, link, and profile checks
  Cargo.toml                 workspace manifest
  rust-toolchain.toml        pinned Rust toolchain
  crates/
    stc-span/                FileId, Span, SourceMap, locations, UTF-8 decoding
    stc-diag/                diagnostics, codes, human/JSON rendering
    stc-lexer/               tokenizer, trivia, literals, pragmas
    stc-syntax/              CST and typed AST wrappers
    stc-parser/              recursive descent + expression parser, recovery
    stc-fmt/                 fixed-style formatter over the lossless CST
    stc-hir/                 resolved/typed HIR, arenas, types
    stc-sema/                name resolution, type checking, constant eval, lowering
    stc-codegen/             textual LLVM IR builder, ABI, mangling
    stc-manifest/            stc.toml schema, dependency graph, source discovery
    stc-runtime/             Rust runtime library with C ABI
    stc-test/                test discovery and runner
    stc-driver/              CLI binary `stc`
  fuzz/                      cargo-fuzz workspace; lexer/parser/analyzer/codegen targets
  stdlib/                    standard function blocks in ST
  lean/                      Lake project: exact models, proofs, vector generator
  tests/                     conformance, UI, codegen, execution, projects
  examples/                  small ST projects
  Specification/             this specification set
    Reference/               local IEC 61131-3:2013 PDF (gitignored)
```

The M0 baseline was reviewed before the workspace was created at M1. The real source tree is
`crates/`; the lexer tests and snapshots live under `crates/stc-lexer/tests/`, and `fuzz/` is a
separate cargo-fuzz workspace.

## Continuous integration

GitHub Actions runs [`.github/workflows/ci.yml`](.github/workflows/ci.yml) on pushes to `main` and
on pull requests.

- `docs` runs from M0: required-document checks, relative-link validation, profile consistency,
  reference-PDF exclusion (`tools/ci/check_docs.py`), and Markdown lint.
- `fmt`, `clippy`, `test`, `ui`, `conformance`, `execution`, `lean`, `fuzz-smoke`, and
  `cross-target` activate when their inputs exist. `test`, `ui`, `conformance`, and `execution`
  run on both Windows Server 2025 x64 and Ubuntu 24.04 x64; the host-independent jobs (`fmt`,
  `clippy`, `lean`, `fuzz-smoke`, `cross-target`) run on Ubuntu 24.04. The `test` job sets
  `STC_REQUIRE_CLANG=1`, and the execution tests build `stc-runtime` and link C harnesses with
  the same toolchain discovery as `stc build`.
- Runner images are pinned (`windows-2025`, `ubuntu-24.04`) so `-latest` migrations do not change
  the matrix silently.
- Suite jobs call `cargo xtask test --suite <name> --locked`; the UI, conformance, and execution
  jobs activate with both the suite directory and `xtask`.
- Every job prints the clang version; builds use `--locked`; warnings are denied.

The job definitions and matrix follow [Testing and Verification](Specification/Architecture/Testing-and-Verification.md) §6.

## Confirmed decisions

The full log with rationale is in [Decisions](Specification/Decisions.md).

| ID | Decision |
| --- | --- |
| D-001 | Base language is IEC 61131-3:2013 ST; no Ed. 4 claim. |
| D-002 | Classes, interfaces, inheritance, methods, namespaces are in v1. |
| D-003 | UTF-8 strings are a v1 project extension (`STC-UTF8`). |
| D-004 | Emit textual LLVM IR; clang lowers/links to the target. |
| D-005 | Executable and library artifacts via `--emit`. |
| D-006 | Lean proofs cover exact functions; float math is test-based. |
| D-007 | Primary development host: Windows x86-64 MSVC; Linux x86-64 and ARM64 hosts follow basic development (D-042); targets selected via clang. |
| D-008 | ST only; no LD/FBD/SFC/IL and no AX preprocessor. |
| D-009 | Specification is original; IEC PDF is local and not redistributed. |
| D-010 | Rust workspace under `crates/`; spec under `Specification/`. |
| D-011 | v1 runtime is single-task; full config is parsed/analysed. |
| D-012 | Standard FBs in ST; standard functions in the runtime. |
| D-013 | Runtime ABI: `stc_init`/`stc_cycle`/`stc_shutdown` + `stc_rt_*`. |
| D-014 | Strings are length-prefixed fixed-capacity values; `USTRING` uses 4-byte scalars; default 255 characters. |
| D-015 | Overflow, divide-by-zero, bounds, invalid conversions trap. |
| D-023 | `ARRAY [*]` is limited to conformant parameters; no dynamic locals/globals/returns. |
| D-024 | v1 includes local path dependencies, a dependency graph, and package-aware namespaces. |
| D-025 | Tests use `{stc:test}` and may live in a separate test project. |
| D-026 | Dependencies are local paths plus a committed `stc.lock`; `--locked` forbids updates. |
| D-027 | The cycle time is latched once per scan and shared by all timers. |
| D-028 | Hardware I/O linking is runtime-owned; v1 backends are null/memory/file; EtherCAT/Modbus/MQTT are v2. |
| D-029 | `AT` takes one `{stc:io <json-object>}` hint; project bindings and runtime overrides take precedence. |
| D-030 | Unbound inputs use type defaults or declared initializers; unbound outputs are discarded; `%M` persists. |
| D-031 | Pragmas are non-nested but may carry balanced JSON payloads. |
| D-032 | v1 persists `RETAIN` variables to a versioned, layout-hashed, CRC-checked state file with atomic writes. |
| D-033 | Identifiers are ASCII-only; UTF-8 is limited to comments and strings. |
| D-034 | Comparison precedence follows Annex A: relational binds tighter than equality. |
| D-035 | Strict 2013 semicolon rules by default; relaxed syntax is opt-in. |
| D-036 | v1 includes minimal `stc fmt` with one fixed style and `--check`. |
| D-037 | v1 runs a single task per scan; the full task model is parsed/analysed. |
| D-038 | `stc test` runs test processes sequentially. |
| D-039 | Lint levels live in `[lints]`; CLI flags override; errors cannot be downgraded. |
| D-040 | Test-only units and `STC_ASSERT` are excluded from production builds; production use is an error. |
| D-041 | `SyntaxKind` is a stable, append-only API; full CST/IDE API is planned for v2/v3. |
| D-042 | After basic development, `stc` and its runtime also run natively on Linux x86-64 and Linux AArch64 (ARM64); Windows remains the primary development host. |
| D-045–D-058 | M2 syntax decisions: arena CST owning the source, stable `SyntaxKind`, parser codes `STC-E-GRAM-105`–`111`, strict pragma JSON, token-extent diagnostics, recovery/missing-token policy, and interning deferred to M3. Details in `Specification/Decisions.md`. |
| D-059–D-075 | M3 analyzer decisions: HIR in M3, the multi-file `stc check` bridge and `_root` session namespace, the interner/session model, the stable diagnostic table, type canonicalization and conversions, overloads, the standard prelude, constant evaluation, access defaults, reference/attempt rules, parse-error suppression, test-only handling, deterministic ordering, and arena-based iterative lowering. Details in `Specification/Decisions.md` and `M3-Plan.md`. |
| D-094–D-108 | Online change is a v1 requirement: persistent host + loadable modules, host-owned GOT and per-field roots, handle-based OOP descriptors, place-descriptor references and transient `POINTER TO`, module lifecycle and double-buffered activation, persistent GOT manifest and exact-baseline/cold-download policy, full typed debug in v1, debug-by-default/`--release`, and the explicit non-safety stance. Details in `Specification/Decisions.md`, `update_plan_got.md`, and the online-change/debug specs. |

## Question log

Answers are recorded here and in `Specification/Decisions.md`, one at a time.

### Answered

| ID | Question | Answer |
| --- | --- | --- |
| A-001 | Language scope? | Core ST + namespaces/interfaces/inheritance; base on the 2013 PDF; UTF-8 from the start. |
| A-002 | What does `stc build` produce? | Both executable and library via `--emit`; IR is the compilation target. |
| A-003 | Role of Lean? | Proofs for exact functions; float/transcendental math is test-based. |
| A-004 | Which standard text? | 2013 PDF only; Ed. 4 is unavailable and will not be purchased; UTF-8 is a project extension. |
| A-005 | `USTRING` capacity/length semantics? | Static 4 bytes per character; `USTRING[n]` holds n characters; `LEN` counts characters; `LEN_CODE_UNIT` reports UTF-8 bytes. |
| A-006 | Do Lean proofs cover the Rust implementation or the model? | Proofs cover the Lean model; Rust is tested against generated vectors. |
| A-007 | How much variable-length array support? | Conformant parameters only (`VAR_IN_OUT`, FB params); no dynamic locals/globals/returns. |
| A-008 | How are tests declared and organised? | `{stc:test}` on POUs; separate test projects are supported; dependency system is v1. |
| A-009 | Dependency system scope? | Local path dependencies plus a committed `stc.lock`; no registry/version solver in v1. |
| A-010 | Standard FBs and time source? | FBs in ST stdlib; Rust clock latched once per scan cycle. |
| A-011 | How is I/O linked? | Runtime-owned; TwinCAT-like device tree; `{stc:io}` JSON hints; unbound inputs default, outputs do nothing. |
| A-012 | RETAIN persistence in v1? | File-backed state with layout hash, CRC, atomic save; warm/cold restart. |
| A-013 | Unicode identifiers? | ASCII-only; UTF-8 only in comments and strings. |
| A-014 | Comparison precedence? | Annex A split: relational binds tighter than equality; both left-associative. |
| A-015 | Strict syntax mode? | Strict 2013 semicolons by default; `--relaxed` enables the superset. |
| A-016 | Formatter in v1? | Minimal `stc fmt`, fixed style, `--check`; no configuration. |
| A-017 | Multi-task scheduling in v1? | Single task per scan; full model parsed/analysed. |
| A-018 | Parallel test processes? | Sequential only in v1. |
| A-019 | Lint configuration? | Manifest `[lints]` with per-code entries; CLI overrides. |
| A-020 | `STC_ASSERT` in production builds? | Test-only; production builds exclude test units and asserts; production use is an error. |
| A-021 | Stable `SyntaxKind`/CST API? | Stable append-only `SyntaxKind`; public CST/IDE API planned for v2/v3. |
| A-022 | Is online change in v1? | Yes: persistent host + loadable whole modules, local host/CLI workflow, M4.5 foundation before M5. |
| A-023 | GOT and persistent memory model? | Host-owned GOT cells, per-field roots, handle-based objects; no persistent raw pointers or module-owned state. |
| A-024 | References and `POINTER TO`? | `REF_TO` uses place descriptors; pointer→`REF_TO` is runtime-validated; `POINTER TO` is a transient-only `STC-POINTER` extension. |
| A-025 | How is a module swapped? | Background load/prepare, double-buffered GOT, O(1) cycle-boundary switch, one-cycle rollback, off-thread unload. |
| A-026 | What is the manifest and baseline policy? | Persistent GOT manifest paired with every module; exact-parent online change; missing/stale/invalid baseline forces cold download. |
| A-027 | What survives an online change? | Matching per-field roots and compatible `RETAIN`; new roots initialize, removed roots quarantine; no field-layout migration in v1. |
| A-028 | How do OOP descriptors survive? | Runtime-owned descriptors and vtable cells; preserved interfaces survive compatible swaps; shape changes are rejected while live. |
| A-029 | Tooling, determinism, and loader security? | Module/manifest CLI and tables, stable diagnostics, deterministic builds, strict validation of untrusted manifest data. |
| A-030 | Debug and non-functional stance? | Full typed debug in v1; debug info by default; swaps end sessions; debug may hold outputs; not a safety PLC. |

### Open

**None as of 2026-10-24.** All v1 planning questions Q-001 through Q-212 are resolved
(A-005 through A-030). The detailed online-change disposition is in `update_plan_got.md`
§13.0. New questions will be added here one at a time.

## Building and running (M4)

```sh
cargo test --workspace --locked
cargo build -p stc-runtime
cargo run -p stc-driver -- parse path/to/file.st
cargo run -p stc-driver -- check path/to/file.st path/to/other.st
cargo run -p stc-driver -- build examples/hello.st --emit=ir
cargo run -p stc-driver -- build examples/hello.st --emit=exe     --runtime target/debug/stc_runtime.lib
cargo run -p stc-driver -- build examples/hello.st --emit=lib,metadata     --runtime target/debug/stc_runtime.lib
```

`stc parse` prints a lossless CST dump (and the typed AST dump with `--ast`). `stc check` accepts
one or more files, builds one in-memory package, and reports lexical, syntax, and semantic
diagnostics. Semantic analysis is skipped when parsing has errors (D-070). `stc build` performs
the same analysis and then emits deterministic IR and, when requested and the toolchain is
available, objects, executables, or static libraries. Runtime artifact names differ by host:
`stc_runtime.lib` on Windows MSVC and `libstc_runtime.a` on Linux GNU. Human diagnostics go to
stderr; JSON diagnostics go to stdout.

## Dev environment

```text
PS C:\repos\stc> clang -v
clang version 23.1.4 (https://github.com/llvm/llvm-project 7a941dfb2877a0b8cf30323fefa85ad019e0b0f5)
Target: x86_64-pc-windows-msvc
Thread model: posix
InstalledDir: C:\Program Files\LLVM\bin
Build config: +alloc:rpmalloc

PS C:\repos\stc> lake --version
Lake version 5.0.0-src+eb6cafb (Lean version 4.35.0)

PS C:\repos\stc> cargo -V
cargo 1.99.1 (5f94df478 2026-08-27)

PS C:\Users\rkord> gh --version
gh version 2.103.2 (2026-10-23)
https://github.com/cli/cli/releases/tag/v2.103.2
```

Windows 11 x86-64 is the primary development host. Native Linux x86-64 and Linux AArch64 host
support follows basic development (D-042).

## References

- IEC 61131-3:2013, *Programmable controllers — Part 3: Programming languages* (local reference
  only; not redistributed).
- [SIMATIC AX Structured Text language reference](https://docs.industrial-operations-x.siemens.cloud/r/en-us/ax/st-docs/11.7.50/structured-text-language-reference)
  — syntax inspiration; AX documents compliance with IEC 61131-3:2013 and an incomplete feature
  set. It is not a conformance target.
- IEC 61131-3:2025 Ed. 4 — not available to this project; used only via public summaries as design
  input for the UTF-8 extension. No Ed. 4 conformance is claimed.
- LLVM textual IR and clang documentation.
