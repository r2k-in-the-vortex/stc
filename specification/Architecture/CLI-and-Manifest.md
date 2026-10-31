# CLI and Project Manifest

Status: M4.5 module/build CLI implemented (2026-10-25); M6 project manifest, diagnostics, report shapes, and all M6 commands implemented (2026-10-27); M6.5 phase 0 fixes the debug/release command, protocol, and `[debug]` schema contract (2026-10-27); M6.5 phase 4 raises the GOT manifest to format 2 (2026-10-27); M6.5 phase 7 adds the host-side manifest-assisted split presentation (2026-10-28); M6.5 phase 8 implements the `stc debug` CLI and server options (2026-10-28); M6.5 complete (2026-10-28); M7 adds the release package layout, runtime adjacency search, and release-candidate status (2026-10-28); WL-1.1 adds `stc lsp`, `stc dap`, and `--diagnostic-paths` (2026-10-30); WL-1.1 F-20 makes stdin EOF a default graceful stop for `stc run --watch` (D-213, 2026-10-31)
Profile: `stc-2013-oop-utf8-v1`  
Related: [Architecture](Architecture.md), [Online Change and GOT](Online-Change-and-GOT.md), [Debug and Monitoring](Debug-and-Monitoring.md), [Testing](Testing-and-Verification.md)

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
> on failure, and exits 130 on Ctrl-C. The watch runs until interrupted, the watch timeout, or
> standard-input EOF; EOF stops it gracefully with `[stc] watch stopped` and exit 0 (D-213). `--store`
> also selects the watch store, and `--watch-timeout <duration>` bounds a run for automation.
> `--message-format` supports
> `human`, `json`, and `short`; exit codes follow §1.3. The loadable `module` with its
> binary/JSON manifest and metadata, the thin `exe` launcher, and the embedding `lib` host
> archive keep their M4.5 shapes, and `lib-type = "shared"` selects the shared host library.
> M6.5 adds `--debug`/`--release`, debug information, checkpoint hooks, and the `stc debug` and
> `stc run --debug-server` surface; `--debug` is the default form and `--release` is the
> optimised, stripped, hook-free form (D-178).
>
> **M6.5 phase 0 status (2026-10-27):** the `stc debug` command, `--release`, debug information,
> checkpoint hooks, `--debug-protocol`, and the `[debug]` session policy are fixed in §1.7 and
> §2.2.
>
> **M6.5 status (2026-10-28):** phases 0–10 are implemented. The `stc debug`, `--debug`,
> `--release`, debug-information, checkpoint, `--debug-protocol`, and `[debug]` contracts are in
> place; the M6 behavior above is historical.

| Command | Purpose |
| --- | --- |
| `stc new <path>` | Scaffold a project (`stc.toml`, `src/main.st`, `.gitignore`) |
| `stc check` | Lex, parse, resolve, and type-check; no codegen |
| `stc build` | Emit artifacts selected by `--emit` / manifest; excludes test-only units |
| `stc run` | Build an executable and run it |
| `stc test` | Build and run the built-in test suite; includes test-only units |
| `stc fmt [paths...]` | Format sources with the fixed style; `--check` for CI |
| `stc clean` | Remove generated artifact directories: debug (`target/stc/debug`), release (`target/stc/release`), test (`target/stc/test`), and an explicit `[build].out-dir`; `--debug`, `--release`, or `--test` narrows the command to one tree, and `--out-dir` selects exactly one directory. Never `stc.lock`, retained-state files, or the runtime module/manifest store |
| `stc metadata` | Print the selected configuration, ABI version, build mode, debug summary, exports, and artifact layout |
| `stc compliance` | Print the profile ID and supported feature matrix |
| `stc online download` | Export the active manifest, layout summary, and controller/deployment identifier |
| `stc online diff` | Compare a candidate manifest with the active baseline and print compatibility |
| `stc rename` | Source-aware tracked rename that preserves `GotId` history |
| `stc run --watch` | Watch sources, rebuild, and deploy an online change |
| `stc debug` | Build and launch a local authenticated debug session, or attach with `--attach` (§1.7) |
| `stc lsp` | Serve the Language Server Protocol for the local project over stdio (§1.8) |
| `stc dap` | Serve the Debug Adapter Protocol for the local project over stdio; distinct from `stc debug` (§1.8) |

