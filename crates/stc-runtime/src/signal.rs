//! Process stop signals for the launcher's run loop.
//!
//! Ctrl-C (and Ctrl-Break on Windows) sets a process-global flag that the loop
//! checks before every scan. The handler only performs an atomic store, so it
//! is safe to run asynchronously. The launcher still shuts down normally and
//! returns exit code 130.

use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
static INSTALL: Once = Once::new();

fn request_stop() {
    STOP_REQUESTED.store(true, Ordering::Relaxed);
}

/// Installs the process stop handlers once.
pub fn install_stop_handler() {
    INSTALL.call_once(|| {
        // SAFETY: the platform installers only register a signal/console
        // handler that performs an atomic store.
        unsafe { platform::install() };
    });
}

/// Returns `true` after a stop signal has been observed.
pub fn stop_requested() -> bool {
    STOP_REQUESTED.load(Ordering::Relaxed)
}

/// Sends an interrupt to one process.
///
/// On Unix this is `SIGINT`; on Windows it is a `CTRL_BREAK_EVENT` for a
/// process created in its own process group. Returns `false` when the
/// platform rejected the request.
#[cfg(unix)]
#[must_use]
pub fn interrupt_process(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    // SAFETY: `SIGINT` has no payload and the pid names the target process.
    unsafe { platform::interrupt(pid) }
}

/// Sends an interrupt to one process.
///
/// On Unix this is `SIGINT`; on Windows it is a `CTRL_BREAK_EVENT` for a
/// process created in its own process group. Returns `false` when the
/// platform rejected the request.
#[cfg(windows)]
#[must_use]
pub fn interrupt_process(pid: u32) -> bool {
    // SAFETY: the target was created in its own process group.
    unsafe { platform::interrupt(pid) }
}

/// Sends an interrupt to one process.
///
/// Not implemented on other hosts; always returns `false`.
#[cfg(not(any(unix, windows)))]
#[must_use]
pub fn interrupt_process(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
mod platform {
    use std::os::raw::c_int;

    const SIGINT: c_int = 2;
    const SIGTERM: c_int = 15;

    type Handler = extern "C" fn(c_int);

    unsafe extern "C" {
        fn signal(signum: c_int, handler: Handler) -> usize;
        fn kill(pid: c_int, signal: c_int) -> c_int;
    }

    pub(super) unsafe fn interrupt(pid: c_int) -> bool {
        // SAFETY: `SIGINT` has no payload; the caller names the target process.
        unsafe { kill(pid, SIGINT) == 0 }
    }

    extern "C" fn handler(_signal: c_int) {
        super::request_stop();
    }

    pub(super) unsafe fn install() {
        // SAFETY: `handler` has the C `void (*)(int)` shape and only stores to
        // an atomic. The results are intentionally ignored.
        unsafe {
            signal(SIGINT, handler);
            signal(SIGTERM, handler);
        }
    }
}

#[cfg(windows)]
mod platform {
    type Handler = extern "system" fn(u32) -> i32;

    unsafe extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<Handler>, add: i32) -> i32;
        fn GenerateConsoleCtrlEvent(event: u32, group_id: u32) -> i32;
    }

    pub(super) unsafe fn interrupt(pid: u32) -> bool {
        // CTRL_BREAK_EVENT targets a process group; `pid` is the group id of
        // a process created with CREATE_NEW_PROCESS_GROUP.
        // SAFETY: the caller supplies the group id for its own child.
        unsafe { GenerateConsoleCtrlEvent(1, pid) != 0 }
    }

    extern "system" fn handler(_control: u32) -> i32 {
        super::request_stop();
        1
    }

    pub(super) unsafe fn install() {
        // SAFETY: `handler` has the `BOOL WINAPI (DWORD)` shape and only
        // performs an atomic store. The result is intentionally ignored.
        unsafe {
            SetConsoleCtrlHandler(Some(handler), 1);
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    pub(super) unsafe fn install() {}
}
