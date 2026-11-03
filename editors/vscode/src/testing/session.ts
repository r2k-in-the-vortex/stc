// VS Code Testing API provider for `{stc:test}` POUs.
//
// The controller is intentionally thin: discovery and result parsing live in
// `model.ts`, and this module only maps that data onto TestItems/TestRuns and
// debug sessions. One `stc test` process serves a run of all selected tests
// in one project; projects are run sequentially.

import * as path from 'node:path';
import { StringDecoder } from 'node:string_decoder';
import * as vscode from 'vscode';

import { resolveExecutable } from '../executable';
import { findManifest } from '../languageClient';
import { killProcessTree } from '../processTree';
import { spawnStc, type StcProcess } from '../stcProcess';
import { stcExecutable } from '../tasks';
import {
  formatDiagnostic,
  formatResultSummary,
  groupTestsByFile,
  parseDiagnostics,
  parseTestList,
  resolveResultPath,
  ResultStream,
  testDebugConfiguration,
  testListArguments,
  testRunArguments,
  type DiscoveredTest,
  type TestDiagnostic,
  type TestResult,
} from './model';

/** One discovered test and the VS Code item that represents it. */
interface TestReference {
  readonly manifest: string;
  readonly root: string;
  readonly folder: vscode.WorkspaceFolder | undefined;
  readonly test: DiscoveredTest;
  readonly item: vscode.TestItem;
}

/** A project manifest resolved through the workspace-folder ancestor search. */
interface Project {
  readonly manifest: string;
  readonly root: string;
  readonly folder: vscode.WorkspaceFolder;
  readonly tests: readonly DiscoveredTest[];
}

/** The tests selected in one project. */
interface RunGroup {
  readonly manifest: string;
  readonly root: string;
  readonly tests: TestReference[];
}

interface FileReference {
  /** Test item ids in this file-level item, in discovery order. */
  readonly children: readonly string[];
}

interface DiscoveryResult {
  readonly success: boolean;
  readonly tests: readonly DiscoveredTest[];
  readonly summary: string | undefined;
}

/**
 * Owns the `stc` TestController.
 *
 * The controller is only created once a workspace folder (or one of its
 * ancestors) contains `stc.toml`, so a workspace without a project does not
 * show an empty test view. It is kept afterwards, even if the manifest
 * disappears, so the Test Explorer view stays available.
 */
export class StcTestingSession implements vscode.Disposable {
  private readonly watchers: vscode.FileSystemWatcher[] = [];
  private readonly disposables: vscode.Disposable[] = [];
  private readonly tests = new Map<string, TestReference>();
  private readonly files = new Map<string, FileReference>();
  private readonly discovered = new Map<string, readonly DiscoveredTest[]>();
  private readonly runningProcesses = new Set<StcProcess>();
  private controller: vscode.TestController | undefined;
  private activeDebugSession: vscode.DebugSession | undefined;
  private refreshQueue: Promise<void> = Promise.resolve();
  private debounce: ReturnType<typeof setTimeout> | undefined;
  private disposed = false;

  public constructor(private readonly output: vscode.OutputChannel) {
    for (const glob of ['**/*.st', '**/stc.toml']) {
      const watcher = vscode.workspace.createFileSystemWatcher(glob);
      watcher.onDidCreate(() => this.scheduleRefresh());
      watcher.onDidChange(() => this.scheduleRefresh());
      watcher.onDidDelete(() => this.scheduleRefresh());
      this.watchers.push(watcher);
    }
    this.disposables.push(
      vscode.workspace.onDidChangeWorkspaceFolders(() => this.scheduleRefresh()),
    );
    void this.refresh();
  }

  /** Re-discovers every project, serialized with any refresh in flight. */
  public refresh(token?: vscode.CancellationToken): Promise<void> {
    const next = this.refreshQueue.then(
      () => this.refreshNow(token),
      () => this.refreshNow(token),
    );
    this.refreshQueue = next.catch((error: unknown) => {
      this.log(`refresh failed: ${message(error)}`);
    });
    return this.refreshQueue;
  }

