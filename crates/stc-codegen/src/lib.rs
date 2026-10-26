//! HIR-to-LLVM-IR code generation for the M4 scalar subset.
//!
//! The crate consumes only [`stc_hir::HirCrate`] and emits deterministic
//! textual LLVM IR plus build metadata (D-076, D-091). Constructs deferred to
//! M5 return structured [`CodegenError::Unsupported`] values; the driver maps
//! them to source diagnostics. The textual builder ([`ir`]) and the module
//! structural verifier ([`verify`]) are exposed for module-mode code
//! generation and for tests that assemble known-good IR shapes.

#![forbid(unsafe_code)]

pub mod abi;
mod call;
pub mod codes;
mod config;
mod expr;
pub mod ir;
mod launcher;
mod layout;
mod mangle;
mod metadata;
pub mod module;
mod oop;
mod place;
mod reference;
mod stdlib;
mod stmt;
mod types;
mod unit;
pub mod verify;

use std::fmt;

use stc_got::{Digest32, Endianness, Manifest, Rename, Target};
use stc_hir::HirCrate;
use stc_span::{FileId, Span};

pub use launcher::{generate_launcher, generate_test_launcher};
pub use metadata::{
    ChannelMetadata, DeviceMetadata, Json, Metadata, ProjectBinding, ProjectChannel,
    VarAccessMetadata,
};
pub use module::{
    FunctionTarget, ModuleArtifacts, ModuleError, ModuleOptions, RootTarget, Severity, SlotBinding,
};

/// Artifact mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Executable host launcher plus a loadable module.
    Executable,
    /// Embedding host library plus a loadable module.
    Library,
}

impl Mode {
    /// Returns the metadata spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Executable => "executable",
            Self::Library => "library",
        }
    }
}

/// Identity and baseline inputs for the module projection.
///
/// The generated artifact is always a loadable module; these values decide the
/// persistent identities, the manifest, and the paired descriptor.
#[derive(Clone, Debug)]
pub struct ModuleBuildOptions<'a> {
    /// Compiler package version embedded in the manifest and descriptor.
    pub compiler_version: String,
    /// Hash of the production source files.
    pub source_fingerprint: Digest32,
    /// Optional identity baseline.
    pub baseline: Option<&'a Manifest>,
    /// Force a cold-layout build and ignore the baseline.
    pub cold: bool,
    /// Permit untracked removal of data roots.
    pub allow_untracked_removal: bool,
    /// Tracked rename/move maps.
    pub renames: Vec<Rename>,
}

impl Default for ModuleBuildOptions<'_> {
    fn default() -> Self {
        Self {
            compiler_version: format!("stc {}", env!("CARGO_PKG_VERSION")),
            // Deterministic non-zero placeholder; the driver passes the real
            // source fingerprint.
            source_fingerprint: Digest32::from_bytes([0xA5; 32]),
            baseline: None,
            cold: false,
            allow_untracked_removal: false,
            renames: Vec::new(),
        }
    }
}

/// Retained-state policy copied into module metadata (State §6).
#[derive(Clone, Debug)]
pub struct RetainMetadataOptions {
    /// Whether retained state is loaded and saved.
    pub enabled: bool,
    /// State file relative to the project root; `None` uses
    /// `<artifact>.stcstate`.
    pub file: Option<String>,
    /// `error` or `reset`.
    pub on_mismatch: &'static str,
}

impl Default for RetainMetadataOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            file: None,
            on_mismatch: "error",
        }
    }
}

/// Project I/O metadata copied from `stc.toml` (IO §5).
#[derive(Clone, Debug)]
pub struct IoMetadataOptions {
    /// Runtime backend (`null`, `memory`, or `file`).
    pub backend: String,
    /// File used by the `file` backend, relative to the project root.
    pub file: Option<String>,
    /// Declared devices.
    pub devices: Vec<DeviceMetadata>,
    /// Project bindings resolved against the HIR.
    pub bindings: Vec<ProjectBinding>,
}

impl Default for IoMetadataOptions {
    fn default() -> Self {
        Self {
            backend: "null".into(),
            file: None,
            devices: Vec::new(),
            bindings: Vec::new(),
        }
    }
}

/// Project-level values that shape metadata but not the module projection.
#[derive(Clone, Debug, Default)]
pub struct ProjectOptions {
    /// Retained-state policy.
    pub retain: RetainMetadataOptions,
    /// Project I/O metadata.
    pub io: IoMetadataOptions,
}

/// Code generation options.
#[derive(Clone, Debug)]
pub struct Options<'a> {
    /// Artifact/name stem used in `source_filename` and metadata.
    pub artifact_name: &'a str,
    /// Optional target triple emitted into the module.
    pub target: Option<&'a str>,
    /// Debug flag; M4 has no debug info yet.
    pub debug: bool,
    /// Whether metadata is requested (`--emit=metadata`).
    pub emit_metadata: bool,
    /// Artifact mode.
    pub mode: Mode,
    /// Relaxed syntax was accepted.
    pub relaxed: bool,
    /// Optimised, stripped build (`--release`).
    pub release: bool,
    /// Source file paths in `FileId` order.
    pub sources: &'a [String],
    /// Explicit package-relative source paths in `FileId` order. When absent,
    /// `sources` is reduced to package-relative paths.
    pub source_paths: Option<&'a [String]>,
    /// Module identity and baseline inputs.
    pub module: ModuleBuildOptions<'a>,
    /// Project-level metadata settings.
    pub project: ProjectOptions,
    /// Whether this is a test build: appends the test-clock service (host ABI
    /// 2.2) and the test-only prelude lowering. Test artifacts are cold,
    /// non-deployable, and never part of `[build].emit`.
    pub test_build: bool,
}

