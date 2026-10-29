//! `stc debug` and `stc run --debug-server`: local authenticated debug
//! sessions (CLI-and-Manifest §1.7, Debug and Monitoring §13–§18).
//!
//! The driver owns the session lifecycle. Launch mode builds and starts the
//! host, waits for the endpoint file, starts one session, runs either the
//! interactive command loop or a JSON-lines batch, then stops the session and
//! shuts the host down. Attach mode connects to an existing endpoint file,
//! starts a session, and detaches without stopping the host. The endpoint
//! token never leaves the endpoint file and is never printed.
//!
//! A swap, timeout, or protocol error is reported with its stable
//! `STC-E-DEBUG-*` code and exits 2; the CLI never reconnects or replays state.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use stc_diag::Severity;
use stc_got::Json;
use stc_manifest::Manifest;
use stc_runtime::control::ControlClient;
use stc_runtime::debug_server::DebugClient;

use crate::build::RunRequest;
use crate::cli::{DebugOptions, parse_break_spec};
use crate::report::{Log, Reporter};
use crate::run as run_util;

/// Object-field access for wire values (`stc_got::Json` exposes maps, not a
/// field accessor).
trait JsonFields {
    fn get(&self, key: &str) -> Option<&Json>;
}

impl JsonFields for Json {
    fn get(&self, key: &str) -> Option<&Json> {
        self.as_object().and_then(|object| object.get(key))
    }
}

/// The non-safety notice printed at every session start and in `--help`. The
/// adapter emits the same text as an `output` event (WL-1.1 I-14).
pub(crate) const NOTICE: &str = stc_dap::NON_SAFETY_NOTICE;
const ENDPOINT_DEADLINE: Duration = Duration::from_secs(5);
pub(crate) const CONTROL_DEADLINE: Duration = Duration::from_secs(10);
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(5);
const DEFAULT_WATCH_INTERVAL_MS: u64 = 1000;

/// Builds the launcher's debug-server arguments and the resolved endpoint
/// path for `stc run --debug-server` (and `stc run --watch --debug-server`).
pub(crate) fn server_arguments(
    debug: &DebugOptions,
    manifest: &Manifest,
    out_dir: &Path,
) -> (Vec<String>, PathBuf) {
    let endpoint = resolve_endpoint(debug.server_endpoint.as_deref(), out_dir);
    let arguments = server_arguments_for(debug, manifest, &endpoint);
    (arguments, endpoint)
}

fn resolve_endpoint(explicit: Option<&str>, out_dir: &Path) -> PathBuf {
    match explicit {
        Some(path) => absolute_path(Path::new(path)),
        None => absolute_path(&out_dir.join(format!("stc-debug-{}.endpoint", random_hex()))),
    }
}

pub(crate) fn server_arguments_for(
    debug: &DebugOptions,
    manifest: &Manifest,
    endpoint: &Path,
) -> Vec<String> {
    let mut arguments = vec![
        "--debug-endpoint".to_owned(),
        endpoint.to_string_lossy().into_owned(),
        "--debug-protocol".to_owned(),
        debug
            .protocol
            .unwrap_or(manifest.debug.protocol)
            .as_str()
            .to_owned(),
    ];
    if debug.allow_write || manifest.debug.allow_write {
        arguments.push("--debug-allow-write".to_owned());
    }
    if debug.allow_force || manifest.debug.allow_force {
        arguments.push("--debug-allow-force".to_owned());
    }
    let timeout = debug
        .session_timeout_ns
        .unwrap_or_else(|| manifest.debug.session_timeout.as_nanos());
    arguments.push("--debug-session-timeout".to_owned());
    arguments.push(format!("{timeout}ns"));
    arguments
}

fn launch_scopes(debug: &DebugOptions, manifest: &Manifest) -> Vec<&'static str> {
    let mut scopes = vec!["observe", "control"];
    if debug.allow_write || manifest.debug.allow_write {
        scopes.push("write");
    }
    if debug.allow_force || manifest.debug.allow_force {
        scopes.push("force");
    }
    scopes
}

/// Attaches to an existing endpoint file and runs one session.
pub(crate) fn attach(options: &DebugOptions, reporter: &mut Reporter, log: Log) -> u8 {
    stc_runtime::signal::install_stop_handler();
    let Some(path) = options.attach.as_deref() else {
        reporter.plain(
            "STC-E-PROJECT-010",
            Severity::Error,
            "`stc debug --attach` requires an endpoint file",
        );
        return 2;
    };
    let path = absolute_path(Path::new(path));
    let mut client = match DebugClient::connect_file(&path) {
        Ok(client) => client,
        Err(error) => {
            reporter.plain(
                "STC-E-DEBUG-020",
                Severity::Error,
                &format!(
                    "cannot connect to the debug endpoint `{}`: {error}",
                    path.display()
                ),
            );
            return 2;
        }
    };
    log.info(format!("attached to {}", path.display()));
    eprintln!("stc: {NOTICE}");
    let json = reporter.is_json();
    let mut state = StopState::default();
    if let Err(code) = start_session(
        &mut client,
        &["observe", "control"],
        json,
        reporter,
        &mut state,
    ) {
        let _ = stop_session(&mut client);
        return code;
    }
    if let Err(code) = apply_initial(&mut client, options, json, reporter, &mut state) {
        let _ = stop_session(&mut client);
        return code;
    }
    let outcome = if let Some(batch) = &options.batch {
        run_batch(&mut client, batch, json, reporter, &mut state)
    } else {
        interactive(&mut client, json, reporter, &mut state)
    };
    finish_attach(&mut client, outcome, reporter)
}

