import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));

function problemMatcher() {
  const matchers = pkg.contributes?.problemMatchers ?? [];
  const matcher = matchers.find((candidate) => candidate.name === 'stc');
  assert.ok(matcher, 'package.json contributes the `stc` problem matcher');
  return matcher;
}

function apply(line) {
  const { pattern } = problemMatcher();
  const match = new RegExp(pattern.regexp).exec(line);
  if (match === null) {
    return undefined;
  }
  return {
    file: match[pattern.file],
    line: match[pattern.line],
    column: match[pattern.column],
    severity: match[pattern.severity],
    code: match[pattern.code],
    message: match[pattern.message],
  };
}

test('the matcher resolves absolute paths and maps every field', () => {
  const matcher = problemMatcher();
  assert.equal(matcher.fileLocation, 'absolute');
  assert.equal(matcher.owner, 'stc');
  assert.equal(matcher.pattern.file, 1);
  assert.equal(matcher.pattern.line, 2);
  assert.equal(matcher.pattern.column, 3);
  assert.equal(matcher.pattern.severity, 4);
  assert.equal(matcher.pattern.code, 5);
  assert.equal(matcher.pattern.message, 6);
});

test('a short line with a Windows drive-letter path matches', () => {
  const result = apply(
    'C:\\repos\\a.st:5:14: error[STC-E-NAME-001]: `Missing` is not declared',
  );
  assert.deepEqual(result, {
    file: 'C:\\repos\\a.st',
    line: '5',
    column: '14',
    severity: 'error',
    code: 'STC-E-NAME-001',
    message: '`Missing` is not declared',
  });
});

test('a short line with a POSIX path matches', () => {
  const result = apply(
    '/home/user/app/src/main.st:12:3: warning[STC-W-LEX-008]: deprecated octal literal',
  );
  assert.deepEqual(result, {
    file: '/home/user/app/src/main.st',
    line: '12',
    column: '3',
    severity: 'warning',
    code: 'STC-W-LEX-008',
    message: 'deprecated octal literal',
  });
});

test('a non-ASCII message is preserved', () => {
  const result = apply(
    'C:\\repos\\ünïcode\\main.st:1:2: warning[STC-W-LEX-008]: ungültiges Zeichen „ü“',
  );
  assert.equal(result?.file, 'C:\\repos\\ünïcode\\main.st');
  assert.equal(result?.message, 'ungültiges Zeichen „ü“');
  assert.equal(result?.severity, 'warning');
  assert.equal(result?.code, 'STC-W-LEX-008');
});

test('the package-qualified display path also matches the line shape', () => {
  const result = apply('app/src/main.st:3:3: error[STC-E-NAME-001]: unknown name');
  assert.equal(result?.file, 'app/src/main.st');
  assert.equal(result?.line, '3');
});

test('JSON and human diagnostic lines do not match', () => {
  assert.equal(
    apply('{"code":"STC-E-NAME-001","severity":"error","message":"unknown name"}'),
    undefined,
  );
  assert.equal(apply('error[STC-E-NAME-001]: unknown name'), undefined);
  assert.equal(apply('STC-E-NAME-001: unknown name'), undefined);
});
