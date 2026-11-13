//! `stc lsp` integration tests (WL-1.1 phase 3).
//!
//! Each test spawns the real binary, speaks `Content-Length` framed JSON-RPC
//! over stdio, and reads responses and `textDocument/publishDiagnostics`
//! notifications. The project is a temporary directory created by the test;
//! nothing under `examples/` is written.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(20);

fn stc() -> &'static str {
    env!("CARGO_BIN_EXE_stc")
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stc-lsp-cli-{}-{name}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create project directory");
        let path = std::fs::canonicalize(&path).expect("canonical project directory");
        Self { path }
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = relative
            .split('/')
            .fold(self.path.clone(), |path, part| path.join(part));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create source directory");
        }
        std::fs::write(&path, contents).expect("write project file");
        path
    }

    fn manifest(&self) -> PathBuf {
        self.path.join("stc.toml")
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
configuration = \"Demo\"

[build]
emit = [\"module\", \"metadata\"]
";

const MAIN_SOURCE: &str = "\
FUNCTION AddTwo : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  AddTwo := a + b;
END_FUNCTION

FUNCTION_BLOCK Counter
VAR_INPUT
  increment : DINT;
END_VAR
VAR_OUTPUT
  value : DINT;
END_VAR
VAR PUBLIC
  state : DINT;
END_VAR
  value := value + increment;
  state := value;
END_FUNCTION_BLOCK

PROGRAM Main
VAR
  counter : Counter;
  sum : DINT;
END_VAR
  counter(increment := 1, value => sum);
  sum := counter.state;
  (* café 😀 *) sum := AddTwo(sum, 1);
END_PROGRAM

CONFIGURATION Demo
RESOURCE Cpu ON Cpu
  TASK MainTask(INTERVAL := T#10ms, PRIORITY := 1);
  PROGRAM demo WITH MainTask : Main;
END_RESOURCE
END_CONFIGURATION
";

const SEMANTIC_ERROR_SOURCE: &str = "\
FUNCTION AddTwo : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  AddTwo := a + b;
END_FUNCTION

PROGRAM Main
VAR
  sum : DINT;
END_VAR
  sum := MissingName;
END_PROGRAM
";

const PARSE_ERROR_SOURCE: &str = "\
PROGRAM Main
VAR
  sum : DINT;
END_VAR
  sum := ;
END_PROGRAM
";

const MESSY_SOURCE: &str = "\
program Main
var
x:dint;
end_var
x:=1+2;
end_program
";

const EXTRA_SOURCE: &str = "\
FUNCTION Triple : DINT
VAR_INPUT
  x : DINT;
END_VAR
  Triple := x * 3;
END_FUNCTION
";

/// A project with tracked, parameter, and local rename targets.
const BASIC_SOURCE: &str = "\
PROGRAM Main
VAR
  Speed : DINT;
  text : STRING[32];
END_VAR
  Speed := Speed + 1;
  text := 'Speed';
END_PROGRAM
FUNCTION AddStep : DINT
VAR_INPUT
  value : DINT;
END_VAR
VAR
  local_value : DINT;
END_VAR
  AddStep := value + 1;
END_FUNCTION
";

/// A project whose rename targets appear in the configuration, bindings, a
/// test case, and the selected machine profile.
const EXTERNAL_SOURCE: &str = "\
VAR_GLOBAL
  Short AT %QX0.0 : BOOL;
  Ns AT %IX0.1 : BOOL;
  Pkg AT %IX0.2 : BOOL;
  Other AT %IX0.3 : BOOL;
END_VAR

FUNCTION AddStep : DINT
VAR_INPUT
  value : DINT;
END_VAR
VAR
  local_value : DINT;
END_VAR
  AddStep := value + 1;
END_FUNCTION

PROGRAM Main
VAR
  Field AT %QX0.1 : BOOL;
  Label AT %QX0.2 : BOOL;
END_VAR
  Field := TRUE;
  Label := FALSE;
END_PROGRAM

CONFIGURATION MainConfig
RESOURCE Cpu ON Cpu
  TASK MainTask(INTERVAL := T#10ms, PRIORITY := 1);
  PROGRAM main WITH MainTask : Main;
END_RESOURCE
END_CONFIGURATION
";

const EXTERNAL_PROFILE: &str = "\
[[routes]]
source = { kind = \"plc-output\", variable = \"Short\" }
sinks = [{ kind = \"plc-input\", variable = \"Other\" }]
";

/// A project with `VAR_ACCESS` and `VAR_CONFIG` path segments.
const ACCESS_SOURCE: &str = "\
TYPE Inner : STRUCT
  x : DINT;
END_STRUCT; END_TYPE
PROGRAM Main
VAR
  x : DINT;
  nested : Inner;
END_VAR
VAR_ACCESS
  visible : x : DINT READ_ONLY;
  nested_path : nested.x : DINT READ_ONLY;
END_VAR
  x := 1;
END_PROGRAM
CONFIGURATION C
VAR_GLOBAL
  seed : DINT;
END_VAR
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms);
  PROGRAM inst WITH t : Main;
END_RESOURCE
VAR_ACCESS
  seed_path : seed : DINT READ_WRITE;
  inst_x : inst.x : DINT READ_ONLY;
  inst_nested : inst.nested.x : DINT READ_ONLY;
END_VAR
VAR_CONFIG
  inst.x AT %MD11 : DINT := 5;
  inst.nested.x AT %MD10 : DINT := 5;
END_VAR
END_CONFIGURATION
";

fn project_manifest(name: &str, extra: &str) -> String {
    format!(
        "schema = 1\n\n[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\
         language = \"stc-2013-oop-utf8-v1\"\n\n[project]\nsources = [\"src/**/*.st\"]\n{extra}\n"
    )
}

fn external_manifest() -> String {
    project_manifest(
        "app",
        "\
configuration = \"MainConfig\"

[io]
config = \"io/bench.toml\"

[[io.devices]]
name = \"bench\"
kind = \"demo\"

[[io.bindings]]
variable = \"Short\"
device = \"bench\"
channel = 0

[[io.bindings]]
variable = \"Ns\"
device = \"bench\"
channel = 1

[[io.bindings]]
variable = \"app.Pkg\"
device = \"bench\"
channel = 2

[[io.bindings]]
variable = \"main.Field\"
device = \"bench\"
channel = 3

[test]
sources = [\"tests/**/*.st\"]

[[test.case]]
name = \"SampleTest\"",
    )
}

fn write_external_project(project: &TestDir) {
    project.write("stc.toml", &external_manifest());
    project.write("io/bench.toml", EXTERNAL_PROFILE);
    project.write("src/main.st", EXTERNAL_SOURCE);
    project.write(
        "tests/sample_test.st",
        "{stc:test}\nFUNCTION SampleTest : BOOL\nVAR\n  result : BOOL;\nEND_VAR\n  result := TRUE;\n  SampleTest := result;\nEND_FUNCTION\n",
    );
}

fn write_access_project(project: &TestDir) {
    project.write(
        "stc.toml",
        &project_manifest("app", "configuration = \"C\""),
    );
    project.write("src/main.st", ACCESS_SOURCE);
}

/// Returns a relative-path map of every file below `root`, sorted by path.
fn read_tree(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fn walk(
        root: &Path,
        directory: &Path,
        files: &mut std::collections::BTreeMap<String, Vec<u8>>,
    ) {
        for entry in std::fs::read_dir(directory).expect("read tree directory") {
            let entry = entry.expect("tree entry");
            let path = entry.path();
            if entry.file_type().expect("tree entry type").is_dir() {
                walk(root, &path, files);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .expect("tree path below root")
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(relative, std::fs::read(&path).expect("read tree file"));
            }
        }
    }
    let mut files = std::collections::BTreeMap::new();
    walk(root, root, &mut files);
    files
}

fn uri(path: &Path) -> String {
    stc_lsp::path_to_uri(path).expect("file URI").to_string()
}

fn position_at_offset(source: &str, offset: usize) -> Value {
    let line_start = source[..offset].rfind('\n').map_or(0, |index| index + 1);
    let line = source[..offset].matches('\n').count();
    let character = source[line_start..offset].encode_utf16().count();
    json!({ "line": line, "character": character })
}

/// Returns the UTF-16 LSP position of the first `needle` occurrence.
fn position_of(source: &str, needle: &str) -> Value {
    let offset = source
        .find(needle)
        .unwrap_or_else(|| panic!("missing {needle:?}"));
    position_at_offset(source, offset)
}

/// Returns the UTF-16 LSP position of `inner` inside the first `needle`.
fn position_within(source: &str, needle: &str, inner: &str) -> Value {
    let base = source
        .find(needle)
        .unwrap_or_else(|| panic!("missing {needle:?}"));
    let offset = needle
        .find(inner)
        .unwrap_or_else(|| panic!("missing {inner:?} in {needle:?}"));
    position_at_offset(source, base + offset)
}

fn range_between(start: Value, length: usize) -> Value {
    let mut end = start.clone();
    end["character"] = json!(start["character"].as_u64().expect("character") + length as u64);
    json!({ "start": start, "end": end })
}

fn range_of_within(source: &str, needle: &str, inner: &str) -> Value {
    range_between(
        position_within(source, needle, inner),
        inner.encode_utf16().count(),
    )
}

/// Returns the UTF-16 LSP position of `inner` inside the first `anchor`.
fn position_after(source: &str, anchor: &str, inner: &str) -> Value {
    let base = source
        .find(anchor)
        .unwrap_or_else(|| panic!("missing {anchor:?}"));
    let offset = anchor
        .find(inner)
        .unwrap_or_else(|| panic!("missing {inner:?} in {anchor:?}"));
    position_at_offset(source, base + offset)
}

/// Returns the LSP range of `inner` inside the first `anchor`.
fn range_after(source: &str, anchor: &str, inner: &str) -> Value {
    range_between(
        position_after(source, anchor, inner),
        inner.encode_utf16().count(),
    )
}

fn text_edit(range: Value, new_text: &str) -> Value {
    json!({ "range": range, "newText": new_text })
}

fn assert_text_edit(edits: &Value, range: Value, new_text: &str) {
    let expected = text_edit(range, new_text);
    let matches: Vec<&Value> = edits
        .as_array()
        .unwrap_or_else(|| panic!("expected an edit array, got {edits}"))
        .iter()
        .filter(|edit| **edit == expected)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "missing edit {expected} in {}",
        json!(edits)
    );
}

/// Applies an LSP `changes` object to the files on disk in descending order.
fn apply_changes(changes: &Value) {
    let changes = changes.as_object().expect("changes object");
    for (uri, edits) in changes {
        let path = stc_lsp::uri_to_path(uri).unwrap_or_else(|| panic!("edit URI {uri}"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let positions = stc_lsp::PositionMap::new(&text);
        let mut replacements: Vec<(usize, usize, String)> = edits
            .as_array()
            .expect("edit array")
            .iter()
            .map(|edit| {
                let start: stc_lsp::Position =
                    serde_json::from_value(edit["range"]["start"].clone()).expect("start position");
                let end: stc_lsp::Position =
                    serde_json::from_value(edit["range"]["end"].clone()).expect("end position");
                (
                    positions.offset(start),
                    positions.offset(end),
                    edit["newText"].as_str().expect("newText").to_owned(),
                )
            })
            .collect();
        replacements.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
        let mut updated = text;
        for (start, end, new_text) in replacements {
            assert!(start <= end, "reversed edit in {uri}");
            updated.replace_range(start..end, &new_text);
        }
        std::fs::write(&path, updated).expect("write applied edit");
    }
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

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn run_stc(directory: &Path, args: &[&str]) -> std::process::Output {
    Command::new(stc())
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run stc")
}

fn assert_success(output: &std::process::Output, context: &str) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{context}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Returns the checked-in sample workspace used by the IDE tests.
fn sample_workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../editors/vscode/samples/workspace")
}

struct LspClient {
    child: Option<Child>,
    stdin: ChildStdin,
    messages: Receiver<Value>,
    notifications: VecDeque<Value>,
    next_id: i64,
    stderr: Arc<Mutex<String>>,
}

impl LspClient {
    fn start(manifest: &Path) -> Self {
        let mut command = Command::new(stc());
        command.arg("lsp").arg("--manifest-path").arg(manifest);
        if let Some(parent) = manifest.parent()
            && parent.is_dir()
        {
            command.current_dir(parent);
        }
        Self::spawn(command)
    }

    fn start_in(directory: &Path) -> Self {
        let mut command = Command::new(stc());
        command.arg("lsp").current_dir(directory);
        Self::spawn(command)
    }

    fn spawn(mut command: Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn stc lsp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");
        let stderr_text = Arc::new(Mutex::new(String::new()));
        let stderr_sink = Arc::clone(&stderr_text);
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut buffer = [0_u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        if let Ok(mut text) = stderr_sink.lock() {
                            text.push_str(&String::from_utf8_lossy(&buffer[..count]));
                        }
                    }
                }
            }
        });
        let (sender, messages) = mpsc::channel();
        thread::spawn(move || read_frames(stdout, sender));
        Self {
            child: Some(child),
            stdin,
            messages,
            notifications: VecDeque::new(),
            next_id: 1,
            stderr: stderr_text,
        }
    }

    fn stderr(&self) -> String {
        self.stderr
            .lock()
            .map(|text| text.clone())
            .unwrap_or_default()
    }

    fn send(&mut self, message: &Value) {
        let body = serde_json::to_vec(message).expect("serialize JSON-RPC message");
        write!(self.stdin, "Content-Length: {}\r\n\r\n", body.len()).expect("write frame header");
        self.stdin.write_all(&body).expect("write frame body");
        self.stdin.flush().expect("flush frame");
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    fn request_raw(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let message = self.next_message();
            if message.get("id") == Some(&json!(id)) {
                return message;
            }
            self.notifications.push_back(message);
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let message = self.request_raw(method, params);
        if let Some(error) = message.get("error") {
            panic!(
                "`{method}` failed: {error}\nserver stderr:\n{}",
                self.stderr()
            );
        }
        message.get("result").cloned().unwrap_or(Value::Null)
    }

    fn next_message(&mut self) -> Value {
        self.messages.recv_timeout(TIMEOUT).unwrap_or_else(|_| {
            panic!(
                "timed out waiting for a language-server message\nserver stderr:\n{}",
                self.stderr()
            )
        })
    }

    fn try_next_message(&mut self, timeout: Duration) -> Option<Value> {
        self.messages.recv_timeout(timeout).ok()
    }

    /// Waits for a notification with `method` matching `predicate`,
    /// consuming intermediate states like [`Self::wait_diagnostics`].
    fn wait_notification<F>(&mut self, method: &str, predicate: F) -> Value
    where
        F: Fn(&Value) -> bool,
    {
        loop {
            if let Some(index) = self
                .notifications
                .iter()
                .position(|message| message["method"] == json!(method) && predicate(message))
            {
                return self
                    .notifications
                    .remove(index)
                    .expect("queued notification");
            }
            let message = self.next_message();
            if message["method"] == json!(method) && predicate(&message) {
                return message;
            }
            self.notifications.push_back(message);
        }
    }

    /// Asserts that no notification with `method` arrives within a short
    /// settling window, preserving any other notifications.
    fn assert_no_notification(&mut self, method: &str) {
        assert!(
            !self
                .notifications
                .iter()
                .any(|message| message["method"] == json!(method)),
            "unexpected queued `{method}`"
        );
        let deadline = Instant::now() + Duration::from_millis(200);
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            if let Some(message) = self.try_next_message(remaining) {
                assert_ne!(
                    message["method"],
                    json!(method),
                    "unexpected `{method}`: {message}"
                );
                self.notifications.push_back(message);
            }
        }
        assert!(
            !self
                .notifications
                .iter()
                .any(|message| message["method"] == json!(method)),
            "unexpected queued `{method}`"
        );
    }

    fn notify_changes(&mut self, paths: &[PathBuf]) {
        let changes: Vec<Value> = paths
            .iter()
            .map(|path| json!({ "uri": uri(path), "type": 2 }))
            .collect();
        self.notify(
            "workspace/didChangeWatchedFiles",
            json!({ "changes": changes }),
        );
    }

    fn diagnostics_uri(message: &Value) -> &str {
        message["params"]["uri"].as_str().unwrap_or_default()
    }

    fn diagnostics(message: &Value) -> Vec<Value> {
        message["params"]["diagnostics"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    /// Waits for a diagnostics notification for `uri` matching `predicate`,
    /// consuming intermediate states for that URI.
    fn wait_diagnostics<F>(&mut self, uri: &str, predicate: F) -> Vec<Value>
    where
        F: Fn(&[Value]) -> bool,
    {
        loop {
            if let Some(index) = self.notifications.iter().position(|message| {
                message["method"] == json!("textDocument/publishDiagnostics")
                    && Self::diagnostics_uri(message) == uri
                    && predicate(&Self::diagnostics(message))
            }) {
                let message = self
                    .notifications
                    .remove(index)
                    .expect("queued diagnostics");
                return Self::diagnostics(&message);
            }
            let message = self.next_message();
            if message["method"] == json!("textDocument/publishDiagnostics")
                && Self::diagnostics_uri(&message) == uri
            {
                let diagnostics = Self::diagnostics(&message);
                if predicate(&diagnostics) {
                    return diagnostics;
                }
                continue;
            }
            self.notifications.push_back(message);
        }
    }

    fn initialize(&mut self, root: &Path) -> Value {
        let result = self.request(
            "initialize",
            json!({
                "processId": null,
                "clientInfo": { "name": "stc-lsp-test" },
                "rootUri": uri(root),
                "capabilities": {},
            }),
        );
        self.notify("initialized", json!({}));
        result
    }

    fn open(&mut self, path: &Path, text: &str, version: i64) {
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri(path),
                    "languageId": "st",
                    "version": version,
                    "text": text,
                },
            }),
        );
    }

    fn change(&mut self, path: &Path, text: &str, version: i64) {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri(path), "version": version },
                "contentChanges": [ { "text": text } ],
            }),
        );
    }

    fn finish(mut self) -> i32 {
        let result = self.request("shutdown", Value::Null);
        assert_eq!(result, Value::Null, "shutdown result");
        self.notify("exit", Value::Null);
        self.wait_for_exit()
    }

    fn exit_without_shutdown(mut self) -> i32 {
        self.notify("exit", Value::Null);
        self.wait_for_exit()
    }

    fn wait_for_exit(&mut self) -> i32 {
        let mut child = self.child.take().expect("child process");
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match child.try_wait().expect("wait for stc lsp") {
                Some(status) => return status.code().unwrap_or(-1),
                None if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                None => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "the language server did not exit\nserver stderr:\n{}",
                        self.stderr()
                    );
                }
            }
        }
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn read_frames(stdout: impl Read, sender: mpsc::Sender<Value>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut length = None;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
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
        let Some(length) = length else {
            return;
        };
        let mut body = vec![0_u8; length];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        match serde_json::from_slice(&body) {
            Ok(message) => {
                if sender.send(message).is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

#[test]
fn definition_after_open_uses_the_analyzed_text() {
    let project = TestDir::new("position-flush");
    // The disk text uses CRLF while the editor buffer below uses LF. Before
    // the analysis flush, positions were mapped against one text and the
    // resolutions came from the other, so some identifiers silently targeted
    // the wrong token. A definition request is sent immediately after the
    // open, inside the debounce window.
    let disk = MAIN_SOURCE.replace('\n', "\r\n");
    let main = project.write("src/main.st", &disk);
    project.write("stc.toml", MANIFEST);
    let main_uri = uri(&main);

    let mut client = LspClient::start(&project.manifest());
    client.initialize(&project.path);
    client.open(&main, MAIN_SOURCE, 1);

    let definition = client.request(
        "textDocument/definition",
        json!({
            "textDocument": { "uri": main_uri },
            "position": position_of(MAIN_SOURCE, "sum := counter.state"),
        }),
    );
    assert_eq!(definition["uri"], json!(main_uri));
    assert_eq!(
        definition["range"]["start"],
        position_of(MAIN_SOURCE, "sum : DINT")
    );
}

#[test]
fn language_server_serves_diagnostics_navigation_completion_and_formatting() {
    let project = TestDir::new("session");
    let main = project.write("src/main.st", MAIN_SOURCE);
    project.write("stc.toml", MANIFEST);
    let main_uri = uri(&main);

    let mut client = LspClient::start(&project.manifest());
    let initialize = client.initialize(&project.path);
    let capabilities = &initialize["capabilities"];
    assert_eq!(capabilities["textDocumentSync"]["openClose"], json!(true));
    assert_eq!(capabilities["textDocumentSync"]["change"], json!(1));
    assert_eq!(
        capabilities["textDocumentSync"]["save"]["includeText"],
        json!(false)
    );
    assert_eq!(capabilities["positionEncoding"], json!("utf-16"));
    assert_eq!(capabilities["definitionProvider"], json!(true));
    assert_eq!(capabilities["referencesProvider"], json!(true));
    assert_eq!(capabilities["hoverProvider"], json!(true));
    assert_eq!(capabilities["documentSymbolProvider"], json!(true));
    assert_eq!(capabilities["documentFormattingProvider"], json!(true));
    assert_eq!(
        capabilities["renameProvider"],
        json!({ "prepareProvider": true })
    );
    assert_eq!(
        capabilities["experimental"],
        json!({ "stc": { "trackedRename": true } })
    );
    assert_eq!(
        capabilities["completionProvider"]["triggerCharacters"],
        json!([".", ":"])
    );
    assert_eq!(initialize["serverInfo"]["name"], json!("stc-lsp"));
    assert!(initialize["serverInfo"]["version"].is_string());

    // The project is clean, so the initial project-wide publication is empty.
    let diagnostics =
        client.wait_diagnostics(&main_uri, |diagnostics: &[Value]| diagnostics.is_empty());
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    client.open(&main, MAIN_SOURCE, 1);

    // Definition from the call site, including the non-ASCII comment line.
    let definition = client.request(
        "textDocument/definition",
        json!({
            "textDocument": { "uri": main_uri },
            "position": position_of(MAIN_SOURCE, "AddTwo(sum"),
        }),
    );
    assert_eq!(definition["uri"], json!(main_uri));
    assert_eq!(
        definition["range"]["start"],
        json!({ "line": 0, "character": 9 })
    );
    assert_eq!(
        definition["range"]["end"],
        json!({ "line": 0, "character": 15 })
    );

    // The non-ASCII prefix means the UTF-16 column is not the byte column.
    let line = MAIN_SOURCE
        .lines()
        .find(|line| line.contains("café"))
        .expect("non-ASCII line");
    let byte_column = line.find("AddTwo").expect("AddTwo column");
    let utf16_column: usize = line[..byte_column].encode_utf16().count();
    assert!(
        utf16_column < byte_column,
        "the emoji makes UTF-16 columns shorter than byte columns"
    );

    // References include the declaration only on request.
    let references = client.request(
        "textDocument/references",
        json!({
            "textDocument": { "uri": main_uri },
            "position": position_of(MAIN_SOURCE, "AddTwo(sum"),
            "context": { "includeDeclaration": true },
        }),
    );
    // Declaration, the function's own result assignment, and the call site.
    assert_eq!(references.as_array().expect("references").len(), 3);
    let uses = client.request(
        "textDocument/references",
        json!({
            "textDocument": { "uri": main_uri },
            "position": position_of(MAIN_SOURCE, "AddTwo(sum"),
            "context": { "includeDeclaration": false },
        }),
    );
    assert_eq!(uses.as_array().expect("references").len(), 2);

    // Hover on a local identifier reports its declared type.
    let hover = client.request(
        "textDocument/hover",
        json!({
            "textDocument": { "uri": main_uri },
            "position": position_of(MAIN_SOURCE, "sum := counter"),
        }),
    );
    let contents = hover["contents"]["value"].as_str().expect("hover text");
    assert!(contents.contains("DINT"), "{contents}");
    assert!(contents.contains("program Main"), "{contents}");

    // General completion offers keywords and visible names.
    let completion = client.request(
        "textDocument/completion",
        json!({
            "textDocument": { "uri": main_uri },
            "position": position_of(MAIN_SOURCE, "counter.state"),
        }),
    );
    let items = completion["items"].as_array().expect("completion items");
    let labels: Vec<&str> = items
        .iter()
        .filter_map(|item| item["label"].as_str())
        .collect();
    for expected in ["FUNCTION", "AddTwo", "sum", "Main"] {
        assert!(
            labels.contains(&expected),
            "missing {expected} in {labels:?}"
        );
    }

    // One-level member completion after `.`.
    let mut member_position = position_of(MAIN_SOURCE, "counter.state");
    let dot = "counter.".encode_utf16().count();
    let base_character = member_position["character"].as_u64().expect("character");
    member_position["character"] = json!(base_character + dot as u64);
    let members = client.request(
        "textDocument/completion",
        json!({
            "textDocument": { "uri": main_uri },
            "position": member_position,
        }),
    );
    let member_labels: Vec<&str> = members["items"]
        .as_array()
        .expect("member items")
        .iter()
        .filter_map(|item| item["label"].as_str())
        .collect();
    for expected in ["value", "increment", "state"] {
        assert!(
            member_labels.contains(&expected),
            "missing member {expected} in {member_labels:?}"
        );
    }

    // Document symbols are hierarchical.
    let symbols = client.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": main_uri } }),
    );
    let symbols = symbols.as_array().expect("document symbols");
    let main_symbol = symbols
        .iter()
        .find(|symbol| symbol["name"] == json!("Main"))
        .expect("PROGRAM Main symbol");
    assert_eq!(main_symbol["kind"], json!(5));
    assert!(
        !main_symbol["children"]
            .as_array()
            .expect("children")
            .is_empty(),
        "the program has variable and method children"
    );
    assert!(
        symbols
            .iter()
            .any(|symbol| symbol["name"] == json!("AddTwo"))
    );

    // The parse error is published first; after the debounced analysis the
    // semantic error follows.
    let error_uri = main_uri.clone();
    client.change(&main, SEMANTIC_ERROR_SOURCE, 2);
    let diagnostics = client.wait_diagnostics(&error_uri, |diagnostics| {
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == json!("STC-E-NAME-001"))
    });
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == json!("STC-E-NAME-001"))
        .expect("unknown-name diagnostic");
    assert_eq!(diagnostic["severity"], json!(1));
    assert_eq!(diagnostic["source"], json!("stc"));
    assert!(
        diagnostic["message"]
            .as_str()
            .is_some_and(|message| message.contains("MissingName"))
    );

    // A syntax error is reported from the parse alone.
    client.change(&main, PARSE_ERROR_SOURCE, 3);
    let diagnostics = client.wait_diagnostics(&error_uri, |diagnostics| !diagnostics.is_empty());
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["severity"] == json!(1)),
        "{diagnostics:?}"
    );

    // Fixing the buffer clears the stale diagnostics after analysis.
    client.change(&main, MAIN_SOURCE, 4);
    let diagnostics =
        client.wait_diagnostics(&error_uri, |diagnostics: &[Value]| diagnostics.is_empty());
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    // Formatting returns one whole-document edit for a messy buffer.
    client.change(&main, MESSY_SOURCE, 5);
    let edits = client.request(
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": main_uri },
            "options": { "tabSize": 2, "insertSpaces": true },
        }),
    );
    let edits = edits.as_array().expect("formatting edits");
    assert_eq!(edits.len(), 1);
    assert_eq!(
        edits[0]["range"]["start"],
        json!({ "line": 0, "character": 0 })
    );
    let formatted = edits[0]["newText"].as_str().expect("formatted text");
    assert!(formatted.contains("PROGRAM Main"), "{formatted}");
    assert!(formatted.contains("  x : DINT;"), "{formatted}");
    assert!(formatted.contains("x := 1 + 2;"), "{formatted}");
    assert_eq!(edits[0]["range"]["end"]["line"], json!(6));

    // Formatting an already formatted buffer is a no-op.
    client.change(&main, formatted, 6);
    let edits = client.request(
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": main_uri },
            "options": { "tabSize": 2, "insertSpaces": true },
        }),
    );
    assert_eq!(edits, json!([]));

    // Formatting a buffer with syntax errors is refused.
    client.change(&main, PARSE_ERROR_SOURCE, 7);
    let edits = client.request(
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": main_uri },
            "options": { "tabSize": 2, "insertSpaces": true },
        }),
    );
    assert_eq!(edits, Value::Null);

    // Save and close are accepted; the disk text still answers queries.
    client.notify(
        "textDocument/didSave",
        json!({ "textDocument": { "uri": main_uri } }),
    );
    client.notify(
        "textDocument/didClose",
        json!({ "textDocument": { "uri": main_uri } }),
    );
    let symbols = client.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": main_uri } }),
    );
    assert!(!symbols.as_array().expect("symbols").is_empty());

    // A watched new source reloads the project graph.
    let extra = project.write("src/extra.st", EXTRA_SOURCE);
    client.notify(
        "workspace/didChangeWatchedFiles",
        json!({ "changes": [ { "uri": uri(&extra), "type": 1 } ] }),
    );
    let extra_symbols = client.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": uri(&extra) } }),
    );
    assert!(
        extra_symbols
            .as_array()
            .expect("symbols")
            .iter()
            .any(|symbol| symbol["name"] == json!("Triple")),
        "{extra_symbols}"
    );

    assert_eq!(client.finish(), 0);
}