  public dispose(): void {
    if (this.disposed) {
      return;
    }
    this.disposed = true;
    if (this.debounce !== undefined) {
      clearTimeout(this.debounce);
      this.debounce = undefined;
    }
    for (const child of this.runningProcesses) {
      void killProcessTree(child);
    }
    this.runningProcesses.clear();
    if (this.activeDebugSession !== undefined) {
      void vscode.debug.stopDebugging(this.activeDebugSession);
      this.activeDebugSession = undefined;
    }
    for (const watcher of this.watchers) {
      watcher.dispose();
    }
    for (const disposable of this.disposables) {
      disposable.dispose();
    }
    this.controller?.dispose();
    this.controller = undefined;
    this.tests.clear();
    this.files.clear();
    this.discovered.clear();
  }

  private log(text: string): void {
    this.output.appendLine(`testing: ${text}`);
  }

  private scheduleRefresh(): void {
    if (this.disposed) {
      return;
    }
    if (this.debounce !== undefined) {
      clearTimeout(this.debounce);
    }
    this.debounce = setTimeout(() => {
      this.debounce = undefined;
      void this.refresh();
    }, 500);
  }

  private ensureController(): vscode.TestController {
    if (this.controller !== undefined) {
      return this.controller;
    }
    const controller = vscode.tests.createTestController('stc', 'stc');
    controller.refreshHandler = (token) => this.refresh(token);
    controller.createRunProfile(
      'Run',
      vscode.TestRunProfileKind.Run,
      (request, token) => this.runTests(request, token),
      true,
    );
    controller.createRunProfile(
      'Debug',
      vscode.TestRunProfileKind.Debug,
      (request, token) => this.debugTests(request, token),
    );
    this.controller = controller;
    return controller;
  }

  private async refreshNow(token: vscode.CancellationToken | undefined): Promise<void> {
    if (this.disposed) {
      return;
    }
    const projects = this.findProjects();
    if (projects.length === 0) {
      this.discovered.clear();
      this.tests.clear();
      this.files.clear();
      this.controller?.items.replace([]);
      return;
    }
    const configured = stcExecutable();
    const executable = resolveExecutable(configured);
    if (executable === undefined) {
      this.log(
        `the stc executable \`${configured}\` was not found; tests were not discovered. ` +
          'Set `stc.path` or add stc to PATH.',
      );
      return;
    }
    for (const project of projects) {
      if (this.disposed || cancelled(token)) {
        return;
      }
      const result = await this.discover(project.manifest, executable);
      if (this.disposed || cancelled(token)) {
        return;
      }
      if (result.success) {
        this.discovered.set(project.manifest, result.tests);
      } else {
        this.log(
          `test discovery failed for ${project.manifest}: ${result.summary ?? 'unknown error'}`,
        );
      }
    }
    const present = new Set(projects.map((project) => project.manifest));
    for (const manifest of [...this.discovered.keys()]) {
      if (!present.has(manifest)) {
        this.discovered.delete(manifest);
      }
    }
    const controller = this.ensureController();
    this.buildItems(
      controller,
      projects.map((project) => ({
        ...project,
        tests: this.discovered.get(project.manifest) ?? [],
      })),
    );
  }

  /** Finds one project per unique manifest, using the language client's search. */
  private findProjects(): Project[] {
    const projects = new Map<string, Project>();
    for (const folder of vscode.workspace.workspaceFolders ?? []) {
      const manifest = findManifest(folder.uri.fsPath);
      if (manifest === undefined) {
        continue;
      }
      if (!projects.has(manifest)) {
        projects.set(manifest, {
          manifest,
          root: path.dirname(manifest),
          folder,
          tests: [],
        });
      }
    }
    return [...projects.values()];
  }

