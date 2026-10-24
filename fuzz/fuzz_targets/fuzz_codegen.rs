#![no_main]

use libfuzzer_sys::fuzz_target;
use stc_diag::DiagnosticSink;
use stc_parser::{ParseOptions, parse};
use stc_span::FileId;

fn generate_stub(analysis: &stc_sema::Analysis) {
    let sources = vec!["fuzz.st".to_string()];
    let options = stc_codegen::Options {
        artifact_name: "fuzz",
        target: None,
        debug: false,
        emit_metadata: true,
        mode: stc_codegen::Mode::Executable,
        sources: &sources,
    };
    match stc_codegen::generate(&analysis.hir, &options) {
        Ok(output) => {
            assert!(output.module.len() < 128 * 1024 * 1024);
            assert!(output.spans.windows(2).all(|pair| pair[0].id < pair[1].id));
            let second =
                stc_codegen::generate(&analysis.hir, &options).expect("deterministic generation");
            assert_eq!(output.module, second.module);
            assert_eq!(
                output.metadata.to_json().render(),
                second.metadata.to_json().render()
            );
        }
        Err(errors) => {
            assert!(!errors.is_empty());
            assert!(errors.len() < 10_000);
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let mut diagnostics = DiagnosticSink::with_limit(200);
    let lexed = stc_lexer::tokenize_bytes(FileId::new(0), data, &mut diagnostics);
    let parsed = parse(
        FileId::new(0),
        lexed.text.as_str(),
        &lexed.stream,
        ParseOptions {
            relaxed_semicolons: data.first().is_some_and(|byte| byte & 1 == 1),
            max_depth: 256,
        },
        &mut diagnostics,
    );
    if diagnostics.has_errors() {
        return;
    }
    let analysis = stc_sema::analyze(&[parsed.clone()], &mut diagnostics);
    if diagnostics.has_errors() {
        return;
    }
    analysis
        .hir
        .check_invariants()
        .expect("HIR invariants after analysis");
    generate_stub(&analysis);
});
