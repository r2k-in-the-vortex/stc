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
use crate::transport::{ClientConnection, Connection, Listener};

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
        let backend = Listener::bind_pipe(endpoint)?;
        let thread_shared = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("stc-control".into())
            .spawn(move || {
                crate::priority::lower_service_priority();
                serve(backend, thread_shared);
            })
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

fn serve(mut backend: Listener, shared: Arc<Shared>) {
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
            .spawn(move || {
                crate::priority::lower_service_priority();
                serve_connection(connection, &connection_shared);
            });
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
            match ClientConnection::connect(&endpoint) {
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
