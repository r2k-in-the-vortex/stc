//! Host ABI 2.3 service table.
//!
//! GOT Artifacts §8 fixes the table layout: version and size followed by trap,
//! clocks, transient allocation, process-image access, retained-state stubs,
//! assert, the test-clock service, and the debug checkpoint hook. Import cells
//! hold the service entry point itself, so generated code reads and calls a
//! service exactly like a function cell.
//!
//! Minor 1 adds no service. It records the inline TYPEINFO, VTABLE, and
//! object-record layout of §8.5 that generated code reads directly; the table
//! shape is unchanged, so a 2.0 host rejects a 2.1 module by minor version
//! before the layout can be used. Minor 2 appends the test-clock service
//! (ID 12); only test builds import it. Minor 3 appends the debug checkpoint
//! hook (ID 13); debug builds with at least one checkpoint import it.
//!
//! The table is a read-only static. Service implementations route transient
//! allocation and free to the generation whose entry point is currently
//! executing; the host sets that generation around attach, prepare, cycle,
//! shutdown, and detach.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::sync::Mutex;

use stc_got::error::{Error, Result, codes};

use crate::memory::TransientAllocator;

/// Host ABI version word: `(major << 16) | minor` (GOT Artifacts §8.1).
pub const ABI_VERSION: u32 = (2 << 16) | 3;

/// Service IDs used by import slots (GOT Artifacts §8.2).
pub mod service {
    /// Non-returning trap service.
    pub const TRAP: u8 = 1;
    /// Cycle-latched clock.
    pub const CYCLE_NOW_NS: u8 = 2;
    /// Live monotonic clock.
    pub const MONOTONIC_NOW_NS: u8 = 3;
    /// Local date/time latch.
    pub const LOCAL_DATE_NS: u8 = 4;
    /// Transient allocation.
    pub const ALLOC: u8 = 5;
    /// Transient free.
    pub const FREE: u8 = 6;
    /// Process-image area base.
    pub const IO_IMAGE: u8 = 7;
    /// Process-image area length.
    pub const IO_SIZE: u8 = 8;
    /// Retained-state load.
    pub const STATE_LOAD: u8 = 9;
    /// Retained-state save.
    pub const STATE_SAVE: u8 = 10;
    /// Assertion service.
    pub const ASSERT: u8 = 11;
    /// Next-cycle clock override (test builds).
    pub const SET_CYCLE_TIME_NS: u8 = 12;
    /// Debug checkpoint hook (debug builds with checkpoints).
    pub const DEBUG_CHECK: u8 = 13;
}

/// Trap function signature.
pub type TrapFn = unsafe extern "C" fn(i32, u32);
/// Latched-clock signature.
pub type CycleNowFn = unsafe extern "C" fn() -> i64;
/// Monotonic-clock signature.
pub type MonotonicNowFn = unsafe extern "C" fn() -> u64;
/// Local-date signature.
pub type LocalDateFn = unsafe extern "C" fn() -> i64;
/// Transient allocation signature.
pub type AllocFn = unsafe extern "C" fn(u64, u64) -> *mut c_void;
/// Transient free signature.
pub type FreeFn = unsafe extern "C" fn(*mut c_void);
/// Process-image base signature.
pub type IoImageFn = unsafe extern "C" fn(u32) -> *mut u8;
/// Process-image size signature.
pub type IoSizeFn = unsafe extern "C" fn(u32) -> u64;
/// Retained-state load signature.
pub type StateLoadFn = unsafe extern "C" fn(*mut u8, i64, i64) -> i32;
/// Retained-state save signature.
pub type StateSaveFn = unsafe extern "C" fn(*const u8, i64, i64) -> i32;
/// Assert signature.
pub type AssertFn = unsafe extern "C" fn(i32, i32, u32);
/// Next-cycle clock override signature.
pub type SetCycleTimeFn = unsafe extern "C" fn(i64);
/// Debug checkpoint hook signature.
pub type DebugCheckFn = unsafe extern "C" fn(u32, u32);

