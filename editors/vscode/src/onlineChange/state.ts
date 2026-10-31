/**
 * Pure online-change state, line classification, endpoint parsing, redaction,
 * and formatting.
 *
 * This module imports nothing from `vscode` and performs no I/O so the unit
 * tests can bundle and exercise it directly. The session layer supplies
 * stderr lines and parsed endpoint files; this layer decides what they mean.
 *
 * The CLI's `[stc]` lines are the phase signal (CLI-and-Manifest §1.7). The
 * runtime's format-1 debug endpoint file is the only source of the numeric
 * generation and the active build id (F20-D3); the "keeping generation" lines
 * carry a build id, not a generation, despite their wording.
 */

/** The endpoint file is bounded to 64 KiB before it is parsed. */
export const ENDPOINT_MAX_BYTES = 64 * 1024;

/** The debug endpoint protocol prefix accepted from the runtime. */
const DEBUG_PROTOCOL_PREFIX = 'stc-debug/1';

/** The non-safety stance repeated in the channel and the status tooltip. */
export const NON_SAFETY_NOTICE =
  'stc is not a safety PLC; online change is an operability feature, not a safety function.';

/** The status lines are written by `Log::status` with this prefix. */
const STATUS_PREFIX = '[stc] ';

/** A parsed format-1 debug endpoint file, limited to non-secret fields. */
export interface EndpointSnapshot {
  readonly generation: number;
  readonly buildId: string;
  readonly mode: string;
  readonly pid: number;
}

/** File metadata supplied by the session before reading the endpoint. */
export interface EndpointFileMeta {
  readonly isFile: boolean;
  readonly size: number;
}

/** Why an endpoint file was rejected. */
export type EndpointParseFailure =
  | 'not-a-regular-file'
  | 'too-large'
  | 'invalid-json'
  | 'wrong-format'
  | 'wrong-protocol'
  | 'missing-fields';

/** The endpoint parser result. */
export type EndpointParseResult =
  | { readonly ok: true; readonly snapshot: EndpointSnapshot }
  | { readonly ok: false; readonly reason: EndpointParseFailure };

/**
 * Parses a runtime debug endpoint file.
 *
 * Only `format`, `protocol`, `generation`, `build_id`, `mode`, and `pid` are
 * inspected. `token`, `endpoint`, and `audit_log` are never read into a
 * returned value, retained, or logged.
 */
export function parseEndpointSnapshot(
  text: string,
  meta?: EndpointFileMeta,
): EndpointParseResult {
  if (meta !== undefined) {
    if (!meta.isFile) {
      return { ok: false, reason: 'not-a-regular-file' };
    }
    if (meta.size > ENDPOINT_MAX_BYTES) {
      return { ok: false, reason: 'too-large' };
    }
  }
  if (text.length > ENDPOINT_MAX_BYTES) {
    return { ok: false, reason: 'too-large' };
  }

  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return { ok: false, reason: 'invalid-json' };
  }
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    return { ok: false, reason: 'missing-fields' };
  }
  const record = value as Record<string, unknown>;
  if (record.format !== 1) {
    return { ok: false, reason: 'wrong-format' };
  }
  const protocol = record.protocol;
  if (typeof protocol !== 'string' || !protocol.startsWith(DEBUG_PROTOCOL_PREFIX)) {
    return { ok: false, reason: 'wrong-protocol' };
  }
  const generation = record.generation;
  const buildId = record.build_id;
  const mode = record.mode;
  const pid = record.pid;
  if (
    typeof generation !== 'number' ||
    !Number.isSafeInteger(generation) ||
    generation < 0 ||
    typeof buildId !== 'string' ||
    buildId === '' ||
    typeof mode !== 'string' ||
    mode === '' ||
    typeof pid !== 'number' ||
    !Number.isSafeInteger(pid) ||
    pid <= 0
  ) {
    return { ok: false, reason: 'missing-fields' };
  }
  return { ok: true, snapshot: { generation, buildId, mode, pid } };
}

const CONTROL_NONCE_LINE = /^\[stc\] control nonce\b/;
const CONTROL_ENDPOINT_LINE = /^\[stc\] watching for changes \(control endpoint .*\)$/;
const CONTROL_ENDPOINT_HIDDEN =
  '[stc] watching for changes (control endpoint <hidden>)';
const DEBUG_ENDPOINT_LINE = /^stc: debug endpoint .*$/;
const DEBUG_AUDIT_LINE = /^stc: debug audit .*$/;

