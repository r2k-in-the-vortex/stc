//! The synchronous DAP session loop and translation to `stc-debug/1.1`.
//!
//! One adapter instance serves one launch or attach. A reader thread frames
//! standard input and forwards request bodies; the session loop polls the
//! connected [`DebugClient`] for runtime events between requests. Standard
//! output carries DAP messages only; diagnostics and tracing go to stderr.
//!
//! Launch ordering (WL-1.1 §5.3): the adapter answers `initialize`, sends the
//! `initialized` event, and buffers `setBreakpoints`/`configurationDone` until
//! both the launch/attach request and `configurationDone` have arrived. The
//! host is started only at that point, so every breakpoint the client set is
//! applied before the adapter reports the session ready. A breakpoint on the
//! first executable statement is therefore honored at the next checkpoint.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use stc_got::json::Json;
use stc_runtime::{DEBUG_PROTOCOL, DEBUG_PROTOCOL_1_1, DebugClient};

use crate::framing::{FrameReader, MAX_MESSAGE_BYTES, write_message};
use crate::launcher::{DapOptions, Launcher, NON_SAFETY_NOTICE, OutputLine, SessionHost};
use crate::protocol::{self, LaunchArguments, Request};
use crate::source::SourceTable;
use crate::value;

/// How often the session loop checks for runtime events and host output.
const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// Maximum variables returned by one scope or aggregate expansion.
const MAX_SCOPE_CHILDREN: usize = 256;
/// How long a target-exited teardown waits for the host process to be reaped.
const EXIT_WAIT: Duration = Duration::from_secs(2);

/// Runs the adapter over the process standard streams.
///
/// Standard output is reserved for DAP messages; all logging goes to stderr.
/// Returns 0 after a clean disconnect or end of input and 1 after a framing or
/// request-decoding failure.
pub fn run_stdio<L: Launcher>(launcher: &mut L, options: DapOptions) -> u8 {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve(stdin, stdout, launcher, options)
}

/// Runs one adapter session over `input` and `output`.
///
/// `input` must be `'static` because a reader thread owns it. See
/// [`run_stdio`] for the exit-code contract.
pub fn serve<R, W, L>(input: R, mut output: W, launcher: &mut L, options: DapOptions) -> u8
where
    R: Read + Send + 'static,
    W: Write,
    L: Launcher,
{
    let (sender, receiver) = mpsc::channel::<InputEvent>();
    let spawned = std::thread::Builder::new()
        .name("stc-dap-reader".to_owned())
        .spawn(move || {
            let mut reader = FrameReader::new(BufReader::new(input));
            loop {
                match reader.read_frame() {
                    Ok(Some(body)) => {
                        if sender.send(InputEvent::Frame(body)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let _ = sender.send(InputEvent::Malformed(error.to_string()));
                        break;
                    }
                }
            }
        });
    let receiver = match spawned {
        Ok(_) => receiver,
        Err(error) => {
            eprintln!("stc-dap: cannot start the input reader: {error}");
            return 1;
        }
    };

    let mut adapter = Adapter {
        output: &mut output,
        launcher,
        options,
        seq: 0,
        initialized: false,
        configuration_done: false,
        pending: None,
        source_table: None,
        start_attempted: false,
        session: None,
        stop_id: None,
        frames: Vec::new(),
        variables: HashMap::new(),
        next_variable: 1,
        breakpoints: BTreeMap::new(),
        next_breakpoint: 1,
        terminated_sent: false,
        exited_sent: false,
        resumed_pending: false,
    };
    adapter.run(&receiver)
}

enum InputEvent {
    Frame(Vec<u8>),
    Malformed(String),
}

/// One runtime session.
struct Session {
    client: DebugClient,
    host: Option<Box<dyn SessionHost>>,
    /// Negotiated protocol version (`stc-debug/1` or `stc-debug/1.1`).
    protocol: String,
    /// Whether the launch configuration and host granted `write`.
    write_enabled: bool,
    /// Whether the host reports stopped-frame information (`stepping`).
    frames_available: bool,
    source_table: SourceTable,
    globals: GlobalsLayout,
    /// Manifest function `GotId`s mapped to display names for stack frames.
    function_names: BTreeMap<String, String>,
}

/// One requested DAP breakpoint.
#[derive(Clone, Debug)]
struct RequestedBreakpoint {
    id: i64,
    line: i64,
    column: Option<i64>,
    verified: bool,
    message: Option<String>,
}

/// One applied breakpoint result.
struct BreakpointOutcome {
    id: i64,
    verified: bool,
    line: i64,
    column: Option<i64>,
    message: Option<String>,
}

/// All breakpoints for one client source file.
#[derive(Clone, Debug)]
struct SourceBreakpoints {
    absolute: PathBuf,
    protocol_file: Option<String>,
    requested: Vec<RequestedBreakpoint>,
    applied: Vec<u64>,
}

/// One frame returned by `stack.list`.
#[derive(Clone, Debug)]
struct FrameEntry {
    index: u64,
    name: String,
    source: Option<Value>,
    line: u64,
    column: u64,
}

/// One expandable variable container.
#[derive(Clone, Debug)]
enum Container {
    /// The Globals scope: program instances plus top-level global roots.
    Globals(GlobalsLayout),
    /// One program instance's field roots.
    Instance(Vec<GlobalRoot>),
    /// One stopped frame's locals (`stc-debug/1.1` only).
    Locals(u64),
    /// Children of a manifest-assisted view node.
    View(Json),
}

/// Global roots grouped by program instance.
#[derive(Clone, Debug, Default)]
struct GlobalsLayout {
    instances: Vec<InstanceGroup>,
    globals: Vec<GlobalRoot>,
}

/// One configured program instance and its roots.
#[derive(Clone, Debug)]
struct InstanceGroup {
    name: String,
    type_key: String,
    roots: Vec<GlobalRoot>,
}

/// One addressable global root.
#[derive(Clone, Debug)]
struct GlobalRoot {
    name: String,
    key: String,
    type_key: String,
}

struct Adapter<'a, W: Write, L: Launcher> {
    output: &'a mut W,
    launcher: &'a mut L,
    options: DapOptions,
    seq: i64,
    initialized: bool,
    configuration_done: bool,
    pending: Option<(LaunchArguments, bool)>,
    /// Project source mapping loaded by launch/attach, before the host starts.
    source_table: Option<SourceTable>,
    start_attempted: bool,
    session: Option<Session>,
    stop_id: Option<u64>,
    frames: Vec<FrameEntry>,
    variables: HashMap<i64, Container>,
    next_variable: i64,
    breakpoints: BTreeMap<String, SourceBreakpoints>,
    next_breakpoint: i64,
    terminated_sent: bool,
    exited_sent: bool,
    resumed_pending: bool,
}

