//! WL-7 phase 3 PDO/mode/DC/channel configuration API tests.
//!
//! The suite drives the typed engineering configuration model with synthetic
//! ESI capability facts. It runs on every CI host without EtherCAT hardware or
//! the IgH shim (D-279).

use std::collections::{BTreeMap, BTreeSet};

use stc_got::json::Json;
use stc_runtime::ethercat::config::{
    ChannelExposure, DcChoice, DcTiming, DeviceModeSet, EngineeringConfiguration, EsiReference,
    MailboxFacts, Mode, SdoValue, StartupAction, StartupOperation, StartupState,
    StartupVerification,
};
use stc_runtime::ethercat::diagnostics::codes;
use stc_runtime::ethercat::esi::model::{DcOpMode, MailboxCapabilities, PdoDirection, SyncManager};
use stc_runtime::ethercat::pdo::{
    CoeValue, CoeView, DeclaredEntry, EntryKey, LiveEntry, LivePdoAssignment, LivePdoMapping,
    LivePdoState, PdoAlternative, PdoCapabilities, is_declared_default, remap_refused,
};
use stc_runtime::ethercat::topology::{CheckResult, DeviceIdentity};

const RUNTIME_CYCLE_NS: u64 = 500_000;

fn alternative(
    direction: PdoDirection,
    index: u16,
    sm: u8,
    fixed: bool,
    mandatory: bool,
    entries: &[(u16, u8, u8)],
) -> PdoAlternative {
    PdoAlternative {
        direction,
        index,
        sm: Some(sm),
        fixed,
        mandatory,
        excluded_sm: Vec::new(),
        exclude: Vec::new(),
        name: Some(format!("pdo_{index:04x}")),
        entries: entries
            .iter()
            .map(|(entry_index, subindex, bit_len)| DeclaredEntry {
                key: EntryKey::new(*entry_index, *subindex),
                bit_len: *bit_len,
                name: Some(format!("entry_{entry_index:04x}_{subindex}")),
                data_type: Some("UINT".into()),
                declared_in: index,
                direction,
            })
            .collect(),
    }
}

fn capabilities() -> PdoCapabilities {
    PdoCapabilities {
        name: "Servo".into(),
        type_name: "SERVO".into(),
        vendor_id: 6,
        product_code: 0x101,
        revision: 1,
        digest: "digest".into(),
        sync_managers: vec![
            SyncManager {
                index: 2,
                start_address: 0x1100,
                control_byte: 0x64,
                enable: true,
                name: Some("Outputs".into()),
            },
            SyncManager {
                index: 3,
                start_address: 0x1300,
                control_byte: 0x20,
                enable: true,
                name: Some("Inputs".into()),
            },
        ],
        rx_pdos: vec![
            alternative(
                PdoDirection::Rx,
                0x1600,
                2,
                false,
                false,
                &[(0x7000, 1, 16), (0x7000, 2, 16)],
            ),
            alternative(PdoDirection::Rx, 0x1601, 2, false, false, &[(0x7000, 3, 8)]),
            alternative(PdoDirection::Rx, 0x1602, 2, true, true, &[(0x7000, 4, 8)]),
        ],
        tx_pdos: vec![
            alternative(
                PdoDirection::Tx,
                0x1a00,
                3,
                true,
                true,
                &[(0x6000, 1, 16), (0x6000, 2, 16)],
            ),
            alternative(PdoDirection::Tx, 0x1a01, 3, false, false, &[(0x6000, 3, 1)]),
        ],
        mailbox: MailboxCapabilities {
            sdo_info: true,
            pdo_assign: true,
            pdo_config: true,
            complete_access: false,
        },
        dc_op_modes: Some(vec![
            DcOpMode {
                index: 0,
                name: "DC Sync0".into(),
                description: None,
                assign_activate: 0x0300,
                sync0_cycle: 0,
                sync0_factor: Some(2),
                sync0_shift: 0,
                sync1_cycle: None,
                sync1_shift: None,
            },
            DcOpMode {
                index: 1,
                name: "DC Sync0 Sync1".into(),
                description: Some("with Sync1".into()),
                assign_activate: 0x0300,
                sync0_cycle: 500_000,
                sync0_factor: None,
                sync0_shift: 0,
                sync1_cycle: Some(2_000_000),
                sync1_shift: Some(0),
            },
        ]),
    }
}

