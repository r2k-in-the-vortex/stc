//! HIR-to-LLVM-IR code generation for the M4 scalar subset.
//!
//! The crate consumes only [`stc_hir::HirCrate`] and emits deterministic
//! textual LLVM IR plus build metadata (D-076, D-091). Constructs deferred to
//! M5 return structured [`CodegenError::Unsupported`] values; the driver maps
//! them to source diagnostics.

#![forbid(unsafe_code)]

pub mod abi;
pub mod codes;
mod config;
mod expr;
mod layout;
mod mangle;
mod metadata;
mod module;
mod stmt;
mod unit;

use std::fmt;

use stc_hir::HirCrate;
use stc_span::{FileId, Span};

pub use metadata::Metadata;

/// Artifact mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Generate `main` and call the runtime cycle loop.
    Executable,
    /// Export `stc_init`/`stc_cycle`/`stc_shutdown` and no `main`.
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
    /// Source file paths in `FileId` order.
    pub sources: &'a [String],
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
/// # Errors
///
/// Returns every unsupported-construct or internal error encountered.
pub fn generate(hir: &HirCrate, options: &Options<'_>) -> Result<Output, Vec<CodegenError>> {
    hir.check_invariants()
        .map_err(|message| vec![CodegenError::internal(message)])?;
    unit::generate(hir, options)
}
