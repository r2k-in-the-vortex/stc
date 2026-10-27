# CLI and Project Manifest

Status: M4.5 module/build CLI implemented (2026-10-25); M6 project manifest, diagnostics, report shapes, and all M6 commands implemented (2026-10-27); M6.5 debug/release options pending  
Profile: `stc-2013-oop-utf8-v1`  
Related: [Architecture](Architecture.md), [Online Change and GOT](Online-Change-and-GOT.md), [Debug and Monitoring](Debug-and-Monitoring.md), [Testing](Testing-and-Verification.md), [Project Plan](../00-Project-Plan.md)

## 1. CLI surface

The binary is `stc`. All commands accept `--help` and `--version`. Logs go to stderr; machine
output goes to stdout.

### 1.1 Commands

> **M6 status (2026-10-27):** all M6 commands are implemented on `stc.toml` projects. `new`,
> `check`, `build`, `run`, `test`, `fmt`, `clean`, `metadata`, and `compliance` provide the
> project workflow; `rename`, `online download`, `online diff`, and `run --watch` provide the
> online-change workflow. The manifest-free `stc parse <file>`, `stc check <file>...`, and
> `stc build <file>...` bridge is unchanged. `stc run` builds `exe,module,metadata`, applies
> `[run]` defaults and the `--cycles`/`--cycle-time`/`--io`/`--io-file`/`--state-file`/
> `--reset-retain`/`--no-retain` overrides, starts the launcher, and returns its exit code.
> `stc test` builds one cold test artifact per test under `target/stc/test/<artifact>` and runs
> each in its own process; `stc fmt` formats the root package or explicit files. `stc rename`
> resolves a dotted key through sema, rewrites the declaration and references, and records the
> mapping in `stc.renames.toml`; builds merge that history with `--rename` (chains compose
> oldest-first, conflicts are `STC-E-PROJECT-013`). `online download` reads `--store`,
> `[runtime].manifest-dir`, or a live `--host` endpoint and writes the canonical envelope;
> `online diff` compares a JSON/binary candidate with `--baseline`, the store, or a live host and
> exits 0/1/2. `run --watch` builds against the active baseline, launches the host with a local
> user-only control endpoint, watches sources/manifests/history, deploys module/manifest pairs,
> reports each compatibility result, retries once on a stale baseline, keeps the old generation
> on failure, and exits 130 on Ctrl-C. `--store` also selects the watch store, and
> `--watch-timeout <duration>` bounds a run for automation. `--message-format` supports
> `human`, `json`, and `short`; exit codes follow §1.3. The loadable `module` with its
> binary/JSON manifest and metadata, the thin `exe` launcher, and the embedding `lib` host
> archive keep their M4.5 shapes, and `lib-type = "shared"` selects the shared host library.
> `--debug`/`--release` and their debug-info behavior are M6.5; M6 honors `[build].opt-level` but
> emits no DWARF/CodeView or checkpoint hooks.

| Command | Purpose |
| --- | --- |
| `stc new <path>` | Scaffold a project (`stc.toml`, `src/main.st`, `.gitignore`) |
| `stc check` | Lex, parse, resolve, and type-check; no codegen |
| `stc build` | Emit artifacts selected by `--emit` / manifest; excludes test-only units |
| `stc run` | Build an executable and run it |
| `stc test` | Build and run the built-in test suite; includes test-only units |
| `stc fmt [paths...]` | Format sources with the fixed style; `--check` for CI |
| `stc clean` | Remove the configured artifact directory (`target/stc/debug` by default); never `stc.lock` or state files |
| `stc metadata` | Print the selected configuration, ABI version, exports, and artifact layout |
| `stc compliance` | Print the profile ID and supported feature matrix |
| `stc online download` | Export the active manifest, layout summary, and controller/deployment identifier |
| `stc online diff` | Compare a candidate manifest with the active baseline and print compatibility |
| `stc rename` | Source-aware tracked rename that preserves `GotId` history |
| `stc run --watch` | Watch sources, rebuild, and deploy an online change |
| `stc debug` | Start a local authenticated debug/monitoring session (M6.5) |

