# Release notes — `stc` 1.0.0-rc.1

Date: 2026-10-29  
Profile: `stc-2013-oop-utf8-v1`  
Milestone: M7 — release candidate; M8 — Linux host portability complete (2026-10-29)

`stc` is a command-line compiler for IEC 61131-3:2013 Structured Text. It emits
deterministic textual LLVM IR, invokes `clang` to produce loadable modules,
thin launchers, and embedding host libraries, and ships a persistent runtime
with online change, retained state, I/O backends, and typed debug/monitoring.

> **`stc` is not a safety PLC and is not responsible for safety.** It can
> control real machinery, but safety must be provided by a separate safety
> system and machine design as required by law and regulation. Debug sessions
> can hold outputs indefinitely. See [`NON-SAFETY.md`](NON-SAFETY.md).

## Highlights

- **Release candidate for the v1 profile.** The workspace, driver, descriptor,
  manifest, and package all report `1.0.0-rc.1`.
- **Reproducible artifacts.** IR, GOT manifests, build IDs, debug metadata,
  sidecars, and rendered views are deterministic functions of source, options,
  and toolchain. Double builds are byte-identical, including PE timestamps.
- **Release packaging.** `cargo xtask package` builds `stc` and the runtime with
  `--locked`, writes the `bin`/`lib`/`docs` layout, a deterministic
  `package.json`, and a sorted `SHA256SUMS`. The packaged layout resolves the
  runtime without environment variables.
- **Debug-free release archives.** Static archives are unpacked, every native
  object is stripped of DWARF/CodeView, and the stripped objects replace the
  originals in place so duplicate import-library members and the symbol table
  survive; LLVM bitcode members are left untouched. The shared host library is
  stripped too. Modules and executables are stripped.
- **Cross-target smoke matrix.** Release IR and driver object emission are
  tested for x86-64 and AArch64 Linux, AArch64 big-endian, x86-64/AArch64
  macOS, x86-64/AArch64 Windows MSVC, RISC-V 64 ELF, and Thumbv7E-M.
- **Examples.** New multi-package [`examples/motor`](examples/motor/README.md)
  with [`motorlib`](examples/motorlib/README.md), and
  [`examples/online-change`](examples/online-change/README.md); the starter
  [`examples/hello`](examples/hello/README.md) is unchanged in scope.
- **Hardening.** Format-1 baselines are rejected and cold-download only;
  debug/release baselines are explicit rather than an implied hot-swap;
  truncated and tampered manifests are rejected; release archives, PDB
  absence, and debug limits are asserted; four non-debug traceability rules
  remain documented as not implemented.
- **Documentation.** This file, [`USER-GUIDE.md`](USER-GUIDE.md),
  [`KNOWN-LIMITATIONS.md`](KNOWN-LIMITATIONS.md),
  [`NON-SAFETY.md`](NON-SAFETY.md), and
  [`RELEASE-CHECKLIST.md`](RELEASE-CHECKLIST.md).

The underlying milestones remain as delivered: M4.5 online-change foundation,
M5 FBs/OOP/aggregates/UTF-8 strings, M6 project system/CLI/verification, and
M6.5 full typed debug and monitoring.

## Host portability (M8, 2026-10-29)

M8 completes the host-portability scope of `1.0.0-rc.1`. It makes no language,
format, ABI, protocol, or version change: `stc` remains `1.0.0-rc.1`. The
supported native hosts are:

| Host | Triple | Package |
| --- | --- | --- |
| Windows 11 x86-64 MSVC | `x86_64-pc-windows-msvc` | `stc-1.0.0-rc.1-x86_64-pc-windows-msvc` |
| Linux x86-64 GNU | `x86_64-unknown-linux-gnu` | `stc-1.0.0-rc.1-x86_64-unknown-linux-gnu` |
| Linux AArch64 GNU | `aarch64-unknown-linux-gnu` | `stc-1.0.0-rc.1-aarch64-unknown-linux-gnu` |

- **Host audit.** Host-specific operations are confined to the runtime platform
  layer and the driver toolchain/build layer; each portable compiler crate
  keeps only its single documented host module, and no OS-behaviour branch
  remains in the module/manifest model. `cargo xtask host-audit --check`
  enforces the allowlist in CI.
- **AArch64 debug parity.** Service-13 capture of `x29`/`sp`/`x30`, DWARF
  `DW_OP_reg29` frame-base extraction, and the existing `fp+16` stopped-frame
  rule give typed locals, breakpoints, stepping, and call stacks on AArch64
  Linux; frame-less functions are reported as optimized-out.
- **State durability and permissions.** Linux state saves create a 0600
  temporary file, sync it, rename over the target, then sync the parent
  directory; a failed replace leaves the previous file intact.
- **Service-thread priority.** Control and debug service threads run below
  normal priority (nice +10 on Linux), best effort; v1 does not pin CPU
  affinity.
- **Per-architecture packaging.** One archive per native host triple with a
  host-derived `package.json.debug_contract` (`object-codeview` on Windows
  MSVC, `dwarf` on Linux GNU), built and exercised on all three hosts.

