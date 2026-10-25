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

- The host latches `cycle_now_ns` once at `stc_module_cycle` entry. All
  `TON`/`TOF`/`TP`/counter/edge/date-time operations in that cycle read the latched value through
  the host API `cycle_now_ns` service.
- The live monotonic clock is separate (the host API `monotonic_now_ns` service) and is not
  visible to ST code.
- Library hosts may set the next cycle's time for deterministic stepping; tests use a mock clock
  with the same semantics.
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

### A-022 — Online change is a v1 requirement

**Answer:** Yes. The v1 execution model is a persistent host runtime plus loadable whole-program
modules, not per-function hot patching. `module` becomes a first-class artifact paired with a GOT
manifest; `exe` is a thin host launcher that loads an adjacent module; `lib` remains the
embedding artifact. Online change is delivered through the host API and local CLI, is a runtime/
platform capability rather than an IEC language feature, and is supported on Windows MSVC x64
and Linux GNU x64 (AArch64 with M8). The foundation is M4.5, inserted before M5; the user-facing
workflow is M6.

**Consequences:** The direct-reference, code-linked M4 artifact model is superseded. The host
owns the cycle, persistent memory, modules, the process image, retained state, and debug
services. No code-linked executable deployment path is maintained for online-change deployments.
M4 artifacts are explicitly non-loadable under host ABI 2. The project is **not a safety PLC**
and is not responsible for safety in any way; this must be stated wherever online change or debug
behavior is described.

**Questions:** Q-018–Q-025, Q-089–Q-095.

### A-023 — GOT, memory ownership, and root granularity

**Answer:** Every access that can outlive or cross a module version is indirected through a
project-defined GOT: persistent roots, user/standard function calls, stored function pointers, and
vtable/type descriptors. Stack, locals, parameters, compiler helpers, intrinsics, and module-owned
read-only data stay direct. The host owns a shared cell table; each module has a local slot table
mapping to host cells. Generated code never writes a cell; only the host does. The host allocates
stable, zeroed storage for persistent roots, and storage never moves.

Persistent aggregates use **recursive per-field roots**: every persistent field has its own
`GotId`, cell, storage, type identity, and manifest record. Objects use stable opaque handles plus
runtime object records and descriptors. Arrays of scalars/strings are one root with GEP indexing;
arrays of aggregates use per-element handles whose fields are rooted recursively. Strings are one
root per value; `OVERLAP` is one atomic root with aliasing typed views. Removed roots are
quarantined until restart or explicit reset. Direct `AT` variables use one host cell per variable
or field per generation (Q-033, non-default).

**Consequences:** Field add/remove/reorder becomes preserve/drop rather than a whole-object layout
change. Internal dynamic allocation is transient and module-version-owned; it cannot be stored in
persistent roots or carried across swaps. User-visible `__NEW`/`__DELETE` is a v2 extension.
Quarantine is a deliberate memory-growth tradeoff.

**Questions:** Q-026–Q-044, Q-120–Q-130, Q-212.

### A-024 — Persistent references and `POINTER TO`

**Answer:** There are no persisted raw references or pointers. `REF_TO` values are validated
**place descriptors**: `Null`, `Root {got_id, offset, type_key}`, `Direct {direct_id | area,
offset, type_key}`, `Stack {frame_id, offset, type_key}`, or `Object {handle, descriptor}`.
`REF(place)` produces the same descriptor directly. Pointer→`REF_TO` conversion is validated by
the host resolver against active roots/process-image ranges and the manifest layout; an invalid
cast yields `NULL`, and dereferencing `NULL` traps. `REF_TO`→interface is allowed only for
class/FB object references with a descriptor check; interfaces are nullable and null/incompatible
conversions yield null. Pointer→object/interface is disallowed.

`POINTER TO` is a distinct transient-only v1 extension (`STC-POINTER`), not IEC `REF_TO`. It
supports full typed arithmetic, indexing, and `SIZEOF` for contiguous values, pointer parameters
and results, null-dereference traps, and documented dangling-pointer behavior. Persistent pointer
fields are rejected. References and pointers are not persisted in the v1 state file; a later
state-format revision may persist `Root`/`Direct` descriptors.

**Consequences:** Reference values are host-resolvable descriptors, not raw addresses. Dereference
costs a descriptor resolution (optionally cached with a generation check). `POINTER TO` is an
explicit vendor profile feature; no IEC conformance claim is made for it.

**Questions:** Q-131–Q-173.

### A-025 — Module lifecycle and the real-time swap boundary

**Answer:** A module is a whole shared library with a descriptor and attach/prepare/cycle/
shutdown/detach entry points. The host loads the new module and prepares it on a background
loader/prepare thread while the old generation continues to run. Activation uses double-buffered
normal/temp GOTs: the host builds a temp GOT, switches to it at a cycle boundary, retains the
previous generation for one successful cycle, then commits and unloads off-thread after a grace
period. The switch is O(1); heavy validation, copying, and migration happen off-thread. The old
version completes its outputs and publishes state before the switch; fresh inputs are latched at
the next cycle start. Task/resource topology changes require a cold download. The clock remains
continuous and the watchdog is paused or extended with a bound.

**Consequences:** The cycle thread only checks and switches. Loader and service threads never
execute ST and never block the cycle. A failed prepare or a trap in the rollback window restores
the previous generation. Rollback is recoverable only within the one-successful-cycle window;
after commit, reverting is another online change or a cold download.

**Questions:** Q-054–Q-076, Q-174–Q-182.

### A-026 — Persistent GOT manifest, identity, and baseline policy

**Answer:** Every module is paired with a persistent GOT manifest in canonical JSON (tooling,
download) and a compact binary projection (runtime), generated from one model. The manifest
carries header/lineage, identity records, layout/field records, function/type/instance/task/
export records, the online-change plan, current slot mappings/cell kinds, and integrity digests.
`GotId`s are content-derived 128-bit identities from scope/name/kind (type/layout excluded for
data so layout changes stay trackable; signature included for functions); collisions are checked.
Tracked renames/moves keep the old `GotId`; new declarations get new IDs; removed entries retire;
a reintroduced retired key gets a new ID unless explicitly reused. Per-module slot numbers stay
local and map to `GotId`s.

A minimal module descriptor remains the source of code addresses, sizes, entry points, and the
paired digest; descriptor and manifest are two projections of one model. A valid baseline is
required for online change. Exact currentness means the manifest parent build ID equals the
active build ID, digests pair, and profile/target/ABI/layout identity match. Missing, invalid,
unpaired, or non-current baselines require a **cold download**; no ancestor matching, merge, or
rebase is attempted. V1 has no field-layout migration: field type changes, resizing, and
array/string/`OVERLAP` layout changes reset the affected root or require cold. The compiler emits
a pre-deployment compatibility report; the runtime stores the active manifest and can export it.
Retained-state migration across restarts is not in v1; a later state format may key leaves by
`GotId` plus path. Manifest size/cost is measured, the active record is cached, and an optional
later scaffold may emit declarations (never bodies) from a downloaded manifest.

**Consequences:** Losing or corrupting the active manifest costs a cold restart. The manifest is
untrusted parsed data: validate every field/digest, access-control download, never execute its
content. A second build from an older baseline also requires a cold download.

**Questions:** Q-096–Q-119.

### A-027 — Compatibility, preservation, and state

**Answer:** The host owns the process image, retain buffer, task guards, clock, counters, export
registry, and diagnostics. Online attach preserves every matching root whose `GotId`, type key,
layout hash, and size match; new roots are initialized once; removed roots are quarantined and
recorded as retired. Initializers apply only to new roots; changed initializers on preserved roots
are ignored. Online attach skips the retained-state file load because in-memory state wins; the
state file keeps its existing layout-hash policy for restarts. A cold download initializes
non-retained state, restores only compatible `RETAIN` values, and does not preserve transient
allocations. Per-field roots already make field add/remove/reorder preserve/drop; no field-layout
migration is in v1. Incompatible changes reject the module by default; an explicit per-field reset
is allowed, and baseline problems force cold.

**Consequences:** Compatibility is conservative and explicit. The compatibility report lists
preserved/allocated/dropped/renamed/migrated/rejected instances before deployment.

**Questions:** Q-045–Q-055, Q-073–Q-077, Q-118–Q-119.

### A-028 — OOP descriptors, host API, and standard library across swaps

**Answer:** Type descriptors are runtime-owned; vtable cells contain cell pointers, not raw code.
Object and interface values store `{handle, descriptor}`. Preserved interfaces survive compatible
swaps because the descriptor is stable; a vtable shape change is rejected while any live instance
or interface depends on it. The host API is a versioned struct (`abi_version` + `struct_size`,
additive within a major). The export registry is host-owned by C name/signature cell. Standard
library code is compiled per module under `STC_STD` with stable keys. User functions, methods, FB
bodies, exports, and standard FBs are registered; compiler helpers are not.

**Consequences:** Module-owned vtables and descriptors are forbidden. Host ABI 2 replaces M4's
ABI 1; no M4 artifact is loadable.

**Questions:** Q-056–Q-068, Q-092.

### A-029 — Tooling, diagnostics, determinism, and loader security

**Answer:** The CLI adds module emission, GOT manifest/cold/rename options, run/watch, online
download/diff, and debug options. The project manifest gains `[runtime]`, `[online]`, and
`[debug]` tables. Diagnostics get stable `MODULE`/`MANIFEST`/`ABI`/`ONLINE`/`W` families. The host
retains per-module span/source metadata for diagnostics. Determinism requires sorted keys/slots,
content-derived build IDs, byte-identical double builds, and no paths/timestamps. Loading trusts
local modules but validates descriptors strictly; an optional build-ID/hash pin is available.
Fuzzing covers `fuzz_codegen` plus descriptor/manifest parsers without executing arbitrary native
modules. Download is local in v1 (no network deployment protocol).

**Consequences:** Manifest download reveals program structure and is access-controlled. The cold
path is a first-class, tested feature.

**Questions:** Q-067–Q-069, Q-078–Q-088, Q-100–Q-116.

### A-030 — Non-real-time services, full debug, and the non-safety stance

**Answer:** The runtime has three thread classes: the cycle thread, the loader/prepare thread, and
lower-priority service/monitor threads. Service readers use generation pinning/epoch refcounts;
status uses lock-free counters, double-buffered snapshots, and bounded rings; writes and forcing
are queued for cycle-boundary application. V1 includes full typed variable read/write/force/watch,
codegen breakpoint checkpoints, stop-the-world stepping, call-stack unwinding, DWARF/CodeView,
and a manifest-assisted split-object view. Every online swap ends the debug session and clears
breakpoints/forces. Debug information is the default build output; `--release` is the optional
stripped/optimised mode with distinct build IDs/manifests. Debug may pause the watchdog and hold
outputs indefinitely. The project is not a safety PLC and assumes no safety responsibility.

**Consequences:** Debug/release artifacts are distinct. Debug hooks are absent in release.
Starting/stopping a session has documented effects on outputs and timing. Non-RT threads must not
cause unbounded locks, allocator contention, or memory-bandwidth stalls in the cycle.

