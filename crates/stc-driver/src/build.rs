//! The `stc build` bridge: module, manifest, launcher, and host artifacts.
//!
//! The compiler emits one loadable ABI 2 module. `exe` is a thin launcher that
//! loads the adjacent module pair; `lib` is the embedding host library (the
//! runtime static archive) with the generated module kept separate. Both may be
//! emitted together (D-109–D-112, M4.5 §6.3).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use stc_driver::toolchain::system_link_args;

use stc_diag::DiagnosticSink;
use stc_got::{Manifest, Rename};
use stc_hir::HirCrate;

/// One requested artifact kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Emit {
    /// Textual LLVM IR.
    Ir,
    /// Object file.
    Obj,
    /// Loadable module and its manifest pair.
    Module,
    /// Thin host launcher.
    Exe,
    /// Embedding host library (runtime archive).
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
    /// `--got-manifest`.
    pub got_manifest: Option<String>,
    /// `--cold`.
    pub cold: bool,
    /// `--rename old=new`.
    pub renames: Vec<(String, String)>,
    /// `--allow-untracked-removal`.
    pub allow_untracked_removal: bool,
    /// `--relaxed` was accepted by the parser.
    pub relaxed: bool,
    /// `-v`: print exact toolchain commands.
    pub verbose: bool,
}

/// Runs code generation and artifact emission.
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

    let target_triple = request
        .target
        .clone()
        .unwrap_or_else(stc_codegen::host_triple);
    let sources = paths.to_vec();
    let mut source_bytes = Vec::new();
    for path in paths {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                return artifact_error(json, &format!("cannot read `{path}`: {error}"));
            }
        };
        source_bytes.push((path.clone(), bytes));
    }
    let fingerprint = match stc_codegen::module::source_fingerprint(&source_bytes) {
        Ok(fingerprint) => fingerprint,
        Err(error) => {
            return artifact_error(
                json,
                &format!("cannot fingerprint sources: {}", error.message),
            );
        }
    };
    let baseline = match &request.got_manifest {
        Some(path) => match read_baseline(Path::new(path)) {
            Ok(manifest) => Some(manifest),
            Err(message) => return artifact_error(json, &message),
        },
        None => None,
    };
    let renames = request
        .renames
        .iter()
        .map(|(old, new)| Rename::new(old.as_str(), new.as_str()))
        .collect();

    let mode = if emit.contains(&Emit::Lib) && !emit.contains(&Emit::Exe) {
        stc_codegen::Mode::Library
    } else {
        stc_codegen::Mode::Executable
    };
    let options = stc_codegen::Options {
        artifact_name: &name,
        target: request.target.as_deref(),
        debug: false,
        emit_metadata: emit.contains(&Emit::Metadata),
        mode,
        relaxed: request.relaxed,
        release: false,
        sources: &sources,
        module: stc_codegen::ModuleBuildOptions {
            compiler_version: format!("stc {}", env!("CARGO_PKG_VERSION")),
            source_fingerprint: fingerprint,
            baseline: baseline.as_ref(),
            cold: request.cold,
            allow_untracked_removal: request.allow_untracked_removal,
            renames,
        },
    };
    let mut output = match stc_codegen::generate(hir, &options) {
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
    for diagnostic in &output.got.diagnostics {
        if diagnostic.is_error() {
            report_no_span(json, diagnostic.code, &diagnostic.message);
            return 1;
        }
        report_warning(json, diagnostic.code, &diagnostic.message);
    }
    if !output.got.is_deployable() {
        report_no_span(
            json,
            "STC-E-MANIFEST-004",
            "the module artifacts are not deployable",
        );
        return 1;
    }
    let module_file = module_file_name(&name, &target_triple, output.got.manifest.header.build_id);

    let needs_ir = emit.iter().any(|kind| {
        matches!(
            kind,
            Emit::Ir | Emit::Obj | Emit::Module | Emit::Exe | Emit::Lib
        )
    });
    let ir_path = out_dir.join(format!("{name}.ll"));
    if needs_ir && let Err(error) = std::fs::write(&ir_path, &output.module) {
        return artifact_error(
            json,
            &format!("cannot write `{}`: {error}", ir_path.display()),
        );
    }

    let needs_metadata = emit
        .iter()
        .any(|kind| matches!(kind, Emit::Metadata | Emit::Module | Emit::Exe | Emit::Lib));
    if needs_metadata {
        output.metadata.module_file = module_file.clone();
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

    let needs_pair = emit
        .iter()
        .any(|kind| matches!(kind, Emit::Module | Emit::Exe | Emit::Lib));
    if needs_pair {
        let json_path = out_dir.join(format!("{name}.stc-got.json"));
        if let Err(error) = std::fs::write(&json_path, &output.got.manifest_json) {
            return artifact_error(
                json,
                &format!("cannot write `{}`: {error}", json_path.display()),
            );
        }
        let binary_path = out_dir.join(format!("{name}.stc-got.bin"));
        if let Err(error) = std::fs::write(&binary_path, &output.got.manifest_binary) {
            return artifact_error(
                json,
                &format!("cannot write `{}`: {error}", binary_path.display()),
            );
        }
    }

    let needs_clang = emit
        .iter()
        .any(|kind| matches!(kind, Emit::Obj | Emit::Module | Emit::Exe | Emit::Lib));
    if !needs_clang {
        return 0;
    }
    if emit.contains(&Emit::Exe)
        && let Some(target) = &request.target
        && !is_host_triple(target)
    {
        return usage_error(
            json,
            &format!("cross-target executables are not supported in M4.5 (`{target}`)"),
        );
    }
    if emit.contains(&Emit::Lib)
        && let Some(target) = &request.target
        && !is_host_triple(target)
    {
        return usage_error(
            json,
            &format!("cross-target embedding libraries are not supported in M4.5 (`{target}`)"),
        );
    }
    let clang = match find_clang(request.clang.as_deref()) {
        Ok(clang) => clang,
        Err(message) => return toolchain_error(json, "STC-E-BUILD-001", &message),
    };
    let object_path = out_dir.join(format!("{name}.o"));
    let pic = !target_triple.contains("windows");
    if let Err(message) = compile_ir(
        &clang,
        &ir_path,
        &object_path,
        request.target.as_deref(),
        pic,
        request.verbose,
    ) {
        return build_failure(json, &message, &ir_path);
    }
    if emit.contains(&Emit::Obj)
        && !needs_pair
        && !emit.contains(&Emit::Exe)
        && !emit.contains(&Emit::Lib)
    {
        return 0;
    }

    if needs_pair {
        let module_path = out_dir.join(&module_file);
        if let Err(message) = link_module(
            &clang,
            &object_path,
            &module_path,
            request.target.as_deref(),
            request.verbose,
        ) {
            return build_failure(json, &message, &ir_path);
        }
    }
    if emit.contains(&Emit::Lib) {
        let runtime = match find_runtime(request.runtime.as_deref()) {
            Ok(runtime) => runtime,
            Err(message) => return toolchain_error(json, "STC-E-BUILD-003", &message),
        };
        let library_path = out_dir.join(lib_name(&name));
        if let Err(error) = std::fs::copy(&runtime, &library_path) {
            return artifact_error(
                json,
                &format!(
                    "cannot write embedding host library `{}`: {error}",
                    library_path.display()
                ),
            );
        }
    }
    if emit.contains(&Emit::Exe) {
        let runtime = match find_runtime(request.runtime.as_deref()) {
            Ok(runtime) => runtime,
            Err(message) => return toolchain_error(json, "STC-E-BUILD-003", &message),
        };
        let launcher_ir =
            stc_codegen::generate_launcher(&name, &module_file, request.target.as_deref());
        let launcher_ir_path = out_dir.join(format!("{name}-launcher.ll"));
        if let Err(error) = std::fs::write(&launcher_ir_path, &launcher_ir) {
            return artifact_error(
                json,
                &format!("cannot write `{}`: {error}", launcher_ir_path.display()),
            );
        }
        let launcher_object = out_dir.join(format!("{name}-launcher.o"));
        if let Err(message) = compile_ir(
            &clang,
            &launcher_ir_path,
            &launcher_object,
            request.target.as_deref(),
            false,
            request.verbose,
        ) {
            return build_failure(json, &message, &launcher_ir_path);
        }
        let exe_path = out_dir.join(exe_name(&name));
        if let Err(message) = link_exe(
            &clang,
            &launcher_object,
            &runtime,
            &exe_path,
            request.target.as_deref(),
            request.verbose,
        ) {
            return build_failure(json, &message, &launcher_ir_path);
        }
    }
    0
}

fn read_baseline(path: &Path) -> Result<Manifest, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("cannot read baseline `{}`: {error}", path.display()))?;
    let manifest = if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        let text = String::from_utf8(bytes)
            .map_err(|_| format!("baseline `{}` is not UTF-8", path.display()))?;
        Manifest::decode_json(&text)
    } else {
        Manifest::decode_binary(&bytes)
    }
    .map_err(|error| {
        format!(
            "baseline `{}` is not a valid GOT manifest: {}",
            path.display(),
            error.message
        )
    })?;
    manifest.validate().map_err(|error| {
        format!(
            "baseline `{}` is not a valid GOT manifest: {}",
            path.display(),
            error.message
        )
    })?;
    Ok(manifest)
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

