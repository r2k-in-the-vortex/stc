import { StringDecoder } from 'node:string_decoder';
import * as vscode from 'vscode';

import { resolveExecutable } from './executable';
import { killProcessTree } from './processTree';
import { spawnStc, type StcProcess } from './stcProcess';

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
  private child: StcProcess | undefined;
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
    let child: StcProcess;
    try {
      child = spawnStc(executable, this.options.args, this.options.cwd);
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

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
