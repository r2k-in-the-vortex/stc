//! The LSP session: lifecycle, document synchronization, and query dispatch.
//!
//! [`serve`] is transport-agnostic: a reader thread frames input and forwards
//! it over a channel, while the session loop receives messages (and waits with
//! a timeout between edits) on the calling thread. Standard input and output
//! are only wired up by [`run_stdio`].

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};
use stc_diag::{Diagnostic, Severity};
use stc_ide::{CompletionKind, DocumentSymbol, Snapshot, SymbolKind};
use stc_span::{FileId, Span};

use crate::documents::DocumentStore;
use crate::framing::{FrameError, FrameEvent, FrameReader, write_message};
use crate::position::{Position, PositionMap, Range, utf16_column};
use crate::protocol::{self, Incoming};
use crate::uri::{normalize_uri, path_to_uri, uri_to_path};

/// Default quiet period before a debounced analysis runs.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(200);

/// Server configuration.
#[derive(Clone, Debug)]
pub struct ServerOptions {
    /// `stc lsp --manifest-path`, which overrides client discovery.
    pub manifest_path: Option<PathBuf>,
    /// Quiet period after the last edit before analysis runs.
    pub debounce: Duration,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            manifest_path: None,
            debounce: DEFAULT_DEBOUNCE,
        }
    }
}

/// Returns the closed LSP capability set (I-7).
///
/// `publishDiagnostics` is not a standard server capability; it is advertised
/// as an informational marker that push diagnostics are the only diagnostic
/// mode. No pull diagnostics, semantic tokens, rename, code actions, range
/// formatting, call hierarchy, workspace symbols, or inlay hints are offered.
#[must_use]
pub fn capabilities() -> Value {
    json!({
        "positionEncoding": "utf-16",
        // Full synchronization (`change: 1`); open/close and save are
        // explicit so clients send `didSave` as well as `didOpen`/`didClose`.
        "textDocumentSync": {
            "openClose": true,
            "change": 1,
            "save": { "includeText": false },
        },
        "publishDiagnostics": { "relatedInformation": true },
        "documentSymbolProvider": true,
        "definitionProvider": true,
        "referencesProvider": true,
        "hoverProvider": true,
        "completionProvider": { "triggerCharacters": [".", ":"] },
        "documentFormattingProvider": true,
    })
}

/// Returns this server's `serverInfo` object.
#[must_use]
pub fn server_info() -> Value {
    json!({
        "name": "stc-lsp",
        "version": env!("CARGO_PKG_VERSION"),
    })
}

/// Runs the language server over the process standard streams.
///
/// Returns the process exit code: 0 after `shutdown` + `exit`, 1 when the
/// input closes or `exit` arrives without `shutdown`. The function returns at
/// end of input, so an interrupted or closed pipe never leaves the process
/// waiting.
#[must_use]
pub fn run_stdio(manifest_path: Option<&Path>) -> u8 {
    let options = ServerOptions {
        manifest_path: manifest_path.map(Path::to_path_buf),
        ..ServerOptions::default()
    };
    serve(std::io::stdin(), std::io::stdout(), options)
}

/// Runs one server session over `input` and `output`.
///
/// The reader thread only performs framing; the session itself runs on the
/// calling thread. `input` must be `'static` because the reader thread owns
/// it. Returns the exit code described by [`run_stdio`].
pub fn serve<R, W>(input: R, mut output: W, options: ServerOptions) -> u8
where
    R: Read + Send + 'static,
    W: Write,
{
    let (sender, receiver) = mpsc::channel::<Result<Vec<u8>, FrameError>>();
    let spawned = thread::Builder::new()
        .name("stc-lsp-reader".to_owned())
        .spawn(move || {
            let mut reader = FrameReader::new(std::io::BufReader::new(input));
            while let Some(event) = reader.next_event() {
                let item = match event {
                    FrameEvent::Message(body) => Ok(body),
                    FrameEvent::Malformed(error) => {
                        eprintln!("stc-lsp: framing: {error}");
                        Err(error)
                    }
                };
                if sender.send(item).is_err() {
                    break;
                }
            }
        });
    let mut reader_thread = match spawned {
        Ok(handle) => Some(handle),
        Err(error) => {
            eprintln!("stc-lsp: cannot start the reader thread: {error}");
            return 1;
        }
    };

    let mut session = Session::new(options);
    let mut debouncer = Debouncer::new(session.debounce());
    loop {
        if debouncer.due(Instant::now()) {
            session.analyze(&mut output);
            debouncer.clear();
            if session.io_failed() {
                return 1;
            }
            continue;
        }
        let received = if debouncer.is_pending() {
            match receiver.recv_timeout(debouncer.remaining(Instant::now())) {
                Ok(item) => Some(item),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    if let Some(handle) = reader_thread.take() {
                        let _ = handle.join();
                    }
                    return eof_exit_code(&session);
                }
            }
        } else {
            match receiver.recv() {
                Ok(item) => Some(item),
                Err(_) => {
                    if let Some(handle) = reader_thread.take() {
                        let _ = handle.join();
                    }
                    return eof_exit_code(&session);
                }
            }
        };
        let Some(item) = received else {
            continue;
        };
        match item {
            Ok(body) => match protocol::parse(&body) {
                Ok(message) => {
                    let effects = session.handle(message, &mut output);
                    if let Some(code) = effects.exit {
                        return code;
                    }
                    if effects.schedule_analysis {
                        debouncer.note(Instant::now());
                    }
                }
                Err(error) => {
                    eprintln!("stc-lsp: {}", error.message);
                    let protocol::ProtocolError { code, message, id } = error;
                    let value = protocol::error_response(id.as_ref(), code, message);
                    session.send(&mut output, value);
                }
            },
            Err(error) => eprintln!("stc-lsp: ignored malformed input: {error}"),
        }
        if session.io_failed() {
            return 1;
        }
    }
}

