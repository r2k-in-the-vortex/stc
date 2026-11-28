//! WL-7 engineering diagnostics (D-280).
//!
//! Codes are a stable public contract. The ESI importer owns the
//! `STC-E-ESI-*` family; every code introduced here has an emitting test.
//! Unused numbers in a family stay reserved: a new need takes the next number
//! in the owning family rather than redefining an existing code.
//!
//! The payload is structured so that a future TUI/GUI client never parses
//! human text for control flow (D-259, D-280). [`EngineeringDiagnostic::code`]
//! and the typed fields are the contract; [`EngineeringDiagnostic::message`]
//! is presentation.

use stc_got::error::Error;
use stc_got::json::Json;

/// Stable diagnostic codes owned by the ESI importer.
pub mod codes {
    /// The ESI source is missing, unreadable, or outside the project root.
    pub const ESI_SOURCE_UNREADABLE: &str = "STC-E-ESI-001";
    /// The document root or `EtherCATInfo` version is not supported.
    pub const ESI_VERSION_UNSUPPORTED: &str = "STC-E-ESI-002";
    /// The document uses a construct the accepted subset does not model.
    pub const ESI_CONSTRUCT_UNSUPPORTED: &str = "STC-E-ESI-003";
    /// The document is not well-formed XML or a required element is missing.
    pub const ESI_MALFORMED: &str = "STC-E-ESI-004";
    /// A configured resource limit was exceeded.
    pub const ESI_LIMIT_EXCEEDED: &str = "STC-E-ESI-005";
    /// One identity/revision selects more than one differing description.
    pub const ESI_IDENTITY_AMBIGUOUS: &str = "STC-E-ESI-006";
    /// No description in the library declares the requested identity.
    pub const ESI_IDENTITY_NOT_FOUND: &str = "STC-E-ESI-007";
    // `STC-E-ESI-008`-`009` are reserved for later importer diagnostics.

    /// Pre-existing identity/slave code, extended by D-280 to declared
    /// serial and alias mismatches detected during engineering.
    pub const ETH_SLAVE: &str = "STC-E-IO-ETH-003";
    /// A configured position is missing, duplicated or not dense.
    pub const TOPOLOGY_POSITION: &str = "STC-E-IO-ETH-009";
    /// A configured identity is not exact (missing, wildcard or zero).
    pub const TOPOLOGY_IDENTITY: &str = "STC-E-IO-ETH-010";
    /// A logical device name is empty or duplicated.
    pub const TOPOLOGY_NAME: &str = "STC-E-IO-ETH-011";
    /// The bus carries a slave the project does not configure.
    pub const TOPOLOGY_EXTRA_SLAVE: &str = "STC-E-IO-ETH-012";
    // `STC-E-IO-ETH-013` is reserved for a missing DC reference (Phase 7).
    /// A missing slave shifts the chain, so downstream positions are affected.
    pub const TOPOLOGY_DOWNSTREAM_SHIFT: &str = "STC-E-IO-ETH-014";
    /// Observed identity or declared instance data cannot be read for capture.
    pub const TOPOLOGY_OBSERVED_UNREADABLE: &str = "STC-E-IO-ETH-015";

    /// A mode bundle is invalid, ambiguous or internally inconsistent.
    pub const MODE_INVALID: &str = "STC-E-IO-ETH-020";
    /// A PDO selection violates a declared constraint or remaps outside the
    /// ESI-declared entry set.
    pub const PDO_SELECTION: &str = "STC-E-IO-ETH-021";
    /// The live device refused the requested PDO remap.
    pub const PDO_REMAP_REFUSED: &str = "STC-E-IO-ETH-022";
    /// A DC selection is missing, ambiguous, not declared or unresolvable.
    pub const DC_SELECTION: &str = "STC-E-IO-ETH-023";
    /// A channel exposure is invalid for the selected effective entries.
    pub const CHANNEL_EXPOSURE: &str = "STC-E-IO-ETH-024";
    // `STC-E-IO-ETH-025`-`029` are reserved for later engineering-configuration
    // diagnostics in this family.