fn module_file_name(name: &str, target: &str, build_id: stc_got::GotId) -> String {
    let extension = if target.contains("windows") {
        "dll"
    } else {
        "so"
    };
    format!("{name}-{}.{extension}", build_id.to_hex_plain())
}

fn is_host_triple(target: &str) -> bool {
    target == stc_codegen::host_triple()
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
    pic: bool,
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
        .arg(object);
    if pic {
        command.arg("-fPIC");
    }
    command.arg("-O0");
    run_tool(&mut command, verbose)
}

fn link_module(
    clang: &str,
    object: &Path,
    output: &Path,
    target: Option<&str>,
    verbose: bool,
) -> Result<(), String> {
    let mut command = Command::new(clang);
    if let Some(target) = target {
        command.arg(format!("--target={target}"));
    }
    command.arg(object).arg("-shared").arg("-o").arg(output);
    let target_triple = target.map_or_else(stc_codegen::host_triple, str::to_owned);
    if target.is_none() || is_host_triple(&target_triple) {
        for argument in system_link_args() {
            command.arg(argument);
        }
    }
    if target_triple.contains("windows") {
        command.args(["-Xlinker", "/NOIMPLIB"]);
    }
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

fn report_warning(json: bool, code: &'static str, message: &str) {
    if json {
        println!(
            "{{\"code\":\"{code}\",\"severity\":\"warning\",\"message\":\"{}\"}}",
            escape_json(message)
        );
    } else {
        eprintln!("warning[{code}]: {message}");
    }
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
