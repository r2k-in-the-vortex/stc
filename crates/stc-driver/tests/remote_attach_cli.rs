//! `stc debug --attach --remote` integration test (WL-1.2).
//!
//! Fake SSH/SFTP tools execute the remote commands locally; the fake SSH
//! `-L` mode runs a small Python TCP forwarder so the driver's real tunnel
//! code connects to a real local `stc serve`. The test covers an online
//! deploy and a cold download/restart through the attach path.

#![cfg(unix)]

mod common;

use std::fs::File;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

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

const TUNNEL_PY: &str = r#"
import select
import socket
import sys
import threading

parts = sys.argv[1].split(':')
local_port = int(parts[1])
remote_host = parts[2]
remote_port = int(parts[3])
listener = socket.socket()
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(('127.0.0.1', local_port))
listener.listen(8)

def handle(client, server):
    try:
        while True:
            readable, _, _ = select.select([client, server], [], [], 5)
            for source in readable:
                data = source.recv(65536)
                if not data:
                    return
                (server if source is client else client).sendall(data)
    except OSError:
        return
    finally:
        client.close()
        server.close()

while True:
    client, _ = listener.accept()
    try:
        server = socket.create_connection((remote_host, remote_port), timeout=5)
    except OSError:
        client.close()
        continue
    threading.Thread(target=handle, args=(client, server), daemon=True).start()
"#;

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
                    eprintln!("note: clang not found; skipping remote attach checks");
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
            "stc-remote-attach-{}-{name}-{unique}",
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

struct ServeGuard {
    child: Option<Child>,
    work: PathBuf,
}

impl Drop for ServeGuard {
    fn drop(&mut self) {
        let _ = Command::new(stc())
            .args([
                "serve",
                "stop",
                "--work-dir",
                self.work.to_str().unwrap(),
                "--message-format",
                "json",
            ])
            .output();
        if let Some(child) = self.child.as_mut()
            && child.try_wait().ok().flatten().is_none()
        {
            let _ = child.kill();
            let _ = child.wait();
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
    let tunnel = bin.join("tunnel.py");
    script(&bin, "tunnel.py", TUNNEL_PY);
    script(
        &bin,
        "ssh",
        &format!(
            "#!/bin/sh\nforward=\nprev=\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"-L\" ]; then forward=$arg; fi\n  prev=$arg\ndone\nif [ -n \"$forward\" ]; then exec python3 {} \"$forward\"; fi\nlast=\nfor arg in \"$@\"; do last=\"$arg\"; done\nexec /bin/sh -c \"$last\"\n",
            tunnel.display()
        ),
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

fn attach(
    bin: &Path,
    root: &Path,
    project: &Path,
    work: &Path,
    deploy: &str,
    batch: &Path,
) -> (std::process::ExitStatus, String, String) {
    let path_env = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let stdout_path = root.join(format!("attach-{deploy}.stdout"));
    let stderr_path = root.join(format!("attach-{deploy}.stderr"));
    let stdout = File::create(&stdout_path).expect("attach stdout");
    let stderr = File::create(&stderr_path).expect("attach stderr");
    let status = Command::new(stc())
        .args([
            "debug",
            "--attach",
            "dummy",
            "--remote",
            "r2k@ecat",
            "--remote-dir",
            work.to_str().unwrap(),
            "--remote-stc",
            stc(),
            "--remote-deploy",
            deploy,
            "--break",
            "src/main.st:9",
            "--batch",
            batch.to_str().unwrap(),
            "--message-format",
            "json",
        ])
        .current_dir(project)
        .env("PATH", path_env)
        .env("RUST_BACKTRACE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .status()
        .expect("run remote attach");
    (
        status,
        std::fs::read_to_string(&stdout_path).unwrap_or_default(),
        std::fs::read_to_string(&stderr_path).unwrap_or_default(),
    )
}

#[test]
fn remote_attach_online_and_cold_deploy() {
    let Some(_clang) = clang() else {
        return;
    };
    let root = TestDir::new("attach");
    let work = root.path.join("work");
    let manifest = root.write("work/hello/stc.toml", MANIFEST);
    root.write("work/hello/src/main.st", SOURCE);
    std::fs::create_dir_all(&work).expect("work");
    let session = work.join("stc-remote-session.json");
    let stdout = File::create(root.path.join("serve.stdout")).expect("stdout");
    let stderr = File::create(root.path.join("serve.stderr")).expect("stderr");
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
        .current_dir(root.path.join("work/hello"))
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("spawn stc serve");
    let mut guard = ServeGuard {
        child: Some(child),
        work: work.clone(),
    };
    let child = guard.child.as_mut().expect("serve child");
    assert!(
        wait_for_file(child, &session, BUILD_TIMEOUT),
        "serve did not publish a session descriptor"
    );

    let bin = fake_tools(&root.path);
    let project = root.path.join("work/hello");
    let batch = root.write(
        "batch.jsonl",
        "{\"op\":\"breakpoint.list\",\"params\":{}}\n\
         {\"op\":\"session.stop\",\"params\":{}}\n",
    );

    root.write(
        "work/hello/src/main.st",
        &format!("{SOURCE}\n(* online attach drift *)\n"),
    );
    let online = attach(&bin, &root.path, &project, &work, "online", &batch);
    assert!(
        online.0.success(),
        "online attach failed\nstdout:\n{}\nstderr:\n{}",
        online.1,
        online.2
    );
    assert!(
        online.2.contains("attached remotely"),
        "stderr: {}",
        online.2
    );
    assert!(online.1.contains("\"bound\":true"), "stdout: {}", online.1);

    root.write(
        "work/hello/src/main.st",
        &format!("{SOURCE}\n(* cold attach drift *)\n(* second change *)\n"),
    );
    let cold = attach(&bin, &root.path, &project, &work, "cold", &batch);
    let cold_log = std::fs::read_to_string(work.join("stc-serve.log")).unwrap_or_default();
    assert!(
        cold.0.success(),
        "cold attach failed ({:?})\nstdout:\n{}\nstderr:\n{}\nserve log:\n{cold_log}",
        cold.0,
        cold.1,
        cold.2
    );
    assert!(session.is_file(), "the cold-restarted serve stays running");

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
    assert!(stop.status.success(), "final stop failed");
    let deadline = Instant::now() + STOP_TIMEOUT;
    while guard
        .child
        .as_mut()
        .is_some_and(|child| child.try_wait().ok().flatten().is_none())
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(20));
    }
}