/// The host API 2.3 table (GOT Artifacts §8.1).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostApi {
    /// `(major << 16) | minor`; 2.3 is `0x0002_0003`.
    pub abi_version: u32,
    /// Size of this table in bytes.
    pub struct_size: u32,
    /// Non-returning trap service.
    pub trap: Option<TrapFn>,
    /// Cycle-latched clock in nanoseconds.
    pub cycle_now_ns: Option<CycleNowFn>,
    /// Live monotonic clock in nanoseconds.
    pub monotonic_now_ns: Option<MonotonicNowFn>,
    /// Local date/time in nanoseconds.
    pub local_date_ns: Option<LocalDateFn>,
    /// Transient zeroed allocation.
    pub alloc: Option<AllocFn>,
    /// Transient free.
    pub free: Option<FreeFn>,
    /// Process-image area base (1 inputs, 2 outputs, 3 markers).
    pub io_image: Option<IoImageFn>,
    /// Process-image area length.
    pub io_size: Option<IoSizeFn>,
    /// Retained-state load stub.
    pub state_load: Option<StateLoadFn>,
    /// Retained-state save stub.
    pub state_save: Option<StateSaveFn>,
    /// Assertion trap.
    pub assert: Option<AssertFn>,
    /// Sets the next cycle's latched time (host ABI 2.2).
    pub set_cycle_time_ns: Option<SetCycleTimeFn>,
    /// Debug checkpoint hook (host ABI 2.3).
    pub debug_check: Option<DebugCheckFn>,
}

/// The process-wide host API table passed to every attached module.
pub static HOST_API: HostApi = HostApi {
    abi_version: ABI_VERSION,
    struct_size: std::mem::size_of::<HostApi>() as u32,
    trap: Some(service_trap),
    cycle_now_ns: Some(service_cycle_now_ns),
    monotonic_now_ns: Some(service_monotonic_now_ns),
    local_date_ns: Some(service_local_date_ns),
    alloc: Some(service_alloc),
    free: Some(service_free),
    io_image: Some(service_io_image),
    io_size: Some(service_io_size),
    state_load: Some(service_state_load),
    state_save: Some(service_state_save),
    assert: Some(service_assert),
    set_cycle_time_ns: Some(service_set_cycle_time_ns),
    debug_check: Some(crate::stop::debug_check_hook),
};

/// Returns the table pointer passed to `stc_module_attach`.
#[must_use]
pub fn api_pointer() -> *const HostApi {
    std::ptr::addr_of!(HOST_API)
}

/// Returns the entry-point address of one host API service for an import cell.
///
/// The returned value is the code address the module calls after loading its
/// import cell value.
#[must_use]
pub fn service_address(service: u8) -> Option<usize> {
    match service {
        service::TRAP => HOST_API.trap.map(|function| function as usize),
        service::CYCLE_NOW_NS => HOST_API.cycle_now_ns.map(|function| function as usize),
        service::MONOTONIC_NOW_NS => HOST_API.monotonic_now_ns.map(|function| function as usize),
        service::LOCAL_DATE_NS => HOST_API.local_date_ns.map(|function| function as usize),
        service::ALLOC => HOST_API.alloc.map(|function| function as usize),
        service::FREE => HOST_API.free.map(|function| function as usize),
        service::IO_IMAGE => HOST_API.io_image.map(|function| function as usize),
        service::IO_SIZE => HOST_API.io_size.map(|function| function as usize),
        service::STATE_LOAD => HOST_API.state_load.map(|function| function as usize),
        service::STATE_SAVE => HOST_API.state_save.map(|function| function as usize),
        service::ASSERT => HOST_API.assert.map(|function| function as usize),
        service::SET_CYCLE_TIME_NS => HOST_API.set_cycle_time_ns.map(|function| function as usize),
        service::DEBUG_CHECK => HOST_API.debug_check.map(|function| function as usize),
        _ => None,
    }
}

