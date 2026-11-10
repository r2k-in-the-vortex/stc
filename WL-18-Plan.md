# WL-18 — Language-server rename (F-19)

Status: **Plan v3 for owner approval; Phase 0 questions open** (2026-11-09). No implementation has
started.

- Branch: `WL-18`
- Base: `v3.0.0-planning` at `e231697`
- Locked scope source: `v3.0.0-plan.md` §WL-18 and `FutureWishlist.md` §WL-18
- Delivery process: `workflows/FEATURE-DEVELOPMENT.md`
- Owner decision (2026-11-09): expand WL-18 from a source-only editor rename to a **tracked**
  editor rename that appends `stc.renames.toml` only after the client confirms the edit was
  applied.
- Owner decision (2026-11-09): **expand WL-18 to edit every external reference**, including
  `[project].configuration`, `[[io.bindings]].variable`, `[[test.case]].name`, machine-profile
  `[[io.routes]]` endpoints, and `VAR_ACCESS`/`VAR_CONFIG` paths. Exact owner wording:
  “Expand WL-18 to edit them, but if the online change ability is too much trouble, then this
  sort of rename will make online change impossible and force next download to be cold. This
  should be case for all other types of renames that might not be implementable for online
  change, it should still work as normal rename”.

## 1. Goal

Add `textDocument/prepareRename` and `textDocument/rename` to `stc-lsp`, backed by the compiler's
reference resolution, so Rename (F2) works in VS Code instead of doing nothing. The language
server reuses the same resolution data as `stc rename` and the existing definition/references
queries; it does not build a second symbol graph.

The rename is **complete first**: it edits the declaration, every resolved source reference,
every qualified path prefix, and every resolved external reference in `stc.toml`, the selected
machine profile, `VAR_ACCESS`, and `VAR_CONFIG`. It never leaves a reference stale.

The rename is **identity best-effort**: when the compiler can preserve the symbol's identity
through `stc.renames.toml`, the VS Code tracked path records it; when it cannot, the rename still
applies as a normal rename and warns that the next online download may be cold and reset retained
state. This applies to every rename type that is not implementable for online change, not only to
external-reference renames.

The standard LSP methods are **history-free** for clients without the tracked handshake: they
return the complete edit but write no history. The VS Code extension uses a private request pair
to record history only after it has confirmed that every edited file contains the expected text.

## 2. Scope

- Advertise `renameProvider` with `prepareProvider`; resolve the symbol at the cursor and return
  a `WorkspaceEdit` that renames every resolved reference.
- Reuse the compiler's reference resolution and rename history; do not build a second symbol
  graph in the language server.
- Edit external references as part of the same `WorkspaceEdit`:
  - `[project].configuration` when a configuration is renamed;
  - `[[io.bindings]].variable` when a global `AT` variable or a direct `AT` program field is
    renamed;
  - `[[test.case]].name` when a test POU is renamed;
  - machine-profile `[[io.routes]]` `plc-input`/`plc-output` `variable` values when a direct
    `%I`/`%Q` variable or program field is renamed;
  - `VAR_ACCESS` paths and `VAR_CONFIG` instance-field paths when a field is renamed.
- Add the same collision check as `stc rename`: reject before editing when the new dotted path
  already names a different declaration in the root package, with
  `STC-E-PROJECT-013` and the CLI message; case-insensitive; same-target case-only renames are
  allowed.
- Fix the `def_path` namespace-name defect while moving the helper to `stc-hir`, so a declaration
  named like the package namespace can be renamed.
- Reopen the closed language-server capability set for rename only; all other capabilities stay
  closed.
- Add the tracked editor rename:
  - advertise `experimental.stc.trackedRename` and the private `stc/rename` /
    `stc/recordRename` requests;
  - `stc/rename` returns the complete edit, the canonical history entries, the online-change
    classification, and a single-use token when history is available; it writes nothing;
  - the VS Code extension applies the edit, verifies the post-apply text of every edited file
    (including `stc.toml` and the profile), then calls `stc/recordRename`;
  - the server appends `stc.renames.toml` only after that confirmation.
- Warn when identity cannot be preserved:
  - the standard method never records history, so an identity-bearing target gets one
    `window/showMessage` warning naming the missing history and the cold-download risk;
  - a configuration rename has no GOT identity and changes task-guard keys, so it warns about a
    possible cold download even on the tracked path;
  - parameters, locals, and case-only renames have no identity change and stay warning-free.
- Keep the project loader, watch, and editor integration consistent with the expanded edit:
  - load and parse the selected machine profile in the IDE snapshot;
  - refresh the language-server snapshot after a manifest/profile edit;
  - watch the selected machine profile in `stc run --watch`.
- Update the IDE Integration, CLI and Manifest, Online Change and GOT, USER-GUIDE,
  KNOWN-LIMITATIONS, and VS Code README documentation, plus the capability tests.
- Add multi-file LSP rename tests, external-reference tests, `VAR_ACCESS`/`VAR_CONFIG` tests,
  prepare/rename negative tests, tracked-protocol tests, an editor smoke that applies the rename
  and checks the history, and a manual online-change verification.

## 3. Exclusions and non-goals

- No manifest-format or schema change. The manifest and profile models gain internal spans only;
  the TOML syntax, keys, validation, and rendered bytes are unchanged.
- No change to `stc rename` CLI semantics, its history-entry rules, or GOT identity handling,
  except the `def_path` bug fix in §4.9. The CLI remains source-only in WL-18 and will leave
  external references stale; the divergence is documented and recorded as a follow-up.
- No rename initiated from a manifest or profile value in WL-18. F2 on
  `[project].configuration`, `[[io.bindings]].variable`, `[[test.case]].name`, or a profile
  `variable` returns `null`; those values are edited as references of a source-initiated rename.
  A follow-up may add manifest/profile entry points.
- No semantic tokens, code actions, inlay hints, range formatting, call hierarchy, workspace
  symbols, or other LSP features.
- No remote editing and no second formatter or type checker.
- No new `STC-*` diagnostic code; language-server errors are JSON-RPC errors with clear
  messages, and history errors reuse the existing `STC-E-PROJECT-012/013` text.
- No new external dependency and no new timing threshold or CI timing gate.
- No new user setting; the VS Code extension uses the tracked path by default when the server
  advertises it.
- No fix for the case-only CLI history defect in §4.5. The tracked LSP writes no case-only
  history; the CLI fix is a follow-up.
- No package-name, dependency-alias, task, resource, program-instance, `VAR_ACCESS` access-name,
  IO-device, or IO-channel rename. Those names are not declaration targets.
- No post-edit full build validation. Source edits get an in-memory semantic reanalysis before
  the edit is returned; manifest and profile edits are consistent by construction because they
  replace a segment that the compiler resolved to the target. The client verification confirms
  that the edit was applied; it does not require the build to succeed, and any residual project
  error stays visible in the editor diagnostics.
- No WL-19+ or other wishlist work on this branch.

## 4. Current-state audit

### 4.1 Language-server surface

- `crates/stc-lsp/src/server.rs` advertises the closed capability set: diagnostics, document
  symbols, definition, references, hover, completion, and formatting. The comment above
  `capabilities()` and `crates/stc-lsp/src/lib.rs` explicitly exclude rename.
- Request dispatch is in `server.rs`; the queries that need a fresh analysis are listed in the
  dispatch, and each handler is a small `*_result` method (for example `references_result`). The
  `initialize` response embeds `capabilities()`.
- The capability test asserts the exact capability key set and is the test that must change.
- Shared helpers already exist: `query_file`, `query_offset`, `location_json`, and
  `range_for_span` (byte spans to UTF-16 ranges). `protocol.rs` defines the JSON-RPC error
  constants; there is no `RequestFailed` (-32803) constant yet.
- `crates/stc-ide/src/snapshot.rs` exposes `SourceFileInfo.package` and `is_test`, so the
  language server can tell root-package files from dependency files. The snapshot includes every
  package's production sources and the root package's test sources. `Snapshot::manifest_path`
  gives the server the root manifest file, and its parent is the root directory for
  `stc.renames.toml`; `Snapshot::manifest_sources` owns the manifest texts and spans.
- The server is single-threaded per connection, so a pending tracked-rename token needs no lock.

### 4.2 Existing rename data and CLI behavior

- `stc-ide` already resolves a cursor to a declaration identity for definition and references:
  `Target`, `target_at`, `references`, `collect_references`, `declaration_span`, and
  `target_declaration`. This is the same resolution map the CLI uses, and the module docs say
  so.
- `stc rename` (`crates/stc-driver/src/rename.rs`) resolves a dotted key in the root package,
  then edits the declaration, every sema-resolved reference, and qualified path prefixes for
  POU/type/namespace targets. Supported targets are `Def`, `Field`, `Param`, `Local`, and
  `EnumValue`; path-prefix kinds are namespaces, functions, function blocks, programs, classes,
  interfaces, methods, structs, enums, named values, subranges, and aliases.
- The CLI validates a new name as one identifier token that is not a reserved keyword, allows
  case-only changes, and appends `stc.renames.toml` only after every write succeeds. D-156 and
  D-169 fix the CLI contract; renaming a dependency's declaration or a package name is out of
  scope.
- The CLI records the **user-supplied** dotted keys, including parameters and locals. A full-path
  invocation such as `stc rename AddStep Advance` or
  `stc rename Accumulator.increment Accumulator.delta` records the canonical full paths. Builds
  uppercase both sides before feeding the identity engine.
- The IDE `DeclarationIndex` is token-keyed, which is exactly what a cursor-based rename needs.
  The CLI's index is dotted-key-keyed; the two serve different entry points over the same
  analysis.
