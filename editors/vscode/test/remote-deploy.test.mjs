import assert from 'node:assert/strict';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { build } from 'esbuild';

// The remote-deploy decision layer is pure TypeScript with no `vscode` import;
// bundle it in memory and exercise the same code the extension ships.
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

const remote = await loadModule(join(root, 'src', 'remoteDeploy.ts'));

test('remote status reports drift, compatibility, and fingerprints', () => {
  const status = remote.parseRemoteStatus(
    JSON.stringify({
      protocol: 'stc-remote-status/1',
      drift: true,
      online: false,
      cold_required: true,
      reason: 'the plan changes a task interval',
      local_fingerprint: '0x11',
      remote_fingerprint: '0x22',
    }),
  );
  assert.deepEqual(status, {
    drift: true,
    online: false,
    coldRequired: true,
    reason: 'the plan changes a task interval',
    localFingerprint: '0x11',
    remoteFingerprint: '0x22',
  });
});

test('remote status rejects malformed reports', () => {
  assert.equal(remote.parseRemoteStatus('not json'), undefined);
  assert.equal(remote.parseRemoteStatus('[]'), undefined);
  assert.equal(remote.parseRemoteStatus('{"drift":true}'), undefined);
  assert.equal(
    remote.parseRemoteStatus('{"drift":true,"online":true,"cold_required":1}'),
    undefined,
  );
});

test('online deployment is offered only when compatible', () => {
  const online = remote.remoteDeployChoices({
    drift: true,
    online: true,
    coldRequired: false,
  });
  assert.deepEqual(
    online.map((item) => item.choice),
    ['online', 'cold', 'cancel'],
  );
  const cold = remote.remoteDeployChoices({
    drift: true,
    online: false,
    coldRequired: true,
  });
  assert.deepEqual(
    cold.map((item) => item.choice),
    ['cold', 'cancel'],
  );
  assert.match(cold[0].description, /restart/);
});

test('a missing cold_required flag follows the online flag', () => {
  const status = remote.parseRemoteStatus('{"drift":true,"online":false}');
  assert.equal(status.coldRequired, true);
  const compatible = remote.parseRemoteStatus(
    '{"drift":true,"online":true,"cold_required":false}',
  );
  assert.equal(compatible.coldRequired, false);
});
