# WL-5.2 Plan — EtherCAT I/O on Linux using IgH

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-31  
Branch: `WL-5.2` (sub-plan of `v2.0.0-planning`)  
Status: Draft. The dedicated IgH/EtherCAT host baseline was verified 2026-10-31; the owner
accepted the decisions on 2026-10-31; implementation is phase-by-phase, starting with phase 0.  
Parent: [V2.0.0-PLAN.md](V2.0.0-PLAN.md) §4 (WL-5.2)  
Reference host: [setup-RT-EtherCAT-host](https://github.com/r2k-in-the-vortex/setup-RT-EtherCAT-host)
(local `C:\repos\rt-host-install`), used as reference information only; this repository does not
depend on it at build time.  
Inputs: [FutureWishlist.md](FutureWishlist.md) WL-5.2,
[IO and Devices](specification/Architecture/IO-and-Devices.md),
[Runtime and Execution](specification/Architecture/Runtime-and-Execution.md),
[CLI and Manifest](specification/Architecture/CLI-and-Manifest.md),
[RT-Host.md](specification/Verification/RT-Host.md),
[RT-Runbook.md](specification/Verification/RT-Runbook.md),
[KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md)

## 1. Objective

Integrate the IgH EtherCAT master into the `stc` runtime as a Linux-only hardware I/O backend.
The backend maps the static `stc.toml` device tree and channel bindings to IgH slave configuration
and process data, exchanges cyclic process data at the runtime's cycle boundary, and handles
startup, faults, recovery, and shutdown. IgH owns the EtherCAT protocol, kernel driver, and slave
configuration; this project does not implement a master, a slave stack, or a raw protocol engine.

Evidence is hardware-in-the-loop on the dedicated PREEMPT_RT EtherCAT host (`ecat-host`) with the
attached EtherCAT hardware, plus a cycle-time sweep to find the fastest frame-safe cycle time, a
runbook, and specification updates. No hard real-time guarantee and no safety claim is made;
results are host- and hardware-specific.

The design keeps the I/O-to-PLC binding layer open to more than one hardware backend in a run.
WL-5.1's Modbus TCP backend is also planned for v2.0.0 and must run in parallel with EtherCAT.
That requirement changes the binding layer and the runtime's backend dispatch, not the EtherCAT
backend, which receives only its own devices and bindings.

The binding layer is a routing graph rather than one-to-one bindings: a source may fan out to
many sinks, but each sink has one source. Physical input→PLC input, PLC output→physical output,
physical input→physical output, and PLC→PLC routes are supported. This covers pass-through and
setups where one PLC task simulates a physical device for another task.

## 2. Locked scope and boundaries

In scope:

- A Linux-only runtime I/O backend using the IgH userspace library, dynamically loaded so the
  base `stc` package has no IgH dependency.
- A static device-tree and channel-descriptor mapping to IgH slave configuration, sync managers,
  PDOs, and process data. ESI-based configuration remains deferred (WL-7).
- Cyclic process data latched at cycle start and published at cycle end, with startup, fault,
  recovery, and shutdown ordering.
- The generic cyclic-PDO and distributed-clock path needed by servo drives. The delivered
  hardware is a digital I/O slave; servo hardware evidence is not available (finding 10).
- Mock/loopback tests where hardware is unavailable, hardware-in-the-loop tests on `ecat-host`,
  and I/O-cycle timing and jitter measurements.
- A runtime/diagnostic surface for EtherCAT status and faults, and the specification,
  known-limitations, and runbook updates.
- The binding-layer interface for more than one hardware backend over the shared process image,
  so WL-5.1's Modbus TCP backend can run in parallel. WL-5.2 ships only the EtherCAT backend.
- The I/O routing graph: physical input→PLC input, PLC output→physical output, physical
  input→physical output, and PLC→PLC routes, with fan-out and single-source sinks.

Out of scope:

- An EtherCAT master, slave stack, or raw protocol implementation in this project.
- ESI parsing, PDO discovery, and SDO-based device configuration beyond the static configuration
  (WL-7 is deferred).
- A motion controller (WL-6), interpolation, and coordinated motion.
- Hard real-time guarantees, certification, safety functions, and safety-rated output states.
- Non-Linux hardware backends and AArch64 hardware evidence.
- General ST dataflow, expression-level routing, and `%M` route endpoints. The v2.0.0 routes
  connect physical channels and `%I`/`%Q` variables only.
- The Modbus TCP backend itself (WL-5.1) or any second hardware backend. WL-5.2 must leave the
  binding layer open for it, but does not implement or run a real second backend; a mock backend
  proves the interface and WL-5.1 supplies the real Modbus instance.
- Building, installing, or modifying IgH on the host; the host is already provisioned and is a
  run target, not a build target for `stc`.
- Modifying the reference host repository.

## 3. Verified baseline (2026-10-31)

The reference repository documents the host setup. The values below were re-checked read-only over
SSH for this plan; they are inputs, not new host changes.

### 3.1 Host and real-time configuration

| Item | Value |
| --- | --- |
| Host | `ecat-host`, Dell Wyse 5070 thin client, Intel Celeron J4105 (4 cores, no SMT), 8 GiB RAM, 64 GB M.2 SATA SSD |
| OS | Ubuntu Server 24.04.5 LTS, x86-64, UEFI, Secure Boot disabled |
| Kernel | `6.12.115-ecat-rt`, `CONFIG_PREEMPT_RT=y`, built 2026-10-30 |
| Boot command line | `intel_idle.max_cstate=1 processor.max_cstate=1 threadirqs pcie_aspm=off nowatchdog isolcpus=3 rcu_nocbs=3 irqaffinity=0-2` |
| CPU state | Fixed 1.5 GHz, `performance` governor, `no_turbo=1`; CPU 3 isolated |
| RT limits | `kernel.sched_rt_runtime_us = -1`; `ulimit -r` is 0 (no unprivileged FIFO) |
| Network | Onboard Realtek RTL8111/8168 `00:be:43:3f:31:05` reserved for EtherCAT; USB ASIX AX88179 is the LAN (`192.168.1.90`) |
| Access | SSH key login verified; no credential is stored in this repository |

The fixed 1.5 GHz profile and the disabled BIOS power features are deliberate and recorded in the
reference repository. The host is tuned already; WL-5.2 does not add or change a boot profile.

### 3.2 IgH EtherCAT installation

| Item | Value |
| --- | --- |
| IgH version | 1.6.13, commit `61cc654f5b721ddd54df0f58bdd34106d91c5359` |
| Kernel modules | `ec_master`, `ec_r8169` (native Realtek driver); stock `r8169` blacklisted |
| Userspace | `libethercat.so.1.2.0`, `ecrt.h` version 1.6, `pkg-config` version 1.6.13 |
| Service | `ethercat.service` enabled and active |
| Master state | Master0, link UP at 100 Mbps/Full, 1 slave, 0 Tx errors, 0 lost frames, ~970 frames/s idle; phase Idle and slave PREOP until an application requests the master |
| Device access | `/dev/EtherCAT0` is `crw-rw---- root:ethercat`; `r2k` is in the `ethercat` group |
| DC API | `EC_HAVE_SELECT_REF_CLOCK`, `EC_HAVE_REF_CLOCK_TIME`, `EC_HAVE_SYNC_TO`, `EC_HAVE_REG_BY_POS` are available |

### 3.3 Attached slave and PDO map

The attached slave is a DIEWU EtherCAT-IO digital module, not a servo drive.

| Item | Value |
| --- | --- |
| Identity | vendor `0x00000b95`, product `0x00010200`, revision `0x00000001` |
| Order number | `DIEWU_EtherCAT_DIO_16` |
| I/O | 16 digital inputs, 16 digital outputs, NPN output type |
| Distributed clocks | Yes, 64-bit; reference clock is slave 0 |
| SM0 | RxPDO `0x1a00` / entry `0x3101:01`, 8 bits, `EC_WD_ENABLE` |
| SM1 | RxPDO `0x1a01` / entry `0x3101:02`, 8 bits, `EC_WD_ENABLE` |
| SM2 | TxPDO `0x1600` / entry `0x3001:01` and `0x1601` / `0x3001:02`, 8 bits each, `EC_WD_DISABLE` |

The vendor's RxPDO/TxPDO numbering is reversed relative to the usual convention; the map above is
the device's actual SII/`ethercat cstruct` mapping and must be used as-is.

A direct IgH C test reached OP (`AL state 0x08`) with domain working counter 3 and ran 10,000
cycles at 1 ms with the outputs walked and zeroed on exit. It ran as `r2k` without `SCHED_FIFO`,
so it is a functional validation, not an RT measurement. The reference repository's
`ethercat/test-diewu.c` is the working example of the IgH call sequence; the shim follows the
same sequence. The SII XML has no `<Dc>` element, so no `AssignActivate` word is available for
this slave; the proven test ran without distributed-clock synchronization and left the
application time at zero.

### 3.4 `stc` runtime state on the host

The released `stc` 1.0.0 Linux package runs on `ecat-host`; it was revalidated with the WL-4
harness. With the reference tuned profile, 1 ms cycles, release build:

| Metric | Debug build | Release build |
| --- | --- | --- |
| Execution p50 | 824.758 µs | 82.139 µs |
| Execution p99 | 828.519 µs | 84.444 µs |
| Wake lateness p99 | 19.585 µs | 19.365 µs |
| Skipped periods / 10 min | 328 | 5 |

These are context for the I/O campaign, not I/O measurements. The debug build's execution is close
to the 1 ms budget; release has ample margin. The `stc` runtime has no EtherCAT backend today:
`[io].backend` accepts only `null`, `memory`, and `file`.

## 4. Findings that shape the plan

1. **The EtherCAT host is not the WL-4 host.** WL-4 was measured on a TianBei N1 PRO (Intel N150,
   kernel `6.17.999-8-generic`). WL-5.2's host is `ecat-host` (J4105, kernel `6.12.115-ecat-rt`).
   WL-4's numbers and boot profile do not transfer. The parent plan's dependency and sequencing
   text is corrected to name the dedicated EtherCAT host; WL-4 supplies the harness and runbook
   pattern, not the hardware.
2. **The IgH and hardware path is proven, but not under the `stc` runtime.** The master, native
   driver, slave OP transition, PDO exchange, and shutdown zeroing (a bench convenience) were
   validated with a separate
   C program. The runtime integration itself is unstarted.
3. **The runtime has no hardware backend contract and discards the device tree.** `IoBackend` has
   three variants (null, memory, file). `IoRuntime` copies defaults or host-supplied image bytes
   and discards outputs. `ModuleInfo` parses the image layout, backend, file, defaults, direct
   bindings, retained-state policy, and debug object; it does not retain `io.devices` or the
   effective binding `device`/`channel`/`direction` fields. The EtherCAT backend must add that
   parsing, validation, and binding resolution. The current model is also single-backend:
   `[io].backend` selects one backend and `IoRuntime` owns one. WL-5.1's Modbus backend must run
   in parallel, so the binding layer needs per-device dispatch and a coordinator (finding 13).
4. **The cycle boundary is the right hook, but the host skips output publication on a failed or
   rolled-back cycle.** `Host::cycle_at` latches inputs before the scan and publishes outputs
   after a successful scan. A trap or rollback returns before `publish_outputs`. For IgH, a
   missing queue/send can trip the slave's process-data watchdog. The integration needs an I/O
   cycle-end hook that runs on every cycle, including rollback, while preserving the existing
   module-output semantics for the other backends.
5. **Static configuration is hand-maintained.** The DIEWU PDO map is known from
   `ethercat cstruct`. There is no ESI. The opaque `[io.devices].config` table is the right place
   for the PDO map; the runtime must validate it and fail closed.
6. **Distributed clocks are available in IgH and in the slave, but not configured for the only
   attached slave.** The SII XML carries no `AssignActivate` value. The backend implements
   `ecrt_slave_config_dc` and application-time synchronization, but the owner chose free-run for
   the DIEWU on 2026-10-31. DC evidence is deferred unless a DC-capable slave or a vendor ESI
   becomes available.
7. **Licensing and link model are release inputs.** The IgH kernel modules are GPLv2. The
   userspace library headers are LGPL 2.1, while the installed `libethercat.pc` text says LGPLv3;
   the exact terms must be reconciled from the pinned 1.6.13 tag. The base `stc` package must not
   depend on IgH. A project-owned C shim loaded with `dlopen` avoids a hard link dependency and
   the ABI risk of mirroring `ecrt.h`'s C bitfields (`ec_slave_config_state_t`,
   `ec_master_state_t`) in Rust.