Future commands (not v1): `stc doc`.

### 1.2 Common options

| Option | Meaning |
| --- | --- |
| `--manifest-path <path>` | Use a specific `stc.toml` |
| `--target <triple>` | Pass a target triple to clang; emit it in the IR |
| `--release` | Optimised, stripped build; omits debug hooks and full debug info; distinct build ID/manifest; default out-dir `target/stc/release`. M6.5 implements it; debug is the default mode (D-178) |
| `--debug` | Force the debug form with debug information and checkpoint hooks (default unless `--release`); `[build].opt-level` still applies (D-178) |
| `--emit <list>` | Comma-separated `ir,obj,module,exe,lib,metadata`; `module` emits the loadable module and manifest pair; `exe` is a launcher and `lib` the embedding host library; the two may coexist; overrides manifest |
| `--got-manifest <path>` | Build against the given persistent GOT manifest as the identity baseline |
| `--cold` | Explicitly build without an online-change baseline (cold-layout build) |
| `--rename <old=new>` | Record a tracked rename/move; repeatable, including field/method/type/POU paths. Renaming a package name is out of scope (D-156) |
| `--clang <path>` | clang executable |
| `--runtime <path>` | Runtime host archive used when linking `exe`/`lib` |
| `--out-dir <path>` | Artifact directory; default `target/stc/debug` (debug) or `target/stc/release` (release); an explicit value is used verbatim for both modes; `[build].out-dir` overrides the project default; for `stc clean`, selects exactly the given directory |
| `--io <backend>` | Override `[io].backend` for `run`/`test` (`null`, `memory`, `file`) |
| `--io-file <path>` | File used by the file backend |
| `--relaxed` | Accept the documented semicolon-relaxation syntax superset |
| `--state-file <path>` | Retained-state file for this run |
| `--reset-retain` | Cold start: ignore existing state, overwrite on shutdown |
| `--no-retain` | Disable retained-state load and save |
| `--debug-protocol <kind>` | Local debug transport (`pipe` default, `socket` optional) for `stc debug` and `stc run --debug-server`; overrides `[debug].protocol` |
| `--allow-untracked-removal` | Permit removing/renaming a root without a tracked rename; otherwise diagnose and require `--cold` |
| `--diagnostic-paths <style>` | `package` (default) names project diagnostics `<package>/<relative>`; `absolute` names the source file by absolute path. Valid only with `check`, `build`, `run`, and `test` |
| `--message-format <fmt>` | `human` (default), `json`, `short` |
| `--color <when>` | `auto`, `always`, `never` |
| `-v`, `-vv` | Verbose phase output; `-vv` includes clang commands and IR paths |
| `-q`, `--quiet` | Suppress non-error output |
| `-D <lint>` / `-W <lint>` / `-A <lint>` | Deny/warn/allow a diagnostic group |
| `--deny warnings` | Equivalent to `-D warnings` |
| `--locked` | Require `stc.lock` to match the manifest/dependency graph; fail instead of updating it |
| `--list` | `stc test`: list discovered tests without building |
| `--test` | `stc clean`: remove only the test artifact tree (`target/stc/test`, or the `--out-dir` root) |
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
adds `--dry-run`. `--release`, debug information, `--debug-protocol`, and the `stc debug` /
`--debug-server` surface are implemented in M6.5; §1.7 fixes the contract. WL-1.1 implements
`--diagnostic-paths` and the `stc lsp` and `stc dap` servers; §1.8 fixes their surface.

### 1.3 Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Compile error or failing test |
| 2 | Usage error, manifest error, or missing toolchain |
| 3 | Internal compiler error (panic caught at the driver boundary) |
| 4 | Trap or runtime failure in `stc run`/`stc test` |
| 130 | Interrupted (Ctrl-C) |

`stc run --watch` exits 0 for a graceful stdin-EOF stop, for a normal `--watch-timeout` expiry, and
for a host exit with status 0; `--watch-timeout` otherwise returns the host's exit status, and a
host exit returns that status. Ctrl-C remains 130 (§1.7).

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
  including the M6 `PROJECT`, `LOCK`, `ASSERT`, `TEST`, `FMT`, `RUN`, and `ONLINE` additions,
  the M6.5 `DEBUG` family, and the semantic/codegen/build families, is
  [Conformance Profile](../01-Conformance-Profile.md) §10.
