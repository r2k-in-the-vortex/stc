//! `stc ethercat slave pdos|config show|config validate` CLI tests (WL-7 phase 3).
//!
//! The tests drive the built binary with engineering-configuration drafts and
//! need no EtherCAT hardware, shim library or Linux host. A live PDO read-back
//! reports the missing ABI 2 capability.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn stc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_stc"))
}

fn workdir(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "stc-ethercat-config-cli-{}-{name}",
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
fn config_show_reports_the_selected_mode_and_stored_alternates() {
    let directory = workdir("show");
    let config = write(&directory, "config.json", CONFIG);
    let output = run(&[
        "ethercat",
        "config",
        "show",
        "--config",
        config.to_str().expect("path"),
    ]);
    assert!(output.status.success(), "{}", text(&output));
    let printed = text(&output);
    assert!(printed.contains("configuration: master 0, 1 device(s)"));
    assert!(printed.contains("position 0: io"));
    assert!(printed.contains("selected=outputs-inputs"));
    assert!(printed.contains("2 channel(s)"), "{printed}");
    assert!(printed.contains("dc free-run"));
}

#[test]
fn config_validate_accepts_a_valid_draft_offline() {
    let directory = workdir("validate");
    let config = write(&directory, "config.json", CONFIG);
    let output = run(&[
        "ethercat",
        "config",
        "validate",
        "--config",
        config.to_str().expect("path"),
    ]);
    assert!(output.status.success(), "{}", text(&output));
    let printed = text(&output);
    assert!(printed.contains("position 0 (io): valid"));
    assert!(printed.contains("esi-resolved=false"));
    assert!(printed.contains("result: valid"));

    let output = run(&[
        "ethercat",
        "config",
        "validate",
        "--config",
        config.to_str().expect("path"),
        "--bound-channel",
        "io:outputs",
        "--bound-channel=io:inputs",
        "--cycle-ns",
        "1000000",
    ]);
    assert!(output.status.success(), "{}", text(&output));
}

#[test]
fn a_bound_channel_missing_from_the_selected_mode_is_rejected() {
    let directory = workdir("bound");
    let config = write(&directory, "config.json", CONFIG);
    let output = run(&[
        "ethercat",
        "config",
        "validate",
        "--config",
        config.to_str().expect("path"),
        "--bound-channel",
        "io:not-exposed",
    ]);
    assert!(!output.status.success());
    let printed = text(&output);
    assert!(printed.contains("STC-E-IO-ETH-024"), "{printed}");
    assert!(printed.contains("not-exposed"), "{printed}");
}

#[test]
fn an_invalid_channel_exposure_is_rejected_by_code() {
    let directory = workdir("channel");
    let invalid = CONFIG.replace(
        "\"bit\": 0,\n              \"width\": 16",
        "\"bit\": 0,\n              \"width\": 64",
    );
    assert_ne!(invalid, CONFIG);
    let config = write(&directory, "config.json", &invalid);
    let output = run(&[
        "ethercat",
        "config",
        "validate",
        "--config",
        config.to_str().expect("path"),
    ]);
    assert!(!output.status.success());
    assert!(
        text(&output).contains("STC-E-IO-ETH-024"),
        "{}",
        text(&output)
    );
}

#[test]
fn a_duplicate_mode_name_and_an_invalid_pdo_are_rejected() {
    let directory = workdir("mode");
    let invalid = CONFIG.replace(
        "\"selected\": \"outputs-inputs\"",
        "\"selected\": \"missing\"",
    );
    let config = write(&directory, "config.json", &invalid);
    let output = run(&[
        "ethercat",
        "config",
        "validate",
        "--config",
        config.to_str().expect("path"),
    ]);
    assert!(!output.status.success());
    assert!(
        text(&output).contains("STC-E-IO-ETH-020"),
        "{}",
        text(&output)
    );

    let invalid = CONFIG.replace(
        "\"index\": 28672, \"subindex\": 1, \"bit_len\": 16, \"declared_in\": 5632",
        "\"index\": 28672, \"subindex\": 1, \"bit_len\": 0, \"declared_in\": 5632",
    );
    assert_ne!(invalid, CONFIG);
    let config = write(&directory, "zero.json", &invalid);
    let output = run(&[
        "ethercat",
        "config",
        "validate",
        "--config",
        config.to_str().expect("path"),
    ]);
    assert!(!output.status.success());
    assert!(
        text(&output).contains("STC-E-IO-ETH-021"),
        "{}",
        text(&output)
    );
}

#[test]
fn config_validation_reports_structured_json() {
    let directory = workdir("json");
    let config = write(&directory, "config.json", CONFIG);
    let output = run(&[
        "ethercat",
        "config",
        "validate",
        "--config",
        config.to_str().expect("path"),
        "--message-format",
        "json",
    ]);
    assert!(output.status.success(), "{}", text(&output));
    let printed = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(printed.contains("\"valid\": true"), "{printed}");
    assert!(printed.contains("\"esi_resolved\": false"), "{printed}");
    assert!(
        printed.contains("\"selected\": \"outputs-inputs\""),
        "{printed}"
    );
}

#[test]
fn an_unreadable_or_malformed_config_is_a_structured_error() {
    let directory = workdir("unreadable");
    let output = run(&[
        "ethercat",
        "config",
        "show",
        "--config",
        directory.join("missing.json").to_str().expect("path"),
    ]);
    assert!(!output.status.success());
    assert!(
        text(&output).contains("STC-E-IO-ETH-020"),
        "{}",
        text(&output)
    );

    let malformed = write(&directory, "malformed.json", "{not json");
    let output = run(&[
        "ethercat",
        "config",
        "validate",
        "--config",
        malformed.to_str().expect("path"),
    ]);
    assert!(!output.status.success());
    assert!(
        text(&output).contains("STC-E-IO-ETH-020"),
        "{}",
        text(&output)
    );
}

#[test]
fn live_pdo_read_back_fails_closed_without_the_abi_2_capability() {
    let output = run(&["ethercat", "slave", "pdos", "0"]);
    assert!(!output.status.success());
    let printed = text(&output);
    assert!(printed.contains("STC-E-IO-ETH-043"), "{printed}");
    assert!(printed.contains("pdo-read-back"), "{printed}");
    assert!(!printed.contains("no slaves"), "{printed}");
}