/**
 * Removes control and debug-endpoint path metadata from one CLI line.
 *
 * Returns `undefined` when the whole line must be dropped. The control nonce
 * line and any line that names the nonce variable or the control-nonce option
 * are dropped; the control endpoint value and the runtime's debug endpoint
 * file and audit path lines are replaced with `<hidden>`. The numeric
 * generation line never contains control metadata.
 */
export function redactLine(line: string): string | undefined {
  const text = line.replace(/\r$/, '');
  if (CONTROL_NONCE_LINE.test(text)) {
    return undefined;
  }
  if (text.includes('STC_CONTROL_NONCE') || text.includes('--control-nonce')) {
    return undefined;
  }
  if (CONTROL_ENDPOINT_LINE.test(text)) {
    return CONTROL_ENDPOINT_HIDDEN;
  }
  if (DEBUG_ENDPOINT_LINE.test(text)) {
    return 'stc: debug endpoint <hidden>';
  }
  if (DEBUG_AUDIT_LINE.test(text)) {
    return 'stc: debug audit <hidden>';
  }
  return text;
}

/** A recognized `[stc]` status line. */
export type StatusSignal =
  | { readonly type: 'baseline'; readonly cold: boolean }
  | { readonly type: 'watching' }
  | { readonly type: 'stale-baseline' }
  | { readonly type: 'deployed'; readonly buildId: string }
  | {
      readonly type: 'kept';
      readonly detail: 'candidate-rejected' | 'deploy-failed' | 'build-failed';
      readonly buildId: string;
    }
  | { readonly type: 'initial-build-failed'; readonly code?: number }
  | { readonly type: 'host-exited'; readonly code?: number }
  | { readonly type: 'interrupted' }
  | { readonly type: 'timeout' }
  | { readonly type: 'watch-stopped' };

const BUILD_TOKEN = /^[0-9a-fA-F]+$/;

function statusMessage(line: string): string | undefined {
  return line.startsWith(STATUS_PREFIX) ? line.slice(STATUS_PREFIX.length) : undefined;
}

function numberSuffix(message: string, prefix: string): number | undefined {
  const rest = message.slice(prefix.length);
  return /^-?\d+$/.test(rest) ? Number(rest) : undefined;
}

/**
 * Classifies a stable `[stc]` status line.
 *
 * Unknown lines return `undefined` and leave the state unchanged (F20-R1).
 */
export function classifyStatusLine(line: string): StatusSignal | undefined {
  const message = statusMessage(line);
  if (message === undefined) {
    return undefined;
  }
  if (message.startsWith('online baseline ')) {
    return { type: 'baseline', cold: false };
  }
  if (message === 'cold layout: no active baseline') {
    return { type: 'baseline', cold: true };
  }
  if (message.startsWith('watching for changes')) {
    return { type: 'watching' };
  }
  if (message.startsWith('stale baseline; rebuilding')) {
    return { type: 'stale-baseline' };
  }
  if (message.startsWith('deployed build ')) {
    const buildId = message.slice('deployed build '.length).trim();
    return BUILD_TOKEN.test(buildId) ? { type: 'deployed', buildId } : undefined;
  }
  const removed = /^keeping generation ([0-9a-fA-F]+); (.+)$/.exec(message);
  if (removed !== null) {
    const detail =
      removed[2] === 'candidate was rejected'
        ? 'candidate-rejected'
        : removed[2] === 'deploy failed'
          ? 'deploy-failed'
          : undefined;
    if (detail !== undefined) {
      return { type: 'kept', detail, buildId: removed[1] };
    }
  }
  const failed = /^build failed; keeping generation ([0-9a-fA-F]+)$/.exec(message);
  if (failed !== null) {
    return { type: 'kept', detail: 'build-failed', buildId: failed[1] };
  }
  if (message.startsWith('initial build failed with status ')) {
    return {
      type: 'initial-build-failed',
      code: numberSuffix(message, 'initial build failed with status '),
    };
  }
  if (message.startsWith('host exited with status ')) {
    return {
      type: 'host-exited',
      code: numberSuffix(message, 'host exited with status '),
    };
  }
  if (message === 'watch interrupted') {
    return { type: 'interrupted' };
  }
  if (message === 'watch timeout reached') {
    return { type: 'timeout' };
  }
  if (message === 'watch stopped') {
    return { type: 'watch-stopped' };
  }
  return undefined;
}