#[test]
fn sample_workspace_serves_diagnostics_navigation_completion_and_formatting() {
    let project = TestDir::new("sample");
    copy_tree(&sample_workspace(), &project.path);
    let main = project.path.join("src").join("main.st");
    let smoke = project.path.join("tests").join("smoke.st");
    let main_source = std::fs::read_to_string(&main).expect("read sample main source");
    let smoke_source = std::fs::read_to_string(&smoke).expect("read sample test source");
    let main_uri = uri(&main);
    let smoke_uri = uri(&smoke);

    let mut client = LspClient::start(&project.manifest());
    let initialize = client.initialize(&project.path);
    assert_eq!(initialize["serverInfo"]["name"], json!("stc-lsp"));

    // The sample is clean in both the production and test source sets.
    let diagnostics =
        client.wait_diagnostics(&main_uri, |diagnostics: &[Value]| diagnostics.is_empty());
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let diagnostics =
        client.wait_diagnostics(&smoke_uri, |diagnostics: &[Value]| diagnostics.is_empty());
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    client.open(&main, &main_source, 1);
    client.open(&smoke, &smoke_source, 1);

    // Definition from the program's call site resolves to the function name.
    let function_declaration = position_of(&main_source, "AddStep : DINT");
    let definition = client.request(
        "textDocument/definition",
        json!({
            "textDocument": { "uri": main_uri },
            "position": position_of(&main_source, "AddStep(sum)"),
        }),
    );
    assert_eq!(definition["uri"], json!(main_uri));
    assert_eq!(definition["range"]["start"], function_declaration);
    assert_eq!(
        definition["range"]["end"]["line"],
        function_declaration["line"]
    );

    // References cover the declaration, the result assignment, and both calls;
    // the test source is part of the analysis.
    let references = client.request(
        "textDocument/references",
        json!({
            "textDocument": { "uri": main_uri },
            "position": position_of(&main_source, "AddStep(sum)"),
            "context": { "includeDeclaration": true },
        }),
    );
    let references = references.as_array().expect("references");
    assert_eq!(references.len(), 4, "{references:?}");
    assert!(
        references
            .iter()
            .any(|reference| reference["uri"] == json!(smoke_uri)),
        "the test call is a reference: {references:?}"
    );
    let uses = client.request(
        "textDocument/references",
        json!({
            "textDocument": { "uri": main_uri },
            "position": position_of(&main_source, "AddStep(sum)"),
            "context": { "includeDeclaration": false },
        }),
    );
    assert_eq!(uses.as_array().expect("uses").len(), 3);

    // The test source resolves into the production file.
    let test_definition = client.request(
        "textDocument/definition",
        json!({
            "textDocument": { "uri": smoke_uri },
            "position": position_of(&smoke_source, "AddStep(2)"),
        }),
    );
    assert_eq!(test_definition["uri"], json!(main_uri));
    assert_eq!(test_definition["range"]["start"], function_declaration);

    // Hover on a program variable reports its type and owner.
    let hover = client.request(
        "textDocument/hover",
        json!({
            "textDocument": { "uri": main_uri },
            "position": position_of(&main_source, "sum := AddStep"),
        }),
    );
    let contents = hover["contents"]["value"].as_str().expect("hover text");
    assert!(contents.contains("DINT"), "{contents}");
    assert!(contents.contains("program Main"), "{contents}");

    // Completion offers keywords and visible names.
    let completion = client.request(
        "textDocument/completion",
        json!({
            "textDocument": { "uri": main_uri },
            "position": position_of(&main_source, "sum := AddStep"),
        }),
    );
    let labels: Vec<&str> = completion["items"]
        .as_array()
        .expect("completion items")
        .iter()
        .filter_map(|item| item["label"].as_str())
        .collect();
    for expected in ["FUNCTION", "AddStep", "INCREMENT", "sum", "Main"] {
        assert!(
            labels.contains(&expected),
            "missing {expected} in {labels:?}"
        );
    }

    // Document symbols cover the program, the function block, and the function.
    let symbols = client.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": main_uri } }),
    );
    let symbols = symbols.as_array().expect("document symbols");
    for expected in ["Main", "Accumulator", "AddStep"] {
        assert!(
            symbols
                .iter()
                .any(|symbol| symbol["name"] == json!(expected)),
            "missing symbol {expected} in {symbols:?}"
        );
    }
    let main_symbol = symbols
        .iter()
        .find(|symbol| symbol["name"] == json!("Main"))
        .expect("PROGRAM Main symbol");
    assert!(
        !main_symbol["children"]
            .as_array()
            .expect("children")
            .is_empty(),
        "the program has variable children"
    );

    // The sample is already formatted; the formatter returns no edit.
    let edits = client.request(
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": main_uri },
            "options": { "tabSize": 2, "insertSpaces": true },
        }),
    );
    assert_eq!(edits, json!([]));

    // An unformatted buffer produces one whole-document edit.
    let messy = main_source.replacen("  sum := AddStep(sum);", "  sum:=AddStep(sum);", 1);
    assert_ne!(messy, main_source);
    client.change(&main, &messy, 2);
    let edits = client.request(
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": main_uri },
            "options": { "tabSize": 2, "insertSpaces": true },
        }),
    );
    let edits = edits.as_array().expect("formatting edits");
    assert_eq!(edits.len(), 1);
    let formatted = edits[0]["newText"].as_str().expect("formatted text");
    assert!(formatted.contains("  sum := AddStep(sum);"), "{formatted}");

    assert_eq!(client.finish(), 0);
}

