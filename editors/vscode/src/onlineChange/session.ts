import { randomBytes } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdir, open, stat, type FileHandle } from 'node:fs/promises';
import * as path from 'node:path';
import * as readline from 'node:readline';
import type { Readable } from 'node:stream';
import * as vscode from 'vscode';

import { findManifest, resolveExecutable } from '../languageClient';
import { killProcessTree, waitForExit } from '../processTree';
import { findProjectRoots, stcExecutable } from '../tasks';
import {
  ENDPOINT_MAX_BYTES,
  NON_SAFETY_NOTICE,
  applyEvent,
  classifyStatusLine,
  onlineChangeSnapshot,
  parseEndpointSnapshot,
  redactLine,
  statusBarText,
  statusTooltip,
  type EndpointSnapshot,
  type OnlineChangeSnapshot,
  type OnlineState,
} from './state';

/** Arguments that would conflict with the extension-owned invocation. */
const OWNED_ARGUMENTS: readonly string[] = [
  '--debug-server',
  '--debug-endpoint',
  '--manifest-path',
  '--control',
  '--control-nonce',
  '-q',
  '--quiet',
  '--help',
  '--version',
];

const POLL_INTERVAL_MS = 750;
const DEPLOY_RECONCILE_MS = 2_000;
const ATTACH_ENDPOINT_WAIT_MS = 10_000;

const START_MESSAGE = 'stc: Start Online Change';
const STOP_ACTION = 'Stop Online Change';

interface ResolvedProject {
  readonly folder: vscode.WorkspaceFolder | undefined;
  readonly root: string;
}

interface AttachRecord {
  readonly runId: number;
  readonly generation: number;
  readonly buildId: string;
}

type EndpointRead =
  | { readonly kind: 'ok'; readonly text: string; readonly size: number }
  | { readonly kind: 'missing' }
  | { readonly kind: 'failure'; readonly reason: string };

/**
 * Reads at most `ENDPOINT_MAX_BYTES` from a regular endpoint file.
 *
 * The handle is opened and inspected first so a non-regular file or an
 * oversized file is rejected without reading it. The text is only used to
 * parse the whitelisted snapshot fields and is not retained by the caller.
 */