- Promoting a warning to an error through `[lints]`, `-D <code>`, `-W <code>`, `-A <code>`, or
  `--deny warnings` changes only its severity. The diagnostic keeps its original code and span,
  so a promoted `STC-W-*` warning renders as an error carrying the same `STC-W-*` code; the
  effective severity, not the code prefix, determines whether the build fails.
- Project diagnostics use `STC-E-PROJECT-*` / `STC-W-PROJECT-*`; lockfile diagnostics use
  `STC-E-LOCK-*`; test and assertion diagnostics use `STC-E-TEST-*` / `STC-E-ASSERT-*`;
  formatter diagnostics use `STC-E-FMT-*`.
- Online-change diagnostics use the `STC-E-MODULE-*`, `STC-E-MANIFEST-*`, `STC-E-ABI-*`, and
  `STC-E-ONLINE-*` / `STC-W-ONLINE-*` families; the M6 control-channel and deploy additions are
  `STC-E-ONLINE-007/008/009` and the compatibility-report warning is `STC-W-ONLINE-007`
  (`STC-W-ONLINE-002` remains the initializer warning). Debug diagnostics use the
  `STC-E-DEBUG-*` / `STC-W-DEBUG-*` families indexed in §10.1; invalid debug settings in
  `stc.toml` use `STC-E-PROJECT-010`.
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
  "artifacts": [ { "kind": "module", "file": "motor-control-<build-id>.dll" } ],
  "build_mode": "debug",
  "debug_info": "dwarf",
  "debug_hooks": true
}
```

The object is a summary; the sidecar in §4 remains the complete metadata. `on_mismatch` is a JSON
field name; the manifest key is `on-mismatch`. `build_mode` is `debug` or `release`; `debug_info`
is `dwarf`, `codeview`, or `none`; `debug_hooks` is the checkpoint-hook capability. The sidecar's
`debug` object (§4) is authoritative for the checkpoint table.

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
    "debug-monitoring": "implemented"
  },
  "safety": "not-a-safety-plc"
}
```

`features` maps the conformance profile's feature categories to `implemented`, `partial`,
`planned`, or `not-in-profile`; it is not a claim beyond
[Conformance Profile](../01-Conformance-Profile.md). `safety` is a fixed string, and the human
output includes the non-safety statement verbatim.

### 1.7 Debug command and server options

`stc debug` is the v1 session entry point (implemented in M6.5). It builds and launches the
project in debug mode by default, opens the local debug endpoint, starts one authenticated
session, and attaches the CLI.
`--attach <endpoint-file>` connects to an already-running host instead of launching. The wire
protocol is fixed in [Debug and Monitoring](Debug-and-Monitoring.md) §13–§18; this section fixes
the command surface.

**`stc debug` options.**

| Option | Meaning |
| --- | --- |
| `--attach <endpoint-file>` | Attach to a running host; incompatible with build/launch options. |
| `--endpoint <path>` | Endpoint file to create in launch mode (default a per-run path under the out-dir). |
| `--batch <file>` | Run request objects from a JSON-lines file, or `-` for stdin, and print responses/events as JSON lines; implies `--message-format json`. |
| `--break <file>:<line>[:<column>]` | Add an initial source breakpoint; repeatable. |
| `--watch <target>` | Add an initial watch target; repeatable. |
| `--allow-write` | Launch the server with the `write` scope and request it. |
| `--allow-force` | Launch the server with the `force` scope and request it. |
| `--session-timeout <duration>` | Launch-mode stop/idle bound; defaults to `[debug].session-timeout` (disabled by default; zero disables). |
| `--debug-protocol <kind>` | Launch transport (`pipe` or `socket`); overrides `[debug].protocol`. |
| `--debug` / `--release` | Build-mode selection; `stc debug` defaults to debug. |

`stc debug` accepts `--message-format human` (the default) or `json`; `short` is rejected as a
usage error. `--batch` implies `json`.

