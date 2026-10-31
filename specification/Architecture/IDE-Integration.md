# IDE Integration

Status: WL-1.1 phase 5 specification (2026-10-30); the VS Code extension, `stc lsp`, and `stc dap`
are implemented and tested; F-20 adds the extension-owned online-change surface (2026-10-31)  
Profile: `stc-2013-oop-utf8-v1`  
Scope: local VS Code language support, tasks, and debugging; not a language, compiler, artifact,
module-ABI, or runtime-protocol feature beyond the additive `stc-debug/1.1` revision

This document fixes the editor-facing contract for the local VS Code extension. The extension is a
client: diagnostics, types, navigation, formatting, and debug values come from `stc` crates and the
runtime. It contains no compiler or debugger logic and adds no second diagnostics engine,
formatter, or debugger protocol. The debug wire contract remains
[Debug and Monitoring](Debug-and-Monitoring.md) §13–§18; the command surface is
[CLI and Manifest](CLI-and-Manifest.md) §1.

Related: [Architecture](Architecture.md), [Debug and Monitoring](Debug-and-Monitoring.md),
[CLI and Manifest](CLI-and-Manifest.md), [Testing and Verification](Testing-and-Verification.md),
[User Guide](../../USER-GUIDE.md), [Known Limitations](../../KNOWN-LIMITATIONS.md).

## 1. Scope, stance, and non-goals

The integration is local only. It opens no network listener, sends no telemetry, performs no
marketplace or update check, and never receives remote source or debug traffic. The language server
and debug adapter are local processes that use standard input and output; the runtime debug endpoint
remains the authenticated local endpoint of Debug and Monitoring §13. The online-change surface
drives the existing `stc run --watch` CLI as an extension-owned child and reads that same local
runtime endpoint file; it adds no wire protocol or transport (§4.7).

**Non-safety statement.** `stc` can control real machinery, but it is not a safety PLC and is not
responsible for safety. Safety must be provided by a separate safety system and machine design as
required by law and regulation. Debug sessions can stop the cycle and hold outputs at their last or
forced values indefinitely. Debugging, monitoring, forcing, and the editor integration are
availability and operability features, not safety functions.

Out of scope for v2.0.0:

- remote or network IDE access, remote source mapping, and remote attachments (WL-1.2);
- semantic tokens, rename, code actions, inlay hints, range formatting, call hierarchy, workspace
  symbols, and incremental analysis;
- conditional, hit-count, log, function, data, and instruction breakpoints; exception breakpoints;
  reverse debugging; multi-thread debugging; `stopOnEntry`; restart; and ST expression evaluation;
- marketplace publishing, code signing, telemetry, and extension-bundled compiler binaries;
- a second formatter, type checker, diagnostics engine, debugger protocol, online-change protocol,
  deploy client, or control-channel client; the editor surface presents the existing CLI workflow
  and the existing runtime endpoint file instead;
- cold-download confirmation, deploy/rollback UI, migration UI, remote deployment, and
  cross-window online-change coordination.

## 2. Processes, transports, and discovery

| Process | Transport | Owner |
| --- | --- | --- |
| `stc lsp [--manifest-path <path>]` | LSP JSON-RPC with `Content-Length` framing over stdio | The extension's language clients |
| `stc dap [--manifest-path <path>]` | DAP JSON with `Content-Length` framing over stdio | The extension's `stc` debug adapter |
| `stc run --watch ... [--debug-server --debug-endpoint <path>]` | stderr `[stc]` status lines; runtime debug endpoint JSON file (format 1) | The extension's online-change session (one per window) |

- Standard output carries protocol messages only; logs and build output go to standard error. The
  language server never writes diagnostics text to standard output outside a JSON-RPC frame. The
  debug adapter never writes a DAP message outside a frame.
- `stc lsp` serves one project. The extension starts one language client per unique `stc.toml`
  found from each workspace folder, searching the folder and its ancestors; two folders that
  resolve to the same manifest share one client. The server discovers the manifest from
  `--manifest-path` first, then the initialize `rootUri`, then `workspaceFolders`, then the process
  working directory. A missing manifest fails `initialize` with a readable internal error rather
  than starting without a project.
- The language client watches `**/*.st` and `**/stc.toml`; a watched change reloads the project
  graph and reanalyzes. `stc.path` changes restart the clients.
