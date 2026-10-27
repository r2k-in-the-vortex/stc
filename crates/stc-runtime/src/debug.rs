//! Host typed debug access (Debug and Monitoring §15.1, §15.2, §15.5,
//! §15.7, and §15.8, read side).
//!
//! Phase 3 defines the host-side target, typed-value, watch, and service
//! models. Reads and watches run on the host/cycle thread at a cycle
//! boundary: the service queue is `Send + Sync`, but it never dereferences
//! module pointers, root storage, or the process image. The cycle thread
//! drains it through [`crate::Host::debug_pump_at`] while no module call is in
//! progress, and copies values under a [`crate::GenerationPin`]. Phase 5 owns
//! the protocol, sessions, authorization, auditing, and the service-thread
//! handoff; phase 4 owns writes and forces.
//!
//! A path target is a stable-key path such as `MAIN.COUNTER`, optionally
//! prefixed with a package (`PKG::MAIN.COUNTER`) and optionally carrying an
//! instance qualifier (`MAIN.TICKS#CFG/RES/INST`). Identifiers are
//! ASCII-case-folded and validated as manifest keys; a raw address, pointer,
//! or offset is rejected as an unknown target. A `GotId` target is the strict
//! `0x` plus 32 lowercase hex form. A local target names a stop-frame token
//! and a local path; locals are read only from the immutable stop snapshot in
//! [`DebugReadRequest::stop_frame`], never from a running frame.
//!
//! Split storage is resolved from the manifest roots. A scalar, enum,
//! subrange, string, single-root array, or `OVERLAP` value is copied as one
//! atomic value. Split aggregates, class/FB objects, interfaces, and
//! references are resolved leaf by leaf; their values report
//! `atomic = false`. All leaves of one read come from one pinned generation
//! and one cycle, so `consistent` is `true`; a future coalesced watch that
//! spans cycles must clear it.
//!
//! The manifest does not carry a type record for a whole program instance or
//! for an array-of-aggregate element's parent shape, so those aggregate paths
//! are rejected with [`stc_got::codes::DEBUG_UNKNOWN`] rather than guessed.
//! Individual fields, including fields inside array elements, remain
//! resolvable.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use stc_got::Manifest;
use stc_got::digest::GotId;
use stc_got::error::{Error, Result, codes};
use stc_got::keys::{normalize_ident, normalize_package};
use stc_got::layout::{ArrayBound, StringKind, Target, TypeKey, elementary_layout};
use stc_got::model::{FieldRecord, PlanAction, RootKind, RootRecord, TypeKind, TypeRecord};
use stc_got::sha256::domain_hash;

/// Maximum number of active watch targets.
pub const MAX_WATCHES: usize = 256;
/// Minimum watch interval in nanoseconds.
pub const MIN_WATCH_INTERVAL_NS: i64 = 50_000_000;
/// Maximum number of queued watch batches.
pub const MAX_WATCH_BATCHES: usize = 64;
/// Maximum approximate byte size of the event queue.
pub const MAX_EVENT_BYTES: usize = 1024 * 1024;

const MAX_DEBUG_COMMANDS: usize = 1024;
const MAX_DECODE_DEPTH: usize = 64;

// ---------------------------------------------------------------------------
// Public request, target, and value model
// ---------------------------------------------------------------------------

/// One typed debug target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DebugTarget {
    /// A stable-key path, optionally package-qualified and instance-qualified.
    Path(String),
    /// A canonical persistent identity.
    GotId(GotId),
    /// A local in a stopped frame snapshot.
    Local {
        /// Opaque stop-frame token supplied by the phase-6 debugger.
        frame: String,
        /// Dot-separated local path.
        path: String,
    },
}

impl DebugTarget {
    /// Creates a path target. The path is normalized when it is resolved.
    #[must_use]
    pub fn path(path: impl Into<String>) -> Self {
        Self::Path(path.into())
    }

    /// Creates a `GotId` target.
    #[must_use]
    pub fn got_id(got_id: GotId) -> Self {
        Self::GotId(got_id)
    }

    /// Creates a stopped-frame local target.
    #[must_use]
    pub fn local(frame: impl Into<String>, path: impl Into<String>) -> Self {
        Self::Local {
            frame: frame.into(),
            path: path.into(),
        }
    }
}

/// One immutable stopped-frame snapshot.
///
/// Phase 6 constructs this from a stop event. Phase 3 accepts it only when
/// its generation and build match the active generation and its frame token
/// matches the request; a missing or stale snapshot is
/// [`codes::DEBUG_STOP`]. Local keys use the same ASCII-case-folded identifier
/// normalization as [`DebugTarget::local`] paths, so phase 6 must insert
/// uppercase local and parameter names. An optimized-out or unavailable local
/// is stored with [`DebugValue::Unavailable`] and
/// [`codes::W_DEBUG_UNAVAILABLE`]; phase 3 returns it unchanged rather than
/// reconstructing a value.
#[derive(Clone, Debug, PartialEq)]
pub struct StopFrame {
    /// Opaque frame token.
    pub token: String,
    /// Generation observed at the stop.
    pub generation: u64,
    /// Build observed at the stop.
    pub build_id: GotId,
    /// Cycle observed at the stop.
    pub cycle: u64,
    /// Typed locals captured at the stop.
    pub locals: BTreeMap<String, StoppedLocal>,
}

/// One typed local in a stopped frame.
#[derive(Clone, Debug, PartialEq)]
pub struct StoppedLocal {
    /// Captured value.
    pub value: TypedValue,
    /// Whether the capture was one atomic copy.
    pub atomic: bool,
}

impl StoppedLocal {
    /// Creates a stopped local.
    #[must_use]
    pub fn new(value: TypedValue, atomic: bool) -> Self {
        Self { value, atomic }
    }
}

/// One typed read request.
#[derive(Clone, Debug, PartialEq)]
pub struct DebugReadRequest {
    /// Target to resolve.
    pub target: DebugTarget,
    /// Expected active generation, when known.
    pub expected_generation: Option<u64>,
    /// Expected active build, when known.
    pub expected_build_id: Option<GotId>,
    /// Expected canonical type key, when known.
    pub expected_type_key: Option<String>,
    /// Expected target layout hash, when known.
    pub expected_layout_hash: Option<GotId>,
    /// Stop snapshot for a local target.
    pub stop_frame: Option<StopFrame>,
}

impl DebugReadRequest {
    /// Creates a request for a target with no expectations.
    #[must_use]
    pub fn new(target: DebugTarget) -> Self {
        Self {
            target,
            expected_generation: None,
            expected_build_id: None,
            expected_type_key: None,
            expected_layout_hash: None,
            stop_frame: None,
        }
    }

    /// Attaches an expected generation.
    #[must_use]
    pub fn with_generation(mut self, generation: u64) -> Self {
        self.expected_generation = Some(generation);
        self
    }

    /// Attaches an expected build.
    #[must_use]
    pub fn with_build(mut self, build_id: GotId) -> Self {
        self.expected_build_id = Some(build_id);
        self
    }

    /// Attaches an expected type key.
    #[must_use]
    pub fn with_type_key(mut self, type_key: impl Into<String>) -> Self {
        self.expected_type_key = Some(type_key.into());
        self
    }

    /// Attaches an expected layout hash.
    #[must_use]
    pub fn with_layout_hash(mut self, layout_hash: GotId) -> Self {
        self.expected_layout_hash = Some(layout_hash);
        self
    }

    /// Attaches a stopped-frame snapshot.
    #[must_use]
    pub fn with_stop_frame(mut self, frame: StopFrame) -> Self {
        self.stop_frame = Some(frame);
        self
    }
}

/// One typed debug value.
#[derive(Clone, Debug, PartialEq)]
pub struct TypedValue {
    /// Canonical type key.
    pub type_key: String,
    /// Value in the host typed model (§15.5).
    pub value: DebugValue,
}

impl TypedValue {
    /// Creates a typed value.
    #[must_use]
    pub fn new(type_key: impl Into<String>, value: DebugValue) -> Self {
        Self {
            type_key: type_key.into(),
            value,
        }
    }
}

/// A string value with its capacity and code-point units.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringValue {
    /// Declared capacity in code units.
    pub capacity: u32,
    /// Number of code points decoded from the stored length.
    pub length: u32,
    /// Unicode scalar values in declaration order.
    pub units: Vec<u32>,
    /// The text when every scalar is a valid Unicode scalar value.
    pub text: Option<String>,
}

impl StringValue {
    /// Returns the decoded text, or the empty string when invalid.
    #[must_use]
    pub fn text_or_empty(&self) -> &str {
        self.text.as_deref().unwrap_or("")
    }
}

/// One named aggregate field.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedValue {
    /// Field path relative to the aggregate.
    pub name: String,
    /// Typed field value.
    pub value: TypedValue,
}

/// One warning attached to a debug value or batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugWarning {
    /// Stable warning code.
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
}

/// The host typed value model (§15.5).
#[derive(Clone, Debug, PartialEq)]
pub enum DebugValue {
    /// `BOOL`.
    Bool(bool),
    /// Every integer, enum, subrange, time, date, TOD, and DT value.
    ///
    /// The type key distinguishes signedness and width; the value is the
    /// exact decimal representation.
    Decimal(String),
    /// `REAL` or `LREAL`.
    Real(f64),
    /// `STRING`, `WSTRING`, or `USTRING`.
    String(StringValue),
    /// A single-root array in declared index order.
    Array(Vec<TypedValue>),
    /// A single-root contiguous struct/FB value in field order.
    Aggregate(Vec<NamedValue>),
    /// A class, interface, or FB object.
    Object {
        /// Deterministic identity derived from storage, never an address.
        identity: Option<String>,
        /// Whether the object handle was non-null.
        present: bool,
        /// Class type key, when this is a class object.
        class: Option<String>,
        /// Interface type key, when this is an interface value.
        interface: Option<String>,
        /// Field values, empty when they are not available.
        fields: Vec<NamedValue>,
    },
    /// A `REF_TO` or `PTR_TO`, represented without addresses.
    Reference {
        /// Deterministic identity derived from descriptor or pointer bytes.
        identity: Option<String>,
        /// Whether the reference was non-null.
        present: bool,
        /// Referenced type key.
        target_type: Option<String>,
    },
    /// `OVERLAP` storage, reported as opaque bytes.
    Overlap {
        /// Raw declared bytes.
        bytes: Vec<u8>,
        /// Standard padded base64 of those bytes.
        base64: String,
    },
    /// A value the manifest/host cannot reconstruct.
    Unavailable {
        /// Per-value warning, always `STC-W-DEBUG-002`.
        warning: DebugWarning,
    },
}

/// One completed typed read.
#[derive(Clone, Debug, PartialEq)]
pub struct DebugRead {
    /// The target as requested.
    pub target: DebugTarget,
    /// Canonical manifest key, or the local path for a stopped local.
    pub resolved_key: String,
    /// Generation copied.
    pub generation: u64,
    /// Build copied.
    pub build_id: GotId,
    /// Completed cycles of that generation at the copy boundary.
    pub cycle: u64,
    /// Whether the value was one atomic single-root copy.
    pub atomic: bool,
    /// Whether every leaf came from the reported generation and cycle.
    pub consistent: bool,
    /// Canonical type key.
    pub type_key: String,
    /// Typed value.
    pub value: DebugValue,
}

/// A readable debug failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugFailure {
    /// Stable diagnostic code.
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
}

impl DebugFailure {
    /// Creates a failure.
    #[must_use]
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<Error> for DebugFailure {
    fn from(error: Error) -> Self {
        Self {
            code: error.code,
            message: error.message,
        }
    }
}

/// Request identity allocated by a [`DebugService`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReadRequestId(pub u64);

/// Watch identity allocated by a [`DebugService`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WatchId(pub u64);

/// One watch registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchSpec {
    /// Target read on every due period.
    pub target: DebugTarget,
    /// Watch period in nanoseconds, at least [`MIN_WATCH_INTERVAL_NS`].
    pub interval_ns: i64,
}

impl WatchSpec {
    /// Creates and validates a watch specification.
    ///
    /// # Errors
    ///
    /// Rejects a local target, an invalid target shape, a period shorter than
    /// 50 ms, and excessive target length.
    pub fn new(target: DebugTarget, interval_ns: i64) -> Result<Self> {
        let spec = Self {
            target,
            interval_ns,
        };
        validate_watch_spec(&spec)?;
        Ok(spec)
    }
}

/// One watch value in a batch.
#[derive(Clone, Debug, PartialEq)]
pub struct WatchValue {
    /// Registered watch identity.
    pub watch_id: WatchId,
    /// Target as registered.
    pub target: DebugTarget,
    /// Generation copied.
    pub generation: u64,
    /// Build copied.
    pub build_id: GotId,
    /// Cycle copied.
    pub cycle: u64,
    /// Whether the value was atomic.
    pub atomic: bool,
    /// Whether every leaf came from the reported cycle.
    pub consistent: bool,
    /// Typed value or an unavailable explanation.
    pub result: WatchResult,
}

/// The result of one watch read.
#[derive(Clone, Debug, PartialEq)]
pub enum WatchResult {
    /// The value was available.
    Available(TypedValue),
    /// The value was unavailable; `warning` is per value.
    Unavailable {
        /// Type key when the manifest identified it.
        type_key: Option<String>,
        /// Failure that made the value unavailable.
        error: Option<DebugFailure>,
        /// `STC-W-DEBUG-002`.
        warning: DebugWarning,
    },
}

/// One coalesced watch batch.
#[derive(Clone, Debug, PartialEq)]
pub struct DebugBatch {
    /// Generation shared by the batch.
    pub generation: u64,
    /// Build shared by the batch.
    pub build_id: GotId,
    /// Cycle shared by the batch.
    pub cycle: u64,
    /// Values in watch-ID order.
    pub values: Vec<WatchValue>,
    /// Batch warnings, including `STC-W-DEBUG-003`.
    pub warnings: Vec<DebugWarning>,
}

/// One completed asynchronous read.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadCompletion {
    /// Request identity.
    pub request: ReadRequestId,
    /// Read result.
    pub result: std::result::Result<DebugRead, DebugFailure>,
}

/// One asynchronous debug error (for example, adding a watch with no active
/// generation).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugError {
    /// Stable error code.
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
    /// Associated read, when any.
    pub request: Option<ReadRequestId>,
    /// Associated watch, when any.
    pub watch: Option<WatchId>,
}

/// One event drained by a service thread.
#[derive(Clone, Debug, PartialEq)]
pub enum DebugEvent {
    /// A completed asynchronous read.
    Read(ReadCompletion),
    /// One coalesced watch batch.
    Batch(DebugBatch),
    /// An asynchronous command failure.
    Error(DebugError),
}

