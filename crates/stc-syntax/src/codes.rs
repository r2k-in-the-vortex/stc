//! Stable diagnostic codes owned by `stc-syntax` (pragma tooling).

/// Duplicate `stc:io` pragma on one declaration (IO §4).
pub const DUPLICATE_IO_PRAGMA: &str = "STC-E-IO-003";

/// `stc:io` on a declaration without `AT` (IO §4).
pub const IO_WITHOUT_AT: &str = "STC-W-IO-002";
