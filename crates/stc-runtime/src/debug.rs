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
use stc_got::json::Json;
use stc_got::keys::{normalize_ident, normalize_package};
use stc_got::layout::{ArrayBound, StringKind, Target, TypeKey, elementary_layout};
use stc_got::model::{
    AggregateFieldRecord, AggregateKind, AggregateRecord, PlanAction, Retention, RootKind,
    RootRecord, TypeKind, TypeMetadataRecord, TypeRecord,
};
use stc_got::sha256::domain_hash;

use crate::debug_view::{
    IdentityKind, SplitView, ViewField, ViewIdentity, ViewKind, ViewNode, ViewOrigin, ViewRoot,
};

/// Maximum number of active watch targets.
pub const MAX_WATCHES: usize = 256;
/// Minimum watch interval in nanoseconds.
pub const MIN_WATCH_INTERVAL_NS: i64 = 50_000_000;
/// Maximum number of queued watch batches.
pub const MAX_WATCH_BATCHES: usize = 64;
/// Maximum approximate byte size of the event queue.
pub const MAX_EVENT_BYTES: usize = 1024 * 1024;
/// Maximum queued writes and forces, including active forces.
pub const MAX_DEBUG_QUEUE_ENTRIES: usize = 1024;
/// Maximum approximate byte size of queued writes and active forces.
pub const MAX_DEBUG_QUEUE_BYTES: usize = 1024 * 1024;
/// Maximum audit value size before truncation.
pub const MAX_AUDIT_VALUE_BYTES: usize = 4096;
/// Maximum completed queue results retained for the embedding host.
pub const MAX_DEBUG_QUEUE_RESULTS: usize = 1024;

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
    /// Whether to build the manifest-assisted split-object view.
    pub include_view: bool,
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
            include_view: false,
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

    /// Requests the manifest-assisted split-object view.
    #[must_use]
    pub fn with_view(mut self) -> Self {
        self.include_view = true;
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
    /// Whether the target reads protected data (`RETAIN`/`PERSISTENT` root or
    /// an `AT` direct variable) requiring an audited `observe` read.
    pub protected: bool,
    /// Canonical type key.
    pub type_key: String,
    /// Typed value.
    pub value: DebugValue,
    /// Manifest-assisted split-object view, when requested.
    pub view: Option<Box<SplitView>>,
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

/// Whether a queued debug operation is a one-shot write or a persistent force.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DebugWriteKind {
    /// Applied once at the next boundary.
    Write,
    /// Applied at every boundary until released.
    Force,
}

impl DebugWriteKind {
    /// Returns the protocol spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Write => "write",
            Self::Force => "force",
        }
    }
}

/// One typed write or force request.
#[derive(Clone, Debug, PartialEq)]
pub struct DebugWriteRequest {
    /// Target root to write.
    pub target: DebugTarget,
    /// Typed value to encode.
    pub value: TypedValue,
    /// Expected active generation, when known.
    pub expected_generation: Option<u64>,
    /// Expected active build, when known.
    pub expected_build_id: Option<GotId>,
    /// Expected root type key, when known.
    pub expected_type_key: Option<String>,
    /// Expected root layout hash, when known.
    pub expected_layout_hash: Option<GotId>,
}

impl DebugWriteRequest {
    /// Creates a request with no expectations.
    #[must_use]
    pub fn new(target: DebugTarget, value: TypedValue) -> Self {
        Self {
            target,
            value,
            expected_generation: None,
            expected_build_id: None,
            expected_type_key: None,
            expected_layout_hash: None,
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

    /// Attaches an expected root type key.
    #[must_use]
    pub fn with_type_key(mut self, type_key: impl Into<String>) -> Self {
        self.expected_type_key = Some(type_key.into());
        self
    }

    /// Attaches an expected root layout hash.
    #[must_use]
    pub fn with_layout_hash(mut self, layout_hash: GotId) -> Self {
        self.expected_layout_hash = Some(layout_hash);
        self
    }
}

/// Server-assigned identity for one queued write or force.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DebugQueueId(pub u64);

/// Final outcome of one queued write or force.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugQueueOutcome {
    /// The operation modified storage at its boundary.
    Applied,
    /// The operation was revalidated and rejected without retargeting.
    Rejected,
}

impl DebugQueueOutcome {
    /// Returns the protocol spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Rejected => "rejected",
        }
    }
}

/// One completed write/force queue result.
#[derive(Clone, Debug, PartialEq)]
pub struct DebugQueueResult {
    /// Queue identity assigned at enqueue.
    pub queue_id: DebugQueueId,
    /// Operation kind.
    pub operation: DebugWriteKind,
    /// Applied or rejected.
    pub outcome: DebugQueueOutcome,
    /// Generation observed at the boundary.
    pub generation: Option<u64>,
    /// Build observed at the boundary.
    pub build_id: Option<GotId>,
    /// Completed-cycle count at the boundary.
    pub cycle: u64,
    /// Rejection code, when rejected.
    pub code: Option<&'static str>,
    /// Rejection message, when rejected.
    pub message: Option<String>,
    /// Non-fatal warning, such as a force on a `RETAIN` root.
    pub warning: Option<DebugWarning>,
}

/// Audit record outcome (Debug and Monitoring §13.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditOutcome {
    /// The operation was accepted or completed.
    Ok,
    /// Host policy denied the operation.
    Denied,
    /// The operation failed before it could be applied.
    Error,
}

impl AuditOutcome {
    /// Returns the protocol spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Denied => "denied",
            Self::Error => "error",
        }
    }
}

/// Audit target description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditTarget {
    /// Stable package key path or local path, never an address.
    pub key: String,
    /// `root`, `local`, or `path`.
    pub kind: String,
}

/// One format-1 debug audit event.
///
/// Phase 4 owns the data model and fail-closed sink; phase 5 owns the JSONL
/// file, session identity, wall-clock timestamps, and append lifecycle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEvent {
    /// Audit format, always `1`.
    pub format: u8,
    /// Per-log sequence starting at 1.
    pub seq: u64,
    /// Host monotonic clock; phase 4 records `0` until phase 5 wires the clock.
    pub monotonic_ns: u64,
    /// Host local date; `0` when unavailable.
    pub local_date_ns: i64,
    /// Session identity, null before phase 5.
    pub session: Option<String>,
    /// Event name from the closed §13.3 set.
    pub event: &'static str,
    /// Outcome.
    pub outcome: AuditOutcome,
    /// Granted scope, when applicable.
    pub scope: Option<String>,
    /// Target, when applicable.
    pub target: Option<AuditTarget>,
    /// Typed value payload, when applicable.
    pub value: Option<Json>,
    /// Generation, when applicable.
    pub generation: Option<u64>,
    /// Build identity, when applicable.
    pub build: Option<GotId>,
    /// Completed-cycle count, when applicable.
    pub cycle: Option<u64>,
    /// Event-specific object.
    pub detail: Json,
}

impl AuditEvent {
    /// Creates an event with an empty detail object.
    #[must_use]
    pub fn new(event: &'static str, outcome: AuditOutcome) -> Self {
        Self {
            format: 1,
            seq: 0,
            monotonic_ns: 0,
            local_date_ns: 0,
            session: None,
            event,
            outcome,
            scope: None,
            target: None,
            value: None,
            generation: None,
            build: None,
            cycle: None,
            detail: Json::Object(std::collections::BTreeMap::new()),
        }
    }

    /// Returns the format-1 JSON object.
    ///
    /// Values larger than [`MAX_AUDIT_VALUE_BYTES`] are replaced by
    /// `{"truncated":true,"size":N}` as §13.3 requires.
    #[must_use]
    pub fn to_json(&self) -> Json {
        let value = self.value.as_ref().map(|value| {
            let size = value.render_compact().len();
            if size > MAX_AUDIT_VALUE_BYTES {
                Json::object([
                    ("truncated".into(), Json::Bool(true)),
                    (
                        "size".into(),
                        Json::unsigned(u64::try_from(size).unwrap_or(u64::MAX)),
                    ),
                ])
            } else {
                value.clone()
            }
        });
        let target = self.target.as_ref().map(|target| {
            Json::object([
                ("key".into(), Json::string(target.key.clone())),
                ("kind".into(), Json::string(target.kind.clone())),
            ])
        });
        Json::object([
            (
                "build_id".into(),
                self.build
                    .map_or(Json::Null, |build| Json::string(build.to_hex())),
            ),
            (
                "cycle".into(),
                self.cycle.map_or(Json::Null, Json::unsigned),
            ),
            ("detail".into(), self.detail.clone()),
            ("event".into(), Json::string(self.event)),
            ("format".into(), Json::unsigned(u64::from(self.format))),
            (
                "generation".into(),
                self.generation.map_or(Json::Null, Json::unsigned),
            ),
            ("local_date_ns".into(), Json::signed(self.local_date_ns)),
            ("monotonic_ns".into(), Json::unsigned(self.monotonic_ns)),
            ("outcome".into(), Json::string(self.outcome.name())),
            (
                "scope".into(),
                self.scope
                    .as_ref()
                    .map_or(Json::Null, |scope| Json::string(scope.clone())),
            ),
            ("seq".into(), Json::unsigned(self.seq)),
            (
                "session".into(),
                self.session
                    .as_ref()
                    .map_or(Json::Null, |session| Json::string(session.clone())),
            ),
            ("target".into(), target.unwrap_or(Json::Null)),
            ("value".into(), value.unwrap_or(Json::Null)),
        ])
    }

    /// Returns the compact one-line JSON record.
    #[must_use]
    pub fn to_json_line(&self) -> String {
        self.to_json().render_compact()
    }
}

/// Fail-closed audit destination.
///
/// Phase 5 supplies a JSONL file implementation. A sink error refuses the
/// audited operation with [`codes::DEBUG_AUDIT`].
pub trait AuditSink: Send {
    /// Appends one event. Returning an error refuses the audited operation.
    fn record(&mut self, event: &AuditEvent) -> Result<()>;
}

/// Why debug state ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugEndReason {
    /// An online change was activated.
    Swap,
    /// The change was rolled back.
    Rollback,
    /// A cold install replaced the active generation.
    ColdInstall,
    /// The host shut down.
    Shutdown,
    /// The target trapped or exited while the host remained alive.
    TargetExit,
    /// A stopped cycle exceeded the stop budget.
    PauseTimeout,
    /// A session ended or disconnected.
    SessionEnd,
}

impl DebugEndReason {
    /// Audit event name for the transition.
    #[must_use]
    pub const fn audit_event(self) -> &'static str {
        match self {
            Self::Swap | Self::Rollback | Self::ColdInstall => "swap-end",
            Self::Shutdown | Self::TargetExit | Self::SessionEnd => "session-end",
            Self::PauseTimeout => "timeout",
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
    /// One completed write/force queue result for an active session.
    Queue(DebugQueueResult),
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
    session: Option<String>,
    snapshot: Option<DebugSnapshot>,
    end_notice: Option<DebugEndNotice>,
}

/// A small snapshot of engine-owned debug state, published by the cycle
/// thread for the service thread.
#[derive(Clone, Debug)]
pub(crate) struct DebugSnapshot {
    pub(crate) generation: u64,
    pub(crate) build_id: GotId,
    pub(crate) cycle: u64,
    pub(crate) mode: String,
    pub(crate) hooks: bool,
    pub(crate) debug_info: String,
    pub(crate) checkpoints: bool,
    pub(crate) stepping: bool,
    pub(crate) frames: bool,
    pub(crate) frames_error: Option<String>,
    pub(crate) metadata_error: Option<String>,
    pub(crate) queued_writes: usize,
    pub(crate) active_forces: usize,
    pub(crate) watches: usize,
    pub(crate) breakpoints: usize,
    pub(crate) tables: Option<std::sync::Arc<crate::stop::DebugTables>>,
}

/// A teardown signal generated on the cycle thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DebugEndNotice {
    pub(crate) reason: &'static str,
    pub(crate) code: Option<&'static str>,
    pub(crate) generation: u64,
    pub(crate) build_id: GotId,
    pub(crate) cycle: u64,
}

/// A synchronous reply slot completed by the cycle thread.
pub(crate) struct Reply<T> {
    value: Mutex<Option<T>>,
    ready: std::sync::Condvar,
}

impl<T> Reply<T> {
    fn new() -> Self {
        Self {
            value: Mutex::new(None),
            ready: std::sync::Condvar::new(),
        }
    }

    fn set(&self, value: T) {
        let mut slot = self
            .value
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = Some(value);
        self.ready.notify_all();
    }

    /// Waits for the cycle thread to complete the reply.
    #[must_use]
    pub(crate) fn wait(&self, deadline: std::time::Instant) -> Option<T> {
        let mut slot = self
            .value
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if let Some(value) = slot.take() {
                return Some(value);
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return None;
            }
            let (next, _) = self
                .ready
                .wait_timeout(slot, deadline.saturating_duration_since(now))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            slot = next;
        }
    }
}

/// One registered watch target list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WatchInfo {
    pub(crate) watch_id: WatchId,
    pub(crate) targets: Vec<DebugTarget>,
    pub(crate) resolved: Vec<String>,
    pub(crate) interval_ns: i64,
    /// Whether any resolved target reads `RETAIN`/`PERSISTENT` or `AT` data.
    pub(crate) protected: bool,
}

/// The manifest projection returned by `manifest.get`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ManifestReply {
    pub(crate) format: &'static str,
    pub(crate) digest: String,
    pub(crate) data: ManifestData,
}

/// JSON or binary manifest data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ManifestData {
    Json(String),
    Binary(Vec<u8>),
}