#[test]
fn initialize_discovers_the_manifest_from_the_workspace_root() {
    let project = TestDir::new("discovery");
    project.write("src/main.st", MAIN_SOURCE);
    project.write("stc.toml", MANIFEST);

    // No `--manifest-path`: discovery uses the initialize rootUri.
    let mut client = LspClient::start_in(&project.path);
    let initialize = client.initialize(&project.path);
    assert_eq!(
        initialize["capabilities"]["definitionProvider"],
        json!(true)
    );
    assert_eq!(client.finish(), 0);
}

#[test]
fn initialize_failure_is_readable_and_exit_without_shutdown_is_one() {
    let project = TestDir::new("failure");
    let missing = project.path.join("missing").join("stc.toml");
    let mut client = LspClient::start(&missing);
    let response = client.request_raw(
        "initialize",
        json!({ "processId": null, "capabilities": {} }),
    );
    let error = &response["error"];
    assert_eq!(
        error["code"],
        json!(-32603),
        "initialize must fail cleanly: {response}"
    );
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|message| message.contains("cannot load project")),
        "{error}"
    );
    // `exit` without `shutdown` exits with code 1.
    assert_eq!(client.exit_without_shutdown(), 1);
}

fn document_position(path: &Path, position: Value) -> Value {
    json!({
        "textDocument": { "uri": uri(path) },
        "position": position,
    })
}

