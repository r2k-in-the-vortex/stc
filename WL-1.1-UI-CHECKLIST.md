# WL-1.1 Manual VS Code UI Checklist

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-30  
Updated: 2026-10-31  
Sample: `editors/vscode/samples/workspace`  
Extension: latest local `editors/vscode/stc-vscode-0.1.0.vsix`  
Compiler: the binary `stc.path` points at; use the latest debug or a rebuilt release binary

Result key: unchecked = pending, `[x]` = pass, **FAIL** = failed with a note, **N/A** =
unsupported or intentionally out of scope.

Summary 2026-10-31: 40/40 entries evaluated — 38 pass (95%), items 12 and 33 fail. The
required/documented/deferred close-out triage is in [WL-1.1-Plan.md](WL-1.1-Plan.md) §12.

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
17. [x] **Hover.** Pass 2026-10-30: `Ctrl+K Ctrl+I` on `sum` shows `sum : DINT` / `field of
        program Main`; on `AddStep` shows `AddStep : DINT` / `function`; on `Accumulator` shows
        `Accumulator` / `function block`.
18. [x] **Completion.** Pass 2026-10-30: `Ctrl+Space` in the `PROGRAM Main` body offers
        keywords/types (`DINT`, `IF`, `FOR`), visible names (`INCREMENT`, `TOTAL`, `AddStep`,
        `Accumulator`), the program fields `accumulator` (detail `Accumulator`), `sum` and
        `cycles` (detail `DINT`), and no duplicate spelling; after `accumulator.` only the FB
        members `increment` and `total` are offered, without general keywords or names.
        While typing the equivalent direct-access statements, `accumulator.increment` and
        `accumulator.total` were rejected with `STC-E-NAME-004` even though the call form
        worked. Root cause: FB interface fields defaulted to `PROTECTED` (D-068) while call
        bindings skip field access checks. Fixed by D-211 (FB `VAR_INPUT`/`VAR_OUTPUT`/
        `VAR_IN_OUT` default to `PUBLIC`; internal `VAR` stays `PROTECTED`; the diagnostic now
        names the member). The sample now uses the equivalent member form and checks cleanly.
        Recorded as follow-up **F-7**.
19. [x] **Document symbols.** Pass 2026-10-30: `Ctrl+Shift+O` lists 16 symbols:
        `INCREMENT`, `TOTAL`, `AddStep` (`value`, `incremented`), `Accumulator` (`increment`,
        `total`), `Main` (`accumulator`, `sum`, `cycles`), and `Sample` (`Cpu` → `MainTask`,
        `sample`). Selecting an entry jumps to its declaration; the hierarchy matches the source.
20. [x] **Cross-file navigation.** Pass 2026-10-30: from `tests/smoke.st`, F12 on `AddStep`
        (line 8) jumps to `src/main.st:12`; F12 on `Accumulator` (line 16) jumps to
        `src/main.st:23`; F12 on the smoke test's call receiver goes to its own field, not
        `Main`'s. `Shift+F12` on `AddStep` returns the declaration, the result assignment, and
        both calls (three without the declaration); `Shift+F12` on `Accumulator` returns the
        declaration and both type uses, with no instance names/calls.

## D. Debugging

21. [x] **Breakpoint gutter.** Pass 2026-10-30: clicking the gutter on line 39 shows a red dot,
        the Breakpoints panel lists `src/main.st` line 39, and toggling removes/restores both.
22. [x] **Launch.** Pass 2026-10-30: F5 reuses the last-selected configuration; with **stc: Launch
        sample** selected, the project builds, the session starts, and it stops at line 39. The
        Debug Console shows the non-safety notice. Note: the endpoint error means the **stc:
        Attach sample (replace endpoint)** configuration ran; attach needs the `stc: run` task,
        launch does not.
