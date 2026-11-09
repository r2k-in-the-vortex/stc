# WL-17 — Manifest and task follow-ups from v2.0.0

Status: **Plan approved; Phase 0 decisions recorded** (2026-11-09). No implementation has
started.

- Branch: `WL-17`
- Base: `v3.0.0-planning` at `1b9860b`
- Locked scope source: `v3.0.0-plan.md` §WL-17 and `FutureWishlist.md` §WL-17
- Delivery process: `workflows/FEATURE-DEVELOPMENT.md`

## 1. Goal

Close the two WL-1.1 close-out follow-ups that are local to the project loader, task
provider, and VS Code extension:

- **F-23:** allow a project without tests and defer test-source errors until a command needs
  them, after auditing the other optional project-file items.
- **F-24:** apply `expandTaskVariables` to every task `args` entry and document the variable
  set and unknown-variable rule.

## 2. Locked scope

- F-23: audit required versus optional project-file items and empty source sets; allow a
  project without tests and defer errors until the command that needs them. This amends
  `STC-E-PROJECT-004`, the conformance profile, and the CLI and language-server documentation.
- F-24: apply `expandTaskVariables` to each `args` entry and document the expansion so
  configured tasks behave consistently.
- Keep the existing manifest identity and required project semantics; change only items the
  audit shows should be optional or deferred.

## 3. Exclusions and non-goals

- No manifest redesign and no change to required project identity (`schema`, `[package]`) or
  required production source semantics (`[project].sources`).
- No F-10/F-13 work; those remain documented limitations.
- No remote or multi-user project loading work.
- No new diagnostic code. F-23 reuses `STC-E-PROJECT-004`; F-24 does not add a diagnostic.
- No new timing threshold or CI timing gate.
- No WL-21 or other WL work on this branch.

## 4. Current-state audit

### 4.1 Manifest items

The loader parses every top-level table in `crates/stc-manifest/src/parse.rs`. The audit
confirms one behavior change: the no-match result for `[test].sources`. Everything else keeps
its current required/optional semantics.

| Item | Required? | Current load behavior | Proposed WL-17 behavior |
| --- | --- | --- | --- |
| `schema` | Yes | Missing or unsupported value is `STC-E-PROJECT-002` | Unchanged |
| `[package]` (`name`, `version`, `language`) | Yes | Missing/invalid identity is an error | Unchanged |
| `[project]` and `[project].sources` | Yes | Missing table, missing list, empty list, invalid glob, or no match is `STC-E-PROJECT-004` | Unchanged; production sources stay required and fail at load |
| `[dependencies]` | No | Absent means no dependencies; present-but-invalid declarations are errors | Unchanged |
| `[build]` | No | Absent uses defaults; present-but-invalid settings are errors | Unchanged |
| `[io]` devices, channels, bindings | No | Absent means no I/O; present-but-invalid declarations are errors | Unchanged |
| `[retain]`, `[runtime]`, `[online]`, `[debug]`, `[lints]`, `[run]` | No | Absent uses defaults; present-but-invalid settings are errors | Unchanged |
| `[test]` | No | Absent uses test defaults | Unchanged |
| `[test].sources` | No | Absent or `[]` means no test sources; a non-empty glob that matches nothing is `STC-E-PROJECT-004` **at project load** | Absent or `[]` means no tests; a non-empty glob that matches nothing is accepted at load and reported only by commands that need tests (Q-2) |
| `[[test.case]]` | No | Absent means no overrides; an unknown case name is already deferred to `stc test` as `STC-E-PROJECT-008` | Unchanged |

Conclusion: only `[test].sources` no-match becomes lazy. `[project].sources` remains required,
so a genuinely missing production source still fails at load. The other optional tables either
default when absent or represent present-but-invalid configuration; deferring those would hide
real configuration errors, so they stay load-time errors. `stc new` is unchanged.

The audit was exercised with `target/debug/stc.exe check` on temporary projects. A minimal
project, empty `[dependencies]`/`[io]`/`[retain]`/`[test]` tables, empty `[io]` device and
binding arrays, `[retain] enabled = true` with no retained variables, `[test] sources = []`,
and a `[test]` table with defaults only all exited 0. Only the non-empty unmatched
`[test].sources` glob exited 2 with `STC-E-PROJECT-004`.

### 4.2 Evidence for the current behavior

- `crates/stc-manifest/src/discover.rs:118-124` emits `STC-E-PROJECT-004` when a glob matches
  no files.
