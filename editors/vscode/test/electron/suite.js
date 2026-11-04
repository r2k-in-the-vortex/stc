'use strict';

// Headless activation smoke for the packaged VSIX.
//
// This suite runs inside the VS Code extension host that `runTest.js` starts
// with `--extensionTestsPath`. It proves the installed extension activates,
// starts the real `stc lsp` language client, runs a `stc` task through the
// contributed task provider and `$stc` matcher path, terminates a `stc: run`
// task and verifies its driver/host tree is gone, and launches a local
// `stc dap` debug session. It also drives the online-change surface end to
// end: start the extension-owned watch, observe the generation, attach the
// debugger, edit a layout-preserving literal, observe the swap teardown, and
// stop. Deep language-server and debug-adapter behavior is covered by the
// Rust `lsp_cli`, `dap_cli`, and `debug_cli` suites; this smoke is about the
// real editor wiring.

const assert = require('node:assert');
const { execFile } = require('node:child_process');
const { existsSync, rmSync } = require('node:fs');
const path = require('node:path');
const vscode = require('vscode');

const LANGUAGE_SERVER_TIMEOUT_MS = 60_000;
const DIAGNOSTICS_SETTLE_MS = 3_000;
const TASK_TIMEOUT_MS = 120_000;
const RUN_TASK_START_TIMEOUT_MS = 180_000;
const RUN_TASK_STOP_TIMEOUT_MS = 60_000;
const DEBUG_START_TIMEOUT_MS = 180_000;
const DEBUG_STOP_TIMEOUT_MS = 60_000;
const ONLINE_CHANGE_TIMEOUT_MS = 180_000;
const ONLINE_CHANGE_ATTACH_TIMEOUT_MS = 60_000;
const ONLINE_CHANGE_STOP_TIMEOUT_MS = 60_000;
const POLL_INTERVAL_MS = 200;

const extensionRoot = path.resolve(__dirname, '..', '..');
const manifest = require(path.join(extensionRoot, 'package.json'));
const extensionId = `${manifest.publisher}.${manifest.name}`;
const sampleWorkspace = path.resolve(
  process.env.STC_SAMPLE_WORKSPACE ?? path.join(extensionRoot, 'samples', 'workspace'),
);

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function withTimeout(promise, timeoutMs, what) {
  let timer;
  const timeout = new Promise((_resolve, reject) => {
    timer = setTimeout(
      () => reject(new Error(`timed out after ${timeoutMs}ms waiting for ${what}`)),
      timeoutMs,
    );
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

/**
 * Polls `probe` until it returns a defined value. Errors are retried because
 * the language client may not be registered yet; the last error is reported
 * when the deadline passes.
 */
async function poll(what, timeoutMs, probe) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  for (;;) {
    try {
      const result = await probe();
      if (result !== undefined) {
        return result;
      }
    } catch (error) {
      lastError = error;
    }
    if (Date.now() >= deadline) {
      const detail = lastError === undefined ? '' : `: ${lastError.message ?? lastError}`;
      throw new Error(`timed out after ${timeoutMs}ms waiting for ${what}${detail}`);
    }
    await delay(POLL_INTERVAL_MS);
  }
}

/** Runs a helper process and resolves its UTF-8 output. */
function execText(file, args) {
  return new Promise((resolve, reject) => {
    execFile(
      file,
      args,
      { encoding: 'utf8', windowsHide: true, maxBuffer: 8 * 1024 * 1024 },
      (error, stdout) => {
        if (error) {
          reject(error);
        } else {
          resolve(stdout);
        }
      },
    );
  });
}

/** Normalizes an image name so `sample.exe` and `sample` compare equal. */
function processName(name) {
  return path.basename(name).toLowerCase().replace(/\.exe$/, '');
}

/**
 * Lists running processes as `{ name, pid }`.
 *
 * The smoke only needs image names: `stc` covers both the run driver and the
 * language server, and the sample's host is named after the package.
 */
async function listProcesses() {
  if (process.platform === 'win32') {
    const output = await execText('tasklist', ['/FO', 'CSV', '/NH']);
    const processes = [];
    for (const line of output.split(/\r?\n/)) {
      const match = /^"([^"]+)","(\d+)"/.exec(line);
      if (match !== null) {
        processes.push({ name: processName(match[1]), pid: Number(match[2]) });
      }
    }
    return processes;
  }
  const output = await execText('ps', ['-axo', 'pid=,comm=']);
  const processes = [];
  for (const line of output.split(/\r?\n/)) {
    const match = /^\s*(\d+)\s+(.+?)\s*$/.exec(line);
    if (match !== null) {
      processes.push({ name: processName(match[2]), pid: Number(match[1]) });
    }
  }
  return processes;
}

