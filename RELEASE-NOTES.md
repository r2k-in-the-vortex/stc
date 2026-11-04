# Release notes — `stc` 2.0.0

- Date: 2026-11-04
- Profile: `stc-2013-oop-utf8-v1`
- Baseline: `stc` 1.0.0 (2026-10-29)

`stc` 2.0.0 is the editor, verification, and connectivity release. It keeps the
1.0.0 compiler, runtime, language profile, and host ABI as the compatibility
baseline; the only artifact-format revision is the documented stopped-frame
sidecar update, plus the new opt-in coverage and RT report schemas. It adds
local VS Code integration, ST test coverage, the v2 verification program,
PREEMPT_RT evidence, a Modbus TCP server, and Linux EtherCAT I/O.

For a quick tour, see the [75-second VS Code walkthrough](https://youtu.be/lqABvDU5pto).

> **`stc` is not a safety PLC and is not responsible for safety.** It can
> control real machinery, but safety must be provided by a separate safety
> system and machine design as required by law and regulation. Debug sessions,
> forcing, motion, and network I/O can hold outputs indefinitely. See
> [`NON-SAFETY.md`](NON-SAFETY.md).

## Highlights

- **VS Code integration.** The `stc Structured Text` extension provides the
  TextMate grammar and language configuration; language server diagnostics,
  symbols, definition, references, hover, completion, and formatting; tasks
  and a problem matcher; a DAP debugger over `stc-debug/1` with the additive
  `stc-debug/1.1` Locals revision; online-change controls; and a Testing API
  provider with run/debug support for `{stc:test}` suites. The online-change
  start gate tracks live debug sessions synchronously, so a just-stopped
  session cannot suppress a start through a stale active-session pointer.
- **Coverage for ST tests.** `stc test --coverage` reports statement coverage
  for the ST code under test, mapped to source spans. It writes deterministic
  `coverage.json` (`stc-coverage/1`), `lcov.info`, and a human-readable
  summary under `target/stc/coverage`; production artifacts are unchanged.
- **A larger verified core.** Lean 4 checks the source and emitted-IR
  semantics, whole-run invariants, the reference interpreter, and a 74/74
  differential corpus. The exact scalar kernel shared by constant folding and
  code generation has a V1 refinement record, and the separate Aeneas/Charon
  bridge package builds on the pinned host. This is a machine-checked semantic
  foundation and kernel refinement, not a whole-compiler refinement proof; see
  the [Phase 0](specification/Verification/Reports/Phase0-Report.md),
  [Phase 1](specification/Verification/Reports/Phase1-Report.md), and
  [Phase 2](specification/Verification/Reports/Phase2-Report.md) reports.
- **Real-time evidence.** The x86-64 PREEMPT_RT campaign records cycle wake-up
  jitter, execution-time distributions, deadline behavior, online-change
  prepare/activation, and rollback under load. The opt-in runtime surface is
  `--rt`, `--rt-priority`, `--rt-cpu`, and `--rt-metrics`, with the
  `stc-rt-metrics/1` schema. Results are specific to the measured host and
  configuration; there is no hard real-time guarantee.
- **Modbus TCP server I/O.** A runtime-owned Modbus TCP server exposes the
  process image through coils, discrete inputs, input registers, and holding
  registers, with multiple listeners and an optional status device. It is
  available on all three native hosts, is server-role only, and is
  unauthenticated and non-real-time.
- **EtherCAT I/O on Linux.** The optional `stc-io-ethercat` shim provides
  cyclic process data and servo-drive integration over the IgH EtherCAT master.
  The base package has no IgH dependency and no kernel modules are
  redistributed. It is Linux-only and has no safety function.
- **Coverage gate.** The CI coverage job is now a pass/fail gate at 85% line
  coverage. The final release run records 85.28% line coverage (99,696/116,909).
- **No new language surface.** v2.0.0 adds tooling, verification, and I/O
  around the released compiler; it does not add core ST language or compiler
  features.

## Delivered targets

| Target | Delivered in 2.0.0 | Evidence |
| --- | --- | --- |
| WL-1.1 — local VS Code integration | Extension, LSP, DAP, tasks, diagnostics, online-change status, Testing API, VSIX, sample workspace, headless smoke | [IDE Integration](specification/Architecture/IDE-Integration.md), [VS Code README](editors/vscode/README.md) |
| WL-2 — coverage to 85% | CI coverage gate at 85%; final run records 85.28% line coverage | Workflow run [41889698752](https://github.com/r2k-in-the-vortex/stc/actions/runs/41889698752) |
| WL-3 — Lean verification | P0.3–P0.7 and Phases 1–2; source/target semantics, 74/74 corpus, exact-kernel V1 bridge record | [Phase 0](specification/Verification/Reports/Phase0-Report.md), [Phase 1](specification/Verification/Reports/Phase1-Report.md), [Phase 2](specification/Verification/Reports/Phase2-Report.md) |
| WL-4 — RT verification | x86-64 PREEMPT_RT campaign, harness, host report, runbook | [RT-Host.md](specification/Verification/RT-Host.md), [RT-Runbook.md](specification/Verification/RT-Runbook.md) |
| WL-5.1 — Modbus TCP server | Server backend, mapping, status, cross-host tests, mixed bench | [Modbus-Server.md](specification/Verification/Modbus-Server.md) |
| WL-5.2 — EtherCAT on Linux | IgH backend, routing, fault/recovery, DIEWU HIL, runbook | [EtherCAT-Host.md](specification/Verification/EtherCAT-Host.md), [EtherCAT-Runbook.md](specification/Verification/EtherCAT-Runbook.md) |
| WL-10 — ST test coverage | `stc test --coverage`, `stc-coverage/1`, LCOV, deterministic golden evidence | [Testing and Verification](specification/Architecture/Testing-and-Verification.md) §2.6, [examples/coverage](examples/coverage/README.md) |

## Verification

The release commit `f49d233` passed the full CI run
[41889698752](https://github.com/r2k-in-the-vortex/stc/actions/runs/41889698752)
on 2026-11-04: **39/39 jobs**, with no skipped or failed jobs, on
`windows-2025`, `ubuntu-24.04`, and `ubuntu-24.04-arm`. The run includes the
workspace tests, online change, debug suites, UI/conformance/execution,
projects, cross-target IR, packages, the VS Code extension smoke, EtherCAT
shim, fuzz smoke, traceability, docs, and **Lean proofs**. The Lean job
passed; the extension smoke includes the debug-stop/online-change sequencing
fix.

The separate bridge package check ran on the pinned `ecat-host` at the same
commit:

- `cd lean/aeneas && lake build` completed **1814 jobs** with exit status 0.
- The pinned tools are Aeneas `ab34bd2` and Charon `7b462d9`.
- `tools/ci/check_bridge_toolchain.py --check-environment` passed; the three
  source pins match and the checked-in exact model is present.
- Aeneas/Mathlib is deliberately **not** part of fast CI; the bridge package
  is a separate manual/nightly check.

This was a build and provenance check, not a fresh Aeneas/Charon regeneration.
The exact model is current by source-hash and generated-body-hash match with
the pinned record; the wire model remains intentionally absent.

The same CI run built the Windows x86-64, Linux x86-64, and Linux AArch64
`2.0.0` packages and the `stc-vscode-0.1.0.vsix`. All three package
`SHA256SUMS` files verify, and each package reports `2.0.0`, host ABI 2.3, and
stopped-frame sidecar format 2.

The release checklist records the detailed evidence and the remaining
release-identity items. The exact-kernel V1 claim applies only to the modeled
scalar kernel; the rest of the compiler and bridge remain at their documented
V0/evidence levels. See [Semantics](specification/Verification/Semantics.md),
[Verification Program Plan](specification/Verification/VerificationProgramPlan.md),
and the reports above.

## Compatibility and upgrading

`stc` 2.0.0 is additive around the 1.0.0 baseline:

- Existing ST projects, manifests, build modes, online change, retained state,
  and debug workflows continue to use the same language profile.
- The GOT manifest/catalog remains format 2; descriptor, module, state,
  metadata sidecar, and debug object remain format 1.
- The stopped-frame sidecar is format 2. Rebuild debug artifacts with the
  matching compiler rather than mixing sidecars across versions.
- Host ABI remains 2.3 (test 2.2, production 2.1).
- `stc-debug/1` clients remain supported. `stc-debug/1.1` is an additive
  revision for Locals and immediate stopped writes; it does not change the
  endpoint file's baseline protocol field.
- `stc-coverage/1` and `stc-rt-metrics/1` are new report schemas. Coverage is
  emitted only by test builds; production builds and release artifacts contain
  no coverage instrumentation.
- The VS Code extension is local-only and does not bundle a compiler. Install a
  local `stc` and set `stc.path` or put `bin/` on `PATH`.
- Network-facing I/O is opt-in. Modbus TCP is a server, not a client, and is
  unauthenticated and non-real-time. EtherCAT is Linux-only and requires the
  operator-installed IgH master.

No format-1 GOT manifest migration is added. Follow the existing cold-download
rule for format-1 baselines, and keep debug and release stores separate.

## Artifacts

The release is prepared as one package per native host:

```text
stc-2.0.0-<host-triple>/
  bin/stc(.exe)
  lib/...                                  (Windows MSVC or Linux GNU runtime)
  docs/{README,USER-GUIDE,RELEASE-NOTES,KNOWN-LIMITATIONS,NON-SAFETY}.md
  package.json
  SHA256SUMS
```

The editor is shipped as `stc-vscode-0.1.0.vsix`. Verify the package with
`sha256sum -c SHA256SUMS`; `package.json` records the host triple, profile,
formats, host ABI, and the non-safety identifier.

## Supported hosts and targets

- Native hosts: Windows 11 x86-64 MSVC (`x86_64-pc-windows-msvc`), Linux x86-64
  GNU (`x86_64-unknown-linux-gnu`), and Linux AArch64 GNU
  (`aarch64-unknown-linux-gnu`). Compile, execute, load modules, extract frames,
  debug, and run online changes on all three.
- IR/object emission: the cross-target matrix in CI for x86-64/AArch64 Linux,
  AArch64 big-endian, x86-64/AArch64 macOS, x86-64/AArch64 Windows MSVC,
  RISC-V 64 ELF, and Thumbv7E-M.
- EtherCAT hardware evidence is Linux x86-64 only. macOS, Windows ARM64,
  32-bit, and big-endian hosts are not supported.

## Known limitations

The v1 limitations remain unless explicitly changed. The v2 additions are:

- **VS Code is local-only.** No remote IDE, remote source mapping, or remote
  attachment; one debug session per host. Release/hook-free builds are
  monitoring-only. See [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) §4.
- **Modbus TCP is server-only and non-real-time.** No client/master, RTU, TLS,
  authentication, or third-party client interoperability claim. Bind explicit
  interfaces and treat every reachable writable channel as untrusted. See
  [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) §10.
- **EtherCAT is Linux-only and hardware-evidence-limited.** IgH is an operator
  prerequisite; static configuration only; HIL used one DIEWU digital slave,
  no servo hardware, and no AArch64 hardware verification. See
  [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) §8.
- **RT results are host-specific.** The campaign is evidence, not a hard
  real-time or safety guarantee; the tuned boot profile was not executed. See
  [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) §7.
- **Coverage is statement coverage.** No branch, condition, or MC/DC coverage;
  coverage is not a correctness claim. See
  [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) §9.
- **Formal verification is scoped.** The Lean proofs and the V1 exact-kernel
  refinement are relative to the documented TCB; the rest of the compiler and
  bridge are not claimed as fully verified. The Aeneas/Charon package is
  separate from fast CI.
- **MQTT is not in 2.0.0.** Process-image MQTT I/O remains deferred and is
  planned for a future release. Modbus client capability, ESI-based EtherCAT
  configuration, motion control, remote IDE debugging, and the WebUI are also
  deferred; see [FutureWishlist.md](FutureWishlist.md).

## Not in this release

- WL-1.2 — remote IDE debugging.
- WL-6 — motion controller.
- WL-7 — ESI-based EtherCAT device configuration.
- WL-8 — tracing UI.
- WL-9 — WebUI for basic runtime control.
- WL-11 — Modbus TCP client capability.
- WL-12 — sub-millisecond cycle-thread wake-up work.
- WL-13 — Lean security verification for the Modbus TCP server boundary.
- WL-14 — macOS and Windows/macOS soft-real-time test work.
- WL-15 — MQTT client for process-image I/O.

## Safety

`stc` can control real machinery. `stc` is not a safety PLC and is not
responsible for safety in any way. Safety must be provided by a separate
safety system and machine design as required by law and regulation. Debugging,
forcing, online change, Modbus TCP, and EtherCAT can hold outputs or move
machinery and are not safety functions. Read [`NON-SAFETY.md`](NON-SAFETY.md).
