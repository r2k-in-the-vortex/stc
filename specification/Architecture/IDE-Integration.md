# IDE Integration

Status: WL-1.1 phase 5 specification (2026-10-30); the VS Code extension, `stc lsp`, and `stc dap`
are implemented and tested; F-20 adds the extension-owned online-change surface (2026-10-31);
WL-18 adds language-server rename and the tracked history handshake (2026-11-12)
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
- semantic tokens, code actions, inlay hints, range formatting, call hierarchy, workspace
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
| `stc test --list ...` / `stc test ... --message-format json` | Line-delimited JSON on stdout; test-host output with `-- --nocapture` | The extension's Testing API session (one-shot child per discovery or project run) |

- Standard output carries protocol messages only; logs and build output go to standard error. The
  language server never writes diagnostics text to standard output outside a JSON-RPC frame. The
  debug adapter never writes a DAP message outside a frame.
- `stc lsp` serves one project. The extension starts one language client per unique `stc.toml`
  found from each workspace folder, searching the folder and its ancestors; two folders that
  resolve to the same manifest share one client. The server discovers the manifest from
  `--manifest-path` first, then the initialize `rootUri`, then `workspaceFolders`, then the process
  working directory. A missing manifest fails `initialize` with a readable internal error rather
  than starting without a project.
- Initialization is all-or-nothing. An invalid or unloadable manifest fails `initialize` and the
  extension classifies the failure as a deterministic configuration error: it does not auto-restart
  that client, writes the `STC-E-PROJECT-*` detail to the **stc Language Server** channel, and
  shows one error notification with **Open Output** and **Retry**. **Retry** recreates only that
  project's client, so fixing `stc.toml` recovers the session without a window reload. A connection
  that closes for any other reason is treated as a crash and restarted up to four times within
  three minutes, after which the client stops and reports the crash; this crash policy is separate
  from the configuration-error policy. A `stc.path` change or a window reload starts fresh clients.
  Degraded initialization that publishes manifest diagnostics and recovers automatically is not
  implemented.
- The language client watches `**/*.st`, `**/stc.toml`, and the selected machine profile; a
  watched change reloads the project graph and reanalyzes. `stc.path` changes restart the clients.
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
- The Testing API session uses the same per-workspace-folder ancestor search as the language
  client to find one `stc.toml` per project. Discovery runs `stc test --list --message-format
  json`; a run uses `stc test --message-format json --exact <name>... -- --nocapture` so selection
  is exact and test-host output is captured. These are one-shot children, not servers.
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
user's choice.

To make ST tokens distinct without selecting a contributed theme, the extension contributes
`editor.tokenColorCustomizations` defaults through `contributes.configurationDefaults`, keyed
`[Default Dark Modern]`, `[Default Light Modern]`, `[Dark Modern]`, `[Light Modern]`,
`[Dark 2026]`, and `[Light 2026]`. The 1.82 IDs are the engines-floor targets; the other keys
cover the renamed built-in themes and the current 2026 defaults, because VS Code changes the
theme IDs without migrating per-theme customization keys. Each key carries `textMateRules`
with the shipped palette (dark on `#1F1F1F` for the modern themes and `#121314` for Dark 2026;
light on `#FFFFFF`), and every selector ends in `.st`, so no other language is affected. The nine F-1 acceptance categories stay distinct and at
or above the 4.5:1 contrast target. The overlay is opt-out: a user or workspace setting that sets
the matching theme key's `textMateRules` to `[]` replaces the contributed array under VS Code's
configuration merge (objects merge, arrays replace); an empty customization object does not
disable the overlay. The standard roots also color under Dark+/Light+ and the One Dark family;
under other built-in themes the pragma name may render as a generic keyword and direct variables
as ordinary variables. High-contrast themes are not a target.

### 2.2 Task provider

