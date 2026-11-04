import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const packageJson = JSON.parse(readFileSync(path.join(root, 'package.json'), 'utf8'));
const grammar = JSON.parse(
  readFileSync(path.join(root, 'syntaxes', 'st.tmLanguage.json'), 'utf8'),
);
const readme = readFileSync(path.join(root, 'README.md'), 'utf8');

const overlays =
  packageJson.contributes?.configurationDefaults?.['editor.tokenColorCustomizations'] ?? {};

// The 1.82 engines-floor setting IDs are the acceptance targets. Later VS Code
// releases renamed the built-in themes and added the 2026 defaults, and only
// migrate `workbench.colorTheme`, not per-theme customization keys, so every
// spelling is contributed with the same rules.
const themeGroups = [
  {
    keys: ['[Default Dark Modern]', '[Dark Modern]', '[Dark 2026]'],
    themeFile: 'stc-dark-color-theme.json',
  },
  {
    keys: ['[Default Light Modern]', '[Light Modern]', '[Light 2026]'],
    themeFile: 'stc-light-color-theme.json',
  },
];

// Backgrounds are per theme: the modern themes use #1F1F1F, Dark 2026 uses
// #121314, and both light themes use #FFFFFF.
const overlayThemes = [
  {
    key: '[Default Dark Modern]',
    themeFile: 'stc-dark-color-theme.json',
    background: '#1F1F1F',
  },
  {
    key: '[Dark Modern]',
    themeFile: 'stc-dark-color-theme.json',
    background: '#1F1F1F',
  },
  {
    key: '[Dark 2026]',
    themeFile: 'stc-dark-color-theme.json',
    background: '#121314',
  },
  {
    key: '[Default Light Modern]',
    themeFile: 'stc-light-color-theme.json',
    background: '#FFFFFF',
  },
  {
    key: '[Light Modern]',
    themeFile: 'stc-light-color-theme.json',
    background: '#FFFFFF',
  },
  {
    key: '[Light 2026]',
    themeFile: 'stc-light-color-theme.json',
    background: '#FFFFFF',
  },
];

const requiredThemeKeys = ['[Default Dark Modern]', '[Default Light Modern]'];
const currentThemeKeys = [
  '[Dark Modern]',
  '[Light Modern]',
  '[Dark 2026]',
  '[Light 2026]',
];

function readTheme(fileName) {
  return JSON.parse(readFileSync(path.join(root, 'themes', fileName), 'utf8'));
}

function selectorMatches(scope, selector) {
  return scope === selector || scope.startsWith(`${selector}.`);
}

