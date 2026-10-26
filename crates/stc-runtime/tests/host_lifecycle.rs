//! Phase-6 host lifecycle integration tests.
//!
//! These tests compile real no-op module fixtures with clang and drive the
//! public host API: cold install, online prepare/activate/commit/rollback,
//! quarantine, active-manifest storage, export/download, pins, and the cold
//! paths selected by stale or missing baselines.

mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use stc_got::{HOST_ABI_MAJOR, HOST_ABI_MINOR, PlanAction, PlanVerdict, RootKind};
use stc_runtime::{
    Candidate, CycleOutcome, DirectBinding, Download, Host, HostPlatform, LayoutState, LoadMode,
    LoadOutcome, ManifestStore, ModuleInfo, PROFILE, PlanDecision,
};

use common::{Behavior, Fixture};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn platform() -> HostPlatform {
    HostPlatform::new(PROFILE, common::target(), HOST_ABI_MAJOR, HOST_ABI_MINOR)
}

fn in_memory_host() -> Host {
    Host::new(platform(), ManifestStore::memory())
}

fn open(fixture: &Fixture) -> Candidate {
    Candidate::open(&platform(), &fixture.module_path, &fixture.manifest_path).expect("candidate")
}

fn fixture(name: &str, artifacts: stc_codegen::module::ModuleArtifacts) -> Option<Fixture> {
    Fixture::build(name, artifacts, Behavior::default())
}

fn root_id(artifacts: &stc_codegen::module::ModuleArtifacts, key: &str) -> stc_got::GotId {
    artifacts
        .manifest
        .roots
        .iter()
        .find(|root| root.key == key)
        .unwrap_or_else(|| panic!("root `{key}`"))
        .got_id
}

fn key_ending(artifacts: &stc_codegen::module::ModuleArtifacts, suffix: &str) -> stc_got::GotId {
    artifacts
        .manifest
        .roots
        .iter()
        .find(|root| root.key.ends_with(suffix))
        .unwrap_or_else(|| panic!("root ending in `{suffix}`"))
        .got_id
}

fn write_u32(host: &mut Host, got_id: stc_got::GotId, value: u32) {
    let storage = host.root_storage_mut(got_id).expect("root storage");
    assert!(storage.len() >= 4);
    storage[..4].copy_from_slice(&value.to_le_bytes());
}

fn read_u32(host: &Host, got_id: stc_got::GotId) -> u32 {
    let storage = host.root_storage(got_id).expect("root storage");
    u32::from_le_bytes(storage[..4].try_into().expect("four bytes"))
}

fn temporary_directory(label: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "stc-runtime-host-test-{}-{id}-{label}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    path
}

fn cold_install(host: &mut Host, fixture: &Fixture) -> u64 {
    let outcome = host
        .load(open(fixture), LoadMode::Auto)
        .expect("cold install");
    let LoadOutcome::ColdInstalled { generation } = outcome else {
        panic!("expected a cold install, got {outcome:?}");
    };
    generation
}

