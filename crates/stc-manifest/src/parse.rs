//! Span-aware `stc.toml` parsing and validation.
//!
//! Diagnostics point into the manifest text. Unknown keys get a
//! "did you mean" suggestion, including kebab-case corrections for the
//! snake_case spellings that v1 rejects (CLI and Manifest §2.2).

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Component;

use stc_diag::{Diagnostic, Severity};
use stc_span::{FileId, Span};
use toml_edit::{Document, Item, Key, TableLike, Value};

use crate::codes;
use crate::glob::Pattern;
use crate::model::{
    Baseline, BuildSettings, ChannelRef, ColdPolicy, ConfigValue, DebugProtocol, DebugSettings,
    DependencyDecl, DurationNs, EmitKind, GlobSpec, IoBackend, IoBinding, IoChannel, IoConfig,
    IoDevice, IoDirection, LibType, LintLevel, LintSettings, MANIFEST_SCHEMA, Manifest,
    MismatchPolicy, OnlineSettings, OptLevel, PROFILE_ID, PackageInfo, ProjectSettings,
    RetainSettings, RunSettings, RuntimeSettings, SyntaxMode, TestCase, TestSettings,
};

const TOP_LEVEL_KEYS: &[&str] = &[
    "schema",
    "package",
    "dependencies",
    "project",
    "build",
    "io",
    "retain",
    "runtime",
    "online",
    "debug",
    "lints",
    "run",
    "test",
];

const PACKAGE_KEYS: &[&str] = &["name", "version", "language"];
const DEPENDENCY_KEYS: &[&str] = &["path", "package", "version"];
const PROJECT_KEYS: &[&str] = &["sources", "stdlib", "configuration"];
const BUILD_KEYS: &[&str] = &[
    "opt-level",
    "out-dir",
    "lib-type",
    "debug",
    "syntax",
    "emit",
    "target",
    "clang",
    "runtime",
];
const IO_KEYS: &[&str] = &[
    "backend",
    "file",
    "input-bytes",
    "output-bytes",
    "marker-bytes",
    "devices",
    "bindings",
];
const DEVICE_KEYS: &[&str] = &["name", "kind", "config", "channels"];
const CHANNEL_KEYS: &[&str] = &["name", "direction", "width"];
const BINDING_KEYS: &[&str] = &["variable", "device", "channel", "direction"];
const RETAIN_KEYS: &[&str] = &["enabled", "file", "on-mismatch"];
const RUNTIME_KEYS: &[&str] = &["module-dir", "manifest-dir", "max-retired-generations"];
const ONLINE_KEYS: &[&str] = &[
    "baseline",
    "manifest",
    "cold-policy",
    "allow-untracked-removal",
];
const DEBUG_KEYS: &[&str] = &["default", "protocol", "force-sessions-end-on-swap"];
const RUN_KEYS: &[&str] = &["cycles", "cycle-time"];
const TEST_KEYS: &[&str] = &[
    "sources",
    "timeout",
    "default-cycles",
    "mock-clock",
    "io",
    "case",
];
const TEST_CASE_KEYS: &[&str] = &["name", "cycles", "timeout", "expect-trap"];

const TRAP_CODES: &[&str] = &[
    "STC_TRAP_OVERFLOW",
    "STC_TRAP_DIV_ZERO",
    "STC_TRAP_RANGE",
    "STC_TRAP_BOUNDS",
    "STC_TRAP_NULL_REF",
    "STC_TRAP_INVALID_UTF8",
    "STC_TRAP_CONVERSION",
    "STC_TRAP_WATCHDOG",
    "STC_TRAP_ASSERT",
    "STC_TRAP_UNIMPLEMENTED",
];

const CONFIG_NESTING_LIMIT: usize = 64;
const SUGGESTION_KEY_LIMIT: usize = 64;

/// A successfully parsed manifest and any non-fatal warnings.
#[derive(Clone, Debug)]
pub struct ParsedManifest {
    /// The validated model.
    pub manifest: Manifest,
    /// Warnings emitted while parsing (for example `debug = false`).
    pub warnings: Vec<Diagnostic>,
}

/// Parses and validates one `stc.toml` document.
///
/// # Errors
///
/// Returns every diagnostic when at least one has error severity.
pub fn parse_manifest(text: &str, file: FileId) -> Result<ParsedManifest, Vec<Diagnostic>> {
    let (stripped, base) = strip_bom(text);
    let mut parser = Parser::new(file, base, text.len());
    let document = match Document::parse(stripped) {
        Ok(document) => document,
        Err(error) => {
            let span = parser.span(error.span());
            parser.error(
                "STC-E-PROJECT-001",
                format!("invalid TOML: {}", error.message()),
                span,
            );
            return Err(parser.diagnostics);
        }
    };
    let manifest = parser.parse_root(document.as_table());
    if parser.has_errors() {
        Err(parser.diagnostics)
    } else {
        Ok(ParsedManifest {
            manifest,
            warnings: parser.diagnostics,
        })
    }
}

fn strip_bom(text: &str) -> (&str, usize) {
    match text.strip_prefix('\u{feff}') {
        Some(rest) => (rest, 3),
        None => (text, 0),
    }
}

struct Parser {
    file: FileId,
    base: usize,
    full_len: usize,
    diagnostics: Vec<Diagnostic>,
}

impl Parser {
    fn new(file: FileId, base: usize, full_len: usize) -> Self {
        Self {
            file,
            base,
            full_len,
            diagnostics: Vec::new(),
        }
    }

    fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
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

