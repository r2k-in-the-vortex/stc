//! `stc ethercat` CLI tests (WL-7 phase 1).
//!
//! The tests use the committed synthetic ESI fixtures and the built binary;
//! they need no EtherCAT hardware, shim library or Linux host.

use std::path::{Path, PathBuf};
use std::process::Command;

fn stc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_stc"))
}

fn fixture(parts: &[&str]) -> PathBuf {
    let mut path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../stc-runtime/tests/fixtures/esi")
        .canonicalize()
        .expect("fixture root");
    for part in parts {
        path.push(part);
    }
    path
}

#[test]
fn esi_list_and_search_emit_structured_results() {
    let output = stc()
        .args(["ethercat", "esi", "list", "--esi-dir"])
        .arg(fixture(&["library"]))
        .args(["--message-format", "json"])
        .output()
        .expect("run stc");
    assert!(output.status.success(), "{:?}", output);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("structured JSON");
    assert_eq!(
        value["descriptions"]
            .as_array()
            .expect("descriptions")
            .len(),
        3
    );
    assert_eq!(value["problems"][0]["code"], "STC-E-ESI-002");
    assert_eq!(value["problems"][0]["severity"], "error");
    assert!(value["descriptions"][0]["digest"].is_string());

    let output = stc()
        .args(["ethercat", "esi", "search", "diewu", "--esi-dir"])
        .arg(fixture(&["library"]))
        .output()
        .expect("run stc");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("utf-8");
    assert!(text.contains("DIEWU 16I/16O"), "{text}");
    assert_eq!(text.lines().count(), 2);
}

#[test]
fn esi_inspect_reports_capabilities_for_a_source_and_an_identity() {
    let output = stc()
        .args(["ethercat", "esi", "inspect"])
        .arg(fixture(&["library", "diewu_16io.xml"]))
        .output()
        .expect("run stc");
    assert!(output.status.success(), "{:?}", output);
    let text = String::from_utf8(output.stdout).expect("utf-8");
    assert!(text.contains("source:"));
    assert!(text.contains("device: DIEWU 16I/16O"));
    assert!(text.contains("dc op modes: DC Sync0, DC Sync1"));
    assert!(text.contains("coe: sdo-info=true"));

    let output = stc()
        .args([
            "ethercat",
            "esi",
            "inspect",
            "--vendor",
            "#x000007d8",
            "--product",
            "0x3052",
            "--revision",
            "#x00110000",
            "--esi-dir",
        ])
        .arg(fixture(&["library"]))
        .args(["--message-format", "json"])
        .output()
        .expect("run stc");
    assert!(output.status.success(), "{:?}", output);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(value["pinned"], true);
    assert_eq!(value["devices"][0]["product_code"], 0x0000_3052);
    assert_eq!(value["devices"][0]["revision"], 0x0011_0000);
    assert_eq!(
        value["devices"][0]["tx_pdos"]
            .as_array()
            .expect("pdos")
            .len(),
        2
    );
}

#[test]
fn a_rejected_operation_exits_1_with_a_stable_code() {
    let output = stc()
        .args(["ethercat", "esi", "inspect"])
        .arg(fixture(&["invalid", "version_1_0.xml"]))
        .output()
        .expect("run stc");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("STC-E-ESI-002"), "{stderr}");

    let output = stc()
        .args([
            "ethercat",
            "esi",
            "inspect",
            "--vendor",
            "1",
            "--product",
            "1",
            "--esi-dir",
        ])
        .arg(fixture(&["library"]))
        .output()
        .expect("run stc");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("STC-E-ESI-007"),
        "{:?}",
        output
    );
}

#[test]
fn usage_errors_exit_2() {
    for args in [
        vec!["ethercat"],
        vec!["ethercat", "esi", "list"],
        vec!["ethercat", "esi", "inspect"],
        vec![
            "ethercat",
            "esi",
            "search",
            "x",
            "--esi-dir",
            "esi",
            "extra",
        ],
    ] {
        let output = stc().args(&args).output().expect("run stc");
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
}
