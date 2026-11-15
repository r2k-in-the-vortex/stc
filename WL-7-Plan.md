# WL-7 Plan — EtherCAT engineering and device configuration

**Status:** Revised proposal incorporating the owner's architectural direction, reviewed against the
branch on 2026-11-15 and aligned to
[Feature Development Workflow](workflows/FEATURE-DEVELOPMENT.md). Phase 0 approval is required
before implementation. No implementation commit is implied by this document.

- Date: 2026-11-15
- Branch: `WL-7`, branched from `v3.0.0-planning` at `7e053a4`
- Parent planning item: `v3.0.0-plan.md` §4 WL-7, re-planned by the owner on 2026-11-15 to this
  scope (§1 row, §3 constraint, §4 target section, retirement checklist, and change log aligned)
- Wishlist entry: `FutureWishlist.md` WL-7, aligned to the same scope
- Baseline: `stc` 2.0.0 plus delivered v3.0.0 targets
- Prerequisite: WL-5.2 EtherCAT backend (IgH 1.6.13 shim, DIEWU slave, static profile)
- Profile: `stc-2013-oop-utf8-v1`
- Working plan location: repository root, temporary; deleted at closeout (workflow §2, §10)

This document follows the workflow's phase discipline: Phase 0 closes every owner question (§13),
no implementation starts before approval, each phase ends with a pushed commit and green CI, and
the pre-PR manual walkthrough is always required. §4.1 records the branch review that produced the
extra decisions in §13.

---

## 1. Goal

Deliver an **EtherCAT engineering subsystem** for `stc`, conceptually similar to the configuration/engineering mode of an industrial PLC environment. It must let an engineer describe, discover, inspect, configure, validate, commission and persist an EtherCAT machine before normal PLC runtime operation.

ESI files are **device capability descriptions**, not the desired machine configuration. They tell the engineering subsystem what a device supports; they do not decide which PDOs, operating mode, SDO parameters or logical PLC channels the engineer intends to use.

The subsystem must support both:

1. **Offline design:** create a machine topology and device configuration manually from schematics and engineering information, before the physical machine exists.
2. **Online commissioning:** discover the physical bus, identify devices, inspect live configuration and object dictionaries, read/write SDOs, compare observed state with the intended project configuration, and capture or apply configuration.

A device's presence on the bus and the availability of an ESI file are not sufficient to configure it automatically. The engineer must choose among the capabilities exposed by the ESI and the live device. For example, a servo drive may offer cyclic synchronous position, velocity and torque modes, each with a different PDO arrangement and potentially different startup SDO requirements.

The resulting desired configuration is persisted in the project. The compiler consumes that already-resolved configuration and validates it; it does not perform ESI discovery or make engineering choices. The runtime applies the configured startup sequence, configures the master/slaves, verifies the resulting state and runs the PLC.

WL-7 provides a complete CLI engineering workflow and a structured API beneath it. The existing SSH-based remote execution/debug/online-change mechanism is reused for remote engineering commands. No new network service or remote protocol is required.

## 2. Architectural principles and decisions

These are the agreed direction for WL-7 and should be recorded as durable decisions during Phase 0.

### 2.1 ESI describes capabilities; the engineer selects configuration

Keep four distinct concepts:

- **Device description:** ESI-derived identity, capabilities, supported sync managers/PDOs, mailbox/CoE features, DC capabilities and any supported object-dictionary metadata.
- **Desired configuration:** the engineer-authored topology, selected operating mode, selected PDO assignments/mappings, startup SDO operations, DC settings and logical channel mapping.
- **Observed state:** what is actually connected and what the live slave currently reports, including identity, current PDO assignment/mapping where readable, and current SDO values.
- **Effective process image:** the validated, ordered PDO entries and bit layout that the runtime will expose to the PLC.

Do not conflate these layers. In particular, importing an ESI or discovering a slave must not silently choose the desired PDO configuration or generate a complete PLC channel interface.

### 2.2 Configuration-time engineering, not compile-time ESI resolution

ESI ingestion, device selection, topology editing, PDO selection, SDO browsing/commissioning and configuration persistence are engineering/configuration-time activities.

The compiler must not search ESI directories, select an ESI revision, parse vendor XML or infer device configuration during an ordinary build. It consumes a persisted, explicit desired configuration and performs deterministic structural validation. Any required device-description facts needed by runtime are persisted in the resolved project configuration or embedded artifact as appropriate.

The runtime must not need vendor ESI files or reread project ESI paths.

### 2.3 Offline and online topology

The device tree/topology can be:

- authored manually while offline, using schematics and known device identities/positions; or
- populated/updated through an online IgH bus scan.

Online discovery is a source of observed facts, not authority to overwrite engineering intent. Provide an explicit operation to compare discovered topology with the project and an explicit capture/import operation when the engineer wants to use observed topology as a starting point.

The existing `stc.toml` logical device tree and channel declarations remain authoritative for PLC-facing names and types unless a deliberate, separately approved schema change is made.

### 2.4 Master ownership and external masters

Support two distinct access modes:

- **Managed master:** `stc` owns the EtherCAT master exclusively for commissioning/configuration operations and for normal runtime. Only one process may own operations that change master/slave state. The same ownership discipline applies regardless of whether `stc` is invoked locally or remotely.
- **Externally managed master:** `stc` can inspect/integrate with a master owned by another system. This mode is inspection-oriented and must not silently take ownership or issue configuration-changing operations. Any write/configure operation requires the managed-master path and its explicit ownership rules.

Do not build a new network service for remote operation. The existing SSH-based `stc` workflow is the transport for remote CLI execution. The engineering operations themselves must not depend on whether the CLI is local or invoked through SSH.

### 2.5 API-first, CLI in scope, TUI/GUI later

Implement typed engineering operations and structured results/errors in an API/service layer. The CLI is the first complete client and is in scope for WL-7. Do not put engineering business logic in command handlers or return terminal-formatted strings as the API contract.

A future TUI and GUI are explicitly anticipated but are **out of scope** for WL-7. The API should leave that door open: structured data, typed values, stable error categories, progress/status for longer operations, and separation of read-only inspection from hardware-changing operations. A TUI may be delivered before a full GUI without requiring a second configuration engine.

### 2.6 Preserve existing PLC/runtime contracts where possible

Keep the language and compiler protocol-agnostic. Preserve the `DeviceBackend` abstraction, coordinator cycle order, route graph, process image and existing status/metadata contracts unless a specific, reviewed change is necessary. Existing hand-written static EtherCAT profiles must remain supported as a compatibility path during the transition.

## 3. Scope

These bounds replace the §4 WL-7 entry in [v3.0.0-plan.md](v3.0.0-plan.md), which was re-planned
to this scope on 2026-11-15. The target identity in the locked v3.0.0 set is unchanged; changing
anything below still needs an owner re-plan (workflow §12).

### 3.1 In scope

