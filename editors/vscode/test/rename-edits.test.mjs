import assert from 'node:assert/strict';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { build } from 'esbuild';

// `renameEdits` is pure TypeScript with no `vscode` import; bundle it in memory
// and exercise the same code the extension ships.
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

const module = await loadModule(join(root, 'src', 'renameEdits.ts'));
const { applyLspEdits, shouldUseTrackedRename, editedUriKind } = module;

function edit(startLine, startCharacter, endLine, endCharacter, newText) {
  return {
    range: {
      start: { line: startLine, character: startCharacter },
      end: { line: endLine, character: endCharacter },
    },
    newText,
  };
}

test('an empty edit list leaves the text unchanged', () => {
  const text = 'PROGRAM Main\r\nEND_PROGRAM\r\n';
  assert.equal(applyLspEdits(text, []), text);
});

test('applies a multi-line edit', () => {
  const text = 'line one\nline two\nline three';
  assert.equal(applyLspEdits(text, [edit(0, 5, 1, 8, 'X')]), 'line X\nline three');
});

test('applies multiple edits in one file in descending order', () => {
  const text = 'alpha beta gamma';
  // Deliberately pass the later edit first so the helper must reorder.
  const edits = [edit(0, 11, 0, 16, 'G'), edit(0, 0, 0, 5, 'A')];
  assert.equal(applyLspEdits(text, edits), 'A beta G');
});

test('applies edits across separate files', () => {
  const main = 'PROGRAM Main\nEND_PROGRAM\n';
  const smoke = 'FUNCTION Test : BOOL\nEND_FUNCTION\n';
  const mainEdit = [edit(0, 8, 0, 12, 'Advance')];
  const smokeEdit = [edit(0, 9, 0, 13, 'Check')];
  assert.equal(applyLspEdits(main, mainEdit), 'PROGRAM Advance\nEND_PROGRAM\n');
  assert.equal(applyLspEdits(smoke, smokeEdit), 'FUNCTION Check : BOOL\nEND_FUNCTION\n');
});

test('counts UTF-16 surrogate pairs as two characters', () => {
  const text = 'a\u{1F600}b';
  assert.equal(applyLspEdits(text, [edit(0, 1, 0, 3, 'x')]), 'axb');
  const second = '\u{1F600}X\u{1F600}';
  assert.equal(applyLspEdits(second, [edit(0, 2, 0, 3, 'Y')]), '\u{1F600}Y\u{1F600}');
});

test('keeps CRLF line endings and excludes the carriage return from a line', () => {
  const text = 'first\r\nsecond\r\nthird';
  // The edit starts inside the first line and ends inside the second; the
  // untouched CRLF pairs stay in place.
  assert.equal(applyLspEdits(text, [edit(0, 2, 1, 3, '!')]), 'fi!ond\r\nthird');
  // The end of a CRLF line is before the `\r`, so characters 1..3 are `bc`.
  assert.equal(applyLspEdits('abc\r\ndef', [edit(0, 1, 0, 3, 'Z')]), 'aZ\r\ndef');
  // The final line has no newline, so its content end is the text end.
  assert.equal(applyLspEdits('abc\r\ndef', [edit(1, 3, 1, 3, '!')]), 'abc\r\ndef!');
});

test('rejects overlapping edits', () => {
  assert.throws(
    () => applyLspEdits('abcdef', [edit(0, 0, 0, 3, 'x'), edit(0, 2, 0, 4, 'y')]),
    /overlap/,
  );
});

test('rejects out-of-range positions', () => {
  assert.throws(() => applyLspEdits('abc', [edit(2, 0, 2, 0, 'x')]), /outside the file/);
  assert.throws(() => applyLspEdits('abc', [edit(0, 4, 0, 4, 'x')]), /outside line/);
  assert.throws(() => applyLspEdits('abc', [edit(0, 3, 0, 2, 'x')]), /starts after it ends/);
});

test('detects the tracked rename capability', () => {
  const capability = (trackedRename) => ({
    capabilities: { experimental: { stc: { trackedRename } } },
  });
  assert.equal(shouldUseTrackedRename(capability(true)), true);
  assert.equal(shouldUseTrackedRename(capability(false)), false);
  assert.equal(shouldUseTrackedRename({ capabilities: {} }), false);
  assert.equal(shouldUseTrackedRename({ capabilities: { experimental: {} } }), false);
  assert.equal(shouldUseTrackedRename({ capabilities: { experimental: { stc: null } } }), false);
  assert.equal(shouldUseTrackedRename(undefined), false);
  assert.equal(shouldUseTrackedRename(null), false);
});

test('classifies source, manifest, and profile URIs', () => {
  const manifestPath = 'C:\\repo\\stc.toml';
  assert.equal(editedUriKind('file:///C:/repo/src/main.st', manifestPath), 'source');
  assert.equal(editedUriKind('file:///C:/repo/src/main.ST', manifestPath), 'source');
  assert.equal(editedUriKind('file:///C:/repo/stc.toml', manifestPath), 'manifest');
  assert.equal(editedUriKind('file:///C:/repo/io/bench.toml', manifestPath), 'profile');
});

test('classifies an encoded manifest path', () => {
  const manifestPath = 'C:\\my repo\\stc.toml';
  assert.equal(
    editedUriKind('file:///C:/my%20repo/stc.toml', manifestPath),
    'manifest',
  );
  assert.equal(
    editedUriKind('file:///C:/my%20repo/io/bench.toml', manifestPath),
    'profile',
  );
});

test('matches the manifest path case-insensitively on Windows', () => {
  if (process.platform !== 'win32') {
    return;
  }
  assert.equal(
    editedUriKind('file:///c:/REPO/STC.TOML', 'C:\\repo\\stc.toml'),
    'manifest',
  );
});