// ---------------------------------------------------------------------------
// Service queue and watch engine
// ---------------------------------------------------------------------------

struct DebugBus {
    state: Mutex<BusState>,
}

#[derive(Default)]
struct BusState {
    commands: VecDeque<DebugCommand>,
    events: VecDeque<DebugEvent>,
    event_bytes: usize,
    next_request: u64,
    next_watch: u64,
    dropped_watch_batch: bool,
}

enum DebugCommand {
    Read {
        request: ReadRequestId,
        target: DebugReadRequest,
    },
    AddWatch {
        watch: WatchId,
        spec: WatchSpec,
    },
    RemoveWatch {
        watch: WatchId,
    },
}

/// A `Send + Sync` debug service facade.
///
/// The facade only enqueues commands and drains events. It never holds a
/// `Host`, root pointer, module pointer, or process-image pointer. A service
/// thread may submit reads and watches while the cycle thread runs; the cycle
/// thread performs the copy at the next safe boundary.
#[derive(Clone)]
pub struct DebugService {
    bus: Arc<DebugBus>,
}

impl DebugService {
    /// Submits a typed read.
    ///
    /// # Errors
    ///
    /// Rejects a syntactically invalid target, a local watch target, or a full
    /// command queue. Target existence and staleness are reported through a
    /// [`DebugEvent::Read`] result.
    pub fn submit_read(&self, request: DebugReadRequest) -> Result<ReadRequestId> {
        validate_request(&request)?;
        let mut state = lock_bus(&self.bus);
        if state.commands.len() >= MAX_DEBUG_COMMANDS {
            return Err(debug_limit("the debug command queue is full"));
        }
        state.next_request = state.next_request.saturating_add(1).max(1);
        let request_id = ReadRequestId(state.next_request);
        state.commands.push_back(DebugCommand::Read {
            request: request_id,
            target: request,
        });
        Ok(request_id)
    }

    /// Adds a watch.
    ///
    /// # Errors
    ///
    /// Rejects a local or invalid target, a period shorter than 50 ms, or a
    /// full command queue. The 256-watch limit is enforced when the cycle
    /// thread applies the command and is reported as a
    /// [`DebugEvent::Error`].
    pub fn add_watch(&self, spec: WatchSpec) -> Result<WatchId> {
        validate_watch_spec(&spec)?;
        let mut state = lock_bus(&self.bus);
        if state.commands.len() >= MAX_DEBUG_COMMANDS {
            return Err(debug_limit("the debug command queue is full"));
        }
        state.next_watch = state.next_watch.saturating_add(1).max(1);
        let watch = WatchId(state.next_watch);
        state
            .commands
            .push_back(DebugCommand::AddWatch { watch, spec });
        Ok(watch)
    }

    /// Removes a watch. The removal is applied at the next boundary.
    ///
    /// # Errors
    ///
    /// Rejects a full command queue.
    pub fn remove_watch(&self, watch: WatchId) -> Result<()> {
        let mut state = lock_bus(&self.bus);
        if state.commands.len() >= MAX_DEBUG_COMMANDS {
            return Err(debug_limit("the debug command queue is full"));
        }
        state
            .commands
            .push_back(DebugCommand::RemoveWatch { watch });
        Ok(())
    }

    /// Drains every queued event in order.
    #[must_use]
    pub fn drain_events(&self) -> Vec<DebugEvent> {
        let mut state = lock_bus(&self.bus);
        state.event_bytes = 0;
        state.events.drain(..).collect()
    }

    /// Returns the number of queued events.
    #[must_use]
    pub fn pending_events(&self) -> usize {
        lock_bus(&self.bus).events.len()
    }
}

struct ActiveWatch {
    target: DebugTarget,
    interval_ns: i64,
    next_due_ns: i64,
}

/// Boundary-serviced debug state owned by [`crate::Host`].
pub(crate) struct DebugEngine {
    bus: Arc<DebugBus>,
    watches: BTreeMap<WatchId, ActiveWatch>,
    last_generation: Option<u64>,
}

impl DebugEngine {
    pub(crate) fn new() -> Self {
        Self {
            bus: Arc::new(DebugBus {
                state: Mutex::new(BusState::default()),
            }),
            watches: BTreeMap::new(),
            last_generation: None,
        }
    }

    pub(crate) fn service(&self) -> DebugService {
        DebugService {
            bus: Arc::clone(&self.bus),
        }
    }

    pub(crate) fn watch_count(&self) -> usize {
        self.watches.len()
    }

    /// Ends every watch immediately at a generation transition.
    pub(crate) fn end_watches(&mut self) {
        self.watches.clear();
    }

    pub(crate) fn add_watch(&mut self, spec: WatchSpec) -> Result<WatchId> {
        validate_watch_spec(&spec)?;
        if self.watches.len() >= MAX_WATCHES {
            return Err(debug_limit(format!(
                "the watch limit of {MAX_WATCHES} is reached"
            )));
        }
        let watch = {
            let mut state = lock_bus(&self.bus);
            state.next_watch = state.next_watch.saturating_add(1).max(1);
            WatchId(state.next_watch)
        };
        self.watches.insert(
            watch,
            ActiveWatch {
                target: spec.target,
                interval_ns: spec.interval_ns,
                next_due_ns: i64::MIN,
            },
        );
        Ok(watch)
    }

    pub(crate) fn remove_watch(&mut self, watch: WatchId) -> bool {
        self.watches.remove(&watch).is_some()
    }

    pub(crate) fn drain_events(&mut self) -> Vec<DebugEvent> {
        let _ = &mut self.watches;
        self.service().drain_events()
    }

    /// Applies service commands, ends watches across a generation change, and
    /// emits at most one coalesced batch for every watch that is due.
    ///
    /// The source is `None` when no generation is active. No command or watch
    /// takes a lock held by the cycle: the caller is the cycle thread at a
    /// boundary.
    pub(crate) fn pump(&mut self, now_ns: i64, source: Option<DebugSource<'_>>) -> usize {
        let current_generation = source.as_ref().map(|source| source.generation);
        if self.last_generation != current_generation {
            // A watch session is bound to one generation; a swap ends every
            // watch. Commands applied below can register watches on the new
            // generation.
            self.watches.clear();
            self.last_generation = current_generation;
        }

        let commands: Vec<DebugCommand> = {
            let mut state = lock_bus(&self.bus);
            state.commands.drain(..).collect()
        };
        let mut emitted = 0;
        for command in commands {
            match command {
                DebugCommand::Read { request, target } => {
                    let result = match source.as_ref() {
                        Some(source) => read_request(source, &target).map_err(DebugFailure::from),
                        None => Err(DebugFailure::new(
                            codes::DEBUG_STALE,
                            "no active generation is available for a typed read",
                        )),
                    };
                    self.push_event(DebugEvent::Read(ReadCompletion { request, result }));
                    emitted += 1;
                }
                DebugCommand::AddWatch { watch, spec } => {
                    if source.is_none() {
                        self.push_event(DebugEvent::Error(DebugError {
                            code: codes::DEBUG_STALE,
                            message: "no active generation is available for a watch".into(),
                            request: None,
                            watch: Some(watch),
                        }));
                    } else if self.watches.len() >= MAX_WATCHES {
                        self.push_event(DebugEvent::Error(DebugError {
                            code: codes::DEBUG_LIMIT,
                            message: format!("the watch limit of {MAX_WATCHES} is reached"),
                            request: None,
                            watch: Some(watch),
                        }));
                    } else {
                        self.watches.insert(
                            watch,
                            ActiveWatch {
                                target: spec.target,
                                interval_ns: spec.interval_ns,
                                next_due_ns: now_ns.saturating_add(spec.interval_ns),
                            },
                        );
                    }
                    emitted += 1;
                }
                DebugCommand::RemoveWatch { watch } => {
                    self.watches.remove(&watch);
                }
            }
        }

        let Some(source) = source.as_ref() else {
            return emitted;
        };
        let due: Vec<(WatchId, DebugTarget)> = self
            .watches
            .iter()
            .filter(|(_, watch)| now_ns >= watch.next_due_ns)
            .map(|(id, watch)| (*id, watch.target.clone()))
            .collect();
        if due.is_empty() {
            return emitted;
        }
        let mut values = Vec::with_capacity(due.len());
        for (watch_id, target) in due {
            let request = DebugReadRequest::new(target.clone());
            let result = read_request(source, &request);
            let (atomic, consistent, result) = match result {
                Ok(read) => (
                    read.atomic,
                    read.consistent,
                    WatchResult::Available(TypedValue {
                        type_key: read.type_key,
                        value: read.value,
                    }),
                ),
                Err(error) => {
                    let failure = DebugFailure::from(error);
                    let warning = unavailable_warning(&failure);
                    (
                        false,
                        false,
                        WatchResult::Unavailable {
                            type_key: None,
                            error: Some(failure),
                            warning,
                        },
                    )
                }
            };
            values.push(WatchValue {
                watch_id,
                target,
                generation: source.generation,
                build_id: source.build_id,
                cycle: source.cycle,
                atomic,
                consistent,
                result,
            });
            if let Some(watch) = self.watches.get_mut(&watch_id) {
                // Missed periods coalesce into this one batch; the next due
                // point starts from the boundary that observed it.
                watch.next_due_ns = now_ns.saturating_add(watch.interval_ns);
            }
        }
        self.push_event(DebugEvent::Batch(DebugBatch {
            generation: source.generation,
            build_id: source.build_id,
            cycle: source.cycle,
            values,
            warnings: Vec::new(),
        }));
        emitted += 1;
        emitted
    }

    fn push_event(&self, event: DebugEvent) {
        let mut state = lock_bus(&self.bus);
        push_event(&mut state, event);
    }
}

pub(crate) struct DebugSource<'a> {
    pub(crate) target: &'a Target,
    pub(crate) manifest: &'a Manifest,
    pub(crate) roots: &'a crate::RootRegistry,
    pub(crate) info: Option<&'a crate::ModuleInfo>,
    pub(crate) generation: u64,
    pub(crate) build_id: GotId,
    pub(crate) cycle: u64,
}

fn lock_bus(bus: &DebugBus) -> MutexGuard<'_, BusState> {
    bus.state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn push_event(state: &mut BusState, mut event: DebugEvent) {
    let hint = event_bytes(&event);
    if matches!(event, DebugEvent::Batch(_)) {
        let mut dropped = state.dropped_watch_batch;
        let estimated = hint.saturating_add(256);
        loop {
            let batches = state
                .events
                .iter()
                .filter(|event| matches!(event, DebugEvent::Batch(_)))
                .count();
            let too_many = batches >= MAX_WATCH_BATCHES;
            let too_large = state.event_bytes.saturating_add(estimated) > MAX_EVENT_BYTES;
            if !too_many && !too_large {
                break;
            }
            let Some(index) = state
                .events
                .iter()
                .position(|event| matches!(event, DebugEvent::Batch(_)))
            else {
                break;
            };
            if let Some(removed) = state.events.remove(index) {
                state.event_bytes = state.event_bytes.saturating_sub(event_bytes(&removed));
                dropped = true;
            }
        }
        if dropped && let DebugEvent::Batch(batch) = &mut event {
            batch.warnings.push(watch_dropped_warning());
        }
        state.dropped_watch_batch = false;
        let size = event_bytes(&event);
        if size > MAX_EVENT_BYTES || state.event_bytes.saturating_add(size) > MAX_EVENT_BYTES {
            // There is no older watch batch left to evict. Drop this batch;
            // the next one carries the warning.
            state.dropped_watch_batch = true;
            return;
        }
        state.event_bytes = state.event_bytes.saturating_add(size);
        state.events.push_back(event);
        return;
    }

    // Read completions and command errors are state events: never evict them
    // to make room for a watch batch. Older watch batches may be evicted if
    // the byte cap is crossed.
    let size = hint;
    while state.event_bytes.saturating_add(size) > MAX_EVENT_BYTES {
        let Some(index) = state
            .events
            .iter()
            .position(|event| matches!(event, DebugEvent::Batch(_)))
        else {
            break;
        };
        if let Some(removed) = state.events.remove(index) {
            state.event_bytes = state.event_bytes.saturating_sub(event_bytes(&removed));
            state.dropped_watch_batch = true;
        }
    }
    state.event_bytes = state.event_bytes.saturating_add(size);
    state.events.push_back(event);
}

fn event_bytes(event: &DebugEvent) -> usize {
    match event {
        DebugEvent::Read(completion) => match &completion.result {
            Ok(read) => {
                128 + read.target_text_len()
                    + typed_value_bytes(&TypedValue {
                        type_key: read.type_key.clone(),
                        value: read.value.clone(),
                    })
            }
            Err(error) => 96 + error.message.len(),
        },
        DebugEvent::Batch(batch) => {
            128 + batch
                .values
                .iter()
                .map(|value| match &value.result {
                    WatchResult::Available(value) => 64 + typed_value_bytes(value),
                    WatchResult::Unavailable {
                        type_key, error, ..
                    } => {
                        64 + type_key.as_ref().map_or(0, String::len)
                            + error.as_ref().map_or(0, |error| error.message.len())
                    }
                })
                .sum::<usize>()
                + batch
                    .warnings
                    .iter()
                    .map(|warning| warning.message.len() + 16)
                    .sum::<usize>()
        }
        DebugEvent::Error(error) => 96 + error.message.len(),
    }
}

trait TargetTextLen {
    fn target_text_len(&self) -> usize;
}

impl TargetTextLen for DebugRead {
    fn target_text_len(&self) -> usize {
        match &self.target {
            DebugTarget::Path(path) => path.len(),
            DebugTarget::GotId(_) => 34,
            DebugTarget::Local { frame, path } => frame.len() + path.len(),
        }
    }
}

fn typed_value_bytes(value: &TypedValue) -> usize {
    value.type_key.len()
        + match &value.value {
            DebugValue::Bool(_) => 1,
            DebugValue::Decimal(text) => text.len(),
            DebugValue::Real(_) => 8,
            DebugValue::String(string) => string.text.as_ref().map_or(4, String::len),
            DebugValue::Array(values) => values.iter().map(typed_value_bytes).sum(),
            DebugValue::Aggregate(fields) => fields
                .iter()
                .map(|field| field.name.len() + typed_value_bytes(&field.value))
                .sum(),
            DebugValue::Object {
                identity,
                class,
                interface,
                fields,
                ..
            } => {
                identity.as_ref().map_or(0, String::len)
                    + class.as_ref().map_or(0, String::len)
                    + interface.as_ref().map_or(0, String::len)
                    + fields
                        .iter()
                        .map(|field| field.name.len() + typed_value_bytes(&field.value))
                        .sum::<usize>()
            }
            DebugValue::Reference {
                identity,
                target_type,
                ..
            } => {
                identity.as_ref().map_or(0, String::len)
                    + target_type.as_ref().map_or(0, String::len)
            }
            DebugValue::Overlap { bytes, .. } => bytes.len(),
            DebugValue::Unavailable { warning } => warning.message.len(),
        }
}

