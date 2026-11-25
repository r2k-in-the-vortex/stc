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
