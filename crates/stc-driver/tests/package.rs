//! Release package layout tests (M7 phase 1).
//!
//! A release package keeps `stc` in `bin/` and the runtime archives in the
//! sibling `lib/` directory (D-189). The driver must find the runtime there
//! without `--runtime` or `STC_RUNTIME`, and explicit overrides must still
//! take precedence.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

fn stc() -> &'static str {
    env!("CARGO_BIN_EXE_stc")
}

fn clang() -> Option<&'static str> {
    static CLANG: OnceLock<Option<&'static str>> = OnceLock::new();
    CLANG
        .get_or_init(|| {
            let candidate = std::env::var("STC_CLANG").unwrap_or_else(|_| "clang".into());
            match Command::new(&candidate).arg("--version").output() {
                Ok(output) if output.status.success() => {
                    Some(Box::leak(candidate.into_boxed_str()))
                }
                _ => {
                    if std::env::var_os("STC_REQUIRE_CLANG").is_some() {
                        panic!("clang is required (STC_REQUIRE_CLANG=1) but was not found");
                    }
                    eprintln!("note: clang not found; skipping package layout tests");
                    None
                }
            }
        })
        .as_ref()
        .copied()
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stc-package-{}-{name}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test directory");
        Self { path }
    }

    fn path(&self, relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(self.path.clone(), |path, part| path.join(part))
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create directory");
        }
        std::fs::write(path, contents).expect("write file");
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn driver_name() -> &'static str {
    if cfg!(windows) { "stc.exe" } else { "stc" }
}

fn runtime_name() -> &'static str {
    if cfg!(windows) {
        "stc_runtime.lib"
    } else {
        "libstc_runtime.a"
    }
}

/// Copies the test-built driver and runtime into the release `bin`/`lib`
/// layout and returns the packaged driver path.
fn package_layout(directory: &TestDir) -> PathBuf {
    let bin = directory.path("package/bin");
    let lib = directory.path("package/lib");
    std::fs::create_dir_all(&bin).expect("create bin");
    std::fs::create_dir_all(&lib).expect("create lib");
    let driver = bin.join(driver_name());
    std::fs::copy(stc(), &driver).expect("copy driver");
    let runtime = common::runtime_static();
    std::fs::copy(&runtime, lib.join(runtime_name())).expect("copy runtime");
    driver
}

const SOURCE: &str = r#"
PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := x + 1;
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

fn run(driver: &Path, directory: &TestDir, args: &[&str]) -> Output {
    Command::new(driver)
        .args(args)
        .arg("--color=never")
        .current_dir(&directory.path)
        .output()
        .expect("run packaged stc")
}

#[test]
fn packaged_driver_reports_its_version() {
    let directory = TestDir::new("version");
    let driver = package_layout(&directory);
    let output = run(&driver, &directory, &["--version"]);
    assert!(
        output.status.success(),
        "packaged driver failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("stc {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn adjacent_lib_directory_supplies_the_runtime() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("adjacent");
    let driver = package_layout(&directory);
    directory.write("main.st", SOURCE);
    let output = run(
        &driver,
        &directory,
        &[
            "build",
            "main.st",
            "--emit=exe",
            "--name",
            "demo",
            "--out-dir",
            "out",
        ],
    );
    assert!(
        output.status.success(),
        "build without --runtime failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let executable = if cfg!(windows) {
        "out/demo.exe"
    } else {
        "out/demo"
    };
    assert!(
        directory.path(executable).is_file(),
        "packaged layout did not produce {executable}"
    );
}

#[test]
fn explicit_runtime_path_keeps_precedence() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("precedence");
    let driver = package_layout(&directory);
    directory.write("main.st", SOURCE);
    let missing = directory.path("missing/stc_runtime.lib");
    let output = run(
        &driver,
        &directory,
        &[
            "build",
            "main.st",
            "--emit=exe",
            "--name",
            "demo",
            "--out-dir",
            "out",
            "--runtime",
            &missing.to_string_lossy(),
        ],
    );
    assert!(!output.status.success(), "missing --runtime was ignored");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("was not found"),
        "unexpected diagnostic:\n{stderr}"
    );
}
