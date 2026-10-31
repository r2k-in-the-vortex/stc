import assert from 'node:assert/strict';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { build } from 'esbuild';

// The state layer is pure TypeScript with no `vscode` import; bundle it in
// memory and exercise the same code the extension ships.
const root = dirname(dirname(fileURLToPath(import.meta.url)));

async function loadStateModule() {
  const result = await build({
    entryPoints: [join(root, 'src', 'onlineChange', 'state.ts')],
    bundle: true,
    write: false,
    format: 'cjs',
    platform: 'node',
    target: 'node18',
    logLevel: 'silent',
  });
  const moduleObject = { exports: {} };
  const source = new Function('module', 'exports', result.outputFiles[0].text);
  source(moduleObject, moduleObject.exports);
  return moduleObject.exports;
}

const state = await loadStateModule();

const BUILD_A = '0123456789abcdef0123456789abcdef';
const BUILD_B = 'fedcba9876543210fedcba9876543210';

const LINES = {
  baseline: `[stc] online baseline ${BUILD_A} from C:\\work\\runtime`,
  cold: '[stc] cold layout: no active baseline',
  watching:
    '[stc] watching for changes (control endpoint \\\\.\\pipe\\stc-1234-2f3a9c0b1d2e4f60)',
  stale: '[stc] stale baseline; rebuilding against the active manifest',
  deployedA: `[stc] deployed build ${BUILD_A}`,
  deployedB: `[stc] deployed build ${BUILD_B}`,
  rejected: `[stc] keeping generation ${BUILD_A}; candidate was rejected`,
  deployFailed: `[stc] keeping generation ${BUILD_A}; deploy failed`,
  buildFailed: `[stc] build failed; keeping generation ${BUILD_A}`,
  initialFailed: '[stc] initial build failed with status 1',
  hostExited: '[stc] host exited with status 7',
  interrupted: '[stc] watch interrupted',
  timeout: '[stc] watch timeout reached',
  stopped: '[stc] watch stopped',
  diagnostic: 'error[STC-E-ONLINE-008]: the candidate is incompatible',
  warning: 'warning[STC-W-ONLINE-007]: the runtime store is invalid',
};

const NONCE = '2f3a9c0b1d2e4f60778899aabbccddee';
const CONTROL_ENDPOINT = '\\\\.\\pipe\\stc-1234-2f3a9c0b1d2e4f60778899aabbccddee';

function snapshot(overrides = {}) {
  return {
    format: 1,
    protocol: 'stc-debug/1.1',
    generation: 4,
    build_id: BUILD_A,
    mode: 'debug',
    pid: 4321,
    transport: 'pipe',
    token: 'debug-token-that-must-not-escape',
    endpoint: CONTROL_ENDPOINT,
    audit_log: 'C:\\work\\runtime\\audit.jsonl',
    ...overrides,
  };
}

function start(project = 'C:\\work\\app') {
  return state.applyEvent({ kind: 'idle' }, { type: 'start', project });
}

function line(current, text) {
  return state.applyEvent(current, { type: 'line', line: text });
}

function endpoint(current, overrides = {}) {
  return state.applyEvent(current, {
    type: 'endpoint',
    snapshot: {
      generation: 4,
      buildId: BUILD_A,
      mode: 'debug',
      pid: 4321,
      ...overrides,
    },
  });
}

test('classifies every stable watch status line', () => {
  assert.deepEqual(state.classifyStatusLine(LINES.baseline), {
    type: 'baseline',
    cold: false,
  });
  assert.deepEqual(state.classifyStatusLine(LINES.cold), {
    type: 'baseline',
    cold: true,
  });
  assert.deepEqual(state.classifyStatusLine(LINES.watching), { type: 'watching' });
  assert.deepEqual(state.classifyStatusLine(LINES.stale), { type: 'stale-baseline' });
  assert.deepEqual(state.classifyStatusLine(LINES.deployedA), {
    type: 'deployed',
    buildId: BUILD_A,
  });
  assert.deepEqual(state.classifyStatusLine(LINES.rejected), {
    type: 'kept',
    detail: 'candidate-rejected',
    buildId: BUILD_A,
  });
  assert.deepEqual(state.classifyStatusLine(LINES.deployFailed), {
    type: 'kept',
    detail: 'deploy-failed',
    buildId: BUILD_A,
  });
  assert.deepEqual(state.classifyStatusLine(LINES.buildFailed), {
    type: 'kept',
    detail: 'build-failed',
    buildId: BUILD_A,
  });
  assert.deepEqual(state.classifyStatusLine(LINES.initialFailed), {
    type: 'initial-build-failed',
    code: 1,
  });
  assert.deepEqual(state.classifyStatusLine(LINES.hostExited), {
    type: 'host-exited',
    code: 7,
  });
  assert.deepEqual(state.classifyStatusLine(LINES.interrupted), { type: 'interrupted' });
  assert.deepEqual(state.classifyStatusLine(LINES.timeout), { type: 'timeout' });
  assert.deepEqual(state.classifyStatusLine(LINES.stopped), { type: 'watch-stopped' });
});