8. **Fault behavior is transparent and PLC-visible, not an automatic data rewrite.** The owner
   confirmed that the master does not modify the data it sends: keep sending the PLC output
   values, keep presenting the last received input values to the PLC as stale, expose master and
   slave status and diagnostic registers to the PLC, and recover when communication returns.
   Connect/disconnect and reconnect of a configured slave are normal; adding or removing a device
   is a topology change and requires a cold download. The runtime does not zero outputs, default
   inputs, or exit on a fieldbus fault; the PLC logic decides. Not a safety function.
9. **Online change must reject EtherCAT topology and binding changes.** The host preserves the
   image and remaps direct cells across a compatible swap. Changing the device tree, PDO map, or
   binding-to-PDO mapping requires a cold download. A code-only swap can preserve the backend and
   the image.
10. **Servo-drive evidence is not available in v2.0.0.** The owner confirmed that only the DIEWU
    16I/16O digital slave is attached. The generic cyclic-PDO and DC path is implemented and
    unit-tested as the foundation for drives, but v2.0.0 hardware evidence covers the digital I/O
    only. The report and known limitations state this explicitly.
11. **The platform boundary is Linux x86-64.** The compiler supports Linux AArch64, but the IgH
    backend is not hardware-verified there. The process-image/PDO mapping assumes little-endian
    hosts. The backend must fail with a clear diagnostic on an unsupported target, and the docs
    must state the limit.
12. **The 1 ms timing margin depends on the build form.** On `ecat-host`, the debug build uses
    almost the whole 1 ms budget while release leaves roughly 918 µs. The I/O timing campaign
    uses release as the primary evidence and records debug as the intended default with its
    tighter margin.
13. **WL-5.1 will run in parallel, so backend selection cannot remain a single global enum.**
    Modbus TCP is also planned for v2.0.0 and must work alongside EtherCAT. The change is in the
    I/O-to-PLC binding layer: bindings group by device, each device's `kind` selects a backend
    instance, and a coordinator latches and publishes each backend over the one shared process
    image. The EtherCAT backend itself is unchanged; it sees only its own devices and bindings.
    WL-5.2 ships the seam and a mock second backend, not the Modbus backend.
14. **The binding model is a routing graph, not one-to-one bindings.** The owner confirmed the
    routing rules: physical input→PLC input, PLC output→physical output, physical input→physical
    output, and PLC→PLC. A source may fan out to many sinks, but each sink has exactly one
    source. This is required for task-to-task device simulation and for cross-backend routing.
    The existing `[[io.bindings]]` becomes the single-sink shorthand; the plan adds a route table
    and a router in the coordinator.

## 5. Decisions (proposed)

`Proposed` means the owner has not confirmed the decision; `Accepted` means the owner confirmed it
on 2026-10-31; `Default` means it follows from the evidence and may be adjusted during
implementation without a scope change.

