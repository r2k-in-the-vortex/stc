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
use crate::report::{Log, Reporter, elapsed};

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
}

struct SourceEntry {
    package: usize,
    path: String,
    relative: String,
    bytes: Vec<u8>,
}

/// Runs a project command.
pub(crate) fn run(
    mode: Mode,
    manifest: Option<&str>,
    locked: bool,
    request: &Request,
    reporter: &mut Reporter,
    log: Log,
) -> u8 {
    let manifest_input = manifest.unwrap_or("stc.toml");
    let graph = match load_project(manifest_input, &LoadOptions { locked }) {
        Ok(graph) => graph,
        Err(error) => {
            reporter.diagnostics(&error.diagnostics, &error.sources);
            return 2;
        }
    };
    let root = graph.root_package();
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
            let display = format!("{}/{}", package.name, source.relative);
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
        return diagnostic_exit(&diagnostics);
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
        return if code == 0 && reporter.denied() {
            1
        } else {
            code
        };
    }
    if diagnostics.has_errors() {
        report_sink(&diagnostics, &sources, reporter, log);
        return diagnostic_exit(&diagnostics);
    }

    let mut request = request.clone();
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
    if mode == Mode::Run {
        // `stc run` always builds the launcher pair and its metadata.
        request.emit = [Emit::Exe, Emit::Module, Emit::Metadata]
            .into_iter()
            .collect();
    }
    if request.emit.is_empty() {
        request.emit.insert(Emit::Module);
    }
    request.relaxed = relaxed;
    request.opt_level = root.manifest.build.opt_level;
    request.lib_type = root.manifest.build.lib_type;
    request.manifest_target = root.manifest.build.target.clone();
    request.manifest_clang = root.manifest.build.clang.clone();
    request.manifest_runtime = root.manifest.build.runtime.clone();
    request.project = build::ProjectSettings::from_manifest(&root.manifest);
    if request.out_dir.is_none() {
        request.out_dir = Some(
            root.root_dir
                .join(&root.manifest.build.out_dir)
                .to_string_lossy()
                .into_owned(),
        );
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
            return 1;
        }
    };

    match mode {
        Mode::Build | Mode::Run => {
            let code = build::run_project(
                &request,
                &analysis.hir,
                &name,
                &source_files,
                &source_paths,
                fingerprint,
                &mut diagnostics,
                reporter,
                log,
            );
            report_sink(&diagnostics, &sources, reporter, log);
            if code != 0 {
                return code;
            }
            let code = diagnostic_exit(&diagnostics);
            if code != 0 {
                return code;
            }
            if reporter.denied() {
                return 1;
            }
            if mode == Mode::Run {
                let out_dir = request
                    .out_dir
                    .clone()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("target/stc/debug"));
                let launcher = build::launcher_path(&out_dir, &name);
                return crate::run::launch(
                    &launcher,
                    &request.run,
                    &root.manifest,
                    &root.root_dir,
                    reporter,
                    log,
                );
            }
        }
        Mode::Metadata => {
            let generated = match build::generate_project(
                &request,
                &analysis.hir,
                &name,
                &source_files,
                &source_paths,
                fingerprint,
                &mut diagnostics,
                reporter,
                log,
            ) {
                Ok(generated) => generated,
                Err(code) => {
                    report_sink(&diagnostics, &sources, reporter, log);
                    return code;
                }
            };
            // Keep the summary the last stdout line in JSON mode.
            report_sink(&diagnostics, &sources, reporter, log);
            metadata::print(&generated, &request, reporter.is_json());
            let code = diagnostic_exit(&diagnostics);
            if code != 0 {
                return code;
            }
        }
        Mode::Check => unreachable!("check returns before code generation"),
    }
    u8::from(reporter.denied())
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