Launch mode also accepts the `stc run` run options (`--cycles`, `--cycle-time`, `--io`,
`--io-file`, `--state-file`, `--reset-retain`, `--no-retain`, `--mock-clock`) and the common
build/project options. `--attach` cannot be combined with build, launch, or server-grant options;
a combination is a usage error (exit 2).

Targets for `--watch` and the interactive commands are a normalized stable key path (for example
`MAIN.COUNTER`) or `0x` + 32 hex `GotId`. A source spelling is resolved through the local
metadata where available; locals are addressed only through the stopped-frame commands. A bad
target fails the command with `STC-E-DEBUG-008`.

The batch file contains one object per line with `op` and `params`; the CLI adds `kind`,
`protocol`, `id`, and `token`, writes responses and events to stdout in arrival order, and stops
at `session-ended`. The batch file is a client-side automation format; the wire schema remains
the normative one.

Interactive commands (phase 8) are `continue`, `pause`, `step`, `next`, `finish`, `break`,
`clear`, `print`, `set`, `force`, `release`, `release-all`, `watch`, `unwatch`, `stack`, `frame`,
and `quit`. They map to the wire operations and add no protocol semantics. `print` and `frame`
present the optional `view` result from Debug and Monitoring §15.5: the text names the view
manifest-assisted, shows per-field runtime roots/handles rather than a raw struct dump, and never
prints an address, absolute path, or timestamp.

`watch` accepts an optional interval in milliseconds and defaults to 1000 ms; initial `--watch`
targets use the same default. The server clamps the interval to its 50 ms minimum (Debug and
Monitoring §13.5).

**Launch versus attach.** Launch mode owns the target: on `quit` or EOF it stops the session,
clears forces and breakpoints, shuts the host down, and returns 0 on a clean shutdown, 4 on a
target trap, and 130 on interrupt. Attach mode stops the session (clearing state), detaches, and
leaves the host running; it returns 0 on a clean detach. A session ended by a swap, a timeout, or
a protocol error is reported with its `STC-E-DEBUG-*` code and exits 2. The CLI never prints the
token; the endpoint path is printed on stderr in launch and server modes. The non-safety notice is
printed at session start and included in `--help`; a stopped or forced controller may hold outputs
indefinitely. `stc` may control machinery, but it is not a safety PLC and safety must be provided
by a separate system and machine design as required by law and regulation.

**`stc run --debug-server`.** `stc run` gains:

| Option | Meaning |
| --- | --- |
| `--debug-server` | Start the local debug service after the host is running. |
| `--debug-endpoint <path>` | Endpoint file to write (default a per-run path under the out-dir). |
| `--allow-write` | Grant the `write` scope. |
| `--allow-force` | Grant the `force` scope. |
| `--session-timeout <duration>` | Stop/idle bound; defaults to `[debug].session-timeout` (disabled by default; zero disables). |

`--debug-server` is valid for debug and release modules; a release module yields a
monitoring-only session (`STC-W-DEBUG-004`). `stc test` rejects `--release`, `--debug-server`,
`--debug-protocol`, and the server-grant options. `stc run --watch` and `--debug-server` may
coexist: the M6 control endpoint and the M6.5 debug endpoint have different names, tokens,
protocols, and messages (D-155, D-173).

**`stc run --watch` lifetime.** The watch runs until interrupted, the `--watch-timeout` bound, or
standard-input EOF. It reads stdin only to detect EOF and never consumes commands from it. On EOF
(a terminal Ctrl-D/Ctrl-Z, `/dev/null`, or a closed stdin pipe) it stops the host through the same
control client as Ctrl-C, prints `[stc] watch stopped`, and exits 0; Ctrl-C prints `[stc] watch
interrupted` and exits 130. `--watch-timeout` stops the host, prints `[stc] watch timeout reached`,
and returns the host's exit status (0 for a clean stop). A host that exits on its own ends the
watch with the host's status and prints `[stc] host exited with status <n>` (D-213). The watch does
not run the stale-endpoint preparation that non-watch debug launches use: an explicit
`--debug-endpoint` must be a fresh path for each watch run, or startup fails with
`STC-E-DEBUG-020`/`STC-E-DEBUG-021`; a caller that uniquely names the path per run, as the VS Code
extension does, is unaffected. IDE Integration §4.7 fixes the editor surface that uses this
contract.

