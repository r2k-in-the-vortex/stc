//! `stc fmt` integration tests (M6 phase 7): in-place formatting, `--check`,
//! manifest-free paths, project sources, encoding preservation, and the
//! `STC-E-FMT-001`/`STC-E-FMT-002` error paths.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn stc() -> &'static str {
    env!("CARGO_BIN_EXE_stc")
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("stc-fmt-{}-{name}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test directory");
        Self { path }
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        self.write_bytes(relative, contents.as_bytes())
    }

    fn write_bytes(&self, relative: &str, contents: &[u8]) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create directory");
        }
        std::fs::write(&path, contents).expect("write file");
        path
    }

    fn read(&self, relative: &str) -> String {
        String::from_utf8(std::fs::read(self.path.join(relative)).expect("read file"))
            .expect("utf-8 file")
    }

    fn read_bytes(&self, relative: &str) -> Vec<u8> {
        std::fs::read(self.path.join(relative)).expect("read file")
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(stc())
            .args(args)
            .current_dir(&self.path)
            .output()
            .expect("run stc")
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

const MESSY: &str = "\
program Main
var
x:dint;
end_var
x:=1+2;
end_program
";

const FORMATTED: &str = "\
PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := 1 + 2;
END_PROGRAM
";

#[test]
fn check_reports_changes_without_writing_and_format_is_idempotent() {
    let directory = TestDir::new("check");
    directory.write("messy.st", MESSY);

    let check = directory.run(&["fmt", "--check", "messy.st"]);
    assert_eq!(code(&check), 1, "stderr:\n{}", stderr(&check));
    assert!(stderr(&check).contains("would be reformatted"));
    assert_eq!(directory.read("messy.st"), MESSY, "--check wrote the file");

    let format = directory.run(&["fmt", "messy.st"]);
    assert_eq!(code(&format), 0, "stderr:\n{}", stderr(&format));
    assert_eq!(directory.read("messy.st"), FORMATTED);

    let again = directory.run(&["fmt", "messy.st"]);
    assert_eq!(code(&again), 0, "stderr:\n{}", stderr(&again));
    assert_eq!(directory.read("messy.st"), FORMATTED);

    let check = directory.run(&["fmt", "--check", "messy.st"]);
    assert_eq!(code(&check), 0, "stderr:\n{}", stderr(&check));
}

#[test]
fn project_format_run_includes_test_sources() {
    let directory = TestDir::new("project");
    directory.write(
        "stc.toml",
        "schema = 1\n\n\
         [package]\n\
         name = \"demo\"\n\
         version = \"0.1.0\"\n\
         language = \"stc-2013-oop-utf8-v1\"\n\n\
         [project]\n\
         sources = [\"src/**/*.st\"]\n\n\
         [test]\n\
         sources = [\"tests/**/*.st\"]\n",
    );
    directory.write("src/main.st", MESSY);
    directory.write(
        "tests/smoke.st",
        "{stc:test}\nfunction SmokeTest:BOOL\nSmokeTest:=TRUE;\nend_function\n",
    );

    let format = directory.run(&["fmt"]);
    assert_eq!(code(&format), 0, "stderr:\n{}", stderr(&format));
    assert_eq!(directory.read("src/main.st"), FORMATTED);
    assert_eq!(
        directory.read("tests/smoke.st"),
        "{stc:test}\nFUNCTION SmokeTest : BOOL\n  SmokeTest := TRUE;\nEND_FUNCTION\n"
    );
    assert!(
        !directory.path.join("stc.lock").exists(),
        "fmt wrote stc.lock"
    );

    let check = directory.run(&["fmt", "--check"]);
    assert_eq!(code(&check), 0, "stderr:\n{}", stderr(&check));
    assert!(
        !directory.path.join("stc.lock").exists(),
        "fmt --check wrote stc.lock"
    );
}

#[test]
fn new_scaffold_is_already_formatted() {
    let directory = TestDir::new("scaffold");
    let created = directory.run(&["new", "app"]);
    assert_eq!(code(&created), 0, "stderr:\n{}", stderr(&created));
    let check = Command::new(stc())
        .args(["fmt", "--check"])
        .current_dir(directory.path.join("app"))
        .output()
        .expect("run stc fmt --check");
    assert_eq!(code(&check), 0, "stderr:\n{}", stderr(&check));
}

#[test]
fn syntax_errors_prevent_formatting() {
    let directory = TestDir::new("syntax");
    let source = "PROGRAM p\n  x := 1;\n";
    directory.write("broken.st", source);

    let output = directory.run(&["fmt", "broken.st"]);
    assert_eq!(code(&output), 1, "stderr:\n{}", stderr(&output));
    assert!(
        stderr(&output).contains("STC-E-FMT-001"),
        "stderr:\n{}",
        stderr(&output)
    );
    assert_eq!(directory.read("broken.st"), source);
}

#[test]
fn unreadable_file_reports_fmt_002() {
    let directory = TestDir::new("missing");
    let output = directory.run(&["fmt", "does-not-exist.st"]);
    assert_eq!(code(&output), 2, "stderr:\n{}", stderr(&output));
    assert!(
        stderr(&output).contains("STC-E-FMT-002"),
        "stderr:\n{}",
        stderr(&output)
    );
}

#[test]
fn bom_and_crlf_are_preserved_when_rewriting() {
    let directory = TestDir::new("encoding");
    let source = "\u{feff}program p\r\nvar\r\nx:int;\r\nend_var\r\nx:=1;\r\nend_program\r\n";
    directory.write("crlf.st", source);

    let output = directory.run(&["fmt", "crlf.st"]);
    assert_eq!(code(&output), 0, "stderr:\n{}", stderr(&output));
    let written = directory.read_bytes("crlf.st");
    assert_eq!(
        written,
        "\u{feff}PROGRAM p\r\nVAR\r\n  x : INT;\r\nEND_VAR\r\n  x := 1;\r\nEND_PROGRAM\r\n"
            .as_bytes()
    );
    assert_eq!(written[..3], [0xEF, 0xBB, 0xBF], "BOM is preserved");
}
