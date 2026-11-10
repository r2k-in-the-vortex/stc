# WL-18 — Language-server rename (F-19)

Status: **Plan for owner approval; Phase 0 questions open** (2026-11-09). No implementation has
started.

- Branch: `WL-18`
- Base: `v3.0.0-planning` at `e231697`
- Locked scope source: `v3.0.0-plan.md` §WL-18 and `FutureWishlist.md` §WL-18
- Delivery process: `workflows/FEATURE-DEVELOPMENT.md`
- Owner decision (2026-11-09): expand WL-18 from a source-only editor rename to a **tracked**
  editor rename that appends `stc.renames.toml` only after the client confirms the edit was
  applied.

## 1. Goal

Add `textDocument/prepareRename` and `textDocument/rename` to `stc-lsp`, backed by the compiler's
reference resolution, so Rename (F2) works in VS Code instead of doing nothing. The language
server reuses the same resolution data as `stc rename` and the existing definition/references
queries; it does not build a second symbol graph.

The owner expanded the goal so a VS Code rename also preserves `GotId` history for local
`stc run --watch` and remote online change. The standard LSP methods remain source-only for
clients without the tracked handshake; the VS Code extension uses a private request pair to
record history only after it has confirmed that every edited file contains the expected text.

## 2. Scope

- Advertise `renameProvider` with `prepareProvider`; resolve the symbol at the cursor and return
  a `WorkspaceEdit` that renames every resolved reference.
- Reuse the compiler's reference resolution and rename history; do not build a second symbol
  graph in the language server.
- Handle the symbol kinds the compiler already supports; reject unsupported symbols with a
  clear message rather than a silent no-op.
- Reopen the closed language-server capability set for rename only; all other capabilities stay
  closed.
- Add the tracked editor rename:
  - advertise `experimental.stc.trackedRename` and the private `stc/rename` /
    `stc/recordRename` requests;
  - `stc/rename` returns the edit, the canonical history entries, and a single-use token; it
    writes nothing;
  - the VS Code extension applies the edit, verifies the post-apply text of every edited file,
    then calls `stc/recordRename`;
  - the server appends `stc.renames.toml` only after that confirmation.
- Keep the standard `textDocument/rename` source-only and warn for identity-bearing targets so
  other LSP clients cannot silently break online change.
- Update the IDE Integration, CLI and Manifest, Online Change and GOT, USER-GUIDE,
  KNOWN-LIMITATIONS, and VS Code README documentation, plus the capability tests.
- Add multi-file LSP rename tests, prepare/rename negative tests, tracked-protocol tests, an
  editor smoke that applies the rename and checks the history, and a manual online-change
  verification.

## 3. Exclusions and non-goals

- No semantic tokens, code actions, inlay hints, range formatting, call hierarchy, workspace
  symbols, or other LSP features.
- No change to `stc rename` CLI semantics, its history-entry rules, or GOT identity handling.
  The tracked editor rename reuses the same `stc.renames.toml` format and the same
  `stc-manifest` load/append/merge rules.
- No remote editing and no second formatter or type checker.
- No new `STC-*` diagnostic code; language-server errors are JSON-RPC errors with clear
  messages, and history errors reuse the existing `STC-E-PROJECT-012/013` text.
- No new external dependency and no new timing threshold or CI timing gate.
- No manifest, loader, runtime, DAP, or task-provider work.
- No new user setting; the VS Code extension uses the tracked path by default when the server
  advertises it.
- No WL-19+ or other wishlist work on this branch.

## 4. Current-state audit

### 4.1 Language-server surface

- `crates/stc-lsp/src/server.rs:54-72` advertises the closed capability set: diagnostics,
  document symbols, definition, references, hover, completion, and formatting. The comment at
  lines 51-52 and `crates/stc-lsp/src/lib.rs:25-26` explicitly exclude rename.
- Request dispatch is in `server.rs:338-421`; the queries that need a fresh analysis are listed
  at `server.rs:376-384`, and each handler is a small `*_result` method (for example
  `references_result` at `server.rs:907-928`). The `initialize` response embeds
  `capabilities()` at `server.rs:456-459`.
- `server.rs:1361-1444` asserts the exact capability key set and is the test that must change.
- Shared helpers already exist: `query_file` (`server.rs:855`), `query_offset` (`server.rs:881`),
  `location_json`, and `range_for_span` (byte spans to UTF-16 ranges). `protocol.rs` defines the
  JSON-RPC error constants; there is no `RequestFailed` (-32803) constant yet.
- `crates/stc-ide/src/snapshot.rs:60-72` exposes `SourceFileInfo.package` and `is_test`, so the
  language server can tell root-package files from dependency files. The snapshot includes every
  package's production sources and the root package's test sources
  (`snapshot.rs:184-188`, `snapshot.rs:338-343`). `Snapshot::manifest_path` (`snapshot.rs:334`)
  gives the server the root directory for `stc.renames.toml`.
- The server is single-threaded per connection, so a pending tracked-rename token needs no lock.

### 4.2 Existing rename data and CLI behavior

- `stc-ide` already resolves a cursor to a declaration identity for definition and references:
  `Target` (`query.rs:31-40`), `target_at` (`query.rs:166-191`), `references`
  (`query.rs:132-148`), `collect_references` (`query.rs:198-223`), `declaration_span`
  (`query.rs:233-241`), and `target_declaration` (`query.rs:243-294`). This is the same
  resolution map the CLI uses, and the module docs say so (`query.rs:1-8`).
- `stc rename` (`crates/stc-driver/src/rename.rs`) resolves a dotted key in the root package,
  then edits the declaration, every sema-resolved reference, and qualified path prefixes for
  POU/type/namespace targets. Supported targets are `Def`, `Field`, `Param`, `Local`, and
  `EnumValue` (`rename.rs:37-44`); path-prefix kinds are listed at `rename.rs:466-482`;
  reference edits at `rename.rs:630-681`; prefix edits at `rename.rs:683-723`; overlap rejection
  at `rename.rs:732-740`; file writes at `rename.rs:772-803`.
