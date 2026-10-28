//! Breakpoints, stops, stepping, and stopped-frame capture (Debug and
//! Monitoring §14.2, §14.3, §16.4, §16.7–§16.9, D-181).
//!
//! The cycle thread enters [`debug_check_entry`] through the host API service
//! 13. When no session is installed the entry only performs a lock-free
//! lookup and returns. With a session it validates the checkpoint against the
//! validated checkpoint table, matches breakpoints and step plans, captures an
//! immutable stack/local snapshot, publishes a `stopped` notice, and parks
//! until the protocol thread requests continue/step, the pause bound expires,
//! or the session ends. While parked it keeps servicing the typed debug
//! engine through the raw cycle context installed by the host, so reads and
//! watches are answered without exposing a raw stack address.
//!
//! All unsafe operations are confined to this module's register capture,
//! frame-pointer chain reads, and the cycle-context pointer. The protocol
//! thread reads only immutable captured snapshots.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use stc_got::Manifest;
use stc_got::digest::{Digest32, GotId};
use stc_got::error::{Error, Result, codes};
use stc_got::layout::{Target, TypeKey};

use crate::debug::{
    DebugEndReason, DebugEngine, DebugSource, DebugValue, TypedValue, decode_type_value,
};
use crate::frames::{FrameFunction, FrameTable};
use crate::module_info::DebugCheckpointRecord;
use crate::{ModuleInfo, RootRegistry};

/// Maximum number of captured stack frames in one stop snapshot.
pub(crate) const MAX_STACK_FRAMES: usize = 1024;
/// Maximum number of breakpoints in one session.
pub(crate) const MAX_BREAKPOINTS: usize = 1024;
/// Maximum number of local bytes copied into one stop snapshot.
pub(crate) const MAX_LOCAL_COPY_BYTES: usize = 1024 * 1024;
/// Maximum address span walked from one frame pointer chain.
const MAX_STACK_WALK_BYTES: usize = 64 * 1024 * 1024;
/// How often a parked cycle thread services the debug engine.
const PARK_POLL: Duration = Duration::from_millis(5);

// ---------------------------------------------------------------------------
// Validated tables
// ---------------------------------------------------------------------------

/// One function code range, used to map a return address to a manifest
/// function identity.
#[derive(Clone, Debug)]
pub(crate) struct CodeRange {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) function: GotId,
    pub(crate) symbol: String,
}

/// The validated checkpoint and stopped-frame tables for one generation.
pub(crate) struct DebugTables {
    records: Vec<DebugCheckpointRecord>,
    ranges: Vec<CodeRange>,
    checkpoint_ranges: BTreeMap<GotId, (u32, u32)>,
    pub(crate) frames: Option<Arc<FrameTable>>,
    pub(crate) frames_error: Option<String>,
    pub(crate) stepping: bool,
    pub(crate) manifest: Manifest,
    pointer_width: u8,
}

impl std::fmt::Debug for DebugTables {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DebugTables")
            .field("checkpoints", &self.records.len())
            .field("ranges", &self.ranges.len())
            .field("frames", &self.frames.is_some())
            .field("stepping", &self.stepping)
            .finish()
    }
}

impl DebugTables {
    /// Validates the checkpoint table together with the additive frame
    /// sidecar. An unusable frame table never fails the module: it disables
    /// `stack.list`/`frame.read` (and stepping) only.
    pub(crate) fn new(
        checkpoints: Vec<DebugCheckpointRecord>,
        frames: Option<Arc<FrameTable>>,
        frames_error: Option<String>,
        manifest: &Manifest,
        functions: &BTreeMap<GotId, usize>,
        target: &Target,
    ) -> Self {
        let mut checkpoint_ranges: BTreeMap<GotId, (u32, u32)> = BTreeMap::new();
        for record in &checkpoints {
            let range = checkpoint_ranges
                .entry(record.function)
                .or_insert((record.id, record.id));
            range.0 = range.0.min(record.id);
            range.1 = range.1.max(record.id);
        }
        let mut tables = Self {
            records: checkpoints,
            ranges: Vec::new(),
            checkpoint_ranges,
            frames: None,
            frames_error,
            stepping: false,
            manifest: manifest.clone(),
            pointer_width: target.pointer_width,
        };
        let Some(table) = frames else {
            return tables;
        };
        match tables.validate_frames(table.as_ref(), functions, target) {
            Ok(()) => {
                tables.frames = Some(table);
                tables.stepping = true;
            }
            Err(message) => {
                tables.frames_error = Some(message);
            }
        }
        tables
    }

    fn validate_frames(
        &mut self,
        table: &FrameTable,
        functions: &BTreeMap<GotId, usize>,
        target: &Target,
    ) -> std::result::Result<(), String> {
        if !target.triple.starts_with("x86_64-") || self.pointer_width != 8 {
            return Err(
                "stopped-frame rules are not defined for the active target architecture".into(),
            );
        }
        if self.records.is_empty() {
            return Err("a stopped-frame table was supplied without checkpoints".into());
        }
        let checkpoint_count =
            u32::try_from(self.records.len()).map_err(|_| "too many checkpoints")?;
        let mut ranges = Vec::with_capacity(table.functions.len());
        for (got_id, function) in &table.functions {
            let Some(address) = functions.get(got_id).copied() else {
                return Err("a stopped-frame function is not a bound manifest function".to_owned());
            };
            if address == 0 {
                return Err("a stopped-frame function has no bound code address".to_owned());
            }
            let size = usize::try_from(function.code_size)
                .map_err(|_| "a stopped-frame code size is invalid")?;
            let end = address
                .checked_add(size)
                .ok_or_else(|| "a stopped-frame code range overflows".to_owned())?;
            for local in &function.locals {
                if local.live_first > local.live_last
                    || local.live_last > checkpoint_count
                    || local.live_first == 0
                {
                    return Err(
                        "a stopped-frame local live range is outside the checkpoint table".into(),
                    );
                }
            }
            ranges.push(CodeRange {
                start: address,
                end,
                function: *got_id,
                symbol: function.symbol.clone(),
            });
        }
        ranges.sort_by_key(|range| range.start);
        for pair in ranges.windows(2) {
            if pair[0].end > pair[1].start {
                return Err("stopped-frame code ranges overlap".into());
            }
        }
        self.ranges = ranges;
        Ok(())
    }

    /// Returns one validated checkpoint record.
    pub(crate) fn checkpoint(&self, id: u32) -> Option<&DebugCheckpointRecord> {
        let index = usize::try_from(id.checked_sub(1)?).ok()?;
        let record = self.records.get(index)?;
        (record.id == id).then_some(record)
    }

