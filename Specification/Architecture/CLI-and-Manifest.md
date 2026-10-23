# CLI and Project Manifest

Status: Draft  
Profile: `stc-2013-oop-utf8-v1`  
Related: [Architecture](Architecture.md), [Testing](Testing-and-Verification.md), [Project Plan](../00-Project-Plan.md)

## 1. CLI surface

The binary is `stc`. All commands accept `--help` and `--version`. Logs go to stderr; machine
output goes to stdout.

### 1.1 Commands

| Command | Purpose |
| --- | --- |
| `stc new <path>` | Scaffold a project (`stc.toml`, `src/main.st`, `.gitignore`) |
| `stc check` | Lex, parse, resolve, and type-check; no codegen |
| `stc build` | Emit artifacts selected by `--emit` / manifest |
| `stc run` | Build an executable and run it |
| `stc test` | Build and run the built-in test suite |
| `stc clean` | Remove the project's `target/stc` directory |
| `stc metadata` | Print the selected configuration, ABI version, exports, and artifact layout |
| `stc compliance` | Print the profile ID and supported feature matrix |

Future commands (not v1): `stc fmt`, `stc lsp`, `stc doc`.

### 1.2 Common options

| Option | Meaning |
| --- | --- |
| `--manifest-path <path>` | Use a specific `stc.toml` |
| `--target <triple>` | Pass a target triple to clang; emit it in the IR |
| `--release` | `opt-level = 3`, debug line info off unless overridden |
| `--emit <list>` | Comma-separated `ir,obj,exe,lib,metadata`; overrides manifest |
| `--clang <path>` | clang executable |
| `--runtime <path>` | Runtime static library |
| `--out-dir <path>` | Artifact directory; default `target/stc/<profile>` |
| `--message-format <fmt>` | `human` (default), `json`, `short` |
| `--color <when>` | `auto`, `always`, `never` |
| `-v`, `-vv` | Verbose phase output; `-vv` includes clang commands and IR paths |
| `-q`, `--quiet` | Suppress non-error output |
| `-D <lint>` / `-W <lint>` / `-A <lint>` | Deny/warn/allow a diagnostic group |
| `--deny warnings` | Equivalent to `-D warnings` |
| `--locked` | Require `stc.lock` to match the manifest/dependency graph; fail instead of updating it |

Precedence: CLI > environment (`STC_TARGET`, `STC_CLANG`, `STC_RUNTIME`, `STC_LOG`) >
`stc.toml` > built-in defaults.

### 1.3 Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Compile error or failing test |
| 2 | Usage error, manifest error, or missing toolchain |
| 3 | Internal compiler error (panic caught at the driver boundary) |
| 4 | Trap or runtime failure in `stc run`/`stc test` |
| 130 | Interrupted (Ctrl-C) |

### 1.4 Diagnostics

Human format:

```
src/main.st:14:9: error[STC-E-TYPE-012]: cannot assign DINT to BOOL
   |
14 |     flag := counter;
   |             ^^^^^^^ expected BOOL, found DINT
   |
   = note: implicit BOOL conversion is not defined for DINT
   = help: use (counter <> 0) to convert explicitly
```

JSON format: one object per diagnostic on stdout, one per line:

```json
{
  "code": "STC-E-TYPE-012",
  "severity": "error",
  "message": "cannot assign DINT to BOOL",
  "file": "src/main.st",
  "span": { "start": 142, "end": 149, "line": 14, "column": 9 },
  "labels": [{ "span": { "start": 142, "end": 149 }, "message": "expected BOOL, found DINT" }],
  "notes": ["implicit BOOL conversion is not defined for DINT"],
  "suggestions": [{ "message": "use (counter <> 0)", "span": { "start": 142, "end": 149 }, "replacement": "(counter <> 0)" }]
}
```

- Error codes are stable; see the conformance profile and each spec.
- Compiler panics are caught and reported as `STC-I-0001` with a bug-report note and the IR/source
  context. They never print a Rust backtrace by default (`RUST_BACKTRACE=1` for developers).

## 2. Project manifest: `stc.toml`

### 2.1 Example

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
stdlib = true
configuration = "Main"            # optional if exactly one CONFIGURATION exists

[build]
target = "x86_64-pc-windows-msvc" # optional; default = clang host
opt-level = 2
debug = true
emit = ["exe", "ir"]              # default
lib-type = "static"               # static | shared, used when emit includes "lib"
out-dir = "target/stc"
clang = "C:\\Program Files\\LLVM\\bin\\clang.exe"  # optional
runtime = "target/debug/stc_runtime.lib"           # optional

