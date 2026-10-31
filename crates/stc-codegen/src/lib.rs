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
mod debug;
pub mod exact;
mod expr;
mod host;
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

use stc_got::{Digest32, Endianness, GotId, Manifest, Rename, Target};
use stc_hir::HirCrate;
use stc_span::{FileId, SourceMap, Span};

pub use launcher::{generate_launcher, generate_test_launcher};
pub use metadata::{
    ChannelMetadata, DebugCheckpointMetadata, DebugMetadata, DeviceMetadata, IoProfileMetadata,
    IoRoute, IoRouteEndpoint, Json, Metadata, ProjectBinding, ProjectChannel,
    RouteEndpointMetadata, RouteMetadata, VarAccessMetadata,
};
pub use module::{
    FunctionTarget, ModuleArtifacts, ModuleError, ModuleOptions, RootTarget, Severity, SlotBinding,
};

/// Artifact mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Executable host launcher plus a loadable module.
    #[default]
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
    /// Embedded machine profile, when `[io].config` selects one.
    pub profile: Option<IoProfileMetadata>,
    /// Profile routes to resolve against the HIR and device tree.
    pub routes: Vec<IoRoute>,
}

impl Default for IoMetadataOptions {
    fn default() -> Self {
        Self {
            backend: "null".into(),
            file: None,
            devices: Vec::new(),
            bindings: Vec::new(),
            profile: None,
            routes: Vec::new(),
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
///
/// The `Default` implementation is the cold production form: unoptimised
/// debug artifacts, no target override, module emit metadata, and no test
/// build. Callers that only vary a few fields (including the `fuzz` workspace)
/// can use `..Options::default()` so a new option with a sensible default does
/// not break them.
#[derive(Clone, Debug, Default)]
pub struct Options<'a> {
    /// Artifact/name stem used in `source_filename` and metadata.
    pub artifact_name: &'a str,
    /// Optional target triple emitted into the module.
    pub target: Option<&'a str>,
    /// Emit deterministic DWARF/CodeView debug metadata. When true,
    /// [`Options::source_map`] supplies the file text and real source
    /// locations. M6.5 phase 1 keeps the CLI default off; phase 2 owns the
    /// debug/release split.
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
    /// Source text used to turn spans into real line/column locations. Debug
    /// metadata is emitted only when this is present and `debug` is true.
    pub source_map: Option<&'a SourceMap>,
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

/// Front-end identity and type hints for one generated function's stopped
/// frames (Debug and Monitoring §16.9).
///
/// The front end cannot know final stack offsets, so the driver merges these
/// hints with backend-derived locations extracted from DWARF/CodeView. The
/// hints are build-time data and are not part of any identity or artifact other
/// than the additive frame sidecar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameFunctionHint {
    /// Manifest function identity.
    pub function: GotId,
    /// Code symbol.
    pub symbol: String,
    /// Parameters and locals in source declaration order.
    pub locals: Vec<FrameLocalHint>,
}

/// One parameter or local hint for the frame sidecar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameLocalHint {
    /// Identifier normalized to ASCII upper case.
    pub name: String,
    /// Canonical manifest type key.
    pub type_key: String,
    /// Storage size in bytes.
    pub size: u32,
    /// Whether this is a parameter.
    pub parameter: bool,
    /// Source byte offset of the declaration.
    pub declaration: u32,
    /// First checkpoint ID of the owning function, or 0 when the function has
    /// no checkpoints (a caller frame is then considered fully initialized).
    pub first: u32,
    /// Last checkpoint ID of the owning function, or 0 when the function has
    /// no checkpoints.
    pub last: u32,
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
    /// Frame identity/type hints; non-empty only for hook-emitting debug
    /// builds. The driver uses this to write `<name>.stc-frames.json`.
    pub frames: Vec<FrameFunctionHint>,
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
    module_options.debug_hooks = options.debug && !options.release && !options.test_build;
    let got = module::project(hir, options.sources, &module_options).map_err(project_errors)?;
    unit::generate(hir, options, &target, got, &module_options, options.sources)
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
    host::host_triple()
}
