//! Desired and observed EtherCAT topology (WL-7 phase 2).
//!
//! Three layers stay apart (WL-7 plan §2): the engineer-authored **desired
//! topology**, the **observed topology** reported by a bus scan, and the
//! per-position **comparison** between them. Comparison is policy-free: every
//! check reports `match`, `different`, `unverified` or `not applicable`, an
//! unavailable read-back is never a match, and nothing here rewrites project
//! intent (D-277, D-278).
//!
//! The desired topology is authored offline: a unique dense position per
//! master plus the exact vendor id, product code and revision, with serial
//! number and station alias as optional instance checks. Absence of a serial
//! or alias means "no check", never a wildcard (D-277).
//!
//! The topology draft is a JSON document because the durable project schema —
//! the dedicated engineering configuration file declared by an `[io]` key — is
//! Phase 5 work (D-263, Q9/Q24). Phase 5 replaces the draft file with that
//! schema and keeps the typed model in this module unchanged.

use std::collections::BTreeSet;

use stc_got::json::{self, Json};

use super::abi::SlaveIdentity;
use super::diagnostics::{EngineeringDiagnostic, codes};

/// Format version of the topology draft document.
pub const TOPOLOGY_FORMAT: u32 = 1;

/// Protocol name of the topology draft document.
pub const TOPOLOGY_PROTOCOL: &str = "stc-ethercat-topology/1";

/// Format version of a recorded observation document.
pub const OBSERVED_FORMAT: u32 = 1;

/// Protocol name of a recorded observation document.
pub const OBSERVED_PROTOCOL: &str = "stc-ethercat-observed/1";

/// The exact identity basis of one configured device (D-277).
///
/// Wildcards, revision ranges and product families cannot be expressed: the
/// only accepted identity is one exact vendor id, product code and revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeviceIdentity {
    /// Vendor id.
    pub vendor_id: u32,
    /// Product code.
    pub product_code: u32,
    /// Revision number.
    pub revision: u32,
}

impl DeviceIdentity {
    /// Creates an identity.
    #[must_use]
    pub fn new(vendor_id: u32, product_code: u32, revision: u32) -> Self {
        Self {
            vendor_id,
            product_code,
            revision,
        }
    }

    /// Stable spelling used in structured output and diagnostics.
    #[must_use]
    pub fn describe(self) -> String {
        format!(
            "#x{:08x}/#x{:08x}/#x{:08x}",
            self.vendor_id, self.product_code, self.revision
        )
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(self) -> Json {
        Json::object([
            (
                "vendor_id".into(),
                Json::unsigned(u64::from(self.vendor_id)),
            ),
            (
                "product_code".into(),
                Json::unsigned(u64::from(self.product_code)),
            ),
            ("revision".into(), Json::unsigned(u64::from(self.revision))),
        ])
    }
}

/// One engineer-authored device in the desired topology.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfiguredDevice {
    /// Logical device association, unique per master.
    pub name: String,
    /// Machine slot: a unique, dense position on the master.
    pub position: u16,
    /// Exact identity basis.
    pub identity: DeviceIdentity,
    /// Optional instance check; absent means no check (D-277).
    pub serial: Option<u32>,
    /// Optional instance check; absent means no check (D-277).
    pub alias: Option<u16>,
}

impl ConfiguredDevice {
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
                "serial".into(),
                Json::optional(self.serial.map(|value| Json::unsigned(u64::from(value)))),
            ),
            (
                "alias".into(),
                Json::optional(self.alias.map(|value| Json::unsigned(u64::from(value)))),
            ),
        ])
    }
}

/// The engineer-authored desired topology of one managed master.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Topology {
    /// Managed master index.
    pub master: u16,
    /// Configured devices, in position order once parsed or captured.
    pub devices: Vec<ConfiguredDevice>,
}

impl Topology {
    /// Creates an empty topology for one master.
    #[must_use]
    pub fn new(master: u16) -> Self {
        Self {
            master,
            devices: Vec::new(),
        }
    }

    /// Creates a topology and sorts its devices by position.
    #[must_use]
    pub fn with_devices(master: u16, mut devices: Vec<ConfiguredDevice>) -> Self {
        devices.sort_by_key(|device| device.position);
        Self { master, devices }
    }

    /// Returns the device configured at `position`.
    #[must_use]
    pub fn device_at(&self, position: u16) -> Option<&ConfiguredDevice> {
        self.devices
            .iter()
            .find(|device| device.position == position)
    }