- `stc-manifest` owns the history format: `load_rename_history`, `append_rename`,
  `render_rename_history`, and `merge_renames`. `append_rename` rejects an exact duplicate
  (`STC-E-PROJECT-013`); `merge_renames` uppercases keys, composes chains oldest-first, and
  rejects a mapping that targets itself.
- `stc run --watch` watches the root `stc.renames.toml`, so a history write after a source apply
  triggers the corrective rebuild. The watch's child build merges the history.

### 4.3 What is missing

- No `prepareRename` or `rename` query in `stc-ide`; no capability advertisement, handler, or
  error mapping in `stc-lsp`.
- No root-package restriction and no path-prefix edit collection in `stc-ide`.
- No public new-name validation helper in `stc-ide` (the CLI helper is private).
- No collision check in the language server.
- No external-reference edit collection for the manifest, profile, `VAR_ACCESS`, or
  `VAR_CONFIG`.
- No tracked-rename protocol, pending token, or history write in `stc-lsp`.
- No VS Code rename middleware or post-apply confirmation controller.
- No multi-file LSP rename test, negative protocol tests, tracked-protocol tests, external-
  reference tests, or editor rename smoke.
- The IDE snapshot does not load or parse the selected machine profile, and `stc run --watch`
  does not watch it.
- The user-facing docs still say rename is not offered (see §4.12).

### 4.4 Why a history-free rename is not enough

A history-free editor rename changes identifiers but leaves `stc.renames.toml` untouched. Both
online-change paths build against the active baseline and merge that history, so the compiler
sees the old identity removed and the new identity added, with no tracked mapping:

- Local `stc run --watch` rebuilds by spawning `stc build` against the store's active baseline;
  the child build merges `stc.renames.toml` and CLI `--rename` but watch forwards neither
  `--rename` nor `--allow-untracked-removal` to the child.
- Remote online deploy builds the candidate on the host with `stc build --got-manifest <active>`;
  the source mirror includes `stc.renames.toml`, and no untracked-removal option is passed.

The resulting behavior depends on the renamed identity:

| Renamed symbol | History-free local watch | History-free remote online | Tracked editor rename |
| --- | --- | --- | --- |
| Data root (global/retained variable, program/FB instance, other root) | Build fails `STC-E-MANIFEST-003`; watch keeps the active generation. With `--allow-untracked-removal` the old root is dropped/quarantined (`STC-W-ONLINE-001`, `STC-W-MANIFEST-002`), its state is discarded, and the new root gets a fresh `GotId`. Watch does not forward the flag. | Candidate build fails `STC-E-MANIFEST-003`; deploy fails and the host keeps the active generation. The candidate build cannot pass `--allow-untracked-removal`. | `old -> new` history preserves the `GotId`; the online plan can preserve the root and its state. |
| Field of a persistent type/FB | Layout hash changes; without a tracked field rename `fields_match_under_renames` fails, the plan is `reject.layout`, and the build fails with `STC-E-ONLINE-*`. | Same rejection in the candidate build or in `online_possible`; deploy fails. | The field rename entry maps the old field path; the layout matches and the plan can be online. |
| Type used by a persistent root | Type key changes, so the root is `reject.layout`; the build fails. | Same. | The type rename entry preserves the type key and the root identity. |
| Function, method, enum value | Old identity is dropped/retired and a new one allocated; an online candidate can deploy when no export or layout is affected, but the identity is not preserved. An exported symbol removal is `STC-E-ONLINE-002`. | Same. | The history entry preserves the identity. |
| Parameter, local, case-only spelling | No GOT identity; the source change is safe for online change. | Safe. | History-free by design; no history entry and no warning. |
| Configuration | No GOT identity, but the configuration name appears in task-guard root keys; an online plan can see changed guard identities and force a cold download or reject. | Same. | History-free; the manifest selection is edited; warn about a possible cold download. |

Two consequences follow. First, a history-free editor rename is **not online-change-safe** for
stateful or layout-bearing symbols; it fails visibly rather than silently for the rest. Second,
`[online].allow-untracked-removal` is parsed but not wired into the driver build/watch/remote
paths; only the CLI flag sets `Request.allow_untracked_removal`, and the watch and remote
candidate builds do not forward it. That is an existing gap outside WL-18, but it means the
manifest setting cannot rescue an untracked editor rename.

After a history-free editor rename, `stc rename Old New` cannot repair the history because the
old name no longer resolves in source. The recovery is to add the `[[rename]]` entry (old key to
new key) to `stc.renames.toml` by hand before the next online build, or to revert the source and
use `stc rename`. The tracked editor rename exists to avoid that trap for VS Code users; the
standard LSP method keeps the visible warning for every other client.

### 4.5 Discovered CLI edge: case-only renames

While validating the history rules, a case-only rename was run on the sample workspace:

```text
stc rename AddStep addstep
```

The CLI accepts it and writes:

```toml
schema = 1

[[rename]]
old = "AddStep"
new = "addstep"
```

The next build fails `STC-E-PROJECT-013: rename 'AddStep' targets itself` because
`merge_renames` uppercases both sides and sees `ADDSTEP -> ADDSTEP`. This is an existing CLI
defect, not a WL-18 regression. The tracked LSP must not reproduce it: GOT keys are
ASCII-uppercase, so a case-only source rename changes no identity and writes no history entry.
The plan records the CLI defect as a follow-up (Q-3f); fixing the CLI is outside WL-18 unless
the owner expands the scope.

### 4.6 External-reference surface (verified)

The compiler resolves these names at build time. A history-free source rename leaves the string
stale and the next build fails visibly:

| External reference | Stale diagnostic |
| --- | --- |
| `[project].configuration` | `STC-E-PROJECT-007`: configuration `X` is not declared in package `Y` |
| `[[io.bindings]].variable` | `STC-E-PROJECT-009`: I/O binding variable `inst.input` does not name a declared `AT` variable or field |
| Profile `[[io.routes]]` `plc-input`/`plc-output` `variable` | `STC-E-IO-BINDING-002`: route endpoint `X` does not name a direct `%I`/`%Q` variable |
| `[[test.case]].name` | `STC-E-PROJECT-008`: `[[test.case]]` names `X`, which is not a `{stc:test}` POU |
| `VAR_ACCESS` path | `STC-E-NAME-001`: type `Main` has no member `X` |
| `VAR_CONFIG` instance-field path | `STC-E-NAME-001`: type `Main` has no field `X` |

All six were reproduced with temporary projects on the current base. The CLI currently allows
all of them and leaves the next build broken. That is an existing CLI gap, not a WL-18
regression; WL-18 closes it in the language server.

Resolution details:

- `[[io.bindings]].variable` and profile routes match the image binding names produced by
  `stc-codegen`: the short global name, the namespace-qualified global path, package-prefixed
  variants, and `instance.field` for a direct `AT` program field. Matching is case-insensitive.
  Only declared `AT` globals and direct `AT` program fields are valid.
- `[project].configuration` matches a root-package `CONFIGURATION` declaration
  case-insensitively.
- `[[test.case]].name` matches a root-package `{stc:test}` POU case-insensitively.
- `VAR_ACCESS` and `VAR_CONFIG` paths are resolved by sema into HIR places and field IDs; they
  are **not** in `analysis.resolutions`, so the CLI reference collector misses them.

### 4.7 `VAR_ACCESS`/`VAR_CONFIG` span gap

- `VarAccessHir` stores the resolved `place: ExprId` and the declaration `span`, but not the
  spans of the path segments. A program-level `VAR_ACCESS` place is a chain of
  `PlaceHir::Field { base, field }` expressions whose expression spans are the segment spans, so
  those spans are recoverable. A configuration/resource-level `VAR_ACCESS` place is
  `PlaceHir::InstanceField { program, instance, fields }` and stores only the whole span.
- `ConfigInitHir` stores `fields: Vec<FieldId>` and the whole `span`, but not the per-field
  spans.
- Sema already iterates the CST `Name` tokens for both constructs, so the segment spans are
  available where the HIR entries are built. WL-18 adds them to the HIR (spans only) so the
  rename engine can edit the exact segment without a second resolver.

### 4.8 Collision behavior

The CLI resolves the new dotted path in the root-package declaration index. If it names a
different declaration, it exits before editing:

```text
error[STC-E-PROJECT-013]: `Accumulator` already names a declaration in the current package
```

The check is case-insensitive (paths are normalized to uppercase), covers definitions, fields,
methods, enum values, parameters, and locals, and allows a case-only rename of the same target.
The LSP must use the same check before returning any edit. The residual gap is a name inherited
from a base class or imported through `USING` that is not in the root-package index; the engine's
in-memory source reanalysis (§7.2 item 9) catches that case before returning the edit.

### 4.9 `def_path` namespace-name defect

`stc rename` builds a declaration path with `def_path` (`rename.rs`). The function walks the
parent chain, excludes the root namespace, and then unconditionally pops the last segment when it
equals the package namespace:

```rust
if path.last().is_some_and(|last| last.eq_ignore_ascii_case(namespace)) {
    path.pop();
}
```

A declaration whose own name equals the package namespace is therefore stripped from its own
path. Verified with a package named `sample` containing `FUNCTION Sample`: `stc rename Sample
SampleX` reports “does not name a declaration”. Removing the pop is correct because the root
namespace is already excluded by the parent-chain walk. WL-18 moves the helper to `stc-hir`,
removes the pop, and adds a regression test; the CLI uses the shared helper.

### 4.10 VS Code client surface

