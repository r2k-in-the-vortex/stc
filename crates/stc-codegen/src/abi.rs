//! Runtime ABI names and trap codes (Runtime-and-Execution §2.3).

/// Runtime ABI major version embedded in metadata.
pub const ABI_VERSION: i32 = 2;

/// Trap codes passed to `stc_rt_trap`.
pub mod trap {
    /// Integer overflow.
    pub const OVERFLOW: i32 = 1;
    /// Division or remainder by zero.
    pub const DIV_ZERO: i32 = 2;
    /// Value does not fit the destination type.
    pub const RANGE: i32 = 3;
    /// Array/string bound violation (M5).
    pub const BOUNDS: i32 = 4;
    /// Null dereference (M5).
    pub const NULL_REF: i32 = 5;
    /// Invalid UTF-8 (M5).
    pub const INVALID_UTF8: i32 = 6;
    /// Invalid conversion (NaN or non-representable).
    pub const CONVERSION: i32 = 7;
    /// Watchdog timeout.
    pub const WATCHDOG: i32 = 8;
    /// Assertion failure.
    pub const ASSERT: i32 = 9;
    /// Reached an unimplemented path.
    pub const UNIMPLEMENTED: i32 = 10;
}

/// Module container entry points (GOT Artifacts §7.3).
pub mod module {
    /// Descriptor accessor.
    pub const DESCRIPTOR: &str = "stc_module_descriptor";
    /// Function-address accessor.
    pub const FUNCTIONS: &str = "stc_module_functions";
    /// Host attach entry point.
    pub const ATTACH: &str = "stc_module_attach";
    /// Root preparation entry point.
    pub const PREPARE: &str = "stc_module_prepare";
    /// Cycle entry point.
    pub const CYCLE: &str = "stc_module_cycle";
    /// Shutdown entry point.
    pub const SHUTDOWN: &str = "stc_module_shutdown";
    /// Detach entry point.
    pub const DETACH: &str = "stc_module_detach";

    /// Module-local cached host API pointer.
    pub const API_GLOBAL: &str = "stc_module_api";
    /// Module-local cached slot-table pointer.
    pub const SLOTS_GLOBAL: &str = "stc_module_slots";
    /// Embedded descriptor blob.
    pub const DESCRIPTOR_DATA: &str = "stc_module_descriptor_data";
    /// Module-local function-address table.
    pub const FUNCTION_TABLE: &str = "stc_module_function_table";
}

/// ABI 2 host API table layout constants used by generated entry points.
///
/// Minor 1 (host ABI 2.1) adds no service: it records the inline TYPEINFO,
/// VTABLE, and object-record layout that generated code reads directly
/// (GOT Artifacts §8.5, Runtime and Execution §2.6). Minor 2 (host ABI 2.2)
/// appends the test-clock service (ID 12) after assert; only test builds
/// import it and therefore require the larger table prefix.
pub mod host_api {
    /// `(major << 16) | minor` for ABI 2.2.
    pub const ABI_VERSION: u32 = (2 << 16) | 2;
    /// Offset of `abi_version` in `stc_host_api`.
    pub const ABI_VERSION_OFFSET: u64 = 0;
    /// Offset of `struct_size` in `stc_host_api`.
    pub const STRUCT_SIZE_OFFSET: u64 = 4;
    /// Required prefix length for a production module: version, size, and the
    /// eleven ABI 2.1 service pointers.
    pub const REQUIRED_SIZE: u64 = 8 + 11 * 8;
    /// Required prefix length for a test-build module: version, size, and the
    /// twelve ABI 2.2 service pointers.
    pub const REQUIRED_SIZE_TEST: u64 = 8 + 12 * 8;
}

/// Built-in unit symbols.
pub mod symbols {
    /// Trap entry point.
    pub const TRAP: &str = "stc_rt_trap";
    /// Executable cycle loop.
    pub const EXECUTE: &str = "stc_rt_execute";
    /// Latched cycle clock.
    pub const CYCLE_NOW: &str = "stc_rt_cycle_now_ns";
    /// Live monotonic clock.
    pub const MONOTONIC_NOW: &str = "stc_rt_monotonic_now_ns";
    /// Next-cycle clock override.
    pub const SET_CYCLE_TIME: &str = "stc_rt_set_cycle_time_ns";
    /// Process-image configuration.
    pub const IO_CONFIGURE: &str = "stc_rt_io_configure";
    /// Process-image base/length accessor.
    pub const IO_IMAGE: &str = "stc_rt_io_image";
    /// Retained-state load.
    pub const STATE_LOAD: &str = "stc_rt_state_load";
    /// Retained-state save.
    pub const STATE_SAVE: &str = "stc_rt_state_save";
    /// Module init.
    pub const INIT: &str = "stc_init";
    /// Module cycle.
    pub const CYCLE: &str = "stc_cycle";
    /// Module shutdown.
    pub const SHUTDOWN: &str = "stc_shutdown";
    /// Metadata JSON function.
    pub const METADATA_JSON: &str = "stc_metadata_json";
    /// Retained payload size.
    pub const RETAIN_SIZE: &str = "stc_retain_size";
    /// Retained layout hash.
    pub const RETAIN_HASH: &str = "stc_retain_layout_hash";
    /// Copy retained leaves into a buffer.
    pub const RETAIN_SAVE: &str = "stc_retain_save";
    /// Restore retained leaves from a buffer.
    pub const RETAIN_LOAD: &str = "stc_retain_load";
    /// Process-image base pointer global.
    pub const IO_BASE: &str = "stc_io_base";
    /// Process-image length global.
    pub const IO_LEN: &str = "stc_io_len";
    /// Retained payload buffer global.
    pub const RETAIN_BUFFER: &str = "stc_retain_buffer";
    /// Embedded metadata JSON global.
    pub const METADATA_DATA: &str = "stc_metadata_json_data";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_abi_is_2_2_with_the_test_clock_appended() {
        assert_eq!(host_api::ABI_VERSION, 0x0002_0002);
        assert_eq!(
            host_api::ABI_VERSION >> 16,
            u32::from(stc_got::HOST_ABI_MAJOR)
        );
        assert_eq!(
            host_api::ABI_VERSION & 0xFFFF,
            u32::from(stc_got::HOST_ABI_MINOR)
        );
        // Production modules keep the ABI 2.1 prefix; test builds append the
        // twelfth service pointer.
        assert_eq!(host_api::REQUIRED_SIZE, 8 + 11 * 8);
        assert_eq!(host_api::REQUIRED_SIZE_TEST, 8 + 12 * 8);
        // Metadata records the ABI major version.
        assert_eq!(ABI_VERSION, i32::from(stc_got::HOST_ABI_MAJOR));
    }
}
