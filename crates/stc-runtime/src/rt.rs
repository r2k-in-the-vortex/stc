//! Opt-in real-time pacing and cycle metrics (R-3).
//!
//! This module is active only when `RunOptions::rt` is set. On Linux it sets
//! the calling (cycle) thread to `SCHED_FIFO`, optionally pins it to one CPU,
//! and provides absolute `CLOCK_MONOTONIC` sleep for the run loop. On Windows
//! it raises the cycle thread to `THREAD_PRIORITY_TIME_CRITICAL`, registers it
//! with MMCSS when available, and waits on a high-resolution waitable timer.
//! On macOS it applies a Mach `THREAD_TIME_CONSTRAINT_POLICY` (falling back to
//! an interactive QoS class) and uses `mach_wait_until`. The metrics writer
//! emits one JSON header, one record per cycle, and a summary record as JSONL.
//! A dedicated writer thread at a lower priority owns the file and its buffer,
//! so the cycle thread never blocks on disk I/O.
//!
//! The mode is measurement instrumentation. It does not change the ST clock,
//! the generated modules, or the default run loop, which keeps the original
//! relative sleep path when `--rt` is absent. It is advisory and is not a
//! hard-real-time or safety guarantee.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::sync::mpsc::{self, SyncSender};
use std::thread::JoinHandle;

/// Metrics protocol identifier written in the header record.
pub(crate) const METRICS_PROTOCOL: &str = "stc-rt-metrics/1";
/// Cycles between `/proc/self/stat` and allocator-counter samples.
pub(crate) const FAULT_INTERVAL_CYCLES: u64 = 1000;
/// Cycle records the writer queue can hold before records are dropped.
///
/// The cycle thread never blocks on the writer: a full queue drops the record
/// and the summary reports the count.
const METRICS_QUEUE_CAPACITY: usize = 8192;
/// The entered RT mode.
pub(crate) struct RtRuntime {
    background_affinity: Option<u64>,
    sched: &'static str,
    priority: u8,
    state: platform::State,
}

impl RtRuntime {
    /// Enters the host's opt-in scheduling mode and optionally pins the
    /// calling thread (Linux only).
    ///
    /// When a cycle CPU is requested, the thread's affinity mask before the
    /// pin is captured without that CPU for the metrics writer thread.
    ///
    /// # Errors
    ///
    /// Returns a message when the platform is unsupported, the facility is
    /// denied, or a Linux-only option is used on another host. The caller
    /// must not continue with an invalid measurement.
    pub(crate) fn enter(priority: u8, cpu: Option<u32>, period_ns: u64) -> Result<Self, String> {
        Ok(Self::from_entered(platform::enter(
            priority, cpu, period_ns,
        )?))
    }

    fn from_entered(entered: Entered) -> Self {
        Self {
            background_affinity: entered.background_affinity,
            sched: entered.sched,
            priority: entered.priority,
            state: entered.state,
        }
    }

    /// Returns the affinity mask for the metrics writer thread, if any.
    pub(crate) fn background_affinity(&self) -> Option<u64> {
        self.background_affinity
    }

    /// Returns the scheduling facility recorded in the metrics header.
    pub(crate) fn sched(&self) -> &'static str {
        self.sched
    }

    /// Returns the header priority value.
    ///
    /// This is the FIFO priority on Linux and zero on hosts where no FIFO
    /// priority applies; the `sched` field names the actual facility.
    pub(crate) fn priority(&self) -> u8 {
        self.priority
    }

    /// Returns the current monotonic time in nanoseconds.
    ///
    /// # Errors
    ///
    /// Returns a message when the clock cannot be read.
    pub(crate) fn now_ns(&self) -> Result<u64, String> {
        platform::now_ns()
    }

    /// Sleeps until an absolute monotonic deadline.
    ///
    /// # Errors
    ///
    /// Returns [`SleepError::Interrupted`] when the wait is aborted; the
    /// caller decides whether to retry or stop. Other failures return
    /// [`SleepError::Failed`].
    pub(crate) fn sleep_until(&self, deadline_ns: u64) -> Result<(), SleepError> {
        platform::sleep_until(&self.state, deadline_ns)
    }
}

impl Drop for RtRuntime {
    fn drop(&mut self) {
        platform::cleanup(&mut self.state);
    }
}

/// One platform entry result.
struct Entered {
    background_affinity: Option<u64>,
    sched: &'static str,
    priority: u8,
    state: platform::State,
}

/// Why an absolute sleep did not complete.
#[derive(Debug)]
pub(crate) enum SleepError {
    /// The sleep was interrupted by a signal or an alertable wait.
    #[cfg_attr(
        not(any(target_os = "linux", windows, target_os = "macos")),
        allow(dead_code)
    )]
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
#[derive(Clone, Copy)]
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
    /// Applied scheduling facility: `fifo`, `mmcss`, `time_critical`,
    /// `time_constraint`, or `qos`.
    pub(crate) sched: &'static str,
    pub(crate) cpu: Option<u32>,
    pub(crate) started_ns: u64,
}

/// One message from the cycle thread to the metrics writer thread.
enum WriterMessage {
    Header(MetricsHeader),
    Cycle(CycleSample),
    Finish(WriterSummary),
}

/// Summary counters computed on the cycle thread.
#[derive(Clone, Copy)]
struct WriterSummary {
    cycles: u64,
    missed: u64,
    skipped: u64,
    max_wake_lateness_ns: i64,
    max_execution_ns: u64,
    max_scan_overrun_ns: u64,
    last_faults: Option<(u64, u64)>,
    last_allocations: Option<(u64, u64)>,
    dropped_records: u64,
}

/// Non-blocking JSONL metrics writer.
///
/// The cycle thread only updates counters and enqueues a copy of the sample.
/// A dedicated thread at `SCHED_OTHER` and below-normal nice owns the file and
/// its 1 MiB buffer, so a buffer flush never stalls the cycle thread. When the
/// bounded queue is full the record is dropped and counted; the summary
/// reports the count as `dropped_records`.
pub(crate) struct MetricsWriter {
    sender: SyncSender<WriterMessage>,
    handle: Option<JoinHandle<Result<(), String>>>,
    finished: bool,
    cycles: u64,
    missed: u64,
    skipped: u64,
    max_wake_lateness_ns: i64,
    max_execution_ns: u64,
    max_scan_overrun_ns: u64,
    last_faults: Option<(u64, u64)>,
    last_allocations: Option<(u64, u64)>,
    dropped_records: u64,
}

impl MetricsWriter {
    /// Opens the metrics file without writing a header yet.
    ///
    /// The caller writes the header after the RT setup is complete so the
    /// first deadline does not include file-creation time. The writer thread
    /// is demoted to `SCHED_OTHER` with a below-normal nice level before this
    /// function returns; when `background_affinity` is set it is moved off the
    /// cycle CPU as well.
    ///
    /// # Errors
    ///
    /// Returns a message when the file cannot be created or the writer thread
    /// cannot start or enter its background scheduling class.
    pub(crate) fn open(path: &Path, background_affinity: Option<u64>) -> Result<Self, String> {
        Self::open_with(path, METRICS_QUEUE_CAPACITY, background_affinity, None)
    }

