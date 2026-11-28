//! `stc ethercat sdo list|inspect|read|write` CLI tests (WL-7 phase 4).
//!
//! The tests drive the built binary with the committed synthetic ESI fixture.
//! Offline dictionary browsing needs no hardware; live reads and writes fail
//! closed with the missing ABI 2 capability, and an SDO transfer outside setup
//! mode is rejected by code.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

fn workdir(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "stc-ethercat-sdo-cli-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("work directory");
    directory
}

const CONFIG: &str = r#"{
  "format": 1,
  "protocol": "stc-ethercat-config/1",
  "master": 0,
  "devices": [
    {
      "name": "io",
      "position": 0,
      "vendor_id": 2008,
      "product_code": 12370,
      "revision": 1114112,
      "esi": {"source": "esi/diewu_16io.xml", "digest": "synthetic"},
      "mailbox": {"pdo_assign": true, "pdo_config": true},
      "selected": "outputs-inputs",
      "modes": [
        {
          "name": "outputs-inputs",
          "description": "Digital IO",
          "provenance": null,
          "pdos": [
            {
              "direction": "rx",
              "index": 5632,
              "sm": 2,
              "fixed": false,
              "mandatory": true,
              "exclude": [],
              "excluded_sm": [],
              "entries": [
                {"index": 28672, "subindex": 1, "bit_len": 16, "declared_in": 5632}
              ]
            },
            {
              "direction": "tx",
              "index": 6656,
              "sm": 3,
              "fixed": true,
              "mandatory": true,
              "exclude": [],
              "excluded_sm": [],
              "entries": [
                {"index": 24576, "subindex": 1, "bit_len": 16, "declared_in": 6656}
              ]
            }
          ],
          "channels": [
            {
              "name": "outputs",
              "pdo": 5632,
              "entry": {"index": 28672, "subindex": 1},
              "direction": "rx",
              "bit": 0,
              "width": 16
            },
            {
              "name": "inputs",
              "pdo": 6656,
              "entry": {"index": 24576, "subindex": 1},
              "direction": "tx",
              "bit": 0,
              "width": 16
            }
          ],
          "startup": [],
          "dc": {"mode": "free-run"}
        }
      ]
    }
  ]
}
"#;

fn write(directory: &Path, name: &str, text: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, text).expect("write fixture");
    path
}

fn run(args: &[&str]) -> Output {
    stc().args(args).output().expect("run stc")
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn sdo_list_and_inspect_report_the_esi_dictionary_offline() {
    let directory = workdir("list");
    let config = write(&directory, "config.json", CONFIG);
    let esi = fixture(&["library"]);
    let output = run(&[
        "ethercat",
        "sdo",
        "list",
        "0",
        "--config",
        config.to_str().expect("path"),
        "--esi-dir",
        esi.to_str().expect("path"),
    ]);
    assert!(output.status.success(), "{}", text(&output));
    let printed = text(&output);
    assert!(printed.contains("position: 0 (io)"), "{printed}");
    assert!(
        printed.contains("0x1018:1 `Vendor ID` type=UDINT access=ro source=esi"),
        "{printed}"
    );

    let output = run(&[
        "ethercat",
        "sdo",
        "inspect",
        "0",
        "0x1018",
        "1",
        "--config",
        config.to_str().expect("path"),
        "--esi-dir",
        esi.to_str().expect("path"),
        "--message-format",
        "json",
    ]);
    assert!(output.status.success(), "{}", text(&output));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(json["device"], "io");
    assert_eq!(json["object"]["index"], 0x1018);
    assert_eq!(json["object"]["subindex"], 1);
    assert_eq!(json["object"]["source"], "esi");
    assert_eq!(json["object"]["access"], "ro");
    assert_eq!(json["object"]["online_only"], false);
}

#[test]
fn an_unknown_object_is_a_structured_error() {
    let directory = workdir("unknown");
    let config = write(&directory, "config.json", CONFIG);
    let esi = fixture(&["library"]);
    let output = run(&[
        "ethercat",
        "sdo",
        "inspect",
        "0",
        "0x9999",
        "0",
        "--config",
        config.to_str().expect("path"),
        "--esi-dir",
        esi.to_str().expect("path"),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let printed = text(&output);
    assert!(printed.contains("STC-E-IO-ETH-030"), "{printed}");
    assert!(printed.contains("0x9999:0"), "{printed}");
}

#[test]
fn live_reads_and_writes_require_setup_mode() {
    let directory = workdir("setup");
    let config = write(&directory, "config.json", CONFIG);
    let esi = fixture(&["library"]);
    for args in [
        vec![
            "ethercat",
            "sdo",
            "read",
            "0",
            "0x1018",
            "1",
            "--config",
            config.to_str().expect("path"),
            "--esi-dir",
            esi.to_str().expect("path"),
        ],
        vec![
            "ethercat",
            "sdo",
            "write",
            "0",
            "0x1018",
            "1",
            "2008",
            "--config",
            config.to_str().expect("path"),
            "--esi-dir",
            esi.to_str().expect("path"),
        ],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        let printed = text(&output);
        assert!(printed.contains("STC-E-IO-ETH-041"), "{printed}");
    }
}

#[test]
fn an_unparseable_value_is_reported_with_its_object() {
    let directory = workdir("value");
    let config = write(&directory, "config.json", CONFIG);
    let esi = fixture(&["library"]);
    let output = run(&[
        "ethercat",
        "sdo",
        "write",
        "0",
        "0x1018",
        "1",
        "not-a-number",
        "--config",
        config.to_str().expect("path"),
        "--esi-dir",
        esi.to_str().expect("path"),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let printed = text(&output);
    assert!(printed.contains("STC-E-IO-ETH-035"), "{printed}");
    assert!(printed.contains("0x1018:1"), "{printed}");
}

#[test]
fn a_negative_literal_is_a_value_not_an_option() {
    let directory = workdir("negative");
    let config = write(&directory, "config.json", CONFIG);
    let esi = fixture(&["library"]);
    let output = run(&[
        "ethercat",
        "sdo",
        "write",
        "0",
        "0x1018",
        "1",
        "-3",
        "--config",
        config.to_str().expect("path"),
        "--esi-dir",
        esi.to_str().expect("path"),
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", text(&output));
    let printed = text(&output);
    assert!(!printed.contains("unknown option"), "{printed}");
    assert!(printed.contains("STC-E-IO-ETH-035"), "{printed}");
}

#[test]
fn sdo_usage_errors_exit_2() {
    let directory = workdir("usage");
    let config = write(&directory, "config.json", CONFIG);
    for args in [
        vec!["ethercat", "sdo", "list", "0"],
        vec![
            "ethercat",
            "sdo",
            "list",
            "0",
            "--config",
            config.to_str().expect("path"),
            "--cycle-ns",
            "1000000",
        ],
        vec![
            "ethercat",
            "sdo",
            "inspect",
            "0",
            "not-an-index",
            "--config",
            config.to_str().expect("path"),
        ],
        vec![
            "ethercat",
            "sdo",
            "write",
            "0",
            "0x1018",
            "1",
            "2008",
            "extra",
            "--config",
            config.to_str().expect("path"),
        ],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
}
