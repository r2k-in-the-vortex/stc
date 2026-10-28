//! The `stc-debug/1` local protocol server (Debug and Monitoring §13, §14).
//!
//! The server owns the transport, endpoint file, audit log, authentication,
//! sessions, authorization, rate and idle limits, and the protocol mapping of
//! the phase-3/4 typed debug service. It never dereferences host, module, root,
//! or process-image storage: reads, writes, watches, and manifest exports are
//! submitted as commands to [`DebugService`] and completed by the cycle thread
//! at a boundary. Breakpoints, stops, stepping, and call stacks are resolved
//! against the session's validated checkpoint and stopped-frame tables; the
//! cycle thread parks at a matching checkpoint and this thread emits the
//! `stopped`/`resumed` events and rejects stale snapshots.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use stc_got::digest::GotId;
use stc_got::error::{Error, Result, codes};
use stc_got::json::{self, Json};

use crate::debug::{
    AuditEvent, AuditOutcome, AuditSink, AuditTarget, DebugBatch, DebugEvent, DebugQueueResult,
    DebugRead, DebugReadRequest, DebugService, DebugSnapshot, DebugTarget, DebugValue,
    DebugWarning, DebugWriteKind, DebugWriteRequest, MAX_WATCHES, MIN_WATCH_INTERVAL_NS, WatchId,
    WatchInfo, WatchResult, base64_encode, typed_value_from_json, typed_value_json,
};
use crate::debug_view::view_json;
use crate::stop::{
    BreakpointInfo, BreakpointTarget, PauseOutcome, SessionStop, SourcePosition, StepMode,
    StopEvent,
};
use crate::transport::{self, ClientConnection, Connection, Listener, TransportKind};

/// The only protocol version accepted by this server.
pub const DEBUG_PROTOCOL: &str = "stc-debug/1";
/// Maximum request frame size.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
/// Maximum response or event frame size.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

const MAX_ID: i128 = (1_i128 << 53) - 1;
const FIRST_REQUEST_DEADLINE: Duration = Duration::from_secs(5);
const REQUEST_DEADLINE: Duration = Duration::from_secs(5);
const READ_POLL: Duration = Duration::from_millis(25);
const CONNECT_RETRY: Duration = Duration::from_millis(25);
const RATE_PER_SECOND: f64 = 100.0;
const RATE_BURST: f64 = 200.0;
const RATE_ERROR_LIMIT: usize = 20;
const RATE_ERROR_WINDOW: Duration = Duration::from_secs(60);
const AUTH_FAIL_ENDPOINT: usize = 20;
const AUTH_FAIL_WINDOW: Duration = Duration::from_secs(60);
const AUTH_REFUSAL: Duration = Duration::from_secs(60);
const MAX_MALFORMED_FRAMES: usize = 3;
const DEFAULT_SESSION_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_WATCH_INTERVAL_MS: i64 = 3_600_000;

// ---------------------------------------------------------------------------
// Public configuration and host-facing information
// ---------------------------------------------------------------------------

/// Configuration for one local debug server.
#[derive(Clone, Debug)]
pub struct DebugServerConfig {
    /// Endpoint file path. Its parent directory must already exist.
    pub endpoint: PathBuf,
    /// Configured transport (`pipe` or loopback `socket`).
    pub transport: TransportKind,
    /// Grants the `write` scope when requested.
    pub allow_write: bool,
    /// Grants the `force` scope when requested.
    pub allow_force: bool,
    /// Idle session bound; zero selects the five-minute default.
    pub session_timeout: Duration,
}

impl DebugServerConfig {
    /// Creates a configuration with the default five-minute idle bound.
    #[must_use]
    pub fn new(endpoint: impl Into<PathBuf>, transport: TransportKind) -> Self {
        Self {
            endpoint: endpoint.into(),
            transport,
            allow_write: false,
            allow_force: false,
            session_timeout: DEFAULT_SESSION_TIMEOUT,
        }
    }
}

/// The public description of a running debug server.
///
/// The token is deliberately absent; a client reads it from the user-only
/// endpoint file. `endpoint` is transport-specific (a pipe name, Unix socket
/// path, or loopback address).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugServerInfo {
    /// Endpoint file path.
    pub endpoint_file: PathBuf,
    /// Transport-specific endpoint.
    pub endpoint: String,
    /// Audit log path.
    pub audit_log: PathBuf,
    /// `pipe` or `socket`.
    pub transport: &'static str,
    /// Always `stc-debug/1`.
    pub protocol: &'static str,
    /// Generation described by the endpoint file at the last refresh.
    pub generation: u64,
    /// Build identity described by the endpoint file.
    pub build_id: GotId,
    /// `debug` or `release`.
    pub mode: String,
    /// Host process identity.
    pub pid: u32,
}

/// Initial active-generation tags used before the first engine snapshot.
#[derive(Clone, Debug)]
pub(crate) struct InitialDebugState {
    pub(crate) generation: u64,
    pub(crate) build_id: GotId,
    pub(crate) cycle: u64,
    pub(crate) mode: String,
}

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

struct AuditLogState {
    file: Option<std::fs::File>,
    seq: u64,
}

/// The single format-1 JSON Lines writer shared by the engine and the server.
///
/// Sequence numbers and clocks are assigned here so one file has one sequence
/// even when engine and server threads append concurrently.
struct SharedAudit {
    state: Mutex<AuditLogState>,
}

impl SharedAudit {
    fn new(file: std::fs::File) -> Self {
        Self {
            state: Mutex::new(AuditLogState {
                file: Some(file),
                seq: 0,
            }),
        }
    }

    fn append(&self, mut event: AuditEvent) -> Result<()> {
        let mut state = lock(&self.state);
        if state.file.is_none() {
            return Err(Error::new(
                codes::DEBUG_AUDIT,
                "the debug audit log is closed",
            ));
        }
        state.seq = state.seq.saturating_add(1).max(1);
        event.seq = state.seq;
        event.monotonic_ns = transport::monotonic_ns();
        event.local_date_ns = transport::local_date_ns();
        let line = event.to_json_line();
        let file = state.file.as_mut().expect("checked above");
        file.write_all(line.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.flush())
            .map_err(|error| {
                Error::new(
                    codes::DEBUG_AUDIT,
                    format!("cannot append the debug audit record: {error}"),
                )
            })
    }

    fn close(&self) {
        let mut state = lock(&self.state);
        if let Some(mut file) = state.file.take() {
            let _ = file.flush();
            let _ = file.sync_all();
        }
    }
}

/// The engine sink facade over the server's audit file.
struct SharedAuditSink {
    audit: Arc<SharedAudit>,
}

impl AuditSink for SharedAuditSink {
    fn record(&mut self, event: &AuditEvent) -> Result<()> {
        self.audit.append(event.clone())
    }
}

// ---------------------------------------------------------------------------
// Shared server state
// ---------------------------------------------------------------------------

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Scope {
    Observe,
    Control,
    Write,
    Force,
}

impl Scope {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "observe" => Some(Self::Observe),
            "control" => Some(Self::Control),
            "write" => Some(Self::Write),
            "force" => Some(Self::Force),
            _ => None,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Control => "control",
            Self::Write => "write",
            Self::Force => "force",
        }
    }
}

struct RateLimiter {
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    fn new(now: Instant) -> Self {
        Self {
            tokens: RATE_BURST,
            last: now,
        }
    }

    fn allow(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * RATE_PER_SECOND).min(RATE_BURST);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

struct SessionRecord {
    id: String,
    connection: u64,
    scopes: BTreeSet<Scope>,
    ending: bool,
    last_activity: Instant,
    rate: RateLimiter,
    rate_errors: VecDeque<Instant>,
}

#[derive(Clone)]
struct SessionView {
    id: String,
    scopes: BTreeSet<Scope>,
    ending: bool,
}

struct SessionsState {
    current: Option<SessionRecord>,
    next: u64,
}

struct AuthLimiter {
    failures: VecDeque<Instant>,
    refuse_until: Option<Instant>,
}

impl AuthLimiter {
    fn new() -> Self {
        Self {
            failures: VecDeque::new(),
            refuse_until: None,
        }
    }

    fn refusing(&mut self, now: Instant) -> bool {
        if self.refuse_until.is_some_and(|until| now < until) {
            return true;
        }
        self.refuse_until = None;
        false
    }

    fn note_failure(&mut self, now: Instant) {
        self.failures.push_back(now);
        while self
            .failures
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) > AUTH_FAIL_WINDOW)
        {
            self.failures.pop_front();
        }
        if self.failures.len() >= AUTH_FAIL_ENDPOINT {
            self.refuse_until = Some(now + AUTH_REFUSAL);
            self.failures.clear();
        }
    }
}

struct Shared {
    service: DebugService,
    token: String,
    allow_write: bool,
    allow_force: bool,
    session_timeout: Duration,
    audit: Arc<SharedAudit>,
    sessions: Mutex<SessionsState>,
    auth: Mutex<AuthLimiter>,
    shutdown: Arc<AtomicBool>,
    next_connection: AtomicU64,
    initial: InitialDebugState,
    transport: &'static str,
    endpoint_file: PathBuf,
    audit_path: PathBuf,
}

impl Shared {
    fn tags(&self) -> (u64, GotId, u64) {
        self.service.snapshot().map_or(
            (
                self.initial.generation,
                self.initial.build_id,
                self.initial.cycle,
            ),
            |snapshot| (snapshot.generation, snapshot.build_id, snapshot.cycle),
        )
    }

    fn snapshot(&self) -> Option<DebugSnapshot> {
        self.service.snapshot()
    }

    fn audit(&self, event: AuditEvent) -> Result<()> {
        self.audit.append(event)
    }

    fn session_view(&self, connection: u64) -> Option<SessionView> {
        let state = lock(&self.sessions);
        state
            .current
            .as_ref()
            .filter(|session| session.connection == connection)
            .map(|session| SessionView {
                id: session.id.clone(),
                scopes: session.scopes.clone(),
                ending: session.ending,
            })
    }

    fn begin_session(
        &self,
        connection: u64,
        scopes: BTreeSet<Scope>,
        now: Instant,
    ) -> Option<SessionView> {
        let mut state = lock(&self.sessions);
        if state.current.is_some() {
            return None;
        }
        state.next = state.next.saturating_add(1).max(1);
        let suffix = transport::random_hex(8).unwrap_or_else(|_| format!("{:016x}", state.next));
        let id = format!("s{}-{suffix}", state.next);
        state.current = Some(SessionRecord {
            id: id.clone(),
            connection,
            scopes: scopes.clone(),
            ending: false,
            last_activity: now,
            rate: RateLimiter::new(now),
            rate_errors: VecDeque::new(),
        });
        Some(SessionView {
            id,
            scopes,
            ending: false,
        })
    }

    fn set_ending(&self, connection: u64) -> Option<SessionView> {
        let mut state = lock(&self.sessions);
        let session = state
            .current
            .as_mut()
            .filter(|session| session.connection == connection)?;
        session.ending = true;
        Some(SessionView {
            id: session.id.clone(),
            scopes: session.scopes.clone(),
            ending: true,
        })
    }

    fn release_session(&self, connection: u64) {
        let mut state = lock(&self.sessions);
        if state
            .current
            .as_ref()
            .is_some_and(|session| session.connection == connection)
        {
            state.current = None;
        }
    }

    fn touch(&self, connection: u64, now: Instant) {
        let mut state = lock(&self.sessions);
        if let Some(session) = state
            .current
            .as_mut()
            .filter(|session| session.connection == connection)
        {
            session.last_activity = now;
        }
    }

    fn rate_allows(&self, connection: u64, now: Instant) -> Option<bool> {
        let mut state = lock(&self.sessions);
        let session = state
            .current
            .as_mut()
            .filter(|session| session.connection == connection)?;
        Some(session.rate.allow(now))
    }

    /// Records a rate error and reports whether the circuit breaker closes.
    fn note_rate_error(&self, connection: u64, now: Instant) -> Option<bool> {
        let mut state = lock(&self.sessions);
        let session = state
            .current
            .as_mut()
            .filter(|session| session.connection == connection)?;
        session.rate_errors.push_back(now);
        while session
            .rate_errors
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) > RATE_ERROR_WINDOW)
        {
            session.rate_errors.pop_front();
        }
        Some(session.rate_errors.len() >= RATE_ERROR_LIMIT)
    }

    fn idle_expired(&self, connection: u64, now: Instant, has_watches: bool) -> bool {
        if has_watches {
            return false;
        }
        let state = lock(&self.sessions);
        state
            .current
            .as_ref()
            .filter(|session| session.connection == connection)
            .is_some_and(|session| {
                now.saturating_duration_since(session.last_activity) >= self.session_timeout
            })
    }
}

// ---------------------------------------------------------------------------
// Server lifecycle
// ---------------------------------------------------------------------------

/// A running local debug service.
pub struct DebugServer {
    shared: Arc<Shared>,
    info: DebugServerInfo,
    listener: Option<Listener>,
    accept: Option<std::thread::JoinHandle<()>>,
    handlers: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    shutdown: Arc<AtomicBool>,
    stopped: bool,
}

