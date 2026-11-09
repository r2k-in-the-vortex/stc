import { statSync } from 'node:fs';
import { stat } from 'node:fs/promises';
import * as path from 'node:path';
import * as vscode from 'vscode';

import { createStcRunTerminal } from './runTerminal';
import {
  expandTaskVariables,
  isStcCommand,
  planTaskExecution,
  STC_COMMANDS,
  type StcTaskOptions,
  type TaskVariables,
} from './taskArgs';

export type { StcCommand } from './taskArgs';

/** The `stc` task definition contributed in `package.json`. */
export interface StcTaskDefinition extends vscode.TaskDefinition, StcTaskOptions {}

/** A project root and the workspace folder that owns it, when there is one. */
export interface ProjectRoot {
  readonly folder: vscode.WorkspaceFolder | undefined;
  readonly root: string | undefined;
}

/** Resolves `stc.path`; an empty setting falls back to `stc` on `PATH`. */
export function stcExecutable(): string {
  const configured = vscode.workspace.getConfiguration('stc').get<string>('path');
  const executable = configured?.trim();
  return executable ? executable : 'stc';
}

/**
 * Provides the `stc` task type.
 *
 * VS Code 1.82 types the provider as `TaskProvider<T extends Task>`, so the
 * generic is the concrete task type while `StcTaskDefinition` types the
 * definition payload.
 */
export class StcTaskProvider implements vscode.TaskProvider<vscode.Task> {
  public constructor(private readonly output: vscode.OutputChannel) {}

  public async provideTasks(_token: vscode.CancellationToken): Promise<vscode.Task[]> {
    const roots = await findProjectRoots();
    this.output.appendLine(
      `task provider: roots=${JSON.stringify(roots.map((root) => root.root))}`,
    );
    const tasks: vscode.Task[] = [];
    for (const project of roots.length > 0 ? roots : [{ folder: undefined, root: undefined }]) {
      for (const command of STC_COMMANDS) {
        tasks.push(
          this.createTask({ type: 'stc', command }, project.root, project.folder),
        );
      }
    }
    this.output.appendLine(
      `task provider: ${tasks.length} tasks (${tasks.map((task) => task.name).join(', ')})`,
    );
    return tasks;
  }

  public resolveTask(
    task: vscode.Task,
    _token: vscode.CancellationToken,
  ): vscode.Task | undefined {
    const definition = task.definition as StcTaskDefinition;
    this.output.appendLine(
      `task provider: resolve ${task.name} project=${String(definition.project)}`,
    );
    if (!isStcCommand(definition.command)) {
      return undefined;
    }
    const folder = isWorkspaceFolder(task.scope)
      ? task.scope
      : vscode.workspace.workspaceFolders?.[0];
    const root = resolveProjectRoot(definition, folder) ?? folder?.uri.fsPath;
    return this.createTask(definition, root, folder);
  }

  private createTask(
    definition: StcTaskDefinition,
    root: string | undefined,
    folder: vscode.WorkspaceFolder | undefined,
  ): vscode.Task {
    const command = definition.command;
    const variables = taskVariables(folder);
    const plan = planTaskExecution(definition, variables);
    let execution: vscode.ShellExecution | vscode.CustomExecution;
    if (plan.kind === 'runTerminal') {
      // VS Code only terminates a shell task's shell. Owning the `stc run`
      // process in a pseudoterminal lets `close()` stop the driver and the
      // launcher it started. The callback receives the definition with VS
      // Code's variables resolved, so the arguments match what a
      // `ShellExecution` would have produced.
      execution = new vscode.CustomExecution(async (resolvedDefinition) => {
        const resolved = resolvedDefinition as StcTaskDefinition;
        return createStcRunTerminal({
          executable: stcExecutable(),
          args: planTaskExecution(resolved, taskVariables(folder)).args,
          cwd: resolveProjectRoot(resolved, folder) ?? root,
        });
      });
    } else {
      const options: vscode.ShellExecutionOptions = {};
      if (root !== undefined) {
        options.cwd = root;
      }
      execution = new vscode.ShellExecution(stcExecutable(), plan.args, options);
    }
    const scope = folder ?? vscode.TaskScope.Workspace;
    const task = new vscode.Task(
      definition,
      scope,
      `stc ${command}`,
      'stc',
      execution,
      ['$stc'],
    );
    if (command === 'build') {
      task.group = vscode.TaskGroup.Build;
    } else if (command === 'test') {
      task.group = vscode.TaskGroup.Test;
    }
    return task;
  }
}

/** Finds `stc.toml` project roots in the workspace. */
export async function findProjectRoots(): Promise<ProjectRoot[]> {
  const folders = vscode.workspace.workspaceFolders ?? [];
  const roots: ProjectRoot[] = [];
  for (const folder of folders) {
    if (await isFile(path.join(folder.uri.fsPath, 'stc.toml'))) {
      roots.push({ folder, root: folder.uri.fsPath });
    }
  }
  if (roots.length > 0) {
    return roots;
  }
  if (folders.length > 0) {
    const found = await vscode.workspace.findFiles('**/stc.toml', '**/node_modules/**', 1);
    const manifest = found[0]?.fsPath;
    if (manifest !== undefined) {
      const root = path.dirname(manifest);
      const folder = folders.find((candidate) => isInside(root, candidate.uri.fsPath));
      return [{ folder, root }];
    }
  }
  return folders.map((folder) => ({ folder, root: folder.uri.fsPath }));
}

/** Resolves a task's `project` option to the manifest directory. */
function resolveProjectRoot(
  definition: StcTaskDefinition,
  folder: vscode.WorkspaceFolder | undefined,
): string | undefined {
  if (definition.project === undefined || definition.project.trim() === '') {
    return undefined;
  }
  let candidate = expandTaskVariables(definition.project, taskVariables(folder));
  if (!path.isAbsolute(candidate)) {
    const base = folder?.uri.fsPath;
    candidate = base === undefined ? path.resolve(candidate) : path.join(base, candidate);
  }
  try {
    return statSync(candidate).isDirectory() ? candidate : path.dirname(candidate);
  } catch {
    return path.basename(candidate).toLowerCase() === 'stc.toml'
      ? path.dirname(candidate)
      : candidate;
  }
}

/**
 * Returns the workspace values for task-variable expansion, preferring the
 * task's own folder and falling back to the first workspace folder.
 */
function taskVariables(folder: vscode.WorkspaceFolder | undefined): TaskVariables {
  const workspace = folder ?? vscode.workspace.workspaceFolders?.[0];
  return {
    workspaceFolder: workspace?.uri.fsPath ?? '',
    workspaceFolderBasename: workspace?.name ?? '',
  };
}

async function isFile(candidate: string): Promise<boolean> {
  try {
    return (await stat(candidate)).isFile();
  } catch {
    return false;
  }
}

function isInside(candidate: string, folder: string): boolean {
  const relative = path.relative(folder, candidate);
  return relative === '' || (!relative.startsWith('..') && !path.isAbsolute(relative));
}

function isWorkspaceFolder(scope: vscode.Task['scope']): scope is vscode.WorkspaceFolder {
  return typeof scope === 'object' && scope !== null && 'uri' in scope;
}
