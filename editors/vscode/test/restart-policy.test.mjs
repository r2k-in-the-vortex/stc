import assert from 'node:assert/strict';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { build } from 'esbuild';

// The restart policy is pure TypeScript with no `vscode` import; bundle it in
// memory and exercise the same code the extension ships.
const root = dirname(dirname(fileURLToPath(import.meta.url)));

async function loadModule(entryPoint) {
  const result = await build({
    entryPoints: [entryPoint],
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

const policyModule = await loadModule(join(root, 'src', 'restartPolicy.ts'));
const { RestartPolicy, DEFAULT_MAX_RESTARTS, DEFAULT_RESTART_WINDOW_MS } = policyModule;

test('an initialization failure never restarts the client', () => {
  const policy = new RestartPolicy({ now: () => 0 });
  assert.equal(policy.hasFailedInitialization, false);
  policy.noteInitializationFailure();
  assert.equal(policy.hasFailedInitialization, true);
  assert.deepEqual(policy.closed(), { action: 'stop', handled: true });
  // A second close (for example after the process termination timer) is still
  // a teardown, not a crash.
  assert.deepEqual(policy.closed(), { action: 'stop', handled: true });
});

test('initialization failure wins over an existing crash history', () => {
  const policy = new RestartPolicy({ maxRestarts: 1, now: () => 0 });
  assert.deepEqual(policy.closed(), { action: 'restart' });
  policy.noteInitializationFailure();
  assert.deepEqual(policy.closed(), { action: 'stop', handled: true });
});

test('unexpected crashes restart up to the default limit and then stop', () => {
  let now = 0;
  const policy = new RestartPolicy({ now: () => now });
  for (let attempt = 0; attempt < DEFAULT_MAX_RESTARTS; attempt += 1) {
    now += 1_000;
    assert.deepEqual(
      policy.closed(),
      { action: 'restart' },
      `restart ${attempt + 1} is allowed`,
    );
  }
  now += 1_000;
  const decision = policy.closed();
  assert.equal(decision.action, 'stop');
  assert.match(decision.message, /crashed 5 times/);
  assert.match(decision.message, /will not be restarted/);
});

test('crashes outside the window are pruned instead of blocking restarts', () => {
  let now = 0;
  const policy = new RestartPolicy({ maxRestarts: 1, windowMs: 1_000, now: () => now });
  assert.deepEqual(policy.closed(), { action: 'restart' });
  now = 500;
  assert.equal(policy.closed().action, 'stop');
  // The oldest entry is now outside the window; a later crash restarts again.
  now = 1_600;
  assert.deepEqual(policy.closed(), { action: 'restart' });
});

test('custom limits are honored', () => {
  let now = 0;
  const policy = new RestartPolicy({ maxRestarts: 0, windowMs: 500, now: () => now });
  const decision = policy.closed();
  assert.equal(decision.action, 'stop');
  assert.match(decision.message, /crashed 1 times/);
});

test('the default window is three minutes', () => {
  assert.equal(DEFAULT_RESTART_WINDOW_MS, 3 * 60 * 1000);
  assert.equal(DEFAULT_MAX_RESTARTS, 4);
});

test('connection errors continue briefly and then shut down', () => {
  const policy = new RestartPolicy({ now: () => 0 });
  assert.deepEqual(policy.connectionError(1), { action: 'continue' });
  assert.deepEqual(policy.connectionError(3), { action: 'continue' });
  assert.deepEqual(policy.connectionError(4), { action: 'shutdown' });
  assert.deepEqual(policy.connectionError(undefined), { action: 'shutdown' });
});
