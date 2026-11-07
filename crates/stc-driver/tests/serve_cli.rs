//! `stc serve` integration tests (WL-1.2 Phase 2).
//!
//! The test builds a temporary project, starts the persistent serve host,
//! validates the session descriptor, queries status, and stops it through the
//! control channel.

#![cfg(unix)]

mod common;

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const BUILD_TIMEOUT: Duration = Duration::from_secs(600);
const STOP_TIMEOUT: Duration = Duration::from_secs(60);

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
                    eprintln!("note: clang not found; skipping serve integration checks");
                    None
                }
            }
        })
        .as_ref()
        .copied()
}

fn runtime_artifact() -> PathBuf {
    common::runtime_static()
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stc-serve-cli-{}-{name}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test directory");
        Self { path }
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = relative
            .split('/')
            .fold(self.path.clone(), |path, part| path.join(part));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create directory");
        }
        std::fs::write(&path, contents).expect("write file");
        path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

const MANIFEST: &str = "\
schema = 1

[package]
name = \"demo\"
version = \"0.1.0\"
language = \"stc-2013-oop-utf8-v1\"

[project]
sources = [\"src/**/*.st\"]
";

const SOURCE: &str = "\
VAR_GLOBAL
  total : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  total := total + ticks;
END_PROGRAM
";

struct ServeChild {
    child: Child,
}

impl Drop for ServeChild {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn wait_for_file(child: &mut Child, path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while !path.is_file() {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return false;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

fn wait_exit(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn serve_status_and_stop_round_trip() {
    let Some(_clang) = clang() else {
        return;
    };
    let project = TestDir::new("round-trip");
    project.write("stc.toml", MANIFEST);
    project.write("src/main.st", SOURCE);
    let work = project.path.clone();
    let session = work.join("stc-remote-session.json");
    let nonce = work.join("stc-control.nonce");
    let debug_endpoint = work.join("stc-debug.endpoint");
    let control_endpoint = work.join("stc-control.sock");
    let stdout = File::create(project.path.join("serve.stdout")).expect("stdout file");
    let stderr = File::create(project.path.join("serve.stderr")).expect("stderr file");
    let runtime = runtime_artifact();
    let child = Command::new(stc())
        .args([
            "serve",
            "--work-dir",
            work.to_str().unwrap(),
            "--manifest-path",
            project.path.join("stc.toml").to_str().unwrap(),
            "--runtime",
            runtime.to_str().unwrap(),
            "--cycles",
            "0",
            "--cycle-time",
            "10ms",
            "--debug-endpoint",
            debug_endpoint.to_str().unwrap(),
            "--control-endpoint",
            control_endpoint.to_str().unwrap(),
            "--session",
            session.to_str().unwrap(),
            "--start-paused",
            "--message-format",
            "json",
        ])
        .current_dir(&project.path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("spawn stc serve");
    let mut serve = ServeChild { child };
    let text = |name: &str| {
        std::fs::read_to_string(project.path.join(name)).unwrap_or_else(|_| String::new())
    };
    assert!(
        wait_for_file(&mut serve.child, &session, BUILD_TIMEOUT),
        "serve did not publish a session descriptor\nstdout:\n{}\nstderr:\n{}",
        text("serve.stdout"),
        text("serve.stderr")
    );
    let descriptor: Value =
        serde_json::from_str(&std::fs::read_to_string(&session).expect("session")).expect("json");
    assert_eq!(descriptor["format"], 1, "{descriptor}");
    assert_eq!(descriptor["protocol"], "stc-remote/1", "{descriptor}");
    assert_eq!(descriptor["mode"], "debug", "{descriptor}");
    assert!(
        descriptor["pid"].as_u64().is_some_and(|pid| pid > 0),
        "{descriptor}"
    );
    assert_eq!(
        descriptor["source_fingerprint"].as_str().map(str::len),
        Some(66),
        "{descriptor}"
    );
    assert!(
        Path::new(descriptor["module"].as_str().expect("module")).is_file(),
        "{descriptor}"
    );
    assert!(
        Path::new(
            descriptor["debug_endpoint_file"]
                .as_str()
                .expect("endpoint")
        )
        .is_file(),
        "{descriptor}"
    );
    assert!(
        Path::new(descriptor["control_nonce_file"].as_str().expect("nonce")).is_file(),
        "{descriptor}"
    );
    let gate = descriptor["start_gate"].as_str().expect("start gate");
    assert!(
        !gate.is_empty(),
        "the start gate is published: {descriptor}"
    );

    let release = Command::new(stc())
        .args(["serve", "release", "--work-dir", work.to_str().unwrap()])
        .output()
        .expect("run release");
    assert!(
        release.status.success(),
        "release failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&release.stdout),
        String::from_utf8_lossy(&release.stderr)
    );

    let status = Command::new(stc())
        .args([
            "serve",
            "status",
            "--work-dir",
            work.to_str().unwrap(),
            "--message-format",
            "json",
        ])
        .output()
        .expect("run status");
    assert!(
        status.status.success(),
        "status failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );
    let status: Value = serde_json::from_slice(&status.stdout).expect("status json");
    assert_eq!(status["session"]["protocol"], "stc-remote/1", "{status}");
    assert_eq!(status["control"]["ok"], true, "{status}");
    assert!(
        status["active"]["source_fingerprint"].as_str().is_some(),
        "{status}"
    );

    let stop = Command::new(stc())
        .args([
            "serve",
            "stop",
            "--work-dir",
            work.to_str().unwrap(),
            "--message-format",
            "json",
        ])
        .output()
        .expect("run stop");
    assert!(
        stop.status.success(),
        "stop failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&stop.stdout),
        String::from_utf8_lossy(&stop.stderr)
    );
    let exit = wait_exit(&mut serve.child, STOP_TIMEOUT);
    assert!(exit.is_some(), "serve did not exit after stop");
    assert_eq!(exit.and_then(|status| status.code()), Some(0));
    assert!(!session.exists(), "the session descriptor is removed");
    assert!(!nonce.exists(), "the nonce file is removed");
}
