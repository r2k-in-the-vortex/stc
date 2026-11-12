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
use stc_ide::{
    CompletionKind, DocumentSymbol, RenameEdit, RenameFile, RenameOnline, RenameResult,
    RenameWarning, Snapshot, SymbolKind,
};
use stc_manifest::{RenameEntry, append_renames_file};
use stc_span::{FileId, SourceMap, Span};

use crate::documents::DocumentStore;
use crate::framing::{FrameError, FrameEvent, FrameReader, write_message};
use crate::position::{Position, PositionMap, Range, utf16_column};
use crate::protocol::{self, Incoming};
use crate::uri::{normalize_uri, path_to_uri, uri_to_path};

/// Default quiet period before a debounced analysis runs.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(200);

/// Default lifetime of a pending tracked-rename token.
pub const DEFAULT_RENAME_TOKEN_TTL: Duration = Duration::from_secs(300);

/// Server configuration.
#[derive(Clone, Debug)]
pub struct ServerOptions {
    /// `stc lsp --manifest-path`, which overrides client discovery.
    pub manifest_path: Option<PathBuf>,
    /// Quiet period after the last edit before analysis runs.
    pub debounce: Duration,
    /// Lifetime of a pending tracked-rename token.
    pub rename_token_ttl: Duration,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            manifest_path: None,
            debounce: DEFAULT_DEBOUNCE,
            rename_token_ttl: DEFAULT_RENAME_TOKEN_TTL,
        }
    }
}

/// Returns the closed LSP capability set (I-7).
///
/// `publishDiagnostics` is not a standard server capability; it is advertised
/// as an informational marker that push diagnostics are the only diagnostic
/// mode. Rename is advertised with `prepareProvider`, and the private
/// tracked-rename handshake is marked in `experimental.stc.trackedRename`. No
/// pull diagnostics, semantic tokens, code actions, range formatting, call
/// hierarchy, workspace symbols, or inlay hints are offered.
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
        "renameProvider": { "prepareProvider": true },
        "experimental": { "stc": { "trackedRename": true } },
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
                    } else if !session.pending_analysis {
                        // A query flushed the analysis; the debounced run is
                        // no longer needed.
                        debouncer.clear();
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
    pending_analysis: bool,
    pending_rename: Option<PendingRename>,
    rename_counter: u64,
}

/// The single pending tracked-rename handshake for this connection.
struct PendingRename {
    token: String,
    entries: Vec<RenameEntry>,
    created: Instant,
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
            pending_analysis: false,
            pending_rename: None,
            rename_counter: 0,
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
        if matches!(
            method,
            "textDocument/definition"
                | "textDocument/references"
                | "textDocument/hover"
                | "textDocument/completion"
                | "textDocument/documentSymbol"
                | "textDocument/formatting"
                | "textDocument/prepareRename"
                | "textDocument/rename"
                | "stc/rename"
        ) {
            self.flush_analysis(output);
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
            "textDocument/prepareRename" => match self.prepare_rename_result(params) {
                Ok(result) => self.send(output, protocol::response(id, result)),
                Err(message) => self.send(
                    output,
                    protocol::error_response(Some(id), protocol::REQUEST_FAILED, message),
                ),
            },
            "textDocument/rename" => match self.rename_result(params, output) {
                Ok(result) => self.send(output, protocol::response(id, result)),
                Err(message) => self.send(
                    output,
                    protocol::error_response(Some(id), protocol::REQUEST_FAILED, message),
                ),
            },
            "stc/rename" => match self.tracked_rename_result(params) {
                Ok(result) => self.send(output, protocol::response(id, result)),
                Err(message) => self.send(
                    output,
                    protocol::error_response(Some(id), protocol::REQUEST_FAILED, message),
                ),
            },
            "stc/recordRename" => match self.record_rename_result(params) {
                Ok(result) => self.send(output, protocol::response(id, result)),
                Err(message) => self.send(
                    output,
                    protocol::error_response(Some(id), protocol::REQUEST_FAILED, message),
                ),
            },
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
                self.schedule_analysis()
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
                self.schedule_analysis()
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
        self.schedule_analysis()
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
            self.schedule_analysis()
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
        self.pending_analysis = false;
    }

