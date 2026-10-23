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

### A-011 — I/O linking

**Answer:** Linking I/O variables to actual hardware is the runtime's job; I/O backends and
schedules are v2. Primary planned backends are EtherCAT, Modbus, and MQTT (noted, not designed
now). The project uses a TwinCAT-like device tree: `stc.toml` describes devices/channels, and an
`AT` declaration may carry a `{stc:io ...}` JSON hint; the runtime may override it. If no linking
is defined, inputs receive their type's default value and outputs do nothing externally.

**Consequences:**

- The compiler validates `AT` addresses/types, computes process-image offsets, and emits device,
  binding, and default-value metadata. It never opens devices or schedules I/O.
- Binding precedence: runtime override > `[[io.bindings]]` in `stc.toml` > `{stc:io}` pragma >
  raw `%` address.
- Unbound `%I` inputs are reset to type defaults (or a declared initializer) each cycle; unbound
  `%Q` outputs are written to the output image and discarded at cycle end; `%M` markers persist.
- v1 runtime backends: `null`, `memory`, `file`. EtherCAT/Modbus/MQTT are v2.
- Pragmas remain non-nested but may carry balanced JSON payloads; the lexer tracks braces and
  JSON strings, and the parser validates the JSON.

### A-012 — RETAIN persistence

**Answer:** File-backed state.

**Consequences:**

- v1 persists retained variables to a versioned state file with a magic/version header, profile
  and layout hashes, payload length, and CRC32.
- A valid file on startup is a warm restart; missing file or `--reset-retain` is a cold restart.
  Layout/profile/corruption mismatch uses `on-mismatch = error|reset` (default error).
- Saves are atomic (`tmp` + rename) at normal shutdown. v1 does not checkpoint every cycle.
- The compiler emits a retained-leaf layout and `stc_retain_size/save/load/layout_hash` functions;
  the runtime owns file I/O and policy. Library hosts may use the functions with their own
  storage.
- References/interfaces/pointers/vtable pointers are not persisted; vtable/type pointers are
  reinitialized before load. Direct `AT` variables are excluded from file state with a warning.

### A-013 — Identifier alphabet

**Answer:** ASCII-only.

**Consequences:** Identifiers use `[A-Za-z_][A-Za-z0-9_]*` with the 2013 underscore rules.
UTF-8 remains supported in comments and string literals. No Unicode normalization or Unicode
case folding is performed; non-ASCII characters outside strings/comments are rejected. This is a
documented subset of the 2013 ISO/IEC 10646 character-set permission.

### A-014 — Comparison precedence

**Answer:** Annex A split.

**Consequences:** Relational comparisons (`<`, `>`, `<=`, `>=`) bind tighter than equality
(`=`, `<>`); both levels are left-associative. `A = B < C` parses as `A = (B < C)`. Chained
comparisons at one level are left-associative and normally type-error. This resolves the
disagreement between the 2013 operator table (one level) and Annex A's formal grammar.

### A-015 — Strict syntax mode

**Answer:** Strict by default.

**Consequences:** v1 requires every statement to be terminated by `;` per the 2013 grammar.
`--relaxed` (or `[build] syntax = "relaxed"`) enables the documented omitted-semicolon-after-
block superset. The formatter always emits strict semicolons. The relaxed mode changes syntax
only, not semantics.

### A-016 — Formatter

**Answer:** Minimal `stc fmt`.

**Consequences:** v1 adds `stc fmt` with one fixed style and `--check`: uppercase keywords,
2-space indentation, normalized operator/comma spacing, strict semicolons, preserved comments
and line breaks, no configuration and no line wrapping. It is idempotent and uses the lossless
CST. A configurable formatter is a later extension.

### A-017 — Task scheduling

**Answer:** Single task only.

**Consequences:** v1 parses and analyses the full configuration/resource/task/program model but
executes one task sequentially per scan. Multiple tasks/priorities are diagnosed or accepted for
analysis but not scheduled. A cooperative or preemptive scheduler is a later milestone.

