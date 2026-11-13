# `stc` user guide — 2.0.0

Profile: `stc-2013-oop-utf8-v1`

`stc` compiles IEC 61131-3:2013 Structured Text to textual LLVM IR, then invokes
`clang` to produce a loadable module, a thin launcher executable, or an
embedding host library. A persistent host runtime provides online change,
retained state, I/O backends, and typed debug/monitoring.

> **`stc` is not a safety PLC and is not responsible for safety.** It can
> control real machinery, but safety must be provided by a separate safety
> system and machine design as required by law and regulation. Debug sessions
> can hold outputs indefinitely. See [`NON-SAFETY.md`](NON-SAFETY.md).

## 1. Install

A release package has this layout:

```text
stc-2.0.0-<host-triple>/
  bin/stc(.exe)
  lib/stc_runtime.lib + stc_runtime.dll        (Windows MSVC)
  lib/libstc_runtime.a + libstc_runtime.so     (Linux GNU)
  lib/libstc_runtime.a + libstc_runtime.dylib  (macOS)
  docs/README.md
  docs/USER-GUIDE.md
  docs/RELEASE-NOTES.md
  docs/KNOWN-LIMITATIONS.md
  docs/NON-SAFETY.md
  package.json
  SHA256SUMS
```

A package is produced for each native host triple:

```text
stc-2.0.0-x86_64-pc-windows-msvc
stc-2.0.0-x86_64-unknown-linux-gnu
stc-2.0.0-aarch64-unknown-linux-gnu
stc-2.0.0-aarch64-apple-darwin
stc-2.0.0-x86_64-apple-darwin
```

Linux GNU packages exist for both x86-64 and AArch64, and macOS packages exist
for both Apple silicon and Intel, each with a host-derived `package.json`
`debug_contract` (`object-codeview` on Windows MSVC, `dwarf` on Linux GNU and
macOS).

Unpack the archive and put `bin` on `PATH` (or call `bin/stc` directly). The
driver finds the runtime archive beside its executable or in the sibling
`lib/` directory, so no environment variable is required for the packaged
layout. Verify the download first:

```sh
sha256sum -c SHA256SUMS
stc --version
stc compliance
```

On macOS use `shasum -a 256 -c SHA256SUMS`; the package CI verifies the same
file that way.

Requirements:

- `clang`/LLVM for assembling and linking IR; `llvm-objcopy` for release
  stripping; `llvm-ar` for release archive stripping; `llvm-readobj`,
  `readelf`, or `llvm-dwarfdump` for stopped-frame sidecar extraction.
- On Windows, clang, `llvm-objcopy`, and `llvm-ar` are normally in the LLVM
  installation directory. The driver finds them beside `clang` or on `PATH`.
- On macOS, install Homebrew `llvm@18` and put its `bin` directory on `PATH`.
  Release packages use `llvm-strip --strip-debug` instead of `llvm-objcopy`
  because Mach-O debug sections live in the `__DWARF` segment.
- No LLVM development library is needed; the compiler emits text IR.

Toolchain discovery order follows CLI > environment > `stc.toml` > built-in
default:

| Setting | CLI | Environment | Manifest |
| --- | --- | --- | --- |
| clang | `--clang <path>` | `STC_CLANG` | `[build].clang` |
| runtime archive | `--runtime <path>` | `STC_RUNTIME` | `[build].runtime` |
| target | `--target <triple>` | `STC_TARGET` | `[build].target` |

## 2. Quick start

```sh
stc new hello
cd hello
stc check
stc build
stc run --cycles 5
stc test
stc fmt --check
```

`stc new` writes `stc.toml`, a production source tree, a `[test].sources`
tree, and a committed `stc.lock`. `stc build` writes a loadable module, its GOT
manifest pair, and metadata under `target/stc/debug`. `stc run` also links a
thin launcher and executes the configured tasks. `stc test` builds each
`{stc:test}` POU into its own process.

The committed examples are the best starting points:

- [`examples/hello`](examples/hello/README.md) — language and workflow basics.
- [`examples/motor`](examples/motor/README.md) — a two-package project with
  OOP, an FB, UTF-8 strings, I/O, retained state, tests, debug, and online
  change (`examples/motorlib` is its dependency).
- [`examples/online-change`](examples/online-change/README.md) — the minimal
  hot-swap, rename, diff, and cold-download walkthrough.

## 3. Project manifest

```toml
schema = 1

[package]
name = "motor"
version = "0.1.0"
language = "stc-2013-oop-utf8-v1"

[dependencies]
motorlib = { path = "../motorlib" }   # local path only in this release

[project]
sources = ["src/**/*.st"]
configuration = "MotorDemo"           # optional if exactly one exists

[build]
emit = ["module", "metadata"]         # module | exe | lib | ir | obj | metadata
debug = true                          # debug form (default); --release selects release
opt-level = 0                         # defaults: 0 debug, 2 release

[run]
cycles = 0                            # 0 = until interrupted
cycle-time = "10ms"

[io]
backend = "null"                      # null | memory | file
file = "io.bin"

[retain]
file = "state/motor.stcstate"
on-mismatch = "reset"                 # error | reset | cold

[test]
sources = ["tests/**/*.st"]
timeout = "5s"

[debug]
default = true
protocol = "pipe"                     # pipe | socket
session-timeout = "5min"              # optional; unset or "0ns" disables
allow-write = false
allow-force = false
```

