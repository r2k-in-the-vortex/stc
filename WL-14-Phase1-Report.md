# WL-14 Phase 1 report — macOS CI and portability

Status: **Phase 1 complete 2026-11-08.** CI run
[42460413226](https://github.com/r2k-in-the-vortex/stc/actions/runs/42460413226) is green: both
macOS jobs, the coverage gate, and the aggregate `CI` job. Phase 2 has not started.

Branch: `WL-14`. Final Phase 1 commit: `e5a6a99`.

This report records the fixes and the decision process. It covers Phase 1 only: macOS
portability, the test-only CI matrix, and the deterministic Mach-O build work needed for the
selected suite. Mach-O frame extraction, the scheduling mode, measurements, and the final
documentation are later phases.

## 1. What Phase 1 delivers

- `macos-15` (ARM64) and `macos-15-intel` (x86-64) CI jobs run:
  - `cargo test --workspace --lib --bins`
  - `cargo xtask test --suite macos`
- The macOS suite covers the functional, project, execution, and selected I/O paths:
  `cli`, `cli_project`, `conformance`, `cross_target`, `examples`, `execution`,
  `exact_vectors_cli`, `fmt_cli`, `hir_dump`, `manifest_hardening`, `modbus_e2e`,
  `module_cli`, `online_cli`, `project_cli`, `projects`, `run_cli`, `state_io_cli`, `test_cli`,
  `ui`, `descriptor_lifecycle`, `embedding_run`, `host_lifecycle`, `image_remap`,
  `modbus_online_change`, `scheduler`.
- The aggregate `CI` job requires the `macos` matrix.
- macOS remains test-only: no package, no supported-host claim, no release-matrix change.
- Debug/frame targets are deliberately deferred to Phase 2, where Mach-O DWARF extraction is
  implemented; they are not silently filtered.

## 2. CI run history

Every failure was diagnosed from the job log, fixed in a focused commit, and retried. The
runs below are in order; the attempt count is higher than the number of distinct causes because
the macOS suite stops at its first failing target.

| Run | Result | Finding / fix |
| --- | --- | --- |
| [42427853084](https://github.com/r2k-in-the-vortex/stc/actions/runs/42427853084) | Fail | macOS link: undefined `_stc_trap_boundary_run`. |
| [42428971386](https://github.com/r2k-in-the-vortex/stc/actions/runs/42428971386) | Fail | Three runtime unit tests plus a 84.96% coverage drop. |
| [42430696250](https://github.com/r2k-in-the-vortex/stc/actions/runs/42430696250) | Fail | `cli_project`: release strip tools missing; ELF debug marker on Mach-O. |
| [42431806750](https://github.com/r2k-in-the-vortex/stc/actions/runs/42431806750) | Fail | `module_cli` byte determinism; coverage exactly at the boundary. |
| [42434205459](https://github.com/r2k-in-the-vortex/stc/actions/runs/42434205459) | Fail | `conformance`: `-no_uuid` removes the `LC_UUID` dyld requires. |
| [42435243006](https://github.com/r2k-in-the-vortex/stc/actions/runs/42435243006) | Fail | Apple `ld` rejects `-uuid`. |
| [42451637454](https://github.com/r2k-in-the-vortex/stc/actions/runs/42451637454) | Fail | Install-name pin alone; module still differed. |
| [42452484726](https://github.com/r2k-in-the-vortex/stc/actions/runs/42452484726) | Fail | `-oso_prefix` + first-difference diagnostic; remaining path/time fields. |
| [42453310180](https://github.com/r2k-in-the-vortex/stc/actions/runs/42453310180) | Fail | UUID rewrites; N_SO/debug-map paths remained. |
| [42454518206](https://github.com/r2k-in-the-vortex/stc/actions/runs/42454518206) | Fail | Relative compile/link paths; path still present. |
| [42455442356](https://github.com/r2k-in-the-vortex/stc/actions/runs/42455442356) | Fail | Debug/file prefix maps; timestamps remained. |
| [42456325769](https://github.com/r2k-in-the-vortex/stc/actions/runs/42456325769) | Fail | Canonical `-oso_prefix` + ASCII diagnostic; timestamp identified. |
| [42457267556](https://github.com/r2k-in-the-vortex/stc/actions/runs/42457267556) | Fail | Determinism fixed; `projects` socket path; Linux `ETXTBSY` flake. |
| [42458322875](https://github.com/r2k-in-the-vortex/stc/actions/runs/42458322875) | Fail | Endpoint file shortened, but the runtime socket directory was still in the project cwd. |
| [42459592026](https://github.com/r2k-in-the-vortex/stc/actions/runs/42459592026) | Fail | Full path confirmed the runtime `.sockd/s` directory under the project. |
| [42460413226](https://github.com/r2k-in-the-vortex/stc/actions/runs/42460413226) | **Pass** | Runtime socket directory moved to the user temp directory. |

## 3. Fixes

### 3.1 macOS CI and host integration

- **CI jobs and suite.** Added the `macos-15` (ARM64) and `macos-15-intel` (x86-64) matrix to
  `ci.yml`, a reusable `cargo xtask test --suite macos` target list, and the aggregate gate
  dependency. Runner images are pinned; `macos-14` is retired and was not used.
- **Host triples.** Added `x86_64-apple-darwin` / `aarch64-apple-darwin` to
  `stc-codegen::host_triple`, `stc_runtime::host_platform`, `HostPlatform::current`, and the
  `cli_project` test helper. Before this, macOS builds fell back to a Linux triple or rejected
  the host.
- **Trap boundary.** Extended the `trap_boundary.rs` target list to both macOS architectures
  and added a macOS-only assembly alias for `_stc_trap_boundary_run`. Rust's `extern "C"`
  reference uses the Mach-O C symbol with a leading underscore; the existing assembly defined
  the unprefixed name, so the link failed with an undefined symbol.
- **Dynamic loader.** Enabled the Unix `dlopen` platform module on macOS and used Darwin's
  `RTLD_LOCAL = 4` (Linux uses 0). Without this, macOS used the "dynamic module loading is not
  implemented" fallback and reported `STC-E-MODULE-002` instead of loading modules.
- **Accepted control socket.** Restored blocking mode on accepted Unix connections. The
  listener is nonblocking; on macOS the accepted stream inherited that flag, so a large control
  response failed part-way through `write_all`. Setting the stream blocking before the 5 s
  write timeout fixed the truncated 8 KiB response.
- **Modbus test timeout.** Stopped replacing the read timeout after the peer closed; macOS
  returns `EINVAL` for that `setsockopt`. `test_client` already sets a five-second bound.
- **Release stripping.** Added the pinned Homebrew LLVM 18 `bin` directory to the macOS job's
  `PATH` and asserted `llvm-objcopy` and `llvm-ar`. The release-build tests require both, and
  neither is on the default macOS runner path.
- **Mach-O debug marker.** Used `__debug_info` on macOS instead of the ELF `.debug_info`
  spelling in the debug-object test.

### 3.2 Deterministic Mach-O builds

The project promises byte-identical artifacts for identical inputs. Linux and Windows already
passed `module_cli`; macOS did not. Each diagnostic narrowed the remaining field:

1. **The install name defaulted to the output path.** Mach-O dylibs record the install name;
   `clang -shared -o <path>` embeds that path, so two build directories differed. Pin it with
   `-Wl,-install_name,@rpath/<file-name>`.
2. **`-no_uuid` was not valid for a loadable module.** It removed the difference but dyld
   rejected the module with `missing LC_UUID load command`. Apple `ld` requires `LC_UUID` to
   dlopen a dylib.
3. **`-uuid` is not an Apple `ld` option.** The next attempt failed at link time with
   `ld: unknown options: -uuid`. The option exists in some other linkers, not Apple's.
4. **The default UUID is content-derived.** Reading the `ld64` source showed the default
   `kUUIDContent` mode hashes the file content plus the output **leaf name**, not the directory.
   The real determinism culprit was the install name, not the UUID.
5. **The debug map still embedded paths.** The diagnostic showed the first differing bytes in
   the symbol table. Running clang and the linker from the output directory with relative names
   removed the directory from ordinary paths; `-fdebug-prefix-map` and `-ffile-prefix-map`
   additionally normalized both the `/var` and canonical `/private/var` spellings that Apple
   clang records in DWARF/N_SO. `ld64 -oso_prefix` used the canonical spelling to strip the
   resolved object path from `N_OSO`.
6. **The debug map recorded the object mtime.** The ASCII diagnostic identified the remaining
   `N_OSO` `n_value` as a one-second timestamp difference. The generated object now gets a fixed
   modification time before linking on Apple targets.
7. **The `LC_UUID` payload was still link-specific.** Apple `ld` seeds the UUID command before
   its content hash, so the payload can differ even when the rest matches. A post-link pass
   rewrites `LC_UUID` from the build ID in the module file name (with a fixed fallback for
   launcher executables). dyld only requires the command to be present; the value is stable.

The final set is intentionally layered: install name (path field), relative paths and prefix
maps (DWARF/N_SO), `-oso_prefix` (N_OSO path), object mtime (N_OSO timestamp), and `LC_UUID`
rewrite (linker-seeded payload). Some measures overlap, but each addresses a distinct Mach-O
field that the diagnostics observed.

The test itself was strengthened to report:
- every differing byte offset (not just the first),
- both artifact lengths,
- the build-directory offset and an ASCII window around it,
- and the canonical path spelling.

### 3.3 macOS debug endpoint and test robustness

- **Driver descriptor path.** On macOS the default debug endpoint file is a short
  `stc-dbg-<16-hex>.sock` name in the user temp directory. Project output directories routinely
  exceed the 104-byte Unix socket path limit.
- **Runtime socket directory.** The runtime creates `.stc-debug-<suffix>.sockd/s` for the actual
  socket. It used the caller's current directory (the project), which was still too long. It
  now creates that private directory under the user temp directory; `create_private_dir` keeps
  it user-only.
- **Diagnostics.** The bind error prints the full path (the file-name-only message hid the
  issue), and `stc debug` logs the resolved endpoint before launch.
- **ETXTBSY retry.** SSH/SFTP spawns retry Linux's transient `Text file busy` (error 26) up to
  four times with a short delay. A just-written fake SSH script can still be open for writing
  when the test spawns it; a real SSH client is unaffected.

### 3.4 Coverage

The new `xtask` suite arm dropped line coverage to 84.96%. A test that exercises
`suite_targets("macos")` raised it to the boundary; a second test that calls every suite and
checks the usage text gave the needed margin. The coverage gate is unchanged at 85%.

## 4. Decision process

- **Owner decisions.** Extend `--rt` rather than add a flag; implement Mach-O extraction rather
  than exclude debug/frame tests; pin `macos-15` / `macos-15-intel`; keep the existing
  no-timing-threshold policy; use up to the granted retry budgets and pause before Phase 2.
- **Evidence over assumption.** Each CI failure was treated as a specific field or platform
  behavior. The Mach-O work was driven by the linker source and the byte diagnostics, not by
  guessing at UUID flags.
- **Rejected approaches.**
  - Excluding debug/frame targets: rejected by owner decision; Phase 2 implements Mach-O.
  - `-no_uuid`: dyld rejects the module.
  - `-uuid`: Apple `ld` rejects the option.
  - Stripping the module's debug information: would weaken the debug contract and the
    `debug_release` assertions.
  - Weakening the determinism test (ignoring the UUID or paths): would hide a real
    reproducibility defect; the build was fixed instead.
  - A numeric timing threshold: not part of Phase 1; the existing D-193 informational policy
    remains.
- **Pause/retry discipline.** The work stopped at the owner's two-failure rule and resumed only
  with explicit retry budgets. The final report records every attempt so the decision trail is
  auditable.

## 5. Final state and hand-off

- Phase 1 CI: [42460413226](https://github.com/r2k-in-the-vortex/stc/actions/runs/42460413226),
  success on both macOS jobs, coverage, and the aggregate `CI` job.
- Branch `WL-14`, head `e5a6a99`.
- Working tree is clean except for this report and the updated working plan.
- Not yet in the macOS suite: `debug_*`, `frames`, `debug_release`, `debug_info`,
  `checkpoints`, `metadata_v2`, and `package`. The first group is Phase 2 (Mach-O frame
  extraction); `package` stays deferred with macOS release packaging (WL-14 item 1.1).
- Phase 2 will implement Mach-O DWARF sidecar extraction, enable the debug/frame targets, and
  verify the stopped-session path on both macOS architectures.
- The owner asked to pause before Phase 2; this report is the hand-off point.
