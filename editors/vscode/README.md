# stc Structured Text for VS Code

Local editor support for `stc` Structured Text (IEC 61131-3) projects: syntax
highlighting, language configuration, build tasks, and compiler diagnostics in
the Problems panel.

The extension is a client. It invokes the local `stc` compiler and reads its
output; it contains no compiler logic. Everything runs locally.

## Features

- Syntax highlighting for `.st` files under the `source.st` TextMate scope:
  nested `(* ... *)` and `/* ... */` block comments, line comments, single- and
  double-quoted strings with `$` escapes, typed and UTF-8 string literals,
  integer, real, based, and typed numeric literals, duration/date/time
  literals, every reserved keyword, `{stc:...}` pragmas, directly represented
  variables (`%IX0.0`, `%QW28`, `%I*`), and punctuation.
- Language configuration: `//` line comments, `(* *)` block comments,
  brackets, auto-closing and surrounding pairs, an ST-aware word pattern, and
  indentation rules for block constructs.
- Tasks for `stc check`, `stc build`, `stc run`, `stc test`, and `stc fmt`.
  `build` is in the build group and `test` is in the test group.
- The `$stc` problem matcher for short-format compiler diagnostics.
- The `stc.path` setting.

The language server (push diagnostics, navigation, hover, completion, and
formatting through `stc lsp`) and the debug adapter are added in later phases.

## Requirements

- VS Code 1.82 or newer.
- The `stc` compiler available as `stc` on `PATH`, or `stc.path` pointing at
  the executable. The extension does not bundle a compiler.

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

## Settings

| Setting | Default | Meaning |
| --- | --- | --- |
| `stc.path` | `"stc"` | Path to the `stc` executable, or its name on `PATH`. |

## Diagnostics

Short-format lines (`path:line:column: severity[CODE]: message`) are matched by
the `$stc` problem matcher with `fileLocation: "absolute"`. Diagnostics come
from the compiler; the extension does not reinterpret them.

## Distribution and privacy

- VSIX-only distribution for v2.0.0; the extension is not published to the
  VS Code Marketplace.
- Local only: no telemetry, no network requests, and no bundled compiler.
- Debug support, when added, is an availability and operability feature, not a
  safety function. `stc` is not a safety PLC and safety must be provided by a
  separate system and machine design.

## Development

From this directory:

```sh
npm install
npm run typecheck
npm test
npm run package
```

`npm test` runs the problem-matcher unit tests and the TextMate grammar
fixtures in `test/grammar/`. The package step writes a `.vsix` next to
`package.json`. The Rust grammar coverage test
(`cargo test -p stc-syntax grammar_covers_lexer_tables`) checks the grammar
against `Keyword::ALL` and the punctuation table.