    fn key_span(&self, key: &Key) -> Span {
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

    fn error(&mut self, code: &'static str, message: impl Into<String>, span: Span) {
        self.diagnostics
            .push(Diagnostic::error(code, message, span));
    }

    fn warning(&mut self, code: &'static str, message: impl Into<String>, span: Span) {
        self.diagnostics
            .push(Diagnostic::warning(code, message, span));
    }

    fn parse_root(&mut self, table: &dyn TableLike) -> Manifest {
        self.check_unknown(table, "top-level", TOP_LEVEL_KEYS);
        let schema = self.parse_schema(table);
        let package = self.parse_package(table);
        let dependencies = self.parse_dependencies(table);
        let project = self.parse_project(table);
        let build = self.parse_build(table);
        let io = self.parse_io(table);
        let retain = self.parse_retain(table);
        let runtime = self.parse_runtime(table);
        let online = self.parse_online(table);
        let debug = self.parse_debug(table);
        let lints = self.parse_lints(table);
        let run = self.parse_run(table);
        let test = self.parse_test(table);
        Manifest {
            schema,
            package,
            dependencies,
            project,
            build,
            io,
            retain,
            runtime,
            online,
            debug,
            lints,
            run,
            test,
        }
    }

    fn parse_schema(&mut self, table: &dyn TableLike) -> u32 {
        let Some(item) = table.get("schema") else {
            self.error(
                "STC-E-PROJECT-002",
                "missing required `schema` key (expected 1)",
                self.table_span(table),
            );
            return MANIFEST_SCHEMA;
        };
        let span = self.item_span(item);
        match item.as_integer() {
            Some(value) if value == i64::from(MANIFEST_SCHEMA) => MANIFEST_SCHEMA,
            Some(value) => {
                self.error(
                    "STC-E-PROJECT-002",
                    format!("unsupported manifest schema {value} (expected {MANIFEST_SCHEMA})"),
                    span,
                );
                MANIFEST_SCHEMA
            }
            None => {
                self.type_error("STC-E-PROJECT-002", "schema", "an integer", item);
                MANIFEST_SCHEMA
            }
        }
    }

    fn parse_package(&mut self, table: &dyn TableLike) -> PackageInfo {
        let Some(item) = table.get("package") else {
            self.error(
                "STC-E-PROJECT-003",
                "missing `[package]` table",
                self.table_span(table),
            );
            return PackageInfo {
                name: String::new(),
                version: String::new(),
                language: String::new(),
            };
        };
        let Some(package) = item.as_table_like() else {
            self.error(
                "STC-E-PROJECT-003",
                format!("`[package]` must be a table, found {}", item.type_name()),
                self.item_span(item),
            );
            return PackageInfo {
                name: String::new(),
                version: String::new(),
                language: String::new(),
            };
        };
        let table_span = self.item_span(item);
        self.check_unknown(package, "[package]", PACKAGE_KEYS);
        let name = self
            .required_string(package, "name", "STC-E-PROJECT-003", table_span)
            .unwrap_or_default();
        if !name.is_empty() && !valid_package_name(&name) {
            let span = package
                .get("name")
                .map_or(table_span, |value| self.item_span(value));
            self.error(
                "STC-E-PROJECT-003",
                format!("invalid package name `{name}`; expected `[A-Za-z_][A-Za-z0-9_-]*`"),
                span,
            );
        }
        let version = self
            .required_string(package, "version", "STC-E-PROJECT-003", table_span)
            .unwrap_or_default();
        if !version.is_empty() && !valid_version(&version) {
            let span = package
                .get("version")
                .map_or(table_span, |value| self.item_span(value));
            self.error(
                "STC-E-PROJECT-003",
                format!("invalid package version `{version}`; expected a semver-like value"),
                span,
            );
        }
        let language = self
            .required_string(package, "language", "STC-E-PROJECT-003", table_span)
            .unwrap_or_default();
        if !language.is_empty() && language != PROFILE_ID {
            let span = package
                .get("language")
                .map_or(table_span, |value| self.item_span(value));
            self.error(
                "STC-E-PROJECT-003",
                format!("unsupported language `{language}`; expected `{PROFILE_ID}`"),
                span,
            );
        }
        PackageInfo {
            name,
            version,
            language,
        }
    }

    fn parse_dependencies(&mut self, table: &dyn TableLike) -> Vec<DependencyDecl> {
        let Some(item) = table.get("dependencies") else {
            return Vec::new();
        };
        let Some(dependencies) = item.as_table_like() else {
            self.error(
                "STC-E-PROJECT-005",
                format!(
                    "`[dependencies]` must be a table, found {}",
                    item.type_name()
                ),
                self.item_span(item),
            );
            return Vec::new();
        };
        let mut declarations = Vec::new();
        for (alias, value) in dependencies.iter() {
            let key_span = dependencies
                .key(alias)
                .map_or_else(|| self.item_span(value), |key| self.key_span(key));
            if !valid_package_name(alias) {
                self.error(
                    "STC-E-PROJECT-005",
                    format!(
                        "invalid dependency alias `{alias}`; expected `[A-Za-z_][A-Za-z0-9_-]*`"
                    ),
                    key_span,
                );
            }
            let Some(entry) = value.as_table_like() else {
                self.error(
                    "STC-E-PROJECT-005",
                    format!(
                        "dependency `{alias}` must be a table with a `path` key, found {}",
                        value.type_name()
                    ),
                    self.item_span(value),
                );
                continue;
            };
            self.check_unknown(entry, "dependency", DEPENDENCY_KEYS);
            let entry_span = self.item_span(value);
            let Some(path_item) = entry.get("path") else {
                self.error(
                    "STC-E-PROJECT-005",
                    format!("dependency `{alias}` is missing the required `path` key"),
                    entry_span,
                );
                continue;
            };
            let path_span = self.item_span(path_item);
            let Some(path) = self.expect_nonempty_string(path_item, "STC-E-PROJECT-005", "path")
            else {
                continue;
            };
            let package = match entry.get("package") {
                Some(package_item) => {
                    let package_span = self.item_span(package_item);
                    match self.expect_nonempty_string(package_item, "STC-E-PROJECT-005", "package")
                    {
                        Some(package) => {
                            if !valid_package_name(&package) {
                                self.error(
                                    "STC-E-PROJECT-005",
                                    format!("invalid dependency package name `{package}`"),
                                    package_span,
                                );
                            }
                            Some((package, package_span))
                        }
                        None => None,
                    }
                }
                None => None,
            };
            if let Some(version_item) = entry.get("version")
                && version_item.as_str().is_none()
            {
                self.type_error("STC-E-PROJECT-002", "version", "a string", version_item);
            }
            declarations.push(DependencyDecl {
                alias: alias.to_owned(),
                path,
                package: package.as_ref().map(|(name, _)| name.clone()),
                key_span,
                path_span,
                package_span: package.map(|(_, span)| span),
            });
        }
        declarations
    }

    fn parse_project(&mut self, table: &dyn TableLike) -> ProjectSettings {
        let Some(item) = table.get("project") else {
            self.error(
                "STC-E-PROJECT-004",
                "missing `[project]` table with a `sources` list",
                self.table_span(table),
            );
            return ProjectSettings::default();
        };
        let Some(project) = item.as_table_like() else {
            self.type_error("STC-E-PROJECT-002", "project", "a table", item);
            return ProjectSettings::default();
        };
        let project_span = self.item_span(item);
        self.check_unknown(project, "[project]", PROJECT_KEYS);
        let sources = match project.get("sources") {
            Some(sources_item) => {
                let globs = self.parse_globs(sources_item, "sources", "STC-E-PROJECT-004");
                if let Some(array) = sources_item.as_array()
                    && array.is_empty()
                {
                    self.error(
                        "STC-E-PROJECT-004",
                        "`[project].sources` must contain at least one glob",
                        self.item_span(sources_item),
                    );
                }
                globs
            }
            None => {
                self.error(
                    "STC-E-PROJECT-004",
                    "missing required `[project].sources` list",
                    project_span,
                );
                Vec::new()
            }
        };
        let stdlib = match project.get("stdlib") {
            Some(value) => self.expect_bool(value, "STC-E-PROJECT-002", "stdlib"),
            None => true,
        };
        let configuration = match project.get("configuration") {
            Some(value) => self.expect_nonempty_string(value, "STC-E-PROJECT-002", "configuration"),
            None => None,
        };
        ProjectSettings {
            sources,
            stdlib,
            configuration,
        }
    }

    fn parse_globs(&mut self, item: &Item, key: &str, code: &'static str) -> Vec<GlobSpec> {
        let Some(array) = item.as_array() else {
            self.type_error(code, key, "an array of strings", item);
            return Vec::new();
        };
        let mut globs = Vec::new();
        for value in array.iter() {
            let span = self.span(value.span());
            let Some(pattern) = value.as_str() else {
                self.error(
                    code,
                    format!(
                        "`{key}` entries must be strings, found {}",
                        value.type_name()
                    ),
                    span,
                );
                continue;
            };
            if let Err(reason) = Pattern::parse(pattern) {
                self.error(
                    code,
                    format!("invalid source glob `{pattern}`: {reason}"),
                    span,
                );
                continue;
            }
            globs.push(GlobSpec {
                pattern: pattern.to_owned(),
                span,
            });
        }
        globs
    }

    fn parse_build(&mut self, table: &dyn TableLike) -> BuildSettings {
        let mut settings = BuildSettings::default();
        let Some(item) = table.get("build") else {
            return settings;
        };
        let Some(build) = item.as_table_like() else {
            self.type_error("STC-E-PROJECT-002", "build", "a table", item);
            return settings;
        };
        self.check_unknown(build, "[build]", BUILD_KEYS);
        if let Some(value) = build.get("opt-level") {
            settings.opt_level = self.parse_opt_level(value).unwrap_or(OptLevel::O0);
        }
        if let Some(value) = build.get("out-dir")
            && let Some(path) = self.expect_nonempty_string(value, "STC-E-PROJECT-010", "out-dir")
        {
            if let Some(reason) = relative_path_problem(&path) {
                self.error(
                    "STC-E-PROJECT-010",
                    format!("invalid `out-dir` `{path}`: {reason}"),
                    self.item_span(value),
                );
            }
            settings.out_dir = path;
        }
        if let Some(value) = build.get("lib-type") {
            settings.lib_type = match value.as_str() {
                Some("static") => LibType::Static,
                Some("shared") => LibType::Shared,
                Some(other) => {
                    self.invalid_enum(
                        "STC-E-PROJECT-010",
                        "lib-type",
                        other,
                        "`static` or `shared`",
                        value,
                    );
                    settings.lib_type
                }
                None => {
                    self.type_error("STC-E-PROJECT-002", "lib-type", "a string", value);
                    settings.lib_type
                }
            };
        }
        if let Some(value) = build.get("debug") {
            settings.debug = self.expect_bool(value, "STC-E-PROJECT-002", "debug");
            if !settings.debug {
                self.warning(
                    "STC-W-PROJECT-001",
                    "`debug = false` is parsed but M6 builds the unoptimised debug form without debug information",
                    self.item_span(value),
                );
            }
        }
        if let Some(value) = build.get("syntax") {
            settings.syntax = match value.as_str() {
                Some("strict") => SyntaxMode::Strict,
                Some("relaxed") => SyntaxMode::Relaxed,
                Some(other) => {
                    self.invalid_enum(
                        "STC-E-PROJECT-010",
                        "syntax",
                        other,
                        "`strict` or `relaxed`",
                        value,
                    );
                    settings.syntax
                }
                None => {
                    self.type_error("STC-E-PROJECT-002", "syntax", "a string", value);
                    settings.syntax
                }
            };
        }
        if let Some(value) = build.get("emit") {
            settings.emit = self.parse_emit(value);
        }
        if let Some(value) = build.get("target") {
            settings.target = self.expect_nonempty_string(value, "STC-E-PROJECT-010", "target");
        }
        if let Some(value) = build.get("clang") {
            settings.clang = self.expect_nonempty_string(value, "STC-E-PROJECT-010", "clang");
        }
        if let Some(value) = build.get("runtime") {
            settings.runtime = self.expect_nonempty_string(value, "STC-E-PROJECT-010", "runtime");
        }
        settings
    }

    fn parse_opt_level(&mut self, item: &Item) -> Option<OptLevel> {
        if let Some(value) = item.as_integer() {
            return match value {
                0 => Some(OptLevel::O0),
                1 => Some(OptLevel::O1),
                2 => Some(OptLevel::O2),
                3 => Some(OptLevel::O3),
                _ => {
                    self.error(
                        "STC-E-PROJECT-010",
                        format!("invalid `opt-level` {value}; expected 0, 1, 2, 3, or \"s\""),
                        self.item_span(item),
                    );
                    None
                }
            };
        }
        if let Some(value) = item.as_str() {
            if value == "s" {
                return Some(OptLevel::Size);
            }
            self.invalid_enum(
                "STC-E-PROJECT-010",
                "opt-level",
                value,
                "0, 1, 2, 3, or \"s\"",
                item,
            );
            return None;
        }
        self.type_error(
            "STC-E-PROJECT-002",
            "opt-level",
            "an integer or string",
            item,
        );
        None
    }

    fn parse_emit(&mut self, item: &Item) -> Vec<EmitKind> {
        let Some(array) = item.as_array() else {
            self.type_error("STC-E-PROJECT-002", "emit", "an array of strings", item);
            return vec![EmitKind::Module];
        };
        let mut kinds = BTreeSet::new();
        for value in array.iter() {
            let span = self.span(value.span());
            let Some(text) = value.as_str() else {
                self.error(
                    "STC-E-PROJECT-002",
                    format!(
                        "`emit` entries must be strings, found {}",
                        value.type_name()
                    ),
                    span,
                );
                continue;
            };
            let kind = match text {
                "ir" => EmitKind::Ir,
                "obj" => EmitKind::Obj,
                "module" => EmitKind::Module,
                "exe" => EmitKind::Exe,
                "lib" => EmitKind::Lib,
                "metadata" => EmitKind::Metadata,
                other => {
                    self.invalid_enum_value(
                        "STC-E-PROJECT-010",
                        "emit",
                        other,
                        "`ir`, `obj`, `module`, `exe`, `lib`, or `metadata`",
                        span,
                    );
                    continue;
                }
            };
            if !kinds.insert(kind) {
                self.error(
                    "STC-E-PROJECT-010",
                    format!("duplicate `emit` value `{text}`"),
                    span,
                );
            }
        }
        if kinds.is_empty() {
            self.error(
                "STC-E-PROJECT-010",
                "`emit` must contain at least one artifact kind",
                self.item_span(item),
            );
            return vec![EmitKind::Module];
        }
        kinds.into_iter().collect()
    }

    fn parse_io(&mut self, table: &dyn TableLike) -> IoConfig {
        let mut config = IoConfig::default();
        let Some(item) = table.get("io") else {
            return config;
        };
        let Some(io) = item.as_table_like() else {
            self.type_error("STC-E-PROJECT-002", "io", "a table", item);
            return config;
        };
        self.check_unknown(io, "[io]", IO_KEYS);
        if let Some(value) = io.get("backend") {
            config.backend = self
                .parse_io_backend(value, "STC-E-PROJECT-009", "backend")
                .unwrap_or(IoBackend::Null);
        }
        if let Some(value) = io.get("file") {
            config.file = self.expect_nonempty_string(value, "STC-E-PROJECT-009", "file");
        }
        if let Some(value) = io.get("input-bytes") {
            config.input_bytes = self.parse_u32_setting(value, "STC-E-PROJECT-009", "input-bytes");
        }
        if let Some(value) = io.get("output-bytes") {
            config.output_bytes =
                self.parse_u32_setting(value, "STC-E-PROJECT-009", "output-bytes");
        }
        if let Some(value) = io.get("marker-bytes") {
            config.marker_bytes =
                self.parse_u32_setting(value, "STC-E-PROJECT-009", "marker-bytes");
        }
        if let Some(value) = io.get("devices") {
            config.devices = self.parse_devices(value);
        }
        if let Some(value) = io.get("bindings") {
            config.bindings = self.parse_bindings(value, &config.devices);
        }
        config
    }

    fn parse_io_backend(
        &mut self,
        item: &Item,
        code: &'static str,
        key: &str,
    ) -> Option<IoBackend> {
        match item.as_str() {
            Some("null") => Some(IoBackend::Null),
            Some("memory") => Some(IoBackend::Memory),
            Some("file") => Some(IoBackend::File),
            Some(other) => {
                self.invalid_enum(code, key, other, "`null`, `memory`, or `file`", item);
                None
            }
            None => {
                self.type_error("STC-E-PROJECT-002", key, "a string", item);
                None
            }
        }
    }

    fn parse_u32_setting(&mut self, item: &Item, code: &'static str, key: &str) -> Option<u32> {
        let Some(value) = item.as_integer() else {
            self.type_error("STC-E-PROJECT-002", key, "an integer", item);
            return None;
        };
        match u32::try_from(value) {
            Ok(value) => Some(value),
            Err(_) => {
                self.error(
                    code,
                    format!("invalid `{key}` {value}; expected a non-negative 32-bit integer"),
                    self.item_span(item),
                );
                None
            }
        }
    }

    fn parse_devices(&mut self, item: &Item) -> Vec<IoDevice> {
        let Some(entries) = self.table_array(item, "STC-E-PROJECT-009", "devices") else {
            return Vec::new();
        };
        let mut devices = Vec::new();
        let mut names: BTreeMap<String, Span> = BTreeMap::new();
        for (device, entry_span) in entries {
            self.check_unknown(device, "device", DEVICE_KEYS);
            let Some(name) =
                self.required_nonempty_string(device, "name", "STC-E-PROJECT-009", entry_span)
            else {
                continue;
            };
            if let Some(previous) = names.get(&name) {
                let diagnostic = Diagnostic::error(
                    "STC-E-PROJECT-009",
                    format!("duplicate I/O device name `{name}`"),
                    entry_span,
                )
                .with_label(*previous, "first declared here");
                self.diagnostics.push(diagnostic);
            } else {
                names.insert(name.clone(), entry_span);
            }
            let Some(kind) =
                self.required_nonempty_string(device, "kind", "STC-E-PROJECT-009", entry_span)
            else {
                continue;
            };
            let config = device
                .get("config")
                .and_then(|value| self.config_value(value, "STC-E-PROJECT-009", "config", 0));
            let channels = match device.get("channels") {
                Some(value) => self.parse_channels(value, &name),
                None => Vec::new(),
            };
            devices.push(IoDevice {
                name,
                kind,
                config,
                channels,
                span: entry_span,
            });
        }
        devices
    }

    fn parse_channels(&mut self, item: &Item, device: &str) -> Vec<IoChannel> {
        let Some(entries) = self.table_array(item, "STC-E-PROJECT-009", "channels") else {
            return Vec::new();
        };
        let mut channels = Vec::new();
        let mut names: BTreeSet<String> = BTreeSet::new();
        for (channel, entry_span) in entries {
            self.check_unknown(channel, "channel", CHANNEL_KEYS);
            let Some(name) =
                self.required_nonempty_string(channel, "name", "STC-E-PROJECT-009", entry_span)
            else {
                continue;
            };
            if !names.insert(name.clone()) {
                self.error(
                    "STC-E-PROJECT-009",
                    format!("duplicate channel name `{name}` on device `{device}`"),
                    entry_span,
                );
            }
            let direction = match channel.get("direction") {
                Some(value) => match value.as_str() {
                    Some("input") => IoDirection::Input,
                    Some("output") => IoDirection::Output,
                    Some("inout") => IoDirection::InOut,
                    Some(other) => {
                        self.invalid_enum(
                            "STC-E-PROJECT-009",
                            "direction",
                            other,
                            "`input`, `output`, or `inout`",
                            value,
                        );
                        continue;
                    }
                    None => {
                        self.type_error("STC-E-PROJECT-002", "direction", "a string", value);
                        continue;
                    }
                },
                None => {
                    self.error(
                        "STC-E-PROJECT-009",
                        format!("channel `{name}` is missing the required `direction` key"),
                        entry_span,
                    );
                    continue;
                }
            };
            let width = match channel.get("width") {
                Some(value) => match self.parse_u32_setting(value, "STC-E-PROJECT-009", "width") {
                    Some(0) => {
                        self.error(
                            "STC-E-PROJECT-009",
                            format!("channel `{name}` width must be greater than zero"),
                            self.item_span(value),
                        );
                        continue;
                    }
                    Some(width) => width,
                    None => continue,
                },
                None => {
                    self.error(
                        "STC-E-PROJECT-009",
                        format!("channel `{name}` is missing the required `width` key"),
                        entry_span,
                    );
                    continue;
                }
            };
            channels.push(IoChannel {
                name,
                direction,
                width,
            });
        }
        channels
    }

    fn parse_bindings(&mut self, item: &Item, devices: &[IoDevice]) -> Vec<IoBinding> {
        let Some(entries) = self.table_array(item, "STC-E-PROJECT-009", "bindings") else {
            return Vec::new();
        };
        let mut bindings = Vec::new();
        let mut variables: BTreeMap<String, Span> = BTreeMap::new();
        for (binding, entry_span) in entries {
            self.check_unknown(binding, "binding", BINDING_KEYS);
            let Some(variable) =
                self.required_nonempty_string(binding, "variable", "STC-E-PROJECT-009", entry_span)
            else {
                continue;
            };
            if let Some(previous) = variables.get(&variable) {
                let diagnostic = Diagnostic::error(
                    "STC-E-PROJECT-009",
                    format!("duplicate I/O binding for `{variable}`"),
                    entry_span,
                )
                .with_label(*previous, "first binding here");
                self.diagnostics.push(diagnostic);
            } else {
                variables.insert(variable.clone(), entry_span);
            }
            let Some(device_name) =
                self.required_nonempty_string(binding, "device", "STC-E-PROJECT-009", entry_span)
            else {
                continue;
            };
            let device = devices.iter().find(|device| device.name == device_name);
            if device.is_none() {
                self.error(
                    "STC-E-PROJECT-009",
                    format!("binding `{variable}` names undeclared device `{device_name}`"),
                    entry_span,
                );
            }
            let channel = match binding.get("channel") {
                Some(value) => self.parse_channel_ref(value),
                None => None,
            };
            let direction = match binding.get("direction") {
                Some(value) => match value.as_str() {
                    Some("input") => Some(IoDirection::Input),
                    Some("output") => Some(IoDirection::Output),
                    Some("inout") => Some(IoDirection::InOut),
                    Some(other) => {
                        self.invalid_enum(
                            "STC-E-PROJECT-009",
                            "direction",
                            other,
                            "`input`, `output`, or `inout`",
                            value,
                        );
                        None
                    }
                    None => {
                        self.type_error("STC-E-PROJECT-002", "direction", "a string", value);
                        None
                    }
                },
                None => None,
            };
            if let (Some(device), Some(ChannelRef::Name(channel_name))) = (device, channel.as_ref())
            {
                match device
                    .channels
                    .iter()
                    .find(|channel| &channel.name == channel_name)
                {
                    Some(declared) => {
                        if let Some(direction) = direction
                            && declared.direction != IoDirection::InOut
                            && direction != IoDirection::InOut
                            && declared.direction != direction
                        {
                            self.error(
                                "STC-E-PROJECT-009",
                                format!(
                                    "binding `{variable}` direction `{}` conflicts with channel `{channel_name}` direction `{}`",
                                    direction.as_str(),
                                    declared.direction.as_str()
                                ),
                                entry_span,
                            );
                        }
                    }
                    None => {
                        if !device.channels.is_empty() {
                            self.error(
                                "STC-E-PROJECT-009",
                                format!(
                                    "binding `{variable}` names undeclared channel `{channel_name}` on device `{device_name}`"
                                ),
                                entry_span,
                            );
                        }
                    }
                }
            }
            bindings.push(IoBinding {
                variable,
                device: device_name,
                channel,
                direction,
                span: entry_span,
            });
        }
        bindings
    }

    fn parse_channel_ref(&mut self, item: &Item) -> Option<ChannelRef> {
        if let Some(name) = item.as_str() {
            if name.is_empty() {
                self.error(
                    "STC-E-PROJECT-009",
                    "binding `channel` must not be empty",
                    self.item_span(item),
                );
                return None;
            }
            return Some(ChannelRef::Name(name.to_owned()));
        }
        if let Some(value) = item.as_integer() {
            return match u32::try_from(value) {
                Ok(value) => Some(ChannelRef::Index(value)),
                Err(_) => {
                    self.error(
                        "STC-E-PROJECT-009",
                        format!(
                            "invalid binding `channel` {value}; expected a non-negative integer"
                        ),
                        self.item_span(item),
                    );
                    None
                }
            };
        }
        self.error(
            "STC-E-PROJECT-009",
            format!(
                "binding `channel` must be a string or non-negative integer, found {}",
                item.type_name()
            ),
            self.item_span(item),
        );
        None
    }

    fn parse_retain(&mut self, table: &dyn TableLike) -> RetainSettings {
        let mut settings = RetainSettings::default();
        let Some(item) = table.get("retain") else {
            return settings;
        };
        let Some(retain) = item.as_table_like() else {
            self.type_error("STC-E-PROJECT-002", "retain", "a table", item);
            return settings;
        };
        self.check_unknown(retain, "[retain]", RETAIN_KEYS);
        if let Some(value) = retain.get("enabled") {
            settings.enabled = self.expect_bool(value, "STC-E-PROJECT-002", "enabled");
        }
        if let Some(value) = retain.get("file")
            && let Some(path) = self.expect_nonempty_string(value, "STC-E-PROJECT-010", "file")
        {
            if let Some(reason) = relative_path_problem(&path) {
                self.error(
                    "STC-E-PROJECT-010",
                    format!("invalid retain `file` `{path}`: {reason}"),
                    self.item_span(value),
                );
            }
            settings.file = Some(path);
        }
        if let Some(value) = retain.get("on-mismatch") {
            match value.as_str() {
                Some("error") => settings.on_mismatch = MismatchPolicy::Error,
                Some("reset") => settings.on_mismatch = MismatchPolicy::Reset,
                Some(other) => {
                    self.invalid_enum(
                        "STC-E-PROJECT-010",
                        "on-mismatch",
                        other,
                        "`error` or `reset`",
                        value,
                    );
                }
                None => {
                    self.type_error("STC-E-PROJECT-002", "on-mismatch", "a string", value);
                }
            }
        }
        settings
    }

    fn parse_runtime(&mut self, table: &dyn TableLike) -> RuntimeSettings {
        let mut settings = RuntimeSettings::default();
        let Some(item) = table.get("runtime") else {
            return settings;
        };
        let Some(runtime) = item.as_table_like() else {
            self.type_error("STC-E-PROJECT-002", "runtime", "a table", item);
            return settings;
        };
        self.check_unknown(runtime, "[runtime]", RUNTIME_KEYS);
        for (key, target) in [
            ("module-dir", &mut settings.module_dir),
            ("manifest-dir", &mut settings.manifest_dir),
        ] {
            if let Some(value) = runtime.get(key)
                && let Some(path) = self.expect_nonempty_string(value, "STC-E-PROJECT-010", key)
            {
                if let Some(reason) = relative_path_problem(&path) {
                    self.error(
                        "STC-E-PROJECT-010",
                        format!("invalid `{key}` `{path}`: {reason}"),
                        self.item_span(value),
                    );
                }
                *target = Some(path);
            }
        }
        if let Some(value) = runtime.get("max-retired-generations")
            && let Some(generations) =
                self.parse_u32_setting(value, "STC-E-PROJECT-010", "max-retired-generations")
        {
            settings.max_retired_generations = generations;
        }
        settings
    }

    fn parse_online(&mut self, table: &dyn TableLike) -> OnlineSettings {
        let mut settings = OnlineSettings::default();
        let Some(item) = table.get("online") else {
            return settings;
        };
        let Some(online) = item.as_table_like() else {
            self.type_error("STC-E-PROJECT-002", "online", "a table", item);
            return settings;
        };
        self.check_unknown(online, "[online]", ONLINE_KEYS);
        if let Some(value) = online.get("baseline") {
            match value.as_str() {
                Some("active") => settings.baseline = Baseline::Active,
                Some("explicit") => settings.baseline = Baseline::Explicit,
                Some(other) => {
                    self.invalid_enum(
                        "STC-E-PROJECT-010",
                        "baseline",
                        other,
                        "`active` or `explicit`",
                        value,
                    );
                }
                None => {
                    self.type_error("STC-E-PROJECT-002", "baseline", "a string", value);
                }
            }
        }
        if let Some(value) = online.get("manifest") {
            settings.manifest = self.expect_nonempty_string(value, "STC-E-PROJECT-010", "manifest");
        }
        if settings.baseline == Baseline::Explicit && settings.manifest.is_none() {
            self.error(
                "STC-E-PROJECT-010",
                "`[online].baseline = \"explicit\"` requires a `manifest` path",
                self.item_span(item),
            );
        }
        if let Some(value) = online.get("cold-policy") {
            match value.as_str() {
                Some("require-confirmation") => {
                    settings.cold_policy = ColdPolicy::RequireConfirmation;
                }
                Some(other) => {
                    self.invalid_enum(
                        "STC-E-PROJECT-010",
                        "cold-policy",
                        other,
                        "`require-confirmation`",
                        value,
                    );
                }
                None => {
                    self.type_error("STC-E-PROJECT-002", "cold-policy", "a string", value);
                }
            }
        }
        if let Some(value) = online.get("allow-untracked-removal") {
            settings.allow_untracked_removal =
                self.expect_bool(value, "STC-E-PROJECT-002", "allow-untracked-removal");
        }
        settings
    }

    fn parse_debug(&mut self, table: &dyn TableLike) -> DebugSettings {
        let mut settings = DebugSettings::default();
        let Some(item) = table.get("debug") else {
            return settings;
        };
        let Some(debug) = item.as_table_like() else {
            self.type_error("STC-E-PROJECT-002", "debug", "a table", item);
            return settings;
        };
        self.check_unknown(debug, "[debug]", DEBUG_KEYS);
        if let Some(value) = debug.get("default") {
            settings.default = self.expect_bool(value, "STC-E-PROJECT-002", "default");
            if !settings.default {
                self.warning(
                    "STC-W-PROJECT-001",
                    "`[debug].default = false` is parsed but debug information and checkpoint hooks arrive in M6.5",
                    self.item_span(value),
                );
            }
        }
        if let Some(value) = debug.get("protocol") {
            match value.as_str() {
                Some("pipe") => settings.protocol = DebugProtocol::Pipe,
                Some("socket") => settings.protocol = DebugProtocol::Socket,
                Some(other) => {
                    self.invalid_enum(
                        "STC-E-PROJECT-010",
                        "protocol",
                        other,
                        "`pipe` or `socket`",
                        value,
                    );
                }
                None => {
                    self.type_error("STC-E-PROJECT-002", "protocol", "a string", value);
                }
            }
        }
        if let Some(value) = debug.get("force-sessions-end-on-swap") {
            settings.force_sessions_end_on_swap =
                self.expect_bool(value, "STC-E-PROJECT-002", "force-sessions-end-on-swap");
        }
        settings
    }

    fn parse_lints(&mut self, table: &dyn TableLike) -> LintSettings {
        let mut settings = LintSettings::default();
        let Some(item) = table.get("lints") else {
            return settings;
        };
        let Some(lints) = item.as_table_like() else {
            self.type_error("STC-E-PROJECT-002", "lints", "a table", item);
            return settings;
        };
        for (key, value) in lints.iter() {
            let span = lints
                .key(key)
                .map_or_else(|| self.item_span(value), |key| self.key_span(key));
            if key == "warnings" {
                if let Some(level) = self.parse_lint_level(value, key) {
                    settings.warnings = level;
                }
                continue;
            }
            if !key.starts_with("STC-W-") && !key.starts_with("STC-E-") {
                self.unknown_key("[lints]", key, span, &["warnings"]);
                continue;
            }
            if !codes::is_known(key) {
                self.error(
                    "STC-E-PROJECT-011",
                    format!("unknown lint code `{key}`"),
                    span,
                );
                continue;
            }
            let Some(level) = self.parse_lint_level(value, key) else {
                continue;
            };
            if key.starts_with("STC-E-") && level != LintLevel::Deny {
                self.error(
                    "STC-E-PROJECT-011",
                    format!(
                        "lint `{key}` is an error and cannot be downgraded to `{}`",
                        level.as_str()
                    ),
                    span,
                );
                continue;
            }
            settings.codes.insert(key.to_owned(), level);
        }
        settings
    }

    fn parse_lint_level(&mut self, item: &Item, key: &str) -> Option<LintLevel> {
        match item.as_str() {
            Some("allow") => Some(LintLevel::Allow),
            Some("warn") => Some(LintLevel::Warn),
            Some("deny") => Some(LintLevel::Deny),
            Some(other) => {
                self.invalid_enum(
                    "STC-E-PROJECT-011",
                    key,
                    other,
                    "`allow`, `warn`, or `deny`",
                    item,
                );
                None
            }
            None => {
                self.type_error("STC-E-PROJECT-002", key, "a string", item);
                None
            }
        }
    }

    fn parse_run(&mut self, table: &dyn TableLike) -> RunSettings {
        let mut settings = RunSettings::default();
        let Some(item) = table.get("run") else {
            return settings;
        };
        let Some(run) = item.as_table_like() else {
            self.type_error("STC-E-PROJECT-002", "run", "a table", item);
            return settings;
        };
        self.check_unknown(run, "[run]", RUN_KEYS);
        if let Some(value) = run.get("cycles") {
            match value.as_integer() {
                Some(cycles) if cycles >= 0 => {
                    settings.cycles = u64::try_from(cycles).unwrap_or(u64::MAX);
                }
                Some(cycles) => {
                    self.error(
                        "STC-E-PROJECT-010",
                        format!("invalid `cycles` {cycles}; expected a non-negative integer"),
                        self.item_span(value),
                    );
                }
                None => {
                    self.type_error("STC-E-PROJECT-002", "cycles", "an integer", value);
                }
            }
        }
        if let Some(value) = run.get("cycle-time") {
            settings.cycle_time =
                self.parse_duration_setting(value, "STC-E-PROJECT-010", "cycle-time");
        }
        settings
    }

    fn parse_duration_setting(
        &mut self,
        item: &Item,
        code: &'static str,
        key: &str,
    ) -> Option<DurationNs> {
        let Some(text) = item.as_str() else {
            self.type_error("STC-E-PROJECT-002", key, "a duration string", item);
            return None;
        };
        match parse_duration(text) {
            Some(duration) => Some(duration),
            None => {
                self.error(
                    code,
                    format!(
                        "invalid `{key}` duration `{text}`; expected an integer and one of `ns`, `us`, `ms`, `s`, `m`, or `h`"
                    ),
                    self.item_span(item),
                );
                None
            }
        }
    }

    fn parse_test(&mut self, table: &dyn TableLike) -> TestSettings {
        let mut settings = TestSettings::default();
        let Some(item) = table.get("test") else {
            return settings;
        };
        let Some(test) = item.as_table_like() else {
            self.type_error("STC-E-PROJECT-002", "test", "a table", item);
            return settings;
        };
        self.check_unknown(test, "[test]", TEST_KEYS);
        if let Some(value) = test.get("sources") {
            settings.sources = self.parse_globs(value, "sources", "STC-E-PROJECT-004");
        }
        if let Some(value) = test.get("timeout")
            && let Some(timeout) =
                self.parse_duration_setting(value, "STC-E-PROJECT-008", "timeout")
        {
            settings.timeout = timeout;
        }
        if let Some(value) = test.get("default-cycles") {
            match value.as_integer() {
                Some(cycles) if cycles >= 1 => {
                    settings.default_cycles = u32::try_from(cycles).unwrap_or(u32::MAX);
                }
                Some(cycles) => {
                    self.error(
                        "STC-E-PROJECT-008",
                        format!("invalid `default-cycles` {cycles}; expected at least 1"),
                        self.item_span(value),
                    );
                }
                None => {
                    self.type_error("STC-E-PROJECT-002", "default-cycles", "an integer", value);
                }
            }
        }
        if let Some(value) = test.get("mock-clock")
            && let Some(clock) =
                self.parse_duration_setting(value, "STC-E-PROJECT-008", "mock-clock")
        {
            settings.mock_clock = clock;
        }
        if let Some(value) = test.get("io")
            && let Some(backend) = self.parse_io_backend(value, "STC-E-PROJECT-008", "io")
        {
            settings.io = Some(backend);
        }
        if let Some(value) = test.get("case") {
            settings.cases = self.parse_test_cases(value);
        }
        settings
    }

    fn parse_test_cases(&mut self, item: &Item) -> Vec<TestCase> {
        let Some(entries) = self.table_array(item, "STC-E-PROJECT-008", "case") else {
            return Vec::new();
        };
        let mut cases = Vec::new();
        let mut names: BTreeMap<String, Span> = BTreeMap::new();
        for (case, entry_span) in entries {
            self.check_unknown(case, "test case", TEST_CASE_KEYS);
            let Some(name) =
                self.required_nonempty_string(case, "name", "STC-E-PROJECT-008", entry_span)
            else {
                continue;
            };
            if let Some(previous) = names.get(&name) {
                let diagnostic = Diagnostic::error(
                    "STC-E-TEST-002",
                    format!("duplicate `[[test.case]]` entry for `{name}`"),
                    entry_span,
                )
                .with_label(*previous, "first entry here");
                self.diagnostics.push(diagnostic);
            } else {
                names.insert(name.clone(), entry_span);
            }
            let cycles = match case.get("cycles") {
                Some(value) => match value.as_integer() {
                    Some(cycles) if cycles >= 1 => Some(u32::try_from(cycles).unwrap_or(u32::MAX)),
                    Some(cycles) => {
                        self.error(
                            "STC-E-PROJECT-008",
                            format!("invalid test-case `cycles` {cycles}; expected at least 1"),
                            self.item_span(value),
                        );
                        None
                    }
                    None => {
                        self.type_error("STC-E-PROJECT-008", "cycles", "an integer", value);
                        None
                    }
                },
                None => None,
            };
            let timeout = match case.get("timeout") {
                Some(value) => self.parse_duration_setting(value, "STC-E-PROJECT-008", "timeout"),
                None => None,
            };
            let expect_trap = match case.get("expect-trap") {
                Some(value) => match value.as_str() {
                    Some(trap) if TRAP_CODES.contains(&trap) => Some(trap.to_owned()),
                    Some(trap) => {
                        self.error(
                            "STC-E-PROJECT-008",
                            format!(
                                "unknown trap code `{trap}`; expected one of {}",
                                TRAP_CODES.join(", ")
                            ),
                            self.item_span(value),
                        );
                        None
                    }
                    None => {
                        self.type_error("STC-E-PROJECT-008", "expect-trap", "a string", value);
                        None
                    }
                },
                None => None,
            };
            cases.push(TestCase {
                name,
                cycles,
                timeout,
                expect_trap,
                span: entry_span,
            });
        }
        cases
    }

    fn check_unknown(&mut self, table: &dyn TableLike, kind: &str, allowed: &[&str]) {
        for (key, value) in table.iter() {
            if allowed.contains(&key) {
                continue;
            }
            let span = table
                .key(key)
                .map_or_else(|| self.item_span(value), |key| self.key_span(key));
            self.unknown_key(kind, key, span, allowed);
        }
    }

    fn unknown_key(&mut self, kind: &str, key: &str, span: Span, allowed: &[&str]) {
        let message = format!("unknown {kind} key `{key}`");
        let mut diagnostic = Diagnostic::error("STC-E-PROJECT-002", message, span);
        if let Some(suggestion) = did_you_mean(key, allowed) {
            diagnostic = diagnostic.with_suggestion(
                format!("did you mean `{suggestion}`?"),
                span,
                suggestion,
            );
        } else {
            diagnostic = diagnostic.with_note(format!("known keys: {}", allowed.join(", ")));
        }
        self.diagnostics.push(diagnostic);
    }

    fn type_error(&mut self, code: &'static str, key: &str, expected: &str, item: &Item) {
        self.error(
            code,
            format!("`{key}` must be {expected}, found {}", item.type_name()),
            self.item_span(item),
        );
    }

    fn invalid_enum(
        &mut self,
        code: &'static str,
        key: &str,
        value: &str,
        expected: &str,
        item: &Item,
    ) {
        let span = self.item_span(item);
        self.invalid_enum_value(code, key, value, expected, span);
    }

    fn invalid_enum_value(
        &mut self,
        code: &'static str,
        key: &str,
        value: &str,
        expected: &str,
        span: Span,
    ) {
        self.error(
            code,
            format!("invalid `{key}` value `{value}`; expected {expected}"),
            span,
        );
    }

    fn expect_bool(&mut self, item: &Item, code: &'static str, key: &str) -> bool {
        match item.as_bool() {
            Some(value) => value,
            None => {
                self.type_error(code, key, "a boolean", item);
                false
            }
        }
    }

    fn expect_nonempty_string(
        &mut self,
        item: &Item,
        code: &'static str,
        key: &str,
    ) -> Option<String> {
        let value = self.expect_string(item, key)?;
        if value.is_empty() {
            self.error(
                code,
                format!("`{key}` must not be empty"),
                self.item_span(item),
            );
            None
        } else {
            Some(value)
        }
    }

    fn expect_string(&mut self, item: &Item, key: &str) -> Option<String> {
        match item.as_str() {
            Some(value) => Some(value.to_owned()),
            None => {
                self.type_error("STC-E-PROJECT-002", key, "a string", item);
                None
            }
        }
    }

    fn required_string(
        &mut self,
        table: &dyn TableLike,
        key: &str,
        code: &'static str,
        missing_span: Span,
    ) -> Option<String> {
        match table.get(key) {
            Some(value) => self.expect_string(value, key),
            None => {
                self.error(code, format!("missing required key `{key}`"), missing_span);
                None
            }
        }
    }

    fn required_nonempty_string(
        &mut self,
        table: &dyn TableLike,
        key: &str,
        code: &'static str,
        missing_span: Span,
    ) -> Option<String> {
        match table.get(key) {
            Some(value) => self.expect_nonempty_string(value, code, key),
            None => {
                self.error(code, format!("missing required key `{key}`"), missing_span);
                None
            }
        }
    }

    fn table_array<'a>(
        &mut self,
        item: &'a Item,
        code: &'static str,
        key: &str,
    ) -> Option<Vec<(&'a dyn TableLike, Span)>> {
        if let Some(tables) = item.as_array_of_tables() {
            let mut entries = Vec::new();
            for table in tables.iter() {
                let span = self.span(table.span());
                entries.push((table as &dyn TableLike, span));
            }
            return Some(entries);
        }
        if let Some(array) = item.as_array() {
            let mut entries = Vec::new();
            for value in array.iter() {
                let span = self.span(value.span());
                if let Some(table) = value.as_inline_table() {
                    entries.push((table as &dyn TableLike, span));
                } else {
                    self.error(
                        code,
                        format!(
                            "`{key}` entries must be tables, found {}",
                            value.type_name()
                        ),
                        span,
                    );
                }
            }
            return Some(entries);
        }
        self.type_error(code, key, "an array of tables", item);
        None
    }

