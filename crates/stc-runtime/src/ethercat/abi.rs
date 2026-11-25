//! Shim ABI 2 capability and structured-result contract (D-279).
//!
//! Phase 1 fixes the Rust-side contract only. The loader in
//! [`crate::io_ethercat`] still performs the ABI 1 handshake until Phase 7
//! replaces it; nothing in this module changes runtime behavior yet.
//!
//! The shim reports **facts and capability flags only**. It never returns a
//! diagnostic code: the Rust backend computes `match`/`unverified`/`stale` and
//! maps results to the stable codes in [`crate::ethercat::diagnostics`]
//! (D-279, D-280).

use stc_got::json::Json;

/// Versioned shim ABI name expected by the ABI 2 handshake.
pub const ABI_NAME: &str = "stc-io-ethercat/2";

/// Numeric ABI version carried by the handshake.
pub const ABI_VERSION: u32 = 2;

/// Capabilities an ABI 2 shim reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Position-authoritative SII identity and discovery.
    pub identity: bool,
    /// Native PDO assignment/mapping configuration.
    pub pdo_configuration: bool,
    /// PDO assignment/mapping read-back.
    pub pdo_read_back: bool,
    /// SDO upload (read).
    pub sdo_upload: bool,
    /// SDO download (write).
    pub sdo_download: bool,
    /// CoE SDO Information services.
    pub sdo_information: bool,
    /// Pre-activation startup-SDO queuing.
    pub startup_sdo_queue: bool,
    /// Distributed-clock read-back.
    pub dc_read_back: bool,
    /// Per-device domain and working-counter status.
    pub per_device_domain: bool,
}

/// One capability flag, used by the host-neutral mock and by capability
/// checks that must name the missing capability in a diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    /// Identity and discovery.
    Identity,
    /// Native PDO configuration.
    PdoConfiguration,
    /// PDO read-back.
    PdoReadBack,
    /// SDO upload.
    SdoUpload,
    /// SDO download.
    SdoDownload,
    /// SDO Information.
    SdoInformation,
    /// Startup-SDO queuing.
    StartupSdoQueue,
    /// DC read-back.
    DcReadBack,
    /// Per-device domain.
    PerDeviceDomain,
}

impl Capability {
    /// Stable spelling used in structured output and diagnostics.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::PdoConfiguration => "pdo-configuration",
            Self::PdoReadBack => "pdo-read-back",
            Self::SdoUpload => "sdo-upload",
            Self::SdoDownload => "sdo-download",
            Self::SdoInformation => "sdo-information",
            Self::StartupSdoQueue => "startup-sdo-queue",
            Self::DcReadBack => "dc-read-back",
            Self::PerDeviceDomain => "per-device-domain",
        }
    }
}

impl Capabilities {
    /// Every capability enabled (a complete ABI 2 shim).
    #[must_use]
    pub fn all() -> Self {
        Self {
            identity: true,
            pdo_configuration: true,
            pdo_read_back: true,
            sdo_upload: true,
            sdo_download: true,
            sdo_information: true,
            startup_sdo_queue: true,
            dc_read_back: true,
            per_device_domain: true,
        }
    }

    /// No capability enabled (an ABI 1-equivalent handshake).
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Reports one capability.
    #[must_use]
    pub fn supports(self, capability: Capability) -> bool {
        match capability {
            Capability::Identity => self.identity,
            Capability::PdoConfiguration => self.pdo_configuration,
            Capability::PdoReadBack => self.pdo_read_back,
            Capability::SdoUpload => self.sdo_upload,
            Capability::SdoDownload => self.sdo_download,
            Capability::SdoInformation => self.sdo_information,
            Capability::StartupSdoQueue => self.startup_sdo_queue,
            Capability::DcReadBack => self.dc_read_back,
            Capability::PerDeviceDomain => self.per_device_domain,
        }
    }

