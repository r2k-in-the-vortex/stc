//! Desired EtherCAT device configuration: named mode bundles (WL-7 phase 3).
//!
//! A mode is a generic, engineer-authored bundle for one device (D-275): its
//! canonical PDO configuration with the effective declared-entry remap, its
//! channel exposure, its ordered startup plan and one explicit DC choice.
//! Exactly one mode is selected per device; alternates are stored but inert
//! until explicitly selected. The generic schema carries no vendor motion
//! semantics: vendor meaning lives in the name, description and authored
//! startup values and is never interpreted here.
//!
//! The desired configuration is a JSON draft in this phase, exactly like the
//! phase-2 topology draft. The durable project schema declared by an `[io]` key
//! is Phase 5 work (D-263, Q9/Q24); Phase 5 replaces the draft file and keeps
//! the typed model.

use std::collections::{BTreeMap, BTreeSet};

use stc_got::json::{self, Json};

use super::diagnostics::{EngineeringDiagnostic, codes};
use super::esi::model::{DcOpMode, MailboxCapabilities, PdoDirection};
use super::pdo::{EntryKey, PdoCapabilities, PdoSelection, SelectedPdo};
use super::topology::{ConfiguredDevice, DeviceIdentity, Topology};

/// Format version of the engineering configuration draft.
pub const CONFIG_FORMAT: u32 = 1;

/// Protocol name of the engineering configuration draft.
pub const CONFIG_PROTOCOL: &str = "stc-ethercat-config/1";

/// The accepted ESI description a configuration was authored against.
///
/// The source reference and the content digest are persisted configuration
/// inputs, not provenance-only metadata: changing either requires an explicit
/// re-import and forces a cold download (D-263, D-267).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EsiReference {
    /// Project-relative source path of the accepted description.
    pub source: String,
    /// Digest of that description.
    pub digest: String,
}

impl EsiReference {
    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("source".into(), Json::string(self.source.as_str())),
            ("digest".into(), Json::string(self.digest.as_str())),
        ])
    }
}

/// The persisted CoE capability gates used to revalidate a remap offline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MailboxFacts {
    /// The device declares `PdoAssign`.
    pub pdo_assign: bool,
    /// The device declares `PdoConfig`.
    pub pdo_config: bool,
}

impl MailboxFacts {
    /// Projects the ESI mailbox flags.
    #[must_use]
    pub fn from_mailbox(mailbox: &MailboxCapabilities) -> Self {
        Self {
            pdo_assign: mailbox.pdo_assign,
            pdo_config: mailbox.pdo_config,
        }
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(self) -> Json {
        Json::object([
            ("pdo_assign".into(), Json::Bool(self.pdo_assign)),
            ("pdo_config".into(), Json::Bool(self.pdo_config)),
        ])
    }
}

/// The explicit distributed-clock choice of one mode (D-276).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DcChoice {
    /// Free-run: no DC configuration is applied.
    FreeRun,
    /// One ESI-declared op mode with its resolved effective timing.
    OpMode(DcTiming),
}

