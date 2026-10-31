import { spawn, type ChildProcess } from 'node:child_process';

/**
 * Waits up to `timeoutMs` for `child` to exit.
 *
 * Returns `true` when the child has exited (or was already gone) and `false`
 * when the deadline passed first.
 */
export function waitForExit(child: ChildProcess, timeoutMs: number): Promise<boolean> {
  if (child.exitCode !== null || child.signalCode !== null) {
    return Promise.resolve(true);
  }
  return new Promise((resolve) => {
    let timer: NodeJS.Timeout | undefined;
    const onExit = (): void => {
      if (timer !== undefined) {
        clearTimeout(timer);
      }
      resolve(true);
    };
    child.once('exit', onExit);
    timer = setTimeout(() => {
      child.removeListener('exit', onExit);
      resolve(false);
    }, timeoutMs);
  });
}

function runTaskkill(pid: number): Promise<void> {
  return new Promise((resolve) => {
    let settled = false;
    const finish = (): void => {
      if (!settled) {
        settled = true;
        clearTimeout(timer);
        resolve();
      }
    };
    const killer = spawn('taskkill', ['/pid', String(pid), '/t', '/f'], {
      stdio: 'ignore',
      windowsHide: true,
    });
    const timer = setTimeout(() => {
      killer.kill();
      finish();
    }, 10_000);
    killer.once('error', finish);
    killer.once('close', finish);
  });
}

/**
 * Kills a child process and its descendants after a graceful stop failed.
 *
 * On Windows this uses `taskkill /T /F`; elsewhere it signals the process
 * group created by spawning the child with `detached: true`. It first sends
 * `SIGINT`, gives the child `graceMs` to stop normally, and only then escalates
 * to `SIGKILL`. Callers must mark the stop as forced: a forced stop bypasses
 * the host's normal shutdown and retained state may not have been saved.
 */
export async function killProcessTree(
  child: ChildProcess,
  graceMs = 2_000,
): Promise<void> {
  const pid = child.pid;
  if (pid === undefined) {
    return;
  }
  if (process.platform === 'win32') {
    await runTaskkill(pid);
    await waitForExit(child, 2_000);
    return;
  }
  try {
    process.kill(pid, 'SIGINT');
  } catch {
    // The child may already be gone.
  }
  if (await waitForExit(child, graceMs)) {
    return;
  }
  // The child was spawned with `detached: true`, so its pid is its process
  // group id; the negative pid reaches the launcher and host it started.
  try {
    process.kill(-pid, 'SIGKILL');
  } catch {
    try {
      process.kill(pid, 'SIGKILL');
    } catch {
      // It exited between the wait and the signal.
    }
  }
  await waitForExit(child, 2_000);
}
