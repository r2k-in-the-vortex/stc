# M8 Implementation Plan — Linux Host Portability

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-28  
Status: In progress. Phase 0 fixes the host contract and decisions D-200–D-208; phases 1–8
implement and verify native Linux x86-64 and Linux AArch64 support.
Inputs: [Project Plan](Specification/00-Project-Plan.md) §8 (M8), `handover_to_M8.md`,
[Architecture](Specification/Architecture/Architecture.md),
[Runtime and Execution](Specification/Architecture/Runtime-and-Execution.md),
[State and RETAIN](Specification/Architecture/State-and-Retain.md),
[CLI and Manifest](Specification/Architecture/CLI-and-Manifest.md),
[Debug and Monitoring](Specification/Architecture/Debug-and-Monitoring.md),
[Testing and Verification](Specification/Architecture/Testing-and-Verification.md),
and Decisions D-042 and D-188–D-199.

M8 turns the CI-only Linux support of M7 into a supported, tested native-host capability on both
Linux x86-64 and Linux AArch64. It adds no language surface and changes no module, descriptor,
state, metadata, debug-object, stopped-frame, manifest, host-ABI, service-ID, build-ID, or
`stc-debug/1` contract. The work is a portability pass: host operations are audited and confined,
AArch64 reaches the same debug/execution behaviour as x86-64, Linux state saves become
power-loss-shaped, service threads get an explicit priority policy, and CI proves both
architectures on every required suite.

## 1. Objective

Make `stc` and its runtime first-class native products on Linux x86-64 and Linux AArch64: every
workspace test, the project workflow, the execution/conformance/online/manifest/debug suites, the
cross-target matrix, and release packaging run on both Linux architectures with no silently
skipped platform assertions.

## 2. Scope

### 2.1 In scope

| Area | Work |
| --- | --- |
| Host audit | Classify every host-specific operation; confine OS calls to the runtime platform layer and the driver toolchain/build layer; enforce the boundary in CI |
| Linux x86-64 | Re-run and record the full M7 verification list natively; fix any remaining host assumption |
| Linux AArch64 | Native builds and full suites; AArch64 service-13 frame capture, DWARF frame extraction, and stack/step support equivalent to x86-64 |
| Toolchain | clang/linker/tool discovery on Linux (`llvm-ar`/`ar`, `llvm-objcopy`/`objcopy`, `llvm-readobj`/`readelf`), GNU link arguments, artifact naming |
| State | Linux atomic replace durability (file and parent-directory sync), user-only state-file permissions, retained-state round-trip |
| Loader | `dlopen`/`dlsym`/`dlclose`, generation pinning and unload grace verified on both Linux architectures |
| Threads | Explicit policy for control/debug service threads on Linux and Windows; no affinity pinning in v1; informational measurement |
| Debug | DWARF generation and frame rules on x86-64 and AArch64; typed locals, breakpoints, step, and call stacks on both |
| Packaging | One package per native host triple; host-correct `debug_contract`; package job on both Linux architectures |
| CI | `ubuntu-24.04-arm` joins the required native matrices; runners stay pinned |
| Documentation | Host-support matrix, limitations, architecture/host sections, release notes, and M8 evidence |

### 2.2 Out of scope

- macOS, Windows ARM64, 32-bit, big-endian, or real-time operating system hosts.
- Cross-compilation as a host-support claim: `stc` is built natively per host; cross-target
  remains IR/object emission only (D-195).
- A combined multi-architecture package, installer, package-manager integration, or registry.
- New language features, standard functions, or conformance beyond `stc-2013-oop-utf8-v1`.
- Changes to module/descriptor/state/metadata/debug-object/frame/manifest formats, host ABI,
  service IDs, build-ID domains, or the `stc-debug/1` wire schema.
- Remote/network debug, raw memory, multi-user sessions, or a privileged mode.
- Affinity pinning, real-time scheduling classes, or a hard real-time claim.
- Safety functions or certification; the non-safety stance is unchanged.

## 3. Exit criteria

- `cargo test --workspace --locked` passes natively on Linux x86-64 and Linux AArch64.
- `stc new/build/check/run/test/clean` work end-to-end on both hosts.
- The execution suite, conformance corpus, hot-swap suite, manifest tests, and debug suite pass
  on both hosts; retained state round-trips.