impl DebugServer {
    pub(crate) fn prepare(
        config: &DebugServerConfig,
        initial: InitialDebugState,
        service: DebugService,
    ) -> Result<Self> {
        let endpoint_file = config.endpoint.clone();
        if endpoint_file.file_name().is_none() {
            return Err(Error::new(
                codes::DEBUG_TRANSPORT,
                "the debug endpoint path has no file name",
            ));
        }
        let audit_path = endpoint_file.with_extension("audit.jsonl");
        let (listener, endpoint) = bind_transport(config.transport)?;
        let token = match transport::random_hex(32) {
            Ok(token) => token,
            Err(_) => {
                let mut listener = listener;
                listener.shutdown();
                return Err(Error::new(
                    codes::DEBUG_TRANSPORT,
                    "cannot obtain the operating-system random source for the debug token",
                ));
            }
        };
        let audit_file = match transport::create_private_file(&audit_path) {
            Ok(file) => file,
            Err(error) => {
                let mut listener = listener;
                listener.shutdown();
                return Err(Error::new(
                    codes::DEBUG_AUDIT,
                    format!("cannot create the user-only debug audit log: {error}"),
                ));
            }
        };
        let audit = Arc::new(SharedAudit::new(audit_file));
        let contents = endpoint_contents(
            &audit_path,
            initial.build_id,
            &endpoint,
            initial.generation,
            &initial.mode,
            &token,
            config.transport,
        );
        if let Err(error) = transport::write_private_atomic(&endpoint_file, &contents, false) {
            let mut listener = listener;
            listener.shutdown();
            audit.close();
            let _ = std::fs::remove_file(&audit_path);
            return Err(Error::new(
                codes::DEBUG_TRANSPORT,
                format!("cannot create the user-only debug endpoint file: {error}"),
            ));
        }
        let shared = Arc::new(Shared {
            service,
            token,
            allow_write: config.allow_write,
            allow_force: config.allow_force,
            session_timeout: if config.session_timeout.is_zero() {
                DEFAULT_SESSION_TIMEOUT
            } else {
                config.session_timeout
            },
            audit: Arc::clone(&audit),
            sessions: Mutex::new(SessionsState {
                current: None,
                next: 0,
            }),
            auth: Mutex::new(AuthLimiter::new()),
            shutdown: Arc::new(AtomicBool::new(false)),
            next_connection: AtomicU64::new(0),
            initial: initial.clone(),
            transport: config.transport.name(),
            endpoint_file: endpoint_file.clone(),
            audit_path: audit_path.clone(),
        });
        let mut start = AuditEvent::new("server-start", AuditOutcome::Ok);
        start.generation = Some(initial.generation);
        start.build = Some(initial.build_id);
        start.detail = object([
            ("mode", Json::string(initial.mode.clone())),
            ("transport", Json::string(config.transport.name())),
        ]);
        if let Err(error) = shared.audit(start) {
            let mut listener = listener;
            listener.shutdown();
            audit.close();
            let _ = std::fs::remove_file(&endpoint_file);
            let _ = std::fs::remove_file(&audit_path);
            return Err(error);
        }
        let info = DebugServerInfo {
            endpoint_file,
            endpoint,
            audit_log: audit_path,
            transport: config.transport.name(),
            protocol: DEBUG_PROTOCOL,
            generation: initial.generation,
            build_id: initial.build_id,
            mode: initial.mode,
            pid: std::process::id(),
        };
        let server = Self {
            shared,
            info,
            listener: Some(listener),
            accept: None,
            handlers: Arc::new(Mutex::new(Vec::new())),
            shutdown: Arc::new(AtomicBool::new(false)),
            stopped: false,
        };
        Ok(server)
    }

    pub(crate) fn audit_sink(&self) -> Box<dyn AuditSink> {
        Box::new(SharedAuditSink {
            audit: Arc::clone(&self.shared.audit),
        })
    }

    pub(crate) fn launch(&mut self) {
        if self.accept.is_some() || self.stopped {
            return;
        }
        let Some(mut listener) = self.listener.take() else {
            return;
        };
        let shared = Arc::clone(&self.shared);
        let shutdown = Arc::clone(&self.shutdown);
        let handlers = Arc::clone(&self.handlers);
        self.accept = Some(std::thread::spawn(move || {
            crate::priority::lower_service_priority();
            while !shutdown.load(Ordering::Acquire) {
                let Some(connection) = listener.accept(&shutdown) else {
                    if shutdown.load(Ordering::Acquire) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let now = Instant::now();
                if lock(&shared.auth).refusing(now) {
                    let mut event = AuditEvent::new("transport-error", AuditOutcome::Error);
                    event.detail = object([("reason", Json::string("auth-refused"))]);
                    let _ = shared.audit(event);
                    drop(connection);
                    continue;
                }
                let connection_id = shared
                    .next_connection
                    .fetch_add(1, Ordering::AcqRel)
                    .saturating_add(1);
                let shared = Arc::clone(&shared);
                let handle = std::thread::spawn(move || {
                    crate::priority::lower_service_priority();
                    run_connection(shared, connection, connection_id);
                });
                lock(&handlers).push(handle);
            }
            listener.shutdown();
        }));
    }

    /// Returns the public server description.
    #[must_use]
    pub fn info(&self) -> DebugServerInfo {
        self.info.clone()
    }

    /// Atomically refreshes the endpoint file's generation, build, and mode.
    pub fn refresh(&mut self, generation: u64, build_id: GotId, mode: &str) -> std::io::Result<()> {
        let contents = endpoint_contents(
            &self.shared.audit_path,
            build_id,
            &self.info.endpoint,
            generation,
            mode,
            &self.shared.token,
            match self.info.transport {
                "socket" => TransportKind::Socket,
                _ => TransportKind::Pipe,
            },
        );
        transport::write_private_atomic(&self.shared.endpoint_file, &contents, true)?;
        self.info.generation = generation;
        self.info.build_id = build_id;
        self.info.mode = mode.to_owned();
        Ok(())
    }

    /// Stops the server, removes the endpoint file, and writes `server-stop`.
    pub fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        self.shutdown.store(true, Ordering::Release);
        // Connection threads poll the shared flag too, so a direct
        // `Host::stop_debug_server` ends an active session instead of waiting
        // for the client to disconnect.
        self.shared.shutdown.store(true, Ordering::Release);
        if let Some(mut listener) = self.listener.take() {
            listener.shutdown();
        }
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        let handlers = std::mem::take(&mut *lock(&self.handlers));
        for handle in handlers {
            let _ = handle.join();
        }
        let mut event = AuditEvent::new("server-stop", AuditOutcome::Ok);
        event.generation = Some(self.info.generation);
        event.build = Some(self.info.build_id);
        event.detail = object([
            ("mode", Json::string(self.info.mode.clone())),
            ("transport", Json::string(self.info.transport)),
        ]);
        let _ = self.shared.audit(event);
        self.shared.audit.close();
        if transport::refuse_symlink(&self.shared.endpoint_file).is_ok() {
            let _ = std::fs::remove_file(&self.shared.endpoint_file);
        }
    }

    /// Returns the endpoint file path.
    #[must_use]
    pub fn endpoint_file(&self) -> &Path {
        &self.shared.endpoint_file
    }
}

impl Drop for DebugServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn bind_transport(kind: TransportKind) -> Result<(Listener, String)> {
    match kind {
        TransportKind::Socket => {
            let (listener, endpoint) = Listener::bind_tcp()?;
            Ok((listener, endpoint))
        }
        TransportKind::Pipe => bind_pipe(),
    }
}

#[cfg(windows)]
fn bind_pipe() -> Result<(Listener, String)> {
    let suffix = transport::random_hex(8).map_err(|_| {
        Error::new(
            codes::DEBUG_TRANSPORT,
            "cannot obtain the operating-system random source for the debug pipe",
        )
    })?;
    let endpoint = transport::default_pipe_endpoint(&format!("debug-{suffix}"));
    let listener = Listener::bind_pipe(&endpoint)?;
    Ok((listener, endpoint))
}

#[cfg(unix)]
fn bind_pipe() -> Result<(Listener, String)> {
    let mut suffix_bytes = [0_u8; 8];
    transport::fill_random(&mut suffix_bytes).map_err(|_| {
        Error::new(
            codes::DEBUG_TRANSPORT,
            "cannot obtain the operating-system random source for the debug socket",
        )
    })?;
    let mut suffix = String::with_capacity(16);
    for byte in suffix_bytes {
        use std::fmt::Write as _;
        let _ = write!(suffix, "{byte:02x}");
    }
    let directory = PathBuf::from(format!(".stc-debug-{suffix}.sockd"));
    let directory = std::env::current_dir()
        .map(|current| current.join(&directory))
        .unwrap_or(directory);
    transport::create_private_dir(&directory).map_err(|error| {
        Error::new(
            codes::DEBUG_TRANSPORT,
            format!("cannot create the user-private debug socket directory: {error}"),
        )
    })?;
    let socket = directory.join("s");
    let endpoint = socket.to_string_lossy().into_owned();
    match Listener::bind_pipe(&endpoint) {
        Ok(listener) => Ok((listener, endpoint)),
        Err(error) => {
            transport::remove_socket_dir(&directory);
            Err(error)
        }
    }
}

fn endpoint_contents(
    audit_log: &Path,
    build_id: GotId,
    endpoint: &str,
    generation: u64,
    mode: &str,
    token: &str,
    transport: TransportKind,
) -> String {
    object([
        (
            "audit_log",
            Json::string(audit_log.to_string_lossy().into_owned()),
        ),
        ("build_id", Json::string(build_id.to_hex())),
        ("endpoint", Json::string(endpoint.to_owned())),
        ("format", Json::unsigned(1)),
        ("generation", Json::unsigned(generation)),
        ("mode", Json::string(mode.to_owned())),
        ("pid", Json::unsigned(u64::from(std::process::id()))),
        ("protocol", Json::string(DEBUG_PROTOCOL)),
        ("token", Json::string(token.to_owned())),
        ("transport", Json::string(transport.name())),
    ])
    .render_compact()
}

fn object<const N: usize>(pairs: [(&str, Json); N]) -> Json {
    Json::object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value)),
    )
}

// ---------------------------------------------------------------------------
// Protocol errors
// ---------------------------------------------------------------------------

struct ProtocolError {
    code: &'static str,
    message: String,
    retryable: bool,
    detail: Json,
    close: bool,
    malformed: bool,
}

impl ProtocolError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: retryable(code),
            detail: Json::Object(BTreeMap::new()),
            close: false,
            malformed: false,
        }
    }

    fn malformed(message: impl Into<String>) -> Self {
        let mut error = Self::new(codes::DEBUG_REQUEST, message);
        error.malformed = true;
        error
    }

    fn with_detail(mut self, detail: Json) -> Self {
        self.detail = detail;
        self
    }

    fn from_stc(error: Error) -> Self {
        if error.code.starts_with("STC-E-DEBUG-") {
            Self::new(error.code, error.message)
        } else {
            Self::new(codes::DEBUG_INTERNAL, error.message)
        }
    }
}

fn retryable(code: &str) -> bool {
    matches!(
        code,
        codes::DEBUG_PROTOCOL | codes::DEBUG_SESSION | codes::DEBUG_STALE | codes::DEBUG_RATE
    )
}

/// Returns whether an I/O error is an ordinary peer disconnect rather than a
/// transport fault.
///
/// Windows reports a dropped named pipe as `ERROR_NO_DATA` or
/// `ERROR_PIPE_NOT_CONNECTED`, and a reset loopback socket as
/// `WSAECONNRESET`; those are normal client disconnects, not protocol errors.
fn peer_closed(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::UnexpectedEof
    ) || matches!(error.raw_os_error(), Some(232 | 233 | 10054))
}

struct ClosePlan {
    reason: &'static str,
    code: Option<&'static str>,
    event: bool,
    clear: bool,
    /// Generation tags captured when the cycle thread ended the session. They
    /// describe the generation the session ran on, which may already have been
    /// replaced by the time the connection thread emits `session-ended`.
    tags: Option<(u64, GotId, u64)>,
}

impl ClosePlan {
    fn new(reason: &'static str, code: Option<&'static str>, event: bool, clear: bool) -> Self {
        Self {
            reason,
            code,
            event,
            clear,
            tags: None,
        }
    }

    fn with_tags(mut self, generation: u64, build_id: GotId, cycle: u64) -> Self {
        self.tags = Some((generation, build_id, cycle));
        self
    }
}

// ---------------------------------------------------------------------------
// Connection handler
// ---------------------------------------------------------------------------

struct Handler {
    shared: Arc<Shared>,
    connection: u64,
    token: String,
    session: Option<SessionView>,
    seq: u64,
    last_id: u64,
    first_request: bool,
    protocol_retry: bool,
    malformed: usize,
    buffer: Vec<u8>,
    frame_started: Option<Instant>,
    first_deadline: Instant,
    close: Option<ClosePlan>,
    finished: bool,
    /// Installed stop session; present only for hook-emitting builds with a
    /// validated checkpoint table.
    session_stop: Option<Arc<SessionStop>>,
}

fn run_connection(shared: Arc<Shared>, mut connection: Connection, connection_id: u64) {
    let _ = connection.set_write_timeout(Some(REQUEST_DEADLINE));
    let mut connect = AuditEvent::new("connect", AuditOutcome::Ok);
    connect.detail = object([("transport", Json::string(shared.transport))]);
    let _ = shared.audit(connect);
    let accepted = Instant::now();
    let mut token = String::new();
    token.push_str(&shared.token);
    let mut handler = Handler {
        shared,
        connection: connection_id,
        token,
        session: None,
        seq: 0,
        last_id: 0,
        first_request: true,
        protocol_retry: false,
        malformed: 0,
        buffer: Vec::new(),
        frame_started: None,
        first_deadline: accepted + FIRST_REQUEST_DEADLINE,
        close: None,
        finished: false,
        session_stop: None,
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        handler.serve(&mut connection);
    }));
    if outcome.is_err() {
        let mut event = AuditEvent::new("transport-error", AuditOutcome::Error);
        event
            .session
            .clone_from(&handler.session.as_ref().map(|view| view.id.clone()));
        event.detail = object([("reason", Json::string("internal"))]);
        let _ = handler.shared.audit(event);
        handler.close = Some(ClosePlan::new(
            "internal",
            Some(codes::DEBUG_INTERNAL),
            true,
            true,
        ));
    }
    let plan = handler
        .close
        .take()
        .unwrap_or_else(|| ClosePlan::new("disconnect", None, false, true));
    handler.finish(&mut connection, plan);
}

