//! The M4 `stc build` bridge (D-079).
//!
//! This module owns artifact naming, clang/runtime discovery, clang
//! invocation, and toolchain diagnostics. It keeps the M6 manifest rules out
//! of the driver until the manifest exists.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use stc_driver::toolchain::system_link_args;

use stc_diag::DiagnosticSink;
use stc_hir::HirCrate;

/// One requested artifact kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Emit {
    /// Textual LLVM IR.
    Ir,
    /// Object file.
    Obj,
    /// Executable.
    Exe,
    /// Static library.
    Lib,
    /// Build metadata JSON.
    Metadata,
}

/// Parsed `stc build` options.
#[derive(Clone, Debug, Default)]
pub(crate) struct Request {
    /// `--name` or the default from the first input.
    pub name: Option<String>,
    /// Requested artifacts.
    pub emit: BTreeSet<Emit>,
    /// `--out-dir`.
    pub out_dir: Option<String>,
    /// `--target`.
    pub target: Option<String>,
    /// `--clang`.
    pub clang: Option<String>,
    /// `--runtime`.
    pub runtime: Option<String>,
    /// `-v`: print exact toolchain commands.
    pub verbose: bool,
}

/// Runs code generation and artifact emission.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    request: &Request,
    hir: &HirCrate,
    paths: &[String],
    diagnostics: &mut DiagnosticSink,
    json: bool,
) -> u8 {
    let name = match request.name.clone() {
        Some(name) => name,
        None => default_name(paths),
    };
    if name.is_empty() || name.contains(['/', '\\']) {
        return usage_error(json, "artifact name must be a non-empty file name");
    }
    let emit = &request.emit;
    if emit.contains(&Emit::Exe) && emit.contains(&Emit::Lib) {
        return usage_error(
            json,
            "`exe` and `lib` cannot be emitted from the same module in M4",
        );
    }
    let out_dir = PathBuf::from(
        request
            .out_dir
            .clone()
            .unwrap_or_else(|| "target/stc/debug".into()),
    );
    if let Err(error) = std::fs::create_dir_all(&out_dir) {
        return artifact_error(
            json,
            &format!(
                "cannot create output directory `{}`: {error}",
                out_dir.display()
            ),
        );
    }

    let mode = if emit.contains(&Emit::Lib) {
        stc_codegen::Mode::Library
    } else {
        stc_codegen::Mode::Executable
    };
    let sources: Vec<String> = paths.to_vec();
    let options = stc_codegen::Options {
        artifact_name: &name,
        target: request.target.as_deref(),
        debug: false,
        emit_metadata: emit.contains(&Emit::Metadata),
        mode,
        sources: &sources,
    };
    let output = match stc_codegen::generate(hir, &options) {
        Ok(output) => output,
        Err(errors) => {
            let mut internal = false;
            for error in errors {
                match error {
                    stc_codegen::CodegenError::Unsupported {
                        code,
                        message,
                        span,
                    } if span.start == 0 && span.end == 0 => {
                        report_no_span(json, code, &message);
                    }
                    stc_codegen::CodegenError::Unsupported {
                        code,
                        message,
                        span,
                    } => diagnostics.error(code, message, span),
                    stc_codegen::CodegenError::Internal { message } => {
                        report_no_span(json, stc_codegen::codes::CODEGEN_INTERNAL, &message);
                        internal = true;
                    }
                }
            }
            return if internal { 3 } else { 1 };
        }
    };

    let ir_path = out_dir.join(format!("{name}.ll"));
    let needs_ir = emit
        .iter()
        .any(|kind| matches!(kind, Emit::Ir | Emit::Obj | Emit::Exe | Emit::Lib));
    if needs_ir && let Err(error) = std::fs::write(&ir_path, &output.module) {
        return artifact_error(
            json,
            &format!("cannot write `{}`: {error}", ir_path.display()),
        );
    }
    let needs_metadata =
        emit.contains(&Emit::Metadata) || emit.contains(&Emit::Exe) || emit.contains(&Emit::Lib);
    if needs_metadata {
        let metadata_path = out_dir.join(format!("{name}.stc-meta.json"));
        if let Err(error) =
            std::fs::write(&metadata_path, output.metadata.to_json().render_pretty())
        {
            return artifact_error(
                json,
                &format!("cannot write `{}`: {error}", metadata_path.display()),
            );
        }
    }

    let needs_clang = emit
        .iter()
        .any(|kind| matches!(kind, Emit::Obj | Emit::Exe | Emit::Lib));
    if !needs_clang {
        return 0;
    }
    let clang = match find_clang(request.clang.as_deref()) {
        Ok(clang) => clang,
        Err(message) => return toolchain_error(json, "STC-E-BUILD-001", &message),
    };
    if emit.contains(&Emit::Exe)
        && let Some(target) = &request.target
        && !is_host_triple(target)
    {
        return usage_error(
            json,
            &format!("cross-target executables are not supported in M4 (`{target}`)"),
        );
    }

    let object_path = out_dir.join(format!("{name}.o"));
    if let Err(message) = compile_ir(
        &clang,
        &ir_path,
        &object_path,
        request.target.as_deref(),
        request.verbose,
    ) {
        return build_failure(json, &message, &ir_path);
    }
    if emit.contains(&Emit::Obj) {
        return 0;
    }
    let runtime = match find_runtime(request.runtime.as_deref()) {
        Ok(runtime) => runtime,
        Err(message) => return toolchain_error(json, "STC-E-BUILD-003", &message),
    };
    if emit.contains(&Emit::Exe) {
        let exe_path = out_dir.join(exe_name(&name));
        if let Err(message) = link_exe(
            &clang,
            &object_path,
            &runtime,
            &exe_path,
            request.target.as_deref(),
            request.verbose,
        ) {
            return build_failure(json, &message, &ir_path);
        }
        return 0;
    }
    if emit.contains(&Emit::Lib) {
        let lib_path = out_dir.join(lib_name(&name));
        if let Err(message) = archive(&object_path, &lib_path, request.verbose) {
            return build_failure(json, &message, &ir_path);
        }
        return 0;
    }
    0
}