- `stc dap` receives no project argument from the extension. It starts in the workspace folder and
  each launch or attach configuration carries `project`; `--manifest-path` is an optional fallback
  for clients that omit it.
- The online-change session runs `stc run --watch --manifest-path <project>/stc.toml --debug-server
  --debug-endpoint <storage>/oc/<8 hex>/d.json --color never <stc.onlineChange.args>`. It parses
  the CLI's `[stc]` stderr status lines and polls the runtime's user-only endpoint file; it does
  not connect to the control channel, does not parse `-v` output for the control nonce, and does
  not read the endpoint's `token`, `endpoint`, or `audit_log` fields. A unique per-session endpoint
  path is required because the watch path does not prepare or rotate a debug endpoint.
- Compiler discovery uses the `stc.path` setting, whose default is `stc`. An absolute or
  path-containing value is used directly; otherwise the value is resolved on `PATH`, including
  `PATHEXT` extensions on Windows. A missing executable is logged to the `stc` output channel and
  the language server is not started; starting a debug session reports an explicit error. The
  extension does not bundle or download a compiler.
- Distribution is a VSIX only in v2.0.0; the extension is not published to the VS Code
  Marketplace. The engine floor is `^1.82.0`, the pinned `vscode-languageclient` 9.0.1 minimum.
  The language id is `st`, the TextMate scope is `source.st`, and `.st` files are the only
  language association.

### 2.1 Language contribution and highlighting

Highlighting is TextMate-only. The extension contributes one grammar for `st` with the root scope
`source.st`; `stc lsp` publishes no semantic tokens, and F-1 adds no semantic-token capability.
The grammar uses standard TextMate scope roots with a `.st` suffix so ordinary themes match them:

| Role | Scope |
| --- | --- |
| Block/line/documentation comment | `comment.block.st`, `comment.line.double-slash.st`, `comment.line.documentation.st` |
| String, escape, character | `string.quoted.single.st`, `string.quoted.double.st`, `constant.character.escape.st`, `constant.character.st` |
| Number, duration, date/time | `constant.numeric.integer.st`, `.real.st`, `.based.st`, `.typed.st`, `.duration.st`, `.date.st`, `.time.st`, `.datetime.st` |
| Declaration keyword | `keyword.declaration.st` |
| Control keyword | `keyword.control.conditional.st`, `keyword.control.loop.st`, `keyword.control.flow.st` |
| Modifier | `storage.modifier.st` |
| Operator | `keyword.operator.st`, `keyword.operator.type.st` |
| Language constant | `constant.language.st` |
| Primitive type | `support.type.primitives.st` |
| Function/method/program declaration name | `entity.name.function.st` |
| Type/function-block/class/interface/type-alias name | `entity.name.type.st` |
| Namespace/configuration/resource name | `entity.name.namespace.st` |
| Extended/implemented type name | `entity.other.inherited-class.st` |
| Variable | `variable.name.st` |
| Parameter | `variable.parameter.st` |
| Member/struct field | `variable.other.member.st` |
| Direct variable | `variable.other.constant.st` |
| Enum member | `variable.other.enummember.st` |
| `THIS`/`SUPER` | `variable.language.st` |
| Built-in function | `support.function.st` |
| Standard function block | `support.class.st` |
| Pragma region/name/braces | `meta.pragma.st`, `keyword.other.pragma.st`, `punctuation.definition.pragma.begin/end.st` |

Specific captures (comments, pragmas, strings, typed literals, direct variables, declaration
lines and regions) take precedence over generic keyword, punctuation, and identifier-fallback
rules. The scope names are a user-facing theme contract; renaming one is a breaking theme change.

Because the classification is lexical, declarations and uses are not resolved. A user function or
function-block call name and a named argument use the variable fallback; only names in the prelude
table receive `support.function.st`/`support.class.st`. A type reference is recognized after `:`,
`ARRAY ... OF`, `REF_TO`, `POINTER TO`, `EXTENDS`, and `IMPLEMENTS`.

The extension contributes two optional token themes, `stc Dark` (`uiTheme: "vs-dark"`) and
`stc Light` (`uiTheme: "vs"`), both with `semanticHighlighting: false`. They define the editor
background, foreground, line number, selection, and cursor colors; workbench chrome follows the
UI theme. The extension does not set `workbench.colorTheme`, so the active theme remains the
user's choice. The standard roots also color under Dark Modern/Light Modern, Dark+/Light+, and the
One Dark family. Built-in themes may render the pragma name as a generic keyword and direct
variables as ordinary variables; only the shipped themes guarantee distinct colors for all nine
F-1 acceptance categories. High-contrast themes are not a target.