const DIAGNOSTIC_ERROR = /\bSTC-E-[A-Z0-9]+(?:-[A-Z0-9]+)*\b/;

/**
 * Returns a diagnostic error line for the tooltip and failure notification.
 *
 * Status lines and warning lines are not errors; only `STC-E-...` diagnostics
 * (the reporter writes `error[CODE]: message`) are tracked.
 */
export function errorLine(line: string): string | undefined {
  const text = line.trim();
  if (text === '' || text.startsWith(STATUS_PREFIX)) {
    return undefined;
  }
  return DIAGNOSTIC_ERROR.test(text) ? text : undefined;
}

/** The running phase shown by the status item. */
export type OnlinePhase =
  | 'starting'
  | 'watching'
  | 'building'
  | 'activating'
  | 'active'
  | 'failed';

/** Why a session stopped. */
export type StopReason =
  | 'user'
  | 'host-exited'
  | 'interrupted'
  | 'timeout'
  | 'failed'
  | 'forced';

/** The online-change state machine value. */
export type OnlineState =
  | { readonly kind: 'idle' }
  | {
      readonly kind: 'starting';
      readonly project: string;
      readonly cold: boolean;
      readonly lastError?: string;
    }
  | {
      readonly kind: 'running';
      readonly project: string;
      readonly phase: OnlinePhase;
      readonly generation?: number;
      readonly buildId?: string;
      readonly mode?: string;
      readonly pid?: number;
      readonly cold: boolean;
      readonly keptBuildId?: string;
      readonly lastError?: string;
      /** Set between a `deployed build` line and endpoint reconciliation. */
      readonly pendingBuildId?: string;
    }
  | {
      readonly kind: 'stopped';
      readonly project: string;
      readonly reason: StopReason;
      readonly exitCode?: number;
      readonly lastGeneration?: number;
      readonly lastBuildId?: string;
      readonly lastError?: string;
    };

/** Events consumed by the pure reducer. */
export type OnlineEvent =
  | { readonly type: 'start'; readonly project: string }
  | { readonly type: 'line'; readonly line: string }
  | { readonly type: 'endpoint'; readonly snapshot: EndpointSnapshot }
  | { readonly type: 'deploy-unconfirmed' }
  | {
      readonly type: 'exited';
      readonly code: number | null;
      readonly requested: boolean;
      readonly forced: boolean;
    };

function withLastError<T extends { lastError?: string }>(
  state: T,
  line: string,
): T {
  const lastError = errorLine(line);
  return lastError === undefined ? state : { ...state, lastError };
}

function running(
  state: OnlineState,
): state is Extract<OnlineState, { kind: 'starting' | 'running' }> {
  return state.kind === 'starting' || state.kind === 'running';
}

/**
 * Applies one event. Returns the same object when the event does not change
 * the state, so callers can use identity to avoid redundant UI work.
 */
