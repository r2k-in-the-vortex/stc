import * as vscode from 'vscode';

import { StcDebugAdapterFactory, StcDebugConfigurationProvider } from './debug';
import { StcLanguageClient } from './languageClient';
import { StcTaskProvider, stcExecutable } from './tasks';

let output: vscode.OutputChannel | undefined;
let languageClient: StcLanguageClient | undefined;

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

/** Stops the language clients and releases the output channels. */
export function deactivate(): Thenable<void> | undefined {
  const stopping = languageClient?.stop();
  languageClient = undefined;
  output = undefined;
  return stopping;
}