    /// An object is neither ESI-declared nor reported by SDO Information.
    pub const SDO_OBJECT_UNAVAILABLE: &str = "STC-E-IO-ETH-030";
    /// The device returned a CoE abort code for the SDO transfer.
    pub const SDO_ABORT: &str = "STC-E-IO-ETH-031";
    /// The SDO transfer timed out.
    pub const SDO_TIMEOUT: &str = "STC-E-IO-ETH-032";
    /// The device state or the master state refused the SDO operation.
    pub const SDO_STATE: &str = "STC-E-IO-ETH-033";
    /// The object's access rights deny the requested read or write.
    pub const SDO_ACCESS_DENIED: &str = "STC-E-IO-ETH-034";
    /// A typed value, transfer length or read-back disagrees with the object.
    pub const SDO_VALUE: &str = "STC-E-IO-ETH-035";
    /// A read or required read-back is unavailable on this host or device.
    pub const SDO_READ_BACK_UNAVAILABLE: &str = "STC-E-IO-ETH-036";
    // `STC-E-IO-ETH-037`-`039` are reserved for later SDO diagnostics.

    /// The connected device or host cannot provide SDO Information; the
    /// ESI-only dictionary view stays usable (D-272).
    pub const SDO_INFO_UNAVAILABLE: &str = "STC-W-IO-ETH-009";

    /// A runtime, application or other engineering session owns the master.
    pub const OWNERSHIP_CONFLICT: &str = "STC-E-IO-ETH-040";
    /// A hardware-changing engineering operation was attempted outside setup
    /// mode (D-260, D-271).
    pub const WRITE_OUTSIDE_SETUP: &str = "STC-E-IO-ETH-041";
    // `STC-E-IO-ETH-042` is reserved for the ABI 2 handshake (Phase 7).
    /// The engineering host does not provide a required ABI 2 capability.
    pub const CAPABILITY_UNAVAILABLE: &str = "STC-E-IO-ETH-043";
    /// A setup session is already active.
    pub const SETUP_SESSION_ACTIVE: &str = "STC-E-IO-ETH-044";
    /// The setup session descriptor is unreadable and needs recovery.
    pub const SETUP_SESSION_UNREADABLE: &str = "STC-E-IO-ETH-045";
}

/// Severity derived from the code family prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// The requested operation is rejected.
    Error,
    /// The operation completed with a reported caveat.
    Warning,
}

impl Severity {
    /// Stable lower-case spelling used in structured output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

/// A structured engineering diagnostic.
///
/// The fields mirror the D-280 payload: code, severity, device/position,
/// operation, expected/observed values, missing capability and remedy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineeringDiagnostic {
    /// Stable diagnostic code, for example `STC-E-ESI-002`.
    pub code: &'static str,
    /// Operation that rejected the input, for example `esi.import`.
    pub operation: &'static str,
    /// ESI source path or library root, when the diagnostic concerns one.
    pub source: Option<String>,
    /// Device name or type, when the diagnostic concerns one.
    pub device: Option<String>,
    /// Configured machine slot, when the diagnostic concerns one.
    pub position: Option<u16>,
    /// Identity fields the requested description was expected to carry.
    pub vendor_id: Option<u32>,
    /// Product code the requested description was expected to carry.
    pub product_code: Option<u32>,
    /// Revision the requested description was expected to carry.
    pub revision: Option<u32>,
    /// Object index the operation addressed, for SDO diagnostics.
    pub index: Option<u16>,
    /// Object subindex the operation addressed, for SDO diagnostics.
    pub subindex: Option<u8>,
    /// CoE abort code the device returned, when it returned one.
    pub abort_code: Option<u32>,
    /// EtherCAT AL state observed at the failure.
    pub al_state: Option<u8>,
    /// What the configuration or caller required.
    pub expected: Option<String>,
    /// What the source or device actually declared.
    pub observed: Option<String>,
    /// Missing shim/device capability, when applicable.
    pub capability: Option<String>,
    /// What the engineer can do about it.
    pub remedy: &'static str,
}

impl EngineeringDiagnostic {
    /// Creates a diagnostic with code, operation and remedy.
    #[must_use]
    pub fn new(code: &'static str, operation: &'static str, remedy: &'static str) -> Self {
        Self {
            code,
            operation,
            source: None,
            device: None,
            position: None,
            vendor_id: None,
            product_code: None,
            revision: None,
            index: None,
            subindex: None,
            abort_code: None,
            al_state: None,
            expected: None,
            observed: None,
            capability: None,
            remedy,
        }
    }

