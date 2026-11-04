/**
 * Live `stc` debug-session tracking for the online-change start gate.
 *
 * `vscode.debug.activeDebugSession` cannot be used as a synchronous liveness
 * check around `onDidTerminateDebugSession`: the extension host delivers the
 * termination event from `$acceptDebugSessionTerminated`, but updates the
 * active session from a separate `$acceptDebugSessionActiveChanged` message.
 * A caller that waits for the termination event (the smoke does, and a user
 * can stop a session and immediately start an online change) can still observe
 * the stopped session as active, which makes the start dialog appear
 * spuriously.
 *
 * This module is pure and imports no `vscode` types so the unit tests can
 * exercise the exact tracking the extension ships.
 */

/** The fields of `vscode.DebugSession` this tracker needs. */
export interface DebugSessionIdentity {
  readonly id: string;
  readonly type: string;
}

/** Tracks the `stc` debug sessions that have started and not yet terminated. */
export class StcDebugSessionTracker {
  private readonly active = new Set<string>();

  /**
   * Seeds the tracker from the active session observed at activation, which
   * covers a session that started before this extension subscribed.
   */
  public constructor(activeSession?: DebugSessionIdentity) {
    if (activeSession !== undefined && activeSession.type === 'stc') {
      this.active.add(activeSession.id);
    }
  }

  /** Records a debug session start; other debugger types are ignored. */
  public start(session: DebugSessionIdentity): void {
    if (session.type === 'stc') {
      this.active.add(session.id);
    }
  }

  /** Records a debug session end. */
  public end(session: DebugSessionIdentity): void {
    this.active.delete(session.id);
  }

  /** Whether at least one `stc` debug session is live. */
  public get hasActiveSession(): boolean {
    return this.active.size > 0;
  }
}
