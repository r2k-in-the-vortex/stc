# WL-10 Plan — Coverage Emission for the ST Test Suite

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-31  
Branch: `WL-10` (sub-plan of `v2.0.0-planning`)  
Status: Locked 2026-10-31. Decisions C-1–C-14 are locked; no open questions remain. No
implementation has started.  
Parent: [V2.0.0-PLAN.md](V2.0.0-PLAN.md) §4 (WL-10)  
Inputs: [FutureWishlist.md](FutureWishlist.md) WL-10,
[Testing and Verification](specification/Architecture/Testing-and-Verification.md) §2,
[CLI and Manifest](specification/Architecture/CLI-and-Manifest.md) §1.2, §1.6, §2.4,
[GOT Artifacts](specification/Architecture/GOT-Artifacts.md) §7.3, §12.1,
[Runtime and Execution](specification/Architecture/Runtime-and-Execution.md),
[KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md)

## 1. Objective

Make `stc test` report coverage for the ST code under test: which executable statements the test
suite executed, attributed to the ST source file, line, and column, aggregated across the suite
and available per test. Reports are emitted in a native deterministic JSON format, in LCOV for
standard tooling and editor gutters, and as a human summary.

Coverage is test tooling only. Production builds, release artifacts, the module descriptor,
GOT/manifest, metadata sidecar, host ABI, and test outcomes are unchanged. Coverage is not a
correctness claim, and coverage of the ST program is separate from WL-2's coverage of the
compiler's Rust code.

## 2. Locked scope and boundaries

In scope:

- Statement coverage for ST code compiled into a test artifact.
- Per-test counters collected from the process-isolated runner and aggregated across the suite.
- Source-span attribution through the existing span and source-map machinery.
- Native JSON (`stc-coverage/1`), LCOV, and a human summary.
- Exclusions for test code, synthetic wrappers, standard-library bodies, and generated helpers.
- Test-build-only instrumentation; production builds and release artifacts are untouched.
- Golden reports, aggregation, determinism, cross-host, dependency-package, and example-project
  tests.
- Documentation of the coverage model, exclusions, formats, and limitations.

Out of scope:

- Branch, condition, and MC/DC coverage. Statement coverage only; the format is versioned so a
  later branch format can be added without changing this one.
- HTML reports, editor integration, and coverage gutters. LCOV is the integration surface; WL-1.1
  may consume it later.
- Coverage in `stc build`, release artifacts, deployed modules, or cross-target IR-only builds.
- Coverage of the compiler's own Rust code (WL-2) and of synthesized standard-library bodies.
- Fail-under thresholds, gating, and CI policy changes.
- Parallel test execution, incremental test builds, and per-test report streams on stdout.
- Coverage of production `PROGRAM` bodies that a test artifact does not link (§6.4, C-4).
- Changes to the host ABI, GOT/manifest artifact formats, metadata sidecar, production module
  format, or test result shapes.

## 3. Current state

### 3.1 Test pipeline

| Component | Current behavior |
| --- | --- |
| `stc test` driver (`crates/stc-driver/src/test.rs`) | Parses every package's production sources plus the root package's `[test].sources`, discovers `{stc:test}` POUs, and builds one cold test artifact per test under `target/stc/test/<artifact>` (or `--out-dir`). Each build appends a synthetic wrapper as the last virtual source file, uses the synthetic `STC_TEST` configuration, forces the hook-free debug form, and rejects online change and retained state. |
| Test host (`crates/stc-runtime/src/test_host.rs`) | A generated launcher whose `main` calls `stc_rt_test_main`. It loads one module, runs a mock-clock scan loop, records assertions instead of trapping, and prints one structured result line. Assertions are resolved through a driver-written sidecar. |
| Runner (`crates/stc-test/src/runner.rs`) | Runs one host process per test, enforces the per-test timeout, kills on expiry, and parses the result line. v1 runs tests sequentially. |
| Reports (`crates/stc-test/src/report.rs`) | Stable human lines and one JSON object per test. Coverage is absent. |

### 3.2 Statements, spans, and source mapping

- HIR statements carry source spans (`crates/stc-hir/src/hir.rs`, `StmtHir`/`StmtKindHir`).
  Executable kinds are assign, reference assign, attempt, call, `RETURN`, `IF`, `CASE`, `FOR`,
  `WHILE`, `REPEAT`, `EXIT`, `CONTINUE`; `Empty` is not executable.
- Codegen lowers every statement through `Lowerer::lower_stmt`
  (`crates/stc-codegen/src/stmt.rs`) and already records debug checkpoints there. Checkpoints
  are debug-build-only and are disabled for test builds (`collect_checkpoints` requires
  `!options.test_build`), so coverage must be an independent path.
