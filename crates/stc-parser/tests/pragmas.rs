mod common;

use common::{assert_error, codes, parse_clean, parse_source};
use stc_syntax::ast::{AstNode, Item, SourceFile, VarDeclLike};

fn items(source: &str) -> Vec<Item> {
    let file = parse_clean(source);
    SourceFile::cast(&file.root()).unwrap().items()
}

#[test]
fn leading_pragma_attaches_to_a_function() {
    let source = "{stc:test {\"cycles\": 25}}\nFUNCTION T : BOOL\n  T := TRUE;\nEND_FUNCTION\n";
    let items = items(source);
    let Item::Function(function) = &items[0] else {
        panic!("expected function");
    };
    let pragmas = function.pragmas();
    assert_eq!(pragmas.len(), 1);
    let annotation = pragmas[0].annotation().expect("annotation");
    assert_eq!(annotation.name, "test");
    assert_eq!(annotation.payload.as_ref().unwrap()["cycles"], 25);
}

#[test]
fn leading_pragma_attaches_to_a_variable_declaration() {
    let source = r#"
PROGRAM P
VAR
  {stc:deprecated {"message": "use other"}}
  old : INT;
END_VAR
END_PROGRAM
"#;
    let items = items(source);
    let Item::Program(program) = &items[0] else {
        panic!("expected program");
    };
    let decl = &program.var_sections()[0].declarations()[0];
    let VarDeclLike::Var(decl) = decl else {
        panic!("expected var");
    };
    let pragmas = decl.pragmas();
    assert_eq!(pragmas.len(), 1);
    assert_eq!(pragmas[0].annotation().unwrap().name, "deprecated");
}

#[test]
fn trailing_io_pragma_attaches_to_the_declaration() {
    let source = r#"
VAR_GLOBAL
  MyBool AT %IX5 : BOOL {stc:io {"device": "drive1", "channel": 5, "direction": "input"}};
END_VAR
"#;
    let (file, sink) = parse_source(source);
    assert!(!sink.has_errors(), "{:?}", sink.diagnostics());
    assert!(
        sink.is_empty(),
        "unexpected warnings: {:?}",
        sink.diagnostics()
    );
    let items = SourceFile::cast(&file.root()).unwrap().items();
    let Item::GlobalVars(section) = &items[0] else {
        panic!("expected global vars");
    };
    let VarDeclLike::Var(decl) = &section.declarations()[0] else {
        panic!("expected var");
    };
    assert!(decl.has_at());
    let pragmas = decl.pragmas();
    assert_eq!(pragmas.len(), 1);
    let annotation = pragmas[0].annotation().unwrap();
    assert_eq!(annotation.name, "io");
    assert_eq!(annotation.object().unwrap()["channel"], 5);
}

#[test]
fn io_without_at_warns() {
    let source = r#"
VAR_GLOBAL
  MyBool : BOOL {stc:io {"device": "drive1"}};
END_VAR
"#;
    let (_, sink) = parse_source(source);
    assert!(!sink.has_errors());
    assert!(
        codes(&sink).contains(&"STC-W-IO-002"),
        "{:?}",
        sink.diagnostics()
    );
}

#[test]
fn duplicate_io_pragmas_are_an_error() {
    let source = r#"
VAR_GLOBAL
  MyBool AT %IX5 : BOOL {stc:io {"device": "a"}} {stc:io {"device": "b"}};
END_VAR
"#;
    let (_, sink) = parse_source(source);
    assert!(
        codes(&sink).contains(&"STC-E-IO-003"),
        "{:?}",
        sink.diagnostics()
    );
}

#[test]
fn invalid_json_pragmas_are_reported() {
    let source = "{stc:test {nope}}\nFUNCTION F : INT END_FUNCTION\n";
    assert_error(source, "STC-E-LEX-010");
    assert_error(
        "{stc:test(cycles := 25)}\nFUNCTION F : INT END_FUNCTION\n",
        "STC-E-LEX-010",
    );
    assert_error(
        "{stc:test {\"x\": 1, \"x\": 2}}\nFUNCTION F : INT END_FUNCTION\n",
        "STC-E-LEX-011",
    );
}

#[test]
fn non_stc_and_unknown_pragmas_are_reported() {
    let (_, sink) = parse_source("{attribute 'x'}\nFUNCTION F : INT END_FUNCTION\n");
    assert!(codes(&sink).contains(&"STC-W-PRAGMA-001"));

    let (_, sink) = parse_source("{stc:mystery}\nFUNCTION F : INT END_FUNCTION\n");
    assert!(codes(&sink).contains(&"STC-E-PRAGMA-001"));
}

#[test]
fn unknown_keys_warn_and_survive() {
    let (file, sink) = parse_source(
        "{stc:test {\"cycles\": 1, \"future\": true}}\nFUNCTION F : INT END_FUNCTION\n",
    );
    assert!(codes(&sink).contains(&"STC-W-PRAGMA-002"));
    let items = SourceFile::cast(&file.root()).unwrap().items();
    let Item::Function(function) = &items[0] else {
        panic!("expected function");
    };
    let pragmas = function.pragmas();
    let annotation = pragmas[0].annotation().unwrap();
    assert_eq!(annotation.object().unwrap()["future"].to_string(), "true");
}

#[test]
fn pragmas_before_statements_are_placed_in_the_tree_with_a_diagnostic() {
    let source = "PROGRAM P\n  {stc:test} x := 1;\nEND_PROGRAM\n";
    assert_error(source, "STC-E-GRAM-110");
}

#[test]
fn pragma_at_the_end_of_a_block_is_diagnosed() {
    let source = "PROGRAM P\n  x := 1; {stc:test}\nEND_PROGRAM\n";
    assert_error(source, "STC-E-GRAM-110");
}

#[test]
fn pragmas_before_methods_attach_to_the_method() {
    let source = r"
FUNCTION_BLOCK FB
{stc:test}
METHOD Run : BOOL
  Run := TRUE;
END_METHOD
END_FUNCTION_BLOCK
";
    let items = items(source);
    let Item::FunctionBlock(fb) = &items[0] else {
        panic!("expected FB");
    };
    let method = &fb.methods()[0];
    // The method node's pragma is a direct child.
    let pragma = method
        .syntax()
        .child_nodes()
        .into_iter()
        .find_map(|node| stc_syntax::ast::Pragma::cast(&node))
        .expect("method pragma");
    assert_eq!(pragma.annotation().unwrap().name, "test");
}

#[test]
fn pragma_prefix_and_name_are_case_insensitive() {
    let source = "{STC:TEST {\"cycles\": 3}}\nFUNCTION F : INT END_FUNCTION\n";
    let items = items(source);
    let Item::Function(function) = &items[0] else {
        panic!("expected function");
    };
    let pragmas = function.pragmas();
    let annotation = pragmas[0].annotation().unwrap();
    assert_eq!(annotation.name, "test");
    assert_eq!(annotation.payload.as_ref().unwrap()["cycles"], 3);
}

#[test]
fn pragma_token_text_is_preserved_in_the_cst() {
    let source = "{stc:test {\"cycles\": 2}}\nFUNCTION F : INT END_FUNCTION\n";
    let file = parse_clean(source);
    let dump = stc_syntax::dump_cst(file.tree());
    assert!(dump.contains("stc:test"));
}