**Questions:** Q-183–Q-211.

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
| D-027 | The cycle time is latched once at cycle entry and shared by all timers in that scan. | Determinism: same-cycle timer calls cannot disagree; tests can step a mock clock. |
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
| D-013 | Runtime ABI: `stc_init`, `stc_cycle`, `stc_shutdown`, plus `stc_rt_*` helpers; ABI version 1. Superseded by D-094 (host ABI 2). | Stable C boundary for exe and host embedding; retained as historical rationale. |
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
| D-045 | The M2 CST is a hand-rolled immutable arena that owns `Arc<str>`; node spans include attached trivia and typed diagnostics use token-only extents. | Keeps the workspace's CST dependency-free and lossless; `rowan` remains an allowed alternative. |
| D-046 | `SyntaxKind` places token kinds in `1..=0x0FF` and node kinds from `0x100`, with explicit discriminants and a checked-in name/discriminant table. | Stable append-only surface for formatter/IDE tooling (A-021). |
| D-047 | Ordinary parser diagnostics use `STC-E-GRAM-105`–`111` (unexpected token, expected token, missing semicolon, mismatched terminator, depth limit, invalid pragma placement, missing terminator). | Gives every recovery path a stable code; Grammar §13.1 documents the table. |
| D-048 | Unsupported constructs are recognized by shape: SFC keywords, a leading `#`, Ed. 4 declaration names (`PROPERTY`/`MUTEX`/`SEMA`), an `ASSERT(...)` call statement, and IL mnemonics followed by an operand. The same spellings remain usable as ordinary identifiers elsewhere. | A deliberate lookahead policy that rejects the out-of-profile constructs without rejecting valid identifiers. |
| D-049 | Duplicate `stc:io` on one declaration is `STC-E-IO-003`; `stc:io` without `AT` is `STC-W-IO-002`. Other duplicate `stc:` names are preserved without a new error code. | IO §4 already assigns these codes; a parallel pragma code would duplicate the contract. |
| D-050 | Pragma payloads use `serde_json` with a custom `DeserializeSeed` that rejects duplicate keys and nesting deeper than 32; bodies over 64 KiB are rejected before parsing; unknown keys are preserved. The `stc:` prefix and known names are matched ASCII case-insensitively and normalized to lowercase. | Strict JSON without losing unknown metadata; matches the language's case-insensitive convention. |
| D-051 | The M2 CLI parses arguments manually and supports only `stc parse <file>` and `stc check <file>` on single files. `parse` prints the CST/AST dump in human mode and suppresses it in JSON mode so stdout stays one JSON object per line. | Keeps the M2 skeleton small; `clap`, directories, and project manifests are deferred to M6. |
| D-052 | Simple dotted access (`a.b`) is a `PathExpr` with multiple `Name` segments; `MemberExpr` is reserved for access on a computed base (`f().x`, `arr[0].x`). Typed AST text accessors and diagnostics use token extents, not trivia-inclusive node spans. | One uniform representation for qualified names, and diagnostics that point at code rather than comments. |
| D-053 | Missing tokens are zero-width token leaves with a diagnostic. A mismatched `END_*` reports both spans and is left for the enclosing construct; a stray statement terminator at POU-body level is consumed and parsing continues. | Preserves losslessness, keeps the enclosing terminator available, and avoids recovery cascades. |
| D-054 | Empty `StmtList` nodes are valid; the AST §3.4 "at least one child" invariant is relaxed for them. | `(stmt? ';')*` permits an empty body, so an empty list is a real node. |
| D-055 | `AssignStmt` versus `RefAssignStmt` is classified syntactically: a dereference on the left or a `REF(...)`/`NULL` on the right selects the reference form; the analyzer finalizes the meaning. | Preserves both shapes for M3 without speculative typing; a bare `x := y` stays an assignment. |
| D-056 | Grammar §4 now defines `initializer`, `array_init`, `struct_init`, `struct_field_init`, and `char_str`. | The draft referenced these productions without defining them. |
| D-057 | The grammar's duplicated construct wrappers are represented by one CST node each: `Path` for qualified names, `TypeRef` for type specs, `ParamList`/`Param` for call arguments, and `ElseBranch` only for `ELSIF`. | Avoids redundant node kinds while keeping every production reachable from the typed AST. |
| D-058 | Identifier interning is deferred to M3. M2 preserves each identifier's original spelling and the lexer's normalized uppercase key in `Name` nodes; the session `Interner` is built during name resolution. | The lexer already provides the normalized key; a second symbol table in M2 would be unused until M3. |
| D-059 | M3 produces HIR: `stc-hir` owns arenas, IDs, the interner, the type arena, and resolved bodies; `stc-sema` lowers the typed AST into it. `stc-codegen` will consume HIR only. | The AST spec places HIR after type checking and M4 declares HIR as its input; producing it now stabilizes the contract. |
| D-060 | The M3 multi-file bridge is `stc check <file>...`. Paths are sorted lexicographically before `FileId` assignment; all files form one in-memory package (`PackageId(0)`) with session root namespace `_root`. | Multi-file namespaces must be testable before `stc.toml` (M6); sorting makes diagnostics and IDs independent of CLI order. |
| D-061 | `Interner`/`Symbol` live in `stc-hir`; the analyzer session owns the interner and the resolution map. Symbols are looked up by uppercase ASCII key and retain the first original spelling. | One intern table shared by HIR and sema, with case-insensitive lookup and stable IDs. |
| D-062 | The M3 diagnostic table in `M3-Plan.md` §7 is stable. `STC-E-TYPE-012`, `STC-W-NAME-002`, parser, pragma, I/O, and retain codes keep their existing meanings. | Diagnostics are snapshot-tested contracts; codes are never renumbered. |
| D-063 | Type canonicalization: aliases are transparent; arrays, refs, strings, and inline structs are structural keys in the type arena except that inline structs receive anonymous nominal defs; enums, subranges, named-value types, named structs, classes, FBs, interfaces, functions, and programs are nominal (`Ty::Named(DefId)`). | Gives stable field IDs for structs/classes while keeping array/string/ref assignment simple. |
| D-064 | Conversion matrix: implicit for int↔int, real↔real, int→real, bit-string same category, same-kind strings/chars, TIME→LTIME, subrange↔integer base with a bounds check, enum/named-value exact type, derived→base class/FB, class/FB→implemented interface, interface widening, ref covariance, and `NULL`; real→int, enum↔integer, cross-kind strings/chars, bit-string narrowing, and downcasts require an explicit conversion or `?=`. `BOOL` is never numeric. Duration arithmetic: `TIME ± TIME` yields the wider of the two, `TIME * numeric`/`numeric * TIME`/`TIME / numeric` yield the duration, and other mixed duration/numeric operations are rejected. | A documented matrix keeps checking predictable; runtime range/trap checks arrive with codegen (D-015). |
| D-065 | Overload resolution ranks candidates by exact matches, then implicit conversions, then literal adaptation; a unique best candidate wins, otherwise `STC-E-CALL-004`. Named arguments must precede no positional arguments after them; output bindings use `=>` and must target a place. | Deterministic rule that is easy to test and explain. |
| D-066 | The standard prelude contains elementary types, a signature table for standard functions, standard FB signatures (bodies arrive with the M5 stdlib), and algorithmically recognized `X_TO_Y` conversion functions. Standard names are shadowable with `STC-W-NAME-002`. | M3 needs names and types, not implementations; the M5 stdlib replaces signature-only FBs. |
| D-067 | Constant evaluation uses checked `i128` integer arithmetic and `f64` reals. Overflow, division by zero, invalid bounds, and non-constant initializers produce `STC-E-CONST-001/002`; aggregate and `REF` initializers are lowered structurally rather than folded. | Constant evaluation must be deterministic and must not produce unchecked values. |
| D-068 | Access defaults: class/FB variables are `PROTECTED`, methods are `PUBLIC`, top-level declarations are `PUBLIC`. `PRIVATE` is declaring-type-only, `PROTECTED` includes derived types, `INTERNAL` is package/subtree-private; with one M3 package `INTERNAL` never fails across packages. | Matches Grammar §6 and defers package boundaries to M6. |
| D-069 | References: `REF(place)` requires an addressable non-constant place; `NULL` is assignable to any reference/interface/class; `^` requires a reference; `?=` requires a class/interface/reference destination and a related source type; `:=` between reference types is a reference assignment, and `p^ := v` is an ordinary assignment through the dereference. | Implements D-055 finalization and the reference/assignment-attempt rules in AST §7. |
| D-070 | `stc check` skips semantic analysis when lexing or parsing produced any error. Recovered trees are not analyzed in M3. | Avoids cascades from missing tokens and keeps diagnostic output actionable. |
| D-071 | Production `stc check` excludes `{stc:test}` POUs from resolution and lowering and reports references to them as `STC-E-TEST-001`. `stc test`, assert enablement, and full A-020 timing remain M6; the parser continues to reject `ASSERT(...)` as `STC-E-GRAM-104`. | Implements the production-exclusion rule without pulling the test runner forward. |
| D-072 | Collection and lowering are deterministic: files in `FileId` order, declarations in source order, members sorted only where the spec requires (vtables), and no hash-map iteration in output paths. | Stable IDs, diagnostics, snapshots, and future IR. |
| D-073 | HIR expressions are arena-allocated (`ExprId`) and left-associative binary chains are lowered iteratively from the left spine. | A 20k-term expression must not overflow the stack (M2 guarantees parsing it). |
| D-074 | `stc:io` payload-shape errors (`stc:io` without an object, non-object payload, invalid known-key types) are reported as `STC-E-IO-004` by the pragma parser. Other pragma payload-shape errors remain `STC-E-PRAGMA-002`. | Reconciles the IO code table with the parser's earlier shared pragma code (D-049); the IO-specific code is stable for users and tests. |
| D-075 | M3 computes the effective `RETAIN` policy per declaration, diagnoses invalid locations and non-persistable aggregates, and records the policy in HIR. Full `RetainLayout` offsets, persistable-leaf enumeration, and the retained-state ABI are emitted with codegen in M4. | The handover requires at least the diagnostics and effective policy in M3; offsets depend on the target layout, which is an M4 concern. |
| D-076 | `stc-codegen` consumes only `HirCrate` and `stc_span::Span`. The API is `generate(&HirCrate, &Options) -> Result<Output, Vec<CodegenError>>`; `Options` carries artifact name, target, mode, debug, metadata, and source file names. Tests may use parser/sema as dev-dependencies; production dependencies stay `stc-hir`/`stc-span` plus std. | Keeps the IR contract independent of syntax and semantic internals, so the driver can reuse one HIR for dumps and codegen. |
| D-077 | `stc-runtime` is a Rust `staticlib` with a C ABI. It uses `std` with a small surface, catches panics at the ABI boundary, and is the only workspace crate allowed `unsafe_code`; the exception is a narrow crate-level `allow(unsafe_code)` documented in its manifest. Edition-2024 `#[unsafe(no_mangle)]` and `unsafe extern` are used. | A std staticlib is simplest to build and debug; all FFI stays in one crate with a recorded exception to the workspace lint. |
| D-078 | Linking uses clang plus the Rust staticlib. On Windows MSVC the driver discovers the MSVC `lib/x64` and highest Windows SDK `ucrt`/`um` x64 directories and passes `-Xlinker /LIBPATH:...` plus `ws2_32`, `ntdll`, `userenv`, `advapi32`, `bcrypt`, `ole32`, `shell32`, `user32`, and `dbghelp`. On non-Windows hosts the archive is linked directly. The spike proved this path with LLVM 23.1.4. | Rust std staticlibs require the MSVC CRT and several system libraries; discovering them avoids requiring a `vcvars` shell and keeps CI reproducible. |
| D-079 | M4 adds `stc build <file>...` as a manifest-free bridge: default emit `ir`, default name the sanitized first sorted file stem, default out-dir `target/stc/debug`. M6 replaces name/emit/out-dir defaults with `stc.toml` values. | Enables artifact and execution testing now without pulling the M6 manifest forward. |
| D-080 | The M4 supported surface is the handover §2 table: elementary scalars, scalar operators/casts, checked arithmetic, scalar functions/programs/globals, `AT` scalars, process image, executable plus static-library modes, and scalar retained leaves. Every other HIR construct is rejected with `STC-E-CODEGEN-001` (or `STC-E-RUN-001/002` for configuration issues). | Explicit deferral diagnostics are safer than emitting IR with unknown semantics. |
| D-081 | IR is textual, uses opaque pointers, omits datalayout always and the triple unless `--target` is given, emits functions/globals/declarations in sorted mangled-name order, uses monotonic `%tN`/`bbN` counters, and formats floats in hexadecimal. Snapshot normalization does not hide real IR differences. | Deterministic, target-independent modules that LLVM 15+ accepts. |
| D-082 | Names use `stc_fn_<ns>_<name>_<hash>`, `stc_g_<ns>_<name>`, `stc_pi_<instance>`, `stc_prog_<type>_body`, and `stc_export_<name>`. Namespace segments join with `_n_` and the root `_root` is omitted. Identifiers use uppercase normalized text with non-C characters escaped as `_uXXXX_`. Signature hashes are FNV-1a 64 over canonical parameter/result type strings from a codegen-owned canonicalizer, never display names or arena IDs. | Stable symbols independent of collection order, with distinct overloads. |
| D-083 | M4 selects exactly one configuration; with no configuration it synthesizes `_default` when exactly one program exists. Zero programs or multiple configurations are `STC-E-RUN-002`. At most one task per resource is accepted (`STC-E-RUN-001`); every task runs each cycle, with a generated first-cycle guard for `SINGLE`. Task attributes are metadata only. | Deterministic execution without a manifest or real scheduler, and no silent selection. |
| D-084 | The process image is contiguous `%I`/`%Q`/`%M`, sized `ceil(max end bit / 8)`. `AT` variables have no ordinary storage. The runtime allocates the image through `stc_rt_io_configure(input_len, output_len, marker_len)` plus `stc_rt_io_image`; generated `stc_init` caches the base pointer and writes input defaults once (not each cycle). M6 owns per-cycle backend precedence and device bindings. | An additive ABI with no maximum image size, and a tested M4 null-backend behavior. |
| D-085 | `stc-hir` gains `IoHint { device, channel, direction, payload, span }` attached to `FieldHir` and `DefKind::Global`. `payload` is the parser-validated canonical JSON text (sorted keys). Sema fills hints from `VarDecl::pragmas()`; unknown devices remain warnings for M6. | Process-image metadata needs the hint without codegen reaching into the syntax tree. |
| D-086 | `Def.export_name: Option<Symbol>` records the final C symbol. In M4 `{stc:export}` is valid only on `FUNCTION`; `{stc:no_mangle}` requires export, must be a C identifier, must not start with `stc_` or `llvm.`, and must be unique across exports, globals, and reserved names. Otherwise `STC-E-EXPORT-001`. Non-no-mangle exports use `stc_export_<normalized>`. Library modules export `stc_init`, `stc_cycle`, `stc_shutdown`, `stc_metadata_json`, export wrappers, and the retain functions; `stc_abi_version` stays runtime-owned because the runtime already defines it and a generated duplicate would fail to link. | Codegen must not re-spell or re-validate exported names, and the link must not contain duplicate ABI symbols. |
| D-087 | M4 computes a scalar-leaf retain layout from retained globals and retained program-instance fields, excluding `AT` leaves. Paths use instance/field source spelling sorted by uppercase path; offsets are cumulative sizes; the hash is FNV-1a 64 over newline-free `path\|type\|offset\|size` records joined by `\n`. Aggregates/strings/references are `STC-E-CODEGEN-001` until M5; file-backed state remains M6. | Implements the D-075 scalar subset honestly and leaves the aggregate/file work explicit. |
| D-088 | `TaskHir` carries folded `single_value: Option<bool>`, `interval_ns: Option<i128>`, and `priority_value: Option<i128>`. Sema folds checked scalar HIR constants (literals, casts, unary, arithmetic, and constant globals); non-constant or mistyped attributes remain diagnostics. | Codegen and metadata never parse or evaluate syntax. |
| D-089 | Date/time units follow the conformance profile: `DATE` is days since 1970-01-01; `LDATE`, `TOD`, `LTOD`, `DT`, and `LDT` are nanoseconds; `TIME`/`LTIME` are nanoseconds. DATE↔LDATE conversions multiply/divide by 86,400,000,000,000 with checked range, and LDATE literals are encoded in nanoseconds. | Removes the M3 inconsistency where LDATE literals carried days. |
| D-090 | Codegen tests live in `crates/stc-codegen/tests` and may dev-depend on parser/sema. Clang verification runs when clang is available and is mandatory when `STC_REQUIRE_CLANG=1`; skips print a message. Execution tests live in `crates/stc-driver/tests` and build the runtime with `cargo build -p stc-runtime --locked` once per test process, locating the artifact from the test binary's target directory. Fuzz smoke coverage includes `fuzz_codegen`. | Tests remain runnable with `cargo test --workspace` while CI can require the toolchain. |
| D-091 | Emission uses no hash-map iteration, no absolute paths/timestamps/host names, counters reset per module, hex floats, and a generate-twice byte-equality test. Metadata JSON keys are sorted. | Reproducible IR and metadata are an M4 exit criterion. |
| D-092 | The M4 diagnostic table is `STC-E-CODEGEN-001/002`, `STC-E-BUILD-001`–`004`, `STC-E-EXPORT-001`, and the reused `STC-E-RUN-001/002`. Toolchain diagnostics carry no fake source span; codegen diagnostics carry HIR spans. | Stable error contract for build automation and tests. |
| D-093 | Codegen runs a definition-level preflight over all non-synthetic definitions and rejects unsupported M5 constructs even when unused; synthetic prelude FBs (empty span, no body) are skipped. One definition-level feature produces one error, not a per-expression cascade. | Prevents emitting an artifact that silently omits part of the program while keeping diagnostics actionable. |