fn timing(capabilities: &PdoCapabilities) -> DcTiming {
    DcTiming::resolve(
        &capabilities.dc_op_modes.as_ref().expect("dc")[0],
        RUNTIME_CYCLE_NS,
    )
    .expect("factor-resolved timing")
}

fn mode(capabilities: &PdoCapabilities, name: &str) -> Mode {
    let pdos = capabilities.default_selection();
    let channels = Mode::propose_channels(&pdos, Some(capabilities));
    Mode {
        name: name.into(),
        description: Some(format!("{name} mode")),
        provenance: Some("ESI op mode 0".into()),
        pdos,
        channels,
        startup: vec![StartupOperation {
            id: "mode-select".into(),
            action: StartupAction::Write,
            index: 0x6060,
            subindex: 0,
            value: SdoValue::Unsigned(1),
            state: StartupState::PreOp,
            verify: StartupVerification::Required,
            timeout_ms: 1_000,
        }],
        dc: DcChoice::OpMode(timing(capabilities)),
    }
}

fn device_set(capabilities: &PdoCapabilities, modes: Vec<Mode>, selected: &str) -> DeviceModeSet {
    DeviceModeSet {
        name: "servo".into(),
        position: 0,
        identity: DeviceIdentity::new(
            capabilities.vendor_id,
            capabilities.product_code,
            capabilities.revision,
        ),
        esi: Some(EsiReference {
            source: "esi/servo.xml".into(),
            digest: capabilities.digest.clone(),
        }),
        mailbox: MailboxFacts::from_mailbox(&capabilities.mailbox),
        selected: selected.into(),
        modes,
    }
}

fn configuration(capabilities: &PdoCapabilities, selected: &str) -> EngineeringConfiguration {
    EngineeringConfiguration {
        master: 0,
        devices: vec![device_set(
            capabilities,
            vec![mode(capabilities, selected)],
            selected,
        )],
    }
}

fn bound(names: &[&str]) -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([(
        "servo".to_owned(),
        names.iter().map(|name| (*name).to_owned()).collect(),
    )])
}

/// Rebuilds a JSON value without any object key named `key`.
fn without_key(value: &Json, key: &str) -> Json {
    match value {
        Json::Object(object) => Json::Object(
            object
                .iter()
                .filter(|(name, _)| name.as_str() != key)
                .map(|(name, value)| (name.clone(), without_key(value, key)))
                .collect(),
        ),
        Json::Array(items) => {
            Json::Array(items.iter().map(|item| without_key(item, key)).collect())
        }
        other => other.clone(),
    }
}

fn channel(
    name: &str,
    pdo: u16,
    entry: EntryKey,
    direction: PdoDirection,
    bit: u32,
    width: u32,
) -> ChannelExposure {
    ChannelExposure {
        name: name.into(),
        pdo,
        entry,
        direction,
        bit,
        width,
    }
}

fn expect_channel_error(capabilities: &PdoCapabilities, mode: Mode) {
    let error = mode
        .validate(Some(capabilities), RUNTIME_CYCLE_NS, 0, "servo")
        .expect_err("invalid channel exposure");
    assert_eq!(error.code, codes::CHANNEL_EXPOSURE);
}