enum DebugCommand {
    Read {
        request: ReadRequestId,
        target: DebugReadRequest,
    },
    ReadWait {
        target: DebugReadRequest,
        reply: Arc<Reply<std::result::Result<DebugRead, DebugFailure>>>,
    },
    WriteWait {
        kind: DebugWriteKind,
        target: DebugWriteRequest,
        reply: Arc<Reply<std::result::Result<DebugQueueId, DebugFailure>>>,
    },
    ReleaseWait {
        target: DebugTarget,
        reply: Arc<Reply<std::result::Result<bool, DebugFailure>>>,
    },
    ReleaseAllWait {
        reply: Arc<Reply<std::result::Result<usize, DebugFailure>>>,
    },
    AddWatch {
        watch: WatchId,
        spec: WatchSpec,
    },
    AddWatchWait {
        targets: Vec<DebugTarget>,
        interval_ns: i64,
        reply: Arc<Reply<std::result::Result<WatchInfo, DebugFailure>>>,
    },
    RemoveWatch {
        watch: WatchId,
    },
    RemoveWatchWait {
        watch: WatchId,
        reply: Arc<Reply<Option<WatchInfo>>>,
    },
    ListWatchesWait {
        reply: Arc<Reply<Vec<WatchInfo>>>,
    },
    ClearSessionWait {
        reply: Arc<Reply<std::result::Result<(), DebugFailure>>>,
    },
    ManifestWait {
        binary: bool,
        reply: Arc<Reply<std::result::Result<ManifestReply, DebugFailure>>>,
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

    pub(crate) fn submit_read_wait(
        &self,
        request: DebugReadRequest,
    ) -> Result<Arc<Reply<std::result::Result<DebugRead, DebugFailure>>>> {
        validate_request(&request)?;
        let reply = Arc::new(Reply::new());
        self.push_command(DebugCommand::ReadWait {
            target: request,
            reply: Arc::clone(&reply),
        })?;
        Ok(reply)
    }

    pub(crate) fn submit_write_wait(
        &self,
        kind: DebugWriteKind,
        request: DebugWriteRequest,
    ) -> Result<Arc<Reply<std::result::Result<DebugQueueId, DebugFailure>>>> {
        let reply = Arc::new(Reply::new());
        self.push_command(DebugCommand::WriteWait {
            kind,
            target: request,
            reply: Arc::clone(&reply),
        })?;
        Ok(reply)
    }

    pub(crate) fn submit_release_wait(
        &self,
        target: DebugTarget,
    ) -> Result<Arc<Reply<std::result::Result<bool, DebugFailure>>>> {
        validate_request(&DebugReadRequest::new(target.clone()))?;
        let reply = Arc::new(Reply::new());
        self.push_command(DebugCommand::ReleaseWait {
            target,
            reply: Arc::clone(&reply),
        })?;
        Ok(reply)
    }

    pub(crate) fn submit_release_all_wait(
        &self,
    ) -> Result<Arc<Reply<std::result::Result<usize, DebugFailure>>>> {
        let reply = Arc::new(Reply::new());
        self.push_command(DebugCommand::ReleaseAllWait {
            reply: Arc::clone(&reply),
        })?;
        Ok(reply)
    }

    pub(crate) fn submit_watch_add_wait(
        &self,
        targets: Vec<DebugTarget>,
        interval_ns: i64,
    ) -> Result<Arc<Reply<std::result::Result<WatchInfo, DebugFailure>>>> {
        if targets.is_empty() || targets.len() > MAX_WATCHES {
            return Err(debug_limit(format!(
                "a watch needs between 1 and {MAX_WATCHES} targets"
            )));
        }
        for target in &targets {
            validate_watch_target(target)?;
        }
        if interval_ns < MIN_WATCH_INTERVAL_NS {
            return Err(debug_limit(format!(
                "the watch interval must be at least {} ms",
                MIN_WATCH_INTERVAL_NS / 1_000_000
            )));
        }
        let reply = Arc::new(Reply::new());
        self.push_command(DebugCommand::AddWatchWait {
            targets,
            interval_ns,
            reply: Arc::clone(&reply),
        })?;
        Ok(reply)
    }

    pub(crate) fn submit_watch_remove_wait(
        &self,
        watch: WatchId,
    ) -> Result<Arc<Reply<Option<WatchInfo>>>> {
        let reply = Arc::new(Reply::new());
        self.push_command(DebugCommand::RemoveWatchWait {
            watch,
            reply: Arc::clone(&reply),
        })?;
        Ok(reply)
    }

    pub(crate) fn submit_watch_list_wait(&self) -> Result<Arc<Reply<Vec<WatchInfo>>>> {
        let reply = Arc::new(Reply::new());
        self.push_command(DebugCommand::ListWatchesWait {
            reply: Arc::clone(&reply),
        })?;
        Ok(reply)
    }

    pub(crate) fn submit_clear_session_wait(
        &self,
    ) -> Result<Arc<Reply<std::result::Result<(), DebugFailure>>>> {
        let reply = Arc::new(Reply::new());
        self.push_command(DebugCommand::ClearSessionWait {
            reply: Arc::clone(&reply),
        })?;
        Ok(reply)
    }

    pub(crate) fn submit_manifest_wait(
        &self,
        binary: bool,
    ) -> Result<Arc<Reply<std::result::Result<ManifestReply, DebugFailure>>>> {
        let reply = Arc::new(Reply::new());
        self.push_command(DebugCommand::ManifestWait {
            binary,
            reply: Arc::clone(&reply),
        })?;
        Ok(reply)
    }

    fn push_command(&self, command: DebugCommand) -> Result<()> {
        let mut state = lock_bus(&self.bus);
        if state.commands.len() >= MAX_DEBUG_COMMANDS {
            return Err(debug_limit("the debug command queue is full"));
        }
        state.commands.push_back(command);
        Ok(())
    }

    /// Binds the authenticated session identity used by engine audit records.
    pub(crate) fn set_session(&self, session: Option<String>) {
        lock_bus(&self.bus).session = session;
    }

    /// Returns the current engine session identity.
    #[must_use]
    pub(crate) fn session(&self) -> Option<String> {
        lock_bus(&self.bus).session.clone()
    }

    /// Returns the latest published engine snapshot.
    #[must_use]
    pub(crate) fn snapshot(&self) -> Option<DebugSnapshot> {
        lock_bus(&self.bus).snapshot.clone()
    }

    /// Takes the latest teardown notice generated on the cycle thread.
    #[must_use]
    pub(crate) fn take_end_notice(&self) -> Option<DebugEndNotice> {
        lock_bus(&self.bus).end_notice.take()
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
    targets: Vec<DebugTarget>,
    resolved: Vec<String>,
    interval_ns: i64,
    next_due_ns: i64,
    protected: bool,
}

impl ActiveWatch {
    fn points(&self) -> usize {
        self.targets.len()
    }

    fn info(&self, watch_id: WatchId) -> WatchInfo {
        WatchInfo {
            watch_id,
            targets: self.targets.clone(),
            resolved: self.resolved.clone(),
            interval_ns: self.interval_ns,
            protected: self.protected,
        }
    }
}

/// One write or force validated and encoded at enqueue.
#[derive(Clone, Debug)]
pub(crate) struct PreparedWrite {
    kind: DebugWriteKind,
    got_id: GotId,
    generation: u64,
    build_id: GotId,
    cycle: u64,
    target_key: String,
    type_key: String,
    layout_hash: GotId,
    expected_generation: Option<u64>,
    expected_build_id: Option<GotId>,
    expected_type_key: Option<String>,
    expected_layout_hash: Option<GotId>,
    bytes: Vec<u8>,
    value: TypedValue,
    warning: Option<DebugWarning>,
    queue_bytes: usize,
}

#[derive(Clone, Debug)]
struct QueuedWrite {
    queue_id: DebugQueueId,
    prepared: PreparedWrite,
}

/// Boundary-serviced debug state owned by [`crate::Host`].
pub(crate) struct DebugEngine {
    bus: Arc<DebugBus>,
    watches: BTreeMap<WatchId, ActiveWatch>,
    last_generation: Option<u64>,
    audit: Option<Box<dyn AuditSink>>,
    audit_seq: u64,
    next_queue: u64,
    queue: VecDeque<QueuedWrite>,
    forces: BTreeMap<GotId, PreparedWrite>,
    results: VecDeque<DebugQueueResult>,
}

impl DebugEngine {
    pub(crate) fn new() -> Self {
        Self {
            bus: Arc::new(DebugBus {
                state: Mutex::new(BusState::default()),
            }),
            watches: BTreeMap::new(),
            last_generation: None,
            audit: None,
            audit_seq: 0,
            next_queue: 0,
            queue: VecDeque::new(),
            forces: BTreeMap::new(),
            results: VecDeque::new(),
        }
    }

    pub(crate) fn service(&self) -> DebugService {
        DebugService {
            bus: Arc::clone(&self.bus),
        }
    }

    pub(crate) fn watch_count(&self) -> usize {
        self.watches.values().map(ActiveWatch::points).sum()
    }

    /// Returns whether an authenticated debug session is still bound.
    pub(crate) fn has_session(&self) -> bool {
        lock_bus(&self.bus).session.is_some()
    }

    pub(crate) fn watch_list(&self) -> Vec<WatchInfo> {
        self.watches
            .iter()
            .map(|(id, watch)| watch.info(*id))
            .collect()
    }

    pub(crate) fn add_watch(&mut self, spec: WatchSpec) -> Result<WatchId> {
        validate_watch_spec(&spec)?;
        if self.watch_count() >= MAX_WATCHES {
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
                targets: vec![spec.target.clone()],
                resolved: vec![display_target(&spec.target)],
                interval_ns: spec.interval_ns,
                next_due_ns: i64::MIN,
                protected: false,
            },
        );
        Ok(watch)
    }

    fn add_watch_targets(
        &mut self,
        targets: Vec<DebugTarget>,
        interval_ns: i64,
        source: &DebugSource<'_>,
    ) -> Result<WatchInfo> {
        let points = self.watch_count();
        if points.saturating_add(targets.len()) > MAX_WATCHES {
            return Err(debug_limit(format!(
                "the watch limit of {MAX_WATCHES} points is reached"
            )));
        }
        if targets.is_empty() {
            return Err(debug_limit("a watch needs at least one target"));
        }
        if interval_ns < MIN_WATCH_INTERVAL_NS {
            return Err(debug_limit(format!(
                "the watch interval must be at least {} ms",
                MIN_WATCH_INTERVAL_NS / 1_000_000
            )));
        }
        let watch = {
            let mut state = lock_bus(&self.bus);
            state.next_watch = state.next_watch.saturating_add(1).max(1);
            WatchId(state.next_watch)
        };
        let resolved = targets
            .iter()
            .map(|target| {
                resolve_target(source.manifest, target).map_or_else(
                    |_| display_target(target),
                    |resolved| resolved.canonical_key,
                )
            })
            .collect();
        let protected = targets.iter().any(|target| {
            resolve_target(source.manifest, target)
                .is_ok_and(|resolved| protected_resolved(source.manifest, &resolved))
        });
        let active = ActiveWatch {
            targets,
            resolved,
            interval_ns,
            next_due_ns: i64::MIN,
            protected,
        };
        let info = active.info(watch);
        self.watches.insert(watch, active);
        Ok(info)
    }

    pub(crate) fn remove_watch(&mut self, watch: WatchId) -> Option<WatchInfo> {
        self.watches.remove(&watch).map(|active| active.info(watch))
    }

    pub(crate) fn set_audit_sink(&mut self, sink: Option<Box<dyn AuditSink>>) {
        self.audit = sink;
        self.audit_seq = 0;
    }

    pub(crate) fn queued_count(&self) -> usize {
        self.queue.len()
    }

    pub(crate) fn active_force_count(&self) -> usize {
        self.forces.len()
    }

    pub(crate) fn take_results(&mut self) -> Vec<DebugQueueResult> {
        self.results.drain(..).collect()
    }

    pub(crate) fn enqueue(&mut self, prepared: PreparedWrite) -> Result<DebugQueueId> {
        let count = self.queue.len().saturating_add(self.forces.len());
        if count >= MAX_DEBUG_QUEUE_ENTRIES {
            let error = debug_limit(format!(
                "the write/force queue limit of {MAX_DEBUG_QUEUE_ENTRIES} is reached"
            ));
            self.audit_prepared(
                &prepared,
                AuditOutcome::Denied,
                None,
                Some(&error),
                None,
                "queue_limit",
            )?;
            return Err(error);
        }
        let queued_bytes = self
            .queue
            .iter()
            .map(|entry| entry.prepared.queue_bytes)
            .sum::<usize>()
            .saturating_add(
                self.forces
                    .values()
                    .map(|force| force.queue_bytes)
                    .sum::<usize>(),
            );
        if queued_bytes
            .saturating_add(prepared.queue_bytes)
            .gt(&MAX_DEBUG_QUEUE_BYTES)
        {
            let error = debug_limit(format!(
                "the write/force queue byte limit of {MAX_DEBUG_QUEUE_BYTES} is reached"
            ));
            self.audit_prepared(
                &prepared,
                AuditOutcome::Denied,
                None,
                Some(&error),
                None,
                "queue_limit",
            )?;
            return Err(error);
        }
        self.next_queue = self.next_queue.saturating_add(1).max(1);
        let queue_id = DebugQueueId(self.next_queue);
        self.audit_prepared(
            &prepared,
            AuditOutcome::Ok,
            None,
            None,
            Some(queue_id),
            "queued",
        )?;
        self.queue.push_back(QueuedWrite { queue_id, prepared });
        Ok(queue_id)
    }

    pub(crate) fn audit_request_error(
        &mut self,
        kind: DebugWriteKind,
        target: &DebugTarget,
        value: Option<&TypedValue>,
        error: &Error,
        context: Option<(u64, GotId, u64)>,
    ) -> Result<()> {
        let mut event = AuditEvent::new(kind.name(), AuditOutcome::Error);
        event.target = Some(AuditTarget {
            key: audit_target_key(target),
            kind: audit_target_kind(target).to_owned(),
        });
        event.value = value.map(typed_value_json);
        event.generation = context.map(|(generation, _, _)| generation);
        event.build = context.map(|(_, build, _)| build);
        event.cycle = context.map(|(_, _, cycle)| cycle);
        event.detail = Json::object([
            ("code".into(), Json::string(error.code)),
            ("message".into(), Json::string(error.message.clone())),
            ("status".into(), Json::string("rejected")),
        ]);
        self.append_audit(event)
    }

    pub(crate) fn release_force(&mut self, got_id: GotId) -> Result<bool> {
        let Some(force) = self.forces.get(&got_id) else {
            return Ok(false);
        };
        let mut event = AuditEvent::new("force-release", AuditOutcome::Ok);
        event.target = Some(AuditTarget {
            key: force.target_key.clone(),
            kind: "root".into(),
        });
        event.value = Some(typed_value_json(&force.value));
        event.generation = Some(force.generation);
        event.build = Some(force.build_id);
        event.cycle = Some(force.cycle);
        event.detail = Json::object([
            ("status".into(), Json::string("released")),
            ("reason".into(), Json::string("request")),
        ]);
        self.append_audit(event)?;
        self.forces.remove(&got_id);
        Ok(true)
    }

    pub(crate) fn release_all_forces(&mut self) -> Result<usize> {
        let ids: Vec<GotId> = self.forces.keys().copied().collect();
        let mut released = 0;
        for got_id in ids {
            if self.release_force(got_id)? {
                released += 1;
            }
        }
        Ok(released)
    }

    /// Ends watches, queued writes, and active forces at a generation or
    /// session boundary. The state is always cleared, even when an audit
    /// append fails; the audit failure is returned to the caller.
    pub(crate) fn end_state(
        &mut self,
        reason: DebugEndReason,
        generation: Option<u64>,
        build: Option<GotId>,
        cycle: Option<u64>,
    ) -> Result<()> {
        let mut audit_error = None;
        self.watches.clear();
        self.last_generation = None;
        let queued = self.queue.drain(..).collect::<Vec<_>>();
        for entry in &queued {
            self.push_result(DebugQueueResult {
                queue_id: entry.queue_id,
                operation: entry.prepared.kind,
                outcome: DebugQueueOutcome::Rejected,
                generation,
                build_id: build,
                cycle: cycle.unwrap_or(0),
                code: Some(codes::DEBUG_STALE),
                message: Some(format!(
                    "queued {} was discarded because debug state ended",
                    entry.prepared.kind.name()
                )),
                warning: entry.prepared.warning.clone(),
            });
            let error = Error::new(
                codes::DEBUG_STALE,
                "debug state ended before the queued write was applied",
            );
            if let Err(failure) = self.audit_prepared_boundary(
                &entry.prepared,
                AuditOutcome::Error,
                Some(&error),
                Some(entry.queue_id),
                "discarded",
                generation,
                build,
                cycle,
            ) {
                audit_error = Some(failure);
            }
        }
        let force_ids: Vec<GotId> = self.forces.keys().copied().collect();
        for got_id in force_ids {
            let force = self.forces.remove(&got_id);
            if let Some(force) = force {
                let mut event = AuditEvent::new("force-release", AuditOutcome::Ok);
                event.target = Some(AuditTarget {
                    key: force.target_key.clone(),
                    kind: "root".into(),
                });
                event.value = Some(typed_value_json(&force.value));
                event.generation = generation;
                event.build = build;
                event.cycle = cycle;
                event.detail = Json::object([
                    ("status".into(), Json::string("released")),
                    ("reason".into(), Json::string(debug_end_detail(reason))),
                ]);
                if let Err(failure) = self.append_audit(event) {
                    audit_error = Some(failure);
                }
            }
        }
        let mut event = AuditEvent::new(reason.audit_event(), AuditOutcome::Ok);
        event.generation = generation;
        event.build = build;
        event.cycle = cycle;
        event.detail = Json::object([
            ("reason".into(), Json::string(debug_end_detail(reason))),
            (
                "queued_writes".into(),
                Json::unsigned(u64::try_from(queued.len()).unwrap_or(u64::MAX)),
            ),
        ]);
        if let Err(failure) = self.append_audit(event) {
            audit_error = Some(failure);
        }
        let notice = match reason {
            DebugEndReason::Swap | DebugEndReason::Rollback | DebugEndReason::ColdInstall => {
                Some(DebugEndNotice {
                    reason: "swap",
                    code: Some(codes::DEBUG_SWAP),
                    generation: generation.unwrap_or(0),
                    build_id: build.unwrap_or(GotId::ZERO),
                    cycle: cycle.unwrap_or(0),
                })
            }
            DebugEndReason::Shutdown => Some(DebugEndNotice {
                reason: "shutdown",
                code: None,
                generation: generation.unwrap_or(0),
                build_id: build.unwrap_or(GotId::ZERO),
                cycle: cycle.unwrap_or(0),
            }),
            DebugEndReason::TargetExit => Some(DebugEndNotice {
                reason: "target-exited",
                code: None,
                generation: generation.unwrap_or(0),
                build_id: build.unwrap_or(GotId::ZERO),
                cycle: cycle.unwrap_or(0),
            }),
            DebugEndReason::PauseTimeout => Some(DebugEndNotice {
                reason: "pause-timeout",
                code: Some(codes::DEBUG_SESSION_TIMEOUT),
                generation: generation.unwrap_or(0),
                build_id: build.unwrap_or(GotId::ZERO),
                cycle: cycle.unwrap_or(0),
            }),
            DebugEndReason::SessionEnd => None,
        };
        {
            let mut state = lock_bus(&self.bus);
            // A later teardown (for example host shutdown after a target trap)
            // must not replace the notice that already ended the session, or
            // the client would see `shutdown` instead of `target-exited`.
            let had_session = state.session.is_some();
            state.session = None;
            // No command submitted before the end may be applied against a
            // later generation or a later session.
            state.commands.clear();
            if notice.is_some() && had_session {
                state.end_notice = notice;
            }
        }
        if let Some(error) = audit_error {
            return Err(error);
        }
        Ok(())
    }

    fn push_result(&mut self, result: DebugQueueResult) {
        if self.results.len() >= MAX_DEBUG_QUEUE_RESULTS {
            self.results.pop_front();
        }
        if lock_bus(&self.bus).session.is_some() {
            self.push_event(DebugEvent::Queue(result.clone()));
        }
        self.results.push_back(result);
    }

    fn append_audit(&mut self, mut event: AuditEvent) -> Result<()> {
        let Some(sink) = self.audit.as_mut() else {
            return Ok(());
        };
        {
            let state = lock_bus(&self.bus);
            if event.session.is_none() {
                event.session.clone_from(&state.session);
            }
            if state.session.is_some() {
                if event.scope.is_none() {
                    event.scope = audit_scope(event.event).map(str::to_owned);
                }
                event.monotonic_ns = crate::transport::monotonic_ns();
                event.local_date_ns = crate::transport::local_date_ns();
            }
        }
        self.audit_seq = self.audit_seq.saturating_add(1).max(1);
        event.seq = self.audit_seq;
        sink.record(&event).map_err(|error| {
            Error::new(
                codes::DEBUG_AUDIT,
                format!("debug audit append failed: {}", error.message),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn audit_prepared(
        &mut self,
        prepared: &PreparedWrite,
        outcome: AuditOutcome,
        error: Option<&Error>,
        append_error: Option<&Error>,
        queue_id: Option<DebugQueueId>,
        status: &str,
    ) -> Result<()> {
        self.audit_prepared_boundary(
            prepared,
            outcome,
            error.or(append_error),
            queue_id,
            status,
            Some(prepared.generation),
            Some(prepared.build_id),
            Some(prepared.cycle),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn audit_prepared_boundary(
        &mut self,
        prepared: &PreparedWrite,
        outcome: AuditOutcome,
        error: Option<&Error>,
        queue_id: Option<DebugQueueId>,
        status: &str,
        generation: Option<u64>,
        build: Option<GotId>,
        cycle: Option<u64>,
    ) -> Result<()> {
        let mut event = AuditEvent::new(prepared.kind.name(), outcome);
        event.target = Some(AuditTarget {
            key: prepared.target_key.clone(),
            kind: "root".into(),
        });
        event.value = Some(typed_value_json(&prepared.value));
        event.generation = generation;
        event.build = build;
        event.cycle = cycle;
        let mut detail = BTreeMap::new();
        if let Some(queue_id) = queue_id {
            detail.insert("queue_id".into(), Json::unsigned(queue_id.0));
        }
        if let Some(error) = error {
            detail.insert("code".into(), Json::string(error.code));
            detail.insert("message".into(), Json::string(error.message.clone()));
        }
        if let Some(warning) = &prepared.warning {
            detail.insert("warning".into(), Json::string(warning.code));
        }
        detail.insert("status".into(), Json::string(status));
        event.detail = Json::Object(detail);
        self.append_audit(event)
    }

    /// Applies queued writes in queue order, then active forces in ascending
    /// identity order. Every entry is revalidated against the current
    /// manifest and rotated with the new generation's layout; a stale entry is
    /// rejected and never retargeted.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_boundary(
        &mut self,
        manifest: &Manifest,
        target: &Target,
        info: Option<&crate::ModuleInfo>,
        roots: &mut crate::RootRegistry,
        generation: u64,
        build: GotId,
        cycle: u64,
    ) {
        let queued: Vec<QueuedWrite> = self.queue.drain(..).collect();
        for entry in queued {
            match revalidate_prepared(&entry.prepared, manifest, target, info, generation, build) {
                Ok(root) => match root {
                    Some(root) => {
                        match write_root_bytes(info, roots, root, &entry.prepared.bytes) {
                            Ok(()) => {
                                if entry.prepared.kind == DebugWriteKind::Force {
                                    self.forces
                                        .insert(entry.prepared.got_id, entry.prepared.clone());
                                }
                                self.push_result(DebugQueueResult {
                                    queue_id: entry.queue_id,
                                    operation: entry.prepared.kind,
                                    outcome: DebugQueueOutcome::Applied,
                                    generation: Some(generation),
                                    build_id: Some(build),
                                    cycle,
                                    code: None,
                                    message: None,
                                    warning: entry.prepared.warning.clone(),
                                });
                            }
                            Err(error) => {
                                self.reject_queued(&entry, &error, generation, build, cycle);
                            }
                        }
                    }
                    None => {
                        let error = Error::new(
                            codes::DEBUG_STALE,
                            "the target root was removed before the write could be applied",
                        );
                        self.reject_queued(&entry, &error, generation, build, cycle);
                    }
                },
                Err(error) => {
                    self.reject_queued(&entry, &error, generation, build, cycle);
                }
            }
        }

        let force_ids: Vec<GotId> = self.forces.keys().copied().collect();
        for got_id in force_ids {
            let Some(force) = self.forces.get(&got_id).cloned() else {
                continue;
            };
            let result = revalidate_prepared(&force, manifest, target, info, generation, build)
                .and_then(|root| match root {
                    Some(root) => write_root_bytes(info, roots, root, &force.bytes),
                    None => Err(Error::new(
                        codes::DEBUG_STALE,
                        "the force target root was removed",
                    )),
                });
            if let Err(error) = result {
                self.forces.remove(&got_id);
                self.audit_active_rejection(&force, &error, generation, build, cycle);
            }
        }
    }

    /// Substitutes active direct input forces after the backend latch.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_input_forces(
        &mut self,
        manifest: &Manifest,
        target: &Target,
        info: Option<&crate::ModuleInfo>,
        roots: &mut crate::RootRegistry,
        generation: u64,
        build: GotId,
        cycle: u64,
    ) {
        let Some(info) = info else {
            return;
        };
        let force_ids: Vec<GotId> = self.forces.keys().copied().collect();
        for got_id in force_ids {
            let Some(force) = self.forces.get(&got_id).cloned() else {
                continue;
            };
            let result =
                revalidate_prepared(&force, manifest, target, Some(info), generation, build)
                    .and_then(|root| match root {
                        Some(root) if root.root_kind == RootKind::Direct => {
                            let binding = validate_direct_binding(info, root)?;
                            if binding.area == 1 {
                                write_root_bytes(Some(info), roots, root, &force.bytes)
                            } else {
                                Ok(())
                            }
                        }
                        _ => Ok(()),
                    });
            if let Err(error) = result {
                self.forces.remove(&got_id);
                self.audit_active_rejection(&force, &error, generation, build, cycle);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn reject_queued(
        &mut self,
        entry: &QueuedWrite,
        error: &Error,
        generation: u64,
        build: GotId,
        cycle: u64,
    ) {
        self.push_result(DebugQueueResult {
            queue_id: entry.queue_id,
            operation: entry.prepared.kind,
            outcome: DebugQueueOutcome::Rejected,
            generation: Some(generation),
            build_id: Some(build),
            cycle,
            code: Some(error.code),
            message: Some(error.message.clone()),
            warning: entry.prepared.warning.clone(),
        });
        if let Err(audit_error) = self.audit_prepared_boundary(
            &entry.prepared,
            AuditOutcome::Error,
            Some(error),
            Some(entry.queue_id),
            "rejected",
            Some(generation),
            Some(build),
            Some(cycle),
        ) {
            // The queue entry is already rejected and no storage was changed.
            // Fail closed; the append failure is the operator-visible problem.
            let _ = audit_error;
        }
    }

    fn audit_active_rejection(
        &mut self,
        force: &PreparedWrite,
        error: &Error,
        generation: u64,
        build: GotId,
        cycle: u64,
    ) {
        let mut event = AuditEvent::new("force", AuditOutcome::Error);
        event.target = Some(AuditTarget {
            key: force.target_key.clone(),
            kind: "root".into(),
        });
        event.value = Some(typed_value_json(&force.value));
        event.generation = Some(generation);
        event.build = Some(build);
        event.cycle = Some(cycle);
        event.detail = Json::object([
            ("code".into(), Json::string(error.code)),
            ("message".into(), Json::string(error.message.clone())),
            ("status".into(), Json::string("rejected")),
        ]);
        let _ = self.append_audit(event);
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
                DebugCommand::ReadWait { target, reply } => {
                    let result = match source.as_ref() {
                        Some(source) => read_request(source, &target).map_err(DebugFailure::from),
                        None => Err(DebugFailure::new(
                            codes::DEBUG_STALE,
                            "no active generation is available for a typed read",
                        )),
                    };
                    reply.set(result);
                }
                DebugCommand::WriteWait {
                    kind,
                    target,
                    reply,
                } => {
                    let result = match source.as_ref() {
                        Some(source) => match prepare_write(source, &target, kind) {
                            Ok(prepared) => self.enqueue(prepared).map_err(DebugFailure::from),
                            Err(error) => {
                                let _ = self.audit_request_error(
                                    kind,
                                    &target.target,
                                    Some(&target.value),
                                    &error,
                                    Some((source.generation, source.build_id, source.cycle)),
                                );
                                Err(DebugFailure::from(error))
                            }
                        },
                        None => Err(DebugFailure::new(
                            codes::DEBUG_STALE,
                            "no active generation is available for a write",
                        )),
                    };
                    reply.set(result);
                }
                DebugCommand::ReleaseWait { target, reply } => {
                    let result = match source.as_ref() {
                        Some(source) => resolve_root_id(source, &target)
                            .and_then(|got_id| self.release_force(got_id))
                            .map_err(DebugFailure::from),
                        None => Err(DebugFailure::new(
                            codes::DEBUG_STALE,
                            "no active generation is available for a release",
                        )),
                    };
                    reply.set(result);
                }
                DebugCommand::ReleaseAllWait { reply } => {
                    let result = self.release_all_forces().map_err(DebugFailure::from);
                    reply.set(result);
                }
                DebugCommand::AddWatch { watch, spec } => {
                    if source.is_none() {
                        self.push_event(DebugEvent::Error(DebugError {
                            code: codes::DEBUG_STALE,
                            message: "no active generation is available for a watch".into(),
                            request: None,
                            watch: Some(watch),
                        }));
                    } else if self.watch_count() >= MAX_WATCHES {
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
                                targets: vec![spec.target.clone()],
                                resolved: vec![display_target(&spec.target)],
                                interval_ns: spec.interval_ns,
                                next_due_ns: now_ns.saturating_add(spec.interval_ns),
                                protected: false,
                            },
                        );
                    }
                    emitted += 1;
                }
                DebugCommand::AddWatchWait {
                    targets,
                    interval_ns,
                    reply,
                } => {
                    let result = match source.as_ref() {
                        Some(source) => self
                            .add_watch_targets(targets, interval_ns, source)
                            .map_err(DebugFailure::from),
                        None => Err(DebugFailure::new(
                            codes::DEBUG_STALE,
                            "no active generation is available for a watch",
                        )),
                    };
                    reply.set(result);
                }
                DebugCommand::RemoveWatch { watch } => {
                    self.watches.remove(&watch);
                }
                DebugCommand::RemoveWatchWait { watch, reply } => {
                    reply.set(self.remove_watch(watch));
                }
                DebugCommand::ListWatchesWait { reply } => {
                    reply.set(self.watch_list());
                }
                DebugCommand::ClearSessionWait { reply } => {
                    let (generation, build, cycle) =
                        source.as_ref().map_or((None, None, None), |source| {
                            (
                                Some(source.generation),
                                Some(source.build_id),
                                Some(source.cycle),
                            )
                        });
                    let result = self
                        .end_state(DebugEndReason::SessionEnd, generation, build, cycle)
                        .map_err(DebugFailure::from);
                    reply.set(result);
                }
                DebugCommand::ManifestWait { binary, reply } => {
                    let result = match source.as_ref() {
                        Some(source) => manifest_reply(source, binary).map_err(DebugFailure::from),
                        None => Err(DebugFailure::new(
                            codes::DEBUG_STALE,
                            "no active manifest is available",
                        )),
                    };
                    reply.set(result);
                }
            }
        }

        if let Some(source) = source.as_ref() {
            let snapshot = self.snapshot(source);
            lock_bus(&self.bus).snapshot = Some(snapshot);
        } else {
            lock_bus(&self.bus).snapshot = None;
        }

        let Some(source) = source.as_ref() else {
            return emitted;
        };
        let due: Vec<(WatchId, Vec<DebugTarget>)> = self
            .watches
            .iter()
            .filter(|(_, watch)| now_ns >= watch.next_due_ns)
            .map(|(id, watch)| (*id, watch.targets.clone()))
            .collect();
        if due.is_empty() {
            return emitted;
        }
        let mut values = Vec::new();
        for (watch_id, targets) in due {
            for target in targets {
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
            }
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

    fn snapshot(&self, source: &DebugSource<'_>) -> DebugSnapshot {
        let frames = source
            .tables
            .as_ref()
            .is_some_and(|tables| tables.frames.is_some());
        DebugSnapshot {
            generation: source.generation,
            build_id: source.build_id,
            cycle: source.cycle,
            mode: source.mode.to_owned(),
            hooks: source.debug_hooks && source.metadata_error.is_none(),
            debug_info: source.debug_info.unwrap_or("none").to_owned(),
            checkpoints: source.checkpoints && source.metadata_error.is_none(),
            stepping: source.tables.as_ref().is_some_and(|tables| tables.stepping),
            frames,
            frames_error: source
                .tables
                .as_ref()
                .and_then(|tables| tables.frames_error.clone()),
            metadata_error: source.metadata_error.map(str::to_owned),
            queued_writes: self.queue.len(),
            active_forces: self.forces.len(),
            watches: self.watch_count(),
            breakpoints: 0,
            tables: source.tables.clone(),
        }
    }

    fn push_event(&self, event: DebugEvent) {
        let mut state = lock_bus(&self.bus);
        push_event(&mut state, event);
    }
}

fn display_target(target: &DebugTarget) -> String {
    audit_target_key(target)
}

fn validate_watch_target(target: &DebugTarget) -> Result<()> {
    match normalize_target(target)? {
        NormalizedTarget::Local { .. } => Err(Error::new(
            codes::DEBUG_STOP,
            "locals are not watchable: a stopped frame is immutable",
        )),
        NormalizedTarget::Path(_) | NormalizedTarget::GotId(_) => Ok(()),
    }
}

fn audit_scope(event: &str) -> Option<&'static str> {
    match event {
        "write" => Some("write"),
        "force" | "force-release" => Some("force"),
        "read-protected" | "manifest-export" => Some("observe"),
        _ => None,
    }
}

const MAX_MANIFEST_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

fn manifest_reply(source: &DebugSource<'_>, binary: bool) -> Result<ManifestReply> {
    let digest = source.manifest_digest.to_hex();
    if binary {
        let bytes = source.manifest_binary.ok_or_else(|| {
            Error::new(
                codes::DEBUG_STALE,
                "no active binary manifest projection is available",
            )
        })?;
        if bytes.len() > MAX_MANIFEST_RESPONSE_BYTES {
            return Err(debug_limit(format!(
                "the binary manifest exceeds the {MAX_MANIFEST_RESPONSE_BYTES}-byte response limit"
            )));
        }
        Ok(ManifestReply {
            format: "binary",
            digest,
            data: ManifestData::Binary(bytes.to_vec()),
        })
    } else {
        let json = source.manifest_json.ok_or_else(|| {
            Error::new(
                codes::DEBUG_STALE,
                "no active canonical JSON manifest is available",
            )
        })?;
        if json.len() > MAX_MANIFEST_RESPONSE_BYTES {
            return Err(debug_limit(format!(
                "the manifest exceeds the {MAX_MANIFEST_RESPONSE_BYTES}-byte response limit"
            )));
        }
        Ok(ManifestReply {
            format: "json",
            digest,
            data: ManifestData::Json(json.to_owned()),
        })
    }
}

fn audit_target_key(target: &DebugTarget) -> String {
    match target {
        DebugTarget::Path(path) => path.clone(),
        DebugTarget::GotId(got_id) => got_id.to_hex(),
        DebugTarget::Local { frame, path } => format!("{frame}::{path}"),
    }
}

fn audit_target_kind(target: &DebugTarget) -> &'static str {
    match target {
        DebugTarget::Path(_) => "path",
        DebugTarget::GotId(_) => "root",
        DebugTarget::Local { .. } => "local",
    }
}

const fn debug_end_detail(reason: DebugEndReason) -> &'static str {
    match reason {
        DebugEndReason::Swap => "swap",
        DebugEndReason::Rollback => "rollback",
        DebugEndReason::ColdInstall => "cold_install",
        DebugEndReason::Shutdown => "shutdown",
        DebugEndReason::TargetExit => "target_exit",
        DebugEndReason::PauseTimeout => "pause_timeout",
        DebugEndReason::SessionEnd => "session_end",
    }
}

#[derive(Clone)]
pub(crate) struct DebugSource<'a> {
    pub(crate) target: &'a Target,
    pub(crate) manifest: &'a Manifest,
    pub(crate) roots: &'a crate::RootRegistry,
    pub(crate) info: Option<&'a crate::ModuleInfo>,
    pub(crate) generation: u64,
    pub(crate) build_id: GotId,
    pub(crate) cycle: u64,
    pub(crate) mode: &'a str,
    pub(crate) debug_hooks: bool,
    pub(crate) debug_info: Option<&'a str>,
    pub(crate) checkpoints: bool,
    pub(crate) metadata_error: Option<&'a str>,
    pub(crate) tables: Option<std::sync::Arc<crate::stop::DebugTables>>,
    pub(crate) manifest_digest: stc_got::digest::Digest32,
    pub(crate) manifest_json: Option<&'a str>,
    pub(crate) manifest_binary: Option<&'a [u8]>,
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
                    + read.view.as_ref().map_or(0, |view| view.estimated_bytes())
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
        DebugEvent::Queue(result) => {
            160 + result.operation.name().len() + result.message.as_ref().map_or(0, String::len)
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
enum AggregateTargetKind {
    Record {
        ty: TypeRecord,
    },
    Program {
        fields: Vec<AggregateFieldRecord>,
        shape_key: String,
        layout_hash: GotId,
    },
    Array {
        ty: TypeKey,
        layout_hash: GotId,
    },
}

#[derive(Clone, Debug)]
struct AggregateTarget {
    kind: AggregateTargetKind,
    package: String,
    path: String,
    qualifier: Option<String>,
}

impl AggregateTarget {
    fn record(&self) -> Option<&TypeRecord> {
        match &self.kind {
            AggregateTargetKind::Record { ty } => Some(ty),
            AggregateTargetKind::Program { .. } | AggregateTargetKind::Array { .. } => None,
        }
    }

    fn type_key(&self) -> String {
        match &self.kind {
            AggregateTargetKind::Record { ty } => ty.type_key.clone(),
            AggregateTargetKind::Program { shape_key, .. } => shape_key.clone(),
            AggregateTargetKind::Array { ty, .. } => ty.render(),
        }
    }

    fn layout_hash(&self) -> GotId {
        match &self.kind {
            AggregateTargetKind::Record { ty } => ty.layout_hash,
            AggregateTargetKind::Program { layout_hash, .. }
            | AggregateTargetKind::Array { layout_hash, .. } => *layout_hash,
        }
    }
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
        type_key: aggregate.type_key(),
        layout_hash: aggregate.layout_hash(),
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
                kind: AggregateTargetKind::Record { ty },
                package,
                path: query.path.clone(),
                qualifier: group.qualifier.clone(),
            });
        }
    }

