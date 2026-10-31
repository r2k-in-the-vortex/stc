//! Operating-system priority for host service threads (D-204).
//!
//! The cycle thread runs at the process default priority. Control-server and
//! debug-server accept/handler threads are lowered one class so maintenance
//! and monitoring work cannot compete with the scan. The policy is advisory:
//! container or sandbox restrictions may reject it, and the runtime never
//! fails a session because priority could not be changed.
//!
//! v1 does not pin CPU affinity. Pinning conflicts with container cpusets and
//! is a deployment decision; the host keeps the process's inherited mask.

/// Lowers the calling thread's operating-system priority (best effort).
pub(crate) fn lower_service_priority() {
    // SAFETY: the platform function changes only the calling thread's
    // scheduling priority and ignores the result.
    unsafe { platform::lower() }
}

#[cfg(unix)]
mod platform {
    use std::os::raw::c_int;

    const PRIO_PROCESS: c_int = 0;
    const SERVICE_NICE: c_int = 10;

    unsafe extern "C" {
        fn setpriority(which: c_int, who: u32, prio: c_int) -> c_int;
    }

    #[cfg(target_os = "linux")]
    const SCHED_OTHER: c_int = 0;

    #[cfg(target_os = "linux")]
    #[repr(C)]
    struct SchedParam {
        sched_priority: c_int,
    }

    #[cfg(target_os = "linux")]
    unsafe extern "C" {
        fn sched_setscheduler(pid: c_int, policy: c_int, param: *const SchedParam) -> c_int;
    }

    pub(super) unsafe fn lower() {
        // A FIFO cycle thread makes new service threads inherit FIFO. Demote
        // the calling service thread back to SCHED_OTHER before lowering its
        // nice value; a sandbox may reject the change, which leaves the
        // inherited policy in place (D-204).
        #[cfg(target_os = "linux")]
        {
            let param = SchedParam { sched_priority: 0 };
            // SAFETY: pid 0 names the calling thread; `param` is a valid
            // `sched_param` for the duration of the call.
            unsafe {
                let _ = sched_setscheduler(0, SCHED_OTHER, &param);
            }
        }
        // SAFETY: `PRIO_PROCESS` with id 0 selects the calling thread on
        // Linux; a permission failure leaves the inherited priority in place.
        unsafe {
            let _ = setpriority(PRIO_PROCESS, 0, SERVICE_NICE);
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;

    const THREAD_PRIORITY_BELOW_NORMAL: i32 = -1;

    unsafe extern "system" {
        fn GetCurrentThread() -> *mut c_void;
        fn SetThreadPriority(thread: *mut c_void, priority: i32) -> i32;
    }

    pub(super) unsafe fn lower() {
        // SAFETY: the pseudo-handle names the calling thread, and the
        // priority class is a documented constant. Failure is ignored.
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    pub(super) unsafe fn lower() {}
}

#[cfg(all(unix, test))]
mod tests {
    use std::os::raw::c_int;

    unsafe extern "C" {
        fn getpriority(which: c_int, who: u32) -> c_int;
    }

    #[test]
    fn service_threads_run_below_normal_priority() {
        let handle = std::thread::spawn(|| {
            super::lower_service_priority();
            // SAFETY: `PRIO_PROCESS` with id 0 reads the calling thread.
            unsafe { getpriority(0, 0) }
        });
        let nice = handle.join().expect("thread");
        assert_eq!(nice, 10, "the service thread runs at nice +10 (D-204)");
    }
}

#[cfg(all(windows, test))]
mod tests {
    use std::ffi::c_void;

    const THREAD_PRIORITY_BELOW_NORMAL: i32 = -1;

    unsafe extern "system" {
        fn GetCurrentThread() -> *mut c_void;
        fn GetThreadPriority(thread: *mut c_void) -> i32;
    }

    #[test]
    fn service_threads_run_below_normal_priority() {
        let handle = std::thread::spawn(|| {
            super::lower_service_priority();
            // SAFETY: the pseudo-handle names the calling thread.
            unsafe { GetThreadPriority(GetCurrentThread()) }
        });
        let priority = handle.join().expect("thread");
        assert_eq!(
            priority, THREAD_PRIORITY_BELOW_NORMAL,
            "the service thread runs below normal priority (D-204)"
        );
    }
}