fn rename_params(path: &Path, position: Value, new_name: &str) -> Value {
    json!({
        "textDocument": { "uri": uri(path) },
        "position": position,
        "newName": new_name,
    })
}

fn assert_request_failed(response: &Value, expected: &str) {
    assert_eq!(
        response["error"]["code"],
        json!(-32803),
        "expected RequestFailed: {response}"
    );
    let message = response["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("expected an error message: {response}"));
    assert!(
        message.contains(expected),
        "expected {expected:?} in {message:?}"
    );
}

/// Applies a tracked-rename `changes` map to disk, records its token when
/// present, and reloads the server's analysis.
fn apply_tracked_rename(client: &mut LspClient, manifest: &Path, response: &Value) {
    let changes = &response["edit"]["changes"];
    let mut changed: Vec<PathBuf> = changes
        .as_object()
        .expect("tracked rename changes")
        .keys()
        .map(|uri| stc_lsp::uri_to_path(uri).expect("change URI"))
        .collect();
    apply_changes(changes);
    if let Some(token) = response["token"].as_str() {
        let recorded = client.request("stc/recordRename", json!({ "token": token }));
        assert_eq!(recorded["recorded"], json!(true), "{recorded}");
        assert_eq!(recorded["entries"], json!(1), "{recorded}");
    }
    changed.push(manifest.to_path_buf());
    changed.sort();
    changed.dedup();
    client.notify_changes(&changed);
}

#[test]
fn standard_rename_edits_sources_without_writing_history() {
    let project = TestDir::new("standard-rename");
    copy_tree(&sample_workspace(), &project.path);
    let main = project.path.join("src").join("main.st");
    let smoke = project.path.join("tests").join("smoke.st");
    let main_source = std::fs::read_to_string(&main).expect("read sample main source");
    let smoke_source = std::fs::read_to_string(&smoke).expect("read sample test source");

    let mut client = LspClient::start(&project.manifest());
    client.initialize(&project.path);
    let response = client.request(
        "textDocument/rename",
        rename_params(&main, position_of(&main_source, "AddStep(sum)"), "Advance"),
    );
    let changes = response["changes"].as_object().expect("changes object");
    assert_eq!(changes.len(), 2, "{changes:?}");
    let main_edits = changes
        .get(&uri(&main))
        .and_then(Value::as_array)
        .expect("main edits");
    assert_eq!(main_edits.len(), 3, "{main_edits:?}");
    assert_text_edit(
        &Value::Array(main_edits.clone()),
        range_after(&main_source, "FUNCTION AddStep : DINT", "AddStep"),
        "Advance",
    );
    assert_text_edit(
        &Value::Array(main_edits.clone()),
        range_after(&main_source, "AddStep := value", "AddStep"),
        "Advance",
    );
    assert_text_edit(
        &Value::Array(main_edits.clone()),
        range_after(&main_source, "AddStep(sum)", "AddStep"),
        "Advance",
    );
    let smoke_edits = changes
        .get(&uri(&smoke))
        .and_then(Value::as_array)
        .expect("smoke edits");
    assert_eq!(smoke_edits.len(), 1, "{smoke_edits:?}");
    assert_text_edit(
        &Value::Array(smoke_edits.clone()),
        range_after(&smoke_source, "AddStep(2)", "AddStep"),
        "Advance",
    );

    // The standard method never writes history.
    assert!(!project.path.join("stc.renames.toml").exists());
    let message = client.wait_notification("window/showMessage", |message| {
        message["params"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("without recording history"))
    });
    assert_eq!(message["params"]["type"], json!(2));
    let text = message["params"]["message"].as_str().expect("warning");
    assert!(text.contains("AddStep -> Advance"), "{text}");
    assert!(text.contains("[[rename]]"), "{text}");
    assert!(text.contains("cold"), "{text}");
    assert_eq!(client.finish(), 0);
}

#[test]
fn prepare_rename_reports_identifier_ranges_and_rejections() {
    let project = TestDir::new("prepare-rename");
    copy_tree(&sample_workspace(), &project.path);
    let main = project.path.join("src").join("main.st");
    let main_source = std::fs::read_to_string(&main).expect("read sample main source");

    let mut client = LspClient::start(&project.manifest());
    client.initialize(&project.path);
    let prepared = client.request(
        "textDocument/prepareRename",
        document_position(&main, position_of(&main_source, "AddStep(sum)")),
    );
    assert_eq!(
        prepared["range"],
        range_after(&main_source, "AddStep(sum)", "AddStep")
    );
    assert_eq!(prepared["placeholder"], json!("AddStep"));

    // A non-identifier position and an unresolved identifier prepare `null`.
    let prepared = client.request(
        "textDocument/prepareRename",
        document_position(&main, json!({ "line": 0, "character": 0 })),
    );
    assert_eq!(prepared, Value::Null);
    assert_eq!(client.finish(), 0);
}

#[test]
fn prepare_rename_reaches_var_access_and_var_config_segments() {
    let project = TestDir::new("prepare-access");
    write_access_project(&project);
    let main = project.path.join("src").join("main.st");

    let mut client = LspClient::start(&project.manifest());
    client.initialize(&project.path);
    for (anchor, inner) in [
        (" : x : DINT READ_ONLY", "x"),
        (" : nested.x : DINT", "x"),
        (" : inst.x : DINT", "x"),
        (" : inst.nested.x : DINT", "x"),
        ("inst.x AT %MD11", "x"),
        ("inst.nested.x AT %MD10", "x"),
        (" : seed : DINT", "seed"),
    ] {
        let prepared = client.request(
            "textDocument/prepareRename",
            document_position(&main, position_after(ACCESS_SOURCE, anchor, inner)),
        );
        assert_eq!(
            prepared["range"],
            range_after(ACCESS_SOURCE, anchor, inner),
            "{anchor}"
        );
        assert_eq!(prepared["placeholder"], json!(inner), "{anchor}");
    }
    assert_eq!(client.finish(), 0);
}

#[test]
fn rename_negatives_return_request_failed() {
    let project = TestDir::new("rename-negatives");
    project.write("stc.toml", &project_manifest("app", ""));
    let source = "\
PROGRAM Main
VAR
  Speed : DINT;
END_VAR
  Speed := Speed + 1;
  Speed := ABS(Speed);
  Speed := MissingName;
END_PROGRAM
";
    let main = project.write("src/main.st", source);

    let mut client = LspClient::start(&project.manifest());
    client.initialize(&project.path);

    let response = client.request_raw(
        "textDocument/rename",
        rename_params(&main, position_of(source, "Speed : DINT"), "not a name"),
    );
    assert_request_failed(&response, "not a valid identifier");

    let response = client.request_raw(
        "textDocument/rename",
        rename_params(&main, position_of(source, "Speed : DINT"), "IF"),
    );
    assert_request_failed(&response, "not a valid identifier");

    let response = client.request_raw(
        "textDocument/prepareRename",
        document_position(&main, position_of(source, "ABS(Speed)")),
    );
    assert_request_failed(&response, "cannot be renamed");
    let response = client.request_raw(
        "textDocument/rename",
        rename_params(&main, position_of(source, "ABS(Speed)"), "Value"),
    );
    assert_request_failed(&response, "cannot be renamed");

    let prepared = client.request(
        "textDocument/prepareRename",
        document_position(&main, position_of(source, "MissingName")),
    );
    assert_eq!(prepared, Value::Null);
    let response = client.request_raw(
        "textDocument/rename",
        rename_params(&main, position_of(source, "MissingName"), "Known"),
    );
    assert_request_failed(&response, "does not resolve");

    // Line 0 is the `PROGRAM` keyword, not an identifier.
    let prepared = client.request(
        "textDocument/prepareRename",
        document_position(&main, json!({ "line": 0, "character": 0 })),
    );
    assert_eq!(prepared, Value::Null);
    let response = client.request_raw(
        "textDocument/rename",
        rename_params(&main, json!({ "line": 0, "character": 0 }), "Other"),
    );
    assert_request_failed(&response, "not on an identifier");

    assert_eq!(client.finish(), 0);
}

#[test]
fn dependency_declarations_are_rejected() {
    let project = TestDir::new("dependency-rename");
    copy_tree(&examples_dir().join("motor"), &project.path.join("motor"));
    copy_tree(
        &examples_dir().join("motorlib"),
        &project.path.join("motorlib"),
    );
    let manifest = project.path.join("motor").join("stc.toml");
    let library = project
        .path
        .join("motorlib")
        .join("src")
        .join("motorlib.st");
    let source = std::fs::read_to_string(&library).expect("read motorlib source");

    let mut client = LspClient::start(&manifest);
    client.initialize(&project.path.join("motor"));
    let position = position_of(&source, "DriveState : (");
    let response = client.request_raw(
        "stc/rename",
        rename_params(&library, position.clone(), "ModeState"),
    );
    assert_request_failed(&response, "belongs to a dependency");
    let response = client.request_raw(
        "textDocument/prepareRename",
        document_position(&library, position),
    );
    assert_request_failed(&response, "belongs to a dependency");
    assert_eq!(client.finish(), 0);
}

#[test]
fn tracked_rename_returns_history_and_tokens_only_for_tracked_targets() {
    let project = TestDir::new("tracked-results");
    project.write("stc.toml", &project_manifest("app", ""));
    let main = project.write("src/main.st", BASIC_SOURCE);

    let mut client = LspClient::start(&project.manifest());
    client.initialize(&project.path);

    let function = client.request(
        "stc/rename",
        rename_params(
            &main,
            position_after(BASIC_SOURCE, "FUNCTION AddStep", "AddStep"),
            "Advance",
        ),
    );
    assert_eq!(
        function["history"],
        json!([{ "old": "AddStep", "new": "Advance" }])
    );
    assert_eq!(function["online"], json!("preserved"));
    assert_eq!(function["warnings"], json!([]));
    assert!(function["token"].is_string(), "{function}");

    let field = client.request(
        "stc/rename",
        rename_params(&main, position_of(BASIC_SOURCE, "Speed : DINT"), "Rate"),
    );
    assert_eq!(
        field["history"],
        json!([{ "old": "Main.Speed", "new": "Main.Rate" }])
    );
    assert!(field["token"].is_string(), "{field}");

    for (needle, new_name) in [
        ("value : DINT", "input"),
        ("local_value : DINT", "scratch"),
        ("Speed : DINT", "speed"),
    ] {
        let response = client.request(
            "stc/rename",
            rename_params(&main, position_of(BASIC_SOURCE, needle), new_name),
        );
        assert_eq!(response["history"], json!([]), "{needle}");
        assert!(response["token"].is_null(), "{needle}: {response}");
        assert_eq!(response["online"], json!("preserved"), "{needle}");
    }
    assert_eq!(client.finish(), 0);
}

#[test]
fn tracked_rename_reports_an_unavailable_profile_without_writing() {
    let project = TestDir::new("tracked-profile");
    project.write(
        "stc.toml",
        &project_manifest("app", "[io]\nconfig = \"io/missing.toml\""),
    );
    let main = project.write("src/main.st", BASIC_SOURCE);

    let mut client = LspClient::start(&project.manifest());
    client.initialize(&project.path);
    let response = client.request(
        "stc/rename",
        rename_params(&main, position_of(BASIC_SOURCE, "Speed : DINT"), "Rate"),
    );
    assert!(response["token"].is_string(), "{response}");
    let warnings = response["warnings"].as_array().expect("warnings");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let text = warnings[0].as_str().expect("warning text");
    assert!(text.contains("profile is unavailable"), "{text}");
    assert!(text.contains("missing.toml"), "{text}");
    assert!(!project.path.join("stc.renames.toml").exists());
    assert_eq!(client.finish(), 0);
}

#[test]
fn case_only_tracked_rename_has_edits_but_no_history() {
    let project = TestDir::new("case-only");
    project.write("stc.toml", &project_manifest("app", ""));
    let main = project.write("src/main.st", BASIC_SOURCE);

    let mut client = LspClient::start(&project.manifest());
    client.initialize(&project.path);
    let response = client.request(
        "stc/rename",
        rename_params(&main, position_of(BASIC_SOURCE, "Speed : DINT"), "speed"),
    );
    assert_eq!(response["history"], json!([]));
    assert!(response["token"].is_null(), "{response}");
    let edits = response["edit"]["changes"][&uri(&main)]
        .as_array()
        .expect("case-only edits");
    assert_eq!(edits.len(), 3, "{edits:?}");
    assert!(edits.iter().all(|edit| edit["newText"] == json!("speed")));
    assert!(!project.path.join("stc.renames.toml").exists());
    assert_eq!(client.finish(), 0);
}

#[test]
fn collisions_and_failed_renames_invalidate_pending_tokens() {
    let project = TestDir::new("collision");
    project.write("stc.toml", &project_manifest("app", ""));
    let main = project.write("src/main.st", BASIC_SOURCE);

    let mut client = LspClient::start(&project.manifest());
    client.initialize(&project.path);
    let first = client.request(
        "stc/rename",
        rename_params(&main, position_of(BASIC_SOURCE, "Speed : DINT"), "Rate"),
    );
    let token = first["token"].as_str().expect("first token").to_owned();

    let response = client.request_raw(
        "stc/rename",
        rename_params(&main, position_of(BASIC_SOURCE, "Speed : DINT"), "TEXT"),
    );
    assert_request_failed(&response, "already names a declaration");
    assert!(response["result"].is_null(), "{response}");

    // The failed rename cleared the pending token just like a successful one.
    let response = client.request_raw("stc/recordRename", json!({ "token": token }));
    assert_request_failed(&response, "no pending rename");

    // A case-only rename of the same target is allowed and history-free.
    let response = client.request(
        "stc/rename",
        rename_params(&main, position_of(BASIC_SOURCE, "Speed : DINT"), "SPEED"),
    );
    assert_eq!(response["history"], json!([]));
    assert!(response["token"].is_null(), "{response}");
    assert_eq!(client.finish(), 0);
}

#[test]
fn tracked_rename_edits_external_references_and_records_history() {
    let project = TestDir::new("external-references");
    write_external_project(&project);
    let main = project.path.join("src").join("main.st");
    let tests = project.path.join("tests").join("sample_test.st");
    let profile = project.path.join("io").join("bench.toml");
    let manifest = project.manifest();
    let source = std::fs::read_to_string(&main).expect("read main source");
    let manifest_text = std::fs::read_to_string(&manifest).expect("read manifest");
    let profile_text = std::fs::read_to_string(&profile).expect("read profile");

    let mut client = LspClient::start(&manifest);
    client.initialize(&project.path);

    // A short global name edits the declaration, the binding, and the profile.
    let response = client.request(
        "stc/rename",
        rename_params(&main, position_of(&source, "Short AT %QX0.0"), "Brief"),
    );
    assert_eq!(
        response["history"],
        json!([{ "old": "Short", "new": "Brief" }])
    );
    assert_eq!(response["online"], json!("preserved"));
    assert_eq!(response["warnings"], json!([]));
    assert!(response["token"].is_string(), "{response}");
    let changes = response["edit"]["changes"].as_object().expect("changes");
    assert_eq!(changes.len(), 3, "{changes:?}");
    assert_text_edit(
        changes.get(&uri(&main)).expect("source edit"),
        range_after(&source, "Short AT %QX0.0", "Short"),
        "Brief",
    );
    assert_text_edit(
        changes.get(&uri(&manifest)).expect("manifest edit"),
        range_of_within(&manifest_text, "variable = \"Short\"", "Short"),
        "Brief",
    );
    assert_text_edit(
        changes.get(&uri(&profile)).expect("profile edit"),
        range_of_within(&profile_text, "variable = \"Short\"", "Short"),
        "Brief",
    );
    // `stc/rename` never writes, even when it returns a token.
    assert_eq!(
        std::fs::read_to_string(&main).expect("unchanged source"),
        source
    );
    assert!(!project.path.join("stc.renames.toml").exists());
    apply_tracked_rename(&mut client, &manifest, &response);

    let history =
        std::fs::read_to_string(project.path.join("stc.renames.toml")).expect("recorded history");
    assert!(history.contains("old = \"Short\""), "{history}");
    assert!(history.contains("new = \"Brief\""), "{history}");

    // A namespace binding edits only the source declaration and the manifest.
    let source = std::fs::read_to_string(&main).expect("read source");
    let manifest_text = std::fs::read_to_string(&manifest).expect("read manifest");
    let response = client.request(
        "stc/rename",
        rename_params(&main, position_of(&source, "Ns AT %IX0.1"), "Node"),
    );
    assert_eq!(response["history"], json!([{ "old": "Ns", "new": "Node" }]));
    let changes = response["edit"]["changes"].as_object().expect("changes");
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert_text_edit(
        changes.get(&uri(&main)).expect("source edit"),
        range_after(&source, "Ns AT %IX0.1", "Ns"),
        "Node",
    );
    assert_text_edit(
        changes.get(&uri(&manifest)).expect("manifest edit"),
        range_of_within(&manifest_text, "variable = \"Ns\"", "Ns"),
        "Node",
    );
    apply_tracked_rename(&mut client, &manifest, &response);

    // A package-prefixed binding edits the last binding segment only.
    let source = std::fs::read_to_string(&main).expect("read source");
    let manifest_text = std::fs::read_to_string(&manifest).expect("read manifest");
    let response = client.request(
        "stc/rename",
        rename_params(&main, position_of(&source, "Pkg AT %IX0.2"), "Bundle"),
    );
    assert_eq!(
        response["history"],
        json!([{ "old": "Pkg", "new": "Bundle" }])
    );
    let changes = response["edit"]["changes"].as_object().expect("changes");
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert_text_edit(
        changes.get(&uri(&manifest)).expect("manifest edit"),
        range_of_within(&manifest_text, "variable = \"app.Pkg\"", "Pkg"),
        "Bundle",
    );
    apply_tracked_rename(&mut client, &manifest, &response);
    assert!(
        std::fs::read_to_string(&manifest)
            .expect("read manifest")
            .contains("variable = \"app.Bundle\"")
    );

    // A profile sink reference is edited too.
    let source = std::fs::read_to_string(&main).expect("read source");
    let profile_text = std::fs::read_to_string(&profile).expect("read profile");
    let response = client.request(
        "stc/rename",
        rename_params(&main, position_of(&source, "Other AT %IX0.3"), "OtherNode"),
    );
    assert_eq!(
        response["history"],
        json!([{ "old": "Other", "new": "OtherNode" }])
    );
    let changes = response["edit"]["changes"].as_object().expect("changes");
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert_text_edit(
        changes.get(&uri(&profile)).expect("profile edit"),
        range_of_within(&profile_text, "variable = \"Other\"", "Other"),
        "OtherNode",
    );
    apply_tracked_rename(&mut client, &manifest, &response);

    // An instance-qualified binding edits `main.Field`.
    let source = std::fs::read_to_string(&main).expect("read source");
    let manifest_text = std::fs::read_to_string(&manifest).expect("read manifest");
    let response = client.request(
        "stc/rename",
        rename_params(&main, position_of(&source, "Field AT %QX0.1"), "Out"),
    );
    assert_eq!(
        response["history"],
        json!([{ "old": "Main.Field", "new": "Main.Out" }])
    );
    let changes = response["edit"]["changes"].as_object().expect("changes");
    assert_eq!(changes.len(), 2, "{changes:?}");
    let edits = changes
        .get(&uri(&main))
        .and_then(Value::as_array)
        .expect("source edits");
    assert_eq!(edits.len(), 2, "{edits:?}");
    assert_text_edit(
        &Value::Array(edits.clone()),
        range_after(&source, "Field AT %QX0.1", "Field"),
        "Out",
    );
    assert_text_edit(
        &Value::Array(edits.clone()),
        range_after(&source, "Field := TRUE", "Field"),
        "Out",
    );
    assert_text_edit(
        changes.get(&uri(&manifest)).expect("manifest edit"),
        range_of_within(&manifest_text, "variable = \"main.Field\"", "Field"),
        "Out",
    );
    apply_tracked_rename(&mut client, &manifest, &response);

    // A test case edits the function and the manifest case name.
    let test_source = std::fs::read_to_string(&tests).expect("read test source");
    let manifest_text = std::fs::read_to_string(&manifest).expect("read manifest");
    let response = client.request(
        "stc/rename",
        rename_params(
            &tests,
            position_after(&test_source, "FUNCTION SampleTest", "SampleTest"),
            "SmokeTest",
        ),
    );
    assert_eq!(
        response["history"],
        json!([{ "old": "SampleTest", "new": "SmokeTest" }])
    );
    let changes = response["edit"]["changes"].as_object().expect("changes");
    assert_eq!(changes.len(), 2, "{changes:?}");
    let edits = changes
        .get(&uri(&tests))
        .and_then(Value::as_array)
        .expect("test edits");
    assert_eq!(edits.len(), 2, "{edits:?}");
    assert_text_edit(
        &Value::Array(edits.clone()),
        range_after(&test_source, "FUNCTION SampleTest", "SampleTest"),
        "SmokeTest",
    );
    assert_text_edit(
        &Value::Array(edits.clone()),
        range_after(&test_source, "SampleTest := result", "SampleTest"),
        "SmokeTest",
    );
    assert_text_edit(
        changes.get(&uri(&manifest)).expect("manifest edit"),
        range_of_within(&manifest_text, "name = \"SampleTest\"", "SampleTest"),
        "SmokeTest",
    );
    apply_tracked_rename(&mut client, &manifest, &response);

    // Renaming the selected configuration edits the manifest but is cold and
    // history-free.
    let source = std::fs::read_to_string(&main).expect("read source");
    let manifest_text = std::fs::read_to_string(&manifest).expect("read manifest");
    let response = client.request(
        "stc/rename",
        rename_params(
            &main,
            position_after(&source, "CONFIGURATION MainConfig", "MainConfig"),
            "MainConfigX",
        ),
    );
    assert_eq!(response["history"], json!([]));
    assert!(response["token"].is_null(), "{response}");
    assert_eq!(response["online"], json!("cold"));
    assert!(
        response["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .any(|warning| warning
                .as_str()
                .is_some_and(|warning| warning.contains("cold"))),
        "{response}"
    );
    let changes = response["edit"]["changes"].as_object().expect("changes");
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert_text_edit(
        changes.get(&uri(&main)).expect("source edit"),
        range_after(&source, "CONFIGURATION MainConfig", "MainConfig"),
        "MainConfigX",
    );
    assert_text_edit(
        changes.get(&uri(&manifest)).expect("manifest edit"),
        range_of_within(
            &manifest_text,
            "configuration = \"MainConfig\"",
            "MainConfig",
        ),
        "MainConfigX",
    );
    apply_tracked_rename(&mut client, &manifest, &response);
    assert!(
        !std::fs::read_to_string(project.path.join("stc.renames.toml"))
            .expect("recorded history")
            .contains("MainConfig")
    );

    let history =
        std::fs::read_to_string(project.path.join("stc.renames.toml")).expect("recorded history");
    for (old, new) in [
        ("Short", "Brief"),
        ("Ns", "Node"),
        ("Pkg", "Bundle"),
        ("Other", "OtherNode"),
        ("Main.Field", "Main.Out"),
        ("SampleTest", "SmokeTest"),
    ] {
        assert!(
            history.contains(&format!("old = \"{old}\"")),
            "missing {old}: {history}"
        );
        assert!(
            history.contains(&format!("new = \"{new}\"")),
            "missing {new}: {history}"
        );
    }

    let check = run_stc(&project.path, &["check"]);
    assert_success(&check, "the renamed external project checks");
    assert_eq!(client.finish(), 0);
}

#[test]
fn tracked_rename_edits_var_access_and_var_config_paths() {
    let project = TestDir::new("access-edits");
    write_access_project(&project);
    let main = project.path.join("src").join("main.st");
    let manifest = project.manifest();

    let mut client = LspClient::start(&manifest);
    client.initialize(&project.path);

    let response = client.request(
        "stc/rename",
        rename_params(
            &main,
            position_after(ACCESS_SOURCE, " : x : DINT READ_ONLY", "x"),
            "value",
        ),
    );
    assert_eq!(
        response["history"],
        json!([{ "old": "Main.x", "new": "Main.value" }])
    );
    let changes = response["edit"]["changes"].as_object().expect("changes");
    assert_eq!(changes.len(), 1, "{changes:?}");
    let edits = changes
        .get(&uri(&main))
        .and_then(Value::as_array)
        .expect("source edits");
    assert_eq!(edits.len(), 5, "{edits:?}");
    assert_text_edit(
        &Value::Array(edits.clone()),
        range_after(ACCESS_SOURCE, "PROGRAM Main\nVAR\n  x : DINT;", "x"),
        "value",
    );
    assert_text_edit(
        &Value::Array(edits.clone()),
        range_after(ACCESS_SOURCE, " : x : DINT READ_ONLY", "x"),
        "value",
    );
    assert_text_edit(
        &Value::Array(edits.clone()),
        range_after(ACCESS_SOURCE, " : inst.x : DINT", "x"),
        "value",
    );
    assert_text_edit(
        &Value::Array(edits.clone()),
        range_after(ACCESS_SOURCE, "inst.x AT %MD11", "x"),
        "value",
    );
    assert_text_edit(
        &Value::Array(edits.clone()),
        range_after(ACCESS_SOURCE, "x := 1;", "x"),
        "value",
    );
    apply_tracked_rename(&mut client, &manifest, &response);

    let source = std::fs::read_to_string(&main).expect("read source");
    let response = client.request(
        "stc/rename",
        rename_params(
            &main,
            position_after(&source, " : nested.x : DINT", "x"),
            "y",
        ),
    );
    assert_eq!(
        response["history"],
        json!([{ "old": "Inner.x", "new": "Inner.y" }])
    );
    let changes = response["edit"]["changes"].as_object().expect("changes");
    let edits = changes
        .get(&uri(&main))
        .and_then(Value::as_array)
        .expect("source edits");
    assert_eq!(edits.len(), 4, "{edits:?}");
    assert_text_edit(
        &Value::Array(edits.clone()),
        range_after(&source, "TYPE Inner : STRUCT\n  x : DINT;", "x"),
        "y",
    );
    assert_text_edit(
        &Value::Array(edits.clone()),
        range_after(&source, " : nested.x : DINT", "x"),
        "y",
    );
    assert_text_edit(
        &Value::Array(edits.clone()),
        range_after(&source, " : inst.nested.x : DINT", "x"),
        "y",
    );
    assert_text_edit(
        &Value::Array(edits.clone()),
        range_after(&source, "inst.nested.x AT %MD10", "x"),
        "y",
    );
    apply_tracked_rename(&mut client, &manifest, &response);

    let source = std::fs::read_to_string(&main).expect("read source");
    let response = client.request(
        "stc/rename",
        rename_params(
            &main,
            position_after(&source, " : seed : DINT", "seed"),
            "start",
        ),
    );
    assert_eq!(
        response["history"],
        json!([{ "old": "C.seed", "new": "C.start" }])
    );
    let changes = response["edit"]["changes"].as_object().expect("changes");
    let edits = changes
        .get(&uri(&main))
        .and_then(Value::as_array)
        .expect("source edits");
    assert_eq!(edits.len(), 2, "{edits:?}");
    assert_text_edit(
        &Value::Array(edits.clone()),
        range_after(&source, "VAR_GLOBAL\n  seed : DINT;", "seed"),
        "start",
    );
    assert_text_edit(
        &Value::Array(edits.clone()),
        range_after(&source, " : seed : DINT", "seed"),
        "start",
    );
    apply_tracked_rename(&mut client, &manifest, &response);

    let check = run_stc(&project.path, &["check"]);
    assert_success(&check, "the renamed access project checks");
    assert_eq!(client.finish(), 0);
}

#[test]
fn standard_rename_show_message_policy_matches_identity_risk() {
    let project = TestDir::new("standard-warnings");
    project.write("stc.toml", &project_manifest("app", ""));
    let main = project.write("src/main.st", BASIC_SOURCE);

    let mut client = LspClient::start(&project.manifest());
    client.initialize(&project.path);

    // A tracked global rename warns once about the missing history.
    let response = client.request(
        "textDocument/rename",
        rename_params(&main, position_of(BASIC_SOURCE, "Speed : DINT"), "Rate"),
    );
    assert!(!response["changes"].is_null(), "{response}");
    let message = client.wait_notification("window/showMessage", |message| {
        message["params"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("without recording history"))
    });
    assert_eq!(message["params"]["type"], json!(2));
    let text = message["params"]["message"].as_str().expect("warning");
    assert!(text.contains("Main.Speed -> Main.Rate"), "{text}");
    assert!(text.contains("[[rename]]"), "{text}");
    assert!(!project.path.join("stc.renames.toml").exists());

    // Parameters, locals, and case-only renames have no identity risk.
    for (needle, new_name) in [
        ("value : DINT", "input"),
        ("local_value : DINT", "scratch"),
        ("Speed : DINT", "speed"),
    ] {
        let response = client.request(
            "textDocument/rename",
            rename_params(&main, position_of(BASIC_SOURCE, needle), new_name),
        );
        assert!(!response["changes"].is_null(), "{needle}");
        client.assert_no_notification("window/showMessage");
    }
    assert_eq!(client.finish(), 0);

    // The selected configuration warns about the cold download instead.
    let external = TestDir::new("standard-cold");
    write_external_project(&external);
    let external_main = external.path.join("src").join("main.st");
    let mut client = LspClient::start(&external.manifest());
    client.initialize(&external.path);
    let response = client.request(
        "textDocument/rename",
        rename_params(
            &external_main,
            position_after(EXTERNAL_SOURCE, "CONFIGURATION MainConfig", "MainConfig"),
            "MainConfigX",
        ),
    );
    assert!(!response["changes"].is_null(), "{response}");
    let message = client.wait_notification("window/showMessage", |message| {
        message["params"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("cold"))
    });
    let text = message["params"]["message"].as_str().expect("warning");
    assert!(text.contains("selected configuration"), "{text}");
    client.assert_no_notification("window/showMessage");
    assert_eq!(client.finish(), 0);

    // A missing profile also produces one warning.
    let missing = TestDir::new("standard-profile");
    missing.write(
        "stc.toml",
        &project_manifest("app", "[io]\nconfig = \"io/missing.toml\""),
    );
    let missing_source = "\
PROGRAM Main
VAR
  counter : DINT;
END_VAR
  counter := counter + 1;
END_PROGRAM
";
    let missing_main = missing.write("src/main.st", missing_source);
    let mut client = LspClient::start(&missing.manifest());
    client.initialize(&missing.path);
    client.request(
        "textDocument/rename",
        rename_params(
            &missing_main,
            position_of(missing_source, "counter : DINT"),
            "count",
        ),
    );
    let history = client.wait_notification("window/showMessage", |message| {
        message["params"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("without recording history"))
    });
    assert_eq!(history["params"]["type"], json!(2));
    let profile = client.wait_notification("window/showMessage", |message| {
        message["params"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("profile is unavailable"))
    });
    let text = profile["params"]["message"].as_str().expect("warning");
    assert!(text.contains("missing.toml"), "{text}");
    client.assert_no_notification("window/showMessage");
    assert_eq!(client.finish(), 0);
}

#[test]
fn non_source_manifest_and_profile_requests_are_rejected() {
    let project = TestDir::new("non-source");
    write_external_project(&project);
    let main = project.path.join("src").join("main.st");
    let manifest = project.manifest();
    let profile = project.path.join("io").join("bench.toml");
    let outside = project.write(
        "outside.st",
        "PROGRAM Outside\nVAR\n  value : DINT;\nEND_VAR\nEND_PROGRAM\n",
    );

    let mut client = LspClient::start(&manifest);
    client.initialize(&project.path);

    for path in [
        &outside,
        &manifest,
        &profile,
        &main.with_file_name("missing.st"),
    ] {
        let prepared = client.request(
            "textDocument/prepareRename",
            document_position(path, json!({ "line": 0, "character": 0 })),
        );
        assert_eq!(prepared, Value::Null, "{}", path.display());
        let response = client.request_raw(
            "textDocument/rename",
            rename_params(path, json!({ "line": 0, "character": 0 }), "Renamed"),
        );
        assert_request_failed(&response, "not a project source");
    }

    let response = client.request_raw(
        "textDocument/codeAction",
        json!({
            "textDocument": { "uri": uri(&main) },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 0 },
            },
            "context": { "diagnostics": [] },
        }),
    );
    assert_eq!(response["error"]["code"], json!(-32601), "{response}");
    assert_eq!(client.finish(), 0);
}

