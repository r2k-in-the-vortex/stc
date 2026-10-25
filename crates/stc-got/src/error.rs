//! Error and diagnostic types for the GOT identity layer.

use std::fmt;

use crate::digest::GotId;

/// A crate-local result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Stable diagnostic codes from GOT Artifacts §6.10.
pub mod codes {
    /// Malformed module descriptor.
    pub const MODULE_INVALID: &str = "STC-E-MODULE-001";
    /// Malformed or unpaired manifest.
    pub const MANIFEST_PAIRING: &str = "STC-E-MANIFEST-001";
    /// Invalid manifest schema, digest, `GotId`, or duplicate key.
    pub const MANIFEST_INVALID: &str = "STC-E-MANIFEST-002";
    /// Untracked removal or rename requires an explicit policy.
    pub const MANIFEST_UNTRACKED: &str = "STC-E-MANIFEST-003";
    /// Online-change plan invalid or inconsistent.
    pub const MANIFEST_PLAN: &str = "STC-E-MANIFEST-004";
    /// Incompatible root type or layout.
    pub const ONLINE_LAYOUT: &str = "STC-E-ONLINE-001";
    /// Incompatible export signature or removed export.
    pub const ONLINE_EXPORT: &str = "STC-E-ONLINE-002";
    /// Incompatible vtable or descriptor shape.
    pub const ONLINE_VTABLE: &str = "STC-E-ONLINE-003";
    /// Incompatible configuration or task topology.
    pub const ONLINE_TOPOLOGY: &str = "STC-E-ONLINE-004";
    /// Removed root quarantined.
    pub const W_ONLINE_REMOVED: &str = "STC-W-ONLINE-001";
    /// Initializer ignored on a preserved root.
    pub const W_ONLINE_INITIALIZER: &str = "STC-W-ONLINE-002";
    /// Direct-variable address remapped during swap.
    pub const W_ONLINE_DIRECT: &str = "STC-W-ONLINE-003";
    /// Stale reference may point at quarantined storage.
    pub const W_ONLINE_STALE_REF: &str = "STC-W-ONLINE-004";
    /// Baseline manifest is not current.
    pub const W_ONLINE_STALE_BASELINE: &str = "STC-W-ONLINE-005";
    /// Manifest missing or invalid; cold download required.
    pub const W_ONLINE_MISSING: &str = "STC-W-ONLINE-006";
    /// No baseline manifest; build uses a cold layout.
    pub const W_MANIFEST_COLD: &str = "STC-W-MANIFEST-001";
    /// Untracked rename or removal will discard state.
    pub const W_MANIFEST_UNTRACKED: &str = "STC-W-MANIFEST-002";
    /// Baseline source fingerprint differs from the current source tree.
    pub const W_MANIFEST_SOURCE: &str = "STC-W-MANIFEST-003";
}

/// A structured parsing, validation, or identity error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// Stable diagnostic code.
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
}

impl Error {
    /// Creates an error with a stable code.
    #[must_use]
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Creates an invalid-manifest error.
    #[must_use]
    pub fn manifest(message: impl Into<String>) -> Self {
        Self::new(codes::MANIFEST_INVALID, message)
    }

    /// Creates a plan-inconsistency error.
    #[must_use]
    pub fn plan(message: impl Into<String>) -> Self {
        Self::new(codes::MANIFEST_PLAN, message)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

/// Severity of a compiler diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// A non-fatal condition.
    Warning,
    /// A condition that makes the build undeployable.
    Error,
}

impl Severity {
    /// Returns the manifest tag.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Warning => 1,
            Self::Error => 2,
        }
    }

    /// Returns the canonical JSON name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    /// Parses the manifest tag.
    #[must_use]
    pub const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            1 => Some(Self::Warning),
            2 => Some(Self::Error),
            _ => None,
        }
    }

    /// Parses the canonical JSON name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "warning" => Some(Self::Warning),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

/// One compiler diagnostic produced while comparing against a baseline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Stable diagnostic code.
    pub code: &'static str,
    /// Warning or error.
    pub severity: Severity,
    /// Human-readable message.
    pub message: String,
    /// Affected identity, when known.
    pub got_id: Option<GotId>,
    /// Baseline key, when relevant.
    pub old_key: Option<String>,
    /// New key, when relevant.
    pub new_key: Option<String>,
}

impl Diagnostic {
    /// Creates a warning.
    #[must_use]
    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Warning,
            message: message.into(),
            got_id: None,
            old_key: None,
            new_key: None,
        }
    }

    /// Creates an error.
    #[must_use]
    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Error,
            message: message.into(),
            got_id: None,
            old_key: None,
            new_key: None,
        }
    }

    /// Attaches an identity.
    #[must_use]
    pub fn with_got_id(mut self, got_id: GotId) -> Self {
        self.got_id = Some(got_id);
        self
    }

    /// Attaches the baseline key.
    #[must_use]
    pub fn with_old_key(mut self, key: impl Into<String>) -> Self {
        self.old_key = Some(key.into());
        self
    }

    /// Attaches the new key.
    #[must_use]
    pub fn with_new_key(mut self, key: impl Into<String>) -> Self {
        self.new_key = Some(key.into());
        self
    }

    /// Returns `true` when this diagnostic is an error.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// A fatal build failure carrying every diagnostic that caused it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildError {
    /// Diagnostics in canonical order.
    pub diagnostics: Vec<Diagnostic>,
}

impl BuildError {
    /// Creates a build error from one diagnostic.
    #[must_use]
    pub fn single(diagnostic: Diagnostic) -> Self {
        Self {
            diagnostics: vec![diagnostic],
        }
    }

    /// Creates a build error from an identity error.
    #[must_use]
    pub fn from_error(error: Error) -> Self {
        Self::single(Diagnostic {
            code: error.code,
            severity: Severity::Error,
            message: error.message,
            got_id: None,
            old_key: None,
            new_key: None,
        })
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            if index > 0 {
                formatter.write_str("; ")?;
            }
            write!(formatter, "{}: {}", diagnostic.code, diagnostic.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for BuildError {}

impl From<Error> for BuildError {
    fn from(error: Error) -> Self {
        Self::from_error(error)
    }
}