    /// Maps a return address to the enclosing manifest function.
    pub(crate) fn function_at(&self, address: usize) -> Option<&CodeRange> {
        let index = self.ranges.partition_point(|range| range.start <= address);
        let range = self.ranges.get(index.checked_sub(1)?)?;
        (address < range.end).then_some(range)
    }

    fn frame_function(&self, got_id: GotId) -> Option<&FrameFunction> {
        self.frames.as_ref()?.functions.get(&got_id)
    }

    /// Returns the inclusive checkpoint ID range emitted by one function.
    pub(crate) fn checkpoint_range(&self, got_id: GotId) -> Option<(u32, u32)> {
        self.checkpoint_ranges.get(&got_id).copied()
    }
}

// ---------------------------------------------------------------------------
// Breakpoints
// ---------------------------------------------------------------------------

/// A breakpoint request target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BreakpointTarget {
    /// A relative source location.
    Source {
        file: String,
        line: u32,
        column: u32,
    },
    /// A direct checkpoint identity.
    Checkpoint { checkpoint_id: u32 },
}

/// A resolved source location reported for a breakpoint or stack frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourcePosition {
    pub(crate) file: String,
    pub(crate) line: u32,
    pub(crate) column: u32,
}

#[derive(Clone, Debug)]
struct Breakpoint {
    id: u64,
    target: BreakpointTarget,
    bound: Option<u32>,
    source: Option<SourcePosition>,
}

/// The protocol-facing description of one breakpoint.
#[derive(Clone, Debug)]
pub(crate) struct BreakpointInfo {
    pub(crate) id: u64,
    pub(crate) target: BreakpointTarget,
    pub(crate) bound: bool,
    pub(crate) checkpoints: Vec<u32>,
    pub(crate) source: Option<SourcePosition>,
}

/// Outcome of a pause request.
pub(crate) enum PauseOutcome {
    /// The cycle is running; a pause was requested.
    Requested,
    /// The cycle is already stopped.
    Stopped(u64),
}

/// Stack frame mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepMode {
    Over,
    Into,
    Out,
}

impl StepMode {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "over" => Some(Self::Over),
            "into" => Some(Self::Into),
            "out" => Some(Self::Out),
            _ => None,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Over => "over",
            Self::Into => "into",
            Self::Out => "out",
        }
    }
}

#[derive(Clone, Debug)]
struct StepPlan {
    mode: StepMode,
    depth: usize,
}

/// Why a stop was reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StopReason {
    Breakpoint,
    Step,
    Pause,
}

impl StopReason {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Breakpoint => "breakpoint",
            Self::Step => "step",
            Self::Pause => "pause",
        }
    }
}

/// Why a stop was resumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResumeCause {
    Continue,
    Step,
}

/// One protocol-visible stop event.
#[derive(Clone, Debug)]
pub(crate) enum StopEvent {
    Stopped {
        stop_id: u64,
        reason: StopReason,
        checkpoint_id: u32,
        source: SourcePosition,
        breakpoint_id: Option<u64>,
        generation: u64,
        build_id: GotId,
        cycle: u64,
    },
    Resumed {
        stop_id: u64,
        cause: ResumeCause,
        cycle: u64,
    },
}

impl StopEvent {
    pub(crate) fn is_step(&self) -> bool {
        matches!(
            self,
            Self::Resumed {
                cause: ResumeCause::Step,
                ..
            }
        )
    }
}

// ---------------------------------------------------------------------------
// Captured stop snapshot
// ---------------------------------------------------------------------------

/// One captured local value or its honest unavailability.
#[derive(Clone, Debug)]
pub(crate) struct CapturedLocal {
    pub(crate) type_key: String,
    pub(crate) value: Option<DebugValue>,
    pub(crate) reason: Option<String>,
}

/// One captured stack frame.
#[derive(Clone, Debug)]
pub(crate) struct CapturedFrame {
    pub(crate) function: Option<GotId>,
    pub(crate) symbol: Option<String>,
    pub(crate) source: Option<SourcePosition>,
    pub(crate) optimized_out: bool,
    pub(crate) locals: BTreeMap<String, CapturedLocal>,
}

/// One stack frame returned by `stack.list`.
#[derive(Clone, Debug)]
pub(crate) struct StackFrameView {
    pub(crate) index: usize,
    pub(crate) function: Option<GotId>,
    pub(crate) symbol: Option<String>,
    pub(crate) source: Option<SourcePosition>,
    pub(crate) optimized_out: bool,
}

/// A typed local returned by `frame.read`.
#[derive(Clone, Debug)]
pub(crate) struct LocalView {
    pub(crate) type_key: String,
    pub(crate) value: DebugValue,
    pub(crate) generation: u64,
    pub(crate) build_id: GotId,
    pub(crate) cycle: u64,
    pub(crate) view: crate::debug_view::SplitView,
}

/// A stop snapshot summary for `session.status`.
#[derive(Clone, Debug)]
pub(crate) struct StopStatus {
    pub(crate) stop_id: u64,
    pub(crate) reason: StopReason,
    pub(crate) checkpoint_id: u32,
    pub(crate) source: SourcePosition,
    pub(crate) breakpoint_id: Option<u64>,
    pub(crate) cycle: u64,
}

/// The status of one debug stop session.
#[derive(Clone, Debug)]
pub(crate) struct SessionStatus {
    pub(crate) stop: Option<StopStatus>,
    pub(crate) pause_remaining_ms: Option<u64>,
    pub(crate) breakpoints: usize,
}

struct StopState {
    id: u64,
    reason: StopReason,
    checkpoint: DebugCheckpointRecord,
    breakpoint_id: Option<u64>,
    cycle: u64,
    deadline: Instant,
    frames: Vec<CapturedFrame>,
    truncated: bool,
    frames_available: bool,
    depth: Option<usize>,
}

struct Control {
    breakpoints: BTreeMap<u64, Breakpoint>,
    next_breakpoint: u64,
    stop: Option<StopState>,
    next_stop: u64,
    step: Option<StepPlan>,
    pause_requested: bool,
    resume: Option<ResumeCause>,
    ending: bool,
    mismatch: Option<String>,
    events: VecDeque<StopEvent>,
}

/// One installed stop session.
pub(crate) struct SessionStop {
    tables: Arc<DebugTables>,
    generation: u64,
    build_id: GotId,
    timeout: Duration,
    control: Mutex<Control>,
    wake: Condvar,
}

impl std::fmt::Debug for SessionStop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionStop")
            .field("generation", &self.generation)
            .field("timeout", &self.timeout)
            .finish()
    }
}

