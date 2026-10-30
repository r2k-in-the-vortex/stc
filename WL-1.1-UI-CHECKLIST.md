# WL-1.1 Manual VS Code UI Checklist

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-30  
Sample: `editors/vscode/samples/workspace`  
Extension: latest local `editors/vscode/stc-vscode-0.1.0.vsix`  
Compiler: the binary `stc.path` points at; use the latest debug or a rebuilt release binary

Result key: unchecked = pending, `[x]` = pass, **FAIL** = failed with a note, **N/A** =
unsupported or intentionally out of scope.

## 0. Prerequisites

- [x] **0.1 Install the latest local VSIX.** Pass 2026-10-30: installed
      `editors/vscode/stc-vscode-0.1.0.vsix --force`. The VS Code CLI `DEP0169 url.parse()`
      warning is from VS Code itself, not the extension.
- [x] **0.2 Point `stc.path` at the latest binary.** Pass 2026-10-30: release path
      `C:/repos/stc3/target/release/stc.exe`; binary built 09:09, server `PID 35220` started
      09:16. Verified start/end-of-word definition against that exact build.
- [x] **0.3 Open the sample workspace** at `editors/vscode/samples/workspace` as the workspace
      folder, then open `src/main.st`. Pass 2026-10-30.
- [x] **0.4 Confirm the server.** Pass 2026-10-30: Output → **stc Language Server** shows
      `stc-lsp: serving ...samples/workspace/stc.toml`; status bar is **Structured Text**;
      Problems panel is empty.

## A. Activation and editing

1. [x] **Syntax highlighting** — pass 2026-10-30. Inspected scopes:
       `keyword.declaration.st` (`VAR_GLOBAL`), `constant.language.st` (`TRUE`), and
       `string.quoted.double.st` (`the accumulator should add each step`). Types, comments,
       numbers/durations, pragmas, direct variables, and operators are scoped. The palette is
       dominated by red and white with the current theme; scope and color-scheme polish is
       recorded as follow-up **F-1**, not a manual-pass failure.
2. [x] **Comment toggling** — pass 2026-10-30. Line toggle via `Ctrl+ä` (VS Code's binding for
       this Estonian layout; inserts/toggles `//`), multi-line toggle on lines 39–42, block
       comment via `Shift+Alt+A` (`(* ... *)`), and nested block-comment highlighting all pass.
3. [x] **Brackets and quotes** — pass 2026-10-30: `(`/`[`/`{` auto-close with step-over,
       `'`/`"` auto-close strings, auto-close is suppressed in comments and strings, matching
       brackets highlight, and a selection is wrapped by surrounding pairs.
4. [x] **Format document** — pass 2026-10-30: a deliberately misindented line with
       `sum:=AddStep(sum);` was normalized to two-space indentation and `sum := AddStep(sum);`;
       the second run made no change and added no undo step.

## B. Diagnostics and tasks

5. [x] **Clean diagnostics** — pass 2026-10-30: the Problems panel is empty for `src/main.st`
       and `tests/smoke.st`, no editor squiggles, and the language-server output has no errors.
6. [x] **Error diagnostics** — pass 2026-10-30. `cycles := Missing + 1;` produced
       `STC-E-NAME-001` (`unknown name \`missing\``, severity 8/Error) from source `stc` at
       line 41, columns 13–20; Problems panel and editor squiggle agree. Undo clears it.
7. [x] **`stc: check` task** — pass 2026-10-30 after the fix. The configured `stc: check` initially
       failed with a doubled cwd because the custom `project` property
       `${workspaceFolder}/stc.toml` was not variable-expanded; `src/tasks.ts` now expands
       `${workspaceFolder}`, `${workspaceFolderBasename}`, and `${workspaceRoot}`, and the
       Electron smoke prefers the configured task. Retested: exits 0 with no diagnostics.
8. [x] **`stc: test` task** — pass 2026-10-30: `AccumulatorWorks` and `AddStepWorks` both pass;
       `test result: ok. 2 passed; 0 failed; 0 timed out; 0 trapped`; exit 0.
9. [x] **`stc: build` task** — pass 2026-10-30: exits 0 and writes the debug artifacts under
       `target/stc/debug`; the `STC-W-MANIFEST-001` cold-layout warning is normal; Problems empty.
