//! Example project smoke tests (M7 phase 4).
//!
//! Every committed example must check, build in both modes, test, run, and
//! format-check with the release-candidate compiler. The examples are copied
//! to a temporary tree so the repository stays clean and the committed
//! `stc.lock` is used with `--locked`.

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
                    eprintln!("note: clang not found; skipping example tests");
                    None
                }
            }
        })
        .as_ref()
        .copied()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stc-examples-{}-{name}-{unique}",
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
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create directory");
    for entry in std::fs::read_dir(from).expect("read directory") {
        let entry = entry.expect("directory entry");
        let name = entry.file_name();
        if matches!(name.to_str(), Some("target" | "state")) {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().expect("entry type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

fn output(command: &mut Command) -> Output {
    command.output().expect("run stc")
}

fn assert_success(output: &Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run(directory: &Path, manifest: &str, args: &[&str]) -> Output {
    output(
        Command::new(stc())
            .args(args)
            .arg("--color=never")
            .arg("--manifest-path")
            .arg(manifest)
            .arg("--locked")
            .current_dir(directory),
    )
}

#[test]
fn examples_work_end_to_end() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("all");
    let source = workspace_root().join("examples");
    copy_tree(&source, &directory.path("examples"));
    let runtime = common::runtime_static();
    let runtime = runtime.to_string_lossy().into_owned();

    // motorlib is the dependency package: it has no configuration of its own,
    // so checking it is the meaningful standalone smoke test.
    let motorlib = directory.path("examples/motorlib");
    let manifest = motorlib.join("stc.toml");
    let manifest = manifest.to_string_lossy().into_owned();
    assert_success(&run(&motorlib, &manifest, &["check"]), "motorlib check");

    for name in ["hello", "motor", "online-change"] {
        let example = directory.path(&format!("examples/{name}"));
        let manifest = example.join("stc.toml");
        let manifest = manifest.to_string_lossy().into_owned();
        assert_success(&run(&example, &manifest, &["check"]), name);
        assert_success(&run(&example, &manifest, &["build"]), name);
        assert_success(&run(&example, &manifest, &["build", "--release"]), name);
        assert_success(
            &run(
                &example,
                &manifest,
                &[
                    "run",
                    "--cycles",
                    "2",
                    "--runtime",
                    &runtime,
                    "--out-dir",
                    "target/run",
                ],
            ),
            name,
        );
        assert_success(
            &run(&example, &manifest, &["test", "--runtime", &runtime]),
            name,
        );
        assert_success(&run(&example, &manifest, &["fmt", "--check"]), name);
    }
}
