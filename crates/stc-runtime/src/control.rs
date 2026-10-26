//! Local deploy/status control channel for `stc run --watch` (D-155).
//!
//! The endpoint is a Windows named pipe or a Unix socket with user-only
//! permissions. Every request carries the protocol version and a per-run
//! nonce; the server validates both before queueing work. The control thread
//! only reads and writes the channel: deploy, status, download, and stop
//! requests are queued for the cycle thread, which owns all [`Host`] state and
//! activates a prepared candidate between cycles.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use stc_got::json::{Json, parse as parse_json};

use crate::Host;
use crate::host::{Candidate, LoadMode, LoadOutcome};

/// Control protocol version.
pub const PROTOCOL_VERSION: i64 = 1;
/// Largest request line the server accepts.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
/// Control-channel unavailable or unauthorized.
pub const CONTROL_UNAVAILABLE: &str = "STC-E-ONLINE-007";
/// Deploy rejected (incompatible candidate or prepare failure).
pub const DEPLOY_REJECTED: &str = "STC-E-ONLINE-008";

/// One deploy/status/download/stop request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlRequest {
    /// Client request identifier.
    pub id: u64,
    /// Operation.
    pub op: ControlOp,
    /// Module path for a deploy.
    pub module: Option<PathBuf>,
    /// Manifest path for a deploy.
    pub manifest: Option<PathBuf>,
}

/// A requested control operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlOp {
    /// Load, prepare, and activate a module/manifest pair.
    Deploy,
    /// Return the host status snapshot.
    Status,
    /// Export the active download envelope.
    Download,
    /// Stop the host cleanly.
    Stop,
}

impl ControlOp {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "deploy" => Some(Self::Deploy),
            "status" => Some(Self::Status),
            "download" => Some(Self::Download),
            "stop" => Some(Self::Stop),
            _ => None,
        }
    }
}

/// The cycle thread's reaction to a processed request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlSignal {
    /// Keep running.
    None,
    /// A validated stop request was processed.
    Stop,
}

/// A bound control endpoint and its accepting thread.
pub struct ControlServer {
    shared: Arc<Shared>,
    shutdown: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
    endpoint: String,
}

struct Shared {
    nonce: String,
    queue: Mutex<VecDeque<Queued>>,
    shutdown: Arc<AtomicBool>,
}

struct Queued {
    request: ControlRequest,
    reply: Arc<Reply>,
}

struct Reply {
    value: Mutex<Option<Json>>,
    ready: Condvar,
}

impl ControlServer {
    /// Binds a control endpoint and starts the accept thread.
    ///
    /// # Errors
    ///
    /// Returns [`CONTROL_UNAVAILABLE`] when the endpoint cannot be created
    /// with user-only access.
    pub fn bind(endpoint: &str, nonce: &str) -> stc_got::error::Result<Self> {
        if nonce.is_empty() {
            return Err(control_error("the control nonce must not be empty"));
        }
        let shutdown = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared {
            nonce: nonce.to_owned(),
            queue: Mutex::new(VecDeque::new()),
            shutdown: Arc::clone(&shutdown),
        });
        let backend = Backend::bind(endpoint)?;
        let thread_shared = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("stc-control".into())
            .spawn(move || serve(backend, thread_shared))
            .map_err(|_| control_error("cannot start the control thread"))?;
        Ok(Self {
            shared,
            shutdown,
            handle: Some(handle),
            endpoint: endpoint.to_owned(),
        })
    }

    /// Returns the bound endpoint spelling.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Processes every queued request against the host.
    ///
    /// Runs on the cycle thread between scans.
    pub fn process(&self, host: &mut Host) -> ControlSignal {
        let pending: Vec<Queued> = {
            let mut queue = self
                .shared
                .queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            queue.drain(..).collect()
        };
        let mut signal = ControlSignal::None;
        for queued in pending {
            let response = handle_request(host, &queued.request);
            if queued.request.op == ControlOp::Stop {
                signal = ControlSignal::Stop;
            }
            queued.reply.set(response);
        }
        signal
    }

    /// Requests that the accept thread stop, without waiting for it.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(handle) = self.handle.take() {
            // The accept thread is detached: a platform accept call can take
            // a polling interval to observe shutdown, and the host must not
            // block on it during exit.
            drop(handle);
        }
    }
}