| ID | Decision | Status |
| --- | --- | --- |
| E-1 | Build a project-owned C shim (`libstc_io_ethercat.so`) against `ecrt.h`; load it with `dlopen` only when the `ethercat` backend is selected. Do not link `libethercat` into `stc`, the launcher, or the base package. The Linux launcher already links `-ldl`, so no new link flag is needed. The shim exposes a small versioned ABI and a version handshake; a missing or incompatible shim is a cold-start error. | Accepted 2026-10-31 (CI-built shim via E-8) |
| E-2 | Store one source-controlled profile per machine (for example `io/bench.toml`, `io/line1.toml`), each containing the backend configuration for every device on that machine and the I/O routes. `[io].config` in `stc.toml` selects the active profile; the compiler resolves and embeds it into metadata so the artifact is self-contained and reproducible. The profile carries host-specific master/slave addressing, the PDO map, and the route table; the logical device tree and channel descriptors stay in `stc.toml` and are referenced by name. `--io-config` remains a diagnostic runtime addressing override and does not change routes. Precedence: runtime override > selected source profile > inline device config. | Accepted 2026-10-31 |
| E-3 | Select hardware backends per device `kind`, not with a single global selector. `[io].backend` and `--io` remain the default for unbound image areas and for projects without hardware devices; they do not constrain the number of device backends. Add `ethercat` as a known kind/backend name, but keep the compiler's output opaque. No ST language or codegen change. | Accepted 2026-10-31 (per-device kind) |
| E-4 | Map physical PDO entries to process-image ranges bit-exactly. Each physical output entry and each PLC input sink has exactly one source; a source may fan out to many sinks. Fan-in (two sources writing one sink) is rejected. Validate slave identity, sync direction, channel direction and width, entry bit ranges, and that every route endpoint resolves. Unbound output PDO entries are zeroed at configuration time; bound output values are never rewritten by the master. Unbound input entries are ignored. | Accepted 2026-10-31 (routing rules) |
| E-5 | At the cycle boundary: receive and process the domain at input latch; copy inputs; run the scan; copy outputs; queue and send every cycle. Configure distributed clocks only when `AssignActivate` is present; the DIEWU runs free-run by owner decision (2026-10-31). `--rt` is required for timing evidence but not for functional I/O. | Accepted 2026-10-31 (DIEWU free-run) |
| E-6 | The master never rewrites the data it exchanges. On a fieldbus fault it keeps sending the PLC's output values unchanged, keeps presenting the last received input values to the PLC as stale, keeps exchanging and retrying to recover, and exposes master and slave status and diagnostic registers to the PLC each cycle. Recovery is automatic when communication returns; disconnect/reconnect of a configured slave is normal. Adding or removing a device is a topology change and requires a cold download (E-7). There is no automatic output zeroing, no input defaulting, no persistent-fault exit, and no safety function. Fault state is per backend; this policy does not impose Modbus behavior. | Accepted 2026-10-31 |
| E-7 | Include a hash of the selected profile, device tree, backend kinds, PDO/channel map, and effective bindings/routes in online-change compatibility. A code-only swap preserves the backends; a changed hash requires a cold download (`STC-E-ONLINE-IO-001`). | Accepted 2026-10-31 (hash + cold on change) |
| E-8 | Leave the base `stc` package unchanged. Build the shim in Linux CI against a pinned IgH 1.6.13 userspace and ship it in the optional `stc-io-ethercat` artifact with a sample configuration and build notes. The loader verifies the host `libethercat` ABI. IgH 1.6.13 is an operator-installed host prerequisite; kernel modules are not redistributed. The runtime locates the shim via `--io-ethercat-lib`, `STC_ETHERCAT_LIBRARY`, or the loader path. | Accepted 2026-10-31 (CI-built shim) |
| E-9 | Evidence: HIL on `ecat-host` with the single DIEWU 16I/16O fixture. Phases 3–5 use open-ended (unwired/unloaded) I/O and treat a cycle as successful when IgH accepts and replies to the frame: a cycle-time sweep from 1 ms down to 50 µs using a non-CPU-bound PLC program, extending below 50 µs if that step passes, with 60 s screening per step and a 10 min confirmation at the fastest passing step, recording the first failing CT and the frame-loss evidence; software-fault, recovery, and online-change matrices. All manual/physical tests, the automated all-points I/O matrix, and physical fault injection are phase 6. Release primary and debug secondary. Multi-slave configurations are covered by mock/config tests only. Raw JSONL retained as campaign artifacts with SHA256; summaries and reports committed. No CI timing gate. | Accepted 2026-10-31 (single-slave HIL; frame-level phases 3–5) |
| E-10 | Implement and unit-test the generic cyclic-PDO/DC path used by servo drives, and document a servo-style configuration. Hardware evidence is limited to the DIEWU; the report and known limitations state the servo limitation explicitly. | Accepted 2026-10-31 — DIEWU only |
| E-11 | Record the exact IgH 1.6.13 license terms; dynamically link the userspace library; distribute the shim source; do not redistribute kernel modules; include notices. The licensing review is a phase-0 gate. | Accepted 2026-10-31 (dynamic link, CI shim) |
| E-12 | Linux x86-64 is the only hardware-evidence platform in v2.0.0. AArch64 builds are allowed but not hardware-verified; the mapping assumes little-endian. | Accepted 2026-10-31 (x86-64 only) |
| E-13 | The runtime I/O interface is a Rust trait in `stc-runtime` and is multi-backend from the start. One coordinator owns the shared process image and a list of per-device backend instances; bindings group by device `kind`. At the input boundary it applies the global input defaults to unbound areas, then calls each backend's input latch in device declaration order. At the output boundary it calls each backend's output publish in the same order, then applies the global output behavior to unbound areas. Cold start and shutdown traverse the instances in device declaration order and reverse order. Bound image ranges are disjoint across devices/backends, and conflicts are rejected. WL-5.2 registers only the EtherCAT backend and proves the plural interface with a mock second backend; WL-5.1 adds Modbus as a Rust module or linked crate without changing the EtherCAT backend. | Accepted 2026-10-31 (Rust trait) |
| E-14 | Expose EtherCAT master and slave status and diagnostics to the PLC as a read-only status device in the device tree, not only through `--status`. The runtime implements the channel set: master phase/active/slave count, link state, domain WC state/working counter, per-slave AL/online/operational state, last error, recovery count, stale flag, and exchange/fault counters. The project binds only the fields it needs, so unbound fields cost no image bytes. The runtime fills the block each cycle from the EtherCAT backend; the layout is versioned and unknown channels are rejected. Lost-frame, Tx-error, and frame-loss counters are not exposed by ecrt 1.6.13 and are read from `ethercat master` at run boundaries, not per cycle. | Accepted 2026-10-31 (status set; frame counters via CLI) |
| E-15 | Support explicit I/O routes as a directed graph: physical input→PLC input, PLC output→physical output, physical input→physical output, and PLC→PLC. A route has one source and one or more sinks; fan-out is allowed, fan-in is rejected. The route table lives in the selected machine profile (`[io].config`) alongside the PDO map; the compiler resolves variables, devices, and channels and emits the routes in metadata; the runtime applies them at the cycle boundary. PLC→PLC routes apply at the input boundary with one scan of latency; route cycles are allowed and resolved by that buffer as expected behavior, with no warning. | Accepted 2026-10-31 (one-scan delay, profile location, cycles allowed) |
| E-16 | Unrouted PLC `%I`/`%Q` variables are warnings, not errors. An unrouted `%I` uses its declared/default value and the runtime reports that default; an unrouted `%Q` is discarded. Fan-in remains an error. The compiler emits the warning; the runtime reports the variable and its default. Warnings emit only when a machine profile is selected; profile-less v1 projects keep their existing behavior. Physical route source/sink representation checks remain runtime fail-closed. The profile is embedded as raw source text and hashed with SHA-256 over those exact bytes. | Accepted 2026-10-31 (warnings, not errors; profile-gated; raw profile text) |
| E-17 | `dc.assign-activate` is a u16 integer matching `ecrt_slave_config_dc`; the manifest validator accepts 0..=65535. Boolean shorthand is not accepted. The DIEWU profile omits DC. | Accepted 2026-10-31 (u16) |

## 6. Architecture

### 6.1 Components

```text
stc.toml ── compiler ──> <name>.stc-meta.json ──> runtime I/O coordinator
                                                        │
                                      ┌─────────────────┴─────────────────┐
                                      │                                   │
                               EtherCAT backend                  Modbus backend
                               (WL-5.2)                          (WL-5.1, later)
                                      │
                            dlopen libstc_io_ethercat.so
                                      │
                               libethercat.so (IgH)
                                      │
                            ec_master + ec_r8169 (kernel)
                                      │
                               EtherCAT slave(s)

                    one shared %I / %Q / %M process image
```

- **Compiler:** unchanged except for accepting the backend name and continuing to copy the
  opaque device `config`, channels, and effective bindings into metadata. It does not choose a
  backend instance.
- **I/O coordinator:** owns the shared process image, parses metadata, groups physical sinks by
  device, and owns the route graph. On inputs it applies the global default to unbound areas,
  calls each backend instance's latch, then applies the input-side routes; on outputs it applies
  the output-side routes, calls each backend instance's publish, then applies the global output
  behavior to unbound areas. The order is device declaration order. It rejects fan-in and
  conflicting sink claims and aggregates per-backend status.
- **Device backends:** each instance owns only its devices and bindings. The EtherCAT backend is
  one instance; WL-5.1 adds a Modbus instance. Backends do not know about each other.
- **C shim:** owns the IgH API calls, C struct layouts, and the domain buffer. It does not decide
  policy.
- **IgH:** owns the EtherCAT protocol, slave configuration, alarms, and the kernel driver.

The coordinator-facing interface is a Rust trait in `stc-runtime`, backend-neutral and
deliberately small. The exact names are a phase-1 decision; the shape is the contract WL-5.1
builds against:

```rust
// Sketch only; no protocol type crosses this interface.
trait DeviceBackend {
    fn cold_start(&mut self, config: &BackendConfig) -> Result<(), Error>;
    fn latch_inputs(&mut self, image: &mut ProcessImage) -> Result<(), Error>;
    fn publish_outputs(&mut self, image: &ProcessImage) -> Result<(), Error>;
    fn status(&self) -> BackendStatus;
    fn shutdown(&mut self) -> Result<(), Error>;
}
```

EtherCAT implements this interface as one backend instance; WL-5.1 adds a Modbus module or linked
crate in `stc-runtime` implementing it. The coordinator does not know IgH, Modbus, or their
protocol types. The interface is plural even when a project uses one hardware backend: a
single-backend project is simply a project whose devices share one kind, not a separate runtime
path or a locked-down mode.

### 6.2 Configuration model

The logical device tree and channel descriptors live in `stc.toml`; one source-controlled profile
per machine carries the backend configuration for every device on that machine and the I/O
routes. `[io].config` selects the active profile. The compiler treats the inline device `config`
and the selected profile as opaque TOML and embeds them in metadata; the runtime interprets them.
A DIEWU project, reduced to one channel per direction for readability:

```toml
# stc.toml
[io]
# A single-backend project may name the hardware backend; a mixed project
# omits backend or leaves it "null" and lets each device's kind select one.
backend = "ethercat"
# Source-controlled machine profile, resolved and embedded at build time.
config = "io/bench.toml"

[[io.devices]]
name = "diewu"
kind = "ethercat"

[[io.devices.channels]]
name = "OUT0"
direction = "output"
width = 1

# OUT1..OUT15 and IN0..IN15 follow.

[[io.bindings]]
variable = "Main.Out0"
device = "diewu"
channel = "OUT0"
direction = "output"

# ...
```

```toml
# io/bench.toml (selected by [io].config; embedded at build time)
[device.diewu]
master = 0
alias = 0
position = 0
vendor = "0x00000b95"
product = "0x00010200"
revision = "0x00000001"
startup-timeout = "5s"
recovery-cycles = 3

[[device.diewu.syncs]]
index = 0
direction = "output"
watchdog = "enable"

[[device.diewu.syncs.pdos]]
index = "0x1a00"

[[device.diewu.syncs.pdos.entries]]
index = "0x3101"
subindex = 1
bits = 8

# Sync 1 (0x1a01 / 0x3101:02) and sync 2 (0x1600, 0x1601 / 0x3001:01, 0x3001:02)
# follow the same pattern.

[[device.diewu.pdo-channels]]
name = "OUT0"
entry = { index = "0x3101", subindex = 1 }
bit = 0

# OUT1..OUT7 use bits 1..7 of 0x3101:01; OUT8..OUT15 use 0x3101:02 bits 0..7.
# IN0..IN15 mirror this on 0x3001:01 (IN0..IN7) and 0x3001:02 (IN8..IN15).

# IN0 fans out to the PLC input and to physical OUT1. OUT0 is driven by
# Main.Out0 through the single-sink binding shorthand in stc.toml.
[[routes]]
source = { kind = "physical-input", device = "diewu", channel = "IN0" }
sinks = [
  { kind = "plc-input", variable = "Main.In0" },
  { kind = "physical-output", device = "diewu", channel = "OUT1" },
]

[[routes]]
source = { kind = "plc-output", variable = "TaskA.Out" }
sinks = [{ kind = "plc-input", variable = "TaskB.In" }]
```