#[test]
fn record_rename_rejects_missing_stale_used_and_bad_history() {
    let project = TestDir::new("record");
    project.write("stc.toml", &project_manifest("app", ""));
    let main = project.write("src/main.st", BASIC_SOURCE);
    let history_path = project.path.join("stc.renames.toml");

    let mut client = LspClient::start(&project.manifest());
    client.initialize(&project.path);

    let response = client.request_raw("stc/recordRename", json!({}));
    assert_request_failed(&response, "no pending rename");

    let first = client.request(
        "stc/rename",
        rename_params(&main, position_of(BASIC_SOURCE, "Speed : DINT"), "Rate"),
    );
    let first_token = first["token"].as_str().expect("first token").to_owned();
    // A missing token consumes the pending record.
    let response = client.request_raw("stc/recordRename", json!({}));
    assert_request_failed(&response, "requires a string `token`");
    let response = client.request_raw("stc/recordRename", json!({ "token": first_token }));
    assert_request_failed(&response, "no pending rename");

    // A newer rename invalidates the older token, and a failed record clears
    // the newer one too.
    let older = client.request(
        "stc/rename",
        rename_params(
            &main,
            position_after(BASIC_SOURCE, "FUNCTION AddStep", "AddStep"),
            "Advance",
        ),
    );
    let older_token = older["token"].as_str().expect("older token").to_owned();
    let newer = client.request(
        "stc/rename",
        rename_params(&main, position_of(BASIC_SOURCE, "Speed : DINT"), "Rate"),
    );
    let newer_token = newer["token"].as_str().expect("newer token").to_owned();
    let response = client.request_raw("stc/recordRename", json!({ "token": older_token }));
    assert_request_failed(&response, "unknown or stale");
    let response = client.request_raw("stc/recordRename", json!({ "token": newer_token }));
    assert_request_failed(&response, "no pending rename");

    // A successful record writes the canonical file once.
    let saved = client.request(
        "stc/rename",
        rename_params(&main, position_of(BASIC_SOURCE, "Speed : DINT"), "Rate"),
    );
    let saved_token = saved["token"].as_str().expect("saved token").to_owned();
    let recorded = client.request("stc/recordRename", json!({ "token": saved_token }));
    assert_eq!(recorded["recorded"], json!(true));
    assert_eq!(recorded["entries"], json!(1));
    assert_eq!(recorded["path"], json!(history_path.display().to_string()));
    let history = std::fs::read_to_string(&history_path).expect("history");
    assert_eq!(
        history,
        "schema = 1\n\n[[rename]]\nold = \"Main.Speed\"\nnew = \"Main.Rate\"\n"
    );
    let response = client.request_raw("stc/recordRename", json!({ "token": saved_token }));
    assert_request_failed(&response, "no pending rename");

    // A malformed history is reported with RequestFailed.
    std::fs::write(&history_path, "schema = 2\n").expect("seed malformed history");
    let saved = client.request(
        "stc/rename",
        rename_params(&main, position_of(BASIC_SOURCE, "Speed : DINT"), "Rate"),
    );
    let token = saved["token"].as_str().expect("malformed token").to_owned();
    let response = client.request_raw("stc/recordRename", json!({ "token": token }));
    assert_request_failed(&response, "STC-E-PROJECT-012");
    assert_eq!(
        std::fs::read_to_string(&history_path).expect("malformed history"),
        "schema = 2\n"
    );

    // A conflicting history is reported with RequestFailed.
    std::fs::write(
        &history_path,
        "schema = 1\n\n[[rename]]\nold = \"Main.Speed\"\nnew = \"Main.Rate\"\n",
    )
    .expect("seed conflicting history");
    let saved = client.request(
        "stc/rename",
        rename_params(&main, position_of(BASIC_SOURCE, "Speed : DINT"), "Rate"),
    );
    let token = saved["token"].as_str().expect("conflict token").to_owned();
    let response = client.request_raw("stc/recordRename", json!({ "token": token }));
    assert_request_failed(&response, "STC-E-PROJECT-013");
    assert_eq!(
        std::fs::read_to_string(&history_path).expect("conflicting history"),
        "schema = 1\n\n[[rename]]\nold = \"Main.Speed\"\nnew = \"Main.Rate\"\n"
    );

    assert_eq!(client.finish(), 0);
}