  private buildItems(controller: vscode.TestController, projects: readonly Project[]): void {
    this.tests.clear();
    this.files.clear();
    const fileItems: vscode.TestItem[] = [];
    for (const project of projects) {
      for (const group of groupTestsByFile(project.tests)) {
        const filePath = resolveResultPath(project.root, group.file);
        const uri = vscode.Uri.file(filePath);
        const folder = vscode.workspace.getWorkspaceFolder(uri) ?? project.folder;
        const fileId = this.fileId(project.manifest, group.file);
        const fileItem = controller.createTestItem(
          fileId,
          displayFileLabel(project.root, group.file),
          uri,
        );
        const children: string[] = [];
        for (const test of group.tests) {
          const testId = this.testId(project.manifest, group.file, test.name);
          const item = controller.createTestItem(testId, test.name, uri);
          // `--list` reports the declaration start, which is the
          // `{stc:test}` annotation line; a one-character range keeps the
          // item visible in the editor gutter and code lens.
          item.range = new vscode.Range(
            test.line - 1,
            test.column - 1,
            test.line - 1,
            test.column,
          );
          fileItem.children.add(item);
          this.tests.set(testId, {
            manifest: project.manifest,
            root: project.root,
            folder,
            test,
            item,
          });
          children.push(testId);
        }
        const first = group.tests[0];
        if (first !== undefined) {
          fileItem.range = new vscode.Range(
            first.line - 1,
            first.column - 1,
            first.line - 1,
            first.column,
          );
        }
        this.files.set(fileId, { children });
        fileItems.push(fileItem);
      }
    }
    controller.items.replace(fileItems);
  }

  private fileId(manifest: string, file: string): string {
    return `file:${encodeURIComponent(manifest)}:${encodeURIComponent(file)}`;
  }

  private testId(manifest: string, file: string, name: string): string {
    return `test:${encodeURIComponent(manifest)}:${encodeURIComponent(file)}:${encodeURIComponent(name)}`;
  }

  /**
   * Expands a request into test item ids.
   *
   * A file-level item selects all of its children. `request.include ===
   * undefined` means the whole controller, as VS Code does for "run all";
   * `exclude` is applied in both cases.
   */
  private selectedTestIds(request: vscode.TestRunRequest): string[] {
    const excluded = new Set<string>();
    for (const item of request.exclude ?? []) {
      const file = this.files.get(item.id);
      if (file === undefined) {
        excluded.add(item.id);
      } else {
        for (const child of file.children) {
          excluded.add(child);
        }
      }
    }
    const included = request.include;
    if (included === undefined) {
      return [...this.tests.keys()].filter((id) => !excluded.has(id));
    }
    const ids: string[] = [];
    const seen = new Set<string>();
    for (const item of included) {
      const file = this.files.get(item.id);
      const candidates = file === undefined ? [item.id] : file.children;
      for (const id of candidates) {
        if (excluded.has(id) || seen.has(id) || !this.tests.has(id)) {
          continue;
        }
        seen.add(id);
        ids.push(id);
      }
    }
    return ids;
  }

  private selectedTests(request: vscode.TestRunRequest): TestReference[] {
    const references: TestReference[] = [];
    for (const id of this.selectedTestIds(request)) {
      const reference = this.tests.get(id);
      if (reference !== undefined) {
        references.push(reference);
      }
    }
    return references;
  }

  private runGroups(request: vscode.TestRunRequest): RunGroup[] {
    const groups = new Map<string, RunGroup>();
    for (const id of this.selectedTestIds(request)) {
      const reference = this.tests.get(id);
      if (reference === undefined) {
        continue;
      }
      const existing = groups.get(reference.manifest);
      if (existing === undefined) {
        groups.set(reference.manifest, {
          manifest: reference.manifest,
          root: reference.root,
          tests: [reference],
        });
      } else {
        existing.tests.push(reference);
      }
    }
    return [...groups.values()];
  }

