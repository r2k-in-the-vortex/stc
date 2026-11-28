# WL-7 Plan — EtherCAT engineering and device configuration

**Status:** Phase 0 approved by the owner on 2026-11-23. Revised proposal incorporating the owner's
architectural direction, reviewed against the branch on 2026-11-15 and aligned to
[Feature Development Workflow](workflows/FEATURE-DEVELOPMENT.md). Phases 1–3 are accepted on
green CI runs (§14); Phase 3 was accepted on 2026-11-27 with the recorded capability limit
(live PDO read-back and device-refused remap unavailable on the mailbox-less DIEWU, so effective
configuration stays `unverified`). Phase 4 is implemented (§10, Phase 4 record) and awaits CI
and owner acceptance of the same mailbox-less limitation before Phase 5 starts.

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

WL-7 provides a complete CLI engineering workflow and a structured API beneath it. Engineering is
invocation-context independent: as with the debug session, `stc` treats a remote host exactly like
a local one. The remote/UI packaging is a later concern (D-259); no new network service or remote
protocol is required.

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

### 2.4 Master ownership

Support one managed-master model plus optional read-only inspection:

- **Managed master:** `stc` owns the EtherCAT master exclusively for commissioning/configuration operations and for normal runtime. Only one process may own operations that change master/slave state.
- **Setup mode is the managed-master commissioning state.** Engineering and SDO operations run in an
explicit setup mode in which the PLC runtime is *not started*: the engineering session holds the
master, runs no cycle, and drives no outputs. Setup mode is entered only when no runtime owns the
master and is left explicitly; a reservation conflict fails closed (D-260, §5.7).
- **Invocation context is not part of the design.** Engineering commands behave identically on the
target host whether they are typed locally or reached over an SSH session, exactly as the debug
session does not care how the developer reached the host. WL-7 adds no remote transport, daemon,
or serve-session coupling; the remote *UX* (VS Code, GUI) arrives after WL-7 (D-259, §6.3).
- **Idle-phase inspection:** when no application owns the master, `stc` may read it without taking ownership. This is inspection only. There is no externally managed master mode: a master owned by another application and a non-IgH master are out of scope. Any hardware-changing operation, including an SDO write, requires setup mode and managed ownership.

Do not build a new network service or remote transport. Engineering operations must not depend on
whether the CLI is local or invoked through SSH, and the remote *UX* is deferred past WL-7
(D-259).

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
- Manual offline topology authoring with dense positions and exact vendor/product/revision identity, and online bus discovery (D-277).
- Comparison of desired topology/configuration with observed bus/device state.
- Device inspection and identification, including identity and revision.
- PDO capability inspection and explicit engineer selection of operating mode, PDO assignment and mapping, including declared-entry remapping within the ESI-declared PDO/entry set (D-269).
- Object dictionary browsing and typed SDO read/write operations where supported by the device and current EtherCAT state.
- Distinction between temporary commissioning writes and persisted startup SDO operations.
- Ordered generic startup SDO operations, including state/precondition handling and read-back verification where applicable, plus a declarative PDO configuration whose effective result is inspectable and verifiable. The two are distinct authoring abstractions: PDO configuration is applied through the master's native PDO path, and generic SDOs through the master's startup SDO mechanism (D-274).
- Persisting the desired machine configuration in project-owned configuration files.
- Mapping selected PDO entries to explicitly declared PLC channels; do not automatically expose every ESI entry as a PLC channel.
- API-first engineering operations and a CLI that exposes the complete WL-7 engineering workflow.
- CLI use on the target host with invocation-context independence; remote/UI packaging is
  deferred (D-259).
- Exclusive managed-master ownership, with idle-phase read-only inspection only; no externally managed master mode.
- Runtime application and verification of the persisted desired configuration, with per-device isolation and PLC cycles starting after master activation (D-278).
- Synthetic fixtures, malformed-input/fuzz tests, vendor-corpus testing, simulator tests and hardware-in-the-loop (HIL) evidence.

### 3.2 Out of scope

- TUI and GUI implementation. Their future compatibility is an architectural requirement, not a WL-7 deliverable.
- A new network daemon, RPC protocol, remote transport, or remote engineering UX; those follow
  WL-7 (D-259).
- Full ESI/XSD coverage or complete modelling of every vendor extension and object-dictionary construct.
- Arbitrary object-dictionary PDO mapping: mapping an object that the accepted ESI does not declare in a PDO, synthesizing PDOs, or editing entry index/subindex/bit length. Q5-B limits edits to ESI-declared entries; the accepted option C is deferred indefinitely and has no wishlist entry (D-269).
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
- `crates/stc-runtime/src/io_ethercat.rs` configures slaves through the project-owned shim `tools/ethercat/stc_ec_shim.{h,c}`, using IgH 1.6.13, `dlopen`, and ABI `stc-io-ethercat/1`; WL-7 replaces ABI 1 with ABI 2 as the only supported shim ABI (D-279).
- The current ABI 1 shim (525 lines) exposes lifecycle, configure, register, status, and cycle calls
  only. It checks vendor/product through `ecrt_master_slave_config()` and, when
  `check_revision` is set and the alias is zero, reads `revision_number` once through
  `ecrt_master_get_slave()`. It returns no identity fields to the backend, never reads the PDO
  assignment, and has no SDO call of any kind. ABI 2 must carry forward the
  lifecycle/configuration/registration/status/cycle surface WL-7 still needs and add the identity,
  PDO, SDO, DC, domain and structured-result surface; ABI 1 is retired with no dual loader or
  fallback (D-279).
- Existing stable diagnostics include `STC-E-IO-ETH-001`–`008` and `STC-W-IO-ETH-003`; status layout and `stc run -v` keys are established contracts.
- Backend tests use a mock shim on non-Linux CI hosts; driver/CLI fixtures live under `crates/stc-driver/tests`; fuzz targets live under `fuzz`; HIL uses the Linux `ecat` host and DIEWU 16I/16O bench per `specification/Verification/EtherCAT-Runbook.md`.
- No XML dependency currently exists in the workspace; workspace lints forbid `unsafe_code`. Phase 0 selects a vetted parser crate, with `quick-xml` as the candidate to evaluate against the local ESI corpus before the selection is locked (D-261, Q1).

The old static profile path remains a valid starting point and regression baseline. WL-7 should not break existing projects merely because the engineering subsystem is added.

### 4.1 Branch review (2026-11-15): what the tree actually provides

Every row below was checked in the working tree, not assumed. The consequence column is why the
matching Phase 0 decision exists.

| Area | Observed state | Consequence for this plan |
| --- | --- | --- |
| Remote execution | `stc remote` parses only the `status` action (`crates/stc-driver/src/cli.rs:938`) and dispatches to `remote_launch::status_command`; `stc serve` offers status/deploy/stop over the `stc-remote-session.json` descriptor; `remote.rs` owns an SSH/SFTP runner but no generic command execution. | Not a WL-7 blocker: the owner decided (D-259) that remote *UX* is post-WL-7 and engineering is invocation-context independent, matching the debug session. No remote enabler is built; the CLI must simply not assume a local TTY or developer-host working directory. |
| Master ownership | IgH `ecrt_request_master()` reserves a master for exclusive use and fails with `-EBUSY` while another application holds it (`module.c`); the `ethercat` command-line tool only works in the master's idle phase, i.e. when no application has requested the master. | Engineering operations cannot run beside `stc run`/`stc serve`. "Managed master" therefore means "the PLC runtime is stopped", not merely "one CLI at a time". An IgH master owned by another application **cannot** be inspected through ecrt. WL-7 therefore has no external-owner mode: only an idle master (no owner) may be inspected read-only, and non-IgH masters are out of scope. Any write, including SDO, requires setup mode. |
| SDO/PDO read-back | The shim has no `ecrt_master_get_sync_manager`, `ecrt_master_get_pdo`, `ecrt_master_get_pdo_entry`, `ecrt_master_sdo_upload/download`, or `ecrt_slave_config_sdo*`. Those userspace calls do exist in IgH 1.6 (`ecrt.h`, `#ifndef __KERNEL__`); SDO transfers block and are processed by the master state machine, so they may run on a commissioning thread but never on the cycle thread; pre-activation startup SDOs use `ecrt_slave_config_sdo*`. | ABI 2 is a substantial new C surface (identity, PDO tree read-back, SDO transfer, startup SDO queue, DC read-back, per-device domains, structured results), not an incremental symbol add. It replaces ABI 1 as the sole supported ABI, so the existing lifecycle/cycle/status surface must be carried forward rather than kept as a second loader; it must be prototyped before Phase 4/7 estimates mean anything (D-279). |
| Privileges | `tools/ethercat/stc-wl52.sudoers` grants only two fixed wrappers (`stc-wl52-service`, `stc-wl52-run`) and explicitly exposes no generic passthrough. | An engineering CLI that requests the master needs a new fixed privileged wrapper or an explicit documented run-as-root requirement. This is a security-reviewed change, not a packaging detail. |
| Unsafe lint | Workspace sets `unsafe_code = "forbid"`; only `crates/stc-runtime` overrides it with a documented exception (D-077). | All `dlopen`/FFI engineering code must live in `stc-runtime`, or a new crate needs its own documented exception and an amendment of D-077. |
| Profile parsing | `crates/stc-manifest/src/profile.rs` rejects unknown keys per table (`check_unknown`, `DEVICE_KEYS`, `CHANNEL_KEYS`, `SYNC_KEYS`, `PDO_KEYS`, `DC_KEYS`) and has no notion of device kind at parse time. | Every new engineering key needs parser, tests, and docs in one change; a separate configuration file needs a new `[io]` key through `model.rs`/`parse.rs`, schema tests, the manifest fuzz seed, and the CLI/manifest contract. |
| Compiler validation | The compiler validates profile structure and route endpoints today. A `pdo-channels` width against a declared channel width is checked at cold start as `STC-E-IO-ETH-007`, not at build time. | Phase 6's compiler claim is new compiler work with a new build-time diagnostic in the project/manifest family (D-266, Q25); the runtime code cannot be reused as a compiler code, and the conformance profile changes. |
| Metadata, hashing, online change | `stc-meta.json` embeds the canonical profile; the coordinator's `configuration_hash` covers profile, device tree, channels, bindings, and routes and drives `STC-E-ONLINE-IO-001`. | The selected mode, PDO selection, and startup SDO sequence must live inside the embedded profile for a startup change to force a cold download. The owner decision is that any EtherCAT configuration change forces a cold download; every persisted configuration input, including the ESI source reference and identity, participates in the hash (D-267, Q26). |
| Test infrastructure | The shim fake (`EcShim`) is private to `io_ethercat.rs` tests; the shim integration test file is `#![cfg(target_os = "linux")]`; non-Linux hosts use `io_mock::MockEthercatBackend`. | A structured engineering API needs a host-neutral mock at the API boundary, otherwise every engineering test is Linux-only and the coverage gate becomes host-dependent. |
| Repository gates | `cargo xtask traceability --check` requires each spec `STC-*` ID to have test evidence or an allowlist entry; `cargo xtask host-audit --check` rejects OS-specific lines outside `tools/ci/host-allowlist.txt` and fails on stale entries; `tools/ci/check_docs.py` requires 29 documents, validates every relative link, and rejects `TODO`/`TBD`/`FIXME`; `check_local.py`, clippy, fmt, the 85% coverage gate, and the fuzz jobs all run in CI. | Each phase must budget documentation, traceability, and allowlist work. A new crate additionally needs workspace membership, `Cargo.lock`, the xtask suite lists, and an `Architecture.md` crate-map row. |
| Static-profile migration | The DIEWU bench profile is hand-written and is the only regression baseline; there is no migration tooling or deprecation notice. | Phase 6 must keep both paths green and Phase 8 must define an equivalence check between the hand-written and the engineering-configured bench project. |

**Conclusion.** No architectural contradiction was found between the owner's direction (§2) and the
tree: the configuration-time split of §2.2 is *simpler* for the compiler than the first draft's
build-time ESI resolution, and it keeps the `DeviceBackend` contract intact. Three items are real
blockers that the plan now treats as explicit work rather than assumptions:

1. ~~Remote engineering execution does not exist.~~ Resolved by decision D-259: engineering is
   invocation-context independent and remote UX is deferred past WL-7, so no work item is created.
2. **Exclusive master ownership plus fixed-command privileges** means commissioning requires the
   runtime to be stopped and a reviewed privileged wrapper (§13 Q11, Q22, Q23).
3. **The shim cannot read identity, PDO assignment, or SDOs**, and the unsafe-code policy pins the
   FFI layer to `stc-runtime` (D-279, §13 Q19).

Everything else is ordinary implementation work with known touchpoints.

## 5. Engineering model and workflow

### 5.1 Device description and ESI repository

The ESI repository is a local engineering resource. It may contain multiple files, multiple device descriptions per file and multiple revisions. The subsystem must support explicit file selection and a vendor-library search path, with deterministic identity/revision selection and clear diagnostics.

ESI is an **import-time input**. Importing a description creates or updates the project TOML
configuration; once written, those project files are authoritative. Later ESI file edits do not
change the project, builds do not read ESI files, and the runtime does not reread project ESI paths.
The project persists the ESI source reference and identity for explicit re-import; the ESI file
itself is no longer required for build or runtime (D-263, Q4). The recorded identity is the accepted
device description's vendor/product/revision plus a content digest of that description. The source
reference and identity are persisted EtherCAT configuration inputs: any change to either, including
a path/link change or a re-import that changes the identity, forces a cold download (D-267, Q26).

ESI ingestion must produce a typed description/capability model, not a ready-to-run device configuration. Unsupported fields must be explicitly reported or rejected according to the documented subset; they must not silently influence configuration.

### 5.2 Topology model

A project must be able to represent a machine before hardware exists. It must be possible to define master(s), slave order/position, expected identity and logical association manually (D-277, Q10).

Each configured device requires a unique, dense machine slot: a `position` per master plus the exact vendor ID, product code and revision number. These fields are the identity basis for matching the configured device to a live slave. Revision ranges, product-family wildcards and "any revision" matching are not accepted. The persisted ESI source reference and identity remain a separate configuration input (D-263, D-267).

Serial number and station alias are optional instance checks. An offline schematic normally does not know them; if the engineer declares either, it becomes a required runtime check, not a hint. An absent optional field means "no instance check", never "match anything". Position is the machine slot: a serial or alias never substitutes for position, never causes automatic rebinding and never lets a device move to another slot. A declared serial or alias mismatch blocks that device even when the slot contains the right product.

Matching uses the master's SII slave identity (vendor/product/revision/serial/alias) as the baseline. CoE identity object `0x1018` is corroboration when readable: agreement is a match; a readable disagreement with SII or the project blocks that device; unavailability is an `unverified` warning when SII already established the required fields. Identity is checked before any SDO, PDO or DC configuration is queued for that device, so a mismatched device's configuration data is not written (D-278).

An online scan returns observed topology and identity information. Provide operations to:

- scan and list the bus;
- inspect an individual slave;
- compare observed topology with project topology per configured position;
- explicitly capture discovered devices/topology into a project configuration draft.

A scan must not silently rewrite the project or decide how a device is configured. Default capture records vendor/product/revision and does not pin serial numbers, so a spare device remains usable in the same slot; serial and alias are captured only on explicit request. A revision or ESI source-reference change invalidates the mapping basis and requires explicit re-import/review; it also forces a cold download (D-267).

### 5.3 PDO selection and process-image mapping

An ESI may describe multiple PDOs and alternative mappings. A device may expose multiple operating modes, with different PDO requirements. The engineering subsystem must expose these alternatives and let the engineer select the intended configuration.

Selection is **constrained selection plus declared-entry remapping** (D-269, Q5). The engineer selects declared PDOs and sync managers and may remap a selected PDO only from entries the accepted ESI declares for that device and direction: optional declared entries may be omitted, included entries reordered, and a declared entry may be placed in another declared PDO of the same direction. Entry index/subindex/bit length come from the ESI and are not editable; PDO indices are not synthesized; objects the ESI does not declare in a PDO are not mappable. This is not a general object-dictionary mapping editor.

Respect the declared constraints exactly: `Fixed` PDOs/entries cannot be reassigned, omitted, reordered or moved; `Mandatory` PDOs/entries must remain included; `Exclude`/`ExcludedSm` combinations are hard errors; and `PdoAssign`/`PdoConfig` gate whether a remap is possible. Missing `Fixed`/`Mandatory` flags take the schema default (false), a PDO without an explicit SM declaration is not assigned by inference, and an unsupported constraint form is rejected rather than ignored. ESI constraints and live capabilities combine as the most restrictive union: live evidence may narrow a selection but never widen it beyond the ESI. Offline selection is validated against the ESI; online commissioning additionally checks identity/revision, SII/CoE flags and the live assignment/mapping, and a live refusal is a fail-closed diagnostic for that device (D-278).

Persist the effective selection together with the declared-entry remap and the constraint facts needed to revalidate it without the ESI. The compiler validates those persisted facts; any change follows the cold-download rule (D-267, Q26). The persisted ESI source reference and identity establish which description the selection was based on; they are not proof that the live device has the selected configuration.

PDO configuration is a declarative, ordered description of sync managers, PDO assignments and mappings, not a user-authored SDO list. At the protocol level the slave applies those objects through CoE SDO writes (or SII/EEPROM on devices that use it), but the runtime applies the canonical configuration through IgH's native `ecrt_slave_config_pdos()` and process-data registration path. The engineering subsystem may generate an inspectable, non-editable representation of the equivalent CoE operations for diagnostics, but it does not claim that IgH executes an arbitrary user-defined PDO SDO sequence. Generic startup SDOs are a separate authoring abstraction and use the master's supported startup SDO mechanism; ordering/dependencies between the two are modeled where supported, without assuming arbitrary interleaving (D-274, Q8). The effective PDO configuration must be inspectable and verifiable against the slave's observed assignment/mapping.

Keep these concepts separate:

1. selected operating mode / device configuration;
2. selected Sync Manager and PDO assignment/mapping, including any declared-entry remap;
3. PLC logical channels mapped to selected PDO entries.

The logical channel interface remains engineer-authored. Do not automatically turn every ESI entry into a PLC channel. A default view or suggested mapping may be offered, but applying it must be an explicit engineering action. Channels bind to the effective entries after remapping, not to the ESI default set, and removing an entry that a channel depends on is a configuration error.

Only selected effective entries are channel candidates; unselected ESI entries and online-only objects are never exposed (D-273, Q14). The authoring default is one channel per selected entry at bit 0 with the full entry width, but applying it is an explicit engineering action, and an entry used only for process-image layout may remain unchanneled. An entry may instead be exposed as explicit, non-overlapping bit slices: a slice declares its start bit and uses the declared channel width, `bit + width` must fit the entry, the direction must match, and any slice wider than one bit must start on a byte boundary; one-bit slices may start at any bit. A whole-entry channel and a slice of the same entry overlap and are rejected. Channel names are engineer-authored, unique per device and matched exactly; ESI names are suggestions only, with a deterministic index/subindex fallback, and duplicates are hard errors rather than silent renames. Bindings target the effective entry after remapping, never display order.

Runtime verification should compare more than total bit length. Where the master/slave API permits, verify direction, Sync Manager, PDO assignment/order, entry index/subindex/bit length/order and registered process-image offsets/lengths. A remap that cannot be read back cannot be reported as verified.