fn handle_request(host: &mut Host, request: &ControlRequest) -> Json {
    let id = Json::unsigned(request.id);
    match request.op {
        ControlOp::Deploy => {
            let (Some(module), Some(manifest)) = (&request.module, &request.manifest) else {
                return error_response(id, DEPLOY_REJECTED, "a deploy needs a module and manifest");
            };
            let started = Instant::now();
            let candidate = match Candidate::open(host.platform(), module, manifest) {
                Ok(candidate) => candidate,
                Err(error) => {
                    return error_response(id, error.code, &error.message);
                }
            };
            let report = compatibility_report(
                candidate.manifest(),
                host.store().active().map(|stored| &stored.manifest),
            );
            let prepare_ns =
                Json::unsigned(u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX));
            match host.load(candidate, LoadMode::Online) {
                Ok(LoadOutcome::Prepared { generation }) => match host.activate() {
                    Ok(active) => Json::object([
                        ("activated".into(), Json::Bool(true)),
                        ("generation".into(), Json::unsigned(active)),
                        ("id".into(), id),
                        ("ok".into(), Json::Bool(true)),
                        ("prepare_ns".into(), prepare_ns),
                        ("prepared_generation".into(), Json::unsigned(generation)),
                        ("protocol".into(), protocol_value()),
                        ("report".into(), report),
                    ]),
                    Err(error) => error_response(id, DEPLOY_REJECTED, &error.message),
                },
                Ok(LoadOutcome::ColdRequired { reason }) => {
                    let mut response = error_response(id, DEPLOY_REJECTED, &reason);
                    if let Json::Object(fields) = &mut response {
                        fields.insert("cold_required".into(), Json::Bool(true));
                        fields.insert("prepare_ns".into(), prepare_ns);
                        fields.insert("report".into(), report);
                    }
                    response
                }
                Ok(other) => error_response(
                    id,
                    DEPLOY_REJECTED,
                    &format!("unexpected load outcome: {other:?}"),
                ),
                Err(error) => {
                    let mut response = error_response(id, error.code, &error.message);
                    if let Json::Object(fields) = &mut response {
                        fields.insert("prepare_ns".into(), prepare_ns);
                        fields.insert("report".into(), report);
                    }
                    response
                }
            }
        }
        ControlOp::Status => {
            let status = host.status();
            Json::object([
                (
                    "active_build".into(),
                    optional_build(status.active_build_id),
                ),
                ("cycles".into(), Json::unsigned(status.cycles)),
                ("id".into(), id),
                (
                    "last_rejection".into(),
                    Json::optional(status.last_rejection.map(Json::String)),
                ),
                ("ok".into(), Json::Bool(true)),
                (
                    "pending_build".into(),
                    optional_build(status.pending_build_id),
                ),
                ("protocol".into(), protocol_value()),
                (
                    "retired_builds".into(),
                    Json::Array(
                        status
                            .retired_build_ids
                            .iter()
                            .map(|build| Json::String(build.to_hex()))
                            .collect(),
                    ),
                ),
            ])
        }
        ControlOp::Download => match host.download() {
            Ok(download) => Json::object([
                ("download".into(), Json::String(download.to_json_text())),
                ("id".into(), id),
                ("ok".into(), Json::Bool(true)),
                ("protocol".into(), protocol_value()),
            ]),
            Err(error) => error_response(id, error.code, &error.message),
        },
        ControlOp::Stop => Json::object([
            ("id".into(), id),
            ("ok".into(), Json::Bool(true)),
            ("protocol".into(), protocol_value()),
        ]),
    }
}

/// Builds the value-free compatibility report the host returns to a watcher.
fn compatibility_report(
    candidate: &stc_got::Manifest,
    baseline: Option<&stc_got::Manifest>,
) -> Json {
    use stc_got::PlanAction;
    let mut preserved = 0_u64;
    let mut allocated = 0_u64;
    let mut dropped = 0_u64;
    let mut rejected = 0_u64;
    for action in &candidate.plan.actions {
        match action.action {
            PlanAction::Preserve | PlanAction::Migrate => preserved += 1,
            PlanAction::Allocate | PlanAction::Reset => allocated += 1,
            PlanAction::Drop => dropped += 1,
            PlanAction::Reject => rejected += 1,
        }
    }
    let baseline_retired = baseline.map_or(0, |manifest| manifest.history.retired.len());
    let retired = candidate
        .history
        .retired
        .len()
        .saturating_sub(baseline_retired);
    let mut warnings = Vec::new();
    let mut rejections = Vec::new();
    for diagnostic in &candidate.plan.diagnostics {
        let entry = Json::object([
            ("code".into(), Json::String(diagnostic.code.clone())),
            ("message".into(), Json::String(diagnostic.message.clone())),
        ]);
        if diagnostic.severity >= 2 {
            rejections.push(entry);
        } else {
            warnings.push(entry);
        }
    }
    let verdict = match candidate.plan.verdict {
        stc_got::PlanVerdict::Cold => "cold",
        stc_got::PlanVerdict::OnlineCandidate => "compatible",
        stc_got::PlanVerdict::Rejected => "incompatible",
    };
    Json::object([
        (
            "baseline_build_id".into(),
            optional_build(baseline.map(|manifest| manifest.header.build_id)),
        ),
        (
            "candidate_build_id".into(),
            Json::String(candidate.header.build_id.to_hex()),
        ),
        (
            "counts".into(),
            Json::object([
                ("allocated".into(), Json::unsigned(allocated)),
                ("dropped".into(), Json::unsigned(dropped)),
                ("preserved".into(), Json::unsigned(preserved)),
                ("rejected".into(), Json::unsigned(rejected)),
                (
                    "retired".into(),
                    Json::unsigned(u64::try_from(retired).unwrap_or(u64::MAX)),
                ),
            ]),
        ),
        ("rejections".into(), Json::Array(rejections)),
        ("verdict".into(), Json::String(verdict.into())),
        ("warnings".into(), Json::Array(warnings)),
    ])
}