fn eof_exit_code(session: &Session) -> u8 {
    if session.shutdown { 0 } else { 1 }
}

/// Coalesces analysis requests into one run after a quiet period.
#[derive(Clone, Debug)]
pub struct Debouncer {
    delay: Duration,
    deadline: Option<Instant>,
}

impl Debouncer {
    /// Creates a debouncer with a quiet period of `delay`.
    #[must_use]
    pub fn new(delay: Duration) -> Self {
        Self {
            delay,
            deadline: None,
        }
    }

    /// Records activity, moving the deadline to `now + delay`.
    pub fn note(&mut self, now: Instant) {
        self.deadline = now.checked_add(self.delay);
    }

    /// Forgets pending work.
    pub fn clear(&mut self) {
        self.deadline = None;
    }

    /// Returns `true` when an analysis is pending.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.deadline.is_some()
    }

    /// Returns `true` when the quiet period has elapsed.
    #[must_use]
    pub fn due(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline)
    }

    /// Returns the time left before the deadline, or zero.
    #[must_use]
    pub fn remaining(&self, now: Instant) -> Duration {
        self.deadline.map_or(Duration::ZERO, |deadline| {
            deadline.saturating_duration_since(now)
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Effects {
    exit: Option<u8>,
    schedule_analysis: bool,
}

impl Effects {
    fn analyze() -> Self {
        Self {
            schedule_analysis: true,
            ..Self::default()
        }
    }

    fn exit(code: u8) -> Self {
        Self {
            exit: Some(code),
            schedule_analysis: false,
        }
    }
}

struct Session {
    options: ServerOptions,
    documents: DocumentStore,
    snapshot: Option<Snapshot>,
    manifest_path: Option<PathBuf>,
    initialized: bool,
    shutdown: bool,
    published: BTreeMap<String, Vec<Value>>,
    io_failed: bool,
}

impl Session {
    fn new(options: ServerOptions) -> Self {
        Self {
            options,
            documents: DocumentStore::new(),
            snapshot: None,
            manifest_path: None,
            initialized: false,
            shutdown: false,
            published: BTreeMap::new(),
            io_failed: false,
        }
    }

    fn debounce(&self) -> Duration {
        self.options.debounce
    }

    fn io_failed(&self) -> bool {
        self.io_failed
    }

    fn handle<W: Write>(&mut self, message: Incoming, output: &mut W) -> Effects {
        match message {
            Incoming::Response => Effects::default(),
            Incoming::Notification { method, params } => {
                self.notification(&method, &params, output)
            }
            Incoming::Request { id, method, params } => self.request(&id, &method, &params, output),
        }
    }

    fn notification<W: Write>(&mut self, method: &str, params: &Value, output: &mut W) -> Effects {
        match method {
            "initialized" => {
                if self.initialized {
                    self.publish_all(output);
                }
                Effects::default()
            }
            "exit" => Effects::exit(if self.shutdown { 0 } else { 1 }),
            "textDocument/didOpen" => self.did_open(params, output),
            "textDocument/didChange" => self.did_change(params, output),
            "textDocument/didSave" => self.did_save(params, output),
            "textDocument/didClose" => self.did_close(params, output),
            "workspace/didChangeWatchedFiles" => self.did_change_watched_files(params, output),
            "$/cancelRequest" | "$/setTrace" => Effects::default(),
            _ => Effects::default(),
        }
    }

    fn request<W: Write>(
        &mut self,
        id: &Value,
        method: &str,
        params: &Value,
        output: &mut W,
    ) -> Effects {
        if method == "initialize" {
            self.initialize(id, params, output);
            return Effects::default();
        }
        if method == "shutdown" {
            self.shutdown = true;
            self.send(output, protocol::response(id, Value::Null));
            return Effects::default();
        }
        if !self.initialized {
            self.send(
                output,
                protocol::error_response(
                    Some(id),
                    protocol::SERVER_NOT_INITIALIZED,
                    "stc-lsp: the server is not initialized",
                ),
            );
            return Effects::default();
        }
        if self.shutdown {
            self.send(
                output,
                protocol::error_response(
                    Some(id),
                    protocol::INVALID_REQUEST,
                    "stc-lsp: the server is shutting down",
                ),
            );
            return Effects::default();
        }
        match method {
            "textDocument/definition" => {
                let result = self.definition_result(params);
                self.send(output, protocol::response(id, result));
            }
            "textDocument/references" => {
                let result = self.references_result(params);
                self.send(output, protocol::response(id, result));
            }
            "textDocument/hover" => {
                let result = self.hover_result(params);
                self.send(output, protocol::response(id, result));
            }
            "textDocument/completion" => {
                let result = self.completion_result(params);
                self.send(output, protocol::response(id, result));
            }
            "textDocument/documentSymbol" => {
                let result = self.document_symbol_result(params);
                self.send(output, protocol::response(id, result));
            }
            "textDocument/formatting" => {
                let result = self.formatting_result(params);
                self.send(output, protocol::response(id, result));
            }
            other => self.send(
                output,
                protocol::error_response(
                    Some(id),
                    protocol::METHOD_NOT_FOUND,
                    format!("stc-lsp: unsupported method `{other}`"),
                ),
            ),
        }
        Effects::default()
    }

    fn initialize<W: Write>(&mut self, id: &Value, params: &Value, output: &mut W) {
        if self.initialized {
            self.send(
                output,
                protocol::error_response(
                    Some(id),
                    protocol::INVALID_REQUEST,
                    "stc-lsp: the server is already initialized",
                ),
            );
            return;
        }
        let Some(manifest) = self.project_manifest(params) else {
            let message = "stc-lsp: no stc.toml found; pass --manifest-path or open a folder \
                           that contains one"
                .to_owned();
            eprintln!("{message}");
            self.send(
                output,
                protocol::error_response(Some(id), protocol::INTERNAL_ERROR, message),
            );
            return;
        };
        match Snapshot::load(&manifest) {
            Ok(snapshot) => {
                self.manifest_path = Some(manifest.clone());
                self.snapshot = Some(snapshot);
                self.initialized = true;
                self.send(
                    output,
                    protocol::response(
                        id,
                        json!({
                            "capabilities": capabilities(),
                            "serverInfo": server_info(),
                        }),
                    ),
                );
                eprintln!("stc-lsp: serving `{}`", manifest.display());
            }
            Err(error) => {
                let detail = error
                    .diagnostics()
                    .iter()
                    .find(|diagnostic| diagnostic.is_error())
                    .map_or_else(
                        || error.to_string(),
                        |diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message),
                    );
                let message = format!(
                    "stc-lsp: cannot load project `{}`: {detail}",
                    manifest.display()
                );
                eprintln!("{message}");
                self.send(
                    output,
                    protocol::error_response(Some(id), protocol::INTERNAL_ERROR, message),
                );
            }
        }
    }

    /// Resolves the project manifest from `--manifest-path`, the client roots,
    /// or the process working directory, in that order.
    fn project_manifest(&self, params: &Value) -> Option<PathBuf> {
        if let Some(manifest) = &self.options.manifest_path {
            return Some(manifest.clone());
        }
        if let Some(path) = params
            .get("rootUri")
            .and_then(Value::as_str)
            .and_then(uri_to_path)
            && let Some(manifest) = find_manifest(&path)
        {
            return Some(manifest);
        }
        if let Some(folders) = params.get("workspaceFolders").and_then(Value::as_array) {
            for folder in folders {
                if let Some(path) = folder
                    .get("uri")
                    .and_then(Value::as_str)
                    .and_then(uri_to_path)
                    && let Some(manifest) = find_manifest(&path)
                {
                    return Some(manifest);
                }
            }
        }
        if let Some(path) = params.get("rootPath").and_then(Value::as_str)
            && let Some(manifest) = find_manifest(Path::new(path))
        {
            return Some(manifest);
        }
        find_manifest(&std::env::current_dir().ok()?)
    }

    fn did_open<W: Write>(&mut self, params: &Value, output: &mut W) -> Effects {
        let Some(document) = params.get("textDocument") else {
            return Effects::default();
        };
        let Some(uri) = document.get("uri").and_then(Value::as_str) else {
            return Effects::default();
        };
        let text = document
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let version = document.get("version").and_then(Value::as_i64).unwrap_or(0);
        if self.documents.open(uri, text.clone(), version).is_none() {
            return Effects::default();
        }
        let path = self
            .documents
            .get(uri)
            .expect("document was just opened")
            .path
            .clone();
        let file = self
            .snapshot
            .as_mut()
            .and_then(|snapshot| snapshot.set_overlay_parsed(&path, text));
        match file {
            Some(file) => {
                self.publish_parse(output, file);
                Effects::analyze()
            }
            None => {
                self.publish_empty(output, uri);
                Effects::default()
            }
        }
    }

    fn did_change<W: Write>(&mut self, params: &Value, output: &mut W) -> Effects {
        let Some(document) = params.get("textDocument") else {
            return Effects::default();
        };
        let Some(uri) = document.get("uri").and_then(Value::as_str) else {
            return Effects::default();
        };
        let Some(changes) = params.get("contentChanges").and_then(Value::as_array) else {
            return Effects::default();
        };
        let Some(text) = changes
            .last()
            .and_then(|change| change.get("text"))
            .and_then(Value::as_str)
        else {
            return Effects::default();
        };
        let version = document.get("version").and_then(Value::as_i64).unwrap_or(0);
        if self
            .documents
            .change(uri, text.to_owned(), version)
            .is_none()
        {
            return Effects::default();
        }
        let path = self
            .documents
            .get(uri)
            .expect("document was just changed")
            .path
            .clone();
        let file = self
            .snapshot
            .as_mut()
            .and_then(|snapshot| snapshot.set_overlay_parsed(&path, text));
        match file {
            Some(file) => {
                self.publish_parse(output, file);
                Effects::analyze()
            }
            None => {
                self.publish_empty(output, uri);
                Effects::default()
            }
        }
    }

    fn did_save<W: Write>(&mut self, params: &Value, output: &mut W) -> Effects {
        let Some(document) = params.get("textDocument") else {
            return Effects::default();
        };
        let Some(uri) = document.get("uri").and_then(Value::as_str) else {
            return Effects::default();
        };
        let text = params
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if self.documents.save(uri, text.clone()).is_none() {
            return Effects::default();
        }
        let path = self
            .documents
            .get(uri)
            .expect("document was just saved")
            .path
            .clone();
        if let Some(text) = text {
            let file = self
                .snapshot
                .as_mut()
                .and_then(|snapshot| snapshot.set_overlay_parsed(&path, text));
            if let Some(file) = file {
                self.publish_parse(output, file);
            }
        }
        Effects::analyze()
    }

    fn did_close<W: Write>(&mut self, params: &Value, output: &mut W) -> Effects {
        let Some(uri) = params
            .get("textDocument")
            .and_then(|document| document.get("uri"))
            .and_then(Value::as_str)
        else {
            return Effects::default();
        };
        let Some(document) = self.documents.close(uri) else {
            return Effects::default();
        };
        let restored = self
            .snapshot
            .as_mut()
            .and_then(|snapshot| snapshot.clear_overlay_parsed(&document.path));
        self.publish_empty(output, uri);
        if restored.is_some() {
            Effects::analyze()
        } else {
            Effects::default()
        }
    }

    fn did_change_watched_files<W: Write>(&mut self, params: &Value, output: &mut W) -> Effects {
        let relevant = params
            .get("changes")
            .and_then(Value::as_array)
            .is_none_or(|changes| {
                changes.iter().any(|change| {
                    change
                        .get("uri")
                        .and_then(Value::as_str)
                        .and_then(uri_to_path)
                        .is_some_and(|path| is_watched_path(&path))
                })
            });
        if relevant {
            self.reload_project(output);
        }
        Effects::default()
    }

    fn reload_project<W: Write>(&mut self, output: &mut W) {
        let Some(manifest) = self.manifest_path.clone() else {
            return;
        };
        match Snapshot::load(&manifest) {
            Ok(mut snapshot) => {
                let documents: Vec<(PathBuf, String)> = self
                    .documents
                    .documents()
                    .map(|document| (document.path.clone(), document.text.clone()))
                    .collect();
                for (path, text) in &documents {
                    snapshot.set_overlay_parsed(path, text.clone());
                }
                snapshot.reanalyze();
                self.snapshot = Some(snapshot);
                self.publish_all(output);
                eprintln!("stc-lsp: reloaded `{}`", manifest.display());
            }
            Err(error) => {
                let detail = error
                    .diagnostics()
                    .iter()
                    .find(|diagnostic| diagnostic.is_error())
                    .map_or_else(
                        || error.to_string(),
                        |diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message),
                    );
                eprintln!(
                    "stc-lsp: cannot reload `{}`: {detail}; keeping the previous analysis",
                    manifest.display()
                );
            }
        }
    }

    fn analyze<W: Write>(&mut self, output: &mut W) {
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.reanalyze();
        }
        self.publish_all(output);
    }

    fn publish_parse<W: Write>(&mut self, output: &mut W, file: FileId) {
        let diagnostics = self
            .snapshot
            .as_ref()
            .map_or_else(Vec::new, |snapshot| diagnostics_for(snapshot, file, false));
        self.publish(output, file, diagnostics);
    }

    fn publish_all<W: Write>(&mut self, output: &mut W) {
        let mut entries = Vec::new();
        if let Some(snapshot) = &self.snapshot {
            for info in snapshot.files() {
                entries.push((info.id, diagnostics_for(snapshot, info.id, true)));
            }
        }
        for (file, diagnostics) in entries {
            self.publish(output, file, diagnostics);
            if self.io_failed {
                return;
            }
        }
        self.clear_stale_diagnostics(output);
    }

    fn publish<W: Write>(&mut self, output: &mut W, file: FileId, diagnostics: Vec<Value>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let Some(info) = snapshot.file(file) else {
            return;
        };
        let Some(uri) = path_to_uri(&info.path) else {
            return;
        };
        let key = uri.to_string();
        let version = self.documents.get(&key).map(|document| document.version);
        if self.published.get(&key) == Some(&diagnostics) {
            return;
        }
        let mut params = Map::new();
        params.insert("uri".into(), json!(key));
        if let Some(version) = version {
            params.insert("version".into(), json!(version));
        }
        params.insert("diagnostics".into(), Value::Array(diagnostics.clone()));
        self.published.insert(key, diagnostics);
        self.send(
            output,
            protocol::notification("textDocument/publishDiagnostics", Value::Object(params)),
        );
    }

    fn publish_empty<W: Write>(&mut self, output: &mut W, uri: &str) {
        let key = normalize_uri(uri);
        if self.published.get(&key).is_some_and(Vec::is_empty) {
            return;
        }
        self.published.insert(key.clone(), Vec::new());
        self.send(
            output,
            protocol::notification(
                "textDocument/publishDiagnostics",
                json!({ "uri": key, "diagnostics": [] }),
            ),
        );
    }

    fn clear_stale_diagnostics<W: Write>(&mut self, output: &mut W) {
        let current: BTreeSet<String> =
            self.snapshot
                .as_ref()
                .map_or_else(BTreeSet::new, |snapshot| {
                    snapshot
                        .files()
                        .iter()
                        .filter_map(|info| path_to_uri(&info.path).map(|uri| uri.to_string()))
                        .collect()
                });
        let stale: Vec<String> = self
            .published
            .keys()
            .filter(|uri| !current.contains(*uri))
            .cloned()
            .collect();
        for uri in stale {
            self.published.remove(&uri);
            self.send(
                output,
                protocol::notification(
                    "textDocument/publishDiagnostics",
                    json!({ "uri": uri, "diagnostics": [] }),
                ),
            );
        }
    }

    fn send<W: Write>(&mut self, output: &mut W, message: Value) {
        if self.io_failed {
            return;
        }
        let body = match serde_json::to_vec(&message) {
            Ok(body) => body,
            Err(error) => {
                eprintln!("stc-lsp: cannot encode a message: {error}");
                self.io_failed = true;
                return;
            }
        };
        if let Err(error) = write_message(output, &body) {
            eprintln!("stc-lsp: cannot write to stdout: {error}");
            self.io_failed = true;
        }
    }

    fn query_file(&self, params: &Value) -> Option<FileId> {
        let uri = params.get("textDocument")?.get("uri")?.as_str()?;
        let path = uri_to_path(uri)?;
        self.snapshot.as_ref()?.file_id_for_path(path)
    }

    fn query_text(&self, file: FileId) -> Option<&str> {
        let snapshot = self.snapshot.as_ref()?;
        let info = snapshot.file(file)?;
        let uri = path_to_uri(&info.path)?;
        if let Some(document) = self.documents.get(uri.as_str()) {
            return Some(document.text.as_str());
        }
        snapshot.source(file)
    }

    fn query_offset(&self, params: &Value, file: FileId) -> Option<usize> {
        let position: Position = serde_json::from_value(params.get("position")?.clone()).ok()?;
        let text = self.query_text(file)?;
        Some(PositionMap::new(text).offset(position))
    }

    fn definition_result(&self, params: &Value) -> Value {
        let Some(file) = self.query_file(params) else {
            return Value::Null;
        };
        let Some(offset) = self.query_offset(params, file) else {
            return Value::Null;
        };
        let Some(snapshot) = &self.snapshot else {
            return Value::Null;
        };
        let Some(location) = snapshot.definition(file, offset) else {
            return Value::Null;
        };
        location_json(snapshot, location.span).unwrap_or(Value::Null)
    }

    fn references_result(&self, params: &Value) -> Value {
        let Some(file) = self.query_file(params) else {
            return json!([]);
        };
        let Some(offset) = self.query_offset(params, file) else {
            return json!([]);
        };
        let include_declaration = params
            .get("context")
            .and_then(|context| context.get("includeDeclaration"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let Some(snapshot) = &self.snapshot else {
            return json!([]);
        };
        let locations: Vec<Value> = snapshot
            .references(file, offset, include_declaration)
            .iter()
            .filter_map(|location| location_json(snapshot, location.span))
            .collect();
        json!(locations)
    }

    fn hover_result(&self, params: &Value) -> Value {
        let Some(file) = self.query_file(params) else {
            return Value::Null;
        };
        let Some(offset) = self.query_offset(params, file) else {
            return Value::Null;
        };
        let Some(snapshot) = &self.snapshot else {
            return Value::Null;
        };
        let Some(info) = snapshot.hover(file, offset) else {
            return Value::Null;
        };
        let mut object = Map::new();
        object.insert(
            "contents".into(),
            json!({ "kind": "plaintext", "value": info.contents }),
        );
        if let Some(range) = range_for_span(snapshot, info.span) {
            object.insert("range".into(), json!(range));
        }
        Value::Object(object)
    }

    fn completion_result(&self, params: &Value) -> Value {
        let items = match self.query_file(params) {
            Some(file) => match self.query_offset(params, file) {
                Some(offset) => {
                    let Some(snapshot) = &self.snapshot else {
                        return json!({ "isIncomplete": false, "items": [] });
                    };
                    snapshot
                        .completion(file, offset)
                        .into_iter()
                        .map(completion_json)
                        .collect()
                }
                None => Vec::new(),
            },
            None => Vec::new(),
        };
        json!({ "isIncomplete": false, "items": items })
    }

    fn document_symbol_result(&self, params: &Value) -> Value {
        let Some(file) = self.query_file(params) else {
            return json!([]);
        };
        let Some(snapshot) = &self.snapshot else {
            return json!([]);
        };
        Value::Array(
            snapshot
                .document_symbols(file)
                .iter()
                .filter_map(|symbol| symbol_json(snapshot, symbol))
                .collect(),
        )
    }

    fn formatting_result(&self, params: &Value) -> Value {
        let Some(file) = self.query_file(params) else {
            return Value::Null;
        };
        let Some(snapshot) = &self.snapshot else {
            return Value::Null;
        };
        let Some(result) = snapshot.formatting(file) else {
            return Value::Null;
        };
        if result.formatted.is_none() {
            return Value::Null;
        }
        let Some(edit) = result.edit() else {
            return json!([]);
        };
        let text = self.query_text(file).unwrap_or(result.source.as_str());
        let end = PositionMap::new(text).position(text.len());
        json!([{
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": end,
            },
            "newText": edit.new_text,
        }])
    }
}

fn is_watched_path(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("stc.toml"))
        || path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("st"))
}