  private async runTests(
    request: vscode.TestRunRequest,
    token: vscode.CancellationToken,
  ): Promise<void> {
    const controller = this.controller;
    if (controller === undefined) {
      return;
    }
    const run = controller.createTestRun(request);
    const reported = new Set<string>();
    const groups = this.runGroups(request);
    const selected = groups.flatMap((group) => group.tests);
    try {
      const configured = stcExecutable();
      const executable = resolveExecutable(configured);
      if (executable === undefined) {
        const text =
          `the stc executable \`${configured}\` was not found; ` +
          'set `stc.path` or add stc to PATH.';
        this.log(text);
        for (const reference of selected) {
          reported.add(reference.item.id);
          run.errored(reference.item, new vscode.TestMessage(text));
        }
        return;
      }
      for (const group of groups) {
        if (token.isCancellationRequested) {
          for (const reference of group.tests) {
            reported.add(reference.item.id);
            run.skipped(reference.item);
          }
          continue;
        }
        await this.runGroup(run, group, executable, token, reported);
      }
    } catch (error) {
      this.log(`test run failed: ${message(error)}`);
    } finally {
      for (const reference of selected) {
        if (reported.has(reference.item.id)) {
          continue;
        }
        reported.add(reference.item.id);
        if (token.isCancellationRequested) {
          run.skipped(reference.item);
        } else {
          run.errored(reference.item, new vscode.TestMessage('the test did not run.'));
        }
      }
      run.end();
    }
  }

  /** Runs one project's selected tests in one `stc test` process. */
  private runGroup(
    run: vscode.TestRun,
    group: RunGroup,
    executable: string,
    token: vscode.CancellationToken,
    reported: Set<string>,
  ): Promise<void> {
    return new Promise((resolve) => {
      let child: StcProcess;
      try {
        child = spawnStc(
          executable,
          testRunArguments(
            group.manifest,
            group.tests.map((reference) => reference.test.name),
          ),
          group.root,
        );
      } catch (error) {
        const text = `stc could not be started: ${message(error)}`;
        this.log(text);
        for (const reference of group.tests) {
          reported.add(reference.item.id);
          run.errored(reference.item, new vscode.TestMessage(text));
        }
        resolve();
        return;
      }
      this.runningProcesses.add(child);
      const byName = new Map<string, TestReference[]>();
      for (const reference of group.tests) {
        const existing = byName.get(reference.test.name);
        if (existing === undefined) {
          byName.set(reference.test.name, [reference]);
        } else {
          existing.push(reference);
        }
      }
      const decoder = new StringDecoder('utf8');
      const stderrDecoder = new StringDecoder('utf8');
      const stream = new ResultStream((result) => {
        this.reportResult(run, result, byName, reported);
      });
      let spawnError: string | undefined;
      let finished = false;
      const finish = (code: number | null, signal: NodeJS.Signals | null): void => {
        if (finished) {
          return;
        }
        finished = true;
        this.runningProcesses.delete(child);
        cancellation.dispose();
        stream.push(decoder.end());
        const stderr = stderrDecoder.end();
        if (stderr !== '') {
          appendRunOutput(run, stderr);
        }
        const results = stream.finish();
        for (const problem of results.problems) {
          this.log(problem);
        }
        if (results.diagnostics.length > 0) {
          for (const diagnostic of results.diagnostics) {
            this.log(formatDiagnostic(diagnostic));
          }
          appendRunOutput(
            run,
            `${results.diagnostics.map(formatDiagnostic).join('\n')}\n`,
          );
        }
        const details = this.missingResultMessage(
          results.diagnostics,
          spawnError,
          code,
          signal,
        );
        for (const reference of group.tests) {
          if (reported.has(reference.item.id)) {
            continue;
          }
          reported.add(reference.item.id);
          if (token.isCancellationRequested) {
            run.skipped(reference.item);
          } else {
            run.errored(reference.item, new vscode.TestMessage(details));
          }
        }
        resolve();
      };
      const cancellation = token.onCancellationRequested(() => {
        void killProcessTree(child);
      });
      child.stdout.on('data', (chunk: Buffer) => {
        stream.push(decoder.write(chunk));
      });
      child.stderr.on('data', (chunk: Buffer) => {
        const text = stderrDecoder.write(chunk);
        if (text !== '') {
          appendRunOutput(run, text);
        }
      });
      child.once('error', (error) => {
        spawnError = message(error);
        if (child.pid === undefined) {
          finish(null, null);
        }
      });
      child.once('close', (code, signal) => {
        finish(code, signal);
      });
    });
  }

