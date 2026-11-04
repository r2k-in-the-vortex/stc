import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const packageJson = JSON.parse(readFileSync(path.join(root, 'package.json'), 'utf8'));

const themeContributions = packageJson.contributes?.themes ?? [];

function readTheme(fileName) {
  return JSON.parse(readFileSync(path.join(root, 'themes', fileName), 'utf8'));
}

function selectorMatches(scope, selector) {
  return scope === selector || scope.startsWith(`${selector}.`);
}

function resolveScope(theme, scope) {
  let best = null;
  let bestDepth = -1;

  for (const [index, rule] of (theme.tokenColors ?? []).entries()) {
    const selectors =
      typeof rule.scope === 'string' ? [rule.scope] : Array.isArray(rule.scope) ? rule.scope : [];

    for (const selector of selectors) {
      if (!selectorMatches(scope, selector)) {
        continue;
      }

      const depth = selector.split('.').length;
      if (depth >= bestDepth) {
        bestDepth = depth;
        best = { index, rule, selector };
      }
    }
  }

  return best;
}

function channel(value) {
  const normalized = value / 255;
  return normalized <= 0.03928
    ? normalized / 12.92
    : ((normalized + 0.055) / 1.055) ** 2.4;
}

function luminance(hex) {
  assert.match(hex, /^#[0-9A-Fa-f]{6}$/, `invalid color ${hex}`);
  const red = Number.parseInt(hex.slice(1, 3), 16);
  const green = Number.parseInt(hex.slice(3, 5), 16);
  const blue = Number.parseInt(hex.slice(5, 7), 16);
  return 0.2126 * channel(red) + 0.7152 * channel(green) + 0.0722 * channel(blue);
}

function contrast(foreground, background) {
  const lighter = Math.max(luminance(foreground), luminance(background));
  const darker = Math.min(luminance(foreground), luminance(background));
  return (lighter + 0.05) / (darker + 0.05);
}

const roles = {
  comment: ['comment.line.double-slash.st', 'comment.block.st'],
  controlKeyword: [
    'keyword.control.conditional.st',
    'keyword.control.loop.st',
    'keyword.control.flow.st',
  ],
  declaration: ['keyword.declaration.st', 'storage.modifier.st'],
  function: ['entity.name.function.st'],
  type: [
    'entity.name.type.st',
    'support.type.primitives.st',
    'entity.name.namespace.st',
    'entity.other.inherited-class.st',
    'support.class.st',
  ],
  variable: ['variable.name.st'],
  parameter: ['variable.parameter.st'],
  member: ['variable.other.member.st'],
  directVariable: ['variable.other.constant.st'],
  languageVariable: ['variable.language.st'],
  string: ['string.quoted.double.st', 'string.quoted.single.st'],
  number: ['constant.numeric.integer.st', 'constant.numeric.duration.st'],
  languageConstant: ['constant.language.st'],
  pragma: [
    'keyword.other.pragma.st',
    'punctuation.definition.pragma.begin.st',
    'punctuation.definition.pragma.end.st',
  ],
  operator: ['keyword.operator.st', 'punctuation.st'],
};

const distinctCategories = [
  'controlKeyword',
  'type',
  'declaration',
  'variable',
  'comment',
  'string',
  'number',
  'pragma',
  'directVariable',
];

const themes = [
  {
    label: 'stc Dark',
    fileName: 'stc-dark-color-theme.json',
    uiTheme: 'vs-dark',
    type: 'dark',
    background: '#1F1F1F',
    foreground: '#D4D4D4',
  },
  {
    label: 'stc Light',
    fileName: 'stc-light-color-theme.json',
    uiTheme: 'vs',
    type: 'light',
    background: '#FFFFFF',
    foreground: '#1F1F1F',
  },
];

test('both themes are contributed from the extension manifest', () => {
  for (const expected of themes) {
    const contribution = themeContributions.find((entry) => entry.label === expected.label);
    assert.ok(contribution, `missing theme contribution for ${expected.label}`);
    assert.equal(contribution.uiTheme, expected.uiTheme);
    assert.equal(contribution.path, `./themes/${expected.fileName}`);
    assert.doesNotThrow(() => readFileSync(path.join(root, 'themes', expected.fileName), 'utf8'));
  }
});

for (const expected of themes) {
  test(`${expected.label} defines the required editor colors and tokens`, () => {
    const theme = readTheme(expected.fileName);

    assert.equal(theme.name, expected.label);
    assert.equal(theme.type, expected.type);
    assert.equal(theme.semanticHighlighting, false);
    assert.equal(theme.colors['editor.background'], expected.background);
    assert.equal(theme.colors['editor.foreground'], expected.foreground);

    for (const key of [
      'editor.background',
      'editor.foreground',
      'editorLineNumber.foreground',
      'editor.selectionBackground',
      'editorCursor.foreground',
    ]) {
      assert.match(theme.colors[key], /^#[0-9A-Fa-f]{6}$/, `missing or invalid ${key}`);
    }

    for (const [role, scopes] of Object.entries(roles)) {
      for (const scope of scopes) {
        const resolved = resolveScope(theme, scope);
        assert.ok(resolved, `${role} scope ${scope} has no explicit theme rule`);
        assert.match(
          resolved.rule.settings?.foreground ?? '',
          /^#[0-9A-Fa-f]{6}$/,
          `${role} scope ${scope} has no explicit foreground`,
        );
      }
    }

    const commentRule = resolveScope(theme, 'comment.line.double-slash.st');
    assert.equal(commentRule.rule.settings.fontStyle, 'italic');
  });

  test(`${expected.label} keeps the acceptance categories distinct and readable`, () => {
    const theme = readTheme(expected.fileName);
    const colors = new Set();

    for (const category of distinctCategories) {
      const scope = roles[category][0];
      const resolved = resolveScope(theme, scope);
      const foreground = resolved.rule.settings.foreground;
      colors.add(foreground.toUpperCase());
      assert.ok(
        contrast(foreground, theme.colors['editor.background']) >= 4.5,
        `${category} color ${foreground} is below the 4.5:1 contrast target`,
      );
    }

    assert.equal(colors.size, distinctCategories.length, 'categories share a foreground color');

    for (const [role, scopes] of Object.entries(roles)) {
      for (const scope of scopes) {
        const foreground = resolveScope(theme, scope).rule.settings.foreground;
        assert.ok(
          contrast(foreground, theme.colors['editor.background']) >= 4.5,
          `${role} color ${foreground} is below the 4.5:1 contrast target`,
        );
      }
    }
  });
}