fn watch_dropped_warning() -> DebugWarning {
    DebugWarning {
        code: codes::W_DEBUG_WATCH_DROPPED,
        message: "a watch batch was dropped because the debug event queue was full".into(),
    }
}

fn unavailable_warning(error: &DebugFailure) -> DebugWarning {
    DebugWarning {
        code: codes::W_DEBUG_UNAVAILABLE,
        message: format!(
            "debug value is unavailable: {}: {}",
            error.code, error.message
        ),
    }
}

// ---------------------------------------------------------------------------
// Target validation and resolution
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct TargetQuery {
    package: Option<String>,
    path: String,
    qualifier: Option<String>,
}

#[derive(Clone, Debug)]
enum NormalizedTarget {
    Path(TargetQuery),
    GotId(GotId),
    Local { frame: String, path: String },
}

pub(crate) fn validate_request(request: &DebugReadRequest) -> Result<()> {
    let target = normalize_target(&request.target)?;
    if let Some(expected) = &request.expected_type_key {
        let parsed = TypeKey::parse(expected).map_err(|error| {
            Error::new(
                codes::DEBUG_TYPE,
                format!(
                    "expected type key `{expected}` is invalid: {}",
                    error.message
                ),
            )
        })?;
        if parsed.render() != *expected {
            return Err(Error::new(
                codes::DEBUG_TYPE,
                format!("expected type key `{expected}` is not canonical"),
            ));
        }
    }
    if matches!(target, NormalizedTarget::Local { .. }) && request.stop_frame.is_none() {
        return Err(Error::new(
            codes::DEBUG_STOP,
            "local targets require an immutable stop-frame snapshot",
        ));
    }
    if let Some(frame) = &request.stop_frame {
        validate_frame_token(&frame.token)?;
    }
    Ok(())
}

fn validate_watch_spec(spec: &WatchSpec) -> Result<()> {
    let normalized = normalize_target(&spec.target)?;
    if matches!(normalized, NormalizedTarget::Local { .. }) {
        return Err(Error::new(
            codes::DEBUG_STOP,
            "locals are not watchable: a stopped frame is immutable",
        ));
    }
    if spec.interval_ns < MIN_WATCH_INTERVAL_NS {
        return Err(debug_limit(format!(
            "watch interval {} ns is below the {} ns minimum",
            spec.interval_ns, MIN_WATCH_INTERVAL_NS
        )));
    }
    Ok(())
}

fn normalize_target(target: &DebugTarget) -> Result<NormalizedTarget> {
    match target {
        DebugTarget::Path(text) => Ok(NormalizedTarget::Path(normalize_path_target(text)?)),
        DebugTarget::GotId(got_id) => Ok(NormalizedTarget::GotId(*got_id)),
        DebugTarget::Local { frame, path } => {
            validate_frame_token(frame)?;
            let path = normalize_mixed_path(path, &['.']).map_err(|error| {
                Error::new(
                    codes::DEBUG_UNKNOWN,
                    format!("debug local path `{path}` is invalid: {}", error.message),
                )
            })?;
            Ok(NormalizedTarget::Local {
                frame: frame.clone(),
                path,
            })
        }
    }
}

fn validate_frame_token(frame: &str) -> Result<()> {
    let valid = !frame.is_empty()
        && frame.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':')
        });
    if !valid {
        return Err(Error::new(
            codes::DEBUG_STOP,
            format!("stop-frame token `{frame}` is invalid or unavailable"),
        ));
    }
    Ok(())
}

fn normalize_path_target(text: &str) -> Result<TargetQuery> {
    if text.is_empty() || text.trim() != text {
        return Err(unknown_target(
            text,
            "a debug path must be non-empty and have no surrounding whitespace",
        ));
    }
    if text.starts_with("0x") || text.starts_with("0X") {
        return Err(unknown_target(
            text,
            "raw addresses are not debug targets; use a stable path or GotId",
        ));
    }
    if !text.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | ':' | '/' | '#' | '-')
    }) {
        return Err(unknown_target(
            text,
            "a debug path contains a character outside the stable-key grammar",
        ));
    }
    let (package, rest) = match text.split_once("::") {
        Some((package, rest)) => {
            if rest.contains("::") {
                return Err(unknown_target(
                    text,
                    "a debug path has more than one package separator",
                ));
            }
            let package = normalize_package(package).map_err(|error| {
                unknown_target(text, &format!("package is invalid: {}", error.message))
            })?;
            (Some(package), rest)
        }
        None => (None, text),
    };
    if rest.is_empty() {
        return Err(unknown_target(
            text,
            "a debug path is empty after the package",
        ));
    }
    let (path, qualifier) = match rest.split_once('#') {
        Some((path, qualifier)) => {
            if path.is_empty() || qualifier.is_empty() || qualifier.contains('#') {
                return Err(unknown_target(
                    text,
                    "a debug path has a malformed qualifier",
                ));
            }
            let qualifier = normalize_mixed_path(qualifier, &['.', '/']).map_err(|error| {
                unknown_target(text, &format!("qualifier is invalid: {}", error.message))
            })?;
            (path, Some(qualifier))
        }
        None => (rest, None),
    };
    let path = normalize_mixed_path(path, &['.', '/'])
        .map_err(|error| unknown_target(text, &format!("path is invalid: {}", error.message)))?;
    Ok(TargetQuery {
        package,
        path,
        qualifier,
    })
}

fn normalize_mixed_path(path: &str, separators: &[char]) -> Result<String> {
    if path.is_empty() {
        return Err(Error::manifest("path is empty"));
    }
    let mut out = String::with_capacity(path.len());
    let mut segment = String::new();
    for character in path.chars() {
        if separators.contains(&character) {
            if segment.is_empty() {
                return Err(Error::manifest("path contains an empty segment"));
            }
            out.push_str(&normalize_ident(&segment)?);
            out.push(character);
            segment.clear();
        } else {
            segment.push(character);
        }
    }
    if segment.is_empty() {
        return Err(Error::manifest("path contains an empty segment"));
    }
    out.push_str(&normalize_ident(&segment)?);
    Ok(out)
}

fn unknown_target(input: &str, detail: &str) -> Error {
    Error::new(
        codes::DEBUG_UNKNOWN,
        format!("debug target `{input}` is not valid: {detail}"),
    )
}

struct ParsedKey<'a> {
    package: &'a str,
    path: &'a str,
    qualifier: Option<&'a str>,
}

fn parse_key(key: &str) -> Option<ParsedKey<'_>> {
    let (package, rest) = key.split_once("::")?;
    let (path, qualifier) = match rest.split_once('#') {
        Some((path, qualifier)) => (path, Some(qualifier)),
        None => (rest, None),
    };
    if package.is_empty() || path.is_empty() {
        return None;
    }
    Some(ParsedKey {
        package,
        path,
        qualifier,
    })
}

fn hidden_qualifier(qualifier: &str) -> Option<(Option<&str>, &str)> {
    for marker in ["HANDLE", "OBJREC"] {
        if qualifier == marker {
            return Some((None, marker));
        }
        if let Some(base) = qualifier.strip_suffix(&format!("/{marker}")) {
            return Some(((!base.is_empty()).then_some(base), marker));
        }
    }
    None
}

fn is_hidden_qualifier(qualifier: &str) -> bool {
    hidden_qualifier(qualifier).is_some()
}

fn root_matches_query(parsed: &ParsedKey<'_>, query: &TargetQuery) -> bool {
    query
        .package
        .as_deref()
        .is_none_or(|package| package == parsed.package)
        && parsed.path == query.path
}

fn exact_root_matches(parsed: &ParsedKey<'_>, query: &TargetQuery) -> bool {
    if !root_matches_query(parsed, query) {
        return false;
    }
    match query.qualifier.as_deref() {
        Some(qualifier) => parsed.qualifier == Some(qualifier),
        None => parsed
            .qualifier
            .is_none_or(|qualifier| !is_hidden_qualifier(qualifier)),
    }
}

#[derive(Clone, Debug)]
struct Resolved {
    canonical_key: String,
    type_key: String,
    layout_hash: GotId,
    atomic: bool,
    shape: ResolvedShape,
}

#[derive(Clone, Debug)]
enum ResolvedShape {
    Root(RootRecord),
    Aggregate(AggregateTarget),
}

#[derive(Clone, Debug)]
struct AggregateTarget {
    ty: TypeRecord,
    package: String,
    path: String,
    qualifier: Option<String>,
}

fn resolve_target(manifest: &Manifest, target: &DebugTarget) -> Result<Resolved> {
    let normalized = normalize_target(target)?;
    match normalized {
        NormalizedTarget::Path(query) => resolve_path(manifest, &query),
        NormalizedTarget::GotId(got_id) => resolve_got_id(manifest, got_id),
        NormalizedTarget::Local { .. } => Err(Error::new(
            codes::DEBUG_STOP,
            "unreachable local resolution",
        )),
    }
}

fn resolve_path(manifest: &Manifest, query: &TargetQuery) -> Result<Resolved> {
    let mut exact = Vec::new();
    for root in &manifest.roots {
        let Some(parsed) = parse_key(&root.key) else {
            continue;
        };
        if exact_root_matches(&parsed, query) {
            exact.push(root);
        }
    }
    if exact.len() > 1 {
        let mut keys: Vec<&str> = exact.iter().map(|root| root.key.as_str()).collect();
        keys.sort_unstable();
        let shown = keys.into_iter().take(4).collect::<Vec<_>>().join(", ");
        return Err(Error::new(
            codes::DEBUG_UNKNOWN,
            format!(
                "debug path `{}` is ambiguous; supply a package, instance qualifier, or GotId (matches {shown})",
                query.path
            ),
        ));
    }
    if let Some(root) = exact.first() {
        let parsed = parse_key(&root.key);
        if let Some(parsed) = parsed
            && retargeted_instance(
                manifest,
                query,
                parsed.package,
                parsed
                    .qualifier
                    .filter(|qualifier| !is_hidden_qualifier(qualifier)),
            )
        {
            return Err(retarget_error(&query.path));
        }
        return resolved_root(manifest, root);
    }

    let groups = aggregate_groups(manifest, query);
    if groups.len() > 1 {
        let names: Vec<String> = groups
            .iter()
            .map(|group| group.canonical(&query.path))
            .collect();
        return Err(Error::new(
            codes::DEBUG_UNKNOWN,
            format!(
                "debug path `{}` is ambiguous across {} storage groups; supply an instance qualifier or GotId",
                query.path,
                names.join(", ")
            ),
        ));
    }
    if let Some(group) = groups.first() {
        if retargeted_instance(manifest, query, group.package, group.qualifier.as_deref()) {
            return Err(retarget_error(&query.path));
        }
        let aggregate = build_aggregate(manifest, group, query)?;
        return resolved_aggregate(aggregate);
    }

    if path_removed(manifest, query) {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "debug path `{}` was removed or renamed in the active generation",
                query.path
            ),
        ));
    }
    Err(Error::new(
        codes::DEBUG_UNKNOWN,
        format!("debug path `{}` is not in the active manifest", query.path),
    ))
}

fn resolve_got_id(manifest: &Manifest, got_id: GotId) -> Result<Resolved> {
    if let Some(root) = manifest.roots.iter().find(|root| root.got_id == got_id) {
        return resolved_root(manifest, root);
    }
    let action_removed = manifest
        .plan
        .actions
        .iter()
        .any(|action| action.got_id == got_id);
    let history_removed = manifest
        .history
        .retired
        .iter()
        .any(|record| record.got_id == got_id);
    if action_removed || history_removed {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "debug target {} was removed in the active generation",
                got_id.to_hex()
            ),
        ));
    }
    Err(Error::new(
        codes::DEBUG_UNKNOWN,
        format!(
            "debug target {} is not in the active manifest",
            got_id.to_hex()
        ),
    ))
}

fn resolved_root(manifest: &Manifest, root: &RootRecord) -> Result<Resolved> {
    let ty = TypeKey::parse(&root.type_key).map_err(|error| {
        Error::new(
            codes::DEBUG_TYPE,
            format!(
                "active root `{}` has an invalid type key: {}",
                root.key, error.message
            ),
        )
    })?;
    Ok(Resolved {
        canonical_key: root.key.clone(),
        type_key: root.type_key.clone(),
        layout_hash: root.layout_hash,
        atomic: is_single_root_type(manifest, &ty, 0),
        shape: ResolvedShape::Root(root.clone()),
    })
}

fn resolved_aggregate(aggregate: AggregateTarget) -> Result<Resolved> {
    Ok(Resolved {
        canonical_key: canonical_key(
            &aggregate.package,
            &aggregate.path,
            aggregate.qualifier.as_deref(),
        ),
        type_key: aggregate.ty.type_key.clone(),
        layout_hash: aggregate.ty.layout_hash,
        atomic: false,
        shape: ResolvedShape::Aggregate(aggregate),
    })
}

fn check_type_layout(resolved: &Resolved, request: &DebugReadRequest) -> Result<()> {
    if let Some(expected) = &request.expected_type_key
        && expected != &resolved.type_key
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "expected type `{expected}` but the target has type `{}`",
                resolved.type_key
            ),
        ));
    }
    if let Some(expected) = request.expected_layout_hash
        && expected != resolved.layout_hash
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "expected layout {} but the target has layout {}",
                expected.to_hex(),
                resolved.layout_hash.to_hex()
            ),
        ));
    }
    Ok(())
}

fn check_generation_build(source: &DebugSource<'_>, request: &DebugReadRequest) -> Result<()> {
    if let Some(expected) = request.expected_generation
        && expected != source.generation
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "expected generation {expected} but generation {} is active",
                source.generation
            ),
        ));
    }
    if let Some(expected) = request.expected_build_id
        && expected != source.build_id
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "expected build {} but build {} is active",
                expected.to_hex(),
                source.build_id.to_hex()
            ),
        ));
    }
    Ok(())
}

fn retarget_error(path: &str) -> Error {
    Error::new(
        codes::DEBUG_STALE,
        format!(
            "debug path `{path}` now resolves to a different instance; use an explicit instance qualifier or GotId"
        ),
    )
}

