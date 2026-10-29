# motor — multi-package example

`motor` is the release-candidate example for a real project shape: a reusable
package (`../motorlib`) plus an application package that supplies the
configuration, direct I/O, retained state, a test suite, and the debug and
online-change workflow.

> `stc` is **not a safety PLC** and is not responsible for safety. It can
> control real machinery, but safety must be provided by a separate safety
> system and machine design as required by law and regulation. This example
> drives a simulated demo device. A stopped or forced controller may hold
> outputs indefinitely.

## What it shows

| Feature | Where |
| --- | --- |
| Local path dependency, package namespace, topological build, `stc.lock` | `stc.toml`, `../motorlib` |
| Interface, implementing class, methods, inheritance of state | `../motorlib/src/motorlib.st` |
| Function block with persistent state, enum, subrange, `CASE` | `../motorlib/src/motorlib.st` |
| UTF-8 `USTRING`, `CONCAT`, `LEN_CODE_UNIT` | `../motorlib/src/motorlib.st`, tests |
| Configuration/task, direct `AT` variables with `{stc:io}` hints | `src/motor.st` |
| Retained variable and file-backed state policy | `stc.toml`, `src/motor.st` |
| Isolated `{stc:test}` suite over the dependency package | `tests/motor_test.st` |

## Workflow

```sh
stc check
stc build                 # target/stc/debug
stc build --release       # target/stc/release, stripped and hook-free
stc run --cycles 5
stc test
stc fmt --check
```

From the repository root, prefix the commands with
`cargo run -p stc-driver --` and add `--manifest-path examples/motor/stc.toml`.

### Debug

```sh
stc run --debug-server --cycles 0
# or, interactively:
stc debug
```

The program instance is `motor`, so the typed path to the held output is
`MOTOR.ENABLE`; `print MOTOR.ENABLE`, `watch MOTOR.DRIVE.SPEED 500`, and
`force MOTOR.ENABLE TRUE` work in the session. Release builds are
monitoring-only. Every swap ends the session.

### Online change

The root package builds against the active manifest store when one exists.
Edit `src/motor.st`, for example add a new `VAR` field, then rebuild while a
host is running:

```sh
stc run --watch
```

`stc run --watch` rebuilds on source changes, prepares the candidate between
scans, and activates it at a cycle boundary. Type or layout changes that v1
cannot migrate are rejected with a compatibility report; remove or change the
field back, or perform the documented cold download. See
[`../online-change`](../online-change/README.md) for the minimal walkthrough.

## Layout

```text
motor/
  stc.toml       root package: dependency, configuration, I/O, retain, tests
  src/motor.st   program, direct variables, task configuration
  tests/         {stc:test} POUs
  state/         retained state (ignored; created by a run)
```
