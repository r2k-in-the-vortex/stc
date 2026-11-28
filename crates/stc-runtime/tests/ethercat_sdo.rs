//! WL-7 phase 4 object dictionary and SDO commissioning API tests.
//!
//! Every test uses the host-neutral mock, so the suite runs on every CI host
//! without EtherCAT hardware, the IgH shim or a mailbox-capable device
//! (D-279). Live bench evidence is recorded in the WL-7 plan.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use stc_runtime::ethercat::abi::{Capability, OnlineObject, ResultCategory, SlaveIdentity};
use stc_runtime::ethercat::api::EngineeringService;
use stc_runtime::ethercat::diagnostics::codes;
use stc_runtime::ethercat::mock::MockEngineeringHost;
use stc_runtime::ethercat::pdo::PdoCapabilities;
use stc_runtime::ethercat::sdo::{
    Access, MetadataSource, ObjectAddress, SdoData, SdoType, object_unavailable,
};
use stc_runtime::ethercat::setup::SetupSession;

const DIEWU: (u32, u32, u32) = (0x0000_07d8, 0x0000_3052, 0x0011_0000);

fn library_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/esi/library")
}

fn session_path(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("stc-ethercat-sdo-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("temp directory");
    let path = directory.join("setup.json");
    let _ = std::fs::remove_file(&path);
    path
}

fn online(index: u16, subindex: u8, name: &str, data_type: &str, access: &str) -> OnlineObject {
    OnlineObject {
        index,
        subindex,
        name: Some(name.into()),
        data_type: Some(data_type.into()),
        access: Some(access.into()),
        description: None,
    }
}

fn with_setup(
    name: &str,
    mut host: MockEngineeringHost,
) -> (EngineeringService, PathBuf, SetupSession) {
    host.insert_slave(SlaveIdentity {
        position: 0,
        vendor_id: DIEWU.0,
        product_code: DIEWU.1,
        revision: DIEWU.2,
        serial: None,
        alias: None,
        al_state: Some(8),
        online: true,
    });
    let path = session_path(name);
    let mut service = EngineeringService::open(&library_root()).expect("library");
    service.set_host(host);
    let status = service.setup_enter(&path, 0, 1).expect("enter setup");
    let session = status.session.expect("session");
    (service, path, session)
}

#[test]
fn an_esi_dictionary_is_available_offline_with_esi_provenance() {
    let service = EngineeringService::open(&library_root()).expect("library");
    let dictionary = service
        .object_dictionary(0, Some(DIEWU), None)
        .expect("dictionary");
    assert!(!dictionary.is_empty());
    assert!(dictionary.warnings.is_empty());
    let vendor = dictionary.object(0x1018, 1).expect("Vendor ID");
    assert_eq!(vendor.name.as_deref(), Some("Vendor ID"));
    assert_eq!(vendor.data_type.as_deref(), Some("UDINT"));
    assert_eq!(vendor.access.as_deref(), Some("ro"));
    assert_eq!(vendor.source, MetadataSource::Esi);
    assert!(!vendor.online_only);
    assert!(dictionary.object_at(0x1018).len() >= 3);
    assert!(
        dictionary
            .to_json()
            .render()
            .contains("\"source\": \"esi\"")
    );
}

#[test]
fn online_metadata_merges_by_index_and_surfaces_field_conflicts() {
    let mut host = MockEngineeringHost::complete();
    host.set_online_dictionary(
        0,
        vec![
            online(0x1018, 1, "Vendor", "UINT", "rw"),
            online(0x6000, 1, "Online only", "INT", "rw"),
        ],
    );
    let (mut service, path, session) = with_setup("merge", host);
    let dictionary = service
        .object_dictionary(0, Some(DIEWU), Some(&session))
        .expect("dictionary");
    assert!(dictionary.warnings.is_empty());

    let vendor = dictionary.object(0x1018, 1).expect("Vendor ID");
    assert_eq!(vendor.source, MetadataSource::Online);
    assert_eq!(vendor.name.as_deref(), Some("Vendor"));
    assert_eq!(vendor.data_type.as_deref(), Some("UINT"));
    assert_eq!(vendor.access.as_deref(), Some("rw"));
    assert_eq!(vendor.conflicts.len(), 3);
    assert!(vendor.conflicts.iter().any(|conflict| {
        conflict.field == "name" && conflict.esi == "Vendor ID" && conflict.online == "Vendor"
    }));

    let online_only = dictionary.object(0x6000, 1).expect("online object");
    assert!(online_only.online_only);
    assert_eq!(online_only.source, MetadataSource::Online);
    service.setup_leave(&path).expect("leave");
}

#[test]
fn unavailable_sdo_information_is_reported_and_keeps_the_esi_view() {
    let mut host = MockEngineeringHost::complete();
    host.set_capability(Capability::SdoInformation, false);
    let (mut service, path, session) = with_setup("noinfo", host);
    let dictionary = service
        .object_dictionary(0, Some(DIEWU), Some(&session))
        .expect("dictionary");
    assert!(!dictionary.is_empty());
    assert_eq!(dictionary.warnings.len(), 1);
    assert_eq!(dictionary.warnings[0].code, codes::SDO_INFO_UNAVAILABLE);
    assert!(dictionary.warnings[0].message().contains("ESI-only"));
    service.setup_leave(&path).expect("leave");
}

#[test]
fn sdo_transfers_require_setup_mode_managed_ownership_and_capabilities() {
    let mut service = EngineeringService::standalone();
    let error = service
        .sdo_read("io", ObjectAddress::new(0, 0x1018, 1), Some(DIEWU), None)
        .expect_err("no setup");
    assert_eq!(error.code, codes::WRITE_OUTSIDE_SETUP);
    assert_eq!(error.operation, "sdo.read");

    let foreign = SetupSession::new(7, 0, 3);
    let mut host = MockEngineeringHost::complete();
    host.claim("runtime");
    service.set_host(host);
    let error = service
        .sdo_read(
            "io",
            ObjectAddress::new(0, 0x1018, 1),
            Some(DIEWU),
            Some(&foreign),
        )
        .expect_err("owned");
    assert_eq!(error.code, codes::OWNERSHIP_CONFLICT);
    assert!(error.observed.expect("observed").contains("runtime"));

    let mut host = MockEngineeringHost::complete();
    host.set_capability(Capability::SdoUpload, false);
    let (mut service, path, session) = with_setup("capability", host);
    let error = service
        .sdo_read(
            "io",
            ObjectAddress::new(0, 0x1018, 1),
            Some(DIEWU),
            Some(&session),
        )
        .expect_err("capability");
    assert_eq!(error.code, codes::CAPABILITY_UNAVAILABLE);
    assert_eq!(error.capability.as_deref(), Some("sdo-upload"));
    service.setup_leave(&path).expect("leave");
}

#[test]
fn a_read_decodes_the_declared_type_and_reports_unavailable_values() {
    let mut host = MockEngineeringHost::complete();
    host.set_online_dictionary(0, vec![online(0x6061, 0, "Mode", "SINT", "rw")]);
    host.set_sdo_value(0, 0x1018, 1, 2008u32.to_le_bytes().to_vec());
    host.set_sdo_value(0, 0x6061, 0, vec![3]);
    let (service, path, session) = with_setup("read", host);

    let result = service
        .sdo_read(
            "io",
            ObjectAddress::new(0, 0x1018, 1),
            Some(DIEWU),
            Some(&session),
        )
        .expect("read identity");
    assert_eq!(result.value, SdoData::Unsigned(2008));
    assert_eq!(result.raw, 2008u32.to_le_bytes().to_vec());
    assert_eq!(result.declared_type.as_deref(), Some("UDINT"));
    assert_eq!(result.source, MetadataSource::Esi);
    assert_eq!(result.al_state, Some(8));

    let result = service
        .sdo_read(
            "io",
            ObjectAddress::new(0, 0x6061, 0),
            Some(DIEWU),
            Some(&session),
        )
        .expect("read online-only");
    assert_eq!(result.value, SdoData::Signed(3));
    assert_eq!(result.source, MetadataSource::Online);

    let error = service
        .sdo_read(
            "io",
            ObjectAddress::new(0, 0x1018, 2),
            Some(DIEWU),
            Some(&session),
        )
        .expect_err("no value");
    assert_eq!(error.code, codes::SDO_READ_BACK_UNAVAILABLE);

    let error = service
        .sdo_read(
            "io",
            ObjectAddress::new(0, 0x9999, 0),
            Some(DIEWU),
            Some(&session),
        )
        .expect_err("unknown object");
    assert_eq!(error.code, codes::SDO_OBJECT_UNAVAILABLE);

    // A value that does not match the declared width is a value failure.
    host = MockEngineeringHost::complete();
    host.set_sdo_value(0, 0x1018, 1, vec![1, 2]);
    let (mut service, path2, session2) = with_setup("width", host);
    let error = service
        .sdo_read(
            "io",
            ObjectAddress::new(0, 0x1018, 1),
            Some(DIEWU),
            Some(&session2),
        )
        .expect_err("width");
    assert_eq!(error.code, codes::SDO_VALUE);

    service.setup_leave(&path).expect("leave");
    service.setup_leave(&path2).expect("leave second");
}

#[test]
fn a_write_is_explicit_verified_and_never_persisted() {
    let mut host = MockEngineeringHost::complete();
    host.set_online_dictionary(0, vec![online(0x6061, 0, "Mode", "SINT", "rw")]);
    let (mut service, path, session) = with_setup("write", host);

    let result = service
        .sdo_write(
            "io",
            ObjectAddress::new(0, 0x6061, 0),
            &SdoData::Signed(3),
            Some(DIEWU),
            Some(&session),
        )
        .expect("write");
    assert_eq!(result.value, SdoData::Signed(3));
    assert_eq!(result.raw, vec![3]);
    assert_eq!(result.verified, Some(true));
    assert!(!result.read_back_unavailable);
    assert!(!result.persisted);
    assert_eq!(
        service.host().sdo_value(0, 0x6061, 0),
        Some(&[3u8][..]),
        "a commissioning write changes the device, not the project"
    );
    assert!(result.to_json().render().contains("\"persisted\": false"));

    // The ESI declares 0x1018:1 read-only, so a write is denied by access.
    let error = service
        .sdo_write(
            "io",
            ObjectAddress::new(0, 0x1018, 1),
            &SdoData::Unsigned(1),
            Some(DIEWU),
            Some(&session),
        )
        .expect_err("read-only");
    assert_eq!(error.code, codes::SDO_ACCESS_DENIED);
    assert_eq!(error.index, Some(0x1018));
    assert_eq!(error.subindex, Some(1));

    service.setup_leave(&path).expect("leave");
}

#[test]
fn a_write_is_unverified_when_read_back_is_unavailable_or_differs() {
    let mut host = MockEngineeringHost::complete();
    host.set_online_dictionary(0, vec![online(0x6061, 0, "Mode", "SINT", "rw")]);
    host.set_capability(Capability::SdoUpload, false);
    let (mut service, path, session) = with_setup("unverified", host);
    let result = service
        .sdo_write(
            "io",
            ObjectAddress::new(0, 0x6061, 0),
            &SdoData::Signed(3),
            Some(DIEWU),
            Some(&session),
        )
        .expect("write");
    assert_eq!(result.verified, None);
    assert!(result.read_back_unavailable);
    service.setup_leave(&path).expect("leave");

    let mut host = MockEngineeringHost::complete();
    host.set_online_dictionary(0, vec![online(0x6061, 0, "Mode", "SINT", "rw")]);
    host.set_sdo_read_back_override(0, 0x6061, 0, vec![9]);
    let (mut service, path, session) = with_setup("mismatch", host);
    let error = service
        .sdo_write(
            "io",
            ObjectAddress::new(0, 0x6061, 0),
            &SdoData::Signed(3),
            Some(DIEWU),
            Some(&session),
        )
        .expect_err("read-back mismatch");
    assert_eq!(error.code, codes::SDO_VALUE);
    assert!(error.expected.expect("expected").contains("read-back"));
    service.setup_leave(&path).expect("leave");

    let mut host = MockEngineeringHost::complete();
    host.set_online_dictionary(0, vec![online(0x6061, 0, "Mode", "SINT", "rw")]);
    let (mut service, path, session) = with_setup("type", host);
    let error = service
        .sdo_write(
            "io",
            ObjectAddress::new(0, 0x6061, 0),
            &SdoData::Unsigned(3),
            Some(DIEWU),
            Some(&session),
        )
        .expect_err("wrong type");
    assert_eq!(error.code, codes::SDO_VALUE);
    service.setup_leave(&path).expect("leave");
}

#[test]
fn an_online_only_object_is_never_pdo_mappable() {
    let mut host = MockEngineeringHost::complete();
    host.set_online_dictionary(0, vec![online(0x6000, 1, "Online only", "INT", "rw")]);
    let (mut service, path, session) = with_setup("mappable", host);
    let dictionary = service
        .object_dictionary(0, Some(DIEWU), Some(&session))
        .expect("dictionary");
    assert!(dictionary.object(0x6000, 1).expect("object").online_only);

    service.setup_leave(&path).expect("leave");
    let library_service = EngineeringService::open(&library_root()).expect("library");
    let capabilities: PdoCapabilities = library_service
        .pdo_capabilities(DIEWU.0, DIEWU.1, Some(DIEWU.2))
        .expect("capabilities");
    let mut selection = capabilities.default_selection();
    let pdo = selection.pdos.first_mut().expect("rx pdo");
    pdo.entries.push(stc_runtime::ethercat::pdo::SelectedEntry {
        index: 0x6000,
        subindex: 1,
        bit_len: 16,
        declared_in: pdo.index,
    });
    let error = selection
        .validate(&capabilities, 0)
        .expect_err("arbitrary mapping");
    assert_eq!(error.code, codes::PDO_SELECTION);
    assert!(
        error
            .observed
            .expect("observed")
            .contains("arbitrary object-dictionary")
    );
}

#[test]
fn every_introduced_sdo_code_has_an_emitting_case() {
    let mut emitted = BTreeSet::new();

    // STC-W-IO-ETH-009: SDO Information unavailable.
    let mut host = MockEngineeringHost::complete();
    host.set_capability(Capability::SdoInformation, false);
    let (mut service, path, session) = with_setup("codes-info", host);
    let dictionary = service
        .object_dictionary(0, Some(DIEWU), Some(&session))
        .expect("dictionary");
    emitted.extend(dictionary.warnings.iter().map(|warning| warning.code));
    service.setup_leave(&path).expect("leave");

    // STC-E-IO-ETH-030: no metadata at all.
    let (mut service, path, session) = with_setup("codes-missing", MockEngineeringHost::complete());
    let error = service
        .sdo_read(
            "io",
            ObjectAddress::new(0, 0x9999, 0),
            Some(DIEWU),
            Some(&session),
        )
        .expect_err("object");
    emitted.insert(error.code);
    assert_eq!(
        object_unavailable("sdo.read", "io", 0, 0x9999, 0).code,
        codes::SDO_OBJECT_UNAVAILABLE
    );
    service.setup_leave(&path).expect("leave");

    // STC-E-IO-ETH-031: abort.
    let mut host = MockEngineeringHost::complete();
    host.set_sdo_value(0, 0x1018, 1, vec![0, 0, 0, 0]);
    host.fail_sdo(0, 0x1018, 1, 0x0602_0000);
    let (mut service, path, session) = with_setup("codes-abort", host);
    let error = service
        .sdo_read(
            "io",
            ObjectAddress::new(0, 0x1018, 1),
            Some(DIEWU),
            Some(&session),
        )
        .expect_err("abort");
    assert_eq!(error.abort_code, Some(0x0602_0000));
    emitted.insert(error.code);
    service.setup_leave(&path).expect("leave");

    // STC-E-IO-ETH-032: timeout.
    let mut host = MockEngineeringHost::complete();
    host.set_sdo_value(0, 0x1018, 1, vec![0, 0, 0, 0]);
    host.fail(Capability::SdoUpload, ResultCategory::Timeout);
    let (mut service, path, session) = with_setup("codes-timeout", host);
    let error = service
        .sdo_read(
            "io",
            ObjectAddress::new(0, 0x1018, 1),
            Some(DIEWU),
            Some(&session),
        )
        .expect_err("timeout");
    emitted.insert(error.code);
    service.setup_leave(&path).expect("leave");

    // STC-E-IO-ETH-033: state refusal.
    let mut host = MockEngineeringHost::complete();
    host.set_sdo_value(0, 0x1018, 1, vec![0, 0, 0, 0]);
    host.fail(Capability::SdoUpload, ResultCategory::StateRefused);
    let (mut service, path, session) = with_setup("codes-state", host);
    let error = service
        .sdo_read(
            "io",
            ObjectAddress::new(0, 0x1018, 1),
            Some(DIEWU),
            Some(&session),
        )
        .expect_err("state");
    emitted.insert(error.code);
    service.setup_leave(&path).expect("leave");

    // STC-E-IO-ETH-034: access denied (ESI read-only object).
    let (mut service, path, session) = with_setup("codes-access", MockEngineeringHost::complete());
    let error = service
        .sdo_write(
            "io",
            ObjectAddress::new(0, 0x1018, 1),
            &SdoData::Unsigned(1),
            Some(DIEWU),
            Some(&session),
        )
        .expect_err("access");
    emitted.insert(error.code);
    service.setup_leave(&path).expect("leave");

    // STC-E-IO-ETH-035: transfer length does not match the declared type.
    let mut host = MockEngineeringHost::complete();
    host.set_sdo_value(0, 0x1018, 1, vec![1]);
    let (mut service, path, session) = with_setup("codes-value", host);
    let error = service
        .sdo_read(
            "io",
            ObjectAddress::new(0, 0x1018, 1),
            Some(DIEWU),
            Some(&session),
        )
        .expect_err("value");
    emitted.insert(error.code);
    service.setup_leave(&path).expect("leave");

    // STC-E-IO-ETH-036: the transfer returned no data.
    let (mut service, path, session) = with_setup("codes-back", MockEngineeringHost::complete());
    let error = service
        .sdo_read(
            "io",
            ObjectAddress::new(0, 0x1018, 2),
            Some(DIEWU),
            Some(&session),
        )
        .expect_err("no data");
    emitted.insert(error.code);
    service.setup_leave(&path).expect("leave");

    let expected: BTreeSet<&str> = [
        codes::SDO_INFO_UNAVAILABLE,
        codes::SDO_OBJECT_UNAVAILABLE,
        codes::SDO_ABORT,
        codes::SDO_TIMEOUT,
        codes::SDO_STATE,
        codes::SDO_ACCESS_DENIED,
        codes::SDO_VALUE,
        codes::SDO_READ_BACK_UNAVAILABLE,
    ]
    .into_iter()
    .collect();
    assert_eq!(emitted, expected);
}

#[test]
fn typed_values_and_access_helpers_are_deterministic() {
    assert_eq!(Access::parse(Some("RO")), Access::ReadOnly);
    assert_eq!(SdoType::from_declaration(Some("LREAL")), Some(SdoType::F64));
    assert_eq!(SdoType::from_declaration(Some("IDENTITY")), None);
    let bytes = SdoData::Unsigned(7)
        .encode(Some(SdoType::U16))
        .expect("encode");
    assert_eq!(
        SdoData::decode(&bytes, Some(SdoType::U16)),
        Ok(SdoData::Unsigned(7))
    );
    assert!(SdoData::parse("300", Some(SdoType::U8)).is_err());
    assert!(SdoData::parse("0x0102", Some(SdoType::Octets)).is_ok());
}