/// A recorded trap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrapRecord {
    /// Trap code from Runtime-and-Execution §2.3.
    pub code: i32,
    /// Span ID supplied by generated code.
    pub span_id: u32,
    /// Generation executing when the trap was raised.
    pub generation: u64,
}

struct ServiceState {
    allocators: BTreeMap<u64, TransientAllocator>,
    faults: Vec<(u64, String)>,
    trap: Option<TrapRecord>,
    record_assertions: bool,
    assertions: Vec<AssertRecord>,
}

impl ServiceState {
    const fn new() -> Self {
        Self {
            allocators: BTreeMap::new(),
            faults: Vec::new(),
            trap: None,
            record_assertions: false,
            assertions: Vec::new(),
        }
    }
}

static SERVICE_STATE: Mutex<ServiceState> = Mutex::new(ServiceState::new());

fn service_state() -> std::sync::MutexGuard<'static, ServiceState> {
    SERVICE_STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

thread_local! {
    static CURRENT_GENERATION: Cell<u64> = const { Cell::new(0) };
}

/// Sets the generation whose entry point is running and restores the previous
/// value on drop.
pub(crate) struct GenerationGuard {
    previous: u64,
}

impl Drop for GenerationGuard {
    fn drop(&mut self) {
        CURRENT_GENERATION.with(|current| current.set(self.previous));
    }
}

#[must_use]
pub(crate) fn current_generation() -> u64 {
    CURRENT_GENERATION.with(Cell::get)
}

pub(crate) fn enter_generation(generation: u64) -> GenerationGuard {
    let previous = CURRENT_GENERATION.with(|current| current.replace(generation));
    GenerationGuard { previous }
}

/// Registers the transient allocator for a new generation.
///
/// # Errors
///
/// Rejects a zero or duplicate generation ID.
pub(crate) fn register_generation(generation: u64) -> Result<()> {
    if generation == 0 {
        return Err(Error::new(
            codes::ONLINE_PREPARE,
            "generation IDs are nonzero",
        ));
    }
    let mut state = service_state();
    if state.allocators.contains_key(&generation) {
        return Err(Error::new(
            codes::ONLINE_PREPARE,
            format!("generation {generation} is already registered"),
        ));
    }
    state
        .allocators
        .insert(generation, TransientAllocator::new(generation));
    Ok(())
}

/// Withdraws a generation's transient allocator and returns the bytes freed.
pub(crate) fn withdraw_generation(generation: u64) -> u64 {
    let mut state = service_state();
    let Some(mut allocator) = state.allocators.remove(&generation) else {
        return 0;
    };
    allocator.free_all()
}

/// Returns and clears module-fault reports for one generation.
pub(crate) fn drain_faults(generation: u64) -> Vec<String> {
    let mut state = service_state();
    let mut faults = Vec::new();
    state.faults.retain(|(owner, message)| {
        if *owner == generation {
            faults.push(message.clone());
            false
        } else {
            true
        }
    });
    faults
}

/// Returns the last recorded trap.
///
/// A trap outside a host boundary exits the process. Host module calls install
/// a boundary, so this records the most recent recoverable trap.
#[must_use]
pub fn last_trap() -> Option<TrapRecord> {
    service_state().trap
}

/// Clears the last recorded trap.
pub(crate) fn clear_last_trap() {
    service_state().trap = None;
}

/// Returns the bytes reserved by one generation's live transient blocks.
///
/// A failed prepare, retire, or unload withdraws the generation, after which
/// this returns `None`.
#[must_use]
pub fn transient_bytes(generation: u64) -> Option<u64> {
    service_state()
        .allocators
        .get(&generation)
        .map(TransientAllocator::reserved_bytes)
}

fn record_fault(generation: u64, message: impl Into<String>) {
    service_state().faults.push((generation, message.into()));
}

fn record_trap(code: i32, span_id: u32) {
    let generation = current_generation();
    service_state().trap = Some(TrapRecord {
        code,
        span_id,
        generation,
    });
}

/// A recorded assertion failure raised through the `assert` service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssertRecord {
    /// Assertion trap code (Runtime-and-Execution §2.3).
    pub code: i32,
    /// Span ID supplied by generated code.
    pub span_id: u32,
}

