//! The `stc build` bridge: module, manifest, launcher, and host artifacts.
//!
//! The compiler emits one loadable ABI 2 module. `exe` is a thin launcher that
//! loads the adjacent module pair; `lib` is the embedding host library (static
//! archive or shared host library) with the generated module kept separate.
//! Both may be emitted together (D-109–D-112, M4.5 §6.3).
//!
//! Project builds resolve defaults from `stc.toml`; the manifest-free
//! `<file>...` bridge keeps the M4.5 defaults. Toolchain discovery follows
//! CLI > `STC_*` environment > manifest > built-in default
//! (CLI-and-Manifest §1.2).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use stc_codegen::Json;
use stc_diag::{DiagnosticSink, Severity};
use stc_driver::toolchain::system_link_args;
use stc_got::{Digest32, Manifest, PlanAction, PlanVerdict, Rename};
use stc_hir::HirCrate;
use stc_manifest::{EmitKind, LibType, OptLevel};

use crate::report::{Log, Reporter, display_path, elapsed};

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
    /// Embedding host library (runtime archive or shared library).
    Lib,
    /// Build metadata JSON.
    Metadata,
}

impl From<EmitKind> for Emit {
    fn from(kind: EmitKind) -> Self {
        match kind {
            EmitKind::Ir => Self::Ir,
            EmitKind::Obj => Self::Obj,
            EmitKind::Module => Self::Module,
            EmitKind::Exe => Self::Exe,
            EmitKind::Lib => Self::Lib,
            EmitKind::Metadata => Self::Metadata,
        }
    }
}

/// Parsed `stc run` options and overrides.
#[derive(Clone, Debug, Default)]
pub(crate) struct RunRequest {
    /// `--cycles`; `None` uses `[run].cycles`.
    pub cycles: Option<u64>,
    /// `--cycle-time`; `None` uses `[run].cycle-time`.
    pub cycle_time_ns: Option<u64>,
    /// `--io`.
    pub io: Option<String>,
    /// `--io-file`.
    pub io_file: Option<String>,
    /// `--state-file`.
    pub state_file: Option<String>,
    /// `--reset-retain`.
    pub reset_retain: bool,
    /// `--no-retain`.
    pub no_retain: bool,
}

/// Parsed project build settings and CLI overrides.
#[derive(Clone, Debug)]
pub(crate) struct Request {
    /// `--name` or the default from the first input or `[package].name`.
    pub name: Option<String>,
    /// Requested artifacts; empty selects the manifest/default.
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
    /// `--relaxed` or `[build].syntax = "relaxed"`.
    pub relaxed: bool,
    /// `--debug` was accepted; M6 always builds the unoptimised debug form.
    pub debug: bool,
    /// `[build].opt-level`.
    pub opt_level: OptLevel,
    /// `[build].lib-type`.
    pub lib_type: LibType,
    /// `[build].target`.
    pub manifest_target: Option<String>,
    /// `[build].clang`.
    pub manifest_clang: Option<String>,
    /// `[build].runtime`.
    pub manifest_runtime: Option<String>,
    /// `stc run` options.
    pub run: RunRequest,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            name: None,
            emit: BTreeSet::new(),
            out_dir: None,
            target: None,
            clang: None,
            runtime: None,
            got_manifest: None,
            cold: false,
            renames: Vec::new(),
            allow_untracked_removal: false,
            relaxed: false,
            debug: false,
            opt_level: OptLevel::O0,
            lib_type: LibType::Static,
            manifest_target: None,
            manifest_clang: None,
            manifest_runtime: None,
            run: RunRequest::default(),
        }
    }
}

/// A generated module projection ready for artifact emission.
pub(crate) struct Generated {
    /// Resolved output directory.
    pub out_dir: PathBuf,
    /// Resolved target triple.
    pub target_triple: String,
    /// Loadable module file name.
    pub module_file: String,
    /// Code generation output.
    pub output: stc_codegen::Output,
}