- No host-specific conditionals remain in `stc-lexer`, `stc-parser`, `stc-sema`, `stc-codegen`,
  or the module/manifest model.

## 4. Work breakdown

| Phase | Work | Acceptance |
| --- | --- | --- |
| 0 | Freeze the host contract and decisions D-200–D-208; record the M7 baseline | Plan reviewed; no format/ABI change; baseline commands green |
| 1 | Host abstraction audit and CI enforcement | `cargo xtask host-audit --check` passes; portable crates contain no OS-behaviour branches |
| 2 | Linux x86-64 native verification and gap fixes | Full workspace and every suite pass on `ubuntu-24.04`; evidence recorded |
| 3 | Linux AArch64 native build and suite fixes | Full workspace and every suite pass on `ubuntu-24.04-arm` |
| 4 | AArch64 debug parity: service-13 capture and DWARF frame extraction | Typed locals, breakpoints, step, and call stacks pass on AArch64 |
| 5 | Linux state-file atomicity and permissions | Retained state round-trips; user-only permissions; replace failure leaves the previous file |
| 6 | Module loading and service-thread policy | Unload/grace verified with `/proc/self/maps`; service threads run below normal priority; measurement recorded |
| 7 | Packaging, artifact naming, and CI matrix | One package per architecture builds, verifies, and runs the unpacked workflow |
| 8 | Documentation, evidence, and milestone sign-off | Host-support docs, limitations, architecture/spec updates, and the M8 evidence record |

## 5. Phase 0 — host contract and decisions

These decisions fix M8 behaviour; later phases implement and test them. Formats and ABI are
unchanged unless a new decision explicitly says otherwise.

| Decision | Choice | Reason |
| --- | --- | --- |
| D-200 | M8's supported native hosts are Windows 11 x86-64 MSVC (`x86_64-pc-windows-msvc`), Linux x86-64 GNU (`x86_64-unknown-linux-gnu`), and Linux AArch64 GNU (`aarch64-unknown-linux-gnu`). macOS, Windows ARM64, 32-bit, and big-endian hosts are not supported. | The project plan names the Linux hosts; naming the exact triples keeps the support matrix testable and prevents silent claims. |
| D-201 | Host-specific operations are confined to `stc-runtime` platform modules (loader, transport, signal, state replace, service priority) and the `stc-driver` toolchain/build/link/run layer. `stc-lexer`, `stc-parser`, `stc-sema`, `stc-codegen`, `stc-got`, and the manifest model carry no OS-behaviour branches; path case/extension semantics live in one manifest host module and host target resolution in one codegen host module. `cargo xtask host-audit --check` enforces the allowlist. | D-042 requires a platform layer; an executable audit is the only way to keep the boundary from drifting. |
| D-202 | AArch64 debug parity uses the existing format-1 frame rule: CFA `fp+16`, saved frame pointer at `CFA-16`, return address at `CFA-8`, with x29 as the frame pointer. The service-13 trampoline captures x29, sp, and x30; DWARF extraction accepts `DW_OP_reg29 (x29)`. AArch64 already has the same physical frame layout, so no format or runtime contract changes. | The stopped-frame table, runtime walk, and toolchain already encode this layout; AArch64 needs capture and extraction, not a new contract. |
| D-203 | Linux atomic writes create the temporary file user-only (mode 0600) for state files, sync the file, rename over the target, then sync the parent directory. Windows keeps `MoveFileEx` with `WRITE_THROUGH`. I/O output files keep ordinary permissions but use the same durability sequence. A failed replace leaves the previous file intact. | State files are runtime-private user data and must not become world-readable; parent-directory sync is required for rename durability after power loss. |
| D-204 | Control-server and debug-server accept/handler threads run at below-normal OS priority on Windows (`THREAD_PRIORITY_BELOW_NORMAL`) and Linux (nice +10, best-effort). The cycle thread keeps the process default priority. v1 does not pin CPU affinity. | Service threads must not compete with the cycle thread; affinity pinning conflicts with container cpusets and is a deployment decision, not a runtime default. |
| D-205 | CI adds the `ubuntu-24.04-arm` hosted runner (public-repository ARM64 image) to the native test, online, UI, conformance, execution, projects, debug, cross-target, and package matrices. Runner labels stay pinned; host-independent jobs remain on `ubuntu-24.04`. | The exit criteria require native AArch64 evidence; pinned labels prevent silent image migration. |
| D-206 | Packaging produces one archive per native host triple. `package.json.debug_contract` is `object-codeview` on Windows MSVC and `dwarf` on Linux GNU. No combined archive and no timestamps. | The debug backend is host-visible metadata; a Windows-only label on a Linux package is false. |
| D-207 | The Unix debug endpoint and audit file remain user-only on both Linux architectures: 0600 files in a 0700 directory, symlinks refused, endpoint/socket/audit removed on stop. Container and headless hosts use the same path rules with no shared-directory fallback. | Security behaviour must not depend on architecture or environment; a fallback would silently weaken the endpoint. |
| D-208 | M8 makes no format, ABI, protocol, language, or version change. `stc` remains `1.0.0-rc.1`; M8 is the host-portability completion of the release candidate. | The milestone is a portability pass; identity churn would invalidate M7 evidence without a contract reason. |

