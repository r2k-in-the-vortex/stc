//! The typed `stc.toml` model and its documented defaults.
//!
//! Values are validated while parsing; this module only holds the resulting
//! data. Spans are retained where later phases need to point back at the
//! manifest (dependency declarations, source globs, I/O bindings).

use std::collections::BTreeMap;
use std::path::PathBuf;

use stc_span::Span;

/// The supported profile identifier (`[package].language`).
pub const PROFILE_ID: &str = "stc-2013-oop-utf8-v1";

/// The supported `stc.toml` schema version.
pub const MANIFEST_SCHEMA: u32 = 1;

/// The supported `stc.lock` schema version.
pub const LOCKFILE_SCHEMA: u32 = 1;

/// A parsed and validated `stc.toml`.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    /// `schema`; always [`MANIFEST_SCHEMA`] after validation.
    pub schema: u32,
    /// `[package]`.
    pub package: PackageInfo,
    /// `[dependencies]`, in declaration order.
    pub dependencies: Vec<DependencyDecl>,
    /// `[project]`.
    pub project: ProjectSettings,
    /// `[build]`.
    pub build: BuildSettings,
    /// `[io]`, `[[io.devices]]`, and `[[io.bindings]]`.
    pub io: IoConfig,
    /// `[retain]`.
    pub retain: RetainSettings,
    /// `[runtime]`.
    pub runtime: RuntimeSettings,
    /// `[online]`.
    pub online: OnlineSettings,
    /// `[debug]`.
    pub debug: DebugSettings,
    /// `[lints]`.
    pub lints: LintSettings,
    /// `[run]`.
    pub run: RunSettings,
    /// `[test]` and `[[test.case]]`.
    pub test: TestSettings,
}

/// One discovered source file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePath {
    /// Absolute path on disk.
    pub path: PathBuf,
    /// Package-relative path with forward slashes.
    pub relative: String,
}

/// `[package]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageInfo {
    /// Package identity used for catalog keys, `GotId`s, and symbols.
    pub name: String,
    /// Semver-ish version; not used for resolution in v1.
    pub version: String,
    /// Profile identifier; equal to [`PROFILE_ID`].
    pub language: String,
}

/// One local path dependency from `[dependencies]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencyDecl {
    /// The table key: the dependency's root namespace.
    pub alias: String,
    /// The manifest-relative path as written.
    pub path: String,
    /// The explicit `package` key, when present.
    pub package: Option<String>,
    /// Span of the dependency table entry.
    pub key_span: Span,
    /// Span of the `path` value.
    pub path_span: Span,
    /// Span of the `package` value, when present.
    pub package_span: Option<Span>,
}

/// One source glob from `[project].sources` or `[test].sources`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobSpec {
    /// The glob text as written.
    pub pattern: String,
    /// Span of the glob string.
    pub span: Span,
}

/// `[project]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectSettings {
    /// Production source globs, relative to the manifest.
    pub sources: Vec<GlobSpec>,
    /// Whether standard function blocks are injected.
    pub stdlib: bool,
    /// Selected `CONFIGURATION`; resolved during semantic analysis.
    pub configuration: Option<String>,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            stdlib: true,
            configuration: None,
        }
    }
}

/// `[build]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildSettings {
    /// Optimisation level passed to clang.
    pub opt_level: OptLevel,
    /// Artifact directory, relative to the manifest.
    pub out_dir: String,
    /// Static or shared host library when `lib` is emitted.
    pub lib_type: LibType,
    /// Parsed debug setting; the build-mode precedence uses the explicit
    /// marker below.
    pub debug: bool,
    /// Whether `[build].debug` was present, as opposed to defaulted.
    pub debug_explicit: bool,
    /// Whether `[build].opt-level` was present, as opposed to defaulted.
    pub opt_level_explicit: bool,
    /// Whether `[build].out-dir` was present, as opposed to defaulted.
    pub out_dir_explicit: bool,
    /// Statement terminator mode.
    pub syntax: SyntaxMode,
    /// Non-empty artifact kinds, in canonical order.
    pub emit: Vec<EmitKind>,
    /// Target triple passed to clang.
    pub target: Option<String>,
    /// clang executable override.
    pub clang: Option<String>,
    /// Runtime host archive override.
    pub runtime: Option<String>,
}

impl Default for BuildSettings {
    fn default() -> Self {
        Self {
            opt_level: OptLevel::O0,
            out_dir: "target/stc/debug".to_owned(),
            lib_type: LibType::Static,
            debug: true,
            debug_explicit: false,
            opt_level_explicit: false,
            out_dir_explicit: false,
            syntax: SyntaxMode::Strict,
            emit: vec![EmitKind::Module],
            target: None,
            clang: None,
            runtime: None,
        }
    }
}

/// `[build].opt-level`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OptLevel {
    /// `0`.
    O0,
    /// `1`.
    O1,
    /// `2`.
    O2,
    /// `3`.
    O3,
    /// `"s"`.
    Size,
}