- `[package].name` is the package identity and namespace. Dependencies use
  local paths; the dependency's package name must match the alias unless an
  explicit `package = "..."` alias is given. The graph is acyclic and compiled
  in topological order.
- `stc.lock` pins the graph and is committed. `--locked` makes a stale graph an
  error; CI uses it everywhere.
- Production builds exclude `{stc:test}` POUs and `STC_ASSERT`. Test sources
  are compiled only by `stc test`.
- The full schema, validation rules, and diagnostic codes are in
  [`specification/Architecture/CLI-and-Manifest.md`](specification/Architecture/CLI-and-Manifest.md)
  §2.

## 4. Build modes and artifacts

| Mode | Selection | Default out-dir | Contents |
| --- | --- | --- | --- |
| Debug | default or `--debug` | `target/stc/debug` | DWARF (ELF/Mach-O) or CodeView (MSVC), checkpoint hooks, `debug_check` import, stopped-frame sidecar |
| Release | `--release` | `target/stc/release` | optimised, stripped, hook-free, distinct build ID/manifest, no debug sidecars |
| Test | `stc test` | `target/stc/test/<artifact>` | hook-free debug form, ABI 2.2 test clock, never deployed |

Artifacts for `--emit=module`:

```text
<name>.ll                     deterministic textual LLVM IR
<name>.o                      object
<name>-<build-id>.dll|.so     loadable ABI 2 module
<name>.stc-got.json           canonical-JSON GOT manifest
<name>.stc-got.bin            compact binary GOT manifest
<name>.stc-meta.json          metadata sidecar (debug object in debug form)
<name>.stc-frames.json        stopped-frame sidecar (hook-bearing debug only)
```

`--emit=exe` adds a thin launcher that loads the adjacent module pair;
`--emit=lib` adds the embedding host archive (or shared host library, by
`lib-type`). The two may be combined. Debug and release artifacts must use
distinct output directories; an explicit out-dir used for both modes is
`STC-E-PROJECT-010`.

Formats and versions in this release:

| Contract | Version |
| --- | --- |
| GOT manifest / catalog | 2 |
| Descriptor, module, state, metadata sidecar, debug object | 1 |
| Stopped-frame sidecar | 2 |
| Host ABI | 2.3 (test 2.2, production 2.1) |
| Debug protocol | `stc-debug/1`, plus the additive `stc-debug/1.1` revision |

IR and manifests are deterministic functions of source, options, and
toolchain: no timestamps, absolute paths, host names, user names, random IDs,
or addresses. Double builds are byte-identical, including PE timestamps.

## 5. Commands

| Command | Purpose |
| --- | --- |
| `stc new <dir>` | Scaffold a project |
| `stc check [files...]` | Lex, parse, and analyse; no artifacts |
| `stc build [files...]` | Emit selected artifacts |
| `stc run` | Build `exe,module,metadata` and run the tasks |
| `stc debug` | Build and launch a local authenticated debug session |
| `stc run --debug-server` | Run with a debug service beside the cycle loop |
| `stc test` | Build and run `{stc:test}` POUs in separate processes; `--coverage` reports ST statement coverage |
| `stc fmt [--check]` | Format with the fixed style |
| `stc clean [--debug\|--release\|--test]` | Remove generated trees; preserves the runtime store and state |
| `stc metadata [--message-format json]` | Print the build/artifact summary |
| `stc compliance [--message-format json]` | Print the profile, extensions, capabilities, and non-safety stance |
| `stc online download` | Export the active manifest from the runtime store |
| `stc online diff <candidate>` | Compare a candidate with a baseline/store/live host |
| `stc rename <old> <new>` | Tracked rename of a source declaration and every resolved reference |
| `stc run --watch` | Rebuild and deploy online while watching sources |
| `stc parse <file> [--ast]` | Dump the CST (and typed AST) |
| `stc lsp` | Serve the Language Server Protocol over stdio |
| `stc dap` | Serve the Debug Adapter Protocol over stdio |

Global options include `--message-format human|short|json`,
`--color auto|always|never`, `-q`, `-v`/`-vv`, `--manifest-path`,
`--locked`, `--relaxed`, and lint controls (`-D`, `-W`, `-A`, `--deny
warnings`). Run `stc --help` for the complete list and exit codes.

### 5.1 Rename

```sh
stc rename AddStep Advance
stc rename AddStep Advance --dry-run
stc rename Accumulator.increment Accumulator.delta --manifest-path stc.toml
```

`<old>` is a dotted path in the root package. `stc rename` finds the declaration through the
compiler's resolution, then rewrites the declaration, every resolved source reference, and the
qualified path prefixes of namespaces, POUs, types, and enum values. It also edits the resolved
external references in the same command: `[project].configuration`, `[[io.bindings]].variable`,
`[[test.case]].name`, the selected machine profile's `[[io.routes]]` endpoint `variable`, and
`VAR_ACCESS`/`VAR_CONFIG` path segments. A reference is never left stale.

- Only root-package declarations are renameable. Dependency declarations, package names,
  standard functions, and built-in types are rejected with a message.
- The new name must be one identifier that is not a reserved keyword; a case-only change is
  allowed. A new path that already names a different declaration in the root package is
  `STC-E-PROJECT-013`, reported before any file changes.
- `stc.renames.toml` receives canonical root-package history only for identity-bearing targets:
  namespaces, aliases, subranges, enums, named values, structs, functions, function blocks,
  programs, classes, interfaces, methods, globals, fields, and enum values. Parameters, locals,
  configurations, and case-only renames write no entry because they have no `GotId` identity.