- `crates/stc-manifest/src/graph.rs:694-731` runs the same discovery for production and test
  globs, so a test glob no-match fails the shared project load.
- `crates/stc-manifest/src/parse.rs:1453-1455` makes `[test].sources` optional; `parse_globs`
  already accepts an empty array.
- `crates/stc-driver/src/test.rs:786-788` already handles a zero-test project with
  `0 tests discovered` and exit 0.
- `editors/vscode/src/tasks.ts:185` expands `project`; `editors/vscode/src/tasks.ts:147-149`
  appends `args` unchanged; `expandTaskVariables` at lines 207-223 supports
  `${workspaceFolder}`, `${workspaceFolderBasename}`, and `${workspaceRoot}` and leaves other
  variables literal.
- Current user-facing statements: `KNOWN-LIMITATIONS.md:95`, `USER-GUIDE.md:367-368` and
  `:684`, `specification/01-Conformance-Profile.md:406`, and
  `specification/Architecture/CLI-and-Manifest.md:957`.

## 5. Phase 0 — Close owner questions and record decisions

Goal: resolve every question that blocks implementation and record durable answers.

Work:

1. Put the questions in §5.1 to the owner. Do not start implementation before the answers
   are recorded.
2. Record the accepted F-23 and F-24 semantics in `specification/Decisions.md` with the next
   free `D-` numbers (currently D-251 and D-252) or as amendments to an existing decision.
   Keep original decision text intact; mark superseded text rather than rewriting it.
3. Update this plan with the owner answers and remove any ambiguity from the phase criteria.
4. If the audit question reveals another item that should become lazy, stop and ask before
   widening scope.

Exit criteria:

- Every question in §5.1 has an owner answer.
- The decisions register contains the durable F-23 and F-24 rules.
- This plan is updated and approved.
- No code has been written.

Evidence: this plan's answer table, the decisions-register entries, and the owner approval.

Status: complete 2026-11-09. The owner approved the plan and answered all questions; D-251 and
D-252 are recorded. The Phase 0 commits `ddb05a5` and `7077b2a` are pushed and green on CI
(see §13).

### 5.1 Owner questions

**Q-1. Which project items become lazy, and does that change `stc check` diagnostics?**

Recommendation: only `[test].sources` no-match becomes lazy. `[project].sources` stays
required and a no-match remains `STC-E-PROJECT-004` at load. `stc check` stops reporting
`STC-E-PROJECT-004` for an unmatched test glob but still reports it for production globs and
for every other project-load error. The §4.1 audit table documents the other optional items
as unchanged.

Options:

- (a) Recommended: only `[test].sources` no-match; all other load diagnostics unchanged.
- (b) Also defer some other item; the owner names it.
- (c) Keep the current load behavior for `[test].sources` and close F-23 as documentation-only.

**Q-2. What does an empty test source set mean for `stc test`?**

Recommendation:

- No `[test]` table, no `[test].sources`, or `[test].sources = []`: no tests are configured.
  `stc test` keeps the existing `0 tests discovered` result and exit 0.
- A non-empty `[test].sources` list where one or more globs match no files: every non-test
  command loads the project, and `stc test` and `stc test --list` report
  `STC-E-PROJECT-004` for each unmatched glob and exit 2 (the manifest/project-error code).
- A test DAP launch (`stc dap` with `test`) surfaces the same deferred error before starting
  the host; an ordinary launch, LSP, `stc fmt`, watch, rename, and remote ignore it.

Options:

- (a) Recommended: the rule above.
- (b) Any empty test source set means "no tests": `stc test` always reports 0 tests and exits
  0, and the no-match error is never deferred.
- (c) Any `[test]` table with no discovered tests is an error at `stc test`.
- (d) The rule above but a different exit code for the deferred error.

**Q-3. Should all task arguments use one variable set, and how are unknown variables reported?**

Recommendation: use one variable set for `project` and every `args` entry:
`${workspaceFolder}`, `${workspaceFolderBasename}`, and `${workspaceRoot}`. Apply it to all
five commands (`check`, `build`, `run`, `test`, `fmt`) and to both the `ShellExecution` path
and the `run` custom terminal. Any other `${...}` token is left literal and documented; no new
diagnostic is added, so `project` and `args` behave identically.

Options:

- (a) Recommended: known variables expand; unknown variables stay literal and are documented.
- (b) Known variables expand; each unknown variable also writes a warning line to the `stc`
  output channel, and the task still runs.
- (c) Known variables expand; an unknown variable fails task resolution with a visible error.
- (d) A different variable set or behavior; the owner specifies it.

