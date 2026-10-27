# Testing and Verification

Status: M4.5, M5, and M6 implementation and test plans complete (2026-10-27); M6 conformance/UI corpus, Lean vectors, traceability, fuzz smoke, and CI matrix active; M6.5 phase 0 fixes the debug suite contracts (protocol, access, checkpoints, builds, diagnostics) and the debug tests are the implementation target (2026-10-27)  
Profile: `stc-2013-oop-utf8-v1`  
Related: [Project Plan](../00-Project-Plan.md), [Runtime](Runtime-and-Execution.md), [Online Change and GOT](Online-Change-and-GOT.md), [Debug and Monitoring](Debug-and-Monitoring.md), [CLI/Manifest](CLI-and-Manifest.md)

> **M4/M4.5 status:** IR snapshots, clang verification (`STC_REQUIRE_CLANG=1` in CI), execution
> tests, C-harness library tests, process-image tests, scalar retained-state tests, determinism
> checks, the 20k-term deep chain, and the six fuzz targets are implemented. The execution tests
> build `stc-runtime` once per test process and link harnesses with the driver toolchain
> discovery. M4.5 phases 6–9 add the host lifecycle, preservation/remap, artifact/metadata
> fuzzing, byte-identical end-to-end double builds across output directories, combined
> launcher/embedding artifact loading, malformed-baseline/metadata negatives, concurrent
> generation pinning, and a recorded switch-latency measurement. CI runs an explicit `online`
> job on Windows x64 and Ubuntu x64 and caps fuzz inputs at one MiB. Phase 10 applies the
> documentation and handover pass. The M5 suites below are implemented, including phase 4b
> interface/attempt/reference C harnesses, module-level warm swaps that preserve interface
> values and class handles with stable descriptors, fresh-record and rollback coverage, and OOP
> fuzz seeds.
>
> **M6 phase 6 status (2026-10-26):** `stc test` and `crates/stc-test` are implemented. Discovery
> is syntax-level; each test builds one cold ABI 2 module under `target/stc/test/<artifact>` with
> the synthetic `STC_TEST` configuration; the test host is a generated launcher over the runtime's
> `stc_rt_test_main`. `STC_ASSERT` lowers to the existing assert service (the host records and
> continues; a driver sidecar maps span IDs to messages/locations), `STC_CLOCK_ADVANCE(T#…)`
> advances the mock clock through host ABI 2.2 service 12, and `STC_IO_SET_INPUT(offset, value)` /
> `STC_IO_GET_OUTPUT(offset)` access the process image. The runner enforces per-test timeouts,
> kills on expiry, parses the one-line result, and supports filters, `--list`, `--no-run`,
> expected traps, `-- --nocapture`, and human/JSON reports. Production builds exclude test units
> and assert calls.
>
> **M6 phase 10 status (2026-10-27):** the conformance corpus, UI snapshots, `xtask`, rule
> traceability, fuzz smoke, and the CI matrix are active. `tests/conformance` has 29 cases in 17
> areas with `.meta.toml` rule annotations; `tests/ui` has 25 diagnostic snapshots with
> `//~ ERROR`/`//~ WARNING` line annotations and committed `.stderr` renders;
> `crates/xtask` implements `cargo xtask test --suite ui|conformance|execution|projects` and
> `cargo xtask traceability --check`; `fuzz_manifest` and the six artifact/compiler targets are
> seeded; and the `ui`, `conformance`, `execution`, `projects`, `traceability`, `lean`, and
> `fuzz-smoke` jobs run on the documented matrix. The phase also exposed and fixed a UTF-8
> dynamic-index codegen defect (`STC-E-CODEGEN-002`).

## 1. Layers of testing

