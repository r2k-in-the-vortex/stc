//! Typed engineering API used by the CLI and future TUI/GUI clients.
//!
//! The API returns structured values and typed diagnostics; no command handler
//! owns engineering logic and no result is terminal-formatted text (D-259,
//! D-270). Phase 1 covers ESI inspection, search and identity resolution; the
//! topology, PDO, SDO and configuration operations land in later phases.

use std::path::{Path, PathBuf};

use stc_got::json::Json;

use super::abi::{Capabilities, Capability};
use super::diagnostics::EngineeringDiagnostic;
use super::esi::catalog::{EsiLibrary, LibraryEntry, ResolvedDevice};
use super::esi::model::{DeviceDescription, device_capabilities_json};
use super::esi::reader::{Limits, parse_file_with};
use super::mock::MockEngineeringHost;

/// One searchable device description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DescriptionSummary {
    /// Source file.
    pub source: PathBuf,
    /// Declaration index inside the source.
    pub device_index: usize,
    /// Vendor id.
    pub vendor_id: u32,
    /// Vendor name.
    pub vendor_name: String,
    /// Product code.
    pub product_code: u32,
    /// Revision number.
    pub revision: u32,
    /// Device name.
    pub name: String,
    /// Device type name.
    pub type_name: String,
    /// Content digest of the accepted description.
    pub digest: String,
}

impl DescriptionSummary {
    fn from_entry(entry: &LibraryEntry) -> Self {
        Self {
            source: entry.source.clone(),
            device_index: entry.device_index,
            vendor_id: entry.vendor_id,
            vendor_name: entry.vendor_name.clone(),
            product_code: entry.product_code,
            revision: entry.revision,
            name: entry.name.clone(),
            type_name: entry.type_name.clone(),
            digest: entry.digest.clone(),
        }
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            (
                "source".into(),
                Json::string(self.source.display().to_string()),
            ),
            (
                "device_index".into(),
                Json::unsigned(self.device_index as u64),
            ),
            (
                "vendor_id".into(),
                Json::unsigned(u64::from(self.vendor_id)),
            ),
            (
                "vendor_name".into(),
                Json::string(self.vendor_name.as_str()),
            ),
            (
                "product_code".into(),
                Json::unsigned(u64::from(self.product_code)),
            ),
            (
                "product_code_hex".into(),
                Json::string(format!("#x{:08x}", self.product_code)),
            ),
            ("revision".into(), Json::unsigned(u64::from(self.revision))),
            ("name".into(), Json::string(self.name.as_str())),
            ("type".into(), Json::string(self.type_name.as_str())),
            ("digest".into(), Json::string(self.digest.as_str())),
        ])
    }
}

/// The inspected capabilities of one or more device descriptions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceInspection {
    /// Source file.
    pub source: PathBuf,
    /// Vendor id.
    pub vendor_id: u32,
    /// Vendor name.
    pub vendor_name: String,
    /// Inspected devices, in declaration order or the resolved single device.
    pub devices: Vec<DeviceDescription>,
    /// Constructs the accepted subset does not model.
    pub unsupported: Vec<String>,
    /// Whether the caller pinned the revision (identity inspections only).
    pub pinned: bool,
}

impl SourceInspection {
    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            (
                "source".into(),
                Json::string(self.source.display().to_string()),
            ),
            (
                "vendor_id".into(),
                Json::unsigned(u64::from(self.vendor_id)),
            ),
            (
                "vendor_name".into(),
                Json::string(self.vendor_name.as_str()),
            ),
            ("pinned".into(), Json::Bool(self.pinned)),
            (
                "devices".into(),
                Json::Array(self.devices.iter().map(device_capabilities_json).collect()),
            ),
            (
                "unsupported".into(),
                Json::Array(
                    self.unsupported
                        .iter()
                        .map(|text| Json::string(text.as_str()))
                        .collect(),
                ),
            ),
        ])
    }
}

/// The engineering service: one library plus the host boundary.
#[derive(Debug)]
pub struct EngineeringService {
    library: Option<EsiLibrary>,
    host: MockEngineeringHost,
}

impl EngineeringService {
    /// Opens an ESI library at `root`.
    ///
    /// # Errors
    ///
    /// Returns an `STC-E-ESI-*` diagnostic when the library cannot be read.
    pub fn open(root: &Path) -> Result<Self, EngineeringDiagnostic> {
        Ok(Self {
            library: Some(EsiLibrary::load(root)?),
            host: MockEngineeringHost::complete(),
        })
    }

    /// Opens an ESI library with explicit reader limits.
    ///
    /// # Errors
    ///
    /// Returns an `STC-E-ESI-*` diagnostic when the library cannot be read.
    pub fn open_with(root: &Path, limits: &Limits) -> Result<Self, EngineeringDiagnostic> {
        Ok(Self {
            library: Some(EsiLibrary::load_with(root, limits)?),
            host: MockEngineeringHost::complete(),
        })
    }

    /// Creates a service without a library, for explicit single-source
    /// inspection.
    #[must_use]
    pub fn standalone() -> Self {
        Self {
            library: None,
            host: MockEngineeringHost::complete(),
        }
    }

    /// The loaded library, when one is configured.
    #[must_use]
    pub fn library(&self) -> Option<&EsiLibrary> {
        self.library.as_ref()
    }

    /// Files the library could not accept.
    #[must_use]
    pub fn problems(&self) -> &[EngineeringDiagnostic] {
        self.library.as_ref().map_or(&[], EsiLibrary::problems)
    }