#[test]
fn noop_module_loads_cold_and_runs_cycles() {
    let artifacts = common::artifacts(common::SIMPLE, "simple");
    let Some(fixture) = fixture("simple", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let generation = cold_install(&mut host, &fixture);

    assert_eq!(host.active_generation(), Some(generation));
    assert_eq!(
        host.active_build_id(),
        Some(artifacts.manifest.header.build_id)
    );
    assert_eq!(host.export_manifest(), Some(&artifacts.manifest));
    assert_eq!(
        host.roots().len(),
        artifacts
            .manifest
            .roots
            .iter()
            .filter(|root| root.root_kind != RootKind::Direct)
            .count()
    );

    assert!(matches!(
        host.cycle().expect("first cycle"),
        CycleOutcome::Ran { generation: id } if id == generation
    ));
    assert!(matches!(
        host.cycle().expect("second cycle"),
        CycleOutcome::Ran { .. }
    ));
    assert_eq!(host.active_cycles(), Some(2));

    host.shutdown().expect("shutdown");
    assert_eq!(host.active_generation(), None);
    assert_eq!(host.retired_generations(), 0);
}

#[test]
fn manifest_round_trips_and_can_seed_the_next_build() {
    let directory = temporary_directory("store");
    let artifacts = common::artifacts(common::SIMPLE, "simple");
    let Some(fixture) = fixture("simple", artifacts.clone()) else {
        let _ = fs::remove_dir_all(&directory);
        return;
    };
    let mut host = Host::new(platform(), ManifestStore::open(&directory));
    cold_install(&mut host, &fixture);
    assert!(directory.join("current").is_file());

    let reopened = ManifestStore::open(&directory);
    let active = reopened.active().expect("reopened baseline");
    assert_eq!(active.manifest, artifacts.manifest);
    assert_eq!(active.digest, artifacts.manifest_digest);
    assert_eq!(
        Some(active.json.as_str()),
        host.export_manifest_json(),
        "the on-disk and in-memory baselines agree"
    );

    let download = host.download().expect("download");
    let text = download.to_json_text();
    let parsed = Download::from_json_text(&text).expect("parse download");
    assert_eq!(parsed.manifest(), &artifacts.manifest);
    assert_eq!(
        parsed.deployment.build_id,
        artifacts.manifest.header.build_id
    );
    assert_eq!(parsed.deployment.manifest_digest, artifacts.manifest_digest);
    assert!(!parsed.layout.entries.is_empty());
    assert!(
        parsed
            .layout
            .entries
            .iter()
            .all(|entry| entry.state == LayoutState::Active)
    );

    // A rebuild against the exported baseline reuses every unchanged identity.
    let rebuilt = common::artifacts_with(
        common::SIMPLE_ALTERNATE,
        "simple",
        parsed.manifest().clone(),
        false,
    );
    for root in &artifacts.manifest.roots {
        let matching = rebuilt
            .manifest
            .roots
            .iter()
            .find(|candidate| candidate.key == root.key)
            .expect("matching root");
        assert_eq!(matching.got_id, root.got_id, "key {}", root.key);
    }
    assert_eq!(rebuilt.verdict(), PlanVerdict::OnlineCandidate);

    drop(host);
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn code_only_change_preserves_state_and_commits() {
    let v1 = common::artifacts(common::SIMPLE, "simple");
    let Some(v1_fixture) = fixture("simple-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let v1_generation = cold_install(&mut host, &v1_fixture);
    let counter = root_id(&v1, "_ROOT::COUNTER");
    write_u32(&mut host, counter, 41);

    let v2 = common::artifacts_with(
        common::SIMPLE_ALTERNATE,
        "simple",
        v1.manifest.clone(),
        false,
    );
    assert_eq!(v2.verdict(), PlanVerdict::OnlineCandidate);
    let Some(v2_fixture) = fixture("simple-v2", v2.clone()) else {
        return;
    };
    let candidate = open(&v2_fixture);
    let diff = host.diff(&candidate).expect("diff");
    assert_eq!(diff.decision, Some(PlanDecision::Online));
    assert_eq!(diff.baseline_build_id, Some(v1.manifest.header.build_id));
    assert!(
        diff.actions
            .iter()
            .any(|action| action.action == PlanAction::Preserve),
        "a code-only change preserves identities"
    );

    let outcome = host.load(candidate, LoadMode::Auto).expect("prepare");
    assert!(matches!(outcome, LoadOutcome::Prepared { .. }));
    assert_eq!(
        host.active_build_id(),
        Some(v1.manifest.header.build_id),
        "the old generation still runs while the candidate is prepared"
    );
    assert!(host.pending_generation().is_some());
    assert_eq!(read_u32(&host, counter), 41);

    let v2_generation = host.activate().expect("activate");
    assert_eq!(host.active_build_id(), Some(v2.manifest.header.build_id));
    assert_eq!(read_u32(&host, counter), 41);
    assert!(host.rollback_pending());
    assert!(
        host.download().is_err(),
        "the download artifact is only stable outside a change window"
    );
    assert_eq!(
        host.commit().expect_err("commit without a cycle").code,
        stc_got::codes::ONLINE_PREPARE
    );

    let outcome = host.cycle().expect("commit cycle");
    assert!(matches!(
        outcome,
        CycleOutcome::Committed { generation } if generation == v2_generation
    ));
    assert_eq!(
        host.export_manifest().expect("baseline").header.build_id,
        v2.manifest.header.build_id
    );
    assert_eq!(read_u32(&host, counter), 41);
    assert!(!host.rollback_pending());
    assert_eq!(host.retired_generations(), 0);
    assert_ne!(v1_generation, v2_generation);
}

#[test]
fn new_global_initializes_and_removed_global_is_quarantined() {
    let v1 = common::artifacts(common::SIMPLE, "simple");
    let Some(v1_fixture) = fixture("simple-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &v1_fixture);
    let counter = root_id(&v1, "_ROOT::COUNTER");
    let flag = root_id(&v1, "_ROOT::FLAG");
    write_u32(&mut host, counter, 7);
    host.root_storage_mut(flag).expect("flag")[0] = 1;

    let v2 = common::artifacts_with_renames(
        common::SIMPLE_REPLACED,
        "simple",
        v1.manifest.clone(),
        false,
        Vec::new(),
        true,
    );
    assert!(v2.is_deployable());
    assert_eq!(v2.verdict(), PlanVerdict::OnlineCandidate);
    let Some(v2_fixture) = fixture("simple-v2", v2.clone()) else {
        return;
    };
    let outcome = host
        .load(open(&v2_fixture), LoadMode::Auto)
        .expect("prepare");
    assert!(matches!(outcome, LoadOutcome::Prepared { .. }));
    host.activate().expect("activate");
    assert!(
        host.warnings().iter().any(|warning| {
            warning.code == stc_got::codes::W_ONLINE_REMOVED
                && warning.message.contains("_ROOT::FLAG")
        }),
        "the dropped root is reported: {:?}",
        host.warnings()
    );

    let added = key_ending(&v2, "::ADDED");
    assert!(host.roots().contains(added));
    assert_eq!(host.root_storage(added).expect("added"), &[0; 4]);
    assert!(!host.roots().contains(flag));
    let quarantined = host
        .roots()
        .quarantined()
        .iter()
        .find(|root| root.record.got_id == flag)
        .expect("quarantined flag");
    assert_eq!(quarantined.cause, stc_got::RetiredCause::Dropped);

    host.cycle().expect("commit");
    assert_eq!(read_u32(&host, counter), 7);
    assert_eq!(host.root_storage(added).expect("added"), &[0; 4]);
    assert!(
        host.roots()
            .quarantined()
            .iter()
            .any(|root| { root.record.got_id == flag && root.record.key == "_ROOT::FLAG" })
    );
    assert!(
        host.runtime_layout()
            .entries
            .iter()
            .any(|entry| entry.got_id == flag && entry.state == LayoutState::Quarantined)
    );
}

#[test]
fn prepare_failure_keeps_the_old_generation_intact() {
    let v1 = common::artifacts(common::SIMPLE, "simple");
    let Some(v1_fixture) = fixture("simple-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &v1_fixture);
    let counter = root_id(&v1, "_ROOT::COUNTER");
    write_u32(&mut host, counter, 9);
    let baseline = host.export_manifest().expect("baseline").clone();

    let v2 = common::artifacts_with(
        common::SIMPLE_ALTERNATE,
        "simple",
        v1.manifest.clone(),
        false,
    );
    let Some(failing) = Fixture::build(
        "simple-failing",
        v2.clone(),
        Behavior {
            prepare: 2,
            ..Behavior::default()
        },
    ) else {
        return;
    };
    let error = host
        .load(open(&failing), LoadMode::Auto)
        .expect_err("prepare failure");
    assert_eq!(error.code, stc_got::codes::ONLINE_PREPARE);
    assert_eq!(host.active_build_id(), Some(v1.manifest.header.build_id));
    assert!(host.pending_generation().is_none());
    assert_eq!(read_u32(&host, counter), 9);
    assert_eq!(host.export_manifest(), Some(&baseline));

    // The candidate's root journal was undone: a good candidate still prepares.
    let Some(good) = fixture("simple-good", v2) else {
        return;
    };
    let outcome = host
        .load(open(&good), LoadMode::Auto)
        .expect("prepare after failure");
    assert!(matches!(outcome, LoadOutcome::Prepared { .. }));
    host.abort_pending().expect("abort");
    assert_eq!(host.active_build_id(), Some(v1.manifest.header.build_id));
    assert_eq!(read_u32(&host, counter), 9);
}

#[test]
fn attach_failure_keeps_the_old_generation_intact() {
    let v1 = common::artifacts(common::SIMPLE, "simple");
    let Some(v1_fixture) = fixture("simple-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &v1_fixture);
    let counter = root_id(&v1, "_ROOT::COUNTER");
    write_u32(&mut host, counter, 3);

    let v2 = common::artifacts_with(
        common::SIMPLE_ALTERNATE,
        "simple",
        v1.manifest.clone(),
        false,
    );
    let Some(failing) = Fixture::build(
        "simple-bad-attach",
        v2,
        Behavior {
            attach: 3,
            ..Behavior::default()
        },
    ) else {
        return;
    };
    let error = host
        .load(open(&failing), LoadMode::Auto)
        .expect_err("attach failure");
    assert_eq!(error.code, stc_got::codes::MODULE_INVALID);
    assert_eq!(host.active_build_id(), Some(v1.manifest.header.build_id));
    assert_eq!(read_u32(&host, counter), 3);
    assert!(host.pending_generation().is_none());
}

#[test]
fn first_cycle_failure_rolls_back_new_state() {
    let v1 = common::artifacts(common::SIMPLE, "simple");
    let Some(v1_fixture) = fixture("simple-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let v1_generation = cold_install(&mut host, &v1_fixture);
    let counter = root_id(&v1, "_ROOT::COUNTER");
    write_u32(&mut host, counter, 5);

    let v2 = common::artifacts_with(common::SIMPLE_ADDED, "simple", v1.manifest.clone(), false);
    let added = key_ending(&v2, "::ADDED");
    let Some(v2_fixture) = Fixture::build(
        "simple-cycle-fails",
        v2.clone(),
        Behavior {
            cycle_failures: 1,
            ..Behavior::default()
        },
    ) else {
        return;
    };
    let outcome = host
        .load(open(&v2_fixture), LoadMode::Auto)
        .expect("prepare");
    assert!(matches!(outcome, LoadOutcome::Prepared { .. }));
    host.activate().expect("activate");
    assert!(host.roots().contains(added));

    let outcome = host.cycle().expect("failed cycle");
    let CycleOutcome::RolledBack { restored, reason } = outcome else {
        panic!("expected rollback, got {outcome:?}");
    };
    assert_eq!(restored, v1_generation);
    assert!(reason.contains("returned 1"), "reason: {reason}");
    assert_eq!(host.active_generation(), Some(v1_generation));
    assert_eq!(host.active_build_id(), Some(v1.manifest.header.build_id));
    assert_eq!(read_u32(&host, counter), 5);
    assert!(!host.roots().contains(added));
    assert!(host.root_storage(added).is_none());
    assert!(!host.rollback_pending());
    assert!(host.last_rollback().is_some());
    assert_eq!(
        host.export_manifest().expect("baseline").header.build_id,
        v1.manifest.header.build_id
    );
    assert!(matches!(
        host.cycle().expect("cycle after rollback"),
        CycleOutcome::Ran { .. }
    ));
}

#[test]
fn stale_and_missing_baselines_select_cold() {
    let v1a = common::artifacts(common::SIMPLE, "simple");
    let Some(v1a_fixture) = fixture("simple-v1a", v1a.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &v1a_fixture);

    let v2 = common::artifacts_with(
        common::SIMPLE_ALTERNATE,
        "simple",
        v1a.manifest.clone(),
        false,
    );
    let Some(v2_fixture) = fixture("simple-v2", v2.clone()) else {
        return;
    };

    // Replace the active baseline with a different cold build.
    let other = common::artifacts(common::SIMPLE_ADDED, "simple");
    let Some(other_fixture) = fixture("simple-other", other.clone()) else {
        return;
    };
    let outcome = host
        .load(open(&other_fixture), LoadMode::Cold)
        .expect("cold other");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));
    assert_eq!(host.active_build_id(), Some(other.manifest.header.build_id));

    let outcome = host
        .load(open(&v2_fixture), LoadMode::Auto)
        .expect("stale auto");
    let LoadOutcome::ColdRequired { reason } = outcome else {
        panic!("expected cold required, got {outcome:?}");
    };
    assert!(reason.contains("not exactly current"), "reason: {reason}");
    assert_eq!(host.active_build_id(), Some(other.manifest.header.build_id));
    assert!(host.pending_generation().is_none());

    let outcome = host
        .load(open(&v2_fixture), LoadMode::Online)
        .expect("stale online");
    assert!(outcome.is_cold_required());
    assert_eq!(host.active_build_id(), Some(other.manifest.header.build_id));

    // A fresh host has no baseline at all.
    let mut fresh = in_memory_host();
    let outcome = fresh
        .load(open(&v2_fixture), LoadMode::Auto)
        .expect("missing auto");
    assert!(outcome.is_cold_required());
    assert_eq!(fresh.active_generation(), None);
    let outcome = fresh
        .load(open(&v2_fixture), LoadMode::Cold)
        .expect("explicit cold");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));
    assert_eq!(fresh.active_build_id(), Some(v2.manifest.header.build_id));
    for root in fresh.roots().active() {
        assert!(
            root.storage().iter().all(|byte| *byte == 0),
            "cold install initializes `{}`",
            root.record.key
        );
    }
}

#[test]
fn rejected_candidates_are_never_applied() {
    let v1 = common::artifacts(common::EXPORTED, "exported");
    let Some(v1_fixture) = fixture("exported-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &v1_fixture);
    let baseline = host.export_manifest().expect("baseline").clone();

    let rejected = common::artifacts_with(
        common::EXPORT_REMOVED,
        "exported",
        v1.manifest.clone(),
        false,
    );
    assert_eq!(rejected.verdict(), PlanVerdict::Rejected);
    let Some(fixture) = fixture("exported-rejected", rejected) else {
        return;
    };
    let candidate = open(&fixture);
    let diff = host.diff(&candidate).expect("diff");
    assert_eq!(diff.decision, None);
    assert!(diff.error.is_some());
    let error = host
        .load(candidate, LoadMode::Auto)
        .expect_err("rejected plan");
    assert_eq!(error.code, stc_got::codes::MANIFEST_PLAN);
    let error = host
        .load(open(&fixture), LoadMode::Cold)
        .expect_err("rejected cold plan");
    assert_eq!(error.code, stc_got::codes::MANIFEST_PLAN);
    assert_eq!(host.export_manifest(), Some(&baseline));
}

#[test]
fn generation_pins_delay_retired_module_unload() {
    let v1 = common::artifacts(common::SIMPLE, "simple");
    let Some(v1_fixture) = fixture("simple-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let v1_generation = cold_install(&mut host, &v1_fixture);
    let pin = host.pin_active().expect("pin");

    let v2 = common::artifacts_with(
        common::SIMPLE_ALTERNATE,
        "simple",
        v1.manifest.clone(),
        false,
    );
    let Some(v2_fixture) = fixture("simple-v2", v2) else {
        return;
    };
    host.load(open(&v2_fixture), LoadMode::Auto)
        .expect("prepare");
    host.activate().expect("activate");
    host.cycle().expect("commit");

    assert_eq!(host.retired_generations(), 1);
    assert_eq!(host.unload_retired(), 0);
    drop(pin);
    assert_eq!(host.unload_retired(), 1);
    assert_eq!(host.retired_generations(), 0);

    // A cold install must not free storage while a reader is pinned.
    let pin = host.pin_active().expect("pin second generation");
    let error = host
        .load(open(&v1_fixture), LoadMode::Cold)
        .expect_err("pinned cold install");
    assert_eq!(error.code, stc_got::codes::ONLINE_PREPARE);
    drop(pin);
    let outcome = host
        .load(open(&v1_fixture), LoadMode::Cold)
        .expect("cold install after unpin");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));
    assert_ne!(v1_generation, host.active_generation().expect("active"));
}

#[test]
fn generation_pins_protect_a_concurrent_reader() {
    let v1 = common::artifacts(common::SIMPLE, "pins");
    let Some(v1_fixture) = fixture("pins-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let v1_generation = cold_install(&mut host, &v1_fixture);

    let v2 = common::artifacts_with(common::SIMPLE_ALTERNATE, "pins", v1.manifest.clone(), false);
    let Some(v2_fixture) = fixture("pins-v2", v2) else {
        return;
    };
    host.load(open(&v2_fixture), LoadMode::Auto)
        .expect("prepare");
    host.activate().expect("activate");

    // Pin the previous generation while it is in the rollback window; commit
    // retires it but must not unload it while the reader still holds the pin.
    let pin = host.pin(v1_generation).expect("previous generation pin");
    assert_eq!(pin.generation(), v1_generation);
    host.cycle().expect("commit");
    assert_eq!(host.retired_generations(), 1);

    let (pinned_tx, pinned_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        pinned_tx.send(()).expect("announce the pin");
        release_rx.recv().expect("wait for release");
        // A service reader observes its pinned generation through the pin.
        assert_eq!(pin.generation(), v1_generation);
        drop(pin);
    });
    pinned_rx.recv().expect("reader pin");

    assert_eq!(host.retired_generations(), 1);
    assert_eq!(
        host.unload_retired(),
        0,
        "a pinned generation must stay loaded"
    );
    release_tx.send(()).expect("release the reader");
    reader.join().expect("reader thread");

    assert_eq!(host.unload_retired(), 1);
    assert_eq!(host.retired_generations(), 0);
}

#[test]
fn direct_cells_require_process_image_bindings() {
    let artifacts = common::artifacts(common::DIRECT, "direct");
    let Some(fixture) = fixture("direct", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let error = host
        .load(open(&fixture), LoadMode::Cold)
        .expect_err("missing direct binding");
    assert_eq!(error.code, stc_got::codes::ONLINE_BINDING);
    assert_eq!(host.active_generation(), None);

    let direct = artifacts
        .manifest
        .roots
        .iter()
        .find(|root| root.root_kind == RootKind::Direct)
        .expect("direct root");
    host.set_direct_binding(direct.got_id, 0x1000);
    let outcome = host
        .load(open(&fixture), LoadMode::Cold)
        .expect("cold install with binding");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));
    let layout = host.runtime_layout();
    let entry = layout
        .entries
        .iter()
        .find(|entry| entry.got_id == direct.got_id)
        .expect("direct layout entry");
    assert_eq!(entry.storage, Some(0x1000));
    assert_eq!(entry.state, LayoutState::Active);
}

#[test]
fn rollback_restores_a_quarantined_root() {
    let v1 = common::artifacts(common::SIMPLE, "simple");
    let Some(v1_fixture) = fixture("simple-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &v1_fixture);
    let counter = root_id(&v1, "_ROOT::COUNTER");
    let flag = root_id(&v1, "_ROOT::FLAG");
    write_u32(&mut host, counter, 6);
    host.root_storage_mut(flag).expect("flag")[0] = 1;

    let v2 = common::artifacts_with_renames(
        common::SIMPLE_REPLACED,
        "simple",
        v1.manifest.clone(),
        false,
        Vec::new(),
        true,
    );
    let added = key_ending(&v2, "::ADDED");
    let Some(v2_fixture) = Fixture::build(
        "simple-replaced-fails",
        v2,
        Behavior {
            cycle_failures: 1,
            ..Behavior::default()
        },
    ) else {
        return;
    };
    host.load(open(&v2_fixture), LoadMode::Auto)
        .expect("prepare");
    host.activate().expect("activate");
    assert!(!host.roots().contains(flag));
    assert!(host.roots().contains(added));

    let outcome = host.cycle().expect("failed cycle");
    assert!(matches!(outcome, CycleOutcome::RolledBack { .. }));
    assert!(host.roots().contains(flag));
    assert_eq!(host.root_storage(flag).expect("flag")[0], 1);
    assert!(
        host.roots()
            .quarantined()
            .iter()
            .all(|root| root.record.got_id != flag)
    );
    assert!(!host.roots().contains(added));
    assert_eq!(read_u32(&host, counter), 6);
}

#[test]
fn rollback_restores_a_tracked_rename() {
    let v1 = common::artifacts(common::SIMPLE, "simple");
    let Some(v1_fixture) = fixture("simple-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &v1_fixture);
    let counter = root_id(&v1, "_ROOT::COUNTER");
    write_u32(&mut host, counter, 17);
    assert_eq!(
        host.roots().get(counter).expect("counter").record.key,
        "_ROOT::COUNTER"
    );

    let renamed_source = common::SIMPLE
        .replace("Counter : DINT", "Count : DINT")
        .replace("Counter := ticks", "Count := ticks")
        .replace("Counter > 3", "Count > 3");
    let v2 = common::artifacts_with_renames(
        &renamed_source,
        "simple",
        v1.manifest.clone(),
        false,
        vec![stc_got::Rename::new("_ROOT::COUNTER", "_ROOT::COUNT")],
        false,
    );
    let Some(v2_fixture) = Fixture::build(
        "simple-renamed-fails",
        v2,
        Behavior {
            cycle_failures: 1,
            ..Behavior::default()
        },
    ) else {
        return;
    };
    host.load(open(&v2_fixture), LoadMode::Auto)
        .expect("prepare");
    host.activate().expect("activate");
    assert_eq!(
        host.roots().get(counter).expect("renamed").record.key,
        "_ROOT::COUNT"
    );
    assert_eq!(read_u32(&host, counter), 17);

    let outcome = host.cycle().expect("failed cycle");
    assert!(matches!(outcome, CycleOutcome::RolledBack { .. }));
    assert_eq!(
        host.roots().get(counter).expect("restored").record.key,
        "_ROOT::COUNTER"
    );
    assert_eq!(read_u32(&host, counter), 17);
}

#[test]
fn reset_quarantine_releases_removed_root_storage() {
    let v1 = common::artifacts(common::SIMPLE, "simple");
    let Some(v1_fixture) = fixture("simple-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &v1_fixture);
    let flag = root_id(&v1, "_ROOT::FLAG");
    host.root_storage_mut(flag).expect("flag")[0] = 1;

    let v2 = common::artifacts_with_renames(
        common::SIMPLE_REPLACED,
        "simple",
        v1.manifest.clone(),
        false,
        Vec::new(),
        true,
    );
    let Some(v2_fixture) = fixture("simple-v2", v2) else {
        return;
    };
    host.load(open(&v2_fixture), LoadMode::Auto)
        .expect("prepare");
    host.activate().expect("activate");
    assert_eq!(
        host.reset_quarantine().expect_err("rollback window").code,
        stc_got::codes::ONLINE_PREPARE,
        "quarantine cannot be released while a rollback may still need it"
    );
    host.cycle().expect("commit");
    assert_eq!(host.roots().quarantined().len(), 1);
    assert!(
        host.runtime_layout()
            .entries
            .iter()
            .any(|entry| { entry.got_id == flag && entry.state == LayoutState::Quarantined })
    );

    assert_eq!(host.reset_quarantine().expect("reset"), 1);
    assert!(host.roots().quarantined().is_empty());
    assert!(!host.roots().contains(flag));
    assert!(
        host.runtime_layout()
            .entries
            .iter()
            .all(|entry| entry.got_id != flag)
    );
}

#[test]
fn transient_blocks_are_reclaimed_on_failure_and_unload() {
    let v1 = common::artifacts(common::SIMPLE, "simple");
    let Some(v1_fixture) = fixture("simple-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &v1_fixture);
    let v2 = common::artifacts_with(
        common::SIMPLE_ALTERNATE,
        "simple",
        v1.manifest.clone(),
        false,
    );

    let Some(failing) = Fixture::build(
        "simple-alloc-fails",
        v2.clone(),
        Behavior {
            prepare: 2,
            prepare_alloc_bytes: 64,
            ..Behavior::default()
        },
    ) else {
        return;
    };
    host.load(open(&failing), LoadMode::Auto)
        .expect_err("prepare failure");
    let failed_generation = host
        .last_candidate_generation()
        .expect("candidate generation");
    assert_eq!(
        host.transient_bytes(failed_generation),
        None,
        "failed prepare reclaims transient blocks"
    );

    let Some(good) = Fixture::build(
        "simple-alloc",
        v2.clone(),
        Behavior {
            prepare_alloc_bytes: 64,
            ..Behavior::default()
        },
    ) else {
        return;
    };
    let outcome = host
        .load(open(&good), LoadMode::Auto)
        .expect("prepare with allocation");
    let LoadOutcome::Prepared { generation } = outcome else {
        panic!("expected prepared");
    };
    let prepared_bytes = host
        .transient_bytes(generation)
        .expect("candidate allocator");
    assert!(prepared_bytes >= 64);
    host.abort_pending().expect("abort");
    assert_eq!(host.transient_bytes(generation), None);

    // A cold generation owns transient blocks until shutdown.
    let mut fresh = in_memory_host();
    let outcome = fresh
        .load(open(&good), LoadMode::Cold)
        .expect("cold install with allocation");
    let LoadOutcome::ColdInstalled { generation } = outcome else {
        panic!("expected cold install");
    };
    assert!(fresh.transient_bytes(generation).is_some());
    fresh.shutdown().expect("shutdown");
    assert_eq!(fresh.transient_bytes(generation), None);
}

#[test]
fn failed_cold_install_keeps_the_active_generation() {
    let v1 = common::artifacts(common::SIMPLE, "simple");
    let Some(v1_fixture) = fixture("simple-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let v1_generation = cold_install(&mut host, &v1_fixture);
    let counter = root_id(&v1, "_ROOT::COUNTER");
    write_u32(&mut host, counter, 8);
    let baseline = host.export_manifest().expect("baseline").clone();

    let replacement = common::artifacts(common::SIMPLE_ALTERNATE, "simple");
    let Some(failing) = Fixture::build(
        "simple-cold-fails",
        replacement,
        Behavior {
            prepare: 2,
            ..Behavior::default()
        },
    ) else {
        return;
    };
    let error = host
        .load(open(&failing), LoadMode::Cold)
        .expect_err("cold prepare failure");
    assert_eq!(error.code, stc_got::codes::ONLINE_PREPARE);
    assert_eq!(host.active_generation(), Some(v1_generation));
    assert_eq!(host.active_build_id(), Some(v1.manifest.header.build_id));
    assert_eq!(read_u32(&host, counter), 8);
    assert_eq!(host.export_manifest(), Some(&baseline));
}

#[test]
fn cold_install_reinitializes_every_root() {
    let artifacts = common::artifacts(common::SIMPLE, "simple");
    let Some(fixture) = fixture("simple", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &fixture);
    let counter = root_id(&artifacts, "_ROOT::COUNTER");
    write_u32(&mut host, counter, 33);
    let old_generation = host.active_generation().expect("active");

    let outcome = host
        .load(open(&fixture), LoadMode::Cold)
        .expect("cold reinstall");
    let LoadOutcome::ColdInstalled { generation } = outcome else {
        panic!("expected cold install");
    };
    assert_ne!(generation, old_generation);
    assert_eq!(host.active_generation(), Some(generation));
    assert_eq!(read_u32(&host, counter), 0);
    assert!(host.export_manifest().is_some());
}

#[test]
fn abort_pending_restores_the_committed_baseline() {
    let v1 = common::artifacts(common::SIMPLE, "simple");
    let Some(v1_fixture) = fixture("simple-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &v1_fixture);
    let counter = root_id(&v1, "_ROOT::COUNTER");
    write_u32(&mut host, counter, 12);

    let v2 = common::artifacts_with(common::SIMPLE_ADDED, "simple", v1.manifest.clone(), false);
    let added = key_ending(&v2, "::ADDED");
    let Some(v2_fixture) = fixture("simple-v2", v2) else {
        return;
    };
    host.load(open(&v2_fixture), LoadMode::Auto)
        .expect("prepare");
    assert!(host.roots().contains(added));
    host.abort_pending().expect("abort");
    assert!(host.pending_generation().is_none());
    assert!(!host.roots().contains(added));
    assert_eq!(read_u32(&host, counter), 12);
    assert_eq!(
        host.export_manifest().expect("baseline").header.build_id,
        v1.manifest.header.build_id
    );
}

#[test]
fn store_problem_is_reported_and_a_cold_install_repairs_it() {
    let directory = temporary_directory("corrupt-store");
    fs::create_dir_all(&directory).expect("create");
    fs::write(directory.join("current"), "broken pointer\n").expect("write");
    let mut host = Host::new(platform(), ManifestStore::open(&directory));
    assert!(host.store().problem().is_some());
    assert!(host.download().is_err());

    let artifacts = common::artifacts(common::SIMPLE, "simple");
    let Some(fixture) = fixture("simple", artifacts.clone()) else {
        let _ = fs::remove_dir_all(&directory);
        return;
    };
    let outcome = host
        .load(open(&fixture), LoadMode::Cold)
        .expect("cold repair");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));
    assert!(host.store().problem().is_none());
    assert_eq!(
        host.export_manifest().map(|m| m.header.build_id),
        Some(artifacts.manifest.header.build_id)
    );
    drop(host);
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn download_layout_excludes_runtime_values() {
    let artifacts = common::artifacts(common::SIMPLE, "simple");
    let Some(fixture) = fixture("simple", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &fixture);
    let counter = root_id(&artifacts, "_ROOT::COUNTER");
    write_u32(&mut host, counter, 0xDEAD_BEEF);
    let text = host.download().expect("download").to_json_text();
    assert!(!text.contains("deadbeef"), "values must not appear");
    assert!(!text.contains("3735928559"), "values must not appear");
    assert!(text.contains("manifest_json"));
    let parsed = Download::from_json_text(&text).expect("round trip");
    assert_eq!(parsed.manifest(), &artifacts.manifest);
}

#[test]
fn missing_manifest_file_is_a_manifest_error() {
    let artifacts = common::artifacts(common::SIMPLE, "simple");
    let Some(fixture) = fixture("simple", artifacts) else {
        return;
    };
    let missing = fixture.directory().join("no-such-manifest.bin");
    let error = match Candidate::open(&platform(), &fixture.module_path, &missing) {
        Ok(_) => panic!("a missing manifest must not validate"),
        Err(error) => error,
    };
    assert_eq!(error.code, stc_got::codes::MANIFEST_INVALID);
}

fn direct_info(artifacts: &stc_codegen::module::ModuleArtifacts, inputs: u64) -> ModuleInfo {
    let direct = artifacts
        .manifest
        .roots
        .iter()
        .find(|root| root.root_kind == RootKind::Direct)
        .expect("direct root");
    ModuleInfo {
        inputs_bytes: inputs,
        outputs_bytes: 0,
        markers_bytes: 0,
        bindings: vec![DirectBinding {
            got_id: direct.got_id,
            area: 1,
            offset_bits: 0,
            width_bits: 8,
        }],
        ..ModuleInfo::default()
    }
}

#[test]
fn failed_cold_install_restores_the_process_image_and_direct_cells() {
    let _guard = common::image_guard();
    let artifacts = common::artifacts(common::DIRECT, "direct");
    let Some(fixture) = fixture("direct-info", artifacts.clone()) else {
        return;
    };
    let info = direct_info(&artifacts, 4);
    let mut host = in_memory_host();
    let platform = platform();
    let candidate = Candidate::open_with_info(
        &platform,
        &fixture.module_path,
        &fixture.manifest_path,
        Some(info),
    )
    .expect("candidate");
    let outcome = host.load(candidate, LoadMode::Cold).expect("cold install");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));

    let mut base: *mut u8 = std::ptr::null_mut();
    let mut len: u64 = 0;
    // SAFETY: the runtime writes the base and length into valid locals.
    unsafe { stc_runtime::stc_rt_io_image(&mut base, &mut len) };
    assert_eq!(len, 4);
    // SAFETY: the image has four bytes.
    unsafe {
        *base.add(0) = 0xA5;
        *base.add(3) = 0x5A;
    }

    let replacement = common::artifacts(common::SIMPLE_ALTERNATE, "replacement");
    let Some(failing) = Fixture::build(
        "direct-cold-fails",
        replacement.clone(),
        Behavior {
            prepare: 2,
            ..Behavior::default()
        },
    ) else {
        return;
    };
    let failing_info = ModuleInfo {
        inputs_bytes: 64,
        outputs_bytes: 0,
        markers_bytes: 0,
        bindings: Vec::new(),
        ..ModuleInfo::default()
    };
    let candidate = Candidate::open_with_info(
        &platform,
        &failing.module_path,
        &failing.manifest_path,
        Some(failing_info),
    )
    .expect("failing candidate");
    let error = host
        .load(candidate, LoadMode::Cold)
        .expect_err("cold prepare failure");
    assert_eq!(error.code, stc_got::codes::ONLINE_PREPARE);

    let mut restored: *mut u8 = std::ptr::null_mut();
    let mut restored_len: u64 = 0;
    // SAFETY: the runtime writes the base and length into valid locals.
    unsafe { stc_runtime::stc_rt_io_image(&mut restored, &mut restored_len) };
    assert_eq!(restored_len, 4);
    // SAFETY: the restored image has four bytes.
    unsafe {
        assert_eq!(*restored.add(0), 0xA5);
        assert_eq!(*restored.add(3), 0x5A);
    }

    let direct = artifacts
        .manifest
        .roots
        .iter()
        .find(|root| root.root_kind == RootKind::Direct)
        .expect("direct root");
    let layout = host.runtime_layout();
    let entry = layout
        .entries
        .iter()
        .find(|entry| entry.got_id == direct.got_id)
        .expect("direct layout entry");
    assert_eq!(entry.storage, Some(restored as usize));
}
