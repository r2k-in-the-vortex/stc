mod common;

use common::{codes, parse_source};
use stc_syntax::SyntaxKind;
use stc_syntax::ast::{AstNode, Item, SourceFile};

#[test]
fn multiple_independent_errors_produce_multiple_diagnostics() {
    let source = r"
PROGRAM Bad
VAR
  x : INT
  y : ;
END_VAR
  a := ;
  IF x THEN
    b := 1;
  END_WHILE;
  CASE x OF
    1 : c := 1;
  END_CASE;
END_PROGRAM
";
    let (file, sink) = parse_source(source);
    let found = codes(&sink);
    assert!(
        sink.error_count() >= 4,
        "expected multiple errors, got {found:?}"
    );
    for expected in [
        "STC-E-GRAM-107",
        "STC-E-GRAM-106",
        "STC-E-GRAM-105",
        "STC-E-GRAM-108",
    ] {
        assert!(found.contains(&expected), "missing {expected} in {found:?}");
    }
    let errors = count_kind(file.tree().root(), SyntaxKind::Error);
    assert!(errors > 0, "expected Error nodes in the recovered CST");
    assert_eq!(file.root().text(), source);
}

#[test]
fn recovery_keeps_later_items_and_statements() {
    let source = r"
PROGRAM First
  a := ;
  b := 2;
END_PROGRAM
PROGRAM Second
  c := 3;
END_PROGRAM
";
    let (file, sink) = parse_source(source);
    assert!(sink.has_errors());
    let items = SourceFile::cast(&file.root()).unwrap().items();
    let programs: Vec<_> = items
        .iter()
        .filter_map(|item| match item {
            Item::Program(program) => Some(program),
            _ => None,
        })
        .collect();
    assert_eq!(programs.len(), 2, "later items must survive recovery");
    let second = programs[1].body().unwrap().statements();
    assert_eq!(second.len(), 1, "the second program body must be parsed");
}

#[test]
fn mismatched_end_reports_both_spans() {
    let source = "FUNCTION F : INT\n  F := 1;\nEND_PROGRAM\n";
    let (_, sink) = parse_source(source);
    let diagnostic = sink
        .iter()
        .find(|diagnostic| diagnostic.code == "STC-E-GRAM-108")
        .expect("mismatched-end diagnostic");
    assert_eq!(diagnostic.labels.len(), 1, "{diagnostic:#?}");
    let opener = diagnostic.labels[0].span;
    assert_eq!(
        &source[opener.start as usize..opener.end as usize],
        "FUNCTION"
    );
    let found = diagnostic.primary;
    assert_eq!(
        &source[found.start as usize..found.end as usize],
        "END_PROGRAM"
    );
}

#[test]
fn missing_terminator_points_at_the_missing_position() {
    let source = "PROGRAM P\n  x := 1\n  y := 2;\nEND_PROGRAM\n";
    let (file, sink) = parse_source(source);
    let diagnostic = sink
        .iter()
        .find(|diagnostic| diagnostic.code == "STC-E-GRAM-107")
        .expect("missing-semicolon diagnostic");
    assert!(diagnostic.primary.start >= "PROGRAM P\n  x := 1".len() as u32);
    let statements = match &SourceFile::cast(&file.root()).unwrap().items()[0] {
        Item::Program(program) => program.body().unwrap().statements(),
        _ => unreachable!(),
    };
    assert_eq!(statements.len(), 2, "the next statement must still parse");
}

#[test]
fn bad_expression_delimiters_recover_at_the_delimiter() {
    for source in [
        "PROGRAM P\n  a := (1 + 2];\nEND_PROGRAM\n",
        "PROGRAM P\n  a := arr[1;\nEND_PROGRAM\n",
        "PROGRAM P\n  a := f(1;\nEND_PROGRAM\n",
    ] {
        let (file, sink) = parse_source(source);
        assert!(sink.has_errors(), "{source}");
        assert_eq!(file.root().text(), source);
    }
}

#[test]
fn malformed_declarations_do_not_stop_the_file() {
    let source = r"
PROGRAM Broken
VAR
  1 : INT;
  good : INT;
END_VAR
  good := 1;
END_PROGRAM
";
    let (file, sink) = parse_source(source);
    assert!(sink.has_errors());
    let items = SourceFile::cast(&file.root()).unwrap().items();
    let Item::Program(program) = &items[0] else {
        panic!("expected program");
    };
    assert_eq!(program.var_sections()[0].declarations().len(), 1);
    assert_eq!(program.body().unwrap().statements().len(), 1);
}

#[test]
fn every_prefix_of_a_valid_source_parses() {
    let source = r"
FUNCTION Add : DINT
VAR_INPUT
  a, b : DINT;
END_VAR
  Add := a + b;
END_FUNCTION

PROGRAM Main
VAR
  x : INT;
END_VAR
  IF x > 0 THEN
    x := x - 1;
  ELSE
    x := 0;
  END_IF;
END_PROGRAM
";
    for end in 0..=source.len() {
        if !source.is_char_boundary(end) {
            continue;
        }
        let prefix = &source[..end];
        let (file, _) = parse_source(prefix);
        assert_eq!(file.root().text(), prefix);
    }
}

#[test]
fn missing_end_at_eof_is_reported_once_per_construct() {
    let source = "FUNCTION F : INT\n  IF A THEN\n    B := 1;\n";
    let (_, sink) = parse_source(source);
    let missing = sink
        .iter()
        .filter(|diagnostic| diagnostic.code == "STC-E-GRAM-111")
        .count();
    assert!(
        missing >= 2,
        "expected END_IF and END_FUNCTION, got {missing}"
    );
}

fn count_kind(node: stc_syntax::SyntaxNode, kind: SyntaxKind) -> usize {
    let mut count = usize::from(node.kind() == kind);
    for child in node.child_nodes() {
        count += count_kind(child, kind);
    }
    count
}