The extension contributes the `stc` task type with the `check`, `build`, `run`, `test`, and
`fmt` commands. Every task runs in the manifest directory. Generated arguments start with the
command and `--message-format short --color never`; `check`, `build`, `run`, and `test` also
receive `--diagnostic-paths absolute` for the `$stc` problem matcher, `build` and `run` receive
`--release` when the task sets it, and the task's configured `args` are appended after the
generated arguments. `run` uses a custom pseudoterminal so terminating the task stops the driver
and the host it launched; the other commands use `ShellExecution`. Both execution paths receive
the same expanded argument list.

`project` and every `args` entry expand `${workspaceFolder}`, `${workspaceFolderBasename}`, and
`${workspaceRoot}`. Expansion is a single pass and is idempotent for an already-resolved value.
Any other `${...}` token is left literal; the extension reports no diagnostic for it.

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
  packages are loaded through the project graph. A non-empty `[test].sources` glob that matches no
  files does not fail initialization; the unmatched-glob error is deferred to `stc test` and test
  debug launches (D-251).
- Parse diagnostics are published immediately for the edited file; the semantic analysis replaces
  them with the full project diagnostics when it completes. Analysis is whole-project on every run;
  incremental analysis is not offered. A failed project reload keeps the previous analysis and logs
  the failure; it does not publish invented diagnostics.
- `workspace/didChangeWatchedFiles` for an `.st` file, `stc.toml`, or the selected machine
  profile reloads the graph and reanalyzes; the extension sends these events from its file
  watchers and after a tracked rename edits a manifest or profile.
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
| `textDocument/prepareRename` | Supported | Range and placeholder for a renameable root-package identifier, including a `VAR_ACCESS`/`VAR_CONFIG` path segment; `null` for a non-identifier, unresolved position, or a document outside the project source set; `RequestFailed` for an unsupported target (§3.4) |
| `textDocument/rename` | Supported, history-free | Complete `changes` `WorkspaceEdit` over source, `stc.toml`, the selected profile, and access paths; writes no history and no file; warns for identity-bearing and cold-download targets (§3.4) |
| `experimental.stc.trackedRename` | Advertised | Opt-in marker for the private tracked-rename pair; the extension uses it when present |
| `stc/rename` | Supported, private | The same edit plus canonical history entries, online classification, warnings, and a single-use token; writes nothing |
| `stc/recordRename` | Supported, private | Appends the pending entries to `stc.renames.toml` after the client confirms the edit was applied |
| `textDocument/didOpen`, `didChange`, `didSave`, `didClose` | Supported | Full synchronization as described in §3.1 |
| `workspace/didChangeWatchedFiles` | Supported | Reloads `.st`, `stc.toml`, and selected-profile changes |

Diagnostics carry a range, severity (`1` error, `2` warning), the stable `STC-*` code, the source
`stc`, the message, and `relatedInformation` built from compiler labels. The extension does not
reinterpret or re-rank them.

### 3.3 Closed unsupported list

No capability is advertised for, and no support is implemented for, semantic tokens, code
actions, inlay hints, range formatting, call hierarchy, workspace symbols, or pull diagnostics.
Rename is the only reopened capability and is limited to §3.4. Unsupported requests receive a
JSON-RPC method-not-found error; they are not silently accepted. Highlighting comes from the
TextMate grammar, not from semantic tokens. Formatting never edits a range; it returns one
whole-document edit or none.

### 3.4 Rename

Rename is one operation shared with `stc rename` (D-253). `stc-ide` owns the engine
(`RenameContext`, `RenameSession`, and `RenameIndex`); the server calls it through `Snapshot`, and
`stc-driver` builds the same context from its own `load_project_readonly` analysis. The two entry
points therefore edit the same files and produce the same canonical history entries; the server
never writes source.

Advertised surface: `renameProvider` with `prepareProvider`, `experimental.stc.trackedRename`, and
the private `stc/rename` and `stc/recordRename` requests.

