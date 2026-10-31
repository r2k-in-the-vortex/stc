# stc sample workspace

A small, self-contained `stc` project used as the local fixture for the
WL-1.1 language-server and debug-adapter integration tests. It contains a
program, a function block, a function, and one `{stc:test}` source set, and it
opens cleanly with no diagnostics.

## What it shows

| Feature | Where |
| --- | --- |
| Manifest, source globs, configuration selection, and the test source set | `stc.toml` |
| Global variables with initial values | `src/main.st` |
| `FUNCTION`, `FUNCTION_BLOCK`, and `PROGRAM` POUs | `src/main.st` |
| Direct FB input/output access (`accumulator.increment`, `accumulator.total`) | `src/main.st` |
| `CONFIGURATION`/`RESOURCE`/`TASK` with a program instance | `src/main.st` |
| Isolated smoke tests with `STC_ASSERT` | `tests/smoke.st` |
| Launch and attach configurations | `.vscode/launch.json` |
| `stc` tasks with the `$stc` problem matcher | `.vscode/tasks.json` |

## Highlighting check

Open `src/main.st`, place the cursor on a token, and run **Developer: Inspect
Editor Tokens and Scopes**. The sample covers these scopes:

| Token | Line | Expected scope |
| --- | --- | --- |
| `FUNCTION` | 12 | `keyword.declaration.st` |
| `AddStep` declaration | 12 | `entity.name.function.st` |
| `DINT` | 12, 14, 17, 28, 36, 37 | `support.type.primitives.st` |
| `Accumulator` type reference | 35 | `entity.name.type.st` |
| `sum` variable | 36, 39 | `variable.name.st` |
| `value` parameter | 14 | `variable.parameter.st` |
| `total` after `.` | 42 | `variable.other.member.st` |
| `(* comment *)` | 1, 7 | `comment.block.st` |
| `T#10ms` | 49 | `constant.numeric.duration.st` |
| `sample` config instance | 50 | `variable.name.st` |

In `tests/smoke.st`, `{stc:test}` names the pragma
(`keyword.other.pragma.st`), the assertion string is
`string.quoted.double.st`, and `TRUE` is `constant.language.st`. The grammar
fixtures under `../../test/grammar/` cover tokens the sample does not contain:
`IF`/`FOR`/`RETURN`, `//` line comments, `%IX0.0`, `ABS(`, and `TON`.

The extension does not set `workbench.colorTheme`. To compare the shipped
palettes, run **Preferences: Color Theme** and choose `stc Dark` or `stc
Light`. Repeat the inspection in the built-in **Dark Modern** and **Light
Modern** themes to see the documented fallback: standard scopes color
correctly, while the pragma name may use the generic keyword color.

## Debug checkpoints

The tests use two known executable lines:

| Stop | Line | Statement | Known values at the stop |
| --- | --- | --- | --- |
| First statement of `PROGRAM Main` | 39 | `sum := AddStep(sum);` | global `INCREMENT = 1`; program fields are exposed under the program instance in the Globals scope (expand `ACCUMULATOR` for its input/output). `TOTAL` reflects the cycles that ran before the breakpoints bound, so it is read as a decimal integer rather than an exact value |
| First statement of `FUNCTION AddStep` | 19 | `incremented := INCREMENT;` | local `INCREMENTED = 0`; global `INCREMENT = 1`. The `VALUE` parameter carries the caller's `sum` and is read as a decimal integer |
| After stepping over line 19 | 20 | `AddStep := value + incremented;` | local `INCREMENTED = 1` |

Root and local names are the debug metadata's normalized uppercase spellings. Values
come from the runtime's typed reads; the debug adapter does not reconstruct
them.

## Run the workflow

From this directory, with `stc` installed and on `PATH` (or with `stc.path`
pointing at the executable):

```sh
stc check
stc build
stc test
stc fmt --check
```

`stc test` builds each `{stc:test}` POU into its own process. `stc build`
writes the loadable module, its GOT manifest pair, and metadata under
`target/stc/debug`. The manifest keeps `[run].cycles = 0`, so a debug host
stays alive until the session ends.

## Debug

The launch configuration builds the project and starts a local debug session:

```json
{
  "type": "stc",
  "request": "launch",
  "name": "stc: Launch sample",
  "project": "${workspaceFolder}/stc.toml",
  "noBuild": false,
  "release": false,
  "allowWrite": false,
  "allowForce": false
}
```

Open `src/main.st`, set a breakpoint on line 39, press F5, and choose **stc:
Launch sample**. The Globals scope shows `INCREMENT` and `TOTAL`; the program
instance contains the program fields. Continue to line 19 to read the
function's locals.

The attach example in `.vscode/launch.json` uses the endpoint path
`${workspaceFolder}/target/stc/debug/debug.endpoint.json`. The `stc: run` task
starts the host with a debug server at that path, so start the task and then
start **stc: Attach sample**. You can also start the host manually:

```sh
stc run --debug-server --debug-endpoint target/stc/debug/debug.endpoint.json
```

Attach mode leaves the host running after the session ends; launch mode owns
and stops it. The driver removes a validated stale endpoint and rotates the
previous audit log to `debug.endpoint.audit.<timestamp>.jsonl` before starting
a debug server with an explicit endpoint, so repeated `stc: run` and attach
cycles work. An older `stc` binary without this behavior requires the stale
endpoint and audit files to be removed first.

Terminating the `stc: run` task currently leaves the driver and launcher
processes running; stop the process tree manually until that is fixed (UI
checklist follow-up F-3).

Writes are disabled by default. `allowWrite` enables `setVariable` for live
roots, and `allowForce` additionally requests the force scope; the host must
also grant `[debug].allow-write` or `[debug].allow-force`. `allowForce` does
not add a DAP request, so forcing remains a `stc debug` operation in v2.0.0.

Release builds are monitoring-only: no breakpoints, stepping, or Locals
scope. A `stc-debug/1`-only host has no Locals scope either.

## Online change check

The extension can run this project's online-change workflow directly:

1. Run **stc: Start Online Change** from the Command Palette. The status bar
   shows `starting`, then `gen 1 · <8 hex>` once the initial generation is
   active. Click the status item (or run **stc: Show Online Change Status**) to
   open the **stc Online Change** channel. The extension uses its own private
   per-session endpoint under extension storage, not the sample's
   `target/stc/debug/debug.endpoint.json`.
2. Run **stc: Attach Debugger to Online Change**. This attaches through
   `stc dap` with default read-only scopes; the Debug Console shows the
   non-safety notice. Breakpoints and Globals work as in the attach example.
3. Edit line 43 from `cycles := cycles + 1;` to `cycles := cycles + 2;` and
   save. The watch rebuilds and activates the new generation at a cycle
   boundary. The Debug Console shows `stc: the debug session ended: swap
   (STC-E-DEBUG-019)`, the status bar shows the new generation and build, and
   the extension offers **Attach Debugger** for the new generation. It never
   reconnects automatically; breakpoints, forces, and watches do not migrate.
4. Run **stc: Stop Online Change**. The extension closes the watch's stdin, the
   CLI stops the host gracefully and prints `[stc] watch stopped`, and the
   status returns to idle. Check that no `stc` or `sample` process remains. A
   forced stop only happens after a five-second wait and warns that retained
   state may not have been saved.

The same edit is the automated smoke's swap. `stc.onlineChange.args` can add
`--release`, `--store`, or `--watch-timeout` (and other `stc run` options) to
the extension-owned watch; the extension owns the manifest, debug-server,
endpoint, color, control, and quiet flags. The CLI equivalent remains:

```sh
stc run --watch --debug-server --debug-endpoint target/stc/debug/debug.endpoint.json
```

The endpoint path must be fresh for each watch run (the watch path does not
rotate a stale endpoint or audit file), so prefer the extension or a unique
path when repeating runs. One online-change session runs per editor window; two
windows can start two watches on the same project.

## Non-safety

`stc` can control real machinery, but it is not a safety PLC and is not
responsible for safety. Safety must be provided by a separate safety system
and machine design as required by law and regulation. A stopped or forced
controller may hold outputs indefinitely; debug and forcing are availability
and operability features, not safety functions.

See the [extension README](../../README.md) and the
[IDE integration specification](../../../../specification/Architecture/IDE-Integration.md)
for the full local contract.
