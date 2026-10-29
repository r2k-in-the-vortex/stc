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
use stc_got::{Digest32, Manifest, Rename};
use stc_hir::HirCrate;
use stc_manifest::{ChannelRef, ConfigValue, EmitKind, LibType, OptLevel};
use stc_span::SourceMap;

use crate::compat::CompatibilityReport;
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
    /// `--mock-clock`; advances without sleeping when present.
    pub mock_clock_ns: Option<u64>,
}

/// Project metadata settings copied from `stc.toml` into module metadata.
#[derive(Clone, Debug)]
pub(crate) struct ProjectSettings {
    /// `[retain].enabled`.
    pub retain_enabled: bool,
    /// `[retain].file`.
    pub retain_file: Option<String>,
    /// `[retain].on-mismatch`.
    pub retain_on_mismatch: &'static str,
    /// `[io].backend`.
    pub io_backend: String,
    /// `[io].file`.
    pub io_file: Option<String>,
    /// Declared devices.
    pub devices: Vec<stc_codegen::DeviceMetadata>,
    /// Project bindings.
    pub bindings: Vec<stc_codegen::ProjectBinding>,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            retain_enabled: true,
            retain_file: None,
            retain_on_mismatch: "error",
            io_backend: "null".into(),
            io_file: None,
            devices: Vec::new(),
            bindings: Vec::new(),
        }
    }
}

impl ProjectSettings {
    /// Builds the metadata settings from a parsed manifest.
    pub(crate) fn from_manifest(manifest: &stc_manifest::Manifest) -> Self {
        Self {
            retain_enabled: manifest.retain.enabled,
            retain_file: manifest.retain.file.clone(),
            retain_on_mismatch: manifest.retain.on_mismatch.as_str(),
            io_backend: manifest.io.backend.as_str().into(),
            io_file: manifest.io.file.clone(),
            devices: manifest
                .io
                .devices
                .iter()
                .map(|device| stc_codegen::DeviceMetadata {
                    name: device.name.clone(),
                    kind: device.kind.clone(),
                    config: device.config.as_ref().map(config_json),
                    channels: device
                        .channels
                        .iter()
                        .map(|channel| stc_codegen::ChannelMetadata {
                            name: channel.name.clone(),
                            direction: channel.direction.as_str().into(),
                            width: channel.width,
                        })
                        .collect(),
                })
                .collect(),
            bindings: manifest
                .io
                .bindings
                .iter()
                .map(|binding| stc_codegen::ProjectBinding {
                    variable: binding.variable.clone(),
                    device: binding.device.clone(),
                    channel: binding.channel.as_ref().map(|channel| match channel {
                        ChannelRef::Index(index) => stc_codegen::ProjectChannel::Index(*index),
                        ChannelRef::Name(name) => stc_codegen::ProjectChannel::Name(name.clone()),
                    }),
                    direction: binding.direction.map(|direction| direction.as_str().into()),
                })
                .collect(),
        }
    }
}

/// Converts an opaque manifest TOML value into canonical metadata JSON.
fn config_json(value: &ConfigValue) -> Json {
    match value {
        ConfigValue::String(value) => Json::String(value.clone()),
        ConfigValue::Integer(value) => Json::Number(i128::from(*value)),
        ConfigValue::Float(value) if value.is_finite() => Json::Float(*value),
        // JSON has no NaN/infinity; keep the TOML spelling as a string so the
        // opaque device configuration stays valid JSON.
        ConfigValue::Float(value) => Json::String(if value.is_nan() {
            "nan".into()
        } else if value.is_sign_positive() {
            "inf".into()
        } else {
            "-inf".into()
        }),
        ConfigValue::Boolean(value) => Json::Bool(*value),
        ConfigValue::Datetime(value) => Json::String(value.clone()),
        ConfigValue::Array(values) => Json::Array(values.iter().map(config_json).collect()),
        ConfigValue::Table(values) => Json::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), config_json(value)))
                .collect(),
        ),
    }
}

