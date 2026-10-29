//! Opt-in real-time pacing and cycle metrics (R-3).
//!
//! This module is active only when `RunOptions::rt` is set and only on Linux.
//! It sets the calling (cycle) thread to `SCHED_FIFO`, optionally pins it to
//! one CPU, and provides absolute `CLOCK_MONOTONIC` sleep for the run loop.
//! The metrics writer emits one JSON header, one record per cycle, and a
//! summary record as JSONL.
//!
//! The mode is measurement instrumentation. It does not change the ST clock,
//! the generated modules, or the default run loop, which keeps the original
//! relative sleep path when `--rt` is absent.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

/// Metrics protocol identifier written in the header record.
pub(crate) const METRICS_PROTOCOL: &str = "stc-rt-metrics/1";
/// Cycles between `/proc/self/stat` and allocator-counter samples.
pub(crate) const FAULT_INTERVAL_CYCLES: u64 = 1000;

/// The entered RT mode.
pub(crate) struct RtRuntime;

impl RtRuntime {
    /// Enters FIFO mode and optionally pins the calling thread.
    ///
    /// # Errors
    ///
    /// Returns a message when the platform is unsupported or the syscall is
    /// denied. The caller must not continue with an invalid measurement.
    pub(crate) fn enter(priority: u8, cpu: Option<u32>) -> Result<Self, String> {
        platform::enter(priority, cpu)?;
        Ok(Self)
    }

    /// Returns the current `CLOCK_MONOTONIC` time in nanoseconds.
    ///
    /// # Errors
    ///
    /// Returns a message when the clock cannot be read.
    pub(crate) fn now_ns(&self) -> Result<u64, String> {
        platform::now_ns()
    }

    /// Sleeps until an absolute `CLOCK_MONOTONIC` deadline.
    ///
    /// # Errors
    ///
    /// Returns [`SleepError::Interrupted`] on `EINTR`; the caller decides
    /// whether to retry or stop. Other failures return [`SleepError::Failed`].
    pub(crate) fn sleep_until(&self, deadline_ns: u64) -> Result<(), SleepError> {
        platform::sleep_until(deadline_ns)
    }
}

/// Why an absolute sleep did not complete.
#[derive(Debug)]
pub(crate) enum SleepError {
    /// The sleep was interrupted by a signal.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Interrupted,
    /// The sleep failed for another reason.
    Failed(String),
}

/// Advances a deadline by whole periods until it is not before `now_ns`.
///
/// Returns the next deadline and the number of periods skipped. A deadline
/// exactly equal to `now_ns` is not skipped.
pub(crate) fn advance_deadline(
    deadline_ns: u64,
    period_ns: u64,
    now_ns: u64,
) -> Result<(u64, u64), String> {
    if period_ns == 0 {
        return Err("the RT cycle time must be positive".into());
    }
    let mut next = deadline_ns
        .checked_add(period_ns)
        .ok_or_else(|| "the RT deadline overflows u64".to_string())?;
    if next < now_ns {
        let behind = now_ns - deadline_ns;
        let skipped = behind / period_ns;
        next =
            deadline_ns
                .checked_add(
                    period_ns
                        .checked_mul(skipped.checked_add(1).ok_or_else(|| {
                            "the RT skipped-period count overflows u64".to_string()
                        })?)
                        .ok_or_else(|| "the RT deadline overflows u64".to_string())?,
                )
                .ok_or_else(|| "the RT deadline overflows u64".to_string())?;
        return Ok((next, skipped));
    }
    Ok((next, 0))
}

/// Returns whether a cycle missed its deadline.
///
/// A cycle misses when it wakes after its scheduled start or completes after
/// the next scheduled start.
#[must_use]
pub(crate) fn deadline_missed(
    wake_ns: u64,
    scan_end_ns: u64,
    deadline_ns: u64,
    period_ns: u64,
) -> bool {
    wake_ns > deadline_ns || scan_end_ns > deadline_ns.saturating_add(period_ns)
}

/// One cycle's metric fields.
pub(crate) struct CycleSample {
    pub(crate) cycle: u64,
    pub(crate) generation: Option<u64>,
    pub(crate) deadline_ns: u64,
    pub(crate) wake_ns: u64,
    pub(crate) scan_start_ns: u64,
    pub(crate) scan_end_ns: u64,
    pub(crate) wake_lateness_ns: i64,
    pub(crate) execution_ns: u64,
    pub(crate) scan_overrun_ns: u64,
    pub(crate) missed_deadline: bool,
    pub(crate) skipped_periods: u64,
    pub(crate) faults: Option<(u64, u64)>,
    pub(crate) allocations: Option<(u64, u64)>,
}

