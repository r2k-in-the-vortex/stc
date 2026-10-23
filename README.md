# stc — Structured Text Compiler

`stc` compiles IEC 61131-3 Structured Text to textual LLVM IR. `clang` takes the IR from there to
x86-64 or any other target clang supports.

## Status

**Specification phase.** No compiler code has been written yet. The repository currently holds
the project plan, the specification baseline, the decision log, and the local IEC reference. The
specification is written before implementation, as required below.

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
| References | `REF_TO`, `REF()`, `^`, `NULL`, `VAR_IN_OUT` |
| Configuration | `CONFIGURATION`, `RESOURCE`, `TASK`, program configuration, `VAR_ACCESS` |
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

```
ST source (UTF-8)
  -> lexer -> lossless CST -> typed AST -> name resolution/type checking -> HIR
  -> textual LLVM IR (.ll)
  -> clang -> .o / executable / static library / shared library
```

- The compiler's primary target is **textual LLVM IR**; it does not link against the LLVM C/C++
  API. This keeps `stc` independent of LLVM binding versions and makes IR inspectable.
- `clang` selects the target and performs assembly, optimization, and linking.
- Both executable and library modes are supported through `--emit`:

```sh
stc build --emit=ir                 # <name>.ll
stc build --emit=ir,obj             # .ll + .o
stc build --emit=exe                # executable + retained .ll
stc build --emit=lib                # static library (or --lib-type=shared)
stc build --emit=exe,lib,ir         # all artifacts
```

- Default is `exe,ir`; `stc build --emit=ir` stops after IR.
- `--target <triple>` is passed to clang and emitted as the IR triple; without it, clang uses the
  host target.
- Executable mode generates a `main` that hands control to the runtime loop. Library mode exports
  `stc_init`, `stc_cycle`, `stc_shutdown`, and `{stc:export}` wrappers.

The full artifact layout, runtime ABI, and process-image model are in
[Architecture](Specification/Architecture/Architecture.md) and
[Runtime](Specification/Architecture/Runtime-and-Execution.md).

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

## CLI (planned)

```text
stc new <path>          scaffold a project
stc check               parse and analyse; no codegen
stc build               emit IR/objects/executable/library
stc run                 build and run an executable
stc test                build and run the built-in test suite
stc fmt                 format sources with the fixed style; --check for CI
stc clean               remove the artifact directory
stc metadata            print configuration, ABI, exports, layout
stc compliance          print the profile and feature matrix
```

Cargo-like diagnostics (`file:line:col: error[STC-E-...]`), source snippets, notes, suggestions,
`--message-format=json`, `-v`/`-vv`, color control, and warnings-as-errors. Details:
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
```

Local path dependencies are resolved into an acyclic package graph and pinned by a committed
`stc.lock`; `--locked` refuses updates. I/O binding is runtime-owned: `[io]` defines devices and
optional bindings, `AT` declarations may carry `{stc:io {...}}` hints, and the runtime may
override them. The full schema, lockfile format, I/O schema, path rules, unknown-key policy, and
environment overrides are in [CLI and Manifest](Specification/Architecture/CLI-and-Manifest.md)
and [I/O and Devices](Specification/Architecture/IO-and-Devices.md).

## Repository layout

```
stc/
  Cargo.toml                 workspace manifest (created at M1, after spec review)
  rust-toolchain.toml        pinned Rust toolchain (created with the workspace)
  crates/
    stc-span/                FileId, Span, SourceMap, locations
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
  stdlib/                    standard function blocks in ST
  lean/                      Lake project: exact models, proofs, vector generator
  tests/                     conformance, UI, codegen, execution, projects
  examples/                  small ST projects
  Specification/             this specification set
    Reference/               local IEC 61131-3:2013 PDF (gitignored)
```

The specification is being written before any compiler code; the workspace is created only after
M0–M2 documents are reviewed. The stray zero-byte `src` file has been removed; the real source
tree will be `crates/`.

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
| D-007 | Primary host: Windows x86-64 MSVC; target selected via clang. |
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
| D-032 | v1 persists `RETAIN` variables to a versioned, layout-hashed, CRC-checked state file with atomic writes. | Warm/cold restart behavior without a database or host dependency. |
| D-033 | Identifiers are ASCII-only; UTF-8 is limited to comments and strings. | Avoids Unicode normalization/case-folding complexity. |
| D-034 | Comparison precedence follows Annex A: relational binds tighter than equality. | Resolves the operator-table/Annex A disagreement. |
| D-035 | Strict 2013 semicolon rules by default; relaxed syntax is opt-in. | Conformance by default with a compatibility mode. |
| D-036 | v1 includes minimal `stc fmt` with one fixed style and `--check`. | Useful tooling without a configuration subsystem. |
| D-037 | v1 runs a single task per scan; the full task model is parsed/analysed. | Bounded runtime scope. |
| D-038 | `stc test` runs test processes sequentially. | Deterministic output and resource use. |
| D-039 | Lint levels live in `[lints]`; CLI flags override; errors cannot be downgraded. | Shared policy with CI control. |
| D-040 | Test-only units and `STC_ASSERT` are excluded from production builds; production use is an error. | Clean release artifacts. |
| D-041 | `SyntaxKind` is a stable, append-only API; full CST/IDE API is planned for v2/v3. | Future tooling without freezing internals now. |

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

### Open

**None as of 2026-10-23.** All v1 planning questions Q-001 through Q-017 are resolved
(A-005 through A-021). New questions will be added here one at a time.

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
```

## References

- IEC 61131-3:2013, *Programmable controllers — Part 3: Programming languages* (local reference
  only; not redistributed).
- [SIMATIC AX Structured Text language reference](https://docs.industrial-operations-x.siemens.cloud/r/en-us/ax/st-docs/11.7.50/structured-text-language-reference)
  — syntax inspiration; AX documents compliance with IEC 61131-3:2013 and an incomplete feature
  set. It is not a conformance target.
- IEC 61131-3:2025 Ed. 4 — not available to this project; used only via public summaries as design
  input for the UTF-8 extension. No Ed. 4 conformance is claimed.
- LLVM textual IR and clang documentation.