test('unknown and non-status lines do not change the state', () => {
  const running = endpoint(line(line(start(), LINES.watching), LINES.deployedA), {
    generation: 1,
    buildId: BUILD_A,
  });
  for (const text of [
    '[stc] something new the extension has not learned yet',
    'Building module...',
    '',
  ]) {
    assert.strictEqual(line(running, text), running);
  }
  assert.equal(state.classifyStatusLine('[stc] control nonce deadbeef'), undefined);
  assert.equal(state.classifyStatusLine(LINES.diagnostic), undefined);
});

test('the reducer walks start, watching, deploy, and reconciliation', () => {
  let current = start();
  assert.equal(current.kind, 'starting');

  current = line(current, LINES.cold);
  assert.equal(current.kind, 'starting');
  assert.equal(current.cold, true);

  current = line(current, LINES.watching);
  assert.equal(current.kind, 'running');
  assert.equal(current.phase, 'watching');

  current = endpoint(current, { generation: 1, buildId: BUILD_A });
  assert.equal(current.phase, 'active');
  assert.equal(current.generation, 1);
  assert.equal(current.buildId, BUILD_A);
  assert.equal(current.mode, 'debug');
  assert.equal(current.pid, 4321);
  assert.equal(current.cold, false);

  // A deploy line enters "activating"; the endpoint still reports the old
  // build, so the numeric generation does not move yet.
  current = line(current, LINES.deployedB);
  assert.equal(current.phase, 'activating');
  assert.equal(current.pendingBuildId, BUILD_B);
  assert.strictEqual(endpoint(current, { generation: 1, buildId: BUILD_A }), current);

  // If the endpoint does not catch up in 2 s, show the deployed build with
  // the last known generation.
  current = state.applyEvent(current, { type: 'deploy-unconfirmed' });
  assert.equal(current.phase, 'active');
  assert.equal(current.buildId, BUILD_B);
  assert.equal(current.generation, 1);

  // The endpoint is authoritative and now reports the activation.
  current = endpoint(current, { generation: 2, buildId: BUILD_B });
  assert.equal(current.phase, 'active');
  assert.equal(current.generation, 2);
  assert.equal(current.buildId, BUILD_B);
});

test('the endpoint is authoritative when it activates before reconciliation', () => {
  let current = line(line(start(), LINES.watching), LINES.deployedB);
  current = endpoint(current, { generation: 2, buildId: BUILD_B });
  assert.equal(current.phase, 'active');
  assert.equal(current.generation, 2);
  assert.equal(current.buildId, BUILD_B);
  assert.equal(current.pendingBuildId, undefined);
});

test('a stale baseline rebuild shows building and a kept failure keeps the old build', () => {
  let current = endpoint(line(line(start(), LINES.watching), LINES.deployedA), {
    generation: 1,
    buildId: BUILD_A,
  });
  current = line(current, LINES.stale);
  assert.equal(current.phase, 'building');
  current = line(current, LINES.rejected);
  assert.equal(current.phase, 'failed');
  assert.equal(current.keptBuildId, BUILD_A);
  // The endpoint still reports the kept build: failure stays visible.
  assert.strictEqual(endpoint(current, { generation: 1, buildId: BUILD_A }), current);
  // A later activation clears the kept state.
  current = endpoint(current, { generation: 2, buildId: BUILD_B });
  assert.equal(current.phase, 'active');
  assert.equal(current.keptBuildId, undefined);
});

