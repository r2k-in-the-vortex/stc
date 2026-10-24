#![no_main]

use libfuzzer_sys::fuzz_target;
use stc_diag::DiagnosticSink;
use stc_parser::{ParseOptions, parse};
use stc_span::FileId;

fuzz_target!(|data: &[u8]| {
    let mut diagnostics = DiagnosticSink::with_limit(200);
    let lexed = stc_lexer::tokenize_bytes(FileId::new(0), data, &mut diagnostics);
    lexed
        .stream
        .check_invariants(&lexed.text)
        .expect("lexer invariants");
    let relaxed = data.first().is_some_and(|byte| byte & 1 == 1);
    let parsed = parse(
        FileId::new(0),
        lexed.text.as_str(),
        &lexed.stream,
        ParseOptions {
            relaxed_semicolons: relaxed,
            max_depth: 256,
        },
        &mut diagnostics,
    );
    parsed.tree().check_invariants().expect("CST invariants");
    assert_eq!(
        parsed.root().text(),
        lexed.text,
        "the CST must reproduce the decoded source"
    );
    let analysis = stc_sema::analyze(&[parsed], &mut diagnostics);
    analysis
        .hir
        .check_invariants()
        .expect("HIR invariants after analysis");
});