export function applyEvent(state: OnlineState, event: OnlineEvent): OnlineState {
  switch (event.type) {
    case 'start': {
      return { kind: 'starting', project: event.project, cold: false };
    }
    case 'line': {
      if (!running(state)) {
        return state;
      }
      const signal = classifyStatusLine(event.line);
      if (signal === undefined) {
        return withLastError(state, event.line);
      }
      switch (signal.type) {
        case 'baseline':
          if (state.kind === 'starting' && state.cold !== signal.cold) {
            return { ...state, cold: signal.cold };
          }
          return state;
        case 'watching':
          if (state.kind === 'running' && state.phase === 'watching') {
            return state;
          }
          return { ...state, kind: 'running', phase: 'watching' };
        case 'stale-baseline':
          if (state.kind === 'running' && state.phase === 'building') {
            return state;
          }
          return { ...state, kind: 'running', phase: 'building' };
        case 'deployed':
          return {
            ...state,
            kind: 'running',
            phase: 'activating',
            pendingBuildId: signal.buildId,
          };
        case 'kept':
          return {
            ...state,
            kind: 'running',
            phase: 'failed',
            keptBuildId: signal.buildId,
            pendingBuildId: undefined,
          };
        case 'initial-build-failed':
          return {
            kind: 'stopped',
            project: state.project,
            reason: 'failed',
            lastGeneration: state.kind === 'running' ? state.generation : undefined,
            lastBuildId: state.kind === 'running' ? state.buildId : undefined,
            lastError: state.lastError,
          };
        case 'host-exited':
          return {
            kind: 'stopped',
            project: state.project,
            reason: 'host-exited',
            exitCode: signal.code,
            lastGeneration: state.kind === 'running' ? state.generation : undefined,
            lastBuildId: state.kind === 'running' ? state.buildId : undefined,
            lastError: state.lastError,
          };
        case 'interrupted':
          return {
            kind: 'stopped',
            project: state.project,
            reason: 'interrupted',
            exitCode: 130,
            lastGeneration: state.kind === 'running' ? state.generation : undefined,
            lastBuildId: state.kind === 'running' ? state.buildId : undefined,
            lastError: state.lastError,
          };
        case 'timeout':
          return {
            kind: 'stopped',
            project: state.project,
            reason: 'timeout',
            lastGeneration: state.kind === 'running' ? state.generation : undefined,
            lastBuildId: state.kind === 'running' ? state.buildId : undefined,
            lastError: state.lastError,
          };
        case 'watch-stopped':
          return {
            kind: 'stopped',
            project: state.project,
            reason: 'user',
            exitCode: 0,
            lastGeneration: state.kind === 'running' ? state.generation : undefined,
            lastBuildId: state.kind === 'running' ? state.buildId : undefined,
            lastError: state.lastError,
          };
      }
    }
    case 'endpoint': {
      if (!running(state)) {
        return state;
      }
      const snapshot = event.snapshot;
      if (state.kind === 'starting') {
        return {
          kind: 'running',
          project: state.project,
          phase: 'active',
          generation: snapshot.generation,
          buildId: snapshot.buildId,
          mode: snapshot.mode,
          pid: snapshot.pid,
          cold: false,
          lastError: state.lastError,
        };
      }
      if (state.pendingBuildId !== undefined) {
        if (snapshot.buildId === state.pendingBuildId) {
          return {
            ...state,
            phase: 'active',
            generation: snapshot.generation,
            buildId: snapshot.buildId,
            mode: snapshot.mode,
            pid: snapshot.pid,
            cold: false,
            pendingBuildId: undefined,
            keptBuildId: undefined,
            lastError: undefined,
          };
        }
        // The endpoint still reports the previous build: stay "activating"
        // until the 2 s reconciliation gives up.
        if (state.pid === snapshot.pid && state.mode === snapshot.mode) {
          return state;
        }
        return { ...state, pid: snapshot.pid, mode: snapshot.mode };
      }
      const changed =
        state.generation !== snapshot.generation || state.buildId !== snapshot.buildId;
      if (changed) {
        return {
          ...state,
          phase: 'active',
          generation: snapshot.generation,
          buildId: snapshot.buildId,
          mode: snapshot.mode,
          pid: snapshot.pid,
          cold: false,
          keptBuildId: undefined,
          lastError: undefined,
        };
      }
      if (state.phase !== 'failed' && state.phase !== 'active') {
        return { ...state, phase: 'active' };
      }
      if (state.pid === snapshot.pid && state.mode === snapshot.mode) {
        return state;
      }
      return { ...state, pid: snapshot.pid, mode: snapshot.mode };
    }
    case 'deploy-unconfirmed': {
      if (state.kind !== 'running' || state.pendingBuildId === undefined) {
        return state;
      }
      return {
        ...state,
        phase: 'active',
        buildId: state.pendingBuildId,
        pendingBuildId: undefined,
      };
    }
    case 'exited': {
      if (state.kind === 'idle') {
        return state;
      }
      if (state.kind === 'stopped') {
        const exitCode = state.exitCode ?? event.code ?? undefined;
        return exitCode === state.exitCode ? state : { ...state, exitCode };
      }
      const reason: StopReason = event.forced
        ? 'forced'
        : event.requested
          ? 'user'
          : 'failed';
      return {
        kind: 'stopped',
        project: state.project,
        reason,
        exitCode: event.code ?? undefined,
        lastGeneration: state.kind === 'running' ? state.generation : undefined,
        lastBuildId: state.kind === 'running' ? state.buildId : undefined,
        lastError: state.lastError,
      };
    }
  }
}

/** The read-only snapshot returned by `stc.showOnlineChangeStatus`. */
export interface OnlineChangeSnapshot {
  readonly kind: 'idle' | 'starting' | 'running' | 'stopped';
  readonly phase?: OnlinePhase;
  readonly project?: string;
  readonly generation?: number;
  readonly buildId?: string;
  readonly mode?: string;
  readonly cold?: boolean;
  readonly lastError?: string;
  readonly reason?: StopReason;
  readonly exitCode?: number;
}

