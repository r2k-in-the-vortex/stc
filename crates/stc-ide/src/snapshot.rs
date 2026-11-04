//! Project snapshots: loading, overlay texts, diagnostics, and positions.

use std::collections::BTreeMap;
#[cfg(windows)]
use std::ffi::OsString;
use std::fmt;
use std::ops::Range;
#[cfg(windows)]
use std::path::{Component, Prefix};
use std::path::{Path, PathBuf};

use stc_diag::{Diagnostic, DiagnosticSink};
use stc_hir::HirCrate;
use stc_lexer::tokenize_bytes;
use stc_manifest::{LoadOptions, SyntaxMode, load_project_readonly};
use stc_parser::{ParseOptions, ParsedFile, parse};
use stc_sema::{Analysis, PackageSession, ProjectSession, Res};
use stc_span::{FileId, SourceMap, Span};
use stc_syntax::NodeId;

use crate::query::DeclarationIndex;

/// Removes the Windows verbatim prefix from `path`.
///
/// `std::fs::canonicalize` returns `\\?\C:\...` or `\\?\UNC\...` on Windows,
/// while project sources carry the normal spelling. Comparing the two
/// directly makes an editor buffer opened through a verbatim path look like it
/// belongs to a different file (and therefore outside the project).
#[cfg(windows)]
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = path.components();
    match components.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(disk) => {
                let mut base = OsString::from(format!("{}:", disk as char));
                base.push(components.as_path());
                PathBuf::from(base)
            }
            Prefix::VerbatimUNC(server, share) => {
                let mut base = OsString::from(r"\\");
                base.push(server);
                base.push(r"\");
                base.push(share);
                base.push(components.as_path());
                PathBuf::from(base)
            }
            _ => path.to_path_buf(),
        },
        _ => path.to_path_buf(),
    }
}

#[cfg(not(windows))]
fn normalize_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

/// One project source file known to a [`Snapshot`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFileInfo {
    /// Compiler file identifier used by every span and diagnostic.
    pub id: FileId,
    /// Package-qualified display path, for example `motor/src/motor.st`.
    pub display: String,
    /// Absolute path on disk.
    pub path: PathBuf,
    /// Owning package name.
    pub package: String,
    /// Whether this file belongs to the root package's `[test]` source set.
    pub is_test: bool,
    pub(crate) package_index: usize,
}

/// A resolved source position.
///
/// `line` and `column` are 1-based and the column counts Unicode scalar
/// values, matching [`stc_span::SourceMap`]; `span` is the byte range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceLocation {
    /// File containing the position.
    pub file: FileId,
    /// Byte range of the resolved identifier or declaration name.
    pub span: Span,
    /// 1-based line number.
    pub line: u32,
    /// 1-based column counted in Unicode scalar values.
    pub column: u32,
}

/// A whole-document replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    /// Inclusive byte offset of the replaced range.
    pub start: u32,
    /// Exclusive byte offset of the replaced range.
    pub end: u32,
    /// Replacement text.
    pub new_text: String,
}

/// The result of formatting one file.
#[derive(Clone, Debug)]
pub struct FormatResult {
    /// The current decoded source text.
    pub source: String,
    /// The formatted text, or `None` when syntax errors prevent formatting.
    pub formatted: Option<String>,
    /// Formatter diagnostics; a refused document reports `STC-E-FMT-001`.
    pub diagnostics: Vec<Diagnostic>,
}

impl FormatResult {
    /// Returns the full-document edit, or `None` when formatting failed or the
    /// document is already formatted.
    #[must_use]
    pub fn edit(&self) -> Option<TextEdit> {
        let formatted = self.formatted.as_ref()?;
        (formatted != &self.source).then(|| TextEdit {
            start: 0,
            end: u32::try_from(self.source.len()).expect("source length fits in a span"),
            new_text: formatted.clone(),
        })
    }
}

/// Project loading failed before a snapshot could be built.
#[derive(Debug)]
pub struct SnapshotError {
    diagnostics: Vec<Diagnostic>,
    sources: SourceMap,
}

impl SnapshotError {
    fn new(diagnostics: Vec<Diagnostic>, sources: SourceMap) -> Self {
        Self {
            diagnostics,
            sources,
        }
    }