    /// Records the ESI source path.
    #[must_use]
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Records the device name or type.
    #[must_use]
    pub fn with_device(mut self, device: impl Into<String>) -> Self {
        self.device = Some(device.into());
        self
    }

    /// Records the configured machine slot.
    #[must_use]
    pub fn with_position(mut self, position: u16) -> Self {
        self.position = Some(position);
        self
    }

    /// Records the identity the caller asked for.
    #[must_use]
    pub fn with_identity(
        mut self,
        vendor_id: u32,
        product_code: u32,
        revision: Option<u32>,
    ) -> Self {
        self.vendor_id = Some(vendor_id);
        self.product_code = Some(product_code);
        self.revision = revision;
        self
    }

    /// Records the object index and subindex the operation addressed.
    #[must_use]
    pub fn with_object(mut self, index: u16, subindex: u8) -> Self {
        self.index = Some(index);
        self.subindex = Some(subindex);
        self
    }

    /// Records the CoE abort code the device returned.
    #[must_use]
    pub fn with_abort(mut self, abort_code: u32) -> Self {
        self.abort_code = Some(abort_code);
        self
    }

    /// Records the AL state observed at the failure.
    #[must_use]
    pub fn with_al_state(mut self, al_state: u8) -> Self {
        self.al_state = Some(al_state);
        self
    }