Future commands (not v1): `stc lsp`, `stc doc`.

### 1.2 Common options

| Option | Meaning |
| --- | --- |
| `--manifest-path <path>` | Use a specific `stc.toml` |
| `--target <triple>` | Pass a target triple to clang; emit it in the IR |
| `--release` | Optimised, stripped build; omits debug hooks and full debug info; distinct build ID/manifest. M6 rejects it as a usage error; M6.5 implements it. Debug is the default mode |
| `--debug` | Force debug information and checkpoint hooks (default unless `--release`). M6 accepts it but emits no debug info or hooks; `[build].opt-level` still applies (D-158) |
| `--emit <list>` | Comma-separated `ir,obj,module,exe,lib,metadata`; `module` emits the loadable module and manifest pair; `exe` is a launcher and `lib` the embedding host library; the two may coexist; overrides manifest |
| `--got-manifest <path>` | Build against the given persistent GOT manifest as the identity baseline |
| `--cold` | Explicitly build without an online-change baseline (cold-layout build) |
| `--rename <old=new>` | Record a tracked rename/move; repeatable, including field/method/type/POU paths. Renaming a package name is out of scope (D-156) |
| `--clang <path>` | clang executable |
| `--runtime <path>` | Runtime host archive used when linking `exe`/`lib` |
| `--out-dir <path>` | Artifact directory; default `target/stc/debug` for both the manifest-free bridge and project builds; `[build].out-dir` overrides the project default |
| `--io <backend>` | Override `[io].backend` for `run`/`test` (`null`, `memory`, `file`) |
| `--io-file <path>` | File used by the file backend |
| `--relaxed` | Accept the documented semicolon-relaxation syntax superset |
| `--state-file <path>` | Retained-state file for this run |
| `--reset-retain` | Cold start: ignore existing state, overwrite on shutdown |
| `--no-retain` | Disable retained-state load and save |
| `--debug-protocol <kind>` | Local debug transport (`pipe` default, `socket` optional) |
| `--allow-untracked-removal` | Permit removing/renaming a root without a tracked rename; otherwise diagnose and require `--cold` |
| `--message-format <fmt>` | `human` (default), `json`, `short` |
| `--color <when>` | `auto`, `always`, `never` |
| `-v`, `-vv` | Verbose phase output; `-vv` includes clang commands and IR paths |
| `-q`, `--quiet` | Suppress non-error output |
| `-D <lint>` / `-W <lint>` / `-A <lint>` | Deny/warn/allow a diagnostic group |
| `--deny warnings` | Equivalent to `-D warnings` |
| `--locked` | Require `stc.lock` to match the manifest/dependency graph; fail instead of updating it |
| `--list` | `stc test`: list discovered tests without building |
| `--no-run` | `stc test`: build test artifacts without executing them |
| `--timeout <duration>` | `stc test`: override the per-test timeout (default 5 s) |
| `-- --nocapture` | `stc test`: forward test-host output instead of only reporting results |

Precedence: CLI > environment (`STC_TARGET`, `STC_CLANG`, `STC_RUNTIME`, `STC_LOG`) >
`stc.toml` > built-in defaults.

The table describes the v1 target surface. M6 implemented `--manifest-path`, `--locked`,
`--color`, `-q`, `-D`/`-W`/`-A`, `--deny warnings`, `-v`/`-vv`, `--debug`, and, for `stc run`,
`--cycles`, `--cycle-time`, `--io`, `--io-file`, `--state-file`, `--reset-retain`, and
`--no-retain`. `run --watch` adds `--store <dir>` and `--watch-timeout <duration>`; the online
commands add `--out`, `--binary`, `--store`, `--host`, `--nonce`, and `--baseline`, and `rename`
adds `--dry-run`. `--release`, debug information, and `--debug-protocol` remain M6.5 work.

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

