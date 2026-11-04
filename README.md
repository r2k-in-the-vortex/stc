# stc

**Structured Text, compiled like software.** `stc` is an independent IEC
61131-3 compiler and runtime. It compiles Structured Text to native code
through LLVM and clang, changes logic on a running controller at the next cycle
boundary, and brings tests, coverage, formatting, and a typed debugger to PLC
projects — with first-class VS Code support, a machine-checked semantic core,
a proved exact scalar kernel, and a Cargo-style CLI, on Windows and Linux.

[![CI](https://github.com/r2k-in-the-vortex/stc/actions/workflows/ci.yml/badge.svg)](https://github.com/r2k-in-the-vortex/stc/actions/workflows/ci.yml)
[![Coverage](https://raw.githubusercontent.com/r2k-in-the-vortex/stc/badges/coverage.svg)](https://github.com/r2k-in-the-vortex/stc/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/r2k-in-the-vortex/stc)](https://github.com/r2k-in-the-vortex/stc/releases)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

> [!WARNING]
> `stc` is **not a safety PLC**. It can control real machinery, but safety must
> be provided by a separate certified safety system. Debug sessions can hold
> outputs indefinitely. [Safety statement](NON-SAFETY.md)

Built for controls engineers who want modern tooling — and for software teams
bringing automation into their existing build and review workflow.

## Why stc

- **Native, not virtual.** ST → textual LLVM IR → `clang` → machine code. No
  VM, no LLVM development library, no proprietary vendor runtime. Deterministic,
  inspectable artifacts; IR and objects for any target clang supports, and
  native packages for Windows x64, Linux x64, and Linux ARM64.
- **Change logic while it runs.** Prepare the next generation in the
  background, activate it at a cycle boundary, preserve compatible state, and
  roll back a failed candidate.
- **Debug like software.** Typed read/write/force/watch, breakpoints,
  stop-the-world stepping, and call stacks with call-site source at any depth —
  in VS Code or from the CLI. Debug builds by default; `--release` is stripped
  and hook-free.
- **Your editor, not a vendor IDE.** A first-class VS Code extension brings
  highlighting, live diagnostics, go-to-definition, references, hover,
  completion, formatting, tasks, launch/attach debugging, and Testing API
  run/debug for `{stc:test}` suites; online-change controls live in the status
  bar. The contract is [IDE Integration](specification/Architecture/IDE-Integration.md);
  the Cargo-style CLI remains for automation and CI.
- **Proof, not just tests.** Lean 4 machine-checks the core source and emitted-IR
  semantics — determinism, type soundness, trap soundness, memory and GOT
  safety, and no modeled undefined behavior — and proves the exact scalar kernel
  behind constant folding and code generation equivalent to its Lean model,
  relative to the documented trusted base. A reference interpreter and a 74/74
  differential corpus back the implementation.

## See it

<!-- markdownlint-disable-next-line MD034 -->
https://github.com/user-attachments/assets/9cfd6ef4-a3c6-408d-b785-d558bcc09d9c

The 75-second walkthrough shows the intended workflow: browse types and
interfaces, read compiler diagnostics, format, set a breakpoint, inspect
Globals and Locals, then edit and hot-swap the running logic from the status
bar. [Watch it on YouTube](https://youtu.be/lqABvDU5pto).

The extension is a local client for `stc`; the compiler and runtime do the
work. Everything runs on the machine.

## Machine-checked core semantics

The v2.0.0 verification program is larger than originally scoped. Lean 4
models the core source language and the emitted-IR fragment, and proves:

- source determinism, progress, preservation, type soundness, and trap soundness;
- target determinism, progress, memory safety, GOT soundness, trap soundness,
  checked-arithmetic refinement, and no modeled undefined behavior;
- executable reference models are equivalent to the declarative judgments;
- whole-run invariant preservation, including the run-level halt theorem.

A frozen 74-case corpus runs through the reference interpreter and the compiled
artifact; the committed report passes 74/74.

Phase 2 adds a formal refinement for the scalar core. The exact kernel behind
checked arithmetic, division, bitwise and shift/rotate operations, conversions,
comparisons, selection, and string lengths is proved equivalent to its Lean
model through Aeneas/Charon. The analyzer's constant folder and the code
generator share that single kernel, and a build-time certificate check rejects
any folded value that does not recompute through it.

This is a machine-checked semantic foundation, a V1 refinement of the exact
scalar kernel, and differential conformance evidence — not a whole-compiler
refinement proof. Scope, trusted base, and version freeze are in the
[Phase 0 report](specification/Verification/Reports/Phase0-Report.md) and
[Phase 2 report](specification/Verification/Reports/Phase2-Report.md).

## What you can do

| Goal | What `stc` provides |
| --- | --- |
| Write a real ST project | Functions, FBs, classes/interfaces/inheritance, namespaces, references, arrays/structs/enums/subranges, `STRING`/`WSTRING`/UTF-8 `USTRING`, the standard FB library, tasks and configurations |
| Build native artifacts | Deterministic IR, objects, loadable modules, thin launchers, static/shared embedding libraries |
| Work in VS Code | Highlighting and themes, live diagnostics, document symbols, go-to-definition, references, hover, completion, formatting, tasks, problem matcher, launch/attach debugging, online-change controls |
| Update a running controller | Online change with background prepare, cycle-boundary activation, state preservation, one-cycle rollback, rename/diff/watch |
| Keep state across restarts | File-backed `RETAIN` state with layout hash and CRC; warm and cold restart |
| Wire I/O | Runtime-owned device tree; null/memory/file; Modbus TCP server; EtherCAT via the optional `stc-io-ethercat` shim on Linux |
| Find bugs | Typed debug session; breakpoints, stepping, call stack; Globals/Locals; force/watch; local authenticated protocol |
| Keep quality | Isolated `{stc:test}` suites; statement coverage (`coverage.json` + `lcov.info`); formatter with `--check` |
| Embed the runtime | `--emit=lib` produces a C ABI host library with generated code in a separate module |

## Connectivity

PLC projects need to talk to the plant. `stc` exposes the runtime-owned process
image to fieldbuses and plant systems.

| Modbus TCP | EtherCAT | MQTT |
| :---: | :---: | :---: |
| [![Modbus TCP](docs/assets/connectivity/modbus-tcp.png)](specification/Verification/Modbus-Server.md) | [![EtherCAT](docs/assets/connectivity/ethercat.png)](specification/Verification/EtherCAT-Host.md) | ![MQTT](docs/assets/connectivity/mqtt.png) |
| **Available** in v2.0.0 | **Available on Linux** in v2.0.0 | **Planned** for the next version |
| Server for coils, discrete inputs, input and holding registers | Cyclic process data and servo-drive I/O through `stc-io-ethercat` | Process-image I/O client for an external broker |

Network-facing I/O is not a safety function: the Modbus TCP server is
unauthenticated and non-real-time, and EtherCAT is Linux-only in v2.0.0.

## How it works

```text
ST source → lexer → lossless CST → typed AST → name resolution/type checking
          → HIR → textual LLVM IR → clang → module / launcher / library
```

`stc` emits textual LLVM IR and lets `clang` select the target and link. It
does not link against the LLVM C/C++ API, so the compiler stays independent of
LLVM binding versions and the IR stays inspectable. The persistent host owns
the GOT, process image, retained state, and debug services; generated module
code never owns persistent storage.

## New in v2.0.0

- **VS Code integration.** Syntax highlighting, language server (diagnostics,
  symbols, definition, references, hover, completion, formatting), tasks and
  problem matcher, DAP debugging, and online-change controls.
- **Stronger Lean verification than originally planned.** Source and target
  semantics, whole-run invariant preservation, a reference interpreter, a
  74/74 differential corpus, and a formal refinement of the exact scalar kernel
  shared by constant folding and code generation.
- **EtherCAT I/O on Linux.** The optional `stc-io-ethercat` shim is loaded at
  run time; the base package has no IgH dependency and no kernel modules are
  redistributed.
- **Modbus TCP server I/O.** Coils, discrete inputs, input and holding
  registers, multiple listeners, and an optional status device.
- **Statement coverage for ST tests.** `stc test --coverage` writes
  deterministic `coverage.json` and `lcov.info`.
- **Verification.** An 85% CI line-coverage gate and measured cycle timing and
  jitter on x86-64 PREEMPT_RT.

## Quick start

### 1. Install the compiler

Download the [v2.0.0 release](https://github.com/r2k-in-the-vortex/stc/releases/tag/v2.0.0)
for Windows x64, Linux x64, or Linux ARM64, unpack it, and put `bin/` on
`PATH`. Only `clang`/LLVM tools are required; there is no LLVM
development-library dependency.

### 2. Install the VS Code extension

Install **stc Structured Text** from the same release (VSIX). The extension
requires VS Code 1.82 or newer and a local `stc` on `PATH` (or the `stc.path`
setting). It starts `stc lsp` for diagnostics and navigation and `stc dap` for
debugging; it does not bundle a compiler.

### 3. Create and open a project

```sh
stc new hello
cd hello
code .
```

`stc new` writes `stc.toml`, a production source tree, a separate test source
set, and a committed `stc.lock`.

### 4. Build, run, and debug

Run the `stc: build`, `stc: run`, or `stc: test` task from the Command
Palette, or press `F5` to launch the debugger. Set a breakpoint in a `.st`
file and inspect Globals and Locals while the program runs.

The same workflow from the terminal:

```sh
stc check
stc build
stc run --cycles 5
stc test
stc test --coverage
stc fmt --check
```

```text
test SmokeTest ... ok
test result: ok. 1 passed; 0 failed; 0 timed out; 0 trapped
```

Next: [User Guide](USER-GUIDE.md) · [Examples](examples/) ·
[Conformance Profile](specification/01-Conformance-Profile.md)

## Examples

| Example | Demonstrates |
| --- | --- |
| [`hello`](examples/hello/README.md) | First project: language basics, FBs, TON, tasks, isolated tests |
| [`motor`](examples/motor/README.md) + [`motorlib`](examples/motorlib/README.md) | Two-package project: OOP, UTF-8 strings, I/O, RETAIN, debug, online change |
| [`online-change`](examples/online-change/README.md) | Minimal hot-swap, rename, diff, and cold-download walkthrough |
| [`coverage`](examples/coverage/README.md) | Statement coverage with committed golden reports |

## Project status

**v2.0.0** is the current release; **v1.0.0** remains the compatibility
baseline. Language profile: `stc-2013-oop-utf8-v1`.

- CI is green on Windows x64, Linux x64, and Linux ARM64.
- Line coverage is gated at 85% (85.29% at the v2.0.0 gate).
- Lean 4 machine-checks the core source and emitted-IR semantics, and the exact
  scalar kernel is proved equivalent to its Lean model; a reference interpreter
  and a 74/74 differential corpus back the implementation.
- Double builds are byte-identical, and fuzz targets cover the front end and
  artifact decoders.
- Cycle timing and jitter are measured on x86-64 PREEMPT_RT; results are
  host-specific and are not a hard real-time claim.

Known limits are in [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md). `stc` is not
a safety PLC; see [NON-SAFETY.md](NON-SAFETY.md).

## Documentation

| Start here | Go deeper |
| --- | --- |
| [User Guide](USER-GUIDE.md) | [Architecture](specification/Architecture/Architecture.md) |
| [Conformance Profile](specification/01-Conformance-Profile.md) | [Runtime and Execution](specification/Architecture/Runtime-and-Execution.md) |
| [Examples](examples/) | [Online Change and GOT](specification/Architecture/Online-Change-and-GOT.md) |
| [v2.0.0 plan](V2.0.0-PLAN.md) | [Debug and Monitoring](specification/Architecture/Debug-and-Monitoring.md) |
| [v1.0.0 release notes](RELEASE-NOTES.md) | [I/O Routing and Backends](specification/Architecture/IO-Routing-and-Backends.md) |
| [Known Limitations](KNOWN-LIMITATIONS.md) | [Testing and Verification](specification/Architecture/Testing-and-Verification.md) |
| [Phase 0 report](specification/Verification/Reports/Phase0-Report.md) · [Phase 2 report](specification/Verification/Reports/Phase2-Report.md) | [Core semantics](specification/Verification/Semantics.md) |
| [Non-Safety Statement](NON-SAFETY.md) | [Decisions](specification/Decisions.md) |

Verification evidence: [Modbus TCP server](specification/Verification/Modbus-Server.md),
[EtherCAT host](specification/Verification/EtherCAT-Host.md),
[PREEMPT_RT host](specification/Verification/RT-Host.md), and
[performance](specification/Verification/Performance.md).

## Build from source

```sh
git clone https://github.com/r2k-in-the-vortex/stc.git
cd stc
cargo xtask package --locked --out dist
```

The package is written under `dist/stc-<version>-<host-triple>/` and contains
`bin/`, `lib/`, `docs/`, `package.json`, and `SHA256SUMS`. The binary is
`bin/stc` (`bin/stc.exe` on Windows). For development:

```sh
cargo test --workspace --locked
python tools/ci/check_local.py
```

`check_local.py` runs the same checks as the CI matrix: formatting, clippy,
workspace tests with clang required, specification and link checks, Markdown
lint, and the separate fuzz workspace.

The compiler, runtime, driver, and test runner live under `crates/`; the VS
Code extension under `editors/vscode/`; the design set under `specification/`;
and runnable projects under `examples/`.

## License

`stc` is dual-licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT license](LICENSE-MIT)

at your option. The repository includes `LICENSE-APACHE` and `LICENSE-MIT`.
Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in this project shall be dual-licensed as above,
without any additional terms or conditions.
