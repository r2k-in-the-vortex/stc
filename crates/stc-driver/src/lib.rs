//! Driver support library.
//!
//! The binary owns the CLI; this library exposes the host toolchain discovery
//! used by `stc build` so integration tests can link C harnesses with the same
//! flags as the driver.

#![forbid(unsafe_code)]

pub mod toolchain;
