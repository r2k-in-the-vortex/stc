//! The persistent host runtime for generated ST programs.
//!
//! The crate is a Rust `staticlib` exposing the stable C ABI from
//! Runtime-and-Execution §2.3 (D-077). It owns the process image, the latched
//! cycle clock, trap reporting, and the executable cycle loop. Retained-state
//! file I/O is deliberately a stub until M6: the entry points exist and return
//! "retention disabled" (5).
//!
//! M4.5 adds the host-side online-change substrate:
//!
//! - [`got`] owns the per-generation cell table and binds data, function,
//!   descriptor, direct, and import cells;
//! - [`memory`] owns persistent roots, quarantine, and transient module blocks;
//! - [`catalog`] validates descriptor/manifest pairs against host policy;
//! - [`migration`] validates an online-change plan against the active baseline;
//! - [`loader`] maps a native module and copies its descriptor and function
//!   table;
//! - [`host_api`] provides the ABI 2.0 service table;
//! - [`store`] persists the committed manifest baseline and builds downloads;
//! - [`host`] owns the load/prepare/activate/commit/rollback/unload state
//!   machine and the generation-bound root and GOT state.
//!
//! The M4 ABI 1 entry points below remain the executable path until phase 7
//! converts generated code to the module execution model.
//!
//! This is the only workspace crate that permits unsafe code; every unsafe
//! operation is an FFI pointer access confined to this crate.

use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub mod catalog;
pub mod got;
pub mod host;
pub mod host_api;
pub mod loader;
pub mod memory;
pub mod migration;
pub mod store;

#[cfg(test)]
mod test_support;

pub use catalog::{HostPlatform, PROFILE, SUPPORTED_FEATURES, ValidatedPair};
pub use got::Got;
pub use host::{Candidate, CycleOutcome, GenerationPin, Host, LoadMode, LoadOutcome, ManifestDiff};
pub use host_api::{ABI_VERSION, HOST_API, HostApi, TrapRecord};
pub use loader::LoadedModule;
pub use memory::{MemoryLimits, RootRegistry, RootStorage, TransientAllocator};
pub use migration::{PlanDecision, validate_deployable, validate_plan};
pub use store::{
    Deployment, Download, LayoutState, ManifestStore, RuntimeLayout, RuntimeLayoutEntry,
    StoredManifest,
};

/// Runtime ABI version (Runtime-and-Execution §2.2).
pub const STC_RUNTIME_ABI: i32 = 1;

/// Exit status used when a trap is raised.
pub const STC_TRAP_EXIT_CODE: i32 = 70;

/// Trap codes (Runtime-and-Execution §2.3).
const TRAP_OVERFLOW: i32 = 1;
const TRAP_DIV_ZERO: i32 = 2;
const TRAP_RANGE: i32 = 3;
const TRAP_BOUNDS: i32 = 4;
const TRAP_NULL_REF: i32 = 5;
const TRAP_INVALID_UTF8: i32 = 6;
const TRAP_CONVERSION: i32 = 7;
const TRAP_WATCHDOG: i32 = 8;
const TRAP_ASSERT: i32 = 9;
const TRAP_UNIMPLEMENTED: i32 = 10;

struct State {
    image: Vec<u8>,
    inputs: u64,
    outputs: u64,
    markers: u64,
    cycle_now_ns: i64,
    pending_cycle_ns: Option<i64>,
}