- A selected-configuration rename edits `[project].configuration` and warns that the next online
  download may be cold, because the configuration name appears in task-guard root keys.
- `--dry-run` prints every source, manifest, profile, and access edit and the history decision,
  and writes nothing.
- A field rename inside a `FUNCTION_BLOCK` or `CLASS` is the one rename that cannot deploy online
even with history: the owner type's `TYPEINFO` descriptor changes shape, so the next candidate is
rejected with `STC-E-ONLINE-001` and a cold download is required. The history still preserves the
identity.
- `--locked` still fails on a stale lockfile before any edit, `--relaxed` accepts the documented
  syntax superset, and `--manifest-path` selects the project.

The VS Code extension performs the same rename through the same engine; see §8.3 for the tracked
history handshake.

## 6. Online change

```sh
stc build                       # baseline generation
stc run --watch                 # host + rebuild-on-save + cycle-boundary swap
stc online download --out active.json
stc online diff target/stc/debug/<name>.stc-got.json --baseline active.json
```

- Code-only changes and field add/remove are compatible; tracked renames
  preserve `GotId`s. A field rename inside a `FUNCTION_BLOCK` or `CLASS` is
  rejected by the next online candidate (`STC-E-ONLINE-001`) and needs a cold
  download. Type, size, array, and string layout changes are rejected or
  require a cold download.
- A failed build, prepare, or compatibility check leaves the previous
  generation active.
- `stc run --watch` runs until interrupted, the `--watch-timeout` bound, or
  standard-input EOF. Ctrl-C stops it gracefully with exit 130; closing stdin
  (Ctrl-D/Ctrl-Z in a terminal, `/dev/null`, or a closed pipe) stops it
  gracefully with `[stc] watch stopped` and exit 0.
- Every swap ends a debug session and clears forces/breakpoints.
- Format-1 manifests are cold-download only; no migration is provided.
- Debug and release forms have distinct identities and are not a supported
  hot-swap pair; keep separate stores.
- The runtime store (`target/stc/runtime` by default) holds the active
  manifest and module; `stc clean` preserves it.

The complete model is in
[`specification/Architecture/Online-Change-and-GOT.md`](specification/Architecture/Online-Change-and-GOT.md)
and [`GOT-Artifacts.md`](specification/Architecture/GOT-Artifacts.md).

## 7. Debug and monitoring

```sh
stc debug                                   # build, launch, interactive session
stc debug --break src/main.st:10            # initial breakpoint
stc debug --attach <endpoint-file>          # attach to a running host
stc debug --attach --remote r2k@ecat        # attach to a remote serve
stc debug --remote r2k@ecat --remote-stc /opt/stc/bin/stc \
  --remote-deploy online                    # remote launch or attach
stc remote status --remote r2k@ecat         # drift and online-change report
stc debug --batch commands.txt              # scripted commands
stc run --debug-server --cycles 0           # run with a debug endpoint
```

Targets are normalized stable key paths or `GotId` paths, for example
`MAIN.COUNTER.VALUE`, `MAIN.SUM`, or a whole aggregate. Available commands
include `print`, `set`, `force`, `release`, `release-all`, `watch`,
`unwatch`, `break`, `clear`, `continue`, `step`, `stack`, `frame`, `scope`,
`audit`, and `quit`.

- Starting a session does not stop the cycle. A stop/breakpoint/step holds
  outputs. The optional `[debug].session-timeout` bound is disabled by default;
  when configured, expiry ends the session and resumes the cycle. A VS Code
  launch session names the effective bound at startup.
- A launch (`stc debug` or a VS Code launch configuration) starts the host
  gated: cycle 0 does not run until the debug session is established and every
  initial breakpoint is bound, so a first-statement breakpoint observes cycle-0
  values. Attach cannot pause an already-running host; its breakpoints bind at
  the next checkpoint.
- Session teardown is reported in the Debug Console: a client stop, a host exit
  with its exit code, a trap (exit code 4), an online swap, a timeout with its
  `STC-E-DEBUG-*` code, and a connection loss are distinguished. `stc debug`
  prints the same distinctions in its own output.
- Writes and forces are queued and applied at a cycle boundary, revalidated
  for generation, root, type, layout, and writability. A `stc-debug/1.1`
  client that writes while the target is stopped gets an immediate
  `status:"applied"` result: the parked cycle applies the write before the
  response, so a read or step in the same stop sees it and no boundary applies
  it again. The `stc debug` CLI and `stc-debug/1` clients keep the queued
  behavior. Forces apply at a boundary on every version.
- Release builds are monitoring-only; breakpoints, stepping, and full locals
  require a debug build. Monitor live Globals and target paths from the Debug
  Console; Variables and Watch require a stopped frame.
- A selected `{stc:test}` POU can be debugged through the DAP `test` launch
  field (see §8.4). Set it to a name printed by `stc test --list`; the run
  builds only that test as a hook-bearing harness under
  `target/stc/test/debug/<artifact>`, applies breakpoints before the test body
  executes, and ends when the test prints its result line and exits. `stc test`
  itself remains hook-free and its artifacts are unchanged. `test` cannot be
  combined with `noBuild`, `release`, or attach.
