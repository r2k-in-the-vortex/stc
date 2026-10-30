# `stc` user guide — 1.0.0

Profile: `stc-2013-oop-utf8-v1`

`stc` compiles IEC 61131-3:2013 Structured Text to textual LLVM IR, then invokes
`clang` to produce a loadable module, a thin launcher executable, or an
embedding host library. A persistent host runtime provides online change,
retained state, I/O backends, and typed debug/monitoring.

> **`stc` is not a safety PLC and is not responsible for safety.** It can
> control real machinery, but safety must be provided by a separate safety
> system and machine design as required by law and regulation. Debug sessions
> can hold outputs indefinitely. See [`NON-SAFETY.md`](NON-SAFETY.md).

## 1. Install

A release package has this layout:

```text
stc-1.0.0-<host-triple>/
  bin/stc(.exe)
  lib/stc_runtime.lib + stc_runtime.dll        (Windows MSVC)
  lib/libstc_runtime.a + libstc_runtime.so     (Linux GNU)
  docs/README.md
  docs/USER-GUIDE.md
  docs/RELEASE-NOTES.md
  docs/KNOWN-LIMITATIONS.md
  docs/NON-SAFETY.md
  package.json
  SHA256SUMS
```

A package is produced for each native host triple:

```text
stc-1.0.0-x86_64-pc-windows-msvc
stc-1.0.0-x86_64-unknown-linux-gnu
stc-1.0.0-aarch64-unknown-linux-gnu
```

Linux GNU packages therefore exist for both x86-64 and AArch64, each with a
host-derived `package.json` `debug_contract` (`object-codeview` on Windows
MSVC, `dwarf` on Linux GNU).

Unpack the archive and put `bin` on `PATH` (or call `bin/stc` directly). The
driver finds the runtime archive beside its executable or in the sibling
`lib/` directory, so no environment variable is required for the packaged
layout. Verify the download first:

```sh
sha256sum -c SHA256SUMS
stc --version
stc compliance
```

Requirements:

- `clang`/LLVM for assembling and linking IR; `llvm-objcopy` for release
  stripping; `llvm-ar` for release archive stripping; `llvm-readobj` or
  `readelf` for stopped-frame sidecar extraction.
- On Windows, clang, `llvm-objcopy`, and `llvm-ar` are normally in the LLVM
  installation directory. The driver finds them beside `clang` or on `PATH`.
- No LLVM development library is needed; the compiler emits text IR.

Toolchain discovery order follows CLI > environment > `stc.toml` > built-in
default:

| Setting | CLI | Environment | Manifest |
| --- | --- | --- | --- |
| clang | `--clang <path>` | `STC_CLANG` | `[build].clang` |
| runtime archive | `--runtime <path>` | `STC_RUNTIME` | `[build].runtime` |
| target | `--target <triple>` | `STC_TARGET` | `[build].target` |

## 2. Quick start

```sh
stc new hello
cd hello
stc check
stc build
stc run --cycles 5
stc test
stc fmt --check
```

`stc new` writes `stc.toml`, a production source tree, a `[test].sources`
tree, and a committed `stc.lock`. `stc build` writes a loadable module, its GOT
manifest pair, and metadata under `target/stc/debug`. `stc run` also links a
thin launcher and executes the configured tasks. `stc test` builds each
`{stc:test}` POU into its own process.

The committed examples are the best starting points:

- [`examples/hello`](examples/hello/README.md) — language and workflow basics.
- [`examples/motor`](examples/motor/README.md) — a two-package project with
  OOP, an FB, UTF-8 strings, I/O, retained state, tests, debug, and online
  change (`examples/motorlib` is its dependency).
- [`examples/online-change`](examples/online-change/README.md) — the minimal
  hot-swap, rename, diff, and cold-download walkthrough.

## 3. Project manifest