function processIds(processes, name) {
  return processes
    .filter((candidate) => candidate.name === name)
    .map((candidate) => candidate.pid);
}

/** Kills anything the run-task test started, even after an assertion failed. */
async function killRunLeftovers(baselineStc) {
  let processes;
  try {
    processes = await listProcesses();
  } catch {
    return;
  }
  const leftovers = [
    ...processIds(processes, 'sample'),
    ...processIds(processes, 'stc').filter((pid) => !baselineStc.has(pid)),
  ];
  for (const pid of leftovers) {
    if (process.platform === 'win32') {
      await execText('taskkill', ['/pid', String(pid), '/t', '/f']).catch(() => undefined);
    } else {
      try {
        process.kill(pid, 'SIGKILL');
      } catch {
        // The process already exited.
      }
    }
  }
}

function samePath(left, right) {
  const normalizedLeft = path.resolve(left);
  const normalizedRight = path.resolve(right);
  return process.platform === 'win32'
    ? normalizedLeft.toLowerCase() === normalizedRight.toLowerCase()
    : normalizedLeft === normalizedRight;
}

function workspaceFolder() {
  const folders = vscode.workspace.workspaceFolders ?? [];
  const folder = folders.find((candidate) => samePath(candidate.uri.fsPath, sampleWorkspace));
  assert.ok(
    folder,
    `the sample workspace ${sampleWorkspace} is open (open folders: ${
      folders.map((candidate) => candidate.uri.fsPath).join(', ') || 'none'
    })`,
  );
  return folder;
}

/** The UTF-16 VS Code position of the first `needle` in `document`. */
function positionOf(document, needle) {
  const offset = document.getText().indexOf(needle);
  assert.notStrictEqual(offset, -1, `the sample source contains ${JSON.stringify(needle)}`);
  return document.positionAt(offset);
}

function errorDiagnostics(uri) {
  return vscode.languages
    .getDiagnostics(uri)
    .filter((diagnostic) => diagnostic.severity === vscode.DiagnosticSeverity.Error);
}

function describeDiagnostics(diagnostics) {
  return diagnostics
    .map(
      (diagnostic) =>
        `${diagnostic.source ?? 'stc'}${diagnostic.code ? `[${diagnostic.code}]` : ''}: ` +
        `${diagnostic.message} (line ${diagnostic.range.start.line + 1})`,
    )
    .join('; ');
}