/// Launches the host, owns one debug session, and shuts the host down.
#[allow(clippy::too_many_arguments)]
pub(crate) fn launch(
    exe: &Path,
    out_dir: &Path,
    request: &RunRequest,
    debug: &DebugOptions,
    manifest: &Manifest,
    root_dir: &Path,
    reporter: &mut Reporter,
    log: Log,
) -> u8 {
    stc_runtime::signal::install_stop_handler();
    let endpoint = resolve_endpoint(debug.endpoint.as_deref(), out_dir);
    let control = control_endpoint();
    let nonce = random_hex();
    let json = reporter.is_json();
    let mut command = Command::new(exe);
    command.args(run_util::run_arguments(
        request,
        manifest,
        root_dir,
        log.verbose(),
    ));
    command.args(server_arguments_for(debug, manifest, &endpoint));
    command
        .arg("--control")
        .arg(&control)
        .arg("--control-nonce")
        .arg(&nonce);
    command.stdin(Stdio::null());
    if json {
        // Keep stdout a clean JSON-lines stream.
        command.stdout(Stdio::null());
    } else {
        command.stdout(Stdio::inherit());
    }
    command.stderr(Stdio::inherit());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            reporter.plain(
                "STC-E-BUILD-002",
                Severity::Error,
                &format!("cannot start the launcher: {error}"),
            );
            return 1;
        }
    };
    let deadline = Instant::now() + ENDPOINT_DEADLINE;
    loop {
        if endpoint.is_file() {
            break;
        }
        if let Ok(Some(status)) = child.try_wait() {
            return child_code(status.code());
        }
        if Instant::now() >= deadline {
            reporter.plain(
                "STC-E-DEBUG-020",
                Severity::Error,
                &format!(
                    "the debug endpoint `{}` was not created within five seconds",
                    endpoint.display()
                ),
            );
            let _ = child.kill();
            let _ = child.wait();
            return 2;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut client = match DebugClient::connect_file(&endpoint) {
        Ok(client) => client,
        Err(error) => {
            reporter.plain(
                "STC-E-DEBUG-020",
                Severity::Error,
                &format!(
                    "cannot connect to the debug endpoint `{}`: {error}",
                    endpoint.display()
                ),
            );
            let _ = shutdown_host(&control, &nonce, &mut child);
            return 2;
        }
    };
    log.status(format!("debug endpoint {}", endpoint.display()));
    eprintln!("stc: {NOTICE}");
    let mut state = StopState::default();
    if let Err(code) = start_session(
        &mut client,
        &launch_scopes(debug, manifest),
        json,
        reporter,
        &mut state,
    ) {
        let _ = stop_session(&mut client);
        let _ = shutdown_host(&control, &nonce, &mut child);
        return code;
    }
    if let Err(code) = apply_initial(&mut client, debug, json, reporter, &mut state) {
        let _ = stop_session(&mut client);
        let _ = shutdown_host(&control, &nonce, &mut child);
        return code;
    }
    let outcome = if let Some(batch) = &debug.batch {
        run_batch(&mut client, batch, json, reporter, &mut state)
    } else {
        interactive(&mut client, json, reporter, &mut state)
    };
    finish_launch(client, outcome, reporter, &control, &nonce, &mut child)
}

/// Sends `session.start` and reports the result, capabilities, and warnings.
fn start_session(
    client: &mut DebugClient,
    scopes: &[&str],
    json: bool,
    reporter: &mut Reporter,
    state: &mut StopState,
) -> Result<Json, u8> {
    let params = Json::object([
        (
            "client".into(),
            Json::object([
                ("name".into(), Json::string("stc")),
                ("version".into(), Json::string(env!("CARGO_PKG_VERSION"))),
            ]),
        ),
        (
            "scopes".into(),
            Json::Array(
                scopes
                    .iter()
                    .map(|scope| Json::string(*scope))
                    .collect::<Vec<_>>(),
            ),
        ),
    ]);
    match request(client, "session.start", params, json, reporter, state, None) {
        Ok(result) => Ok(result),
        Err(failure) => {
            report_failure(&failure, reporter);
            Err(exit_for_failure(&failure))
        }
    }
}

/// Applies repeatable `--break`/`--watch` options after `session.start`.
fn apply_initial(
    client: &mut DebugClient,
    options: &DebugOptions,
    json: bool,
    reporter: &mut Reporter,
    state: &mut StopState,
) -> Result<(), u8> {
    for spec in &options.breaks {
        let (file, line, column) = match parse_break_spec(spec) {
            Ok(parts) => parts,
            Err(message) => {
                reporter.plain("STC-E-PROJECT-010", Severity::Error, &message);
                return Err(2);
            }
        };
        let params = Json::object([(
            "target".into(),
            Json::object([
                ("kind".into(), Json::string("source")),
                ("file".into(), Json::string(file)),
                ("line".into(), Json::unsigned(u64::from(line))),
                ("column".into(), Json::unsigned(u64::from(column))),
            ]),
        )]);
        if let Err(failure) = request(
            client,
            "breakpoint.set",
            params,
            json,
            reporter,
            state,
            None,
        ) {
            report_failure(&failure, reporter);
            return Err(exit_for_failure(&failure));
        }
    }
    for spec in &options.watches {
        let target = match parse_target_text(spec) {
            Ok(target) => target,
            Err(message) => {
                reporter.plain("STC-E-DEBUG-008", Severity::Error, &message);
                return Err(2);
            }
        };
        let params = Json::object([
            ("targets".into(), Json::Array(vec![target])),
            (
                "interval_ms".into(),
                Json::unsigned(DEFAULT_WATCH_INTERVAL_MS),
            ),
        ]);
        if let Err(failure) = request(client, "watch.add", params, json, reporter, state, None) {
            report_failure(&failure, reporter);
            return Err(exit_for_failure(&failure));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Interactive command loop
// ---------------------------------------------------------------------------

enum Outcome {
    /// `quit` or stdin EOF.
    Clean,
    /// Ctrl-C.
    Interrupted,
    /// A `session-ended` event.
    Ended(EndInfo),
    /// The connection closed or failed without a session-ended event.
    Transport(String),
}

#[derive(Clone, Debug)]
struct EndInfo {
    reason: String,
    code: Option<String>,
}

#[derive(Default)]
struct StopState {
    stop_id: Option<u64>,
}

enum Failure {
    Server { code: String, message: String },
    Ended(EndInfo),
    Transport(String),
}

enum CommandOutcome {
    Continue,
    Quit,
}

fn interactive(
    client: &mut DebugClient,
    json: bool,
    reporter: &mut Reporter,
    state: &mut StopState,
) -> Outcome {
    let reader = spawn_stdin_reader();
    loop {
        match drain_events(client, json, reporter, state) {
            Drain::Idle => {}
            Drain::Ended(end) => return Outcome::Ended(end),
            Drain::Closed(message) => return Outcome::Transport(message),
        }
        if stc_runtime::signal::stop_requested() {
            return Outcome::Interrupted;
        }
        match reader.recv_timeout(Duration::from_millis(50)) {
            Ok(Some(line)) => match dispatch(client, &line, json, reporter, state) {
                Ok(CommandOutcome::Continue) => {}
                Ok(CommandOutcome::Quit) => return Outcome::Clean,
                Err(Failure::Ended(end)) => return Outcome::Ended(end),
                Err(Failure::Transport(message)) => return Outcome::Transport(message),
                Err(Failure::Server { code, message }) => {
                    reporter.plain(&code, Severity::Error, &message);
                }
            },
            Ok(None) | Err(RecvTimeoutError::Disconnected) => return Outcome::Clean,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

enum Drain {
    Idle,
    Ended(EndInfo),
    Closed(String),
}

fn drain_events(
    client: &mut DebugClient,
    json: bool,
    reporter: &mut Reporter,
    state: &mut StopState,
) -> Drain {
    loop {
        match client.poll_message() {
            Ok(Some(message)) => {
                if json {
                    println!("{}", message.render_compact());
                }
                if let Some(end) = handle_event(&message, json, reporter, state) {
                    return Drain::Ended(end);
                }
            }
            Ok(None) => return Drain::Idle,
            Err(error) => return Drain::Closed(error.to_string()),
        }
    }
}

fn spawn_stdin_reader() -> Receiver<Option<String>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(line) => {
                    if sender.send(Some(line)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = sender.send(None);
    });
    receiver
}

fn dispatch(
    client: &mut DebugClient,
    line: &str,
    json: bool,
    reporter: &mut Reporter,
    state: &mut StopState,
) -> Result<CommandOutcome, Failure> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(CommandOutcome::Continue);
    }
    let (command, rest) = match line.split_once(char::is_whitespace) {
        Some((command, rest)) => (command.to_ascii_lowercase(), rest.trim()),
        None => (line.to_ascii_lowercase(), ""),
    };
    match command.as_str() {
        "continue" => {
            let params = Json::object([(
                "stop_id".into(),
                Json::optional(state.stop_id.map(Json::unsigned)),
            )]);
            request(client, "exec.continue", params, json, reporter, state, None)?;
        }
        "pause" => {
            request(
                client,
                "exec.pause",
                Json::object(Vec::new()),
                json,
                reporter,
                state,
                None,
            )?;
        }
        "step" | "next" | "finish" => {
            let Some(stop_id) = state.stop_id else {
                reporter.plain(
                    "STC-E-DEBUG-015",
                    Severity::Error,
                    "there is no stopped frame; `pause` or wait for a breakpoint first",
                );
                return Ok(CommandOutcome::Continue);
            };
            let mode = match command.as_str() {
                "step" => "into",
                "next" => "over",
                _ => "out",
            };
            let params = Json::object([
                ("mode".into(), Json::string(mode)),
                ("stop_id".into(), Json::unsigned(stop_id)),
            ]);
            request(client, "exec.step", params, json, reporter, state, None)?;
        }
        "break" => {
            if rest.is_empty() {
                reporter.plain(
                    "STC-E-PROJECT-010",
                    Severity::Error,
                    "usage: break <file>:<line>[:<column>]",
                );
                return Ok(CommandOutcome::Continue);
            }
            let (file, line, column) = match parse_break_spec(rest) {
                Ok(parts) => parts,
                Err(message) => {
                    reporter.plain("STC-E-PROJECT-010", Severity::Error, &message);
                    return Ok(CommandOutcome::Continue);
                }
            };
            let params = Json::object([(
                "target".into(),
                Json::object([
                    ("kind".into(), Json::string("source")),
                    ("file".into(), Json::string(file)),
                    ("line".into(), Json::unsigned(u64::from(line))),
                    ("column".into(), Json::unsigned(u64::from(column))),
                ]),
            )]);
            request(
                client,
                "breakpoint.set",
                params,
                json,
                reporter,
                state,
                None,
            )?;
        }
        "clear" => {
            if rest.is_empty() || rest.eq_ignore_ascii_case("all") {
                let listed = request(
                    client,
                    "breakpoint.list",
                    Json::object(Vec::new()),
                    json,
                    reporter,
                    state,
                    None,
                )?;
                for id in breakpoint_ids(&listed) {
                    let params = Json::object([("breakpoint_id".into(), Json::unsigned(id))]);
                    let _ = request(
                        client,
                        "breakpoint.clear",
                        params,
                        json,
                        reporter,
                        state,
                        None,
                    );
                }
            } else {
                let Ok(id) = rest.parse::<u64>() else {
                    reporter.plain(
                        "STC-E-PROJECT-010",
                        Severity::Error,
                        "usage: clear [<breakpoint-id>|all]",
                    );
                    return Ok(CommandOutcome::Continue);
                };
                let params = Json::object([("breakpoint_id".into(), Json::unsigned(id))]);
                request(
                    client,
                    "breakpoint.clear",
                    params,
                    json,
                    reporter,
                    state,
                    None,
                )?;
            }
        }
        "print" => {
            if rest.is_empty() {
                reporter.plain(
                    "STC-E-PROJECT-010",
                    Severity::Error,
                    "usage: print <target>",
                );
                return Ok(CommandOutcome::Continue);
            }
            let target = parse_target_text(rest).map_err(|message| Failure::Server {
                code: "STC-E-DEBUG-008".into(),
                message,
            })?;
            let params = Json::object([("target".into(), target)]);
            request(
                client,
                "var.read",
                params,
                json,
                reporter,
                state,
                Some(rest),
            )?;
        }
        "set" | "force" => {
            let (target, value) = match split_target_value(rest) {
                Some(parts) => parts,
                None => {
                    reporter.plain(
                        "STC-E-PROJECT-010",
                        Severity::Error,
                        &format!("usage: {command} <target> <value>"),
                    );
                    return Ok(CommandOutcome::Continue);
                }
            };
            let wire = match parse_wire_value(value) {
                Ok(value) => value,
                Err(message) => {
                    reporter.plain("STC-E-PROJECT-010", Severity::Error, &message);
                    return Ok(CommandOutcome::Continue);
                }
            };
            let target_json = parse_target_text(target).map_err(|message| Failure::Server {
                code: "STC-E-DEBUG-008".into(),
                message,
            })?;
            let read = request(
                client,
                "var.read",
                Json::object([("target".into(), target_json.clone())]),
                json,
                reporter,
                state,
                Some(target),
            )?;
            let type_key = typed_value(&read)
                .and_then(|value| value.get("type"))
                .and_then(Json::as_str)
                .map(str::to_owned);
            let Some(type_key) = type_key else {
                reporter.plain(
                    "STC-E-DEBUG-008",
                    Severity::Error,
                    "the target value does not carry a type; use batch mode for this write",
                );
                return Ok(CommandOutcome::Continue);
            };
            let op = if command == "set" {
                "var.write"
            } else {
                "var.force"
            };
            let params = Json::object([
                ("target".into(), target_json),
                (
                    "value".into(),
                    Json::object([
                        ("type".into(), Json::string(type_key)),
                        ("value".into(), wire),
                    ]),
                ),
            ]);
            request(client, op, params, json, reporter, state, None)?;
        }
        "release" => {
            if rest.is_empty() {
                reporter.plain(
                    "STC-E-PROJECT-010",
                    Severity::Error,
                    "usage: release <target>",
                );
                return Ok(CommandOutcome::Continue);
            }
            let target = parse_target_text(rest).map_err(|message| Failure::Server {
                code: "STC-E-DEBUG-008".into(),
                message,
            })?;
            let params = Json::object([("target".into(), target)]);
            request(client, "var.release", params, json, reporter, state, None)?;
        }
        "release-all" => {
            request(
                client,
                "var.release-all",
                Json::object(Vec::new()),
                json,
                reporter,
                state,
                None,
            )?;
        }
        "watch" => {
            let (target, interval) = match split_target_value(rest) {
                Some((target, interval)) => {
                    let interval = if interval.is_empty() {
                        DEFAULT_WATCH_INTERVAL_MS
                    } else {
                        match interval.parse::<u64>() {
                            Ok(interval) => interval,
                            Err(_) => {
                                reporter.plain(
                                    "STC-E-PROJECT-010",
                                    Severity::Error,
                                    "usage: watch <target> [<interval-ms>]",
                                );
                                return Ok(CommandOutcome::Continue);
                            }
                        }
                    };
                    (target, interval)
                }
                None => {
                    reporter.plain(
                        "STC-E-PROJECT-010",
                        Severity::Error,
                        "usage: watch <target> [<interval-ms>]",
                    );
                    return Ok(CommandOutcome::Continue);
                }
            };
            let target = parse_target_text(target).map_err(|message| Failure::Server {
                code: "STC-E-DEBUG-008".into(),
                message,
            })?;
            let params = Json::object([
                ("targets".into(), Json::Array(vec![target])),
                ("interval_ms".into(), Json::unsigned(interval)),
            ]);
            request(client, "watch.add", params, json, reporter, state, None)?;
        }
        "unwatch" => {
            let Ok(watch_id) = rest.parse::<u64>() else {
                reporter.plain(
                    "STC-E-PROJECT-010",
                    Severity::Error,
                    "usage: unwatch <watch-id>",
                );
                return Ok(CommandOutcome::Continue);
            };
            let params = Json::object([("watch_id".into(), Json::unsigned(watch_id))]);
            request(client, "watch.remove", params, json, reporter, state, None)?;
        }
        "stack" => {
            let stop_id = match stack_stop_id(rest, state) {
                Ok(stop_id) => stop_id,
                Err(Failure::Server { code, message }) => {
                    reporter.plain(&code, Severity::Error, &message);
                    return Ok(CommandOutcome::Continue);
                }
                Err(failure) => return Err(failure),
            };
            let params = Json::object([("stop_id".into(), Json::unsigned(stop_id))]);
            request(client, "stack.list", params, json, reporter, state, None)?;
        }
        "frame" => {
            let mut parts = rest.split_whitespace();
            let (Some(frame), Some(name)) = (parts.next(), parts.next()) else {
                reporter.plain(
                    "STC-E-PROJECT-010",
                    Severity::Error,
                    "usage: frame <frame> <local> [<stop-id>]",
                );
                return Ok(CommandOutcome::Continue);
            };
            let frame = match frame.parse::<usize>() {
                Ok(frame) => frame,
                Err(_) => {
                    reporter.plain(
                        "STC-E-PROJECT-010",
                        Severity::Error,
                        "the frame index must be a non-negative integer",
                    );
                    return Ok(CommandOutcome::Continue);
                }
            };
            let stop_id = match stack_stop_id(parts.next().unwrap_or(""), state) {
                Ok(stop_id) => stop_id,
                Err(Failure::Server { code, message }) => {
                    reporter.plain(&code, Severity::Error, &message);
                    return Ok(CommandOutcome::Continue);
                }
                Err(failure) => return Err(failure),
            };
            let params = Json::object([
                (
                    "target".into(),
                    Json::object([
                        ("kind".into(), Json::string("local")),
                        ("frame".into(), Json::unsigned(frame as u64)),
                        ("name".into(), Json::string(name)),
                    ]),
                ),
                ("stop_id".into(), Json::unsigned(stop_id)),
            ]);
            request(
                client,
                "frame.read",
                params,
                json,
                reporter,
                state,
                Some(&format!("frame {frame} {name}")),
            )?;
        }
        "quit" | "exit" => return Ok(CommandOutcome::Quit),
        "help" => print_command_help(),
        other => {
            reporter.plain(
                "STC-E-PROJECT-010",
                Severity::Error,
                &format!("unknown debug command `{other}`; type `help` for the command list"),
            );
        }
    }
    Ok(CommandOutcome::Continue)
}

fn print_command_help() {
    eprintln!(
        "debug commands: continue, pause, step, next, finish, break <file>:<line>[:<column>], \
         clear [<id>|all], print <target>, set <target> <value>, force <target> <value>, \
         release <target>, release-all, watch <target> [<interval-ms>], unwatch <watch-id>, \
         stack [<stop-id>], frame <frame> <local> [<stop-id>], quit"
    );
}

fn stack_stop_id(text: &str, state: &StopState) -> Result<u64, Failure> {
    if text.trim().is_empty() {
        return state.stop_id.ok_or_else(|| Failure::Server {
            code: "STC-E-DEBUG-015".into(),
            message: "there is no stopped frame; `pause` or wait for a breakpoint first".into(),
        });
    }
    text.trim().parse::<u64>().map_err(|_| Failure::Server {
        code: "STC-E-PROJECT-010".into(),
        message: "the stop id must be a positive integer".into(),
    })
}

fn split_target_value(rest: &str) -> Option<(&str, &str)> {
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    match rest.split_once(char::is_whitespace) {
        Some((target, value)) => Some((target, value.trim())),
        None => Some((rest, "")),
    }
}

fn parse_wire_value(text: &str) -> Result<Json, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("a value is required".into());
    }
    if text.eq_ignore_ascii_case("true") {
        return Ok(Json::Bool(true));
    }
    if text.eq_ignore_ascii_case("false") {
        return Ok(Json::Bool(false));
    }
    if (text.starts_with('\'') && text.ends_with('\''))
        || (text.starts_with('"') && text.ends_with('"'))
    {
        if text.len() < 2 {
            return Err(format!("`{text}` is not a quoted string"));
        }
        return Ok(Json::string(&text[1..text.len() - 1]));
    }
    if let Ok(value) = text.parse::<i128>() {
        return Ok(Json::Number(value));
    }
    if let Ok(value) = text.parse::<f64>()
        && value.is_finite()
    {
        return Ok(Json::NumberLiteral(text.to_owned()));
    }
    Err(format!(
        "`{text}` is not a boolean, integer, real, or quoted string value"
    ))
}

/// Parses a normalized stable key path or `0x` + 32 hex `GotId`.
fn parse_target_text(text: &str) -> Result<Json, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("a target is required".into());
    }
    if text.starts_with("0x") || text.starts_with("0X") {
        let digits = &text[2..];
        if digits.len() == 32 && digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Ok(Json::object([
                ("kind".into(), Json::string("got_id")),
                (
                    "got_id".into(),
                    Json::string(format!("0x{}", digits.to_ascii_lowercase())),
                ),
            ]));
        }
        return Err(format!(
            "a GotId target is `0x` plus 32 hexadecimal characters, found `{text}`"
        ));
    }
    Ok(Json::object([
        ("kind".into(), Json::string("path")),
        ("key".into(), Json::string(text)),
    ]))
}