/// Starts recording assertion failures instead of trapping.
///
/// The test host enables this around its run loop so `STC_ASSERT` records
/// failures and execution continues (Testing §2.3). Returns any records left
/// from a previous recording session.
pub fn begin_assert_recording() -> Vec<AssertRecord> {
    let mut state = service_state();
    state.record_assertions = true;
    std::mem::take(&mut state.assertions)
}

/// Stops recording assertion failures and returns the recorded failures.
pub fn end_assert_recording() -> Vec<AssertRecord> {
    let mut state = service_state();
    state.record_assertions = false;
    std::mem::take(&mut state.assertions)
}

/// Records the trap and transfers control to the host trap boundary.
///
/// Inside a host module call this performs a non-local exit to
/// [`crate::trap_boundary::with_boundary`]. With no boundary installed the M4.5
/// behavior remains: print the trap name and exit with code 70.
unsafe extern "C" fn service_trap(code: i32, span_id: u32) {
    record_trap(code, span_id);
    if !crate::trap_boundary::escape(code, span_id) {
        crate::stc_rt_trap(code, span_id);
    }
}

unsafe extern "C" fn service_cycle_now_ns() -> i64 {
    crate::stc_rt_cycle_now_ns()
}

unsafe extern "C" fn service_monotonic_now_ns() -> u64 {
    crate::stc_rt_monotonic_now_ns()
}

unsafe extern "C" fn service_local_date_ns() -> i64 {
    crate::stc_rt_local_date_ns()
}

unsafe extern "C" fn service_alloc(size: u64, align: u64) -> *mut c_void {
    let generation = current_generation();
    if generation == 0 {
        return std::ptr::null_mut();
    }
    let Ok(align) = u32::try_from(align) else {
        return std::ptr::null_mut();
    };
    let mut state = service_state();
    let Some(allocator) = state.allocators.get_mut(&generation) else {
        return std::ptr::null_mut();
    };
    match allocator.alloc(size, align) {
        Ok(address) => address as *mut c_void,
        Err(_) => std::ptr::null_mut(),
    }
}

unsafe extern "C" fn service_free(pointer: *mut c_void) {
    let generation = current_generation();
    if generation == 0 {
        record_fault(0, "free called with no active generation");
        return;
    }
    let mut state = service_state();
    if !state.allocators.contains_key(&generation) {
        state.faults.push((
            generation,
            format!("free called for an unknown generation {generation}"),
        ));
        return;
    }
    let allocator = state
        .allocators
        .get_mut(&generation)
        .expect("checked above");
    if let Err(error) = allocator.free(pointer as usize) {
        state.faults.push((generation, error.message));
    }
}

/// Returns the base and length of one process-image area.
///
/// Area `1` is inputs, `2` outputs, and `3` markers. An empty or unknown area
/// reports a null base and zero length, matching the ABI's "unconfigured"
/// result.
fn image_area(state: &mut crate::State, area: u32) -> (*mut u8, u64) {
    let (offset, length) = match area {
        1 => (0, state.inputs),
        2 => (state.inputs, state.outputs),
        3 => {
            let offset = state.inputs.saturating_add(state.outputs);
            (offset, state.markers)
        }
        _ => return (std::ptr::null_mut(), 0),
    };
    if length == 0 {
        return (std::ptr::null_mut(), 0);
    }
    let (Ok(offset), Ok(length_usize)) = (usize::try_from(offset), usize::try_from(length)) else {
        return (std::ptr::null_mut(), 0);
    };
    if offset.saturating_add(length_usize) > state.image.len() {
        return (std::ptr::null_mut(), 0);
    }
    // SAFETY: the checked range lies inside the process-image allocation, and
    // the image is not resized while a module reads an area.
    let base = unsafe { state.image.as_mut_ptr().add(offset) };
    (base, length)
}

unsafe extern "C" fn service_io_image(area: u32) -> *mut u8 {
    image_area(&mut crate::state(), area).0
}

