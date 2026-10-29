# WL-1.1 Plan — Local VS Code Integration and Debugging

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-29  
Branch: `WL-1.1` (sub-plan of `v2.0.0-planning`)  
Status: Planning started 2026-10-29. Decisions accepted 2026-10-29; I-11, I-13, and I-15 are
implementation defaults. Implementation has not started.  
Parent: [V2.0.0-PLAN.md](V2.0.0-PLAN.md) §4 (WL-1.1)  
Inputs: [FutureWishlist.md](FutureWishlist.md) WL-1.1,
[Debug and Monitoring](specification/Architecture/Debug-and-Monitoring.md),
[CLI and Manifest](specification/Architecture/CLI-and-Manifest.md),
[Testing and Verification](specification/Architecture/Testing-and-Verification.md),
[Decisions](specification/Decisions.md) D-041,
[USER-GUIDE.md](USER-GUIDE.md), [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md)

## 1. Objective

Make VS Code the practical local development environment for `stc`: edit and navigate ST, run
build/test/run/fmt tasks, see compiler diagnostics in the Problems panel, format with `stc fmt`,
and debug a local runtime through breakpoints, stepping, variables, and call stacks.

Everything runs locally against the local compiler and runtime. The extension is a client: the
compiler remains the single source of truth for syntax, diagnostics, types, and formatting. WL-1.1
adds no language, compiler, artifact-format, or module-ABI feature; the only runtime change is the
versioned `stc-debug/1.1` addition described in §4.2.

## 2. Locked scope and boundaries

In scope:

- A local-only VS Code extension with a TextMate grammar and language configuration.
- Tasks and problem matchers for `stc check`, `stc build`, `stc run`, `stc test`, and `stc fmt`.
- A language server (`stc-lsp`) for push diagnostics, document symbols, go-to-definition,
  references, hover, completion, and `stc fmt` formatting.
- A debug adapter (`stc-dap`) translating DAP to the local `stc-debug/1.1` protocol (with
  `stc-debug/1` compatibility): launch and attach, source breakpoints, stepping, variables,
  watch, and call stack.
- Packaging as a VSIX, with a sample ST workspace and launch/task configurations.
- Smoke tests for diagnostics, navigation, formatting, and a local debug session.

Out of scope:

- Remote or network IDE access, remote source mapping, and remote attachments (WL-1.2).
- A second diagnostics/type engine, a second formatter, or a second debugger protocol. The
  extension surfaces compiler and runtime output; it does not reimplement them.
- Semantic tokens, call hierarchy, rename refactoring, code actions, inlay hints, workspace
  symbols, and incremental analysis.
- Conditional, hit-count, log, function, data, and instruction breakpoints; exception
  breakpoints; reverse debugging; multi-thread debugging; and ST expression evaluation.
- Marketplace publishing, code signing, telemetry, and extension-bundled compiler binaries.
- Any safety function. Debug write, force, and stop can hold outputs or move machinery.

## 3. Findings from the current tree (2026-10-29)

1. **The debug substrate is complete and local-only.** The M6.5 runtime implements the
   `stc-debug/1` protocol: authenticated local endpoint, scopes (observe/control/write/force),
   typed reads/writes/forces/watches, source and checkpoint breakpoints, over/into/out stepping,
   stopped-frame stack and local reads, audit, and the stopped-frame sidecar
   ([Debug and Monitoring §13–§18](specification/Architecture/Debug-and-Monitoring.md)). The
   adapter is a translation layer, not a new debugger.
2. **The driver already owns build, launch, and attach.** [`debug.rs`](crates/stc-driver/src/debug.rs)
   builds or attaches, starts the host, waits for the endpoint file, starts one session, and
   shuts the host down. The runtime exposes a public `DebugClient` with ordered calls, events,
   and polling ([`debug_server.rs`](crates/stc-runtime/src/debug_server.rs)). The DAP adapter can
   reuse this plumbing instead of re-implementing host launch.
3. **`stc debug --batch` is not an adapter transport.** Batch mode is a one-shot JSON-lines
   script of `op`/`params` requests; it does not stream DAP events or serve interactive
   requests. The adapter must link `DebugClient` (or the equivalent protocol client), not drive
   the CLI batch command.