**Q-4. Is Phase 2's task-provider change accepted only after a manual VS Code check?**

Recommendation: yes. Phase 2 is a UI-facing task-provider change. Accept it after green CI
plus the owner's manual check of a configured task whose `args` contain
`${workspaceFolder}` (and, if chosen, an unknown variable). Phase 1 is automatable and can be
accepted on green CI plus its test evidence.

Options:

- (a) Recommended: owner manual acceptance for Phase 2.
- (b) Green CI plus unit tests is enough for Phase 2; the final pre-PR manual walkthrough
  covers the UI.

### 5.2 Answer record

| Question | Owner answer | Decision |
| --- | --- | --- |
| Q-1 | Optional items are lazy in the narrow sense: absence, empty tables/arrays, and no-op declarations must not fail load. The audit shows `[dependencies]`, `[io]`, and `[retain]` already comply; only `[test].sources` no-match needs a code change. Present-but-invalid content still fails at load; `[project].sources` stays required. `stc new` is unchanged. | D-251 |
| Q-2 | Deferred error: no `[test]`/no sources/`[]` means 0 tests and exit 0; a non-empty unmatched test glob is reported by `stc test` and `stc test --list` with `STC-E-PROJECT-004` and exit 2; a test DAP launch surfaces the same error. | D-251 |
| Q-3 | One variable set for `project` and every `args` entry (`${workspaceFolder}`, `${workspaceFolderBasename}`, `${workspaceRoot}`), all five commands and both execution paths; unknown `${...}` tokens stay literal and are documented. | D-252 |
| Q-4 | Owner manually accepts Phase 2 after green CI plus a manual VS Code task check. | Plan gate |

## 6. Phase 1 — F-23 loader and deferred test-source errors

Goal: a project without tests, or with test source globs that match nothing, loads for every
non-test consumer; commands that need tests report the deferred error.

The §4.1 audit is the acceptance evidence for the other optional items. Do not add loader
changes for them unless a regression test shows a no-op failure; stop and ask if one does.

Work:

1. In `crates/stc-manifest/src/discover.rs`, distinguish required globs from optional globs.
   Required globs keep the no-match diagnostic. Optional globs return the unmatched
   `GlobSpec`s separately and still return every real error (invalid pattern, unreadable
   directory, duplicate match, symlink outside the project).
2. In `crates/stc-manifest/src/graph.rs`, run production discovery in required mode and test
   discovery in optional mode. Store the unmatched test globs on the package or graph so a
   test consumer can report them without re-walking the tree. Keep production/test overlap
   (`STC-E-TEST-004`) and all other load errors unchanged.
3. In `crates/stc-driver/src/test.rs`, report the deferred `STC-E-PROJECT-004` diagnostics for
   the root package before reading or discovering tests, per Q-2. Keep the existing zero-test
   path for projects with no test source declaration.
4. In `crates/stc-dap/src/source.rs`, surface the same deferred diagnostics when
   `include_tests` is set, per Q-2. Ordinary launches keep the current behavior.
5. Do not change `stc check`, LSP, `stc fmt`, watch, rename, or remote behavior beyond the
   loader no longer failing; add regression tests that those consumers load an unmatched
   test glob successfully.
6. Update the durable documentation listed in §8.
7. Add tests:
   - `[test]` absent, `[test].sources = []`, and a non-empty test glob with no match all load.
   - A non-empty test glob with one matching and one non-matching glob keeps the matching
     sources and defers only the non-matching glob.
   - A production glob no-match still fails with `STC-E-PROJECT-004`.
   - Invalid test glob syntax, duplicate matches, and a symlink outside the project still
     fail at load.
   - `stc check` exits 0 for a project with an unmatched test glob.
   - `stc test` and `stc test --list` follow Q-2; a test DAP launch follows Q-2.
   - LSP snapshot load succeeds for an unmatched test glob.

Exit criteria:

- A project without `[test]`, with `[test].sources = []`, or with a non-empty test glob that
  matches nothing loads and passes `stc check`.
- A project with a genuinely missing production source still fails at load with
  `STC-E-PROJECT-004`.
- Commands that need tests report the deferred error per Q-2; commands that do not need tests
  do not.
- The loader, CLI, LSP, DAP, and test-discovery behavior is consistent.
- New code has 95% line coverage; project coverage stays at or above 85%.
- Documentation is updated; local checks and CI are green.