- Bounded ESI ingestion, parsing, validation, identity/revision matching and a local vendor ESI library/catalog.
- ESI inspection: supported device identities, sync managers, PDOs/entries, DC modes, mailbox/CoE features and supported object-dictionary metadata.
- Manual offline topology authoring and online bus discovery.
- Comparison of desired topology/configuration with observed bus/device state.
- Device inspection and identification, including identity and revision.
- PDO capability inspection and explicit engineer selection of operating mode, PDO assignment and mapping.
- Object dictionary browsing and typed SDO read/write operations where supported by the device and current EtherCAT state.
- Distinction between temporary commissioning writes and persisted startup SDO operations.
- Ordered startup SDO sequences, including state/precondition handling and read-back verification where applicable.
- Persisting the desired machine configuration in project-owned configuration files.
- Mapping selected PDO entries to explicitly declared PLC channels; do not automatically expose every ESI entry as a PLC channel.
- API-first engineering operations and a CLI that exposes the complete WL-7 engineering workflow.
- Local and remote CLI use through the existing SSH mechanism.
- Managed-master exclusive ownership and separate inspection/integration for externally managed masters.
- Runtime application and verification of the persisted desired configuration.
- Synthetic fixtures, malformed-input/fuzz tests, vendor-corpus testing, simulator tests and hardware-in-the-loop (HIL) evidence.

### 3.2 Out of scope

- TUI and GUI implementation. Their future compatibility is an architectural requirement, not a WL-7 deliverable.
- A new network daemon, RPC protocol or remote transport. Reuse the existing SSH workflow.
- Full ESI/XSD coverage or complete modelling of every vendor extension and object-dictionary construct.
- EoE, FoE, AoE, SoE, redundancy, hot-connect and cable redundancy configuration.
- Dynamic PDO remapping while the PLC is executing its cyclic runtime. Engineering changes require an appropriate stopped/commissioning state and explicit application.
- General-purpose motion control, servo tuning, tuning wizards or vendor-specific drive commissioning workflows. WL-7 must support selecting a drive's advertised mode and configuring the associated PDO/SDO requirements, but does not claim to implement motion-control functionality.
- Safety functions or any claim that `stc` is a safety PLC. See `NON-SAFETY.md`.
- Hard-real-time guarantees beyond the existing runtime contract.
- New core language features. Compiler work remains in manifest/configuration validation and artifact embedding.
- Vendor ESI files, TwinCAT installations or external services as build/CI/release prerequisites. Vendor corpus remains local-only unless redistribution permission is confirmed.

## 4. Starting point and existing contracts

The following are the known baseline from the original WL-7 plan and must be rechecked against the branch before implementation:

- `stc.toml` owns the logical device tree and `[[io.devices.channels]]` declarations (names, directions, widths). The machine profile references devices by name and carries the EtherCAT mapping.
- The manifest/profile path parses `[device.<name>]` and routes into `Profile`/`ProfileDevice` and a `ConfigValue` tree. The compiler canonicalizes and embeds the selected effective configuration in `.stc-meta.json`; runtime does not read the profile file.
- `crates/stc-runtime/src/io_ethercat.rs` configures slaves through the project-owned shim `tools/ethercat/stc_ec_shim.{h,c}`, using IgH 1.6.13, `dlopen`, and ABI `stc-io-ethercat/1`.
- The current shim (525 lines) exposes lifecycle, configure, register, status, and cycle calls
  only. It checks vendor/product through `ecrt_master_slave_config()` and, when
  `check_revision` is set and the alias is zero, reads `revision_number` once through
  `ecrt_master_get_slave()`. It returns no identity fields to the backend, never reads the PDO
  assignment, and has no SDO call of any kind.
- Existing stable diagnostics include `STC-E-IO-ETH-001`–`008` and `STC-W-IO-ETH-003`; status layout and `stc run -v` keys are established contracts.
- Backend tests use a mock shim on non-Linux CI hosts; driver/CLI fixtures live under `crates/stc-driver/tests`; fuzz targets live under `fuzz`; HIL uses the Linux `ecat` host and DIEWU 16I/16O bench per `specification/Verification/EtherCAT-Runbook.md`.
- No XML dependency currently exists in the workspace; workspace lints forbid `unsafe_code`. Parser choice remains a Phase 0 technical decision.

The old static profile path remains a valid starting point and regression baseline. WL-7 should not break existing projects merely because the engineering subsystem is added.

### 4.1 Branch review (2026-11-15): what the tree actually provides

Every row below was checked in the working tree, not assumed. The consequence column is why the
matching Phase 0 decision exists.

| Area | Observed state | Consequence for this plan |
| --- | --- | --- |
| Remote execution | `stc remote` parses only the `status` action (`crates/stc-driver/src/cli.rs:938`) and dispatches to `remote_launch::status_command`; `stc serve` offers status/deploy/stop over the `stc-remote-session.json` descriptor; `remote.rs` owns an SSH/SFTP runner but no generic command execution. | §6.3's "reuse SSH" is **not available today**. A remote engineering path needs new work: either a generic `stc remote` execution action with project sync and fingerprint, or an explicit decision that WL-7 engineering is local-only. Blocks the Phase 9 walkthrough item "remote invocation over existing SSH". |
| Master ownership | IgH `ecrt_request_master()` reserves a master for exclusive use and fails with `-EBUSY` while another application holds it (`module.c`); the `ethercat` command-line tool only works in the master's idle phase, i.e. when no application has requested the master. | Engineering operations cannot run beside `stc run`/`stc serve`. "Managed master" therefore means "the PLC runtime is stopped", not merely "one CLI at a time". An externally owned IgH master **cannot** be inspected through ecrt, so §2.4's external-master mode must be narrowed to the idle-phase/read-only case or documented as unsupported. |
| SDO/PDO read-back | The shim has no `ecrt_master_get_sync_manager`, `ecrt_master_get_pdo`, `ecrt_master_get_pdo_entry`, `ecrt_master_sdo_upload/download`, or `ecrt_slave_config_sdo*`. Those userspace calls do exist in IgH 1.6 (`ecrt.h`, `#ifndef __KERNEL__`); SDO transfers block and are processed by the master state machine, so they may run on a commissioning thread but never on the cycle thread; pre-activation startup SDOs use `ecrt_slave_config_sdo*`. | ABI 2 is a substantial new C surface (identity, PDO tree read-back, SDO transfer, startup SDO queue), not an incremental symbol add. It must be prototyped before Phase 4/7 estimates mean anything. |
| Privileges | `tools/ethercat/stc-wl52.sudoers` grants only two fixed wrappers (`stc-wl52-service`, `stc-wl52-run`) and explicitly exposes no generic passthrough. | An engineering CLI that requests the master needs a new fixed privileged wrapper or an explicit documented run-as-root requirement. This is a security-reviewed change, not a packaging detail. |
| Unsafe lint | Workspace sets `unsafe_code = "forbid"`; only `crates/stc-runtime` overrides it with a documented exception (D-077). | All `dlopen`/FFI engineering code must live in `stc-runtime`, or a new crate needs its own documented exception and an amendment of D-077. |
| Profile parsing | `crates/stc-manifest/src/profile.rs` rejects unknown keys per table (`check_unknown`, `DEVICE_KEYS`, `CHANNEL_KEYS`, `SYNC_KEYS`, `PDO_KEYS`, `DC_KEYS`) and has no notion of device kind at parse time. | Every new engineering key needs parser, tests, and docs in one change; a separate configuration file needs a new `[io]` key through `model.rs`/`parse.rs`, schema tests, the manifest fuzz seed, and the CLI/manifest contract. |
| Compiler validation | The compiler validates profile structure and route endpoints today. A `pdo-channels` width against a declared channel width is checked at cold start as `STC-E-IO-ETH-007`, not at build time. | Phase 6's compiler claim is new compiler work with a new build-time diagnostic; the runtime code cannot be reused as a compiler code, and the conformance profile changes. |
| Metadata, hashing, online change | `stc-meta.json` embeds the canonical profile; the coordinator's `configuration_hash` covers profile, device tree, channels, bindings, and routes and drives `STC-E-ONLINE-IO-001`. | The selected mode, PDO selection, and startup SDO sequence must live inside the embedded profile for a startup change to force a cold download. A provenance hash must stay outside the effective hash, or an unrelated ESI edit forces a cold download. |
| Test infrastructure | The shim fake (`EcShim`) is private to `io_ethercat.rs` tests; the shim integration test file is `#![cfg(target_os = "linux")]`; non-Linux hosts use `io_mock::MockEthercatBackend`. | A structured engineering API needs a host-neutral mock at the API boundary, otherwise every engineering test is Linux-only and the coverage gate becomes host-dependent. |
| Repository gates | `cargo xtask traceability --check` requires each spec `STC-*` ID to have test evidence or an allowlist entry; `cargo xtask host-audit --check` rejects OS-specific lines outside `tools/ci/host-allowlist.txt` and fails on stale entries; `tools/ci/check_docs.py` requires 29 documents, validates every relative link, and rejects `TODO`/`TBD`/`FIXME`; `check_local.py`, clippy, fmt, the 85% coverage gate, and the fuzz jobs all run in CI. | Each phase must budget documentation, traceability, and allowlist work. A new crate additionally needs workspace membership, `Cargo.lock`, the xtask suite lists, and an `Architecture.md` crate-map row. |
| Static-profile migration | The DIEWU bench profile is hand-written and is the only regression baseline; there is no migration tooling or deprecation notice. | Phase 6 must keep both paths green and Phase 8 must define an equivalence check between the hand-written and the engineering-configured bench project. |

