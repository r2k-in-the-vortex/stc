//! `stc remote status` integration test (WL-1.2 Phase 4).
//!
//! A fake SSH/SFTP pair executes the remote commands on the local filesystem,
//! so a real `stc serve` and a real remote candidate build run under the same
//! process model the launcher uses. The test checks the no-drift report and an
//! online-compatible source change.

#![cfg(unix)]

mod common;

use std::fs::File;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const BUILD_TIMEOUT: Duration = Duration::from_secs(600);
const STOP_TIMEOUT: Duration = Duration::from_secs(60);

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
                    eprintln!("note: clang not found; skipping remote status checks");
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
            "stc-remote-status-{}-{name}-{unique}",
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

fn script(directory: &Path, name: &str, body: &str) {
    std::fs::create_dir_all(directory).expect("script directory");
    let path = directory.join(name);
    std::fs::write(&path, body).expect("write script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

fn fake_tools(root: &Path) -> PathBuf {
    let bin = root.join("bin");
    script(
        &bin,
        "ssh",
        "#!/bin/sh\nlast=\nfor arg in \"$@\"; do last=\"$arg\"; done\nexec /bin/sh -c \"$last\"\n",
    );
    script(
        &bin,
        "sftp",
        "#!/bin/sh\nwhile read -r cmd first second; do\n\
         \x20 case \"$cmd\" in\n\
         \x20   -mkdir) p=${first#\\\"}; p=${p%\\\"}; mkdir -p -- \"$p\" ;;\n\
         \x20   put) s=${first#\\\"}; s=${s%\\\"}; d=${second#\\\"}; d=${d%\\\"}; cp -- \"$s\" \"$d\" ;;\n\
         \x20 esac\ndone\n",
    );
    bin
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

fn run_status(work: &Path, manifest: &Path, path_env: &str) -> Value {
    let output = Command::new(stc())
        .args([
            "remote",
            "status",
            "--remote",
            "r2k@ecat",
            "--remote-dir",
            work.to_str().unwrap(),
            "--remote-stc",
            stc(),
            "--manifest-path",
            manifest.to_str().unwrap(),
            "--message-format",
            "json",
        ])
        .env("PATH", path_env)
        .output()
        .expect("run remote status");
    assert!(
        output.status.success(),
        "remote status failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("status json")
}

#[test]
fn remote_status_detects_drift_and_online_compatibility() {
    let Some(_clang) = clang() else {
        return;
    };
    let project = TestDir::new("project");
    let work = project.path.join("work");
    let manifest = project.write("work/hello/stc.toml", MANIFEST);
    project.write("work/hello/src/main.st", SOURCE);
    std::fs::create_dir_all(&work).expect("work");
    let session = work.join("stc-remote-session.json");
    let stdout = File::create(project.path.join("serve.stdout")).expect("stdout");
    let stderr = File::create(project.path.join("serve.stderr")).expect("stderr");
    let runtime = common::runtime_static();
    let child = Command::new(stc())
        .args([
            "serve",
            "--work-dir",
            work.to_str().unwrap(),
            "--manifest-path",
            manifest.to_str().unwrap(),
            "--runtime",
            runtime.to_str().unwrap(),
            "--cycles",
            "0",
            "--cycle-time",
            "10ms",
            "--debug-endpoint",
            work.join("stc-debug.endpoint").to_str().unwrap(),
            "--control-endpoint",
            work.join("stc-control.sock").to_str().unwrap(),
            "--session",
            session.to_str().unwrap(),
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

    let bin = fake_tools(&project.path);
    let path_env = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let matched = run_status(&work, &manifest, &path_env);
    assert_eq!(matched["drift"], false, "{matched}");
    assert_eq!(matched["online"], true, "{matched}");
    assert_eq!(matched["cold_required"], false, "{matched}");

    project.write(
        "work/hello/src/main.st",
        &SOURCE.replace(
            "  ticks := ticks + 1;",
            "  // remote status drift test\n  ticks := ticks + 1;",
        ),
    );
    let drifted = run_status(&work, &manifest, &path_env);
    assert_eq!(drifted["drift"], true, "{drifted}");
    assert_eq!(drifted["online"], true, "{drifted}");
    let local = drifted["local_fingerprint"].as_str().expect("local");
    let remote = drifted["remote_fingerprint"].as_str().expect("remote");
    assert_ne!(local, remote, "{drifted}");

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
        .expect("stop serve");
    assert!(
        stop.status.success(),
        "stop failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&stop.stdout),
        String::from_utf8_lossy(&stop.stderr)
    );
    let deadline = Instant::now() + STOP_TIMEOUT;
    while serve.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
}
