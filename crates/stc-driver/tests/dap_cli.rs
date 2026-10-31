//! `stc dap` end-to-end tests (WL-1.1 phase 4).
//!
//! Each test spawns the real binary and speaks `Content-Length` framed DAP
//! over stdio. The launch tests build a temporary project with clang and stop
//! at a breakpoint on the first executable statement; the attach test connects
//! to an already-running `stc run --debug-server`. Nothing under `examples/`
//! is written.

mod common;

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Waits that include a project build and host launch.
const BUILD_TIMEOUT: Duration = Duration::from_secs(600);
/// Protocol-only waits.
const PROTOCOL_TIMEOUT: Duration = Duration::from_secs(60);

fn stc() -> &'static str {
    env!("CARGO_BIN_EXE_stc")
}

fn clang() -> Option<&'static str> {
    static CLANG: OnceLock<Option<&'static str>> = OnceLock::new();
    CLANG
        .get_or_init(|| {
            let candidate = std::env::var("STC_CLANG").unwrap_or_else(|_| "clang".into());
            match Command::new(&candidate).arg("--version").output() {
                Ok(output) if output.status.success() => {
                    Some(Box::leak(candidate.into_boxed_str()))
                }
                _ => {
                    if std::env::var_os("STC_REQUIRE_CLANG").is_some() {
                        panic!("clang is required (STC_REQUIRE_CLANG=1) but was not found");
                    }
                    eprintln!("note: clang not found; skipping DAP integration checks");
                    None
                }
            }
        })
        .as_ref()
        .copied()
}

fn runtime_artifact() -> PathBuf {
    common::runtime_static()
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stc-dap-cli-{}-{name}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test directory");
        let path = std::fs::canonicalize(&path).expect("canonical test directory");
        Self { path }
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = relative
            .split('/')
            .fold(self.path.clone(), |path, part| path.join(part));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create directory");
        }
        std::fs::write(&path, contents).expect("write file");
        path
    }

    fn manifest(&self) -> PathBuf {
        self.path.join("stc.toml")
    }

    fn source(&self) -> PathBuf {
        self.path.join("src").join("main.st")
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

const MANIFEST: &str = "\
schema = 1

[package]
name = \"demo\"
version = \"0.1.0\"
language = \"stc-2013-oop-utf8-v1\"

[project]
sources = [\"src/**/*.st\"]
";

/// Line 20 is the program's first executable statement; line 8 is the first
/// statement of the `AddOne` function, whose locals are stack locals. PROGRAM
/// variables (`ticks`) are instance roots, not stack locals, so the function
/// call is what exercises the Locals scope.
const SOURCE: &str = "\
FUNCTION AddOne : DINT
VAR_INPUT
  n : DINT;
END_VAR
VAR
  doubled : DINT;
END_VAR
  doubled := n + n;
  AddOne := doubled + 1;
END_FUNCTION

VAR_GLOBAL
  total : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  total := AddOne(ticks);
END_PROGRAM
";

const FIRST_STATEMENT_LINE: i64 = 20;
/// First executable statement of `AddOne`.
const FUNCTION_LINE: i64 = 8;
/// The statement after `AddOne`'s first executable statement.
const FUNCTION_NEXT_LINE: i64 = 9;
/// The statement after `Main`'s first executable statement.
const PROGRAM_SECOND_LINE: i64 = 21;

fn scaffold(name: &str) -> TestDir {
    let project = TestDir::new(name);
    project.write("stc.toml", MANIFEST);
    project.write("src/main.st", SOURCE);
    project
}

/// Returns the checked-in sample workspace used by the IDE tests.
fn sample_workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../editors/vscode/samples/workspace")
}

/// Copies a directory tree, skipping generated artifact directories.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create directory");
    for entry in std::fs::read_dir(from).expect("read directory") {
        let entry = entry.expect("directory entry");
        let name = entry.file_name();
        if matches!(name.to_str(), Some("target" | "state")) {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().expect("entry type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

/// Returns the 1-based line containing the first `needle`.
fn line_of(source: &str, needle: &str) -> i64 {
    let offset = source
        .find(needle)
        .unwrap_or_else(|| panic!("missing {needle:?}"));
    i64::try_from(source[..offset].matches('\n').count()).expect("line") + 1
}

// ---------------------------------------------------------------------------
// Raw DAP client
// ---------------------------------------------------------------------------

struct DapClient {
    child: Child,
    stdin: ChildStdin,
    receiver: Receiver<Value>,
    events: VecDeque<Value>,
    seq: i64,
}

impl DapClient {
    fn spawn(directory: &Path) -> Self {
        let runtime = runtime_artifact();
        let mut child = Command::new(stc())
            .arg("dap")
            .current_dir(directory)
            .env("STC_RUNTIME", runtime)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn stc dap");
        let stdin = child.stdin.take().expect("dap stdin");
        let stdout = child.stdout.take().expect("dap stdout");
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("dap-test-reader".to_owned())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                while let Some(message) = read_frame(&mut reader) {
                    if sender.send(message).is_err() {
                        break;
                    }
                }
            })
            .expect("reader thread");
        Self {
            child,
            stdin,
            receiver,
            events: VecDeque::new(),
            seq: 0,
        }
    }

    fn send_request(&mut self, command: &str, arguments: Value) -> i64 {
        self.seq += 1;
        let message = json!({
            "seq": self.seq,
            "type": "request",
            "command": command,
            "arguments": arguments,
        });
        let text = message.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n", text.len()).expect("write header");
        self.stdin.write_all(text.as_bytes()).expect("write body");
        self.stdin.flush().expect("flush");
        self.seq
    }

    /// Sends a request and waits for its response, buffering events.
    fn request(&mut self, command: &str, arguments: Value) -> Value {
        let seq = self.send_request(command, arguments);
        self.response(seq, BUILD_TIMEOUT)
    }

    fn response(&mut self, request_seq: i64, timeout: Duration) -> Value {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.receiver.recv_timeout(remaining) {
                Ok(message) => {
                    if message["type"] == "response" && message["request_seq"] == request_seq {
                        return message;
                    }
                    self.events.push_back(message);
                }
                Err(RecvTimeoutError::Timeout) => {
                    panic!(
                        "no response for request {request_seq}; events: {:?}",
                        self.events
                    )
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let status = self.child.wait().ok();
                    panic!(
                        "the adapter exited before responding to request {request_seq} ({status:?}); events: {:?}",
                        self.events
                    );
                }
            }
        }
    }

    /// Waits for one event by name, buffering everything else.
    fn event(&mut self, name: &str, timeout: Duration) -> Value {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.receiver.recv_timeout(remaining) {
                Ok(message) => {
                    if message["type"] == "event" && message["event"] == name {
                        return message;
                    }
                    self.events.push_back(message);
                }
                Err(RecvTimeoutError::Timeout) => {
                    panic!("no `{name}` event; events: {:?}", self.events)
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let status = self.child.wait().ok();
                    panic!(
                        "the adapter exited before the `{name}` event ({status:?}); events: {:?}",
                        self.events
                    );
                }
            }
        }
    }

    /// Moves every already-received message into the event buffer.
    fn collect_events(&mut self) {
        while let Ok(message) = self.receiver.try_recv() {
            self.events.push_back(message);
        }
    }

    fn events(&self) -> impl Iterator<Item = &Value> {
        self.events.iter()
    }

    fn wait_exit(&mut self, timeout: Duration) -> Option<i32> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return status.code(),
                Ok(None) => {}
                Err(_) => return None,
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                return None;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for DapClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Reads one `Content-Length` framed message, or `None` at end of stream.
fn read_frame(reader: &mut BufReader<std::process::ChildStdout>) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return None,
            Ok(_) => {}
            Err(_) => return None,
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let length = length.expect("Content-Length header");
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body).expect("frame body");
    Some(serde_json::from_slice(&body).expect("DAP JSON"))
}