struct ParityStep {
    file: &'static str,
    anchor: &'static str,
    key: &'static str,
    new_name: &'static str,
    tracked_history: bool,
}

fn run_parity(name: &str, source: &TestDir, steps: &[ParityStep]) {
    let cli = TestDir::new(&format!("{name}-cli"));
    let server = TestDir::new(&format!("{name}-lsp"));
    copy_tree(&source.path, &cli.path);
    copy_tree(&source.path, &server.path);

    let mut client = LspClient::start(&server.manifest());
    client.initialize(&server.path);
    for step in steps {
        let output = run_stc(&cli.path, &["rename", step.key, step.new_name]);
        assert_success(
            &output,
            &format!("stc rename {} {}", step.key, step.new_name),
        );

        let path = server.path.join(step.file);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let offset = text
            .find(step.anchor)
            .unwrap_or_else(|| panic!("missing {:?} in {}", step.anchor, path.display()));
        // The CLI takes a dotted key; the LSP `newName` is the identifier.
        let lsp_new_name = step.new_name.rsplit('.').next().unwrap_or(step.new_name);
        let response = client.request(
            "stc/rename",
            rename_params(&path, position_at_offset(&text, offset), lsp_new_name),
        );
        assert_eq!(
            response["token"].is_string(),
            step.tracked_history,
            "token presence for `{}`: {response}",
            step.key
        );
        apply_tracked_rename(&mut client, &server.manifest(), &response);
    }
    assert_eq!(client.finish(), 0);

    let cli_tree = read_tree(&cli.path);
    let server_tree = read_tree(&server.path);
    let cli_paths: Vec<&String> = cli_tree.keys().collect();
    let server_paths: Vec<&String> = server_tree.keys().collect();
    assert_eq!(cli_paths, server_paths, "the file sets differ");
    for (path, cli_bytes) in &cli_tree {
        let server_bytes = server_tree
            .get(path)
            .unwrap_or_else(|| panic!("missing {path} from the LSP copy"));
        assert_eq!(
            cli_bytes, server_bytes,
            "`{path}` differs between `stc rename` and `stc/rename`"
        );
    }
}