    /// Validates the desired topology (D-277).
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-009` for an empty, duplicate or non-dense
    /// position, `STC-E-IO-ETH-010` for an identity that is not exact, and
    /// `STC-E-IO-ETH-011` for a missing or duplicated logical device name.
    pub fn validate(&self) -> Result<(), EngineeringDiagnostic> {
        if self.devices.is_empty() {
            return Err(invalid_position(
                "topology.validate",
                "at least one configured device",
                "no devices",
            ));
        }
        let mut positions: Vec<u16> = self.devices.iter().map(|device| device.position).collect();
        positions.sort_unstable();
        for (index, position) in positions.iter().enumerate() {
            if usize::from(*position) != index {
                return Err(invalid_position(
                    "topology.validate",
                    &format!("a dense position {index}"),
                    &format!("position {position}"),
                ));
            }
        }
        let mut names = BTreeSet::new();
        for device in &self.devices {
            if device.name.trim().is_empty() {
                return Err(invalid_name(
                    "topology.validate",
                    "a non-empty logical device name",
                    "an empty name",
                    device.position,
                ));
            }
            if !names.insert(device.name.as_str()) {
                return Err(invalid_name(
                    "topology.validate",
                    "a unique logical device name",
                    &format!("duplicate name `{}`", device.name),
                    device.position,
                ));
            }
            if device.identity.vendor_id == 0 || device.identity.product_code == 0 {
                return Err(invalid_identity(
                    "topology.validate",
                    &device.name,
                    device.position,
                    "an exact vendor id and product code",
                    &device.identity.describe(),
                ));
            }
        }
        Ok(())
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("format".into(), Json::unsigned(u64::from(TOPOLOGY_FORMAT))),
            ("protocol".into(), Json::string(TOPOLOGY_PROTOCOL)),
            ("master".into(), Json::unsigned(u64::from(self.master))),
            (
                "devices".into(),
                Json::Array(self.devices.iter().map(ConfiguredDevice::to_json).collect()),
            ),
        ])
    }

    /// Deterministic document text.
    #[must_use]
    pub fn to_text(&self) -> String {
        self.to_json().render()
    }

    /// Parses a topology draft document.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-009` for a malformed document, unsupported
    /// format or unknown key, `STC-E-IO-ETH-010` for an identity that is
    /// missing, non-numeric or a wildcard, and `STC-E-IO-ETH-011` for a
    /// missing device name. Structural validation is a separate step.
    pub fn parse(text: &str) -> Result<Self, EngineeringDiagnostic> {
        let value = json::parse(text).map_err(|error| {
            invalid_position(
                "topology.parse",
                "a JSON topology draft",
                &format!("not valid JSON: {}", error.message),
            )
        })?;
        let object = value.as_object().ok_or_else(|| {
            invalid_position(
                "topology.parse",
                "a JSON topology draft object",
                "a non-object document",
            )
        })?;
        reject_unknown_keys(
            object.keys().map(String::as_str),
            &["format", "protocol", "master", "devices"],
            "topology.parse",
        )?;
        if number(object.get("format"), "format", "topology.parse")? != u64::from(TOPOLOGY_FORMAT) {
            return Err(invalid_position(
                "topology.parse",
                &format!("format {TOPOLOGY_FORMAT}"),
                "an unsupported format",
            ));
        }
        let protocol = object
            .get("protocol")
            .and_then(Json::as_str)
            .ok_or_else(|| {
                invalid_position("topology.parse", "a protocol string", "no protocol")
            })?;
        if protocol != TOPOLOGY_PROTOCOL {
            return Err(invalid_position(
                "topology.parse",
                TOPOLOGY_PROTOCOL,
                &format!("protocol `{protocol}`"),
            ));
        }
        let master = u16::try_from(number(object.get("master"), "master", "topology.parse")?)
            .map_err(|_| {
                invalid_position(
                    "topology.parse",
                    "a master index in 0..=65535",
                    "a master index out of range",
                )
            })?;
        let entries = object
            .get("devices")
            .and_then(Json::as_array)
            .ok_or_else(|| {
                invalid_position("topology.parse", "a devices array", "no devices array")
            })?;
        let mut devices = Vec::with_capacity(entries.len());
        for entry in entries {
            devices.push(parse_device(entry)?);
        }
        Ok(Self::with_devices(master, devices))
    }
}

fn parse_device(value: &Json) -> Result<ConfiguredDevice, EngineeringDiagnostic> {
    let object = value.as_object().ok_or_else(|| {
        invalid_position(
            "topology.parse",
            "a device object",
            "a non-object device entry",
        )
    })?;
    reject_unknown_keys(
        object.keys().map(String::as_str),
        &[
            "name",
            "position",
            "vendor_id",
            "product_code",
            "revision",
            "serial",
            "alias",
        ],
        "topology.parse",
    )?;
    let name = object
        .get("name")
        .and_then(Json::as_str)
        .map(str::to_owned)
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| {
            invalid_name(
                "topology.parse",
                "a non-empty logical device name",
                "no name",
                0,
            )
        })?;
    let position = u16::try_from(number(
        object.get("position"),
        "position",
        "topology.parse",
    )?)
    .map_err(|_| {
        invalid_position(
            "topology.parse",
            "a position in 0..=65535",
            "a position out of range",
        )
    })?;
    let identity = DeviceIdentity::new(
        identity_field(object.get("vendor_id"), "vendor_id", &name, position)?,
        identity_field(object.get("product_code"), "product_code", &name, position)?,
        identity_field(object.get("revision"), "revision", &name, position)?,
    );
    let serial = match object.get("serial") {
        None | Some(Json::Null) => None,
        Some(value) => Some(
            u32::try_from(value.as_number().ok_or_else(|| {
                invalid_identity(
                    "topology.parse",
                    &name,
                    position,
                    "a numeric serial number or null",
                    "a non-numeric serial number",
                )
            })?)
            .map_err(|_| {
                invalid_identity(
                    "topology.parse",
                    &name,
                    position,
                    "a serial number in 0..=4294967295",
                    "a serial number out of range",
                )
            })?,
        ),
    };
    let alias = match object.get("alias") {
        None | Some(Json::Null) => None,
        Some(value) => Some(
            u16::try_from(value.as_number().ok_or_else(|| {
                invalid_identity(
                    "topology.parse",
                    &name,
                    position,
                    "a numeric station alias or null",
                    "a non-numeric station alias",
                )
            })?)
            .map_err(|_| {
                invalid_identity(
                    "topology.parse",
                    &name,
                    position,
                    "a station alias in 0..=65535",
                    "a station alias out of range",
                )
            })?,
        ),
    };
    Ok(ConfiguredDevice {
        name,
        position,
        identity,
        serial,
        alias,
    })
}

fn identity_field(
    value: Option<&Json>,
    field: &str,
    name: &str,
    position: u16,
) -> Result<u32, EngineeringDiagnostic> {
    let value = value.ok_or_else(|| {
        invalid_identity(
            "topology.parse",
            name,
            position,
            &format!("an exact `{field}`"),
            &format!("no `{field}`"),
        )
    })?;
    let number = value.as_number().ok_or_else(|| {
        let observed = value.as_str().map_or_else(
            || "a non-numeric value".to_owned(),
            |text| format!("`{text}` is not an exact identity"),
        );
        invalid_identity(
            "topology.parse",
            name,
            position,
            &format!("an exact numeric `{field}`"),
            &observed,
        )
    })?;
    u32::try_from(number).map_err(|_| {
        invalid_identity(
            "topology.parse",
            name,
            position,
            &format!("`{field}` in 0..=4294967295"),
            &format!("`{field}` out of range"),
        )
    })
}

fn number(
    value: Option<&Json>,
    field: &str,
    operation: &'static str,
) -> Result<u64, EngineeringDiagnostic> {
    let value = value.and_then(Json::as_number).ok_or_else(|| {
        invalid_position(
            operation,
            &format!("a numeric `{field}`"),
            &format!("no numeric `{field}`"),
        )
    })?;
    u64::try_from(value).map_err(|_| {
        invalid_position(
            operation,
            &format!("`{field}` in 0..=18446744073709551615"),
            &format!("`{field}` out of range"),
        )
    })
}

fn reject_unknown_keys<'a>(
    keys: impl Iterator<Item = &'a str>,
    allowed: &[&str],
    operation: &'static str,
) -> Result<(), EngineeringDiagnostic> {
    for key in keys {
        if !allowed.contains(&key) {
            return Err(invalid_position(
                operation,
                &format!("one of {allowed:?}"),
                &format!("unknown key `{key}`"),
            ));
        }
    }
    Ok(())
}