fn initialize(client: &mut DapClient) {
    let response = client.request(
        "initialize",
        json!({
            "clientID": "stc-test",
            "adapterID": "stc",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "pathFormat": "path",
        }),
    );
    assert_eq!(response["success"], true, "{response}");
    let capabilities = &response["body"];
    assert_eq!(capabilities["supportsConfigurationDoneRequest"], true);
    assert_eq!(capabilities["supportsConditionalBreakpoints"], false);
    assert_eq!(capabilities["supportsRestartRequest"], false);
    let initialized = client.event("initialized", PROTOCOL_TIMEOUT);
    assert_eq!(initialized["type"], "event");
}

fn launch_arguments(project: &TestDir) -> Value {
    json!({
        "project": project.manifest().to_string_lossy(),
        "allowWrite": true,
        "args": ["--cycles", "20000", "--cycle-time", "10ms"],
    })
}

fn breakpoint_arguments(project: &TestDir) -> Value {
    json!({
        "source": { "path": project.source().to_string_lossy() },
        "breakpoints": [
            { "line": FIRST_STATEMENT_LINE },
            { "line": FUNCTION_LINE },
        ],
        "lines": [FIRST_STATEMENT_LINE, FUNCTION_LINE],
    })
}

/// Returns the display names of a `stackTrace` response in stack order.
fn stack_frame_names(stack: &Value) -> Vec<&str> {
    stack["body"]["stackFrames"]
        .as_array()
        .expect("stack frames")
        .iter()
        .map(|frame| frame["name"].as_str().expect("frame name"))
        .collect()
}

/// Returns the DAP frame ids of a `stackTrace` response in stack order.
fn stack_frame_ids(stack: &Value) -> Vec<i64> {
    stack["body"]["stackFrames"]
        .as_array()
        .expect("stack frames")
        .iter()
        .map(|frame| frame["id"].as_i64().expect("frame id"))
        .collect()
}