    /// Records the expected value.
    #[must_use]
    pub fn with_expected(mut self, expected: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    /// Records the observed value.
    #[must_use]
    pub fn with_observed(mut self, observed: impl Into<String>) -> Self {
        self.observed = Some(observed.into());
        self
    }

    /// Records a missing capability.
    #[must_use]
    pub fn with_capability(mut self, capability: impl Into<String>) -> Self {
        self.capability = Some(capability.into());
        self
    }

    /// Severity derived from the code family prefix.
    #[must_use]
    pub fn severity(&self) -> Severity {
        if self.code.starts_with("STC-W-") {
            Severity::Warning
        } else {
            Severity::Error
        }
    }

    /// One-line human summary; never parsed for control flow.
    #[must_use]
    pub fn message(&self) -> String {
        let mut text = format!("{}: {} failed", self.code, self.operation);
        if let Some(source) = &self.source {
            text.push_str(&format!(" for `{source}`"));
        }
        if let Some(device) = &self.device {
            text.push_str(&format!(" on device `{device}`"));
        }
        if let Some(index) = self.index {
            match self.subindex {
                Some(subindex) => text.push_str(&format!(" object 0x{index:04x}:{subindex}")),
                None => text.push_str(&format!(" object 0x{index:04x}")),
            }
        }
        if let Some(abort_code) = self.abort_code {
            text.push_str(&format!(" abort 0x{abort_code:08x}"));
        }
        if let Some(al_state) = self.al_state {
            text.push_str(&format!(" al_state {al_state}"));
        }
        if let Some(expected) = &self.expected
            && let Some(observed) = &self.observed
        {
            text.push_str(&format!(" (expected {expected}, observed {observed})"));
        } else if let Some(observed) = &self.observed {
            text.push_str(&format!(" ({observed})"));
        } else if let Some(expected) = &self.expected {
            text.push_str(&format!(" ({expected})"));
        }
        text.push_str(&format!("; remedy: {}", self.remedy));
        text
    }

    /// Structured projection for the engineering API and CLI JSON output.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("code".into(), Json::string(self.code)),
            ("severity".into(), Json::string(self.severity().as_str())),
            ("operation".into(), Json::string(self.operation)),
            (
                "source".into(),
                Json::optional(
                    self.source
                        .as_ref()
                        .map(|value| Json::string(value.as_str())),
                ),
            ),
            (
                "device".into(),
                Json::optional(
                    self.device
                        .as_ref()
                        .map(|value| Json::string(value.as_str())),
                ),
            ),
            (
                "position".into(),
                Json::optional(self.position.map(|value| Json::unsigned(u64::from(value)))),
            ),
            (
                "vendor_id".into(),
                Json::optional(self.vendor_id.map(|value| Json::unsigned(u64::from(value)))),
            ),
            (
                "product_code".into(),
                Json::optional(
                    self.product_code
                        .map(|value| Json::unsigned(u64::from(value))),
                ),
            ),
            (
                "revision".into(),
                Json::optional(self.revision.map(|value| Json::unsigned(u64::from(value)))),
            ),
            (
                "index".into(),
                Json::optional(self.index.map(|value| Json::unsigned(u64::from(value)))),
            ),
            (
                "subindex".into(),
                Json::optional(self.subindex.map(|value| Json::unsigned(u64::from(value)))),
            ),
            (
                "abort_code".into(),
                Json::optional(
                    self.abort_code
                        .map(|value| Json::unsigned(u64::from(value))),
                ),
            ),
            (
                "al_state".into(),
                Json::optional(self.al_state.map(|value| Json::unsigned(u64::from(value)))),
            ),
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
            (
                "capability".into(),
                Json::optional(
                    self.capability
                        .as_ref()
                        .map(|value| Json::string(value.as_str())),
                ),
            ),
            ("remedy".into(), Json::string(self.remedy)),
        ])
    }

    /// Converts the diagnostic into a typed runtime error.
    #[must_use]
    pub fn into_error(self) -> Error {
        Error::new(self.code, self.message())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_follows_the_code_family() {
        let error = EngineeringDiagnostic::new(codes::ESI_MALFORMED, "esi.import", "fix it");
        assert_eq!(error.severity(), Severity::Error);
        assert_eq!(error.severity().as_str(), "error");
        assert_eq!(Severity::Warning.as_str(), "warning");
    }

    #[test]
    fn structured_payload_carries_every_documented_field() {
        let diagnostic =
            EngineeringDiagnostic::new(codes::ESI_VERSION_UNSUPPORTED, "esi.import", "use 1.2")
                .with_source("esi/device.xml")
                .with_device("EL1008")
                .with_position(2)
                .with_identity(2, 0x0000_0002, Some(1))
                .with_expected("1.2")
                .with_observed("1.0")
                .with_capability("ESI 1.2");
        let json = diagnostic.to_json().render();
        for field in [
            "\"code\": \"STC-E-ESI-002\"",
            "\"severity\": \"error\"",
            "\"operation\": \"esi.import\"",
            "\"source\": \"esi/device.xml\"",
            "\"device\": \"EL1008\"",
            "\"position\": 2",
            "\"vendor_id\": 2",
            "\"product_code\": 2",
            "\"revision\": 1",
            "\"expected\": \"1.2\"",
            "\"observed\": \"1.0\"",
            "\"capability\": \"ESI 1.2\"",
            "\"remedy\": \"use 1.2\"",
        ] {
            assert!(json.contains(field), "missing {field} in {json}");
        }
        let message = diagnostic.message();
        assert!(message.starts_with("STC-E-ESI-002: esi.import failed"));
        assert!(message.contains("expected 1.2, observed 1.0"));
        assert!(message.contains("remedy: use 1.2"));
    }

    #[test]
    fn optional_payload_fields_render_as_null() {
        let json =
            EngineeringDiagnostic::new(codes::ESI_SOURCE_UNREADABLE, "esi.list", "pass --esi-dir")
                .to_json()
                .render();
        assert!(json.contains("\"source\": null"));
        assert!(json.contains("\"position\": null"));
        let message =
            EngineeringDiagnostic::new(codes::ESI_LIMIT_EXCEEDED, "esi.import", "raise limits")
                .message();
        assert!(!message.contains("expected"));
        assert!(
            EngineeringDiagnostic::new(codes::ESI_MALFORMED, "esi.import", "fix it")
                .with_observed("byte 4")
                .message()
                .contains("(byte 4)")
        );
        assert!(
            EngineeringDiagnostic::new(codes::ESI_MALFORMED, "esi.import", "fix it")
                .with_expected("a device")
                .message()
                .contains("(a device)")
        );
    }

    #[test]
    fn converts_to_a_typed_error_with_the_stable_code() {
        let error =
            EngineeringDiagnostic::new(codes::ESI_IDENTITY_NOT_FOUND, "esi.resolve", "import it")
                .with_identity(2, 7, Some(1))
                .into_error();
        assert_eq!(error.code, "STC-E-ESI-007");
        assert!(error.message.contains("STC-E-ESI-007"));
    }
}