    if let Some(target) = build_manifest_aggregate(manifest, group, query)? {
        return Ok(target);
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
                    "debug path `{}` names an array-of-aggregate storage whose parent shape is not described by the active manifest; read a field leaf instead",
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
                "debug path `{}` has split roots but the active manifest does not identify its aggregate shape",
                query.path
            ),
        ));
    };
    Ok(AggregateTarget {
        kind: AggregateTargetKind::Record { ty },
        package,
        path: query.path.clone(),
        qualifier: group.qualifier.clone(),
    })
}

fn build_manifest_aggregate(
    manifest: &Manifest,
    group: &Group<'_>,
    query: &TargetQuery,
) -> Result<Option<AggregateTarget>> {
    let prefix = format!("{}.", query.path);
    let mut shapes: Vec<&AggregateRecord> = manifest
        .aggregates
        .iter()
        .filter(|shape| {
            shape.package == group.package
                && shape.qualifier == group.qualifier
                && (shape.path == query.path
                    || query.path.starts_with(&format!("{}.", shape.path))
                    || shape.path.starts_with(&prefix))
        })
        .collect();
    if shapes.is_empty() {
        return Ok(None);
    }
    // Prefer the exact matching shape, otherwise the longest declared prefix.
    shapes.sort_by_key(|shape| {
        (
            shape.path != query.path,
            std::cmp::Reverse(shape.path.len()),
            shape.path.clone(),
        )
    });
    let shape = shapes[0];
    if shape.path != query.path && !query.path.starts_with(&format!("{}.", shape.path)) {
        return Ok(None);
    }
    if shape.path == query.path {
        let kind = if shape.kind == AggregateKind::Program {
            AggregateTargetKind::Program {
                fields: shape.fields.clone(),
                shape_key: shape.shape_key.clone(),
                layout_hash: shape.layout_hash,
            }
        } else {
            let ty = TypeKey::parse(&shape.shape_key).map_err(|error| {
                Error::new(
                    codes::DEBUG_TYPE,
                    format!(
                        "aggregate `{}::{}` has an invalid shape key: {}",
                        shape.package, shape.path, error.message
                    ),
                )
            })?;
            match &ty {
                TypeKey::Named(got_id) => {
                    let Some(record) = type_by_id(manifest, *got_id) else {
                        return Err(Error::new(
                            codes::DEBUG_TYPE,
                            format!(
                                "aggregate `{}::{}` names type {} with no type record",
                                shape.package,
                                shape.path,
                                got_id.to_hex()
                            ),
                        ));
                    };
                    match record.type_kind {
                        TypeKind::Struct
                        | TypeKind::FunctionBlock
                        | TypeKind::Class
                        | TypeKind::Interface => AggregateTargetKind::Record { ty: record.clone() },
                        TypeKind::Enum | TypeKind::Subrange | TypeKind::Alias => return Ok(None),
                    }
                }
                TypeKey::Array { .. } => AggregateTargetKind::Array {
                    ty,
                    layout_hash: shape.layout_hash,
                },
                _ => return Ok(None),
            }
        };
        return Ok(Some(AggregateTarget {
            kind,
            package: shape.package.clone(),
            path: shape.path.clone(),
            qualifier: shape.qualifier.clone(),
        }));
    }

    let remainder = query
        .path
        .strip_prefix(&shape.path)
        .and_then(|text| text.strip_prefix('.'))
        .ok_or_else(|| Error::new(codes::DEBUG_UNKNOWN, "invalid aggregate shape prefix"))?;
    let mut segments = remainder.split('.');
    let mut path = shape.path.clone();
    let (mut ty, mut layout_hash) = if shape.kind == AggregateKind::Program {
        let Some(segment) = segments.next() else {
            return Ok(None);
        };
        let Some(field) = shape
            .fields
            .iter()
            .find(|field| field.path == segment || field.path.starts_with(&format!("{segment}.")))
        else {
            return Err(Error::new(
                codes::DEBUG_UNKNOWN,
                format!(
                    "program `{}` has no field `{segment}` for this debug path",
                    shape.shape_key
                ),
            ));
        };
        if field.path != segment {
            return Err(Error::new(
                codes::DEBUG_UNKNOWN,
                format!(
                    "program field path `{}` is flattened; the intermediate aggregate type cannot be resolved",
                    field.path
                ),
            ));
        }
        path.push('.');
        path.push_str(segment);
        let ty = TypeKey::parse(&field.type_key).map_err(|error| {
            Error::new(
                codes::DEBUG_TYPE,
                format!(
                    "program `{}` field `{segment}` has an invalid type key: {}",
                    shape.shape_key, error.message
                ),
            )
        })?;
        (ty, field.layout_hash)
    } else {
        let ty = TypeKey::parse(&shape.shape_key).map_err(|error| {
            Error::new(
                codes::DEBUG_TYPE,
                format!(
                    "aggregate `{}::{}` has an invalid shape key: {}",
                    shape.package, shape.path, error.message
                ),
            )
        })?;
        (ty, shape.layout_hash)
    };
    for segment in segments {
        path.push('.');
        path.push_str(segment);
        let (next, next_layout) = descend_shape(manifest, &ty, segment, layout_hash, &path)?;
        ty = next;
        layout_hash = next_layout;
    }
    let Some(target) = target_from_key(manifest, shape, ty, &path, layout_hash) else {
        return Err(Error::new(
            codes::DEBUG_UNKNOWN,
            format!(
                "debug path `{}` does not name an aggregate shape",
                query.path
            ),
        ));
    };
    Ok(Some(target))
}

