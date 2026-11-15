# WL-7 Plan — ESI-based EtherCAT device configuration

Status: Phase 0 is open. The plan is drafted and submitted for owner approval; no implementation
commit exists on the branch. This is a temporary working document; it is deleted at closeout.

- Date: 2026-11-15
- Branch: `WL-7`, branched from `v3.0.0-planning` at `7e053a4`
- Parent planning item: [v3.0.0-plan.md](v3.0.0-plan.md) §4 WL-7 (locked 2026-11-04)
- Wishlist entry: [FutureWishlist.md](FutureWishlist.md) WL-7 (status `Locked`)
- Baseline: `stc` 2.0.0 plus the delivered v3.0.0 targets (WL-1.2, WL-14, WL-15, WL-16, WL-17,
  WL-18)
- Prerequisite delivered: WL-5.2 EtherCAT backend (IgH 1.6.13 shim, DIEWU slave, static profile)
- Profile: `stc-2013-oop-utf8-v1`

## 1. Goal

Let a project describe an EtherCAT slave by its vendor ESI XML instead of a hand-written
`syncs`/`pdo-channels` map. Validated ESI data feeds three places: the effective machine profile
that the compiler embeds in the artifact, the IgH slave configuration the runtime applies, and a
live startup verification that the discovered slave matches the description it was configured
from.

