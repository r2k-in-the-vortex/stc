//! Shared host run loop for the launcher and embedding hosts (Runtime §1.1).
//!
//! `stc_rt_execute` and `stc_host_run` both build [`RunOptions`] and call
//! [`run_loop`]. The loop latches a scan time, runs one host cycle, and sleeps
//! to the next deadline when a cycle time is configured. A mock clock advances
//! deterministically without sleeping; the test prelude's `set_cycle_time_ns`
//! override is consumed at the next latch.
//!
//! Exit codes follow CLI-and-Manifest §1.3: `0` normal, `1` runtime failure,
//! `4` trap, and `130` interrupted (D-154).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use stc_got::error::Error;

use crate::Host;
use crate::control::{ControlServer, ControlSignal};

/// I/O backend selected for a run.
///
/// The `memory` and `file` backends are wired in M6 phase 5; the phase-4
/// launcher carries the selection so the CLI surface is stable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IoBackend {
    /// Unbound inputs receive defaults; outputs are discarded.
    #[default]
    Null,
    /// Embedding/test input and output bytes.
    Memory,
    /// Input image read at cold start; output image written at shutdown.
    File,
}

impl IoBackend {
    /// Parses the `--io` spelling.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "null" => Some(Self::Null),
            "memory" => Some(Self::Memory),
            "file" => Some(Self::File),
            _ => None,
        }
    }

    /// Returns the `--io` spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Memory => "memory",
            Self::File => "file",
        }
    }
}

/// Options for one run loop.
#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    /// Cycle count; zero runs until interrupted or stopped.
    pub cycles: u64,
    /// Fixed cycle time in nanoseconds; controls the loop deadline.
    pub cycle_time_ns: Option<i64>,
    /// I/O backend override; `None` uses module metadata.
    pub io: Option<IoBackend>,
    /// File backend image path (phase 5).
    pub io_file: Option<PathBuf>,
    /// Retained-state file (phase 5).
    pub state_file: Option<PathBuf>,
    /// Cold start: ignore existing retained state.
    pub reset_retain: bool,
    /// Disable retained-state load and save.
    pub no_retain: bool,
    /// Initial mock-clock time; negative or absent selects the live clock.
    pub mock_clock_ns: Option<i64>,
    /// Print the host status summary at exit.
    pub status: bool,
}

/// How the loop learns that an embedding host asked it to stop.
#[derive(Clone, Copy, Debug)]
pub(crate) enum StopSource {
    /// No external stop condition.
    None,
    /// A C `int32_t` stop flag polled before every scan.
    External(*const i32),
}

impl StopSource {
    fn requested(self) -> bool {
        match self {
            Self::None => false,
            Self::External(flag) => {
                if flag.is_null() {
                    false
                } else {
                    // SAFETY: the caller of `stc_host_run` guarantees the stop
                    // flag stays readable for the duration of the call.
                    unsafe { *flag != 0 }
                }
            }
        }
    }
}

/// Why a run loop stopped early.
#[derive(Debug)]
pub enum RunLoopError {
    /// A stop signal or stop condition ended the loop.
    Interrupted {
        /// Cycles completed before the interruption.
        cycles: u64,
    },
    /// The active generation failed a cycle.
    Cycle {
        /// Cycles completed before the failure.
        cycles: u64,
        /// Host error.
        error: Error,
        /// The module trapped.
        trap: bool,
    },
}

impl RunLoopError {
    /// Returns the CLI exit code for this outcome.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        match self {
            Self::Interrupted { .. } => 130,
            Self::Cycle { trap: true, .. } => crate::STC_RUN_TRAP_EXIT_CODE,
            Self::Cycle { trap: false, .. } => 1,
        }
    }

    /// Returns the cycles completed before the loop stopped.
    #[must_use]
    pub const fn cycles(&self) -> u64 {
        match self {
            Self::Interrupted { cycles } | Self::Cycle { cycles, .. } => *cycles,
        }
    }
}

/// Runs the host loop with no external stop condition.
///
/// # Errors
///
/// Returns [`RunLoopError`] when the loop is interrupted or a cycle fails.
pub fn run_loop(host: &mut Host, options: &RunOptions) -> Result<u64, RunLoopError> {
    run_loop_with_stop(host, options, StopSource::None)
}

pub(crate) fn run_loop_with_stop(
    host: &mut Host,
    options: &RunOptions,
    stop: StopSource,
) -> Result<u64, RunLoopError> {
    run_loop_with_control(host, options, stop, None)
}