fn descend_shape(
    manifest: &Manifest,
    ty: &TypeKey,
    segment: &str,
    layout_hash: GotId,
    path: &str,
) -> Result<(TypeKey, GotId)> {
    match ty {
        TypeKey::Array { element, .. } => {
            if !is_element_segment(segment) {
                return Err(Error::new(
                    codes::DEBUG_UNKNOWN,
                    format!("debug path `{path}` must enter an array with an `E<index>` segment"),
                ));
            }
            let layout = match element.as_ref() {
                TypeKey::Named(got_id) => type_by_id(manifest, *got_id).map(|ty| ty.layout_hash),
                TypeKey::Array { .. } => Some(GotId::ZERO),
                _ => None,
            }
            .unwrap_or(layout_hash);
            Ok(((**element).clone(), layout))
        }
        TypeKey::Named(got_id) => {
            let Some(record) = type_by_id(manifest, *got_id) else {
                return Err(Error::new(
                    codes::DEBUG_TYPE,
                    format!("type {} has no manifest type record", got_id.to_hex()),
                ));
            };
            if !matches!(
                record.type_kind,
                TypeKind::Struct | TypeKind::FunctionBlock | TypeKind::Class | TypeKind::Interface
            ) {
                return Err(Error::new(
                    codes::DEBUG_UNKNOWN,
                    format!(
                        "type `{}` is not an aggregate for debug path `{path}`",
                        record.type_key
                    ),
                ));
            }
            let Some(field) = record.fields.iter().find(|field| {
                field.path == segment || field.path.starts_with(&format!("{segment}."))
            }) else {
                return Err(Error::new(
                    codes::DEBUG_UNKNOWN,
                    format!(
                        "type `{}` has no field `{segment}` for this debug path",
                        record.type_key
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
                        field.path, record.type_key, error.message
                    ),
                )
            })?;
            Ok((field_ty, field.layout_hash))
        }
        _ => Err(Error::new(
            codes::DEBUG_UNKNOWN,
            format!("debug path `{path}` descends into a non-aggregate type"),
        )),
    }
}