impl DcChoice {
    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        match self {
            Self::FreeRun => Json::object([("mode".into(), Json::string("free-run"))]),
            Self::OpMode(timing) => Json::object([
                ("mode".into(), Json::string("op-mode")),
                ("name".into(), Json::string(timing.name.as_str())),
                ("index".into(), Json::unsigned(timing.index as u64)),
                (
                    "assign_activate".into(),
                    Json::unsigned(u64::from(timing.assign_activate)),
                ),
                ("sync0_cycle".into(), Json::unsigned(timing.sync0_cycle)),
                (
                    "sync0_shift".into(),
                    Json::signed(i64::from(timing.sync0_shift)),
                ),
                (
                    "sync1_cycle".into(),
                    Json::optional(timing.sync1_cycle.map(Json::unsigned)),
                ),
                (
                    "sync1_shift".into(),
                    Json::optional(
                        timing
                            .sync1_shift
                            .map(|value| Json::signed(i64::from(value))),
                    ),
                ),
            ]),
        }
    }

    /// Validates the explicit choice and its resolved effective values.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-023` for a missing or ambiguous declared op mode,
    /// a device without a DC section, an edited `AssignActivate`, an
    /// unresolvable factor, a persisted effective value that disagrees with
    /// the declaration, or an invalid Sync0/Sync1 combination.
    pub fn validate(
        &self,
        op_modes: Option<&[DcOpMode]>,
        runtime_cycle_ns: u64,
        position: u16,
        device: &str,
    ) -> Result<(), EngineeringDiagnostic> {
        let Self::OpMode(timing) = self else {
            return Ok(());
        };
        let Some(op_modes) = op_modes else {
            return Err(dc_error(
                device,
                position,
                "one ESI-declared DC op mode or `free-run`",
                "the ESI declares no `<Dc>` section",
            ));
        };
        let Some(declared) = op_modes
            .iter()
            .find(|mode| mode.index == timing.index && mode.name == timing.name)
        else {
            let duplicate = op_modes
                .iter()
                .filter(|mode| mode.name == timing.name)
                .count()
                > 1;
            return Err(dc_error(
                device,
                position,
                &format!(
                    "an ESI-declared op mode named `{}` with declaration index {}",
                    timing.name, timing.index
                ),
                if duplicate {
                    "the name is duplicated and the stored declaration index does not match"
                } else {
                    "no declared op mode matches"
                },
            ));
        };
        if timing.assign_activate != declared.assign_activate {
            return Err(dc_error(
                device,
                position,
                &format!(
                    "`AssignActivate` #x{:04x} from the ESI",
                    declared.assign_activate
                ),
                &format!("#x{:04x} in the configuration", timing.assign_activate),
            ));
        }
        let expected_sync0 = resolve_cycle(
            declared.sync0_cycle,
            declared.sync0_factor,
            runtime_cycle_ns,
        )
        .map_err(|observed| dc_error(device, position, "a resolvable Sync0 cycle", &observed))?;
        if timing.sync0_cycle != expected_sync0 {
            return Err(dc_error(
                device,
                position,
                &format!("resolved Sync0 cycle {expected_sync0} ns"),
                &format!("{} ns in the configuration", timing.sync0_cycle),
            ));
        }
        validate_shift(
            device,
            position,
            "Sync0",
            timing.sync0_shift,
            expected_sync0,
        )?;
        match (declared.sync1_cycle, timing.sync1_cycle) {
            (None, None) => {
                if timing.sync1_shift.is_some() {
                    return Err(dc_error(
                        device,
                        position,
                        "no Sync1 shift when the ESI declares no Sync1",
                        "a Sync1 shift in the configuration",
                    ));
                }
            }
            (Some(cycle), Some(effective)) => {
                let expected =
                    resolve_cycle(cycle, None, runtime_cycle_ns).map_err(|observed| {
                        dc_error(device, position, "a resolvable Sync1 cycle", &observed)
                    })?;
                if effective != expected {
                    return Err(dc_error(
                        device,
                        position,
                        &format!("resolved Sync1 cycle {expected} ns"),
                        &format!("{effective} ns in the configuration"),
                    ));
                }
                if effective < expected_sync0 || effective % expected_sync0 != 0 {
                    return Err(dc_error(
                        device,
                        position,
                        "a Sync1 cycle that is a multiple of Sync0",
                        &format!("Sync1 {effective} ns, Sync0 {expected_sync0} ns"),
                    ));
                }
                match (declared.sync1_shift, timing.sync1_shift) {
                    (None, None) => {}
                    (Some(shift), Some(effective_shift)) => {
                        if shift != effective_shift {
                            return Err(dc_error(
                                device,
                                position,
                                &format!("Sync1 shift {shift} ns from the ESI"),
                                &format!("{effective_shift} ns in the configuration"),
                            ));
                        }
                        validate_shift(device, position, "Sync1", effective_shift, effective)?;
                    }
                    _ => {
                        return Err(dc_error(
                            device,
                            position,
                            "the declared Sync1 shift presence",
                            "a different Sync1 shift presence in the configuration",
                        ));
                    }
                }
            }
            _ => {
                return Err(dc_error(
                    device,
                    position,
                    "the declared Sync1 cycle presence",
                    "a different Sync1 cycle presence in the configuration",
                ));
            }
        }
        Ok(())
    }

    /// Structural validation without the ESI, using the persisted effective
    /// timing only (D-263, D-276).
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-023` for an empty name, a zero Sync0 cycle, a
    /// shift outside its cycle, a Sync1 cycle that is not a multiple of Sync0,
    /// or a Sync1 shift without a Sync1 cycle.
    pub fn validate_facts(&self, position: u16, device: &str) -> Result<(), EngineeringDiagnostic> {
        let Self::OpMode(timing) = self else {
            return Ok(());
        };
        if timing.name.trim().is_empty() {
            return Err(dc_error(
                device,
                position,
                "a non-empty declared op-mode name",
                "an empty op-mode name",
            ));
        }
        if timing.sync0_cycle == 0 {
            return Err(dc_error(
                device,
                position,
                "a non-zero resolved Sync0 cycle",
                "a zero Sync0 cycle",
            ));
        }
        validate_shift(
            device,
            position,
            "Sync0",
            timing.sync0_shift,
            timing.sync0_cycle,
        )?;
        match (timing.sync1_cycle, timing.sync1_shift) {
            (None, None) => {}
            (None, Some(_)) => {
                return Err(dc_error(
                    device,
                    position,
                    "no Sync1 shift when no Sync1 cycle is configured",
                    "a Sync1 shift without a Sync1 cycle",
                ));
            }
            (Some(cycle), shift) => {
                if cycle < timing.sync0_cycle || cycle % timing.sync0_cycle != 0 {
                    return Err(dc_error(
                        device,
                        position,
                        "a Sync1 cycle that is a multiple of Sync0",
                        &format!("Sync1 {cycle} ns, Sync0 {} ns", timing.sync0_cycle),
                    ));
                }
                if let Some(shift) = shift {
                    validate_shift(device, position, "Sync1", shift, cycle)?;
                }
            }
        }
        Ok(())
    }
}

/// The resolved effective timing of one declared DC op mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DcTiming {
    /// Declared op-mode name.
    pub name: String,
    /// Declaration index, which disambiguates duplicate names.
    pub index: usize,
    /// `AssignActivate` word, copied from the ESI and never edited.
    pub assign_activate: u32,
    /// Effective Sync0 cycle in nanoseconds.
    pub sync0_cycle: u64,
    /// Sync0 shift in nanoseconds.
    pub sync0_shift: i32,
    /// Effective Sync1 cycle in nanoseconds, when declared.
    pub sync1_cycle: Option<u64>,
    /// Sync1 shift in nanoseconds, when declared.
    pub sync1_shift: Option<i32>,
}

impl DcTiming {
    /// Resolves the effective timing of one declared op mode.
    ///
    /// # Errors
    ///
    /// Returns the reason a factor or declared cycle cannot be resolved.
    pub fn resolve(mode: &DcOpMode, runtime_cycle_ns: u64) -> Result<Self, String> {
        let sync0_cycle = resolve_cycle(mode.sync0_cycle, mode.sync0_factor, runtime_cycle_ns)?;
        let sync1_cycle = mode
            .sync1_cycle
            .map(|cycle| resolve_cycle(cycle, None, runtime_cycle_ns))
            .transpose()?;
        Ok(Self {
            name: mode.name.clone(),
            index: mode.index,
            assign_activate: mode.assign_activate,
            sync0_cycle,
            sync0_shift: mode.sync0_shift,
            sync1_cycle,
            sync1_shift: mode.sync1_shift,
        })
    }
}

fn resolve_cycle(declared: u32, factor: Option<u32>, runtime_cycle_ns: u64) -> Result<u64, String> {
    match factor {
        Some(factor) => {
            if factor == 0 {
                return Err("the declared factor is zero".into());
            }
            if runtime_cycle_ns == 0 {
                return Err("the configured runtime cycle is zero".into());
            }
            runtime_cycle_ns
                .checked_mul(u64::from(factor))
                .ok_or_else(|| {
                    format!("cycle {runtime_cycle_ns} ns times factor {factor} overflows")
                })
        }
        None => {
            if declared == 0 {
                return Err("the declared cycle is zero and has no factor".into());
            }
            Ok(u64::from(declared))
        }
    }
}

fn validate_shift(
    device: &str,
    position: u16,
    sync: &str,
    shift: i32,
    cycle: u64,
) -> Result<(), EngineeringDiagnostic> {
    let magnitude = u64::from(shift.unsigned_abs());
    if magnitude >= cycle {
        return Err(dc_error(
            device,
            position,
            &format!("a {sync} shift inside the {cycle} ns cycle"),
            &format!("{shift} ns shift"),
        ));
    }
    Ok(())
}

/// One explicit channel exposure of a selected effective entry (D-273).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelExposure {
    /// Engineer-authored channel name, unique per device.
    pub name: String,
    /// Selected PDO the entry belongs to.
    pub pdo: u16,
    /// Effective entry after remapping.
    pub entry: EntryKey,
    /// Declared PDO direction; must match the PDO.
    pub direction: PdoDirection,
    /// Start bit inside the entry.
    pub bit: u32,
    /// Channel width in bits.
    pub width: u32,
}

impl ChannelExposure {
    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("name".into(), Json::string(self.name.as_str())),
            ("pdo".into(), Json::unsigned(u64::from(self.pdo))),
            ("entry".into(), self.entry.to_json()),
            ("direction".into(), Json::string(self.direction.as_str())),
            ("bit".into(), Json::unsigned(u64::from(self.bit))),
            ("width".into(), Json::unsigned(u64::from(self.width))),
        ])
    }
}

/// Validates every channel exposure against the effective selection.
///
/// # Errors
///
/// Returns `STC-E-IO-ETH-024` for an unselected PDO or entry, a direction
/// mismatch, a width that overruns the entry, a misaligned multi-bit slice, an
/// overlapping slice, an empty name or a duplicate name.
pub fn validate_channels(
    channels: &[ChannelExposure],
    pdos: &PdoSelection,
    position: u16,
    device: &str,
) -> Result<(), EngineeringDiagnostic> {
    let mut names = BTreeSet::new();
    let mut ranges: BTreeMap<(PdoDirection, u16, EntryKey), Vec<(u32, u32)>> = BTreeMap::new();
    for channel in channels {
        if channel.name.trim().is_empty() {
            return Err(channel_error(
                device,
                position,
                "a non-empty engineer-authored channel name",
                "an empty channel name",
            ));
        }
        if !names.insert(channel.name.as_str()) {
            return Err(channel_error(
                device,
                position,
                "unique channel names per device",
                &format!("duplicate channel name `{}`", channel.name),
            ));
        }
        let Some(pdo) = pdos.pdo(channel.pdo, channel.direction) else {
            return Err(channel_error(
                device,
                position,
                &format!(
                    "a selected {} PDO 0x{:04x}",
                    channel.direction.as_str(),
                    channel.pdo
                ),
                &format!("channel `{}` targets an unselected PDO", channel.name),
            ));
        };
        let Some(entry) = pdo
            .entries
            .iter()
            .find(|entry| entry.key() == channel.entry)
        else {
            return Err(channel_error(
                device,
                position,
                &format!(
                    "effective entry {} of selected PDO 0x{:04x}",
                    channel.entry.describe(),
                    channel.pdo
                ),
                &format!(
                    "channel `{}` targets an entry the selected mode does not expose",
                    channel.name
                ),
            ));
        };
        if channel.width == 0 {
            return Err(channel_error(
                device,
                position,
                "a non-zero channel width",
                &format!("channel `{}` has width 0", channel.name),
            ));
        }
        if channel.bit.saturating_add(channel.width) > u32::from(entry.bit_len) {
            return Err(channel_error(
                device,
                position,
                &format!(
                    "channel `{}` to fit entry {} ({} bits)",
                    channel.name,
                    channel.entry.describe(),
                    entry.bit_len
                ),
                &format!(
                    "bits {}..{}",
                    channel.bit,
                    channel.bit.saturating_add(channel.width)
                ),
            ));
        }
        if channel.width > 1 && channel.bit % 8 != 0 {
            return Err(channel_error(
                device,
                position,
                "a multi-bit slice to start on a byte boundary",
                &format!("channel `{}` starts at bit {}", channel.name, channel.bit),
            ));
        }
        let slots = ranges
            .entry((channel.direction, channel.pdo, channel.entry))
            .or_default();
        for (start, width) in slots.iter() {
            if channel.bit < start.saturating_add(*width)
                && *start < channel.bit.saturating_add(channel.width)
            {
                return Err(channel_error(
                    device,
                    position,
                    "non-overlapping channel slices",
                    &format!(
                        "channel `{}` overlaps bits {}..{}",
                        channel.name,
                        start,
                        start.saturating_add(*width)
                    ),
                ));
            }
        }
        slots.push((channel.bit, channel.width));
    }
    Ok(())
}

/// One generic startup operation (D-274).
///
/// Generic startup operations never author PDO assignment or mapping objects:
/// PDO configuration is the declarative [`PdoSelection`] applied through the
/// master's native PDO path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupOperation {
    /// Stable operation id, unique per mode.
    pub id: String,
    /// `write` or `verify`.
    pub action: StartupAction,
    /// Object index.
    pub index: u16,
    /// Object subindex.
    pub subindex: u8,
    /// Typed value.
    pub value: SdoValue,
    /// Required EtherCAT state before the operation.
    pub state: StartupState,
    /// Read-back verification policy.
    pub verify: StartupVerification,
    /// Bounded timeout in milliseconds.
    pub timeout_ms: u32,
}

/// The action of one startup operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupAction {
    /// Write the value and optionally verify the read-back.
    Write,
    /// Read the value back and compare it.
    Verify,
}

impl StartupAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Write => "write",
            Self::Verify => "verify",
        }
    }
}

/// The EtherCAT state precondition of one startup operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupState {
    /// PREOP.
    PreOp,
    /// SAFEOP.
    SafeOp,
}

impl StartupState {
    fn as_str(self) -> &'static str {
        match self {
            Self::PreOp => "preop",
            Self::SafeOp => "safeop",
        }
    }
}

/// The read-back verification policy of one startup operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupVerification {
    /// No read-back.
    None,
    /// Read back when supported; a mismatch still fails.
    BestEffort,
    /// A required read-back; unavailable read-back fails closed.
    Required,
}

impl StartupVerification {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::BestEffort => "best-effort",
            Self::Required => "required",
        }
    }
}

/// A generic typed startup value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SdoValue {
    /// Boolean.
    Bool(bool),
    /// Unsigned integer of the declared width.
    Unsigned(u64),
    /// Signed integer of the declared width.
    Signed(i64),
    /// UTF-8 text.
    Text(String),
    /// Wire-order bytes.
    Bytes(Vec<u8>),
}

impl SdoValue {
    fn type_name(&self) -> &'static str {
        match self {
            Self::Bool(_) => "bool",
            Self::Unsigned(_) => "unsigned",
            Self::Signed(_) => "signed",
            Self::Text(_) => "string",
            Self::Bytes(_) => "bytes",
        }
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        let value = match self {
            Self::Bool(value) => Json::Bool(*value),
            Self::Unsigned(value) => Json::unsigned(*value),
            Self::Signed(value) => Json::signed(*value),
            Self::Text(value) => Json::string(value.as_str()),
            Self::Bytes(bytes) => Json::Array(
                bytes
                    .iter()
                    .map(|byte| Json::unsigned(u64::from(*byte)))
                    .collect(),
            ),
        };
        Json::object([
            ("type".into(), Json::string(self.type_name())),
            ("value".into(), value),
        ])
    }
}

impl StartupOperation {
    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("id".into(), Json::string(self.id.as_str())),
            ("action".into(), Json::string(self.action.as_str())),
            ("index".into(), Json::unsigned(u64::from(self.index))),
            (
                "index_hex".into(),
                Json::string(format!("0x{:04x}", self.index)),
            ),
            ("subindex".into(), Json::unsigned(u64::from(self.subindex))),
            ("value".into(), self.value.to_json()),
            ("state".into(), Json::string(self.state.as_str())),
            ("verify".into(), Json::string(self.verify.as_str())),
            (
                "timeout_ms".into(),
                Json::unsigned(u64::from(self.timeout_ms)),
            ),
        ])
    }
}

/// Whether an object index belongs to PDO assignment/mapping configuration.
///
/// Mapping objects are `0x1600`-`0x17FF` (RxPDO) and `0x1A00`-`0x1BFF`
/// (TxPDO); assignment objects are `0x1C10`-`0x1C2F` (sync managers 0-31).
/// Sync-manager type and parameter objects are not PDO configuration.
#[must_use]
pub fn is_pdo_configuration_object(index: u16) -> bool {
    matches!(index, 0x1600..=0x17ff | 0x1a00..=0x1bff | 0x1c10..=0x1c2f)
}

/// Validates an ordered startup plan.
///
/// # Errors
///
/// Returns `STC-E-IO-ETH-020` for a duplicated or empty id, an unbounded
/// timeout, or a generic operation that owns a PDO assignment/mapping object.
pub fn validate_startup(
    startup: &[StartupOperation],
    position: u16,
    device: &str,
) -> Result<(), EngineeringDiagnostic> {
    let mut ids = BTreeSet::new();
    for operation in startup {
        if operation.id.trim().is_empty() {
            return Err(mode_error(
                "startup.validate",
                device,
                position,
                "a non-empty startup operation id",
                "an empty id",
            ));
        }
        if !ids.insert(operation.id.as_str()) {
            return Err(mode_error(
                "startup.validate",
                device,
                position,
                "unique startup operation ids per mode",
                &format!("duplicate id `{}`", operation.id),
            ));
        }
        if operation.timeout_ms == 0 || operation.timeout_ms > 600_000 {
            return Err(mode_error(
                "startup.validate",
                device,
                position,
                "a timeout in 1..=600000 ms",
                &format!("timeout {} ms", operation.timeout_ms),
            ));
        }
        if is_pdo_configuration_object(operation.index) {
            return Err(mode_error(
                "startup.validate",
                device,
                position,
                "a generic startup operation that does not own PDO configuration",
                &format!(
                    "object 0x{:04x} is a PDO assignment/mapping object; author it in the mode's \
                     PDO configuration",
                    operation.index
                ),
            ));
        }
        if operation.action == StartupAction::Verify
            && operation.verify == StartupVerification::None
        {
            return Err(mode_error(
                "startup.validate",
                device,
                position,
                "a verification for a `verify` operation",
                "`verify` with policy `none`",
            ));
        }
    }
    Ok(())
}

/// One engineer-authored mode bundle for one device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mode {
    /// Stable mode name, unique per device.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Optional ESI DC op-mode or vendor-documentation provenance; descriptive
    /// only and never constraining.
    pub provenance: Option<String>,
    /// Canonical PDO configuration.
    pub pdos: PdoSelection,
    /// Effective channel exposure.
    pub channels: Vec<ChannelExposure>,
    /// Ordered, required generic startup plan.
    pub startup: Vec<StartupOperation>,
    /// Explicit DC choice.
    pub dc: DcChoice,
}

impl Mode {
    /// Validates the mode against the ESI capability model when available.
    ///
    /// # Errors
    ///
    /// Returns the mode, PDO, DC, channel or startup diagnostic that rejects
    /// the bundle.
    pub fn validate(
        &self,
        capabilities: Option<&PdoCapabilities>,
        runtime_cycle_ns: u64,
        position: u16,
        device: &str,
    ) -> Result<(), EngineeringDiagnostic> {
        if self.name.trim().is_empty() {
            return Err(mode_error(
                "mode.validate",
                device,
                position,
                "a non-empty mode name",
                "an empty mode name",
            ));
        }
        match capabilities {
            Some(capabilities) => {
                self.pdos
                    .validate(capabilities, position)
                    .map_err(|mut diagnostic| {
                        diagnostic.device = Some(device.to_owned());
                        diagnostic
                    })?;
                self.dc.validate(
                    capabilities.dc_op_modes.as_deref(),
                    runtime_cycle_ns,
                    position,
                    device,
                )?;
            }
            None => {
                self.pdos.validate_facts(device, position)?;
                self.dc.validate_facts(position, device)?;
            }
        }
        validate_channels(&self.channels, &self.pdos, position, device)?;
        validate_startup(&self.startup, position, device)?;
        Ok(())
    }

    /// Proposes one full-entry channel per selected entry (D-273).
    ///
    /// The proposal is an explicit engineering action: ESI entry names are
    /// suggestions only and are made unique deterministically.
    #[must_use]
    pub fn propose_channels(
        selection: &PdoSelection,
        capabilities: Option<&PdoCapabilities>,
    ) -> Vec<ChannelExposure> {
        let mut used = BTreeSet::new();
        let mut channels = Vec::new();
        for pdo in &selection.pdos {
            for entry in &pdo.entries {
                let key = entry.key();
                let base = capabilities
                    .and_then(|capabilities| capabilities.declared_entry(key, pdo.direction))
                    .and_then(|declared| declared.name.clone())
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| {
                        format!("pdo{:04x}_{:04x}_{}", pdo.index, key.index, key.subindex)
                    });
                let mut name = base.clone();
                let mut suffix = 1u32;
                while !used.insert(name.clone()) {
                    name = format!("{base}_{suffix}");
                    suffix += 1;
                }
                channels.push(ChannelExposure {
                    name,
                    pdo: pdo.index,
                    entry: key,
                    direction: pdo.direction,
                    bit: 0,
                    width: u32::from(entry.bit_len),
                });
            }
        }
        channels
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("name".into(), Json::string(self.name.as_str())),
            (
                "description".into(),
                Json::optional(
                    self.description
                        .as_ref()
                        .map(|text| Json::string(text.as_str())),
                ),
            ),
            (
                "provenance".into(),
                Json::optional(
                    self.provenance
                        .as_ref()
                        .map(|text| Json::string(text.as_str())),
                ),
            ),
            ("pdos".into(), self.pdos.to_json()),
            (
                "channels".into(),
                Json::Array(self.channels.iter().map(ChannelExposure::to_json).collect()),
            ),
            (
                "startup".into(),
                Json::Array(self.startup.iter().map(StartupOperation::to_json).collect()),
            ),
            ("dc".into(), self.dc.to_json()),
        ])
    }
}

/// One configured device with its stored modes and explicit selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceModeSet {
    /// Logical device name; unique per master.
    pub name: String,
    /// Machine slot.
    pub position: u16,
    /// Exact identity basis.
    pub identity: DeviceIdentity,
    /// Accepted ESI source reference and digest.
    pub esi: Option<EsiReference>,
    /// Persisted CoE capability gates.
    pub mailbox: MailboxFacts,
    /// Name of the selected mode.
    pub selected: String,
    /// Stored modes, in authoring order.
    pub modes: Vec<Mode>,
}

impl DeviceModeSet {
    /// Returns the explicitly selected mode.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-020` when no mode is selected or the selection
    /// does not resolve.
    pub fn selected_mode(&self) -> Result<&Mode, EngineeringDiagnostic> {
        if self.selected.trim().is_empty() {
            return Err(mode_error(
                "mode.select",
                &self.name,
                self.position,
                "an explicit selected mode",
                "no mode is selected",
            ));
        }
        self.modes
            .iter()
            .find(|mode| mode.name == self.selected)
            .ok_or_else(|| {
                mode_error(
                    "mode.select",
                    &self.name,
                    self.position,
                    &format!("a stored mode named `{}`", self.selected),
                    "the selected mode is not stored",
                )
            })
    }

    /// Validates the device's modes, selection, ESI reference and exposure.
    ///
    /// `bound_channels` are the channel names the project declares and binds
    /// for this device (`stc.toml` owns them). An alternate mode is inert; only
    /// the selected mode must expose every bound channel.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-020` for an invalid device/mode/ESI/projection,
    /// `021` for an invalid PDO selection, `023` for an invalid DC choice and
    /// `024` for an invalid or missing channel exposure.
    pub fn validate(
        &self,
        capabilities: Option<&PdoCapabilities>,
        bound_channels: &[String],
        runtime_cycle_ns: u64,
    ) -> Result<(), EngineeringDiagnostic> {
        if self.name.trim().is_empty() {
            return Err(mode_error(
                "config.validate",
                "<unnamed>",
                self.position,
                "a non-empty logical device name",
                "an empty device name",
            ));
        }
        if self.modes.is_empty() {
            return Err(mode_error(
                "config.validate",
                &self.name,
                self.position,
                "at least one stored mode",
                "no modes",
            ));
        }
        let mut names = BTreeSet::new();
        for mode in &self.modes {
            if !names.insert(mode.name.as_str()) {
                return Err(mode_error(
                    "config.validate",
                    &self.name,
                    self.position,
                    "unique mode names per device",
                    &format!("duplicate mode name `{}`", mode.name),
                ));
            }
        }
        let selected = self.selected_mode()?;

        let Some(esi) = &self.esi else {
            return Err(mode_error(
                "config.validate",
                &self.name,
                self.position,
                "the accepted ESI source reference and digest",
                "no ESI reference is persisted",
            ));
        };
        if esi.source.trim().is_empty() || esi.digest.trim().is_empty() {
            return Err(mode_error(
                "config.validate",
                &self.name,
                self.position,
                "a non-empty ESI source and digest",
                "an incomplete ESI reference",
            ));
        }
        if let Some(capabilities) = capabilities {
            if capabilities.vendor_id != self.identity.vendor_id
                || capabilities.product_code != self.identity.product_code
                || capabilities.revision != self.identity.revision
            {
                return Err(mode_error(
                    "config.validate",
                    &self.name,
                    self.position,
                    &format!("the ESI identity {}", self.identity.describe()),
                    &format!(
                        "the resolved description is #x{:08x}/#x{:08x}/#x{:08x}",
                        capabilities.vendor_id, capabilities.product_code, capabilities.revision
                    ),
                ));
            }
            if capabilities.digest != esi.digest {
                return Err(mode_error(
                    "config.validate",
                    &self.name,
                    self.position,
                    &format!("the persisted ESI digest {}", esi.digest),
                    &format!(
                        "the accepted description digest {}; re-import and review the configuration",
                        capabilities.digest
                    ),
                ));
            }
            let mailbox = MailboxFacts::from_mailbox(&capabilities.mailbox);
            if mailbox != self.mailbox {
                return Err(mode_error(
                    "config.validate",
                    &self.name,
                    self.position,
                    &format!(
                        "persisted mailbox gates PdoAssign={} PdoConfig={}",
                        self.mailbox.pdo_assign, self.mailbox.pdo_config
                    ),
                    &format!(
                        "the ESI declares PdoAssign={} PdoConfig={}",
                        mailbox.pdo_assign, mailbox.pdo_config
                    ),
                ));
            }
        }

        for mode in &self.modes {
            mode.validate(capabilities, runtime_cycle_ns, self.position, &self.name)?;
        }

        let exposed: BTreeSet<&str> = selected
            .channels
            .iter()
            .map(|channel| channel.name.as_str())
            .collect();
        for bound in bound_channels {
            if !exposed.contains(bound.as_str()) {
                return Err(channel_error(
                    &self.name,
                    self.position,
                    &format!(
                        "the selected mode `{}` to expose bound channel `{bound}`",
                        selected.name
                    ),
                    "the selected mode leaves the bound channel unexposed; select another mode or expose the channel",
                ));
            }
        }
        Ok(())
    }

    /// The device as a desired-topology entry, for dense-position validation.
    #[must_use]
    pub fn configured_device(&self) -> ConfiguredDevice {
        ConfiguredDevice {
            name: self.name.clone(),
            position: self.position,
            identity: self.identity,
            serial: None,
            alias: None,
        }
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("name".into(), Json::string(self.name.as_str())),
            ("position".into(), Json::unsigned(u64::from(self.position))),
            (
                "vendor_id".into(),
                Json::unsigned(u64::from(self.identity.vendor_id)),
            ),
            (
                "product_code".into(),
                Json::unsigned(u64::from(self.identity.product_code)),
            ),
            (
                "revision".into(),
                Json::unsigned(u64::from(self.identity.revision)),
            ),
            (
                "esi".into(),
                Json::optional(self.esi.as_ref().map(EsiReference::to_json)),
            ),
            ("mailbox".into(), self.mailbox.to_json()),
            ("selected".into(), Json::string(self.selected.as_str())),
            (
                "modes".into(),
                Json::Array(self.modes.iter().map(Mode::to_json).collect()),
            ),
        ])
    }
}