10. [x] **`stc: fmt` task** — pass 2026-10-30: a deliberately malformed line
        (`cycles:=cycles+1;` with extra indentation) was rewritten to `cycles := cycles + 1;`;
        a second run made no change; Problems stayed empty.
11. [x] **Problem matcher** — pass 2026-10-30: `STC-E-NAME-001` on `src/main.st` line 41,
        columns 13–20, appears once (no duplicate with the language server), clicking it navigates
        to the line, and the entry clears when the edit is undone.
12. [ ] **`stc: run` task** — FAIL on clean stop 2026-10-30. The task starts (the
        `STC-W-MANIFEST-001` cold-layout warning is normal) and now passes
        `--debug-server --debug-endpoint target/stc/debug/debug.endpoint.json` so attach can be
        retested (F-2). Terminating the task left `stc run` (PID 33308) and `sample.exe`
        (PID 39704) running; they were killed manually with `taskkill /T /F`. Recorded as
        follow-up **F-3**.
        Note: the provider returns all five detected tasks (`stc check, stc build, stc run,
        stc test, stc fmt`), but VS Code's default two-level picker shows only configured
        `tasks.json` entries. The configured `stc: run` task was added to the sample; setting
        `"task.quickOpen.showAll": true` also exposes the detected tasks in one flat list.

## C. Language server navigation

13. [x] **Go to definition — function call.** pass 2026-10-30 on the normal path: F12 on
        `AddStep` (start, middle, and end of the word) jumps to the `FUNCTION AddStep`
        declaration.
        Note: an earlier failure came from a persisted `\\?\C:\...` editor tab. That form is not a
        supported UI route and is no longer produced; the CLI's absolute diagnostic path is a
        normal `C:\...` path. The verbatim prefix is still normalized defensively in `stc-ide` and
        `stc-lsp`, with regression tests, and the server logs any unmapped document URI.
14. [x] **Go to definition — FB type and instance.** pass 2026-10-30: F12 on `Accumulator`
        jumps to `FUNCTION_BLOCK Accumulator`; F12 on the call receiver on line 40 jumps to the
        field declaration on line 35 (start/middle/end). On a declaration, F12 goes to the first
        reference (or cycles references for `sum`), which is VS Code's fallback behavior and is
        preferable to jumping to the same line.
15. [x] **Go to definition — variables and labels.** pass 2026-10-30: F12 on `sum`, `cycles`,
        `INCREMENT`, `TOTAL`, and the named labels `increment`/`total` on line 40 jumps to the
        corresponding declarations (line 36, line 37, line 8, line 9, line 25, and line 28).
16. [x] **Find references.** `Shift+F12` on `INCREMENT` finds the declaration (line 8) and the
        uses on lines 19 and 40; on `Accumulator` it finds the FB declaration (line 23), the type
        use on line 35, and the type use in `tests/smoke.st` line 16; on `sum` it finds the
        declaration (line 36) and all uses. Include-declaration is honored.
        Pass 2026-10-30. An earlier release build also listed the same-spelled instance
        `accumulator` on `src/main.st` line 40 and `tests/smoke.st` lines 19–20; fixed in
        `stc-sema` by recording the instance field on an FB call callee instead of the FB type,
        and covered by `navigation.rs`. Retest on the rebuilt release binary passes: the
        `Accumulator` result is exactly the FB declaration and the two type uses.
17. [ ] **Hover.** `Ctrl+K Ctrl+I` on `sum`, `AddStep`, and `Accumulator` shows the declared type,
        declaration kind, and owner.
18. [ ] **Completion.** `Ctrl+Space` in a `PROGRAM Main` body offers keywords, visible names
        (`INCREMENT`, `AddStep`, `accumulator`, `sum`), and after `accumulator.` offers the FB
        members.
19. [ ] **Document symbols.** `Ctrl+Shift+O` lists `AddStep`, `Accumulator`, `Main`, and
        `Sample`; the entries are hierarchical and jump to the right declaration.
20. [ ] **Cross-file navigation.** From `tests/smoke.st`, F12 on `AddStep` and `Accumulator`
        jumps to `src/main.st`; references from the test file are included.

## D. Debugging

21. [ ] **Breakpoint gutter.** Click the gutter on line 39 (`sum := AddStep(sum);`); a red dot
        appears and the Breakpoints panel lists it.
22. [ ] **Launch.** Press F5 and choose **stc: Launch sample**. The session builds, starts, and
        stops at line 39. The Debug Console shows the non-safety notice.