- Error codes are stable; see the conformance profile and each spec. The complete code index,
  including the M6 `PROJECT`, `LOCK`, `ASSERT`, `TEST`, `FMT`, `RUN`, and `ONLINE` additions, is
  [Conformance Profile](../01-Conformance-Profile.md) §10.
- Project diagnostics use `STC-E-PROJECT-*` / `STC-W-PROJECT-*`; lockfile diagnostics use
  `STC-E-LOCK-*`; test and assertion diagnostics use `STC-E-TEST-*` / `STC-E-ASSERT-*`;
  formatter diagnostics use `STC-E-FMT-*`.
- Online-change diagnostics use the `STC-E-MODULE-*`, `STC-E-MANIFEST-*`, `STC-E-ABI-*`, and
  `STC-E-ONLINE-*` / `STC-W-ONLINE-*` families; the M6 control-channel and deploy additions are
  `STC-E-ONLINE-007/008/009` and the compatibility-report warning is `STC-W-ONLINE-007`
  (`STC-W-ONLINE-002` remains the initializer warning). Debug diagnostics use `STC-E-DEBUG-*`
  and `STC-W-DEBUG-*`.
- Compiler panics are caught and reported as `STC-I-0001` with a bug-report note and the IR/source
  context. They never print a Rust backtrace by default (`RUST_BACKTRACE=1` for developers).

### 1.5 Formatter (`stc fmt`)

`stc fmt [paths...]` formats the root package's production and test sources (or the given files)
in place. `--check` writes nothing and exits 1 if any file would change.

The v1 style is fixed:

- keywords uppercase; identifiers keep their original spelling;
- 2-space indentation; one statement per line;
- one space around binary operators, after commas, and around `:=`/`=>`;
- no space before a call's `(`; `name : TYPE` in declarations;
- `END_*` on its own line; strict semicolons in the output even when the input used `--relaxed`;
- comments preserved; existing line breaks preserved; no automatic line wrapping;
- idempotent: formatting an already-formatted file produces no change.

No configuration, range formatting, or lint integration in v1. The formatter uses the lossless
CST and never rewrites tokens it does not understand. Syntax errors prevent formatting
(`STC-E-FMT-001`); a file that cannot be read or written is `STC-E-FMT-002`.

### 1.6 JSON report shapes

These shapes are stable for v1. Required keys are listed; a compatible addition may append a
key, but a required key's name, type, or meaning does not change. JSON goes to stdout; human
logging stays on stderr.

**Diagnostics.** One object per diagnostic per line, as in §1.4. This shape is unchanged.

**Test results.** `stc test --message-format json` writes one object per test, in discovery
order, after the test process exits:

```json
{
  "name": "TimerBlink",
  "status": "fail",
  "duration_ms": 25,
  "failures": [
    { "message": "timer should complete", "file": "tests/timer.st", "line": 6, "column": 3 }
  ],
  "trap": null
}
```

- `status` is `pass`, `fail`, `trap`, or `timeout`.
- `duration_ms` is the wall-clock time spent in the test process.
- `failures` contains one entry per recorded `STC_ASSERT` failure; `file`, `line`, and `column`
  are omitted only when the call site has no source location.
- `trap` is the trap name (for example `STC_TRAP_DIV_ZERO`) for `status = "trap"`, otherwise
  `null`. An expected trap that matches `expect-trap` produces `pass`.

**Compatibility report.** `stc build` and `stc online` write one object when a baseline is in
use (human output is the default; `--message-format json` selects this shape):

```json
{
  "verdict": "compatible",
  "baseline_build_id": "0x…",
  "candidate_build_id": "0x…",
  "counts": { "preserved": 12, "allocated": 2, "dropped": 1, "retired": 1, "rejected": 0 },
  "warnings": [ { "code": "STC-W-ONLINE-004", "message": "…" } ],
  "rejections": []
}
```

