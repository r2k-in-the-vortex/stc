mod common;

use common::{assert_error, parse_clean, parse_relaxed_clean, parse_source};
use stc_syntax::ast::{
    AstNode, Expr, Item, MethodDecl, SourceFile, Stmt, StmtList, TypeDecl, VarDeclLike,
};
use stc_syntax::{SyntaxKind, dump_typed_ast};

fn items(source: &str) -> Vec<Item> {
    let file = parse_clean(source);
    SourceFile::cast(&file.root()).unwrap().items()
}

fn first_program(source: &str) -> Vec<Stmt> {
    let file = parse_clean(source);
    let items = SourceFile::cast(&file.root()).unwrap().items();
    for item in items {
        if let Item::Program(program) = item {
            let body: StmtList = program.body().unwrap();
            return body.statements();
        }
    }
    panic!("no program");
}

fn program_rhs(source: &str) -> Expr {
    let statements = first_program(source);
    let Stmt::Assign(assign) = &statements[0] else {
        panic!("expected assignment, got {:?}", statements[0]);
    };
    assign.rhs().expect("assignment rhs")
}

fn wrap(body: &str) -> String {
    format!("PROGRAM P\nVAR a, b, c : DINT;\nEND_VAR\n{body}\nEND_PROGRAM\n")
}

// ---- STC-GRAM-001: source unit ------------------------------------------

#[test]
fn stc_gram_001_source_file_and_top_items() {
    let source = r"
USING Company.Types;
TYPE Simple : INT; END_TYPE
FUNCTION F : INT END_FUNCTION
FUNCTION_BLOCK FB END_FUNCTION_BLOCK
PROGRAM P END_PROGRAM
CLASS C END_CLASS
INTERFACE I END_INTERFACE
CONFIGURATION K
PROGRAM Main : MainProgram;
END_CONFIGURATION
VAR_GLOBAL
  g : INT;
END_VAR
";
    let items = items(source);
    assert_eq!(items.len(), 9);
}

#[test]
fn stc_gram_001_source_file_negative() {
    assert_error("PROGRAM P\n  x := ;\nEND_PROGRAM\n", "STC-E-GRAM-105");
}

// ---- STC-GRAM-002: namespaces and USING ----------------------------------

#[test]
fn stc_gram_002_namespaces_and_using() {
    let source = r"
NAMESPACE INTERNAL A.B
USING C.D, E.F;
NAMESPACE C
TYPE T : INT; END_TYPE
END_NAMESPACE
FUNCTION F : INT END_FUNCTION
END_NAMESPACE
";
    let items = items(source);
    let Item::Namespace(namespace) = &items[0] else {
        panic!("expected namespace");
    };
    assert!(namespace.is_internal());
    assert_eq!(namespace.name().unwrap().segments().len(), 2);
    assert_eq!(namespace.using_directives().len(), 1);
    assert_eq!(namespace.elements().len(), 2);
}

#[test]
fn stc_gram_002_namespaces_and_using_negative() {
    assert_error(
        "NAMESPACE A\nPROGRAM P END_PROGRAM\nEND_NAMESPACE\n",
        "STC-E-GRAM-105",
    );
    assert_error("USING ;\n", "STC-E-GRAM-106");
    assert_error("USING A.;\n", "STC-E-GRAM-106");
}

// ---- STC-GRAM-003: data types -------------------------------------------

