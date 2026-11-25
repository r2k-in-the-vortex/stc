//! Host-neutral tests for the WL-7 phase 1 engineering API.
//!
//! The tests exercise the typed API directly; no test asserts on terminal text
//! or depends on a CLI, Linux, a shim library or a running EtherCAT master
//! (D-259, D-270, D-279).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use stc_runtime::ethercat::abi::{Capabilities, Capability, ResultCategory};
use stc_runtime::ethercat::api::EngineeringService;
use stc_runtime::ethercat::diagnostics::codes;
use stc_runtime::ethercat::esi::catalog::EsiLibrary;
use stc_runtime::ethercat::esi::model::PdoDirection;
use stc_runtime::ethercat::esi::reader::{Limits, parse_bytes, parse_bytes_for_fuzz, parse_file};
use stc_runtime::ethercat::mock::MockEngineeringHost;

fn fixture(parts: &[&str]) -> PathBuf {
    let mut path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/esi");
    for part in parts {
        path.push(part);
    }
    path
}

fn temp_library(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("stc-esi-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp library");
    dir
}

#[test]
fn library_lists_every_accepted_device_in_deterministic_order() {
    let library = EsiLibrary::load(&fixture(&["library"])).expect("library");
    let entries = library.entries();
    assert_eq!(entries.len(), 3, "one entry per accepted device");

    let mut sources: Vec<&Path> = entries.iter().map(|entry| entry.source.as_path()).collect();
    let sorted = {
        let mut clone = sources.clone();
        clone.sort();
        clone
    };
    assert_eq!(sources, sorted, "entries are ordered by source path");
    sources.dedup();

    // The ESI 1.1 file is reported as a per-file problem and does not hide the
    // rest of the library.
    assert_eq!(library.problems().len(), 1);
    assert_eq!(library.problems()[0].code, codes::ESI_VERSION_UNSUPPORTED);
    assert!(
        library.problems()[0]
            .source
            .as_deref()
            .expect("problem source")
            .ends_with("broken.xml")
    );

    // Loading again is deterministic.
    let again = EsiLibrary::load(&fixture(&["library"])).expect("library");
    assert_eq!(library.entries(), again.entries());
    assert_eq!(library.documents().len(), again.documents().len());

    let search = library.search("diewu");
    assert_eq!(search.len(), 2);
    assert_eq!(library.search("#x00003052").len(), 2);
    assert!(library.search("no-such-device").is_empty());
    assert!(library.search("   ").is_empty());
}