impl Handler {
    fn serve(&mut self, connection: &mut Connection) {
        loop {
            // A malformed-frame or authentication path may finish the
            // connection from inside `process_frame`; its caller still reports
            // "continue", so the terminal state is checked here as well.
            if self.finished {
                return;
            }
            if let Some(plan) = self.close.take() {
                self.finish(connection, plan);
                return;
            }
            if self.shared.shutdown.load(Ordering::Acquire) {
                let plan = self.shutdown_plan();
                self.finish(connection, plan);
                return;
            }
            if self.session.is_some()
                && let Some(plan) = self.poll_session(connection)
            {
                self.finish(connection, plan);
                return;
            }
            if let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=position).collect();
                let line = &line[..line.len().saturating_sub(1)];
                self.frame_started = None;
                if self.process_frame(connection, line) {
                    continue;
                }
                return;
            }
            if self.buffer.len() > MAX_REQUEST_BYTES {
                self.framing_close(connection, "request frame exceeds the 1 MiB limit");
                return;
            }
            let now = Instant::now();
            if self.buffer.is_empty() {
                if self.first_request && now >= self.first_deadline {
                    self.audit_timeout("first-request");
                    self.finish(
                        connection,
                        ClosePlan::new("protocol-error", None, true, true),
                    );
                    return;
                }
            } else {
                let started = *self.frame_started.get_or_insert(now);
                if now.saturating_duration_since(started) >= REQUEST_DEADLINE {
                    self.audit_timeout("partial-frame");
                    self.framing_close(connection, "a partial frame did not complete in time");
                    return;
                }
            }
            connection.set_read_deadline(Some(now + READ_POLL));
            let mut chunk = [0_u8; 16 * 1024];
            match connection.read(&mut chunk) {
                Ok(0) => {
                    let plan = if self.shared.shutdown.load(Ordering::Acquire) {
                        self.shutdown_plan()
                    } else {
                        ClosePlan::new("disconnect", None, false, true)
                    };
                    self.finish(connection, plan);
                    return;
                }
                Ok(count) => {
                    self.buffer.extend_from_slice(&chunk[..count]);
                    connection.set_read_deadline(None);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    connection.set_read_deadline(None);
                }
                Err(error) if peer_closed(&error) => {
                    let plan = if self.shared.shutdown.load(Ordering::Acquire) {
                        self.shutdown_plan()
                    } else {
                        ClosePlan::new("disconnect", None, false, true)
                    };
                    self.finish(connection, plan);
                    return;
                }
                Err(_) => {
                    let mut event = AuditEvent::new("transport-error", AuditOutcome::Error);
                    event
                        .session
                        .clone_from(&self.session.as_ref().map(|view| view.id.clone()));
                    event.detail = object([("reason", Json::string("read"))]);
                    let _ = self.shared.audit(event);
                    let plan = self.close_plan("protocol-error", None, true, true);
                    self.finish(connection, plan);
                    return;
                }
            }
        }
    }

    fn close_plan(
        &self,
        reason: &'static str,
        code: Option<&'static str>,
        event: bool,
        clear: bool,
    ) -> ClosePlan {
        ClosePlan::new(reason, code, event && self.session.is_some(), clear)
    }

    /// Chooses the plan for a host-initiated shutdown.
    ///
    /// A target trap or other teardown may have queued its notice just before
    /// host shutdown. That reason wins over the transport-level shutdown so the
    /// client can distinguish a target exit from an operator shutdown.
    fn shutdown_plan(&mut self) -> ClosePlan {
        if self.session.is_some()
            && let Some(notice) = self.shared.service.take_end_notice()
        {
            return ClosePlan::new(notice.reason, notice.code, true, false).with_tags(
                notice.generation,
                notice.build_id,
                notice.cycle,
            );
        }
        self.close_plan("shutdown", None, true, false)
    }

    fn poll_session(&mut self, connection: &mut Connection) -> Option<ClosePlan> {
        self.drain_stop_events(connection);
        if let Some(plan) = self.close.take() {
            return Some(plan);
        }
        self.drain_events(connection);
        if let Some(notice) = self.shared.service.take_end_notice() {
            let plan = ClosePlan::new(notice.reason, notice.code, true, false).with_tags(
                notice.generation,
                notice.build_id,
                notice.cycle,
            );
            return Some(plan);
        }
        if self.shared.shutdown.load(Ordering::Acquire) {
            return Some(self.close_plan("shutdown", None, true, false));
        }
        let now = Instant::now();
        let watches = self
            .shared
            .snapshot()
            .map_or(0, |snapshot| snapshot.watches);
        // While the cycle is parked the stop bound in §14.3 governs; the idle
        // bound must not preempt it. Both use the same configured duration.
        let stopped = self
            .session_stop
            .as_ref()
            .is_some_and(|session| session.status().stop.is_some());
        if !stopped && self.shared.idle_expired(self.connection, now, watches > 0) {
            let mut event = AuditEvent::new("timeout", AuditOutcome::Error);
            event
                .session
                .clone_from(&self.session.as_ref().map(|view| view.id.clone()));
            event.detail = object([("reason", Json::string("idle"))]);
            let _ = self.shared.audit(event);
            return Some(ClosePlan::new(
                "idle-timeout",
                Some(codes::DEBUG_SESSION_TIMEOUT),
                true,
                true,
            ));
        }
        None
    }

    fn audit_timeout(&self, phase: &str) {
        let mut event = AuditEvent::new("timeout", AuditOutcome::Error);
        event.detail = object([("phase", Json::string(phase))]);
        let _ = self.shared.audit(event);
    }

    fn framing_close(&mut self, connection: &mut Connection, detail: &str) {
        let id = 0_u64;
        let error = ProtocolError::new(codes::DEBUG_FRAME, detail);
        self.send_error(connection, id, &error);
        let mut event = AuditEvent::new("transport-error", AuditOutcome::Error);
        event
            .session
            .clone_from(&self.session.as_ref().map(|view| view.id.clone()));
        event.detail = object([("reason", Json::string("framing"))]);
        let _ = self.shared.audit(event);
        let plan = self.close_plan("protocol-error", None, true, true);
        self.finish(connection, plan);
    }

    fn finish(&mut self, connection: &mut Connection, plan: ClosePlan) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Some(view) = self.session.clone() {
            let (generation, build_id, cycle) = plan.tags.unwrap_or_else(|| self.shared.tags());
            if plan.event {
                let mut data = BTreeMap::new();
                data.insert("reason".into(), Json::string(plan.reason));
                if let Some(code) = plan.code {
                    data.insert("code".into(), Json::string(code));
                }
                data.insert("generation".into(), Json::unsigned(generation));
                data.insert("build_id".into(), Json::string(build_id.to_hex()));
                data.insert("cycle".into(), Json::unsigned(cycle));
                self.send_event(connection, "session-ended", Json::Object(data), Vec::new());
            }
            if plan.clear
                && let Ok(reply) = self.shared.service.submit_clear_session_wait()
            {
                let _ = reply.wait(Instant::now() + REQUEST_DEADLINE);
            }
            // A parked cycle services the clear command from its own pump;
            // this wake is the fallback when the cycle is running or the
            // command could not be submitted.
            if let Some(stop) = self.session_stop.take() {
                crate::stop::uninstall(&stop);
            }
            self.shared.service.set_session(None);
            self.shared.release_session(self.connection);
            self.session = None;
            let mut event = AuditEvent::new("disconnect", AuditOutcome::Ok);
            event.session = Some(view.id);
            event.generation = Some(generation);
            event.build = Some(build_id);
            event.cycle = Some(cycle);
            event.detail = object([("reason", Json::string(plan.reason))]);
            let _ = self.shared.audit(event);
        } else {
            let mut event = AuditEvent::new("disconnect", AuditOutcome::Ok);
            event.detail = object([("reason", Json::string(plan.reason))]);
            let _ = self.shared.audit(event);
        }
    }

    fn process_frame(&mut self, connection: &mut Connection, line: &[u8]) -> bool {
        if line.starts_with(&[0xEF, 0xBB, 0xBF]) {
            self.malformed_response(connection, "a frame must not start with a byte-order mark");
            return self.close.is_none();
        }
        if line.is_empty() {
            self.malformed_response(connection, "an empty frame is invalid");
            return self.close.is_none();
        }
        let Ok(text) = std::str::from_utf8(line) else {
            self.malformed_response(connection, "a frame must be UTF-8");
            return self.close.is_none();
        };
        let value = match json::parse_protocol(text) {
            Ok(value) => value,
            Err(_) => {
                self.malformed_response(connection, "a frame must be one JSON object");
                return self.close.is_none();
            }
        };
        let Some(object) = value.as_object() else {
            self.malformed_response(connection, "a frame must be one JSON object");
            return self.close.is_none();
        };
        let id = object
            .get("id")
            .and_then(Json::as_number)
            .filter(|id| (1..=MAX_ID).contains(id))
            .and_then(|id| u64::try_from(id).ok());
        let response_id = id.unwrap_or(0);
        if let Some(request_id) = id {
            if request_id <= self.last_id {
                self.malformed_with_id(
                    connection,
                    response_id,
                    "id must strictly increase on one connection",
                );
                return self.close.is_none();
            }
            self.last_id = request_id;
        }
        if has_unknown_keys(object, &["id", "kind", "op", "params", "protocol", "token"]) {
            self.malformed_with_id(connection, response_id, "an unknown request key is invalid");
            return self.close.is_none();
        }
        if object.get("kind").and_then(Json::as_str) != Some("request") {
            self.malformed_with_id(connection, response_id, "the kind must be `request`");
            return self.close.is_none();
        }
        let Some(request_id) = id else {
            self.malformed_with_id(
                connection,
                response_id,
                "id must be an integer in [1, 2^53-1] that strictly increases",
            );
            return self.close.is_none();
        };
        let protocol = object.get("protocol").and_then(Json::as_str);
        if protocol.is_none() {
            self.malformed_with_id(connection, response_id, "protocol is required");
            return self.close.is_none();
        }
        if protocol != Some(DEBUG_PROTOCOL) {
            let error = ProtocolError::new(
                codes::DEBUG_PROTOCOL,
                "the requested debug protocol version is not supported",
            )
            .with_detail(json_object([(
                "supported",
                Json::Array(vec![Json::string(DEBUG_PROTOCOL)]),
            )]));
            self.send_error(connection, response_id, &error);
            if self.first_request && !self.protocol_retry {
                self.protocol_retry = true;
                self.first_request = false;
                return true;
            }
            self.finish(
                connection,
                ClosePlan::new("protocol-error", None, true, true),
            );
            return false;
        }
        let Some(token) = object.get("token").and_then(Json::as_str) else {
            self.authentication_failure(
                connection,
                response_id,
                "a request requires the endpoint token",
            );
            return false;
        };
        if !token_matches(&self.token, token) {
            self.authentication_failure(
                connection,
                response_id,
                "the endpoint token is missing, malformed, or wrong",
            );
            return false;
        }
        let Some(op) = object.get("op").and_then(Json::as_str) else {
            self.malformed_with_id(connection, response_id, "op must be a string");
            return self.close.is_none();
        };
        let Some(params) = object.get("params").and_then(Json::as_object) else {
            self.malformed_with_id(connection, response_id, "params must be an object");
            return self.close.is_none();
        };
        self.first_request = false;
        if !known_operation(op) {
            self.malformed_with_id(
                connection,
                response_id,
                "the operation is not defined by stc-debug/1",
            );
            return self.close.is_none();
        }
        if op == "session.start" {
            self.start_session(connection, request_id, params);
            return self.close.is_none();
        }
        let Some(view) = self.session.clone().and_then(|view| {
            self.shared
                .session_view(self.connection)
                .filter(|current| current.id == view.id)
        }) else {
            let error = ProtocolError::new(
                codes::DEBUG_SESSION,
                "no debug session is active on this connection",
            );
            self.send_error(connection, request_id, &error);
            return self.close.is_none();
        };
        if view.ending {
            let error = ProtocolError::new(
                codes::DEBUG_SESSION,
                "the debug session is ending; reconnect after teardown",
            );
            self.send_error(connection, request_id, &error);
            return self.close.is_none();
        }
        let now = Instant::now();
        match self.shared.rate_allows(self.connection, now) {
            Some(true) => {}
            Some(false) => {
                let close = self
                    .shared
                    .note_rate_error(self.connection, now)
                    .unwrap_or(false);
                let error = ProtocolError::new(
                    codes::DEBUG_RATE,
                    "the request rate limit was exceeded; retry after the window",
                );
                self.send_error(connection, request_id, &error);
                if close {
                    let mut event = AuditEvent::new("transport-error", AuditOutcome::Error);
                    event.session = Some(view.id.clone());
                    event.detail = json_object([("reason", Json::string("rate-limit"))]);
                    let _ = self.shared.audit(event);
                    self.finish(
                        connection,
                        ClosePlan::new("protocol-error", None, true, true),
                    );
                    return false;
                }
                return true;
            }
            None => {
                let error = ProtocolError::new(
                    codes::DEBUG_SESSION,
                    "no debug session is active on this connection",
                );
                self.send_error(connection, request_id, &error);
                return self.close.is_none();
            }
        }
        if let Some(required) = required_scope(op)
            && !view.scopes.contains(&required)
        {
            let mut event = AuditEvent::new("policy-deny", AuditOutcome::Denied);
            event.session = Some(view.id.clone());
            event.scope = Some(required.name().to_owned());
            event.detail = json_object([
                ("code", Json::string(codes::DEBUG_AUTHORIZATION)),
                ("op", Json::string(op)),
            ]);
            let _ = self.shared.audit(event);
            let error = ProtocolError::new(
                codes::DEBUG_AUTHORIZATION,
                format!(
                    "the `{}` scope is required for this operation and was not granted",
                    required.name()
                ),
            )
            .with_detail(json_object([(
                "required_scope",
                Json::string(required.name()),
            )]));
            self.send_error(connection, request_id, &error);
            return self.close.is_none();
        }
        if op == "session.stop" {
            self.send_ok(
                connection,
                request_id,
                json_object([("ended", Json::Bool(true))]),
                Vec::new(),
            );
            self.shared.set_ending(self.connection);
            self.finish(connection, ClosePlan::new("client", None, true, true));
            return false;
        }
        let deadline = Instant::now() + REQUEST_DEADLINE;
        match self.execute(op, params, deadline) {
            Ok((result, warnings)) => {
                if op != "session.status" {
                    self.shared.touch(self.connection, Instant::now());
                }
                self.send_ok(connection, request_id, result, warnings);
                self.drain_stop_events(connection);
                self.drain_events(connection);
                true
            }
            Err(error) => {
                if error.malformed {
                    self.malformed = self.malformed.saturating_add(1);
                }
                self.send_error(connection, request_id, &error);
                if error.close || self.malformed >= MAX_MALFORMED_FRAMES {
                    self.finish(
                        connection,
                        ClosePlan::new("protocol-error", error.code.into(), true, true),
                    );
                    return false;
                }
                if error.code == codes::DEBUG_INTERNAL {
                    self.finish(
                        connection,
                        ClosePlan::new("internal", Some(codes::DEBUG_INTERNAL), true, true),
                    );
                    return false;
                }
                true
            }
        }
    }

    fn start_session(
        &mut self,
        connection: &mut Connection,
        request_id: u64,
        params: &BTreeMap<String, Json>,
    ) {
        if self.session.is_some() || self.shared.session_view(self.connection).is_some() {
            let error = ProtocolError::new(
                codes::DEBUG_SESSION,
                "a debug session is already active on this host",
            );
            self.send_error(connection, request_id, &error);
            return;
        }
        if has_unknown_keys(params, &["client", "scopes"]) {
            self.malformed_with_id(
                connection,
                request_id,
                "an unknown session.start parameter is invalid",
            );
            return;
        }
        let Some(scopes_json) = params.get("scopes").and_then(Json::as_array) else {
            self.malformed_with_id(
                connection,
                request_id,
                "session.start requires a scopes array",
            );
            return;
        };
        let mut requested = BTreeSet::new();
        for value in scopes_json {
            let Some(name) = value.as_str() else {
                self.malformed_with_id(connection, request_id, "a scope name must be a string");
                return;
            };
            let Some(scope) = Scope::parse(name) else {
                self.malformed_with_id(connection, request_id, "an unknown scope name is invalid");
                return;
            };
            if !requested.insert(scope) {
                self.malformed_with_id(
                    connection,
                    request_id,
                    "a scope is requested more than once",
                );
                return;
            }
        }
        let mut client = None;
        if let Some(value) = params.get("client") {
            let Some(object) = value.as_object() else {
                self.malformed_with_id(connection, request_id, "client must be an object");
                return;
            };
            if has_unknown_keys(object, &["name", "version"]) {
                self.malformed_with_id(connection, request_id, "an unknown client key is invalid");
                return;
            }
            let name = object.get("name").and_then(Json::as_str);
            let version = object.get("version").and_then(Json::as_str);
            if object.get("name").is_some() && name.is_none()
                || object.get("version").is_some() && version.is_none()
            {
                self.malformed_with_id(
                    connection,
                    request_id,
                    "client name and version must be strings",
                );
                return;
            }
            client = Some(json_object([
                ("name", name.map(Json::string).unwrap_or(Json::Null)),
                ("version", version.map(Json::string).unwrap_or(Json::Null)),
            ]));
        }
        let Some(snapshot) = self.shared.snapshot() else {
            let error = ProtocolError::new(
                codes::DEBUG_UNAVAILABLE,
                "no active generation is available for a debug session",
            );
            self.send_error(connection, request_id, &error);
            return;
        };
        if self.shared.session_view(self.connection).is_some() {
            let error = ProtocolError::new(
                codes::DEBUG_SESSION,
                "a debug session is already active on this host",
            );
            self.send_error(connection, request_id, &error);
            return;
        }
        let mut granted = BTreeSet::new();
        granted.insert(Scope::Observe);
        for scope in &requested {
            let allowed = match scope {
                Scope::Observe => true,
                Scope::Control => true,
                Scope::Write => self.shared.allow_write,
                Scope::Force => self.shared.allow_force,
            };
            if allowed {
                granted.insert(*scope);
            }
        }
        // No previous event, notice, watch, or force may be replayed into a
        // new session.
        let _ = self.shared.service.drain_events();
        let _ = self.shared.service.take_end_notice();
        let now = Instant::now();
        let Some(view) = self
            .shared
            .begin_session(self.connection, granted.clone(), now)
        else {
            let error = ProtocolError::new(
                codes::DEBUG_SESSION,
                "a debug session is already active on this host",
            );
            self.send_error(connection, request_id, &error);
            return;
        };
        self.shared.service.set_session(Some(view.id.clone()));
        self.seq = 0;
        self.session = Some(view.clone());
        self.session_stop = None;
        if snapshot.hooks
            && snapshot.checkpoints
            && snapshot.metadata_error.is_none()
            && let Some(tables) = snapshot.tables.clone()
        {
            self.session_stop = Some(crate::stop::install(
                tables,
                snapshot.generation,
                snapshot.build_id,
                self.shared.session_timeout,
            ));
        }
        let mut event = AuditEvent::new("session-start", AuditOutcome::Ok);
        event.session = Some(view.id.clone());
        event.generation = Some(snapshot.generation);
        event.build = Some(snapshot.build_id);
        event.cycle = Some(snapshot.cycle);
        let granted_names = granted
            .iter()
            .map(|scope| Json::string(scope.name()))
            .collect::<Vec<_>>();
        let mut detail = BTreeMap::new();
        detail.insert("granted_scopes".into(), Json::Array(granted_names));
        detail.insert(
            "requested_scopes".into(),
            Json::Array(
                requested
                    .iter()
                    .map(|scope| Json::string(scope.name()))
                    .collect(),
            ),
        );
        if let Some(client) = client {
            detail.insert("client".into(), client);
        }
        event.detail = Json::Object(detail);
        let _ = self.shared.audit(event);
        for (scope, enabled) in [
            (Scope::Write, self.shared.allow_write),
            (Scope::Force, self.shared.allow_force),
        ] {
            if requested.contains(&scope) && !enabled {
                let mut deny = AuditEvent::new("policy-deny", AuditOutcome::Denied);
                deny.session = Some(view.id.clone());
                deny.scope = Some(scope.name().to_owned());
                deny.generation = Some(snapshot.generation);
                deny.build = Some(snapshot.build_id);
                deny.cycle = Some(snapshot.cycle);
                deny.detail = object([
                    ("code", Json::string(codes::DEBUG_AUTHORIZATION)),
                    ("op", Json::string("session.start")),
                    ("reason", Json::string("scope-disabled")),
                ]);
                let _ = self.shared.audit(deny);
            }
        }
        let scopes_json = granted
            .iter()
            .map(|scope| Json::string(scope.name()))
            .collect::<Vec<_>>();
        let (capabilities, warnings) = capabilities(&snapshot, scopes_json);
        let result = object([
            ("session", Json::string(view.id)),
            ("capabilities", capabilities),
            ("generation", Json::unsigned(snapshot.generation)),
            ("build_id", Json::string(snapshot.build_id.to_hex())),
            ("cycle", Json::unsigned(snapshot.cycle)),
            ("mode", Json::string(snapshot.mode)),
        ]);
        self.send_ok(connection, request_id, result, warnings);
        self.drain_events(connection);
    }

    fn authentication_failure(
        &mut self,
        connection: &mut Connection,
        request_id: u64,
        message: &str,
    ) {
        let now = Instant::now();
        lock(&self.shared.auth).note_failure(now);
        let mut event = AuditEvent::new("auth-fail", AuditOutcome::Denied);
        event.detail = object([]);
        let _ = self.shared.audit(event);
        let error = ProtocolError::new(codes::DEBUG_AUTH, message);
        self.send_error(connection, request_id, &error);
        let plan = self.close_plan("protocol-error", None, true, true);
        self.finish(connection, plan);
    }

    fn malformed_response(&mut self, connection: &mut Connection, message: &str) -> bool {
        self.malformed_with_id(connection, 0, message)
    }

    fn malformed_with_id(
        &mut self,
        connection: &mut Connection,
        request_id: u64,
        message: &str,
    ) -> bool {
        self.malformed = self.malformed.saturating_add(1);
        let error = ProtocolError::malformed(message);
        self.send_error(connection, request_id, &error);
        if self.malformed >= MAX_MALFORMED_FRAMES {
            self.finish(
                connection,
                ClosePlan::new("protocol-error", Some(codes::DEBUG_REQUEST), true, true),
            );
            return false;
        }
        true
    }

    fn send_ok(&mut self, connection: &mut Connection, id: u64, result: Json, warnings: Vec<Json>) {
        let response = json_object([
            ("kind", Json::string("response")),
            ("protocol", Json::string(DEBUG_PROTOCOL)),
            ("id", Json::unsigned(id)),
            ("status", Json::string("ok")),
            ("result", result),
            ("warnings", Json::Array(warnings)),
        ]);
        if response.render_compact().len() > MAX_MESSAGE_BYTES {
            let error = ProtocolError::new(
                codes::DEBUG_LIMIT,
                "the response exceeds the 4 MiB limit; read the manifest from the artifact",
            );
            self.send_error(connection, id, &error);
            return;
        }
        let _ = write_message(connection, &response);
    }

    fn send_error(&mut self, connection: &mut Connection, id: u64, error: &ProtocolError) {
        let error_object = json_object([
            ("code", Json::string(error.code)),
            ("message", Json::string(error.message.clone())),
            ("retryable", Json::Bool(error.retryable)),
            ("detail", error.detail.clone()),
        ]);
        let response = json_object([
            ("kind", Json::string("response")),
            ("protocol", Json::string(DEBUG_PROTOCOL)),
            ("id", Json::unsigned(id)),
            ("status", Json::string("error")),
            ("error", error_object),
        ]);
        if write_message(connection, &response).is_err() {
            self.close
                .get_or_insert(ClosePlan::new("disconnect", None, false, true));
        }
    }

    fn send_event(
        &mut self,
        connection: &mut Connection,
        name: &str,
        data: Json,
        warnings: Vec<Json>,
    ) {
        let Some(view) = self.session.clone() else {
            return;
        };
        self.seq = self.seq.saturating_add(1).max(1);
        let event = json_object([
            ("kind", Json::string("event")),
            ("protocol", Json::string(DEBUG_PROTOCOL)),
            ("session", Json::string(view.id)),
            ("seq", Json::unsigned(self.seq)),
            ("event", Json::string(name)),
            ("data", data),
            ("warnings", Json::Array(warnings)),
        ]);
        let _ = write_message(connection, &event);
    }

    fn drain_events(&mut self, connection: &mut Connection) {
        if self.session.is_none() {
            return;
        }
        let events = self.shared.service.drain_events();
        for event in events {
            match event {
                DebugEvent::Batch(batch) => {
                    self.emit_batch(connection, batch);
                    self.shared.touch(self.connection, Instant::now());
                }
                DebugEvent::Queue(result) => self.emit_queue(connection, result),
                DebugEvent::Read(_) | DebugEvent::Error(_) => {}
            }
        }
    }

    fn emit_batch(&mut self, connection: &mut Connection, batch: DebugBatch) {
        let mut grouped: BTreeMap<u64, Vec<Json>> = BTreeMap::new();
        let mut consistent: BTreeMap<u64, bool> = BTreeMap::new();
        for value in batch.values {
            let target = target_json(&value.target);
            let (available, payload) = match &value.result {
                WatchResult::Available(typed) => (
                    true,
                    json_object([
                        ("target", target),
                        ("available", Json::Bool(true)),
                        ("value", typed_value_json(typed)),
                    ]),
                ),
                WatchResult::Unavailable { error, warning, .. } => {
                    let code = error.as_ref().map_or(warning.code, |failure| failure.code);
                    (
                        false,
                        json_object([
                            ("target", target),
                            ("available", Json::Bool(false)),
                            ("code", Json::string(code)),
                        ]),
                    )
                }
            };
            let _ = available;
            grouped.entry(value.watch_id.0).or_default().push(payload);
            consistent
                .entry(value.watch_id.0)
                .and_modify(|current| *current &= value.consistent)
                .or_insert(value.consistent);
        }
        let warnings = batch
            .warnings
            .iter()
            .map(|warning| warning_json(warning.code, warning.message.clone(), Json::Null))
            .collect::<Vec<_>>();
        let mut first = true;
        for (watch_id, values) in grouped {
            let data = json_object([
                ("watch_id", Json::unsigned(watch_id)),
                ("generation", Json::unsigned(batch.generation)),
                ("build_id", Json::string(batch.build_id.to_hex())),
                ("cycle", Json::unsigned(batch.cycle)),
                (
                    "consistent",
                    Json::Bool(consistent.get(&watch_id).copied().unwrap_or(false)),
                ),
                ("values", Json::Array(values)),
            ]);
            let event_warnings = if first {
                first = false;
                warnings.clone()
            } else {
                Vec::new()
            };
            self.send_event(connection, "watch", data, event_warnings);
        }
    }

    fn emit_queue(&mut self, connection: &mut Connection, result: DebugQueueResult) {
        let mut data = BTreeMap::new();
        data.insert("queue_id".into(), Json::unsigned(result.queue_id.0));
        data.insert("operation".into(), Json::string(result.operation.name()));
        data.insert("outcome".into(), Json::string(result.outcome.name()));
        data.insert("cycle".into(), Json::unsigned(result.cycle));
        if let Some(generation) = result.generation {
            data.insert("generation".into(), Json::unsigned(generation));
        }
        if let Some(build) = result.build_id {
            data.insert("build_id".into(), Json::string(build.to_hex()));
        }
        if let Some(code) = result.code {
            data.insert("code".into(), Json::string(code));
        }
        if let Some(message) = result.message {
            data.insert("message".into(), Json::string(message));
        }
        let warnings = result.warning.map_or_else(Vec::new, |warning| {
            vec![warning_json(warning.code, warning.message, Json::Null)]
        });
        self.send_event(connection, "queue-result", Json::Object(data), warnings);
    }
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