- `build_span_table` (`crates/stc-codegen/src/unit.rs`) assigns dense span IDs and the metadata
  sidecar records the span table (`spans.files`, `spans.entries` with file/start/end).
- `SourceMap::location` (`crates/stc-span/src/source_map.rs`) converts a byte offset to a
  1-based line and a 1-based column counted in Unicode scalar values (STC-LEX-005).
- The driver already maps `FileId` to report paths: root files use the package-relative path;
  dependency files use `<package>/<relative>`; the synthetic wrapper is a virtual file with no
  report path (`test.rs`, `write_assertion_sites`).

### 3.3 Module exports and the verifier

- Modules are shared libraries. The loader resolves fixed exports with `GetProcAddress` on
  Windows and `dlsym` on Unix (`crates/stc-runtime/src/loader.rs`), and
  `Host::export_symbol` exposes any exported name of the active generation
  (`crates/stc-runtime/src/host.rs`).
- Codegen can emit a non-preemptable exported global with `SymbolAttrs::module_export()`
  (`dso_local` plus `dllexport`; `crates/stc-codegen/src/ir.rs`). A local probe on 2026-10-31
  with clang 23.1.4 confirmed the object-level export data for both targets:

  ```
  @stc_coverage_counters = dso_local dllexport global [2 x i64] zeroinitializer, align 8
  ```

  The COFF object's `.drectve` contains `/EXPORT:stc_coverage_counters,DATA`; the ELF object
  reports the symbol as Global binding with default visibility in `.bss`, and the module link
  has no version script, so a default-visibility global is exported from the shared object.
- The module verifier rejects direct access to module globals unless they are listed in
  `Contract.local_globals` (`crates/stc-codegen/src/verify.rs`, `contract()` in `unit.rs`).
  Any new directly accessed global must be allowlisted when it is emitted.

### 3.4 Test-build filtering and hosts

- Production builds exclude `{stc:test}` definitions; test builds include them. Test-only
  definitions can live in production source files, so file-based exclusion alone is not enough.
- Synthesized standard function-block bodies are emitted directly by
  `crates/stc-codegen/src/stdlib.rs` and do not lower HIR statements, so they have no coverage
  sites.
- The `test` CI job runs `cargo test --workspace --locked` on `windows-2025`,
  `ubuntu-24.04`, and `ubuntu-24.04-arm` (`.github/workflows/ci.yml`), so driver integration
  tests are cross-host by construction.
- No ST-program coverage exists today. The CI `coverage` job runs the Rust workspace under
  `cargo llvm-cov` for WL-2 and is unrelated.

## 4. Findings that shape the plan

1. **Codegen is the correct instrumentation point.** It sees exactly the statements emitted into
   the test artifact and has the span and IR builders. HIR-level instrumentation would add a
   semantic node, duplicate emission filters, and drift from what is actually compiled.
2. **A module-owned counter array avoids an ABI change.** The counter global is ordinary module
   data; increments are a GEP, load, add, and store. The test host reads it through the export
   lookup that already exists. No host service, ABI minor bump, GOT cell, descriptor field, or
   metadata key is needed. A per-hit host service would add an ABI minor, a service dispatch for
   every statement, and runtime-side bookkeeping for no benefit; a GOT root would change the
   manifest and identity.
3. **Process isolation makes per-test data natural.** Each test is a separate process with its
   own module and counters. The host writes a raw sidecar, matching the existing assertion
   sidecar pattern; the runner aggregates after the process exits.
4. **Canonicalization by source span makes aggregation independent of local IDs.** Production
   function, function-block, and method bodies are emitted in every test artifact, so the site
   universe is stable, but the report should not depend on per-artifact lowering order. Mapping
   local counters to canonical `(file, start, end, kind)` sites keeps the merge sound.
5. **Test builds are cold and non-deployable.** Instrumentation can be confined to them without
   touching production identity, descriptors, manifests, or release artifacts. Test artifacts
   never enter `[build].emit`.
6. **The verifier must allowlist the counter global.** The counter access is a direct module
   global load/store, which the structural verifier otherwise rejects.
7. **Report shapes and exit codes can stay unchanged.** Coverage travels in files, not on the
   test-result stdout stream; instrumented tests must preserve the same status, failures, trap,
   and exit semantics.
8. **Cross-host evidence is free.** The existing workspace test matrix already runs the driver
   suites on all three supported hosts.

## 5. Decisions

The decision set was locked by the owner on 2026-10-31; C-11 was locked as revised. Locked
decisions are changed only by a re-plan. Each decision follows from the current code and evidence
above.

