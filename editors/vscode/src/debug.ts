import { spawn } from 'node:child_process';
import * as vscode from 'vscode';

import { resolveExecutable } from './executable';
import {
  expandWorkspaceFolder,
  parseRemoteStatus,
  remoteDeployChoices,
  type RemoteStatusReport,
} from './remoteDeploy';
import { stcExecutable } from './tasks';

/**
 * Launches the local debug adapter (`stc dap`) for an `stc` debug session.
 *
 * The extension contains no debugger logic: it resolves the `stc` executable
 * from the `stc.path` setting (or `PATH`) and starts `stc dap` with the
 * workspace folder as its working directory. Launch/attach arguments are
 * translated by the adapter.
 */
export class StcDebugAdapterFactory implements vscode.DebugAdapterDescriptorFactory {
  private readonly output: vscode.OutputChannel;

  public constructor(output: vscode.OutputChannel) {
    this.output = output;
  }

  public createDebugAdapterDescriptor(
    session: vscode.DebugSession,
  ): vscode.ProviderResult<vscode.DebugAdapterDescriptor> {
    const configured = stcExecutable();
    const executable = resolveExecutable(configured);
    if (executable === undefined) {
      const message =
        `stc executable \`${configured}\` was not found; the debug session cannot start. ` +
        'Set `stc.path` or add stc to PATH.';
      this.output.appendLine(message);
      throw new Error(message);
    }
    const cwd =
      session.workspaceFolder?.uri.fsPath ??
      vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    this.output.appendLine(`Starting stc dap with ${executable}`);
    return new vscode.DebugAdapterExecutable(
      executable,
      ['dap'],
      cwd === undefined ? undefined : { cwd },
    );
  }
}

/**
 * Fills the launch/attach defaults the extension documents.
 *
 * `project` defaults to the workspace folder's `stc.toml`; a configuration
 * with no type, request, or name returns `undefined` so VS Code opens
 * `launch.json` instead of starting an empty session.
 */
export class StcDebugConfigurationProvider implements vscode.DebugConfigurationProvider {
  public constructor(private readonly output?: vscode.OutputChannel) {}

  public async resolveDebugConfiguration(
    folder: vscode.WorkspaceFolder | undefined,
    config: vscode.DebugConfiguration,
  ): Promise<vscode.DebugConfiguration | undefined> {
    if (!config.type && !config.request && !config.name) {
      return undefined;
    }
    config.type = config.type || 'stc';
    config.request = config.request || 'launch';
    config.name = config.name || 'stc: Launch';
    const workspaceFolder =
      folder ?? vscode.workspace.workspaceFolders?.[0];
    if (!config.project) {
      config.project = workspaceFolder
        ? `${workspaceFolder.uri.fsPath}/stc.toml`
        : '${workspaceFolder}/stc.toml';
    } else if (typeof config.project === 'string' && workspaceFolder) {
      // The pre-launch status query runs before VS Code substitutes
      // `${workspaceFolder}` in the configuration.
      config.project = expandWorkspaceFolder(
        config.project,
        workspaceFolder.uri.fsPath,
      );
    }
    this.log(
      `debug configuration ${String(config.request)}${config.remote ? ` remote=${String(config.remote)}` : ''}`,
    );
    if (config.request === 'attach' && typeof config.remote === 'string') {
      const proceed = await resolveRemoteDeploy(
        config,
        workspaceFolder,
        (message) => this.log(message),
      );
      this.log(`remote deploy choice: ${String(config.remoteDeploy)}`);
      if (!proceed) {
        return undefined;
      }
    }
    return config;
  }

  private log(message: string): void {
    this.output?.appendLine(`[debug] ${message}`);
  }
}

/**
 * Offers the attach-time deployment choice when the remote sources drifted.
 *
 * `stc remote status` syncs and builds a remote candidate without changing the
 * running host. A drift report becomes a QuickPick; the chosen value is passed
 * to the adapter as `remoteDeploy`, and the Rust driver performs the deploy.
 * Cancel aborts the session before it starts.
 */
async function resolveRemoteDeploy(
  config: vscode.DebugConfiguration,
  folder: vscode.WorkspaceFolder | undefined,
  log: (message: string) => void,
): Promise<boolean> {
  const configured = config.remoteDeploy;
  if (configured !== undefined && configured !== 'ask') {
    return true;
  }
  const executable = resolveExecutable(stcExecutable());
  if (executable === undefined) {
    log('remote status skipped: stc executable was not found');
    return true;
  }
  const { status, detail } = await runRemoteStatus(
    executable,
    remoteStatusArguments(config),
    folder?.uri.fsPath,
  );
  if (status === undefined) {
    log(`remote status failed: ${detail}`);
    return true;
  }
  log(
    `remote status: drift=${String(status.drift)} online=${String(status.online)}`,
  );
  if (!status.drift) {
    config.remoteDeploy = 'never';
    return true;
  }
  const choice = await vscode.window.showQuickPick(
    remoteDeployChoices(status),
    {
      title: 'Remote sources differ',
      placeHolder: status.online
        ? 'Choose how to deploy the local changes'
        : 'Online change is not compatible; cold download is required',
    },
  );
  if (choice === undefined || choice.choice === 'cancel') {
    return false;
  }
  config.remoteDeploy = choice.choice;
  return true;
}

function remoteStatusArguments(config: vscode.DebugConfiguration): string[] {
  const args = [
    'remote',
    'status',
    '--remote',
    config.remote as string,
    '--message-format',
    'json',
  ];
  for (const [option, key] of [
    ['--manifest-path', 'project'],
    ['--remote-dir', 'remoteDir'],
    ['--remote-stc', 'remoteStc'],
    ['--remote-session', 'remoteSession'],
    ['--remote-deploy', 'remoteDeploy'],
  ] as const) {
    if (typeof config[key] === 'string' && config[key].length > 0) {
      args.push(option, config[key] as string);
    }
  }
  return args;
}

interface RemoteStatusRun {
  status: RemoteStatusReport | undefined;
  detail: string;
}

function runRemoteStatus(
  executable: string,
  args: readonly string[],
  cwd: string | undefined,
): Promise<RemoteStatusRun> {
  return new Promise((resolve) => {
    let stderr = '';
    const child = (() => {
      try {
        return spawn(executable, [...args], {
          cwd,
          windowsHide: true,
          stdio: ['ignore', 'pipe', 'pipe'],
        });
      } catch (error) {
        resolve({
          status: undefined,
          detail: `cannot start ${executable}: ${String(error)}`,
        });
        return undefined;
      }
    })();
    if (child === undefined) {
      return;
    }
    let stdout = '';
    child.stdout.on('data', (chunk: Buffer) => {
      stdout += chunk.toString();
    });
    child.stderr.on('data', (chunk: Buffer) => {
      stderr += chunk.toString();
    });
    child.on('error', (error) => {
      resolve({
        status: undefined,
        detail: `cannot start ${executable}: ${String(error)}`,
      });
    });
    child.on('close', (code) => {
      const status = parseRemoteStatus(stdout);
      resolve({
        status,
        detail:
          status === undefined
            ? `exit ${String(code)}; stdout=${stdout.trim()}; stderr=${stderr.trim()}`
            : `exit ${String(code)}`,
      });
    });
  });
}