impl<W: Write, L: Launcher> Adapter<'_, W, L> {
    fn run(&mut self, receiver: &Receiver<InputEvent>) -> u8 {
        loop {
            self.poll_runtime();
            match receiver.recv_timeout(POLL_INTERVAL) {
                Ok(InputEvent::Frame(body)) => match protocol::parse_request(&body) {
                    Ok(request) => {
                        if self.dispatch(&request) {
                            break;
                        }
                    }
                    Err(message) => {
                        eprintln!("stc-dap: {message}");
                        self.send_output("stderr", &format!("stc-dap: {message}"));
                        self.end_session("protocol-error");
                        self.flush();
                        return 1;
                    }
                },
                Ok(InputEvent::Malformed(message)) => {
                    eprintln!("stc-dap: framing: {message}");
                    self.send_output("stderr", &format!("stc-dap: framing: {message}"));
                    self.end_session("protocol-error");
                    self.flush();
                    return 1;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        self.end_session("disconnect");
        0
    }

    fn flush(&mut self) {
        let _ = self.output.flush();
    }

    // -----------------------------------------------------------------------
    // DAP output
    // -----------------------------------------------------------------------

    fn send(&mut self, mut message: Value) {
        self.seq = self.seq.saturating_add(1).max(1);
        if let Some(object) = message.as_object_mut() {
            object.insert("seq".into(), json!(self.seq));
        }
        let text = message.to_string();
        if text.len() > MAX_MESSAGE_BYTES {
            eprintln!("stc-dap: dropping an oversized DAP message");
            return;
        }
        if let Err(error) = write_message(self.output, text.as_bytes()) {
            eprintln!("stc-dap: cannot write to the client: {error}");
        }
    }

    fn respond(
        &mut self,
        request: &Request,
        success: bool,
        body: Option<Value>,
        message: Option<String>,
    ) {
        let response = protocol::response(request.seq, &request.command, success, body, message);
        self.send(response);
    }

    fn fail(&mut self, request: &Request, message: impl Into<String>) {
        self.respond(request, false, None, Some(message.into()));
    }

    fn send_event(&mut self, name: &str, body: Value) {
        let event = protocol::event(name, body);
        self.send(event);
    }

    fn send_output(&mut self, category: &str, text: &str) {
        let mut text = text.to_owned();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        self.send_event("output", json!({ "category": category, "output": text }));
    }

    fn send_exited(&mut self, exit_code: Option<i32>) {
        if self.exited_sent {
            return;
        }
        let Some(exit_code) = exit_code else {
            return;
        };
        self.exited_sent = true;
        self.send_event("exited", json!({ "exitCode": exit_code }));
    }

    fn send_terminated(&mut self) {
        if self.terminated_sent {
            return;
        }
        self.terminated_sent = true;
        self.send_event("terminated", json!({ "restart": false }));
    }

    // -----------------------------------------------------------------------
    // Request dispatch
    // -----------------------------------------------------------------------

    /// Dispatches one request; returns `true` when the adapter must exit.
    fn dispatch(&mut self, request: &Request) -> bool {
        match request.command.as_str() {
            "initialize" => {
                self.initialize(request);
                false
            }
            "launch" => {
                self.launch_or_attach(request, false);
                false
            }
            "attach" => {
                self.launch_or_attach(request, true);
                false
            }
            "configurationDone" => {
                self.configuration_done(request);
                false
            }
            "setBreakpoints" => {
                self.set_breakpoints(request);
                false
            }
            "setExceptionBreakpoints" => {
                self.respond(request, true, Some(json!({ "breakpoints": [] })), None);
                false
            }
            "threads" => {
                self.respond(
                    request,
                    true,
                    Some(json!({ "threads": [{ "id": 1, "name": "cycle" }] })),
                    None,
                );
                false
            }
            "stackTrace" => {
                self.stack_trace(request);
                false
            }
            "scopes" => {
                self.scopes(request);
                false
            }
            "variables" => {
                self.variables(request);
                false
            }
            "setVariable" => {
                self.set_variable(request);
                false
            }
            "evaluate" => {
                self.evaluate(request);
                false
            }
            "continue" => {
                self.resume(request, ResumeKind::Continue);
                false
            }
            "next" => {
                self.resume(request, ResumeKind::Step("over"));
                false
            }
            "stepIn" => {
                self.resume(request, ResumeKind::Step("into"));
                false
            }
            "stepOut" => {
                self.resume(request, ResumeKind::Step("out"));
                false
            }
            "pause" => {
                self.pause(request);
                false
            }
            "terminate" => {
                self.respond(request, true, None, None);
                self.end_session("terminate");
                false
            }
            "disconnect" => {
                self.respond(request, true, None, None);
                self.end_session("disconnect");
                true
            }
            other => {
                self.fail(request, format!("unsupported request `{other}`"));
                false
            }
        }
    }

    fn initialize(&mut self, request: &Request) {
        self.respond(request, true, Some(protocol::capabilities()), None);
        if !self.initialized {
            self.initialized = true;
            self.send_event("initialized", json!({}));
        }
    }

    fn launch_or_attach(&mut self, request: &Request, attach: bool) {
        if self.session.is_some() || self.start_attempted {
            self.fail(request, "a debug session is already active");
            return;
        }
        if self.pending.is_some() {
            self.fail(request, "a launch or attach request is already pending");
            return;
        }
        let arguments: LaunchArguments = if request.arguments.is_null() {
            LaunchArguments::default()
        } else {
            match serde_json::from_value(request.arguments.clone()) {
                Ok(arguments) => arguments,
                Err(error) => {
                    self.fail(request, format!("invalid debug arguments: {error}"));
                    return;
                }
            }
        };
        if arguments.stop_on_entry {
            self.fail(request, "stopOnEntry is not supported");
            return;
        }
        if attach && arguments.endpoint.as_deref().is_none_or(str::is_empty) {
            self.fail(request, "attach requires an `endpoint` file");
            return;
        }
        let project = arguments.project_path(self.options.manifest_path.as_deref());
        let source_root = arguments.source_root.as_deref().map(Path::new);
        let source_table = match SourceTable::from_project(&project, source_root) {
            Ok(table) => table,
            Err(error) => {
                self.fail(
                    request,
                    format!("cannot load the project `{}`: {error}", project.display()),
                );
                return;
            }
        };
        self.source_table = Some(source_table);
        self.resolve_pending_breakpoints();
        self.respond(request, true, Some(json!({})), None);
        self.pending = Some((arguments, attach));
        if self.configuration_done
            && let Err(message) = self.start_pending()
        {
            self.send_output("stderr", &format!("stc: {message}"));
            self.send_terminated();
        }
    }

    fn configuration_done(&mut self, request: &Request) {
        if self.configuration_done {
            self.respond(request, true, None, None);
            return;
        }
        self.configuration_done = true;
        if self.pending.is_some() {
            match self.start_pending() {
                Ok(()) => self.respond(request, true, None, None),
                Err(message) => {
                    self.send_output("stderr", &format!("stc: {message}"));
                    self.respond(request, false, None, Some(message));
                    self.end_session("launch-failed");
                }
            }
        } else {
            self.respond(request, true, None, None);
        }
    }

    /// Starts the pending host. Called only once both the launch/attach
    /// request and `configurationDone` have arrived.
    fn start_pending(&mut self) -> Result<(), String> {
        let Some((arguments, attach)) = self.pending.take() else {
            return Ok(());
        };
        self.start_attempted = true;
        let source_table = match self.source_table.take() {
            Some(table) => table,
            None => {
                let project = arguments.project_path(self.options.manifest_path.as_deref());
                let source_root = arguments.source_root.as_deref().map(Path::new);
                SourceTable::from_project(&project, source_root).map_err(|error| {
                    format!("cannot load the project `{}`: {error}", project.display())
                })?
            }
        };
        let launched = if attach {
            self.launcher.attach(&arguments)?
        } else {
            self.launcher.launch(&arguments)?
        };
        let mut client = launched.client;
        let scopes = session_scopes(&arguments);
        let (start, protocol) = start_protocol_session(&mut client, &scopes)?;
        // The host may grant less than the launch configuration requested
        // (for example, `[debug].allow-write` is off); trust the granted set.
        let capabilities = jget(&start, "capabilities");
        let write_enabled = capabilities
            .and_then(|capabilities| jget(capabilities, "scopes"))
            .and_then(Json::as_array)
            .is_some_and(|scopes| scopes.iter().any(|scope| scope.as_str() == Some("write")));
        let frames_available = capabilities
            .and_then(|capabilities| jget(capabilities, "stepping"))
            .and_then(Json::as_bool)
            .unwrap_or(false);
        let manifest_body = client
            .call_ok(
                "manifest.get",
                got_object([("format", Json::string("json"))]),
            )
            .ok()
            .and_then(|result| {
                jget(&result, "data")
                    .and_then(|data| jget(data, "body"))
                    .cloned()
            });
        let globals = manifest_body
            .as_ref()
            .map_or_else(GlobalsLayout::default, GlobalsLayout::from_manifest);
        let function_names = manifest_body
            .as_ref()
            .map_or_else(BTreeMap::new, function_names_from_manifest);
        self.session = Some(Session {
            client,
            host: launched.host,
            protocol: protocol.clone(),
            write_enabled,
            frames_available,
            source_table,
            globals,
            function_names,
        });
        self.send_output("console", NON_SAFETY_NOTICE);
        if protocol == DEBUG_PROTOCOL_1_1 && frames_available {
            self.send_output(
                "console",
                "stc: debug session started with stc-debug/1.1; the Locals scope is available",
            );
        } else if protocol == DEBUG_PROTOCOL_1_1 {
            self.send_output(
                "console",
                "stc: debug session started with stc-debug/1.1; stopped-frame information is unavailable, so there is no Locals scope",
            );
        } else {
            self.send_output(
                "console",
                "stc: debug session started with stc-debug/1; this host has no Locals scope",
            );
        }
        self.apply_all_breakpoints();
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Breakpoints
    // -----------------------------------------------------------------------

    fn set_breakpoints(&mut self, request: &Request) {
        let Some(source) = request.arguments.get("source") else {
            self.fail(request, "setBreakpoints requires a `source` object");
            return;
        };
        let Some(path) = source.get("path").and_then(Value::as_str) else {
            self.fail(request, "setBreakpoints requires `source.path`");
            return;
        };
        let absolute = PathBuf::from(path);
        let key = crate::source::path_key(&absolute);
        let requested = match collect_requested_breakpoints(request, &mut self.next_breakpoint) {
            Ok(requested) => requested,
            Err(message) => {
                self.fail(request, message);
                return;
            }
        };
        // A source arriving before the launch request cannot be resolved yet:
        // keep it pending and report the state honestly. `launch`/`attach`
        // resolves it and emits a changed event.
        let protocol_file = self
            .session
            .as_ref()
            .map(|session| &session.source_table)
            .or(self.source_table.as_ref())
            .and_then(|table| table.protocol_path(&absolute));
        let mut requested = requested;
        if protocol_file.is_none() {
            let message = if self.session.is_some() || self.source_table.is_some() {
                "the source is not in the project graph"
            } else {
                "waiting for the launch or attach configuration"
            };
            for breakpoint in &mut requested {
                breakpoint.verified = false;
                breakpoint.message = Some(message.to_owned());
            }
        }
        // Update an existing entry in place: `apply_source_breakpoints` needs
        // the previous host `breakpoint_id`s so it can clear them, including
        // when the new list is empty. Replacing the entry would drop those IDs
        // and leave stale host breakpoints active in the live session.
        match self.breakpoints.entry(key.clone()) {
            Entry::Occupied(mut occupied) => {
                let entry = occupied.get_mut();
                entry.absolute = absolute.clone();
                if protocol_file.is_some() {
                    // Keep an already resolved mapping when this request
                    // cannot resolve the source again, so the old host IDs
                    // are still cleared.
                    entry.protocol_file = protocol_file;
                }
                entry.requested = requested;
            }
            Entry::Vacant(vacant) => {
                vacant.insert(SourceBreakpoints {
                    absolute: absolute.clone(),
                    protocol_file,
                    requested,
                    applied: Vec::new(),
                });
            }
        }
        if self.session.is_some() && self.breakpoints[&key].protocol_file.is_some() {
            self.apply_source_breakpoints(&key);
        } else if self.breakpoints[&key].protocol_file.is_some() {
            // Optimistic before the host starts; `apply_source_breakpoints`
            // reports any line that turns out to be unbound.
            if let Some(entry) = self.breakpoints.get_mut(&key) {
                for breakpoint in &mut entry.requested {
                    breakpoint.verified = true;
                }
            }
        }
        let body = self.breakpoints_response(&absolute, &key);
        self.respond(request, true, Some(body), None);
    }

    /// Re-resolves breakpoints received before the launch configuration.
    fn resolve_pending_breakpoints(&mut self) {
        let Some(table) = self.source_table.clone() else {
            return;
        };
        let keys: Vec<String> = self.breakpoints.keys().cloned().collect();
        let mut changed = Vec::new();
        for key in keys {
            let Some(entry) = self.breakpoints.get_mut(&key) else {
                continue;
            };
            if entry.protocol_file.is_some() {
                continue;
            }
            entry.protocol_file = table.protocol_path(&entry.absolute);
            if entry.protocol_file.is_none() {
                continue;
            }
            for breakpoint in &mut entry.requested {
                breakpoint.verified = true;
                breakpoint.message = None;
                changed.push((
                    breakpoint.id,
                    breakpoint.line,
                    breakpoint.column,
                    entry.absolute.clone(),
                ));
            }
        }
        for (id, line, column, absolute) in changed {
            self.send_breakpoint_event(id, true, line, column, &absolute, None);
        }
    }

    fn breakpoints_response(&self, absolute: &Path, key: &str) -> Value {
        let source = source_object(absolute, None);
        let breakpoints = self.breakpoints.get(key).map_or_else(Vec::new, |entry| {
            entry
                .requested
                .iter()
                .map(|breakpoint| {
                    let mut object = serde_json::Map::new();
                    object.insert("id".into(), json!(breakpoint.id));
                    object.insert("verified".into(), json!(breakpoint.verified));
                    object.insert("line".into(), json!(breakpoint.line));
                    if let Some(column) = breakpoint.column {
                        object.insert("column".into(), json!(column));
                    }
                    object.insert("source".into(), source.clone());
                    if let Some(message) = &breakpoint.message {
                        object.insert("message".into(), json!(message));
                    }
                    Value::Object(object)
                })
                .collect()
        });
        json!({ "breakpoints": breakpoints })
    }

    fn apply_all_breakpoints(&mut self) {
        let keys: Vec<String> = self.breakpoints.keys().cloned().collect();
        for key in keys {
            self.apply_source_breakpoints(&key);
        }
    }

    fn apply_source_breakpoints(&mut self, key: &str) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let Some(entry) = self.breakpoints.get(key) else {
            return;
        };
        let Some(file) = entry.protocol_file.clone() else {
            return;
        };
        let previous = entry.applied.clone();
        let requested: Vec<(i64, i64, Option<i64>)> = entry
            .requested
            .iter()
            .map(|breakpoint| (breakpoint.id, breakpoint.line, breakpoint.column))
            .collect();
        for id in previous {
            let _ = session.client.call_ok(
                "breakpoint.clear",
                got_object([("breakpoint_id", Json::unsigned(id))]),
            );
        }
        let mut applied = Vec::new();
        let mut results: Vec<BreakpointOutcome> = Vec::new();
        for (dap_id, line, column) in requested {
            let params = got_object([(
                "target",
                got_object([
                    ("kind", Json::string("source")),
                    ("file", Json::string(file.clone())),
                    ("line", Json::unsigned(u64::try_from(line).unwrap_or(0))),
                    (
                        "column",
                        Json::unsigned(column.and_then(|c| u64::try_from(c).ok()).unwrap_or(0)),
                    ),
                ]),
            )]);
            match session.client.call_ok("breakpoint.set", params) {
                Ok(result) => {
                    let bound = jget(&result, "bound")
                        .and_then(Json::as_bool)
                        .unwrap_or(false);
                    let protocol_id = jget(&result, "breakpoint_id")
                        .and_then(Json::as_number)
                        .and_then(|id| u64::try_from(id).ok());
                    if let Some(id) = protocol_id {
                        applied.push(id);
                    }
                    let source = jget(&result, "source");
                    let actual_line = source
                        .and_then(|source| jget(source, "line"))
                        .and_then(Json::as_number)
                        .and_then(|line| i64::try_from(line).ok())
                        .unwrap_or(line);
                    let actual_column = source
                        .and_then(|source| jget(source, "column"))
                        .and_then(Json::as_number)
                        .and_then(|column| i64::try_from(column).ok())
                        .or(column);
                    let message = (!bound).then(|| {
                        "unbound: no checkpoint at this source location (STC-W-DEBUG-001)"
                            .to_owned()
                    });
                    results.push(BreakpointOutcome {
                        id: dap_id,
                        verified: bound,
                        line: actual_line,
                        column: actual_column,
                        message,
                    });
                }
                Err(error) => {
                    results.push(BreakpointOutcome {
                        id: dap_id,
                        verified: false,
                        line,
                        column,
                        message: Some(error.to_string()),
                    });
                }
            }
        }
        if let Some(entry) = self.breakpoints.get_mut(key) {
            entry.applied = applied;
            for (index, result) in results.iter().enumerate() {
                if let Some(breakpoint) = entry.requested.get_mut(index) {
                    breakpoint.verified = result.verified;
                    breakpoint.message.clone_from(&result.message);
                }
            }
        }
        let absolute = self
            .breakpoints
            .get(key)
            .map(|entry| entry.absolute.clone())
            .unwrap_or_default();
        for result in results {
            if result.verified {
                continue;
            }
            self.send_breakpoint_event(
                result.id,
                result.verified,
                result.line,
                result.column,
                &absolute,
                result.message,
            );
        }
    }

    fn send_breakpoint_event(
        &mut self,
        id: i64,
        verified: bool,
        line: i64,
        column: Option<i64>,
        absolute: &Path,
        message: Option<String>,
    ) {
        let mut breakpoint = serde_json::Map::new();
        breakpoint.insert("id".into(), json!(id));
        breakpoint.insert("verified".into(), json!(verified));
        breakpoint.insert("line".into(), json!(line));
        if let Some(column) = column {
            breakpoint.insert("column".into(), json!(column));
        }
        breakpoint.insert("source".into(), source_object(absolute, None));
        if let Some(message) = message {
            breakpoint.insert("message".into(), json!(message));
        }
        self.send_event(
            "breakpoint",
            json!({ "reason": "changed", "breakpoint": breakpoint }),
        );
    }

    // -----------------------------------------------------------------------
    // Runtime event polling
    // -----------------------------------------------------------------------

    fn poll_runtime(&mut self) {
        let mut messages = Vec::new();
        let mut outputs: Vec<OutputLine> = Vec::new();
        let mut failed = None;
        let mut exited = None;
        if let Some(session) = self.session.as_mut() {
            loop {
                match session.client.poll_message() {
                    Ok(Some(message)) => messages.push(message),
                    Ok(None) => break,
                    Err(error) => {
                        failed = Some(error.to_string());
                        break;
                    }
                }
            }
            messages.extend(session.client.take_events());
            if let Some(host) = session.host.as_mut() {
                outputs = host.drain_output();
                exited = host.try_exit();
            }
        }
        for message in &messages {
            self.handle_runtime_message(message);
        }
        for line in outputs {
            self.send_output(line.category, &line.text);
        }
        if let Some(error) = failed {
            self.send_output(
                "stderr",
                &format!("stc: the debug connection failed: {error}"),
            );
            self.end_session("transport-error");
        } else if let Some(code) = exited
            && self.session.is_some()
        {
            self.end_session_with_exit("target-exited", Some(code));
        }
    }

    fn handle_runtime_message(&mut self, message: &Json) {
        if message
            .as_object()
            .and_then(|object| object.get("kind"))
            .and_then(Json::as_str)
            != Some("event")
        {
            return;
        }
        let name = message
            .as_object()
            .and_then(|object| object.get("event"))
            .and_then(Json::as_str)
            .unwrap_or("");
        let data = message
            .as_object()
            .and_then(|object| object.get("data"))
            .cloned()
            .unwrap_or(Json::Null);
        match name {
            "stopped" => {
                let stop_id = jget(&data, "stop_id")
                    .and_then(Json::as_number)
                    .and_then(|id| u64::try_from(id).ok());
                let Some(stop_id) = stop_id else {
                    return;
                };
                let reason = jget(&data, "reason")
                    .and_then(Json::as_str)
                    .unwrap_or("breakpoint");
                self.stop_id = Some(stop_id);
                self.frames.clear();
                self.variables.clear();
                let mut body = json!({
                    "reason": reason,
                    "threadId": 1,
                    "allThreadsStopped": true,
                });
                if let Some(source) = jget(&data, "source") {
                    let file = jget(source, "file").and_then(Json::as_str).unwrap_or("?");
                    let line = jget(source, "line").and_then(Json::as_number).unwrap_or(0);
                    body["description"] = json!(format!("{file}:{line}"));
                }
                self.send_event("stopped", body);
            }
            "resumed" => {
                self.stop_id = None;
                self.frames.clear();
                self.variables.clear();
                if !self.resumed_pending {
                    self.send_event(
                        "continued",
                        json!({ "threadId": 1, "allThreadsContinued": true }),
                    );
                }
                self.resumed_pending = false;
            }
            "session-ended" => {
                let reason = jget(&data, "reason")
                    .and_then(Json::as_str)
                    .unwrap_or("internal");
                let code = jget(&data, "code").and_then(Json::as_str);
                if reason != "client" {
                    let suffix = code.map_or_else(String::new, |code| format!(" ({code})"));
                    self.send_output(
                        "stderr",
                        &format!("stc: the debug session ended: {reason}{suffix}"),
                    );
                }
                self.end_session(reason);
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Stack, scopes, variables, and evaluate
    // -----------------------------------------------------------------------

    fn stack_trace(&mut self, request: &Request) {
        let Some(stop_id) = self.stop_id else {
            self.fail(request, "the target is not stopped");
            return;
        };
        let Some(session) = self.session.as_mut() else {
            self.fail(request, "no debug session is active");
            return;
        };
        let result = match session.client.call_ok(
            "stack.list",
            got_object([("stop_id", Json::unsigned(stop_id))]),
        ) {
            Ok(result) => result,
            Err(error) => {
                self.fail(request, error.to_string());
                return;
            }
        };
        let function_names = session.function_names.clone();
        let truncated = jget(&result, "truncated")
            .and_then(Json::as_bool)
            .unwrap_or(false);
        let mut frames = Vec::new();
        let mut top_resolved = false;
        if let Some(entries) = jget(&result, "frames").and_then(Json::as_array) {
            for (position, entry) in entries.iter().enumerate() {
                // Only frames with a resolved manifest function identity are
                // ST frames. The runtime reports launcher/host frames outside
                // the manifest code ranges with a null `function` (and
                // `optimized_out: true`); they are not part of the ST program
                // and must not appear in the call stack. Do not filter on
                // `source`: resolved ST caller frames legitimately have no
                // source location (F-11).
                let Some(function) = jget(entry, "function").and_then(Json::as_str) else {
                    continue;
                };
                let Some(name) = function_names.get(function).cloned() else {
                    continue;
                };
                if position == 0 {
                    top_resolved = true;
                }
                let index = jget(entry, "index")
                    .and_then(Json::as_number)
                    .and_then(|index| u64::try_from(index).ok())
                    .unwrap_or(0);
                let source = jget(entry, "source");
                let file = source
                    .and_then(|source| jget(source, "file"))
                    .and_then(Json::as_str);
                let line = source
                    .and_then(|source| jget(source, "line"))
                    .and_then(Json::as_number)
                    .and_then(|line| u64::try_from(line).ok())
                    .unwrap_or(0);
                let column = source
                    .and_then(|source| jget(source, "column"))
                    .and_then(Json::as_number)
                    .and_then(|column| u64::try_from(column).ok())
                    .unwrap_or(0);
                let source_object = file.map(|file| self.frame_source(file));
                frames.push(FrameEntry {
                    index,
                    name,
                    source: source_object,
                    line,
                    column,
                });
            }
        }
        if !top_resolved {
            // The stopped frame itself is not a resolved ST frame (for
            // example the manifest is unavailable). Caller frames would then
            // be misreported as the stop location, so list no frames and
            // fabricate none; an empty `stackTrace` is valid DAP.
            frames.clear();
        }
        self.frames = frames;
        let start = request
            .arguments
            .get("startFrame")
            .and_then(Value::as_u64)
            .map_or(0, |start| usize::try_from(start).unwrap_or(usize::MAX));
        let levels = request
            .arguments
            .get("levels")
            .and_then(Value::as_u64)
            .map_or(0, |levels| usize::try_from(levels).unwrap_or(usize::MAX));
        let len = self.frames.len();
        let start = start.min(len);
        let end = if levels == 0 {
            len
        } else {
            start.saturating_add(levels).min(len)
        };
        let stack_frames: Vec<Value> = self.frames[start..end]
            .iter()
            .map(|frame| {
                let mut object = serde_json::Map::new();
                object.insert("id".into(), json!(frame.index));
                object.insert("name".into(), json!(frame.name));
                object.insert("line".into(), json!(frame.line));
                object.insert("column".into(), json!(frame.column));
                if let Some(source) = &frame.source {
                    object.insert("source".into(), source.clone());
                }
                Value::Object(object)
            })
            .collect();
        let mut body = json!({ "stackFrames": stack_frames });
        if !truncated {
            body["totalFrames"] = json!(len);
        }
        self.respond(request, true, Some(body), None);
    }

    /// Maps one protocol source file to a DAP `Source` object.
    fn frame_source(&self, file: &str) -> Value {
        let Some(session) = self.session.as_ref() else {
            return json!({ "name": file, "path": file });
        };
        match session.source_table.absolute_path(file) {
            Some(path) => source_object(&path, Some(file)),
            None => json!({ "name": file, "path": file }),
        }
    }

    fn scopes(&mut self, request: &Request) {
        if self.stop_id.is_none() {
            self.fail(request, "the target is not stopped");
            return;
        }
        let frame = request
            .arguments
            .get("frameId")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let Some(session) = self.session.as_ref() else {
            self.fail(request, "no debug session is active");
            return;
        };
        let protocol_1_1 = session.protocol == DEBUG_PROTOCOL_1_1;
        let locals_available = protocol_1_1 && session.frames_available;
        let globals = session.globals.clone();
        let mut scopes = Vec::new();
        if locals_available {
            let reference = self.new_container(Container::Locals(frame));
            scopes.push(json!({
                "name": "Locals",
                "variablesReference": reference,
                "expensive": false,
            }));
        }
        let reference = self.new_container(Container::Globals(globals));
        scopes.push(json!({
            "name": "Globals",
            "variablesReference": reference,
            "expensive": true,
        }));
        self.respond(request, true, Some(json!({ "scopes": scopes })), None);
    }

    fn variables(&mut self, request: &Request) {
        let Some(reference) = request
            .arguments
            .get("variablesReference")
            .and_then(Value::as_i64)
        else {
            self.fail(request, "variables requires `variablesReference`");
            return;
        };
        let Some(container) = self.variables.get(&reference).cloned() else {
            self.fail(request, "unknown variablesReference");
            return;
        };
        let variables = match container {
            Container::Globals(layout) => self.globals_variables(&layout),
            Container::Instance(roots) => self.root_variables(&roots),
            Container::Locals(frame) => self.local_variables(frame),
            Container::View(node) => self.view_variables(&node),
        };
        match variables {
            Ok(variables) => {
                self.respond(request, true, Some(json!({ "variables": variables })), None)
            }
            Err(message) => self.fail(request, message),
        }
    }

    fn new_container(&mut self, container: Container) -> i64 {
        let reference = self.next_variable;
        self.next_variable = self.next_variable.saturating_add(1);
        self.variables.insert(reference, container);
        reference
    }

    fn globals_variables(&mut self, layout: &GlobalsLayout) -> Result<Vec<Value>, String> {
        let mut variables = Vec::new();
        for instance in &layout.instances {
            if variables.len() >= MAX_SCOPE_CHILDREN {
                break;
            }
            let reference = self.new_container(Container::Instance(instance.roots.clone()));
            variables.push(json!({
                "name": instance.name,
                "value": "<program instance>",
                "type": instance.type_key,
                "variablesReference": reference,
            }));
        }
        self.append_root_variables(&mut variables, &layout.globals);
        Ok(variables)
    }

    fn root_variables(&mut self, roots: &[GlobalRoot]) -> Result<Vec<Value>, String> {
        let mut variables = Vec::new();
        self.append_root_variables(&mut variables, roots);
        Ok(variables)
    }

    fn append_root_variables(&mut self, variables: &mut Vec<Value>, roots: &[GlobalRoot]) {
        for (rendered, root) in roots.iter().enumerate() {
            if variables.len() >= MAX_SCOPE_CHILDREN {
                variables.push(truncation_variable(roots.len().saturating_sub(rendered)));
                break;
            }
            let (text, type_key, reference) = match self.read_target(&root.key) {
                Ok(result) => {
                    let typed = jget(&result, "value");
                    let text =
                        typed.map_or_else(|| "<unavailable>".into(), value::typed_value_text);
                    let type_key =
                        typed.map_or_else(|| root.type_key.clone(), value::typed_value_type);
                    let reference = jget(&result, "view")
                        .and_then(|view| jget(view, "view"))
                        .filter(|node| value::view_node_expandable(node))
                        .map_or(0, |node| self.new_container(Container::View(node.clone())));
                    (text, type_key, reference)
                }
                Err(message) => (
                    format!("<unavailable: {message}>"),
                    root.type_key.clone(),
                    0,
                ),
            };
            let mut variable = serde_json::Map::new();
            variable.insert("name".into(), json!(root.name));
            variable.insert("value".into(), json!(text));
            variable.insert("type".into(), json!(type_key));
            variable.insert("variablesReference".into(), json!(reference));
            variable.insert("evaluateName".into(), json!(root.key));
            variables.push(Value::Object(variable));
        }
    }

    fn local_variables(&mut self, frame: u64) -> Result<Vec<Value>, String> {
        let stop_id = self
            .stop_id
            .ok_or_else(|| "the target is not stopped".to_owned())?;
        let Some(session) = self.session.as_mut() else {
            return Err("no debug session is active".into());
        };
        let locals = session
            .client
            .call_ok(
                "frame.locals",
                got_object([
                    ("stop_id", Json::unsigned(stop_id)),
                    ("frame", Json::unsigned(frame)),
                ]),
            )
            .map_err(|error| error.to_string())?;
        let entries = jget(&locals, "locals")
            .and_then(Json::as_array)
            .map_or_else(Vec::new, <[Json]>::to_vec);
        let mut variables = Vec::new();
        for local in &entries {
            if variables.len() >= MAX_SCOPE_CHILDREN {
                variables.push(truncation_variable(
                    entries.len().saturating_sub(MAX_SCOPE_CHILDREN),
                ));
                break;
            }
            let name = jget(local, "name")
                .and_then(Json::as_str)
                .unwrap_or("?")
                .to_owned();
            let type_key = jget(local, "type")
                .and_then(Json::as_str)
                .unwrap_or("UNKNOWN")
                .to_owned();
            let available = jget(local, "available")
                .and_then(Json::as_bool)
                .unwrap_or(false);
            let live = jget(local, "live").and_then(Json::as_bool).unwrap_or(false);
            let (text, reference) = if available {
                match self.read_local(frame, stop_id, &name) {
                    Ok(result) => {
                        let typed = jget(&result, "value");
                        let text =
                            typed.map_or_else(|| "<unavailable>".into(), value::typed_value_text);
                        let reference = jget(&result, "view")
                            .and_then(|view| jget(view, "view"))
                            .filter(|node| value::view_node_expandable(node))
                            .map_or(0, |node| self.new_container(Container::View(node.clone())));
                        (text, reference)
                    }
                    Err(message) => (format!("<unavailable: {message}>"), 0),
                }
            } else if live {
                ("<unavailable>".to_owned(), 0)
            } else {
                ("<not live at this stop>".to_owned(), 0)
            };
            let mut variable = serde_json::Map::new();
            variable.insert("name".into(), json!(name));
            variable.insert("value".into(), json!(text));
            variable.insert("type".into(), json!(type_key));
            variable.insert("variablesReference".into(), json!(reference));
            variables.push(Value::Object(variable));
        }
        Ok(variables)
    }

    fn view_variables(&mut self, node: &Json) -> Result<Vec<Value>, String> {
        let children = value::view_children(node);
        let total = children.len();
        let mut variables = Vec::new();
        for child in children {
            if variables.len() >= MAX_SCOPE_CHILDREN {
                variables.push(truncation_variable(
                    total.saturating_sub(MAX_SCOPE_CHILDREN),
                ));
                break;
            }
            let type_key = jget(child.node, "type")
                .and_then(Json::as_str)
                .unwrap_or("UNKNOWN")
                .to_owned();
            let reference = value::view_node_expandable(child.node)
                .then(|| self.new_container(Container::View(child.node.clone())));
            let mut variable = serde_json::Map::new();
            variable.insert("name".into(), json!(child.name));
            variable.insert("value".into(), json!(value::view_node_text(child.node)));
            variable.insert("type".into(), json!(type_key));
            variable.insert("variablesReference".into(), json!(reference.unwrap_or(0)));
            if let Some(key) = jget(child.node, "root")
                .and_then(|root| jget(root, "key"))
                .and_then(Json::as_str)
            {
                variable.insert("evaluateName".into(), json!(key));
            }
            variables.push(Value::Object(variable));
        }
        Ok(variables)
    }

    fn read_target(&mut self, key: &str) -> Result<Json, String> {
        let Some(session) = self.session.as_mut() else {
            return Err("no debug session is active".into());
        };
        session
            .client
            .call_ok(
                "var.read",
                got_object([(
                    "target",
                    got_object([("kind", Json::string("path")), ("key", Json::string(key))]),
                )]),
            )
            .map_err(|error| error.to_string())
    }

    fn read_local(&mut self, frame: u64, stop_id: u64, name: &str) -> Result<Json, String> {
        let Some(session) = self.session.as_mut() else {
            return Err("no debug session is active".into());
        };
        session
            .client
            .call_ok(
                "frame.read",
                got_object([
                    (
                        "target",
                        got_object([
                            ("kind", Json::string("local")),
                            ("frame", Json::unsigned(frame)),
                            ("name", Json::string(name)),
                        ]),
                    ),
                    ("stop_id", Json::unsigned(stop_id)),
                ]),
            )
            .map_err(|error| error.to_string())
    }

    fn set_variable(&mut self, request: &Request) {
        let Some(reference) = request
            .arguments
            .get("variablesReference")
            .and_then(Value::as_i64)
        else {
            self.fail(request, "setVariable requires `variablesReference`");
            return;
        };
        let Some(name) = request.arguments.get("name").and_then(Value::as_str) else {
            self.fail(request, "setVariable requires `name`");
            return;
        };
        let Some(text) = request.arguments.get("value").and_then(Value::as_str) else {
            self.fail(request, "setVariable requires a string `value`");
            return;
        };
        let Some(session) = self.session.as_ref() else {
            self.fail(request, "no debug session is active");
            return;
        };
        if !session.write_enabled {
            self.fail(
                request,
                "debug writes are not enabled; set `allowWrite` and the host's `[debug].allow-write`",
            );
            return;
        }
        let root = match self.variables.get(&reference) {
            Some(Container::Globals(layout)) => find_root(&layout.globals, name).cloned(),
            Some(Container::Instance(roots)) => find_root(roots, name).cloned(),
            // Live split fields carry their own runtime root; stopped-frame
            // and contiguous snapshot views do not.
            Some(Container::View(node)) => view_root(node, name),
            Some(Container::Locals(_)) => {
                self.fail(request, "stopped-frame variables are read-only snapshots");
                return;
            }
            None => {
                self.fail(request, "unknown variablesReference");
                return;
            }
        };
        let Some(root) = root else {
            self.fail(
                request,
                format!("`{name}` is not a writable live root in this scope"),
            );
            return;
        };
        let wire = match parse_write_value(&root.type_key, text) {
            Ok(wire) => wire,
            Err(message) => {
                self.fail(request, message);
                return;
            }
        };
        let value_envelope = Json::object([
            ("type".into(), Json::string(root.type_key.clone())),
            ("value".into(), wire.clone()),
        ]);
        let rendered = value::typed_value_text(&value_envelope);
        let params = got_object([
            (
                "target",
                got_object([
                    ("kind", Json::string("path")),
                    ("key", Json::string(root.key.clone())),
                ]),
            ),
            ("value", value_envelope),
        ]);
        let Some(session) = self.session.as_mut() else {
            self.fail(request, "no debug session is active");
            return;
        };
        if let Err(error) = session.client.call_ok("var.write", params) {
            self.fail(request, error.to_string());
            return;
        }
        self.respond(
            request,
            true,
            Some(json!({
                "value": rendered,
                "type": root.type_key,
                "variablesReference": 0,
            })),
            None,
        );
    }

    fn evaluate(&mut self, request: &Request) {
        let Some(expression) = request.arguments.get("expression").and_then(Value::as_str) else {
            self.fail(request, "evaluate requires `expression`");
            return;
        };
        let expression = expression.trim();
        if expression.is_empty() {
            self.fail(request, "evaluate requires a non-empty `expression`");
            return;
        }
        let frame = request
            .arguments
            .get("frameId")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let mut last_error = None;
        if let Some(stop_id) = self.stop_id {
            match self.read_local(frame, stop_id, expression) {
                Ok(result) => {
                    self.evaluate_result(request, &result);
                    return;
                }
                Err(message) => last_error = Some(message),
            }
        }
        // Watch/evaluate also accepts a visible global or program-instance
        // field by name; the raw target path alone would need `MAIN.SUM`.
        let root = self
            .session
            .as_ref()
            .and_then(|session| {
                find_root(&session.globals.globals, expression).or_else(|| {
                    session
                        .globals
                        .instances
                        .iter()
                        .find_map(|instance| find_root(&instance.roots, expression))
                })
            })
            .cloned();
        if let Some(root) = root {
            match self.read_target(&root.key) {
                Ok(result) => {
                    self.evaluate_result(request, &result);
                    return;
                }
                Err(message) => last_error = Some(message),
            }
        }
        match self.read_target(expression) {
            Ok(result) => self.evaluate_result(request, &result),
            Err(message) => {
                let detail = last_error.map_or(message.clone(), |local| {
                    format!("{message}; local: {local}")
                });
                self.fail(
                    request,
                    format!(
                        "`{expression}` is not a stopped-frame local name or a manifest target path ({detail}); \
                         ST expressions are not supported"
                    ),
                );
            }
        }
    }

    fn evaluate_result(&mut self, request: &Request, result: &Json) {
        let typed = jget(result, "value");
        let text = typed.map_or_else(|| "<unavailable>".into(), value::typed_value_text);
        let type_key = typed.map_or_else(|| "UNKNOWN".into(), value::typed_value_type);
        let reference = jget(result, "view")
            .and_then(|view| jget(view, "view"))
            .filter(|node| value::view_node_expandable(node))
            .map_or(0, |node| self.new_container(Container::View(node.clone())));
        self.respond(
            request,
            true,
            Some(json!({
                "result": text,
                "type": type_key,
                "variablesReference": reference,
            })),
            None,
        );
    }

    // -----------------------------------------------------------------------
    // Execution and teardown
    // -----------------------------------------------------------------------

    fn resume(&mut self, request: &Request, kind: ResumeKind) {
        let Some(stop_id) = self.stop_id else {
            self.fail(request, "the target is not stopped");
            return;
        };
        let Some(session) = self.session.as_mut() else {
            self.fail(request, "no debug session is active");
            return;
        };
        let result = match kind {
            ResumeKind::Continue => session.client.call_ok(
                "exec.continue",
                got_object([("stop_id", Json::unsigned(stop_id))]),
            ),
            ResumeKind::Step(mode) => session.client.call_ok(
                "exec.step",
                got_object([
                    ("mode", Json::string(mode)),
                    ("stop_id", Json::unsigned(stop_id)),
                ]),
            ),
        };
        if let Err(error) = result {
            self.fail(request, error.to_string());
            return;
        }
        self.stop_id = None;
        self.frames.clear();
        self.variables.clear();
        let body = if matches!(kind, ResumeKind::Continue) {
            Some(json!({ "allThreadsContinued": true }))
        } else {
            None
        };
        self.respond(request, true, body, None);
        self.send_event(
            "continued",
            json!({ "threadId": 1, "allThreadsContinued": true }),
        );
        self.resumed_pending = true;
        self.poll_runtime();
    }

    fn pause(&mut self, request: &Request) {
        let Some(session) = self.session.as_mut() else {
            self.fail(request, "no debug session is active");
            return;
        };
        match session.client.call_ok("exec.pause", got_object([])) {
            Ok(_) => self.respond(request, true, None, None),
            Err(error) => self.fail(request, error.to_string()),
        }
    }

    /// Ends the runtime session and emits `terminated`/`exited`.
    fn end_session(&mut self, reason: &str) {
        self.end_session_with_exit(reason, None);
    }

    fn end_session_with_exit(&mut self, reason: &str, exit_code: Option<i32>) {
        let Some(mut session) = self.session.take() else {
            self.send_terminated();
            return;
        };
        self.stop_id = None;
        self.frames.clear();
        self.variables.clear();
        let _ = session.client.call_messages("session.stop", got_object([]));
        let mut exit_code = exit_code;
        if let Some(mut host) = session.host.take() {
            // A target exit reports its own code; other teardown paths stop
            // the host through the control channel without waiting first.
            if exit_code.is_none() && reason == "target-exited" {
                exit_code = wait_exit(host.as_mut(), EXIT_WAIT);
            }
            host.shutdown();
            if exit_code.is_none() && reason == "target-exited" {
                exit_code = host.try_exit();
            }
        }
        if !matches!(reason, "client" | "disconnect" | "terminate") {
            self.send_output("stderr", &format!("stc: the debug session ended: {reason}"));
        }
        self.send_exited(exit_code);
        self.send_terminated();
    }
}

enum ResumeKind {
    Continue,
    Step(&'static str),
}

fn truncation_variable(omitted: usize) -> Value {
    json!({
        "name": "(truncated)",
        "value": format!("{omitted} more variable(s) were not shown"),
        "variablesReference": 0,
    })
}

/// Returns the backing root of a live view child, when it has one.
fn view_root(node: &Json, name: &str) -> Option<GlobalRoot> {
    let child = value::view_children(node)
        .into_iter()
        .find(|child| child.name == name)?;
    let key = jget(child.node, "root")
        .and_then(|root| jget(root, "key"))
        .and_then(Json::as_str)?;
    let type_key = jget(child.node, "type")
        .and_then(Json::as_str)
        .unwrap_or("UNKNOWN");
    Some(GlobalRoot {
        name: name.to_owned(),
        key: key.to_owned(),
        type_key: type_key.to_owned(),
    })
}

fn find_root<'a>(roots: &'a [GlobalRoot], name: &str) -> Option<&'a GlobalRoot> {
    roots.iter().find(|root| root.name == name).or_else(|| {
        roots
            .iter()
            .find(|root| root.name.eq_ignore_ascii_case(name))
    })
}

fn source_object(absolute: &Path, name: Option<&str>) -> Value {
    let name = name.map_or_else(
        || {
            absolute.file_name().map_or_else(
                || absolute.to_string_lossy().into_owned(),
                |name| name.to_string_lossy().into_owned(),
            )
        },
        str::to_owned,
    );
    json!({ "name": name, "path": absolute.to_string_lossy() })
}

fn session_scopes(arguments: &LaunchArguments) -> Vec<&'static str> {
    let mut scopes = vec!["observe", "control"];
    if arguments.allow_write {
        scopes.push("write");
    }
    if arguments.allow_force {
        scopes.push("force");
    }
    scopes
}

fn session_start_parameters(scopes: &[&str]) -> Json {
    got_object([
        (
            "client",
            got_object([
                ("name", Json::string("stc-dap")),
                ("version", Json::string(env!("CARGO_PKG_VERSION"))),
            ]),
        ),
        (
            "scopes",
            Json::Array(scopes.iter().map(|scope| Json::string(*scope)).collect()),
        ),
    ])
}

/// Starts the debug session at 1.1 and falls back once to 1.0 when the host
/// rejects the revision with `STC-E-DEBUG-002`.
fn start_protocol_session(
    client: &mut DebugClient,
    scopes: &[&str],
) -> Result<(Json, String), String> {
    let parameters = session_start_parameters(scopes);
    client.set_protocol_version(DEBUG_PROTOCOL_1_1);
    match client.call_ok("session.start", parameters.clone()) {
        Ok(result) => Ok((result, DEBUG_PROTOCOL_1_1.to_owned())),
        Err(error) if error.code.as_deref() == Some("STC-E-DEBUG-002") => {
            client.set_protocol_version(DEBUG_PROTOCOL);
            let result = client
                .call_ok("session.start", parameters)
                .map_err(|error| error.to_string())?;
            Ok((result, DEBUG_PROTOCOL.to_owned()))
        }
        Err(error) => Err(error.to_string()),
    }
}

fn collect_requested_breakpoints(
    request: &Request,
    next_id: &mut i64,
) -> Result<Vec<RequestedBreakpoint>, String> {
    let mut requested = Vec::new();
    if let Some(breakpoints) = request
        .arguments
        .get("breakpoints")
        .and_then(Value::as_array)
    {
        for breakpoint in breakpoints {
            let Some(line) = breakpoint.get("line").and_then(Value::as_i64) else {
                return Err("every breakpoint requires an integer `line`".into());
            };
            if line <= 0 {
                return Err("a breakpoint line must be a positive integer".into());
            }
            let column = breakpoint.get("column").and_then(Value::as_i64);
            let id = *next_id;
            *next_id = next_id.saturating_add(1);
            requested.push(RequestedBreakpoint {
                id,
                line,
                column,
                verified: false,
                message: None,
            });
        }
        return Ok(requested);
    }
    if let Some(lines) = request.arguments.get("lines").and_then(Value::as_array) {
        for line in lines {
            let Some(line) = line.as_i64() else {
                return Err("every entry in `lines` must be an integer".into());
            };
            if line <= 0 {
                return Err("a breakpoint line must be a positive integer".into());
            }
            let id = *next_id;
            *next_id = next_id.saturating_add(1);
            requested.push(RequestedBreakpoint {
                id,
                line,
                column: None,
                verified: false,
                message: None,
            });
        }
    }
    Ok(requested)
}

/// Parses a DAP `setVariable` text into the §15.5 typed value form.
fn parse_write_value(type_key: &str, text: &str) -> Result<Json, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("a value is required".into());
    }
    let parsed_type = stc_got::layout::TypeKey::parse(type_key).ok();
    let is_bool = matches!(parsed_type, Some(stc_got::layout::TypeKey::Bool));
    let is_real = matches!(
        parsed_type,
        Some(stc_got::layout::TypeKey::Real | stc_got::layout::TypeKey::Lreal)
    );
    let is_string = matches!(parsed_type, Some(stc_got::layout::TypeKey::String { .. }));
    if is_bool || (!is_real && !is_string && text.eq_ignore_ascii_case("true")) {
        return Ok(Json::Bool(true));
    }
    if is_bool || (!is_real && !is_string && text.eq_ignore_ascii_case("false")) {
        return Ok(Json::Bool(false));
    }
    if (text.starts_with('\'') && text.ends_with('\''))
        || (text.starts_with('"') && text.ends_with('"'))
    {
        if text.len() < 2 {
            return Err(format!("`{text}` is not a quoted string"));
        }
        let units: Vec<Json> = text[1..text.len() - 1]
            .chars()
            .map(|character| Json::unsigned(u64::from(character as u32)))
            .collect();
        return Ok(Json::object([
            (
                "length".into(),
                Json::unsigned(u64::try_from(units.len()).unwrap_or(u64::MAX)),
            ),
            ("units".into(), Json::Array(units)),
        ]));
    }
    if is_real {
        if text.eq_ignore_ascii_case("nan") {
            return Ok(Json::string("NaN"));
        }
        if text.eq_ignore_ascii_case("inf") || text.eq_ignore_ascii_case("+inf") {
            return Ok(Json::string("INF"));
        }
        if text.eq_ignore_ascii_case("-inf") {
            return Ok(Json::string("-INF"));
        }
        if text.parse::<f64>().is_ok_and(f64::is_finite) {
            return Ok(Json::NumberLiteral(text.to_owned()));
        }
        return Err(format!("`{text}` is not a finite REAL value"));
    }
    if text.parse::<i128>().is_ok() {
        return Ok(Json::string(text));
    }
    Err(format!(
        "`{text}` is not a boolean, integer, real, or quoted string value"
    ))
}

// ---------------------------------------------------------------------------
// Protocol JSON access and conversion
// ---------------------------------------------------------------------------

fn jget<'a>(value: &'a Json, key: &str) -> Option<&'a Json> {
    value.as_object().and_then(|object| object.get(key))
}

