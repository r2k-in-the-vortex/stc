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
//!   machine and the generation-bound root and GOT state;
//! - [`module_info`] reads the process-image and direct-binding metadata
//!   sidecar;
//! - [`image`] validates image layouts and applies direct-variable remaps at
//!   activation, preserving values by `GotId`;
//! - `trap_boundary` provides the non-local trap exit around module calls.
//!
//! Generated ABI 2 modules execute through [`stc_rt_execute`] (the thin
//! launcher path) or the `stc_host_*` embedding exports. The legacy ABI 1
//! helper symbols remain as host utilities for the process image, clock, and
//! retained-state stubs; generated modules do not import them.
//!
//! The runtime is the only workspace crate that permits unsafe code; every
//! unsafe operation is an FFI pointer access confined to this crate.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub mod catalog;
pub mod got;
pub mod host;
pub mod host_api;
pub mod image;
pub mod loader;
pub mod memory;
pub mod migration;
pub mod module_info;
pub mod store;
mod trap_boundary;

#[cfg(test)]
mod test_support;

pub use catalog::{HostPlatform, PROFILE, SUPPORTED_FEATURES, ValidatedPair};
pub use got::Got;
pub use host::{
    Candidate, CycleOutcome, GenerationPin, Host, HostWarning, LoadMode, LoadOutcome, ManifestDiff,
};
pub use host_api::{ABI_VERSION, HOST_API, HostApi, TrapRecord};
pub use image::{DirectRemap, MAX_IMAGE_BYTES};
pub use loader::LoadedModule;
pub use memory::{MemoryLimits, RootRegistry, RootStorage, TransientAllocator};
pub use migration::{PlanDecision, validate_deployable, validate_plan};
pub use module_info::{DirectBinding, ImageLayout, ModuleInfo};
pub use store::{
    Deployment, Download, LayoutState, ManifestStore, RuntimeLayout, RuntimeLayoutEntry,
    StoredManifest,
};

/// Runtime ABI version (Runtime-and-Execution §2.2).
pub const STC_RUNTIME_ABI: i32 = 2;

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

/// Runs the persistent host loop for a loadable module.
///
/// `module_path` names the module file; when it has no directory component the
/// launcher's directory is used. The adjacent `<name>.stc-got.bin` manifest and
/// `<name>.stc-meta.json` metadata are loaded automatically.
///
/// Recognizes `--cycles N`. The scheduling-related options `--cycle-time`,
/// `--io`, and `--trace` are rejected explicitly (exit 2) rather than silently
/// ignored.
///
/// # Safety
///
/// `argv` must point to `argc` valid, NUL-terminated strings. `module_path`
/// must be null or point to a NUL-terminated string. Both stay valid for the
/// duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_rt_execute(
    argc: i32,
    argv: *const *const c_char,
    module_path: *const c_char,
) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute_module(argc, argv, module_path)
    }));
    match result {
        Ok(code) => code,
        Err(_) => {
            eprintln!("stc: internal runtime panic");
            3
        }
    }
}

