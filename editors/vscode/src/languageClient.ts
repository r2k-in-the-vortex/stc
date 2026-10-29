import { statSync } from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  RevealOutputChannelOn,
  ServerOptions,
} from 'vscode-languageclient/node';

import { stcExecutable } from './tasks';

const MANIFEST = 'stc.toml';

/**
 * Hosts one `stc lsp` process per workspace folder that contains a project.
 *
 * The extension is only the client: diagnostics, navigation, hover,
 * completion, and formatting all come from the compiler through the language
 * server. One server instance serves one project (the nearest `stc.toml`).
 */
export class StcLanguageClient implements vscode.Disposable {
  private readonly output: vscode.OutputChannel;
  private clients: LanguageClient[] = [];
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
    for (const [manifest, folder] of projects) {
      this.clients.push(this.createClient(executable, manifest, folder));
    }
    for (const client of this.clients) {
      try {
        await client.start();
      } catch (error) {
        this.log(`Failed to start the language server: ${message(error)}`);
      }
    }
  }

  private async stopNow(): Promise<void> {
    const clients = this.clients;
    this.clients = [];
    for (const client of clients) {
      try {
        await client.stop();
      } catch (error) {
        this.log(`Failed to stop the language server: ${message(error)}`);
      }
      try {
        client.dispose();
      } catch {
        // The client was already stopped.
      }
    }
  }

  private createClient(
    executable: string,
    manifest: string,
    folder: vscode.WorkspaceFolder,
  ): LanguageClient {
    const serverOptions: ServerOptions = {
      command: executable,
      args: ['lsp', '--manifest-path', manifest],
      options: { cwd: path.dirname(manifest) },
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
    };
    this.log(`Starting stc lsp for ${manifest}`);
    return new LanguageClient(
      `stc-lsp:${manifest}`,
      'stc Language Server',
      serverOptions,
      clientOptions,
    );
  }
}

/** Finds `stc.toml` in `start` or one of its ancestors. */
function findManifest(start: string): string | undefined {
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

/** Resolves `stc` from an explicit path or from PATH (with PATHEXT on Windows). */
export function resolveExecutable(command: string): string | undefined {
  const trimmed = command.trim();
  if (trimmed === '') {
    return undefined;
  }
  if (path.isAbsolute(trimmed) || trimmed.includes('/') || trimmed.includes('\\')) {
    try {
      return statSync(trimmed).isFile() ? trimmed : undefined;
    } catch {
      return undefined;
    }
  }
  const extensions =
    process.platform === 'win32'
      ? (process.env.PATHEXT ?? '.EXE;.CMD;.BAT;.COM')
          .split(';')
          .filter((extension) => extension !== '')
      : [''];
  for (const directory of (process.env.PATH ?? '').split(path.delimiter)) {
    if (directory === '') {
      continue;
    }
    for (const extension of extensions) {
      const candidate = path.join(directory, trimmed + extension);
      try {
        if (statSync(candidate).isFile()) {
          return candidate;
        }
      } catch {
        // Not in this directory.
      }
    }
  }
  return undefined;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