/// Header fields for one metrics stream.
pub(crate) struct MetricsHeader {
    pub(crate) build_id: Option<String>,
    pub(crate) cycle_time_ns: u64,
    pub(crate) priority: u8,
    pub(crate) cpu: Option<u32>,
    pub(crate) started_ns: u64,
}

/// Buffered JSONL metrics writer.
pub(crate) struct MetricsWriter {
    writer: std::io::BufWriter<std::fs::File>,
    buffer: String,
    finished: bool,
    cycles: u64,
    missed: u64,
    skipped: u64,
    max_wake_lateness_ns: i64,
    max_execution_ns: u64,
    max_scan_overrun_ns: u64,
    last_faults: Option<(u64, u64)>,
    last_allocations: Option<(u64, u64)>,
}

impl MetricsWriter {
    /// Opens the metrics file without writing a header yet.
    ///
    /// The caller writes the header after the RT setup is complete so the
    /// first deadline does not include file-creation time.
    ///
    /// # Errors
    ///
    /// Returns a message when the file cannot be created.
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::create(path).map_err(|error| {
            format!(
                "cannot create the RT metrics file `{}`: {error}",
                path.display()
            )
        })?;
        Ok(Self {
            writer: std::io::BufWriter::with_capacity(1 << 20, file),
            buffer: String::with_capacity(4096),
            finished: false,
            cycles: 0,
            missed: 0,
            skipped: 0,
            max_wake_lateness_ns: i64::MIN,
            max_execution_ns: 0,
            max_scan_overrun_ns: 0,
            last_faults: None,
            last_allocations: None,
        })
    }

    /// Writes the header record.
    ///
    /// # Errors
    ///
    /// Returns a message when the buffered write fails.
    pub(crate) fn write_header(&mut self, header: &MetricsHeader) -> Result<(), String> {
        self.buffer.clear();
        self.buffer
            .push_str("{\"record\":\"header\",\"protocol\":\"");
        self.buffer.push_str(METRICS_PROTOCOL);
        self.buffer.push_str("\",\"build_id\":");
        match &header.build_id {
            Some(build) => {
                self.buffer.push('"');
                self.buffer.push_str(build);
                self.buffer.push('"');
            }
            None => self.buffer.push_str("null"),
        }
        let _ = write!(
            self.buffer,
            ",\"cycle_time_ns\":{},\"priority\":{},\"cpu\":",
            header.cycle_time_ns, header.priority
        );
        match header.cpu {
            Some(cpu) => {
                let _ = write!(self.buffer, "{cpu}");
            }
            None => self.buffer.push_str("null"),
        }
        let _ = writeln!(
            self.buffer,
            ",\"pid\":{},\"started_monotonic_ns\":{},\"fault_interval\":{}}}",
            std::process::id(),
            header.started_ns,
            FAULT_INTERVAL_CYCLES
        );
        self.flush_buffer()
    }

    /// Appends one cycle record.
    ///
    /// # Errors
    ///
    /// Returns a message when the buffered write fails.
    pub(crate) fn record(&mut self, sample: &CycleSample) -> Result<(), String> {
        self.cycles = sample.cycle;
        if sample.missed_deadline {
            self.missed = self.missed.saturating_add(1);
        }
        self.skipped = self.skipped.saturating_add(sample.skipped_periods);
        self.max_wake_lateness_ns = self.max_wake_lateness_ns.max(sample.wake_lateness_ns);
        self.max_execution_ns = self.max_execution_ns.max(sample.execution_ns);
        self.max_scan_overrun_ns = self.max_scan_overrun_ns.max(sample.scan_overrun_ns);
        if let Some(faults) = sample.faults {
            self.last_faults = Some(faults);
        }
        if let Some(allocations) = sample.allocations {
            self.last_allocations = Some(allocations);
        }

        self.buffer.clear();
        let _ = write!(
            self.buffer,
            "{{\"record\":\"cycle\",\"cycle\":{},\"generation\":",
            sample.cycle
        );
        write_opt_u64(&mut self.buffer, sample.generation);
        let _ = write!(
            self.buffer,
            ",\"deadline_ns\":{},\"wake_ns\":{},\"scan_start_ns\":{},\"scan_end_ns\":{},\
             \"wake_lateness_ns\":{},\"execution_ns\":{},\"scan_overrun_ns\":{},\
             \"missed_deadline\":{},\"skipped_periods\":{},\"minor_faults\":",
            sample.deadline_ns,
            sample.wake_ns,
            sample.scan_start_ns,
            sample.scan_end_ns,
            sample.wake_lateness_ns,
            sample.execution_ns,
            sample.scan_overrun_ns,
            sample.missed_deadline,
            sample.skipped_periods
        );
        write_opt_pair(&mut self.buffer, sample.faults, false);
        self.buffer.push_str(",\"major_faults\":");
        write_opt_pair(&mut self.buffer, sample.faults, true);
        self.buffer.push_str(",\"allocations\":");
        write_opt_pair(&mut self.buffer, sample.allocations, false);
        self.buffer.push_str(",\"frees\":");
        write_opt_pair(&mut self.buffer, sample.allocations, true);
        self.buffer.push_str("}\n");
        self.flush_buffer()
    }

    /// Writes the summary record and flushes the file.
    ///
    /// # Errors
    ///
    /// Returns a message when the final write fails.
    pub(crate) fn finish(&mut self) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }
        self.flush_buffer()?;
        self.finished = true;
        self.buffer.clear();
        let _ = write!(
            self.buffer,
            "{{\"record\":\"summary\",\"cycles\":{},\"missed_deadlines\":{},\
             \"skipped_periods\":{},\"max_wake_lateness_ns\":{},\"max_execution_ns\":{},\
             \"max_scan_overrun_ns\":{},\"last_minor_faults\":",
            self.cycles,
            self.missed,
            self.skipped,
            self.max_wake_lateness_ns.max(0),
            self.max_execution_ns,
            self.max_scan_overrun_ns
        );
        write_opt_pair(&mut self.buffer, self.last_faults, false);
        self.buffer.push_str(",\"last_major_faults\":");
        write_opt_pair(&mut self.buffer, self.last_faults, true);
        self.buffer.push_str(",\"last_allocations\":");
        write_opt_pair(&mut self.buffer, self.last_allocations, false);
        self.buffer.push_str(",\"last_frees\":");
        write_opt_pair(&mut self.buffer, self.last_allocations, true);
        let _ = writeln!(
            self.buffer,
            ",\"finished_monotonic_ns\":{}}}",
            platform::now_ns().unwrap_or(0)
        );
        self.flush_buffer()?;
        self.writer
            .flush()
            .map_err(|error| format!("cannot flush the RT metrics file: {error}"))
    }

    fn flush_buffer(&mut self) -> Result<(), String> {
        self.writer
            .write_all(self.buffer.as_bytes())
            .map_err(|error| format!("cannot write the RT metrics file: {error}"))?;
        self.buffer.clear();
        Ok(())
    }
}