fn default_name(paths: &[String]) -> String {
    let stem = paths
        .first()
        .and_then(|path| Path::new(path).file_stem())
        .map_or_else(
            || "stc_out".to_string(),
            |stem| stem.to_string_lossy().into_owned(),
        );
    let mut sanitized = String::new();
    for character in stem.chars() {
        if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
            sanitized.push(character);
        } else {
            sanitized.push('_');
        }
    }
    if sanitized.is_empty() {
        "stc_out".into()
    } else {
        sanitized
    }
}

fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    }
}

fn lib_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.lib")
    } else {
        format!("{name}.a")
    }
}

fn is_host_triple(target: &str) -> bool {
    let host = if cfg!(all(windows, target_env = "msvc")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else {
        ""
    };
    target == host
}

fn find_clang(explicit: Option<&str>) -> Result<String, String> {
    let candidate = explicit
        .map(str::to_owned)
        .or_else(|| std::env::var("STC_CLANG").ok())
        .unwrap_or_else(|| "clang".into());
    match Command::new(&candidate).arg("--version").output() {
        Ok(output) if output.status.success() => Ok(candidate),
        Ok(_) => Err(format!("`{candidate} --version` failed")),
        Err(error) => Err(format!("cannot run clang `{candidate}`: {error}")),
    }
}

fn find_runtime(explicit: Option<&str>) -> Result<PathBuf, String> {
    let file_name = if cfg!(windows) {
        "stc_runtime.lib"
    } else {
        "libstc_runtime.a"
    };
    if let Some(required) = explicit.map(PathBuf::from) {
        if required.is_file() {
            return Ok(required);
        }
        return Err(format!(
            "runtime static library `{}` was not found",
            required.display()
        ));
    }
    if let Ok(value) = std::env::var("STC_RUNTIME") {
        let required = PathBuf::from(value);
        if required.is_file() {
            return Ok(required);
        }
        return Err(format!(
            "runtime static library `{}` was not found",
            required.display()
        ));
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(directory) = exe.parent()
    {
        candidates.push(directory.join(file_name));
    }
    if let Ok(current) = std::env::current_dir() {
        candidates.push(current.join("target/debug").join(file_name));
        candidates.push(current.join("target/release").join(file_name));
    }
    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "runtime static library `{file_name}` was not found; build it with \
         `cargo build -p stc-runtime` or pass `--runtime <path>`"
    ))
}