#[test]
fn cli_and_tracked_rename_are_in_full_parity() {
    let external = TestDir::new("parity-external-source");
    write_external_project(&external);
    run_parity(
        "parity-external",
        &external,
        &[
            ParityStep {
                file: "src/main.st",
                anchor: "Short AT %QX0.0",
                key: "Short",
                new_name: "Brief",
                tracked_history: true,
            },
            ParityStep {
                file: "src/main.st",
                anchor: "Ns AT %IX0.1",
                key: "Ns",
                new_name: "Node",
                tracked_history: true,
            },
            ParityStep {
                file: "src/main.st",
                anchor: "Pkg AT %IX0.2",
                key: "Pkg",
                new_name: "Bundle",
                tracked_history: true,
            },
            ParityStep {
                file: "src/main.st",
                anchor: "Other AT %IX0.3",
                key: "Other",
                new_name: "OtherNode",
                tracked_history: true,
            },
            ParityStep {
                file: "src/main.st",
                anchor: "Field AT %QX0.1",
                key: "Main.Field",
                new_name: "Main.Out",
                tracked_history: true,
            },
            ParityStep {
                file: "src/main.st",
                anchor: "Label AT %QX0.2",
                key: "Main.Label",
                new_name: "Main.LABEL",
                tracked_history: false,
            },
            ParityStep {
                file: "tests/sample_test.st",
                anchor: "SampleTest : BOOL",
                key: "SampleTest",
                new_name: "SmokeTest",
                tracked_history: true,
            },
            ParityStep {
                file: "src/main.st",
                anchor: "value : DINT;",
                key: "AddStep.value",
                new_name: "AddStep.input",
                tracked_history: false,
            },
            ParityStep {
                file: "src/main.st",
                anchor: "local_value : DINT;",
                key: "AddStep.local_value",
                new_name: "AddStep.scratch",
                tracked_history: false,
            },
            ParityStep {
                file: "src/main.st",
                anchor: "MainConfig\nRESOURCE",
                key: "MainConfig",
                new_name: "MainConfigX",
                tracked_history: false,
            },
        ],
    );

    let basic = TestDir::new("parity-basic-source");
    basic.write("stc.toml", &project_manifest("app", ""));
    basic.write("src/main.st", BASIC_SOURCE);
    run_parity(
        "parity-basic",
        &basic,
        &[
            ParityStep {
                file: "src/main.st",
                anchor: "Speed : DINT",
                key: "Main.Speed",
                new_name: "Main.Rate",
                tracked_history: true,
            },
            ParityStep {
                file: "src/main.st",
                anchor: "value : DINT;",
                key: "AddStep.value",
                new_name: "AddStep.input",
                tracked_history: false,
            },
            ParityStep {
                file: "src/main.st",
                anchor: "local_value : DINT;",
                key: "AddStep.local_value",
                new_name: "AddStep.scratch",
                tracked_history: false,
            },
            ParityStep {
                file: "src/main.st",
                anchor: "Rate := Rate + 1",
                key: "Main.Rate",
                new_name: "Main.RATE",
                tracked_history: false,
            },
        ],
    );
}