impl Drop for MetricsWriter {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn write_opt_u64(buffer: &mut String, value: Option<u64>) {
    match value {
        Some(value) => {
            let _ = write!(buffer, "{value}");
        }
        None => buffer.push_str("null"),
    }
}

fn write_opt_pair(buffer: &mut String, value: Option<(u64, u64)>, second: bool) {
    match value {
        Some((first, second_value)) => {
            let _ = write!(buffer, "{}", if second { second_value } else { first });
        }
        None => buffer.push_str("null"),
    }
}

/// Reads the process's minor and major page-fault counts.
///
/// Linux only; returns `None` when `/proc/self/stat` cannot be read or parsed.
/// The run loop samples this at [`FAULT_INTERVAL_CYCLES`] rather than every
/// cycle to keep the measurement perturbation bounded.
pub(crate) fn fault_counts() -> Option<(u64, u64)> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/self/stat").ok()?;
        parse_faults(&text)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn parse_faults(text: &str) -> Option<(u64, u64)> {
    let close = text.rfind(')')?;
    let rest = text.get(close + 1..)?.trim_start();
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let minor = fields.get(7)?.parse().ok()?;
    let major = fields.get(9)?.parse().ok()?;
    Some((minor, major))
}

#[cfg(target_os = "linux")]
mod platform {
    use std::io;
    use std::os::raw::c_int;

    const CLOCK_MONOTONIC: c_int = 1;
    const TIMER_ABSTIME: c_int = 1;
    const SCHED_FIFO: c_int = 1;
    const EINTR: c_int = 4;

    #[repr(C)]
    struct Timespec {
        tv_sec: i64,
        tv_nsec: i64,
    }

    #[repr(C)]
    struct SchedParam {
        sched_priority: c_int,
    }