M8 verification: CI run
[40821738218](https://github.com/r2k-in-the-vortex/stc/actions/runs/40821738218)
(commit `ba97c5a`) passed all 36 jobs on the three native hosts; the
phase evidence runs are recorded in [`M8-Plan.md`](M8-Plan.md) §18.

## Artifacts

A package contains:

```text
stc-1.0.0-rc.1-<host-triple>/
  bin/stc(.exe)
  lib/stc_runtime.lib, stc_runtime.dll        (Windows MSVC)
  lib/libstc_runtime.a, libstc_runtime.so     (Linux GNU)
  docs/{README,USER-GUIDE,RELEASE-NOTES,KNOWN-LIMITATIONS,NON-SAFETY}.md
  package.json
  SHA256SUMS
```

Verify with `sha256sum -c SHA256SUMS`. `package.json` records the host triple,
profile, artifact formats, host ABI, and the non-safety identifier, with no
timestamps or paths.

Formats and versions:

| Contract | Version |
| --- | --- |
| GOT manifest / catalog | 2 |
| Descriptor, module, state, metadata sidecar, debug object, stopped-frame sidecar | 1 |
| Host ABI | 2.3 (test 2.2, production 2.1) |
| Debug protocol | `stc-debug/1` |
| Build-ID domain | `stc:build:v1` |

## Build modes

- **Debug** is the default; it emits DWARF (ELF/Mach-O) or CodeView (MSVC),
  checkpoint hooks, the `debug_check` import, and a stopped-frame sidecar.
- **`--release`** is optimised, stripped, hook-free, and separately identified;
  it emits no debug object or frame sidecar and has a distinct build ID and
  manifest.
- **`stc test`** builds the hook-free debug form at ABI 2.2.

Keep debug and release artifacts in separate out-dirs; one explicit out-dir for
both forms is rejected (`STC-E-PROJECT-010`).

## Upgrading

- **Pre-RC build IDs are not reused.** The version change and the M7 test
  hardening alter identity inputs; rebuild both the active module and the
  candidate, or cold-download.
- **Format-1 GOT manifests are rejected.** There is no in-place migration;
  perform the documented cold download (D-192).
- **Debug/release cross-mode swaps are not a supported pair.** A release
  candidate against a debug baseline selects a cold layout; the reverse is
  classified compatible by the engine but is not a supported deployment path.
  Keep separate stores.
- **M4-era code-linked artifacts are not loadable.** Rebuild with the module
  model.
- **Windows debug information is object-level CodeView.** There is no PDB or
  DLL-level CodeView claim; the runtime uses build-time sidecars.

## Supported hosts and targets

- Native hosts: Windows 11 x86-64 MSVC (`x86_64-pc-windows-msvc`, primary
  development host), Linux x86-64 GNU (`x86_64-unknown-linux-gnu`), and Linux
  AArch64 GNU (`aarch64-unknown-linux-gnu`). Compile, execute, load modules,
  extract frames, debug, and run online changes on all three.
- IR/object emission: the cross-target matrix above, for any other clang
  target.
- macOS, Windows ARM64, 32-bit, and big-endian hosts are not supported. There
  is no CPU-affinity pinning and no remote/network debug transport.

## Verification

The release checklist records the evidence. The candidate is expected to pass:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked                       # STC_REQUIRE_CLANG=1
cargo xtask test --suite debug --locked
cargo xtask test --suite projects --locked            # includes examples and hardening
cargo xtask test --suite cross-target --locked
cargo xtask traceability --check
cargo xtask host-audit --check
python tools/ci/check_docs.py
npx --yes markdownlint-cli2@0.24.0
cargo xtask package --locked
```

CI runs the workspace, online, debug, UI, conformance, execution, projects,
cross-target, package, traceability, Lean, and fuzz-smoke jobs natively on
`windows-2025`, `ubuntu-24.04`, and `ubuntu-24.04-arm`. M7 release evidence:
CI run
[40777754963](https://github.com/r2k-in-the-vortex/stc/actions/runs/40777754963)
passed on `windows-2025` and `ubuntu-24.04` for commit `8dbcbf4`. M8 host
evidence: every phase run in [`M8-Plan.md`](M8-Plan.md) §18 is green on the
three native hosts, and the final sign-off run
[40821738218](https://github.com/r2k-in-the-vortex/stc/actions/runs/40821738218)
passed all 36 jobs on commit `ba97c5a`.
Performance measurements are informational and not gating.

## Known limitations

See [`KNOWN-LIMITATIONS.md`](KNOWN-LIMITATIONS.md). The most relevant for a
first deployment are: no field-layout migration, cold-download-only format-1
baselines, local-only debug with no raw memory or remote transport, object-only
Windows CodeView, no macOS or Windows ARM64 host, no CPU-affinity pinning,
prepare on the cycle thread rather than fully off-thread, retained state saved
at normal shutdown, and null/memory/file I/O backends only. Frame extraction
covers x86-64 and AArch64; frame-less functions are reported as optimized-out,
and caller frames carry no source location.

## Safety

`stc` can control real machinery. `stc` is not a safety PLC and is not
responsible for safety in any way. Safety must be provided by a separate
safety system and machine design as required by law and regulation. Debug and
forcing can hold outputs indefinitely and are not safety functions. Read
[`NON-SAFETY.md`](NON-SAFETY.md).