fn optional_build(build: Option<stc_got::GotId>) -> Json {
    Json::optional(build.map(|build| Json::String(build.to_hex())))
}

fn protocol_value() -> Json {
    Json::unsigned(u64::try_from(PROTOCOL_VERSION).unwrap_or(1))
}

fn error_response(id: Json, code: &str, message: &str) -> Json {
    Json::object([
        ("code".into(), Json::String(code.to_owned())),
        ("id".into(), id),
        ("message".into(), Json::String(message.to_owned())),
        ("ok".into(), Json::Bool(false)),
        ("protocol".into(), protocol_value()),
    ])
}

fn control_error(message: impl Into<String>) -> stc_got::error::Error {
    stc_got::error::Error::new(CONTROL_UNAVAILABLE, message.into())
}

impl Reply {
    fn new() -> Self {
        Self {
            value: Mutex::new(None),
            ready: Condvar::new(),
        }
    }

    fn set(&self, value: Json) {
        let mut slot = self
            .value
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = Some(value);
        self.ready.notify_all();
    }

    fn wait(&self, shared: &Shared) -> Option<Json> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut slot = self
            .value
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if let Some(value) = slot.take() {
                return Some(value);
            }
            if shared.shutdown.load(Ordering::Acquire) || Instant::now() >= deadline {
                return None;
            }
            let (next, _) = self
                .ready
                .wait_timeout(slot, Duration::from_millis(100))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            slot = next;
        }
    }
}

fn serve(mut backend: Backend, shared: Arc<Shared>) {
    loop {
        if shared.shutdown.load(Ordering::Acquire) {
            break;
        }
        let Some(connection) = backend.accept(&shared.shutdown) else {
            continue;
        };
        // Each client gets a handler thread so the watcher can keep its
        // connection open while `online download --host` connects. Only the
        // queue and channel I/O are touched off the cycle thread.
        let connection_shared = Arc::clone(&shared);
        let _ = std::thread::Builder::new()
            .name("stc-control-client".into())
            .spawn(move || serve_connection(connection, &connection_shared));
    }
    backend.shutdown();
}

fn serve_connection(connection: Connection, shared: &Shared) {
    let mut reader = BufReader::new(connection);
    loop {
        let mut line = Vec::new();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if line.len() > MAX_REQUEST_BYTES {
            let _ = write_line(
                reader.get_mut(),
                &error_response_json(0, CONTROL_UNAVAILABLE, "the request line is too large"),
            );
            return;
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let value = match parse_json(text) {
            Ok(value) => value,
            Err(error) => {
                let _ = write_line(
                    reader.get_mut(),
                    &error_response_json(0, CONTROL_UNAVAILABLE, &error.message),
                );
                return;
            }
        };
        match parse_request(&value, &shared.nonce) {
            Ok(request) => {
                let reply = Arc::new(Reply::new());
                {
                    let mut queue = shared
                        .queue
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    queue.push_back(Queued {
                        request: request.clone(),
                        reply: Arc::clone(&reply),
                    });
                }
                let Some(response) = reply.wait(shared) else {
                    return;
                };
                let stop = request.op == ControlOp::Stop;
                if write_line(reader.get_mut(), &response).is_err() || stop {
                    return;
                }
            }
            Err(message) => {
                let _ = write_line(
                    reader.get_mut(),
                    &error_response_json(0, CONTROL_UNAVAILABLE, &message),
                );
                return;
            }
        }
    }
}

fn error_response_json(id: u64, code: &str, message: &str) -> Json {
    error_response(Json::unsigned(id), code, message)
}

fn parse_request(value: &Json, nonce: &str) -> Result<ControlRequest, String> {
    let Some(object) = value.as_object() else {
        return Err("the control request must be a JSON object".into());
    };
    let protocol = object
        .get("protocol")
        .and_then(Json::as_number)
        .ok_or_else(|| "the control request has no protocol version".to_string())?;
    if protocol != i128::from(PROTOCOL_VERSION) {
        return Err(format!(
            "unsupported control protocol {protocol} (expected {PROTOCOL_VERSION})"
        ));
    }
    let request_nonce = object
        .get("nonce")
        .and_then(Json::as_str)
        .ok_or_else(|| "the control request has no nonce".to_string())?;
    if request_nonce != nonce {
        return Err("the control nonce is invalid".into());
    }
    let id = object
        .get("id")
        .and_then(Json::as_number)
        .ok_or_else(|| "the control request has no id".to_string())?;
    let id = u64::try_from(id).map_err(|_| "the control request id is out of range".to_string())?;
    let op_name = object
        .get("op")
        .and_then(Json::as_str)
        .ok_or_else(|| "the control request has no op".to_string())?;
    let op = ControlOp::parse(op_name)
        .ok_or_else(|| format!("unknown control operation `{op_name}`"))?;
    let module = optional_path(object.get("module"), "module")?;
    let manifest = optional_path(object.get("manifest"), "manifest")?;
    if op == ControlOp::Deploy && (module.is_none() || manifest.is_none()) {
        return Err("a deploy request needs module and manifest paths".into());
    }
    Ok(ControlRequest {
        id,
        op,
        module,
        manifest,
    })
}

fn optional_path(value: Option<&Json>, name: &str) -> Result<Option<PathBuf>, String> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(text)) => Ok(Some(PathBuf::from(text))),
        Some(_) => Err(format!("the control request `{name}` must be a string")),
    }
}

