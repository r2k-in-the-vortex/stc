/**
 * VS Code tracked-rename controller.
 *
 * The standard `textDocument/rename` method is history-free. When the server
 * advertises `experimental.stc.trackedRename`, this controller asks the private
 * `stc/rename` method for the same edit plus a single-use history token,
 * returns the edit to VS Code, confirms the post-apply text of every edited
 * file, and only then asks `stc/recordRename` to append `stc.renames.toml`.
 *
 * A cancel, undo, partial apply, or failed history write leaves the history
 * unchanged and warns with the recovery. Manifest and profile edits are also
 * announced through `workspace/didChangeWatchedFiles` so the language server
 * reloads the selected profile.
 */

import * as vscode from 'vscode';
import type {
  LanguageClient,
  ProvideRenameEditsSignature,
} from 'vscode-languageclient/node';

import {
  applyLspEdits,
  editedUriKind,
  shouldUseTrackedRename,
} from './renameEdits';
import type {
  EditedUriKind,
  LspTextEdit,
  LspWorkspaceEdit,
} from './renameEdits';

const TRACKED_RENAME = 'stc/rename';
const RECORD_RENAME = 'stc/recordRename';
const WATCHED_FILES = 'workspace/didChangeWatchedFiles';

/** Quiet period after a text change before the apply is verified. */
const VERIFY_DEBOUNCE_MS = 150;
/** How long an unconfirmed rename may stay pending. */
const VERIFY_DEADLINE_MS = 5_000;
/** JSON-RPC error code for an unsupported method. */
const METHOD_NOT_FOUND = -32601;
/** `FileChangeType.Changed`. */
const CHANGED = 2;

/** One edited file captured when the rename edit was returned to VS Code. */
interface TrackedRenameFile {
  readonly uri: vscode.Uri;
  readonly kind: EditedUriKind;
  readonly original: string;
  readonly expected: string;
}

/** One canonical history entry, used only in recovery warnings. */
interface RenameHistoryEntry {
  readonly old: string;
  readonly new: string;
}

/** A rename whose edit has been returned but not yet confirmed. */
interface PendingRename {
  readonly token: string | undefined;
  readonly history: readonly RenameHistoryEntry[];
  readonly files: readonly TrackedRenameFile[];
  timer: ReturnType<typeof setTimeout> | undefined;
}

/** The private `stc/rename` response. */
interface TrackedRenameResponse {
  readonly edit: LspWorkspaceEdit;
  readonly history?: readonly unknown[];
  readonly warnings?: readonly unknown[];
  readonly token?: unknown;
}

/** The private `stc/recordRename` response. */
interface RecordRenameResponse {
  readonly recorded?: unknown;
  readonly entries?: unknown;
  readonly path?: unknown;
}

/** The protocol workspace-edit type accepted by `protocol2CodeConverter`. */
type ProtocolWorkspaceEdit = NonNullable<
  Parameters<LanguageClient['protocol2CodeConverter']['asWorkspaceEdit']>[0]
>;

/**
 * Owns the tracked-rename handshake for one language client.
 *
 * The client is supplied after construction because the middleware closure
 * needs the controller before the `LanguageClient` itself exists.
 */
export class TrackedRenameController implements vscode.Disposable {
  private readonly manifestPath: string;
  private readonly output: vscode.OutputChannel;
  private readonly watcher: vscode.FileSystemWatcher;
  private readonly changeListener: vscode.Disposable;
  private readonly watcherListener: vscode.Disposable;
  private client: LanguageClient | undefined;
  private pending: PendingRename | undefined;
  private verifyTimer: ReturnType<typeof setTimeout> | undefined;
  private warnedFallback = false;
  private disposed = false;

  public constructor(
    manifestPath: string,
    watcher: vscode.FileSystemWatcher,
    output: vscode.OutputChannel,
  ) {
    this.manifestPath = manifestPath;
    this.watcher = watcher;
    this.output = output;
    this.changeListener = vscode.workspace.onDidChangeTextDocument(() =>
      this.scheduleVerification(),
    );
    this.watcherListener = watcher.onDidChange(() => this.scheduleVerification());
  }