/// Constant-time comparison of the endpoint token.
///
/// The comparison always visits all 64 stored bytes and the validity checks
/// are accumulated rather than short-circuited, so a wrong token is not
/// distinguishable by an early return. Length and hex validity are folded into
/// the same result.
fn token_matches(stored: &str, candidate: &str) -> bool {
    let stored = stored.as_bytes();
    let candidate = candidate.as_bytes();
    let mut difference = 0_u8;
    let mut valid = u8::from(stored.len() == 64);
    for index in 0..64 {
        let byte = candidate.get(index).copied().unwrap_or(0);
        let expected = stored.get(index).copied().unwrap_or(0);
        difference |= byte ^ expected;
        valid &= u8::from(byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    }
    valid &= u8::from(candidate.len() == 64);
    !std::hint::black_box(difference != 0) && std::hint::black_box(valid) != 0
}

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

fn json_object<const N: usize>(pairs: [(&str, Json); N]) -> Json {
    Json::object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value)),
    )
}

fn has_unknown_keys(object: &BTreeMap<String, Json>, allowed: &[&str]) -> bool {
    object.keys().any(|key| !allowed.contains(&key.as_str()))
}

fn warning_json(code: &str, message: String, detail: Json) -> Json {
    let mut object = BTreeMap::new();
    object.insert("code".into(), Json::string(code));
    object.insert("message".into(), Json::string(message));
    if !detail.is_null() {
        object.insert("detail".into(), detail);
    }
    Json::Object(object)
}

fn target_json(target: &DebugTarget) -> Json {
    match target {
        DebugTarget::Path(path) => json_object([
            ("kind", Json::string("path")),
            ("key", Json::string(path.clone())),
        ]),
        DebugTarget::GotId(got_id) => json_object([
            ("kind", Json::string("got_id")),
            ("got_id", Json::string(got_id.to_hex())),
        ]),
        DebugTarget::Local { frame, path } => json_object([
            ("kind", Json::string("local")),
            ("frame", Json::string(frame.clone())),
            ("name", Json::string(path.clone())),
        ]),
    }
}