#### 5.3.1 Operating-mode representation

A mode is a generic, engineer-authored, named bundle for one device, not an ESI mode pointer and not
a vendor motion profile (D-275, Q6). A mode consists of:

- a stable mode name, unique per device, plus an optional description;
- its canonical PDO configuration: sync managers, selected declared PDOs and entries, and the
  effective declared-entry remap (D-269, D-274);
- its effective channel exposure (`pdo-channels`), mapping the logical channel names declared in
  `stc.toml` to effective entries and bit offsets (D-273);
- its ordered, required generic startup SDO plan, including any vendor mode-selection SDO authored as
  ordinary startup data (D-274);
- its explicit DC selection (D-276, Q15);
- optional ESI DC op-mode or vendor-documentation provenance, recorded for review only.

Exactly one mode is selected per device at a time. The project may store alternate modes for review
and switching, but selection is always explicit: there is no "first ESI DC op mode" default and no mode
inferred from observed device state. Switching is an explicit engineering action; it changes the
effective configuration and follows the cold-download rule (D-267). Mode identity is the
engineer-authored name, not the ESI DC op mode; two modes that share PDOs but differ in remap, channel
exposure, startup operations or DC are distinct. The name stays stable across content edits, while
the content participates in the effective-configuration hash.

The generic schema carries no vendor-specific motion semantics: no drive-mode enumerations, units or
motion-profile fields. Vendor meaning lives in the mode name/description and in the startup values the
engineer authors; the engineering subsystem never interprets them. The PDO/sync-manager set is
validated under D-269; optional provenance is descriptive and never constrains or widens it. The
startup plan must be consistent with the selected PDO configuration under the master ordering limits
(D-274), and the DC selection must be one of the device's ESI-declared DC op modes or an explicit
free-run choice (D-276, Q15). Channel exposure is per mode: switching to a mode that leaves a bound
channel unexposed is a configuration error.

The desired engineering configuration file is the authoring source for all stored modes. The
compiler-facing machine profile carries the selected mode's effective projection — PDO tree, channel
exposure, startup plan and DC — and is cross-checked against the engineering file; the projection is
not a second authoring source (D-263, Q24). Exact key spellings and the projection/cross-check
mechanics are Phase 1/5 schema work. A machine-wide preset that selects one mode across several
devices is a separate convenience, not part of the mode bundle.

#### 5.3.2 Distributed-clock policy

ESI `<Dc><OpMode>` entries describe DC synchronization alternatives, not drive operating modes.
Every mode records exactly one explicit DC choice: one ESI-declared DC op mode or `free-run`
(D-276, Q15). There is no implicit first-op-mode default and no inference from live state.

The engineering tool presents every declared op mode with its name, description, `AssignActivate`,
Sync0/Sync1 cycle and shift, and any `Factor` resolved against the configured runtime cycle. The
selected op mode's `AssignActivate` is never edited or synthesized. Cycle and shift values are
derived from the declared op mode and persisted as effective values; an unresolvable factor or an
invalid Sync0/Sync1 combination fails closed. WL-7 does not allow arbitrary per-field timing
overrides; if no declared op mode expresses the required timing, the configuration is rejected.
Duplicate op-mode names are disambiguated by declaration index, never silently first-matched.

A device whose ESI declares no DC section may prefill `free-run` at import, but the choice is still
persisted and reviewable; missing DC in a new engineering configuration is an error, not an implicit
free-run. The compiler-facing projection keeps the existing five `dc` fields; free-run is encoded as
no `dc`/no `assign-activate`. The declared op modes live in the device-level ESI facts so the compiler
can validate offline; the mode stores the selected identity and effective values.

The runtime applies the selected effective values through `ecrt_slave_config_dc` before activation
and never selects or switches DC at runtime. Changing the DC choice or any effective value forces a
cold download (D-267); a remap alone does not change the DC selection. Live DC read-back is compared
where the master exposes it and otherwise reported `unverified`, never `match` (Q16).

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

Object metadata follows field-level authority, not one global winner (D-272). ESI is the offline
baseline and the sole PDO mapping-eligibility source; online SDO Information is the live authority
for the connected device's object existence, name, type, access and description when available and
identity/revision matches. Live reads are observed values; the project is the desired/startup-value
source; ESI default/description values are metadata, not observed values. Merge by object
index/subindex in deterministic order, keep provenance, and surface conflicts instead of silently
overwriting. A conflict on a mapped entry or persisted operation blocks configuration. Online-only
objects are inspectable and explicitly readable/writable in setup mode when access and state permit,
but are never PDO-mappable or auto-exposed as PLC channels; startup persistence follows Q8. If SDO
Information is unavailable, the ESI-only view remains usable and the absence is reported.

Do not treat all SDO writes as independent key/value assignments. Some devices require a sequence, a particular PREOP/SAFEOP state, disabling a feature before changing it, or a read-back/verification step. The model must support ordered startup operations, preconditions/state requirements, explicit failure handling and verification. Q8/D-274 fixes the generic sequencing model: one ordered, required plan, with PDO assignment/mapping kept under the separate PDO configuration abstraction rather than authored as generic startup operations.

At minimum, distinguish:

- **Read-only inspection:** never changes device state.
- **Commissioning write:** explicitly changes the currently connected device; requires managed-master ownership and a clear confirmation/diagnostic path.
- **Persist as startup configuration:** records an approved operation in the project so runtime can apply it on subsequent startup.

A commissioning write must not automatically become a persistent startup write. Conversely, saving a desired startup value must not be represented as proof that the live device currently has that value.

### 5.5 Desired configuration, observed state and diff

Provide a structured, policy-free comparison between project intent and the connected device/master. At minimum, compare:

- master and slave topology and expected dense positions;
- vendor, product, revision, serial and alias where declared and readable;
- selected operating mode where it can be observed;
- PDO assignment and mapping, including entry identity and widths where readable;
- selected DC configuration and observable state;
- configured startup SDO values versus live read-back values.

Every check reports exactly one of four results: `match`, `different`, `unverified` or `not applicable` (D-278, Q16). `unverified` covers missing read-back or an unreadable value, and missing read-back support must never be reported as a successful match. No check may infer a match from a fallback field. `different` is a factual result; what the runtime does about it is a separate policy decision.

WL-7's default consequence is **degraded per-device isolation** (D-278, Q16). A device that is offline, identity-mismatched, or fails required configuration does not enter OP and its configuration data is not written; its inputs are held and marked stale, and its outputs are not applied. Other matching devices and PLC cycles continue. There is no lazy or best-effort matching, no serial/alias/position fallback, and no automatic rebind. Master open or activation failure remains fatal; a missing or failed slave is a runtime-degraded condition, not a cold-start failure. A future `strict` policy may stop the run on a required mismatch without changing these comparison semantics.

Topology comparison evaluates every configured position independently rather than stopping at the first difference. A present slave that is not in OP normally still forwards frames, so a later position can match and operate; a missing slave breaks or shifts the chain, and downstream positions then report `different` or `unverified` and stay offline. No position renumbering, serial/alias substitution or automatic topology repair is performed. Comparison and capture never rewrite project identity or topology; capture is a separate explicit action (§5.2).

### 5.6 Project persistence and compiler/runtime boundary

Persist the engineer's desired configuration in project files. Phase 0 selects a dedicated EtherCAT
engineering configuration file, declared by a new `[io]` key, for the desired configuration; the
effective process-image selection is embedded into the existing machine profile that the compiler
already canonicalizes (D-263, Q24). The two surfaces are cross-checked, and static profiles migrate
through an explicit path. The constraints are:

- project configuration is the source of truth for desired settings;
- logical PLC devices/channels remain owned by `stc.toml` unless a deliberate schema change is approved;
- the persisted ESI source reference and identity are configuration inputs, not provenance-only metadata; they participate in the effective-configuration hash and any change forces a cold download (D-267);
- selected mode, PDO assignment/mapping and declared-entry remap, startup SDO sequence, DC settings and the constraint facts used for validation are explicit and deterministic;
- project-relative path resolution is confined to the project root, including symlink handling;
- builds do not depend on live hardware, vendor installations or an ESI directory being present after the configuration has been resolved/persisted;
- compiler validation checks internal consistency and channel mapping widths; it does not make engineering selections.

The artifact should contain the effective configuration required by runtime and enough
identity/configuration data to diagnose what was built. Any EtherCAT configuration change of any
sort forces a cold download. The effective-configuration hash covers every persisted EtherCAT
configuration input: topology, selected operating mode, PDO assignment/mapping and declared-entry
remap, startup SDO operations, DC values, persisted constraint facts, and the ESI source reference
and identity. There is no provenance-only category and no split: a difference in any of these inputs
forces a cold download (D-267, Q26). ESI files themselves are not read at build or runtime, so
editing an ESI file without re-importing it does not change the persisted project; a re-import that
changes any persisted input does.

### 5.7 Setup mode and commissioning sessions

Setup mode is the only state in which WL-7 changes hardware (`D-260`). A setup-mode session spans
several CLI invocations: it is entered and left explicitly and exposes a `status` query; commands
do not silently re-acquire the master per invocation. Entry needs no separate confirmation flag
beyond the explicit enter command.

- **Entry:** requires that no runtime or serve session owns the master. `stc` requests the master
exclusively, reports the observed bus, and records a session identity so a second engineering
invocation is rejected rather than silently sharing state.
- **Privileges:** engineering master access uses reviewed fixed-command wrappers with argument
validation and no generic passthrough; running the engineering CLI as the service/root user is the
documented alternative, not the default (`D-265`, Q23).
- **No PLC:** setup mode never activates the master for cyclic operation, never runs the PLC cycle,
and never writes process outputs. Process data is not exchanged; identity, PDO, and SDO access use
the non-realtime master state machine.
- **Operations:** inspection is allowed in any setup-mode stage; commissioning writes, including
SDO writes, are explicit and attributed to the session; startup SDO persistence is a separate,
explicit action that never follows a live write automatically.
- **Exit and recovery:** setup mode is left explicitly and releases the master. An interrupted
session (Ctrl-C, SSH drop, crash) must leave the master releasable and the project files untouched;
the next invocation reports and recovers rather than assuming a held reservation. If the process
dies while holding the master, the OS closes the device and the reservation lapses.
- **Normal runtime is unchanged:** `stc run`/`stc serve` still own the master for cycles. A
commissioning attempt while they run fails with an ownership-conflict diagnostic naming the
conflict; no automatic stop is performed.
- **Boundary:** setup mode is an engineering diagnostic/commissioning state, not a safety state and
not a hard-real-time state.

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

### 6.3 Invocation context and deferred remote UX

**Owner decision D-259 (2026-11-15).** Engineering is invocation-context independent, exactly like
the debug session: the VS Code integration reaches a debug session on a remote machine over SSH,
and from `stc`'s perspective that session is identical to a local one. Wire engineering the same
way. The remote *packaging* — a remote engineering UX, VS Code commands, and the eventual GUI —
comes after WL-7. WL-7 therefore adds **no** `stc remote exec`, no daemon, no RPC protocol, no
serve-session coupling, and no remote project-sync workflow.

What WL-7 must therefore guarantee:

- Engineering operations are expressed once, in the API, and every entry point (CLI now, UI later)
  calls the same operations. Nothing may depend on the process's working directory being the
  project root beyond the documented `--project-dir` resolution, on a terminal, or on a local TTY.
- Structured results and typed errors are the contract, so a later remote/UI client needs no
  terminal-output parsing (see §6.1/§6.4).
- Path resolution, credential/material handling, and privileged access follow the same rules on the
  target host whether the command arrives locally or through an SSH shell; nothing presumes the
  developer host is the engineering host.
- A later remote layer reuses the existing SSH/SFTP runner and the production source fingerprint.
  That work is explicitly **outside** WL-7; the reviewed gap (no generic remote execution in
  `stc remote` today) is therefore not a WL-7 blocker, and Phase 9's walkthrough drops the remote
  item.

New CLI wiring is still in scope and is not an existing surface: `stc` dispatches a fixed `Command`
enum (`crates/stc-driver/src/cli.rs`), so the `stc ethercat` family is a new variant, parser,
dispatch, help table row, and CLI tests. `crates/stc-driver/src/cli.rs` is already in
`tools/ci/host-allowlist.txt`, so Linux-only gating stays inside an allowed file.

### 6.4 Future TUI and GUI compatibility

No TUI or GUI is implemented in WL-7. The API must nevertheless avoid CLI-only assumptions: no parsing terminal output as an integration interface, no command-handler-owned business logic, no opaque string-only results, and no requirement that future clients duplicate validation or device access logic.

## 7. ESI subset and untrusted-input policy

The original WL-7 subset remains the initial candidate, subject to Phase 0 review. It includes:

- `EtherCATInfo` version 1.2, vendor ID/name, device identity and revision;
- groups/types/names for diagnostic context;
- device sync managers, RxPDO/TxPDO definitions and entries, including `Fixed`, `Sm`, `Mandatory`, `Exclude` and `ExcludedSm` where present;
- DC op modes and supported `AssignActivate`, cycle and shift fields;
- mailbox and CoE capability flags, including `SdoInfo`, `PdoAssign`, `PdoConfig` and `CompleteAccess`;
- multiple devices per file and multiple files/revisions.

Constraint attributes are interpreted exactly as declared; `PdoAssign`/`PdoConfig` are capability
gates, and mapping edits are limited to the declared-entry remap policy (D-269).

Only `EtherCATInfo` version 1.2 is accepted. Versions 1.0 and 1.1 are rejected with an explicit
unsupported-version diagnostic and are tracked as WL-24 in `FutureWishlist.md` (D-262, Q2).

The first subset need not fully model all `ObjectDictionary`, `DataType`, profile/dictionary references, vendor-specific extensions or images. However, the engineering subsystem must support online object-dictionary browsing where the device provides it; ESI's incomplete dictionary coverage must not block live SDO inspection.

Reject unsupported roots/versions and malformed or ambiguous input with stable diagnostics. Do not silently ignore fields that could affect the selected configuration. No DTD/DOCTYPE, entity expansion, external resource fetch or XInclude. Enforce file-size, nesting-depth, element/attribute-count and text-length bounds. Handle at least UTF-8 and ISO-8859-1 if confirmed by corpus measurement. Fuzz the parser and guarantee no panic/hang/unbounded allocation on arbitrary input.

The TwinCAT corpus measured in the original plan (458 files, 387 XML files, 144 declaring at least
one `Device`, UTF-8 and ISO-8859-1, largest 70.6 MB) remains local development/manual input only. CI
must not require it. Vendor ESI files are kept local and are not committed or redistributed.
Synthetic fixtures are committed. For the ASIX AX58100 bench device, the validation runbook
publishes the vendor product/ESI download link
<https://www.asix.com.tw/en/product/IndustrialEthernet/EtherCAT/AX58100> instead of shipping the ESI
sample; the engineer obtains it directly from ASIX (D-264, Q18). Use `STC_ESI_CORPUS` for an
ignored/gated local corpus campaign and record acceptance/rejection counts and diagnostic codes.

## 8. Runtime configuration and safety boundaries

The runtime consumes the persisted desired configuration. It must:

1. establish exclusive managed-master ownership;
2. open the master;
3. for each configured device, compare the live SII identity against the persisted dense position, vendor, product, revision and any declared serial/alias **before queuing any configuration**; queue the canonical PDO assignment/mapping, DC and ordered startup operations only for a device that matches (D-277, D-278);
4. activate the master; master open or activation failure is fatal and starts no PLC cycles;
5. start PLC cyclic execution once the master is active, without waiting for every configured slave to reach OP; a missing, mismatched or failed device is a runtime-degraded condition, not a cold-start failure (D-278);
6. let each matched device progress independently to OP; its queued startup operations apply at the declared states, and a device that fails a required step remains out of OP, does not have its outputs applied, and has its inputs held and marked stale while other devices continue;
7. read back and verify identity and configuration, including the remapped assignment/mapping and configured startup values, to the extent supported by the master/device; unavailable read-back is reported `unverified`, never `match`.

The list is the required logical order, not a claim that the master interleaves native PDO configuration and generic SDO downloads arbitrarily. The master's configuration lifecycle governs the actual application order; where a required ordering cannot be honored, that device's configuration fails closed and the limitation is documented (D-274).

A failure in a required startup operation or required verification is fail-closed **for that device**: it does not enter OP and its configuration is not treated as verified. An identity mismatch means zero configuration writes to that device; a matched device that fails later may retain partial writes, and no transactional rollback is claimed. Diagnostics identify the device and the failed operation. Runtime never switches mode, remaps PDOs, lazily re-matches or rebinds a device as an automatic recovery action.

The former `startup-timeout` gate is retired: no startup deadline fails the run, and PLC cycles start after master activation (D-278). A device present and configured at activation reconnects through the existing recovery path. A device absent at activation was never configured and stays offline until restart or cold download unless Phase 3 proves a safe runtime reconfiguration path; hot-connect groups are out of WL-7 scope.

**Process-image validity.** The process image stays fixed for the build: a skipped, failed or reconnecting device never shifts or lets its region be reused by a healthy device. Per-device domains or an equivalent fixed-image mechanism keep each device's declared offsets and mark only that region stale (D-278). The PLC validity rule is per device, not global: `slave<N>.input_stale` in status layout 2 reports that device's inputs as held/not current, while global `input.stale` is a summary meaning at least one device is stale or the backend exchange itself failed. `online`, `operational` and `al_state` remain diagnostic. Degraded operation is not a safety function and adds no safety claim.

**Shim ABI boundary.** ABI 2 is the only supported shim ABI (D-279). ABI 1 is retired with no dual loader, fallback or legacy `verification unavailable` path; a library that fails the ABI 2 handshake is rejected with one explicit diagnostic. ABI 2 carries forward the lifecycle, configuration, registration, status and cycle calls WL-7 still needs; adds position-authoritative identity/discovery, PDO configuration and read-back, SDO upload/download and startup-SDO queuing, DC read-back where supported, per-device domain/WC status, capability reporting and structured results; and may retire unused ABI 1 surface. The shim reports facts and capabilities only; the Rust backend computes `match`/`unverified`/`stale` and maps them to diagnostics (Q17, D-280). A required call that IgH 1.6.13 cannot provide makes that device's check fail closed or report `unverified`, never `match`; SDO Information is attempted and a proven gap is reported while the ESI-only view remains usable (D-272).

**Master ownership boundary.** IgH reserves a master for exclusive use:
`ecrt_request_master()` fails with `-EBUSY` while another application holds it, and the `ethercat`
tool can only access a master in its idle phase (no application reservation). WL-7 has no
externally managed master mode. The supported scope is:

- an **idle** local master (nobody has requested it) may be inspected read-only, which is the same
  state a managed-master commissioning operation starts from;
- a master **reserved by `stc run`, `stc serve`, or another `stc` engineering session** is reported
  as unavailable (ownership conflict) and no read is attempted; setup-mode entry fails closed;
- a master owned by a non-IgH stack is out of scope and must be documented as unsupported rather
  than partially supported.

Any hardware-changing operation, including an SDO write or a remap, requires managed setup mode
(D-260). SDO writes are normal setup activity, not a forbidden operation. The API still separates
inspection from mutation so the command surface and ownership rules stay explicit.

