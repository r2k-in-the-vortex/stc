//! IDE-facing project snapshots and compiler queries.
//!
//! This crate is the editor layer between the compiler crates and a language
//! server (WL-1.1 I-1). It loads a project with
//! [`stc_manifest::load_project_readonly`], parses every production source and
//! the root package's test sources exactly as the driver does, and analyzes the
//! merged package graph with `stc_sema`. It owns the resulting
//! `stc_sema::Analysis`, the source texts, the file-id to display-path mapping,
//! and per-file
//! diagnostics.
//!
//! Queries use the same resolution data as `stc rename`: the innermost CST node
//! at a byte offset maps to a [`stc_sema::Res`], then to a declaration span or a
//! set of reference tokens. All public query positions are **byte offsets** that
//! are clamped to the source length; the language server converts line/column
//! and UTF-16 positions itself. [`Snapshot::location`] and
//! [`Snapshot::span_location`] expose the compiler's 1-based Unicode-scalar
//! line/column mapping for tests and clients.
//!
//! The supported query set is the WL-1.1 LSP set (I-7): diagnostics, document
//! symbols, definition, references, identifier hover, bounded completion
//! (keywords, visible names, and one-level member completion after `.`), and
//! whole-document formatting. There is no LSP or DAP protocol code here.
//!
//! ```no_run
//! use stc_ide::Snapshot;
//!
//! let snapshot = Snapshot::load("examples/hello/stc.toml")?;
//! let file = snapshot.files()[0].id;
//! let source = snapshot.source(file).expect("project source");
//! let offset = source.find("AddTwo").expect("declaration");
//! let definition = snapshot.definition(file, offset);
//! assert!(definition.is_some());
//! # Ok::<(), stc_ide::SnapshotError>(())
//! ```
//!
//! Unsaved editor buffers are supplied with [`Snapshot::set_overlay`]; the
//! snapshot re-parses the affected file and re-runs the project analysis
//! without touching the disk. [`Snapshot::clear_overlay`] restores the file
//! from disk. Editor servers that publish parse diagnostics before a
//! debounced analysis use [`Snapshot::set_overlay_parsed`],
//! [`Snapshot::clear_overlay_parsed`], and [`Snapshot::reanalyze`] to split
//! the re-parse from the re-analysis.

#![forbid(unsafe_code)]

mod completion;
mod query;
mod snapshot;
mod symbols;

pub use completion::{CompletionItem, CompletionKind};
pub use query::HoverInfo;
pub use snapshot::{
    FormatResult, Snapshot, SnapshotError, SourceFileInfo, SourceLocation, TextEdit,
};
pub use symbols::{DocumentSymbol, SymbolKind};
