import * as vscode from 'vscode';

import { resolveExecutable } from './languageClient';
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
  public resolveDebugConfiguration(
    folder: vscode.WorkspaceFolder | undefined,
    config: vscode.DebugConfiguration,
  ): vscode.ProviderResult<vscode.DebugConfiguration> {
    if (!config.type && !config.request && !config.name) {
      return undefined;
    }
    config.type = config.type || 'stc';
    config.request = config.request || 'launch';
    config.name = config.name || 'stc: Launch';
    if (!config.project) {
      config.project = folder
        ? `${folder.uri.fsPath}/stc.toml`
        : '${workspaceFolder}/stc.toml';
    }
    return config;
  }
}