// ---------------------------------------------------------------------------
// Batch command loop
// ---------------------------------------------------------------------------

fn run_batch(
    client: &mut DebugClient,
    source: &str,
    json: bool,
    reporter: &mut Reporter,
    state: &mut StopState,
) -> Outcome {
    let reader: Box<dyn BufRead> = if source == "-" {
        Box::new(BufReader::new(std::io::stdin()))
    } else {
        match std::fs::File::open(source) {
            Ok(file) => Box::new(BufReader::new(file)),
            Err(error) => {
                reporter.plain(
                    "STC-E-PROJECT-010",
                    Severity::Error,
                    &format!("cannot read the batch file `{source}`: {error}"),
                );
                return Outcome::Transport(format!("cannot read the batch file `{source}`"));
            }
        }
    };
    for line in reader.lines() {
        if stc_runtime::signal::stop_requested() {
            return Outcome::Interrupted;
        }
        match drain_events(client, json, reporter, state) {
            Drain::Idle => {}
            Drain::Ended(end) => return Outcome::Ended(end),
            Drain::Closed(message) => return Outcome::Transport(message),
        }
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                reporter.plain(
                    "STC-E-DEBUG-003",
                    Severity::Error,
                    &format!("cannot read the batch request: {error}"),
                );
                return Outcome::Transport(error.to_string());
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let value = match stc_got::json::parse_protocol(&line) {
            Ok(value) => value,
            Err(error) => {
                reporter.plain("STC-E-DEBUG-003", Severity::Error, &error.message);
                return Outcome::Transport(error.message);
            }
        };
        let Some(object) = value.as_object() else {
            reporter.plain(
                "STC-E-DEBUG-003",
                Severity::Error,
                "a batch request must be a JSON object",
            );
            return Outcome::Transport("a batch request must be a JSON object".into());
        };
        if object
            .keys()
            .any(|key| matches!(key.as_str(), "kind" | "protocol" | "id" | "token"))
        {
            reporter.plain(
                "STC-E-DEBUG-003",
                Severity::Error,
                "a batch request must contain only `op` and `params`; the CLI adds the envelope",
            );
            return Outcome::Transport("a batch request repeated an envelope field".into());
        }
        let Some(op) = object.get("op").and_then(Json::as_str) else {
            reporter.plain(
                "STC-E-DEBUG-003",
                Severity::Error,
                "a batch request requires a string `op`",
            );
            return Outcome::Transport("a batch request requires a string `op`".into());
        };
        if object.keys().any(|key| key != "op" && key != "params") {
            reporter.plain(
                "STC-E-DEBUG-003",
                Severity::Error,
                "a batch request must contain only `op` and `params`",
            );
            return Outcome::Transport("a batch request has an unknown key".into());
        }
        let params = match object.get("params") {
            None => Json::object(Vec::new()),
            Some(params) if params.as_object().is_some() => params.clone(),
            Some(_) => {
                reporter.plain(
                    "STC-E-DEBUG-003",
                    Severity::Error,
                    "`params` must be an object",
                );
                return Outcome::Transport("`params` must be an object".into());
            }
        };
        let messages = match client.call_messages(op, params) {
            Ok(messages) => messages,
            Err(error) => return Outcome::Transport(error.to_string()),
        };
        let mut answered = false;
        for message in messages {
            println!("{}", message.render_compact());
            if let Some(end) = handle_event(&message, true, reporter, state) {
                return Outcome::Ended(end);
            }
            if message_kind(&message) == Some("response") {
                answered = true;
            }
        }
        if !answered {
            return Outcome::Transport("the debug server closed the connection".into());
        }
    }
    Outcome::Clean
}