Rules:

- `[io].config` is relative to the project root and must stay inside it. The compiler checks that
  the file exists and parses as TOML, then embeds its canonical contents in metadata. It also
  resolves route endpoints against the program and the profile, validates fan-in, direction, and
  width, and emits the effective device tree, PDO map, and routes. The build is self-contained
  and reproducible.
- Inline `[io.devices].config` remains valid as a default. The selected profile wins field by
  field; `--io-config <path>` is a diagnostic runtime override and may change addressing for the
  selected devices, but does not add devices or change routes in v2.0.0. Precedence: runtime
  override > selected source profile > inline device config.
- Device names and channel names are unique and are the compiler's validation surface. The
  runtime cross-checks that every channel declared in `[[io.devices.channels]]` has a
  `pdo-channels` entry when the device is selected, and vice versa.
- `vendor`, `product`, `revision`, and the numeric fields are parsed as integers; string spellings
  are accepted for hex convenience but normalized before comparison.
- The sync manager tree mirrors `ecrt_slave_config_pdos`; the `pdo-channels` map references an
  entry already declared in that tree. A binding resolves through the channel name to the entry,
  then through the sync/PDO tree to the domain offset.
- `recovery-cycles` is the number of consecutive healthy exchanges before the stale/fault status
  clears. There is no output-modifying fault policy and no persistent-fault exit.
- The optional `dc` table carries `assign-activate`, `sync0-cycle`, `sync0-shift`,
  `sync1-cycle`, and `sync1-shift` in the same units as `ecrt_slave_config_dc`. Without
  `assign-activate` the device runs free-run; the DIEWU profile omits it by owner decision.
- Hardware dispatch is per device `kind`. `[io].backend` and `--io` are not a single-run
  hardware selector; they are the default for unbound image areas and for projects without
  hardware devices. A mixed project omits `[io].backend` or leaves it `null`.
- A device whose `kind` has no registered backend is a cold-start error. WL-5.2 registers
  `ethercat` and the read-only `ethercat-status` kind; the same EtherCAT backend instance handles
  both so the status device reads the same master/slave state. WL-5.1 will register `modbus` and
  attach it as another backend instance.
- A declared device of a registered kind is configured by default even when it has no bindings,
  so its health is visible; its unbound output entries are zeroed. A backend may define an
  opt-out for unused devices.
- When `[io].backend` names a hardware backend, the coordinator uses the `null` behavior for
  unbound image areas; the hardware device backends handle their bound ranges. The name records
  project intent and keeps single-backend projects readable.

A future mixed project declares an EtherCAT device and a Modbus device and omits
`[io].backend`; each device's `kind` selects its backend. WL-5.2 alone rejects the Modbus device
until WL-5.1 registers that kind. This is the configuration shape the binding layer must accept,
even though WL-5.2 delivers only the EtherCAT implementation.

### 6.3 Process-image mapping

The host process image is `%I`, then `%Q`, then `%M`. Each direct binding carries an area,
`offset_bits`, and `width_bits`; each bound channel carries a PDO entry and a start bit within
that entry. The copy is bit-exact:

- input: domain entry bit range → `%I` area at the binding's `offset_bits`;
- output: `%Q` area at the binding's `offset_bits` → domain entry bit range;
- `%M` is never sent or received;
- widths must match; a 1-bit `BOOL` may occupy any entry bit, and an 8/16/32-bit value must be
  byte-aligned in the domain entry and in the image; a binding must fit inside one PDO entry, so
  split or multi-entry mappings are out of scope for v2.0.0;
- unbound input locations receive the compiler's input defaults as usual; unbound output image
  locations have no PDO sink, and declared output PDO entries with no binding are zeroed in the
  domain at configuration time; bound output values are never rewritten by the master;
- the mapping is little-endian, matching the supported Linux hosts.

Before mapping, the coordinator groups physical sinks by device and resolves each device's `kind`
to a backend instance. Each physical output PDO entry and each PLC input sink has exactly one
source; a source may fan out to many sinks. Two sources claiming the same sink are rejected at
cold start with `STC-E-IO-BINDING-001`. Same-device aliases follow the existing image semantics.
Unbound areas keep the global default backend's behavior.

The DIEWU fixture is bit-level and uses all 16 inputs and all 16 outputs, matching the module's
16I/16O points. Each direction has two 8-bit PDO entries: `IN0..IN7` map to bits 0–7 of
`0x3001:01`, `IN8..IN15` to bits 0–7 of `0x3001:02`, and `OUT0..OUT15` mirror that on
`0x3101:01`/`0x3101:02`. The process image declares 16 `BOOL` inputs (`%IX0.0..%IX0.15`) and 16
`BOOL` outputs (`%QX0.0..%QX0.15`); each channel has width 1 and a `bit` position 0–7 in its PDO
entry. A two-`BYTE` variant is equivalent for a loopback test, but the reference fixture is
bit-level.

### 6.3.1 Routing model

The process image is connected by directed routes, not only by one-to-one bindings. A route has
one source and one or more sinks:

| Source | Allowed sinks | Use |
| --- | --- | --- |
| Physical input | PLC input (`%I`), physical output | Normal input and pass-through |
| PLC output (`%Q`) | Physical output, PLC input (`%I`) | Normal output and task-to-task simulation |
| Physical input | Physical output | Direct pass-through / loopback |

Rules:

- A source may fan out to many sinks; each sink has exactly one source. Fan-in is rejected.
- Source and sink widths must match. A physical channel uses its declared width; a PLC variable
  uses its type width.
- Physical inputs are sources only; physical outputs are sinks only. `%I` variables are sinks
  only; `%Q` variables are sources only. `%M` markers are not route endpoints in v2.0.0.
- Routes are declared in the selected machine profile under `[[routes]]`, alongside the PDO map.
  The compiler resolves variable paths, device names, and channels, validates the graph, and
  emits the routes in metadata. The existing `[[io.bindings]]` in `stc.toml` remains the
  single-sink shorthand.
- The runtime applies routes at the cycle boundary. Physical input→PLC input and physical
  input→physical output routes run after the input latch; PLC output→physical output and
  PLC→PLC routes run after the scan. PLC→PLC routes use a route buffer with a one-scan delay
  (E-15). Route cycles are allowed and resolved by that buffer; no warning is emitted because
  this is expected behavior.
- Cross-backend routes are allowed (for example, a Modbus input can drive an EtherCAT output)
  because all routes operate on the shared process image.

Example:

```toml
# io/bench.toml (machine profile; routes are part of the I/O configuration)
[[routes]]
source = { kind = "physical-input", device = "diewu", channel = "IN0" }
sinks = [
  { kind = "plc-input", variable = "Main.In0" },
  { kind = "physical-output", device = "diewu", channel = "OUT1" },
]

[[routes]]
source = { kind = "plc-output", variable = "TaskA.Out" }
sinks = [{ kind = "plc-input", variable = "TaskB.In" }]
```

### 6.4 Lifecycle and cycle boundary

1. **Cold start.** Read metadata; resolve each device's `kind` to a registered backend; group
   bindings by device; cold-start every backend instance. For EtherCAT that is: load the shim;
   validate the configuration; `ecrt_request_master`; create the domain; configure each slave
   (`ecrt_master_slave_config`, `ecrt_slave_config_pdos`, optional `ecrt_slave_config_dc`,
   `ecrt_slave_config_reg_pdo_entry`); `ecrt_master_activate`; obtain `ecrt_domain_data`; and map
   entry offsets to bindings. The run does not begin until every backend's startup check is ready
   (for EtherCAT, OP and a healthy working counter) or its startup timeout fails the run.
2. **Each cycle, input boundary.** The coordinator applies the global input defaults to unbound
   areas, then calls each backend's input latch in device declaration order. For EtherCAT:
   `ecrt_master_receive`, `ecrt_domain_process`, read `ecrt_domain_state` and each
   `ecrt_slave_config_state`; if healthy, copy its bound inputs and clear the stale flag; if not,
   keep the last received input image, set the stale/fault status registers, and record the
   transition. Input defaults are not applied on a fault. The coordinator then applies physical
   input→PLC input and physical input→physical output routes, and applies buffered PLC→PLC routes
   to their `%I` sinks.
3. **Each cycle, after the scan.** The coordinator applies PLC output→physical output routes and
   stages the PLC→PLC route sources, then calls each backend's output publish in the same order,
   then applies the global output behavior to unbound areas. For EtherCAT: copy its bound
   outputs; `ecrt_domain_queue`; if DC is configured, set `ecrt_master_application_time` and
   queue the reference/slave clock synchronization; then `ecrt_master_send`. This path runs on
   every cycle, including a rolled-back or faulted cycle, so the slave's process-data watchdog is
   not tripped by a missing frame. The output image is never rewritten on a fault; the PLC's
   values are sent unchanged.