fn scope_reference(scopes: &Value, name: &str) -> i64 {
    scopes["body"]["scopes"]
        .as_array()
        .expect("scopes")
        .iter()
        .find(|scope| scope["name"] == name)
        .unwrap_or_else(|| panic!("scope `{name}` in {scopes}"))["variablesReference"]
        .as_i64()
        .expect("variablesReference")
}

fn find_variable<'a>(variables: &'a Value, name: &str) -> &'a Value {
    variables["body"]["variables"]
        .as_array()
        .expect("variables")
        .iter()
        .find(|variable| variable["name"] == name)
        .unwrap_or_else(|| panic!("variable `{name}` in {variables}"))
}

/// Strips the Windows verbatim prefix the DAP source path intentionally omits.
fn normalized_path(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
}

/// Returns the normal (non-verbatim) spelling of a test path.
///
/// On Windows the test directory is canonicalized to `\\?\C:\...`; stripping
/// the prefix gives the spelling a DAP client normally sends. Both spellings
/// must map to the same adapter breakpoint key.
fn plain_source_path(path: &Path) -> String {
    #[cfg(windows)]
    {
        normalized_path(path)
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().into_owned()
    }
}

/// Returns the Windows verbatim (`\\?\`) spelling of a test path.
///
/// Outside Windows there is no verbatim prefix; backslash separators still
/// normalize to the same source key.
fn verbatim_source_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    {
        if text.starts_with(r"\\?\") {
            text.into_owned()
        } else {
            format!(r"\\?\{text}")
        }
    }
    #[cfg(not(windows))]
    {
        text.replace('/', "\\")
    }
}

/// Waits for a breakpoint stop and checks the frame and source mapping.
fn stop_at_line(client: &mut DapClient, project: &TestDir, line: i64) -> (Value, Value) {
    let stopped = client.event("stopped", PROTOCOL_TIMEOUT);
    assert_eq!(stopped["body"]["threadId"], 1, "{stopped}");
    assert_eq!(stopped["body"]["reason"], "breakpoint", "{stopped}");
    let description = stopped["body"]["description"]
        .as_str()
        .expect("stop description");
    assert!(
        description.ends_with(&format!(":{line}")),
        "the stop is at line {line}: {stopped}"
    );

    let stack = client.request("stackTrace", json!({ "threadId": 1 }));
    assert_eq!(stack["success"], true, "{stack}");
    let frames = stack["body"]["stackFrames"].as_array().expect("frames");
    assert!(!frames.is_empty(), "{stack}");
    let source = &frames[0]["source"];
    let expected_source = normalized_path(&project.source());
    assert_eq!(
        source["path"].as_str(),
        Some(expected_source.as_str()),
        "frame source maps to the workspace file: {stack}"
    );
    assert_eq!(frames[0]["line"], line, "{stack}");
    assert!(
        frames[0]["name"]
            .as_str()
            .is_some_and(|name| !name.is_empty()),
        "{stack}"
    );

    let frame_id = frames[0]["id"].as_i64().expect("frame id");
    let scopes = client.request("scopes", json!({ "frameId": frame_id }));
    assert_eq!(scopes["success"], true, "{scopes}");
    (stack, scopes)
}

fn read_global(client: &mut DapClient, scopes: &Value, name: &str) -> String {
    let globals = scope_reference(scopes, "Globals");
    let variables = client.request("variables", json!({ "variablesReference": globals }));
    find_variable(&variables, name)["value"]
        .as_str()
        .unwrap_or_else(|| panic!("`{name}` value in {variables}"))
        .to_owned()
}

fn read_local(client: &mut DapClient, scopes: &Value, name: &str) -> String {
    let locals = scope_reference(scopes, "Locals");
    let variables = client.request("variables", json!({ "variablesReference": locals }));
    find_variable(&variables, name)["value"]
        .as_str()
        .unwrap_or_else(|| panic!("`{name}` value in {variables}"))
        .to_owned()
}