`prepareRename` returns `{ range, placeholder }` for a renameable root-package identifier,
including a `VAR_ACCESS`/`VAR_CONFIG` path segment that resolves to a field. It returns `null`
for a position that is not an identifier, has no resolution, or is in a document outside the
project source set, including `stc.toml` and the machine profile. It returns `RequestFailed`
(-32803) with a clear message for a resolved but unsupported target, so an unsupported symbol is
visible instead of a silent no-op.

`textDocument/rename` is history-free for every client. It validates the new name with the lexer
(one identifier, not a reserved keyword), allows a case-only change, and returns one `changes`
`WorkspaceEdit`. The edit covers the declaration, every resolved source reference, qualified
path-prefix segments, `[project].configuration` for the selected configuration,
`[[io.bindings]].variable`, `[[test.case]].name`, the selected profile's `[[io.routes]]`
`plc-input`/`plc-output` `variable`, and `VAR_ACCESS`/`VAR_CONFIG` path segments. It edits no
file and writes no history. For an identity-bearing target it sends one `window/showMessage`
warning that no history was recorded, names the required `[[rename]]` entry, and says the next
online download may be cold. A selected-configuration rename warns about the cold-download risk
on both paths; parameters, locals, and case-only renames stay warning-free. A missing or invalid
selected profile adds a warning that profile references must be fixed by hand, and the rename
proceeds without profile edits.

Tracked path. The VS Code extension sends `stc/rename`, which returns the same edit plus the
canonical root-package history entries, the online classification, warnings, and a single-use
token when history is available. The extension applies the edit, verifies the post-apply text of
every edited file (source, `stc.toml`, and the profile), then calls `stc/recordRename`. The server
reloads and appends `stc.renames.toml` only after that confirmation, and writes nothing else. A
cancel, undo, partial apply, failed verification, or failed history write leaves the history
unrecorded and shows a warning with the recovery (revert and use `stc rename`, or add the
`[[rename]]` entry by hand). A token is single-use, latest-wins per connection, and expires after
five minutes. The controller then sends `workspace/didChangeWatchedFiles` for the edited
manifest/profile files so the server reloads its snapshot.

Rules shared with the CLI:

- only root-package declarations are renameable; standard functions, built-in types, dependency
  declarations, and the root namespace (a package rename) are rejected with a visible message;
- supported targets are the definition, field, parameter, local, and enum-value kinds the CLI
  supports, including qualified path-prefix kinds;
- a new dotted path that already names a different declaration in the root package is rejected
  with `STC-E-PROJECT-013` before anything is edited; the check is case-insensitive and a
  case-only rename of the same target is allowed;
- history is written only for identity-bearing targets (namespaces, aliases, subranges, enums,
  named values, structs, functions, function blocks, programs, classes, interfaces, methods,
  globals, fields, and enum values); parameters, locals, configurations, and case-only renames
  are history-free;
- a missing or invalid selected profile never blocks the rename.

Fallback. When the tracked capability is absent, or `stc/rename` returns method-not-found
(-32601), the extension uses the standard method and warns once per session that history was not
updated and the next online download may be cold. Other errors are rethrown so VS Code shows the
server's message. A server without rename support shows VS Code's normal failure.

## 4. Debug-adapter contract

### 4.1 Session startup and configuration

The extension launches `stc dap` with `DebugAdapterExecutable`; the adapter emits the DAP
`initialized` event once after the `initialize` response. A launch or attach configuration and
`configurationDone` are both required before the adapter starts the host. Breakpoints received
before that point are buffered, resolved against the project graph, and re-reported with a
`breakpoint`/`changed` event when the source mapping becomes available. A launch-owned host is
start-gated (Debug and Monitoring §14.2.1): it publishes its endpoint and waits before cycle 0
while the adapter starts the session, fetches the manifest, and applies every breakpoint received
before `configurationDone`. A launch breakpoint on the first executable statement therefore stops
before the first cycle and observes cycle-0 values; a breakpoint set after configuration completes
binds at the next checkpoint. Attach connects to an already-running host and cannot gate it:
cycles before the attach are not observed or replayed, and its breakpoints bind at the next
checkpoint.