- `verdict` is `compatible`, `incompatible`, or `cold`; `baseline_build_id` is `null` for a cold
  build with no baseline.
- `counts` always contains all five counters, including zeros.
- `warnings` and `rejections` contain stable `code`/`message` objects; rejection codes are
  `STC-E-ONLINE-*`/`STC-E-MODULE-*`/`STC-E-MANIFEST-*`/`STC-E-ABI-*`.
- `-v` additionally emits an `actions` array of `{ "got_id", "kind", "key" }` objects; without
  `-v` the key is absent.
- `stc online diff` exits 0 for `compatible`, 1 for `incompatible`, and 2 for usage/parse errors;
  the object is printed in both human and JSON modes.

**Metadata summary.** `stc metadata --message-format json` writes one object:

```json
{
  "profile": "stc-2013-oop-utf8-v1",
  "abi_version": 2,
  "artifact": "motor-control",
  "build_id": "0x…",
  "configuration": "Main",
  "target": "x86_64-pc-windows-msvc",
  "tasks": [ { "name": "MainTask", "resource": "Cpu", "interval_ns": 1000000, "priority": 1, "single": false } ],
  "exports": [ { "symbol": "stc_export_Main", "function": "stc_fn_…" } ],
  "retain": { "enabled": true, "file": "motor.stcstate", "on_mismatch": "error" },
  "io": { "backend": "null", "bindings": 1, "devices": 1, "var_access": 1 },
  "artifacts": [ { "kind": "module", "file": "motor-control-<build-id>.dll" } ]
}
```

The object is a summary; the sidecar in §4 remains the complete metadata. `on_mismatch` is a JSON
field name; the manifest key is `on-mismatch`.

**Compliance summary.** `stc compliance --message-format json` writes one object:

```json
{
  "profile": "stc-2013-oop-utf8-v1",
  "edition": "IEC 61131-3:2013",
  "ed4_claimed": false,
  "extensions": ["STC-UTF8", "STC-PRAGMA", "STC-TEST", "STC-POINTER"],
  "runtime_capabilities": ["online-change", "got-manifest", "debug-monitoring"],
  "features": {
    "lexical": "implemented",
    "oop": "implemented",
    "project-system": "implemented",
    "test-runner": "implemented",
    "formatter": "implemented",
    "online-change": "implemented",
    "debug-monitoring": "planned"
  },
  "safety": "not-a-safety-plc"
}
```

`features` maps the conformance profile's feature categories to `implemented`, `partial`,
`planned`, or `not-in-profile`; it is not a claim beyond
[Conformance Profile](../01-Conformance-Profile.md). `safety` is a fixed string, and the human
output includes the non-safety statement verbatim.

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
debug = true                      # parsed; M6 always builds the debug form without debug info (D-158)
syntax = "strict"                 # strict (default) | relaxed
emit = ["module", "ir"]           # v1 online-change deployment; exe/ir/lib also available
lib-type = "static"               # static | shared, used when emit includes "lib"
out-dir = "target/stc"
clang = "C:\\Program Files\\LLVM\\bin\\clang.exe"  # optional
runtime = "target/debug/stc_runtime.lib"           # optional

[io]
backend = "null"                  # v1: null | memory | file
file = "io.bin"                   # used when backend = "file"
input-bytes = 16                  # optional; the compiler computes image sizes from AT declarations
output-bytes = 16
marker-bytes = 64

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

[runtime]
module-dir = "runtime/modules"
manifest-dir = "runtime/manifests"
max-retired-generations = 1

[online]
baseline = "active"                 # active | explicit
# manifest = "runtime/manifests/motor.stc-got.json"  # required when baseline = "explicit"
cold-policy = "require-confirmation"  # v1 value; confirmation is required before a cold download
allow-untracked-removal = false