async function assertLanguageServer(document) {
  const uri = document.uri;
  const callSite = positionOf(document, 'AddStep(sum)');
  const declaration = positionOf(document, 'AddStep : DINT');

  // Go-to-definition is served by `stc lsp`; the client may still be starting,
  // so poll until the provider answers with a real location.
  const locations = await poll(
    'stc lsp to answer go-to-definition for the sample call site',
    LANGUAGE_SERVER_TIMEOUT_MS,
    async () => {
      const result = await vscode.commands.executeCommand(
        'vscode.executeDefinitionProvider',
        uri,
        callSite,
      );
      return Array.isArray(result) && result.length > 0 ? result : undefined;
    },
  );
  const definition = locations[0];
  const targetUri = definition.targetUri ?? definition.uri;
  const targetRange = definition.targetSelectionRange ?? definition.targetRange ?? definition.range;
  assert.ok(targetUri, `the definition has a target URI: ${JSON.stringify(definition)}`);
  assert.ok(
    samePath(targetUri.fsPath, path.join(sampleWorkspace, 'src', 'main.st')),
    `the definition resolves inside the sample: ${targetUri.fsPath}`,
  );
  assert.strictEqual(
    targetRange.start.line,
    declaration.line,
    `the definition points at the AddStep declaration (line ${declaration.line + 1})`,
  );

  // The analysis is debounced; give the server its quiet period and the
  // project analysis before deciding the clean sample is diagnostic-free.
  const deadline = Date.now() + DIAGNOSTICS_SETTLE_MS;
  for (;;) {
    const errors = errorDiagnostics(uri);
    assert.strictEqual(
      errors.length,
      0,
      `the clean sample has no error diagnostics: ${describeDiagnostics(errors)}`,
    );
    if (Date.now() >= deadline) {
      break;
    }
    await delay(POLL_INTERVAL_MS);
  }
}

async function runCheckTask() {
  const tasks = await vscode.tasks.fetchTasks({ type: 'stc' });
  assert.ok(tasks.length > 0, 'the extension provides stc tasks');
  // Prefer the configured task from `.vscode/tasks.json`: it carries a
  // `${workspaceFolder}` project path and therefore exercises `resolveTask`
  // and variable expansion, which the detected task does not.
  const check =
    tasks.find(
      (task) =>
        task.definition.command === 'check' &&
        task.definition.project !== undefined,
    ) ?? tasks.find((task) => task.definition.command === 'check');
  assert.ok(
    check,
    `a stc check task is provided (available: ${tasks
      .map((task) => String(task.definition.command ?? task.name))
      .join(', ')})`,
  );

  let execution;
  let resolveEnd;
  const ended = new Promise((resolve) => {
    resolveEnd = resolve;
  });
  const listener = vscode.tasks.onDidEndTaskProcess((event) => {
    if (execution !== undefined && event.execution === execution) {
      resolveEnd(event.exitCode);
    }
  });
  try {
    execution = await vscode.tasks.executeTask(check);
    const exitCode = await withTimeout(
      ended,
      TASK_TIMEOUT_MS,
      'the stc check task to finish',
    );
    assert.strictEqual(exitCode, 0, 'the stc check task exits successfully');
  } finally {
    listener.dispose();
  }
}

/**
 * Runs the configured `stc: run` task, waits for the driver and host, then
 * terminates the task through the task API and proves the whole tree stops.
 *
 * The language server is also an `stc` process, so `stc` pids present before
 * the task starts are the allowed baseline; anything added by the run task
 * must disappear.
 */
async function runRunTaskTermination() {
  const tasks = await vscode.tasks.fetchTasks({ type: 'stc' });
  const run =
    tasks.find(
      (task) =>
        task.definition.command === 'run' &&
        task.definition.project !== undefined,
    ) ?? tasks.find((task) => task.definition.command === 'run');
  assert.ok(
    run,
    `a stc run task is provided (available: ${tasks
      .map((task) => String(task.definition.command ?? task.name))
      .join(', ')})`,
  );
  assert.ok(
    Array.isArray(run.definition.args) && run.definition.args.includes('--debug-server'),
    'the configured run task starts the debug server for attach',
  );

  const before = await listProcesses();
  const baselineStc = new Set(processIds(before, 'stc'));
  assert.strictEqual(
    processIds(before, 'sample').length,
    0,
    'no sample host is left over before the run task test',
  );

  let execution;
  let resolveEnded;
  const ended = new Promise((resolve) => {
    resolveEnded = resolve;
  });
  const endListener = vscode.tasks.onDidEndTask((event) => {
    if (execution !== undefined && event.execution === execution) {
      resolveEnded();
    }
  });
  try {
    execution = await vscode.tasks.executeTask(run);
    const active = await poll(
      'the stc run driver and host to start',
      RUN_TASK_START_TIMEOUT_MS,
      async () => {
        const processes = await listProcesses();
        return processIds(processes, 'sample').length > 0 ? processes : undefined;
      },
    );
    const drivers = processIds(active, 'stc').filter((pid) => !baselineStc.has(pid));
    assert.ok(
      drivers.length > 0,
      `the stc run driver is running (stc pids: ${
        processIds(active, 'stc').join(', ') || 'none'
      })`,
    );
    console.log(
      `stc smoke: stc run started (driver ${drivers.join(', ')}; ` +
        `host ${processIds(active, 'sample').join(', ')})`,
    );

    execution.terminate();
    await withTimeout(ended, RUN_TASK_STOP_TIMEOUT_MS, 'the terminated stc run task to end');

    await poll(
      'the terminated stc run process tree to stop',
      RUN_TASK_STOP_TIMEOUT_MS,
      async () => {
        const processes = await listProcesses();
        const hosts = processIds(processes, 'sample');
        const extraDrivers = processIds(processes, 'stc').filter(
          (pid) => !baselineStc.has(pid),
        );
        return hosts.length === 0 && extraDrivers.length === 0 ? true : undefined;
      },
    );
    console.log('stc smoke: stc run process tree stopped');
  } finally {
    endListener.dispose();
    await killRunLeftovers(baselineStc).catch(() => undefined);
  }
}