| ID | Decision | Status |
| --- | --- | --- |
| C-1 | Instrument in codegen at `Lowerer::lower_stmt`, not in HIR or sema. A coverage site is one distinct executable statement span (`StmtKindHir` other than `Empty`, non-empty span, instrumented file). A repeated span reuses its counter. The site's report location is the span start. | Locked |
| C-2 | Each instrumented test module carries one module-owned `[N x i64]` global named `stc_coverage_counters`, zero-initialized, 8-byte aligned, non-constant, exported with `dso_local dllexport`. A site increments its cell by one before the statement executes. The test host reads the array through `Host::export_symbol` after the scan loop and before shutdown. No host ABI service, minor bump, GOT cell, descriptor field, or metadata key is added. The symbol is a test-build-only internal export, not part of the deployable module ABI. | Locked |
| C-3 | Instrument statements lowered from non-test definitions in production source files of every package in the test build (root and dependencies). Exclude `[test].sources`, `{stc:test}` POUs and their members, the synthetic wrapper virtual file, prelude/synthetic/compiler-generated definitions, synthesized standard-library FB bodies, declarations and initializers that are not statements, `Empty` statements, zero-length spans, and anything outside emitted bodies. Coverage measures exactly what the test artifact links and lowers. | Locked |
| C-4 | Production `PROGRAM` bodies not selected by the synthetic `STC_TEST` configuration are not linked into a test artifact and are outside the measured universe. This follows the existing test model and is documented as a limitation. `{stc:test}` program tests remain test code and are excluded. | Locked |
| C-5 | The test host writes one per-test raw sidecar, `<artifact>.coverage`, in the internal `stc-coverage-raw/1` format: a header with the site count, then `id count` lines for nonzero counters in ascending ID order. The driver reads it after the process exits. Missing or malformed data is a warning, never a test failure; timed-out tests are marked as having no data. The result line, stdout shape, and exit codes do not change. | Locked |
| C-6 | Map per-test counters to canonical sites by source span `(file, start, end, kind)`; identical spans across artifacts merge. Aggregate hit counts are sums; a site is covered when any test hits it. Canonical site order is `(file path, start byte, end byte)`. Per-test detail in the native report is the set of covered canonical site indices; per-test hit counts are not published. Site tables come from codegen output, not from rescanning sources. | Locked |
| C-7 | Write `coverage.json` (`stc-coverage/1`) and `coverage.info` (LCOV) under `target/stc/coverage/` by default, or `--coverage-out <dir>`. Print the human summary to stdout after the test summary in human mode and suppress it on stdout in JSON mode (log the file paths to stderr). No HTML in v2.0.0. An explicit `--coverage` is rejected with `--list` (the manifest default is ignored there); `--no-run` builds instrumented artifacts but writes no report. | Locked |
| C-8 | Statement coverage is primary: covered sites / total sites. Line coverage is the documented projection for LCOV and editors: a line is covered when any site starting on it executed, and LCOV `DA` sums the hit counts of sites starting on that line. Both metrics appear in JSON and the human summary. No fail-under threshold or gating behavior in v2.0.0. | Locked |
| C-9 | Reports contain no timestamps, durations, absolute paths, host names, build IDs, or test durations. Files and sites are canonically ordered; paths are package-relative with forward slashes; percentages use integer arithmetic with half-up rounding to one decimal. Identical sources, tests, filters, and options produce byte-identical reports on every supported host. | Locked |
| C-10 | Add `STC-W-COVERAGE-001` (a test's raw coverage data is missing or malformed and is excluded from the aggregate) and `STC-W-COVERAGE-002` (a coverage site cannot be attributed to a reportable source file and is omitted). Reuse `STC-E-BUILD-002` for report-directory/file write failures. Register the warnings in `crates/stc-manifest/src/codes.rs` and Conformance Profile §10, and record this decision in `specification/Decisions.md` with the next free `D-` number when phase 3 lands. | Locked |
| C-11 | Coverage is enabled by `[test] coverage = true` as a project default; `--coverage` forces it on and `--no-coverage` forces it off, with the CLI overriding the manifest. `--coverage` and `--no-coverage` together are a usage error. An explicit `--coverage` is rejected with `--list`; the manifest default is ignored by `--list`. `--coverage-out` requires effective coverage and is rejected with `--list`. | Locked |
| C-12 | `stc clean` and `stc clean --test` also remove the default `target/stc/coverage` directory; a custom `--coverage-out` directory is not removed. | Locked |
| C-13 | No per-package or per-file coverage filter in v2.0.0; all production packages are reported. Consumers can filter the native JSON; a filter is future work. | Locked |
| C-14 | Record an informational instrumented-vs-uninstrumented `stc test` timing in the phase 4 evidence. There is no overhead budget, threshold, or CI gate. | Locked |

## 6. Coverage model

### 6.1 Sites, counters, and instrumentation

- A site is identified by its statement span. The lowerer keeps a `BTreeMap<Span, u32>` so a
  repeated span reuses one counter.
- `Output.coverage_sites` lists `{ id, span, kind }` in ID order; `kind` is `Statement` today.
  The output is empty when coverage is disabled.
- Instrumentation emits, in the current block before the statement:

  ```
  %cov.ptr = getelementptr inbounds [N x i64], ptr @stc_coverage_counters, i64 0, i64 <id>
  %cov.old = load i64, ptr %cov.ptr, align 8
  %cov.new = add i64 %cov.old, 1
  store i64 %cov.new, ptr %cov.ptr, align 8
  ```

  The operations cannot trap, allocate, or change ST-visible values. No ordering relative to the
  statement's own effects is observable except that the counter records entry to the statement,
  including a statement that traps.
- The counter global is emitted only when at least one site exists, with
  `SymbolAttrs::module_export()`, type `[N x i64]`, initializer `zeroinitializer`, and `align 8`.
  It is added to `Contract.local_globals` for the structural verifier.
- `Options` gains a coverage option carrying the allow-list of instrumented source `FileId`s.
  Codegen rejects coverage outside a test build as an internal error, so production builds can
  never instrument even if a caller sets the flag by mistake.

### 6.2 Test-host collection

`stc_rt_test_main` accepts two internal arguments, passed by the driver only for coverage runs:

- `--coverage <path>`: write the raw sidecar at this path.
- `--coverage-sites <n>`: number of counters, from the generated artifact.

After `run_loop` returns (success, trap, or runtime error) and before the expected-trap and
`PASSED` checks and before `host.shutdown()`, the host:

1. Validates `n <= 1_000_000` (documented cap) and skips collection otherwise.
2. Resolves `stc_coverage_counters` with `host.export_symbol`; a missing symbol or null pointer
   logs a stderr warning and skips collection.
3. Reads `n` `u64` counters. Safety: the module is the active generation, the array has `n`
   elements by construction, and the test host is single-threaded while the module runs.
4. Writes the raw sidecar with the nonzero entries. A write failure logs a warning and changes
   nothing else.

Coverage collection never changes the result line, the process exit code, or the recorded
failure set. A test that times out is killed before the sidecar is written; the driver marks it
as having no coverage data. The test artifact directory is removed and rebuilt for each test on
each run, so a stale sidecar cannot be read. The raw sidecar is an internal protocol between the
generated host and the driver; only `coverage.json`, `coverage.info`, and the human summary are
stable interfaces.

### 6.3 Aggregation

For each built test the driver has the codegen site table. It maps each site to its report path
through the same `FileId` map used for assertion locations:

| Package | Report path |
| --- | --- |
| Root | package-relative path, forward slashes |
| Dependency | `<package>/<relative>`, forward slashes |
| Test sources | excluded |
| Synthetic wrapper | excluded |

The raw counter `id` indexes the artifact's site table. The driver reads the sidecar for every
test that is not a timeout, including failed and trapped tests. It maps every nonzero entry to a
canonical site key `(report path, start, end, kind)`; identical keys across tests merge. The
canonical site list is sorted by `(report path, start, end)`, and each test's covered entries
become ascending indices into that list. Aggregate hit counts are saturating sums. A site is
covered when its aggregate count is greater than zero.

A site whose report path cannot be resolved is an internal invariant violation: it is omitted
with `STC-W-COVERAGE-002`, and the rest of the report is still produced.

### 6.4 Exclusions

| Source region | Measured | Reason |
| --- | --- | --- |
| Production sources in the root and dependency packages | Yes | Code under test |
| `[test].sources` | No | Test code |
| `{stc:test}` POUs and members of test-only definitions | No | Test code |
| Synthetic test wrapper (`STC_TEST_WRAPPER`, virtual file) | No | Generated harness |
| Prelude, synthetic, and compiler-generated definitions | No | Not user code |
| Synthesized standard function-block bodies | No | No ST source; emitted directly |
| Declarations and initializer expressions | No | Not executable statements |
| `Empty` statements and zero-length spans | No | No source location |
| Unlinked production `PROGRAM` bodies | No | Not in the test artifact (C-4) |
| Shutdown-time execution | No | Collection ends before `host.shutdown()` |
| Unreachable or optimizer-removed code | Site remains, count zero | Conservative, deterministic |

Statements in a production file that belong to a `{stc:test}` definition are excluded by the
definition's test-only flag, not by the file. Methods whose enclosing POU or type is test-only are
excluded through the owner chain.

### 6.5 Determinism and identity

- Site IDs are artifact-local. Reports depend on canonical span order, never on test order or
  local IDs.
- Reports omit timings, durations, build IDs, absolute paths, and host data.
- Line endings do not affect line/column attribution; columns count Unicode scalar values.
- Test artifacts keep the source-derived build identity: the coverage option is not part of the
  source fingerprint. Test artifacts are cold, non-deployable, and rebuilt each run, so an
  instrumented and an uninstrumented artifact sharing the identity is harmless.
- The instrumentation is gated by `test_build`; production builds cannot emit the counter global,
  the site table, or any report.

### 6.6 Limitations to document

- Statement coverage only; no branch/condition/MC/DC coverage.
- Wall-clock test timeouts include instrumentation overhead. A test near its timeout may need a
  larger `--timeout` under coverage; outcomes otherwise do not change.
- A timed-out, crashed, or failed-to-load test contributes no coverage data.
- Failed tests do contribute coverage for what they executed, and expected-trap tests that pass
  contribute coverage up to the trap.
- A statement after an unconditional transfer, or one removed by the optimizer, stays in the
  universe with a zero count.
- Production `PROGRAM` bodies are outside the measured universe unless the test artifact links
  them (it does not in v2.0.0).
- Dependency entries are package-qualified labels (`<package>/<relative>`), not filesystem paths.
- Coverage is not correctness evidence and is not the rule-level traceability check in
  [Testing and Verification](specification/Architecture/Testing-and-Verification.md).

## 7. CLI and report formats

### 7.1 CLI surface

```
stc test [<filter>...] [--coverage] [--coverage-out <dir>] ...
```

| Option | Meaning |
| --- | --- |
| `--coverage` | Force coverage on for this run, overriding `[test] coverage`. Rejected with `--list`. With `--no-run`, artifacts are instrumented but no report is written. |
| `--no-coverage` | Force coverage off for this run, overriding `[test] coverage`. Conflicts with `--coverage`. |
| `--coverage-out <dir>` | Report directory; requires effective coverage. Default `target/stc/coverage` relative to the project root; an explicit value is used as given, like `--out-dir`. |

`[test] coverage = true` sets the project default; absent or false leaves coverage off. The CLI
overrides the manifest in either direction: effective coverage is off with `--no-coverage`, on
with `--coverage`, and otherwise the manifest value. `--list` ignores the manifest default and
never collects coverage; an explicit `--coverage` with `--list` is a usage error, and
`--coverage-out` is rejected with `--list` or without effective coverage.

Exit codes are unchanged: 0 pass, 1 failure or report write error, 2 usage/manifest error, 4
trap. Test filters select the tests that contribute; the report records the filters as given.
`--quiet` suppresses the human summary but still writes the files. `--message-format json` keeps
stdout for the stable per-test result objects; the coverage summary is not printed there.

The report and summary are produced regardless of test outcomes, including failures and traps.
If any test artifact fails to build, the command returns the build error and writes no report.
If no test is discovered or the filters select none, no report is written. The report directory
is created if missing, and the two report files are overwritten. The coverage directory is
independent of `--out-dir`; use `--coverage-out` to place the reports elsewhere.

`stc clean` and `stc clean --test` remove the default coverage directory as part of the test
tree. A custom `--coverage-out`, like a custom `--out-dir`, is outside the default tree and is
not removed.

### 7.2 Native JSON (`stc-coverage/1`)

`coverage.json` is pretty-printed JSON with sorted keys; all arrays are canonically ordered. The
example below shows the logical key order; serialized keys are alphabetical.

```json
{
  "protocol": "stc-coverage/1",
  "profile": "stc-2013-oop-utf8-v1",
  "package": "coverage-demo",
  "kind": "statement",
  "filters": [],
  "totals": {
    "sites": 5,
    "covered": 3,
    "percent": 60.0,
    "lines": 5,
    "lines_covered": 3,
    "line_percent": 60.0
  },
  "files": [
    {
      "file": "src/stats.st",
      "sites": 5,
      "covered": 3,
      "percent": 60.0,
      "lines": 5,
      "lines_covered": 3,
      "line_percent": 60.0
    }
  ],
  "sites": [
    { "file": "src/stats.st", "line": 4, "column": 3, "count": 1 },
    { "file": "src/stats.st", "line": 7, "column": 3, "count": 0 },
    { "file": "src/stats.st", "line": 12, "column": 3, "count": 2 },
    { "file": "src/stats.st", "line": 16, "column": 3, "count": 0 },
    { "file": "src/stats.st", "line": 20, "column": 3, "count": 1 }
  ],
  "tests": [
    {
      "name": "ClassifyPositive",
      "status": "pass",
      "coverage": "reported",
      "sites_covered": 2,
      "covered": [0, 2]
    },
    {
      "name": "SlowTest",
      "status": "timeout",
      "coverage": "timeout"
    }
  ]
}
```

- `sites` is sorted by `(file, start byte, end byte)`; the displayed `line` and `column` are the
  span start. `count` is the aggregate hit count (`u64`).
- `package` is the root package name; `protocol` is `stc-coverage/1`; `kind` is `statement` in
  v2.0.0.
- `files` is sorted by `file`; each entry summarizes the sites whose span starts in that file.
  Files with no executable sites are absent from both `files` and `sites`.
- `lines` counts distinct start lines with at least one site; `lines_covered` counts those with
  at least one hit.
- `percent` and `line_percent` are `null` when the denominator is zero; otherwise they are
  half-up rounded to one decimal.
- `tests` is in run order. `coverage` is `reported`, `missing`, or `timeout`. `covered` contains
  ascending indices into `sites` and is present only for `reported` tests.
- `filters` records the positional filters as given, `[]` when none.

### 7.3 LCOV

`coverage.info` uses the standard tracefile shape. Each record covers one file, and records are
sorted by file path:

```
TN:
SF:src/stats.st
DA:4,1
DA:7,0
DA:12,2
DA:16,0
DA:20,1
LF:5
LH:3
end_of_record
```

- `SF` is the same package-relative, forward-slash label as the JSON.
- One `DA` line per distinct start line with at least one site, ascending; the hit count is the
  sum of the site counts starting on that line.
- `LF` is the number of `DA` lines; `LH` the number with a nonzero count.
- No `BRDA`, function, or checksum records in v2.0.0.

### 7.4 Human summary

After the test result summary in human mode:

```
coverage: 3/5 statements (60.0%), 3/5 lines (60.0%)
  src/stats.st: 3/5 statements (60.0%), 3/5 lines (60.0%)
```

Files are sorted by path. With zero sites the first line is `coverage: no sites`. In JSON mode
the summary is suppressed on stdout; the report paths are logged to stderr. `-q` suppresses the
summary in human mode; files are still written.

## 8. Work breakdown

| Phase | Work | Acceptance |
| --- | --- | --- |
| 0 | Lock C-1–C-14 and record this plan in the parent. Confirm the schema, exclusion rules, and output locations. | Decisions locked; no open questions; parent plan links this sub-plan; no code. |
| 1 | Codegen: coverage option, site table, span-keyed counters, exported global, verifier allowlist, test-only/file filters, output field. | Codegen tests show the counter global and one increment per site; no instrumentation when disabled or outside a test build; test-only/test-source/synthetic exclusions; deterministic IR; structural verifier passes. |
| 2 | Runtime and `stc-test`: test-host options, counter readout, raw sidecar writer, raw parser, caps and warnings. | An instrumented artifact's host writes the expected raw sidecar; uninstrumented hosts are unchanged; malformed/missing data is handled without changing test outcomes. |
| 3 | Driver: CLI flags and validation, codegen plumbing, per-test point mapping, aggregation, JSON/LCOV/human writers, default directory, warnings, `stc clean`. Aggregator unit tests. | `stc test --coverage` writes all three reports; JSON stdout is unchanged; exit codes are unchanged; clean removes the default directory; aggregation tests pass without clang. |
| 4 | Evidence: example project, golden reports, aggregation/determinism/equivalence/production-isolation/dependency/timeout/filter/opt-level tests; cross-host runs; informational overhead measurement. | All tests pass on Windows x64, Linux x64, and Linux AArch64 in the existing workspace test matrix; the overhead figure is recorded and non-gating. |
| 5 | Documentation and review: specification sections, warning codes, user guide, limitations, README; mark WL-10 done and consolidate the plan into the specifications. | Documentation is complete and reviewed; no production format, ABI, or behavior change. |

### 8.1 Phase 1 detail

- `crates/stc-got`: add the internal `stc_coverage_counters` symbol constant so codegen and the
  runtime share one spelling. Document it as a test-build-only export.
- `crates/stc-codegen`:
  - Add `CoverageKind`, `CoverageSite`, and `Output.coverage_sites`.
  - Add the coverage option with the instrumented `FileId` allow-list.
  - Track the current body owner so statements in test-only definitions and members are skipped.
  - Add `record_coverage` in `lower_stmt` after the debug location is set and before the
    statement is lowered; dedupe by span.
  - Emit the counter global in `emit_module_data` and allowlist it in `contract()`.
  - Reject coverage in non-test builds.
- Tests: rendered IR contains the global and increments; excluded regions have no increments;
  non-coverage builds are byte-identical to today; repeated generation is deterministic; the
  structural verifier accepts the instrumented module.

### 8.2 Phase 2 detail

- `crates/stc-runtime/src/test_host.rs`: parse `--coverage`/`--coverage-sites`, restructure
  `run_test` so the readout happens once after the run loop on every outcome path, write the raw
  sidecar, keep stderr warnings, and leave the result line untouched.
- `crates/stc-test/src/coverage.rs` (new): `RawCoverage` parsing with protocol, count, bounds,
  and monotonic-ID validation; error type for missing/malformed data.
- Tests: valid, empty, malformed, out-of-range, and duplicate-ID raw files; host readout and
  bounds are covered by phase 4 end-to-end tests.

### 8.3 Phase 3 detail

- `crates/stc-driver/src/cli.rs`: `coverage: Option<bool>` and `coverage_out` in `TestOptions`;
  parse `--coverage`, `--no-coverage`, and `--coverage-out`; reject the conflicting pair,
  `--coverage` with `--list`, and `--coverage-out` with `--list` or without effective coverage;
  update usage text.
- `crates/stc-manifest`: add `[test] coverage = <bool>` (default false) to the test settings,
  validation, and schema tests.
- `crates/stc-driver/src/test.rs`: build the instrumented-file allow-list from the non-test
  source entries; set the codegen coverage option per test; store each test's coverage points and
  raw-sidecar path; pass the host arguments; read and aggregate after each run; write the
  reports; print the summary; emit warnings.
- `crates/stc-test/src/coverage.rs`: aggregation and the three renderers, with unit tests for
  union, disjoint tests, duplicate lines, zero sites, percentages, line projection, and
  order independence.
- `crates/stc-driver/src/clean.rs`: add the default coverage directory to the default and
  `--test` targets; test the removal and the custom-directory exception.
- `crates/stc-manifest/src/codes.rs`: register the two warnings. Add them to Conformance
  Profile §10 and record the warning-code decision in `specification/Decisions.md`.

### 8.4 Phase 4 detail

- Add `examples/coverage/` with production functions that have known covered and uncovered
  statements, two or more tests covering disjoint parts, a README with the expected report, and a
  committed `stc.lock`. Extend the example smoke test to check it.
- Add `crates/stc-driver/tests/coverage_cli.rs` with:
  - Golden `coverage.json`, `coverage.info`, and human summary for the example, with
    `STC_UPDATE_SNAPSHOTS=1` regeneration like the UI snapshots.
  - Aggregation across tests with per-test `covered` arrays.
  - Byte-identical reruns and no host-specific or temporal data.
  - Instrumented/uninstrumented equivalence for pass, fail, expected trap, and unexpected trap;
    compare statuses, failures, traps, cycles, and exit codes (durations excluded).
  - Production isolation: `stc build` debug and release contain no counter symbol and write no
    coverage files; `stc build --coverage` is an unknown option; `stc test` without `--coverage`
    produces no counter global or report.
  - Test-source, test-only-POU, and wrapper exclusion; dependency-package coverage in a separate
    test project; timeout and missing-data handling; `--no-run`; filters; `[build].opt-level = 3`.
  - Cross-host determinism: the same test target runs on the existing three-host CI matrix. No
    new xtask suite is required; `cargo test --workspace` already runs the target.
- Record an informational instrumented-vs-uninstrumented `stc test` wall-clock comparison for
  the example. It is reported as evidence and is not a gate.

## 9. Exit criteria

- `stc test --coverage` emits statement coverage for ST production code in the root and
  dependency packages, mapped to file, line, and column.
- Per-test and suite-level results are available in the native report.
- Instrumented tests produce the same statuses, failures, traps, cycles, and exit codes as
  uninstrumented tests.
- Production builds and release artifacts contain no counter global, site table, or coverage
  output.
- Golden reports, aggregation, determinism, dependency-package, exclusion, and cross-host tests
  pass on all three supported native hosts.
- Reports are byte-identical for identical inputs on every host and contain no time, duration,
  build-ID, absolute-path, or host data.
- The exclusion rules, report formats, limitations, and CLI surface are documented and tested.
- No host ABI, GOT/manifest, metadata, descriptor, production module, or test-result shape
  changes.

## 10. Risks and open questions

### 10.1 Risks

- **Optimizer behavior.** Test builds apply `[build].opt-level`, so instrumentation must survive
  optimization. The counter global has external linkage and export attributes, making stores
  observable; an `opt-level = 3` test proves counts match the O0 run. Optimizer-removed code
  reports zero, which is conservative.
- **Wall-clock overhead and timeouts.** Instrumented tests are slower; the per-test timeout is
  wall-clock. Document the caveat and keep the equivalence fixture well below the timeout.
- **Site-universe drift.** The universe is what codegen lowers. Future emission changes can add
  or remove sites; canonical span keys keep aggregation sound, and golden tests intentionally
  change when the universe changes.
- **Report size.** Per-test covered-site arrays grow with tests × sites. The example is small;
  measure before adding a verbosity toggle in a future version.
- **Exclusion mistakes.** A missed test-only definition or wrapper would inflate coverage. Tests
  cover every exclusion class; the wrapper is a virtual file and cannot be mapped to a path.
- **Export behavior.** `dso_local dllexport` data exports were probed on COFF and ELF. The phase
  2/4 end-to-end tests are the real proof; a missing symbol degrades to a coverage warning, not a
  test failure.
- **Cross-host paths and line endings.** Reports use package-relative forward-slash labels and
  span-based attribution; the golden tests run on all three hosts.
- **User confusion with WL-2.** Documentation must distinguish ST-program coverage from the
  compiler's Rust coverage job and the rule traceability check.

### 10.2 Open questions

None as of 2026-10-31. The plan is locked; new questions are appended only with a re-plan.

## 11. Documentation updates

| Document | Change |
| --- | --- |
| [Testing and Verification](specification/Architecture/Testing-and-Verification.md) | New coverage subsection under §2: model, sites, collection protocol, exclusions, report formats, determinism, limitations, and the non-correctness statement. Update the status block when WL-10 completes. |
| [CLI and Manifest](specification/Architecture/CLI-and-Manifest.md) | `stc test` options `--coverage`/`--no-coverage`/`--coverage-out`; `[test] coverage`; report file shapes in §1.6; default `target/stc/coverage`; `stc clean` behavior; exit codes unchanged. |
| [Conformance Profile](specification/01-Conformance-Profile.md) §10 | Register `STC-W-COVERAGE-001/002` with Testing and Verification ownership. |
| [Decisions](specification/Decisions.md) | Record C-10's warning-code decision with the next free `D-` number when phase 3 lands. |
| [USER-GUIDE.md](USER-GUIDE.md) | Add the `stc test --coverage` workflow and report examples. |
| [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) | Statement-only scope, timing/timeout caveat, unlinked program bodies, dependency labels, failed/timed-out test handling, and the non-correctness stance. |
| [Performance.md](specification/Verification/Performance.md) | Informational instrumented-vs-uninstrumented `stc test` timing from phase 4; non-gating. |
| [README.md](README.md) | Add the feature to the command/status surface when WL-10 completes. |
| [GOT Artifacts](specification/Architecture/GOT-Artifacts.md) | No change. The counter symbol is an internal test-build export, not a deployable module ABI symbol. |
| [V2.0.0-PLAN.md](V2.0.0-PLAN.md) | Link this sub-plan now; mark WL-10 done and consolidate the plan into the specifications at completion. |

## 12. Change log

| Date | Change |
| --- | --- |
| 2026-10-31 | Started the WL-10 sub-plan; recorded the current test, span, export, and verifier state; proposed the initial decision set (C-1–C-11); fixed the coverage model, report formats, exclusions, and phase plan; linked it from the parent plan. |
| 2026-10-31 | Locked C-1–C-4: codegen lowering, exported module counter array, all production packages, and unlinked production `PROGRAM` bodies excluded and documented. |
| 2026-10-31 | Locked C-6–C-8 and C-11: JSON+LCOV+human reports, statement-primary with a line projection, per-test covered-site indices, and `[test] coverage` with `--coverage`/`--no-coverage` overrides. Added C-12 for clean behavior. |
| 2026-10-31 | Locked C-5 and C-10: per-test raw sidecar transport and new coded coverage warnings; no fail-under threshold and no HTML in v2.0.0. |
| 2026-10-31 | Locked C-9 and C-12: byte-identical reports across hosts and `stc clean` removal of the default coverage directory; no CI upload and a full per-file human summary. |
| 2026-10-31 | Locked C-13 and C-14: no report filter and an informational, non-gating overhead measurement. All decisions C-1–C-14 are locked; the plan has no open questions. |