#[test]
fn filesystem_creation_order_does_not_change_resolution() {
    let directory = temp_library("order");
    std::fs::copy(
        fixture(&["library", "diewu_16io_rev2.xml"]),
        directory.join("a_rev2.xml"),
    )
    .expect("copy");
    std::fs::copy(
        fixture(&["library", "diewu_16io.xml"]),
        directory.join("z_rev1.xml"),
    )
    .expect("copy");

    let library = EsiLibrary::load(&directory).expect("library");
    let entries = library.entries();
    assert_eq!(entries.len(), 2);
    assert!(entries[0].source.ends_with("a_rev2.xml"));
    assert!(entries[1].source.ends_with("z_rev1.xml"));

    let resolved = library
        .resolve(0x0000_07d8, 0x0000_3052, Some(0x0011_0000))
        .expect("pinned revision");
    assert!(resolved.entry.source.ends_with("z_rev1.xml"));
    assert!(resolved.pinned);

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn resolution_is_exact_deterministic_and_reports_ambiguity() {
    let library = EsiLibrary::load(&fixture(&["library"])).expect("library");
    let pinned = library
        .resolve(0x0000_07d8, 0x0000_3052, Some(0x0011_0000))
        .expect("revision 1");
    assert!(pinned.pinned);
    assert_eq!(pinned.device.revision, 0x0011_0000);

    let unpinned = library
        .resolve(0x0000_07d8, 0x0000_3052, None)
        .expect("highest revision");
    assert!(!unpinned.pinned, "an unpinned selection is conspicuous");
    assert_eq!(unpinned.device.revision, 0x0012_0000);

    let missing = library
        .resolve(0x0000_07d8, 0x0000_3052, Some(0x0099_9999))
        .expect_err("unknown revision");
    assert_eq!(missing.code, codes::ESI_IDENTITY_NOT_FOUND);
    assert_eq!(missing.vendor_id, Some(0x0000_07d8));
    assert_eq!(missing.product_code, Some(0x0000_3052));
    assert!(missing.expected.is_some());

    let ambiguous = EsiLibrary::load(&fixture(&["ambiguous"])).expect("library");
    let error = ambiguous
        .resolve(0x0000_1111, 0x0000_2222, Some(1))
        .expect_err("ambiguous identity");
    assert_eq!(error.code, codes::ESI_IDENTITY_AMBIGUOUS);
    let observed = error.observed.expect("observed sources");
    assert!(observed.contains("a.xml"));
    assert!(observed.contains("b.xml"));
}

#[test]
fn inspection_exposes_typed_capabilities_without_cli_text() {
    let service = EngineeringService::open(&fixture(&["library"])).expect("service");
    let list = service.list().expect("list");
    assert_eq!(list.len(), 3);
    assert!(list.iter().all(|summary| summary.digest.len() == 64));

    let diewu = list
        .iter()
        .find(|summary| summary.product_code == 0x0000_3052 && summary.revision == 0x0011_0000)
        .expect("rev 1 summary");
    // The API contract is typed fields, not formatted text.
    assert_eq!(diewu.vendor_id, 0x0000_07d8);
    assert_eq!(diewu.type_name, "DIEWU 16I/16O");
    let summary_json = diewu.to_json().render();
    stc_got::json::parse(&summary_json).expect("summary JSON is canonical");

    let inspection = service
        .inspect_identity(0x0000_07d8, 0x0000_3052, Some(0x0011_0000))
        .expect("inspection");
    assert_eq!(inspection.devices.len(), 1);
    assert!(inspection.pinned);
    let device = &inspection.devices[0];
    assert_eq!(device.identity(), (0x0000_07d8, 0x0000_3052, 0x0011_0000));
    assert_eq!(device.sync_managers.len(), 3);
    assert_eq!(device.rx_pdos.len(), 1);
    assert_eq!(device.tx_pdos.len(), 2);
    assert_eq!(device.pdos().count(), 3);
    assert_eq!(device.tx_pdos[0].direction, PdoDirection::Tx);
    assert_eq!(device.tx_pdos[0].direction.channel_direction(), "input");
    assert_eq!(device.rx_pdos[0].direction.channel_direction(), "output");
    assert!(device.tx_pdos[0].fixed && device.tx_pdos[0].mandatory);
    assert_eq!(device.tx_pdos[1].exclude, vec![0x1a02]);
    assert!(!device.tx_pdos[1].mandatory);
    let dc = device.dc.as_ref().expect("DC alternatives");
    assert_eq!(
        dc.op_modes.len(),
        2,
        "alternatives are exposed, not selected"
    );
    assert_eq!(dc.op_modes[0].name, "DC Sync0");
    assert_eq!(dc.op_modes[0].sync0_factor, Some(1));
    assert_eq!(dc.op_modes[1].assign_activate, 0x0700);
    assert_eq!(dc.op_modes[1].sync1_shift, Some(50));
    assert!(device.mailbox.sdo_info && device.mailbox.pdo_assign && device.mailbox.pdo_config);
    assert!(!device.mailbox.complete_access);
    assert_eq!(device.dictionary.len(), 3);
    assert_eq!(device.dictionary[0].index, 0x1018);
    assert_eq!(device.dictionary[1].subindex, 1);
    assert_eq!(device.dictionary[1].access.as_deref(), Some("ro"));

    let inspection_json = inspection.to_json().render();
    let parsed = stc_got::json::parse(&inspection_json).expect("inspection JSON");
    assert!(parsed.render().contains("\"rx_pdos\""));
    assert!(!inspection_json.contains("STC-"));
}

#[test]
fn unmodelled_constructs_are_reported_with_review_provenance() {
    let service = EngineeringService::open(&fixture(&["library"])).expect("service");
    let inspection = service
        .inspect_identity(2, 2, None)
        .expect("vendor-noise device");
    assert!(!inspection.pinned);
    assert_eq!(inspection.vendor_name, "Vendor & Noise GmbH");
    assert_eq!(inspection.unsupported.len(), 1);
    assert!(inspection.unsupported[0].contains("VendorFlag"));
    assert!(inspection.unsupported[0].contains("TxPdo"));
}

#[test]
fn explicit_source_selection_is_confined_to_the_library_root() {
    let service = EngineeringService::open(&fixture(&["library"])).expect("service");
    let inside = service
        .inspect_source(&fixture(&["library", "diewu_16io.xml"]))
        .expect("inside the root");
    assert_eq!(inside.devices.len(), 1);

    let outside = service
        .inspect_source(&fixture(&["invalid", "doctype.xml"]))
        .expect_err("outside the root");
    assert_eq!(outside.code, codes::ESI_SOURCE_UNREADABLE);
    assert!(
        outside
            .observed
            .as_deref()
            .unwrap_or_default()
            .contains("outside")
    );

    // A standalone service has no library root to confine against.
    let standalone = EngineeringService::standalone();
    let error = standalone
        .inspect_source(&fixture(&["invalid", "doctype.xml"]))
        .expect_err("rejected construct");
    assert_eq!(error.code, codes::ESI_CONSTRUCT_UNSUPPORTED);
    let latin = standalone
        .inspect_source(&fixture(&["invalid", "latin1.xml"]))
        .expect("ISO-8859-1 source");
    assert_eq!(latin.vendor_name, "B\u{fc}cher Antriebe");
    assert_eq!(
        parse_file(&fixture(&["invalid", "latin1.xml"]))
            .expect("latin-1 file")
            .vendor
            .name,
        "B\u{fc}cher Antriebe"
    );
}

#[test]
fn importer_emits_every_introduced_code() {
    let mut emitted: BTreeSet<&'static str> = BTreeSet::new();

    let missing = std::env::temp_dir().join(format!("stc-esi-missing-{}", std::process::id()));
    emitted.insert(
        EsiLibrary::load(&missing)
            .expect_err("missing library")
            .code,
    );
    emitted.insert(
        parse_file(&fixture(&["invalid", "version_1_0.xml"]))
            .expect_err("version")
            .code,
    );
    emitted.insert(
        parse_file(&fixture(&["invalid", "doctype.xml"]))
            .expect_err("doctype")
            .code,
    );
    emitted.insert(
        parse_file(&fixture(&["invalid", "malformed.xml"]))
            .expect_err("malformed")
            .code,
    );
    let limits = Limits {
        max_elements: 2,
        ..Limits::default()
    };
    emitted.insert(
        stc_runtime::ethercat::esi::reader::parse_bytes_with(
            Path::new("fixture"),
            &std::fs::read(fixture(&["library", "diewu_16io.xml"])).expect("read"),
            &limits,
        )
        .expect_err("limit")
        .code,
    );
    let ambiguous = EsiLibrary::load(&fixture(&["ambiguous"])).expect("library");
    emitted.insert(
        ambiguous
            .resolve(0x0000_1111, 0x0000_2222, None)
            .expect_err("ambiguous")
            .code,
    );
    emitted.insert(ambiguous.resolve(1, 1, None).expect_err("not found").code);

    let expected: BTreeSet<&'static str> = [
        codes::ESI_SOURCE_UNREADABLE,
        codes::ESI_VERSION_UNSUPPORTED,
        codes::ESI_CONSTRUCT_UNSUPPORTED,
        codes::ESI_MALFORMED,
        codes::ESI_LIMIT_EXCEEDED,
        codes::ESI_IDENTITY_AMBIGUOUS,
        codes::ESI_IDENTITY_NOT_FOUND,
    ]
    .into_iter()
    .collect();
    assert_eq!(emitted, expected);
    // `STC-E-ESI-008`/`009` stay reserved and are never emitted.
    assert!(!emitted.contains("STC-E-ESI-008"));
    assert!(!emitted.contains("STC-E-ESI-009"));
}