async function runDebugSession(folder) {
  const configuration = {
    type: 'stc',
    request: 'launch',
    name: 'STC sample',
    project: path.join(sampleWorkspace, 'stc.toml'),
  };

  let resolveStarted;
  const started = new Promise((resolve) => {
    resolveStarted = resolve;
  });
  const startListener = vscode.debug.onDidStartDebugSession((session) => {
    if (session.type === 'stc') {
      resolveStarted(session);
    }
  });
  let session;
  try {
    const [launched, startedSession] = await withTimeout(
      Promise.all([
        vscode.debug.startDebugging(folder, configuration),
        started,
      ]),
      DEBUG_START_TIMEOUT_MS,
      'the stc debug session to launch',
    );
    assert.strictEqual(launched, true, 'vscode.debug.startDebugging accepts the stc configuration');
    session = startedSession;
    assert.strictEqual(session.type, 'stc', 'the started debug session uses the stc adapter');
  } finally {
    startListener.dispose();
  }

  let resolveTerminated;
  const terminated = new Promise((resolve) => {
    resolveTerminated = resolve;
  });
  const terminateListener = vscode.debug.onDidTerminateDebugSession((ended) => {
    if (ended.id === session.id) {
      resolveTerminated();
    }
  });
  try {
    await vscode.debug.stopDebugging(session);
    await withTimeout(terminated, DEBUG_STOP_TIMEOUT_MS, 'the stc debug session to terminate');
    // The extension host clears `activeDebugSession` with a different RPC than
    // the one that fires `onDidTerminateDebugSession`, so it can still name
    // the stopped session after the termination event. Wait for it to clear
    // before the online change starts; otherwise the extension's active-session
    // warning becomes a modal dialog that the test host refuses to show.
    await poll(
      'the stopped stc debug session to clear from vscode.debug.activeDebugSession',
      DEBUG_STOP_TIMEOUT_MS,
      () => (vscode.debug.activeDebugSession?.type === 'stc' ? undefined : true),
    );
  } finally {
    terminateListener.dispose();
  }
}

async function onlineChangeStatus() {
  return vscode.commands.executeCommand('stc.showOnlineChangeStatus');
}

async function waitForOnlineChange(what, timeoutMs, predicate) {
  try {
    return await poll(what, timeoutMs, async () => {
      const snapshot = await onlineChangeStatus();
      return snapshot !== undefined && predicate(snapshot) ? snapshot : undefined;
    });
  } catch (error) {
    let last;
    try {
      last = await onlineChangeStatus();
    } catch {
      last = undefined;
    }
    const detail = last === undefined ? '' : ` (last status: ${JSON.stringify(last)})`;
    throw new Error(`${error.message}${detail}`);
  }
}