static SESSION: Mutex<Option<Arc<SessionStop>>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Installs a stop session, replacing an ended leftover if one exists.
///
/// The caller owns the returned handle and must call [`uninstall`] when the
/// protocol session ends.
pub(crate) fn install(
    tables: Arc<DebugTables>,
    generation: u64,
    build_id: GotId,
    timeout: Duration,
) -> Arc<SessionStop> {
    let session = Arc::new(SessionStop {
        tables,
        generation,
        build_id,
        timeout,
        control: Mutex::new(Control {
            breakpoints: BTreeMap::new(),
            next_breakpoint: 0,
            stop: None,
            next_stop: 0,
            step: None,
            pause_requested: false,
            resume: None,
            ending: false,
            mismatch: None,
            events: VecDeque::new(),
        }),
        wake: Condvar::new(),
    });
    let previous = {
        let mut slot = lock(&SESSION);
        slot.replace(Arc::clone(&session))
    };
    if let Some(previous) = previous {
        previous.end();
    }
    session
}

/// Uninstalls a session handle and wakes a parked cycle thread.
pub(crate) fn uninstall(session: &Arc<SessionStop>) {
    let removed = {
        let mut slot = lock(&SESSION);
        if slot
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, session))
        {
            slot.take()
        } else {
            None
        }
    };
    session.end();
    drop(removed);
}

/// Ends the installed session without touching the debug engine.
///
/// Used by the host when a swap, shutdown, or target exit already ended debug
/// state on the cycle thread, and as a defensive wake for a parked cycle.
pub(crate) fn abort_session() {
    let removed = lock(&SESSION).take();
    if let Some(session) = removed {
        session.end();
    }
}

fn current() -> Option<Arc<SessionStop>> {
    lock(&SESSION).clone()
}

impl SessionStop {
    fn end(&self) {
        let mut control = lock(&self.control);
        control.ending = true;
        control.stop = None;
        control.step = None;
        control.pause_requested = false;
        control.resume = None;
        control.breakpoints.clear();
        control.events.clear();
        self.wake.notify_all();
    }

    /// Returns whether the session is ending.
    pub(crate) fn ending(&self) -> bool {
        lock(&self.control).ending
    }

    /// Adds a source or checkpoint breakpoint.
    pub(crate) fn set_breakpoint(&self, target: BreakpointTarget) -> Result<BreakpointInfo> {
        let mut control = lock(&self.control);
        if control.ending {
            return Err(Error::new(
                codes::DEBUG_SESSION,
                "the debug session is ending",
            ));
        }
        if let Some(message) = &control.mismatch {
            return Err(Error::new(codes::DEBUG_METADATA, message.clone()));
        }
        if control.breakpoints.len() >= MAX_BREAKPOINTS {
            return Err(Error::new(
                codes::DEBUG_LIMIT,
                format!("the breakpoint limit of {MAX_BREAKPOINTS} is reached"),
            ));
        }
        let bound = match &target {
            BreakpointTarget::Source { file, line, column } => {
                self.tables.bind_source(file, *line, *column)
            }
            BreakpointTarget::Checkpoint { checkpoint_id } => {
                let Some(record) = self.tables.checkpoint(*checkpoint_id) else {
                    return Err(Error::new(
                        codes::DEBUG_UNKNOWN,
                        format!("checkpoint {checkpoint_id} is not in the validated table"),
                    ));
                };
                Some(record.id)
            }
        };
        let source = bound
            .and_then(|id| self.tables.checkpoint(id))
            .map(|record| SourcePosition {
                file: record.file.clone(),
                line: record.line,
                column: record.column,
            })
            .or_else(|| match &target {
                BreakpointTarget::Source { file, line, column } => Some(SourcePosition {
                    file: file.clone(),
                    line: *line,
                    column: *column,
                }),
                BreakpointTarget::Checkpoint { .. } => None,
            });
        control.next_breakpoint = control.next_breakpoint.saturating_add(1).max(1);
        let id = control.next_breakpoint;
        let info = BreakpointInfo {
            id,
            target: target.clone(),
            bound: bound.is_some(),
            checkpoints: bound.into_iter().collect(),
            source,
        };
        control.breakpoints.insert(
            id,
            Breakpoint {
                id,
                target,
                bound,
                source: info.source.clone(),
            },
        );
        Ok(info)
    }

    /// Returns one breakpoint description.
    pub(crate) fn breakpoint(&self, id: u64) -> Option<BreakpointInfo> {
        lock(&self.control)
            .breakpoints
            .get(&id)
            .map(breakpoint_info)
    }

    /// Removes one breakpoint. Returns `false` when the ID is unknown.
    pub(crate) fn clear_breakpoint(&self, id: u64) -> bool {
        lock(&self.control).breakpoints.remove(&id).is_some()
    }

    /// Lists breakpoints in ID order.
    pub(crate) fn breakpoints(&self) -> Vec<BreakpointInfo> {
        lock(&self.control)
            .breakpoints
            .values()
            .map(breakpoint_info)
            .collect()
    }

    /// Resumes a stopped cycle.
    pub(crate) fn continue_execution(&self, stop_id: Option<u64>) -> Result<u64> {
        let mut control = lock(&self.control);
        if control.ending {
            return Err(Error::new(
                codes::DEBUG_SESSION,
                "the debug session is ending",
            ));
        }
        let Some(stop) = control.stop.as_ref() else {
            return Err(Error::new(
                codes::DEBUG_STOP,
                "the cycle is not stopped; continue requires a current stop_id",
            ));
        };
        if stop_id.is_some_and(|id| id != stop.id) {
            return Err(Error::new(
                codes::DEBUG_STOP,
                "the stop_id is stale; read the current stop_id from session.status",
            ));
        }
        let id = stop.id;
        let cycle = stop.cycle;
        control.stop = None;
        control.step = None;
        control.pause_requested = false;
        control.resume = Some(ResumeCause::Continue);
        control.events.push_back(StopEvent::Resumed {
            stop_id: id,
            cause: ResumeCause::Continue,
            cycle,
        });
        self.wake.notify_all();
        Ok(id)
    }

    /// Arms a step plan and resumes a stopped cycle.
    pub(crate) fn step(&self, mode: StepMode, stop_id: u64) -> Result<u64> {
        let mut control = lock(&self.control);
        if control.ending {
            return Err(Error::new(
                codes::DEBUG_SESSION,
                "the debug session is ending",
            ));
        }
        let Some(stop) = control.stop.as_ref() else {
            return Err(Error::new(
                codes::DEBUG_STOP,
                "the cycle is not stopped; step requires a current stop_id",
            ));
        };
        if stop.id != stop_id {
            return Err(Error::new(
                codes::DEBUG_STOP,
                "the stop_id is stale; read the current stop_id from session.status",
            ));
        }
        if !stop.frames_available {
            return Err(Error::new(
                codes::DEBUG_STOP,
                "stepping requires stopped-frame information, which this build does not provide",
            ));
        }
        let Some(depth) = stop.depth else {
            return Err(Error::new(
                codes::DEBUG_STOP,
                "step depth is unavailable because the frame chain is not walkable",
            ));
        };
        let id = stop.id;
        let cycle = stop.cycle;
        control.step = Some(StepPlan { mode, depth });
        control.stop = None;
        control.pause_requested = false;
        control.resume = Some(ResumeCause::Step);
        control.events.push_back(StopEvent::Resumed {
            stop_id: id,
            cause: ResumeCause::Step,
            cycle,
        });
        self.wake.notify_all();
        Ok(id)
    }

