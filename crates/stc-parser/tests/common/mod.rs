#![allow(dead_code)]

use stc_diag::DiagnosticSink;
use stc_lexer::tokenize;
use stc_parser::{ParseOptions, ParsedFile, parse};
use stc_span::FileId;
use stc_syntax::{SyntaxTree, dump_cst};

pub const FILE: FileId = FileId::new(0);

pub fn parse_with(source: &str, options: ParseOptions) -> (ParsedFile, DiagnosticSink) {
    let mut sink = DiagnosticSink::unlimited();
    let stream = tokenize(FILE, source, &mut sink);
    stream
        .check_invariants(source)
        .unwrap_or_else(|message| panic!("token stream: {message}"));
    let file = parse(FILE, source, &stream, options, &mut sink);
    file.tree()
        .check_invariants()
        .unwrap_or_else(|message| panic!("tree invariant: {message}\n{}", dump_cst(file.tree())));
    assert_eq!(
        file.root().text(),
        source,
        "CST did not reproduce the source:\n{}",
        dump_cst(file.tree())
    );
    assert_eq!(file.tree().source(), source);
    (file, sink)
}

pub fn parse_source(source: &str) -> (ParsedFile, DiagnosticSink) {
    parse_with(source, ParseOptions::default())
}

pub fn parse_relaxed(source: &str) -> (ParsedFile, DiagnosticSink) {
    parse_with(
        source,
        ParseOptions {
            relaxed_semicolons: true,
            ..ParseOptions::default()
        },
    )
}

pub fn parse_clean(source: &str) -> ParsedFile {
    let (file, sink) = parse_source(source);
    assert!(
        !sink.has_errors(),
        "unexpected errors for {source:?}:\n{:#?}\nCST:\n{}",
        sink.diagnostics(),
        dump_cst(file.tree())
    );
    file
}

pub fn parse_relaxed_clean(source: &str) -> ParsedFile {
    let (file, sink) = parse_relaxed(source);
    assert!(
        !sink.has_errors(),
        "unexpected errors for {source:?}:\n{:#?}",
        sink.diagnostics()
    );
    file
}

pub fn codes(sink: &DiagnosticSink) -> Vec<&'static str> {
    sink.iter().map(|diagnostic| diagnostic.code).collect()
}

pub fn assert_error(source: &str, code: &str) {
    let (file, sink) = parse_source(source);
    assert!(
        sink.has_errors(),
        "expected {code} for {source:?}, got no errors\nCST:\n{}",
        dump_cst(file.tree())
    );
    assert!(
        codes(&sink).contains(&code),
        "expected {code} for {source:?}, got {:?}",
        codes(&sink)
    );
}

pub fn tree(source: &str) -> SyntaxTree {
    parse_source(source).0.into_tree()
}

pub fn dump(source: &str) -> String {
    let (file, _) = parse_source(source);
    dump_cst(file.tree())
}