fn capabilities(snapshot: &DebugSnapshot, scopes: Vec<Json>) -> (Json, Vec<Json>) {
    let mut warnings = Vec::new();
    // Monitoring-only is the release and hook-free case (no checkpoint
    // table). A hook-emitting build whose frame sidecar is missing keeps
    // breakpoints and pause; only stepping and stack/frame reads degrade.
    if !snapshot.hooks || !snapshot.checkpoints {
        let mut detail = BTreeMap::new();
        if let Some(error) = &snapshot.metadata_error {
            detail.insert("code".into(), Json::string(codes::DEBUG_METADATA));
            detail.insert("message".into(), Json::string(error.clone()));
        } else {
            detail.insert(
                "capability".into(),
                Json::string(if !snapshot.checkpoints {
                    "checkpoints"
                } else {
                    "hooks"
                }),
            );
        }
        warnings.push(warning_json(
            codes::W_DEBUG_MONITORING_ONLY,
            "debug features are limited to typed monitoring in this build".into(),
            Json::Object(detail),
        ));
    } else if !snapshot.frames {
        let mut detail = BTreeMap::new();
        if let Some(error) = &snapshot.frames_error {
            detail.insert("reason".into(), Json::string(error.clone()));
        } else {
            detail.insert("capability".into(), Json::string("frames"));
        }
        warnings.push(warning_json(
            codes::W_DEBUG_UNAVAILABLE,
            "stopped-frame information is unavailable; stack and local reads will fail".into(),
            Json::Object(detail),
        ));
    }
    let result = json_object([
        ("scopes", Json::Array(scopes)),
        ("hooks", Json::Bool(snapshot.hooks)),
        ("debug_info", Json::string(snapshot.debug_info.clone())),
        ("checkpoints", Json::Bool(snapshot.checkpoints)),
        ("stepping", Json::Bool(snapshot.stepping)),
        ("max_watch_points", Json::unsigned(MAX_WATCHES as u64)),
        ("max_breakpoints", Json::unsigned(1024)),
    ]);
    (result, warnings)
}

// ---------------------------------------------------------------------------
// Operation dispatch
// ---------------------------------------------------------------------------

fn known_operation(op: &str) -> bool {
    matches!(
        op,
        "session.start"
            | "session.status"
            | "session.stop"
            | "var.read"
            | "var.write"
            | "var.force"
            | "var.release"
            | "var.release-all"
            | "watch.add"
            | "watch.remove"
            | "watch.list"
            | "breakpoint.set"
            | "breakpoint.clear"
            | "breakpoint.list"
            | "exec.continue"
            | "exec.step"
            | "exec.pause"
            | "stack.list"
            | "frame.read"
            | "manifest.get"
    )
}

fn required_scope(op: &str) -> Option<Scope> {
    match op {
        "var.read" | "watch.add" | "watch.remove" | "watch.list" | "stack.list" | "frame.read"
        | "manifest.get" => Some(Scope::Observe),
        "breakpoint.set" | "breakpoint.clear" | "breakpoint.list" | "exec.continue"
        | "exec.step" | "exec.pause" => Some(Scope::Control),
        "var.write" => Some(Scope::Write),
        "var.force" | "var.release" | "var.release-all" => Some(Scope::Force),
        _ => None,
    }
}

impl Handler {
    fn execute(
        &mut self,
        op: &str,
        params: &BTreeMap<String, Json>,
        deadline: Instant,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        match op {
            "session.status" => {
                check_keys(params, &[])?;
                Ok((self.status_result(), Vec::new()))
            }
            "var.read" => self.var_read(params, deadline),
            "var.write" => self.var_write(DebugWriteKind::Write, params, deadline),
            "var.force" => self.var_write(DebugWriteKind::Force, params, deadline),
            "var.release" => self.var_release(params, deadline),
            "var.release-all" => self.var_release_all(params, deadline),
            "watch.add" => self.watch_add(params, deadline),
            "watch.remove" => self.watch_remove(params, deadline),
            "watch.list" => self.watch_list(params, deadline),
            "manifest.get" => self.manifest_get(params, deadline),
            "breakpoint.set" => self.breakpoint_set(params),
            "breakpoint.clear" => self.breakpoint_clear(params),
            "breakpoint.list" => self.breakpoint_list(params),
            "exec.continue" => self.exec_continue(params),
            "exec.step" => self.exec_step(params),
            "exec.pause" => self.exec_pause(params),
            "stack.list" => self.stack_list(params),
            "frame.read" => self.frame_read(params),
            _ => Err(ProtocolError::malformed(
                "the operation is not defined by stc-debug/1",
            )),
        }
    }

    fn status_result(&self) -> Json {
        let snapshot = self.shared.snapshot();
        let status = self.session_stop.as_ref().map(|session| session.status());
        let stop = status.as_ref().and_then(|status| status.stop.as_ref());
        let (generation, build_id, cycle) = match (stop, self.session_stop.as_ref()) {
            (Some(stop), Some(session)) => (session.generation(), session.build_id(), stop.cycle),
            _ => self.shared.tags(),
        };
        let mut object = BTreeMap::new();
        object.insert(
            "state".into(),
            Json::string(if stop.is_some() { "stopped" } else { "active" }),
        );
        object.insert(
            "session".into(),
            Json::string(
                self.session
                    .as_ref()
                    .map_or_else(String::new, |view| view.id.clone()),
            ),
        );
        object.insert("generation".into(), Json::unsigned(generation));
        object.insert("build_id".into(), Json::string(build_id.to_hex()));
        object.insert("cycle".into(), Json::unsigned(cycle));
        object.insert(
            "stop".into(),
            stop.map_or(Json::Null, |stop| {
                let mut data = BTreeMap::new();
                data.insert("stop_id".into(), Json::unsigned(stop.stop_id));
                data.insert("reason".into(), Json::string(stop.reason.name()));
                data.insert(
                    "checkpoint_id".into(),
                    Json::unsigned(u64::from(stop.checkpoint_id)),
                );
                data.insert("source".into(), source_json(&stop.source));
                if let Some(breakpoint_id) = stop.breakpoint_id {
                    data.insert("breakpoint_id".into(), Json::unsigned(breakpoint_id));
                }
                Json::Object(data)
            }),
        );
        object.insert(
            "pause_remaining_ms".into(),
            status
                .as_ref()
                .and_then(|status| status.pause_remaining_ms)
                .map_or(Json::Null, Json::unsigned),
        );
        object.insert(
            "queued_writes".into(),
            Json::unsigned(
                snapshot
                    .as_ref()
                    .map_or(0, |snapshot| snapshot.queued_writes)
                    .try_into()
                    .unwrap_or(u64::MAX),
            ),
        );
        object.insert(
            "active_forces".into(),
            Json::unsigned(
                snapshot
                    .as_ref()
                    .map_or(0, |snapshot| snapshot.active_forces)
                    .try_into()
                    .unwrap_or(u64::MAX),
            ),
        );
        object.insert(
            "watches".into(),
            Json::unsigned(
                snapshot
                    .as_ref()
                    .map_or(0, |snapshot| snapshot.watches)
                    .try_into()
                    .unwrap_or(u64::MAX),
            ),
        );
        let breakpoints = status.as_ref().map_or_else(
            || snapshot.as_ref().map_or(0, |snapshot| snapshot.breakpoints),
            |status| status.breakpoints,
        );
        object.insert(
            "breakpoints".into(),
            Json::unsigned(breakpoints.try_into().unwrap_or(u64::MAX)),
        );
        Json::Object(object)
    }
}

// ---------------------------------------------------------------------------
// Breakpoints, stops, stepping, and stopped frames (§16.7–§16.9)
// ---------------------------------------------------------------------------

impl Handler {
    /// Returns the installed stop session, or the documented unavailable code.
    fn stop_session(&self) -> std::result::Result<Arc<SessionStop>, ProtocolError> {
        if let Some(session) = &self.session_stop {
            if session.ending() {
                return Err(ProtocolError::new(
                    codes::DEBUG_SESSION,
                    "the debug session is ending; reconnect after teardown",
                ));
            }
            return Ok(Arc::clone(session));
        }
        if self
            .shared
            .snapshot()
            .is_some_and(|snapshot| snapshot.metadata_error.is_some())
        {
            return Err(ProtocolError::new(
                codes::DEBUG_METADATA,
                "the checkpoint table is invalid; breakpoints and stepping are unavailable",
            ));
        }
        Err(ProtocolError::new(
            codes::DEBUG_NO_HOOKS,
            "breakpoints, stops, and stepping require an artifact with debug hooks and a valid checkpoint table",
        )
        .with_detail(json_object([(
            "warning",
            Json::string(codes::W_DEBUG_MONITORING_ONLY),
        )])))
    }

    /// Returns the installed stop session for a stack or local read.
    fn frame_session(&self) -> std::result::Result<Arc<SessionStop>, ProtocolError> {
        if let Some(session) = &self.session_stop {
            if session.ending() {
                return Err(ProtocolError::new(
                    codes::DEBUG_SESSION,
                    "the debug session is ending; reconnect after teardown",
                ));
            }
            return Ok(Arc::clone(session));
        }
        if self
            .shared
            .snapshot()
            .is_some_and(|snapshot| snapshot.metadata_error.is_some())
        {
            return Err(ProtocolError::new(
                codes::DEBUG_METADATA,
                "the checkpoint table is invalid; stack and local reads are unavailable",
            ));
        }
        Err(ProtocolError::new(
            codes::DEBUG_INFO,
            "source, stack, and local debug information is unavailable for this artifact",
        ))
    }

    fn audit_breakpoint(&self, operation: &'static str, info: &BreakpointInfo) -> Result<()> {
        let (generation, build_id, cycle) = self.shared.tags();
        let mut event = AuditEvent::new(operation, AuditOutcome::Ok);
        event.session = self.session.as_ref().map(|view| view.id.clone());
        event.scope = Some("control".into());
        event.generation = Some(generation);
        event.build = Some(build_id);
        event.cycle = Some(cycle);
        event.target = Some(match &info.target {
            BreakpointTarget::Source { file, .. } => AuditTarget {
                key: file.clone(),
                kind: "source".into(),
            },
            BreakpointTarget::Checkpoint { checkpoint_id } => AuditTarget {
                key: checkpoint_id.to_string(),
                kind: "checkpoint".into(),
            },
        });
        event.detail = json_object([
            ("bound", Json::Bool(info.bound)),
            (
                "checkpoint_ids",
                Json::Array(
                    info.checkpoints
                        .iter()
                        .map(|id| Json::unsigned(u64::from(*id)))
                        .collect(),
                ),
            ),
            (
                "source",
                info.source.as_ref().map_or(Json::Null, source_json),
            ),
        ]);
        self.shared.audit(event)
    }

    fn breakpoint_set(
        &mut self,
        params: &BTreeMap<String, Json>,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        let session = self.stop_session()?;
        check_keys(params, &["target"])?;
        let target = parse_breakpoint_target(required(params, "target")?)?;
        let info = session
            .set_breakpoint(target)
            .map_err(ProtocolError::from_stc)?;
        if let Err(error) = self.audit_breakpoint("breakpoint-set", &info) {
            // The audit is part of the modification contract; refuse and
            // remove the breakpoint that was just installed.
            let _ = session.clear_breakpoint(info.id);
            return Err(ProtocolError::from_stc(error));
        }
        let warnings = if info.bound {
            Vec::new()
        } else {
            let mut detail = BTreeMap::new();
            if let Some(source) = &info.source {
                detail.insert("file".into(), Json::string(source.file.clone()));
                detail.insert("line".into(), Json::unsigned(u64::from(source.line)));
                detail.insert("column".into(), Json::unsigned(u64::from(source.column)));
            }
            vec![warning_json(
                codes::W_DEBUG_UNBOUND,
                "no checkpoint exists at the requested source location; the breakpoint is not moved"
                    .into(),
                Json::Object(detail),
            )]
        };
        Ok((breakpoint_json(&info), warnings))
    }

    fn breakpoint_clear(
        &mut self,
        params: &BTreeMap<String, Json>,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        let session = self.stop_session()?;
        check_keys(params, &["breakpoint_id"])?;
        let breakpoint_id = required_positive(params, "breakpoint_id")?;
        let Some(info) = session.breakpoint(breakpoint_id) else {
            return Err(ProtocolError::new(
                codes::DEBUG_UNKNOWN,
                format!("breakpoint {breakpoint_id} is unknown"),
            ));
        };
        self.audit_breakpoint("breakpoint-clear", &info)
            .map_err(ProtocolError::from_stc)?;
        if !session.clear_breakpoint(breakpoint_id) {
            return Err(ProtocolError::new(
                codes::DEBUG_UNKNOWN,
                format!("breakpoint {breakpoint_id} is unknown"),
            ));
        }
        Ok((object([("cleared", Json::Bool(true))]), Vec::new()))
    }

    fn breakpoint_list(
        &mut self,
        params: &BTreeMap<String, Json>,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        let session = self.stop_session()?;
        check_keys(params, &[])?;
        let breakpoints = session.breakpoints();
        let mut warnings = Vec::new();
        if breakpoints.iter().any(|info| !info.bound) {
            warnings.push(warning_json(
                codes::W_DEBUG_UNBOUND,
                "one or more breakpoints have no checkpoint at their source location".into(),
                Json::Null,
            ));
        }
        Ok((
            object([(
                "breakpoints",
                Json::Array(breakpoints.iter().map(breakpoint_json).collect()),
            )]),
            warnings,
        ))
    }

    fn exec_continue(
        &mut self,
        params: &BTreeMap<String, Json>,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        let session = self.stop_session()?;
        check_keys(params, &["stop_id"])?;
        let stop_id = optional_positive(params, "stop_id")?;
        let previous = session
            .continue_execution(stop_id)
            .map_err(ProtocolError::from_stc)?;
        Ok((
            object([
                ("status", Json::string("running")),
                ("stop_id", Json::unsigned(previous)),
            ]),
            Vec::new(),
        ))
    }

    fn exec_step(
        &mut self,
        params: &BTreeMap<String, Json>,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        let session = self.stop_session()?;
        check_keys(params, &["mode", "stop_id"])?;
        let mode_name = required(params, "mode")?
            .as_str()
            .ok_or_else(|| ProtocolError::malformed("mode must be a string"))?;
        let Some(mode) = StepMode::parse(mode_name) else {
            return Err(ProtocolError::new(
                codes::DEBUG_STOP,
                "the step mode must be `over`, `into`, or `out`",
            ));
        };
        let stop_id = required_positive(params, "stop_id")?;
        let previous = session
            .step(mode, stop_id)
            .map_err(ProtocolError::from_stc)?;
        Ok((
            object([
                ("status", Json::string("stepping")),
                ("stop_id", Json::unsigned(previous)),
                ("mode", Json::string(mode.name())),
            ]),
            Vec::new(),
        ))
    }

