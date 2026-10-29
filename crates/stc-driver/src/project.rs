//! Project mode: load `stc.toml`, compile the package graph in order, and
//! report package-qualified diagnostics (M6 phases 2–3, D-153).
//!
//! `check`, `build`, and `metadata` without input files operate on the project
//! manifest; passing input files keeps the explicit manifest-free bridge in
//! `main.rs`/`build.rs`.

use std::path::PathBuf;
use std::time::Instant;

use stc_diag::DiagnosticSink;
use stc_lexer::tokenize_bytes;
use stc_manifest::{LoadOptions, SyntaxMode, load_project};
use stc_parser::{ParseOptions, parse};
use stc_sema::{PackageSession, ProjectSession};
use stc_span::{FileId, SourceMap, Span};

use crate::build::{self, Emit, Request};
use crate::metadata;
use crate::report::{DiagnosticPaths, Log, Reporter, absolute_path, elapsed};

/// The project command being run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Analyze only.
    Check,
    /// Generate and write artifacts.
    Build,
    /// Generate and print the metadata summary.
    Metadata,
    /// Build the launcher, run it, and return its exit code.
    Run,
    /// Build the launcher and own a local debug session.
    Debug,
}

/// Resolves the build mode from CLI flags, then an explicit `[build].debug`,
/// then an explicit `[debug].default`, then the debug default
/// (Debug and Monitoring §17.1). The manifest parser already rejects explicit
/// settings that disagree.
pub(crate) fn select_mode(
    request: &Request,
    manifest: &stc_manifest::Manifest,
) -> build::BuildMode {
    if let Some(mode) = request.mode_override {
        return mode;
    }
    let debug = if manifest.build.debug_explicit {
        manifest.build.debug
    } else if manifest.debug.default_explicit {
        manifest.debug.default
    } else {
        true
    };
    if debug {
        build::BuildMode::Debug
    } else {
        build::BuildMode::Release
    }
}

struct SourceEntry {
    package: usize,
    path: String,
    relative: String,
    bytes: Vec<u8>,
}

/// A project prepared through parsing, analysis, and build-request
/// resolution. Shared by `check`/`build`/`run`/`debug`/`metadata` and by the
/// DAP launcher, which builds without an interactive debug loop.
struct Prepared {
    graph: stc_manifest::ProjectGraph,
    analysis: stc_sema::Analysis,
    sources: SourceMap,
    diagnostics: DiagnosticSink,
    request: Request,
    name: String,
    source_files: Vec<String>,
    source_paths: Vec<String>,
    fingerprint: stc_got::Digest32,
    build_mode: build::BuildMode,
}

/// The artifacts and settings a run or debug launch needs.
pub(crate) struct ProjectBuild {
    /// Built launcher executable.
    pub(crate) launcher: PathBuf,
    /// Output directory that holds the launcher, module, metadata, and sides.
    pub(crate) out_dir: PathBuf,
    /// Resolved run options (`--cycles`, clock, I/O, state).
    pub(crate) run: crate::build::RunRequest,
    /// Root-package manifest used for debug-server flags and source mapping.
    pub(crate) manifest: stc_manifest::Manifest,
    /// Root package directory.
    pub(crate) root_dir: PathBuf,
}

/// Runs a project command.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    mode: Mode,
    manifest: Option<&str>,
    locked: bool,
    diagnostic_paths: DiagnosticPaths,
    request: &Request,
    debug: &crate::cli::DebugOptions,
    reporter: &mut Reporter,
    log: Log,
) -> u8 {
    let mut prepared = match prepare(
        mode,
        manifest,
        locked,
        diagnostic_paths,
        request,
        reporter,
        log,
    ) {
        Ok(prepared) => prepared,
        Err(code) => return code,
    };
    match mode {
        Mode::Build | Mode::Run | Mode::Debug => {
            let code = execute_build(&mut prepared, reporter, log);
            if code != 0 {
                return code;
            }
            if mode == Mode::Run || mode == Mode::Debug {
                let build = project_build(&prepared);
                if mode == Mode::Debug {
                    return crate::debug::launch(
                        &build.launcher,
                        &build.out_dir,
                        &build.run,
                        debug,
                        &build.manifest,
                        &build.root_dir,
                        reporter,
                        log,
                    );
                }
                return crate::run::launch(
                    &build.launcher,
                    &build.run,
                    debug,
                    &build.manifest,
                    &build.root_dir,
                    &build.out_dir,
                    reporter,
                    log,
                );
            }
        }
        Mode::Metadata => {
            let generated = match build::generate_project(
                &prepared.request,
                &prepared.analysis.hir,
                &prepared.name,
                &prepared.source_files,
                &prepared.source_paths,
                &prepared.sources,
                prepared.fingerprint,
                &mut prepared.diagnostics,
                reporter,
                log,
            ) {
                Ok(generated) => generated,
                Err(code) => {
                    report_sink(&prepared.diagnostics, &prepared.sources, reporter, log);
                    return code;
                }
            };
            // Keep the summary the last stdout line in JSON mode.
            report_sink(&prepared.diagnostics, &prepared.sources, reporter, log);
            metadata::print(&generated, &prepared.request, reporter.is_json());
            let code = diagnostic_exit(&prepared.diagnostics);
            if code != 0 {
                return code;
            }
        }
        Mode::Check => unreachable!("check returns from prepare"),
    }
    u8::from(reporter.denied())
}