Launch builds the project in the debug form by default and then starts the launcher with a local
debug endpoint and control endpoint. `noBuild` uses the existing artifacts; `outDir` overrides their
directory. Attach connects to an existing endpoint file and does not build or launch.

A remote configuration names an SSH destination and remote work directory instead of a local
endpoint. The Rust driver mirrors the project, builds remotely, starts or reads `stc serve`, and
connects through an OpenSSH tunnel (Debug and Monitoring §13.1.1); the extension never speaks
SSH and adds no network path of its own. A remote launch owns and stops its `stc serve`; a remote
attach leaves an existing serve running. When a remote attach finds source drift, the extension's
pre-launch hook runs `stc remote status --message-format json`, which syncs and builds a candidate
without deploying, and offers **Online change**, **Cold download**, or **Cancel**. Online is
offered only when the host reports a compatible plan. The chosen value is passed as
`remoteDeploy`; cancel starts no session and changes no running host.

A launch with `test` selects one discovered `{stc:test}` POU by exact name (`stc test --list`
prints the names) and builds only that test as the per-test debug harness (Testing and
Verification §2.5): breakpoints bind before the test body runs, the top stack frame is the test
POU, and the session ends when the test host prints its result line and exits. `test` is a launch
setting only; it cannot be combined with `noBuild`, `release`, or attach. A test launch builds
under `<outDir or target/stc/test>/debug/<artifact>`, so ordinary `stc test` artifacts are not
touched. The start gate is the same driver/host launch handshake used by every launch (Debug and
Monitoring §14.2.1), not a protocol operation; the adapter still uses only `stc-debug/1.1`
operations.

| Configuration key | Requests | Meaning |
| --- | --- | --- |
| `project` | launch, attach | `stc.toml`, its directory, or a project directory; the extension defaults it to `${workspaceFolder}/stc.toml` |
| `test` | launch | Exact discovered test name; build and debug only that test |
| `noBuild` | launch, attach | Launch existing artifacts without building; default `false` |
| `outDir` | launch, attach | Artifact directory override, primarily for `noBuild`; a test launch appends `debug/<artifact>` |
| `release` | launch, attach | Build in release mode; monitoring only |
| `allowWrite` | launch, attach | Request the `write` scope and enable `setVariable`; the host must also allow `[debug].allow-write` |
| `allowForce` | launch, attach | Request the `force` scope; the host must also allow `[debug].allow-force`. DAP has no force request, so forcing remains a `stc debug` operation |
| `sourceRoot` | launch, attach | Override the source-path root instead of resolving package roots |
| `args` | launch, attach | Extra arguments appended to the launched host |
| `cwd` | launch, attach | Working directory for the launched host |
| `endpoint` | attach | Endpoint file printed by `stc run --debug-server`; required for a local attach |
| `remote` | launch, attach | SSH destination (`[user@]host`); selects the remote transport |
| `remoteDir` | launch, attach | Remote work directory; default `~/stc-wl12` |
| `remoteStc` | launch, attach | Remote `stc` program on `PATH` or an absolute path; default `stc` |
| `remoteDeploy` | attach | Source-drift choice `ask`, `online`, `cold`, or `never`; default `ask` |
| `remoteSession` | launch, attach | Session descriptor path; defaults under `remoteDir` |

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
- A program or FB body frame has no frame-table locals: its fields are instance roots and appear
  under Globals, so its Locals scope is empty. A function frame still lists its locals. This split
  is the documented F-13 limitation; a frame-to-instance mapping is not implemented.
- Globals groups manifest roots by program instance and exposes top-level global and direct roots.
  It is sourced from `manifest.get` and read with `var.read`. A program instance is shown as a
  container whose fields are its manifest roots.