/// The selected debug/release build form (Debug and Monitoring §17).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuildMode {
    /// Debug form: debug information and checkpoint hooks.
    Debug,
    /// Optimised, stripped, hook-free form.
    Release,
}

impl BuildMode {
    /// Returns the mode spelling used by diagnostics and metadata summaries.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }

    /// Returns whether this is the release form.
    #[must_use]
    pub(crate) const fn is_release(self) -> bool {
        matches!(self, Self::Release)
    }
}

/// Returns the default out-dir for a build mode, relative to the project root.
#[must_use]
pub(crate) const fn default_out_dir(mode: BuildMode) -> &'static str {
    match mode {
        BuildMode::Debug => "target/stc/debug",
        BuildMode::Release => "target/stc/release",
    }
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
    /// Whether the out-dir came from `--out-dir` or `[build].out-dir`, as
    /// opposed to the mode default. Explicit directories are used verbatim
    /// and may not mix debug and release artifacts.
    pub out_dir_explicit: bool,
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
    /// An explicit CLI mode (`--debug`/`--release`), before manifest
    /// precedence is applied.
    pub mode_override: Option<BuildMode>,
    /// Resolved debug form: debug information and checkpoint hooks.
    pub debug: bool,
    /// Resolved release form: optimised, stripped, and hook-free.
    pub release: bool,
    /// Whether this is a test build (`stc test`): emit the test-host launcher
    /// and the host ABI 2.2 test-clock import.
    pub test_build: bool,
    /// Effective `[build].opt-level`.
    pub opt_level: OptLevel,
    /// Whether `[build].opt-level` was explicitly present; absent levels use
    /// the mode default (0 for debug, 2 for release).
    pub opt_level_explicit: bool,
    /// `[build].lib-type`.
    pub lib_type: LibType,
    /// `[build].target`.
    pub manifest_target: Option<String>,
    /// `[build].clang`.
    pub manifest_clang: Option<String>,
    /// `[build].runtime`.
    pub manifest_runtime: Option<String>,
    /// Project metadata settings.
    pub project: ProjectSettings,
    /// `stc run` options.
    pub run: RunRequest,
}

impl Request {
    /// Records the selected mode as the resolved debug/release flags.
    pub(crate) fn apply_mode(&mut self, mode: BuildMode) {
        self.mode_override = Some(mode);
        self.debug = !mode.is_release();
        self.release = mode.is_release();
    }
}