/// The complete desired engineering configuration for one managed master.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineeringConfiguration {
    /// Managed master index.
    pub master: u16,
    /// Configured devices.
    pub devices: Vec<DeviceModeSet>,
}

impl EngineeringConfiguration {
    /// Creates an empty configuration for one master.
    #[must_use]
    pub fn new(master: u16) -> Self {
        Self {
            master,
            devices: Vec::new(),
        }
    }

    /// The desired topology implied by the configuration.
    #[must_use]
    pub fn topology(&self) -> Topology {
        Topology::with_devices(
            self.master,
            self.devices
                .iter()
                .map(DeviceModeSet::configured_device)
                .collect(),
        )
    }

    /// Returns the device with this logical name.
    #[must_use]
    pub fn device(&self, name: &str) -> Option<&DeviceModeSet> {
        self.devices.iter().find(|device| device.name == name)
    }

    /// Validates the whole configuration offline.
    ///
    /// `resolve` supplies the accepted ESI capability projection for a device,
    /// or `None` when the description is not available on this host; the
    /// persisted facts are then validated without the ESI. `bound` maps a
    /// device name to the channel names the project binds for it.
    #[must_use]
    pub fn validate(
        &self,
        resolve: impl Fn(&DeviceModeSet) -> Option<PdoCapabilities>,
        bound: &BTreeMap<String, Vec<String>>,
        runtime_cycle_ns: u64,
    ) -> ConfigurationReport {
        let mut diagnostics = Vec::new();
        if let Err(diagnostic) = self.topology().validate() {
            diagnostics.push(diagnostic);
        }
        let mut devices = Vec::with_capacity(self.devices.len());
        for device in &self.devices {
            let capabilities = resolve(device);
            let esi_resolved = capabilities.is_some();
            let bound_channels = bound.get(&device.name).cloned().unwrap_or_default();
            let result = device
                .validate(capabilities.as_ref(), &bound_channels, runtime_cycle_ns)
                .err();
            if let Some(diagnostic) = &result {
                diagnostics.push(diagnostic.clone());
            }
            devices.push(DeviceReport {
                name: device.name.clone(),
                position: device.position,
                selected: (!device.selected.trim().is_empty()).then(|| device.selected.clone()),
                esi_resolved,
                result,
            });
        }
        ConfigurationReport {
            master: self.master,
            devices,
            diagnostics,
        }
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("format".into(), Json::unsigned(u64::from(CONFIG_FORMAT))),
            ("protocol".into(), Json::string(CONFIG_PROTOCOL)),
            ("master".into(), Json::unsigned(u64::from(self.master))),
            (
                "devices".into(),
                Json::Array(self.devices.iter().map(DeviceModeSet::to_json).collect()),
            ),
        ])
    }

    /// Deterministic document text.
    #[must_use]
    pub fn to_text(&self) -> String {
        self.to_json().render()
    }

    /// Parses an engineering configuration draft.
    ///
    /// # Errors
    ///
    /// Returns the `STC-E-IO-ETH-020`-`024` diagnostic that rejects a
    /// malformed, unknown or out-of-range field.
    pub fn parse(text: &str) -> Result<Self, EngineeringDiagnostic> {
        let value = json::parse(text).map_err(|error| {
            mode_error(
                "config.parse",
                "<config>",
                0,
                "a JSON engineering configuration",
                &format!("not valid JSON: {}", error.message),
            )
        })?;
        let object = object_value(&value, "config.parse", "<config>", 0)?;
        reject_unknown_keys(
            object,
            &["format", "protocol", "master", "devices"],
            "config.parse",
            "<config>",
            0,
        )?;
        if number(object, "format", "config.parse", "<config>", 0)? != i128::from(CONFIG_FORMAT) {
            return Err(mode_error(
                "config.parse",
                "<config>",
                0,
                &format!("format {CONFIG_FORMAT}"),
                "an unsupported format",
            ));
        }
        let protocol = string(object, "protocol", "config.parse", "<config>", 0)?;
        if protocol != CONFIG_PROTOCOL {
            return Err(mode_error(
                "config.parse",
                "<config>",
                0,
                CONFIG_PROTOCOL,
                &format!("protocol `{protocol}`"),
            ));
        }
        let master = u16::try_from(number(object, "master", "config.parse", "<config>", 0)?)
            .map_err(|_| {
                mode_error(
                    "config.parse",
                    "<config>",
                    0,
                    "a master index in 0..=65535",
                    "a master index out of range",
                )
            })?;
        let entries = array(object, "devices", "config.parse", "<config>", 0)?;
        let mut devices = Vec::with_capacity(entries.len());
        for entry in entries {
            devices.push(parse_device(entry)?);
        }
        Ok(Self { master, devices })
    }
}

/// Per-device validation outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceReport {
    /// Logical device name.
    pub name: String,
    /// Machine slot.
    pub position: u16,
    /// Selected mode name, when explicitly selected.
    pub selected: Option<String>,
    /// Whether the accepted ESI description was resolved on this host.
    pub esi_resolved: bool,
    /// The rejecting diagnostic, when the device is invalid.
    pub result: Option<EngineeringDiagnostic>,
}

impl DeviceReport {
    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("name".into(), Json::string(self.name.as_str())),
            ("position".into(), Json::unsigned(u64::from(self.position))),
            (
                "selected".into(),
                Json::optional(
                    self.selected
                        .as_ref()
                        .map(|name| Json::string(name.as_str())),
                ),
            ),
            ("esi_resolved".into(), Json::Bool(self.esi_resolved)),
            ("valid".into(), Json::Bool(self.result.is_none())),
            (
                "diagnostic".into(),
                Json::optional(self.result.as_ref().map(EngineeringDiagnostic::to_json)),
            ),
        ])
    }
}

/// The outcome of validating a whole configuration offline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigurationReport {
    /// Managed master.
    pub master: u16,
    /// Per-device outcomes, in configuration order.
    pub devices: Vec<DeviceReport>,
    /// Every rejecting diagnostic; empty means valid.
    pub diagnostics: Vec<EngineeringDiagnostic>,
}

impl ConfigurationReport {
    /// Whether the configuration is valid.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("master".into(), Json::unsigned(u64::from(self.master))),
            ("valid".into(), Json::Bool(self.is_valid())),
            (
                "devices".into(),
                Json::Array(self.devices.iter().map(DeviceReport::to_json).collect()),
            ),
            (
                "diagnostics".into(),
                Json::Array(
                    self.diagnostics
                        .iter()
                        .map(EngineeringDiagnostic::to_json)
                        .collect(),
                ),
            ),
        ])
    }
}

fn parse_device(value: &Json) -> Result<DeviceModeSet, EngineeringDiagnostic> {
    let object = object_value(value, "config.parse", "<device>", 0)?;
    let name = string(object, "name", "config.parse", "<device>", 0)?;
    let position =
        u16::try_from(number(object, "position", "config.parse", &name, 0)?).map_err(|_| {
            mode_error(
                "config.parse",
                &name,
                0,
                "a position in 0..=65535",
                "a position out of range",
            )
        })?;
    reject_unknown_keys(
        object,
        &[
            "name",
            "position",
            "vendor_id",
            "product_code",
            "revision",
            "esi",
            "mailbox",
            "selected",
            "modes",
        ],
        "config.parse",
        &name,
        position,
    )?;
    let identity = DeviceIdentity::new(
        u32_field(object, "vendor_id", "config.parse", &name, position)?,
        u32_field(object, "product_code", "config.parse", &name, position)?,
        u32_field(object, "revision", "config.parse", &name, position)?,
    );
    let esi = match object.get("esi") {
        None | Some(Json::Null) => None,
        Some(value) => {
            let reference = object_value(value, "config.parse", &name, position)?;
            reject_unknown_keys(
                reference,
                &["source", "digest"],
                "config.parse",
                &name,
                position,
            )?;
            Some(EsiReference {
                source: string(reference, "source", "config.parse", &name, position)?,
                digest: string(reference, "digest", "config.parse", &name, position)?,
            })
        }
    };
    let mailbox = match object.get("mailbox") {
        Some(value) => {
            let mailbox = object_value(value, "config.parse", &name, position)?;
            reject_unknown_keys(
                mailbox,
                &["pdo_assign", "pdo_config"],
                "config.parse",
                &name,
                position,
            )?;
            MailboxFacts {
                pdo_assign: boolean(mailbox, "pdo_assign", "config.parse", &name, position)?,
                pdo_config: boolean(mailbox, "pdo_config", "config.parse", &name, position)?,
            }
        }
        None => MailboxFacts::default(),
    };
    let selected = string(object, "selected", "config.parse", &name, position)?;
    let modes = array(object, "modes", "config.parse", &name, position)?;
    let mut parsed_modes = Vec::with_capacity(modes.len());
    for mode in modes {
        parsed_modes.push(parse_mode(mode, &name, position)?);
    }
    Ok(DeviceModeSet {
        name,
        position,
        identity,
        esi,
        mailbox,
        selected,
        modes: parsed_modes,
    })
}

fn parse_mode(value: &Json, device: &str, position: u16) -> Result<Mode, EngineeringDiagnostic> {
    let object = object_value(value, "config.parse", device, position)?;
    reject_unknown_keys(
        object,
        &[
            "name",
            "description",
            "provenance",
            "pdos",
            "channels",
            "startup",
            "dc",
        ],
        "config.parse",
        device,
        position,
    )?;
    let name = string(object, "name", "config.parse", device, position)?;
    let description = optional_string(object, "description", "config.parse", device, position)?;
    let provenance = optional_string(object, "provenance", "config.parse", device, position)?;
    let pdos = array(object, "pdos", "config.parse", device, position)?;
    let mut selection = PdoSelection::default();
    for pdo in pdos {
        selection
            .pdos
            .push(parse_selected_pdo(pdo, device, position)?);
    }
    let channels = array(object, "channels", "config.parse", device, position)?;
    let mut parsed_channels = Vec::with_capacity(channels.len());
    for channel in channels {
        parsed_channels.push(parse_channel(channel, device, position)?);
    }
    let startup = array(object, "startup", "config.parse", device, position)?;
    let mut parsed_startup = Vec::with_capacity(startup.len());
    for operation in startup {
        parsed_startup.push(parse_startup(operation, device, position)?);
    }
    let dc = object
        .get("dc")
        .ok_or_else(|| {
            dc_error(
                device,
                position,
                "an explicit `dc` choice (`free-run` or one declared op mode)",
                "no `dc`",
            )
        })
        .and_then(|value| parse_dc(value, device, position))?;
    Ok(Mode {
        name,
        description,
        provenance,
        pdos: selection,
        channels: parsed_channels,
        startup: parsed_startup,
        dc,
    })
}