- `vscode-languageclient` 9.0.1 exposes `RenameMiddleware.provideRenameEdits` and
  `prepareRename`. The rename feature calls the middleware with
  `(document, position, newName, token, next)`.
- `LanguageClient.protocol2CodeConverter.asWorkspaceEdit` converts the private response's LSP
  `WorkspaceEdit` into a real `vscode.WorkspaceEdit`, so the middleware can return it to VS Code.
- `@types/vscode` 1.82 has no `workspace.onDidApplyEdits`. The controller must observe
  `workspace.onDidChangeTextDocument` and the existing `**/*.st` file watcher, then read only
  the edited files and compare their text with the expected post-edit text. It must also read
  the edited `stc.toml` and profile files, which are not covered by the source watcher.
- `workspace.applyEdit` is available for the smoke, which applies the returned edit and then
  checks the history file. The extension has no public command to drive the interactive rename
  input box.

### 4.11 Watch and refresh gap

- `stc run --watch` watches manifests, sources, the lockfile, and `stc.renames.toml`, but not
  the selected machine profile. A profile edit from a rename would not trigger a rebuild until
  the next unrelated change.
- The VS Code client watches `**/*.st` and `**/stc.toml`, not the profile. The language server
  would keep a stale snapshot after a profile edit until a source or manifest event.
- WL-18 adds the selected profile to the watch snapshot and makes the tracked-rename controller
  send `workspace/didChangeWatchedFiles` for edited manifest/profile files so the server
  reloads. This is integration work required by the expanded edit, not a new LSP feature.

### 4.12 Documentation and test surface

- `specification/Architecture/IDE-Integration.md` §3.2 capability map and §3.3 closed
  unsupported list.
- `specification/Architecture/CLI-and-Manifest.md` §3.1 rename history.
- `specification/Architecture/Online-Change-and-GOT.md` §7.3 identity and history.
- `USER-GUIDE.md` §8.3 language features.
- `KNOWN-LIMITATIONS.md` §4 capability row.
- `editors/vscode/README.md` language-features paragraph.
- Tests: `crates/stc-ide/tests/ide.rs` and `navigation.rs` (multi-file navigation),
  `crates/stc-driver/tests/lsp_cli.rs` (real binary, framed JSON-RPC),
  `crates/stc-lsp/src/server.rs` capability test, the extension unit tests under
  `editors/vscode/test/*.test.mjs`, and the headless VS Code smoke
  (`editors/vscode/test/electron/suite.js`).

## 5. Phase 0 — Close owner questions and record decisions

### 5.1 Owner questions

Each question has a recommendation. The owner may accept the recommendation or choose the
alternative; the answers are recorded in `specification/Decisions.md` as D-253 before Phase 1
starts. Q-10 and Q-11 are already answered by the owner decisions in the header.

**Q-1. Which symbol kinds does the first slice cover?**

Recommendation: every kind `stc rename` already supports — definitions (functions, function
blocks, programs, classes, interfaces, methods, structs, enums, named values, subranges, aliases,
namespaces, globals, configurations), fields, parameters, locals, and enum values, including the
path-prefix kinds. Reject standard-library functions (`Target::StdFunction`) and built-in types
(`Target::Type` that is not a named definition) with a visible error. This keeps CLI and LSP
behavior aligned and avoids a second "supported kinds" table. A configuration rename is
supported, edits `[project].configuration`, writes no history, and warns about a possible cold
download.

**Q-2. May the editor rename a dependency's declaration?**

Recommendation: no. Match D-156: only declarations owned by the root package are renameable.
`SourceFileInfo.package` plus a root-package accessor gives the check. Root test sources are
included, matching the CLI. A dependency declaration request returns a visible error.

**Q-3. Is the editor rename tracked?**

Recommendation: yes, as the owner selected. The standard LSP methods are history-free; the VS
Code extension uses `stc/rename` and `stc/recordRename`. `stc/rename` computes the edit and the
canonical history entries but writes nothing. The extension applies the edit, verifies every
edited file's post-apply text (source, manifest, and profile), then calls `stc/recordRename`; the
server appends the pending entries. This is a safe-apply handshake, not a request-time write: a
cancel, undo, partial apply, or failed verification leaves history unrecorded. §4.4 is the
evidence for why this is needed.

**Q-3a. Which targets write history?**

Recommendation: only identity-bearing root-package declarations:

- `Target::Def` kinds `Namespace`, `Alias`, `Subrange`, `Enum`, `NamedValue`, `Struct`,
  `Function`, `FunctionBlock`, `Program`, `Class`, `Interface`, `Method`, and `Global`;
- `Target::Field`;
- `Target::EnumValue`.

Parameters, locals, and configurations are history-free: they have no GOT key and no online
identity to preserve. A case-only rename writes no history because GOT keys are uppercase and a
case-only source change cannot change an identity (this also avoids the CLI defect in §4.5).
The root namespace is a package rename and is rejected; every other tracked target has a
canonical path. Standard functions, built-in types, and dependency declarations are rejected. A
test POU is a `Function`; it writes the normal history entry and its `[[test.case]].name`
reference is edited.

**Q-3b. What happens on cancel, undo, partial apply, or a failed history write?**

Recommendation: the extension records only when every edited file's current text equals the
expected post-edit text. Cancel or undo before confirmation leaves the text at the original
value, so no `recordRename` is sent and no history is written. A partial apply leaves at least
one file at the original or an intermediate value, so verification fails and the extension shows
a warning naming the unrecorded rename and the recovery (revert and use `stc rename`, or add the
`[[rename]]` entry by hand). A failed history write (conflict, malformed file, or I/O) returns
`RequestFailed`; the extension warns with the server's message and the source stays applied. The
pending token is single-use and expires after five minutes. If the server or extension restarts
before confirmation, the token is lost and no history is written; the docs state the recovery.
The watch may attempt one rebuild between the source apply and the history write; it keeps the
old generation, and the history write triggers the corrective rebuild because the watch already
watches `stc.renames.toml`.

**Q-3c. How are history conflicts handled?**

Recommendation: reload `stc.renames.toml` at `recordRename` time, not at `rename` time, so a
concurrent CLI write is not overwritten. Use the existing `append_rename` rules: an exact
duplicate or a conflicting chain is `STC-E-PROJECT-013`; a malformed file is
`STC-E-PROJECT-012`. Return `RequestFailed` with the existing code and message; do not merge or
overwrite silently. The source stays applied and the warning gives the recovery. A case-only
rename writes no entry, so it cannot create a self-conflict.

**Q-3d. Standard method or tracked method?**

Recommendation: keep `textDocument/rename` as the standard history-free method for every client.
It returns the complete edit (source, manifest, profile, and access paths). For an
identity-bearing target it sends one `window/showMessage` warning that no history was written,
names the required `[[rename]]` entry, and says the next online download may be cold. For a
configuration rename it also warns about the cold-download risk. Advertise
`experimental.stc.trackedRename: true` and the private `stc/rename`/`stc/recordRename` pair. The
VS Code extension uses the tracked path by default when the capability is present; there is no
user setting in WL-18. This keeps other editors working and keeps the history-free trap visible.

**Q-3e. What is the older-server fallback?**

Recommendation: if the capability is absent or `stc/rename` returns MethodNotFound (-32601), the
middleware calls `next()` (the standard rename) and shows one warning per session that history
was not updated and the next online download may be cold. Other errors are rethrown so VS Code
shows the server's message. This keeps a new extension usable with an older `stc` binary.

**Q-3f. Do we fix the case-only CLI defect in WL-18?**

Recommendation: no. The tracked LSP writes no history for case-only renames, so the feature is
correct without touching the CLI. Record the defect in this plan and add a `FutureWishlist.md`
entry at closeout: `merge_renames` should skip an entry whose `old` and `new` are equal after
uppercase normalization. If the owner wants it fixed in WL-18, it is a small `stc-manifest`
change plus a test, but it changes CLI history handling and therefore expands the locked scope.

**Q-4. What does `prepareRename` return for a rejection?**

Recommendation: return `{ range, placeholder }` for a renameable source identifier, including a
`VAR_ACCESS`/`VAR_CONFIG` path segment that resolves to a field; return `null` for a position
that is not an identifier, has no resolution (VS Code shows its generic "cannot rename"
message), or is in a document outside the project source set, including `stc.toml` and the
profile (rename is source-initiated in WL-18); return a JSON-RPC `RequestFailed` (-32803) error
with a clear message for a resolved but unsupported target (standard function, built-in type,
dependency declaration, root namespace). The `rename` request repeats the validation and returns
`RequestFailed` for an invalid new name, an unsupported target, a collision, or a document
outside the project source set. This gives unsupported symbols a visible reason instead of a
silent no-op.

**Q-5. How are ambiguous or unresolved references handled?**

Recommendation: the cursor resolution is unique by construction: the innermost node with a
recorded resolution, then the token-matching rule used by definition/references, then the new
access-path index for `VAR_ACCESS`/`VAR_CONFIG`. An unresolved identifier is rejected; no
separate ambiguity policy is needed. References are collected from every resolution with the
same identity, using the existing rule that picks the identifier token spelling the declared
name inside each resolved node (the same rule as the CLI). Files outside the project source set
are never edited; `prepareRename` returns `null` and `rename` returns `RequestFailed` for such a
file.

**Q-6. Is a workspace-edit size limit needed?**

Recommendation: no hard limit in WL-18. The edit is proportional to the project the user already
opened, and a cap could block a legitimate rename. Add a multi-file test large enough to prove
the edit spans files and the manifest, but do not add a threshold or timing gate. If the owner
prefers a guard, propose 10,000 edits with a clear error and a test.

