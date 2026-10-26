//! `stc run`: build the launcher and drive the host run loop.
//!
//! `stc run` forces `exe,module,metadata` artifacts, resolves `[run]` defaults
//! and CLI overrides, starts the launcher with inherited stdio, and returns the
//! launcher's exit code (CLI-and-Manifest §1.3, Runtime §1.1, D-154). `--watch`
//! remains the phase-8 workflow.

use std::path::{Path, PathBuf};
use std::process::Command;

use stc_diag::Severity;
use stc_manifest::Manifest;

use crate::build::RunRequest;
use crate::report::{Log, Reporter};

/// Launches a built project executable with the resolved run options.
pub(crate) fn launch(
    exe: &Path,
    request: &RunRequest,
    manifest: &Manifest,
    root_dir: &Path,
    reporter: &mut Reporter,
    log: Log,
) -> u8 {
    let mut args = Vec::new();
    let cycles = request.cycles.unwrap_or(manifest.run.cycles);
    args.push("--cycles".to_owned());
    args.push(cycles.to_string());
    if let Some(ns) = request
        .cycle_time_ns
        .or_else(|| manifest.run.cycle_time.map(|value| value.as_nanos()))
    {
        args.push("--cycle-time".to_owned());
        args.push(format!("{ns}ns"));
    }
    let io = request
        .io
        .clone()
        .unwrap_or_else(|| manifest.io.backend.as_str().to_owned());
    args.push("--io".to_owned());
    args.push(io);
    if let Some(path) = request
        .io_file
        .as_deref()
        .map(absolute)
        .or_else(|| manifest.io.file.as_deref().map(|file| root_dir.join(file)))
    {
        args.push("--io-file".to_owned());
        args.push(path.to_string_lossy().into_owned());
    }
    if let Some(path) = request.state_file.as_deref().map(absolute) {
        args.push("--state-file".to_owned());
        args.push(path.to_string_lossy().into_owned());
    }
    if request.reset_retain {
        args.push("--reset-retain".to_owned());
    }
    if request.no_retain {
        args.push("--no-retain".to_owned());
    }
    if log.verbose() {
        args.push("--status".to_owned());
    }

    log.info(format!(
        "run {}{}",
        exe.display(),
        if args.is_empty() {
            String::new()
        } else {
            format!(" {}", args.join(" "))
        }
    ));
    match Command::new(exe).args(&args).status() {
        Ok(status) => match status.code() {
            Some(code) => u8::try_from(code).unwrap_or_else(|_| signal_exit_code(code)),
            None => 130,
        },
        Err(error) => {
            reporter.plain(
                "STC-E-BUILD-002",
                Severity::Error,
                &format!("cannot start launcher `{}`: {error}", exe.display()),
            );
            1
        }
    }
}

/// Maps a Windows console-interrupt termination status to exit code 130.
fn signal_exit_code(code: i32) -> u8 {
    if cfg!(windows) && code == 0xC000_013A_u32 as i32 {
        130
    } else {
        1
    }
}

/// Resolves a CLI path against the current directory (D-163).
fn absolute(path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}