// ---------------------------------------------------------------------------
// Messages, events, and diagnostics
// ---------------------------------------------------------------------------

fn message_kind(message: &Json) -> Option<&str> {
    message
        .as_object()
        .and_then(|object| object.get("kind").and_then(Json::as_str))
}

fn message_event(message: &Json) -> Option<(&str, &BTreeMap<String, Json>)> {
    let object = message.as_object()?;
    let event = object.get("event").and_then(Json::as_str)?;
    let data = object.get("data").and_then(Json::as_object)?;
    Some((event, data))
}

/// Handles one event, printing it in human mode and updating the stop state.
fn handle_event(
    message: &Json,
    json: bool,
    reporter: &mut Reporter,
    state: &mut StopState,
) -> Option<EndInfo> {
    let (event, data) = message_event(message)?;
    match event {
        "stopped" => {
            if let Some(stop_id) = data.get("stop_id").and_then(Json::as_number)
                && let Ok(stop_id) = u64::try_from(stop_id)
            {
                state.stop_id = Some(stop_id);
            }
            if !json {
                let source = data.get("source").and_then(Json::as_object);
                let file = source
                    .and_then(|source| source.get("file"))
                    .and_then(Json::as_str)
                    .unwrap_or("<unknown>");
                let line = source
                    .and_then(|source| source.get("line"))
                    .and_then(Json::as_number)
                    .unwrap_or(0);
                let column = source
                    .and_then(|source| source.get("column"))
                    .and_then(Json::as_number)
                    .unwrap_or(0);
                let reason = data
                    .get("reason")
                    .and_then(Json::as_str)
                    .unwrap_or("unknown");
                let stop_id = data.get("stop_id").and_then(Json::as_number).unwrap_or(0);
                eprintln!(
                    "stc: stopped at {file}:{line}:{column} (reason {reason}, stop {stop_id})"
                );
            }
        }
        "resumed" => {
            state.stop_id = None;
            if !json {
                let reason = data
                    .get("reason")
                    .and_then(Json::as_str)
                    .unwrap_or("unknown");
                eprintln!("stc: resumed (reason {reason})");
            }
        }
        "watch" => {
            if !json {
                let watch_id = data.get("watch_id").and_then(Json::as_number).unwrap_or(0);
                let cycle = data.get("cycle").and_then(Json::as_number).unwrap_or(0);
                if let Some(values) = data.get("values").and_then(Json::as_array) {
                    for value in values {
                        let label = target_label(value.get("target"));
                        if value.get("available").and_then(Json::as_bool) == Some(false) {
                            let code = value
                                .get("code")
                                .and_then(Json::as_str)
                                .unwrap_or("STC-W-DEBUG-002");
                            eprintln!(
                                "stc: watch {watch_id} cycle {cycle} {label} unavailable [{code}]"
                            );
                        } else {
                            let text = value
                                .get("value")
                                .map_or_else(|| "null".into(), Json::render_compact);
                            eprintln!("stc: watch {watch_id} cycle {cycle} {label} = {text}");
                        }
                    }
                }
            }
        }
        "queue-result" => {
            if !json {
                let queue_id = data.get("queue_id").and_then(Json::as_number).unwrap_or(0);
                let operation = data
                    .get("operation")
                    .and_then(Json::as_str)
                    .unwrap_or("unknown");
                let outcome = data
                    .get("outcome")
                    .and_then(Json::as_str)
                    .unwrap_or("unknown");
                let code = data.get("code").and_then(Json::as_str);
                match code {
                    Some(code) => {
                        eprintln!("stc: queue {queue_id} {operation} {outcome} [{code}]");
                    }
                    None => eprintln!("stc: queue {queue_id} {operation} {outcome}"),
                }
            }
        }
        "session-ended" => {
            let reason = data
                .get("reason")
                .and_then(Json::as_str)
                .unwrap_or("unknown")
                .to_owned();
            let code = data.get("code").and_then(Json::as_str).map(str::to_owned);
            if !json {
                match &code {
                    Some(code) => {
                        eprintln!("stc: debug session ended (reason {reason}, code {code})");
                    }
                    None => eprintln!("stc: debug session ended (reason {reason})"),
                }
            }
            let _ = reporter;
            return Some(EndInfo { reason, code });
        }
        _ => {
            if !json {
                eprintln!("stc: event {event}");
            }
        }
    }
    None
}

