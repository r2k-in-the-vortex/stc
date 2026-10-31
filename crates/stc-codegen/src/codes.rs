//! Stable codegen and build diagnostic codes (D-092; Conformance Profile §10.3).

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
/// Invalid I/O device, channel, or binding from `stc.toml`.
pub const PROJECT_IO: &str = "STC-E-PROJECT-009";
/// A route sink has more than one source (fan-in), or a binding and a route
/// claim the same sink.
pub const IO_BINDING_CONFLICT: &str = "STC-E-IO-BINDING-001";
/// A route endpoint does not resolve, has the wrong direction, or its width
/// does not match.
pub const IO_BINDING_INVALID: &str = "STC-E-IO-BINDING-002";
/// Project binding overrides a pragma hint.
pub const W_PROJECT_OVERRIDES_HINT: &str = "STC-W-IO-003";
/// Pragma names a device not declared in `stc.toml`.
pub const W_IO_UNKNOWN_DEVICE: &str = "STC-W-IO-001";