4. **Shutdown.** The coordinator stops every backend in reverse order. For EtherCAT: stop
   exchanging, `ecrt_release_master`, and unload/close; the output entries are not rewritten. The
   slave's process-data watchdog handles the stopped frames. The same path runs on a setup
   failure after the master was requested. The direct C test's zero-on-exit behavior is a bench
   convenience, not the runtime policy.

If one backend's cold start or startup check fails, the coordinator shuts down the backends
already started in reverse order and fails the run; a partial start never leaves one backend
holding the fieldbus while another aborts. A startup failure is fatal with a clear error, and
the run never begins; no PLC outputs are published.

The host change in finding 4 is small: an I/O cycle-end hook from the run loop that is invoked
after every scan attempt for every backend. The null, memory, and file backends keep their
current behavior as the global default.

### 6.5 Fault and recovery states

| State | Entry | Behavior |
| --- | --- | --- |
| `Opening` | Cold start | Request master, configure slaves, activate; failures are fatal startup errors |
| `WaitingForOp` | After activation | Exchange frames until all configured slaves are operational and the domain is healthy, or `startup-timeout` expires |
| `Running` | Healthy exchange | Latch inputs, publish outputs, count exchanges |
| `Faulted` | WC/AL/link fault | Keep sending the PLC output values unchanged; keep the last received input image and set the stale/fault status registers; keep exchanging and retrying; record the transition |
| `Recovering` | Healthy exchange observed | Require `recovery-cycles` consecutive healthy exchanges before returning to `Running`; clear the stale flag and increment the recovery counter |
| `Closed` | Stop, trap, or setup failure | Stop exchanging and release the master; do not rewrite outputs |

A runtime fault is non-fatal and recovery continues indefinitely. The master never zeroes
outputs, defaults inputs, or exits on a fieldbus fault; the stale flag and status registers tell
the PLC what happened, and the PLC logic decides. `STC-E-IO-ETH-009` is removed from the fault
policy and replaced by `STC-W-IO-ETH-003`. Connect/disconnect of a configured slave is normal;
adding or removing a device is a topology change. This is not a safety function.

Fault state is per backend. The policy above is the EtherCAT policy; WL-5.1 defines the Modbus
policy. The coordinator reports every backend's state and keeps one backend's fault from masking
or silently changing another backend's data. If the run ends (stop, trap, or setup failure), all
backends are shut down in reverse order.

Reconnect of a configured slave is normal and is covered by the recovery path. Adding or removing
a slave is a topology change and requires a cold download; the runtime does not reconfigure the
master while running.

### 6.6 Online change

The coordinator computes a configuration hash over the selected profile, device tree, backend
kinds, PDO/channel map, and effective bindings, including each bound variable's image area,
offset, and width. During an online change:

- a code-only or same-configuration swap keeps the backends active, their mappings live, and the
  last published outputs live;
- a new generation whose hash differs is rejected with `STC-E-ONLINE-IO-001` and the active
  generation continues; the deployment path must use a cold download;
- the image remap rules already implemented by the host apply; the backend re-resolves bindings
  only when the image layout changes compatibly.

### 6.7 Status and diagnostics

`--status` and the control/status surface add a per-backend I/O summary list. For EtherCAT:
master index, slaves responding and configured, OP state, WC state, exchange count, fault count,
last fault, and the configuration hash. A second backend adds its own entry without changing the
EtherCAT fields.

Status and diagnostics are also exposed to the PLC, not only to the operator. The project may
declare a read-only status device (`kind = "ethercat-status"`, `config = { layout = 1 }`) and
bind `AT %I` variables to its channels. The EtherCAT backend fills the block at the input
boundary before the module runs, so the PLC sees the current state in the same scan. The channel
set is versioned and unknown channels are rejected. Proposed layout:

| Channel | Type | Meaning |
| --- | --- | --- |
| `master.phase` | `UINT` | IgH master phase |
| `master.active` | `BOOL` | Master has an active application |
| `master.slaves_responding` | `UINT` | Responding slave count |
| `master.link_up` | `BOOL` | EtherCAT link state |
| `master.last_error` | `UDINT` | Last backend error code |
| `master.recoveries` | `UDINT` | Successful recovery count |
| `domain.wc_state` | `UINT` | Working-counter state |
| `domain.working_counter` | `UINT` | Last working counter |
| `slave<N>.al_state` | `UINT` | Per-slave application-layer state |
| `slave<N>.online` | `BOOL` | Per-slave online flag |
| `slave<N>.operational` | `BOOL` | Per-slave operational flag |
| `input.stale` | `BOOL` | Inputs are the last received values, not current |
| `cycle.exchanges` | `UDINT` | Successful exchanges since start |
| `cycle.faults` | `UDINT` | Fault episodes since start |

The status device is optional; the runtime implements the full set and the project binds only the
fields it needs, so unbound fields cost no image bytes. `--status` prints the same information
for operators. The status block is part of the process image, so the PLC can react to
`input.stale`, per-slave state, and the counters in its normal logic. It is diagnostic, not
safety-rated.

The phase-2 diagnostic codes are:

| Code | Condition |
| --- | --- |
| `STC-E-IO-BINDING-001` | A sink has more than one source (fan-in), or two routes claim the same output |
| `STC-E-IO-BINDING-002` | A route endpoint does not resolve, or its direction/width is invalid |
| `STC-E-IO-BINDING-003` | A route names a device with no registered backend |
| `STC-W-IO-ROUTE-001` | An unrouted direct `%I` variable keeps its declared/default value |
| `STC-W-IO-ROUTE-002` | An unrouted direct `%Q` variable is discarded |
| `STC-E-IO-ETH-001` | The shim/library is missing, cannot be loaded, or reports an incompatible ABI |
| `STC-E-IO-ETH-002` | The IgH master cannot be requested (busy, unavailable, or permission denied) |
| `STC-E-IO-ETH-003` | Slave configuration failed (identity mismatch or invalid position/alias) |
| `STC-E-IO-ETH-004` | PDO/sync configuration or entry registration failed |
| `STC-E-IO-ETH-005` | Master activation or domain data failed |
| `STC-E-IO-ETH-006` | The EtherCAT device or channel configuration is invalid |
| `STC-E-IO-ETH-007` | A bound channel does not resolve, or direction/width mismatches |
| `STC-E-IO-ETH-008` | Startup did not reach OP within the startup timeout |
| `STC-W-IO-ETH-001` | A declared EtherCAT channel is unbound |
| `STC-W-IO-ETH-002` | DC was requested but no `assign-activate` is configured |
| `STC-W-IO-ETH-003` | Fieldbus fault: outputs unchanged, inputs stale, status registers updated; recovery continues |
| `STC-E-ONLINE-IO-001` | Existing code: the EtherCAT configuration changed and a cold download is required |

### 6.8 Metrics

A separate optional `stc-io-metrics/1` JSONL stream keeps the existing `stc-rt-metrics/1` schema
unchanged. Each cycle record carries the cycle number, the backend and device identity,
deadline/wake/latch/scan-end times, input exchange duration, output exchange duration, WC state
and working counter, AL/OP state, fault flag, send errors, and (when DC is active) application
time and clock offset. Multiple backends write the same stream with their identity fields, so a
future Modbus backend can share it. The harness merges the stream with the RT metrics by cycle
and computes per-backend percentiles and fault counts.

Frame loss is any of: domain WC state not OK, a slave not operational, a send/receive error, a
missed exchange, or an increase in the IgH master's lost-frame, Tx-error, or frame-loss counters.
The harness samples `ethercat master` before and after each run and records the domain and slave
state per cycle. Those three master counters are not exposed by ecrt 1.6.13 and are not in the PLC
status block; they are read from the CLI at run boundaries.

### 6.9 Cycle-time sweep

The timing campaign finds the fastest cycle time the host, IgH master, and slave sustain without
losing frames, using a non-CPU-bound PLC program. The fixture copies inputs to outputs and
advances a counter; its execution time must stay well below the candidate cycle time so the
result is a fieldbus and scheduling limit, not ST compute.

- Steps: 1 ms, 500 µs, 250 µs, 200 µs, 150 µs, 100 µs, 75 µs, 50 µs. Start at 1 ms and go down;
  stop at the first step that loses a frame. If 50 µs passes, extend the sweep below 50 µs until
  the first failure.
- Each step gets a 60 s screening run. The fastest passing step gets a 10 min confirmation run
  (three repetitions where time allows).
- A step passes only with zero lost frames, zero WC/AL faults, zero skipped periods and missed
  deadlines, the slave operational throughout, and execution p99 well below the cycle time.
- Record the first failing step and whether the failure is WC/AL, send/receive, master lost
  frames, or skipped periods. The fastest maintainable CT is the fastest confirmed passing step;
  it is host- and hardware-specific.
- The sweep uses the tuned host profile, release build, same IgH master state, and a fixed
  workload; the host snapshot and master counters go in every run header.
- The DIEWU sweep runs free-run; distributed clocks are not part of the CT result.
- The CT sweep runs with open-ended I/O; a cycle succeeds at the frame level: `ecrt_master_send`
  accepts the frame, the receive/process path sees the reply, the domain WC state is healthy, and
  the master's lost-frame and Tx-error counters do not increase. Physical I/O values are not
  verified; the automated all-points I/O correctness matrix is phase 6.

## 7. Work breakdown

