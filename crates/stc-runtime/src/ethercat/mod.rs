//! EtherCAT engineering subsystem (WL-7).
//!
//! WL-7 keeps four layers apart: an ESI-derived **device description**, the
//! engineer-authored **desired configuration**, the **observed state** of a
//! connected bus, and the validated **effective process image** (WL-7 plan §2).
//! Importing a description never selects PDOs, an operating mode or PLC
//! channels.
//!
//! Phase 1 delivers the typed engineering API, the bounded ESI reader and
//! capability model, the deterministic ESI library, the `STC-E-ESI-*`
//! diagnostic catalog, and the shim ABI 2 capability/result contract with its
//! host-neutral mock. The ABI 2 loader, topology, PDO/SDO and runtime work land
//! in later phases.

// The structured diagnostic payload (D-280) is deliberately rich: code,
// severity, operation, source, device/position, expected/observed, missing
// capability and remedy. It is the API contract for CLI, TUI and GUI clients
// and is only constructed when an engineering operation is rejected, never on
// the cycle path, so the large `Err` variant is intentional.
#![allow(clippy::result_large_err)]

pub mod abi;
pub mod api;
pub mod diagnostics;
pub mod esi;
pub mod mock;