    /// Requests a pause, or reports the current stop.
    pub(crate) fn pause(&self) -> Result<PauseOutcome> {
        let mut control = lock(&self.control);
        if control.ending {
            return Err(Error::new(
                codes::DEBUG_SESSION,
                "the debug session is ending",
            ));
        }
        if let Some(stop) = control.stop.as_ref() {
            return Ok(PauseOutcome::Stopped(stop.id));
        }
        control.pause_requested = true;
        Ok(PauseOutcome::Requested)
    }

    /// Returns the current stop snapshot summary.
    pub(crate) fn status(&self) -> SessionStatus {
        let control = lock(&self.control);
        let now = Instant::now();
        let stop = control.stop.as_ref().map(|stop| StopStatus {
            stop_id: stop.id,
            reason: stop.reason,
            checkpoint_id: stop.checkpoint.id,
            source: SourcePosition {
                file: stop.checkpoint.file.clone(),
                line: stop.checkpoint.line,
                column: stop.checkpoint.column,
            },
            breakpoint_id: stop.breakpoint_id,
            cycle: stop.cycle,
        });
        let pause_remaining_ms = control.stop.as_ref().map(|stop| {
            let remaining = stop.deadline.saturating_duration_since(now);
            u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX)
        });
        SessionStatus {
            stop,
            pause_remaining_ms,
            breakpoints: control.breakpoints.len(),
        }
    }

    /// Lists captured frames for one stop.
    pub(crate) fn stack(&self, stop_id: u64) -> Result<(Vec<StackFrameView>, bool)> {
        let control = lock(&self.control);
        let Some(stop) = current_stop(&control, stop_id)? else {
            return Err(Error::new(
                codes::DEBUG_STOP,
                "the cycle is not stopped; stack.list requires a current stop_id",
            ));
        };
        if !stop.frames_available {
            return Err(Error::new(
                codes::DEBUG_INFO,
                "stopped-frame information is unavailable for this build",
            ));
        }
        let frames = stop
            .frames
            .iter()
            .enumerate()
            .map(|(index, frame)| StackFrameView {
                index,
                function: frame.function,
                symbol: frame.symbol.clone(),
                source: frame.source.clone(),
                optimized_out: frame.optimized_out,
            })
            .collect();
        Ok((frames, stop.truncated))
    }

    /// Reads one typed local from a captured frame.
    pub(crate) fn read_local(&self, stop_id: u64, frame: usize, name: &str) -> Result<LocalView> {
        let control = lock(&self.control);
        let Some(stop) = current_stop(&control, stop_id)? else {
            return Err(Error::new(
                codes::DEBUG_STOP,
                "the cycle is not stopped; frame.read requires a current stop_id",
            ));
        };
        if !stop.frames_available {
            return Err(Error::new(
                codes::DEBUG_INFO,
                "stopped-frame information is unavailable for this build",
            ));
        }
        let Some(captured) = stop.frames.get(frame) else {
            return Err(Error::new(
                codes::DEBUG_UNKNOWN,
                format!("frame {frame} is not in the current stop snapshot"),
            ));
        };
        let Some(local) = captured.locals.get(name) else {
            return Err(Error::new(
                codes::DEBUG_UNKNOWN,
                format!("local `{name}` is not described by frame {frame}"),
            ));
        };
        let value = local
            .value
            .clone()
            .unwrap_or_else(|| DebugValue::Unavailable {
                warning: crate::debug::DebugWarning {
                    code: codes::W_DEBUG_UNAVAILABLE,
                    message: local
                        .reason
                        .clone()
                        .unwrap_or_else(|| "the local is unavailable".into()),
                },
            });
        let typed = TypedValue {
            type_key: local.type_key.clone(),
            value: value.clone(),
        };
        let atomic = local.value.is_some()
            && crate::debug::type_is_single_root(&self.tables.manifest, &local.type_key);
        let view = crate::debug::snapshot_view(
            &self.tables.manifest,
            &typed,
            format!("{frame}:{name}"),
            name.to_owned(),
            self.generation,
            self.build_id,
            stop.cycle,
            atomic,
        );
        Ok(LocalView {
            type_key: local.type_key.clone(),
            value,
            generation: self.generation,
            build_id: self.build_id,
            cycle: stop.cycle,
            view,
        })
    }

    /// Drains protocol stop events in order.
    pub(crate) fn drain_events(&self) -> Vec<StopEvent> {
        let mut control = lock(&self.control);
        control.events.drain(..).collect()
    }

    /// Returns the session generation.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the session build identity.
    pub(crate) fn build_id(&self) -> GotId {
        self.build_id
    }
}

fn current_stop(control: &Control, stop_id: u64) -> Result<Option<&StopState>> {
    let Some(stop) = control.stop.as_ref() else {
        return Ok(None);
    };
    if stop.id != stop_id {
        return Err(Error::new(
            codes::DEBUG_STOP,
            "the stop_id is stale; read the current stop_id from session.status",
        ));
    }
    Ok(Some(stop))
}

fn breakpoint_info(breakpoint: &Breakpoint) -> BreakpointInfo {
    BreakpointInfo {
        id: breakpoint.id,
        target: breakpoint.target.clone(),
        bound: breakpoint.bound.is_some(),
        checkpoints: breakpoint.bound.into_iter().collect(),
        source: breakpoint.source.clone(),
    }
}

impl DebugTables {
    /// Binds a source location per §16.7: exact file and line, smallest
    /// checkpoint whose column is at or after a positive requested column,
    /// otherwise the smallest checkpoint on the line. No match stays unbound.
    fn bind_source(&self, file: &str, line: u32, column: u32) -> Option<u32> {
        let mut on_line = self
            .records
            .iter()
            .filter(|record| record.file == file && record.line == line);
        if column > 0 {
            let mut first_on_line = None;
            for record in on_line.by_ref() {
                first_on_line.get_or_insert(record.id);
                if record.column >= column {
                    return Some(record.id);
                }
            }
            first_on_line
        } else {
            on_line.next().map(|record| record.id)
        }
    }
}

// ---------------------------------------------------------------------------
// Cycle context and the parked pump
// ---------------------------------------------------------------------------