    /// Enables or disables one capability.
    pub fn set(&mut self, capability: Capability, enabled: bool) {
        match capability {
            Capability::Identity => self.identity = enabled,
            Capability::PdoConfiguration => self.pdo_configuration = enabled,
            Capability::PdoReadBack => self.pdo_read_back = enabled,
            Capability::SdoUpload => self.sdo_upload = enabled,
            Capability::SdoDownload => self.sdo_download = enabled,
            Capability::SdoInformation => self.sdo_information = enabled,
            Capability::StartupSdoQueue => self.startup_sdo_queue = enabled,
            Capability::DcReadBack => self.dc_read_back = enabled,
            Capability::PerDeviceDomain => self.per_device_domain = enabled,
        }
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(self) -> Json {
        let flags = [
            Capability::Identity,
            Capability::PdoConfiguration,
            Capability::PdoReadBack,
            Capability::SdoUpload,
            Capability::SdoDownload,
            Capability::SdoInformation,
            Capability::StartupSdoQueue,
            Capability::DcReadBack,
            Capability::PerDeviceDomain,
        ];
        Json::object(flags.into_iter().map(|capability| {
            (
                capability.as_str().to_owned(),
                Json::Bool(self.supports(capability)),
            )
        }))
    }
}

/// Category of a structured shim result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultCategory {
    /// The operation succeeded and the requested facts are present.
    Ok,
    /// The shim or device does not provide the capability.
    Unavailable,
    /// The device returned a CoE abort code.
    Abort,
    /// The transfer timed out.
    Timeout,
    /// The device state refused the operation.
    StateRefused,
    /// The live identity disagrees with the configured identity.
    IdentityMismatch,
    /// The live device refused the requested remap.
    RemapRefused,
    /// Another owner holds the master.
    OwnershipConflict,
    /// The master is not active.
    NotActive,
}

impl ResultCategory {
    /// Stable spelling used in structured output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Unavailable => "unavailable",
            Self::Abort => "abort",
            Self::Timeout => "timeout",
            Self::StateRefused => "state-refused",
            Self::IdentityMismatch => "identity-mismatch",
            Self::RemapRefused => "remap-refused",
            Self::OwnershipConflict => "ownership-conflict",
            Self::NotActive => "not-active",
        }
    }
}

/// Structured shim result.
///
/// The struct deliberately has no code field: results carry facts and an
/// availability flag, and the Rust layer owns diagnostics (D-279, D-280).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationResult {
    /// Result category.
    pub category: ResultCategory,
    /// Configured machine slot the result concerns.
    pub position: Option<u16>,
    /// Object index, for SDO and PDO operations.
    pub index: Option<u16>,
    /// Object subindex, for SDO operations.
    pub subindex: Option<u8>,
    /// CoE abort code, when the device returned one.
    pub abort_code: Option<u32>,
    /// AL status code reported by the device.
    pub al_status: Option<u16>,
    /// Current EtherCAT AL state.
    pub al_state: Option<u8>,
    /// Whether the requested facts were available.
    pub available: bool,
}

impl OperationResult {
    /// A successful result for a position.
    #[must_use]
    pub fn ok(position: Option<u16>) -> Self {
        Self {
            category: ResultCategory::Ok,
            position,
            index: None,
            subindex: None,
            abort_code: None,
            al_status: None,
            al_state: None,
            available: true,
        }
    }

    /// An unavailable result: the shim or device cannot provide the facts.
    #[must_use]
    pub fn unavailable(position: Option<u16>) -> Self {
        Self {
            category: ResultCategory::Unavailable,
            position,
            index: None,
            subindex: None,
            abort_code: None,
            al_status: None,
            al_state: None,
            available: false,
        }
    }

    /// A failed result with an explicit category.
    #[must_use]
    pub fn failed(position: Option<u16>, category: ResultCategory) -> Self {
        Self {
            category,
            position,
            index: None,
            subindex: None,
            abort_code: None,
            al_status: None,
            al_state: None,
            available: false,
        }
    }
}