fn execute_module(argc: i32, argv: *const *const c_char, module_path: *const c_char) -> i32 {
    let args = collect_args(argc, argv);
    let mut cycles: i64 = 1;
    let mut module_argument: Option<String> = None;
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
            "--module" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("stc: `--module` requires a value");
                    return 2;
                };
                module_argument = Some(value.clone());
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

    let Some(module_argument) = module_argument.or_else(|| {
        if module_path.is_null() {
            None
        } else {
            // SAFETY: the ABI contract requires a NUL-terminated string.
            Some(
                unsafe { CStr::from_ptr(module_path) }
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    }) else {
        eprintln!("stc: no module path was supplied");
        return 2;
    };
    let resolved = resolve_module_path(&module_argument);
    let platform = host_platform();
    let manifest = module_info::manifest_path(&resolved);
    let mut host = Host::new(platform.clone(), ManifestStore::memory());
    host_api::clear_last_trap();
    let candidate = match Candidate::open(&platform, &resolved, &manifest) {
        Ok(candidate) => candidate,
        Err(error) => {
            eprintln!(
                "stc: cannot load module `{}`: {}",
                resolved.display(),
                error.message
            );
            return 1;
        }
    };
    if let Err(error) = host.load(candidate, LoadMode::Cold) {
        eprintln!(
            "stc: cannot install module `{}`: {}",
            resolved.display(),
            error.message
        );
        return if host_api::last_trap().is_some() {
            print_last_trap();
            STC_TRAP_EXIT_CODE
        } else {
            1
        };
    }
    for _ in 0..cycles {
        latch_cycle_clock();
        if let Err(error) = host.cycle() {
            if host_api::last_trap().is_some() {
                print_last_trap();
                let _ = host.shutdown();
                return STC_TRAP_EXIT_CODE;
            }
            eprintln!("stc: cycle failed: {}", error.message);
            let _ = host.shutdown();
            return 1;
        }
    }
    if let Err(error) = host.shutdown() {
        eprintln!("stc: shutdown failed: {}", error.message);
        return 1;
    }
    0
}

fn print_last_trap() {
    if let Some(trap) = host_api::last_trap() {
        eprintln!(
            "stc: runtime trap: {:?} (code {}) at span {}",
            trap_name(trap.code),
            trap.code,
            trap.span_id
        );
    }
}

fn resolve_module_path(argument: &str) -> PathBuf {
    let path = Path::new(argument);
    if path.is_absolute() || path.components().count() > 1 {
        return path.to_path_buf();
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|directory| directory.join(path)))
        .unwrap_or_else(|| path.to_path_buf())
}

