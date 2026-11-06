import { statSync } from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import {
  CloseAction,
  ErrorAction,
  LanguageClient,
  LanguageClientOptions,
  RevealOutputChannelOn,
  ServerOptions,
} from 'vscode-languageclient/node';
import type { ErrorHandler } from 'vscode-languageclient/node';

import { resolveExecutable } from './executable';
import { RestartPolicy } from './restartPolicy';
import { stcExecutable } from './tasks';

const MANIFEST = 'stc.toml';

/** One language client and the restart policy that governs it. */
interface ClientEntry {
  readonly manifest: string;
  readonly folder: vscode.WorkspaceFolder;
  readonly client: LanguageClient;
  readonly policy: RestartPolicy;
}

/**
 * Hosts one `stc lsp` process per workspace folder that contains a project.
 *
 * The extension is only the client: diagnostics, navigation, hover,
 * completion, and formatting all come from the compiler through the language
 * server. One server instance serves one project (the nearest `stc.toml`).
 */
export class StcLanguageClient implements vscode.Disposable {
  private readonly output: vscode.OutputChannel;
  private clients: ClientEntry[] = [];
  private queue: Promise<void> = Promise.resolve();
  private disposed = false;

  public constructor() {
    this.output = vscode.window.createOutputChannel('stc Language Server');
  }

  /** Starts one client for every workspace folder with a project. */
  public start(): Promise<void> {
    return this.enqueue(() => this.startNow());
  }

  /** Stops every client. */
  public stop(): Promise<void> {
    return this.enqueue(() => this.stopNow());
  }

  /** Stops and starts the clients again (after `stc.path` changed). */
  public restart(): Promise<void> {
    return this.enqueue(async () => {
      await this.stopNow();
      await this.startNow();
    });
  }

  public dispose(): void {
    this.disposed = true;
    void this.stop();
    this.output.dispose();
  }

  private enqueue(action: () => Promise<void>): Promise<void> {
    const next = this.queue.then(action, action);
    this.queue = next.catch(() => undefined);
    return next;
  }

  private log(message: string): void {
    this.output.appendLine(message);
  }

  private async startNow(): Promise<void> {
    await this.stopNow();
    if (this.disposed) {
      return;
    }
    const configured = stcExecutable();
    const executable = resolveExecutable(configured);
    if (executable === undefined) {
      this.log(
        `stc executable \`${configured}\` was not found; the language server is not started. ` +
          'Set `stc.path` or add stc to PATH.',
      );
      return;
    }
    const folders = vscode.workspace.workspaceFolders ?? [];
    if (folders.length === 0) {
      this.log('No workspace folder is open; the language server is not started.');
      return;
    }
    const projects = new Map<string, vscode.WorkspaceFolder>();
    for (const folder of folders) {
      const manifest = findManifest(folder.uri.fsPath);
      if (manifest === undefined) {
        this.log(
          `No stc.toml found for ${folder.uri.fsPath}; no language server is started for it.`,
        );
        continue;
      }
      if (!projects.has(manifest)) {
        projects.set(manifest, folder);
      }
    }
    if (projects.size === 0) {
      this.log('No stc.toml was found in any workspace folder; the language server is not started.');
      return;
    }
    const entries: ClientEntry[] = [];
    for (const [manifest, folder] of projects) {
      entries.push(this.createEntry(executable, manifest, folder));
    }
    this.clients.push(...entries);
    for (const entry of entries) {
      await this.startEntry(entry);
    }
  }

  private async stopNow(): Promise<void> {
    const clients = this.clients;
    this.clients = [];
    for (const entry of clients) {
      await this.stopEntry(entry);
    }
  }

  private async stopEntry(entry: ClientEntry): Promise<void> {
    try {
      await entry.client.stop();
    } catch (error) {
      // A failed initialization leaves the client in `startFailed`, where the
      // language client's own `stop` refuses to run. The process is already
      // being torn down and `dispose` finishes the cleanup.
      if (!entry.policy.hasFailedInitialization) {
        this.log(`Failed to stop the language server: ${message(error)}`);
      }
    }
    try {
      await entry.client.dispose();
    } catch {
      // The client was already stopped.
    }
  }