    fn config_value(
        &mut self,
        item: &Item,
        code: &'static str,
        key: &str,
        depth: usize,
    ) -> Option<ConfigValue> {
        if depth > CONFIG_NESTING_LIMIT {
            self.error(
                code,
                format!("`{key}` nests tables more than {CONFIG_NESTING_LIMIT} levels deep"),
                self.item_span(item),
            );
            return None;
        }
        match item {
            Item::Value(value) => self.config_scalar(value, code, key, depth),
            Item::Table(table) => self.config_table(table, code, key, depth),
            Item::ArrayOfTables(tables) => {
                let mut values = Vec::new();
                for table in tables.iter() {
                    let value = self.config_table(table, code, key, depth + 1)?;
                    values.push(value);
                }
                Some(ConfigValue::Array(values))
            }
            Item::None => None,
        }
    }

    fn config_table(
        &mut self,
        table: &dyn TableLike,
        code: &'static str,
        key: &str,
        depth: usize,
    ) -> Option<ConfigValue> {
        let mut values = BTreeMap::new();
        for (entry_key, item) in table.iter() {
            let value = self.config_value(item, code, key, depth + 1)?;
            values.insert(entry_key.to_owned(), value);
        }
        Some(ConfigValue::Table(values))
    }

    fn config_scalar(
        &mut self,
        value: &Value,
        code: &'static str,
        key: &str,
        depth: usize,
    ) -> Option<ConfigValue> {
        let converted = match value {
            Value::String(text) => ConfigValue::String(text.value().clone()),
            Value::Integer(integer) => ConfigValue::Integer(*integer.value()),
            Value::Float(float) => ConfigValue::Float(*float.value()),
            Value::Boolean(boolean) => ConfigValue::Boolean(*boolean.value()),
            Value::Datetime(datetime) => ConfigValue::Datetime(datetime.value().to_string()),
            Value::Array(array) => {
                let mut values = Vec::new();
                for element in array.iter() {
                    values.push(self.config_scalar(element, code, key, depth + 1)?);
                }
                ConfigValue::Array(values)
            }
            Value::InlineTable(table) => self.config_table(table, code, key, depth + 1)?,
        };
        Some(converted)
    }
}