| Layer | What it proves | Location |
| --- | --- | --- |
| Lexer unit/snapshot | Tokenization, spans, trivia, errors | `crates/stc-lexer/tests` |
| Parser/CST | Grammar coverage, round-trip, recovery | `crates/stc-parser/tests`, `tests/ui` |
| Analyzer | Names, types, conversions, OOP rules, diagnostics | `crates/stc-sema/tests`, `tests/ui` |
| HIR lowering | Resolution, explicit casts, call targets | `crates/stc-sema/tests` |
| IR generation | Deterministic IR, ABI, structure, clang verification, snapshots | `crates/stc-codegen/tests` |
| Execution | Generated executables, library harnesses, traps, process image | `crates/stc-driver/tests` |
| Conformance | Profile feature matrix and edge cases | `tests/conformance` |
| Standard library | Functions and FBs | `crates/stc-codegen/tests`, `crates/stc-driver/tests/execution.rs`, `tests/conformance/stdlib` |
| Pragmas | JSON payload parsing, balanced braces, non-nesting, validation, precedence | `crates/stc-parser/tests`, `tests/ui` |
| I/O metadata | Process-image layout, `AT` type checks, binding hints/overrides, unbound defaults | `crates/stc-codegen/tests`, `crates/stc-driver/tests/state_io_cli.rs`, `tests/conformance/io` |
| State/retain | Warm/cold restart, layout hash, CRC/corruption, atomic write, mismatch policy | `crates/stc-runtime/tests`, `crates/stc-driver/tests/state_io_cli.rs`, `tests/conformance/state` |
| Online change | Load/prepare/activate/commit/rollback/unload, preservation, cold download, O(1) switch, jitter | `crates/stc-runtime/tests`, `crates/stc-driver/tests/online_cli.rs` |
| GOT/manifest | `GotId` identity/rename/move, exact-parent, download/consume, byte-identical double builds, corrupt/manifest fuzzing | `crates/stc-got/tests`, `crates/stc-codegen/tests`, `crates/stc-driver/tests` |
| Descriptors/OOP | Runtime descriptors, vtable cells, handle preservation, interface nullability, place descriptors | `crates/stc-runtime/tests/descriptor_lifecycle.rs`, `crates/stc-codegen/tests`, `tests/conformance/oop` |
| Debug/monitoring | Typed read/write/force/watch, breakpoints/step/call stack, DWARF/CodeView, swap teardown | `tests/debug` (M6.5 target) |
| Real-time boundary | Switch latency, maintenance jitter, service-thread contention, generation pinning | `crates/stc-runtime/tests` |
| Formatter | Idempotence, comment/trivia preservation, strict output, `--check` | `crates/stc-fmt/tests`, `crates/stc-driver/tests/fmt_cli.rs` |
| Syntax API | `SyntaxKind` discriminants/names stable and append-only; semver surface | `crates/stc-syntax/tests` |
| Lean | Exact-function models and proofs | `lean/`, `tests/vectors` |
| CLI/manifest | Verbs, exit codes, JSON, configuration, dependency graph, `stc.lock`/`--locked` | `crates/stc-manifest/tests`, `crates/stc-driver/tests/cli_project.rs`, `project_cli.rs` |
| Project workflow | `new/check/build/run/test/fmt/clean` end-to-end, rename/download/diff/watch | `crates/stc-driver/tests/projects.rs`, `online_cli.rs` |
| Fuzzing | Robustness of lexer/parser/analyzer/codegen | `fuzz/` |
| Cross-target | IR/object emission for targets other than host | CI |
| Host portability | Full workspace tests and end-to-end CLI on Linux x86-64 and Linux AArch64 | CI |

## 2. Built-in test suite

### 2.1 Test discovery

Tests are POUs marked with `{stc:test}`:

```st
{stc:test}
FUNCTION AddWorks : BOOL
VAR
  r : DINT;
END_VAR
  r := Add(2, 3);
  STC_ASSERT(r = 5, "2 + 3 should be 5");
  AddWorks := TRUE;
END_FUNCTION
```

```st
{stc:test {"cycles": 25}}
PROGRAM TimerRuns
VAR
  t : TON;
  done : BOOL;
END_VAR
  t(IN := NOT done, PT := T#10ms);
  done := t.Q;
  STC_ASSERT(done, "timer should complete");
END_PROGRAM
```

Rules:

- `{stc:test}` is valid on `FUNCTION`, `FUNCTION_BLOCK`, and `PROGRAM`.
- A test `FUNCTION` must have no inputs and return `BOOL`. It is called once with a fresh runtime.
  It passes if it returns `TRUE` and no assertion failed.
