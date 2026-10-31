//! Statement-coverage protocol constants shared by codegen, the test host, and
//! the test driver (WL-10).
//!
//! The counter symbol itself is
//! [`crate::descriptor::COVERAGE_COUNTERS`]; this module adds the raw sidecar
//! protocol and the cap that bounds a host readout. Both are internal
//! compiler protocols: they are not part of the deployable module ABI and have
//! no stability guarantee outside the compiler.
//!
//! A raw sidecar starts with `stc-coverage-raw/1 <site-count>` on its first
//! line, followed by one `id count` line per nonzero counter in ascending ID
//! order (WL-10 C-5, §6.2).

/// Internal raw coverage sidecar protocol string.
pub const RAW_PROTOCOL: &str = "stc-coverage-raw/1";

/// Maximum number of coverage sites a test host accepts.
///
/// A test host validates this before reading the exported counter array; a
/// larger declared count is a warning and no sidecar is written (WL-10 §6.2).
pub const MAX_SITES: u32 = 1_000_000;