fn did_you_mean(key: &str, allowed: &[&str]) -> Option<String> {
    if key.len() > SUGGESTION_KEY_LIMIT {
        return None;
    }
    let kebab = key.replace('_', "-");
    if kebab != key && allowed.contains(&kebab.as_str()) {
        return Some(kebab);
    }
    let mut best: Option<(usize, &str)> = None;
    for candidate in allowed {
        let distance = levenshtein(key, candidate);
        let limit = if candidate.len() >= 8 { 3 } else { 2 };
        if distance <= limit && best.is_none_or(|(best_distance, _)| distance < best_distance) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| (*candidate).to_owned())
}

fn levenshtein(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (row, left_char) in left.iter().enumerate() {
        current[0] = row + 1;
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_char != right_char);
            current[column + 1] = (previous[column + 1] + 1)
                .min(current[column] + 1)
                .min(substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

/// Returns whether `name` is a valid package name or dependency alias.
#[must_use]
pub(crate) fn valid_package_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn valid_version(version: &str) -> bool {
    if version.is_empty() {
        return false;
    }
    let (without_build, build) = match version.split_once('+') {
        Some((core, build)) => (core, Some(build)),
        None => (version, None),
    };
    if let Some(build) = build
        && !valid_version_identifiers(build)
    {
        return false;
    }
    let (core, prerelease) = match without_build.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease)),
        None => (without_build, None),
    };
    if let Some(prerelease) = prerelease
        && !valid_version_identifiers(prerelease)
    {
        return false;
    }
    let mut components = 0;
    for component in core.split('.') {
        if component.is_empty() || !component.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
        components += 1;
    }
    components == 3
}