- A test `FUNCTION_BLOCK` is instantiated once, its body is run for `cycles` scans (default 1),
  and it passes if the body does not trap, no assertion failed, and the optional output
  `PASSED : BOOL` is `TRUE` (if declared).
- A test `PROGRAM` is configured as the selected program, run for `cycles` scans, and checked for
  traps/assertions. Program tests are integration tests; they may inspect globals through
  assertions.
- `{stc:test {"cycles": N}}` overrides the default cycle count; `[[test.case]]` in `stc.toml` can
  also override by POU name. The JSON payload may also carry `timeout` and `expect-trap`.
  A conflict between the pragma and the manifest is `STC-E-PROJECT-008`.
- Test POU names must be unique within the package being tested; a duplicate is
  `STC-E-TEST-002`. A malformed test signature (function inputs/return, FB/program shape, or a
  `PASSED` output that is not `BOOL`) is `STC-E-TEST-003`. A source that is both a project source
  and a test source, or a test source outside the project, is `STC-E-TEST-004`.
- Tests may live in the current package or in a separate test project that lists the code under
  test in `[dependencies]`. `stc test` compiles the dependencies but discovers and runs only test
  POUs in the current package.
- Tests are compiled into test-only artifacts; v1 builds one artifact per test (§2.5). Project
  `main`/executable code is not linked into a test artifact unless the test body needs it.
- `stc build` (debug or release) does not load `[test].sources` and excludes `{stc:test}` POUs
  from semantic analysis/codegen. Normal source files are still parsed; syntax errors in a normal
  file are reported even if the file also contains test units. `stc test` includes test sources
  and units and enables the assert prelude. Release builds additionally guarantee no assert calls
  are emitted.

### 2.2 Assertions

```st
FUNCTION STC_ASSERT : BOOL
VAR_INPUT
  condition : BOOL;
  message : STRING;   (* optional in a future overload *)
END_VAR
END_FUNCTION
```

- `STC_ASSERT(condition, message)` records a failure with the call site's source span and message.
  The ABI `assert` service carries only the condition, code, and span ID, so the driver records
  each call site's message in a span-keyed sidecar next to the test artifact; a string literal
  supplies its value and any other string expression contributes its source text.
- Assertions do not abort the test by default; the runner collects all failures.
- `STC_ASSERT` is **test-only**. It resolves only in test-only units: POUs marked `{stc:test}` and
  sources listed in `[test].sources`.
- Production builds (debug and release) exclude test-only units and their asserts before
  semantic analysis/codegen. A production POU using `STC_ASSERT` is `STC-E-ASSERT-001`; a
  production POU referencing a test-only POU is `STC-E-TEST-001`. `STC_ASSERT` in a context that
  is neither a test-only unit nor a production POU body, or with an invalid argument form, is
  `STC-E-ASSERT-002`.
- Release artifacts contain no test POUs and no `stc_rt_assert` calls.
- Equality helpers (`STC_ASSERT_EQ`) are future conveniences; comparison expressions are enough
  for v1.
- A test that traps (overflow, bounds, null) fails with the trap code and span.

### 2.3 Runner behavior

- Each test runs in a separate process with its own test artifact. The runner does not select a
  test inside a shared binary; the artifact is built for one test (§2.5).
- A fresh host and module are initialized per test; globals/persistent roots do not leak between
  tests. Tests that exercise online change explicitly opt into a shared long-lived host.
- Default timeout: 5 s; per-test override via `[[test.case]] timeout`.
- Mock clock starts at `[test].mock-clock` (`0ns` by default). Time is latched once at the start
  of each `stc_module_cycle`; two timers in the same scan see the same value. Tests schedule the
  next cycle's time through the test prelude:

  ```st
  STC_CLOCK_ADVANCE(T#10ms);
  ```

  It lowers to host ABI 2.2 service ID 12, `set_cycle_time_ns(int64_t)`, as
  `set(cycle_now_ns() + duration)`; it sets the next cycle's latched time and does not change the
  time already latched in the current scan (D-157).
