import * as vscode from 'vscode';

import { StcTaskProvider, stcExecutable } from './tasks';

let output: vscode.OutputChannel | undefined;

/** Activates the local ST support shell: task provider only. */
export function activate(context: vscode.ExtensionContext): void {
  output = vscode.window.createOutputChannel('stc');
  context.subscriptions.push(output);
  output.appendLine(`stc extension ${context.extension.packageJSON.version as string} activated`);
  output.appendLine(`stc executable: ${stcExecutable()}`);

  context.subscriptions.push(
    vscode.tasks.registerTaskProvider('stc', new StcTaskProvider()),
  );
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration('stc.path')) {
        output?.appendLine(`stc executable: ${stcExecutable()}`);
      }
    }),
  );
}

/** Deactivates the extension. */
export function deactivate(): void {
  output = undefined;
}
