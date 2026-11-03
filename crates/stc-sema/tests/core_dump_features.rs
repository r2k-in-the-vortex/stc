//! Core-HIR dump coverage for out-of-core features, every statement kind, and
//! every declaration and expression category.

use stc_diag::DiagnosticSink;
use stc_hir::{dump_core_hir, verify_core_dump};
use stc_lexer::tokenize_bytes;
use stc_parser::{ParseOptions, parse};
use stc_sema::analyze;
use stc_span::{FileId, SourceMap};

fn dump(source: &str) -> String {
    let mut sink = DiagnosticSink::unlimited();
    let mut map = SourceMap::new();
    let file = FileId::new(0);
    let lexed = tokenize_bytes(file, source.as_bytes(), &mut sink);
    let mapped = map.add_file("case.st", lexed.text.clone());
    assert_eq!(mapped, file);
    let parsed = parse(
        file,
        lexed.text.as_str(),
        &lexed.stream,
        ParseOptions::default(),
        &mut sink,
    );
    assert!(
        !sink.has_errors(),
        "parse errors for:\n{source}\n{:?}",
        sink.diagnostics()
    );
    let analysis = analyze(&[parsed], &mut sink);
    assert!(
        !sink.has_errors(),
        "analysis errors for:\n{source}\n{:?}",
        sink.diagnostics()
    );
    analysis
        .hir
        .check_invariants()
        .unwrap_or_else(|message| panic!("HIR invariant: {message}"));
    let plain = stc_hir::dump_hir(&analysis.hir);
    assert!(!plain.is_empty(), "the plain HIR dump is empty");
    let text = dump_core_hir(&analysis.hir, &map);
    verify_core_dump(&text).unwrap_or_else(|message| panic!("dump invalid: {message}"));
    text
}

#[test]
fn control_flow_statements_are_dumped() {
    let text = dump(
        "FUNCTION F : DINT
VAR_INPUT
  n : DINT;
END_VAR
VAR
  i : DINT;
END_VAR
  IF n > 0 THEN
    i := 1;
  ELSIF n < 0 THEN
    i := 2;
  ELSE
    i := 3;
  END_IF;
  CASE i OF
    1: i := 10;
    2, 3: i := 20;
    4..6: i := 30;
  ELSE
    i := 40;
  END_CASE;
  FOR i := 1 TO n BY 2 DO
    IF i = 4 THEN
      CONTINUE;
    END_IF;
    IF i = 8 THEN
      EXIT;
    END_IF;
  END_FOR;
  WHILE i > 0 DO
    i := i - 1;
  END_WHILE;
  REPEAT
    i := i + 1;
  UNTIL i >= 5
  END_REPEAT;
  IF n = 0 THEN
    RETURN;
  END_IF;
  F := i;
END_FUNCTION
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := F(3);
END_PROGRAM
",
    );
    assert!(text.contains("\"for\""), "{text}");
    assert!(text.contains("\"while\""), "{text}");
    assert!(text.contains("\"repeat\""), "{text}");
    assert!(text.contains("\"case\""), "{text}");
}

#[test]
fn literals_operators_and_places_are_dumped() {
    let text = dump(
        "VAR_GLOBAL
  g : DINT;
  direct AT %MW10 : INT;
END_VAR
FUNCTION Expressions : DINT
VAR
  a : DINT;
  b : DINT;
  flag : BOOL;
  bits : BYTE;
  r : REAL;
  t : TIME;
  d : DATE;
  todv : TOD;
  dtv : DT;
  l : LINT;
  s : STRING[8];
  arr : ARRAY[1..3] OF DINT;
  point : POINTER TO DINT;
  x : DINT;
END_VAR
  a := 16#10 + 2#1010 + 8#17 + 10;
  b := -a + +a;
  flag := NOT flag;
  bits := NOT bits;
  flag := bits.0;
  flag := (a = b) AND (a <> b) OR (a < b) XOR (a <= b) AND (a > b) OR (a >= b);
  a := (a + b) * 2 / 3 MOD 5;
  a := a ** 2;
  bits := bits & BYTE#16#0F;
  r := a;
  t := T#1s;
  d := D#2024-01-02;
  todv := TOD#12:30:00;
  dtv := DT#2024-01-02-12:30:00;
  l := DINT_TO_LINT(a);
  a := SIZEOF(DINT);
  a := SIZEOF(arr[1]);
  g := g + direct;
  arr[1] := a;
  point := ADR(arr[2]);
  x := point^;
  s := 'text';
  a := LEN(s);
  Expressions := a + b + x + LINT_TO_DINT(l);
END_FUNCTION
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := Expressions();
END_PROGRAM
",
    );
    assert!(text.contains("\"binary\""), "{text}");
    assert!(text.contains("\"unary\""), "{text}");
    assert!(text.contains("expr.size-of"), "{text}");
    assert!(text.contains("expr.address-of"), "{text}");
    assert!(text.contains("place.deref"), "{text}");
    assert!(text.contains("place.bit-field"), "{text}");
    assert!(text.contains("literal.time"), "{text}");
    assert!(text.contains("literal.date"), "{text}");
    assert!(text.contains("literal.string"), "{text}");
}