fn invalid_position(
    operation: &'static str,
    expected: &str,
    observed: &str,
) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::TOPOLOGY_POSITION,
        operation,
        "fix the topology draft or capture a fresh one",
    )
    .with_expected(expected)
    .with_observed(observed)
}

fn invalid_identity(
    operation: &'static str,
    name: &str,
    position: u16,
    expected: &str,
    observed: &str,
) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::TOPOLOGY_IDENTITY,
        operation,
        "author the exact vendor id, product code and revision",
    )
    .with_device(name)
    .with_position(position)
    .with_expected(expected)
    .with_observed(observed)
}

fn invalid_name(
    operation: &'static str,
    expected: &str,
    observed: &str,
    position: u16,
) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::TOPOLOGY_NAME,
        operation,
        "give every device a unique, non-empty logical name",
    )
    .with_position(position)
    .with_expected(expected)
    .with_observed(observed)
}

/// The observed topology of one master, in position order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedTopology {
    /// Master index the observation was taken from.
    pub master: u16,
    /// Observed slaves, sorted by position.
    pub slaves: Vec<SlaveIdentity>,
}

impl ObservedTopology {
    /// Creates an observation and sorts it by position.
    #[must_use]
    pub fn new(master: u16, mut slaves: Vec<SlaveIdentity>) -> Self {
        slaves.sort_by_key(|slave| slave.position);
        Self { master, slaves }
    }

    /// Returns the slave observed at `position`.
    #[must_use]
    pub fn slave(&self, position: u16) -> Option<&SlaveIdentity> {
        self.slaves.iter().find(|slave| slave.position == position)
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("format".into(), Json::unsigned(u64::from(OBSERVED_FORMAT))),
            ("protocol".into(), Json::string(OBSERVED_PROTOCOL)),
            ("master".into(), Json::unsigned(u64::from(self.master))),
            (
                "slaves".into(),
                Json::Array(self.slaves.iter().map(SlaveIdentity::to_json).collect()),
            ),
        ])
    }

    /// Deterministic document text.
    #[must_use]
    pub fn to_text(&self) -> String {
        self.to_json().render()
    }

    /// Parses a recorded observation.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-009` for a malformed document, unsupported
    /// format or unknown key.
    pub fn parse(text: &str) -> Result<Self, EngineeringDiagnostic> {
        let value = json::parse(text).map_err(|error| {
            invalid_position(
                "topology.observe",
                "a JSON observation",
                &format!("not valid JSON: {}", error.message),
            )
        })?;
        let object = value.as_object().ok_or_else(|| {
            invalid_position(
                "topology.observe",
                "a JSON observation object",
                "a non-object document",
            )
        })?;
        reject_unknown_keys(
            object.keys().map(String::as_str),
            &["format", "protocol", "master", "slaves"],
            "topology.observe",
        )?;
        if number(object.get("format"), "format", "topology.observe")? != u64::from(OBSERVED_FORMAT)
        {
            return Err(invalid_position(
                "topology.observe",
                &format!("format {OBSERVED_FORMAT}"),
                "an unsupported format",
            ));
        }
        let protocol = object
            .get("protocol")
            .and_then(Json::as_str)
            .ok_or_else(|| {
                invalid_position("topology.observe", "a protocol string", "no protocol")
            })?;
        if protocol != OBSERVED_PROTOCOL {
            return Err(invalid_position(
                "topology.observe",
                OBSERVED_PROTOCOL,
                &format!("protocol `{protocol}`"),
            ));
        }
        let master = u16::try_from(number(object.get("master"), "master", "topology.observe")?)
            .map_err(|_| {
                invalid_position(
                    "topology.observe",
                    "a master index in 0..=65535",
                    "a master index out of range",
                )
            })?;
        let entries = object
            .get("slaves")
            .and_then(Json::as_array)
            .ok_or_else(|| {
                invalid_position("topology.observe", "a slaves array", "no slaves array")
            })?;
        let mut slaves = Vec::with_capacity(entries.len());
        for entry in entries {
            slaves.push(parse_slave(entry)?);
        }
        Ok(Self::new(master, slaves))
    }
}

fn parse_slave(value: &Json) -> Result<SlaveIdentity, EngineeringDiagnostic> {
    let object = value.as_object().ok_or_else(|| {
        invalid_position(
            "topology.observe",
            "a slave object",
            "a non-object slave entry",
        )
    })?;
    reject_unknown_keys(
        object.keys().map(String::as_str),
        &[
            "position",
            "vendor_id",
            "product_code",
            "revision",
            "serial",
            "alias",
            "al_state",
            "online",
        ],
        "topology.observe",
    )?;
    let position = u16::try_from(number(
        object.get("position"),
        "position",
        "topology.observe",
    )?)
    .map_err(|_| {
        invalid_position(
            "topology.observe",
            "a position in 0..=65535",
            "a position out of range",
        )
    })?;
    let field = |key: &str| -> Result<u32, EngineeringDiagnostic> {
        let value = object.get(key).and_then(Json::as_number).ok_or_else(|| {
            invalid_position(
                "topology.observe",
                &format!("a numeric `{key}`"),
                &format!("no numeric `{key}`"),
            )
        })?;
        u32::try_from(value).map_err(|_| {
            invalid_position(
                "topology.observe",
                &format!("`{key}` in 0..=4294967295"),
                &format!("`{key}` out of range"),
            )
        })
    };
    let optional = |key: &str, max: u64| -> Result<Option<u32>, EngineeringDiagnostic> {
        match object.get(key) {
            None | Some(Json::Null) => Ok(None),
            Some(value) => {
                let number = value.as_number().ok_or_else(|| {
                    invalid_position(
                        "topology.observe",
                        &format!("a numeric `{key}` or null"),
                        &format!("a non-numeric `{key}`"),
                    )
                })?;
                match u64::try_from(number).ok().filter(|number| *number <= max) {
                    Some(number) => Ok(Some(number as u32)),
                    None => Err(invalid_position(
                        "topology.observe",
                        &format!("`{key}` in 0..={max}"),
                        &format!("`{key}` out of range"),
                    )),
                }
            }
        }
    };
    let online = match object.get("online") {
        Some(Json::Bool(online)) => *online,
        _ => {
            return Err(invalid_position(
                "topology.observe",
                "a boolean `online`",
                "no boolean `online`",
            ));
        }
    };
    Ok(SlaveIdentity {
        position,
        vendor_id: field("vendor_id")?,
        product_code: field("product_code")?,
        revision: field("revision")?,
        serial: optional("serial", u64::from(u32::MAX))?,
        alias: optional("alias", u64::from(u16::MAX))?.map(|alias| alias as u16),
        al_state: optional("al_state", u64::from(u8::MAX))?.map(|state| state as u8),
        online,
    })
}