/// Returns `true` when an unqualified path would silently move to another
/// instance because a previously qualified target for the same path is gone.
///
/// An unqualified query must never retarget a same-named field that lived in a
/// different instance. The active manifest does not retain the previous build,
/// so any prior key for the path with a different non-hidden qualifier is
/// treated as evidence of a retarget. A hidden `HANDLE`/`OBJREC` root is part
/// of the same instance and does not count.
fn retargeted_instance(
    manifest: &Manifest,
    query: &TargetQuery,
    chosen_package: &str,
    chosen_qualifier: Option<&str>,
) -> bool {
    if query.qualifier.is_some() {
        return false;
    }
    let Some(chosen_qualifier) = chosen_qualifier else {
        return false;
    };
    let mut previous_keys: Vec<&str> = Vec::new();
    for root in &manifest.roots {
        previous_keys.extend(root.previous_keys.iter().map(String::as_str));
    }
    for retired in &manifest.history.retired {
        previous_keys.push(retired.key.as_str());
        previous_keys.extend(retired.previous_keys.iter().map(String::as_str));
    }
    for action in &manifest.plan.actions {
        if let Some(old) = &action.old_key {
            previous_keys.push(old);
        }
        if matches!(action.action, PlanAction::Drop | PlanAction::Reset) {
            previous_keys.push(&action.key);
        }
    }
    previous_keys.into_iter().any(|key| {
        let Some(parsed) = parse_key(key) else {
            return false;
        };
        if parsed.package != chosen_package || parsed.path != query.path {
            return false;
        }
        match parsed.qualifier {
            Some(qualifier) => !is_hidden_qualifier(qualifier) && qualifier != chosen_qualifier,
            None => true,
        }
    })
}

fn path_removed(manifest: &Manifest, query: &TargetQuery) -> bool {
    for root in &manifest.roots {
        for previous in &root.previous_keys {
            if let Some(parsed) = parse_key(previous)
                && previous_matches(&parsed, query)
            {
                return true;
            }
        }
    }
    for retired in &manifest.history.retired {
        if let Some(parsed) = parse_key(&retired.key)
            && previous_matches(&parsed, query)
        {
            return true;
        }
        for previous in &retired.previous_keys {
            if let Some(parsed) = parse_key(previous)
                && previous_matches(&parsed, query)
            {
                return true;
            }
        }
    }
    for action in &manifest.plan.actions {
        if let Some(old) = &action.old_key
            && let Some(parsed) = parse_key(old)
            && previous_matches(&parsed, query)
        {
            return true;
        }
        if let Some(parsed) = parse_key(&action.key)
            && previous_matches(&parsed, query)
            && matches!(action.action, PlanAction::Drop | PlanAction::Reset)
        {
            return true;
        }
    }
    false
}

fn previous_matches(parsed: &ParsedKey<'_>, query: &TargetQuery) -> bool {
    query
        .package
        .as_deref()
        .is_none_or(|package| package == parsed.package)
        && parsed.path == query.path
        && match query.qualifier.as_deref() {
            Some(qualifier) => parsed.qualifier == Some(qualifier),
            None => parsed
                .qualifier
                .is_none_or(|qualifier| !is_hidden_qualifier(qualifier)),
        }
}

struct Group<'a> {
    package: &'a str,
    qualifier: Option<String>,
    roots: Vec<&'a RootRecord>,
}

impl Group<'_> {
    fn canonical(&self, path: &str) -> String {
        canonical_key(self.package, path, self.qualifier.as_deref())
    }
}

fn aggregate_groups<'a>(manifest: &'a Manifest, query: &TargetQuery) -> Vec<Group<'a>> {
    let prefix = format!("{}.", query.path);
    let mut groups: BTreeMap<(String, Option<String>), Group<'a>> = BTreeMap::new();
    for root in &manifest.roots {
        let Some(parsed) = parse_key(&root.key) else {
            continue;
        };
        if query
            .package
            .as_deref()
            .is_some_and(|package| package != parsed.package)
        {
            continue;
        }
        if parsed.path != query.path && !parsed.path.starts_with(&prefix) {
            continue;
        }
        let base = match parsed.qualifier {
            Some(qualifier) => match hidden_qualifier(qualifier) {
                Some((base, _)) => base.map(str::to_owned),
                None => Some(qualifier.to_owned()),
            },
            None => None,
        };
        if let Some(expected) = query.qualifier.as_deref()
            && base.as_deref() != Some(expected)
        {
            continue;
        }
        let key = (parsed.package.to_owned(), base.clone());
        groups
            .entry(key)
            .or_insert_with(|| Group {
                package: parsed.package,
                qualifier: base,
                roots: Vec::new(),
            })
            .roots
            .push(root);
    }
    groups.into_values().collect()
}

fn canonical_key(package: &str, path: &str, qualifier: Option<&str>) -> String {
    let mut out = String::with_capacity(package.len() + path.len() + 8);
    out.push_str(package);
    out.push_str("::");
    out.push_str(path);
    if let Some(qualifier) = qualifier {
        out.push('#');
        out.push_str(qualifier);
    }
    out
}

fn build_aggregate(
    manifest: &Manifest,
    group: &Group<'_>,
    query: &TargetQuery,
) -> Result<AggregateTarget> {
    let package = group.package.to_owned();
    // First honor a hidden class handle or FB object record: the compiler
    // persisted the object type there even when no field leaf is present.
    for marker in ["HANDLE", "OBJREC"] {
        if let Some(root) = find_hidden_root(
            manifest,
            group.package,
            &query.path,
            group.qualifier.as_deref(),
            marker,
        ) {
            let ty = hidden_type(manifest, root, marker)?;
            return Ok(AggregateTarget {
                ty,
                package,
                path: query.path.clone(),
                qualifier: group.qualifier.clone(),
            });
        }
    }

    let mut candidate: Option<TypeRecord> = None;
    for root in &group.roots {
        if parse_key(&root.key)
            .and_then(|parsed| parsed.qualifier)
            .is_some_and(is_hidden_qualifier)
        {
            continue;
        }
        let Some(owner_id) = root.owner_type else {
            continue;
        };
        let Some(owner) = manifest.types.iter().find(|ty| ty.got_id == owner_id) else {
            continue;
        };
        if root.field_path.is_empty() {
            continue;
        }
        let Some(declaration) = declaration_path(&root.key, &root.field_path) else {
            continue;
        };
        if query.path != declaration && !query.path.starts_with(&format!("{declaration}.")) {
            continue;
        }
        if has_element_segment(&root.field_path) {
            return Err(Error::new(
                codes::DEBUG_UNKNOWN,
                format!(
                    "debug path `{}` names an array-of-aggregate storage whose parent shape is not in manifest format 1; read a field leaf instead",
                    query.path
                ),
            ));
        }
        let remainder = query
            .path
            .strip_prefix(declaration)
            .map_or("", |text| text.strip_prefix('.').unwrap_or(text));
        let ty = traverse_fields(manifest, owner, remainder)?;
        if let Some(previous) = &candidate {
            if previous.got_id != ty.got_id {
                return Err(Error::new(
                    codes::DEBUG_UNKNOWN,
                    format!(
                        "debug path `{}` maps to more than one manifest type",
                        query.path
                    ),
                ));
            }
        } else {
            candidate = Some(ty);
        }
    }
    let Some(ty) = candidate else {
        return Err(Error::new(
            codes::DEBUG_UNKNOWN,
            format!(
                "debug path `{}` has split roots but manifest format 1 does not identify its aggregate type",
                query.path
            ),
        ));
    };
    Ok(AggregateTarget {
        ty,
        package,
        path: query.path.clone(),
        qualifier: group.qualifier.clone(),
    })
}

fn declaration_path<'a>(key: &'a str, field_path: &str) -> Option<&'a str> {
    let (_, rest) = key.split_once("::")?;
    let (path, _) = match rest.split_once('#') {
        Some((path, _)) => (path, true),
        None => (rest, false),
    };
    path.strip_suffix(&format!(".{field_path}"))
}

fn has_element_segment(field_path: &str) -> bool {
    field_path.split('.').next().is_some_and(is_element_segment)
}

fn is_element_segment(segment: &str) -> bool {
    let Some(index) = segment.strip_prefix('E') else {
        return false;
    };
    !index.is_empty() && index.chars().all(|character| character.is_ascii_digit())
}

fn traverse_fields(manifest: &Manifest, owner: &TypeRecord, remainder: &str) -> Result<TypeRecord> {
    if remainder.is_empty() {
        return Ok(owner.clone());
    }
    let (segment, tail) = remainder
        .split_once('.')
        .map_or((remainder, ""), |(segment, tail)| (segment, tail));
    if is_element_segment(segment) {
        return Err(Error::new(
            codes::DEBUG_UNKNOWN,
            format!(
                "debug path enters array element `{segment}`, whose aggregate parent shape is not in manifest format 1"
            ),
        ));
    }
    let Some(field) = owner
        .fields
        .iter()
        .find(|field| field.path == segment || field.path.starts_with(&format!("{segment}.")))
    else {
        return Err(Error::new(
            codes::DEBUG_UNKNOWN,
            format!(
                "type `{}` has no field `{segment}` for this debug path",
                owner.type_key
            ),
        ));
    };
    if field.path != segment {
        return Err(Error::new(
            codes::DEBUG_UNKNOWN,
            format!(
                "manifest field path `{}` is flattened; the intermediate aggregate type cannot be resolved",
                field.path
            ),
        ));
    }
    let field_ty = TypeKey::parse(&field.type_key).map_err(|error| {
        Error::new(
            codes::DEBUG_TYPE,
            format!(
                "field `{}` of `{}` has an invalid type key: {}",
                field.path, owner.type_key, error.message
            ),
        )
    })?;
    let TypeKey::Named(got_id) = field_ty else {
        return Err(Error::new(
            codes::DEBUG_UNKNOWN,
            format!(
                "field `{}` of `{}` is not a named aggregate",
                field.path, owner.type_key
            ),
        ));
    };
    let Some(ty) = manifest.types.iter().find(|ty| ty.got_id == got_id) else {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!(
                "field `{}` names type {} which has no manifest type record",
                field.path,
                got_id.to_hex()
            ),
        ));
    };
    traverse_fields(manifest, ty, tail)
}

fn hidden_type(manifest: &Manifest, root: &RootRecord, marker: &str) -> Result<TypeRecord> {
    let key = TypeKey::parse(&root.type_key).map_err(|error| {
        Error::new(
            codes::DEBUG_TYPE,
            format!(
                "hidden root `{}` has an invalid type key: {}",
                root.key, error.message
            ),
        )
    })?;
    let key = if marker == "OBJREC" {
        match key {
            TypeKey::Array { element, .. } => *element,
            other => other,
        }
    } else {
        key
    };
    let inner = match key {
        TypeKey::RefTo(inner) | TypeKey::PtrTo(inner) => *inner,
        _ => {
            return Err(Error::new(
                codes::DEBUG_UNKNOWN,
                format!("hidden root `{}` does not carry an object handle", root.key),
            ));
        }
    };
    let TypeKey::Named(got_id) = inner else {
        return Err(Error::new(
            codes::DEBUG_UNKNOWN,
            format!("hidden root `{}` does not name a manifest type", root.key),
        ));
    };
    manifest
        .types
        .iter()
        .find(|ty| ty.got_id == got_id)
        .cloned()
        .ok_or_else(|| {
            Error::new(
                codes::DEBUG_TYPE,
                format!(
                    "hidden root `{}` names type {} with no manifest type record",
                    root.key,
                    got_id.to_hex()
                ),
            )
        })
}

fn type_by_id(manifest: &Manifest, got_id: GotId) -> Option<&TypeRecord> {
    manifest.types.iter().find(|ty| ty.got_id == got_id)
}

fn find_hidden_root<'a>(
    manifest: &'a Manifest,
    package: &str,
    path: &str,
    qualifier: Option<&str>,
    marker: &str,
) -> Option<&'a RootRecord> {
    let expected = qualifier.map_or_else(|| marker.to_owned(), |base| format!("{base}/{marker}"));
    manifest.roots.iter().find(|root| {
        parse_key(&root.key).is_some_and(|parsed| {
            parsed.package == package
                && parsed.path == path
                && parsed.qualifier == Some(expected.as_str())
        })
    })
}

fn find_field_root<'a>(
    manifest: &'a Manifest,
    package: &str,
    path: &str,
    qualifier: Option<&str>,
) -> Option<&'a RootRecord> {
    manifest.roots.iter().find(|root| {
        parse_key(&root.key).is_some_and(|parsed| {
            parsed.package == package && parsed.path == path && parsed.qualifier == qualifier
        })
    })
}

fn is_single_root_type(manifest: &Manifest, ty: &TypeKey, depth: usize) -> bool {
    if depth > MAX_DECODE_DEPTH {
        return false;
    }
    match ty {
        TypeKey::Bool
        | TypeKey::Sint
        | TypeKey::Int
        | TypeKey::Dint
        | TypeKey::Lint
        | TypeKey::Usint
        | TypeKey::Uint
        | TypeKey::Udint
        | TypeKey::Ulint
        | TypeKey::Real
        | TypeKey::Lreal
        | TypeKey::Time
        | TypeKey::Ltime
        | TypeKey::Date
        | TypeKey::Ldate
        | TypeKey::Tod
        | TypeKey::Ltod
        | TypeKey::Dt
        | TypeKey::Ldt
        | TypeKey::Byte
        | TypeKey::Word
        | TypeKey::Dword
        | TypeKey::Lword
        | TypeKey::Char
        | TypeKey::Wchar
        | TypeKey::Uchar
        | TypeKey::String { .. } => true,
        TypeKey::Array { element, .. } => is_single_root_type(manifest, element, depth + 1),
        TypeKey::RefTo(_) | TypeKey::PtrTo(_) => false,
        TypeKey::Named(got_id) => {
            let Some(record) = type_by_id(manifest, *got_id) else {
                return false;
            };
            match record.type_kind {
                TypeKind::Enum | TypeKind::Subrange => true,
                TypeKind::Alias => record.base.is_some_and(|base| {
                    type_by_id(manifest, base)
                        .and_then(|base| TypeKey::parse(&base.type_key).ok())
                        .is_some_and(|base| is_single_root_type(manifest, &base, depth + 1))
                }),
                TypeKind::Struct => is_overlap(record),
                TypeKind::FunctionBlock | TypeKind::Class | TypeKind::Interface => false,
            }
        }
        TypeKey::Void => false,
    }
}

