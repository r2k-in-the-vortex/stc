import { statSync } from 'node:fs';
import * as path from 'node:path';

/** Resolves `stc` from an explicit path or from PATH (with PATHEXT on Windows). */
export function resolveExecutable(command: string): string | undefined {
  const trimmed = command.trim();
  if (trimmed === '') {
    return undefined;
  }
  if (path.isAbsolute(trimmed) || trimmed.includes('/') || trimmed.includes('\\')) {
    try {
      return statSync(trimmed).isFile() ? trimmed : undefined;
    } catch {
      return undefined;
    }
  }
  const extensions =
    process.platform === 'win32'
      ? (process.env.PATHEXT ?? '.EXE;.CMD;.BAT;.COM')
          .split(';')
          .filter((extension) => extension !== '')
      : [''];
  for (const directory of (process.env.PATH ?? '').split(path.delimiter)) {
    if (directory === '') {
      continue;
    }
    for (const extension of extensions) {
      const candidate = path.join(directory, trimmed + extension);
      try {
        if (statSync(candidate).isFile()) {
          return candidate;
        }
      } catch {
        // Not in this directory.
      }
    }
  }
  return undefined;
}