    fn exec_pause(
        &mut self,
        params: &BTreeMap<String, Json>,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        let session = self.stop_session()?;
        check_keys(params, &[])?;
        match session.pause().map_err(ProtocolError::from_stc)? {
            PauseOutcome::Requested => Ok((
                object([("status", Json::string("pause-requested"))]),
                Vec::new(),
            )),
            PauseOutcome::Stopped(stop_id) => Ok((
                object([
                    ("status", Json::string("stopped")),
                    ("stop_id", Json::unsigned(stop_id)),
                ]),
                Vec::new(),
            )),
        }
    }

    fn stack_list(
        &mut self,
        params: &BTreeMap<String, Json>,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        let session = self.frame_session()?;
        check_keys(params, &["stop_id"])?;
        let stop_id = required_positive(params, "stop_id")?;
        let (frames, truncated) = session.stack(stop_id).map_err(ProtocolError::from_stc)?;
        let mut warnings = Vec::new();
        if frames.iter().any(|frame| frame.optimized_out) {
            warnings.push(warning_json(
                codes::W_DEBUG_UNAVAILABLE,
                "one or more frames could not be unwound and are reported as optimized out".into(),
                Json::Null,
            ));
        }
        let frames = frames
            .iter()
            .map(|frame| {
                let mut data = BTreeMap::new();
                data.insert("index".into(), Json::unsigned(frame.index as u64));
                data.insert(
                    "function".into(),
                    frame
                        .function
                        .map_or(Json::Null, |id| Json::string(id.to_hex())),
                );
                data.insert(
                    "symbol".into(),
                    frame
                        .symbol
                        .as_ref()
                        .map_or(Json::Null, |symbol| Json::string(symbol.clone())),
                );
                data.insert(
                    "source".into(),
                    frame.source.as_ref().map_or(Json::Null, source_json),
                );
                data.insert("optimized_out".into(), Json::Bool(frame.optimized_out));
                Json::Object(data)
            })
            .collect();
        Ok((
            object([
                ("frames", Json::Array(frames)),
                ("truncated", Json::Bool(truncated)),
            ]),
            warnings,
        ))
    }

    fn frame_read(
        &mut self,
        params: &BTreeMap<String, Json>,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        let session = self.frame_session()?;
        check_keys(params, &["target", "stop_id"])?;
        let target = parse_target(required(params, "target")?, true)?;
        let DebugTarget::Local { frame, path } = target else {
            return Err(ProtocolError::malformed(
                "frame.read requires a local target",
            ));
        };
        let frame = frame.parse::<usize>().map_err(|_| {
            ProtocolError::malformed("the frame index is not a non-negative integer")
        })?;
        let name = path.to_ascii_uppercase();
        if name.is_empty() {
            return Err(ProtocolError::malformed("a local name must not be empty"));
        }
        let stop_id = required_positive(params, "stop_id")?;
        let view = session
            .read_local(stop_id, frame, &name)
            .map_err(ProtocolError::from_stc)?;
        let warning = match &view.value {
            DebugValue::Unavailable { warning } => Some(warning.clone()),
            _ => None,
        };
        let typed = crate::debug::TypedValue {
            type_key: view.type_key,
            value: view.value,
        };
        let mut data = BTreeMap::new();
        data.insert("frame".into(), Json::unsigned(frame as u64));
        data.insert("name".into(), Json::string(name));
        data.insert("stop_id".into(), Json::unsigned(stop_id));
        data.insert(
            "available".into(),
            Json::Bool(!matches!(&typed.value, DebugValue::Unavailable { .. })),
        );
        data.insert("value".into(), typed_value_json(&typed));
        data.insert("generation".into(), Json::unsigned(view.generation));
        data.insert("build_id".into(), Json::string(view.build_id.to_hex()));
        data.insert("cycle".into(), Json::unsigned(view.cycle));
        data.insert("view".into(), view_json(&view.view));
        let warnings = warning.map_or_else(Vec::new, |warning: DebugWarning| {
            vec![warning_json(warning.code, warning.message, Json::Null)]
        });
        Ok((Json::Object(data), warnings))
    }

    fn drain_stop_events(&mut self, connection: &mut Connection) {
        if self.session.is_none() {
            return;
        }
        let Some(session) = self.session_stop.clone() else {
            return;
        };
        for event in session.drain_events() {
            let was_step = event.is_step();
            match event {
                StopEvent::Stopped {
                    stop_id,
                    reason,
                    checkpoint_id,
                    source,
                    breakpoint_id,
                    generation,
                    build_id,
                    cycle,
                } => {
                    let mut data = BTreeMap::new();
                    data.insert("stop_id".into(), Json::unsigned(stop_id));
                    data.insert("reason".into(), Json::string(reason.name()));
                    data.insert(
                        "checkpoint_id".into(),
                        Json::unsigned(u64::from(checkpoint_id)),
                    );
                    data.insert("source".into(), source_json(&source));
                    data.insert("generation".into(), Json::unsigned(generation));
                    data.insert("build_id".into(), Json::string(build_id.to_hex()));
                    data.insert("cycle".into(), Json::unsigned(cycle));
                    if let Some(breakpoint_id) = breakpoint_id {
                        data.insert("breakpoint_id".into(), Json::unsigned(breakpoint_id));
                    }
                    self.send_event(connection, "stopped", Json::Object(data), Vec::new());
                    let mut audit = AuditEvent::new("stop", AuditOutcome::Ok);
                    audit.session = self.session.as_ref().map(|view| view.id.clone());
                    audit.generation = Some(generation);
                    audit.build = Some(build_id);
                    audit.cycle = Some(cycle);
                    audit.detail = json_object([
                        ("reason", Json::string(reason.name())),
                        ("checkpoint_id", Json::unsigned(u64::from(checkpoint_id))),
                        ("source", source_json(&source)),
                        (
                            "breakpoint_id",
                            breakpoint_id.map_or(Json::Null, Json::unsigned),
                        ),
                    ]);
                    if self.shared.audit(audit).is_err() {
                        self.close = Some(ClosePlan::new(
                            "internal",
                            Some(codes::DEBUG_AUDIT),
                            true,
                            true,
                        ));
                        return;
                    }
                }
                StopEvent::Resumed {
                    stop_id,
                    cause: _,
                    cycle,
                } => {
                    let data = object([
                        ("stop_id", Json::unsigned(stop_id)),
                        ("cycle", Json::unsigned(cycle)),
                    ]);
                    self.send_event(connection, "resumed", data, Vec::new());
                    let name = if was_step { "step" } else { "resume" };
                    let mut audit = AuditEvent::new(name, AuditOutcome::Ok);
                    audit.session = self.session.as_ref().map(|view| view.id.clone());
                    audit.cycle = Some(cycle);
                    audit.detail = object([("stop_id", Json::unsigned(stop_id))]);
                    if self.shared.audit(audit).is_err() {
                        self.close = Some(ClosePlan::new(
                            "internal",
                            Some(codes::DEBUG_AUDIT),
                            true,
                            true,
                        ));
                        return;
                    }
                }
            }
            self.shared.touch(self.connection, Instant::now());
        }
    }
}

fn source_json(source: &SourcePosition) -> Json {
    object([
        ("file", Json::string(source.file.clone())),
        ("line", Json::unsigned(u64::from(source.line))),
        ("column", Json::unsigned(u64::from(source.column))),
    ])
}

fn breakpoint_json(info: &BreakpointInfo) -> Json {
    let mut data = BTreeMap::new();
    data.insert("breakpoint_id".into(), Json::unsigned(info.id));
    data.insert("bound".into(), Json::Bool(info.bound));
    data.insert(
        "checkpoints".into(),
        Json::Array(
            info.checkpoints
                .iter()
                .map(|id| Json::unsigned(u64::from(*id)))
                .collect(),
        ),
    );
    data.insert(
        "source".into(),
        info.source.as_ref().map_or(Json::Null, source_json),
    );
    data.insert("target".into(), breakpoint_target_json(&info.target));
    Json::Object(data)
}

fn breakpoint_target_json(target: &BreakpointTarget) -> Json {
    match target {
        BreakpointTarget::Source { file, line, column } => object([
            ("kind", Json::string("source")),
            ("file", Json::string(file.clone())),
            ("line", Json::unsigned(u64::from(*line))),
            ("column", Json::unsigned(u64::from(*column))),
        ]),
        BreakpointTarget::Checkpoint { checkpoint_id } => object([
            ("kind", Json::string("checkpoint")),
            ("checkpoint_id", Json::unsigned(u64::from(*checkpoint_id))),
        ]),
    }
}

fn parse_breakpoint_target(value: &Json) -> std::result::Result<BreakpointTarget, ProtocolError> {
    let object = value
        .as_object()
        .ok_or_else(|| ProtocolError::malformed("a breakpoint target must be an object"))?;
    let kind = object
        .get("kind")
        .and_then(Json::as_str)
        .ok_or_else(|| ProtocolError::malformed("a breakpoint target requires a kind"))?;
    match kind {
        "source" => {
            if has_unknown_keys(object, &["kind", "file", "line", "column"]) {
                return Err(ProtocolError::malformed(
                    "an unknown breakpoint target key is invalid",
                ));
            }
            let file = object
                .get("file")
                .and_then(Json::as_str)
                .ok_or_else(|| ProtocolError::malformed("a source breakpoint requires file"))?;
            if !crate::frames::valid_source_file(file) {
                return Err(ProtocolError::malformed(
                    "the source file must be a relative path without control characters",
                ));
            }
            let line = object
                .get("line")
                .and_then(Json::as_number)
                .filter(|line| (1..=i128::from(u32::MAX)).contains(line))
                .and_then(|line| u32::try_from(line).ok())
                .ok_or_else(|| {
                    ProtocolError::malformed("a source breakpoint line must be a positive integer")
                })?;
            let column = match object.get("column") {
                None => 0,
                Some(value) => value
                    .as_number()
                    .filter(|column| (0..=i128::from(u32::MAX)).contains(column))
                    .and_then(|column| u32::try_from(column).ok())
                    .ok_or_else(|| {
                        ProtocolError::malformed(
                            "a source breakpoint column must be a non-negative integer",
                        )
                    })?,
            };
            Ok(BreakpointTarget::Source {
                file: file.to_owned(),
                line,
                column,
            })
        }
        "checkpoint" => {
            if has_unknown_keys(object, &["kind", "checkpoint_id"]) {
                return Err(ProtocolError::malformed(
                    "an unknown breakpoint target key is invalid",
                ));
            }
            let checkpoint_id = object
                .get("checkpoint_id")
                .and_then(Json::as_number)
                .filter(|id| (1..=i128::from(u32::MAX)).contains(id))
                .and_then(|id| u32::try_from(id).ok())
                .ok_or_else(|| {
                    ProtocolError::malformed(
                        "a checkpoint breakpoint requires a positive checkpoint_id",
                    )
                })?;
            Ok(BreakpointTarget::Checkpoint { checkpoint_id })
        }
        _ => Err(ProtocolError::malformed(
            "a breakpoint target kind must be `source` or `checkpoint`",
        )),
    }
}

fn required_positive(
    params: &BTreeMap<String, Json>,
    key: &str,
) -> std::result::Result<u64, ProtocolError> {
    optional_positive(params, key)?
        .ok_or_else(|| ProtocolError::malformed(format!("{key} is required")))
}

fn optional_positive(
    params: &BTreeMap<String, Json>,
    key: &str,
) -> std::result::Result<Option<u64>, ProtocolError> {
    match optional_u64(params, key)? {
        Some(0) => Err(ProtocolError::malformed(format!(
            "{key} must be a positive integer"
        ))),
        value => Ok(value),
    }
}

fn check_keys(
    params: &BTreeMap<String, Json>,
    allowed: &[&str],
) -> std::result::Result<(), ProtocolError> {
    if has_unknown_keys(params, allowed) {
        return Err(ProtocolError::malformed(
            "a field for another operation is invalid",
        ));
    }
    Ok(())
}

fn required<'a>(
    params: &'a BTreeMap<String, Json>,
    key: &str,
) -> std::result::Result<&'a Json, ProtocolError> {
    params
        .get(key)
        .ok_or_else(|| ProtocolError::malformed(format!("{key} is required")))
}

fn optional_u64(
    params: &BTreeMap<String, Json>,
    key: &str,
) -> std::result::Result<Option<u64>, ProtocolError> {
    match params.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_number()
            .filter(|value| *value >= 0)
            .and_then(|value| u64::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| ProtocolError::malformed(format!("{key} must be an unsigned integer"))),
    }
}

fn optional_got_id(
    params: &BTreeMap<String, Json>,
    key: &str,
) -> std::result::Result<Option<GotId>, ProtocolError> {
    match params.get(key) {
        None => Ok(None),
        Some(value) => {
            let text = value
                .as_str()
                .ok_or_else(|| ProtocolError::malformed(format!("{key} must be a string")))?;
            GotId::parse(text)
                .map(Some)
                .map_err(|_| ProtocolError::malformed(format!("{key} is not a canonical identity")))
        }
    }
}

fn optional_string(
    params: &BTreeMap<String, Json>,
    key: &str,
) -> std::result::Result<Option<String>, ProtocolError> {
    match params.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(|text| Some(text.to_owned()))
            .ok_or_else(|| ProtocolError::malformed(format!("{key} must be a string"))),
    }
}

