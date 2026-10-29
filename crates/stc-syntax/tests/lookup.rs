//! Byte-offset lookup tests for [`SyntaxTree`]: tokens, nodes, trivia, `Eof`,
//! offset clamping, and non-ASCII text.

use std::sync::Arc;

use stc_diag::DiagnosticSink;
use stc_lexer::{TriviaKind, tokenize};
use stc_span::{FileId, Span};
use stc_syntax::{SyntaxKind, SyntaxTree, TreeBuilder, build_flat_tree};

const FILE: FileId = FileId::new(0);

fn flat(source: &str) -> SyntaxTree {
    let mut diagnostics = DiagnosticSink::unlimited();
    let stream = tokenize(FILE, source, &mut diagnostics);
    build_flat_tree(FILE, Arc::from(source), &stream)
}

/// Builds `SourceFile > AssignStmt > (Name "x", ":=", "1", ";")` over
/// `x := 1;` with the source whitespace as trivia leaves.
fn nested() -> SyntaxTree {
    let mut builder = TreeBuilder::new(FILE, Arc::from("x := 1;"));
    builder.start_node(SyntaxKind::SourceFile);
    builder.start_node(SyntaxKind::AssignStmt);
    builder.start_node(SyntaxKind::Name);
    builder.token(SyntaxKind::Ident, Span::new(FILE, 0, 1), None);
    builder.finish_node();
    builder.trivia(TriviaKind::Whitespace, Span::new(FILE, 1, 2));
    builder.token(SyntaxKind::PunctAssign, Span::new(FILE, 2, 4), None);
    builder.trivia(TriviaKind::Whitespace, Span::new(FILE, 4, 5));
    builder.token(SyntaxKind::IntLiteral, Span::new(FILE, 5, 6), None);
    builder.token(SyntaxKind::PunctSemicolon, Span::new(FILE, 6, 7), None);
    builder.finish_node();
    builder.finish_node();
    builder.finish()
}

#[test]
fn tokens_are_found_by_half_open_containment() {
    let tree = nested();
    let token = tree.token_at(0).expect("offset 0 is inside `x`");
    assert_eq!(token.kind(), SyntaxKind::Ident);
    assert_eq!(token.text(), "x");

    let assign = tree.token_at(2).expect("offset 2 is inside `:=`");
    assert_eq!(assign.kind(), SyntaxKind::PunctAssign);
    assert_eq!(assign.text(), ":=");

    // A caret on the offset after a token is not inside it.
    assert!(tree.token_at(1).is_none(), "offset 1 is whitespace");
    assert!(tree.token_at(4).is_none(), "offset 4 is whitespace");
    assert!(tree.token_at(7).is_none(), "the manual tree has no Eof");
}

#[test]
fn token_before_finds_the_preceding_token_and_clamps() {
    let tree = nested();
    let token = tree.token_before(1).expect("`x` precedes offset 1");
    assert_eq!(token.text(), "x");
    let token = tree.token_before(2).expect("`x` precedes offset 2");
    assert_eq!(token.text(), "x");
    let token = tree.token_before(7).expect("`;` precedes offset 7");
    assert_eq!(token.kind(), SyntaxKind::PunctSemicolon);
    let token = tree.token_before(1_000).expect("offsets clamp to the end");
    assert_eq!(token.kind(), SyntaxKind::PunctSemicolon);
    assert!(tree.token_before(0).is_none());
}

#[test]
fn node_at_returns_the_token_owner() {
    let tree = nested();
    assert_eq!(tree.node_at(0).kind(), SyntaxKind::Name);
    assert_eq!(tree.node_at(1).kind(), SyntaxKind::Name);
    assert_eq!(tree.node_at(2).kind(), SyntaxKind::AssignStmt);
    assert_eq!(tree.node_at(6).kind(), SyntaxKind::AssignStmt);
    assert_eq!(tree.node_at(1_000).kind(), SyntaxKind::AssignStmt);
}