test('diagnostic error lines feed lastError and a successful activation clears it', () => {
  let current = line(start(), LINES.diagnostic);
  assert.equal(current.lastError, LINES.diagnostic);
  current = line(current, LINES.warning);
  assert.equal(current.lastError, LINES.diagnostic, 'warnings are not tracked as the last error');
  current = line(current, LINES.buildFailed);
  assert.equal(current.lastError, LINES.diagnostic);
  current = endpoint(current, { generation: 2, buildId: BUILD_B });
  assert.equal(current.lastError, undefined);
});

test('stop lines and the exit event produce the documented reasons', () => {
  const cases = [
    [LINES.stopped, 'user', 0],
    [LINES.interrupted, 'interrupted', 130],
    [LINES.timeout, 'timeout', undefined],
    [LINES.hostExited, 'host-exited', 7],
    [LINES.initialFailed, 'failed', undefined],
  ];
  for (const [text, reason, exitCode] of cases) {
    let current = line(line(start(), LINES.watching), text);
    assert.equal(current.kind, 'stopped');
    assert.equal(current.reason, reason, text);
    if (exitCode !== undefined) {
      assert.equal(current.exitCode, exitCode, text);
    }
    current = state.applyEvent(current, {
      type: 'exited',
      code: exitCode ?? 1,
      requested: reason === 'user',
      forced: false,
    });
    assert.equal(current.reason, reason, `${text} keeps the recognized reason`);
  }

  let requested = state.applyEvent(line(start(), LINES.watching), {
    type: 'exited',
    code: 0,
    requested: true,
    forced: false,
  });
  assert.equal(requested.reason, 'user');

  let forced = state.applyEvent(line(start(), LINES.watching), {
    type: 'exited',
    code: null,
    requested: true,
    forced: true,
  });
  assert.equal(forced.reason, 'forced');

  let unexpected = state.applyEvent(line(start(), LINES.watching), {
    type: 'exited',
    code: 1,
    requested: false,
    forced: false,
  });
  assert.equal(unexpected.reason, 'failed');
});

test('endpoint parsing accepts format 1 and discards secret fields', () => {
  const result = state.parseEndpointSnapshot(JSON.stringify(snapshot()));
  assert.equal(result.ok, true);
  assert.deepEqual(result.snapshot, {
    generation: 4,
    buildId: BUILD_A,
    mode: 'debug',
    pid: 4321,
  });
  const serialized = JSON.stringify(result.snapshot);
  assert.ok(!serialized.includes('token'), serialized);
  assert.ok(!serialized.includes(CONTROL_ENDPOINT), serialized);
  assert.ok(!serialized.includes('audit'), serialized);
});

test('endpoint parsing accepts stc-debug/1 and rejects malformed files', () => {
  const accepted = state.parseEndpointSnapshot(
    JSON.stringify(snapshot({ protocol: 'stc-debug/1' })),
  );
  assert.equal(accepted.ok, true);

  const failures = [
    ['not json', undefined, 'invalid-json'],
    ['[]', undefined, 'missing-fields'],
    [JSON.stringify(snapshot({ format: 2 })), undefined, 'wrong-format'],
    [JSON.stringify(snapshot({ protocol: 'stc-debug/2' })), undefined, 'wrong-protocol'],
    [JSON.stringify(snapshot({ generation: -1 })), undefined, 'missing-fields'],
    [JSON.stringify(snapshot({ build_id: '' })), undefined, 'missing-fields'],
    [JSON.stringify(snapshot({ mode: 3 })), undefined, 'missing-fields'],
    [JSON.stringify(snapshot({ pid: 0 })), undefined, 'missing-fields'],
    [JSON.stringify(snapshot()), { isFile: false, size: 10 }, 'not-a-regular-file'],
    [
      JSON.stringify(snapshot()),
      { isFile: true, size: state.ENDPOINT_MAX_BYTES + 1 },
      'too-large',
    ],
  ];
  for (const [text, meta, reason] of failures) {
    const result = state.parseEndpointSnapshot(text, meta);
    assert.equal(result.ok, false, `${reason}: ${String(text).slice(0, 40)}`);
    assert.equal(result.reason, reason);
  }
  const oversized = state.parseEndpointSnapshot(
    'x'.repeat(state.ENDPOINT_MAX_BYTES + 1),
  );
  assert.equal(oversized.ok, false);
  assert.equal(oversized.reason, 'too-large');
});