function resolveScope(rules, scope) {
  let best = null;
  let bestDepth = -1;

  for (const [index, rule] of (rules ?? []).entries()) {
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

function collectGrammarScopes(value, scopes) {
  if (Array.isArray(value)) {
    for (const item of value) {
      collectGrammarScopes(item, scopes);
    }
    return scopes;
  }

  if (value === null || typeof value !== 'object') {
    return scopes;
  }

  if (typeof value.name === 'string' && value.name.endsWith('.st')) {
    scopes.add(value.name);
  }

  for (const child of Object.values(value)) {
    collectGrammarScopes(child, scopes);
  }

  return scopes;
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
  controlKeyword: ['keyword.control.conditional.st'],
  declaration: ['keyword.declaration.st', 'storage.modifier.st'],
  function: ['entity.name.function.st'],
  type: ['entity.name.type.st', 'support.type.primitives.st'],
  variable: ['variable.name.st'],
  parameter: ['variable.parameter.st'],
  member: ['variable.other.member.st'],
  directVariable: ['variable.other.constant.st'],
  languageVariable: ['variable.language.st'],
  string: ['string.quoted.double.st', 'string.quoted.single.st'],
  number: ['constant.numeric.integer.st', 'constant.numeric.duration.st'],
  languageConstant: ['constant.language.st'],
  operator: ['keyword.operator.st', 'punctuation.st'],
  pragma: ['keyword.other.pragma.st'],
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

const grammarScopes = collectGrammarScopes(grammar, new Set());

function overlayRules(themeKey) {
  const contribution = overlays[themeKey];
  assert.ok(contribution, `missing overlay for ${themeKey}`);
  assert.ok(Array.isArray(contribution.textMateRules), `${themeKey} has no textMateRules`);
  return contribution.textMateRules;
}

test('the overlay is contributed as configuration defaults for both Modern theme IDs', () => {
  assert.equal(
    Object.keys(packageJson.contributes?.configurationDefaults ?? {}).join(','),
    'editor.tokenColorCustomizations',
    'configurationDefaults must only target editor.tokenColorCustomizations',
  );

  for (const themeKey of [...requiredThemeKeys, ...currentThemeKeys]) {
    const rules = overlayRules(themeKey);
    assert.ok(rules.length >= distinctCategories.length, `${themeKey} has too few rules`);
  }

  // VS Code renamed the built-in IDs and added the 2026 defaults; keep every
  // spelling of a theme in lockstep with the acceptance-target rules.
  for (const { keys } of themeGroups) {
    for (const key of keys.slice(1)) {
      assert.deepEqual(overlays[key], overlays[keys[0]], `${key} differs from ${keys[0]}`);
    }
  }

  assert.deepEqual(
    (packageJson.contributes?.themes ?? []).map((theme) => theme.label),
    ['stc Dark', 'stc Light'],
    'the contributed themes must remain the explicit alternatives',
  );

  const serialized = JSON.stringify(packageJson);
  assert.ok(
    !serialized.includes('workbench.colorTheme'),
    'the extension must not set workbench.colorTheme',
  );
});

for (const { key, themeFile, background } of overlayThemes) {
  test(`${key} only uses .st-scoped selectors that exist in the grammar`, () => {
    const rules = overlayRules(key);

    for (const rule of rules) {
      const selectors = typeof rule.scope === 'string' ? [rule.scope] : rule.scope;
      assert.ok(Array.isArray(selectors) && selectors.length > 0, `${key} rule has no scope`);

      for (const selector of selectors) {
        assert.match(selector, /\.st$/, `${key} selector ${selector} is not .st-scoped`);
        assert.ok(
          !/[,\s*]/.test(selector),
          `${key} selector ${selector} uses an unsupported TextMate selector form`,
        );
        assert.ok(
          [...grammarScopes].some((scope) => selectorMatches(scope, selector)),
          `${key} selector ${selector} matches no grammar scope`,
        );
      }

      assert.match(
        rule.settings?.foreground ?? '',
        /^#[0-9A-Fa-f]{6}$/,
        `${key} rule ${rule.name ?? ''} has no explicit foreground`,
      );
    }

    // An unqualified textMateRules array would apply to every language.
    for (const overlayKey of Object.keys(overlays)) {
      assert.match(overlayKey, /^\[.+\]$/, `overlay key ${overlayKey} is not theme-scoped`);
      assert.deepEqual(
        Object.keys(overlays[overlayKey]),
        ['textMateRules'],
        `${overlayKey} contributes more than textMateRules`,
      );
    }

    assert.ok(background);
  });

  test(`${key} reuses the shipped theme palette for every role`, () => {
    const theme = readTheme(themeFile);
    const rules = overlayRules(key);

    for (const [role, scopes] of Object.entries(roles)) {
      for (const scope of scopes) {
        const shipped = resolveScope(theme.tokenColors, scope);
        const overlay = resolveScope(rules, scope);
        assert.ok(shipped, `${role} scope ${scope} has no shipped theme rule`);
        assert.ok(overlay, `${role} scope ${scope} has no overlay rule`);
        assert.equal(
          overlay.rule.settings.foreground.toUpperCase(),
          shipped.rule.settings.foreground.toUpperCase(),
          `${role} scope ${scope} does not reuse the ${themeFile} palette`,
        );
      }
    }

    const comment = resolveScope(rules, 'comment.line.double-slash.st');
    assert.equal(comment.rule.settings.fontStyle, 'italic');
  });

  test(`${key} keeps the acceptance categories distinct and readable`, () => {
    const rules = overlayRules(key);
    const colors = new Set();

    for (const category of distinctCategories) {
      const resolved = resolveScope(rules, roles[category][0]);
      const foreground = resolved.rule.settings.foreground;
      colors.add(foreground.toUpperCase());
      assert.ok(
        contrast(foreground, background) >= 4.5,
        `${category} color ${foreground} is below the 4.5:1 contrast target`,
      );
    }

    assert.equal(colors.size, distinctCategories.length, 'categories share a foreground color');

    // Check every contributed selector, not only the category representatives.
    for (const rule of rules) {
      const selectors = typeof rule.scope === 'string' ? [rule.scope] : rule.scope;
      for (const selector of selectors) {
        assert.ok(
          contrast(rule.settings.foreground, background) >= 4.5,
          `${selector} color ${rule.settings.foreground} is below the 4.5:1 contrast target`,
        );
      }
    }
  });
}

// VS Code merges configuration layers with `ConfigurationModel.mergeContents`
// (src/vs/platform/configuration/common/configurationModels.ts): nested objects
// merge recursively, while arrays and scalars from the later layer replace the
// earlier value. The opt-out documentation depends on exactly this behavior.
function mergeContents(source, target) {
  for (const key of Object.keys(target)) {
    if (
      key in source &&
      typeof source[key] === 'object' &&
      source[key] !== null &&
      !Array.isArray(source[key]) &&
      typeof target[key] === 'object' &&
      target[key] !== null &&
      !Array.isArray(target[key])
    ) {
      mergeContents(source[key], target[key]);
    } else {
      source[key] = structuredClone(target[key]);
    }
  }
  return source;
}

test('the documented opt-out overrides the overlay despite configuration merging', () => {
  const defaults = { 'editor.tokenColorCustomizations': structuredClone(overlays) };

  const mergedEmpty = mergeContents(structuredClone(defaults), {
    'editor.tokenColorCustomizations': {},
  });
  assert.ok(
    mergedEmpty['editor.tokenColorCustomizations']['[Default Dark Modern]'].textMateRules.length >
      0,
    'an empty object must not be documented as an opt-out: it leaves the defaults in place',
  );

  const mergedOptOut = mergeContents(structuredClone(defaults), {
    'editor.tokenColorCustomizations': {
      '[Default Dark Modern]': { textMateRules: [] },
    },
  });
  assert.equal(
    mergedOptOut['editor.tokenColorCustomizations']['[Default Dark Modern]'].textMateRules.length,
    0,
    'an empty textMateRules array must replace the contributed rules',
  );
  assert.ok(
    mergedOptOut['editor.tokenColorCustomizations']['[Default Light Modern]'].textMateRules
      .length > 0,
    'opting out of one theme must leave the other theme overlay intact',
  );

  for (const themeKey of [...requiredThemeKeys, ...currentThemeKeys]) {
    assert.ok(
      readme.includes(themeKey),
      `README.md must document the overlay key ${themeKey}`,
    );
  }
  assert.ok(
    /"textMateRules":\s*\[\s*\]/.test(readme),
    'README.md must document the empty textMateRules opt-out',
  );
  assert.match(
    readme,
    /merges object settings across configuration scopes|does not remove\s+the\s+contributed/i,
    'README.md must explain why an empty object is not an opt-out',
  );
});
