//! Phase-8 process-image reconfiguration and direct-variable remap tests.
//!
//! The fixtures are real native no-op modules with compiler-produced
//! descriptor/manifest pairs. Explicit metadata supplies the process-image
//! bindings so each test can vary addresses without rebuilding generated code.

mod common;

use stc_got::{GotId, PlanVerdict, RootKind};
use stc_runtime::module_info::{BindingChannel, ChannelInfo, DeviceInfo, ProfileInfo};
use stc_runtime::{
    Candidate, CycleOutcome, DirectBinding, Host, HostPlatform, LayoutState, LoadMode, LoadOutcome,
    ManifestStore, ModuleInfo, PROFILE, RuntimeLayout,
};

use common::{Behavior, Fixture};

fn platform() -> HostPlatform {
    HostPlatform::new(
        PROFILE,
        common::target(),
        stc_got::HOST_ABI_MAJOR,
        stc_got::HOST_ABI_MINOR,
    )
}

fn in_memory_host() -> Host {
    Host::new(platform(), ManifestStore::memory())
}

fn direct_root(artifacts: &stc_codegen::module::ModuleArtifacts, suffix: &str) -> GotId {
    artifacts
        .manifest
        .roots
        .iter()
        .find(|root| root.root_kind == RootKind::Direct && root.key.ends_with(suffix))
        .unwrap_or_else(|| panic!("direct root ending in `{suffix}`"))
        .got_id
}

fn binding(
    artifacts: &stc_codegen::module::ModuleArtifacts,
    suffix: &str,
    area: u8,
    offset_bits: u64,
    width_bits: u64,
) -> DirectBinding {
    DirectBinding {
        got_id: direct_root(artifacts, suffix),
        area,
        offset_bits,
        width_bits,
        device: None,
        channel: None,
        direction: None,
    }
}

fn info(sizes: (u64, u64, u64), bindings: Vec<DirectBinding>) -> ModuleInfo {
    ModuleInfo {
        inputs_bytes: sizes.0,
        outputs_bytes: sizes.1,
        markers_bytes: sizes.2,
        bindings,
        ..ModuleInfo::default()
    }
}

/// Metadata with one direct input bound to a device channel.
///
/// The device kind is not registered by the runtime defaults, so the host
/// validates and hashes the configuration without activating a backend. This
/// keeps the online-change policy test independent of host hardware.
fn hardware_info(artifacts: &stc_codegen::module::ModuleArtifacts, channel: &str) -> ModuleInfo {
    ModuleInfo {
        inputs_bytes: 2,
        markers_bytes: 4,
        bindings: vec![
            DirectBinding {
                got_id: direct_root(artifacts, "INPUT"),
                area: 1,
                offset_bits: 0,
                width_bits: 16,
                device: Some("bench".into()),
                channel: Some(BindingChannel::Name(channel.into())),
                direction: Some("input".into()),
            },
            binding(artifacts, "MARKER", 3, 0, 32),
        ],
        devices: vec![DeviceInfo {
            name: "bench".into(),
            kind: "embedded-bench".into(),
            config: Some("{\"input\":\"0x1a00\"}".into()),
            channels: vec![
                ChannelInfo {
                    name: "IN0".into(),
                    direction: "input".into(),
                    width_bits: 16,
                },
                ChannelInfo {
                    name: "IN1".into(),
                    direction: "input".into(),
                    width_bits: 16,
                },
            ],
        }],
        ..ModuleInfo::default()
    }
}

fn image_length() -> usize {
    usize::try_from(stc_runtime::image_length()).expect("image length")
}

fn image_base() -> usize {
    let base = stc_runtime::image_base();
    assert_ne!(base, 0, "the process image is configured");
    base
}

fn image_bytes() -> Vec<u8> {
    let len = image_length();
    let mut bytes = vec![0; len];
    assert_eq!(stc_runtime::image_read(0, &mut bytes), len);
    bytes
}

fn write_image(offset: usize, bytes: &[u8]) {
    assert!(
        stc_runtime::image_write(offset, bytes),
        "write inside the process image"
    );
}

fn storage(layout: &RuntimeLayout, got_id: GotId) -> Option<usize> {
    layout
        .entries
        .iter()
        .find(|entry| entry.got_id == got_id)
        .and_then(|entry| entry.storage)
}

const REMAP_V1: &str = r#"
VAR_GLOBAL
  Input AT %IW0 : WORD;
  Marker AT %MD0 : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  Marker := Marker + 1;
END_PROGRAM
"#;

