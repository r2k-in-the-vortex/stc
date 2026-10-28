# hello

A small, self-contained `stc` project and the recommended first example for the
implemented language core and project workflow.

## What it shows

| Feature | Where |
| --- | --- |
| Manifest, source globs, configuration selection, and the separate test source set | `stc.toml` |
| Constants, scalar variables, and operators | `src/main.st` |
| `FUNCTION` and `FUNCTION_BLOCK` POUs | `src/main.st` |
| `IF`/`ELSIF`/`ELSE`, `FOR`, `CASE`, `WHILE`, and `REPEAT` | `src/main.st`, `tests/hello_test.st` |
| A standard function block (`TON`) and a `TIME` literal | `src/main.st` |
| `CONFIGURATION`/`RESOURCE`/`TASK` with a program instance | `src/main.st` |
| Isolated unit tests with `STC_ASSERT` | `tests/hello_test.st` |

`src/main.st` is the production source. The test source set is compiled only by
`stc test`, so test POUs never appear in production artifacts.

## Layout

```text
hello/
  stc.toml       project manifest
  stc.lock       pinned dependency graph (the project has no dependencies)
  src/main.st    function, function block, program, configuration
  tests/         {stc:test} POUs
  target/        build output (ignored)
```

## Run the workflow

From the repository root, using the workspace binary:

```sh
cargo run -p stc-driver -- check --manifest-path examples/hello/stc.toml
cargo run -p stc-driver -- build --manifest-path examples/hello/stc.toml
cargo run -p stc-driver -- run --manifest-path examples/hello/stc.toml --cycles 5 -v
cargo run -p stc-driver -- test --manifest-path examples/hello/stc.toml
cargo run -p stc-driver -- fmt --manifest-path examples/hello/stc.toml --check
cargo run -p stc-driver -- clean --manifest-path examples/hello/stc.toml
```

From this directory, with an installed `stc`:

```sh
stc check
stc build
stc run --cycles 5 -v
stc test
stc fmt --check
stc clean
```

`stc build` uses the manifest's `[build].emit` selection and writes a loadable
module with its GOT manifest and metadata under `target/stc/debug`. `stc run
--cycles 5` also builds an executable launcher and runs five cycles; `-v` makes
the host print the cycle count and the scheduled task. The manifest keeps
`[run].cycles = 0` (run until interrupted), which is what a debug host needs.
`stc test` builds each `{stc:test}` POU into its own artifact and runs it in a
separate process.

`stc build --release` produces the optimised, stripped form under
`target/stc/release`. The full command surface is in the repository
[README](../../README.md) and [CLI and Manifest](../../Specification/Architecture/CLI-and-Manifest.md).

### Debugging

`stc debug` builds the debug form, launches the host with a local authenticated
debug endpoint, and opens an interactive session. Because the manifest keeps
`[run].cycles = 0`, the host stays alive until you quit the session:

```sh
stc debug
```

Program fields are addressed by their instance path, so in this project use
`print MAIN.SUM`, `print MAIN.COUNTER.VALUE`, or `print INCREMENT` for the
global constant. `watch MAIN.SUM 500` polls a value every 500 ms. `print` and
`watch` are read-only; writes and forces additionally need `--allow-write` or
`--allow-force`, then use `set <path> <value>` or `force <path> <value>`.

Debug is local-only. For automation, `--batch <file>` runs JSON-lines requests
instead of the interactive loop. The endpoint file and audit log are created
under the active out-dir, and the endpoint is removed on clean shutdown. If
`[run].cycles` is nonzero, the host can finish before the CLI connects and the
launch fails with `STC-E-DEBUG-020`; use `--cycles 0` for debugging.

### About `STC-W-MANIFEST-001`

A standalone `stc build` has no online-change baseline, so it prints
`STC-W-MANIFEST-001` and uses a cold layout: every persistent `GotId` is newly
allocated. That is expected for a first build and for `stc run` on a development
machine; `stc test` builds cold test artifacts and suppresses the warning. To
build against an existing manifest, pass it explicitly:

```sh
stc build --got-manifest target/stc/debug/hello.stc-got.json
```

The build then reports the compatibility verdict and the preserved/allocated
counts. For a deployed controller, use the active manifest from
`[runtime].manifest-dir` with `[online].baseline = "active"` instead of a local
candidate; a plain build warns again because the driver does not assume the
previous local artifact is the active generation. `-A STC-W-MANIFEST-001`
suppresses the warning without supplying a baseline. The baseline and plan
contract is in [Online Change and GOT](../../Specification/Architecture/Online-Change-and-GOT.md).

## Where to go next

The rest of the implemented language and runtime is documented in the
[specification](../../Specification/): arrays, records, strings, references, and
classes in [Grammar](../../Specification/Syntax/Grammar.md); I/O devices and
retained state in [I/O and Devices](../../Specification/Architecture/IO-and-Devices.md)
and [State and RETAIN](../../Specification/Architecture/State-and-Retain.md);
tasks, tests, and CI in [Testing and Verification](../../Specification/Architecture/Testing-and-Verification.md);
and debug and monitoring in [Debug and Monitoring](../../Specification/Architecture/Debug-and-Monitoring.md).