[debug]
default = true
protocol = "pipe"                   # pipe | socket (local only)
force-sessions-end-on-swap = true

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
expect-trap = "STC_TRAP_DIV_ZERO"  # optional; a matching trap makes the test pass
```

### 2.2 Schema rules

- `schema` is required and currently `1`. Unknown top-level keys are errors.
- Every schema-defined key in `stc.toml` is kebab-case as shown in §2.1. Snake_case spellings
  (`opt_level`, `out_dir`, `on_mismatch`, `module_dir`, `cold_policy`, `cycle_time`, `mock_clock`,
  `expect_trap`, and so on) are unknown keys, not aliases; they produce `STC-E-PROJECT-002` with
  a did-you-mean suggestion. Dependency alias keys are user package names and `[lints]` keys are
  quoted `STC-*` codes, so those table keys are not schema keys. JSON field names in metadata are
  separate and may use snake_case (for example `on_mismatch`).
- `[package] name` is required, ASCII, matches `[A-Za-z_][A-Za-z0-9_-]*`, and is used for artifact
  names.
- `[package] version` is required and semver-ish (not used for dependency resolution in v1).
- `[package] language` is required and must equal the supported profile ID. A future edition adds
  new profile IDs; it does not silently change semantics.
- `[dependencies]` entries are local path dependencies in v1:
  `name = { path = "../dir" }`. The key is the dependency alias; the dependency's
  `[package] name` must match it unless an explicit alias is present. An explicit alias is
  `alias = { path = "../dir", package = "real-name" }`: `alias` is the root namespace and
  `package` is the dependency's `[package].name`, which remains the package identity for
  catalog keys, `GotId`s, and symbols (D-153). Paths are canonicalized; packages are
  deduplicated by canonical path; cycles are errors. The graph is compiled in topological order.
  A `version` key is reserved for a future registry (beyond v1) and is ignored for path
  dependencies. The resolved graph is written to `stc.lock`; no network access or registry in v1.
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
- `[build] debug` is parsed from phase 1. M6 honors `[build].opt-level` but emits no
  DWARF/CodeView or checkpoint hooks; `debug = false` emits `STC-W-PROJECT-001`, and `--release`
  is a usage error until M6.5 implements the stripped/optimised form and its distinct build
  ID/manifest (D-158).
- `[build] syntax` is `strict` (default) or `relaxed`. Strict requires the 2013 statement
  terminator rules; relaxed enables the documented omitted-semicolon-after-block superset.
  `--relaxed` overrides the manifest.
- `[build] emit` is a non-empty subset of `ir,obj,module,exe,lib,metadata`. `module` emits the
  loadable module and its GOT manifest pair. `exe` is a thin launcher and `lib` is the embedding
  host library; generated code stays in the separate module, and `exe` and `lib` may coexist.
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
- `[runtime]` sets the runtime module/manifest directories and the number of retired generations
  retained for rollback/grace (`max-retired-generations`, default 1).
- `[online].baseline` is `active` (the host's committed baseline) or `explicit`; when it is
  `explicit`, `manifest` names the baseline path. `cold-policy` is `require-confirmation` in v1:
  a cold download needs an explicit confirmation or the documented cold path.
  `allow-untracked-removal` controls whether untracked root removal is allowed without a tracked
  rename (default false). `--got-manifest`, `--cold`, `--allow-untracked-removal`, and `--rename`
  override it.
- `[debug]` controls the default debug mode, local protocol transport (`pipe` default, `socket`
  optional), and whether every online swap ends the debug session (default true). Debug remains a
  local, authenticated capability; it is not a safety function. M6 parses the table but builds
  the debug form without debug information; `default = false` emits `STC-W-PROJECT-001`, and the
  protocol/session settings take effect in M6.5 (D-158).
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
- `[test].default-cycles` is the cycle count for FB/program tests when neither the `{stc:test}`
  payload nor `[[test.case]].cycles` sets one; the default is `1`.
- `[[test.case]].cycles` overrides the test's default cycle count.
- `timeout` is a duration string; the runner kills the test process on expiry. The default is 5 s.
- `[test].mock-clock` sets the initial mock time; tests advance the next cycle's latched time with
  `STC_CLOCK_ADVANCE` through host ABI 2.2 service ID 12 (D-157).
- `[[test.case]].expect-trap` (M6) records the expected trap code; a test that traps with that
  code passes, and any other trap fails it. The same key is accepted in the `{stc:test}` JSON
  payload; a pragma/manifest conflict is `STC-E-PROJECT-008`.

> **M6 phase 6 status (2026-10-26):** implemented. `stc test` parses production sources plus the
> root package's `[test].sources`, discovers only the current package's `{stc:test}` POUs (in
> production and test sources), and merges `[[test.case]]` overrides with the pragma. Both set
> the same field is `STC-E-PROJECT-008`; an unknown case name is also `STC-E-PROJECT-008`;
> duplicate names are `STC-E-TEST-002`; invalid signatures and reserved synthetic names are
> `STC-E-TEST-003`. The synthetic configuration uses lexable names (`STC_TEST`,
> `STC_TEST_RES`, `STC_TEST_TASK`, `STC_TEST_CPU`, `STC_TEST_INSTANCE`, `STC_TEST_WRAPPER`);
> the phase-0 `__STC_TEST__` spelling is not lexable and is superseded. Test artifacts go to
> `target/stc/test/<artifact>` (or `<out-dir>/<artifact>`), never join `[build].emit`, and
> never read a baseline or the active manifest store.

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
- `--locked` fails if the lockfile is missing (`STC-E-LOCK-001`) or would change
  (`STC-E-LOCK-002`: new/removed package, changed version/path/edge). A malformed or unsupported
  lockfile is `STC-E-LOCK-003`. CI uses `--locked`.
- `stc clean` does not delete `stc.lock` or retained-state files. Deleting them is an explicit
  user action.
- Local source contents are not hashed in v1; editing a path dependency's sources does not
  rewrite the lockfile. A content-hash field can be added later without changing the schema's
  package identity.

### 3.1 Rename history: `stc.renames.toml`

`stc rename` records tracked renames and moves in a committed `stc.renames.toml` at the root
package directory. The file is optional; a missing file is an empty history. Builds read it to
feed the existing `stc-got` rename maps, and `stc rename` appends to it after a successful source
rewrite.

```toml
schema = 1