  private reportResult(
    run: vscode.TestRun,
    result: TestResult,
    byName: ReadonlyMap<string, readonly TestReference[]>,
    reported: Set<string>,
  ): void {
    const references = byName.get(result.name);
    if (references === undefined) {
      this.log(`ignoring result for unselected test ${result.name}`);
      return;
    }
    let first = true;
    for (const reference of references) {
      if (reported.has(reference.item.id)) {
        continue;
      }
      reported.add(reference.item.id);
      if (first && result.output.trim() !== '') {
        appendRunOutput(run, `[${result.name}]\n${result.output}\n`, reference.item);
      }
      first = false;
      appendRunOutput(run, `${formatResultSummary(result)}\n`, reference.item);
      switch (result.status) {
        case 'pass':
          run.passed(reference.item, result.durationMs);
          break;
        case 'fail':
        case 'trap':
        case 'timeout':
          run.failed(
            reference.item,
            this.failureMessage(reference, result),
            result.durationMs,
          );
          break;
      }
    }
  }

  private failureMessage(reference: TestReference, result: TestResult): vscode.TestMessage {
    const lines: string[] = [];
    if (result.status === 'trap') {
      lines.push(
        result.trap === undefined ? 'the test trapped' : `the test trapped: ${result.trap}`,
      );
    } else if (result.status === 'timeout') {
      lines.push('the test timed out');
    }
    for (const failure of result.failures) {
      lines.push(failure.message);
    }
    if (lines.length === 0) {
      lines.push('the test failed');
    }
    const message = new vscode.TestMessage(lines.join('\n'));
    const failure = result.failures.find(
      (candidate) => candidate.file !== undefined && candidate.line !== undefined,
    );
    if (failure?.file !== undefined && failure.line !== undefined) {
      const uri = vscode.Uri.file(resolveResultPath(reference.root, failure.file));
      const column = failure.column ?? 1;
      message.location = new vscode.Location(
        uri,
        new vscode.Range(failure.line - 1, column - 1, failure.line - 1, column),
      );
    }
    return message;
  }

  private missingResultMessage(
    diagnostics: readonly TestDiagnostic[],
    spawnError: string | undefined,
    code: number | null,
    signal: NodeJS.Signals | null,
  ): string {
    if (spawnError !== undefined) {
      return `stc could not be started: ${spawnError}`;
    }
    if (diagnostics.length > 0) {
      return 'the test did not run because the project did not build.';
    }
    if (signal !== null) {
      return `stc was terminated by ${signal}.`;
    }
    if (code !== 0) {
      return `stc test exited with code ${code ?? 'unknown'} without reporting this test.`;
    }
    return 'stc test did not report this test.';
  }

  /**
   * Starts one debug session per selected test, sequentially.
   *
   * F-17 accepts one `test` name per launch, so a file-level or multi-test
   * selection opens one session per test. Test Explorer results are not
   * fabricated for debug sessions: the F-17 Debug Console result line is the
   * observable outcome, and the session stays open under the debugger.
   */
  private async debugTests(
    request: vscode.TestRunRequest,
    token: vscode.CancellationToken,
  ): Promise<void> {
    const selected = this.selectedTests(request);
    if (selected.length === 0) {
      return;
    }
    for (const reference of selected) {
      if (this.disposed || token.isCancellationRequested) {
        return;
      }
      const configuration: vscode.DebugConfiguration = {
        ...testDebugConfiguration(reference.manifest, reference.test.name),
      };
      try {
        await this.debugOne(reference, configuration, token);
      } catch (error) {
        const text = `could not debug test ${reference.test.name}: ${message(error)}`;
        this.log(text);
        void vscode.window.showErrorMessage(`stc: ${text}`);
      }
    }
  }