Evidence: loader tests in `crates/stc-manifest/tests/sources.rs`; CLI tests in
`crates/stc-driver/tests/`; DAP source tests; LSP snapshot test; the exact commands and output;
the phase commit; the CI run link.

## 7. Phase 2 — F-24 task-argument expansion

Goal: every task argument uses the same variable expansion as `project`, for every command and
both execution paths.

Work:

1. Extract the pure argument-building and variable-expansion logic into a module that does not
   import `vscode` (for example `editors/vscode/src/taskArgs.ts`), or otherwise provide a pure
   test seam. Keep `tasks.ts` as the VS Code wiring.
2. Apply `expandTaskVariables` to each `args` entry before appending it to the generated
   argument list. Do this for `check`, `build`, `run`, `test`, and `fmt`, and for both
   `ShellExecution` and the `run` custom terminal.
3. Keep `project` expansion and all generated arguments unchanged. Do not double-expand a
   value that VS Code already resolved; the known-variable replacement is idempotent for the
   three supported variables.
4. Implement the Q-3 unknown-variable rule and document it.
5. Add `editors/vscode/test/tasks.test.mjs` (or extend the existing test style) that bundles
   the pure module with esbuild and covers:
   - all five commands and their generated argument lists;
   - `--release` only for `build` and `run`;
   - diagnostic arguments only for `check`, `build`, `run`, and `test`;
   - `args` appended after generated arguments;
   - every supported variable in each `args` entry and in `project`;
   - the Q-3 unknown-variable behavior;
   - the same argument list for the `run` custom-terminal path.
6. Update the durable documentation listed in §8.

Exit criteria:

- Every task type expands the supported variables in every `args` entry.
- `ShellExecution` and the `run` custom terminal receive the same expanded arguments.
- Unknown variables follow the Q-3 rule.
- `editors/vscode` typecheck, tests, and package checks pass.
- New code has 95% line coverage where the coverage tooling applies; project coverage stays at
  or above 85%.
- Documentation is updated; local checks and CI are green.
- The owner manually accepts the task-provider behavior per Q-4.

Evidence: `editors/vscode/test/tasks.test.mjs` output, `npm run typecheck`, `npm test`,
`npm run package`; the manual VS Code task command line and output; the phase commit; the CI
run link; the owner acceptance record.

## 8. Documentation updates

F-23:

- `specification/Architecture/CLI-and-Manifest.md` §2.3: state that a production glob that
  matches nothing still fails at load, while a test glob that matches nothing is deferred to
  the test command; §2.4: state that `[test].sources` is optional and that a project without
  tests may omit `[test]` or use an empty list.
- `specification/01-Conformance-Profile.md`: amend the `STC-E-PROJECT-004` row to distinguish
  production and test source globs and to name the deferred test-command behavior.
- `specification/Architecture/IDE-Integration.md`: state that the language server initializes
  for a project with an unmatched test glob and that the error is deferred to test commands.
- `USER-GUIDE.md` §8.1 and the troubleshooting table: remove the "project without tests must
  omit `[test]`" limitation and describe the new behavior.
- `KNOWN-LIMITATIONS.md` §4: remove the F-23 limitation row once the behavior is delivered.
- `specification/Decisions.md`: the Phase 0 entries.

F-24:

- `editors/vscode/README.md` task table: document expansion in `project` and every `args`
  entry, the supported variable set, and the unknown-variable rule.
- `USER-GUIDE.md` §8.2: the same user-facing rule.
- `specification/Architecture/IDE-Integration.md`: add a short task-provider subsection under
  §2 describing the `stc` task type, generated arguments, and the expansion rule; update §6
  verification to include the task-argument tests.

Do not update the WL status/change-log rows in `v3.0.0-plan.md` or `FutureWishlist.md` until
the closeout in Phase 3; those are delivery records, not implementation docs.

## 9. Phase 3 — Manual verification, PR, and closeout

Goal: prove the delivered behavior, open the PR, and record the merge.

Work:

1. Run the full local checks that apply:
   `python tools/ci/check_docs.py`, `markdownlint`,
   `cargo xtask traceability --check`, `cargo xtask host-audit --check`, and
   `git diff --check`.
2. Build from the branch and run the manual walkthrough:
   - a project without `[test]`: `stc check` and `stc test`;
   - a project with `[test].sources = []`: `stc check` and `stc test`;
   - a project with a non-empty test glob that matches nothing: `stc check` passes, `stc test`
     and `stc test --list` follow Q-2, and the language server starts;
   - a project with a missing production source: still fails at load;
   - a VS Code task whose `args` contain `${workspaceFolder}` and, if chosen, an unknown
     variable: confirm the expanded command line and output.