**Conclusion.** No architectural contradiction was found between the owner's direction (§2) and the
tree: the configuration-time split of §2.2 is *simpler* for the compiler than the first draft's
build-time ESI resolution, and it keeps the `DeviceBackend` contract intact. Three items are real
blockers that the plan now treats as explicit work rather than assumptions:

1. **Remote engineering execution does not exist** and must be built or explicitly deferred (§13
   Q20, Q21).
2. **Exclusive master ownership plus fixed-command privileges** means commissioning requires the
   runtime to be stopped and a reviewed privileged wrapper (§13 Q11, Q22, Q23).
3. **The shim cannot read identity, PDO assignment, or SDOs**, and the unsafe-code policy pins the
   FFI layer to `stc-runtime` (§13 Q12, Q19).

Everything else is ordinary implementation work with known touchpoints.

## 5. Engineering model and workflow

### 5.1 Device description and ESI repository

The ESI repository is a local engineering resource. It may contain multiple files, multiple device descriptions per file and multiple revisions. The subsystem must support explicit file selection and a vendor-library search path, with deterministic identity/revision selection and clear diagnostics.

ESI ingestion must produce a typed description/capability model, not a ready-to-run device configuration. Unsupported fields must be explicitly reported or rejected according to the documented subset; they must not silently influence configuration.

### 5.2 Topology model

A project must be able to represent a machine before hardware exists. It must be possible to define master(s), slave order/position, expected identity and logical association manually.

An online scan returns observed topology and identity information. Provide operations to:

- scan and list the bus;
- inspect an individual slave;
- compare observed topology with project topology;
- explicitly capture discovered devices/topology into a project configuration draft.

A scan must not silently rewrite the project or decide how a device is configured.

### 5.3 PDO selection and process-image mapping

An ESI may describe multiple PDOs and alternative mappings. A device may expose multiple operating modes, with different PDO requirements. The engineering subsystem must expose these alternatives and let the engineer select the intended configuration.

Respect ESI constraints such as `Fixed`, `Mandatory`, `PdoAssign` and `PdoConfig` where present and supported. Do not assume every advertised PDO is freely selectable. Validate the selected configuration against the device description and, during commissioning, against the live device's actual capabilities/state.

Keep these concepts separate:

1. selected operating mode / device configuration;
2. selected Sync Manager and PDO assignment/mapping;
3. PLC logical channels mapped to selected PDO entries.

The logical channel interface remains engineer-authored. Do not automatically turn every ESI entry into a PLC channel. A default view or suggested mapping may be offered, but applying it must be an explicit engineering action.

Runtime verification should compare more than total bit length. Where the master/slave API permits, verify direction, Sync Manager, PDO assignment/order, entry index/subindex/bit length/order and registered process-image offsets/lengths. A source ESI hash is provenance, not proof that the live device has the selected configuration.

### 5.4 Object dictionary and SDO commissioning

The engineering subsystem must provide a browsable object dictionary and live SDO operations.

Required user capabilities:

- list/browse objects and subindices, including names, types, access rights and descriptions when available;
- inspect a specific object/subindex;
- read the current value from a live device;
- write a value during commissioning when access rights and device state permit;
- display the result, including abort/error information and read-back verification where applicable;
- distinguish values observed on the device from values saved as desired project configuration.

ESI object-dictionary information may be incomplete, vendor-specific or reference external dictionary files. The implementation must not assume ESI is the sole source of object metadata. Where supported, live CoE SDO Information / object dictionary queries should supplement the ESI. Objects discovered online but absent from the ESI should remain inspectable, with their metadata provenance identified.

Do not treat all SDO writes as independent key/value assignments. Some devices require a sequence, a particular PREOP/SAFEOP state, disabling a feature before changing it, or a read-back/verification step. The model must support ordered startup operations, preconditions/state requirements, explicit failure handling and verification. The exact generic sequencing model is a Phase 0/early implementation design item.

At minimum, distinguish:

- **Read-only inspection:** never changes device state.
- **Commissioning write:** explicitly changes the currently connected device; requires managed-master ownership and a clear confirmation/diagnostic path.
- **Persist as startup configuration:** records an approved operation in the project so runtime can apply it on subsequent startup.

A commissioning write must not automatically become a persistent startup write. Conversely, saving a desired startup value must not be represented as proof that the live device currently has that value.

### 5.5 Desired configuration, observed state and diff

Provide a structured comparison between project intent and the connected device/master. At minimum, compare:

- master and slave topology / expected positions;
- vendor, product, revision and other available identity fields;
- selected operating mode where it can be observed;
- PDO assignment and mapping, including entry identity and widths where readable;
- selected DC configuration and observable state;
- configured startup SDO values versus live read-back values.

The comparison must distinguish `match`, `different`, `unknown/unreadable` and `not applicable`. Missing read-back support must not be reported as a successful match.

### 5.6 Project persistence and compiler/runtime boundary