```toml
schema = 1

[package]
name = "motor"
version = "0.1.0"
language = "stc-2013-oop-utf8-v1"

[dependencies]
motorlib = { path = "../motorlib" }   # local path only in v1

[project]
sources = ["src/**/*.st"]
configuration = "MotorDemo"           # optional if exactly one exists

[build]
emit = ["module", "metadata"]         # module | exe | lib | ir | obj | metadata
debug = true                          # debug form (default); --release selects release
opt-level = 0                         # defaults: 0 debug, 2 release

[run]
cycles = 0                            # 0 = until interrupted
cycle-time = "10ms"

[io]
backend = "null"                      # null | memory | file
file = "io.bin"

[retain]
file = "state/motor.stcstate"
on-mismatch = "reset"                 # error | reset | cold

[test]
sources = ["tests/**/*.st"]
timeout = "5s"

[debug]
default = true
protocol = "pipe"                     # pipe | socket
session-timeout = "5min"
allow-write = false
allow-force = false
```

- `[package].name` is the package identity and namespace. Dependencies use
  local paths; the dependency's package name must match the alias unless an
  explicit `package = "..."` alias is given. The graph is acyclic and compiled
  in topological order.
- `stc.lock` pins the graph and is committed. `--locked` makes a stale graph an
  error; CI uses it everywhere.
- Production builds exclude `{stc:test}` POUs and `STC_ASSERT`. Test sources
  are compiled only by `stc test`.
- The full schema, validation rules, and diagnostic codes are in
  [`specification/Architecture/CLI-and-Manifest.md`](specification/Architecture/CLI-and-Manifest.md)
  §2.

## 4. Build modes and artifacts

| Mode | Selection | Default out-dir | Contents |
| --- | --- | --- | --- |
| Debug | default or `--debug` | `target/stc/debug` | DWARF (ELF/Mach-O) or CodeView (MSVC), checkpoint hooks, `debug_check` import, stopped-frame sidecar |
| Release | `--release` | `target/stc/release` | optimised, stripped, hook-free, distinct build ID/manifest, no debug sidecars |
| Test | `stc test` | `target/stc/test/<artifact>` | hook-free debug form, ABI 2.2 test clock, never deployed |

Artifacts for `--emit=module`:

```text
<name>.ll                     deterministic textual LLVM IR
<name>.o                      object
<name>-<build-id>.dll|.so     loadable ABI 2 module
<name>.stc-got.json           canonical-JSON GOT manifest
<name>.stc-got.bin            compact binary GOT manifest
<name>.stc-meta.json          metadata sidecar (debug object in debug form)
<name>.stc-frames.json        stopped-frame sidecar (hook-bearing debug only)
```

`--emit=exe` adds a thin launcher that loads the adjacent module pair;
`--emit=lib` adds the embedding host archive (or shared host library, by
`lib-type`). The two may be combined. Debug and release artifacts must use
distinct output directories; an explicit out-dir used for both modes is
`STC-E-PROJECT-010`.

Formats and versions in this release:

| Contract | Version |
| --- | --- |
| GOT manifest / catalog | 2 |
| Descriptor, module, state, metadata sidecar, debug object, stopped-frame sidecar | 1 |
| Host ABI | 2.3 (test 2.2, production 2.1) |
| Debug protocol | `stc-debug/1` |

IR and manifests are deterministic functions of source, options, and
toolchain: no timestamps, absolute paths, host names, user names, random IDs,
or addresses. Double builds are byte-identical, including PE timestamps.

## 5. Commands

| Command | Purpose |
| --- | --- |
| `stc new <dir>` | Scaffold a project |
| `stc check [files...]` | Lex, parse, and analyse; no artifacts |
| `stc build [files...]` | Emit selected artifacts |
| `stc run` | Build `exe,module,metadata` and run the tasks |
| `stc debug` | Build and launch a local authenticated debug session |
| `stc run --debug-server` | Run with a debug service beside the cycle loop |
| `stc test` | Build and run `{stc:test}` POUs in separate processes |
| `stc fmt [--check]` | Format with the fixed style |
| `stc clean [--debug\|--release\|--test]` | Remove generated trees; preserves the runtime store and state |
| `stc metadata [--message-format json]` | Print the build/artifact summary |
| `stc compliance [--message-format json]` | Print the profile, extensions, capabilities, and non-safety stance |
| `stc online download` | Export the active manifest from the runtime store |
| `stc online diff <candidate>` | Compare a candidate with a baseline/store/live host |
| `stc rename <old> <new>` | Source-aware tracked rename |
| `stc run --watch` | Rebuild and deploy online while watching sources |
| `stc parse <file> [--ast]` | Dump the CST (and typed AST) |

