mod common;

use common::{dump, parse_clean, parse_source};
use stc_syntax::ast::{AstNode, Item, Stmt};
use stc_syntax::{SyntaxKind, dump_typed_ast};

#[test]
fn empty_file_parses() {
    let file = parse_clean("");
    assert_eq!(file.root().kind(), SyntaxKind::SourceFile);
    assert!(file.root().child_nodes().is_empty());
}

#[test]
fn function_parses_with_a_body() {
    let source = "FUNCTION Add : DINT\nVAR_INPUT\n  a : DINT;\n  b : DINT;\nEND_VAR\n  Add := a + b;\nEND_FUNCTION\n";
    let file = parse_clean(source);
    let items = stc_syntax::ast::SourceFile::cast(&file.root())
        .unwrap()
        .items();
    assert_eq!(items.len(), 1);
    let Item::Function(function) = &items[0] else {
        panic!("expected a function");
    };
    assert_eq!(function.name().unwrap().text(), "Add");
    assert_eq!(function.var_sections().len(), 1);
    let body = function.body().unwrap();
    assert_eq!(body.statements().len(), 1);
    assert!(matches!(body.statements()[0], Stmt::Assign(_)));
}

#[test]
fn object_oriented_surface_parses() {
    let source = r"
INTERFACE Drivable
  METHOD Drive : BOOL
    VAR_INPUT
      speed : DINT;
    END_VAR
  END_METHOD
END_INTERFACE

FUNCTION_BLOCK FINAL Motor IMPLEMENTS Drivable
VAR
  speed : DINT;
END_VAR

METHOD PUBLIC OVERRIDE Drive : BOOL
VAR_INPUT
  speed : DINT;
END_VAR
  THIS.speed := speed;
  Drive := TRUE;
END_METHOD
END_FUNCTION_BLOCK

CLASS ABSTRACT Vehicle EXTENDS Base IMPLEMENTS Drivable
VAR
  id : DINT;
END_VAR
METHOD Drive : BOOL
  Drive := TRUE;
END_METHOD
END_CLASS
";
    let file = parse_clean(source);
    let items = stc_syntax::ast::SourceFile::cast(&file.root())
        .unwrap()
        .items();
    assert_eq!(items.len(), 3);
}

#[test]
fn namespaces_types_and_configuration_parse() {
    let source = r"
NAMESPACE INTERNAL Company.Controls
USING Company.Types;
TYPE
  Speed : INT (0..100);
  Color : ( Red, Green, Blue );
  Flags : DWORD ( Read := 16#01, Write := 16#02 );
  Vector : ARRAY [1..3] OF REAL;
  Point : STRUCT OVERLAP
    x : REAL;
    y : REAL;
  END_STRUCT;
END_TYPE

FUNCTION Clamp : INT
VAR_INPUT
  value : INT;
END_VAR
  Clamp := value;
END_FUNCTION
END_NAMESPACE

CONFIGURATION Plant
VAR_GLOBAL
  running : BOOL;
END_VAR
RESOURCE Line ON PLC
  TASK MainTask(INTERVAL := T#10ms, PRIORITY := 1);
  PROGRAM Main WITH MainTask : MainProgram;
END_RESOURCE
VAR_CONFIG
  Main.Counter.Preset AT %MW10 : INT := 5;
END_VAR
END_CONFIGURATION
";
    let file = parse_clean(source);
    let typed = dump_typed_ast(file.tree());
    assert!(typed.contains("NamespaceDecl"));
    assert!(typed.contains("ConfigDecl"));
}

#[test]
fn statements_and_expressions_parse() {
    let source = r"
PROGRAM Main
VAR
  a, b, c : DINT;
  flag : BOOL;
  arr : ARRAY [0..9] OF DINT;
  p : REF_TO DINT;
END_VAR
  a := 1 + 2 * 3 ** 2;
  b := -2 ** 2;
  flag := NOT a = b;
  IF flag THEN
    a := a + 1;
  ELSIF b > 0 THEN
    b := b - 1;
  ELSE
    a := 0;
  END_IF;
  CASE a OF
    0 : b := 1;
    1, 2 : b := 2;
    3..5 : b := 3;
    ELSE
      b := 4;
  END_CASE;
  FOR a := 0 TO 9 BY 1 DO
    arr[a] := a;
  END_FOR;
  WHILE a < 10 DO
    a := a + 1;
  END_WHILE;
  REPEAT
    a := a - 1;
  UNTIL a = 0 END_REPEAT;
  p := REF(a);
  p^ := b;
  EXIT;
  CONTINUE;
  RETURN;
END_PROGRAM
";
    let (file, sink) = parse_source(source);
    assert!(
        !sink.has_errors(),
        "errors: {:#?}\n{}",
        sink.diagnostics(),
        dump_typed_ast(file.tree())
    );
}

#[test]
fn cst_dump_is_stable_and_round_trips() {
    let source = "PROGRAM P\n  x := 1;\nEND_PROGRAM\n";
    let first = dump(source);
    let second = dump(source);
    assert_eq!(first, second);
    assert!(first.contains("ProgramDecl"));
    assert!(first.contains("AssignStmt"));
}

#[test]
fn directly_represented_variables_are_primary_expressions() {
    let source = "PROGRAM P
VAR
  x : WORD;
  b : BOOL;
END_VAR
  x := %IW0;
  b := %IX0.0;
  %QW0 := x;
END_PROGRAM
";
    let file = parse_clean(source);
    let items = stc_syntax::ast::SourceFile::cast(&file.root())
        .unwrap()
        .items();
    let Item::Program(program) = &items[0] else {
        panic!("expected a program");
    };
    assert_eq!(program.body().unwrap().statements().len(), 3);
}