[[rename]]
old = "Main.Speed"
new = "Main.Rate"
```

Rules:

- `schema` is required and currently `1`; unknown keys are errors (`STC-E-PROJECT-012`).
- Each `[[rename]]` entry has non-empty `old` and `new` strings in the same form accepted by
  `--rename` (a GOT key or a field path); entries are stored oldest first.
- Builds merge history entries oldest-first with CLI `--rename` entries in command order and
  resolve chains by composition: `A -> B` followed by `B -> C` is passed to the identity engine
  as `A -> C`. Duplicate `old` values with different `new` values, or a chain that revisits a
  key, are `STC-E-PROJECT-013`.
- `stc rename` rewrites only sema-resolved identifier tokens (the declaration and its references),
  leaves comments and strings untouched, preserves formatting and file encoding, and appends the
  entry only after the rewrite succeeds. `--dry-run` prints the edits and writes neither source
  nor history.
- Identity is preserved only when an online baseline is in use; a cold build emits
  `STC-W-PROJECT-002` and the new key receives a fresh `GotId`. Renaming a dependency's
  declaration, a package name, or a key outside the current package is out of scope.
- The history does not change the manifest format, state file, or module ABI.

## 4. Artifact and metadata layout

```text
target/stc/debug/                               default out-dir (project and bridge)
  <name>.ll
  <name>.o
  <name>.exe                                     thin host launcher (exe)
  <name>-launcher.ll | <name>-launcher.o         launcher IR/object when `exe`
  <name>.lib | <name>.a | <name>.dll/.so         embedding host library (lib; static or shared)
  <name>-<build-id>.dll | <name>-<build-id>.so  loadable module (module)
  <name>.stc-meta.json
  <name>.stc-got.json                           canonical GOT manifest
  <name>.stc-got.bin                            compact runtime manifest
  build.log
