//! WL-7 phase 3 engineering API tests for PDO selection, application and
//! configuration validation.
//!
//! Every test uses the host-neutral mock, so the suite runs on every CI host
//! without EtherCAT hardware or the IgH shim (D-279).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use stc_runtime::ethercat::abi::{Capability, ResultCategory};
use stc_runtime::ethercat::api::EngineeringService;
use stc_runtime::ethercat::config::{
    DcChoice, DeviceModeSet, EngineeringConfiguration, EsiReference, MailboxFacts, Mode,
};
use stc_runtime::ethercat::diagnostics::codes;
use stc_runtime::ethercat::mock::MockEngineeringHost;
use stc_runtime::ethercat::pdo::PdoCapabilities;
use stc_runtime::ethercat::setup::SetupSession;
use stc_runtime::ethercat::topology::DeviceIdentity;

fn library_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/esi/library")
}

fn session_path(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "stc-ethercat-pdo-api-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("temp directory");
    let path = directory.join("setup.json");
    let _ = std::fs::remove_file(&path);
    path
}

fn diewu_capabilities() -> PdoCapabilities {
    let service = EngineeringService::open(&library_root()).expect("library");
    service
        .pdo_capabilities(0x0000_07d8, 0x0000_3052, Some(0x0011_0000))
        .expect("diewu description")
}

fn diewu_configuration(capabilities: &PdoCapabilities) -> EngineeringConfiguration {
    let selection = capabilities.default_selection();
    let channels = Mode::propose_channels(&selection, Some(capabilities));
    let mode = Mode {
        name: "default".into(),
        description: Some("Digital IO".into()),
        provenance: None,
        pdos: selection,
        channels,
        startup: Vec::new(),
        dc: DcChoice::FreeRun,
    };
    EngineeringConfiguration {
        master: 0,
        devices: vec![DeviceModeSet {
            name: "io".into(),
            position: 0,
            identity: DeviceIdentity::new(
                capabilities.vendor_id,
                capabilities.product_code,
                capabilities.revision,
            ),
            esi: Some(EsiReference {
                source: "diewu_16io.xml".into(),
                digest: capabilities.digest.clone(),
            }),
            mailbox: MailboxFacts::from_mailbox(&capabilities.mailbox),
            selected: "default".into(),
            modes: vec![mode],
        }],
    }
}

#[test]
fn an_esi_description_projects_inspectable_pdo_alternatives() {
    let capabilities = diewu_capabilities();
    assert_eq!(capabilities.type_name, "DIEWU 16I/16O");
    assert_eq!(capabilities.rx_pdos.len(), 1);
    assert_eq!(capabilities.tx_pdos.len(), 2);
    assert!(capabilities.tx_pdos[0].fixed);
    assert!(capabilities.tx_pdos[0].mandatory);
    assert_eq!(capabilities.tx_pdos[1].exclude, vec![0x1a02]);
    let selection = capabilities.default_selection();
    selection
        .validate(&capabilities, 0)
        .expect("default selection");
    assert!(selection.bit_len() > 0);
    let json = capabilities.to_json().render();
    assert!(json.contains("\"index_hex\": \"0x1600\""), "{json}");
    assert!(json.contains("\"entries\""), "{json}");
}

#[test]
fn a_mode_bundle_validates_offline_against_the_resolved_esi() {
    let capabilities = diewu_capabilities();
    let configuration = diewu_configuration(&capabilities);
    assert!(
        configuration
            .to_json()
            .render()
            .contains("\"stc-ethercat-config/1\"")
    );
    let text = configuration.to_text();
    let parsed = EngineeringConfiguration::parse(&text).expect("round trip");
    assert_eq!(parsed, configuration);

    let mut bound = BTreeMap::new();
    bound.insert("io".to_owned(), vec!["Inputs".to_owned()]);
    let service = EngineeringService::open(&library_root()).expect("library");
    let report = service.validate_configuration(&configuration, &bound, 1_000_000);
    assert!(report.is_valid(), "{:?}", report.diagnostics);
    assert!(report.devices[0].esi_resolved);

    // A stale digest is reported as a re-import requirement.
    let mut stale = configuration.clone();
    stale.devices[0].esi = Some(EsiReference {
        source: "diewu_16io.xml".into(),
        digest: "stale".into(),
    });
    let report = service.validate_configuration(&stale, &bound, 1_000_000);
    assert!(!report.is_valid());
    assert_eq!(report.diagnostics[0].code, codes::MODE_INVALID);
    assert!(
        report.diagnostics[0]
            .observed
            .as_deref()
            .expect("observed")
            .contains("re-import")
    );
}