3. Present the commands, output, logs, and any screenshots to the owner and get approval.
4. After the owner approves the manual walkthrough, update the WL-17 target row, status,
   retirement checklist, and change log in `v3.0.0-plan.md` and `FutureWishlist.md`, and
   delete this temporary plan from the tree.
5. Open the PR against `v3.0.0-planning`, never `main`, with a concrete body: summary, what
   changed, verification evidence, CI links, and exclusions/known limitations. Record the PR
   number in the planning documents. Get the PR CI green and report the run link. Leave
   merging to the owner.
6. After the owner merges:
   - record the merge hash in the planning documents;
   - delete the local and remote `WL-17` branches and prune stale remote-tracking refs;
   - run a full CI pass on the parent branch after the merge record is pushed;
   - leave the working tree clean.

Exit criteria:

- The manual walkthrough is approved by the owner.
- The PR is open against `v3.0.0-planning` with green CI.
- The owner merges; the closeout records and branch cleanup are complete.
- The parent branch is green after the merge record.

## 10. Cross-cutting constraints

- Do not start implementation before the owner approves this plan and answers §5.1.
- Hand one phase at a time to a subagent. Give it this plan, the phase goal, the locked scope,
  the constraints, and the definition of done. Require the changed files, commands run, test
  results, and remaining questions.
- Every phase ends with pushed commits and a green CI run covering the changed surface.
  Pushing `WL-17` may not trigger CI; use `gh workflow run ci.yml --ref WL-17` when needed.
- Do not write new code while CI is running. Stop and ask after two consecutive CI failures.
- Small logical commits with conventional prefixes (`feat:`, `fix:`, `test:`, `docs:`,
  `refactor:`). Never amend or rewrite pushed history.
- New code targets 95% line coverage; the project gate is 85%.
- No timing threshold or CI timing gate is added.
- The owner is the only decision authority and the only person who merges the PR.
- If a question changes scope, semantics, contracts, or user-visible behavior, stop and ask.

## 11. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Project loading is shared by CLI, LSP, DAP, IDE, watch, fmt, rename, and remote | Change only the shared loader policy; add a consumer matrix test for unmatched test globs |
| Deferring the test no-match error loses the diagnostic | Store the unmatched `GlobSpec`s explicitly; report each one from the test consumer; test partial matches |
| Allowing empty test globs masks a genuinely missing production source | Keep `[project].sources` in required mode; test the production no-match case |
| A test DAP launch starts with no test sources | Surface the deferred diagnostics when `include_tests` is set, per Q-2 |
| Task-provider logic is hard to test because it imports `vscode` | Extract a pure argument/expansion module and bundle it in the `.mjs` test |
| VS Code already expanded a variable before the extension sees it | The three replacements are idempotent; the manual task check confirms the final command line |
| Documentation drift across CLI, LSP, user guide, extension README, and conformance profile | Update every document in §8 in the same phase; run the docs check |

## 12. Evidence checklist

- [x] Phase 0: owner answers recorded; decisions register updated; plan approved; phase commit
      green on CI.
- [x] Phase 1: loader/CLI/LSP/DAP tests; docs; local checks; phase commit; green CI link.
- [ ] Phase 2: task-provider tests; docs; typecheck/test/package; phase commit; green CI link;
      owner manual acceptance.
- [ ] Phase 3: manual walkthrough evidence; owner approval; PR with green CI; merge record;
      closeout; parent-branch CI; clean tree.

## 13. Status log

| Date | Phase | Status | Commit | CI |
| --- | --- | --- | --- | --- |
| 2026-11-09 | Plan | Approved by owner | — | — |
| 2026-11-09 | Phase 0 | Accepted: D-251/D-252 recorded; audit evidence added | `ddb05a5` | [42664026651](https://github.com/r2k-in-the-vortex/stc/actions/runs/42664026651): attempt 1 macOS smoke `cancelled` (infrastructure flake, reported); attempt 2 green |
| 2026-11-09 | Workflow | Flaky-test reporting rule added to `workflows/FEATURE-DEVELOPMENT.md` | this docs commit | dispatched after push |
| 2026-11-09 | Phase 1 | Accepted: F-23 loader, CLI, LSP, and DAP behavior; docs and regression tests added; local checks green | `cffe301`, `7452b8b`, `8b9287e`, `7b35e71`, `9823d9a` | [42675783819](https://github.com/r2k-in-the-vortex/stc/actions/runs/42675783819): green |