23. [ ] **Globals.** The Variables pane → **Globals** shows `INCREMENT = 1`; expand the program
        instance to see `sum`, `cycles`, `accumulator`, and `TOTAL`.
24. [ ] **Call stack.** The Call Stack shows `Main`; continue (F5) to the `AddStep` breakpoint if
        one is set, or step into the function.
25. [ ] **Step into.** With the stop on line 39, press F11 (`Step Into`); the session stops in
        `AddStep`. **Locals** shows `VALUE` and `INCREMENTED = 0`; the stack shows `AddStep` above
        `Main`.
26. [ ] **Step over.** Press F10 on line 19; execution moves to line 20 and `INCREMENTED = 1`.
27. [ ] **Step out.** Press `Shift+F11` from inside `AddStep`; execution returns to `Main`.
28. [ ] **Watch and evaluate.** Add `INCREMENT` and `sum` to the Watch pane; values appear at the
        next stop. In the Debug Console, `INCREMENT` returns `1`; an arbitrary ST expression is
        rejected with a clear message.
29. [ ] **setVariable gating.** Without `allowWrite`, editing a variable in the Variables pane is
        refused. Set `"allowWrite": true` in the launch configuration and relaunch; editing a
        live root succeeds. Revert the value and the configuration afterwards.
30. [ ] **Terminate and relaunch.** `Shift+F5` ends the session; the launch-owned host exits. F5
        launches a fresh session successfully.
31. [x] **Attach.** pass 2026-10-30 using the `stc: run` task, which now passes
        `--debug-server --debug-endpoint target/stc/debug/debug.endpoint.json` (F-2). The
        **stc: Attach sample** session connected, a breakpoint stopped, and disconnect left the
        host running. Stopping the run task still leaves its process tree (F-3), so the driver and
        `sample.exe` were killed manually; the `stc lsp` process remained.
32. [ ] **Conditional breakpoints.** Right-click the gutter → **Add Conditional Breakpoint**.
        The adapter does not support conditions; record the exact behavior (VS Code may disable
        the action or the condition may be ignored).
33. [ ] **Release monitoring-only.** Launch with `"release": true`; the session starts without
        breakpoints, stepping, or a Locals scope and reports the limitation honestly.

## E. Safety and limits

34. [ ] **Non-safety notice.** The Debug Console shows the local-session notice; the extension
        does not present debugging as a safety function.
35. [ ] **Local-only.** No remote/network debug or language-server path is offered.
36. [ ] **Known unsupported UI actions.** Record the behavior of semantic tokens, rename, code
        actions, range formatting, call hierarchy, workspace symbols, restart, multi-thread, and
        `stopOnEntry`: they must be absent or fail honestly, not silently misbehave.

## Follow-ups discovered during verification