fn parse_target(
    value: &Json,
    allow_local: bool,
) -> std::result::Result<DebugTarget, ProtocolError> {
    let object = value
        .as_object()
        .ok_or_else(|| ProtocolError::malformed("a target must be an object"))?;
    let kind = object
        .get("kind")
        .and_then(Json::as_str)
        .ok_or_else(|| ProtocolError::malformed("a target requires a kind"))?;
    match kind {
        "got_id" => {
            if has_unknown_keys(object, &["kind", "got_id"]) {
                return Err(ProtocolError::malformed("an unknown target key is invalid"));
            }
            let text = object
                .get("got_id")
                .and_then(Json::as_str)
                .ok_or_else(|| ProtocolError::malformed("got_id target requires got_id"))?;
            let got_id = GotId::parse(text)
                .map_err(|_| ProtocolError::malformed("got_id is not a canonical identity"))?;
            Ok(DebugTarget::GotId(got_id))
        }
        "path" => {
            if has_unknown_keys(object, &["kind", "key"]) {
                return Err(ProtocolError::malformed("an unknown target key is invalid"));
            }
            let key = object
                .get("key")
                .and_then(Json::as_str)
                .ok_or_else(|| ProtocolError::malformed("path target requires key"))?;
            Ok(DebugTarget::Path(key.to_owned()))
        }
        "local" if allow_local => {
            if has_unknown_keys(object, &["kind", "frame", "name"]) {
                return Err(ProtocolError::malformed("an unknown target key is invalid"));
            }
            let frame = object
                .get("frame")
                .and_then(Json::as_number)
                .ok_or_else(|| ProtocolError::malformed("local target requires a frame number"))?;
            let name = object
                .get("name")
                .and_then(Json::as_str)
                .ok_or_else(|| ProtocolError::malformed("local target requires a name"))?;
            Ok(DebugTarget::Local {
                frame: frame.to_string(),
                path: name.to_owned(),
            })
        }
        "local" => Err(ProtocolError::malformed(
            "a local target is valid only for frame.read",
        )),
        _ => Err(ProtocolError::malformed(
            "an unknown target kind is invalid",
        )),
    }
}

fn parse_read(
    params: &BTreeMap<String, Json>,
) -> std::result::Result<DebugReadRequest, ProtocolError> {
    check_keys(
        params,
        &[
            "target",
            "expected_generation",
            "expected_build_id",
            "range",
        ],
    )?;
    let target = parse_target(required(params, "target")?, false)?;
    let mut request = DebugReadRequest::new(target);
    if let Some(generation) = optional_u64(params, "expected_generation")? {
        request = request.with_generation(generation);
    }
    if let Some(build) = optional_got_id(params, "expected_build_id")? {
        request = request.with_build(build);
    }
    Ok(request)
}

fn parse_range(
    params: &BTreeMap<String, Json>,
) -> std::result::Result<Option<(usize, usize)>, ProtocolError> {
    let Some(value) = params.get("range") else {
        return Ok(None);
    };
    let object = value
        .as_object()
        .ok_or_else(|| ProtocolError::malformed("range must be an object"))?;
    if has_unknown_keys(object, &["start", "count"]) {
        return Err(ProtocolError::malformed("an unknown range key is invalid"));
    }
    let start = object
        .get("start")
        .and_then(Json::as_number)
        .filter(|value| *value >= 0)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| ProtocolError::malformed("range.start must be an unsigned integer"))?;
    let count = object
        .get("count")
        .and_then(Json::as_number)
        .filter(|value| *value >= 0)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| ProtocolError::malformed("range.count must be an unsigned integer"))?;
    start
        .checked_add(count)
        .ok_or_else(|| ProtocolError::malformed("range.start + range.count overflows"))?;
    Ok(Some((start, count)))
}

fn parse_write(
    params: &BTreeMap<String, Json>,
) -> std::result::Result<DebugWriteRequest, ProtocolError> {
    check_keys(
        params,
        &[
            "target",
            "value",
            "expected_generation",
            "expected_build_id",
            "expected_type",
            "expected_layout",
        ],
    )?;
    let target = parse_target(required(params, "target")?, false)?;
    let value =
        typed_value_from_json(required(params, "value")?).map_err(ProtocolError::from_stc)?;
    let mut request = DebugWriteRequest::new(target, value);
    if let Some(generation) = optional_u64(params, "expected_generation")? {
        request = request.with_generation(generation);
    }
    if let Some(build) = optional_got_id(params, "expected_build_id")? {
        request = request.with_build(build);
    }
    if let Some(type_key) = optional_string(params, "expected_type")? {
        request = request.with_type_key(type_key);
    }
    if let Some(layout) = optional_got_id(params, "expected_layout")? {
        request = request.with_layout_hash(layout);
    }
    Ok(request)
}

fn wait_reply<T>(
    reply: &crate::debug::Reply<T>,
    deadline: Instant,
) -> std::result::Result<T, ProtocolError> {
    reply.wait(deadline).ok_or_else(|| {
        ProtocolError::new(
            codes::DEBUG_TIMEOUT,
            "the request did not complete within the processing deadline",
        )
    })
}

fn failure_to_error(failure: crate::debug::DebugFailure) -> ProtocolError {
    ProtocolError::new(failure.code, failure.message)
}

fn slice_value(
    value: &DebugValue,
    start: usize,
    count: usize,
) -> std::result::Result<DebugValue, ProtocolError> {
    let end = start
        .checked_add(count)
        .ok_or_else(|| ProtocolError::new(codes::DEBUG_TYPE, "the requested range overflows"))?;
    match value {
        DebugValue::String(string) => {
            if end > string.units.len() {
                return Err(ProtocolError::new(
                    codes::DEBUG_TYPE,
                    "the requested range exceeds the string value",
                ));
            }
            let units = string.units[start..end].to_vec();
            let text = units
                .iter()
                .map(|unit| char::from_u32(*unit))
                .collect::<Option<String>>();
            Ok(DebugValue::String(crate::debug::StringValue {
                capacity: string.capacity,
                length: u32::try_from(units.len()).unwrap_or(u32::MAX),
                units,
                text,
            }))
        }
        DebugValue::Array(elements) => {
            if end > elements.len() {
                return Err(ProtocolError::new(
                    codes::DEBUG_TYPE,
                    "the requested range exceeds the array value",
                ));
            }
            Ok(DebugValue::Array(elements[start..end].to_vec()))
        }
        _ => Err(ProtocolError::new(
            codes::DEBUG_TYPE,
            "a range is valid only for a string or single-root array",
        )),
    }
}

fn read_result(read: &DebugRead, value: DebugValue, stopped: bool, include_view: bool) -> Json {
    let mut object = BTreeMap::new();
    object.insert("value".into(), {
        let typed = crate::debug::TypedValue {
            type_key: read.type_key.clone(),
            value,
        };
        typed_value_json(&typed)
    });
    object.insert("generation".into(), Json::unsigned(read.generation));
    object.insert("build_id".into(), Json::string(read.build_id.to_hex()));
    object.insert("cycle".into(), Json::unsigned(read.cycle));
    object.insert("consistent".into(), Json::Bool(read.consistent));
    object.insert("atomic".into(), Json::Bool(read.atomic));
    object.insert("stopped".into(), Json::Bool(stopped));
    if include_view && let Some(view) = &read.view {
        object.insert("view".into(), view_json(view));
    }
    Json::Object(object)
}

impl Handler {
    fn var_read(
        &mut self,
        params: &BTreeMap<String, Json>,
        deadline: Instant,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        let request = parse_read(params)?;
        let range = parse_range(params)?;
        let request = if range.is_none() {
            request.with_view()
        } else {
            request
        };
        let reply = self
            .shared
            .service
            .submit_read_wait(request)
            .map_err(ProtocolError::from_stc)?;
        let read = wait_reply(&reply, deadline)?.map_err(failure_to_error)?;
        let value = match range {
            None => read.value.clone(),
            Some((start, count)) => slice_value(&read.value, start, count)?,
        };
        if read.protected {
            let mut event = AuditEvent::new("read-protected", AuditOutcome::Ok);
            event.scope = Some("observe".into());
            event.target = Some(AuditTarget {
                key: read.resolved_key.clone(),
                kind: "root".into(),
            });
            event.generation = Some(read.generation);
            event.build = Some(read.build_id);
            event.cycle = Some(read.cycle);
            self.shared.audit(event).map_err(ProtocolError::from_stc)?;
        }
        let stopped = self
            .session_stop
            .as_ref()
            .is_some_and(|session| session.status().stop.is_some());
        Ok((
            read_result(&read, value, stopped, range.is_none()),
            Vec::new(),
        ))
    }

    fn var_write(
        &mut self,
        kind: DebugWriteKind,
        params: &BTreeMap<String, Json>,
        deadline: Instant,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        let request = parse_write(params)?;
        let reply = self
            .shared
            .service
            .submit_write_wait(kind, request)
            .map_err(ProtocolError::from_stc)?;
        let queue_id = wait_reply(&reply, deadline)?.map_err(failure_to_error)?;
        Ok((
            object([
                ("status", Json::string("queued")),
                ("queue_id", Json::unsigned(queue_id.0)),
            ]),
            Vec::new(),
        ))
    }

    fn var_release(
        &mut self,
        params: &BTreeMap<String, Json>,
        deadline: Instant,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        check_keys(params, &["target"])?;
        let target = parse_target(required(params, "target")?, false)?;
        let reply = self
            .shared
            .service
            .submit_release_wait(target)
            .map_err(ProtocolError::from_stc)?;
        wait_reply(&reply, deadline)?.map_err(failure_to_error)?;
        Ok((
            object([
                ("released", Json::Bool(true)),
                ("effective", Json::string("next-boundary")),
            ]),
            Vec::new(),
        ))
    }

    fn var_release_all(
        &mut self,
        params: &BTreeMap<String, Json>,
        deadline: Instant,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        check_keys(params, &[])?;
        let reply = self
            .shared
            .service
            .submit_release_all_wait()
            .map_err(ProtocolError::from_stc)?;
        let released = wait_reply(&reply, deadline)?.map_err(failure_to_error)?;
        Ok((
            object([(
                "released",
                Json::unsigned(u64::try_from(released).unwrap_or(u64::MAX)),
            )]),
            Vec::new(),
        ))
    }

    fn watch_add(
        &mut self,
        params: &BTreeMap<String, Json>,
        deadline: Instant,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        check_keys(params, &["targets", "interval_ms"])?;
        let targets_json = required(params, "targets")?
            .as_array()
            .ok_or_else(|| ProtocolError::malformed("targets must be an array"))?;
        if targets_json.is_empty() {
            return Err(ProtocolError::malformed(
                "a watch needs at least one target",
            ));
        }
        if targets_json.len() > MAX_WATCHES {
            return Err(ProtocolError::new(
                codes::DEBUG_LIMIT,
                format!("a watch may have at most {MAX_WATCHES} targets"),
            ));
        }
        let mut targets = Vec::with_capacity(targets_json.len());
        for value in targets_json {
            targets.push(parse_target(value, false)?);
        }
        let interval_ms = required(params, "interval_ms")?
            .as_number()
            .ok_or_else(|| ProtocolError::malformed("interval_ms must be an integer"))?;
        let interval_ms = interval_ms.clamp(1, i128::from(i64::MAX / 1_000_000));
        let mut interval_ms = i64::try_from(interval_ms).unwrap_or(i64::MAX / 1_000_000);
        interval_ms = interval_ms.max(MIN_WATCH_INTERVAL_NS / 1_000_000);
        interval_ms = interval_ms.min(MAX_WATCH_INTERVAL_MS);
        let interval_ns = interval_ms.saturating_mul(1_000_000);
        let reply = self
            .shared
            .service
            .submit_watch_add_wait(targets, interval_ns)
            .map_err(ProtocolError::from_stc)?;
        let info = wait_reply(&reply, deadline)?.map_err(failure_to_error)?;
        if info.protected {
            let mut event = AuditEvent::new("read-protected", AuditOutcome::Ok);
            event.scope = Some("observe".into());
            event.detail = object([
                ("op", Json::string("watch.add")),
                ("watch_id", Json::unsigned(info.watch_id.0)),
                (
                    "resolved",
                    Json::Array(info.resolved.iter().cloned().map(Json::string).collect()),
                ),
            ]);
            if let Err(error) = self.shared.audit(event) {
                if let Ok(reply) = self.shared.service.submit_watch_remove_wait(info.watch_id) {
                    let _ = reply.wait(Instant::now() + REQUEST_DEADLINE);
                }
                return Err(ProtocolError::from_stc(error));
            }
        }
        Ok((
            watch_info_json(&info, info.interval_ns / 1_000_000),
            Vec::new(),
        ))
    }

    fn watch_remove(
        &mut self,
        params: &BTreeMap<String, Json>,
        deadline: Instant,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        check_keys(params, &["watch_id"])?;
        let watch_id = required(params, "watch_id")?
            .as_number()
            .filter(|value| *value > 0)
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| ProtocolError::malformed("watch_id must be a positive integer"))?;
        let reply = self
            .shared
            .service
            .submit_watch_remove_wait(WatchId(watch_id))
            .map_err(ProtocolError::from_stc)?;
        let Some(info) = wait_reply(&reply, deadline)? else {
            return Err(ProtocolError::new(
                codes::DEBUG_UNKNOWN,
                "the watch identity is unknown",
            ));
        };
        if info.protected {
            let mut event = AuditEvent::new("read-protected", AuditOutcome::Ok);
            event.scope = Some("observe".into());
            event.detail = object([
                ("op", Json::string("watch.remove")),
                ("watch_id", Json::unsigned(info.watch_id.0)),
                (
                    "resolved",
                    Json::Array(info.resolved.iter().cloned().map(Json::string).collect()),
                ),
            ]);
            self.shared.audit(event).map_err(ProtocolError::from_stc)?;
        }
        Ok((object([("removed", Json::Bool(true))]), Vec::new()))
    }

    fn watch_list(
        &mut self,
        params: &BTreeMap<String, Json>,
        deadline: Instant,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        check_keys(params, &[])?;
        let reply = self
            .shared
            .service
            .submit_watch_list_wait()
            .map_err(ProtocolError::from_stc)?;
        let watches = wait_reply(&reply, deadline)?;
        let watches = watches
            .iter()
            .map(|info| watch_info_json(info, info.interval_ns / 1_000_000))
            .collect();
        Ok((object([("watches", Json::Array(watches))]), Vec::new()))
    }

    fn manifest_get(
        &mut self,
        params: &BTreeMap<String, Json>,
        deadline: Instant,
    ) -> std::result::Result<(Json, Vec<Json>), ProtocolError> {
        check_keys(params, &["format"])?;
        let format = required(params, "format")?
            .as_str()
            .ok_or_else(|| ProtocolError::malformed("format must be a string"))?;
        if !matches!(format, "json" | "binary") {
            return Err(ProtocolError::malformed(
                "format must be `json` or `binary`",
            ));
        }
        let binary = format == "binary";
        let reply = self
            .shared
            .service
            .submit_manifest_wait(binary)
            .map_err(ProtocolError::from_stc)?;
        let reply = wait_reply(&reply, deadline)?.map_err(failure_to_error)?;
        let data = match &reply.data {
            crate::debug::ManifestData::Json(text) => json::parse(text).map_err(|_| {
                ProtocolError::new(
                    codes::DEBUG_INTERNAL,
                    "the canonical manifest projection cannot be re-read",
                )
            })?,
            crate::debug::ManifestData::Binary(bytes) => Json::string(base64_encode(bytes)),
        };
        let mut event = AuditEvent::new("manifest-export", AuditOutcome::Ok);
        event.scope = Some("observe".into());
        let (generation, build_id, cycle) = self.shared.tags();
        event.generation = Some(generation);
        event.build = Some(build_id);
        event.cycle = Some(cycle);
        event.detail = object([
            ("format", Json::string(reply.format)),
            ("digest", Json::string(reply.digest.clone())),
        ]);
        self.shared.audit(event).map_err(ProtocolError::from_stc)?;
        Ok((
            object([
                ("format", Json::string(reply.format)),
                ("digest", Json::string(reply.digest)),
                ("data", data),
            ]),
            Vec::new(),
        ))
    }
}

