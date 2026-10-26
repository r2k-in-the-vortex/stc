//! Host task scheduling across scans, swaps, and cold installs (M6 phase 4).
//!
//! The fixtures are compiler-produced manifests paired with no-op C modules, so
//! the host state machine and the scheduler are exercised without ST lowering.

mod common;

use stc_got::{HOST_ABI_MAJOR, HOST_ABI_MINOR};
use stc_runtime::{
    Candidate, Host, HostPlatform, LoadMode, LoadOutcome, ManifestStore, PROFILE, RunOptions,
    run_loop,
};

use common::{Behavior, Fixture};

const TASKS: &str = r#"
PROGRAM Main
VAR x : DINT; END_VAR
  x := x + 1;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  TASK Once (SINGLE := TRUE);
  TASK Tick (INTERVAL := T#10ms, PRIORITY := 1);
  TASK Always (PRIORITY := 2);
  PROGRAM once WITH Once : Main;
  PROGRAM tick WITH Tick : Main;
  PROGRAM always WITH Always : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const TASKS_ALTERNATE: &str = r#"
PROGRAM Main
VAR x : DINT; END_VAR
  x := x + 2;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  TASK Once (SINGLE := TRUE);
  TASK Tick (INTERVAL := T#10ms, PRIORITY := 1);
  TASK Always (PRIORITY := 2);
  PROGRAM once WITH Once : Main;
  PROGRAM tick WITH Tick : Main;
  PROGRAM always WITH Always : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const TASKS_CHANGED_INTERVAL: &str = r#"
PROGRAM Main
VAR x : DINT; END_VAR
  x := x + 2;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  TASK Once (SINGLE := TRUE);
  TASK Tick (INTERVAL := T#20ms, PRIORITY := 1);
  TASK Always (PRIORITY := 2);
  PROGRAM once WITH Once : Main;
  PROGRAM tick WITH Tick : Main;
  PROGRAM always WITH Always : Main;
END_RESOURCE
END_CONFIGURATION
"#;

fn platform() -> HostPlatform {
    HostPlatform::new(PROFILE, common::target(), HOST_ABI_MAJOR, HOST_ABI_MINOR)
}

fn open(fixture: &Fixture) -> Candidate {
    Candidate::open(&platform(), &fixture.module_path, &fixture.manifest_path).expect("candidate")
}

fn fixture(name: &str, artifacts: stc_codegen::module::ModuleArtifacts) -> Option<Fixture> {
    Fixture::build(name, artifacts, Behavior::default())
}

fn install_cold(host: &mut Host, fixture: &Fixture) {
    let outcome = host
        .load(open(fixture), LoadMode::Auto)
        .expect("cold install");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));
}

fn task(host: &Host, name: &str) -> stc_runtime::TaskStatus {
    let suffix = format!("#{}", name.to_ascii_uppercase());
    host.task_status()
        .into_iter()
        .find(|task| task.key.ends_with(&suffix))
        .unwrap_or_else(|| panic!("task `{name}`"))
}

#[test]
fn due_state_intervals_and_single_completion_follow_the_scan_clock() {
    let artifacts = common::artifacts(TASKS, "scheduler");
    let Some(fixture) = fixture("scheduler", artifacts) else {
        return;
    };
    let mut host = Host::new(platform(), ManifestStore::memory());
    install_cold(&mut host, &fixture);

    // First scan: the interval task is not due yet; SINGLE and interval-less
    // tasks run.
    host.cycle_at(0).expect("first scan");
    assert!(task(&host, "Once").completed);
    assert!(task(&host, "Once").due);
    assert!(!task(&host, "Tick").due);
    assert_eq!(task(&host, "Tick").next_deadline_ns, Some(10_000_000));
    assert!(task(&host, "Always").due);

    host.cycle_at(5_000_000).expect("second scan");
    assert!(
        !task(&host, "Once").due,
        "a completed SINGLE task is skipped"
    );
    assert!(!task(&host, "Tick").due);

    host.cycle_at(10_000_000).expect("third scan");
    assert!(task(&host, "Tick").due);
    assert_eq!(task(&host, "Tick").next_deadline_ns, Some(20_000_000));

    // A late scan runs the task once and does not catch up.
    host.cycle_at(35_000_000).expect("late scan");
    assert!(task(&host, "Tick").due);
    assert_eq!(task(&host, "Tick").next_deadline_ns, Some(45_000_000));
    assert_eq!(host.status().cycles, 4);
}

#[test]
fn task_state_survives_a_compatible_swap_and_resets_on_cold() {
    let v1 = common::artifacts(TASKS, "scheduler");
    let Some(v1_fixture) = fixture("scheduler-v1", v1.clone()) else {
        return;
    };
    let mut host = Host::new(platform(), ManifestStore::memory());
    install_cold(&mut host, &v1_fixture);
    host.cycle_at(0).expect("first scan");
    host.cycle_at(5_000_000).expect("second scan");
    assert!(task(&host, "Once").completed);
    assert_eq!(task(&host, "Tick").next_deadline_ns, Some(10_000_000));

    let v2 = common::artifacts_with(TASKS_ALTERNATE, "scheduler", v1.manifest.clone(), false);
    let Some(v2_fixture) = fixture("scheduler-v2", v2.clone()) else {
        return;
    };
    let prepared = host
        .load(open(&v2_fixture), LoadMode::Auto)
        .expect("prepare");
    assert!(matches!(prepared, LoadOutcome::Prepared { .. }));
    host.activate().expect("activate");
    assert_eq!(host.active_build_id(), Some(v2.manifest.header.build_id));
    assert!(
        task(&host, "Once").completed,
        "SINGLE completion survives a compatible swap"
    );
    assert_eq!(
        task(&host, "Tick").next_deadline_ns,
        Some(10_000_000),
        "interval deadlines survive a compatible swap"
    );

    host.cycle_at(10_000_000).expect("scan after swap");
    assert!(!task(&host, "Once").due);
    assert!(task(&host, "Tick").due);
    assert_eq!(task(&host, "Tick").next_deadline_ns, Some(20_000_000));

    // A cold install starts from no completion and no deadlines.
    let outcome = host
        .load(open(&v2_fixture), LoadMode::Cold)
        .expect("cold install");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));
    assert!(!task(&host, "Once").completed);
    assert_eq!(task(&host, "Tick").next_deadline_ns, None);
    host.cycle_at(0).expect("cold first scan");
    assert!(task(&host, "Once").completed);
    assert_eq!(task(&host, "Tick").next_deadline_ns, Some(10_000_000));
}