/// Builds a project for a `stc dap` launch without running an interactive
/// debug loop.
///
/// `Mode::Debug` fixes the build form to debug unless `request` carries an
/// explicit `--release` override, exactly like `stc debug`.
pub(crate) fn build_for_launch(
    manifest: Option<&str>,
    locked: bool,
    diagnostic_paths: DiagnosticPaths,
    request: &Request,
    reporter: &mut Reporter,
    log: Log,
) -> std::result::Result<ProjectBuild, u8> {
    let mut prepared = prepare(
        Mode::Debug,
        manifest,
        locked,
        diagnostic_paths,
        request,
        reporter,
        log,
    )?;
    let code = execute_build(&mut prepared, reporter, log);
    if code != 0 {
        return Err(code);
    }
    Ok(project_build(&prepared))
}

fn project_build(prepared: &Prepared) -> ProjectBuild {
    let out_dir = prepared.request.out_dir.clone().map_or_else(
        || PathBuf::from(build::default_out_dir(prepared.build_mode)),
        PathBuf::from,
    );
    let launcher = build::launcher_path(&out_dir, &prepared.name);
    let root = prepared.graph.root_package();
    ProjectBuild {
        launcher,
        out_dir,
        run: prepared.request.run.clone(),
        manifest: root.manifest.clone(),
        root_dir: root.root_dir.clone(),
    }
}

fn execute_build(prepared: &mut Prepared, reporter: &mut Reporter, log: Log) -> u8 {
    let code = build::run_project(
        &prepared.request,
        &prepared.analysis.hir,
        &prepared.name,
        &prepared.source_files,
        &prepared.source_paths,
        &prepared.sources,
        prepared.fingerprint,
        &mut prepared.diagnostics,
        reporter,
        log,
    );
    report_sink(&prepared.diagnostics, &prepared.sources, reporter, log);
    if code != 0 {
        return code;
    }
    let code = diagnostic_exit(&prepared.diagnostics);
    if code != 0 {
        return code;
    }
    if reporter.denied() {
        return 1;
    }
    0
}