fn find_manifest(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        if path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("stc.toml"))
        {
            return Some(path.to_path_buf());
        }
        let parent = path.parent()?;
        return parent
            .ancestors()
            .map(|directory| directory.join("stc.toml"))
            .find(|candidate| candidate.is_file());
    }
    path.ancestors()
        .map(|directory| directory.join("stc.toml"))
        .find(|candidate| candidate.is_file())
}

fn diagnostics_for(snapshot: &Snapshot, file: FileId, semantic: bool) -> Vec<Value> {
    let mut diagnostics: Vec<Value> = snapshot
        .parse_diagnostics(file)
        .iter()
        .filter_map(|diagnostic| diagnostic_json(snapshot, diagnostic))
        .collect();
    if semantic {
        diagnostics.extend(
            snapshot
                .semantic_diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.primary.file == file)
                .filter_map(|diagnostic| diagnostic_json(snapshot, diagnostic)),
        );
    }
    diagnostics
}

fn diagnostic_json(snapshot: &Snapshot, diagnostic: &Diagnostic) -> Option<Value> {
    let range = range_for_span(snapshot, diagnostic.primary)?;
    let severity = match diagnostic.severity {
        Severity::Error => 1,
        Severity::Warning => 2,
    };
    let mut object = Map::new();
    object.insert("range".into(), json!(range));
    object.insert("severity".into(), json!(severity));
    object.insert("code".into(), json!(diagnostic.code));
    object.insert("source".into(), json!("stc"));
    object.insert("message".into(), json!(diagnostic.message));
    let related: Vec<Value> = diagnostic
        .labels
        .iter()
        .filter_map(|label| {
            let info = snapshot.file(label.span.file)?;
            let uri = path_to_uri(&info.path)?;
            let range = range_for_span(snapshot, label.span)?;
            Some(json!({
                "location": { "uri": uri.to_string(), "range": range },
                "message": label.message,
            }))
        })
        .collect();
    if !related.is_empty() {
        object.insert("relatedInformation".into(), Value::Array(related));
    }
    Some(Value::Object(object))
}