fn is_overlap(record: &TypeRecord) -> bool {
    if record.fields.len() < 2 {
        return false;
    }
    for (index, field) in record.fields.iter().enumerate() {
        if record.fields[..index]
            .iter()
            .any(|previous| previous.offset == field.offset)
        {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Reads and decoding
// ---------------------------------------------------------------------------

pub(crate) fn read_request(
    source: &DebugSource<'_>,
    request: &DebugReadRequest,
) -> Result<DebugRead> {
    match normalize_target(&request.target)? {
        NormalizedTarget::Local { frame, path } => read_local(source, request, &frame, &path),
        NormalizedTarget::Path(_) | NormalizedTarget::GotId(_) => {
            let resolved = resolve_target(source.manifest, &request.target)?;
            check_generation_build(source, request)?;
            check_type_layout(&resolved, request)?;
            let typed = match &resolved.shape {
                ResolvedShape::Root(root) => {
                    let bytes = root_bytes(source, root)?;
                    let ty = TypeKey::parse(&root.type_key).map_err(|error| {
                        Error::new(
                            codes::DEBUG_TYPE,
                            format!(
                                "active root `{}` has an invalid type key: {}",
                                root.key, error.message
                            ),
                        )
                    })?;
                    decode_type_value(
                        source.manifest,
                        source.target,
                        &ty,
                        &root.type_key,
                        &bytes,
                        source.generation,
                    )?
                }
                ResolvedShape::Aggregate(aggregate) => decode_aggregate(source, aggregate)?,
            };
            Ok(DebugRead {
                target: request.target.clone(),
                resolved_key: resolved.canonical_key,
                generation: source.generation,
                build_id: source.build_id,
                cycle: source.cycle,
                atomic: resolved.atomic,
                consistent: true,
                type_key: typed.type_key,
                value: typed.value,
            })
        }
    }
}

fn read_local(
    source: &DebugSource<'_>,
    request: &DebugReadRequest,
    frame_token: &str,
    path: &str,
) -> Result<DebugRead> {
    let Some(frame) = &request.stop_frame else {
        return Err(Error::new(
            codes::DEBUG_STOP,
            format!("stop frame `{frame_token}` is unavailable"),
        ));
    };
    if frame.token != frame_token {
        return Err(Error::new(
            codes::DEBUG_STOP,
            format!(
                "stop frame `{frame_token}` is not the supplied frame `{}`",
                frame.token
            ),
        ));
    }
    if frame.generation != source.generation || frame.build_id != source.build_id {
        return Err(Error::new(
            codes::DEBUG_STOP,
            format!(
                "stop frame `{frame_token}` is stale: it was captured at generation {} build {}, active generation is {} build {}",
                frame.generation,
                frame.build_id.to_hex(),
                source.generation,
                source.build_id.to_hex()
            ),
        ));
    }
    if frame.cycle > source.cycle {
        return Err(Error::new(
            codes::DEBUG_STOP,
            format!(
                "stop frame `{frame_token}` is from cycle {} but only {} cycles have completed",
                frame.cycle, source.cycle
            ),
        ));
    }
    if let Some(expected) = request.expected_generation
        && expected != frame.generation
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "expected generation {expected} but the stop frame is generation {}",
                frame.generation
            ),
        ));
    }
    if let Some(expected) = request.expected_build_id
        && expected != frame.build_id
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "expected build {} but the stop frame is build {}",
                expected.to_hex(),
                frame.build_id.to_hex()
            ),
        ));
    }
    let Some(local) = frame.locals.get(path) else {
        return Err(Error::new(
            codes::DEBUG_UNKNOWN,
            format!("local `{path}` is not in stop frame `{frame_token}`"),
        ));
    };
    if let Some(expected) = &request.expected_type_key
        && expected != &local.value.type_key
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "expected type `{expected}` but local `{path}` has type `{}`",
                local.value.type_key
            ),
        ));
    }
    if request.expected_layout_hash.is_some() {
        return Err(Error::new(
            codes::DEBUG_STALE,
            "stopped locals do not carry a layout hash",
        ));
    }
    Ok(DebugRead {
        target: request.target.clone(),
        resolved_key: path.to_owned(),
        generation: frame.generation,
        build_id: frame.build_id,
        cycle: frame.cycle,
        atomic: local.atomic,
        consistent: true,
        type_key: local.value.type_key.clone(),
        value: local.value.value.clone(),
    })
}

fn root_bytes(source: &DebugSource<'_>, root: &RootRecord) -> Result<Vec<u8>> {
    if root.root_kind == RootKind::Direct {
        let Some(info) = source.info else {
            return Err(Error::new(
                codes::DEBUG_STALE,
                format!(
                    "direct root `{}` has no active process-image metadata",
                    root.key
                ),
            ));
        };
        let Some(binding) = info.binding(root.got_id) else {
            return Err(Error::new(
                codes::DEBUG_STALE,
                format!(
                    "direct root `{}` has no active process-image binding",
                    root.key
                ),
            ));
        };
        let Some(layout) = info.image_layout().area(binding.area) else {
            return Err(Error::new(
                codes::DEBUG_STALE,
                format!("direct root `{}` names an invalid image area", root.key),
            ));
        };
        let byte_offset = binding.offset_bits / 8;
        let width_bytes = binding.width_bits.div_ceil(8).max(1);
        let size = usize::try_from(root.size).map_err(|_| {
            Error::new(
                codes::DEBUG_TYPE,
                format!("direct root `{}` is too large to read", root.key),
            )
        })?;
        if binding.width_bits == 1 {
            // A one-bit binding shares its byte with other direct variables.
            // Read the containing byte and isolate the declared bit.
            if size != 1 {
                return Err(Error::new(
                    codes::DEBUG_TYPE,
                    format!("one-bit direct root `{}` has a {size}-byte value", root.key),
                ));
            }
            if byte_offset.checked_add(1).is_none_or(|end| end > layout.1) {
                return Err(Error::new(
                    codes::DEBUG_STALE,
                    format!(
                        "direct root `{}` storage does not fit its process-image binding",
                        root.key
                    ),
                ));
            }
            let absolute = layout
                .0
                .checked_add(byte_offset)
                .and_then(|offset| usize::try_from(offset).ok())
                .ok_or_else(|| {
                    Error::new(
                        codes::DEBUG_STALE,
                        format!("direct root `{}` process-image offset overflows", root.key),
                    )
                })?;
            let mut byte = [0_u8; 1];
            if crate::image_read(absolute, &mut byte) != 1 {
                return Err(Error::new(
                    codes::DEBUG_STALE,
                    format!(
                        "direct root `{}` process-image range is unavailable",
                        root.key
                    ),
                ));
            }
            let bit = u8::try_from(binding.offset_bits % 8).unwrap_or(0);
            return Ok(vec![(byte[0] >> bit) & 1]);
        }
        if binding.offset_bits % 8 != 0 {
            return Err(Error::new(
                codes::DEBUG_TYPE,
                format!(
                    "direct root `{}` starts at bit {}, not a byte boundary",
                    root.key, binding.offset_bits
                ),
            ));
        }
        if size > usize::try_from(width_bytes).unwrap_or(usize::MAX)
            || byte_offset
                .checked_add(width_bytes)
                .is_none_or(|end| end > layout.1)
        {
            return Err(Error::new(
                codes::DEBUG_STALE,
                format!(
                    "direct root `{}` storage does not fit its process-image binding",
                    root.key
                ),
            ));
        }
        let absolute = layout
            .0
            .checked_add(byte_offset)
            .and_then(|offset| usize::try_from(offset).ok())
            .ok_or_else(|| {
                Error::new(
                    codes::DEBUG_STALE,
                    format!("direct root `{}` process-image offset overflows", root.key),
                )
            })?;
        let mut bytes = vec![0_u8; size];
        if crate::image_read(absolute, &mut bytes) != size {
            return Err(Error::new(
                codes::DEBUG_STALE,
                format!(
                    "direct root `{}` process-image range is unavailable",
                    root.key
                ),
            ));
        }
        return Ok(bytes);
    }
    source
        .roots
        .storage(root.got_id)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| {
            Error::new(
                codes::DEBUG_STALE,
                format!(
                    "root `{}` ({}) has no active storage",
                    root.key,
                    root.got_id.to_hex()
                ),
            )
        })
}

fn decode_aggregate(source: &DebugSource<'_>, aggregate: &AggregateTarget) -> Result<TypedValue> {
    let ty = &aggregate.ty;
    let mut fields = Vec::with_capacity(ty.fields.len());
    for field in &ty.fields {
        fields.push(decode_aggregate_field(source, aggregate, field)?);
    }
    match ty.type_kind {
        TypeKind::Class => {
            let (identity, present) = object_identity(source, aggregate)?;
            Ok(TypedValue {
                type_key: ty.type_key.clone(),
                value: DebugValue::Object {
                    identity,
                    present,
                    class: Some(ty.type_key.clone()),
                    interface: None,
                    fields,
                },
            })
        }
        TypeKind::Struct | TypeKind::FunctionBlock => Ok(TypedValue {
            type_key: ty.type_key.clone(),
            value: DebugValue::Aggregate(fields),
        }),
        TypeKind::Interface => Ok(TypedValue {
            type_key: ty.type_key.clone(),
            value: DebugValue::Object {
                identity: None,
                present: false,
                class: None,
                interface: Some(ty.type_key.clone()),
                fields: Vec::new(),
            },
        }),
        TypeKind::Enum | TypeKind::Subrange | TypeKind::Alias => Err(Error::new(
            codes::DEBUG_UNKNOWN,
            format!("type `{}` is not a split aggregate object", ty.type_key),
        )),
    }
}

fn decode_aggregate_field(
    source: &DebugSource<'_>,
    aggregate: &AggregateTarget,
    field: &FieldRecord,
) -> Result<NamedValue> {
    let field_path = format!("{}.{}", aggregate.path, field.path);
    if let Some(root) = find_field_root(
        source.manifest,
        &aggregate.package,
        &field_path,
        aggregate.qualifier.as_deref(),
    ) {
        if root.type_key != field.type_key {
            return Err(Error::new(
                codes::DEBUG_TYPE,
                format!(
                    "root `{}` has type `{}` but field `{}` declares `{}`",
                    root.key, root.type_key, field.path, field.type_key
                ),
            ));
        }
        if root.layout_hash != field.layout_hash {
            return Err(Error::new(
                codes::DEBUG_STALE,
                format!(
                    "root `{}` layout does not match field `{}`",
                    root.key, field.path
                ),
            ));
        }
        let bytes = root_bytes(source, root)?;
        let ty = TypeKey::parse(&field.type_key).map_err(|error| {
            Error::new(
                codes::DEBUG_TYPE,
                format!(
                    "field `{}` has an invalid type key: {}",
                    field.path, error.message
                ),
            )
        })?;
        let value = decode_type_value(
            source.manifest,
            source.target,
            &ty,
            &field.type_key,
            &bytes,
            source.generation,
        )?;
        return Ok(NamedValue {
            name: field.path.clone(),
            value,
        });
    }

    let ty = TypeKey::parse(&field.type_key).map_err(|error| {
        Error::new(
            codes::DEBUG_TYPE,
            format!(
                "field `{}` has an invalid type key: {}",
                field.path, error.message
            ),
        )
    })?;
    match &ty {
        TypeKey::Named(got_id) => {
            let Some(record) = type_by_id(source.manifest, *got_id) else {
                return Ok(NamedValue {
                    name: field.path.clone(),
                    value: unavailable_value(field.type_key.clone()),
                });
            };
            match record.type_kind {
                TypeKind::Struct | TypeKind::FunctionBlock | TypeKind::Class => {
                    let child = AggregateTarget {
                        ty: record.clone(),
                        package: aggregate.package.clone(),
                        path: field_path,
                        qualifier: aggregate.qualifier.clone(),
                    };
                    let value = decode_aggregate(source, &child)?;
                    Ok(NamedValue {
                        name: field.path.clone(),
                        value,
                    })
                }
                TypeKind::Interface => Ok(NamedValue {
                    name: field.path.clone(),
                    value: unavailable_value(field.type_key.clone()),
                }),
                TypeKind::Enum | TypeKind::Subrange | TypeKind::Alias => Ok(NamedValue {
                    name: field.path.clone(),
                    value: unavailable_value(field.type_key.clone()),
                }),
            }
        }
        TypeKey::Array { .. } => match decode_split_array(source, aggregate, field, &ty)? {
            Some(value) => Ok(NamedValue {
                name: field.path.clone(),
                value,
            }),
            None => Ok(NamedValue {
                name: field.path.clone(),
                value: unavailable_value(field.type_key.clone()),
            }),
        },
        _ => Ok(NamedValue {
            name: field.path.clone(),
            value: unavailable_value(field.type_key.clone()),
        }),
    }
}

fn decode_split_array(
    source: &DebugSource<'_>,
    aggregate: &AggregateTarget,
    field: &FieldRecord,
    ty: &TypeKey,
) -> Result<Option<TypedValue>> {
    let TypeKey::Array {
        lower: ArrayBound::Value(lower),
        upper: ArrayBound::Value(upper),
        element,
    } = ty
    else {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!(
                "split array field `{}` has a conformant or invalid bound",
                field.path
            ),
        ));
    };
    if upper < lower {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!("split array field `{}` has inverted bounds", field.path),
        ));
    }
    let count = u128::try_from(*upper - *lower)
        .ok()
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| {
            Error::new(
                codes::DEBUG_LIMIT,
                format!("split array field `{}` is too large", field.path),
            )
        })?;
    if count > 1_000_000 {
        return Err(Error::new(
            codes::DEBUG_LIMIT,
            format!(
                "split array field `{}` has more than one million elements",
                field.path
            ),
        ));
    }
    let base_path = format!("{}.{}", aggregate.path, field.path);
    let mut values = Vec::with_capacity(usize::try_from(count).unwrap_or(usize::MAX));
    let mut any = false;
    for index in 0..count {
        let child_path = format!("{base_path}.E{index}");
        let present = source.manifest.roots.iter().any(|root| {
            parse_key(&root.key).is_some_and(|parsed| {
                parsed.package == aggregate.package
                    && (parsed.path == child_path
                        || parsed.path.starts_with(&format!("{child_path}.")))
            })
        });
        if !present {
            values.push(unavailable_typed(element.render()));
            continue;
        }
        any = true;
        if let Some(root) = find_field_root(
            source.manifest,
            &aggregate.package,
            &child_path,
            aggregate.qualifier.as_deref(),
        ) {
            let bytes = root_bytes(source, root)?;
            let root_ty = TypeKey::parse(&root.type_key).map_err(|error| {
                Error::new(
                    codes::DEBUG_TYPE,
                    format!(
                        "array element root `{}` has an invalid type key: {}",
                        root.key, error.message
                    ),
                )
            })?;
            values.push(decode_type_value(
                source.manifest,
                source.target,
                &root_ty,
                &root.type_key,
                &bytes,
                source.generation,
            )?);
            continue;
        }
        match element.as_ref() {
            TypeKey::Named(got_id) => {
                let Some(record) = type_by_id(source.manifest, *got_id) else {
                    values.push(unavailable_typed(element.render()));
                    continue;
                };
                let child = AggregateTarget {
                    ty: record.clone(),
                    package: aggregate.package.clone(),
                    path: child_path,
                    qualifier: aggregate.qualifier.clone(),
                };
                values.push(decode_aggregate(source, &child)?);
            }
            _ => values.push(unavailable_typed(element.render())),
        }
    }
    if !any {
        return Ok(None);
    }
    Ok(Some(TypedValue {
        type_key: field.type_key.clone(),
        value: DebugValue::Array(values),
    }))
}

