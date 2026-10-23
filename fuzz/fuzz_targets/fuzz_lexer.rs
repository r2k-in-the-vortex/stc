#![no_main]

use libfuzzer_sys::fuzz_target;

// STC-LEX-002: tokenizing arbitrary bytes must never panic or hang. The
// token/trivia stream must remain lossless and cover the decoded input exactly
// once (STC-LEX-004).
fuzz_target!(|data: &[u8]| {
    let mut diagnostics = stc_diag::DiagnosticSink::new();
    let lexed = stc_lexer::tokenize_bytes(stc_span::FileId::new(0), data, &mut diagnostics);

    if let Err(message) = lexed.stream.check_invariants(&lexed.text) {
        panic!("lexer invariant violated: {message}");
    }
    if lexed.stream.reconstruct(&lexed.text) != lexed.text {
        panic!("lossless reconstruction failed");
    }
    if lexed.stream.eof().is_none() {
        panic!("token stream has no Eof token");
    }
});