fn target_from_key(
    manifest: &Manifest,
    shape: &AggregateRecord,
    ty: TypeKey,
    path: &str,
    layout_hash: GotId,
) -> Option<AggregateTarget> {
    let kind = match &ty {
        TypeKey::Named(got_id) => {
            let record = type_by_id(manifest, *got_id)?;
            match record.type_kind {
                TypeKind::Struct
                | TypeKind::FunctionBlock
                | TypeKind::Class
                | TypeKind::Interface => AggregateTargetKind::Record { ty: record.clone() },
                TypeKind::Enum | TypeKind::Subrange | TypeKind::Alias => return None,
            }
        }
        TypeKey::Array { .. } => {
            // A nested array reached through another array has no declared layout
            // hash in the manifest; reject it rather than report a fabricated one.
            if layout_hash == GotId::ZERO {
                return None;
            }
            AggregateTargetKind::Array { ty, layout_hash }
        }
        _ => return None,
    };
    Some(AggregateTarget {
        kind,
        package: shape.package.clone(),
        path: path.to_owned(),
        qualifier: shape.qualifier.clone(),
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
                "debug path enters array element `{segment}`, whose aggregate parent shape is not described by the active manifest"
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
            let protected = protected_resolved(source.manifest, &resolved);
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
            let view = request
                .include_view
                .then(|| Box::new(build_live_view(source, &resolved, &typed, &request.target)));
            Ok(DebugRead {
                target: request.target.clone(),
                resolved_key: resolved.canonical_key,
                generation: source.generation,
                build_id: source.build_id,
                cycle: source.cycle,
                atomic: resolved.atomic,
                consistent: true,
                protected,
                type_key: typed.type_key,
                value: typed.value,
                view,
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
    let view = request.include_view.then(|| {
        Box::new(snapshot_view(
            source.manifest,
            &local.value,
            view_target(&request.target),
            path.to_owned(),
            frame.generation,
            frame.build_id,
            frame.cycle,
            local.atomic,
        ))
    });
    Ok(DebugRead {
        target: request.target.clone(),
        resolved_key: path.to_owned(),
        generation: frame.generation,
        build_id: frame.build_id,
        cycle: frame.cycle,
        atomic: local.atomic,
        consistent: true,
        protected: false,
        type_key: local.value.type_key.clone(),
        value: local.value.value.clone(),
        view,
    })
}

fn protected_resolved(manifest: &Manifest, resolved: &Resolved) -> bool {
    match &resolved.shape {
        ResolvedShape::Root(root) => protected_root(root),
        ResolvedShape::Aggregate(_) => {
            let prefix = format!("{}.", resolved.canonical_key);
            manifest.roots.iter().any(|root| {
                protected_root(root)
                    && (root.key == resolved.canonical_key
                        || root.key.starts_with(&prefix)
                        || root.key.eq_ignore_ascii_case(&resolved.canonical_key))
            })
        }
    }
}

fn protected_root(root: &RootRecord) -> bool {
    root.retention == Retention::Retain || root.root_kind == RootKind::Direct
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
    match &aggregate.kind {
        AggregateTargetKind::Record { ty } => {
            let mut fields = Vec::with_capacity(ty.fields.len());
            for field in &ty.fields {
                fields.push(decode_aggregate_field_spec(
                    source,
                    aggregate,
                    &field.path,
                    &field.type_key,
                    field.layout_hash,
                )?);
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
        AggregateTargetKind::Program {
            fields, shape_key, ..
        } => {
            let mut decoded = Vec::with_capacity(fields.len());
            for field in fields {
                decoded.push(decode_aggregate_field_spec(
                    source,
                    aggregate,
                    &field.path,
                    &field.type_key,
                    field.layout_hash,
                )?);
            }
            Ok(TypedValue {
                type_key: shape_key.clone(),
                value: DebugValue::Aggregate(decoded),
            })
        }
        AggregateTargetKind::Array { ty, .. } => {
            let type_key = ty.render();
            match decode_split_array_path(source, aggregate, &aggregate.path, &type_key, ty)? {
                Some(value) => Ok(value),
                None => Ok(unavailable_typed(type_key)),
            }
        }
    }
}

fn decode_aggregate_field_spec(
    source: &DebugSource<'_>,
    aggregate: &AggregateTarget,
    field_path_segment: &str,
    field_type_key: &str,
    field_layout_hash: GotId,
) -> Result<NamedValue> {
    let field_path = format!("{}.{}", aggregate.path, field_path_segment);
    if let Some(root) = find_field_root(
        source.manifest,
        &aggregate.package,
        &field_path,
        aggregate.qualifier.as_deref(),
    ) {
        if root.type_key != field_type_key {
            return Err(Error::new(
                codes::DEBUG_TYPE,
                format!(
                    "root `{}` has type `{}` but field `{}` declares `{field_type_key}`",
                    root.key, root.type_key, field_path_segment
                ),
            ));
        }
        if root.layout_hash != field_layout_hash {
            return Err(Error::new(
                codes::DEBUG_STALE,
                format!(
                    "root `{}` layout does not match field `{field_path_segment}`",
                    root.key
                ),
            ));
        }
        let bytes = root_bytes(source, root)?;
        let ty = TypeKey::parse(field_type_key).map_err(|error| {
            Error::new(
                codes::DEBUG_TYPE,
                format!(
                    "field `{field_path_segment}` has an invalid type key: {}",
                    error.message
                ),
            )
        })?;
        let value = decode_type_value(
            source.manifest,
            source.target,
            &ty,
            field_type_key,
            &bytes,
            source.generation,
        )?;
        return Ok(NamedValue {
            name: field_path_segment.to_owned(),
            value,
        });
    }

    let ty = TypeKey::parse(field_type_key).map_err(|error| {
        Error::new(
            codes::DEBUG_TYPE,
            format!(
                "field `{field_path_segment}` has an invalid type key: {}",
                error.message
            ),
        )
    })?;
    match &ty {
        TypeKey::Named(got_id) => {
            let Some(record) = type_by_id(source.manifest, *got_id) else {
                return Ok(NamedValue {
                    name: field_path_segment.to_owned(),
                    value: unavailable_value(field_type_key.to_owned()),
                });
            };
            match record.type_kind {
                TypeKind::Struct | TypeKind::FunctionBlock | TypeKind::Class => {
                    let child = AggregateTarget {
                        kind: AggregateTargetKind::Record { ty: record.clone() },
                        package: aggregate.package.clone(),
                        path: field_path,
                        qualifier: aggregate.qualifier.clone(),
                    };
                    let value = decode_aggregate(source, &child)?;
                    Ok(NamedValue {
                        name: field_path_segment.to_owned(),
                        value,
                    })
                }
                TypeKind::Interface => Ok(NamedValue {
                    name: field_path_segment.to_owned(),
                    value: unavailable_value(field_type_key.to_owned()),
                }),
                TypeKind::Enum | TypeKind::Subrange | TypeKind::Alias => Ok(NamedValue {
                    name: field_path_segment.to_owned(),
                    value: unavailable_value(field_type_key.to_owned()),
                }),
            }
        }
        TypeKey::Array { .. } => {
            match decode_split_array_path(source, aggregate, &field_path, field_type_key, &ty)? {
                Some(value) => Ok(NamedValue {
                    name: field_path_segment.to_owned(),
                    value,
                }),
                None => Ok(NamedValue {
                    name: field_path_segment.to_owned(),
                    value: unavailable_value(field_type_key.to_owned()),
                }),
            }
        }
        _ => Ok(NamedValue {
            name: field_path_segment.to_owned(),
            value: unavailable_value(field_type_key.to_owned()),
        }),
    }
}

fn decode_split_array_path(
    source: &DebugSource<'_>,
    aggregate: &AggregateTarget,
    base_path: &str,
    type_key: &str,
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
            format!("split array `{base_path}` has a conformant or invalid bound"),
        ));
    };
    if upper < lower {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!("split array `{base_path}` has inverted bounds"),
        ));
    }
    let count = u128::try_from(*upper - *lower)
        .ok()
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| {
            Error::new(
                codes::DEBUG_LIMIT,
                format!("split array `{base_path}` is too large"),
            )
        })?;
    if count > 1_000_000 {
        return Err(Error::new(
            codes::DEBUG_LIMIT,
            format!("split array `{base_path}` has more than one million elements"),
        ));
    }
    let mut values = Vec::with_capacity(usize::try_from(count).unwrap_or(usize::MAX));
    let mut any = false;
    for index in 0..count {
        let child_path = format!("{base_path}.E{index}");
        let present = source.manifest.roots.iter().any(|root| {
            parse_key(&root.key).is_some_and(|parsed| {
                parsed.package == aggregate.package
                    && parsed.path.starts_with(&format!("{child_path}."))
                    || (parsed.path == child_path
                        && match &parsed.qualifier {
                            None => true,
                            Some(qualifier) => is_hidden_qualifier(qualifier),
                        })
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
                match record.type_kind {
                    TypeKind::Struct | TypeKind::FunctionBlock | TypeKind::Class => {
                        let child = AggregateTarget {
                            kind: AggregateTargetKind::Record { ty: record.clone() },
                            package: aggregate.package.clone(),
                            path: child_path,
                            qualifier: aggregate.qualifier.clone(),
                        };
                        values.push(decode_aggregate(source, &child)?);
                    }
                    TypeKind::Interface | TypeKind::Enum | TypeKind::Subrange | TypeKind::Alias => {
                        values.push(unavailable_typed(element.render()));
                    }
                }
            }
            TypeKey::Array { .. } => {
                match decode_split_array_path(
                    source,
                    aggregate,
                    &child_path,
                    &element.render(),
                    element,
                )? {
                    Some(value) => values.push(value),
                    None => values.push(unavailable_typed(element.render())),
                }
            }
            _ => values.push(unavailable_typed(element.render())),
        }
    }
    if !any {
        return Ok(None);
    }
    Ok(Some(TypedValue {
        type_key: type_key.to_owned(),
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
    let Some(record) = aggregate.record() else {
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
            verify_object_target(&inner, record, &root.key)?;
            let (identity, present) = pointer_identity(
                &bytes,
                source.target.pointer_width,
                source.target.endianness,
                &record.type_key,
                source.generation,
            )?;
            Ok((identity, present))
        }
        TypeKey::RefTo(inner) => {
            verify_object_target(&inner, record, &root.key)?;
            let (identity, present) = reference_identity(
                &bytes,
                source.target.endianness,
                &record.type_key,
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

// ---------------------------------------------------------------------------
// Manifest-assisted split-object view
// ---------------------------------------------------------------------------

/// Builds the presentation view for one successful live read.
///
/// The decoded value drives the nodes, so a view can never disagree with the
/// typed read. Manifest type/aggregate records and root records supply the
/// split metadata; no storage is read a second time and no address is exposed.
fn build_live_view(
    source: &DebugSource<'_>,
    resolved: &Resolved,
    typed: &TypedValue,
    target: &DebugTarget,
) -> SplitView {
    let node = match &resolved.shape {
        ResolvedShape::Root(root) => {
            let ty = TypeKey::parse(&root.type_key).ok();
            let base = parse_key(&root.key)
                .map_or_else(|| root.key.clone(), |parsed| parsed.path.to_owned());
            contiguous_view_node(
                source.manifest,
                ty.as_ref(),
                &root.type_key,
                root.layout_hash,
                &typed.value,
                Some(view_root(root, None)),
                resolved.atomic,
                &base,
            )
        }
        ResolvedShape::Aggregate(aggregate) => {
            aggregate_view_node(source.manifest, aggregate, &typed.value)
        }
    };
    SplitView {
        target: view_target(target),
        resolved_key: resolved.canonical_key.clone(),
        generation: source.generation,
        build_id: source.build_id,
        cycle: source.cycle,
        origin: ViewOrigin::Live,
        atomic: resolved.atomic,
        consistent: true,
        manifest_assisted: node.manifest_assisted,
        contiguous: node.contiguous,
        node,
    }
}

/// Builds a stop-snapshot view. The snapshot is one bounded contiguous copy,
/// so fields are described as contiguous snapshot data with no live roots.
#[allow(clippy::too_many_arguments)]
pub(crate) fn snapshot_view(
    manifest: &Manifest,
    typed: &TypedValue,
    target: String,
    resolved_key: String,
    generation: u64,
    build_id: GotId,
    cycle: u64,
    atomic: bool,
) -> SplitView {
    let ty = TypeKey::parse(&typed.type_key).ok();
    let layout = record_layout(manifest, ty.as_ref());
    let node = contiguous_view_node(
        manifest,
        ty.as_ref(),
        &typed.type_key,
        layout,
        &typed.value,
        None,
        atomic,
        &resolved_key,
    );
    SplitView {
        target,
        resolved_key,
        generation,
        build_id,
        cycle,
        origin: ViewOrigin::StopSnapshot,
        atomic,
        consistent: true,
        manifest_assisted: node.manifest_assisted,
        contiguous: node.contiguous,
        node,
    }
}

fn view_target(target: &DebugTarget) -> String {
    match target {
        DebugTarget::Path(path) => path.clone(),
        DebugTarget::GotId(got_id) => got_id.to_hex(),
        DebugTarget::Local { frame, path } => format!("{frame}:{path}"),
    }
}

/// Returns whether a stopped-local type is one atomic single-root copy.
pub(crate) fn type_is_single_root(manifest: &Manifest, type_key: &str) -> bool {
    TypeKey::parse(type_key).is_ok_and(|ty| is_single_root_type(manifest, &ty, 0))
}

fn view_root(root: &RootRecord, marker: Option<&str>) -> ViewRoot {
    ViewRoot {
        got_id: root.got_id,
        key: root.key.clone(),
        kind: root.root_kind,
        marker: marker.map(str::to_owned),
    }
}

fn hidden_view_roots(
    manifest: &Manifest,
    package: &str,
    path: &str,
    qualifier: Option<&str>,
) -> Vec<ViewRoot> {
    let mut roots = Vec::new();
    for marker in ["HANDLE", "OBJREC"] {
        if let Some(root) = find_hidden_root(manifest, package, path, qualifier, marker) {
            roots.push(view_root(root, Some(marker)));
        }
    }
    roots
}

fn record_layout(manifest: &Manifest, ty: Option<&TypeKey>) -> GotId {
    match ty {
        Some(TypeKey::Named(got_id)) => {
            type_by_id(manifest, *got_id).map_or(GotId::ZERO, |ty| ty.layout_hash)
        }
        _ => GotId::ZERO,
    }
}

fn element_type_layout(manifest: &Manifest, element: &TypeKey) -> GotId {
    match element {
        TypeKey::Named(got_id) => {
            type_by_id(manifest, *got_id).map_or(GotId::ZERO, |ty| ty.layout_hash)
        }
        _ => GotId::ZERO,
    }
}

fn unavailable_view_node(
    type_key: &str,
    layout_hash: GotId,
    warning: &DebugWarning,
    manifest_assisted: bool,
    root: Option<ViewRoot>,
) -> ViewNode {
    ViewNode {
        kind: ViewKind::Unavailable,
        type_key: type_key.to_owned(),
        layout_hash,
        value: DebugValue::Unavailable {
            warning: warning.clone(),
        },
        available: false,
        manifest_assisted,
        contiguous: false,
        atomic: false,
        consistent: true,
        root,
        identity: None,
        fields: Vec::new(),
        elements: Vec::new(),
    }
}

fn leaf_view_kind(value: &DebugValue) -> ViewKind {
    match value {
        DebugValue::Overlap { .. } => ViewKind::Overlap,
        DebugValue::Array(_) => ViewKind::Array,
        DebugValue::Aggregate(_) => ViewKind::Aggregate,
        DebugValue::Object {
            class, interface, ..
        } => {
            if class.is_some() {
                ViewKind::Class
            } else if interface.is_some() {
                ViewKind::Interface
            } else {
                ViewKind::FunctionBlock
            }
        }
        DebugValue::Reference { .. } => ViewKind::Reference,
        DebugValue::Unavailable { .. } => ViewKind::Unavailable,
        DebugValue::Bool(_)
        | DebugValue::Decimal(_)
        | DebugValue::Real(_)
        | DebugValue::String(_) => ViewKind::Scalar,
    }
}

fn identity_for_value(
    value: &DebugValue,
    ty: Option<&TypeKey>,
    roots: Vec<ViewRoot>,
) -> Option<ViewIdentity> {
    match value {
        DebugValue::Object {
            identity,
            present,
            class,
            interface,
            ..
        } => {
            let kind = if class.is_some() {
                IdentityKind::Class
            } else if interface.is_some() {
                IdentityKind::Interface
            } else {
                IdentityKind::FunctionBlock
            };
            Some(ViewIdentity {
                kind,
                identity: identity.clone(),
                present: *present,
                class: class.clone(),
                interface: interface.clone(),
                target_type: None,
                roots,
            })
        }
        DebugValue::Reference {
            identity,
            present,
            target_type,
        } => Some(ViewIdentity {
            kind: if matches!(ty, Some(TypeKey::PtrTo(_))) {
                IdentityKind::Pointer
            } else {
                IdentityKind::Reference
            },
            identity: identity.clone(),
            present: *present,
            class: None,
            interface: None,
            target_type: target_type.clone(),
            roots,
        }),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn contiguous_view_node(
    manifest: &Manifest,
    ty: Option<&TypeKey>,
    type_key: &str,
    layout_hash: GotId,
    value: &DebugValue,
    root: Option<ViewRoot>,
    atomic: bool,
    base_path: &str,
) -> ViewNode {
    if let DebugValue::Unavailable { warning } = value {
        return unavailable_view_node(type_key, layout_hash, warning, false, None);
    }
    let record = ty.and_then(|ty| match ty {
        TypeKey::Named(got_id) => type_by_id(manifest, *got_id),
        _ => None,
    });
    let kind = match value {
        DebugValue::Aggregate(_) => match record.map(|record| record.type_kind) {
            Some(TypeKind::FunctionBlock) => ViewKind::FunctionBlock,
            Some(TypeKind::Class) => ViewKind::Class,
            Some(TypeKind::Interface) => ViewKind::Interface,
            _ => ViewKind::Aggregate,
        },
        other => leaf_view_kind(other),
    };
    let identity = identity_for_value(value, ty, Vec::new());
    let mut fields = Vec::new();
    let mut elements = Vec::new();
    match value {
        DebugValue::Aggregate(values) | DebugValue::Object { fields: values, .. } => {
            fields = contiguous_fields(manifest, record, base_path, values, atomic);
        }
        DebugValue::Array(values) => {
            let element = match ty {
                Some(TypeKey::Array { element, .. }) => Some(element.as_ref()),
                _ => None,
            };
            let element_type_key = element.map_or_else(String::new, TypeKey::render);
            let element_layout = element.map_or(GotId::ZERO, |element| {
                element_type_layout(manifest, element)
            });
            for (index, value) in values.iter().enumerate() {
                let child_path = format!("{base_path}.E{index}");
                let child_type = if value.type_key == element_type_key {
                    element_type_key.as_str()
                } else {
                    value.type_key.as_str()
                };
                let child_layout = if child_type == element_type_key {
                    element_layout
                } else {
                    GotId::ZERO
                };
                elements.push(contiguous_view_node(
                    manifest,
                    element,
                    child_type,
                    child_layout,
                    &value.value,
                    None,
                    atomic,
                    &child_path,
                ));
            }
        }
        _ => {}
    }
    ViewNode {
        kind,
        type_key: type_key.to_owned(),
        layout_hash,
        value: value.clone(),
        available: true,
        manifest_assisted: false,
        contiguous: true,
        atomic,
        consistent: true,
        root,
        identity,
        fields,
        elements,
    }
}

fn contiguous_fields(
    manifest: &Manifest,
    record: Option<&TypeRecord>,
    base_path: &str,
    values: &[NamedValue],
    atomic: bool,
) -> Vec<ViewField> {
    let mut fields = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let declared = record.and_then(|record| {
            record
                .fields
                .iter()
                .find(|field| field.path == value.name)
                .or_else(|| record.fields.get(index))
        });
        let type_key = declared.map_or(value.value.type_key.as_str(), |field| {
            field.type_key.as_str()
        });
        let layout_hash = declared.map_or(GotId::ZERO, |field| field.layout_hash);
        let path = format!("{base_path}.{}", value.name);
        let child_ty = TypeKey::parse(type_key).ok();
        let view = contiguous_view_node(
            manifest,
            child_ty.as_ref(),
            type_key,
            layout_hash,
            &value.value.value,
            None,
            atomic,
            &path,
        );
        fields.push(ViewField {
            name: value.name.clone(),
            path,
            view,
        });
    }
    fields
}

fn aggregate_view_node(
    manifest: &Manifest,
    aggregate: &AggregateTarget,
    value: &DebugValue,
) -> ViewNode {
    match &aggregate.kind {
        AggregateTargetKind::Record { ty } => split_record_view_node(
            manifest,
            &aggregate.package,
            &aggregate.path,
            aggregate.qualifier.as_deref(),
            ty,
            value,
        ),
        AggregateTargetKind::Program {
            fields,
            shape_key,
            layout_hash,
        } => {
            if let DebugValue::Unavailable { warning } = value {
                return unavailable_view_node(shape_key, *layout_hash, warning, true, None);
            }
            let values: &[NamedValue] = match value {
                DebugValue::Aggregate(values) => values,
                _ => &[],
            };
            let declared: Vec<FieldDecl<'_>> = fields
                .iter()
                .map(|field| FieldDecl {
                    path: &field.path,
                    type_key: &field.type_key,
                    layout_hash: field.layout_hash,
                })
                .collect();
            let fields = split_fields(
                manifest,
                &aggregate.package,
                &aggregate.path,
                aggregate.qualifier.as_deref(),
                &declared,
                values,
            );
            ViewNode {
                kind: ViewKind::Program,
                type_key: shape_key.clone(),
                layout_hash: *layout_hash,
                value: value.clone(),
                available: true,
                manifest_assisted: true,
                contiguous: false,
                atomic: false,
                consistent: true,
                root: None,
                identity: None,
                fields,
                elements: Vec::new(),
            }
        }
        AggregateTargetKind::Array { ty, layout_hash } => split_array_view_node(
            manifest,
            &aggregate.package,
            &aggregate.path,
            aggregate.qualifier.as_deref(),
            ty,
            *layout_hash,
            value,
        ),
    }
}

#[derive(Clone, Copy)]
struct FieldDecl<'a> {
    path: &'a str,
    type_key: &'a str,
    layout_hash: GotId,
}

fn split_record_view_node(
    manifest: &Manifest,
    package: &str,
    path: &str,
    qualifier: Option<&str>,
    record: &TypeRecord,
    value: &DebugValue,
) -> ViewNode {
    if let DebugValue::Unavailable { warning } = value {
        return unavailable_view_node(&record.type_key, record.layout_hash, warning, true, None);
    }
    let values: &[NamedValue] = match value {
        DebugValue::Aggregate(values) | DebugValue::Object { fields: values, .. } => values,
        _ => &[],
    };
    let declared: Vec<FieldDecl<'_>> = record
        .fields
        .iter()
        .map(|field| FieldDecl {
            path: &field.path,
            type_key: &field.type_key,
            layout_hash: field.layout_hash,
        })
        .collect();
    let fields = split_fields(manifest, package, path, qualifier, &declared, values);
    let kind = match record.type_kind {
        TypeKind::Struct => ViewKind::Aggregate,
        TypeKind::FunctionBlock => ViewKind::FunctionBlock,
        TypeKind::Class => ViewKind::Class,
        TypeKind::Interface => ViewKind::Interface,
        TypeKind::Enum | TypeKind::Subrange | TypeKind::Alias => ViewKind::Scalar,
    };
    let identity = match record.type_kind {
        TypeKind::Struct => None,
        TypeKind::FunctionBlock => Some(ViewIdentity {
            kind: IdentityKind::FunctionBlock,
            identity: None,
            present: true,
            class: None,
            interface: None,
            target_type: None,
            roots: hidden_view_roots(manifest, package, path, qualifier),
        }),
        TypeKind::Class => {
            let (identity, present, roots) = match value {
                DebugValue::Object {
                    identity, present, ..
                } => (
                    identity.clone(),
                    *present,
                    hidden_view_roots(manifest, package, path, qualifier),
                ),
                _ => (
                    None,
                    false,
                    hidden_view_roots(manifest, package, path, qualifier),
                ),
            };
            Some(ViewIdentity {
                kind: IdentityKind::Class,
                identity,
                present,
                class: Some(record.type_key.clone()),
                interface: None,
                target_type: None,
                roots,
            })
        }
        TypeKind::Interface => {
            let (identity, present) = match value {
                DebugValue::Object {
                    identity, present, ..
                } => (identity.clone(), *present),
                _ => (None, false),
            };
            Some(ViewIdentity {
                kind: IdentityKind::Interface,
                identity,
                present,
                class: None,
                interface: Some(record.type_key.clone()),
                target_type: None,
                roots: Vec::new(),
            })
        }
        TypeKind::Enum | TypeKind::Subrange | TypeKind::Alias => None,
    };
    ViewNode {
        kind,
        type_key: record.type_key.clone(),
        layout_hash: record.layout_hash,
        value: value.clone(),
        available: true,
        manifest_assisted: true,
        contiguous: false,
        atomic: false,
        consistent: true,
        root: None,
        identity,
        fields,
        elements: Vec::new(),
    }
}

fn split_fields(
    manifest: &Manifest,
    package: &str,
    base_path: &str,
    qualifier: Option<&str>,
    declared: &[FieldDecl<'_>],
    values: &[NamedValue],
) -> Vec<ViewField> {
    let mut fields = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let decl = declared
            .get(index)
            .filter(|field| field.path == value.name)
            .or_else(|| declared.iter().find(|field| field.path == value.name));
        let (name, type_key, layout_hash) = decl.map_or(
            (
                value.name.as_str(),
                value.value.type_key.as_str(),
                GotId::ZERO,
            ),
            |field| (field.path, field.type_key, field.layout_hash),
        );
        let path = format!("{base_path}.{name}");
        let root = find_field_root(manifest, package, &path, qualifier);
        let node = if let Some(root) = root {
            let root_ty = TypeKey::parse(&root.type_key).ok();
            let atomic = root_ty
                .as_ref()
                .is_some_and(|ty| is_single_root_type(manifest, ty, 0));
            contiguous_view_node(
                manifest,
                root_ty.as_ref(),
                &root.type_key,
                root.layout_hash,
                &value.value.value,
                Some(view_root(root, None)),
                atomic,
                &path,
            )
        } else {
            let ty = TypeKey::parse(type_key).ok();
            split_view_node(
                manifest,
                package,
                &path,
                qualifier,
                ty.as_ref(),
                type_key,
                layout_hash,
                &value.value.value,
            )
        };
        fields.push(ViewField {
            name: name.to_owned(),
            path,
            view: node,
        });
    }
    fields
}

#[allow(clippy::too_many_arguments)]
fn split_view_node(
    manifest: &Manifest,
    package: &str,
    path: &str,
    qualifier: Option<&str>,
    ty: Option<&TypeKey>,
    type_key: &str,
    layout_hash: GotId,
    value: &DebugValue,
) -> ViewNode {
    if let DebugValue::Unavailable { warning } = value {
        return unavailable_view_node(type_key, layout_hash, warning, true, None);
    }
    match ty {
        Some(TypeKey::Named(got_id)) => match type_by_id(manifest, *got_id) {
            Some(record)
                if matches!(
                    record.type_kind,
                    TypeKind::Struct
                        | TypeKind::FunctionBlock
                        | TypeKind::Class
                        | TypeKind::Interface
                ) =>
            {
                split_record_view_node(manifest, package, path, qualifier, record, value)
            }
            _ => split_leaf_view_node(type_key, layout_hash, ty, value),
        },
        Some(array @ TypeKey::Array { .. }) => split_array_view_node(
            manifest,
            package,
            path,
            qualifier,
            array,
            layout_hash,
            value,
        ),
        _ => split_leaf_view_node(type_key, layout_hash, ty, value),
    }
}

fn split_leaf_view_node(
    type_key: &str,
    layout_hash: GotId,
    ty: Option<&TypeKey>,
    value: &DebugValue,
) -> ViewNode {
    ViewNode {
        kind: leaf_view_kind(value),
        type_key: type_key.to_owned(),
        layout_hash,
        value: value.clone(),
        available: true,
        manifest_assisted: true,
        contiguous: false,
        atomic: false,
        consistent: true,
        root: None,
        identity: identity_for_value(value, ty, Vec::new()),
        fields: Vec::new(),
        elements: Vec::new(),
    }
}

fn split_array_view_node(
    manifest: &Manifest,
    package: &str,
    base_path: &str,
    qualifier: Option<&str>,
    ty: &TypeKey,
    layout_hash: GotId,
    value: &DebugValue,
) -> ViewNode {
    let type_key = ty.render();
    if let DebugValue::Unavailable { warning } = value {
        return unavailable_view_node(&type_key, layout_hash, warning, true, None);
    }
    let TypeKey::Array { element, .. } = ty else {
        return split_leaf_view_node(&type_key, layout_hash, Some(ty), value);
    };
    let element = element.as_ref();
    let values = match value {
        DebugValue::Array(values) => values.as_slice(),
        _ => &[],
    };
    let element_type_key = element.render();
    let element_layout = element_type_layout(manifest, element);
    let mut elements = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let child_path = format!("{base_path}.E{index}");
        if let Some(root) = find_field_root(manifest, package, &child_path, qualifier) {
            let root_ty = TypeKey::parse(&root.type_key).ok();
            let atomic = root_ty
                .as_ref()
                .is_some_and(|ty| is_single_root_type(manifest, ty, 0));
            elements.push(contiguous_view_node(
                manifest,
                root_ty.as_ref(),
                &root.type_key,
                root.layout_hash,
                &value.value,
                Some(view_root(root, None)),
                atomic,
                &child_path,
            ));
        } else {
            elements.push(split_view_node(
                manifest,
                package,
                &child_path,
                qualifier,
                Some(element),
                &element_type_key,
                element_layout,
                &value.value,
            ));
        }
    }
    ViewNode {
        kind: ViewKind::Array,
        type_key,
        layout_hash,
        value: value.clone(),
        available: true,
        manifest_assisted: true,
        contiguous: false,
        atomic: false,
        consistent: true,
        root: None,
        identity: None,
        fields: Vec::new(),
        elements,
    }
}

#[allow(clippy::too_many_arguments)]
fn revalidate_prepared<'a>(
    prepared: &PreparedWrite,
    manifest: &'a Manifest,
    target: &Target,
    info: Option<&crate::ModuleInfo>,
    generation: u64,
    build: GotId,
) -> Result<Option<&'a RootRecord>> {
    if prepared.generation != generation
        || prepared
            .expected_generation
            .is_some_and(|expected| expected != generation)
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "queued {} for root `{}` belongs to generation {}, not the active generation {generation}",
                prepared.kind.name(),
                prepared.target_key,
                prepared.generation
            ),
        ));
    }
    if prepared.build_id != build
        || prepared
            .expected_build_id
            .is_some_and(|expected| expected != build)
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "queued {} for root `{}` belongs to a different build",
                prepared.kind.name(),
                prepared.target_key
            ),
        ));
    }
    let Some(root) = manifest
        .roots
        .iter()
        .find(|root| root.got_id == prepared.got_id)
    else {
        return Ok(None);
    };
    if root.key != prepared.target_key
        || root.type_key != prepared.type_key
        || root.layout_hash != prepared.layout_hash
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "queued {} target `{}` was removed or its type/layout changed; it is never retargeted",
                prepared.kind.name(),
                prepared.target_key
            ),
        ));
    }
    if prepared
        .expected_type_key
        .as_deref()
        .is_some_and(|expected| expected != root.type_key)
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "root `{}` no longer has the expected type",
                prepared.target_key
            ),
        ));
    }
    if prepared
        .expected_layout_hash
        .is_some_and(|expected| expected != root.layout_hash)
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "root `{}` no longer has the expected layout",
                prepared.target_key
            ),
        ));
    }
    if root.root_kind == RootKind::TaskGuard {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!("task-guard root `{}` is not writable", root.key),
        ));
    }
    let ty = TypeKey::parse(&root.type_key).map_err(|error| {
        Error::new(
            codes::DEBUG_TYPE,
            format!(
                "root `{}` has an invalid type key: {}",
                root.key, error.message
            ),
        )
    })?;
    validate_writable_type(manifest, &ty, 0)?;
    let layout = type_layout(manifest, target, &ty)?;
    if layout.size != root.size || u64::try_from(prepared.bytes.len()).ok() != Some(root.size) {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "root `{}` storage changed shape before the write",
                prepared.target_key
            ),
        ));
    }
    if root.root_kind == RootKind::Direct {
        let Some(info) = info else {
            return Err(Error::new(
                codes::DEBUG_STALE,
                format!(
                    "direct root `{}` has no active process-image metadata",
                    prepared.target_key
                ),
            ));
        };
        validate_direct_binding(info, root)?;
    }
    Ok(Some(root))
}