### 1.8 IDE servers (`stc lsp` and `stc dap`)

Both servers own standard input and standard output. Standard output carries protocol messages
only; logs, build output, and the debug endpoint line go to standard error. Both accept only
`--manifest-path <path>` (plus `--help`/`--version`); diagnostic, build, and formatting options are
rejected. The editor contracts are fixed in
[IDE Integration](IDE-Integration.md) §3 and §4.

**`stc lsp`.** Serves the Language Server Protocol with `Content-Length` framed JSON-RPC over
stdio. `--manifest-path` selects the project; otherwise the server discovers `stc.toml` from the
initialize `rootUri`, then `workspaceFolders`, then the process working directory, searching
ancestors. A missing or unloadable project fails `initialize` with a readable error and logs the
detail to stderr. The server exits 0 after the `shutdown`/`exit` handshake and 1 when input closes
or `exit` arrives without `shutdown`. It provides push diagnostics, document symbols, definition,
references, hover, completion, and whole-document formatting, with full synchronization and
UTF-16 positions; the closed unsupported list is in IDE Integration §3.3.

**`stc dap`.** Serves the Debug Adapter Protocol over stdio. Launch and attach settings arrive as
DAP messages (`project`, `noBuild`, `outDir`, `release`, `allowWrite`, `allowForce`, `sourceRoot`,
`args`, `cwd`, and `endpoint` for attach); `--manifest-path` is only a fallback project path.
Launch builds the debug form by default and starts a local endpoint through the same driver
plumbing as `stc debug`; `noBuild` launches existing artifacts. The adapter translates DAP to
`stc-debug/1.1` and falls back to `stc-debug/1`, reports unsupported capabilities honestly, and
owns no compiler or debugger logic. `disconnect` stops the session and exits the adapter; a
launch-owned host is shut down, while attach leaves it running. `stc dap` is distinct from
`stc debug`: `stc debug` is the interactive CLI client, and `stc dap` is the editor server.

`--diagnostic-paths` affects project-mode diagnostics only. `package` is the default and keeps
`<package>/<relative>` names for human and JSON output; `absolute` names the source file by its
absolute path so an editor problem matcher can resolve multi-package sources. `stc run --watch`
forwards the selected style to its child build.

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
debug = true                      # debug form (default); --release selects the release form (D-178)
syntax = "strict"                 # strict (default) | relaxed
emit = ["module", "ir"]           # v1 online-change deployment; exe/ir/lib also available
lib-type = "static"               # static | shared, used when emit includes "lib"
# out-dir = "target/stc/debug"    # optional; default is target/stc/debug or target/stc/release
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
force-sessions-end-on-swap = true   # must be true in v1
# session-timeout = "5m"            # optional continuous stop/idle bound; omit or "0ns" to disable
allow-write = false                 # grant the write scope (implemented, M6.5)
allow-force = false                 # grant the force scope (implemented, M6.5)

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
- `[project] configuration` selects one `CONFIGURATION` declaration. Only `CONFIGURATION`
  declarations in the root package are build entries. Configurations declared by dependency
  packages are declarations, not selectable builds; naming one is `STC-E-PROJECT-007`
  ("configuration `<name>` is not declared in package `<root>`"). If the key is omitted and
  exactly one root-package configuration exists, it is selected. If omitted and multiple exist,
  the build fails with a list. When no configuration is declared, the synthesized `_default`
  plan considers only programs declared in the root package (D-083).
- `[build] target` is passed to clang and emitted as the IR triple.
- `[build] opt-level` is `0|1|2|3|"s"`; maps to clang `-O` flags. When the key is absent, debug
  builds default to 0 and release builds to 2 (D-178).
- `[build] debug` selects the default build mode (default `true`). It is the canonical key and
  must agree with an explicitly present `[debug].default`; a disagreement is
  `STC-E-PROJECT-010`. `--debug`/`--release` override both. Debug builds emit DWARF/CodeView and
  checkpoint hooks; release builds are optimised, stripped, and hook-free with distinct build IDs
  and manifests (D-178).
