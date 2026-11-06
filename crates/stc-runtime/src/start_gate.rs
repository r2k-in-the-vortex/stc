//! Driver/host start-gate handshake for debug launches (WL-16).
//!
//! A launch-owned debug host publishes its endpoint and then parks before its
//! first cycle until the driver writes the gate file. While parked it services
//! the debug command queue, so `session.start`, `manifest.get`, breakpoint
//! binding, and the launch mode's initial watches are complete before cycle 0.
//! This is not part of the `stc-debug/1` wire protocol: it is a driver/host
//! handshake, and the same mechanism backs the per-test debug harness
//! (Debug and Monitoring §17.7).
//!
//! The wait is bounded by `--start-gate-timeout` (default
//! [`DEFAULT_START_GATE_TIMEOUT`]). A gate that is never released fails the
//! launch instead of running an unbound first cycle.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::host::Host;

/// Poll interval for the start gate.
pub(crate) const START_GATE_POLL: Duration = Duration::from_millis(5);

/// Default budget for the driver to release a launch start gate.
pub(crate) const DEFAULT_START_GATE_TIMEOUT: Duration = Duration::from_secs(30);

/// Blocks until `path` exists, pumping the debug command queue at each poll.
///
/// The parked cycle services debug commands through [`Host::debug_pump_at`],
/// so `manifest.get` and other boundary operations do not wait for cycle 0.
///
/// # Errors
///
/// Returns a readable message when the gate is not released within `timeout`.
pub(crate) fn wait_for_start_gate(
    host: &mut Host,
    path: &Path,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while !path.exists() {
        host.debug_pump_at(0);
        if Instant::now() >= deadline {
            return Err(format!(
                "the debug start gate was not released within {timeout:?}; the first cycle did not run"
            ));
        }
        std::thread::sleep(START_GATE_POLL);
    }
    // The gate is consumed once observed, so repeated launches do not leave
    // empty marker files in the artifact directory.
    let _ = std::fs::remove_file(path);
    Ok(())
}
