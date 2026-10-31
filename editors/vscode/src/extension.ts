import * as vscode from 'vscode';

import { StcDebugAdapterFactory, StcDebugConfigurationProvider } from './debug';
import { StcLanguageClient } from './languageClient';
import { OnlineChangeSession } from './onlineChange/session';
import { StcTaskProvider, stcExecutable } from './tasks';

let output: vscode.OutputChannel | undefined;
let languageClient: StcLanguageClient | undefined;
let onlineChange: OnlineChangeSession | undefined;

/** Activates local ST support: tasks, the `stc lsp` language client, and the `stc dap` debug adapter. */
export function activate(context: vscode.ExtensionContext): void {
  output = vscode.window.createOutputChannel('stc');
  context.subscriptions.push(output);
  output.appendLine(`stc extension ${context.extension.packageJSON.version as string} activated`);
  output.appendLine(`stc executable: ${stcExecutable()}`);

  context.subscriptions.push(
    vscode.tasks.registerTaskProvider('stc', new StcTaskProvider(output)),
  );

  context.subscriptions.push(
    vscode.debug.registerDebugAdapterDescriptorFactory(
      'stc',
      new StcDebugAdapterFactory(output),
    ),
    vscode.debug.registerDebugConfigurationProvider(
      'stc',
      new StcDebugConfigurationProvider(),
    ),
  );

  languageClient = new StcLanguageClient();
  context.subscriptions.push(languageClient);
  void languageClient.start().catch((error: unknown) => {
    output?.appendLine(`language server start failed: ${String(error)}`);
  });

  onlineChange = new OnlineChangeSession(context);
  context.subscriptions.push(onlineChange);

  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration('stc.path')) {
        output?.appendLine(`stc executable: ${stcExecutable()}`);
        void languageClient?.restart().catch((error: unknown) => {
          output?.appendLine(`language server restart failed: ${String(error)}`);
        });
      }
    }),
  );
}

/** Stops the language client and any online-change host, and releases the output channels. */
export function deactivate(): Thenable<void> | undefined {
  const stopping: Array<Promise<void> | Thenable<void>> = [];
  const languageStopping = languageClient?.stop();
  if (languageStopping !== undefined) {
    stopping.push(languageStopping);
  }
  const onlineStopping = onlineChange?.stopForDeactivate();
  if (onlineStopping !== undefined) {
    stopping.push(onlineStopping);
  }
  languageClient = undefined;
  onlineChange = undefined;
  output = undefined;
  if (stopping.length === 0) {
    return undefined;
  }
  return Promise.all(stopping).then(() => undefined);
}