unsafe extern "C" fn service_io_size(area: u32) -> u64 {
    image_area(&mut crate::state(), area).1
}

unsafe extern "C" fn service_state_load(buffer: *mut u8, len: i64, layout_hash: i64) -> i32 {
    // SAFETY: the generated module passes the ABI buffer contract.
    unsafe { crate::state::service_load(buffer, len, layout_hash) }
}

unsafe extern "C" fn service_state_save(buffer: *const u8, len: i64, layout_hash: i64) -> i32 {
    // SAFETY: the generated module passes the ABI buffer contract.
    unsafe { crate::state::service_save(buffer, len, layout_hash) }
}

unsafe extern "C" fn service_assert(condition: i32, code: i32, span_id: u32) {
    if condition == 0 {
        let mut state = service_state();
        if state.record_assertions {
            state.assertions.push(AssertRecord { code, span_id });
            return;
        }
        drop(state);
        // SAFETY: the trap service has the table's non-returning signature.
        unsafe {
            service_trap(code, span_id);
        }
    }
}

unsafe extern "C" fn service_set_cycle_time_ns(ns: i64) {
    crate::stc_rt_set_cycle_time_ns(ns);
}

// The host API 2.3 checkpoint hook is [`crate::stop::debug_check_hook`]. Its
// x86-64 trampoline captures the checkpointing function's frame pointer,
// stack pointer, and return address, then calls the session handler. Without
// an installed session the handler returns immediately; no module state is
// observed and no allocation occurs.

#[cfg(test)]
mod tests {
    use super::*;

    use crate::memory::MemoryLimits;

    static SERVICE_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        SERVICE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn table_has_the_abi_2_3_shape() {
        assert_eq!(HOST_API.abi_version, 0x0002_0003);
        assert_eq!(
            ABI_VERSION,
            (u32::from(stc_got::HOST_ABI_MAJOR) << 16) | u32::from(stc_got::HOST_ABI_MINOR)
        );
        assert_eq!(
            HOST_API.struct_size as usize,
            std::mem::size_of::<HostApi>()
        );
        assert!(HOST_API.trap.is_some());
        assert!(HOST_API.cycle_now_ns.is_some());
        assert!(HOST_API.monotonic_now_ns.is_some());
        assert!(HOST_API.local_date_ns.is_some());
        assert!(HOST_API.alloc.is_some());
        assert!(HOST_API.free.is_some());
        assert!(HOST_API.io_image.is_some());
        assert!(HOST_API.io_size.is_some());
        assert!(HOST_API.state_load.is_some());
        assert!(HOST_API.state_save.is_some());
        assert!(HOST_API.assert.is_some());
        assert!(HOST_API.set_cycle_time_ns.is_some());
        assert!(HOST_API.debug_check.is_some());
    }

    #[test]
    fn every_service_id_has_a_distinct_entry_point() {
        let mut addresses = Vec::new();
        for id in 1..=13 {
            let address = service_address(id).expect("service address");
            assert!(!addresses.contains(&address), "duplicate address for {id}");
            addresses.push(address);
        }
        assert_eq!(service_address(0), None);
        assert_eq!(service_address(14), None);
        assert_eq!(
            service_address(service::TRAP),
            Some(HOST_API.trap.expect("trap") as usize)
        );
    }

    #[test]
    fn debug_check_is_a_lock_free_no_op_without_a_session() {
        let check = HOST_API.debug_check.expect("debug_check");
        assert_eq!(service::DEBUG_CHECK, 13);
        assert_eq!(service_address(service::DEBUG_CHECK), Some(check as usize));
        // SAFETY: the table entry has the documented ABI signature and the
        // phase-2 body only reads its arguments.
        unsafe {
            check(1, 2);
            check(u32::MAX, u32::MAX);
        }
        assert!(last_trap().is_none());
        assert_eq!(current_generation(), 0, "the hook takes no host state");
    }

