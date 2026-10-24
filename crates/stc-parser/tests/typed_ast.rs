mod common;

use common::parse_clean;
use stc_syntax::ast::{Arg, AstNode, Expr, Item, PouKind, SourceFile, Stmt, TypeDecl};

fn items(source: &str) -> Vec<Item> {
    let file = parse_clean(source);
    SourceFile::cast(&file.root()).unwrap().items()
}

#[test]
fn fb_flags_are_not_taken_from_nested_methods() {
    let items = items("FUNCTION_BLOCK FB\nMETHOD FINAL M END_METHOD\nEND_FUNCTION_BLOCK\n");
    let Item::FunctionBlock(fb) = &items[0] else {
        panic!("expected FB");
    };
    assert!(!fb.is_final());
    assert!(fb.methods()[0].is_final());
    assert_eq!(fb.methods()[0].pou_kind(), Some(PouKind::FunctionBlock));
}

#[test]
fn class_flags_are_not_taken_from_nested_methods() {
    let items = items("CLASS C\nMETHOD ABSTRACT M END_METHOD\nEND_CLASS\n");
    let Item::Class(class) = &items[0] else {
        panic!("expected class");
    };
    assert!(!class.is_abstract());
    assert!(class.methods()[0].is_abstract());
    assert_eq!(class.methods()[0].pou_kind(), Some(PouKind::Class));
}

#[test]
fn namespace_internal_is_not_inherited_from_a_nested_namespace() {
    let items = items(
        "NAMESPACE Outer\nNAMESPACE INTERNAL Inner\nTYPE T : INT; END_TYPE\nEND_NAMESPACE\nEND_NAMESPACE\n",
    );
    let Item::Namespace(outer) = &items[0] else {
        panic!("expected namespace");
    };
    assert!(!outer.is_internal());
    let inner = outer.elements();
    let Item::Namespace(inner) = &inner[0] else {
        panic!("expected nested namespace");
    };
    assert!(inner.is_internal());
}

#[test]
fn struct_overlap_is_not_taken_from_a_nested_struct() {
    let items = items(
        "TYPE\nOuter : STRUCT\ninner : STRUCT OVERLAP\nx : INT;\nEND_STRUCT;\nEND_STRUCT;\nEND_TYPE\n",
    );
    let Item::DataType(data) = &items[0] else {
        panic!("expected data type");
    };
    let TypeDecl::Struct(outer) = &data.types()[0] else {
        panic!("expected struct");
    };
    assert!(!outer.is_overlap());
}

#[test]
fn extends_and_implements_are_accessible() {
    let items = items(
        r"
INTERFACE I2
END_INTERFACE
INTERFACE I3
END_INTERFACE
INTERFACE I1 EXTENDS I2, I3
END_INTERFACE
FUNCTION_BLOCK FB EXTENDS Base IMPLEMENTS I1, I2
END_FUNCTION_BLOCK
CLASS C EXTENDS Base IMPLEMENTS I1
END_CLASS
",
    );
    let Item::Interface(interface) = &items[2] else {
        panic!("expected interface");
    };
    let extends: Vec<String> = interface
        .extends()
        .iter()
        .map(|path| path.text().to_owned())
        .collect();
    assert_eq!(extends, vec!["I2", "I3"]);

    let Item::FunctionBlock(fb) = &items[3] else {
        panic!("expected FB");
    };
    assert_eq!(fb.extends().unwrap().text(), "Base");
    let implements: Vec<String> = fb
        .implements()
        .iter()
        .map(|path| path.text().to_owned())
        .collect();
    assert_eq!(implements, vec!["I1", "I2"]);

    let Item::Class(class) = &items[4] else {
        panic!("expected class");
    };
    assert_eq!(class.extends().unwrap().text(), "Base");
    assert_eq!(class.implements().len(), 1);
}

#[test]
fn a_reference_to_null_is_not_itself_null() {
    let items = items("PROGRAM P\nVAR p : REF_TO DINT; END_VAR\np := REF(NULL);\nEND_PROGRAM\n");
    let Item::Program(program) = &items[0] else {
        panic!("expected program");
    };
    let Stmt::RefAssign(assign) = &program.body().unwrap().statements()[0] else {
        panic!("expected reference assignment");
    };
    let Expr::RefValue(outer) = assign.rhs().unwrap() else {
        panic!("expected REF value");
    };
    assert!(!outer.is_null());
    let Expr::RefValue(inner) = outer.target().unwrap() else {
        panic!("expected nested REF value");
    };
    assert!(inner.is_null());
}

#[test]
fn nested_output_bindings_do_not_leak_into_the_outer_argument() {
    let items = items("PROGRAM P\nVAR a, b : DINT; END_VAR\nf(g(a => b));\nEND_PROGRAM\n");
    let Item::Program(program) = &items[0] else {
        panic!("expected program");
    };
    let Stmt::Call(statement) = &program.body().unwrap().statements()[0] else {
        panic!("expected call");
    };
    let outer = statement.call().unwrap();
    let outer_args = outer.arguments();
    assert_eq!(outer_args.len(), 1);
    let Arg::Positional(Expr::Call(inner)) = &outer_args[0] else {
        panic!("expected a positional nested call, got {:?}", outer_args[0]);
    };
    let inner_args = inner.arguments();
    assert!(
        matches!(&inner_args[0], Arg::Output(_, _)),
        "{inner_args:?}"
    );
}

#[test]
fn negated_output_binding_is_recognized() {
    let items = items(
        "PROGRAM P\nVAR a, b : DINT; flag : BOOL; END_VAR\nf(NOT flag => a, b => b);\nEND_PROGRAM\n",
    );
    let Item::Program(program) = &items[0] else {
        panic!("expected program");
    };
    let Stmt::Call(statement) = &program.body().unwrap().statements()[0] else {
        panic!("expected call");
    };
    let args = statement.call().unwrap().arguments();
    assert!(matches!(&args[0], Arg::NegatedOutput(_, _)), "{args:?}");
    assert!(matches!(&args[1], Arg::Output(_, _)), "{args:?}");
}