    /// Marks parsed overlays as awaiting the project analysis.
    fn schedule_analysis(&mut self) -> Effects {
        self.pending_analysis = true;
        Effects::analyze()
    }

    /// Runs a pending debounced analysis before a query needs the snapshot.
    ///
    /// The document store and the parsed overlay are updated on every change,
    /// but the semantic analysis (and therefore every resolution) is debounced.
    /// A query answered during that window would interpret the client's
    /// position against text whose analysis has not caught up yet.
    fn flush_analysis<W: Write>(&mut self, output: &mut W) {
        if self.pending_analysis {
            self.analyze(output);
        }
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
        let Some(path) = uri_to_path(uri) else {
            eprintln!("stc-lsp: cannot convert document URI `{uri}` to a path");
            return None;
        };
        let file = self.snapshot.as_ref()?.file_id_for_path(&path);
        if file.is_none() {
            eprintln!(
                "stc-lsp: document `{uri}` (`{}`) is not a project source",
                path.display()
            );
        }
        file
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
        // Map against the text the snapshot analyzed, not the open document.
        // The two can differ (CRLF on disk vs LF in the editor, or an edit
        // inside the debounce window); mixing them silently targets another
        // token, so some identifiers resolve and others appear undefined.
        let text = self.snapshot.as_ref()?.source(file)?;
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

    /// Answers `textDocument/prepareRename`.
    ///
    /// A document outside the project source set, a non-identifier position,
    /// and an unresolved identifier all return `null`. A resolved but
    /// unsupported target is a `RequestFailed` error.
    fn prepare_rename_result(&self, params: &Value) -> Result<Value, String> {
        let Some(file) = self.query_file(params) else {
            return Ok(Value::Null);
        };
        let Some(offset) = self.query_offset(params, file) else {
            return Ok(Value::Null);
        };
        let Some(snapshot) = &self.snapshot else {
            return Ok(Value::Null);
        };
        let Some(placeholder) = snapshot
            .prepare_rename(file, offset)
            .map_err(|error| error.to_string())?
        else {
            return Ok(Value::Null);
        };
        let range = range_for_span(snapshot, placeholder.span)
            .ok_or_else(|| "stc-lsp: the rename target has no LSP range".to_owned())?;
        Ok(json!({
            "range": range,
            "placeholder": placeholder.name,
        }))
    }

    /// Answers the standard, history-free `textDocument/rename`.
    fn rename_result<W: Write>(&mut self, params: &Value, output: &mut W) -> Result<Value, String> {
        let Some(file) = self.query_file(params) else {
            return Err(
                "stc-lsp: the document is not a project source and cannot be renamed".to_owned(),
            );
        };
        let Some(offset) = self.query_offset(params, file) else {
            return Err("stc-lsp: the rename position is outside the document".to_owned());
        };
        let Some(new_name) = params.get("newName").and_then(Value::as_str) else {
            return Err("stc-lsp: rename requires a string `newName`".to_owned());
        };
        let Some(snapshot) = &self.snapshot else {
            return Err("stc-lsp: the project is not loaded".to_owned());
        };
        let result = snapshot
            .rename(file, offset, new_name)
            .map_err(|error| error.to_string())?;
        let changes = changes_json(snapshot, &result.edits)?;
        let messages = standard_warnings(&result);
        for message in messages {
            self.show_message(output, 2, message);
        }
        Ok(json!({ "changes": changes }))
    }

    /// Answers the private tracked `stc/rename` request without writing.
    fn tracked_rename_result(&mut self, params: &Value) -> Result<Value, String> {
        // A new rename invalidates any pending token.
        self.pending_rename = None;
        let Some(file) = self.query_file(params) else {
            return Err(
                "stc-lsp: the document is not a project source and cannot be renamed".to_owned(),
            );
        };
        let Some(offset) = self.query_offset(params, file) else {
            return Err("stc-lsp: the rename position is outside the document".to_owned());
        };
        let Some(new_name) = params.get("newName").and_then(Value::as_str) else {
            return Err("stc-lsp: rename requires a string `newName`".to_owned());
        };
        let Some(snapshot) = &self.snapshot else {
            return Err("stc-lsp: the project is not loaded".to_owned());
        };
        let result = snapshot
            .rename(file, offset, new_name)
            .map_err(|error| error.to_string())?;
        let changes = changes_json(snapshot, &result.edits)?;
        let history: Vec<Value> = result
            .history
            .iter()
            .map(|entry| json!({ "old": entry.old, "new": entry.new }))
            .collect();
        let online = match result.online {
            RenameOnline::Preserved => "preserved",
            RenameOnline::Cold(_) => "cold",
        };
        let mut response = json!({
            "edit": { "changes": changes },
            "history": history,
            "online": online,
            "warnings": tracked_warnings(&result),
        });
        if !result.history.is_empty() {
            self.rename_counter = self.rename_counter.wrapping_add(1);
            let token = format!("{}-{}", std::process::id(), self.rename_counter);
            self.pending_rename = Some(PendingRename {
                token: token.clone(),
                entries: result
                    .history
                    .iter()
                    .map(|entry| RenameEntry::new(entry.old.clone(), entry.new.clone()))
                    .collect(),
                created: Instant::now(),
            });
            response["token"] = json!(token);
        }
        Ok(response)
    }

    /// Answers `stc/recordRename` by appending the pending history entries.
    fn record_rename_result(&mut self, params: &Value) -> Result<Value, String> {
        let pending = self.pending_rename.take();
        let Some(pending) = pending else {
            return Err(
                "stc-lsp: no pending rename; the token is unknown, stale, or already used"
                    .to_owned(),
            );
        };
        let Some(token) = params.get("token").and_then(Value::as_str) else {
            return Err("stc-lsp: `stc/recordRename` requires a string `token`".to_owned());
        };
        if pending.token != token {
            return Err("stc-lsp: the rename token is unknown or stale".to_owned());
        }
        if pending.created.elapsed() > self.options.rename_token_ttl {
            return Err("stc-lsp: the rename token has expired; apply the rename again".to_owned());
        }
        let Some(snapshot) = &self.snapshot else {
            return Err("stc-lsp: the project is not loaded".to_owned());
        };
        let Some(root) = snapshot.manifest_path().parent() else {
            return Err("stc-lsp: the project manifest has no parent directory".to_owned());
        };
        let path = root.join("stc.renames.toml");
        match append_renames_file(&path, &pending.entries) {
            Ok(entries) => Ok(json!({
                "recorded": true,
                "entries": entries,
                "path": path.display().to_string(),
            })),
            Err(error) => Err(format!("{}: {}", error.code, error.message)),
        }
    }

    /// Sends one `window/showMessage` warning or information notification.
    fn show_message<W: Write>(&mut self, output: &mut W, message_type: u32, message: String) {
        self.send(
            output,
            protocol::notification(
                "window/showMessage",
                json!({ "type": message_type, "message": message }),
            ),
        );
    }
}

/// Returns the standard-method warnings for a successful rename.
///
/// The standard method never records history, so an identity-bearing target
/// gets one warning naming the missing mapping and the online-change risk. A
/// selected-configuration rename instead warns about the cold-download risk.
fn standard_warnings(result: &RenameResult) -> Vec<String> {
    let mut messages = Vec::new();
    if !result.history.is_empty() {
        let keys = result
            .history
            .iter()
            .map(|entry| format!("{} -> {}", entry.old, entry.new))
            .collect::<Vec<_>>()
            .join(", ");
        messages.push(format!(
            "stc-lsp: rename applied without recording history ({keys}); add the matching \
             [[rename]] entry to stc.renames.toml or the next online download may be cold and \
             reset retained state"
        ));
    } else if matches!(result.online, RenameOnline::Cold(_)) {
        messages.push(cold_warning());
    }
    messages.extend(result.warnings.iter().map(warning_message));
    messages
}

/// Returns the tracked-rename response warnings for a successful rename.
fn tracked_warnings(result: &RenameResult) -> Vec<String> {
    let mut messages = Vec::new();
    if matches!(result.online, RenameOnline::Cold(_)) {
        messages.push(cold_warning());
    }
    messages.extend(result.warnings.iter().map(warning_message));
    messages
}

fn cold_warning() -> String {
    "stc-lsp: renaming the selected configuration can change task-guard identities, so the next \
     online download may be cold and reset retained state"
        .to_owned()
}

fn warning_message(warning: &RenameWarning) -> String {
    match warning {
        RenameWarning::ProfileUnavailable(detail) => format!(
            "stc-lsp: the selected machine profile is unavailable ({detail}); the rename skipped \
             profile edits and the profile reference must be fixed manually"
        ),
    }
}

/// Converts one engine edit into its URI and LSP `TextEdit`.
///
/// The owning source map is used for the range conversion: source edits use
/// the project map, manifest edits the manifest map, and profile edits the
/// profile map.
fn rename_edit_json(snapshot: &Snapshot, edit: &RenameEdit) -> Result<(String, Value), String> {
    let (uri, sources, file) = match edit.file {
        RenameFile::Source(file) => {
            let info = snapshot.file(file).ok_or_else(|| {
                "stc-lsp: the rename edit names an unknown source file".to_owned()
            })?;
            let uri = path_to_uri(&info.path)
                .ok_or_else(|| "stc-lsp: cannot convert a source path to a URI".to_owned())?;
            (uri.to_string(), snapshot.source_map(), file)
        }
        RenameFile::Manifest => {
            let uri = path_to_uri(snapshot.manifest_path())
                .ok_or_else(|| "stc-lsp: cannot convert the manifest path to a URI".to_owned())?;
            (
                uri.to_string(),
                snapshot.manifest_sources(),
                snapshot.manifest_file(),
            )
        }
        RenameFile::Profile => {
            let path = snapshot
                .profile_path()
                .ok_or_else(|| "stc-lsp: the rename edit names a missing profile".to_owned())?;
            let file = snapshot.profile_file().ok_or_else(|| {
                "stc-lsp: the rename edit names a missing profile file".to_owned()
            })?;
            let uri = path_to_uri(path)
                .ok_or_else(|| "stc-lsp: cannot convert the profile path to a URI".to_owned())?;
            (uri.to_string(), snapshot.profile_sources(), file)
        }
    };
    let span = Span::new(file, edit.start, edit.end);
    let range = range_for_span_in(sources, span)
        .ok_or_else(|| "stc-lsp: a rename edit has no LSP range".to_owned())?;
    Ok((uri, json!({ "range": range, "newText": edit.new_text })))
}

/// Groups engine edits by file URI into a `WorkspaceEdit.changes` object.
fn changes_json(snapshot: &Snapshot, edits: &[RenameEdit]) -> Result<Value, String> {
    let mut changes = Map::new();
    for edit in edits {
        let (uri, text_edit) = rename_edit_json(snapshot, edit)?;
        let Value::Array(items) = changes
            .entry(uri)
            .or_insert_with(|| Value::Array(Vec::new()))
        else {
            return Err("stc-lsp: cannot build a rename edit map".to_owned());
        };
        items.push(text_edit);
    }
    Ok(Value::Object(changes))
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
    range_for_span_in(snapshot.source_map(), span)
}

/// Converts a byte span with its owning source map to an LSP range.
fn range_for_span_in(sources: &SourceMap, span: Span) -> Option<Range> {
    let source_file = sources.file(span.file)?;
    let start = sources.location(Span::empty(span.file, span.start));
    let end = sources.location(Span::empty(span.file, span.end));
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
                ..ServerOptions::default()
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
                ..ServerOptions::default()
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
                "renameProvider",
                "experimental",
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
        assert_eq!(
            capabilities["renameProvider"]["prepareProvider"],
            json!(true)
        );
        assert_eq!(
            capabilities["experimental"]["stc"]["trackedRename"],
            json!(true)
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
                ..ServerOptions::default()
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
                ..ServerOptions::default()
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

    /// A temporary project directory that removes itself after the test.
    struct TempProject {
        path: PathBuf,
    }

    impl TempProject {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("stc-lsp-unit-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create project directory");
            Self { path }
        }

        fn sample(name: &str) -> Self {
            let project = Self::new(name);
            copy_tree(&sample_workspace(), &project.path);
            project
        }

        fn path(&self, relative: &str) -> PathBuf {
            relative
                .split('/')
                .fold(self.path.clone(), |path, part| path.join(part))
        }

        fn manifest(&self) -> PathBuf {
            self.path.join("stc.toml")
        }

        fn write(&self, relative: &str, text: &str) {
            let path = self.path(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create parent directory");
            }
            std::fs::write(path, text).expect("write project file");
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn sample_workspace() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../editors/vscode/samples/workspace")
    }

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

    fn session(project: &TempProject) -> Session {
        session_with_ttl(project, DEFAULT_RENAME_TOKEN_TTL)
    }

    fn session_with_ttl(project: &TempProject, rename_token_ttl: Duration) -> Session {
        let manifest = project.manifest();
        let snapshot = Snapshot::load(&manifest).expect("snapshot");
        let mut session = Session::new(ServerOptions {
            manifest_path: Some(manifest.clone()),
            debounce: Duration::ZERO,
            rename_token_ttl,
        });
        session.manifest_path = Some(manifest);
        session.snapshot = Some(snapshot);
        session.initialized = true;
        session
    }

    fn position_of(source: &str, needle: &str) -> Value {
        let offset = source
            .find(needle)
            .unwrap_or_else(|| panic!("missing {needle:?}"));
        let line_start = source[..offset].rfind('\n').map_or(0, |index| index + 1);
        let line = source[..offset].matches('\n').count();
        let character = source[line_start..offset].encode_utf16().count();
        json!({ "line": line, "character": character })
    }

    fn uri(path: &Path) -> String {
        path_to_uri(path).expect("file URI").to_string()
    }

    fn rename_params(path: &Path, source: &str, needle: &str, new_name: &str) -> Value {
        json!({
            "textDocument": { "uri": uri(path) },
            "position": position_of(source, needle),
            "newName": new_name,
        })
    }

    fn show_messages(output: &[u8]) -> Vec<Value> {
        frames(output)
            .into_iter()
            .filter(|message| message["method"] == json!("window/showMessage"))
            .collect()
    }

    #[test]
    fn standard_rename_warns_for_history_and_cold_downloads() {
        let project = TempProject::sample("standard-warnings");
        let main = project.path("src/main.st");
        let smoke = project.path("tests/smoke.st");
        let main_source = std::fs::read_to_string(&main).expect("main source");
        let mut session = session(&project);

        let mut output = Vec::new();
        let params = rename_params(&main, &main_source, "AddStep(sum)", "Advance");
        let result = session
            .rename_result(&params, &mut output)
            .expect("standard rename");
        let changes = result["changes"].as_object().expect("changes");
        assert!(changes.contains_key(&uri(&main)), "{changes:?}");
        assert!(changes.contains_key(&uri(&smoke)), "{changes:?}");
        let messages = show_messages(&output);
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert_eq!(messages[0]["params"]["type"], json!(2));
        let message = messages[0]["params"]["message"]
            .as_str()
            .expect("warning text");
        assert!(message.contains("AddStep -> Advance"), "{message}");
        assert!(message.contains("stc.renames.toml"), "{message}");
        assert!(message.contains("cold"), "{message}");

        // A local has no history and no online-change risk: no warning.
        let mut output = Vec::new();
        let params = rename_params(&main, &main_source, "incremented : DINT;", "scratch");
        session
            .rename_result(&params, &mut output)
            .expect("local rename");
        assert!(
            show_messages(&output).is_empty(),
            "{:?}",
            show_messages(&output)
        );

        // A selected configuration may force a cold download: one warning.
        let mut output = Vec::new();
        let params = rename_params(&main, &main_source, "Sample\nRESOURCE", "SampleX");
        let result = session
            .rename_result(&params, &mut output)
            .expect("configuration rename");
        assert!(result["changes"].get(uri(&project.manifest())).is_some());
        let messages = show_messages(&output);
        assert_eq!(messages.len(), 1, "{messages:?}");
        let message = messages[0]["params"]["message"]
            .as_str()
            .expect("warning text");
        assert!(message.contains("configuration"), "{message}");
        assert!(message.contains("cold"), "{message}");
    }

    #[test]
    fn tracked_rename_tokens_are_single_use_and_invalidated() {
        let project = TempProject::sample("tracked-token");
        let main = project.path("src/main.st");
        let main_source = std::fs::read_to_string(&main).expect("main source");
        let mut session = session(&project);
        let params = rename_params(&main, &main_source, "AddStep(sum)", "Advance");

        let first = session
            .tracked_rename_result(&params)
            .expect("tracked rename");
        assert_eq!(
            first["history"],
            json!([{ "old": "AddStep", "new": "Advance" }])
        );
        assert_eq!(first["online"], json!("preserved"));
        assert_eq!(first["warnings"], json!([]));
        assert!(first["edit"]["changes"].is_object());
        let token = first["token"].as_str().expect("token").to_owned();

        let recorded = session
            .record_rename_result(&json!({ "token": token }))
            .expect("record history");
        assert_eq!(recorded["recorded"], json!(true));
        assert_eq!(recorded["entries"], json!(1));
        assert_eq!(
            PathBuf::from(recorded["path"].as_str().expect("path")),
            project.path("stc.renames.toml")
        );
        let history =
            std::fs::read_to_string(project.path("stc.renames.toml")).expect("history file");
        assert!(history.contains("old = \"AddStep\""), "{history}");
        assert!(history.contains("new = \"Advance\""), "{history}");

        let error = session
            .record_rename_result(&json!({ "token": token }))
            .expect_err("a token is single-use");
        assert!(error.contains("no pending rename"), "{error}");

        // A new rename replaces the pending record and invalidates the old
        // token, even when the edit is identical.
        let second = session
            .tracked_rename_result(&params)
            .expect("second tracked rename");
        let stale = second["token"].as_str().expect("second token").to_owned();
        let third = session
            .tracked_rename_result(&params)
            .expect("third tracked rename");
        let current = third["token"].as_str().expect("third token").to_owned();
        let error = session
            .record_rename_result(&json!({ "token": stale }))
            .expect_err("stale token");
        assert!(error.contains("unknown or stale"), "{error}");
        // A failed record clears the pending record.
        let error = session
            .record_rename_result(&json!({ "token": current }))
            .expect_err("cleared token");
        assert!(error.contains("no pending rename"), "{error}");
        let history =
            std::fs::read_to_string(project.path("stc.renames.toml")).expect("history file");
        assert_eq!(history.matches("[[rename]]").count(), 1, "{history}");
    }

    #[test]
    fn an_expired_tracked_rename_token_is_rejected() {
        let project = TempProject::sample("tracked-expiry");
        let main = project.path("src/main.st");
        let main_source = std::fs::read_to_string(&main).expect("main source");
        let mut session = session_with_ttl(&project, Duration::from_millis(1));
        let params = rename_params(&main, &main_source, "AddStep(sum)", "Advance");
        let response = session
            .tracked_rename_result(&params)
            .expect("tracked rename");
        let token = response["token"].as_str().expect("token").to_owned();
        std::thread::sleep(Duration::from_millis(20));
        let error = session
            .record_rename_result(&json!({ "token": token }))
            .expect_err("expired token");
        assert!(error.contains("expired"), "{error}");
        assert!(!project.path("stc.renames.toml").exists());
    }

    #[test]
    fn rename_edit_json_uses_the_owning_source_map() {
        let project = TempProject::sample("edit-json");
        let main = project.path("src/main.st");
        let main_source = std::fs::read_to_string(&main).expect("main source");
        let manifest_path = project.manifest();
        let manifest_text = std::fs::read_to_string(&manifest_path).expect("manifest text");
        let snapshot = Snapshot::load(&manifest_path).expect("snapshot");

        let file = snapshot.file_id_for_path(&main).expect("main file");
        let result = snapshot
            .rename(
                file,
                main_source.find("AddStep(sum)").expect("call"),
                "Advance",
            )
            .expect("function rename");
        let source_edit = result
            .edits
            .iter()
            .find(|edit| edit.file == RenameFile::Source(file))
            .expect("source edit");
        let (source_uri, source_value) =
            rename_edit_json(&snapshot, source_edit).expect("source conversion");
        assert_eq!(source_uri, uri(&main));
        assert_eq!(source_value["newText"], json!("Advance"));
        assert_eq!(
            source_value["range"]["start"],
            position_of(&main_source, "AddStep : DINT")
        );

        let configuration = main_source
            .find("CONFIGURATION Sample")
            .expect("configuration")
            + "CONFIGURATION ".len();
        let result = snapshot
            .rename(file, configuration, "SampleX")
            .expect("configuration rename");
        let manifest_edit = result
            .edits
            .iter()
            .find(|edit| edit.file == RenameFile::Manifest)
            .expect("manifest edit");
        let (manifest_uri, manifest_value) =
            rename_edit_json(&snapshot, manifest_edit).expect("manifest conversion");
        assert_eq!(manifest_uri, uri(&manifest_path));
        assert_eq!(manifest_value["newText"], json!("SampleX"));
        let mut expected_start = position_of(&manifest_text, "\"Sample\"");
        expected_start["character"] =
            json!(expected_start["character"].as_u64().expect("character") + 1);
        assert_eq!(manifest_value["range"]["start"], expected_start);

        // A profile edit is owned by the profile source map.
        let profile_project = TempProject::new("edit-json-profile");
        profile_project.write(
            "stc.toml",
            "schema = 1\n\n[package]\nname = \"app\"\nversion = \"0.1.0\"\n\
             language = \"stc-2013-oop-utf8-v1\"\n\n[project]\nsources = [\"src/**/*.st\"]\n\
             configuration = \"C\"\n\n[io]\nconfig = \"io/bench.toml\"\n\n\
             [[io.devices]]\nname = \"bench\"\nkind = \"demo\"\n\n\
             [[io.bindings]]\nvariable = \"Short\"\ndevice = \"bench\"\nchannel = 0\n",
        );
        let profile_text = "[[routes]]\nsource = { kind = \"plc-output\", variable = \"Short\" }\n\
                            sinks = [{ kind = \"plc-input\", variable = \"Other\" }]\n";
        profile_project.write("io/bench.toml", profile_text);
        profile_project.write(
            "src/main.st",
            "VAR_GLOBAL\n  Short AT %QX0.0 : BOOL;\n  Other AT %IX0.1 : BOOL;\nEND_VAR\n\n\
             PROGRAM Main\nVAR\n  x : BOOL;\nEND_VAR\n  x := Short;\nEND_PROGRAM\n\n\
             CONFIGURATION C\nRESOURCE R ON CPU\n  TASK t (INTERVAL := T#10ms);\n\
             PROGRAM inst WITH t : Main;\nEND_RESOURCE\nEND_CONFIGURATION\n",
        );
        let manifest_path = profile_project.manifest();
        let snapshot = Snapshot::load(&manifest_path).expect("profile snapshot");
        let main = profile_project.path("src/main.st");
        let source = std::fs::read_to_string(&main).expect("profile project source");
        let file = snapshot
            .file_id_for_path(&main)
            .expect("profile project file");
        let result = snapshot
            .rename(file, source.find("Short AT").expect("global"), "Brief")
            .expect("global rename");
        let profile_edit = result
            .edits
            .iter()
            .find(|edit| edit.file == RenameFile::Profile)
            .expect("profile edit");
        let (profile_uri, profile_value) =
            rename_edit_json(&snapshot, profile_edit).expect("profile conversion");
        assert_eq!(profile_uri, uri(&profile_project.path("io/bench.toml")));
        assert_eq!(profile_value["newText"], json!("Brief"));
        let mut expected = position_of(profile_text, "\"Short\"");
        expected["character"] = json!(expected["character"].as_u64().expect("character") + 1);
        assert_eq!(profile_value["range"]["start"], expected);
    }
}