fn location_json(snapshot: &Snapshot, span: Span) -> Option<Value> {
    let info = snapshot.file(span.file)?;
    let uri = path_to_uri(&info.path)?;
    let range = range_for_span(snapshot, span)?;
    Some(json!({ "uri": uri.to_string(), "range": range }))
}

/// Converts a compiler byte span to an LSP range.
///
/// The start and end pass through `stc-span` for the line/scalar-column
/// mapping, then the scalar column is widened to UTF-16 code units.
fn range_for_span(snapshot: &Snapshot, span: Span) -> Option<Range> {
    let source_file = snapshot.source_map().file(span.file)?;
    let start = snapshot.span_location(Span::empty(span.file, span.start))?;
    let end = snapshot.span_location(Span::empty(span.file, span.end))?;
    Some(Range::new(
        Position {
            line: start.line - 1,
            character: utf16_column(source_file.line_text(start.line)?, start.column),
        },
        Position {
            line: end.line - 1,
            character: utf16_column(source_file.line_text(end.line)?, end.column),
        },
    ))
}

fn completion_json(item: stc_ide::CompletionItem) -> Value {
    let mut object = Map::new();
    object.insert("label".into(), json!(item.label));
    object.insert("kind".into(), json!(completion_kind(item.kind)));
    if let Some(detail) = item.detail {
        object.insert("detail".into(), json!(detail));
    }
    Value::Object(object)
}