#[test]
fn pdo_alternatives_and_the_declared_entry_pool_are_inspectable() {
    let capabilities = capabilities();
    let alternatives: Vec<&PdoAlternative> = capabilities.alternatives().collect();
    assert_eq!(alternatives.len(), 5);
    assert_eq!(alternatives[0].direction, PdoDirection::Rx);
    assert_eq!(alternatives[0].index, 0x1600);
    assert_eq!(alternatives[0].sm, Some(2));
    assert!(alternatives[2].fixed && alternatives[2].mandatory);
    assert!(capabilities.alternative(0x1a00, PdoDirection::Tx).is_some());
    assert!(capabilities.alternative(0x1a00, PdoDirection::Rx).is_none());

    let declared = capabilities
        .declared_entry(EntryKey::new(0x7000, 1), PdoDirection::Rx)
        .expect("declared entry");
    assert_eq!(declared.bit_len, 16);
    assert_eq!(declared.declared_in, 0x1600);
    assert!(
        capabilities
            .declared_entry(EntryKey::new(0x7000, 1), PdoDirection::Tx)
            .is_none(),
        "the declared-entry pool is direction-scoped"
    );

    let json = capabilities.to_json().render();
    assert!(json.contains("\"fixed\": true"));
    assert!(json.contains("\"excluded_sm\""));
    assert!(json.contains("\"op_modes\""));
    assert!(json.contains("\"assign_activate_hex\": \"#x0300\""));

    let default = capabilities.default_selection();
    assert_eq!(default.pdos.len(), 5);
    assert_eq!(default.bit_len(), 16 + 16 + 8 + 8 + 16 + 16 + 1);
    assert!(is_declared_default(&default, &capabilities));
    let channels = Mode::propose_channels(&default, Some(&capabilities));
    assert_eq!(channels.len(), 7);
    assert_eq!(channels[0].name, "entry_7000_1");
    assert_eq!(channels[0].bit, 0);
    assert_eq!(channels[0].width, 16);
}

#[test]
fn mode_selection_is_explicit_and_alternates_do_not_affect_the_effective_configuration() {
    let capabilities = capabilities();
    let production = mode(&capabilities, "production");
    let mut commissioning = mode(&capabilities, "commissioning");
    commissioning
        .channels
        .retain(|channel| channel.name != "entry_7000_1");
    let device = device_set(&capabilities, vec![production, commissioning], "production");
    device
        .validate(
            Some(&capabilities),
            &["entry_7000_1".into()],
            RUNTIME_CYCLE_NS,
        )
        .expect("the selected mode exposes the bound channel");

    let mut switched = device.clone();
    switched.selected = "commissioning".into();
    let error = switched
        .validate(
            Some(&capabilities),
            &["entry_7000_1".into()],
            RUNTIME_CYCLE_NS,
        )
        .expect_err("switching loses the bound channel");
    assert_eq!(error.code, codes::CHANNEL_EXPOSURE);
    assert!(
        error
            .observed
            .as_deref()
            .expect("observed")
            .contains("leaves the bound channel unexposed")
    );

    let mut missing = device.clone();
    missing.selected = "not-stored".into();
    assert_eq!(
        missing
            .validate(Some(&capabilities), &[], RUNTIME_CYCLE_NS)
            .expect_err("missing selection")
            .code,
        codes::MODE_INVALID
    );

    let mut duplicate = device.clone();
    duplicate.modes.push(mode(&capabilities, "production"));
    assert_eq!(
        duplicate
            .validate(Some(&capabilities), &[], RUNTIME_CYCLE_NS)
            .expect_err("duplicate names")
            .code,
        codes::MODE_INVALID
    );

    let config = EngineeringConfiguration {
        master: 0,
        devices: vec![device],
    };
    let report = config.validate(
        |_| Some(capabilities.clone()),
        &bound(&["entry_7000_1"]),
        RUNTIME_CYCLE_NS,
    );
    assert!(report.is_valid());
    assert_eq!(report.devices[0].selected.as_deref(), Some("production"));
    assert!(report.devices[0].esi_resolved);
    assert!(
        report
            .to_json()
            .render()
            .contains("\"selected\": \"production\"")
    );
}