/// Loads, parses, analyzes, and resolves the build request for one command.
/// `check` reports here and returns its exit code through `Err`.
#[allow(clippy::too_many_arguments)]
fn prepare(
    mode: Mode,
    manifest: Option<&str>,
    locked: bool,
    diagnostic_paths: DiagnosticPaths,
    request: &Request,
    reporter: &mut Reporter,
    log: Log,
) -> std::result::Result<Prepared, u8> {
    let manifest_input = manifest.unwrap_or("stc.toml");
    let graph = match load_project(manifest_input, &LoadOptions { locked }) {
        Ok(graph) => graph,
        Err(error) => {
            reporter.diagnostics(&error.diagnostics, &error.sources);
            return Err(2);
        }
    };
    let root = graph.root_package();
    let mut build_mode = select_mode(request, &root.manifest);
    if mode == Mode::Debug && request.mode_override.is_none() {
        // `stc debug` defaults to the debug form regardless of `[build].debug`;
        // `--release` explicitly selects the monitoring-only release form.
        build_mode = build::BuildMode::Debug;
    }
    reporter.merge_manifest_lints(&root.manifest.lints);
    reporter.diagnostics(&graph.warnings, &graph.sources);
    log.info(format!("manifest {manifest_input}"));
    log.info(format!("profile {}", stc_manifest::PROFILE_ID));

    let relaxed = request.relaxed || root.manifest.build.syntax == SyntaxMode::Relaxed;
    let mut sources = SourceMap::new();
    let mut diagnostics = DiagnosticSink::new();
    let mut entries: Vec<SourceEntry> = Vec::new();
    let mut parsed_files = Vec::new();
    let mut ranges = Vec::new();
    let mut line_count = 0_usize;
    let mut lex_time = std::time::Duration::ZERO;
    let mut parse_time = std::time::Duration::ZERO;
    let mut read_error = false;
    for (index, package) in graph.packages.iter().enumerate() {
        let start = parsed_files.len();
        for source in &package.sources {
            let display = match diagnostic_paths {
                DiagnosticPaths::Package => format!("{}/{}", package.name, source.relative),
                DiagnosticPaths::Absolute => absolute_path(&source.path),
            };
            let file = FileId::new(u32::try_from(parsed_files.len()).expect("too many sources"));
            let bytes = match std::fs::read(&source.path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    let mapped = sources.add_file(display, String::new());
                    diagnostics.error(
                        "STC-E-BUILD-002",
                        format!("cannot read `{}`: {error}", source.path.to_string_lossy()),
                        Span::new(mapped, 0, 0),
                    );
                    read_error = true;
                    continue;
                }
            };
            line_count += bytes.iter().filter(|byte| **byte == b'\n').count();
            let lex_start = Instant::now();
            let lexed = tokenize_bytes(file, &bytes, &mut diagnostics);
            let lex_elapsed = lex_start.elapsed();
            lex_time += lex_elapsed;
            let mapped = sources.add_file(display, lexed.text.clone());
            debug_assert_eq!(mapped, file);
            let parse_start = Instant::now();
            let parsed = parse(
                file,
                lexed.text.as_str(),
                &lexed.stream,
                ParseOptions {
                    relaxed_semicolons: relaxed,
                    ..ParseOptions::default()
                },
                &mut diagnostics,
            );
            let parse_elapsed = parse_start.elapsed();
            parse_time += parse_elapsed;
            log.detail(format!(
                "{} lex {} parse {}",
                sources.file_path(file).unwrap_or("<unknown>"),
                duration(lex_elapsed),
                duration(parse_elapsed)
            ));
            entries.push(SourceEntry {
                package: index,
                path: source.path.to_string_lossy().into_owned(),
                relative: source.relative.clone(),
                bytes,
            });
            parsed_files.push(parsed);
        }
        ranges.push((start, parsed_files.len()));
    }

    log.info(format!(
        "sources {} files, {line_count} lines",
        entries.len()
    ));
    log.info(format!(
        "lex {}  parse {}",
        duration(lex_time),
        duration(parse_time)
    ));

    // D-070: do not analyze recovered trees when syntax errors exist.
    if diagnostics.has_errors() || read_error {
        report_sink(&diagnostics, &sources, reporter, log);
        return Err(diagnostic_exit(&diagnostics));
    }

    let packages: Vec<PackageSession<'_>> = graph
        .packages
        .iter()
        .enumerate()
        .map(|(index, package)| PackageSession {
            name: package.name.clone(),
            root_namespace: package.namespace.clone(),
            dependencies: package
                .dependencies
                .iter()
                .map(|dependency| dependency.package.0)
                .collect(),
            files: &parsed_files[ranges[index].0..ranges[index].1],
            is_root: package.is_root,
        })
        .collect();
    let session = ProjectSession {
        packages,
        configuration: root.manifest.project.configuration.clone(),
        stdlib: root.manifest.project.stdlib,
        test_build: false,
        test_files: &[],
    };
    let analyze_start = Instant::now();
    let analysis = stc_sema::analyze_project(&session, &mut diagnostics);
    log.info(format!("analyze {}", elapsed(analyze_start)));

    if mode == Mode::Check {
        report_sink(&diagnostics, &sources, reporter, log);
        let code = diagnostic_exit(&diagnostics);
        return Err(if code == 0 && reporter.denied() {
            1
        } else {
            code
        });
    }
    if diagnostics.has_errors() {
        report_sink(&diagnostics, &sources, reporter, log);
        return Err(diagnostic_exit(&diagnostics));
    }

    let mut request = request.clone();
    if matches!(mode, Mode::Build | Mode::Run | Mode::Debug | Mode::Metadata) {
        // Builds merge the committed rename history (oldest first) with CLI
        // `--rename` entries; chains compose and conflicts are errors.
        match crate::rename::merged_project_renames(&root.root_dir, &request.renames) {
            Ok(renames) => request.renames = renames,
            Err(error) => {
                reporter.plain(error.code, stc_diag::Severity::Error, &error.message);
                return Err(2);
            }
        }
    }
    if request.emit.is_empty() {
        request.emit = root
            .manifest
            .build
            .emit
            .iter()
            .copied()
            .map(Emit::from)
            .collect();
    }
    if mode == Mode::Run || mode == Mode::Debug {
        // `stc run` and `stc debug` always build the launcher pair and its
        // metadata.
        request.emit = [Emit::Exe, Emit::Module, Emit::Metadata]
            .into_iter()
            .collect();
    }
    if request.emit.is_empty() {
        request.emit.insert(Emit::Module);
    }
    request.relaxed = relaxed;
    request.apply_mode(build_mode);
    request.opt_level = root.manifest.build.opt_level;
    request.opt_level_explicit = root.manifest.build.opt_level_explicit;
    request.lib_type = root.manifest.build.lib_type;
    request.manifest_target = root.manifest.build.target.clone();
    request.manifest_clang = root.manifest.build.clang.clone();
    request.manifest_runtime = root.manifest.build.runtime.clone();
    request.project = build::ProjectSettings::from_manifest(&root.manifest);
    let manifest_out_dir_explicit = root.manifest.build.out_dir_explicit;
    if request.out_dir.is_none() {
        let configured = if manifest_out_dir_explicit {
            root.root_dir.join(&root.manifest.build.out_dir)
        } else {
            root.root_dir.join(build::default_out_dir(build_mode))
        };
        request.out_dir = Some(configured.to_string_lossy().into_owned());
    }
    request.out_dir_explicit |= manifest_out_dir_explicit;
    if request.got_manifest.is_none() && !request.cold {
        match root.manifest.online.baseline {
            stc_manifest::Baseline::Explicit => {
                let Some(path) = root.manifest.online.manifest.as_deref() else {
                    reporter.plain(
                        "STC-E-PROJECT-010",
                        stc_diag::Severity::Error,
                        "`[online].baseline = \"explicit\"` requires a `manifest` path",
                    );
                    return Err(2);
                };
                let path = root.root_dir.join(path);
                if !path.is_file() {
                    reporter.plain(
                        "STC-E-ONLINE-009",
                        stc_diag::Severity::Error,
                        &format!(
                            "the explicit online baseline `{}` does not exist",
                            path.display()
                        ),
                    );
                    return Err(2);
                }
                request.got_manifest = Some(path.to_string_lossy().into_owned());
            }
            stc_manifest::Baseline::Active => {
                if let Some(dir) = &root.manifest.runtime.manifest_dir {
                    let store = stc_runtime::store::ManifestStore::open(root.root_dir.join(dir));
                    if let Some(problem) = store.problem() {
                        reporter.plain(
                            "STC-W-ONLINE-007",
                            stc_diag::Severity::Warning,
                            &format!(
                                "the runtime store is invalid ({problem}); building a cold-layout candidate"
                            ),
                        );
                    } else if let Some(path) = store.active_json_path() {
                        log.info(format!("baseline {}", path.display()));
                        request.got_manifest = Some(path.to_string_lossy().into_owned());
                    }
                }
            }
        }
    }
    let name = request.name.clone().unwrap_or_else(|| root.name.clone());
    let source_files: Vec<String> = entries.iter().map(|entry| entry.path.clone()).collect();
    let source_paths: Vec<String> = entries.iter().map(|entry| entry.relative.clone()).collect();
    let fingerprint_sources: Vec<(String, String, Vec<u8>)> = entries
        .iter()
        .map(|entry| {
            (
                graph.packages[entry.package].name.clone(),
                entry.relative.clone(),
                entry.bytes.clone(),
            )
        })
        .collect();
    let fingerprint = match stc_codegen::module::project_source_fingerprint(&fingerprint_sources) {
        Ok(fingerprint) => fingerprint,
        Err(error) => {
            let file = sources.add_file("<project>", String::new());
            diagnostics.error(error.code, error.message, Span::new(file, 0, 0));
            report_sink(&diagnostics, &sources, reporter, log);
            return Err(1);
        }
    };

    Ok(Prepared {
        graph,
        analysis,
        sources,
        diagnostics,
        request,
        name,
        source_files,
        source_paths,
        fingerprint,
        build_mode,
    })
}

fn duration(duration: std::time::Duration) -> String {
    format!("{:.1}ms", duration.as_secs_f64() * 1000.0)
}

fn report_sink(sink: &DiagnosticSink, sources: &SourceMap, reporter: &mut Reporter, log: Log) {
    reporter.diagnostics(sink.diagnostics(), sources);
    if sink.suppressed() > 0 {
        log.status(format!(
            "note: {} further diagnostic(s) suppressed",
            sink.suppressed()
        ));
    }
}

/// Returns 2 for project/lockfile errors, 1 for compile errors, and 0.
fn diagnostic_exit(sink: &DiagnosticSink) -> u8 {
    if sink.suppressed_errors() > 0 {
        return 1;
    }
    let mut any = false;
    let mut manifest_only = true;
    for diagnostic in sink.iter().filter(|diagnostic| diagnostic.is_error()) {
        any = true;
        if !(diagnostic.code.starts_with("STC-E-PROJECT-")
            || diagnostic.code.starts_with("STC-E-LOCK-"))
        {
            manifest_only = false;
        }
    }
    if !any {
        0
    } else if manifest_only {
        2
    } else {
        1
    }
}
