//! `stc hir-dump`: emit or verify the canonical core-HIR dump (P0.6).
//!
//! Emit mode builds one in-memory package from `<file>...`, analyzes it, and
//! writes the versioned JSON dump. Verify mode reads a dump and checks its
//! format, version, schema, and feature/status consistency without executing
//! it. The dump specification is `specification/Verification/HIR-Dump.md`.

use std::fs;
use std::time::Instant;

use serde_json::json;
use stc_diag::DiagnosticSink;
use stc_hir::{CoreDumpSummary, dump_core_hir, verify_core_dump};
use stc_lexer::tokenize_bytes;
use stc_parser::{ParseOptions, parse};
use stc_span::{FileId, SourceMap};

use crate::cli::Invocation;
use crate::report::{Format, Log, Reporter, elapsed};

/// Runs `stc hir-dump`.
pub(crate) fn run(invocation: &Invocation, reporter: &mut Reporter, log: Log) -> u8 {
    if let Some(path) = &invocation.hir_dump.verify {
        verify(path, invocation)
    } else {
        emit(invocation, reporter, log)
    }
}

fn emit(invocation: &Invocation, _reporter: &mut Reporter, log: Log) -> u8 {
    let mut paths = invocation.paths.clone();
    paths.sort();

    let mut sources = SourceMap::new();
    let mut diagnostics = DiagnosticSink::new();
    let mut parsed_files = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("stc: cannot read `{path}`: {error}");
                return 2;
            }
        };
        let file = FileId::new(u32::try_from(index).expect("too many input files"));
        let lexed = tokenize_bytes(file, &bytes, &mut diagnostics);
        let mapped = sources.add_file(path.clone(), lexed.text.clone());
        debug_assert_eq!(mapped, file);
        parsed_files.push(parse(
            file,
            lexed.text.as_str(),
            &lexed.stream,
            ParseOptions {
                relaxed_semicolons: invocation.relaxed,
                ..ParseOptions::default()
            },
            &mut diagnostics,
        ));
    }
    log.detail(format!("sources {} file(s)", paths.len()));

    // Diagnostics stay on stderr so the dump is the only stdout payload.
    let mut reporter = Reporter::new(
        Format::Human,
        invocation.color,
        invocation.quiet,
        &invocation.lints,
    );
    if diagnostics.has_errors() {
        reporter.diagnostics(diagnostics.diagnostics(), &sources);
        return 1;
    }
    let analyze_start = Instant::now();
    let analysis = stc_sema::analyze(&parsed_files, &mut diagnostics);
    log.info(format!("analyze {}", elapsed(analyze_start)));
    reporter.diagnostics(diagnostics.diagnostics(), &sources);
    if diagnostics.has_errors() || reporter.denied() {
        return 1;
    }

    let dump = dump_core_hir(&analysis.hir, &sources);
    let summary = match verify_core_dump(&dump) {
        Ok(summary) => summary,
        Err(error) => {
            eprintln!("stc: internal HIR dump self-check failed: {error}");
            return 3;
        }
    };
    match &invocation.hir_dump.output {
        Some(path) => {
            if let Err(error) = fs::write(path, &dump) {
                eprintln!("stc: cannot write `{path}`: {error}");
                return 2;
            }
            log.info(format!("wrote {path}"));
        }
        None => print!("{dump}"),
    }
    if summary.accepted() {
        0
    } else {
        eprintln!(
            "stc: core HIR rejected ({} feature(s)): {}",
            summary.features.len(),
            summary.features.join(", ")
        );
        1
    }
}

fn verify(path: &str, invocation: &Invocation) -> u8 {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("stc: cannot read `{path}`: {error}");
            return 2;
        }
    };
    let summary = match verify_core_dump(&text) {
        Ok(summary) => summary,
        Err(error) => {
            eprintln!("stc: `{path}`: {error}");
            return 2;
        }
    };
    if invocation.format.is_json() {
        println!("{}", json_summary(&summary));
    } else {
        println!("format {} {}", summary.format, summary.format_version);
        println!("semantics {}", summary.semantics_version);
        println!("profile {}", summary.profile);
        println!("package {}", summary.package);
        println!("status {}", summary.status);
        for tag in &summary.features {
            println!("feature {tag}");
        }
    }
    if summary.accepted() { 0 } else { 1 }
}

fn json_summary(summary: &CoreDumpSummary) -> serde_json::Value {
    json!({
        "format": summary.format,
        "formatVersion": summary.format_version,
        "semanticsVersion": summary.semantics_version,
        "profile": summary.profile,
        "package": summary.package,
        "status": summary.status,
        "features": summary.features,
    })
}