#[test]
fn a_declared_entry_remap_round_trips_and_revalidates_without_the_esi() {
    let capabilities = capabilities();
    let mut remapped = mode(&capabilities, "remapped");
    let moved = {
        let source = remapped
            .pdos
            .pdos
            .iter_mut()
            .find(|pdo| pdo.index == 0x1600)
            .expect("source PDO");
        source.entries.remove(1)
    };
    remapped
        .pdos
        .pdos
        .iter_mut()
        .find(|pdo| pdo.index == 0x1601)
        .expect("target PDO")
        .entries
        .push(moved);
    remapped.channels = Mode::propose_channels(&remapped.pdos, Some(&capabilities));

    remapped
        .validate(Some(&capabilities), RUNTIME_CYCLE_NS, 0, "servo")
        .expect("declared-entry remap");
    let (assignment_changed, mapping_changed) = remapped.pdos.deviations(&capabilities);
    assert!(!assignment_changed);
    assert!(mapping_changed);
    assert!(!is_declared_default(&remapped.pdos, &capabilities));

    let config = EngineeringConfiguration {
        master: 3,
        devices: vec![device_set(
            &capabilities,
            vec![remapped.clone()],
            "remapped",
        )],
    };
    let text = config.to_text();
    let parsed = EngineeringConfiguration::parse(&text).expect("parse remap");
    assert_eq!(parsed, config);
    let report = parsed.validate(|_| None, &BTreeMap::new(), RUNTIME_CYCLE_NS);
    assert!(report.is_valid());
    assert!(!report.devices[0].esi_resolved);

    let mut arbitrary = remapped;
    arbitrary.pdos.pdos[0].entries[0].index = 0x7009;
    let error = arbitrary
        .validate(Some(&capabilities), RUNTIME_CYCLE_NS, 0, "servo")
        .expect_err("object-dictionary-only mapping");
    assert_eq!(error.code, codes::PDO_SELECTION);
    assert!(
        error
            .observed
            .as_deref()
            .expect("observed")
            .contains("arbitrary object-dictionary mapping")
    );
}

#[test]
fn fixed_mandatory_exclusion_and_capability_constraints_are_enforced() {
    let capabilities = capabilities();

    let mut omitted_fixed = capabilities.default_selection();
    omitted_fixed.pdos.retain(|pdo| pdo.index != 0x1602);
    assert_eq!(
        omitted_fixed
            .validate(&capabilities, 0)
            .expect_err("fixed PDO omitted")
            .code,
        codes::PDO_SELECTION
    );

    let mut omitted_mandatory = capabilities.default_selection();
    omitted_mandatory
        .pdos
        .iter_mut()
        .find(|pdo| pdo.index == 0x1a00)
        .expect("mandatory PDO")
        .entries
        .pop();
    assert!(omitted_mandatory.validate(&capabilities, 0).is_err());

    let mut excluded = capabilities.clone();
    excluded.rx_pdos[1].exclude = vec![0x1602];
    assert!(excluded.default_selection().validate(&excluded, 0).is_err());

    let mut no_config = capabilities.clone();
    no_config.mailbox.pdo_config = false;
    let mut reordered = no_config.default_selection();
    reordered
        .pdos
        .iter_mut()
        .find(|pdo| pdo.index == 0x1600)
        .expect("PDO")
        .entries
        .swap(0, 1);
    let error = reordered
        .validate(&no_config, 0)
        .expect_err("PdoConfig gate");
    assert_eq!(error.capability.as_deref(), Some("pdo-config"));

    let mut no_assign = capabilities.clone();
    no_assign.mailbox.pdo_assign = false;
    let mut omitted = no_assign.default_selection();
    omitted.pdos.retain(|pdo| pdo.index != 0x1a01);
    let error = omitted.validate(&no_assign, 0).expect_err("PdoAssign gate");
    assert_eq!(error.capability.as_deref(), Some("pdo-assign"));

    let mut conflict = capabilities;
    conflict.tx_pdos[0].sm = Some(2);
    let error = conflict
        .default_selection()
        .validate(&conflict, 0)
        .expect_err("one direction per sync manager");
    assert!(
        error
            .observed
            .as_deref()
            .expect("observed")
            .contains("used by both rx and tx")
    );
}