fn target_label(target: Option<&Json>) -> String {
    let Some(target) = target.and_then(Json::as_object) else {
        return "<target>".into();
    };
    match target.get("kind").and_then(Json::as_str) {
        Some("path") => target
            .get("key")
            .and_then(Json::as_str)
            .unwrap_or("<target>")
            .to_owned(),
        Some("got_id") => target
            .get("got_id")
            .and_then(Json::as_str)
            .unwrap_or("<target>")
            .to_owned(),
        _ => "<target>".into(),
    }
}

/// Sends one request, prints every message in arrival order, and returns the
/// successful result.
fn request(
    client: &mut DebugClient,
    op: &str,
    params: Json,
    json: bool,
    reporter: &mut Reporter,
    state: &mut StopState,
    context: Option<&str>,
) -> Result<Json, Failure> {
    let messages = client
        .call_messages(op, params)
        .map_err(|error| Failure::Transport(error.to_string()))?;
    let mut result = None;
    for message in messages {
        if json {
            println!("{}", message.render_compact());
        }
        if let Some(end) = handle_event(&message, json, reporter, state) {
            return Err(Failure::Ended(end));
        }
        if message_kind(&message) != Some("response") {
            continue;
        }
        let object = message.as_object().expect("response object");
        let ok = object.get("status").and_then(Json::as_str) == Some("ok");
        if let Some(warnings) = object.get("warnings").and_then(Json::as_array)
            && !json
        {
            for warning in warnings {
                let code = warning
                    .get("code")
                    .and_then(Json::as_str)
                    .unwrap_or("STC-W-DEBUG-002");
                let text = warning
                    .get("message")
                    .and_then(Json::as_str)
                    .unwrap_or("debug warning");
                reporter.plain(code, Severity::Warning, text);
            }
        }
        if ok {
            let value = object
                .get("result")
                .cloned()
                .unwrap_or_else(|| Json::object(Vec::new()));
            if !json {
                render_response(op, &value, context);
            }
            result = Some(value);
        } else {
            let error = object.get("error").and_then(Json::as_object);
            let code = error
                .and_then(|error| error.get("code"))
                .and_then(Json::as_str)
                .unwrap_or("STC-E-DEBUG-003")
                .to_owned();
            let message = error
                .and_then(|error| error.get("message"))
                .and_then(Json::as_str)
                .unwrap_or("the debug request failed")
                .to_owned();
            return Err(Failure::Server { code, message });
        }
    }
    result.ok_or_else(|| Failure::Transport("the debug server closed the connection".into()))
}

