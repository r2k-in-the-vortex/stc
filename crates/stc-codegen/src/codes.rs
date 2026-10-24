//! Stable codegen and build diagnostic codes (M4-Plan §8, D-092).

/// A valid HIR construct that M4 does not implement.
pub const CODEGEN_UNSUPPORTED: &str = "STC-E-CODEGEN-001";
/// Internal codegen failure or IR invariant violation.
pub const CODEGEN_INTERNAL: &str = "STC-E-CODEGEN-002";
/// Clang missing, unsupported, or unusable.
pub const BUILD_CLANG: &str = "STC-E-BUILD-001";
/// Clang invocation or link failure.
pub const BUILD_LINK: &str = "STC-E-BUILD-002";
/// Runtime static library missing or ABI mismatch.
pub const BUILD_RUNTIME: &str = "STC-E-BUILD-003";
/// Invalid build option or artifact combination.
pub const BUILD_OPTIONS: &str = "STC-E-BUILD-004";
/// Multiple tasks or unsupported scheduling.
pub const RUN_TASK: &str = "STC-E-RUN-001";
/// No runnable program/configuration, or ambiguous configuration.
pub const RUN_PLAN: &str = "STC-E-RUN-002";