- Default I/O backend is memory; tests set input bytes with `STC_IO_SET_INPUT(offset, value)`
  and read outputs with `STC_IO_GET_OUTPUT(offset)`, both lowered through the existing I/O
  services. Offsets are byte offsets into the input/output image; values are `DINT` low bytes.
- A test may declare expected traps via `[[test.case]] expect-trap = "STC_TRAP_DIV_ZERO"` (or the
  `{stc:test}` payload); the runner passes if the test host reports that trap code and fails on
  any other trap or on a timeout.
- Assertions use the existing ABI 2 `assert` service; the test host records the failure and
  continues instead of trapping.
- Output: human-readable by default, with one line per test and failure details;
  `--message-format json` emits the stable shape in
  [CLI and Manifest](CLI-and-Manifest.md) §1.6 (`name`, `status`, `duration_ms`, `failures`,
  `trap`).

### 2.4 Filters and options

```sh
stc test                     # all tests
stc test Timer               # substring filter on POU name
stc test --list              # list discovered tests
stc test --no-run            # build the test artifacts only
stc test -- --nocapture      # pass through runtime output
# tests run sequentially in v1; no --test-threads
stc test --timeout 2s        # override the default
```

### 2.5 Test build mode

M6 phase 0 fixes the per-test build interface (D-157). `stc test` builds one test artifact per
test case; the artifact is a normal ABI 2 module/manifest pair plus a test host, not the product
`exe` and not a shared test registry.

- **Body.** A function test gets a synthetic wrapper program that calls the function once and
  asserts its `BOOL` result. An FB test gets a wrapper that instantiates the FB and calls it once
  per scan; because FB fields are `PROTECTED` by default, the test host reads the optional
  `PASSED : BOOL` leaf through its GOT root instead of source access. A program test uses the
  program itself.
- **Synthetic configuration.** Each artifact has one synthetic configuration `STC_TEST`, resource
  `STC_TEST_RES`, task `STC_TEST_TASK`, processor `STC_TEST_CPU`, instance `STC_TEST_INSTANCE`,
  and wrapper `STC_TEST_WRAPPER`. The lexer rejects leading/trailing double underscores, so these
  lexable single-underscore spellings replace the phase-0 `__STC_TEST__` sketch. The task has no
  interval, priority, or `SINGLE`; the test host sets its guard before each cycle, exactly as the
  scheduler would. The synthetic names are reserved and are not visible to user code.
- **Build policy.** Test builds are always cold-layout: they do not read or write the runtime's
  active manifest store, do not use online change, do not load or save retained state, and are
  excluded from `[build].emit`. Dependencies compile as production packages; only the current package's
  test POUs are discovered and run.
- **Runtime settings.** The I/O backend is `memory` unless `[test].io` overrides it; the mock
  clock starts at `[test].mock-clock`; the host uses the existing I/O services, the existing
  assert service (ID 11) with a recording sink, and the appended test-clock service (ID 12).
- **Result protocol.** The test host prints exactly one result line to stdout:

  ```json
  {"kind":"stc-test-result","name":"TimerBlink","status":"pass","assertions":[],"trap":null,"cycles":25}
  ```

  `status` is `pass`, `fail`, or `trap`; `assertions` carries failures with
  `message`/`file`/`line`/`column`; `trap` names the trap when the module trapped. The runner
  adds `duration_ms`, maps a killed process to `timeout`, and emits the public JSON shape in
  [CLI and Manifest](CLI-and-Manifest.md) §1.6. Test-body output is captured unless
  `-- --nocapture`, so the result line stays parseable; a missing or malformed result line fails
  the test as an internal test-host error.
- **Isolation and ordering.** Each test runs in its own process, sequentially in v1. Discovery
  order is package order then test name; artifact file names derive deterministically from the
  package and test name.
- **Exclusion and diagnostics.** Production builds parse `[test].sources` only for syntax errors
  and exclude `{stc:test}` POUs before analysis/codegen. `STC_ASSERT` in production is
  `STC-E-ASSERT-001`; a reference to a test-only POU is `STC-E-TEST-001`; invalid test
  signatures, duplicates, and source conflicts use `STC-E-TEST-002/003/004`.
