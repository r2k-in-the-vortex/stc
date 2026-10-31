# WL-1.1 F-1 Sub-Plan — Highlighting Scopes and Color Scheme

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-31  
Branch: `WL-1.1` (sub-plan of `v2.0.0-planning`)  
Status: Planning (2026-10-31); decisions are recommendations pending owner review.  
Parent: [WL-1.1-Plan.md](WL-1.1-Plan.md) §12.1 F-1  
Inputs: [WL-1.1-Plan.md](WL-1.1-Plan.md) §1–§2, §8, §12.1;
[WL-1.1-UI-CHECKLIST.md](WL-1.1-UI-CHECKLIST.md) item 1 and F-1;
[Tokenization.md](specification/Syntax/Tokenization.md);
[Grammar.md](specification/Syntax/Grammar.md);
[IDE-Integration.md](specification/Architecture/IDE-Integration.md);
[st.tmLanguage.json](editors/vscode/syntaxes/st.tmLanguage.json);
[package.json](editors/vscode/package.json);
[grammar_coverage.rs](crates/stc-syntax/tests/grammar_coverage.rs);
[Serhioromano/vscode-st](https://github.com/Serhioromano/vscode-st) pinned at
`4178d989f74a9986359db0f65b3b0e1f6721d4d7`; VS Code 1.144.0 local theme defaults.

This plan is the only F-1 deliverable created during planning. No grammar, theme, extension, test,
CI, or specification file has been modified.

## 1. Objective and scope

F-1 closes the one highlighting gap in WL-1.1: the current grammar scopes the tested tokens
correctly, but declared POU/type names, type references, variables, parameters, members, and
built-in functions have no semantic scopes, so most source text renders in the theme foreground.
The result was recorded in checklist item 1 as a palette "dominated by red and white."

The F-1 acceptance is:

> Enrich the grammar with standard semantic scopes (declarations, types, variables, parameters,
> control keywords, built-ins) using [Serhioromano/vscode-st](https://github.com/Serhioromano/vscode-st)
> as a reference, and ship a small theme or document a recommended one; verify distinct
> keyword/type/declaration/variable/comment/string/number/pragma/direct-variable colors in a
> standard dark and light theme.

This plan reads "a standard dark and light theme" as conventional VS Code color-theme files
(`uiTheme: "vs-dark"` and `uiTheme: "vs"`), not necessarily the built-in themes. The recommended
deliverable ships two small token themes so the nine acceptance categories can be made distinct in
both modes; the built-in Dark Modern/Light Modern themes are also verified for scope compatibility.
Owner question F1-Q1 asks the owner to confirm this reading.

In scope:

- Standard TextMate semantic scopes for declarations, type references, primitives, variables,
  parameters, members, direct variables, control keywords, modifiers, built-ins, and pragmas.
- A conservative identifier fallback so remaining identifiers are colored rather than left at the
  theme foreground.
- Two optional bundled themes (`stc Dark`, `stc Light`) and their manifest contribution.
- Grammar fixtures, a theme test, coverage checks against the lexer and prelude tables, and the
  manual Inspect Editor Tokens and Scopes matrix.
- Documentation, checklist evidence, and VSIX/CI content updates.

Out of scope:

- A semantic-token provider or any LSP capability change (I-7). Highlighting remains TextMate-only.
- A language-id, scope-name, or file-extension change (I-16): `st`, `source.st`, `.st`.
- Compiler, runtime, artifact-format, ABI, or language-specification changes. Built-in names are
  read from the existing prelude; they are not redefined.
- Rename, code actions, inlay hints, call hierarchy, workspace symbols, or other I-7 features.
- A full workbench theme, marketplace publication, high-contrast themes, or remote IDE access.
- Copying the reference grammar or any third-party theme without the attribution required by its
  license.

| Constraint | Effect on F-1 |
| --- | --- |
| I-7: closed LSP capability set, no semantic tokens | Declaration/use distinctions are lexical heuristics. Uses cannot be resolved; the plan documents the ambiguity. |
| I-16: language id `st`, TextMate scope `source.st`, `.st` only | Scope names keep the `.st` suffix; the language id and root scope do not change. |
| I-13: grammar coverage stays green | Every keyword and punctuation spelling remains present; new scope-name checks are added without weakening the existing check. |
| VS Code engine `^1.82.0` | The theme contribution schema and Inspect Editor Tokens and Scopes command are available. |
| Local-only, VSIX-only v2.0.0 | Themes ship as extension assets; no marketplace or external download. |
| Reference use must respect its license | Serhioromano/vscode-st is MIT and is used as a reference only; see F1-D12. |

## 2. Current state and findings

### 2.1 Grammar inventory

[`editors/vscode/syntaxes/st.tmLanguage.json`](editors/vscode/syntaxes/st.tmLanguage.json) is a
397-line JSON grammar with `scopeName: "source.st"`. Its top-level patterns are listed at lines
7–56. The existing scopes are:

| Area | Current scope | Lines | Finding |
| --- | --- | --- | --- |
| Comments | `comment.line.documentation.st`, `comment.line.double-slash.st`, `comment.block.st`; begin/end punctuation | 58–121 | Correct and standard. |
| Pragmas | `meta.pragma.st`; braces `punctuation.definition.pragma.begin/end.st`; payload word `variable.parameter.pragma.st`; pragma number `constant.numeric.pragma.st`; nested `meta.pragma.payload.st` | 122–190 | The outer region is scoped, but the pragma name (`test`, `io`, `deprecated`, …) is indistinguishable from payload words. |
| Strings | `string.quoted.single/double.st`; typed prefix `storage.type.string.st`; escape `constant.character.escape.st` | 192–301 | Correct and standard. |
| Character and numeric literals | `constant.character.st`; `constant.numeric.duration.st`, `.datetime.st`, `.date.st`, `.time.st`, `.typed.st`, `.based.st`, `.real.st`, `.integer.st` | 302–363 | Correct and standard. |
| Direct variables | `variable.other.direct.st` | 364–367 | Correct category, but the custom `.direct` suffix is not the standard constant-like scope. |
| Primitive types | `storage.type.st` for the full primitive alternation | 368–370 | Standard prefix, but F-1 requires a primitive-specific type scope. |
| Declaration keywords | `keyword.declaration.st` for types, POU kinds, variable sections, OOP/config/SFC keywords | 372–374 | Correct, but the declared name after the keyword is not captured. |
| Control keywords | flat `keyword.control.st` for all conditional, loop, and flow keywords | 376–378 | Correct prefix, but no control subfamilies. |
| Word operators | `keyword.operator.st` for `AND`, `OR`, `XOR`, `NOT`, `MOD`, `ADR`, `SIZEOF` | 380–382 | Correct. |
| Language constants | `constant.language.st` for `TRUE`, `FALSE` | 384–386 | Correct; `NULL` is currently classified as a declaration keyword. |
| Symbolic operators | `keyword.operator.st` for `:=`, `=>`, `?=`, `**`, `..`, `<>`, `<=`, `>=`, `^`, `+`, `-`, `*`, `/`, `=`, `<`, `>`, `&` | 388–390 | Correct. |
| Punctuation | `punctuation.st` for `( ) [ ] ; , : . # @ %` | 392–394 | Correct. |

Missing from the grammar:

- Captures for declared `FUNCTION`, `METHOD`, `PROGRAM`, `FUNCTION_BLOCK`, `CLASS`, `INTERFACE`,
  `TYPE`, `CONFIGURATION`, `RESOURCE`, and `NAMESPACE` names.
- Type-reference scopes after `:`, after `ARRAY ... OF`, after `REF_TO`/`POINTER TO`, and after
  `EXTENDS`/`IMPLEMENTS`.
- Scopes for variable declarations, `VAR_INPUT`/`VAR_OUTPUT`/`VAR_IN_OUT` parameters, struct
  fields, member access after `.`, `THIS`/`SUPER`, and enum values.
- Built-in function and standard function-block scopes.
- A pragma-name scope distinct from payload words.
- A general identifier rule, so uncaptured identifiers fall through to the theme foreground.

### 2.2 Why the palette reads as red and white

Checklist item 1 (2026-10-30) inspected `keyword.declaration.st` on `VAR_GLOBAL`,
`constant.language.st` on `TRUE`, and `string.quoted.double.st` on a string. Types, comments,
numbers/durations, pragmas, direct variables, and operators were also scoped. The conclusion was
that the palette was "dominated by red and white with the current theme," recorded as F-1 rather
than a manual-pass failure.

The grammar itself assigns no colors. The symptom follows from the scope gaps above: strings take a
red/salmon color, declaration and type keywords take a blue color, and every uncaptured identifier
(declared names, type names, variables, parameters, members, built-ins) falls back to the theme
foreground, which is near-white on a dark theme. The active theme at inspection time was not
recorded, so F-1 must be verified both with the built-in themes and with the themes shipped by the
extension.

### 2.3 Theme contribution is absent

- [`editors/vscode/package.json`](editors/vscode/package.json) contributes `languages` (lines
  27–40), one `grammars` entry with `scopeName: "source.st"` (lines 41–47), breakpoints,
  debuggers, tasks, and settings. It has no `contributes.themes` array. There is no
  `editors/vscode/themes/` directory.
- [`editors/vscode/.vscodeignore`](editors/vscode/.vscodeignore) excludes `.vscode/**`, `src/**`,
  `test/**`, `samples/**`, `node_modules/**`, build files, the lockfile, and `*.vsix`. A
  `themes/` directory would be packaged unless it is explicitly excluded.
- [`editors/vscode/README.md`](editors/vscode/README.md) lists grammar features at lines 13–18 and
  says at lines 54–57 that highlighting comes from the grammar; it has no theme section.
- [`specification/Architecture/IDE-Integration.md`](specification/Architecture/IDE-Integration.md)
  states at line 121 that "Highlighting comes from the TextMate grammar, not from semantic
  tokens" and at lines 257–260 that the VSIX contains the manifest, bundled extension, grammar,
  language configuration, and README. It has no theme contribution.
- The CI VSIX content check (`.github/workflows/ci.yml`, lines 778–812) requires those five files
  and does not require a theme. It also does not exclude `extension/themes/`.
- The reference extension contributes no theme either; F-1's theme half is therefore a new
  decision, not a port.

### 2.4 Standard-theme evidence

The local VS Code 1.144.0 test install (`editors/vscode/.vscode-test/vscode-win32-x64-archive-1.144.0`) bundles
Dark Modern, which includes the Dark+ token colors, and Light Modern, which includes Light+. One
Dark was checked from the MIT `akamud/vscode-theme-onedark` theme at commit `a8be970` (2023-10-07).
The relevant scope families render as follows:

| Selector family | Dark+ / Dark Modern | Light+ / Light Modern | One Dark family |
| --- | --- | --- | --- |
| `comment` | `#6A9955` | `#008000` | `#5C6370` italic |
| `string` | `#CE9178` | `#A31515` | `#98C379` |
| `constant.numeric` | `#B5CEA8` | `#098658` | `#D19A66` |
| `keyword.control` | `#C586C0` | `#AF00DB` | `#C678DD` |
| `keyword` / `storage` | `#569CD6` | `#0000FF` | `#C678DD` |
| `entity.name.function` | `#DCDCAA` | `#795E26` | `#61AFEF` |
| `support.function` | `#DCDCAA` | `#795E26` | `#56B6C2` |
| `entity.name.type` / `support.type` | `#4EC9B0` | `#267F99` | `#E5C07B` / `#56B6C2` |
| `variable` | `#9CDCFE` | `#001080` | `#E06C75` |
| `variable.parameter` | inherits `variable` | inherits `variable` | `#ABB2BF` |
| `variable.other.constant` | `#4FC1FF` | `#0070C1` | no rule; inherits `variable` |
| `meta.pragma` / pragma name | no dedicated rule | no dedicated rule | no dedicated rule |

Consequences:

- Declaration names, type names, functions, and variables can be made distinct in all three theme
  families with standard scopes.
- Parameters and members do not separate from variables in Dark+/Light+; a shipped theme can make
  them distinct, but F-1 only requires a distinct "variable" category, so this is a quality gain,
  not a blocker.
- A pragma name has no dedicated color in any standard theme. A pragma-specific scope gives a
  theme something to target, but the built-in themes will render it as a keyword unless the user
  adds token customizations.
- Direct variables can use `variable.other.constant`, which is distinct in Dark+/Light+ but not in
  the One Dark family.
- Therefore a grammar-only change plus a documented third-party theme cannot guarantee every
  F-1 category in both modes. The two shipped themes guarantee it; the built-in themes remain
  supported fallbacks with the gaps above documented.

### 2.5 Coverage checks and fixtures

[`crates/stc-syntax/tests/grammar_coverage.rs`](crates/stc-syntax/tests/grammar_coverage.rs) is the
I-13 drift check:

- `collect_patterns` (lines 24–45) walks the grammar JSON and collects every `match`, `begin`, and
  `end` regex.
- `grammar_covers_lexer_tables` (lines 76–109) asserts `scopeName == "source.st"`, checks every
  `Keyword::ALL` spelling with a whole-alternative helper (lines 53–61), and checks every
  `Punct::ALL` spelling with an escaped-punctuation helper (lines 65–74).
- It checks that spellings exist somewhere in the grammar. It does not check which `name` a
  pattern carries, so moving a keyword from one rule to another does not break it.

[`editors/vscode/package.json`](editors/vscode/package.json) line 275 runs:

```text
node --test test/*.test.mjs && vscode-tmgrammar-test -g syntaxes/st.tmLanguage.json "test/grammar/*.st"
```

The grammar fixtures are:

| Fixture | Current assertions |
| --- | --- |
| `test/grammar/keywords.st` | POU/config/declaration keywords → `keyword.declaration.st`; `DINT`, `USTRING`, `ANY_DERIVED` → `storage.type.st`; `IF`/`THEN`/`END_IF` → `keyword.control.st`; word operators → `keyword.operator.st`; `TRUE`/`FALSE` → `constant.language.st`. |
| `test/grammar/direct-variables.st` | `%IX0.0`, `%QW28`, `%MD100`, `%I*`, `%B0`, `%X0` → `variable.other.direct.st`. |
| `test/grammar/pragmas.st` | The outer line → `meta.pragma.st` only. |
| `test/grammar/comments.st`, `strings.st`, `numbers.st`, `literals.st`, `operators.st` | Existing standard scopes; no declaration or identifier scopes. |

There is no fixture for declaration names, type references, variables, parameters, members,
built-ins, or the config `PROGRAM` shape. There is no test that the primitive alternation uses a
primitive-specific scope, no test that the grammar's built-in lists match the prelude, and no theme
test.

### 2.6 Sample workspace and specifications

- [`editors/vscode/samples/workspace/src/main.st`](editors/vscode/samples/workspace/src/main.st)
  contains every declaration shape needed for the manual matrix: `VAR_GLOBAL` lines 6–10,
  `FUNCTION AddStep : DINT` lines 12–21, `FUNCTION_BLOCK Accumulator` lines 23–31, `PROGRAM Main`
  lines 33–45, and the config `PROGRAM sample WITH MainTask : Main;` at line 50. Member access is
  at lines 40–42 (`accumulator.increment`, `accumulator.total`).
- [`editors/vscode/samples/workspace/tests/smoke.st`](editors/vscode/samples/workspace/tests/smoke.st)
  contains test functions and `STC_ASSERT` calls, plus a named argument call at line 19.
- [Tokenization.md](specification/Syntax/Tokenization.md) §3.1 defines the primitive type keywords,
  §3.2–§3.8 define declarations, POU/OOP, statements/operators, configuration, SFC, and pointer
  keywords; §6 defines direct variables; §8 defines `{stc:<name> ...}` pragmas and the known
  names `test`, `export`, `no_mangle`, `inline`, `deprecated`, `io`.
- [Grammar.md](specification/Syntax/Grammar.md) §5 defines variable sections and declarations, §6
  defines the POU/OOP declarations and their modifier order, and §7 defines configuration/resource/
  task/program-instance syntax.
- [IDE-Integration.md](specification/Architecture/IDE-Integration.md) line 71 records the language
  id and scope; line 121 records TextMate-only highlighting; lines 272–274 record the extension
  checks and the Rust grammar coverage test.
- [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) line 73 records that the LSP capability set has no
  semantic tokens and that highlighting comes from the TextMate grammar.
- [RELEASE-CHECKLIST.md](RELEASE-CHECKLIST.md) §7 lines 126–153 records the extension gate,
  including VSIX contents and the manual in-editor pass.

### 2.7 Reference extension observations

[Serhioromano/vscode-st](https://github.com/Serhioromano/vscode-st) at
`4178d989f74a9986359db0f65b3b0e1f6721d4d7` (fetched 2026-10-31):

- `package.json` has `license: "MIT"`, no `contributes.themes`, and a `source.st` grammar for
  language `st`; its README has no theme recommendation.
- `LICENSE` is the MIT License, "Copyright (c) 2018 Sergey Romanov".
- The grammar uses `storage.type.class.st` for POU headers, `entity.name.function.st` for declared
  POU names, `entity.name.type.st` for type regions, `support.type.primitives.st` for primitives,
  `variable.name.st` for declared variables, `variable.parameter.st` for members, and
  `variable.language.st` for `THIS`/`SUPER`.
- It uses `support.function.st` for a conversion/built-in list and a generic
  "identifier before `(`" rule that scopes user calls as `entity.name.function.st`; the latter is
  an aggressive lexical heuristic this plan rejects (F1-D5, F1-Q3).
- It scopes direct variables as `variable.other.constant` without the `.st` suffix, uses
  `storage.modifier.st` for modifiers, and has no ENUM-specific rule.

The reference is useful evidence for scope names and broad shape. It is not copied: its language
surface includes CODESYS/GX Works conversion names, it has no ENUM-specific rule, and its
member/call choices are not this plan's recommendations.

## 3. Decisions

The decisions below are recommendations. They become accepted when the owner answers the questions
in §4; implementation should start with the grammar decisions that do not depend on the theme
answer.

| ID | Decision | Recommendation | Status |
| --- | --- | --- | --- |
| F1-D1 | Theme deliverable | Grammar plus two optional bundled themes (`stc Dark`, `stc Light`), with Dark Modern/Light Modern documented as fallback themes | Recommended |
| F1-D2 | Scope taxonomy and naming | Standard TextMate scope roots with the `.st` suffix; no custom-only top-level scopes | Recommended |
| F1-D3 | POU and type declaration names | Line-shape rules after declaration keywords; a specific config-`PROGRAM` rule; no POU-wide regions | Recommended |
| F1-D4 | Type references and primitives | Primitive-specific scope plus bounded type-reference rules in var/struct/type contexts; inherited-class scope after `EXTENDS`/`IMPLEMENTS` | Recommended |
| F1-D5 | Variables, parameters, members, and uses | Section-based parameter/variable scopes, member scope after `.`, conservative identifier fallback | Recommended |
| F1-D6 | Control keywords, modifiers, constants, operators | Conditional/loop/flow subscopes; modifiers as `storage.modifier.st`; `NULL` as a language constant; keep operator scopes | Recommended |
| F1-D7 | Built-in functions and standard FBs | `support.function.st` for prelude functions called with `(`; `support.class.st` for `STANDARD_FBS`; grammar-local lists with a prelude drift test | Recommended |
| F1-D8 | Pragmas, direct variables, comments, strings, numbers | Pragma name `keyword.other.pragma.st`; direct variables `variable.other.constant.st`; keep existing literal scopes | Recommended |
| F1-D9 | Theme files, palette, accessibility | Two minimal token themes with all acceptance categories distinct and at least 4.5:1 contrast; no automatic theme selection | Recommended |
| F1-D10 | Coverage checks and fixtures | Keep the keyword/punctuation check; add primitive and prelude drift checks, scope fixtures, and a theme test | Recommended |
| F1-D11 | Verification and CI | Deterministic grammar/theme tests plus a manual Inspect Editor Tokens and Scopes matrix; extend the VSIX content check if themes ship | Recommended |
| F1-D12 | Reference use and licensing | Reference-only use; no copy; MIT attribution and a third-party notice only if any pattern/color is copied | Recommended |
| F1-D13 | Scope boundaries | No semantic tokens, LSP/DAP changes, language-id change, or compiler/runtime change | Recommended |

### F1-D1 — Theme deliverable

**Recommendation.** Enrich the grammar and ship two small optional color themes, `stc Dark` and
`stc Light`, contributed by the extension. Document Dark Modern and Light Modern as supported
fallback themes and Dark+/Light+ and the One Dark family as scope-compatible. The extension does
not select a theme automatically; the user picks one or keeps their current theme.

The grammar work is independent of the theme answer and gives richer colors under every theme.
The two theme files are token themes only: they define editor background/foreground and token
colors, while `uiTheme` inherits the rest of the workbench chrome. They are not a full workbench
theme.

**Rationale.**

- The acceptance requires distinct colors across nine categories in both dark and light. The
  standard-theme evidence in §2.4 shows that a pragma name has no dedicated color in Dark+,
  Light+, or One Dark, and direct variables are not distinct in One Dark. Parameters and members
  also collapse in Dark+/Light+. A grammar-only change cannot close those gaps.
- A documented third-party theme would make acceptance depend on the user's installed theme and
  its version; the extension cannot test rendered colors.
- Two small JSON assets preserve the standard scopes, so users who never select `stc Dark`/`stc
  Light` still get the improved function/type/variable colors from their current theme.
- The themes are conventional VS Code themes (`uiTheme: "vs-dark"`/`"vs"`), which is the plan's
  reading of "standard dark and light theme."

**Alternatives considered.**

- Grammar plus a documented recommended theme only: smallest change, but F-1's pragma and direct
  variable distinctness would be unverifiable and theme-version-dependent.
- Grammar only: fails the theme half of F-1 and leaves the pragma keyword-colored.
- Document an `editor.tokenColorCustomizations` snippet for Dark Modern/Light Modern: no theme
  files, but every user must paste settings and acceptance remains settings-dependent.
- One theme derived from Dark+ only: no light variant; duplicates an upstream theme and raises
  licensing/maintenance questions.
- A full workbench theme: out of scope and high maintenance.

The owner decision is F1-Q1.

### F1-D2 — Scope taxonomy and naming

**Recommendation.** Use standard TextMate scope roots, suffixed with `.st` to keep the language
identity. Do not introduce a custom-only top-level namespace such as `stc.*`, because no theme
matches it. The taxonomy is:

| Role | Scope |
| --- | --- |
| Block/line/doc comment | `comment.block.st`, `comment.line.double-slash.st`, `comment.line.documentation.st` |
| String | `string.quoted.single.st`, `string.quoted.double.st`; escape `constant.character.escape.st` |
| Number, duration, date/time | `constant.numeric.integer.st`, `.real.st`, `.based.st`, `.typed.st`, `.duration.st`, `.date.st`, `.time.st`, `.datetime.st` |
| Character literal | `constant.character.st` |
| Declaration keyword | `keyword.declaration.st` |
| Control keyword | `keyword.control.conditional.st`, `keyword.control.loop.st`, `keyword.control.flow.st` |
| Modifier | `storage.modifier.st` |
| Operator | `keyword.operator.st`, `keyword.operator.type.st` |
| Language constant | `constant.language.st` |
| Primitive type | `support.type.primitives.st` |
| Function/method/program declaration name | `entity.name.function.st` |
| Type/FB/class/interface/type-alias name | `entity.name.type.st` |
| Namespace/config/resource name | `entity.name.namespace.st` |
| Extended/implemented type name | `entity.other.inherited-class.st` |
| Variable | `variable.name.st` |
| Parameter | `variable.parameter.st` |
| Member / struct field | `variable.other.member.st` |
| Direct variable | `variable.other.constant.st` |
| Enum member (optional) | `variable.other.enummember.st` |
| `THIS`/`SUPER` | `variable.language.st` |
| Built-in function | `support.function.st` |
| Standard function block | `support.class.st` |
| Pragma region / name / braces | `meta.pragma.st`, `keyword.other.pragma.st`, `punctuation.definition.pragma.begin/end.st` |

Compatibility target: Dark Modern/Light Modern (the current VS Code defaults), Dark+/Light+, and the
One Dark family. High-contrast themes are not a target; `.st` scopes remain standard, so
high-contrast themes can still color the standard families.

Use generic theme selectors in the shipped themes (for example `comment`, `keyword.control`,
`entity.name.type`, `support.function`) so the theme also gives reasonable colors to other
languages, with `.st` selectors only for the ST-specific pragma roles. Treat these scope names as
user-facing API: document them, and record any later rename as a breaking theme change.

**Implementation ordering.** The top-level `patterns` and the region contents must be ordered so a
specific capture wins before a generic keyword, punctuation, or fallback rule:

1. comments; 2. pragmas; 3. strings and typed-character literals; 4. duration/date/time/typed
literals; 5. numbers; 6. direct variables; 7. the config `PROGRAM ... : type` line; 8. POU
declaration lines; 9. namespace/config/resource/using lines; 10. primitive types; 11.
`VAR_INPUT`/`VAR_OUTPUT`/`VAR_IN_OUT` regions; 12. other `VAR_*` regions; 13. `TYPE` and `STRUCT`
regions; 14. the `CASE` region's `OF` override; 15. `POINTER TO`; 16. declaration keywords; 17.
modifiers; 18. conditional/loop/flow keywords; 19. word operators; 20. language constants; 21.
symbolic operators; 22. member access after `.`; 23. punctuation; 24. identifier fallback.

Primitive-specific type rules must precede generic type-reference rules, and the member-access rule
must precede punctuation and the fallback.

**Rationale.** Standard prefixes are what themes match; the `.st` suffix preserves the language
identity and avoids coloring the same names in other languages. A custom `stc.*` root would require
every theme to add custom rules, which is exactly the theme-dependence F-1 is trying to remove.

**Alternatives considered.** Reference-exact names (`storage.type.class.st`, `keyword.other.storage`
and direct variables without the suffix) are less standard and not matched by all target themes.
Custom-only scopes are rejected. Semantic tokens would solve the use ambiguity but are out of scope
under I-7.

### F1-D3 — POU and type declaration names

**Recommendation.** Add line-shape rules that capture the declared name immediately after the
declaration keyword. Do not create POU begin/end regions for name coloring; the only special POU
shape is the configuration `PROGRAM` line.

| Shape | Captures |
| --- | --- |
| `FUNCTION name ( : type )?` | `name` → `entity.name.function.st`; `FUNCTION` → `keyword.declaration.st`; a simple return type → primitive or `entity.name.type.st` |
| `METHOD ( access_spec )? ( FINAL\|ABSTRACT )? ( OVERRIDE )? name ( : type )?` | modifiers → `storage.modifier.st`; `name` → `entity.name.function.st`; simple return type → primitive or type |
| `PROGRAM name` without `:` | `name` → `entity.name.function.st` |
| `PROGRAM ( RETAIN\|NON_RETAIN )? instance ( WITH task )? : type` | `instance`/`task` → `variable.name.st`; `type` → primitive or `entity.name.type.st`; keywords → `keyword.declaration.st` |
| `FUNCTION_BLOCK ( FINAL\|ABSTRACT )? name` | `name` → `entity.name.type.st` |
| `CLASS ( FINAL\|ABSTRACT )? name` | `name` → `entity.name.type.st` |
| `INTERFACE name` | `name` → `entity.name.type.st` |
| `TYPE alias : …` | `alias` → `entity.name.type.st`; the alias rule applies inside a `TYPE ... END_TYPE` region to the identifier immediately before `:` at the alias level, not to struct fields or enum members |
| `CONFIGURATION name`, `NAMESPACE name` | `name` → `entity.name.namespace.st` |
| `RESOURCE name ON target` | both names → `entity.name.namespace.st` |
| `USING qualified.name` | qualified name segments → `entity.name.namespace.st` |
| `ACTION name` (reserved SFC surface) | `name` → `entity.name.function.st` |
| `TASK name(...)` | `name` falls back to `variable.name.st`; `TASK`, `INTERVAL`, `PRIORITY` remain declaration keywords |

The config-`PROGRAM` rule must precede the plain `PROGRAM` rule. It is selected by the colon and
the optional `WITH`, so it does not need a `CONFIGURATION` region. The plain rule is used for
`PROGRAM Main` declarations and does not capture a config instance as a function.

`FUNCTION`/`METHOD` return types: capture the simple `: type_spec` shape in the same line rule. A
multi-line or compound return type (an inline `ARRAY`/`STRUCT` return type) may leave some type
tokens at the fallback color; this is a documented TextMate limit, not a semantic claim.

**Rationale.** A declaration name is the highest-confidence lexical signal in ST. Line anchoring
avoids consuming the POU body, keeps the grammar state small, and keeps `END_FUNCTION`/`END_CLASS`
as declaration keywords. The config `PROGRAM sample WITH MainTask : Main;` in the sample is the
known trap: a naive "word after PROGRAM" rule would color `sample` as a function, so the specific
rule is required.

**Alternatives considered.** POU begin/end regions with a backreference would allow the declaration
name to be recognized elsewhere in the body, but they add state, interact badly with nested POUs in
malformed files, and are not needed to color declarations. A global "identifier after a declaration
keyword" rule would miscolor the config instance and `END_*` keywords. Not coloring declarations
fails F-1.

### F1-D4 — Type references and primitives

**Recommendation.**

- Move the primitive alternation from `storage.type.st` to `support.type.primitives.st`, preserving
  the full list in Tokenization §3.1. Primitive rules are placed before generic type-reference
  rules so `DINT` after a colon remains a primitive.
- Scope a declared type reference after `:` as `entity.name.type.st` inside `VAR_*`, `TYPE`, and
  `STRUCT` regions, and in a POU header's simple return type. There is no global `:\s*ident` rule;
  that would miscolor CASE labels, `VAR_CONFIG` paths, and other colon uses.
- Scope the type name after `ARRAY ... OF`, `REF_TO`, and `POINTER TO` as `entity.name.type.st`;
  the primitive-specific rule wins for primitive element types.
- Scope names after `EXTENDS` and `IMPLEMENTS` as `entity.other.inherited-class.st`; accept a
  comma-separated interface list.
- Support qualified type names with up to four dot-separated segments as `entity.name.type.st`.
  Longer qualified names degrade to the member color for the tail; document the limit.
- `OF` in an array type is `keyword.operator.type.st`; `OF` in `CASE` is
  `keyword.control.conditional.st` (F1-D6). `TO` in `POINTER TO` is `keyword.operator.type.st`;
  `TO` in `FOR` is `keyword.control.loop.st`.
- Struct field types follow the same rules; the field name itself is a member (F1-D5).

**Rationale.** Type references after `:` and after type operators are lexical and common. Bounding
them to declaration regions prevents the known miscolors. Primitives need their own scope because
F-1 requires a distinct type color and because `storage.type` is already overloaded by declaration
keywords and typed string prefixes.

**Alternatives considered.** A global colon rule would color CASE labels, `VAR_CONFIG` paths, and
other colon uses as types. Scoping every identifier as a type would be wrong. Leaving type
references unscoped keeps types and variables the same color and fails F-1.

### F1-D5 — Variables, parameters, members, and uses

**Recommendation.**

- `VAR_INPUT`, `VAR_OUTPUT`, and `VAR_IN_OUT` sections: declared identifiers → `variable.parameter.st`.
- `VAR`, `VAR_TEMP`, `VAR_GLOBAL`, `VAR_EXTERNAL`, and `VAR_ACCESS` sections: declared
  identifiers → `variable.name.st`. This includes class/FB internal `VAR` sections. `VAR_CONFIG`
  entries receive the fallback `variable.name.st` unless a more specific member rule applies.
- `STRUCT ... END_STRUCT` fields → `variable.other.member.st`.
- Member access after `.` → `variable.other.member.st` (for example `accumulator.total` and
  `accumulator.increment`).
- `THIS` and `SUPER` → `variable.language.st`.
- General identifier fallback at the end of the pattern list → `variable.name.st`.
- Enum values inside a type declaration's `( ... )` list → `variable.other.enummember.st` if the
  enum rule is implemented; otherwise they fall back to `variable.name.st`. This is a quality
  gain, not an F-1 acceptance category.
- Do not add a generic call-name rule. A user function call (`AddStep(sum)`), an FB instance call
  (`accumulator()`), and a method call (`obj.method()`) are lexically similar but semantically
  different; coloring every callee as `entity.name.function.st` would lie about the last two.
  Only standard prelude functions (F1-D7) get `support.function.st`.
- Named call arguments remain at the fallback `variable.name.st`; a call-argument region is not
  added in F-1.

The declaration matcher inside a var section matches `ident` followed by `,`, `AT`, or `:`; this
handles comma-separated declarations and `AT %I...` addresses without requiring a line anchor, and
initializer expressions do not contain a bare `:`.

Known lexical miscolors, accepted and documented:

| Construct | Lexical result | Why it is acceptable |
| --- | --- | --- |
| Enum values without the optional enum rule | `variable.name.st` | They are still colored; the grammar does not resolve the enum owner. |
| Named call arguments | `variable.name.st` | A parameter reference is indistinguishable from a variable at the call site without resolution. |
| `TASK` definition names and config instance names | `variable.name.st` | They are configuration identifiers, not expressions; the fallback is honest. |
| Namespace segments in arbitrary expressions | `variable.name.st` or member color | Qualified type references are handled only in declaration contexts. |
| User function/method/FB callees | `variable.name.st` or member color | No semantic call heuristic; documented as a TextMate limit. |
| Struct-initializer keys | `variable.name.st` | They are not declarations. |

**Rationale.** Section-based scopes give parameters and variables distinct colors where the grammar
knows the declaration kind. The fallback guarantees that every identifier is colored, so no source
text remains at the theme foreground. The conservative call policy is the honest reconciliation of
TextMate's lexical limits with the requirement for variable/declaration colors.

**Alternatives considered.** The reference's member scope (`variable.parameter.st`) conflates
members with parameters; `variable.other.member.st` is the standard member scope and is the
recommendation (F1-Q5). An aggressive call-name rule was rejected. Declaration-only coloring leaves
most identifiers white and fails F-1. Semantic resolution is out of scope under I-7.

### F1-D6 — Control keywords, modifiers, constants, operators

**Recommendation.**

| Family | Keywords | Scope |
| --- | --- | --- |
| Conditional | `IF`, `THEN`, `ELSIF`, `ELSE`, `END_IF`, `CASE`, `END_CASE`, and `OF` inside `CASE` | `keyword.control.conditional.st` |
| Loop | `FOR`, `TO` in `FOR`, `BY`, `DO`, `END_FOR`, `WHILE`, `END_WHILE`, `REPEAT`, `UNTIL`, `END_REPEAT` | `keyword.control.loop.st` |
| Flow | `EXIT`, `CONTINUE`, `RETURN` | `keyword.control.flow.st` |
| Declaration | `TYPE`/`END_TYPE`, `STRUCT`/`END_STRUCT`, `ARRAY`, `AT`, `VAR*`/`END_VAR`, `REF_TO`, `REF`, `POINTER`, `FUNCTION*`, `PROGRAM*`, `CLASS*`, `INTERFACE*`, `METHOD*`, `EXTENDS`, `IMPLEMENTS`, `NAMESPACE*`, `USING`, `CONFIGURATION*`, `RESOURCE*`, `ON`, `TASK`, `SINGLE`, `INTERVAL`, `PRIORITY`, `WITH`, `ACTION*`, SFC keywords, `R_EDGE`, `F_EDGE` | `keyword.declaration.st` |
| Modifier | `PUBLIC`, `PRIVATE`, `PROTECTED`, `INTERNAL`, `FINAL`, `ABSTRACT`, `OVERRIDE`, `OVERLAP`, `CONSTANT`, `RETAIN`, `NON_RETAIN`, `READ_ONLY`, `READ_WRITE` | `storage.modifier.st` |
| Language constant | `TRUE`, `FALSE`, `NULL` | `constant.language.st` |
| Operator | `AND`, `OR`, `XOR`, `NOT`, `MOD`, `ADR`, `SIZEOF`, and all symbolic operators | `keyword.operator.st` |
| Type operator | `OF` in `ARRAY ... OF`, `TO` in `POINTER TO` | `keyword.operator.type.st` |
| Language variable | `THIS`, `SUPER` | `variable.language.st` |

`NULL` moves out of the declaration list; `THIS`/`SUPER` move out of the declaration list; `OF`
moves out of the declaration list. `POINTER` remains a declaration keyword. The `CASE` region
overrides `OF` before the generic type-operator rule, and `POINTER TO` is a specific rule before
the generic loop `TO`.

Symbolic operators keep the single `keyword.operator.st` scope. Splitting assignment/comparison/
arithmetic subscopes would add fixture churn without an F-1 acceptance gain.

**Rationale.** Standard control subscopes match the built-in themes' `keyword.control` rules and
make conditional/loop/flow visually distinct without inventing custom roots. The `OF`/`TO` context
rules resolve the only real ambiguities in the current keyword table.

**Alternatives considered.** Keeping the flat `keyword.control.st` is simpler but loses the
semantic split; F-1 does not require the split, but it is cheap and standard. Refining operators
has no acceptance value. Moving `NULL`/`THIS`/`SUPER` is needed so the categories are honest.

### F1-D7 — Built-in functions and standard function blocks

**Recommendation.**

- `support.function.st` for the standard prelude functions in `ALL_STD_FNS` and the test-only
  `TEST_STD_FNS`, when the name is followed by optional whitespace and `(`. The match is
  case-insensitive:
  `ABS`, `SQRT`, `LN`, `LOG`, `EXP`, `SIN`, `COS`, `TAN`, `ASIN`, `ACOS`, `ATAN`, `ATAN2`, `EXPT`,
  `TRUNC`, `ADD`, `SUB`, `MUL`, `DIV`, `MOD`, `MOVE`, `SHL`, `SHR`, `ROL`, `ROR`, `AND`, `OR`,
  `XOR`, `NOT`, `SEL`, `MAX`, `MIN`, `LIMIT`, `MUX`, `GT`, `GE`, `EQ`, `LE`, `LT`, `NE`, `LEN`,
  `LEN_MAX`, `LEN_CODE_UNIT`, `LEFT`, `RIGHT`, `MID`, `CONCAT`, `INSERT`, `DELETE`, `REPLACE`,
  `FIND`, plus `STC_ASSERT`, `STC_CLOCK_ADVANCE`, `STC_IO_SET_INPUT`, `STC_IO_GET_OUTPUT`.
- Exclude names that are lexer keywords from the function alternation (`AND`, `OR`, `XOR`, `NOT`,
  `MOD` are already `keyword.operator`; they are listed above only because they are in the prelude);
  keyword rules also run before the support rules.
- `support.class.st` for `STANDARD_FBS`: `TON`, `TOF`, `TP`, `CTU`, `CTD`, `CTUD`, `R_TRIG`,
  `F_TRIG`, `SR`, `RS`. These are types; an instance variable still gets `variable.name.st`.
- Do not invent conversion names such as `TO_INT` or `TRUNC_INT`. The stc prelude is the source of
  truth; the reference extension's CODESYS/GX Works conversion list is not part of this language.
- Do not add a generic call rule for user POUs, FB instances, or methods (F1-D5).
- Maintain the lists in the grammar as one alternation each. Add a test-only unit test in
  `stc-sema` that compares the grammar's `support.function.st` and `support.class.st` rules with
  `ALL_STD_FNS`, `TEST_STD_FNS`, and `STANDARD_FBS`; keyword-colliding names are excluded. The test
  adds `serde_json` as a test-only dev-dependency; it must not change the public API or add a
  runtime dependency.

**Rationale.** `support.function`/`support.class` are the standard, theme-recognized scopes. The
prelude lists are the compiler's truth; a drift test prevents the grammar from quietly diverging as
functions are added. The call lookahead avoids coloring a variable that merely shares a built-in
name.

**Alternatives considered.** A generated grammar from the prelude would remove the list duplication
but adds a build step and a generated-file review burden; the test gives most of the benefit with
less machinery. The reference's broad call rule was rejected. Scoping all standard FB names as
`entity.name.type` rather than `support.class` is possible, but `support.class` is the standard
class-like scope and remains in the type color family.

### F1-D8 — Pragmas, direct variables, comments, strings, numbers

**Recommendation.**

- Pragmas: keep the `meta.pragma.st` begin/end region and the brace punctuation. Capture the
  pragma name in the begin pattern: `(\{stc:)([A-Za-z_][A-Za-z0-9_]*)`, with group 1 as
  `punctuation.definition.pragma.begin.st` and group 2 as `keyword.other.pragma.st`. Keep
  `{stc:<name> ...}` payload strings as `string.*.st`, payload numbers as
  `constant.numeric.pragma.st`, and unquoted payload words as `variable.parameter.pragma.st`.
  Known names (`test`, `export`, `no_mangle`, `inline`, `deprecated`, `io`) are not separately
  enumerated; the capture handles any name.
- Direct variables: change `variable.other.direct.st` to `variable.other.constant.st` for the full
  `%[IQM]?[XBWDL]?(?:\d+(?:\.\d+)*|\*)` match. The `.direct` suffix is nonstandard and is not
  matched by built-in themes.
- Comments, strings, escapes, numbers, durations, dates, and times keep their existing scopes. The
  documentation comment scope remains distinct from the ordinary line comment.
- The theme colors the pragma name and braces with the pragma role; the payload keeps string and
  number colors so JSON remains readable.

**Rationale.** The pragma name needs a colorable token separate from payload keys; capturing it in
the begin is precise and does not leak to later payload words. `variable.other.constant` is the
standard constant-like scope and is theme-distinct in Dark+/Light+. Keeping the literal scopes
unchanged limits churn and preserves the existing fixtures.

**Alternatives considered.** A word alternation for known pragma names would also color payload
keys with the same spelling. `entity.name.tag.pragma.st` is less standard and not themed by the
target families. Treating the whole pragma as a comment hides the payload's string/number colors.
Keeping `variable.other.direct.st` leaves the direct-variable category dependent on a custom theme
rule.

### F1-D9 — Theme files, palette, and accessibility

**Recommendation.** Ship two theme files:

- `editors/vscode/themes/stc-dark-color-theme.json` — `type: "dark"`, token colors and minimal
  editor colors, editor background `#1F1F1F`, foreground `#D4D4D4`.
- `editors/vscode/themes/stc-light-color-theme.json` — `type: "light"`, editor background
  `#FFFFFF`, foreground `#1F1F1F`.

Contribute them in `package.json`:

```json
"themes": [
  { "label": "stc Dark", "uiTheme": "vs-dark", "path": "./themes/stc-dark-color-theme.json" },
  { "label": "stc Light", "uiTheme": "vs", "path": "./themes/stc-light-color-theme.json" }
]
```

Set `"semanticHighlighting": false` because the extension has no semantic-token provider. Define
at least `editor.background`, `editor.foreground`, `editorLineNumber.foreground`,
`editor.selectionBackground`, and `editorCursor.foreground`; inherit the rest of the workbench
chrome from `uiTheme`. Do not set `workbench.colorTheme` in the sample settings; document how to
select the themes.

Palette (all values are foreground colors; contrast is against the theme's editor background):

| Role | Scopes | Dark | Contrast | Light | Contrast |
| --- | --- | --- | --- | --- | --- |
| Comment | `comment` | `#6A9955` | 4.95 | `#5A6673` | 5.86 |
| Control keyword | `keyword.control` | `#C586C0` | 5.92 | `#AF00DB` | 5.46 |
| Declaration keyword | `keyword.declaration`, `storage.modifier` | `#569CD6` | 5.59 | `#0000FF` | 8.59 |
| Function/declaration name | `entity.name.function` | `#DCDCAA` | 11.66 | `#795E26` | 6.10 |
| Type | `entity.name.type`, `support.type`, `support.type.primitives`, `entity.name.namespace`, `entity.other.inherited-class`, `support.class` | `#4EC9B0` | 8.09 | `#267F99` | 4.59 |
| Variable | `variable`, `variable.other.member` | `#9CDCFE` | 11.05 | `#001080` | 15.15 |
| Parameter | `variable.parameter` | `#7FB3D5` | 7.31 | `#005A9E` | 7.10 |
| Direct variable | `variable.other.constant` | `#4FC1FF` | 8.17 | `#C2185B` | 5.87 |
| String | `string` | `#CE9178` | 6.24 | `#A31515` | 7.85 |
| Number | `constant.numeric` | `#B5CEA8` | 9.70 | `#098658` | 4.60 |
| Pragma | `keyword.other.pragma`, `punctuation.definition.pragma` | `#FFB86C` | 9.67 | `#B45309` | 5.02 |
| Language constant | `constant.language` | `#569CD6` | 5.59 | `#0000FF` | 8.59 |
| Language variable | `variable.language` | `#569CD6` | 5.59 | `#0000FF` | 8.59 |
| Operator/punctuation | `keyword.operator`, `punctuation` | `#D4D4D4` | 11.12 | `#1F1F1F` | 16.48 |

All ratios are at least 4.5:1 (WCAG AA for normal text). The nine F-1 categories — keyword,
type, declaration, variable, comment, string, number, pragma, direct variable — use a distinct
foreground in each theme. `entity.name.function` is an additional distinct color. Comments are
italic; other roles use the default font style.

Theme rule order is generic to specific: `variable` first, then `variable.parameter`,
`variable.other.member`, `variable.other.constant`, and `variable.language`; `entity.name.type`
before the more specific primitive selector if both are used. Use generic selectors without `.st`
for the standard families so other languages in the same workbench remain readable; add `.st`
selectors for the pragma role.

**Rationale.** The shipped themes are the only way to guarantee the nine distinct categories in
both modes while preserving standard scopes. The palette uses standard theme colors where they
already meet the contrast target and adjusted values where the standard palette does not separate
the roles (for example the light comment and light pragma). The 4.5:1 check is deterministic and
catches later edits.

**Alternatives considered.** Reusing Dark+ and One Dark values verbatim would inherit their license
and maintenance constraints and still not distinguish pragma/direct variables. A single theme has
no light mode. A settings-only recommendation is theme-version-dependent. A full workbench theme
is out of scope.

### F1-D10 — Coverage checks and fixtures

**Recommendation.**

- Keep `grammar_covers_lexer_tables` unchanged in meaning: every `Keyword::ALL` and `Punct::ALL`
  spelling must remain present.
- Add `primitive_type_keywords_use_primitive_scope` to
  `crates/stc-syntax/tests/grammar_coverage.rs`. It holds the Tokenization §3.1 primitive list,
  asserts each name is a lexer keyword (catches typos), collects patterns whose `name` is
  `support.type.primitives.st`, and asserts each primitive appears as a whole alternative. This
  requires a small helper that collects `(scope, regex)` pairs; the existing helper can remain.
- Add `prelude_names_are_scoped_in_the_vscode_grammar` in `crates/stc-sema/src/prelude.rs` as a
  `#[cfg(test)]` module unit test. It iterates `ALL_STD_FNS`, `TEST_STD_FNS`, and `STANDARD_FBS`,
  skips names that are lexer keywords, reads the grammar with `serde_json`, and asserts each name
  appears as a whole alternative in the `support.function.st` or `support.class.st` rule. Add
  `serde_json` as a dev-dependency to `crates/stc-sema/Cargo.toml`; no public API changes.
- Update `test/grammar/keywords.st`: `DINT`/`USTRING`/`ANY_DERIVED` → `support.type.primitives.st`;
  `IF`/`THEN`/`END_IF` → `keyword.control.conditional.st`; add `NULL` → `constant.language.st`,
  and `THIS`/`SUPER` → `variable.language.st`.
- Update `test/grammar/direct-variables.st`: `variable.other.constant.st`.
- Update `test/grammar/pragmas.st`: outer `meta.pragma.st`, pragma name
  `keyword.other.pragma.st`, and the existing payload assertions.
- Add `test/grammar/declarations.st`: function/method/program names, FB/class/interface names,
  type aliases, config instance vs plain program, namespaces, and modifiers.
- Add `test/grammar/variables.st`: `VAR_INPUT` parameter, `VAR` variable, struct field, member
  access, `THIS`/`SUPER`, and a negative case that a call callee is not a function scope.
- Add `test/grammar/types.st`: primitive, type reference after `:`, `ARRAY ... OF`, `POINTER TO`,
  `EXTENDS`/`IMPLEMENTS`, qualified names, `OF` in array vs `CASE`, `TO` in `FOR` vs `POINTER TO`.
- Add `test/grammar/builtins.st`: `ABS(...)` → `support.function.st`; `TON` → `support.class.st`;
  a user function call and an FB instance call do not get support scopes.
- Add `editors/vscode/test/theme.test.mjs`: both themes exist and are contributed with the right
  `uiTheme`; every F-1 role resolves to an explicit foreground; the nine categories are pairwise
  distinct per theme; every role contrast is at least 4.5:1 against `editor.background`. `npm test`
  picks this file up because the script runs `test/*.test.mjs`.
- Each new fixture should include at least one negative assertion for a likely miscolor (for
  example, the config `sample` instance is not `entity.name.function.st`, and `OF` in an array is
  not `keyword.control.conditional.st`).

**Rationale.** The existing coverage test protects keyword/punctuation presence; the new checks
protect the two new drift surfaces (primitives and prelude built-ins). The fixtures pin the scope
taxonomy and catch ordering regressions, while the theme test makes the palette acceptance
deterministic.

**Alternatives considered.** Fixture-only verification would not catch prelude/grammar drift or
theme contrast regressions. A generated grammar would remove duplication but adds build complexity.
Snapshotting theme JSON is brittle and not more informative than the role/contrast test.

### F1-D11 — Verification and CI

**Recommendation.** Use deterministic tests for grammar scopes and theme values, and a manual
Inspect Editor Tokens and Scopes matrix for the rendered result. The commands, fixture matrix,
theme matrix, and VSIX check are in §6.

The existing Electron smoke does not and cannot inspect token colors; it remains a rendering and
feature smoke. Do not add a fragile screenshot harness for F-1. If the owner asks for screenshot
evidence, capture it manually into the release evidence outside the repository.

The CI extension job already runs `npm ci`, `npm run typecheck`, `npm test`, and `npm run package`,
and the workspace matrix runs the Rust tests. No new CI job is needed. If themes ship, extend the
VSIX content check to require both theme files.

**Rationale.** Token colors are rendered by the workbench, and there is no extension API to query
them. Grammar fixtures plus a theme JSON test cover what can be tested; the Inspect command covers
what cannot.

**Alternatives considered.** A Playwright/Electron screenshot comparison would be brittle across
VS Code versions and platforms. Relying only on manual inspection would allow silent regressions.

### F1-D12 — Reference use and licensing

**Recommendation.** Use Serhioromano/vscode-st only as a reference for scope names and broad
grammar shape. Implement the patterns from the stc specifications and the existing lexer tables.
Do not copy its grammar or any third-party theme. Record in the implementation notes that the
reference is MIT, "Copyright (c) 2018 Sergey Romanov", pinned at
`4178d989f74a9986359db0f65b3b0e1f6721d4d7`, and that it ships no theme.

If implementation later copies any regex, scope list, or color value from the reference or another
MIT theme, add `editors/vscode/THIRD-PARTY-NOTICES.md` with the MIT text and copyright, include it
in the VSIX (it is not excluded by `.vscodeignore`), and reference it from the extension README.
Scope names alone are not treated as copied code, but the notice is required for copied patterns.

**Rationale.** The reference's language surface is CODESYS/GX Works-oriented, not the stc 2013 OOP
profile, and it has no theme. Independent implementation avoids attribution complexity and keeps
the grammar tied to the stc lexer tables. The MIT terms are recorded in case an implementation
detail is copied.

**Alternatives considered.** Copying the reference grammar with an MIT notice would import
mismatched constructs and maintenance. Using a third-party theme as the shipped theme would add
attribution and version-coupling. No reference use would lose the naming evidence.

### F1-D13 — Scope boundaries

**Recommendation.** F-1 changes only the extension grammar, optional theme assets, tests,
documentation, and CI content checks. It does not add a semantic-token provider, change the LSP or
DAP capability maps, change the language id or root scope, or modify the compiler, runtime,
artifacts, ABI, or language specification. Built-in names are read from the existing prelude, not
redefined. The parent plan's statement that highlighting comes from the TextMate grammar remains
true.

**Rationale.** I-7 and I-16 lock these boundaries, and WL-1.1's exit criteria accept TextMate
highlighting. The lexical limits are documented rather than solved by expanding the LSP.

**Alternatives considered.** Semantic tokens would resolve declarations and uses but are a
separate capability (I-7) and a separate plan. A language-server highlight capability would still
need semantic classification and is out of scope.

## 4. Open questions for the owner

| ID | Question | Recommended default |
| --- | --- | --- |
| F1-Q1 | Ship two small themes (`stc Dark`, `stc Light`) and document Dark Modern/Light Modern as fallbacks, or only document a recommended theme, or change the grammar only? Should the distinct-color acceptance be verified on the shipped themes, with the built-ins verified for scope compatibility? | Ship both themes; verify distinctness on the shipped themes and compatibility on the built-ins. |
| F1-Q2 | Which theme families must the scope names support? | Dark Modern/Light Modern defaults, Dark+/Light+, and the One Dark family. High-contrast themes are not a target. |
| F1-Q3 | How aggressive should declaration/use heuristics be? | Conservative: declaration keywords, section regions, and bounded type contexts only; no generic call-name rule. |
| F1-Q4 | Should remaining identifiers be colored by a fallback? | Yes: `variable.name.st` for all otherwise-unscoped identifiers, with the known miscolors documented. |
| F1-Q5 | Which scope for members and struct fields? | `variable.other.member.st` (standard), not the reference's `variable.parameter.st`. |
| F1-Q6 | Which pragma name scope? | `keyword.other.pragma.st` on the captured name, with pragma-colored braces; payload keeps string/number scopes. |
| F1-Q7 | Should the sample workspace set a theme automatically? | No: document selection; do not set `workbench.colorTheme` in sample settings. |
| F1-Q8 | How should the built-in and primitive lists be maintained? | Grammar-local lists plus the `stc-sema` prelude drift test and the `stc-syntax` primitive test; test-only `serde_json` dev-dependency allowed. |

If the owner chooses the grammar-only option in F1-Q1, F1-D9 and the theme test are dropped, the
theme half of the acceptance is documented as not met, and the standard-theme gaps in §2.4 are
recorded as limitations. If the owner requires only the built-in themes, the pragma/direct-variable
distinctness must be relaxed or moved to user token customizations.

## 5. Work breakdown

| Phase | Work | Acceptance |
| --- | --- | --- |
| F1-P0 | Owner review of F1-Q1–F1-Q8; record the answers and mark the affected decisions accepted | Every question has an answer; the theme decision and heuristic policy are recorded |
| F1-P1 | Grammar taxonomy: primitives, declaration names, type references, variables/parameters/members, control keywords/modifiers/constants, pragmas, direct variables, built-ins, fallback | The sample workspace's declaration, type, variable, parameter, member, pragma, direct-variable, and built-in tokens carry the scopes in F1-D2–F1-D8; keyword/punctuation coverage is green |
| F1-P2 | Theme assets: two theme JSON files, manifest contribution, palette, minimal editor colors | Both themes load in VS Code; every role resolves; all acceptance categories are distinct and at least 4.5:1 |
| F1-P3 | Tests: update existing fixtures; add declarations, variables, types, built-ins fixtures; add the primitive and prelude drift tests; add `test/theme.test.mjs` | `npm test`, the new Cargo tests, and the theme test pass; each new fixture has positive and negative assertions |
| F1-P4 | Verification: run the local commands, the Inspect Tokens matrix in Dark Modern, Light Modern, stc Dark, stc Light, and optionally One Dark; inspect the VSIX | Evidence is recorded in the checklist; the VSIX contains the theme files and no excluded sources/tests |
| F1-P5 | Documentation and close-out: extension README, IDE-Integration, KNOWN-LIMITATIONS, sample README, release checklist, checklist item 1/F-1, parent plan change log | Docs match the observed behavior; F-1 is checked with the scope/color evidence; the parent plan's §12.1 F-1 is closed |

Files expected to change:

- `editors/vscode/syntaxes/st.tmLanguage.json` — grammar scopes and ordering.
- `editors/vscode/package.json` — `contributes.themes` (if F1-Q1 ships themes).
- `editors/vscode/themes/stc-dark-color-theme.json`,
  `editors/vscode/themes/stc-light-color-theme.json` — new theme assets.
- `editors/vscode/test/grammar/*.st` — updated and new fixtures.
- `editors/vscode/test/theme.test.mjs` — new theme test.
- `crates/stc-syntax/tests/grammar_coverage.rs` — primitive scope test.
- `crates/stc-sema/src/prelude.rs` and `crates/stc-sema/Cargo.toml` — test-only prelude drift
  check and dev-dependency.
- `.github/workflows/ci.yml` — VSIX required-file list (if themes ship).
- `editors/vscode/README.md`, `editors/vscode/samples/workspace/README.md`,
  `specification/Architecture/IDE-Integration.md`, `KNOWN-LIMITATIONS.md`,
  `RELEASE-CHECKLIST.md`, `WL-1.1-UI-CHECKLIST.md`, `WL-1.1-Plan.md` — documentation and evidence.

No compiler, runtime, parser, HIR, sema production code, LSP, or DAP file changes.

## 6. Verification and CI

### 6.1 Commands

From the repository root on Windows (PowerShell) or the equivalent shell:

```text
cd editors/vscode
npm ci
npm run typecheck
npm test
npm run package
cd ..\..
cargo test -p stc-syntax grammar_covers_lexer_tables
cargo test -p stc-syntax primitive_type_keywords_use_primitive_scope
cargo test -p stc-sema prelude_names_are_scoped_in_the_vscode_grammar
```

`npm test` runs the problem-matcher tests, the new theme test, and the grammar fixtures. The
package step writes the VSIX. CI runs the same extension checks in the `extension` job and the Rust
tests in the workspace matrix; no new job is required.

VSIX check after packaging:

```text
npx vsce ls | Select-String "themes/"
```

or, in CI's bash step, the existing `unzip -l` list. The VSIX must contain both theme files and
must still exclude `src/**`, `test/**`, `samples/**`, `tsconfig.json`, and `esbuild.mjs`.

### 6.2 Grammar fixture matrix

| Fixture | Positive assertions | Negative assertions |
| --- | --- | --- |
| `keywords.st` | Primitive names → `support.type.primitives.st`; conditional keywords → `keyword.control.conditional.st`; loop/flow keywords → their subscopes; `NULL` → `constant.language.st`; `THIS`/`SUPER` → `variable.language.st`; operators unchanged | A primitive is not `storage.type.st`; a control keyword is not `keyword.declaration.st` |
| `declarations.st` | POU/type/namespace names per F1-D3; config instance → `variable.name.st`; config type → `entity.name.type.st`; modifiers → `storage.modifier.st` | Config `sample` is not `entity.name.function.st`; `END_FUNCTION` is not a declaration name |
| `variables.st` | `VAR_INPUT` name → `variable.parameter.st`; `VAR` name → `variable.name.st`; struct field and `.` member → `variable.other.member.st`; `THIS`/`SUPER` | A user call callee is not `entity.name.function.st`; a member is not `variable.parameter.st` |
| `types.st` | Primitive after `:`; named type after `:`; `ARRAY ... OF`; `POINTER TO`; `EXTENDS`/`IMPLEMENTS` → inherited-class; qualified type segments | `OF` in an array is not conditional; `TO` in `POINTER TO` is not loop; `CASE` labels are not types |
| `builtins.st` | `ABS(...)` → `support.function.st`; `TON` → `support.class.st`; test functions | User function call and FB instance call do not get `support.function.st`/`support.class.st` |
| `pragmas.st` | Outer `meta.pragma.st`; name → `keyword.other.pragma.st`; payload string/number scopes | A payload key is not the pragma name scope |
| `direct-variables.st` | All forms → `variable.other.constant.st` | Not `variable.other.direct.st` |
| `comments.st`, `strings.st`, `numbers.st`, `literals.st`, `operators.st` | Existing assertions remain green | No regression in nested comments, escapes, typed literals, or operator precedence |

### 6.3 Manual Inspect Editor Tokens and Scopes matrix

Open `editors/vscode/samples/workspace/src/main.st` (and `tests/smoke.st`) and run **Developer:
Inspect Editor Tokens and Scopes** on each token. For tokens that the sample does not contain
(`FOR`, `RETURN`, `ABS`, `TON`, `%IX0.0`, `TRUE`), open the matching grammar fixture under
`editors/vscode/test/grammar/`. Record the rendered color and the scope stack.

| Token | Source line | Expected semantic scope | Acceptance category |
| --- | --- | --- | --- |
| `FUNCTION` | 12 | `keyword.declaration.st` | declaration |
| `AddStep` declaration | 12 | `entity.name.function.st` | declaration name |
| `DINT` | 12, 14, 17, 28, 36, 37 | `support.type.primitives.st` | type |
| `Accumulator` type reference | 35 | `entity.name.type.st` | type |
| `IF` / `END_IF` | sample/smoke | `keyword.control.conditional.st` | keyword |
| `FOR` / `TO` / `DO` | keywords/types fixture | `keyword.control.loop.st` | keyword |
| `RETURN` | keywords fixture | `keyword.control.flow.st` | keyword |
| `sum` variable | 36, 39 | `variable.name.st` | variable |
| `value` parameter | 14 | `variable.parameter.st` | variable/parameter |
| `total` after `.` | 42 | `variable.other.member.st` | variable/member |
| `// comment` | any | `comment.line.double-slash.st` | comment |
| `"text"` | smoke | `string.quoted.double.st` | string |
| `1` / `T#10ms` | 8, 49 | `constant.numeric.integer.st` / `constant.numeric.duration.st` | number |
| `test` in `{stc:test}` | smoke | `keyword.other.pragma.st` | pragma |
| `%IX0.0` / `%I*` | fixture | `variable.other.constant.st` | direct variable |
| `TRUE` | fixture | `constant.language.st` | constant |
| `ABS(` | builtins fixture | `support.function.st` | built-in |
| `TON` | builtins fixture | `support.class.st` | type/built-in |
| `sample` config instance | 50 | `variable.name.st` | not a function |

Theme matrix: repeat the matrix in **Dark Modern**, **Light Modern**, **stc Dark**, and **stc
Light**. Optionally spot-check **One Dark** for prefix compatibility. Record the actual color for
each category in the checklist item 1 result.

Expected outcomes:

- The shipped themes show nine distinct category colors in both modes, with all contrast ratios at
  or above 4.5:1.
- Dark Modern/Light Modern show distinct keyword, declaration, function/type, variable, comment,
  string, number, and direct-variable colors; the pragma name may render as a keyword because the
  built-ins have no pragma-specific rule. This gap is documented, not a failure of the shipped
  themes.
- One Dark shows the standard families; direct variables and parameters follow its variable
  colors. This is a compatibility spot-check, not the acceptance vehicle.
- No inspected token shows a custom-only scope that the standard themes ignore.

### 6.4 CI and packaging

- The extension job runs `npm ci`, `npm run typecheck`, `npm test`, and `npm run package`
  (`.github/workflows/ci.yml` lines 753–812; the commands are lines 767–779).
- The VSIX content check (lines 778–812) adds both theme files,
  `extension/themes/stc-dark-color-theme.json` and
  `extension/themes/stc-light-color-theme.json`, when F1-Q1 ships the themes. The excluded list is
  unchanged.
- The Rust workspace matrix runs the new `stc-syntax` and `stc-sema` tests.
- The headless Electron smoke remains unchanged: it activates the extension and exercises
  highlighting as a rendering smoke, but it does not assert token colors.
- The VSIX artifact is uploaded by the existing CI job; size impact is two small JSON files.

## 7. Documentation

| File | Change |
| --- | --- |
| `editors/vscode/README.md` | Extend the feature list with the semantic scope families. Add a "Highlighting and color themes" section: the scope table, `stc Dark`/`stc Light` selection, Dark Modern/Light Modern fallback compatibility, One Dark compatibility, and the lexical limits (no semantic resolution; user calls are variables; pragma distinctness in built-in themes is not guaranteed). Update the development commands with the theme test. |
| `editors/vscode/samples/workspace/README.md` | Add a short "Highlighting check" table pointing at the tokens in §6.3 and how to select `stc Dark`/`stc Light`; do not set a theme automatically. |
| `specification/Architecture/IDE-Integration.md` | Add `### 2.1 Language contribution and highlighting` with the scope taxonomy, TextMate limits, theme contribution, and compatibility statement. Update §5 VSIX contents to include the themes and §6 verification to add the theme test, the primitive/prelude drift tests, and the Inspect Tokens matrix. |
| `KNOWN-LIMITATIONS.md` §4 | Keep the no-semantic-tokens row, and add the lexical-highlighting limit: declarations and uses are not resolved; user call names and named arguments are colored as variables; pragma and direct-variable colors are guaranteed only in the shipped themes. |
| `RELEASE-CHECKLIST.md` §7 | Item 3 includes the theme files in the VSIX contents; the manual in-editor checks include the Inspect Tokens matrix in Dark Modern/Light Modern/stc Dark/stc Light; item 5 keeps its rendering-smoke wording, not a token-color claim. |
| `WL-1.1-UI-CHECKLIST.md` | Update item 1 with the inspected scopes and colors for both modes; mark F-1 checked with the date and evidence; update the results log row for item 1 and the F-1 result row. |
| `WL-1.1-Plan.md` | At close-out, add the 2026-10-31 (or completion date) change-log row and mark §12.1 F-1 closed with a pointer to this sub-plan and the checklist evidence. Do not edit the parent plan during planning. |

The language semantics documents ([Tokenization.md](specification/Syntax/Tokenization.md),
[Grammar.md](specification/Syntax/Grammar.md)) do not change: F-1 only classifies tokens that the
language already defines.

Markdown changes under `specification/**` and `README.md` are checked by the existing
`markdownlint-cli2` CI step; `MD013` is disabled. Tables and headings follow the repository style.

## 8. Risks and mitigations

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Standard themes ignore a scope or collapse categories | The palette remains partly undifferentiated under the user's theme | Use standard scope roots; ship the two themes for guaranteed distinctness; document the built-in gaps; verify the compatibility matrix in §6.3 |
| A fallback identifier rule miscolors enum values, named arguments, config names, or initializer keys | Some identifiers get the variable color that may not match their semantic role | Keep the rule last; use section/declaration rules for the high-confidence cases; add negative fixtures; document the lexical limits |
| Aggressive call/declaration heuristics color FB instances or methods as functions | Misleading highlighting and user confusion | F1-D3/F1-D5 conservative policy; only prelude built-ins get support scopes; no generic call rule |
| Rule ordering regressions swallow keywords, punctuation, or members | Existing scopes break; fixtures fail | Follow the ordering in F1-D2; run the full fixture matrix and the keyword/punctuation coverage test on every grammar change |
| `OF`/`TO` context rules conflict | Array types or `POINTER TO` get control colors, or CASE gets type/operator colors | Specific context rules before the generic keyword rules; fixtures assert both sides |
| TextMate regex performance or catastrophic backtracking | Slow tokenization on large files | Use bounded alternations and fixed-length lookaheads; avoid nested quantified groups and unbounded lookbehind; spot-check the sample and a large source file |
| Built-in/primitive lists drift from the compiler | Built-ins stop highlighting or nonexistent names are colored | Grammar-local lists plus the `stc-sema` prelude test and the `stc-syntax` primitive test |
| Theme files are omitted from the VSIX or excluded by `.vscodeignore` | The contributed themes fail to load after installation | Keep `themes/` outside the ignore list; add both files to the CI VSIX required list; install the packaged VSIX in the manual pass |
| Theme values fail contrast or distinctness after an edit | Accessibility/acceptance regression | `test/theme.test.mjs` fails on a category collision or a ratio below 4.5:1 |
| Scope names are treated as a stable public surface | A later rename breaks users' `editor.tokenColorCustomizations` | Document the taxonomy in IDE-Integration and the extension README; treat renames as breaking changes |
| A third-party license is imported with copied patterns | Licensing/compliance failure | Reference-only implementation; record MIT and add `THIRD-PARTY-NOTICES.md` if any pattern or color is copied |
| Headless test environments have no theme/rendering path | Token colors cannot be asserted in Electron | Deterministic grammar/theme tests plus the manual Inspect matrix; no screenshot dependency in CI |

## 9. Change log

| Date | Change |
| --- | --- |
| 2026-10-31 | Created the F-1 highlighting sub-plan: recorded the grammar/theme/coverage findings; recommended grammar enrichment plus `stc Dark`/`stc Light`; recorded F1-D1–F1-D13, F1-Q1–F1-Q8, the phase breakdown, verification matrix, documentation actions, and risks. No implementation files changed. |