#[test]
fn applying_a_remap_requires_setup_mode_and_managed_ownership() {
    let capabilities = diewu_capabilities();
    let selection = capabilities.default_selection();
    let path = session_path("apply");
    let mut service = EngineeringService::standalone();

    let error = service
        .apply_pdo_configuration("io", 0, &selection, None)
        .expect_err("no setup");
    assert_eq!(error.code, codes::WRITE_OUTSIDE_SETUP);
    assert_eq!(error.operation, "pdo.apply");
    assert!(
        error
            .observed
            .expect("observed")
            .contains("no setup session")
    );

    let mut host = MockEngineeringHost::complete();
    host.claim("runtime");
    service.set_host(host);
    let foreign = SetupSession::new(1, 0, 5);
    let error = service
        .apply_pdo_configuration("io", 0, &selection, Some(&foreign))
        .expect_err("conflict");
    assert_eq!(error.code, codes::OWNERSHIP_CONFLICT);
    assert!(error.observed.expect("observed").contains("runtime"));

    let mut host = MockEngineeringHost::complete();
    host.set_capability(Capability::PdoConfiguration, false);
    let mut service = EngineeringService::standalone();
    service.set_host(host);
    let status = service.setup_enter(&path, 0, 1).expect("enter");
    let session = status.session.expect("session");
    let error = service
        .apply_pdo_configuration("io", 0, &selection, Some(&session))
        .expect_err("capability");
    assert_eq!(error.code, codes::CAPABILITY_UNAVAILABLE);
    assert_eq!(error.capability.as_deref(), Some("pdo-configuration"));
    service.setup_leave(&path).expect("leave");
}

#[test]
fn a_live_remap_refusal_is_reported_and_never_verified() {
    let capabilities = diewu_capabilities();
    let selection = capabilities.default_selection();
    let path = session_path("refusal");
    let mut host = MockEngineeringHost::complete();
    host.fail_remap(0);
    let mut service = EngineeringService::standalone();
    service.set_host(host);
    let status = service.setup_enter(&path, 0, 1).expect("enter");
    let session = status.session.expect("session");

    let error = service
        .apply_pdo_configuration("io", 0, &selection, Some(&session))
        .expect_err("refused");
    assert_eq!(error.code, codes::PDO_REMAP_REFUSED);
    assert_eq!(error.position, Some(0));
    assert!(error.observed.expect("observed").contains("remap-refused"));

    service.setup_leave(&path).expect("leave");
}

#[test]
fn an_applied_configuration_reads_back_as_a_match() {
    let capabilities = diewu_capabilities();
    let selection = capabilities.default_selection();
    let path = session_path("applied");
    let mut service = EngineeringService::standalone();
    let status = service.setup_enter(&path, 0, 1).expect("enter");
    let session = status.session.expect("session");
    service
        .apply_pdo_configuration("io", 0, &selection, Some(&session))
        .expect("apply");

    let live = service.slave_pdos(0, Some(&session)).expect("read back");
    let verification = selection.verify(&live);
    assert!(verification.is_match());
    assert!(!verification.is_unverified());

    // Unknown positions read back as empty, which is unverified, not a match.
    let empty = service
        .slave_pdos(7, Some(&session))
        .expect("missing read-back");
    assert!(!selection.verify(&empty).is_match());
    assert!(selection.verify(&empty).is_unverified());

    let error = service.slave_pdos(0, None).expect_err("owned by setup");
    assert_eq!(error.code, codes::OWNERSHIP_CONFLICT);
    service.setup_leave(&path).expect("leave");

    // Without the read-back capability the inspection fails closed.
    let mut host = MockEngineeringHost::complete();
    host.set_capability(Capability::PdoReadBack, false);
    let mut service = EngineeringService::standalone();
    service.set_host(host);
    let error = service.slave_pdos(0, None).expect_err("capability");
    assert_eq!(error.code, codes::CAPABILITY_UNAVAILABLE);
    assert_eq!(error.capability.as_deref(), Some("pdo-read-back"));
    assert!(!format!("{error:?}").contains("no slaves"));
}

#[test]
fn a_remap_outside_the_declared_entry_set_is_rejected() {
    let capabilities = diewu_capabilities();
    let mut selection = capabilities.default_selection();
    let pdo = selection
        .pdos
        .iter_mut()
        .find(|pdo| pdo.index == 0x1600)
        .expect("rx pdo");
    pdo.entries[0].index = 0x7001;
    let error = selection
        .validate(&capabilities, 0)
        .expect_err("undeclared entry");
    assert_eq!(error.code, codes::PDO_SELECTION);
    assert!(
        error
            .observed
            .expect("observed")
            .contains("arbitrary object-dictionary mapping")
    );
    assert_eq!(
        ResultCategory::RemapRefused.as_str(),
        "remap-refused",
        "the mock reports categories, not codes"
    );
}