- `[build] syntax` is `strict` (default) or `relaxed`. Strict requires the 2013 statement
  terminator rules; relaxed enables the documented omitted-semicolon-after-block superset.
  `--relaxed` overrides the manifest.
- `[build] emit` is a non-empty subset of `ir,obj,module,exe,lib,metadata`. `module` emits the
  loadable module and its GOT manifest pair. `exe` is a thin launcher and `lib` is the embedding
  host library; generated code stays in the separate module, and `exe` and `lib` may coexist.
- `[build] lib-type` selects static or shared library output.
- `[build] out-dir` is relative to the manifest and must stay inside the project unless the user
  passes `--out-dir`. The default is `target/stc/debug` (debug) or `target/stc/release`
  (release). An explicit out-dir is used verbatim for both modes; building both modes into one
  explicit directory is not a supported layout because manifest/metadata names collide (D-178).
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
- `[debug]` controls the default build mode (`default`), local transport (`protocol`, `pipe`
  default or `socket`), the swap teardown policy (`force-sessions-end-on-swap`, which must be
  `true` in v1), the optional stop/idle bound (`session-timeout`, a duration string; unset or
  `0ns` disables it), and the default write/force grants (`allow-write`, `allow-force`, both
  false). `[debug].default` must agree with an explicitly present `[build].debug`; a disagreement
  or `force-sessions-end-on-swap = false` is `STC-E-PROJECT-010`. `session-timeout` must be zero
  or a positive duration and must not exceed 24 hours; `allow-write`/`allow-force` are booleans.
  These settings are runtime/session policy and do not affect the build ID or manifest identity.
  `stc test` ignores them and always builds the hook-free debug form. Debug remains a local,
  authenticated capability; it is not a safety function (D-173–D-179).
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
  user action. It also preserves the runtime module/manifest store (including the default
  `target/stc/runtime`) when a removed artifact tree contains it.
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
target/stc/debug/                               default debug out-dir (project and bridge)
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

target/stc/release/                             default release out-dir (same layout; stripped)

target/stc/test/<artifact>/                     per-test artifact (stc test; hook-free debug form)
  <artifact>.ll
  <artifact>.o
  <artifact>.exe | <artifact>                    thin host launcher (exe)
  <artifact>-launcher.ll | <artifact>-launcher.o
  <artifact>-<build-id>.dll | <artifact>-<build-id>.so
  <artifact>.stc-meta.json
  <artifact>.stc-got.json
  <artifact>.stc-got.bin

target/stc/runtime/                             default runtime store (`stc run --watch`)
  modules/                                      runtime module generations (when configured)
  manifests/                                    active/retired GOT manifests
```

`stc clean` removes the debug, release, and test trees and an explicit `[build].out-dir`. It
preserves `stc.lock`, every retained-state file (`*.stcstate`), an explicit `[retain].file`, and
the runtime module/manifest store (`[runtime].module-dir`, `[runtime].manifest-dir`, or the
default `target/stc/runtime`); a preserved entry keeps its parent directories in place. With no
selector, clean covers the three generated trees and the explicit out-dir; `--debug`, `--release`,
or `--test` narrows the command to one, and `--out-dir` selects exactly the given directory.
Cleaning a directory that contains the project root is refused. A runtime store selected only by a
`stc run --watch --store` CLI override is not discoverable by clean; keep it outside the generated
trees or delete it explicitly.

### 4.1 Release package layout

`cargo xtask package --locked [--out <dir>]` builds `stc-driver` and `stc-runtime` in release
mode (always `--locked`) and writes a versioned, timestamp-free package:

```text
stc-<version>-<host-triple>/
  bin/stc(.exe)
  lib/stc_runtime.lib | libstc_runtime.a      static host archive
  lib/stc_runtime.dll | libstc_runtime.so     shared host library
  docs/README.md
  docs/USER-GUIDE.md
  docs/RELEASE-NOTES.md
  docs/KNOWN-LIMITATIONS.md
  docs/NON-SAFETY.md
  package.json
  SHA256SUMS