### A-018 — Test process concurrency

**Answer:** Sequential only.

**Consequences:** `stc test` runs one test process at a time. `--test-threads` is rejected in v1
(or accepted only with value 1). Parallel execution is a later extension; deterministic CI and
simple output ordering take priority.

### A-019 — Lint configuration

**Answer:** Manifest `[lints]`, with CLI overrides.

**Consequences:** `stc.toml` has a `[lints]` table: `warnings = "warn"|"deny"|"allow"` and
per-code `"STC-W-*"`/`"STC-E-*"` entries. Errors cannot be downgraded. CLI `-D`/`-W`/`-A` and
`--deny warnings` override the manifest; defaults apply otherwise. Unknown codes are errors.

### A-020 — Assertions and test-only code

**Answer:** `STC_ASSERT` is test-only. When compiling release, all test code is ignored and
compiled out along with any asserts in it before error checking; asserts outside test code are
errors.

**Consequences:**

- Test-only units are POUs marked `{stc:test}` and sources listed in `[test].sources`.
- Production builds (debug and release) exclude test-only units from semantic analysis, codegen,
  and artifact linking. Release artifacts contain no test POUs and no `stc_rt_assert` calls.
- `STC_ASSERT` resolves only inside test-only units. A production POU using it is
  `STC-E-ASSERT-001`; a production POU referencing a test-only POU is `STC-E-TEST-001`.
- Because test-only units are removed before these checks, asserts inside them do not cause
  production errors.
- `stc test` compiles production sources plus test sources and enables the assert prelude.

### A-021 — Syntax-kind/CST API stability

**Answer:** Stable `SyntaxKind`, with IDE tooling in view for v2/v3.

**Consequences:**

- `SyntaxKind` is a `#[repr(u16)]`, `#[non_exhaustive]` enum with explicit discriminants,
  published from `stc-syntax`. Existing kinds are never renumbered or reused; new kinds are
  appended.
- Kind names and their canonical debug/JSON spellings are stable. A checked-in stability table is
  enforced by a test; additive changes update the table and follow semver.
- The rest of the CST (node handles, traversal internals) remains internal in v1. The design
  keeps spans, trivia, error nodes, and stable kinds so a supported public CST/traversal API can
  be added in v2/v3 for IDE tools.
- `stc-syntax` follows semver for the published `SyntaxKind` surface.

## 2. Confirmed decisions