fn render_response(op: &str, result: &Json, context: Option<&str>) {
    match op {
        "session.start" => {
            let session = result.get("session").and_then(Json::as_str).unwrap_or("-");
            let mode = result
                .get("mode")
                .and_then(Json::as_str)
                .unwrap_or("unknown");
            let generation = result
                .get("generation")
                .and_then(Json::as_number)
                .unwrap_or(0);
            let cycle = result.get("cycle").and_then(Json::as_number).unwrap_or(0);
            let build = result
                .get("build_id")
                .and_then(Json::as_str)
                .unwrap_or("0x0");
            eprintln!(
                "stc: debug session {session} (mode {mode}, generation {generation}, build {build}, cycle {cycle})"
            );
            if let Some(capabilities) = result.get("capabilities").and_then(Json::as_object) {
                let scopes = capabilities
                    .get("scopes")
                    .and_then(Json::as_array)
                    .map(|scopes| {
                        scopes
                            .iter()
                            .filter_map(Json::as_str)
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_default();
                let boolean = |key: &str| {
                    capabilities
                        .get(key)
                        .and_then(Json::as_bool)
                        .unwrap_or(false)
                };
                let debug_info = capabilities
                    .get("debug_info")
                    .and_then(Json::as_str)
                    .unwrap_or("none");
                eprintln!("stc: debug scopes {scopes}");
                eprintln!(
                    "stc: debug capabilities hooks={} checkpoints={} stepping={} debug_info={debug_info}",
                    boolean("hooks"),
                    boolean("checkpoints"),
                    boolean("stepping")
                );
            }
        }
        "session.stop" => eprintln!("stc: debug session stopped"),
        "breakpoint.set" => {
            let id = result
                .get("breakpoint_id")
                .and_then(Json::as_number)
                .unwrap_or(0);
            let bound = result.get("bound").and_then(Json::as_bool).unwrap_or(false);
            eprintln!("stc: breakpoint {id} set (bound: {bound})");
        }
        "breakpoint.clear" => {
            let id = result
                .get("breakpoint_id")
                .and_then(Json::as_number)
                .unwrap_or(0);
            eprintln!("stc: breakpoint {id} cleared");
        }
        "breakpoint.list" => {
            let ids = breakpoint_ids(result);
            if ids.is_empty() {
                eprintln!("stc: no breakpoints");
            } else {
                eprintln!(
                    "stc: breakpoints {}",
                    ids.iter()
                        .map(u64::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        "watch.add" => {
            let id = result
                .get("watch_id")
                .and_then(Json::as_number)
                .unwrap_or(0);
            let interval = result
                .get("interval_ms")
                .and_then(Json::as_number)
                .unwrap_or(0);
            eprintln!("stc: watch {id} added (interval {interval}ms)");
        }
        "watch.remove" => {
            let id = result
                .get("watch_id")
                .and_then(Json::as_number)
                .unwrap_or(0);
            eprintln!("stc: watch {id} removed");
        }
        "watch.list" => {
            let watches = result
                .get("watches")
                .and_then(Json::as_array)
                .map_or(0, <[Json]>::len);
            eprintln!("stc: {watches} watch(es)");
        }
        "var.read" => {
            render_value(result, context);
        }
        "frame.read" => {
            let name = context.unwrap_or("frame");
            if let Some(text) = rendered_view(result) {
                print!("{text}");
            } else {
                let text =
                    typed_value(result).map_or_else(|| "unavailable".into(), Json::render_compact);
                eprintln!("stc: {name} = {text}");
            }
        }
        "var.write" | "var.force" => {
            let operation = if op == "var.write" { "write" } else { "force" };
            let status = result
                .get("status")
                .and_then(Json::as_str)
                .unwrap_or("accepted");
            let queue_id = result
                .get("queue_id")
                .and_then(Json::as_number)
                .unwrap_or(0);
            eprintln!(
                "stc: {operation} {status} (queue {queue_id}); it applies at the next cycle boundary"
            );
        }
        "var.release" => {
            let effective = result
                .get("effective")
                .and_then(Json::as_str)
                .unwrap_or("next-boundary");
            eprintln!("stc: release accepted; effective {effective}");
        }
        "var.release-all" => {
            let released = result
                .get("released")
                .and_then(Json::as_number)
                .unwrap_or(0);
            eprintln!("stc: released {released} force(s)");
        }
        "exec.continue" => eprintln!("stc: running"),
        "exec.step" => {
            let mode = result.get("mode").and_then(Json::as_str).unwrap_or("step");
            eprintln!("stc: stepping ({mode})");
        }
        "exec.pause" => {
            let status = result
                .get("status")
                .and_then(Json::as_str)
                .unwrap_or("requested");
            eprintln!("stc: pause {status}");
        }
        "stack.list" => {
            if let Some(frames) = result.get("frames").and_then(Json::as_array) {
                if frames.is_empty() {
                    eprintln!("stc: no frames");
                }
                for frame in frames {
                    let index = frame.get("index").and_then(Json::as_number).unwrap_or(0);
                    let symbol = frame
                        .get("symbol")
                        .and_then(Json::as_str)
                        .unwrap_or("<unknown>");
                    let optimized = frame
                        .get("optimized_out")
                        .and_then(Json::as_bool)
                        .unwrap_or(false);
                    if optimized {
                        eprintln!("stc: frame {index} {symbol} (optimized out)");
                    } else {
                        eprintln!("stc: frame {index} {symbol}");
                    }
                }
            }
        }
        _ => eprintln!("stc: {}", result.render_compact()),
    }
}

/// Renders the phase-7 presentation view when the result carries one. Range
/// reads omit the key, so the caller falls back to the typed value.
fn rendered_view(result: &Json) -> Option<String> {
    result.get("view").and_then(stc_runtime::render_view_json)
}

fn render_value(result: &Json, context: Option<&str>) {
    if let Some(text) = rendered_view(result) {
        print!("{text}");
        return;
    }
    let label = context.unwrap_or("value");
    let text = typed_value(result).map_or_else(|| "unavailable".into(), Json::render_compact);
    eprintln!("stc: {label} = {text}");
}

fn typed_value(result: &Json) -> Option<&Json> {
    result.get("value")
}

fn breakpoint_ids(result: &Json) -> Vec<u64> {
    result
        .get("breakpoints")
        .and_then(Json::as_array)
        .map(|breakpoints| {
            breakpoints
                .iter()
                .filter_map(|breakpoint| breakpoint.get("breakpoint_id"))
                .filter_map(Json::as_number)
                .filter_map(|id| u64::try_from(id).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn report_failure(failure: &Failure, reporter: &mut Reporter) {
    match failure {
        Failure::Server { code, message } => {
            reporter.plain(code, Severity::Error, message);
        }
        Failure::Ended(end) => report_end(end, reporter),
        Failure::Transport(message) => {
            reporter.plain(
                "STC-E-DEBUG-020",
                Severity::Error,
                &format!("the debug connection failed: {message}"),
            );
        }
    }
}

fn report_end(end: &EndInfo, reporter: &mut Reporter) {
    let Some((code, message)) = end_diagnostic(end) else {
        return;
    };
    reporter.plain(code, Severity::Error, message);
}

fn end_diagnostic(end: &EndInfo) -> Option<(&str, &str)> {
    match end.reason.as_str() {
        "client" | "shutdown" | "target-exited" => None,
        "swap" => Some((
            "STC-E-DEBUG-019",
            "the debug session ended because of an online swap; reconnect and re-resolve all targets against the new generation",
        )),
        "idle-timeout" | "pause-timeout" => Some((
            "STC-E-DEBUG-018",
            "the debug session ended because the stop or idle bound expired; the cycle resumed and all forces, breakpoints, and queued writes were cleared",
        )),
        "protocol-error" => Some((
            end.code.as_deref().unwrap_or("STC-E-DEBUG-022"),
            "the debug session ended after a protocol error; the connection is closed",
        )),
        "internal" => Some((
            "STC-E-DEBUG-022",
            "the debug session ended after an internal debug service error",
        )),
        "disconnect" => Some((
            "STC-E-DEBUG-020",
            "the debug session ended because the connection closed",
        )),
        _ => Some((
            end.code.as_deref().unwrap_or("STC-E-DEBUG-022"),
            "the debug session ended",
        )),
    }
}

fn exit_for_failure(failure: &Failure) -> u8 {
    match failure {
        Failure::Ended(end) => exit_for_end(end),
        Failure::Server { .. } | Failure::Transport(_) => 2,
    }
}

fn exit_for_end(end: &EndInfo) -> u8 {
    match end.reason.as_str() {
        "client" | "shutdown" => 0,
        _ => 2,
    }
}

// ---------------------------------------------------------------------------
// Session teardown
// ---------------------------------------------------------------------------

fn finish_launch(
    mut client: DebugClient,
    outcome: Outcome,
    reporter: &mut Reporter,
    control: &str,
    nonce: &str,
    child: &mut Child,
) -> u8 {
    let mut report: Option<(String, String)> = None;
    let code = match &outcome {
        Outcome::Clean => 0,
        Outcome::Interrupted => 130,
        Outcome::Ended(end) => {
            if end.reason == "target-exited" {
                // The target owns the exit code; wait for the host process.
                let code = wait_child(child).unwrap_or(2);
                let _ = stop_session(&mut client);
                return code;
            }
            report =
                end_diagnostic(end).map(|(code, message)| (code.to_owned(), message.to_owned()));
            exit_for_end(end)
        }
        Outcome::Transport(message) => {
            report = Some((
                "STC-E-DEBUG-020".to_owned(),
                format!("the debug connection failed: {message}"),
            ));
            2
        }
    };
    let _ = stop_session(&mut client);
    let child_code = shutdown_host(control, nonce, child);
    if let Some((code, message)) = &report {
        reporter.plain(code, Severity::Error, message);
    }
    match outcome {
        Outcome::Clean if matches!(child_code, Some(4 | 130)) => child_code.unwrap_or(code),
        _ => code,
    }
}

fn finish_attach(client: &mut DebugClient, outcome: Outcome, reporter: &mut Reporter) -> u8 {
    match outcome {
        Outcome::Clean => {
            let _ = stop_session(client);
            0
        }
        Outcome::Interrupted => {
            let _ = stop_session(client);
            130
        }
        Outcome::Ended(end) => {
            report_end(&end, reporter);
            exit_for_end(&end)
        }
        Outcome::Transport(message) => {
            reporter.plain(
                "STC-E-DEBUG-020",
                Severity::Error,
                &format!("the debug connection failed: {message}"),
            );
            2
        }
    }
}

/// Best-effort `session.stop`; the host clears every force, breakpoint, watch,
/// and queued write when the session ends (§14.7).
fn stop_session(client: &mut DebugClient) -> std::io::Result<()> {
    let messages = client.call_messages("session.stop", Json::object(Vec::new()))?;
    let _ = messages;
    Ok(())
}

fn shutdown_host(control: &str, nonce: &str, child: &mut Child) -> Option<u8> {
    if let Ok(mut client) = ControlClient::connect(control, CONTROL_DEADLINE) {
        let _ = client.request(nonce, Json::object([("op".into(), Json::string("stop"))]));
    }
    wait_child(child)
}

pub(crate) fn wait_child(child: &mut Child) -> Option<u8> {
    let deadline = Instant::now() + SHUTDOWN_DEADLINE;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(child_code(status.code())),
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return child.wait().ok().map(|status| child_code(status.code()));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn child_code(code: Option<i32>) -> u8 {
    match code {
        Some(code) if cfg!(windows) && code == 0xC000_013A_u32 as i32 => 130,
        Some(code) => u8::try_from(code).unwrap_or(1),
        None => 1,
    }
}

// ---------------------------------------------------------------------------
// Endpoint and token helpers
// ---------------------------------------------------------------------------

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn random_hex() -> String {
    let counter = TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let seed = format!("{}-{counter}-{nanos}", std::process::id());
    let digest = stc_got::sha256::sha256(seed.as_bytes());
    let mut token = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        token.push_str(&format!("{byte:02x}"));
    }
    token
}

pub(crate) fn control_endpoint() -> String {
    let token = random_hex();
    if cfg!(windows) {
        format!(r"\\.\pipe\stc-debug-{}-{token}", std::process::id())
    } else {
        std::env::temp_dir()
            .join(format!("stc-debug-{}-{token}.sock", std::process::id()))
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_text_parses_paths_and_got_ids() {
        let path = parse_target_text("MAIN.COUNTER").expect("path");
        assert_eq!(path.get("kind").and_then(Json::as_str), Some("path"));
        assert_eq!(path.get("key").and_then(Json::as_str), Some("MAIN.COUNTER"));
        let id = parse_target_text("0x0102030405060708090A0B0C0D0E0F10").expect("got id");
        assert_eq!(id.get("kind").and_then(Json::as_str), Some("got_id"));
        assert_eq!(
            id.get("got_id").and_then(Json::as_str),
            Some("0x0102030405060708090a0b0c0d0e0f10")
        );
        assert!(parse_target_text("0x1234").is_err());
    }

    #[test]
    fn wire_values_cover_the_scalar_forms() {
        assert_eq!(parse_wire_value("true").expect("bool"), Json::Bool(true));
        assert_eq!(parse_wire_value("-12").expect("int"), Json::Number(-12));
        assert_eq!(
            parse_wire_value("1.5").expect("real"),
            Json::NumberLiteral("1.5".into())
        );
        assert_eq!(
            parse_wire_value("'abc'").expect("string"),
            Json::string("abc")
        );
        assert!(parse_wire_value("not-a-value").is_err());
    }

    #[test]
    fn end_reasons_map_to_stable_diagnostics_and_exit_codes() {
        let end = |reason: &str, code: Option<&str>| EndInfo {
            reason: reason.into(),
            code: code.map(str::to_owned),
        };
        let swap = end("swap", None);
        assert_eq!(
            end_diagnostic(&swap).map(|(code, _)| code),
            Some("STC-E-DEBUG-019")
        );
        assert_eq!(exit_for_end(&swap), 2);
        for reason in ["idle-timeout", "pause-timeout"] {
            let end = end(reason, None);
            assert_eq!(
                end_diagnostic(&end).map(|(code, _)| code),
                Some("STC-E-DEBUG-018")
            );
            assert_eq!(exit_for_end(&end), 2);
        }
        let protocol = end("protocol-error", Some("STC-E-DEBUG-003"));
        assert_eq!(
            end_diagnostic(&protocol).map(|(code, _)| code),
            Some("STC-E-DEBUG-003")
        );
        assert_eq!(
            end_diagnostic(&end("protocol-error", None)).map(|(code, _)| code),
            Some("STC-E-DEBUG-022")
        );
        assert_eq!(
            end_diagnostic(&end("disconnect", None)).map(|(code, _)| code),
            Some("STC-E-DEBUG-020")
        );
        for reason in ["client", "shutdown", "target-exited"] {
            let end = end(reason, None);
            assert!(end_diagnostic(&end).is_none(), "{reason}");
        }
        assert_eq!(exit_for_end(&end("client", None)), 0);
        assert_eq!(exit_for_end(&end("shutdown", None)), 0);
    }

    #[test]
    fn print_and_frame_render_the_optional_presentation_view() {
        use stc_got::digest::GotId;
        use stc_runtime::{DebugValue, SplitView, ViewKind, ViewNode, ViewOrigin, view_json};
        let view = SplitView {
            target: "MAIN.POINT".into(),
            resolved_key: "MAIN.POINT".into(),
            generation: 3,
            build_id: GotId::ZERO,
            cycle: 9,
            origin: ViewOrigin::Live,
            atomic: true,
            consistent: true,
            manifest_assisted: false,
            contiguous: true,
            node: ViewNode {
                kind: ViewKind::Scalar,
                type_key: "DINT".into(),
                layout_hash: GotId::ZERO,
                value: DebugValue::Decimal("7".into()),
                available: true,
                manifest_assisted: false,
                contiguous: true,
                atomic: true,
                consistent: true,
                root: None,
                identity: None,
                fields: Vec::new(),
                elements: Vec::new(),
            },
        };
        let result = Json::object([("view".into(), view_json(&view))]);
        let text = rendered_view(&result).expect("the view is rendered");
        assert!(
            text.starts_with("contiguous single-root object view"),
            "{text}"
        );
        assert!(text.contains("target: MAIN.POINT"), "{text}");
        assert!(text.contains("= 7"), "{text}");

        // A range (sliced) read has no `view` key; the caller uses the typed
        // value instead (D-182).
        let sliced = Json::object([(
            "value".into(),
            Json::object([
                ("type".into(), Json::string("DINT")),
                ("value".into(), Json::string("7")),
            ]),
        )]);
        assert!(rendered_view(&sliced).is_none());
    }

    #[test]
    fn break_specs_split_from_the_right() {
        assert_eq!(
            parse_break_spec("src/main.st:14:9").expect("full"),
            ("src/main.st".into(), 14, 9)
        );
        assert_eq!(
            parse_break_spec("src/main.st:14").expect("line only"),
            ("src/main.st".into(), 14, 0)
        );
        assert!(parse_break_spec("src/main.st").is_err());
        assert!(parse_break_spec("src/main.st:0").is_err());
    }
}