  private debugOne(
    reference: TestReference,
    configuration: vscode.DebugConfiguration,
    token: vscode.CancellationToken,
  ): Promise<void> {
    return new Promise((resolve, reject) => {
      if (token.isCancellationRequested) {
        resolve();
        return;
      }
      let settled = false;
      let session: vscode.DebugSession | undefined;
      const matches = (candidate: vscode.DebugSession): boolean =>
        candidate.type === 'stc' &&
        candidate.configuration.test === configuration.test &&
        candidate.configuration.project === configuration.project;
      const finish = (error?: unknown): void => {
        if (settled) {
          return;
        }
        settled = true;
        startListener.dispose();
        endListener.dispose();
        cancelListener.dispose();
        if (this.activeDebugSession === session) {
          this.activeDebugSession = undefined;
        }
        if (error === undefined) {
          resolve();
        } else {
          reject(error instanceof Error ? error : new Error(String(error)));
        }
      };
      const startListener = vscode.debug.onDidStartDebugSession((started) => {
        if (!matches(started)) {
          return;
        }
        session = started;
        this.activeDebugSession = started;
        if (token.isCancellationRequested) {
          void vscode.debug.stopDebugging(started);
        }
      });
      const endListener = vscode.debug.onDidTerminateDebugSession((ended) => {
        if (matches(ended)) {
          finish();
        }
      });
      const cancelListener = token.onCancellationRequested(() => {
        if (session === undefined) {
          finish();
        } else {
          void vscode.debug.stopDebugging(session);
        }
      });
      vscode.debug.startDebugging(reference.folder, configuration).then(
        (started) => {
          if (!started) {
            finish(new Error('the debugger did not start'));
          } else if (token.isCancellationRequested) {
            if (session !== undefined) {
              void vscode.debug.stopDebugging(session);
            } else {
              finish();
            }
          }
        },
        (error: unknown) => finish(error),
      );
    });
  }

  private discover(manifest: string, executable: string): Promise<DiscoveryResult> {
    return new Promise((resolve) => {
      const root = path.dirname(manifest);
      let child: StcProcess;
      try {
        child = spawnStc(executable, testListArguments(manifest), root);
      } catch (error) {
        resolve({
          success: false,
          tests: [],
          summary: `stc could not be started: ${message(error)}`,
        });
        return;
      }
      this.runningProcesses.add(child);
      const decoder = new StringDecoder('utf8');
      const stderrDecoder = new StringDecoder('utf8');
      let stdout = '';
      let stderr = '';
      let spawnError: string | undefined;
      let finished = false;
      const finish = (code: number | null): void => {
        if (finished) {
          return;
        }
        finished = true;
        this.runningProcesses.delete(child);
        stdout += decoder.end();
        stderr += stderrDecoder.end();
        const parsed = parseTestList(stdout);
        const diagnostics = parseDiagnostics(stdout);
        for (const problem of parsed.problems) {
          this.log(`${manifest}: ${problem}`);
        }
        for (const diagnostic of diagnostics) {
          this.log(formatDiagnostic(diagnostic));
        }
        if (spawnError !== undefined) {
          resolve({
            success: false,
            tests: [],
            summary: `stc could not be started: ${spawnError}`,
          });
          return;
        }
        if (code !== 0) {
          const detail = stderr.trim() === '' ? '' : ` ${stderr.trim()}`;
          resolve({
            success: false,
            tests: [],
            summary: `stc test --list exited with code ${code ?? 'unknown'}.${detail}`,
          });
          return;
        }
        resolve({ success: true, tests: parsed.tests, summary: undefined });
      };
      child.stdout.on('data', (chunk: Buffer) => {
        stdout += decoder.write(chunk);
      });
      child.stderr.on('data', (chunk: Buffer) => {
        stderr += stderrDecoder.write(chunk);
      });
      child.once('error', (error) => {
        spawnError = message(error);
        if (child.pid === undefined) {
          finish(null);
        }
      });
      child.once('close', (code) => {
        finish(code);
      });
    });
  }
}

function appendRunOutput(
  run: vscode.TestRun,
  text: string,
  test?: vscode.TestItem,
): void {
  if (text === '') {
    return;
  }
  const normalized = text.replace(/\r?\n/g, '\r\n');
  run.appendOutput(
    normalized.endsWith('\r\n') ? normalized : `${normalized}\r\n`,
    undefined,
    test,
  );
}

function displayFileLabel(root: string, file: string): string {
  const absolute = resolveResultPath(root, file);
  const relative = path.relative(root, absolute);
  if (relative === '' || relative.startsWith('..') || path.isAbsolute(relative)) {
    return file;
  }
  return relative.split(path.sep).join('/');
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function cancelled(token: vscode.CancellationToken | undefined): boolean {
  return token?.isCancellationRequested === true;
}