static STATE: Mutex<State> = Mutex::new(State {
    image: Vec::new(),
    inputs: 0,
    outputs: 0,
    markers: 0,
    cycle_now_ns: 0,
    pending_cycle_ns: None,
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn trap_name(code: i32) -> &'static str {
    match code {
        TRAP_OVERFLOW => "OVERFLOW",
        TRAP_DIV_ZERO => "DIV_ZERO",
        TRAP_RANGE => "RANGE",
        TRAP_BOUNDS => "BOUNDS",
        TRAP_NULL_REF => "NULL_REF",
        TRAP_INVALID_UTF8 => "INVALID_UTF8",
        TRAP_CONVERSION => "CONVERSION",
        TRAP_WATCHDOG => "WATCHDOG",
        TRAP_ASSERT => "ASSERT",
        TRAP_UNIMPLEMENTED => "UNIMPLEMENTED",
        _ => "UNKNOWN",
    }
}

/// Reports a fatal runtime trap and exits.
///
/// M4 prints the trap name and span ID only; M6 formats a full source location
/// from the metadata span table.
#[unsafe(no_mangle)]
pub extern "C" fn stc_rt_trap(code: i32, span_id: u32) {
    eprintln!(
        "stc: runtime trap: {:?} (code {code}) at span {span_id}",
        trap_name(code)
    );
    std::process::exit(STC_TRAP_EXIT_CODE);
}

/// Returns the runtime ABI version.
#[unsafe(no_mangle)]
pub extern "C" fn stc_abi_version() -> i32 {
    STC_RUNTIME_ABI
}

/// Runs `init`, `cycles` iterations of `cycle`, then `shutdown`.
///
/// Recognizes `--cycles N`. The scheduling-related options `--cycle-time`,
/// `--io`, and `--trace` are rejected explicitly (exit 2) rather than silently
/// ignored.
#[unsafe(no_mangle)]
pub extern "C" fn stc_rt_execute(
    argc: i32,
    argv: *const *const c_char,
    init: Option<extern "C" fn()>,
    cycle: Option<extern "C" fn()>,
    shutdown: Option<extern "C" fn()>,
) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute(argc, argv, init, cycle, shutdown)
    }));
    match result {
        Ok(code) => code,
        Err(_) => {
            eprintln!("stc: internal runtime panic");
            3
        }
    }
}

fn execute(
    argc: i32,
    argv: *const *const c_char,
    init: Option<extern "C" fn()>,
    cycle: Option<extern "C" fn()>,
    shutdown: Option<extern "C" fn()>,
) -> i32 {
    let args = collect_args(argc, argv);
    let mut cycles: i64 = 1;
    let mut index = usize::from(!args.is_empty());
    while index < args.len() {
        match args[index].as_str() {
            "--cycles" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("stc: `--cycles` requires a value");
                    return 2;
                };
                match value.parse::<i64>() {
                    Ok(value) if value >= 0 => cycles = value,
                    _ => {
                        eprintln!("stc: invalid cycle count `{value}`");
                        return 2;
                    }
                }
                index += 1;
            }
            "--cycle-time" | "--io" | "--trace" => {
                eprintln!(
                    "stc: runtime option `{}` is not supported in this build",
                    args[index]
                );
                return 2;
            }
            other => {
                eprintln!("stc: unknown runtime option `{other}`");
                return 2;
            }
        }
        index += 1;
    }

    if let Some(init) = init {
        init();
    }
    for _ in 0..cycles {
        {
            let mut state = state();
            let now = state
                .pending_cycle_ns
                .take()
                .unwrap_or_else(monotonic_now_ns);
            state.cycle_now_ns = now;
        }
        if let Some(cycle) = cycle {
            cycle();
        }
    }
    if let Some(shutdown) = shutdown {
        shutdown();
    }
    0
}

fn collect_args(argc: i32, argv: *const *const c_char) -> Vec<String> {
    if argv.is_null() || argc <= 0 {
        return Vec::new();
    }
    let mut args = Vec::new();
    for index in 0..argc as usize {
        // SAFETY: the C caller guarantees `argv[0..argc]` are valid,
        // NUL-terminated strings for the duration of the call.
        let pointer = unsafe { *argv.add(index) };
        if pointer.is_null() {
            break;
        }
        let text = unsafe { CStr::from_ptr(pointer) };
        args.push(text.to_string_lossy().into_owned());
    }
    args
}

/// Returns the cycle time latched at the start of the current cycle.
#[unsafe(no_mangle)]
pub extern "C" fn stc_rt_cycle_now_ns() -> i64 {
    state().cycle_now_ns
}

