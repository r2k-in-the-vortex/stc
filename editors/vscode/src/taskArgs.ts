/**
 * Pure task-argument construction shared by the VS Code task wiring.
 *
 * This module must not import `vscode`, so `test/tasks.test.mjs` can bundle it
 * and exercise the same code the extension ships.
 */

/** The commands the extension offers as tasks. */
export type StcCommand = 'check' | 'build' | 'run' | 'test' | 'fmt';

/** The commands in task-picker order. */
export const STC_COMMANDS: readonly StcCommand[] = ['check', 'build', 'run', 'test', 'fmt'];

/** The extension-defined `stc` task properties. */
export interface StcTaskOptions {
  /** The stc command to run. */
  readonly command: StcCommand;
  /** Optional path to `stc.toml`, its directory, or a project directory. */
  readonly project?: string;
  /** Additional arguments appended after the generated ones. */
  readonly args?: readonly string[];
  /** Pass `--release` for `build` and `run`. */
  readonly release?: boolean;
}

/** The workspace values used to expand task variables. */
export interface TaskVariables {
  readonly workspaceFolder: string;
  readonly workspaceFolderBasename: string;
}

/** Project commands that report source diagnostics. */
const DIAGNOSTIC_COMMANDS: ReadonlySet<StcCommand> = new Set(['check', 'build', 'run', 'test']);

/** Commands that accept `--release`. */
const RELEASE_COMMANDS: ReadonlySet<StcCommand> = new Set(['build', 'run']);

/** The two task execution paths. */
export type TaskExecutionKind = 'shell' | 'runTerminal';

/** A task's execution path and the argument list both paths receive. */
export interface TaskExecutionPlan {
  readonly kind: TaskExecutionKind;
  readonly args: string[];
}

/** Returns true for a command the extension contributes. */
export function isStcCommand(value: unknown): value is StcCommand {
  return STC_COMMANDS.includes(value as StcCommand);
}

/**
 * Expands the VS Code variables users expect in the custom task properties.
 *
 * VS Code expands variables in standard task fields such as `cwd`, but not in
 * extension-defined properties such as `project` or `args`; a literal
 * `${workspaceFolder}/stc.toml` would otherwise be treated as a relative path
 * and produce a doubled working directory.
 *
 * The replacement is a single pass, so an already-resolved value is unchanged
 * and a replacement value is never re-expanded. Any other `${...}` token is
 * left literal; no diagnostic is reported for it.
 */
export function expandTaskVariables(value: string, variables: TaskVariables): string {
  return value.replace(/\$\{([^}]+)\}/g, (token, name: string) => {
    switch (name) {
      case 'workspaceFolder':
      case 'workspaceRoot':
        return variables.workspaceFolder;
      case 'workspaceFolderBasename':
        return variables.workspaceFolderBasename;
      default:
        return token;
    }
  });
}

/** Builds the generated argument list for a stc task. */
export function commandArguments(
  definition: StcTaskOptions,
  variables: TaskVariables,
): string[] {
  const command = definition.command;
  const args: string[] = [command, '--message-format', 'short', '--color', 'never'];
  if (DIAGNOSTIC_COMMANDS.has(command)) {
    args.push('--diagnostic-paths', 'absolute');
  }
  if (definition.release === true && RELEASE_COMMANDS.has(command)) {
    args.push('--release');
  }
  if (definition.args !== undefined) {
    for (const argument of definition.args) {
      args.push(expandTaskVariables(argument, variables));
    }
  }
  return args;
}

/** Chooses the execution path and the shared argument list for a task. */
export function planTaskExecution(
  definition: StcTaskOptions,
  variables: TaskVariables,
): TaskExecutionPlan {
  return {
    kind: definition.command === 'run' ? 'runTerminal' : 'shell',
    args: commandArguments(definition, variables),
  };
}
