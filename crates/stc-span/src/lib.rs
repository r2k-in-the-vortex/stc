//! Source files, byte spans, and line/column mapping.
//!
//! [Span]s are byte ranges into a source file. Files are identified by [FileId]
//! and owned by a [SourceMap], which also converts spans to 1-based line and
//! column positions (STC-LEX-005, STC-LEX-006, STC-LEX-007).
//!
//! The crate is dependency-free and contains no host-specific code.

#![forbid(unsafe_code)]

mod source_map;
mod span;
mod utf8;

pub use source_map::{Location, SourceFile, SourceMap};
pub use span::{FileId, Span};
pub use utf8::{DecodedUtf8, Utf8Issue, Utf8IssueKind, decode_utf8};