## 6. Phase 1 — host abstraction audit

### 6.1 Inventory

Audit every occurrence of `cfg(windows)`, `cfg(unix)`, `cfg(target_os = ...)`,
`cfg(target_arch = ...)`, `std::env::consts`, drive-letter paths, `.exe`/`.dll`/`.lib`
spelling, and OS-specific FFI. Classify each as:

- **platform layer** — loader, signal, transport, state replace, priority, clock;
- **toolchain layer** — clang/linker/archive/objcopy/readobj discovery, artifact names;
- **target resolution** — host triple selection for codegen and catalogs;
- **filesystem semantics** — case-insensitive path/extension comparison on Windows;
- **none** — a branch that should not exist.

### 6.2 Consolidation

- Move manifest path semantics (`is_target`, `is_excluded`, `has_st_extension`,
  `components_equal`, glob case folding) into `crates/stc-manifest/src/host.rs` behind one
  documented `PathSemantics` surface.
- Move the codegen host-triple fallback into `crates/stc-codegen/src/host.rs`. It remains the
  single target-resolution function; no codegen rule branches on the host OS.
- Do not introduce a new crate or dependency for the platform layer.
- Keep the runtime platform modules (`loader::platform`, `transport::platform`,
  `signal::platform`, `state::replace_file`, `debug_server` bind) as the only OS FFI there.

### 6.3 Enforcement

- Add `cargo xtask host-audit [--check]` that scans workspace Rust sources and fails on a
  forbidden conditional in the portable crates, listing each finding as `path:line`.
- Keep an explicit allowlist under `tools/ci/host-allowlist.txt` for the two target-resolution
  files, runtime/driver platform modules, test-only fixtures, and the target-architecture
  assembly in `trap_boundary.rs`/`stop.rs`.
- Run the audit in the `docs` or `traceability` CI job and in the M8 evidence list.
- Add a unit test for the audit itself (allowed and rejected samples).

## 7. Phase 2 — Linux x86-64 native verification

- Record the M7/M8 baseline on `ubuntu-24.04`: `fmt`, clippy, `cargo test --workspace --locked`,
  the debug/online/projects/conformance/execution/ui/cross-target suites, traceability, docs,
  and package.
- Fix every failure rather than skipping: the same suites run on Windows and Linux with the same
  assertions. A test that is genuinely host-conditional is expressed as a Unix/Windows variant,
  not a silent skip.
- Add Unix-only coverage that was previously absent:
  - state-file mode is 0600 and the temporary file never survives a failed replace;
  - debug endpoint and audit files are 0600 in a 0700 directory on the socket transport;
  - `dlopen`/`dlsym`/`dlclose` load and retire compiler-produced modules;
  - launcher exit codes on SIGINT and child timeout kill;
  - artifact names: `stc`, `libstc_runtime.a`, `libstc_runtime.so`, `<module>-<build>.so`,
    `<name>.a`, `lib<name>.so`.
- Keep the execution, conformance, and online suites independent of the host where they are
  host-independent, and assert the actual host triple where they are not.
- Update `Specification/Architecture/Testing-and-Verification.md` with the host-portability
  evidence row and the M8 commands.

## 8. Phase 3 — Linux AArch64 native build

- Add `aarch64-unknown-linux-gnu` to the local check targets and run
  `cargo check --workspace --all-targets --target aarch64-unknown-linux-gnu --locked` in review.