  /** Supplies the client once it has been constructed. */
  public setClient(client: LanguageClient): void {
    this.client = client;
  }

  /**
   * Rename middleware: uses the tracked pair when the server supports it and
   * falls back to the standard history-free method once per session.
   */
  public async provideRenameEdits(
    document: vscode.TextDocument,
    position: vscode.Position,
    newName: string,
    token: vscode.CancellationToken,
    next: ProvideRenameEditsSignature,
  ): Promise<vscode.WorkspaceEdit | null | undefined> {
    // A pending rename must be recorded before the server discards its token.
    await this.finishPending();

    const client = this.client;
    if (this.disposed || client === undefined) {
      return next(document, position, newName, token);
    }
    if (!shouldUseTrackedRename(client.initializeResult)) {
      this.warnFallbackOnce(
        'the language server does not advertise the tracked rename capability',
      );
      return next(document, position, newName, token);
    }

    let response: TrackedRenameResponse;
    try {
      response = await client.sendRequest<TrackedRenameResponse>(
        TRACKED_RENAME,
        {
          textDocument: { uri: document.uri.toString() },
          position: { line: position.line, character: position.character },
          newName,
        },
        token,
      );
    } catch (error) {
      if (isMethodNotFound(error)) {
        this.warnFallbackOnce('the language server does not support `stc/rename`');
        return next(document, position, newName, token);
      }
      throw error;
    }

    if (token.isCancellationRequested) {
      return undefined;
    }
    const edit = trackedEdit(response);
    for (const warning of response.warnings ?? []) {
      if (typeof warning === 'string' && warning.length > 0) {
        this.log(`warning: ${warning}`);
        void vscode.window.showWarningMessage(warning);
      }
    }

    const files = await this.captureFiles(edit);
    const history = renameHistory(response.history);
    const recordToken =
      typeof response.token === 'string' && response.token.length > 0
        ? response.token
        : undefined;

    let converted: vscode.WorkspaceEdit | null | undefined;
    try {
      converted = await client.protocol2CodeConverter.asWorkspaceEdit(
        edit as unknown as ProtocolWorkspaceEdit,
        token,
      );
    } catch (error) {
      throw new Error(`stc: cannot convert the rename edit: ${message(error)}`);
    }
    if (converted === undefined || converted === null || token.isCancellationRequested) {
      return undefined;
    }

    if (files.length === 0) {
      this.log('the rename changed no files; nothing to record or refresh');
      return converted;
    }
    if (recordToken !== undefined && history.length === 0) {
      this.log('the server supplied a history token without history entries');
    }
    this.beginPending(recordToken, history, files);
    return converted;
  }

  /**
   * Confirms a rename that is still pending before a new rename starts.
   *
   * Commits the history when every file matches the expected text; clears
   * silently when the edit was cancelled or undone; otherwise warns with the
   * recovery and clears the record.
   */
  public async finishPending(): Promise<void> {
    if (this.pending === undefined) {
      return;
    }
    try {
      await this.verifyPending(true);
    } catch (error) {
      const pending = this.pending;
      this.clearPending();
      this.log(`failed to verify the previous rename: ${message(error)}`);
      if (pending !== undefined) {
        this.warnUnrecorded(
          pending,
          `the edited files could not be read (${message(error)})`,
        );
      }
    }
  }

  public dispose(): void {
    this.disposed = true;
    this.clearPending();
    this.changeListener.dispose();
    this.watcherListener.dispose();
    this.watcher.dispose();
  }