WL-7 replaces the static-profile authoring shortcut described in
[I/O Routing and Backends](specification/Architecture/IO-Routing-and-Backends.md) §2.2 ("The
V2.0.0 profile model is static: an ESI parser, dynamic PDO discovery, and SDO-based device
configuration are deferred (WL-7)"). It is additive: a profile that spells out `syncs` and
`pdo-channels` keeps working unchanged, the `DeviceBackend` trait, coordinator cycle order, route
graph, process image, metadata model, and host ABI are untouched, and the language and compiler
stay protocol-agnostic.

## 2. Locked scope

Taken from [v3.0.0-plan.md](v3.0.0-plan.md) §4 WL-7; not editable without an owner re-plan.

- Ingest ESI XML from project or runtime configuration, including package-relative paths and a
  vendor file library.
- Parse and validate identity (vendor ID, product code, revision), PDO mappings, sync managers,
  distributed clocks, and mailbox/SDO data. Handle multiple files and revisions with
  deterministic selection and clear diagnostics.
- Map validated slave descriptions to the `stc` device/channel model and to IgH configuration;
  verify at startup that the discovered slave matches the expected vendor/product/revision and
  PDO layout.
- Decide the compiler/runtime split: the compiler may validate binding widths against ESI at
  build time while keeping the language and metadata protocol-agnostic; the runtime owns IgH
  configuration and live verification.
- Define and document the supported ESI subset; reject unsupported or ambiguous input explicitly
  rather than silently ignoring fields.
- Test with synthetic and vendor-format ESI files, malformed/ambiguous/mismatched inputs, a
  simulator, and hardware-in-the-loop on the RT host. Ship only a small curated ESI set for
  tests; users supply vendor files.

## 3. Exclusions

- Full ESI, ESI-XSD, or object-dictionary coverage; complete `DataType`/`ObjectDictionary`
  modelling; EoE, FoE, AoE, or SoE protocol handling; redundancy, hot-connect, and cable
  redundancy configuration; dynamic PDO remapping at run time.
- Distributed-clock behavior beyond the `AssignActivate`/sync0/sync1 subset that the delivered
  `dc` profile table already exposes (D-227). No drift compensation work, no reference-clock
  selection policy beyond what IgH does.
- Motion control, servo commissioning, and drive configuration (WL-6 stays deferred). No
  claims about vendor drive objects.
- Changing the EtherCAT backend semantics, the `DeviceBackend` trait, the coordinator cycle
  order, the route graph, the process image, the metadata/protocol contract, or the host ABI.
- New core language or compiler features. Compiler changes stay inside manifest/profile
  validation and the existing metadata embedding path.
- Committing vendor ESI files, or making any vendor file, TwinCAT installation, or external
  service a prerequisite of the build, the CI matrix, or the release package.
- A new network service, a safety function, or a hard-real-time claim. `stc` remains not a
  safety PLC ([NON-SAFETY.md](NON-SAFETY.md)).
- SDO startup parameters as a shipped feature unless Phase 0 answers Q7 "yes"; the parse and
  rejection path exists either way.

## 4. Starting point: the contracts this feature must fit

These are delivered and treated as fixed inputs, not open design:

- **Device and channel model.** `stc.toml` owns the logical device tree (`kind`,
  `[[io.devices.channels]]` names, directions, widths). The machine profile references devices
  by name and carries the PDO map ([I/O and Devices](specification/Architecture/IO-and-Devices.md)
  §3, [I/O Routing and Backends](specification/Architecture/IO-Routing-and-Backends.md) §2.2).
  A selected device requires every declared channel to map to exactly one `pdo-channels` entry
  and every entry back to a declared channel; duplicates, gaps, and overlaps are cold-start
  errors.
- **Profile parsing and embedding.** `crates/stc-manifest/src/profile.rs` parses
  `[device.<name>]` tables and `[[routes]]` into `Profile`/`ProfileDevice` and an opaque
  `ConfigValue` tree. The compiler canonicalizes the selected profile, validates routes, and
  embeds the effective profile in `<name>.stc-meta.json`; the runtime never reads the profile
  file, so any ESI-derived data must be resolved at build time
  ([CLI and Manifest](specification/Architecture/CLI-and-Manifest.md) §2).
- **EtherCAT backend.** `crates/stc-runtime/src/io_ethercat.rs` validates the profile, configures
  each slave through the project-owned shim (`tools/ethercat/stc_ec_shim.{h,c}`, IgH 1.6.13,
  `dlopen` only, ABI `stc-io-ethercat/1`), activates the master, registers PDO entries, maps
  domain bits, waits for OP, and owns fault/recovery and the status block. The shim already
  receives `vendor_id`, `product_code`, `revision`, and `check_revision` per slave, but the
  backend never reads back what the slave actually reports, and it does not verify the PDO
  assignment the slave ended up with.
- **Diagnostics.** Stable `STC-E-IO-ETH-001`–`008` and `STC-W-IO-ETH-003` exist. `-status` layout
  1 channels and `stc run -v` diagnostic keys are fixed ([§5](specification/Architecture/IO-Routing-and-Backends.md),
  [§8](specification/Architecture/IO-Routing-and-Backends.md)).
- **Test conventions.** Backend behavior in `crates/stc-runtime/tests/*` on every CI host
  (mock shim on non-Linux), driver/CLI fixtures in `crates/stc-driver/tests/*`, fuzz targets in
  `fuzz/`, HIL on the Linux `ecat` host with the DIEWU 16I/16O bench (vendor `0x00000b95`,
  product `0x00010200`, revision `1`) per [EtherCAT-Runbook.md](specification/Verification/EtherCAT-Runbook.md).
- **No XML stack exists.** The workspace has no XML dependency
  (`grep xml --include=Cargo.toml` → none) and workspace lints forbid `unsafe_code`, so the
  parser choice is a Phase 0 decision (Q1), not an assumption.

## 5. ESI test corpus

The TwinCAT installation on the development host ships a large vendor ESI library at
`C:\Program Files (x86)\Beckhoff\TwinCAT\3.1\Config\Io\EtherCAT`. Measured 2026-11-15:

| Property | Value |
| --- | --- |
| Files under the tree | 458 (387 `.xml`, the rest module/dictionary sidecars) |
| `<EtherCATInfo Version>` | `1.2` in all 142 files that spell the attribute |
| Encoding declarations | 249 UTF-8 (any case), 136 ISO-8859-1 (any case) |
| Files declaring at least one `<Device>` | 144 |
| Largest file | `Beckhoff ELM31xx.xml`, 70.6 MB |
| Multi-device files | Common: one file carries many `<Device Type ProductCode RevisionNo>` entries, and `-zzzz-0101`/`-0102` companions carry revision variants |
| Module/safety sidecars | `<...> Modules.xml`, `<...> Dict.xml`, `... Modules Safety.xml` — a second document shape that is not a slave description |

Corpus policy (proposed, confirmed in Phase 0 Q3):

- The corpus is **reference input for development and manual campaigns, never a committed
  fixture**. `WL-7-Plan.md` and the runbook name the path and record the counts above.
- Automated tests read the corpus only when `STC_ESI_CORPUS` points at a directory; those tests
  are `#[ignore]`d by default (or gated) so the CI matrix needs no vendor files. A local run is
  recorded in the verification report with the corpus path, counts, and the pass/fail tally.
- Committed fixtures under `tests/esi/` are hand-authored synthetic documents in the ESI 1.2
  shape, plus any file the owner confirms is redistributable. The ASIX AX58100 sample ESI
  (`AX58100_EVB_DIO_ESI_00010200_00000001_20220323.xml`, 94 lines, the same identity as the
  bench slave) is a candidate for permission-cleared inclusion; until the owner confirms its
  licence, it is used from the local corpus only.
- The corpus is used to measure three things that synthetic files cannot: real-world encoding
  mix (ISO-8859-1!), file size bounds, and vendor-extension noise that must be rejected
  explicitly or tolerated in a documented way.

## 6. Interface design (proposed; confirmed in Phase 0)

### 6.1 Supported ESI subset

Parsed and used:

- `<EtherCATInfo Version="1.2">`, `<Vendor><Id>`, `<Vendor><Name>`.
- `<Descriptions><Groups><Group>` — used only for `<Type>`/`<Name>` context in diagnostics and
  channel naming; group images are ignored.
- `<Descriptions><Devices><Device Physics>` → `<Type ProductCode RevisionNo>`, `<Name>`,
  `<GroupType>`, `<Fmmu>` (declared but not mapped: IgH derives FMMUs), `<Sm StartAddress
  ControlByte Enable OpOnly>` in document order, `<RxPdo>`/`<TxPdo>` (`Index`, `Name`, optional
  `Fixed`, `Sm`, `Mandatory`) with `<Entry>` (`Index`, `SubIndex`, `BitLen`, `DataType`, `Name`,
  optional `Comment`), `<Dc>` (`<OpMode>` with `<Name>`, `<AssignActivate>`, `<CycleTime>`
  `<Sync0>`/`<Sync1>`, `<Shift>` `<Sync0>`/`<Sync1>`), `<Mailbox>` (`<CoE>` and its
  `SdoInfo`/`PdoAssign`/`PdoConfig`/`CompleteAccess` flags, `<Bootstrap>`, and the mailbox
  protocol list).
- Multiple `<Device>` entries per file and multiple files per device, subject to §6.2.

Parsed, validated, then ignored with a documented note rather than silently dropped:

- `<ObjectDictionary>`, `<DataType>` definitions, `<Profile>`/`<Dictionary>` file references,
  `<VendorSpecific>`/`<xs:any>` content, and `<Info>`/`<Img>`. They are recorded as
  "present but unused" in `stc run -v` diagnostics (counts only, no content echo) so a user can
  see what was not applied.

Rejected with an explicit diagnostic (never silently ignored):

- Any ESI document rooted at something other than `EtherCATInfo`, a `Version` other than `1.2`
  (Q4: 1.0/1.1 acceptance, proposed: reject unless Phase 0 says otherwise), a missing
  `<Vendor><Id>`, a `<Device>` without `<Type>` `ProductCode`/`RevisionNo`, a PDO entry with
  `BitLen` outside `1..=64` or a non-byte-aligned `BitLen` below 8 where the entry is exposed as
  a channel, a `Sm` index that has no matching `<Sm>` declaration, an `<Entry>` referencing a
  PDO that is declared with an inconsistent direction, overlapping bit ranges inside one PDO, a
  `<Dc><OpMode>` without `AssignActivate`, and a document that is not well-formed, exceeds the
  size/depth/element limits, or contains a `DOCTYPE`/entity declaration (no entity expansion, no
  external fetch).
- Module sidecars (`Modules.xml`, `Dict.xml`) are recognised by root element and rejected as
  "not a slave description" with the expected-document diagnostic, not a parse error.

### 6.2 File resolution and deterministic selection

Proposed profile surface (new keys, additive to §2.2):

```toml
[device.diewu]
master = 0
alias = 0
position = 0
# ESI sources: either explicit files or a vendor library directory searched for *.xml.
esi = ["esi/ASIX_AX58100_EVB.xml"]
# esi-dir = "esi/vendor"           # alternative: library search
# revision = "0x00000001"          # optional pin; omitted -> highest revision
# esi-strict = true                # default: identity must match exactly
```

Selection rules (Q5, Q6):

- Resolution is per device, at build time, relative to the project root and confined to it
  (the existing `[io].config` rule). `.stc` packages embed the resolved data, so no path is read
  at run time.
- A candidate matches when `Vendor/Id` equals the device vendor and `Device/Type@ProductCode`
  equals the product code. Among matches, `revision` pins an exact `RevisionNo`; without a pin
  the highest numeric `RevisionNo` wins and the choice is reported in the build log and
  `stc run -v`.
- Zero matches, more than one match at the same revision, two files describing the same
  vendor/product/revision with different content, or a device that declares both `esi` and
  hand-written `syncs` are errors, not precedence puzzles.
- The effective profile records the resolved description's vendor/product/revision plus a
  content hash of the source document; the artifact is reproducible and self-describing. The
  hash is what startup verification compares against the *configured* description.

### 6.3 Mapping to the device/channel model and IgH configuration

- `<Sm>` order becomes the `syncs` array order; `ControlByte`/`Enable`/`OpOnly` map to
  `direction` and `watchdog` using the IgH conventions the shim already implements; an
  `<Sm>` whose control byte does not map to a known input/output form is rejected.
- Each `<RxPdo>`/`<TxPdo>` becomes a `pdos` entry under its `Sm`; each `<Entry>` becomes an
  `entries` entry with its `BitLen`, in document order, which is the bit layout IgH concatenates.
- Channels: by default one `pdo-channels` entry per ESI `<Entry>`, named from `<Name>` when it is
  a unique identifier-safe token and otherwise from `0x<index>:<subindex>`, with `bit = 0` and
  the entry width. Bit-granular access follows the existing model: a declared
  `[[io.devices.channels]]` may name `entry = { index, subindex }, bit, width` and is validated
  against the ESI entry width (§2.2 already allows this shape). Q8 fixes whether sub-entry
  slicing is auto-generated (one channel per bit for `BITARRn`-style entries) or explicit only.
- Values the ESI does not determine stay profile-owned: `master`, `alias`, `position`,
  `startup-timeout`, `recovery-cycles`, and the `dc` table's enablement. The ESI supplies
  `assign-activate`, `sync0/sync1-cycle`, and shifts as defaults for `dc`; an explicit profile
  `dc` value wins and the difference is logged (Q9).
- The compiler validates each declared channel width against the ESI-derived entry width and
  rejects a mismatch at build time. It does not interpret PDOs beyond that comparison: no
  protocol type enters the compiler's language or metadata paths, and no `stc-meta.json` key
  changes beyond the profile/config tree it already embeds.

### 6.4 Startup verification and mismatch policy

Delivered today: the shim is handed the expected identity and IgH checks vendor/product at
configuration time; the backend verifies that registered entry offsets fit the domain. Missing:
a read-back of the slave that is actually on the wire, and of the PDO assignment it actually
applied.

Proposed:

- Shim ABI `stc-io-ethercat/2` adds `stc_ec_slave_identity(master, slave, out)` (vendor, product,
  revision, serial, alias, position from `ecrt_master_get_slave`) and
  `stc_ec_pdo_assignment(master, slave, out)` (the RxPdo/TxPdo index list read from
  `0x1C12`/`0x1C13` while the slave is in PREOP, before activation). The loader keeps accepting
  ABI 1 and reports verification as unavailable for it; ABI 2 is required for the new checks, and
  the shipped shim source is bumped and rebuilt for the HIL run.
- Verification runs after slave configuration and before `master_activate`: compare
  vendor/product/revision, the PDO index list, and the summed input/output bit length against the
  description the build resolved.
- Mismatch policy (Q10): proposed **fail closed** — cold start fails with
  `STC-E-IO-ETH-003` naming expected vs discovered values and the slave position, no cycle runs,
  and outputs stay unwritten. A `esi-strict = false` escape hatch is not added unless the owner
  asks for one; a warning-only mode would silently run a wrong layout, which the v3.0.0 exit
  criterion forbids.
- The mismatch decision is recorded in the status block: `master.last_error` carries the code and
  `stc run -v` prints expected/discovered identity and PDO lists.

### 6.5 Diagnostics

New build-time codes (namespace proposed in Q11):

| Code | Meaning |
| --- | --- |
| `STC-E-IO-ESI-001` | ESI document not found, unreadable, or outside the project root. |
| `STC-E-IO-ESI-002` | ESI document is not well-formed, unsupported version, or not a slave description. |
| `STC-E-IO-ESI-003` | ESI document exceeds a size, depth, or element-count limit. |
| `STC-E-IO-ESI-004` | Identity or revision is missing, ambiguous, or selected non-deterministically. |
| `STC-E-IO-ESI-005` | Sync, PDO, entry, or DC data is inconsistent or unsupported by the subset. |
| `STC-E-IO-ESI-006` | A declared channel does not match an ESI entry (missing, duplicate, or width). |
| `STC-E-IO-ESI-007` | The device declares both an ESI source and a hand-written map. |
| `STC-W-IO-ESI-001` | ESI content was parsed but is not applied (object dictionary, profile, images). |

Runtime mismatch keeps `STC-E-IO-ETH-003` and `STC-E-IO-ETH-004`; a new
`STC-W-IO-ETH-004` reports "verification unavailable because the shim is ABI 1". Every new code
gets a traceability row and a test that emits it.

### 6.6 Parser and untrusted input

- Bounded by construction: file size cap (default 32 MiB, configurable, Q12), element and
  attribute counts, nesting depth, and text-node length; no entity expansion, no DTD/DOCTYPE, no
  external resource fetch, no XInclude. Encoding declarations are honoured for UTF-8 and
  ISO-8859-1 (the corpus's two real encodings) and anything else is a clear error.
- No panics on untrusted input; every scan is checked arithmetic, and the decoder is a fuzz
  target (`fuzz/fuzz_targets/fuzz_esi.rs`) with synthetic seeds plus a small generated mutation
  corpus.
- Whatever the parser choice (Q1: project-owned minimal reader versus a vetted crate such as
  `quick-xml`), the security stance is the same: no external dependency may add a
  network/filesystem surface, and the licence must be MIT/Apache-compatible.

## 7. Phases

Each phase follows [Feature Development Workflow](workflows/FEATURE-DEVELOPMENT.md) §4–§6: one
subagent handoff, local checks, pushed commit, green CI, recorded acceptance. Phase 0 and the
pre-PR manual walkthrough (§8 of the workflow) need the owner.

### Phase 0 — Close owner questions

Goal: no open question blocks implementation, and every durable answer is a decision.

Work:

- Ask the owner the questions in §8 and record the answers as `D-259` and following in
  [Decisions](specification/Decisions.md).
- Fix the supported subset, parser choice, profile keys, selection rules, channel-mapping rule,
  DC default policy, mismatch policy, diagnostic codes, corpus policy, and shim ABI plan in this
  plan.
- Confirm the corpus measurement in §5 and the redistribution status of the ASIX sample ESI.

Exit criteria: every §8 question has an owner answer; the plan is approved; no implementation
commit exists on the branch before approval.

Evidence: decisions `D-259`–`D-26N`, this plan updated, owner approval recorded in §10.

### Phase 1 — Bounded ESI reader and document model

Goal: a host-independent, bounded reader that turns ESI bytes into a typed description with no
panic on arbitrary input.

Work:

- Implement the reader and the minimal document model (vendor, devices, type identity, syncs,
  PDOs, entries, DC op modes, mailbox flags) in a protocol-neutral module that the manifest and
  runtime can share (proposed: `crates/stc-manifest/src/esi/` so the build path owns it and the
  runtime consumes the embedded result, Q13).
- Enforce the limits and the encoding rules of §6.6; produce typed errors with byte offsets.
- Add the synthetic fixture set (`tests/esi/`) for the shapes in §6.1 and malformed variants.
- Register `fuzz_esi` and run a local campaign.

Exit criteria: synthetic fixtures parse or fail as declared; no panic, hang, or unbounded
allocation on the malformed set; fuzz smoke green; unit tests pass on all CI hosts.

Evidence: unit and fixture tests, fuzz target plus campaign output, updated plan.

### Phase 2 — Subset validation, diagnostics, and deterministic selection

Goal: unsupported or ambiguous ESI input fails deterministically with an actionable message.

Work:

- Implement subset validation (sync/PDO/entry/bit-layout/DC/mailbox rules), the unsupported-field
  accounting, and every `STC-E-IO-ESI-*` code with a test that emits it.
- Implement multi-file and multi-revision selection, the content hash, and the ambiguity and
  duplicate-content rules of §6.2.
- Add the ignore-gated corpus harness (`STC_ESI_CORPUS`) that walks the TwinCAT tree, tallies
  accept/reject with codes, and asserts the measured invariants from §5.

Exit criteria: every malformed, ambiguous, and unsupported fixture and corpus sample fails with
one stable code; selection is deterministic across repeated runs and file orderings; the corpus
tally is recorded.

Evidence: fixture matrix tests, corpus tally in the verification report, fuzz updates.

### Phase 3 — Mapping, compile-time pipeline, and channel validation

Goal: a device that names an ESI source produces a validated effective profile, and a declared
channel that disagrees with the ESI fails the build.

Work:

- Map a validated description to `syncs`/`pdo-channels`/`dc` and merge it into the effective
  profile with the precedence in §6.3; reject a device that also hand-writes a map.
- Wire the resolution into the compile path so the embedded profile is authoritative, with the
  resolved identity and content hash recorded in the metadata; no run-time file read.
- Add the compiler-side channel-width validation and the route/width checks that already exist
  must keep working for ESI-derived channels.
- Implement `esi-strict`, `esi-dir`, revision pinning, and the build-log lines that name the
  chosen file, revision, and hash.
- Add driver-level tests: valid project builds and embeds; every invalid project fails with the
  documented code and writes no artifact.

Exit criteria: an ESI-described slave produces validated channels/PDOs and IgH configuration
(the profile the runtime consumes); a width or mapping mismatch fails the build; the artifact is
self-contained and byte-reproducible for the same inputs.

Evidence: `stc-manifest` and driver tests, metadata inspection, updated
[CLI and Manifest](specification/Architecture/CLI-and-Manifest.md) §2.2.

### Phase 4 — Runtime startup verification and mismatch policy

Goal: the runtime proves the discovered slave is the slave it was configured for.

Work:

- Extend the shim to ABI `stc-io-ethercat/2` with identity and PDO-assignment read-back; keep the
  loader's ABI 1 path working and report verification as unavailable.
- Implement the check before activation, the fail-closed policy, the diagnostic text, the status
  and `stc run -v` fields, and the `STC-W-IO-ETH-004` path.
- Extend the mock shim with identity/PDO responses and add negative tests: wrong vendor, wrong
  product, wrong revision, wrong PDO index list, wrong total bit length, ABI 1 shim, and a shim
  that returns an empty assignment.
- Keep the recovery policy unchanged: a post-start identity change is a fieldbus fault, not a
  reconfiguration.

Exit criteria: every mismatch case fails cold start with expected-vs-discovered values and no
cycle; the ABI 1 path is explicit and warning-diagnosed; existing EtherCAT tests stay green.

Evidence: runtime backend tests, shim ABI tests, updated
[I/O Routing and Backends](specification/Architecture/IO-Routing-and-Backends.md) §2.2/§5.

### Phase 5 — Corpus campaign, simulator, and HIL evidence

Goal: evidence that the subset handles real vendor files and the bench device.

Work:

- Run the full TwinCAT corpus through the harness and record the tally by code and by file size
  band; investigate every unexpected rejection and either fix it or document the exclusion.
- Run a longer fuzz campaign on the Linux fuzz host and record executions, coverage edges, and
  artifacts (none expected).
- Replace the hand-written DIEWU profile with an ESI-derived profile for the same identity in a
  parallel bench project, keeping the existing `tests/ethercat/diewu` profile as the regression
  baseline; run the open-ended loopback and the PLC-driven output checks on `ecat`.
- Record the HIL run: host, OS, IgH version, shim ABI, ESI source and hash, commands, and output;
  include one deliberate mismatch run (wrong revision pin) to capture the fail-closed diagnostic
  on hardware.

Exit criteria: corpus and fuzz results recorded; the ESI-driven bench project runs the same
input/output paths as the hand-written profile; the deliberate mismatch fails as designed on
hardware; no hard-real-time or safety claim is added.

Evidence: corpus tally, fuzz output, HIL log and commands, updated
[EtherCAT-Runbook.md](specification/Verification/EtherCAT-Runbook.md).

### Phase 6 — Documentation, manual walkthrough, and PR

Goal: the durable contract and the operator story are in the specification tree, and the
walkthrough is approved.

Work:

- Write the durable contract into
  [I/O Routing and Backends](specification/Architecture/IO-Routing-and-Backends.md) (subset,
  selection, mapping, verification, failure policy),
  [I/O and Devices](specification/Architecture/IO-and-Devices.md),
  [CLI and Manifest](specification/Architecture/CLI-and-Manifest.md) (profile keys),
  [Decisions](specification/Decisions.md), [USER-GUIDE.md](USER-GUIDE.md), and
  [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) (documented unsupported fields, ABI 1 shim,
  corpus policy, no SDO startup parameters if Q7 defers them).
- Create `specification/Verification/ESI-Configuration.md` with the subset table, evidence, corpus
  tally, fuzz campaign, simulator results, and HIL run.
- Run the manual walkthrough: build the bench project from the branch, run it against the DIEWU
  slave on `ecat`, edit the ESI revision pin to force a mismatch, and capture output.
- Open the pull request against `v3.0.0-planning` with the summary, verification evidence, CI
  links, and exclusions.

Exit criteria: durable docs current; walkthrough evidence approved by the owner; PR CI green;
the item is not marked complete and the branch is not merged or deleted by the agent.

Evidence: documentation commits, `ESI-Configuration.md`, walkthrough log, PR link and CI run.

## 8. Owner questions (Phase 0)

Each question carries a proposed default so implementation is not blocked on wording.

1. **Q1 — parser.** Project-owned bounded XML reader (matches the Modbus/MQTT precedent, no new
   dependency, more parsing code to fuzz) or a vetted crate (`quick-xml`, MIT, no unsafe in the
   path)? *Proposed: project-owned, because the ESI subset is small and the repository has
   avoided XML dependencies so far; revisit if Phase 1 shows the reader is the dominant cost.*
2. **Q2 — where ESI lives.** Per-device `esi`/`esi-dir` keys in the machine profile only, or also
   an `[io].esi-dir` project-wide library plus inline `[io.devices].config.esi`? *Proposed:
   per-device profile keys plus an `[io].esi-dir` default library; no inline device ESI.*
3. **Q3 — corpus.** Is the TwinCAT tree usable as development/manual input only (never committed,
   never in CI), and is the ASIX AX58100 sample ESI redistributable for a committed fixture?
   *Proposed: yes to development-only use; commit synthetic fixtures until the owner confirms the
   ASIX licence in writing.*
4. **Q4 — ESI versions.** Accept only `EtherCATInfo Version="1.2"`, or also 1.0/1.1? *Proposed:
   1.2 only, with a clear diagnostic; the corpus is 1.2 throughout.*
5. **Q5 — selection.** Highest revision when unpinned, or require an explicit pin for every
   production profile? *Proposed: highest revision with the choice logged, exact pin supported,
   ambiguity always an error.*
6. **Q6 — precedence.** If a device has both an ESI source and hand-written `syncs`/
   `pdo-channels`, is that an error, or does the explicit map override the ESI? *Proposed: error
   (`STC-E-IO-ESI-007`); one source of truth per device.*
7. **Q7 — SDO startup parameters.** The subset parses mailbox/CoE flags. Do we ship startup SDO
   writes (`<Mailbox><CoE><InitCmd>` and object-dictionary-driven startup) in WL-7? *Proposed:
   no — parse and report as not applied; startup parameters are a follow-up, because SDO
   execution adds a new runtime state machine and is not needed for the bench device.*
8. **Q8 — channel exposure.** One channel per ESI entry (sub-bit slicing only when the user
   declares `bit`/`width`), or auto-generate one channel per bit for `BITARRn`/`BOOL` entries?
   *Proposed: one channel per entry; explicit slicing only, so channel names stay predictable and
   the process image does not silently grow.*
9. **Q9 — DC defaults.** When the ESI's DC op mode supplies `AssignActivate`/cycle/shift, are
   those applied by default or only when the profile opts in? *Proposed: applied as defaults with
   the chosen op mode named in the build log; an explicit `dc` table wins field by field.*
10. **Q10 — mismatch policy.** Fail closed on identity/PDO mismatch (proposed), or allow an
    explicit `esi-strict = false` warning mode? *Proposed: fail closed only; a wrong layout must
    never run silently.*
11. **Q11 — diagnostic namespace.** New `STC-E-IO-ESI-00N` build-time family plus
    `STC-W-IO-ETH-004`, or fold build-time failures into `STC-E-PROJECT-0NN` because they are
    manifest/profile errors? *Proposed: the separate `ESI` family, because the failure is about
    the vendor document, not the project file, and it keeps `STC-E-PROJECT` stable.*
12. **Q12 — limits.** Default file-size cap 32 MiB (corpus maximum is 70.6 MB, so the cap would
    reject the largest Beckhoff terminal families), depth 64, 1 000 000 elements — or a
    128 MiB cap that accepts the whole corpus? *Proposed: 128 MiB default with a documented
    override, because rejecting a legitimate vendor terminal file on size is a usability defect;
    the element/depth limits carry the real DoS protection.*
13. **Q13 — module home.** Put the ESI reader in `stc-manifest` (build path, shared with the
    runtime through the embedded profile) or a new crate? *Proposed: `stc-manifest`, so the build
    path owns ingestion and the runtime never parses ESI.*
14. **Q14 — shim ABI.** Bump to `stc-io-ethercat/2` with identity/PDO read-back and keep ABI 1
    accepted-with-warning, or require ABI 2 for maintenance simplicity? *Proposed: bump to 2 and
    keep 1 working with `STC-W-IO-ETH-004`, because existing users' shim libraries should not
    stop loading; the shipped source is rebuilt for HIL.*

## 9. Dependencies and risks

- WL-5.2 is delivered: IgH 1.6.13, the DIEWU bench slave, the shim, and the static profile path
  exist. No target is a prerequisite.
- ESI is vendor-extended XML. The subset and the rejection policy are the primary design risk;
  the corpus tally in Phase 5 is the mitigation and the evidence.
- Real corpus files are large (up to 70.6 MB) and include ISO-8859-1 documents; the reader's
  limits and encoding rules must be decided before the parser is written, not after.
- PDO assignment read-back needs a live slave in PREOP; the mock shim must model it or the
  verification path stays untested on non-Linux hosts. The misconfiguration test is the
  compensating evidence.
- The shim ABI bump is a compatibility surface: an old `.so` must not break cold start, and the
  warning path must be tested.
- HIL is limited to the DIEWU 16I/16O bench. Servo-drive, multi-slave chain, and distributed-clock
  behavior beyond `AssignActivate` are not claimed.
- ESI parsing is untrusted-input handling. No entity expansion, bounded sizes, fuzzing, and
  explicit rejection are the contract; a crash on a vendor file is a bug, not a limitation.
- Adding an ESI source changes a device's effective configuration hash, so a project that
  switches from a hand-written map to ESI needs a cold download; the online-change error must be
  the existing `STC-E-ONLINE-IO-001` with a clear message, not a silent swap.

## 10. Acceptance log

| Phase | Accepted | Commit | CI run | Notes |
| --- | --- | --- | --- | --- |
| 0 | | | | Questions 1–14 open; plan awaiting owner approval |

## 11. Change log

| Date | Change |
| --- | --- |
| 2026-11-15 | Plan drafted on branch `WL-7` from `v3.0.0-planning` (`7e053a4`); TwinCAT ESI corpus measured (458 files, 387 `.xml`, 144 with `<Device>`, ESI 1.2, UTF-8 and ISO-8859-1, largest 70.6 MB); submitted for owner approval. |