```

- One package is produced per native host triple: `x86_64-pc-windows-msvc`,
  `x86_64-unknown-linux-gnu`, and `aarch64-unknown-linux-gnu`. There is no combined archive.
- `package.json` records `schema`, `version`, `profile`, `host`, `debug_contract`, the supported
  artifact/ABI versions, the artifact list, and the non-safety identifier. It contains no
  timestamp, absolute path, user name, or random ID. `debug_contract` is host-derived:
  `object-codeview` on Windows MSVC and `dwarf` on Linux GNU.
- `SHA256SUMS` is sorted by package-relative path (`/` separators), lists every file in the
  package except itself, and is compatible with `sha256sum -c`.
- Runtime discovery checks, in order: `--runtime`, `STC_RUNTIME`, `[build].runtime`, a file beside
  the executable, `<exe>/lib`, `<exe>/../lib`, and the workspace `target/{release,debug}` fallback.
  The `bin`/`lib` package layout therefore works without environment variables.
- The unpacked-package exercise runs `stc --version`, `stc compliance`, and then `new`, `check`,
  `build`, `build --release`, `build --release --emit=lib`, `run --cycles 2`, `test`, `clean`, and
  a final `build` that proves the project is usable after the clean.
- Static archives are unpacked, native object members are stripped with
  `llvm-objcopy --strip-debug`, and the stripped objects replace the originals with `llvm-ar rD`;
  every import-library or LLVM-bitcode member and symbol-table entry is preserved. The shared host
  library is stripped directly. `package.json` and the package are written only after the archive,
  shared library, module, and executable checks pass.
- The package is an archive, not an installer; there is no package-manager integration in v1.

`<name>.stc-meta.json`:

```json
{
  "abi_version": 2,
  "artifact": "motor-control",
  "build_id": "0x…",
  "compiler": "stc 1.0.0",
  "configuration": "Main",
  "descriptor_digest": "0x…",
  "debug": {
    "build_id": "0x…",
    "checkpoints": [
      { "column": 3, "flags": 1, "function": "0x…", "id": 1, "line": 6, "span": 12 }
    ],
    "format": 1,
    "hooks": true,
    "info": "dwarf"
  },
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
  "manifest": { "digest": "0x…", "format": 2 },
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
from M4.5 artifacts. The `manifest.format` value is the paired GOT manifest's outer format:
format 2 is emitted from M6.5 phase 4 onward, while the module and descriptor formats in this
sidecar remain 1 and the metadata schema itself is unchanged (D-180). Debug-form artifacts carry
the optional `debug` object (with `hooks` true or false and a possibly empty checkpoint array);
release artifacts omit it. The object is paired with the module/manifest by `build_id` but is
not part of build-ID derivation, manifest identity, or online-change compatibility; it is
validated per build (D-177). `parent_build_id` is carried by the manifest lineage rather than the metadata sidecar.

- `lib` is the embedding host library, not generated module code. `lib-type = "shared"` selects
  the shared host library; the default is a static host archive. `exe` is a thin launcher and
  emits the adjacent module pair. The generated module and manifest remain separate artifacts;
  `exe` and `lib` may be emitted together.
- The metadata is consumed by the runtime and by embedding hosts.
- It contains no timestamps or absolute paths, so builds are reproducible.
- The span table is optional in library mode if the host does not need diagnostics. A debug
  artifact that emits checkpoint hooks includes the span table entries its checkpoints reference;
  a checkpoint with an invalid span index is `STC-E-DEBUG-016`.
- The `debug` object is the checkpoint table defined in
  [Debug and Monitoring](Debug-and-Monitoring.md) §16: `build_id`, `format`, `hooks`, `info`, and
  the dense `checkpoints` array (`id`, `span`, `function`, `line`, `column`, `flags`). It is
  emitted only for debug-form artifacts; release artifacts have no `debug` key.
- Release artifacts use the same layout under the release default out-dir, with debug information
  stripped and no `debug` object. Debug and release sets never mix in one directory.
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

None as of 2026-10-28. The M6.5 debug/release CLI, protocol, and `[debug]` schema were resolved
in §1.7, §2.2, §4, and [Decisions](../Decisions.md) D-173–D-183. The online-change
CLI/tables/manifest schema is resolved in [Online Change and GOT](Online-Change-and-GOT.md) and
[Decisions](../Decisions.md). New CLI/manifest questions are appended here or in
[Decisions](../Decisions.md).