23. [x] **Globals.** Pass 2026-10-30: **Globals** shows `INCREMENT = 1` and `TOTAL`; the
        program instance (`SAMPLE/CPU/SAMPLE`) expands to `ACCUMULATOR`, `CYCLES`, and `SUM`;
        `ACCUMULATOR` expands to `INCREMENT` and `TOTAL`. Names are the debug metadata's
        normalized uppercase spellings; `Main` has no frame locals, so an empty Locals scope is
        expected at this stop.
        First run 2026-10-30 exposed two `stc-dap` defects, both fixed and retested: hidden FB
        object roots were dropped, so the instance had no `ACCUMULATOR`; and the runtime's
        flattened `view` field shape was not expanded, so aggregates expanded to nothing.
        Covered by `adapter.rs` unit tests, `value.rs` view tests, and `dap_cli.rs` (**F-8**).
        `TOTAL` may be nonzero at the first stop because cycles run before breakpoints bind;
        launching with a breakpoint is expected to stop on the first cycle, so this remains a
        defect (**F-9**). The `[debug].session-timeout` bound ends the session with
        `STC-E-DEBUG-018` when configured; after D-212 it is opt-in and disabled by default, so
        the earlier 5-minute default no longer applies (**F-10** resolved; the optional warning is
        part of F-15).
24. [x] **Call stack.** Pass 2026-10-30: at line 39 the stack shows `MAIN` with
        `src/main.st:39:3`; stepping into `AddStep` shows `ADDSTEP` with `src/main.st:19:3` above
        `MAIN`. Frame names are mapped from the manifest function `GotId` (`stc-dap` fix; the
        previous UI showed mangled symbols). Deviations recorded: caller frames have no source
        location (**F-11**), and two unresolved host frames appear as `frame`/Unknown Source
        (**F-12**). `Locals` is populated in the function frame but empty for program/FB bodies,
        whose fields are instance roots in `Globals` (**F-13**).
25. [x] **Step into.** Pass 2026-10-30: F11 from line 39 stops in `AddStep` at line 19;
        **Locals** shows `VALUE` (decimal) and `INCREMENTED = 0`; the stack shows `ADDSTEP` above
        `MAIN`.
26. [x] **Step over.** Pass 2026-10-30: F10 on line 19 moves to line 20 and
        `INCREMENTED = 1`.
27. [x] **Step out.** Pass 2026-10-30: `Shift+F11` from inside `AddStep` returns to `Main`.
28. [x] **Watch and evaluate.** Pass 2026-10-30: Watch shows `INCREMENT = 1` and `sum` as a
        decimal, and both refresh at the next stop. The Debug Console evaluates `INCREMENT = 1`
        and `sum`; `1 + 2`/`sum + 1` are rejected with the explicit
        "ST expressions are not supported" message. `stc-dap` now also resolves a visible global
        or program-instance field by name for `evaluate`; covered by `dap_cli.rs`.
29. [x] **setVariable gating.** Pass 2026-10-30: without `allowWrite`, editing `INCREMENT` is
        refused with a message mentioning `allowWrite`; with `"allowWrite": true`, editing
        `INCREMENT` (and program-instance fields) succeeds. The write is queued: the Variables
        pane shows the requested value immediately, the stored value reads as `1` again while
        stepping through the current cycle, and the new value is visible after the cycle
        boundary (the next line-39 stop). Recorded as **F-14**; the launch configuration was
        returned to `"allowWrite": false`.
30. [x] **Terminate and relaunch.** Pass 2026-10-30: `Shift+F5` ends the session and the
        launch-owned `sample.exe` exits; the remaining `stc` process is the language server.
        `F5` launches a fresh session and stops at line 39 again; the non-safety notice repeats
        and no stale endpoint/audit error appears. The Debug Console prints no terminate/exit
        report (**F-15**).
31. [x] **Attach.** pass 2026-10-30 using the `stc: run` task, which now passes
        `--debug-server --debug-endpoint target/stc/debug/debug.endpoint.json` (F-2). The
        **stc: Attach sample** session connected, a breakpoint stopped, and disconnect left the
        host running. Stopping the run task still leaves its process tree (F-3), so the driver and
        `sample.exe` were killed manually; the `stc lsp` process remained.