/// A cycle-thread context that lets a parked cycle service the typed debug
/// engine and decode frame locals.
///
/// The raw pointers are valid only for the enclosing `module.cycle` call; the
/// host installs a [`CyclePumpGuard`] around that call.
pub(crate) struct CyclePump {
    target: *const Target,
    manifest: *const Manifest,
    roots: *const RootRegistry,
    info: *const ModuleInfo,
    metadata_error: *const Option<String>,
    manifest_json: *const str,
    manifest_binary: *const [u8],
    tables: Option<Arc<DebugTables>>,
    generation: u64,
    build_id: GotId,
    stop_cycle: u64,
    mode: &'static str,
    debug_hooks: bool,
    debug_info: &'static str,
    checkpoints: bool,
    manifest_digest: Digest32,
    engine: *mut DebugEngine,
}

/// Inputs for one cycle execution context.
pub(crate) struct CyclePumpInfo<'a> {
    pub(crate) target: &'a Target,
    pub(crate) manifest: &'a Manifest,
    pub(crate) roots: &'a RootRegistry,
    pub(crate) info: Option<&'a ModuleInfo>,
    pub(crate) generation: u64,
    pub(crate) build_id: GotId,
    pub(crate) stop_cycle: u64,
    pub(crate) mode: &'static str,
    pub(crate) debug_hooks: bool,
    pub(crate) debug_info: &'static str,
    pub(crate) checkpoints: bool,
    pub(crate) metadata_error: &'a Option<String>,
    pub(crate) tables: Option<Arc<DebugTables>>,
    pub(crate) manifest_digest: Digest32,
    pub(crate) manifest_json: &'a str,
    pub(crate) manifest_binary: &'a [u8],
}

thread_local! {
    static CYCLE_PUMP: std::cell::Cell<*const CyclePump> =
        const { std::cell::Cell::new(std::ptr::null()) };
}

impl CyclePump {
    /// Builds a cycle context. The caller must keep it alive for the whole
    /// module cycle call and must not use the engine or referenced host fields
    /// until the guard is dropped.
    pub(crate) fn new(info: CyclePumpInfo<'_>, engine: &mut DebugEngine) -> Self {
        Self {
            target: std::ptr::from_ref(info.target),
            manifest: std::ptr::from_ref(info.manifest),
            roots: std::ptr::from_ref(info.roots),
            info: info.info.map_or(std::ptr::null(), std::ptr::from_ref),
            metadata_error: std::ptr::from_ref(info.metadata_error),
            manifest_json: info.manifest_json as *const str,
            manifest_binary: info.manifest_binary as *const [u8],
            tables: info.tables,
            generation: info.generation,
            build_id: info.build_id,
            stop_cycle: info.stop_cycle,
            mode: info.mode,
            debug_hooks: info.debug_hooks,
            debug_info: info.debug_info,
            checkpoints: info.checkpoints,
            manifest_digest: info.manifest_digest,
            engine: std::ptr::from_mut(engine),
        }
    }

    fn depth(&self, fp: usize, sp: usize, tables: &DebugTables) -> Option<usize> {
        walk_chain(fp, sp, tables).0
    }

    fn debug_source(&self) -> DebugSource<'_> {
        // SAFETY: every pointer was derived from a live reference in
        // `CyclePump::new`, and the guard keeps the referenced data alive for
        // the whole module cycle call. The references never escape this call.
        unsafe {
            let info = if self.info.is_null() {
                None
            } else {
                Some(&*self.info)
            };
            DebugSource {
                target: &*self.target,
                manifest: &*self.manifest,
                roots: &*self.roots,
                info,
                generation: self.generation,
                build_id: self.build_id,
                cycle: self.stop_cycle,
                mode: self.mode,
                debug_hooks: self.debug_hooks,
                debug_info: Some(self.debug_info),
                checkpoints: self.checkpoints,
                metadata_error: (&*self.metadata_error).as_deref(),
                tables: self.tables.clone(),
                manifest_digest: self.manifest_digest,
                manifest_json: Some(&*self.manifest_json),
                manifest_binary: Some(&*self.manifest_binary),
            }
        }
    }

    fn pump(&self, now_ns: i64) {
        let source = self.debug_source();
        // SAFETY: the engine pointer is the host's `DebugEngine` field, kept
        // exclusively available to this thread for the cycle call. The parked
        // cycle thread is the only user, and it never holds a lock across ST.
        unsafe {
            (&mut *self.engine).pump(now_ns, Some(source));
        }
    }

    fn engine_has_session(&self) -> bool {
        // SAFETY: see `pump`.
        unsafe { (&*self.engine).has_session() }
    }

    fn pause_timeout(&self, session: &SessionStop, cycle: u64) {
        // SAFETY: see `pump`.
        unsafe {
            let _ = (&mut *self.engine).end_state(
                DebugEndReason::PauseTimeout,
                Some(session.generation),
                Some(session.build_id),
                Some(cycle),
            );
        }
    }

    fn manifest_target(&self) -> (&Manifest, &Target) {
        // SAFETY: the pointers are live for the whole module cycle call.
        unsafe { (&*self.manifest, &*self.target) }
    }
}

/// Installs a cycle context for the enclosing module cycle call.
pub(crate) struct CyclePumpGuard {
    previous: *const CyclePump,
    _not_send: std::marker::PhantomData<*mut ()>,
}

impl CyclePumpGuard {
    pub(crate) fn enter(pump: &CyclePump) -> Self {
        let previous = CYCLE_PUMP.with(|slot| slot.replace(std::ptr::from_ref(pump)));
        Self {
            previous,
            _not_send: std::marker::PhantomData,
        }
    }
}

impl Drop for CyclePumpGuard {
    fn drop(&mut self) {
        CYCLE_PUMP.with(|slot| slot.set(self.previous));
    }
}

fn cycle_pump() -> Option<*const CyclePump> {
    CYCLE_PUMP.with(|slot| {
        let pointer = slot.get();
        (!pointer.is_null()).then_some(pointer)
    })
}

// ---------------------------------------------------------------------------
// Service 13 entry and the parking loop
// ---------------------------------------------------------------------------

