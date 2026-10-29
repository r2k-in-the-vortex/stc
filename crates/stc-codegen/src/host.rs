//! Host target resolution.
//!
//! This module is the single operating-system conditional in code generation
//! (D-201). It chooses the default target triple when the caller does not name
//! one; no code-generation rule branches on the host.

/// Returns the host target triple used when `--target` is omitted.
#[must_use]
pub fn host_triple() -> String {
    if cfg!(all(windows, target_env = "msvc")) {
        "x86_64-pc-windows-msvc".into()
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu".into()
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu".into()
    } else {
        "x86_64-unknown-linux-gnu".into()
    }
}