impl Default for Request {
    fn default() -> Self {
        Self {
            name: None,
            emit: BTreeSet::new(),
            out_dir: None,
            out_dir_explicit: false,
            target: None,
            clang: None,
            runtime: None,
            got_manifest: None,
            cold: false,
            renames: Vec::new(),
            allow_untracked_removal: false,
            relaxed: false,
            mode_override: None,
            debug: false,
            release: false,
            test_build: false,
            opt_level: OptLevel::O0,
            opt_level_explicit: false,
            lib_type: LibType::Static,
            manifest_target: None,
            manifest_clang: None,
            manifest_runtime: None,
            project: ProjectSettings::default(),
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
    source_map: &SourceMap,
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
        source_map,
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
    source_map: &SourceMap,
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
        source_map,
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
    source_map: &SourceMap,
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
        source_map,
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
    source_map: &SourceMap,
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
        source_map,
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
pub(crate) fn generate(
    request: &Request,
    hir: &HirCrate,
    name: &str,
    sources: &[String],
    source_paths: Option<&[String]>,
    source_map: &SourceMap,
    fingerprint: Digest32,
    diagnostics: &mut DiagnosticSink,
    reporter: &mut Reporter,
    log: Log,
) -> Result<Generated, u8> {
    let out_dir = PathBuf::from(request.out_dir.clone().unwrap_or_else(|| {
        default_out_dir(if request.release {
            BuildMode::Release
        } else {
            BuildMode::Debug
        })
        .into()
    }));
    let target = effective_target(request);
    let target_triple = target.clone().unwrap_or_else(stc_codegen::host_triple);
    let baseline = match &request.got_manifest {
        Some(path) => match read_baseline(Path::new(path)) {
            Ok(manifest) => Some(manifest),
            Err(message) => return Err(artifact_error(reporter, &message)),
        },
        None => None,
    };
    // GOT keys and field paths are normalized to ASCII uppercase, so a
    // source-level history entry such as `Main.Speed` matches the engine's
    // `APP::MAIN.RATE#...` keys.
    let renames: Vec<Rename> = request
        .renames
        .iter()
        .map(|(old, new)| {
            Rename::new(
                old.to_ascii_uppercase().as_str(),
                new.to_ascii_uppercase().as_str(),
            )
        })
        .collect();
    if !renames.is_empty() && (baseline.is_none() || request.cold) {
        reporter.plain(
            "STC-W-PROJECT-002",
            Severity::Warning,
            "the rename history has no active baseline; renamed keys receive fresh GotIds",
        );
    }

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
        release: request.release,
        sources,
        source_paths,
        source_map: Some(source_map),
        module: stc_codegen::ModuleBuildOptions {
            compiler_version: format!("stc {}", env!("CARGO_PKG_VERSION")),
            source_fingerprint: fingerprint,
            baseline: baseline.as_ref(),
            cold: request.cold,
            allow_untracked_removal: request.allow_untracked_removal,
            renames,
        },
        project: stc_codegen::ProjectOptions {
            retain: stc_codegen::RetainMetadataOptions {
                enabled: request.project.retain_enabled,
                file: request.project.retain_file.clone(),
                on_mismatch: request.project.retain_on_mismatch,
            },
            io: stc_codegen::IoMetadataOptions {
                backend: request.project.io_backend.clone(),
                file: request.project.io_file.clone(),
                devices: request.project.devices.clone(),
                bindings: request.project.bindings.clone(),
            },
        },
        test_build: request.test_build,
    };
    let codegen_start = Instant::now();
    let output = match stc_codegen::generate(hir, &options) {
        Ok(output) => output,
        Err(errors) => {
            let mut internal = false;
            let mut project = false;
            for error in errors {
                match error {
                    stc_codegen::CodegenError::Unsupported {
                        code,
                        message,
                        span,
                    } if span.start == 0 && span.end == 0 => {
                        project |= code.starts_with("STC-E-PROJECT-");
                        reporter.plain(code, Severity::Error, &message);
                    }
                    stc_codegen::CodegenError::Unsupported {
                        code,
                        message,
                        span,
                    } => {
                        project |= code.starts_with("STC-E-PROJECT-");
                        diagnostics.error(code, message, span);
                    }
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
            return Err(if internal {
                3
            } else if project {
                2
            } else {
                1
            });
        }
    };
    log.info(format!("codegen {}", elapsed(codegen_start)));

    let mut plan_error = false;
    for diagnostic in &output.got.diagnostics {
        // Test artifacts are deliberately cold-layout and never deployed; the
        // missing-baseline warning is expected noise for `stc test`.
        if request.test_build && !diagnostic.is_error() && diagnostic.code == "STC-W-MANIFEST-001" {
            continue;
        }
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
        let report = CompatibilityReport::new(&output.got.manifest, Some(baseline));
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

pub(crate) fn emit(
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
    if request.out_dir_explicit
        && let Some(existing) = existing_build_modes(out_dir)
            .into_iter()
            .find(|release| *release != request.release)
    {
        let existing = if existing { "release" } else { "debug" };
        reporter.plain(
            "STC-E-PROJECT-010",
            Severity::Error,
            &format!(
                "out-dir `{}` contains {existing} artifacts; debug and release builds must use \
                 distinct output directories",
                out_dir.display()
            ),
        );
        return 2;
    }
    let target_triple = &generated.target_triple;
    let output = &generated.output;
    let opt_level = effective_opt_level(request);

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
        &generated.target_triple,
        pic,
        request.debug,
        opt_level,
        log,
    ) {
        return build_failure(reporter, &message, &ir_path);
    }
    clang_time += start.elapsed();
    if needs_pair
        && request.debug
        && !request.release
        && !request.test_build
        && !output.frames.is_empty()
        && let Some(debug) = &output.metadata.debug
    {
        let target = stc_codegen::resolve_target(Some(target_triple));
        let extract = crate::frames::ExtractRequest {
            object: &object_path,
            clang: &clang,
            target_triple,
            build_id: &debug.build_id,
            pointer_width: target.pointer_width,
            hints: &output.frames,
        };
        if let Some(table) = crate::frames::extract(&extract) {
            let frames_path = out_dir.join(format!("{name}.stc-frames.json"));
            if let Err(error) = std::fs::write(&frames_path, table) {
                return artifact_error(
                    reporter,
                    &format!("cannot write `{}`: {error}", frames_path.display()),
                );
            }
            log.detail(format!("frames {}", display_path(&frames_path)));
        }
    }
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
        if request.release
            && let Err(message) = strip_release_artifact(&clang, &module_path, log)
        {
            return toolchain_error(reporter, "STC-E-BUILD-001", &message);
        }
        clang_time += start.elapsed();
    }
    if emit.contains(&Emit::Lib) {
        let kind = RuntimeKind::from(request.lib_type);
        let runtime = match find_runtime(request, kind, request.release) {
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
        if request.release
            && let Err(message) = strip_release_artifact(&clang, &library_path, log)
        {
            return toolchain_error(reporter, "STC-E-BUILD-001", &message);
        }
    }
    if emit.contains(&Emit::Exe) {
        let runtime = match find_runtime(request, RuntimeKind::Static, request.release) {
            Ok(runtime) => runtime,
            Err(message) => return toolchain_error(reporter, "STC-E-BUILD-003", &message),
        };
        let launcher_ir = if request.test_build {
            stc_codegen::generate_test_launcher(
                name,
                &generated.module_file,
                effective_target(request).as_deref(),
            )
        } else {
            stc_codegen::generate_launcher(
                name,
                &generated.module_file,
                effective_target(request).as_deref(),
            )
        };
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
            &generated.target_triple,
            false,
            false,
            opt_level,
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
        if request.release
            && let Err(message) = strip_release_artifact(&clang, &exe_path, log)
        {
            return toolchain_error(reporter, "STC-E-BUILD-001", &message);
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

fn effective_opt_level(request: &Request) -> OptLevel {
    if request.opt_level_explicit {
        request.opt_level
    } else if request.release {
        OptLevel::O2
    } else {
        OptLevel::O0
    }
}

/// Finds every paired manifest in an explicit out-dir and returns the build
/// form each records (`true` for release). Malformed or unrelated JSON is
/// ignored; this check only guards against a supported debug set being
/// overwritten by a release set, or the reverse.
fn existing_build_modes(out_dir: &Path) -> Vec<bool> {
    let Ok(entries) = std::fs::read_dir(out_dir) else {
        return Vec::new();
    };
    let mut modes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !path.is_file() {
            continue;
        }
        if name.ends_with(".stc-got.json") {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(manifest) = Manifest::decode_json(&text) else {
                continue;
            };
            modes.push(manifest.header.generator_flags & (1 << 1) != 0);
        } else if name.ends_with(".stc-meta.json") {
            // Emit sets that contain metadata but no paired manifest still
            // identify their mode: every debug-form sidecar carries the
            // additive debug object, and release sidecars never do.
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(root) = stc_got::json::parse(&text) else {
                continue;
            };
            let Some(object) = root.as_object() else {
                continue;
            };
            modes.push(!object.contains_key("debug"));
        }
    }
    modes
}

/// Removes DWARF/CodeView debug sections from a release artifact.
///
/// The compiler already emits release objects without `-g`; stripping also
/// removes debug information contributed by the runtime host archive and any
/// corresponding CodeView records. Debug-sidecars are not produced for
/// release, and `--strip-debug` preserves the build-ID note, descriptor,
/// manifest metadata, and symbols.
///
/// The static host archive contains precompiled Rust standard-library objects
/// that carry CodeView/DWARF regardless of the crate's own strip setting.
/// Archives are therefore unpacked, each object member is stripped, and the
/// stripped objects replace the originals in place (D-191).
fn strip_release_artifact(clang: &str, artifact: &Path, log: Log) -> Result<(), String> {
    if is_archive(artifact) {
        return strip_archive(clang, artifact, log);
    }
    let tool = find_objcopy(clang)?;
    let mut command = Command::new(&tool);
    command.arg("--strip-debug").arg(artifact);
    run_tool(&mut command, log).map_err(|error| {
        format!(
            "cannot strip debug information from `{}`: {error}",
            artifact.display()
        )
    })
}

/// Strips object members of a static archive and replaces them in place.
///
/// The archive is not repacked from scratch: Rust staticlibs contain repeated
/// import-library members (for example several copies of the same Windows
/// API-set DLL), and a fresh `ar rcs` would collapse duplicate names and drop
/// their symbol-table entries. Extracting, stripping, and updating each object
/// member with `llvm-ar rD` keeps every original member and symbol.
fn strip_archive(clang: &str, archive: &Path, log: Log) -> Result<(), String> {
    // Extraction runs with the archive's temporary directory as its working
    // directory, so resolve the archive path first.
    let archive = std::fs::canonicalize(archive)
        .map_err(|error| format!("cannot resolve archive `{}`: {error}", archive.display()))?;
    let archive = archive.as_path();
    let archive_tool = find_beside_clang(
        clang,
        &["llvm-ar.exe", "llvm-ar", "ar.exe", "ar"],
        "`llvm-ar` or `ar`",
    )?;
    let objcopy = find_objcopy(clang)?;
    let directory = archive.with_extension(format!("strip-{}", std::process::id()));
    if directory.exists() {
        std::fs::remove_dir_all(&directory).map_err(|error| {
            format!(
                "cannot clear strip directory `{}`: {error}",
                directory.display()
            )
        })?;
    }
    std::fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "cannot create strip directory `{}`: {error}",
            directory.display()
        )
    })?;
    let result = (|| -> Result<(), String> {
        let mut extract = Command::new(&archive_tool);
        extract.arg("x").arg(archive).current_dir(&directory);
        run_tool(&mut extract, log)
            .map_err(|error| format!("cannot extract `{}`: {error}", archive.display()))?;
        let mut candidates: Vec<PathBuf> = std::fs::read_dir(&directory)
            .map_err(|error| format!("cannot read `{}`: {error}", directory.display()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<_, _>>()
            .map_err(|error| error.to_string())?;
        candidates.retain(|member| {
            member.is_file()
                && member
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        extension.eq_ignore_ascii_case("o") || extension.eq_ignore_ascii_case("obj")
                    })
                // Strip only members we can positively identify as native
                // objects. Rust staticlibs also carry LLVM bitcode, import
                // libraries, and other members that some `ar` cannot rewrite.
                && is_native_object(member)
        });
        candidates.sort();
        if candidates.is_empty() {
            return Err(format!(
                "archive `{}` contains no native object members",
                archive.display()
            ));
        }
        for member in &candidates {
            let mut command = Command::new(&objcopy);
            command.arg("--strip-debug").arg(member);
            run_tool(&mut command, log).map_err(|error| {
                format!(
                    "cannot strip archive member `{}`: {error}",
                    member.display()
                )
            })?;
        }
        // A member whose format did not survive stripping is left in the
        // original archive untouched rather than handed to `ar`.
        let objects: Vec<PathBuf> = candidates
            .into_iter()
            .filter(|member| is_native_object(member))
            .collect();
        if objects.is_empty() {
            return Err(format!(
                "archive `{}` lost every native object member during stripping",
                archive.display()
            ));
        }
        // A Rust staticlib has hundreds of members; on Windows pass them
        // through a response file so the command line stays below the limit.
        // Unix command lines are long enough for direct arguments and avoid
        // relying on an `ar @file` extension.
        let mut replace = Command::new(&archive_tool);
        replace.arg("rD");
        if cfg!(windows) {
            let mut response = String::new();
            response.push_str(&archive.to_string_lossy());
            response.push('\n');
            for member in &objects {
                let name = member.file_name().ok_or_else(|| {
                    format!("archive member `{}` has no file name", member.display())
                })?;
                response.push_str(&name.to_string_lossy());
                response.push('\n');
            }
            std::fs::write(directory.join("objects.txt"), response).map_err(|error| {
                format!(
                    "cannot write archive member list in `{}`: {error}",
                    directory.display()
                )
            })?;
            replace.arg("@objects.txt");
        } else {
            replace.arg(archive);
            for member in &objects {
                let name = member.file_name().ok_or_else(|| {
                    format!("archive member `{}` has no file name", member.display())
                })?;
                replace.arg(name);
            }
        }
        // `r` replaces the named object members in place, preserving every
        // other member and its index; `D` zeroes the new members' timestamps.
        replace.current_dir(&directory);
        if let Err(error) = run_tool(&mut replace, log) {
            // Include the format of any member named in the tool error so a
            // platform-specific archive failure is diagnosable from the log.
            let mut message = format!("cannot update `{}`: {error}", archive.display());
            for member in &objects {
                let Some(name) = member.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if !error.contains(name) {
                    continue;
                }
                match std::fs::read(member) {
                    Ok(bytes) => {
                        let magic = bytes
                            .get(..8)
                            .map(|magic| {
                                magic
                                    .iter()
                                    .map(|byte| format!("{byte:02x}"))
                                    .collect::<String>()
                            })
                            .unwrap_or_default();
                        message.push_str(&format!(
                            "\n  member `{name}`: {} bytes, magic {magic}",
                            bytes.len()
                        ));
                    }
                    Err(read) => {
                        message.push_str(&format!("\n  member `{name}`: {read}"));
                    }
                }
            }
            return Err(message);
        }
        log.detail(format!("stripped archive {}", archive.display()));
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&directory);
    result
}

/// Finds a tool beside clang or on `PATH`.
fn find_beside_clang(clang: &str, names: &[&str], description: &str) -> Result<String, String> {
    for directory in clang_directories(clang) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Ok(candidate.to_string_lossy().into_owned());
            }
        }
    }
    for name in names {
        if Command::new(name)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return Ok((*name).to_owned());
        }
    }
    Err(format!(
        "release builds require {description} beside clang or on PATH"
    ))
}

fn is_archive(path: &Path) -> bool {
    use std::io::Read;

    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0_u8; 8];
    file.read_exact(&mut magic).is_ok() && &magic == b"!<arch>\n"
}