#[test]
fn stc_gram_003_type_declarations() {
    let source = r"
TYPE
  Alias : INT;
  Range : INT (0..100);
  Color : (Red, Green := 2, Blue);
  Mask : DWORD (Read := 16#01, Write := 16#02);
  Vector : ARRAY [1..3] OF REAL;
  Matrix : ARRAY [0..1, 0..1] OF INT;
  Point : STRUCT OVERLAP
    x : REAL;
    y : REAL;
  END_STRUCT;
  Name : STRING[32];
  Other : REF_TO Point;
END_TYPE
";
    let items = items(source);
    let Item::DataType(data) = &items[0] else {
        panic!("expected data type");
    };
    let types = data.types();
    assert_eq!(types.len(), 9);
    assert!(matches!(types[0], TypeDecl::Simple(_)));
    assert!(matches!(types[1], TypeDecl::Subrange(_)));
    assert!(matches!(types[2], TypeDecl::Enum(_)));
    assert!(matches!(types[3], TypeDecl::NamedValue(_)));
    assert!(matches!(types[4], TypeDecl::Array(_)));
    assert!(matches!(types[5], TypeDecl::Array(_)));
    assert!(matches!(types[6], TypeDecl::Struct(_)));
    assert!(matches!(types[7], TypeDecl::String(_)));
    assert!(matches!(types[8], TypeDecl::Ref(_)));
}

#[test]
fn stc_gram_003_type_declarations_negative() {
    assert_error("TYPE X : ; END_TYPE\n", "STC-E-GRAM-106");
    assert_error("TYPE X : INT END_TYPE\n", "STC-E-GRAM-107");
    assert_error(
        "TYPE X : STRUCT a : INT END_STRUCT; END_TYPE\n",
        "STC-E-GRAM-107",
    );
}

// ---- STC-GRAM-004: variable declarations --------------------------------

#[test]
fn stc_gram_004_variable_declarations() {
    let source = r"
PROGRAM P
VAR_INPUT CONSTANT
  a, b : INT;
END_VAR
VAR_OUTPUT RETAIN
  out : BOOL;
END_VAR
VAR_IN_OUT
  data : ARRAY [*] OF INT;
END_VAR
VAR_TEMP
  temp : DINT;
END_VAR
VAR_EXTERNAL
  ext : INT;
END_VAR
VAR
  at_var AT %IX0.0 : BOOL;
  point : Point := (x := 1.0, y := 2.0);
  vector : ARRAY [1..2] OF INT := [1, 2];
  text : STRING[10] := 'hello';
  edge : BOOL R_EDGE;
END_VAR
END_PROGRAM
";
    let file = parse_clean(source);
    let items = SourceFile::cast(&file.root()).unwrap().items();
    let Item::Program(program) = &items[0] else {
        panic!("expected program");
    };
    let sections = program.var_sections();
    assert_eq!(sections.len(), 6);
    let last = sections.last().unwrap();
    let declarations = last.declarations();
    assert_eq!(declarations.len(), 5);
    let VarDeclLike::Var(at_var) = &declarations[0] else {
        panic!("expected var decl");
    };
    assert!(at_var.has_at());
    assert!(matches!(&declarations[4], VarDeclLike::Edge(_)));
}

#[test]
fn stc_gram_004_variable_declarations_negative() {
    assert_error(
        "PROGRAM P\nVAR\n  x INT;\nEND_VAR\nEND_PROGRAM\n",
        "STC-E-GRAM-106",
    );
    assert_error(
        "PROGRAM P\nVAR\n  x : INT;\nEND_PROGRAM\n",
        "STC-E-GRAM-108",
    );
    assert_error(
        "PROGRAM P\nVAR\n  x : INT\nEND_VAR\nEND_PROGRAM\n",
        "STC-E-GRAM-107",
    );
}

// ---- STC-GRAM-005: POU declarations -------------------------------------

#[test]
fn stc_gram_005_pou_declarations() {
    let source = r"
FUNCTION Add : DINT
VAR_INPUT
  a, b : DINT;
END_VAR
  Add := a + b;
END_FUNCTION

FUNCTION_BLOCK FINAL Base IMPLEMENTS I1, I2
VAR
  state : DINT;
END_VAR
METHOD PUBLIC FINAL OVERRIDE Run : BOOL
  Run := TRUE;
END_METHOD
END_FUNCTION_BLOCK

CLASS ABSTRACT Derived EXTENDS Base IMPLEMENTS I1
VAR
  id : DINT;
END_VAR
METHOD PROTECTED ABSTRACT Stop
END_METHOD
END_CLASS

INTERFACE I1 EXTENDS I2
METHOD Run : BOOL
END_METHOD
END_INTERFACE

INTERFACE I2
END_INTERFACE

PROGRAM Main
VAR
  x : INT;
END_VAR
VAR_ACCESS
  visible : Main.x : INT READ_ONLY;
END_VAR
  x := 1;
END_PROGRAM
";
    let file = parse_clean(source);
    let items = SourceFile::cast(&file.root()).unwrap().items();
    assert_eq!(items.len(), 6);
    let Item::FunctionBlock(fb) = &items[1] else {
        panic!("expected FB");
    };
    assert!(fb.is_final());
    assert_eq!(fb.methods().len(), 1);
    let Item::Class(class) = &items[2] else {
        panic!("expected class");
    };
    assert!(class.is_abstract());
    let method: &MethodDecl = &class.methods()[0];
    assert!(method.is_abstract());
    assert!(method.access().is_some());
    let Item::Program(program) = &items[5] else {
        panic!("expected program");
    };
    assert!(program.var_access().is_some());
}

#[test]
fn stc_gram_005_pou_declarations_negative() {
    assert_error("FUNCTION F : INT\n  F := 1;\n", "STC-E-GRAM-111");
    assert_error("FUNCTION F\n  F := 1;\nEND_PROGRAM\n", "STC-E-GRAM-108");
    assert_error(
        "CLASS C\nVAR_INPUT\n  x : INT;\nEND_VAR\nEND_CLASS\n",
        "STC-E-GRAM-105",
    );
    assert_error(
        "INTERFACE I\nMETHOD M\n  x := 1;\nEND_METHOD\nEND_INTERFACE\n",
        "STC-E-GRAM-105",
    );
}

// ---- STC-GRAM-006: configurations ---------------------------------------

#[test]
fn stc_gram_006_configurations() {
    let source = r"
CONFIGURATION Plant
VAR_GLOBAL
  running : BOOL;
END_VAR
RESOURCE Line ON PLC
VAR_GLOBAL
  local : INT;
END_VAR
  TASK Fast(INTERVAL := T#10ms, PRIORITY := 1);
  TASK Slow(SINGLE := T#100ms);
  PROGRAM Main WITH Fast : MainProgram (cycle := 1, out => %QW0);
  PROGRAM Backup WITH Slow : BackupProgram;
END_RESOURCE
VAR_ACCESS
  running_path : Plant.running : BOOL READ_WRITE;
END_VAR
VAR_CONFIG
  Main.Timer.Preset AT %MW10 : INT := 5;
END_VAR
END_CONFIGURATION
";
    let file = parse_clean(source);
    let items = SourceFile::cast(&file.root()).unwrap().items();
    let Item::Configuration(config) = &items[0] else {
        panic!("expected configuration");
    };
    assert_eq!(config.resources().len(), 1);
    let resource = &config.resources()[0];
    assert_eq!(resource.tasks().len(), 2);
    assert_eq!(resource.program_configs().len(), 2);
    assert!(config.var_access().is_some());
    assert!(config.config_init().is_some());
}

#[test]
fn stc_gram_006_configurations_negative() {
    assert_error(
        "CONFIGURATION C\nRESOURCE R ON PLC\nPROGRAM P : Main;\nEND_CONFIGURATION\n",
        "STC-E-GRAM-108",
    );
    assert_error(
        "CONFIGURATION C\nRESOURCE R ON PLC\nTASK T(INTERVAL := 1);\nEND_RESOURCE\nEND_CONFIGURATION\n",
        "STC-E-GRAM-106",
    );
}

// ---- STC-GRAM-007: statements -------------------------------------------

#[test]
fn stc_gram_007_statements() {
    let source = r"
PROGRAM P
VAR
  a, b : DINT;
  flag : BOOL;
  p : REF_TO DINT;
  fb : FB;
END_VAR
  a := 1;
  b := a + 1;
  p := REF(a);
  p := NULL;
  p^ := b;
  p ?= REF(b);
  fb(in := a, out => b, NOT flag => a);
  ;
  IF flag THEN a := 1; ELSIF b > 0 THEN a := 2; ELSE a := 3; END_IF;
  CASE a OF 0 : b := 1; 1, 2 : b := 2; 3..5 : b := 3; ELSE b := 4; END_CASE;
  FOR a := 0 TO 9 BY 2 DO b := a; END_FOR;
  WHILE flag DO a := a + 1; END_WHILE;
  REPEAT a := a - 1; UNTIL a = 0 END_REPEAT;
  EXIT;
  CONTINUE;
  RETURN;
END_PROGRAM
";
    let statements = first_program(source);
    assert!(matches!(statements[0], Stmt::Assign(_)));
    assert!(matches!(statements[2], Stmt::RefAssign(_)));
    assert!(matches!(statements[5], Stmt::Attempt(_)));
    assert!(matches!(statements[6], Stmt::Call(_)));
    assert!(matches!(statements[7], Stmt::Empty(_)));
    assert!(matches!(statements[8], Stmt::If(_)));
    assert!(matches!(statements[9], Stmt::Case(_)));
    assert!(matches!(statements[10], Stmt::For(_)));
    assert!(matches!(statements[11], Stmt::While(_)));
    assert!(matches!(statements[12], Stmt::Repeat(_)));
    assert!(matches!(statements[13], Stmt::Exit(_)));
    assert!(matches!(statements[14], Stmt::Continue(_)));
    assert!(matches!(statements[15], Stmt::Return(_)));
}

#[test]
fn stc_gram_007_statements_negative() {
    assert_error(&wrap("a := 1"), "STC-E-GRAM-107");
    assert_error("PROGRAM P\nIF a THEN a := 1;\n", "STC-E-GRAM-111");
    assert_error(&wrap("IF a THEN a := 1; END_CASE;"), "STC-E-GRAM-108");
    assert_error(&wrap("CASE a OF END_CASE;"), "STC-E-GRAM-106");
    assert_error(
        &wrap("FOR a := 0 TO 1 DO a := 1; END_FOR"),
        "STC-E-GRAM-107",
    );
    assert_error(&wrap("x;"), "STC-E-GRAM-105");
}

// ---- STC-GRAM-008: expressions ------------------------------------------

#[test]
fn stc_gram_008_expressions() {
    let rhs = program_rhs(&wrap(
        "a := f(1, named := 2, out => b, NOT flag => b) + arr[1, 2] + p^ + THIS.b + obj.member + a.0 + a.%X0 + REF(b) + NULL;",
    ));
    assert!(matches!(rhs, Expr::Binary(_)));
}

#[test]
fn stc_gram_008_expressions_negative() {
    assert_error(&wrap("a := 1 +;"), "STC-E-GRAM-105");
    assert_error(&wrap("a := (1 + 2;"), "STC-E-GRAM-106");
    assert_error(&wrap("a := f(1,);"), "STC-E-GRAM-105");
    assert_error(&wrap("a := 1(2);"), "STC-E-GRAM-105");
    assert_error(&wrap("a := (f)(1);"), "STC-E-GRAM-105");
}

// ---- STC-GRAM-009 to 015: precedence ------------------------------------

#[test]
fn stc_gram_009_comparison_precedence() {
    let rhs = program_rhs(&wrap("a := A = B < C;"));
    let Expr::Binary(equality) = rhs else {
        panic!("expected binary expression");
    };
    assert_eq!(equality.operator().unwrap().kind(), SyntaxKind::PunctEqual);
    let Expr::Binary(relational) = equality.rhs().unwrap() else {
        panic!("expected relational rhs");
    };
    assert_eq!(relational.operator().unwrap().kind(), SyntaxKind::PunctLess);
}

#[test]
fn stc_gram_010_power_associativity() {
    let rhs = program_rhs(&wrap("a := 2 ** 3 ** 4;"));
    let Expr::Binary(outer) = rhs else {
        panic!("expected binary");
    };
    assert_eq!(outer.operator().unwrap().kind(), SyntaxKind::PunctPower);
    assert!(matches!(outer.lhs().unwrap(), Expr::Binary(_)));
    assert!(matches!(outer.rhs().unwrap(), Expr::Literal(_)));
}

#[test]
fn stc_gram_011_unary_before_power() {
    let rhs = program_rhs(&wrap("a := -2 ** 2;"));
    let Expr::Binary(power) = rhs else {
        panic!("expected binary");
    };
    assert_eq!(power.operator().unwrap().kind(), SyntaxKind::PunctPower);
    assert!(matches!(power.lhs().unwrap(), Expr::Unary(_)));
}

#[test]
fn stc_gram_012_operator_aliases() {
    let rhs = program_rhs(&wrap("a := A & B AND C;"));
    let Expr::Binary(outer) = rhs else {
        panic!("expected binary");
    };
    assert_eq!(outer.operator().unwrap().kind(), SyntaxKind::KwAnd);
    let Expr::Binary(inner) = outer.lhs().unwrap() else {
        panic!("expected left-associative nest");
    };
    assert_eq!(inner.operator().unwrap().kind(), SyntaxKind::PunctAmpersand);
}

#[test]
fn stc_gram_013_short_circuit_syntax_parses() {
    let _ = parse_clean(&wrap("a := A AND B OR C;"));
}

#[test]
fn stc_gram_014_operand_order_parses() {
    let rhs = program_rhs(&wrap("a := f(1) + g(2);"));
    let Expr::Binary(sum) = rhs else {
        panic!("expected binary");
    };
    assert!(matches!(sum.lhs().unwrap(), Expr::Call(_)));
    assert!(matches!(sum.rhs().unwrap(), Expr::Call(_)));
}

#[test]
fn stc_gram_015_not_precedence() {
    let rhs = program_rhs(&wrap("flag := NOT A = B;"));
    let Expr::Binary(equality) = rhs else {
        panic!("expected binary");
    };
    assert_eq!(equality.operator().unwrap().kind(), SyntaxKind::PunctEqual);
    assert!(matches!(equality.lhs().unwrap(), Expr::Unary(_)));
}

// ---- STC-GRAM-016: semicolons -------------------------------------------

#[test]
fn stc_gram_016_strict_and_relaxed_semicolons() {
    let strict = wrap("IF a THEN a := 1; END_IF");
    assert_error(&strict, "STC-E-GRAM-107");
    let _ = parse_relaxed_clean(&strict);

    let body = wrap("IF a THEN a := 1; END_IF;");
    let _ = parse_clean(&body);
}

// ---- STC-UTF8: type names and literals ----------------------------------

#[test]
fn stc_gram_utf8_type_names_and_literals() {
    let source = r"
PROGRAM P
VAR
  u : USTRING[16] := U#'héllo';
  c : UCHAR;
  s : WSTRING[8];
END_VAR
  u := USTRING#'emoji ${1F600}';
  c := UCHAR#'A';
END_PROGRAM
";
    let file = parse_clean(source);
    let items = SourceFile::cast(&file.root()).unwrap().items();
    let Item::Program(program) = &items[0] else {
        panic!("expected program");
    };
    assert_eq!(program.var_sections().len(), 1);
}

// ---- unsupported constructs --------------------------------------------

#[test]
fn stc_e_gram_101_il_body() {
    assert_error(&wrap("LD a;"), "STC-E-GRAM-101");
    assert_error(&wrap("CAL f;"), "STC-E-GRAM-101");
}

#[test]
fn stc_e_gram_102_sfc() {
    assert_error("ACTION A\nEND_ACTION\n", "STC-E-GRAM-102");
    assert_error("STEP S\n", "STC-E-GRAM-102");
}

#[test]
fn stc_e_gram_103_preprocessor() {
    assert_error("#if TRUE\n", "STC-E-GRAM-103");
    assert_error(&wrap("#define X"), "STC-E-GRAM-103");
}

#[test]
fn stc_e_gram_104_ed4_constructs() {
    assert_error("PROPERTY Value : INT\nEND_PROPERTY\n", "STC-E-GRAM-104");
    assert_error("MUTEX lock;\n", "STC-E-GRAM-104");
    assert_error(&wrap("ASSERT(flag);"), "STC-E-GRAM-104");
}

#[test]
fn stc_e_gram_105_unexpected_token() {
    assert_error("PROGRAM P\nEND_PROGRAM\nEND_PROGRAM\n", "STC-E-GRAM-105");
}

#[test]
fn stc_e_gram_106_expected_token() {
    assert_error(
        "PROGRAM P\nVAR\n  x : ;\nEND_VAR\nEND_PROGRAM\n",
        "STC-E-GRAM-106",
    );
}

#[test]
fn stc_e_gram_107_missing_semicolon() {
    assert_error(&wrap("a := 1"), "STC-E-GRAM-107");
}

#[test]
fn stc_e_gram_108_mismatched_end() {
    assert_error(&wrap("IF a THEN a := 1; END_WHILE;"), "STC-E-GRAM-108");
}

#[test]
fn stc_e_gram_109_depth_limit() {
    let deep = format!("a := {}1{};", "(".repeat(400), ")".repeat(400));
    assert_error(&wrap(&deep), "STC-E-GRAM-109");

    let refs = format!("a := {}NULL{};", "REF(".repeat(400), ")".repeat(400));
    assert_error(&wrap(&refs), "STC-E-GRAM-109");
}

#[test]
fn stc_e_gram_110_pragma_placement() {
    let source = wrap("a := 1; {stc:test}");
    assert_error(&source, "STC-E-GRAM-110");
}

#[test]
fn stc_e_gram_111_missing_end() {
    assert_error("FUNCTION F : INT\n  F := 1;\n", "STC-E-GRAM-111");
}

// ---- typed AST sanity ---------------------------------------------------

#[test]
fn typed_ast_dump_covers_all_item_kinds() {
    let source = r"
TYPE T : INT; END_TYPE
FUNCTION F : INT END_FUNCTION
FUNCTION_BLOCK B END_FUNCTION_BLOCK
CLASS C END_CLASS
INTERFACE I END_INTERFACE
PROGRAM P END_PROGRAM
CONFIGURATION K
PROGRAM Main : MainProgram;
END_CONFIGURATION
";
    let (file, sink) = parse_source(source);
    assert!(!sink.has_errors(), "{:?}", sink.diagnostics());
    let dump = dump_typed_ast(file.tree());
    for kind in [
        "DataTypeDecl",
        "FunctionDecl",
        "FunctionBlockDecl",
        "ClassDecl",
        "InterfaceDecl",
        "ProgramDecl",
        "ConfigDecl",
    ] {
        assert!(dump.contains(kind), "missing {kind} in\n{dump}");
    }
}

#[test]
fn program_name_is_available() {
    let file = parse_clean("PROGRAM Main END_PROGRAM\n");
    let items = SourceFile::cast(&file.root()).unwrap().items();
    let Item::Program(program) = &items[0] else {
        panic!("expected program");
    };
    assert_eq!(program.name().unwrap().text(), "Main");
}