async function replaceDocumentText(document, needle, replacement) {
  const text = document.getText();
  const offset = text.indexOf(needle);
  assert.notStrictEqual(offset, -1, `the document contains ${JSON.stringify(needle)}`);
  const range = new vscode.Range(
    document.positionAt(offset),
    document.positionAt(offset + needle.length),
  );
  const edit = new vscode.WorkspaceEdit();
  edit.replace(document.uri, range, replacement);
  assert.ok(await vscode.workspace.applyEdit(edit), 'the online-change edit applies');
  assert.ok(await document.save(), 'the online-change edit saves');
}

async function restoreDocument(document, original) {
  if (document.getText() === original) {
    return;
  }
  const current = document.getText();
  const edit = new vscode.WorkspaceEdit();
  edit.replace(
    document.uri,
    new vscode.Range(document.positionAt(0), document.positionAt(current.length)),
    original,
  );
  assert.ok(await vscode.workspace.applyEdit(edit), 'the source restore applies');
  assert.ok(await document.save(), 'the source restore saves');
}

async function stopOnlineChangeIfActive() {
  try {
    const snapshot = await onlineChangeStatus();
    if (
      snapshot === undefined ||
      (snapshot.kind !== 'starting' && snapshot.kind !== 'running')
    ) {
      return;
    }
    await vscode.commands.executeCommand('stc.stopOnlineChange');
    await waitForOnlineChange(
      'the online change to stop during cleanup',
      ONLINE_CHANGE_STOP_TIMEOUT_MS,
      (current) => current.kind !== 'starting' && current.kind !== 'running',
    ).catch(() => undefined);
  } catch {
    // Cleanup must not mask the original failure.
  }
}

/**
 * Drives the real online-change surface: start, read the active generation,
 * attach, edit a layout-preserving literal, observe the swap-caused debug
 * session end and the new generation, then stop gracefully.
 */
async function runOnlineChange(document) {
  const original = document.getText();
  let startListener;
  let terminateListener;
  let startedSession;
  try {
    console.log('stc smoke: online change start');
    await vscode.commands.executeCommand('stc.startOnlineChange');
    // The command resolves after the extension's start gates have run. If a
    // gate aborted (for example the active-debug-session warning was refused
    // instead of answered), the status is still idle; fail now with that
    // status rather than waiting the full activation timeout for a generation
    // that will never arrive.
    const accepted = await onlineChangeStatus();
    assert.ok(
      accepted !== undefined &&
        (accepted.kind === 'starting' || accepted.kind === 'running'),
      `the online change start was accepted (status: ${JSON.stringify(accepted)}; ` +
        'see the "stc Online Change" output channel for the start failure)',
    );
    const active = await waitForOnlineChange(
      'the online change to report an active generation',
      ONLINE_CHANGE_TIMEOUT_MS,
      (snapshot) =>
        snapshot.kind === 'running' &&
        snapshot.phase === 'active' &&
        typeof snapshot.generation === 'number' &&
        typeof snapshot.buildId === 'string' &&
        snapshot.buildId.length > 0,
    );
    console.log(
      `stc smoke: online change active at generation ${active.generation} (${active.buildId})`,
    );

    console.log('stc smoke: online change attach');
    let resolveStarted;
    const sessionStarted = new Promise((resolve) => {
      resolveStarted = resolve;
    });
    startListener = vscode.debug.onDidStartDebugSession((session) => {
      if (
        session.type === 'stc' &&
        session.configuration?.name === 'stc: Online Change'
      ) {
        resolveStarted(session);
      }
    });
    await vscode.commands.executeCommand('stc.attachOnlineDebugger');
    startedSession = await withTimeout(
      sessionStarted,
      ONLINE_CHANGE_ATTACH_TIMEOUT_MS,
      'the online-change debug session to start',
    );
    assert.strictEqual(
      startedSession.type,
      'stc',
      'the online-change debugger attaches through stc dap',
    );

    let resolveTerminated;
    const sessionTerminated = new Promise((resolve) => {
      resolveTerminated = resolve;
    });
    terminateListener = vscode.debug.onDidTerminateDebugSession((ended) => {
      if (ended.id === startedSession.id) {
        resolveTerminated();
      }
    });

    console.log('stc smoke: online change edit and swap');
    await replaceDocumentText(document, 'cycles := cycles + 1;', 'cycles := cycles + 2;');
    await withTimeout(
      sessionTerminated,
      ONLINE_CHANGE_TIMEOUT_MS,
      'the attached debug session to end on the swap',
    );
    const updated = await waitForOnlineChange(
      'the online change to report the new generation',
      ONLINE_CHANGE_TIMEOUT_MS,
      (snapshot) =>
        snapshot.kind === 'running' &&
        typeof snapshot.generation === 'number' &&
        snapshot.generation > active.generation &&
        typeof snapshot.buildId === 'string' &&
        snapshot.buildId !== active.buildId,
    );
    console.log(
      `stc smoke: online change swapped to generation ${updated.generation} (${updated.buildId})`,
    );

    console.log('stc smoke: online change stop');
    await vscode.commands.executeCommand('stc.stopOnlineChange');
    const stopped = await waitForOnlineChange(
      'the online change to stop',
      ONLINE_CHANGE_STOP_TIMEOUT_MS,
      (snapshot) =>
        snapshot.kind === 'idle' ||
        (snapshot.kind === 'stopped' && snapshot.reason === 'user'),
    );
    assert.ok(stopped, 'the online change reports a user stop');
  } finally {
    startListener?.dispose();
    terminateListener?.dispose();
    if (startedSession !== undefined) {
      await vscode.debug.stopDebugging(startedSession).catch(() => undefined);
    }
    await stopOnlineChangeIfActive();
    await restoreDocument(document, original);
  }
}