#[test]
fn cli_and_lsp_write_byte_identical_history() {
    let source = TestDir::new("history-source");
    source.write("stc.toml", &project_manifest("app", ""));
    source.write("src/main.st", BASIC_SOURCE);
    let cli = TestDir::new("history-cli");
    let server = TestDir::new("history-lsp");
    copy_tree(&source.path, &cli.path);
    copy_tree(&source.path, &server.path);

    let output = run_stc(&cli.path, &["rename", "Main.Speed", "Main.Rate"]);
    assert_success(&output, "CLI speed rename");
    let output = run_stc(&cli.path, &["rename", "AddStep", "AddStepNext"]);
    assert_success(&output, "CLI function rename");

    let main = server.path.join("src").join("main.st");
    let mut client = LspClient::start(&server.manifest());
    client.initialize(&server.path);
    for (anchor, old, new_name, history_new) in [
        ("Speed : DINT", "Main.Speed", "Rate", "Main.Rate"),
        ("AddStep : DINT", "AddStep", "AddStepNext", "AddStepNext"),
    ] {
        let text = std::fs::read_to_string(&main).expect("read history source");
        let response = client.request(
            "stc/rename",
            rename_params(&main, position_of(&text, anchor), new_name),
        );
        assert_eq!(
            response["history"],
            json!([{ "old": old, "new": history_new }]),
            "{response}"
        );
        apply_tracked_rename(&mut client, &server.manifest(), &response);
    }
    assert_eq!(client.finish(), 0);

    let cli_history = std::fs::read(cli.path.join("stc.renames.toml")).expect("CLI history bytes");
    let lsp_history =
        std::fs::read(server.path.join("stc.renames.toml")).expect("LSP history bytes");
    assert_eq!(
        cli_history, lsp_history,
        "history files must be byte-identical"
    );
    let text = String::from_utf8(cli_history).expect("history text");
    assert!(text.contains("old = \"Main.Speed\""), "{text}");
    assert!(text.contains("new = \"Main.Rate\""), "{text}");
    assert!(text.contains("old = \"AddStep\""), "{text}");
    assert!(text.contains("new = \"AddStepNext\""), "{text}");
}