/// Entry called by the x86-64 trampoline with the checkpointing function's
/// frame pointer, stack pointer, and return address.
///
/// # Safety
///
/// `fp`, `sp`, and `ra` must come from the module call site's register state
/// and be used only on the cycle thread.
pub(crate) unsafe extern "C" fn debug_check_entry(
    checkpoint_id: u32,
    span_id: u32,
    fp: usize,
    sp: usize,
    _ra: usize,
) {
    let Some(session) = current() else {
        return;
    };
    let Some(pump_pointer) = cycle_pump() else {
        // A hook outside a hosted cycle (a loader/prepare thread or a direct
        // unit-test call) must never park. §16.4 requires debug_check to be
        // callable without a session and to remain a no-op, so being outside
        // a cycle is treated the same way.
        return;
    };
    // SAFETY: the guard keeps the context alive for the whole module cycle
    // call, which is the only call path that can reach service 13.
    let pump = unsafe { &*pump_pointer };

    let Some(record) = validate_checkpoint(&session, checkpoint_id, span_id) else {
        return;
    };
    let hit = {
        let control = lock(&session.control);
        if control.ending || control.mismatch.is_some() {
            return;
        }
        let breakpoint = control
            .breakpoints
            .values()
            .find(|breakpoint| breakpoint.bound == Some(checkpoint_id))
            .map(|breakpoint| breakpoint.id);
        let pause = control.pause_requested;
        let step = control.step.clone();
        (breakpoint, pause, step)
    };
    let (breakpoint_id, pause_requested, step) = hit;

    let mut reason = None;
    if breakpoint_id.is_some() {
        reason = Some(StopReason::Breakpoint);
    } else if pause_requested {
        reason = Some(StopReason::Pause);
    } else if let Some(plan) = &step {
        match plan.mode {
            StepMode::Into => reason = Some(StopReason::Step),
            StepMode::Over => {
                if pump
                    .depth(fp, sp, &session.tables)
                    .is_some_and(|depth| depth <= plan.depth)
                {
                    reason = Some(StopReason::Step);
                }
            }
            StepMode::Out => {
                if pump
                    .depth(fp, sp, &session.tables)
                    .is_some_and(|depth| depth < plan.depth)
                {
                    reason = Some(StopReason::Step);
                }
            }
        }
        if reason.is_none()
            && matches!(plan.mode, StepMode::Over | StepMode::Out)
            && pump.depth(fp, sp, &session.tables).is_none()
        {
            // The chain became unwalkable; the plan cannot be satisfied.
            lock(&session.control).step = None;
        }
    }
    let Some(reason) = reason else {
        return;
    };

    let (frames, truncated, depth) = if session.tables.frames.is_some() {
        let (manifest, target) = pump.manifest_target();
        capture_frames(
            &session.tables,
            manifest,
            target,
            &record,
            fp,
            sp,
            session.generation,
        )
    } else {
        (Vec::new(), false, None)
    };

    let now = Instant::now();
    let stop_id;
    {
        let mut control = lock(&session.control);
        if control.ending {
            return;
        }
        control.next_stop = control.next_stop.saturating_add(1).max(1);
        stop_id = control.next_stop;
        control.pause_requested = false;
        control.step = None;
        let cycle = pump.stop_cycle;
        control.stop = Some(StopState {
            id: stop_id,
            reason,
            checkpoint: record.clone(),
            breakpoint_id,
            cycle,
            deadline: now + session.timeout,
            frames,
            truncated,
            frames_available: session.tables.frames.is_some(),
            depth,
        });
        control.events.push_back(StopEvent::Stopped {
            stop_id,
            reason,
            checkpoint_id: record.id,
            source: SourcePosition {
                file: record.file.clone(),
                line: record.line,
                column: record.column,
            },
            breakpoint_id,
            generation: session.generation,
            build_id: session.build_id,
            cycle,
        });
    }
    session.wake.notify_all();
    park(&session, pump);
}

fn validate_checkpoint(
    session: &SessionStop,
    checkpoint_id: u32,
    span_id: u32,
) -> Option<DebugCheckpointRecord> {
    if let Some(record) = session.tables.checkpoint(checkpoint_id)
        && record.span == span_id
    {
        return Some(record.clone());
    }
    let mut control = lock(&session.control);
    if control.mismatch.is_none() {
        control.mismatch = Some(format!(
            "the module called debug_check with checkpoint {checkpoint_id} span {span_id}, which does not match the validated checkpoint table"
        ));
        control.breakpoints.clear();
        control.step = None;
        control.pause_requested = false;
    }
    None
}