/// Returns the live monotonic clock in nanoseconds.
#[unsafe(no_mangle)]
pub extern "C" fn stc_rt_monotonic_now_ns() -> u64 {
    monotonic_now_ns() as u64
}

fn monotonic_now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX)
        })
}

/// Overrides the clock value used by the next cycle.
#[unsafe(no_mangle)]
pub extern "C" fn stc_rt_set_cycle_time_ns(ns: i64) {
    state().pending_cycle_ns = Some(ns);
}

/// Returns the local date/time in nanoseconds (M4: same as the latched cycle).
#[unsafe(no_mangle)]
pub extern "C" fn stc_rt_local_date_ns() -> i64 {
    state().cycle_now_ns
}

/// Allocates the process image with the given area sizes (D-084).
#[unsafe(no_mangle)]
pub extern "C" fn stc_rt_io_configure(input_len: u64, output_len: u64, marker_len: u64) {
    let total = input_len
        .checked_add(output_len)
        .and_then(|value| value.checked_add(marker_len))
        .unwrap_or(0);
    let mut state = state();
    state.inputs = input_len;
    state.outputs = output_len;
    state.markers = marker_len;
    state.image = vec![0; usize::try_from(total).unwrap_or(0)];
}

/// Returns the process-image base pointer and total length.
///
/// # Safety
///
/// `base` and `len` must be valid, writable pointers to a `uint8_t*` and a
/// `uint64_t` respectively.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_rt_io_image(base: *mut *mut u8, len: *mut u64) {
    let mut state = state();
    if !base.is_null() {
        // SAFETY: the caller contract above guarantees writable pointers.
        unsafe { *base = state.image.as_mut_ptr() };
    }
    if !len.is_null() {
        let total = state.image.len() as u64;
        // SAFETY: the caller contract above guarantees writable pointers.
        unsafe { *len = total };
    }
}

/// Retained-state load stub (5 = retention disabled until M6).
#[unsafe(no_mangle)]
pub extern "C" fn stc_rt_state_load(_buffer: *mut u8, _len: i64, _layout_hash: i64) -> i32 {
    5
}

/// Retained-state save stub (5 = retention disabled until M6).
#[unsafe(no_mangle)]
pub extern "C" fn stc_rt_state_save(_buffer: *const u8, _len: i64, _layout_hash: i64) -> i32 {
    5
}

/// Retained-state configuration stub.
#[unsafe(no_mangle)]
pub extern "C" fn stc_rt_state_configure(_path: *const c_char, _policy: i32, _enabled: i32) {}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicI64, Ordering};

    static SEEN_CYCLE_TIME: AtomicI64 = AtomicI64::new(0);

    extern "C" fn record_cycle_time() {
        SEEN_CYCLE_TIME.store(stc_rt_cycle_now_ns(), Ordering::SeqCst);
    }

    #[test]
    fn image_is_contiguous_and_exposed() {
        stc_rt_io_configure(4, 2, 3);
        let mut base: *mut u8 = std::ptr::null_mut();
        let mut len: u64 = 0;
        // SAFETY: the test passes valid local pointers.
        unsafe { stc_rt_io_image(&mut base, &mut len) };
        assert_eq!(len, 9);
        assert!(!base.is_null());
        // SAFETY: the image is 9 bytes and owned by the runtime.
        unsafe { *base.add(8) = 7 };
        let state = state();
        assert_eq!(state.image[8], 7);
        assert_eq!(state.inputs, 4);
        assert_eq!(state.outputs, 2);
        assert_eq!(state.markers, 3);
    }

    #[test]
    fn cycle_time_is_latched() {
        stc_rt_set_cycle_time_ns(1234);
        SEEN_CYCLE_TIME.store(0, Ordering::SeqCst);
        stc_rt_execute(1, std::ptr::null(), None, Some(record_cycle_time), None);
        assert_eq!(SEEN_CYCLE_TIME.load(Ordering::SeqCst), 1234);
    }
}