- The CLI validates a new name as one identifier token that is not a reserved keyword
  (`rename.rs:333-341`), allows case-only changes (`rename.rs:101-105`), and appends
  `stc.renames.toml` only after every write succeeds (`rename.rs:286-311`). D-156
  (`specification/Decisions.md:624`) and D-169 (`Decisions.md:637`) fix the CLI contract;
  renaming a dependency's declaration or a package name is out of scope.
- The CLI records the **user-supplied** dotted keys, including parameters and locals
  (`rename.rs:286-297`). A full-path invocation such as `stc rename AddStep Advance` or
  `stc rename Accumulator.increment Accumulator.delta` records the canonical full paths.
  Builds uppercase both sides before feeding the identity engine (`build.rs:695-707`).
- The IDE `DeclarationIndex` is token-keyed (`query.rs:47-111`), which is exactly what a
  cursor-based rename needs. The CLI's index is dotted-key-keyed (`rename.rs:343-457`); the two
  serve different entry points over the same analysis.
- `stc-manifest` owns the history format: `load_rename_history`, `append_rename`,
  `render_rename_history`, and `merge_renames` (`crates/stc-manifest/src/renames.rs:84-302`).
  `append_rename` rejects an exact duplicate (`STC-E-PROJECT-013`); `merge_renames` uppercases
  keys, composes chains oldest-first, and rejects a mapping that targets itself.
- `stc run --watch` watches the root `stc.renames.toml` (`watch.rs:589-618`), so a history write
  after a source apply triggers the corrective rebuild. The watch's child build merges the
  history (`project.rs:428-438`).

### 4.3 What is missing

- No `prepareRename` or `rename` query in `stc-ide`; no capability advertisement, handler, or
  error mapping in `stc-lsp`.
- No root-package restriction and no path-prefix edit collection in `stc-ide`.
- No public new-name validation helper in `stc-ide` (the CLI helper is private).
- No tracked-rename protocol, pending token, or history write in `stc-lsp`.
- No VS Code rename middleware or post-apply confirmation controller.
- No multi-file LSP rename test, negative protocol tests, tracked-protocol tests, or editor
  rename smoke.
- The user-facing docs still say rename is not offered (see 4.4).

### 4.4 Documentation and test surface

- `specification/Architecture/IDE-Integration.md` §3.2 capability map (lines 204-216) and §3.3
  closed unsupported list (lines 222-228).
- `specification/Architecture/CLI-and-Manifest.md` §3.1 rename history (lines 1074-1105).
- `specification/Architecture/Online-Change-and-GOT.md` §7.3 identity and history
  (lines 303-311).
- `USER-GUIDE.md` §8.3 language features (lines 406-417).
- `KNOWN-LIMITATIONS.md` §4 capability row (line 95).
- `editors/vscode/README.md` language-features paragraph (lines 67-76).
- Tests: `crates/stc-ide/tests/ide.rs` and `navigation.rs` (multi-file navigation),
  `crates/stc-driver/tests/lsp_cli.rs` (real binary, framed JSON-RPC),
  `crates/stc-lsp/src/server.rs` capability test, the extension unit tests under
  `editors/vscode/test/*.test.mjs`, and the headless VS Code smoke
  (`editors/vscode/test/electron/suite.js:208-256`, `637-687`).

### 4.5 Why a source-only rename is not enough

A source-only editor rename changes identifiers but leaves `stc.renames.toml` untouched. Both
online-change paths build against the active baseline and merge that history, so the compiler
sees the old identity removed and the new identity added, with no tracked mapping:

- Local `stc run --watch` rebuilds by spawning `stc build` against the store's active baseline
  (`watch.rs:316-425`); the child build merges `stc.renames.toml` and CLI `--rename`
  (`project.rs:428-438`) but watch forwards neither `--rename` nor `--allow-untracked-removal`
  to the child.
- Remote online deploy builds the candidate on the host with
  `stc build --got-manifest <active>` (`remote_launch.rs:754-819`); the source mirror includes
  `stc.renames.toml` (`remote.rs:803-917`), and no untracked-removal option is passed.

The resulting behavior depends on the renamed identity:

| Renamed symbol | Source-only local watch | Source-only remote online | Tracked editor rename |
| --- | --- | --- | --- |
| Data root (global/retained variable, program/FB instance, other root) | Build fails `STC-E-MANIFEST-003`; watch keeps the active generation. With `--allow-untracked-removal` the old root is dropped/quarantined (`STC-W-ONLINE-001`, `STC-W-MANIFEST-002`), its state is discarded, and the new root gets a fresh `GotId`. Watch does not forward the flag. | Candidate build fails `STC-E-MANIFEST-003`; deploy fails and the host keeps the active generation. The candidate build cannot pass `--allow-untracked-removal`. | `old -> new` history preserves the `GotId`; the online plan can preserve the root and its state. |
| Field of a persistent type/FB | Layout hash changes; without a tracked field rename `fields_match_under_renames` fails (`engine.rs:1786-1823`, `1828-1870`), the plan is `reject.layout`, and the build fails with `STC-E-ONLINE-*`. | Same rejection in the candidate build or in `online_possible`; deploy fails. | The field rename entry maps the old field path; the layout matches and the plan can be online. |
| Type used by a persistent root | Type key changes, so the root is `reject.layout`; the build fails. | Same. | The type rename entry preserves the type key and the root identity. |
| Function, method, enum value | Old identity is dropped/retired and a new one allocated; an online candidate can deploy when no export or layout is affected, but the identity is not preserved. An exported symbol removal is `STC-E-ONLINE-002`. | Same. | The history entry preserves the identity. |
| Parameter, local, configuration | No GOT identity; the source change is safe for online change. | Safe. | Source-only by design; no history entry. |

Two consequences follow. First, a source-only editor rename is **not online-change-safe** for
stateful or layout-bearing symbols; it fails visibly rather than silently for the rest. Second,
`[online].allow-untracked-removal` is parsed (`model.rs:465-482`) but not wired into the driver
build/watch/remote paths; only the CLI flag sets `Request.allow_untracked_removal`, and the watch
and remote candidate builds do not forward it. That is an existing gap outside WL-18, but it
means the manifest setting cannot rescue an untracked editor rename.