  /** Opens every edited file and stores the pre- and post-edit text. */
  private async captureFiles(
    edit: LspWorkspaceEdit,
  ): Promise<TrackedRenameFile[]> {
    const changes = edit.changes ?? {};
    const files: TrackedRenameFile[] = [];
    for (const [uri, edits] of Object.entries(changes)) {
      if (!Array.isArray(edits) || edits.length === 0) {
        continue;
      }
      const document = await this.openDocument(uri);
      const original = document.getText();
      let expected: string;
      try {
        expected = applyLspEdits(original, edits as readonly LspTextEdit[]);
      } catch (error) {
        throw new Error(
          `stc: cannot verify the rename edit for ${uri}: ${message(error)}`,
        );
      }
      files.push({
        uri: document.uri,
        kind: editedUriKind(uri, this.manifestPath),
        original,
        expected,
      });
    }
    return files;
  }

  /** Finds the open document for a URI or opens it from disk. */
  private async openDocument(uri: string): Promise<vscode.TextDocument> {
    const target = vscode.Uri.parse(uri);
    const open = vscode.workspace.textDocuments.find((document) =>
      sameUri(document.uri, target),
    );
    return open ?? (await vscode.workspace.openTextDocument(target));
  }

  private beginPending(
    token: string | undefined,
    history: readonly RenameHistoryEntry[],
    files: readonly TrackedRenameFile[],
  ): void {
    this.clearPending();
    const pending: PendingRename = {
      token,
      history,
      files,
      timer: undefined,
    };
    pending.timer = setTimeout(() => {
      if (this.pending !== pending) {
        return;
      }
      void this.verifyPending(true).catch((error: unknown) => {
        this.log(`failed to verify the rename at the deadline: ${message(error)}`);
      });
    }, VERIFY_DEADLINE_MS);
    this.pending = pending;
    this.log(
      `tracking rename of ${files.length} file(s)` +
        (token === undefined ? ' without history' : ' with history'),
    );
  }

  /** Debounces a verification pass after a text or file change. */
  private scheduleVerification(): void {
    if (this.disposed || this.pending === undefined) {
      return;
    }
    if (this.verifyTimer !== undefined) {
      clearTimeout(this.verifyTimer);
    }
    this.verifyTimer = setTimeout(() => {
      this.verifyTimer = undefined;
      void this.verifyPending(false).catch((error: unknown) => {
        this.log(`failed to verify the rename: ${message(error)}`);
      });
    }, VERIFY_DEBOUNCE_MS);
  }

  /**
   * Compares every edited file with its expected text.
   *
   * Success commits the history and refreshes manifest/profile snapshots. With
   * `final` set, a non-match either clears a cancelled/undone rename or warns
   * about a partial apply; otherwise the record stays pending.
   */
  private async verifyPending(final: boolean): Promise<void> {
    const pending = this.pending;
    if (pending === undefined) {
      return;
    }
    const states = await Promise.all(
      pending.files.map(async (file) => ({
        file,
        text: (await this.openDocument(file.uri.toString())).getText(),
      })),
    );
    // A new rename or a dispose replaced the record while documents opened.
    if (this.pending !== pending) {
      return;
    }
    if (states.every(({ file, text }) => text === file.expected)) {
      this.clearPending();
      await this.completePending(pending);
      return;
    }
    if (!final) {
      return;
    }
    this.clearPending();
    if (states.every(({ file, text }) => text === file.original)) {
      this.log('the rename was cancelled or undone; no history was written');
      return;
    }
    this.warnUnrecorded(pending, 'not every edited file matches the expected text');
  }

  /** Records confirmed history and refreshes edited manifest/profile files. */
  private async completePending(pending: PendingRename): Promise<void> {
    let failure: string | undefined;
    if (pending.token !== undefined) {
      const client = this.client;
      if (client === undefined) {
        failure = 'the language client is not available';
      } else {
        try {
          const result = await client.sendRequest<RecordRenameResponse>(
            RECORD_RENAME,
            { token: pending.token },
          );
          const entries =
            typeof result.entries === 'number'
              ? result.entries
              : pending.history.length;
          const path =
            typeof result.path === 'string' ? result.path : 'stc.renames.toml';
          this.log(
            `recorded ${entries} rename history ` +
              `${entries === 1 ? 'entry' : 'entries'} in ${path}`,
          );
        } catch (error) {
          failure = message(error);
        }
      }
    } else {
      this.log('the rename was verified; it has no history to record');
    }

    await this.refreshEditedFiles(pending.files);

    if (failure !== undefined) {
      this.log(`failed to record the rename history: ${failure}`);
      this.warnUnrecorded(pending, failure);
    }
  }