- Variable expansion uses the typed value's manifest-assisted `view`; aggregates and strings expand
  without further runtime requests. Each scope container is capped at 256 children with a
  truncation entry; values contain no addresses.
- `setVariable` maps to `var.write` only for writable live roots when `allowWrite` is enabled and
  the host granted the `write` scope: global roots, program-instance fields, and live split-field
  children that carry their own runtime root. Stopped-frame locals and contiguous snapshot views
  are read-only and are rejected. The adapter returns the parsed typed value after the host
  accepts the write; on a `stc-debug/1.1` host the write is applied while stopped before the
  response, so a later `variables`, `evaluate`, or step in the same stop reads the new value from
  storage. A `stc-debug/1` host queues the write for the next boundary.
- `evaluate` accepts a stopped-frame local name or a manifest target path. It does not parse ST
  expressions; `1 + 2`, conditions, and arbitrary field expressions fail with an explicit message.

### 4.4 Source mapping

Compiler and protocol source paths are package-relative and `/`-separated. The adapter resolves a
protocol path against the owning package's root directory from the project graph; the root package
wins when relative paths collide. `sourceRoot` overrides the graph resolution. Comparison
normalizes `\` to `/`, strips Windows verbatim/device prefixes, and folds ASCII case on Windows. DAP
`Source` objects carry absolute host paths. A breakpoint on a file that is not a project source is
reported unbound rather than mapped to a guessed path. A test launch also joins the root package's
`[test].sources` to the source table, so breakpoints in test files resolve; the synthetic
`STC_TEST_WRAPPER` frame is hidden from `stackTrace`, leaving the test POU at the top. A test
launch surfaces the deferred `STC-E-PROJECT-004` for an unmatched test glob before starting the
host; an ordinary launch does not.

### 4.5 Events and teardown

Runtime events map to DAP as follows:

| Runtime event | DAP event |
| --- | --- |
| `stopped` | `stopped` with `threadId: 1`, `reason` (`breakpoint`, `step`, `pause`), `allThreadsStopped: true`, and a `description` of `file:line` |
| `resumed` | `continued` with `threadId: 1`, unless the resume response already emitted it |
| `session-ended` | one Debug Console summary line, then `exited` for a launch-owned host, then `terminated` |
| Launcher stdout/stderr | `output` with the corresponding category |
| Session or host exit | one Debug Console summary line, `exited` when a launch-owned host reports an exit code, then `terminated` |

The summary line distinguishes a client stop (`terminate`/`disconnect`, saying whether the
launch-owned host exited with code N or the attached host was left running), a target/host exit
(with its process exit code), a trap (exit code 4), an online swap or stop/idle timeout (with its
`STC-E-DEBUG-*` code), and a connection loss. It is emitted for every teardown path, including a
user stop, so a clean exit and a failed session are not silent. A launch-owned host's real process
exit code is sent in the DAP `exited` event even when the stop was client-initiated; attach has no
owned process and sends no `exited`.

Breakpoint resolution changes produce `breakpoint` events with `reason: "changed"`. `disconnect`
stops the runtime session and exits the adapter; `terminate` ends the session but leaves the adapter
to finish its protocol loop. A launch-owned host is shut down through the control endpoint; an
attached host is left running. An online swap, timeout, or target exit ends the session with no
migration of breakpoints, forces, or watches. There is exactly one debug session per host and one
adapter session per launch. For a test launch, the host's structured `stc-test-result` line is
forwarded as `output` before the session terminates; the test's exit ends the DAP session.

Every launch is start-paused relative to the first cycle. The adapter applies breakpoints, writes
the launch-owned host's start gate on configuration-complete, and only then does the host execute
cycle 0; this guarantees that a launch breakpoint on the first executable statement stops before
the first cycle. A breakpoint set after configuration-complete binds at the next checkpoint.

### 4.6 Honest limits

- One debug session per host; a second session fails until the first ends.
- Every online swap ends the session; reconnect and re-resolve.
- Launch is start-gated, but attach and a host started directly by `stc run --debug-server` cannot
  be paused before they run. Their breakpoints bind at the next checkpoint, and cycles before the
  attach are not observed or replayed.
- When the opt-in `[debug].session-timeout` bound is enabled, a launch session prints the
  effective bound at session start. It does not show a countdown or warn before expiry, and an
  attach session cannot know the running host's bound. An expired bound ends the session with
  `STC-E-DEBUG-018` and resumes the cycle.
- Release and hook-free builds are monitoring-only: no breakpoints, stepping, or Locals. The
  accepted monitoring path is the Debug Console: evaluate visible Globals and target paths while
  the host runs. The adapter also exposes one synthetic `monitoring` frame with the Globals scope
  over DAP, but VS Code's built-in Variables and Watch panes only request a stack after a
  `stopped` event and therefore remain unavailable while the release host is running.
  `evaluate` accepts a manifest key, not a source-language path: the sample global `TOTAL` is
  `SAMPLE::TOTAL`, not `MAIN.TOTAL`.
- Local names are uppercase normalized spellings from the stopped-frame metadata.
- Caller frames report their call-site file and line from the build-time line map, at any depth;
  the stopped frame keeps its checkpoint source. A caller with no line map or an optimized-out
  function reports unavailable information honestly rather than a fabricated location.
- `evaluate` does not parse ST expressions.
- Remote debugging requires an OpenSSH client, an SSH agent/key, and a `known_hosts` entry on the
  development host; the extension never prompts for a password or accepts an unknown host key.
  The remote login account must own the packaged `stc`, the work directory, and the user-only
  session files. One remote serve session at a time owns a work directory. There is no automatic
  reconnect after a tunnel or host loss, and an online swap or cold install ends the session;
  reattach starts a new session and re-applies breakpoints.
- Debug write and force can hold outputs indefinitely; they are opt-in and never presented as
  safety functions.
- A write made while the host is running takes effect at the next cycle boundary, not immediately.
  While stopped, a `stc-debug/1.1` host applies a one-shot write before answering; a
  `stc-debug/1` host queues it until the cycle resumes. A force is always applied at a boundary.

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

### 4.8 Testing API

The extension contributes a VS Code Testing API controller for `{stc:test}` POUs. The controller
lives in `editors/vscode/src/testing/session.ts`; list/result parsing, argument construction, and
file grouping live in `editors/vscode/src/testing/model.ts`, which does not import `vscode`. There
is no new compiler protocol: both discovery and runs use the `stc test` JSON surface in
[CLI and Manifest](CLI-and-Manifest.md) §1.6.

**Controller and discovery.** The controller id and label are `stc`. It is created only after at
least one workspace folder, or one of its ancestors, contains `stc.toml`; it stays available if the
manifest later disappears. Discovery starts one `stc test --list --message-format json` per unique
manifest in the manifest directory. Tests are grouped by source file under a file-level TestItem;
each test item carries a `file:` URI and a 1-based range at the test declaration start (the
`{stc:test}` annotation line when present). The item hierarchy shows file paths and test names, and
the gutter/code lens open the declaration. Discovery refreshes on `**/*.st` and `**/stc.toml`
create/change/delete with a 500 ms debounce, on workspace-folder changes, on `stc.path` changes,
through the Test Explorer refresh button, and through the `stc.refreshTests` command. A failed
listing logs its diagnostics to the `stc` output channel and keeps the previous successful list;
a missing `stc` executable is logged and no items are created.

**Run profile.** The default Run profile expands a file-level selection to its children and honors
`exclude`; with no `include`, it selects every discovered test. Selected tests are grouped per
manifest and each project runs sequentially:

```text
stc test --manifest-path <project>/stc.toml --message-format json --color never
    --diagnostic-paths absolute --exact <test>... -- --nocapture