- [ ] **F-1 — Highlighting scope and color scheme.** The grammar's scopes are correct for the
      tested tokens (`keyword.declaration.st`, `constant.language.st`,
      `string.quoted.double.st`), but the resulting palette is dominated by red and white.
      Enrich the grammar with standard semantic scopes — declared POU/type names
      (`entity.name.function.st` / `entity.name.type.st`), primitives
      (`support.type.primitives.st`), variables (`variable.name.st`), parameters and members
      (`variable.parameter.st`), control keywords (`keyword.control.*`), and built-in functions
      (`support.function.st`) — using
      [Serhioromano/vscode-st](https://github.com/Serhioromano/vscode-st) as a reference. That
      extension ships a grammar but no theme, so also decide whether to ship a small color theme
      contribution or document a recommended theme. Acceptance: distinct
      keyword/type/declaration/variable/comment/string/number/pragma/direct-variable colors in a
      standard dark and a standard light theme, verified with **Developer: Inspect Editor Tokens
      and Scopes**.

- [x] **F-2 — Make the sample `stc: run` task debuggable.** Pass 2026-10-30: the task passes
      `--debug-server --debug-endpoint target/stc/debug/debug.endpoint.json` (relative to the task
      cwd), the endpoint file is created, **stc: Attach sample** connects, and disconnect leaves
      the host running. If a user-defined task needs a variable in `args`, the provider should
      also expand `${workspaceFolder}` there.

- [x] **F-5 — Debug stops open a `\\?\` source tab.** Fixed 2026-10-30 and retested: `stc-dap`
      normalizes absolute source paths (package entries and the `sourceRoot` override) before
      emitting DAP `Source.path`; a breakpoint stop now uses the normal tab and does not open a
      second `\\?\` tab. A regression test covers both branches.

- [ ] **F-6 — Removing a breakpoint does not remove it from the running session.** Clearing the
      breakpoint in the editor/Breakpoints panel updates the visual list, but the debug session
      still stops at the removed location. Investigate the DAP `setBreakpoints` reconciliation:
      the adapter must clear the previous host `breakpoint_id`s for the source even when the new
      list is empty, and must use one path key for the normal and verbatim spellings. Acceptance:
      remove a breakpoint while running or stopped; the session no longer stops there; re-adding
      it works without restarting.

- [x] **F-4 — Reusable fixed debug endpoint.** Fixed 2026-10-30 in the driver and retested:
      `stc run --debug-server --debug-endpoint` and `stc debug --endpoint` remove a validated
      stale `stc-debug/1` endpoint and rotate a valid stale audit log to
      `<base>.audit.<nanos>.jsonl` before launching. A non-debug endpoint or non-audit file is
      refused and preserved. Unit tests cover removal, audit-only rotation, and the foreign-file
      refusal; a second run and attach now work.

- [ ] **F-3 — Stop the `stc: run` task's process tree.** Terminating the VS Code task kills the
      shell but leaves `stc run` and the launcher (`sample.exe`) running. Make task termination
      stop the full tree (for example a custom execution that performs a process-tree kill, or a
      driver-side Windows job object/signal handler that kills the launcher). Acceptance: after
      terminating the task, `Get-Process stc,sample` shows no run driver or host; the language
      server may remain.

## Results log

| Item | Result | Notes |
| --- | --- | --- |
| 0.1 | Pass | Local VSIX installed 2026-10-30; upstream CLI deprecation warning ignored |
| 0.2 | Pass | Release path; binary 09:09, server PID 35220 started 09:16; end-of-word definition verified |
| 0.3 | Pass | Sample workspace opened; `src/main.st` active |
| 0.4 | Pass | Server serving; language mode Structured Text; Problems empty |
| 1 | Pass | Scopes correct; poor palette recorded as follow-up F-1 |
| 2 | Pass | Line/multi-line/block/nested comment toggling; layout shortcut is `Ctrl+ä` |
| 3 | Pass | Auto-close, comment/string suppression, bracket matching, surrounding pairs |
| 4 | Pass | Format normalizes spacing/indentation; idempotent; no extra undo step |
| 5 | Pass | Problems panel and editor clean for both sample sources |
| 6 | Pass | `STC-E-NAME-001` at line 41, columns 13–20; source `stc`; undo clears |
| 7 | Pass | Configured `stc: check` exits 0 after `${workspaceFolder}` expansion fix |
| 8 | Pass | `stc: test`: 2 passed, 0 failed, exit 0 |
| 9 | Pass | Build exits 0; cold-layout warning expected; Problems empty |
| 10 | Pass | `stc: fmt` rewrites the deviation; idempotent; Problems empty |
| 11 | Pass | Single Problems entry, correct line, click navigates, clears on undo |
| 12 | Fail (stop) | Run starts and now starts a debug server; terminating the task orphaned `stc run` + `sample.exe` (F-3) |
| 13 | Pass | F12 on `AddStep` at start/middle/end; verbatim tab was a stale unsupported route |
| 14 | Pass | FB type and instance resolve; declaration F12 uses VS Code's reference fallback |
| 15 | Pass | Variables, globals, and named labels resolve to declarations |
| 16 | Pass | 16.1/16.3/16.4 pass; 16.2 fixed (instances no longer counted as type references) and retest passes |
| 17–30 | Pending | Record pass/fail and any deviation above |
| 31 | Pass | Attach via `stc: run` debug server connects; disconnect leaves host running |
| 32–36 | Pending | Record pass/fail and any deviation above |
| F-2 | Pass | Run task is debuggable; attach connects |
| F-3 | Pending | Task termination does not stop the driver/host process tree |
| F-4 | Pass | Second debug-server run and attach work; audit rotated |
| F-5 | Pass | Breakpoint stop uses the normal tab; no second `\\?\` tab |
| F-6 | Pending | Removing a breakpoint does not remove it from the running session |
| F-1 | Pending | Highlighting scope/color follow-up recorded |