**Q-7. Does the CLI and LSP share one rename operation?**

Recommendation: implement the cursor-based engine in `stc-ide` over the existing analysis, keep
the CLI's dotted-key entry point and history behavior unchanged, and add a parity test in
`crates/stc-driver/tests/lsp_cli.rs` that runs `stc rename` on one temp copy and the tracked LSP
flow on another, then compares the source files and the identity-bearing history entries. The
parity test excludes external-reference cases (the CLI is source-only in WL-18) and cases where
the CLI records parameters/locals or the case-only defect in §4.5; those cases compare source
text only. This satisfies "reuse the resolution, do not build a second symbol graph" without
destabilizing the CLI.

**Q-8. What is the editor smoke and manual gate?**

Recommendation: extend the headless VS Code smoke with two renames:

1. `AddStep` -> `Advance`: call `vscode.executeDocumentRenameProvider`, apply the returned edit
   with `vscode.workspace.applyEdit`, poll for `stc.renames.toml` with `old = "AddStep"` and
   `new = "Advance"`, and assert both `src/main.st` and `tests/smoke.st` contain the new name.
2. `Sample` -> `SampleX`: rename the configuration, assert `stc.toml` contains
   `configuration = "SampleX"` and `src/main.st` contains `CONFIGURATION SampleX`, and assert no
   history entry was added for it. The cold-download warning is a `window/showMessage`
   notification, which the headless smoke cannot observe; the `lsp_cli` test asserts the
   warning payload instead.

The smoke restores the sample workspace and deletes the history file in a `finally` block. The
owner's manual check is:

1. F2 in VS Code on the sample workspace: apply the `AddStep` rename, confirm every file
   updates, diagnostics stay clean, and the history entry appears; revert and delete the history.
2. F2 on the configuration `Sample`: confirm `stc.toml` and the source change, the cold-download
   warning appears, and no history entry is written.
3. F2 on a `VAR_ACCESS` path segment and a `VAR_CONFIG` field path in a temp project: confirm
   the path segment updates and the build stays clean.
4. F2 on a project with `[[io.bindings]]` and a machine-profile route: rename the direct
   `AT` variable/field and confirm both strings update and the build stays clean.
5. F2 on an unsupported symbol (for example a standard function) and confirm a visible message.
6. A live `stc run --watch` on a copy of `examples/online-change`: rename
   `Counter.Retained` to `Counter.Persistent` through the editor, confirm the watch deploys
   without `STC-E-MANIFEST-003`/`STC-E-ONLINE-*`, and confirm the root's `GotId` is unchanged in
   `stc online download` before and after.
7. A negative control on a fresh copy: apply the same source edit without history and confirm
   the watch rejects the candidate and keeps the old generation.

The manual check is required before the PR (workflow §8).

**Q-9. Which durable decision and documentation changes are recorded?**

Recommendation: add D-253 (LSP rename policy) covering Q-1 through Q-16. Update IDE Integration
§3.2/§3.3, CLI and Manifest §3.1, Online Change and GOT §7.3, USER-GUIDE §8.3,
KNOWN-LIMITATIONS §4, and the VS Code README. The v3.0.0 plan and wishlist are updated at
closeout, not in Phase 0.

**Q-10. How should WL-18 handle declarations that are referenced by `stc.toml`, a machine
profile, `VAR_ACCESS`, or `VAR_CONFIG`?**

Answered by the owner: edit them as part of the rename. The rename must not reject a target
merely because it has an external reference. The edit covers `[project].configuration`,
`[[io.bindings]].variable`, `[[test.case]].name`, profile route `variable` values, and
`VAR_ACCESS`/`VAR_CONFIG` path segments.

**Q-11. What happens when identity cannot be preserved for online change?**

Answered by the owner: the rename still works as a normal rename. If preserving identity is too
expensive or not implementable for online change, the rename makes online change impossible and
forces the next download to be cold; warn the user and proceed. This applies to all rename types
that are not implementable for online change, not only external-reference renames. Completeness
is mandatory; identity is best-effort.

**Q-12. How is the collision check validated?**

Recommendation: before returning any edit, resolve the new dotted path in the same root-package
declaration index as the CLI. If it names a different declaration, return `RequestFailed` with
`STC-E-PROJECT-013` and the CLI message; write no history and return no edit. Allow a case-only
rename of the same target. Then apply the source edits in memory, re-parse the changed files,
re-run the semantic analysis with the same configuration/stdlib/test set, and reject if a new
error diagnostic appears. This catches duplicate declarations, inherited/imported member
conflicts, and invalid new spellings before the client applies anything. Manifest/profile
references are not re-resolved by this pass; they are consistent by construction because the
engine replaces the segment the compiler resolved to the target, and the collision check
guarantees the new path is unique. If the owner wants a full build validation, it is a separate
follow-up because it requires an in-memory project reload.

**Q-13. Do we fix the `def_path` namespace-name defect in WL-18?**

Recommendation: yes. It is a small, tested bug fix required for rename completeness: a
declaration named like the package namespace must be renameable. Move `def_path` to `stc-hir`,
remove the namespace-name pop, and switch the CLI to the shared helper. This is a CLI behavior
fix, not a semantic change: the old behavior rejected a valid declaration. Add a regression test
in `stc-hir` and a CLI test.

**Q-14. How are manifest and profile spans retained?**

Recommendation: add internal spans to the existing models without changing the TOML format or
schema:

- `ProjectSettings.configuration_span: Option<Span>` for the `[project].configuration` string
  contents;
- `IoBinding.variable_span: Span` for the `[[io.bindings]].variable` string contents;
- `TestCase.name_span: Span` for the `[[test.case]].name` string contents;
- `ProfileEndpoint::PlcInput`/`PlcOutput` `variable_span: Span` for the route endpoint
  `variable` string contents.

Spans are byte offsets into the text after BOM handling, with the quotes excluded. The parser
computes them from the existing `toml_edit` value spans. Add parser tests for basic strings,
BOM-prefixed files, and multi-line arrays of tables.

**Q-15. How are `VAR_ACCESS`/`VAR_CONFIG` spans retained?**

Recommendation: add spans only to the HIR:

- `VarAccessHir.path_spans: Vec<Span>`: the resolved path segment spans. For a program-level
  access, these align with the `PlaceHir::Field` chain; for a configuration/resource-level
  access, they align with `PlaceHir::InstanceField.fields`; for a single global, the one span.
- `ConfigInitHir.field_spans: Vec<Span>`: the resolved instance-field segment spans, aligned
  with `fields`.

Sema already has the CST `Name` tokens when it builds these entries. No new resolver and no
manifest change. Add sema tests that assert the spans point at the right identifiers.

**Q-16. What is the CLI divergence and follow-up?**

Recommendation: `stc rename` stays source-only in WL-18. It will leave `stc.toml`, profile, and
access references stale, exactly as it does today. The language server edits them. Document the
divergence in CLI and Manifest §3.1 and IDE Integration, and add a `FutureWishlist.md` entry to
extend `stc rename` with the same external-reference edits (reusing the `stc-ide` engine or
moving it to a shared crate). Do not add a `stc-driver` -> `stc-ide` dependency edge in WL-18.

### 5.2 Proposed decision D-253

> LSP rename uses the compiler's resolution data in `stc-ide` and advertises `renameProvider`
> with `prepareProvider`, plus `experimental.stc.trackedRename` and the private `stc/rename` and
> `stc/recordRename` requests. Only root-package declarations are renameable; supported targets
> are the definition, field, parameter, local, and enum-value kinds that `stc rename` supports,
> including qualified path-prefix kinds. Standard functions, built-in types, dependency
> declarations, and the root namespace (a package rename) are rejected with a JSON-RPC
> `RequestFailed` message. The rename is complete first: it edits the declaration, every
> resolved source reference, every qualified path prefix, and every resolved external reference
> in `[project].configuration`, `[[io.bindings]].variable`, `[[test.case]].name`, machine-profile
> `[[io.routes]]` `plc-input`/`plc-output` `variable` values, `VAR_ACCESS` paths, and
> `VAR_CONFIG` instance-field paths. It never leaves a reference stale. Before returning an
> edit it resolves the new dotted path in the same root-package declaration index as the CLI and
> rejects a different existing declaration with `STC-E-PROJECT-013`; a case-only rename of the
> same target is allowed. It then applies the source edits in memory, re-parses and re-analyzes
> them, and rejects a new error diagnostic. `prepareRename` returns the identifier range and
> placeholder for a renameable target (including a `VAR_ACCESS`/`VAR_CONFIG` path segment),
> `null` for a non-identifier, unresolved position, or a document outside the project source set
> (including `stc.toml` and the profile), and `RequestFailed` for a resolved but unsupported
> target. The standard `textDocument/rename` validates the new spelling with the lexer (one
> identifier, not a reserved keyword), allows case-only changes, collects the declaration, every
> resolved reference, qualified path-prefix edits, and the external-reference edits, deduplicates
> them, rejects overlaps, returns a `changes` `WorkspaceEdit`, and writes no history. For an
> identity-bearing target or a configuration rename it sends one `window/showMessage` warning
> naming the required history entry or the cold-download risk. The private `stc/rename` returns
> the same edit plus canonical root-package history entries, an online-change classification,
> optional warnings, and a single-use token when history is available; it writes nothing.
> `stc/recordRename` appends the pending entries to `stc.renames.toml` with the existing
> `stc-manifest` load/append/render rules only after the client confirms the edit was applied.
> History is written only for identity-bearing targets:
> namespaces, aliases, subranges, enums, named values, structs, functions, function blocks,
> programs, classes, interfaces, methods, globals, fields, and enum values. Parameters, locals,
> configurations, and case-only renames are history-free. A configuration rename edits
> `[project].configuration` and warns that task-guard identities may change, so the next online
> download may be cold. Identity preservation is best-effort: a rename that cannot preserve
> identity still applies as a normal rename and warns about a possible cold download and retained
> state reset; it is never rejected for that reason. A missing or invalid selected profile is
> reported as a warning and the rename proceeds without profile edits. The pending token is
> single per connection, latest wins, and expires after five minutes. The VS Code extension
> verifies every edited file's post-apply text (source, `stc.toml`, and the profile) before
> recording; cancel, undo, partial apply, a failed verification, or a failed history write leaves
> history unrecorded and surfaces a warning with the recovery. The controller sends `workspace/didChangeWatchedFiles` for edited
> manifest/profile files so the server reloads, and `stc run --watch` watches the selected
> machine profile. If the capability or method is absent, the extension falls back to the
> standard history-free method and warns once. The server writes only `stc.renames.toml`, never
> source. There is no edit-size cap and no new `STC-*` code.