#[test]
fn missing_tokens_are_never_returned() {
    let mut builder = TreeBuilder::new(FILE, Arc::from("x"));
    builder.start_node(SyntaxKind::SourceFile);
    builder.start_node(SyntaxKind::Name);
    builder.token(SyntaxKind::Ident, Span::new(FILE, 0, 1), None);
    builder.missing_token(SyntaxKind::Ident);
    builder.finish_node();
    builder.finish_node();
    let tree = builder.finish();
    assert_eq!(tree.token_at(0).expect("real token").text(), "x");
    assert!(tree.token_at(1).is_none(), "a zero-width token is skipped");
    assert!(tree.token_before(1).is_some());
}

#[test]
fn eof_is_returned_at_the_end_of_a_flat_tree() {
    let tree = flat("x");
    let eof = tree.token_at(1).expect("offset at the end returns Eof");
    assert_eq!(eof.kind(), SyntaxKind::Eof);
    assert!(eof.span().is_empty());
    // Offsets past the end clamp to the end.
    assert_eq!(tree.token_at(usize::MAX).unwrap().kind(), SyntaxKind::Eof);
    assert_eq!(tree.node_at(1).kind(), SyntaxKind::SourceFile);
}

#[test]
fn empty_and_trivia_only_sources_still_answer_node_lookups() {
    let empty = flat("");
    assert_eq!(empty.token_at(0).unwrap().kind(), SyntaxKind::Eof);
    assert!(empty.token_before(0).is_none());
    assert_eq!(empty.node_at(0).kind(), SyntaxKind::SourceFile);

    let trivia = flat("  \n");
    assert!(trivia.token_at(0).is_none());
    assert!(trivia.token_at(1).is_none());
    assert_eq!(trivia.token_at(3).unwrap().kind(), SyntaxKind::Eof);
    assert_eq!(trivia.node_at(0).kind(), SyntaxKind::SourceFile);
    assert!(trivia.token_before(2).is_none());
}

#[test]
fn non_ascii_trivia_and_literals_use_byte_offsets() {
    let source = "(* é *)\nx := 'ü';\n";
    let tree = flat(source);
    assert_eq!(tree.root().text(), source);

    let comment = source.find('é').expect("comment scalar");
    assert!(tree.token_at(comment).is_none(), "comment text is trivia");
    assert_eq!(tree.node_at(comment).kind(), SyntaxKind::SourceFile);

    // Every byte inside the multi-byte scalar belongs to the same trivia leaf.
    for offset in comment..comment + 'é'.len_utf8() {
        assert!(tree.token_at(offset).is_none(), "offset {offset}");
    }

    let string = source.find("'ü'").expect("string literal");
    let token = tree.token_at(string + 1).expect("inside the literal");
    assert_eq!(token.kind(), SyntaxKind::StringLiteral);
    assert_eq!(token.text(), "'ü'");
    for offset in string..string + "'ü'".len() {
        assert_eq!(
            tree.token_at(offset).expect("literal byte").text(),
            "'ü'",
            "offset {offset}"
        );
    }

    let before_string = tree
        .token_before(string)
        .expect("`:=` precedes the literal");
    assert_eq!(before_string.text(), ":=");
    assert_eq!(tree.token_before(string + 1).unwrap().text(), ":=");
    let eof = tree.token_at(source.len()).expect("Eof after the newline");
    assert_eq!(eof.kind(), SyntaxKind::Eof);
}

#[test]
fn every_byte_offset_answers_without_panicking() {
    let source = "(* ünïcode *)\nx := U#'héllo ${1F600}';\ny := 'ü';\n";
    let tree = flat(source);
    let semicolon = source.rfind(';').expect("statement terminator");
    assert_eq!(
        tree.token_at(semicolon).expect("semicolon").kind(),
        SyntaxKind::PunctSemicolon
    );
    assert_eq!(
        tree.token_before(semicolon)
            .expect("literal precedes `;`")
            .text(),
        "'ü'"
    );
    for offset in 0..=source.len() {
        let _ = tree.token_at(offset);
        let _ = tree.token_before(offset);
        let node = tree.node_at(offset);
        assert_eq!(node.tree().file(), FILE);
        assert!(node.kind().is_node());
    }
    // Offsets past the end clamp rather than panic.
    assert_eq!(
        tree.token_at(source.len() + 64).unwrap().kind(),
        SyntaxKind::Eof
    );
    assert_eq!(tree.token_at(usize::MAX).unwrap().kind(), SyntaxKind::Eof);
}