/// Result of one topology check (D-278, Q16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckResult {
    /// The observed fact equals the configured fact.
    Match,
    /// The observed fact differs from the configured fact.
    Different,
    /// The fact could not be read back; never treated as a match.
    Unverified,
    /// The check does not apply, for example an undeclared optional serial.
    NotApplicable,
}

impl CheckResult {
    /// Stable spelling used in structured output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Match => "match",
            Self::Different => "different",
            Self::Unverified => "unverified",
            Self::NotApplicable => "not applicable",
        }
    }

    /// Reports whether the result needs no engineer action.
    #[must_use]
    pub fn is_clean(self) -> bool {
        matches!(self, Self::Match | Self::NotApplicable)
    }

    fn rank(self) -> u8 {
        match self {
            Self::NotApplicable => 0,
            Self::Match => 1,
            Self::Unverified => 2,
            Self::Different => 3,
        }
    }

    /// The more significant of two results.
    #[must_use]
    pub fn worst(self, other: Self) -> Self {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(self) -> Json {
        Json::string(self.as_str())
    }
}

/// One named field of a position comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldCheck {
    /// Field name, for example `identity` or `serial`.
    pub field: &'static str,
    /// Check result.
    pub result: CheckResult,
    /// What the project configured, when applicable.
    pub expected: Option<String>,
    /// What the bus reported, when applicable.
    pub observed: Option<String>,
}

impl FieldCheck {
    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("field".into(), Json::string(self.field)),
            ("result".into(), self.result.to_json()),
            (
                "expected".into(),
                Json::optional(
                    self.expected
                        .as_ref()
                        .map(|value| Json::string(value.as_str())),
                ),
            ),
            (
                "observed".into(),
                Json::optional(
                    self.observed
                        .as_ref()
                        .map(|value| Json::string(value.as_str())),
                ),
            ),
        ])
    }
}

fn field(
    field: &'static str,
    result: CheckResult,
    expected: impl Into<Option<String>>,
    observed: impl Into<Option<String>>,
) -> FieldCheck {
    FieldCheck {
        field,
        result,
        expected: expected.into(),
        observed: observed.into(),
    }
}

/// The checks and diagnostics for one configured position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PositionCheck {
    /// Field checks, in a stable order.
    pub checks: Vec<FieldCheck>,
    /// Overall result: the most significant field result.
    pub result: CheckResult,
    /// Identity, serial or alias mismatches, when any.
    pub diagnostics: Vec<EngineeringDiagnostic>,
}

/// Compares one configured device with the slave observed at its position.
///
/// A missing slave is `different`; an offline slave is `unverified` because
/// its identity cannot be read back. Undeclared optional instance checks are
/// `not applicable`, never a wildcard match (D-277, D-278).
#[must_use]
pub fn compare_position(
    device: &ConfiguredDevice,
    observed: Option<&SlaveIdentity>,
) -> PositionCheck {
    let mut checks = Vec::new();
    let mut diagnostics = Vec::new();
    let Some(slave) = observed else {
        checks.push(field(
            "presence",
            CheckResult::Different,
            "present".to_owned(),
            "absent".to_owned(),
        ));
        return PositionCheck {
            checks,
            result: CheckResult::Different,
            diagnostics,
        };
    };
    checks.push(field(
        "presence",
        CheckResult::Match,
        "present".to_owned(),
        "present".to_owned(),
    ));
    if !slave.online {
        checks.push(field(
            "online",
            CheckResult::Unverified,
            "online".to_owned(),
            "not responding".to_owned(),
        ));
        return PositionCheck {
            checks,
            result: CheckResult::Unverified,
            diagnostics,
        };
    }
    checks.push(field(
        "online",
        CheckResult::Match,
        "online".to_owned(),
        "online".to_owned(),
    ));

    let mut result = CheckResult::Match;
    let observed_identity =
        DeviceIdentity::new(slave.vendor_id, slave.product_code, slave.revision);
    let identity = if observed_identity == device.identity {
        CheckResult::Match
    } else {
        CheckResult::Different
    };
    checks.push(field(
        "identity",
        identity,
        device.identity.describe(),
        observed_identity.describe(),
    ));
    if identity == CheckResult::Different {
        diagnostics.push(
            EngineeringDiagnostic::new(
                codes::ETH_SLAVE,
                "topology.compare",
                "review the configured identity and the connected device",
            )
            .with_device(device.name.as_str())
            .with_position(device.position)
            .with_identity(
                device.identity.vendor_id,
                device.identity.product_code,
                Some(device.identity.revision),
            )
            .with_expected(device.identity.describe())
            .with_observed(observed_identity.describe()),
        );
    }
    result = result.worst(identity);

    let instance = |configured: Option<u64>,
                    observed: Option<u64>,
                    name: &'static str,
                    label: &str|
     -> (FieldCheck, CheckResult, Option<EngineeringDiagnostic>) {
        match (configured, observed) {
            (None, _) => (
                field(name, CheckResult::NotApplicable, None, None),
                CheckResult::NotApplicable,
                None,
            ),
            (Some(expected), Some(actual)) if expected == actual => (
                field(
                    name,
                    CheckResult::Match,
                    format!("{label} {expected}"),
                    format!("{label} {actual}"),
                ),
                CheckResult::Match,
                None,
            ),
            (Some(expected), Some(actual)) => (
                field(
                    name,
                    CheckResult::Different,
                    format!("{label} {expected}"),
                    format!("{label} {actual}"),
                ),
                CheckResult::Different,
                Some(
                    EngineeringDiagnostic::new(
                        codes::ETH_SLAVE,
                        "topology.compare",
                        "review the declared instance check or the connected device",
                    )
                    .with_device(device.name.as_str())
                    .with_position(device.position)
                    .with_expected(format!("{label} {expected}"))
                    .with_observed(format!("{label} {actual}")),
                ),
            ),
            (Some(expected), None) => (
                field(
                    name,
                    CheckResult::Unverified,
                    format!("{label} {expected}"),
                    format!("{label} not readable"),
                ),
                CheckResult::Unverified,
                None,
            ),
        }
    };

    let (serial_check, serial_result, serial_diagnostic) = instance(
        device.serial.map(u64::from),
        slave.serial.map(u64::from),
        "serial",
        "serial",
    );
    checks.push(serial_check);
    result = result.worst(serial_result);
    diagnostics.extend(serial_diagnostic);

    let (alias_check, alias_result, alias_diagnostic) = instance(
        device.alias.map(u64::from),
        slave.alias.map(u64::from),
        "alias",
        "alias",
    );
    checks.push(alias_check);
    result = result.worst(alias_result);
    diagnostics.extend(alias_diagnostic);

    PositionCheck {
        checks,
        result,
        diagnostics,
    }
}