[run]
cycles = 0                        # 0 = until interrupted
cycle-time = "10ms"
io = "null"                       # null | memory | file
io-file = "io.bin"

[test]
sources = ["tests/**/*.st"]
timeout = "5s"
default-cycles = 1
mock-clock = "0ns"
io = "memory"

[[test.case]]
name = "TimerBlink"
cycles = 25
timeout = "2s"
```

### 2.2 Schema rules

- `schema` is required and currently `1`. Unknown top-level keys are errors.
- `[package] name` is required, ASCII, matches `[A-Za-z_][A-Za-z0-9_-]*`, and is used for artifact
  names.
- `[package] version` is required and semver-ish (not used for dependency resolution in v1).
- `[package] language` is required and must equal the supported profile ID. A future edition adds
  new profile IDs; it does not silently change semantics.
- `[dependencies]` entries are local path dependencies in v1:
  `name = { path = "../dir" }`. The key is the dependency alias; the dependency's
  `[package] name` must match it unless an explicit alias is present. Paths are canonicalized;
  packages are deduplicated by canonical path; cycles are errors. The graph is compiled in
  topological order. A `version` key is reserved for a future registry (beyond v1) and is ignored
  for path dependencies. The resolved graph is written to `stc.lock`; no network access or
  registry in v1.
- `[project] sources` is a list of globs relative to the manifest. All matching `.st` files are
  compiled as one package; file order is not semantic. Duplicate paths and paths outside the
  project root are errors unless explicitly allowed. Dependencies are separate packages with
  their own `[project] sources` and manifests.
- `[project] stdlib` defaults to `true`. When false, standard FBs are not injected (standard
  functions remain, because they are part of the language prelude). This is for conformance tests.
- `[project] configuration` selects one `CONFIGURATION` declaration. If omitted and exactly one
  exists, it is selected. If omitted and multiple exist, the build fails with a list.
- `[build] target` is passed to clang and emitted as the IR triple.
- `[build] opt-level` is `0|1|2|3|"s"`; maps to clang `-O` flags.
- `[build] debug` controls line tables / `-g`.
- `[build] emit` is a non-empty subset of `ir,obj,exe,lib,metadata`. `exe` and `lib` may coexist.
- `[build] lib-type` selects static or shared library output.
- `[build] out-dir` is relative to the manifest and must stay inside the project unless the user
  passes `--out-dir`.
- `[run] cycles` bounds executable runs; `stc run` overrides it with `--cycles`.
- `[run] cycle-time` is a duration literal string (for example `"10ms"`). It is not ST syntax.
- `[run] io` selects the default I/O backend.
- `[test]` keys define defaults; `[[test.case]]` entries override per POU name.
- Unknown keys in known tables are errors with a "did you mean" suggestion. Unknown
  `[[test.case]]` keys are errors.
- Environment variable substitution is not supported in v1 (reproducibility).
- `[dependencies]` is part of v1 for local path dependencies. A dependency entry without `path`
  is an error until/unless a registry resolver is added (beyond v1, A-009); unknown dependency
  keys are errors.

### 2.3 Source discovery

- Globs use `*`, `**`, `?`, and character classes; matching is case-insensitive on Windows for
  path comparison but the file name recorded in diagnostics is the on-disk spelling.
- Hidden directories and `target/` are excluded by default.
- Symlinked directories are not followed by default; a symlinked file is accepted if its real
  path is inside the project.
- If a glob matches nothing, the build fails with a manifest error.
- The reference PDF and non-`.st` files are ignored.

### 2.4 Test manifest

- `[test] sources` adds test-only source files to the current package. They are compiled together
  with the package sources and its dependencies.
- `[[test.case]] name` must match a POU carrying `{stc:test}`; duplicate names are errors.
- A **separate test project** is a normal package with `{stc:test}` POUs and a dependency on the
  code under test:
  ```toml
  [package]
  name = "motor-tests"
  version = "0.1.0"
  language = "stc-2013-oop-utf8-v1"

  [dependencies]
  motor-control = { path = "../motor-control" }

  [project]
  sources = ["tests/**/*.st"]

  [test]
  timeout = "5s"
  ```
  `stc test` in that package compiles `motor-control` and the test sources but runs only the test
  POUs in `motor-tests`.
- `cycles` overrides the test's default cycle count.
- `timeout` is a duration string; the runner kills the test process on expiry.
- `mock-clock` sets the initial mock time; tests can advance it via the test harness API.
- `expect-trap` (future) records the expected trap code; a test that traps with that code passes.

## 3. Lockfile: `stc.lock`

Every build/check writes `stc.lock` at the root package directory unless `--locked` is given.
The lockfile pins the resolved package graph for reproducible builds. It is committed.

```toml
schema = 1