fn compile_ir(
    clang: &str,
    ir: &Path,
    object: &Path,
    target: Option<&str>,
    verbose: bool,
) -> Result<(), String> {
    let mut command = Command::new(clang);
    if let Some(target) = target {
        command.arg(format!("--target={target}"));
    }
    command
        .args(["-x", "ir"])
        .arg(ir)
        .args(["-c", "-o"])
        .arg(object)
        .arg("-O0");
    run_tool(&mut command, verbose)
}

fn link_exe(
    clang: &str,
    object: &Path,
    runtime: &Path,
    output: &Path,
    target: Option<&str>,
    verbose: bool,
) -> Result<(), String> {
    let mut command = Command::new(clang);
    if let Some(target) = target {
        command.arg(format!("--target={target}"));
    }
    command.arg(object).arg(runtime);
    for argument in system_link_args() {
        command.arg(argument);
    }
    command.args(["-o"]).arg(output);
    run_tool(&mut command, verbose)
}

fn archive(object: &Path, library: &Path, verbose: bool) -> Result<(), String> {
    let candidates: &[&str] = if cfg!(windows) {
        &["llvm-ar"]
    } else {
        &["llvm-ar", "ar"]
    };
    let mut last_error = String::new();
    for candidate in candidates {
        let mut command = Command::new(candidate);
        command.arg("rcs").arg(library).arg(object);
        match run_tool(&mut command, verbose) {
            Ok(()) => return Ok(()),
            Err(message) => {
                let missing = message.starts_with("cannot run");
                last_error = message;
                if missing {
                    continue;
                }
                break;
            }
        }
    }
    Err(last_error)
}

fn run_tool(command: &mut Command, verbose: bool) -> Result<(), String> {
    if verbose {
        let mut rendered = command.get_program().to_string_lossy().into_owned();
        for argument in command.get_args() {
            rendered.push(' ');
            rendered.push_str(&argument.to_string_lossy());
        }
        eprintln!("+ {rendered}");
    }
    match command.output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            Err(format!(
                "`{}` failed with status {}:\n{}",
                command.get_program().to_string_lossy(),
                output.status,
                stderr.trim_end()
            ))
        }
        Err(error) => Err(format!(
            "cannot run `{}`: {error}",
            command.get_program().to_string_lossy()
        )),
    }
}

fn usage_error(json: bool, message: &str) -> u8 {
    report_no_span(json, "STC-E-BUILD-004", message);
    2
}

fn toolchain_error(json: bool, code: &'static str, message: &str) -> u8 {
    report_no_span(json, code, message);
    2
}

fn artifact_error(json: bool, message: &str) -> u8 {
    report_no_span(json, "STC-E-BUILD-002", message);
    1
}

fn build_failure(json: bool, message: &str, ir_path: &Path) -> u8 {
    report_no_span(
        json,
        "STC-E-BUILD-002",
        &format!(
            "{message}\n  = note: the generated IR is kept at `{}`",
            ir_path.display()
        ),
    );
    1
}

fn report_no_span(json: bool, code: &'static str, message: &str) {
    if json {
        println!(
            "{{\"code\":\"{code}\",\"severity\":\"error\",\"message\":\"{}\"}}",
            escape_json(message)
        );
    } else {
        eprintln!("error[{code}]: {message}");
    }
}

fn escape_json(text: &str) -> String {
    let mut out = String::new();
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04X}", character as u32));
            }
            character => out.push(character),
        }
    }
    out
}