### 5.3 Phase 0 exit criteria

- Every question above has an owner answer recorded in this plan.
- D-253 is written in `specification/Decisions.md` and the affected docs list is fixed.
- No implementation starts before the plan and Phase 0 answers are approved.
- The plan is committed on `WL-18` and CI is green (docs-only change).

## 6. Phase 1 — Span retention, shared helpers, and profile loading

### 6.1 Goal

Make the data the rename engine needs available without changing the manifest format or building
a second resolver: exact spans for manifest, profile, `VAR_ACCESS`, and `VAR_CONFIG` values; a
shared declaration-path helper with the namespace-name fix; a shared identifier validator; and a
snapshot that loads the selected machine profile.

### 6.2 Work

1. `stc-manifest` model and parser:
   - add `ProjectSettings.configuration_span: Option<Span>`;
   - add `IoBinding.variable_span: Span`;
   - add `TestCase.name_span: Span`;
   - add `variable_span: Span` to `ProfileEndpoint::PlcInput` and `PlcOutput`;
   - add a `string_content_span(text, value_span)` helper that skips the opening and closing
     quote (basic and literal strings) and returns the content span;
   - populate the spans in `parse_project`, `parse_bindings`, `parse_test_cases`, and
     `parse_endpoint_table`;
   - update every construction site and test fixture; add parser tests for basic strings, BOM
     prefixes, and arrays of tables.
   - No schema, key, validation, or rendering change.
2. `stc-hir` and `stc-sema`:
   - add `VarAccessHir.path_spans: Vec<Span>` and `ConfigInitHir.field_spans: Vec<Span>`;
   - populate them from the CST `Name` tokens in `check_program_var_access`,
     `check_config_var_access`, and `check_config_inits`;
   - add sema tests asserting the spans point at the right segments for a program-level access,
     a configuration-level access, a single global, and a nested `VAR_CONFIG` path.
3. `stc-hir` shared declaration path:
   - add `HirCrate::def_path(def: DefId) -> Option<Vec<String>>`, using the existing parent-chain
     walk that stops at the package root namespace;
   - remove the namespace-name pop (§4.9); the helper no longer needs a namespace argument;
   - switch `stc-driver` `def_path` to the shared helper with no other behavior change;
   - add `stc-hir` tests for a root declaration, a nested namespace declaration, a declaration
     named like the package namespace, and a dependency declaration.
4. `stc-lexer` shared identifier validator:
   - add `is_identifier(text: &str) -> bool` (one identifier token, no lexer errors, not a
     keyword);
   - move the CLI's `valid_identifier` onto it;
   - test empty, multi-token, keyword, dotted, and Unicode inputs.
5. `Snapshot` profile loading:
   - retain the root `Manifest` (or the fields the engine needs) and the root manifest
     `FileId`; normalize the stored `manifest_path` to the file (a directory argument becomes
     `<dir>/stc.toml`) and store the root directory;
   - read `[io].config` relative to the root directory, apply the same canonicalization and
     path-escape rule as the build, parse it with `stc_manifest::parse_profile`, and store the
     profile text, path, `FileId`, and parse diagnostics;
   - add the profile file to a dedicated profile source map (not the manifest source map) so its
     spans and ranges are resolvable and `RenameFile::Profile` has an unambiguous owner;
   - expose `root_manifest()`, `root_package()`, `manifest_file()`, `profile()`,
     `profile_path()`, `profile_sources()`, and `profile_file()`;
   - a missing, unreadable, escaping, or invalid profile is stored as an error, not a snapshot
     load failure; `Snapshot::load` keeps working so the other language features stay available;
   - add `stc-ide` tests for a project without a profile, a valid profile, a missing profile, an
     invalid profile, and a profile path that escapes the root.
6. Do not add a `stc-driver` -> `stc-ide` dependency edge.

### 6.3 Exit criteria

- Manifest and profile values expose exact content spans; `VAR_ACCESS`/`VAR_CONFIG` expose
  exact segment spans.
- `def_path` is shared, the namespace-name defect is fixed, and the CLI behavior is otherwise
  unchanged.
- `is_identifier` is shared and the CLI uses it.
- The snapshot loads the selected profile without failing the project.
- No rename engine or protocol code is added in this phase.
- `cargo test -p stc-manifest`, `cargo test -p stc-hir`, `cargo test -p stc-sema`,
  `cargo test -p stc-lexer`, `cargo test -p stc-driver`, `cargo test -p stc-ide`, clippy, and fmt
  pass; the phase commit is pushed and CI is green.

### 6.4 Evidence

Parser/HIR/sema span tests, the `def_path` regression test, the profile-loading tests, the CLI
regression test, and the phase commit/CI link recorded in §14.

## 7. Phase 2 — Rename engine in `stc-ide`

### 7.1 Goal

Add a pure, cursor-based rename engine over the existing analysis so the language server and
tests share one implementation, with no protocol code in `stc-ide`. The engine returns the
complete edit set, the target classification, the canonical history entries, and the
online-change warning; it does not write files.

### 7.2 Work

1. Add `crates/stc-ide/src/rename.rs` and export the public types from `lib.rs`:
   - `RenameFile { Source(FileId), Manifest, Profile }` so edits in the three source maps do not
     collide;
   - `RenameEdit { file: RenameFile, start: u32, end: u32, new_text: String }` (byte offsets);
   - `RenameHistoryEntry { old: String, new: String }` (canonical root-package paths);
   - `RenamePlaceholder { span: Span, name: String }`;
   - `RenameTargetKind { Tracked, HistoryFree }`: `Tracked` means this rename has canonical
     history entries; `HistoryFree` covers parameters, locals, configurations, and case-only
     renames;
   - `RenameOnline { Preserved, Cold(ColdReason) }` with `ColdReason::ConfigurationTaskGuard`;
   - `RenameWarning { ProfileUnavailable(String) }` for a missing or invalid selected profile;
   - `RenameResult { edits: Vec<RenameEdit>, history: Vec<RenameHistoryEntry>, online:
     RenameOnline, target: RenameTargetKind, warnings: Vec<RenameWarning> }`;
   - `RenameError` with stable messages for not-an-identifier, unresolved, unsupported symbol,
     dependency declaration, invalid new name, same name, collision (`STC-E-PROJECT-013`), no
     occurrences, overlapping edits, untracked key, and a post-edit analysis error.
   - `Snapshot::prepare_rename(file, offset) -> Result<RenamePlaceholder, RenameError>`.
   - `Snapshot::rename(file, offset, new_name) -> Result<RenameResult, RenameError>`.
2. Reuse `target_at`, `target_declaration`, `declaration_span`, `collect_references`, and the
   resolution map. Change the needed `query.rs` helpers to `pub(crate)` or add `pub(crate)`
   wrappers; do not widen the crate's public API beyond the rename types. Add
   `Snapshot::is_root_package(file)`, `Snapshot::root_namespace()`, and
   `Snapshot::root_package()`.
3. Add a root-package dotted-path index in `stc-ide` (`BTreeMap<String, Vec<Target>>`) built
   from the same HIR, mirroring the CLI `DeclarationIndex::build`/`resolve` rules:
   case-insensitive; definitions and methods; fields; parameters; locals; enum values; and
   namespace-qualified paths. It is a lookup over the existing analysis, not a second symbol
   graph. Use it for the collision check and for resolving manifest/profile strings to a
   target. The CLI keeps its own copy in WL-18; sharing it is a follow-up.
4. Add `access_target_at(file, offset)`: scan the root package's `VarAccessHir` and
   `ConfigInitHir` entries, match the cursor offset against `path_spans`/`field_spans`, and
   return the resolved `Target::Field`. `target_at` falls back to it when the resolution map
   misses. This lets F2 work on a `VAR_ACCESS`/`VAR_CONFIG` path segment.
5. Validate the new name with `stc_lexer::is_identifier`; allow case-only changes and reject an
   identical spelling. Reject a dependency declaration before collecting edits.
6. Collect source edits:
   - the declaration edit, every reference edit, and the qualified path-prefix edits for
     `is_path_prefix_kind`; sort, deduplicate, and reject overlaps;
   - the reference token rule must match the CLI (`collect_references` already does; prefix
     edits use the HIR parent chain and `PathExpr`/`Path` windows);
   - the `VAR_ACCESS`/`VAR_CONFIG` edits from the access target/index, using the exact segment
     spans from Phase 1.
