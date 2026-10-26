//! `stc fmt`: fixed-style source formatting over the lossless CST (M6 phase 7).
//!
//! Without paths the command formats the root package's production and test
//! sources discovered through `stc.toml`; explicit paths format those files
//! without loading a manifest. `--check` never rewrites a source file and
//! exits 1 when a file would change. Syntax errors prevent formatting one file
//! (`STC-E-FMT-001`); unreadable or unwritable files report `STC-E-FMT-002`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use stc_diag::{DiagnosticSink, Severity};
use stc_manifest::{LoadOptions, load_project_readonly};
use stc_span::{FileId, SourceMap};

use crate::report::{Log, Reporter, display_path};

/// Runs `stc fmt`.
pub(crate) fn run(
    paths: &[String],
    check: bool,
    manifest: Option<&str>,
    locked: bool,
    reporter: &mut Reporter,
    log: Log,
) -> u8 {
    let files = if paths.is_empty() {
        match project_sources(manifest, locked, reporter, log) {
            Ok(files) => files,
            Err(code) => return code,
        }
    } else {
        paths.iter().map(PathBuf::from).collect()
    };

    let mut sources = SourceMap::new();
    let mut diagnostics = DiagnosticSink::new();
    let mut io_error = false;
    let mut syntax_error = false;
    let mut changed = false;
    for path in files {
        let display = display_path(&path);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                reporter.plain(
                    stc_fmt::IO_ERROR,
                    Severity::Error,
                    &format!("cannot read `{display}`: {error}"),
                );
                io_error = true;
                continue;
            }
        };
        let file = FileId::new(u32::try_from(sources.len()).expect("too many input files"));
        let outcome = stc_fmt::format_bytes(file, &bytes, &mut diagnostics);
        let mapped = sources.add_file(display.clone(), outcome.source);
        debug_assert_eq!(mapped, file);

        let Some(formatted) = outcome.formatted else {
            syntax_error = true;
            continue;
        };
        if formatted.as_bytes() == bytes.as_slice() {
            continue;
        }
        changed = true;
        if check {
            reporter.note(format!("{display}: would be reformatted"));
        } else if let Err(error) = std::fs::write(&path, formatted.as_bytes()) {
            reporter.plain(
                stc_fmt::IO_ERROR,
                Severity::Error,
                &format!("cannot write `{display}`: {error}"),
            );
            io_error = true;
        } else {
            log.info(format!("formatted {display}"));
        }
    }

    reporter.diagnostics(diagnostics.diagnostics(), &sources);
    if diagnostics.suppressed() > 0 {
        log.status(format!(
            "note: {} further diagnostic(s) suppressed",
            diagnostics.suppressed()
        ));
    }

    if io_error {
        2
    } else if syntax_error || (check && changed) || reporter.denied() {
        1
    } else {
        0
    }
}

/// Discovers the root package's production and test sources. Dependencies are
/// separate projects with their own manifest, so they are not reformatted.
fn project_sources(
    manifest: Option<&str>,
    locked: bool,
    reporter: &mut Reporter,
    log: Log,
) -> Result<Vec<PathBuf>, u8> {
    let manifest_input = manifest.unwrap_or("stc.toml");
    let graph = match load_project_readonly(manifest_input, &LoadOptions { locked }) {
        Ok(graph) => graph,
        Err(error) => {
            reporter.diagnostics(&error.diagnostics, &error.sources);
            return Err(2);
        }
    };
    let root = graph.root_package();
    reporter.merge_manifest_lints(&root.manifest.lints);
    reporter.diagnostics(&graph.warnings, &graph.sources);
    log.info(format!("manifest {manifest_input}"));
    let mut files: BTreeSet<PathBuf> = BTreeSet::new();
    for source in root.sources.iter().chain(root.test_sources.iter()) {
        files.insert(source.path.clone());
    }
    let files: Vec<PathBuf> = files.into_iter().collect();
    log.info(format!("sources {} files", files.len()));
    Ok(files)
}