pub(crate) fn prepare_write(
    source: &DebugSource<'_>,
    request: &DebugWriteRequest,
    kind: DebugWriteKind,
) -> Result<PreparedWrite> {
    if matches!(request.target, DebugTarget::Local { .. }) {
        return Err(Error::new(
            codes::DEBUG_UNKNOWN,
            "locals and parameters are not writable debug targets",
        ));
    }
    let resolved = resolve_target(source.manifest, &request.target)?;
    let ResolvedShape::Root(root) = resolved.shape else {
        return Err(Error::new(
            codes::DEBUG_UNKNOWN,
            "only a root scalar, string, enum, subrange, or single-root array is writable",
        ));
    };
    if root.root_kind == RootKind::TaskGuard {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!("task-guard root `{}` is not writable", root.key),
        ));
    }
    if root.root_kind != RootKind::Direct && source.roots.get(root.got_id).is_none() {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!("root `{}` has no active storage", root.key),
        ));
    }
    if request
        .expected_generation
        .is_some_and(|generation| generation != source.generation)
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "target `{}` is from generation {}, not the active generation {}",
                resolved.canonical_key,
                request.expected_generation.unwrap_or(source.generation),
                source.generation
            ),
        ));
    }
    if request
        .expected_build_id
        .is_some_and(|build| build != source.build_id)
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "target `{}` is from a different build",
                resolved.canonical_key
            ),
        ));
    }
    if request
        .expected_type_key
        .as_deref()
        .is_some_and(|type_key| type_key != root.type_key)
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "target `{}` has type `{}`, not the expected type",
                root.key, root.type_key
            ),
        ));
    }
    if request
        .expected_layout_hash
        .is_some_and(|layout| layout != root.layout_hash)
    {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!("target `{}` has a different layout", root.key),
        ));
    }
    if request.value.type_key != root.type_key {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!(
                "value type `{}` does not match target type `{}`; no implicit conversion is performed",
                request.value.type_key, root.type_key
            ),
        ));
    }
    let ty = TypeKey::parse(&root.type_key).map_err(|error| {
        Error::new(
            codes::DEBUG_TYPE,
            format!(
                "target `{}` has an invalid type key: {}",
                root.key, error.message
            ),
        )
    })?;
    validate_writable_type(source.manifest, &ty, 0)?;
    let layout = type_layout(source.manifest, source.target, &ty)?;
    if layout.size != root.size {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!(
                "target `{}` has {} bytes but its type layout is {} bytes",
                root.key, root.size, layout.size
            ),
        ));
    }
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
        validate_direct_binding(info, &root)?;
    }
    let bytes = encode_type_value(
        source.manifest,
        source.target,
        &ty,
        &root.type_key,
        &request.value,
        layout.size,
    )?;
    if u64::try_from(bytes.len()).ok() != Some(root.size) {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!("encoded target `{}` has the wrong size", root.key),
        ));
    }
    let warning =
        (kind == DebugWriteKind::Force && root.retention == Retention::Retain).then(|| {
            DebugWarning {
                code: codes::W_DEBUG_RETAIN,
                message: format!(
                    "force on retained root `{}` is not saved to the state file",
                    root.key
                ),
            }
        });
    let queue_bytes = bytes
        .len()
        .saturating_add(root.key.len())
        .saturating_add(root.type_key.len())
        .saturating_add(128);
    Ok(PreparedWrite {
        kind,
        got_id: root.got_id,
        generation: source.generation,
        build_id: source.build_id,
        cycle: source.cycle,
        target_key: resolved.canonical_key,
        type_key: root.type_key.clone(),
        layout_hash: root.layout_hash,
        expected_generation: request.expected_generation,
        expected_build_id: request.expected_build_id,
        expected_type_key: request.expected_type_key.clone(),
        expected_layout_hash: request.expected_layout_hash,
        bytes,
        value: request.value.clone(),
        warning,
        queue_bytes,
    })
}

