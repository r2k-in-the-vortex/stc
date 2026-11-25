//! WL-7 phase 2 topology, ownership and setup-mode API tests.
//!
//! Every test drives the typed engineering API through the host-neutral mock,
//! so the suite runs on every CI host without EtherCAT hardware or the IgH
//! shim (D-279).

use std::path::PathBuf;

use stc_runtime::ethercat::abi::SlaveIdentity;
use stc_runtime::ethercat::api::EngineeringService;
use stc_runtime::ethercat::mock::MockEngineeringHost;
use stc_runtime::ethercat::setup::SetupSession;
use stc_runtime::ethercat::topology::{
    CaptureOptions, CheckResult, ConfiguredDevice, DeviceIdentity, ObservedTopology, Topology,
};

fn workdir(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "stc-ethercat-topology-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("work directory");
    directory
}

fn configured(name: &str, position: u16) -> ConfiguredDevice {
    ConfiguredDevice {
        name: name.into(),
        position,
        identity: DeviceIdentity::new(0x0000_0b95, 0x0001_0200, 1),
        serial: None,
        alias: None,
    }
}

fn topology() -> Topology {
    Topology::with_devices(0, vec![configured("input", 0), configured("output", 1)])
}

fn observed_slave(position: u16, online: bool) -> SlaveIdentity {
    SlaveIdentity {
        position,
        vendor_id: 0x0000_0b95,
        product_code: 0x0001_0200,
        revision: 1,
        serial: Some(11),
        alias: Some(0),
        al_state: Some(8),
        online,
    }
}

fn service_with(slaves: Vec<SlaveIdentity>) -> EngineeringService {
    let mut host = MockEngineeringHost::complete();
    for slave in slaves {
        host.insert_slave(slave);
    }
    let mut service = EngineeringService::standalone();
    service.set_host(host);
    service
}

#[test]
fn a_topology_is_authored_and_validated_without_hardware() {
    let directory = workdir("author");
    let path = directory.join("topology.json");
    let mut draft = topology();
    draft.devices[1].serial = Some(4711);
    std::fs::write(&path, draft.to_text()).expect("write draft");

    let text = std::fs::read_to_string(&path).expect("read draft");
    let parsed = Topology::parse(&text).expect("parse draft");
    assert_eq!(parsed, draft);
    parsed.validate().expect("valid");
    assert_eq!(
        parsed.devices[0].identity.describe(),
        "#x00000b95/#x00010200/#x00000001"
    );
    assert!(
        parsed
            .to_json()
            .render()
            .contains("\"protocol\": \"stc-ethercat-topology/1\"")
    );

    // A gap is rejected offline, before any hardware exists.
    let mut gap = draft.clone();
    gap.devices[1].position = 2;
    assert_eq!(gap.validate().expect_err("gap").code, "STC-E-IO-ETH-009");
}

#[test]
fn a_scan_reports_observed_facts_in_position_order() {
    let mut host = MockEngineeringHost::complete();
    host.set_master(1);
    host.insert_slave(observed_slave(1, true));
    host.insert_slave(observed_slave(0, true));
    let mut service = EngineeringService::standalone();
    service.set_host(host);

    let observed = service.scan(None).expect("scan");
    assert_eq!(observed.master, 1);
    assert_eq!(observed.slaves.len(), 2);
    assert_eq!(observed.slaves[0].position, 0);
    assert_eq!(observed.slaves[1].position, 1);
    assert!(observed.slaves[0].online);
    assert_eq!(observed.slaves[0].serial, Some(11));
    assert!(
        observed
            .to_text()
            .contains("\"protocol\": \"stc-ethercat-observed/1\"")
    );
}

#[test]
fn inspection_is_refused_while_a_runtime_owns_the_master() {
    let mut host = MockEngineeringHost::complete();
    host.insert_slave(observed_slave(0, true));
    host.claim("stc-run");
    let mut service = EngineeringService::standalone();
    service.set_host(host);

    let error = service.scan(None).expect_err("owned");
    assert_eq!(error.code, "STC-E-IO-ETH-040");
    assert!(error.observed.expect("observed").contains("stc-run"));
    assert_eq!(
        service
            .slave_inspect(0, None, None)
            .expect_err("owned")
            .code,
        "STC-E-IO-ETH-040"
    );

    // The recorded setup session may inspect through its own reservation.
    let setup = SetupSession::new(1, 0, 9);
    let mut host = MockEngineeringHost::complete();
    host.insert_slave(observed_slave(0, true));
    host.claim(&setup.session);
    service.set_host(host);
    assert!(service.scan(Some(&setup)).is_ok());
}

#[test]
fn a_scan_without_the_identity_capability_fails_closed() {
    let mut host = MockEngineeringHost::complete();
    host.set_capability(stc_runtime::ethercat::abi::Capability::Identity, false);
    let mut service = EngineeringService::standalone();
    service.set_host(host);
    let error = service.scan(None).expect_err("capability");
    assert_eq!(error.code, "STC-E-IO-ETH-043");
    assert_eq!(error.capability.as_deref(), Some("identity"));
}

#[test]
fn comparison_reports_each_position_and_never_rewrites_the_project() {
    let service = service_with(vec![observed_slave(0, true), observed_slave(1, true)]);
    let project = topology();
    let before = project.clone();
    let comparison = service
        .compare(&project, &service.scan(None).expect("scan"))
        .expect("compare");
    assert!(comparison.is_match());
    assert_eq!(comparison.positions.len(), 2);
    assert_eq!(project, before);

    let mut mismatched = observed_slave(1, true);
    mismatched.revision = 2;
    let comparison = service
        .compare(
            &project,
            &ObservedTopology::new(0, vec![observed_slave(0, true), mismatched]),
        )
        .expect("compare");
    assert!(!comparison.is_match());
    assert_eq!(comparison.positions[0].result, CheckResult::Match);
    assert_eq!(comparison.positions[1].result, CheckResult::Different);
    assert_eq!(comparison.diagnostics[0].code, "STC-E-IO-ETH-003");
    assert_eq!(comparison.diagnostics[0].position, Some(1));
    assert_eq!(project, before);
}

