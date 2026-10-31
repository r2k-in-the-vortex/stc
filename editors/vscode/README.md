# stc Structured Text for VS Code

Local editor support for `stc` Structured Text (IEC 61131-3) projects: syntax
highlighting, language configuration, build tasks, compiler diagnostics,
navigation, formatting, and a local debug session.

The extension is a client. It invokes the local `stc` compiler and runtime and
reads their output; it contains no compiler or debugger logic. Everything runs
locally.

## Features

- Syntax highlighting for `.st` files under the `source.st` TextMate scope:
  nested `(* ... *)` and `/* ... */` block comments, line comments, single- and
  double-quoted strings with `$` escapes, typed and UTF-8 string literals,
  integer, real, based, and typed numeric literals, duration/date/time
  literals, every reserved keyword, `{stc:...}` pragmas, directly represented
  variables (`%IX0.0`, `%QW28`, `%I*`), and punctuation.
- Semantic scope families for declared POU and type names, type references,
  primitive types, variables, parameters, members, direct variables, control
  keywords, modifiers, language constants and variables, prelude functions and
  standard function blocks, and pragma names. See
  [Highlighting and color themes](#highlighting-and-color-themes).
- The bundled `stc Dark` and `stc Light` color themes, which give every
  acceptance category a distinct, readable color.
- Language configuration: `//` line comments, `(* *)` block comments,
  brackets, auto-closing and surrounding pairs, an ST-aware word pattern, and
  indentation rules for block constructs.
- A language server (`stc lsp`) per project: push diagnostics, document
  symbols, go-to-definition, references, hover on identifiers, completion
  (keywords, visible names, and one level of members after `.`), and
  whole-document formatting with `stc fmt`.
- Tasks for `stc check`, `stc build`, `stc run`, `stc test`, and `stc fmt`.
- The `$stc` problem matcher for short-format compiler diagnostics.
- A debug adapter (`stc dap`): launch and attach, source breakpoints, continue,
  pause, over/into/out stepping, call stack, Globals and Locals scopes,
  variable expansion, `evaluate` for locals and manifest target paths, and
  `setVariable` when writes are enabled.
- The `stc.path` setting.

The language-server and debug-adapter contracts, including the closed
capability lists, are in `specification/Architecture/IDE-Integration.md` in
the repository.

## Requirements

- VS Code 1.82 or newer.
- The `stc` compiler available as `stc` on `PATH`, or `stc.path` pointing at
  the executable. The extension does not bundle a compiler.
- `clang` and the runtime artifacts that `stc build`, `stc run`, `stc test`,
  and debug launches already require.

## Language server

Open a folder containing `stc.toml`; the extension starts one `stc lsp` client
for the project and watches `**/*.st` and `**/stc.toml`. The server analyzes
the root package's production and test sources and publishes diagnostics in
the Problems panel. Save, reopen, or edit a file to reanalyze; open buffers are
analyzed from the editor text, and closed files from disk.

The supported language features are listed above. Semantic tokens, rename,
code actions, inlay hints, range formatting, call hierarchy, and workspace
symbols are not implemented; highlighting comes from the grammar. A missing
compiler is logged to the **stc** output channel and the server is not started.

## Highlighting and color themes

Highlighting is TextMate-only. The language server publishes no semantic
tokens, so declaration and use classification is lexical. The grammar uses
standard TextMate scope roots with a `.st` suffix:

| Role | Scope |
| --- | --- |
| Block/line/documentation comment | `comment.block.st`, `comment.line.double-slash.st`, `comment.line.documentation.st` |
| String, escape, character | `string.quoted.single.st`, `string.quoted.double.st`, `constant.character.escape.st`, `constant.character.st` |
| Number, duration, date/time | `constant.numeric.integer.st`, `.real.st`, `.based.st`, `.typed.st`, `.duration.st`, `.date.st`, `.time.st`, `.datetime.st` |
| Declaration keyword | `keyword.declaration.st` |
| Control keyword | `keyword.control.conditional.st`, `keyword.control.loop.st`, `keyword.control.flow.st` |
| Modifier | `storage.modifier.st` |
| Operator | `keyword.operator.st`, `keyword.operator.type.st` |
| Language constant | `constant.language.st` |
| Primitive type | `support.type.primitives.st` |
| Function/method/program declaration name | `entity.name.function.st` |
| Type/function-block/class/interface/type-alias name | `entity.name.type.st` |
| Namespace/configuration/resource name | `entity.name.namespace.st` |
| Extended/implemented type name | `entity.other.inherited-class.st` |
| Variable | `variable.name.st` |
| Parameter | `variable.parameter.st` |
| Member/struct field | `variable.other.member.st` |
| Direct variable | `variable.other.constant.st` |
| Enum member | `variable.other.enummember.st` |
| `THIS`/`SUPER` | `variable.language.st` |
| Built-in function | `support.function.st` |
| Standard function block | `support.class.st` |
| Pragma region, name, braces | `meta.pragma.st`, `keyword.other.pragma.st`, `punctuation.definition.pragma.begin/end.st` |

These scope names are a user-facing contract: `editor.tokenColorCustomizations`
can target them, and a later rename is a breaking theme change.

### Selecting a theme

The extension contributes two optional token themes. Run **Preferences: Color
Theme** and choose `stc Dark` or `stc Light`. The extension never changes
`workbench.colorTheme` and the sample workspace does not set it; the active
theme remains your choice. Both themes set `semanticHighlighting: false`,
since no semantic-token provider exists, and they define the editor background,
foreground, line number, selection, and cursor colors; the rest of the
workbench chrome follows the contributed `vs-dark`/`vs` UI theme.

The grammar's standard scope roots also color correctly under **Dark Modern**,
**Light Modern**, **Dark+**, **Light+**, and the **One Dark** family. Under
built-in themes the pragma name may render as a generic keyword and direct
variables may follow the variable color; only the shipped themes guarantee
distinct pragma and direct-variable colors. High-contrast themes are not a
target, though the standard scope families still apply.

Lexical limits:

- Declarations and uses are not resolved. A user function or function-block
  call name, and a named argument, is colored as a variable; only names in the
  prelude table receive `support.function.st`/`support.class.st`.
- A type reference is recognized after `:`, `ARRAY ... OF`, `REF_TO`,
  `POINTER TO`, `EXTENDS`, and `IMPLEMENTS`; other ambiguous identifier
  positions use the variable fallback.
- Pragma-name and direct-variable distinctness depends on the theme, as noted
  above.

## Tasks

Run `Tasks: Run Task` from the Command Palette and choose an `stc ...` task, or
define your own in `.vscode/tasks.json`:

```json
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "stc: build (release)",
      "type": "stc",
      "command": "build",
      "release": true,
      "project": "${workspaceFolder}/stc.toml",
      "problemMatcher": ["$stc"],
      "group": "build"
    }
  ]
}
```

The contributed task type `stc` accepts:

| Property | Type | Meaning |
| --- | --- | --- |
| `command` | string | One of `check`, `build`, `run`, `test`, `fmt`. |
| `project` | string | Path to `stc.toml`, its directory, or a project directory. Defaults to the first `stc.toml` found in the workspace. |
| `args` | string[] | Extra arguments appended to the generated command line. |
| `release` | boolean | Pass `--release` for `build` and `run`. |

Every task runs in the manifest directory. Generated tasks pass
`--message-format short --color never`; `check`, `build`, `run`, and `test`
also pass `--diagnostic-paths absolute`, so the problem matcher can resolve
source files from multi-package projects. `stc run` tasks keep running until
you terminate the task.

## Debugging

Add a launch or attach configuration to `.vscode/launch.json`:

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "type": "stc",
      "request": "launch",
      "name": "stc: Launch",
      "project": "${workspaceFolder}/stc.toml",
      "noBuild": false,
      "release": false,
      "allowWrite": false,
      "allowForce": false
    },
    {
      "type": "stc",
      "request": "attach",
      "name": "stc: Attach",
      "project": "${workspaceFolder}/stc.toml",
      "endpoint": "${workspaceFolder}/target/stc/debug/debug.endpoint.json"
    }
  ]
}
```

Launch builds the debug form by default, starts a local debug endpoint, and
stops the launch-owned host when the session ends. Attach connects to the
endpoint file printed by `stc run --debug-server` and leaves the host running.

| Key | Meaning |
| --- | --- |
| `project` | `stc.toml`, its directory, or a project directory. |
| `noBuild` | Launch existing artifacts without building. |
| `outDir` | Artifact directory override, primarily for `noBuild`. |
| `release` | Build the monitoring-only release form. |
| `allowWrite` | Request the write scope and enable `setVariable` for writable live roots; the host must also allow `[debug].allow-write`. |
| `allowForce` | Request the force scope; the host must also allow `[debug].allow-force`. DAP has no force request, so forcing remains a `stc debug` operation. |
| `sourceRoot` | Override package-root source mapping. |
| `args`, `cwd` | Extra host arguments and working directory. |
| `endpoint` | Attach endpoint file. |

Locals require a debug build and a `stc-debug/1.1` host; a release or
hook-free build is monitoring-only and offers no breakpoints, stepping, or
Locals. Local names are shown in the frame table's normalized uppercase form.
`evaluate` accepts a stopped-frame local name or a manifest target path; it
does not parse ST expressions. There is one debug session per host, and every
online swap ends the session without migrating breakpoints, forces, or
watches.

## Sample workspace

`samples/workspace/` is a clean, self-contained project with a program, a
function block, a function, smoke tests, launch and attach configurations, and
`stc` tasks. It documents the first-statement
breakpoint lines and the known values used by the automated tests. Open the
folder as the workspace to try the extension.

## Settings

| Setting | Default | Meaning |
| --- | --- | --- |
| `stc.path` | `"stc"` | Path to the `stc` executable, or its name on `PATH`. |

## Diagnostics

Short-format lines (`path:line:column: severity[CODE]: message`) are matched by
the `$stc` problem matcher with `fileLocation: "absolute"`. Diagnostics from
the language server use the same compiler codes and are published directly;
the extension does not reinterpret either source.

## Distribution and privacy

- VSIX-only distribution for v2.0.0; the extension is not published to the
  VS Code Marketplace.
- Local only: no telemetry, no network requests, and no bundled compiler.
- Debugging and forcing are availability and operability features, not safety
  functions. `stc` is not a safety PLC and safety must be provided by a
  separate system and machine design. A stopped or forced controller may hold
  outputs indefinitely.

## Development

From this directory:

```sh
npm ci
npm run typecheck
npm test
npm run package
```

`npm test` runs the problem-matcher and theme unit tests and the TextMate
grammar fixtures in `test/grammar/`. `test/theme.test.mjs` checks that both
themes are contributed, define the required editor colors and token rules, use
nine distinct category colors, and keep every role at or above the 4.5:1
contrast target. The package step writes a `.vsix` next to `package.json`;
`samples/**` is excluded by `.vscodeignore`.

The Rust tests keep the grammar in step with the compiler tables:
`cargo test -p stc-syntax` runs `grammar_covers_lexer_tables` against
`Keyword::ALL` and the punctuation table plus
`primitive_type_keywords_use_primitive_scope`; `cargo test -p stc-sema` runs
`prelude_names_are_scoped_in_the_vscode_grammar`, which checks the built-in
function and standard function-block lists against the grammar. The Rust
language-server and debug-adapter integration tests live in
`crates/stc-driver/tests/lsp_cli.rs` and `dap_cli.rs`.
