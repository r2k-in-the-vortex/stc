//! M6.5 phase-8 debug CLI tests: launch, attach, batch, interactive commands,
//! initial breakpoints/watches, endpoint/token handling, release monitoring,
//! `stc run --debug-server`, swap/timeout/protocol-error teardown, and exit
//! codes (CLI-and-Manifest §1.7; Debug and Monitoring §13–§18).

mod common;

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use stc_got::json::Json;
use stc_runtime::{
    Candidate, DebugClient, DebugEndpointFile, DebugServerConfig, Host, LoadMode, LoadOutcome,
    ManifestStore, TransportKind,
};

fn stc() -> &'static str {
    env!("CARGO_BIN_EXE_stc")
}

/// Waits that include a project build and host launch. A loaded Windows CI
/// runner can be several times slower than a local machine, so these waits
/// need a larger budget than protocol-only waits.
const BUILD_AND_LAUNCH_TIMEOUT: Duration = Duration::from_secs(600);

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
                    eprintln!("note: clang not found; skipping compile-backed debug CLI checks");
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
            "stc-debug-cli-{}-{name}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test directory");
        Self { path }
    }

    fn path(&self, relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(self.path.clone(), |path, part| path.join(part))
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.path(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create directory");
        }
        std::fs::write(&path, contents).expect("write file");
        path
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(stc())
            .args(args)
            .current_dir(&self.path)
            .output()
            .expect("run stc")
    }

    fn run_with_runtime(&self, args: &[&str]) -> Output {
        let runtime = runtime_artifact();
        let mut all = args.to_vec();
        let runtime_text = runtime.to_string_lossy().into_owned();
        all.push("--runtime");
        all.push(&runtime_text);
        self.run(&all)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn combined(output: &Output) -> String {
    format!("{}\n{}", stdout(output), stderr(output))
}

fn manifest(name: &str, extra: &str) -> String {
    format!(
        "schema = 1\n\
         \n\
         [package]\n\
         name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         language = \"stc-2013-oop-utf8-v1\"\n\
         \n\
         [project]\n\
         sources = [\"src/**/*.st\"]\n\
         {extra}"
    )
}

const SOURCE: &str = "\
VAR_GLOBAL
  total : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  total := total + ticks;
END_PROGRAM
";

const SOURCE_V2: &str = "\
VAR_GLOBAL
  total : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 2;
  total := total + ticks;
END_PROGRAM
";

fn scaffold(name: &str) -> TestDir {
    let project = TestDir::new(name);
    project.write("stc.toml", &manifest("app", ""));
    project.write("src/main.st", SOURCE);
    project
}

fn json_lines(text: &str) -> Vec<Value> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("invalid JSON line `{line}`: {error}"))
        })
        .collect()
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    &value[key]
}

fn response_with<'a>(lines: &'a [Value], key: &str) -> &'a Value {
    lines
        .iter()
        .find(|line| {
            line["kind"] == "response"
                && line["status"] == "ok"
                && line
                    .get("result")
                    .and_then(|result| result.get(key))
                    .is_some()
        })
        .unwrap_or_else(|| panic!("no successful response with `{key}` in {lines:#?}"))
}

fn json_field<'a>(value: &'a Json, key: &str) -> Option<&'a Json> {
    value.as_object().and_then(|object| object.get(key))
}

fn has_code(lines: &[Value], code: &str) -> bool {
    lines.iter().any(|line| line.to_string().contains(code))
}