/// One configured position in a desired-vs-observed comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PositionComparison {
    /// Configured machine slot.
    pub position: u16,
    /// Logical device name.
    pub device: String,
    /// Overall result.
    pub result: CheckResult,
    /// Field checks.
    pub checks: Vec<FieldCheck>,
    /// The observed slave, when one responds at the position.
    pub observed: Option<SlaveIdentity>,
}

impl PositionComparison {
    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("position".into(), Json::unsigned(u64::from(self.position))),
            ("device".into(), Json::string(self.device.as_str())),
            ("result".into(), self.result.to_json()),
            (
                "checks".into(),
                Json::Array(self.checks.iter().map(FieldCheck::to_json).collect()),
            ),
            (
                "observed".into(),
                Json::optional(self.observed.as_ref().map(SlaveIdentity::to_json)),
            ),
        ])
    }
}

/// The policy-free comparison of a desired topology with an observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologyComparison {
    /// Master index.
    pub master: u16,
    /// One entry per configured position, evaluated independently.
    pub positions: Vec<PositionComparison>,
    /// Observed slaves the project does not configure.
    pub extra: Vec<SlaveIdentity>,
    /// Structured diagnostics for mismatches, extras and shifts.
    pub diagnostics: Vec<EngineeringDiagnostic>,
}

impl TopologyComparison {
    /// Reports whether every configured position matches and nothing is extra.
    #[must_use]
    pub fn is_match(&self) -> bool {
        self.extra.is_empty()
            && self
                .positions
                .iter()
                .all(|position| position.result.is_clean())
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("master".into(), Json::unsigned(u64::from(self.master))),
            ("match".into(), Json::Bool(self.is_match())),
            (
                "positions".into(),
                Json::Array(
                    self.positions
                        .iter()
                        .map(PositionComparison::to_json)
                        .collect(),
                ),
            ),
            (
                "extra".into(),
                Json::Array(self.extra.iter().map(SlaveIdentity::to_json).collect()),
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

/// Compares the desired topology with an observation, per position (D-278).
///
/// Every configured position is evaluated independently: a present but
/// non-operational slave is `unverified` rather than absent, a missing slave
/// makes only its own position `different`, and observed slaves the project
/// does not configure are reported as extras. Nothing is renumbered, rebound
/// or repaired.
///
/// # Errors
///
/// Returns the validation diagnostic of the desired topology, or
/// `STC-E-IO-ETH-009` when the observation belongs to another master.
pub fn compare(
    topology: &Topology,
    observed: &ObservedTopology,
) -> Result<TopologyComparison, EngineeringDiagnostic> {
    topology.validate()?;
    if topology.master != observed.master {
        return Err(invalid_position(
            "topology.compare",
            &format!("master {}", topology.master),
            &format!("master {}", observed.master),
        ));
    }
    let highest_observed = observed.slaves.iter().map(|slave| slave.position).max();
    let mut positions = Vec::with_capacity(topology.devices.len());
    let mut diagnostics = Vec::new();
    for device in &topology.devices {
        let slave = observed.slave(device.position);
        let check = compare_position(device, slave);
        diagnostics.extend(check.diagnostics);
        if slave.is_none()
            && let Some(highest) = highest_observed
            && highest > device.position
        {
            diagnostics.push(
                EngineeringDiagnostic::new(
                    codes::TOPOLOGY_DOWNSTREAM_SHIFT,
                    "topology.compare",
                    "restore the missing slave; positions are never renumbered",
                )
                .with_device(device.name.as_str())
                .with_position(device.position)
                .with_expected(format!("a slave at position {}", device.position))
                .with_observed(format!(
                    "the chain shifted; position {highest} still responds"
                )),
            );
        }
        positions.push(PositionComparison {
            position: device.position,
            device: device.name.clone(),
            result: check.result,
            checks: check.checks,
            observed: slave.cloned(),
        });
    }
    let mut extra = Vec::new();
    for slave in &observed.slaves {
        if topology.device_at(slave.position).is_none() {
            extra.push(slave.clone());
            diagnostics.push(
                EngineeringDiagnostic::new(
                    codes::TOPOLOGY_EXTRA_SLAVE,
                    "topology.compare",
                    "review the machine wiring or capture the observed topology explicitly",
                )
                .with_position(slave.position)
                .with_observed(
                    DeviceIdentity::new(slave.vendor_id, slave.product_code, slave.revision)
                        .describe(),
                ),
            );
        }
    }
    Ok(TopologyComparison {
        master: topology.master,
        positions,
        extra,
        diagnostics,
    })
}

/// What an explicit capture records (D-277).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaptureOptions {
    /// Pin the observed serial number as a hard instance check.
    pub pin_serial: bool,
    /// Pin the observed station alias as a hard instance check.
    pub pin_alias: bool,
}

/// Captures an observed topology into a new desired-topology draft.
///
/// Default capture records vendor id, product code and revision and does not
/// pin serial numbers or aliases, so a spare device stays usable in the same
/// slot. Serial and alias are captured only on explicit request. The `base`
/// topology is never modified: its logical device names are reused where a
/// position still matches, so an explicit capture does not discard intent.
///
/// # Errors
///
/// Returns `STC-E-IO-ETH-009` for an empty or non-dense observation,
/// `STC-E-IO-ETH-010` for an unreadable identity, and
/// `STC-E-IO-ETH-015` for a slave whose declared instance data cannot be read.
pub fn capture(
    base: Option<&Topology>,
    observed: &ObservedTopology,
    options: CaptureOptions,
) -> Result<Topology, EngineeringDiagnostic> {
    if observed.slaves.is_empty() {
        return Err(invalid_position(
            "topology.capture",
            "at least one responding slave",
            "the scan reported no slaves",
        ));
    }
    if let Some(base) = base
        && base.master != observed.master
    {
        return Err(invalid_position(
            "topology.capture",
            &format!("master {}", base.master),
            &format!("master {}", observed.master),
        ));
    }
    let mut devices = Vec::with_capacity(observed.slaves.len());
    for slave in &observed.slaves {
        if !slave.online {
            return Err(unreadable_observed(
                slave.position,
                "a responding slave",
                "not responding",
            ));
        }
        if slave.vendor_id == 0 || slave.product_code == 0 {
            return Err(invalid_identity(
                "topology.capture",
                &format!("slave{}", slave.position),
                slave.position,
                "a readable vendor id and product code",
                "an unreadable identity",
            ));
        }
        let serial = if options.pin_serial {
            Some(slave.serial.ok_or_else(|| {
                unreadable_observed(slave.position, "a readable serial number", "not readable")
            })?)
        } else {
            None
        };
        let alias = if options.pin_alias {
            Some(slave.alias.ok_or_else(|| {
                unreadable_observed(slave.position, "a readable station alias", "not readable")
            })?)
        } else {
            None
        };
        let name = base
            .and_then(|base| base.device_at(slave.position))
            .map_or_else(
                || format!("slave{}", slave.position),
                |device| device.name.clone(),
            );
        devices.push(ConfiguredDevice {
            name,
            position: slave.position,
            identity: DeviceIdentity::new(slave.vendor_id, slave.product_code, slave.revision),
            serial,
            alias,
        });
    }
    let topology = Topology::with_devices(observed.master, devices);
    topology.validate()?;
    Ok(topology)
}

fn unreadable_observed(position: u16, expected: &str, observed: &str) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::TOPOLOGY_OBSERVED_UNREADABLE,
        "topology.capture",
        "make the slave respond and readable, or capture without the instance check",
    )
    .with_position(position)
    .with_expected(expected)
    .with_observed(observed)
}