    /// Returns the manifest, dependency, and lockfile diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Returns the source map used by the diagnostics' spans.
    #[must_use]
    pub fn sources(&self) -> &SourceMap {
        &self.sources
    }
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.is_error())
        {
            Some(diagnostic) => write!(formatter, "{}: {}", diagnostic.code, diagnostic.message),
            None => write!(
                formatter,
                "project loading failed with {} diagnostic(s)",
                self.diagnostics.len()
            ),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// One package in the analyzed graph.
struct PackageData {
    name: String,
    namespace: String,
    dependencies: Vec<usize>,
    is_root: bool,
    files: Range<usize>,
}

/// An owned, queryable analysis of one project.
///
/// The snapshot parses every production source of every package in dependency
/// order and then the root package's test sources, mirroring the driver's
/// project pipeline. It analyzes with `test_build: true` and the manifest's
/// `[project].configuration` and `stdlib`, so editor results match `stc test`.
pub struct Snapshot {
    manifest_path: PathBuf,
    packages: Vec<PackageData>,
    files: Vec<SourceFileInfo>,
    parsed: Vec<ParsedFile>,
    texts: Vec<String>,
    file_diagnostics: Vec<Vec<Diagnostic>>,
    analysis: Analysis,
    analysis_diagnostics: Vec<Diagnostic>,
    manifest_diagnostics: Vec<Diagnostic>,
    manifest_sources: SourceMap,
    source_map: SourceMap,
    overlays: BTreeMap<FileId, String>,
    declarations: DeclarationIndex,
    test_range: Range<usize>,
    relaxed: bool,
    configuration: Option<String>,
    stdlib: bool,
}

impl fmt::Debug for Snapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Snapshot")
            .field("manifest_path", &self.manifest_path)
            .field("files", &self.files.len())
            .field("overlays", &self.overlays.len())
            .finish_non_exhaustive()
    }
}

impl Snapshot {
    /// Loads `manifest_path` (default `stc.toml` when the caller passes a
    /// directory) and builds the analysis.
    ///
    /// A directory argument is interpreted as `<dir>/stc.toml` by the loader.
    /// The lockfile is never created or updated.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError`] with manifest, dependency, source-discovery,
    /// or lockfile diagnostics when the project cannot be loaded.
    pub fn load(manifest_path: impl AsRef<Path>) -> Result<Self, SnapshotError> {
        let manifest_path = manifest_path.as_ref().to_path_buf();
        let graph = load_project_readonly(&manifest_path, &LoadOptions::default())
            .map_err(|error| SnapshotError::new(error.diagnostics, error.sources))?;
        let root = graph.root_package();
        let relaxed = root.manifest.build.syntax == SyntaxMode::Relaxed;
        let configuration = root.manifest.project.configuration.clone();
        let stdlib = root.manifest.project.stdlib;
        let root_index = graph.root.0;
        let root_test_count = root.test_sources.len();
        let manifest_diagnostics = graph.warnings.clone();

        let mut packages = Vec::with_capacity(graph.packages.len());
        let mut files = Vec::new();
        let mut parsed = Vec::new();
        let mut texts = Vec::new();
        let mut file_diagnostics = Vec::new();
        for (package_index, package) in graph.packages.iter().enumerate() {
            let start = files.len();
            let production_count = package.sources.len();
            let mut sources: Vec<&stc_manifest::SourcePath> = package.sources.iter().collect();
            if package.is_root {
                sources.extend(package.test_sources.iter());
            }
            for (offset, source) in sources.into_iter().enumerate() {
                let file = FileId::new(
                    u32::try_from(files.len()).expect("more than u32::MAX project sources"),
                );
                let display = format!("{}/{}", package.name, source.relative);
                let is_test = offset >= production_count;
                let mut sink = DiagnosticSink::new();
                let (parsed_file, text) = match std::fs::read(&source.path) {
                    Ok(bytes) => parse_source(file, &bytes, relaxed, &mut sink),
                    Err(error) => {
                        sink.error(
                            "STC-E-BUILD-002",
                            format!("cannot read `{}`: {error}", source.path.to_string_lossy()),
                            Span::new(file, 0, 0),
                        );
                        parse_source(file, b"", relaxed, &mut sink)
                    }
                };
                files.push(SourceFileInfo {
                    id: file,
                    display,
                    path: source.path.clone(),
                    package: package.name.clone(),
                    is_test,
                    package_index,
                });
                parsed.push(parsed_file);
                texts.push(text);
                file_diagnostics.push(sink.into_diagnostics());
            }
            packages.push(PackageData {
                name: package.name.clone(),
                namespace: package.namespace.clone(),
                dependencies: package
                    .dependencies
                    .iter()
                    .map(|dependency| dependency.package.0)
                    .collect(),
                is_root: package.is_root,
                files: start..files.len(),
            });
        }

        let root_files = packages[root_index].files.clone();
        debug_assert!(root_test_count <= root_files.len());
        let test_range = (root_files.end - root_test_count)..root_files.end;
        let manifest_sources = graph.sources;
        let (analysis, analysis_diagnostics) = run_analysis(
            &packages,
            &parsed,
            test_range.clone(),
            &configuration,
            stdlib,
        );
        let declarations = DeclarationIndex::build(&analysis.hir, &parsed);
        let mut snapshot = Self {
            manifest_path,
            packages,
            files,
            parsed,
            texts,
            file_diagnostics,
            analysis,
            analysis_diagnostics,
            manifest_diagnostics,
            manifest_sources,
            source_map: SourceMap::new(),
            overlays: BTreeMap::new(),
            declarations,
            test_range,
            relaxed,
            configuration,
            stdlib,
        };
        snapshot.rebuild_source_map();
        Ok(snapshot)
    }