fn write_line(connection: &mut Connection, value: &Json) -> std::io::Result<()> {
    let mut line = value.render_compact();
    line.push('\n');
    connection.write_all(line.as_bytes())
}

/// A client for one control endpoint.
pub struct ControlClient {
    reader: BufReader<ClientConnection>,
    next_id: u64,
}

impl ControlClient {
    /// Connects to a control endpoint, retrying until `timeout` elapses.
    ///
    /// # Errors
    ///
    /// Returns a message when the endpoint cannot be reached.
    pub fn connect(endpoint: &str, timeout: Duration) -> Result<Self, String> {
        let deadline = Instant::now() + timeout;
        loop {
            match ClientConnection::connect(endpoint) {
                Ok(connection) => {
                    return Ok(Self {
                        reader: BufReader::new(connection),
                        next_id: 1,
                    });
                }
                Err(error) if Instant::now() >= deadline => {
                    return Err(format!(
                        "cannot connect to control endpoint `{endpoint}`: {error}"
                    ));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(25)),
            }
        }
    }

    /// Sends one request and returns the parsed response.
    ///
    /// `body` holds the operation fields; `protocol`, `nonce`, and `id` are
    /// added here.
    ///
    /// # Errors
    ///
    /// Returns a message when the request cannot be written or the response is
    /// malformed.
    pub fn request(&mut self, nonce: &str, body: Json) -> Result<Json, String> {
        let Json::Object(mut fields) = body else {
            return Err("the control request body must be an object".into());
        };
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        fields.insert("id".into(), Json::unsigned(id));
        fields.insert("nonce".into(), Json::String(nonce.to_owned()));
        fields.insert("protocol".into(), protocol_value());
        let mut line = Json::Object(fields).render_compact();
        line.push('\n');
        self.reader
            .get_mut()
            .write_all(line.as_bytes())
            .map_err(|error| format!("cannot write the control request: {error}"))?;
        let mut response = Vec::new();
        self.reader
            .read_until(b'\n', &mut response)
            .map_err(|error| format!("cannot read the control response: {error}"))?;
        if response.is_empty() {
            return Err("the control host closed the connection".into());
        }
        let text = String::from_utf8_lossy(&response);
        parse_json(text.trim())
            .map_err(|error| format!("invalid control response: {}", error.message))
    }
}