/// Identity facts reported by the shim for one position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlaveIdentity {
    /// Configured machine slot.
    pub position: u16,
    /// SII vendor id.
    pub vendor_id: u32,
    /// SII product code.
    pub product_code: u32,
    /// SII revision number.
    pub revision: u32,
    /// SII serial number, when readable.
    pub serial: Option<u32>,
    /// Station alias, when readable and verified against the position.
    pub alias: Option<u16>,
    /// Current AL state.
    pub al_state: Option<u8>,
    /// Whether the slave currently responds.
    pub online: bool,
}

impl SlaveIdentity {
    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("position".into(), Json::unsigned(u64::from(self.position))),
            (
                "vendor_id".into(),
                Json::unsigned(u64::from(self.vendor_id)),
            ),
            (
                "product_code".into(),
                Json::unsigned(u64::from(self.product_code)),
            ),
            ("revision".into(), Json::unsigned(u64::from(self.revision))),
            (
                "serial".into(),
                Json::optional(self.serial.map(|value| Json::unsigned(u64::from(value)))),
            ),
            (
                "alias".into(),
                Json::optional(self.alias.map(|value| Json::unsigned(u64::from(value)))),
            ),
            (
                "al_state".into(),
                Json::optional(self.al_state.map(|value| Json::unsigned(u64::from(value)))),
            ),
            ("online".into(), Json::Bool(self.online)),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_identifies_abi_2() {
        assert_eq!(ABI_NAME, "stc-io-ethercat/2");
        assert_eq!(ABI_VERSION, 2);
    }

    #[test]
    fn capability_sets_are_addressable_by_name() {
        let mut capabilities = Capabilities::all();
        assert!(capabilities.supports(Capability::SdoInformation));
        capabilities.set(Capability::SdoInformation, false);
        assert!(!capabilities.supports(Capability::SdoInformation));
        assert!(!Capabilities::none().supports(Capability::Identity));
        let json = Capabilities::all().to_json().render();
        for name in [
            "identity",
            "pdo-configuration",
            "pdo-read-back",
            "sdo-upload",
            "sdo-download",
            "sdo-information",
            "startup-sdo-queue",
            "dc-read-back",
            "per-device-domain",
        ] {
            assert!(json.contains(name), "missing {name}");
        }
        assert_eq!(Capability::DcReadBack.as_str(), "dc-read-back");
    }

    #[test]
    fn results_carry_facts_and_categories_but_never_codes() {
        let ok = OperationResult::ok(Some(3));
        assert!(ok.available);
        assert_eq!(ok.category.as_str(), "ok");

        let unavailable = OperationResult::unavailable(Some(1));
        assert!(!unavailable.available);
        assert_eq!(unavailable.category, ResultCategory::Unavailable);

        let failed = OperationResult::failed(Some(2), ResultCategory::Abort);
        assert!(!failed.available);
        for category in [
            ResultCategory::Ok,
            ResultCategory::Unavailable,
            ResultCategory::Abort,
            ResultCategory::Timeout,
            ResultCategory::StateRefused,
            ResultCategory::IdentityMismatch,
            ResultCategory::RemapRefused,
            ResultCategory::OwnershipConflict,
            ResultCategory::NotActive,
        ] {
            assert!(!category.as_str().is_empty());
            assert!(!category.as_str().contains("STC"));
        }
        // The structured contract never carries a diagnostic code; only the
        // Rust layer maps results to codes.
        assert!(!format!("{ok:?}").contains("STC-"));
        assert!(!format!("{failed:?}").contains("STC-"));
    }

    #[test]
    fn slave_identity_renders_structured_facts() {
        let identity = SlaveIdentity {
            position: 2,
            vendor_id: 2,
            product_code: 0x07d8_3052,
            revision: 0x0011_0000,
            serial: Some(0),
            alias: Some(0),
            al_state: Some(8),
            online: true,
        };
        let json = identity.to_json().render();
        assert!(json.contains("\"position\": 2"));
        assert!(json.contains("\"vendor_id\": 2"));
        assert!(json.contains("\"online\": true"));
    }
}
