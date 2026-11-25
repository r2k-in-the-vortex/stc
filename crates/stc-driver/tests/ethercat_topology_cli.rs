//! `stc ethercat topology|slave|setup` CLI tests (WL-7 phase 2).
//!
//! The tests drive the built binary with topology drafts and recorded
//! observations in a temporary directory. They need no EtherCAT hardware, shim
//! library or Linux host; a live scan reports the missing ABI 2 capability.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn stc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_stc"))
}

fn workdir(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "stc-ethercat-topology-cli-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("work directory");
    directory
}

const TOPOLOGY: &str = r#"{
  "format": 1,
  "protocol": "stc-ethercat-topology/1",
  "master": 0,
  "devices": [
    {"name": "input", "position": 0, "vendor_id": 2965, "product_code": 66048, "revision": 1},
    {"name": "output", "position": 1, "vendor_id": 2965, "product_code": 66048, "revision": 1}
  ]
}
"#;

const OBSERVED: &str = r#"{
  "format": 1,
  "protocol": "stc-ethercat-observed/1",
  "master": 0,
  "slaves": [
    {"position": 0, "vendor_id": 2965, "product_code": 66048, "revision": 1,
     "serial": 11, "alias": 0, "al_state": 8, "online": true},
    {"position": 1, "vendor_id": 2965, "product_code": 66048, "revision": 1,
     "serial": 12, "alias": 0, "al_state": 8, "online": true}
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
fn topology_show_validates_a_draft_offline() {
    let directory = workdir("show");
    let topology = write(&directory, "topology.json", TOPOLOGY);
    let output = run(&[
        "ethercat",
        "topology",
        "show",
        "--topology",
        topology.to_str().expect("path"),
    ]);
    assert!(output.status.success(), "{}", text(&output));
    let printed = text(&output);
    assert!(
        printed.contains("topology: master 0, 2 device(s)"),
        "{printed}"
    );
    assert!(printed.contains("position 0: input"), "{printed}");
    assert!(printed.contains("valid: dense positions"), "{printed}");

    let output = run(&[
        "ethercat",
        "topology",
        "show",
        "--topology",
        topology.to_str().expect("path"),
        "--message-format",
        "json",
    ]);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(value["valid"], true);
    assert_eq!(value["topology"]["devices"][1]["name"], "output");
    assert_eq!(
        value["topology"]["devices"][0]["serial"],
        serde_json::Value::Null
    );

    let gap = write(
        &directory,
        "gap.json",
        &TOPOLOGY.replace("\"position\": 1", "\"position\": 2"),
    );
    let output = run(&[
        "ethercat",
        "topology",
        "show",
        "--topology",
        gap.to_str().expect("path"),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output).contains("STC-E-IO-ETH-009"),
        "{}",
        text(&output)
    );

    let wildcard = write(
        &directory,
        "wildcard.json",
        &TOPOLOGY.replace("\"vendor_id\": 2965", "\"vendor_id\": \"any\""),
    );
    let output = run(&[
        "ethercat",
        "topology",
        "show",
        "--topology",
        wildcard.to_str().expect("path"),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output).contains("STC-E-IO-ETH-010"),
        "{}",
        text(&output)
    );
}

#[test]
fn topology_diff_compares_every_position() {
    let directory = workdir("diff");
    let topology = write(&directory, "topology.json", TOPOLOGY);
    let observed = write(&directory, "observed.json", OBSERVED);

    let output = run(&[
        "ethercat",
        "topology",
        "diff",
        "--topology",
        topology.to_str().expect("path"),
        "--observed",
        observed.to_str().expect("path"),
    ]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("result: match"), "{}", text(&output));

    let mismatched = write(
        &directory,
        "mismatched.json",
        &OBSERVED.replace(
            "\"revision\": 1,\n     \"serial\": 12",
            "\"revision\": 2,\n     \"serial\": 12",
        ),
    );
    let output = run(&[
        "ethercat",
        "topology",
        "diff",
        "--topology",
        topology.to_str().expect("path"),
        "--observed",
        mismatched.to_str().expect("path"),
        "--message-format",
        "json",
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", text(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(value["match"], false);
    assert_eq!(value["positions"][0]["result"], "match");
    assert_eq!(value["positions"][1]["result"], "different");
    assert_eq!(value["diagnostics"][0]["code"], "STC-E-IO-ETH-003");
    assert_eq!(value["diagnostics"][0]["position"], 1);
}

#[test]
fn topology_capture_records_identity_and_pins_only_on_request() {
    let directory = workdir("capture");
    let observed = write(&directory, "observed.json", OBSERVED);
    let output_path = directory.join("draft.json");

    let output = run(&[
        "ethercat",
        "topology",
        "capture",
        output_path.to_str().expect("path"),
        "--observed",
        observed.to_str().expect("path"),
    ]);
    assert!(output.status.success(), "{}", text(&output));
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&output_path).expect("draft")).expect("JSON");
    assert_eq!(value["protocol"], "stc-ethercat-topology/1");
    assert_eq!(value["devices"][0]["name"], "slave0");
    assert_eq!(value["devices"][0]["vendor_id"], 2965);
    assert_eq!(value["devices"][0]["serial"], serde_json::Value::Null);

    let pinned = directory.join("pinned.json");
    let output = run(&[
        "ethercat",
        "topology",
        "capture",
        pinned.to_str().expect("path"),
        "--observed",
        observed.to_str().expect("path"),
        "--pin-serial",
        "--pin-alias",
    ]);
    assert!(output.status.success(), "{}", text(&output));
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&pinned).expect("draft")).expect("JSON");
    assert_eq!(value["devices"][0]["serial"], 11);
    assert_eq!(value["devices"][1]["alias"], 0);

    // Capture reuses project names and never rewrites the project draft.
    let topology = write(&directory, "topology.json", TOPOLOGY);
    let before = std::fs::read_to_string(&topology).expect("topology");
    let recaptured = directory.join("recaptured.json");
    let output = run(&[
        "ethercat",
        "topology",
        "capture",
        recaptured.to_str().expect("path"),
        "--topology",
        topology.to_str().expect("path"),
        "--observed",
        observed.to_str().expect("path"),
    ]);
    assert!(output.status.success(), "{}", text(&output));
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&recaptured).expect("draft")).expect("JSON");
    assert_eq!(value["devices"][0]["name"], "input");
    assert_eq!(
        std::fs::read_to_string(&topology).expect("topology"),
        before
    );
}

