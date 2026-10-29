//! `stc.lock` model, canonical rendering, parsing, and comparison.
//!
//! The lockfile is a resolved graph, not a source of requirements. Packages
//! are sorted by `(name, source)`, dependency lists are sorted, and paths are
//! forward-slash relative to the root package (CLI and Manifest §3).

use std::ops::Range;
use std::path::{Component, Path};

use stc_diag::Diagnostic;
use stc_span::{FileId, Span};
use toml_edit::{
    Array, ArrayOfTables, Document, DocumentMut, InlineTable, Item, Table, TableLike, Value,
};

use crate::model::LOCKFILE_SCHEMA;

/// The resolved package graph written to `stc.lock`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lockfile {
    /// Always [`LOCKFILE_SCHEMA`] after validation.
    pub schema: u32,
    /// Packages sorted by `(name, source)`.
    pub packages: Vec<LockPackage>,
}

impl Lockfile {
    /// Creates an empty lockfile with the supported schema.
    #[must_use]
    pub fn new() -> Self {
        Self {
            schema: LOCKFILE_SCHEMA,
            packages: Vec::new(),
        }
    }

    /// Sorts packages and dependency lists into canonical order.
    pub fn canonicalize(&mut self) {
        for package in &mut self.packages {
            package.dependencies.sort();
            package.dependencies.dedup();
        }
        self.packages.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.source.sort_key().cmp(right.source.sort_key()))
        });
    }
}

impl Default for Lockfile {
    fn default() -> Self {
        Self::new()
    }
}

/// One package entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockPackage {
    /// Package identity from `[package].name`.
    pub name: String,
    /// Package version.
    pub version: String,
    /// Root marker or a relative path.
    pub source: LockSource,
    /// Dependency edge names (aliases), sorted.
    pub dependencies: Vec<String>,
}

/// Where a package comes from.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LockSource {
    /// The root package.
    Root,
    /// A local path relative to the root package directory.
    Path(String),
}

impl LockSource {
    fn sort_key(&self) -> &str {
        match self {
            Self::Root => "root",
            Self::Path(path) => path,
        }
    }
}

/// Renders the canonical lockfile text.
#[must_use]
pub fn render_lockfile(lock: &Lockfile) -> String {
    let mut document = DocumentMut::new();
    document["schema"] = toml_edit::value(i64::from(lock.schema));
    let mut packages = ArrayOfTables::new();
    for package in &lock.packages {
        let mut table = Table::new();
        table.insert("name", toml_edit::value(package.name.as_str()));
        table.insert("version", toml_edit::value(package.version.as_str()));
        match &package.source {
            LockSource::Root => {
                table.insert("source", toml_edit::value("root"));
            }
            LockSource::Path(path) => {
                let mut inline = InlineTable::new();
                inline.insert("path", Value::from(path.as_str()));
                table.insert("source", Item::Value(Value::InlineTable(inline)));
            }
        }
        let mut dependencies = Array::new();
        for dependency in &package.dependencies {
            dependencies.push(dependency.as_str());
        }
        table.insert("dependencies", Item::Value(Value::Array(dependencies)));
        packages.push(table);
    }
    document["package"] = Item::ArrayOfTables(packages);
    let mut rendered = document.to_string();
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered
}

/// Parses and validates one `stc.lock` document.
///
/// # Errors
///
/// Returns the lockfile diagnostics when the document is malformed, has an
/// unsupported schema, or violates the canonical-form rules.
pub fn parse_lockfile(text: &str, file: FileId) -> Result<Lockfile, Vec<Diagnostic>> {
    let (stripped, base) = strip_bom(text);
    let mut parser = LockParser::new(file, base, text.len());
    let document = match Document::parse(stripped) {
        Ok(document) => document,
        Err(error) => {
            let span = parser.span(error.span());
            parser.error(format!("invalid TOML: {}", error.message()), span);
            return Err(parser.diagnostics);
        }
    };
    let lockfile = parser.parse_root(document.as_table());
    if parser.diagnostics.is_empty() {
        Ok(lockfile)
    } else {
        Err(parser.diagnostics)
    }
}

fn strip_bom(text: &str) -> (&str, usize) {
    match text.strip_prefix('\u{feff}') {
        Some(rest) => (rest, 3),
        None => (text, 0),
    }
}

struct LockParser {
    file: FileId,
    base: usize,
    full_len: usize,
    diagnostics: Vec<Diagnostic>,
}

impl LockParser {
    fn new(file: FileId, base: usize, full_len: usize) -> Self {
        Self {
            file,
            base,
            full_len,
            diagnostics: Vec::new(),
        }
    }

    fn span(&self, range: Option<Range<usize>>) -> Span {
        let range = range.unwrap_or(0..0);
        let start = self.base.saturating_add(range.start).min(self.full_len);
        let end = self
            .base
            .saturating_add(range.end)
            .max(start)
            .min(self.full_len);
        Span::new(
            self.file,
            u32::try_from(start).unwrap_or(u32::MAX),
            u32::try_from(end).unwrap_or(u32::MAX),
        )
    }

    fn item_span(&self, item: &Item) -> Span {
        self.span(item.span())
    }