**SDO and read-back feasibility (IgH 1.6.13).** `ecrt_master_get_slave`, `get_sync_manager`,
`get_pdo`, and `get_pdo_entry` provide the connected-slave information interface in userspace;
SDO upload/download exist and block on the master state machine, so they run on a commissioning
thread and never on the cycle thread; pre-activation startup writes use `ecrt_slave_config_sdo*`
and `ecrt_slave_config_complete_sdo`. Phase 3 prototypes the exact idle/operation-phase behavior
of each call and records what is genuinely observable before Phase 4/7 depend on it.

No safety or hard-real-time claim is added. The HIL scope remains the DIEWU bench unless additional hardware is explicitly available and approved.

## 9. Diagnostics and observability

Existing stable diagnostic codes keep their meanings; `STC-E-IO-ETH-008` is retired/reserved by D-278 and is never reused. Q17 (D-280) fixes the WL-7 families and emission rules:

| Family | Scope |
| --- | --- |
| `STC-E-ESI-001`–`009` | ESI import/capability: missing/unreadable/outside root, unsupported version/construct, malformed/limits, ambiguous identity/revision, re-import required. |
| `STC-E-IO-ETH-009`–`019` | Topology/compare: invalid/duplicate dense position, extra slave, downstream shift, serial/alias mismatch, missing DC reference. |
| `STC-E-IO-ETH-020`–`029` | Engineering mode/PDO/DC/channel configuration: invalid/ambiguous mode, constraint violation or remap outside the declared set, live remap refusal, invalid/missing DC or factor, channel exposure. |
| `STC-E-IO-ETH-030`–`039` | SDO commissioning/startup: object unavailable/access denied, abort, timeout/state, verification mismatch, required read-back unavailable. |
| `STC-E-IO-ETH-040`–`049` | Ownership/setup/ABI: master ownership conflict, write outside setup mode, required ABI 2 capability unavailable. A failed ABI 2 handshake, including ABI 1, is `STC-E-IO-ETH-001`. |
| `STC-W-IO-ETH-004`–`019` | Runtime degraded: device isolated, per-device stale transition, missing DC reference, extra slave, downstream shift, SDO Information unavailable, verification unverified. |

`E` means the requested engineering operation/configuration is rejected or run start is fatal; `W` means the run continues degraded. The importer emits `STC-E-ESI-*`; the engineering CLI/API emits topology/config/SDO/ownership `STC-E-IO-ETH-*`; the compiler emits `STC-E-PROJECT-*`; the runtime emits fatal `STC-E-IO-ETH-*` and degraded `STC-W-IO-ETH-*`; the shim returns structured facts/capabilities and never codes. Every diagnostic carries code, severity, device name/position, operation, object index/subindex, expected/observed, AL state, abort code, missing capability and remedy; human text is secondary and never parsed for control flow. Runtime degraded diagnostics emit on state transitions, not every cycle; per-cycle truth is `slave<N>.input_stale` and `stc run -v`. `match` is not a diagnostic; `different`/`unverified` are structured compare results, and `unverified` becomes a warning when it affects a required check. Every new code has an emitting test and a traceability entry; unused numbers in each range are reserved for later diagnostics in that family.

Diagnostics distinguish at least:

- ESI missing/unreadable/outside project root;
- unsupported ESI document/version/field or an external resource;
- malformed or resource-limit-exceeding ESI;
- ambiguous identity/revision selection or a source change requiring re-import;
- invalid/duplicate dense position, extra slave, downstream position shift, or missing DC reference;
- invalid or unsupported PDO/SM/mapping selection;
- PDO selection outside the ESI-declared set, or a synthesized/edited entry or PDO;
- a `Fixed`, `Mandatory`, `Exclude` or `ExcludedSm` constraint violation, or a live refusal of a requested remap;
- PDO assignment/mapping read-back unavailable or different from the desired remap;
- logical channel-to-PDO mapping mismatch;
- live topology/identity mismatch, including a declared serial/alias mismatch;
- SDO object unavailable, access denied, invalid value, abort code, timeout or state/precondition failure;
- startup sequence failure and read-back mismatch;
- master ownership conflict or an attempted write outside setup mode;
- ABI 1, a failed ABI 2 handshake, or a missing required ABI 2 capability;
- configuration differs from observed hardware;
- a device isolated at runtime with held/stale inputs and no applied outputs;
- verification unavailable because the master/shim/device does not expose the required read-back;
- SDO Information unavailable, with the ESI-only view still usable.

`stc run -v` and existing status surfaces report configuration provenance and runtime verification state without changing established layouts gratuitously; per-cycle device validity is `slave<N>.input_stale` in status layout 2.

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
- Define the project configuration model and file ownership: logical topology/channels, desired device config, ESI source reference and identity, PDO selection, startup SDO sequence and DC configuration.
- Define managed-master ownership and the idle-phase read-only inspection boundary; no externally managed master mode.
- Define the API boundary and the CLI command surface, and confirm the API/CLI stay
  invocation-context independent with remote UX out of scope (D-259).
- Define the generic representation of typed SDO values, ordered operations, state/preconditions, failure policy and verification.
- Decide parser crate versus project-owned reader; ESI versions; limits; file resolution and revision-pinning policy.
- Decide the first supported ESI subset and how ESI constraints (`Fixed`, `Mandatory`, `PdoAssign`, `PdoConfig`) affect selection.
- Decide how selected drive modes are represented without hard-coding vendor-specific motion semantics.
- Decide shim ABI evolution and which runtime read-back operations are feasible with IgH 1.6.13.
- Decide diagnostic ownership, stable families, severity and emission policy across importer, engineering CLI/API, compiler, runtime and shim (D-280).
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
- Evaluate the vetted parser candidate (`quick-xml` first) against the local ESI corpus and record the selection before locking the parser implementation; keep the bounds, fuzzing and no-external-resource policy either way (D-261, Q1, Q13).
- Implement bounded ESI reader and typed capability model (vendor, devices, identity/revision, syncs, PDOs/entries, DC modes, mailbox/CoE flags and supported dictionary metadata).
- Implement ESI library search, explicit source selection, identity/revision matching and deterministic diagnostics. Importer failures use the `STC-E-ESI-001`–`009` family with the D-280 structured payload (device/position, operation, expected/observed, capability, remedy); human text is never parsed for control flow.
- Define the WL-7 diagnostic catalog and traceability mapping as codes are introduced: every emitted code has one owner layer, a stable meaning, an emitting test and a traceability entry, and unused numbers stay reserved (D-280).
- Define the ABI 2 capability/result contract and its host-neutral mock at the API boundary, so engineering tests can exercise unavailable-capability and structured-error cases on every CI host; ABI 1 is retired, not kept as a fallback (D-279). The shim result contract carries facts and capability flags only; it returns no diagnostic codes.
- Add synthetic fixtures, malformed cases and fuzz target.
- Add CLI commands for ESI list/search/inspect.

The Q19 answer (D-270) adds no crate: the engineering engine and FFI stay in `stc-runtime` and the
CLI stays in `stc-driver`, so no workspace membership, `Cargo.lock`, xtask suite, host-allowlist, or
`Architecture.md` crate-map change is needed for a new crate.

**Exit criteria:** ESI inspection is deterministic; unsupported/ambiguous inputs produce stable
diagnostics; API tests do not depend on CLI text; parser fuzz smoke is green; the parser selection
and its corpus evaluation are recorded; the ABI 2 capability/result contract and host-neutral mock
exist and the API tests run on every CI host; every ESI diagnostic the importer can emit has an
emitting test and a traceability entry, and no shim result contains a diagnostic code.

**Evidence:** unit and fixture tests, fuzz smoke output, the parser-selection/corpus record, the
crate/home record, the diagnostic catalog/traceability entries, and the API test listing that proves
no CLI-text coupling.

#### Phase 1 record (2026-11-24)

**Parser selection (D-261, Q1).** `quick-xml` 0.42.0 is accepted, with default
features disabled. The reader (`stc-runtime::ethercat::esi::reader`) owns the
bounds and the rejection policy rather than the crate: a 128 MiB file-size cap,
nesting-depth, element, attribute and text limits, UTF-8 and ISO-8859-1 only, and
explicit rejection of DOCTYPE/DTD, entity declarations and XInclude before the
parser sees the document. It resolves only predefined and numeric character
references; any other entity is rejected. Attribute values are normalised with
the XML 1.0 rules.

**Corpus evaluation.** The local TwinCAT corpus is not present on the
implementation host (`STC_ESI_CORPUS` is unset), so the selection was evaluated
against the committed synthetic fixtures in
`crates/stc-runtime/tests/fixtures/esi`. The fixtures cover these corpus
features: multiple devices per document, `Fixed`/`Mandatory`/`Exclude`/
`ExcludedSm`, DC op-mode alternatives with `Factor`, `AssignActivate`, Sync0/Sync1
cycle and shift, mailbox/CoE flags, partial `Profile/Dictionary` metadata with
sub-items, vendor-extension noise, predefined and numeric entity references,
ISO-8859-1 bytes, and unsupported or malformed documents. The gated local corpus
campaign with accept/reject counts by code and size band remains Phase 8 work
(§7).

**Code placement (D-270).** The engineering engine, ESI reader, catalog, API and
shim contract live in `stc-runtime`; `stc-driver` holds only the `stc ethercat`
CLI wiring. No crate was added.

**ABI 2 contract (D-279).** `stc-runtime::ethercat::abi` fixes the Rust-side ABI 2
name/version, capability flags, structured result categories and position-
authoritative identity facts. `stc-runtime::ethercat::mock` is the host-neutral
boundary that lets every CI host exercise unavailable-capability,
ownership-conflict and structured-error cases. The shim loader still performs the
ABI 1 handshake; Phase 1 does not change runtime behavior, and Phase 7 replaces
the loader.

**Diagnostic catalog (D-280).** `STC-E-ESI-001`-`007` are implemented with an
emitting test each (`importer_emits_every_introduced_code` asserts the emitted
set equals the introduced set); `STC-E-ESI-008`-`009` stay reserved and are never
emitted. Every diagnostic carries code, severity, operation, source, device,
position, expected/observed, capability and remedy, and the CLI prints it without
parsing human text for control flow.

**API test listing.** `crates/stc-runtime/tests/ethercat_esi.rs` exercises the
typed API only: library listing/search determinism (including creation order),
pinned and unpinned identity resolution, ambiguity and not-found diagnostics,
capability inspection, explicit source confinement to the library root,
ISO-8859-1 decoding, the ABI 2 contract and mock, and arbitrary-input safety.
`crates/stc-driver/tests/ethercat_cli.rs` covers the CLI surface, and the two
`cli::tests::ethercat_*` unit tests cover parsing. No test asserts on terminal
text from the API.

**Fuzz.** `fuzz/fuzz_targets/fuzz_esi.rs` drives `parse_bytes_for_fuzz`; seeds are
committed under `fuzz/seeds/fuzz_esi` and the CI fuzz-smoke job seeds and runs it
with the other targets.

### Phase 2 — Topology authoring, bus discovery and master ownership

**Goal:** support offline machine definition and online discovery without conflating desired and observed topology.

Work:

- Define and validate a manually authored topology with master/slave order, a unique dense position per master, the exact vendor/product/revision identity, optional serial/alias instance checks, and logical device associations (D-277).
- Implement managed-master ownership and conflict detection for commissioning/runtime operations.
- Add the reviewed fixed-command privileged wrappers for engineering master access, with argument validation and no generic passthrough; document the run-as-root alternative (D-265, Q23).
- Implement online bus scan and structured observed-topology results.
- Implement slave identity inspection and per-position desired-vs-observed topology comparison; evaluate each position independently and never stop at the first difference. Topology/compare failures use `STC-E-IO-ETH-009`–`019` (invalid/duplicate dense position, extra slave, downstream shift, serial/alias mismatch, missing DC reference) with the D-280 payload, and observed facts never silently rewrite project intent.
- Implement explicit capture/import from discovered topology into a project draft; default capture records vendor/product/revision without pinning serial numbers, and serial/alias are captured only on explicit request.
- Implement idle-phase read-only inspection when no application owns the master; any write, including SDO, requires setup mode. Do not add a separate externally managed master mode.
- Add CLI commands for topology show/scan/diff/capture and slave inspect.

Implement setup mode (§5.7, D-260) here: explicit persistent entry and leave with a `status` query,
session identity, no PLC cycle, no output writes, recovery after an interrupted session, and the
ownership-conflict diagnostic when a runtime holds the master. No remote execution work is done
(D-259).

**Exit criteria:** a topology can be authored with no hardware, including dense positions and exact identities; a live scan reports observed
facts; scan never silently rewrites project intent; default capture does not pin serial numbers; a declared serial/alias mismatch is reported against its position, not used to rebind; setup mode is entered only with the master
free, runs no cycle, and is released on normal and interrupted exit; engineering master access
uses the reviewed fixed wrappers; a conflicting runtime owner is rejected with a diagnostic
naming the conflict; a second engineering session is rejected.

**Evidence:** topology/scan/mock tests, setup-mode entry/exit/recovery/status tests,
ownership-conflict and privileged-wrapper tests, and the recorded D-259/D-260/D-265 decisions.

#### Phase 2 record (2026-11-25)

**Topology model (D-277).** `stc-runtime::ethercat::topology` owns the desired topology: one
managed master, a unique dense `position` per device, the exact vendor id/product code/revision,
optional `serial`/`alias` instance checks, and the logical device name. Validation emits
`STC-E-IO-ETH-009` (missing, duplicate or non-dense position), `010` (identity not exact: zero,
missing, or a wildcard such as `"any"`) and `011` (empty or duplicate name). The desired topology
is authored offline as a JSON draft (`stc-ethercat-topology/1`) because the durable engineering
configuration file declared by an `[io]` key is Phase 5 schema work (D-263, Q9/Q24); Phase 5
replaces the draft file and keeps the typed model unchanged.

**Observed topology and comparison (D-278).** `scan` reports the host's slave facts in position
order and records them as `stc-ethercat-observed/1`, which `topology diff`, `topology capture` and
`slave inspect` accept through `--observed` so an observation can be reviewed offline. Comparison
evaluates every configured position independently and reports `match`, `different`, `unverified`
or `not applicable` per field (`presence`, `online`, `identity`, `serial`, `alias`). A missing
slave is `different` and emits `STC-E-IO-ETH-014` when later positions still respond; an observed
slave the project does not configure emits `012`; identity, serial and alias mismatches use the
existing `STC-E-IO-ETH-003`, extended to declared instance checks per D-280. Nothing is
renumbered, rebound or repaired, and the desired topology is never modified.

**Capture (D-277).** `topology capture` records vendor/product/revision and does not pin serial
numbers or aliases; `--pin-serial`/`--pin-alias` add them as hard checks and fail with
`STC-E-IO-ETH-015` when the value cannot be read. Capture reuses the project's logical names where
a position still matches, rejects an empty, offline or non-dense observation, and writes only the
requested output file, so it never rewrites project intent.

**Master ownership and setup mode (D-260, D-271).** Inspection is refused with
`STC-E-IO-ETH-040` while a runtime or another owner holds the master, and the diagnostic names the
owner; a recorded setup session may inspect through its own reservation. Setup mode is an explicit
persistent session (`stc-ethercat-setup/1` descriptor, default `.stc/ethercat-setup.json`):
`setup enter` records a session identity and pid, a second entry fails with
`STC-E-IO-ETH-044`, `setup leave` releases and clears the record idempotently, and `status`
reports the recorded session, the observed master owner and any conflict. An unreadable descriptor
is `STC-E-IO-ETH-045` and is recovered by `setup leave`, so an interrupted session is reported and
cleared rather than assumed to be held. Setup mode runs no cycle and writes no outputs.

**Host boundary.** The ABI 2 shim binding is Phase 7 work (D-279), so the CLI host reports no
capabilities and every live operation fails closed with `STC-E-IO-ETH-043` naming the missing
`identity` capability instead of claiming an empty bus. Scan, comparison, capture and inspection
are exercised through `MockEngineeringHost` on every CI host
(`crates/stc-runtime/tests/ethercat_topology.rs`,
`crates/stc-driver/tests/ethercat_topology_cli.rs`).

**Privileged wrapper (D-265).** `tools/ethercat/stc-wl7-eng` is a fixed-command wrapper installed
root-owned and granted to the `stc-eng` service account by `tools/ethercat/stc-wl7.sudoers`; every
verb and argument is validated, no generic passthrough exists, and the launcher, project, session,
draft and observation paths are fixed. Argument validation runs before any host path is touched,
and `the_privileged_wrapper_rejects_every_unlisted_verb_and_argument` pins that rejection surface.
`specification/Verification/EtherCAT-Runbook.md` §14 records the install, use, privilege model and
the documented run-as-root alternative.

