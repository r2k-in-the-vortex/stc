# Release notes — `stc` 1.0.0-rc.1

Date: 2026-10-28  
Profile: `stc-2013-oop-utf8-v1`  
Milestone: M7 — release candidate

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

- Compile: Windows 11 x86-64 MSVC (primary), Linux x86-64 GNU (CI).
- Execute, load modules, and extract frames: x86-64 Windows MSVC and x86-64
  Linux GNU.
- IR/object emission: the M7 cross-target matrix above.
- Linux AArch64 host support, execution, and AArch64 frame rules are M8
  (D-042, D-195).

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
python tools/ci/check_docs.py
npx --yes markdownlint-cli2@0.24.0
cargo xtask package --locked
```

CI runs the workspace, online, debug, UI, conformance, execution, projects,
cross-target, traceability, Lean, and fuzz-smoke jobs on Windows and Ubuntu.
Performance measurements are informational and not gating.

## Known limitations

See [`KNOWN-LIMITATIONS.md`](KNOWN-LIMITATIONS.md). The most relevant for a
first deployment are: no field-layout migration, cold-download-only format-1
baselines, local-only debug with no raw memory, object-only Windows CodeView,
x86-64-only frame extraction, prepare on the cycle thread rather than fully
off-thread, retained state saved at normal shutdown, and null/memory/file I/O
backends only.

## Safety

`stc` can control real machinery. `stc` is not a safety PLC and is not
responsible for safety in any way. Safety must be provided by a separate
safety system and machine design as required by law and regulation. Debug and
forcing can hold outputs indefinitely and are not safety functions. Read
[`NON-SAFETY.md`](NON-SAFETY.md).