    fn open_with(
        path: &Path,
        capacity: usize,
        background_affinity: Option<u64>,
        start_gate: Option<std::sync::Arc<std::sync::Barrier>>,
    ) -> Result<Self, String> {
        let file = std::fs::File::create(path).map_err(|error| {
            format!(
                "cannot create the RT metrics file `{}`: {error}",
                path.display()
            )
        })?;
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let (ready_sender, ready_receiver) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("stc-rt-metrics".into())
            .spawn(move || {
                writer_main(
                    receiver,
                    file,
                    background_affinity,
                    start_gate,
                    ready_sender,
                )
            })
            .map_err(|error| format!("cannot start the RT metrics writer thread: {error}"))?;
        match ready_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                sender,
                handle: Some(handle),
                finished: false,
                cycles: 0,
                missed: 0,
                skipped: 0,
                max_wake_lateness_ns: i64::MIN,
                max_execution_ns: 0,
                max_scan_overrun_ns: 0,
                last_faults: None,
                last_allocations: None,
                dropped_records: 0,
            }),
            Ok(Err(message)) => {
                let _ = handle.join();
                Err(message)
            }
            Err(_) => {
                let _ = handle.join();
                Err("the RT metrics writer thread stopped during startup".into())
            }
        }
    }

    /// Queues the header record.
    ///
    /// # Errors
    ///
    /// Returns a message when the writer thread has stopped.
    pub(crate) fn write_header(&mut self, header: MetricsHeader) -> Result<(), String> {
        self.sender
            .send(WriterMessage::Header(header))
            .map_err(|_| "the RT metrics writer stopped before the header".to_string())
    }

    /// Queues one cycle record without blocking the cycle thread.
    ///
    /// A full queue drops the record; the drop count is written in the
    /// summary.
    pub(crate) fn record(&mut self, sample: &CycleSample) {
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
        if self.sender.try_send(WriterMessage::Cycle(*sample)).is_err() {
            self.dropped_records = self.dropped_records.saturating_add(1);
        }
    }

    /// Queues the summary, drains the writer, and joins it.
    ///
    /// # Errors
    ///
    /// Returns a message when the writer thread stopped, panicked, or could
    /// not write or flush the file.
    pub(crate) fn finish(&mut self) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        let summary = WriterSummary {
            cycles: self.cycles,
            missed: self.missed,
            skipped: self.skipped,
            max_wake_lateness_ns: self.max_wake_lateness_ns,
            max_execution_ns: self.max_execution_ns,
            max_scan_overrun_ns: self.max_scan_overrun_ns,
            last_faults: self.last_faults,
            last_allocations: self.last_allocations,
            dropped_records: self.dropped_records,
        };
        let sent = self.sender.send(WriterMessage::Finish(summary));
        let Some(handle) = self.handle.take() else {
            return sent
                .map_err(|_| "the RT metrics writer stopped before the summary".to_string());
        };
        match handle.join() {
            Ok(Ok(())) => {
                sent.map_err(|_| "the RT metrics writer stopped before the summary".to_string())
            }
            Ok(Err(message)) => Err(message),
            Err(_) => Err("the RT metrics writer thread panicked".into()),
        }
    }
}