| Phase | Work | Acceptance | Size |
| --- | --- | --- | --- |
| 0 | Lock the configuration/profile schema, routing schema, multi-backend coordinator contract, status register layout, and sample fixture; record this baseline | Schemas, coordinator contract, status layout, and fixture reviewed; no code started before the profile model, routing model, coordinator contract, and status layout are settled | S |
| 1 | Backend surface, routing, and mapping: manifest/CLI/metadata/C-ABI spelling, `[io].config` profile resolution and embedding, runtime metadata parsing, per-device backend dispatch, coordinator, profile `[[routes]]` parsing/validation and route metadata, fan-in rejection, route application with a mock backend, binding grouping/conflict checks, process-image mapping, unit and property tests; no hardware | `ethercat` is accepted end to end; device `kind` selects a backend; routes fan out and reject fan-in; physical→physical and PLC→PLC routes run with the mock backend; the coordinator runs EtherCAT plus a mock second backend over one image; invalid configs and conflicting sinks fail closed; mapping tests pass on Windows and Linux; existing null/memory/file behavior is unchanged | M |
| 2 | Shim and lifecycle: C shim against `ecrt.h`, `dlopen` loader and version handshake, EtherCAT backend as one coordinator instance, cold start/input/output/shutdown, Linux CI job that builds IgH 1.6.13 userspace and the shim | Shim builds against 1.6.13; missing/incompatible shim errors are clear; the EtherCAT instance cycles through the coordinator; CI job is green; base package has no IgH dependency | M |
| 3 | HIL bring-up on `ecat-host`: install the shim, build the DIEWU fixture, run with open-ended (unwired/unloaded) I/O, reach OP, verify frame acceptance and reply, verify no runtime output rewrite on exit; draft the runbook | DIEWU reaches OP with a healthy WC; IgH accepts and replies to every frame with no send/receive errors or lost frames; shutdown leaves the output image unchanged and the slave watchdog handles stopped frames; a 10-minute run has no unexplained frame-level faults. Physical I/O values are not verified in this phase | M |
| 4 | Software fault, recovery, status: master service stop/start, master already owned, wrong identity, startup timeout; recovery window, transparent output/input behavior, PLC-visible status/diagnostic registers, per-backend fault isolation, code-only and topology-changing swaps | The software-fault matrix passes; outputs are unchanged during faults; stale inputs and status registers are visible; recovery clears the stale flag; one backend's fault does not alter another's data; code-only swaps preserve I/O; a topology/binding change returns `cold_required` or `STC-E-ONLINE-IO-001`. Physical fault injection is phase 6 | M |
| 5 | Campaign and draft publication with open-ended I/O: cycle-time sweep from 1 ms to 50 µs (extending lower if 50 µs passes) with the non-CPU-bound 16I/16O fixture, 60 s screening per step and a 10 min confirmation at the fastest passing CT; a cycle succeeds when IgH accepts and replies to the frame; draft report, runbook, licensing record, IO/CLI/known-limitations updates, the multi-backend seam documented for WL-5.1 | The draft report states the fastest maintainable CT, the first failing CT and failure mode, host, method, distributions, frame-level faults, and limits; the runbook repeats the sweep; licensing and packaging are recorded; the WL-5.1 contract is documented; no safety claim | M |
| 6 | Wiring harness and all manual tests: build the loopback harness (all 16 outputs to 16 inputs with the correct NPN common), run the automated all-points functional matrix, perform the physical fault injection moved from phase 4 (cable unplug/replug, slave power cycle, configured-slave reconnect), finalize the report, runbook, and known limitations; final review | The harness wiring is documented and installed; all 16 inputs and 16 outputs are verified automatically; the physical fault/recovery matrix passes with the harness; the final report separates frame-level from physical I/O evidence; the runbook and known limitations are reviewed | M |

The total is L, matching the parent plan's tentative size. Owners are assigned during phase 0;
each phase ends with a commit, a push, and a green CI run. A phase may fix a failing CI run once;
a second failure stops the sequence for a human decision.

### 7.1 Phase status