4. **There is no IDE-facing compiler API yet.** [`stc-sema`](crates/stc-sema/src/session.rs)
   exposes `Analysis { hir, resolutions }` and the project entry points; D-041 reserves the full
   CST/IDE API. Navigation is nevertheless proven: [`rename.rs`](crates/stc-driver/src/rename.rs)
   already resolves a declaration, walks `resolutions`, maps resolved nodes to identifier
   tokens, and rewrites only those tokens. Go-to-definition and references can use the same
   data.
5. **The CST is lossless and stable, but has no position lookup.** `SyntaxKind` is stable,
   append-only, and enumerated by `SyntaxKind::ALL` with the checked-in
   [`syntax_kinds.txt`](crates/stc-syntax/tests/syntax_kinds.txt) table. There is no public
   "token/node at byte offset" helper; the language server needs one small additive view.
6. **Diagnostics are already structured and machine-readable.** [`stc-diag`](crates/stc-diag/src/diagnostic.rs)
   carries stable codes, severity, a primary span, labels, notes, and suggestions; the driver
   renders them as JSON and as `file:line:column: severity[CODE]: message` short lines
   ([`report.rs`](crates/stc-driver/src/report.rs)). LSP diagnostics and a short-format problem
   matcher both map directly.
7. **Formatting is a public library.** [`stc-fmt`](crates/stc-fmt/src/lib.rs) exposes
   `format_bytes` and `format_tree` with the fixed style. The language server must call it and
   return a text edit; the extension must not format anything itself.
8. **Project loading and analysis are reusable libraries.** `stc-manifest` exposes
   `load_project_readonly`, the package graph, and source paths; `stc-sema` exposes
   `analyze_project`. An editor snapshot can be built without the driver binary, but the loader
   logic currently exists only inside driver modules (`project.rs`, `rename.rs`) and needs a
   shared home.
9. **The CLI boundary anticipates `stc lsp`.** [CLI and Manifest §1.1](specification/Architecture/CLI-and-Manifest.md)
   lists `stc lsp` and `stc doc` as future commands. The released package ships one `bin/stc`,
   so implementing the servers as subcommands avoids new binaries and packaging changes.
10. **The workspace already uses `serde_json`, has no async runtime, and favors hand-rolled
    protocols.** `stc-syntax` depends on `serde`/`serde_json`; the runtime and control/debug
    protocols are synchronous and hand-written. A synchronous, single-threaded LSP/DAP loop with
    `Content-Length` framing is consistent with the codebase.
11. **`stc-debug/1` cannot enumerate locals.** `stack.list` returns frame index, function
    identity, symbol, source, and `optimized_out`, but not local names; `frame.read` is addressed
    by `{frame, name}`. The runtime captures locals internally (`StopFrame.locals`) and reads
    names from the stopped-frame sidecar, whose format is documented in §16.9 and parsed
    privately in [`frames.rs`](crates/stc-runtime/src/frames.rs). This is the one material
    protocol gap for a useful Variables view; I-4 resolves it with an additive
    `stc-debug/1.1` operation.
12. **Debug source paths are package-relative.** Codegen normalizes source names to
    `/`-separated package-relative paths ([`debug.rs`](crates/stc-codegen/src/debug.rs),
    [`module.rs`](crates/stc-codegen/src/module.rs)), and breakpoint/stack results report them
    verbatim. The adapter and extension must map them to workspace files; the Windows
    development host makes drive letters and case folding part of that mapping.
13. **Positions differ by contract.** `stc-span` columns count Unicode scalar values and are
    1-based; LSP positions are 0-based UTF-16 code units by default. All conversions belong in
    one layer and must be tested with non-ASCII comments and strings.
14. **There is no editor tooling to build on.** The repository has no `package.json`, Node build,
    extension, grammar, or launch configuration; [`.vscode/`](.vscode/) contains only Rust
    coverage tasks. The extension, its toolchain, and its CI job are new surface.
15. **CI is matrix- and detect-gated.** The workspace tests run on Windows x86-64, Linux x86-64,
    and Linux AArch64; the debug suite runs through `cargo xtask test --suite debug` with clang
    and debug-inspection tools; docs use Node 22 for Markdown lint; packaging builds one `stc`
    package per host. IDE tests must fit these patterns rather than add an unrelated pipeline.