#[test]
fn launch_stops_at_a_breakpoint_and_reads_locals_and_globals() {
    let Some(_clang) = clang() else {
        return;
    };
    let project = scaffold("launch");
    let mut client = DapClient::spawn(&project.path);
    initialize(&mut client);

    // Breakpoints arrive before the launch request.
    let breakpoints = client.request("setBreakpoints", breakpoint_arguments(&project));
    assert_eq!(breakpoints["success"], true, "{breakpoints}");
    assert_eq!(
        breakpoints["body"]["breakpoints"]
            .as_array()
            .expect("breakpoints")
            .len(),
        2
    );

    let launch = client.request("launch", launch_arguments(&project));
    assert_eq!(launch["success"], true, "{launch}");
    let exception = client.request("setExceptionBreakpoints", json!({ "filters": [] }));
    assert_eq!(exception["success"], true, "{exception}");
    assert_eq!(
        exception["body"]["breakpoints"]
            .as_array()
            .expect("exception breakpoints")
            .len(),
        0
    );
    let threads = client.request("threads", json!({}));
    assert_eq!(
        threads["body"]["threads"],
        json!([{ "id": 1, "name": "cycle" }])
    );
    let configuration = client.request("configurationDone", json!({}));
    assert_eq!(configuration["success"], true, "{configuration}");

    let (_stack, scopes) = stop_at_line(&mut client, &project, FIRST_STATEMENT_LINE);
    let total = read_global(&mut client, &scopes, "TOTAL");
    assert!(
        total.parse::<i64>().is_ok(),
        "TOTAL is a decimal integer: {total}"
    );

    // Writes are enabled by `allowWrite`: a global root accepts a new value.
    let globals = scope_reference(&scopes, "Globals");
    let write = client.request(
        "setVariable",
        json!({ "variablesReference": globals, "name": "TOTAL", "value": "42" }),
    );
    assert_eq!(write["success"], true, "{write}");
    assert_eq!(write["body"]["value"], "42");
    assert_eq!(write["body"]["type"], "DINT");

    // Continue into the function: its locals live on the stack.
    let continued = client.request("continue", json!({ "threadId": 1 }));
    assert_eq!(continued["success"], true, "{continued}");
    let (_stack, function_scopes) = stop_at_line(&mut client, &project, FUNCTION_LINE);
    let doubled = read_local(&mut client, &function_scopes, "DOUBLED");
    assert!(
        doubled.parse::<i64>().is_ok(),
        "DOUBLED is a decimal integer: {doubled}"
    );
    let argument = read_local(&mut client, &function_scopes, "N");
    assert!(
        argument.parse::<i64>().is_ok(),
        "N is a decimal integer: {argument}"
    );
    let total = read_global(&mut client, &function_scopes, "TOTAL");
    assert!(
        total.parse::<i64>().is_ok(),
        "TOTAL is a decimal integer: {total}"
    );

    // Evaluate accepts a stopped-frame local name or a manifest target path;
    // it does not parse ST expressions.
    let local_evaluate = client.request(
        "evaluate",
        json!({ "expression": "DOUBLED", "frameId": 0, "context": "watch" }),
    );
    assert_eq!(local_evaluate["success"], true, "{local_evaluate}");
    assert_eq!(local_evaluate["body"]["type"], "DINT");
    assert!(
        local_evaluate["body"]["result"]
            .as_str()
            .is_some_and(|result| result.parse::<i64>().is_ok()),
        "{local_evaluate}"
    );
    let global_evaluate = client.request(
        "evaluate",
        json!({ "expression": "TOTAL", "frameId": 0, "context": "watch" }),
    );
    assert_eq!(global_evaluate["success"], true, "{global_evaluate}");
    let expression = client.request(
        "evaluate",
        json!({ "expression": "1 + 2", "frameId": 0, "context": "repl" }),
    );
    assert_eq!(expression["success"], false, "{expression}");
    assert!(
        expression["message"]
            .as_str()
            .is_some_and(|message| message.contains("ST expressions are not supported")),
        "{expression}"
    );

    // Snapshot variables are read-only.
    let locals = scope_reference(&function_scopes, "Locals");
    let read_only = client.request(
        "setVariable",
        json!({ "variablesReference": locals, "name": "DOUBLED", "value": "7" }),
    );
    assert_eq!(read_only["success"], false, "{read_only}");

    // Step over the function's first statement.
    let next = client.request("next", json!({ "threadId": 1 }));
    assert_eq!(next["success"], true, "{next}");
    let stepped = client.event("stopped", PROTOCOL_TIMEOUT);
    assert_eq!(stepped["body"]["reason"], "step", "{stepped}");
    let stepped_stack = client.request("stackTrace", json!({ "threadId": 1 }));
    let line = stepped_stack["body"]["stackFrames"][0]["line"]
        .as_i64()
        .expect("line");
    assert!(
        line >= FUNCTION_LINE,
        "the step landed at or after the function statement: {stepped_stack}"
    );

    let continued = client.request("continue", json!({ "threadId": 1 }));
    assert_eq!(continued["success"], true, "{continued}");
    let continued_event = client.event("continued", PROTOCOL_TIMEOUT);
    assert_eq!(continued_event["body"]["threadId"], 1);

    let disconnect = client.request("disconnect", json!({ "terminateDebuggee": true }));
    assert_eq!(disconnect["success"], true, "{disconnect}");
    assert!(
        client.wait_exit(PROTOCOL_TIMEOUT).is_some(),
        "the adapter exits after disconnect"
    );

    // The non-safety notice is an output event at session start.
    client.collect_events();
    let output: Vec<String> = client
        .events()
        .filter(|event| event["event"] == "output")
        .filter_map(|event| event["body"]["output"].as_str().map(str::to_owned))
        .collect();
    assert!(
        output.iter().any(|line| line.contains("not a safety PLC")),
        "the non-safety notice is emitted: {output:?}"
    );
}

