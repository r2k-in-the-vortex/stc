# Decisions and Open Questions

Status: Living document  
Profile: `stc-2013-oop-utf8-v1`

This is the detailed decision log. The top-level `README.md` contains the compact, authoritative
question log; this file records rationale and the open backlog.

## 1. Answered questions

### A-001 — Language scope

**Answer:** Core ST plus namespaces, interfaces, and inheritance are required from the start.
The basis is `Specification/Reference/IEC 61131-3_2013.pdf`. UTF-8 strings are required from the
start.

**Consequences:** The profile includes 2013 OOP and namespaces in v1, not as a later extension.
The 2013 PDF is the normative reference. Ed. 4 is not available and is not a target.

### A-002 — Execution artifacts

**Answer:** Both executable and library outputs via `--emit`.

**Consequences:** The generated module exports `stc_init`, `stc_cycle`, and `stc_shutdown`; it
also generates a `main` in executable mode. The runtime ABI is shared by both modes.

### A-003 — Lean role

**Answer:** Proofs for exact functions.

**Consequences:** Lean models and proofs cover integer, bitwise, conversion, comparison,
selection, and exact string operations. Float/transcendental math remains test-based. The
model/implementation boundary is resolved by A-006: proofs cover the Lean model, and the Rust
implementation is validated against generated vectors.

### A-004 — Standard availability

**Answer:** Only the 2013 edition is available; the 4th edition will not be purchased.

**Consequences:** No Ed. 4 conformance claim. The UTF-8 extension is defined by this project from
public descriptions and first principles. The 2013 text is used as a reference only; the project
writes an independent specification.

### A-005 — UTF-8 string representation

**Answer:** `USTRING` allocates 4 bytes per character to preserve character indexability and to
hold any UTF-8 character, including emoji. Storage size must be static; ASCII text using 4 bytes
per character is acceptable.

**Consequences:** `USTRING[n]` has a static capacity of `n` Unicode scalar values stored as a
`[n x i32]` array (4 bytes each). `LEN` counts characters, `LEN_CODE_UNIT` reports the UTF-8 byte
length for interchange, and `LEN_MAX` returns the character capacity. UTF-8 is the
source/interchange encoding, not the in-memory encoding. Indexing is O(1).

### A-006 — Lean verification boundary

**Answer:** Model + test vectors.

**Consequences:** Lean proofs cover the Lean reference model for exact functions. The Rust
implementation is tested against deterministic vectors generated from the model. No Rust-to-Lean
refinement proof or Lean-to-C extraction is required in v1. Float/transcendental functions remain
test-based against high-precision references.

### A-007 — Variable-length arrays

**Answer:** Conformant parameters.

**Consequences:** `ARRAY [*]` is implemented for `VAR_IN_OUT` parameters and FB input/output
parameters using a descriptor that points at caller-owned storage. Dynamically sized locals,
globals, and function results are rejected with an explicit diagnostic. Static arrays are fully
supported. No runtime allocation is introduced.

### A-008 — Test declaration and project dependencies

**Answer:** Tests use `{stc:test}` on POUs, but the user will usually choose a separate test
project. Namespaces and a dependency system are expected in v1.

**Dependency resolution:** A-009 selects local paths plus a lockfile.

**Consequences:**
- Test discovery is pragma-based; tests compile into a separate binary with a fresh runtime.
- A test project is a normal ST project with `[dependencies]` pointing at the code under test.
- v1 includes a project/dependency model: package manifests, a dependency graph, topological
  compilation, cross-package namespace visibility, and `INTERNAL`/`PRIVATE` access enforcement.
- The dependency source is local paths plus a `stc.lock` lockfile (A-009). A registry/version
  solver is a later extension.

### A-009 — Dependency resolution

**Answer:** Local paths plus a lockfile.

**Consequences:** v1 resolves dependencies from local paths and writes a deterministic `stc.lock`
recording package names, versions, relative source paths, and dependency edges. `stc build` and
`stc check` update the lockfile unless `--locked` is given; `--locked` fails if an update would be
required. The lockfile is committed. There is no network access, registry, or version solver in
v1; a `version` requirement without a path is an error. A registry/version solver can be added
later without changing the manifest's dependency shape.