    #[test]
    fn transient_services_follow_the_current_generation() {
        let _lock = test_guard();
        let first = 11;
        let second = 12;
        register_generation(first).expect("first");
        register_generation(second).expect("second");
        assert!(register_generation(first).is_err());
        assert!(register_generation(0).is_err());

        let pointer = {
            let _guard = enter_generation(first);
            // SAFETY: the table's alloc entry is installed and the test owns
            // the current generation.
            unsafe { HOST_API.alloc.expect("alloc")(8, 8) }
        };
        assert!(!pointer.is_null());
        assert_eq!(current_generation(), 0);

        // A different generation does not own the block.
        {
            let _guard = enter_generation(second);
            // SAFETY: as above.
            unsafe { HOST_API.free.expect("free")(pointer) };
        }
        let faults = drain_faults(second);
        assert_eq!(faults.len(), 1);
        assert!(faults[0].contains("unknown or double free"));

        {
            let _guard = enter_generation(first);
            // SAFETY: as above.
            unsafe { HOST_API.free.expect("free")(pointer) };
        }
        assert!(drain_faults(first).is_empty());
        // A live block's alignment padding counts towards its reserved bytes.
        {
            let _guard = enter_generation(first);
            // SAFETY: the alloc entry is installed.
            let live = unsafe { HOST_API.alloc.expect("alloc")(8, 8) };
            assert!(!live.is_null());
        }
        assert_eq!(withdraw_generation(first), 15);
        assert_eq!(withdraw_generation(first), 0);
    }

    #[test]
    fn allocation_failures_return_null_without_faults() {
        let _lock = test_guard();
        let generation = 21;
        register_generation(generation).expect("register");
        let mut state = service_state();
        state.allocators.insert(
            generation,
            TransientAllocator::with_limits(
                generation,
                MemoryLimits {
                    max_total_bytes: 8,
                    max_allocation_bytes: 8,
                    max_align: 8,
                },
            ),
        );
        drop(state);
        let _guard = enter_generation(generation);
        // SAFETY: the alloc entry is installed.
        let pointer = unsafe { HOST_API.alloc.expect("alloc")(16, 8) };
        assert!(pointer.is_null());
        assert!(drain_faults(generation).is_empty());
        withdraw_generation(generation);
    }

    #[test]
    fn image_areas_index_the_process_image() {
        let mut state = crate::State {
            image: vec![0; 9],
            inputs: 4,
            outputs: 2,
            markers: 3,
            cycle_now_ns: 0,
            pending_cycle_ns: None,
        };
        let base = state.image.as_ptr() as usize;
        let (inputs, input_len) = image_area(&mut state, 1);
        let (outputs, output_len) = image_area(&mut state, 2);
        let (markers, marker_len) = image_area(&mut state, 3);
        assert_eq!(input_len, 4);
        assert_eq!(output_len, 2);
        assert_eq!(marker_len, 3);
        assert_eq!(inputs as usize, base);
        assert_eq!(outputs as usize, base + 4);
        assert_eq!(markers as usize, base + 6);
        assert_eq!(image_area(&mut state, 0).1, 0);
        assert_eq!(image_area(&mut state, 4).1, 0);
        assert!(image_area(&mut state, 9).0.is_null());

        let mut empty = crate::State {
            image: Vec::new(),
            inputs: 0,
            outputs: 0,
            markers: 0,
            cycle_now_ns: 0,
            pending_cycle_ns: None,
        };
        assert!(image_area(&mut empty, 1).0.is_null());
        assert_eq!(image_area(&mut empty, 1).1, 0);
    }

    #[test]
    fn retained_state_services_report_disabled() {
        let _lock = crate::test_support::state_guard();
        crate::state::configure(crate::state::RetainConfig::default());
        // SAFETY: the state entries are installed and use the documented stub
        // signatures.
        unsafe {
            assert_eq!(
                HOST_API.state_load.expect("load")(std::ptr::null_mut(), 0, 0),
                5
            );
            assert_eq!(
                HOST_API.state_save.expect("save")(std::ptr::null(), 0, 0),
                5
            );
        }
    }
}