#[cfg(unix)]
mod platform {
    use std::io::{self, Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::control_error;

    pub(super) struct Connection {
        stream: UnixStream,
        shutdown: Arc<AtomicBool>,
    }

    impl Read for Connection {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            loop {
                match self.stream.read(buffer) {
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                        ) =>
                    {
                        if self.shutdown.load(Ordering::Acquire) {
                            return Ok(0);
                        }
                    }
                    other => return other,
                }
            }
        }
    }

    impl Write for Connection {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.stream.write(buffer)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.stream.flush()
        }
    }

    pub(super) struct ClientConnection {
        stream: UnixStream,
    }

    impl ClientConnection {
        pub(super) fn connect(endpoint: &str) -> io::Result<Self> {
            let stream = UnixStream::connect(endpoint)?;
            stream.set_read_timeout(Some(Duration::from_secs(30)))?;
            stream.set_write_timeout(Some(Duration::from_secs(5)))?;
            Ok(Self { stream })
        }
    }

    impl Read for ClientConnection {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.stream.read(buffer)
        }
    }

    impl Write for ClientConnection {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.stream.write(buffer)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.stream.flush()
        }
    }

    pub(super) struct Backend {
        listener: UnixListener,
        path: PathBuf,
    }

    impl Backend {
        pub(super) fn bind(endpoint: &str) -> stc_got::error::Result<Self> {
            use std::os::unix::fs::PermissionsExt;
            let path = PathBuf::from(endpoint);
            let listener = UnixListener::bind(&path).map_err(|error| {
                control_error(format!(
                    "cannot bind control socket `{}`: {error}",
                    path.display()
                ))
            })?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(
                |error| {
                    control_error(format!(
                        "cannot restrict control socket permissions: {error}"
                    ))
                },
            )?;
            listener.set_nonblocking(true).map_err(|error| {
                control_error(format!("cannot configure control socket: {error}"))
            })?;
            Ok(Self { listener, path })
        }

        pub(super) fn accept(&mut self, shutdown: &Arc<AtomicBool>) -> Option<Connection> {
            loop {
                if shutdown.load(Ordering::Acquire) {
                    return None;
                }
                match self.listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                        return Some(Connection {
                            stream,
                            shutdown: Arc::clone(shutdown),
                        });
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(_) => return None,
                }
            }
        }

        pub(super) fn shutdown(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::{OsStr, c_void};
    use std::fs::File;
    use std::io::{self, Read, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{FromRawHandle, RawHandle};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    type Handle = *mut c_void;

    const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
    const PIPE_TYPE_BYTE: u32 = 0x0000_0000;
    const PIPE_READMODE_BYTE: u32 = 0x0000_0000;
    const PIPE_WAIT: u32 = 0x0000_0000;
    const PIPE_NOWAIT: u32 = 0x0000_0001;
    const PIPE_UNLIMITED_INSTANCES: u32 = 255;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const OPEN_EXISTING: u32 = 3;
    const ERROR_FILE_NOT_FOUND: u32 = 2;
    const ERROR_PIPE_BUSY: u32 = 231;
    const ERROR_PIPE_CONNECTED: u32 = 535;
    const ERROR_PIPE_LISTENING: u32 = 536;
    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_USER: u32 = 1;
    const ACL_REVISION: u32 = 2;
    const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
    const GENERIC_ALL: u32 = 0x1000_0000;

    #[repr(C)]
    struct SecurityAttributes {
        n_length: u32,
        security_descriptor: *mut c_void,
        inherit_handle: i32,
    }

    #[repr(C, align(8))]
    struct Aligned<const N: usize>([u8; N]);

    impl<const N: usize> Aligned<N> {
        fn new() -> Self {
            Self([0; N])
        }

        fn as_mut(&mut self) -> *mut c_void {
            self.0.as_mut_ptr().cast()
        }
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateNamedPipeW(
            name: *const u16,
            open_mode: u32,
            pipe_mode: u32,
            max_instances: u32,
            out_buffer: u32,
            in_buffer: u32,
            timeout: u32,
            attributes: *const SecurityAttributes,
        ) -> Handle;
        fn ConnectNamedPipe(pipe: Handle, overlapped: *mut c_void) -> i32;
        fn DisconnectNamedPipe(pipe: Handle) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            attributes: *const c_void,
            disposition: u32,
            flags: u32,
            template: Handle,
        ) -> Handle;
        fn WaitNamedPipeW(name: *const u16, timeout: u32) -> i32;
        fn PeekNamedPipe(
            pipe: Handle,
            buffer: *mut c_void,
            buffer_size: u32,
            read: *mut u32,
            available: *mut u32,
            left: *mut u32,
        ) -> i32;
        fn ReadFile(
            file: Handle,
            buffer: *mut u8,
            bytes: u32,
            read: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn WriteFile(
            file: Handle,
            buffer: *const u8,
            bytes: u32,
            written: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn GetCurrentProcess() -> Handle;
        fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
        fn GetTokenInformation(
            token: Handle,
            class: u32,
            info: *mut c_void,
            length: u32,
            needed: *mut u32,
        ) -> i32;
        fn GetLastError() -> u32;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn InitializeSecurityDescriptor(descriptor: *mut c_void, revision: u32) -> i32;
        fn SetSecurityDescriptorDacl(
            descriptor: *mut c_void,
            present: i32,
            acl: *mut c_void,
            defaulted: i32,
        ) -> i32;
        fn InitializeAcl(acl: *mut c_void, length: u32, revision: u32) -> i32;
        fn AddAccessAllowedAce(
            acl: *mut c_void,
            revision: u32,
            access: u32,
            sid: *mut c_void,
        ) -> i32;
    }

    fn wide(path: &str) -> Vec<u16> {
        OsStr::new(path).encode_wide().chain([0]).collect()
    }

    fn last_error() -> u32 {
        // SAFETY: GetLastError has no preconditions.
        unsafe { GetLastError() }
    }

    /// A user-only security descriptor and its ACL.
    struct UserSecurity {
        _descriptor: Aligned<64>,
        _acl: Aligned<256>,
        _token: Aligned<1024>,
        attributes: SecurityAttributes,
    }

    impl UserSecurity {
        fn new() -> Option<Self> {
            let mut token = Aligned::<1024>::new();
            let mut token_handle: Handle = std::ptr::null_mut();
            // SAFETY: the handles and buffers are valid for these calls; the
            // access mask, class, and lengths are fixed Windows contract values.
            unsafe {
                if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token_handle) == 0 {
                    return None;
                }
                let mut needed = 0_u32;
                let _ = GetTokenInformation(
                    token_handle,
                    TOKEN_USER,
                    token.as_mut(),
                    1024,
                    &mut needed,
                );
                if needed == 0
                    || needed > 1024
                    || GetTokenInformation(
                        token_handle,
                        TOKEN_USER,
                        token.as_mut(),
                        1024,
                        &mut needed,
                    ) == 0
                {
                    CloseHandle(token_handle);
                    return None;
                }
                CloseHandle(token_handle);
            }
            // TOKEN_USER starts with SID_AND_ATTRIBUTES { PSID Sid; DWORD }.
            let sid = unsafe { *token.0.as_ptr().cast::<*mut c_void>() };
            if sid.is_null() {
                return None;
            }
            let mut descriptor = Aligned::<64>::new();
            let mut acl = Aligned::<256>::new();
            // SAFETY: the buffers meet the documented minimum sizes and the
            // SID came from the current process token.
            unsafe {
                if InitializeSecurityDescriptor(descriptor.as_mut(), SECURITY_DESCRIPTOR_REVISION)
                    == 0
                    || InitializeAcl(acl.as_mut(), 256, ACL_REVISION) == 0
                    || AddAccessAllowedAce(acl.as_mut(), ACL_REVISION, GENERIC_ALL, sid) == 0
                    || SetSecurityDescriptorDacl(descriptor.as_mut(), 1, acl.as_mut(), 0) == 0
                {
                    return None;
                }
            }
            Some(Self {
                attributes: SecurityAttributes {
                    n_length: u32::try_from(std::mem::size_of::<SecurityAttributes>()).ok()?,
                    security_descriptor: descriptor.as_mut(),
                    inherit_handle: 0,
                },
                _descriptor: descriptor,
                _acl: acl,
                _token: token,
            })
        }
    }

    pub(super) struct Connection {
        handle: Handle,
        shutdown: Arc<AtomicBool>,
    }

    // The handle is used by one handler thread at a time; Windows pipe
    // handles are process-wide values.
    unsafe impl Send for Connection {}

    pub(super) struct ClientConnection {
        file: File,
    }

    impl ClientConnection {
        pub(super) fn connect(endpoint: &str) -> io::Result<Self> {
            let name = wide(endpoint);
            // SAFETY: the name is NUL-terminated and the access mask is valid.
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if !handle.is_null() && handle != (-1_isize as Handle) {
                // SAFETY: CreateFileW returned an owned handle.
                let file = unsafe {
                    File::from_raw_handle(handle.cast::<std::ffi::c_void>() as RawHandle)
                };
                return Ok(Self { file });
            }
            let error = io::Error::last_os_error();
            if matches!(last_error(), ERROR_FILE_NOT_FOUND | ERROR_PIPE_BUSY) {
                // SAFETY: the name is NUL-terminated.
                unsafe { WaitNamedPipeW(name.as_ptr(), 25) };
            }
            Err(error)
        }
    }

    impl Read for ClientConnection {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.file.read(buffer)
        }
    }

    impl Write for ClientConnection {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.file.write(buffer)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.file.flush()
        }
    }

    pub(super) struct Backend {
        endpoint: String,
    }

    impl Backend {
        pub(super) fn bind(endpoint: &str) -> stc_got::error::Result<Self> {
            Ok(Self {
                endpoint: endpoint.to_owned(),
            })
        }

        pub(super) fn accept(&mut self, shutdown: &Arc<AtomicBool>) -> Option<Connection> {
            let name = wide(&self.endpoint);
            let security = UserSecurity::new()?;
            // SAFETY: the name is NUL-terminated and the security attributes
            // point to live buffers for the duration of the call.
            let handle = unsafe {
                CreateNamedPipeW(
                    name.as_ptr(),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT,
                    PIPE_UNLIMITED_INSTANCES,
                    64 * 1024,
                    64 * 1024,
                    0,
                    &security.attributes,
                )
            };
            if handle.is_null() || handle == (-1_isize as Handle) {
                return None;
            }
            loop {
                if shutdown.load(Ordering::Acquire) {
                    // SAFETY: the handle is live and owned here.
                    unsafe { CloseHandle(handle) };
                    return None;
                }
                // SAFETY: the handle is live.
                let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
                if connected != 0 {
                    break;
                }
                match last_error() {
                    ERROR_PIPE_CONNECTED => break,
                    ERROR_PIPE_LISTENING => std::thread::sleep(Duration::from_millis(10)),
                    _ => {
                        // SAFETY: the handle is live.
                        unsafe { CloseHandle(handle) };
                        return None;
                    }
                }
            }
            let mut mode = PIPE_READMODE_BYTE | PIPE_WAIT;
            // SAFETY: the connected handle accepts a mode change.
            unsafe {
                SetNamedPipeHandleState(
                    handle,
                    &mut mode,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
            }
            Some(Connection {
                handle,
                shutdown: Arc::clone(shutdown),
            })
        }

        pub(super) fn shutdown(&mut self) {}
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "SetNamedPipeHandleState"]
        fn SetNamedPipeHandleState(
            pipe: Handle,
            mode: *mut u32,
            max_collection_count: *mut u32,
            collect_data_timeout: *mut u32,
        ) -> i32;
    }

    impl Drop for Connection {
        fn drop(&mut self) {
            // SAFETY: the handle is owned by this connection.
            unsafe {
                DisconnectNamedPipe(self.handle);
                CloseHandle(self.handle);
            }
        }
    }

    impl Read for Connection {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if buffer.is_empty() {
                return Ok(0);
            }
            loop {
                if self.shutdown.load(Ordering::Acquire) {
                    return Ok(0);
                }
                let mut available = 0_u32;
                // SAFETY: the handle is live and the out pointer is valid.
                let ok = unsafe {
                    PeekNamedPipe(
                        self.handle,
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        &mut available,
                        std::ptr::null_mut(),
                    )
                };
                if ok == 0 {
                    return Ok(0);
                }
                if available == 0 {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                let wanted = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
                let mut read = 0_u32;
                // SAFETY: the handle is live and the buffer has `wanted` bytes.
                let ok = unsafe {
                    ReadFile(
                        self.handle,
                        buffer.as_mut_ptr(),
                        wanted,
                        &mut read,
                        std::ptr::null_mut(),
                    )
                };
                if ok == 0 {
                    return Ok(0);
                }
                return Ok(usize::try_from(read).unwrap_or(0));
            }
        }
    }

    impl Write for Connection {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            let mut written = 0_u32;
            // SAFETY: the handle is live and the buffer covers `bytes`.
            let ok = unsafe {
                WriteFile(
                    self.handle,
                    buffer.as_ptr(),
                    u32::try_from(buffer.len()).unwrap_or(u32::MAX),
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(usize::try_from(written).unwrap_or(0))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(unix)]
use platform::{Backend, ClientConnection, Connection};
#[cfg(windows)]
use platform::{Backend, ClientConnection, Connection};

#[cfg(not(any(unix, windows)))]
compile_error!("the control channel requires a Unix or Windows host");

#[cfg(test)]
mod tests {
    use super::*;

    use stc_got::json::Json;

    use crate::store::ManifestStore;

    fn endpoint(label: &str) -> String {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let token = format!("{}-{id}-{label}", std::process::id());
        if cfg!(windows) {
            format!(r"\\.\pipe\stc-control-test-{token}")
        } else {
            std::env::temp_dir()
                .join(format!("stc-control-test-{token}.sock"))
                .to_string_lossy()
                .into_owned()
        }
    }

    #[test]
    fn request_parsing_checks_protocol_and_nonce() {
        let request = Json::object([
            ("id".into(), Json::unsigned(1)),
            ("nonce".into(), Json::string("abc")),
            ("op".into(), Json::string("status")),
            ("protocol".into(), Json::unsigned(1)),
        ]);
        assert!(parse_request(&request, "abc").is_ok());
        assert!(parse_request(&request, "wrong").is_err());
        let wrong_protocol = Json::object([
            ("id".into(), Json::unsigned(1)),
            ("nonce".into(), Json::string("abc")),
            ("op".into(), Json::string("status")),
            ("protocol".into(), Json::unsigned(2)),
        ]);
        assert!(
            parse_request(&wrong_protocol, "abc")
                .expect_err("protocol")
                .contains("protocol")
        );
    }

    #[test]
    fn endpoint_rejects_a_wrong_nonce_before_queueing() {
        let endpoint = endpoint("nonce");
        let server = ControlServer::bind(&endpoint, "right").expect("bind");
        let mut client =
            ControlClient::connect(&endpoint, Duration::from_secs(5)).expect("connect");
        let response = client
            .request(
                "wrong",
                Json::object([("op".into(), Json::string("status"))]),
            )
            .expect("response");
        assert_eq!(
            response
                .as_object()
                .and_then(|object| object.get("code"))
                .and_then(Json::as_str),
            Some(CONTROL_UNAVAILABLE)
        );
        server.shutdown();
    }

    #[test]
    fn a_valid_stop_request_is_queued_and_replied() {
        let _state = crate::test_support::state_guard();
        let endpoint = endpoint("stop");
        let server = ControlServer::bind(&endpoint, "nonce").expect("bind");
        let mut host = Host::new(crate::host_platform(), ManifestStore::memory());
        let worker = std::thread::spawn({
            let endpoint = endpoint.clone();
            move || {
                let mut client =
                    ControlClient::connect(&endpoint, Duration::from_secs(5)).expect("connect");
                client
                    .request("nonce", Json::object([("op".into(), Json::string("stop"))]))
                    .expect("response")
            }
        });
        let mut signal = ControlSignal::None;
        for _ in 0..100 {
            signal = server.process(&mut host);
            if signal == ControlSignal::Stop {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(signal, ControlSignal::Stop);
        let response = worker.join().expect("worker");
        assert!(
            response
                .as_object()
                .and_then(|object| object.get("ok"))
                .and_then(Json::as_bool)
                .unwrap_or(false)
        );
        server.shutdown();
    }

    #[cfg(unix)]
    #[test]
    fn unix_socket_is_user_only() {
        use std::os::unix::fs::PermissionsExt;
        let endpoint = endpoint("perms");
        let server = ControlServer::bind(&endpoint, "nonce").expect("bind");
        let mode = std::fs::metadata(&endpoint)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        server.shutdown();
    }

    #[test]
    fn protocol_version_mismatch_is_rejected_by_the_server() {
        let endpoint = endpoint("protocol");
        let server = ControlServer::bind(&endpoint, "nonce").expect("bind");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut raw = loop {
            match platform::ClientConnection::connect(&endpoint) {
                Ok(connection) => break connection,
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("connect: {error}"),
            }
        };
        raw.write_all(b"{\"id\":1,\"nonce\":\"nonce\",\"op\":\"status\",\"protocol\":2}\n")
            .expect("write");
        let mut reader = BufReader::new(raw);
        let mut line = String::new();
        reader.read_line(&mut line).expect("read response");
        let response = parse_json(line.trim()).expect("json");
        assert_eq!(
            response
                .as_object()
                .and_then(|object| object.get("code"))
                .and_then(Json::as_str),
            Some(CONTROL_UNAVAILABLE)
        );
        server.shutdown();
    }

    #[test]
    fn concurrent_clients_are_served() {
        let _state = crate::test_support::state_guard();
        let endpoint = endpoint("concurrent");
        let server = ControlServer::bind(&endpoint, "nonce").expect("bind");
        let mut host = Host::new(crate::host_platform(), ManifestStore::memory());
        let mut workers: Vec<Option<_>> = (0..2)
            .map(|_| {
                let endpoint = endpoint.clone();
                Some(std::thread::spawn(move || {
                    let mut client =
                        ControlClient::connect(&endpoint, Duration::from_secs(5)).expect("connect");
                    client
                        .request(
                            "nonce",
                            Json::object([("op".into(), Json::string("status"))]),
                        )
                        .expect("response")
                }))
            })
            .collect();
        let mut responses = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while responses.len() < 2 && Instant::now() < deadline {
            server.process(&mut host);
            for slot in &mut workers {
                if slot
                    .as_ref()
                    .is_some_and(std::thread::JoinHandle::is_finished)
                {
                    responses.push(slot.take().expect("worker").join().expect("worker"));
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(responses.len(), 2, "both clients must be served");
        for response in responses {
            assert!(
                response
                    .as_object()
                    .and_then(|object| object.get("ok"))
                    .and_then(Json::as_bool)
                    .unwrap_or(false)
            );
        }
        server.shutdown();
    }

    #[test]
    fn download_returns_the_active_envelope() {
        let _state = crate::test_support::state_guard();
        let endpoint = endpoint("download");
        let server = ControlServer::bind(&endpoint, "nonce").expect("bind");
        let artifacts = crate::test_support::artifacts(crate::test_support::SIMPLE, "simple");
        let stored = crate::store::StoredManifest::new(artifacts.manifest).expect("stored");
        let build_id = stored.build_id;
        let mut store = ManifestStore::memory();
        store.install(stored).expect("install");
        let mut host = Host::new(crate::host_platform(), store);
        let worker = std::thread::spawn({
            let endpoint = endpoint.clone();
            move || {
                let mut client =
                    ControlClient::connect(&endpoint, Duration::from_secs(5)).expect("connect");
                client
                    .request(
                        "nonce",
                        Json::object([("op".into(), Json::string("download"))]),
                    )
                    .expect("response")
            }
        });
        let mut response = None;
        for _ in 0..100 {
            server.process(&mut host);
            if worker.is_finished() {
                response = Some(worker.join().expect("worker"));
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let response = response.expect("response");
        let text = response
            .as_object()
            .and_then(|object| object.get("download"))
            .and_then(Json::as_str)
            .expect("download text");
        let download = crate::store::Download::from_json_text(text).expect("envelope");
        assert_eq!(download.deployment.build_id, build_id);
        server.shutdown();
    }

    #[cfg(windows)]
    #[test]
    fn windows_control_endpoint_round_trips() {
        let _state = crate::test_support::state_guard();
        let endpoint = endpoint("roundtrip");
        let server = ControlServer::bind(&endpoint, "nonce").expect("bind");
        let mut host = Host::new(crate::host_platform(), ManifestStore::memory());
        let worker = std::thread::spawn({
            let endpoint = endpoint.clone();
            move || {
                let mut client =
                    ControlClient::connect(&endpoint, Duration::from_secs(5)).expect("connect");
                client
                    .request(
                        "nonce",
                        Json::object([("op".into(), Json::string("status"))]),
                    )
                    .expect("response")
            }
        });
        let mut response = None;
        for _ in 0..100 {
            server.process(&mut host);
            if worker.is_finished() {
                response = Some(worker.join().expect("worker"));
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let response = response.expect("response");
        assert!(
            response
                .as_object()
                .and_then(|object| object.get("ok"))
                .and_then(Json::as_bool)
                .unwrap_or(false)
        );
        server.shutdown();
    }
}