    /// Returns the manifest path this snapshot was loaded from.
    #[must_use]
    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    /// Returns every source file in analysis order (dependencies first, then
    /// the root package's production and test sources).
    #[must_use]
    pub fn files(&self) -> &[SourceFileInfo] {
        &self.files
    }

    /// Returns one file's information.
    #[must_use]
    pub fn file(&self, file: FileId) -> Option<&SourceFileInfo> {
        self.files.get(file.index())
    }

    /// Looks up a file by its display path (`package/relative/path.st`).
    #[must_use]
    pub fn file_id(&self, display: &str) -> Option<FileId> {
        self.files
            .iter()
            .find(|info| info.display == display)
            .map(|info| info.id)
    }

    /// Looks up a project source by path.
    ///
    /// The path is canonicalized when it exists, so a non-normalized or
    /// differently-cased absolute path still resolves. Paths outside the
    /// project return `None`.
    #[must_use]
    pub fn file_id_for_path(&self, path: impl AsRef<Path>) -> Option<FileId> {
        let path = normalize_path(path.as_ref());
        let canonical = std::fs::canonicalize(&path)
            .ok()
            .map(|canonical| normalize_path(&canonical));
        self.files
            .iter()
            .find(|info| {
                let info_path = normalize_path(&info.path);
                info_path == path
                    || canonical
                        .as_ref()
                        .is_some_and(|canonical| canonical == &info_path)
            })
            .map(|info| info.id)
    }

    /// Returns the current text of a file (the overlay text when one is set).
    #[must_use]
    pub fn source(&self, file: FileId) -> Option<&str> {
        self.source_map.file(file).map(stc_span::SourceFile::text)
    }

    /// Returns the source map that owns the current file texts.
    #[must_use]
    pub fn source_map(&self) -> &SourceMap {
        &self.source_map
    }

    /// Converts a byte offset to a 1-based line/column position.
    ///
    /// Offsets are clamped to the source length, and offsets inside a UTF-8
    /// scalar are floored to the start of that scalar by the source map.
    #[must_use]
    pub fn location(&self, file: FileId, offset: usize) -> Option<SourceLocation> {
        let source = self.source(file)?;
        let offset = u32::try_from(offset.min(source.len())).ok()?;
        self.span_location(Span::empty(file, offset))
    }

    /// Converts a byte span to a [`SourceLocation`].
    #[must_use]
    pub fn span_location(&self, span: Span) -> Option<SourceLocation> {
        self.source(span.file)?;
        let location = self.source_map.location(span);
        Some(SourceLocation {
            file: span.file,
            span,
            line: location.line,
            column: location.column,
        })
    }

    /// Returns the lexer and parser diagnostics recorded for a file.
    ///
    /// These are produced when the file is parsed and re-produced when an
    /// overlay replaces it.
    #[must_use]
    pub fn parse_diagnostics(&self, file: FileId) -> &[Diagnostic] {
        self.file_diagnostics
            .get(file.index())
            .map_or(&[], Vec::as_slice)
    }

    /// Returns the analyzer diagnostics of the last analysis.
    ///
    /// Callers that publish parse-first should send [`Snapshot::parse_diagnostics`]
    /// before these.
    #[must_use]
    pub fn semantic_diagnostics(&self) -> &[Diagnostic] {
        &self.analysis_diagnostics
    }

