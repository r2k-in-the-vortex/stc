//! A local Debug Adapter Protocol server that translates DAP to
//! `stc-debug/1.1` (WL-1.1 I-3 and I-9).
//!
//! The crate owns DAP `Content-Length` framing, the request/response/event
//! sequence, launch/attach argument handling, breakpoint and stepping
//! translation, stack/scopes/variables/evaluate responses, and source-path
//! mapping. It does not build, launch, or connect the runtime host: the driver
//! implements [`Launcher`] and hands back a connected `DebugClient` plus
//! process ownership, so this translation layer is testable on its own.
//!
//! The loop is synchronous and single-threaded. A reader thread only frames
//! standard input and forwards request bodies over a channel; the session loop
//! polls the runtime client between requests. There is no async runtime.
//!
//! The advertised DAP capability set is closed: launch/attach, source
//! breakpoints, continue/pause/next/stepIn/stepOut, one thread, stack traces,
//! Locals (1.1 hosts only) and Globals scopes, variable expansion,
//! `setVariable` when `allowWrite` is enabled, evaluate for a stopped-frame
//! local name or manifest target path, terminate, and disconnect.
//! Conditional/hit/log/function/data/instruction breakpoints, exception
//! breakpoints, restart, multi-thread debugging, stop-on-entry, and ST
//! expression evaluation are unsupported and reported honestly.
//!
//! Standard output carries DAP messages only; logs go to stderr. The runtime
//! non-safety notice is emitted as an `output` event at every session start.
//!
//! ```no_run
//! // The driver entry point used by `stc dap`.
//! struct Launcher;
//! impl stc_dap::Launcher for Launcher {
//!     fn launch(
//!         &mut self,
//!         _arguments: &stc_dap::LaunchArguments,
//!     ) -> Result<stc_dap::LaunchedSession, String> {
//!         Err("not configured".into())
//!     }
//!     fn attach(
//!         &mut self,
//!         _arguments: &stc_dap::LaunchArguments,
//!     ) -> Result<stc_dap::LaunchedSession, String> {
//!         Err("not configured".into())
//!     }
//! }
//! let code = stc_dap::run_stdio(&mut Launcher, stc_dap::DapOptions::default());
//! std::process::exit(i32::from(code));
//! ```

#![forbid(unsafe_code)]

pub mod adapter;
pub mod framing;
pub mod launcher;
pub mod protocol;
pub mod source;
mod value;

pub use adapter::{run_stdio, serve};
pub use framing::{FrameError, FrameReader, MAX_MESSAGE_BYTES, write_message};
pub use launcher::{
    DapOptions, LaunchedSession, Launcher, NON_SAFETY_NOTICE, OutputLine, SessionHost,
};
pub use protocol::{LaunchArguments, Request, capabilities, event, parse_request, response};
pub use source::SourceTable;
