import {
  spawn,
  type ChildProcessByStdio,
  type SpawnOptionsWithStdioTuple,
  type StdioPipe,
} from 'node:child_process';
import type { Readable, Writable } from 'node:stream';

/** An `stc` process with piped stdin, stdout, and stderr. */
export type StcProcess = ChildProcessByStdio<Writable, Readable, Readable>;

/**
 * Spawns `stc` with piped stdio.
 *
 * On POSIX the child is detached so its pid doubles as the process-group id
 * that `killProcessTree` signals. Batch shims cannot be started directly on
 * Windows, so they go through `cmd.exe`; the taskkill-based tree stop still
 * reaches their children.
 */
export function spawnStc(
  executable: string,
  args: readonly string[],
  cwd: string | undefined,
): StcProcess {
  const options: SpawnOptionsWithStdioTuple<StdioPipe, StdioPipe, StdioPipe> = {
    cwd,
    stdio: ['pipe', 'pipe', 'pipe'],
    windowsHide: true,
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