fn unavailable_value(type_key: String) -> TypedValue {
    TypedValue {
        type_key,
        value: DebugValue::Unavailable {
            warning: DebugWarning {
                code: codes::W_DEBUG_UNAVAILABLE,
                message: "the manifest does not provide a readable storage leaf for this value"
                    .into(),
            },
        },
    }
}

fn unavailable_typed(type_key: String) -> TypedValue {
    unavailable_value(type_key)
}

fn object_identity(
    source: &DebugSource<'_>,
    aggregate: &AggregateTarget,
) -> Result<(Option<String>, bool)> {
    let Some(root) = find_hidden_root(
        source.manifest,
        &aggregate.package,
        &aggregate.path,
        aggregate.qualifier.as_deref(),
        "HANDLE",
    ) else {
        return Ok((None, false));
    };
    let ty = TypeKey::parse(&root.type_key).map_err(|error| {
        Error::new(
            codes::DEBUG_TYPE,
            format!(
                "hidden handle root `{}` has an invalid type key: {}",
                root.key, error.message
            ),
        )
    })?;
    let bytes = root_bytes(source, root)?;
    match ty {
        TypeKey::PtrTo(inner) => {
            verify_object_target(&inner, &aggregate.ty, &root.key)?;
            let (identity, present) = pointer_identity(
                &bytes,
                source.target.pointer_width,
                source.target.endianness,
                &aggregate.ty.type_key,
                source.generation,
            )?;
            Ok((identity, present))
        }
        TypeKey::RefTo(inner) => {
            verify_object_target(&inner, &aggregate.ty, &root.key)?;
            let (identity, present) = reference_identity(
                &bytes,
                source.target.endianness,
                &aggregate.ty.type_key,
                source.generation,
            )?;
            Ok((identity, present))
        }
        _ => Err(Error::new(
            codes::DEBUG_TYPE,
            format!(
                "hidden handle root `{}` is not a pointer or reference",
                root.key
            ),
        )),
    }
}

fn verify_object_target(inner: &TypeKey, ty: &TypeRecord, root_key: &str) -> Result<()> {
    if let TypeKey::Named(got_id) = inner
        && *got_id != ty.got_id
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!("hidden handle root `{root_key}` points at a different object type"),
        ));
    }
    Ok(())
}

fn decode_type_value(
    manifest: &Manifest,
    target: &Target,
    ty: &TypeKey,
    type_key: &str,
    bytes: &[u8],
    generation: u64,
) -> Result<TypedValue> {
    decode_type_value_depth(manifest, target, ty, type_key, bytes, generation, 0)
}

#[allow(clippy::too_many_arguments)]
fn decode_type_value_depth(
    manifest: &Manifest,
    target: &Target,
    ty: &TypeKey,
    type_key: &str,
    bytes: &[u8],
    generation: u64,
    depth: usize,
) -> Result<TypedValue> {
    if depth > MAX_DECODE_DEPTH {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            "debug value nesting exceeds the decoder limit",
        ));
    }
    let layout = type_layout(manifest, target, ty)?;
    if u64::try_from(bytes.len()).ok() != Some(layout.size) {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!(
                "storage for `{type_key}` is {} bytes but its layout is {} bytes",
                bytes.len(),
                layout.size
            ),
        ));
    }
    match ty {
        TypeKey::Bool => {
            if bytes.len() != 1 {
                return Err(type_shape_error(type_key, bytes.len(), 1));
            }
            Ok(TypedValue {
                type_key: type_key.to_owned(),
                value: DebugValue::Bool(bytes[0] != 0),
            })
        }
        TypeKey::Sint
        | TypeKey::Int
        | TypeKey::Dint
        | TypeKey::Lint
        | TypeKey::Time
        | TypeKey::Ltime
        | TypeKey::Date
        | TypeKey::Ldate
        | TypeKey::Tod
        | TypeKey::Ltod
        | TypeKey::Dt
        | TypeKey::Ldt => {
            let value = read_signed(bytes, target.endianness).ok_or_else(|| {
                Error::new(
                    codes::DEBUG_TYPE,
                    format!("`{type_key}` has an unsupported integer storage width"),
                )
            })?;
            Ok(TypedValue {
                type_key: type_key.to_owned(),
                value: DebugValue::Decimal(value.to_string()),
            })
        }
        TypeKey::Usint
        | TypeKey::Uint
        | TypeKey::Udint
        | TypeKey::Ulint
        | TypeKey::Byte
        | TypeKey::Word
        | TypeKey::Dword
        | TypeKey::Lword
        | TypeKey::Char
        | TypeKey::Wchar
        | TypeKey::Uchar => {
            let value = read_unsigned(bytes, target.endianness).ok_or_else(|| {
                Error::new(
                    codes::DEBUG_TYPE,
                    format!("`{type_key}` has an unsupported integer storage width"),
                )
            })?;
            Ok(TypedValue {
                type_key: type_key.to_owned(),
                value: DebugValue::Decimal(value.to_string()),
            })
        }
        TypeKey::Real => Ok(TypedValue {
            type_key: type_key.to_owned(),
            value: DebugValue::Real(f64::from(f32::from_bits(
                read_unsigned(bytes, target.endianness)
                    .ok_or_else(|| type_shape_error(type_key, bytes.len(), 4))?
                    as u32,
            ))),
        }),
        TypeKey::Lreal => Ok(TypedValue {
            type_key: type_key.to_owned(),
            value: DebugValue::Real(f64::from_bits(
                read_unsigned(bytes, target.endianness)
                    .ok_or_else(|| type_shape_error(type_key, bytes.len(), 8))?,
            )),
        }),
        TypeKey::String { kind, capacity } => Ok(TypedValue {
            type_key: type_key.to_owned(),
            value: DebugValue::String(decode_string(*kind, *capacity, bytes, target.endianness)?),
        }),
        TypeKey::Array {
            lower,
            upper,
            element,
        } => decode_array(
            manifest, target, type_key, *lower, *upper, element, bytes, generation, depth,
        ),
        TypeKey::RefTo(inner) => {
            let (identity, present) =
                reference_identity(bytes, target.endianness, type_key, generation)?;
            Ok(TypedValue {
                type_key: type_key.to_owned(),
                value: DebugValue::Reference {
                    identity,
                    present,
                    target_type: Some(inner.render()),
                },
            })
        }
        TypeKey::PtrTo(inner) => {
            let (identity, present) = pointer_identity(
                bytes,
                target.pointer_width,
                target.endianness,
                type_key,
                generation,
            )?;
            Ok(TypedValue {
                type_key: type_key.to_owned(),
                value: DebugValue::Reference {
                    identity,
                    present,
                    target_type: Some(inner.render()),
                },
            })
        }
        TypeKey::Named(got_id) => {
            let Some(record) = type_by_id(manifest, *got_id) else {
                return Err(Error::new(
                    codes::DEBUG_TYPE,
                    format!(
                        "type `{type_key}` ({}) has no manifest type record",
                        got_id.to_hex()
                    ),
                ));
            };
            decode_named(manifest, target, record, type_key, bytes, generation, depth)
        }
        TypeKey::Void => Err(Error::new(codes::DEBUG_TYPE, "`VOID` has no debug value")),
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_named(
    manifest: &Manifest,
    target: &Target,
    record: &TypeRecord,
    type_key: &str,
    bytes: &[u8],
    generation: u64,
    depth: usize,
) -> Result<TypedValue> {
    match record.type_kind {
        TypeKind::Enum | TypeKind::Subrange => {
            if let Some(value) = read_signed(bytes, target.endianness) {
                return Ok(TypedValue {
                    type_key: type_key.to_owned(),
                    value: DebugValue::Decimal(value.to_string()),
                });
            }
            if let Some(value) = read_unsigned(bytes, target.endianness) {
                return Ok(TypedValue {
                    type_key: type_key.to_owned(),
                    value: DebugValue::Decimal(value.to_string()),
                });
            }
            Err(Error::new(
                codes::DEBUG_TYPE,
                format!("`{type_key}` has an unsupported enum storage width"),
            ))
        }
        TypeKind::Struct | TypeKind::FunctionBlock => {
            if is_overlap(record) {
                return Ok(TypedValue {
                    type_key: type_key.to_owned(),
                    value: DebugValue::Overlap {
                        bytes: bytes.to_vec(),
                        base64: base64_encode(bytes),
                    },
                });
            }
            let fields =
                decode_contiguous_fields(manifest, target, record, bytes, generation, depth)?;
            Ok(TypedValue {
                type_key: type_key.to_owned(),
                value: DebugValue::Aggregate(fields),
            })
        }
        TypeKind::Class => {
            if record.size != u64::from(target.pointer_width) {
                return Err(Error::new(
                    codes::DEBUG_TYPE,
                    format!(
                        "class `{type_key}` storage is {} bytes but pointer width is {}",
                        record.size, target.pointer_width
                    ),
                ));
            }
            let (identity, present) = pointer_identity(
                bytes,
                target.pointer_width,
                target.endianness,
                type_key,
                generation,
            )?;
            Ok(TypedValue {
                type_key: type_key.to_owned(),
                value: DebugValue::Object {
                    identity,
                    present,
                    class: Some(type_key.to_owned()),
                    interface: None,
                    fields: Vec::new(),
                },
            })
        }
        TypeKind::Interface => {
            if bytes.len() < usize::from(target.pointer_width).saturating_mul(2) {
                return Err(type_shape_error(
                    type_key,
                    bytes.len(),
                    usize::from(target.pointer_width) * 2,
                ));
            }
            let present = bytes.iter().any(|byte| *byte != 0);
            let identity = present.then(|| {
                identity_hash(
                    "stc:debug-interface:v1",
                    &[type_key.as_bytes(), &generation.to_le_bytes(), bytes],
                )
            });
            Ok(TypedValue {
                type_key: type_key.to_owned(),
                value: DebugValue::Object {
                    identity,
                    present,
                    class: None,
                    interface: Some(type_key.to_owned()),
                    fields: Vec::new(),
                },
            })
        }
        TypeKind::Alias => {
            let Some(base) = record.base else {
                return Err(Error::new(
                    codes::DEBUG_TYPE,
                    format!("alias `{type_key}` has no base type"),
                ));
            };
            let Some(base_record) = type_by_id(manifest, base) else {
                return Err(Error::new(
                    codes::DEBUG_TYPE,
                    format!(
                        "alias `{type_key}` names base {} with no type record",
                        base.to_hex()
                    ),
                ));
            };
            let base_key = TypeKey::parse(&base_record.type_key).map_err(|error| {
                Error::new(
                    codes::DEBUG_TYPE,
                    format!(
                        "alias base type `{}` is invalid: {}",
                        base_record.type_key, error.message
                    ),
                )
            })?;
            let mut value = decode_type_value_depth(
                manifest,
                target,
                &base_key,
                &base_record.type_key,
                bytes,
                generation,
                depth + 1,
            )?;
            value.type_key = type_key.to_owned();
            Ok(value)
        }
    }
}

fn decode_contiguous_fields(
    manifest: &Manifest,
    target: &Target,
    record: &TypeRecord,
    bytes: &[u8],
    generation: u64,
    depth: usize,
) -> Result<Vec<NamedValue>> {
    let mut out = Vec::with_capacity(record.fields.len());
    for field in &record.fields {
        let start = usize::try_from(field.offset).map_err(|_| {
            Error::new(
                codes::DEBUG_TYPE,
                format!("field `{}` offset is too large", field.path),
            )
        })?;
        let size = usize::try_from(field.size).map_err(|_| {
            Error::new(
                codes::DEBUG_TYPE,
                format!("field `{}` size is too large", field.path),
            )
        })?;
        let end = start.checked_add(size).ok_or_else(|| {
            Error::new(
                codes::DEBUG_TYPE,
                format!("field `{}` range overflows", field.path),
            )
        })?;
        if end > bytes.len() {
            return Err(Error::new(
                codes::DEBUG_TYPE,
                format!(
                    "field `{}` range {}..{} lies outside {} storage bytes",
                    field.path,
                    start,
                    end,
                    bytes.len()
                ),
            ));
        }
        let ty = TypeKey::parse(&field.type_key).map_err(|error| {
            Error::new(
                codes::DEBUG_TYPE,
                format!(
                    "field `{}` type key is invalid: {}",
                    field.path, error.message
                ),
            )
        })?;
        let value = decode_type_value_depth(
            manifest,
            target,
            &ty,
            &field.type_key,
            &bytes[start..end],
            generation,
            depth + 1,
        )?;
        out.push(NamedValue {
            name: field.path.clone(),
            value,
        });
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn decode_array(
    manifest: &Manifest,
    target: &Target,
    type_key: &str,
    lower: ArrayBound,
    upper: ArrayBound,
    element: &TypeKey,
    bytes: &[u8],
    generation: u64,
    depth: usize,
) -> Result<TypedValue> {
    let (ArrayBound::Value(lower), ArrayBound::Value(upper)) = (lower, upper) else {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!("`{type_key}` has a conformant bound and no standalone value"),
        ));
    };
    if upper < lower {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!("`{type_key}` has inverted array bounds"),
        ));
    }
    let count = u128::try_from(upper - lower)
        .ok()
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| Error::new(codes::DEBUG_LIMIT, format!("`{type_key}` is too large")))?;
    if count > 1_000_000 {
        return Err(Error::new(
            codes::DEBUG_LIMIT,
            format!("`{type_key}` has more than one million elements"),
        ));
    }
    let element_layout = type_layout(manifest, target, element)?;
    let stride = round_up(element_layout.size, u64::from(element_layout.align));
    let mut values = Vec::with_capacity(usize::try_from(count).unwrap_or(usize::MAX));
    for index in 0..count {
        let offset = index.checked_mul(u128::from(stride)).ok_or_else(|| {
            Error::new(codes::DEBUG_LIMIT, format!("`{type_key}` offset overflows"))
        })?;
        let start = usize::try_from(offset).map_err(|_| {
            Error::new(
                codes::DEBUG_LIMIT,
                format!("`{type_key}` offset is too large"),
            )
        })?;
        let size = usize::try_from(element_layout.size).map_err(|_| {
            Error::new(
                codes::DEBUG_LIMIT,
                format!("`{type_key}` element is too large"),
            )
        })?;
        let end = start.checked_add(size).ok_or_else(|| {
            Error::new(codes::DEBUG_LIMIT, format!("`{type_key}` range overflows"))
        })?;
        if end > bytes.len() {
            return Err(Error::new(
                codes::DEBUG_TYPE,
                format!(
                    "`{type_key}` element {index} lies outside {} storage bytes",
                    bytes.len()
                ),
            ));
        }
        values.push(decode_type_value_depth(
            manifest,
            target,
            element,
            &element.render(),
            &bytes[start..end],
            generation,
            depth + 1,
        )?);
    }
    Ok(TypedValue {
        type_key: type_key.to_owned(),
        value: DebugValue::Array(values),
    })
}

