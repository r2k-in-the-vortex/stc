// Shared by several integration-test binaries; each binary uses a subset.
#![allow(dead_code)]

use stc_diag::DiagnosticSink;
use stc_lexer::{LexedSource, Token, TokenKind, TokenStream, tokenize, tokenize_bytes};
use stc_span::FileId;

pub const FILE: FileId = FileId::new(0);

pub fn lex(source: &str) -> (TokenStream, DiagnosticSink) {
    let mut sink = DiagnosticSink::unlimited();
    let stream = tokenize(FILE, source, &mut sink);
    stream.check_invariants(source).unwrap_or_else(|message| {
        panic!("token stream invariant failed: {message}\nsource: {source:?}")
    });
    assert_eq!(
        stream.reconstruct(source),
        source,
        "tokens and trivia did not reproduce the source"
    );
    (stream, sink)
}

pub fn lex_clean(source: &str) -> TokenStream {
    let (stream, sink) = lex(source);
    assert!(
        sink.is_empty(),
        "expected no diagnostics for {source:?}, got {:?}",
        sink.diagnostics()
    );
    stream
}

pub fn lex_bytes(bytes: &[u8]) -> (LexedSource, DiagnosticSink) {
    let mut sink = DiagnosticSink::unlimited();
    let lexed = tokenize_bytes(FILE, bytes, &mut sink);
    lexed
        .stream
        .check_invariants(&lexed.text)
        .unwrap_or_else(|message| panic!("token stream invariant failed: {message}"));
    assert_eq!(lexed.stream.reconstruct(&lexed.text), lexed.text);
    (lexed, sink)
}

pub fn tokens_without_eof(stream: &TokenStream) -> &[Token] {
    let (last, rest) = stream.tokens.split_last().expect("stream has Eof");
    assert!(last.kind.is_eof());
    rest
}

pub fn clean_kinds(source: &str) -> Vec<TokenKind> {
    let stream = lex_clean(source);
    tokens_without_eof(&stream)
        .iter()
        .map(|token| token.kind.clone())
        .collect()
}

pub fn texts(source: &str) -> Vec<String> {
    let (stream, _) = lex(source);
    tokens_without_eof(&stream)
        .iter()
        .map(|token| token.text(source).to_string())
        .collect()
}

pub fn first_token(source: &str) -> Token {
    let (stream, _) = lex(source);
    tokens_without_eof(&stream)
        .first()
        .cloned()
        .expect("at least one token")
}

pub fn has_code(sink: &DiagnosticSink, code: &str) -> bool {
    sink.iter().any(|diagnostic| diagnostic.code == code)
}

pub fn assert_code(source: &str, code: &str) {
    let (_, sink) = lex(source);
    assert!(
        has_code(&sink, code),
        "expected {code} for {source:?}, got {:?}",
        sink.diagnostics()
    );
}
