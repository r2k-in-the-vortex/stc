/**
 * Restart policy for one `stc lsp` language client.
 *
 * The language client reports two independent signals: protocol errors on the
 * connection (`connectionError`) and a closed connection (`closed`). A close
 * that was not requested is treated as a crash and restarted, up to a bounded
 * number of times inside a time window (the language client's default policy).
 *
 * Initialization failures are different. When the project cannot load (any
 * `STC-E-PROJECT-*` manifest error) retrying the same manifest cannot succeed
 * until the user edits `stc.toml`, so an automatic restart only produces a
 * storm of identical failures. The extension records the failure here and
 * surfaces the manifest error itself; the connection close that follows from
 * tearing the failed client down is then not mistaken for a crash.
 *
 * This module is intentionally free of `vscode` and `vscode-languageclient`
 * imports so it can be unit-tested with `node --test`.
 */

/** What to do when the server connection closed unexpectedly. */
export type CloseDecision =
  | { readonly action: 'restart'; readonly message?: string }
  | {
      readonly action: 'stop';
      readonly message?: string;
      /** The extension has already reported the cause; do not notify again. */
      readonly handled?: boolean;
    };

/** What to do about a protocol error on the server connection. */
export type ConnectionErrorDecision =
  | { readonly action: 'continue' }
  | { readonly action: 'shutdown'; readonly message?: string };

/** Policy tuning; the defaults mirror `vscode-languageclient`'s default handler. */
export interface RestartPolicyOptions {
  /** Automatic restarts allowed inside `windowMs` before the client gives up. */
  readonly maxRestarts?: number;
  /** Window in milliseconds in which restarts are counted. */
  readonly windowMs?: number;
  /** Clock override for tests. */
  readonly now?: () => number;
}

export const DEFAULT_MAX_RESTARTS = 4;
export const DEFAULT_RESTART_WINDOW_MS = 3 * 60 * 1000;

/**
 * Tracks one client's failures and decides whether closing the connection
 * should restart the server or stop the client.
 */
export class RestartPolicy {
  private readonly maxRestarts: number;
  private readonly windowMs: number;
  private readonly now: () => number;
  private readonly restarts: number[] = [];
  private initializationFailed = false;

  public constructor(options: RestartPolicyOptions = {}) {
    this.maxRestarts = options.maxRestarts ?? DEFAULT_MAX_RESTARTS;
    this.windowMs = options.windowMs ?? DEFAULT_RESTART_WINDOW_MS;
    this.now = options.now ?? Date.now;
  }

  /**
   * Records that `initialize` failed because the project could not load.
   *
   * The next connection close is the extension tearing down the failed client,
   * not a crash, so it must not restart the server.
   */
  public noteInitializationFailure(): void {
    this.initializationFailed = true;
  }

  /** Returns whether an initialization failure has been recorded. */
  public get hasFailedInitialization(): boolean {
    return this.initializationFailed;
  }

  /** Chooses the action for a closed connection. */
  public closed(): CloseDecision {
    if (this.initializationFailed) {
      return { action: 'stop', handled: true };
    }
    const now = this.now();
    this.restarts.push(now);
    if (this.restarts.length <= this.maxRestarts) {
      return { action: 'restart' };
    }
    const oldest = this.restarts[0];
    if (now - oldest <= this.windowMs) {
      const minutes = Math.round(this.windowMs / 60_000);
      return {
        action: 'stop',
        message:
          `The stc Language Server crashed ${this.maxRestarts + 1} times in the last ` +
          `${minutes} minutes. The server will not be restarted. ` +
          'See the stc Language Server output for more information.',
      };
    }
    this.restarts.shift();
    return { action: 'restart' };
  }

  /** Chooses the action for a read/write error on the connection. */
  public connectionError(count: number | undefined): ConnectionErrorDecision {
    if (count !== undefined && count <= 3) {
      return { action: 'continue' };
    }
    return { action: 'shutdown' };
  }
}