## 3. Language-server contract

### 3.1 Documents and analysis

- Synchronization is full (`textDocumentSync.change = 1`). `didOpen`, `didChange`, `didSave`, and
  `didClose` are handled; `didSave` carries no text (`includeText = false`). Only the last
  `contentChanges` entry is used.
- Open documents come from the client's buffer; closed documents come from disk. `didClose`
  restores the disk text. Changes are parsed immediately and analyzed after a 200 ms debounce that
  coalesces rapid edits into one analysis.
- The snapshot includes the root package's production sources and its test sources, so editor
  behavior matches `stc test` and test files resolve against production declarations. Dependency
  packages are loaded through the project graph.
- Parse diagnostics are published immediately for the edited file; the semantic analysis replaces
  them with the full project diagnostics when it completes. Analysis is whole-project on every run;
  incremental analysis is not offered. A failed project reload keeps the previous analysis and logs
  the failure; it does not publish invented diagnostics.
- `workspace/didChangeWatchedFiles` for an `.st` file or `stc.toml` reloads the graph and
  reanalyzes; the extension sends these events from its file watchers.
- The server advertises `positionEncoding: utf-16`. All positions are zero-based LSP UTF-16 code
  units and all URIs are `file:` URIs. Byte spans are converted through `stc-span` to one-based
  line/scalar columns and then widened to UTF-16 in one conversion layer; non-ASCII comments and
  strings are covered by tests.

### 3.2 Capability map

| Capability | Status | Behavior |
| --- | --- | --- |
| `textDocument/publishDiagnostics` | Supported | Push diagnostics only; parse first, then semantic; warnings and errors; stable `STC-*` codes |
| `textDocument/documentSymbol` | Supported | Hierarchical POU/type/variable symbols from the typed AST |
| `textDocument/definition` | Supported | Identifier to declaration through `resolutions`; an enum value resolves to its value span |
| `textDocument/references` | Supported | All resolutions for the same identity; the declaration is included only when requested |
| `textDocument/hover` | Supported, identifiers only | Declared type, declaration kind, and owner; expression typing is not offered |
| `textDocument/completion` | Supported, bounded | Keywords, visible names, and one-level member completion after `.`; trigger characters are `.` and `:`; no auto-import |
| `textDocument/formatting` | Supported | Whole-document edit from `stc-fmt`; an already formatted document returns `[]`; syntax errors that prevent formatting return `null` |
| `textDocument/didOpen`, `didChange`, `didSave`, `didClose` | Supported | Full synchronization as described in §3.1 |
| `workspace/didChangeWatchedFiles` | Supported | Reloads `.st` and `stc.toml` changes |

Diagnostics carry a range, severity (`1` error, `2` warning), the stable `STC-*` code, the source
`stc`, the message, and `relatedInformation` built from compiler labels. The extension does not
reinterpret or re-rank them.

### 3.3 Closed unsupported list

No capability is advertised for, and no support is implemented for, semantic tokens, rename, code
actions, inlay hints, range formatting, call hierarchy, workspace symbols, or pull diagnostics.
Unsupported requests receive a JSON-RPC method-not-found error; they are not silently accepted.
Highlighting comes from the TextMate grammar, not from semantic tokens. Formatting never edits a
range; it returns one whole-document edit or none.

## 4. Debug-adapter contract

### 4.1 Session startup and configuration

The extension launches `stc dap` with `DebugAdapterExecutable`; the adapter emits the DAP
`initialized` event once after the `initialize` response. A launch or attach configuration and
`configurationDone` are both required before the adapter starts the host. Breakpoints received
before that point are buffered, resolved against the project graph, and re-reported with a
`breakpoint`/`changed` event when the source mapping becomes available. The adapter applies the
breakpoints as soon as the session is established; cycles that execute before the adapter connects
are not observed and are not replayed, so a debugger can rely on the stop location but must not
assume it is the first program cycle.

Launch builds the project in the debug form by default and then starts the launcher with a local
debug endpoint and control endpoint. `noBuild` uses the existing artifacts; `outDir` overrides their
directory. Attach connects to an existing endpoint file and does not build or launch.