fn park(session: &SessionStop, pump: &CyclePump) {
    loop {
        {
            let mut control = lock(&session.control);
            if control.ending || control.resume.is_some() {
                control.resume = None;
                return;
            }
            let Some(stop) = control.stop.as_ref() else {
                return;
            };
            let now = Instant::now();
            if now >= stop.deadline {
                let cycle = stop.cycle;
                control.ending = true;
                control.stop = None;
                control.step = None;
                control.pause_requested = false;
                control.breakpoints.clear();
                drop(control);
                pump.pause_timeout(session, cycle);
                return;
            }
            let wait = stop.deadline.saturating_duration_since(now).min(PARK_POLL);
            let (mut guard, _) = session
                .wake
                .wait_timeout(control, wait)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if guard.ending || guard.resume.is_some() {
                guard.resume = None;
                return;
            }
        }
        pump.pump(crate::transport::monotonic_ns() as i64);
        if !pump.engine_has_session() {
            // The engine ended the session (for example a queued
            // ClearSessionWait); leave service 13 without a timeout notice.
            let mut control = lock(&session.control);
            control.ending = true;
            control.stop = None;
            control.step = None;
            control.pause_requested = false;
            control.breakpoints.clear();
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Frame and local capture
// ---------------------------------------------------------------------------

/// Walks the saved-frame-pointer chain.
///
/// Returns the known depth (including the top frame) when the top frame
/// pointer is walkable, the caller levels as `(return_address, caller_fp)`,
/// and whether the 1024-frame cap truncated the chain.
fn walk_chain(
    fp: usize,
    sp: usize,
    _tables: &DebugTables,
) -> (Option<usize>, Vec<(usize, usize)>, bool) {
    if fp == 0
        || !fp.is_multiple_of(std::mem::align_of::<usize>())
        || fp < sp
        || fp.saturating_sub(sp) > MAX_STACK_WALK_BYTES
    {
        return (None, Vec::new(), false);
    }
    let mut levels = Vec::new();
    let mut current = fp;
    let mut depth = 1usize;
    let mut truncated = false;
    loop {
        if depth >= MAX_STACK_FRAMES {
            // There may be another caller; only claim truncation when the
            // next link itself is valid.
            if let (Some(saved), Some(_)) = (
                read_stack_word(current),
                read_stack_word(current.wrapping_add(std::mem::size_of::<usize>())),
            ) && saved > current
                && saved.saturating_sub(sp) <= MAX_STACK_WALK_BYTES
                && saved.is_multiple_of(std::mem::align_of::<usize>())
            {
                truncated = true;
            }
            break;
        }
        let Some(saved) = read_stack_word(current) else {
            break;
        };
        let Some(ret) = read_stack_word(current.wrapping_add(std::mem::size_of::<usize>())) else {
            break;
        };
        if saved == 0 || ret == 0 {
            break;
        }
        if saved <= current
            || !saved.is_multiple_of(std::mem::align_of::<usize>())
            || saved.saturating_sub(sp) > MAX_STACK_WALK_BYTES
        {
            break;
        }
        levels.push((ret, saved));
        depth += 1;
        current = saved;
    }
    (Some(depth), levels, truncated)
}

fn read_stack_word(address: usize) -> Option<usize> {
    if !address.is_multiple_of(std::mem::align_of::<usize>()) || address == 0 {
        return None;
    }
    // SAFETY: callers validate that the address is aligned and within a
    // bounded span of the current stack pointer. The saved-frame-pointer
    // chain is produced by generated debug code with forced frame pointers.
    Some(unsafe { std::ptr::read_unaligned(address as *const usize) })
}

fn capture_frames(
    tables: &DebugTables,
    manifest: &Manifest,
    target: &Target,
    checkpoint: &DebugCheckpointRecord,
    fp: usize,
    sp: usize,
    generation: u64,
) -> (Vec<CapturedFrame>, bool, Option<usize>) {
    let (depth, callers, truncated) = walk_chain(fp, sp, tables);
    let mut budget = MAX_LOCAL_COPY_BYTES;
    let mut frames = Vec::with_capacity(callers.len().saturating_add(1));

    let top_symbol = tables
        .frame_function(checkpoint.function)
        .map(|function| function.symbol.clone());
    let top_locals = match depth {
        Some(_) => capture_locals(
            tables,
            manifest,
            target,
            checkpoint.function,
            fp,
            sp,
            Liveness::At(checkpoint.id),
            generation,
            &mut budget,
        ),
        None => capture_locals(
            tables,
            manifest,
            target,
            checkpoint.function,
            0,
            sp,
            Liveness::Unavailable,
            generation,
            &mut budget,
        ),
    };
    frames.push(CapturedFrame {
        function: Some(checkpoint.function),
        symbol: top_symbol,
        source: Some(SourcePosition {
            file: checkpoint.file.clone(),
            line: checkpoint.line,
            column: checkpoint.column,
        }),
        optimized_out: false,
        locals: top_locals,
    });

    for (ret, caller_fp) in callers {
        let address = ret.wrapping_sub(1);
        let range = tables.function_at(address);
        let (function, symbol, locals) = match range {
            Some(range) => (
                Some(range.function),
                Some(range.symbol.clone()),
                capture_locals(
                    tables,
                    manifest,
                    target,
                    range.function,
                    caller_fp,
                    sp,
                    Liveness::WholeFunction,
                    generation,
                    &mut budget,
                ),
            ),
            None => (None, None, BTreeMap::new()),
        };
        frames.push(CapturedFrame {
            function,
            symbol,
            source: None,
            optimized_out: range.is_none(),
            locals: function.map_or_else(BTreeMap::new, |_| locals),
        });
    }
    (frames, truncated, depth)
}

enum Liveness {
    At(u32),
    WholeFunction,
    Unavailable,
}

#[allow(clippy::too_many_arguments)]
fn capture_locals(
    tables: &DebugTables,
    manifest: &Manifest,
    target: &Target,
    function: GotId,
    fp: usize,
    sp: usize,
    liveness: Liveness,
    generation: u64,
    budget: &mut usize,
) -> BTreeMap<String, CapturedLocal> {
    let mut captured = BTreeMap::new();
    let Some(function_frame) = tables.frame_function(function) else {
        return captured;
    };
    let checkpoint_range = tables.checkpoint_range(function);
    for local in &function_frame.locals {
        let live = match liveness {
            Liveness::At(checkpoint) => {
                local.live_first <= checkpoint && checkpoint <= local.live_last
            }
            Liveness::WholeFunction => checkpoint_range
                .is_some_and(|(first, last)| local.live_first <= first && last <= local.live_last),
            Liveness::Unavailable => false,
        };
        if !live {
            captured.insert(
                local.name.clone(),
                CapturedLocal {
                    type_key: local.type_key.clone(),
                    value: None,
                    reason: Some("the local is not live at this stop".into()),
                },
            );
            continue;
        }
        let Some(address) = local.address(fp) else {
            captured.insert(
                local.name.clone(),
                CapturedLocal {
                    type_key: local.type_key.clone(),
                    value: None,
                    reason: Some("the local storage is outside the stopped frame".into()),
                },
            );
            continue;
        };
        let size = usize::try_from(local.size).unwrap_or(usize::MAX);
        let Some(end) = address.checked_add(size) else {
            captured.insert(
                local.name.clone(),
                CapturedLocal {
                    type_key: local.type_key.clone(),
                    value: None,
                    reason: Some("the local storage range overflows".into()),
                },
            );
            continue;
        };
        if address < sp || end > fp {
            captured.insert(
                local.name.clone(),
                CapturedLocal {
                    type_key: local.type_key.clone(),
                    value: None,
                    reason: Some("the local storage is outside the live stack frame".into()),
                },
            );
            continue;
        }
        if size > *budget {
            captured.insert(
                local.name.clone(),
                CapturedLocal {
                    type_key: local.type_key.clone(),
                    value: None,
                    reason: Some("the stop snapshot local copy limit was reached".into()),
                },
            );
            continue;
        }
        // SAFETY: the sidecar validated the slot below the frame pointer; the
        // range is checked above to lie between the live stack pointer and the
        // frame pointer. The copy is one bounded read of the described size.
        let bytes = unsafe { std::slice::from_raw_parts(address as *const u8, size) };
        *budget -= size;
        let typed = TypeKey::parse(&local.type_key).and_then(|ty| {
            decode_type_value(manifest, target, &ty, &local.type_key, bytes, generation)
        });
        match typed {
            Ok(TypedValue { value, .. }) => {
                captured.insert(
                    local.name.clone(),
                    CapturedLocal {
                        type_key: local.type_key.clone(),
                        value: Some(value),
                        reason: None,
                    },
                );
            }
            Err(_) => {
                captured.insert(
                    local.name.clone(),
                    CapturedLocal {
                        type_key: local.type_key.clone(),
                        value: None,
                        reason: Some(
                            "the local could not be decoded from its recorded location".into(),
                        ),
                    },
                );
            }
        }
    }
    captured
}

// ---------------------------------------------------------------------------
// Service 13 trampolines
// ---------------------------------------------------------------------------

/// The host API service-13 entry point.
///
/// x86-64 builds capture the caller's `rbp`, `rsp`, and return address in an
/// architecture trampoline and call [`debug_check_entry`]. Other architectures
/// fall back to a call with no register context; the checkpoint table then
/// keeps stops unavailable rather than guessing frame rules.
#[cfg(all(target_arch = "x86_64", windows))]
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn debug_check_hook(_checkpoint_id: u32, _span_id: u32) {
    core::arch::naked_asm!(
        // Win64: the hook is entered with rsp pointing at the return address.
        // A fifth argument goes at [rsp+0x20] after the 0x28-byte call frame.
        "mov r10, [rsp]",
        "mov r11, rbp",
        "mov rax, rsp",
        "sub rsp, 0x28",
        "mov r8, r11",
        "mov r9, rax",
        "mov [rsp + 0x20], r10",
        "call {entry}",
        "add rsp, 0x28",
        "ret",
        entry = sym debug_check_entry,
    );
}

/// The host API service-13 entry point (System V x86-64).
#[cfg(all(target_arch = "x86_64", not(windows)))]
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn debug_check_hook(_checkpoint_id: u32, _span_id: u32) {
    core::arch::naked_asm!(
        // SysV: args are edi/esi; pass fp, sp, and the return address as the
        // third, fourth, and fifth arguments.
        "mov r8, [rsp]",
        "mov rdx, rbp",
        "mov rcx, rsp",
        "sub rsp, 8",
        "call {entry}",
        "add rsp, 8",
        "ret",
        entry = sym debug_check_entry,
    );
}

/// Fallback service-13 entry for targets without v1 frame rules.
#[cfg(not(target_arch = "x86_64"))]
pub(crate) unsafe extern "C" fn debug_check_hook(checkpoint_id: u32, span_id: u32) {
    // SAFETY: the caller uses the documented service-13 ABI; the fallback
    // performs no stack access and stops are unavailable without frame rules.
    unsafe {
        debug_check_entry(checkpoint_id, span_id, 0, 0, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: u32, file: &str, line: u32, column: u32, span: u32) -> DebugCheckpointRecord {
        DebugCheckpointRecord {
            id,
            flags: 0,
            function: GotId::ZERO,
            line,
            column,
            span,
            file: file.into(),
        }
    }

    fn empty_manifest() -> Manifest {
        Manifest {
            header: stc_got::Header {
                profile: "test".into(),
                compiler_version: "test".into(),
                target_triple: "x86_64-unknown-linux-gnu".into(),
                module_name: "stop-test".into(),
                active_configuration: String::new(),
                endianness: 0,
                pointer_width: 8,
                module_format: 1,
                host_abi_major: 1,
                host_abi_minor: 0,
                generator_flags: 0,
                required_features: 0,
                build_id: GotId::ZERO,
                parent_build_id: None,
                parent_manifest_digest: None,
                descriptor_digest: Digest32::ZERO,
                source_fingerprint: Digest32::ZERO,
                catalog_digest: Digest32::ZERO,
                plan_digest: Digest32::ZERO,
            },
            roots: Vec::new(),
            functions: Vec::new(),
            types: Vec::new(),
            descriptors: Vec::new(),
            instances: Vec::new(),
            tasks: Vec::new(),
            exports: Vec::new(),
            slots: Vec::new(),
            plan: stc_got::Plan {
                verdict: stc_got::PlanVerdict::OnlineCandidate,
                parent_build_id: None,
                parent_manifest_digest: None,
                actions: Vec::new(),
                diagnostics: Vec::new(),
            },
            history: stc_got::History::default(),
            type_metadata: Vec::new(),
            aggregates: Vec::new(),
        }
    }

    fn tables_for(records: Vec<DebugCheckpointRecord>) -> DebugTables {
        let mut checkpoint_ranges: BTreeMap<GotId, (u32, u32)> = BTreeMap::new();
        for record in &records {
            let range = checkpoint_ranges
                .entry(record.function)
                .or_insert((record.id, record.id));
            range.0 = range.0.min(record.id);
            range.1 = range.1.max(record.id);
        }
        DebugTables {
            records,
            ranges: Vec::new(),
            checkpoint_ranges,
            frames: None,
            frames_error: None,
            stepping: false,
            manifest: empty_manifest(),
            pointer_width: 8,
        }
    }

    #[test]
    fn source_binding_uses_smallest_column_at_or_after_request() {
        let tables = tables_for(vec![
            record(1, "src/main.st", 12, 3, 0),
            record(2, "src/main.st", 12, 7, 0),
            record(3, "src/main.st", 12, 9, 0),
            record(4, "src/main.st", 13, 1, 0),
        ]);
        assert_eq!(tables.bind_source("src/main.st", 12, 7), Some(2));
        assert_eq!(tables.bind_source("src/main.st", 12, 8), Some(3));
        assert_eq!(tables.bind_source("src/main.st", 12, 10), Some(1));
        assert_eq!(tables.bind_source("src/main.st", 12, 0), Some(1));
        assert_eq!(tables.bind_source("src/main.st", 12, 1), Some(1));
        assert_eq!(tables.bind_source("src/main.st", 14, 1), None);
        assert_eq!(tables.bind_source("other.st", 12, 1), None);
    }

    #[test]
    fn checkpoint_target_rejects_unknown_ids() {
        let tables = tables_for(vec![record(1, "src/main.st", 1, 1, 0)]);
        assert!(tables.checkpoint(1).is_some());
        assert!(tables.checkpoint(2).is_none());
        assert!(tables.checkpoint(0).is_none());
    }

    #[test]
    fn breakpoint_limit_is_1024_and_clearing_frees_a_slot() {
        let session = Arc::new(SessionStop {
            tables: Arc::new(tables_for(vec![record(1, "src/main.st", 1, 1, 0)])),
            generation: 1,
            build_id: GotId::ZERO,
            timeout: Duration::from_secs(60),
            control: Mutex::new(Control {
                breakpoints: BTreeMap::new(),
                next_breakpoint: 0,
                stop: None,
                next_stop: 0,
                step: None,
                pause_requested: false,
                resume: None,
                ending: false,
                mismatch: None,
                events: VecDeque::new(),
            }),
            wake: Condvar::new(),
        });
        let target = BreakpointTarget::Checkpoint { checkpoint_id: 1 };
        for _ in 0..MAX_BREAKPOINTS {
            let info = session.set_breakpoint(target.clone()).expect("bound");
            assert!(info.bound);
        }
        let error = session
            .set_breakpoint(target.clone())
            .expect_err("the 1025th breakpoint exceeds the limit");
        assert_eq!(error.code, codes::DEBUG_LIMIT);
        assert_eq!(session.breakpoints().len(), MAX_BREAKPOINTS);
        assert!(session.clear_breakpoint(1));
        let info = session
            .set_breakpoint(target)
            .expect("clearing frees a slot");
        assert!(info.bound);
        assert_eq!(session.breakpoints().len(), MAX_BREAKPOINTS);
    }
}