32. [x] **Conditional breakpoints.** Pass 2026-10-30: **Add Conditional Breakpoint** accepts a
        condition with no error at creation; at launch VS Code warns that this breakpoint type is
        not supported by the debugger and the breakpoint stops unconditionally. The condition is
        not applied, but the unsupported action is reported rather than silently honored.
33. [ ] **Release monitoring-only.** FAIL 2026-10-30: with `"release": true`, the session builds
        release and starts (no breakpoints, stepping, or Locals), and reports
        "stopped-frame information is unavailable, so there is no Locals scope". But Variables
        shows no Globals, Watch reports values unavailable, `pause` fails with
        `STC-E-DEBUG-014`, and F10/F11 report the target is not stopped. Typed reads do work while
        running (verified with `stc debug --release`: `INCREMENT = 1` via `var.read`), but the
        IDE exposes no monitoring surface. Recorded as follow-up **F-16**.

## E. Safety and limits

34. [x] **Non-safety notice.** Pass 2026-10-30: every session-start Debug Console output includes
        the non-safety notice verbatim, and the extension surfaces checked (launch names, status
        bar, README/tooltips) do not present debugging, forcing, or monitoring as a safety
        function.
35. [x] **Local-only.** Pass 2026-10-31: the launch/attach schema exposes only local fields
        (`project`, `outDir`, `noBuild`, `release`, `allowWrite`, `allowForce`, `sourceRoot`,
        `args`, `cwd`, and the attach `endpoint` file); no host, port, URL, or remote field.
        `stc lsp` runs as a stdio child (`stc.exe lsp --manifest-path ...`) with no TCP
        listeners, and the debug endpoint/audit paths are local files. The only
        “`debugServer`” mention is the CLI flag in the attach `endpoint` description; the
        extension schema (installed VSIX and source) has no such property. The only setting is
        `stc.path`.
36. [x] **Known unsupported UI actions.** Pass 2026-10-31: (1) semantic tokens are absent —
        **Developer: Inspect Editor Tokens and Scopes** shows only the TextMate `source.st`
        scope. (2) F2 rename does nothing and prints no message; the compiler/runtime already
        implement rename history and online change, so IDE exposure is recorded as **F-19**/
        **F-20**. (3) `Ctrl+.` reports “No code actions available”. (4) `Ctrl+K Ctrl+F` is not a
        command in this VS Code and the server advertises no range-formatting provider, so range
        formatting is absent. (5) `Shift+Alt+H` opens Help; no call-hierarchy provider is
        advertised, so call hierarchy is absent. (6) `Ctrl+T` reports “No matching workspace
        symbols”. (7) **Debug: Restart** works by client-side relaunch; the adapter does not
        implement the DAP `restart` request (`supportsRestartRequest: false`). (8) One thread
        (`cycle`) is shown; there is no multi-thread UI. (9) `"stopOnEntry": true` is rejected by
        the launch schema (“Property stopOnEntry is not allowed”) before a session starts; the
        adapter also rejects it if sent directly. None of the unsupported actions silently
        applies a false result.

## Follow-ups discovered during verification

The close-out triage (required for WL-1.1, documented as unsupported, or deferred to a separate
plan) is in [WL-1.1-Plan.md](WL-1.1-Plan.md) §12.

- [ ] **F-1 — Highlighting scope and color scheme.** Required for WL-1.1 (plan §12.1);
      sub-plan: [WL-1.1-F1-Plan.md](WL-1.1-F1-Plan.md). The grammar's scopes are correct for the
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