    /// Returns every diagnostic whose primary span is in `file`, parse
    /// diagnostics first.
    #[must_use]
    pub fn diagnostics(&self, file: FileId) -> Vec<&Diagnostic> {
        let mut diagnostics: Vec<&Diagnostic> = self.parse_diagnostics(file).iter().collect();
        diagnostics.extend(
            self.analysis_diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.primary.file == file),
        );
        diagnostics
    }

    /// Returns non-fatal manifest warnings collected by the loader.
    #[must_use]
    pub fn manifest_diagnostics(&self) -> &[Diagnostic] {
        &self.manifest_diagnostics
    }

    /// Returns the source map that owns manifest warning spans.
    #[must_use]
    pub fn manifest_sources(&self) -> &SourceMap {
        &self.manifest_sources
    }

    /// Returns `true` when an unsaved overlay is active for `file`.
    #[must_use]
    pub fn is_overlaid(&self, file: FileId) -> bool {
        self.overlays.contains_key(&file)
    }

    /// Replaces a project file's text with an unsaved buffer and re-runs the
    /// project analysis. Nothing is written to disk.
    ///
    /// Returns `None` when `path` is not a project source. The overlay is
    /// remembered until [`Snapshot::clear_overlay`]; applying another overlay
    /// to the same file replaces it.
    pub fn set_overlay(
        &mut self,
        path: impl AsRef<Path>,
        text: impl Into<String>,
    ) -> Option<FileId> {
        let file = self.set_overlay_parsed(path, text)?;
        self.refresh();
        Some(file)
    }

    /// Replaces a project file's text with an unsaved buffer and re-parses it
    /// without re-running the project analysis.
    ///
    /// This is the parse-first half of [`Snapshot::set_overlay`] for editors:
    /// [`Snapshot::parse_diagnostics`] reflects the new text immediately while
    /// [`Snapshot::semantic_diagnostics`] and the query answers still describe
    /// the previous analysis until [`Snapshot::reanalyze`] runs. Nothing is
    /// written to disk. Returns `None` when `path` is not a project source.
    pub fn set_overlay_parsed(
        &mut self,
        path: impl AsRef<Path>,
        text: impl Into<String>,
    ) -> Option<FileId> {
        let file = self.file_id_for_path(path)?;
        let text = text.into();
        let index = file.index();
        let mut sink = DiagnosticSink::new();
        let (parsed, decoded) = parse_source(file, text.as_bytes(), self.relaxed, &mut sink);
        self.parsed[index] = parsed;
        self.texts[index] = decoded;
        self.file_diagnostics[index] = sink.into_diagnostics();
        self.overlays.insert(file, text);
        self.rebuild_source_map();
        Some(file)
    }

    /// Re-runs the project analysis over the current parsed texts and rebuilds
    /// the declaration index.
    ///
    /// Call after one or more [`Snapshot::set_overlay_parsed`] or
    /// [`Snapshot::clear_overlay_parsed`] updates when semantic diagnostics and
    /// queries should reflect them. Files without an overlay keep their current
    /// text.
    pub fn reanalyze(&mut self) {
        self.refresh();
    }

    /// Discards an unsaved overlay and restores the file from disk.
    ///
    /// Returns `None` when `path` is not a project source. When the file cannot
    /// be read back (for example because it was deleted), the snapshot keeps an
    /// empty text and records `STC-E-BUILD-002` for the file.
    pub fn clear_overlay(&mut self, path: impl AsRef<Path>) -> Option<FileId> {
        let file = self.file_id_for_path(&path)?;
        if !self.overlays.contains_key(&file) {
            return Some(file);
        }
        let file = self.clear_overlay_parsed(path)?;
        self.refresh();
        Some(file)
    }

    /// Discards an unsaved overlay, restores the file text from disk, and
    /// re-parses it without re-running the project analysis.
    ///
    /// The same read failure rules as [`Snapshot::clear_overlay`] apply. A file
    /// without an overlay is left unchanged and returned.
    pub fn clear_overlay_parsed(&mut self, path: impl AsRef<Path>) -> Option<FileId> {
        let file = self.file_id_for_path(path)?;
        if self.overlays.remove(&file).is_none() {
            return Some(file);
        }
        let index = file.index();
        let mut sink = DiagnosticSink::new();
        let (parsed, text) = match std::fs::read(&self.files[index].path) {
            Ok(bytes) => parse_source(file, &bytes, self.relaxed, &mut sink),
            Err(error) => {
                sink.error(
                    "STC-E-BUILD-002",
                    format!(
                        "cannot read `{}`: {error}",
                        self.files[index].path.to_string_lossy()
                    ),
                    Span::new(file, 0, 0),
                );
                parse_source(file, b"", self.relaxed, &mut sink)
            }
        };
        self.parsed[index] = parsed;
        self.texts[index] = text;
        self.file_diagnostics[index] = sink.into_diagnostics();
        self.rebuild_source_map();
        Some(file)
    }

    /// Formats the current text of a file with the fixed `stc-fmt` style.
    ///
    /// A file with syntax errors reports `STC-E-FMT-001` and leaves `formatted`
    /// empty, so [`FormatResult::edit`] returns `None`.
    #[must_use]
    pub fn formatting(&self, file: FileId) -> Option<FormatResult> {
        let source = self.source(file)?;
        let mut sink = DiagnosticSink::new();
        let outcome = stc_fmt::format_bytes(file, source.as_bytes(), &mut sink);
        Some(FormatResult {
            source: outcome.source,
            formatted: outcome.formatted,
            diagnostics: sink.into_diagnostics(),
        })
    }

    pub(crate) fn hir(&self) -> &HirCrate {
        &self.analysis.hir
    }

    pub(crate) fn resolutions(&self) -> &BTreeMap<(FileId, NodeId), Res> {
        &self.analysis.resolutions
    }

    pub(crate) fn parsed(&self, file: FileId) -> Option<&ParsedFile> {
        self.parsed.get(file.index())
    }

    pub(crate) fn declarations(&self) -> &DeclarationIndex {
        &self.declarations
    }

    /// Returns the source spelling of `symbol` declared at `span`.
    ///
    /// The HIR interner is case-insensitive and keeps the first spelling it
    /// saw, so `drive` and `Drive` share one symbol. Editors need the spelling
    /// written at the declaration, which the token text provides.
    pub(crate) fn spelling(&self, span: Span, symbol: stc_hir::Symbol) -> String {
        let canonical = self.analysis.hir.interner.text(symbol);
        self.parsed(span.file)
            .and_then(|parsed| crate::query::name_token(parsed, span, canonical))
            .and_then(|(start, end)| {
                self.source(span.file)
                    .and_then(|source| source.get(start as usize..end as usize))
            })
            .map_or_else(|| canonical.to_owned(), str::to_owned)
    }

    /// Returns the source spelling of a definition's name, or `""`.
    pub(crate) fn definition_spelling(&self, def: stc_hir::DefId) -> String {
        let data = &self.analysis.hir.defs[def.index()];
        data.name
            .map(|symbol| self.spelling(data.span, symbol))
            .unwrap_or_default()
    }

    fn rebuild_source_map(&mut self) {
        let mut source_map = SourceMap::new();
        for info in &self.files {
            source_map.add_file(info.display.clone(), self.texts[info.id.index()].clone());
        }
        self.source_map = source_map;
    }

    fn refresh(&mut self) {
        self.rebuild_source_map();
        let (analysis, diagnostics) = run_analysis(
            &self.packages,
            &self.parsed,
            self.test_range.clone(),
            &self.configuration,
            self.stdlib,
        );
        self.analysis = analysis;
        self.analysis_diagnostics = diagnostics;
        self.declarations = DeclarationIndex::build(&self.analysis.hir, &self.parsed);
    }
}