7. Collect manifest edits:
   - `[project].configuration` when the target is a root `Configuration` def: replace the
     string content span with the new name;
   - `[[io.bindings]].variable` when the target is a declared `AT` global or a direct `AT`
     program field: replace the matching segment inside the value span;
   - `[[test.case]].name` when the target is a root `{stc:test}` POU: replace the string content
     span with the new name.
   For `[[io.bindings]]` and profile routes, the engine computes the same candidate names
   `stc-codegen` uses for the image binding (`binding_names`): the short global name, the
   namespace-qualified path, package-prefixed variants, and `instance.field` for direct program
   fields (the instance names come from the HIR configuration/resource program instances), then
   compares the manifest/profile string case-insensitively. Use the raw text from the manifest
   or profile source map to locate the segment offset, not the unescaped parsed value. When a
   candidate matches, the edited segment is the last dot-separated segment of the value;
   configuration and test-case values replace the whole content span. It does not build a
   second resolver. A program-type rename does not match an `instance.field` string; the
   program-type reference is in the source `PROGRAM inst : Type` clause and is edited by the
   source reference collector.
8. Collect profile edits:
   - for each `[[io.routes]]` `plc-input`/`plc-output` endpoint whose `variable` matches a
     candidate name of the target (the same `binding_names` rules), replace the matching
     segment inside `variable_span`;
   - if the selected profile is missing or invalid, collect no profile edits and add
     `RenameWarning::ProfileUnavailable` with the parse error; the rename proceeds because the
     project already has a build error and the owner policy is to keep rename working. The
     warning tells the user to fix the profile reference manually.
9. Collision and validation:
   - resolve the new dotted path in the root-package declaration index; a different existing
     declaration is `RenameError::Collision` with `STC-E-PROJECT-013` and the CLI message;
   - apply the source edits in memory, re-parse the changed files, re-run the semantic analysis
     with the same configuration/stdlib/test set, and reject a new error diagnostic with
     `RenameError::Analysis`;
   - do not write files; do not re-run the full build.
10. History and online classification:
    - `Target::Def` of a tracked kind: `def_path`; new path = old path with the last segment
      replaced by the new name;
    - `Target::Field`: `def_path(field.owner) + field name`; new path replaces the last
      segment;
    - `Target::EnumValue`: `def_path(enum) + value name`; new path replaces the last segment;
    - `Target::Param`, `Target::Local`, and `Target::Def(Configuration)`: `HistoryFree`;
    - case-only rename: `HistoryFree`, even when the declaration kind is normally tracked;
    - tracked target with no canonical path (the root namespace, a package rename):
      `RenameError::UntrackedKey`;
    - standard function and built-in type: `RenameError::Unsupported`;
    - `RenameOnline::Cold` only for a configuration rename (task-guard identity); `Preserved`
      otherwise. The LSP standard handler adds the “history not recorded” warning when
      `target == RenameTargetKind::Tracked`, because the standard method never writes history.
11. Add unit and integration tests in `stc-ide`:
    - prepare/rename on a function, FB type, field, parameter, local, enum value, namespace or
      other path-prefix kind, program field, global, configuration, and test POU;
    - multi-file references (sample workspace `src/main.st` + `tests/smoke.st`; motor example
      `motor` + `motorlib`);
    - external references: `[project].configuration`, `[[io.bindings]].variable` in its short,
      namespace, package-prefixed, and `instance.field` spellings, `[[test.case]].name`, and a
      profile route `variable`;
    - `VAR_ACCESS` program-level, configuration-level, and single-global paths; `VAR_CONFIG`
      top-level and nested field paths; F2 on the path segment;
    - case-only rename: edits present, history empty;
    - invalid name, reserved keyword, same name, unresolved position, non-identifier position;
    - standard function, built-in type, dependency declaration, root-namespace rejection;
    - collision: existing declaration, case-insensitive, same-target case-only allowed;
    - post-edit analysis rejection: an inherited/imported member conflict;
    - overlap and duplicate-edit handling;
    - history keys: function `AddStep -> Advance`, field
      `Accumulator.increment -> Accumulator.delta`, enum value, namespace prefix;
    - parameter/local/configuration history empty; configuration `RenameOnline::Cold`.
12. Do not add a `stc-driver` dependency edge. The CLI parity comparison lives in the Phase 3
    `lsp_cli` test, which already runs the real binary.

### 7.3 Exit criteria

- The engine returns correct declaration + reference + prefix + manifest + profile + access
  edits for every supported kind.
- The engine returns canonical history entries only for tracked targets, classifies online
  change, and never writes files.
- Every rejection path has a stable, tested message.
- No `stc-lsp` or extension code is added in this phase.
- `cargo test -p stc-ide`, clippy, and fmt pass; the phase commit is pushed and CI is green.

### 7.4 Evidence

`stc-ide` test output covering source, manifest, profile, access, collision, and warning cases,
plus the phase commit/CI link.

## 8. Phase 3 — LSP standard and tracked rename

### 8.1 Goal

Expose the engine through the language server, update the closed capability set for rename only,
and implement the private tracked-rename handshake and history write.

### 8.2 Work

1. Add `REQUEST_FAILED` (-32803) to `crates/stc-lsp/src/protocol.rs`.
2. Advertise in `capabilities()`:
   - `"renameProvider": { "prepareProvider": true }`;
   - `"experimental": { "stc": { "trackedRename": true } }`;
   - keep every other capability unchanged and update the comment that excludes rename.
3. Add to the dispatch:
   - `textDocument/prepareRename`, `textDocument/rename`, and `stc/rename` also to the
     `flush_analysis` list (they need a fresh analysis);
   - `stc/recordRename` to the dispatch only (it does not need analysis; it only writes
     history).
   Use a `Result<Value, String>` handler shape so an `Err` becomes a `RequestFailed` response;
   the existing `Value` handlers stay as they are.
4. Add a `rename_edit_json(snapshot, edit)` helper:
   - `RenameFile::Source(file)` -> `snapshot.file(file).path` and `snapshot.source_map()`;
   - `RenameFile::Manifest` -> `snapshot.manifest_path()` and `snapshot.manifest_sources()`,
     with `snapshot.manifest_file()` for the range text;
   - `RenameFile::Profile` -> `snapshot.profile_path()` and `snapshot.profile_sources()`,
     with `snapshot.profile_file()` for the range text;
   - convert the byte span with the owning source map (do not use `span_location`, which uses
     the project source map);
   - group edits by URI into the `changes` map, preserving file order.
5. Standard handlers:
   - add a `show_message` helper that sends `window/showMessage` through the existing
     `protocol::notification` and `send` path;
   - if the document is not a project source, `prepareRename` returns `null` and `rename`
     returns `RequestFailed` with a clear message;
   - `prepare_rename_result`: `Ok` -> `{ range, placeholder }`; `NotAnIdentifier`/`Unresolved`
     -> `null`; other errors -> `RequestFailed`.
   - `rename_result`: call `Snapshot::rename`, map `Ok` to `{ "changes": ... }`, and map errors
     to `RequestFailed`.
   - After a successful standard rename, send one `window/showMessage` warning when the target
     is identity-bearing (history not recorded) or `RenameOnline::Cold`; name the old/new keys
     and the recovery. Send a `window/showMessage` for every `RenameResult.warnings` entry
     (profile unavailable) as well. A parameter/local/case-only target sends no history
     warning.
6. `stc/rename` handler:
   - read `textDocument`, `position`, and `newName`;
   - call `Snapshot::rename`;
   - build the `changes` edit;
   - clear any pending record first, so a new rename invalidates the old token; if `history` is
     non-empty, generate a token and store `PendingRename { token, entries, created }`;
   - return `{ "edit": { "changes": ... }, "history": [...], "online": "...", "warnings":
     [...], "token": "..." }`; omit `token` when `history` is empty; include a warning for
     `RenameOnline::Cold` and every `RenameResult.warnings` entry (profile unavailable);
   - the handler writes nothing.
7. `stc/recordRename` handler:
   - read `token`;
   - reject a missing, stale, expired, or already-used token with `RequestFailed`;
   - take the pending entries and write them with the shared history helper (below), which
     reloads `stc.renames.toml` from the root directory (`Snapshot::manifest_path().parent()`),
     appends every entry, renders once, and writes once;
   - clear the pending record on success and on failure;
   - return `{ "recorded": true, "entries": N, "path": "<absolute path>" }`;
   - the server writes only `stc.renames.toml`, never source.
8. Token rules: one pending record per connection; a new `stc/rename` replaces it and
   invalidates the old token; a token is single-use; a five-minute TTL is enforced (add a
   `rename_token_ttl` field to `ServerOptions` so the unit test can use a short TTL while the CLI
   keeps the default). Generate the token from the process id and a monotonically increasing
   counter; it is opaque to the client.
9. Add `stc_manifest::append_renames_file(path, entries: &[(String, String)])` that loads,
   appends every entry, renders once, and writes once, with the same codes and messages as the
   CLI. Switch `crates/stc-driver/src/rename.rs` to it with one entry and no behavior change.
   Add unit tests for missing files, duplicates, malformed files, multiple entries, and write
   failures. (The write uses `std::fs::write`, matching the CLI; an atomic history write is a
   separate follow-up.)
10. Update the capability tests:
    - `crates/stc-lsp/src/server.rs` exact capability key set and
      `experimental.stc.trackedRename`;
    - `crates/stc-driver/tests/lsp_cli.rs` capability assertions.