/// Runs code generation and artifact emission for the manifest-free bridge.
pub(crate) fn run(
    request: &Request,
    hir: &HirCrate,
    paths: &[String],
    diagnostics: &mut DiagnosticSink,
    reporter: &mut Reporter,
    log: Log,
) -> u8 {
    let name = match request.name.clone() {
        Some(name) => name,
        None => default_name(paths),
    };
    if name.is_empty() || name.contains(['/', '\\']) {
        return usage_error(reporter, "artifact name must be a non-empty file name");
    }
    let sources = paths.to_vec();
    let mut source_bytes = Vec::new();
    for path in paths {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                return artifact_error(reporter, &format!("cannot read `{path}`: {error}"));
            }
        };
        source_bytes.push((path.clone(), bytes));
    }
    let fingerprint = match stc_codegen::module::source_fingerprint(&source_bytes) {
        Ok(fingerprint) => fingerprint,
        Err(error) => {
            return artifact_error(
                reporter,
                &format!("cannot fingerprint sources: {}", error.message),
            );
        }
    };
    finish(
        request,
        hir,
        name,
        &sources,
        None,
        fingerprint,
        diagnostics,
        reporter,
        log,
    )
}

/// Runs code generation and artifact emission for a project build.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_project(
    request: &Request,
    hir: &HirCrate,
    name: &str,
    sources: &[String],
    source_paths: &[String],
    fingerprint: Digest32,
    diagnostics: &mut DiagnosticSink,
    reporter: &mut Reporter,
    log: Log,
) -> u8 {
    if name.is_empty() || name.contains(['/', '\\']) {
        return usage_error(reporter, "artifact name must be a non-empty file name");
    }
    finish(
        request,
        hir,
        name.to_owned(),
        sources,
        Some(source_paths),
        fingerprint,
        diagnostics,
        reporter,
        log,
    )
}