| ID | Decision | Rationale |
| --- | --- | --- |
| D-001 | Base language is IEC 61131-3:2013 (Ed. 3.0), Structured Text only. | The 2013 PDF is available; Ed. 4 is not. |
| D-002 | Classes, interfaces, inheritance, methods, namespaces, and `USING` are in v1. | Required by the user; all are in the 2013 edition. |
| D-003 | UTF-8 strings are a v1 project extension (`STC-UTF8`). | Required from the start; Ed. 4 text is unavailable. |
| D-004 | Compiler emits textual LLVM IR; `clang` lowers/links to the target. | User requirement; removes LLVM API version coupling; IR is inspectable. |
| D-005 | Both executable and library artifacts are supported via `--emit`. | User requirement. |
| D-006 | Lean proofs cover exact functions; proofs are about the Lean model, and Rust is tested against generated vectors (A-006). | User requirement; transcendentals are not proof-feasible here. |
| D-007 | Primary development host is Windows x86-64 MSVC; targets are selected via clang. Linux x86-64 and AArch64 hosts follow basic development (D-042). | Matches the development environment; Linux support is required later, not during initial development. |
| D-008 | Input is ST only; LD/FBD/SFC/IL and the AX preprocessor are out of profile. | Keeps the front end bounded; AX docs are syntax inspiration only. |
| D-009 | Specifications are original prose/grammar; the IEC PDF is not committed or redistributed. | Copyright compliance; the PDF is a local reference. |
| D-010 | Rust Cargo workspace under `crates/`; spec docs under `Specification/`. | Standard Rust structure; the stray `src` file is removed. |
| D-011 | v1 runtime is single-task; the full configuration model is parsed/analysed. | Bounded runtime scope; language surface remains complete. |
| D-012 | Standard FBs are implemented in ST under `stdlib/`; standard functions live in the runtime. | Dogfoods the compiler; keeps FB behavior in ST. |
| D-027 | The cycle time is latched once at `stc_cycle` entry and shared by all timers in that scan. | Determinism: same-cycle timer calls cannot disagree; tests can step a mock clock. |
| D-028 | I/O hardware linking is runtime-owned; the compiler emits process-image and binding metadata. v1 backends are null/memory/file; EtherCAT/Modbus/MQTT are v2. | Keeps protocol/scheduling out of the compiler; enables the TwinCAT-like project model. |
| D-029 | `AT` declarations accept one `{stc:io <json-object>}` hint; project bindings and runtime overrides take precedence over the pragma. | Local hints plus central device configuration, with runtime authority. |
| D-030 | Unbound inputs receive type defaults (or declared initializers) each cycle; unbound outputs are discarded; `%M` markers persist. | "No linking" has defined, safe behavior rather than undefined reads/writes. |
| D-031 | Pragmas are non-nested but their payload may contain balanced JSON; the parser validates JSON and rejects duplicates/oversize payloads. | Future-proof I/O and tooling metadata without a new language construct. |
| D-032 | `RETAIN` variables persist to a versioned, layout-hashed, CRC-checked state file with atomic saves; warm restart restores, cold start initializes. | File-backed retention without a database or host dependency. |
| D-033 | Identifiers are ASCII-only; UTF-8 is limited to comments and string literals. | Avoids Unicode normalization/case-folding/confusable complexity; matches 2013 examples. |
| D-034 | Comparison precedence follows Annex A: relational binds tighter than equality; both left-associative. | Resolves the operator-table/Annex A disagreement; deterministic parsing. |
| D-035 | Strict 2013 semicolon rules are the default; relaxed syntax is opt-in. | Conformance by default without rejecting common ST source irreversibly. |
| D-036 | v1 includes a minimal `stc fmt` with one fixed style and `--check`, no configuration. | Useful tooling without a style-configuration subsystem. |
| D-037 | v1 executes a single task per scan; the full task model is parsed/analysed. | Bounded runtime scope; scheduler can be added without front-end changes. |
| D-038 | `stc test` runs test processes sequentially; `--test-threads` is not a v1 feature. | Deterministic output and resource use. |
| D-039 | Lint levels live in `[lints]` with per-code entries; CLI flags override them; errors cannot be downgraded. | Shared project policy with CI control. |
| D-040 | Test-only units and `STC_ASSERT` are excluded from production builds; production use of either is an error. | Keeps test code and assertions out of shipped artifacts. |
| D-041 | `SyntaxKind` is a stable, append-only, `#[non_exhaustive]` API; full CST/IDE API is planned for v2/v3. | Enables future tooling without freezing CST internals now. |
| D-013 | Runtime ABI: `stc_init`, `stc_cycle`, `stc_shutdown`, plus `stc_rt_*` helpers; ABI version 1. | Stable C boundary for exe and host embedding. |
| D-014 | Strings are length-prefixed fixed-capacity values. `USTRING[n]` stores `n` characters as 32-bit scalars (4 bytes each, static); default 255 characters. | Bounded storage, O(1) character indexing, room for every Unicode scalar. |
| D-015 | Signed overflow, division by zero, bounds, and invalid conversions trap. | "Strict correctness" policy; no silent wraparound. |
| D-016 | Boolean `AND`/`&`/`OR` short-circuit left-to-right. | Deterministic and standard-permitted. |
| D-017 | Relational comparisons bind tighter than equality; both are left-associative. | Follows Annex A's formal grammar (A-014). |
| D-018 | Identifiers are ASCII-only in v1. | 2013 examples and practical tooling; resolved by A-013. |
| D-019 | Strict 2013 semicolon rules are the default; `--relaxed` enables the optional-semicolon-after-block superset. | Conformance by default, with an explicit compatibility mode. |
| D-020 | Project file is `stc.toml`, schema version 1. | Explicit profile/artifact/test configuration. |
| D-021 | Build artifacts and local reference PDF are gitignored. | Reproducibility and copyright hygiene. |
| D-022 | Diagnostics use stable codes (`STC-E-*`, `STC-W-*`) with spans and JSON output. | Cargo-like UX and testability. |
| D-023 | `ARRAY [*]` is supported only as conformant parameters (`VAR_IN_OUT`, FB inputs/outputs); no dynamic locals/globals/returns in v1. | Bounded ABI and lifetime scope; caller owns the storage. |
| D-024 | v1 includes a project dependency system with local path dependencies, a dependency graph, and package-aware namespaces. | Separate test projects require it; source packages compile together. |
| D-025 | Tests are declared with `{stc:test}` and may live in a separate test project that depends on the code under test. | Keeps test code out of production packages while sharing the language tooling. |
| D-026 | Dependency resolution is local paths plus a committed `stc.lock`; `--locked` forbids updates; no registry/network/version solver in v1. | Reproducible multi-package builds without a registry subsystem. |
| D-042 | After basic development, `stc` and its runtime also build and run natively on Linux x86-64 and Linux AArch64 (ARM64). Windows remains the primary development host; target selection stays with clang. | User requirement. Rust plus the narrow C ABI make the port additive if host-specific code stays behind a small platform layer. |
| D-043 | M1 lexical implementation limits: identifiers up to 1024 bytes; invalid UTF-8 replaced by U+FFFD for recovery; initial BOM as trivia; same-line trailing trivia; unterminated block comments as trivia plus a zero-width `Error` token; duration fractions contain digits only; date/time fractions have 1–9 digits; direct-variable indices are 32-bit. | Implements the implementation-defined latitude in STC-LEX-002, -016, -038, -041, -043, and -049 without changing the profile. |
| D-044 | Typed string and character prefixes determine the encoding and accept either quote delimiter; `$'` and `$"` are accepted in any quoted literal; an unquoted `UCHAR#` accepts one named, `$HH`, `$HHHH`, or `${HEX}` escape; `$L`/`$N` both denote U+000A. | Makes the STC-LEX-027, -030, and -035 examples explicit and removes delimiter/encoding coupling; each choice has a lexer test. |

D-042 is a host-portability requirement, not a language/profile change. It does not alter the IR,
runtime ABI, or conformance profile. It requires host-specific operations (clang/linker
discovery, artifact naming, path semantics, monotonic clock, atomic file replace) to be confined
to the driver and runtime. Affected documents: [Architecture](Architecture/Architecture.md) §6.4,
[Runtime](Architecture/Runtime-and-Execution.md) §2, [State](Architecture/State-and-Retain.md) §4.2,
[Testing](Architecture/Testing-and-Verification.md) §6, and the [Project Plan](00-Project-Plan.md)
M8.

## 3. Open questions

**None as of 2026-10-23.** All v1 planning questions Q-001 through Q-017 are resolved
(A-005 through A-021). New questions will be appended here and in `README.md` one at a time, with
a proposed default so implementation can continue without blocking.

## 4. Decision process

- New decisions get the next `D-NNN`; superseded decisions are kept with a `Superseded by`
  line, not deleted.
- Questions get the next `Q-NNN`; answered questions move to §1 with an `A-NNN` entry and a
  summary in `README.md`.
- Each decision records the reason and the affected specification sections.
- A decision that changes the language surface also changes the conformance profile and adds a
  test ID.
