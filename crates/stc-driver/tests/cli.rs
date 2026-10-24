use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_stc")
}

struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn new(name: &str, contents: &[u8]) -> Self {
        let directory = std::env::temp_dir().join(format!("stc-cli-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("create temp directory");
        let path = directory.join(format!("{name}.st"));
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

const VALID: &str = "PROGRAM P\n  x := 1;\nEND_PROGRAM\n";
const INVALID: &str = "PROGRAM P\n  x := ;\nEND_PROGRAM\n";
const RELAXED: &str = "PROGRAM P\nIF TRUE THEN x := 1; END_IF\nEND_PROGRAM\n";

#[test]
fn parse_prints_a_cst_and_succeeds() {
    let file = TempFile::new("parse_ok", VALID.as_bytes());
    let output = run(&["parse", file.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    let text = stdout(&output);
    assert!(text.contains("SourceFile"));
    assert!(text.contains("ProgramDecl"));
    assert!(text.contains("AssignStmt"));
    assert!(stderr(&output).is_empty(), "{}", stderr(&output));
}

#[test]
fn parse_ast_adds_the_typed_dump() {
    let file = TempFile::new("parse_ast", VALID.as_bytes());
    let output = run(&["parse", file.path().to_str().unwrap(), "--ast"]);
    assert_eq!(output.status.code(), Some(0));
    let text = stdout(&output);
    assert!(text.contains("# ast"), "{text}");
    assert!(text.contains("ProgramDecl@"), "{text}");
}

#[test]
fn check_reports_errors_on_stderr_and_exits_one() {
    let file = TempFile::new("check_bad", INVALID.as_bytes());
    let output = run(&["check", file.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).is_empty());
    assert!(
        stderr(&output).contains("STC-E-GRAM-105"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn check_json_emits_one_object_per_line_on_stdout() {
    let file = TempFile::new("check_json", INVALID.as_bytes());
    let output = run(&[
        "check",
        file.path().to_str().unwrap(),
        "--message-format=json",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 1, "{text}");
    assert!(
        lines[0].starts_with('{') && lines[0].ends_with('}'),
        "{text}"
    );
    assert!(lines[0].contains("\"code\":\"STC-E-GRAM-105\""), "{text}");
    assert!(stderr(&output).is_empty(), "{}", stderr(&output));
}

#[test]
fn relaxed_mode_accepts_the_superset() {
    let file = TempFile::new("relaxed", RELAXED.as_bytes());
    let strict = run(&["check", file.path().to_str().unwrap()]);
    assert_eq!(strict.status.code(), Some(1));
    assert!(stderr(&strict).contains("STC-E-GRAM-107"));

    let relaxed = run(&["check", file.path().to_str().unwrap(), "--relaxed"]);
    assert_eq!(relaxed.status.code(), Some(0), "{}", stderr(&relaxed));
}

#[test]
fn invalid_utf8_is_reported_through_tokenize_bytes() {
    let file = TempFile::new("invalid_utf8", b"PROGRAM P\n  x := \xFF;\nEND_PROGRAM\n");
    let output = run(&["check", file.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("STC-E-LEX-001"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn missing_file_and_usage_errors_exit_two() {
    let output = run(&["check", "definitely-not-a-file.st"]);
    assert_eq!(output.status.code(), Some(2));

    let output = run(&[]);
    assert_eq!(output.status.code(), Some(2));

    let output = run(&["parse"]);
    assert_eq!(output.status.code(), Some(2));

    let output = run(&["frobnicate", "x.st"]);
    assert_eq!(output.status.code(), Some(2));

    let file = TempFile::new("usage", VALID.as_bytes());
    let output = run(&[
        "check",
        file.path().to_str().unwrap(),
        "--message-format=xml",
    ]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn help_and_version_succeed() {
    let help = run(&["--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(stdout(&help).contains("Usage:"));

    let version = run(&["--version"]);
    assert_eq!(version.status.code(), Some(0));
    assert!(stdout(&version).starts_with("stc "));
}

#[test]
fn ast_with_check_is_a_usage_error() {
    let file = TempFile::new("ast_check", VALID.as_bytes());
    let output = run(&["check", file.path().to_str().unwrap(), "--ast"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn check_does_not_print_a_dump() {
    let file = TempFile::new("check_quiet", VALID.as_bytes());
    let output = run(&["check", file.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).is_empty());
}

#[test]
fn check_reports_lexical_and_syntax_errors_together() {
    let file = TempFile::new(
        "check_both",
        b"PROGRAM P\n  x := \xFF;\n  y := ;\nEND_PROGRAM\n",
    );
    let output = run(&["check", file.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    let text = stderr(&output);
    assert!(text.contains("STC-E-LEX-001"), "{text}");
    assert!(text.contains("STC-E-GRAM-105"), "{text}");
}

#[test]
fn parse_json_suppresses_the_dump() {
    let file = TempFile::new("parse_json", VALID.as_bytes());
    let output = run(&[
        "parse",
        file.path().to_str().unwrap(),
        "--message-format",
        "json",
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).is_empty());
}

#[test]
fn parse_with_errors_still_prints_the_dump_and_exits_one() {
    let file = TempFile::new("parse_bad", INVALID.as_bytes());
    let output = run(&["parse", file.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("SourceFile"));
    assert!(stderr(&output).contains("STC-E-GRAM-105"));
}