| Configuration key | Requests | Meaning |
| --- | --- | --- |
| `project` | launch, attach | `stc.toml`, its directory, or a project directory; the extension defaults it to `${workspaceFolder}/stc.toml` |
| `noBuild` | launch, attach | Launch existing artifacts without building; default `false` |
| `outDir` | launch, attach | Artifact directory override, primarily for `noBuild` |
| `release` | launch, attach | Build in release mode; monitoring only |
| `allowWrite` | launch, attach | Request the `write` scope and enable `setVariable`; the host must also allow `[debug].allow-write` |
| `allowForce` | launch, attach | Request the `force` scope; the host must also allow `[debug].allow-force`. DAP has no force request, so forcing remains a `stc debug` operation |
| `sourceRoot` | launch, attach | Override the source-path root instead of resolving package roots |
| `args` | launch, attach | Extra arguments appended to the launched host |
| `cwd` | launch, attach | Working directory for the launched host |
| `endpoint` | attach | Endpoint file printed by `stc run --debug-server`; required for attach |

`stopOnEntry` is rejected explicitly. Every session start emits the runtime's non-safety notice as
an `output` event, followed by one line naming the negotiated protocol and whether the Locals scope
is available.

### 4.2 Capability map

The adapter advertises this closed set:

| DAP capability | Value | Meaning |
| --- | --- | --- |
| `supportsConfigurationDoneRequest` | `true` | `configurationDone` completes launch setup |
| `supportsSetVariable` | `true` | Implemented for live global roots when write is granted |
| `supportsTerminateRequest` | `true` | `terminate` ends the session |
| `supportTerminateDebuggee` | `true` | A launch-owned host is stopped on teardown |
| `supportsEvaluateForHovers` | `true` | Hover evaluation uses the same target/local lookup |
| `supportsConditionalBreakpoints`, `supportsHitConditionalBreakpoints`, `supportsLogPoints` | `false` | Source breakpoints only |
| `supportsFunctionBreakpoints`, `supportsDataBreakpoints`, `supportsInstructionBreakpoints` | `false` | Not implemented |
| `supportsRestartRequest`, `supportsStepBack` | `false` | No restart or reverse debugging |
| `supportsSetExpression` | `false` | No expression assignment |
| `supportsDelayedStackTraceLoading`, `supportsLoadedSourcesRequest` | `false` | Stack and sources are returned directly |
| `supportsExceptionInfoRequest`, `supportsExceptionOptions` | `false` | Traps follow runtime session teardown |
| `exceptionBreakpointFilters` | `[]` | No exception breakpoints |

Supported requests are `initialize`, `launch`, `attach`, `configurationDone`, `setBreakpoints`,
`setExceptionBreakpoints` (accepting no filters), `threads` (one thread, id `1`, name `cycle`),
`stackTrace`, `scopes`, `variables`, `setVariable`, `evaluate`, `continue`, `pause`, `next`,
`stepIn`, `stepOut`, `terminate`, and `disconnect`. Other requests fail explicitly. There is no
multi-thread, restart, conditional-breakpoint, or expression support.

### 4.3 Session protocol and Locals

- The adapter starts `stc-debug/1.1` and falls back to `stc-debug/1` once when the host answers
  `STC-E-DEBUG-002`. The negotiated version is used for the whole connection. The endpoint file's
  `protocol` field stays `stc-debug/1` and does not advertise the revision; negotiation is a
  first-request wire decision (Debug and Monitoring §13.1–§13.2).
- The Locals scope is offered only when the session negotiated `stc-debug/1.1` and the host reports
  `capabilities.stepping = true` with stopped-frame information. A `stc-debug/1` host has no Locals
  scope even when the build carries frame metadata. Globals is always offered while stopped.
- Locals lists a frame's names, canonical types, and live/available state from `frame.locals`;
  values are read one by one with `frame.read`. Names are the frame table's normalized uppercase
  spellings; the source spelling is not retained by the debug metadata.
- Globals groups manifest roots by program instance and exposes top-level global and direct roots.
  It is sourced from `manifest.get` and read with `var.read`. A program instance is shown as a
  container whose fields are its manifest roots.
- Variable expansion uses the typed value's manifest-assisted `view`; aggregates and strings expand
  without further runtime requests. Each scope container is capped at 256 children with a
  truncation entry; values contain no addresses.