    fn key_span(&self, key: &toml_edit::Key) -> Span {
        self.span(key.span())
    }

    fn table_span(&self, table: &dyn TableLike) -> Span {
        table.iter().next().map_or_else(
            || self.span(None),
            |(key, value)| {
                table
                    .key(key)
                    .map_or_else(|| self.item_span(value), |key| self.key_span(key))
            },
        )
    }

    fn error(&mut self, message: impl Into<String>, span: Span) {
        self.diagnostics
            .push(Diagnostic::error("STC-E-LOCK-003", message, span));
    }

    fn parse_root(&mut self, table: &dyn TableLike) -> Lockfile {
        for (key, value) in table.iter() {
            if key != "schema" && key != "package" {
                let span = table
                    .key(key)
                    .map_or_else(|| self.item_span(value), |key| self.key_span(key));
                self.error(format!("unknown lockfile key `{key}`"), span);
            }
        }
        let schema = match table.get("schema") {
            Some(item) => match item.as_integer() {
                Some(value) if value == i64::from(LOCKFILE_SCHEMA) => LOCKFILE_SCHEMA,
                Some(value) => {
                    let span = self.item_span(item);
                    self.error(
                        format!("unsupported lockfile schema {value} (expected {LOCKFILE_SCHEMA})"),
                        span,
                    );
                    LOCKFILE_SCHEMA
                }
                None => {
                    let span = self.item_span(item);
                    self.error("lockfile `schema` must be an integer", span);
                    LOCKFILE_SCHEMA
                }
            },
            None => {
                let span = self.table_span(table);
                self.error("missing required lockfile `schema` key", span);
                LOCKFILE_SCHEMA
            }
        };
        let mut packages = Vec::new();
        let mut seen: Vec<(String, LockSource)> = Vec::new();
        if let Some(item) = table.get("package") {
            match self.table_array(item) {
                Some(entries) => {
                    for (package, entry_span) in entries {
                        if let Some(package) = self.parse_package(package, entry_span) {
                            if seen.iter().any(|(name, source)| {
                                name == &package.name && source == &package.source
                            }) {
                                self.error(
                                    format!(
                                        "duplicate lockfile package `{}` with the same source",
                                        package.name
                                    ),
                                    entry_span,
                                );
                            } else {
                                seen.push((package.name.clone(), package.source.clone()));
                                packages.push(package);
                            }
                        }
                    }
                }
                None => {
                    let span = self.item_span(item);
                    self.error("`package` must be an array of tables", span);
                }
            }
        }
        let mut lockfile = Lockfile { schema, packages };
        lockfile.canonicalize();
        lockfile
    }

    fn parse_package(&mut self, package: &dyn TableLike, entry_span: Span) -> Option<LockPackage> {
        for (key, value) in package.iter() {
            if !matches!(key, "name" | "version" | "source" | "dependencies") {
                let span = package
                    .key(key)
                    .map_or_else(|| self.item_span(value), |key| self.key_span(key));
                self.error(format!("unknown package key `{key}`"), span);
            }
        }
        let name = match package.get("name") {
            Some(item) => match item.as_str() {
                Some(name) if !name.is_empty() => name.to_owned(),
                Some(_) => {
                    let span = self.item_span(item);
                    self.error("package `name` must not be empty", span);
                    return None;
                }
                None => {
                    let span = self.item_span(item);
                    self.error("package `name` must be a string", span);
                    return None;
                }
            },
            None => {
                self.error("package entry is missing `name`", entry_span);
                return None;
            }
        };
        let version = match package.get("version") {
            Some(item) => match item.as_str() {
                Some(version) if !version.is_empty() => version.to_owned(),
                Some(_) => {
                    let span = self.item_span(item);
                    self.error("package `version` must not be empty", span);
                    return None;
                }
                None => {
                    let span = self.item_span(item);
                    self.error("package `version` must be a string", span);
                    return None;
                }
            },
            None => {
                self.error("package entry is missing `version`", entry_span);
                return None;
            }
        };
        let source = match package.get("source") {
            Some(item) => self.parse_source(item)?,
            None => {
                self.error("package entry is missing `source`", entry_span);
                return None;
            }
        };
        let mut dependencies = Vec::new();
        if let Some(item) = package.get("dependencies") {
            match item.as_array() {
                Some(array) => {
                    for value in array.iter() {
                        let span = self.span(value.span());
                        match value.as_str() {
                            Some(dependency) if !dependency.is_empty() => {
                                if dependencies.iter().any(|existing| existing == dependency) {
                                    self.error(
                                        format!("duplicate dependency `{dependency}`"),
                                        span,
                                    );
                                } else {
                                    dependencies.push(dependency.to_owned());
                                }
                            }
                            Some(_) => self.error("dependency names must not be empty", span),
                            None => self.error("dependency names must be strings", span),
                        }
                    }
                }
                None => {
                    let span = self.item_span(item);
                    self.error("package `dependencies` must be an array of strings", span);
                }
            }
        }
        Some(LockPackage {
            name,
            version,
            source,
            dependencies,
        })
    }