```

`--exact` makes each positional filter a complete-name match (ASCII case-insensitive), so one
selected test never also runs another test whose name contains it. The runner's result objects are
matched to items by exact reported name; an unexpected result is logged and never reported against
another item. `pass` reports passed; `fail`, `trap`, and `timeout` report failed with the failure
messages and the first available file/line/column resolved against the manifest directory. The
reported `duration_ms` becomes the Test Explorer duration. Test-host output captured between
results, including JSON printed by tests, is attached to the test's run output. Compiler
diagnostics and stderr are appended to the run output; if a selected test has no result after a
failed build or a non-zero exit, it is reported errored with the diagnostic or exit reason.
Cancellation stops the process tree and marks the remaining selected tests skipped. A missing
`stc` executable reports every selected test errored and is logged.

**Debug profile.** The Debug profile starts one ordinary launch session per selected test,
sequentially; a file-level or multi-test selection therefore opens one session per test, and the
next starts only after the previous terminates. Each session uses the F-17 per-test launch:
`type: "stc"`, `request: "launch"`, `name: "stc: Debug <test>"`, `project` set to the manifest,
`test` set to the exact test name, and `noBuild`, `release`, `allowWrite`, and `allowForce` false.
The workspace folder is resolved from the test URI with the same ancestor search used for
discovery; multi-root projects use their own folder. Cancellation stops the active session and
aborts the remaining selection. No Test Explorer results are fabricated for a debug session: the
F-17 Debug Console result line and the debug session are authoritative.

**Limits.** Running and debugging require the same clang and runtime artifacts as the CLI; discovery
alone does not build. Continuous run, coverage integration, and test tags are not offered. The
Testing API presents the CLI's per-test model; it does not add test retries, sharding, or
parallelism (v1 tests run sequentially).

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
  clang, and cover launch with a first-statement breakpoint on cycle 0, known locals and globals,
  stepping, evaluate, write gating, attach, no-build, terminate/exit reporting, the timeout-budget
  line, and clean teardown. They run through the debug suite and require clang there.
- The extension checks are `npm ci`, `npm run typecheck`, `npm test` (problem-matcher unit tests,
  theme JSON tests, online-change state tests, Testing API model tests, task-argument tests, and
  TextMate grammar fixtures), and `npm run package`. `test/online-change.test.mjs` bundles the pure
  state module and covers every status line, the stopped reasons, endpoint parsing and field
  whitelisting (including wrong format/protocol, oversized and non-regular files, and malformed
  fields), redaction, and status formatting. `test/testing.test.mjs` bundles the pure Testing API
  model and covers list parsing with file/line/column and malformed entries, result parsing and
  pass/fail/trap/timeout mapping with failure locations and captured output, empty and malformed
  output, exact run/debug argument construction, file-level grouping, and result-path resolution.
  `test/tasks.test.mjs` bundles the pure task-argument module and covers all five commands, the
  generated argument lists, `--release` and diagnostic gating, appended `args`, expansion of every
  supported variable in `project` and each argument, the unknown-token rule, and the shared `run`
  terminal argument list. The theme test checks the
  contributed themes, required editor colors and token rules, distinct acceptance categories, and
  the 4.5:1 contrast target. The Rust tests compare the
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
  tasks, run one launch and one attach session, run and debug one test from the Testing view, and
  run the online-change start/attach/swap/stop walkthrough while inspecting the channel and logs
  for control metadata.

## 7. Related documents

- [Debug and Monitoring](Debug-and-Monitoring.md) — `stc-debug/1` and `stc-debug/1.1`, operations,
  limits, and the stopped-frame sidecar.
- [CLI and Manifest](CLI-and-Manifest.md) §1 — `stc lsp`, `stc dap`, and `--diagnostic-paths`.
- [Testing and Verification](Testing-and-Verification.md) — CI matrices and the headless smoke job.
- [User Guide](../../USER-GUIDE.md) — installation, settings, tasks, and launch/attach usage.
- [Known Limitations](../../KNOWN-LIMITATIONS.md) — the consolidated editor limits.