#[test]
fn sample_workspace_launch_reads_known_values_and_steps() {
    let Some(_clang) = clang() else {
        return;
    };
    let project = TestDir::new("sample");
    copy_tree(&sample_workspace(), &project.path);
    let main = project.source();
    let source = std::fs::read_to_string(&main).expect("read sample source");
    let first_statement = line_of(&source, "sum := AddStep(sum);");
    let function_statement = line_of(&source, "incremented := INCREMENT;");
    let function_next = line_of(&source, "AddStep := value + incremented;");

    let mut client = DapClient::spawn(&project.path);
    initialize(&mut client);
    let breakpoints = client.request(
        "setBreakpoints",
        json!({
            "source": { "path": main.to_string_lossy() },
            "breakpoints": [
                { "line": first_statement },
                { "line": function_statement },
            ],
            "lines": [first_statement, function_statement],
        }),
    );
    assert_eq!(breakpoints["success"], true, "{breakpoints}");

    // The sample launch configuration: build, debug form, writes and forces
    // disabled.
    let launch = client.request(
        "launch",
        json!({
            "project": project.manifest().to_string_lossy(),
            "noBuild": false,
            "release": false,
            "allowWrite": false,
            "allowForce": false,
            "args": ["--cycles", "20000", "--cycle-time", "10ms"],
        }),
    );
    assert_eq!(launch["success"], true, "{launch}");
    let configuration = client.request("configurationDone", json!({}));
    assert_eq!(configuration["success"], true, "{configuration}");

    // First-statement stop: the never-written global has its initial value.
    // `TOTAL` depends on how many cycles ran before the breakpoints bound, so
    // the test only requires it to be a decimal integer.
    let (stack, scopes) = stop_at_line(&mut client, &project, first_statement);
    assert_eq!(stack["body"]["stackFrames"][0]["name"], "MAIN", "{stack}");
    // F-12: the call stack lists only the resolved ST frame. Launcher/host
    // frames have no manifest function identity and must not leak in as
    // `frame`/Unknown Source entries.
    assert_eq!(stack_frame_names(&stack), ["MAIN"], "{stack}");
    assert_eq!(stack_frame_ids(&stack), [0], "{stack}");
    assert_eq!(stack["body"]["totalFrames"], json!(1), "{stack}");
    assert_eq!(read_global(&mut client, &scopes, "INCREMENT"), "1");
    let total = read_global(&mut client, &scopes, "TOTAL");
    assert!(
        total.parse::<i64>().is_ok(),
        "TOTAL is a decimal integer: {total}"
    );
    let globals = scope_reference(&scopes, "Globals");
    // The program instance exposes its nested aggregate fields, including the
    // FB instance whose children come from the manifest-assisted view.
    let globals_variables = client.request("variables", json!({ "variablesReference": globals }));
    let instance = globals_variables["body"]["variables"]
        .as_array()
        .expect("globals")
        .iter()
        .find(|variable| variable["value"] == "<program instance>")
        .expect("program instance");
    let instance_reference = instance["variablesReference"]
        .as_i64()
        .expect("instance reference");
    let fields = client.request(
        "variables",
        json!({ "variablesReference": instance_reference }),
    );
    for expected in ["ACCUMULATOR", "CYCLES", "SUM"] {
        find_variable(&fields, expected);
    }
    let accumulator = find_variable(&fields, "ACCUMULATOR");
    let accumulator_reference = accumulator["variablesReference"]
        .as_i64()
        .expect("accumulator reference");
    assert_ne!(accumulator_reference, 0, "{accumulator}");
    let members = client.request(
        "variables",
        json!({ "variablesReference": accumulator_reference }),
    );
    for expected in ["INCREMENT", "TOTAL"] {
        find_variable(&members, expected);
    }

    // Watch/evaluate resolves visible instance fields by name, not only raw
    // manifest paths; arbitrary ST expressions are still rejected.
    let increment = client.request(
        "evaluate",
        json!({ "expression": "INCREMENT", "frameId": 0, "context": "watch" }),
    );
    assert_eq!(increment["body"]["result"], "1", "{increment}");
    let sum = client.request(
        "evaluate",
        json!({ "expression": "sum", "frameId": 0, "context": "watch" }),
    );
    assert!(
        sum["body"]["result"]
            .as_str()
            .is_some_and(|value| value.parse::<i64>().is_ok()),
        "{sum}"
    );
    let expression = client.request(
        "evaluate",
        json!({ "expression": "1 + 2", "frameId": 0, "context": "repl" }),
    );
    assert_eq!(expression["success"], false, "{expression}");
    assert!(
        expression["message"]
            .as_str()
            .is_some_and(|message| message.contains("ST expressions are not supported")),
        "{expression}"
    );

    let denied = client.request(
        "setVariable",
        json!({ "variablesReference": globals, "name": "TOTAL", "value": "42" }),
    );
    assert_eq!(denied["success"], false, "{denied}");
    assert!(
        denied["message"]
            .as_str()
            .is_some_and(|message| message.contains("allowWrite")),
        "{denied}"
    );

    // Continue into the function and read its known locals.
    let continued = client.request("continue", json!({ "threadId": 1 }));
    assert_eq!(continued["success"], true, "{continued}");
    let (stack, function_scopes) = stop_at_line(&mut client, &project, function_statement);
    assert_eq!(
        stack["body"]["stackFrames"][0]["name"], "ADDSTEP",
        "{stack}"
    );
    // F-12: only the two resolved ST frames remain. `MAIN` has no source
    // location yet (F-11) but keeps its resolved manifest identity, so the
    // filter must not drop it for the missing source.
    assert_eq!(stack_frame_names(&stack), ["ADDSTEP", "MAIN"], "{stack}");
    assert_eq!(stack_frame_ids(&stack), [0, 1], "{stack}");
    assert_eq!(stack["body"]["totalFrames"], json!(2), "{stack}");
    assert!(
        stack["body"]["stackFrames"][1].get("source").is_none(),
        "the caller frame has no source location: {stack}"
    );
    // The kept runtime indices still address the captured frames: scopes and
    // frame.locals for the caller (`MAIN`) succeed with its own index rather
    // than a renumbered one.
    let caller_scopes = client.request("scopes", json!({ "frameId": 1 }));
    assert_eq!(caller_scopes["success"], true, "{caller_scopes}");
    let caller_locals = scope_reference(&caller_scopes, "Locals");
    let caller_variables =
        client.request("variables", json!({ "variablesReference": caller_locals }));
    assert_eq!(caller_variables["success"], true, "{caller_variables}");
    assert_eq!(
        read_local(&mut client, &function_scopes, "INCREMENTED"),
        "0"
    );
    let value = read_local(&mut client, &function_scopes, "VALUE");
    assert!(
        value.parse::<i64>().is_ok(),
        "VALUE is a decimal integer: {value}"
    );
    assert_eq!(read_global(&mut client, &function_scopes, "INCREMENT"), "1");

    // Step over the first function statement: the local becomes 1.
    let next = client.request("next", json!({ "threadId": 1 }));
    assert_eq!(next["success"], true, "{next}");
    let stepped = client.event("stopped", PROTOCOL_TIMEOUT);
    assert_eq!(stepped["body"]["reason"], "step", "{stepped}");
    let stepped_stack = client.request("stackTrace", json!({ "threadId": 1 }));
    assert_eq!(
        stepped_stack["body"]["stackFrames"][0]["line"],
        json!(function_next),
        "{stepped_stack}"
    );
    let frame_id = stepped_stack["body"]["stackFrames"][0]["id"]
        .as_i64()
        .expect("frame id");
    let stepped_scopes = client.request("scopes", json!({ "frameId": frame_id }));
    assert_eq!(read_local(&mut client, &stepped_scopes, "INCREMENTED"), "1");

    let disconnect = client.request("disconnect", json!({ "terminateDebuggee": true }));
    assert_eq!(disconnect["success"], true, "{disconnect}");
    assert!(
        client.wait_exit(PROTOCOL_TIMEOUT).is_some(),
        "the adapter exits after disconnect"
    );
}

