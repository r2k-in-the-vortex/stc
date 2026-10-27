//! Shared helpers for the driver integration tests.
//!
//! The runtime host libraries are linked into C harnesses, which do not link
//! LLVM's profiling runtime. They are therefore built uninstrumented in a
//! separate target directory, even when the test binary itself runs under
//! `cargo llvm-cov`. Building into a separate directory also keeps the
//! coverage cache clean and makes the harness independent of the default
//! target directory.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// Builds `stc-runtime` once and returns the static and shared library paths.
fn runtime_libraries() -> &'static (PathBuf, PathBuf) {
    static CACHE: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();
    CACHE.get_or_init(|| {
        let profile = std::env::current_exe()
            .expect("test executable")
            .parent()
            .and_then(Path::parent)
            .expect("target profile directory")
            .to_path_buf();
        let profile_name = profile.file_name().expect("target profile directory name");
        let runtime_target = profile
            .parent()
            .expect("target directory")
            .join("test-runtime");
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut command = Command::new(cargo);
        command
            .args(["build", "-p", "stc-runtime", "--locked"])
            .arg("--target-dir")
            .arg(&runtime_target);
        match profile_name.to_str().unwrap_or("debug") {
            "debug" => {}
            "release" => {
                command.arg("--release");
            }
            other => {
                command.arg("--profile").arg(other);
            }
        }
        let output = command
            // Strip cargo-llvm-cov's rustc wrapper (and any inherited
            // rustflags): the harness links this library without the
            // profiling runtime.
            .env_remove("RUSTC_WRAPPER")
            .env_remove("RUSTC_WORKSPACE_WRAPPER")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .current_dir(workspace_root())
            .output()
            .expect("run cargo build -p stc-runtime");
        assert!(
            output.status.success(),
            "failed to build stc-runtime:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let directory = runtime_target.join(profile_name);
        let static_name = if cfg!(windows) {
            "stc_runtime.lib"
        } else {
            "libstc_runtime.a"
        };
        let shared_name = if cfg!(windows) {
            "stc_runtime.dll"
        } else if cfg!(target_os = "macos") {
            "libstc_runtime.dylib"
        } else {
            "libstc_runtime.so"
        };
        (directory.join(static_name), directory.join(shared_name))
    })
}

/// Returns the static runtime library, building it on first use.
pub fn runtime_static() -> PathBuf {
    let (static_library, _) = runtime_libraries();
    assert!(
        static_library.is_file(),
        "runtime artifact missing at {}",
        static_library.display()
    );
    static_library.clone()
}

/// Returns the static and shared runtime libraries, building them on first
/// use. Returns `None` when either artifact is missing.
pub fn runtime_pair() -> Option<(PathBuf, PathBuf)> {
    let (static_library, shared_library) = runtime_libraries();
    if static_library.is_file() && shared_library.is_file() {
        Some((static_library.clone(), shared_library.clone()))
    } else {
        None
    }
}