test('redaction drops the nonce and hides control and debug paths', () => {
  assert.equal(
    state.redactLine(`[stc] control nonce ${NONCE}`),
    undefined,
  );
  assert.equal(
    state.redactLine(LINES.watching),
    '[stc] watching for changes (control endpoint <hidden>)',
  );
  assert.equal(state.redactLine(`STC_CONTROL_NONCE=${NONCE}`), undefined);
  assert.equal(
    state.redactLine(`stc run --control x --control-nonce ${NONCE}`),
    undefined,
  );
  assert.equal(
    state.redactLine('stc: debug endpoint C:\\storage\\oc\\1234abcd\\d.json'),
    'stc: debug endpoint <hidden>',
  );
  assert.equal(
    state.redactLine('stc: debug audit C:\\storage\\oc\\1234abcd\\d.audit.jsonl'),
    'stc: debug audit <hidden>',
  );
  assert.equal(state.redactLine('[stc] watch stopped'), '[stc] watch stopped');
  assert.equal(state.redactLine(LINES.diagnostic), LINES.diagnostic);

  const captured = [
    `[stc] watching for changes (control endpoint ${CONTROL_ENDPOINT})`,
    `[stc] control nonce ${NONCE}`,
    'stc: debug endpoint C:\\storage\\oc\\1234abcd\\d.json',
    'stc: debug audit C:\\storage\\oc\\1234abcd\\d.audit.jsonl',
    LINES.baseline,
    LINES.deployedA,
    LINES.stopped,
  ];
  const visible = captured
    .map((text) => state.redactLine(text))
    .filter((text) => text !== undefined)
    .join('\n');
  assert.ok(!visible.includes(NONCE), visible);
  assert.ok(!visible.includes(CONTROL_ENDPOINT), visible);
  assert.ok(!visible.includes('C:\\storage\\oc'), visible);
  assert.ok(visible.includes('control endpoint <hidden>'), visible);
  assert.ok(visible.includes('stc: debug endpoint <hidden>'), visible);
  assert.ok(visible.includes('stc: debug audit <hidden>'), visible);
});

test('the status item and tooltip expose state without secrets', () => {
  assert.equal(state.statusBarText({ kind: 'idle' }), undefined);
  assert.match(state.statusBarText(start()), /starting/);

  const active = endpoint(line(line(start(), LINES.watching), LINES.deployedA), {
    generation: 3,
    buildId: BUILD_A,
  });
  const text = state.statusBarText(active);
  assert.match(text, /gen 3/);
  assert.match(text, /01234567/);
  assert.ok(!text.includes(BUILD_A), text);

  const failed = line(active, LINES.rejected);
  assert.match(state.statusBarText(failed), /build failed; gen 3 kept/);

  const userStop = line(failed, LINES.stopped);
  assert.equal(state.statusBarText(userStop), undefined);

  const failedStop = line(active, LINES.interrupted);
  assert.match(state.statusBarText(failedStop), /stopped \(interrupted\)/);

  const forced = state.applyEvent(active, {
    type: 'exited',
    code: null,
    requested: true,
    forced: true,
  });
  assert.match(state.statusBarText(forced), /stopped \(forced\)/);

  const tooltip = state.statusTooltip(active);
  assert.match(tooltip, /Project: C:\\work\\app/);
  assert.match(tooltip, new RegExp(BUILD_A));
  assert.match(tooltip, /Host pid: 4321/);
  assert.match(tooltip, /not a safety PLC/);
});

test('the status snapshot is a plain copy with no secrets', () => {
  const active = endpoint(line(line(start(), LINES.watching), LINES.deployedB), {
    generation: 9,
    buildId: BUILD_B,
  });
  const snapshot = state.onlineChangeSnapshot(active);
  assert.deepEqual(snapshot, {
    kind: 'running',
    phase: 'active',
    project: 'C:\\work\\app',
    generation: 9,
    buildId: BUILD_B,
    mode: 'debug',
    cold: false,
    lastError: undefined,
  });
  const serialized = JSON.stringify(snapshot);
  assert.ok(!serialized.includes('token'), serialized);
  assert.ok(!serialized.includes(CONTROL_ENDPOINT), serialized);
});