const REMAP_V2: &str = r#"
VAR_GLOBAL
  Input AT %IW1 : WORD;
  Marker AT %MD0 : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  Marker := Marker + 1;
END_PROGRAM
"#;

#[test]
fn online_change_reconfigures_the_image_and_remaps_direct_cells() {
    let _guard = common::image_guard();
    let v1 = common::artifacts(REMAP_V1, "remap");
    let v2 = common::artifacts_with(REMAP_V2, "remap", v1.manifest.clone(), false);
    assert_eq!(v2.verdict(), PlanVerdict::OnlineCandidate);
    let Some(v1_fixture) = Fixture::build("remap-v1", v1.clone(), Behavior::default()) else {
        return;
    };
    let Some(v2_fixture) = Fixture::build("remap-v2", v2.clone(), Behavior::default()) else {
        return;
    };
    let platform = platform();
    let mut host = in_memory_host();
    let v1_info = info(
        (2, 0, 4),
        vec![
            binding(&v1, "INPUT", 1, 0, 16),
            binding(&v1, "MARKER", 3, 0, 32),
        ],
    );
    let candidate = Candidate::open_with_info(
        &platform,
        &v1_fixture.module_path,
        &v1_fixture.manifest_path,
        Some(v1_info),
    )
    .expect("v1 candidate");
    assert!(matches!(
        host.load(candidate, LoadMode::Cold).expect("cold install"),
        LoadOutcome::ColdInstalled { .. }
    ));
    assert_eq!(image_bytes().len(), 6);
    write_image(0, &0x1234_u16.to_le_bytes());
    write_image(2, &42_u32.to_le_bytes());

    let input = direct_root(&v1, "INPUT");
    let marker = direct_root(&v1, "MARKER");
    let old_base = image_base();
    assert_eq!(storage(&host.runtime_layout(), input), Some(old_base));
    assert_eq!(storage(&host.runtime_layout(), marker), Some(old_base + 2));

    let v2_info = info(
        (4, 0, 4),
        vec![
            binding(&v2, "INPUT", 1, 16, 16),
            binding(&v2, "MARKER", 3, 0, 32),
        ],
    );
    let candidate = Candidate::open_with_info(
        &platform,
        &v2_fixture.module_path,
        &v2_fixture.manifest_path,
        Some(v2_info),
    )
    .expect("v2 candidate");
    assert!(matches!(
        host.load(candidate, LoadMode::Auto).expect("prepare"),
        LoadOutcome::Prepared { .. }
    ));
    assert_eq!(
        image_bytes().len(),
        6,
        "prepare must not touch the active image"
    );
    assert!(
        host.warnings().is_empty(),
        "warnings are published at activation"
    );

    host.activate().expect("activate");
    let bytes = image_bytes();
    assert_eq!(bytes.len(), 8);
    assert_eq!(
        &bytes[2..4],
        &0x1234_u16.to_le_bytes(),
        "a moved direct variable keeps its value"
    );
    assert_eq!(
        &bytes[4..8],
        &42_u32.to_le_bytes(),
        "the marker value is preserved"
    );
    assert!(
        host.warnings().iter().any(|warning| {
            warning.code == stc_got::codes::W_ONLINE_IO && warning.message.contains("INPUT")
        }),
        "the remap is reported: {:?}",
        host.warnings()
    );

    let base = image_base();
    let layout = host.runtime_layout();
    assert_eq!(storage(&layout, input), Some(base + 2));
    assert_eq!(storage(&layout, marker), Some(base + 4));
    assert!(matches!(
        host.cycle().expect("commit cycle"),
        CycleOutcome::Committed { .. }
    ));
    assert_eq!(&image_bytes()[4..8], &42_u32.to_le_bytes());
}