- Debug is local by default. Remote debugging uses an OpenSSH local port-forward to the
  existing loopback `socket` endpoint; the runtime opens no network listener and the wire
  protocol is unchanged. `stc debug --remote <user@host>` owns a remote `stc serve` and stops it
  at teardown; `--attach --remote` leaves it running. The remote work directory holds the source
  mirror, build outputs, descriptor, and user-only endpoint/nonce files. The endpoint file still
  contains the per-run token; never share it.
- A remote attach with source drift chooses `remoteDeploy`: `ask` (the VS Code Online/Cold/Cancel
  prompt), `online` (control-channel deploy when the plan is compatible), `cold` (stop, install,
  restart), or `never` (reject drift with `STC-E-REMOTE-003`). `stc remote status` reports drift
  and whether online is possible without changing the running host.
- The remote session requires an SSH key/agent and a `known_hosts` entry; it never prompts for a
  password or accepts an unknown host key. There is no automatic reconnect after a tunnel or host
  loss; start a new attach and re-apply breakpoints. Remote debugging adds no safety claim.
- There is no raw-memory access or unauthenticated mode.
- Every swap/rollback/cold install/shutdown/exit ends the session.
- The server accepts `stc-debug/1` and the additive `stc-debug/1.1` revision. A 1.1 client can
  list a stopped frame's local names, types, and liveness with `frame.locals`; values still come
  from `frame.read`, and the endpoint-file `protocol` field stays `stc-debug/1`.

See
[`specification/Architecture/Debug-and-Monitoring.md`](specification/Architecture/Debug-and-Monitoring.md)
§12–§18 and [`KNOWN-LIMITATIONS.md`](KNOWN-LIMITATIONS.md) §3 for the limits.

## 8. VS Code integration

The local extension in [`editors/vscode`](editors/vscode) provides syntax highlighting, language
features, tasks with compiler diagnostics, formatting, and a debug adapter for `stc` projects.

### 8.1 Install

Install the VSIX from the release artifacts (VSIX-only distribution in v2.0.0; it is not published
to the marketplace):

```sh
code --install-extension stc-vscode-0.1.0.vsix
```

Or open **Extensions**, choose **...**, **Install from VSIX...**, and select the file. VS Code
1.82 or newer is required. The VSIX is platform-independent: it bundles no native binary and
works on Windows, Linux, and macOS against a locally installed `stc` package. The extension does
not bundle a compiler: install the `stc` release package and either put `bin` on `PATH` or set
`stc.path` to the executable. A missing compiler is reported in the **stc** output channel; the
language server is not started until it is resolved.

Open a folder containing `stc.toml`. The extension starts one language server per project and
watches `**/*.st` and `**/stc.toml`. No configuration is required beyond the compiler path.

An invalid `stc.toml` fails language-server initialization. The extension does not retry a
deterministic project-load error: the `STC-E-PROJECT-*` detail appears in the **stc Language
Server** output channel and in one error notification with **Open Output** and **Retry**. Fix the
manifest and choose **Retry** (a window reload also works). Unexpected server crashes still restart
automatically up to four times in three minutes. A project without tests may omit `[test]`, omit
`sources`, or set `[test].sources = []`. A non-empty `[test].sources` glob that matches no files
does not prevent the language server from starting; `stc test` and `stc test --list` report
`STC-E-PROJECT-004` for each unmatched test glob. A `[project].sources` glob that matches no files
still fails project load.

### 8.2 Tasks and diagnostics

Run **Tasks: Run Task** and choose an `stc check`, `stc build`, `stc test`, or `stc fmt` task, or
define tasks in `.vscode/tasks.json`:

```json
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "stc: build",
      "type": "stc",
      "command": "build",
      "project": "${workspaceFolder}/stc.toml",
      "problemMatcher": ["$stc"],
      "group": "build"
    }
  ]
}
```

The task type accepts `command` (`check`, `build`, `run`, `test`, or `fmt`), `project`, `args`, and
`release` (for `build` and `run`). Generated tasks run in the manifest directory and pass
`--message-format short --color never`; diagnostic commands also pass
`--diagnostic-paths absolute` so the `$stc` problem matcher can resolve sources in multi-package
projects. The matcher consumes `path:line:column: severity[CODE]: message` lines and does not
reinterpret diagnostics.

`project` and every `args` entry expand `${workspaceFolder}`, `${workspaceFolderBasename}`, and
`${workspaceRoot}`. Any other `${...}` token is left literal and is not reported as an error. The
same expansion applies to all five commands and to the `run` task terminal, so both execution
paths receive the same command line.

### 8.3 Language features

With a project open and `stc` available:

- diagnostics from the compiler in the Problems panel, updated as you type;
- document symbols, go-to-definition, references, and hover for identifiers;
- completion for keywords, visible names, and one level of members after `.`;
- format document through `stc fmt` (one whole-document edit; no range formatting);
- rename (F2) on a root-package declaration in source. The edit covers the declaration, every
  reference, and the resolved `stc.toml`, machine-profile, `VAR_ACCESS`, and `VAR_CONFIG`
  references. The extension writes `stc.renames.toml` only after it has confirmed that every
  edited file contains the expected new text; cancelling, undoing, or a failed write leaves the
  history unchanged and shows a warning with the recovery.