pub(crate) fn run_loop_with_control(
    host: &mut Host,
    options: &RunOptions,
    stop: StopSource,
    control: Option<&ControlServer>,
) -> Result<u64, RunLoopError> {
    crate::signal::install_stop_handler();
    let period = options
        .cycle_time_ns
        .and_then(|ns| u64::try_from(ns).ok())
        .map(Duration::from_nanos);
    let mut cycles = 0_u64;
    let mut mock = options.mock_clock_ns;
    while options.cycles == 0 || cycles < options.cycles {
        if crate::signal::stop_requested() || stop.requested() {
            return Err(RunLoopError::Interrupted { cycles });
        }
        let now = match crate::take_pending_cycle_ns() {
            Some(pending) => pending,
            None => mock.unwrap_or_else(crate::monotonic_now_ns),
        };
        let started = Instant::now();
        if let Err(error) = host.cycle_at(now) {
            let trap = crate::host_api::last_trap().is_some();
            if trap {
                let _ = host.debug_target_exited();
            }
            return Err(RunLoopError::Cycle {
                cycles,
                error,
                trap,
            });
        }
        cycles += 1;
        // Deploy work is prepared and activated between scans on the cycle
        // thread, so ST is never re-entered (D-155).
        if let Some(control) = control
            && control.process(host) == ControlSignal::Stop
        {
            return Ok(cycles);
        }
        if let Some(mock) = mock.as_mut() {
            *mock = now.saturating_add(options.cycle_time_ns.unwrap_or(0));
        } else if let Some(period) = period {
            let elapsed = started.elapsed();
            if period > elapsed {
                std::thread::sleep(period - elapsed);
            }
        }
    }
    Ok(cycles)
}

/// Parses a duration literal (`10ms`, `5s`, `0ns`, `2us`, `1m`, `1h`).
///
/// The grammar matches the `stc.toml` duration values; `--cycle-time` and the
/// mock-clock start use the same spelling.
#[must_use]
pub fn parse_duration_ns(text: &str) -> Option<u64> {
    let split = text
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len());
    if split == 0 {
        return None;
    }
    let number: u64 = text.get(..split)?.parse().ok()?;
    let factor = match text.get(split..)? {
        "ns" => 1,
        "us" | "µs" => 1_000,
        "ms" => 1_000_000,
        "s" => 1_000_000_000,
        "m" => 60_000_000_000,
        "h" => 3_600_000_000_000,
        _ => return None,
    };
    number.checked_mul(factor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_match_the_manifest_spelling() {
        assert_eq!(parse_duration_ns("10ms"), Some(10_000_000));
        assert_eq!(parse_duration_ns("5s"), Some(5_000_000_000));
        assert_eq!(parse_duration_ns("0ns"), Some(0));
        assert_eq!(parse_duration_ns("2us"), Some(2_000));
        assert_eq!(parse_duration_ns("10"), None);
        assert_eq!(parse_duration_ns("ms"), None);
        assert_eq!(parse_duration_ns("-1s"), None);
        assert_eq!(parse_duration_ns("10xs"), None);
    }

    #[test]
    fn io_backends_parse_their_cli_spelling() {
        assert_eq!(IoBackend::parse("null"), Some(IoBackend::Null));
        assert_eq!(IoBackend::parse("memory"), Some(IoBackend::Memory));
        assert_eq!(IoBackend::parse("file"), Some(IoBackend::File));
        assert_eq!(IoBackend::parse("disk"), None);
        assert_eq!(IoBackend::Null.name(), "null");
    }

    #[test]
    fn run_loop_errors_map_to_cli_exit_codes() {
        let interrupted = RunLoopError::Interrupted { cycles: 3 };
        assert_eq!(interrupted.exit_code(), 130);
        assert_eq!(interrupted.cycles(), 3);

        let trapped = RunLoopError::Cycle {
            cycles: 1,
            error: Error::new("STC-E-ONLINE-005", "trapped"),
            trap: true,
        };
        assert_eq!(trapped.exit_code(), crate::STC_RUN_TRAP_EXIT_CODE);
        assert_eq!(trapped.exit_code(), 4);

        let failed = RunLoopError::Cycle {
            cycles: 2,
            error: Error::new("STC-E-ONLINE-005", "failed"),
            trap: false,
        };
        assert_eq!(failed.exit_code(), 1);
        assert_eq!(failed.cycles(), 2);
    }
}
