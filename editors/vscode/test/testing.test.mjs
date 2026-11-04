import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { build } from 'esbuild';

// The testing model is pure TypeScript with no `vscode` import; bundle it in
// memory and exercise the same code the extension ships.
const root = dirname(dirname(fileURLToPath(import.meta.url)));
const require = createRequire(import.meta.url);

async function loadModelModule() {
  const result = await build({
    entryPoints: [join(root, 'src', 'testing', 'model.ts')],
    bundle: true,
    write: false,
    format: 'cjs',
    platform: 'node',
    target: 'node18',
    logLevel: 'silent',
  });
  const moduleObject = { exports: {} };
  const source = new Function('module', 'exports', 'require', result.outputFiles[0].text);
  source(moduleObject, moduleObject.exports, require);
  return moduleObject.exports;
}

const model = await loadModelModule();

const LIST_ENTRY = (name, file, line, column, packageName = 'sample') =>
  JSON.stringify({ name, package: packageName, file, line, column });

test('list JSON yields test locations and ignores diagnostics', () => {
  const stdout = [
    LIST_ENTRY('AddStepWorks', 'tests/smoke.st', 3, 1),
    LIST_ENTRY('AccumulatorWorks', 'tests/smoke.st', 13, 1),
    LIST_ENTRY('DependencyWorks', 'dep/tests/dep.st', 7, 5, 'dep'),
    '{"code":"STC-E-001","severity":"error","message":"ignored in list mode"}',
    '',
  ].join('\r\n');
  const parsed = model.parseTestList(stdout);
  assert.deepEqual(parsed.problems, []);
  assert.deepEqual(parsed.tests, [
    {
      name: 'AddStepWorks',
      package: 'sample',
      file: 'tests/smoke.st',
      line: 3,
      column: 1,
    },
    {
      name: 'AccumulatorWorks',
      package: 'sample',
      file: 'tests/smoke.st',
      line: 13,
      column: 1,
    },
    {
      name: 'DependencyWorks',
      package: 'dep',
      file: 'dep/tests/dep.st',
      line: 7,
      column: 5,
    },
  ]);
});

test('list parsing rejects old and malformed entries without throwing', () => {
  assert.deepEqual(model.parseTestList(''), { tests: [], problems: [] });
  assert.deepEqual(model.parseTestList('\n\n'), { tests: [], problems: [] });

  const parsed = model.parseTestList(
    [
      '{"name":"OldShape"}',
      '{"name":"NoColumn","file":"tests/x.st","line":2}',
      '{"name":"ZeroLine","file":"tests/x.st","line":0,"column":1}',
      '{"name":"","file":"tests/x.st","line":1,"column":1}',
      'not json',
      '{"name":',
    ].join('\n'),
  );
  assert.deepEqual(parsed.tests, []);
  assert.equal(parsed.problems.length, 6);
  assert.match(parsed.problems[0], /no source file/);
  assert.match(parsed.problems[1], /no 1-based line and column/);
  assert.match(parsed.problems[2], /no 1-based line and column/);
  assert.match(parsed.problems[3], /no test name/);
  assert.match(parsed.problems[4], /unrecognized --list output/);
  assert.match(parsed.problems[5], /unrecognized --list output/);
});

test('file-level grouping keeps discovery order and package', () => {
  const tests = [
    { name: 'AlphaWorks', package: 'sample', file: 'tests/smoke.st', line: 3, column: 1 },
    { name: 'DependencyWorks', package: 'dep', file: 'dep/tests/dep.st', line: 7, column: 5 },
    { name: 'BetaWorks', package: 'sample', file: 'tests/smoke.st', line: 13, column: 1 },
    { name: 'GammaWorks', package: 'sample', file: 'tests/more.st', line: 2, column: 1 },
  ];
  const groups = model.groupTestsByFile(tests);
  assert.deepEqual(
    groups.map((group) => ({
      file: group.file,
      package: group.package,
      names: group.tests.map((entry) => entry.name),
    })),
    [
      { file: 'tests/smoke.st', package: 'sample', names: ['AlphaWorks', 'BetaWorks'] },
      { file: 'dep/tests/dep.st', package: 'dep', names: ['DependencyWorks'] },
      { file: 'tests/more.st', package: 'sample', names: ['GammaWorks'] },
    ],
  );
  assert.deepEqual(model.groupTestsByFile([]), []);
});