11. Add `lsp_cli` tests:
    - multi-file standard rename over a temp project (declaration + references in `src/` and
      `tests/`), with exact URIs, ranges, and new text;
    - `prepareRename` range/placeholder, including a `VAR_ACCESS`/`VAR_CONFIG` segment;
    - invalid name, reserved keyword, standard function, dependency declaration, unresolved
      position, non-identifier position, and root namespace;
    - case-only rename: edits present, no history;
    - collision: existing declaration, case-insensitive, same-target case-only allowed, no
      edit and no token;
    - external references: `[project].configuration`, `[[io.bindings]].variable` (short,
      namespace, package-prefixed, and `instance.field`), `[[test.case]].name`, and a profile
      route; assert the exact TOML/profile edits;
    - `VAR_ACCESS`/`VAR_CONFIG` edits, including a nested path;
    - the identity-sensitive `window/showMessage` for a global rename and no warning for a
      local rename; the cold-download warning for a configuration rename;
    - a request from a file outside the source set is rejected; a manifest/profile position
      returns `null` from `prepareRename`;
    - unsupported methods still return method-not-found;
    - `stc/rename` returns the edit, canonical history, online classification, warnings, and
      token for a function and a field; no token for a parameter/local/configuration; case-only
      history empty;
    - `stc/recordRename` writes the history, rejects stale/expired/already-used tokens, and
      reports malformed/conflicting history with `RequestFailed`;
    - applying the `stc/rename` edits and calling `stc/recordRename` on temp copy B produces the
      same source and identity-bearing history as `stc rename` on temp copy A (the parity test);
      external-reference cases are excluded from the comparison and asserted separately;
    - the standard `textDocument/rename` does not write history.
12. Keep the server's existing behavior for open buffers, overlays, debounce, and UTF-16
    positions; use the analyzed snapshot text for ranges.

### 8.3 Protocol shapes

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
      ],
      "file:///project/stc.toml": [
        { "range": { "start": { "line": 9, "character": 16 },
                     "end": { "line": 9, "character": 22 } },
          "newText": "Advance" }
      ]
    }
  },
  "history": [ { "old": "AddStep", "new": "Advance" } ],
  "online": "preserved",
  "warnings": [],
  "token": "12345-7"
}

// stc/recordRename params
{ "token": "12345-7" }

// stc/recordRename result
{ "recorded": true, "entries": 1, "path": "/project/stc.renames.toml" }
```

Errors use JSON-RPC `RequestFailed` (-32803) with the engine's stable message or the
`stc-manifest` `STC-E-PROJECT-012/013` text. `online` is the string `"preserved"` or `"cold"`;
the cold reason is included in `warnings`.

### 8.4 Exit criteria

- F2 rename works over a multi-file project for every supported kind and edits the manifest,
  profile, and access paths.
- `prepareRename` returns a clear range or rejection; unsupported symbols produce a visible
  message, not a silent no-op.
- The standard method is history-free and warns for identity-bearing and cold-download targets.
- The private method returns the edit and history without writing; `recordRename` appends only
  after the token is confirmed and is single-use.
- Only rename is added to the capability set; all other unsupported capabilities remain
  unadvertised.
- `cargo test -p stc-lsp`, `cargo test -p stc-manifest`,
  `cargo test -p stc-driver --test lsp_cli`, clippy, and fmt pass; the phase commit is pushed
  and CI is green.

### 8.5 Evidence

`lsp_cli` output with exact source/manifest/profile/access edits and history, capability test
output, the parity test result, and the phase commit/CI link.

## 9. Phase 4 — VS Code tracked rename controller and watch integration

### 9.1 Goal

Make the VS Code extension use the tracked path: request the edit and token, return the edit to
VS Code, confirm the post-apply text of every edited file, record the history, and refresh the
server/watch after manifest/profile edits. Fall back safely to the standard method when the
server does not support the handshake.

### 9.2 Work

1. Add a pure module `editors/vscode/src/renameEdits.ts`:
   - `applyLspEdits(text, edits)` converts LSP line/character ranges to UTF-16 offsets, applies
     the edits in descending order, and throws on an overlap;
   - `shouldUseTrackedRename(initializeResult)` returns true only for
     `capabilities.experimental.stc.trackedRename === true`;
   - `editedUriKind(uri, manifestPath)` classifies a URI as source (`.st`), the root manifest
     (`manifestPath`), or profile (any other edited URI) for refresh handling;
   - no `vscode` import, so it is unit-testable with the existing esbuild test harness.
2. Add `editors/vscode/src/trackedRename.ts` with `TrackedRenameController`:
   - one controller per `ClientEntry`, holding the manifest path, the output channel, the
     `LanguageClient`, the `**/*.st` watcher, and the pending record;
   - `provideRenameEdits(document, position, newName, token, next)`:
     - if the capability is absent, call `next()` and warn once per session;
     - otherwise send `stc/rename`; on MethodNotFound (-32601), fall back to `next()` and warn
       once; on other errors, rethrow so VS Code shows the message;
     - convert the LSP edit with `client.protocol2CodeConverter.asWorkspaceEdit`; show any
       `warnings`;
     - for every URI in the edit, open the document, capture its current text as `original`,
       compute `expected` with `applyLspEdits`, and store both; the pending record's token is
       optional so history-free manifest/profile edits are still verified and refreshed;
     - finish or abandon any previous pending record before starting a new one;
     - return the converted edit.
   - change listeners: `workspace.onDidChangeTextDocument` and the `**/*.st` watcher's
     `onDidChange`; debounce a verification pass by 150 ms;
   - verification: open every edited document, compare its text with `expected`; when all
     match, call `stc/recordRename` when the token is present, always send
     `workspace/didChangeWatchedFiles` for edited manifest/profile files, clear the pending
     record, and log the result; when the deadline (5 s) passes:
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
4. Watch integration:
   - add the selected machine profile to `stc run --watch`'s snapshot so a profile edit triggers
     a rebuild;
   - add a watch test with a temp project and a profile edit.
5. Add `editors/vscode/test/rename-edits.test.mjs`:
   - multi-line edits, multiple edits in one file, edits across files, UTF-16 surrogate pairs,
     descending application, overlap rejection, and an empty edit list;
   - `shouldUseTrackedRename` with present, absent, and false capability shapes;
   - `editedUriKind` for source, manifest, and profile URIs, using the manifest path argument.
6. Add the editor smoke to `editors/vscode/test/electron/suite.js`:
   - `assertTrackedRename(document)` calls `vscode.executeDocumentRenameProvider` on the
     `AddStep` call site with `Advance`;
   - assert the returned `WorkspaceEdit` covers `src/main.st` and `tests/smoke.st`;
   - apply it with `vscode.workspace.applyEdit`; poll for `stc.renames.toml` with
     `old = "AddStep"` and `new = "Advance"`;
   - assert both files contain `Advance` and diagnostics stay clean;
   - `assertConfigurationRename()` renames `Sample` -> `SampleX`, applies the edit, asserts
     `stc.toml` and `src/main.st` contain the new name, and asserts no history entry was added;
     the cold-download warning is asserted in the `lsp_cli` test, not the headless smoke;
   - restore the original text of every edited file, save, delete `stc.renames.toml`, and wait
     for diagnostics; do the cleanup in a `finally` block so a failure does not leave the sample
     dirty;
   - keep the existing definition and diagnostics assertions.

### 9.3 Controller state machine

1. `provideRenameEdits` starts: finish any previous pending record, then check the capability.
2. Send `stc/rename`; receive `{ edit, history, online, warnings, token? }`.
3. Show any `warnings`; the token is optional (history-free target).
4. Read every edited document; compute `original` and `expected`; store
   `{ token: Option<String>, files, deadline }`.
5. Return the converted edit; VS Code applies it.
6. Change events debounce a verification pass. All files match `expected` -> `recordRename` when
   a token is present, always refresh manifest/profile URIs, and clear the record; all still
   match `original` at the deadline -> silent cancel; otherwise -> warning.
7. `recordRename` succeeds -> log; fails -> warn with the server's message and the recovery.
8. A new rename, a client stop, or a restart clears the pending record; no history is written
   for an unconfirmed rename.

### 9.4 Exit criteria

- A VS Code rename writes `stc.renames.toml` only after the edit is applied and verified.
- A cancel, undo, partial apply, or failed history write leaves the history unchanged and shows
  a warning where the user can see it.
- Manifest and profile edits are verified and trigger a server/watch refresh.
- An older server falls back to the standard history-free method and warns once.
- The smoke applies a function rename and a configuration rename, sees the history and manifest
  behavior, and restores the sample workspace.
- `npm run typecheck`, `npm test`, `npm run compile`, and `npm run test:electron` pass; the phase
  commit is pushed and CI is green.

### 9.5 Evidence

The extension unit-test output, the headless smoke output with the history and configuration
edits, the watch profile test, and the phase commit/CI link.

## 10. Phase 5 — Documentation, manual verification, PR, closeout

### 10.1 Documentation

- `specification/Architecture/IDE-Integration.md`: add `textDocument/prepareRename` and
  `textDocument/rename` to §3.2, remove rename from §3.3, and document
  `experimental.stc.trackedRename`, `stc/rename`, `stc/recordRename`, the post-apply
  confirmation, the standard history-free warning, the root-package/supported-kind rules, the
  external-reference edits, the cold-download policy, and the older-server fallback.
- `specification/Architecture/CLI-and-Manifest.md` §3.1: state that the tracked editor rename
  writes the same `stc.renames.toml` entries as `stc rename`, only after the client confirms the
  edit; it also edits `[project].configuration`, `[[io.bindings]].variable`,
  `[[test.case]].name`, profile routes, and `VAR_ACCESS`/`VAR_CONFIG` paths, while `stc rename`
  remains source-only in this release; parameters, locals, configurations, and case-only renames
  are history-free; the standard LSP method does not write history.
- `specification/Architecture/Online-Change-and-GOT.md` §7.3: say that `stc rename` and the
  VS Code tracked editor rename persist the mappings in `stc.renames.toml`; a history-free
  standard LSP rename does not; a configuration rename can change task-guard identities and may
  force a cold download.
- `USER-GUIDE.md` §8.3: add rename to the language features and remove it from the not-offered
  list; describe the VS Code tracked behavior, the manifest/profile edits, the standard
  history-free caveat, and the cold-download warning.
- `KNOWN-LIMITATIONS.md` §4: replace the closed-capability row with the remaining limitations
  (other editors get the history-free standard method; parameters/locals/configurations do not
  write history; `stc rename` is source-only; manifest/profile F2 entry points are not
  available; the case-only CLI defect is a follow-up).
- `editors/vscode/README.md`: add rename to the supported features and describe the history
  handshake and the manifest/profile edits.
- `specification/Decisions.md`: D-253.
- `FutureWishlist.md` at closeout: record the case-only CLI defect, the `stc rename` external-
  reference follow-up, the manifest/profile F2 entry points, and the full-build validation
  follow-up.

### 10.2 Verification and PR

1. Run the full local checks: `check_docs.py`, markdownlint, traceability, host audit,
   `git diff --check`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
   --locked -- -D warnings`, `cargo test --workspace --locked`, and the VS Code extension
   tests/smoke.