After a source-only editor rename, `stc rename Old New` cannot repair the history because the old
name no longer resolves in source. The recovery is to add the `[[rename]]` entry (old key to new
key) to `stc.renames.toml` by hand before the next online build, or to revert the source and use
`stc rename`. The tracked editor rename exists to avoid that trap for VS Code users; the
standard LSP method keeps the visible warning for every other client.

### 4.6 Discovered CLI edge: case-only renames

While validating the history rules, a case-only rename was run on the sample workspace:

```text
stc rename AddStep addstep
```

The CLI accepts it (`rename.rs:101-105`) and writes:

```toml
schema = 1

[[rename]]
old = "AddStep"
new = "addstep"
```

The next build fails `STC-E-PROJECT-013: rename 'AddStep' targets itself` because
`merge_renames` uppercases both sides and sees `ADDSTEP -> ADDSTEP` (`renames.rs:283-289`).
This is an existing CLI defect, not a WL-18 regression. The tracked LSP must not reproduce it:
GOT keys are ASCII-uppercase, so a case-only source rename changes no identity and writes no
history entry. The plan records the CLI defect as a follow-up (Q-3f); fixing the CLI is outside
WL-18 unless the owner expands the scope.

### 4.7 VS Code client surface

- `vscode-languageclient` 9.0.1 exposes `RenameMiddleware.provideRenameEdits` and
  `prepareRename` (`editors/vscode/node_modules/vscode-languageclient/lib/common/rename.d.ts`).
  The rename feature calls the middleware with `(document, position, newName, token, next)`.
- `LanguageClient.protocol2CodeConverter.asWorkspaceEdit` converts the private response's LSP
  `WorkspaceEdit` into a real `vscode.WorkspaceEdit`, so the middleware can return it to VS Code.
- `@types/vscode` 1.82 has no `workspace.onDidApplyEdits`. The controller must observe
  `workspace.onDidChangeTextDocument` and the existing `**/*.st` file watcher, then read only
  the edited files and compare their text with the expected post-edit text.
- `workspace.applyEdit` is available for the smoke, which applies the returned edit and then
  checks the history file. The extension has no public command to drive the interactive rename
  input box.

## 5. Phase 0 — Close owner questions and record decisions

### 5.1 Owner questions

Each question has a recommendation. The owner may accept the recommendation or choose the
alternative; the answers are recorded in `specification/Decisions.md` as D-253 before Phase 1
starts.

**Q-1. Which symbol kinds does the first slice cover?**

Recommendation: every kind `stc rename` already supports — definitions (functions, function
blocks, programs, classes, interfaces, methods, structs, enums, named values, subranges, aliases,
namespaces, globals, configurations), fields, parameters, locals, and enum values, including the
path-prefix kinds. Reject standard-library functions (`Target::StdFunction`) and built-in types
(`Target::Type` that is not a named definition) with a visible error. This keeps CLI and LSP
behavior aligned and avoids a second "supported kinds" table.

**Q-2. May the editor rename a dependency's declaration?**

Recommendation: no. Match D-156: only declarations owned by the root package are renameable.
`SourceFileInfo.package` plus a root-package accessor gives the check. Root test sources are
included, matching the CLI. A dependency declaration request returns a visible error.

**Q-3. Is the editor rename tracked?**

Recommendation: yes, as the owner selected. The standard LSP methods stay source-only; the VS
Code extension uses `stc/rename` and `stc/recordRename`. `stc/rename` computes the edit and the
canonical history entries but writes nothing. The extension applies the edit, verifies every
edited file's post-apply text, then calls `stc/recordRename`; the server appends the pending
entries. This is a safe-apply handshake, not a request-time write: a cancel, undo, partial
apply, or failed verification leaves history unrecorded. §4.5 is the evidence for why this is
needed.

**Q-3a. Which targets write history?**

Recommendation: only identity-bearing root-package declarations:

- `Target::Def` kinds `Namespace`, `Alias`, `Subrange`, `Enum`, `NamedValue`, `Struct`,
  `Function`, `FunctionBlock`, `Program`, `Class`, `Interface`, `Method`, and `Global`;
- `Target::Field`;
- `Target::EnumValue`.

Parameters, locals, and configurations are source-only: they have no GOT key and no online
identity to preserve. A case-only rename writes no history because GOT keys are uppercase and a
case-only source change cannot change an identity (this also avoids the CLI defect in §4.6).
A tracked target whose canonical path cannot be computed (for example the root namespace) is
rejected with a clear message rather than silently skipping history. Standard functions,
built-in types, and dependency declarations are rejected.

**Q-3b. What happens on cancel, undo, partial apply, or a failed history write?**

Recommendation: the extension records only when every edited file's current text equals the
expected post-edit text. Cancel or undo before confirmation leaves the text at the original
value, so no `recordRename` is sent and no history is written. A partial apply leaves at least
one file at the original or an intermediate value, so verification fails and the extension shows
a warning naming the unrecorded rename and the recovery (revert and use `stc rename`, or add the
`[[rename]]` entry by hand). A failed history write (conflict, malformed file, or I/O) returns
`RequestFailed`; the extension warns with the server's message and the source stays applied.
The pending token is single-use and expires after five minutes. If the server or extension
restarts before confirmation, the token is lost and no history is written; the docs state the
recovery. The watch may attempt one rebuild between the source apply and the history write; it
keeps the old generation, and the history write triggers the corrective rebuild because the
watch already watches `stc.renames.toml`.

**Q-3c. How are history conflicts handled?**

Recommendation: reload `stc.renames.toml` at `recordRename` time, not at `rename` time, so a
concurrent CLI write is not overwritten. Use the existing `append_rename` rules: an exact
duplicate or a conflicting chain is `STC-E-PROJECT-013`; a malformed file is
`STC-E-PROJECT-012`. Return `RequestFailed` with the existing code and message; do not merge or
overwrite silently. The source stays applied and the warning gives the recovery. A case-only
rename writes no entry, so it cannot create a self-conflict.

**Q-3d. Standard method or tracked method?**

Recommendation: keep `textDocument/rename` as the standard source-only method for every client.
For an identity-bearing target it sends one `window/showMessage` warning that no history was
written and names the required `[[rename]]` entry. Advertise `experimental.stc.trackedRename:
true` and the private `stc/rename`/`stc/recordRename` pair. The VS Code extension uses the
tracked path by default when the capability is present; there is no user setting in WL-18. This
keeps other editors working and keeps the source-only trap visible.

