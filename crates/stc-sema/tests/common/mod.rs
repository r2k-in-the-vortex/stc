#![allow(dead_code)]

use stc_diag::DiagnosticSink;
use stc_lexer::tokenize_bytes;
use stc_parser::{ParseOptions, ParsedFile, parse};
use stc_sema::{Analysis, analyze};
use stc_span::FileId;

pub fn analyze_sources(sources: &[&str]) -> (Analysis, DiagnosticSink) {
    let mut sink = DiagnosticSink::unlimited();
    let mut files = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let file = FileId::new(u32::try_from(index).expect("file limit"));
        let lexed = tokenize_bytes(file, source.as_bytes(), &mut sink);
        lexed
            .stream
            .check_invariants(&lexed.text)
            .unwrap_or_else(|message| panic!("token stream: {message}"));
        let parsed = parse(
            file,
            lexed.text.as_str(),
            &lexed.stream,
            ParseOptions::default(),
            &mut sink,
        );
        parsed
            .tree()
            .check_invariants()
            .unwrap_or_else(|message| panic!("tree invariant: {message}"));
        assert!(
            !sink.has_errors(),
            "unexpected parse errors for:\n{source}\n{:?}",
            sink.diagnostics()
        );
        files.push(parsed);
    }
    let analysis = analyze(&files, &mut sink);
    analysis
        .hir
        .check_invariants()
        .unwrap_or_else(|message| panic!("HIR invariant: {message}"));
    (analysis, sink)
}

pub fn analyze_source(source: &str) -> (Analysis, DiagnosticSink) {
    analyze_sources(&[source])
}

pub fn assert_clean(source: &str) -> Analysis {
    let (analysis, sink) = analyze_source(source);
    let errors: Vec<_> = sink
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .collect();
    assert!(
        errors.is_empty(),
        "unexpected errors for:\n{source}\n{errors:#?}"
    );
    analysis
}

pub fn codes(sink: &DiagnosticSink) -> Vec<&'static str> {
    sink.iter().map(|diagnostic| diagnostic.code).collect()
}

pub fn error_codes(sink: &DiagnosticSink) -> Vec<&'static str> {
    sink.iter()
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| diagnostic.code)
        .collect()
}

pub fn assert_error(source: &str, code: &str) -> (Analysis, DiagnosticSink) {
    let (analysis, sink) = analyze_source(source);
    let codes = error_codes(&sink);
    assert!(
        codes.contains(&code),
        "expected {code} for:\n{source}\ngot {codes:?}\n{:#?}",
        sink.diagnostics()
    );
    (analysis, sink)
}

pub fn assert_errors(source: &str, expected: &[&str]) -> (Analysis, DiagnosticSink) {
    let (analysis, sink) = analyze_source(source);
    let codes = error_codes(&sink);
    for code in expected {
        assert!(
            codes.contains(code),
            "expected {code} for:\n{source}\ngot {codes:?}\n{:#?}",
            sink.diagnostics()
        );
    }
    (analysis, sink)
}

pub fn assert_no_error(source: &str) -> DiagnosticSink {
    let (_, sink) = analyze_source(source);
    let errors = error_codes(&sink);
    assert!(
        errors.is_empty(),
        "unexpected errors for:\n{source}\ngot {errors:?}\n{:#?}",
        sink.diagnostics()
    );
    sink
}

pub fn assert_warning(source: &str, code: &str) -> DiagnosticSink {
    let (_, sink) = analyze_source(source);
    let codes = codes(&sink);
    assert!(
        codes.contains(&code),
        "expected warning {code} for:
{source}
got {codes:?}
{:#?}",
        sink.diagnostics()
    );
    sink
}

pub fn parse_only(source: &str) -> ParsedFile {
    let mut sink = DiagnosticSink::unlimited();
    let lexed = tokenize_bytes(FileId::new(0), source.as_bytes(), &mut sink);
    parse(
        FileId::new(0),
        lexed.text.as_str(),
        &lexed.stream,
        ParseOptions::default(),
        &mut sink,
    )
}

/// Analyzes one package as a test build: `{stc:test}` units are included and
/// the test-only prelude resolves.
pub fn analyze_test_source(source: &str) -> (Analysis, DiagnosticSink) {
    let mut sink = DiagnosticSink::unlimited();
    let file = FileId::new(0);
    let lexed = tokenize_bytes(file, source.as_bytes(), &mut sink);
    let parsed = parse(
        file,
        lexed.text.as_str(),
        &lexed.stream,
        ParseOptions::default(),
        &mut sink,
    );
    assert!(!sink.has_errors(), "parse errors for:\n{source}");
    let files = vec![parsed];
    let session = stc_sema::ProjectSession {
        packages: vec![stc_sema::PackageSession::new(
            "_root", "_root", &files, true,
        )],
        configuration: None,
        stdlib: true,
        test_build: true,
        test_files: &[],
    };
    let analysis = stc_sema::analyze_project(&session, &mut sink);
    analysis
        .hir
        .check_invariants()
        .unwrap_or_else(|message| panic!("HIR invariant: {message}"));
    (analysis, sink)
}

pub fn assert_test_clean(source: &str) {
    let (_, sink) = analyze_test_source(source);
    let errors = error_codes(&sink);
    assert!(
        errors.is_empty(),
        "unexpected test-build errors for:\n{source}\ngot {errors:?}\n{:#?}",
        sink.diagnostics()
    );
}

pub fn assert_test_error(source: &str, code: &str) {
    let (_, sink) = analyze_test_source(source);
    let codes = error_codes(&sink);
    assert!(
        codes.contains(&code),
        "expected {code} for:\n{source}\ngot {codes:?}\n{:#?}",
        sink.diagnostics()
    );
}

/// Analyzes a production file plus one `[test].sources` file.
pub fn assert_test_source_clean(production: &str, test_source: &str) {
    let mut sink = DiagnosticSink::unlimited();
    let mut files = Vec::new();
    for (index, source) in [production, test_source].iter().enumerate() {
        let file = FileId::new(u32::try_from(index).expect("file limit"));
        let lexed = tokenize_bytes(file, source.as_bytes(), &mut sink);
        let parsed = parse(
            file,
            lexed.text.as_str(),
            &lexed.stream,
            ParseOptions::default(),
            &mut sink,
        );
        files.push(parsed);
    }
    let test_files = vec![files[1].clone()];
    let session = stc_sema::ProjectSession {
        packages: vec![stc_sema::PackageSession::new(
            "_root", "_root", &files, true,
        )],
        configuration: None,
        stdlib: true,
        test_build: true,
        test_files: &test_files,
    };
    let analysis = stc_sema::analyze_project(&session, &mut sink);
    analysis
        .hir
        .check_invariants()
        .unwrap_or_else(|message| panic!("HIR invariant: {message}"));
    let errors = error_codes(&sink);
    assert!(
        errors.is_empty(),
        "unexpected test-source errors:\n{test_source}\ngot {errors:?}\n{:#?}",
        sink.diagnostics()
    );
}