#[test]
fn online_change_rejects_a_changed_io_configuration() {
    let _guard = common::image_guard();
    let v1 = common::artifacts(REMAP_V1, "hardware");
    let Some(v1_fixture) = Fixture::build("hardware-v1", v1.clone(), Behavior::default()) else {
        return;
    };
    let platform = platform();
    let mut host = in_memory_host();
    let candidate = Candidate::open_with_info(
        &platform,
        &v1_fixture.module_path,
        &v1_fixture.manifest_path,
        Some(hardware_info(&v1, "IN0")),
    )
    .expect("v1 candidate");
    assert!(matches!(
        host.load(candidate, LoadMode::Cold).expect("cold install"),
        LoadOutcome::ColdInstalled { .. }
    ));

    // A same-configuration rebuild preserves the backend configuration, so it
    // can be prepared and activated online.
    let v2 = common::artifacts_with(REMAP_V1, "hardware", v1.manifest.clone(), false);
    assert_eq!(v2.verdict(), PlanVerdict::OnlineCandidate);
    let Some(v2_fixture) = Fixture::build("hardware-v2", v2.clone(), Behavior::default()) else {
        return;
    };
    let candidate = Candidate::open_with_info(
        &platform,
        &v2_fixture.module_path,
        &v2_fixture.manifest_path,
        Some(hardware_info(&v2, "IN0")),
    )
    .expect("v2 candidate");
    assert!(matches!(
        host.load(candidate, LoadMode::Online)
            .expect("same configuration"),
        LoadOutcome::Prepared { .. }
    ));
    host.activate().expect("activate");
    assert!(matches!(
        host.cycle().expect("commit cycle"),
        CycleOutcome::Committed { .. }
    ));

    let baseline = host.export_manifest().expect("baseline").clone();
    let active = host.active_generation();
    let build = host.active_build_id();
    let v3 = common::artifacts_with(REMAP_V1, "hardware", baseline.clone(), false);
    let Some(v3_fixture) = Fixture::build("hardware-v3", v3.clone(), Behavior::default()) else {
        return;
    };

    let mut binding_changed = hardware_info(&v3, "IN0");
    binding_changed.bindings[0].channel = Some(BindingChannel::Index(1));
    let mut pdo_changed = hardware_info(&v3, "IN0");
    pdo_changed.devices[0].config = Some("{\"input\":\"0x1600\"}".into());
    let mut topology_changed = hardware_info(&v3, "IN0");
    topology_changed.devices.push(DeviceInfo {
        name: "aux".into(),
        kind: "embedded-bench".into(),
        config: None,
        channels: vec![ChannelInfo {
            name: "OUT0".into(),
            direction: "output".into(),
            width_bits: 8,
        }],
    });
    let mut profile_changed = hardware_info(&v3, "IN0");
    profile_changed.profile = Some(ProfileInfo {
        path: "io/bench.toml".into(),
        text: String::new(),
        digest: "0xfeed".into(),
        devices: vec!["bench".into()],
    });

    for (label, changed) in [
        ("binding", binding_changed),
        ("pdo", pdo_changed),
        ("topology", topology_changed),
        ("profile", profile_changed),
    ] {
        let candidate = Candidate::open_with_info(
            &platform,
            &v3_fixture.module_path,
            &v3_fixture.manifest_path,
            Some(changed),
        )
        .expect("changed candidate");
        let error = host
            .load(candidate, LoadMode::Online)
            .expect_err("a changed I/O configuration is rejected");
        assert_eq!(error.code, stc_got::codes::ONLINE_IO, "{label}");
        assert!(
            error.message.contains("cold download"),
            "{label}: {}",
            error.message
        );
        assert_eq!(host.active_generation(), active, "{label}");
        assert_eq!(host.active_build_id(), build, "{label}");
        assert!(host.pending_generation().is_none(), "{label}");
        assert_eq!(host.export_manifest(), Some(&baseline), "{label}");
    }
    assert!(
        matches!(
            host.cycle().expect("active generation continues"),
            CycleOutcome::Ran { .. }
        ),
        "the active generation keeps running after each rejection"
    );
}

const SAME_V1: &str = r#"
VAR_GLOBAL
  Out AT %QW1 : WORD;
  Marker AT %MD0 : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

const SAME_V2: &str = r#"
VAR_GLOBAL
  Out AT %QW0 : WORD;
  Pad AT %QW1 : WORD;
  Marker AT %MD0 : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

