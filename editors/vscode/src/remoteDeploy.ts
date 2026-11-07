/**
 * Attach-time deployment choice for remote debug sessions (WL-1.2 F-21).
 *
 * This module is pure (no `vscode` import) so the extension tests exercise the
 * same parsing and decision logic that `resolveDebugConfiguration` uses.
 */

/** The subset of `stc remote status --message-format json` the extension uses. */
export interface RemoteStatusReport {
  /** The remote serve's source fingerprint differs from the local project. */
  readonly drift: boolean;
  /** An online change is compatible with the active remote baseline. */
  readonly online: boolean;
  /** The host can only accept a cold download. */
  readonly coldRequired: boolean;
  /** Why online deployment is unavailable, when known. */
  readonly reason?: string;
  readonly localFingerprint?: string;
  readonly remoteFingerprint?: string;
}

/** A deployment action the user can choose. */
export type RemoteDeployChoice = 'online' | 'cold' | 'cancel';

/**
 * Replaces `${workspaceFolder}` before VS Code's own variable substitution.
 *
 * The remote status query runs in `resolveDebugConfiguration`, which VS Code
 * calls before it substitutes launch-configuration variables. A literal
 * `${workspaceFolder}/stc.toml` would otherwise reach `stc` unchanged.
 */
export function expandWorkspaceFolder(
  value: string,
  folder: string | undefined,
): string {
  if (folder === undefined) {
    return value;
  }
  return value.replace(/\$\{workspaceFolder\}/g, folder);
}

/** One QuickPick entry. */
export interface RemoteChoiceItem {
  readonly label: string;
  readonly description: string;
  readonly choice: RemoteDeployChoice;
}

/**
 * Parses the `stc remote status` JSON report.
 *
 * Returns `undefined` for malformed output so the caller can fall back to the
 * adapter's own drift diagnostic instead of offering a stale choice.
 */
export function parseRemoteStatus(text: string): RemoteStatusReport | undefined {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return undefined;
  }
  if (typeof value !== 'object' || value === null) {
    return undefined;
  }
  const record = value as Record<string, unknown>;
  if (typeof record.drift !== 'boolean' || typeof record.online !== 'boolean') {
    return undefined;
  }
  if (
    record.cold_required !== undefined &&
    typeof record.cold_required !== 'boolean'
  ) {
    return undefined;
  }
  const optionalString = (key: string): string | undefined =>
    typeof record[key] === 'string' ? (record[key] as string) : undefined;
  return {
    drift: record.drift,
    online: record.online,
    coldRequired:
      record.cold_required === true || record.online === false,
    reason: optionalString('reason'),
    localFingerprint: optionalString('local_fingerprint'),
    remoteFingerprint: optionalString('remote_fingerprint'),
  };
}

/**
 * Builds the QuickPick entries for one drift report.
 *
 * Online change is offered only when the host reported compatibility; cold
 * download and cancel are always available.
 */
export function remoteDeployChoices(
  status: RemoteStatusReport,
): RemoteChoiceItem[] {
  const items: RemoteChoiceItem[] = [];
  if (status.online) {
    items.push({
      label: 'Online change',
      description: 'Deploy compatible changes and preserve retained state',
      choice: 'online',
    });
  }
  items.push({
    label: 'Cold download',
    description: 'Stop the remote runtime, deploy, and restart',
    choice: 'cold',
  });
  items.push({
    label: 'Cancel',
    description: 'Leave the remote host unchanged and do not start the session',
    choice: 'cancel',
  });
  return items;
}