#[test]
fn launch_accepts_breakpoints_after_the_launch_request() {
    let Some(_clang) = clang() else {
        return;
    };
    let project = scaffold("launch-after");
    let mut client = DapClient::spawn(&project.path);
    initialize(&mut client);

    // The launch request arrives first; breakpoints follow before
    // configurationDone.
    let launch = client.request("launch", launch_arguments(&project));
    assert_eq!(launch["success"], true, "{launch}");
    let breakpoints = client.request("setBreakpoints", breakpoint_arguments(&project));
    assert_eq!(breakpoints["success"], true, "{breakpoints}");
    let configuration = client.request("configurationDone", json!({}));
    assert_eq!(configuration["success"], true, "{configuration}");

    let _ = stop_at_line(&mut client, &project, FIRST_STATEMENT_LINE);
    let disconnect = client.request("disconnect", json!({ "terminateDebuggee": true }));
    assert_eq!(disconnect["success"], true, "{disconnect}");
    assert!(client.wait_exit(PROTOCOL_TIMEOUT).is_some());
}

/// F-6: removing breakpoints while stopped clears their host IDs, and
/// re-adding a later breakpoint works without restarting the session.
#[test]
fn removing_breakpoints_while_stopped_clears_them_from_the_session() {
    let Some(_clang) = clang() else {
        return;
    };
    let project = scaffold("remove-stopped");
    let mut client = DapClient::spawn(&project.path);
    initialize(&mut client);

    // The initial request uses the test directory's canonical spelling; the
    // update below uses the normal spelling, so both must share one key.
    let breakpoints = client.request("setBreakpoints", breakpoint_arguments(&project));
    assert_eq!(breakpoints["success"], true, "{breakpoints}");
    let launch = client.request("launch", launch_arguments(&project));
    assert_eq!(launch["success"], true, "{launch}");
    let configuration = client.request("configurationDone", json!({}));
    assert_eq!(configuration["success"], true, "{configuration}");
    let _ = stop_at_line(&mut client, &project, FIRST_STATEMENT_LINE);

    // Remove every breakpoint while the target is stopped. The adapter must
    // send `breakpoint.clear` for both previously applied host IDs.
    let removed = client.request(
        "setBreakpoints",
        json!({
            "source": { "path": plain_source_path(&project.source()) },
            "breakpoints": [],
        }),
    );
    assert_eq!(removed["success"], true, "{removed}");
    assert_eq!(
        removed["body"]["breakpoints"]
            .as_array()
            .expect("breakpoints")
            .len(),
        0,
        "{removed}"
    );

    // Re-add a breakpoint at a later `AddOne` statement the target reaches in
    // the same call. A stale host breakpoint would stop at the function's
    // first statement instead.
    let readded = client.request(
        "setBreakpoints",
        json!({
            "source": { "path": verbatim_source_path(&project.source()) },
            "breakpoints": [{ "line": FUNCTION_NEXT_LINE }],
        }),
    );
    assert_eq!(readded["success"], true, "{readded}");
    let readded_breakpoints = readded["body"]["breakpoints"]
        .as_array()
        .expect("breakpoints");
    assert_eq!(readded_breakpoints.len(), 1, "{readded}");
    assert_eq!(
        readded_breakpoints[0]["line"], FUNCTION_NEXT_LINE,
        "{readded}"
    );
    assert_eq!(readded_breakpoints[0]["verified"], true, "{readded}");

    let continued = client.request("continue", json!({ "threadId": 1 }));
    assert_eq!(continued["success"], true, "{continued}");
    let _ = stop_at_line(&mut client, &project, FUNCTION_NEXT_LINE);

    let disconnect = client.request("disconnect", json!({ "terminateDebuggee": true }));
    assert_eq!(disconnect["success"], true, "{disconnect}");
    assert!(client.wait_exit(PROTOCOL_TIMEOUT).is_some());
}