- **Contract boundary.** No manifest/descriptor/state format changes; the only ABI addition is
  service ID 12. Test artifacts never participate in a baseline or a swap, so they cannot affect
  production identity or retained state.

## 3. Compiler tests

### 3.1 Snapshot tests

- Lexer: token/trivia stream for each corpus file.
- Parser: compact CST dump and typed AST dump.
- HIR: resolved names, types, and explicit casts.
- Diagnostics: code, message, primary span, labels, notes, suggestions.
- IR: normalized `.ll` files.

Use `insta` (or an equivalent snapshot tool) with `cargo insta review`. Snapshots are committed.
Normalization removes absolute paths, temporary names beyond the documented numbering, and
target-specific datalayout lines.

M1 and M2 use an equivalent hand-rolled harness: `STC_UPDATE_SNAPSHOTS=1` regenerates committed
snapshots, matching the lexer's workflow. The parser corpus is `crates/stc-parser/tests/corpus`
and its snapshots are `crates/stc-parser/tests/snapshots`; a traceability test maps every
Grammar §2–§10 production and every `STC-GRAM-*`/`STC-E-GRAM-*` code to named positive and
negative tests. `tests/ui` and `xtask` are implemented (M6), and the UI harness uses the same
annotation/`STC_UPDATE_SNAPSHOTS=1` workflow.

### 3.2 UI tests

`tests/ui/*.st` with expected `.stderr` files:

- one file per diagnostic scenario, or multiple errors in one file,
- expected output uses the human diagnostic format,
- `//~ ERROR STC-E-...` annotations mark the expected line and code (rustc-style),
- the harness fails on unexpected diagnostics, missing diagnostics, or changed spans.

### 3.3 Property tests

- Lexer/parser: arbitrary source text never panics; token/trivia spans cover the input exactly;
  CST round-trip holds.
- Numeric parsing: decimal/real/based/underscore forms round-trip to values and back where
  defined.
- Name normalization: `ABC`, `abc`, `aBc` resolve identically.
- Constant folding: the folded value equals the runtime value for a sampled domain.
- IR: emitting the same HIR twice yields identical bytes.

### 3.4 Fuzzing

Targets:

- `fuzz_lexer`: arbitrary bytes -> token stream; no panic/hang.
- `fuzz_parser`: arbitrary bytes -> CST; no panic/hang; depth limit respected.
- `fuzz_analyzer`: valid-ish token streams -> diagnostics; no panic.
- `fuzz_codegen`: clean parse/analysis -> IR/metadata/GOT model; deterministic, bounded, no panic.
- `fuzz_manifest`: arbitrary TOML -> manifest errors, no panic.
- `fuzz_got_manifest`: arbitrary JSON/binary manifest bytes -> validated records or errors, no
  panic, bounded allocation; no module execution.
- `fuzz_module_descriptor`: arbitrary descriptor/metadata bytes -> accepted/rejected cleanly, no
  panic; no native loading in the fuzz target.

`fuzz_manifest` targets the M6 `stc.toml`/lockfile parser. The other six targets are
implemented. `fuzz_got_manifest` and `fuzz_module_descriptor` validate artifact bytes only; they
never map, load, or execute native module code. Committed seeds under `fuzz/seeds/` provide a
compiler-produced class/interface manifest and descriptor blob plus decodable malformed
TYPEINFO, VTABLE, and interface-map variants, and CI copies them into both artifact corpora.
Use `cargo-fuzz` with a bounded input size and a
time limit. CI runs each target for 60 s with `-max_len=1048576`; run a longer campaign
locally/nightly.

## 4. Conformance corpus

`tests/conformance/<area>/<case>.st` plus a small expectation file:

```toml
# case.meta.toml
profile = "stc-2013-oop-utf8-v1"
kind = "compile-pass"        # compile-pass | compile-fail | run-pass | run-fail | trap
expected_exit = 0
expected_stdout = "..."
expected_trap = "STC_TRAP_OVERFLOW"
cycles = 1
rules = ["STC-GRAM-003"]
```

Areas:

- lexical (comments, literals, escapes, UTF-8),
- types (all elementary types, subranges, enums, static arrays, conformant `ARRAY [*]` parameters,
  structs, references),