#[test]
fn slave_inspect_reports_position_facts() {
    let directory = workdir("slave");
    let topology = write(&directory, "topology.json", TOPOLOGY);
    let observed = write(&directory, "observed.json", OBSERVED);

    let output = run(&[
        "ethercat",
        "slave",
        "inspect",
        "0",
        "--topology",
        topology.to_str().expect("path"),
        "--observed",
        observed.to_str().expect("path"),
    ]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        text(&output).contains("configured: input"),
        "{}",
        text(&output)
    );
    assert!(text(&output).contains("result: match"), "{}", text(&output));

    let mismatched = write(
        &directory,
        "mismatched.json",
        &OBSERVED.replace(
            "\"position\": 0, \"vendor_id\": 2965, \"product_code\": 66048, \"revision\": 1",
            "\"position\": 0, \"vendor_id\": 2965, \"product_code\": 66048, \"revision\": 2",
        ),
    );
    let output = run(&[
        "ethercat",
        "slave",
        "inspect",
        "0",
        "--topology",
        topology.to_str().expect("path"),
        "--observed",
        mismatched.to_str().expect("path"),
        "--message-format",
        "json",
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", text(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(value["result"], "different");
    assert_eq!(value["observed"]["position"], 0);
    assert_eq!(value["diagnostics"][0]["code"], "STC-E-IO-ETH-003");

    let absent = write(
        &directory,
        "absent.json",
        "{\n  \"format\": 1,\n  \"protocol\": \"stc-ethercat-observed/1\",\n  \"master\": 0,\n  \"slaves\": []\n}\n",
    );
    let output = run(&[
        "ethercat",
        "slave",
        "inspect",
        "1",
        "--topology",
        topology.to_str().expect("path"),
        "--observed",
        absent.to_str().expect("path"),
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", text(&output));
    assert!(
        text(&output).contains("no slave responds"),
        "{}",
        text(&output)
    );
}

#[test]
fn setup_mode_is_recorded_and_cleared() {
    let directory = workdir("setup");
    let session = directory.join("session.json");

    let output = run(&[
        "ethercat",
        "setup",
        "status",
        "--session",
        session.to_str().expect("path"),
    ]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        text(&output).contains("setup: not active"),
        "{}",
        text(&output)
    );

    let output = run(&[
        "ethercat",
        "setup",
        "enter",
        "--session",
        session.to_str().expect("path"),
    ]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("setup: active"), "{}", text(&output));
    assert!(session.exists());

    let output = run(&[
        "ethercat",
        "setup",
        "enter",
        "--session",
        session.to_str().expect("path"),
        "--message-format",
        "json",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output).contains("STC-E-IO-ETH-044"),
        "{}",
        text(&output)
    );

    let output = run(&[
        "ethercat",
        "setup",
        "status",
        "--session",
        session.to_str().expect("path"),
        "--message-format",
        "json",
    ]);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(value["active"], true);
    assert!(
        value["session"]["session"]
            .as_str()
            .expect("session")
            .starts_with("setup-")
    );

    let output = run(&[
        "ethercat",
        "setup",
        "leave",
        "--session",
        session.to_str().expect("path"),
    ]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(!session.exists());

    std::fs::write(&session, "not a descriptor").expect("corrupt record");
    let output = run(&[
        "ethercat",
        "setup",
        "status",
        "--session",
        session.to_str().expect("path"),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output).contains("STC-E-IO-ETH-045"),
        "{}",
        text(&output)
    );
    let output = run(&[
        "ethercat",
        "setup",
        "leave",
        "--session",
        session.to_str().expect("path"),
    ]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("recovered"), "{}", text(&output));
    assert!(!session.exists());
}

#[test]
fn hardware_operations_fail_closed_without_the_abi_2_host() {
    let directory = workdir("live");
    let topology = write(&directory, "topology.json", TOPOLOGY);

    let output = run(&["ethercat", "topology", "scan"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output).contains("STC-E-IO-ETH-043"),
        "{}",
        text(&output)
    );
    assert!(text(&output).contains("identity"), "{}", text(&output));

    let output = run(&[
        "ethercat",
        "topology",
        "diff",
        "--topology",
        topology.to_str().expect("path"),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output).contains("STC-E-IO-ETH-043"),
        "{}",
        text(&output)
    );

    let output = run(&["ethercat", "slave", "inspect", "0"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output).contains("STC-E-IO-ETH-043"),
        "{}",
        text(&output)
    );
}

#[cfg(unix)]
#[test]
fn the_privileged_wrapper_rejects_every_unlisted_verb_and_argument() {
    // D-265: the wrapper is the security boundary, so its rejection surface is
    // pinned. Argument validation runs before any host path is touched, which
    // lets the test run on a CI host without the engineering installation.
    let wrapper = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/ethercat/stc-wl7-eng");
    let cases: &[(&[&str], &str)] = &[
        (&[], "usage"),
        (&["bogus"], "usage"),
        (&["setup"], "usage"),
        (&["setup", "bogus"], "usage"),
        (&["setup", "status", "extra"], "usage"),
        (&["topology"], "usage"),
        (&["topology", "bogus"], "usage"),
        (&["topology", "show", "extra"], "show takes no arguments"),
        (
            &["topology", "scan", "--pin-serial"],
            "scan takes no arguments",
        ),
        (
            &["topology", "capture", "--evil"],
            "unsupported capture argument",
        ),
        (&["slave", "bogus", "0"], "usage"),
        (&["slave", "inspect"], "usage"),
        (&["slave", "inspect", "abc"], "invalid position"),
        (&["slave", "inspect", "0", "extra"], "usage"),
    ];
    for (arguments, needle) in cases {
        let output = Command::new(&wrapper)
            .args(*arguments)
            .output()
            .expect("run the wrapper");
        assert_eq!(output.status.code(), Some(64), "{arguments:?}");
        let printed = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(printed.contains(needle), "{arguments:?}: {printed}");
        assert!(
            !printed.contains("launcher not found"),
            "{arguments:?} must be rejected before the host check: {printed}"
        );
    }
}

#[test]
fn usage_errors_exit_2_for_the_new_actions() {
    for args in [
        vec!["ethercat", "topology", "capture"],
        vec!["ethercat", "slave", "inspect"],
        vec!["ethercat", "setup"],
        vec!["ethercat", "topology", "diff"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
}