async function run() {
  assert.ok(existsSync(path.join(sampleWorkspace, 'stc.toml')), `sample workspace at ${sampleWorkspace}`);

  const extension = vscode.extensions.getExtension(extensionId);
  assert.ok(extension, `the installed extension ${extensionId} is present`);
  // The smoke exists to prove the packaged VSIX activates. The runner installs
  // it into the throwaway `--extensions-dir`; a source-tree development path
  // would be a different kind of test.
  assert.strictEqual(
    path.basename(extension.extensionPath),
    `${extensionId}-${manifest.version}`,
    `the extension under test is the installed VSIX (${extension.extensionPath})`,
  );
  await extension.activate();
  assert.strictEqual(extension.isActive, true, `the extension ${extensionId} is active`);

  // `contributes.breakpoints` is what enables breakpoints in the editor
  // gutter for a language; without it, clicking the margin does nothing.
  const breakpointLanguages = (extension.packageJSON?.contributes?.breakpoints ?? [])
    .map((entry) => entry?.language);
  assert.ok(
    breakpointLanguages.includes('st'),
    `the extension enables breakpoints for st (found: ${JSON.stringify(breakpointLanguages)})`,
  );

  const folder = workspaceFolder();
  const document = await vscode.workspace.openTextDocument(
    vscode.Uri.file(path.join(sampleWorkspace, 'src', 'main.st')),
  );
  await vscode.window.showTextDocument(document);
  assert.strictEqual(document.languageId, 'st', 'the sample source is the st language');

  try {
    console.log('stc smoke: language server');
    await assertLanguageServer(document);
    console.log('stc smoke: stc check task');
    await runCheckTask();
    console.log('stc smoke: stc run task termination');
    await runRunTaskTermination();
    console.log('stc smoke: stc dap launch');
    await runDebugSession(folder);
    console.log('stc smoke: stc online change');
    await runOnlineChange(document);
    console.log('stc smoke: passed');
  } finally {
    // The check and launch built the sample in place; remove the generated
    // artifacts so the checked-in fixture stays clean. Everything a build
    // writes lives under `target/`.
    rmSync(path.join(sampleWorkspace, 'target'), { recursive: true, force: true });
  }
}

module.exports = { run };
