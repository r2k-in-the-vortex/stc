//! A local Language Server Protocol implementation for `stc`.
//!
//! The crate is the protocol layer between `stc-ide` and an editor (WL-1.1
//! I-2). It owns JSON-RPC `Content-Length` framing, the LSP lifecycle, the
//! open-document store, capability dispatch, and the byte-span to UTF-16
//! position mapping. Compiler answers come from [`stc_ide::Snapshot`]; this
//! crate does not parse, analyze, or format ST itself and contains no editor
//! specific code.
//!
//! The loop is synchronous and single-threaded. A reader thread only frames
//! incoming bytes and forwards them over a channel; [`serve`] receives from
//! that channel with a timeout so repeated edits coalesce into one analysis
//! after a short quiet period.
//!
//! One server instance serves one project. The manifest comes from
//! `stc lsp --manifest-path`, otherwise from the initialize `rootUri` or
//! workspace folders, otherwise from the process working directory. When no
//! `stc.toml` is found, `initialize` fails with a readable error instead of
//! switching to a degraded single-file mode.
//!
//! The advertised capability set is closed (I-7): full document
//! synchronization, push diagnostics, document symbols, definition,
//! references, identifier hover, completion (keywords, visible names, and
//! one-level member completion after `.`), and whole-document formatting.
//! Semantic tokens, rename, code actions, range formatting, call hierarchy,
//! workspace symbols, and inlay hints are not advertised.
//!
//! All logs are written to stderr. Standard output carries protocol messages
//! only.
//!
//! ```no_run
//! // The driver entry point used by `stc lsp`.
//! let code = stc_lsp::run_stdio(None);
//! std::process::exit(i32::from(code));
//! ```

#![forbid(unsafe_code)]

pub mod documents;
pub mod framing;
pub mod position;
pub mod protocol;
pub mod server;
pub mod uri;

pub use documents::{Document, DocumentStore};
pub use framing::{FrameError, FrameEvent, FrameReader, MAX_MESSAGE_BYTES, write_message};
pub use position::{Position, PositionMap, Range};
pub use protocol::{Incoming, ProtocolError};
pub use server::{Debouncer, ServerOptions, capabilities, run_stdio, serve, server_info};
pub use uri::{path_to_uri, uri_to_path};