impl OptLevel {
    /// Returns the clang `-O` argument.
    #[must_use]
    pub const fn clang_flag(self) -> &'static str {
        match self {
            Self::O0 => "-O0",
            Self::O1 => "-O1",
            Self::O2 => "-O2",
            Self::O3 => "-O3",
            Self::Size => "-Os",
        }
    }
}

/// `[build].lib-type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibType {
    /// Static host archive.
    Static,
    /// Shared host library.
    Shared,
}

/// `[build].syntax`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxMode {
    /// The 2013 statement terminator rules.
    Strict,
    /// The documented omitted-semicolon superset.
    Relaxed,
}

/// One `[build].emit` artifact kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EmitKind {
    /// LLVM IR.
    Ir,
    /// Native object.
    Obj,
    /// Loadable module and GOT manifest pair.
    Module,
    /// Thin launcher executable.
    Exe,
    /// Embedding host library.
    Lib,
    /// Metadata sidecar.
    Metadata,
}

impl EmitKind {
    /// Returns the `--emit` spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ir => "ir",
            Self::Obj => "obj",
            Self::Module => "module",
            Self::Exe => "exe",
            Self::Lib => "lib",
            Self::Metadata => "metadata",
        }
    }
}

/// `[io]` and the project device tree.
#[derive(Clone, Debug, PartialEq)]
pub struct IoConfig {
    /// Runtime backend.
    pub backend: IoBackend,
    /// File used by the `file` backend.
    pub file: Option<String>,
    /// Optional input image size in bytes.
    pub input_bytes: Option<u32>,
    /// Optional output image size in bytes.
    pub output_bytes: Option<u32>,
    /// Optional marker image size in bytes.
    pub marker_bytes: Option<u32>,
    /// Declared devices.
    pub devices: Vec<IoDevice>,
    /// Project-level bindings.
    pub bindings: Vec<IoBinding>,
}

impl Default for IoConfig {
    fn default() -> Self {
        Self {
            backend: IoBackend::Null,
            file: None,
            input_bytes: None,
            output_bytes: None,
            marker_bytes: None,
            devices: Vec::new(),
            bindings: Vec::new(),
        }
    }
}

/// `[io].backend`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoBackend {
    /// Every input unbound, outputs discarded.
    Null,
    /// Embedding/test supplied image bytes.
    Memory,
    /// File-backed process image.
    File,
}

impl IoBackend {
    /// Returns the manifest spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Memory => "memory",
            Self::File => "file",
        }
    }
}

/// One `[[io.devices]]` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct IoDevice {
    /// Unique device name.
    pub name: String,
    /// Opaque, runtime-defined device kind.
    pub kind: String,
    /// Opaque TOML copied to metadata.
    pub config: Option<ConfigValue>,
    /// Optional channel descriptors.
    pub channels: Vec<IoChannel>,
    /// Span of the device entry.
    pub span: Span,
}

/// One `[[io.devices.channels]]` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoChannel {
    /// Channel name.
    pub name: String,
    /// Data direction.
    pub direction: IoDirection,
    /// Channel width in bits.
    pub width: u32,
}

/// A data direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoDirection {
    /// Input.
    Input,
    /// Output.
    Output,
    /// Bidirectional.
    InOut,
}

impl IoDirection {
    /// Returns the manifest spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
            Self::InOut => "inout",
        }
    }
}

/// A binding's channel reference: a numeric channel or a declared name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelRef {
    /// Non-negative channel number.
    Index(u32),
    /// Declared channel name.
    Name(String),
}

/// One `[[io.bindings]]` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoBinding {
    /// Fully qualified ST variable.
    pub variable: String,
    /// Declared device name.
    pub device: String,
    /// Optional channel hint.
    pub channel: Option<ChannelRef>,
    /// Optional direction hint.
    pub direction: Option<IoDirection>,
    /// Span of the binding entry.
    pub span: Span,
}

/// An opaque TOML value copied into metadata.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigValue {
    /// A string.
    String(String),
    /// An integer.
    Integer(i64),
    /// A float.
    Float(f64),
    /// A boolean.
    Boolean(bool),
    /// A date, time, or date-time.
    Datetime(String),
    /// An array.
    Array(Vec<Self>),
    /// A table with sorted keys.
    Table(BTreeMap<String, Self>),
}

/// `[retain]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetainSettings {
    /// Whether retained state is loaded and saved.
    pub enabled: bool,
    /// State file, relative to the project root.
    pub file: Option<String>,
    /// Policy for layout/profile/format mismatches.
    pub on_mismatch: MismatchPolicy,
}

impl Default for RetainSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            file: None,
            on_mismatch: MismatchPolicy::Error,
        }
    }
}

/// `[retain].on-mismatch`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MismatchPolicy {
    /// Fail the run.
    Error,
    /// Discard the stored state and start cold.
    Reset,
}

impl MismatchPolicy {
    /// Returns the manifest spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Reset => "reset",
        }
    }
}