/// Generates the module projection for `metadata` without writing artifacts.
#[allow(clippy::too_many_arguments)]
pub(crate) fn generate_project(
    request: &Request,
    hir: &HirCrate,
    name: &str,
    sources: &[String],
    source_paths: &[String],
    fingerprint: Digest32,
    diagnostics: &mut DiagnosticSink,
    reporter: &mut Reporter,
    log: Log,
) -> Result<Generated, u8> {
    if name.is_empty() || name.contains(['/', '\\']) {
        return Err(usage_error(
            reporter,
            "artifact name must be a non-empty file name",
        ));
    }
    generate(
        request,
        hir,
        name,
        sources,
        Some(source_paths),
        fingerprint,
        diagnostics,
        reporter,
        log,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish(
    request: &Request,
    hir: &HirCrate,
    name: String,
    sources: &[String],
    source_paths: Option<&[String]>,
    fingerprint: Digest32,
    diagnostics: &mut DiagnosticSink,
    reporter: &mut Reporter,
    log: Log,
) -> u8 {
    let generated = match generate(
        request,
        hir,
        &name,
        sources,
        source_paths,
        fingerprint,
        diagnostics,
        reporter,
        log,
    ) {
        Ok(generated) => generated,
        Err(code) => return code,
    };
    emit(request, &name, &generated, reporter, log)
}

#[allow(clippy::too_many_arguments)]
fn generate(
    request: &Request,
    hir: &HirCrate,
    name: &str,
    sources: &[String],
    source_paths: Option<&[String]>,
    fingerprint: Digest32,
    diagnostics: &mut DiagnosticSink,
    reporter: &mut Reporter,
    log: Log,
) -> Result<Generated, u8> {
    let out_dir = PathBuf::from(
        request
            .out_dir
            .clone()
            .unwrap_or_else(|| "target/stc/debug".into()),
    );
    let target = effective_target(request);
    let target_triple = target.clone().unwrap_or_else(stc_codegen::host_triple);
    let baseline = match &request.got_manifest {
        Some(path) => match read_baseline(Path::new(path)) {
            Ok(manifest) => Some(manifest),
            Err(message) => return Err(artifact_error(reporter, &message)),
        },
        None => None,
    };
    let renames = request
        .renames
        .iter()
        .map(|(old, new)| Rename::new(old.as_str(), new.as_str()))
        .collect();

    let mode = if request.emit.contains(&Emit::Lib) && !request.emit.contains(&Emit::Exe) {
        stc_codegen::Mode::Library
    } else {
        stc_codegen::Mode::Executable
    };
    let options = stc_codegen::Options {
        artifact_name: name,
        // Only an explicitly configured target emits a `target triple` line;
        // the resolved host triple still drives layouts and file names.
        target: target.as_deref(),
        debug: request.debug,
        emit_metadata: request.emit.contains(&Emit::Metadata),
        mode,
        relaxed: request.relaxed,
        release: false,
        sources,
        source_paths,
        module: stc_codegen::ModuleBuildOptions {
            compiler_version: format!("stc {}", env!("CARGO_PKG_VERSION")),
            source_fingerprint: fingerprint,
            baseline: baseline.as_ref(),
            cold: request.cold,
            allow_untracked_removal: request.allow_untracked_removal,
            renames,
        },
    };
    let codegen_start = Instant::now();
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
                        reporter.plain(code, Severity::Error, &message);
                    }
                    stc_codegen::CodegenError::Unsupported {
                        code,
                        message,
                        span,
                    } => diagnostics.error(code, message, span),
                    stc_codegen::CodegenError::Internal { message } => {
                        reporter.plain(
                            stc_codegen::codes::CODEGEN_INTERNAL,
                            Severity::Error,
                            &message,
                        );
                        internal = true;
                    }
                }
            }
            return Err(if internal { 3 } else { 1 });
        }
    };
    log.info(format!("codegen {}", elapsed(codegen_start)));

    let mut plan_error = false;
    for diagnostic in &output.got.diagnostics {
        let severity = if diagnostic.is_error() {
            plan_error = true;
            Severity::Error
        } else {
            Severity::Warning
        };
        reporter.plain(diagnostic.code, severity, &diagnostic.message);
    }
    if plan_error {
        return Err(1);
    }
    if !output.got.is_deployable() {
        reporter.plain(
            "STC-E-MANIFEST-004",
            Severity::Error,
            "the module artifacts are not deployable",
        );
        return Err(1);
    }
    if let Some(baseline) = &baseline
        && !request.cold
    {
        let report = CompatibilityReport::new(&output.got.manifest, baseline);
        report.print(reporter, log);
    }
    let module_file = module_file_name(name, &target_triple, output.got.manifest.header.build_id);
    Ok(Generated {
        out_dir,
        target_triple,
        module_file,
        output,
    })
}