The language server analyzes the root package's production and test sources, so test files resolve
against production declarations. It does not offer semantic tokens, code actions, inlay hints, call
hierarchy, or workspace symbols. The grammar provides syntax highlighting. Rename is
source-initiated: F2 in `stc.toml` or in the machine profile returns no rename, and those values
are edited as references of a source rename. Only the VS Code extension uses the tracked history
handshake; another editor gets the standard history-free method and must record the
`[[rename]]` entry by hand, or the next online download can be cold. Parameters, locals,
configurations, and case-only renames have no history entry in either entry point. A field rename
inside a `FUNCTION_BLOCK` or `CLASS` keeps its history entry but still makes the next online
candidate cold (`STC-E-ONLINE-001`), because the type's `TYPEINFO` descriptor changes shape.

### 8.4 Launch and attach

Add a configuration to `.vscode/launch.json`:

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "type": "stc",
      "request": "launch",
      "name": "stc: Launch",
      "project": "${workspaceFolder}/stc.toml",
      "noBuild": false,
      "release": false,
      "allowWrite": false,
      "allowForce": false
    },
    {
      "type": "stc",
      "request": "attach",
      "name": "stc: Attach",
      "project": "${workspaceFolder}/stc.toml",
      "endpoint": "${workspaceFolder}/target/stc/debug/debug.endpoint.json"
    },
    {
      "type": "stc",
      "request": "attach",
      "name": "stc: Attach remote",
      "project": "${workspaceFolder}/stc.toml",
      "remote": "r2k@ecat",
      "remoteDir": "~/stc-wl12",
      "remoteStc": "/home/r2k/stc-wl12/bin/stc",
      "remoteDeploy": "ask"
    }
  ]
}
```

Launch builds the debug form by default and stops a launch-owned host when the session ends.
Launch is start-gated: a breakpoint set before the session is configured binds before cycle 0, so
a breakpoint on the first executable statement stops before that statement executes. Attach
connects to the endpoint file printed by `stc run --debug-server`, leaves the host running, and
binds breakpoints at the next checkpoint; cycles before the attach are not observed. The Debug
Console reports a client stop, the host's exit code, a trap, a swap or timeout with its code, and a
connection loss. `noBuild` launches existing artifacts; `outDir` selects them; `release` builds the
monitoring-only release form; `sourceRoot` overrides package-root source mapping; `args` and `cwd`
apply to the launched host. Replace the placeholder endpoint path with the printed file.

Add `"test": "<name>"` to a launch configuration to debug one test. The name must match a test
listed by `stc test --list` exactly. The Testing view's **Debug Test** starts this launch
automatically for one test (see §8.5). The launch builds only that test as a hook-bearing harness
under `target/stc/test/debug/<artifact>` (or under `<outDir>/debug/<artifact>`), so the ordinary
`stc test` artifacts stay hook-free and unchanged. Breakpoints in test source bind before the test
body runs, the test POU is the top frame, and the session ends after the test prints its result
line and exits. `test` cannot be combined with `noBuild`, `release`, or attach.

A remote attach (`remote` set) reads the remote session descriptor and source fingerprint and
opens an SSH tunnel; `remoteDir`, `remoteStc`, `remoteSession`, and `remoteDeploy` are optional.
When `remoteDeploy` is `ask`, the extension runs `stc remote status` before starting the session.
If sources drifted it shows **Online change** (only when the host reports a compatible plan),
**Cold download**, or **Cancel**; Cancel starts no session and changes no running host. The Rust
driver performs the chosen deploy. A remote launch starts and stops its own `stc serve`.

Writes are disabled by default. `allowWrite` requests the write scope and enables `setVariable` for
writable live roots (globals, program-instance fields, and live split fields); the host must also
allow `[debug].allow-write`. `allowForce` requests the force
scope and requires `[debug].allow-force`; DAP has no force request, so forcing remains a
`stc debug` operation. Locals require a debug build and a `stc-debug/1.1` host; release builds are
monitoring-only. Use the Debug Console to evaluate live Globals and target paths while the release
host runs; Variables and Watch require a stopped frame and remain unavailable. Target paths use
manifest keys (the sample global `TOTAL` is `SAMPLE::TOTAL`, not `MAIN.TOTAL`). There is one debug
session per host, every online swap ends the session, and local names appear in the frame table's
uppercase normalized spelling. `evaluate` accepts a local name or a manifest target path, not an ST
expression.

The local sample at
[`editors/vscode/samples/workspace`](editors/vscode/samples/workspace/README.md) contains launch and
attach configurations, tasks, a known breakpoint line, and known values, and is the fixture used by
the automated tests.

### 8.5 Running tests from the editor

Open a folder containing `stc.toml`, show the **Testing** view, and expand a file to see its
`{stc:test}` POUs. Each test is placed at its declaration line, including the `{stc:test}`
annotation, so the Test Explorer and editor gutter navigate to it. The extension discovers tests
with `stc test --list --message-format json` and refreshes the list when an `.st` file or
`stc.toml` changes, when the workspace folders or `stc.path` change, with the view's refresh
button, or with **stc: Refresh Tests**.

Run Test on a test runs only that test; Run Test on a file runs all of its tests. The extension
passes `--exact` so the selected test name is a complete-name match, not a substring, and captures
test output. Passed tests show their duration; failed, trapped, and timed-out tests show their
failure messages and the first known source location. Compiler diagnostics and captured output
appear in the Test Results output, and a build failure marks the selected tests errored rather
than reporting invented results.

Debug Test opens the test under the debugger through the same per-test launch described in §8.4.
Because a launch accepts one test name, selecting several tests (or a file) for debugging starts
them one after another, each in its own session. The Debug Console result line reports the test
outcome; debug sessions do not produce Test Explorer pass/fail results. A missing compiler or a
discovery failure is logged to the **stc** output channel. Runs require the same clang and runtime
artifacts as `stc test`. See
[`IDE-Integration.md`](specification/Architecture/IDE-Integration.md) §4.8 for the full contract.

### 8.6 Online change

The extension can drive the same online-change workflow from the editor. Open a project and run
**stc: Start Online Change** from the Command Palette. The extension starts an extension-owned
`stc run --watch --debug-server --debug-endpoint <private per-session path>` child; the status bar
shows `starting`, then `building` when a rebuild starts, then `gen <n> · <8 hex>` for the active
generation and build. Click the status item or run **stc: Show Online Change Status** to open the
**stc Online Change** channel, which prints the CLI's output with the control nonce, control
endpoint, and the runtime's debug endpoint/audit path lines redacted. The runtime debug token is
never read or logged.

Run **stc: Attach Debugger to Online Change** to attach a read-only debug session to the running
watch host. Saving a compatible change rebuilds and activates a new generation at a cycle
boundary; the attached session ends with `STC-E-DEBUG-019` (shown in the Debug Console), and the
extension offers **Attach Debugger** for the new generation. It does not reconnect automatically,
and breakpoints, forces, and watches do not survive the swap.

**stc: Stop Online Change** closes the watch's stdin, which the CLI treats as a graceful stop; the
status returns to idle and the host saves retained state at normal shutdown. If the process does
not exit within five seconds, the extension warns and kills the process tree, and retained state
may not be saved. Closing the window stops an owned watch the same way.

`stc.onlineChange.args` (string array, default empty) appends options such as `--release`,
`--store`, or `--watch-timeout` to the watch. The extension owns the manifest, debug-server,
endpoint, color, control, and quiet flags; a value for one of those is rejected. Only one
online-change session runs per window, and there is no cross-window coordination. The surface is
local only and offers no cold-download, rollback, or migration action; use the CLI for those. See
the [extension README](editors/vscode/README.md) and
[`IDE-Integration.md`](specification/Architecture/IDE-Integration.md) §4.7 for the full contract.

### 8.7 Non-safety

The extension is local only: no telemetry, network requests, remote debugging, or bundled compiler.
Debug and force can stop the cycle and hold outputs indefinitely, and an online change can activate
new logic while the machine is running. `stc` is not a safety PLC and is not responsible for
safety; safety must be provided by a separate safety system and machine design as required by law
and regulation. See
[`specification/Architecture/IDE-Integration.md`](specification/Architecture/IDE-Integration.md)
and [`KNOWN-LIMITATIONS.md`](KNOWN-LIMITATIONS.md) for the complete contract.

## 9. Tests and formatting

```sh
stc test                 # all discovered {stc:test} POUs
stc test --list          # list without building
stc test --exact MyTest  # complete-name match, not a substring
stc test --no-run        # build artifacts only
stc test --timeout 5s
stc test -- --nocapture
```

Each test runs in its own process with a deterministic clock. `STC_ASSERT`
failures, traps, and timeouts are reported per test; `[[test.case]]` can set
cycles, mock-clock, and expected traps. Test-only code never enters production
or release artifacts.

### Coverage

```sh
stc test --coverage                         # write target/stc/coverage/
stc test --coverage --coverage-out out/cov  # custom report directory
stc test --no-coverage                      # override [test] coverage = true
```

Set `[test] coverage = true` in `stc.toml` to make coverage the project
default. `--coverage` forces it on and `--no-coverage` forces it off for one
run; the CLI wins in either direction. `--coverage` is rejected with `--list`,
and `--coverage-out` requires coverage to be enabled. `stc clean` and
`stc clean --test` remove the default `target/stc/coverage` directory; a custom
`--coverage-out` directory is left alone.

`stc test --coverage` instruments the production statements compiled into each
test artifact, runs the same tests with the same statuses and exit codes, then
writes:

- `coverage.json` — the native `stc-coverage/1` report: suite and per-file
  totals, every canonical site with line, column, and summed hit count, and
  the site indices each test covered.
- `lcov.info` — LCOV for editors and `lcov`/`genhtml` tooling; one `DA`
  line per line with a site, with counts summed per line.
- A human summary on stdout after the test summary.

```text
coverage: 3/5 statements (60.0%), 3/5 lines (60.0%)
  src/coverage.st: 3/5 statements (60.0%), 3/5 lines (60.0%)