fn validate_writable_type(manifest: &Manifest, ty: &TypeKey, depth: usize) -> Result<()> {
    if depth > MAX_DECODE_DEPTH {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            "writable type nesting exceeds the decoder limit",
        ));
    }
    match ty {
        TypeKey::RefTo(_) | TypeKey::PtrTo(_) => Err(Error::new(
            codes::DEBUG_TYPE,
            "reference and pointer identities are not writable",
        )),
        TypeKey::Void => Err(Error::new(codes::DEBUG_TYPE, "`VOID` is not writable")),
        TypeKey::Named(got_id) => {
            let Some(record) = type_by_id(manifest, *got_id) else {
                return Err(Error::new(
                    codes::DEBUG_TYPE,
                    format!("type {} has no manifest type record", got_id.to_hex()),
                ));
            };
            match record.type_kind {
                TypeKind::Enum | TypeKind::Subrange => Ok(()),
                TypeKind::Alias => {
                    let Some(base) = record.base else {
                        return Err(Error::new(
                            codes::DEBUG_TYPE,
                            format!("alias `{}` has no base type", record.type_key),
                        ));
                    };
                    let Some(base_record) = type_by_id(manifest, base) else {
                        return Err(Error::new(
                            codes::DEBUG_TYPE,
                            format!(
                                "alias `{}` names base {} with no type record",
                                record.type_key,
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
                    validate_writable_type(manifest, &base_key, depth + 1)
                }
                TypeKind::Struct
                | TypeKind::FunctionBlock
                | TypeKind::Class
                | TypeKind::Interface => Err(Error::new(
                    codes::DEBUG_TYPE,
                    format!(
                        "aggregate type `{}` is not a writable root; write its leaves",
                        record.type_key
                    ),
                )),
            }
        }
        TypeKey::Array { element, .. } => {
            if matches!(
                element.as_ref(),
                TypeKey::Named(got_id)
                    if type_by_id(manifest, *got_id).is_some_and(|record| matches!(
                        record.type_kind,
                        TypeKind::Struct | TypeKind::FunctionBlock | TypeKind::Class | TypeKind::Interface
                    ))
            ) {
                return Err(Error::new(
                    codes::DEBUG_TYPE,
                    "arrays of aggregates are not writable; write their leaves",
                ));
            }
            validate_writable_type(manifest, element, depth + 1)
        }
        _ => Ok(()),
    }
}

fn encode_type_value(
    manifest: &Manifest,
    target: &Target,
    ty: &TypeKey,
    type_key: &str,
    value: &TypedValue,
    size: u64,
) -> Result<Vec<u8>> {
    let size = usize::try_from(size).map_err(|_| {
        Error::new(
            codes::DEBUG_LIMIT,
            format!("target `{type_key}` is too large to write"),
        )
    })?;
    let mut bytes = vec![0_u8; size];
    encode_value_into(
        manifest,
        target,
        ty,
        value,
        Some(type_key),
        &mut bytes,
        0,
        0,
    )?;
    Ok(bytes)
}

#[allow(clippy::too_many_arguments)]
fn encode_value_into(
    manifest: &Manifest,
    target: &Target,
    ty: &TypeKey,
    value: &TypedValue,
    expected_type_key: Option<&str>,
    out: &mut [u8],
    offset: usize,
    depth: usize,
) -> Result<()> {
    if depth > MAX_DECODE_DEPTH {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            "write value nesting exceeds the decoder limit",
        ));
    }
    if let Some(expected) = expected_type_key
        && value.type_key != expected
    {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!(
                "value type `{}` does not match declared type `{expected}`",
                value.type_key
            ),
        ));
    }
    let layout = type_layout(manifest, target, ty)?;
    let end = offset
        .checked_add(usize::try_from(layout.size).unwrap_or(usize::MAX))
        .ok_or_else(|| Error::new(codes::DEBUG_LIMIT, "write offset overflows"))?;
    if end > out.len() {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!("write for `{}` lies outside its storage", ty.render()),
        ));
    }
    match ty {
        TypeKey::Bool => {
            let DebugValue::Bool(value) = &value.value else {
                return Err(type_mismatch(&value.type_key, "BOOL"));
            };
            out[offset] = u8::from(*value);
            Ok(())
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
        | TypeKey::Ldt => encode_signed(
            &value.value,
            &value.type_key,
            integer_storage_size(layout.size)?,
            target.endianness,
            out,
            offset,
        ),
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
        | TypeKey::Uchar => encode_unsigned(
            &value.value,
            &value.type_key,
            integer_storage_size(layout.size)?,
            target.endianness,
            out,
            offset,
        ),
        TypeKey::Real => {
            let DebugValue::Real(value) = &value.value else {
                return Err(type_mismatch(&value.type_key, "REAL"));
            };
            write_integer(
                u128::from((*value as f32).to_bits()),
                4,
                target.endianness,
                out,
                offset,
            );
            Ok(())
        }
        TypeKey::Lreal => {
            let DebugValue::Real(value) = &value.value else {
                return Err(type_mismatch(&value.type_key, "LREAL"));
            };
            write_integer(
                u128::from(value.to_bits()),
                8,
                target.endianness,
                out,
                offset,
            );
            Ok(())
        }
        TypeKey::String { kind, capacity } => encode_string(
            &value.value,
            &value.type_key,
            *kind,
            *capacity,
            target.endianness,
            out,
            offset,
            layout.size,
        ),
        TypeKey::Array {
            lower,
            upper,
            element,
        } => encode_array(
            manifest,
            target,
            value,
            &value.type_key,
            *lower,
            *upper,
            element,
            out,
            offset,
            layout.size,
            depth,
        ),
        TypeKey::Named(got_id) => {
            let Some(record) = type_by_id(manifest, *got_id) else {
                return Err(Error::new(
                    codes::DEBUG_TYPE,
                    format!("type {} has no manifest type record", got_id.to_hex()),
                ));
            };
            match record.type_kind {
                TypeKind::Enum => {
                    encode_enum(manifest, record, value, target, out, offset, layout.size)
                }
                TypeKind::Subrange => {
                    encode_subrange(manifest, record, value, target, out, offset, layout.size)
                }
                TypeKind::Alias => {
                    let Some(base) = record.base else {
                        return Err(Error::new(
                            codes::DEBUG_TYPE,
                            format!("alias `{}` has no base type", record.type_key),
                        ));
                    };
                    let Some(base_record) = type_by_id(manifest, base) else {
                        return Err(Error::new(
                            codes::DEBUG_TYPE,
                            format!(
                                "alias `{}` names base {} with no type record",
                                record.type_key,
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
                    encode_value_into(
                        manifest,
                        target,
                        &base_key,
                        value,
                        None,
                        out,
                        offset,
                        depth + 1,
                    )
                }
                TypeKind::Struct
                | TypeKind::FunctionBlock
                | TypeKind::Class
                | TypeKind::Interface => Err(Error::new(
                    codes::DEBUG_TYPE,
                    format!(
                        "aggregate type `{}` is not a writable root",
                        record.type_key
                    ),
                )),
            }
        }
        TypeKey::RefTo(_) | TypeKey::PtrTo(_) => Err(Error::new(
            codes::DEBUG_TYPE,
            "reference and pointer identities are not writable",
        )),
        TypeKey::Void => Err(Error::new(codes::DEBUG_TYPE, "`VOID` is not writable")),
    }
}

fn integer_storage_size(size: u64) -> Result<usize> {
    match size {
        1 | 2 | 4 | 8 => usize::try_from(size)
            .map_err(|_| Error::new(codes::DEBUG_TYPE, "integer storage width is invalid")),
        _ => Err(Error::new(
            codes::DEBUG_TYPE,
            format!("`{size}`-byte integers are not supported"),
        )),
    }
}

fn type_mismatch(actual: &str, expected: &str) -> Error {
    Error::new(
        codes::DEBUG_TYPE,
        format!("value type `{actual}` is not compatible with `{expected}`"),
    )
}

fn integer_decimal<'a>(value: &'a DebugValue, type_key: &str) -> Result<&'a str> {
    let DebugValue::Decimal(text) = value else {
        return Err(type_mismatch(type_key, "an integer"));
    };
    Ok(text)
}

fn encode_signed(
    value: &DebugValue,
    type_key: &str,
    size: usize,
    endianness: stc_got::layout::Endianness,
    out: &mut [u8],
    offset: usize,
) -> Result<()> {
    let text = integer_decimal(value, type_key)?;
    let parsed = text.parse::<i128>().map_err(|_| {
        Error::new(
            codes::DEBUG_TYPE,
            format!("`{text}` is not a decimal integer for `{type_key}`"),
        )
    })?;
    let bits = size * 8;
    let minimum = -(1_i128 << (bits - 1));
    let maximum = (1_i128 << (bits - 1)) - 1;
    if parsed < minimum || parsed > maximum {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!("`{parsed}` is outside the range of `{type_key}`"),
        ));
    }
    write_integer(parsed as u128, size, endianness, out, offset);
    Ok(())
}

fn encode_unsigned(
    value: &DebugValue,
    type_key: &str,
    size: usize,
    endianness: stc_got::layout::Endianness,
    out: &mut [u8],
    offset: usize,
) -> Result<()> {
    let text = integer_decimal(value, type_key)?;
    let parsed = text.parse::<i128>().map_err(|_| {
        Error::new(
            codes::DEBUG_TYPE,
            format!("`{text}` is not a decimal integer for `{type_key}`"),
        )
    })?;
    let maximum = (1_i128 << (size * 8)) - 1;
    if parsed < 0 || parsed > maximum {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!("`{parsed}` is outside the range of `{type_key}`"),
        ));
    }
    write_integer(parsed as u128, size, endianness, out, offset);
    Ok(())
}

fn write_integer(
    value: u128,
    size: usize,
    endianness: stc_got::layout::Endianness,
    out: &mut [u8],
    offset: usize,
) {
    let raw = value.to_le_bytes();
    if endianness == stc_got::layout::Endianness::Little {
        out[offset..offset + size].copy_from_slice(&raw[..size]);
    } else {
        for index in 0..size {
            out[offset + index] = raw[size - 1 - index];
        }
    }
}

fn type_metadata(manifest: &Manifest, got_id: GotId) -> Option<&TypeMetadataRecord> {
    manifest
        .type_metadata
        .iter()
        .find(|metadata| metadata.type_id == got_id)
}

fn encode_enum(
    manifest: &Manifest,
    record: &TypeRecord,
    value: &TypedValue,
    target: &Target,
    out: &mut [u8],
    offset: usize,
    size: u64,
) -> Result<()> {
    let text = integer_decimal(&value.value, &record.type_key)?;
    let parsed = text.parse::<i128>().map_err(|_| {
        Error::new(
            codes::DEBUG_TYPE,
            format!(
                "`{text}` is not a decimal enum value for `{}`",
                record.type_key
            ),
        )
    })?;
    let Some(metadata) = type_metadata(manifest, record.got_id) else {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!(
                "enum `{}` has no member metadata; membership cannot be validated",
                record.type_key
            ),
        ));
    };
    if !metadata
        .enum_members
        .iter()
        .any(|member| member.value == parsed)
    {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!("`{parsed}` is not a member of enum `{}`", record.type_key),
        ));
    }
    encode_signed(
        &value.value,
        &record.type_key,
        integer_storage_size(size)?,
        target.endianness,
        out,
        offset,
    )
}

fn encode_subrange(
    manifest: &Manifest,
    record: &TypeRecord,
    value: &TypedValue,
    target: &Target,
    out: &mut [u8],
    offset: usize,
    size: u64,
) -> Result<()> {
    let text = integer_decimal(&value.value, &record.type_key)?;
    let parsed = text.parse::<i128>().map_err(|_| {
        Error::new(
            codes::DEBUG_TYPE,
            format!(
                "`{text}` is not a decimal subrange value for `{}`",
                record.type_key
            ),
        )
    })?;
    let Some(metadata) = type_metadata(manifest, record.got_id) else {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!(
                "subrange `{}` has no bounds metadata; range cannot be validated",
                record.type_key
            ),
        ));
    };
    let Some(bounds) = metadata.subrange else {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!(
                "subrange `{}` has no bounds metadata; range cannot be validated",
                record.type_key
            ),
        ));
    };
    if parsed < bounds.lower || parsed > bounds.upper {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!(
                "`{parsed}` is outside the bounds {}..={} of `{}`",
                bounds.lower, bounds.upper, record.type_key
            ),
        ));
    }
    encode_signed(
        &value.value,
        &record.type_key,
        integer_storage_size(size)?,
        target.endianness,
        out,
        offset,
    )
}

#[allow(clippy::too_many_arguments)]
fn encode_string(
    value: &DebugValue,
    type_key: &str,
    kind: StringKind,
    capacity: u32,
    endianness: stc_got::layout::Endianness,
    out: &mut [u8],
    offset: usize,
    size: u64,
) -> Result<()> {
    let DebugValue::String(string) = value else {
        return Err(type_mismatch(
            type_key,
            &format!("a {} string", kind.keyword()),
        ));
    };
    if string.capacity != capacity {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!(
                "string value capacity {} does not match `{type_key}` capacity {capacity}",
                string.capacity
            ),
        ));
    }
    let capacity = usize::try_from(capacity).map_err(|_| {
        Error::new(
            codes::DEBUG_LIMIT,
            format!("`{type_key}` capacity is too large"),
        )
    })?;
    let unit_size = usize::from(kind.unit_bytes());
    let needed = 4_usize
        .checked_add(capacity.saturating_mul(unit_size))
        .ok_or_else(|| Error::new(codes::DEBUG_LIMIT, format!("`{type_key}` size overflows")))?;
    let storage = usize::try_from(size)
        .map_err(|_| Error::new(codes::DEBUG_LIMIT, format!("`{type_key}` is too large")))?;
    if needed > storage {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!("`{type_key}` layout is shorter than its string header"),
        ));
    }
    if string.units.len() > capacity {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!(
                "string value has {} units but `{type_key}` capacity is {capacity}",
                string.units.len()
            ),
        ));
    }
    let length = i32::try_from(string.units.len()).map_err(|_| {
        Error::new(
            codes::DEBUG_TYPE,
            format!("string value for `{type_key}` is too long"),
        )
    })?;
    write_integer(u128::from(length as u32), 4, endianness, out, offset);
    for (index, unit) in string.units.iter().enumerate() {
        let valid = match kind {
            StringKind::Sbcs => *unit <= u32::from(u8::MAX),
            StringKind::Dbcs => *unit <= u32::from(u16::MAX),
            StringKind::Unicode => *unit <= 0x10_FFFF && !(0xD800..=0xDFFF).contains(unit),
        };
        if !valid {
            return Err(Error::new(
                codes::DEBUG_TYPE,
                format!(
                    "string unit {unit:#x} is not representable in `{}`",
                    type_key
                ),
            ));
        }
        let unit_offset = offset + 4 + index * unit_size;
        write_integer(u128::from(*unit), unit_size, endianness, out, unit_offset);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn encode_array(
    manifest: &Manifest,
    target: &Target,
    value: &TypedValue,
    type_key: &str,
    lower: ArrayBound,
    upper: ArrayBound,
    element: &TypeKey,
    out: &mut [u8],
    offset: usize,
    size: u64,
    depth: usize,
) -> Result<()> {
    let (ArrayBound::Value(lower), ArrayBound::Value(upper)) = (lower, upper) else {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!("`{type_key}` has a conformant bound and is not writable"),
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
    let DebugValue::Array(values) = &value.value else {
        return Err(type_mismatch(type_key, "an array"));
    };
    if values.len() != usize::try_from(count).unwrap_or(usize::MAX) {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            format!(
                "array value has {} elements but `{type_key}` has {count}",
                values.len()
            ),
        ));
    }
    let element_layout = type_layout(manifest, target, element)?;
    let stride = round_up(element_layout.size, u64::from(element_layout.align));
    let storage = usize::try_from(size)
        .map_err(|_| Error::new(codes::DEBUG_LIMIT, format!("`{type_key}` is too large")))?;
    let element_key = element.render();
    for (index, element_value) in values.iter().enumerate() {
        let element_offset = u128::try_from(index)
            .ok()
            .and_then(|index| index.checked_mul(u128::from(stride)))
            .and_then(|offset| usize::try_from(offset).ok())
            .ok_or_else(|| {
                Error::new(codes::DEBUG_LIMIT, format!("`{type_key}` offset overflows"))
            })?;
        if element_offset > storage {
            return Err(Error::new(
                codes::DEBUG_TYPE,
                format!("`{type_key}` element {index} lies outside its storage"),
            ));
        }
        encode_value_into(
            manifest,
            target,
            element,
            element_value,
            Some(&element_key),
            out,
            offset + element_offset,
            depth + 1,
        )?;
    }
    Ok(())
}