/// One harness-visible span-table entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpanEntry {
    /// Trap span ID.
    pub id: u32,
    /// Source file.
    pub file: FileId,
    /// Start byte.
    pub start: u32,
    /// End byte.
    pub end: u32,
}

/// The result of a successful generation.
#[derive(Clone, Debug)]
pub struct Output {
    /// Rendered textual LLVM IR.
    pub module: String,
    /// Metadata document.
    pub metadata: Metadata,
    /// Span table entries in ID order.
    pub spans: Vec<SpanEntry>,
    /// The module projection: descriptor, manifest, canonical slots, and the
    /// canonical JSON/binary manifest bytes.
    pub got: ModuleArtifacts,
}

/// A structured code generation failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodegenError {
    /// A valid HIR construct that M4 does not implement.
    Unsupported {
        /// Stable diagnostic code.
        code: &'static str,
        /// Human-readable message.
        message: String,
        /// Primary source span.
        span: Span,
    },
    /// An internal invariant failed.
    Internal {
        /// Human-readable message.
        message: String,
    },
}

impl CodegenError {
    /// Creates an unsupported-construct error.
    #[must_use]
    pub fn unsupported(code: &'static str, message: String, span: Span) -> Self {
        Self::Unsupported {
            code,
            message,
            span,
        }
    }

    /// Creates an internal error.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Attaches `span` when the error has no useful location yet.
    #[must_use]
    pub fn with_span(self, span: Span) -> Self {
        match self {
            Self::Unsupported {
                code,
                message,
                span: existing,
            } => {
                if existing.start == 0 && existing.end == 0 {
                    Self::Unsupported {
                        code,
                        message,
                        span,
                    }
                } else {
                    Self::Unsupported {
                        code,
                        message,
                        span: existing,
                    }
                }
            }
            other => other,
        }
    }

    /// Returns the stable diagnostic code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unsupported { code, .. } => code,
            Self::Internal { .. } => codes::CODEGEN_INTERNAL,
        }
    }

    /// Returns the message.
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::Unsupported { message, .. } | Self::Internal { message } => message,
        }
    }

    /// Returns the source span for unsupported errors.
    #[must_use]
    pub fn span(&self) -> Option<Span> {
        match self {
            Self::Unsupported { span, .. } => Some(*span),
            Self::Internal { .. } => None,
        }
    }
}

impl fmt::Display for CodegenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code(), self.message())
    }
}

/// Generates IR and metadata for a checked HIR crate.
///
/// The artifact is a loadable ABI 2 module. `exe` and `lib` host wrappers are
/// separate artifacts emitted by the driver around the same module.
///
/// # Errors
///
/// Returns every unsupported-construct, identity, projection, or internal
/// error encountered.
pub fn generate(hir: &HirCrate, options: &Options<'_>) -> Result<Output, Vec<CodegenError>> {
    hir.check_invariants()
        .map_err(|message| vec![CodegenError::internal(message)])?;
    let target = resolve_target(options.target);
    let mut module_options = ModuleOptions::new(
        target.clone(),
        options.artifact_name,
        options.module.compiler_version.clone(),
        options.module.source_fingerprint,
    );
    module_options.release = options.release;
    module_options.relaxed = options.relaxed;
    module_options.source_paths = options.source_paths;
    module_options.baseline = options.module.baseline;
    module_options.cold = options.module.cold;
    module_options.allow_untracked_removal = options.module.allow_untracked_removal;
    module_options.renames = options.module.renames.clone();
    module_options.test_build = options.test_build;
    let got = module::project(hir, options.sources, &module_options).map_err(project_errors)?;
    unit::generate(hir, options, &target, got)
}

fn project_errors(errors: Vec<ModuleError>) -> Vec<CodegenError> {
    // `module::project` returns only errors here; non-fatal warnings are
    // carried in `ModuleArtifacts::diagnostics` on success.
    errors
        .into_iter()
        .map(|error| CodegenError::Unsupported {
            code: error.code,
            message: error.message,
            span: error.span.unwrap_or_else(|| Span::empty(FileId::new(0), 0)),
        })
        .collect()
}

/// Resolves the build target triple.
///
/// Every supported target is little-endian with 64-bit pointers.
#[must_use]
pub fn resolve_target(target: Option<&str>) -> Target {
    let triple = target.map_or_else(host_triple, str::to_owned);
    Target::new(triple, Endianness::Little, 8)
}

/// Returns the host target triple used when `--target` is omitted.
#[must_use]
pub fn host_triple() -> String {
    if cfg!(all(windows, target_env = "msvc")) {
        "x86_64-pc-windows-msvc".into()
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu".into()
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu".into()
    } else {
        "x86_64-unknown-linux-gnu".into()
    }
}