/// Returns whether a file starts with a native object format header
/// (ELF, COFF, or Mach-O). LLVM bitcode, import libraries, and other archive
/// members are not rewritten.
fn is_native_object(path: &Path) -> bool {
    use std::io::Read;

    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0_u8; 4];
    if file.read_exact(&mut magic).is_err() {
        return false;
    }
    if magic == *b"\x7fELF" {
        return true;
    }
    if matches!(
        magic,
        [0xcf, 0xfa, 0xed, 0xfe]
            | [0xfe, 0xed, 0xfa, 0xcf]
            | [0xce, 0xfa, 0xed, 0xfe]
            | [0xca, 0xfe, 0xba, 0xbe]
            | [0xbe, 0xba, 0xfe, 0xca]
    ) {
        return true;
    }
    let machine = u16::from_le_bytes([magic[0], magic[1]]);
    matches!(
        machine,
        0x014c
            | 0x0162
            | 0x0166
            | 0x0168
            | 0x01c0
            | 0x01c2
            | 0x01c4
            | 0x01f0
            | 0x01f1
            | 0x0200
            | 0x8664
            | 0xaa64
            | 0xa641
    )
}

fn find_objcopy(clang: &str) -> Result<String, String> {
    for directory in clang_directories(clang) {
        for name in ["llvm-objcopy.exe", "llvm-objcopy", "objcopy.exe", "objcopy"] {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Ok(candidate.to_string_lossy().into_owned());
            }
        }
    }
    for name in ["llvm-objcopy", "objcopy"] {
        if Command::new(name)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return Ok(name.into());
        }
    }
    Err("release builds require `llvm-objcopy` or `objcopy` beside clang or on PATH".into())
}