fn parse_selected_pdo(
    value: &Json,
    device: &str,
    position: u16,
) -> Result<SelectedPdo, EngineeringDiagnostic> {
    let object = object_value(value, "config.parse", device, position)?;
    reject_unknown_keys(
        object,
        &[
            "direction",
            "index",
            "index_hex",
            "sm",
            "fixed",
            "mandatory",
            "exclude",
            "excluded_sm",
            "entries",
        ],
        "config.parse",
        device,
        position,
    )?;
    let direction = match string(object, "direction", "config.parse", device, position)?.as_str() {
        "rx" => PdoDirection::Rx,
        "tx" => PdoDirection::Tx,
        other => {
            return Err(mode_error(
                "config.parse",
                device,
                position,
                "direction `rx` or `tx`",
                &format!("direction `{other}`"),
            ));
        }
    };
    let index = u16_field(object, "index", "config.parse", device, position)?;
    let sm = u8_field(object, "sm", "config.parse", device, position)?;
    let entries = array(object, "entries", "config.parse", device, position)?;
    let mut parsed = Vec::with_capacity(entries.len());
    for entry in entries {
        parsed.push(parse_selected_entry(entry, device, position)?);
    }
    Ok(SelectedPdo {
        direction,
        index,
        sm,
        fixed: boolean_or_default(object, "fixed", "config.parse", device, position)?,
        mandatory: boolean_or_default(object, "mandatory", "config.parse", device, position)?,
        exclude: u16_list_or_empty(object, "exclude", "config.parse", device, position)?,
        excluded_sm: u8_list_or_empty(object, "excluded_sm", "config.parse", device, position)?,
        entries: parsed,
    })
}

fn parse_selected_entry(
    value: &Json,
    device: &str,
    position: u16,
) -> Result<super::pdo::SelectedEntry, EngineeringDiagnostic> {
    let object = object_value(value, "config.parse", device, position)?;
    reject_unknown_keys(
        object,
        &["index", "index_hex", "subindex", "bit_len", "declared_in"],
        "config.parse",
        device,
        position,
    )?;
    Ok(super::pdo::SelectedEntry {
        index: u16_field(object, "index", "config.parse", device, position)?,
        subindex: u8_field(object, "subindex", "config.parse", device, position)?,
        bit_len: u8_field(object, "bit_len", "config.parse", device, position)?,
        declared_in: u16_field(object, "declared_in", "config.parse", device, position)?,
    })
}

fn parse_channel(
    value: &Json,
    device: &str,
    position: u16,
) -> Result<ChannelExposure, EngineeringDiagnostic> {
    let object = object_value(value, "config.parse", device, position)?;
    reject_unknown_keys(
        object,
        &["name", "pdo", "entry", "direction", "bit", "width"],
        "config.parse",
        device,
        position,
    )?;
    let direction = match string(object, "direction", "config.parse", device, position)?.as_str() {
        "rx" => PdoDirection::Rx,
        "tx" => PdoDirection::Tx,
        other => {
            return Err(mode_error(
                "config.parse",
                device,
                position,
                "direction `rx` or `tx`",
                &format!("direction `{other}`"),
            ));
        }
    };
    let entry = object
        .get("entry")
        .ok_or_else(|| {
            mode_error(
                "config.parse",
                device,
                position,
                "an `entry` object",
                "no `entry`",
            )
        })
        .and_then(|value| {
            let entry = object_value(value, "config.parse", device, position)?;
            reject_unknown_keys(
                entry,
                &["index", "index_hex", "subindex"],
                "config.parse",
                device,
                position,
            )?;
            Ok(EntryKey::new(
                u16_field(entry, "index", "config.parse", device, position)?,
                u8_field(entry, "subindex", "config.parse", device, position)?,
            ))
        })?;
    Ok(ChannelExposure {
        name: string(object, "name", "config.parse", device, position)?,
        pdo: u16_field(object, "pdo", "config.parse", device, position)?,
        entry,
        direction,
        bit: u32_field(object, "bit", "config.parse", device, position)?,
        width: u32_field(object, "width", "config.parse", device, position)?,
    })
}

fn parse_startup(
    value: &Json,
    device: &str,
    position: u16,
) -> Result<StartupOperation, EngineeringDiagnostic> {
    let object = object_value(value, "config.parse", device, position)?;
    reject_unknown_keys(
        object,
        &[
            "id",
            "action",
            "index",
            "index_hex",
            "subindex",
            "value",
            "state",
            "verify",
            "timeout_ms",
        ],
        "config.parse",
        device,
        position,
    )?;
    let action = match string(object, "action", "config.parse", device, position)?.as_str() {
        "write" => StartupAction::Write,
        "verify" => StartupAction::Verify,
        other => {
            return Err(mode_error(
                "config.parse",
                device,
                position,
                "action `write` or `verify`",
                &format!("action `{other}`"),
            ));
        }
    };
    let state = match string(object, "state", "config.parse", device, position)?.as_str() {
        "preop" => StartupState::PreOp,
        "safeop" => StartupState::SafeOp,
        other => {
            return Err(mode_error(
                "config.parse",
                device,
                position,
                "state `preop` or `safeop`",
                &format!("state `{other}`"),
            ));
        }
    };
    let verify = match string(object, "verify", "config.parse", device, position)?.as_str() {
        "none" => StartupVerification::None,
        "best-effort" => StartupVerification::BestEffort,
        "required" => StartupVerification::Required,
        other => {
            return Err(mode_error(
                "config.parse",
                device,
                position,
                "verify `none`, `best-effort` or `required`",
                &format!("verify `{other}`"),
            ));
        }
    };
    let value = object
        .get("value")
        .ok_or_else(|| {
            mode_error(
                "config.parse",
                device,
                position,
                "a typed `value`",
                "no `value`",
            )
        })
        .and_then(|value| parse_sdo_value(value, device, position))?;
    Ok(StartupOperation {
        id: string(object, "id", "config.parse", device, position)?,
        action,
        index: u16_field(object, "index", "config.parse", device, position)?,
        subindex: u8_field(object, "subindex", "config.parse", device, position)?,
        value,
        state,
        verify,
        timeout_ms: u32_field(object, "timeout_ms", "config.parse", device, position)?,
    })
}

fn parse_sdo_value(
    value: &Json,
    device: &str,
    position: u16,
) -> Result<SdoValue, EngineeringDiagnostic> {
    let object = object_value(value, "config.parse", device, position)?;
    reject_unknown_keys(object, &["type", "value"], "config.parse", device, position)?;
    let kind = string(object, "type", "config.parse", device, position)?;
    let raw = object.get("value").ok_or_else(|| {
        mode_error(
            "config.parse",
            device,
            position,
            "a `value` field",
            "no `value`",
        )
    })?;
    let observed = || {
        mode_error(
            "config.parse",
            device,
            position,
            &format!("a `{kind}` value"),
            "a value of another shape",
        )
    };
    match kind.as_str() {
        "bool" => raw.as_bool().map(SdoValue::Bool).ok_or_else(observed),
        "unsigned" => signed_or_unsigned(raw)
            .and_then(|value| u64::try_from(value).ok())
            .map(SdoValue::Unsigned)
            .ok_or_else(observed),
        "signed" => signed_or_unsigned(raw)
            .and_then(|value| i64::try_from(value).ok())
            .map(SdoValue::Signed)
            .ok_or_else(observed),
        "string" => raw
            .as_str()
            .map(|text| SdoValue::Text(text.to_owned()))
            .ok_or_else(observed),
        "bytes" => {
            let Some(items) = raw.as_array() else {
                return Err(observed());
            };
            let mut bytes = Vec::with_capacity(items.len());
            for item in items {
                let number = signed_or_unsigned(item).ok_or_else(observed)?;
                bytes.push(u8::try_from(number).map_err(|_| observed())?);
            }
            Ok(SdoValue::Bytes(bytes))
        }
        other => Err(mode_error(
            "config.parse",
            device,
            position,
            "a supported value type (`bool`, `unsigned`, `signed`, `string`, `bytes`)",
            &format!("type `{other}`"),
        )),
    }
}

fn parse_dc(value: &Json, device: &str, position: u16) -> Result<DcChoice, EngineeringDiagnostic> {
    let object = object_value(value, "config.parse", device, position)?;
    let kind = string(object, "mode", "config.parse", device, position)?;
    match kind.as_str() {
        "free-run" => {
            reject_unknown_keys(object, &["mode"], "config.parse", device, position)?;
            Ok(DcChoice::FreeRun)
        }
        "op-mode" => {
            reject_unknown_keys(
                object,
                &[
                    "mode",
                    "name",
                    "index",
                    "assign_activate",
                    "sync0_cycle",
                    "sync0_shift",
                    "sync1_cycle",
                    "sync1_shift",
                ],
                "config.parse",
                device,
                position,
            )?;
            Ok(DcChoice::OpMode(DcTiming {
                name: string(object, "name", "config.parse", device, position)?,
                index: usize::try_from(number(object, "index", "config.parse", device, position)?)
                    .map_err(|_| {
                        mode_error(
                            "config.parse",
                            device,
                            position,
                            "a declaration index in 0..=18446744073709551615",
                            "a declaration index out of range",
                        )
                    })?,
                assign_activate: u32_field(
                    object,
                    "assign_activate",
                    "config.parse",
                    device,
                    position,
                )?,
                sync0_cycle: u64_field(object, "sync0_cycle", "config.parse", device, position)?,
                sync0_shift: i32_field(object, "sync0_shift", "config.parse", device, position)?,
                sync1_cycle: optional_u64_field(
                    object,
                    "sync1_cycle",
                    "config.parse",
                    device,
                    position,
                )?,
                sync1_shift: optional_i32_field(
                    object,
                    "sync1_shift",
                    "config.parse",
                    device,
                    position,
                )?,
            }))
        }
        other => Err(dc_error(
            device,
            position,
            "DC mode `free-run` or `op-mode`",
            &format!("mode `{other}`"),
        )),
    }
}

fn signed_or_unsigned(value: &Json) -> Option<i128> {
    value.as_number()
}

fn object_value<'a>(
    value: &'a Json,
    operation: &'static str,
    device: &str,
    position: u16,
) -> Result<&'a BTreeMap<String, Json>, EngineeringDiagnostic> {
    value.as_object().ok_or_else(|| {
        mode_error(
            operation,
            device,
            position,
            "a JSON object",
            "a non-object value",
        )
    })
}

fn reject_unknown_keys(
    object: &BTreeMap<String, Json>,
    allowed: &[&str],
    operation: &'static str,
    device: &str,
    position: u16,
) -> Result<(), EngineeringDiagnostic> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(mode_error(
                operation,
                device,
                position,
                &format!("one of {allowed:?}"),
                &format!(
                    "unknown key `{key}`; vendor-specific or motion fields are not part of the \
                     generic mode schema"
                ),
            ));
        }
    }
    Ok(())
}

fn number(
    object: &BTreeMap<String, Json>,
    key: &str,
    operation: &'static str,
    device: &str,
    position: u16,
) -> Result<i128, EngineeringDiagnostic> {
    object.get(key).and_then(Json::as_number).ok_or_else(|| {
        mode_error(
            operation,
            device,
            position,
            &format!("a numeric `{key}`"),
            &format!("no numeric `{key}`"),
        )
    })
}

fn string(
    object: &BTreeMap<String, Json>,
    key: &str,
    operation: &'static str,
    device: &str,
    position: u16,
) -> Result<String, EngineeringDiagnostic> {
    object
        .get(key)
        .and_then(Json::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            mode_error(
                operation,
                device,
                position,
                &format!("a string `{key}`"),
                &format!("no string `{key}`"),
            )
        })
}

fn optional_string(
    object: &BTreeMap<String, Json>,
    key: &str,
    operation: &'static str,
    device: &str,
    position: u16,
) -> Result<Option<String>, EngineeringDiagnostic> {
    match object.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(|text| Some(text.to_owned()))
            .ok_or_else(|| {
                mode_error(
                    operation,
                    device,
                    position,
                    &format!("a string `{key}` or null"),
                    &format!("a non-string `{key}`"),
                )
            }),
    }
}

fn boolean(
    object: &BTreeMap<String, Json>,
    key: &str,
    operation: &'static str,
    device: &str,
    position: u16,
) -> Result<bool, EngineeringDiagnostic> {
    object.get(key).and_then(Json::as_bool).ok_or_else(|| {
        mode_error(
            operation,
            device,
            position,
            &format!("a boolean `{key}`"),
            &format!("no boolean `{key}`"),
        )
    })
}

fn boolean_or_default(
    object: &BTreeMap<String, Json>,
    key: &str,
    operation: &'static str,
    device: &str,
    position: u16,
) -> Result<bool, EngineeringDiagnostic> {
    match object.get(key) {
        None | Some(Json::Null) => Ok(false),
        Some(_) => boolean(object, key, operation, device, position),
    }
}

fn array<'a>(
    object: &'a BTreeMap<String, Json>,
    key: &str,
    operation: &'static str,
    device: &str,
    position: u16,
) -> Result<&'a [Json], EngineeringDiagnostic> {
    object.get(key).and_then(Json::as_array).ok_or_else(|| {
        mode_error(
            operation,
            device,
            position,
            &format!("an array `{key}`"),
            &format!("no array `{key}`"),
        )
    })
}

macro_rules! integer_field {
    ($name:ident, $ty:ty, $max:expr) => {
        fn $name(
            object: &BTreeMap<String, Json>,
            key: &str,
            operation: &'static str,
            device: &str,
            position: u16,
        ) -> Result<$ty, EngineeringDiagnostic> {
            let value = number(object, key, operation, device, position)?;
            <$ty>::try_from(value).map_err(|_| {
                mode_error(
                    operation,
                    device,
                    position,
                    &format!("`{key}` in 0..={}", $max),
                    &format!("`{key}` out of range"),
                )
            })
        }
    };
}

integer_field!(u8_field, u8, u8::MAX);
integer_field!(u16_field, u16, u16::MAX);
integer_field!(u32_field, u32, u32::MAX);
integer_field!(u64_field, u64, u64::MAX);
integer_field!(i32_field, i32, i32::MAX);

