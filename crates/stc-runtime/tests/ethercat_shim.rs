//! Linux smoke test for the dynamically loaded IgH EtherCAT shim.
//!
//! The test only performs the version and symbol handshake; it never opens a
//! master, so CI does not need EtherCAT hardware or kernel modules. The shim
//! path comes from `STC_ETHERCAT_LIBRARY` (decision E-8). When the variable is
//! unset the test is skipped so a plain Linux `cargo test` still passes.

#![cfg(target_os = "linux")]

use std::path::Path;

use stc_runtime::{SHIM_ENV, probe_shim};

#[test]
fn configured_shim_resolves_the_versioned_abi_without_a_master() {
    let Some(path) = std::env::var_os(SHIM_ENV) else {
        eprintln!("skipping: {SHIM_ENV} is not set");
        return;
    };
    let info = probe_shim(Path::new(&path)).expect("shim must load and pass the ABI handshake");
    assert_eq!(info.abi_version, 1);
    assert_eq!(info.name, "stc-io-ethercat/1");
    assert!(
        info.shim_version.starts_with("stc-io-ethercat-shim/1"),
        "unexpected shim version `{}`",
        info.shim_version
    );
}

#[test]
fn missing_shim_reports_eth_001() {
    let error = probe_shim(Path::new("/nonexistent/stc/libstc_io_ethercat.so"))
        .expect_err("a missing shim must fail");
    assert_eq!(error.code, "STC-E-IO-ETH-001");
}

#[test]
fn the_bare_loader_name_matches_the_project_shim() {
    assert_eq!(stc_runtime::SHIM_LIBRARY, "libstc_io_ethercat.so");
}