- On `ubuntu-24.04-arm`, run the full workspace tests and every suite. Fix compile or runtime
  assumptions:
  - the AArch64 system-link arguments (`-ldl -lpthread -lm` and the compiler runtime);
  - `stc-test` process isolation, timeouts, and signal handling;
  - launcher argument/environment handling and exit codes;
  - memory alignment, pointer width, and little-endian assumptions in descriptors/state;
  - module/manifest target-triple checks for `aarch64-unknown-linux-gnu`.
- Assert the AArch64 host triple in `stc metadata`, manifests, package name, and `stc --version`
  diagnostics.
- Keep the AArch64 cross-target matrix entry (IR/object emission) separate from native host
  support; native execution is proved by the suite jobs, not by cross-emission.

## 9. Phase 4 — AArch64 debug parity

### 9.1 Service-13 capture

- Add an AArch64 `debug_check_hook` naked trampoline that passes checkpoint ID, span ID,
  `x29` (frame pointer), `sp`, and `x30` (return address) to `debug_check_entry`. Args are
  x0/x1; place fp/sp/ra in x2/x3/x4 and branch to the entry point.
- Keep the existing x86-64 Windows/Linux trampolines and the no-frame fallback unchanged.
- Document the trampoline ABI in Debug and Monitoring §16 (frame rules), including the
  `x29`/`x30` layout and the no-frame case.

### 9.2 DWARF extraction

- Accept `aarch64-` targets in `stc-driver/src/frames.rs` (8-byte pointers, little-endian,
  `readelf` backend).
- Recognize `DW_AT_frame_base` with `DW_OP_reg29 (x29)` in addition to x86-64 `DW_OP_reg6
  (rbp)`; reject functions without a frame pointer (they are reported as optimized-out callers).
- Keep `DW_OP_fbreg` local offsets, the `fp+16` CFA, `saved_fp = -16`, and
  `return_address = -8`; add unit tests using AArch64 `readelf` text.
- Compile AArch64 modules and fixtures with `-mbranch-protection=none`: pointer authentication
  would sign the saved link register, and the format-1 walk needs the raw caller address.
- Machine-check the AArch64 prologue layout in a codegen/CI test: compile a debug fixture with
  `clang --target=aarch64-unknown-linux-gnu -gdwarf-5 -O0` and assert frame extraction succeeds
  on an x86-64 or AArch64 host.

### 9.3 Tests

- Run the full debug suite on `ubuntu-24.04-arm`: typed read/write/force/watch, breakpoints,
  pause, step over/into/out, recursion, truncation, missing/malformed/mispaired sidecars, CLI
  launch/attach/swap, release hook absence, metadata v2, and checkpoint/DWARF generation.
- Add an AArch64-specific test asserting the real module stop reports frames and typed locals;
  no AArch64 "unavailable" limitation remains.
- Verify breakpoint-only monitoring still works when a sidecar is absent (unchanged).
- Check that a function compiled without a frame pointer degrades to `optimized_out` rather
  than producing a bad walk.

## 10. Phase 5 — Linux state-file atomicity and permissions

- Split the atomic-write helper into a private form used by state saves and a general form used
  by I/O output:
  - create the temporary file with mode 0600 on Unix;
  - write all bytes and `sync_all`;
  - rename over the target (`std::fs::rename` on Linux; `MoveFileExW` with
    `REPLACE_EXISTING | WRITE_THROUGH` on Windows);
  - open the parent directory and `sync_all` on Linux so the rename survives power loss;
  - remove the temporary file on any failure; leave the previous target intact.
- Apply the private form to `state::save` and the retained-state file; apply the durability
  sequence to `io::flush_output`.
- Tests (Unix-gated, run on both Linux architectures):
  - saved file mode is exactly 0600;
  - a replace failure (for example a read-only directory or a directory in place of the
    target) leaves the previous bytes and removes the temporary;
  - a normal save/load round-trip preserves the payload and CRC;
  - the state file is portable between Linux x86-64 and AArch64 when the layout hash matches
    (a committed fixture loads on both);
  - concurrent saves leave one complete file, never a partial one.
- Update [State and RETAIN](Specification/Architecture/State-and-Retain.md) §4.2 with the
  permission and directory-sync rules; keep `STC-E-RUN-RETAIN-003` semantics.

## 11. Phase 6 — module loading and service-thread policy

### 11.1 Loader