test('result JSON maps pass, fail, trap, timeout, output, and locations', () => {
  const stdout = [
    'test host says hi',
    '{"kind":"stc-test-result","name":"AlphaWorks","status":"pass","duration_ms":1}',
    '{"name":"AlphaWorks","status":"pass","duration_ms":3,"failures":[],"trap":null}',
    '{"name":"not-a-result"}',
    'printed before Beta',
    JSON.stringify({
      name: 'BetaWorks',
      status: 'fail',
      duration_ms: 7,
      failures: [
        {
          message: 'result should be 3',
          file: 'tests/smoke.st',
          line: 9,
          column: 3,
        },
        { message: 'second failure' },
      ],
      trap: null,
    }),
    JSON.stringify({
      name: 'TrapWorks',
      status: 'trap',
      duration_ms: 1,
      failures: [],
      trap: 'STC_TRAP_DIV_ZERO',
    }),
    JSON.stringify({
      name: 'TimeoutWorks',
      status: 'timeout',
      duration_ms: 1000,
      failures: [],
    }),
    '{"code":"STC-E-BUILD","severity":"error","message":"bad source",' +
      '"file":"src/main.st","span":{"start":0,"end":1,"line":2,"column":4}}',
  ].join('\n');
  const parsed = model.parseTestResults(stdout);
  assert.deepEqual(parsed.problems, []);
  assert.equal(parsed.results.length, 4);
  assert.equal(parsed.diagnostics.length, 1);
  assert.deepEqual(parsed.diagnostics[0], {
    code: 'STC-E-BUILD',
    severity: 'error',
    message: 'bad source',
    file: 'src/main.st',
    line: 2,
    column: 4,
  });

  const [alpha, beta, trapped, timedOut] = parsed.results;
  assert.equal(alpha.name, 'AlphaWorks');
  assert.equal(alpha.status, 'pass');
  assert.equal(alpha.durationMs, 3);
  assert.equal(
    alpha.output,
    'test host says hi',
    'the host result line is dropped',
  );
  assert.equal(beta.status, 'fail');
  assert.equal(beta.durationMs, 7);
  assert.deepEqual(beta.failures, [
    {
      message: 'result should be 3',
      file: 'tests/smoke.st',
      line: 9,
      column: 3,
    },
    {
      message: 'second failure',
      file: undefined,
      line: undefined,
      column: undefined,
    },
  ]);
  assert.equal(
    beta.output,
    '{"name":"not-a-result"}\nprinted before Beta',
    'test JSON stays output instead of being mistaken for a result',
  );
  assert.equal(trapped.status, 'trap');
  assert.equal(trapped.trap, 'STC_TRAP_DIV_ZERO');
  assert.equal(timedOut.status, 'timeout');
  assert.equal(timedOut.trap, undefined);
});

test('the result stream handles chunks, CRLF, and a final partial line', () => {
  const reported = [];
  const stream = new model.ResultStream((result) => reported.push(result.name));
  stream.push('{"name":"AlphaWorks","status":"pa');
  stream.push('ss","duration_ms":2,"failures":[]}\r\n');
  stream.push('{"name":"BetaWorks","status":"fail","duration_ms":4,');
  stream.push('"failures":[{"message":"nope"}]}\n');
  const parsed = stream.finish();
  assert.deepEqual(reported, ['AlphaWorks', 'BetaWorks']);
  assert.deepEqual(
    parsed.results.map((result) => [result.name, result.status, result.durationMs]),
    [
      ['AlphaWorks', 'pass', 2],
      ['BetaWorks', 'fail', 4],
    ],
  );
  assert.equal(parsed.results[1].failures[0].message, 'nope');
});

test('empty, malformed, and result-shaped output is handled without throwing', () => {
  assert.deepEqual(model.parseTestResults(''), {
    results: [],
    diagnostics: [],
    problems: [],
  });
  assert.deepEqual(model.parseTestResults('plain host output\n'), {
    results: [],
    diagnostics: [],
    problems: [],
  });
  const malformed = model.parseTestResults('{"name":');
  assert.equal(malformed.results.length, 0);
  assert.equal(malformed.problems.length, 0);

  const badStatus = model.parseTestResults(
    '{"name":"Mystery","status":"exploded","duration_ms":1,"failures":[]}',
  );
  assert.equal(badStatus.results.length, 0);
  assert.equal(badStatus.problems.length, 1);
  assert.match(badStatus.problems[0], /unrecognized test result/);

  assert.deepEqual(
    model.parseDiagnostics(
      [
        'ignored',
        '{"code":"STC-E-A","severity":"error","message":"one"}',
        '{"code":"STC-W-B","severity":"warning","message":"two","span":{"line":4,"column":2}}',
      ].join('\n'),
    ),
    [
      {
        code: 'STC-E-A',
        severity: 'error',
        message: 'one',
        file: undefined,
        line: undefined,
        column: undefined,
      },
      {
        code: 'STC-W-B',
        severity: 'warning',
        message: 'two',
        file: undefined,
        line: 4,
        column: 2,
      },
    ],
  );
  assert.deepEqual(model.parseDiagnostics('nothing here'), []);
});