**CI note (2026-11-25).** Run
[45653669868](https://github.com/r2k-in-the-vortex/stc/actions/runs/45653669868) at `62658aa` is the
phase's acceptance run and is green across all jobs (Rust tests on three hosts, coverage, fuzz
smoke, EtherCAT shim, docs, traceability, clippy). The follow-up test-only commit `9648020` ran as
[45659703746](https://github.com/r2k-in-the-vortex/stc/actions/runs/45659703746): 44 of 45 jobs
succeeded, and `VS Code extension smoke (macos-15)` was cancelled twice before a runner was
assigned (no steps ran, no runner name, about 16 minutes queued each time; the second attempt was
authorised by the owner). `macOS tests (macos-15)` in the acceptance run had the same signature
once and passed on rerun. Both cancellations are recorded as CI infrastructure events for
follow-up, not as a test failure; no code was changed to make CI pass.

### Phase 3 — PDO capability inspection and explicit configuration selection

**Goal:** let the engineer choose a valid device configuration from ESI/live capabilities.

Work:

- Model supported PDOs, sync managers, entries, the declared-entry pool per device/direction, and selection constraints.
- Expose alternative operating modes/configurations as named per-device mode bundles (D-275).
- Implement constrained selection plus declared-entry remapping (D-269): omit optional declared entries, reorder, and move declared entries between declared PDOs of the same direction; reject object-dictionary-only entries, synthesized PDOs, and edited index/subindex/bit length.
- Validate PDO assignment and mapping, including direction/order/entry widths and `Fixed`/`Mandatory`/`Exclude`/`ExcludedSm`/`PdoAssign`/`PdoConfig`.
- Emit the engineering-configuration diagnostics from `STC-E-IO-ETH-020`–`029` for invalid or ambiguous modes, constraint violations, remap refusals, invalid DC/factor choices and channel-exposure errors, using the D-280 structured payload. A live remap refusal is `STC-E-IO-ETH-022`; unavailable read-back is reported as `unverified`, never `match`, and never becomes a fabricated code.
- Persist exactly one selected mode per device plus any stored alternates; each mode carries its selected PDO assignment/mapping, declared-entry remap, per-mode channel exposure, ordered startup plan, DC selection and optional ESI provenance, together with the constraint facts needed for offline revalidation (D-275).
- Validate mode selection: names unique per device, selection explicit and resolvable, alternates inert until selected, optional ESI DC op-mode provenance kept descriptive and behavior-neutral, startup/PDO/DC consistency checked, and a switch that leaves a bound channel unexposed rejected; reject vendor-specific motion semantics in the generic schema (D-275).
- Expose ESI DC op modes as explicit per-mode choices with `AssignActivate`, Sync0/Sync1 timing and factor-resolved cycles; reject missing or ambiguous selections, duplicate-name ambiguity and invalid/no-DC combinations, and never choose the first op mode (D-276).
- Model the canonical PDO configuration as declarative sync-manager/PDO/entry structure plus a generated, inspectable diagnostic view of the equivalent CoE operations; the view is not a user-authored startup sequence (D-274).
- Keep logical PLC channels explicit; expose selected entries only, with one full-entry channel as the default proposal, explicit non-overlapping bit slices, per-device unique names, and no auto-exposure of unselected entries (D-273).
- Add live inspection of actual PDO assignment/mapping where the master/device supports it, and reject a remap the live device refuses.
- Prototype IgH 1.6.13 native PDO configuration, startup SDO ordering/lifecycle and PREOP/SAFEOP timing, including whether a declared-entry remap is applied and observable; record what ordering the master actually guarantees and investigate literal SDO-based PDO configuration only if the native API cannot support a required configuration or sequence. Record the ABI 2 capability matrix at PREOP/SAFEOP — identity/discovery, master PDO tree and live CoE upload of assignment/mapping objects, DC read-back, per-device domains and SDO Information availability; an unavailable required call fails closed or reports `unverified`, never `match`, and the ABI 2 surface is amended if a required call cannot be made to work (D-279).

**Exit criteria:** engineer can inspect alternatives, select a mode and configuration, remap within the
declared-entry set, validate it offline and compare it with live device state; mode selection is
explicit, alternates do not affect the build, and switching to a mode that leaves a bound channel
unexposed is rejected; DC selection is explicit, factor-resolved and validated, with no first-op-mode
default or arbitrary timing override; channel exposure rejects unselected entries, width overruns,
misaligned multi-bit slices, overlaps and duplicate names; unsupported combinations, arbitrary
object-dictionary mappings and live refusals fail with actionable diagnostics; the IgH prototype
record states which identity/PDO facts are observable, whether a remap is applied and read back, in
which master phase, and what ordering between native PDO configuration and generic startup SDOs the
master actually guarantees; the ABI 2 PREOP/SAFEOP capability matrix is recorded, including SDO
Information availability and any capability gap; the effective PDO configuration is inspectable and
verifiable against observed state.

**Evidence:** selection/validation tests, named-mode accept/reject fixtures (explicit selection,
duplicate names, alternate inertness, vendor-semantics rejection, channel loss on switch), DC
op-mode selection/factor/free-run fixtures (missing selection, duplicate names, invalid combinations,
no-DC device, live read-back unavailable), declared-entry remap accept/reject fixtures, attempted
arbitrary-mapping rejection, the read-back prototype record (calls, master phase, observed fields),
the ABI 2 PREOP/SAFEOP capability matrix and SDO Information availability record, fixtures for
fixed/mandatory/exclude/alternative PDO sets, and the emitting-test/traceability record for every
`STC-E-IO-ETH-020`–`029` code introduced here.

#### Phase 3 record (2026-11-26)

**PDO capability model (D-269).** `stc-runtime::ethercat::pdo` projects an
accepted description into sync managers, `Rx`/`Tx` alternatives, declared
entries with their originating PDO, and the `Fixed`/`Mandatory`/`Exclude`/
`ExcludedSm`/`PdoAssign`/`PdoConfig` facts. It also carries the declared DC op
modes. `PdoSelection::validate` rejects an undeclared PDO or entry, a
synthesized PDO index, an edited entry identity or bit length, a missing PDO
sync-manager declaration (no inference), a constraint-fact disagreement with
the ESI, an exclusion conflict, a fixed or mandatory PDO omission, and a remap
that the device's `PdoAssign`/`PdoConfig` gates do not allow. Optional entries
may be omitted or reordered, and a declared entry may move to another declared
PDO of the same direction; fixed and mandatory PDO entries stay in place.
`validate_facts` revalidates the persisted facts without the ESI for the
compiler-facing path (Phase 6).

**Mode bundles (D-275).** `stc-runtime::ethercat::config` stores the
authoring source for every mode: a stable name unique per device, description,
descriptive-only provenance, the canonical PDO configuration, per-mode channel
exposure, the ordered generic startup plan and the explicit DC choice. Exactly
one mode is selected; alternates are validated structurally but stay inert and
are not checked against bound channels. A missing or unresolvable selection is
`STC-E-IO-ETH-020`. The parser rejects unknown keys, so vendor-specific motion
fields cannot enter the generic schema; vendor meaning stays in names,
descriptions and authored values.

**DC policy (D-276).** Every mode records `free-run` or one ESI-declared op
mode addressed by name and declaration index. `DcTiming::resolve` applies the
declared `Factor` to the configured runtime cycle and fails closed on a zero or
overflowing factor, a zero cycle, an edited `AssignActivate`, a persisted
effective value that disagrees with the declaration, an invalid Sync0/Sync1
multiple, or a shift outside its cycle. A device without a `<Dc>` section can
only select free-run; free-run remains valid for a DC-capable device as an
explicit choice. Duplicate op-mode names are disambiguated by index.

**Channel exposure (D-273).** `validate_channels` accepts only selected
effective entries, checks direction, width and whole-entry fit, requires a
byte boundary for multi-bit slices, rejects overlapping slices and duplicate
or empty names, and rejects a bound `stc.toml` channel the selected mode does
not expose (`STC-E-IO-ETH-024`). `Mode::propose_channels` produces the one
full-entry-per-selected-entry proposal with ESI names made deterministic and
unique; applying it is an explicit authoring action.

**Canonical PDO configuration and CoE view (D-274).** The selection is the only
PDO authoring source. `CoeView::from_selection` generates a deterministic,
non-editable diagnostic sequence of the equivalent CoE assignment and mapping
operations; it is not persisted and not a startup list. A generic startup
operation addressing `0x1600`–`0x17FF`, `0x1A00`–`0x1BFF` or `0x1C00`–`0x1C4F`
is rejected as an invalid mode (`STC-E-IO-ETH-020`), as is a duplicated id, an
unbounded timeout or a `verify` operation with no verification policy.

**Diagnostics (D-280).** `STC-E-IO-ETH-020` (invalid/ambiguous mode, ESI
reference or mailbox-fact disagreement, startup ownership of PDO objects),
`021` (selection/remap constraints), `022` (live remap refusal), `023` (invalid,
missing, ambiguous or unresolvable DC) and `024` (channel exposure) are
implemented, each with an emitting fixture; `025`–`029` stay reserved.
`STC-E-IO-ETH-041` is introduced for a hardware-changing operation outside
setup mode and is exercised by the PDO apply path. Unavailable read-back is
always `unverified`, never a fabricated code and never `match`.

**Persistence.** The engineering draft is `stc-ethercat-config/1` JSON, exactly
as the phase-2 topology draft: one master, each device with its stored modes,
the selected mode name, the persisted identity, the ESI source reference and
digest, the persisted mailbox gates, and per mode the PDO tree with constraint
facts, channels, startup plan and resolved DC values. Phase 5 replaces the
draft file with the durable project schema declared by an `[io]` key and keeps
the typed model.

**API and CLI.** The typed API adds `pdo_capabilities`, `slave_pdos`,
`apply_pdo_configuration` and `validate_configuration`; the mock host adds live
PDO read-back plus structured apply with a refusal category, and returns facts
only, never codes. `stc ethercat esi inspect` now prints each PDO's entries and
constraints and every DC op mode with `AssignActivate`, cycle, factor and
shift. New commands are `slave pdos <position>`, `config show --config <file>`
and `config validate --config <file> [--bound-channel <device>:<channel>]
[--cycle-ns <ns>]`. Validation is offline and needs no hardware or ESI library;
when the library is present the resolved description is cross-checked against
the persisted digest and mailbox facts.

**Host boundary.** The ABI 2 shim binding is Phase 7 work (D-279), so the CLI
host reports no capabilities and a live PDO read-back or remap fails closed
with `STC-E-IO-ETH-043` naming `pdo-read-back` or `pdo-configuration` instead
of claiming an empty or accepted bus. All live behavior is exercised through
`MockEngineeringHost` on every CI host:
`crates/stc-runtime/tests/ethercat_config.rs` (11 tests),
`crates/stc-runtime/tests/ethercat_pdo_api.rs` (6 tests) and
`crates/stc-driver/tests/ethercat_config_cli.rs` (8 tests). The bench prototype
below adds the real IgH/userspace behavior for the same calls.

**IgH 1.6.13 prototype record — executed on the `ecat` bench (2026-11-26).** The
probe `tools/ethercat/phase3_probe.c` was built on the host with `-Wall -Wextra
-Werror` against the installed userspace (`libethercat.so.1.2.0`) and run as the
login user in the `ethercat` group; no `sudo` and no privileged wrapper were
used. Each mode released the master before exit and the master returned to
`Phase: Idle` with the slave in PREOP, as shown by the before/after samples.
The probe was run in four modes: `probe` (identity, master PDO tree, CoE upload,
legal apply), `remap` (an RxPDO set that violates the SII's fixed mapping),
`startup-sdo` (a queued startup SDO) and `dc` (Sync0 op mode applied with
application time). The results replace the earlier call-level matrix.

| Fact | Call | Observed on `ecat` | What it proves |
| --- | --- | --- | --- |
| SII identity | `ecrt_master_get_slave` | rc 0; position 0, alias 0, vendor `0x00000b95`, product `0x00010200`, revision 1, serial 0, PREOP, `sync_count=3`, `sdo_count=0` | Position-authoritative identity for preflight. `sdo_count=0` is the SII signal that this slave has no CoE dictionary. |
| Configured PDO tree | `ecrt_master_get_sync_manager`, `ecrt_master_get_pdo`, `ecrt_master_get_pdo_entry` | rc 0 for SM0–SM2: `0x1a00`/`0x3101:01`, `0x1a01`/`0x3101:02`, `0x1600`/`0x3001:01`, `0x1601`/`0x3001:02`, all 8 bit | The master's configuration tree is readable after `ecrt_slave_config_pdos` and matches the applied description. It is **not** proof that the device accepted it. |
| Live assignment/mapping | `ecrt_master_sdo_upload` of `0x1018:1`, `0x1C12:0`, `0x1C13:0`, `0x1A00:0`, `0x1A00:1` in PREOP | rc `-93` (`EPROTONOSUPPORT`) with abort 0 for every object; kernel `Slave does not support CoE!`; `ethercat sdos` prints nothing and exits 0 | The DIEWU has no mailbox and no CoE dictionary, so live assignment/mapping read-back is unavailable. The effective configuration must be `unverified`, never `match`. It is the SII, not CoE, that fixed the mapping. |
| Remap application | `ecrt_slave_config_pdos` with the declared set, then with both output bytes merged into `0x1a00` and SM1 empty | Both activate rc 0; slave OP, domain WC 3, `wc_state=2`, in both runs | IgH accepts an undeclared remap and the slave still reaches OP. This device exposes no refusal and no per-object error, so a remap can neither be confirmed nor refused at runtime; the outcome is `unverified`. |
| Generic startup SDOs | `ecrt_slave_config_sdo8(0x1C12, 0, 1)` queued before activation | queue rc 0, activation rc 0; slave stayed PREOP (`al_state=2`) with WC 0 through 200 cycles; kernel `SDO configuration failed` and `Slave does not support CoE!` | A startup SDO on a mailbox-less slave fails closed and blocks OP, which is the runtime's startup-timeout path. It must not be reported as applied. |
| DC apply | `ecrt_slave_config_dc(sc, 0x0300, 1 ms, 0, 0, 0)` with `ecrt_master_application_time`, `ecrt_master_sync_reference_clock` and `ecrt_master_sync_slave_clocks` each cycle | apply rc 0; slave reached OP and WC 3 by cycle 200; after release register `0x0980` read 0. SII reports 64-bit DC and `ethercat slaves -v` shows `Distributed clocks: yes` | The Sync0 assignment is accepted and the slave operates while application time is supplied. Userspace has no DC getter, and the activation register is cleared on release, so the effective DC configuration stays `unverified` (D-276). The `0x0300` word comes from the synthetic ESI fixture; the real device ships no vendor ESI in this repository. |
| Per-device domain/WC | `ecrt_domain_state` | WC 3 / `wc_state=2` in the legal, merged and DC runs; WC 0 with PREOP in the startup-SDO run | The working counter is the only live health signal. It did **not** change for the merged mapping, so WC alone cannot detect a mapping mismatch. |
| SDO Information | `ethercat sdos` | no output, exit 0; no SII mailbox | SDO Information is absent on this slave. Phase 4 reports `STC-W-IO-ETH-009` and keeps the ESI-only view usable; no dictionary is claimed. |

**Environment and evidence.** Host `ecat-host`, kernel `6.12.115-ecat-rt`, IgH
1.6.13 at commit `61cc654f5b721ddd54df0f58bdd34106d91c5359`, userspace
`libethercat.so.1.2.0` SHA-256
`f8b81293fd856684f632826f2d1dcebad0646889a2c67dfbbedae47a32626de9`, master
`Phase: Idle`, one DIEWU slave in PREOP before and after. Probe source SHA-256
`bec55b6da734158f735209e103a40d1663ca04921c5b2266ff3637a501b6a14e`; the binary
built from it is `cdf909fce794cb49ab6238390a62b3cb82d6930ceed74b292cfbb8bd65e8f231`.
Logs stay on the host under `~/stc-wl7-phase3/logs/`:

| Log | SHA-256 |
| --- | --- |
| `probe-20261126.log` | `e6cac7d2b77c03f88bf3e57a7b06789546f080eddc29f1df48bf4a27346b963f` |
| `remap-20261126.log` | `9e8fe7f4478e0b65f8afe45b36f729efd372e4b8efcb0bc1d207e970da329d85` |
| `startup-sdo-20261126.log` | `4869f3bbaf4a9103c20f25facb8aba46c1770fbf981fc3d45e57acb817c001cb` |
| `dc-20261126.log` | `a8987fc9c2332005438f92bb1d527846a8b44016325477d3bf7d9a60522042f6` |
| `probe-master-before.txt` / `probe-master-after.txt` | `ab85919e4b855ae9dd945f8211d5c55670fdc9a72d93e8e73558a8183bd212c8` / `8b60c7c245eeb7e6b96cea0683e3534c0a867d7b2f12103303d0af5be5f584f1` |
| `probe-slaves-before.txt` / `probe-slaves-after.txt` | `4f94d5060a0386cc9a684443117919cd69b82ea5aeaf55a45de6cc94c1ccdbe0` (both) |

**Consequences for ABI 2.** The shim must gate CoE read-back and SDO startup on
the slave's SII/CoE capability (`sdo_count`, mailbox, and the kernel's "does not
support CoE" result), not on the presence of ESI metadata. Where the gate fails,
reported PDO and DC state is `unverified` and startup SDOs fail closed; where it
passes, Phase 7 must still compare the uploaded assignment/mapping before
`match`. A merged remap that the master accepts but the device cannot verify
must never be reported as applied.

**Owner acceptance.** The bench closes the prototype gate: every call the ABI 2
contract depends on exists, and the recorded behavior is what a read-back-less,
fixed-mapping slave can provide. It also adds a device limit the plan could not
know from the synthetic fixture: the attached DIEWU has no mailbox, so the
phase's "live inspection of actual PDO assignment/mapping where the
master/device supports it" and "reject a remap the live device refuses" have no
live path on this slave. Selection and remap refusal are covered offline and by
the mock; effective read-back stays `unverified`.

**Owner decision (2026-11-27).** The owner accepted Phase 3 with that capability
limit. Live PDO read-back and device-refused remap are recorded as unavailable
on the fixed, mailbox-less DIEWU: the effective configuration is always
`unverified` there, never `match`, and no refusal is claimed. The limit is a
device property, not a workaround; a CoE-capable slave can repeat the read-back
and refusal cases without changing the phase's implementation. The acceptance
row is in §14.

**CI evidence.** Implementation `c3afb51`, CLI-assertion follow-up `3fa2b80`; CI run
[45864256766](https://github.com/r2k-in-the-vortex/stc/actions/runs/45864256766) attempt 2 is green
on all 45 jobs (rustfmt, clippy, docs, traceability, host-audit, workspace tests on Windows,
Ubuntu and Ubuntu arm64, macOS arm64 and Intel, coverage with the 85% gate, debug suites, fuzz
smoke, Lean proofs, release packages and extension smoke). Two earlier dispatches failed only on
the phase-1 `ethercat_cli` assertion, which still expected the old single-line DC summary replaced
here; the second of them was a dispatch race whose `headSha` proves the follow-up was not in the
tree. Attempt 1 of the green run had one unrelated macOS-Intel failure
(`stc-runtime debug_protocol::disconnect_clears_state_and_reconnect_has_no_replay`), which passed
on rerun and on every other host and is untouched by Phase 3; it is a watch item, not a claimed
fix.

### Phase 4 — Object dictionary and SDO commissioning

**Goal:** inspect live object dictionaries and read/write SDO values safely.

Work:

- Attempt SDO Information in WL-7 and implement object/subindex listing and inspection from ESI metadata plus online CoE SDO Information where supported; if IgH 1.6.13 cannot provide it, record the capability gap and keep the ESI-only view usable with the absence reported (D-279).
- Merge ESI and online metadata by index/subindex with field-level provenance; surface conflicts instead of silently overwriting.
- Represent unknown or online-only objects without assuming ESI completeness; keep them inspectable and explicitly readable/writable in setup mode, but never PDO-mappable or auto-exposed as channels.
- Implement typed SDO reads/writes in setup mode, including conversion, access checks, abort/error decoding, timeout and state diagnostics. SDO failures use `STC-E-IO-ETH-030`–`039` with the D-280 payload, including object index/subindex, abort code, AL state and remedy; SDO Information unavailability is `STC-W-IO-ETH-009` and leaves the ESI-only view usable.
- Distinguish read-only inspection and explicit live commissioning writes.
- Add CLI commands for object listing, object inspection, SDO read and SDO write.
- Test with mocks and available hardware; document unsupported device/master capabilities.

**Exit criteria:** live values can be browsed/read inside setup mode; ESI and online metadata merge
with visible provenance and conflicts are reported; online-only objects are inspectable and
writable but never mappable; supported writes are explicit and produce structured results; errors
and access/state restrictions are diagnosable; no commissioning write silently changes project
configuration; SDO execution is proven off the cycle thread and unavailable while a PLC runtime owns
the master; SDO Information is attempted and its availability or capability gap is recorded.

**Evidence:** SDO codec/abort/state tests, mock and HIL read/write logs, the SDO Information
availability or capability-gap record, the emitting-test/traceability record for every
`STC-E-IO-ETH-030`–`039` and `STC-W-IO-ETH-009` code introduced here, and a note on which objects
are readable only via SDO Information and which remain ESI-only.

#### Phase 4 record (2026-11-28)

**Object dictionary and provenance (D-272).** `stc-runtime::ethercat::sdo` merges
ESI-declared metadata with online CoE SDO Information by index/subindex. Online
values win where the connected device reports them, ESI values fill the gaps,
and every disagreement is kept as a `name`/`type`/`access` conflict instead of
being silently overwritten. Online-only objects are marked `online_only`; they
stay inspectable and are never PDO-mappable or auto-exposed as channels. The
ESI remains the sole mapping-eligibility source (D-269).

**Reader correction.** The phase-1 ESI reader stored `<SubItem>` entries with
object index `0x0000` because a sub-item declares only its subindex; the
enclosing `<Object>` index is now inherited when the object closes. This is
what makes `0x1018:1`-style dictionary addressing correct; the phase-4
dictionary tests pin it.

**Typed values and access (D-272, D-280).** `SdoType` maps the CoE type names
the accepted subset models (`BOOL`, integer types, `REAL`/`LREAL`, strings and
`OCTET_STRING`); record types such as `IDENTITY` stay raw bytes rather than a
guessed width. Encoding is little-endian at the declared width, decoding rejects
a length that disagrees with the type, and access rights parse `ro`/`rw`/`wo`.
Reads treat unknown access as readable; writes fail closed unless the metadata
explicitly states read/write.

**API and CLI.** The typed API adds `object_dictionary`, `sdo_read` and
`sdo_write`. Transfers require an active setup session that owns the master, so
they run on the commissioning path, never on a PLC cycle thread, and a runtime
owner is an ownership conflict. A write is an explicit commissioning action:
the result always reports `persisted: false`, and a read-back is compared when
available; an unavailable read-back is reported unverified, never a match. The
CLI adds `sdo list|inspect|read|write`, all taking `--config`; list/inspect work
offline from the ESI, and live read/write fail closed on a host without the ABI
2 shim (`STC-E-IO-ETH-043`) or outside setup mode (`041`).

**Diagnostics (D-280).** `STC-E-IO-ETH-030` (object unavailable), `031` (CoE
abort, with the abort code in the payload), `032` (timeout), `033` (state
refusal), `034` (access denied), `035` (value/read-back mismatch) and `036`
(unavailable read-back) are implemented, plus `STC-W-IO-ETH-009` (SDO
Information unavailable, ESI-only view usable). The structured payload gained
`index`, `subindex`, `abort_code` and `al_state`. Every introduced code has an
emitting case in `ethercat_sdo.rs`, and the now-implemented `030`/`W-009`
allowlist entries were removed; traceability reports 290/290 covered.

**Host boundary.** The ABI 2 shim binding remains Phase 7 work (D-279), so the
CLI host reports no capabilities and every live SDO transfer fails closed with
`STC-E-IO-ETH-043` naming `sdo-upload` or `sdo-download`. All live behavior is
exercised through `MockEngineeringHost` on every CI host:
`crates/stc-runtime/tests/ethercat_sdo.rs` (10 tests) and
`crates/stc-driver/tests/ethercat_sdo_cli.rs` (5 tests), plus the added CLI
parse assertions.

**IgH 1.6.13 prototype record — executed on the `ecat` bench (2026-11-28).**
`tools/ethercat/phase4_probe.c` was built with `-Wall -Wextra -Werror` against
the installed userspace and run as the login user in the `ethercat` group; no
`sudo` and no privileged wrapper were used. It attempted SDO Information and
CoE uploads of the identity, assignment and TxPDO mapping objects, plus one
download of the identity object's current vendor-id value.

| Fact | Call | Observed on `ecat` | Consequence |
| --- | --- | --- | --- |
| Identity and CoE capability | `ecrt_master_get_slave` | rc 0; position 0, vendor `0x00000b95`, product `0x00010200`, revision 1, PREOP (`al_state=2`), `sync_count=3`, `sdo_count=0` | The SII signal already says this slave has no CoE SDO dictionary. |
| SDO Information | userspace `ecrt` entry point; `ethercat sdos` | IgH 1.6.13 exposes no userspace SDO Information call; `ethercat sdos` printed nothing and exited 0 | SDO Information cannot be attempted through this host stack. The API reports `STC-W-IO-ETH-009` and keeps the ESI-only view usable (D-272, D-279). |
| SDO upload | `ecrt_master_sdo_upload` of `0x1018:0/1/2`, `0x1C12:0`, `0x1C13:0`, `0x1A00:0/1` | rc `-93` (`EPROTONOSUPPORT`), `result_size=0`, abort `0x00000000` for all seven objects | The DIEWU has no mailbox and no CoE, so live values cannot be read. This is a device property, not an implementation gap. |
| SDO download | `ecrt_master_sdo_download` of `0x1018:1` | rc `-93` (`EPROTONOSUPPORT`), abort `0` | Commissioning writes cannot be exercised on this bench; the ABI 2 shim must gate them on SII/mailbox capability and fail closed. |
| Command-line confirmation | `ethercat upload 0x1018 1 --type uint32`; `ethercat download 0x1018 1 --type uint32 2008` | `Failed to upload SDO: Protocol not supported` / `Failed to download SDO: Protocol not supported`, exit 1 | Independent confirmation of the same limitation outside the probe. |
| Master after the probe | `ethercat master`, `ethercat slaves` | `Phase: Idle`, `Active: no`; slave `0 0:0 PREOP +` | The probe released the master; the bench is unchanged. |

**Environment and evidence.** Host `ecat-host`, kernel `6.12.115-ecat-rt`, IgH
1.6.13, one DIEWU slave in PREOP before and after. Logs stay on the host under
`~/stc-wl7-phase4/logs/`:

| File | SHA-256 |
| --- | --- |
| `phase4_probe.c` | `ac08d6077070f7148fa64f712e9d687ff88899f711a0935b087107df39f076ee` |
| `phase4_probe` | `4a09ef2e4086ed7599028cba61f1473edaf3951da075e95e16790936bc5c3065` |
| `probe-20261128.log` | `02cf9d70539861b2629d398be9b6a670ef2e7f12e1fafc74bc036996dfa18848` |
| `ethercat-sdos-20261128.log` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` (empty) |
| `ethercat-upload-20261128.log` | `a52ee470e2de5844b3ecc59dc10291f8a69709cc030d041212675e7ac8101384` |
| `ethercat-download-20261128.log` | `221cc720444deb9f49eb63b31f4bebd5c42592504891de1540e95a6810e4ac68` |
| `master-after.txt` / `slaves-after.txt` | `f621990fdf8a1905f6a46911983777373d9e0011384dc27307bf65027228b741` / `4f94d5060a0386cc9a684443117919cd69b82ea5aeaf55a45de6cc94c1ccdbe0` |

**Owner acceptance — pending.** Phase 4 contains live-hardware evidence, so the
workflow requires owner acceptance before Phase 5. The recorded result is the
same mailbox-less DIEWU limitation as Phase 3: no live SDO value was read or
written, and none is claimed. The implementation keeps the limitation honest
(SDO Information unavailable is a warning, transfers fail closed rather than
fabricating a value), and a CoE-capable slave can repeat the read/write cases
without changing the implementation.

**Local checks (2026-11-28).** `cargo fmt --all`, `cargo clippy -p stc-runtime
-p stc-driver --all-targets -- -D warnings`, the phase-4 API and CLI suites,
the phase-1–3 EtherCAT suites, `cargo xtask traceability --check` (290/290
covered), `cargo xtask host-audit --check`, `python tools/ci/check_docs.py` and
`git diff --check` all pass. The branch has not been pushed and no CI run exists
yet, so Phase 4 is **not accepted** and Phase 5 must not start until CI is green
and the owner accepts the capability record.

### Phase 5 — Startup SDO sequences and configuration persistence

**Goal:** persist desired device settings and make them reproducible at runtime.

Work:

- Define an ordered generic startup-operation model with typed values, state/preconditions, timeout, failure policy and read-back verification; every persisted operation is required and best-effort only controls verification (D-274). Generic startup operations do not own PDO assignment/mapping objects; PDO configuration is a separate declarative source applied through the master's native PDO path (D-269, D-274).
- Provide an explicit way to save a commissioning value as a startup setting; never persist a live write implicitly.
- Define project configuration schema and deterministic serialization/canonicalization.
- Model the canonical PDO configuration and its inspectable diagnostic representation separately from the generic startup list; do not persist a user-editable expanded SDO sequence.
- Persist the exact vendor/product/revision identity, dense per-master positions and any declared serial/alias for each device; retire the former fatal `startup-timeout` from the persisted schema (D-277, D-278).
- Persist the effective selection, declared-entry remap and the constraint facts needed for offline compiler validation; keep the project authoritative without the ESI.
- Persist the selected mode and any stored alternates as named per-device bundles; project the selected mode's effective PDO tree, channel exposure, startup plan and DC into the compiler-facing profile, and cross-check both surfaces (D-263, D-275).
- Persist the selected DC op-mode identity and effective values, including factor-resolved cycles, with the device-level op-mode facts; project the compiler-facing `dc` fields and validate them offline (D-276).
- Implement policy-free desired-vs-observed configuration diff with `match`/`different`/`unverified`/`not applicable`, and explicit capture of selected observed values; a diff never rewrites identity or topology (D-278).
- Validate consistency between the selected mode, generic startup SDO operations, PDO mapping and DC settings, including explicit mode and DC selection, factor-resolved DC values, alternate inertness, per-mode channel exposure and agreement with the compiler-facing projection; validate the effective PDO configuration against observed assignment/mapping where supported (D-275, D-276).
- Add CLI config show/validate/diff/capture/save operations.

**Exit criteria:** desired configuration including the declared-entry remap and constraint facts is
fully project-persisted and reviewable offline; identity and positions are exact, dense and
serial/alias are optional hard checks; `startup-timeout` is absent from the persisted schema; the
mode selection is explicit, alternates are inert
until selected, and the compiler-facing projection matches the engineering file; the DC choice and
effective values are persisted and validated, with free-run explicit and no timing override; generic
startup operations are ordered, required and validated; the canonical PDO configuration is applied
through the native PDO path and its effective result is inspectable and verifiable; generic startup
operations do not own or hand-author PDO assignment/mapping objects; a live commissioning value is
not confused with a saved desired value; the configuration model from Q9/Q24 is implemented without
unknown-key regressions in `stc-manifest`.

**Evidence:** schema/canonicalization tests, round-trip persistence tests for identity, positions,
optional serial/alias, the selection, mode
bundles, DC selection/effective values and constraint facts, factor-resolution tests,
profile-projection cross-check tests, profile-parser tests for every new key, a retired-key test for
`startup-timeout`, and the manifest fuzz seed update.

### Phase 6 — Compiler/artifact integration

**Goal:** compiler consumes the persisted configuration without performing engineering discovery or ESI resolution.

Work:

- Integrate the explicit desired configuration with existing manifest/profile validation and artifact embedding.
- Validate the persisted topology: unique dense positions per master, exact vendor/product/revision present, and serial/alias optional; reject wildcards, revision ranges and slot gaps (D-277).
- Validate logical channel mappings against the selected effective PDO process image, including the declared-entry remap and the channel-exposure rules: entry fit, direction, slice alignment and overlap, per-device name uniqueness, and one-to-one channel mapping (D-273).
- Validate the persisted selection against the persisted constraint facts (declared-entry eligibility, fixed/mandatory/exclusion) without reading ESI or re-running engineering selection.
- Embed the effective runtime configuration and relevant identity/provenance data.
- Include every persisted EtherCAT configuration input — topology, selected mode, PDO assignment/mapping and declared-entry remap, startup operations, DC values, persisted constraint facts, and the ESI source reference and identity — in the effective-configuration hash; any EtherCAT configuration change forces a cold download (D-267, Q26).
- Ensure project builds are deterministic and do not require live hardware, TwinCAT or vendor ESI directories once the desired configuration is persisted.
- Preserve existing static-profile projects and add migration/compatibility diagnostics where needed.
- Ensure effective process-image/startup changes participate in existing online-change/cold-download compatibility checks.

Add the build-time channel/PDO width diagnostic required by the review (Q25): a new code in the
project/manifest family, its conformance-profile row, and traceability evidence; the runtime
`STC-E-IO-ETH-007` keeps its cold-start meaning.

**Exit criteria:** compiler validates but does not select, including dense positions and exact
identity, the persisted constraint facts,
post-remap channel widths, and channel-exposure rules (slice fit, alignment, overlap and name
uniqueness); `startup-timeout` is rejected as a retired key; artifact contains all runtime-required
configuration; same inputs produce reproducible
output; existing static-profile regression suite remains green; the new build-time diagnostic has a
failing fixture and a passing fixture.

**Evidence:** driver build tests (valid/invalid), identity/dense-position validation tests,
retired-key tests, constraint-fact and remap compile tests, metadata
inspection, cold-download/hash tests for every persisted EtherCAT configuration input (Q26), and
updated CLI/manifest and conformance documents.

### Phase 7 — Runtime application and verification

**Goal:** runtime applies the saved desired configuration, isolates every device that does not match or fails, and lets healthy devices and PLC cycles continue.

Work:

- Implement ABI 2 as the only supported shim ABI and retire ABI 1 with no dual loader, fallback or legacy verification-unavailable path: carry forward the lifecycle/configuration/registration/status/cycle surface WL-7 needs, and add position-authoritative identity/discovery, PDO configuration/read-back, SDO upload/download and startup-SDO queuing, DC read-back where supported, per-device domain/WC status, capability reporting and structured results (D-279).
- Enforce exclusive managed-master ownership.
- Consume only the selected mode's projected PDO/channel/startup/DC configuration; never select, switch or infer a mode at runtime (D-275).
- Preflight identity for every configured position before any configuration write: exact vendor/product/revision, plus a declared serial/alias as a hard check; SII is the baseline and `0x1018` corroborates when readable. A mismatch means zero writes to that device, with no lazy matching, fallback or rebind (D-277, D-278).
- Queue the ordered generic startup SDOs through the master's supported startup SDO mechanism at their declared states. Every persisted operation is required; verification follows `none`/`best-effort`/`required`, and a required failure or unavailable required read-back isolates that device (D-274, D-278).
- Configure PDOs and DC according to the persisted selection through the master's native PDO configuration and process-data registration path, applying the declared-entry remap and verifying the effective assignment/mapping against observed state.
- Apply the selected DC choice through `ecrt_slave_config_dc`; free-run means no DC configuration; never select or switch DC at runtime; compare live DC read-back where the master exposes it and otherwise report `unverified`, never `match` (D-276).
- Do not hand-author PDO assignment/mapping objects as generic startup SDOs; document and enforce the ordering the master actually guarantees between native PDO configuration and generic SDOs.
- Activate the master after the matched devices' configuration is queued. Master open or activation failure is fatal. Once the master is active, report the backend ready and start PLC cycles without waiting for every slave (D-278).
- Isolate failures per device: a device that fails identity, configuration, startup, DC or OP does not enter OP, its outputs are not applied, and its inputs are held and marked stale while other devices continue. No rollback is claimed for partial writes.
- Retire `startup-timeout` as a fatal gate. Reconnect support applies only to devices configured at activation; a startup-offline device stays offline until restart or cold download unless Phase 3 proves a safe runtime reconfiguration path. Hot-connect groups stay out of WL-7.
- Keep the process image fixed for the build: skipped or failed devices do not shift or corrupt healthy channel offsets. Use per-device domains or an equivalent fixed-image approach, and add status layout 2 with `slave<N>.input_stale`; global `input.stale` is a summary (D-278).
- Reject a library that fails the ABI 2 name/version/symbol handshake, including an ABI 1 library, with `STC-E-IO-ETH-001`; a master-ownership conflict, a write outside setup mode or a missing required ABI 2 capability uses the `STC-E-IO-ETH-040`–`049` family. Report capability gaps and unavailable read-back as `unverified`, never `match`, and never parse human text for control flow (D-279, D-280).
- Emit runtime degraded diagnostics from `STC-W-IO-ETH-004`–`019` on state transitions only, never every cycle: device isolation is `STC-W-IO-ETH-004`, missing DC reference `006`, extra slave `007`, downstream shift `008`, SDO Information unavailable `009`, and required read-back/capability unavailable `010`. Per-cycle truth stays `slave<N>.input_stale`; global `input.stale` is a summary. The shim returns structured facts and capabilities, never diagnostic codes (D-278, D-280).
- Extend mock shim and runtime tests for success, identity mismatch with zero writes, device offline at activation, partial chain where a present non-OP slave forwards, downstream position shift, excess slave, missing DC reference, SDO abort, timeout, state failure, ownership conflict, remap refusal, remap read-back unavailable, best-effort versus required verification, per-device stale inputs, and other unsupported read-back.

**Exit criteria:** PLC cycles start after master activation even when configured slaves are missing
or mismatched; master open/activation failure remains fatal; a mismatched or offline device gets no
configuration writes and stays out of OP while healthy devices continue; the process image does not
shift; status layout 2 reports per-device `slave<N>.input_stale`; a remap that cannot be read back
fails closed for that device; runtime never selects, switches or infers a mode; diagnostics show
expected and observed state per position; an ABI 1 library is rejected with an explicit
ABI 2-required diagnostic; runtime degraded diagnostics emit once per state transition and every
introduced code has an emitting test and a traceability entry; the fixed
privileged wrappers from Phase 2 (Q23) are exercised by the runtime tests.

**Evidence:** shim ABI 2 handshake/capability/structured-result tests, ABI 1 rejection tests, mock
identity-mismatch/zero-write/offline-at-activation/partial-chain tests, per-device stale and layout-2
status tests, abort/timeout/ownership/remap-refusal tests,
startup-ordering tests, DC apply/free-run/read-back-unavailable tests, transition-emission tests for
`STC-W-IO-ETH-004`–`019`, the emitting-test/traceability record for the ABI/ownership and runtime
families, and the runtime section of the verification report.

### Phase 8 — Corpus, simulator, HIL and acceptance evidence

**Goal:** demonstrate the engineering workflow with synthetic, vendor-format and hardware evidence.

Work:

- Run the local TwinCAT corpus harness and record accept/reject counts by diagnostic and file-size band; investigate unexpected outcomes.
- Record the ASIX AX58100 ESI download link and vendor-file handling in the EtherCAT validation runbook; do not commit or redistribute vendor ESI files (D-264, Q18).
- Run a longer ESI fuzz campaign and record executions, coverage and artifacts.
- Test offline topology/configuration without hardware.
- Test online discovery, SDO browsing/read/write, configuration diff and apply using simulator/mock coverage and available hardware.
- Keep the existing DIEWU static-profile project as regression baseline; create a parallel ESI/configuration-driven project and compare I/O behavior.
- Run HIL on `ecat`; record host, OS, IgH version, shim ABI, device identity, ESI provenance, desired config, commands and results.
- Include deliberate negative cases: wrong identity/revision, a mismatched device receiving zero configuration writes, a device offline at activation, a present non-OP slave forwarding to a later healthy position, a missing slave shifting downstream positions, an excess slave, a missing DC reference clock, PDO mismatch, an object-dictionary-only mapping attempt, a fixed/mandatory/exclusion violation, a live remap refusal, failed startup SDO, per-device stale inputs, and a configured-slave disconnect/reconnect, where hardware permits.

**Exit criteria:** evidence covers offline and online engineering workflows; the hand-written and
engineering-configured bench projects produce equivalent I/O behavior; HIL results and unsupported
cases are documented, including the single-slave bench limitation for partial-chain behavior; no
safety or hard-real-time claim is added.

**Evidence:** corpus tally by code and size band, fuzz campaign output, simulator results,
equivalence comparison, HIL logs with host/OS/IgH/ABI/provenance, partial-chain simulator records,
and the negative-case records.

### Phase 9 — Documentation, walkthrough and PR

**Goal:** publish the durable engineering contract and obtain owner acceptance.

Work:

- Update `I/O Routing and Backends.md`, `I/O and Devices.md`, `CLI and Manifest.md`, `Decisions.md`, `USER-GUIDE.md` and `KNOWN-LIMITATIONS.md`.
- Document the engineering API contract, CLI command reference, offline/online workflows, master ownership and idle-phase inspection, SDO semantics, startup sequencing, ESI subset and declared-entry remap boundaries.
- Publish the D-280 diagnostic catalog: families, severities, owning layer, structured payload, transition-emission rule and representative mappings. State that codes are stable and append-only, `STC-E-IO-ETH-008` stays retired/reserved, and human text is never parsed.
- Add `specification/Verification/ESI-Configuration.md` or a suitably renamed EtherCAT engineering verification document with corpus, fuzz, simulator and HIL evidence.
- Do not change `lean/verification-status.toml`: WL-7 changes no theorem (D-268, Q27).
- Run an owner walkthrough covering: offline topology creation; ESI inspection; PDO/mode selection;
  setup-mode entry and exit; live scan; SDO browse/read/write; explicit save of startup values;
  config diff/apply; runtime startup and mismatch handling. Remote invocation is not a WL-7
  walkthrough item (D-259).
- Open PR against `v3.0.0-planning` with summary, exclusions, verification evidence, CI links and known limitations.

**Exit criteria:** durable docs current (including the conformance profile, `Architecture.md` crate
map, `USER-GUIDE`, `KNOWN-LIMITATIONS`, the published diagnostic catalog, and the traceability/host
allowlists); the manual
walkthrough is approved; PR CI is green; the branch is not merged or deleted and WL-7 is not marked
complete by an agent without owner approval.

**Evidence:** documentation commits, the verification report, the diagnostic-catalog review, the
walkthrough log, the PR link, and the PR CI run.

## 11. Test strategy

Tests must cover the engineering model as well as the parser and runtime:

- ESI fixtures: valid, malformed, unsupported, ambiguous, multiple devices/files/revisions, ISO-8859-1, large bounded inputs, vendor extension noise and sidecar roots.
- Deterministic resolution independent of filesystem enumeration order.
- Offline topology and configuration editing with no hardware or ESI installation present after import/resolution.
- Bus scan and observed-state model using mock/simulator and HIL.
- PDO alternatives, `Fixed`/`Mandatory`/`Exclude`/`ExcludedSm` constraints, `PdoAssign`/`PdoConfig` gates, declared-entry remaps (accept, reject, move within a direction, reorder, omit optional), attempted object-dictionary mapping rejection, multiple sync managers/PDOs, disabled SMs, non-byte-aligned widths, unsupported mappings, the canonical PDO configuration versus its generated diagnostic view, and rejection of hand-authored PDO objects in the generic startup list.
- Named mode bundles: explicit selection, per-device unique names, duplicate-name rejection, alternates stored but inert until selected, missing or ambiguous selection rejection, provenance ignored for behavior, no vendor motion semantics, startup/PDO/DC consistency, per-mode channel exposure, a switch that leaves a bound channel unexposed rejected, profile-projection cross-check, and cold-download/hash behavior on selection or content change.
- DC op modes: explicit selection, no first-op-mode default, free-run as an explicit choice, duplicate-name disambiguation, `AssignActivate` immutability, Sync0/Sync1 timing and factor resolution, unresolvable factor and invalid-combination rejection, no-DC-device rejection, no timing overrides, persisted effective values, live read-back comparison and `unverified` when unavailable.
- Compiler validation of the persisted selection and constraint facts without ESI access, including post-remap channel width and binding consistency, default full-entry channels, explicit bit slices, byte-alignment and overlap rejection, and duplicate-name rejection.
- SDO typed values, subindices/records, read-only objects, access denial, abort codes, timeouts, state preconditions, ordered operations, required-operation failure, `none`/`best-effort`/`required` verification and read-back.
- Native PDO configuration and generic startup SDO ordering/lifecycle, including what the master actually guarantees, unsupported ordering, effective PDO configuration inspection/verification against observed assignment/mapping, and failure/recovery behavior.
- Desired/observed diff semantics: `match`, `different`, `unverified` and `not applicable`; unavailable read-back is never `match`; every position is evaluated independently; comparison never rewrites identity or topology.
- Identity preflight: exact vendor/product/revision, unique dense position, optional serial/alias as hard checks when declared, SII baseline with `0x1018` corroboration, zero configuration writes on mismatch, and revision/source-reference change forcing re-import and a cold download.
- Shim ABI 2: name/version/symbol handshake, capability report, structured-result fields, position-authoritative lookup with a verified alias, PDO/DC read-back availability, SDO Information availability, per-device WC, and rejection of an ABI 1 library with an explicit ABI 2-required diagnostic.
- Runtime startup and degraded isolation: PLC cycles start after master activation even with missing slaves; a failed device stays out of OP with no output application and held stale inputs while healthy devices continue; a present non-OP slave forwards, a missing slave shifts downstream positions, an excess slave and a missing DC reference are reported; no free-run fallback or topology repair; a startup-offline device does not reconnect; layout 2 `slave<N>.input_stale` and the global `input.stale` summary.
- Master ownership conflicts, idle-phase inspection restrictions, and setup-mode-only writes.
- Diagnostics: every introduced code has an emitting test and a traceability entry; `STC-E-IO-ETH-008` is never emitted; `E` rejects the operation or fails run start and `W` continues degraded; structured payload fields (device/position, operation, object, expected/observed, AL state, abort code, capability, remedy) are populated; degraded runtime codes emit on transitions, not every cycle; control flow never parses human text; the shim returns facts/capabilities only and no codes.
- CLI behavior through the same API, including invocation-context independence (no TTY, cwd, or
  terminal-format assumptions), since the remote/UI layer calls the same operations (D-259).
- Runtime startup ordering and device-scoped fail-closed behavior.
- Regression of existing static DIEWU profile, status layout, online-change behavior and current CI matrix.

## 12. Dependencies and risks

- WL-5.2 provides the initial IgH backend and DIEWU bench, but its current ABI 1 shim exposes no identity, PDO read-back or SDO calls. ABI 2 is a new sole ABI that must carry forward the lifecycle/cycle/status behavior WL-7 needs and add the engineering surface; prototype the capability matrix early rather than assuming IgH support (D-279).
- ESI is vendor-extended XML. The subset, constraints and rejection policy are a primary risk; the measured local corpus is the mitigation and evidence source.
- Object dictionaries are not guaranteed to be complete in ESI. Online CoE discovery and unknown-object handling are required for a useful SDO browser. SDO Information availability is not assumed: Phase 4 attempts it, and if IgH 1.6.13 cannot provide it the ESI-only dictionary remains the fallback and the capability gap is reported (D-272, D-279).
- SDO writes may depend on EtherCAT state, sequence and vendor-specific behavior. The generic startup-operation model must not pretend all writes are interchangeable.
- Some PDO selections may require vendor-specific mode-selection SDOs before the mapping is applied. Configuration consistency between operating mode, generic startup SDOs and the PDO map must be validated. PDO assignment/mapping objects are authored only under the PDO configuration path; the native PDO API governs their application, and generic startup SDO ordering is limited to what the master actually guarantees (D-269, D-274).
- The compiler-facing profile projection is derived from the selected mode; a stale or hand-edited projection could diverge. Cross-check both surfaces, reject disagreement, and force a cold download on any selection or content change (D-263, D-275).
- DC op-mode `Factor` resolution depends on the configured runtime cycle, so a cycle change can alter the applied Sync0/Sync1 timings. Resolve and persist effective values explicitly, detect drift at build, and require re-selection or a cold download rather than silently re-scaling (D-267, D-276).
- Declared-entry remapping depends on both ESI legality and live support. A device may accept the ESI declaration but refuse the remap at the `PdoAssign`/`PdoConfig` or SII level; runtime must detect this, fail closed for that device, and never silently fall back to the ESI default. Remap-capable hardware may not be available, so synthetic/mock evidence must be explicit.
- Degraded operation requires per-device identity, health, validity and input qualification. The current runtime is global: startup waits for every slave and fails the run, `is_healthy` requires all slaves plus a complete working counter, and a fault stops every input copy. Phase 7 must replace those with per-device state while keeping the process image fixed, or healthy devices will be corrupted or frozen by one failure (D-278).
- Existing HIL evidence is single-slave; partial-chain behavior, a present non-OP slave forwarding, downstream shifts and excess-slave reporting need simulator/mock coverage, with any multi-slave HIL evidence recorded separately.
- IgH queues slave configuration before master activation. A device absent at activation cannot be configured later through the current shim, so a startup-offline device stays offline until restart or cold download; runtime reconfiguration and hot-connect groups are not WL-7 work.
- Degraded per-device isolation is not a safety function and does not make partial operation safe. Continuing the PLC while devices are failed is diagnostic behavior only; a future strict policy or machine-level safety function is out of WL-7 scope.
- A live master may be owned by `stc run`, `stc serve`, or another `stc` engineering session. Setup-mode entry fails closed with an ownership-conflict diagnostic; no automatic stop. A master owned by a non-IgH stack is out of scope.
- Engineering is invocation-context independent and the remote UX follows WL-7; when that layer
  is built it reuses the existing SSH behavior, with no parallel remote-control mechanism.
- ABI 2 is the only supported shim ABI; ABI 1 is retired with no dual loader or fallback. The ABI 2 surface must carry forward the lifecycle/cycle/status behavior WL-7 needs, and its capability/result contract must be proven on IgH 1.6.13 before Phase 4/7 estimates are relied on. An unavailable required call is `unverified` or a device-scoped failure, never `match` (D-279).
- HIL currently covers only the DIEWU 16I/16O bench. Servo-drive mode-selection behavior should be tested with synthetic/mock fixtures unless a suitable drive is available; do not claim general vendor-drive support from DIEWU evidence.
- Any EtherCAT configuration change forces a cold download, including a declared-entry remap or an ESI source-reference/identity change. Preserve existing online-change safeguards and produce a clear diagnostic.
- No safety or hard-real-time claim is made.
- Diagnostic codes are a public contract. An implementation could emit ad-hoc codes, reuse the retired `STC-E-IO-ETH-008`, over-report degraded state every cycle, or parse human text. D-280's stable append-only families, reserved ranges, transition-emission rule, emitting tests and traceability entries are the guard; a new need takes the next number in the owning family rather than redefining an existing code (D-280).

## 13. Phase 0 decisions to record

These are the decisions that must be answered before Phase 1 starts. The agreed architecture in §2
is not reopened by them. Answers become `D-259` and following in `specification/Decisions.md`.
Questions 19–28 come from the branch review in §4.1 and were not visible in the first draft.

Required answer set: **all 28 questions**. A question whose answer is "deferred" is still a
decision, and it changes the phase where the deferred work appears.

1. **Parser:** project-owned bounded reader or vetted crate such as `quick-xml`? **ANSWERED 2026-11-15 (D-261).** Use a vetted crate; evaluate `quick-xml` against the local ESI corpus first and record the result before locking the selection. The limits, fuzzing and no-external-resource policy are unchanged either way.
2. **ESI versions:** accept only `EtherCATInfo Version="1.2"`, or include 1.0/1.1? **ANSWERED 2026-11-15 (D-262).** Accept only version 1.2; reject 1.0/1.1 with an explicit diagnostic and track them as WL-24 in `FutureWishlist.md`.
3. **ESI source/library model:** per-device explicit source plus project-wide default library; exact TOML keys and package-relative path behavior to be specified. **ANSWERED 2026-11-15 (D-263).** Per-device explicit source plus project-wide default library; exact TOML keys and package-relative path behavior are specified with the Phase 1/5 schema work.
4. **Revision policy:** exact revision pin required for production or highest revision as a convenience default? **ANSWERED 2026-11-15 (D-263).** Support an exact pin and make unpinned selection deterministic and conspicuous. The ESI is used to create the initial project TOML setup; once written, the project TOML files are authoritative and ESI files no longer matter. Production validation therefore validates the persisted project configuration, not an ESI pin; re-import is explicit.
5. **PDO constraints:** exact handling of `Fixed`, `Mandatory`, `PdoAssign` and `PdoConfig`, including what can be selected offline versus confirmed online. **ANSWERED 2026-11-15 (D-269).** Use constrained selection plus declared-entry remapping: select ESI-declared PDOs/SMs and remap only from entries the ESI declares for the same device/direction; omit optional entries, reorder, or move them between declared PDOs of that direction; never synthesize PDOs, edit entry identity/width, or map object-dictionary-only objects. Enforce `Fixed`, `Mandatory`, `Exclude` and `ExcludedSm` as hard constraints, treat missing flags as schema defaults, combine ESI and live capabilities as the most restrictive union, persist the effective selection and constraint facts for offline compiler validation, and fail closed for that device when a live device refuses a remap (D-278). The user-authored generic startup list does not own PDO assignment/mapping objects; PDO configuration is the sole authoring source and is applied through the master's native PDO path, with an inspectable diagnostic view of the equivalent CoE operations (D-274). Option C (arbitrary object-dictionary mapping) is deferred indefinitely with no wishlist entry because it has no demonstrated need and the bench cannot exercise it.
6. **Operating-mode representation:** generic named mode plus its selected PDO configuration and required startup operations; avoid vendor-specific motion semantics in the generic schema. **ANSWERED 2026-11-21 (D-275).** A mode is an engineer-authored, named, per-device bundle stored in the dedicated EtherCAT engineering configuration file: a stable mode name unique per device plus optional description; the canonical PDO configuration including the effective declared-entry remap; the mode's effective channel exposure; the ordered, required generic startup plan, including any vendor mode-selection SDO as ordinary startup data; the explicit DC selection (D-276, Q15); and optional ESI DC op-mode/vendor provenance recorded for review only. Exactly one mode is selected per device at a time; alternates may be stored for review and switching, but selection is always explicit — no "first ESI DC op mode" default and no inference from observed device state. Switching is explicit, changes the effective configuration and forces a cold download (D-267). Mode identity is the engineer-authored name, not the ESI DC op mode; two modes that share PDOs but differ in remap, channel exposure, startup or DC are distinct. The generic schema carries no vendor-specific motion semantics, drive-mode enumerations, units or motion-profile fields; vendor meaning lives in names/descriptions and authored startup values and is never interpreted. The PDO/sync-manager set is validated under D-269, and optional provenance never constrains or widens it. The startup plan must be consistent with the selected PDO configuration under D-274 ordering limits; DC must be one of the device's ESI-declared DC op modes or an explicit free-run choice (D-276); channel exposure is per mode, and switching to a mode that leaves a bound channel unexposed is a configuration error. The desired engineering file is the authoring source for all stored modes; the compiler-facing machine profile carries the selected mode's effective projection (PDO tree, channel exposure, startup plan and DC) and is cross-checked against the engineering file, and the projection is not a second authoring source. Exact key spellings and the projection/cross-check mechanics are Phase 1/5 schema work.
7. **Object dictionary sources:** exact precedence/merge rules between ESI metadata, online SDO Information and unknown online objects. **ANSWERED 2026-11-20 (D-272).** Merge by object index/subindex with field-level authority and visible provenance: ESI is the offline baseline and sole PDO mapping-eligibility source (D-269); online CoE SDO Information is the live authority for the connected device's object existence, name, type, access and description when available and identity/revision matches; live reads are observed values; the project is desired/startup values. Conflicts are surfaced, not overwritten; a conflict on a mapped entry or persisted operation blocks configuration. Online-only objects are inspectable and explicitly readable/writable in setup mode but never PDO-mappable or auto-exposed as channels; startup persistence follows Q8. If SDO Information is unavailable, the ESI-only view remains usable and the absence is reported.
8. **SDO startup sequence schema:** operation types, value encoding, EtherCAT state preconditions, ordering, retries/timeouts, abort handling and read-back policy. **ANSWERED 2026-11-21 (D-274).** Generic startup SDOs form one ordered, deterministic, required plan. Operations are `write` and `verify`, with explicit index/subindex, explicit type/value (`bool`, `i8`–`i64`, `u8`–`u64`, `f32`/`f64`, `string`, wire-order `bytes`), explicit `PREOP`/`SAFEOP` state precondition, bounded timeout/retry and a stable id. Startup runs before OP; OP and backward state transitions are invalid. The first required failure stops the plan, reports device/operation/object/abort and prevents OP; no transactional rollback is claimed. Retries apply only to transport/mailbox-busy conditions, not SDO aborts unless a documented transient code is allowlisted. Verification is `none`, `best-effort` or `required`: mismatch always fails; unavailable upload is `unverified`, never `match`; best-effort only controls verification, and required plus unavailable fails closed. PDO configuration is a separate declarative source applied through `ecrt_slave_config_pdos`/process-data registration; the equivalent CoE operation view is diagnostic and non-editable, not a user-authored startup sequence. Ordering/dependencies are modeled where the master supports them; arbitrary interleaving is not assumed. Phase 3 records the actual lifecycle and investigates literal SDO-based PDO configuration only if the native API cannot support a required configuration or sequence.
9. **Project file layout:** extend machine profile or introduce a dedicated EtherCAT engineering configuration file; define canonical ownership and migration from static profiles. **ANSWERED 2026-11-15 (D-263, with Q24).** Add a dedicated engineering configuration file declared by a new `[io]` key; embed the effective process-image selection in the existing machine profile; define the cross-check and an explicit migration path for static profiles.
10. **Topology identity:** required and optional identity fields, handling of serial number/alias/position, and behavior when offline schematics lack a serial number. **ANSWERED 2026-11-21 (D-277).** Each configured device requires a unique dense position per master plus the exact vendor ID, product code and revision number; wildcards, revision ranges and product-family matching are not accepted. Serial number and station alias are optional instance checks: an offline schematic normally omits them, a declared value is a hard runtime check, and absence means no check. Position is the machine slot; serial/alias never substitute for position, cause automatic rebinding or allow a device to move slots. SII is the matching baseline; CoE `0x1018` corroborates when readable — a readable disagreement blocks that device, and unavailability is `unverified` when SII already established the required fields. Identity is checked before any configuration write. Default capture records vendor/product/revision and does not pin serial; serial/alias are captured only on explicit request. A revision or ESI source-reference change invalidates the mapping basis and forces re-import/review plus a cold download (D-267). Scan and compare never rewrite identity or topology.
11. **Master inspection boundary:** supported integration path and exact read-only capabilities when no application owns the master. **ANSWERED 2026-11-20 (D-271).** There is no externally managed master mode: a master owned by another application and a non-IgH master are out of scope. An idle local master (no owner) may be inspected read-only; any hardware-changing operation, including an SDO write, requires managed setup mode (D-260). SDO writes are normal setup activity, not forbidden; the read-only restriction applies only to inspection that does not own the master.
12. **Shim ABI:** ABI 2 scope, ABI 1 compatibility policy, and proof of which identity/PDO/SDO read-back calls are available at the required PREOP/SAFEOP stages. **ANSWERED 2026-11-21 (D-279).** ABI 2 is the only supported shim ABI; ABI 1 is retired with no dual loader, fallback or legacy "verification unavailable" path. ABI 2 carries forward the ABI 1 functionality WL-7 needs — master open/close/activate, slave configuration/registration, domain/process-data registration, status and cycle calls, and error reporting — and may retire unused surface. It adds position-authoritative identity/discovery (SII vendor/product/revision/serial/alias, position, online/AL state, `0x1018` corroboration), PDO configuration/read-back (master PDO tree plus live CoE upload of `0x1C12`/`0x1C13`/`0x1600`/`0x1A00` where supported), SDO upload/download and startup-SDO queuing, DC read-back where supported, per-device domain/WC status, capability reporting and a structured result model; the shim reports facts and the Rust backend computes `match`/`unverified`/`stale`. Position is the lookup key and a verified alias may be used only after position verification. SDO Information is attempted in WL-7; if IgH 1.6.13 cannot provide it, the capability gap is recorded and the ESI-only metadata view remains usable with the absence reported. Phase 3 records the IgH 1.6.13 capability matrix at PREOP/SAFEOP; unavailable required calls fail closed or report `unverified`, never `match`, and the ABI surface is amended if a required call cannot be made to work.
13. **ESI resource limits/encoding:** default file-size/depth/element limits and UTF-8/ISO-8859-1 handling based on corpus evidence. **ANSWERED 2026-11-15 (D-261).** Keep the proposed 128 MiB file-size cap to include the measured 70.6 MB file, plus structural depth/element/attribute/text limits; accept UTF-8 and ISO-8859-1 where corpus measurement confirms them.
14. **Channel exposure:** one channel per selected PDO entry by default; bit-level slicing only when explicitly declared. Confirm naming and duplicate-name behavior. **ANSWERED 2026-11-20 (D-273).** Expose selected entries only, with one full-entry channel as the default proposal and an entry optionally left unchanneled for layout. Explicit non-overlapping bit slices are allowed; the slice uses the declared channel width, must fit the entry, must match the PDO direction, and a slice wider than one bit must start on a byte boundary while one-bit slices may start anywhere. A whole-entry channel and a slice of the same entry overlap and are rejected. Names are engineer-authored, unique per device and matched exactly; ESI names are suggestions only, with a deterministic index/subindex fallback, and duplicate names are hard errors, never silent renames. Bindings target the effective entry after remapping, not display order.
15. **DC policy:** how ESI DC op modes are presented and selected; do not implicitly choose the first ESI DC op mode. Persist the engineer's explicit choice. **ANSWERED 2026-11-21 (D-276).** Every mode records exactly one explicit DC choice: one ESI-declared `<Dc><OpMode>` or `free-run`; there is no first-op-mode default and no inference from live state. Presentation lists every declared op mode with name, description, `AssignActivate`, Sync0/Sync1 cycle and shift, and any `Factor` resolved against the configured runtime cycle; duplicate names are disambiguated by declaration index. `AssignActivate` is never edited or synthesized; cycle/shift values are derived from the selected op mode and persisted as effective values; an unresolvable factor, invalid Sync0/Sync1 combination or out-of-range value fails closed. WL-7 does not allow arbitrary per-field timing overrides; if no declared op mode expresses the required timing, the configuration is rejected. A device whose ESI declares no DC section may prefill `free-run` at import, but the choice is persisted and reviewable; missing DC in a new engineering configuration is an error, not an implicit free-run, and the compiler-facing projection encodes free-run as no `dc`/no `assign-activate`. Declared op modes live in the device-level ESI facts so the compiler can validate offline; the mode stores the selected identity and effective values. Runtime applies the effective values through `ecrt_slave_config_dc` before activation, never selects or switches DC, compares live read-back where the master exposes it and otherwise reports `unverified`, never `match` (Q16). Changing the DC choice or any effective value forces a cold download (D-267); a remap alone does not change the DC selection.
16. **Mismatch policy:** fail closed for required identity, PDO and startup configuration mismatches; define which unavailable read-backs are warnings versus blockers. **ANSWERED 2026-11-21 (D-278).** Comparison is policy-free: every identity, PDO, startup and DC check reports `match`, `different`, `unverified` or `not applicable`, and unavailable read-back is never `match`. The WL-7 consequence is **degraded per-device isolation**: a mismatched, offline or failed device does not enter OP and its configuration data is not written; its inputs are held and marked stale, its outputs are not applied, and other devices plus PLC cycles continue. There is no lazy or best-effort matching, no fallback and no automatic rebind; identity mismatch means zero writes. A matched device that later fails may retain partial writes with no rollback. D-274, D-272 and D-269 "block" consequences are device-scoped, not run-scoped. Master open/activation failure remains fatal; once the master is active, the backend is ready even with missing slaves. A missing slave may break or shift downstream positions; every position is evaluated independently, with no renumbering or serial/alias fallback. `startup-timeout` is retired as a fatal gate, and PLC cycles start after master activation. Reconnect support applies only to devices configured at activation; a startup-offline device stays offline until restart or cold download unless Phase 3 proves a safe runtime reconfiguration path, and hot-connect groups stay out of WL-7. A missing DC reference clock, extra slaves and downstream shifts are reported with no free-run fallback and no automatic topology repair. Status layout 2 adds per-device `slave<N>.input_stale`; global `input.stale` becomes a summary, not the per-device validity gate. Strict mode remains a future policy, and no safety claim is added.
17. **Diagnostics:** stable ESI, topology, PDO, SDO, ownership and runtime-configuration diagnostic families. **ANSWERED 2026-11-23 (D-280).** Diagnostics are stable, structured and layer-owned. Existing `STC-E-IO-ETH-001`–`007` and `STC-W-IO-ETH-001`–`003` keep their meanings; `STC-E-IO-ETH-008` is retired/reserved by D-278 and never reused. New append-only families: `STC-E-ESI-001`–`009` (ESI import/capability), `STC-E-IO-ETH-009`–`019` (topology/compare), `STC-E-IO-ETH-020`–`029` (engineering mode/PDO/DC/channel configuration), `STC-E-IO-ETH-030`–`039` (SDO commissioning/startup), `STC-E-IO-ETH-040`–`049` (ownership/setup/ABI), and `STC-W-IO-ETH-004`–`019` (runtime degraded). `E` means the requested engineering operation/configuration is rejected or run start is fatal; `W` means the run continues degraded. Runtime device isolation is a `W` with the reason in the structured payload. The importer, engineering CLI/API, compiler, runtime and shim have distinct ownership; the shim returns facts/capabilities, never codes. Every diagnostic carries code, severity, device name/position, operation, object index/subindex, expected/observed, AL state, abort code, missing capability and remedy; human text is secondary and never parsed. Runtime degraded diagnostics emit on transitions, not every cycle; per-cycle truth is `slave<N>.input_stale` and `stc run -v`. `match` is not a diagnostic; `different`/`unverified` are structured compare results, and `unverified` becomes a warning when it affects a required check. Fixed representative mappings: ABI 1/handshake → `STC-E-IO-ETH-001`; master busy → `002`; activation → `005`; identity/serial/alias mismatch in engineering → `003`; runtime isolation → `STC-W-IO-ETH-004`; remap refusal → `STC-E-IO-ETH-022`; SDO abort/timeout/state → `031`–`033`; missing DC reference → `STC-E-IO-ETH-013`/`STC-W-IO-ETH-006`; extra slave → `STC-W-IO-ETH-007`; downstream shift → `STC-W-IO-ETH-008`; SDO Information unavailable → `STC-W-IO-ETH-009`; required read-back/capability unavailable → `STC-W-IO-ETH-010`. Every new code has an emitting test and a traceability entry; unused numbers in each range are reserved.
18. **Corpus/license:** confirm local-only TwinCAT corpus policy and whether the ASIX AX58100 sample ESI can be redistributed. **ANSWERED 2026-11-15 (D-264).** Keep ESI files local; do not commit or redistribute the ASIX AX58100 sample. Publish the vendor product/ESI download link <https://www.asix.com.tw/en/product/IndustrialEthernet/EtherCAT/AX58100> in the validation runbook so the engineer obtains the file directly from ASIX.
19. **Code placement and the unsafe policy:** keep all shim FFI in `stc-runtime` and put the
    engineering API there or in `stc-driver`, or add a new `stc-ecat` crate with its own
    documented `unsafe_code` exception? **ANSWERED 2026-11-20 (D-270).** Keep the engineering engine
    and all shim FFI in `stc-runtime` (the only crate allowed `unsafe`, D-077) and the CLI in
    `stc-driver`; no new crate in WL-7. A new crate would add packaging, xtask, host-audit, and
    doc-map work without a boundary benefit. ABI 2 calls stay under the existing documented
    exception, and the host-neutral API/mock remains the test boundary.
20. **Remote engineering execution — ANSWERED 2026-11-15 (D-259).** Owner: engineering is the same
    from `stc`'s perspective whether local or reached over SSH, exactly like the debug session that
    VS Code opens on a remote machine. Remote packaging (and GUI) comes after WL-7. **Decision: no
    remote transport, daemon, serve-session coupling, or remote UX in WL-7; engineering logic must
    be invocation-context independent.**
21. **Project sync for remote engineering — ANSWERED 2026-11-15 (D-259).** Not applicable in WL-7:
    there is no remote engineering workflow, so no SFTP/fingerprint requirement. The post-WL-7
    remote layer reuses the delivered WL-1.2 synchronization; the API contract (§6.1) must simply
    not depend on a developer-host working directory.
22. **Setup mode and ownership arbitration — ANSWERED 2026-11-15 (D-260).** Owner: EtherCAT
    engineering and SDO setup need a setup mode in which the PLC is not yet started. **Decision:
    engineering runs in an explicit setup mode that holds the master, runs no cycle, and drives no
    outputs; a reservation conflict fails closed and names the conflict rather than stopping a
    running PLC.** The session spans several CLI invocations, with explicit enter/leave and a
    `status` query; commands do not silently re-acquire the master per invocation. Interrupted or
    crashed sessions lapse when the OS device close releases the master, and the next invocation
    reports the recovery. Entry needs no extra confirmation flag beyond the explicit enter command.
    Setup-state diagnostics use the D-280 ownership/setup family (`STC-E-IO-ETH-040`/`041`).
23. **Privileged master access:** add fixed privileged wrappers for the engineering operations
    (extending `tools/ethercat/stc-wl52.sudoers` under review), or require the engineering CLI to
    run as the service/root user? **ANSWERED 2026-11-15 (D-265).** Add new fixed-command wrappers
    with argument validation and no generic passthrough, reviewed as a security change; document
    the run-as-root alternative.
24. **Configuration persistence surface:** extend the existing machine profile, or add a dedicated
    engineering configuration file declared by a new `[io]` key? **ANSWERED 2026-11-15 (D-263).**
    Add a dedicated file for the desired configuration with the effective process-image selection
    embedded into the profile the compiler already canonicalizes; record how the two are
    cross-checked and how a static profile migrates.
25. **Build-time channel width validation:** which diagnostic family owns it, and does the
    conformance profile gain a rule? **ANSWERED 2026-11-15 (D-266).** Add a new project/manifest-level
    code (not the runtime `STC-E-IO-ETH-007`), one conformance-profile row, and a failing fixture.
26. **Effective-configuration hash scope:** which inputs force a cold download when they change?
    **ANSWERED 2026-11-15 (D-267); clarified 2026-11-20.** Any EtherCAT configuration change of any
    sort forces a cold download. The hash covers every persisted EtherCAT configuration input:
    topology, mode, PDO assignment/mapping and declared-entry remap, startup operations, DC values,
    persisted constraint facts, and the ESI source reference and identity. There is no provenance-only
    category and no split; a change to any persisted input forces a cold download. ESI files are not
    read at build/runtime, so an ESI file edit alone does not change the project; a re-import that
    changes any persisted input does.
27. **Documentation and gate set:** which documents and registries change, and in which phase?
    **ANSWERED 2026-11-15 (D-268).** Phase 6/9 update `CLI-and-Manifest`, `IO-Routing-and-Backends`,
    `IO-and-Devices`, `01-Conformance-Profile`, `Architecture` crate map, `USER-GUIDE`,
    `KNOWN-LIMITATIONS`, the verification report, plus traceability and host-audit allowlists. The
    Lean verification registry (`lean/verification-status.toml`) is not touched because no theorem
    changes.
28. **Target-set bookkeeping:** does the engineering-subsystem scope keep WL-7 at its current size
    class and wording in `v3.0.0-plan.md`? **ANSWERED 2026-11-15 (D-268).** Keep the target identity
    and commitment, restate the outcome and size (L/XL), and record the 2026-11-15 owner re-plan in
    the change log and the wishlist. This is applied in the same change as this plan revision.

### 13.1 Q5-B consequences for the remaining questions

Q5 is closed by D-269. All remaining questions are now closed; the table records the Q5-B
consequence that shaped each answer.

| Question | Consequence of Q5-B |
| --- | --- |
| Q6 mode representation | **Closed by D-275.** A mode is a generic, engineer-authored, named per-device bundle: stable unique name, canonical PDO configuration and effective declared-entry remap, per-mode channel exposure, ordered required startup plan (including any vendor mode-selection SDO as startup data), explicit DC selection and optional ESI DC op-mode provenance for review only. Exactly one mode is selected per device at a time; alternates are stored but inert until explicitly selected, with no first-ESI-op-mode default and no inference from observed state. Switching is explicit and forces a cold download (D-267). Mode identity is the name, not the ESI DC op mode; the generic schema carries no vendor motion semantics. Channel exposure is per mode, and switching that leaves a bound channel unexposed is an error. The compiler-facing profile carries the selected mode's cross-checked effective projection. |
| Q7 object-dictionary sources | **Closed by D-272.** Merge by index/subindex with field-level authority: ESI is the offline baseline and sole mapping-eligibility set; online SDO Information is the live metadata authority when available and identity-matched; live reads are observed values; the project is desired values. Conflicts are surfaced, not overwritten; mapped-entry or persisted-operation conflicts block. Online-only objects are inspectable and explicitly readable/writable in setup mode but never PDO-mappable or auto-exposed as channels; startup persistence follows Q8. |
| Q8 startup SDO schema | **Closed by D-274.** One ordered, deterministic, required generic startup plan; `write`/`verify` with explicit index/subindex, typed values, explicit `PREOP`/`SAFEOP` state, bounded timeout/retry and fail-closed abort handling. Verification is `none`/`best-effort`/`required`; best-effort only controls verification. PDO configuration is a separate declarative source applied through `ecrt_slave_config_pdos`/process-data registration, with a generated non-editable diagnostic view of the equivalent CoE operations. Ordering/dependencies are modeled where the master supports them; arbitrary interleaving is not assumed. Phase 3 records the actual lifecycle and investigates literal SDO-based PDO configuration only if the native API cannot support a required configuration or sequence. |
| Q10 topology identity | **Closed by D-277.** Each device requires a unique dense position per master plus exact vendor/product/revision; no wildcards or revision ranges. Serial and alias are optional instance checks: absent means no check, declared means a hard runtime check. Position is the machine slot; serial/alias never substitute for it and never rebind. SII is the matching baseline; `0x1018` corroborates when readable, with readable disagreement blocking that device and unavailable reported `unverified` when SII matched. Identity is checked before configuration. Offline authoring needs position plus vendor/product/revision; default capture does not pin serial numbers. A revision or ESI source-reference change invalidates the mapping basis and forces re-import/review plus a cold download (D-267). Scan/compare never rewrite identity or topology. |
| Q11 master inspection boundary | **Closed by D-271.** There is no externally managed master mode and non-IgH masters are out of scope. An idle local master may be inspected read-only; a master owned by `stc run`/`stc serve` or another `stc` engineering session is an ownership conflict. Any write, including an SDO write or remap, requires managed setup mode. A live difference is reported as `different`, never auto-corrected; unreadable state is `unknown`, never a match. |
| Q12 shim ABI | **Closed by D-279.** ABI 2 is the only supported shim ABI; ABI 1 is retired with no dual loader, fallback or legacy verification-unavailable path. ABI 2 carries forward the ABI 1 lifecycle/configuration/registration/status/cycle functionality WL-7 needs and may retire unused surface. It adds position-authoritative identity and discovery (SII vendor/product/revision/serial/alias, position, AL state, `0x1018` corroboration), PDO configuration and read-back (master tree plus live CoE upload of assignment/mapping objects), SDO upload/download and startup-SDO queuing, DC read-back where supported, per-device domain/WC status, capability reporting and a structured result. Position is the lookup key; an alias may be used only after position verification. The shim reports facts; Rust computes `match`/`unverified`/`stale`. SDO Information must be attempted; if IgH cannot provide it, record the gap and keep the ESI-only view usable with the absence reported. Phase 3 records the IgH 1.6.13 capability matrix at PREOP/SAFEOP; unavailable required calls fail closed or report `unverified`, never `match`. |
| Q14 channel exposure | **Closed by D-273.** Only selected effective entries are candidates; unselected entries and online-only objects are never auto-exposed. The default proposal is one full-entry channel per selected entry, but an entry may remain unchanneled for layout; explicit non-overlapping bit slices are allowed, with `bit + width` inside the entry, matching direction, and a byte-boundary start for slices wider than one bit. A whole-entry channel and a slice overlap. Names are engineer-authored, per-device unique and matched exactly; ESI names are suggestions only and duplicates are hard errors. Bindings target the effective entry after remapping, not display order. Removing an entry a channel depends on is an error; adding an optional entry creates no channel automatically. |
| Q15 DC policy | **Closed by D-276.** DC remains an explicit component of the mode bundle: exactly one ESI-declared DC op mode or `free-run`, with no first-op-mode default and no live inference. Presentation includes each op mode's name, `AssignActivate`, Sync0/Sync1 timing and factor-resolved cycle; duplicate names are disambiguated by index. `AssignActivate` is immutable, timing values are derived and persisted, unresolvable factors or invalid combinations fail closed, and arbitrary per-field overrides are not allowed. A no-DC ESI may prefill `free-run` at import, but the choice is persisted; missing DC in a new configuration is an error. Declared op modes live in device-level facts; the compiler validates offline. Runtime applies the values before activation, never selects DC, and reports `unverified` when read-back is unavailable. Changing DC forces a cold download; a remap does not change the DC selection. |
| Q16 mismatch policy | **Closed by D-278.** Comparison is policy-free (`match`/`different`/`unverified`/`not applicable`). WL-7's consequence is degraded per-device isolation: a mismatched, offline or failed device gets no configuration writes, stays out of OP, has its inputs held/marked stale and its outputs not applied, while healthy devices and PLC cycles continue. No lazy matching, fallback or rebind; identity mismatch means zero writes, and a later failure may retain partial writes. D-274/D-272/D-269 blocks are device-scoped. Master failure is fatal; missing/mismatched slaves are runtime-degraded. `startup-timeout` is retired; PLC cycles start after activation. A missing slave may shift downstream positions; positions are evaluated independently with no renumbering. Reconnect only for devices configured at activation; a startup-offline device needs restart/cold download. Missing DC reference, extra slaves and shifts are reported; no free-run fallback. Layout 2 adds `slave<N>.input_stale`; global `input.stale` is a summary. Strict mode is future. |
| Q17 diagnostics | **Closed by D-280.** Stable, structured, layer-owned families: `STC-E-ESI-*` for import, `STC-E-IO-ETH-009`–`019` for topology/compare, `020`–`029` for engineering mode/PDO/DC/channel, `030`–`039` for SDO, `040`–`049` for ownership/setup/ABI, and `STC-W-IO-ETH-004`–`019` for runtime degraded. Existing codes keep their meanings; `STC-E-IO-ETH-008` is retired/reserved. `E` rejects an operation or fails run start; `W` continues degraded. Runtime isolation is a `W` with the reason in the payload. The shim reports facts, not codes. Payload includes device/position, operation, object, expected/observed, AL state, abort code, capability and remedy; text is never parsed. Degraded diagnostics emit on transitions; `slave<N>.input_stale` is the per-cycle surface. Every code has an emitting test and traceability entry. |
| Q19 code placement/unsafe | **Closed by D-270.** Q5-B adds no new FFI surface: selection and remap validation are safe Rust in the engineering layer; setting PDOs stays in the existing shim call, and read-back stays in `stc-runtime` under D-077. No new crate; the CLI stays in `stc-driver`, and the host-neutral API/mock remains the test boundary. |

## 14. Acceptance log

| Phase | Accepted | Commit | CI run | Notes |
| --- | --- | --- | --- | --- |
| 0 | 2026-11-23 | `9f9914e` | n/a (planning phase; docs only) | Revised architecture reviewed against the branch on 2026-11-15 and aligned to the workflow; §13 has 28 questions. Owner answered Q20/Q21 (remote UX deferred, D-259) and Q22 (setup mode, D-260) on 2026-11-15, then Q1–Q16 and Q18–Q28 (D-260 amended, D-261–D-279); Q9 is closed by the Q24 answer. Q5-B consequences are recorded in §13.1. Q10 (D-277), Q12 (D-279) and Q16 (D-278) were accepted on 2026-11-21, and Q17 (D-280) on 2026-11-23. Owner approved Phase 0 on 2026-11-23; all 28 questions are answered and Phase 1 is next |
| 1 | 2026-11-25 | `1a1cc32` | [run 45561822998](https://github.com/r2k-in-the-vortex/stc/actions/runs/45561822998) green | Typed engineering service in `stc-runtime::ethercat` (`open`/`open_with`/`standalone`/`list`/`search`/`inspect_source`/`inspect_identity`/`capabilities`/`supports`/`set_host`) with no terminal text in the API. Parser selection locked to `quick-xml` 0.42 with `default-features = false` and reader-owned bounds/encoding/rejection policy; the local corpus campaign stays gated to Phase 8 (`STC_ESI_CORPUS` unset here), so the evaluation used the committed synthetic fixtures. Bounded ESI reader, typed capability model and deterministic library search/identity resolution emit `STC-E-ESI-001`–`007`; `008`–`009` stay reserved and are never emitted. ABI 2 capability/result contract plus `MockEngineeringHost` added as types only — the loader still speaks ABI 1 until Phase 7. CLI `stc ethercat esi list|search|inspect` covered by unit and integration tests, and `fuzz_esi` plus seeds wired into the CI fuzz-smoke job. |
| 2 | 2026-11-25 | `62658aa` | [run 45653669868](https://github.com/r2k-in-the-vortex/stc/actions/runs/45653669868) green | Desired topology (dense positions, exact vendor/product/revision, optional serial/alias instance checks), observed scan, per-position comparison (`match`/`different`/`unverified`/`not applicable`) and explicit capture that does not pin instance checks by default. Comparison evaluates every position independently, reports extras (`STC-E-IO-ETH-012`) and downstream shifts (`014`), and never rewrites project intent. Explicit setup-mode session (enter/status/leave, session identity, second session `044`, unreadable-descriptor `045` recovery, ownership conflict `040`) that runs no cycle. CLI `topology show|scan|diff|capture`, `slave inspect` and `setup enter|leave|status`, with `--observed` for offline review; live operations fail closed with `043` until Phase 7 binds ABI 2. Fixed-command wrapper `stc-wl7-eng` with its sudoers rule and runbook §14. The first `macOS tests (macos-15)` job was cancelled before a runner was assigned (no steps ran); the rerun passed and the run is green. |
| 3 | 2026-11-27 | `b685b37` | [run 45890691095](https://github.com/r2k-in-the-vortex/stc/actions/runs/45890691095) green | Capability model, constrained PDO selection with declared-entry remap, engineer-authored named mode bundles, channel exposure, explicit DC selection with factor-resolved cycles, and diagnostics `STC-E-IO-ETH-020`–`024`/`041`. Live paths run through `MockEngineeringHost` and the CLI fails closed with `043` until the ABI 2 shim lands in Phase 7. IgH 1.6.13 prototype on the `ecat` bench (`tools/ethercat/phase3_probe.c`, four modes) recorded in the Phase 3 record. **Owner-approved capability limit:** the attached DIEWU has no CoE mailbox (`sdo_count=0`, all uploads `-93`), so live assignment/mapping read-back and device-refused remap cannot be demonstrated on it; the effective configuration is always `unverified`, never `match`, and no refusal is claimed. A CoE-capable slave can repeat those cases without changing this phase's implementation. Attempt 1 of the green run had one unrelated macOS-Intel `debug_protocol` flake, which passed on rerun; watch item. |

**Owner acceptance and CI waiver (2026-11-25).** The owner accepted the phase 2 result with
`VS Code extension smoke (macos-15)` cancelled because GitHub had no macOS runner available to take
the job, and directed that this job does not block acceptance for the remaining WL-7 phases: it
exercises the VS Code extension smoke path, not the engineering changes under review. The
cancellation stays recorded with the phase evidence (run, job, and the fact that no steps ran and
no runner was assigned). The waiver covers only that job when it is cancelled for lack of a runner;
any other failed, cancelled or missing job still blocks acceptance until it is explained or rerun,
and a waiver is never applied retroactively to hide a failure.

## 15. Change log

| Date | Change |
| --- | --- |
| 2026-11-15 | Revised from ESI-to-static-profile feature into an EtherCAT engineering subsystem. ESI is a capability description; desired configuration is engineer-selected and persisted at configuration time. Added offline/manual topology, online discovery, PDO/mode selection, live object-dictionary/SDO commissioning, ordered startup SDO operations, desired-vs-observed comparison, API-first design, CLI scope, existing SSH remote workflow, managed/external master distinction, and future TUI/GUI compatibility. |
| 2026-11-15 | Branch review and workflow alignment. Added §4.1 with verified findings (no generic remote execution; exclusive IgH reservation and idle-phase-only tooling; shim has no identity/PDO/SDO read-back; fixed-command sudoers; `unsafe_code` limited to `stc-runtime`; strict profile keys; no build-time width validation; embedded-profile hashing; Linux-only shim mocks; doc/traceability/host-audit gates; no static-profile migration). Narrowed the external-master mode to idle-phase read-only with an unsupported case, added SDO/read-back feasibility, added per-phase evidence, the workflow acceptance rule, local-check and coverage expectations, the remote-exec enabler in Phase 2, the build-time diagnostic in Phase 6, ownership/privilege work in Phase 7, and expanded Phase 0 from 18 to 28 required decisions. `v3.0.0-plan.md` §1/§3/§4/§8/§9 and `FutureWishlist.md` WL-7 were aligned to the re-planned scope. |
| 2026-11-15 | Owner clarification recorded as D-259 and D-260. Remote engineering is not a WL-7 work item: engineering is invocation-context independent like the debug session, and remote/GUI packaging follows WL-7. EtherCAT engineering and SDO commissioning run in an explicit setup mode where the PLC is not started (exclusive master, no cycle, no output writes, fail-closed conflict). §2.4, §4.1, §5.7, §6.3, §8, Phase 2/4/9, §13 Q20–Q22, and the acceptance log were updated; the addressable enabler count dropped from three to two. |
| 2026-11-15 | Owner Phase 0 answers recorded. Q1: vetted parser crate, with `quick-xml` evaluated on the local ESI corpus before locking the selection. Q2: ESI 1.2 only; 1.0/1.1 tracked as WL-24 in `FutureWishlist.md`. Q3/Q4: ESI source and revision policy; ESI creates the initial project TOML setup, after which the TOML files are authoritative and ESI files no longer matter. Q13: parser bounds. Q18: vendor ESI stays local and the validation runbook publishes the ASIX AX58100 download link. Q22: setup session lifecycle finalized. Q23: fixed privileged wrappers. Q24/Q9: dedicated engineering configuration file with the effective selection embedded in the profile. Q25: build-time width diagnostic. Q26: any EtherCAT configuration change forces a cold download. Q27: documentation and gate set. Q28: target-set bookkeeping unchanged. Recorded as amended D-260 and new D-261–D-268; §4, §4.1, §5.1, §5.6, §5.7, §7, §8, §12, Phases 1/2/6/7/8/9 and §13 were aligned. |
| 2026-11-15 | Owner Q5 answer recorded as D-269: constrained PDO selection plus declared-entry remapping, limited to entries the ESI declares for the device/direction; `Fixed`/`Mandatory`/`Exclude`/`ExcludedSm` are hard constraints, missing flags take schema defaults, ESI and live capabilities combine as the most restrictive union, and the effective selection plus constraint facts are persisted for offline compiler validation. Startup SDOs do not own PDO assignment/mapping objects. Arbitrary object-dictionary mapping (option C) is deferred indefinitely with no wishlist entry. §3.1/§3.2, §5.3, §5.6, §7, §9, §11, §12, Phases 3/5/6/7/8, §13 Q5, new §13.1 consequences for Q6–Q8/Q10–Q12/Q14–Q17/Q19, and the acceptance log were aligned. |
| 2026-11-20 | Owner clarified Q26: there is no hash split and no provenance-only category. Every persisted EtherCAT configuration input, including the declared-entry remap, persisted constraint facts, and the ESI source reference and identity, forces a cold download when it changes. Renamed the Q26 heading to effective-configuration hash scope and aligned §4.1, §5.1, §5.3, §5.6, §12, Phase 6, §13 Q26, and §13.1 Q10. |
| 2026-11-20 | Owner Q19 answer recorded as D-270: the engineering engine and all shim FFI stay in `stc-runtime`, the CLI stays in `stc-driver`, and WL-7 adds no `stc-ecat` crate. `unsafe_code` remains limited to the existing documented `stc-runtime` exception (D-077). §13 Q19, §13.1 Q19, the Phase 0 acceptance log, Phase 1 plumbing, and `specification/Decisions.md` were aligned. |
| 2026-11-20 | Owner Q11 answer recorded as D-271: there is no externally managed master mode; a master owned by another application and a non-IgH master are out of scope. An idle local master may be inspected read-only, and any write, including an SDO write, requires managed setup mode. SDO writes are normal setup activity, not forbidden. §2.4, §3.1, §4.1, §5.7, §12, Phases 0/2/4, §13 Q11, and §13.1 Q11 were aligned. |
| 2026-11-20 | Owner Q7 answer recorded as D-272: ESI and online SDO Information merge by object index/subindex with field-level authority and visible provenance, not a global winner. ESI stays the offline baseline and sole mapping-eligibility source; online metadata is the live authority when available and identity-matched; live reads are observed values and the project is desired values. Conflicts are surfaced, and mapped-entry or persisted-operation conflicts block. Online-only objects are inspectable and explicitly writable but never mappable or auto-exposed as channels. §5.4, Phase 4 work/exit criteria, §13 Q7, §13.1 Q7, and `specification/Decisions.md` were aligned. |
| 2026-11-20 | Owner Q14 answer recorded as D-273: only selected effective entries become channels; the default proposal is one full-entry channel per selected entry, with an entry optionally left unchanneled for layout. Explicit non-overlapping bit slices are allowed, must fit the entry and match direction, and slices wider than one bit must start on a byte boundary. Names are engineer-authored, per-device unique and matched exactly; duplicate names are hard errors, never silent renames. §5.3, Phase 3/6 work and exit criteria, §11 tests, §13 Q14, §13.1 Q14, and `specification/Decisions.md` were aligned. |
| 2026-11-21 | Owner Q8 answer recorded as D-274, and D-269 clarified: generic startup SDOs form one ordered, deterministic, required plan (`write`/`verify`, typed values, explicit PREOP/SAFEOP state, bounded timeout/retry, fail-closed abort handling, `none`/`best-effort`/`required` verification). PDO configuration remains a separate declarative authoring source applied through IgH's native `ecrt_slave_config_pdos`/process-data registration path; the equivalent CoE operation view is diagnostic and non-editable, and arbitrary interleaving with generic SDOs is not assumed. Phase 3 records the actual lifecycle and investigates literal SDO-based PDO configuration only if the native API cannot support a required configuration or sequence. §3.1, §5.3, §5.4, §8, §12, Phases 3/5/7, §11, §13 Q5/Q8, §13.1 Q8, and `specification/Decisions.md` were aligned. |
| 2026-11-21 | Owner Q6 answer recorded as D-275: a mode is a generic, engineer-authored, named per-device bundle stored in the dedicated engineering configuration file. It carries a stable unique name, the canonical PDO configuration and effective declared-entry remap, per-mode channel exposure, the ordered required startup plan (including any vendor mode-selection SDO as startup data), the explicit DC selection and optional ESI DC op-mode provenance for review only. Exactly one mode is selected per device at a time; alternates are stored but inert until explicitly selected, with no first-ESI-DC-op-mode default and no inference from observed state. Switching is explicit and forces a cold download. Mode identity is the name, not the ESI DC op mode, and the generic schema carries no vendor motion semantics. The compiler-facing profile carries the selected mode's cross-checked effective projection. §5.3.1, §12, Phases 3/5/7, §11, §13 Q6, §13.1 Q6, and `specification/Decisions.md` were aligned. |
| 2026-11-21 | Owner Q15 answer recorded as D-276: every mode selects exactly one ESI-declared DC op mode or explicit free-run, with no first-op-mode default and no live inference. Presentation includes `AssignActivate`, Sync0/Sync1 timing and factor-resolved cycles; duplicate names are index-disambiguated; `AssignActivate` is immutable and timing values are derived and persisted; unresolvable factors, invalid combinations and arbitrary overrides fail closed. A no-DC ESI may prefill free-run at import, but missing DC in a new configuration is an error. Declared op modes live in device-level facts; runtime applies the values before activation and reports `unverified` when read-back is unavailable. §5.3.1, §5.3.2, §12, Phases 3/5/7, §11, §13 Q15, §13.1 Q15, and `specification/Decisions.md` were aligned. |
| 2026-11-21 | Owner Q10 answer recorded as D-277: each configured device requires a unique dense position per master plus exact vendor/product/revision, with no wildcards or revision ranges. Serial and alias are optional instance checks — absent means no check, declared means a hard runtime check — and neither substitutes for position or rebinds. SII is the baseline, `0x1018` corroborates when readable, and identity is checked before configuration. Default capture does not pin serial; revision or ESI source-reference changes force re-import/review and a cold download. §5.2, Phase 2/5/6, §11, §12, §13 Q10, §13.1 Q10, the acceptance log, and `specification/Decisions.md` were aligned. |
| 2026-11-21 | Owner Q16 answer recorded as D-278: comparison is policy-free (`match`/`different`/`unverified`/`not applicable`) and the WL-7 consequence is degraded per-device isolation. Master open/activation failure stays fatal; missing/mismatched slaves are runtime-degraded. A failed device gets no configuration writes, does not enter OP, has inputs held/marked stale and outputs not applied, while healthy devices and PLC cycles continue. No lazy matching or rebind; `startup-timeout` is retired; reconnect applies only to devices configured at activation; missing DC reference, extra slaves and downstream shifts are reported; layout 2 adds `slave<N>.input_stale` and global `input.stale` becomes a summary. §5.5, §8, Phases 2/5/6/7/8, §11, §12, §13 Q16, §13.1 Q12/Q16, the acceptance log, and `specification/Decisions.md` were aligned. |
| 2026-11-21 | Owner Q12 answer recorded as D-279: ABI 1 is retired and ABI 2 is the only supported shim ABI, with no dual loader, fallback or legacy verification-unavailable path. ABI 2 carries forward the ABI 1 lifecycle/configuration/registration/status/cycle functionality WL-7 needs and may retire unused surface; it adds position-authoritative identity/discovery, PDO configuration and read-back, SDO upload/download and startup-SDO queuing, DC read-back where supported, per-device domain/WC status, capability reporting and structured results. The shim reports facts and Rust computes `match`/`unverified`/`stale`. SDO Information must be attempted in WL-7; an IgH capability gap is recorded and the ESI-only view remains usable with the absence reported. Phase 3 records the IgH 1.6.13 PREOP/SAFEOP capability matrix, and unavailable required calls fail closed or report `unverified`, never `match`. §4.1, §8, Phases 1/3/4/7, §11, §12, §13 Q12, §13.1 Q12, the acceptance log, and `specification/Decisions.md` were aligned. |
| 2026-11-23 | Owner Q17 answer recorded as D-280: diagnostics are stable, structured and layer-owned. Existing `STC-E-IO-ETH-001`–`007` and `STC-W-IO-ETH-001`–`003` keep their meanings; `STC-E-IO-ETH-008` stays retired/reserved. New append-only families are `STC-E-ESI-001`–`009`, `STC-E-IO-ETH-009`–`019`, `020`–`029`, `030`–`039`, `040`–`049`, and `STC-W-IO-ETH-004`–`019`. `E` rejects an operation or fails run start; `W` continues degraded. The shim reports facts and capabilities, never codes; the Rust layers own codes. Runtime degraded diagnostics emit on transitions while `slave<N>.input_stale` carries per-cycle truth. Every new code needs an emitting test and a traceability entry. §9, §13 Q17, §13.1 Q17, Phases 1/3/4/7/9, §11, §12, the acceptance log, and `specification/Decisions.md` were aligned. |
| 2026-11-23 | Owner approved Phase 0. All 28 questions are answered by D-259–D-280, the Phase 0 decisions are accepted, and Phase 1 is next. The plan status and acceptance log were updated; no implementation started. |
| 2026-11-25 | WL-7 Phase 1 implemented: `stc-runtime::ethercat` engineering service, bounded ESI reader with the `STC-E-ESI-001`–`007` catalog, deterministic ESI library and identity resolution, ABI 2 capability/result contract and mock, `stc ethercat esi` CLI, and the `fuzz_esi` target. Parser selection locked to `quick-xml`; the local corpus campaign remains Phase 8. Acceptance recorded in §14. |
| 2026-11-25 | WL-7 Phase 2 implemented and accepted: desired/observed topology, comparison, capture, slave inspection, setup session, CLI surface and the fixed privileged wrapper, accepted on run 45653669868 at `62658aa`. Owner then waived the macOS runner-less `VS Code extension smoke (macos-15)` cancellation for the remaining phases (§14). |
| 2026-11-27 | WL-7 Phase 3 accepted with the owner-approved capability limit: PDO capability inspection, mode bundles, declared-entry remap with offline validation, explicit DC and factor resolution, channel exposure, diagnostics `020`–`024`/`041`, and the IgH 1.6.13 bench prototype. Live read-back and device-refused remap stay `unverified` because the attached DIEWU has no CoE mailbox; the limit is recorded and does not carry into Phase 4. |

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