fn wait_file_contains(path: &Path, needle: &str, deadline: Duration) -> bool {
    let limit = Instant::now() + deadline;
    while Instant::now() < limit {
        if let Ok(text) = std::fs::read_to_string(path)
            && text.contains(needle)
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn endpoint_token(path: &Path) -> String {
    DebugEndpointFile::read(path)
        .expect("read endpoint file")
        .token
}

fn wait_endpoint(path: &Path, child: &mut Child, deadline: Duration) -> bool {
    let limit = Instant::now() + deadline;
    while Instant::now() < limit {
        if path.is_file() {
            return true;
        }
        if let Ok(Some(_)) = child.try_wait() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn text_file(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn wait_exit(child: &mut Child, deadline: Duration) -> ExitStatus {
    let limit = Instant::now() + deadline;
    loop {
        if let Some(status) = child.try_wait().expect("wait for child") {
            return status;
        }
        assert!(Instant::now() < limit, "child did not exit in {deadline:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ---------------------------------------------------------------------------
// Option parsing, help, and usage errors
// ---------------------------------------------------------------------------

#[test]
fn debug_help_usage_and_option_incompatibilities() {
    let project = scaffold("usage");
    let help = project.run(&["debug", "--help"]);
    assert_eq!(help.status.code(), Some(0), "{}", stderr(&help));
    let text = combined(&help);
    assert!(
        text.contains("stc debug [--attach <endpoint-file>]"),
        "{text}"
    );
    assert!(text.contains("stc run --debug-server"), "{text}");
    assert!(
        text.contains("not a safety PLC"),
        "the non-safety stance is explicit in `--help`: {text}"
    );
    assert!(text.contains("never prints the session token"), "{text}");

    let cases: &[(&[&str], &str)] = &[
        (
            &["debug", "--attach", "ep.json", "--release"],
            "cannot be combined with build, launch, or server-grant options",
        ),
        (
            &["debug", "--attach", "ep.json", "--allow-write"],
            "cannot be combined with build, launch, or server-grant options",
        ),
        (
            &["debug", "--attach", "ep.json", "--out-dir", "out"],
            "cannot be combined with build, launch, or server-grant options",
        ),
        (&["debug", "--debug-server"], "only valid with `stc run`"),
        (
            &["debug", "--message-format", "short"],
            "supports `--message-format human` or `json`",
        ),
        (
            &["debug", "--break", "src/main.st"],
            "expects `<file>:<line>[:<column>]`",
        ),
        (
            &["run", "--debug-endpoint", "ep.json"],
            "require `stc run --debug-server`",
        ),
        (
            &["run", "--allow-force"],
            "require `stc run --debug-server`",
        ),
        (
            &["run", "--session-timeout", "1s"],
            "require `stc run --debug-server`",
        ),
        (
            &["build", "--debug-server"],
            "only valid with `stc run --debug-server`",
        ),
        (
            &["build", "--break", "src/main.st:3"],
            "only valid with `stc debug`",
        ),
        (&["test", "--debug-server"], "STC-E-PROJECT-010"),
        (&["test", "--batch", "b.json"], "STC-E-PROJECT-010"),
        (
            &["check", "--debug-protocol", "socket"],
            "only valid with `stc run --debug-server`",
        ),
    ];
    for (args, needle) in cases {
        let output = project.run(args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            combined(&output)
        );
        assert!(
            combined(&output).contains(needle),
            "{args:?} did not mention `{needle}`:\n{}",
            combined(&output)
        );
    }
}

// ---------------------------------------------------------------------------
// Fake servers: transport failure and protocol-error teardown
// ---------------------------------------------------------------------------

fn write_fake_endpoint(path: &Path, address: &str, token: &str, audit: &Path) {
    let build = format!("0x{}", "00".repeat(32));
    let audit = audit.to_string_lossy().replace('\\', "/");
    let text = format!(
        "{{\"audit_log\":\"{audit}\",\"build_id\":\"{build}\",\"endpoint\":\"{address}\",\"format\":1,\
         \"generation\":1,\"mode\":\"debug\",\"protocol\":\"stc-debug/1\",\"token\":\"{token}\",\
         \"transport\":\"socket\"}}"
    );
    std::fs::write(path, text).expect("write fake endpoint");
}

fn accept_with_deadline(listener: &TcpListener) -> std::net::TcpStream {
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_nonblocking(false)
                    .expect("blocking client stream");
                stream
                    .set_read_timeout(Some(Duration::from_secs(20)))
                    .expect("client read timeout");
                return stream;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "no client connected to the fake debug endpoint"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("fake accept: {error}"),
        }
    }
}

fn start_fake_server(listener: TcpListener) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut stream = accept_with_deadline(&listener);
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("read request");
        let request: Value =
            serde_json::from_str(line.trim()).unwrap_or_else(|_| panic!("request line `{line}`"));
        let id = request["id"].as_u64().expect("request id");
        let response = serde_json::json!({
            "kind": "response",
            "protocol": "stc-debug/1",
            "id": id,
            "status": "ok",
            "result": {
                "session": "fake-session",
                "mode": "debug",
                "generation": 1,
                "build_id": format!("0x{}", "00".repeat(32)),
                "cycle": 0,
                "capabilities": {
                    "scopes": ["observe", "control"],
                    "hooks": true,
                    "debug_info": "none",
                    "checkpoints": true,
                    "stepping": true,
                    "max_watch_points": 64,
                    "max_breakpoints": 1024
                }
            },
            "warnings": []
        });
        writeln!(stream, "{response}").expect("write response");
        stream.flush().expect("flush response");
        // End the session in reply to the first command. Sending the event
        // only when the client is waiting on a request makes the teardown
        // deterministic under parallel test load instead of depending on
        // idle polling timing.
        line.clear();
        reader.read_line(&mut line).expect("read command");
        let event = serde_json::json!({
            "kind": "event",
            "protocol": "stc-debug/1",
            "session": "fake-session",
            "seq": 1,
            "event": "session-ended",
            "data": {"reason": "protocol-error", "code": "STC-E-DEBUG-022"},
            "warnings": []
        });
        let _ = writeln!(stream, "{event}");
        let _ = stream.flush();
        // Wait for the client to close first so a reset cannot discard the
        // event before the client reads it.
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut sink = [0_u8; 64];
        while matches!(stream.read(&mut sink), Ok(count) if count > 0) {}
    })
}

fn spawn_attach(
    endpoint: &Path,
    project: &TestDir,
    name: &str,
) -> (Child, std::process::ChildStdin) {
    let out = File::create(project.path(&format!("{name}.stdout"))).expect("stdout file");
    let err = File::create(project.path(&format!("{name}.stderr"))).expect("stderr file");
    let endpoint_text = endpoint.to_string_lossy().into_owned();
    let mut child = Command::new(stc())
        .args(["debug", "--attach", &endpoint_text])
        .current_dir(&project.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
        .expect("spawn attach");
    let stdin = child.stdin.take().expect("piped stdin");
    (child, stdin)
}

#[test]
fn transport_close_reports_e020_and_never_prints_the_token() {
    let project = TestDir::new("attach-close");
    let token = "b".repeat(64);
    let audit = project.path("audit.jsonl");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake socket");
    let address = listener.local_addr().expect("local address").to_string();
    let server = std::thread::spawn(move || {
        let stream = accept_with_deadline(&listener);
        drop(stream);
    });
    let endpoint = project.path("fake.endpoint.json");
    write_fake_endpoint(&endpoint, &address, &token, &audit);

    let output = Command::new(stc())
        .args(["debug", "--attach", endpoint.to_str().unwrap()])
        .current_dir(&project.path)
        .stdin(Stdio::null())
        .output()
        .expect("run attach");
    server.join().expect("fake server");
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(text.contains("STC-E-DEBUG-020"), "{text}");
    assert!(
        !text.contains(&token),
        "the token must never be printed: {text}"
    );
}

#[test]
fn protocol_error_teardown_reports_e022_without_replaying() {
    let project = TestDir::new("attach-protocol");
    let token = "c".repeat(64);
    let audit = project.path("audit.jsonl");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake socket");
    let address = listener.local_addr().expect("local address").to_string();
    let endpoint = project.path("fake.endpoint.json");
    write_fake_endpoint(&endpoint, &address, &token, &audit);
    let server = start_fake_server(listener);

    let (mut child, mut stdin) = spawn_attach(&endpoint, &project, "protocol");
    stdin
        .write_all(b"watch MAIN.TICKS 100\n")
        .expect("write command");
    stdin.flush().expect("flush command");
    let status = wait_exit(&mut child, Duration::from_secs(20));
    server.join().expect("fake server");
    let text = format!(
        "{}\n{}",
        text_file(&project.path("protocol.stdout")),
        text_file(&project.path("protocol.stderr"))
    );
    assert_eq!(status.code(), Some(2), "{text}");
    assert!(text.contains("STC-E-DEBUG-022"), "{text}");
    assert!(text.contains("protocol-error"), "{text}");
    assert!(
        !text.contains(&token),
        "the token must never be printed: {text}"
    );
}

fn scripted_view() -> Json {
    use stc_got::digest::GotId;
    use stc_runtime::{DebugValue, SplitView, ViewKind, ViewNode, ViewOrigin, view_json};
    let view = SplitView {
        target: "MAIN.POINT".into(),
        resolved_key: "MAIN.POINT".into(),
        generation: 1,
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
    view_json(&view)
}

fn response_line(id: u64, result: Json) -> String {
    Json::object([
        ("kind".into(), Json::string("response")),
        ("protocol".into(), Json::string("stc-debug/1")),
        ("id".into(), Json::unsigned(id)),
        ("status".into(), Json::string("ok")),
        ("result".into(), result),
        ("warnings".into(), Json::Array(Vec::new())),
    ])
    .render_compact()
}

fn scripted_server(listener: TcpListener) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut stream = accept_with_deadline(&listener);
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        loop {
            let mut line = String::new();
            let read = reader.read_line(&mut line).expect("read request");
            if read == 0 {
                return;
            }
            let request: Value = serde_json::from_str(line.trim())
                .unwrap_or_else(|_| panic!("request line `{line}`"));
            let id = request["id"].as_u64().expect("request id");
            let op = request["op"].as_str().expect("request op");
            let result = match op {
                "session.start" => Json::object([
                    ("session".into(), Json::string("scripted-session")),
                    ("mode".into(), Json::string("debug")),
                    ("generation".into(), Json::unsigned(1)),
                    (
                        "build_id".into(),
                        Json::string(format!("0x{}", "00".repeat(32))),
                    ),
                    ("cycle".into(), Json::unsigned(0)),
                    (
                        "capabilities".into(),
                        Json::object([
                            (
                                "scopes".into(),
                                Json::Array(vec![Json::string("observe"), Json::string("control")]),
                            ),
                            ("hooks".into(), Json::Bool(true)),
                            ("debug_info".into(), Json::string("none")),
                            ("checkpoints".into(), Json::Bool(true)),
                            ("stepping".into(), Json::Bool(true)),
                            ("max_watch_points".into(), Json::unsigned(64)),
                            ("max_breakpoints".into(), Json::unsigned(1024)),
                        ]),
                    ),
                ]),
                "var.read" => {
                    let key = request["params"]["target"]["key"]
                        .as_str()
                        .unwrap_or_default();
                    if key == "MAIN.POINT" {
                        Json::object([
                            ("available".into(), Json::Bool(true)),
                            (
                                "value".into(),
                                Json::object([
                                    ("type".into(), Json::string("DINT")),
                                    ("value".into(), Json::string("7")),
                                ]),
                            ),
                            ("view".into(), scripted_view()),
                        ])
                    } else {
                        Json::object([
                            ("available".into(), Json::Bool(true)),
                            (
                                "value".into(),
                                Json::object([
                                    ("type".into(), Json::string("DINT")),
                                    ("value".into(), Json::string("7")),
                                ]),
                            ),
                        ])
                    }
                }
                "frame.read" => Json::object([
                    ("available".into(), Json::Bool(true)),
                    (
                        "value".into(),
                        Json::object([
                            ("type".into(), Json::string("DINT")),
                            ("value".into(), Json::string("7")),
                        ]),
                    ),
                    ("view".into(), scripted_view()),
                ]),
                _ => Json::object(Vec::new()),
            };
            writeln!(stream, "{}", response_line(id, result)).expect("write response");
            stream.flush().expect("flush response");
            if op == "session.stop" {
                // The protocol closes the connection after the stop event.
                let event = Json::object([
                    ("kind".into(), Json::string("event")),
                    ("protocol".into(), Json::string("stc-debug/1")),
                    ("session".into(), Json::string("scripted-session")),
                    ("seq".into(), Json::unsigned(1)),
                    ("event".into(), Json::string("session-ended")),
                    (
                        "data".into(),
                        Json::object([("reason".into(), Json::string("client"))]),
                    ),
                    ("warnings".into(), Json::Array(Vec::new())),
                ]);
                let _ = writeln!(stream, "{}", event.render_compact());
                let _ = stream.flush();
                return;
            }
        }
    })
}

#[test]
fn print_and_frame_render_the_optional_view_and_skip_it_for_sliced_reads() {
    let project = TestDir::new("attach-view");
    let token = "d".repeat(64);
    let audit = project.path("audit.jsonl");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake socket");
    let address = listener.local_addr().expect("local address").to_string();
    let endpoint = project.path("fake.endpoint.json");
    write_fake_endpoint(&endpoint, &address, &token, &audit);
    let server = scripted_server(listener);

    let out = File::create(project.path("view.stdout")).expect("stdout file");
    let err = File::create(project.path("view.stderr")).expect("stderr file");
    let mut child = Command::new(stc())
        .args(["debug", "--attach", endpoint.to_str().unwrap()])
        .current_dir(&project.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
        .expect("spawn attach");
    let mut stdin = child.stdin.take().expect("piped stdin");
    writeln!(stdin, "print MAIN.POINT").expect("print view target");
    writeln!(stdin, "print MAIN.SCALAR").expect("print sliced target");
    writeln!(stdin, "frame 0 point 1").expect("read frame view");
    writeln!(stdin, "quit").expect("quit");
    drop(stdin);
    let status = wait_exit(&mut child, Duration::from_secs(30));
    server.join().expect("scripted server");
    let stdout = text_file(&project.path("view.stdout"));
    let stderr = text_file(&project.path("view.stderr"));
    let text = format!("{stdout}\n{stderr}");
    assert_eq!(status.code(), Some(0), "{text}");
    assert_eq!(
        stdout.matches("contiguous single-root object view").count(),
        2,
        "print and frame both render the optional view: {text}"
    );
    assert_eq!(stdout.matches("target: MAIN.POINT").count(), 2, "{text}");
    assert!(
        stderr.contains("stc: MAIN.SCALAR = "),
        "a sliced read has no `view` and renders the typed value: {text}"
    );
    assert!(!text.contains("view: MAIN.SCALAR"), "{text}");
    assert!(
        !text.contains(&token),
        "the token must never be printed: {text}"
    );
    assert!(stderr.contains("not a safety PLC"), "{text}");
}

// ---------------------------------------------------------------------------
// Launch: batch, initial breakpoints/watches, endpoint and token output
// ---------------------------------------------------------------------------

#[test]
fn launch_batch_applies_initial_breakpoints_and_watches() {
    let Some(_clang) = clang() else {
        return;
    };
    let project = scaffold("batch");
    project.write(
        "requests.jsonl",
        "{\"op\":\"breakpoint.list\",\"params\":{}}\n\
         {\"op\":\"watch.list\",\"params\":{}}\n",
    );
    let output = project.run_with_runtime(&[
        "debug",
        "--batch",
        "requests.jsonl",
        "--break",
        "src/main.st:9",
        "--watch",
        "MAIN.TICKS",
        "--cycles",
        "2000",
        "--cycle-time",
        "10ms",
    ]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    let lines = json_lines(&stdout(&output));
    let breakpoints = response_with(&lines, "breakpoints");
    assert_eq!(
        field(breakpoints, "result")["breakpoints"]
            .as_array()
            .expect("breakpoint array")
            .len(),
        1,
        "{breakpoints}"
    );
    let watches = response_with(&lines, "watches");
    assert_eq!(
        field(watches, "result")["watches"]
            .as_array()
            .expect("watch array")
            .len(),
        1,
        "{watches}"
    );
    assert!(
        stderr(&output).contains("debug endpoint"),
        "launch mode prints the endpoint path on stderr:\n{}",
        stderr(&output)
    );
    assert!(stderr(&output).contains("not a safety PLC"), "{text}");
}

#[test]
fn launch_interactive_reports_session_status_and_never_prints_the_token() {
    let Some(_clang) = clang() else {
        return;
    };
    let project = scaffold("interactive");
    let out = File::create(project.path("interactive.stdout")).expect("stdout file");
    let err = File::create(project.path("interactive.stderr")).expect("stderr file");
    let runtime = runtime_artifact();
    let runtime_text = runtime.to_string_lossy().into_owned();
    // The host and the CLI both print the endpoint to the inherited stderr,
    // and the format machinery writes each line in several pieces. Locate the
    // endpoint file directly instead of parsing the interleaved stream.
    let endpoint = project.path("interactive.endpoint.json");
    let endpoint_text = endpoint.to_string_lossy().into_owned();
    let mut child = Command::new(stc())
        .args([
            "debug",
            "--endpoint",
            &endpoint_text,
            "--message-format",
            "json",
            "--cycles",
            "2000",
            "--cycle-time",
            "10ms",
            "--runtime",
            &runtime_text,
        ])
        .current_dir(&project.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
        .expect("spawn debug");
    let mut stdin = child.stdin.take().expect("piped stdin");

    let stderr_path = project.path("interactive.stderr");
    let stdout_path = project.path("interactive.stdout");
    assert!(
        wait_file_contains(
            &stdout_path,
            "\"kind\":\"response\"",
            BUILD_AND_LAUNCH_TIMEOUT
        ),
        "no debug session started:\n{}\n{}",
        text_file(&stderr_path),
        text_file(&stdout_path)
    );
    // The endpoint line is written to stderr during launch; wait for it rather
    // than racing the first response against the stderr flush.
    assert!(
        wait_file_contains(
            &stderr_path,
            "stc: debug endpoint ",
            Duration::from_secs(30)
        ),
        "the endpoint path was not printed on stderr:\n{}",
        text_file(&stderr_path)
    );
    let endpoint_name = endpoint
        .file_name()
        .expect("endpoint file name")
        .to_string_lossy()
        .into_owned();
    assert!(
        wait_file_contains(&stderr_path, &endpoint_name, Duration::from_secs(30)),
        "the endpoint path is printed on stderr:\n{}",
        text_file(&stderr_path)
    );
    let started = text_file(&stderr_path);
    let token = endpoint_token(&endpoint);
    assert_eq!(token.len(), 64, "the endpoint token is 64 hex characters");
    assert!(
        !started.contains(&token),
        "the token is never printed at session start"
    );

    writeln!(stdin, "watch MAIN.TICKS 100").expect("write watch");
    writeln!(stdin, "quit").expect("write quit");
    drop(stdin);
    let status = wait_exit(&mut child, Duration::from_secs(30));
    let text = format!(
        "{}\n{}",
        text_file(&project.path("interactive.stdout")),
        text_file(&project.path("interactive.stderr"))
    );
    assert_eq!(status.code(), Some(0), "{text}");
    assert!(
        !text.contains(&token),
        "the token must never be printed: {text}"
    );
    assert!(text.contains("not a safety PLC"), "{text}");
    assert!(text.contains("debug endpoint"), "{text}");
    let lines = json_lines(&text_file(&project.path("interactive.stdout")));
    assert!(
        lines.iter().any(|line| line["kind"] == "response"),
        "interactive JSON output is one message per line"
    );
}

#[test]
fn launch_reports_the_target_trap_exit_code() {
    let Some(_clang) = clang() else {
        return;
    };
    let project = TestDir::new("trap");
    project.write("stc.toml", &manifest("app", ""));
    project.write(
        "src/main.st",
        "PROGRAM Main\n\
         VAR\n\
         \x20 counter : DINT;\n\
         \x20 x : DINT;\n\
         END_VAR\n\
         \x20 counter := counter + 1;\n\
         \x20 IF counter >= 200 THEN\n\
         \x20   x := 1 / 0;\n\
         \x20 END_IF;\n\
         END_PROGRAM\n",
    );
    let out = File::create(project.path("trap.stdout")).expect("stdout file");
    let err = File::create(project.path("trap.stderr")).expect("stderr file");
    let runtime = runtime_artifact();
    let runtime_text = runtime.to_string_lossy().into_owned();
    let mut child = Command::new(stc())
        .args([
            "debug",
            "--cycles",
            "1000",
            "--cycle-time",
            "20ms",
            "--runtime",
            &runtime_text,
        ])
        .current_dir(&project.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
        .expect("spawn debug");
    // Keep stdin open so the session does not end on EOF before the trap.
    let stdin = child.stdin.take().expect("piped stdin");
    let status = wait_exit(&mut child, BUILD_AND_LAUNCH_TIMEOUT);
    drop(stdin);
    let text = format!(
        "{}\n{}",
        text_file(&project.path("trap.stdout")),
        text_file(&project.path("trap.stderr"))
    );
    assert_eq!(status.code(), Some(4), "{text}");
    assert!(text.contains("DIV_ZERO"), "{text}");
    assert!(
        text.contains("target-exited") || text.contains("debug session"),
        "{text}"
    );
}

// ---------------------------------------------------------------------------
// `stc run --debug-server`
// ---------------------------------------------------------------------------

#[test]
fn run_debug_server_accepts_attach_and_release_is_monitoring_only() {
    let Some(_clang) = clang() else {
        return;
    };
    let project = scaffold("run-server");
    let endpoint = project.path("run-debug.endpoint.json");
    let out = File::create(project.path("run.stdout")).expect("stdout file");
    let err = File::create(project.path("run.stderr")).expect("stderr file");
    let runtime = runtime_artifact();
    let runtime_text = runtime.to_string_lossy().into_owned();
    let mut run = Command::new(stc())
        .args([
            "run",
            "--debug-server",
            "--debug-endpoint",
            endpoint.to_str().unwrap(),
            "--release",
            "--cycles",
            "200",
            "--cycle-time",
            "50ms",
            "--runtime",
            &runtime_text,
        ])
        .current_dir(&project.path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
        .expect("spawn run");
    assert!(
        wait_endpoint(&endpoint, &mut run, BUILD_AND_LAUNCH_TIMEOUT),
        "no debug endpoint; run output:\n{}\n{}",
        text_file(&project.path("run.stdout")),
        text_file(&project.path("run.stderr"))
    );
    let token = endpoint_token(&endpoint);
    assert_eq!(token.len(), 64);

    project.write(
        "attach.jsonl",
        "{\"op\":\"session.status\",\"params\":{}}\n\
         {\"op\":\"breakpoint.set\",\"params\":{\"target\":{\"kind\":\"source\",\"file\":\"src/main.st\",\"line\":9}}}\n\
         {\"op\":\"watch.list\",\"params\":{}}\n",
    );
    let attach = project.run(&[
        "debug",
        "--attach",
        endpoint.to_str().unwrap(),
        "--batch",
        "attach.jsonl",
    ]);
    let attach_text = combined(&attach);
    assert_eq!(attach.status.code(), Some(0), "{attach_text}");
    let lines = json_lines(&stdout(&attach));
    assert!(
        has_code(&lines, "STC-W-DEBUG-004"),
        "release yields a monitoring-only session: {attach_text}"
    );
    assert!(
        has_code(&lines, "STC-E-DEBUG-014"),
        "breakpoints are unavailable in a monitoring-only session: {attach_text}"
    );
    assert!(!attach_text.contains(&token), "{attach_text}");

    let status = wait_exit(&mut run, Duration::from_secs(60));
    let run_text = format!(
        "{}\n{}",
        text_file(&project.path("run.stdout")),
        text_file(&project.path("run.stderr"))
    );
    assert_eq!(status.code(), Some(0), "{run_text}");
    assert!(
        run_text.contains("debug endpoint"),
        "the endpoint path is printed on stderr: {run_text}"
    );
    assert!(
        !run_text.contains(&token),
        "`stc run --debug-server` never prints the token: {run_text}"
    );
    assert!(
        !endpoint.exists(),
        "the host removes its endpoint file on shutdown"
    );
}

// ---------------------------------------------------------------------------
// Real-host teardown: idle timeout and swap
// ---------------------------------------------------------------------------

fn build_candidate(project: &TestDir, out: &str, baseline: Option<&Path>) -> (PathBuf, PathBuf) {
    let mut args = vec![
        "build".to_owned(),
        "--emit".to_owned(),
        "module".to_owned(),
        "--out-dir".to_owned(),
        out.to_owned(),
    ];
    if let Some(baseline) = baseline {
        args.push("--got-manifest".to_owned());
        args.push(baseline.to_string_lossy().into_owned());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = project.run(&refs);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let metadata_path = project.path(&format!("{out}/app.stc-meta.json"));
    let metadata: Value =
        serde_json::from_str(&std::fs::read_to_string(&metadata_path).expect("read metadata"))
            .expect("parse metadata");
    let module = project.path(&format!(
        "{out}/{}",
        metadata["module"]["file"].as_str().expect("module file")
    ));
    let manifest = project.path(&format!("{out}/app.stc-got.bin"));
    (module, manifest)
}

fn open_candidate(module: &Path, manifest: &Path) -> Candidate {
    Candidate::open(&stc_runtime::host_platform(), module, manifest)
        .unwrap_or_else(|error| panic!("candidate: {error}"))
}

fn install_cold(host: &mut Host, module: &Path, manifest: &Path) -> u64 {
    match host
        .load(open_candidate(module, manifest), LoadMode::Auto)
        .expect("cold install")
    {
        LoadOutcome::ColdInstalled { generation } => generation,
        other => panic!("expected a cold install, got {other:?}"),
    }
}

fn start_server(host: &mut Host, endpoint: &Path, timeout: Duration) {
    let mut config = DebugServerConfig::new(endpoint.to_path_buf(), TransportKind::Socket);
    config.session_timeout = timeout;
    host.start_debug_server(&config).expect("debug server");
}

fn pump_until_finished<T>(
    host: &mut Host,
    handle: &std::thread::ScopedJoinHandle<'_, T>,
    what: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !handle.is_finished() {
        assert!(Instant::now() < deadline, "`{what}` did not finish");
        host.debug_pump_at(0);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn pumped<T: Send>(host: &mut Host, call: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        let handle = scope.spawn(call);
        pump_until_finished(host, &handle, "debug call");
        handle.join().expect("debug call thread")
    })
}

fn pump_until_exit(host: &mut Host, child: &mut Child, deadline: Duration) -> ExitStatus {
    let limit = Instant::now() + deadline;
    loop {
        host.debug_pump_at(0);
        if let Some(status) = child.try_wait().expect("wait for attach") {
            return status;
        }
        assert!(Instant::now() < limit, "the attach client did not exit");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn pump_until_text(
    host: &mut Host,
    child: &mut Child,
    path: &Path,
    needle: &str,
    deadline: Duration,
) {
    let limit = Instant::now() + deadline;
    loop {
        host.debug_pump_at(0);
        if text_file(path).contains(needle) {
            return;
        }
        assert!(
            child.try_wait().expect("wait for attach").is_none(),
            "the attach client exited early:\n{}",
            text_file(path)
        );
        assert!(
            Instant::now() < limit,
            "`{needle}` did not appear:\n{}",
            text_file(path)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn audit_session_starts(endpoint: &Path) -> usize {
    let endpoint = DebugEndpointFile::read(endpoint).expect("endpoint file");
    std::fs::read_to_string(&endpoint.audit_log)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains("\"session-start\""))
        .count()
}

/// The runtime keeps the stop-session registry in a process-global static,
/// so tests that run an in-process host must not overlap.
static HOST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn attach_idle_timeout_reports_e018_and_ends_the_session() {
    let Some(_clang) = clang() else {
        return;
    };
    let _host = HOST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let project = scaffold("idle");
    let (module, manifest_path) = build_candidate(&project, "out", None);
    let mut host = Host::new(stc_runtime::host_platform(), ManifestStore::memory());
    install_cold(&mut host, &module, &manifest_path);
    let endpoint = project.path("idle.endpoint.json");
    start_server(&mut host, &endpoint, Duration::from_millis(300));

    let (mut child, _stdin) = spawn_attach(&endpoint, &project, "idle");
    let status = pump_until_exit(&mut host, &mut child, Duration::from_secs(30));
    let text = format!(
        "{}\n{}",
        text_file(&project.path("idle.stdout")),
        text_file(&project.path("idle.stderr"))
    );
    assert_eq!(status.code(), Some(2), "{text}");
    assert!(text.contains("STC-E-DEBUG-018"), "{text}");
    assert!(text.contains("idle-timeout"), "{text}");
    assert_eq!(audit_session_starts(&endpoint), 1, "{text}");

    host.stop_debug_server();
}

#[test]
fn swap_teardown_reports_e019_and_the_host_recovers_without_replay() {
    let Some(_clang) = clang() else {
        return;
    };
    let _host = HOST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let project = scaffold("swap");
    let (module_v1, manifest_v1) = build_candidate(&project, "out1", None);
    let baseline = project.path("baseline.json");
    std::fs::copy(project.path("out1/app.stc-got.json"), &baseline).expect("copy baseline");
    project.write("src/main.st", SOURCE_V2);
    let (module_v2, manifest_v2) = build_candidate(&project, "out2", Some(&baseline));

    let mut host = Host::new(stc_runtime::host_platform(), ManifestStore::memory());
    let old_generation = install_cold(&mut host, &module_v1, &manifest_v1);
    let endpoint = project.path("swap.endpoint.json");
    start_server(&mut host, &endpoint, Duration::ZERO);

    let (mut child, _stdin) = spawn_attach(&endpoint, &project, "swap");
    let stderr_path = project.path("swap.stderr");
    pump_until_text(
        &mut host,
        &mut child,
        &stderr_path,
        "stc: debug session ",
        Duration::from_secs(20),
    );
    assert_eq!(audit_session_starts(&endpoint), 1);
    let token = endpoint_token(&endpoint);

    match host
        .load(open_candidate(&module_v2, &manifest_v2), LoadMode::Online)
        .expect("online load")
    {
        LoadOutcome::Prepared { .. } => {}
        other => panic!("expected an online change, got {other:?}"),
    }
    let generation = host.activate().expect("activate");
    assert!(generation > old_generation);
    let status = pump_until_exit(&mut host, &mut child, Duration::from_secs(30));
    let text = format!(
        "{}\n{}",
        text_file(&project.path("swap.stdout")),
        text_file(&project.path("swap.stderr"))
    );
    assert_eq!(status.code(), Some(2), "{text}");
    assert!(text.contains("STC-E-DEBUG-019"), "{text}");
    assert!(text.contains("reason swap"), "{text}");
    assert!(
        !text.contains(&token),
        "the token must never be printed: {text}"
    );
    assert_eq!(
        audit_session_starts(&endpoint),
        1,
        "the CLI must not reconnect or replay after a swap: {text}"
    );

    // The host remains healthy: it cycles, serves a fresh session on the new
    // generation, and the old session state is gone.
    host.cycle_at(1_000_000)
        .expect("the host cycles after the swap");
    assert!(host.debug_server_info().is_some());
    assert_eq!(
        host.debug_server_info().expect("server").generation,
        generation
    );
    let mut client = DebugClient::connect_file(&endpoint).expect("reconnect");
    let mut started = None;
    for _ in 0..20 {
        match pumped(&mut host, || client.start_session(&["observe", "control"])) {
            Ok(result) => {
                started = Some(result);
                break;
            }
            Err(error) => {
                assert_eq!(
                    error.code.as_deref(),
                    Some("STC-E-DEBUG-007"),
                    "unexpected reconnect failure: {error}"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    let started = started.expect("a new session on the new generation");
    assert_eq!(
        json_field(&started, "generation").and_then(Json::as_number),
        Some(i128::from(generation))
    );
    let listed = pumped(&mut host, || {
        client.call_ok("breakpoint.list", Json::object(Vec::new()))
    })
    .expect("breakpoint.list after swap");
    assert_eq!(
        json_field(&listed, "breakpoints")
            .and_then(Json::as_array)
            .map(<[Json]>::len),
        Some(0),
        "session state is not replayed: {listed:?}"
    );
    let _ = pumped(&mut host, || {
        client.call_ok("session.stop", Json::object(Vec::new()))
    });
    host.stop_debug_server();
}
