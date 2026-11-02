//! P0.6: `stc hir-dump` emits, writes, and verifies the canonical core-HIR
//! dump with the documented exit-code contract.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_stc")
}

fn directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!("stc-hir-dump-{}", std::process::id()));
    std::fs::create_dir_all(&path).expect("create temp directory");
    path
}

struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn new(name: &str, contents: &[u8]) -> Self {
        let path = directory().join(format!("{name}.st"));
        std::fs::write(&path, contents).expect("write temp file");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .output()
        .expect("run the stc binary")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).expect("stdout is JSON")
}

const VALID: &str = "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := 1;
END_PROGRAM
";
const REJECTED: &str = "PROGRAM P
VAR
  r : REAL;
END_VAR
  r := 1.0;
END_PROGRAM
";

#[test]
fn emit_writes_an_accepted_dump_to_stdout() {
    let file = TempFile::new("emit", VALID.as_bytes());
    let output = run(&["hir-dump", file.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stderr(&output).is_empty(), "{}", stderr(&output));
    let dump = json(&stdout(&output));
    assert_eq!(dump["format"], "stc-core-hir");
    assert_eq!(dump["formatVersion"], "0.1.0");
    assert_eq!(dump["semanticsVersion"], "1.0-draft");
    assert_eq!(dump["profile"], "stc-2013-oop-utf8-v1");
    assert_eq!(dump["status"], "ok");
    assert_eq!(dump["features"], Value::Array(Vec::new()));
    assert_eq!(dump["unit"]["program"]["name"], "P");
    assert!(stdout(&output).ends_with('\n'));
}

#[test]
fn emit_can_write_the_dump_to_a_file() {
    let file = TempFile::new("emit_file", VALID.as_bytes());
    let dump_path = directory().join("emit_file.json");
    let _ = std::fs::remove_file(&dump_path);
    let output = run(&[
        "hir-dump",
        file.path().to_str().unwrap(),
        "--output",
        dump_path.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).is_empty(), "{}", stdout(&output));
    let text = std::fs::read_to_string(&dump_path).expect("the dump was written");
    assert_eq!(json(&text)["status"], "ok");

    let checked = run(&["hir-dump", "--verify", dump_path.to_str().unwrap()]);
    assert_eq!(checked.status.code(), Some(0), "{}", stderr(&checked));
    let summary = stdout(&checked);
    assert!(summary.contains("format stc-core-hir 0.1.0"), "{summary}");
    assert!(summary.contains("semantics 1.0-draft"), "{summary}");
    assert!(summary.contains("status ok"), "{summary}");
    let _ = std::fs::remove_file(&dump_path);
}

#[test]
fn verify_emits_a_json_summary() {
    let file = TempFile::new("verify_json", VALID.as_bytes());
    let dump_path = directory().join("verify_json.json");
    let output = run(&[
        "hir-dump",
        file.path().to_str().unwrap(),
        "--output",
        dump_path.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));

    let checked = run(&[
        "hir-dump",
        "--verify",
        dump_path.to_str().unwrap(),
        "--message-format=json",
    ]);
    assert_eq!(checked.status.code(), Some(0), "{}", stderr(&checked));
    let summary = json(&stdout(&checked));
    assert_eq!(summary["status"], "ok");
    assert_eq!(summary["formatVersion"], "0.1.0");
    assert_eq!(summary["features"], Value::Array(Vec::new()));
    let _ = std::fs::remove_file(&dump_path);
}

#[test]
fn rejected_sources_still_emit_a_valid_rejected_dump() {
    let file = TempFile::new("rejected", REJECTED.as_bytes());
    let output = run(&["hir-dump", file.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("core HIR rejected"),
        "{}",
        stderr(&output)
    );
    assert!(stderr(&output).contains("type.real"), "{}", stderr(&output));
    let dump = json(&stdout(&output));
    assert_eq!(dump["status"], "rejected");
    assert!(!dump["features"].as_array().expect("features").is_empty());

    let dump_path = directory().join("rejected.json");
    std::fs::write(&dump_path, stdout(&output)).expect("write rejected dump");
    let checked = run(&["hir-dump", "--verify", dump_path.to_str().unwrap()]);
    assert_eq!(checked.status.code(), Some(1));
    assert!(stdout(&checked).contains("status rejected"));
    let _ = std::fs::remove_file(&dump_path);
}

#[test]
fn verify_rejects_unknown_versions_and_malformed_input() {
    let file = TempFile::new("version", VALID.as_bytes());
    let output = run(&["hir-dump", file.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let text = stdout(&output);

    let wrong = directory().join("wrong_version.json");
    std::fs::write(
        &wrong,
        text.replace("\"formatVersion\":\"0.1.0\"", "\"formatVersion\":\"9.9.9\""),
    )
    .expect("write mutated dump");
    let checked = run(&["hir-dump", "--verify", wrong.to_str().unwrap()]);
    assert_eq!(checked.status.code(), Some(2));
    assert!(
        stderr(&checked).contains("format version"),
        "{}",
        stderr(&checked)
    );
    let _ = std::fs::remove_file(&wrong);

    let malformed = directory().join("malformed.json");
    std::fs::write(&malformed, "{}").expect("write malformed dump");
    let checked = run(&["hir-dump", "--verify", malformed.to_str().unwrap()]);
    assert_eq!(checked.status.code(), Some(2));
    assert!(stderr(&checked).contains("format"), "{}", stderr(&checked));
    let _ = std::fs::remove_file(&malformed);

    let missing = directory().join("missing.json");
    let _ = std::fs::remove_file(&missing);
    let checked = run(&["hir-dump", "--verify", missing.to_str().unwrap()]);
    assert_eq!(checked.status.code(), Some(2));
    assert!(
        stderr(&checked).contains("cannot read"),
        "{}",
        stderr(&checked)
    );
}

#[test]
fn hir_dump_cli_contract_is_enforced() {
    let file = TempFile::new("cli", VALID.as_bytes());
    let path = file.path().to_str().unwrap();
    let dump_path = directory().join("cli.json");
    let dump = dump_path.to_str().unwrap();

    let conflict = run(&["hir-dump", path, "--output", dump, "--verify", dump]);
    assert_eq!(conflict.status.code(), Some(2));
    assert!(
        stderr(&conflict).contains("mutually exclusive"),
        "{}",
        stderr(&conflict)
    );

    let positional = run(&["hir-dump", "--verify", dump, path]);
    assert_eq!(positional.status.code(), Some(2));
    assert!(
        stderr(&positional).contains("does not accept input files"),
        "{}",
        stderr(&positional)
    );

    let empty = run(&["hir-dump"]);
    assert_eq!(empty.status.code(), Some(2));
    assert!(
        stderr(&empty).contains("needs at least one input file"),
        "{}",
        stderr(&empty)
    );

    let wrong_command = run(&["check", path, "--output", dump]);
    assert_eq!(wrong_command.status.code(), Some(2));
    assert!(
        stderr(&wrong_command).contains("only valid with"),
        "{}",
        stderr(&wrong_command)
    );

    let help = run(&["--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(stdout(&help).contains("hir-dump"), "{}", stdout(&help));
}

#[test]
fn got_keys_match_the_got_crate() {
    let file = TempFile::new(
        "got",
        b"VAR_GLOBAL
  g : DINT;
END_VAR
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := g;
END_PROGRAM
",
    );
    let output = run(&["hir-dump", file.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let dump = json(&stdout(&output));
    let globals = dump["unit"]["globals"].as_array().expect("globals");
    let key = globals[0]["gotKey"].as_str().expect("got key");
    assert_eq!(key, stc_got::global_root_key("_root", "G").unwrap());

    let namespaced = TempFile::new(
        "got_ns",
        b"NAMESPACE Lib
FUNCTION F : DINT
VAR_INPUT
  a : DINT;
END_VAR
  F := a;
END_FUNCTION
END_NAMESPACE
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := Lib.F(1);
END_PROGRAM
",
    );
    let output = run(&["hir-dump", namespaced.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let dump = json(&stdout(&output));
    let functions = dump["unit"]["functions"].as_array().expect("functions");
    let name = functions[0]["name"].as_str().expect("function name");
    assert_eq!(name, stc_got::normalize_name_path("Lib.F").unwrap());
}