    unsafe extern "C" {
        fn clock_gettime(clockid: c_int, tp: *mut Timespec) -> c_int;
        fn clock_nanosleep(
            clockid: c_int,
            flags: c_int,
            request: *const Timespec,
            remain: *mut Timespec,
        ) -> c_int;
        fn sched_setscheduler(pid: c_int, policy: c_int, param: *const SchedParam) -> c_int;
        fn sched_setaffinity(pid: c_int, cpusetsize: usize, mask: *const u64) -> c_int;
    }

    pub(super) fn enter(priority: u8, cpu: Option<u32>) -> Result<(), String> {
        let param = SchedParam {
            sched_priority: c_int::from(priority),
        };
        // SAFETY: pid 0 names the calling thread; `param` is a valid
        // `sched_param` for the duration of the call.
        let result = unsafe { sched_setscheduler(0, SCHED_FIFO, &param) };
        if result != 0 {
            return Err(format!(
                "cannot set the cycle thread to SCHED_FIFO {priority}: {}",
                io::Error::last_os_error()
            ));
        }
        if let Some(cpu) = cpu {
            let Some(mask) = 1_u64.checked_shl(cpu) else {
                return Err(format!(
                    "--rt-cpu {cpu} is outside the supported 0..=63 range"
                ));
            };
            // SAFETY: pid 0 names the calling thread; the mask is one u64 and
            // the kernel accepts a byte length that covers the selected CPU.
            let result = unsafe { sched_setaffinity(0, std::mem::size_of::<u64>(), &mask) };
            if result != 0 {
                return Err(format!(
                    "cannot pin the cycle thread to CPU {cpu}: {}",
                    io::Error::last_os_error()
                ));
            }
        }
        Ok(())
    }

    pub(super) fn now_ns() -> Result<u64, String> {
        let mut timespec = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `CLOCK_MONOTONIC` writes the supplied timespec.
        let result = unsafe { clock_gettime(CLOCK_MONOTONIC, &mut timespec) };
        if result != 0 {
            return Err(format!(
                "clock_gettime(CLOCK_MONOTONIC) failed: {}",
                io::Error::last_os_error()
            ));
        }
        let seconds = u64::try_from(timespec.tv_sec)
            .map_err(|_| "CLOCK_MONOTONIC seconds are negative".to_string())?;
        let nanos = u64::try_from(timespec.tv_nsec)
            .map_err(|_| "CLOCK_MONOTONIC nanoseconds are negative".to_string())?;
        seconds
            .checked_mul(1_000_000_000)
            .and_then(|value| value.checked_add(nanos))
            .ok_or_else(|| "CLOCK_MONOTONIC timestamp overflows u64".to_string())
    }