Persist the engineer's desired configuration in project files. The exact file split/schema (existing machine profile versus a dedicated EtherCAT engineering configuration file) must be settled in Phase 0, with the following constraints:

- project configuration is the source of truth for desired settings;
- logical PLC devices/channels remain owned by `stc.toml` unless a deliberate schema change is approved;
- ESI file paths and source hashes are provenance/capability inputs, not substitutes for selected PDO/SDO configuration;
- selected mode, PDO mapping, startup SDO sequence and DC settings are explicit and deterministic;
- project-relative path resolution is confined to the project root, including symlink handling;
- builds do not depend on live hardware, vendor installations or an ESI directory being present after the configuration has been resolved/persisted;
- compiler validation checks internal consistency and channel mapping widths; it does not make engineering selections.

The artifact should contain the effective configuration required by runtime and enough identity/configuration provenance to diagnose what was built. A change that alters the effective process image or startup configuration must participate in compatibility/online-change checks; irrelevant changes to unused ESI documentation should not necessarily force a cold download. Define separate source/provenance and effective-configuration hashes.

## 6. API and CLI design

### 6.1 API requirements

The engineering API is the stable boundary between engineering logic and user interfaces. It need not be a network API or daemon. It may initially be an in-process Rust API/service layer used by the CLI.

Operations should be typed and return structured data. The API should cover, at minimum:

- ESI: list/search/import/inspect descriptions; resolve identity/revision; report capabilities and unsupported fields.
- Topology: define/edit project topology; scan bus; retrieve observed topology; inspect slave identity; compare/capture topology.
- PDO: retrieve capabilities; list valid configurations; select operating mode/PDOs; validate mapping; inspect live assignment.
- Object dictionary: list objects/subindices; inspect metadata; read/write SDO; decode/encode typed values; report abort codes.
- Configuration: show desired config; validate; diff against observed state; capture selected observed values; save changes to project.
- Lifecycle/ownership: acquire/release or otherwise enforce exclusive managed-master ownership; report master state; reject conflicting operations.
- Runtime: apply configured startup operations and verify configured identity/mapping before PLC cyclic execution.

Use typed errors and structured diagnostics. Include device position, object index/subindex, operation, expected/observed values and EtherCAT state where relevant. Do not expose only formatted text as the API result.

Long-running operations should expose status/progress in a way that a future TUI/GUI can consume. Read-only and mutating operations must be distinguishable in the API.

### 6.2 CLI requirements

The CLI is in scope and must expose the complete WL-7 engineering workflow. Exact command names are to be designed consistently with existing `stc` CLI conventions. The following are illustrative command families, not a frozen syntax:

```text
stc ethercat esi list
stc ethercat esi inspect <file-or-device>

stc ethercat topology show
stc ethercat topology scan
stc ethercat topology diff
stc ethercat topology capture

stc ethercat slave inspect <position>
stc ethercat slave pdos <position>
stc ethercat slave configure <position> --mode <mode>

stc ethercat sdo list <position>
stc ethercat sdo inspect <position> <index> [subindex]
stc ethercat sdo read <position> <index> [subindex]
stc ethercat sdo write <position> <index> [subindex] <value>
stc ethercat sdo save-startup <position> <index> [subindex] <value>

stc ethercat config show
stc ethercat config validate
stc ethercat config diff
stc ethercat config apply
stc ethercat config capture
```

The CLI must clearly distinguish project configuration operations from live hardware operations. For example, `sdo read` reads the connected device; a project-config inspection command shows the saved desired startup value. Mutating live hardware must be explicit and must obey managed-master ownership/state rules.

### 6.3 Remote use

Remote operation is provided by the existing SSH-based `stc` workflow. WL-7 must ensure the engineering CLI commands work through that existing path; it does not add a daemon, RPC protocol, remote transport or separate remote configuration subsystem. Engineering logic must be independent of the local/SSH invocation context.

**Gap to close (reviewed 2026-11-15).** The shipped `stc remote` supports only `status` against a
serve-session descriptor, and `stc serve` exposes only status/deploy/stop. There is no generic
remote command execution, so this requirement needs an explicit enabler in Phase 2 (Q20, Q21):
a `stc remote exec`-style action that reuses the existing SSH runner with project synchronization
and the production source fingerprint, or a documented decision to keep WL-7 engineering
local-only and drop the remote walkthrough item. A serve session is *not* a suitable transport
for commissioning: it owns a running runtime, and commissioning needs the exclusive master.

Also new CLI wiring, not an existing surface: `stc` dispatches a fixed `Command` enum
(`crates/stc-driver/src/cli.rs`), so the `stc ethercat` family is a new variant, parser,
dispatch, help table row, and CLI tests. `crates/stc-driver/src/cli.rs` is already in
`tools/ci/host-allowlist.txt`, so Linux-only gating stays inside an allowed file.

### 6.4 Future TUI and GUI compatibility

No TUI or GUI is implemented in WL-7. The API must nevertheless avoid CLI-only assumptions: no parsing terminal output as an integration interface, no command-handler-owned business logic, no opaque string-only results, and no requirement that future clients duplicate validation or device access logic.

## 7. ESI subset and untrusted-input policy

The original WL-7 subset remains the initial candidate, subject to Phase 0 review. It includes:

- `EtherCATInfo` version 1.2, vendor ID/name, device identity and revision;
- groups/types/names for diagnostic context;
- device sync managers, RxPDO/TxPDO definitions and entries, including `Fixed`, `Sm` and `Mandatory` where present;
- DC op modes and supported `AssignActivate`, cycle and shift fields;
- mailbox and CoE capability flags, including `SdoInfo`, `PdoAssign`, `PdoConfig` and `CompleteAccess`;
- multiple devices per file and multiple files/revisions.

The first subset need not fully model all `ObjectDictionary`, `DataType`, profile/dictionary references, vendor-specific extensions or images. However, the engineering subsystem must support online object-dictionary browsing where the device provides it; ESI's incomplete dictionary coverage must not block live SDO inspection.

Reject unsupported roots/versions and malformed or ambiguous input with stable diagnostics. Do not silently ignore fields that could affect the selected configuration. No DTD/DOCTYPE, entity expansion, external resource fetch or XInclude. Enforce file-size, nesting-depth, element/attribute-count and text-length bounds. Handle at least UTF-8 and ISO-8859-1 if confirmed by corpus measurement. Fuzz the parser and guarantee no panic/hang/unbounded allocation on arbitrary input.

The TwinCAT corpus measured in the original plan (458 files, 387 XML files, 144 declaring at least one `Device`, UTF-8 and ISO-8859-1, largest 70.6 MB) remains local development/manual input only. CI must not require it. Synthetic fixtures are committed; vendor samples are committed only after redistribution permission is confirmed. Use `STC_ESI_CORPUS` for an ignored/gated local corpus campaign and record acceptance/rejection counts and diagnostic codes.

## 8. Runtime configuration and safety boundaries

The runtime consumes the persisted desired configuration. It must:

1. establish exclusive managed-master ownership;
2. configure the master and slaves in the required EtherCAT states;
3. execute the ordered startup SDO operations at their declared state/preconditions;
4. apply the selected PDO assignment/mapping and DC configuration;
5. read back and verify identity and configuration to the extent supported by the master/device;
6. activate the master and enter OP only after required configuration and verification succeed;
7. start PLC cyclic execution only after successful initialization.