2. Run the manual walkthrough in Q-8 on the sample workspace, a temp `VAR_ACCESS`/`VAR_CONFIG`
   project, a temp IO/profile project, and a copy of `examples/online-change` with a live
   `stc run --watch`. Capture the exact steps, the history file, the watch output, and the
   before/after `GotId` comparison. Run the negative control.
3. Present the evidence to the owner; wait for approval (workflow §8).
4. Open the PR against `v3.0.0-planning` with the summary, changes, verification evidence, CI
   links, and exclusions (standard method history-free, CLI source-only, no manifest/profile F2
   entry points, no size cap, the case-only CLI follow-up).
5. Get the PR CI green; the owner merges. After approval, record the merge in
   `v3.0.0-plan.md` and `FutureWishlist.md`, delete the temporary plan, and clean up branches.

## 11. Cross-cutting constraints

- `stc-ide` stays `#![forbid(unsafe_code)]`; no new external dependency.
- No manifest-format or schema change; only internal spans and string edits.
- No CLI, manifest-format, runtime, DAP, or task-provider behavior change, except the `def_path`
  bug fix and the watch profile addition. The `append_renames_file` refactor is behavior-
  preserving for the CLI.
- No new `STC-*` diagnostic code; protocol errors use JSON-RPC `RequestFailed`.
- The capability set stays closed except for rename; `experimental.stc.trackedRename` is the
  only addition.
- The standard `textDocument/rename` is history-free; only the private tracked pair writes
  history.
- The server writes only `stc.renames.toml`, never source.
- One pending tracked-rename token per connection; latest wins; single-use; five-minute TTL.
- No timing threshold or CI timing gate.
- Keep the existing UTF-16 position conversion and analyzed-text rule; do not mix open-document
  and snapshot text for ranges.
- Branch from `v3.0.0-planning`; PR to `v3.0.0-planning`; the owner merges.
- New code targets 95% line coverage; the project gate stays 85%.

## 12. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| CLI and LSP edit collection drift | Parity test in `lsp_cli` against `stc rename` on the same fixture; the CLI remains the source-only behavioral reference; external-reference cases are asserted separately |
| Unsaved editor text changes the offsets | The request flushes the debounced analysis and uses the analyzed snapshot text, as definition/references do |
| A dependency declaration is renamed | Root-package check using `SourceFileInfo.package`; negative test |
| Case-insensitive names cause a false no-op | Allow case-only renames; write no history for them; test `AddStep` -> `addstep` |
| Path-prefix edits are missed | Test a namespace/POU path target and compare with the CLI |
| A manifest/profile reference is missed | Resolve every reference through the same root index and HIR instance data; tests for every spelling; manual IO/profile check |
| A manifest/profile edit uses the wrong segment | Replace the segment the resolver matched; test short, namespace, package-prefixed, and `instance.field` spellings |
| A manifest/profile value span includes quotes or a BOM | Store content spans after BOM handling and test basic strings, BOM files, and arrays of tables |
| A `VAR_ACCESS`/`VAR_CONFIG` segment is missed | HIR path spans from sema; tests for program-level, configuration-level, single-global, and nested paths |
| A rename creates a duplicate declaration | Root-index collision check with `STC-E-PROJECT-013`; source reanalysis for inherited/imported names |
| The collision check misses an inherited/imported name | In-memory source reanalysis before returning the edit; test the conflict |
| The profile is invalid or missing | Store the error in the snapshot; skip profile edits, add a `ProfileUnavailable` warning, and let the rename proceed; the project already has a build error |
| The server snapshot is stale after a manifest/profile edit | Controller sends `workspace/didChangeWatchedFiles`; server reloads |
| The watch misses a profile edit | Add the selected profile to the watch snapshot; test |
| A configuration rename changes task-guard identity | History-free, edit `[project].configuration`, warn about a possible cold download |
| A history-free standard rename is mistaken for a tracked rename | Standard warning for identity-bearing and cold targets; docs; D-253 |
| Local watch or remote online change rejects a history-free rename | The failure is visible (`STC-E-MANIFEST-003` or `STC-E-ONLINE-*`) and keeps the active generation; the warning and docs give the recovery; VS Code uses the tracked path |
| A data-root rename deploys with discarded state under `--allow-untracked-removal` | The tracked path preserves the `GotId`; the standard warning and docs state that state is reset |
| `[online].allow-untracked-removal` is not wired into build/watch/remote | Record as an existing gap outside WL-18; do not rely on the manifest setting in the docs |
| The extension verifies a partial apply or a stale buffer | Compare the full expected text of every edited file, including TOML; no match means no history and a warning |
| A history write fails after the source apply | `RequestFailed` with the `STC-E-PROJECT-012/013` text; the source stays applied; the warning gives the recovery |
| The server or extension restarts before confirmation | The token is lost and no history is written; docs state the recovery; the verification window is short |
| A new rename replaces a pending token | The controller finishes or abandons the previous record before starting a new one |
| The watch rebuilds between the source apply and the history write | The watch keeps the old generation and watches `stc.renames.toml`; the history write triggers the corrective rebuild; the manual test tolerates one transient rejection |
| The CLI case-only defect from §4.5 | The LSP writes no case-only history; record the CLI fix as a follow-up in `FutureWishlist.md` |
| The CLI leaves external references stale | Document the divergence; follow-up to extend `stc rename`; LSP tests assert the complete edit |
| The `def_path` fix changes CLI behavior | It only accepts a declaration the old code wrongly rejected; regression tests in `stc-hir` and the CLI |
| An older server lacks the capability | Fall back to `next()` and warn once per session |
| Multiple workspace folders | One controller and one pending token per language-client connection; the client routes the request by document |
| A concurrent CLI history write | Reload the history at `recordRename` time; a conflict is `STC-E-PROJECT-013`, not a silent overwrite |

## 13. Evidence checklist

- [ ] D-253 recorded and the plan/Phase 0 answers approved.
- [ ] Phase 1 manifest/profile/HIR span tests, `def_path` regression, identifier helper, and
      profile-loading tests green.
- [ ] Phase 2 `stc-ide` engine, history-key, external-reference, access-path, collision, and
      warning tests green.
- [ ] Phase 3 capability, standard rename, tracked-protocol, history-write, external-reference,
      access-path, and parity tests green.
- [ ] Phase 4 extension unit tests, typecheck, compile, headless smoke, and watch profile test
      green.
- [ ] Local checks (docs, lint, traceability, host audit, diff, fmt, clippy, tests) green.
- [ ] Manual sample, external-reference, and online-change walkthrough evidence approved by the
      owner, including the before/after `GotId` comparison and the negative control.
- [ ] PR against `v3.0.0-planning` green; owner merge recorded.
- [ ] `v3.0.0-plan.md` and `FutureWishlist.md` updated (including the case-only CLI, CLI
      external-reference, manifest/profile F2, and full-build validation follow-ups);
      temporary plan deleted.

## 14. Status log

| Date | Status |
| --- | --- |
| 2026-11-09 | Plan created; Phase 0 questions open; no implementation started. |
| 2026-11-09 | Online-change audit added (§4.4): a history-free rename is not identity-preserving; local watch and remote online change reject data-root, field, and type renames or reset state. |
| 2026-11-09 | Owner selected the tracked-rename scope. Plan rewritten around `experimental.stc.trackedRename`, `stc/rename`/`stc/recordRename`, the post-apply confirmation, canonical history keys, the standard history-free warning, and the older-server fallback. |
| 2026-11-09 | Discovered the case-only CLI defect (§4.5): `stc rename AddStep addstep` writes a self-conflicting history entry and the next build fails `STC-E-PROJECT-013`. The tracked LSP will write no case-only history; the CLI fix is proposed as a follow-up (Q-3f). |
| 2026-11-09 | Owner expanded the scope to edit external references and made identity preservation best-effort. Plan v3 adds `[project].configuration`, `[[io.bindings]].variable`, `[[test.case]].name`, profile route, `VAR_ACCESS`, and `VAR_CONFIG` edits; the collision check; the `def_path` fix; span retention; profile loading; the cold-download warning; the watch/refresh integration; and the CLI divergence/follow-ups. |
