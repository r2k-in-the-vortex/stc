# Testing and Verification

Status: Draft  
Profile: `stc-2013-oop-utf8-v1`  
Related: [Project Plan](../00-Project-Plan.md), [Runtime](Runtime-and-Execution.md), [CLI/Manifest](CLI-and-Manifest.md)

## 1. Layers of testing

| Layer | What it proves | Location |
| --- | --- | --- |
| Lexer unit/snapshot | Tokenization, spans, trivia, errors | `crates/stc-lexer/tests` |
| Parser/CST | Grammar coverage, round-trip, recovery | `crates/stc-parser/tests`, `tests/ui` |
| Analyzer | Names, types, conversions, OOP rules, diagnostics | `crates/stc-sema/tests`, `tests/ui` |
| HIR lowering | Resolution, explicit casts, call targets | `crates/stc-sema/tests` |
| IR generation | Deterministic IR, ABI, structure | `tests/codegen` |
| Execution | Generated programs behave correctly | `tests/execution` |
| Conformance | Profile feature matrix and edge cases | `tests/conformance` |
| Standard library | Functions and FBs | `tests/stdlib` |
| Lean | Exact-function models and proofs | `lean/`, `tests/vectors` |
| CLI/manifest | Verbs, exit codes, JSON, configuration, dependency graph, `stc.lock`/`--locked` | `tests/cli` |
| Fuzzing | Robustness of lexer/parser/analyzer | `fuzz/` |
| Cross-target | IR/object emission for targets other than host | CI |

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
{stc:test(cycles := 25)}
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
- `{stc:test(cycles := N)}` overrides the default cycle count; `[[test.case]]` in `stc.toml` can
  also override by POU name. A conflict is an error.
- Test POU names must be unique within the package being tested.
- Tests may live in the current package or in a separate test project that lists the code under
  test in `[dependencies]`. `stc test` compiles the dependencies but discovers and runs only test
  POUs in the current package.
- Tests are compiled into a separate test binary; project `main`/executable code is not linked
  into the test binary unless a test needs it.

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
- Assertions do not abort the test by default; the runner collects all failures.
- `STC_ASSERT` is a test/development facility. It is available in all builds in v1 but is not part
  of the IEC surface; `{stc:test}` code should not ship in production modules.
- Equality helpers (`STC_ASSERT_EQ`) are future conveniences; comparison expressions are enough
  for v1.
- A test that traps (overflow, bounds, null) fails with the trap code and span.

### 2.3 Runner behavior

- Each test runs in a separate process. The runner passes `--test <name>` to the test binary.
- A fresh runtime is initialized per test; globals do not leak between tests.
- Default timeout: 5 s; per-test override via `[[test.case]] timeout`.
- Mock clock starts at `0ns` by default; `mock-clock` sets it. Time is latched once at the start
  of each `stc_cycle`; two timers in the same scan see the same value. Tests schedule the next
  cycle's time through the test harness API:
  ```st
  STC_CLOCK_ADVANCE(T#10ms);
  ```
  The harness API is a test-only prelude; it lowers to `stc_rt_set_cycle_time_ns` for the next
  cycle. It does not change the time already latched in the current scan.
- Default I/O backend is memory; tests can set input bytes with `STC_IO_SET_INPUT(...)` and read
  outputs with `STC_IO_GET_OUTPUT(...)` (future helpers; v1 exposes them as runtime calls).
- A test may declare expected traps via `[[test.case]] expect-trap = "STC_TRAP_DIV_ZERO"`; the runner
  passes if the process exits with that trap code.
- Output: human-readable by default, with one line per test and failure details; `--message-format
  json` emits one object per test with `name`, `status`, `duration_ms`, `failures`.

### 2.4 Filters and options

```
stc test                     # all tests
stc test Timer               # substring filter on POU name
stc test --list              # list discovered tests
stc test --no-run            # build the test binary only
stc test -- --nocapture      # pass through runtime output
stc test --test-threads 1    # serialize process launches
stc test --timeout 2s        # override the default
```

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
- `fuzz_manifest`: arbitrary TOML -> manifest errors, no panic.

Use `cargo-fuzz` with a bounded input size and a time limit. Run a smoke corpus in CI and a longer
campaign locally/nightly.

## 4. Conformance corpus

`tests/conformance/<area>/<case>.st` plus a small expectation file:

```
# case.meta.toml
profile = "stc-2013-oop-utf8-v1"
kind = "compile-pass"        # compile-pass | compile-fail | run-pass | run-fail | trap
expected_exit = 0
expected_stdout = "..."
expected_trap = "STC_TRAP_OVERFLOW"
cycles = 1
```

Areas:

- lexical (comments, literals, escapes, UTF-8),
- types (all elementary types, subranges, enums, static arrays, conformant `ARRAY [*]` parameters,
  structs, references),
- conversions and overloads,
- statements/control flow,
- functions/FBs/programs,
- OOP (inheritance, interfaces, override, access, THIS/SUPER, assignment attempt),
- namespaces/USING,
- configuration/tasks,
- strings (STRING/WSTRING/USTRING),
- standard functions/FBs,
- traps and diagnostics.

Every implemented rule ID must map to at least one case. A generated traceability report lists
missing mappings and fails CI if a rule is marked implemented without a test.

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

```
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
  "function": "ADD_DINT",
  "version": 1,
  "cases": [
    { "a": 2147483647, "b": 1, "result": "overflow" },
    { "a": -7, "b": 3, "result": -4 }
  ]
}
```

- Values are decimal strings for widths > 64 bits, integers otherwise.
- Every vector file records the Lean model version and the generator hash.
- Rust tests fail if a vector is missing for an implemented exact function.

## 6. CI

### 6.1 Jobs

| Job | Runs |
| --- | --- |
| `fmt` | `cargo fmt --check` |
| `clippy` | `cargo clippy --all-targets -- -D warnings` |
| `test` | `cargo test --workspace` on Windows x64 |
| `ui` | diagnostic snapshots |
| `conformance` | conformance corpus |
| `execution` | compile/run tests |
| `lean` | `lake build` + vector check |
| `fuzz-smoke` | 60 s per target |
| `cross-target` | `clang --target=aarch64-... -c` smoke (IR/object) |
| `docs` | spec link check / markdown lint |

### 6.2 Matrix

- Primary: Windows 11, `x86_64-pc-windows-msvc`, pinned clang/LLVM 23.1.4.
- Secondary: Ubuntu `x86_64-unknown-linux-gnu` (object/IR and execution tests).
- Optional: `aarch64` cross-target object emission.
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
- `cargo xtask traceability` scans the spec and tests and emits a report:
  - covered rules,
  - rules marked "not implemented",
  - rules with no test (CI failure if the rule is normative and implemented).
- Coverage is semantic, not line-based: a rule is covered by a named test, not by hitting a line.

## 8. Manual and exploratory testing

- `examples/` projects are built and run in CI.
- A smoke script builds each example for `exe`, `lib`, and `ir`, then runs a finite cycle count.
- Release candidates get a manual pass: fresh clone, toolchain install, `stc new`, build, test,
  run, JSON diagnostics, and cross-target object emission.

## 9. Open testing questions

- Q-013: Parallel test execution by default.
- Q-015: Whether `STC_ASSERT` should be available in production builds or gated behind a profile.