/// Returns the host policy for this process.
#[must_use]
pub fn host_platform() -> HostPlatform {
    let triple = if cfg!(all(windows, target_env = "msvc")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    HostPlatform::new(
        PROFILE,
        stc_got::Target::new(triple, stc_got::Endianness::Little, 8),
        stc_got::HOST_ABI_MAJOR,
        stc_got::HOST_ABI_MINOR,
    )
}

/// Captures the process image for rollback of a failed cold install.
pub(crate) fn image_snapshot() -> (Vec<u8>, u64, u64, u64) {
    let state = state();
    (
        state.image.clone(),
        state.inputs,
        state.outputs,
        state.markers,
    )
}

/// Restores a process-image snapshot.
pub(crate) fn restore_image(snapshot: (Vec<u8>, u64, u64, u64)) {
    let mut state = state();
    state.image = snapshot.0;
    state.inputs = snapshot.1;
    state.outputs = snapshot.2;
    state.markers = snapshot.3;
}

/// Runs `f` with mutable access to the configured process image.
///
/// The host calls this on the activation path, after the old generation has
/// completed its cycle and before the new generation starts.
pub(crate) fn image_with<R>(f: impl FnOnce(&mut [u8]) -> R) -> R {
    let mut state = state();
    f(&mut state.image)
}

/// Returns the process-image base address without exposing the buffer.
///
/// Planned direct-cell values use this base before activation; the values are
/// rebound once the image is configured for the candidate layout.
pub(crate) fn image_base_address() -> usize {
    let mut state = state();
    state.image.as_mut_ptr() as usize
}

/// Returns the byte address of a process-image range.
///
/// Areas are `1` inputs, `2` outputs, and `3` markers.
pub(crate) fn image_area_address(area: u8, byte_offset: u64, bytes: u64) -> Option<usize> {
    let mut state = state();
    let (offset, length) = match area {
        1 => (0, state.inputs),
        2 => (state.inputs, state.outputs),
        3 => (state.inputs.saturating_add(state.outputs), state.markers),
        _ => return None,
    };
    if bytes == 0 || byte_offset.saturating_add(bytes) > length {
        return None;
    }
    let offset = usize::try_from(offset).ok()?;
    let byte_offset = usize::try_from(byte_offset).ok()?;
    let total = offset.checked_add(byte_offset)?;
    // SAFETY: the range was checked against the area length and the process
    // image is not resized while a module generation uses it.
    Some(unsafe { state.image.as_mut_ptr().add(total) } as usize)
}

/// Latches the cycle clock for the next scan.
pub(crate) fn latch_cycle_clock() {
    let mut state = state();
    let now = state
        .pending_cycle_ns
        .take()
        .unwrap_or_else(monotonic_now_ns);
    state.cycle_now_ns = now;
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
    let total = usize::try_from(total).unwrap_or(0);
    let mut state = state();
    state.inputs = input_len;
    state.outputs = output_len;
    state.markers = marker_len;
    // Reuse the allocation when it already has capacity. Direct cells point
    // into this buffer, so moving it invalidates the active generation's
    // address bindings until they are recomputed.
    if total <= state.image.capacity() {
        state.image.clear();
        state.image.resize(total, 0);
    } else {
        state.image = vec![0; total];
    }
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

/// Returns the configured process-image length in bytes.
///
/// This is the safe Rust accessor used by embedding hosts and tests; generated
/// code reaches the image through the host I/O service.
#[must_use]
pub fn image_length() -> u64 {
    state().image.len() as u64
}

/// Returns the process-image base address, or `0` when no image is configured.
///
/// The address is only meaningful while the image layout is unchanged; a
/// reconfiguration may move the buffer.
#[must_use]
pub fn image_base() -> usize {
    let mut state = state();
    if state.image.is_empty() {
        0
    } else {
        state.image.as_mut_ptr() as usize
    }
}

/// Copies up to `buffer.len()` process-image bytes starting at `offset`.
///
/// Returns the number of bytes copied. A range outside the image copies
/// nothing.
pub fn image_read(offset: usize, buffer: &mut [u8]) -> usize {
    let state = state();
    let Some(slice) = state.image.get(offset..) else {
        return 0;
    };
    let count = slice.len().min(buffer.len());
    buffer[..count].copy_from_slice(&slice[..count]);
    count
}

/// Writes `bytes` into the process image at `offset`.
///
/// Returns `false` when the range lies outside the configured image.
pub fn image_write(offset: usize, bytes: &[u8]) -> bool {
    let mut state = state();
    let Some(slice) = state.image.get_mut(offset..) else {
        return false;
    };
    if bytes.len() > slice.len() {
        return false;
    }
    slice[..bytes.len()].copy_from_slice(bytes);
    true
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

/// Load mode values for [`stc_host_load_module`].
pub mod load_mode {
    /// Stop and reinitialize the program for this module.
    pub const COLD: i32 = 0;
    /// Prepare an online change; the old generation stays active until the
    /// host activates the candidate.
    pub const ONLINE: i32 = 1;
    /// Install cold when the host is empty, otherwise prepare online.
    pub const AUTO: i32 = 2;
}

struct Embedded {
    host: Option<Host>,
    metadata: Option<CString>,
}

static EMBEDDED: Mutex<Embedded> = Mutex::new(Embedded {
    host: None,
    metadata: None,
});

fn embedded() -> std::sync::MutexGuard<'static, Embedded> {
    EMBEDDED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Loads a module and its adjacent manifest into the embedding host.
///
/// Returns `0` when the candidate is installed or prepared, `1` for an invalid
/// argument or load failure, and `2` when an online request requires a cold
/// download.
///
/// # Safety
///
/// `module_path` must be null or point to a NUL-terminated string that stays
/// valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_host_load_module(module_path: *const c_char, mode: i32) -> i32 {
    if module_path.is_null() {
        return 1;
    }
    // SAFETY: the ABI contract requires a NUL-terminated path.
    let argument = unsafe { CStr::from_ptr(module_path) }
        .to_string_lossy()
        .into_owned();
    let resolved = resolve_module_path(&argument);
    let platform = host_platform();
    let manifest = module_info::manifest_path(&resolved);
    let candidate = match Candidate::open(&platform, &resolved, &manifest) {
        Ok(candidate) => candidate,
        Err(error) => {
            eprintln!(
                "stc: cannot load module `{}`: {}",
                resolved.display(),
                error.message
            );
            return 1;
        }
    };
    let metadata_text = candidate.info().map(|info| info.text.clone());
    let mode = match mode {
        load_mode::COLD => LoadMode::Cold,
        load_mode::ONLINE => LoadMode::Online,
        load_mode::AUTO => LoadMode::Auto,
        _ => return 1,
    };
    let mut state = embedded();
    if state.host.is_none() {
        state.host = Some(Host::new(platform, ManifestStore::memory()));
    }
    let host = state.host.as_mut().expect("host created above");
    host_api::clear_last_trap();
    match host.load(candidate, mode) {
        Ok(LoadOutcome::ColdRequired { .. }) => 2,
        Ok(_) => {
            state.metadata = metadata_text.and_then(|text| CString::new(text).ok());
            0
        }
        Err(error) => {
            eprintln!("stc: cannot install module: {}", error.message);
            1
        }
    }
}

/// Runs one cycle of the active embedding-host generation.
///
/// Returns `0` on success, `1` on a cycle failure or trap, and `2` when no
/// module is loaded.
#[unsafe(no_mangle)]
pub extern "C" fn stc_host_cycle() -> i32 {
    latch_cycle_clock();
    let mut state = embedded();
    let Some(host) = state.host.as_mut() else {
        return 2;
    };
    match host.cycle() {
        Ok(_) => 0,
        Err(error) => {
            eprintln!("stc: cycle failed: {}", error.message);
            1
        }
    }
}

/// Shuts down and unloads the embedding-host module.
#[unsafe(no_mangle)]
pub extern "C" fn stc_host_shutdown() -> i32 {
    let mut state = embedded();
    state.metadata = None;
    let Some(mut host) = state.host.take() else {
        return 0;
    };
    match host.shutdown() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("stc: shutdown failed: {}", error.message);
            1
        }
    }
}

