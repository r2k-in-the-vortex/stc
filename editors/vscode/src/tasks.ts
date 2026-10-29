import { statSync } from 'node:fs';
import { stat } from 'node:fs/promises';
import * as path from 'node:path';
import * as vscode from 'vscode';

/** The commands the extension offers as tasks. */
export type StcCommand = 'check' | 'build' | 'run' | 'test' | 'fmt';

/** The `stc` task definition contributed in `package.json`. */
export interface StcTaskDefinition extends vscode.TaskDefinition {
  /** The stc command to run. */
  command: StcCommand;
  /** Optional path to `stc.toml`, its directory, or a project directory. */
  project?: string;
  /** Additional arguments appended after the generated ones. */
  args?: string[];
  /** Pass `--release` for `build` and `run`. */
  release?: boolean;
}

const COMMANDS: readonly StcCommand[] = ['check', 'build', 'run', 'test', 'fmt'];

/** Project commands that report source diagnostics. */
const DIAGNOSTIC_COMMANDS: ReadonlySet<StcCommand> = new Set(['check', 'build', 'run', 'test']);

/** Commands that accept `--release`. */
const RELEASE_COMMANDS: ReadonlySet<StcCommand> = new Set(['build', 'run']);

/** A project root and the workspace folder that owns it, when there is one. */
interface ProjectRoot {
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
  public async provideTasks(_token: vscode.CancellationToken): Promise<vscode.Task[]> {
    const roots = await findProjectRoots();
    const tasks: vscode.Task[] = [];
    for (const project of roots.length > 0 ? roots : [{ folder: undefined, root: undefined }]) {
      for (const command of COMMANDS) {
        tasks.push(
          this.createTask({ type: 'stc', command }, project.root, project.folder),
        );
      }
    }
    return tasks;
  }

  public resolveTask(
    task: vscode.Task,
    _token: vscode.CancellationToken,
  ): vscode.Task | undefined {
    const definition = task.definition as StcTaskDefinition;
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
    const args: string[] = [command, '--message-format', 'short', '--color', 'never'];
    if (DIAGNOSTIC_COMMANDS.has(command)) {
      args.push('--diagnostic-paths', 'absolute');
    }
    if (definition.release === true && RELEASE_COMMANDS.has(command)) {
      args.push('--release');
    }
    if (definition.args !== undefined) {
      args.push(...definition.args);
    }
    const options: vscode.ShellExecutionOptions = {};
    if (root !== undefined) {
      options.cwd = root;
    }
    const execution = new vscode.ShellExecution(stcExecutable(), args, options);
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
async function findProjectRoots(): Promise<ProjectRoot[]> {
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
  let candidate = definition.project;
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

function isStcCommand(value: unknown): value is StcCommand {
  return COMMANDS.includes(value as StcCommand);
}

function isWorkspaceFolder(scope: vscode.Task['scope']): scope is vscode.WorkspaceFolder {
  return typeof scope === 'object' && scope !== null && 'uri' in scope;
}