fn optional_u64_field(
    object: &BTreeMap<String, Json>,
    key: &str,
    operation: &'static str,
    device: &str,
    position: u16,
) -> Result<Option<u64>, EngineeringDiagnostic> {
    match object.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(_) => u64_field(object, key, operation, device, position).map(Some),
    }
}

fn optional_i32_field(
    object: &BTreeMap<String, Json>,
    key: &str,
    operation: &'static str,
    device: &str,
    position: u16,
) -> Result<Option<i32>, EngineeringDiagnostic> {
    match object.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(_) => i32_field(object, key, operation, device, position).map(Some),
    }
}

fn u16_list(
    object: &BTreeMap<String, Json>,
    key: &str,
    operation: &'static str,
    device: &str,
    position: u16,
) -> Result<Vec<u16>, EngineeringDiagnostic> {
    let mut values = Vec::new();
    for item in array(object, key, operation, device, position)? {
        let number = item.as_number().ok_or_else(|| {
            mode_error(
                operation,
                device,
                position,
                &format!("numeric `{key}` entries"),
                "a non-numeric entry",
            )
        })?;
        values.push(u16::try_from(number).map_err(|_| {
            mode_error(
                operation,
                device,
                position,
                &format!("`{key}` entries in 0..=65535"),
                "an entry out of range",
            )
        })?);
    }
    Ok(values)
}

fn u16_list_or_empty(
    object: &BTreeMap<String, Json>,
    key: &str,
    operation: &'static str,
    device: &str,
    position: u16,
) -> Result<Vec<u16>, EngineeringDiagnostic> {
    match object.get(key) {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(_) => u16_list(object, key, operation, device, position),
    }
}

fn u8_list(
    object: &BTreeMap<String, Json>,
    key: &str,
    operation: &'static str,
    device: &str,
    position: u16,
) -> Result<Vec<u8>, EngineeringDiagnostic> {
    let mut values = Vec::new();
    for item in array(object, key, operation, device, position)? {
        let number = item.as_number().ok_or_else(|| {
            mode_error(
                operation,
                device,
                position,
                &format!("numeric `{key}` entries"),
                "a non-numeric entry",
            )
        })?;
        values.push(u8::try_from(number).map_err(|_| {
            mode_error(
                operation,
                device,
                position,
                &format!("`{key}` entries in 0..=255"),
                "an entry out of range",
            )
        })?);
    }
    Ok(values)
}

fn u8_list_or_empty(
    object: &BTreeMap<String, Json>,
    key: &str,
    operation: &'static str,
    device: &str,
    position: u16,
) -> Result<Vec<u8>, EngineeringDiagnostic> {
    match object.get(key) {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(_) => u8_list(object, key, operation, device, position),
    }
}

fn mode_error(
    operation: &'static str,
    device: &str,
    position: u16,
    expected: &str,
    observed: &str,
) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::MODE_INVALID,
        operation,
        "author the generic mode bundle with a stored mode explicitly selected",
    )
    .with_device(device)
    .with_position(position)
    .with_expected(expected)
    .with_observed(observed)
}

fn dc_error(device: &str, position: u16, expected: &str, observed: &str) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::DC_SELECTION,
        "dc.validate",
        "select one ESI-declared DC op mode or an explicit free-run choice",
    )
    .with_device(device)
    .with_position(position)
    .with_expected(expected)
    .with_observed(observed)
}