```

In `coverage.json`, `percent` and `line_percent` are `null` when the
denominator is zero. Reports contain no timings, absolute paths, or host data and are
byte-identical for identical inputs on every supported host. Positional test
filters select the tests that contribute, and the filters are recorded in the
report.

Coverage is statement-based only: there is no branch, condition, or MC/DC
coverage, no fail-under gate, and no HTML report. Timeouts are wall-clock and
include instrumentation overhead, so a test near its limit may need a larger
`--timeout`. Timed-out, crashed, and failed-to-load tests contribute no data;
failed and trapped tests contribute what they executed. Production `PROGRAM`
bodies that a test artifact does not link are outside the measured universe.
Coverage is not correctness evidence. See
[`specification/Architecture/Testing-and-Verification.md`](specification/Architecture/Testing-and-Verification.md)
§2.6, [`KNOWN-LIMITATIONS.md`](KNOWN-LIMITATIONS.md) §9, and
[`examples/coverage/`](examples/coverage/README.md).

`stc fmt` uses one fixed style over the lossless CST, preserves comments,
emits strict semicolons, and supports `--check`. It is idempotent.

## 10. Cross-target smoke tests

`stc build --target=<triple> --emit=ir` (or `obj`) emits target-independent IR
and an object for the requested triple. The release-candidate matrix covers:

```text
x86_64-unknown-linux-gnu       aarch64-unknown-linux-gnu
aarch64_be-unknown-linux-gnu   x86_64-apple-darwin
aarch64-apple-darwin           x86_64-pc-windows-msvc
aarch64-pc-windows-msvc        riscv64-unknown-elf
thumbv7em-none-eabihf
```

Execution, module loading, and frame extraction are native on Windows x86-64
MSVC, Linux x86-64 GNU, Linux AArch64 GNU, macOS ARM64 (`aarch64-apple-darwin`),
and macOS x86-64 (`x86_64-apple-darwin`). `--emit=exe` and `--emit=lib` reject a
non-host target with `STC-E-BUILD-004`. macOS uses the Mach-O
`llvm-dwarfdump` frame path; the full workspace suite and the VS Code extension
smoke run on both macOS architectures.

## 11. Performance

Timing is informational, not a real-time guarantee. Record the host and regime
when comparing numbers:

```sh
cargo test -p stc-driver --test performance -- --ignored --nocapture
cargo test -p stc-driver --test debug_overhead -- --ignored --nocapture
cargo test -p stc-runtime --test switch_latency -- --ignored --nocapture
cargo test -p stc-runtime --release --test modbus_overhead -- --ignored --nocapture
```

Recorded figures are in
[`specification/Verification/Performance.md`](specification/Verification/Performance.md).
The Modbus harness measures the public `latch_inputs`/`publish_outputs` seam with 0, 1, 4, and
8 loopback clients; the method and host-specific results are in
[`specification/Verification/Modbus-Server.md`](specification/Verification/Modbus-Server.md) §11.

`stc run --rt --cycle-time 1ms --rt-metrics rt.jsonl` is an opt-in measurement mode; without
`--rt` the run loop and the generated artifacts are unchanged. On Linux it sets the cycle thread
to `SCHED_FIFO` (default priority 90), optionally pins it with `--rt-cpu`, and paces with absolute
`CLOCK_MONOTONIC` sleeps. On Windows it raises the cycle thread to
`THREAD_PRIORITY_TIME_CRITICAL`, registers it with MMCSS when available, and uses a
high-resolution waitable timer. On macOS it applies a Mach `THREAD_TIME_CONSTRAINT_POLICY` and
falls back to an interactive QoS class when the policy is rejected. `--rt-priority` and `--rt-cpu`
are Linux-only. The JSONL metrics are described in
[specification/Verification/RT-Host.md](specification/Verification/RT-Host.md) §3.4. The mode is
measurement-only and is not a hard-real-time or safety guarantee; macOS is a supported native
host for the advisory soft-scheduling mode (D-249, D-250).

## 12. Troubleshooting

| Symptom | Cause / action |
| --- | --- |
| `STC-E-BUILD-001` “cannot run clang” | Install clang/LLVM or pass `--clang`/`STC_CLANG` |
| `STC-E-BUILD-003` “runtime … was not found” | Use the packaged `bin` + `lib` layout, build `stc-runtime`, or pass `--runtime` |
| `STC-E-BUILD-004` cross-target executable/library | Only IR/object emission is supported off-host; link on the target host |
| `STC-W-MANIFEST-001` / `STC-W-ONLINE-005` cold layout | There is no usable baseline; the build is deployable only as a cold download |
| `STC-E-MANIFEST-002` host ABI minor | The baseline needs a newer ABI than the selected build form; use the matching mode or cold-download |
| `STC-E-DEBUG-017` stack/frame unavailable | The stopped-frame sidecar is missing or unusable (`llvm-readobj`/`readelf` at build time); typed monitoring still works |
| `STC-E-PROJECT-010` debug/release mixed | Use distinct out-dirs for the two forms; one explicit out-dir cannot hold both |
| `STC-E-PROJECT-004` “source glob matched no files” | For `[project].sources`, add a matching production source file; the error fails project load. For `[test].sources`, the project still loads, and `stc test`/`stc test --list` (or a test debug launch) reports the error until a test file matches or the glob is removed |
| “The language server could not start” or an `STC-E-PROJECT-*` error | The project manifest cannot load; the extension does not restart a deterministic configuration error. Fix the `STC-E-PROJECT-*` error shown in the notification or the **stc Language Server** output channel, then choose **Retry** (or reload the window) |
| Version or build-ID mismatch after upgrading | Pre-RC artifacts are not compatible; cold-download or rebuild both sides |

For every diagnostic, `--message-format json` gives a stable object with code,
severity, spans, notes, and related locations.

## 13. Modbus TCP server I/O (v2.0.0)

A project can expose its process image as a Modbus TCP server. Declare a device with
`kind = "modbus-tcp"` and map each declared channel to a Modbus table in the selected machine
profile. The compiler embeds the profile; the runtime opens one listener per `bind` entry and
serves all bind entries from one register bank.

```toml
# stc.toml
[io]
config = "io/bench.toml"