/// `[runtime]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeSettings {
    /// Directory holding runtime modules.
    pub module_dir: Option<String>,
    /// Directory holding runtime manifests.
    pub manifest_dir: Option<String>,
    /// Number of retired generations kept for rollback.
    pub max_retired_generations: u32,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            module_dir: None,
            manifest_dir: None,
            max_retired_generations: 1,
        }
    }
}

/// `[online]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnlineSettings {
    /// Active or explicit baseline.
    pub baseline: Baseline,
    /// Explicit baseline path; required when `baseline = "explicit"`.
    pub manifest: Option<String>,
    /// Cold-download policy.
    pub cold_policy: ColdPolicy,
    /// Whether untracked root removal is allowed without a rename.
    pub allow_untracked_removal: bool,
}

impl Default for OnlineSettings {
    fn default() -> Self {
        Self {
            baseline: Baseline::Active,
            manifest: None,
            cold_policy: ColdPolicy::RequireConfirmation,
            allow_untracked_removal: false,
        }
    }
}

/// `[online].baseline`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Baseline {
    /// The host's committed active manifest.
    Active,
    /// An explicit manifest path.
    Explicit,
}

/// `[online].cold-policy`; v1 has a single value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColdPolicy {
    /// A cold download requires confirmation.
    RequireConfirmation,
}

impl ColdPolicy {
    /// Returns the manifest spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        "require-confirmation"
    }
}

/// `[debug]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugSettings {
    /// Default debug mode.
    pub default: bool,
    /// Whether `[debug].default` was present, as opposed to defaulted.
    pub default_explicit: bool,
    /// Local transport for the M6.5 debug session.
    pub protocol: DebugProtocol,
    /// Whether an online swap ends debug sessions.
    pub force_sessions_end_on_swap: bool,
    /// Default continuous stop/idle session bound; zero disables it.
    pub session_timeout: DurationNs,
    /// Default `write` scope grant for a launched debug service.
    pub allow_write: bool,
    /// Default `force` scope grant for a launched debug service.
    pub allow_force: bool,
}

impl Default for DebugSettings {
    fn default() -> Self {
        Self {
            default: true,
            default_explicit: false,
            protocol: DebugProtocol::Pipe,
            force_sessions_end_on_swap: true,
            session_timeout: DurationNs(0),
            allow_write: false,
            allow_force: false,
        }
    }
}

/// `[debug].protocol`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugProtocol {
    /// Named pipe.
    Pipe,
    /// Unix socket.
    Socket,
}

impl DebugProtocol {
    /// Returns the manifest spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pipe => "pipe",
            Self::Socket => "socket",
        }
    }
}

/// `[lints]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintSettings {
    /// Baseline level for warning diagnostics.
    pub warnings: LintLevel,
    /// Per-code overrides, sorted by code.
    pub codes: BTreeMap<String, LintLevel>,
}

impl Default for LintSettings {
    fn default() -> Self {
        Self {
            warnings: LintLevel::Warn,
            codes: BTreeMap::new(),
        }
    }
}

/// A lint override level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LintLevel {
    /// Silenced.
    Allow,
    /// Reported as a warning.
    Warn,
    /// Reported as an error.
    Deny,
}

impl LintLevel {
    /// Returns the manifest spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Warn => "warn",
            Self::Deny => "deny",
        }
    }
}

/// `[run]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunSettings {
    /// Cycle bound; zero means until interrupted.
    pub cycles: u64,
    /// Fixed cycle time.
    pub cycle_time: Option<DurationNs>,
}

/// A non-negative duration in nanoseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DurationNs(pub u64);

impl DurationNs {
    /// Returns the nanosecond count.
    #[must_use]
    pub const fn as_nanos(self) -> u64 {
        self.0
    }

    /// Returns the duration in milliseconds, truncated.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0 / 1_000_000
    }
}

/// `[test]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestSettings {
    /// Test-only source globs, relative to the manifest.
    pub sources: Vec<GlobSpec>,
    /// Default per-test timeout.
    pub timeout: DurationNs,
    /// Default cycle count for FB/program tests.
    pub default_cycles: u32,
    /// Initial mock-clock time.
    pub mock_clock: DurationNs,
    /// Test I/O backend; `None` selects `memory`.
    pub io: Option<IoBackend>,
    /// Per-POU overrides.
    pub cases: Vec<TestCase>,
}

impl Default for TestSettings {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            timeout: DurationNs(5_000_000_000),
            default_cycles: 1,
            mock_clock: DurationNs(0),
            io: None,
            cases: Vec::new(),
        }
    }
}

impl TestSettings {
    /// Returns the effective test I/O backend (`memory` when unset).
    #[must_use]
    pub const fn effective_io(&self) -> IoBackend {
        match self.io {
            Some(backend) => backend,
            None => IoBackend::Memory,
        }
    }
}

/// One `[[test.case]]` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestCase {
    /// Test POU name.
    pub name: String,
    /// Cycle count override.
    pub cycles: Option<u32>,
    /// Timeout override.
    pub timeout: Option<DurationNs>,
    /// Expected trap code.
    pub expect_trap: Option<String>,
    /// Span of the test-case entry.
    pub span: Span,
}