#[test]
fn declaration_kinds_and_types_are_dumped() {
    let text = dump(
        "TYPE
  Color : (Red, Green := 5, Blue);
  Small : INT(0..10);
  Alias : DINT;
  Point : STRUCT
    x : DINT;
    y : DINT;
  END_STRUCT;
  View : STRUCT OVERLAP
    i : DINT;
    r : REAL;
  END_STRUCT;
  Vec : ARRAY[1..2] OF DINT;
END_TYPE
INTERFACE IShape
METHOD Area : DINT
END_METHOD
END_INTERFACE
CLASS Shape IMPLEMENTS IShape
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Area : DINT
  Area := id;
END_METHOD
END_CLASS
FUNCTION_BLOCK Ticker
VAR PUBLIC
  n : DINT;
END_VAR
END_FUNCTION_BLOCK
PROGRAM P
VAR_TEMP
  temp : DINT;
END_VAR
VAR_EXTERNAL
  seed : DINT;
END_VAR
VAR
  c : Color;
  s : Small;
  a : Alias;
  p : Point;
  v : View;
  vec : Vec;
  shape : Shape;
  iface : IShape;
  fb : Ticker;
END_VAR
  c := Red;
  s := 1;
  a := 2;
  p.x := 3;
  v.i := 4;
  vec[1] := 5;
  shape.id := 6;
  iface := shape;
  fb.n := 7;
  temp := seed + p.x + v.i + vec[1] + shape.id + a + s;
  IF iface <> NULL THEN
    temp := temp + iface.Area();
  END_IF;
END_PROGRAM
VAR_GLOBAL
  seed : DINT;
END_VAR
CONFIGURATION C
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms);
  PROGRAM inst WITH t : P;
END_RESOURCE
END_CONFIGURATION
",
    );
    assert!(text.contains("\"def.function-block\""), "{text}");
    assert!(text.contains("\"def.class\""), "{text}");
}

