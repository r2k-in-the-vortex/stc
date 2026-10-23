# stc Project Plan

Status: Draft for review  
Date: 2026-10-23  
Profile: `stc-2013-oop-utf8-v1`  
Standard reference: IEC 61131-3:2013 (Ed. 3.0), local copy under `Specification/Reference/`

## 1. Purpose

`stc` is a command-line compiler for IEC 61131-3 Structured Text. It will:

- parse and semantically analyse ST source written against the IEC 61131-3:2013 language,
- include the 2013 object-oriented features (classes, interfaces, inheritance, methods),
  namespaces, references, and configuration/task declarations,
- add a project-defined UTF-8 string extension (the 4th edition is not available and will not
  be purchased),
- emit textual LLVM IR, and
- invoke `clang` to produce objects, executables, or libraries.

The compiler is written in Rust. Specifications are written before compiler code, as required by
the project README. Mathematical functions with exact discrete semantics are specified and
proved in Lean before implementation.

## 2. Goals

1. A correct, well-diagnosed ST front end with a lossless syntax tree.
2. A typed middle end that resolves names, overloads, inheritance, interfaces, and conversions
   according to a written profile.
3. Deterministic textual LLVM IR with a documented ABI and name-mangling scheme.
4. A small Rust runtime with a stable C ABI, usable by both generated executables and embedding
   hosts.
5. Both artifact modes:
   - executable: generated `main` + cyclic runtime scheduler;
   - library: no `main`; exported `stc_init`, `stc_cycle`, `stc_shutdown`, and explicit exports.
6. A Cargo-like CLI with human and JSON diagnostics, verbose phase output, and a built-in test
   runner.
7. Lean models and proofs for exact functions (integer, bitwise, conversion, selection, and
   related discrete operations), plus generated test vectors consumed by Rust tests.
8. A project/dependency model: packages, local path dependencies, a dependency graph, and
   package-aware namespace visibility, enough for separate test projects.
9. A conformance corpus and traceability from specification rules to tests.

## 3. Non-goals for v1

- IEC 61131-3:2025 (Ed. 4.0) conformance. The project targets the 2013 language plus documented
  extensions and makes no Ed. 4 claim.
- LD, FBD, SFC, and IL as input languages. Only ST is parsed.
- The IEC communication model, HMI, online monitoring, and online debugging.
- Hardware I/O backends (EtherCAT, Modbus, MQTT) and their schedules in v1. They are planned for
  v2 and are noted in the I/O spec; v1 uses null/memory/file backends.
- Vendor project import (TIA Portal, CODESYS, etc.).
- The SIMATIC AX preprocessor and vendor-specific pragma semantics. AX documentation is used
  only as syntax inspiration.
- A network package registry and version-solving resolver in v1. Local path dependencies are in
  v1; a registry/version solver is a later extension (A-009).
- Formal verification of IEEE-754 transcendental functions. These remain test-based.
- Multi-threaded PLC task execution in v1 (architecture must not preclude it).
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
| Standard FBs / time | ST stdlib; Rust clock latched once at `stc_cycle` entry and shared by all timers in that scan (A-010) |
| I/O | Runtime-owned linking; compiler emits process-image/device/binding metadata; `{stc:io}` JSON hints; v1 null/memory/file; EtherCAT/Modbus/MQTT v2 (A-011) |
| Retained state | File-backed, versioned, layout-hashed, CRC-checked, atomic save; warm/cold restart; v1 saves at shutdown (A-012) |
| Standard text | Local IEC 61131-3:2013 PDF only; not committed or redistributed |
| UTF-8 strings | Project-defined extension; `USTRING[n]` stores `n` characters as 32-bit scalars (4 bytes each), static storage, UTF-8 source/interchange |
| Lean scope | Proofs for exact discrete functions cover the Lean model; Rust is tested against generated vectors (A-006); float/transcendental math is test-based |

Assumption: the host is single-threaded for compiler execution in v1. The generated runtime is
single-threaded; concurrency primitives added by Ed. 4 are out of scope.

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
- `Specification/Decisions.md` — decision log and open questions.
- This plan.

### 5.2 Implementation

- Rust workspace under `crates/`.
- `crates/stc-runtime` — Rust `staticlib`/`cdylib` runtime with a C ABI.
- `stdlib/` — standard function blocks written in ST (dogfooding the compiler).
- `lean/` — Lake project for exact-function models, proofs, and vector generation.
- `tests/` — compiler, conformance, codegen, execution, and end-to-end tests.
- `examples/` — small ST projects.
- `tools/xtask` or `crates/xtask` — build/test automation (runtime build, clang discovery,
  vector regeneration).

### 5.3 Documentation

- Updated top-level `README.md` with the confirmed profile, decisions, and question log.
- `examples/README.md` and a short user guide once the CLI exists.

## 6. Architecture at a glance

```
ST source (UTF-8)
   |
   v
lexer  ->  lossless CST  ->  typed AST  ->  name resolution  ->  type checking / HIR
   |                                                                        |
   +-- diagnostics <--------------------------------------------------------+
                                                                            |
                                                                            v
                                                             textual LLVM IR builder
                                                                            |
                                                          clang (assemble/link/optimize)
                                                                            |
                                              .ll / .o / executable / static or shared library
```

Key choices:

- Hand-written lexer and recursive-descent parser with a Pratt-style expression parser.
- Lossless CST (rowan-style) plus a typed AST/HIR; every node carries a byte span.
- Identifier interning with case-insensitive lookup and original-spelling preservation.
- A typed HIR after resolution; codegen never walks the raw AST.
- Textual IR is emitted deterministically by a small typed IR builder; `clang` validates and
  lowers it. This avoids binding the project to a specific LLVM C++/C API version.
- Runtime ABI is a narrow C ABI; generated modules call `stc_rt_*` functions.
- Object-oriented lowering: class/FB instances are structs, methods are functions with an
  explicit `this` pointer, interfaces are fat pointers `{data, vtable}`, and `OVERRIDE` calls
  through base/interface references dispatch through vtables.
- `VAR_IN_OUT` and `REF_TO` lower to pointers; value parameters are passed according to the C
  ABI for the target.
- Executable mode generates `main`; library mode exports `stc_init`, `stc_cycle`, and
  `stc_shutdown`, with the same generated module.
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

### M5 — FBs, OOP, aggregates, and UTF-8 strings

Deliverables: FB instances/state, methods, inheritance, interfaces/vtables, `THIS`/`SUPER`,
references, `VAR_IN_OUT`; static arrays, conformant `ARRAY [*]` parameters, structs, enums,
subranges; `STRING`/`WSTRING`/`USTRING`; standard functions; standard FB stdlib; cycle-latched
clock; single-task cyclic runtime; test runner MVP.

Exit criteria:
- OOP conformance cases (inheritance, overriding, interface dispatch, assignment attempt) pass.
- Static arrays and conformant `ARRAY [*]` parameters pass bounds/aliasing tests; dynamic
  locals/globals/returns are rejected with explicit diagnostics.
- UTF-8 vectors (valid/invalid sequences, character counts, UTF-8 byte lengths, 4-byte scalar
  indexing, slicing, emoji/supplementary scalars) pass.
- Standard timer/counter/edge/bistable FBs pass deterministic tests with a mock clock; all timers
  in one scan see the same latched cycle time.

### M6 — Project system, CLI, and verification hardening

Deliverables: full `stc.toml`, local path dependency graph, `stc.lock`, package-aware namespace
visibility, configurations/tasks, I/O device tree/bindings/defaults metadata, file-backed
retained state, build artifacts, JSON diagnostics, verbose output, `stc test`, minimal `stc fmt`,
Lean vector integration, conformance corpus, fuzzing, CI matrix, docs.

Exit criteria:
- `stc new/build/check/run/test/clean` work end-to-end on Windows.
- CI runs unit, snapshot, conformance, execution, fuzz-smoke, and Lean proof jobs.
- Traceability report shows every implemented rule has a test.

### M7 — Release candidate

Deliverables: performance pass, cross-target smoke tests via `clang --target`, packaging,
examples, release notes, known-limitations list.

Exit criteria:
- No silent acceptance of unsupported features.
- Documented artifacts and ABI; reproducible IR; clean build from a fresh checkout.
- Release checklist signed off.

### M8 — Linux host portability

Deliverables: host abstraction audit; native builds of `stc` and the runtime for Linux x86-64
and Linux AArch64; clang/linker discovery and artifact naming on Linux; state-file atomicity on
Linux; CI jobs on both hosts; host-support documentation.

Exit criteria:
- `cargo test --workspace` passes natively on Linux x86-64 and Linux AArch64.
- `stc new/build/check/run/test/clean` work end-to-end on both hosts.
- The execution suite and conformance corpus pass on both hosts; retained state round-trips.
- No host-specific conditionals remain in `stc-lexer`, `stc-parser`, `stc-sema`, or
  `stc-codegen`.

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
| Windows-only assumptions in compiler/runtime | Linux port requires rework | Confine host calls (paths, clock, linker, atomic replace) to a platform layer; run Linux CI from M1; no shell or drive-letter assumptions |

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
- `stc fmt` is idempotent, preserves comments, emits strict semicolons, and supports `--check`.
- Production and release artifacts contain no test-only units or `STC_ASSERT` calls; production
  use of either is an error.
- Host-specific code is isolated behind a small platform layer; the planned Linux x86-64/AArch64
  port is additive (D-042).
- No known silent correctness gaps.

## 12. Immediate next actions

1. Review and approve this plan, the conformance profile, and the tokenization/grammar/AST specs.
2. All v1 planning questions Q-001 through Q-017 are resolved (A-005 through A-021). New
   questions are logged in `Specification/Decisions.md` and `README.md` one at a time.
3. Create the Rust workspace skeleton and CI after the M0–M2 specs are approved.
4. Keep host-specific operations (paths, clock, linker discovery, atomic replace) behind a
   platform layer from M1; Linux x86-64/AArch64 host support follows basic development
   (M8, D-042).
5. Implement the lexer first; do not write codegen before the parser/analyzer contracts are
   stable.

## 13. Post-v1 outlook

- Linux x86-64 and Linux AArch64 host support (M8, D-042), then further hosts only on demand.
- Public CST/traversal API for IDE tools (v2/v3), building on the stable `SyntaxKind`.
- Hardware I/O backends and schedules: EtherCAT, Modbus, MQTT.
- Multi-task scheduler (cooperative or preemptive) and task stepping.
- Periodic/checkpointed RETAIN saves and stronger state integrity.
- Registry/version solving for dependencies.
- Configurable formatter and additional tooling (`stc lsp`, `stc doc`).