/// One position inspected on the bus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlaveInspection {
    /// Inspected machine slot.
    pub position: u16,
    /// Observed slave facts, when one responds.
    pub observed: Option<SlaveIdentity>,
    /// Configured device at the position, when the project declares one.
    pub configured: Option<ConfiguredDevice>,
    /// Field checks against the configured device, when both sides exist.
    pub checks: Vec<FieldCheck>,
    /// Overall result; `not applicable` when the position is unconfigured.
    pub result: CheckResult,
    /// Identity, serial or alias mismatches, when any.
    pub diagnostics: Vec<EngineeringDiagnostic>,
}

impl SlaveInspection {
    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("position".into(), Json::unsigned(u64::from(self.position))),
            (
                "observed".into(),
                Json::optional(self.observed.as_ref().map(SlaveIdentity::to_json)),
            ),
            (
                "configured".into(),
                Json::optional(self.configured.as_ref().map(ConfiguredDevice::to_json)),
            ),
            ("result".into(), self.result.to_json()),
            (
                "checks".into(),
                Json::Array(self.checks.iter().map(FieldCheck::to_json).collect()),
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

/// Inspects one observed position against the desired topology (D-277).
#[must_use]
pub fn inspect_position(
    position: u16,
    observed: &ObservedTopology,
    topology: Option<&Topology>,
) -> SlaveInspection {
    let slave = observed.slave(position);
    let configured = topology
        .and_then(|topology| topology.device_at(position))
        .cloned();
    match &configured {
        Some(device) => {
            let check = compare_position(device, slave);
            SlaveInspection {
                position,
                observed: slave.cloned(),
                configured,
                checks: check.checks,
                result: check.result,
                diagnostics: check.diagnostics,
            }
        }
        None => SlaveInspection {
            position,
            observed: slave.cloned(),
            configured: None,
            checks: Vec::new(),
            result: CheckResult::NotApplicable,
            diagnostics: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, position: u16, identity: (u32, u32, u32)) -> ConfiguredDevice {
        ConfiguredDevice {
            name: name.into(),
            position,
            identity: DeviceIdentity::new(identity.0, identity.1, identity.2),
            serial: None,
            alias: None,
        }
    }

    fn slave(position: u16, online: bool) -> SlaveIdentity {
        SlaveIdentity {
            position,
            vendor_id: 0x0000_0b95,
            product_code: 0x0001_0200,
            revision: 1,
            serial: Some(7),
            alias: Some(0),
            al_state: Some(8),
            online,
        }
    }

    fn topology() -> Topology {
        Topology::with_devices(
            0,
            vec![
                device("input", 0, (0x0000_0b95, 0x0001_0200, 1)),
                device("output", 1, (0x0000_0b95, 0x0001_0200, 1)),
            ],
        )
    }

    #[test]
    fn a_dense_exact_topology_validates() {
        topology().validate().expect("valid");
        assert_eq!(topology().device_at(1).expect("device").name, "output");
        assert!(topology().device_at(2).is_none());
    }

    #[test]
    fn positions_must_be_dense_unique_and_present() {
        let error = Topology::new(0).validate().expect_err("empty");
        assert_eq!(error.code, "STC-E-IO-ETH-009");
        assert!(error.observed.expect("observed").contains("no devices"));

        let gap = Topology::with_devices(
            0,
            vec![device("a", 0, (1, 2, 3)), device("b", 2, (1, 2, 3))],
        );
        let error = gap.validate().expect_err("gap");
        assert_eq!(error.code, "STC-E-IO-ETH-009");
        assert!(error.expected.expect("expected").contains("position 1"));

        let duplicate = Topology::with_devices(
            0,
            vec![device("a", 0, (1, 2, 3)), device("b", 0, (1, 2, 3))],
        );
        assert_eq!(
            duplicate.validate().expect_err("duplicate").code,
            "STC-E-IO-ETH-009"
        );
    }

    #[test]
    fn names_are_unique_and_non_empty() {
        let empty = Topology::with_devices(0, vec![device("", 0, (1, 2, 3))]);
        let error = empty.validate().expect_err("empty name");
        assert_eq!(error.code, "STC-E-IO-ETH-011");

        let duplicate = Topology::with_devices(
            0,
            vec![device("a", 0, (1, 2, 3)), device("a", 1, (1, 2, 3))],
        );
        let error = duplicate.validate().expect_err("duplicate name");
        assert_eq!(error.code, "STC-E-IO-ETH-011");
        assert!(error.observed.expect("observed").contains("duplicate"));
    }

    #[test]
    fn identity_must_be_exact() {
        let wildcard = Topology::with_devices(0, vec![device("a", 0, (0, 2, 3))]);
        let error = wildcard.validate().expect_err("zero vendor");
        assert_eq!(error.code, "STC-E-IO-ETH-010");
        assert_eq!(error.position, Some(0));

        let text = r#"{"format":1,"protocol":"stc-ethercat-topology/1","master":0,"devices":[{"name":"a","position":0,"vendor_id":"any","product_code":2,"revision":3}]}"#;
        let error = Topology::parse(text).expect_err("wildcard");
        assert_eq!(error.code, "STC-E-IO-ETH-010");
        assert!(
            error
                .observed
                .expect("observed")
                .contains("not an exact identity")
        );
    }

    #[test]
    fn a_topology_round_trips_through_its_document() {
        let topology = topology();
        let text = topology.to_text();
        let parsed = Topology::parse(&text).expect("parse");
        assert_eq!(parsed, topology);
        assert!(text.contains("\"protocol\": \"stc-ethercat-topology/1\""));
        assert!(text.contains("\"serial\": null"));
        assert_eq!(parsed.devices[0].position, 0);
    }

    #[test]
    fn optional_instance_checks_round_trip() {
        let mut topology = topology();
        topology.devices[0].serial = Some(42);
        topology.devices[1].alias = Some(9);
        let parsed = Topology::parse(&topology.to_text()).expect("parse");
        assert_eq!(parsed.devices[0].serial, Some(42));
        assert_eq!(parsed.devices[1].alias, Some(9));
        assert_eq!(parsed.devices[0].alias, None);
    }

    #[test]
    fn malformed_documents_report_structure_and_unknown_keys() {
        for text in [
            "not json",
            "[]",
            r#"{"format":2,"protocol":"stc-ethercat-topology/1","master":0,"devices":[]}"#,
            r#"{"format":1,"protocol":"other","master":0,"devices":[]}"#,
            r#"{"format":1,"protocol":"stc-ethercat-topology/1","master":0}"#,
            r#"{"format":1,"protocol":"stc-ethercat-topology/1","master":0,"devices":[],"extra":1}"#,
            r#"{"format":1,"protocol":"stc-ethercat-topology/1","master":70000,"devices":[]}"#,
        ] {
            let error = Topology::parse(text).expect_err(text);
            assert_eq!(error.code, "STC-E-IO-ETH-009", "{text}");
        }
        let unknown = r#"{"format":1,"protocol":"stc-ethercat-topology/1","master":0,"devices":[{"name":"a","position":0,"vendor_id":1,"product_code":2,"revision":3,"width":4}]}"#;
        let error = Topology::parse(unknown).expect_err("unknown key");
        assert!(error.observed.expect("observed").contains("width"));
        let no_name = r#"{"format":1,"protocol":"stc-ethercat-topology/1","master":0,"devices":[{"position":0,"vendor_id":1,"product_code":2,"revision":3}]}"#;
        assert_eq!(
            Topology::parse(no_name).expect_err("no name").code,
            "STC-E-IO-ETH-011"
        );
    }

    #[test]
    fn comparison_reports_every_position_independently() {
        let observed = ObservedTopology::new(0, vec![slave(0, true), slave(1, true)]);
        let comparison = compare(&topology(), &observed).expect("compare");
        assert!(comparison.is_match());
        assert_eq!(comparison.positions.len(), 2);
        assert_eq!(comparison.positions[0].result, CheckResult::Match);
        assert!(comparison.diagnostics.is_empty());
        assert_eq!(comparison.positions[0].checks[2].field, "identity");
        let json = comparison.to_json().render();
        assert!(json.contains("\"match\": true"));
        assert!(json.contains("\"result\": \"match\""));
        assert!(json.contains("\"field\": \"alias\""));
        assert!(json.contains("\"result\": \"not applicable\""));
    }

    #[test]
    fn identity_serial_and_alias_mismatches_are_device_scoped() {
        let mut observed_slave = slave(0, true);
        observed_slave.revision = 2;
        observed_slave.serial = Some(9);
        let mut configured = topology();
        configured.devices[0].serial = Some(7);
        configured.devices[0].alias = Some(3);
        let comparison = compare(
            &configured,
            &ObservedTopology::new(0, vec![observed_slave, slave(1, true)]),
        )
        .expect("compare");
        assert!(!comparison.is_match());
        assert_eq!(comparison.positions[0].result, CheckResult::Different);
        assert_eq!(comparison.positions[1].result, CheckResult::Match);
        let codes: Vec<&str> = comparison
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert_eq!(
            codes,
            ["STC-E-IO-ETH-003", "STC-E-IO-ETH-003", "STC-E-IO-ETH-003"]
        );
        assert!(comparison.diagnostics[0].position == Some(0));
        assert!(
            comparison.diagnostics[0]
                .expected
                .as_ref()
                .expect("expected")
                .contains("#x")
        );
    }

    #[test]
    fn declared_instance_checks_never_match_unreadable_values() {
        let mut configured = topology();
        configured.devices[0].serial = Some(7);
        let mut observed_slave = slave(0, true);
        observed_slave.serial = None;
        let comparison = compare(
            &configured,
            &ObservedTopology::new(0, vec![observed_slave, slave(1, true)]),
        )
        .expect("compare");
        assert_eq!(comparison.positions[0].result, CheckResult::Unverified);
        assert!(comparison.diagnostics.is_empty());
        let serial = &comparison.positions[0].checks[3];
        assert_eq!(serial.field, "serial");
        assert_eq!(serial.result, CheckResult::Unverified);
    }

    #[test]
    fn an_offline_slave_is_unverified_not_absent() {
        let comparison = compare(
            &topology(),
            &ObservedTopology::new(0, vec![slave(0, false), slave(1, true)]),
        )
        .expect("compare");
        assert_eq!(comparison.positions[0].result, CheckResult::Unverified);
        assert_eq!(comparison.positions[0].checks[0].result, CheckResult::Match);
        assert_eq!(comparison.positions[0].checks[1].field, "online");
    }

    #[test]
    fn a_missing_slave_is_different_and_reports_the_downstream_shift() {
        let comparison =
            compare(&topology(), &ObservedTopology::new(0, vec![slave(1, true)])).expect("compare");
        assert_eq!(comparison.positions[0].result, CheckResult::Different);
        assert_eq!(comparison.positions[1].result, CheckResult::Match);
        let codes: Vec<&str> = comparison
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert_eq!(codes, ["STC-E-IO-ETH-014"]);
        assert_eq!(comparison.diagnostics[0].position, Some(0));

        let alone =
            compare(&topology(), &ObservedTopology::new(0, vec![slave(0, true)])).expect("compare");
        assert_eq!(alone.positions[1].result, CheckResult::Different);
        assert!(alone.diagnostics.is_empty());
    }

    #[test]
    fn observed_extras_are_reported_without_renumbering() {
        let comparison = compare(
            &topology(),
            &ObservedTopology::new(0, vec![slave(0, true), slave(1, true), slave(2, true)]),
        )
        .expect("compare");
        assert!(!comparison.is_match());
        assert_eq!(comparison.extra.len(), 1);
        assert_eq!(comparison.extra[0].position, 2);
        assert_eq!(comparison.diagnostics.len(), 1);
        assert_eq!(comparison.diagnostics[0].code, "STC-E-IO-ETH-012");
        assert_eq!(comparison.positions.len(), 2);
    }

    #[test]
    fn comparing_another_master_is_rejected() {
        let error = compare(&topology(), &ObservedTopology::new(1, vec![slave(0, true)]))
            .expect_err("master");
        assert_eq!(error.code, "STC-E-IO-ETH-009");
        assert_eq!(error.expected.as_deref(), Some("master 0"));
    }

    #[test]
    fn capture_records_identity_without_pinning_instance_checks() {
        let observed = ObservedTopology::new(0, vec![slave(1, true), slave(0, true)]);
        let captured = capture(None, &observed, CaptureOptions::default()).expect("capture");
        assert_eq!(captured.devices.len(), 2);
        assert_eq!(captured.devices[0].position, 0);
        assert_eq!(captured.devices[0].name, "slave0");
        assert_eq!(captured.devices[1].name, "slave1");
        assert_eq!(captured.devices[0].serial, None);
        assert_eq!(captured.devices[0].alias, None);
        assert_eq!(
            captured.devices[0].identity,
            DeviceIdentity::new(0x0000_0b95, 0x0001_0200, 1)
        );
        captured.validate().expect("valid");
    }

    #[test]
    fn capture_pins_instance_checks_only_on_request() {
        let observed = ObservedTopology::new(0, vec![slave(0, true)]);
        let pinned = capture(
            None,
            &observed,
            CaptureOptions {
                pin_serial: true,
                pin_alias: true,
            },
        )
        .expect("capture");
        assert_eq!(pinned.devices[0].serial, Some(7));
        assert_eq!(pinned.devices[0].alias, Some(0));

        let mut unreadable = slave(0, true);
        unreadable.serial = None;
        let error = capture(
            None,
            &ObservedTopology::new(0, vec![unreadable]),
            CaptureOptions {
                pin_serial: true,
                pin_alias: false,
            },
        )
        .expect_err("unreadable serial");
        assert_eq!(error.code, "STC-E-IO-ETH-015");
        assert_eq!(error.position, Some(0));
    }

    #[test]
    fn capture_reuses_names_and_never_touches_the_base() {
        let base = topology();
        let before = base.clone();
        let captured = capture(
            Some(&base),
            &ObservedTopology::new(0, vec![slave(0, true), slave(1, true)]),
            CaptureOptions::default(),
        )
        .expect("capture");
        assert_eq!(captured, base);
        assert_eq!(base, before);
        assert_eq!(captured.devices[0].name, "input");
    }

    #[test]
    fn capture_rejects_empty_offline_and_gapped_observations() {
        let error = capture(
            None,
            &ObservedTopology::new(0, Vec::new()),
            CaptureOptions::default(),
        )
        .expect_err("empty");
        assert_eq!(error.code, "STC-E-IO-ETH-009");

        let error = capture(
            None,
            &ObservedTopology::new(0, vec![slave(0, false)]),
            CaptureOptions::default(),
        )
        .expect_err("offline");
        assert_eq!(error.code, "STC-E-IO-ETH-015");

        let error = capture(
            None,
            &ObservedTopology::new(0, vec![slave(1, true)]),
            CaptureOptions::default(),
        )
        .expect_err("gap");
        assert_eq!(error.code, "STC-E-IO-ETH-009");
    }

    #[test]
    fn observations_round_trip_through_their_document() {
        let observed = ObservedTopology::new(0, vec![slave(1, true), slave(0, false)]);
        let text = observed.to_text();
        let parsed = ObservedTopology::parse(&text).expect("parse");
        assert_eq!(parsed, observed);
        assert!(text.contains("\"protocol\": \"stc-ethercat-observed/1\""));
        assert_eq!(parsed.slaves[0].position, 0);
        for text in [
            "not json",
            "{}",
            r#"{"format":1,"protocol":"other","master":0,"slaves":[]}"#,
            r#"{"format":1,"protocol":"stc-ethercat-observed/1","master":0,"slaves":[{"position":0}]}"#,
            r#"{"format":1,"protocol":"stc-ethercat-observed/1","master":0,"slaves":[{"position":0,"vendor_id":1,"product_code":2,"revision":3,"online":"yes"}]}"#,
        ] {
            let error = ObservedTopology::parse(text).expect_err(text);
            assert_eq!(error.code, "STC-E-IO-ETH-009", "{text}");
        }
    }

    #[test]
    fn inspecting_a_position_reports_configured_and_observed_facts() {
        let observed = ObservedTopology::new(0, vec![slave(0, true)]);
        let inspection = inspect_position(0, &observed, Some(&topology()));
        assert_eq!(inspection.result, CheckResult::Match);
        assert!(inspection.configured.is_some());
        assert_eq!(inspection.checks.len(), 5);
        let json = inspection.to_json().render();
        assert!(json.contains("\"configured\""));
        assert!(json.contains("\"observed\""));

        let unconfigured = inspect_position(4, &observed, Some(&topology()));
        assert_eq!(unconfigured.result, CheckResult::NotApplicable);
        assert!(unconfigured.observed.is_none());
        assert!(unconfigured.configured.is_none());

        let absent = inspect_position(1, &observed, None);
        assert_eq!(absent.result, CheckResult::NotApplicable);
    }

    #[test]
    fn check_results_rank_from_clean_to_different() {
        assert!(CheckResult::Match.is_clean());
        assert!(CheckResult::NotApplicable.is_clean());
        assert!(!CheckResult::Unverified.is_clean());
        assert!(!CheckResult::Different.is_clean());
        assert_eq!(
            CheckResult::Match.worst(CheckResult::Unverified),
            CheckResult::Unverified
        );
        assert_eq!(
            CheckResult::Different.worst(CheckResult::Unverified),
            CheckResult::Different
        );
        assert_eq!(
            CheckResult::NotApplicable.worst(CheckResult::Match),
            CheckResult::Match
        );
        assert_eq!(CheckResult::Different.as_str(), "different");
        assert_eq!(CheckResult::Unverified.as_str(), "unverified");
        assert_eq!(CheckResult::NotApplicable.as_str(), "not applicable");
    }
}