/// Returns the directories that may contain the clang toolchain's binary
/// utilities. A bare `clang` is resolved through `PATH`; the canonical path is
/// also considered so a `/usr/bin/clang` symlink finds its sibling
/// `llvm-objcopy`.
fn clang_directories(clang: &str) -> Vec<PathBuf> {
    let configured = Path::new(clang);
    let resolved = if configured
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
    {
        Some(configured.to_path_buf())
    } else {
        let name = configured
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or(clang);
        executable_on_path(name)
    };
    let mut directories = Vec::new();
    if let Some(path) = resolved {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            directories.push(parent.to_path_buf());
        }
        if let Ok(canonical) = path.canonicalize()
            && let Some(parent) = canonical.parent()
            && !directories.iter().any(|directory| directory == parent)
        {
            directories.push(parent.to_path_buf());
        }
    }
    directories
}

fn executable_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let extensions: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ".com", ""]
    } else {
        &[""]
    };
    for directory in std::env::split_paths(&path) {
        for extension in extensions {
            let candidate = directory.join(format!("{name}{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
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

fn find_runtime(request: &Request, kind: RuntimeKind, release: bool) -> Result<PathBuf, String> {
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
        // An unpacked release package keeps `stc` in `bin/` and the runtime
        // archives in the sibling `lib/` directory (D-189).
        candidates.push(directory.join(file_name));
        candidates.push(directory.join("lib").join(file_name));
        if let Some(root) = directory.parent() {
            candidates.push(root.join("lib").join(file_name));
        }
    }
    if let Ok(current) = std::env::current_dir() {
        if release {
            candidates.push(current.join("target/release").join(file_name));
            candidates.push(current.join("target/debug").join(file_name));
        } else {
            candidates.push(current.join("target/debug").join(file_name));
            candidates.push(current.join("target/release").join(file_name));
        }
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

#[allow(clippy::too_many_arguments)]
fn compile_ir(
    clang: &str,
    ir: &Path,
    object: &Path,
    target: Option<&str>,
    target_triple: &str,
    pic: bool,
    debug: bool,
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
    if target_triple.contains("windows") {
        // LLVM stamps COFF objects with the wall-clock time when incremental
        // linker compatibility is enabled. Disable it so identical inputs
        // produce byte-identical objects in any output directory.
        command.arg("-mno-incremental-linker-compatible");
    }
    if target_triple.starts_with("aarch64-") {
        // The format-1 frame walk compares saved link registers with manifest
        // code ranges. Pointer authentication would sign those saved values
        // and make caller frames unrecognizable; generated code does not rely
        // on PAC/BTI, so emit without branch protection (D-202).
        command.arg("-mbranch-protection=none");
    }
    if debug {
        command.arg("-g");
        // Keep compilation-directory and object-name metadata relative so the
        // same inputs produce byte-identical objects in any output directory.
        command.arg("-fdebug-compilation-dir=.");
        if target_triple.contains("windows") && target_triple.contains("msvc") {
            command.args(["-gcodeview", "-gno-codeview-command-line"]);
            let object_name = object
                .file_name()
                .map_or_else(|| "stc.o".into(), |name| name.to_string_lossy());
            command
                .arg("-Xclang")
                .arg(format!("-object-file-name={object_name}"));
        } else {
            command.arg("-gdwarf-5");
        }
    }
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