- [x] **F-7 — Direct FB interface member access contradicted the call form.** Fixed 2026-10-30
      by D-211. Before the fix, `fb.in := x` and `x := fb.out` were rejected with
      `STC-E-NAME-004` while the equivalent `fb(in := x, out => y)` call worked: FB fields
      defaulted to `PROTECTED` (D-068), but call bindings skip field access checks. FB
      `VAR_INPUT`/`VAR_OUTPUT`/`VAR_IN_OUT` now default to `PUBLIC`; internal `VAR` and class
      variables remain `PROTECTED`, explicit specifiers still override for direct access and for
      call bindings (a `PRIVATE` input cannot be supplied and a `PROTECTED` output cannot be read
      through `fb(...)` from an unauthorized context either), and `STC-E-NAME-004` names the
      inaccessible member. Covered by `conformance.rs` and `navigation.rs`; the sample now uses
      the member form and `stc check` exits 0.

- [x] **F-8 — Globals omitted FB-typed program fields and views did not expand.** Fixed in
      `stc-dap` 2026-10-30 and retested in the UI: the instance now expands to `ACCUMULATOR`,
      `CYCLES`, and `SUM`, and `ACCUMULATOR` expands to `INCREMENT`/`TOTAL`. Two defects:
      (1) a hidden `OBJREC` root was filtered out while its nested leaf roots were skipped, so a
      program instance never showed an FB-typed field such as `ACCUMULATOR`; the layout now
      exposes the aggregate path (`SAMPLE::MAIN.ACCUMULATOR#<instance>`) and the runtime returns
      its manifest-assisted object view. (2) The runtime flattens a field's node into the field
      object, but `view_children` expected a nested `view`, so every manifest-assisted aggregate
      expanded to an empty list; it now accepts the flattened shape and prefers the field `name`
      over the path. Covered by the `adapter.rs` layout tests, `value.rs` view tests, and the
      `sample_workspace_launch_reads_known_values_and_steps` DAP test.

- [ ] **F-11 — Caller stack frames have no source location.** The top frame carries the
      checkpoint's `file`/`line`/`column`, but `capture_frames` sets `source: None` for every
      caller. The frame sidecar has function ranges and locals but no address-to-source mapping,
      so a caller such as `MAIN` shows "Unknown Source" while stopped in `ADDSTEP` until the
      session returns. Fixing this needs a line/address table in the debug metadata (or a
      sidecar call-site map) and runtime resolution of the return address. Acceptance: the
      caller frame shows the call-site file and line (for the sample, `src/main.st:39`) at any
      depth.

- [ ] **F-12 — Unknown host frames appear as `frame` entries.** The frame-pointer walk continues
      past the ST program into the launcher/host, producing frames with no symbol/source that
      the adapter labels `frame` (two in the sample). The runtime reports them honestly as
      `optimized_out:true`, so this is a presentation choice: stop the walk at the first address
      outside the manifest function ranges, or filter/label non-ST frames in `stc-dap`.
      Acceptance: the Call Stack lists only resolved ST frames for a sample stop.

- [ ] **F-13 — Locals is empty for program and FB body frames.** `Locals` lists only
      `frame.locals`; program/FB fields are instance roots and appear under `Globals`, so a stop
      in `Main` or an FB body shows an empty Locals scope even though that frame's state is
      visible one scope up. Consider mirroring the frame's instance fields in `Locals` (which
      needs a frame-to-instance mapping when several instances exist) or documenting the split
      in the IDE spec/sample README. Acceptance: a stop in a program/FB body shows its fields
      in a discoverable place without switching scopes blindly.

- [ ] **F-9 — A launch with a breakpoint must stop on the first cycle.** Currently the host starts
      its cycle loop before `session.start` and `breakpoint.set` complete, so 1–2 cycles run
      first and the first stop observes `TOTAL = 2`/`sum = 2`/`cycles = 2` instead of the cycle-0
      values. The sample README documents the value caveat, and `stopOnEntry` is rejected with no
      start-paused mode. Fixing this needs a protocol/runtime change (start the host paused until
      the session has bound breakpoints and resumed). Acceptance: the first stop at line 39
      observes `TOTAL = 0`, `sum = 0`, `cycles = 0` (or the values written by the first cycle only
      after the breakpoint is bound).