#[test]
fn calls_and_standard_functions_are_dumped() {
    let text = dump(
        "FUNCTION_BLOCK Ticker
VAR_INPUT
  i : DINT;
END_VAR
VAR_OUTPUT
  o : DINT;
END_VAR
METHOD PUBLIC Bump : DINT
  o := i + 1;
  Bump := o;
END_METHOD
END_FUNCTION_BLOCK
FUNCTION Add : DINT
VAR_INPUT
  a : DINT;
  b : DINT := 2;
END_VAR
  Add := a + b;
END_FUNCTION
FUNCTION Calls : DINT
VAR_INPUT
  p : DINT;
END_VAR
VAR
  t : Ticker;
  x : DINT;
  s : STRING[16];
  r : REAL;
  lr : LREAL;
  bits : BYTE;
  flag : BOOL;
  arr : ARRAY[1..2] OF DINT;
END_VAR
  x := Add(b := 3, a := 2);
  Add(1);
  x := t.Bump();
  x := ABS(-x);
  x := MAX(1, 2, 3);
  x := MIN(1, 2);
  x := LIMIT(0, x, 10);
  x := SEL(flag, 1, 2);
  x := MUX(0, 10, 20);
  x := MOVE(x);
  x := ADD(1, 2);
  x := SUB(2, 1);
  x := MUL(2, 3);
  x := DIV(6, 2);
  x := TRUNC(r);
  bits := SHL(bits, 1);
  bits := SHR(bits, 1);
  bits := ROL(bits, 1);
  bits := ROR(bits, 1);
  r := SQRT(r);
  r := LN(r);
  r := LOG(r);
  r := EXP(r);
  r := SIN(r);
  r := COS(r);
  r := TAN(r);
  r := ASIN(r);
  r := ACOS(r);
  r := ATAN(r);
  r := ATAN2(r, r);
  r := EXPT(r, r);
  lr := SQRT(lr);
  lr := LN(lr);
  lr := LOG(lr);
  lr := EXP(lr);
  lr := SIN(lr);
  lr := COS(lr);
  lr := TAN(lr);
  lr := ASIN(lr);
  lr := ACOS(lr);
  lr := ATAN(lr);
  lr := ATAN2(lr, lr);
  lr := EXPT(lr, lr);
  s := LEFT('abcdef', 3);
  s := RIGHT('abcdef', 2);
  s := MID('abcdef', 2, 1);
  s := CONCAT('ab', 'cd');
  s := INSERT('abcd', 'XY', 2);
  s := DELETE('abcdef', 2, 1);
  s := REPLACE('abcdef', 'XY', 2, 2);
  x := LEN(s);
  x := LEN_MAX(s);
  x := LEN_CODE_UNIT(s);
  x := FIND('abcdef', 'cd');
  flag := GT(1, 2);
  flag := GE(1, 2);
  flag := EQ(1, 2);
  flag := LE(1, 2);
  flag := LT(1, 2);
  flag := NE(1, 2);
  arr[1] := x;
  x := arr[1];
  Calls := x + p;
END_FUNCTION
PROGRAM P
VAR
  x : DINT;
  t : Ticker;
END_VAR
VAR_ACCESS
  ax : x : DINT READ_WRITE;
END_VAR
  x := t.Bump() + Add(1);
END_PROGRAM
",
    );
    assert!(text.contains("call.std-function-sqrt"), "{text}");
    assert!(text.contains("call.std-function-concat"), "{text}");
    assert!(text.contains("call.std-function-len"), "{text}");
}

#[test]
fn declaration_edge_cases_are_dumped() {
    let text = dump(
        "PROGRAM Empty
VAR_INPUT
  i : DINT;
END_VAR
VAR_OUTPUT
  o : DINT;
END_VAR
VAR_TEMP
  t : DINT;
END_VAR
VAR_EXTERNAL
  g : DINT;
END_VAR
END_PROGRAM
VAR_GLOBAL
  g : DINT;
END_VAR
FUNCTION WithLocal : DINT
VAR_TEMP
  temp : DINT;
END_VAR
VAR_EXTERNAL
  g : DINT;
END_VAR
  temp := g;
  WithLocal := temp;
END_FUNCTION
",
    );
    assert!(text.contains("program.body-local"), "{text}");
    assert!(text.contains("program.field-section"), "{text}");
    assert!(text.contains("local.temp"), "{text}");
    assert!(text.contains("local.external"), "{text}");

    let text = dump(
        "VAR_GLOBAL
  g : DINT;
END_VAR
FUNCTION WithLocal : DINT
VAR
  temp : DINT;
END_VAR
  temp := g;
  WithLocal := temp;
END_FUNCTION
PROGRAM Main
VAR RETAIN
  held : DINT;
END_VAR
VAR CONSTANT
  limit : DINT := 10;
END_VAR
VAR
  x : DINT;
END_VAR
VAR_ACCESS
  visible : held : DINT READ_WRITE;
END_VAR
  held := held + 1;
  x := WithLocal() + g + limit;
END_PROGRAM
",
    );
    assert!(text.contains("program.field-retain"), "{text}");
    assert!(text.contains("program.field-constant"), "{text}");
}

#[test]
fn function_block_call_statements_are_dumped_as_unsupported_calls() {
    let text = dump(
        "FUNCTION_BLOCK Ticker
VAR_INPUT
  i : DINT;
END_VAR
END_FUNCTION_BLOCK
PROGRAM P
VAR
  fb : Ticker;
END_VAR
  fb(i := 1);
END_PROGRAM
",
    );
    assert!(text.contains("\"callee.fb\""), "{text}");
    assert!(text.contains("\"kind\":\"unsupported\""), "{text}");
}