**Q-3e. What is the older-server fallback?**

Recommendation: if the capability is absent or `stc/rename` returns MethodNotFound (-32601), the
middleware calls `next()` (the standard rename) and shows one warning per session that history
was not updated. Other errors are rethrown so VS Code shows the server's message. This keeps a
new extension usable with an older `stc` binary.

**Q-3f. Do we fix the case-only CLI defect in WL-18?**

Recommendation: no. The tracked LSP writes no history for case-only renames, so the feature is
correct without touching the CLI. Record the defect in this plan and add a `FutureWishlist.md`
entry at closeout: `merge_renames` should skip an entry whose `old` and `new` are equal after
uppercase normalization. If the owner wants it fixed in WL-18, it is a small `stc-manifest`
change plus a test, but it changes CLI history handling and therefore expands the locked scope.

**Q-4. What does `prepareRename` return for a rejection?**

Recommendation: return `{ range, placeholder }` for a renameable identifier; return `null` for a
position that is not an identifier or has no resolution (VS Code shows its generic "cannot
rename" message); return a JSON-RPC `RequestFailed` (-32803) error with a clear message for a
resolved but unsupported target (standard function, built-in type, dependency declaration, root
namespace). The `rename` request repeats the validation and returns `RequestFailed` for an
invalid new name or an unsupported target. This gives unsupported symbols a visible reason
instead of a silent no-op.

**Q-5. How are ambiguous or unresolved references handled?**

Recommendation: the cursor resolution is unique by construction: the innermost node with a
recorded resolution, then the token-matching rule used by definition/references. An unresolved
identifier is rejected; no separate ambiguity policy is needed. References are collected from
every resolution with the same identity, using the existing rule that picks the identifier
token spelling the declared name inside each resolved node (the same rule as the CLI). Files
outside the project source set are never edited; a request from such a file is rejected by the
existing `query_file` check.

**Q-6. Is a workspace-edit size limit needed?**

Recommendation: no hard limit in WL-18. The edit is proportional to the project the user already
opened, and a cap could block a legitimate rename. Add a multi-file test large enough to prove
the edit spans files, but do not add a threshold or timing gate. If the owner prefers a guard,
propose 10,000 edits with a clear error and a test.

**Q-7. Does the CLI and LSP share one rename operation?**

Recommendation: implement the cursor-based engine in `stc-ide` over the existing analysis, keep
the CLI's dotted-key entry point and history behavior unchanged, and add a parity test in
`crates/stc-driver/tests/lsp_cli.rs` that runs `stc rename` on one temp copy and the tracked LSP
flow on another, then compares the source files and the identity-bearing history entries. This
satisfies "reuse the resolution, do not build a second symbol graph" without destabilizing the
CLI. The parity test does not compare history for parameters/locals (the CLI records them; the
LSP intentionally does not) or for case-only renames (the CLI defect in §4.6); those cases
compare source text only.

**Q-8. What is the editor smoke and manual gate?**

Recommendation: extend the headless VS Code smoke to call
`vscode.executeDocumentRenameProvider` on `AddStep`, apply the returned edit with
`vscode.workspace.applyEdit`, poll for `stc.renames.toml` with `old = "AddStep"` and
`new = "Advance"`, and assert both `src/main.st` and `tests/smoke.st` contain the new name. The
smoke restores the sample workspace and deletes the history file in a `finally` block. The
owner's manual check is:

1. F2 in VS Code on the sample workspace: apply the rename, confirm every file updates,
   diagnostics stay clean, and the history entry appears; revert and delete the history.
2. F2 on an unsupported symbol (for example a standard function) and confirm a visible message.
3. A live `stc run --watch` on a copy of `examples/online-change`: rename
   `Counter.Retained` to `Counter.Persistent` through the editor, confirm the watch deploys
   without `STC-E-MANIFEST-003`/`STC-E-ONLINE-*`, and confirm the root's `GotId` is unchanged in
   `stc online download` before and after.
4. A negative control on a fresh copy: apply the same source edit without history and confirm
   the watch rejects the candidate and keeps the old generation.

The manual check is required before the PR (workflow §8).

**Q-9. Which durable decision and documentation changes are recorded?**

Recommendation: add D-253 (tracked LSP rename policy) covering Q-1 through Q-8. Update IDE
Integration §3.2/§3.3, CLI and Manifest §3.1, Online Change and GOT §7.3, USER-GUIDE §8.3,
KNOWN-LIMITATIONS §4, and the VS Code README. The v3.0.0 plan and wishlist are updated at
closeout, not in Phase 0.

### 5.2 Proposed decision D-253

> LSP rename uses the compiler's resolution data in `stc-ide` and advertises `renameProvider`
> with `prepareProvider`, plus `experimental.stc.trackedRename` and the private `stc/rename` and
> `stc/recordRename` requests. Only root-package declarations are renameable; supported targets
> are the definition, field, parameter, local, and enum-value kinds that `stc rename` supports,
> including qualified path-prefix kinds. Standard functions, built-in types, dependency
> declarations, and a tracked target without a canonical key are rejected with a JSON-RPC
> `RequestFailed` message. `prepareRename` returns the identifier range and placeholder for a
> renameable target, `null` for a non-identifier or unresolved position, and `RequestFailed` for
> a resolved but unsupported target. The standard `textDocument/rename` validates the new
> spelling with the lexer (one identifier, not a reserved keyword), allows case-only changes,
> collects the declaration, every resolved reference, and qualified path-prefix edits,
> deduplicates them, rejects overlaps, returns a `changes` `WorkspaceEdit`, and writes no
> history. For an identity-bearing target it also sends one `window/showMessage` warning naming
> the required history entry. The private `stc/rename` returns the same edit plus canonical
> root-package history entries and a single-use token; it writes nothing. `stc/recordRename`
> appends the pending entries to `stc.renames.toml` with the existing `stc-manifest`
> load/append/render rules only after the client confirms the edit was applied. History is
> written only for identity-bearing targets: namespaces, aliases, subranges, enums, named
> values, structs, functions, function blocks, programs, classes, interfaces, methods, globals,
> fields, and enum values. Parameters, locals, configurations, and case-only renames are
> source-only. The pending token is single per connection, latest wins, and expires after five
> minutes. The VS Code extension verifies every edited file's post-apply text before recording;
> cancel, undo, partial apply, a failed verification, or a failed history write leaves history
> unrecorded and surfaces a warning with the recovery. If the capability or method is absent, the
> extension falls back to the standard source-only method and warns once. The server writes only
> `stc.renames.toml`, never source. There is no edit-size cap and no new `STC-*` code.

### 5.3 Phase 0 exit criteria

- Every question above has an owner answer recorded in this plan.
- D-253 is written in `specification/Decisions.md` and the affected docs list is fixed.
- No implementation starts before the plan and Phase 0 answers are approved.
- The plan is committed on `WL-18` and CI is green (docs-only change).

## 6. Phase 1 — Rename engine and history model in `stc-ide`

### 6.1 Goal

Add a pure, cursor-based rename engine over the existing analysis so the language server and
tests share one implementation, with no protocol code in `stc-ide`. The engine returns the edit
set, the target classification, and the canonical history entries; it does not write files.

### 6.2 Work

1. Share the two helpers that would otherwise drift:
   - move the CLI's `def_path` (`rename.rs:572-595`) to `stc-hir` as a public
     `HirCrate::def_path(def, namespace) -> Option<Vec<String>>`, with tests for root and nested
     namespaces; the CLI calls it with no behavior change;
   - add `stc_lexer::is_identifier(text) -> bool` (one identifier token, no errors, not a
     keyword), move the CLI's `valid_identifier` (`rename.rs:333-341`) onto it, and test empty,
     multi-token, keyword, and Unicode cases.
2. Add `crates/stc-ide/src/rename.rs` and export the public types from `lib.rs`:
   - `RenameError` with stable messages for not-an-identifier, unresolved, unsupported symbol,
     dependency declaration, invalid new name, same name, no occurrences, overlapping edits, and
     untracked key.
   - `RenamePlaceholder { span: Span, name: String }`.
   - `RenameEdit { file: FileId, start: u32, end: u32, new_text: String }` (byte offsets).
   - `RenameHistoryEntry { old: String, new: String }` (canonical root-package paths).
   - `RenameTargetKind { Tracked, SourceOnly }` and
     `RenameResult { edits: Vec<RenameEdit>, history: Vec<RenameHistoryEntry>, target: RenameTargetKind }`.
   - `Snapshot::prepare_rename(file, offset) -> Result<RenamePlaceholder, RenameError>`.
   - `Snapshot::rename(file, offset, new_name) -> Result<RenameResult, RenameError>`.
3. Reuse `target_at`, `target_declaration`, `declaration_span`, `collect_references`, and the
   resolution map. Change the needed `query.rs` helpers to `pub(crate)` or add `pub(crate)`
   wrappers; do not widen the crate's public API beyond the rename types. Add
   `Snapshot::is_root_package(file)` backed by `PackageData.is_root` and
   `Snapshot::root_namespace()`.
4. Validate the new name with `stc_lexer::is_identifier`; allow case-only changes and reject an
   identical spelling. Reject a dependency declaration before collecting edits.
5. Collect the declaration edit, every reference edit, and the qualified path-prefix edits for
   `is_path_prefix_kind`; sort, deduplicate, and reject overlaps. The reference token rule must
   match the CLI (`collect_references` already does; prefix edits use the HIR parent chain and
   `PathExpr`/`Path` windows, as in `rename.rs:683-723`).
6. Compute the history entry and target classification:
   - `Target::Def` of a tracked kind: `def_path`; new path = old path with the last segment
     replaced by the new name.
   - `Target::Field`: `def_path(field.owner) + field name`; new path replaces the last segment.
   - `Target::EnumValue`: `def_path(enum) + value name`; new path replaces the last segment.
   - `Target::Param`, `Target::Local`, and `Target::Def(Configuration)`: `SourceOnly`, empty
     history.
   - Case-only rename: empty history.
   - Tracked target with no canonical path (root namespace): `RenameError::UntrackedKey`.
   - Standard function and built-in type: `RenameError::Unsupported`.
7. Add unit and integration tests in `stc-ide`:
   - prepare/rename on a function, FB type, field, parameter, local, enum value, namespace or
     other path-prefix kind, program field, and global;
   - multi-file references (sample workspace `src/main.st` + `tests/smoke.st`; motor example
     `motor` + `motorlib`);
   - case-only rename: edits present, history empty;
   - invalid name, reserved keyword, same name, unresolved position, non-identifier position;
   - standard function, built-in type, dependency declaration, and root-namespace rejection;
   - overlap and duplicate-edit handling;
   - history keys: function `AddStep -> Advance`, field
     `Accumulator.increment -> Accumulator.delta`, enum value, namespace prefix;
   - parameter/local/configuration history empty.
8. Do not add a `stc-driver` dependency edge. The CLI parity comparison lives in the Phase 2
   `lsp_cli` test, which already runs the real binary.

### 6.3 Exit criteria

- The engine returns correct declaration + reference + prefix edits for every supported kind.
- The engine returns canonical history entries only for tracked targets and never writes files.
- Every rejection path has a stable, tested message.
- No `stc-lsp` or extension code is added in this phase.
- `cargo test -p stc-ide`, `cargo test -p stc-hir`, `cargo test -p stc-lexer`,
  `cargo test -p stc-driver`, clippy, and fmt pass; the phase commit is pushed and CI is green.

### 6.4 Evidence

`stc-ide`, `stc-hir`, and `stc-lexer` test output, the CLI regression test result, and the phase
commit/CI link recorded in §13.

## 7. Phase 2 — LSP standard and tracked rename

### 7.1 Goal

Expose the engine through the language server, update the closed capability set for rename only,
and implement the private tracked-rename handshake and history write.

### 7.2 Work

1. Add `REQUEST_FAILED` (-32803) to `crates/stc-lsp/src/protocol.rs`.
2. Advertise in `capabilities()`:
   - `"renameProvider": { "prepareProvider": true }`;
   - `"experimental": { "stc": { "trackedRename": true } }`;
   - keep every other capability unchanged and update the comment that excludes rename.
3. Add to the dispatch and the `flush_analysis` list:
   - `textDocument/prepareRename` and `textDocument/rename`;
   - `stc/rename` (needs a fresh analysis);
   - `stc/recordRename` (does not need analysis; it only writes history).
   Use a `Result<Value, String>` handler shape so an `Err` becomes a `RequestFailed` response;
   the existing `Value` handlers stay as they are.
4. Standard handlers:
   - `prepare_rename_result`: `Ok` -> `{ range, placeholder }`; `NotAnIdentifier`/`Unresolved`
     -> `null`; other errors -> `RequestFailed`.
   - `rename_result`: call `Snapshot::rename`, map `Ok` to
     `{ "changes": { "<uri>": [ { "range": ..., "newText": ... } ] } }`, and map errors to
     `RequestFailed`. Reuse `range_for_span` and `path_to_uri`.
   - After a successful standard rename of a tracked target, send one `window/showMessage`
     warning (type 2) naming the old/new keys and the recovery. A source-only target sends no
     warning.
5. `stc/rename` handler:
   - read `textDocument`, `position`, and `newName`;
   - call `Snapshot::rename`;
   - build the `changes` edit;
   - if `history` is non-empty, generate a token, replace any pending record, and store
     `PendingRename { token, entries, created }`;
   - return `{ "edit": { "changes": ... }, "history": [...], "token": "..." }`; omit `token`
     when `history` is empty.
   - The handler writes nothing.
6. `stc/recordRename` handler:
   - read `token`;
   - reject a missing, stale, expired, or already-used token with `RequestFailed`;
   - take the pending entries, reload `stc.renames.toml` from the root directory
     (`Snapshot::manifest_path().parent()`), append each entry, render, and write;
   - clear the pending record on success and on failure;
   - return `{ "recorded": true, "entries": N, "path": "<absolute path>" }`.
   - The server writes only `stc.renames.toml`, never source.
7. Token rules: one pending record per connection; a new `stc/rename` replaces it and
   invalidates the old token; a token is single-use; a five-minute TTL is enforced (add a
   `rename_token_ttl` field to `ServerOptions` so the unit test can use a short TTL while the CLI
   keeps the default). Generate the token from the process id and a monotonically increasing
   counter; it is opaque to the client.
8. Add `stc_manifest::append_rename_file(path, old, new)` that loads, appends, renders, and
   writes the history with the same codes and messages as the CLI. Switch
   `crates/stc-driver/src/rename.rs` to it with no behavior change. Add unit tests for missing
   files, duplicates, malformed files, and write failures. (The write uses `std::fs::write`,
   matching the CLI; an atomic history write is a separate follow-up.)
9. Update the capability tests:
   - `crates/stc-lsp/src/server.rs` exact capability key set and
     `experimental.stc.trackedRename`;
   - `crates/stc-driver/tests/lsp_cli.rs` capability assertions.
10. Add `lsp_cli` tests:
    - multi-file standard rename over a temp project (declaration + references in `src/` and
      `tests/`), with exact URIs, ranges, and new text;
    - `prepareRename` range/placeholder;
    - invalid name, reserved keyword, standard function, dependency declaration, unresolved
      position, non-identifier position, and root namespace;
    - case-only rename: edits present, no history;
    - the identity-sensitive `window/showMessage` for a global rename and no warning for a
      local rename;
    - a request from a file outside the source set is rejected;
    - unsupported methods still return method-not-found;
    - `stc/rename` returns the edit, canonical history, and token for a function and a field;
      no token for a parameter/local/configuration; case-only history empty;
    - `stc/recordRename` writes the history, rejects stale/expired/already-used tokens, and
      reports malformed/conflicting history with `RequestFailed`;
    - applying the `stc/rename` edits and calling `stc/recordRename` on temp copy B produces the
      same source and identity-bearing history as `stc rename` on temp copy A (the parity test);
    - the standard `textDocument/rename` does not write history.
11. Keep the server's existing behavior for open buffers, overlays, debounce, and UTF-16
    positions; use the analyzed snapshot text for ranges.

### 7.3 Protocol shapes

```json
// stc/rename params
{
  "textDocument": { "uri": "file:///project/src/main.st" },
  "position": { "line": 10, "character": 8 },
  "newName": "Advance"
}

// stc/rename result (token omitted when history is empty)
{
  "edit": {
    "changes": {
      "file:///project/src/main.st": [
        { "range": { "start": { "line": 10, "character": 2 },
                     "end": { "line": 10, "character": 9 } },
          "newText": "Advance" }
      ]
    }
  },
  "history": [ { "old": "AddStep", "new": "Advance" } ],
  "token": "12345-7"
}

// stc/recordRename params
{ "token": "12345-7" }

// stc/recordRename result
{ "recorded": true, "entries": 1, "path": "/project/stc.renames.toml" }
```

Errors use JSON-RPC `RequestFailed` (-32803) with the engine's stable message or the
`stc-manifest` `STC-E-PROJECT-012/013` text.

### 7.4 Exit criteria

- F2 rename works over a multi-file project for every supported kind.
- `prepareRename` returns a clear range or rejection; unsupported symbols produce a visible
  message, not a silent no-op.
- The standard method is source-only and warns for tracked targets.
- The private method returns the edit and history without writing; `recordRename` appends only
  after the token is confirmed and is single-use.
- Only rename is added to the capability set; all other unsupported capabilities remain
  unadvertised.
- `cargo test -p stc-lsp`, `cargo test -p stc-manifest`,
  `cargo test -p stc-driver --test lsp_cli`, clippy, and fmt pass; the phase commit is pushed
  and CI is green.

### 7.5 Evidence

`lsp_cli` output with exact edits and history, capability test output, the parity test result,
and the phase commit/CI link.

## 8. Phase 3 — VS Code tracked rename controller

### 8.1 Goal

Make the VS Code extension use the tracked path: request the edit and token, return the edit to
VS Code, confirm the post-apply text, and record the history. Fall back safely to the standard
method when the server does not support the handshake.

### 8.2 Work

1. Add a pure module `editors/vscode/src/renameEdits.ts`:
   - `applyLspEdits(text, edits)` converts LSP line/character ranges to UTF-16 offsets, applies
     the edits in descending order, and throws on an overlap;
   - `shouldUseTrackedRename(initializeResult)` returns true only for
     `capabilities.experimental.stc.trackedRename === true`;
   - no `vscode` import, so it is unit-testable with the existing esbuild test harness.
2. Add `editors/vscode/src/trackedRename.ts` with `TrackedRenameController`:
   - one controller per `ClientEntry`, holding the manifest path, the output channel, the
     `LanguageClient`, the `**/*.st` watcher, and the pending record;
   - `provideRenameEdits(document, position, newName, token, next)`:
     - if the capability is absent, call `next()` and warn once per session;
     - otherwise send `stc/rename`; on MethodNotFound (-32601), fall back to `next()` and warn
       once; on other errors, rethrow so VS Code shows the message;
     - if the response has no token, return the converted edit;
     - convert the LSP edit with `client.protocol2CodeConverter.asWorkspaceEdit`;
     - for every URI in the edit, open the document, capture its current text as `original`,
       compute `expected` with `applyLspEdits`, and store both;
     - finish or abandon any previous pending record before starting a new one;
     - return the converted edit.
   - change listeners: `workspace.onDidChangeTextDocument` and the `**/*.st` watcher's
     `onDidChange`; debounce a verification pass by 150 ms;
   - verification: open every edited document, compare its text with `expected`; when all match,
     call `stc/recordRename` with the token, clear the pending record, and log the result; when
     the deadline (5 s) passes:
     - all files still equal `original` -> treat as cancel/undo, clear silently;
     - otherwise warn that the rename was applied but not recorded and give the recovery.
   - `finishPending()` verifies and records an existing pending record before a new rename; if
     it cannot be confirmed, it warns and clears it;
   - `dispose()` clears the timer, the listeners, and the pending record.
3. Wire in `editors/vscode/src/languageClient.ts`:
   - create the `**/*.st` watcher once, pass it to `synchronize.fileEvents` and the controller;
   - set `middleware.provideRenameEdits` to the controller's method;
   - store the controller in `ClientEntry` and dispose it in `stopEntry`;
   - the controller receives the client after construction (the middleware closure is called
     only after the client starts).
4. Add `editors/vscode/test/rename-edits.test.mjs`:
   - multi-line edits, multiple edits in one file, edits across files, UTF-16 surrogate pairs,
     descending application, overlap rejection, and an empty edit list;
   - `shouldUseTrackedRename` with present, absent, and false capability shapes.
5. Add the editor smoke to `editors/vscode/test/electron/suite.js`:
   - `assertTrackedRename(document)` calls `vscode.executeDocumentRenameProvider` on the
     `AddStep` call site with `Advance`;
   - assert the returned `WorkspaceEdit` covers `src/main.st` and `tests/smoke.st`;
   - apply it with `vscode.workspace.applyEdit`; poll for `stc.renames.toml` with
     `old = "AddStep"` and `new = "Advance"`;
   - assert both files contain `Advance` and diagnostics stay clean;
   - restore the original text of both files, save, delete `stc.renames.toml`, and wait for
     diagnostics; do the cleanup in a `finally` block so a failure does not leave the sample
     dirty.
   - keep the existing definition and diagnostics assertions.

### 8.3 Controller state machine

1. `provideRenameEdits` starts: finish any previous pending record, then check the capability.
2. Send `stc/rename`; receive `{ edit, history, token? }`.
3. If no token, return the edit and stop (source-only target).
4. Read every edited document; compute `original` and `expected`; store
   `{ token, files, deadline }`.
5. Return the converted edit; VS Code applies it.
6. Change events debounce a verification pass. All files match `expected` -> `recordRename`;
   all still match `original` at the deadline -> silent cancel; otherwise -> warning.
7. `recordRename` succeeds -> log; fails -> warn with the server's message and the recovery.
8. A new rename, a client stop, or a restart clears the pending record; no history is written
   for an unconfirmed rename.

### 8.4 Exit criteria

- A VS Code rename writes `stc.renames.toml` only after the edit is applied and verified.
- A cancel, undo, partial apply, or failed history write leaves the history unchanged and shows
  a warning where the user can see it.
- An older server falls back to the standard source-only method and warns once.
- The smoke applies a rename, sees the history entry, and restores the sample workspace.
- `npm run typecheck`, `npm test`, `npm run compile`, and `npm run test:electron` pass; the phase
  commit is pushed and CI is green.

### 8.5 Evidence

The extension unit-test output, the headless smoke output with the history entry, and the phase
commit/CI link.

## 9. Phase 4 — Documentation, manual verification, PR, closeout

### 9.1 Documentation

- `specification/Architecture/IDE-Integration.md`: add `textDocument/prepareRename` and
  `textDocument/rename` to §3.2, remove rename from §3.3, and document
  `experimental.stc.trackedRename`, `stc/rename`, `stc/recordRename`, the post-apply
  confirmation, the standard source-only warning, the root-package/supported-kind rules, and the
  older-server fallback.
- `specification/Architecture/CLI-and-Manifest.md` §3.1: state that the tracked editor rename
  writes the same `stc.renames.toml` entries as `stc rename`, only after the client confirms the
  edit; parameters, locals, configurations, and case-only renames are source-only; the standard
  LSP method does not write history.
- `specification/Architecture/Online-Change-and-GOT.md` §7.3: say that `stc rename` and the
  VS Code tracked editor rename persist the mappings in `stc.renames.toml`; a source-only
  standard LSP rename does not.
- `USER-GUIDE.md` §8.3: add rename to the language features and remove it from the not-offered
  list; describe the VS Code tracked behavior and the standard source-only caveat.
- `KNOWN-LIMITATIONS.md` §4: replace the closed-capability row with the remaining limitations
  (other editors get the source-only standard method; parameters/locals/configurations do not
  write history; the case-only CLI defect is a follow-up).
- `editors/vscode/README.md`: add rename to the supported features and describe the history
  handshake.
- `specification/Decisions.md`: D-253.
- `FutureWishlist.md` at closeout: record the case-only CLI defect from §4.6.

### 9.2 Verification and PR

1. Run the full local checks: `check_docs.py`, markdownlint, traceability, host audit,
   `git diff --check`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
   --locked -- -D warnings`, `cargo test --workspace --locked`, and the VS Code extension
   tests/smoke.
2. Run the manual walkthrough in Q-8 on the sample workspace and on a copy of
   `examples/online-change` with a live `stc run --watch`. Capture the exact steps, the history
   file, the watch output, and the before/after `GotId` comparison. Run the negative control.
3. Present the evidence to the owner; wait for approval (workflow §8).
4. Open the PR against `v3.0.0-planning` with the summary, changes, verification evidence, CI
   links, and exclusions (standard method source-only, no size cap, rejected kinds, the
   case-only CLI follow-up).
5. Get the PR CI green; the owner merges. After approval, record the merge in
   `v3.0.0-plan.md` and `FutureWishlist.md`, delete the temporary plan, and clean up branches.

## 10. Cross-cutting constraints

- `stc-ide` stays `#![forbid(unsafe_code)]`; no new external dependency.
- No CLI, manifest-format, runtime, DAP, or task-provider behavior change. The
  `append_rename_file` refactor is behavior-preserving for the CLI.
- No new `STC-*` diagnostic code; protocol errors use JSON-RPC `RequestFailed`.
- The capability set stays closed except for rename; `experimental.stc.trackedRename` is the
  only addition.
- The standard `textDocument/rename` is source-only; only the private tracked pair writes
  history.
- The server writes only `stc.renames.toml`, never source.
- One pending tracked-rename token per connection; latest wins; single-use; five-minute TTL.
- No timing threshold or CI timing gate.
- Keep the existing UTF-16 position conversion and analyzed-text rule; do not mix open-document
  and snapshot text for ranges.
- Branch from `v3.0.0-planning`; PR to `v3.0.0-planning`; the owner merges.
- New code targets 95% line coverage; the project gate stays 85%.

## 11. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| CLI and LSP edit collection drift | Parity test in `lsp_cli` against `stc rename` on the same fixture; the CLI remains the behavioral reference |
| Unsaved editor text changes the offsets | The request flushes the debounced analysis and uses the analyzed snapshot text, as definition/references do |
| A dependency declaration is renamed | Root-package check using `SourceFileInfo.package`; negative test |
| Case-insensitive names cause a false no-op | Allow case-only renames; write no history for them; test `AddStep` -> `addstep` |
| Path-prefix edits are missed | Test a namespace/POU path target and compare with the CLI |
| Large multi-file edits | No cap; test a project spanning at least three files |
| VS Code shows a generic message | Use `RequestFailed` for resolved-but-unsupported targets; manual check |
| The standard rename is mistaken for a tracked rename | Source-only warning for tracked targets; docs; D-253 |
| Local watch or remote online change rejects a source-only rename | The failure is visible (`STC-E-MANIFEST-003` or `STC-E-ONLINE-*`) and keeps the active generation; the warning and docs give the recovery; VS Code uses the tracked path |
| A data-root rename deploys with discarded state under `--allow-untracked-removal` | The tracked path preserves the `GotId`; the standard warning and docs state that state is reset |
| `[online].allow-untracked-removal` is not wired into build/watch/remote | Record as an existing gap outside WL-18; do not rely on the manifest setting in the docs |
| The extension verifies a partial apply or a stale buffer | Compare the full expected text of every edited file; no match means no history and a warning |
| A history write fails after the source apply | `RequestFailed` with the `STC-E-PROJECT-012/013` text; the source stays applied; the warning gives the recovery |
| The server or extension restarts before confirmation | The token is lost and no history is written; docs state the recovery; the verification window is short |
| A new rename replaces a pending token | The controller finishes or abandons the previous record before starting a new one |
| The watch rebuilds between the source apply and the history write | The watch keeps the old generation and watches `stc.renames.toml`; the history write triggers the corrective rebuild; the manual test tolerates one transient rejection |
| The CLI case-only defect from §4.6 | The LSP writes no case-only history; record the CLI fix as a follow-up in `FutureWishlist.md` |
| An older server lacks the capability | Fall back to `next()` and warn once per session |
| Multiple workspace folders | One controller and one pending token per language-client connection; the client routes the request by document |
| A concurrent CLI history write | Reload the history at `recordRename` time; a conflict is `STC-E-PROJECT-013`, not a silent overwrite |

## 12. Evidence checklist

- [ ] D-253 recorded and the plan/Phase 0 answers approved.
- [ ] Phase 1 `stc-ide` engine, history-key, and shared-helper tests green.
- [ ] Phase 2 capability, standard rename, tracked-protocol, history-write, and parity tests
      green.
- [ ] Phase 3 extension unit tests, typecheck, compile, and headless smoke green.
- [ ] Local checks (docs, lint, traceability, host audit, diff, fmt, clippy, tests) green.
- [ ] Manual sample and online-change walkthrough evidence approved by the owner, including the
      before/after `GotId` comparison and the negative control.
- [ ] PR against `v3.0.0-planning` green; owner merge recorded.
- [ ] `v3.0.0-plan.md` and `FutureWishlist.md` updated (including the case-only CLI follow-up);
      temporary plan deleted.

## 13. Status log

| Date | Status |
| --- | --- |
| 2026-11-09 | Plan created; Phase 0 questions open; no implementation started. |
| 2026-11-09 | Online-change audit added (§4.5): a source-only rename is not identity-preserving; local watch and remote online change reject data-root, field, and type renames or reset state. |
| 2026-11-09 | Owner selected the tracked-rename scope. Plan rewritten around `experimental.stc.trackedRename`, `stc/rename`/`stc/recordRename`, the post-apply confirmation, canonical history keys, the standard source-only warning, and the older-server fallback. Phase 3 extension work and Phase 4 docs/manual/PR added. |
| 2026-11-09 | Discovered the case-only CLI defect (§4.6): `stc rename AddStep addstep` writes a self-conflicting history entry and the next build fails `STC-E-PROJECT-013`. The tracked LSP will write no case-only history; the CLI fix is proposed as a follow-up (Q-3f). |