fn emit(
    request: &Request,
    name: &str,
    generated: &Generated,
    reporter: &mut Reporter,
    log: Log,
) -> u8 {
    let emit = &request.emit;
    let out_dir = &generated.out_dir;
    if let Err(error) = std::fs::create_dir_all(out_dir) {
        return artifact_error(
            reporter,
            &format!(
                "cannot create output directory `{}`: {error}",
                out_dir.display()
            ),
        );
    }
    let target_triple = &generated.target_triple;
    let output = &generated.output;

    let needs_ir = emit.iter().any(|kind| {
        matches!(
            kind,
            Emit::Ir | Emit::Obj | Emit::Module | Emit::Exe | Emit::Lib
        )
    });
    let ir_path = out_dir.join(format!("{name}.ll"));
    if needs_ir && let Err(error) = std::fs::write(&ir_path, &output.module) {
        return artifact_error(
            reporter,
            &format!("cannot write `{}`: {error}", ir_path.display()),
        );
    }
    if needs_ir {
        log.detail(format!("ir {}", display_path(&ir_path)));
    }

    let needs_metadata = emit
        .iter()
        .any(|kind| matches!(kind, Emit::Metadata | Emit::Module | Emit::Exe | Emit::Lib));
    if needs_metadata {
        let mut metadata = output.metadata.clone();
        metadata.module_file = generated.module_file.clone();
        let metadata_path = out_dir.join(format!("{name}.stc-meta.json"));
        if let Err(error) = std::fs::write(&metadata_path, metadata.to_json().render_pretty()) {
            return artifact_error(
                reporter,
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
                reporter,
                &format!("cannot write `{}`: {error}", json_path.display()),
            );
        }
        let binary_path = out_dir.join(format!("{name}.stc-got.bin"));
        if let Err(error) = std::fs::write(&binary_path, &output.got.manifest_binary) {
            return artifact_error(
                reporter,
                &format!("cannot write `{}`: {error}", binary_path.display()),
            );
        }
    }

    let needs_clang = emit
        .iter()
        .any(|kind| matches!(kind, Emit::Obj | Emit::Module | Emit::Exe | Emit::Lib));
    if !needs_clang {
        log.info(format!("artifacts {}", display_path(out_dir)));
        return 0;
    }
    if emit.contains(&Emit::Exe)
        && let Some(target) = effective_target(request)
        && !is_host_triple(&target)
    {
        return usage_error(
            reporter,
            &format!("cross-target executables are not supported (`{target}`)"),
        );
    }
    if emit.contains(&Emit::Lib)
        && let Some(target) = effective_target(request)
        && !is_host_triple(&target)
    {
        return usage_error(
            reporter,
            &format!("cross-target embedding libraries are not supported (`{target}`)"),
        );
    }
    let clang = match find_clang(request) {
        Ok(clang) => clang,
        Err(message) => return toolchain_error(reporter, "STC-E-BUILD-001", &message),
    };
    let mut clang_time = Duration::ZERO;
    let object_path = out_dir.join(format!("{name}.o"));
    let pic = !target_triple.contains("windows");
    let start = Instant::now();
    if let Err(message) = compile_ir(
        &clang,
        &ir_path,
        &object_path,
        effective_target(request).as_deref(),
        pic,
        request.opt_level,
        log,
    ) {
        return build_failure(reporter, &message, &ir_path);
    }
    clang_time += start.elapsed();
    if emit.contains(&Emit::Obj)
        && !needs_pair
        && !emit.contains(&Emit::Exe)
        && !emit.contains(&Emit::Lib)
    {
        log.info(format!("clang {}", duration(clang_time)));
        log.info(format!("artifacts {}", display_path(out_dir)));
        return 0;
    }

    if needs_pair {
        let module_path = out_dir.join(&generated.module_file);
        let start = Instant::now();
        if let Err(message) = link_module(
            &clang,
            &object_path,
            &module_path,
            effective_target(request).as_deref(),
            log,
        ) {
            return build_failure(reporter, &message, &ir_path);
        }
        clang_time += start.elapsed();
    }
    if emit.contains(&Emit::Lib) {
        let kind = RuntimeKind::from(request.lib_type);
        let runtime = match find_runtime(request, kind) {
            Ok(runtime) => runtime,
            Err(message) => return toolchain_error(reporter, "STC-E-BUILD-003", &message),
        };
        let library_path = out_dir.join(lib_name(name, request.lib_type));
        if let Err(error) = std::fs::copy(&runtime, &library_path) {
            return artifact_error(
                reporter,
                &format!(
                    "cannot write embedding host library `{}`: {error}",
                    library_path.display()
                ),
            );
        }
    }
    if emit.contains(&Emit::Exe) {
        let runtime = match find_runtime(request, RuntimeKind::Static) {
            Ok(runtime) => runtime,
            Err(message) => return toolchain_error(reporter, "STC-E-BUILD-003", &message),
        };
        let launcher_ir = stc_codegen::generate_launcher(
            name,
            &generated.module_file,
            effective_target(request).as_deref(),
        );
        let launcher_ir_path = out_dir.join(format!("{name}-launcher.ll"));
        if let Err(error) = std::fs::write(&launcher_ir_path, &launcher_ir) {
            return artifact_error(
                reporter,
                &format!("cannot write `{}`: {error}", launcher_ir_path.display()),
            );
        }
        log.detail(format!("ir {}", display_path(&launcher_ir_path)));
        let launcher_object = out_dir.join(format!("{name}-launcher.o"));
        let start = Instant::now();
        if let Err(message) = compile_ir(
            &clang,
            &launcher_ir_path,
            &launcher_object,
            effective_target(request).as_deref(),
            false,
            request.opt_level,
            log,
        ) {
            return build_failure(reporter, &message, &launcher_ir_path);
        }
        let exe_path = out_dir.join(exe_name(name));
        if let Err(message) = link_exe(
            &clang,
            &launcher_object,
            &runtime,
            &exe_path,
            effective_target(request).as_deref(),
            log,
        ) {
            return build_failure(reporter, &message, &launcher_ir_path);
        }
        clang_time += start.elapsed();
    }
    log.info(format!("clang {}", duration(clang_time)));
    log.info(format!("artifacts {}", display_path(out_dir)));
    0
}

