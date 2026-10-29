//! Host launch/attach abstraction and DAP entry points.
//!
//! The driver owns building, spawning, and connecting the runtime host; the
//! adapter owns DAP translation. [`Launcher`] is the seam: the driver
//! implements it and returns a connected [`DebugClient`] plus optional process
//! ownership. Keeping the trait here means protocol translation is testable
//! with a fake connection.

use std::path::PathBuf;

use stc_runtime::DebugClient;

use crate::protocol::LaunchArguments;

/// One line of launcher output forwarded as a DAP `output` event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputLine {
    /// DAP output category: `stdout`, `stderr`, or `console`.
    pub category: &'static str,
    /// The text, with a trailing newline already normalized by the caller.
    pub text: String,
}

/// Ownership of the launched process (a launch-owned host).
///
/// The adapter drains output while the session runs and shuts the host down
/// through the control channel when the session ends. Attach sessions have no
/// host: the process was not started by the adapter.
pub trait SessionHost {
    /// Returns output produced since the last call without blocking.
    fn drain_output(&mut self) -> Vec<OutputLine>;
    /// Returns the host's exit code when the process has exited.
    fn try_exit(&mut self) -> Option<i32>;
    /// Stops the host through the control channel and reaps the process.
    fn shutdown(&mut self);
}

/// A connected runtime session handed to the adapter.
pub struct LaunchedSession {
    /// Connected `stc-debug/1` client. The adapter starts the protocol session
    /// itself so it can negotiate 1.1 and fall back once to 1.0.
    pub client: DebugClient,
    /// Process ownership for a launch-owned host; `None` for attach.
    pub host: Option<Box<dyn SessionHost>>,
}

/// Builds, launches, and attaches the runtime host.
pub trait Launcher {
    /// Builds (unless `noBuild`) and starts a host, returning a connected
    /// client and process ownership.
    ///
    /// # Errors
    ///
    /// Returns a readable message when the build fails or the host cannot be
    /// started or connected.
    fn launch(&mut self, arguments: &LaunchArguments) -> Result<LaunchedSession, String>;

    /// Connects to an existing endpoint file without owning the host.
    ///
    /// # Errors
    ///
    /// Returns a readable message when the endpoint is missing or the
    /// connection fails.
    fn attach(&mut self, arguments: &LaunchArguments) -> Result<LaunchedSession, String>;
}

/// Adapter options fixed at process start (WL-1.1 I-3).
#[derive(Clone, Debug, Default)]
pub struct DapOptions {
    /// `stc dap --manifest-path` fallback when a launch configuration omits
    /// `project`.
    pub manifest_path: Option<PathBuf>,
}

/// The runtime non-safety notice emitted as an `output` event at every
/// session start (WL-1.1 I-14). The `stc debug` CLI prints the same text.
pub const NON_SAFETY_NOTICE: &str = "debug sessions are availability features, not safety functions; a stopped or forced controller may hold outputs indefinitely. stc may control machinery, but it is not a safety PLC and safety must be provided by a separate system and machine design as required by law and regulation";