### Online-change, manifest, and debug decisions

| ID | Decision | Rationale |
| --- | --- | --- |
| D-094 | The v1 execution model is a persistent host plus loadable whole-program modules; `module` is a first-class artifact paired with a GOT manifest, `exe` is a host launcher, and `lib` embeds the host. Host ABI 2 replaces ABI 1; M4 artifacts are not loadable. | Whole-module replacement is the only tractable safe hot-swap model; no per-function patching. |
| D-095 | All persistent roots, user/standard calls, stored function pointers, and vtable/type descriptors go through a host-owned GOT; generated code never writes cells. Stack, locals, helpers, intrinsics, and read-only module data stay direct. | A stable indirection boundary is required for unload safety; host-only writes keep cell state authoritative. |
| D-096 | Persistent aggregates use recursive per-field roots with opaque handles and runtime object records; scalar/string arrays are one root, aggregate arrays use per-element handles, strings are one root, and `OVERLAP` is one atomic root. | Field add/remove/reorder becomes preserve/drop; handles keep object identity stable without contiguous-object assumptions. |
| D-097 | No persisted raw references or pointers. `REF_TO` uses place descriptors (`Root`/`Direct`/`Stack`/`Object`); pointer→`REF_TO` is validated by the host resolver and yields `NULL` on failure; `REF_TO`→interface uses a descriptor check; interfaces are nullable. `POINTER TO` is a distinct transient-only `STC-POINTER` extension with full arithmetic/indexing/`SIZEOF` and no persistent fields. | Makes references remappable and type-safe across swaps while keeping a useful transient pointer surface. |
| D-098 | A module exports attach/prepare/cycle/shutdown/detach plus a descriptor; the host owns descriptors, vtables, exports, and the export registry. Vtable shape changes are rejected while live instances/interfaces depend on them. | Module-owned descriptors would dangle after unload; host ownership makes preserved OOP values safe. |
| D-099 | Activation uses double-buffered normal/temp GOTs, a background loader/prepare thread, an O(1) cycle-boundary switch, a one-successful-cycle rollback window, and off-thread unload. Heavy work never runs on the cycle thread. | Preserves real-time behavior and gives a bounded, recoverable rollback window. |
| D-100 | Every module is paired with a persistent GOT manifest in canonical JSON and compact binary form; identities are content-derived 128-bit `GotId`s with tracked rename/move history and per-module slot mappings. A minimal module descriptor remains the code-address source; descriptor and manifest are two projections. | Identity must survive edits, reordering, renames, and cross-developer builds without making the runtime parse one giant format. |
| D-101 | Online change requires an exact current baseline (parent build ID equals active build ID, digests pair, profile/target/ABI/layout match). Missing, invalid, unpaired, or stale baselines force a cold download; no merge/rebase/ancestor matching. | Turns a large class of recovery problems into one predictable, testable operation. |
| D-102 | Preservation is per matching `GotId`/type/layout/size; new roots initialize once; removed roots quarantine and retire; no field-layout migration in v1; incompatible changes reject the module or require an explicit reset, and baseline problems force cold. Retained-state migration is not in v1. | Conservative by default; per-field roots already cover add/remove/reorder without allocating migration risk. |
| D-103 | The runtime adds loader/prepare and lower-priority service/monitor threads; readers use generation pinning/epoch refcounts, status uses lock-free/double-buffered/bounded structures, and writes/forcing are queued for cycle boundaries. | Keeps non-RT work off the cycle while keeping preserved data alive and diagnostics lock-free. |
| D-104 | V1 includes full typed debug/monitoring: read/write/force/watch, breakpoint checkpoints, stop-the-world stepping, call stacks, DWARF/CodeView, and a manifest-assisted split-object view. Every online swap ends the debug session and clears forces/breakpoints. | Full debug is a v1 requirement; session teardown prevents stale code/descriptor references. |
| D-105 | Debug information is the default build output; `--release` is the optional stripped/optimised mode with distinct build IDs/manifests and sidecar symbols. Debug hooks are absent from release artifacts. | Debuggability by default without conflating debug and release artifact identities. |
| D-106 | `stc` is not a safety PLC and is not responsible for safety in any way. Debug may pause the watchdog and hold outputs indefinitely; no automatic fail-safe is provided. | Explicit scope boundary; safety requires an independent qualified design and certification. |
| D-107 | Online-change foundation is M4.5 before M5; full debug is M6.5 between M6 and M7; M8 covers Linux host parity including module loading, threads, and debug info. | Front-loads the architecture before M5 features depend on it, and keeps debug and portability independently testable. |
| D-108 | The CLI and project manifest gain module emission, GOT manifest/cold/rename options, `stc online download/diff`, run/watch, `[runtime]`/`[online]`/`[debug]` tables, and stable `MODULE`/`MANIFEST`/`ABI`/`ONLINE`/debug diagnostics. Manifest data is untrusted, validated, and access-controlled. | A usable, auditable workflow and a stable automation surface without a network protocol in v1. |
| D-109 | M4.5 exposes manifest export/download and diff data through the host API and test entry points; the user-facing `stc online download`/`stc online diff` commands remain M6. | Keeps the foundation milestone focused on the host/module/manifest contract; M6 owns the user workflow. |
| D-110 | `lib` is the embedding host library, not generated module code. In M4.5 it is a static host archive; shared host libraries are M6. The generated module and manifest remain separate artifacts. | Preserves the host/module split and online-change capability for embedding hosts; avoids linking generated code into the host. |
| D-111 | `exe` is a thin host launcher for an adjacent module and manifest pair. The M4 code-linked executable is retired; M4 ABI-1 artifacts are not loadable. | Matches D-094's host/module model and prevents a second, non-swappable execution path. |
| D-112 | `exe` and `lib` may be emitted together in M4.5. The M4 mutual-exclusion rule is removed because the two artifacts now have distinct roles (launcher vs embedding host library). | Matches the resolved CLI artifact model; the M4 restriction existed only because both were alternative code-linked artifacts. |
| D-113 | Stable keys are normalized uppercase ASCII paths scoped by package name, with kind-specific forms for roots, functions, types, descriptors, instances, tasks, exports, fields, and direct variables. `GotId` is the first 128 bits of a domain-separated SHA-256 over `(kind, package, path, qualifier)`; data/type identities exclude layout, function identities include the signature, and tracked renames carry the old ID. No heuristic matching is used. | Gives identity a precise, collision-checked, cross-developer contract while keeping layout changes trackable and function signature changes explicit. |
| D-114 | Binary artifacts are byte-packed little-endian records with length-prefixed strings/lists; canonical JSON uses sorted keys, fixed formatting, integer-only numbers, and the same logical fields. Manifest and descriptor digests are SHA-256 with domain separation; the descriptor digest is computed with its `manifest_digest` field zeroed so descriptor/manifest pairing is acyclic. | Deterministic byte-identical artifacts and a verifiable pairing chain without self-referential hashes. |
| D-115 | The module descriptor (format 1) is a self-contained pointer-free blob embedded in module read-only data; code addresses come from a separate `stc_module_functions` array, and the host copies descriptor and function data before unload. Slot records and function records are canonical, sorted, and duplicated in the manifest, where the host checks equality. | Keeps the loading contract stable across platforms, avoids module-owned persistent pointers, and makes descriptor/manifest disagreement detectable. |
| D-116 | Host ABI 2.0 fixes the table to version/size plus trap, cycle/monotonic/local clocks, alloc/free, I/O image/size, state load/save, and assert, with service IDs for import slots. Services are append-only within major 2; minor versions gate availability by `struct_size`. `trap` is non-returning to module code and the host catches it at the prepare/cycle boundary; module entry points are `stc_module_*`. | A narrow, versioned, testable boundary that supports rollback and keeps heavy services out of the cycle; the trap boundary is the only way to make a trap recoverable. |
| D-117 | Build IDs are 128-bit domain-separated SHA-256 truncations over profile, resolved target, compiler version, active configuration, ABI/format constants, generator flags, source fingerprint, parent identity, catalog digest, and plan digest. They exclude emitted bytes, output paths, clocks, and the environment; module file names embed them. | Reproducible builds and exact-parent online change without circular dependencies between module, descriptor, and manifest. |
| D-118 | Phase 2 clarifies three format points without changing a record layout or version number: elementary layout encoding tags 1–13 are fixed as listed in GOT Artifacts §4.3 with natural target sizes/alignments; an export wrapper is module-local code paired with the wrapped callable's function identity rather than a second callable identity; and a reintroduced key whose content-derived `GotId` appears in the baseline history is rejected with `STC-E-MANIFEST-002` because v1 has no explicit reuse policy. Direct-variable area/address metadata stays in the sidecar, so address remaps are detected by the host against module metadata. | Resolves ambiguities found while implementing the identity layer while keeping format 1 and the identity derivation contract unchanged. |
| D-119 | Phase 3 fixes the module IR shape: generated code reads a GOT cell with a `getelementptr` of `ptr` by slot number followed by a pointer-sized `load ptr` (align 8 on the x64 targets) and treats every cell value as opaque `ptr`; user and host-service calls are indirect through `Function`/`Import` cell values; entry points are defined `dso_local dllexport` with the ABI 2.0 signatures; and the descriptor blob and function-address table are private/internal constants. A structural verifier runs before rendering: bodies reference no persistent root or user callable directly, cell uses match their kinds, generated code never writes a cell, the pre-attach entry points do not touch the slot table, direct global access is limited to an allowlisted module-local set, and the function-address table is the only initializer that names a user function. | Fixes the IR-level contract that slot assignment and execution lowering in later phases must satisfy, and turns exit criterion 11 into an executable check on both hosts. |
| D-120 | Phase 4 projects the M4 scalar surface to the compiler catalog: scalar globals are `Global` roots, non-`AT` program fields are `ProgramField` roots per configured instance, `AT` globals/fields are `Direct` roots, retained scalars keep `Retain`, and every task gets a BOOL `TaskGuard` root initialized true. A configuration-less build synthesizes `_DEFAULT` configuration/resource/task/instance names and binds the instance to the synthesized task. Every module imports trap, I/O image/size, and retained-state load/save; alloc/free/assert and clocks are added when generated code first uses them. The projection exposes declaration-to-slot bindings; execution lowering still moves to the GOT model in phase 7. | Gives phase 4 a complete, deterministic catalog for the M4 surface while keeping the phase boundary with IR execution lowering explicit. |
| D-121 | Phase 4 implements descriptor format 1 in `stc-got::descriptor` (encode, decode, digest, format validation, and descriptor/manifest pairing) so compiler and host share one codec. The manifest is built once with a placeholder descriptor digest; because the build ID, catalog digest, and plan digest exclude the descriptor digest, the header is patched with the real digest and the descriptor sealed with the manifest digest without a second identity pass. Source locations are package-relative; a declaration without a source path carries no location. Manifest slot validation now rejects equal `GotId`s within a group, matching §6.2. | Keeps descriptor bytes and pairing checks in the shared format crate and avoids a circular digest computation without duplicating identity logic in the driver. |
| D-122 | Phase 5 implements the host substrate in `stc-runtime`. `got` owns a fixed per-generation cell vector, exposes stable cell and slot-table addresses, builds the prepare-mode fresh-root mask, and binds data/function/descriptor/direct/import cells by identity or service; `memory` owns zeroed aligned root storage (over-allocated `Box<[u8]>`, no raw allocator), exact-match preservation, quarantine until an explicit clear, transactional reset, failed-candidate discard, memory budgets, and per-generation transient blocks with exact-base free and `free_all`; `catalog` validates descriptor/manifest pairs against the host profile, target, endianness, pointer width, ABI range, and implemented feature bits and parses both manifest projections; `migration` validates a plan against the exact parent build/digest, preserve-shape compatibility, allocate reuse (including identities retired by the baseline), direct/export drop restrictions, retirement coverage, and history consistency. Diagnostic codes `STC-E-MODULE-002`, `STC-E-ABI-001`, `STC-E-ONLINE-005`, and `STC-E-ONLINE-006` are assigned. Loader, swap, and plan execution remain in phase 6 and later. | Establishes the host memory/GOT/validation contract before the loader and execution lowering depend on it, and keeps untrusted artifacts fully validated before any state is allocated or changed. |
| D-123 | Phase 6 implements the module loader and swap controller in `stc-runtime`. `loader` maps modules with `LoadLibraryW`/`GetProcAddress`/`FreeLibrary` and `dlopen`/`dlsym`/`dlclose`, copies the descriptor blob and function table, resolves the seven ABI entry points, and reports load failures as non-fatal module errors. `host_api` provides the static ABI 2.0 table (version/size plus the eleven services), binds import cells to table-field addresses, routes transient allocation and free per executing generation, implements the process-image areas, and records traps; the trap service remains non-returning and exits 70 in M4.5, with a non-local rollback-window boundary deferred to the execution conversion. `store` keeps the committed baseline as an immutable versioned manifest pair plus an atomically replaced `current` pointer, reports a missing or corrupt store without synthesizing a baseline, and builds a canonical download envelope with both manifest projections, deployment identifiers, and a value-free runtime layout. `host` validates candidates (pairing, function-table count, entry flags), implements `Auto`/`Cold`/`Online` load modes, gates online changes on the exact committed store and active generation, cold-installs into a fresh root registry, prepares online changes through a root-transaction journal, activates at an explicit boundary, commits after one successful cycle with atomic baseline replacement, rolls back a failed first cycle (including preserved records, dropped roots, and allocated roots), and delays retired-module unload behind generation pins. Generation IDs come from a process-wide counter. The Rust `Host` exposes export/download/diff; the C-facing launcher and embedding host exports land with phase 7 execution lowering. | A working, testable swap foundation that keeps the active generation and committed baseline intact on every validation, prepare, load, and first-cycle failure, and fixes the loader/state-machine contract that execution lowering and later milestones use. |
| D-124 | Phase 7 converts generated execution to the module model. `unit`, `expr`, and `stmt` lower every persistent scalar, program field, direct `AT` variable, retained leaf, and task guard through GOT cell values; program bodies receive an `[N x ptr]` per-instance field-storage array; user functions and program bodies are called only through function cells; host services are called through import cells; and modules emit the seven ABI 2 entry points, descriptor, function table, metadata, retain helpers, and export wrappers. Slot access now performs the two loads fixed by GOT Artifacts §7.3 (table entry to cell address, then cell value), and import cells hold the service entry point rather than the host API table field address. Prepare initializes only cold roots or fresh-mask slots, applies direct defaults on cold install only, and skips the retained-state file on online attach. The host reads `<name>.stc-meta.json` for image sizes and direct-root bindings, configures the image on cold install, and requires an identical layout/binding set for an online candidate until phase 8 adds remap. The driver emits `module`, `exe` (launcher calling `stc_rt_execute(argc, argv, module_path)`), and `lib` (embedding host archive) artifacts, allows `exe` and `lib` together, and wires `--got-manifest`, `--cold`, `--rename`, and `--allow-untracked-removal`. The runtime adds a non-local trap boundary around module entry-point calls (`__intrinsic_setjmpex` on Windows, `_setjmp` on POSIX, `uwtable` on generated functions). A small x86_64/aarch64 assembly shim owns the second `setjmp` return because Rust cannot attach LLVM's `returns_twice` attribute to the external call; without the shim the boundary's control flow can be miscompiled after `longjmp`. The runtime rolls back a rollback-window trap and returns exit code 70 otherwise. `stc_host_load_module`, `stc_host_cycle`, `stc_host_shutdown`, `stc_host_symbol`, `stc_host_export_manifest`, and `stc_metadata_json` form the phase-7 embedding C ABI. | Completes the GOT/module execution conversion so existing M4 execution behavior runs through compiler-produced module pairs, and gives phase 8 a working preservation/rollback substrate with a tested cold/online install path. |
| D-125 | Phase 8 completes the scalar compatibility and preservation policy in `stc-runtime` and tests it through the driver. A new `image` module validates metadata layouts and direct bindings against a 256 MiB host limit, computes provisional and actual direct-cell addresses, detects moved bindings, and preserves direct values by `GotId` across a reconfiguration; `%M` and same-address variables therefore survive, and every other identity-preserved direct variable keeps its value too. Activation applies the candidate layout after the old cycle: the view of the image is snapshotted, reconfigured or remapped in place, the candidate GOT is rebound, and the swap then records the snapshot and old bindings for rollback-window restoration. Prepare does not touch the active image and binds direct cells provisionally. A changed image layout or binding set no longer selects the cold path; invalid metadata, duplicate/missing bindings, unsafe layouts, and out-of-range areas are rejected as `STC-E-ONLINE-IO-001`, while a missing binding for a direct root remains `STC-E-ONLINE-006`. `Host::warnings` reports moved direct variables as `STC-W-ONLINE-IO-001` and dropped data roots as `STC-W-ONLINE-001`. `Host::reset_quarantine` is the explicit reset primitive; `PlanAction::Reset` and `Migrate` remain reserved and rejected in M4.5, so layout/type changes are still rejected by default and require a `--cold` rebuild. Cold installation reinitializes roots and the process image and keeps the retained-state file disabled; online preparation initializes only fresh roots, so preserved values and initializers follow State and RETAIN §4.1.1. Driver tests use compiler-produced modules for initializer/state preservation, new-root allocation, tracked rename, untracked-removal policy, type-change rejection with a cold reset, and direct-variable remap through generated code; runtime tests cover reconfiguration, same-layout remap, rollback, cold image reset, quarantine reset, and the warning surface. | Makes the M4 scalar surface obey the full preserve/allocate/drop and reject/cold policy at the cycle boundary, gives M5 a tested remap/rollback substrate for aggregate roots, and keeps reset/migration explicitly out of v1. |
| D-126 | Phase 9 completes test and CI coverage. `fuzz_got_manifest` decodes binary and canonical-JSON manifests, validates them, and checks digest-stable re-encodings; `fuzz_module_descriptor` decodes descriptor blobs and parses metadata sidecars; neither target maps or executes native code, and the smoke job caps inputs at one MiB. Driver tests build the same source and baseline in two directories and compare module, IR, metadata, manifest, plan, and build IDs byte-for-byte, which required `-Xlinker /Brepro` for Windows module and launcher links so PE timestamps are content-derived. A combined `exe,lib` build is run through the launcher and linked into the C embedding harness against the same adjacent module. Negative coverage adds missing, truncated, and malformed baselines plus invalid `--rename`, focused metadata-sidecar rejection tests, and a concurrent generation-pin test. The ignored `switch_latency` measurement records 1.7 ms prepare/500 ns switch at 16 roots and 86 ms prepare/1 µs switch at 1024 roots, supporting the O(1) activation claim; M4.5 loads and prepares synchronously (D-123), so maintenance jitter is not measured yet. CI adds an explicit `online` job on Windows and Ubuntu for the GOT, descriptor/manifest projection, host lifecycle, online-change CLI, and launcher/embedding suites. | Closes the phase-9 verification gap on both hosts and records the switch evidence before the phase-10 documentation pass. |
| D-127 | Phase 10 is the M4.5 documentation and handover pass. The README, project plan, conformance profile, architecture/runtime/state/I/O/CLI/testing/debug specifications, `M4.5-Plan.md`, and this log now describe the implemented host/module model instead of the retired M4 direct-reference model. `handover_to_M5.md` carries the final addendum: implemented contracts, the catalog/GOT/host extension points M5 must use, the M4 facts that still hold, and the open M5 decisions. [Debug and Monitoring](Architecture/Debug-and-Monitoring.md) §12 records the M4.5 interface available to M6.5 (identity/layout/root access, generation pins, descriptor debug feature bits, append-only ABI 2 services, trap records, swap/rollback semantics) and the missing pieces (checkpoint emission, DWARF/CodeView, typed access, write/force queues, session protocol). The phase also clarifies the debug flags: M4.5's default artifact form may set the descriptor debug-build entry bit, but it emits no debug information or checkpoint hooks, and the debug-checkpoint required-feature bit remains clear until M6.5. Metadata/state documentation now matches the emitted schema (retained leaf identity lives in the manifest; the sidecar carries path/type/offset/size) and the CLI documents which options are M6/M6.5 rather than implemented. | Makes the milestone's documentation truthful at handover and prevents M5/M6.5 from reintroducing the retired model or assuming debug hooks, CLI options, or metadata fields that do not exist. |
| D-128 | M5 lowers the existing M3/M4 HIR surface on the M4.5 host/module model. The only language-surface addition is the `POINTER TO` / `STC-POINTER` vendor extension already fixed by Q-131–Q-150; everything else is codegen for constructs the analyzer already resolves. Persistent storage remains host-owned per-field roots: M5 adds recursive `NestedField` roots, per-object descriptor records, and handle-based object views but does not introduce contiguous persistent parent allocations, module-owned vtables/descriptors, raw `this` pointers, persisted references, or module writes to GOT cells. Every new root, callable, type, descriptor, instance, task, and export is projected through the existing `stc-got` catalog/engine; identity derivation is not duplicated in a second path. | Keeps M5 an extension of the tested M4.5 artifacts rather than a second execution or identity model, and keeps the milestone boundary at codegen plus the narrow runtime services the new values need. |
| D-129 | The type/layout substrate computes one canonical type key per type (GOT Artifacts §4.1): elementary keys unchanged, `STRING/WSTRING/USTRING[n]`, `ARRAY[lower..upper]OF(key)`, `REF_TO(key)`, `PTR_TO(key)`, `T:<type GotId>`, and `VOID`. A named type's `GotId` derives from `(IdentityKind::Type, package, normalized qualified path, "")` and is independent of layout; aliases are transparent and resolve to the target key. Layouts use natural alignment and the target pointer width, with strings `{ i32 len, [n x unit] }`, nested LLVM arrays, declaration-order structs aligned to their largest member, `OVERLAP` as a union at offset zero, and the conformant descriptor `{ ptr data, i64 len, [rank x { i64 lower, i64 upper }] }`. Sizes, offsets, and layout hashes are compiler metadata only: emitted IR uses LLVM types and GEP and never embeds a target datalayout, pointer size, struct offset, or alignment beyond explicit length fields. `IrType` gains a rendered aggregate variant so operand types remain checkable without a second type arena. | Gives the catalog, metadata, retain layout, and IR one deterministic type source that works for every supported target and keeps layout changes visible to online-change classification. |
| D-130 | Aggregate and string ABI: structs, static arrays, strings, and object values pass by reference to caller-owned storage; aggregate function results use a caller-supplied hidden result pointer; scalar, enum/subrange, reference, and interface values pass by value. `VAR_IN_OUT` and output parameters receive pointers/place descriptors to the actual storage; a persistent aggregate split across roots is materialized into a transient alloca, passed, and copied back. A string value is one host root `{ i32 len, [n x unit] }`; copy normalizes the length to destination capacity, and capacity, array-bound, subrange, conversion, and UTF-8 failures trap with span IDs (`STC-TRAP_BOUNDS/RANGE/CONVERSION/INVALID_UTF8`). Transient aggregate values are contiguous LLVM values; only persistent storage is split per field. | A small, uniform calling convention that avoids target ABI lowering in textual IR while keeping persistent identity per field; copying split aggregates is the cost of the per-field root model and is documented. |
| D-131 | Persistent root enumeration (amended by D-147: class-typed fields are pointer-sized handles over separately rooted owned instances, not inline recursion; FB fields still recurse): elementary values, enums/subranges, strings, arrays of scalars/strings, and `OVERLAP` values are one root each; struct/FB/class fields recurse as `NestedField` roots; arrays of aggregates recurse per element so every element field is a root; reference and interface values are ordinary in-memory roots but are excluded from retained-state leaves. Each root path is the normalized configuration/instance path plus base-chain field and element segments, carrying its own type key, layout hash, size, alignment, retention, initializer, and `GotId`. A program/FB/method body receives a per-instance root table (`[N x ptr]`) indexed by the object type's static leaf ordinal; callers build it from the active instance's slots. V1 has statically configured instances only, so every object path is known at build time; dynamic array-of-aggregate indices select among static element leaves with bounded generated code rather than a dynamic root lookup. | Applies Q-042/Q-123–Q-129 to the existing GOT without a second root model, keeps every persistent field independently preserve/drop-classified, and makes the static-instance limitation explicit. |
| D-132 | `REF_TO` values use a fixed 48-byte place descriptor `{ i32 kind, i32 flags, i64 offset, i64 id_lo, i64 id_hi, ptr data, ptr descriptor }` with kinds `Null`, `Root`, `Direct`, `Stack`, and `Object`; `REF(place)` constructs one, and dereference calls a host resolver that validates nullness, generation, identity, and layout before returning storage. Dereferencing `NULL` or an invalid descriptor traps `STC-TRAP_NULL_REF`. Host services for place dereference/resolution, descriptor compatibility (`?=`), and pointer→reference validation are appended to the ABI 2 table with a minor-version bump; existing service IDs and signatures do not change. `POINTER TO` is implemented as the Q-131–Q-150 transient `STC-POINTER` extension: a distinct raw-pointer type for locals, parameters, results, contiguous leaves, array elements, strings, and `OVERLAP` views, with typed arithmetic, indexing, `SIZEOF`, null-deref traps, and documented dangling behavior; persistent pointer fields and pointer↔integer or pointer↔object/interface conversions are rejected. | Fixes the reference ABI M5 codegen and the host share, keeps references remappable across swaps, and contains the vendor pointer extension to transient values. |
| D-133 | Class, FB, and interface representation (amended by D-147–D-151): type-info and vtable records are runtime-owned and projected from `Catalog.descriptors`; vtable slot order comes from the sema-assigned HIR `vtable` carried on each method's `vtable_slot` and is never recomputed. An object record is host-owned `{ ptr typeinfo, [N x ptr] leaf storage addresses }`; the leaf entries are resolved host root addresses, not GOT cell pointers, so a preserved record needs no rebinding. A class value is a pointer-sized non-retained handle to an owned instance whose field roots keep their declared retention; an FB value is an inline instance. Class-typed fields, array elements, parameters, locals, and results project `PTR_TO(T:<class>)` storage while the class `TypeRecord` keeps its instance size, alignment, and layout hash; class `:=` copies the handle and orphans the destination's owned instance, while FB `:=` copies fields. `THIS`/object handles are record pointers; interfaces are `{object record, per-(object type, interface) vtable}` pairs; `NULL` is all-zero. Every class/FB method dispatches virtually through TYPEINFO and the class VTABLE; `SUPER` calls the base implementation directly. Interface upcasts use a compile-time map index with inherited indices fixed, and interface-to-interface upcasts and `?=` use a bounded search. `?=` is inline and never traps or calls a host service; failure yields `NULL`. `DescriptorKind::Object` stays defined but is unprojected because object records are host storage. Method and FB body calls take the object handle and root table first, then parameters; FB inputs are written before the body and outputs read afterwards, including nested instances and `NOT =>` negation. A handle or interface that refers to a stack record must not escape its activation: returning it, storing it persistently, or writing it through an `OUT`/`VAR_IN_OUT` parameter is rejected. A vtable shape change is rejected while a live instance or interface depends on it. | Fixes the OOP ABI and descriptor ownership so preserved OOP values survive module unload, and keeps dispatch swap-safe through host cells. |
| D-134 | Standard functions are split by semantics: `ABS`, `MIN`, `MAX`, `LIMIT`, `SEL`, `MUX`, comparisons, integer arithmetic, and bit/shift/rotate are inline checked IR (`llvm.fshl`/`llvm.fshr` for rotates); strings, float math, date/time, and checked conversions call appended host services (`stc_rt_str_*`, `stc_rt_math_*`, `stc_rt_time_*`). The standard FBs `SR`, `RS`, `R_TRIG`, `F_TRIG`, `CTU`, `CTD`, `CTUD`, `TP`, `TON`, `TOF` are synthesized as production HIR in the reserved `STC_STD` scope with stable keys and hidden state fields rather than textual `stdlib/*.st` injection, which needs the M6 package graph to resolve and shadow a separate standard package without duplicate definitions. Synthesized defs are not skipped as empty-span artifacts when instantiated, and their bodies read the latched `cycle_now_ns` service only. | Delivers the standard library without pulling the M6 project graph forward, records the textual-injection deviation explicitly, and guarantees the single-latched-clock rule. |
| D-135 | Retained-state layout extends to aggregate/string/FB leaves: paths flatten recursively (`Main.Timer.ET`, `Arr[1].x`), arrays/structs inherit the effective policy, strings save their fixed-capacity value, enums/subranges save their base integer, and FB/class instance data fields are included. Payload offsets are packed cumulative sizes in sorted-path order, never LLVM struct offsets; save/load use GEP chains and `align 1`. References, interfaces, pointers, `VAR_IN_OUT`, and descriptor/vtable pointers are excluded; a leaf containing one keeps `STC-E-RETAIN-001`. The FNV-1a layout-hash format and metadata leaf schema are unchanged; file-backed state remains disabled until M6. | Extends the tested retain contract without a format change and keeps non-persistable values out of the state file. |
| D-136 | M5 does not implement `stc test`; the M6 manifest owns test discovery. M5 verification uses codegen unit/snapshot tests, mandatory clang acceptance, driver execution harnesses, embedding-host C harnesses, hot-swap CLI tests, and extended fuzzing. Metadata additions are limited to aggregate-derived retain leaves in the existing schema and to catalog/manifest type/descriptor records; no new sidecar keys are invented. Types, descriptors, vtable entries, and string constants are ordered deterministically, and double builds remain byte-identical. | Confirms the handover's test-runner deferral, keeps the metadata schema stable, and makes M5 verification executable with the existing suites. |
| D-137 | M5 phases 1–2 add the type/layout and aggregate substrate. `stc-codegen::types` computes canonical keys, named-type identities, natural layouts, layout hashes, contiguous LLVM views, `field_gep_indices`, array views, and recursive root leaves; `IrType` gains a rendered aggregate variant plus multi-index `extractvalue`/`insertvalue`, `memcpy`, `switch`, and value casts. The catalog projects named types and field records and enumerates per-leaf roots: globals and program-instance fields become `Global`/`GlobalLeaf` or `ProgramField`/`NestedField` roots, arrays of scalars/strings remain one root, arrays of aggregates recurse per element (`E<index>` path segments), and strings/`OVERLAP`/references/interfaces are single roots. Program/FB bodies receive a leaf-ordered `[N x ptr]` root table; `place.rs` resolves static leaf paths, GEP indexes single-root arrays with bounds traps, selects per-element roots for runtime indices in arrays of aggregates (bounded at 4096 alternatives), and materializes/splits structs across roots for value reads, writes, and initializers. Aggregate parameters/results remain deferred to phase 3, but locals, globals, program fields, initializers, assignment, `ArrayInit`/`StructInit`, string literals/capacity casts/`LEN`/`LEN_MAX`/`LEN_CODE_UNIT`/`CONCAT`/`=`/`<>`, subrange checks, enum literals and `CASE` labels are lowered. Standard inline functions (`ABS`, `MIN`, `MAX`, `LIMIT`, `SEL`, `MUX`, comparisons, arithmetic, bitwise, shifts/rotates, `MOVE`) are implemented; math/date and search/replace functions stay phase 5. Codegen, clang acceptance, and a C-harness library test cover struct/array/string globals, locals, dynamic array-of-struct indices, enums/subranges, multi-dimensional arrays, initialized aggregates, and range/bounds traps. | Establishes the per-leaf root/value model M5 needs without changing the M4.5 artifact contract, and gives later phases a tested base for references, OOP, the standard FB library, and retained state. |
| D-138 | M5 phase 3 completes the parameter/aggregate ABI and references. Function parameters are passed by value for scalar and object-handle inputs and by pointer for aggregate inputs, outputs, and `VAR_IN_OUT`; aggregate results use a caller-supplied hidden result pointer. The caller copies aggregate inputs into caller-owned temporaries so IN semantics are preserved, materializes split persistent output targets and copies them back after the call, passes contiguous targets directly, and applies `NOT =>` after the call. Reference values use the 48-byte place descriptor from D-132; `REF(place)` stores the resolved contiguous storage pointer (kind `Root` or `Stack`, with the owner `GotId` when known) or an area/byte offset for `AT` places (kind `Direct`), and dereference null-checks the descriptor, resolves `Direct` through the existing `io_image` service, and offsets by the descriptor offset. `RefAssign`, `NULL`, reference equality/inequality, `REF_TO` casts, and `STC-TRAP_NULL_REF` are implemented; references to split aggregates are rejected. Rank-1 conformant arrays use `{ ptr, i64, [1 x { i64 lower, i64 upper }] }`, are built from the actual array's contiguous storage and declared bounds, are indexed through the descriptor with dynamic bounds traps, and required a sema assignability rule allowing a static array argument for a conformant parameter. The catalog records `OUT`/`INOUT` parameter modes. A C-harness library test covers aggregate input/output/in-out, aggregate results, IN copy isolation, `VAR_IN_OUT` copyback, references to globals/locals, and conformant array reads/writes and bounds. Deferred: interface/class assignment attempts (`?=`, phase 4), FB field binding (phase 4), multi-dimensional conformant arrays, REFERENCES to split aggregates, and the transient `STC-POINTER` extension. | Completes the value/call boundary that OOP and the standard FB library need, and gives references a memory-safe representation that survives module unload because root storage is stable. |
| D-139 | M5 phase 4a implements classes, function blocks, methods, inheritance, `THIS`, and `SUPER`. `config::object_layouts` enumerates each class/FB type's persistent leaves base-first; FB bodies and methods are emitted with the semantic parameters and an implementation prelude that receives the instance's `[N x ptr]` leaf-root table (`%this` for methods, `%fields` for FB bodies). Catalog function records use `FunctionKind::FbBody`/`Method`, the owning type key, the FB input/output signature, and the HIR vtable slot. `CalleeHir::Fb` writes bound inputs into the instance's fields, calls the FB body cell with the table, then reads outputs into `=>` targets with `NOT =>` negation; `CalleeHir::Method` calls the statically resolved method cell with the receiver table; `SUPER.m()` and `SUPER()` call the base implementation on the same table. Object assignment is a per-field recursive copy; local and persistent objects both get tables, persistent objects reading the program/global root table and local objects using their contiguous alloca state. Method lookup in sema now prefers the most-derived method for an identical signature instead of reporting an ambiguous override, and `#BODY` function keys accept the FB parameter signature. A C-harness library test covers class fields/methods, overrides, `SUPER`, `THIS`, object copy, local and global FB state across cycles, and nested FB/class calls. Deferred to phase 4b: interface values, `?=`, runtime type descriptors/vtables, and preserved interface values. | Delivers the non-polymorphic OOP/FB surface on the existing per-leaf root model without requiring a runtime descriptor-table change, and leaves the descriptor/vtable contract as the next isolated step. |
| D-140 | M5 phase 5 synthesizes the standard FB bodies. The prelude keeps the ten standard FB types as empty-span synthetic defs with public input/output fields and private state fields (`TON`/`TOF`: `__START`, `__RUN`; `TP` adds `__DONE`; `CTU`/`CTD`: `__PREV`; `CTUD`: `__PREVU`/`__PREVD`; edge and bistable blocks need none). `types::used_standard_fbs` scans every field, local, parameter, global, and result type transitively, and `module`/`unit` no longer skip those synthetic defs when they are used: they receive catalog type/field/body records, GOT function cells, and `[N x ptr]` field tables from the normal object layout. `stdlib.rs` emits each body as checked IR over the field pointers: timers read the `cycle_now_ns` import service at body entry, counters count rising edges and are reset/load dominant, edge triggers store the previous clock, `SR` is set-dominant and `RS` reset-dominant, and all bistables retain state when idle. `stc_host_cycle` now latches `stc_rt_set_cycle_time_ns` before running the module cycle so embedding hosts observe the same clock contract as the launcher. A C harness drives every block with explicit cycle timestamps and checks ET/CV/Q state. String standard functions are recorded in D-141; retained-state flattening remains. | Completes the standard FB library without textual stdlib injection or runtime ABI changes, and makes timer behavior deterministic and testable through the existing latched-clock service. |
| D-141 | M5 phase 5 completes the string standard functions inline in `place.rs`. `LEFT`/`RIGHT`/`MID`/`DELETE` copy unit slices with `memcpy`, `INSERT`/`REPLACE` copy a prefix, the inserted string, and a suffix, and `FIND` scans candidates with `memcmp`. Positions are 1-based; negative or zero counts produce empty results; positions and lengths are clamped to the input length; `INSERT`/`REPLACE` results that exceed the fixed result capacity trap `STC-TRAP_RANGE`. String values remain the `{i32 length, [capacity x iN]}` representation, so the same code serves `STRING` (`i8`), `WSTRING` (`i16`), and `USTRING` (`i32`, one code point per unit); mixing encodings is rejected explicitly. The last `StdFnKind` variants now have implementations, so no standard call is rejected for being unimplemented. Driver coverage compares results for ASCII strings, a wide string, and a UTF-8 string with a four-byte scalar (`aé😀z`), including `LEN`, `LEN_CODE_UNIT`, slicing, and `FIND`, plus a capacity-overflow trap. | Closes the standard-function surface without new runtime ABI or host helpers, and gives UTF-8 behavior code-point semantics through the existing representation. |
| D-142 | M5 phase 6 flattens retained state over the persistent leaf model. `RetainLeaf` now carries the leaf's LLVM storage type, canonical spelling, size, and a `Global`/`GlobalLeaf`/`InstanceField`/`InstanceLeaf` place; `config::retain_layout` walks `TypeSystem::root_leaves` for every non-`AT` global and every configured program field with the declaration's effective policy, so structs, arrays of structs, class/FB objects (including input/output/state fields), strings, scalar arrays, and `OVERLAP` values each contribute exactly their persistent roots. Paths use readable source field names and source array indices (`SavedPoints[1].x`, `inst.sp.x`, `SavedFb.label`); references, interfaces, and pointers remain rejected by sema. Leaves are sorted by uppercase path and assigned packed cumulative offsets in that order, as before; the FNV-1a layout-hash format and the metadata leaf schema (`path`, `ty`, `offset`, `size`) are unchanged, with `ty` rendered by `HirCrate::type_name`. `stc_retain_save`/`stc_retain_load` load and store whole leaf values through their GOT root slots with `align 1` buffer accesses, so strings and arrays preserve their fixed-capacity representation. Driver coverage round-trips a retained struct, array of structs, strings, FB instance state, and program-instance aggregate/string fields, checking metadata paths plus raw payload bytes at their packed offsets. | Extends the tested retain contract to every M5 persistent aggregate without changing the state-file or metadata formats, and keeps non-persistable values out of the payload. |
| D-143 | Multi-dimensional conformant `ARRAY [*, *]` parameters use the same `{ ptr data, i64 total_elements, [rank x { i64 lower, i64 upper }] }` descriptor as rank 1. The caller records every dimension's declared bounds and the total element count; the callee's subscript path consumes exactly `rank` consecutive indices, bounds-checks each against its descriptor entry, and computes a row-major linear index from the runtime dimension lengths before one element-typed GEP from the data pointer. A missing subscript for the declared rank is an explicit codegen diagnostic, and any out-of-range dimension traps `STC-TRAP_BOUNDS`. Coverage sums and writes a rank-2 array through `VAR_IN_OUT` parameters and checks the rank-2 bounds trap. | Removes the phase-3 rank-1 limitation without changing the descriptor shape or ABI, using the existing runtime bounds and place model. |
| D-144 | `REF` accepts any place that resolves to contiguous storage, not only single-root types. Local, parameter, and result structs and arrays of structs are contiguous today, so references to them produce a `Stack` place descriptor and aggregate dereference reads/writes the whole storage value. Persistent split structs still have no single storage location and continue to report the explicit "reference target has no stable storage" diagnostic; `OVERLAP` and single-root persistent values are unchanged. A harness case stores `REF(local Point)`, replaces the whole aggregate through `r^`, and reads it back through the reference. | Removes an unnecessary type-based restriction while preserving the rule that references never point at materialized temporary copies of split roots. |
| D-145 | `STC-POINTER` surface grammar: `POINTER`, `ADR`, and `SIZEOF` are reserved keywords; `POINTER TO type_spec` is a `type_spec` alternative (named pointer types use `simple_type_decl`; `POINTER TO POINTER TO T` is allowed but `POINTER TO REF_TO T` and `REF_TO POINTER TO T` are rejected). `ADR(place)` and `SIZEOF(type_spec \| place)` are primary expressions; `place` widens the assignment LHS so `p^`, `p[i]`, `(p + 1)^`, and `ADR(x)^` are writable. Explicit conversions are `REF_TO T(expr)` and `POINTER TO T(expr)`. `ADR(array)` decays one dimension to an element pointer; `SIZEOF` is a compile-time constant for statically sized operands and yields `UDINT`, while `ADR` is never constant; pointer locals/fields default to `NULL`. Pointer values remain transient, including inside aggregates proven transient, while pointer targets may be contiguous persistent roots/leaves, transient aggregates, strings/arrays/`OVERLAP`, and other pointers; FB/class/interface objects, split persistent aggregates, `REF_TO` targets, `ARRAY [*]`, `%I`/`CONSTANT` addresses, pointer-typed `VAR_IN_OUT`, and pointer-typed `VAR_ACCESS` are rejected. | Records the missing surface before implementation, keeps the parser context-free, and preserves the Q-131–Q-150 transient/safety envelope. |
| D-146 | M5 implements the `STC-POINTER` surface recorded in D-145. HIR adds `Ty::Ptr(target)`, `AddressOf`, `SizeOf { Type \| Place }`, and an explicit `PointerConversion`; pointer types use the `PTR_TO(key)` canonical key with a `ptr` LLVM value and target pointer-width layout, and reference/pointer layout hashes use reference row kind 2 for pointers (kind 1 remains `REF_TO`). Sema enforces the transient-storage rule (globals, program/FB/class fields, pointer-containing persistent aggregates, `VAR_IN_OUT`, `VAR_ACCESS`, and `RETAIN` reject pointers), contiguous non-conformant targets, `POINTER TO REF_TO T`/`REF_TO POINTER TO T` rejection, `ADR` array decay, `%I`/`CONSTANT`/bit-field/`REF_TO`-deref/split-persistent `ADR` rejection, `SIZEOF` restrictions, pointer-only arithmetic/comparisons with `p - q : LINT`, same-target `REF_TO`↔pointer conversions, and rejection of a direct `ADR(local\|parameter\|result)` escape through the function/method result. Codegen lowers `ADR` to the existing resolved storage pointer for GOT roots/leaves, process image, transient allocas, and pointer-chain places; checks null before dereference and pointer indexing (`STC-TRAP_NULL_REF`); emits typed GEP arithmetic and scaled pointer differences; and converts `REF_TO`→pointer by extracting the descriptor (null/object/`%I`/invalid yields `NULL`) and pointer→`REF_TO` by building a `Stack` descriptor (null yields `NULL`). `SIZEOF` is folded during code generation, so it is a constant in emitted IR but is not accepted in analyzer constant-expression positions. Parser, sema, C-harness, and null-deref trap coverage exercise pointer locals, fields, parameters/results, arrays of pointers, string/array/struct targets, direct `%M` addresses, leaf roots, pointer-to-pointer, conversions, and `SIZEOF`. | Completes the vendor extension within the transient/contiguous safety envelope without a runtime ABI change, and keeps layout computation in the single codegen type source. |
| D-147 | Phase 4b object representation and ABI (Q4B-1–Q4B-5, Q4B-16): class and FB object values are pointers to records `{ ptr typeinfo, [N x ptr] leaf storage pointers }`; persistent record entries are resolved host root addresses, not GOT cell pointers, so roots never move and preserved records need no rebinding (this clarifies D-133's "field-root cell pointers" wording). Persistent objects get one hidden non-retained record root each (program-instance object fields, globals, nested FB/class instances, and array elements); the host allocates the storage and generated `stc_module_prepare` fills typeinfo and leaf pointers for fresh objects only, using the fresh-root mask. Function and method locals use per-activation stack records; class object parameters are handles to caller records, while FB parameters are values handled by the IN/OUT rules below. Generated code performs no heap allocation, and recursion is allowed. Class values are references: a class-typed persistent variable or field owns an embedded instance (per-field roots plus a hidden record root) and stores a non-retained handle root initialized to that record on fresh prepare; a class-typed local owns a stack instance and a stack handle slot. The handle occupies the declaration's slot in the containing layout (a pointer-sized field, array element, local, parameter, or result); the owned instance's fields are separate roots or stack storage, not inline, and handle storage is projected with the internal `PTR_TO(T:<class>)` key. Arrays of classes are arrays of handles with one owned instance per element. Class-typed `FieldRecord`s therefore project pointer layouts while the class `TypeRecord` keeps its instance size, alignment, and layout hash. FB-typed declarations remain inline. This supersedes D-131's inline recursion for class-typed fields; FB fields still recurse. `:=` between class values copies the handle (sema's `RefAssign`, D-069), and assigning away orphans the owned instance, which static memory keeps rooted. The owned instance's field roots keep their declared retention policy even when the handle is reassigned; class handles themselves are not retained, so a cold start re-points each fresh handle at its owned instance while the owned instance's retained fields restore normally. FB values remain embedded instances and `:=` between FB values remains a per-field copy. Assignment through a dereferenced `Object` reference copies state into the referenced object and never rebinds the original variable. Sema rejects storing or returning a handle or interface that refers to a stack record where it can outlive the activation, including writing it through an `OUT` or `VAR_IN_OUT` parameter, analogous to the pointer-escape rule. Methods and FB bodies receive `(handle, root table, ...)`; `THIS` is the handle and `SUPER` uses the same handle with the base implementation. Class `IN` parameters are handles (assigning the parameter only rebinds the callee handle); FB `IN` parameters are copied by the caller for IN isolation; `OUT`/`VAR_IN_OUT` objects pass pointers to actual storage. | Gives every object a stable type identity and leaf view without heap allocation, makes class references and FB values match the analyzer's assignment classification, and keeps persistent records swap-stable. This supersedes D-139's per-field class copy; FB assignment is unchanged. |
| D-148 | Phase 4b descriptor projection and interface dispatch (Q4B-6–Q4B-8, Q4B-12, Q4B-14): only TYPEINFO and VTABLE descriptors are projected: one TYPEINFO per class/FB type and one VTABLE when the type has vtable entries; interface types have no descriptor of their own because their prototype order is sema-assigned and their per-pair vtables live in the object TYPEINFO. `DescriptorKind::Object` stays defined but is not projected in M5 because object records are host storage. Interface prototype methods are projected as ordinary function records (`FunctionKind::Method`, `owner_type` = interface, HIR `vtable_slot`, trap-stub code cell); the host matches each prototype's name and signature to the most-derived implementor by walking the type's base chain and `TypeRecord.interfaces` (the method name is the final path segment of `FunctionRecord.key`), and builds one vtable per (object type, interface) in the interface's sema-assigned slot order. TYPEINFO for a class/FB contains its type identity, layout hash, base-chain identity list, class-vtable pointer, and a base-first interface map of `{interface GotId, per-pair vtable pointer}`: inherited entries keep their indices and newly implemented interfaces are appended sorted by `GotId`. VTABLE contains class method cells in HIR class slot order, with overrides rebinding the base slot. All class/FB method calls dispatch virtually by loading typeinfo, then VTABLE, then the slot cell; `SUPER.method()` stays a direct base-implementation call; interface calls index the interface value's per-pair vtable. Dispatch checks for a null class handle, object record, or interface value and traps `STC-TRAP_NULL_REF` before loading a vtable entry. Class/FB-to-interface upcasts use a compile-time index into the source static type's base-first map; the inherited-index rule makes that index valid for any derived runtime object. Interface-to-interface upcasts and dynamic attempts use a bounded search by interface `GotId` over the small map. No manifest format change is required: only new function and descriptor records are added. | Keeps format 1, preserves per-type slot stability, and gives O(1) interface dispatch without module-owned vtables. |
| D-149 | Phase 4b interface ABI, upcasts, and `?=` (Q4B-11, Q4B-12): interface values are 16-byte `{ ptr object record, ptr per-interface vtable }` values passed by value for parameters and results, stored as single roots in fields/globals/arrays, and all-zero for `NULL`. Comparisons support `= NULL`/`<> NULL` only; identity comparison is rejected in M5. Upcasts from a class/FB type use the compile-time interface-map index; interface-to-interface upcasts and dynamic attempts use a bounded search by interface `GotId` over the object type's base-first interface map. `?=` performs full dynamic checks inline: a class destination checks the object typeinfo's base chain, an interface destination checks interface membership (including `EXTENDS`), and a reference destination uses the place-descriptor/`Object` check. Failure assigns the all-zero `NULL` value; success copies the source value (a class handle, an interface pair, or a reference descriptor); a null source makes the attempt fail rather than trap, and attempts never trap otherwise. No host attempt service is added; this supersedes D-132's appended-attempt-service clause, matching the already-inline dereference path of D-138. | Implements the documented nullable-interface and assignment-attempt semantics without an extra ABI call, and keeps dynamic dispatch and attempts consistent with D-148's TYPEINFO layout. |
| D-150 | Phase 4b object references and `REF_TO`-to-interface (Q4B-13): `REF(object place)` produces a place descriptor of kind `Object {handle, descriptor}` instead of a descriptor over the variable's handle storage; dereferencing an `Object` descriptor yields the object, and field access/assignment through it uses the normal object record/root-table rules. Assigning a whole class or FB value through a dereferenced reference copies state into the referenced object; it cannot rebind the original variable's handle. Object-reference `?=` compares the referenced object's typeinfo/descriptor using the same compatibility rules as D-149. `REF_TO`-to-interface is allowed only when the reference target is a class/FB object type: a null source yields a null interface, an incompatible object yields null under attempt semantics (Q-171), and the conversion uses the `Object` descriptor check. Pointer-to-object/interface conversions and `ADR` of object places remain rejected, and references to split persistent aggregates keep their existing diagnostic. | Completes the reference side of the OOP model without reintroducing raw object pointers, and aligns the `Object` kind with Q-152/Q-165/Q-171. |
| D-151 | Phase 4b host ABI 2.1, descriptor lifecycle, and feature bits (Q4B-9, Q4B-10): generated code reads the TYPEINFO, VTABLE, and object-record layouts inline, so the host ABI minor is bumped from 2.0 to 2.1 and the layout is documented in GOT Artifacts and Runtime and Execution; no new host service is appended. TypeInfo/vtable records are host-owned and keep stable addresses across compatible swaps; prepare builds shadow descriptor contents, and activation patches method-cell entries and interface maps under the cycle boundary, so preserved interface values remain valid. Shape changes (kind, owner, type key, layout hash, size, align, vtable shape) reject while a live object/interface depends on them; removing a target type while a live class handle or interface value depends on it also rejects the swap, so dispatch can never reach retired code. Removed descriptors retire and quarantine with their roots. Hidden record roots and class handle roots are non-retained roots: fresh records and fresh class handles (pointing at their owned instance record) are initialized once at prepare, preserved records and handles are not rewritten, a live handle to a removed instance is quarantined rather than freed, and rollback releases fresh records and handles. Required-feature bits 2 (OOP descriptors/vtables) and 3 (place descriptors) are emitted when used, and the runtime advertises both. | Makes the layout an explicit, versioned contract while keeping manifest/descriptor format 1 and the existing preserve/reject rules. |
| D-152 | Phase 4b scope and verification (Q4B-15): the phase covers the full documented surface — class reference assignment, interface parameters/results/fields/globals/arrays, function blocks implementing interfaces, interface `EXTENDS`, `?=` in both directions, `THIS` as an interface, arrays of objects with per-element records, and `REF_TO` objects/interfaces. Unsupported combinations remain explicit `STC-E-CODEGEN-001` diagnostics. Verification updates the phase-4 `?=` and interface-dispatch rejection tests in `stc-codegen/tests/codegen.rs` and the driver rejection case, adds C-harness tests for virtual dispatch, class reference assignment/aliasing and downcasts through handles, class handle layout and arrays with per-element owned instances, retention of owned-instance fields across handle reassignment, interface dispatch, upcast/attempt success and failure, NULL tests, object references, `REF_TO`-to-interface, interface parameters/results/arrays, and arrays of objects, adds hot-swap tests for preserved interface values and class handles, descriptor identity, vtable-shape rejection, fresh-record initialization, and rollback, and adds malformed TYPEINFO/VTABLE/interface-map fuzz cases. Interface maps are base-first with newly implemented interfaces sorted by `GotId`, descriptor and vtable ordering stays deterministic, and double builds remain byte-identical. | Closes phase 4b against the documented exit criteria and keeps the artifact/determinism contracts covered by tests. |

D-118 was reviewed and approved on 2026-10-24; the four clarifications stand as recorded and phase 2
is approved. Phase 3 is complete for review and D-119 records its IR-level choices. Phase 4 is
complete for review (2026-10-25); D-120 records the catalog projection and D-121 the descriptor
codec and pairing order. Phase 5 is complete for review (2026-10-25); D-122 records the host
substrate and validation boundaries. Phase 6 is complete for review (2026-10-25); D-123 records
the loader, host API table, active-manifest store, and swap controller. Phase 7 is complete for
review (2026-10-25); D-124 records the GOT/module execution conversion, the metadata-driven
process image/direct bindings, the launcher/embedding C ABI, and the non-local trap boundary.
Phase 8 is complete for review (2026-10-25); D-125 records process-image reconfiguration and
direct remapping, the runtime warning surface, the explicit quarantine reset, and the real-module
hot-swap/rename/reject/cold coverage. Phase 9 is complete for review (2026-10-25); D-126 records
the artifact/metadata fuzz targets, deterministic link timestamps, the combined launcher/embedding
artifact test, concurrent pinning, and the recorded switch measurement. Phase 10 is complete
(2026-10-25); D-127 records the documentation/handover pass, the M6.5 interface boundary, and the
debug-flag and metadata-schema clarifications. M4.5 is complete and M5 is the next milestone.
M5 phase 0 (2026-10-25) records the milestone boundary and architecture in D-128–D-136 and
`M5-Plan.md`; phases 1–6 are recorded in D-137–D-144. D-145 records the `STC-POINTER` surface
grammar and D-146 records its implementation. Phase 4b is planned in D-147–D-152 and
`M5-Plan.md`; step 4b.0 (host ABI 2.1 and the inline record contracts) is complete, and the
implementation continues from that plan with 4b.1.

D-094 through D-152 affect
[Online Change and GOT](Architecture/Online-Change-and-GOT.md),
[GOT Artifacts and Schemas](Architecture/GOT-Artifacts.md),
[Debug and Monitoring](Architecture/Debug-and-Monitoring.md),
[Architecture](Architecture/Architecture.md),
[Runtime and Execution](Architecture/Runtime-and-Execution.md),
[State and RETAIN](Architecture/State-and-Retain.md),
[I/O and Devices](Architecture/IO-and-Devices.md),
[CLI and Manifest](Architecture/CLI-and-Manifest.md),
[Testing and Verification](Architecture/Testing-and-Verification.md), the
[Conformance Profile](01-Conformance-Profile.md), `README.md`, `M4.5-Plan.md`, and
`handover_to_M5.md`.

D-042 is a host-portability requirement, not a language/profile change. It does not alter the IR,
runtime ABI, or conformance profile. It requires host-specific operations (clang/linker
discovery, artifact naming, path semantics, monotonic clock, atomic file replace) to be confined
to the driver and runtime. Affected documents: [Architecture](Architecture/Architecture.md) §6.4,
[Runtime](Architecture/Runtime-and-Execution.md) §2, [State](Architecture/State-and-Retain.md) §4.2,
[Testing](Architecture/Testing-and-Verification.md) §6, and the [Project Plan](00-Project-Plan.md)
M8.

D-045 through D-058 resolve the M2 handover's open items. The syntax surface is implemented in
`crates/stc-syntax` (stable kinds, lossless CST, typed AST views, pragma JSON),
`crates/stc-parser` (recursive descent, Annex A precedence, recovery, unsupported-construct
diagnostics), and the `stc` driver.

D-059 through D-075 resolve the M3 handover's open items and are implemented in `crates/stc-hir`
(arenas, IDs, interner, canonical types, HIR) and `crates/stc-sema` (collection, resolution, type
checking, constant evaluation, initializers, configurations, access control, references, I/O and
retention checks, lowering). `stc check <file>...` reports lexical, syntax, and semantic
diagnostics; HIR is produced in M3 and is the M4 codegen input.

D-076 through D-093 resolve the M4 handover's open items and are implemented in
`crates/stc-codegen` (textual IR builder, scalar layout, mangling, expressions, checked
arithmetic, casts, statements, units, configurations, metadata), `crates/stc-runtime` (C ABI,
cycle loop, process image, clock, traps, state stubs), the HIR/sema gap closures (`{stc:io}`
hints, export names, folded task values, LDATE units), and the driver's manifest-free
`stc build` bridge. M4 codegen consumes only HIR; unsupported M5 constructs are explicit
`STC-E-CODEGEN-001` diagnostics.

## 3. Open questions

**None as of 2026-10-24.** All v1 planning questions Q-001 through Q-017 are resolved
(A-005 through A-021). The online-change/GOT review Q-018 through Q-212 is resolved
(A-022 through A-030); the full question/answer disposition is in `update_plan_got.md` §13.0.
New questions will be appended here and in `README.md` one at a time, with a proposed default so
implementation can continue without blocking.

## 4. Decision process

- New decisions get the next `D-NNN`; superseded decisions are kept with a `Superseded by`
  line, not deleted.
- Questions get the next `Q-NNN`; answered questions move to §1 with an `A-NNN` entry and a
  summary in `README.md`.
- Each decision records the reason and the affected specification sections.
- A decision that changes the language surface also changes the conformance profile and adds a
  test ID.