fn got_object<const N: usize>(pairs: [(&str, Json); N]) -> Json {
    Json::object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value)),
    )
}

/// Waits up to `timeout` for a host to exit.
fn wait_exit(host: &mut dyn SessionHost, timeout: Duration) -> Option<i32> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(code) = host.try_exit() {
            return Some(code);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

// ---------------------------------------------------------------------------
// Manifest root grouping
// ---------------------------------------------------------------------------

impl GlobalsLayout {
    /// Groups the manifest body's roots by program instance.
    fn from_manifest(body: &Json) -> Self {
        let mut layout = Self::default();
        if let Some(instances) = jget(body, "instances").and_then(Json::as_array) {
            for instance in instances {
                let Some(path) = jget(instance, "instance_path").and_then(Json::as_str) else {
                    continue;
                };
                layout.instances.push(InstanceGroup {
                    name: path.to_owned(),
                    type_key: jget(instance, "program_type_key")
                        .and_then(Json::as_str)
                        .unwrap_or("PROGRAM")
                        .to_owned(),
                    roots: Vec::new(),
                });
            }
        }
        if let Some(roots) = jget(body, "roots").and_then(Json::as_array) {
            for root in roots {
                let Some(key) = jget(root, "key").and_then(Json::as_str) else {
                    continue;
                };
                let type_key = jget(root, "type_key")
                    .and_then(Json::as_str)
                    .unwrap_or("UNKNOWN")
                    .to_owned();
                let root_kind = jget(root, "root_kind")
                    .and_then(Json::as_str)
                    .unwrap_or("global");
                let field_path = jget(root, "field_path")
                    .and_then(Json::as_str)
                    .unwrap_or("");
                let Some(parsed) = parse_root_key(key) else {
                    continue;
                };
                if root_kind == "task_guard" {
                    continue;
                }
                if let Some(qualifier) = parsed.qualifier
                    && hidden_qualifier(qualifier)
                {
                    // Hidden object storage (HANDLE/OBJREC) holds leaf
                    // pointers, not a user value. Expose the aggregate path
                    // without the hidden marker so the runtime returns its
                    // manifest-assisted field view instead.
                    if root_kind == "nested_field" {
                        let Some((prefix, _)) = key.rsplit_once('#') else {
                            continue;
                        };
                        let base = qualifier
                            .strip_suffix("/HANDLE")
                            .or_else(|| qualifier.strip_suffix("/OBJREC"))
                            .filter(|base| !base.is_empty());
                        let aggregate = match base {
                            Some(base) => format!("{prefix}#{base}"),
                            None => prefix.to_owned(),
                        };
                        let name = if field_path.is_empty() {
                            parsed.path.to_owned()
                        } else {
                            field_path.to_owned()
                        };
                        let entry = GlobalRoot {
                            name,
                            key: aggregate,
                            type_key,
                        };
                        if let Some(base) = base
                            && let Some(group) =
                                layout.instances.iter_mut().find(|group| group.name == base)
                        {
                            push_unique_root(&mut group.roots, entry);
                        } else {
                            push_unique_root(&mut layout.globals, entry);
                        }
                    }
                    continue;
                }
                let qualifier = parsed.qualifier;
                if qualifier.is_some() && root_kind != "program_field" {
                    // Nested aggregate fields are presented by their parent's
                    // manifest-assisted view instead of as separate roots.
                    continue;
                }
                let name = if field_path.is_empty() {
                    parsed.path.to_owned()
                } else {
                    field_path.to_owned()
                };
                let entry = GlobalRoot {
                    name,
                    key: key.to_owned(),
                    type_key,
                };
                if let Some(qualifier) = qualifier {
                    if let Some(group) = layout
                        .instances
                        .iter_mut()
                        .find(|group| group.name == qualifier)
                    {
                        group.roots.push(entry);
                    } else {
                        layout.instances.push(InstanceGroup {
                            name: qualifier.to_owned(),
                            type_key: "PROGRAM".into(),
                            roots: vec![entry],
                        });
                    }
                } else if matches!(root_kind, "global" | "direct") {
                    layout.globals.push(entry);
                }
            }
        }
        for instance in &mut layout.instances {
            instance
                .roots
                .sort_by(|left, right| left.name.cmp(&right.name));
        }
        layout
            .instances
            .sort_by(|left, right| left.name.cmp(&right.name));
        layout
            .globals
            .sort_by(|left, right| left.name.cmp(&right.name));
        layout
    }
}

fn push_unique_root(roots: &mut Vec<GlobalRoot>, entry: GlobalRoot) {
    if roots.iter().any(|existing| existing.key == entry.key) {
        return;
    }
    roots.push(entry);
}

/// Maps each manifest function's `GotId` to a short display name.
fn function_names_from_manifest(body: &Json) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    if let Some(functions) = jget(body, "functions").and_then(Json::as_array) {
        for function in functions {
            let Some(got_id) = jget(function, "got_id").and_then(Json::as_str) else {
                continue;
            };
            let Some(key) = jget(function, "key").and_then(Json::as_str) else {
                continue;
            };
            names.insert(got_id.to_owned(), frame_display_name(key));
        }
    }
    names
}