16. **The examples are not an IDE fixture.** `examples/hello`, `motor`, `motorlib`, and
    `online-change` exercise compiler features, but none provides a stable multi-file
    navigation, diagnostics, and debug session fixture with editor configurations.

## 4. Decisions

Statuses: **Accepted** means the owner confirmed the choice on 2026-10-29; **Default** follows
from the tree evidence and may change during implementation without a scope change.

| ID | Decision | Status |
| --- | --- | --- |
| I-1 | Add a new `stc-ide` crate as the IDE-facing query layer over `stc-manifest`, `stc-parser`, `stc-sema`, `stc-syntax`, `stc-span`, `stc-diag`, and `stc-fmt`. Add only the minimal public CST view it needs (`SyntaxTree` token/node lookup by byte offset); make no semantic or compiler change. | Accepted |
| I-2 | Implement the language server as a new `stc-lsp` crate with a `stc lsp` subcommand in the driver. Transport is stdio with hand-rolled `Content-Length` JSON-RPC over `serde_json`; file URIs use the `url` crate; the loop is synchronous and sequential. LSP positions are UTF-16. | Accepted |
| I-3 | Implement the debug adapter as a new `stc-dap` crate with a `stc dap` subcommand. DAP runs over stdio; translation targets `stc-debug/1.1` through the runtime's `DebugClient`. The driver owns build/launch/attach and passes a connected client and process handle to the adapter; `launch` builds by default with a `noBuild` opt-out. | Accepted |
| I-4 | Add a versioned `stc-debug/1.1` protocol revision with a `frame.locals` operation that lists a stopped frame's local and parameter names, declared types, and live/unavailable state from the captured stop snapshot. Servers advertise both `stc-debug/1` and `stc-debug/1.1`; the v1 contract is unchanged. Values still come from `frame.read`. | Accepted |
| I-5 | Put the extension at `editors/vscode/` as TypeScript, bundled with esbuild, using `vscode-languageclient` for LSP and a `DebugAdapterExecutable` for DAP. Pin the Node toolchain with `package-lock.json`; the engine floor is the pinned language client's declared minimum (currently VS Code 1.82), verified at implementation. | Accepted |
| I-6 | Distribute a VSIX built in CI, uploaded as an artifact and attached to the release; do not publish to the marketplace in v2.0.0. Do not bundle the compiler: the extension resolves `stc` from the `stc.path` setting or `PATH`. | Accepted |
| I-7 | LSP capability set is closed: push diagnostics, document symbols, definition, references, hover on identifiers, completion (keywords, visible names, one-level member completion after `.`), and whole-document formatting. No semantic tokens, rename, code actions, range formatting, call hierarchy, or workspace symbols. | Accepted |
| I-8 | Recompute the project analysis on open/change/save with a short debounce, using unsaved-buffer overlays; parse diagnostics are published first, semantic diagnostics after the analysis. Include root-package test sources so editor behavior matches test builds. No incremental analysis in v2.0.0. | Accepted |
| I-9 | DAP capability set is closed: launch/attach, source breakpoints, continue/pause/over/into/out, call stack, Locals and Globals scopes, variables with expansion, `setVariable` for writable live roots when write is enabled, and `evaluate` for target paths only. Conditional/hit/log/function/data/instruction breakpoints, exception breakpoints, restart, multi-thread, and ST expression evaluation are unsupported and reported honestly. | Accepted |
| I-10 | Map protocol source paths automatically from the project graph: `src/main.st` resolves against the owning package's root directory, with `/` separators and host-normalized case; an optional `sourceRoot` override remains available. Launch/attach configurations carry `project` (manifest path) and an optional `outDir` for `noBuild`. | Accepted |
| I-11 | Add `editors/vscode/samples/workspace/` as the IDE fixture: a small multi-file project (program, function block, function, test) with a known breakpoint location, known locals/globals, and committed `.vscode/launch.json` and `tasks.json`. Use it for LSP and DAP tests. | Default |
| I-12 | Verification: Rust LSP tests in the workspace test matrix; Rust DAP and `stc-debug/1.1` protocol tests in the debug suite matrix (they need clang); a CI extension job that runs `npm ci`, typecheck, grammar fixtures, problem-matcher unit tests, and `vsce package`, plus a headless `@vscode/test-electron` job that installs the VSIX and exercises activation, language features, and a local debug session. The VSIX is uploaded as an artifact. | Accepted |
| I-13 | The TextMate grammar is maintained against `Tokenization.md` and the lexer tables; a small deterministic check compares the grammar's keyword/operator coverage with the `Keyword::ALL`/`SyntaxKind` lists so drift fails CI. | Default |
| I-14 | Debug writes and forces are disabled by default in the editor. `allowWrite` enables `setVariable`, and `allowForce` additionally enables force/release, mirroring the CLI flags; the adapter emits the runtime's non-safety notice as an `output` event at session start. | Accepted |
| I-15 | Documentation: add `specification/Architecture/IDE-Integration.md` (LSP/DAP contracts, capability maps, source mapping, packaging, limits); update CLI §1, Debug and Monitoring cross-references, README, USER-GUIDE, and KNOWN-LIMITATIONS. | Default |
| I-16 | Use language id `st` with TextMate scope `source.st`; confirm there is no conflict with other installed Structured Text extensions in the Electron and local checks. | Accepted |

