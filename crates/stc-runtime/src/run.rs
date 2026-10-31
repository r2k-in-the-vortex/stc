//! Shared host run loop for the launcher and embedding hosts (Runtime §1.1).
//!
//! `stc_rt_execute` and `stc_host_run` both build [`RunOptions`] and call
//! [`run_loop`]. The loop latches a scan time, runs one host cycle, and sleeps
//! to the next deadline when a cycle time is configured. A mock clock advances
//! deterministically without sleeping; the test prelude's `set_cycle_time_ns`
//! override is consumed at the next latch. When [`RunOptions::rt`] is set, the
//! loop instead uses absolute `CLOCK_MONOTONIC` deadlines and the cycle thread
//! runs FIFO; the default relative-sleep path is unchanged.
//!
//! Exit codes follow CLI-and-Manifest §1.3: `0` normal, `1` runtime failure,
//! `4` trap, and `130` interrupted (D-154).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use stc_got::error::Error;

use crate::Host;
use crate::control::{ControlServer, ControlSignal};
use crate::rt::{self, CycleSample, MetricsHeader, MetricsWriter, RtRuntime, SleepError};

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
    /// Opt-in Linux RT mode: absolute pacing, FIFO priority, and metrics.
    pub rt: bool,
    /// FIFO priority for `--rt`; the launcher defaults to 90 when absent.
    pub rt_priority: Option<u8>,
    /// CPU to pin the cycle thread to when `--rt` is set.
    pub rt_cpu: Option<u32>,
    /// JSONL metrics output path when `--rt` is set.
    pub rt_metrics: Option<PathBuf>,
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
    /// The loop could not enter or maintain the requested RT mode.
    Setup {
        /// Description of the setup failure.
        message: String,
    },
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
            Self::Setup { .. } => 2,
            Self::Interrupted { .. } => 130,
            Self::Cycle { trap: true, .. } => crate::STC_RUN_TRAP_EXIT_CODE,
            Self::Cycle { trap: false, .. } => 1,
        }
    }

    /// Returns the cycles completed before the loop stopped.
    #[must_use]
    pub const fn cycles(&self) -> u64 {
        match self {
            Self::Setup { .. } => 0,
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
    let rt_priority = options.rt_priority.unwrap_or(90);
    let rt = if options.rt {
        Some(
            RtRuntime::enter(rt_priority, options.rt_cpu)
                .map_err(|message| RunLoopError::Setup { message })?,
        )
    } else {
        None
    };
    let rt_period_ns = if rt.is_some() {
        match options.cycle_time_ns {
            Some(ns) if ns > 0 => u64::try_from(ns).map_err(|_| RunLoopError::Setup {
                message: "the RT cycle time is out of range".into(),
            })?,
            _ => {
                return Err(RunLoopError::Setup {
                    message: "--rt requires a positive --cycle-time".into(),
                });
            }
        }
    } else {
        0
    };
    let mut metrics = match (&rt, &options.rt_metrics) {
        (Some(_), Some(path)) => {
            Some(MetricsWriter::open(path).map_err(|message| RunLoopError::Setup { message })?)
        }
        _ => None,
    };
    // The first deadline is set after the metrics file is open so file
    // creation does not appear as wake-up lateness.
    let mut deadline_ns = if let Some(rt) = &rt {
        Some(
            rt.now_ns()
                .map_err(|message| RunLoopError::Setup { message })?,
        )
    } else {
        None
    };
    if let Some(metrics) = metrics.as_mut() {
        let header = MetricsHeader {
            build_id: host.status().active_build_id.map(|build| build.to_hex()),
            cycle_time_ns: rt_period_ns,
            priority: rt_priority,
            cpu: options.rt_cpu,
            started_ns: deadline_ns.unwrap_or(0),
        };
        metrics
            .write_header(&header)
            .map_err(|message| RunLoopError::Setup { message })?;
    }
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
        let wake_ns = match &rt {
            Some(rt) => Some(
                rt.now_ns()
                    .map_err(|message| RunLoopError::Setup { message })?,
            ),
            None => None,
        };
        let generation = host.active_generation();
        let started = Instant::now();
        let cycle_result = host.cycle_at(now);
        let scan_end_ns = match &rt {
            Some(rt) => Some(
                rt.now_ns()
                    .map_err(|message| RunLoopError::Setup { message })?,
            ),
            None => None,
        };
        if let Err(error) = cycle_result {
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
        let control_signal = if let Some(control) = control {
            control.process(host)
        } else {
            ControlSignal::None
        };
        let mut next_deadline_ns = None;
        if let (Some(rt), Some(wake_ns), Some(scan_end_ns), Some(current_deadline)) =
            (&rt, wake_ns, scan_end_ns, deadline_ns)
        {
            let now_after_ns = rt
                .now_ns()
                .map_err(|message| RunLoopError::Setup { message })?;
            let (next, skipped) =
                rt::advance_deadline(current_deadline, rt_period_ns, now_after_ns)
                    .map_err(|message| RunLoopError::Setup { message })?;
            next_deadline_ns = Some(next);
            if let Some(metrics) = metrics.as_mut() {
                let next_start_ns = current_deadline.saturating_add(rt_period_ns);
                let wake_lateness_ns = i64::try_from(wake_ns)
                    .unwrap_or(i64::MAX)
                    .saturating_sub(i64::try_from(current_deadline).unwrap_or(i64::MAX));
                let sampled = cycles == 1 || cycles.is_multiple_of(rt::FAULT_INTERVAL_CYCLES);
                let faults = if sampled { rt::fault_counts() } else { None };
                let allocations = if sampled {
                    generation.and_then(|generation| host.transient_counts(generation))
                } else {
                    None
                };
                let sample = CycleSample {
                    cycle: cycles,
                    generation,
                    deadline_ns: current_deadline,
                    wake_ns,
                    scan_start_ns: wake_ns,
                    scan_end_ns,
                    wake_lateness_ns,
                    execution_ns: scan_end_ns.saturating_sub(wake_ns),
                    scan_overrun_ns: scan_end_ns.saturating_sub(next_start_ns),
                    missed_deadline: rt::deadline_missed(
                        wake_ns,
                        scan_end_ns,
                        current_deadline,
                        rt_period_ns,
                    ),
                    skipped_periods: skipped,
                    faults,
                    allocations,
                };
                metrics
                    .record(&sample)
                    .map_err(|message| RunLoopError::Setup { message })?;
            }
        }
        if control_signal == ControlSignal::Stop {
            if let Some(metrics) = metrics.as_mut() {
                let _ = metrics.finish();
            }
            return Ok(cycles);
        }
        if rt.is_some() && options.cycles != 0 && cycles >= options.cycles {
            if let Some(metrics) = metrics.as_mut() {
                let _ = metrics.finish();
            }
            return Ok(cycles);
        }
        if let Some(rt) = &rt {
            let deadline = next_deadline_ns.ok_or_else(|| RunLoopError::Setup {
                message: "the RT loop has no next deadline".into(),
            })?;
            loop {
                match rt.sleep_until(deadline) {
                    Ok(()) => break,
                    Err(SleepError::Interrupted) => {
                        if crate::signal::stop_requested() || stop.requested() {
                            return Err(RunLoopError::Interrupted { cycles });
                        }
                    }
                    Err(SleepError::Failed(message)) => {
                        return Err(RunLoopError::Setup { message });
                    }
                }
            }
            deadline_ns = Some(deadline);
        } else if let Some(mock) = mock.as_mut() {
            *mock = now.saturating_add(options.cycle_time_ns.unwrap_or(0));
        } else if let Some(period) = period {
            let elapsed = started.elapsed();
            if period > elapsed {
                std::thread::sleep(period - elapsed);
            }
        }
    }
    if let Some(metrics) = metrics.as_mut() {
        let _ = metrics.finish();
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