/// Returns the POU or method name from a manifest function key.
///
/// Keys are `package::path@signature`; FB and program bodies use
/// `path#BODY`, and methods use `owner.method`.
fn frame_display_name(key: &str) -> String {
    let head = key.split_once('@').map_or(key, |(head, _)| head);
    let path = head.split_once("::").map_or(head, |(_, path)| path);
    let path = path.strip_suffix("#BODY").unwrap_or(path);
    path.rsplit_once('.')
        .map_or(path, |(_, name)| name)
        .to_owned()
}

struct ParsedRootKey<'a> {
    path: &'a str,
    qualifier: Option<&'a str>,
}

fn parse_root_key(key: &str) -> Option<ParsedRootKey<'_>> {
    let (_, rest) = key.split_once("::")?;
    let (path, qualifier) = match rest.split_once('#') {
        Some((path, qualifier)) => (path, Some(qualifier)),
        None => (rest, None),
    };
    (!path.is_empty()).then_some(ParsedRootKey { path, qualifier })
}

fn hidden_qualifier(qualifier: &str) -> bool {
    qualifier == "HANDLE"
        || qualifier == "OBJREC"
        || qualifier.ends_with("/HANDLE")
        || qualifier.ends_with("/OBJREC")
}

#[cfg(test)]
mod tests {
    use super::*;
    use stc_got::json;