```

`<name>.stc-meta.json`:

```json
{
  "abi_version": 2,
  "artifact": "motor-control",
  "build_id": "0x…",
  "compiler": "stc 0.1.0",
  "configuration": "Main",
  "descriptor_digest": "0x…",
  "exports": [
    { "function": "stc_fn_…", "no_mangle": false, "symbol": "stc_export_Main" }
  ],
  "io": {
    "backend": "null",
    "bindings": [
      { "area": "I", "address": "IX5", "got_id": "0x…", "offset_bits": 5,
        "source": "pragma", "type": "BOOL", "variable": "Main.MyBool", "width_bits": 1 }
    ],
    "image": { "inputs_bytes": 16, "markers_bytes": 64, "outputs_bytes": 16 },
    "input_defaults_hex": "0000…",
    "output_defaults_hex": "0000…"
  },
  "manifest": { "digest": "0x…", "format": 1 },
  "mode": "executable",
  "module": { "file": "motor-control-<build-id>.dll", "format": 1 },
  "profile": "stc-2013-oop-utf8-v1",
  "programs": [
    { "name": "main_instance", "program": "Main", "resource": "Cpu", "task": "MainTask" }
  ],
  "retain": {
    "enabled": true,
    "file": "motor.stcstate",
    "on_mismatch": "error",
    "layout_hash": "0x…",
    "leaves": [
      { "offset": 0, "path": "Main.Counter", "size": 4, "type": "DINT" }
    ],
    "size": 4
  },
  "spans": {
    "entries": [ { "end": 42, "file": 0, "id": 0, "start": 12 } ],
    "files": ["src/main.st"]
  },
  "target": "x86_64-pc-windows-msvc",
  "tasks": [
    { "interval_ns": 1000000, "name": "MainTask", "priority": 1,
      "resource": "Cpu", "single": false }
  ]
}
```

The driver writes the schema above. M6 adds package/device/`VAR_ACCESS` metadata and the
`file`/`on-mismatch` retained-state policy (JSON field `on_mismatch`); these fields are absent
from M4.5 artifacts. `parent_build_id` is carried by the manifest lineage rather
than the metadata sidecar.

- `lib` is the embedding host library, not generated module code. `lib-type = "shared"` selects
  the shared host library; the default is a static host archive. `exe` is a thin launcher and
  emits the adjacent module pair. The generated module and manifest remain separate artifacts;
  `exe` and `lib` may be emitted together.
- The metadata is consumed by the runtime and by embedding hosts.
- It contains no timestamps or absolute paths, so builds are reproducible.
- The span table is optional in library mode if the host does not need diagnostics.
- `build_id` is content-derived; the online-change parent identity is carried by the GOT
  manifest lineage, not the metadata sidecar. The module/manifest digest pair is validated by
  the host before activation. The exact build-ID, descriptor, manifest, and pairing rules are in
  [GOT Artifacts and Schemas](GOT-Artifacts.md) §7–§11.
- The full identity/layout/history record is in the GOT manifest; metadata carries only the
  summary needed by tooling and the host.

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
[stc] artifacts target/stc/debug/motor-control.exe
```

`-vv` adds per-file phases, the exact clang command line, and retained IR paths. The log never
contains secrets; paths are printed as written by the user.

## 7. Open CLI/manifest questions

None as of 2026-10-24. The online-change CLI/tables/manifest schema is resolved in
[Online Change and GOT](Online-Change-and-GOT.md) and `update_plan_got.md` §13.0. New
CLI/manifest questions are appended here or in [Decisions](../Decisions.md).