  /** Sends one change notification for edited manifest/profile files. */
  private async refreshEditedFiles(
    files: readonly TrackedRenameFile[],
  ): Promise<void> {
    const changed = files.filter((file) => file.kind !== 'source');
    if (changed.length === 0) {
      return;
    }
    const client = this.client;
    if (client === undefined) {
      return;
    }
    try {
      await client.sendNotification(WATCHED_FILES, {
        changes: changed.map((file) => ({
          uri: file.uri.toString(),
          type: CHANGED,
        })),
      });
      this.log(
        `refreshed ${changed.length} manifest/profile ` +
          `${changed.length === 1 ? 'file' : 'files'} after the rename`,
      );
    } catch (error) {
      this.log(`failed to refresh the renamed manifest/profile: ${message(error)}`);
    }
  }

  private clearPending(): void {
    if (this.pending?.timer !== undefined) {
      clearTimeout(this.pending.timer);
    }
    this.pending = undefined;
    if (this.verifyTimer !== undefined) {
      clearTimeout(this.verifyTimer);
      this.verifyTimer = undefined;
    }
  }

  /** Shows the unrecorded-rename warning with the documented recovery. */
  private warnUnrecorded(pending: PendingRename, reason: string): void {
    const entries =
      pending.history.length > 0
        ? pending.history
        : [{ old: '<old>', new: '<new>' }];
    const mappings = entries
      .map((entry) => `old = ${JSON.stringify(entry.old)}, new = ${JSON.stringify(entry.new)}`)
      .join('; ');
    const text =
      `stc: the rename was applied but not recorded in stc.renames.toml (${reason}). ` +
      `Revert the edit and run \`stc rename\`, or add the [[rename]] entry ` +
      `(${mappings}) to stc.renames.toml by hand before the next online build; ` +
      'otherwise the next online change may require a cold download.';
    this.log(text);
    void vscode.window.showWarningMessage(text);
  }

  private warnFallbackOnce(reason: string): void {
    if (this.warnedFallback) {
      return;
    }
    this.warnedFallback = true;
    const text =
      `stc: tracked rename is unavailable because ${reason}; ` +
      'this rename uses the standard history-free method and will not be recorded ' +
      'in stc.renames.toml. Update stc to keep online changes warm.';
    this.log(text);
    void vscode.window.showWarningMessage(text);
  }

  private log(message: string): void {
    this.output.appendLine(message);
  }
}

/** Validates the private response's edit field. */
function trackedEdit(response: TrackedRenameResponse): LspWorkspaceEdit {
  if (typeof response !== 'object' || response === null) {
    throw new Error('stc: the tracked rename response is not an object');
  }
  const edit = response.edit;
  if (typeof edit !== 'object' || edit === null) {
    throw new Error('stc: the tracked rename response has no edit');
  }
  return edit;
}

/** Copies only well-formed history entries from the private response. */
function renameHistory(history: readonly unknown[] | undefined): RenameHistoryEntry[] {
  const entries: RenameHistoryEntry[] = [];
  for (const entry of history ?? []) {
    if (typeof entry !== 'object' || entry === null) {
      continue;
    }
    const { old, new: next } = entry as Record<string, unknown>;
    if (typeof old === 'string' && typeof next === 'string') {
      entries.push({ old, new: next });
    }
  }
  return entries;
}

function isMethodNotFound(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    (error as { code?: unknown }).code === METHOD_NOT_FOUND
  );
}

function sameUri(left: vscode.Uri, right: vscode.Uri): boolean {
  if (left.toString() === right.toString()) {
    return true;
  }
  return process.platform === 'win32'
    ? left.fsPath.toLowerCase() === right.fsPath.toLowerCase()
    : left.fsPath === right.fsPath;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