fn symbol_json(snapshot: &Snapshot, symbol: &DocumentSymbol) -> Option<Value> {
    let selection = range_for_span(snapshot, symbol.selection)?;
    let range = range_for_span(snapshot, symbol.range).unwrap_or(selection);
    let children: Vec<Value> = symbol
        .children
        .iter()
        .filter_map(|child| symbol_json(snapshot, child))
        .collect();
    let mut object = Map::new();
    object.insert("name".into(), json!(symbol.name));
    object.insert("kind".into(), json!(symbol_kind(symbol.kind)));
    if let Some(detail) = &symbol.detail {
        object.insert("detail".into(), json!(detail));
    }
    object.insert("range".into(), json!(range));
    object.insert("selectionRange".into(), json!(selection));
    object.insert("children".into(), Value::Array(children));
    Some(Value::Object(object))
}

fn symbol_kind(kind: SymbolKind) -> u32 {
    match kind {
        SymbolKind::Namespace => 3,
        SymbolKind::Function => 12,
        SymbolKind::FunctionBlock | SymbolKind::Program | SymbolKind::Class => 5,
        SymbolKind::Interface => 11,
        SymbolKind::Method => 6,
        SymbolKind::Type => 23,
        SymbolKind::Variable | SymbolKind::ProgramInstance => 13,
        SymbolKind::Constant => 14,
        SymbolKind::EnumValue => 22,
        SymbolKind::Configuration => 4,
        SymbolKind::Resource => 19,
        SymbolKind::Task => 24,
    }
}