fn decode_string(
    kind: StringKind,
    capacity: u32,
    bytes: &[u8],
    endianness: stc_got::layout::Endianness,
) -> Result<StringValue> {
    if bytes.len() < 4 {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            "string storage is shorter than its four-byte length",
        ));
    }
    let length = i32::try_from(
        read_signed(&bytes[..4], endianness)
            .ok_or_else(|| Error::new(codes::DEBUG_TYPE, "string length is unreadable"))?,
    )
    .map_err(|_| Error::new(codes::DEBUG_TYPE, "string length does not fit i32"))?;
    if length < 0 {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!("string has negative stored length {length}"),
        ));
    }
    let length = u32::try_from(length)
        .map_err(|_| Error::new(codes::DEBUG_TYPE, "string stored length does not fit u32"))?;
    if length > capacity {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!("string stored length {length} exceeds its capacity {capacity}"),
        ));
    }
    let unit_width = usize::from(kind.unit_bytes());
    let data_len = usize::try_from(length)
        .ok()
        .and_then(|length| length.checked_mul(unit_width))
        .ok_or_else(|| Error::new(codes::DEBUG_TYPE, "string storage length overflows"))?;
    let data_end = 4_usize
        .checked_add(data_len)
        .ok_or_else(|| Error::new(codes::DEBUG_TYPE, "string storage range overflows"))?;
    if data_end > bytes.len() {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!(
                "string length {length} requires {data_end} bytes but storage has {}",
                bytes.len()
            ),
        ));
    }
    let mut units = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
    match kind {
        StringKind::Sbcs => {
            for byte in &bytes[4..data_end] {
                units.push(u32::from(*byte));
            }
        }
        StringKind::Dbcs => {
            let mut index = 4;
            while index < data_end {
                let first = read_u16(&bytes[index..index + 2], endianness);
                index += 2;
                if (0xD800..=0xDBFF).contains(&first) {
                    if index + 2 > data_end {
                        return Err(Error::new(
                            codes::DEBUG_TYPE,
                            "WSTRING ends after a high surrogate",
                        ));
                    }
                    let second = read_u16(&bytes[index..index + 2], endianness);
                    index += 2;
                    if !(0xDC00..=0xDFFF).contains(&second) {
                        return Err(Error::new(
                            codes::DEBUG_TYPE,
                            "WSTRING high surrogate is not followed by a low surrogate",
                        ));
                    }
                    let scalar = 0x1_0000
                        + ((u32::from(first) - 0xD800) << 10)
                        + (u32::from(second) - 0xDC00);
                    units.push(scalar);
                } else if (0xDC00..=0xDFFF).contains(&first) {
                    return Err(Error::new(
                        codes::DEBUG_TYPE,
                        "WSTRING contains an unpaired low surrogate",
                    ));
                } else {
                    units.push(u32::from(first));
                }
            }
        }
        StringKind::Unicode => {
            let mut index = 4;
            while index < data_end {
                let scalar = read_u32(&bytes[index..index + 4], endianness);
                index += 4;
                if scalar > 0x10_FFFF || (0xD800..=0xDFFF).contains(&scalar) {
                    return Err(Error::new(
                        codes::DEBUG_TYPE,
                        format!("USTRING contains an invalid scalar value {scalar:#x}"),
                    ));
                }
                units.push(scalar);
            }
        }
    }
    let text = units
        .iter()
        .map(|unit| char::from_u32(*unit))
        .collect::<Option<String>>();
    Ok(StringValue {
        capacity,
        length: u32::try_from(units.len()).unwrap_or(u32::MAX),
        units,
        text,
    })
}

fn type_layout(
    manifest: &Manifest,
    target: &Target,
    ty: &TypeKey,
) -> Result<stc_got::layout::Layout> {
    if let Some(layout) = elementary_layout(ty) {
        return Ok(layout);
    }
    match ty {
        TypeKey::String { kind, capacity } => {
            let data = u64::from(*capacity)
                .checked_mul(u64::from(kind.unit_bytes()))
                .and_then(|data| data.checked_add(4))
                .ok_or_else(|| {
                    Error::new(codes::DEBUG_LIMIT, "string capacity overflows layout")
                })?;
            Ok(stc_got::layout::Layout {
                size: round_up(data, 4),
                align: 4,
            })
        }
        TypeKey::Array {
            lower: ArrayBound::Value(lower),
            upper: ArrayBound::Value(upper),
            element,
        } => {
            if upper < lower {
                return Err(Error::new(codes::DEBUG_TYPE, "array bounds are inverted"));
            }
            let count = u128::try_from(*upper - *lower)
                .ok()
                .and_then(|count| count.checked_add(1))
                .ok_or_else(|| Error::new(codes::DEBUG_LIMIT, "array count overflows layout"))?;
            let element_layout = type_layout(manifest, target, element)?;
            let stride = round_up(element_layout.size, u64::from(element_layout.align));
            let size = u64::try_from(count)
                .ok()
                .and_then(|count| count.checked_mul(stride))
                .ok_or_else(|| Error::new(codes::DEBUG_LIMIT, "array size overflows layout"))?;
            Ok(stc_got::layout::Layout {
                size,
                align: element_layout.align,
            })
        }
        TypeKey::Array { .. } => Err(Error::new(
            codes::DEBUG_TYPE,
            "conformant arrays have no standalone layout",
        )),
        TypeKey::RefTo(_) => Ok(stc_got::layout::Layout { size: 48, align: 8 }),
        TypeKey::PtrTo(_) => Ok(stc_got::layout::Layout {
            size: u64::from(target.pointer_width),
            align: u32::from(target.pointer_width),
        }),
        TypeKey::Named(got_id) => {
            let record = type_by_id(manifest, *got_id).ok_or_else(|| {
                Error::new(
                    codes::DEBUG_TYPE,
                    format!("type {} has no manifest type record", got_id.to_hex()),
                )
            })?;
            Ok(stc_got::layout::Layout {
                size: record.size,
                align: record.align,
            })
        }
        TypeKey::Void => Ok(stc_got::layout::Layout { size: 0, align: 1 }),
        TypeKey::Bool
        | TypeKey::Sint
        | TypeKey::Int
        | TypeKey::Dint
        | TypeKey::Lint
        | TypeKey::Usint
        | TypeKey::Uint
        | TypeKey::Udint
        | TypeKey::Ulint
        | TypeKey::Real
        | TypeKey::Lreal
        | TypeKey::Time
        | TypeKey::Ltime
        | TypeKey::Date
        | TypeKey::Ldate
        | TypeKey::Tod
        | TypeKey::Ltod
        | TypeKey::Dt
        | TypeKey::Ldt
        | TypeKey::Byte
        | TypeKey::Word
        | TypeKey::Dword
        | TypeKey::Lword
        | TypeKey::Char
        | TypeKey::Wchar
        | TypeKey::Uchar => Err(Error::new(
            codes::DEBUG_TYPE,
            "internal: elementary layout was not found",
        )),
    }
}

fn round_up(value: u64, align: u64) -> u64 {
    if align <= 1 {
        return value;
    }
    value.div_ceil(align).saturating_mul(align)
}

fn read_unsigned(bytes: &[u8], endianness: stc_got::layout::Endianness) -> Option<u64> {
    if bytes.is_empty() || bytes.len() > 8 {
        return None;
    }
    let mut value = 0_u64;
    match endianness {
        stc_got::layout::Endianness::Little => {
            for (index, byte) in bytes.iter().enumerate() {
                value |= u64::from(*byte) << (index * 8);
            }
        }
        stc_got::layout::Endianness::Big => {
            for byte in bytes {
                value = (value << 8) | u64::from(*byte);
            }
        }
    }
    Some(value)
}

fn read_signed(bytes: &[u8], endianness: stc_got::layout::Endianness) -> Option<i64> {
    let value = read_unsigned(bytes, endianness)?;
    let shift = 64 - bytes.len() * 8;
    Some(((value << shift) as i64) >> shift)
}

fn read_u16(bytes: &[u8], endianness: stc_got::layout::Endianness) -> u16 {
    let value = read_unsigned(bytes, endianness).unwrap_or(0);
    u16::try_from(value).unwrap_or(0)
}

fn read_u32(bytes: &[u8], endianness: stc_got::layout::Endianness) -> u32 {
    let value = read_unsigned(bytes, endianness).unwrap_or(0);
    u32::try_from(value).unwrap_or(0)
}

fn type_shape_error(type_key: &str, actual: usize, expected: usize) -> Error {
    Error::new(
        codes::DEBUG_TYPE,
        format!("`{type_key}` storage is {actual} bytes but requires {expected} bytes"),
    )
}

fn reference_identity(
    bytes: &[u8],
    endianness: stc_got::layout::Endianness,
    type_key: &str,
    generation: u64,
) -> Result<(Option<String>, bool)> {
    if bytes.len() != 48 {
        return Err(type_shape_error(type_key, bytes.len(), 48));
    }
    let kind = i32::try_from(
        read_signed(&bytes[..4], endianness)
            .ok_or_else(|| Error::new(codes::DEBUG_TYPE, "reference place kind is unreadable"))?,
    )
    .map_err(|_| Error::new(codes::DEBUG_TYPE, "reference place kind does not fit i32"))?;
    if !(0..=4).contains(&kind) {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!("reference has unknown place kind {kind}"),
        ));
    }
    if kind == 0 {
        return Ok((None, false));
    }
    let identity = identity_hash(
        "stc:debug-reference:v1",
        &[type_key.as_bytes(), &generation.to_le_bytes(), bytes],
    );
    Ok((Some(identity), true))
}

fn pointer_identity(
    bytes: &[u8],
    pointer_width: u8,
    endianness: stc_got::layout::Endianness,
    type_key: &str,
    generation: u64,
) -> Result<(Option<String>, bool)> {
    let expected = usize::from(pointer_width);
    if bytes.len() != expected {
        return Err(type_shape_error(type_key, bytes.len(), expected));
    }
    let value = read_unsigned(bytes, endianness)
        .ok_or_else(|| Error::new(codes::DEBUG_TYPE, "pointer value is unreadable"))?;
    if value == 0 {
        return Ok((None, false));
    }
    let identity = identity_hash(
        "stc:debug-pointer:v1",
        &[type_key.as_bytes(), &generation.to_le_bytes(), bytes],
    );
    Ok((Some(identity), true))
}

fn identity_hash(domain: &str, parts: &[&[u8]]) -> String {
    let digest = domain_hash(domain, parts);
    let mut out = String::with_capacity(18);
    out.push_str("0x");
    for byte in &digest[..8] {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        out.push(char::from(ALPHABET[usize::from(first >> 2)]));
        out.push(char::from(
            ALPHABET[usize::from(((first & 0x03) << 4) | (second >> 4))],
        ));
        if chunk.len() > 1 {
            out.push(char::from(
                ALPHABET[usize::from(((second & 0x0f) << 2) | (third >> 6))],
            ));
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(char::from(ALPHABET[usize::from(third & 0x3f)]));
        } else {
            out.push('=');
        }
    }
    out
}