- conversions and overloads,
- statements/control flow,
- expression precedence (Annex A comparison split, `**`, unary, short-circuit),
- syntax modes (strict semicolons by default; `--relaxed` accepted/rejected cases),
- functions/FBs/programs,
- OOP (inheritance, interfaces, override, access, THIS/SUPER, assignment attempt),
- namespaces/USING,
- configuration/tasks,
- I/O (`AT`, process image, pragma hints, project bindings, unbound defaults),
- retained state (warm/cold restart, layout changes, corruption, atomic save),
- strings (STRING/WSTRING/USTRING),
- the UTF-8 extension (lexing, length, indexing, conversions, supplementary scalars),
- standard functions/FBs,
- project (manifest, lockfile, local dependencies, project CLI),
- traps and diagnostics.

Every implemented rule ID must map to at least one case. A generated traceability report lists
missing mappings and fails CI if a rule is marked implemented without a test.

### 4.1 Online-change and manifest suite

`crates/stc-runtime/tests` and `crates/stc-driver/tests` exercise the host/module lifecycle on
both hosts:

- cold install, code-only online change, field add/remove/reorder, tracked rename/move;
- untracked rename/removal diagnosis and explicit allow/cold paths;
- incompatible type/layout/string/array/`OVERLAP` changes: reject or explicit reset;
- missing, invalid, unpaired, and stale manifests: cold download, no merge/rebase;
- prepare failure, trap in the rollback window, and unload after grace;
- preserved FB/class/interface state and descriptor shape checks;
- preserved interface values and class handles across a real module warm swap: stable TYPEINFO,
  VTABLE, record, handle, and per-pair vtable addresses, activation-patched pair entries, and
  dispatch to the new method cells;
- fresh object records, handles, leaf roots, and TYPEINFO descriptors initialized once at
  prepare and released on rollback while preserved records keep their address and state;
- vtable-shape changes rejected against an active baseline, with the runtime registry as the
  load-time defense;
- direct-variable preservation/remap and I/O reconfiguration;
- O(1) switch latency (measured by the ignored `switch_latency` test), bounded maintenance
  jitter, service-reader pinning, and unload safety;
- double-build byte equality for module, metadata, and manifest;
- canonical JSON/binary round-trip, digest pairing, slot/table ordering, and descriptor/manifest
  equality per [GOT Artifacts and Schemas](GOT-Artifacts.md) §7–§12;
- manifest export/download and a second build consuming the active baseline;
- cold start, warm restart, online attach, and cold download state behavior.

The suite treats a corrupt or hostile manifest as a parser test, not as a reason to execute
arbitrary module code.

### 4.2 Debug and monitoring suite

`tests/debug` covers typed reads/writes/forces/watches, removed-field rejection across a swap,
breakpoint binding/unbinding, step/continue, call-stack and optimized-out reporting, DWARF/
CodeView inspection, the stop/idle session bound (pause expiry, state clearing, and resume with a
fresh deadline), force teardown, session authentication/audit, and the rule that every online swap
ends the debug session. Debug/release tests assert that release artifacts contain no debug hooks
and have distinct build IDs/manifests.

The M6.5 debug suite also covers the externally fixed contracts of D-173–D-179:

- protocol framing, version negotiation, request IDs, error objects, size limits, timeouts, rate
  limits, and unknown-key rejection against `stc-debug/1`;
- token authentication, the four scopes (`observe`, `control`, `write`, `force`), protected-data
  auditing, fail-closed behavior when the audit log is unavailable, and the absence of any raw
  memory operation;
- one-session enforcement, stop output hold, disconnect/timeout/reconnect behavior, and
  teardown on swap;
- checkpoint-table encoding, dense IDs, source-span binding, service ID 13 at ABI minor 3, the
  entry-flag/required-feature agreement, release/test hook-free builds, and
  `STC-E-MODULE-002` on mismatch;
- default debug vs `--release` out-dirs, build-ID/manifest distinctness, deterministic
  DWARF/CodeView paths/order (no timestamps), sidecar naming, and `STC-E-PROJECT-010` for
  conflicting debug settings;
- traceability allowlist status for the `STC-*-DEBUG-*` family until phases 3–8 implement the
  corresponding behavior.