fn valid_version_identifiers(identifiers: &str) -> bool {
    !identifiers.is_empty()
        && identifiers.split('.').all(|identifier| {
            !identifier.is_empty()
                && identifier
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn parse_duration(text: &str) -> Option<DurationNs> {
    let split = text
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(text.len());
    if split == 0 {
        return None;
    }
    let number: u64 = text.get(..split)?.parse().ok()?;
    let factor = match text.get(split..)? {
        "ns" => 1,
        "us" | "µs" => 1_000,
        "ms" => 1_000_000,
        "s" => 1_000_000_000,
        "m" => 60_000_000_000,
        "h" => 3_600_000_000_000,
        _ => return None,
    };
    number.checked_mul(factor).map(DurationNs)
}

/// Returns why a manifest path setting is not a project-relative path.
pub(crate) fn relative_path_problem(value: &str) -> Option<&'static str> {
    if value.is_empty() {
        return Some("must not be empty");
    }
    if value.contains('\\') {
        return Some("must use `/` separators");
    }
    if value.starts_with('/') {
        return Some("must be relative to the manifest");
    }
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Some("must be relative to the manifest");
    }
    let path = std::path::Path::new(value);
    if path.is_absolute() {
        return Some("must be relative to the manifest");
    }
    for component in path.components() {
        match component {
            Component::ParentDir => return Some("must stay inside the project"),
            Component::RootDir | Component::Prefix(_) => {
                return Some("must be relative to the manifest");
            }
            Component::Normal(_) | Component::CurDir => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names() {
        assert!(valid_package_name("motor-control"));
        assert!(valid_package_name("_internal"));
        assert!(valid_package_name("a1"));
        assert!(!valid_package_name(""));
        assert!(!valid_package_name("1abc"));
        assert!(!valid_package_name("a.b"));
        assert!(!valid_package_name("a b"));
    }

    #[test]
    fn versions() {
        assert!(valid_version("0.1.0"));
        assert!(valid_version("1.2.3-alpha.1+build.5"));
        assert!(!valid_version("1.2"));
        assert!(!valid_version("1.2.x"));
        assert!(!valid_version(""));
        assert!(!valid_version("1.2.3-"));
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("10ms"), Some(DurationNs(10_000_000)));
        assert_eq!(parse_duration("5s"), Some(DurationNs(5_000_000_000)));
        assert_eq!(parse_duration("0ns"), Some(DurationNs(0)));
        assert_eq!(parse_duration("2us"), Some(DurationNs(2_000)));
        assert_eq!(parse_duration("10"), None);
        assert_eq!(parse_duration("ms"), None);
        assert_eq!(parse_duration("10xs"), None);
        assert_eq!(parse_duration("-1s"), None);
    }

    #[test]
    fn suggestions() {
        assert_eq!(
            did_you_mean("opt_level", BUILD_KEYS).as_deref(),
            Some("opt-level")
        );
        assert_eq!(
            did_you_mean("projet", TOP_LEVEL_KEYS).as_deref(),
            Some("project")
        );
        assert_eq!(did_you_mean("zzzzzz", TOP_LEVEL_KEYS), None);
    }
}