- `setVariable` maps to `var.write` only for writable live roots when `allowWrite` is enabled and
  the host granted the `write` scope: global roots, program-instance fields, and live split-field
  children that carry their own runtime root. Stopped-frame locals and contiguous snapshot views
  are read-only and are rejected.
- `evaluate` accepts a stopped-frame local name or a manifest target path. It does not parse ST
  expressions; `1 + 2`, conditions, and arbitrary field expressions fail with an explicit message.

### 4.4 Source mapping

Compiler and protocol source paths are package-relative and `/`-separated. The adapter resolves a
protocol path against the owning package's root directory from the project graph; the root package
wins when relative paths collide. `sourceRoot` overrides the graph resolution. Comparison
normalizes `\` to `/`, strips Windows verbatim/device prefixes, and folds ASCII case on Windows. DAP
`Source` objects carry absolute host paths. A breakpoint on a file that is not a project source is
reported unbound rather than mapped to a guessed path.

### 4.5 Events and teardown

Runtime events map to DAP as follows:

| Runtime event | DAP event |
| --- | --- |
| `stopped` | `stopped` with `threadId: 1`, `reason` (`breakpoint`, `step`, `pause`), `allThreadsStopped: true`, and a `description` of `file:line` |
| `resumed` | `continued` with `threadId: 1`, unless the resume response already emitted it |
| `session-ended` | `terminated`; the reason and optional `STC-E-DEBUG-*` code are written to the debug console |
| Launcher stdout/stderr | `output` with the corresponding category |
| Session or host exit | `exited` when a launch-owned host reports an exit code, then `terminated` |

Breakpoint resolution changes produce `breakpoint` events with `reason: "changed"`. `disconnect`
stops the runtime session and exits the adapter; `terminate` ends the session but leaves the adapter
to finish its protocol loop. A launch-owned host is shut down through the control endpoint; an
attached host is left running. An online swap, timeout, or target exit ends the session with no
migration of breakpoints, forces, or watches. There is exactly one debug session per host and one
adapter session per launch.

### 4.6 Honest limits

- One debug session per host; a second session fails until the first ends.
- Every online swap ends the session; reconnect and re-resolve.
- Release and hook-free builds are monitoring-only: no breakpoints, stepping, or Locals.
- Local names are uppercase normalized spellings from the stopped-frame metadata.
- Caller frames and optimized-out functions report unavailable information honestly.
- `evaluate` does not parse ST expressions.
- No remote transport, TLS, or authentication beyond the local endpoint token and user-only
  endpoint file.
- Debug write and force can hold outputs indefinitely; they are opt-in and never presented as
  safety functions.

### 4.7 Online-change surface

The extension starts, observes, and stops the existing `stc run --watch` workflow. It adds no
online-change implementation, deploy/control client, protocol, or debugger. One session per editor
window is owned by `editors/vscode/src/onlineChange/session.ts`; the pure state machine, status-line
classification, endpoint parsing, redaction, and formatting live in
`editors/vscode/src/onlineChange/state.ts`, which does not import `vscode`.

**Commands and status item.** The palette contributes four commands in the `stc` category:

| Command | Behavior |
| --- | --- |
| `stc.startOnlineChange` | Resolve the project and spawn the extension-owned watch. Refused while a session is running; when an `stc` debug session is active it warns, because a second local host will run. |
| `stc.stopOnlineChange` | Close the child's stdin and wait; fall back to a process-tree kill. Gated by `stc.onlineChangeActive`. |
| `stc.showOnlineChangeStatus` | Reveal the `stc Online Change` channel and return the read-only state snapshot used by the smoke. |
| `stc.attachOnlineDebugger` | Attach the `stc` debug adapter to the running watch host. Gated by `stc.onlineChangeAttachable`. |

There is no default keybinding. One status-bar item shows `starting`, `building`,
`gen <n> · <8 hex>` while active, `build failed; gen <n> kept` when a candidate is rejected, and
`stopped (<reason>)` after an unexpected end; it is hidden while idle and after a user stop. Its
tooltip names the project and shows the full generation/build, mode, host pid, cold flag, last
error, and the one-line non-safety notice.

**Child process and configuration.** The invocation is:

```text
stc run --watch --manifest-path <project>/stc.toml
    --debug-server --debug-endpoint <storage>/oc/<8 hex>/d.json
    --color never <stc.onlineChange.args>