## 5. Lean verification

### 5.1 Scope

"Proofs for exact functions" means:

- integer arithmetic (add/sub/mul/div/mod, checked overflow),
- bitwise and/or/xor/not, shifts, rotates,
- conversions between integer/bit types,
- comparisons, selection (`SEL`, `MIN`, `MAX`, `LIMIT`, `MUX`),
- boolean operations,
- exact string character-count and UTF-8 byte-length operations where the semantics are discrete.

Floating-point transcendentals, platform libm behavior, and NaN/infinity edge cases are **not**
proved; they are tested against high-precision reference vectors with documented tolerances.

### 5.2 Lean project

```text
lean/
  lakefile.lean
  lean-toolchain          # pinned to Lean 4.35.0
  Stc/
    Exact/
      Int.lean            # signed/unsigned widths, overflow, div/mod
      Bits.lean           # bitwise, shifts, rotates
      Conv.lean           # conversions
      Select.lean         # SEL/MIN/MAX/LIMIT/MUX
      Bool.lean
      Strings.lean        # exact character-count/UTF-8-byte-length operations
    Vectors/
      Gen.lean            # executable vector generator
  VectorCli.lean          # lake exe vectors
```

- No Mathlib dependency in v1; core Lean + `BitVec`/`bv_decide`/`omega` as available in 4.35.
- Each exact function has:
  1. a Lean model with a named predicate/function,
  2. theorems for the properties that matter (range, sign, overflow, round-trip),
  3. a `#eval`-generated vector set for implementation tests.
- CI runs `lake build` and fails on sorry/admit (configurable; v1 requires no `sorry` in the
  exact modules).
- `lake exe vectors` writes `tests/vectors/exact/*.json`, which is committed and consumed by Rust
  tests. The generator is deterministic and versioned.
- The proof/implementation boundary is explicit and resolved by A-006: proofs are about the Lean
  model; Rust tests validate the implementation against the model's vectors. A future refinement
  proof (Rust->Lean via Aeneas/Charon, or Lean->C extraction) is out of v1 scope.

### 5.3 Vector format

```json
{
  "function": "ADD",
  "version": 1,
  "model_version": 1,
  "generator": "0x75d79f07d61818dc",
  "cases": [
    { "a": 127, "b": 1, "result": "overflow", "type": "SINT" },
    { "a": -7, "b": 3, "result": -4, "type": "SINT" }
  ]
}
```

- One file per operation groups the operation across its IEC type instantiations; every case
  carries a `type` field. The conversion file carries the full 16-by-16 ordered `X_TO_Y` matrix
  over BOOL, the integer types, the bit strings, and CHAR/WCHAR/UCHAR, with `from`/`to`/`value`
  case fields.
- Values are decimal strings for widths > 64 bits, integers otherwise.
- Every vector file records the format version, the Lean model version, and an FNV-1a 64 digest
  of the canonical case text (`generator`); the 30 committed files under `tests/vectors/exact/`
  are regenerated by `lake exe vectors`.
- Rust tests fail if a vector is missing for an implemented exact function or type pairing.

## 6. CI

### 6.1 Jobs

| Job | Runs |
| --- | --- |
| `fmt` | `cargo fmt --all --check` |
| `clippy` | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| `test` | `cargo test --workspace --locked` on Windows x64 and Ubuntu x64; Linux AArch64 after basic development (D-042) |
| `coverage` | `cargo llvm-cov --workspace --locked` on Ubuntu x64; job summary, LCOV/JSON/HTML artifacts, and the `badges`-branch badge on `main` |
| `ui` | diagnostic snapshots on Windows x64 and Ubuntu x64 |
| `conformance` | conformance corpus on Windows x64 and Ubuntu x64 |
| `execution` | compile/run tests on Windows x64 and Ubuntu x64 |
| `projects` | end-to-end project and online workflow on Windows x64 and Ubuntu x64 |
| `online` | hot-swap/manifest/cold-download suite on Windows x64 and Ubuntu x64 |
| `traceability` | `cargo xtask traceability --check` on Ubuntu x64 |
| `debug` | M6.5 target: typed debug, breakpoint/step, DWARF/CodeView, swap-teardown suite |
| `lean` | `lake build` + vector check |
| `fuzz-smoke` | 60 s per target |
| `cross-target` | `clang --target=aarch64-... -c` smoke (IR/object) |
| `docs` | required-document, spec link, and profile checks; Markdown lint |