fn validate_direct_binding<'a>(
    info: &'a crate::ModuleInfo,
    root: &RootRecord,
) -> Result<&'a crate::module_info::DirectBinding> {
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
    if binding.width_bits == 1 {
        if root.size != 1 {
            return Err(Error::new(
                codes::DEBUG_TYPE,
                format!(
                    "one-bit direct root `{}` has a {}-byte value",
                    root.key, root.size
                ),
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
    } else {
        if width_bytes != root.size {
            return Err(Error::new(
                codes::DEBUG_TYPE,
                format!(
                    "direct root `{}` has {} bytes but its binding is {width_bytes} bytes",
                    root.key, root.size
                ),
            ));
        }
        if binding.width_bits % 8 != 0 {
            return Err(Error::new(
                codes::DEBUG_TYPE,
                format!(
                    "direct root `{}` has a non-byte-aligned multi-byte binding",
                    root.key
                ),
            ));
        }
        if byte_offset
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
    }
    Ok(binding)
}

fn write_root_bytes(
    info: Option<&crate::ModuleInfo>,
    roots: &mut crate::RootRegistry,
    root: &RootRecord,
    bytes: &[u8],
) -> Result<()> {
    if root.root_kind == RootKind::Direct {
        let Some(info) = info else {
            return Err(Error::new(
                codes::DEBUG_STALE,
                format!(
                    "direct root `{}` has no active process-image metadata",
                    root.key
                ),
            ));
        };
        let binding = validate_direct_binding(info, root)?;
        let Some(layout) = info.image_layout().area(binding.area) else {
            return Err(Error::new(
                codes::DEBUG_STALE,
                format!("direct root `{}` names an invalid image area", root.key),
            ));
        };
        let byte_offset = binding.offset_bits / 8;
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
        if binding.width_bits == 1 {
            let mut byte = [0_u8; 1];
            if crate::image_read(absolute, &mut byte) != 1 {
                return Err(Error::new(
                    codes::DEBUG_STALE,
                    format!("direct root `{}` is outside the process image", root.key),
                ));
            }
            let bit = u8::try_from(binding.offset_bits % 8).unwrap_or(0);
            let mask = 1_u8 << bit;
            if bytes.first().is_some_and(|value| *value != 0) {
                byte[0] |= mask;
            } else {
                byte[0] &= !mask;
            }
            if !crate::image_write(absolute, &byte) {
                return Err(Error::new(
                    codes::DEBUG_STALE,
                    format!("direct root `{}` is outside the process image", root.key),
                ));
            }
        } else if !crate::image_write(absolute, bytes) {
            return Err(Error::new(
                codes::DEBUG_STALE,
                format!("direct root `{}` is outside the process image", root.key),
            ));
        }
        return Ok(());
    }
    let Some(active) = roots.get_mut(root.got_id) else {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!("root `{}` has no active storage", root.key),
        ));
    };
    if active.storage().len() != bytes.len() {
        return Err(Error::new(
            codes::DEBUG_STALE,
            format!(
                "root `{}` storage is {} bytes but the queued value is {} bytes",
                root.key,
                active.storage().len(),
                bytes.len()
            ),
        ));
    }
    active.storage_mut().copy_from_slice(bytes);
    Ok(())
}

pub(crate) fn resolve_root_id(source: &DebugSource<'_>, target: &DebugTarget) -> Result<GotId> {
    if matches!(target, DebugTarget::Local { .. }) {
        return Err(Error::new(
            codes::DEBUG_UNKNOWN,
            "locals and parameters cannot carry a force",
        ));
    }
    let resolved = resolve_target(source.manifest, target)?;
    match resolved.shape {
        ResolvedShape::Root(root) => Ok(root.got_id),
        ResolvedShape::Aggregate(_) => Err(Error::new(
            codes::DEBUG_UNKNOWN,
            "aggregates cannot carry a force; target one of their writable roots",
        )),
    }
}

pub(crate) fn typed_value_json(value: &TypedValue) -> Json {
    Json::object([
        ("type".into(), Json::string(value.type_key.clone())),
        ("value".into(), debug_value_json(&value.value)),
    ])
}

/// Encodes a REAL per Debug and Monitoring §15.5: finite values are JSON
/// numbers and non-finite values use the canonical `NaN`/`INF`/`-INF` strings.
pub(crate) fn real_value_json(value: f64) -> Json {
    if value.is_nan() {
        return Json::string("NaN");
    }
    if value == f64::INFINITY {
        return Json::string("INF");
    }
    if value == f64::NEG_INFINITY {
        return Json::string("-INF");
    }
    let token = if value == 0.0 {
        "0".to_owned()
    } else {
        value.to_string()
    };
    Json::NumberLiteral(token)
}

pub(crate) fn debug_value_json(value: &DebugValue) -> Json {
    match value {
        DebugValue::Bool(value) => Json::Bool(*value),
        DebugValue::Decimal(text) => Json::string(text.clone()),
        DebugValue::Real(value) => real_value_json(*value),
        DebugValue::String(string) => Json::object([
            ("length".into(), Json::unsigned(u64::from(string.length))),
            (
                "units".into(),
                Json::Array(
                    string
                        .units
                        .iter()
                        .map(|unit| Json::unsigned(u64::from(*unit)))
                        .collect(),
                ),
            ),
        ]),
        DebugValue::Array(values) => Json::object([(
            "elements".into(),
            Json::Array(values.iter().map(typed_value_json).collect()),
        )]),
        DebugValue::Aggregate(fields) => Json::object([
            ("aggregate".into(), Json::Bool(true)),
            ("atomic".into(), Json::Bool(false)),
            (
                "fields".into(),
                Json::Array(
                    fields
                        .iter()
                        .map(|field| {
                            Json::object([
                                ("name".into(), Json::string(field.name.clone())),
                                ("value".into(), typed_value_json(&field.value)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
        DebugValue::Object {
            identity,
            class,
            interface,
            fields,
            ..
        } => Json::object([
            (
                "class".into(),
                class
                    .as_ref()
                    .map_or(Json::Null, |text| Json::string(text.clone())),
            ),
            (
                "fields".into(),
                Json::Array(
                    fields
                        .iter()
                        .map(|field| {
                            Json::object([
                                ("name".into(), Json::string(field.name.clone())),
                                ("value".into(), typed_value_json(&field.value)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "identity".into(),
                identity
                    .as_ref()
                    .map_or(Json::Null, |id| Json::string(id.clone())),
            ),
            (
                "interface".into(),
                interface
                    .as_ref()
                    .map_or(Json::Null, |text| Json::string(text.clone())),
            ),
        ]),
        DebugValue::Reference {
            identity,
            present,
            target_type,
        } => Json::object([
            (
                "identity".into(),
                identity
                    .as_ref()
                    .map_or(Json::Null, |id| Json::string(id.clone())),
            ),
            ("present".into(), Json::Bool(*present)),
            (
                "target_type".into(),
                target_type
                    .as_ref()
                    .map_or(Json::Null, |text| Json::string(text.clone())),
            ),
        ]),
        DebugValue::Overlap { base64, .. } => {
            Json::object([("bytes".into(), Json::string(base64.clone()))])
        }
        DebugValue::Unavailable { warning } => Json::object([
            ("available".into(), Json::Bool(false)),
            ("code".into(), Json::string(warning.code)),
        ]),
    }
}

/// Decodes the §15.5 typed value envelope supplied by a write or force.
pub(crate) fn typed_value_from_json(value: &Json) -> Result<TypedValue> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::new(codes::DEBUG_REQUEST, "a typed value must be a JSON object"))?;
    require_keys(object, &["type", "value"], "typed value")?;
    require_allowed_keys(object, &["type", "value"], "typed value")?;
    let type_key = object
        .get("type")
        .and_then(Json::as_str)
        .ok_or_else(|| Error::new(codes::DEBUG_REQUEST, "the typed value has no string type"))?;
    let value = object
        .get("value")
        .ok_or_else(|| Error::new(codes::DEBUG_REQUEST, "the typed value has no value"))?;
    Ok(TypedValue::new(type_key, debug_value_from_json(value, 0)?))
}

/// Decodes a §15.5 debug value from its wire JSON shape.
pub(crate) fn debug_value_from_json(value: &Json, depth: usize) -> Result<DebugValue> {
    if depth > MAX_DECODE_DEPTH {
        return Err(Error::new(
            codes::DEBUG_TYPE,
            "the typed value nesting depth exceeds the limit",
        ));
    }
    match value {
        Json::Bool(value) => Ok(DebugValue::Bool(*value)),
        Json::String(text) if text == "NaN" => Ok(DebugValue::Real(f64::NAN)),
        Json::String(text) if text == "INF" => Ok(DebugValue::Real(f64::INFINITY)),
        Json::String(text) if text == "-INF" => Ok(DebugValue::Real(f64::NEG_INFINITY)),
        Json::String(text) => Ok(DebugValue::Decimal(text.clone())),
        Json::Number(value) => Ok(DebugValue::Real(*value as f64)),
        Json::NumberLiteral(text) => {
            let parsed = text.parse::<f64>().map_err(|_| {
                Error::new(
                    codes::DEBUG_TYPE,
                    format!("`{text}` is not a finite REAL value"),
                )
            })?;
            if !parsed.is_finite() {
                return Err(Error::new(
                    codes::DEBUG_TYPE,
                    format!("`{text}` is not a finite REAL value"),
                ));
            }
            Ok(DebugValue::Real(parsed))
        }
        Json::Object(object) => {
            if object.contains_key("units") {
                require_keys(object, &["length", "units"], "string value")?;
                let length = object
                    .get("length")
                    .and_then(Json::as_number)
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| {
                        Error::new(codes::DEBUG_REQUEST, "the string length is not a u32")
                    })?;
                let units = object
                    .get("units")
                    .and_then(Json::as_array)
                    .ok_or_else(|| {
                        Error::new(codes::DEBUG_REQUEST, "the string units are missing")
                    })?
                    .iter()
                    .map(|unit| {
                        unit.as_number()
                            .and_then(|value| u32::try_from(value).ok())
                            .ok_or_else(|| {
                                Error::new(
                                    codes::DEBUG_REQUEST,
                                    "a string unit is not a valid code point",
                                )
                            })
                    })
                    .collect::<Result<Vec<u32>>>()?;
                let text = units
                    .iter()
                    .map(|unit| char::from_u32(*unit))
                    .collect::<Option<String>>();
                return Ok(DebugValue::String(StringValue {
                    capacity: length,
                    length: u32::try_from(units.len()).unwrap_or(u32::MAX),
                    units,
                    text,
                }));
            }
            if object.contains_key("elements") {
                require_keys(object, &["elements"], "array value")?;
                let elements =
                    object
                        .get("elements")
                        .and_then(Json::as_array)
                        .ok_or_else(|| {
                            Error::new(codes::DEBUG_REQUEST, "the array elements are missing")
                        })?;
                return Ok(DebugValue::Array(
                    elements
                        .iter()
                        .map(typed_value_from_json)
                        .collect::<Result<Vec<_>>>()?,
                ));
            }
            if object.contains_key("aggregate") {
                require_keys(
                    object,
                    &["aggregate", "atomic", "fields"],
                    "aggregate value",
                )?;
                let fields = parse_named_values(object.get("fields"))?;
                return Ok(DebugValue::Aggregate(fields));
            }
            if object.contains_key("bytes") && object.len() == 1 {
                return Err(Error::new(
                    codes::DEBUG_TYPE,
                    "OVERLAP values are read-only and cannot be written",
                ));
            }
            if object.get("available").and_then(Json::as_bool) == Some(false) {
                return Err(Error::new(
                    codes::DEBUG_TYPE,
                    "an unavailable value cannot be written",
                ));
            }
            if object.contains_key("present") && object.contains_key("identity") {
                let allowed = ["identity", "present", "target_type"];
                require_allowed_keys(object, &allowed, "reference value")?;
                let identity = optional_string(object.get("identity"), "identity")?;
                let present = object
                    .get("present")
                    .and_then(Json::as_bool)
                    .ok_or_else(|| {
                        Error::new(codes::DEBUG_REQUEST, "the reference presence is missing")
                    })?;
                let target_type = optional_string(object.get("target_type"), "target_type")?;
                return Ok(DebugValue::Reference {
                    identity,
                    present,
                    target_type,
                });
            }
            if object.contains_key("class")
                || object.contains_key("interface")
                || object.contains_key("fields")
            {
                let allowed = ["class", "fields", "identity", "interface"];
                require_allowed_keys(object, &allowed, "object value")?;
                return Ok(DebugValue::Object {
                    identity: optional_string(object.get("identity"), "identity")?,
                    present: true,
                    class: optional_string(object.get("class"), "class")?,
                    interface: optional_string(object.get("interface"), "interface")?,
                    fields: parse_named_values(object.get("fields"))?,
                });
            }
            Err(Error::new(
                codes::DEBUG_REQUEST,
                "the value object does not match a typed value shape",
            ))
        }
        _ => Err(Error::new(
            codes::DEBUG_REQUEST,
            "the typed value is not a scalar, string, array, aggregate, object, or reference",
        )),
    }
}

fn parse_named_values(value: Option<&Json>) -> Result<Vec<NamedValue>> {
    let array = value
        .and_then(Json::as_array)
        .ok_or_else(|| Error::new(codes::DEBUG_REQUEST, "the named value list is missing"))?;
    array
        .iter()
        .map(|entry| {
            let object = entry.as_object().ok_or_else(|| {
                Error::new(codes::DEBUG_REQUEST, "a named value entry is not an object")
            })?;
            require_keys(object, &["name", "value"], "named value")?;
            let name = object
                .get("name")
                .and_then(Json::as_str)
                .ok_or_else(|| Error::new(codes::DEBUG_REQUEST, "a named value has no name"))?;
            Ok(NamedValue {
                name: name.to_owned(),
                value: typed_value_from_json(object.get("value").ok_or_else(|| {
                    Error::new(codes::DEBUG_REQUEST, "a named value has no value")
                })?)?,
            })
        })
        .collect()
}

fn optional_string(value: Option<&Json>, name: &str) -> Result<Option<String>> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(Error::new(
            codes::DEBUG_REQUEST,
            format!("the `{name}` field must be a string or null"),
        )),
    }
}

pub(crate) fn require_keys(
    object: &BTreeMap<String, Json>,
    required: &[&str],
    what: &str,
) -> Result<()> {
    for key in required {
        if !object.contains_key(*key) {
            return Err(Error::new(
                codes::DEBUG_REQUEST,
                format!("the {what} is missing `{key}`"),
            ));
        }
    }
    Ok(())
}

pub(crate) fn require_allowed_keys(
    object: &BTreeMap<String, Json>,
    allowed: &[&str],
    what: &str,
) -> Result<()> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(Error::new(
                codes::DEBUG_REQUEST,
                format!("the {what} has an unknown key `{key}`"),
            ));
        }
    }
    Ok(())
}

pub(crate) fn decode_type_value(
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

pub(crate) fn base64_encode(bytes: &[u8]) -> String {
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
            type_metadata: Vec::new(),
            aggregates: Vec::new(),
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
        assert!(matches!(
            aggregate.kind,
            AggregateTargetKind::Record {
                ty: TypeRecord {
                    type_kind: TypeKind::Struct,
                    ..
                }
            }
        ));
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