fn watch_info_json(info: &WatchInfo, interval_ms: i64) -> Json {
    object([
        ("watch_id", Json::unsigned(info.watch_id.0)),
        (
            "interval_ms",
            Json::unsigned(u64::try_from(interval_ms).unwrap_or(0)),
        ),
        (
            "targets",
            Json::Array(info.resolved.iter().cloned().map(Json::string).collect()),
        ),
    ])
}

fn message_kind(message: &Json) -> Option<&str> {
    message
        .as_object()
        .and_then(|object| object.get("kind"))
        .and_then(Json::as_str)
}

fn write_message(connection: &mut Connection, message: &Json) -> std::io::Result<()> {
    let text = message.render_compact();
    connection.write_all(text.as_bytes())?;
    connection.write_all(b"\n")?;
    connection.flush()
}

// ---------------------------------------------------------------------------
// Public test/client helper
// ---------------------------------------------------------------------------

/// One parsed endpoint file. The token is retained for the client connection.
#[derive(Clone, Debug)]
pub struct DebugEndpointFile {
    /// Transport-specific endpoint.
    pub endpoint: String,
    /// 64 lowercase hex characters.
    pub token: String,
    /// Configured transport.
    pub transport: TransportKind,
    /// Build identity.
    pub build_id: String,
    /// Generation.
    pub generation: u64,
    /// `debug` or `release`.
    pub mode: String,
    /// Audit log path recorded by the host.
    pub audit_log: String,
    /// Protocol version.
    pub protocol: String,
}

impl DebugEndpointFile {
    /// Reads and validates an endpoint file.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the file cannot be read or is not a valid
    /// format-1 endpoint object.
    pub fn read(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let value = json::parse_protocol(&text)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error.message))?;
        let object = value.as_object().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the endpoint file is not an object",
            )
        })?;
        let string = |key: &str| -> std::io::Result<String> {
            object
                .get(key)
                .and_then(Json::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("the endpoint file has no string `{key}`"),
                    )
                })
        };
        let transport = TransportKind::parse(&string("transport")?).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unknown endpoint transport",
            )
        })?;
        let protocol = string("protocol")?;
        if protocol != DEBUG_PROTOCOL {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unsupported endpoint protocol",
            ));
        }
        if object.get("format").and_then(Json::as_number) != Some(1) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unsupported endpoint file format",
            ));
        }
        let generation = object
            .get("generation")
            .and_then(Json::as_number)
            .filter(|value| *value >= 0)
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid endpoint generation",
                )
            })?;
        Ok(Self {
            endpoint: string("endpoint")?,
            token: string("token")?,
            transport,
            build_id: string("build_id")?,
            generation,
            mode: string("mode")?,
            audit_log: string("audit_log")?,
            protocol,
        })
    }
}

/// A minimal blocking `stc-debug/1` client used by protocol tests and the
/// phase-8 CLI. It keeps one request outstanding and stashes events that arrive
/// while a response is awaited.
pub struct DebugClient {
    connection: ClientConnection,
    endpoint: String,
    token: String,
    next_id: u64,
    buffer: Vec<u8>,
    events: VecDeque<Json>,
}

/// A client-side protocol failure.
#[derive(Debug)]
pub struct DebugClientError {
    /// Stable error code, when the server returned an error response.
    pub code: Option<String>,
    /// Human-readable message.
    pub message: String,
}

impl std::fmt::Display for DebugClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.code {
            Some(code) => write!(formatter, "{code}: {}", self.message),
            None => formatter.write_str(&self.message),
        }
    }
}

impl std::error::Error for DebugClientError {}

impl DebugClient {
    /// Connects using an endpoint file, retrying until the connect deadline.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the endpoint file is invalid or the server
    /// does not accept the connection within five seconds.
    pub fn connect_file(path: &Path) -> std::io::Result<Self> {
        let deadline = Instant::now() + FIRST_REQUEST_DEADLINE;
        loop {
            let attempt = DebugEndpointFile::read(path).and_then(|endpoint| {
                Self::connect_known(&endpoint.endpoint, &endpoint.token, endpoint.transport)
            });
            match attempt {
                Ok(client) => return Ok(client),
                Err(error) if Instant::now() >= deadline => return Err(error),
                Err(_) => std::thread::sleep(CONNECT_RETRY),
            }
        }
    }

    /// Connects to a known transport endpoint and token.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the transport refuses the connection.
    pub fn connect_known(
        endpoint: &str,
        token: &str,
        transport: TransportKind,
    ) -> std::io::Result<Self> {
        let mut connection = ClientConnection::connect_debug(transport, endpoint)?;
        let _ = connection.set_read_timeout(Some(Duration::from_secs(10)));
        let _ = connection.set_write_timeout(Some(REQUEST_DEADLINE));
        Ok(Self {
            connection,
            endpoint: endpoint.to_owned(),
            token: token.to_owned(),
            next_id: 0,
            buffer: Vec::new(),
            events: VecDeque::new(),
        })
    }

    /// Returns the transport endpoint.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Sends one request and returns the full response object.
    ///
    /// # Errors
    ///
    /// Returns a transport error, or a protocol error for a closed connection.
    pub fn call(&mut self, op: &str, params: Json) -> std::io::Result<Json> {
        self.next_id = self.next_id.saturating_add(1).max(1);
        let id = self.next_id;
        let request = json_object([
            ("kind", Json::string("request")),
            ("protocol", Json::string(DEBUG_PROTOCOL)),
            ("id", Json::unsigned(id)),
            ("token", Json::string(self.token.clone())),
            ("op", Json::string(op)),
            ("params", params),
        ]);
        self.write(&request)?;
        loop {
            let message = self.read_message()?.ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "the debug server closed the connection",
                )
            })?;
            if message_kind(&message) == Some("event") {
                self.events.push_back(message);
                continue;
            }
            if message
                .as_object()
                .and_then(|object| object.get("id"))
                .and_then(Json::as_number)
                != Some(i128::from(id))
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "the debug server returned an unexpected response id",
                ));
            }
            return Ok(message);
        }
    }

    /// Sends a request and returns every message read, in arrival order.
    ///
    /// The vector holds interleaved events and the response. A closed
    /// connection before the response yields the messages read so far. For
    /// `session.stop` the final `session-ended` event is collected too.
    ///
    /// # Errors
    ///
    /// Returns a transport error or a protocol error for an unexpected
    /// response id.
    pub fn call_messages(&mut self, op: &str, params: Json) -> std::io::Result<Vec<Json>> {
        self.next_id = self.next_id.saturating_add(1).max(1);
        let id = self.next_id;
        let request = json_object([
            ("kind", Json::string("request")),
            ("protocol", Json::string(DEBUG_PROTOCOL)),
            ("id", Json::unsigned(id)),
            ("token", Json::string(self.token.clone())),
            ("op", Json::string(op)),
            ("params", params),
        ]);
        self.write(&request)?;
        let mut messages = Vec::new();
        let mut answered = false;
        loop {
            let Some(message) = self.read_message()? else {
                return Ok(messages);
            };
            if message_kind(&message) == Some("event") {
                let session_ended = message
                    .as_object()
                    .and_then(|object| object.get("event"))
                    .and_then(Json::as_str)
                    == Some("session-ended");
                messages.push(message);
                if session_ended || (answered && op != "session.stop") {
                    return Ok(messages);
                }
                continue;
            }
            if message
                .as_object()
                .and_then(|object| object.get("id"))
                .and_then(Json::as_number)
                != Some(i128::from(id))
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "the debug server returned an unexpected response id",
                ));
            }
            messages.push(message);
            answered = true;
            if op != "session.stop" {
                return Ok(messages);
            }
            // `session.stop` sends its response before the final
            // `session-ended` event; keep reading so batch output includes it.
        }
    }

    /// Reads one message only when the transport already has data.
    ///
    /// Returns `None` when the connection is idle. This is how an interactive
    /// client notices a spontaneous `session-ended` event between requests.
    ///
    /// # Errors
    ///
    /// Returns a transport error or an invalid frame error.
    pub fn poll_message(&mut self) -> std::io::Result<Option<Json>> {
        if !self.connection.poll_readable()? {
            return Ok(None);
        }
        self.read_message()
    }

    /// Sends a request and returns `result`, or the server error.
    ///
    /// # Errors
    ///
    /// Returns a transport error or the stable protocol error object.
    pub fn call_ok(
        &mut self,
        op: &str,
        params: Json,
    ) -> std::result::Result<Json, DebugClientError> {
        let response = self.call(op, params).map_err(|error| DebugClientError {
            code: None,
            message: error.to_string(),
        })?;
        let object = response.as_object().ok_or_else(|| DebugClientError {
            code: None,
            message: "the server returned a non-object response".into(),
        })?;
        if object.get("status").and_then(Json::as_str) == Some("ok") {
            return Ok(object
                .get("result")
                .cloned()
                .unwrap_or(Json::Object(BTreeMap::new())));
        }
        let error = object
            .get("error")
            .and_then(Json::as_object)
            .ok_or_else(|| DebugClientError {
                code: None,
                message: "the server returned an error response without an error object".into(),
            })?;
        Err(DebugClientError {
            code: error.get("code").and_then(Json::as_str).map(str::to_owned),
            message: error
                .get("message")
                .and_then(Json::as_str)
                .unwrap_or("unknown")
                .to_owned(),
        })
    }

    /// Starts a session and returns the result object.
    ///
    /// # Errors
    ///
    /// Returns the server error or a transport failure.
    pub fn start_session(
        &mut self,
        scopes: &[&str],
    ) -> std::result::Result<Json, DebugClientError> {
        self.call_ok(
            "session.start",
            object([
                (
                    "client",
                    object([
                        ("name", Json::string("stc-debug-test")),
                        ("version", Json::string("1")),
                    ]),
                ),
                (
                    "scopes",
                    Json::Array(scopes.iter().map(|scope| Json::string(*scope)).collect()),
                ),
            ]),
        )
    }

    /// Sends a raw frame without an added LF.
    ///
    /// # Errors
    ///
    /// Returns a transport error.
    pub fn send_raw(&mut self, text: &str) -> std::io::Result<()> {
        self.connection.write_all(text.as_bytes())?;
        self.connection.write_all(b"\n")?;
        self.connection.flush()
    }

    /// Writes one raw JSON request frame.
    ///
    /// # Errors
    ///
    /// Returns a transport error when the frame is over the request limit.
    pub fn write_frame(&mut self, line: &[u8]) -> std::io::Result<()> {
        self.connection.write_all(line)?;
        self.connection.write_all(b"\n")?;
        self.connection.flush()
    }

    /// Reads one message. Returns `None` at end of stream.
    ///
    /// # Errors
    ///
    /// Returns a transport error, including a timeout, or an invalid frame.
    pub fn read_message(&mut self) -> std::io::Result<Option<Json>> {
        loop {
            if let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=position).collect();
                let text =
                    std::str::from_utf8(&line[..line.len().saturating_sub(1)]).map_err(|_| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "the debug server sent invalid UTF-8",
                        )
                    })?;
                let value = json::parse_protocol(text).map_err(|error| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, error.message)
                })?;
                return Ok(Some(value));
            }
            let mut chunk = [0_u8; 16 * 1024];
            match self.connection.read(&mut chunk) {
                Ok(0) => return Ok(None),
                Ok(count) => self.buffer.extend_from_slice(&chunk[..count]),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(error) if peer_closed(&error) => return Ok(None),
                Err(error) => return Err(error),
            }
        }
    }

    /// Reads the next event, stashing any response that arrives first.
    ///
    /// # Errors
    ///
    /// Returns an I/O error on a closed connection or invalid frame.
    pub fn next_event(&mut self) -> std::io::Result<Json> {
        if let Some(event) = self.events.pop_front() {
            return Ok(event);
        }
        let message = self.read_message()?.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "the debug server closed the connection",
            )
        })?;
        if message_kind(&message) == Some("event") {
            return Ok(message);
        }
        // At most one request is outstanding, so a response here belongs to a
        // caller that has already returned; keep it out of the way.
        self.events.push_back(message);
        Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "a response arrived while waiting for an event",
        ))
    }

    /// Returns events stashed while a response was awaited.
    #[must_use]
    pub fn take_events(&mut self) -> Vec<Json> {
        self.events.drain(..).collect()
    }

    fn write(&mut self, message: &Json) -> std::io::Result<()> {
        let text = message.render_compact();
        self.connection.write_all(text.as_bytes())?;
        self.connection.write_all(b"\n")?;
        self.connection.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_comparison_is_exact_case_sensitive_and_length_bound() {
        let token = "0123456789abcdef".repeat(4);
        assert!(token_matches(&token, &token));
        assert!(!token_matches(&token, &token.to_uppercase()));
        assert!(!token_matches(&token, &"0".repeat(64)));
        assert!(!token_matches(&token, &token[..63]));
        assert!(!token_matches(&token, &format!("{token}0")));
        assert!(!token_matches(&token, ""));
        assert!(!token_matches(&token, &token.replace('0', "g")));
        let mut changed = token.clone();
        changed.replace_range(63..64, "0");
        if changed != token {
            assert!(!token_matches(&token, &changed));
        }
    }

    #[test]
    fn closed_audit_log_refuses_appends_with_e021() {
        let path = std::env::temp_dir().join(format!(
            "stc-debug-audit-closed-{}-{}.jsonl",
            std::process::id(),
            transport::monotonic_ns()
        ));
        let file = std::fs::File::create(&path).expect("audit file");
        let audit = SharedAudit::new(file);
        audit.close();
        let error = audit
            .append(AuditEvent::new("server-stop", AuditOutcome::Ok))
            .expect_err("closed audit log");
        assert_eq!(error.code, codes::DEBUG_AUDIT);
        let _ = std::fs::remove_file(&path);
    }
}