#[test]
fn abi_2_contract_and_mock_cover_unavailable_capabilities_on_every_host() {
    let mut service = EngineeringService::standalone();
    assert_eq!(service.capabilities(), Capabilities::all());
    assert!(service.supports(Capability::Identity));

    let mut host = MockEngineeringHost::complete();
    host.set_capability(Capability::SdoInformation, false);
    host.fail(Capability::PdoReadBack, ResultCategory::RemapRefused);
    service.set_host(host);
    assert!(!service.supports(Capability::SdoInformation));
    let unavailable = service.host().require(Capability::SdoInformation, Some(1));
    assert_eq!(unavailable.category, ResultCategory::Unavailable);
    assert!(!unavailable.available);
    let refused = service.host().require(Capability::PdoReadBack, Some(1));
    assert_eq!(refused.category, ResultCategory::RemapRefused);
    assert!(!format!("{refused:?}").contains("STC-"));
    assert!(
        service
            .capabilities()
            .to_json()
            .render()
            .contains("sdo-information")
    );
}

#[test]
fn arbitrary_input_never_panics_the_importer() {
    for bytes in [
        b"".as_slice(),
        b"<".as_slice(),
        b"<EtherCATInfo".as_slice(),
        b"\x00\xFF\xFE\x01".as_slice(),
        b"<?xml version=\"1.0\"?><EtherCATInfo Version=\"1.2\"><Device/></EtherCATInfo>".as_slice(),
    ] {
        parse_bytes_for_fuzz(bytes);
    }
    let fixture_bytes = std::fs::read(fixture(&["library", "diewu_16io.xml"])).expect("read");
    for length in [1, 17, 128, fixture_bytes.len() / 2] {
        parse_bytes_for_fuzz(&fixture_bytes[..length]);
    }
    assert!(
        parse_bytes(
            Path::new("truncated"),
            &fixture_bytes[..fixture_bytes.len() / 3]
        )
        .is_err()
    );
}