```

`cwd` is the project directory. All three stdio streams are pipes; on POSIX the child is
`detached` so the fallback can signal its process group, and on Windows it uses `windowsHide`.
`stc.onlineChange.args` (string array, resource scope, default empty) appends run options such as
`--release`, `--store`, `--watch-timeout`, or `-v`; the extension-owned flags (`--manifest-path`,
`--debug-server`, `--debug-endpoint`, `--color`, `--control`, `--control-nonce`, `-q`, `--quiet`,
`--help`, `--version`) are rejected with a message naming the flag. The endpoint path is internal
and not configurable. Project selection is the active editor's nearest ancestor `stc.toml` inside a
workspace folder, otherwise a single discovered project, otherwise a QuickPick; no project is an
error and nothing starts.

**Status parsing.** stderr lines classify into `starting` (`online baseline`, `cold layout: no
active baseline`), `watching`, `building` (`stale baseline; rebuilding`), `activating` (`deployed
build <hex>`), and a kept-generation failure (`keeping generation <hex>; candidate was rejected`,
`...; deploy failed`, `build failed; keeping generation <hex>`). Terminal lines map `initial build
failed with status <n>` to `failed`, `host exited with status <n>` to `host-exited`, `watch
interrupted` to `interrupted`, `watch timeout reached` to `timeout`, and `watch stopped` to `user`.
A child exit without a recognized line is `failed` or `forced`. Unknown lines are logged and do not
change state. The "keeping generation" token is a build id, not the numeric generation.

**Endpoint polling and secrets.** Every 750 ms while the child is alive, and immediately after a
`deployed build` line, the extension reads the per-session endpoint file: a regular file of at most
64 KiB, `format === 1`, protocol beginning `stc-debug/1`, with a safe integer `generation`, a
non-empty `build_id`/`mode`, and a positive `pid`. Only those four fields are retained; `token`,
`endpoint`, and `audit_log` are discarded immediately and never logged, copied, or written. The
file is authoritative for the numeric generation and build. If a `deployed build` line is not
confirmed by the endpoint within two seconds, the status shows the new build id with the last known
generation and the channel logs the discrepancy honestly. A missing or unparseable file while the
child is alive keeps the last known state; it is never reported as an ended session.

Every channel line passes redaction: a `^[stc] control nonce` line is dropped, lines containing
`STC_CONTROL_NONCE` or `--control-nonce` are refused, the control endpoint in the `watching for
changes` line is replaced with `<hidden>`, and the runtime's own `stc: debug endpoint <path>` and
`stc: debug audit <path>` lines are replaced with `<hidden>`. The status item, tooltip,
notifications, and snapshot contain no nonce, token, control endpoint, or debug endpoint/audit
path.

**Attach and swap teardown.** `stc.attachOnlineDebugger` requires a running watch, waits up to ten
seconds for the endpoint file, then starts an attach session with `project` and `endpoint` and
default read-only scopes, recording the generation observed at attach time. On
`onDidTerminateDebugSession`, if the tracked session ended while the watch is running, the user did
not request the stop, the extension is not deactivating, and the endpoint's generation differs from
the recorded one, the extension logs the new generation and shows one information message: "The
online change ended the debug session (`STC-E-DEBUG-019` is in the Debug Console). Re-attach to
debug the new generation." with **Attach Debugger** and **Show Output** actions. The Debug Console
line and the DAP `terminated` event remain authoritative. An unchanged generation is never reported
as a swap. There is no automatic reconnect and no breakpoint, force, or watch replay (D-175).

**Stop and deactivate.** `stc.stopOnlineChange` calls `stdin.end()` and waits up to five seconds. If
the child has not exited, the extension warns that normal shutdown may not have run, so retained
state may not be saved, then kills the process tree: `taskkill /pid <pid> /t /f` on Windows, SIGINT
then a process-group SIGKILL on POSIX. A forced stop is recorded as such and suppresses the swap
notification. `deactivate` performs the same stop with a two-second graceful wait so a window
close or reload does not leave an owned child. A clean stdin-close stop exits 0 and prints `[stc]
watch stopped` (CLI and Manifest §1.7).

**Limits.** One online-change session per window; no cross-window coordination, so two windows can
start two watches on one project (the CLI does not lock a store). The surface offers no
cold-download, rollback, migration, write, or force action, and it is local only. A per-session
directory under extension storage remains as local audit evidence after a stop; the runtime removes
the endpoint file, and the audit log is kept. Status-bar and channel text is presentation only;
activation and stop are not safety functions.

## 5. Packaging and distribution

The extension lives at `editors/vscode/`. It is TypeScript bundled by esbuild into
`dist/extension.js`, with a committed `package-lock.json` and pinned `vscode-languageclient` 9.0.1,
`@types/vscode` 1.82.0, esbuild, `@vscode/vsce`, and the grammar test tooling. `npm run package`
runs `vsce package --no-dependencies` and writes a `.vsix` next to `package.json`. The VSIX is
built in CI and attached to the release; no marketplace publication occurs in v2.0.0.

The VSIX contains the manifest, bundled extension, grammar, language configuration, the two theme
files under `themes/`, and README. It excludes TypeScript sources, tests, build scripts, the
lockfile, and the sample workspace (`samples/**` in `.vscodeignore`). The compiler and runtime are
separate release artifacts; the extension resolves `stc` from `stc.path` or `PATH` and never
bundles it.

## 6. Verification

- Rust LSP integration tests spawn the real binary, speak framed JSON-RPC over stdio, and cover
  initialization, diagnostics on a clean and a malformed project, document symbols, definition,
  references, hover, completion, formatting, UTF-16 positions, watched-file reload, shutdown, and
  exit. They need no clang and run in the workspace test matrix.
- Rust DAP integration tests spawn `stc dap`, build a temporary copy of the sample workspace with
  clang, and cover launch with a first-statement breakpoint, known locals and globals, stepping,
  evaluate, write gating, attach, no-build, and clean teardown. They run through the debug suite
  and require clang there.
- The extension checks are `npm ci`, `npm run typecheck`, `npm test` (problem-matcher unit tests,
  theme JSON tests, online-change state tests, and TextMate grammar fixtures), and `npm run
  package`. `test/online-change.test.mjs` bundles the pure state module and covers every status
  line, the stopped reasons, endpoint parsing and field whitelisting (including wrong
  format/protocol, oversized and non-regular files, and malformed fields), redaction, and status
  formatting. The theme test checks the contributed themes, required editor colors and token
  rules, distinct acceptance categories, and the 4.5:1 contrast target. The Rust tests compare the
  grammar against the lexer keyword and
  punctuation tables (`grammar_covers_lexer_tables`), check every primitive type name against
  `support.type.primitives.st` (`primitive_type_keywords_use_primitive_scope`), and check the
  `stc-sema` prelude function and function-block lists against the built-in scopes
  (`prelude_names_are_scoped_in_the_vscode_grammar`).
- The manual highlighting check uses **Developer: Inspect Editor Tokens and Scopes** on the sample
  workspace and the grammar fixtures, in Dark Modern, Light Modern, `stc Dark`, and `stc Light`,
  optionally spot-checking One Dark. It records the rendered color and scope stack for the
  declaration, type, variable/parameter/member, keyword, comment, string, number, pragma,
  direct-variable, language-constant, and built-in tokens in the release checklist.
- The phase 6 CI extension job installs the VSIX in a headless `@vscode/test-electron` session,
  activates the extension on the sample workspace, and exercises highlighting, diagnostics,
  definition, completion, formatting, a task, and a launch debug session. The smoke then starts the
  online-change command, waits for a numeric generation and build id, attaches the debugger, saves
  a layout-preserving literal edit, waits for the attached session to terminate on the swap and for
  a higher generation and different build id, and stops the watch; it restores the source and stops
  any running child in a `finally`. The job and its evidence are part of the release checklist.
- The release checklist records a manual pass on the Windows development host and at least one
  Linux host: install the VSIX, open the sample workspace, confirm the language features and
  tasks, run one launch and one attach session, and run the online-change start/attach/swap/stop
  walkthrough while inspecting the channel and logs for control metadata.

## 7. Related documents

- [Debug and Monitoring](Debug-and-Monitoring.md) — `stc-debug/1` and `stc-debug/1.1`, operations,
  limits, and the stopped-frame sidecar.
- [CLI and Manifest](CLI-and-Manifest.md) §1 — `stc lsp`, `stc dap`, and `--diagnostic-paths`.
- [Testing and Verification](Testing-and-Verification.md) — CI matrices and the headless smoke job.
- [User Guide](../../USER-GUIDE.md) — installation, settings, tasks, and launch/attach usage.
- [Known Limitations](../../KNOWN-LIMITATIONS.md) — the consolidated editor limits.
