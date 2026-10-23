use stc_span::Span;

use crate::{Diagnostic, Severity};

/// The default maximum number of retained diagnostics.
pub const DEFAULT_LIMIT: usize = 100;

/// Collects diagnostics from a compiler phase.
///
/// A limit bounds memory and output on pathological input. Diagnostics beyond
/// the limit are counted in [`DiagnosticSink::suppressed`] instead of being
/// stored.
#[derive(Debug)]
pub struct DiagnosticSink {
    diagnostics: Vec<Diagnostic>,
    limit: usize,
    suppressed: usize,
    suppressed_errors: usize,
}

impl Default for DiagnosticSink {
    fn default() -> Self {
        Self::new()
    }
}

impl DiagnosticSink {
    /// Creates a sink with [`DEFAULT_LIMIT`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_limit(DEFAULT_LIMIT)
    }

    /// Creates a sink retaining at most `limit` diagnostics.
    #[must_use]
    pub fn with_limit(limit: usize) -> Self {
        Self {
            diagnostics: Vec::new(),
            limit,
            suppressed: 0,
            suppressed_errors: 0,
        }
    }

    /// Creates a sink with no limit.
    #[must_use]
    pub fn unlimited() -> Self {
        Self::with_limit(usize::MAX)
    }

    /// Adds a diagnostic.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        if self.diagnostics.len() < self.limit {
            self.diagnostics.push(diagnostic);
        } else {
            self.suppressed = self.suppressed.saturating_add(1);
            if diagnostic.is_error() {
                self.suppressed_errors = self.suppressed_errors.saturating_add(1);
            }
        }
    }

    /// Adds an error diagnostic.
    pub fn error(&mut self, code: &'static str, message: impl Into<String>, span: Span) {
        self.push(Diagnostic::error(code, message, span));
    }

    /// Adds a warning diagnostic.
    pub fn warning(&mut self, code: &'static str, message: impl Into<String>, span: Span) {
        self.push(Diagnostic::warning(code, message, span));
    }

    /// Returns all retained diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Iterates over retained diagnostics.
    pub fn iter(&self) -> std::slice::Iter<'_, Diagnostic> {
        self.diagnostics.iter()
    }

    /// Returns the number of retained diagnostics.
    #[must_use]
    pub fn len(&self) -> usize {
        self.diagnostics.len()
    }

    /// Returns `true` if no diagnostics were retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// Returns the number of diagnostics dropped after the limit was reached.
    #[must_use]
    pub fn suppressed(&self) -> usize {
        self.suppressed
    }

    /// Returns the number of suppressed diagnostics that had error severity.
    #[must_use]
    pub fn suppressed_errors(&self) -> usize {
        self.suppressed_errors
    }

    /// Returns `true` if any retained or suppressed diagnostic is an error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.suppressed_errors > 0 || self.diagnostics.iter().any(Diagnostic::is_error)
    }

    /// Returns the number of retained errors.
    #[must_use]
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .count()
    }

    /// Removes and returns all retained diagnostics and resets counters.
    #[must_use]
    pub fn take(&mut self) -> Vec<Diagnostic> {
        self.suppressed = 0;
        self.suppressed_errors = 0;
        std::mem::take(&mut self.diagnostics)
    }

    /// Consumes the sink and returns its diagnostics.
    #[must_use]
    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }
}