  private async startEntry(entry: ClientEntry): Promise<void> {
    try {
      await entry.client.start();
    } catch (error) {
      this.log(
        `Failed to start the language server for ${entry.manifest}: ${message(error)}`,
      );
      // A deterministic project-load failure must be reported once and must
      // not restart; `RestartPolicy` has already marked the failure.
      void this.reportStartFailure(entry, error);
    }
  }

  /**
   * Surfaces an initialization failure with the two actions it needs: show the
   * output channel, or retry after `stc.toml` was fixed.
   *
   * The language client still forces one generic "couldn't create connection"
   * notification of its own; this notification carries the manifest detail and
   * the recovery action.
   */
  private async reportStartFailure(entry: ClientEntry, error: unknown): Promise<void> {
    const choice = await vscode.window.showErrorMessage(
      `stc: the language server could not start. ${message(error)}`,
      'Open Output',
      'Retry',
    );
    if (choice === 'Open Output') {
      this.output.show(true);
    } else if (choice === 'Retry') {
      await this.retry(entry).catch((retryError: unknown) => {
        this.log(`Failed to retry the language server: ${message(retryError)}`);
      });
    }
  }

  /** Recreates one failed client after the user fixes the project. */
  private retry(entry: ClientEntry): Promise<void> {
    return this.enqueue(async () => {
      if (this.disposed || !this.clients.includes(entry)) {
        return;
      }
      await this.stopEntry(entry);
      this.clients = this.clients.filter((candidate) => candidate !== entry);
      const configured = stcExecutable();
      const executable = resolveExecutable(configured);
      if (executable === undefined) {
        this.log(
          `stc executable \`${configured}\` was not found; the language server is not started. ` +
            'Set `stc.path` or add stc to PATH.',
        );
        return;
      }
      const next = this.createEntry(executable, entry.manifest, entry.folder);
      this.clients.push(next);
      await this.startEntry(next);
    });
  }

  private createEntry(
    executable: string,
    manifest: string,
    folder: vscode.WorkspaceFolder,
  ): ClientEntry {
    const serverOptions: ServerOptions = {
      command: executable,
      args: ['lsp', '--manifest-path', manifest],
      options: { cwd: path.dirname(manifest) },
    };
    const policy = new RestartPolicy();
    const errorHandler: ErrorHandler = {
      error: (_error, _message, count) => {
        const decision = policy.connectionError(count);
        return decision.action === 'continue'
          ? { action: ErrorAction.Continue }
          : { action: ErrorAction.Shutdown, message: decision.message };
      },
      closed: () => {
        const decision = policy.closed();
        return decision.action === 'restart'
          ? { action: CloseAction.Restart, message: decision.message }
          : {
              action: CloseAction.DoNotRestart,
              message: decision.message,
              handled: decision.handled,
            };
      },
    };
    const clientOptions: LanguageClientOptions = {
      documentSelector: [{ scheme: 'file', language: 'st' }],
      workspaceFolder: folder,
      synchronize: {
        fileEvents: [
          vscode.workspace.createFileSystemWatcher('**/*.st'),
          vscode.workspace.createFileSystemWatcher('**/stc.toml'),
        ],
      },
      outputChannel: this.output,
      revealOutputChannelOn: RevealOutputChannelOn.Never,
      initializationFailedHandler: (error: unknown) => {
        policy.noteInitializationFailure();
        this.log(`Language server initialization failed: ${message(error)}`);
        return false;
      },
      errorHandler,
    };
    this.log(`Starting stc lsp for ${manifest}`);
    const client = new LanguageClient(
      `stc-lsp:${manifest}`,
      'stc Language Server',
      serverOptions,
      clientOptions,
    );
    return { manifest, folder, client, policy };
  }
}

/** Finds `stc.toml` in `start` or one of its ancestors. */
export function findManifest(start: string): string | undefined {
  let directory = start;
  for (;;) {
    const candidate = path.join(directory, MANIFEST);
    try {
      if (statSync(candidate).isFile()) {
        return candidate;
      }
    } catch {
      // Keep looking in the parent directory.
    }
    const parent = path.dirname(directory);
    if (parent === directory) {
      return undefined;
    }
    directory = parent;
  }
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