[[io.devices]]
name = "modbus0"
kind = "modbus-tcp"

  [[io.devices.channels]]
  name = "CoilIn0"
  direction = "input"
  width = 1

  [[io.devices.channels]]
  name = "WordOut0"
  direction = "output"
  width = 16

[[io.bindings]]
variable = "Main.CoilIn0"
device = "modbus0"
channel = "CoilIn0"
direction = "input"

[[io.bindings]]
variable = "Main.WordOut0"
device = "modbus0"
channel = "WordOut0"
direction = "output"
```

```toml
# io/bench.toml
[device.modbus0]
bind = ["127.0.0.1:15020", "192.168.1.7:15020"]
unit = 1
max-connections = 8
idle-timeout = "60s"
word-order = "high-first"

# Zero-based: address 0 is the first element of the table. Vendor 00001,
# 10001, 30001, and 40001 are one-based references; subtract one.
[[device.modbus0.channels]]
name = "CoilIn0"
table = "coil"
address = 0

[[device.modbus0.channels]]
name = "WordOut0"
table = "holding-register"
address = 0
```

- Functions: FC1/2/3/4 reads and FC5/6/15/16 writes.
- Coils and discrete inputs carry 1-bit values. Holding and input registers carry 8-, 16-,
  32-, or 64-bit values; 32/64-bit channels occupy 2/4 registers. A boolean in a register is a
  16-bit channel.
- `coil` and `holding-register` accept input (client-writable) and output (PLC-readable)
  channels. `discrete-input` and `input-register` accept output channels only.
- Addresses are zero-based. A 32-bit channel at address 20 covers registers 20 and 21 (vendor
  `40021..40022` on holding registers). A write that only partly covers a wide channel returns
  exception 0x02 and changes nothing.
- `word-order` defaults to `high-first`; `low-first` and per-channel `abcd`/`cdab`/`dcba`/`badc`
  or an integer byte list are supported. `byte = "low"`/`"high"` selects a byte for 8-bit
  register channels.
- Optional status device:

```toml
[[io.devices]]
name = "MBStatus"
kind = "modbus-tcp-status"
config = { layout = 1 }