### 4.1 Why the crate split

The driver binary's modules are `pub(crate)` and mix project loading with command policy. The
servers need long-running loops and unit-testable protocol translation, which do not belong in
the `stc` binary. Splitting into `stc-ide` (compiler queries), `stc-lsp` (LSP protocol), and
`stc-dap` (DAP protocol) keeps each layer testable and lets the driver stay a dispatcher. The
alternative — implementing both servers inside `stc-driver` — avoids two crates but makes the
protocol work untestable outside the binary and grows the driver's already large surface.

### 4.2 Why a versioned protocol for locals

The v1 protocol exposes typed access by name and a stopped-frame snapshot behind `stop_id`, but
`stack.list` does not enumerate local names and `frame.read` requires one. The runtime already
captures the full typed local map for each stopped frame (`StopFrame.locals`), so the missing
piece is an enumeration operation, not new data.

The owner chose a protocol revision over adapter-side sidecar reading. The server supports both
`stc-debug/1` and `stc-debug/1.1`: version negotiation stays exact at the first request, and the
`STC-E-DEBUG-002` `detail.supported` list names both versions. `stc-debug/1.1` adds
`frame.locals` with scope `observe`, parameters `{stop_id, frame}`, and a result of
`{locals:[{name, type, live, available}], truncated}` sorted by normalized name. It returns no
value and no address; values remain `frame.read` requests. The existing request/response limits
apply, with a 1024-entry local list cap and `truncated:true`.

This keeps the runtime the single source of frame information and makes locals work in attach
mode without an out-dir. The cost is a protocol-version contract: the server must preserve exact
`stc-debug/1` behavior, the specification gains a 1.1 section, and both versions need tests. The
adapter's capability map records that a 1.1 session is required for the Locals scope.

### 4.3 Why not a second protocol

The debug adapter must not become a second debugger. It only translates DAP requests to the
negotiated `stc-debug/1.1` operations (or the `stc-debug/1` subset) and DAP events from protocol
events; it does not read host memory, parse DWARF/CodeView, or reconstruct values. Type labels
and values come from the protocol's typed envelope; expansion uses the returned `value` and
manifest-assisted `view`. This keeps debug values consistent with `stc debug` and the runtime's
audit behavior.

## 5. Architecture

### 5.1 Component layout

| Component | Location | Responsibility | Depends on |
| --- | --- | --- | --- |
| IDE queries | `crates/stc-ide` (new) | Project snapshot, overlay texts, diagnostics, definition, references, hover, completion, symbols, formatting results | `stc-diag`, `stc-fmt`, `stc-hir`, `stc-manifest`, `stc-parser`, `stc-sema`, `stc-span`, `stc-syntax` |
| Language server | `crates/stc-lsp` (new) | JSON-RPC/LSP framing, lifecycle, document store, capability dispatch, position/URI conversion | `stc-ide`, `stc-span`, `stc-diag`, `serde_json`, `url` |
| Debug adapter | `crates/stc-dap` (new) | DAP framing, launch/attach arguments, breakpoint/step/stack/variable translation, 1.1 locals listing, source mapping | `stc-runtime`, `stc-got`, `stc-manifest`, `serde_json` |
| Driver dispatch | `crates/stc-driver` | `stc lsp` and `stc dap` subcommands; build, launch, attach, process ownership via existing debug plumbing | `stc-lsp`, `stc-dap`, existing crates |
| Runtime protocol | `crates/stc-runtime` (existing) | `stc-debug/1.1` negotiation and `frame.locals` from the captured stop snapshot | unchanged |
| VS Code extension | `editors/vscode/` (new) | Language client, debug adapter factory, task provider, problem matcher, grammar, language configuration, settings, sample workspace | VS Code API, `vscode-languageclient` |
| CST view | `crates/stc-syntax` | Additive token/node lookup by byte offset | unchanged |

