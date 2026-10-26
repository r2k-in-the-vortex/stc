//! Project mode: load `stc.toml`, compile the package graph in order, and
//! report package-qualified diagnostics (M6 phase 2, D-153).
//!
//! `stc check`/`stc build` without input files operate on the project
//! manifest; passing input files keeps the manifest-free bridge in
//! `main.rs`/`build.rs`.

use stc_diag::{Diagnostic, DiagnosticSink, render_human, render_json};
use stc_lexer::tokenize_bytes;
use stc_manifest::{LoadOptions, load_project};
use stc_parser::{ParseOptions, parse};
use stc_sema::{PackageSession, ProjectSession};
use stc_span::{FileId, SourceMap, Span};

use crate::build::{self, Emit, Request};

struct SourceEntry {
    package: usize,
    path: String,
    relative: String,
    bytes: Vec<u8>,
}

/// Runs a project `check` or `build`.
pub(crate) fn run(
    check_only: bool,
    manifest: Option<&str>,
    relaxed: bool,
    json: bool,
    request: &Request,
) -> u8 {
    let manifest_input = manifest.unwrap_or("stc.toml");
    let graph = match load_project(manifest_input, &LoadOptions::default()) {
        Ok(graph) => graph,
        Err(error) => {
            emit(&error.diagnostics, &error.sources, json);
            return 1;
        }
    };
    emit(&graph.warnings, &graph.sources, json);

    let mut sources = SourceMap::new();
    let mut diagnostics = DiagnosticSink::new();
    let mut entries: Vec<SourceEntry> = Vec::new();
    let mut parsed_files = Vec::new();
    let mut ranges = Vec::new();
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
                    emit_sink(&diagnostics, &sources, json);
                    return 1;
                }
            };
            let lexed = tokenize_bytes(file, &bytes, &mut diagnostics);
            let mapped = sources.add_file(display, lexed.text.clone());
            debug_assert_eq!(mapped, file);
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

    // D-070: do not analyze recovered trees when syntax errors exist.
    if diagnostics.has_errors() {
        emit_sink(&diagnostics, &sources, json);
        return 1;
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
    let root = graph.root_package();
    let session = ProjectSession {
        packages,
        configuration: root.manifest.project.configuration.clone(),
        stdlib: root.manifest.project.stdlib,
    };
    let analysis = stc_sema::analyze_project(&session, &mut diagnostics);

    if check_only {
        emit_sink(&diagnostics, &sources, json);
        return u8::from(diagnostics.has_errors());
    }
    if diagnostics.has_errors() {
        emit_sink(&diagnostics, &sources, json);
        return 1;
    }

    let mut request = request.clone();
    if request.emit.is_empty() {
        request.emit.insert(Emit::Module);
    }
    if request.out_dir.is_none() {
        request.out_dir = Some(
            root.root_dir
                .join("target")
                .join("stc")
                .join("debug")
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
            emit_sink(&diagnostics, &sources, json);
            return 1;
        }
    };

    let code = build::run_project(
        &request,
        &analysis.hir,
        &name,
        &source_files,
        &source_paths,
        fingerprint,
        &mut diagnostics,
        json,
    );
    emit_sink(&diagnostics, &sources, json);
    if code != 0 {
        return code;
    }
    u8::from(diagnostics.has_errors())
}

fn emit(diagnostics: &[Diagnostic], sources: &SourceMap, json: bool) {
    for diagnostic in diagnostics {
        if json {
            println!("{}", render_json(diagnostic, sources));
        } else {
            eprint!("{}", render_human(diagnostic, sources));
        }
    }
}

fn emit_sink(sink: &DiagnosticSink, sources: &SourceMap, json: bool) {
    emit(sink.diagnostics(), sources, json);
    if !json && sink.suppressed() > 0 {
        eprintln!(
            "note: {} further diagnostic(s) suppressed",
            sink.suppressed()
        );
    }
}
