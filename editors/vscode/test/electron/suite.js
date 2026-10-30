'use strict';

// Headless activation smoke for the packaged VSIX.
//
// This suite runs inside the VS Code extension host that `runTest.js` starts
// with `--extensionTestsPath`. It proves the installed extension activates,
// starts the real `stc lsp` language client, runs a `stc` task through the
// contributed task provider and `$stc` matcher path, and launches a local
// `stc dap` debug session. Deep language-server and debug-adapter behavior is
// covered by the Rust `lsp_cli` and `dap_cli` suites; this smoke is about the
// real editor wiring.

const assert = require('node:assert');
const { existsSync, rmSync } = require('node:fs');
const path = require('node:path');
const vscode = require('vscode');

const LANGUAGE_SERVER_TIMEOUT_MS = 60_000;
const DIAGNOSTICS_SETTLE_MS = 3_000;
const TASK_TIMEOUT_MS = 120_000;
const DEBUG_START_TIMEOUT_MS = 180_000;
const DEBUG_STOP_TIMEOUT_MS = 60_000;
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
  const check = tasks.find((task) => task.definition.command === 'check');
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
  } finally {
    terminateListener.dispose();
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
    console.log('stc smoke: stc dap launch');
    await runDebugSession(folder);
    console.log('stc smoke: passed');
  } finally {
    // The check and launch built the sample in place; remove the generated
    // artifacts so the checked-in fixture stays clean. Everything a build
    // writes lives under `target/`.
    rmSync(path.join(sampleWorkspace, 'target'), { recursive: true, force: true });
  }
}

module.exports = { run };
