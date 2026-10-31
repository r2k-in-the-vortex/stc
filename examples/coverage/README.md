# coverage-demo — statement coverage for `stc test`

A small library/test-style package that shows what `stc test --coverage`
reports. It has no `CONFIGURATION` and no runnable program: the whole package
exists to give the test suite production functions with known covered and
uncovered statements.

`stc` is not a safety PLC and is not responsible for safety. Coverage is test
tooling only; it is not a correctness claim and it does not gate a build.

## What it shows

| Feature | Where |
| --- | --- |
| A production function called by a test (`AddValues`) | `src/coverage.st` |
| A production function never called (`Average`) | `src/coverage.st` |
| A branch with a covered `IF`/`THEN` and an uncovered `ELSE` (`Classify`) | `src/coverage.st` |
| Two tests covering disjoint production statements | `tests/coverage_test.st` |
| A `{stc:test}` POU in a production file, excluded from the report | `src/coverage.st` |
| The native JSON, LCOV, and human reports | `target/stc/coverage/` after a coverage run |

`ExcludedHelper` is marked `{stc:test}` even though it lives in
`src/coverage.st`. Test builds compile and run it (it is one of the three
discovered tests), but its statements are test code, so codegen does not
instrument them. The committed golden report in
`crates/stc-driver/tests/coverage/expected/` proves both sides: `AddValues` in
the same file is reported, and `ExcludedHelper` is not.

## Layout

```text
coverage/
  stc.toml                 project manifest, production and test sources
  stc.lock                 committed dependency graph (no dependencies)
  src/coverage.st          production functions and one test-only POU
  tests/coverage_test.st   {stc:test} POUs
  target/                  build output (ignored)
```

## Run it

From the repository root, using the workspace binary:

```sh
cargo run -p stc-driver -- check --manifest-path examples/coverage/stc.toml --locked
cargo run -p stc-driver -- test --manifest-path examples/coverage/stc.toml --locked
cargo run -p stc-driver -- test --manifest-path examples/coverage/stc.toml --locked --coverage
cargo run -p stc-driver -- fmt --manifest-path examples/coverage/stc.toml --check --locked
```

From this directory, with an installed `stc`:

```sh
stc check
stc test
stc test --coverage
stc fmt --check
```

`stc test --coverage` writes `coverage.json` (`stc-coverage/1`) and
`lcov.info` (LCOV) under `target/stc/coverage/` and prints the human
summary after the test summary. `--coverage-out <dir>` places the two files
elsewhere.

## Expected result

```text
test AddWorks ... ok
test ClassifyPositive ... ok
test ExcludedHelper ... ok
test result: ok. 3 passed; 0 failed; 0 timed out; 0 trapped
coverage: 3/5 statements (60.0%), 3/5 lines (60.0%)
  src/coverage.st: 3/5 statements (60.0%), 3/5 lines (60.0%)
```

- `AddValues` (one statement) is hit once by `AddWorks`.
- `Average` (one statement) is never called and stays at zero.
- `Classify`'s `IF` and `THEN` statements are hit by `ClassifyPositive`; its
  `ELSE` statement stays at zero.
- `ExcludedHelper` runs but contributes no sites, so the suite has five
  statements and three covered.

Failed and trapped tests still contribute the statements they executed; only
a timed-out, crashed, or missing-data test is left out of the aggregate.
Coverage is statement-based (LCOV projects it to lines), contains no timings or
host data, and is byte-identical for identical inputs on every supported host.
