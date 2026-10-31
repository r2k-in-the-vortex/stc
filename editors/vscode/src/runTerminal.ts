import {
  spawn,
  type ChildProcessByStdio,
  type SpawnOptionsWithStdioTuple,
  type StdioPipe,
} from 'node:child_process';
import type { Readable, Writable } from 'node:stream';
import { StringDecoder } from 'node:string_decoder';
import * as vscode from 'vscode';

import { resolveExecutable } from './executable';
import { killProcessTree } from './processTree';

/** A `stc run` process with piped stdin, stdout, and stderr. */
type RunProcess = ChildProcessByStdio<Writable, Readable, Readable>;

export interface StcRunTerminalOptions {
  /** The configured `stc.path` value, or `stc` on `PATH`. */
  readonly executable: string;
  /** The generated arguments, starting with `run`. */
  readonly args: readonly string[];
  /** The project root, when one was resolved. */
  readonly cwd: string | undefined;
}

/** Creates the pseudoterminal that runs a `stc run` task. */
export function createStcRunTerminal(options: StcRunTerminalOptions): vscode.Pseudoterminal {
  return new StcRunTerminal(options);
}

/**
 * Runs `stc run` inside a task terminal so the extension owns the process.
 *
 * A `ShellExecution` task only lets VS Code terminate the shell; the shell's
 * children (`stc run` and the host launcher) survive. Here the extension
 * spawns `stc` itself and stops the whole tree when the terminal closes.
 */
class StcRunTerminal implements vscode.Pseudoterminal {
  private readonly writeEmitter = new vscode.EventEmitter<string>();
  private readonly closeEmitter = new vscode.EventEmitter<number | void>();
  private readonly stdoutDecoder = new StringDecoder('utf8');
  private readonly stderrDecoder = new StringDecoder('utf8');
  private child: RunProcess | undefined;
  private opened = false;
  private closing = false;
  private finished = false;

  public readonly onDidWrite = this.writeEmitter.event;
  public readonly onDidClose = this.closeEmitter.event;

  public constructor(private readonly options: StcRunTerminalOptions) {}

  public open(): void {
    if (this.opened || this.finished) {
      return;
    }
    this.opened = true;
    this.start();
  }

  public close(): void {
    if (this.finished || this.closing) {
      return;
    }
    this.closing = true;
    const child = this.child;
    if (child === undefined) {
      this.finish(undefined);
      return;
    }
    // The user terminated the task, so stop the driver and the host it
    // launched. The helper escalates after its grace period; the task ends
    // even if the child cannot be reaped.
    void killProcessTree(child).then(() => this.finish(undefined));
  }

  public handleInput(data: string): void {
    if (this.finished) {
      return;
    }
    const stdin = this.child?.stdin;
    if (stdin !== undefined && stdin.writable) {
      stdin.write(data);
    }
  }

  private start(): void {
    const configured = this.options.executable;
    const executable = resolveExecutable(configured);
    if (executable === undefined) {
      this.report(
        `the stc executable \`${configured}\` was not found; ` +
          'set `stc.path` or add stc to PATH.',
      );
      this.finish(1);
      return;
    }
    let child: RunProcess;
    try {
      child = spawnRun(executable, this.options.args, this.options.cwd);
    } catch (error) {
      this.report(`the run task could not start stc: ${message(error)}`);
      this.finish(1);
      return;
    }
    this.child = child;
    child.stdout.on('data', (chunk: Buffer) => this.write(this.stdoutDecoder.write(chunk)));
    child.stderr.on('data', (chunk: Buffer) => this.write(this.stderrDecoder.write(chunk)));
    child.once('error', (error) => {
      if (this.finished) {
        return;
      }
      this.report(`the run task failed: ${message(error)}`);
      if (child.pid === undefined) {
        this.finish(1);
        return;
      }
      void killProcessTree(child).then(() => this.finish(1));
    });
    child.once('close', (code, signal) => {
      this.write(this.stdoutDecoder.end());
      this.write(this.stderrDecoder.end());
      if (this.closing) {
        // A terminated task reports no exit code, matching VS Code's
        // `TaskProcessEndEvent.exitCode` contract.
        this.finish(undefined);
      } else {
        this.finish(code ?? (signal === null ? 0 : 1));
      }
    });
  }

  private write(text: string): void {
    if (this.finished || text === '') {
      return;
    }
    // A pseudoterminal needs CRLF line endings; stc writes bare LF.
    this.writeEmitter.fire(text.replace(/\r?\n/g, '\r\n'));
  }

  private report(message: string): void {
    this.write(`stc: ${message}\n`);
  }

  private finish(code: number | undefined): void {
    if (this.finished) {
      return;
    }
    this.finished = true;
    this.child = undefined;
    this.closeEmitter.fire(code);
    this.writeEmitter.dispose();
    this.closeEmitter.dispose();
  }
}

/**
 * Spawns `stc` with the task's arguments and working directory.
 *
 * Batch shims cannot be started directly on Windows, so they go through
 * `cmd.exe`; the taskkill-based tree stop still reaches their children.
 */
function spawnRun(
  executable: string,
  args: readonly string[],
  cwd: string | undefined,
): RunProcess {
  const options: SpawnOptionsWithStdioTuple<StdioPipe, StdioPipe, StdioPipe> = {
    cwd,
    stdio: ['pipe', 'pipe', 'pipe'],
    windowsHide: true,
    // On POSIX the pid doubles as the process-group id that killProcessTree
    // signals when the graceful stop does not finish in time.
    detached: process.platform !== 'win32',
  };
  if (process.platform === 'win32' && /\.(?:cmd|bat)$/i.test(executable)) {
    const commandLine = [executable, ...args].map(quoteCmdArgument).join(' ');
    // The outer quotes are for `cmd.exe /s /c`; `/s` strips exactly that
    // wrapper and leaves the per-argument quoting intact.
    return spawn(process.env.ComSpec ?? 'cmd.exe', ['/d', '/s', '/c', `"${commandLine}"`], {
      ...options,
      windowsVerbatimArguments: true,
    });
  }
  return spawn(executable, [...args], options);
}

/**
 * Quotes one argument for a `cmd.exe /s /c` command line.
 *
 * The escaping follows the `CommandLineToArgvW` rules for a quoted argument:
 * backslashes are doubled where they would escape a quote or the closing
 * quote. `cmd.exe` itself still expands `%NAME%` inside the quotes, which is
 * inherent to running a batch shim.
 */
function quoteCmdArgument(value: string): string {
  let result = '"';
  let backslashes = 0;
  for (const character of value) {
    if (character === '\\') {
      backslashes += 1;
      continue;
    }
    if (character === '"') {
      result += `${'\\'.repeat(backslashes * 2 + 1)}"`;
    } else {
      result += `${'\\'.repeat(backslashes)}${character}`;
    }
    backslashes = 0;
  }
  result += `${'\\'.repeat(backslashes * 2)}"`;
  return result;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
