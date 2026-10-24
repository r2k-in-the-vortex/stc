mod common;

use common::parse_source;
use stc_syntax::{dump_cst, dump_typed_ast};

const FILES: &[(&str, &str)] = &[
    ("empty", include_str!("corpus/empty.st")),
    ("function", include_str!("corpus/function.st")),
    ("types", include_str!("corpus/types.st")),
    ("oop", include_str!("corpus/oop.st")),
    ("statements", include_str!("corpus/statements.st")),
    ("configuration", include_str!("corpus/configuration.st")),
    ("pragmas", include_str!("corpus/pragmas.st")),
    ("expressions", include_str!("corpus/expressions.st")),
    ("comments", include_str!("corpus/comments.st")),
    ("errors", include_str!("corpus/errors.st")),
];

#[test]
fn corpus_round_trips_and_valid_files_have_no_errors() {
    for (name, source) in FILES {
        let (file, sink) = parse_source(source);
        assert_eq!(file.root().text(), *source, "{name}");
        if *name != "errors" {
            assert!(
                !sink.has_errors(),
                "{name} should parse cleanly, got {:?}",
                sink.diagnostics()
            );
        } else {
            assert!(sink.has_errors(), "{name} should report errors");
        }
    }
}

#[test]
fn bom_and_crlf_variants_round_trip() {
    for (name, source) in FILES {
        let bom = format!("\u{feff}{source}");
        let (file, _) = parse_source(&bom);
        assert_eq!(file.root().text(), bom, "{name} with BOM");

        let crlf = source.replace('\n', "\r\n");
        let (file, _) = parse_source(&crlf);
        assert_eq!(file.root().text(), crlf, "{name} with CRLF");
    }
}

#[test]
fn very_deep_operator_chains_are_lossless_and_dumpable() {
    let terms = 20_000;
    let source = format!(
        "PROGRAM P\nVAR a : DINT; END_VAR\na := 1{};\nEND_PROGRAM\n",
        " + 1".repeat(terms)
    );
    let (file, sink) = parse_source(&source);
    assert!(!sink.has_errors(), "{:?}", sink.diagnostics());
    assert_eq!(file.root().text(), source);
    let cst = dump_cst(file.tree());
    assert!(cst.contains("BinaryExpr"));
    let ast = dump_typed_ast(file.tree());
    assert!(ast.contains("BinaryExpr"));
}

#[test]
fn corpus_trees_satisfy_the_cst_contract() {
    for (name, source) in FILES {
        let (file, _) = parse_source(source);
        let tree = file.tree();
        tree.check_invariants()
            .unwrap_or_else(|message| panic!("{name}: {message}"));
        assert_eq!(tree.root().text(), *source);
    }
}