#[test]
fn same_layout_remap_preserves_values_and_rolls_back() {
    let _guard = common::image_guard();
    let v1 = common::artifacts(SAME_V1, "same");
    let v2 = common::artifacts_with(SAME_V2, "same", v1.manifest.clone(), false);
    assert_eq!(v2.verdict(), PlanVerdict::OnlineCandidate);
    let Some(v1_fixture) = Fixture::build("same-v1", v1.clone(), Behavior::default()) else {
        return;
    };
    let Some(v2_fixture) = Fixture::build(
        "same-v2",
        v2.clone(),
        Behavior {
            cycle_failures: 1,
            ..Behavior::default()
        },
    ) else {
        return;
    };
    let platform = platform();
    let mut host = in_memory_host();
    let v1_info = info(
        (0, 4, 4),
        vec![
            binding(&v1, "OUT", 2, 16, 16),
            binding(&v1, "MARKER", 3, 0, 32),
        ],
    );
    let candidate = Candidate::open_with_info(
        &platform,
        &v1_fixture.module_path,
        &v1_fixture.manifest_path,
        Some(v1_info),
    )
    .expect("v1 candidate");
    assert!(matches!(
        host.load(candidate, LoadMode::Cold).expect("cold install"),
        LoadOutcome::ColdInstalled { .. }
    ));
    assert_eq!(image_bytes().len(), 8);
    write_image(2, &0xBEEF_u16.to_le_bytes());
    write_image(4, &7_u32.to_le_bytes());

    let out = direct_root(&v1, "OUT");
    let marker = direct_root(&v1, "MARKER");
    let old_base = image_base();
    assert_eq!(storage(&host.runtime_layout(), out), Some(old_base + 2));

    let v2_info = info(
        (0, 4, 4),
        vec![
            binding(&v2, "OUT", 2, 0, 16),
            binding(&v2, "PAD", 2, 16, 16),
            binding(&v2, "MARKER", 3, 0, 32),
        ],
    );
    let candidate = Candidate::open_with_info(
        &platform,
        &v2_fixture.module_path,
        &v2_fixture.manifest_path,
        Some(v2_info),
    )
    .expect("v2 candidate");
    assert!(matches!(
        host.load(candidate, LoadMode::Auto).expect("prepare"),
        LoadOutcome::Prepared { .. }
    ));
    host.activate().expect("activate");
    let bytes = image_bytes();
    assert_eq!(
        &bytes[0..2],
        &0xBEEF_u16.to_le_bytes(),
        "the moved output keeps its value"
    );
    assert_eq!(&bytes[4..8], &7_u32.to_le_bytes());
    let base = image_base();
    assert_eq!(storage(&host.runtime_layout(), out), Some(base));

    let outcome = host.cycle().expect("failed cycle");
    assert!(matches!(outcome, CycleOutcome::RolledBack { .. }));
    let bytes = image_bytes();
    assert_eq!(
        &bytes[2..4],
        &0xBEEF_u16.to_le_bytes(),
        "rollback restores the old image"
    );
    assert_eq!(&bytes[4..8], &7_u32.to_le_bytes());
    let base = image_base();
    let layout = host.runtime_layout();
    assert_eq!(storage(&layout, out), Some(base + 2));
    assert_eq!(storage(&layout, marker), Some(base + 4));
}

#[test]
fn cold_install_reinitializes_the_process_image() {
    let _guard = common::image_guard();
    let v1 = common::artifacts(REMAP_V1, "remap");
    let v2 = common::artifacts_with(REMAP_V2, "remap", v1.manifest.clone(), false);
    let Some(v1_fixture) = Fixture::build("cold-v1", v1.clone(), Behavior::default()) else {
        return;
    };
    let Some(v2_fixture) = Fixture::build("cold-v2", v2.clone(), Behavior::default()) else {
        return;
    };
    let platform = platform();
    let mut host = in_memory_host();
    let candidate = Candidate::open_with_info(
        &platform,
        &v1_fixture.module_path,
        &v1_fixture.manifest_path,
        Some(info(
            (2, 0, 4),
            vec![
                binding(&v1, "INPUT", 1, 0, 16),
                binding(&v1, "MARKER", 3, 0, 32),
            ],
        )),
    )
    .expect("v1 candidate");
    host.load(candidate, LoadMode::Cold).expect("cold install");
    write_image(0, &0xFFFF_u16.to_le_bytes());
    write_image(2, &99_u32.to_le_bytes());

    let candidate = Candidate::open_with_info(
        &platform,
        &v2_fixture.module_path,
        &v2_fixture.manifest_path,
        Some(info(
            (4, 0, 4),
            vec![
                binding(&v2, "INPUT", 1, 16, 16),
                binding(&v2, "MARKER", 3, 0, 32),
            ],
        )),
    )
    .expect("v2 candidate");
    host.load(candidate, LoadMode::Cold)
        .expect("cold reinstall");
    let bytes = image_bytes();
    assert_eq!(bytes.len(), 8);
    assert!(
        bytes.iter().all(|byte| *byte == 0),
        "a cold install starts from an initialized image: {bytes:?}"
    );
    let base = image_base();
    let layout = host.runtime_layout();
    assert_eq!(storage(&layout, direct_root(&v2, "INPUT")), Some(base + 2));
    assert!(
        layout
            .entries
            .iter()
            .all(|entry| entry.state == LayoutState::Active),
        "replaced roots are not quarantined by a cold install"
    );
}