### 5.2 Language server flow

1. VS Code starts `stc lsp` for a workspace folder. The server discovers `stc.toml` from
   `rootUri`/`workspaceFolders` (or `--manifest-path`) and builds a snapshot with
   `stc-manifest::load_project_readonly`.
2. Open documents are parsed from the client's text; closed documents from disk. The snapshot
   runs `stc_sema::analyze_project` with production and root test sources and keeps the
   `Analysis`, the `SourceMap`, and per-file diagnostics.
3. On open/change, the server records the new text and schedules a debounced reanalysis. Parse
   diagnostics are published immediately; semantic diagnostics replace them when the analysis
   completes. `didChangeWatchedFiles` for `**/*.st` and `**/stc.toml` reloads the graph.
4. Queries use `Analysis.resolutions` exactly as `stc rename` does: find the innermost CST node
   at the position, map it to a `Res`, then to a `Def`/`Field`/`Param`/`Local`/`EnumValue`
   declaration span. References scan all resolutions for the same identity and include the
   declaration name when requested.
5. Formatting parses the open document with `stc-fmt::format_bytes` and returns one full-document
   edit, or no edit when syntax errors prevent formatting.

### 5.3 Debug adapter flow

1. VS Code starts `stc dap` and sends `initialize`; the adapter replies with its closed DAP
   capability set.
2. `launch`: the driver builds the project (unless `noBuild` is set), starts the launcher with a
   debug endpoint and control endpoint, waits for the endpoint file, connects `DebugClient`,
   and starts a session with observe/control (plus write/force when enabled), negotiating
   `stc-debug/1.1`. `attach` connects to an existing endpoint file. Breakpoints received before
   the session is active are buffered; `configurationDone` applies them and completes `launch`.
3. Protocol events drive DAP events: `stopped` → `stopped`, `resumed` → `continued`,
   `session-ended` → `terminated` with the protocol reason, and launcher output → `output`.
   One DAP thread (`threadId: 1`) represents the cycle thread.
4. `stackTrace` calls `stack.list`; `scopes` offers Locals (`frame.locals` at the current stop)
   and Globals (manifest roots, grouped by program instance); `variables` calls
   `var.read`/`frame.read` and expands from the returned typed value and `view` without further
   requests. `setVariable` maps to `var.write` only for live roots when the write scope is
   granted; stopped-snapshot children are read-only.
5. `disconnect`/`terminate` stop the session; a launch-owned host is shut down through the
   control channel, while attach mode leaves it running. Swaps end the session; the adapter
   reports `terminated` rather than reconnecting or migrating breakpoints.

### 5.4 Position and path mapping

- LSP: byte spans → (1-based line, 1-based Unicode-scalar column) via `stc-span` → 0-based
  UTF-16 LSP positions. All conversions live in one module and are tested with non-ASCII text.
