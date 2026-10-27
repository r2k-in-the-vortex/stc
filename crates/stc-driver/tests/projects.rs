//! End-to-end project workflow (M6 phase 10): the `projects` CI suite runs
//! `stc new/check/build/run/test/fmt/clean`, the file I/O backend, and the
//! active-baseline store path through the real binary and exit codes.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

fn stc() -> &'static str {
    env!("CARGO_BIN_EXE_stc")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
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
                    eprintln!("note: clang not found; skipping project execution tests");
                    None
                }
            }
        })
        .as_ref()
        .copied()
}

fn runtime_artifact() -> PathBuf {
    static CACHE: OnceLock<PathBuf> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
            let output = Command::new(cargo)
                .args(["build", "-p", "stc-runtime", "--locked"])
                .current_dir(workspace_root())
                .output()
                .expect("run cargo build -p stc-runtime");
            assert!(
                output.status.success(),
                "failed to build stc-runtime:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let profile = std::env::current_exe()
                .expect("test executable")
                .parent()
                .and_then(Path::parent)
                .expect("target profile directory")
                .to_path_buf();
            let name = if cfg!(windows) {
                "stc_runtime.lib"
            } else {
                "libstc_runtime.a"
            };
            profile.join(name)
        })
        .clone()
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stc-projects-{}-{name}-{unique}",
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

    fn run(&self, directory: &str, args: &[&str]) -> Output {
        Command::new(stc())
            .args(args)
            .arg("--color=never")
            .current_dir(self.path(directory))
            .output()
            .expect("run stc")
    }

    fn run_with_runtime(&self, directory: &str, args: &[&str]) -> Output {
        let runtime = runtime_artifact();
        let mut all: Vec<&str> = args.to_vec();
        all.push("--runtime");
        let runtime = runtime.to_string_lossy().into_owned();
        all.push(&runtime);
        self.run(directory, &all)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_success(output: &Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} failed ({})\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout(output),
        stderr(output)
    );
}

fn app_manifest(name: &str, extra: &str) -> String {
    format!(
        "schema = 1\n\
         \n\
         [package]\n\
         name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         language = \"stc-2013-oop-utf8-v1\"\n\
         \n\
         [project]\n\
         sources = [\"src/**/*.st\"]\n\
         \n\
         [build]\n\
         emit = [\"ir\"]\n\
         {extra}"
    )
}

const PROGRAM: &str = "\
PROGRAM Main
VAR
  counter : DINT;
END_VAR
  counter := counter + 1;
END_PROGRAM
";

#[test]
fn new_project_runs_the_v1_workflow() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("lifecycle");
    assert_success(&directory.run(".", &["new", "app"]), "stc new");
    for file in ["stc.toml", "src/main.st", "tests/smoke.st", ".gitignore"] {
        assert!(directory.path(&format!("app/{file}")).is_file(), "{file}");
    }

    assert_success(&directory.run("app", &["check"]), "stc check");
    assert!(directory.path("app/stc.lock").is_file());

    assert_success(
        &directory.run_with_runtime("app", &["build", "--emit", "ir,module"]),
        "stc build",
    );

    let run = directory.run_with_runtime("app", &["run", "--cycles", "3"]);
    assert_success(&run, "stc run");

    let test = directory.run_with_runtime("app", &["test"]);
    assert_success(&test, "stc test");

    assert_success(
        &directory.run("app", &["fmt", "--check"]),
        "stc fmt --check",
    );

    assert_success(&directory.run("app", &["clean"]), "stc clean");
    assert!(
        !directory.path("app/target/stc/debug").exists(),
        "clean removes the configured out-dir"
    );
    assert!(
        directory.path("app/stc.lock").is_file(),
        "clean preserves stc.lock"
    );
}

#[test]
fn file_backend_output_failure_is_reported() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("io-output");
    directory.write("app/stc.toml", &app_manifest("app", ""));
    directory.write("app/src/main.st", PROGRAM);
    let blocked = directory.path("app/io-dir");
    std::fs::create_dir_all(&blocked).expect("create blocked path");

    // The output image cannot replace a directory: STC-E-RUN-IO-001 keeps
    // the previous file (here, the directory) intact.
    let output = directory.run_with_runtime(
        "app",
        &[
            "run",
            "--cycles",
            "1",
            "--io",
            "file",
            "--io-file",
            blocked.to_str().unwrap(),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout:\n{}\nstderr:\n{}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("STC-E-RUN-IO-001"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_invalid_runtime_store_builds_a_cold_candidate_with_a_warning() {
    let directory = TestDir::new("store");
    directory.write(
        "app/stc.toml",
        &app_manifest(
            "app",
            "\n[runtime]\nmanifest-dir = \"store\"\n\n[online]\nbaseline = \"active\"\n",
        ),
    );
    directory.write("app/src/main.st", PROGRAM);
    directory.write("app/store/current", "not-a-build-id\n");

    // An unreadable active store is STC-W-ONLINE-007 and selects a cold
    // layout; the build still succeeds.
    let output = directory.run("app", &["build"]);
    assert_success(&output, "stc build");
    assert!(
        stderr(&output).contains("STC-W-ONLINE-007"),
        "{}",
        stderr(&output)
    );
    assert!(
        directory.path("app/target/stc/debug/app.ll").is_file(),
        "the cold candidate was emitted"
    );
}