#[test]
fn comparison_reports_extras_and_shifts_without_renumbering() {
    let service = service_with(Vec::new());
    let project = topology();

    let extra = service
        .compare(
            &project,
            &ObservedTopology::new(
                0,
                vec![
                    observed_slave(0, true),
                    observed_slave(1, true),
                    observed_slave(2, true),
                ],
            ),
        )
        .expect("compare");
    assert_eq!(extra.extra.len(), 1);
    assert_eq!(extra.extra[0].position, 2);
    assert_eq!(extra.diagnostics[0].code, "STC-E-IO-ETH-012");

    let shifted = service
        .compare(
            &project,
            &ObservedTopology::new(0, vec![observed_slave(1, true)]),
        )
        .expect("compare");
    assert_eq!(shifted.positions[0].result, CheckResult::Different);
    assert_eq!(shifted.diagnostics[0].code, "STC-E-IO-ETH-014");
}

#[test]
fn capture_records_identity_only_by_default() {
    let service = service_with(Vec::new());
    let observed = ObservedTopology::new(0, vec![observed_slave(0, true), observed_slave(1, true)]);

    let draft = service
        .capture(None, &observed, CaptureOptions::default())
        .expect("capture");
    assert_eq!(draft.devices[0].serial, None);
    assert_eq!(draft.devices[0].alias, None);
    assert_eq!(draft.devices[0].name, "slave0");

    let pinned = service
        .capture(
            None,
            &observed,
            CaptureOptions {
                pin_serial: true,
                pin_alias: true,
            },
        )
        .expect("capture");
    assert_eq!(pinned.devices[0].serial, Some(11));
    assert_eq!(pinned.devices[0].alias, Some(0));

    // An existing project keeps its logical names; the base is untouched.
    let base = topology();
    let before = base.clone();
    let recaptured = service
        .capture(Some(&base), &observed, CaptureOptions::default())
        .expect("capture");
    assert_eq!(recaptured.devices[0].name, "input");
    assert_eq!(base, before);
}

#[test]
fn slave_inspection_reports_identity_checks_per_position() {
    let service = service_with(vec![observed_slave(0, true)]);
    let project = topology();
    let inspection = service
        .slave_inspect(0, Some(&project), None)
        .expect("inspect");
    assert_eq!(inspection.result, CheckResult::Match);
    assert_eq!(inspection.configured.expect("configured").name, "input");
    assert_eq!(inspection.observed.expect("observed").position, 0);
    assert_eq!(inspection.checks.len(), 5);
    assert!(inspection.diagnostics.is_empty());

    let missing = service
        .slave_inspect(1, Some(&project), None)
        .expect("inspect");
    assert_eq!(missing.result, CheckResult::Different);
    assert!(missing.observed.is_none());
    assert!(
        missing
            .to_json()
            .render()
            .contains("\"result\": \"different\"")
    );
}

#[test]
fn setup_mode_is_explicit_persistent_and_recoverable() {
    let directory = workdir("setup");
    let path = directory.join("session.json");
    let mut service = EngineeringService::standalone();

    let entered = service.setup_enter(&path, 0, 1_700_000_000).expect("enter");
    assert!(entered.active());
    assert_eq!(entered.master_held, Some(true));
    assert!(SetupSession::exists(&path));
    assert!(entered.to_json().render().contains("\"active\": true"));

    // A second engineering session is rejected rather than sharing state.
    let second = service
        .setup_enter(&path, 0, 1_700_000_001)
        .expect_err("second session");
    assert_eq!(second.code, "STC-E-IO-ETH-044");
    assert!(second.observed.expect("observed").contains("pid"));

    // Leaving releases the record and is idempotent.
    let left = service.setup_leave(&path).expect("leave");
    assert!(!left.active());
    assert!(!SetupSession::exists(&path));
    assert!(!service.setup_leave(&path).expect("idempotent").active());

    // An interrupted session leaves a record that `leave` clears.
    std::fs::write(&path, "not a descriptor").expect("corrupt record");
    assert_eq!(
        service.setup_status(&path).expect_err("corrupt").code,
        "STC-E-IO-ETH-045"
    );
    let recovered = service.setup_leave(&path).expect("recover");
    assert!(recovered.recovered);
    assert!(!SetupSession::exists(&path));
    service
        .setup_enter(&path, 0, 2)
        .expect("enter after recovery");
    service.setup_leave(&path).expect("final leave");
}

#[test]
fn setup_entry_is_refused_while_a_runtime_owns_the_master() {
    let directory = workdir("setup-owned");
    let path = directory.join("session.json");
    let mut host = MockEngineeringHost::complete();
    host.claim("stc-serve");
    let mut service = EngineeringService::standalone();
    service.set_host(host);

    let error = service.setup_enter(&path, 0, 1).expect_err("owned");
    assert_eq!(error.code, "STC-E-IO-ETH-040");
    assert!(error.observed.expect("observed").contains("stc-serve"));
    assert!(!SetupSession::exists(&path));
    let status = service.setup_status(&path).expect("status");
    assert_eq!(status.master_held, Some(false));
    assert_eq!(status.conflict.as_deref(), Some("stc-serve"));
}