A failure in a required startup operation or required verification is fail-closed: do not enter normal cyclic execution with an unknown or mismatched process image. Diagnostics must identify the failed operation and device. Runtime must not dynamically decide to switch a drive mode or remap PDOs as an automatic recovery action.

The existing ABI 1 shim compatibility question remains to be resolved. If ABI 2 is introduced for richer identity/PDO/SDO access, define whether ABI 1 remains loadable with an explicit `verification unavailable` warning or whether the new engineering/runtime feature requires ABI 2. Do not claim verification that the available shim cannot perform.

**Externally managed masters (narrowed after review).** IgH reserves a master for exclusive use:
`ecrt_request_master()` fails with `-EBUSY` while another application holds it, and the `ethercat`
tool can only access a master in its idle phase (no application reservation). WL-7 therefore cannot
inspect, through ecrt, a master owned by another process. The supported scope is:

- an **idle** local master (nobody has requested it) may be inspected read-only, which is the same
  state a managed-master commissioning operation starts from;
- a master **reserved by another application** is reported as unavailable
  (`STC-E-IO-ETH-002`-style ownership conflict, new engineering code) and no read is attempted;
- a master owned by a non-IgH stack on another host is out of scope and must be documented as
  such rather than partially supported.

The API still separates inspection from mutating operations, so a future supported external
interface can be added without changing the operation model.

**SDO and read-back feasibility (IgH 1.6.13).** `ecrt_master_get_slave`, `get_sync_manager`,
`get_pdo`, and `get_pdo_entry` provide the connected-slave information interface in userspace;
SDO upload/download exist and block on the master state machine, so they run on a commissioning
thread and never on the cycle thread; pre-activation startup writes use `ecrt_slave_config_sdo*`
and `ecrt_slave_config_complete_sdo`. Phase 3 prototypes the exact idle/operation-phase behavior
of each call and records what is genuinely observable before Phase 4/7 depend on it.

No safety or hard-real-time claim is added. The HIL scope remains the DIEWU bench unless additional hardware is explicitly available and approved.

## 9. Diagnostics and observability

Retain existing stable diagnostic families where possible. Define dedicated ESI/configuration/SDO diagnostics where they materially improve actionability. Every new stable code must have a test that emits it and a traceability entry.

Diagnostics should distinguish at least:

- ESI missing/unreadable/outside project root;
- unsupported ESI document/version/field;
- malformed or resource-limit-exceeding ESI;
- ambiguous identity/revision selection;
- invalid or unsupported PDO/SM/mapping selection;
- logical channel-to-PDO mapping mismatch;
- live topology/identity mismatch;
- SDO object unavailable, access denied, invalid value, abort code, timeout or state/precondition failure;
- startup sequence failure and read-back mismatch;
- master ownership conflict or unsupported external-master operation;
- configuration differs from observed hardware;
- verification unavailable because the master/shim/device does not expose the required read-back.

`stc run -v` and existing status surfaces should report useful configuration provenance and runtime verification state without changing established layouts gratuitously.

## 10. Phases

Each phase follows `workflows/FEATURE-DEVELOPMENT.md` §4–§6: implementation handoff, local checks,
pushed commit, green CI and recorded acceptance. The owner approves Phase 0 and the pre-PR manual
walkthrough.

**Acceptance rule (workflow §4).** A phase whose gates are all automatable is accepted on a green
CI run plus its evidence, and the agent records the acceptance and continues. A phase containing
a hardware, interactive, or otherwise non-automatable gate needs owner acceptance before the next
phase starts; the owner may request manual acceptance of any phase. Never start a phase while the
previous phase's CI is running, and never write code while CI runs (workflow §5).

**Local checks before every push (workflow §5).** `python tools/ci/check_docs.py`,
`markdownlint-cli2`, `cargo xtask traceability --check`, `cargo xtask host-audit --check`,
`git diff --check`, plus the phase's own tests and clippy.

**Coverage (workflow §11).** New engineering code targets 95% line coverage; the project gate stays
85%. The engineering API, parser, and command paths are the coverage-sensitive parts, which is why
the host-neutral mock is a Phase 1 deliverable rather than a Phase 7 afterthought.

### Phase 0 — Architecture and configuration-model decisions

**Goal:** approve the engineering subsystem boundaries and resolve decisions that block implementation.

Work:

- Record the agreed principles in §2 as durable decisions.
- Define the project configuration model and file ownership: logical topology/channels, desired device config, ESI provenance, PDO selection, startup SDO sequence and DC configuration.
- Define managed-master ownership and the supported external-master inspection boundary.
- Define the API boundary and the CLI command surface; confirm existing SSH execution is sufficient for remote use.
- Define the generic representation of typed SDO values, ordered operations, state/preconditions, failure policy and verification.
- Decide parser crate versus project-owned reader; ESI versions; limits; file resolution and revision-pinning policy.
- Decide the first supported ESI subset and how ESI constraints (`Fixed`, `Mandatory`, `PdoAssign`, `PdoConfig`) affect selection.
- Decide how selected drive modes are represented without hard-coding vendor-specific motion semantics.
- Decide shim ABI evolution and which runtime read-back operations are feasible with IgH 1.6.13.
- Confirm corpus policy and sample ESI redistribution status.

**Exit criteria:** every question in §13 is answered; the answers are recorded as `D-259` and
following; the configuration model is reviewed; CLI/API responsibilities are clear; no
implementation starts against the old compile-time-ESI assumption.

**Evidence:** the §13 answer table, the decision entries, the owner approval recorded in §14, and
an updated `v3.0.0-plan.md` §4 target section if any Phase 0 answer changes the locked scope.

### Phase 1 — Engineering API foundation and ESI repository/catalog

**Goal:** provide structured engineering operations and safely inspect vendor device descriptions.

Work:

- Establish the typed API/service layer used by CLI handlers. Add structured results/errors and tests independent of terminal formatting.
- Implement bounded ESI reader and typed capability model (vendor, devices, identity/revision, syncs, PDOs/entries, DC modes, mailbox/CoE flags and supported dictionary metadata).
- Implement ESI library search, explicit source selection, identity/revision matching and deterministic diagnostics.
- Add synthetic fixtures, malformed cases and fuzz target.
- Add CLI commands for ESI list/search/inspect.

Add the crate/home and lint decision (Q19) to the repository plumbing in the same phase it is
approved: workspace membership, `Cargo.lock`, xtask suite lists, host allowlist, and the
`Architecture.md` crate map when a new crate is chosen.

**Exit criteria:** ESI inspection is deterministic; unsupported/ambiguous inputs produce stable
diagnostics; API tests do not depend on CLI text; parser fuzz smoke is green; the host-neutral
mock exists and the API tests run on every CI host.

**Evidence:** unit and fixture tests, fuzz smoke output, the crate/home record, and the API test
listing that proves no CLI-text coupling.

### Phase 2 — Topology authoring, bus discovery and master ownership

**Goal:** support offline machine definition and online discovery without conflating desired and observed topology.

Work:

- Define and validate a manually authored topology with master/slave order, expected identities and logical device associations.
- Implement managed-master ownership and conflict detection for commissioning/runtime operations.
- Implement online bus scan and structured observed-topology results.
- Implement slave identity inspection and desired-vs-observed topology comparison.
- Implement explicit capture/import from discovered topology into a project draft.
- Define and implement the inspection-only external-master path to the extent supported by the selected master integration mechanism.
- Add CLI commands for topology show/scan/diff/capture and slave inspect.

Close the remote-execution gap here rather than in Phase 9 (Q20/Q21): either implement the
generic engineering execution action over the existing SSH runner with project sync and
fingerprint, or record the owner decision that WL-7 engineering is local-only.

**Exit criteria:** a topology can be authored with no hardware; a live scan reports observed
facts; scan never silently rewrites project intent; conflicting managed-master ownership is
rejected and tested; the remote path is either implemented and tested or explicitly out of scope
with the §6.3 text corrected.

**Evidence:** topology/scan/mock tests, ownership-conflict tests, and either remote execution
tests or the recorded scope decision.

### Phase 3 — PDO capability inspection and explicit configuration selection

**Goal:** let the engineer choose a valid device configuration from ESI/live capabilities.

Work:

- Model supported PDOs, sync managers, entries and selection constraints.
- Expose alternative operating modes/configurations where described by ESI or device metadata.
- Validate PDO assignment and mapping, including direction/order/entry widths and mandatory/fixed constraints.
- Persist the selected operating mode and PDO configuration as desired project state.
- Keep logical PLC channels explicit; validate their mapping to selected entries without auto-exposing all entries.
- Add live inspection of actual PDO assignment/mapping where the master/device supports it.
- Prototype IgH 1.6.13 read-back and PREOP timing; document any information that cannot be reliably observed.

**Exit criteria:** engineer can inspect alternatives, select a configuration, validate it offline
and compare it with live device state; unsupported combinations fail with actionable diagnostics;
the IgH prototype record states which identity/PDO facts are observable and in which master phase.

**Evidence:** selection/validation tests, the read-back prototype record (calls, master phase,
observed fields), and fixtures for fixed/mandatory/alternative PDO sets.

### Phase 4 — Object dictionary and SDO commissioning

**Goal:** inspect live object dictionaries and read/write SDO values safely.

Work:

- Implement object/subindex listing and inspection using ESI metadata and online CoE SDO Information where supported.
- Represent unknown or online-only objects without assuming ESI completeness.
- Implement typed SDO reads/writes, conversion, access checks, abort/error decoding, timeout and state diagnostics.
- Distinguish read-only inspection and explicit live commissioning writes.
- Add CLI commands for object listing, object inspection, SDO read and SDO write.
- Test with mocks and available hardware; document unsupported device/master capabilities.

**Exit criteria:** live values can be browsed/read; supported writes are explicit and produce
structured results; errors and access/state restrictions are diagnosable; no commissioning write
silently changes project configuration; SDO execution is proven off the cycle thread.

**Evidence:** SDO codec/abort/state tests, mock and HIL read/write logs, and a note on which
objects are readable only via SDO Information and which remain ESI-only.

### Phase 5 — Startup SDO sequences and configuration persistence

**Goal:** persist desired device settings and make them reproducible at runtime.

Work:

- Define an ordered startup-operation model with typed values, state/preconditions, timeout, failure policy and optional read-back verification.
- Provide an explicit way to save a commissioning value as a startup setting; never persist a live write implicitly.
- Define project configuration schema and deterministic serialization/canonicalization.
- Implement desired-vs-observed configuration diff and explicit capture of selected observed values.
- Validate consistency between selected operating mode, startup SDO operations, PDO mapping and DC settings.
- Add CLI config show/validate/diff/capture/save operations.

**Exit criteria:** desired configuration is fully project-persisted and reviewable offline;
startup operations are ordered and validated; a live commissioning value is not confused with a
saved desired value; the configuration model from Q9/Q24 is implemented without unknown-key
regressions in `stc-manifest`.

**Evidence:** schema/canonicalization tests, round-trip persistence tests, profile-parser tests for
every new key, and the manifest fuzz seed update.

### Phase 6 — Compiler/artifact integration

**Goal:** compiler consumes the persisted configuration without performing engineering discovery or ESI resolution.

Work:

- Integrate the explicit desired configuration with existing manifest/profile validation and artifact embedding.
- Validate logical channel mappings against the selected effective PDO process image.
- Embed the effective runtime configuration and relevant identity/provenance data.
- Define source/provenance hash separately from effective-configuration hash.
- Ensure project builds are deterministic and do not require live hardware, TwinCAT or vendor ESI directories once the desired configuration is persisted.
- Preserve existing static-profile projects and add migration/compatibility diagnostics where needed.
- Ensure effective process-image/startup changes participate in existing online-change/cold-download compatibility checks.

Add the build-time channel/PDO width diagnostic required by the review (Q25): a new code in the
project/manifest family, its conformance-profile row, and traceability evidence; the runtime
`STC-E-IO-ETH-007` keeps its cold-start meaning.

**Exit criteria:** compiler validates but does not select; artifact contains all runtime-required
configuration; same inputs produce reproducible output; existing static-profile regression suite
remains green; the new build-time diagnostic has a failing fixture and a passing fixture.

**Evidence:** driver build tests (valid/invalid), metadata inspection, hash-split tests (Q26), and
updated CLI/manifest and conformance documents.

### Phase 7 — Runtime application and verification

**Goal:** runtime applies the saved desired configuration safely and verifies the resulting device state before PLC cycles begin.

Work:

- Extend shim/API as required for identity, PDO assignment/mapping and SDO operations/read-back.
- Enforce exclusive managed-master ownership.
- Apply startup SDO sequences in the correct state/order and verify configured values where requested.
- Configure PDOs and DC according to the persisted selection.
- Verify identity and process-image mapping before activation/OP; fail closed on required mismatch or failed startup operation.
- Preserve ABI compatibility according to the Phase 0 decision and report verification limitations honestly.
- Extend mock shim and runtime tests for success, mismatch, SDO abort, timeout, state failure, ownership conflict and unsupported read-back.

**Exit criteria:** runtime only starts PLC cycles after required configuration succeeds;
mismatch/failure paths do not run with an invalid process image; diagnostics show expected and
observed state; an ABI 1 shim reports verification *unavailable*, never *passed*; the privileged
wrapper decision (Q23) is implemented or documented with its limitation.

**Evidence:** shim ABI 2 tests, mock mismatch/abort/timeout/ownership tests, startup-ordering
tests, and the runtime section of the verification report.

### Phase 8 — Corpus, simulator, HIL and acceptance evidence

**Goal:** demonstrate the engineering workflow with synthetic, vendor-format and hardware evidence.

Work:

- Run the local TwinCAT corpus harness and record accept/reject counts by diagnostic and file-size band; investigate unexpected outcomes.
- Run a longer ESI fuzz campaign and record executions, coverage and artifacts.
- Test offline topology/configuration without hardware.
- Test online discovery, SDO browsing/read/write, configuration diff and apply using simulator/mock coverage and available hardware.
- Keep the existing DIEWU static-profile project as regression baseline; create a parallel ESI/configuration-driven project and compare I/O behavior.
- Run HIL on `ecat`; record host, OS, IgH version, shim ABI, device identity, ESI provenance, desired config, commands and results.
- Include deliberate negative cases: wrong identity/revision, PDO mismatch, failed startup SDO, and configuration mismatch where hardware permits.

**Exit criteria:** evidence covers offline and online engineering workflows; the hand-written and
engineering-configured bench projects produce equivalent I/O behavior; HIL results and unsupported
cases are documented; no safety or hard-real-time claim is added.

**Evidence:** corpus tally by code and size band, fuzz campaign output, simulator results,
equivalence comparison, HIL logs with host/OS/IgH/ABI/provenance, and the negative-case records.

### Phase 9 — Documentation, walkthrough and PR

**Goal:** publish the durable engineering contract and obtain owner acceptance.

Work:

- Update `I/O Routing and Backends.md`, `I/O and Devices.md`, `CLI and Manifest.md`, `Decisions.md`, `USER-GUIDE.md` and `KNOWN-LIMITATIONS.md`.
- Document the engineering API contract, CLI command reference, offline/online workflows, managed/external master behavior, SDO semantics, startup sequencing and ESI subset.
- Add `specification/Verification/ESI-Configuration.md` or a suitably renamed EtherCAT engineering verification document with corpus, fuzz, simulator and HIL evidence.
- Run an owner walkthrough covering: offline topology creation; ESI inspection; PDO/mode selection; live scan; SDO browse/read/write; explicit save of startup values; config diff/apply; runtime startup and mismatch handling; remote invocation over existing SSH.
- Open PR against `v3.0.0-planning` with summary, exclusions, verification evidence, CI links and known limitations.

**Exit criteria:** durable docs current (including the conformance profile, `Architecture.md` crate
map, `USER-GUIDE`, `KNOWN-LIMITATIONS`, and the traceability/host allowlists); the manual
walkthrough is approved; PR CI is green; the branch is not merged or deleted and WL-7 is not marked
complete by an agent without owner approval.

**Evidence:** documentation commits, the verification report, the walkthrough log, the PR link,
and the PR CI run.

## 11. Test strategy

Tests must cover the engineering model as well as the parser and runtime:

- ESI fixtures: valid, malformed, unsupported, ambiguous, multiple devices/files/revisions, ISO-8859-1, large bounded inputs, vendor extension noise and sidecar roots.
- Deterministic resolution independent of filesystem enumeration order.
- Offline topology and configuration editing with no hardware or ESI installation present after import/resolution.
- Bus scan and observed-state model using mock/simulator and HIL.
- PDO alternatives, fixed/mandatory constraints, multiple sync managers/PDOs, disabled SMs, non-byte-aligned widths and unsupported mappings.
- SDO typed values, subindices/records, read-only objects, access denial, abort codes, timeouts, state preconditions, ordered operations and read-back.
- Desired/observed diff semantics, including `unknown` when a value cannot be read.
- Master ownership conflicts and external-master inspection restrictions.
- CLI behavior through the same API, including invocation through the existing SSH workflow.
- Runtime startup ordering and fail-closed behavior.
- Regression of existing static DIEWU profile, status layout, online-change behavior and current CI matrix.

## 12. Dependencies and risks

- WL-5.2 provides the initial IgH backend and DIEWU bench, but its current shim may not expose all required discovery, SDO or PDO read-back operations. Prototype these capabilities early rather than assuming them.
- ESI is vendor-extended XML. The subset, constraints and rejection policy are a primary risk; the measured local corpus is the mitigation and evidence source.
- Object dictionaries are not guaranteed to be complete in ESI. Online CoE discovery and unknown-object handling are required for a useful SDO browser.
- SDO writes may depend on EtherCAT state, sequence and vendor-specific behavior. The generic startup-operation model must not pretend all writes are interchangeable.
- Some PDO selections may require SDO writes to assignment/mapping objects or vendor-specific mode selection. Configuration consistency between operating mode, SDO sequence and PDO map must be validated.
- A live master may be owned by another process. External inspection must use a supported interface and must not violate exclusive ownership; if the IgH deployment cannot support safe external inspection, document the exact limitation rather than attempting unsafe concurrent access.
- Remote operation should reuse existing SSH behavior. Avoid introducing a parallel remote-control mechanism.
- Shim ABI evolution is a compatibility surface. Old shim behavior must be explicit and tested; verification unavailable is not equivalent to verification passed.
- HIL currently covers only the DIEWU 16I/16O bench. Servo-drive mode-selection behavior should be tested with synthetic/mock fixtures unless a suitable drive is available; do not claim general vendor-drive support from DIEWU evidence.
- Any effective process-image or startup configuration change may require a cold download. Preserve existing online-change safeguards and produce a clear diagnostic.
- No safety or hard-real-time claim is made.

## 13. Phase 0 decisions to record

These are the decisions that must be answered before Phase 1 starts. The agreed architecture in §2
is not reopened by them. Answers become `D-259` and following in `specification/Decisions.md`.
Questions 19–28 come from the branch review in §4.1 and were not visible in the first draft.

Required answer set: **all 28 questions**. A question whose answer is "deferred" is still a
decision, and it changes the phase where the deferred work appears.

1. **Parser:** project-owned bounded reader or vetted crate such as `quick-xml`? Proposed: evaluate `quick-xml` first; retain the same limits, fuzzing and no-external-resource policy either way.
2. **ESI versions:** accept only `EtherCATInfo Version="1.2"`, or include 1.0/1.1? Proposed: 1.2 initially with explicit diagnostics for other versions.
3. **ESI source/library model:** per-device explicit source plus project-wide default library; exact TOML keys and package-relative path behavior to be specified.
4. **Revision policy:** exact revision pin required for production or highest revision as a convenience default? Proposed: support exact pin and make unpinned selection deterministic and conspicuous; decide whether production validation requires a pin.
5. **PDO constraints:** exact handling of `Fixed`, `Mandatory`, `PdoAssign` and `PdoConfig`, including what can be selected offline versus confirmed online.
6. **Operating-mode representation:** generic named mode plus its selected PDO configuration and required startup operations; avoid vendor-specific motion semantics in the generic schema.
7. **Object dictionary sources:** exact precedence/merge rules between ESI metadata, online SDO Information and unknown online objects.
8. **SDO startup sequence schema:** operation types, value encoding, EtherCAT state preconditions, ordering, retries/timeouts, abort handling and read-back policy.
9. **Project file layout:** extend machine profile or introduce a dedicated EtherCAT engineering configuration file; define canonical ownership and migration from static profiles.
10. **Topology identity:** required and optional identity fields, handling of serial number/alias/position, and behavior when offline schematics lack a serial number.
11. **External master inspection:** supported integration path and exact read-only capabilities. Confirm what IgH exposes safely when another process owns the master.
12. **Shim ABI:** ABI 2 scope, ABI 1 compatibility policy, and proof of which identity/PDO/SDO read-back calls are available at the required PREOP/SAFEOP stages.
13. **ESI resource limits/encoding:** default file-size/depth/element limits and UTF-8/ISO-8859-1 handling based on corpus evidence. Original proposal: 128 MiB cap to include the measured 70.6 MB file, plus structural limits.
14. **Channel exposure:** one channel per selected PDO entry by default; bit-level slicing only when explicitly declared. Confirm naming and duplicate-name behavior.
15. **DC policy:** how ESI DC op modes are presented and selected; do not implicitly choose the first ESI op mode. Persist the engineer's explicit choice.
16. **Mismatch policy:** fail closed for required identity, PDO and startup configuration mismatches; define which unavailable read-backs are warnings versus blockers.
17. **Diagnostics:** stable ESI, topology, PDO, SDO, ownership and runtime-configuration diagnostic families.
18. **Corpus/license:** confirm local-only TwinCAT corpus policy and whether the ASIX AX58100 sample ESI can be redistributed.
19. **Code placement and the unsafe policy:** keep all shim FFI in `stc-runtime` and put the
    engineering API there or in `stc-driver`, or add a new `stc-ecat` crate with its own
    documented `unsafe_code` exception? *Proposed: engine and FFI in `stc-runtime` (the only crate
    allowed `unsafe` today, D-077) and the CLI in `stc-driver`; no new crate in WL-7, because a
    new crate adds packaging, xtask, host-audit, and doc-map work without a boundary benefit.*