    fn parse_source(&mut self, item: &Item) -> Option<LockSource> {
        if let Some(text) = item.as_str() {
            if text == "root" {
                return Some(LockSource::Root);
            }
            let span = self.item_span(item);
            self.error(
                format!("package `source` string `{text}` is invalid; expected \"root\""),
                span,
            );
            return None;
        }
        let Some(table) = item.as_table_like() else {
            let span = self.item_span(item);
            self.error(
                "package `source` must be \"root\" or a table with a `path`",
                span,
            );
            return None;
        };
        for (key, value) in table.iter() {
            if key != "path" {
                let span = table
                    .key(key)
                    .map_or_else(|| self.item_span(value), |key| self.key_span(key));
                self.error(format!("unknown source key `{key}`"), span);
            }
        }
        let Some(path_item) = table.get("path") else {
            let span = self.item_span(item);
            self.error("package `source` table is missing `path`", span);
            return None;
        };
        let span = self.item_span(path_item);
        let Some(path) = path_item.as_str() else {
            self.error("package `source.path` must be a string", span);
            return None;
        };
        if let Some(reason) = lock_path_problem(path) {
            self.error(format!("invalid package path `{path}`: {reason}"), span);
            return None;
        }
        Some(LockSource::Path(path.to_owned()))
    }
}

impl LockParser {
    fn table_array<'a>(&self, item: &'a Item) -> Option<Vec<(&'a dyn TableLike, Span)>> {
        if let Some(tables) = item.as_array_of_tables() {
            return Some(
                tables
                    .iter()
                    .map(|table| (table as &dyn TableLike, self.span(table.span())))
                    .collect(),
            );
        }
        if let Some(array) = item.as_array() {
            let mut entries = Vec::new();
            for value in array.iter() {
                let table = value.as_inline_table()?;
                entries.push((table as &dyn TableLike, self.span(value.span())));
            }
            return Some(entries);
        }
        None
    }
}

/// Returns why a lockfile path is not a normalized relative path.
fn lock_path_problem(path: &str) -> Option<&'static str> {
    if path.is_empty() {
        return Some("must not be empty");
    }
    if path.contains('\\') {
        return Some("must use `/` separators");
    }
    if path.starts_with('/') {
        return Some("must be relative");
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Some("must be relative");
    }
    if Path::new(path).is_absolute() {
        return Some("must be relative");
    }
    None
}

/// Computes a forward-slash relative path from `root` to `target`.
///
/// Returns `None` when the paths are on different volumes.
#[must_use]
pub(crate) fn relative_lock_path(root: &Path, target: &Path) -> Option<String> {
    let root_components: Vec<Component<'_>> = root.components().collect();
    let target_components: Vec<Component<'_>> = target.components().collect();
    let mut common = 0;
    while common < root_components.len()
        && common < target_components.len()
        && components_equal(root_components[common], target_components[common])
    {
        common += 1;
    }
    if common == 0 {
        return None;
    }
    let mut parts = Vec::new();
    for _ in common..root_components.len() {
        parts.push("..".to_owned());
    }
    for component in &target_components[common..] {
        parts.push(component.as_os_str().to_string_lossy().into_owned());
    }
    if parts.is_empty() {
        parts.push(".".to_owned());
    }
    Some(parts.join("/"))
}

fn components_equal(left: Component<'_>, right: Component<'_>) -> bool {
    crate::host::components_equal(left, right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Lockfile {
        Lockfile {
            schema: 1,
            packages: vec![
                LockPackage {
                    name: "motor-lib".to_owned(),
                    version: "0.2.0".to_owned(),
                    source: LockSource::Path("../motor-lib".to_owned()),
                    dependencies: vec!["a".to_owned(), "b".to_owned()],
                },
                LockPackage {
                    name: "motor-control".to_owned(),
                    version: "0.1.0".to_owned(),
                    source: LockSource::Root,
                    dependencies: vec!["motor-lib".to_owned()],
                },
            ],
        }
    }

    #[test]
    fn renders_deterministically() {
        let mut first = sample();
        first.canonicalize();
        let mut second = first.clone();
        second.packages.reverse();
        second.canonicalize();
        assert_eq!(render_lockfile(&first), render_lockfile(&second));
        let text = render_lockfile(&first);
        assert!(text.contains("source = \"root\""));
        assert!(text.contains("source = { path = \"../motor-lib\" }"));
        assert!(text.contains("dependencies = [\"a\", \"b\"]"));
        assert!(!text.contains('\\'));
    }

    #[test]
    fn parses_canonical_text() {
        let mut lock = sample();
        lock.canonicalize();
        let text = render_lockfile(&lock);
        let parsed = parse_lockfile(&text, FileId::new(0)).expect("valid lockfile");
        assert_eq!(parsed, lock);
    }

    #[test]
    fn rejects_absolute_and_backslash_paths() {
        assert!(lock_path_problem("C:/x").is_some());
        assert!(lock_path_problem("/x").is_some());
        assert!(lock_path_problem("a\\b").is_some());
        assert!(lock_path_problem("../a").is_none());
        assert!(lock_path_problem("a/b").is_none());
    }
}