[[package]]
name = "motor-control"
version = "0.1.0"
source = "root"
dependencies = ["motor-lib"]

[[package]]
name = "motor-lib"
version = "0.2.0"
source = { path = "../motor-lib" }
dependencies = []
```

Rules:

- `schema` is required and currently `1`.
- Packages are sorted by `(name, source)`; dependency lists are sorted.
- The root package uses `source = "root"`. Dependency paths are normalized relative to the
  root package directory, use forward slashes, and may contain `..`. Absolute paths are never
  written.
- `name` and `version` come from each dependency's `[package]` table. A dependency alias in the
  parent manifest is recorded as the edge name; the package's own name is the `name` field.
- The lockfile is a resolved graph, not a source of version requirements. A dependency without a
  `path` is an error in v1.
- `--locked` fails if the lockfile is missing or would change (new/removed package, changed
  version/path/edge). CI uses `--locked`.
- `stc clean` does not delete `stc.lock`. Deleting the lockfile is an explicit user action.
- Local source contents are not hashed in v1; editing a path dependency's sources does not
  rewrite the lockfile. A content-hash field can be added later without changing the schema's
  package identity.

## 4. Artifact and metadata layout

```
target/stc/<profile>/
  <name>.ll
  <name>.o
  <name>.exe
  <name>.lib | <name>.a | <name>.dll | <name>.so
  <name>.stc-meta.json
  build.log
```

`<name>.stc-meta.json`:

```json
{
  "profile": "stc-2013-oop-utf8-v1",
  "compiler": "0.1.0",
  "abi": 1,
  "target": "x86_64-pc-windows-msvc",
  "configuration": "Main",
  "packages": [
    { "name": "motor-control", "version": "0.1.0", "path": "." },
    { "name": "motor-lib", "version": "0.2.0", "path": "../motor-lib" }
  ],
  "artifacts": ["exe", "ir"],
  "exports": [
    { "name": "stc_init", "kind": "entry" },
    { "name": "stc_cycle", "kind": "entry" },
    { "name": "stc_export_Main", "kind": "pou", "source": "Main" }
  ],
  "io_image": { "inputs": 16, "outputs": 16, "markers": 64 },
  "strings": [{ "type": "USTRING", "capacity": 255, "unit_bytes": 4, "length_unit": "character" }],
  "spans": { "files": ["src/main.st"], "entries": [[0, 0, 12]] },
  "retained": []
}
```

- The metadata is consumed by the runtime and by embedding hosts.
- It contains no timestamps or absolute paths, so builds are reproducible.
- The span table is optional in library mode if the host does not need diagnostics.

## 5. `stc new` scaffold

```
<name>/
  stc.toml
  src/main.st
  tests/smoke.st
  .gitignore
```

`src/main.st`:

```st
PROGRAM Main
VAR
  counter : DINT;
END_VAR
  counter := counter + 1;
END_PROGRAM
```

`tests/smoke.st`:

```st
{stc:test}
FUNCTION SmokeTest : BOOL
  SmokeTest := TRUE;
END_FUNCTION
```

## 6. Verbose output

`stc build -v` prints phases and timings to stderr:

```
[stc] profile stc-2013-oop-utf8-v1
[stc] manifest stc.toml
[stc] sources 3 files, 812 lines
[stc] lex 3.1ms  parse 5.4ms  resolve 2.2ms  typeck 4.8ms  ir 1.9ms
[stc] clang 12.1ms
[stc] artifacts target/stc/stc-2013-oop-utf8-v1/motor-control.exe
```

`-vv` adds per-file phases, the exact clang command line, and retained IR paths. The log never
contains secrets; paths are printed as written by the user.

## 7. Open CLI/manifest questions

- Q-010: `--strict` mode for strict 2013 semicolons and no syntax supersets.
- Q-011: Formatter command and its style configuration.
- Q-013: Whether `stc test` should support parallel test processes by default.
- Q-014: Whether the manifest should support `[lints]` tables or only CLI lint flags.