/// F-6: removing breakpoints while the target is running clears them, and a
/// later breakpoint added in the same update stops on the next cycle.
#[test]
fn removing_breakpoints_while_running_clears_them_from_the_session() {
    let Some(_clang) = clang() else {
        return;
    };
    let project = scaffold("remove-running");
    let mut client = DapClient::spawn(&project.path);
    initialize(&mut client);

    let breakpoints = client.request(
        "setBreakpoints",
        json!({
            "source": { "path": project.source().to_string_lossy() },
            "breakpoints": [{ "line": FIRST_STATEMENT_LINE }],
        }),
    );
    assert_eq!(breakpoints["success"], true, "{breakpoints}");

    // A one-second cycle leaves ample time for the update requests to reach
    // the host between cycles; the assertion still checks deterministically
    // which later-line stop occurs.
    let launch = client.request(
        "launch",
        json!({
            "project": project.manifest().to_string_lossy(),
            "allowWrite": true,
            "args": ["--cycles", "20000", "--cycle-time", "1s"],
        }),
    );
    assert_eq!(launch["success"], true, "{launch}");
    let configuration = client.request("configurationDone", json!({}));
    assert_eq!(configuration["success"], true, "{configuration}");
    let _ = stop_at_line(&mut client, &project, FIRST_STATEMENT_LINE);

    // Resume so the target is running, then remove the first-statement
    // breakpoint and add the next statement's. A stale host ID would stop at
    // the first statement again in the next cycle.
    let continued = client.request("continue", json!({ "threadId": 1 }));
    assert_eq!(continued["success"], true, "{continued}");
    let removed = client.request(
        "setBreakpoints",
        json!({
            "source": { "path": plain_source_path(&project.source()) },
            "breakpoints": [],
        }),
    );
    assert_eq!(removed["success"], true, "{removed}");
    let readded = client.request(
        "setBreakpoints",
        json!({
            "source": { "path": verbatim_source_path(&project.source()) },
            "breakpoints": [{ "line": PROGRAM_SECOND_LINE }],
        }),
    );
    assert_eq!(readded["success"], true, "{readded}");
    assert_eq!(
        readded["body"]["breakpoints"][0]["verified"], true,
        "{readded}"
    );

    let _ = stop_at_line(&mut client, &project, PROGRAM_SECOND_LINE);

    let disconnect = client.request("disconnect", json!({ "terminateDebuggee": true }));
    assert_eq!(disconnect["success"], true, "{disconnect}");
    assert!(client.wait_exit(PROTOCOL_TIMEOUT).is_some());
}