test('run arguments select exact tests and capture test-host output', () => {
  const manifest = join(root, 'samples', 'workspace', 'stc.toml');
  assert.deepEqual(model.testListArguments(manifest), [
    'test',
    '--manifest-path',
    manifest,
    '--message-format',
    'json',
    '--color',
    'never',
    '--diagnostic-paths',
    'absolute',
    '--list',
  ]);
  assert.deepEqual(model.testRunArguments(manifest, ['AddStepWorks']), [
    'test',
    '--manifest-path',
    manifest,
    '--message-format',
    'json',
    '--color',
    'never',
    '--diagnostic-paths',
    'absolute',
    '--exact',
    'AddStepWorks',
    '--',
    '--nocapture',
  ]);
  const multiple = model.testRunArguments(manifest, ['AddStepWorks', 'AccumulatorWorks']);
  assert.deepEqual(multiple.slice(multiple.indexOf('--exact')), [
    '--exact',
    'AddStepWorks',
    'AccumulatorWorks',
    '--',
    '--nocapture',
  ]);
  assert.equal(multiple.filter((argument) => argument === '--exact').length, 1);
  assert.deepEqual(model.testRunArguments(manifest, []).slice(-3), [
    '--exact',
    '--',
    '--nocapture',
  ]);
});

test('debug configuration uses the F-17 per-test launch field', () => {
  const manifest = join(root, 'samples', 'workspace', 'stc.toml');
  assert.deepEqual(model.testDebugConfiguration(manifest, 'AddStepWorks'), {
    type: 'stc',
    request: 'launch',
    name: 'stc: Debug AddStepWorks',
    project: manifest,
    test: 'AddStepWorks',
    noBuild: false,
    release: false,
    allowWrite: false,
    allowForce: false,
  });
});

test('result summaries give every test readable output', () => {
  const base = { output: '', trap: undefined };
  assert.equal(
    model.formatResultSummary({
      ...base,
      name: 'AddWorks',
      status: 'pass',
      durationMs: 3,
      failures: [],
    }),
    'test AddWorks ... ok (3 ms)',
  );
  assert.equal(
    model.formatResultSummary({
      ...base,
      name: 'FailsOnPurpose',
      status: 'fail',
      durationMs: 7,
      failures: [
        { message: 'intentional failure', file: 'tests/smoke.st', line: 13, column: 3 },
        { message: 'plain failure' },
      ],
    }),
    [
      'test FailsOnPurpose ... FAILED (7 ms)',
      '  intentional failure (tests/smoke.st:13:3)',
      '  plain failure',
    ].join('\n'),
  );
  assert.equal(
    model.formatResultSummary({
      ...base,
      name: 'TrapWorks',
      status: 'trap',
      durationMs: 1,
      failures: [],
      trap: 'STC_TRAP_DIV_ZERO',
    }),
    'test TrapWorks ... TRAPPED: STC_TRAP_DIV_ZERO (1 ms)',
  );
  assert.equal(
    model.formatResultSummary({
      ...base,
      name: 'TimeoutWorks',
      status: 'timeout',
      durationMs: 1000,
      failures: [],
    }),
    'test TimeoutWorks ... TIMEOUT (1000 ms)',
  );
});

test('result paths resolve against the project and respect absolute paths', () => {
  const project = resolve(root, 'samples', 'workspace');
  assert.equal(
    model.resolveResultPath(project, 'tests/smoke.st'),
    resolve(project, 'tests/smoke.st'),
  );
  const absolute = resolve(project, 'tests', 'smoke.st');
  assert.equal(model.resolveResultPath(project, absolute), absolute);
});

test('diagnostics format with their source location', () => {
  assert.equal(
    model.formatDiagnostic({
      code: 'STC-E-001',
      severity: 'error',
      message: 'bad source',
      file: 'src/main.st',
      line: 2,
      column: 4,
    }),
    'error[STC-E-001]: bad source (src/main.st:2:4)',
  );
  assert.equal(
    model.formatDiagnostic({
      code: 'STC-W-002',
      severity: 'warning',
      message: 'careful',
      file: 'src/main.st',
      line: undefined,
      column: undefined,
    }),
    'warning[STC-W-002]: careful (src/main.st)',
  );
  assert.equal(
    model.formatDiagnostic({
      code: 'STC-E-003',
      severity: 'error',
      message: 'nowhere',
      file: undefined,
      line: undefined,
      column: undefined,
    }),
    'error[STC-E-003]: nowhere',
  );
});