impl Drop for MetricsWriter {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

/// Owns the metrics file and writes queued records until the finish message.
fn writer_main(
    receiver: mpsc::Receiver<WriterMessage>,
    file: std::fs::File,
    background_affinity: Option<u64>,
    start_gate: Option<std::sync::Arc<std::sync::Barrier>>,
    ready_sender: mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    // Explicitly leave the inherited FIFO scheduling class and the cycle
    // CPU before consuming any records.
    if let Err(message) = platform::background(background_affinity) {
        let _ = ready_sender.send(Err(message.clone()));
        return Err(message);
    }
    if ready_sender.send(Ok(())).is_err() {
        return Ok(());
    }
    if let Some(gate) = start_gate {
        gate.wait();
    }
    let mut writer = std::io::BufWriter::with_capacity(1 << 20, file);
    let mut buffer = String::with_capacity(4096);
    loop {
        match receiver.recv() {
            Ok(WriterMessage::Header(header)) => {
                format_header(&mut buffer, &header);
                write_line(&mut writer, &mut buffer)?;
            }
            Ok(WriterMessage::Cycle(sample)) => {
                format_cycle(&mut buffer, &sample);
                write_line(&mut writer, &mut buffer)?;
            }
            Ok(WriterMessage::Finish(summary)) => {
                format_summary(&mut buffer, &summary);
                write_line(&mut writer, &mut buffer)?;
                return writer
                    .flush()
                    .map_err(|error| format!("cannot flush the RT metrics file: {error}"));
            }
            Err(_) => return Ok(()),
        }
    }
}

fn write_line(
    writer: &mut std::io::BufWriter<std::fs::File>,
    buffer: &mut String,
) -> Result<(), String> {
    writer
        .write_all(buffer.as_bytes())
        .map_err(|error| format!("cannot write the RT metrics file: {error}"))?;
    buffer.clear();
    Ok(())
}

fn format_header(buffer: &mut String, header: &MetricsHeader) {
    buffer.clear();
    buffer.push_str("{\"record\":\"header\",\"protocol\":\"");
    buffer.push_str(METRICS_PROTOCOL);
    buffer.push_str("\",\"build_id\":");
    match &header.build_id {
        Some(build) => {
            buffer.push('"');
            buffer.push_str(build);
            buffer.push('"');
        }
        None => buffer.push_str("null"),
    }
    let _ = write!(
        buffer,
        ",\"cycle_time_ns\":{},\"priority\":{},\"sched\":\"{}\",\"cpu\":",
        header.cycle_time_ns, header.priority, header.sched
    );
    match header.cpu {
        Some(cpu) => {
            let _ = write!(buffer, "{cpu}");
        }
        None => buffer.push_str("null"),
    }
    let _ = writeln!(
        buffer,
        ",\"pid\":{},\"started_monotonic_ns\":{},\"fault_interval\":{}}}",
        std::process::id(),
        header.started_ns,
        FAULT_INTERVAL_CYCLES
    );
}

fn format_cycle(buffer: &mut String, sample: &CycleSample) {
    buffer.clear();
    let _ = write!(
        buffer,
        "{{\"record\":\"cycle\",\"cycle\":{},\"generation\":",
        sample.cycle
    );
    write_opt_u64(buffer, sample.generation);
    let _ = write!(
        buffer,
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
    write_opt_pair(buffer, sample.faults, false);
    buffer.push_str(",\"major_faults\":");
    write_opt_pair(buffer, sample.faults, true);
    buffer.push_str(",\"allocations\":");
    write_opt_pair(buffer, sample.allocations, false);
    buffer.push_str(",\"frees\":");
    write_opt_pair(buffer, sample.allocations, true);
    buffer.push_str("}\n");
}

fn format_summary(buffer: &mut String, summary: &WriterSummary) {
    buffer.clear();
    let _ = write!(
        buffer,
        "{{\"record\":\"summary\",\"cycles\":{},\"missed_deadlines\":{},\
         \"skipped_periods\":{},\"dropped_records\":{},\"max_wake_lateness_ns\":{},\
         \"max_execution_ns\":{},\"max_scan_overrun_ns\":{},\"last_minor_faults\":",
        summary.cycles,
        summary.missed,
        summary.skipped,
        summary.dropped_records,
        summary.max_wake_lateness_ns.max(0),
        summary.max_execution_ns,
        summary.max_scan_overrun_ns
    );
    write_opt_pair(buffer, summary.last_faults, false);
    buffer.push_str(",\"last_major_faults\":");
    write_opt_pair(buffer, summary.last_faults, true);
    buffer.push_str(",\"last_allocations\":");
    write_opt_pair(buffer, summary.last_allocations, false);
    buffer.push_str(",\"last_frees\":");
    write_opt_pair(buffer, summary.last_allocations, true);
    let _ = writeln!(
        buffer,
        ",\"finished_monotonic_ns\":{}}}",
        platform::now_ns().unwrap_or(0)
    );
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

    /// Linux keeps no state beyond the scheduling change.
    pub(super) struct State;

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
        fn sched_getaffinity(pid: c_int, cpusetsize: usize, mask: *mut u64) -> c_int;
        fn sched_setaffinity(pid: c_int, cpusetsize: usize, mask: *const u64) -> c_int;
    }

    pub(super) fn enter(
        priority: u8,
        cpu: Option<u32>,
        _period_ns: u64,
    ) -> Result<super::Entered, String> {
        let pinned = match cpu {
            Some(cpu) => {
                let Some(mask) = 1_u64.checked_shl(cpu) else {
                    return Err(format!(
                        "--rt-cpu {cpu} is outside the supported 0..=63 range"
                    ));
                };
                Some((cpu, mask))
            }
            None => None,
        };
        // Read the caller's mask before pinning it, so the metrics writer can
        // be moved to the remaining CPUs instead of inheriting the cycle CPU.
        let background_affinity = match pinned {
            Some((cpu, mask)) => {
                let mut current = 0_u64;
                // SAFETY: pid 0 names the calling thread; the mask is one u64.
                let result =
                    unsafe { sched_getaffinity(0, std::mem::size_of::<u64>(), &mut current) };
                if result != 0 {
                    return Err(format!(
                        "cannot read the cycle thread affinity: {}",
                        io::Error::last_os_error()
                    ));
                }
                let background = current & !mask;
                if background == 0 {
                    return Err(format!(
                        "--rt-cpu {cpu} leaves no CPU for the RT metrics writer thread"
                    ));
                }
                Some(background)
            }
            None => None,
        };
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
        if let Some((cpu, mask)) = pinned {
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
        Ok(super::Entered {
            background_affinity,
            sched: "fifo",
            priority,
            state: State,
        })
    }

    pub(super) fn background(affinity: Option<u64>) -> Result<(), String> {
        crate::priority::lower_service_priority_checked()?;
        if let Some(mask) = affinity {
            // SAFETY: pid 0 names the calling thread; the mask is one u64.
            let result = unsafe { sched_setaffinity(0, std::mem::size_of::<u64>(), &mask) };
            if result != 0 {
                return Err(format!(
                    "cannot move the metrics writer thread off the cycle CPU: {}",
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

    pub(super) fn sleep_until(_state: &State, deadline_ns: u64) -> Result<(), super::SleepError> {
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

    pub(super) fn cleanup(_state: &mut State) {}
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
    use std::io;

    const THREAD_PRIORITY_TIME_CRITICAL: i32 = 15;
    const THREAD_PRIORITY_ERROR_RETURN: i32 = 0x7fff_ffff;
    const CREATE_WAITABLE_TIMER_HIGH_RESOLUTION: u32 = 0x0000_0002;
    const TIMER_ALL_ACCESS: u32 = 0x001f_0003;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_IO_COMPLETION: u32 = 0x0000_00c0;
    const WAIT_FAILED: u32 = 0xffff_ffff;
    const INFINITE: u32 = 0xffff_ffff;
    const MMCSS_TASK_NAME: [u16; 6] = [0x41, 0x75, 0x64, 0x69, 0x6f, 0];

    #[link(name = "avrt")]
    unsafe extern "system" {
        fn AvSetMmThreadCharacteristicsW(
            task_name: *const u16,
            task_index: *mut u32,
        ) -> *mut c_void;
        fn AvRevertMmThreadCharacteristics(handle: *mut c_void) -> i32;
    }

    unsafe extern "system" {
        fn GetCurrentThread() -> *mut c_void;
        fn GetThreadPriority(thread: *mut c_void) -> i32;
        fn SetThreadPriority(thread: *mut c_void, priority: i32) -> i32;
        fn CreateWaitableTimerExW(
            attributes: *const c_void,
            name: *const u16,
            flags: u32,
            desired_access: u32,
        ) -> *mut c_void;
        fn SetWaitableTimer(
            timer: *mut c_void,
            due_time: *const i64,
            period: i32,
            completion: *const c_void,
            argument: *mut c_void,
            resume: i32,
        ) -> i32;
        fn WaitForSingleObjectEx(handle: *mut c_void, milliseconds: u32, alertable: i32) -> u32;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn QueryPerformanceCounter(counter: *mut i64) -> i32;
        fn QueryPerformanceFrequency(frequency: *mut i64) -> i32;
    }

    /// Windows state captured for `Drop`.
    pub(super) struct State {
        previous_priority: i32,
        mmcss: Option<*mut c_void>,
        timer: *mut c_void,
    }

    pub(super) fn enter(
        _priority: u8,
        cpu: Option<u32>,
        _period_ns: u64,
    ) -> Result<super::Entered, String> {
        if cpu.is_some() {
            return Err("`--rt-cpu` is only supported on Linux".into());
        }
        // SAFETY: the pseudo-handle names the calling thread.
        let thread = unsafe { GetCurrentThread() };
        // SAFETY: the pseudo-handle names the calling thread.
        let previous_priority = unsafe { GetThreadPriority(thread) };
        if previous_priority == THREAD_PRIORITY_ERROR_RETURN {
            return Err(format!(
                "cannot read the cycle thread priority: {}",
                io::Error::last_os_error()
            ));
        }
        // SAFETY: the pseudo-handle names the calling thread, and the priority
        // class is a documented constant.
        let result = unsafe { SetThreadPriority(thread, THREAD_PRIORITY_TIME_CRITICAL) };
        if result == 0 {
            return Err(format!(
                "cannot set the cycle thread to THREAD_PRIORITY_TIME_CRITICAL: {}",
                io::Error::last_os_error()
            ));
        }
        let mut task_index = 0_u32;
        // SAFETY: the task name is a static NUL-terminated wide string and the
        // index points to a writable u32.
        let mmcss =
            unsafe { AvSetMmThreadCharacteristicsW(MMCSS_TASK_NAME.as_ptr(), &mut task_index) };
        let (mmcss, sched) = if mmcss.is_null() {
            eprintln!(
                "warning: MMCSS registration failed ({}); --rt uses \
                 THREAD_PRIORITY_TIME_CRITICAL only",
                io::Error::last_os_error()
            );
            (None, "time_critical")
        } else {
            (Some(mmcss), "mmcss")
        };
        let timer = match create_timer(true) {
            Ok(timer) => timer,
            Err(high_resolution_error) => match create_timer(false) {
                Ok(timer) => timer,
                Err(error) => {
                    // The timer is part of the RT setup, so undo the priority
                    // and MMCSS changes before failing the run.
                    if let Some(handle) = mmcss {
                        // SAFETY: `handle` came from
                        // `AvSetMmThreadCharacteristicsW`.
                        unsafe {
                            let _ = AvRevertMmThreadCharacteristics(handle);
                        }
                    }
                    // SAFETY: the pseudo-handle names the calling thread.
                    unsafe {
                        let _ = SetThreadPriority(thread, previous_priority);
                    }
                    return Err(format!(
                        "{error} (the high-resolution timer failed: {high_resolution_error})"
                    ));
                }
            },
        };
        Ok(super::Entered {
            background_affinity: None,
            sched,
            priority: 0,
            state: State {
                previous_priority,
                mmcss,
                timer,
            },
        })
    }

    fn create_timer(high_resolution: bool) -> Result<*mut c_void, String> {
        let flags = if high_resolution {
            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION
        } else {
            0
        };
        // SAFETY: null attributes and name are accepted; the access mask
        // requests the rights needed to arm and wait on the timer.
        let timer = unsafe {
            CreateWaitableTimerExW(std::ptr::null(), std::ptr::null(), flags, TIMER_ALL_ACCESS)
        };
        if timer.is_null() {
            Err(format!(
                "cannot create the RT waitable timer: {}",
                io::Error::last_os_error()
            ))
        } else {
            Ok(timer)
        }
    }

    pub(super) fn background(_affinity: Option<u64>) -> Result<(), String> {
        crate::priority::lower_writer_priority_checked()
    }

    pub(super) fn now_ns() -> Result<u64, String> {
        let mut counter = 0_i64;
        let mut frequency = 0_i64;
        // SAFETY: both functions write one i64 to the supplied pointer.
        if unsafe { QueryPerformanceCounter(&mut counter) } == 0 {
            return Err(format!(
                "QueryPerformanceCounter failed: {}",
                io::Error::last_os_error()
            ));
        }
        // SAFETY: the function writes one i64 to the supplied pointer.
        if unsafe { QueryPerformanceFrequency(&mut frequency) } == 0 {
            return Err(format!(
                "QueryPerformanceFrequency failed: {}",
                io::Error::last_os_error()
            ));
        }
        let counter = u64::try_from(counter)
            .map_err(|_| "the performance counter is negative".to_string())?;
        let frequency = u64::try_from(frequency)
            .map_err(|_| "the performance counter frequency is not positive".to_string())?;
        let nanos = u128::from(counter) * 1_000_000_000 / u128::from(frequency);
        u64::try_from(nanos).map_err(|_| "the performance counter timestamp overflows u64".into())
    }

    pub(super) fn sleep_until(state: &State, deadline_ns: u64) -> Result<(), super::SleepError> {
        let now = now_ns().map_err(super::SleepError::Failed)?;
        let remaining_ns = deadline_ns.saturating_sub(now);
        // A relative due time is negative and expressed in 100 ns units. Zero
        // would disarm the timer, so an expired deadline waits one tick.
        let hundred_ns = remaining_ns.saturating_add(99) / 100;
        let due = -i64::try_from(hundred_ns).unwrap_or(i64::MAX).max(1);
        // SAFETY: the timer handle is live and the due time is a valid i64.
        let result = unsafe {
            SetWaitableTimer(
                state.timer,
                &due,
                0,
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
            )
        };
        if result == 0 {
            return Err(super::SleepError::Failed(format!(
                "cannot arm the RT waitable timer: {}",
                io::Error::last_os_error()
            )));
        }
        // SAFETY: the timer handle is live; the wait is alertable so an APC
        // can interrupt it.
        let result = unsafe { WaitForSingleObjectEx(state.timer, INFINITE, 1) };
        match result {
            WAIT_OBJECT_0 => Ok(()),
            WAIT_IO_COMPLETION => Err(super::SleepError::Interrupted),
            WAIT_FAILED => Err(super::SleepError::Failed(format!(
                "the RT waitable timer wait failed: {}",
                io::Error::last_os_error()
            ))),
            code => Err(super::SleepError::Failed(format!(
                "the RT waitable timer wait returned unexpected code {code}"
            ))),
        }
    }

    pub(super) fn cleanup(state: &mut State) {
        if let Some(handle) = state.mmcss.take() {
            // SAFETY: `handle` came from `AvSetMmThreadCharacteristicsW` and
            // has not been reverted.
            unsafe {
                let _ = AvRevertMmThreadCharacteristics(handle);
            }
        }
        // SAFETY: the pseudo-handle names the calling thread.
        let result = unsafe { SetThreadPriority(GetCurrentThread(), state.previous_priority) };
        if result == 0 {
            eprintln!(
                "warning: cannot restore the cycle thread priority: {}",
                io::Error::last_os_error()
            );
        }
        let timer = std::mem::replace(&mut state.timer, std::ptr::null_mut());
        if !timer.is_null() {
            // SAFETY: `timer` came from `CreateWaitableTimerExW` and has not
            // been closed.
            unsafe {
                let _ = CloseHandle(timer);
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::io;
    use std::os::raw::c_int;

    const CLOCK_MONOTONIC: c_int = 6;
    const KERN_SUCCESS: c_int = 0;
    const KERN_ABORTED: c_int = 14;
    const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
    const QOS_CLASS_UNSPECIFIED: u32 = 0;
    const THREAD_STANDARD_POLICY: u32 = 1;
    const THREAD_STANDARD_POLICY_COUNT: u32 = 0;
    const THREAD_TIME_CONSTRAINT_POLICY: u32 = 2;
    const THREAD_TIME_CONSTRAINT_POLICY_COUNT: u32 = 4;

    #[repr(C)]
    struct Timespec {
        tv_sec: i64,
        tv_nsec: i64,
    }

    #[repr(C)]
    struct TimebaseInfo {
        numer: u32,
        denom: u32,
    }

    #[repr(C)]
    struct TimeConstraintPolicy {
        period: u32,
        computation: u32,
        constraint: u32,
        preemptible: i32,
    }

    unsafe extern "C" {
        fn clock_gettime(clockid: c_int, tp: *mut Timespec) -> c_int;
        fn mach_absolute_time() -> u64;
        fn mach_timebase_info(info: *mut TimebaseInfo) -> c_int;
        fn mach_wait_until(deadline: u64) -> c_int;
        fn pthread_self() -> usize;
        fn pthread_mach_thread_np(thread: usize) -> u32;
        fn pthread_get_qos_class_np(
            thread: usize,
            qos_class: *mut u32,
            relative_priority: *mut c_int,
        ) -> c_int;
        fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: c_int) -> c_int;
        fn thread_policy_set(
            thread: u32,
            policy: u32,
            policy_info: *const i32,
            count: u32,
        ) -> c_int;
    }

    /// macOS state captured for `Drop`.
    pub(super) struct State {
        captured_qos: u32,
        captured_relative_priority: c_int,
        /// A successful time-constraint set makes the kernel remove the
        /// thread's QoS policy; the captured class is then no longer valid.
        time_constraint: bool,
    }

    pub(super) fn enter(
        _priority: u8,
        cpu: Option<u32>,
        period_ns: u64,
    ) -> Result<super::Entered, String> {
        if cpu.is_some() {
            return Err("`--rt-cpu` is only supported on Linux".into());
        }
        // SAFETY: `pthread_self` returns the calling thread; the out pointers
        // are writable.
        let thread = unsafe { pthread_self() };
        let mut captured_qos = QOS_CLASS_UNSPECIFIED;
        let mut captured_relative_priority = 0;
        // SAFETY: the thread is the calling thread and both out pointers are
        // writable.
        let result = unsafe {
            pthread_get_qos_class_np(thread, &mut captured_qos, &mut captured_relative_priority)
        };
        if result != 0 {
            return Err(format!(
                "cannot read the cycle thread QoS class: error {result}"
            ));
        }
        let info = timebase_info()?;
        let policy = TimeConstraintPolicy {
            period: clamp_u32(ns_to_ticks(period_ns.saturating_mul(2), &info)),
            computation: clamp_u32(ns_to_ticks(period_ns / 2, &info)),
            constraint: clamp_u32(ns_to_ticks(period_ns, &info)),
            preemptible: 1,
        };
        // SAFETY: the thread port names the calling thread and the policy
        // struct matches `thread_time_constraint_policy`.
        let thread_port = unsafe { pthread_mach_thread_np(thread) };
        // SAFETY: the thread port is valid for the call and the policy struct
        // is `THREAD_TIME_CONSTRAINT_POLICY_COUNT` integers.
        let result = unsafe {
            thread_policy_set(
                thread_port,
                THREAD_TIME_CONSTRAINT_POLICY,
                (&policy as *const TimeConstraintPolicy).cast::<i32>(),
                THREAD_TIME_CONSTRAINT_POLICY_COUNT,
            )
        };
        let sched = if result == KERN_SUCCESS {
            "time_constraint"
        } else {
            eprintln!(
                "warning: THREAD_TIME_CONSTRAINT_POLICY was rejected \
                 (kern_return {result}); --rt falls back to \
                 QOS_CLASS_USER_INTERACTIVE"
            );
            // SAFETY: the call changes only the calling thread's QoS class.
            let result = unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0) };
            if result != 0 {
                return Err(format!(
                    "cannot apply the interactive QoS fallback: error {result}"
                ));
            }
            "qos"
        };
        Ok(super::Entered {
            background_affinity: None,
            sched,
            priority: 0,
            state: State {
                captured_qos,
                captured_relative_priority,
                time_constraint: sched == "time_constraint",
            },
        })
    }

    fn timebase_info() -> Result<TimebaseInfo, String> {
        let mut info = TimebaseInfo { numer: 0, denom: 0 };
        // SAFETY: the function writes one `mach_timebase_info_data_t`.
        let result = unsafe { mach_timebase_info(&mut info) };
        if result != KERN_SUCCESS || info.numer == 0 || info.denom == 0 {
            return Err(format!("mach_timebase_info failed: kern_return {result}"));
        }
        Ok(info)
    }

    fn ns_to_ticks(ns: u64, info: &TimebaseInfo) -> u64 {
        let ticks = u128::from(ns) * u128::from(info.denom) / u128::from(info.numer);
        u64::try_from(ticks).unwrap_or(u64::MAX)
    }

    fn clamp_u32(value: u64) -> u32 {
        u32::try_from(value).unwrap_or(u32::MAX).max(1)
    }

    pub(super) fn background(_affinity: Option<u64>) -> Result<(), String> {
        crate::priority::lower_writer_priority_checked()
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

    pub(super) fn sleep_until(_state: &State, deadline_ns: u64) -> Result<(), super::SleepError> {
        let now = now_ns().map_err(super::SleepError::Failed)?;
        let remaining_ns = deadline_ns.saturating_sub(now);
        let info = timebase_info().map_err(super::SleepError::Failed)?;
        let ticks = ns_to_ticks(remaining_ns, &info);
        // SAFETY: `mach_absolute_time` reads the monotonic clock.
        let deadline = unsafe { mach_absolute_time() }.saturating_add(ticks);
        // SAFETY: the absolute deadline is in Mach time units.
        match unsafe { mach_wait_until(deadline) } {
            KERN_SUCCESS => Ok(()),
            KERN_ABORTED => Err(super::SleepError::Interrupted),
            code => Err(super::SleepError::Failed(format!(
                "mach_wait_until failed with kern_return {code}"
            ))),
        }
    }

    pub(super) fn cleanup(state: &mut State) {
        if state.time_constraint {
            // SAFETY: `pthread_self` returns the calling thread.
            let thread = unsafe { pthread_self() };
            // SAFETY: the thread port names the calling thread; the standard
            // policy has no policy data.
            let thread_port = unsafe { pthread_mach_thread_np(thread) };
            // SAFETY: the standard policy has no policy data and count zero.
            let result = unsafe {
                thread_policy_set(
                    thread_port,
                    THREAD_STANDARD_POLICY,
                    std::ptr::null(),
                    THREAD_STANDARD_POLICY_COUNT,
                )
            };
            if result != KERN_SUCCESS {
                eprintln!(
                    "warning: cannot restore the standard thread policy: kern_return {result}"
                );
            }
            // Applying the time-constraint policy made the kernel remove the
            // thread's QoS policy, and `pthread_set_qos_class_self_np`
            // refuses to re-create it. The captured class cannot be restored.
            return;
        }
        // The failed time-constraint set kept the standard policy and
        // restored the captured QoS, and the fallback replaced that class.
        // Restoring the captured class is therefore valid here.
        if state.captured_qos != QOS_CLASS_UNSPECIFIED {
            // SAFETY: the call changes only the calling thread's QoS class.
            let result = unsafe {
                pthread_set_qos_class_self_np(state.captured_qos, state.captured_relative_priority)
            };
            if result != 0 {
                eprintln!("warning: cannot restore the captured QoS class: error {result}");
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod platform {
    /// Other hosts have no opt-in scheduling facility.
    pub(super) struct State;

    pub(super) fn enter(
        _priority: u8,
        _cpu: Option<u32>,
        _period_ns: u64,
    ) -> Result<super::Entered, String> {
        Err("--rt is only supported on Linux, Windows, and macOS".into())
    }

    pub(super) fn background(_affinity: Option<u64>) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn now_ns() -> Result<u64, String> {
        Err("--rt is only supported on Linux, Windows, and macOS".into())
    }

    pub(super) fn sleep_until(_state: &State, _deadline_ns: u64) -> Result<(), super::SleepError> {
        Err(super::SleepError::Failed(
            "--rt is only supported on Linux, Windows, and macOS".into(),
        ))
    }

    pub(super) fn cleanup(_state: &mut State) {}
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
    fn deadline_overflow_is_reported() {
        assert!(advance_deadline(u64::MAX, 1, 0).is_err());
        assert!(advance_deadline(0, 1, u64::MAX).is_err());
        assert!(advance_deadline(0, 1 << 63, u64::MAX).is_err());
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
                sched: "fifo",
                cpu: Some(3),
                started_ns: 123,
            };
            let mut writer = MetricsWriter::open(&path, None).expect("open");
            writer.write_header(header).expect("header");
            writer.record(&CycleSample {
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
            });
            writer.record(&CycleSample {
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
            });
            writer.finish().expect("finish");
        }
        let text = std::fs::read_to_string(&path).expect("read metrics");
        let _ = std::fs::remove_file(&path);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "header, two cycles, and summary: {text:?}");
        assert!(lines[0].contains("\"record\":\"header\""));
        assert!(lines[0].contains("\"protocol\":\"stc-rt-metrics/1\""));
        assert!(lines[0].contains("\"build_id\":\"deadbeef\""));
        assert!(lines[0].contains("\"sched\":\"fifo\""));
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
        assert!(lines[3].contains("\"dropped_records\":0"));
    }

    #[test]
    fn absent_optional_metrics_fields_encode_as_null() {
        let mut header = String::new();
        format_header(
            &mut header,
            &MetricsHeader {
                build_id: None,
                cycle_time_ns: 1_000_000,
                priority: 0,
                sched: "qos",
                cpu: None,
                started_ns: 0,
            },
        );
        assert!(header.contains("\"build_id\":null"), "{header}");
        assert!(header.contains("\"cpu\":null"), "{header}");
        let mut cycle = String::new();
        format_cycle(
            &mut cycle,
            &CycleSample {
                cycle: 1,
                generation: None,
                deadline_ns: 0,
                wake_ns: 0,
                scan_start_ns: 0,
                scan_end_ns: 0,
                wake_lateness_ns: 0,
                execution_ns: 0,
                scan_overrun_ns: 0,
                missed_deadline: false,
                skipped_periods: 0,
                faults: None,
                allocations: None,
            },
        );
        assert!(cycle.contains("\"generation\":null"), "{cycle}");
        assert!(cycle.contains("\"minor_faults\":null"), "{cycle}");
        assert!(cycle.contains("\"major_faults\":null"), "{cycle}");
        assert!(cycle.contains("\"allocations\":null"), "{cycle}");
        assert!(cycle.contains("\"frees\":null"), "{cycle}");
    }

    #[test]
    fn metrics_file_creation_failure_is_reported() {
        let path = std::env::temp_dir()
            .join(format!("stc-rt-missing-{}", std::process::id()))
            .join("metrics.jsonl");
        match MetricsWriter::open(&path, None) {
            Ok(_) => panic!("a missing parent directory must be rejected"),
            Err(error) => assert!(
                error.contains("cannot create the RT metrics file"),
                "{error}"
            ),
        }
    }

    fn metrics_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "stc-rt-metrics-{name}-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        ))
    }

    fn cycle_sample(cycle: u64, skipped_periods: u64) -> CycleSample {
        CycleSample {
            cycle,
            generation: Some(7),
            deadline_ns: 123,
            wake_ns: 124,
            scan_start_ns: 124,
            scan_end_ns: 200,
            wake_lateness_ns: 1,
            execution_ns: 76,
            scan_overrun_ns: 0,
            missed_deadline: false,
            skipped_periods,
            faults: None,
            allocations: None,
        }
    }

    fn metrics_header() -> MetricsHeader {
        MetricsHeader {
            build_id: Some("deadbeef".into()),
            cycle_time_ns: 1_000_000,
            priority: 90,
            sched: "fifo",
            cpu: Some(3),
            started_ns: 123,
        }
    }

    #[test]
    fn metrics_writer_reports_dropped_records() {
        let path = metrics_path("dropped");
        let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut writer =
            MetricsWriter::open_with(&path, 4, None, Some(gate.clone())).expect("open");
        writer.write_header(metrics_header()).expect("header");
        // The writer is parked behind the gate, so the header and three cycle
        // records fill the four slots and the fourth cycle record is dropped.
        for cycle in 1..=4 {
            writer.record(&cycle_sample(cycle, 0));
        }
        gate.wait();
        writer.finish().expect("finish");
        let text = std::fs::read_to_string(&path).expect("read metrics");
        let _ = std::fs::remove_file(&path);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 5, "header, three cycles, summary: {text:?}");
        let cycles = lines
            .iter()
            .filter(|line| line.contains("\"record\":\"cycle\""))
            .count();
        assert_eq!(cycles, 3);
        let summary = lines.last().expect("summary");
        assert!(summary.contains("\"dropped_records\":1"), "{summary}");
        assert!(summary.contains("\"cycles\":4"), "{summary}");
    }

    #[test]
    fn metrics_writer_finish_drains_without_hanging() {
        let path = metrics_path("shutdown");
        let mut writer = MetricsWriter::open(&path, None).expect("open");
        writer.write_header(metrics_header()).expect("header");
        for cycle in 1..=100 {
            writer.record(&cycle_sample(cycle, 0));
        }
        let (done_sender, done_receiver) = std::sync::mpsc::channel();
        let finish_thread = std::thread::spawn(move || {
            let _ = done_sender.send(writer.finish());
        });
        let result = done_receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("finish did not complete");
        assert!(result.is_ok(), "{result:?}");
        finish_thread.join().expect("finish thread");
        let text = std::fs::read_to_string(&path).expect("read metrics");
        let _ = std::fs::remove_file(&path);
        let cycles = text
            .lines()
            .filter(|line| line.contains("\"record\":\"cycle\""))
            .count();
        assert_eq!(cycles, 100);
        assert!(
            text.lines()
                .last()
                .expect("summary")
                .contains("\"dropped_records\":0")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn proc_stat_fault_positions_are_parsed() {
        let stat = "1234 (stc worker) S 1 2 3 4 5 6 7 100 200 300 400 0 0 0";
        assert_eq!(parse_faults(stat), Some((7, 200)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn malformed_process_stat_has_no_faults() {
        assert_eq!(parse_faults("1234 (stc) S 1 2"), None);
        assert_eq!(parse_faults("no closing parenthesis"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fault_counts_read_the_live_process_stat() {
        assert!(
            fault_counts().is_some(),
            "/proc/self/stat exposes the page-fault counters"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires Linux RT privileges; run explicitly with --ignored on the RT host"]
    fn rt_mode_enters_and_sleeps_to_an_absolute_deadline() {
        let rt = match RtRuntime::enter(90, None, 1_000_000) {
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

    #[cfg(target_os = "linux")]
    mod linux_rt {
        use super::super::platform::State;
        use super::super::{Entered, MetricsWriter, RtRuntime};
        use super::{metrics_header, metrics_path};

        #[test]
        fn metadata_accessors_and_absolute_sleep_are_available_without_privileges() {
            let rt = RtRuntime::from_entered(Entered {
                background_affinity: Some(0b10),
                sched: "fifo",
                priority: 90,
                state: State,
            });
            assert_eq!(rt.background_affinity(), Some(0b10));
            assert_eq!(rt.sched(), "fifo");
            assert_eq!(rt.priority(), 90);
            let started = rt.now_ns().expect("CLOCK_MONOTONIC");
            let deadline = started.saturating_add(1_000_000);
            rt.sleep_until(deadline).expect("absolute sleep");
            let finished = rt.now_ns().expect("CLOCK_MONOTONIC");
            assert!(finished >= deadline, "the absolute sleep returned early");
            assert!(finished >= started, "the monotonic clock went backwards");
            drop(rt);
        }

        #[test]
        fn metrics_writer_reports_the_background_affinity_result() {
            let path = metrics_path("background-affinity");
            match MetricsWriter::open(&path, Some(0b1)) {
                Ok(mut writer) => {
                    writer.write_header(metrics_header()).expect("header");
                    writer.finish().expect("finish");
                }
                Err(message) => assert!(message.contains("metrics writer"), "{message}"),
            }
            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn a_cpu_outside_the_supported_mask_is_rejected() {
            match RtRuntime::enter(90, Some(64), 1_000_000) {
                Ok(_) => panic!("--rt-cpu 64 must be rejected"),
                Err(message) => assert!(message.contains("0..=63"), "{message}"),
            }
        }

        #[test]
        fn entering_without_a_cpu_reports_the_fifo_result() {
            match RtRuntime::enter(90, None, 1_000_000) {
                Ok(rt) => {
                    assert_eq!(rt.sched(), "fifo");
                    assert_eq!(rt.priority(), 90);
                }
                Err(message) => assert!(
                    message.contains("SCHED_FIFO"),
                    "the error names the denied scheduling change: {message}"
                ),
            }
        }

        #[test]
        fn entering_with_a_cpu_reports_the_pin_result() {
            match RtRuntime::enter(90, Some(0), 1_000_000) {
                Ok(rt) => {
                    assert_eq!(rt.sched(), "fifo");
                    assert!(
                        rt.background_affinity().is_some(),
                        "the writer keeps the CPUs left after the pin"
                    );
                }
                Err(message) => assert!(
                    message.contains("SCHED_FIFO")
                        || message.contains("affinity")
                        || message.contains("CPU"),
                    "the error names the denied pin or scheduling change: {message}"
                ),
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn rt_mode_rejects_the_linux_only_cpu_option() {
        match RtRuntime::enter(90, Some(0), 1_000_000) {
            Ok(_) => panic!("--rt-cpu must be rejected on this host"),
            Err(message) => assert!(
                message.contains("only supported on Linux"),
                "the error names the Linux-only option: {message}"
            ),
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn rt_mode_writes_metrics_on_non_linux_hosts() {
        let path = metrics_path("host");
        let rt = RtRuntime::enter(90, None, 1_000_000).expect("enter RT mode");
        let started = rt.now_ns().expect("monotonic clock");
        let mut writer = MetricsWriter::open(&path, rt.background_affinity()).expect("open");
        writer
            .write_header(MetricsHeader {
                build_id: Some("deadbeef".into()),
                cycle_time_ns: 1_000_000,
                priority: rt.priority(),
                sched: rt.sched(),
                cpu: None,
                started_ns: started,
            })
            .expect("header");
        let mut previous = started;
        for cycle in 1..=3 {
            let deadline = previous.saturating_add(1_000_000);
            rt.sleep_until(deadline).expect("absolute sleep");
            let wake = rt.now_ns().expect("monotonic clock");
            assert!(wake >= deadline, "the absolute sleep returned early");
            assert!(wake >= previous, "the monotonic clock went backwards");
            previous = wake;
            writer.record(&CycleSample {
                cycle,
                generation: Some(7),
                deadline_ns: deadline,
                wake_ns: wake,
                scan_start_ns: wake,
                scan_end_ns: wake,
                wake_lateness_ns: i64::try_from(wake.saturating_sub(deadline)).unwrap_or(0),
                execution_ns: 0,
                scan_overrun_ns: 0,
                missed_deadline: false,
                skipped_periods: 0,
                faults: None,
                allocations: None,
            });
        }
        writer.finish().expect("finish");
        drop(rt);
        let text = std::fs::read_to_string(&path).expect("read metrics");
        let _ = std::fs::remove_file(&path);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 5, "header, three cycles, summary: {text:?}");
        assert!(lines[0].contains("\"record\":\"header\""));
        assert!(lines[0].contains("\"protocol\":\"stc-rt-metrics/1\""));
        assert!(
            lines[0].contains("\"sched\":\"mmcss\"")
                || lines[0].contains("\"sched\":\"time_critical\"")
                || lines[0].contains("\"sched\":\"time_constraint\"")
                || lines[0].contains("\"sched\":\"qos\""),
            "the header names the applied facility: {}",
            lines[0]
        );
        assert!(lines[3].contains("\"record\":\"cycle\""));
        assert!(lines[4].contains("\"record\":\"summary\""));
        assert!(lines[4].contains("\"cycles\":3"), "{}", lines[4]);
        assert!(lines[4].contains("\"missed_deadlines\":0"), "{}", lines[4]);
    }

    #[cfg(windows)]
    mod windows_rt {
        use super::super::RtRuntime;
        use std::ffi::c_void;

        const THREAD_PRIORITY_TIME_CRITICAL: i32 = 15;

        unsafe extern "system" {
            fn GetCurrentThread() -> *mut c_void;
            fn GetThreadPriority(thread: *mut c_void) -> i32;
        }

        #[test]
        fn rt_mode_sets_time_critical_priority_and_restores_it_on_drop() {
            std::thread::spawn(|| {
                // SAFETY: the pseudo-handle names the calling thread.
                let before = unsafe { GetThreadPriority(GetCurrentThread()) };
                assert_ne!(before, THREAD_PRIORITY_TIME_CRITICAL);
                let rt = RtRuntime::enter(90, None, 1_000_000).expect("enter RT mode");
                assert_eq!(rt.priority(), 0, "the header priority is zero");
                assert!(
                    matches!(rt.sched(), "mmcss" | "time_critical"),
                    "the applied facility is recorded: {}",
                    rt.sched()
                );
                // SAFETY: the pseudo-handle names the calling thread.
                let during = unsafe { GetThreadPriority(GetCurrentThread()) };
                if rt.sched() == "time_critical" {
                    assert_eq!(
                        during, THREAD_PRIORITY_TIME_CRITICAL,
                        "the cycle thread runs at time-critical priority"
                    );
                } else {
                    // MMCSS owns the final priority of a registered thread;
                    // it may tune the value we set before registration.
                    assert!(
                        during > before,
                        "MMCSS raised the cycle thread priority: {before} -> {during}"
                    );
                }
                let started = rt.now_ns().expect("QPC");
                let deadline = started.saturating_add(1_000_000);
                rt.sleep_until(deadline).expect("waitable timer");
                let finished = rt.now_ns().expect("QPC");
                assert!(finished >= deadline, "the timer wait returned early");
                assert!(finished >= started, "the QPC clock went backwards");
                drop(rt);
                // SAFETY: the pseudo-handle names the calling thread.
                let after = unsafe { GetThreadPriority(GetCurrentThread()) };
                assert_eq!(after, before, "Drop restores the previous priority");
            })
            .join()
            .expect("RT thread");
        }
    }

    #[cfg(target_os = "macos")]
    mod macos_rt {
        use super::super::RtRuntime;
        use std::os::raw::c_int;

        const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
        const QOS_CLASS_UTILITY: u32 = 0x11;
        const QOS_CLASS_UNSPECIFIED: u32 = 0;
        const THREAD_TIME_CONSTRAINT_POLICY: u32 = 2;
        const THREAD_TIME_CONSTRAINT_POLICY_COUNT: u32 = 4;

        #[repr(C)]
        struct TimeConstraintPolicy {
            period: u32,
            computation: u32,
            constraint: u32,
            preemptible: i32,
        }

        unsafe extern "C" {
            fn pthread_self() -> usize;
            fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: c_int) -> c_int;
            fn pthread_get_qos_class_np(
                thread: usize,
                qos_class: *mut u32,
                relative_priority: *mut c_int,
            ) -> c_int;
            fn pthread_mach_thread_np(thread: usize) -> u32;
            fn thread_policy_get(
                thread: u32,
                policy: u32,
                policy_info: *mut i32,
                count: *mut u32,
                get_default: *mut i32,
            ) -> c_int;
        }

        fn read_qos() -> (u32, c_int) {
            let mut qos = QOS_CLASS_UNSPECIFIED;
            let mut relative = 0;
            // SAFETY: the thread is the calling thread and both out pointers
            // are writable.
            let result =
                unsafe { pthread_get_qos_class_np(pthread_self(), &mut qos, &mut relative) };
            assert_eq!(result, 0, "pthread_get_qos_class_np");
            (qos, relative)
        }

        #[test]
        fn rt_mode_applies_a_policy_or_qos_and_restores_it_on_drop() {
            std::thread::spawn(|| {
                // Start from a known class so the restore is observable.
                // SAFETY: the call changes only the calling thread's QoS.
                assert_eq!(
                    unsafe { pthread_set_qos_class_self_np(QOS_CLASS_UTILITY, 0) },
                    0
                );
                let rt = RtRuntime::enter(90, None, 1_000_000).expect("enter RT mode");
                assert_eq!(rt.priority(), 0, "the header priority is zero");
                let rt_sched = rt.sched();
                assert!(
                    matches!(rt_sched, "time_constraint" | "qos"),
                    "the applied facility is recorded: {rt_sched}"
                );
                if rt_sched == "time_constraint" {
                    // SAFETY: the thread port names the calling thread and the
                    // policy struct is writable.
                    let thread = unsafe { pthread_mach_thread_np(pthread_self()) };
                    let mut policy = TimeConstraintPolicy {
                        period: 0,
                        computation: 0,
                        constraint: 0,
                        preemptible: 0,
                    };
                    let mut count = THREAD_TIME_CONSTRAINT_POLICY_COUNT;
                    let mut get_default = 0;
                    // SAFETY: the thread port is valid for the call and the
                    // policy struct is writable.
                    let result = unsafe {
                        thread_policy_get(
                            thread,
                            THREAD_TIME_CONSTRAINT_POLICY,
                            (&mut policy as *mut TimeConstraintPolicy).cast::<i32>(),
                            &mut count,
                            &mut get_default,
                        )
                    };
                    assert_eq!(result, 0, "the time-constraint policy is readable");
                    assert_eq!(count, THREAD_TIME_CONSTRAINT_POLICY_COUNT);
                    assert_eq!(policy.preemptible, 1);
                    assert!(policy.period >= policy.constraint);
                    assert!(policy.constraint >= policy.computation);
                } else {
                    let (qos, _) = read_qos();
                    assert_eq!(
                        qos, QOS_CLASS_USER_INTERACTIVE,
                        "the fallback applies the interactive QoS class"
                    );
                }
                let started = rt.now_ns().expect("CLOCK_MONOTONIC");
                let deadline = started.saturating_add(1_000_000);
                rt.sleep_until(deadline).expect("mach_wait_until");
                let finished = rt.now_ns().expect("CLOCK_MONOTONIC");
                assert!(finished >= deadline, "the absolute sleep returned early");
                assert!(finished >= started, "the monotonic clock went backwards");
                drop(rt);
                let (qos, _) = read_qos();
                if rt_sched == "time_constraint" {
                    assert_eq!(
                        qos, QOS_CLASS_UNSPECIFIED,
                        "the kernel removed the QoS policy when the time-constraint \
                         policy was applied and userspace cannot restore it"
                    );
                    // SAFETY: the thread port names the calling thread and the
                    // policy struct is writable.
                    let thread = unsafe { pthread_mach_thread_np(pthread_self()) };
                    let mut policy = TimeConstraintPolicy {
                        period: 0,
                        computation: 0,
                        constraint: 0,
                        preemptible: 0,
                    };
                    let mut count = THREAD_TIME_CONSTRAINT_POLICY_COUNT;
                    let mut get_default = 0;
                    // SAFETY: the thread port is valid for the call and the
                    // policy struct is writable.
                    let result = unsafe {
                        thread_policy_get(
                            thread,
                            THREAD_TIME_CONSTRAINT_POLICY,
                            (&mut policy as *mut TimeConstraintPolicy).cast::<i32>(),
                            &mut count,
                            &mut get_default,
                        )
                    };
                    assert_eq!(result, 0, "the policy state is readable");
                    assert_ne!(get_default, 0, "Drop restores the standard thread policy");
                } else {
                    assert_eq!(
                        qos, QOS_CLASS_UTILITY,
                        "Drop restores the captured QoS class in the fallback path"
                    );
                }
            })
            .join()
            .expect("RT thread");
        }
    }
}