#[test]
fn launch_no_build_uses_the_existing_artifacts() {
    let Some(_clang) = clang() else {
        return;
    };
    let project = scaffold("no-build");
    let runtime = runtime_artifact();
    let build = Command::new(stc())
        .args([
            "build",
            "--manifest-path",
            &project.manifest().to_string_lossy(),
            "--emit",
            "exe,module,metadata",
            "--runtime",
            &runtime.to_string_lossy(),
        ])
        .current_dir(&project.path)
        .output()
        .expect("run stc build");
    assert!(
        build.status.success(),
        "the noBuild fixture must build:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out_dir = project.path.join("target").join("stc").join("debug");
    let mut client = DapClient::spawn(&project.path);
    initialize(&mut client);
    let breakpoints = client.request("setBreakpoints", breakpoint_arguments(&project));
    assert_eq!(breakpoints["success"], true, "{breakpoints}");
    let launch = client.request(
        "launch",
        json!({
            "project": project.manifest().to_string_lossy(),
            "noBuild": true,
            "outDir": out_dir.to_string_lossy(),
            "args": ["--cycles", "20000", "--cycle-time", "10ms"],
        }),
    );
    assert_eq!(launch["success"], true, "{launch}");
    let configuration = client.request("configurationDone", json!({}));
    assert_eq!(configuration["success"], true, "{configuration}");
    let _ = stop_at_line(&mut client, &project, FIRST_STATEMENT_LINE);
    let disconnect = client.request("disconnect", json!({ "terminateDebuggee": true }));
    assert_eq!(disconnect["success"], true, "{disconnect}");
    assert!(client.wait_exit(PROTOCOL_TIMEOUT).is_some());
}

#[test]
fn attach_leaves_the_running_host_alive() {
    let Some(_clang) = clang() else {
        return;
    };
    let project = scaffold("attach");
    let endpoint = project.path.join("attach.endpoint.json");
    let runtime = runtime_artifact();
    let mut host = Command::new(stc())
        .args([
            "run",
            "--debug-server",
            "--debug-endpoint",
            &endpoint.to_string_lossy(),
            "--cycles",
            "20000",
            "--cycle-time",
            "10ms",
            "--runtime",
            &runtime.to_string_lossy(),
        ])
        .current_dir(&project.path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn stc run --debug-server");
    let deadline = Instant::now() + BUILD_TIMEOUT;
    while !endpoint.is_file() {
        assert!(
            host.try_wait().ok().flatten().is_none(),
            "the host exited before publishing its endpoint"
        );
        assert!(
            Instant::now() < deadline,
            "the debug endpoint was not created"
        );
        thread::sleep(Duration::from_millis(20));
    }

    let mut client = DapClient::spawn(&project.path);
    initialize(&mut client);
    let breakpoints = client.request("setBreakpoints", breakpoint_arguments(&project));
    assert_eq!(breakpoints["success"], true, "{breakpoints}");
    let attach = client.request(
        "attach",
        json!({
            "project": project.manifest().to_string_lossy(),
            "endpoint": endpoint.to_string_lossy(),
        }),
    );
    assert_eq!(attach["success"], true, "{attach}");
    let configuration = client.request("configurationDone", json!({}));
    assert_eq!(configuration["success"], true, "{configuration}");
    let (_stack, scopes) = stop_at_line(&mut client, &project, FIRST_STATEMENT_LINE);
    // Attach did not set `allowWrite`: the adapter refuses the write.
    let globals = scope_reference(&scopes, "Globals");
    let denied = client.request(
        "setVariable",
        json!({ "variablesReference": globals, "name": "TOTAL", "value": "1" }),
    );
    assert_eq!(denied["success"], false, "{denied}");
    assert!(
        denied["message"]
            .as_str()
            .is_some_and(|message| message.contains("allowWrite")),
        "{denied}"
    );
    let continued = client.request("continue", json!({ "threadId": 1 }));
    assert_eq!(continued["success"], true, "{continued}");
    let (_stack, function_scopes) = stop_at_line(&mut client, &project, FUNCTION_LINE);
    let doubled = read_local(&mut client, &function_scopes, "DOUBLED");
    assert!(
        doubled.parse::<i64>().is_ok(),
        "DOUBLED is a decimal integer: {doubled}"
    );

    let disconnect = client.request("disconnect", json!({ "terminateDebuggee": true }));
    assert_eq!(disconnect["success"], true, "{disconnect}");
    assert!(client.wait_exit(PROTOCOL_TIMEOUT).is_some());
    // Attach never owns the host: it is still running after the adapter exits.
    assert!(
        host.try_wait().ok().flatten().is_none(),
        "attach leaves the host running"
    );
    let _ = host.kill();
    let _ = host.wait();
}
