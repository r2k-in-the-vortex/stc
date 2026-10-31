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

/** Sends a signal to the child's process group, or to the child itself. */
function signalGroup(pid: number, signal: NodeJS.Signals): void {
  try {
    // The child was spawned with `detached: true`, so its pid is its process
    // group id; the negative pid reaches the launcher and host it started.
    process.kill(-pid, signal);
  } catch {
    try {
      process.kill(pid, signal);
    } catch {
      // It exited between the check and the signal.
    }
  }
}

/** Returns `true` while any process remains in the child's process group. */
function processGroupAlive(pid: number): boolean {
  try {
    process.kill(-pid, 0);
    return true;
  } catch (error) {
    // A permission error means the group still exists; only `ESRCH` proves
    // that every member is gone.
    return (error as NodeJS.ErrnoException).code !== 'ESRCH';
  }
}

/** Waits up to `timeoutMs` for every process in the group to exit. */
async function waitForGroupExit(pid: number, timeoutMs: number): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (processGroupAlive(pid)) {
    const remaining = deadline - Date.now();
    if (remaining <= 0) {
      return false;
    }
    await new Promise((resolve) => setTimeout(resolve, Math.min(50, remaining)));
  }
  return true;
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
 * `SIGINT` to the group, gives every member `graceMs` to stop normally, and
 * only then escalates to `SIGKILL`. Callers must mark the stop as forced: a
 * forced stop bypasses the host's normal shutdown and retained state may not
 * have been saved.
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
  // Signal the whole group so the launcher receives the same stop request as
  // the driver. Plain `stc run` has no stop handler and exits on SIGINT while
  // the host it started handles the request; returning as soon as the driver
  // exits would orphan that host.
  signalGroup(pid, 'SIGINT');
  if (await waitForGroupExit(pid, graceMs)) {
    return;
  }
  signalGroup(pid, 'SIGKILL');
  await waitForExit(child, 2_000);
  await waitForGroupExit(pid, 2_000);
}