20. **Remote engineering execution:** add a generic engineering execution action to `stc remote`
    over the existing SSH runner, or declare WL-7 engineering local-only?
    *Proposed: add `stc remote exec`-style execution with project sync and fingerprint reuse, and
    remove the remote item from this target if the owner prefers to defer it. The serve-session
    descriptor is not used for engineering.*
21. **Project sync for remote engineering:** reuse the WL-1.2 SFTP synchronization and
    `SHA-256`/production-fingerprint checks, or require the operator to place the project on the
    host? *Proposed: reuse the delivered mechanism; a commissioning run must never operate on a
    different project revision than the one the operator selected.*
22. **Ownership arbitration against the runtime:** when a commissioning operation finds the
    master already reserved, fail with a diagnostic that names the owning session, or attempt a
    documented stop-then-commission sequence? *Proposed: fail closed and name the conflict; an
    automatic stop of a running PLC is an operator decision, not a convenience.*
23. **Privileged master access:** add fixed privileged wrappers for the engineering operations
    (extending `tools/ethercat/stc-wl52.sudoers` under review), or require the engineering CLI to
    run as the service/root user? *Proposed: new fixed-command wrappers with argument validation
    and no generic passthrough, reviewed as a security change; document the alternative.*
24. **Configuration persistence surface:** extend the existing machine profile, or add a dedicated
    engineering configuration file declared by a new `[io]` key? *Proposed: a dedicated file for
    the desired configuration with the effective process-image selection embedded into the profile
    the compiler already canonicalizes; record how the two are cross-checked and how a static
    profile migrates.*
25. **Build-time channel width validation:** which diagnostic family owns it, and does the
    conformance profile gain a rule? *Proposed: a new project/manifest-level code (not the runtime
    `STC-E-IO-ETH-007`), one conformance-profile row, and a failing fixture.*
26. **Hash split:** which inputs belong to the effective-configuration hash that forces a cold
    download, and which are provenance only? *Proposed: mode selection, PDO assignment/mapping,
    startup operations, and DC values are effective; ESI file path, source hash, and unused
    description text are provenance and must not force a cold download.*
27. **Documentation and gate set:** which documents and registries change, and in which phase?
    *Proposed: Phase 6/9 update `CLI-and-Manifest`, `IO-Routing-and-Backends`, `IO-and-Devices`,
    `01-Conformance-Profile`, `Architecture` crate map, `USER-GUIDE`, `KNOWN-LIMITATIONS`, the
    verification report, plus traceability and host-audit allowlists. The Lean verification
    registry (`lean/verification-status.toml`) is not touched because no theorem changes.*
28. **Target-set bookkeeping:** does the engineering-subsystem scope keep WL-7 at its current size
    class and wording in `v3.0.0-plan.md`? *Proposed: keep the target identity and commitment,
    restate the outcome and size (L/XL), and record the 2026-11-15 owner re-plan in the change log
    and the wishlist. This is applied in the same change as this plan revision.*

## 14. Acceptance log

| Phase | Accepted | Commit | CI run | Notes |
| --- | --- | --- | --- | --- |
| 0 | | | | Revised architecture reviewed against the branch on 2026-11-15 and aligned to the workflow; §13 has 28 questions and awaits owner answers and plan approval |

## 15. Change log

| Date | Change |
| --- | --- |
| 2026-11-15 | Revised from ESI-to-static-profile feature into an EtherCAT engineering subsystem. ESI is a capability description; desired configuration is engineer-selected and persisted at configuration time. Added offline/manual topology, online discovery, PDO/mode selection, live object-dictionary/SDO commissioning, ordered startup SDO operations, desired-vs-observed comparison, API-first design, CLI scope, existing SSH remote workflow, managed/external master distinction, and future TUI/GUI compatibility. |
| 2026-11-15 | Branch review and workflow alignment. Added §4.1 with verified findings (no generic remote execution; exclusive IgH reservation and idle-phase-only tooling; shim has no identity/PDO/SDO read-back; fixed-command sudoers; `unsafe_code` limited to `stc-runtime`; strict profile keys; no build-time width validation; embedded-profile hashing; Linux-only shim mocks; doc/traceability/host-audit gates; no static-profile migration). Narrowed the external-master mode to idle-phase read-only with an unsupported case, added SDO/read-back feasibility, added per-phase evidence, the workflow acceptance rule, local-check and coverage expectations, the remote-exec enabler in Phase 2, the build-time diagnostic in Phase 6, ownership/privilege work in Phase 7, and expanded Phase 0 from 18 to 28 required decisions. `v3.0.0-plan.md` §1/§3/§4/§8/§9 and `FutureWishlist.md` WL-7 were aligned to the re-planned scope. |

## 16. Phase checklist

Use this for every phase ([Feature Development Workflow](workflows/FEATURE-DEVELOPMENT.md) §4–§6):

- [ ] The phase goal, work, exit criteria, and evidence are in this plan.
- [ ] The subagent has the plan, the scope, the constraints, and the definition of done.
- [ ] The change is implemented and the relevant tests are added or updated (new code targets 95%
      line coverage).
- [ ] Local checks pass: `check_docs.py`, `markdownlint-cli2`, `cargo xtask traceability --check`,
      `cargo xtask host-audit --check`, `git diff --check`, plus clippy, fmt, and the phase tests.
- [ ] The phase commit is pushed and CI is green over the changed surface.
- [ ] The plan status is updated with the acceptance date, commit hash, and CI link.
- [ ] No new code was written while CI was running.
- [ ] Owner acceptance is obtained when the phase has hardware, interactive, or other
      non-automatable gates.
- [ ] Every owner question is answered and recorded as a decision before the next phase starts.
- [ ] No failing or flaky CI job is dismissed as unrelated; a rerun may separate a flake from a
      deterministic failure, and either result is reported to the owner.
- [ ] The working tree is clean before handoff.
