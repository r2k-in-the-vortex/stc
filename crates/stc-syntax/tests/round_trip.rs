use std::sync::Arc;

use stc_diag::DiagnosticSink;
use stc_lexer::{tokenize, tokenize_bytes};
use stc_span::FileId;
use stc_syntax::{build_flat_tree, dump_cst};

const FILE: FileId = FileId::new(0);

fn flat(source: &str) -> stc_syntax::SyntaxTree {
    let mut diagnostics = DiagnosticSink::unlimited();
    let stream = tokenize(FILE, source, &mut diagnostics);
    stream
        .check_invariants(source)
        .unwrap_or_else(|message| panic!("{message}"));
    let tree = build_flat_tree(FILE, Arc::from(source), &stream);
    tree.check_invariants()
        .unwrap_or_else(|message| panic!("{message}\n{}", dump_cst(&tree)));
    tree
}

#[test]
fn cst_round_trips_every_byte() {
    let cases = [
        "",
        "\u{feff}",
        "FUNCTION Add : DINT\n  Add := 1 + 2;\nEND_FUNCTION\n",
        "VAR_GLOBAL\r\n  x AT %IX0.0 : BOOL;\r\nEND_VAR\r\n",
        "// line comment\n(* block (* nested *) *)\n/* other */\n",
        "{stc:test {\"cycles\": 25}}\nPROGRAM P END_PROGRAM\n",
        "x := U#'héllo ${1F600}';\n",
        "VAR x : INT; (* never closed",
        "VAR x : INT; {stc:test",
        "@",
        "\n\n   \t",
        "a.%X0 := 0;",
    ];
    for source in cases {
        let tree = flat(source);
        assert_eq!(tree.root().text(), source, "source: {source:?}");
        assert_eq!(tree.source(), source);
    }
}

#[test]
fn invalid_utf8_is_preserved_after_replacement() {
    let bytes = b"VAR x : INT; // \xFF\n";
    let mut diagnostics = DiagnosticSink::unlimited();
    let lexed = tokenize_bytes(FILE, bytes, &mut diagnostics);
    let tree = build_flat_tree(FILE, Arc::from(lexed.text.as_str()), &lexed.stream);
    tree.check_invariants().unwrap();
    assert_eq!(tree.root().text(), lexed.text);
    assert!(lexed.text.contains('\u{fffd}'));
}

#[test]
fn cst_text_of_each_node_is_its_span_slice() {
    let source = "FUNCTION F : INT\n(* c *)\n  F := 1;\nEND_FUNCTION\n";
    let tree = flat(source);
    let mut pending = vec![tree.root()];
    while let Some(node) = pending.pop() {
        assert_eq!(
            node.text(),
            &source[node.span().start as usize..node.span().end as usize]
        );
        pending.extend(node.child_nodes());
    }
}

#[test]
fn every_byte_is_covered_exactly_once() {
    let source = "\u{feff}FUNCTION F\n  F := 1;\nEND_FUNCTION\n// tail\n";
    let tree = flat(source);
    let mut cursor = 0;
    let mut pending: Vec<stc_syntax::SyntaxElement> = tree.root().children();
    pending.reverse();
    while let Some(element) = pending.pop() {
        if !element.is_trivia() {
            let span = element.span();
            assert_eq!(span.start, cursor, "gap or overlap at {cursor}");
            cursor = span.end;
        } else {
            let span = element.span();
            assert_eq!(span.start, cursor);
            cursor = span.end;
        }
        if let stc_syntax::SyntaxElement::Node(node) = element {
            let mut children = node.children();
            children.reverse();
            pending.extend(children);
        }
    }
    assert_eq!(cursor, source.len() as u32);
}
