import assert from 'node:assert/strict';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { build } from 'esbuild';

// Task argument construction and variable expansion are pure TypeScript with
// no `vscode` import; bundle them in memory and exercise the same code the
// extension ships.
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

const taskArgs = await loadModule(join(root, 'src', 'taskArgs.ts'));

const variables = {
  workspaceFolder: '/workspace/demo',
  workspaceFolderBasename: 'demo',
};
const generated = ['--message-format', 'short', '--color', 'never'];
const diagnostic = ['--diagnostic-paths', 'absolute'];
const commands = ['check', 'build', 'run', 'test', 'fmt'];

test('all five commands generate the documented argument list', () => {
  const expected = new Map([
    ['check', ['check', ...generated, ...diagnostic]],
    ['build', ['build', ...generated, ...diagnostic]],
    ['run', ['run', ...generated, ...diagnostic]],
    ['test', ['test', ...generated, ...diagnostic]],
    ['fmt', ['fmt', ...generated]],
  ]);
  for (const command of commands) {
    assert.deepEqual(
      taskArgs.commandArguments({ command }, variables),
      expected.get(command),
      command,
    );
  }
});

test('--release is added only for build and run', () => {
  for (const command of commands) {
    const args = taskArgs.commandArguments({ command, release: true }, variables);
    assert.equal(
      args.includes('--release'),
      command === 'build' || command === 'run',
      command,
    );
  }
  assert.deepEqual(
    taskArgs.commandArguments(
      { command: 'build', release: true, args: ['--warnings-as-errors'] },
      variables,
    ),
    ['build', ...generated, ...diagnostic, '--release', '--warnings-as-errors'],
  );
  assert.deepEqual(
    taskArgs.commandArguments({ command: 'run', release: true }, variables),
    ['run', ...generated, ...diagnostic, '--release'],
  );
});

test('diagnostic arguments are added only for check, build, run, and test', () => {
  for (const command of commands) {
    const args = taskArgs.commandArguments({ command }, variables);
    assert.equal(args.includes('--diagnostic-paths'), command !== 'fmt', command);
  }
});

test('every args entry is expanded and appended after generated arguments', () => {
  const expansion = '${workspaceFolder}|${workspaceFolderBasename}|${workspaceRoot}';
  const expanded = '/workspace/demo|demo|/workspace/demo';
  for (const command of commands) {
    const generatedArgs = taskArgs.commandArguments({ command }, variables);
    const args = taskArgs.commandArguments(
      { command, args: [expansion, expansion] },
      variables,
    );
    assert.deepEqual(args.slice(0, generatedArgs.length), generatedArgs, command);
    assert.deepEqual(args.slice(generatedArgs.length), [expanded, expanded], command);
  }
});

test('project expands every supported variable', () => {
  const expected = new Map([
    ['workspaceFolder', '/workspace/demo'],
    ['workspaceFolderBasename', 'demo'],
    ['workspaceRoot', '/workspace/demo'],
  ]);
  for (const [variable, value] of expected) {
    assert.equal(
      taskArgs.expandTaskVariables(`\${${variable}}/stc.toml`, variables),
      `${value}/stc.toml`,
      variable,
    );
  }
});

test('unknown variables stay literal', () => {
  const value =
    '${env:HOME}/${unknown}/${workspaceFolder:foo}/${workspaceFolder}/' +
    '${workspaceFolderBasename}/${workspaceRoot}/${}';
  assert.equal(
    taskArgs.expandTaskVariables(value, variables),
    '${env:HOME}/${unknown}/${workspaceFolder:foo}//workspace/demo/' +
      'demo//workspace/demo/${}',
  );
  assert.deepEqual(
    taskArgs.commandArguments({ command: 'fmt', args: ['${unknown}'] }, variables).slice(-1),
    ['${unknown}'],
  );
});

test('expansion is idempotent for an already-resolved value', () => {
  const expanded = taskArgs.expandTaskVariables('${workspaceFolder}/stc.toml', variables);
  assert.equal(expanded, '/workspace/demo/stc.toml');
  assert.equal(taskArgs.expandTaskVariables(expanded, variables), expanded);
});

test('the run terminal receives the same argument list as the shell path', () => {
  const definition = {
    command: 'run',
    release: true,
    args: ['--store', '${workspaceFolder}/state'],
  };
  const expected = [
    'run',
    ...generated,
    ...diagnostic,
    '--release',
    '--store',
    '/workspace/demo/state',
  ];
  const plan = taskArgs.planTaskExecution(definition, variables);
  assert.equal(plan.kind, 'runTerminal');
  assert.deepEqual(plan.args, expected);
  assert.deepEqual(taskArgs.commandArguments(definition, variables), expected);
  // VS Code resolves the definition before the custom-execution callback; the
  // already-resolved value must not be expanded twice.
  const resolved = {
    ...definition,
    args: ['--store', '/workspace/demo/state'],
  };
  assert.deepEqual(taskArgs.planTaskExecution(resolved, variables).args, expected);
  for (const command of ['check', 'build', 'test', 'fmt']) {
    assert.equal(taskArgs.planTaskExecution({ command }, variables).kind, 'shell', command);
  }
});

test('an empty args list adds nothing', () => {
  assert.deepEqual(
    taskArgs.commandArguments({ command: 'fmt', args: [] }, variables),
    ['fmt', ...generated],
  );
});

test('only the contributed commands are accepted', () => {
  for (const command of commands) {
    assert.equal(taskArgs.isStcCommand(command), true, command);
  }
  assert.equal(taskArgs.isStcCommand('clean'), false);
  assert.equal(taskArgs.isStcCommand(undefined), false);
});