- [x] **F-10 — The pause timeout was a silent default.** Respecified by D-212 on 2026-10-31:
      `[debug].session-timeout` is opt-in and disabled by default; when configured, expiry still
      emits `STC-E-DEBUG-018`, clears session state, and resumes the cycle. An automatic resume
      can move machinery unexpectedly, so the bound must be an explicit operator choice. The
      optional session-start budget/warning message is part of the F-15 session-status polish
      plan. Manifest, runtime, CLI, host ABI, and specifications updated; tests cover the default
      and explicit zero.

- [ ] **F-14 — `setVariable` shows the queued value before the program applies it.** With writes
      enabled, editing `INCREMENT` displays the new value immediately, but the host applies the
      write at the next cycle boundary (Debug and Monitoring §15.3). Stepping through the current
      cycle reads the stored value again, and the new value appears after the boundary. This is
      the documented queue behavior, but the UI has no pending indicator, so the temporary revert
      looks like a lost write. Consider reading back the stored value until the queue applies
      (with a pending marker) or flushing queued writes when a stopped cycle resumes. Acceptance:
      the displayed value and the program's value agree, or the UI clearly marks the value as
      queued.

- [ ] **F-15 — Terminating a launch session gives no Debug Console report.** `Shift+F5` ends the
      session and the launch-owned host exits, but the Debug Console prints no terminated/exited
      text, so the stop is silent unless the Debug toolbar disappears. Consider emitting an
      output event or relying on VS Code's session-ended indication. Acceptance: stopping a
      session leaves a clear line in the Debug Console.

- [ ] **F-16 — Release monitoring has no IDE surface.** Required for WL-1.1; implementation chosen (plan §12.1). With `"release": true`, the session
      starts and warns that stopped-frame information is unavailable, but `stackTrace` fails
      (`STC-E-DEBUG-017`), so `scopes`/`variables` have no frame, Variables shows no Globals, and
      Watch expressions stay unavailable; `pause` fails with `STC-E-DEBUG-014`. Typed monitoring
      reads do work while running (`stc debug --release` read `INCREMENT = 1` via `var.read`, and
      the Debug Console can evaluate target names), but the IDE exposes no monitoring view.
      Provide a surface — a synthetic running/monitoring frame with the Globals scope, a polling
      monitor view, or documented Debug-Console-only reads — and make the session-start message
      name the available monitoring path. Acceptance: a release session can observe globals from
      the IDE without breakpoints.

- [ ] **F-17 — No way to debug a test.** The launch configuration builds and runs the project,
      not a `{stc:test}` POU; there is no launch configuration, command, or code lens to start a
      per-test artifact under the debugger. Consider a `stc: Debug test` configuration that
      builds the selected test and starts the test host with a debug server, or a Testing-API
      integration with a Debug Test action. Acceptance: a test in `tests/smoke.st` can be
      debugged with breakpoints and locals.

- [ ] **F-18 — No per-test run control in VS Code.** `stc: test` runs the whole test source set
      through a task; the editor offers no Testing view, gutter/code-lens, or per-test command to
      run `AccumulatorWorks` or `AddStepWorks` individually. Consider a VS Code Testing API
      provider backed by `stc test` JSON results, with Run and Debug actions. Acceptance: an
      individual `{stc:test}` POU can be run (and later debugged) from the editor.

- [ ] **F-19 — Rename is not exposed in the IDE.** F2 does nothing and prints no message, while
      the compiler/runtime already carry rename history and online-change rename tracking. Add
      `textDocument/prepareRename`/`textDocument/rename` to `stc-lsp` backed by the reference
      resolution (and rename history where relevant), so VS Code offers a symbol rename.
      Acceptance: F2 on a field/function renames all references, and unsupported cases fail with
      a clear message.