    fn body(text: &str) -> Json {
        json::parse(text).expect("manifest body")
    }

    #[test]
    fn groups_roots_by_program_instance() {
        let layout = GlobalsLayout::from_manifest(&body(
            r#"{
                "instances": [
                    {"instance_path": "SYNTHETIC/SYNTHETIC/MAIN", "program_type_key": "MAIN"}
                ],
                "roots": [
                    {"key": "app::G_TOTAL", "type_key": "DINT", "root_kind": "global", "field_path": ""},
                    {"key": "app::MAIN.TICKS#SYNTHETIC/SYNTHETIC/MAIN", "type_key": "DINT", "root_kind": "program_field", "field_path": "TICKS"},
                    {"key": "app::MAIN.HIDDEN#SYNTHETIC/SYNTHETIC/MAIN/HANDLE", "type_key": "DINT", "root_kind": "direct", "field_path": "HIDDEN"},
                    {"key": "app::MAIN.GUARD#TASK", "type_key": "BOOL", "root_kind": "task_guard", "field_path": "GUARD"}
                ]
            }"#,
        ));
        assert_eq!(layout.instances.len(), 1);
        assert_eq!(layout.instances[0].name, "SYNTHETIC/SYNTHETIC/MAIN");
        assert_eq!(layout.instances[0].roots.len(), 1);
        assert_eq!(layout.instances[0].roots[0].name, "TICKS");
        assert_eq!(layout.globals.len(), 1);
        assert_eq!(layout.globals[0].name, "G_TOTAL");
    }

    #[test]
    fn exposes_hidden_object_roots_as_aggregate_fields() {
        let layout = GlobalsLayout::from_manifest(&body(
            r#"{
                "instances": [
                    {"instance_path": "SYNTHETIC/SYNTHETIC/MAIN", "program_type_key": "MAIN"}
                ],
                "roots": [
                    {"key": "app::MAIN.COUNTER#SYNTHETIC/SYNTHETIC/MAIN/OBJREC", "type_key": "ARRAY[0..1]OF(PTR_TO(T:1))", "root_kind": "nested_field", "field_path": "COUNTER"},
                    {"key": "app::MAIN.COUNTER.Q#SYNTHETIC/SYNTHETIC/MAIN", "type_key": "DINT", "root_kind": "nested_field", "field_path": "COUNTER.Q"},
                    {"key": "app::MAIN.SUM#SYNTHETIC/SYNTHETIC/MAIN", "type_key": "DINT", "root_kind": "program_field", "field_path": "SUM"},
                    {"key": "app::G_FB#OBJREC", "type_key": "ARRAY[0..1]OF(PTR_TO(T:2))", "root_kind": "nested_field", "field_path": "G_FB"}
                ]
            }"#,
        ));
        let instance = &layout.instances[0];
        let names: Vec<&str> = instance
            .roots
            .iter()
            .map(|root| root.name.as_str())
            .collect();
        assert_eq!(names, ["COUNTER", "SUM"]);
        assert_eq!(
            instance.roots[0].key,
            "app::MAIN.COUNTER#SYNTHETIC/SYNTHETIC/MAIN"
        );
        assert_eq!(layout.globals.len(), 1);
        assert_eq!(layout.globals[0].name, "G_FB");
        assert_eq!(layout.globals[0].key, "app::G_FB");
    }

    #[test]
    fn frame_names_use_manifest_keys() {
        let body = body(
            r#"{
                "functions": [
                    {"got_id": "0x01", "key": "app::MAIN#BODY@()->VOID"},
                    {"got_id": "0x02", "key": "app::ADDSTEP@(IN:DINT)->DINT"},
                    {"got_id": "0x03", "key": "app::DRIVE.START@()->BOOL"}
                ]
            }"#,
        );
        let names = function_names_from_manifest(&body);
        assert_eq!(names.get("0x01").map(String::as_str), Some("MAIN"));
        assert_eq!(names.get("0x02").map(String::as_str), Some("ADDSTEP"));
        assert_eq!(names.get("0x03").map(String::as_str), Some("START"));
    }

    #[test]
    fn parses_write_text_for_scalar_types() {
        assert_eq!(
            parse_write_value("BOOL", "true").expect("bool"),
            Json::Bool(true)
        );
        assert_eq!(
            parse_write_value("DINT", "42").expect("dint"),
            Json::string("42")
        );
        assert_eq!(
            parse_write_value("REAL", "1.5").expect("real"),
            Json::NumberLiteral("1.5".into())
        );
        assert_eq!(
            parse_write_value("STRING[10]", "\"hi\"").expect("string"),
            Json::object([
                ("length".into(), Json::unsigned(2)),
                (
                    "units".into(),
                    Json::Array(vec![Json::unsigned(104), Json::unsigned(105)])
                )
            ])
        );
        assert!(parse_write_value("DINT", "hello").is_err());
        assert!(parse_write_value("REAL", "not-a-real").is_err());
    }
}
