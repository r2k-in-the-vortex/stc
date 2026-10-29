# motorlib — dependency package

A small reusable package used by [`../motor`](../motor/README.md). It is a
normal `stc` package with no configuration of its own; the application package
provides the configuration and task.

> `stc` is not a safety PLC and is not responsible for safety. It can control
> real machinery, but safety must be provided by a separate safety system and
> machine design as required by law and regulation.

## What it shows

- `INTERFACE` with method prototypes and a `CLASS` implementing it.
- A `FUNCTION_BLOCK` with persistent state and a public output.
- An enumeration and a subrange (`Percent : INT(0..100)`).
- A UTF-8 string result (`USTRING[16]`) built with `CONCAT` and `CASE`.
- `MIN`/`MAX` standard functions.

## Commands

```sh
stc check --manifest-path examples/motorlib/stc.toml --locked
```

The package is compiled as part of the root `motor` build; it is not meant to
be deployed by itself. Its committed `stc.lock` records the empty dependency
graph so `--locked` works in CI.