#[test]
fn a_changed_task_schedule_is_rejected_or_cold() {
    let v1 = common::artifacts(TASKS, "scheduler");
    let Some(v1_fixture) = fixture("scheduler-v1", v1.clone()) else {
        return;
    };
    let mut host = Host::new(platform(), ManifestStore::memory());
    install_cold(&mut host, &v1_fixture);
    host.cycle_at(0).expect("first scan");
    let active = host.active_build_id();

    let changed = common::artifacts_with(
        TASKS_CHANGED_INTERVAL,
        "scheduler",
        v1.manifest.clone(),
        false,
    );
    let Some(changed_fixture) = fixture("scheduler-changed", changed) else {
        return;
    };
    let outcome = host.load(open(&changed_fixture), LoadMode::Online);
    let refused = match outcome {
        Ok(LoadOutcome::ColdRequired { .. }) => true,
        Err(error) => error.code == "STC-E-ONLINE-004" || error.code == "STC-E-MANIFEST-004",
        Ok(other) => panic!("a changed task schedule was accepted: {other:?}"),
    };
    assert!(refused, "a task topology change must be rejected or cold");
    assert_eq!(host.active_build_id(), active);
    assert!(host.status().last_rejection.is_some());
    assert_eq!(
        task(&host, "Tick").next_deadline_ns,
        Some(10_000_000),
        "a rejected change leaves the schedule untouched"
    );
}

#[test]
fn the_run_loop_advances_a_mock_clock_without_sleeping() {
    let artifacts = common::artifacts(TASKS, "scheduler");
    let Some(fixture) = fixture("scheduler", artifacts) else {
        return;
    };
    let mut host = Host::new(platform(), ManifestStore::memory());
    install_cold(&mut host, &fixture);
    let options = RunOptions {
        cycles: 3,
        cycle_time_ns: Some(10_000_000),
        mock_clock_ns: Some(0),
        ..RunOptions::default()
    };
    let cycles = run_loop(&mut host, &options).expect("run loop");
    assert_eq!(cycles, 3);
    assert_eq!(host.active_cycles(), Some(3));
    assert!(task(&host, "Once").completed);
    assert_eq!(
        task(&host, "Tick").next_deadline_ns,
        Some(30_000_000),
        "scans latched at 0, 10ms, and 20ms advance the deadline to 30ms"
    );
}