| Phase | Status | Commit | CI |
| --- | --- | --- | --- |
| 0 — schema/contracts | Done | `63b8a7e` | [41177379223](https://github.com/r2k-in-the-vortex/stc/actions/runs/41177379223) — success |
| 1 — backend surface, routing, mapping | Done | `d254583` | [41181242085](https://github.com/r2k-in-the-vortex/stc/actions/runs/41181242085) — success |
| 2 — shim, lifecycle, CI build | In progress — human-authorized fourth attempt; anti-link script fix | `c2f8272` + pending fix | [41204656995](https://github.com/r2k-in-the-vortex/stc/actions/runs/41204656995) — failure |
| 3 — HIL frame-level bring-up | Not started | — | — |
| 4 — software fault, recovery, status | Not started | — | — |
| 5 — CT sweep, draft report | Not started | — | — |
| 6 — harness, all manual tests, final report | Not started | — | — |

Status values: Not started, In progress, Done, Blocked. The orchestrator updates this table after
each phase; the next phase's commit carries the update.

## 8. Test matrix

| Area | Environment | Cases | Evidence |
| --- | --- | --- | --- |
| Backend surface | CI, Windows and Linux | `[io].backend`, `--io ethercat`, metadata string, C ABI value; unknown names still rejected | Unit and integration tests |
| Backend dispatch | CI | device `kind` selects a backend; global default applies to unbound areas; unknown kind; EtherCAT plus a mock second backend in one run | Unit and integration tests |
| Config validation | CI | missing/duplicate fields, bad types, invalid master/slave positions, duplicate slave positions, overlapping entries, width mismatches, unknown/duplicate channels, missing channel map, invalid bit range | Unit tests with golden diagnostics |
| PDO mapping and sink conflicts | CI | 1/8/16/32-bit entries, bit offsets, image offsets, unbound inputs/outputs, byte alignment, same-device aliases, duplicate sink rejection | Unit and property tests |
| Routing graph | CI | fan-out, fan-in rejection, physical→physical pass-through, PLC→PLC buffering, cross-backend route, width/direction mismatch, route endpoint resolution, cycle policy | Unit and property tests |
| Multi-backend coordination | CI | two backends latch/publish over one image; deterministic declaration order; per-backend status and metrics; one backend absent or unconfigured | Rust mock tests |
| Shim load | Linux CI | correct version, missing library, wrong ABI, missing symbol, master unavailable | `dlopen` test and error-path tests |
| EtherCAT mock lifecycle | CI | open, OP wait, latch, publish, stop, setup failure, shutdown without output rewrite, send on failed cycle | Rust mock tests |
| HIL functional | `ecat-host` + DIEWU | phases 3–5: open-ended I/O, OP, WC healthy, IgH frame acceptance/reply, no send/receive errors, no runtime output rewrite on exit; phase 6: all 16 inputs and 16 outputs via the loopback harness, 10-minute run | Logs, master counters, run manifest, wiring note |
| HIL software fault | `ecat-host` | master service stop/start, master already owned, wrong identity, startup timeout; verify outputs unchanged, stale inputs and status registers, recovery clears stale | Logs, status-register traces |
| HIL physical fault | `ecat-host`, phase 6 | link unplug/replug, slave power cycle, configured-slave reconnect; verify outputs unchanged, stale inputs and status registers, recovery clears stale | Logs, status-register traces, output monitor |
| Status registers | CI + HIL | version/layout, unknown channel rejection, values track master/slave state, stale set/cleared, read-only direction enforced | Unit, integration, and HIL traces |
| Config profile | CI | `[io].config` selection, embedding, inline/profile precedence, profile route validation, `--io-config` override, missing/invalid profile, path escape rejection | Unit and integration tests |
| Fault isolation | CI | EtherCAT fault does not alter the mock second backend's outputs; mock-backend fault does not alter EtherCAT's; aggregate status reports both | Rust mock tests |
| HIL online change | `ecat-host` | code-only swap, image-compatible swap, binding/PDO change, device removal | Status and control-channel evidence |
| Timing / CT sweep | `ecat-host` | non-CPU-bound 16I/16O fixture; 1 ms → 50 µs steps, extending lower if 50 µs passes; 60 s screening per step; 10 min confirmation at the fastest passing CT; master lost frames/Tx errors, WC/AL state, skipped periods, execution p99; release primary and debug secondary | JSONL, master logs, report |
| Build and packaging | CI | base package unchanged; optional artifact; checksums; no IgH link in base | CI artifacts and package inspection |

Phases 3–5 use open-ended I/O and frame-level success; they do not require the loopback harness.
Phase 6 supplies the wiring diagram and procedure, runs the automated all-points matrix, and
performs all manual/physical tests; building and attaching the harness is physical work for the
owner or technician. If physical constraints prevent the harness, a recorded manual procedure is
the fallback and its limitation goes into the report.

Multi-slave configurations are covered by configuration/mapping and mock tests. The v2.0.0 HIL
evidence is single-slave (E-9).

## 9. Evidence and deliverables

- Code: the runtime I/O coordinator and multi-backend interface, the EtherCAT backend, config
  parser and mapping, shim, versioned loader, per-backend status fields, metrics stream, and
  tests.
- Multi-backend seam: per-device backend dispatch, binding groups, conflict checks, deterministic
  ordering, per-backend fault isolation, and a mock second backend. WL-5.1 attaches Modbus as
  another instance without changing the EtherCAT backend.
- Routing: the machine-profile `[[routes]]` schema, compiler resolution and metadata,
  fan-out/fan-in validation, examples for normal input/output, physical pass-through, and
  PLC→PLC simulation, and router tests across the mock second backend.
- Fixture: `tests/ethercat/diewu/` with the `stc.toml` device tree, 16 input and 16 output
  channel map, bindings, the selected source profile, the status-device bindings, and a
  non-CPU-bound input-to-output program that exercises all points.
- Profile selection: `[io].config` resolution and canonical embedding, sample `io/bench.toml`
  and `io/line1.toml` profiles, and the `--io-config` diagnostic override.
- PLC-visible status: the E-14 channel layout, the fixture's status variables, and tests for
  stale flags, per-slave state, counters, and read-only enforcement.
- Shim and build notes: `tools/ethercat/` (C source, version header, build script, sample
  profile). CI builds the shim against pinned IgH 1.6.13; the optional artifact carries it.
- Loopback harness (phase 6): wiring diagram, parts list, and procedure for all 16 outputs to 16
  inputs, plus the automated all-points matrix (or a recorded manual bench fallback if the harness
  cannot be built).
- Optional package artifact: `stc-io-ethercat` with the shim, sample configuration, notices, and
  SHA256.
- Report: `specification/Verification/EtherCAT-Host.md` with host, IgH version, slave identity,
  wiring, the CT-sweep table, the fastest maintainable CT, the first failing CT and failure
  mode, transparent-fault evidence, status-register traces, distributions, faults, and limits.
  It separates frame-level evidence (phases 3–5) from physical I/O evidence (phase 6).
- Runbook: `specification/Verification/EtherCAT-Runbook.md` with prerequisites, shim install,
  fixture build, profile selection, the open-ended phase 3–5 procedure, the phase-6 harness and
  manual tests, physical fault injection, recovery, and cleanup.
- Specification updates: [I/O Routing and Backends](specification/Architecture/IO-Routing-and-Backends.md)
  (the phase-0 contract: machine profiles, route graph, coordinator, status device, fault model,
  and online change), [IO and Devices](specification/Architecture/IO-and-Devices.md) §6–§7
  (per-device backend dispatch, the coordinator, and binding rules),
  [CLI and Manifest](specification/Architecture/CLI-and-Manifest.md) `[io]` and `--io-config`,
  [Runtime and Execution](specification/Architecture/Runtime-and-Execution.md) run-option
  surface, [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md), and the decisions register.
- WL-5.1 handoff: the coordinator/backend contract, the device-`kind` registration model, and
  the binding-group rules, so Modbus attaches as a second backend without EtherCAT changes.
- Raw results retained as campaign artifacts with SHA256; summaries and reports committed.

## 10. Risks and mitigations

- **Servo hardware is not available (owner-confirmed 2026-10-31).** The locked scope includes
  servo-drive integration, but only the DIEWU digital slave is attached. Mitigation: implement
  and unit-test the generic PDO/DC path, document a servo configuration, and record the
  limitation; a drive can be added in a later target.
- **The DIEWU runs free-run by owner decision (2026-10-31).** The SII has no `AssignActivate`
  value, so DC is implemented but not exercised on the DIEWU. Mitigation: exercise DC with a
  DC-capable slave or a vendor ESI if one becomes available; record the limitation in the report
  and known limitations.
- **IgH is coupled to the kernel.** A kernel update requires rebuilding the modules. Mitigation:
  pin kernel and IgH versions in the runbook; record the pairing; verify after any update.
- **The master is single-owner.** Another application holding the master blocks startup.
  Mitigation: startup check, clear diagnostic, runbook step to stop conflicting applications.
- **The fastest CT may be fieldbus-limited, not runtime-limited.** The sweep stops at the first
  lost frame and records whether the limit is IgH/NIC/slave or the runtime; it does not assume a
  target CT. Results are host- and hardware-specific.
- **The 1 ms debug margin is tight.** Mitigation: release is the primary timing evidence; debug
  is recorded as the default with its margin; no hard-RT claim.
- **Transparent fault handling depends on the PLC.** The runtime does not zero outputs, default
  inputs, or exit on a fieldbus fault; the PLC must use `input.stale` and the status registers.
  Mitigation: document and test the status block, exercise configured-slave disconnect/reconnect,
  and state that it is not safety.
- **Profile and route selection can drift.** Multiple source profiles can be mixed up, or the
  embedded profile/routes can go stale. Mitigation: `[io].config` is explicit, the compiler
  embeds the selected file and validates routes against the program, the online-change hash
  includes it, and tests cover selection, precedence, route validation, and path escape.
- **No loopback harness exists yet.** Phases 3–5 do not need it; they run open-ended I/O and
  frame-level success. Phase 6 builds it and runs the automated all-points matrix; the fallback
  limitation goes into the report.
- **Frame-level success is not physical I/O proof.** Open-ended I/O means floating inputs and
  unloaded outputs; a healthy WC and accepted/replied frames prove the fieldbus path, not the
  field wiring or signal integrity. Mitigation: state the limitation, and verify physical I/O in
  phase 6 with the harness.
- **Physical fault tests are deferred to phase 6.** Cable unplug, slave power cycle, and
  configured-slave reconnect need the final manual phase. Mitigation: schedule the phase-6
  window and keep the software-fault matrix in phase 4.
- **Multi-slave is not HIL-tested.** The HIL evidence is single-slave; multi-slave is covered by
  configuration/mapping and mock tests. Mitigation: document the boundary and add HIL when a
  second slave is available.
- **Shim/ABI drift.** Mitigation: a CI build against pinned IgH 1.6.13, a version handshake, and
  a load test against the host `libethercat`.
- **Licensing and packaging.** Mitigation: phase-0 review, dynamic linking, no kernel-module
  redistribution, shim source and notices.
- **Online change across a topology change.** Mitigation: configuration hash, rejection, and cold
  download; tests.
- **AArch64 is not hardware-verified.** Mitigation: Linux x86-64 only for hardware evidence;
  the mapping is explicitly little-endian; build-only AArch64 is documented.
- **Reference host drift.** Mitigation: the STC runbook is authoritative and records the exact
  host/IgH versions; the external repository is cited as the provisioning reference.
- **Privileges for FIFO and the master device.** Mitigation: extend the existing root-owned
  wrapper with a fixed `ethercat-run` subcommand, or install a second narrow wrapper; no broad
  NOPASSWD rule.
- **Parallel backend conflicts.** Two backends could claim the same sink, ordering could be
  ambiguous, or one backend's fault could mask another's. Mitigation: sinks have one source,
  conflicting claims are rejected, order is device declaration order, fault state is per backend,
  and a mock second backend tests the seam.
- **Route graph mistakes.** Accidental fan-in, route cycles, or a wrong PLC→PLC timing choice can
  produce surprising data. Mitigation: compile-time and cold-start validation, a documented cycle
  policy, and tests for fan-out, fan-in, pass-through, and simulation routes.
- **PLC→PLC latency.** A boundary-applied route adds one scan of latency; same-scan routing needs
  task-boundary hooks. Mitigation: choose and document the policy, expose the route behavior in
  the tests, and state the latency in the specification.
- **WL-5.1 integration drift.** Modbus has a different lifecycle (non-RT, network) and could
  force the coordinator interface to change. Mitigation: settle the coordinator/backend contract
  with WL-5.1 before phase 1 locks it; keep the EtherCAT backend unaware of other backends; the
  contract is a deliverable.
- **Mixed-project configuration ambiguity.** `[io].backend` cannot name two hardware backends.
  Mitigation: device `kind` is authoritative; the global value is only the default for unbound
  areas and projects without hardware devices; document the mixed-project form.

## 11. Exit criteria

Mapped from the parent plan:

- The backend runs on `ecat-host` with IgH 1.6.13 and the delivered EtherCAT hardware.
- Cyclic process data is latched and published deterministically at the cycle boundary; the
  functional and timing evidence shows no unexplained lost exchanges.
- Startup, software fault, and shutdown paths are tested in phases 3–5; physical fault injection
  and configured-slave reconnect are tested in phase 6. A fieldbus failure leaves outputs
  unchanged, presents the last received inputs as stale, and is visible in the PLC
  status/diagnostic registers; recovery clears the stale state.
- Phases 3–5 use open-ended I/O and frame-level success; the report distinguishes frame-level
  from physical I/O evidence.
- Adding or removing a device is rejected as a topology change and requires a cold download.
- The device tree and channel descriptors map to IgH configuration without protocol details
  leaking into the language or compiler.
- Hardware-in-the-loop tests and I/O-cycle timing and jitter measurements are recorded.
- The fastest maintainable cycle time without lost frames is recorded with the first failing CT,
  the failure mode, and the master/WC/AL evidence; the fixture is non-CPU-bound.
- The IgH licensing and packaging review is complete; the runbook and specification updates are
  reviewed.
- The backend is documented as Linux-only and not a safety function.

Additional sub-plan criteria:

- The base `stc` package still builds and runs on all supported hosts without IgH.
- The optional artifact installs and loads only when at least one EtherCAT device is configured.
- Online change rejects an EtherCAT topology/binding change and requires a cold download.
- The servo hardware limitation and the DIEWU free-run DC limitation are stated in the report and
  known limitations.
- The selected I/O profile (backend config and routes) is source-controlled, embedded in the
  artifact, and reproducible; `--io-config` is documented as a diagnostic addressing override
  that does not change routes.
- The PLC-visible status/diagnostic register layout is documented and tested, including the stale
  flag, per-slave state, counters, and read-only enforcement.
- HIL evidence is single-slave; multi-slave configurations are covered by mock/config tests and
  the boundary is documented.
- Final phase 6 verifies all 16 inputs and 16 outputs automatically through the loopback harness;
  if the harness cannot be built, the manual fallback is documented.
- No ST language, compiler-feature, or artifact-format change is introduced beyond the backend
  spelling and the runtime metadata/CLI surface required by this target.
- The runtime I/O interface is multi-backend: the coordinator runs EtherCAT plus a mock second
  backend over one process image, and WL-5.1's Modbus backend can attach without changing the
  EtherCAT backend.
- The routing graph supports physical→PLC, PLC→physical, physical→physical, and PLC→PLC routes
  with fan-out; fan-in is rejected; the PLC→PLC latency policy is documented and tested.
- Cross-backend routes and sink conflicts are tested; conflicting claims are rejected at cold
  start.
- Fault state is per backend; one backend's fault does not silently alter another backend's data.

## 12. Open questions

Resolved 2026-10-31:

- **Servo drive:** none available. E-10 is accepted: DIEWU-only hardware evidence, generic PDO/DC
  path unit-tested, limitation documented.
- **DIEWU DC:** free-run. DC is implemented but not exercised on the DIEWU; a DC-capable slave or
  vendor ESI would be needed for DC evidence.
- **Fault policy:** transparent master. Outputs are sent unchanged, the last received inputs are
  presented as stale, master/slave status and diagnostics are exposed to the PLC, and recovery
  continues; configured-slave reconnect is normal. Not safety. E-6 and E-14 record it.
- **I/O config:** `--io-config` is used, and the profile file is a source artifact selected by
  `[io].config`; the runtime flag is a diagnostic override. One profile per machine. E-2 records
  it.
- **Shim packaging:** CI builds the shim against pinned IgH 1.6.13 and ships it in the optional
  artifact. E-8 is accepted.
- **HIL access:** `ecat-host` is available for the full matrix on a scheduled basis.
- **Loopback:** no harness exists yet. Final phase 6 builds it and runs the all-points matrix;
  phases 3–5 use open-ended I/O and frame-level success.
- **Hot-swap:** configured-slave reconnect only. Adding/removing a device is a topology change
  and requires a cold download. E-6/E-7 record it.
- **Backend ABI:** the contract is a Rust trait in `stc-runtime`; WL-5.1 adds a Modbus module or
  linked crate. E-13 is accepted.
- **Status layout:** the runtime implements the full E-14 set; the project binds only the fields
  it needs. E-14 is accepted.
- **Profile schema:** one source-controlled profile per machine, selected by `[io].config`.
- **AArch64:** x86-64 is the only hardware-evidence platform. E-12 is accepted.
- **HIL slave set:** single DIEWU slave first; multi-slave is mock/config-tested only. E-9 is
  accepted.
- **Backend dispatch:** per device `kind`; `[io].backend`/`--io` are defaults for unbound areas.
  E-3 is accepted.
- **Routing:** physical→PLC, PLC→physical, physical→physical, and PLC→PLC; fan-out allowed,
  fan-in rejected. E-4 is accepted; E-15 covers the route table.
- **Online change:** hash the profile/device tree/PDO map/routes; code-only swaps stay online,
  any configuration change requires a cold download. E-7 is accepted.
- **Licensing:** dynamic-link the userspace library, ship the shim source, do not redistribute
  kernel modules, include notices, complete the review in phase 0. E-11 is accepted.
- **Route timing:** PLC→PLC routes apply at the input boundary with a one-scan delay. E-15 is
  accepted.
- **Route location:** routes live in the selected machine profile alongside the PDO map;
  `stc.toml` keeps the logical device tree and channel descriptors. E-15 is accepted.
- **Route cycles:** cycles are allowed and resolved by the one-scan buffer as expected behavior,
  with no warning. E-15 is accepted.
- **Backend dispatch, online change, licensing:** E-3, E-7, and E-11 are accepted.
- **Test sequencing:** all manual/physical tests are phase 6; phases 3–5 use open-ended I/O and
  treat frame acceptance/reply by IgH as success.
- **Phase-2 contract decisions:** `dc.assign-activate` is a u16 integer (E-17); lost-frame and
  Tx-error channels are removed from the PLC status block and read from `ethercat master` at run
  boundaries (E-14); E-16 warnings are emitted only when a machine profile is selected.

Remaining: no owner-level questions. All decisions E-1–E-17 are accepted. Implementation details
such as the exact trait names, the `[io].config` key spelling, and the status channel names are
phase-0/1 defaults and do not change the scope.

## 13. Change log

| Date | Change |
| --- | --- |
| 2026-10-31 | Started the WL-5.2 sub-plan. Verified the dedicated `ecat-host` baseline (kernel `6.12.115-ecat-rt`, IgH 1.6.13, DIEWU digital slave, master and slave artifacts), mapped the runtime integration points, and proposed decisions E-1–E-12. |
| 2026-10-31 | Added the multi-backend requirement: WL-5.1's Modbus TCP backend will run in parallel, so the runtime I/O interface is per-device and multi-backend from the start. E-13 adds the coordinator, binding groups, conflict checks, per-backend fault isolation, and a mock second backend; the EtherCAT backend itself is unchanged. |
| 2026-10-31 | Made the DIEWU fixture bit-level 16I/16O and changed the timing campaign to a cycle-time sweep for the fastest frame-safe CT, using a non-CPU-bound PLC program. |
| 2026-10-31 | Accepted the owner decisions: DIEWU-only HIL, DIEWU free-run, transparent fault handling with PLC-visible status registers and automatic recovery, and source-selected `[io].config` profiles with a runtime `--io-config` override. Updated E-2, E-5, E-6, E-10, E-14, the lifecycle, status model, test matrix, and open questions. |
| 2026-10-31 | Accepted CI-built shim packaging, dedicated HIL access, no current loopback harness (phase-3 prerequisite), and configured-slave reconnect only. Updated E-6, E-8, the phase-3/test matrix, risks, exit criteria, and open questions. |
| 2026-10-31 | Closed the remaining design questions: Rust-trait backend contract in `stc-runtime`, full PLC status set bound as needed, one I/O profile per machine, x86-64-only hardware evidence, and single-slave HIL. Updated E-2, E-9, E-12, E-13, E-14, the architecture, test matrix, risks, and exit criteria. |
| 2026-10-31 | Moved the loopback wiring harness and the automated all-points I/O matrix to final phase 6; phases 3–5 use a recorded manual/bench procedure. Updated E-9, the phase table, test matrix, deliverables, risks, and exit criteria. |
| 2026-10-31 | Accepted E-3 per-device dispatch, E-4 routing rules, E-7 hash/cold online change, and E-11 licensing. Added E-15 and §6.3.1: fan-out routes, single-source sinks, physical→physical and PLC→PLC routing, route metadata, and boundary application. |
| 2026-10-31 | Accepted the last route decisions: one-scan PLC→PLC delay, routes in the machine profile with the PDO map, and route cycles allowed without warning. All decisions E-1–E-15 are now accepted. |
| 2026-10-31 | Moved all manual/physical tests to phase 6. Phases 3–5 run open-ended (unwired/unloaded) I/O and treat a frame accepted and replied by IgH as success; phase 6 runs the loopback harness, the all-points matrix, and physical fault injection. Updated E-9, the phase table, test matrix, risks, and exit criteria. |
| 2026-10-31 | Phase 0 complete: commit `63b8a7e`, CI run 41177379223 green. The contract is locked in `specification/Architecture/IO-Routing-and-Backends.md`; phase 1 started. |
| 2026-10-31 | Phase 1 complete: commit `d254583`, CI run 41181242085 green. Profile embedding, route graph, coordinator, mock backends, and mapping implemented with tests. Phase 2 paused for confirmation of two phase-1 defaults: profile raw-text embedding vs canonicalization, and runtime-only fail-closed validation for routed physical outputs. |
| 2026-10-31 | Owner confirmed raw profile text + SHA-256 and directed that unrouted PLC `%I`/`%Q` variables be warnings, not errors; an unrouted `%I` uses its default value and an unrouted `%Q` is discarded. Added E-16 and resumed phase 2 with the warning follow-up. |
| 2026-10-31 | Phase 2 second CI run failed in the Linux shim build: `stc_ec_master_status`/`stc_ec_domain_status`/`stc_ec_slave_status` are used as both typedefs and exported functions. One fix was attempted (run 41203057703); no third attempt was made. Phase 2 blocked for a human decision. Also pending: `dc.assign-activate` type, lost-frame/Tx-error status channels, and E-16 warning gating. |
| 2026-10-31 | Owner authorized a third phase-2 attempt. Set `dc.assign-activate` to u16 (E-17), removed lost-frame/Tx-error channels from the PLC status block (E-14; `ethercat master` at run boundaries), and kept E-16 warnings profile-gated. |
| 2026-10-31 | Phase 2 third run: the shim compiles and the loader/lifecycle tests pass; the anti-link step fails because it looks for `libstc_runtime-*.rlib` while a clean `cargo build -p stc-runtime` produces `libstc_runtime.rlib`. Human decision needed for a fourth attempt; suggested fix is to match `libstc_runtime*.rlib`. |
| 2026-10-31 | Owner authorized a fourth phase-2 attempt; fix the anti-link script to match the unhashed runtime rlib and rerun CI. |