fn channel_error(
    device: &str,
    position: u16,
    expected: &str,
    observed: &str,
) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::CHANNEL_EXPOSURE,
        "channel.validate",
        "expose only selected effective entries with explicit, non-overlapping channels",
    )
    .with_device(device)
    .with_position(position)
    .with_expected(expected)
    .with_observed(observed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ethercat::esi::model::SyncManager;
    use crate::ethercat::pdo::{DeclaredEntry, PdoAlternative, SelectedEntry};

    /// One malformed-document case: label, mutation and expected code.
    type ParseCase = (&'static str, fn(&mut Json), &'static str);

    fn alternative(
        direction: PdoDirection,
        index: u16,
        sm: u8,
        fixed: bool,
        mandatory: bool,
        entries: &[(u16, u8, u8)],
    ) -> PdoAlternative {
        PdoAlternative {
            direction,
            index,
            sm: Some(sm),
            fixed,
            mandatory,
            excluded_sm: Vec::new(),
            exclude: Vec::new(),
            name: Some(format!("pdo_{index:04x}")),
            entries: entries
                .iter()
                .map(|(entry_index, subindex, bit_len)| DeclaredEntry {
                    key: EntryKey::new(*entry_index, *subindex),
                    bit_len: *bit_len,
                    name: Some(format!("entry_{entry_index:04x}_{subindex}")),
                    data_type: Some("UINT".into()),
                    declared_in: index,
                    direction,
                })
                .collect(),
        }
    }

    fn capabilities() -> PdoCapabilities {
        PdoCapabilities {
            name: "Servo".into(),
            type_name: "SERVO".into(),
            vendor_id: 6,
            product_code: 0x101,
            revision: 1,
            digest: "digest".into(),
            sync_managers: vec![
                SyncManager {
                    index: 2,
                    start_address: 0x1100,
                    control_byte: 0x64,
                    enable: true,
                    name: Some("Outputs".into()),
                },
                SyncManager {
                    index: 3,
                    start_address: 0x1300,
                    control_byte: 0x20,
                    enable: true,
                    name: Some("Inputs".into()),
                },
            ],
            rx_pdos: vec![
                alternative(
                    PdoDirection::Rx,
                    0x1600,
                    2,
                    false,
                    false,
                    &[(0x7000, 1, 16), (0x7000, 2, 16)],
                ),
                alternative(PdoDirection::Rx, 0x1601, 2, false, false, &[(0x7000, 3, 8)]),
                alternative(PdoDirection::Rx, 0x1602, 2, true, true, &[(0x7000, 4, 8)]),
            ],
            tx_pdos: vec![
                alternative(
                    PdoDirection::Tx,
                    0x1a00,
                    3,
                    true,
                    true,
                    &[(0x6000, 1, 16), (0x6000, 2, 16)],
                ),
                alternative(PdoDirection::Tx, 0x1a01, 3, false, false, &[(0x6000, 3, 1)]),
            ],
            mailbox: MailboxCapabilities {
                sdo_info: true,
                pdo_assign: true,
                pdo_config: true,
                complete_access: false,
            },
            dc_op_modes: Some(vec![
                DcOpMode {
                    index: 0,
                    name: "DC Sync0".into(),
                    description: None,
                    assign_activate: 0x0300,
                    sync0_cycle: 0,
                    sync0_factor: Some(2),
                    sync0_shift: 0,
                    sync1_cycle: None,
                    sync1_shift: None,
                },
                DcOpMode {
                    index: 1,
                    name: "DC Sync0 Sync1".into(),
                    description: Some("with Sync1".into()),
                    assign_activate: 0x0300,
                    sync0_cycle: 500_000,
                    sync0_factor: None,
                    sync0_shift: 0,
                    sync1_cycle: Some(2_000_000),
                    sync1_shift: Some(0),
                },
            ]),
        }
    }

    fn timing() -> DcTiming {
        let capabilities = capabilities();
        DcTiming::resolve(&capabilities.dc_op_modes.as_ref().expect("dc")[0], 500_000)
            .expect("timing")
    }

    fn startup_operation(id: &str, index: u16, value: SdoValue) -> StartupOperation {
        StartupOperation {
            id: id.into(),
            action: StartupAction::Write,
            index,
            subindex: 0,
            value,
            state: StartupState::PreOp,
            verify: StartupVerification::Required,
            timeout_ms: 1_000,
        }
    }

    fn mode() -> Mode {
        let capabilities = capabilities();
        let pdos = capabilities.default_selection();
        let channels = Mode::propose_channels(&pdos, Some(&capabilities));
        Mode {
            name: "default".into(),
            description: Some("default mode".into()),
            provenance: Some("ESI op mode 0".into()),
            pdos,
            channels,
            startup: vec![
                startup_operation("write-bool", 0x6061, SdoValue::Bool(true)),
                startup_operation("write-unsigned", 0x6062, SdoValue::Unsigned(7)),
                startup_operation("write-signed", 0x6063, SdoValue::Signed(-7)),
                startup_operation("write-string", 0x6064, SdoValue::Text("text".into())),
                StartupOperation {
                    id: "verify-bytes".into(),
                    action: StartupAction::Verify,
                    index: 0x6065,
                    subindex: 0,
                    value: SdoValue::Bytes(vec![1, 2, 255]),
                    state: StartupState::SafeOp,
                    verify: StartupVerification::Required,
                    timeout_ms: 5_000,
                },
                StartupOperation {
                    id: "verify-best-effort".into(),
                    action: StartupAction::Verify,
                    index: 0x6066,
                    subindex: 1,
                    value: SdoValue::Unsigned(1),
                    state: StartupState::PreOp,
                    verify: StartupVerification::BestEffort,
                    timeout_ms: 6_000,
                },
            ],
            dc: DcChoice::OpMode(timing()),
        }
    }

    fn device_set() -> DeviceModeSet {
        let capabilities = capabilities();
        DeviceModeSet {
            name: "servo".into(),
            position: 0,
            identity: DeviceIdentity::new(6, 0x101, 1),
            esi: Some(EsiReference {
                source: "esi/servo.xml".into(),
                digest: "digest".into(),
            }),
            mailbox: MailboxFacts::from_mailbox(&capabilities.mailbox),
            selected: "default".into(),
            modes: vec![mode()],
        }
    }

    fn configuration() -> EngineeringConfiguration {
        EngineeringConfiguration {
            master: 0,
            devices: vec![device_set()],
        }
    }

    fn alternate_mode() -> Mode {
        let capabilities = capabilities();
        let mut pdos = capabilities.default_selection();
        pdos.pdos
            .retain(|pdo| pdo.index != 0x1601 && pdo.index != 0x1a01);
        Mode {
            name: "alternate".into(),
            description: None,
            provenance: None,
            pdos,
            channels: Vec::new(),
            startup: Vec::new(),
            dc: DcChoice::FreeRun,
        }
    }

    #[test]
    fn configuration_projections_are_structured_and_deterministic() {
        let configuration = configuration();
        let text = configuration.to_text();
        assert_eq!(text, configuration.to_json().render());
        for expected in [
            "\"protocol\": \"stc-ethercat-config/1\"",
            "\"format\": 1",
            "\"selected\": \"default\"",
            "\"mailbox\"",
            "\"pdo_assign\": true",
            "\"dc\"",
            "\"mode\": \"op-mode\"",
            "\"channels\"",
            "\"startup\"",
        ] {
            assert!(text.contains(expected), "missing {expected}");
        }
        let reference = EsiReference {
            source: "esi/servo.xml".into(),
            digest: "digest".into(),
        };
        assert!(
            reference
                .to_json()
                .render()
                .contains("\"source\": \"esi/servo.xml\"")
        );
        let facts = MailboxFacts {
            pdo_assign: true,
            pdo_config: false,
        };
        assert!(facts.to_json().render().contains("\"pdo_config\": false"));
        assert_eq!(
            MailboxFacts::from_mailbox(&capabilities().mailbox),
            MailboxFacts {
                pdo_assign: true,
                pdo_config: true,
            }
        );
        assert!(
            DcChoice::FreeRun
                .to_json()
                .render()
                .contains("\"mode\": \"free-run\"")
        );
        assert!(
            DcChoice::OpMode(timing())
                .to_json()
                .render()
                .contains("\"assign_activate\": 768")
        );
        let channel = &configuration.devices[0].modes[0].channels[0];
        let channel_json = channel.to_json().render();
        assert!(channel_json.contains("\"name\": \"entry_7000_1\""));
        assert!(channel_json.contains("\"direction\": \"rx\""));
        assert!(channel_json.contains("\"index_hex\": \"0x7000\""));
    }

    #[test]
    fn dc_timing_resolution_covers_factors_and_failures() {
        let capabilities = capabilities();
        let op_mode = &capabilities.dc_op_modes.as_ref().expect("dc")[0];
        assert_eq!(
            DcTiming::resolve(op_mode, 500_000)
                .expect("factor")
                .sync0_cycle,
            1_000_000
        );
        let mut fixed = op_mode.clone();
        fixed.sync0_factor = None;
        fixed.sync0_cycle = 750_000;
        assert_eq!(
            DcTiming::resolve(&fixed, 0).expect("declared").sync0_cycle,
            750_000
        );
        let mut zero_factor = op_mode.clone();
        zero_factor.sync0_factor = Some(0);
        assert!(
            DcTiming::resolve(&zero_factor, 500_000)
                .expect_err("zero factor")
                .contains("factor is zero")
        );
        assert!(
            DcTiming::resolve(op_mode, 0)
                .expect_err("zero runtime")
                .contains("runtime cycle is zero")
        );
        let mut overflow = op_mode.clone();
        overflow.sync0_factor = Some(u32::MAX);
        assert!(
            DcTiming::resolve(&overflow, u64::MAX)
                .expect_err("overflow")
                .contains("overflows")
        );
        let mut zero_declared = op_mode.clone();
        zero_declared.sync0_factor = None;
        zero_declared.sync0_cycle = 0;
        assert!(
            DcTiming::resolve(&zero_declared, 0)
                .expect_err("zero declared")
                .contains("declared cycle is zero")
        );
        let sync1 = &capabilities.dc_op_modes.as_ref().expect("dc")[1];
        let resolved = DcTiming::resolve(sync1, 500_000).expect("sync1");
        assert_eq!(resolved.sync1_cycle, Some(2_000_000));
        assert_eq!(resolved.sync1_shift, Some(0));
    }

    #[test]
    fn dc_selection_matches_declared_op_modes_and_effective_values() {
        let capabilities = capabilities();
        let op_modes = capabilities.dc_op_modes.as_deref();
        let timing = timing();
        DcChoice::OpMode(timing.clone())
            .validate(op_modes, 500_000, 0, "servo")
            .expect("valid op mode");
        DcChoice::FreeRun
            .validate(op_modes, 500_000, 0, "servo")
            .expect("free-run");
        DcChoice::FreeRun
            .validate(None, 0, 0, "servo")
            .expect("free-run without dc");
        assert_eq!(
            DcChoice::OpMode(timing.clone())
                .validate(None, 500_000, 0, "servo")
                .expect_err("no dc")
                .code,
            codes::DC_SELECTION
        );

        let mut wrong_name = timing.clone();
        wrong_name.name = "missing".into();
        assert_eq!(
            DcChoice::OpMode(wrong_name)
                .validate(op_modes, 500_000, 0, "servo")
                .expect_err("name")
                .code,
            codes::DC_SELECTION
        );

        let mut wrong_index = timing.clone();
        wrong_index.index = 99;
        assert!(
            DcChoice::OpMode(wrong_index)
                .validate(op_modes, 500_000, 0, "servo")
                .expect_err("index")
                .observed
                .expect("observed")
                .contains("no declared op mode matches")
        );

        let mut wrong_assign = timing.clone();
        wrong_assign.assign_activate = 0;
        assert!(
            DcChoice::OpMode(wrong_assign)
                .validate(op_modes, 500_000, 0, "servo")
                .expect_err("assign")
                .expected
                .expect("expected")
                .contains("AssignActivate")
        );

        let mut wrong_cycle = timing.clone();
        wrong_cycle.sync0_cycle = 1;
        assert!(
            DcChoice::OpMode(wrong_cycle)
                .validate(op_modes, 500_000, 0, "servo")
                .expect_err("cycle")
                .expected
                .expect("expected")
                .contains("resolved Sync0 cycle")
        );

        let mut wrong_shift = timing.clone();
        wrong_shift.sync0_shift = 2_000_000;
        assert!(
            DcChoice::OpMode(wrong_shift)
                .validate(op_modes, 500_000, 0, "servo")
                .expect_err("shift")
                .expected
                .expect("expected")
                .contains("Sync0 shift")
        );

        let mut extra_sync1 = timing.clone();
        extra_sync1.sync1_cycle = Some(2_000_000);
        extra_sync1.sync1_shift = Some(0);
        assert!(
            DcChoice::OpMode(extra_sync1)
                .validate(op_modes, 500_000, 0, "servo")
                .expect_err("sync1 presence")
                .observed
                .expect("observed")
                .contains("different Sync1 cycle presence")
        );

        let mut shift_without_sync1 = timing.clone();
        shift_without_sync1.sync1_shift = Some(0);
        assert!(
            DcChoice::OpMode(shift_without_sync1)
                .validate(op_modes, 500_000, 0, "servo")
                .expect_err("sync1 shift presence")
                .expected
                .expect("expected")
                .contains("no Sync1 shift")
        );

        let sync1_mode = &capabilities.dc_op_modes.as_ref().expect("dc")[1];
        let sync1 = DcTiming::resolve(sync1_mode, 500_000).expect("sync1");
        DcChoice::OpMode(sync1.clone())
            .validate(op_modes, 500_000, 0, "servo")
            .expect("valid sync1");

        let mut missing_cycle = sync1.clone();
        missing_cycle.sync1_cycle = None;
        missing_cycle.sync1_shift = None;
        assert!(
            DcChoice::OpMode(missing_cycle)
                .validate(op_modes, 500_000, 0, "servo")
                .expect_err("missing cycle")
                .observed
                .expect("observed")
                .contains("different Sync1 cycle presence")
        );

        let mut missing_shift = sync1.clone();
        missing_shift.sync1_shift = None;
        assert!(
            DcChoice::OpMode(missing_shift)
                .validate(op_modes, 500_000, 0, "servo")
                .expect_err("missing shift")
                .observed
                .expect("observed")
                .contains("different Sync1 shift presence")
        );

        let mut wrong_sync1_cycle = sync1.clone();
        wrong_sync1_cycle.sync1_cycle = Some(3_000_000);
        assert!(
            DcChoice::OpMode(wrong_sync1_cycle)
                .validate(op_modes, 500_000, 0, "servo")
                .expect_err("sync1 cycle")
                .expected
                .expect("expected")
                .contains("resolved Sync1 cycle")
        );

        let mut wrong_sync1_shift = sync1.clone();
        wrong_sync1_shift.sync1_shift = Some(2_000_000);
        assert!(
            DcChoice::OpMode(wrong_sync1_shift)
                .validate(op_modes, 500_000, 0, "servo")
                .expect_err("sync1 shift")
                .expected
                .expect("expected")
                .contains("Sync1 shift")
        );

        let mut not_multiple_capabilities = capabilities.clone();
        not_multiple_capabilities.dc_op_modes.as_mut().expect("dc")[1].sync1_cycle =
            Some(1_250_000);
        let mut not_multiple = sync1.clone();
        not_multiple.sync1_cycle = Some(1_250_000);
        assert!(
            DcChoice::OpMode(not_multiple)
                .validate(
                    not_multiple_capabilities.dc_op_modes.as_deref(),
                    500_000,
                    0,
                    "servo"
                )
                .expect_err("not multiple")
                .expected
                .expect("expected")
                .contains("multiple of Sync0")
        );

        let mut zero_factor_capabilities = capabilities.clone();
        zero_factor_capabilities.dc_op_modes.as_mut().expect("dc")[0].sync0_factor = Some(0);
        assert!(
            DcChoice::OpMode(timing.clone())
                .validate(
                    zero_factor_capabilities.dc_op_modes.as_deref(),
                    500_000,
                    0,
                    "servo"
                )
                .expect_err("zero factor")
                .observed
                .expect("observed")
                .contains("factor is zero")
        );

        assert!(
            DcChoice::OpMode(timing.clone())
                .validate(op_modes, 0, 0, "servo")
                .expect_err("zero runtime")
                .observed
                .expect("observed")
                .contains("runtime cycle is zero")
        );

        let mut zero_sync1_capabilities = capabilities.clone();
        zero_sync1_capabilities.dc_op_modes.as_mut().expect("dc")[1].sync1_cycle = Some(0);
        assert!(
            DcChoice::OpMode(sync1.clone())
                .validate(
                    zero_sync1_capabilities.dc_op_modes.as_deref(),
                    500_000,
                    0,
                    "servo"
                )
                .expect_err("zero sync1")
                .observed
                .expect("observed")
                .contains("declared cycle is zero")
        );

        let mut duplicate_capabilities = capabilities.clone();
        duplicate_capabilities.dc_op_modes.as_mut().expect("dc")[1].name = "DC Sync0".into();
        let mut ambiguous = timing.clone();
        ambiguous.index = 99;
        assert!(
            DcChoice::OpMode(ambiguous)
                .validate(
                    duplicate_capabilities.dc_op_modes.as_deref(),
                    500_000,
                    0,
                    "servo"
                )
                .expect_err("ambiguous")
                .observed
                .expect("observed")
                .contains("duplicated")
        );
        DcChoice::OpMode(timing.clone())
            .validate(
                duplicate_capabilities.dc_op_modes.as_deref(),
                500_000,
                0,
                "servo",
            )
            .expect("the declaration index disambiguates a duplicate name");
    }

    #[test]
    fn dc_fact_validation_is_structural_without_the_esi() {
        DcChoice::FreeRun
            .validate_facts(0, "servo")
            .expect("free-run");
        DcChoice::OpMode(timing())
            .validate_facts(0, "servo")
            .expect("valid");
        let mut empty = timing();
        empty.name.clear();
        assert_eq!(
            DcChoice::OpMode(empty)
                .validate_facts(0, "servo")
                .expect_err("empty name")
                .code,
            codes::DC_SELECTION
        );
        let mut zero = timing();
        zero.sync0_cycle = 0;
        assert!(
            DcChoice::OpMode(zero)
                .validate_facts(0, "servo")
                .expect_err("zero cycle")
                .expected
                .expect("expected")
                .contains("non-zero resolved Sync0")
        );
        let mut shift = timing();
        shift.sync0_shift = 1_000_000;
        assert!(
            DcChoice::OpMode(shift)
                .validate_facts(0, "servo")
                .expect_err("shift")
                .expected
                .expect("expected")
                .contains("Sync0 shift")
        );
        let mut shift_only = timing();
        shift_only.sync1_shift = Some(0);
        assert!(
            DcChoice::OpMode(shift_only)
                .validate_facts(0, "servo")
                .expect_err("shift without cycle")
                .expected
                .expect("expected")
                .contains("no Sync1 shift")
        );
        let mut sync1 = timing();
        sync1.sync1_cycle = Some(2_000_000);
        DcChoice::OpMode(sync1)
            .validate_facts(0, "servo")
            .expect("sync1 without shift");
        let mut not_multiple = timing();
        not_multiple.sync1_cycle = Some(1_500_000);
        assert!(
            DcChoice::OpMode(not_multiple)
                .validate_facts(0, "servo")
                .expect_err("not multiple")
                .expected
                .expect("expected")
                .contains("multiple of Sync0")
        );
        let mut below = timing();
        below.sync1_cycle = Some(500_000);
        assert!(
            DcChoice::OpMode(below)
                .validate_facts(0, "servo")
                .expect_err("below")
                .expected
                .expect("expected")
                .contains("multiple of Sync0")
        );
        let mut sync1_shift = timing();
        sync1_shift.sync1_cycle = Some(2_000_000);
        sync1_shift.sync1_shift = Some(2_000_000);
        assert!(
            DcChoice::OpMode(sync1_shift)
                .validate_facts(0, "servo")
                .expect_err("sync1 shift")
                .expected
                .expect("expected")
                .contains("Sync1 shift")
        );
    }

    #[test]
    fn channel_validation_accepts_explicit_slices_and_rejects_invalid_exposure() {
        let capabilities = capabilities();
        let selection = capabilities.default_selection();
        let channels = Mode::propose_channels(&selection, Some(&capabilities));
        assert_eq!(channels.len(), 7);
        validate_channels(&channels, &selection, 0, "servo").expect("valid");

        let mut empty = channels.clone();
        empty[0].name = "  ".into();
        assert_eq!(
            validate_channels(&empty, &selection, 0, "servo")
                .expect_err("empty name")
                .code,
            codes::CHANNEL_EXPOSURE
        );
        let mut duplicate = channels.clone();
        duplicate[1].name = duplicate[0].name.clone();
        assert!(
            validate_channels(&duplicate, &selection, 0, "servo")
                .expect_err("duplicate")
                .observed
                .expect("observed")
                .contains("duplicate channel name")
        );
        let mut unselected_pdo = channels.clone();
        unselected_pdo[0].pdo = 0x1700;
        assert!(
            validate_channels(&unselected_pdo, &selection, 0, "servo")
                .expect_err("pdo")
                .observed
                .expect("observed")
                .contains("unselected PDO")
        );
        let mut unselected_entry = channels.clone();
        unselected_entry[0].entry = EntryKey::new(0x7000, 9);
        assert!(
            validate_channels(&unselected_entry, &selection, 0, "servo")
                .expect_err("entry")
                .observed
                .expect("observed")
                .contains("does not expose")
        );
        let mut zero_width = channels.clone();
        zero_width[0].width = 0;
        assert!(
            validate_channels(&zero_width, &selection, 0, "servo")
                .expect_err("width")
                .expected
                .expect("expected")
                .contains("non-zero channel width")
        );
        let mut overrun = channels.clone();
        overrun[0].width = 17;
        assert!(
            validate_channels(&overrun, &selection, 0, "servo")
                .expect_err("overrun")
                .expected
                .expect("expected")
                .contains("to fit entry")
        );
        let mut misaligned = channels.clone();
        misaligned[0].bit = 1;
        misaligned[0].width = 8;
        assert!(
            validate_channels(&misaligned, &selection, 0, "servo")
                .expect_err("misaligned")
                .expected
                .expect("expected")
                .contains("byte boundary")
        );
        let mut overlap = channels.clone();
        overlap[1].pdo = overlap[0].pdo;
        overlap[1].entry = overlap[0].entry;
        overlap[1].direction = overlap[0].direction;
        overlap[1].bit = 0;
        overlap[1].width = 8;
        assert!(
            validate_channels(&overlap, &selection, 0, "servo")
                .expect_err("overlap")
                .expected
                .expect("expected")
                .contains("non-overlapping")
        );
        let mut direction = channels.clone();
        direction[0].direction = PdoDirection::Tx;
        assert!(
            validate_channels(&direction, &selection, 0, "servo")
                .expect_err("direction")
                .observed
                .expect("observed")
                .contains("unselected PDO")
        );

        let mut cross = selection.clone();
        cross.pdos.push(SelectedPdo {
            direction: PdoDirection::Tx,
            index: 0x1600,
            sm: 3,
            fixed: false,
            mandatory: false,
            exclude: Vec::new(),
            excluded_sm: Vec::new(),
            entries: vec![SelectedEntry {
                index: 0x7000,
                subindex: 1,
                bit_len: 16,
                declared_in: 0x1600,
            }],
        });
        let cross_channels = vec![
            ChannelExposure {
                name: "rx".into(),
                pdo: 0x1600,
                entry: EntryKey::new(0x7000, 1),
                direction: PdoDirection::Rx,
                bit: 0,
                width: 16,
            },
            ChannelExposure {
                name: "tx".into(),
                pdo: 0x1600,
                entry: EntryKey::new(0x7000, 1),
                direction: PdoDirection::Tx,
                bit: 0,
                width: 16,
            },
        ];
        validate_channels(&cross_channels, &cross, 0, "servo")
            .expect("the same index in another direction is a distinct PDO");
    }

    #[test]
    fn startup_validation_rejects_pdo_ownership_and_invalid_policies() {
        validate_startup(&mode().startup, 0, "servo").expect("valid");
        for index in [0x1600, 0x1a00, 0x1c12] {
            let mut startup = mode().startup;
            startup[0].index = index;
            assert_eq!(
                validate_startup(&startup, 0, "servo")
                    .expect_err("pdo object")
                    .code,
                codes::MODE_INVALID
            );
        }
        assert!(is_pdo_configuration_object(0x1600));
        assert!(is_pdo_configuration_object(0x17ff));
        assert!(is_pdo_configuration_object(0x1a00));
        assert!(is_pdo_configuration_object(0x1bff));
        assert!(is_pdo_configuration_object(0x1c10));
        assert!(is_pdo_configuration_object(0x1c2f));
        assert!(!is_pdo_configuration_object(0x15ff));
        assert!(!is_pdo_configuration_object(0x1800));
        assert!(!is_pdo_configuration_object(0x1c00));
        assert!(!is_pdo_configuration_object(0x1c0f));
        assert!(!is_pdo_configuration_object(0x1c30));

        let mut empty_id = mode().startup;
        empty_id[0].id.clear();
        assert_eq!(
            validate_startup(&empty_id, 0, "servo")
                .expect_err("empty id")
                .code,
            codes::MODE_INVALID
        );
        let mut duplicate_id = mode().startup;
        duplicate_id[1].id = duplicate_id[0].id.clone();
        assert_eq!(
            validate_startup(&duplicate_id, 0, "servo")
                .expect_err("duplicate id")
                .code,
            codes::MODE_INVALID
        );
        let mut zero_timeout = mode().startup;
        zero_timeout[0].timeout_ms = 0;
        assert_eq!(
            validate_startup(&zero_timeout, 0, "servo")
                .expect_err("zero timeout")
                .code,
            codes::MODE_INVALID
        );
        let mut huge_timeout = mode().startup;
        huge_timeout[0].timeout_ms = 600_001;
        assert_eq!(
            validate_startup(&huge_timeout, 0, "servo")
                .expect_err("huge timeout")
                .code,
            codes::MODE_INVALID
        );
        let mut verify_none = mode().startup;
        verify_none[0].action = StartupAction::Verify;
        verify_none[0].verify = StartupVerification::None;
        assert_eq!(
            validate_startup(&verify_none, 0, "servo")
                .expect_err("verify none")
                .code,
            codes::MODE_INVALID
        );
        let mut verify_best_effort = mode().startup;
        verify_best_effort[0].action = StartupAction::Verify;
        verify_best_effort[0].verify = StartupVerification::BestEffort;
        validate_startup(&verify_best_effort, 0, "servo").expect("best-effort verify");
        let mut sync_manager_parameter = mode().startup;
        sync_manager_parameter[0].index = 0x1c32;
        validate_startup(&sync_manager_parameter, 0, "servo")
            .expect("sync-manager parameters are not PDO configuration");
    }

    #[test]
    fn mode_validation_and_channel_proposals_cover_declared_and_offline_facts() {
        let capabilities = capabilities();
        mode()
            .validate(Some(&capabilities), 500_000, 0, "servo")
            .expect("valid");
        mode().validate(None, 0, 0, "servo").expect("offline valid");

        let mut unnamed = mode();
        unnamed.name.clear();
        assert_eq!(
            unnamed
                .validate(Some(&capabilities), 500_000, 0, "servo")
                .expect_err("name")
                .code,
            codes::MODE_INVALID
        );
        let mut invalid_pdo = mode();
        invalid_pdo.pdos.pdos.retain(|pdo| pdo.index != 0x1602);
        let error = invalid_pdo
            .validate(Some(&capabilities), 500_000, 0, "servo")
            .expect_err("pdo");
        assert_eq!(error.code, codes::PDO_SELECTION);
        assert_eq!(error.device.as_deref(), Some("servo"));
        let mut invalid_facts = mode();
        invalid_facts.pdos.pdos[0].entries[0].bit_len = 0;
        assert_eq!(
            invalid_facts
                .validate(None, 0, 0, "servo")
                .expect_err("facts")
                .code,
            codes::PDO_SELECTION
        );
        let mut invalid_dc = mode();
        invalid_dc.dc = DcChoice::OpMode(DcTiming {
            name: "missing".into(),
            index: 99,
            assign_activate: 0,
            sync0_cycle: 1,
            sync0_shift: 0,
            sync1_cycle: None,
            sync1_shift: None,
        });
        assert_eq!(
            invalid_dc
                .validate(Some(&capabilities), 500_000, 0, "servo")
                .expect_err("dc")
                .code,
            codes::DC_SELECTION
        );
        let mut invalid_channel = mode();
        invalid_channel.channels[0].width = 0;
        assert_eq!(
            invalid_channel
                .validate(Some(&capabilities), 500_000, 0, "servo")
                .expect_err("channel")
                .code,
            codes::CHANNEL_EXPOSURE
        );
        let mut invalid_startup = mode();
        invalid_startup.startup[0].index = 0x1600;
        assert_eq!(
            invalid_startup
                .validate(Some(&capabilities), 500_000, 0, "servo")
                .expect_err("startup")
                .code,
            codes::MODE_INVALID
        );

        let proposed =
            Mode::propose_channels(&capabilities.default_selection(), Some(&capabilities));
        assert_eq!(proposed.len(), 7);
        assert_eq!(proposed[0].name, "entry_7000_1");
        let offline = Mode::propose_channels(&capabilities.default_selection(), None);
        assert_eq!(offline[0].name, "pdo1600_7000_1");

        let mut duplicate_capabilities = capabilities.clone();
        duplicate_capabilities.rx_pdos[1]
            .entries
            .push(DeclaredEntry {
                key: EntryKey::new(0x7000, 1),
                bit_len: 16,
                name: Some("entry_7000_1".into()),
                data_type: None,
                declared_in: 0x1601,
                direction: PdoDirection::Rx,
            });
        let proposed = Mode::propose_channels(
            &duplicate_capabilities.default_selection(),
            Some(&duplicate_capabilities),
        );
        let names: Vec<&str> = proposed
            .iter()
            .map(|channel| channel.name.as_str())
            .collect();
        assert!(names.contains(&"entry_7000_1"));
        assert!(names.contains(&"entry_7000_1_1"));
        assert_eq!(names.len(), names.iter().collect::<BTreeSet<_>>().len());
    }

    #[test]
    fn device_selection_and_alternate_inertness() {
        let capabilities = capabilities();
        let mut device = device_set();
        device.modes.push(alternate_mode());
        let bound = ["entry_7000_1".to_string()];
        device
            .validate(Some(&capabilities), &bound, 500_000)
            .expect("the selected mode exposes the bound channel");
        assert_eq!(device.selected_mode().expect("selected").name, "default");
        assert!(
            device
                .modes
                .iter()
                .find(|mode| mode.name == "alternate")
                .expect("alternate")
                .channels
                .is_empty()
        );
        let configured = device.configured_device();
        assert_eq!(configured.name, "servo");
        assert_eq!(configured.position, 0);
        assert_eq!(configured.identity, DeviceIdentity::new(6, 0x101, 1));
        device
            .validate(None, &bound, 0)
            .expect("offline validation uses persisted facts");

        let mut switched = device.clone();
        switched.selected = "alternate".into();
        let error = switched
            .validate(Some(&capabilities), &bound, 500_000)
            .expect_err("channel loss");
        assert_eq!(error.code, codes::CHANNEL_EXPOSURE);
        assert!(error.expected.expect("expected").contains("alternate"));

        let mut empty = device_set();
        empty.selected.clear();
        assert_eq!(
            empty
                .validate(Some(&capabilities), &[], 500_000)
                .expect_err("empty selection")
                .code,
            codes::MODE_INVALID
        );
        let mut missing = device_set();
        missing.selected = "missing".into();
        assert_eq!(
            missing
                .validate(Some(&capabilities), &[], 500_000)
                .expect_err("missing selection")
                .code,
            codes::MODE_INVALID
        );
        let mut unnamed = device_set();
        unnamed.name.clear();
        assert_eq!(
            unnamed
                .validate(Some(&capabilities), &[], 500_000)
                .expect_err("device name")
                .code,
            codes::MODE_INVALID
        );
        let mut no_modes = device_set();
        no_modes.modes.clear();
        assert_eq!(
            no_modes
                .validate(Some(&capabilities), &[], 500_000)
                .expect_err("no modes")
                .code,
            codes::MODE_INVALID
        );
        let mut duplicate = device_set();
        duplicate.modes.push(duplicate.modes[0].clone());
        assert_eq!(
            duplicate
                .validate(Some(&capabilities), &[], 500_000)
                .expect_err("duplicate mode")
                .code,
            codes::MODE_INVALID
        );
        let mut no_esi = device_set();
        no_esi.esi = None;
        assert_eq!(
            no_esi
                .validate(Some(&capabilities), &[], 500_000)
                .expect_err("no esi")
                .code,
            codes::MODE_INVALID
        );
        let mut incomplete = device_set();
        incomplete.esi.as_mut().expect("esi").digest.clear();
        assert_eq!(
            incomplete
                .validate(Some(&capabilities), &[], 500_000)
                .expect_err("incomplete esi")
                .code,
            codes::MODE_INVALID
        );
        let mut identity = device_set();
        identity.identity = DeviceIdentity::new(7, 0x101, 1);
        assert_eq!(
            identity
                .validate(Some(&capabilities), &[], 500_000)
                .expect_err("identity")
                .code,
            codes::MODE_INVALID
        );
        let mut digest = device_set();
        digest.esi.as_mut().expect("esi").digest = "other".into();
        assert_eq!(
            digest
                .validate(Some(&capabilities), &[], 500_000)
                .expect_err("digest")
                .code,
            codes::MODE_INVALID
        );
        let mut mailbox = device_set();
        mailbox.mailbox.pdo_config = false;
        assert_eq!(
            mailbox
                .validate(Some(&capabilities), &[], 500_000)
                .expect_err("mailbox")
                .code,
            codes::MODE_INVALID
        );
        let mut invalid_mode = device_set();
        invalid_mode.modes[0].name.clear();
        assert_eq!(
            invalid_mode
                .validate(Some(&capabilities), &[], 500_000)
                .expect_err("mode")
                .code,
            codes::MODE_INVALID
        );
        let error = device_set()
            .validate(Some(&capabilities), &["missing".into()], 500_000)
            .expect_err("bound channel");
        assert_eq!(error.code, codes::CHANNEL_EXPOSURE);
        assert!(error.expected.expect("expected").contains("missing"));
        assert!(
            device
                .to_json()
                .render()
                .contains("\"selected\": \"default\"")
        );
    }

    #[test]
    fn configuration_report_collects_topology_and_device_diagnostics() {
        let configuration = configuration();
        let capabilities = capabilities();
        let bound: BTreeMap<String, Vec<String>> =
            BTreeMap::from([("servo".into(), vec!["entry_7000_1".into()])]);
        let report = configuration.validate(
            |device| (device.name == "servo").then(|| capabilities.clone()),
            &bound,
            500_000,
        );
        assert!(report.is_valid());
        assert_eq!(report.devices.len(), 1);
        assert!(report.devices[0].esi_resolved);
        assert_eq!(report.devices[0].selected.as_deref(), Some("default"));
        assert!(report.devices[0].result.is_none());
        assert!(report.to_json().render().contains("\"valid\": true"));

        let offline = configuration.validate(|_| None, &bound, 0);
        assert!(offline.is_valid());
        assert!(!offline.devices[0].esi_resolved);

        let missing_bound: BTreeMap<String, Vec<String>> =
            BTreeMap::from([("servo".into(), vec!["missing".into()])]);
        let missing =
            configuration.validate(|_| Some(capabilities.clone()), &missing_bound, 500_000);
        assert!(!missing.is_valid());
        assert_eq!(missing.diagnostics[0].code, codes::CHANNEL_EXPOSURE);
        assert_eq!(
            missing.devices[0].result.as_ref().expect("result").code,
            codes::CHANNEL_EXPOSURE
        );
        assert!(missing.to_json().render().contains("\"diagnostic\""));

        let empty = EngineeringConfiguration::new(0);
        let report = empty.validate(|_| None, &BTreeMap::new(), 0);
        assert!(!report.is_valid());
        assert_eq!(report.diagnostics[0].code, codes::TOPOLOGY_POSITION);
        assert!(report.devices.is_empty());
        assert!(configuration.device("servo").is_some());
        assert!(configuration.device("missing").is_none());
        assert_eq!(configuration.topology().devices.len(), 1);

        let mut unnamed_selection = configuration.clone();
        unnamed_selection.devices[0].selected.clear();
        let report = unnamed_selection.validate(|_| None, &BTreeMap::new(), 0);
        assert!(report.devices[0].selected.is_none());
        assert!(
            report.devices[0]
                .to_json()
                .render()
                .contains("\"selected\": null")
        );
    }

    fn object_map(value: &mut Json) -> &mut BTreeMap<String, Json> {
        match value {
            Json::Object(map) => map,
            _ => panic!("expected a JSON object"),
        }
    }

    fn array_items(value: &mut Json) -> &mut Vec<Json> {
        match value {
            Json::Array(items) => items,
            _ => panic!("expected a JSON array"),
        }
    }

    fn top(document: &mut Json) -> &mut BTreeMap<String, Json> {
        object_map(document)
    }

    fn devices(document: &mut Json) -> &mut Vec<Json> {
        array_items(top(document).get_mut("devices").expect("devices"))
    }

    fn device(document: &mut Json) -> &mut BTreeMap<String, Json> {
        object_map(&mut devices(document)[0])
    }

    fn modes(document: &mut Json) -> &mut Vec<Json> {
        array_items(device(document).get_mut("modes").expect("modes"))
    }

    fn mode_object(document: &mut Json) -> &mut BTreeMap<String, Json> {
        object_map(&mut modes(document)[0])
    }

    fn pdos(document: &mut Json) -> &mut Vec<Json> {
        array_items(mode_object(document).get_mut("pdos").expect("pdos"))
    }

    fn pdo(document: &mut Json) -> &mut BTreeMap<String, Json> {
        object_map(&mut pdos(document)[0])
    }

    fn entries(document: &mut Json) -> &mut Vec<Json> {
        array_items(pdo(document).get_mut("entries").expect("entries"))
    }

    fn entry(document: &mut Json) -> &mut BTreeMap<String, Json> {
        object_map(&mut entries(document)[0])
    }

    fn channels(document: &mut Json) -> &mut Vec<Json> {
        array_items(mode_object(document).get_mut("channels").expect("channels"))
    }

    fn channel(document: &mut Json) -> &mut BTreeMap<String, Json> {
        object_map(&mut channels(document)[0])
    }

    fn channel_entry(document: &mut Json) -> &mut BTreeMap<String, Json> {
        object_map(channel(document).get_mut("entry").expect("entry"))
    }

    fn startup(document: &mut Json) -> &mut Vec<Json> {
        array_items(mode_object(document).get_mut("startup").expect("startup"))
    }

    fn operation(document: &mut Json) -> &mut BTreeMap<String, Json> {
        object_map(&mut startup(document)[0])
    }

    fn value(document: &mut Json) -> &mut BTreeMap<String, Json> {
        object_map(operation(document).get_mut("value").expect("value"))
    }

    fn dc(document: &mut Json) -> &mut BTreeMap<String, Json> {
        object_map(mode_object(document).get_mut("dc").expect("dc"))
    }

    fn esi(document: &mut Json) -> &mut BTreeMap<String, Json> {
        object_map(device(document).get_mut("esi").expect("esi"))
    }

    fn mailbox(document: &mut Json) -> &mut BTreeMap<String, Json> {
        object_map(device(document).get_mut("mailbox").expect("mailbox"))
    }

    fn parse_error(mutate: fn(&mut Json)) -> EngineeringDiagnostic {
        let mut document = json::parse(&configuration().to_text()).expect("fixture");
        mutate(&mut document);
        EngineeringConfiguration::parse(&document.render()).expect_err("must fail")
    }

    fn parse_document(mutate: fn(&mut Json)) -> EngineeringConfiguration {
        let mut document = json::parse(&configuration().to_text()).expect("fixture");
        mutate(&mut document);
        EngineeringConfiguration::parse(&document.render()).expect("must parse")
    }

    #[test]
    fn configuration_round_trips_and_ignores_derived_index_hex() {
        let configuration = configuration();
        let parsed = EngineeringConfiguration::parse(&configuration.to_text()).expect("round-trip");
        assert_eq!(parsed, configuration);
        let parsed = parse_document(|document| {
            pdo(document).insert("index_hex".into(), Json::string("0xdead"));
            entry(document).insert("index_hex".into(), Json::string("0xdead"));
            channel_entry(document).insert("index_hex".into(), Json::string("0xdead"));
            operation(document).insert("index_hex".into(), Json::string("0xdead"));
        });
        assert_eq!(parsed, configuration);
    }

    #[test]
    fn configuration_parse_rejects_malformed_documents_and_unknown_keys() {
        assert_eq!(
            EngineeringConfiguration::parse("not json")
                .expect_err("json")
                .code,
            codes::MODE_INVALID
        );
        assert_eq!(
            EngineeringConfiguration::parse("[]")
                .expect_err("root")
                .code,
            codes::MODE_INVALID
        );
        let cases: Vec<ParseCase> = vec![
            (
                "unknown top-level key",
                |document| {
                    top(document).insert("vendor".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "missing format",
                |document| {
                    top(document).remove("format");
                },
                codes::MODE_INVALID,
            ),
            (
                "wrong format",
                |document| {
                    top(document).insert("format".into(), Json::unsigned(2));
                },
                codes::MODE_INVALID,
            ),
            (
                "missing protocol",
                |document| {
                    top(document).remove("protocol");
                },
                codes::MODE_INVALID,
            ),
            (
                "wrong protocol",
                |document| {
                    top(document).insert("protocol".into(), Json::string("other"));
                },
                codes::MODE_INVALID,
            ),
            (
                "master out of range",
                |document| {
                    top(document).insert("master".into(), Json::unsigned(65_536));
                },
                codes::MODE_INVALID,
            ),
            (
                "missing devices",
                |document| {
                    top(document).remove("devices");
                },
                codes::MODE_INVALID,
            ),
            (
                "device not object",
                |document| {
                    devices(document)[0] = Json::Bool(true);
                },
                codes::MODE_INVALID,
            ),
            (
                "device missing name",
                |document| {
                    device(document).remove("name");
                },
                codes::MODE_INVALID,
            ),
            (
                "device position out of range",
                |document| {
                    device(document).insert("position".into(), Json::unsigned(65_536));
                },
                codes::MODE_INVALID,
            ),
            (
                "device unknown key",
                |document| {
                    device(document).insert("vendor".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "esi not object",
                |document| {
                    device(document).insert("esi".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "esi unknown key",
                |document| {
                    esi(document).insert("vendor".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "esi missing digest",
                |document| {
                    esi(document).remove("digest");
                },
                codes::MODE_INVALID,
            ),
            (
                "mailbox not object",
                |document| {
                    device(document).insert("mailbox".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "mailbox unknown key",
                |document| {
                    mailbox(document).insert("vendor".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "mailbox missing flag",
                |document| {
                    mailbox(document).remove("pdo_assign");
                },
                codes::MODE_INVALID,
            ),
            (
                "selected missing",
                |document| {
                    device(document).remove("selected");
                },
                codes::MODE_INVALID,
            ),
            (
                "modes not array",
                |document| {
                    device(document).insert("modes".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "mode not object",
                |document| {
                    modes(document)[0] = Json::Bool(true);
                },
                codes::MODE_INVALID,
            ),
            (
                "mode unknown key",
                |document| {
                    mode_object(document).insert("vendor".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "mode missing name",
                |document| {
                    mode_object(document).remove("name");
                },
                codes::MODE_INVALID,
            ),
            (
                "description not string",
                |document| {
                    mode_object(document).insert("description".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "pdos not array",
                |document| {
                    mode_object(document).insert("pdos".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "channels not array",
                |document| {
                    mode_object(document).insert("channels".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup not array",
                |document| {
                    mode_object(document).insert("startup".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "missing dc",
                |document| {
                    mode_object(document).remove("dc");
                },
                codes::DC_SELECTION,
            ),
            (
                "dc not object",
                |document| {
                    mode_object(document).insert("dc".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "dc unknown mode",
                |document| {
                    dc(document).insert("mode".into(), Json::string("vendor"));
                },
                codes::DC_SELECTION,
            ),
            (
                "free-run extra key",
                |document| {
                    dc(document).insert("mode".into(), Json::string("free-run"));
                    dc(document).insert("name".into(), Json::string("x"));
                },
                codes::MODE_INVALID,
            ),
            (
                "op-mode unknown key",
                |document| {
                    dc(document).insert("vendor".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "op-mode missing name",
                |document| {
                    dc(document).remove("name");
                },
                codes::MODE_INVALID,
            ),
            (
                "op-mode index out of range",
                |document| {
                    dc(document).insert("index".into(), Json::signed(-1));
                },
                codes::MODE_INVALID,
            ),
            (
                "op-mode assign_activate out of range",
                |document| {
                    dc(document).insert("assign_activate".into(), Json::signed(-1));
                },
                codes::MODE_INVALID,
            ),
            (
                "op-mode sync0_cycle out of range",
                |document| {
                    dc(document).insert("sync0_cycle".into(), Json::signed(-1));
                },
                codes::MODE_INVALID,
            ),
            (
                "op-mode sync0_shift out of range",
                |document| {
                    dc(document)
                        .insert("sync0_shift".into(), Json::Number(i128::from(i32::MAX) + 1));
                },
                codes::MODE_INVALID,
            ),
        ];
        for (label, mutate, code) in cases {
            let error = parse_error(mutate);
            assert_eq!(error.code, code, "{label}");
        }
    }

    #[test]
    fn configuration_parse_rejects_invalid_pdo_channel_startup_and_value_fields() {
        let cases: Vec<ParseCase> = vec![
            (
                "pdo not object",
                |document| {
                    pdos(document)[0] = Json::Bool(true);
                },
                codes::MODE_INVALID,
            ),
            (
                "pdo unknown key",
                |document| {
                    pdo(document).insert("vendor".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "pdo bad direction",
                |document| {
                    pdo(document).insert("direction".into(), Json::string("up"));
                },
                codes::MODE_INVALID,
            ),
            (
                "pdo index out of range",
                |document| {
                    pdo(document).insert("index".into(), Json::unsigned(65_536));
                },
                codes::MODE_INVALID,
            ),
            (
                "pdo sm out of range",
                |document| {
                    pdo(document).insert("sm".into(), Json::unsigned(256));
                },
                codes::MODE_INVALID,
            ),
            (
                "pdo entries not array",
                |document| {
                    pdo(document).insert("entries".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "pdo fixed not bool",
                |document| {
                    pdo(document).insert("fixed".into(), Json::unsigned(1));
                },
                codes::MODE_INVALID,
            ),
            (
                "pdo exclude not array",
                |document| {
                    pdo(document).insert("exclude".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "pdo exclude non-numeric",
                |document| {
                    pdo(document).insert("exclude".into(), Json::Array(vec![Json::string("x")]));
                },
                codes::MODE_INVALID,
            ),
            (
                "pdo exclude out of range",
                |document| {
                    pdo(document)
                        .insert("exclude".into(), Json::Array(vec![Json::unsigned(65_536)]));
                },
                codes::MODE_INVALID,
            ),
            (
                "pdo excluded_sm out of range",
                |document| {
                    pdo(document)
                        .insert("excluded_sm".into(), Json::Array(vec![Json::unsigned(256)]));
                },
                codes::MODE_INVALID,
            ),
            (
                "entry not object",
                |document| {
                    entries(document)[0] = Json::Bool(true);
                },
                codes::MODE_INVALID,
            ),
            (
                "entry unknown key",
                |document| {
                    entry(document).insert("vendor".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "entry missing index",
                |document| {
                    entry(document).remove("index");
                },
                codes::MODE_INVALID,
            ),
            (
                "entry bit_len out of range",
                |document| {
                    entry(document).insert("bit_len".into(), Json::unsigned(256));
                },
                codes::MODE_INVALID,
            ),
            (
                "entry declared_in out of range",
                |document| {
                    entry(document).insert("declared_in".into(), Json::unsigned(65_536));
                },
                codes::MODE_INVALID,
            ),
            (
                "channel not object",
                |document| {
                    channels(document)[0] = Json::Bool(true);
                },
                codes::MODE_INVALID,
            ),
            (
                "channel unknown key",
                |document| {
                    channel(document).insert("vendor".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "channel bad direction",
                |document| {
                    channel(document).insert("direction".into(), Json::string("up"));
                },
                codes::MODE_INVALID,
            ),
            (
                "channel missing entry",
                |document| {
                    channel(document).remove("entry");
                },
                codes::MODE_INVALID,
            ),
            (
                "channel entry not object",
                |document| {
                    channel(document).insert("entry".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "channel entry unknown key",
                |document| {
                    channel_entry(document).insert("vendor".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "channel bit out of range",
                |document| {
                    channel(document).insert("bit".into(), Json::signed(-1));
                },
                codes::MODE_INVALID,
            ),
            (
                "channel width out of range",
                |document| {
                    channel(document).insert("width".into(), Json::signed(-1));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup not object",
                |document| {
                    startup(document)[0] = Json::Bool(true);
                },
                codes::MODE_INVALID,
            ),
            (
                "startup unknown key",
                |document| {
                    operation(document).insert("vendor".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup bad action",
                |document| {
                    operation(document).insert("action".into(), Json::string("read"));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup bad state",
                |document| {
                    operation(document).insert("state".into(), Json::string("op"));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup bad verify",
                |document| {
                    operation(document).insert("verify".into(), Json::string("maybe"));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup missing value",
                |document| {
                    operation(document).remove("value");
                },
                codes::MODE_INVALID,
            ),
            (
                "startup value not object",
                |document| {
                    operation(document).insert("value".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup value unknown key",
                |document| {
                    value(document).insert("vendor".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup value unsupported type",
                |document| {
                    value(document).insert("type".into(), Json::string("float"));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup bool wrong shape",
                |document| {
                    value(document).insert("type".into(), Json::string("bool"));
                    value(document).insert("value".into(), Json::unsigned(1));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup unsigned negative",
                |document| {
                    value(document).insert("type".into(), Json::string("unsigned"));
                    value(document).insert("value".into(), Json::signed(-1));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup signed out of range",
                |document| {
                    value(document).insert("type".into(), Json::string("signed"));
                    value(document).insert("value".into(), Json::Number(i128::from(i64::MAX) + 1));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup string wrong shape",
                |document| {
                    value(document).insert("type".into(), Json::string("string"));
                    value(document).insert("value".into(), Json::unsigned(1));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup bytes wrong shape",
                |document| {
                    value(document).insert("type".into(), Json::string("bytes"));
                    value(document).insert("value".into(), Json::string("x"));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup bytes out of range",
                |document| {
                    value(document).insert("type".into(), Json::string("bytes"));
                    value(document).insert("value".into(), Json::Array(vec![Json::unsigned(256)]));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup timeout out of range",
                |document| {
                    operation(document).insert("timeout_ms".into(), Json::unsigned(4_294_967_296));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup index out of range",
                |document| {
                    operation(document).insert("index".into(), Json::unsigned(65_536));
                },
                codes::MODE_INVALID,
            ),
            (
                "startup subindex out of range",
                |document| {
                    operation(document).insert("subindex".into(), Json::unsigned(256));
                },
                codes::MODE_INVALID,
            ),
            (
                "dc mode not string",
                |document| {
                    dc(document).insert("mode".into(), Json::unsigned(1));
                },
                codes::MODE_INVALID,
            ),
            (
                "dc sync1_cycle out of range",
                |document| {
                    dc(document).insert("sync1_cycle".into(), Json::signed(-1));
                },
                codes::MODE_INVALID,
            ),
            (
                "dc sync1_shift out of range",
                |document| {
                    dc(document)
                        .insert("sync1_shift".into(), Json::Number(i128::from(i32::MAX) + 1));
                },
                codes::MODE_INVALID,
            ),
            (
                "provenance not string",
                |document| {
                    mode_object(document).insert("provenance".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
            (
                "selected not string",
                |document| {
                    device(document).insert("selected".into(), Json::Bool(true));
                },
                codes::MODE_INVALID,
            ),
        ];
        for (label, mutate, code) in cases {
            let error = parse_error(mutate);
            assert_eq!(error.code, code, "{label}");
        }
    }

    #[test]
    fn configuration_parse_defaults_optional_constraints_and_nullable_fields() {
        let parsed = parse_document(|document| {
            pdo(document).remove("fixed");
            pdo(document).remove("mandatory");
            pdo(document).remove("exclude");
            pdo(document).remove("excluded_sm");
            device(document).remove("mailbox");
            device(document).insert("esi".into(), Json::Null);
            mode_object(document).insert("description".into(), Json::Null);
            mode_object(document).insert("provenance".into(), Json::Null);
            dc(document).clear();
            dc(document).insert("mode".into(), Json::string("free-run"));
        });
        let device = &parsed.devices[0];
        assert!(device.esi.is_none());
        assert_eq!(device.mailbox, MailboxFacts::default());
        assert!(device.modes[0].description.is_none());
        assert!(device.modes[0].provenance.is_none());
        assert_eq!(device.modes[0].dc, DcChoice::FreeRun);
        let pdo = &device.modes[0].pdos.pdos[0];
        assert!(!pdo.fixed);
        assert!(!pdo.mandatory);
        assert!(pdo.exclude.is_empty());
        assert!(pdo.excluded_sm.is_empty());
    }
}