- DAP: protocol source paths are package-relative. Resolve each path against the owning package's
  root directory from the project graph, with an optional `sourceRoot` override. Normalize `\`
  to `/`, compare case-insensitively on Windows, and emit absolute paths in DAP `Source`
  objects.
- Absolute paths never appear in compiler or protocol output; only the adapter/editor layer
  resolves them.

## 6. Capability map

### 6.1 LSP

| Capability | Status | Notes |
| --- | --- | --- |
| `publishDiagnostics` | Supported | Parse-first, then semantic; stable `STC-*` codes; related information from labels |
| `textDocument/documentSymbol` | Supported | Hierarchical POU/type/variable symbols from the typed AST |
| `textDocument/definition` | Supported | Identifier → declaration through `resolutions`; enum values resolve to their value span |
| `textDocument/references` | Supported | All resolutions for the same identity, including declaration on request |
| `textDocument/hover` | Supported, identifiers only | Declared type, declaration kind, and owner; expression typing is not part of v2 |
| `textDocument/completion` | Supported, bounded | Keywords, visible names, and one-level member completion after `.`; no auto-import |
| `textDocument/formatting` | Supported | Whole document via `stc fmt`; no range formatting |
| `textDocument/didChange` | Full sync | Reparse the whole document; no incremental sync |
| Semantic tokens, rename, code actions, inlay hints, workspace symbols, call hierarchy | Not supported | Not in the locked scope; the grammar provides highlighting |

### 6.2 DAP

| DAP feature | Mapping | Status |
| --- | --- | --- |
| `initialize` | Adapter capabilities | Supported |
| Session protocol | `stc-debug/1.1` with `stc-debug/1` still accepted | Negotiated at the first request |
| `launch` / `attach` | Build + `stc run --debug-server` / endpoint file + `session.start` | Supported; `launch` builds by default |
| `setBreakpoints` | `breakpoint.set` per source line; unbound lines reported `verified:false` | Supported |
| `configurationDone` | Apply buffered breakpoints, complete launch | Supported |
| `continue` / `pause` | `exec.continue` / `exec.pause` | Supported |
| `next` / `stepIn` / `stepOut` | `exec.step` over/into/out | Supported |
| `stackTrace` | `stack.list` with the current `stop_id` | Supported |
| `scopes` | Locals (`frame.locals`) and Globals (manifest roots) | Supported; Locals require frame metadata |
| `variables` / `setVariable` | `var.read`/`frame.read`; `var.write` for live roots when write is enabled | Supported; snapshot children read-only |
| `evaluate` | Target path or stopped-frame local name only | Supported with limits; no ST expressions |
| Stopped/continued/terminated/output events | `stopped`/`resumed`/`session-ended`/launcher stdio | Supported |
| Conditional, hit-count, log, function, data, instruction breakpoints | None | Unsupported; advertised false |
| Exception breakpoints | None; traps end the session per the runtime | Unsupported |
| `restart`, multi-thread, `stopOnEntry`, memory/disassembly | None | Unsupported |

### 6.3 Honest limits

- One debug session per host and one adapter session per launch; the runtime enforces both.
- Every online swap ends the session; breakpoints, forces, and watches do not migrate.
- Release and hook-free builds are monitoring-only: no breakpoints, stepping, or locals.
- A `stc-debug/1`-only host has no Locals scope even when the build has frame metadata.
- Local names are shown in the frame table's normalized uppercase form; source spelling is not
  retained by the debug metadata.
- `evaluate` does not parse ST expressions; watches are manifest target paths.
- No remote transport, TLS, or authentication beyond the local endpoint token and user-only
  endpoint file; those remain WL-1.2/WebUI concerns.
- Debug write/force can hold outputs indefinitely; the extension shows the non-safety notice and
  keeps write/force opt-in.

## 7. Work breakdown

| Phase | Work | Acceptance |
| --- | --- | --- |
| 0 | Decisions locked 2026-10-29; review the capability maps, the `stc-debug/1.1` contract, and the verification plan before phase 1 | Decisions recorded; review notes resolved |
| 1 | `stc-ide` snapshot and query layer; additive CST token/node lookup; diagnostics, definition, references, hover, completion, symbols, formatting | Unit tests over `examples/` and the sample workspace pass; no compiler semantics change |
| 2 | Extension shell: manifest, grammar, language configuration, task provider, problem matcher, settings, esbuild, VSIX packaging | Extension activates; grammar fixtures and matcher tests pass; `vsce package` succeeds |
| 3 | `stc-lsp`: framing, lifecycle, document store, capability dispatch, position/URI conversion, diagnostics, queries, formatting; `stc lsp` command | LSP integration test covers diagnostics, symbols, definition, references, hover, completion, and formatting |
| 4 | Runtime `stc-debug/1.1` negotiation and `frame.locals`; `stc-dap`: framing, launch/attach, breakpoints, stepping, stack, scopes/variables, evaluate, write/force; `stc dap` command | Protocol 1.0 and 1.1 tests pass; DAP integration test stops at a breakpoint, steps, lists frames, and reads globals and locals |
| 5 | Sample workspace, launch/task configurations, IDE-Integration spec, `stc-debug/1.1` spec update, CLI/README/USER-GUIDE/KNOWN-LIMITATIONS updates, release checklist | Extension and docs reviewed; sample workspace opens, navigates, formats, and debugs |
| 6 | CI: LSP tests in the workspace matrix, DAP/protocol tests in the debug suite, extension package job and VSIX artifact, headless Electron smoke; record evidence | All jobs pass on the supported hosts; VSIX artifact and smoke evidence recorded |

## 8. Exit criteria

- The VSIX installs and activates in VS Code; syntax highlighting works on the sample workspace.
- Diagnostics appear in the Problems panel from the language server and from `stc check` tasks.
- Definition, references, hover, completion, document symbols, and formatting work on the
  sample workspace and are covered by automated tests.
- A local debug session supports launch and attach, source breakpoints, stepping, variables
  (globals and locals), watch targets, and call stacks.
- The DAP capability map and the LSP capability map are documented, including unsupported
  features and their reporting.
- `stc-debug/1.1` is documented and tested; `stc-debug/1` clients keep their exact v1 behavior.
- The extension is packaged and pinned; packaging instructions and the in-editor checklist are
  documented.
- LSP, DAP, and headless VS Code smoke tests run in CI; the release checklist records the manual
  in-editor pass.
- The extension has no compiler logic: diagnostics, types, and formatting come from `stc` crates.
- No remote or network capability is added, and the non-safety stance is visible in the editor.

## 9. Verification and CI

### 9.1 Rust LSP tests

- Spawn `stc lsp` and speak `Content-Length`-framed JSON-RPC over stdio; no editor required.
- Cover `initialize`, `didOpen`, `didChange`, `didSave`, `didClose`, `definition`, `references`,
  `hover`, `completion`, `documentSymbol`, `formatting`, `shutdown`, and `exit`.
- Run in the workspace test matrix on Windows and Linux; they need no clang.
- Fixtures: the sample workspace plus a deliberately malformed file for parse diagnostics.

### 9.2 Rust DAP tests

- Spawn `stc dap`; drive `initialize`, `launch`, `setBreakpoints`, `configurationDone`,
  `stackTrace`, `scopes`, `variables`, `next`, `continue`, `disconnect`.
- Build the sample workspace with the existing driver in the test setup; require clang like the
  debug suite and run through `cargo xtask test --suite debug`, extending the suite definition to
  include the adapter tests.
- Assert the stop location, frame name, a known local and global value, and clean teardown.
- Add runtime protocol tests for `stc-debug/1.1` negotiation, `frame.locals` ordering, limits,
  unavailable locals, and `stc-debug/1` rejection of the new operation.
- Add an attach case against an already-running `stc run --debug-server`.
- Skip cleanly when clang is unavailable, but the CI debug job sets `STC_REQUIRE_CLANG=1` so it
  cannot silently skip there.

### 9.3 Extension checks

- `npm ci` from the committed lockfile; TypeScript typecheck and unit tests.
- Grammar fixtures with scope assertions, including comments, strings, numbers, keywords,
  operators, and pragmas.
- Problem-matcher tests that apply the package's regex to sample short-format lines.
- `vsce package` produces the VSIX; CI uploads it with the extension job.
- The keyword/operator coverage check compares the grammar against the lexer's `Keyword::ALL`
  and `SyntaxKind` lists.

### 9.4 In-editor smoke and release checklist

CI runs a headless `@vscode/test-electron` job that installs the VSIX, activates the extension
on the sample workspace, and exercises highlighting, diagnostics, definition, completion,
formatting, a task run, and a launch debug session. The job runs on Linux under `xvfb`; Windows
and Linux local runs are the fallback when the runner is unavailable.

The release checklist keeps a manual pass on the Windows development host and at least one Linux
host: install the VSIX, open the sample workspace, confirm all language features and tasks, and
run one launch and one attach debug session. The checklist and its result are recorded in the
release evidence.

## 10. Documentation and specification updates

- New `specification/Architecture/IDE-Integration.md`: transports, capability maps, source and
  position mapping, packaging, configuration keys, limits, and the non-safety stance.
- [CLI and Manifest](specification/Architecture/CLI-and-Manifest.md) §1: implement `stc lsp` and
  add `stc dap`, with their options and exit behavior.
- [Debug and Monitoring](specification/Architecture/Debug-and-Monitoring.md): add the
  `stc-debug/1.1` negotiation, the `frame.locals` operation, its limits, and its result shape to
  §13 and §18; keep the `stc-debug/1` contract unchanged and cross-reference the adapter as a
  client.
- [README.md](README.md), [USER-GUIDE.md](USER-GUIDE.md): editor installation, settings, tasks,
  launch/attach configurations, and the sample workspace.
- [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md): the IDE section from §6.3 and the local-only
  stance.
- [RELEASE-CHECKLIST.md](RELEASE-CHECKLIST.md) and [RELEASE-NOTES.md](RELEASE-NOTES.md) at
  release time, including the VSIX evidence.

## 11. Risks and open questions

### 11.1 Risks

- **Scope creep.** LSP/DAP capability lists are closed; requests for semantic tokens, rename,
  expression evaluation, or remote debugging are separate work.
- **Analysis latency.** Full-project reanalysis is simple and correct but may be slow on large
  projects; the debounce and measurement come first, incremental analysis is out of scope.
- **API sufficiency.** Hover and completion are limited by the public views; if expression typing
  is required, it is an additive view under D-041, not a language change, and must be budgeted.
- **Protocol evolution.** `stc-debug/1` and `stc-debug/1.1` must both stay correct: exact 1.0
  behavior for old clients, 1.1 negotiation, and a new operation whose limits and unavailable
  cases are tested. This is a runtime and specification change, larger than an adapter-only
  sidecar read.
- **DAP edge behavior.** Breakpoints sent before launch, Windows path case, and package-relative
  sources need integration tests on both Windows and Linux.
- **Editor toolchain cost.** Node, esbuild, VS Code packaging, and the headless Electron job add
  a second ecosystem and CI time; versions and the lockfile are pinned and the job is isolated.
- **Compiler discovery.** The extension does not bundle the compiler; users must set `stc.path`
  or have `stc` on `PATH`. The error path must be clear.
- **Debug safety.** Write/force is opt-in, the non-safety notice is shown, and the extension
  never presents debugging as a safety feature.
- **Single-session lifecycle.** Swaps end sessions; the adapter reports termination and the UI
  must not pretend breakpoints or forces persist.

### 11.2 Resolved questions (2026-10-29)

1. Use the `stc-ide`/`stc-lsp`/`stc-dap` crate split and the `stc lsp`/`stc dap` commands.
2. Enumerate locals with a versioned `stc-debug/1.1` `frame.locals` operation, not adapter-side
   sidecar reading.
3. Hover covers identifiers and completion includes one-level member completion; expression
   typing is deferred.
4. The editor analyzes root-package test sources so test files resolve as in `stc test`.
5. `stc dap launch` builds by default with a `noBuild` opt-out.
6. The extension supports the pinned language client's VS Code minimum (currently 1.82).
7. Distribution is VSIX-only in v2.0.0.
8. The compiler is resolved from `stc.path` or `PATH`, not bundled.
9. Debug variables are read-only unless `allowWrite` is enabled; forces additionally require
   `allowForce`.
10. Source paths map automatically from the project graph, with an optional `sourceRoot` override.
11. CI includes a headless `@vscode/test-electron` smoke job.
12. The language id is `st` with the `source.st` TextMate scope.

### 11.3 Remaining implementation defaults

I-11 (sample workspace path), I-13 (grammar coverage check), and I-15 (documentation set) are
defaults rather than owner decisions; they may be adjusted during implementation without a scope
change.

## 12. Change log

| Date | Change |
| --- | --- |
| 2026-10-29 | Started the WL-1.1 sub-plan. Recorded the debug-protocol, driver, CST/sema, CLI, packaging, and CI findings; proposed the IDE API, crate, extension, capability, and verification decisions; opened the review questions. |
| 2026-10-29 | Locked I-1–I-10: three-crate split with `stc lsp`/`stc dap`, versioned `stc-debug/1.1` `frame.locals`, identifier-level hover/completion, test sources in editor analysis, build-on-launch with `noBuild`, the language-client VS Code floor, VSIX-only distribution, and automatic project-graph source mapping. |
| 2026-10-29 | Locked the remaining decisions: read-only variables with `allowWrite`/`allowForce`, a headless `@vscode/test-electron` CI smoke, and the `st`/`source.st` language id. |