### A-010 — Standard function-block implementation and time

**Answer:** Standard FBs are implemented in ST under `stdlib/`. The deterministic time source is
in Rust. Time is latched once at the start of each scan cycle, so two timers evaluated in the same
cycle cannot observe different times.

**Consequences:**
- `stc_cycle` latches `cycle_now_ns` once at entry. All `TON`/`TOF`/`TP`/counter/edge/date-time
  operations in that cycle read the latched value via `stc_rt_cycle_now_ns()`.
- The live monotonic clock is separate (`stc_rt_monotonic_now_ns()`) and is not visible to ST code.
- Library hosts may set the next cycle's time (`stc_rt_set_cycle_time_ns`) for deterministic
  stepping; tests use a mock clock with the same semantics.
- The standard FBs are ST source compiled with the user project; the runtime supplies only the
  clock and exact helpers.

## 2. Confirmed decisions

| ID | Decision | Rationale |
| --- | --- | --- |
| D-001 | Base language is IEC 61131-3:2013 (Ed. 3.0), Structured Text only. | The 2013 PDF is available; Ed. 4 is not. |
| D-002 | Classes, interfaces, inheritance, methods, namespaces, and `USING` are in v1. | Required by the user; all are in the 2013 edition. |
| D-003 | UTF-8 strings are a v1 project extension (`STC-UTF8`). | Required from the start; Ed. 4 text is unavailable. |
| D-004 | Compiler emits textual LLVM IR; `clang` lowers/links to the target. | User requirement; removes LLVM API version coupling; IR is inspectable. |
| D-005 | Both executable and library artifacts are supported via `--emit`. | User requirement. |
| D-006 | Lean proofs cover exact functions; proofs are about the Lean model, and Rust is tested against generated vectors (A-006). | User requirement; transcendentals are not proof-feasible here. |
| D-007 | Primary host is Windows x86-64 MSVC; targets are selected via clang. | Matches the development environment and the user's target statement. |
| D-008 | Input is ST only; LD/FBD/SFC/IL and the AX preprocessor are out of profile. | Keeps the front end bounded; AX docs are syntax inspiration only. |
| D-009 | Specifications are original prose/grammar; the IEC PDF is not committed or redistributed. | Copyright compliance; the PDF is a local reference. |
| D-010 | Rust Cargo workspace under `crates/`; spec docs under `Specification/`. | Standard Rust structure; the stray `src` file is removed. |
| D-011 | v1 runtime is single-task; the full configuration model is parsed/analysed. | Bounded runtime scope; language surface remains complete. |
| D-012 | Standard FBs are implemented in ST under `stdlib/`; standard functions live in the runtime. | Dogfoods the compiler; keeps FB behavior in ST. |
| D-027 | The cycle time is latched once at `stc_cycle` entry and shared by all timers in that scan. | Determinism: same-cycle timer calls cannot disagree; tests can step a mock clock. |
| D-013 | Runtime ABI: `stc_init`, `stc_cycle`, `stc_shutdown`, plus `stc_rt_*` helpers; ABI version 1. | Stable C boundary for exe and host embedding. |
| D-014 | Strings are length-prefixed fixed-capacity values. `USTRING[n]` stores `n` characters as 32-bit scalars (4 bytes each, static); default 255 characters. | Bounded storage, O(1) character indexing, room for every Unicode scalar. |
| D-015 | Signed overflow, division by zero, bounds, and invalid conversions trap. | "Strict correctness" policy; no silent wraparound. |
| D-016 | Boolean `AND`/`&`/`OR` short-circuit left-to-right. | Deterministic and standard-permitted. |
| D-017 | All comparison operators share one precedence level and are left-associative. | Follows the 2013 operator table; recorded as an interpretation. |
| D-018 | Identifiers are ASCII-only in v1. | 2013 examples and practical tooling; Unicode is Q-008. |
| D-019 | Semicolon after block statements is optional before a block delimiter. | Common ST practice; syntax superset, strict mode is Q-010. |
| D-020 | Project file is `stc.toml`, schema version 1. | Explicit profile/artifact/test configuration. |
| D-021 | Build artifacts and local reference PDF are gitignored. | Reproducibility and copyright hygiene. |
| D-022 | Diagnostics use stable codes (`STC-E-*`, `STC-W-*`) with spans and JSON output. | Cargo-like UX and testability. |
| D-023 | `ARRAY [*]` is supported only as conformant parameters (`VAR_IN_OUT`, FB inputs/outputs); no dynamic locals/globals/returns in v1. | Bounded ABI and lifetime scope; caller owns the storage. |
| D-024 | v1 includes a project dependency system with local path dependencies, a dependency graph, and package-aware namespaces. | Separate test projects require it; source packages compile together. |
| D-025 | Tests are declared with `{stc:test}` and may live in a separate test project that depends on the code under test. | Keeps test code out of production packages while sharing the language tooling. |
| D-026 | Dependency resolution is local paths plus a committed `stc.lock`; `--locked` forbids updates; no registry/network/version solver in v1. | Reproducible multi-package builds without a registry subsystem. |