The workflow is `.github/workflows/ci.yml`; the documentation checks are implemented in
`tools/ci/check_docs.py`. The `docs` job runs from M0. Jobs that need the Rust workspace, suite
harnesses, Lean project, fuzz targets, or cross-target IR fixtures stay skipped until those inputs
exist, so the skeleton never reports a pass for a job that did not run. The `online` job runs the
GOT identity, descriptor/manifest projection, host lifecycle, online-change CLI, and
launcher/embedding artifact suites; the `projects` job runs `new/check/build/run/test/fmt/clean`
and the rename/download/diff/watch workflow; the `traceability` job checks every implemented rule
has test evidence. The `coverage` job runs the workspace tests under LLVM instrumentation, reports
totals and the lowest-coverage files, uploads the LCOV/JSON/HTML reports, and publishes the README
coverage badge to the `badges` branch on `main`; it is reporting-only, and the `test` job remains
the pass/fail gate.
Line coverage is a reporting metric; rule coverage remains the semantic traceability check in §7.
The M6 UI, conformance, execution, projects, traceability, Lean, and fuzz-smoke
jobs are active on the documented matrix.

### 6.2 Matrix

- Primary: Windows x64, `x86_64-pc-windows-msvc`; the development host is Windows 11 and CI uses
  GitHub's `windows-2025` runner. Pinned clang/LLVM 23.1.4.
- Secondary: Ubuntu 24.04 (`ubuntu-24.04`), `x86_64-unknown-linux-gnu` — workspace tests,
  diagnostic snapshots, conformance corpus, execution tests, project workflow, and end-to-end CLI.
- Runner images are pinned (`windows-2025`, `ubuntu-24.04`); `-latest` labels are not used, so
  image migrations do not silently change the matrix.
- Host portability (after basic development, D-042): native or emulated Linux AArch64
  (`aarch64-unknown-linux-gnu`) host jobs. Emulation is a smoke gate; release sign-off uses
  native ARM64 runners. Further `aarch64` cross-target object emission remains in the
  `cross-target` job.
- Rust toolchain pinned in `rust-toolchain.toml`; Lean pinned in `lean/lean-toolchain`.
- CI builds with `--locked`; a changed lockfile without a manifest change fails the job.
- CI caches cargo and Lake builds; the clang version is printed in every job.

### 6.3 Failure policy

- Any error diagnostic, test failure, snapshot mismatch, Lean `sorry`, fuzz crash, or unsupported
  silent acceptance fails the job.
- Warnings are denied in CI (`-D warnings`), with documented allow-list exceptions.
- Flaky tests are quarantined and fixed; timeouts are not silently retried.

## 7. Traceability

- Every rule ID (`STC-*`) appears in at least one test name, test annotation, or conformance
  metadata.
- `cargo xtask traceability` scans `Specification/**/*.md` and the tests/corpus and emits a
  report:
  - covered rules,
  - rules marked "not implemented",
  - rules with no test (CI failure if the rule is normative and implemented).
- Evidence is resolved in integration-test names and `STC-*` comments, unit-test sections, UI
  annotations, and conformance `rules` metadata, including shared `codes::CONSTANT` references.
  `tools/ci/traceability-allowlist.toml` records the explicit not-implemented rules.
- Coverage is semantic, not line-based: a rule is covered by a named test, not by hitting a line.

## 8. Manual and exploratory testing

- `examples/` projects are built and run in CI.
- A smoke script builds each example for `module`, `exe`, `lib`, and `ir`, then runs a finite
  cycle count and performs one online swap.
- Release candidates get a manual pass: fresh clone, toolchain install, `stc new`, build, test,
  run, JSON diagnostics, manifest download/rebuild, debug session, and cross-target object
  emission.

## 9. Open testing questions

None as of 2026-10-24. New testing questions are appended here or in
[Decisions](../Decisions.md).
