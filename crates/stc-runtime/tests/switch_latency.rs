//! Cycle-boundary switch measurement.
//!
//! The architecture fixes activation as an O(1) pointer swap: root
//! preservation and GOT binding happen during prepare, and `Host::activate`
//! only installs the prepared generation and detaches the previous one. This
//! test records the measured switch and prepare cost for small and large root
//! catalogs so the claim is checked against real compiler-produced module
//! pairs instead of inferred from the code.
//!
//! It is ignored by default because wall-clock thresholds are noisy on shared
//! CI runners. Run it explicitly and record the numbers in the phase notes:
//!
//! ```text
//! cargo test -p stc-runtime --test switch_latency -- --ignored --nocapture
//! ```

mod common;

use std::time::Duration;

use stc_got::{HOST_ABI_MAJOR, HOST_ABI_MINOR};
use stc_runtime::{Candidate, Host, HostPlatform, LoadMode, LoadOutcome, ManifestStore, PROFILE};

use common::{Behavior, Fixture};

const REPEATS: usize = 25;

fn platform() -> HostPlatform {
    HostPlatform::new(PROFILE, common::target(), HOST_ABI_MAJOR, HOST_ABI_MINOR)
}

fn source(roots: usize, increment: i32) -> String {
    let mut text = String::from("VAR_GLOBAL\n");
    for index in 0..roots {
        text.push_str(&format!("  G{index} : DINT;\n"));
    }
    text.push_str("END_VAR\n\nPROGRAM Main\nVAR\n  ticks : DINT;\nEND_VAR\n");
    text.push_str("  ticks := ticks + 1;\n");
    text.push_str(&format!("  G0 := G0 + {increment};\n"));
    text.push_str("END_PROGRAM\n");
    text
}

fn open(host: &Host, fixture: &Fixture) -> Candidate {
    Candidate::open(
        host.platform(),
        &fixture.module_path,
        &fixture.manifest_path,
    )
    .expect("candidate")
}

/// Returns the best-of-[`REPEATS`] prepare and activation durations.
fn measure(roots: usize) -> (Duration, Duration) {
    let name = format!("switch{roots}");
    let v1_artifacts = common::artifacts(&source(roots, 1), &name);
    let v1_fixture = Fixture::build(
        &format!("{name}-v1"),
        v1_artifacts.clone(),
        Behavior::default(),
    )
    .expect("clang is required for the switch measurement");

    let mut host = Host::new(platform(), ManifestStore::memory());
    let outcome = host
        .load(open(&host, &v1_fixture), LoadMode::Auto)
        .expect("cold install");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));

    let v2_artifacts = common::artifacts_with(
        &source(roots, 2),
        &name,
        v1_artifacts.manifest.clone(),
        false,
    );
    let v2_fixture = Fixture::build(&format!("{name}-v2"), v2_artifacts, Behavior::default())
        .expect("clang is required for the switch measurement");

    let mut prepare_best = Duration::MAX;
    let mut activate_best = Duration::MAX;
    for _ in 0..REPEATS {
        let candidate = open(&host, &v2_fixture);
        let prepare_start = std::time::Instant::now();
        host.load(candidate, LoadMode::Auto).expect("prepare");
        prepare_best = prepare_best.min(prepare_start.elapsed());

        let activate_start = std::time::Instant::now();
        host.activate().expect("activate");
        activate_best = activate_best.min(activate_start.elapsed());

        host.rollback()
            .expect("rollback to the baseline generation");
    }
    host.shutdown().expect("shutdown");
    (prepare_best, activate_best)
}

#[test]
#[ignore = "performance measurement; run explicitly with --ignored --nocapture"]
fn activation_latency_is_independent_of_the_root_catalog() {
    let (small_prepare, small_switch) = measure(16);
    let (large_prepare, large_switch) = measure(1024);

    eprintln!("roots=16   prepare={small_prepare:?} switch={small_switch:?}");
    eprintln!("roots=1024 prepare={large_prepare:?} switch={large_switch:?}");

    for (label, duration) in [
        ("small switch", small_switch),
        ("large switch", large_switch),
    ] {
        assert!(
            duration < Duration::from_millis(10),
            "{label} took {duration:?}; activation must stay a bounded pointer swap"
        );
    }
}