/** Builds the snapshot; it contains no control or debug secret. */
export function onlineChangeSnapshot(state: OnlineState): OnlineChangeSnapshot {
  switch (state.kind) {
    case 'idle':
      return { kind: 'idle' };
    case 'starting':
      return {
        kind: 'starting',
        phase: 'starting',
        project: state.project,
        cold: state.cold,
        lastError: state.lastError,
      };
    case 'running':
      return {
        kind: 'running',
        phase: state.phase,
        project: state.project,
        generation: state.generation,
        buildId: state.buildId,
        mode: state.mode,
        cold: state.cold,
        lastError: state.lastError,
      };
    case 'stopped':
      return {
        kind: 'stopped',
        project: state.project,
        generation: state.lastGeneration,
        buildId: state.lastBuildId,
        lastError: state.lastError,
        reason: state.reason,
        exitCode: state.exitCode,
      };
  }
}

/** The first eight hex characters of a build id, or the whole short value. */
export function shortBuildId(buildId: string): string {
  return buildId.length > 8 ? buildId.slice(0, 8) : buildId;
}

function generationText(state: Extract<OnlineState, { kind: 'running' }>): string {
  if (state.generation === undefined) {
    return 'unknown';
  }
  return state.buildId === undefined
    ? `gen ${state.generation}`
    : `gen ${state.generation} · ${shortBuildId(state.buildId)}`;
}

/**
 * The status-bar text, or `undefined` when the item is hidden.
 *
 * Idle and a user-requested stop hide the item; an unexpected stop remains
 * visible until the next start or the status command.
 */
export function statusBarText(state: OnlineState): string | undefined {
  switch (state.kind) {
    case 'idle':
      return undefined;
    case 'starting':
      return '$(sync~spin) stc online: starting';
    case 'running':
      switch (state.phase) {
        case 'starting':
          return '$(sync~spin) stc online: starting';
        case 'building':
          return '$(sync~spin) stc online: building';
        case 'activating':
          return '$(sync~spin) stc online: activating';
        case 'failed':
          return state.generation === undefined
            ? '$(warning) stc online: build failed; previous build kept'
            : `$(warning) stc online: build failed; gen ${state.generation} kept`;
        case 'watching':
        case 'active':
          return state.generation === undefined
            ? '$(sync~spin) stc online: watching'
            : `$(circle-filled) stc online: ${generationText(state)}`;
      }
      break;
    case 'stopped':
      if (state.reason === 'user') {
        return undefined;
      }
      if (state.reason === 'forced') {
        return '$(warning) stc online: stopped (forced)';
      }
      return `$(error) stc online: stopped (${state.reason})`;
  }
}

/** The multi-line status tooltip; it contains no control or debug secret. */
export function statusTooltip(state: OnlineState): string {
  const lines: string[] = ['stc online change'];
  switch (state.kind) {
    case 'idle':
      lines.push('No online change is running.');
      break;
    case 'starting':
      lines.push(`Project: ${state.project}`);
      lines.push('State: starting');
      lines.push(`Cold start: ${state.cold ? 'yes' : 'no'}`);
      break;
    case 'running':
      lines.push(`Project: ${state.project}`);
      lines.push(`State: ${state.phase}`);
      lines.push(
        `Generation: ${state.generation === undefined ? 'unknown' : String(state.generation)}`,
      );
      lines.push(`Build: ${state.buildId ?? 'unknown'}`);
      lines.push(`Mode: ${state.mode ?? 'unknown'}`);
      lines.push(`Host pid: ${state.pid === undefined ? 'unknown' : String(state.pid)}`);
      lines.push(`Cold start: ${state.cold ? 'yes' : 'no'}`);
      break;
    case 'stopped':
      lines.push(`Project: ${state.project}`);
      lines.push(`State: stopped (${state.reason})`);
      lines.push(
        `Generation: ${
          state.lastGeneration === undefined ? 'unknown' : String(state.lastGeneration)
        }`,
      );
      lines.push(`Build: ${state.lastBuildId ?? 'unknown'}`);
      if (state.exitCode !== undefined) {
        lines.push(`Exit code: ${state.exitCode}`);
      }
      break;
  }
  if (state.kind !== 'idle' && state.lastError !== undefined) {
    lines.push(`Last error: ${state.lastError}`);
  }
  lines.push(NON_SAFETY_NOTICE);
  return lines.join('\n');
}