fn completion_kind(kind: CompletionKind) -> u32 {
    match kind {
        CompletionKind::Keyword => 14,
        CompletionKind::Function => 3,
        CompletionKind::FunctionBlock | CompletionKind::Program | CompletionKind::Class => 7,
        CompletionKind::Interface => 8,
        CompletionKind::Method => 2,
        CompletionKind::Type => 22,
        CompletionKind::Variable | CompletionKind::Parameter | CompletionKind::Local => 6,
        CompletionKind::Field => 5,
        CompletionKind::EnumValue => 20,
        CompletionKind::Namespace | CompletionKind::Configuration => 9,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde_json::json;

    use super::*;

    fn manifest() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello/stc.toml")
    }

    fn frame(value: &Value, output: &mut Vec<u8>) {
        let body = serde_json::to_vec(value).expect("serialize");
        write_message(output, &body).expect("write frame");
    }

    fn frames(bytes: &[u8]) -> Vec<Value> {
        let mut reader = FrameReader::new(std::io::BufReader::new(Cursor::new(bytes)));
        let mut values = Vec::new();
        while let Some(event) = reader.next_event() {
            match event {
                FrameEvent::Message(body) => {
                    values.push(serde_json::from_slice(&body).expect("JSON response"));
                }
                FrameEvent::Malformed(error) => panic!("unexpected {error}"),
            }
        }
        values
    }

    #[test]
    fn debouncer_coalesces_activity() {
        let start = Instant::now();
        let mut debouncer = Debouncer::new(Duration::from_millis(200));
        assert!(!debouncer.is_pending());
        debouncer.note(start);
        assert!(debouncer.is_pending());
        assert!(!debouncer.due(start + Duration::from_millis(199)));
        assert!(debouncer.due(start + Duration::from_millis(200)));
        debouncer.note(start + Duration::from_millis(100));
        assert_eq!(
            debouncer.remaining(start + Duration::from_millis(150)),
            Duration::from_millis(150)
        );
        debouncer.clear();
        assert!(!debouncer.is_pending());
        assert!(!debouncer.due(start + Duration::from_secs(10)));
    }

    /// A reader that supplies one frame-sized chunk per `read` after a delay.
    /// It lets a test space messages in time without a real client.
    struct PacedReader {
        chunks: Vec<Vec<u8>>,
        delays: Vec<Duration>,
        index: usize,
    }

    impl std::io::Read for PacedReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.index >= self.chunks.len() {
                return Ok(0);
            }
            let delay = self.delays.get(self.index).copied().unwrap_or_default();
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
            let chunk = &mut self.chunks[self.index];
            let count = chunk.len().min(buffer.len());
            buffer[..count].copy_from_slice(&chunk[..count]);
            chunk.drain(..count);
            if chunk.is_empty() {
                self.index += 1;
            }
            Ok(count)
        }
    }

    fn frame_bytes(value: &Value) -> Vec<u8> {
        let mut output = Vec::new();
        frame(value, &mut output);
        output
    }

    #[test]
    fn rapid_changes_coalesce_into_one_analysis() {
        let source = "PROGRAM Main\nVAR x : DINT; END_VAR\nx := MissingOne;\nEND_PROGRAM\n";
        let changed = source.replace("MissingOne", "MissingTwo");
        let main_path = std::fs::canonicalize(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello/src/main.st"),
        )
        .expect("example source");
        let main_uri = path_to_uri(&main_path).expect("source URI").to_string();
        let chunks = vec![
            frame_bytes(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {},
            })),
            frame_bytes(&json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} })),
            frame_bytes(&json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": main_uri,
                        "languageId": "st",
                        "version": 1,
                        "text": source,
                    },
                },
            })),
            frame_bytes(&json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didChange",
                "params": {
                    "textDocument": { "uri": main_uri, "version": 2 },
                    "contentChanges": [ { "text": changed } ],
                },
            })),
            frame_bytes(&json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" })),
            frame_bytes(&json!({ "jsonrpc": "2.0", "method": "exit" })),
        ];
        let delays = vec![
            Duration::ZERO,
            Duration::from_millis(10),
            Duration::from_millis(10),
            Duration::from_millis(10),
            // Long enough for the debounced analysis to run once before the
            // shutdown request.
            Duration::from_millis(400),
            Duration::ZERO,
        ];
        let reader = PacedReader {
            chunks,
            delays,
            index: 0,
        };
        let mut output = Vec::new();
        let code = serve(
            reader,
            &mut output,
            ServerOptions {
                manifest_path: Some(manifest()),
                debounce: Duration::from_millis(200),
            },
        );
        assert_eq!(code, 0);
        let messages = frames(&output);
        let published: Vec<&Value> = messages
            .iter()
            .filter(|message| {
                message["method"] == json!("textDocument/publishDiagnostics")
                    && message["params"]["uri"].as_str() == Some(main_uri.as_str())
                    && !message["params"]["diagnostics"]
                        .as_array()
                        .is_none_or(Vec::is_empty)
            })
            .collect();
        assert_eq!(
            published.len(),
            1,
            "two changes inside the debounce window must produce one analysis: {messages:?}"
        );
        assert!(
            published[0]["params"]["diagnostics"]
                .as_array()
                .expect("diagnostics")
                .iter()
                .any(|diagnostic| diagnostic["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("MissingTwo"))),
            "the coalesced analysis uses the latest text"
        );
    }

    #[test]
    fn initialize_advertises_the_closed_capability_set() {
        let mut input = Vec::new();
        frame(
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {},
            }),
            &mut input,
        );
        frame(
            &json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" }),
            &mut input,
        );
        frame(&json!({ "jsonrpc": "2.0", "method": "exit" }), &mut input);
        let mut output = Vec::new();
        let code = serve(
            Cursor::new(input),
            &mut output,
            ServerOptions {
                manifest_path: Some(manifest()),
                debounce: Duration::from_millis(10),
            },
        );
        assert_eq!(code, 0);
        let messages = frames(&output);
        let response = messages
            .iter()
            .find(|message| message.get("id") == Some(&json!(1)))
            .expect("initialize response");
        let capabilities = &response["result"]["capabilities"];
        let keys: BTreeSet<&str> = capabilities
            .as_object()
            .expect("capabilities object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            BTreeSet::from([
                "positionEncoding",
                "textDocumentSync",
                "publishDiagnostics",
                "documentSymbolProvider",
                "definitionProvider",
                "referencesProvider",
                "hoverProvider",
                "completionProvider",
                "documentFormattingProvider",
            ]),
            "the advertised capability set is closed (I-7)"
        );
        assert_eq!(capabilities["textDocumentSync"]["openClose"], json!(true));
        assert_eq!(capabilities["textDocumentSync"]["change"], json!(1));
        assert_eq!(
            capabilities["textDocumentSync"]["save"]["includeText"],
            json!(false)
        );
        assert_eq!(capabilities["positionEncoding"], json!("utf-16"));
        assert_eq!(
            capabilities["publishDiagnostics"]["relatedInformation"],
            json!(true)
        );
        assert_eq!(capabilities["definitionProvider"], json!(true));
        assert_eq!(capabilities["referencesProvider"], json!(true));
        assert_eq!(capabilities["hoverProvider"], json!(true));
        assert_eq!(capabilities["documentSymbolProvider"], json!(true));
        assert_eq!(capabilities["documentFormattingProvider"], json!(true));
        assert_eq!(
            capabilities["completionProvider"]["triggerCharacters"],
            json!([".", ":"])
        );
        assert_eq!(response["result"]["serverInfo"]["name"], json!("stc-lsp"));
        assert_eq!(
            response["result"]["serverInfo"]["version"],
            json!(env!("CARGO_PKG_VERSION"))
        );
        let shutdown = messages
            .iter()
            .find(|message| message.get("id") == Some(&json!(2)))
            .expect("shutdown response");
        assert_eq!(shutdown["result"], Value::Null);
    }

    #[test]
    fn exit_without_shutdown_returns_one_and_malformed_input_is_answered() {
        let mut input = Vec::new();
        input.extend_from_slice(b"Content-Length: 3\r\n\r\nnot");
        frame(
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "textDocument/hover", "params": {} }),
            &mut input,
        );
        frame(&json!({ "jsonrpc": "2.0", "method": "exit" }), &mut input);
        let mut output = Vec::new();
        let code = serve(
            Cursor::new(input),
            &mut output,
            ServerOptions {
                manifest_path: Some(manifest()),
                debounce: Duration::from_millis(10),
            },
        );
        assert_eq!(code, 1);
        let messages = frames(&output);
        assert!(
            messages
                .iter()
                .any(|message| message["error"]["code"] == json!(protocol::PARSE_ERROR))
        );
        assert!(messages.iter().any(|message| {
            message["error"]["code"] == json!(protocol::SERVER_NOT_INITIALIZED)
        }));
    }

    #[test]
    fn a_missing_manifest_fails_initialize_readably() {
        let manifest = std::env::temp_dir()
            .join(format!("stc-lsp-missing-{}", std::process::id()))
            .join("missing")
            .join("stc.toml");
        let mut input = Vec::new();
        frame(
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {},
            }),
            &mut input,
        );
        frame(&json!({ "jsonrpc": "2.0", "method": "exit" }), &mut input);
        let mut output = Vec::new();
        let code = serve(
            Cursor::new(input),
            &mut output,
            ServerOptions {
                manifest_path: Some(manifest),
                debounce: Duration::from_millis(10),
            },
        );
        assert_eq!(code, 1);
        let messages = frames(&output);
        let response = messages
            .iter()
            .find(|message| message.get("id") == Some(&json!(1)))
            .expect("initialize response");
        assert_eq!(response["error"]["code"], json!(protocol::INTERNAL_ERROR));
        let message = response["error"]["message"].as_str().expect("message");
        assert!(message.contains("cannot load project"), "{message}");
    }
}