fn debug_limit(message: impl Into<String>) -> Error {
    Error::new(codes::DEBUG_LIMIT, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stc_got::keys::global_root_key;
    use stc_got::layout::Endianness;
    use stc_got::model::{
        ActionRecord, FieldRecord, Header, History, InitializerKind, Plan, Retention,
    };

    fn target() -> Target {
        Target::new("x86_64-unknown-linux-gnu", Endianness::Little, 8)
    }

    fn field(path: &str, type_key: &str, offset: u64, size: u64) -> FieldRecord {
        FieldRecord {
            got_id: GotId::ZERO,
            path: path.into(),
            type_key: type_key.into(),
            layout_hash: GotId::ZERO,
            offset,
            size,
            align: 1,
            retention: Retention::None,
            source: None,
        }
    }

    fn type_record(
        key: &str,
        type_key: &str,
        kind: TypeKind,
        size: u64,
        fields: Vec<FieldRecord>,
    ) -> TypeRecord {
        TypeRecord {
            got_id: GotId::ZERO,
            key: key.into(),
            previous_keys: Vec::new(),
            type_key: type_key.into(),
            layout_hash: GotId::ZERO,
            size,
            align: 1,
            type_kind: kind,
            base: None,
            interfaces: Vec::new(),
            vtable_shape: None,
            fields,
            source: None,
        }
    }

    fn root(key: &str, type_key: &str, size: u64, field_path: &str) -> RootRecord {
        RootRecord {
            got_id: GotId::ZERO,
            key: key.into(),
            previous_keys: Vec::new(),
            root_kind: RootKind::Global,
            owner_type: None,
            field_path: field_path.into(),
            type_key: type_key.into(),
            layout_hash: GotId::ZERO,
            size,
            align: 1,
            retention: Retention::Retain,
            initializer: InitializerKind::Zero,
            initializer_digest: None,
            source: None,
            slot: 0,
        }
    }

    fn manifest(roots: Vec<RootRecord>, types: Vec<TypeRecord>) -> Manifest {
        Manifest {
            header: Header {
                profile: "test".into(),
                compiler_version: "test".into(),
                target_triple: "x86_64-unknown-linux-gnu".into(),
                module_name: "debug-test".into(),
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
                descriptor_digest: stc_got::Digest32::ZERO,
                source_fingerprint: stc_got::Digest32::ZERO,
                catalog_digest: stc_got::Digest32::ZERO,
                plan_digest: stc_got::Digest32::ZERO,
            },
            roots,
            functions: Vec::new(),
            types,
            descriptors: Vec::new(),
            instances: Vec::new(),
            tasks: Vec::new(),
            exports: Vec::new(),
            slots: Vec::new(),
            plan: Plan {
                verdict: stc_got::PlanVerdict::OnlineCandidate,
                parent_build_id: None,
                parent_manifest_digest: None,
                actions: Vec::new(),
                diagnostics: Vec::new(),
            },
            history: History::default(),
        }
    }

    fn query(text: &str) -> TargetQuery {
        normalize_path_target(text).expect("query")
    }

    #[test]
    fn path_targets_normalize_package_and_instance() {
        let q = query("main.counter#cfg/res/inst");
        assert_eq!(q.package, None);
        assert_eq!(q.path, "MAIN.COUNTER");
        assert_eq!(q.qualifier.as_deref(), Some("CFG/RES/INST"));
        let q = query("MyPkg::main.counter");
        assert_eq!(q.package.as_deref(), Some("MYPKG"));
        assert_eq!(q.path, "MAIN.COUNTER");
    }

    #[test]
    fn raw_addresses_and_malformed_targets_are_unknown() {
        for text in [
            "0x1234",
            "0X1234",
            "MAIN..COUNTER",
            " MAIN",
            "MAIN ",
            "MAIN#",
            "#_DEFAULT",
            "PKG::A::B",
            "MAIN.COU-NTER",
        ] {
            let error = normalize_path_target(text).expect_err(text);
            assert_eq!(error.code, codes::DEBUG_UNKNOWN, "{text}");
        }
    }

    #[test]
    fn got_id_targets_use_strict_hex() {
        let id = GotId::parse("0x00000000000000000000000000000000").expect("parse");
        let normalized = normalize_target(&DebugTarget::GotId(id)).expect("target");
        assert!(matches!(normalized, NormalizedTarget::GotId(value) if value == id));
        assert_eq!(
            GotId::parse("0X00000000000000000000000000000000")
                .expect_err("uppercase")
                .code,
            codes::MANIFEST_INVALID
        );
    }

    #[test]
    fn exact_root_resolution_checks_type_shape() {
        let counter = global_root_key("_ROOT", "COUNTER").expect("key");
        let manifest = manifest(vec![root(&counter, "DINT", 4, "")], Vec::new());
        let request = DebugReadRequest::new(DebugTarget::path("counter"));
        let resolved = resolve_target(&manifest, &request.target).expect("resolve");
        let ResolvedShape::Root(root) = &resolved.shape else {
            panic!("root");
        };
        assert_eq!(root.type_key, "DINT");
        assert!(resolved.atomic);
        check_type_layout(&resolved, &request).expect("matching expectations");
        let request = request.with_type_key("LINT");
        let error = check_type_layout(&resolved, &request).expect_err("type");
        assert_eq!(error.code, codes::DEBUG_STALE);
    }

    #[test]
    fn removed_and_renamed_paths_are_stale() {
        let mut old = root("_ROOT::OLD", "DINT", 4, "");
        old.previous_keys.push("_ROOT::OLDER".into());
        let mut manifest = manifest(vec![old], Vec::new());
        manifest.plan.actions.push(ActionRecord {
            got_id: GotId::ZERO,
            kind: 1,
            key: "_ROOT::NEW".into(),
            old_key: Some("_ROOT::GONE".into()),
            action: PlanAction::Drop,
            reason_code: "test".into(),
            detail: String::new(),
        });
        assert_eq!(
            resolve_path(&manifest, &query("OLDER"))
                .expect_err("renamed")
                .code,
            codes::DEBUG_STALE
        );
        assert_eq!(
            resolve_path(&manifest, &query("GONE"))
                .expect_err("removed")
                .code,
            codes::DEBUG_STALE
        );
    }

    #[test]
    fn ambiguous_instance_paths_are_reported() {
        let manifest = manifest(
            vec![
                root("_ROOT::MAIN.TICKS#CFG/RES/A", "DINT", 4, "TICKS"),
                root("_ROOT::MAIN.TICKS#CFG/RES/B", "DINT", 4, "TICKS"),
            ],
            Vec::new(),
        );
        let request = DebugReadRequest::new(DebugTarget::path("MAIN.TICKS"));
        let error = resolve_target(&manifest, &request.target).expect_err("ambiguous");
        assert_eq!(error.code, codes::DEBUG_UNKNOWN);
        assert!(error.message.contains("ambiguous"));
        let request = DebugReadRequest::new(DebugTarget::path("MAIN.TICKS#CFG/RES/B"));
        assert!(resolve_target(&manifest, &request.target).is_ok());
    }

    #[test]
    fn unqualified_paths_do_not_retarget_another_instance() {
        let mut manifest = manifest(
            vec![root("_ROOT::MAIN.TICKS#CFG/RES/B", "DINT", 4, "TICKS")],
            Vec::new(),
        );
        manifest.plan.actions.push(ActionRecord {
            got_id: GotId::ZERO,
            kind: 1,
            key: "_ROOT::MAIN.TICKS#CFG/RES/A".into(),
            old_key: Some("_ROOT::MAIN.TICKS#CFG/RES/A".into()),
            action: PlanAction::Drop,
            reason_code: "test".into(),
            detail: String::new(),
        });
        let error = resolve_path(&manifest, &query("MAIN.TICKS")).expect_err("retarget");
        assert_eq!(error.code, codes::DEBUG_STALE);
        assert!(error.message.contains("different instance"));
        assert!(
            resolve_path(&manifest, &query("MAIN.TICKS#CFG/RES/B")).is_ok(),
            "the explicit instance still resolves"
        );
        assert_eq!(
            resolve_path(&manifest, &query("MAIN.TICKS#CFG/RES/A"))
                .expect_err("removed instance")
                .code,
            codes::DEBUG_STALE
        );
    }

    #[test]
    fn every_scalar_class_decodes_without_implicit_conversion() {
        let manifest = manifest(Vec::new(), Vec::new());
        let target = target();
        let cases: &[(&TypeKey, &[u8], DebugValue)] = &[
            (&TypeKey::Bool, &[1], DebugValue::Bool(true)),
            (&TypeKey::Sint, &[0xff], DebugValue::Decimal("-1".into())),
            (
                &TypeKey::Uint,
                &[0xff, 0xff],
                DebugValue::Decimal("65535".into()),
            ),
            (&TypeKey::Lint, &[0xff; 8], DebugValue::Decimal("-1".into())),
            (
                &TypeKey::Ulint,
                &[0xff; 8],
                DebugValue::Decimal("18446744073709551615".into()),
            ),
            (&TypeKey::Date, &[0xff; 8], DebugValue::Decimal("-1".into())),
            (&TypeKey::Char, &[65], DebugValue::Decimal("65".into())),
        ];
        for (ty, bytes, expected) in cases {
            let value =
                decode_type_value(&manifest, &target, ty, &ty.render(), bytes, 1).expect("decode");
            assert_eq!(&value.value, expected, "{}", ty.render());
        }
        let real = decode_type_value(
            &manifest,
            &target,
            &TypeKey::Real,
            "REAL",
            &1.5_f32.to_le_bytes(),
            1,
        )
        .expect("real");
        assert_eq!(real.value, DebugValue::Real(1.5));
        let nan = decode_type_value(
            &manifest,
            &target,
            &TypeKey::Lreal,
            "LREAL",
            &f64::NAN.to_le_bytes(),
            1,
        )
        .expect("nan");
        assert!(matches!(nan.value, DebugValue::Real(value) if value.is_nan()));
    }

    #[test]
    fn strings_validate_capacity_and_utf8_units() {
        let manifest = manifest(Vec::new(), Vec::new());
        let target = target();
        let mut sbcs = vec![2, 0, 0, 0, b'h', b'i', 0, 0];
        let value = decode_type_value(
            &manifest,
            &target,
            &TypeKey::String {
                kind: StringKind::Sbcs,
                capacity: 4,
            },
            "STRING[4]",
            &sbcs,
            1,
        )
        .expect("string");
        let DebugValue::String(string) = value.value else {
            panic!("string");
        };
        assert_eq!(string.text.as_deref(), Some("hi"));
        assert_eq!(string.length, 2);
        sbcs[0] = 5;
        let error = decode_type_value(
            &manifest,
            &target,
            &TypeKey::String {
                kind: StringKind::Sbcs,
                capacity: 4,
            },
            "STRING[4]",
            &sbcs,
            1,
        )
        .expect_err("capacity");
        assert_eq!(error.code, codes::DEBUG_TYPE);

        let mut wstring = vec![0_u8; 12];
        wstring[..4].copy_from_slice(&2_i32.to_le_bytes());
        wstring[4..6].copy_from_slice(&0xD83D_u16.to_le_bytes());
        wstring[6..8].copy_from_slice(&0xDE00_u16.to_le_bytes());
        let value = decode_type_value(
            &manifest,
            &target,
            &TypeKey::String {
                kind: StringKind::Dbcs,
                capacity: 3,
            },
            "WSTRING[3]",
            &wstring,
            1,
        )
        .expect("wstring");
        let DebugValue::String(string) = value.value else {
            panic!("string");
        };
        assert_eq!(string.units, vec![0x1F600]);
        assert_eq!(string.text.as_deref(), Some("😀"));
    }

    #[test]
    fn arrays_decode_in_declared_index_order() {
        let manifest = manifest(Vec::new(), Vec::new());
        let target = target();
        let ty = TypeKey::Array {
            lower: ArrayBound::Value(1),
            upper: ArrayBound::Value(3),
            element: Box::new(TypeKey::Dint),
        };
        let mut bytes = Vec::new();
        for value in [7_i32, 8, 9] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let value =
            decode_type_value(&manifest, &target, &ty, &ty.render(), &bytes, 1).expect("array");
        let DebugValue::Array(elements) = value.value else {
            panic!("array");
        };
        assert_eq!(
            elements
                .iter()
                .map(|element| match &element.value {
                    DebugValue::Decimal(text) => text.as_str(),
                    _ => panic!("scalar"),
                })
                .collect::<Vec<_>>(),
            vec!["7", "8", "9"]
        );
    }

    #[test]
    fn overlap_is_opaque_base64_bytes() {
        let fields = vec![field("A", "DINT", 0, 4), field("B", "REAL", 0, 4)];
        let record = type_record(
            "_ROOT::U",
            "T:00000000000000000000000000000000",
            TypeKind::Struct,
            4,
            fields,
        );
        let manifest = manifest(Vec::new(), vec![record]);
        let target = target();
        let ty = TypeKey::Named(manifest.types[0].got_id);
        let value = decode_type_value(&manifest, &target, &ty, &ty.render(), &[1, 2, 3, 4], 1)
            .expect("overlap");
        assert_eq!(
            value.value,
            DebugValue::Overlap {
                bytes: vec![1, 2, 3, 4],
                base64: "AQIDBA==".into(),
            }
        );
    }

    #[test]
    fn references_have_identity_without_addresses() {
        let manifest = manifest(Vec::new(), Vec::new());
        let target = target();
        let mut descriptor = vec![0_u8; 48];
        descriptor[..4].copy_from_slice(&1_i32.to_le_bytes());
        descriptor[8..16].copy_from_slice(&0x1234_i64.to_le_bytes());
        let first = decode_type_value(
            &manifest,
            &target,
            &TypeKey::RefTo(Box::new(TypeKey::Dint)),
            "REF_TO(DINT)",
            &descriptor,
            7,
        )
        .expect("reference");
        let DebugValue::Reference {
            identity,
            present,
            target_type,
        } = &first.value
        else {
            panic!("reference");
        };
        assert!(present);
        assert_eq!(target_type.as_deref(), Some("DINT"));
        let identity = identity.clone().expect("identity");
        assert!(identity.starts_with("0x"));
        assert!(!identity.contains("1234"));
        let second = decode_type_value(
            &manifest,
            &target,
            &TypeKey::RefTo(Box::new(TypeKey::Dint)),
            "REF_TO(DINT)",
            &descriptor,
            8,
        )
        .expect("reference");
        assert_ne!(first.value, second.value);
    }

    #[test]
    fn split_aggregate_type_and_removed_field_follow_manifest() {
        let point = type_record(
            "_ROOT::POINT",
            "T:00000000000000000000000000000001",
            TypeKind::Struct,
            8,
            vec![field("X", "DINT", 0, 4), field("Y", "DINT", 4, 4)],
        );
        let mut manifest = manifest(
            vec![
                {
                    let mut root = root("_ROOT::P.X", "DINT", 4, "X");
                    root.owner_type = Some(point.got_id);
                    root
                },
                {
                    let mut root = root("_ROOT::P.SUB", "DINT", 4, "SUB");
                    root.owner_type = Some(point.got_id);
                    root
                },
            ],
            vec![point],
        );
        manifest.types[0].fields.retain(|field| field.path == "X");
        let aggregate = build_aggregate(
            &manifest,
            &aggregate_groups(&manifest, &query("P"))[0],
            &query("P"),
        )
        .expect("aggregate");
        assert_eq!(aggregate.ty.type_kind, TypeKind::Struct);
        let resolved = resolved_aggregate(aggregate).expect("resolved");
        assert!(!resolved.atomic);
        let request = DebugReadRequest::new(DebugTarget::path("P"));
        check_type_layout(&resolved, &request).expect("aggregate expectations");
    }

    #[test]
    fn watch_specs_enforce_minimum_interval_and_reject_locals() {
        let error = validate_watch_spec(&WatchSpec {
            target: DebugTarget::path("P.X"),
            interval_ns: MIN_WATCH_INTERVAL_NS - 1,
        })
        .expect_err("interval");
        assert_eq!(error.code, codes::DEBUG_LIMIT);
        let error = validate_watch_spec(&WatchSpec {
            target: DebugTarget::local("F1", "X"),
            interval_ns: MIN_WATCH_INTERVAL_NS,
        })
        .expect_err("local");
        assert_eq!(error.code, codes::DEBUG_STOP);
    }

    #[test]
    fn event_queue_drops_oldest_batches_and_marks_the_next() {
        let mut state = BusState::default();
        for cycle in 0..(MAX_WATCH_BATCHES as u64 + 2) {
            push_event(
                &mut state,
                DebugEvent::Batch(DebugBatch {
                    generation: 1,
                    build_id: GotId::ZERO,
                    cycle,
                    values: Vec::new(),
                    warnings: Vec::new(),
                }),
            );
        }
        let batches: Vec<&DebugBatch> = state
            .events
            .iter()
            .filter_map(|event| match event {
                DebugEvent::Batch(batch) => Some(batch),
                _ => None,
            })
            .collect();
        assert_eq!(batches.len(), MAX_WATCH_BATCHES);
        assert_eq!(batches.first().expect("first").cycle, 2);
        assert_eq!(batches.last().expect("last").cycle, 65);
        assert!(
            batches
                .last()
                .expect("last")
                .warnings
                .iter()
                .any(|warning| warning.code == codes::W_DEBUG_WATCH_DROPPED)
        );
    }

    #[test]
    fn state_events_are_not_dropped_for_watch_batches() {
        let mut state = BusState::default();
        for request in 0..4 {
            push_event(
                &mut state,
                DebugEvent::Read(ReadCompletion {
                    request: ReadRequestId(request),
                    result: Err(DebugFailure::new(codes::DEBUG_STALE, "test")),
                }),
            );
        }
        for cycle in 0..(MAX_WATCH_BATCHES as u64 + 2) {
            push_event(
                &mut state,
                DebugEvent::Batch(DebugBatch {
                    generation: 1,
                    build_id: GotId::ZERO,
                    cycle,
                    values: Vec::new(),
                    warnings: Vec::new(),
                }),
            );
        }
        assert_eq!(
            state
                .events
                .iter()
                .filter(|event| matches!(event, DebugEvent::Read(_)))
                .count(),
            4
        );
    }

    #[test]
    fn service_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<DebugService>();
    }
}
