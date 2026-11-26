//! Typed engineering API used by the CLI and future TUI/GUI clients.
//!
//! The API returns structured values and typed diagnostics; no command handler
//! owns engineering logic and no result is terminal-formatted text (D-259,
//! D-270). Phase 1 covers ESI inspection, search and identity resolution;
//! Phase 2 adds topology scan, comparison and capture plus the setup-mode
//! session and master ownership arbitration. The PDO, SDO and configuration
//! operations land in later phases.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use stc_got::json::Json;

use super::abi::{Capabilities, Capability, OperationResult, ResultCategory};
use super::config::{ConfigurationReport, EngineeringConfiguration};
use super::diagnostics::{EngineeringDiagnostic, codes};
use super::esi::catalog::{EsiLibrary, LibraryEntry, ResolvedDevice};
use super::esi::model::{DeviceDescription, device_capabilities_json};
use super::esi::reader::{Limits, parse_file_with};
use super::mock::MockEngineeringHost;
use super::pdo::{LivePdoState, PdoCapabilities, PdoSelection, remap_refused};
use super::setup::{SetupSession, SetupStatus};
use super::topology::{
    self, CaptureOptions, ObservedTopology, SlaveInspection, Topology, TopologyComparison,
};

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

    /// The managed master index the host reports.
    #[must_use]
    pub fn master(&self) -> u16 {
        self.host.master()
    }

    /// Scans the bus and returns the observed topology (D-277).
    ///
    /// Inspection needs no setup session: an idle master may be read without
    /// taking ownership (D-271). A master owned by a runtime or by another
    /// engineering session is an ownership conflict and no read is attempted.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-043` when the host cannot report identity and
    /// `STC-E-IO-ETH-040` for an ownership conflict that `setup` does not name.
    pub fn scan(
        &self,
        setup: Option<&SetupSession>,
    ) -> Result<ObservedTopology, EngineeringDiagnostic> {
        self.require_host(Capability::Identity, "topology.scan")?;
        self.ensure_inspectable(setup, "topology.scan")?;
        Ok(ObservedTopology::new(
            self.host.master(),
            self.host.slaves().to_vec(),
        ))
    }

    /// Compares the desired topology with an observation (D-278).
    ///
    /// # Errors
    ///
    /// Returns the desired topology's validation diagnostic, or
    /// `STC-E-IO-ETH-009` when the observation belongs to another master.
    pub fn compare(
        &self,
        topology: &Topology,
        observed: &ObservedTopology,
    ) -> Result<TopologyComparison, EngineeringDiagnostic> {
        topology::compare(topology, observed)
    }

    /// Captures an observation into a new desired-topology draft (D-277).
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-009` for an empty or non-dense observation,
    /// `STC-E-IO-ETH-010` for an unreadable identity and
    /// `STC-E-IO-ETH-015` for a slave whose declared instance data cannot be
    /// read.
    pub fn capture(
        &self,
        base: Option<&Topology>,
        observed: &ObservedTopology,
        options: CaptureOptions,
    ) -> Result<Topology, EngineeringDiagnostic> {
        topology::capture(base, observed, options)
    }

    /// Inspects one position against the desired topology (D-277).
    ///
    /// # Errors
    ///
    /// Returns the scan diagnostics: `STC-E-IO-ETH-043` without the identity
    /// capability and `STC-E-IO-ETH-040` for an ownership conflict.
    pub fn slave_inspect(
        &self,
        position: u16,
        topology: Option<&Topology>,
        setup: Option<&SetupSession>,
    ) -> Result<SlaveInspection, EngineeringDiagnostic> {
        let observed = self.scan(setup)?;
        Ok(topology::inspect_position(position, &observed, topology))
    }

    /// Projects the declared PDO/sync-manager alternatives of one identity.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-ESI-001` without a library, `STC-E-ESI-006` for an
    /// ambiguous identity and `STC-E-ESI-007` when nothing matches.
    pub fn pdo_capabilities(
        &self,
        vendor_id: u32,
        product_code: u32,
        revision: Option<u32>,
    ) -> Result<PdoCapabilities, EngineeringDiagnostic> {
        let library = self
            .library
            .as_ref()
            .ok_or_else(|| self.no_library("pdo.capabilities"))?;
        let ResolvedDevice { device, .. } = library.resolve(vendor_id, product_code, revision)?;
        Ok(PdoCapabilities::from_device(&device))
    }

    /// Reads back the live PDO assignment/mapping of one position.
    ///
    /// Read-only inspection needs no setup session, but a master owned by a
    /// runtime or another session is an ownership conflict (D-271). A host or
    /// device that cannot provide read-back yields an empty observation, which
    /// comparison reports `unverified`, never a match (D-278).
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-043` without the `pdo-read-back` capability and
    /// `STC-E-IO-ETH-040` for an ownership conflict.
    pub fn slave_pdos(
        &self,
        position: u16,
        setup: Option<&SetupSession>,
    ) -> Result<LivePdoState, EngineeringDiagnostic> {
        self.require_host(Capability::PdoReadBack, "slave.pdos")?;
        self.ensure_inspectable(setup, "slave.pdos")?;
        Ok(self
            .host
            .live_pdos(position)
            .cloned()
            .unwrap_or_else(|| LivePdoState::new(position)))
    }

    /// Applies a declared-entry remap in setup mode (D-260, D-269).
    ///
    /// A hardware-changing operation requires an active setup session that
    /// holds the master; a remap the live device refuses is
    /// `STC-E-IO-ETH-022`.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-041` outside setup mode, `STC-E-IO-ETH-040` when
    /// another owner holds the master, `STC-E-IO-ETH-043` without the
    /// `pdo-configuration` capability and `STC-E-IO-ETH-022` for a live
    /// refusal.
    pub fn apply_pdo_configuration(
        &mut self,
        device: &str,
        position: u16,
        selection: &PdoSelection,
        setup: Option<&SetupSession>,
    ) -> Result<OperationResult, EngineeringDiagnostic> {
        let Some(setup) = setup else {
            return Err(EngineeringDiagnostic::new(
                codes::WRITE_OUTSIDE_SETUP,
                "pdo.apply",
                "enter setup mode with `stc ethercat setup enter` before changing device state",
            )
            .with_device(device)
            .with_position(position)
            .with_expected("an active setup-mode session")
            .with_observed("no setup session"));
        };
        if self.host.owner().map(str::to_owned).as_deref() != Some(setup.session.as_str()) {
            let owner = self.host.owner().unwrap_or("no owner");
            return Err(ownership_conflict("pdo.apply", self.host.master(), owner));
        }
        self.require_host(Capability::PdoConfiguration, "pdo.apply")?;
        let result = self.host.apply_pdos(position, selection);
        if result.category == ResultCategory::Ok {
            return Ok(result);
        }
        Err(remap_refused(
            device,
            position,
            "the selected PDO assignment/mapping",
            &format!("the live device reported `{}`", result.category.as_str()),
        ))
    }

    /// Validates the desired configuration offline (D-269, D-273, D-275).
    ///
    /// Each device is resolved against the loaded ESI library when possible;
    /// otherwise its persisted constraint facts are validated without the ESI.
    /// `bound` maps a device name to the channel names the project binds for
    /// it, and `runtime_cycle_ns` resolves declared DC factors.
    #[must_use]
    pub fn validate_configuration(
        &self,
        configuration: &EngineeringConfiguration,
        bound: &BTreeMap<String, Vec<String>>,
        runtime_cycle_ns: u64,
    ) -> ConfigurationReport {
        configuration.validate(
            |device| {
                self.library.as_ref().and_then(|library| {
                    library
                        .resolve(
                            device.identity.vendor_id,
                            device.identity.product_code,
                            Some(device.identity.revision),
                        )
                        .ok()
                        .map(|resolved| PdoCapabilities::from_device(&resolved.device))
                })
            },
            bound,
            runtime_cycle_ns,
        )
    }

    /// Enters setup mode and records the session (D-260).
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-044` when a session record is already active,
    /// `STC-E-IO-ETH-045` when the record is unreadable and needs recovery, and
    /// `STC-E-IO-ETH-040` when another owner holds the master.
    pub fn setup_enter(
        &mut self,
        path: &Path,
        master: u16,
        now_unix: u64,
    ) -> Result<SetupStatus, EngineeringDiagnostic> {
        if let Some(existing) = SetupSession::read(path, "setup.enter")? {
            return Err(EngineeringDiagnostic::new(
                codes::SETUP_SESSION_ACTIVE,
                "setup.enter",
                "leave the active session with `stc ethercat setup leave`",
            )
            .with_expected("no active setup session")
            .with_observed(format!(
                "session `{}` entered by pid {}",
                existing.session, existing.pid
            )));
        }
        if SetupSession::exists(path) {
            return Err(EngineeringDiagnostic::new(
                codes::SETUP_SESSION_UNREADABLE,
                "setup.enter",
                "run `stc ethercat setup leave` to clear the session record, then enter again",
            )
            .with_expected("a readable setup session descriptor")
            .with_observed("the descriptor is unreadable")
            .with_source(path.display().to_string()));
        }
        if let Some(owner) = self.host.owner() {
            return Err(ownership_conflict("setup.enter", self.host.master(), owner));
        }
        let session = SetupSession::new(std::process::id(), master, now_unix);
        if self.host.claim(&session.session).category != ResultCategory::Ok {
            let owner = self.host.owner().unwrap_or("another owner").to_owned();
            return Err(ownership_conflict("setup.enter", master, &owner));
        }
        session.write(path, "setup.enter")?;
        Ok(self.setup_status_for(Some(session), false))
    }

    /// Leaves setup mode and releases the recorded session (D-260).
    ///
    /// Leaving is idempotent and always clears the record, so an interrupted
    /// session is recovered rather than assumed to be held.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-045` when the record cannot be removed.
    pub fn setup_leave(&mut self, path: &Path) -> Result<SetupStatus, EngineeringDiagnostic> {
        let mut recovered = false;
        let session = match SetupSession::read(path, "setup.leave") {
            Ok(session) => session,
            Err(_) => {
                recovered = true;
                SetupSession::read_lossy(path)
            }
        };
        if let Some(session) = &session {
            self.host.release(&session.session);
        }
        SetupSession::remove(path, "setup.leave")?;
        Ok(self.setup_status_for(None, recovered))
    }

    /// Reports the setup-mode state without changing anything.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-045` when the session record is unreadable.
    pub fn setup_status(&self, path: &Path) -> Result<SetupStatus, EngineeringDiagnostic> {
        let session = SetupSession::read(path, "setup.status")?;
        Ok(self.setup_status_for(session, false))
    }

    fn setup_status_for(&self, session: Option<SetupSession>, recovered: bool) -> SetupStatus {
        let owner = self.host.owner().map(str::to_owned);
        let master_held = match (&session, &owner) {
            (Some(session), Some(owner)) => Some(*owner == session.session),
            (None, Some(_)) => Some(false),
            _ => None,
        };
        let conflict = match (&session, &owner) {
            (Some(session), Some(owner)) if *owner != session.session => Some(owner.clone()),
            (None, Some(owner)) => Some(owner.clone()),
            _ => None,
        };
        SetupStatus {
            session,
            master_owner: owner,
            master_held,
            conflict,
            recovered,
        }
    }

    fn require_host(
        &self,
        capability: Capability,
        operation: &'static str,
    ) -> Result<(), EngineeringDiagnostic> {
        if self.host.capabilities().supports(capability) {
            return Ok(());
        }
        Err(EngineeringDiagnostic::new(
            codes::CAPABILITY_UNAVAILABLE,
            operation,
            "run on a host with the ABI 2 shim installed (D-279)",
        )
        .with_capability(capability.as_str())
        .with_expected(format!("the `{}` capability", capability.as_str()))
        .with_observed("the engineering host does not provide it"))
    }

    fn ensure_inspectable(
        &self,
        setup: Option<&SetupSession>,
        operation: &'static str,
    ) -> Result<(), EngineeringDiagnostic> {
        let Some(owner) = self.host.owner() else {
            return Ok(());
        };
        if setup.map(|session| session.session.as_str()) == Some(owner) {
            return Ok(());
        }
        Err(ownership_conflict(operation, self.host.master(), owner))
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

fn ownership_conflict(operation: &'static str, master: u16, owner: &str) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::OWNERSHIP_CONFLICT,
        operation,
        "stop the owning runtime or leave the other engineering session; setup mode never stops a running PLC",
    )
    .with_expected("an idle master or this setup session")
    .with_observed(format!("master {master} is owned by `{owner}`"))
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

    fn session_path(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("stc-ethercat-api-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp directory");
        let path = directory.join("setup.json");
        let _ = std::fs::remove_file(&path);
        path
    }

    fn slave(position: u16) -> crate::ethercat::abi::SlaveIdentity {
        crate::ethercat::abi::SlaveIdentity {
            position,
            vendor_id: 2,
            product_code: 3,
            revision: 1,
            serial: None,
            alias: None,
            al_state: Some(8),
            online: true,
        }
    }

    #[test]
    fn a_scan_needs_the_identity_capability_and_a_free_master() {
        let mut service = EngineeringService::standalone();
        service.set_host(MockEngineeringHost::default());
        let error = service.scan(None).expect_err("no capability");
        assert_eq!(error.code, "STC-E-IO-ETH-043");
        assert_eq!(error.capability.as_deref(), Some("identity"));

        let mut host = MockEngineeringHost::complete();
        host.insert_slave(slave(0));
        host.claim("runtime");
        service.set_host(host);
        let error = service.scan(None).expect_err("conflict");
        assert_eq!(error.code, "STC-E-IO-ETH-040");
        assert!(error.observed.expect("observed").contains("runtime"));

        let setup = SetupSession::new(1, 0, 5);
        let observed = service.scan(Some(&setup)).expect_err("other session");
        assert_eq!(observed.code, "STC-E-IO-ETH-040");

        let mut host = MockEngineeringHost::complete();
        host.insert_slave(slave(0));
        host.claim(&setup.session);
        service.set_host(host);
        let observed = service.scan(Some(&setup)).expect("scan");
        assert_eq!(observed.slaves.len(), 1);
        assert_eq!(service.master(), 0);
    }

    #[test]
    fn setup_mode_is_entered_left_and_recovered() {
        let path = session_path("lifecycle");
        let mut service = EngineeringService::standalone();
        assert!(!service.setup_status(&path).expect("status").active());

        let entered = service.setup_enter(&path, 0, 1_700_000_000).expect("enter");
        assert!(entered.active());
        assert_eq!(entered.master_held, Some(true));
        assert!(entered.conflict.is_none());
        assert!(SetupSession::exists(&path));

        let error = service
            .setup_enter(&path, 0, 1_700_000_001)
            .expect_err("second");
        assert_eq!(error.code, "STC-E-IO-ETH-044");
        assert!(error.observed.expect("observed").contains("pid"));

        let status = service.setup_status(&path).expect("status");
        assert!(status.active());
        assert_eq!(status.master_held, Some(true));

        let left = service.setup_leave(&path).expect("leave");
        assert!(!left.active());
        assert!(!left.recovered);
        assert!(!SetupSession::exists(&path));

        let idempotent = service.setup_leave(&path).expect("idempotent leave");
        assert!(!idempotent.active());

        service.setup_enter(&path, 0, 2).expect("re-enter");
        service.setup_leave(&path).expect("final leave");
    }

    #[test]
    fn a_corrupt_session_record_is_reported_then_recovered() {
        let path = session_path("corrupt");
        std::fs::write(&path, "{not json").expect("write");
        let mut service = EngineeringService::standalone();
        assert_eq!(
            service.setup_status(&path).expect_err("corrupt").code,
            "STC-E-IO-ETH-045"
        );
        assert_eq!(
            service.setup_enter(&path, 0, 1).expect_err("corrupt").code,
            "STC-E-IO-ETH-045"
        );
        let recovered = service.setup_leave(&path).expect("recover");
        assert!(recovered.recovered);
        assert!(!SetupSession::exists(&path));
        service
            .setup_enter(&path, 0, 3)
            .expect("enter after recovery");
        service.setup_leave(&path).expect("leave");
    }

    #[test]
    fn entering_setup_mode_is_rejected_while_a_runtime_owns_the_master() {
        let path = session_path("owned");
        let mut host = MockEngineeringHost::complete();
        host.claim("stc-run");
        let mut service = EngineeringService::standalone();
        service.set_host(host);
        let error = service.setup_enter(&path, 0, 1).expect_err("owned");
        assert_eq!(error.code, "STC-E-IO-ETH-040");
        assert!(error.observed.expect("observed").contains("stc-run"));
        assert!(!SetupSession::exists(&path));
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