fn parse_source(
    file: FileId,
    bytes: &[u8],
    relaxed: bool,
    sink: &mut DiagnosticSink,
) -> (ParsedFile, String) {
    let lexed = tokenize_bytes(file, bytes, sink);
    let parsed = parse(
        file,
        lexed.text.as_str(),
        &lexed.stream,
        ParseOptions {
            relaxed_semicolons: relaxed,
            ..ParseOptions::default()
        },
        sink,
    );
    (parsed, lexed.text)
}

fn run_analysis(
    packages: &[PackageData],
    parsed: &[ParsedFile],
    test_range: Range<usize>,
    configuration: &Option<String>,
    stdlib: bool,
) -> (Analysis, Vec<Diagnostic>) {
    let sessions: Vec<PackageSession<'_>> = packages
        .iter()
        .map(|package| PackageSession {
            name: package.name.clone(),
            root_namespace: package.namespace.clone(),
            dependencies: package.dependencies.clone(),
            files: &parsed[package.files.clone()],
            is_root: package.is_root,
        })
        .collect();
    let session = ProjectSession {
        packages: sessions,
        configuration: configuration.clone(),
        stdlib,
        test_build: true,
        test_files: &parsed[test_range],
    };
    let mut sink = DiagnosticSink::new();
    let analysis = stc_sema::analyze_project(&session, &mut sink);
    (analysis, sink.into_diagnostics())
}