- Verify `dlopen(RTLD_NOW)`/`dlsym`/`dlclose` for compiler-produced `<module>.so` on both Linux
  architectures, including the `RTLD_LOCAL` default: symbols from a retired module must not
  satisfy a new module's unresolved imports.
- Add a Linux test that reads `/proc/self/maps` while a generation is loaded and after retire +
  unload grace: the mapped module path is present/absent as documented. Skip the check when
  `/proc` is unavailable rather than failing.
- Keep generation pinning, retire grace, and the failed-load non-fatal behaviour unchanged.

### 11.2 Service priority

- Add `platform::lower_service_priority()`:
  - Windows: `SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL)`;
  - Linux: `setpriority(PRIO_PROCESS, 0, 10)` for the calling thread, best-effort;
  - other hosts: no-op.
- Call it at the start of the control-server thread and every debug accept/handler thread.
  Never change the cycle thread's priority.
- Do not call `sched_setaffinity`; document that affinity is a deployment concern.
- Add a Unix test that a spawned service thread observes nice +10 (read with `getpriority`),
  and keep the Windows behaviour assertion behind `cfg(windows)`.
- Record the maintenance-jitter measurement as informational in
  [Performance](Specification/Verification/Performance.md) (host/regime labelled), not a gate.

## 12. Phase 7 — packaging, artifact naming, and CI

### 12.1 Packaging

- `cargo xtask package` remains one archive per native host triple:
  `stc-<version>-x86_64-unknown-linux-gnu` and `stc-<version>-aarch64-unknown-linux-gnu`.
- `package.json.debug_contract` is host-derived: `object-codeview` for Windows MSVC and
  `dwarf` for Linux GNU.
- Verify the Linux archive on both architectures: `SHA256SUMS`, stripped
  `libstc_runtime.a`/`.so`, the `bin`/`lib` adjacency lookup, and the unpacked
  `new/build/check/run/test/clean` workflow.
- Add `clean` to the packaged workflow after `test`, then rebuild once to prove the project is
  usable again.
- Do not add a combined archive, installer, or package-manager integration.

### 12.2 CI

- Add `ubuntu-24.04-arm` to the `test`, `online`, `ui`, `conformance`, `execution`, `projects`,
  `debug`, `cross-target`, and `package` matrices. Keep `windows-2025` and `ubuntu-24.04`.
- Extend the `debug-overhead` workflow to `ubuntu-24.04-arm` (informational).
- Keep the `ci` aggregator list in sync; a failed or cancelled ARM job must fail the run.
- The debug job keeps its platform tool assertion: `llvm-readobj` on Windows, `readelf` on
  Linux; both are present on the ARM64 image.
- Do not silently change runner labels to `-latest`; a label migration is a decision.

## 13. Phase 8 — documentation and milestone evidence

- `M8-Plan.md` status becomes complete with the evidence run.
- [Project Plan](Specification/00-Project-Plan.md): record the M8 status block and update the
  verification-strategy host row.
- [Testing and Verification](Specification/Architecture/Testing-and-Verification.md): host
  matrix, commands, ARM runner policy, and what is native versus cross-emitted.
- [Debug and Monitoring](Specification/Architecture/Debug-and-Monitoring.md): AArch64 frame
  rule, trampoline, and DWARF register; remove the "x86-64 only" limitation.
- [State and RETAIN](Specification/Architecture/State-and-Retain.md): Linux permission and
  durability rules.
- [Runtime and Execution](Specification/Architecture/Runtime-and-Execution.md): service-thread
  priority policy and the no-affinity decision.
- [CLI and Manifest](Specification/Architecture/CLI-and-Manifest.md): per-architecture package
  naming and `debug_contract`.
- `README.md`, `USER-GUIDE.md`, `KNOWN-LIMITATIONS.md`, `RELEASE-NOTES.md`: support matrix
  (three native hosts), AArch64 debug parity, state permissions, service priority, and the
  remaining honest limitations (no macOS/Windows ARM64, no affinity, no remote debug).
- `Specification/Decisions.md`: D-200–D-208 and the M8 status paragraph.
- Record the evidence run (commit, workflow URL, job results) in the M8 status block and the
  release docs. If ARM64 hosted runners are unavailable for the public image, stop and record
  the blocker rather than claiming green AArch64 support.

## 14. Verification matrix