#[test]
fn offline_validation_uses_persisted_facts_and_reports_each_device() {
    let capabilities = capabilities();
    let config = configuration(&capabilities, "production");
    let report = config.validate(|_| None, &BTreeMap::new(), RUNTIME_CYCLE_NS);
    assert!(report.is_valid());
    assert!(!report.devices[0].esi_resolved);
    assert!(report.to_json().render().contains("\"valid\": true"));

    let mut broken = config.clone();
    broken.devices[0].modes[0].pdos.pdos[0].entries[0].bit_len = 0;
    let report = broken.validate(|_| None, &BTreeMap::new(), RUNTIME_CYCLE_NS);
    assert!(!report.is_valid());
    assert_eq!(report.diagnostics[0].code, codes::PDO_SELECTION);
    assert_eq!(
        report.devices[0]
            .result
            .as_ref()
            .expect("device result")
            .code,
        codes::PDO_SELECTION
    );

    let mut missing_esi = config.clone();
    missing_esi.devices[0].esi = None;
    assert_eq!(
        missing_esi
            .validate(|_| None, &BTreeMap::new(), RUNTIME_CYCLE_NS)
            .diagnostics[0]
            .code,
        codes::MODE_INVALID
    );

    let mut other_digest = capabilities;
    other_digest.digest = "other-digest".into();
    let report = config.validate(
        |_| Some(other_digest.clone()),
        &BTreeMap::new(),
        RUNTIME_CYCLE_NS,
    );
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
fn live_assignment_and_mapping_compare_as_match_different_or_unverified() {
    let capabilities = capabilities();
    let selection = capabilities.default_selection();

    let unverified = selection.verify(&LivePdoState::new(0));
    assert_eq!(unverified.result, CheckResult::Unverified);
    assert!(unverified.is_unverified());
    assert!(!unverified.is_match());
    assert!(!unverified.checks.is_empty());

    let mut live = LivePdoState::new(0);
    live.set_assignment(LivePdoAssignment::new(2, vec![0x1600, 0x1601, 0x1602]));
    live.set_assignment(LivePdoAssignment::new(3, vec![0x1a00, 0x1a01]));
    for (pdo, entries) in [
        (
            0x1600,
            vec![LiveEntry::new(0x7000, 1, 16), LiveEntry::new(0x7000, 2, 16)],
        ),
        (0x1601, vec![LiveEntry::new(0x7000, 3, 8)]),
        (0x1602, vec![LiveEntry::new(0x7000, 4, 8)]),
        (
            0x1a00,
            vec![LiveEntry::new(0x6000, 1, 16), LiveEntry::new(0x6000, 2, 16)],
        ),
        (0x1a01, vec![LiveEntry::new(0x6000, 3, 1)]),
    ] {
        live.set_mapping(LivePdoMapping::new(pdo, entries));
    }
    let matched = selection.verify(&live);
    assert_eq!(matched.result, CheckResult::Match);
    assert!(matched.is_match());
    assert!(matched.to_json().render().contains("\"result\": \"match\""));

    live.set_mapping(LivePdoMapping::new(
        0x1a01,
        vec![LiveEntry::new(0x6000, 3, 8)],
    ));
    assert_eq!(selection.verify(&live).result, CheckResult::Different);

    let mut partial = LivePdoState::new(0);
    partial.set_assignment(LivePdoAssignment::new(2, vec![0x1600, 0x1601, 0x1602]));
    assert_eq!(selection.verify(&partial).result, CheckResult::Unverified);
}

#[test]
fn a_live_remap_refusal_is_022_and_missing_read_back_never_becomes_a_match() {
    let diagnostic = remap_refused("servo", 0, "Rx sync manager 2", "device aborted 0x06040000");
    assert_eq!(diagnostic.code, "STC-E-IO-ETH-022");
    assert_eq!(diagnostic.code, codes::PDO_REMAP_REFUSED);
    assert_eq!(diagnostic.operation, "pdo.remap");
    assert!(
        diagnostic
            .expected
            .as_deref()
            .expect("expected")
            .contains("Rx sync manager 2")
    );
    assert_eq!(
        diagnostic.observed.as_deref(),
        Some("device aborted 0x06040000")
    );

    let verification = capabilities()
        .default_selection()
        .verify(&LivePdoState::new(0));
    assert_eq!(verification.result, CheckResult::Unverified);
    assert!(!verification.is_match());
    assert!(
        verification
            .checks
            .iter()
            .all(|check| check.result == CheckResult::Unverified)
    );
}

#[test]
fn dc_selection_is_explicit_factor_resolved_and_rejects_ambiguity() {
    let capabilities = capabilities();
    let op_modes = capabilities.dc_op_modes.as_deref();
    let timing = timing(&capabilities);
    assert_eq!(timing.sync0_cycle, 1_000_000);
    assert_eq!(timing.assign_activate, 0x0300);
    DcChoice::OpMode(timing.clone())
        .validate(op_modes, RUNTIME_CYCLE_NS, 0, "servo")
        .expect("declared op mode");
    DcChoice::FreeRun
        .validate(op_modes, RUNTIME_CYCLE_NS, 0, "servo")
        .expect("explicit free-run");

    let mut no_dc = capabilities.clone();
    no_dc.dc_op_modes = None;
    let error = DcChoice::OpMode(timing.clone())
        .validate(no_dc.dc_op_modes.as_deref(), RUNTIME_CYCLE_NS, 0, "servo")
        .expect_err("no DC section");
    assert_eq!(error.code, codes::DC_SELECTION);

    let mut duplicate_modes = capabilities.clone();
    let mut duplicate = duplicate_modes.dc_op_modes.as_mut().expect("dc")[0].clone();
    duplicate.index = 2;
    duplicate_modes
        .dc_op_modes
        .as_mut()
        .expect("dc")
        .push(duplicate);
    let mut ambiguous = timing.clone();
    ambiguous.index = 99;
    let error = DcChoice::OpMode(ambiguous)
        .validate(
            duplicate_modes.dc_op_modes.as_deref(),
            RUNTIME_CYCLE_NS,
            0,
            "servo",
        )
        .expect_err("duplicate name");
    assert!(
        error
            .observed
            .as_deref()
            .expect("observed")
            .contains("duplicated")
    );

    let mut zero_factor = capabilities.clone();
    zero_factor.dc_op_modes.as_mut().expect("dc")[0].sync0_factor = Some(0);
    let error = DcChoice::OpMode(timing.clone())
        .validate(
            zero_factor.dc_op_modes.as_deref(),
            RUNTIME_CYCLE_NS,
            0,
            "servo",
        )
        .expect_err("zero factor");
    assert!(
        error
            .observed
            .as_deref()
            .expect("observed")
            .contains("factor is zero")
    );

    let mut edited = timing;
    edited.assign_activate = 0;
    let error = DcChoice::OpMode(edited)
        .validate(op_modes, RUNTIME_CYCLE_NS, 0, "servo")
        .expect_err("edited AssignActivate");
    assert!(
        error
            .expected
            .as_deref()
            .expect("expected")
            .contains("AssignActivate")
    );

    // Missing `dc` and an unknown mode are parse errors, never a default.
    let config = configuration(&capabilities, "production");
    let without_dc = without_key(&config.to_json(), "dc");
    let error = EngineeringConfiguration::parse(&without_dc.render()).expect_err("missing dc");
    assert_eq!(error.code, codes::DC_SELECTION);

    let unknown_mode = config
        .to_text()
        .replace("\"mode\": \"op-mode\"", "\"mode\": \"observed\"");
    let error = EngineeringConfiguration::parse(&unknown_mode).expect_err("unknown dc mode");
    assert_eq!(error.code, codes::DC_SELECTION);
}

#[test]
fn channel_exposure_accepts_explicit_slices_and_rejects_invalid_exposure() {
    let capabilities = capabilities();

    let mut sliced = mode(&capabilities, "sliced");
    sliced.channels = vec![
        channel(
            "status_word_low",
            0x1a00,
            EntryKey::new(0x6000, 1),
            PdoDirection::Tx,
            0,
            8,
        ),
        channel(
            "status_word_high",
            0x1a00,
            EntryKey::new(0x6000, 1),
            PdoDirection::Tx,
            8,
            8,
        ),
        channel(
            "ready",
            0x1a01,
            EntryKey::new(0x6000, 3),
            PdoDirection::Tx,
            0,
            1,
        ),
    ];
    sliced
        .validate(Some(&capabilities), RUNTIME_CYCLE_NS, 0, "servo")
        .expect("explicit non-overlapping slices");

    let mut bit_slices = mode(&capabilities, "bit-slices");
    bit_slices.channels = vec![
        channel(
            "flag_3",
            0x1a00,
            EntryKey::new(0x6000, 1),
            PdoDirection::Tx,
            3,
            1,
        ),
        channel(
            "flag_4",
            0x1a00,
            EntryKey::new(0x6000, 1),
            PdoDirection::Tx,
            4,
            1,
        ),
    ];
    bit_slices
        .validate(Some(&capabilities), RUNTIME_CYCLE_NS, 0, "servo")
        .expect("one-bit slices may start anywhere");

    let mut unselected_pdo = mode(&capabilities, "unselected-pdo");
    unselected_pdo.channels = vec![channel(
        "missing",
        0x1603,
        EntryKey::new(0x7000, 1),
        PdoDirection::Rx,
        0,
        16,
    )];
    expect_channel_error(&capabilities, unselected_pdo);

    let mut unselected_entry = mode(&capabilities, "unselected-entry");
    unselected_entry.channels = vec![channel(
        "missing",
        0x1600,
        EntryKey::new(0x7009, 1),
        PdoDirection::Rx,
        0,
        16,
    )];
    expect_channel_error(&capabilities, unselected_entry);

    let mut wrong_direction = mode(&capabilities, "wrong-direction");
    wrong_direction.channels = vec![channel(
        "wrong",
        0x1600,
        EntryKey::new(0x7000, 1),
        PdoDirection::Tx,
        0,
        16,
    )];
    expect_channel_error(&capabilities, wrong_direction);

    let mut width_overrun = mode(&capabilities, "width-overrun");
    width_overrun.channels = vec![channel(
        "overrun",
        0x1600,
        EntryKey::new(0x7000, 1),
        PdoDirection::Rx,
        8,
        16,
    )];
    expect_channel_error(&capabilities, width_overrun);

    let mut zero_width = mode(&capabilities, "zero-width");
    zero_width.channels = vec![channel(
        "zero",
        0x1600,
        EntryKey::new(0x7000, 1),
        PdoDirection::Rx,
        0,
        0,
    )];
    expect_channel_error(&capabilities, zero_width);

    let mut misaligned = mode(&capabilities, "misaligned");
    misaligned.channels = vec![channel(
        "misaligned",
        0x1600,
        EntryKey::new(0x7000, 1),
        PdoDirection::Rx,
        4,
        4,
    )];
    expect_channel_error(&capabilities, misaligned);

    let mut overlapping = mode(&capabilities, "overlapping");
    overlapping.channels = vec![
        channel(
            "first",
            0x1600,
            EntryKey::new(0x7000, 1),
            PdoDirection::Rx,
            0,
            8,
        ),
        channel(
            "second",
            0x1600,
            EntryKey::new(0x7000, 1),
            PdoDirection::Rx,
            0,
            8,
        ),
    ];
    expect_channel_error(&capabilities, overlapping);

    let mut duplicate_names = mode(&capabilities, "duplicate-names");
    duplicate_names.channels = vec![
        channel(
            "same",
            0x1600,
            EntryKey::new(0x7000, 1),
            PdoDirection::Rx,
            0,
            16,
        ),
        channel(
            "same",
            0x1600,
            EntryKey::new(0x7000, 2),
            PdoDirection::Rx,
            0,
            16,
        ),
    ];
    expect_channel_error(&capabilities, duplicate_names);

    let mut empty_name = mode(&capabilities, "empty-name");
    empty_name.channels[0].name.clear();
    expect_channel_error(&capabilities, empty_name);
}

#[test]
fn the_coe_view_is_generated_and_non_editable_and_startup_cannot_own_pdo_objects() {
    let capabilities = capabilities();
    let selection = capabilities.default_selection();
    let view = CoeView::from_selection(&selection);
    assert_eq!(view.operations.len(), 26);
    assert_eq!(view.operations[0].object, EntryKey::new(0x1c12, 0));
    assert_eq!(view.operations[0].purpose, "assignment-clear");
    let mapping = view
        .operations
        .iter()
        .find(|operation| {
            operation.object == EntryKey::new(0x1600, 1) && operation.purpose == "mapping"
        })
        .expect("mapping operation");
    assert_eq!(
        mapping.value,
        CoeValue::Mapping {
            index: 0x7000,
            subindex: 1,
            bit_len: 16,
        }
    );
    assert_eq!(mapping.value.packed(), Some(0x7000_0110));

    let json = view.to_json().render();
    assert!(json.contains("\"editable\": false"));
    assert!(json.contains("\"kind\": \"pdo-coe-diagnostic\""));
    assert!(view.operations.iter().all(|operation| {
        matches!(
            operation.object.index,
            0x1600..=0x17ff | 0x1a00..=0x1bff | 0x1c10..=0x1c2f
        )
    }));

    let mut hand_authored = mode(&capabilities, "hand-authored");
    hand_authored.startup.push(StartupOperation {
        id: "pdo-assignment".into(),
        action: StartupAction::Write,
        index: 0x1c12,
        subindex: 0,
        value: SdoValue::Unsigned(1),
        state: StartupState::PreOp,
        verify: StartupVerification::Required,
        timeout_ms: 1_000,
    });
    let error = hand_authored
        .validate(Some(&capabilities), RUNTIME_CYCLE_NS, 0, "servo")
        .expect_err("PDO object in startup");
    assert_eq!(error.code, codes::MODE_INVALID);
    assert!(
        error
            .observed
            .as_deref()
            .expect("observed")
            .contains("PDO assignment/mapping object")
    );
}

#[test]
fn every_phase_three_diagnostic_code_has_an_emitting_fixture() {
    let capabilities = capabilities();
    let mut emitted: BTreeSet<&'static str> = BTreeSet::new();

    let mut missing_mode = configuration(&capabilities, "production");
    missing_mode.devices[0].selected = "missing".into();
    emitted.insert(
        missing_mode
            .validate(
                |_| Some(capabilities.clone()),
                &BTreeMap::new(),
                RUNTIME_CYCLE_NS,
            )
            .diagnostics[0]
            .code,
    );

    let mut arbitrary = mode(&capabilities, "arbitrary");
    arbitrary.pdos.pdos[0].entries[0].index = 0x7009;
    emitted.insert(
        arbitrary
            .validate(Some(&capabilities), RUNTIME_CYCLE_NS, 0, "servo")
            .expect_err("PDO selection")
            .code,
    );

    emitted.insert(remap_refused("servo", 0, "Rx sync manager 2", "device aborted").code);

    let mut bad_dc = mode(&capabilities, "bad-dc");
    bad_dc.dc = DcChoice::OpMode(DcTiming {
        name: "missing".into(),
        index: 0,
        assign_activate: 0x0300,
        sync0_cycle: 1_000_000,
        sync0_shift: 0,
        sync1_cycle: None,
        sync1_shift: None,
    });
    emitted.insert(
        bad_dc
            .validate(Some(&capabilities), RUNTIME_CYCLE_NS, 0, "servo")
            .expect_err("DC selection")
            .code,
    );

    let mut bad_channel = mode(&capabilities, "bad-channel");
    bad_channel.channels[0].width = 0;
    emitted.insert(
        bad_channel
            .validate(Some(&capabilities), RUNTIME_CYCLE_NS, 0, "servo")
            .expect_err("channel exposure")
            .code,
    );

    let text = configuration(&capabilities, "production")
        .to_text()
        .replace(
            "\"name\": \"production\",",
            "\"name\": \"production\",\n      \"velocity\": 10,",
        );
    emitted.insert(
        EngineeringConfiguration::parse(&text)
            .expect_err("vendor motion semantics")
            .code,
    );

    assert_eq!(
        emitted,
        BTreeSet::from([
            codes::MODE_INVALID,
            codes::PDO_SELECTION,
            codes::PDO_REMAP_REFUSED,
            codes::DC_SELECTION,
            codes::CHANNEL_EXPOSURE,
        ])
    );
}
