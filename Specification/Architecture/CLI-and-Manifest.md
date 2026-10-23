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
| `stc build` | Emit artifacts selected by `--emit` / manifest; excludes test-only units |
| `stc run` | Build an executable and run it |
| `stc test` | Build and run the built-in test suite; includes test-only units |
| `stc fmt [paths...]` | Format sources with the fixed style; `--check` for CI |
| `stc clean` | Remove the project's `target/stc` directory |
| `stc metadata` | Print the selected configuration, ABI version, exports, and artifact layout |
| `stc compliance` | Print the profile ID and supported feature matrix |

Future commands (not v1): `stc lsp`, `stc doc`.

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
| `--io <backend>` | Override `[io].backend` for `run`/`test` (`null`, `memory`, `file`) |
| `--io-file <path>` | File used by the file backend |
| `--relaxed` | Accept the documented semicolon-relaxation syntax superset |
| `--state-file <path>` | Retained-state file for this run |
| `--reset-retain` | Cold start: ignore existing state, overwrite on shutdown |
| `--no-retain` | Disable retained-state load and save |
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

```text
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

### 1.5 Formatter (`stc fmt`)

`stc fmt [paths...]` formats the project sources (or the given files) in place. `--check` writes
nothing and exits 1 if any file would change.

The v1 style is fixed:

- keywords uppercase; identifiers keep their original spelling;
- 2-space indentation; one statement per line;
- one space around binary operators, after commas, and around `:=`/`=>`;
- no space before a call's `(`; `name : TYPE` in declarations;
- `END_*` on its own line; strict semicolons in the output even when the input used `--relaxed`;
- comments preserved; existing line breaks preserved; no automatic line wrapping;
- idempotent: formatting an already-formatted file produces no change.

No configuration, range formatting, or lint integration in v1. The formatter uses the lossless
CST and never rewrites tokens it does not understand.

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
syntax = "strict"                 # strict (default) | relaxed
emit = ["exe", "ir"]              # default
lib-type = "static"               # static | shared, used when emit includes "lib"
out-dir = "target/stc"
clang = "C:\\Program Files\\LLVM\\bin\\clang.exe"  # optional
runtime = "target/debug/stc_runtime.lib"           # optional

[io]
backend = "null"                  # v1: null | memory | file
file = "io.bin"                   # used when backend = "file"
# input-bytes / output-bytes / marker-bytes are optional; the compiler computes them

[[io.devices]]
name = "drive1"
kind = "ethercat"                 # opaque identifier; runtime-defined
config = { vendor = "0x1234", revision = 3 }  # opaque to the compiler

  [[io.devices.channels]]
  name = "DI5"
  direction = "input"
  width = 1

[[io.bindings]]
variable = "Main.MyBool"
device = "drive1"
channel = 5
direction = "input"

[retain]
enabled = true
file = "motor.stcstate"     # relative to the project root
on-mismatch = "error"       # error | reset

[lints]
warnings = "warn"           # warn | deny | allow (baseline for warning diagnostics)
"STC-W-NAME-002" = "allow"  # per-code override
"STC-W-LEX-008" = "deny"

[run]
cycles = 0                        # 0 = until interrupted
cycle-time = "10ms"

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
- `[build] syntax` is `strict` (default) or `relaxed`. Strict requires the 2013 statement
  terminator rules; relaxed enables the documented omitted-semicolon-after-block superset.
  `--relaxed` overrides the manifest.
- `[build] emit` is a non-empty subset of `ir,obj,exe,lib,metadata`. `exe` and `lib` may coexist.
- `[build] lib-type` selects static or shared library output.
- `[build] out-dir` is relative to the manifest and must stay inside the project unless the user
  passes `--out-dir`.
- `[io]` is optional. `backend` is `null`, `memory`, or `file` in v1; hardware backend names are
  runtime-defined in v2. `file` names the file used by the file backend.
  `input-bytes`/`output-bytes`/`marker-bytes` are optional; the compiler computes them from `AT`
  declarations when omitted. `[test] io` overrides the backend for test runs.
- `[[io.devices]]` declares a device tree. `name` is unique; `kind` is an opaque string;
  `config` is opaque TOML copied to metadata. `[[io.devices.channels]]` is optional and gives the
  compiler enough information to validate hints (`name`, `direction`, `width`).
- `[[io.bindings]]` is an optional project-level binding: `variable` is a fully qualified ST
  variable, `device` must name a declared device, and `channel`/`direction` are optional hints.
  The compiler resolves `variable` and emits the binding in metadata. Binding precedence is
  runtime override > `[[io.bindings]]` > `{stc:io}` pragma > raw `%` address.
- I/O binding to hardware is runtime-owned; the compiler only emits process-image and binding
  metadata, including a default-value image for inputs. Unbound inputs receive type defaults each
  cycle; unbound outputs are discarded. Details: [I/O and Devices](IO-and-Devices.md).
- `[retain]` controls file-backed retained state:
  `enabled` (default true), `file` (default `<artifact>.stcstate` in `out-dir`), and
  `on-mismatch` (`error` default, or `reset`). The file path is relative to the project root.
  `--state-file`, `--reset-retain`, and `--no-retain` override it. Details:
  [State and RETAIN](State-and-Retain.md).
- `[lints]` sets project lint levels. `warnings` is the baseline (`warn` default, `deny`, or
  `allow`). Per-code keys are `"STC-W-*"`/`"STC-E-*"` with values `allow`, `warn`, or `deny`.
  Errors cannot be downgraded below `error`; a manifest that tries is invalid. Unknown codes are
  errors. CLI `-D`/`-W`/`-A` and `--deny warnings` override the manifest.
- `[run] cycles` bounds executable runs; `stc run` overrides it with `--cycles`.
- `[run] cycle-time` is a duration literal string (for example `"10ms"`). It is not ST syntax.
- `[test] io` overrides the `[io] backend` for test runs.
- `[test]` keys define defaults; `[[test.case]]` entries override per POU name.
- Unknown keys in known tables are errors with a "did you mean" suggestion. Unknown
  `[[test.case]]` keys are errors.
- Environment variable substitution is not supported in v1 (reproducibility).
- `[dependencies]` is part of v1 for local path dependencies. A dependency entry without `path`
  is an error until/unless a registry resolver is added (beyond v1, A-009); unknown dependency
  keys are errors.

### 2.3 Source discovery

- Globs use `*`, `**`, `?`, and character classes; matching is case-insensitive on Windows and
  case-sensitive on Linux for path comparison, but the file name recorded in diagnostics is the
  on-disk spelling. Manifest and lockfile paths use forward slashes on every host (D-042).
- Hidden directories and `target/` are excluded by default.
- Symlinked directories are not followed by default; a symlinked file is accepted if its real
  path is inside the project.
- If a glob matches nothing, the build fails with a manifest error.
- The reference PDF and non-`.st` files are ignored.

### 2.4 Test manifest

- `[test] sources` adds test-only source files to the current package. They are compiled only for
  `stc test`, together with the package sources and its dependencies. Production builds do not
  load these files and exclude `{stc:test}` POUs from semantic analysis/codegen; release artifacts
  contain no test code or `STC_ASSERT` calls. A production POU using `STC_ASSERT` or a test-only
  POU is an error.
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
- `stc clean` does not delete `stc.lock` or retained-state files. Deleting them is an explicit
  user action.
- Local source contents are not hashed in v1; editing a path dependency's sources does not
  rewrite the lockfile. A content-hash field can be added later without changing the schema's
  package identity.

## 4. Artifact and metadata layout

```text
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
  "io": {
    "backend": "null",
    "image": { "input_bytes": 16, "output_bytes": 16, "marker_bytes": 64 },
    "input_defaults_hex": "0000...",
    "devices": [
      { "name": "drive1", "kind": "ethercat", "config": { "revision": 3, "vendor": "0x1234" } }
    ],
    "bindings": [
      { "variable": "Main.MyBool", "area": "I", "address": "IX5", "offset_bits": 5,
        "type": "BOOL", "hint": { "device": "drive1", "channel": 5 }, "source": "pragma" }
    ]
  },
  "retain": {
    "size": 128,
    "layout_hash": "0x9f2c...",
    "file": "motor.stcstate",
    "on_mismatch": "error",
    "leaves": [
      { "path": "Main.Counter", "type": "DINT", "offset": 0, "size": 4 }
    ]
  },
  "strings": [{ "type": "USTRING", "capacity": 255, "unit_bytes": 4, "length_unit": "character" }],
  "spans": { "files": ["src/main.st"], "entries": [[0, 0, 12]] },
  "retained": []
}
```

- The metadata is consumed by the runtime and by embedding hosts.
- It contains no timestamps or absolute paths, so builds are reproducible.
- The span table is optional in library mode if the host does not need diagnostics.

## 5. `stc new` scaffold

```text
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

```text
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

None currently. New CLI/manifest questions are appended here or in [Decisions](../Decisions.md).