/// Builds the stable artifact file name for one emit kind.
#[must_use]
pub(crate) fn artifact_file(
    kind: Emit,
    name: &str,
    module_file: &str,
    lib_type: LibType,
) -> String {
    match kind {
        Emit::Ir => format!("{name}.ll"),
        Emit::Obj => format!("{name}.o"),
        Emit::Module => module_file.to_owned(),
        Emit::Exe => exe_name(name),
        Emit::Lib => lib_name(name, lib_type),
        Emit::Metadata => format!("{name}.stc-meta.json"),
    }
}

/// Returns the `stc metadata` emit-kind spelling.
#[must_use]
pub(crate) const fn emit_kind(kind: Emit) -> &'static str {
    match kind {
        Emit::Ir => "ir",
        Emit::Obj => "obj",
        Emit::Module => "module",
        Emit::Exe => "exe",
        Emit::Lib => "lib",
        Emit::Metadata => "metadata",
    }
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

/// Returns the launcher path for a project artifact name and output directory.
#[must_use]
pub(crate) fn launcher_path(out_dir: &Path, name: &str) -> PathBuf {
    out_dir.join(exe_name(name))
}

fn lib_name(name: &str, lib_type: LibType) -> String {
    match lib_type {
        LibType::Static => {
            if cfg!(windows) {
                format!("{name}.lib")
            } else {
                format!("{name}.a")
            }
        }
        LibType::Shared => {
            if cfg!(windows) {
                format!("{name}.dll")
            } else if cfg!(target_os = "macos") {
                format!("lib{name}.dylib")
            } else {
                format!("lib{name}.so")
            }
        }
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

fn effective_target(request: &Request) -> Option<String> {
    request
        .target
        .clone()
        .or_else(|| environment("STC_TARGET"))
        .or_else(|| request.manifest_target.clone())
}

fn find_clang(request: &Request) -> Result<String, String> {
    let candidate = request
        .clang
        .clone()
        .or_else(|| environment("STC_CLANG"))
        .or_else(|| request.manifest_clang.clone())
        .unwrap_or_else(|| "clang".into());
    match Command::new(&candidate).arg("--version").output() {
        Ok(output) if output.status.success() => Ok(candidate),
        Ok(_) => Err(format!("`{candidate} --version` failed")),
        Err(error) => Err(format!("cannot run clang `{candidate}`: {error}")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeKind {
    Static,
    Shared,
}

impl From<LibType> for RuntimeKind {
    fn from(lib_type: LibType) -> Self {
        match lib_type {
            LibType::Static => Self::Static,
            LibType::Shared => Self::Shared,
        }
    }
}

impl RuntimeKind {
    fn file_name(self) -> &'static str {
        match self {
            Self::Static => {
                if cfg!(windows) {
                    "stc_runtime.lib"
                } else {
                    "libstc_runtime.a"
                }
            }
            Self::Shared => {
                if cfg!(windows) {
                    "stc_runtime.dll"
                } else if cfg!(target_os = "macos") {
                    "libstc_runtime.dylib"
                } else {
                    "libstc_runtime.so"
                }
            }
        }
    }
}

fn find_runtime(request: &Request, kind: RuntimeKind) -> Result<PathBuf, String> {
    let file_name = kind.file_name();
    if let Some(required) = request
        .runtime
        .clone()
        .or_else(|| environment("STC_RUNTIME"))
        .or_else(|| request.manifest_runtime.clone())
        .map(PathBuf::from)
    {
        if required.is_file() {
            return Ok(required);
        }
        return Err(format!(
            "runtime host library `{}` was not found",
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
    let action = if kind == RuntimeKind::Shared {
        "the shared host library"
    } else {
        "the static host archive"
    };
    Err(format!(
        "{action} `{file_name}` was not found; build it with \
         `cargo build -p stc-runtime` or pass `--runtime <path>`"
    ))
}

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn compile_ir(
    clang: &str,
    ir: &Path,
    object: &Path,
    target: Option<&str>,
    pic: bool,
    opt_level: OptLevel,
    log: Log,
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
    command.arg(opt_level.clang_flag());
    run_tool(&mut command, log)
}

fn link_module(
    clang: &str,
    object: &Path,
    output: &Path,
    target: Option<&str>,
    log: Log,
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
        // A PE timestamp would make two builds of the same module differ by
        // wall-clock time. /Brepro turns it into a content-derived value.
        command.args(["-Xlinker", "/Brepro"]);
    }
    run_tool(&mut command, log)
}

fn link_exe(
    clang: &str,
    object: &Path,
    runtime: &Path,
    output: &Path,
    target: Option<&str>,
    log: Log,
) -> Result<(), String> {
    let mut command = Command::new(clang);
    if let Some(target) = target {
        command.arg(format!("--target={target}"));
    }
    command.arg(object).arg(runtime);
    for argument in system_link_args() {
        command.arg(argument);
    }
    let target_triple = target.map_or_else(stc_codegen::host_triple, str::to_owned);
    if target_triple.contains("windows") {
        command.args(["-Xlinker", "/Brepro"]);
    }
    command.args(["-o"]).arg(output);
    run_tool(&mut command, log)
}

fn run_tool(command: &mut Command, log: Log) -> Result<(), String> {
    if log.trace() {
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

fn duration(duration: Duration) -> String {
    format!("{:.1}ms", duration.as_secs_f64() * 1000.0)
}

fn usage_error(reporter: &mut Reporter, message: &str) -> u8 {
    reporter.plain("STC-E-BUILD-004", Severity::Error, message);
    2
}

fn toolchain_error(reporter: &mut Reporter, code: &'static str, message: &str) -> u8 {
    reporter.plain(code, Severity::Error, message);
    2
}

fn artifact_error(reporter: &mut Reporter, message: &str) -> u8 {
    reporter.plain("STC-E-BUILD-002", Severity::Error, message);
    1
}

fn build_failure(reporter: &mut Reporter, message: &str, ir_path: &Path) -> u8 {
    reporter.plain(
        "STC-E-BUILD-002",
        Severity::Error,
        &format!(
            "{message}\n  = note: the generated IR is kept at `{}`",
            ir_path.display()
        ),
    );
    1
}

/// The `stc build` compatibility summary when a baseline is used.
struct CompatibilityReport {
    verdict: &'static str,
    baseline_build_id: Option<String>,
    candidate_build_id: String,
    preserved: usize,
    allocated: usize,
    dropped: usize,
    retired: usize,
    rejected: usize,
    warnings: Vec<(String, String)>,
    rejections: Vec<(String, String)>,
    actions: Vec<(String, String, String)>,
}

impl CompatibilityReport {
    fn new(candidate: &Manifest, baseline: &Manifest) -> Self {
        let verdict = match candidate.plan.verdict {
            PlanVerdict::Cold => "cold",
            PlanVerdict::OnlineCandidate => "compatible",
            PlanVerdict::Rejected => "incompatible",
        };
        let mut preserved = 0;
        let mut allocated = 0;
        let mut dropped = 0;
        let mut rejected = 0;
        let mut actions = Vec::new();
        for action in &candidate.plan.actions {
            match action.action {
                PlanAction::Preserve | PlanAction::Migrate => preserved += 1,
                PlanAction::Allocate | PlanAction::Reset => allocated += 1,
                PlanAction::Drop => dropped += 1,
                PlanAction::Reject => rejected += 1,
            }
            actions.push((
                action.got_id.to_hex(),
                identity_kind_name(action.kind),
                action.key.clone(),
            ));
        }
        let retired = candidate
            .history
            .retired
            .len()
            .saturating_sub(baseline.history.retired.len());
        let mut warnings = Vec::new();
        let mut rejections = Vec::new();
        for diagnostic in &candidate.plan.diagnostics {
            let entry = (diagnostic.code.clone(), diagnostic.message.clone());
            if diagnostic.severity >= 2 {
                rejections.push(entry);
            } else {
                warnings.push(entry);
            }
        }
        Self {
            verdict,
            baseline_build_id: candidate.plan.parent_build_id.map(stc_got::GotId::to_hex),
            candidate_build_id: candidate.header.build_id.to_hex(),
            preserved,
            allocated,
            dropped,
            retired,
            rejected,
            warnings,
            rejections,
            actions,
        }
    }

    fn to_json(&self, verbose: bool) -> Json {
        let mut fields = vec![
            ("verdict".into(), Json::String(self.verdict.into())),
            (
                "baseline_build_id".into(),
                self.baseline_build_id
                    .clone()
                    .map_or(Json::Null, Json::String),
            ),
            (
                "candidate_build_id".into(),
                Json::String(self.candidate_build_id.clone()),
            ),
            ("counts".into(), self.counts_json()),
            ("warnings".into(), diagnostics_json(&self.warnings)),
            ("rejections".into(), diagnostics_json(&self.rejections)),
        ];
        if verbose {
            fields.push((
                "actions".into(),
                Json::Array(
                    self.actions
                        .iter()
                        .map(|(got_id, kind, key)| {
                            Json::object([
                                ("got_id".into(), Json::String(got_id.clone())),
                                ("kind".into(), Json::String(kind.clone())),
                                ("key".into(), Json::String(key.clone())),
                            ])
                        })
                        .collect(),
                ),
            ));
        }
        Json::object(fields)
    }

    fn counts_json(&self) -> Json {
        Json::object([
            (
                "preserved".into(),
                Json::Number(i128::try_from(self.preserved).unwrap_or(i128::MAX)),
            ),
            (
                "allocated".into(),
                Json::Number(i128::try_from(self.allocated).unwrap_or(i128::MAX)),
            ),
            (
                "dropped".into(),
                Json::Number(i128::try_from(self.dropped).unwrap_or(i128::MAX)),
            ),
            (
                "retired".into(),
                Json::Number(i128::try_from(self.retired).unwrap_or(i128::MAX)),
            ),
            (
                "rejected".into(),
                Json::Number(i128::try_from(self.rejected).unwrap_or(i128::MAX)),
            ),
        ])
    }

    fn print(&self, reporter: &Reporter, log: Log) {
        if reporter.is_json() {
            println!("{}", self.to_json(log.verbose()).render());
            return;
        }
        let baseline = self.baseline_build_id.as_deref().unwrap_or("<none>");
        log.status(format!(
            "compatibility {}: baseline {baseline} candidate {}",
            self.verdict, self.candidate_build_id
        ));
        log.status(format!(
            "  preserved {}  allocated {}  dropped {}  retired {}  rejected {}",
            self.preserved, self.allocated, self.dropped, self.retired, self.rejected
        ));
        for (code, message) in &self.warnings {
            log.status(format!("  warning[{code}]: {message}"));
        }
        for (code, message) in &self.rejections {
            log.status(format!("  rejection[{code}]: {message}"));
        }
        if log.verbose() {
            for (got_id, kind, key) in &self.actions {
                log.info(format!("  action {kind} {key} {got_id}"));
            }
        }
    }
}

fn identity_kind_name(tag: u8) -> String {
    stc_got::IdentityKind::from_tag(tag).map_or_else(
        || format!("kind-{tag}"),
        |kind| kind.name().replace(' ', "-"),
    )
}

fn diagnostics_json(entries: &[(String, String)]) -> Json {
    Json::Array(
        entries
            .iter()
            .map(|(code, message)| {
                Json::object([
                    ("code".into(), Json::String(code.clone())),
                    ("message".into(), Json::String(message.clone())),
                ])
            })
            .collect(),
    )
}