async function readEndpointFile(file: string): Promise<EndpointRead> {
  let handle: FileHandle;
  try {
    handle = await open(file, 'r');
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    return code === 'ENOENT' ? { kind: 'missing' } : { kind: 'failure', reason: 'open' };
  }
  try {
    const info = await handle.stat();
    if (!info.isFile()) {
      return { kind: 'failure', reason: 'not-a-regular-file' };
    }
    if (info.size > ENDPOINT_MAX_BYTES) {
      return { kind: 'failure', reason: 'too-large' };
    }
    const buffer = Buffer.alloc(ENDPOINT_MAX_BYTES);
    const { bytesRead } = await handle.read(buffer, 0, ENDPOINT_MAX_BYTES, 0);
    return { kind: 'ok', text: buffer.subarray(0, bytesRead).toString('utf8'), size: bytesRead };
  } catch {
    return { kind: 'failure', reason: 'read' };
  } finally {
    await handle.close().catch(() => undefined);
  }
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function withTimeout<T>(promise: Promise<T>, timeoutMs: number, what: string): Promise<T> {
  let timer: NodeJS.Timeout | undefined;
  const timeout = new Promise<T>((_resolve, reject) => {
    timer = setTimeout(() => reject(new Error(`timed out after ${timeoutMs}ms waiting for ${what}`)), timeoutMs);
  });
  return Promise.race([promise, timeout]).finally(() => {
    if (timer !== undefined) {
      clearTimeout(timer);
    }
  });
}

function isInside(candidate: string, folder: string): boolean {
  const relative = path.relative(folder, candidate);
  return relative === '' || (!relative.startsWith('..') && !path.isAbsolute(relative));
}

function rejectedArgument(args: readonly string[]): string | undefined {
  for (const arg of args) {
    for (const owned of OWNED_ARGUMENTS) {
      if (arg === owned || arg.startsWith(`${owned}=`)) {
        return owned;
      }
    }
  }
  return undefined;
}

function failureMessage(detail: string): string {
  switch (detail) {
    case 'candidate-rejected':
      return 'The online-change candidate was rejected; the previous build is still active.';
    case 'deploy-failed':
      return 'The online-change deploy failed; the previous build is still active.';
    default:
      return 'The online-change build failed; the previous build is still active.';
  }
}

/**
 * Owns the extension-side online-change session.
 *
 * The extension starts the existing `stc run --watch` workflow as a child
 * process, parses its `[stc]` status lines, and reads the runtime's private
 * debug endpoint file for the numeric generation and build. It never connects
 * to the control channel and never reads or retains control metadata.
 */
export class OnlineChangeSession {
  private readonly channel: vscode.OutputChannel;
  private readonly statusItem: vscode.StatusBarItem;
  private readonly disposables: vscode.Disposable[] = [];

  private state: OnlineState = { kind: 'idle' };
  private child: import('node:child_process').ChildProcess | undefined;
  private readers: readline.Interface[] = [];
  private pollTimer: NodeJS.Timeout | undefined;
  private deployTimer: NodeJS.Timeout | undefined;
  private endpointPath: string | undefined;
  private endpointProblem: string | undefined;

  private runId = 0;
  private starting = false;
  private attachInProgress = false;
  private requestedStop = false;
  private userStopped = false;
  private forced = false;
  private finalized = false;
  private deactivating = false;
  private lastFailureKey: string | undefined;
  private readonly attachedSessions = new Map<string, AttachRecord>();

  public constructor(private readonly context: vscode.ExtensionContext) {
    this.channel = vscode.window.createOutputChannel('stc Online Change');
    this.statusItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
    this.statusItem.command = 'stc.showOnlineChangeStatus';
    context.subscriptions.push(this.channel, this.statusItem);

    this.register(context, 'stc.startOnlineChange', () => this.start());
    this.register(context, 'stc.stopOnlineChange', () => this.stop());
    this.register(context, 'stc.showOnlineChangeStatus', () => this.showStatus());
    this.register(context, 'stc.attachOnlineDebugger', () => this.attach());

    const terminated = vscode.debug.onDidTerminateDebugSession((session) =>
      this.onDebugTerminated(session),
    );
    context.subscriptions.push(terminated);
    this.disposables.push(terminated);

    this.updateUi();
  }

  private register(
    context: vscode.ExtensionContext,
    command: string,
    handler: () => unknown,
  ): void {
    const disposable = vscode.commands.registerCommand(command, handler);
    context.subscriptions.push(disposable);
    this.disposables.push(disposable);
  }

  /** Stops an owned child during extension deactivation with a short wait. */
  public async stopForDeactivate(): Promise<void> {
    this.deactivating = true;
    await this.stopInternal(2_000);
  }

  /** Releases the listeners and timers this session registered itself. */
  public dispose(): void {
    this.clearTimers();
    this.closeReaders();
    for (const disposable of this.disposables.splice(0)) {
      disposable.dispose();
    }
    this.statusItem.dispose();
    this.channel.dispose();
  }

  private async start(): Promise<void> {
    if (this.deactivating || this.starting) {
      return;
    }
    if (this.child !== undefined || this.state.kind === 'starting' || this.state.kind === 'running') {
      const action = await vscode.window.showWarningMessage(
        'An online change is already running in this window. Stop it before starting another.',
        STOP_ACTION,
      );
      if (action === STOP_ACTION) {
        await this.stop();
      }
      return;
    }
    this.starting = true;
    try {
      await this.startSession();
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      this.channel.appendLine(`[stc-vscode] the online change failed to start: ${message}`);
      void vscode.window.showErrorMessage(`The online change failed to start: ${message}`);
    } finally {
      this.starting = false;
    }
  }

  private async startSession(): Promise<void> {
    const configured = stcExecutable();
    const executable = resolveExecutable(configured);
    if (executable === undefined) {
      const message =
        `stc executable \`${configured}\` was not found; set \`stc.path\` or add stc to PATH.`;
      this.channel.appendLine(`[stc-vscode] ${message}`);
      void vscode.window.showErrorMessage(message);
      return;
    }
    if (process.platform === 'win32' && /\.(cmd|bat)$/i.test(executable)) {
      const message =
        `stc executable \`${configured}\` resolves to a .cmd/.bat shim. ` +
        'Set `stc.path` to the native stc.exe so the extension can start and stop it reliably.';
      this.channel.appendLine(`[stc-vscode] ${message}`);
      void vscode.window.showErrorMessage(message);
      return;
    }

    const project = await this.selectProject();
    if (project === undefined) {
      return;
    }
    const manifest = path.join(project.root, 'stc.toml');
    try {
      if (!(await stat(manifest)).isFile()) {
        throw new Error(`${manifest} is not a file`);
      }
    } catch {
      const message = `No stc.toml was found at ${manifest}; choose a project directory.`;
      this.channel.appendLine(`[stc-vscode] ${message}`);
      void vscode.window.showErrorMessage(message);
      return;
    }

    const userArgs = this.userArguments(project);
    if (userArgs === undefined) {
      return;
    }

    if (vscode.debug.activeDebugSession?.type === 'stc') {
      const proceed = await vscode.window.showWarningMessage(
        'A stc debug session is active. Starting an online change launches a second local host. Continue?',
        { modal: true },
        'Continue',
      );
      if (proceed !== 'Continue') {
        return;
      }
    }

    const storage = this.context.storageUri ?? this.context.globalStorageUri;
    const endpointDirectory = path.join(
      storage.fsPath,
      'oc',
      randomBytes(4).toString('hex'),
    );
    const endpointPath = path.join(endpointDirectory, 'd.json');
    try {
      await mkdir(endpointDirectory, { recursive: true });
    } catch (error) {
      const message = `The online-change state directory could not be created: ${
        error instanceof Error ? error.message : String(error)
      }`;
      this.channel.appendLine(`[stc-vscode] ${message}`);
      void vscode.window.showErrorMessage(message);
      return;
    }

    this.runId += 1;
    this.requestedStop = false;
    this.userStopped = false;
    this.forced = false;
    this.finalized = false;
    this.lastFailureKey = undefined;
    this.endpointProblem = undefined;
    this.attachedSessions.clear();
    this.endpointPath = endpointPath;

    this.channel.appendLine('');
    this.channel.appendLine(`[stc-vscode] ${NON_SAFETY_NOTICE}`);
    this.channel.appendLine(`[stc-vscode] starting stc run --watch in ${project.root}`);
    this.channel.show(true);
    this.applyState(applyEvent(this.state, { type: 'start', project: project.root }));

    const args = [
      'run',
      '--watch',
      '--manifest-path',
      manifest,
      '--debug-server',
      '--debug-endpoint',
      endpointPath,
      '--color',
      'never',
      ...userArgs,
    ];
    let child: import('node:child_process').ChildProcess;
    try {
      child = spawn(executable, args, {
        cwd: project.root,
        stdio: ['pipe', 'pipe', 'pipe'],
        detached: process.platform !== 'win32',
        windowsHide: true,
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      this.channel.appendLine(`[stc-vscode] the watch process could not be started: ${message}`);
      this.finalized = true;
      this.endpointPath = undefined;
      const stopped = applyEvent(this.state, {
        type: 'exited',
        code: null,
        requested: false,
        forced: false,
      });
      this.applyState(
        stopped.kind === 'stopped' ? { ...stopped, lastError: message } : stopped,
      );
      this.notifyFailure(`spawn:${message}`, 'error', `The watch process could not be started: ${message}`);
      return;
    }

    this.child = child;
    this.readers = [
      this.createReader(child.stdout, false),
      this.createReader(child.stderr, true),
    ].filter((reader): reader is readline.Interface => reader !== undefined);
    child.once('error', (error) => this.onChildError(error));
    child.once('exit', (code, signal) => this.onChildExit(code, signal));
    this.pollTimer = setInterval(() => {
      void this.pollEndpoint();
    }, POLL_INTERVAL_MS);
    void this.pollEndpoint();
  }

  private userArguments(project: ResolvedProject): string[] | undefined {
    const scope = project.folder?.uri ?? vscode.Uri.file(project.root);
    const configured = vscode.workspace
      .getConfiguration('stc', scope)
      .get<unknown>('onlineChange.args', []);
    if (!Array.isArray(configured) || configured.some((value) => typeof value !== 'string')) {
      const message = '`stc.onlineChange.args` must be an array of strings.';
      this.channel.appendLine(`[stc-vscode] ${message}`);
      void vscode.window.showErrorMessage(message);
      return undefined;
    }
    const args = configured as string[];
    const rejected = rejectedArgument(args);
    if (rejected !== undefined) {
      const message =
        `\`stc.onlineChange.args\` contains \`${rejected}\`, which is owned by the extension. ` +
        'Remove it from the setting.';
      this.channel.appendLine(`[stc-vscode] ${message}`);
      void vscode.window.showErrorMessage(message);
      return undefined;
    }
    return args;
  }

  private async selectProject(): Promise<ResolvedProject | undefined> {
    const editor = vscode.window.activeTextEditor;
    if (editor !== undefined && editor.document.uri.scheme === 'file') {
      const manifest = findManifest(path.dirname(editor.document.uri.fsPath));
      if (manifest !== undefined) {
        const root = path.dirname(manifest);
        const folder = (vscode.workspace.workspaceFolders ?? []).find((candidate) =>
          isInside(root, candidate.uri.fsPath),
        );
        if (folder !== undefined) {
          return { folder, root };
        }
      }
    }

    const roots: ResolvedProject[] = [];
    const seen = new Set<string>();
    for (const discovered of await findProjectRoots()) {
      if (discovered.root === undefined) {
        continue;
      }
      const key = process.platform === 'win32' ? discovered.root.toLowerCase() : discovered.root;
      if (!seen.has(key)) {
        seen.add(key);
        roots.push({ folder: discovered.folder, root: discovered.root });
      }
    }
    if (roots.length === 1) {
      return roots[0];
    }
    if (roots.length === 0) {
      const message = 'No stc project was found in this workspace. Open a folder with stc.toml.';
      this.channel.appendLine(`[stc-vscode] ${message}`);
      void vscode.window.showErrorMessage(message);
      return undefined;
    }
    const picked = await vscode.window.showQuickPick(
      roots.map((project) => ({
        label: project.folder?.name ?? path.basename(project.root),
        description: project.root,
        project,
      })),
      { placeHolder: 'Select the stc project for the online change' },
    );
    return picked?.project;
  }

  private createReader(
    stream: Readable | null,
    isStderr: boolean,
  ): readline.Interface | undefined {
    if (stream === null) {
      return undefined;
    }
    const reader = readline.createInterface({ input: stream, crlfDelay: Infinity });
    reader.on('line', (line) => this.handleLine(line, isStderr));
    reader.on('error', () => undefined);
    return reader;
  }

  private handleLine(raw: string, isStderr: boolean): void {
    const visible = redactLine(raw);
    if (visible !== undefined && visible !== '') {
      this.channel.appendLine(visible);
    }
    if (!isStderr) {
      return;
    }
    const signal = classifyStatusLine(raw);
    this.applyState(applyEvent(this.state, { type: 'line', line: raw }));
    if (signal === undefined) {
      return;
    }
    if (signal.type === 'deployed') {
      void this.pollEndpoint();
      this.scheduleDeployReconciliation();
    } else if (signal.type === 'kept') {
      this.notifyFailure(
        `kept:${signal.detail}:${signal.buildId}`,
        'warning',
        failureMessage(signal.detail),
      );
    }
  }

  private scheduleDeployReconciliation(): void {
    this.clearDeployTimer();
    this.deployTimer = setTimeout(() => {
      this.deployTimer = undefined;
      if (this.state.kind !== 'running' || this.state.pendingBuildId === undefined) {
        return;
      }
      const pending = this.state.pendingBuildId;
      const generation = this.state.generation;
      this.applyState(applyEvent(this.state, { type: 'deploy-unconfirmed' }));
      this.channel.appendLine(
        `[stc-vscode] the debug endpoint did not confirm build ${pending} within ` +
          `${DEPLOY_RECONCILE_MS / 1000} s; showing generation ` +
          `${generation === undefined ? 'unknown' : String(generation)} as the last confirmed value`,
      );
    }, DEPLOY_RECONCILE_MS);
  }

  private async pollEndpoint(): Promise<void> {
    const endpointPath = this.endpointPath;
    if (endpointPath === undefined || this.child === undefined) {
      return;
    }
    const result = await readEndpointFile(endpointPath);
    if (result.kind === 'missing') {
      return;
    }
    if (result.kind === 'failure') {
      this.reportEndpointProblem(result.reason);
      return;
    }
    const parsed = parseEndpointSnapshot(result.text, { isFile: true, size: result.size });
    if (!parsed.ok) {
      this.reportEndpointProblem(parsed.reason);
      return;
    }
    this.endpointProblem = undefined;
    this.applyState(applyEvent(this.state, { type: 'endpoint', snapshot: parsed.snapshot }));
  }

  private reportEndpointProblem(reason: string): void {
    if (this.endpointProblem === reason) {
      return;
    }
    this.endpointProblem = reason;
    this.channel.appendLine(`[stc-vscode] the debug endpoint is not readable yet (${reason})`);
  }

  private applyState(next: OnlineState): void {
    const previous = this.state;
    if (next === previous) {
      return;
    }
    this.state = next;
    if (next.kind === 'running' && next.pendingBuildId === undefined) {
      this.clearDeployTimer();
    }
    if (
      next.kind === 'running' &&
      next.phase === 'active' &&
      next.buildId !== undefined &&
      (previous.kind !== 'running' || previous.buildId !== next.buildId)
    ) {
      this.channel.appendLine(
        `[stc-vscode] active generation ${next.generation ?? 'unknown'} (build ${next.buildId})`,
      );
      this.lastFailureKey = undefined;
    }
    this.updateUi();
  }

  private updateUi(): void {
    const text = statusBarText(this.state);
    if (text === undefined) {
      this.statusItem.hide();
    } else {
      this.statusItem.text = text;
      this.statusItem.tooltip = statusTooltip(this.state);
      this.statusItem.show();
    }
    const active = this.state.kind === 'starting' || this.state.kind === 'running';
    const attachable =
      this.state.kind === 'running' &&
      this.state.generation !== undefined &&
      this.endpointPath !== undefined &&
      this.child !== undefined;
    void vscode.commands.executeCommand('setContext', 'stc.onlineChangeActive', active);
    void vscode.commands.executeCommand('setContext', 'stc.onlineChangeAttachable', attachable);
  }

  private showStatus(): OnlineChangeSnapshot {
    this.channel.show(true);
    return onlineChangeSnapshot(this.state);
  }

  private async stop(): Promise<void> {
    await this.stopInternal(5_000);
  }

  private async stopInternal(graceMs: number): Promise<void> {
    const child = this.child;
    if (child === undefined) {
      return;
    }
    this.requestedStop = true;
    this.userStopped = true;
    this.channel.appendLine(
      '[stc-vscode] stopping the online change (closing the watch stdin)',
    );
    try {
      child.stdin?.end();
    } catch (error) {
      this.channel.appendLine(
        `[stc-vscode] closing the watch stdin failed: ${
          error instanceof Error ? error.message : String(error)
        }`,
      );
    }
    let exited = await waitForExit(child, graceMs);
    if (!exited) {
      this.forced = true;
      this.channel.appendLine(
        `[stc-vscode] the watch did not stop within ${Math.round(graceMs / 1000)} s; ` +
          'forcing the process tree; retained state may not have been saved',
      );
      await killProcessTree(child, 2_000);
      exited = await waitForExit(child, 3_000);
    }
    if (!this.finalized) {
      if (!exited) {
        this.channel.appendLine('[stc-vscode] the watch process is still present after a forced stop');
      }
      this.finishChild(child.exitCode, child.signalCode);
    }
  }

  private onChildError(error: Error): void {
    if (this.finalized) {
      return;
    }
    this.channel.appendLine(`[stc-vscode] the watch process failed: ${error.message}`);
    this.finalized = true;
    this.clearTimers();
    this.closeReaders();
    this.child = undefined;
    this.endpointPath = undefined;
    const stopped = applyEvent(this.state, {
      type: 'exited',
      code: null,
      requested: false,
      forced: false,
    });
    this.applyState(
      stopped.kind === 'stopped' ? { ...stopped, lastError: error.message } : stopped,
    );
    this.notifyFailure(
      `spawn:${error.message}`,
      'error',
      `The online-change watch process failed: ${error.message}`,
    );
  }

  private onChildExit(code: number | null, signal: NodeJS.Signals | null): void {
    this.finishChild(code, signal);
  }

  private finishChild(code: number | null, signal: NodeJS.Signals | null): void {
    if (this.finalized) {
      return;
    }
    this.finalized = true;
    this.clearTimers();
    this.closeReaders();
    this.child = undefined;
    this.endpointPath = undefined;
    this.applyState(
      applyEvent(this.state, {
        type: 'exited',
        code,
        requested: this.requestedStop,
        forced: this.forced,
      }),
    );
    this.channel.appendLine(
      `[stc-vscode] watch process exited (${
        code === null ? (signal === null ? 'no exit code' : signal) : `code ${code}`
      })`,
    );
    if (this.deactivating || this.state.kind !== 'stopped') {
      return;
    }
    const stopped = this.state;
    if (stopped.reason === 'failed' || stopped.reason === 'host-exited') {
      const ended =
        stopped.exitCode === undefined
          ? `The online change ended (${stopped.reason}).`
          : `The online change ended (${stopped.reason}, exit code ${stopped.exitCode}).`;
      this.notifyFailure(
        `exit:${stopped.reason}:${stopped.lastError ?? ''}`,
        stopped.reason === 'failed' ? 'error' : 'warning',
        stopped.lastError ?? ended,
      );
    } else if (stopped.reason === 'forced') {
      this.notifyFailure(
        'exit:forced',
        'warning',
        'The online change was force-stopped; retained state may not have been saved.',
      );
    }
  }

  private notifyFailure(
    key: string,
    severity: 'warning' | 'error',
    message: string,
  ): void {
    if (this.deactivating || this.lastFailureKey === key) {
      return;
    }
    this.lastFailureKey = key;
    const shown =
      severity === 'error'
        ? vscode.window.showErrorMessage(message, 'Show Output')
        : vscode.window.showWarningMessage(message, 'Show Output');
    void shown.then((action) => {
      if (action === 'Show Output') {
        this.channel.show();
      }
    });
  }

  private async attach(): Promise<void> {
    if (this.attachInProgress) {
      return;
    }
    if (this.state.kind !== 'running' || this.endpointPath === undefined) {
      void vscode.window.showErrorMessage(
        `No online change is running. Start one with "${START_MESSAGE}" first.`,
      );
      return;
    }
    if (this.attachedSessions.size > 0) {
      void vscode.window.showWarningMessage(
        'A debug session is already attached to this online change.',
      );
      return;
    }
    this.attachInProgress = true;
    try {
      const snapshot = await this.waitForEndpoint(ATTACH_ENDPOINT_WAIT_MS);
      const endpoint = this.endpointPath;
      if (snapshot === undefined || endpoint === undefined || this.state.kind !== 'running') {
        const message =
          'The online-change host is not ready yet (no debug endpoint). ' +
          'Wait for the first activation and try again.';
        this.channel.appendLine(`[stc-vscode] ${message}`);
        void vscode.window.showErrorMessage(message);
        return;
      }
      const project = this.state.project;
      const configuration: vscode.DebugConfiguration = {
        type: 'stc',
        request: 'attach',
        name: 'stc: Online Change',
        project: path.join(project, 'stc.toml'),
        endpoint,
      };

      let resolveStarted: ((session: vscode.DebugSession) => void) | undefined;
      const started = new Promise<vscode.DebugSession>((resolve) => {
        resolveStarted = resolve;
      });
      const listener = vscode.debug.onDidStartDebugSession((session) => {
        if (session.type === 'stc' && session.configuration?.name === configuration.name) {
          resolveStarted?.(session);
        }
      });
      try {
        const launched = await vscode.debug.startDebugging(
          this.folderFor(project),
          configuration,
        );
        if (!launched) {
          const message =
            'The stc debug session was not started. Check the "stc" output channel for the adapter error.';
          this.channel.appendLine(`[stc-vscode] ${message}`);
          void vscode.window.showErrorMessage(message);
          return;
        }
        const session = await withTimeout(
          started,
          10_000,
          'the stc debug session to start',
        ).catch(() => undefined);
        if (session === undefined) {
          this.channel.appendLine(
            '[stc-vscode] the debug session started but was not observed; not tracking a swap',
          );
          return;
        }
        this.attachedSessions.set(session.id, {
          runId: this.runId,
          generation: snapshot.generation,
          buildId: snapshot.buildId,
        });
        this.channel.appendLine(
          `[stc-vscode] attached the debugger to generation ${snapshot.generation} (build ${snapshot.buildId})`,
        );
      } finally {
        listener.dispose();
      }
    } finally {
      this.attachInProgress = false;
    }
  }

  private async waitForEndpoint(timeoutMs: number): Promise<EndpointSnapshot | undefined> {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const endpointPath = this.endpointPath;
      if (endpointPath === undefined || this.state.kind !== 'running') {
        return undefined;
      }
      const result = await readEndpointFile(endpointPath);
      if (result.kind === 'ok') {
        const parsed = parseEndpointSnapshot(result.text, {
          isFile: true,
          size: result.size,
        });
        if (parsed.ok) {
          return parsed.snapshot;
        }
      }
      if (Date.now() >= deadline) {
        return undefined;
      }
      await delay(100);
    }
  }

  private folderFor(project: string): vscode.WorkspaceFolder | undefined {
    return (vscode.workspace.workspaceFolders ?? []).find((folder) =>
      isInside(project, folder.uri.fsPath),
    );
  }

  private onDebugTerminated(session: vscode.DebugSession): void {
    const record = this.attachedSessions.get(session.id);
    if (record === undefined) {
      return;
    }
    this.attachedSessions.delete(session.id);
    if (
      record.runId !== this.runId ||
      this.userStopped ||
      this.deactivating ||
      this.state.kind !== 'running'
    ) {
      return;
    }
    void this.reportSessionEnded(record);
  }

  private async reportSessionEnded(record: AttachRecord): Promise<void> {
    const endpointPath = this.endpointPath;
    if (endpointPath === undefined) {
      return;
    }
    const result = await readEndpointFile(endpointPath);
    if (result.kind !== 'ok') {
      return;
    }
    const parsed = parseEndpointSnapshot(result.text, {
      isFile: true,
      size: result.size,
    });
    if (!parsed.ok) {
      return;
    }
    if (parsed.snapshot.generation === record.generation) {
      this.channel.appendLine(
        `[stc-vscode] the debug session ended without an online-change generation change ` +
          `(generation ${record.generation}); no swap claimed`,
      );
      return;
    }
    this.channel.appendLine(
      `[stc-vscode] the debug session ended on a swap ` +
        `(generation ${record.generation} -> ${parsed.snapshot.generation}); ` +
        're-attach to debug the new generation',
    );
    const action = await vscode.window.showInformationMessage(
      'The online change ended the debug session (STC-E-DEBUG-019 is in the Debug Console). ' +
        'Re-attach to debug the new generation.',
      'Attach Debugger',
      'Show Output',
    );
    if (action === 'Attach Debugger') {
      await this.attach();
    } else if (action === 'Show Output') {
      this.channel.show();
    }
  }

  private clearPollTimer(): void {
    if (this.pollTimer !== undefined) {
      clearInterval(this.pollTimer);
      this.pollTimer = undefined;
    }
  }

  private clearDeployTimer(): void {
    if (this.deployTimer !== undefined) {
      clearTimeout(this.deployTimer);
      this.deployTimer = undefined;
    }
  }

  private clearTimers(): void {
    this.clearPollTimer();
    this.clearDeployTimer();
  }

  private closeReaders(): void {
    for (const reader of this.readers.splice(0)) {
      reader.close();
    }
  }
}