## 3. Open questions

Answers are recorded here and in `README.md` one at a time. Each question has a proposed default
so implementation can continue without blocking.

### Q-006 — I/O backend

**Question:** What should direct variables do in executable mode by default?

**Proposed default:** A zeroed memory process image; `--io=file` reads/writes a file; library
hosts install a callback backend.

### Q-007 — RETAIN/PERSISTENT persistence

**Question:** Does executable mode need persistent retained variables in v1?

**Proposed default:** No persistence in v1; parse/validate and warn. Library hosts own
persistence.

### Q-008 — Unicode identifiers

**Question:** ASCII-only identifiers or Unicode letters?

**Proposed default:** ASCII-only; UTF-8 is supported in source, comments, and strings.

### Q-009 — Comparison and grammar ambiguity policy

**Question:** Should equality/inequality and relational comparisons share one precedence level
(current) or follow Annex A's split? Are chained comparisons allowed?

**Proposed default:** One level, left-associative; chained comparisons type-check and are normally
rejected. Recorded as `STC-GRAM-009`.

### Q-010 — Strict mode and syntax supersets

**Question:** Should `stc` provide `--strict` to require strict 2013 semicolons and reject syntax
supersets?

**Proposed default:** Add `--strict` after v1; default behavior remains the documented superset.

### Q-011 — Formatter

**Question:** Is `stc fmt` part of v1?

**Proposed default:** No; the lossless CST supports it later. No style configuration in v1.

### Q-012 — Multi-task scheduling

**Question:** Should v1 implement multiple tasks/priorities?

**Proposed default:** No; parse/analyse the full model, run one task sequentially. A real
scheduler is a later milestone.

### Q-013 — Parallel test execution

**Question:** Should `stc test` run test processes in parallel by default?

**Proposed default:** Sequential in v1; `--test-threads N` is accepted but serializes until the
runner is proven deterministic.

### Q-014 — Lint configuration

**Question:** Manifest `[lints]` table vs CLI flags only?

**Proposed default:** CLI flags only in v1; manifest lints later.

### Q-015 — ASSERT in production builds

**Question:** Should `STC_ASSERT` be available outside tests?

**Proposed default:** Available but documented as a test/development facility; future gating by
profile.

### Q-016 — Stable syntax-kind API

**Question:** Should `SyntaxKind` numbering become a stable public API for external tools?

**Proposed default:** Internal in v1; stabilize if a formatter/LSP becomes a supported tool.

## 4. Decision process

- New decisions get the next `D-NNN`; superseded decisions are kept with a `Superseded by`
  line, not deleted.
- Questions get the next `Q-NNN`; answered questions move to §1 with an `A-NNN` entry and a
  summary in `README.md`.
- Each decision records the reason and the affected specification sections.
- A decision that changes the language surface also changes the conformance profile and adds a
  test ID.