    pub(super) fn sleep_until(deadline_ns: u64) -> Result<(), super::SleepError> {
        let request = Timespec {
            tv_sec: i64::try_from(deadline_ns / 1_000_000_000).map_err(|_| {
                super::SleepError::Failed("deadline seconds overflow i64".to_string())
            })?,
            tv_nsec: i64::try_from(deadline_ns % 1_000_000_000).map_err(|_| {
                super::SleepError::Failed("deadline nanoseconds overflow i64".to_string())
            })?,
        };
        // SAFETY: the request is an absolute, read-only deadline; `remain` is
        // null because an absolute sleep does not report the remaining time.
        let result = unsafe {
            clock_nanosleep(
                CLOCK_MONOTONIC,
                TIMER_ABSTIME,
                &request,
                std::ptr::null_mut(),
            )
        };
        if result == 0 {
            Ok(())
        } else if result == EINTR {
            Err(super::SleepError::Interrupted)
        } else {
            Err(super::SleepError::Failed(format!(
                "clock_nanosleep failed: {}",
                io::Error::from_raw_os_error(result)
            )))
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    pub(super) fn enter(_priority: u8, _cpu: Option<u32>) -> Result<(), String> {
        Err("--rt is only supported on Linux".into())
    }

    pub(super) fn now_ns() -> Result<u64, String> {
        Err("--rt is only supported on Linux".into())
    }

    pub(super) fn sleep_until(_deadline_ns: u64) -> Result<(), super::SleepError> {
        Err(super::SleepError::Failed(
            "--rt is only supported on Linux".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadlines_advance_by_whole_periods() {
        assert_eq!(advance_deadline(0, 1_000_000, 0), Ok((1_000_000, 0)));
        assert_eq!(
            advance_deadline(0, 1_000_000, 2_500_000),
            Ok((3_000_000, 2))
        );
        assert_eq!(
            advance_deadline(1_000_000, 1_000_000, 1_000_000),
            Ok((2_000_000, 0))
        );
        assert_eq!(
            advance_deadline(1_000_000, 1_000_000, 3_000_000),
            Ok((4_000_000, 2))
        );
        assert!(advance_deadline(0, 0, 0).is_err());
    }

    #[test]
    fn missed_deadlines_cover_late_wake_and_late_completion() {
        assert!(!deadline_missed(0, 1_000_000, 0, 1_000_000));
        assert!(deadline_missed(1, 1_000_000, 0, 1_000_000));
        assert!(deadline_missed(0, 1_000_001, 0, 1_000_000));
        assert!(!deadline_missed(0, 999_999, 0, 1_000_000));
    }

    #[test]
    fn metrics_records_are_json_lines() {
        let path = std::env::temp_dir().join(format!(
            "stc-rt-metrics-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        ));
        {
            let header = MetricsHeader {
                build_id: Some("deadbeef".into()),
                cycle_time_ns: 1_000_000,
                priority: 90,
                cpu: Some(3),
                started_ns: 123,
            };
            let mut writer = MetricsWriter::open(&path).expect("open");
            writer.write_header(&header).expect("header");
            writer
                .record(&CycleSample {
                    cycle: 1,
                    generation: Some(7),
                    deadline_ns: 123,
                    wake_ns: 124,
                    scan_start_ns: 124,
                    scan_end_ns: 200,
                    wake_lateness_ns: 1,
                    execution_ns: 76,
                    scan_overrun_ns: 0,
                    missed_deadline: false,
                    skipped_periods: 0,
                    faults: Some((3, 4)),
                    allocations: Some((5, 6)),
                })
                .expect("record");
            writer
                .record(&CycleSample {
                    cycle: 2,
                    generation: Some(7),
                    deadline_ns: 1_000_123,
                    wake_ns: 1_000_200,
                    scan_start_ns: 1_000_200,
                    scan_end_ns: 1_000_400,
                    wake_lateness_ns: 77,
                    execution_ns: 200,
                    scan_overrun_ns: 0,
                    missed_deadline: true,
                    skipped_periods: 1,
                    faults: None,
                    allocations: None,
                })
                .expect("record");
            writer.finish().expect("finish");
        }
        let text = std::fs::read_to_string(&path).expect("read metrics");
        let _ = std::fs::remove_file(&path);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "header, two cycles, and summary: {text:?}");
        assert!(lines[0].contains("\"record\":\"header\""));
        assert!(lines[0].contains("\"protocol\":\"stc-rt-metrics/1\""));
        assert!(lines[0].contains("\"build_id\":\"deadbeef\""));
        assert!(lines[0].contains("\"cpu\":3"));
        assert!(lines[1].contains("\"record\":\"cycle\""));
        assert!(lines[1].contains("\"generation\":7"));
        assert!(lines[1].contains("\"minor_faults\":3"));
        assert!(lines[1].contains("\"major_faults\":4"));
        assert!(lines[1].contains("\"allocations\":5"));
        assert!(lines[1].contains("\"frees\":6"));
        assert!(lines[2].contains("\"missed_deadline\":true"));
        assert!(lines[2].contains("\"skipped_periods\":1"));
        assert!(lines[3].contains("\"record\":\"summary\""));
        assert!(lines[3].contains("\"cycles\":2"));
        assert!(lines[3].contains("\"missed_deadlines\":1"));
        assert!(lines[3].contains("\"skipped_periods\":1"));
        assert!(lines[3].contains("\"max_wake_lateness_ns\":77"));
        assert!(lines[3].contains("\"max_execution_ns\":200"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn proc_stat_fault_positions_are_parsed() {
        let stat = "1234 (stc worker) S 1 2 3 4 5 6 7 100 200 300 400 0 0 0";
        assert_eq!(parse_faults(stat), Some((7, 200)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires Linux RT privileges; run explicitly with --ignored on the RT host"]
    fn rt_mode_enters_and_sleeps_to_an_absolute_deadline() {
        let rt = match RtRuntime::enter(90, None) {
            Ok(rt) => rt,
            Err(message) => {
                eprintln!("skipping RT smoke test: {message}");
                return;
            }
        };
        let started = rt.now_ns().expect("CLOCK_MONOTONIC");
        let deadline = started.saturating_add(1_000_000);
        rt.sleep_until(deadline).expect("absolute sleep");
        let finished = rt.now_ns().expect("CLOCK_MONOTONIC");
        assert!(finished >= deadline, "the absolute sleep returned early");
    }
}