/// Resolves an exported symbol in the active embedding-host module.
///
/// Returns a null pointer when the host has no active module or the module does
/// not export the name. The address stays valid until the generation retires.
///
/// # Safety
///
/// `name` must be null or point to a NUL-terminated string that stays valid
/// for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_host_symbol(name: *const c_char) -> *mut c_void {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: the ABI contract requires a NUL-terminated symbol name.
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    let state = embedded();
    state
        .host
        .as_ref()
        .and_then(|host| host.export_symbol(&name))
        .map_or(std::ptr::null_mut(), |address| address.cast_mut())
}

/// Copies the committed manifest JSON into a caller buffer.
///
/// With a null buffer, writes the required length (including the terminating
/// NUL) and returns `0`. With a buffer shorter than required, writes the
/// required length and returns `1`.
///
/// # Safety
///
/// `len` must point to a writable `i64`. When `buffer` is non-null it must
/// point to at least the capacity reported in `*len` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_host_export_manifest(buffer: *mut c_char, len: *mut i64) -> i32 {
    if len.is_null() {
        return 1;
    }
    let state = embedded();
    let Some(host) = state.host.as_ref() else {
        return 1;
    };
    let Some(json) = host.export_manifest_json() else {
        return 1;
    };
    let required = i64::try_from(json.len())
        .unwrap_or(i64::MAX)
        .saturating_add(1);
    // SAFETY: the caller supplies a readable capacity through `*len`.
    let capacity = if buffer.is_null() { 0 } else { unsafe { *len } };
    // SAFETY: the caller supplies a writable `i64` pointer.
    unsafe { *len = required };
    if buffer.is_null() {
        return 0;
    }
    if capacity < required {
        return 1;
    }
    let required_usize = usize::try_from(required).unwrap_or(usize::MAX);
    // SAFETY: the caller's buffer has at least `required` bytes; the source
    // has `required - 1` bytes plus the terminating NUL written below.
    unsafe {
        std::ptr::copy_nonoverlapping(json.as_ptr().cast::<c_char>(), buffer, required_usize - 1);
        *buffer.add(required_usize - 1) = 0;
    }
    0
}

/// Returns the loaded module's metadata JSON, or null before a load.
#[unsafe(no_mangle)]
pub extern "C" fn stc_metadata_json() -> *const c_char {
    let state = embedded();
    state
        .metadata
        .as_ref()
        .map_or(std::ptr::null(), |text| text.as_ptr())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        latch_cycle_clock();
        assert_eq!(stc_rt_cycle_now_ns(), 1234);
        stc_rt_set_cycle_time_ns(4321);
        assert_eq!(stc_rt_cycle_now_ns(), 1234);
        latch_cycle_clock();
        assert_eq!(stc_rt_cycle_now_ns(), 4321);
    }
}
