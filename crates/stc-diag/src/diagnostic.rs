use std::fmt;

use stc_span::Span;

/// Severity of a diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    /// Compilation cannot continue to a valid artifact.
    Error,
    /// Compilation continues, but the source is suspicious or deprecated.
    Warning,
}

impl Severity {
    /// Returns the lowercase name used in human and JSON output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A secondary span with an explanatory message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    /// The span this label points at.
    pub span: Span,
    /// The label text.
    pub message: String,
}

/// A machine-applicable replacement offered to the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// The help text.
    pub message: String,
    /// The span to replace.
    pub span: Span,
    /// The replacement text.
    pub replacement: String,
}

/// A compiler diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Stable diagnostic code, for example `STC-E-LEX-002`.
    pub code: &'static str,
    /// Error or warning.
    pub severity: Severity,
    /// Primary message.
    pub message: String,
    /// The span the message is about.
    pub primary: Span,
    /// Additional spans with messages.
    pub labels: Vec<Label>,
    /// Free-form notes, rendered after the snippet.
    pub notes: Vec<String>,
    /// Suggested replacements.
    pub suggestions: Vec<Suggestion>,
}

impl Diagnostic {
    /// Creates an error diagnostic.
    #[must_use]
    pub fn error(code: &'static str, message: impl Into<String>, primary: Span) -> Self {
        Self::new(Severity::Error, code, message, primary)
    }

    /// Creates a warning diagnostic.
    #[must_use]
    pub fn warning(code: &'static str, message: impl Into<String>, primary: Span) -> Self {
        Self::new(Severity::Warning, code, message, primary)
    }

    fn new(
        severity: Severity,
        code: &'static str,
        message: impl Into<String>,
        primary: Span,
    ) -> Self {
        Self {
            code,
            severity,
            message: message.into(),
            primary,
            labels: Vec::new(),
            notes: Vec::new(),
            suggestions: Vec::new(),
        }
    }

    /// Adds a secondary label.
    #[must_use]
    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            message: message.into(),
        });
        self
    }

    /// Adds a note.
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Adds a suggested replacement.
    #[must_use]
    pub fn with_suggestion(
        mut self,
        message: impl Into<String>,
        span: Span,
        replacement: impl Into<String>,
    ) -> Self {
        self.suggestions.push(Suggestion {
            message: message.into(),
            span,
            replacement: replacement.into(),
        });
        self
    }

    /// Returns `true` if this diagnostic has error severity.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}
