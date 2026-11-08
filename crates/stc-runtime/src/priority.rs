//! Operating-system priority for host service threads (D-204) and the RT
//! metrics writer.
//!
//! The cycle thread runs at the process default priority. Control-server and
//! debug-server accept/handler threads are lowered one class so maintenance
//! and monitoring work cannot compete with the scan. The RT metrics writer
//! uses a checked lowering so an opt-in measurement fails closed instead of
//! inheriting the cycle thread's scheduling class. The policy is advisory:
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

/// Lowers the calling thread to `SCHED_OTHER` with the service nice level.
///
/// Unlike [`lower_service_priority`], this reports a syscall failure so a
/// measurement writer can fail closed instead of inheriting a real-time
/// scheduling class.
///
/// # Errors
///
/// Returns a message when the scheduler policy or the nice level cannot be
/// changed.
#[cfg(target_os = "linux")]
pub(crate) fn lower_service_priority_checked() -> Result<(), String> {
    // SAFETY: the platform function changes only the calling thread's
    // scheduling priority.
    unsafe { platform::lower_checked() }
}

/// Lowers the calling RT metrics writer thread to the host writer class.
///
/// Windows uses `THREAD_PRIORITY_BELOW_NORMAL`; macOS sets the standard
/// thread policy and nice +10. Unlike [`lower_service_priority`], this reports
/// a failure so an opt-in measurement can fail closed.
///
/// # Errors
///
/// Returns a message when the priority or policy cannot be changed.
#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn lower_writer_priority_checked() -> Result<(), String> {
    // SAFETY: the platform function changes only the calling thread's
    // scheduling priority.
    unsafe { platform::lower_writer_checked() }
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

    #[cfg(target_os = "macos")]
    const THREAD_STANDARD_POLICY: u32 = 1;
    #[cfg(target_os = "macos")]
    const THREAD_STANDARD_POLICY_COUNT: u32 = 0;

    #[cfg(target_os = "macos")]
    unsafe extern "C" {
        fn pthread_self() -> usize;
        fn pthread_mach_thread_np(thread: usize) -> u32;
        fn thread_policy_set(
            thread: u32,
            policy: u32,
            policy_info: *const i32,
            count: u32,
        ) -> c_int;
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

    #[cfg(target_os = "linux")]
    pub(super) unsafe fn lower_checked() -> Result<(), String> {
        use std::io;
        let param = SchedParam { sched_priority: 0 };
        // SAFETY: pid 0 names the calling thread; `param` is a valid
        // `sched_param` for the duration of the call.
        let result = unsafe { sched_setscheduler(0, SCHED_OTHER, &param) };
        if result != 0 {
            return Err(format!(
                "cannot set the metrics writer thread to SCHED_OTHER: {}",
                io::Error::last_os_error()
            ));
        }
        // SAFETY: `PRIO_PROCESS` with id 0 selects the calling thread on
        // Linux; lowering the priority is permitted without privilege.
        let result = unsafe { setpriority(PRIO_PROCESS, 0, SERVICE_NICE) };
        if result != 0 {
            return Err(format!(
                "cannot lower the metrics writer thread priority: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub(super) unsafe fn lower_writer_checked() -> Result<(), String> {
        use std::io;
        // SAFETY: `pthread_self` returns the calling thread.
        let thread = unsafe { pthread_mach_thread_np(pthread_self()) };
        // SAFETY: the standard policy has no policy data and count zero.
        let result = unsafe {
            thread_policy_set(
                thread,
                THREAD_STANDARD_POLICY,
                std::ptr::null(),
                THREAD_STANDARD_POLICY_COUNT,
            )
        };
        if result != 0 {
            return Err(format!(
                "cannot set the metrics writer thread to the standard policy: \
                 kern_return {result}"
            ));
        }
        // SAFETY: `PRIO_PROCESS` with id 0 selects the calling process;
        // lowering the priority is permitted without privilege.
        let result = unsafe { setpriority(PRIO_PROCESS, 0, SERVICE_NICE) };
        if result != 0 {
            return Err(format!(
                "cannot lower the metrics writer thread priority: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(())
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

    pub(super) unsafe fn lower_writer_checked() -> Result<(), String> {
        use std::io;
        // SAFETY: the pseudo-handle names the calling thread, and the
        // priority class is a documented constant.
        let result = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) };
        if result == 0 {
            return Err(format!(
                "cannot lower the metrics writer thread priority: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(())
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

    #[cfg(target_os = "macos")]
    #[test]
    fn rt_writer_thread_runs_at_the_service_nice_level() {
        let handle = std::thread::spawn(|| {
            super::lower_writer_priority_checked().expect("lower the RT writer");
            // SAFETY: `PRIO_PROCESS` with id 0 reads the calling thread.
            unsafe { getpriority(0, 0) }
        });
        let nice = handle.join().expect("thread");
        assert_eq!(nice, 10, "the RT writer runs at nice +10");
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

    #[test]
    fn rt_writer_thread_runs_below_normal_priority() {
        let handle = std::thread::spawn(|| {
            super::lower_writer_priority_checked().expect("lower the RT writer");
            // SAFETY: the pseudo-handle names the calling thread.
            unsafe { GetThreadPriority(GetCurrentThread()) }
        });
        let priority = handle.join().expect("thread");
        assert_eq!(
            priority, THREAD_PRIORITY_BELOW_NORMAL,
            "the RT writer runs below normal priority"
        );
    }
}