    /// The host capability/result boundary.
    #[must_use]
    pub fn host(&self) -> &MockEngineeringHost {
        &self.host
    }

    /// Replaces the host boundary, for tests and future clients.
    pub fn set_host(&mut self, host: MockEngineeringHost) {
        self.host = host;
    }

    /// Capabilities reported by the host boundary.
    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        self.host.capabilities()
    }

    /// Reports whether the host boundary supports one capability.
    #[must_use]
    pub fn supports(&self, capability: Capability) -> bool {
        self.host.capabilities().supports(capability)
    }

    /// Lists every description in the library.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-ESI-001` when no library was opened.
    pub fn list(&self) -> Result<Vec<DescriptionSummary>, EngineeringDiagnostic> {
        let library = self
            .library
            .as_ref()
            .ok_or_else(|| self.no_library("esi.list"))?;
        Ok(library
            .entries()
            .iter()
            .map(DescriptionSummary::from_entry)
            .collect())
    }

    /// Searches the library.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-ESI-001` when no library was opened.
    pub fn search(&self, query: &str) -> Result<Vec<DescriptionSummary>, EngineeringDiagnostic> {
        let library = self
            .library
            .as_ref()
            .ok_or_else(|| self.no_library("esi.search"))?;
        Ok(library
            .search(query)
            .iter()
            .map(DescriptionSummary::from_entry)
            .collect())
    }

    /// Inspects one explicit ESI source.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-ESI-*` for an unreadable, rejected or out-of-root source.
    pub fn inspect_source(&self, source: &Path) -> Result<SourceInspection, EngineeringDiagnostic> {
        let document = match &self.library {
            Some(library) => library.parse_source(source)?,
            None => parse_file_with(source, &Limits::default())?,
        };
        Ok(SourceInspection {
            source: document.source,
            vendor_id: document.vendor.id,
            vendor_name: document.vendor.name,
            devices: document.devices,
            unsupported: document.unsupported,
            pinned: false,
        })
    }

    /// Inspects the description that one exact identity resolves to.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-ESI-001` without a library, `STC-E-ESI-006` for an
    /// ambiguous identity and `STC-E-ESI-007` when nothing matches.
    pub fn inspect_identity(
        &self,
        vendor_id: u32,
        product_code: u32,
        revision: Option<u32>,
    ) -> Result<SourceInspection, EngineeringDiagnostic> {
        let library = self
            .library
            .as_ref()
            .ok_or_else(|| self.no_library("esi.inspect"))?;
        let ResolvedDevice {
            entry,
            device,
            pinned,
            unsupported,
        } = library.resolve(vendor_id, product_code, revision)?;
        Ok(SourceInspection {
            source: entry.source,
            vendor_id,
            vendor_name: entry.vendor_name,
            devices: vec![device],
            unsupported,
            pinned,
        })
    }

    fn no_library(&self, operation: &'static str) -> EngineeringDiagnostic {
        EngineeringDiagnostic::new(
            super::diagnostics::codes::ESI_SOURCE_UNREADABLE,
            operation,
            "pass an explicit ESI library directory",
        )
        .with_expected("an ESI library")
        .with_observed("no library configured")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ethercat::abi::Capability;

    #[test]
    fn a_service_without_a_library_reports_a_structured_diagnostic() {
        let service = EngineeringService::standalone();
        let error = service.list().expect_err("no library");
        assert_eq!(error.code, "STC-E-ESI-001");
        assert_eq!(error.operation, "esi.list");
        assert!(service.problems().is_empty());
        assert!(service.library().is_none());
        assert_eq!(
            service.search("el").expect_err("no library").code,
            "STC-E-ESI-001"
        );
        assert_eq!(
            service
                .inspect_identity(2, 2, Some(1))
                .expect_err("no library")
                .code,
            "STC-E-ESI-001"
        );
    }

    #[test]
    fn host_boundary_is_exposed_as_a_typed_contract() {
        let mut service = EngineeringService::standalone();
        assert!(service.supports(Capability::SdoInformation));
        assert!(service.capabilities().identity);
        let mut host = MockEngineeringHost::new(Capabilities::none());
        host.fail(
            Capability::Identity,
            super::super::abi::ResultCategory::Timeout,
        );
        service.set_host(host);
        assert!(!service.supports(Capability::Identity));
        assert_eq!(service.capabilities(), Capabilities::none());
        assert!(!format!("{:?}", service.host()).contains("STC-"));
    }

    #[test]
    fn summaries_and_inspections_project_structured_json() {
        let summary = DescriptionSummary {
            source: PathBuf::from("esi/device.xml"),
            device_index: 0,
            vendor_id: 2,
            vendor_name: "Vendor".into(),
            product_code: 2,
            revision: 1,
            name: "Device".into(),
            type_name: "TYPE".into(),
            digest: "ab".into(),
        };
        let json = summary.to_json().render();
        assert!(json.contains("\"vendor_name\": \"Vendor\""));
        assert!(json.contains("\"product_code_hex\": \"#x00000002\""));
        let inspection = SourceInspection {
            source: PathBuf::from("esi/device.xml"),
            vendor_id: 2,
            vendor_name: "Vendor".into(),
            devices: Vec::new(),
            unsupported: vec!["attribute `Foo`".into()],
            pinned: true,
        };
        let json = inspection.to_json().render();
        assert!(json.contains("\"pinned\": true"));
        assert!(json.contains("attribute `Foo`"));
        assert!(json.contains("\"devices\": []"));
    }
}