| Level | Windows x86-64 | Linux x86-64 | Linux AArch64 |
| --- | --- | --- | --- |
| `cargo test --workspace --locked` | `windows-2025` | `ubuntu-24.04` | `ubuntu-24.04-arm` |
| `stc new/build/check/run/test/clean` | package job | package job | package job |
| Execution suite | `execution` | `execution` | `execution` |
| Conformance corpus | `conformance` | `conformance` | `conformance` |
| Hot-swap/manifest suites | `online`/`projects` | `online`/`projects` | `online`/`projects` |
| Debug suite | `debug` | `debug` | `debug` |
| Frame sidecar + typed locals | `debug` | `debug` | `debug` |
| Retained-state round-trip + permissions | `test`/`projects` | `test`/`projects` | `test`/`projects` |
| Loader unload/grace | `online` | `online` | `online` |
| Cross-target object emission | `cross-target` | `cross-target` | `cross-target` |
| Release package | `package` | `package` | `package` |
| Host audit | `docs`/`traceability` | same | same |

## 15. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| AArch64 frame capture or DWARF mismatch | Keep the existing `fp+16` rule; test the real prologue and `readelf` text; fall back to the documented unavailable path only if the code cannot be made truthful |
| ARM64 runner availability or image drift | Pin `ubuntu-24.04-arm`; stop and record a blocker if the label is unavailable; no silent skip |
| Linux link failures from Rust staticlib dependencies | Keep the GNU link set explicit (`-ldl -lpthread -lm`); diagnose the actual missing symbol rather than adding blanket libraries |
| State durability change alters failure semantics | Preserve the "previous file intact" contract and `STC-E-RUN-RETAIN-003`; test failure paths |
| Service-priority calls fail under containers | Best-effort with no hard failure; document that the policy is advisory |
| Host audit becomes a blanket suppression | Small allowlist with reasons; audit tests; review every new entry |
| Debug tests accidentally skip on AArch64 | The debug job runs the same suite; no architecture-wide `ignore`; only a missing tool may skip |
| Windows regressions from shared changes | Run the full Windows matrix before sign-off; keep Windows-shaped behaviour behind the platform layer |

## 16. Definition of done

- [ ] Native Linux x86-64 and Linux AArch64 builds of `stc` and the runtime.
- [ ] `cargo test --workspace --locked` passes on both.
- [ ] `stc new/build/check/run/test/clean` works end-to-end on both.
- [ ] Execution, conformance, hot-swap, manifest, package, and debug suites pass on both.
- [ ] Retained state round-trips; atomic save, directory sync, and 0600 permissions verified.
- [ ] AArch64 `stack.list`/`frame.read` work for frame-pointer functions; no AArch64
  unavailable limitation remains.
- [ ] No host-specific conditionals in the portable compiler crates or the module/manifest
  model; the audit enforces it.
- [ ] Service-thread priority policy implemented and measured; affinity documented as absent.
- [ ] Host-support documentation, known limitations, decisions, and CI jobs updated.
- [ ] M8 evidence recorded with the green both-architecture run.

Where this plan and a specification disagree, the specification and a decision record win.

## 17. Execution protocol

Phases are executed sequentially. Each phase is complete only when it has:

1. passed the local `fmt`, clippy, workspace-test, host-audit, and docs checks;
2. committed its work on the `M8` branch;
3. been pushed to `origin/M8` and verified by a dispatched `CI` workflow run
   (`gh workflow run ci.yml --ref M8`), with every job green; and
4. recorded the commit and run ID in the phase-status table below.

If a CI run fails, the phase agent diagnoses and fixes it, then runs CI once more. If the second
run fails, or a decision needs human input, the sequence pauses and the failure is reported with
the run log. CI runs are never silently skipped or narrowed to make a phase pass.

## 18. Phase status

| Phase | State | Commit | CI run |
| --- | --- | --- | --- |
| 0 — host contract and decisions | Complete (2026-10-28) | `857fb20` | Local checks |
| 1 — host abstraction audit | Complete (2026-10-28) | `252af75` | `40784925474` |
| 2 — Linux x86-64 verification | Pending | | |
| 3 — Linux AArch64 native build | Pending | | |
| 4 — AArch64 debug parity | Pending | | |
| 5 — Linux state atomicity | Pending | | |
| 6 — loader and service priority | Pending | | |
| 7 — packaging and CI matrix | Pending | | |
| 8 — documentation and sign-off | Pending | | |