Global options include `--message-format human|short|json`,
`--color auto|always|never`, `-q`, `-v`/`-vv`, `--manifest-path`,
`--locked`, `--relaxed`, and lint controls (`-D`, `-W`, `-A`, `--deny
warnings`). Run `stc --help` for the complete list and exit codes.

## 6. Online change

```sh
stc build                       # baseline generation
stc run --watch                 # host + rebuild-on-save + cycle-boundary swap
stc online download --out active.json
stc online diff target/stc/debug/<name>.stc-got.json --baseline active.json
```

- Code-only changes and field add/remove/rename are compatible; tracked
  renames preserve `GotId`s. Type, size, array, and string layout changes are
  rejected or require a cold download.
- A failed build, prepare, or compatibility check leaves the previous
  generation active.
- Every swap ends a debug session and clears forces/breakpoints.
- Format-1 manifests are cold-download only; no migration is provided.
- Debug and release forms have distinct identities and are not a supported
  hot-swap pair; keep separate stores.
- The runtime store (`target/stc/runtime` by default) holds the active
  manifest and module; `stc clean` preserves it.

The complete model is in
[`specification/Architecture/Online-Change-and-GOT.md`](specification/Architecture/Online-Change-and-GOT.md)
and [`GOT-Artifacts.md`](specification/Architecture/GOT-Artifacts.md).

## 7. Debug and monitoring

```sh
stc debug                                   # build, launch, interactive session
stc debug --break src/main.st:10            # initial breakpoint
stc debug --attach <endpoint-file>          # attach to a running host
stc debug --batch commands.txt              # scripted commands
stc run --debug-server --cycles 0           # run with a debug endpoint
```

Targets are normalized stable key paths or `GotId` paths, for example
`MAIN.COUNTER.VALUE`, `MAIN.SUM`, or a whole aggregate. Available commands
include `print`, `set`, `force`, `release`, `release-all`, `watch`,
`unwatch`, `break`, `clear`, `continue`, `step`, `stack`, `frame`, `scope`,
`audit`, and `quit`.

- Starting a session does not stop the cycle. A stop/breakpoint/step holds
  outputs and is bounded by `[debug].session-timeout` (default five minutes);
  expiry ends the session and resumes the cycle.
- Writes and forces are queued and applied at a cycle boundary, revalidated
  for generation, root, type, layout, and writability.
- Release builds are monitoring-only; breakpoints, stepping, and full locals
  require a debug build.
- Debug is local only. There is no raw-memory access, unauthenticated mode, or
  remote transport. The endpoint file is user-only and contains the per-run
  token; never share it.
- Every swap/rollback/cold install/shutdown/exit ends the session.

See
[`specification/Architecture/Debug-and-Monitoring.md`](specification/Architecture/Debug-and-Monitoring.md)
§12–§18 and [`KNOWN-LIMITATIONS.md`](KNOWN-LIMITATIONS.md) §3 for the limits.

## 8. Tests and formatting

```sh
stc test                 # all discovered {stc:test} POUs
stc test --list          # list without building
stc test --no-run        # build artifacts only
stc test --timeout 5s
stc test -- --nocapture
```

Each test runs in its own process with a deterministic clock. `STC_ASSERT`
failures, traps, and timeouts are reported per test; `[[test.case]]` can set
cycles, mock-clock, and expected traps. Test-only code never enters production
or release artifacts.

`stc fmt` uses one fixed style over the lossless CST, preserves comments,
emits strict semicolons, and supports `--check`. It is idempotent.

## 9. Cross-target smoke tests

`stc build --target=<triple> --emit=ir` (or `obj`) emits target-independent IR
and an object for the requested triple. The release-candidate matrix covers:

```text
x86_64-unknown-linux-gnu       aarch64-unknown-linux-gnu
aarch64_be-unknown-linux-gnu   x86_64-apple-darwin
aarch64-apple-darwin           x86_64-pc-windows-msvc
aarch64-pc-windows-msvc        riscv64-unknown-elf
thumbv7em-none-eabihf
```

Execution, module loading, and frame extraction are native on Windows x86-64
MSVC, Linux x86-64 GNU, and Linux AArch64 GNU. `--emit=exe` and `--emit=lib`
reject a non-host target with `STC-E-BUILD-004`.

## 10. Performance

Timing is informational, not a real-time guarantee. Record the host and regime
when comparing numbers:

```sh
cargo test -p stc-driver --test performance -- --ignored --nocapture
cargo test -p stc-driver --test debug_overhead -- --ignored --nocapture
cargo test -p stc-runtime --test switch_latency -- --ignored --nocapture
```

Recorded figures are in
[`specification/Verification/Performance.md`](specification/Verification/Performance.md).

On Linux, `stc run --rt --cycle-time 1ms --rt-priority 90 --rt-cpu 3 --rt-metrics rt.jsonl`
runs the cycle thread with FIFO scheduling and absolute `CLOCK_MONOTONIC` pacing, and writes the
JSONL metrics described in [specification/Verification/RT-Host.md](specification/Verification/RT-Host.md) §3.4. The mode is measurement-only;
without `--rt` the run loop and the generated artifacts are unchanged.

## 11. Troubleshooting

| Symptom | Cause / action |
| --- | --- |
| `STC-E-BUILD-001` “cannot run clang” | Install clang/LLVM or pass `--clang`/`STC_CLANG` |
| `STC-E-BUILD-003` “runtime … was not found” | Use the packaged `bin` + `lib` layout, build `stc-runtime`, or pass `--runtime` |
| `STC-E-BUILD-004` cross-target executable/library | Only IR/object emission is supported off-host; link on the target host |
| `STC-W-MANIFEST-001` / `STC-W-ONLINE-005` cold layout | There is no usable baseline; the build is deployable only as a cold download |
| `STC-E-MANIFEST-002` host ABI minor | The baseline needs a newer ABI than the selected build form; use the matching mode or cold-download |
| `STC-E-DEBUG-017` stack/frame unavailable | The stopped-frame sidecar is missing or unusable (`llvm-readobj`/`readelf` at build time); typed monitoring still works |
| `STC-E-PROJECT-010` debug/release mixed | Use distinct out-dirs for the two forms; one explicit out-dir cannot hold both |
| Version or build-ID mismatch after upgrading | Pre-RC artifacts are not compatible; cold-download or rebuild both sides |

For every diagnostic, `--message-format json` gives a stable object with code,
severity, spans, notes, and related locations.

## 12. Support matrix

| Host | Triple | Package | Compile | Run/model | Debug | Online change |
| --- | --- | --- | --- | --- | --- | --- |
| Windows 11 x86-64 MSVC | `x86_64-pc-windows-msvc` | `stc-1.0.0-x86_64-pc-windows-msvc` | yes | yes | yes | yes |
| Linux x86-64 GNU | `x86_64-unknown-linux-gnu` | `stc-1.0.0-x86_64-unknown-linux-gnu` | yes | yes | yes | yes |
| Linux AArch64 GNU | `aarch64-unknown-linux-gnu` | `stc-1.0.0-aarch64-unknown-linux-gnu` | yes | yes | yes | yes |
| Other targets | any clang triple | none | IR/object emission only | no | no | no |

The three native hosts are supported from M8 (D-200); macOS, Windows ARM64,
32-bit, and big-endian hosts are not supported. CI runs the workspace, debug,
online, projects, conformance, execution, UI, cross-target, package,
traceability, Lean, and fuzz-smoke jobs natively on all three hosts.

## 13. Safety

See [`NON-SAFETY.md`](NON-SAFETY.md). In short: `stc` can control real
machinery, but it is not a safety PLC; safety must be provided by a separate
safety system and machine design as required by law and regulation. Debug and
forcing can hold outputs indefinitely and are not safety functions.