- [ ] **F-20 — Online change is not exposed in the IDE.** Required for WL-1.1 (plan §12.1). The compiler/runtime implement online
      change and the CLI drives it, but the VS Code extension offers no command or status to
      activate a swap, view the generation/build state, or surface the session ending on swap.
      Consider a command/status item that invokes the online workflow and reports the generation.
      Acceptance: an online change can be started and inspected from the IDE.

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
| 17 | Pass | Hover shows type, kind, and owner for `sum`, `AddStep`, and `Accumulator` |
| 18 | Pass | Completion offers keywords/names and only `increment`/`total` after `accumulator.`; direct member access fixed by D-211 (F-7) |
| 19 | Pass | 16 symbols; hierarchy and jumps match the source |
| 20 | Pass | Cross-file F12/references both directions; smoke receiver resolves to its own field |
| 21 | Pass | Breakpoint gutter and Breakpoints panel stay in sync; toggle works |
| 22 | Pass | Launch builds/starts/stops at line 39; non-safety notice shown (F5 had reused Attach, which needs the run task) |
| 23 | Pass | Instance expands to `ACCUMULATOR`/`CYCLES`/`SUM`; `ACCUMULATOR` expands to `INCREMENT`/`TOTAL` |
| 24 | Pass | `MAIN`/`ADDSTEP` frames with source; caller source, unknown host frames, and Locals split recorded as F-11/F-12/F-13 |
| 25 | Pass | Step into stops at line 19; `VALUE` populated; `INCREMENTED = 0` |
| 26 | Pass | Step over moves to line 20; `INCREMENTED = 1` |
| 27 | Pass | Step out returns to `Main` |
| 28 | Pass | Watch/evaluate resolve names; values refresh; arbitrary ST expressions rejected |
| 29 | Pass | Gating works; write queue applies at the next cycle boundary (F-14) |
| 30 | Pass | Session stops, host exits, relaunch stops at line 39; terminate is silent (F-15) |
| 31 | Pass | Attach via `stc: run` debug server connects; disconnect leaves host running |
| 32 | Pass | Conditional breakpoint accepted; launch warns unsupported and stops unconditionally |
| 33 | Fail (monitoring) | Release starts/warns, but Variables/Watch cannot monitor; typed reads work only over the wire (F-16) |
| 34 | Pass | Non-safety notice shown verbatim at every session start; no safety claims in the UI |
| 35 | Pass | Local schema only; `stc lsp` is stdio with no TCP listeners; endpoint files are local |
| 36 | Pass | Unsupported actions absent or honestly rejected; restart is a client relaunch |
| F-2 | Pass | Run task is debuggable; attach connects |
| F-3 | Pending | Task termination does not stop the driver/host process tree |
| F-4 | Pass | Second debug-server run and attach work; audit rotated |
| F-5 | Pass | Breakpoint stop uses the normal tab; no second `\\?\` tab |
| F-6 | Pending | Removing a breakpoint does not remove it from the running session |
| F-7 | Pass | FB interface member access now matches the call form (D-211); sample checks cleanly |
| F-8 | Pass | Globals exposes FB fields and expands manifest-assisted views; UI retest passes |
| F-9 | Pending | First stop is 1–2 cycles late; launch-with-breakpoint must stop on cycle 0 |
| F-10 | Pass (respecified) | Timeout is opt-in and disabled by default (D-212); explicit bounds still end the session |
| F-11 | Pending | Caller frames lose file/line while stopped in a callee |
| F-12 | Pending | Unknown host frames appear as `frame`/Unknown Source |
| F-13 | Pending | Locals is empty for program/FB bodies; fields live in Globals |
| F-14 | Pending | `setVariable` shows the queued value before the program applies it at the next boundary |
| F-15 | Pending | Terminating a launch session is silent in the Debug Console |
| F-16 | Pending | Release monitoring has no IDE surface; typed reads work only over the wire |
| F-17 | Pending | No way to debug a `{stc:test}` POU |
| F-18 | Pending | No per-test run control (Testing view/code lens) in VS Code |
| F-19 | Pending | `textDocument/rename` not exposed; F2 is silent in the IDE |
| F-20 | Pending | Online change not exposed in the IDE |
| F-1 | Pending | Highlighting scope/color follow-up recorded |
