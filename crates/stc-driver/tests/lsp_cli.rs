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

fn uri(path: &Path) -> String {
    stc_lsp::path_to_uri(path).expect("file URI").to_string()
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

/// Returns the UTF-16 LSP position of the first `needle` occurrence.
fn position_of(source: &str, needle: &str) -> Value {
    let offset = source
        .find(needle)
        .unwrap_or_else(|| panic!("missing {needle:?}"));
    let line_start = source[..offset].rfind('\n').map_or(0, |index| index + 1);
    let line = source[..offset].matches('\n').count();
    let character = source[line_start..offset].encode_utf16().count();
    json!({ "line": line, "character": character })
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
            "position": position_of(&main_source, "accumulator(increment"),
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
