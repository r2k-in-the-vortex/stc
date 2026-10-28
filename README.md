# stc — Structured Text Compiler

[![CI](https://github.com/r2k-in-the-vortex/stc/actions/workflows/ci.yml/badge.svg)](https://github.com/r2k-in-the-vortex/stc/actions/workflows/ci.yml)
[![Coverage](https://raw.githubusercontent.com/r2k-in-the-vortex/stc/badges/coverage.svg)](https://github.com/r2k-in-the-vortex/stc/actions/workflows/ci.yml)

`stc` compiles IEC 61131-3 Structured Text to textual LLVM IR. `clang` takes the IR from there to
x86-64 or any other target clang supports.

## Status

**M6.5 (debug and monitoring) is complete (2026-10-28); M7 (release candidate) is next.** M6
(project system, CLI, and verification hardening) is complete (2026-10-27). The repository
contains the source-map and diagnostic crates, the
full tokenizer, `stc-syntax` (stable `SyntaxKind`, lossless CST, typed AST views, strict pragma
JSON), `stc-parser` (full ST grammar, Annex A precedence, error recovery, unsupported-construct
diagnostics), `stc-hir` (arenas, stable IDs, interner, canonical types, resolved bodies,
`{stc:io}` hints, export names, folded task values), `stc-sema` (collection, namespaces/`USING`,
name resolution, type checking, constant evaluation, initializers, access control, OOP,
references, I/O and retention checks, HIR lowering), `stc-codegen` (deterministic textual LLVM
IR, the GOT module IR builder and verifier, and the catalog/descriptor/manifest projection),
`stc-got` (stable keys, `GotId`, catalog, manifest/plan and descriptor codecs), `stc-manifest`
(`stc.toml`, source discovery, local dependency graph, `stc.lock`), `stc-test` (test discovery
and the process-isolated runner), `stc-fmt` (the fixed-style formatter), and `stc-runtime`
(persistent host with the GOT, root allocator, loader, manifest store, swap state machine,
control/status thread, scheduler, ABI 2.2 services, cycle loop, process image, state, and I/O
backends). The driver provides the M6 CLI (`new`, `check`, `build`, `run`, `test`, `fmt`,
`clean`, `metadata`, `compliance`, `online download/diff`, `rename`, and `run --watch`), the
manifest-free `stc parse`/`check`/`build <file>...` bridge, and `--emit=ir,obj,module,exe,lib,
metadata`; clang lowers and links `module`, `executable` (thin launcher), and embedding-library
artifacts.

The online-change architecture is **resolved** (2026-10-24): the v1 runtime is a persistent host
plus loadable whole-program modules, persistent memory is host-owned and accessed through a
project-defined GOT, every module is paired with a persistent GOT manifest, and activation is a
cycle-boundary module swap. M4.5 phases 0–10 are complete (2026-10-25): the versioned `GotId`,
catalog, descriptor, host-ABI, manifest/plan, and build-ID contracts are fixed in
[GOT Artifacts and Schemas](Specification/Architecture/GOT-Artifacts.md); `stc-got` implements
stable keys, catalog records, binary/canonical-JSON manifests, baseline matching, tracked
renames, preserve/allocate/drop/reject classification, slot assignment, descriptor format-1
encoding/decoding and descriptor/manifest pairing, and deterministic snapshots; `stc-codegen`
has the module IR builder (GOT slot-table access, indirect cell calls, exported entry points,
descriptor/function-table globals) plus a structural verifier checked by clang-acceptance tests;
`stc-codegen::module` projects the M4 scalar surface to the root catalog, canonical slot
bindings, descriptor bytes, manifest pair, and online-change plan with golden snapshots and
second-build baseline tests; and `stc-runtime` provides the host GOT, root registry, quarantine,
transient allocator, artifact validation, platform module loader, ABI 2.2 host services,
active-manifest store/download, the load/prepare/activate/commit/rollback/unload state machine,
and a non-local trap boundary. Phase 7 converts generated execution to that model: globals,
program fields, direct variables, retain leaves, and task guards use GOT cells; user calls are
indirect; the launcher and embedding host load real compiler-produced module pairs; and existing
execution, trap, process-image, binding, and retain tests run through modules. Phase 8 completes
the scalar compatibility policy: online changes reconfigure the process image and remap direct
cells at activation, preserve values by `GotId`, warn about moved variables and quarantined roots,
and restore the image with a rollback; cold installs reinitialize roots and the image;
incompatible type/layout changes remain rejected until an explicit cold rebuild; and
compiler-produced modules exercise state preservation, new-root initialization, tracked rename,
untracked-removal policy, reject/cold, and direct remap. Phase 9 completes test and CI coverage:
`fuzz_got_manifest` and `fuzz_module_descriptor` validate untrusted artifacts without loading
native code; end-to-end double builds are byte-identical across output directories (including
deterministic Windows PE timestamps); a combined `exe,lib` build is loaded by both the launcher
and the embedding C harness; malformed baselines and options are negative-tested; concurrent
generation pins and metadata parsing have focused tests; and a recorded switch measurement (1 µs
at 1024 roots versus 86 ms prepare) supports the O(1) activation claim. CI runs an explicit
online-change job on Windows and Ubuntu. Phase 10 closes the milestone's documentation: the
specification set, plan, decision log, and M5 handover describe the implemented host/module
model, [Debug and Monitoring](Specification/Architecture/Debug-and-Monitoring.md) §12 records
the boundary M6.5 builds on, and the M5 handover addendum records the contracts M5 must extend.
The foundation milestone is M4.5 before M5; M6.5 implements full typed debug and monitoring.

M5 phases 1–7 extend that model with structs, enums, subranges, static arrays,
`STRING`/`WSTRING`/`USTRING`, references and output/in-out parameters, transient pointers
(`STC-POINTER`), classes and function blocks with methods/inheritance/`THIS`/`SUPER`, all
standard functions, the standard FB library with cycle-latched timers, and
aggregate/string/FB retained-state leaves. These constructs are
covered by clang acceptance, C-harness execution, and an online change that preserves FB,
aggregate, interface, class-handle, and descriptor state across a code-only swap. The transient
`STC-POINTER` extension (`POINTER TO`,
`ADR`, `SIZEOF`, pointer arithmetic/indexing, and `REF_TO`↔pointer conversions) is implemented.
Interface source constructs, dispatch, assignment attempts (`?=`), object references, and
`REF_TO`-to-interface are implemented and verified in phase 4b; references to split aggregate
values remain explicit diagnostics. Step 4b.0 records
host ABI 2.1 with the inline TYPEINFO, VTABLE, and object-record layout in
[GOT Artifacts](Specification/Architecture/GOT-Artifacts.md) §8.5 and
[Runtime and Execution](Specification/Architecture/Runtime-and-Execution.md) §2.6. Step 4b.1
projects TYPEINFO/class VTABLE descriptors, interface-prototype function records with trap stubs,
the class value/instance layout split (`PTR_TO` handle keys), and hidden non-retained
`HANDLE`/`OBJREC` roots; generated prepare fills fresh records and handles, and the catalog
formats remain version 1. Step 4b.2 builds the host-owned TYPEINFO/VTABLE registry before
prepare, keeps compatible record addresses stable across a warm swap, patches entries at
activation, rejects shape changes that a preserved object or interface depends on, and
quarantines removed descriptors. Step 4b.3 makes object values record handles: class and FB
locals own stack records, persistent objects use the hidden `HANDLE`/`OBJREC` roots, `THIS` is
the record handle, method and FB bodies receive `(handle, table, ...)`, class `:=` copies the
handle while FB `:=` copies fields, class/FB parameters follow handle/record isolation rules,
and every class/FB method call dispatches through TYPEINFO and the class VTABLE (`SUPER` calls
the base cell directly). Handles to activation-owned records are rejected from escaping. Step
4b.4 makes interface values 16-byte `{record, per-pair vtable}` pairs passed and returned by
value and stored as single roots in fields, globals, program fields, and arrays;
class/FB-to-interface upcasts use the compile-time base-first map index, interface-to-interface
upcasts search the TYPEINFO interface map, `= NULL`/`<> NULL` compare the record pointer, and
interface calls dispatch through the pair's vtable with `(record, table, ...)`. Interfaces to
activation-owned records are also rejected from escaping. Step 4b.5 lowers assignment attempts
inline (class base-chain checks, interface-map membership, and reference descriptor
compatibility; failure yields the all-zero `NULL` value without trapping), builds `Object` place
descriptors for `REF(object)` with record/state dereference and whole-object state assignment,
and converts `REF_TO` class/FB references to interfaces with null on null or incompatible input.
Step 4b.6 verifies the phase: real module warm swaps preserve interface values and class handles
while descriptor, record, and per-pair vtable addresses stay stable, activation patches the
preserved pair to the new method cells, fresh object records and handles initialize once and
roll back, vtable-shape changes are rejected against an active baseline, and committed OOP fuzz
seeds cover malformed TYPEINFO/VTABLE/interface-map inputs. Interface identity comparison
remains a diagnostic. M6 is complete (2026-10-27, phases 0–11): the `stc.toml` project manifest,
dependency graph and `stc.lock`, package-aware analysis, the project CLI, cooperative multi-task
scheduling and `stc run`, file-backed retained state, I/O device metadata with the
`null`/`memory`/`file` backends, the test runner, the formatter, the online-change workflow
(`rename`, `online download/diff`, `run --watch`), Lean vectors, the conformance/UI/traceability
corpus, `fuzz_manifest`, and the CI matrix are implemented. M6.5 is complete (2026-10-28,
phases 0–10): debug is the default build mode with DWARF/CodeView, a checkpoint table, forced
frame pointers, and a stopped-frame sidecar; `--release` is the distinct optimised, stripped,
hook-free mode; the host provides typed read/write/force/watch, breakpoints, stop-the-world
stepping, call stacks, the manifest-assisted split-object view, and a local authenticated
`stc-debug/1` session protocol with audit, a five-minute default stop/idle bound, and swap
teardown; and the `stc debug` CLI and `stc run --debug-server` expose it. Debug hooks add one
host service (ID 13 at host ABI 2.3) and are absent from release and test artifacts.
Debug-disabled overhead is measured on Windows and Linux and recorded in
[M6.5-Plan.md](M6.5-Plan.md); it is not a CI gate.
The persistent host, loadable modules, GOT roots, manifests, swap protocol,
and process-image remap are implemented and shared by the M5 lowering. Parser and semantic snapshots, IR snapshots, clang verification, execution
and C-harness tests, a committed
corpus, traceability tables, deterministic property tests, a 20k-term deep-chain test, and
`fuzz_lexer`, `fuzz_parser`, `fuzz_analyzer`, `fuzz_codegen`, `fuzz_got_manifest`, and
`fuzz_module_descriptor` targets are in place. The
specification continues to be written before implementation, as required below.

**Known M6.5 limitations.** Windows debug DLLs carry no CodeView and produce no PDB because the
current `lld-link` invocation omits `/DEBUG` (the CodeView assertion runs against the object);
stack unwinding and typed locals are limited to the stopped-frame table on x86-64, with
optimized-out values reported honestly and AArch64 frame rules deferred; a release embedding
static archive is copied from the runtime toolchain and not stripped by the driver; format-1
manifests require the documented cold download rather than migration; debug is local-only with
no remote/network or raw-memory API, one session per host, bounded watches/events, and a
five-minute default stop/idle bound. `stc` is not a safety PLC: debug may hold outputs
indefinitely, and no fail-safe, output timeout, or interlock is provided. The full list with
file references is in [handover_to_M7.md](handover_to_M7.md).

| Document | Purpose |
| --- | --- |
| [Project Plan](Specification/00-Project-Plan.md) | Scope, milestones, risks, definition of done |
| [Conformance Profile](Specification/01-Conformance-Profile.md) | Exactly what is implemented and claimed; stable diagnostic code index (§10) |
| [Tokenization](Specification/Syntax/Tokenization.md) | Source model, tokens, literals, trivia |
| [Grammar](Specification/Syntax/Grammar.md) | EBNF for the supported ST surface |
| [UTF-8 Strings](Specification/UTF8-Strings.md) | `USTRING`/`UCHAR` extension, encoding, length, ABI |
| [AST/CST/HIR](Specification/AST/AST.md) | Syntax tree and typed middle-end design |
| [Architecture](Specification/Architecture/Architecture.md) | Pipeline, crates, LLVM IR, ABI |
| [Runtime and Execution](Specification/Architecture/Runtime-and-Execution.md) | Execution model, runtime, standard functions/FBs |
| [I/O and Devices](Specification/Architecture/IO-and-Devices.md) | Process image, AT linking, device model, JSON pragma hints |
| [State and RETAIN](Specification/Architecture/State-and-Retain.md) | File-backed retained state, warm/cold restart, layout hash |
| [CLI and Manifest](Specification/Architecture/CLI-and-Manifest.md) | Commands, diagnostics, `stc.toml` |
| [Testing and Verification](Specification/Architecture/Testing-and-Verification.md) | Test framework, Lean, CI |
| [Online Change and GOT](Specification/Architecture/Online-Change-and-GOT.md) | Host/module model, GOT, manifest, swap protocol |
| [GOT Artifacts and Schemas](Specification/Architecture/GOT-Artifacts.md) | Versioned `GotId`, catalog, descriptor, manifest, plan, host ABI, build-ID formats |
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
| Debug/monitoring | Full typed debug implemented (M6.5): typed read/write/force/watch, breakpoints/step/call stack, DWARF/CodeView by default; `--release` stripped/hook-free; local authenticated `stc-debug/1`; every swap ends the debug session |
| Safety | Not a safety PLC and not responsible for safety in any way; debug may hold outputs indefinitely |
| UTF-8 strings | Project extension: `USTRING`, `UCHAR`, `U#'...'`, `USTRING#'...'`, `${HEX}` escapes; `USTRING[n]` stores `n` characters as 32-bit scalars (4 bytes each, static) |
| Strings | 2013 `STRING`/`WSTRING`/`CHAR`/`WCHAR` plus the UTF-8 extension |
| Standard library | 2013 standard functions and FBs (`TON`, `TOF`, `TP`, counters, edge, bistable, strings, math); FB bodies are synthesized HIR in `STC_STD` (D-134), clock latched once per scan |
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
stc build --emit=exe                # thin launcher + adjacent module pair
stc build --emit=lib                # embedding host library + separate module
stc build --emit=metadata           # <name>.stc-meta.json
stc build --emit=module             # loadable module + GOT manifest
```

- The manifest-free `stc build` defaults to `ir`; project builds default to `module` (with its
  paired manifest) for online-change deployments. `exe` is a thin host launcher for an adjacent
  module, and `lib` is the embedding host library with generated code kept in the separate
  module; `lib-type = "shared"` selects the shared host-library form. The two emit kinds may be
  combined.
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

## CLI (M6 implemented)

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
stc online download     export the active manifest and layout summary
stc online diff         compare a candidate manifest with the active baseline
stc rename              source-aware tracked rename
stc run --watch         watch, rebuild, and deploy online
stc debug               start a local debug/monitoring session
```

M6 implements the project workflow: `stc new/check/build/run/test/fmt/clean/metadata/compliance`,
`stc online download/diff`, `stc rename`, and `stc run --watch`, together with the `stc.toml`
manifest, a committed `stc.lock`, `--locked`, human/short/JSON diagnostics, `--color`, `-q`,
`-v`/`-vv`, lint levels, `--deny warnings`, the file-backed state and I/O options, and the
`--watch`/`--store`/`--host`/`--nonce`/`--baseline`/`--dry-run` options. The manifest-free
`stc parse`, `stc check <file>...`, and `stc build <file>...` bridge remains for tests and one-off
checks, with `--name`, `--emit`, `--out-dir`, `--target`, `--clang`, `--runtime`, `--relaxed`,
`--message-format`, `-v`, `--got-manifest`, `--cold`, `--rename`, and
`--allow-untracked-removal`. M4.5 added `--emit=module`, the thin `exe` launcher, and the
embedding `lib` host; M6 adds `lib-type = "shared"`. M6.5 adds `stc debug`, `--debug` and
`--release`, debug information, checkpoint hooks, and `stc run --debug-server`; debug is the
default form, and `--release` is the optimised, stripped, hook-free form with its own build ID
and manifest (D-178).

Cargo-like diagnostics (`file:line:col: error[STC-E-...]`), source snippets, notes, suggestions,
and JSON output are implemented. Details:
[CLI and Manifest](Specification/Architecture/CLI-and-Manifest.md). The stable diagnostic code
index, including the M6 project/test/online target codes, is
[Conformance Profile](Specification/01-Conformance-Profile.md) §10; JSON report shapes (test,
compatibility, metadata, compliance) are in CLI and Manifest §1.6.

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
opt-level = 0
debug = true                        # parsed; M6 builds the debug form without debug info (D-158)
emit = ["module", "exe", "metadata"]
out-dir = "target/stc/debug"
lib-type = "static"                 # static | shared, used when emit includes "lib"

[io]
backend = "null"                    # null | memory | file
file = "io.bin"

[[io.devices]]
name = "drive1"
kind = "ethercat"
config = { vendor = "0x1234", revision = 3 }

  [[io.devices.channels]]
  name = "DI5"
  direction = "input"
  width = 1

[[io.bindings]]
variable = "Main.Start"
device = "drive1"
channel = "DI5"
direction = "input"

[retain]
enabled = true
file = "motor.stcstate"
on-mismatch = "error"               # error | reset

[runtime]
module-dir = "target/stc/runtime/modules"
manifest-dir = "target/stc/runtime/manifests"
max-retired-generations = 1

[run]
cycles = 0                          # 0 = until interrupted

[test]
sources = ["tests/**/*.st"]
timeout = "5s"
default-cycles = 1
mock-clock = "0ns"
io = "memory"

[online]
baseline = "active"                 # active | explicit
cold-policy = "require-confirmation"
allow-untracked-removal = false

[debug]
default = true
protocol = "local"
force-sessions-end-on-swap = true
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
  .vscode/                   rust-analyzer test explorer and coverage tasks
  .markdownlint-cli2.jsonc   Markdown lint configuration
  M4.5-Plan.md               online-change foundation plan (complete)
  M5-Plan.md                 structs/OOP/standard-library plan (complete)
  M6-Plan.md                 project system, CLI, and verification plan (complete)
  M6.5-Plan.md               debug/monitoring plan (complete)
  handover_to_M5.md          M4.5 handover and M5 opening contract
  handover_to_M7.md          M6.5 handover and M7 opening contract
  update_plan_got.md         resolved online-change planning record
  tools/ci/check_docs.py     specification, link, and profile checks
  tools/ci/check_local.py    local CI parity checks (docs, lint, tests, fuzz)
  tools/ci/coverage_badge.py LCOV line-coverage badge generator
  tools/ci/coverage_summary.py Markdown summary from the llvm-cov JSON report
  tools/ci/traceability-allowlist.toml  explicit not-implemented rules
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
    stc-codegen/             textual LLVM IR, GOT module builder, catalog/manifest projection
    stc-got/                 stable keys, GotId, catalog, GOT manifest/plan and descriptor codecs
    stc-manifest/            stc.toml schema, dependency graph, source discovery, stc.lock
    stc-runtime/             persistent host: GOT, roots, loader, swap, scheduler, ABI 2 services, C ABI
    stc-test/                test discovery, wrapper generation, process-isolated runner
    stc-driver/              CLI binary `stc`
    xtask/                   `cargo xtask test --suite ...` and `traceability`
  fuzz/                      cargo-fuzz workspace; lexer/parser/analyzer/codegen/manifest/got/descriptor targets
  lean/                      Lake project: exact models, proofs, vector generator
  tests/
    conformance/             corpus with per-case .meta.toml rule annotations
    ui/                      diagnostic snapshots
    vectors/exact/           committed Lean-generated vectors
    cross-target/            AArch64 IR smoke fixture
  examples/                  small ST projects
  Specification/             this specification set
    Reference/               local IEC 61131-3:2013 PDF (gitignored)
```

There is no textual `stdlib/` package: the standard function blocks are synthesized HIR in the
reserved `STC_STD` scope (D-134), and the standard functions lower inline or through runtime
helpers.

The M0 baseline was reviewed before the workspace was created at M1. The real source tree is
`crates/`; the lexer tests and snapshots live under `crates/stc-lexer/tests/`, and `fuzz/` is a
separate cargo-fuzz workspace.

## Continuous integration

GitHub Actions runs [`.github/workflows/ci.yml`](.github/workflows/ci.yml) on pushes to `main` and
on pull requests.

- `docs` runs from M0: required-document checks, relative-link validation, profile consistency,
  reference-PDF exclusion (`tools/ci/check_docs.py`), and Markdown lint.
- `fmt`, `clippy`, `test`, `coverage`, `online`, `ui`, `conformance`, `execution`, `projects`,
  `debug`, `traceability`, `lean`, `fuzz-smoke`, and `cross-target` are active for the present
  tree. `test`, `online`, `ui`, `conformance`, `execution`, `projects`, and `debug` run on both
  Windows Server 2025 x64 and Ubuntu 24.04 x64; the host-independent jobs (`fmt`, `clippy`,
  `coverage`, `traceability`, `lean`, `fuzz-smoke`, `cross-target`) run on Ubuntu 24.04. The
  `test` and `online` jobs set `STC_REQUIRE_CLANG=1`; the execution tests build `stc-runtime`
  and link C harnesses with the same toolchain discovery as `stc build`. The `online` job runs
  the GOT identity, descriptor/manifest projection, host lifecycle, online-change CLI, and
  launcher/embedding artifact suites. The `projects` job runs the end-to-end
  `new/check/build/run/test/fmt/clean` workflow and the rename/download/diff/watch online
  workflow. The `debug` job runs `cargo xtask test --suite debug` with `STC_REQUIRE_CLANG=1`,
  preflights `llvm-readobj`/`readelf` so object-content assertions cannot silently skip, and
  records the informational overhead measurement only on manual dispatch; the opt-in
  `Debug overhead` workflow runs the same measurement on both hosts. The `traceability` job
  runs `cargo xtask traceability --check`. `fuzz-smoke` caps each input at one MiB.
- `coverage` runs the workspace tests under `cargo llvm-cov` with clang required, writes line,
  region, and function totals plus the lowest-coverage files to the job summary, uploads
  `lcov.info`, `summary.json`, and the HTML report as artifacts, and on `main` publishes
  `coverage.svg`/`coverage.json` to the `badges` branch that the README badge reads. It is a
  reporting job: the `test` job remains the pass/fail gate.
- Runner images are pinned (`windows-2025`, `ubuntu-24.04`) so `-latest` migrations do not change
  the matrix silently.
- Suite jobs call `cargo xtask test --suite <name> --locked`; the UI, conformance, execution, and
  projects jobs activate with both the suite directory and `xtask`.
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
| D-011 | v1 runtime is single-task; full config is parsed/analysed. Superseded by D-154 (cooperative multi-task scheduling). |
| D-012 | Standard FBs in ST; standard functions in the runtime. Superseded by D-134: FB bodies are synthesized HIR in `STC_STD`. |
| D-013 | Runtime ABI: `stc_init`/`stc_cycle`/`stc_shutdown` + `stc_rt_*`. Superseded by D-094/ABI 2 (`stc_module_*` + host API). |
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
| D-037 | v1 runs a single task per scan; the full task model is parsed/analysed. Superseded for M6 by D-154 (cooperative multi-task scheduling). |
| D-038 | `stc test` runs test processes sequentially. |
| D-039 | Lint levels live in `[lints]`; CLI flags override; errors cannot be downgraded. |
| D-040 | Test-only units and `STC_ASSERT` are excluded from production builds; production use is an error. |
| D-041 | `SyntaxKind` is a stable, append-only API; full CST/IDE API is planned for v2/v3. |
| D-042 | After basic development, `stc` and its runtime also run natively on Linux x86-64 and Linux AArch64 (ARM64); Windows remains the primary development host. |
| D-045–D-058 | M2 syntax decisions: arena CST owning the source, stable `SyntaxKind`, parser codes `STC-E-GRAM-105`–`111`, strict pragma JSON, token-extent diagnostics, recovery/missing-token policy, and interning deferred to M3. Details in `Specification/Decisions.md`. |
| D-059–D-075 | M3 analyzer decisions: HIR in M3, the multi-file `stc check` bridge and `_root` session namespace, the interner/session model, the stable diagnostic table, type canonicalization and conversions, overloads, the standard prelude, constant evaluation, access defaults, reference/attempt rules, parse-error suppression, test-only handling, deterministic ordering, and arena-based iterative lowering. Details in `Specification/Decisions.md` and `M3-Plan.md`. |
| D-094–D-108 | Online change is a v1 requirement: persistent host + loadable modules, host-owned GOT and per-field roots, handle-based OOP descriptors, place-descriptor references and transient `POINTER TO`, module lifecycle and double-buffered activation, persistent GOT manifest and exact-baseline/cold-download policy, full typed debug in v1, debug-by-default/`--release`, and the explicit non-safety stance. Details in `Specification/Decisions.md`, `update_plan_got.md`, and the online-change/debug specs. |
| D-113–D-117 | M4.5 phase 1 fixes the artifact contracts: normalized stable keys and content-derived `GotId`s, canonical binary/JSON encodings and digest chain, a pointer-free module descriptor with a separate function table, host ABI 2.0's exact service table and non-returning trap boundary, and build IDs derived from source/catalog/plan inputs. Details in `Specification/Architecture/GOT-Artifacts.md`. |
| D-118 | M4.5 phase 2 records implementation clarifications without changing format 1: elementary layout encoding tags, export wrappers as module-local code paired with the wrapped callable's identity, rejection of reintroduced retired `GotId`s, and direct-variable address metadata remaining in the sidecar. The `stc-got` crate implements the identity, manifest, and plan layer. |
| D-119–D-121 | M4.5 phases 3–4 fix the module IR shape (slot-table cell loads, indirect cell calls, ABI 2.0 entry points, structural verification), the M4 catalog projection (global/program-field/direct/task-guard roots, synthesized `_DEFAULT` topology, canonical imports, declaration-to-slot bindings), and the shared descriptor format-1 codec with digest/pairing validation and the acyclic descriptor/manifest sealing order. Details in `Specification/Architecture/GOT-Artifacts.md` and `Specification/Decisions.md`. |
| D-122–D-123 | M4.5 phases 5–6 implement the host substrate and swap controller: stable root/GOT storage, quarantine and transient lifetimes, descriptor/manifest/plan validation, the platform module loader, the ABI 2.0 host service table, atomic active-manifest storage and download, and load/prepare/activate/commit/rollback/unload with exact-currentness and cold paths. Details in `Specification/Architecture/GOT-Artifacts.md` and `Specification/Decisions.md`. |
| D-124 | M4.5 phase 7 converts generated execution to the GOT/module model: persistent state, direct variables, retain leaves, and task guards use GOT cells; calls are indirect through function cells; host services use import cells; slot access is the two-load sequence; generated modules export the ABI 2 entry-point set; the host configures the process image and direct bindings from the metadata sidecar; `--emit=module`, the thin launcher, and the embedding host archive are driver artifacts; and a non-local trap boundary enables rollback-window recovery. Existing execution tests run through compiler-produced module pairs, completing the execution conversion before M5 and giving phase 8 a working preservation/rollback substrate. |
| D-125 | M4.5 phase 8 completes the scalar compatibility/preservation policy: online activation reconfigures the process image or remaps direct cells in place, preserves values by `GotId`, and snapshots the image for rollback; moved variables and quarantined roots are exposed as runtime warnings; `reset_quarantine` is the explicit reset primitive while `Reset`/`Migrate` plans remain rejected; cold installs reinitialize roots and the image, and online prepare initializes only fresh roots; and compiler-produced modules cover initializer/state preservation, tracked rename, untracked-removal policy, type-change rejection with a cold reset, and direct-variable remap. |
| D-126 | M4.5 phase 9 completes test and CI coverage: `fuzz_got_manifest` and `fuzz_module_descriptor` decode untrusted artifacts without loading native code; end-to-end double builds are byte-identical after adding content-derived Windows PE timestamps (`/Brepro`); combined `exe,lib` output is loaded by both the launcher and an embedding C harness; malformed baselines, invalid options, metadata sidecars, and concurrent generation pins are covered; an ignored measurement records 500 ns/1 µs activation at 16/1024 roots against 1.7 ms/86 ms prepare; and CI adds an explicit Windows/Linux online-change job plus one-MiB fuzz input caps; this closes the M4.5 verification gap on both hosts and records the O(1) switch evidence. |
| D-127 | M4.5 phase 10 closes the milestone with a documentation and handover pass: the README, project plan, conformance profile, architecture/runtime/state/I/O/CLI/testing/debug specifications, plan, and decision log describe the implemented host/module model; `handover_to_M5.md` records the implemented contracts, M5 extension points, and open M5 decisions; and `Debug-and-Monitoring.md` §12 records the M4.5 boundary for M6.5. The debug-build entry flag is clarified: M4.5 marks its default non-release artifact form but emits no debug information or checkpoint hooks, and the M6.5 required-feature bit stays clear. This keeps the documentation truthful at the milestone boundary and prevents M5/M6.5 from reintroducing the retired direct-reference model or assuming debug hooks that do not exist yet. |
| D-153–D-164 | M6 phase 0 fixes the project/package model (kebab-case `stc.toml`, package identity, lockfile), the cooperative host scheduler, the local deploy/status control channel, tracked-rename history, per-test artifacts and the host ABI 2.2 test clock, the debug/release boundary, the formatter style, the file I/O lifecycle, the Lean/vector contract, traceability annotations, runtime path resolution, and the applied diagnostic/report-shape contracts. Details in `Specification/Decisions.md` and `M6-Plan.md` §13–§14. |
| D-165–D-172 | M6 phases 4–11 record the implementation details: run-loop exit codes and failed-scan scheduling, the `STCRET01` state file and I/O backends, the test-host result protocol and lexable synthetic names, formatter encoding preservation, the online download/diff/rename/watch workflow, the Lean vector format and the three codegen defects it exposed, the conformance/UI/traceability/CI pass, and the completion documentation pass. |
| D-173–D-183 | M6.5 debug and monitoring: the local `stc-debug/1` protocol and four scopes, one session per host with the stop/idle bound, typed read/write/force/watch and checkpoint-hook rules, debug-by-default with a stripped hook-free `--release`, the format-2 manifest extension, the stopped-frame sidecar, the split-object `view` key, and the phase-10 documentation/M7 handover pass. Details in `Specification/Decisions.md` and `M6.5-Plan.md`. |

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
| A-010 | Standard FBs and time source? | Standard FB bodies are synthesized HIR in `STC_STD` (D-134); the Rust clock is latched once per scan cycle. |
| A-011 | How is I/O linked? | Runtime-owned; TwinCAT-like device tree; `{stc:io}` JSON hints; unbound inputs default, outputs do nothing. |
| A-012 | RETAIN persistence in v1? | File-backed state with layout hash, CRC, atomic save; warm/cold restart. |
| A-013 | Unicode identifiers? | ASCII-only; UTF-8 only in comments and strings. |
| A-014 | Comparison precedence? | Annex A split: relational binds tighter than equality; both left-associative. |
| A-015 | Strict syntax mode? | Strict 2013 semicolons by default; `--relaxed` enables the superset. |
| A-016 | Formatter in v1? | Minimal `stc fmt`, fixed style, `--check`; no configuration. |
| A-017 | Multi-task scheduling in v1? | Single task through M5; M6 adds cooperative multi-task scheduling (D-154); preemptive/multi-threaded ST later. |
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

## Building and running (M6.5)

```sh
cargo run -p stc-driver -- new examples/motor
cargo run -p stc-driver -- check  --manifest-path examples/motor/stc.toml
cargo run -p stc-driver -- build  --manifest-path examples/motor/stc.toml
cargo run -p stc-driver -- build  --release --manifest-path examples/motor/stc.toml
cargo run -p stc-driver -- run    --manifest-path examples/motor/stc.toml --cycles 10
cargo run -p stc-driver -- run    --debug-server --manifest-path examples/motor/stc.toml
cargo run -p stc-driver -- debug  --manifest-path examples/motor/stc.toml
cargo run -p stc-driver -- test   --manifest-path examples/motor/stc.toml
cargo run -p stc-driver -- fmt    --manifest-path examples/motor/stc.toml --check
cargo run -p stc-driver -- metadata --manifest-path examples/motor/stc.toml --message-format json
cargo run -p stc-driver -- online download --manifest-path examples/motor/stc.toml --out motor.stc-got.json
cargo run -p stc-driver -- online diff examples/motor/target/stc/debug/motor-control.stc-got.json --manifest-path examples/motor/stc.toml
cargo run -p stc-driver -- rename Main.Speed Main.Rate --manifest-path examples/motor/stc.toml
cargo run -p stc-driver -- run --watch --manifest-path examples/motor/stc.toml

# Manifest-free bridge and the other artifact forms
cargo run -p stc-driver -- parse path/to/file.st
cargo run -p stc-driver -- check path/to/file.st path/to/other.st
cargo run -p stc-driver -- build examples/hello.st --emit=ir
cargo run -p stc-driver -- build examples/hello.st --emit=module,metadata
cargo run -p stc-driver -- build examples/hello.st --emit=module --got-manifest active.stc-got.json
cargo run -p stc-driver -- build examples/hello.st --emit=exe,lib,metadata --runtime target/debug/stc_runtime.lib
```

`stc new` writes a project that builds, runs, tests, and formats with the commands above; the
generated `stc.lock` is committed, and `--locked` makes a stale graph an error. `stc run` builds
`exe,module,metadata` under `target/stc/debug` by default, and `stc test` writes one cold test
artifact per test under `target/stc/test`. `stc build --release` selects the optimised,
stripped, hook-free form and `target/stc/release`; `stc debug` builds the debug form and launches
the authenticated local session. `stc run --watch` needs a local control endpoint and is intended
for interactive deployment.

Before pushing, run the same checks as the CI matrix (formatting, clippy, the
workspace tests with clang required, specification/link checks, Markdown lint, and a
compile of the separate `fuzz/` workspace):

```sh
python tools/ci/check_local.py
```

`fuzz/` is its own workspace, so `cargo test --workspace` and
`cargo clippy --workspace` do not compile it; the local script does. Pass
`--skip-markdownlint` or `--skip-fuzz` when Node.js or the fuzz workspace is
unavailable, and `--fail-fast` to stop at the first failing check.

Line coverage uses `cargo-llvm-cov`. The pinned toolchain already includes
`llvm-tools-preview`; install the subcommand once, then run the same command as CI:

```sh
cargo install cargo-llvm-cov --locked
cargo llvm-cov --workspace --locked --lcov --output-path target/lcov.info
cargo llvm-cov report --html
```

The HTML report is `target/llvm-cov/html/index.html`. The pinned toolchain includes `rust-src`,
so rust-analyzer can load the standard library. In VS Code,
`.vscode/settings.json` enables the rust-analyzer Test Explorer (reload the window once after
pulling this change), which populates the Testing panel, and points the Coverage Gutters
extension at `target/lcov.info`. The
`Rust: coverage` task runs the lcov command and then regenerates the HTML report from the same
data; coverage is generated by that task rather than by the Testing panel's run buttons.

`stc parse` prints a lossless CST dump (and the typed AST dump with `--ast`). `stc check` accepts
one or more files, builds one in-memory package, and reports lexical, syntax, and semantic
diagnostics. Semantic analysis is skipped when parsing has errors (D-070). `stc build` performs
the same analysis and then emits deterministic IR and, when requested and the toolchain is
available, the loadable ABI 2 module with its manifest, the thin launcher, the embedding host
library, and metadata. Runtime artifact names differ by host:
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
