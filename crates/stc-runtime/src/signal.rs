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
pub(crate) fn install_stop_handler() {
    INSTALL.call_once(|| {
        // SAFETY: the platform installers only register a signal/console
        // handler that performs an atomic store.
        unsafe { platform::install() };
    });
}

/// Returns `true` after a stop signal has been observed.
pub(crate) fn stop_requested() -> bool {
    STOP_REQUESTED.load(Ordering::Relaxed)
}

#[cfg(unix)]
mod platform {
    use std::os::raw::c_int;

    const SIGINT: c_int = 2;
    const SIGTERM: c_int = 15;

    type Handler = extern "C" fn(c_int);

    unsafe extern "C" {
        fn signal(signum: c_int, handler: Handler) -> usize;
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
