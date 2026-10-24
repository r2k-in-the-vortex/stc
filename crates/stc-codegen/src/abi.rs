//! Runtime ABI names and trap codes (Runtime-and-Execution §2.3).

/// Runtime ABI version embedded in metadata.
pub const ABI_VERSION: i32 = 1;

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
