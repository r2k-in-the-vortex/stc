//! Diagnostics, severities, and human/JSON rendering.
//!
//! Diagnostic codes are stable strings (`STC-E-*`, `STC-W-*`, `STC-I-*`). This
//! crate owns the data model, the sink, and both output formats; it does not
//! know about specific language rules.

#![forbid(unsafe_code)]

mod diagnostic;
mod render;
mod sink;

pub use diagnostic::{Diagnostic, Label, Severity, Suggestion};
pub use render::{render_human, render_json};
pub use sink::{DEFAULT_LIMIT, DiagnosticSink};