[[io.devices.channels]]
name = "server0.listening"
direction = "input"
width = 1
```

Run with `stc run -v` to print the `io-backend modbus-tcp` line with `server0.*` listener,
connection, request, read/write, exception, malformed, and last-error diagnostics. A client
write is visible to the PLC at the next input boundary; a PLC output is readable after the
publish boundary. An unbound or read-only write, a partial wide write, and an unmapped range
return exception 0x02.

The server is **non-real-time** and has **no authentication or encryption**. Every
input-mapped coil/register on every bind is writable by any reachable client. Bind only
interfaces you intend to expose, use a trusted/segmented network or host firewall, and never use
Modbus as a safety function or an interpolated motion channel. See the full contract in
[`specification/Architecture/IO-Routing-and-Backends.md`](specification/Architecture/IO-Routing-and-Backends.md)
§4.5, the profile schema in
[`specification/Architecture/CLI-and-Manifest.md`](specification/Architecture/CLI-and-Manifest.md)
§2.2.1, the evidence report in
[`specification/Verification/Modbus-Server.md`](specification/Verification/Modbus-Server.md),
and the runnable fixture in [`tests/modbus/`](tests/modbus/README.md).

## 14. Support matrix

| Host | Triple | Package | Compile | Run/model | Debug | Online change |
| --- | --- | --- | --- | --- | --- | --- |
| Windows 11 x86-64 MSVC | `x86_64-pc-windows-msvc` | `stc-2.0.0-x86_64-pc-windows-msvc` | yes | yes | yes | yes |
| Linux x86-64 GNU | `x86_64-unknown-linux-gnu` | `stc-2.0.0-x86_64-unknown-linux-gnu` | yes | yes | yes | yes |
| Linux AArch64 GNU | `aarch64-unknown-linux-gnu` | `stc-2.0.0-aarch64-unknown-linux-gnu` | yes | yes | yes | yes |
| macOS ARM64 (Apple silicon) | `aarch64-apple-darwin` | `stc-2.0.0-aarch64-apple-darwin` | yes | yes | yes | yes |
| macOS x86-64 (Intel) | `x86_64-apple-darwin` | `stc-2.0.0-x86_64-apple-darwin` | yes | yes | yes | yes |
| Other targets | any clang triple | none | IR/object emission only | no | no | no |

The five native hosts are supported (D-200, D-250). Windows and both Linux
hosts are supported from M8; both macOS hosts are supported from WL-14
(v3.0.0) at Windows parity for the Windows-available surface. Windows ARM64,
32-bit, and big-endian hosts are not supported. CI runs the full workspace
suite natively on all five hosts. The separate UI, conformance, execution,
projects, online, debug, and cross-target jobs run on Windows and both Linux
hosts; their test binaries are also part of the macOS full-workspace run. The
package job runs on all five hosts, and the VS Code extension smoke runs on
Linux and both macOS hosts.

Two exclusions are accepted on macOS: the EtherCAT backend and IgH shim are
Linux-only, and PREEMPT_RT-style hard real time is not available. `stc run
--rt` on macOS is the advisory soft-scheduling mode (D-249); it is not a
hard-real-time or safety guarantee. See
[`KNOWN-LIMITATIONS.md`](KNOWN-LIMITATIONS.md#macos-support-wl-14) for the
macOS packaging, frame-path, and scheduling limits, and §11 for the per-host
`--rt` behavior.

## 15. Safety

See [`NON-SAFETY.md`](NON-SAFETY.md). In short: `stc` can control real
machinery, but it is not a safety PLC; safety must be provided by a separate
safety system and machine design as required by law and regulation. Debug and
forcing can hold outputs indefinitely and are not safety functions.
