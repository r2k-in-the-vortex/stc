//! Place-lowering coverage for persistent aggregates, field tables, objects,
//! standard functions, addresses, and aggregate value passing.

mod common;

use common::{generate, verify_with_clang};
use stc_codegen::Mode;

const PERSISTENT_AGGREGATE: &str = r#"
TYPE Pair : STRUCT
  a : DINT;
  b : DINT;
END_STRUCT; END_TYPE
TYPE Aggregate : STRUCT
  nums : ARRAY[1..4] OF DINT;
  pairs : ARRAY[1..3] OF Pair;
  text : STRING[8];
END_STRUCT; END_TYPE
VAR_GLOBAL
  g : Aggregate;
END_VAR
PROGRAM Main
VAR
  i : DINT;
  x : DINT;
END_VAR
  i := 2;
  g.nums[i] := 1;
  x := g.nums[3];
  g.pairs[i].a := 2;
  g.pairs[i].b := g.pairs[i].a + 1;
  x := g.pairs[2].b + g.pairs[i].a;
  g.text[1] := CHAR#'a';
  x := x + LEN(g.text);
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn persistent_aggregates_lower_dynamic_indices_and_string_elements() {
    let output = generate(PERSISTENT_AGGREGATE, Mode::Executable);
    let module = &output.module;
    assert!(
        output
            .got
            .manifest
            .roots
            .iter()
            .any(|root| root.key.contains(".PAIRS.E2.A")),
        "{:#?}",
        output.got.manifest.roots
    );
    assert!(module.contains("select i1"), "{module}");
    verify_with_clang("persistent_aggregate", module);
}

const PROGRAM_FIELDS: &str = r#"
TYPE Pair : STRUCT
  a : DINT;
  b : DINT;
END_STRUCT; END_TYPE
PROGRAM Main
VAR
  pairs : ARRAY[1..3] OF Pair;
  nums : ARRAY[1..4] OF DINT;
  text : STRING[8];
  i : DINT;
  x : DINT;
END_VAR
  i := 2;
  nums[i] := 1;
  x := nums[3];
  pairs[i].a := 2;
  pairs[i].b := pairs[i].a + 1;
  x := pairs[2].b + pairs[i].a;
  text[1] := CHAR#'a';
  x := x + LEN(text);
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn program_field_tables_lower_dynamic_indices_and_string_elements() {
    let output = generate(PROGRAM_FIELDS, Mode::Executable);
    let module = &output.module;
    assert!(module.contains("select i1"), "{module}");
    verify_with_clang("program_fields", module);
}

const OBJECTS: &str = r#"
CLASS Shape
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
METHOD PUBLIC Bump : DINT
  Bump := n;
END_METHOD
END_FUNCTION_BLOCK
TYPE Holder : STRUCT
  one : Shape;
  arr : ARRAY[1..2] OF Shape;
  counter : Ticker;
END_STRUCT; END_TYPE
VAR_GLOBAL
  h : Holder;
  shapes : ARRAY[1..2] OF Shape;
  gc : Ticker;
END_VAR
PROGRAM Main
VAR
  s : Shape;
  local : ARRAY[1..2] OF Shape;
  c : Ticker;
  i : DINT;
  x : DINT;
END_VAR
  i := 1;
  h.one.id := 1;
  x := h.one.Area();
  h.arr[i].id := 2;
  h.arr[1] := s;
  local[i] := s;
  local[1].id := 3;
  x := x + local[2].id;
  h.counter.n := 4;
  gc.n := 5;
  c.n := 6;
  x := x + c.Bump();
  shapes[i].id := 7;
  s.id := x;
  s := h.one;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn class_and_function_block_places_lower_through_object_records() {
    let output = generate(OBJECTS, Mode::Executable);
    verify_with_clang("objects", &output.module);
}

const MATH: &str = r#"
PROGRAM Main
VAR
  r : REAL;
  l : LREAL;
  x : DINT;
END_VAR
  r := SQRT(4.0);
  r := LN(2.0);
  r := LOG(10.0);
  r := EXP(1.0);
  r := SIN(0.5);
  r := COS(0.5);
  r := TAN(0.5);
  r := ASIN(0.5);
  r := ACOS(0.5);
  r := ATAN(0.5);
  r := ATAN2(1.0, 2.0);
  r := EXPT(2.0, 3.0);
  x := TRUNC(r);
  l := SQRT(LREAL#4.0);
  l := LN(LREAL#2.0);
  l := LOG(LREAL#10.0);
  l := EXP(LREAL#1.0);
  l := SIN(LREAL#0.5);
  l := COS(LREAL#0.5);
  l := TAN(LREAL#0.5);
  l := ASIN(LREAL#0.5);
  l := ACOS(LREAL#0.5);
  l := ATAN(LREAL#0.5);
  l := ATAN2(LREAL#1.0, LREAL#2.0);
  l := EXPT(LREAL#2.0, LREAL#3.0);
  r := ABS(-1.5);
  l := ABS(-LREAL#1.5);
  x := ABS(-2);
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn floating_point_intrinsics_and_abs_lower_for_both_widths() {
    let output = generate(MATH, Mode::Executable);
    let module = &output.module;
    assert!(module.contains("llvm.sqrt.f32"), "{module}");
    assert!(module.contains("llvm.sqrt.f64"), "{module}");
    assert!(module.contains("llvm.pow.f32"), "{module}");
    verify_with_clang("math", module);
}

const ADDRESSES: &str = r#"
TYPE Point : STRUCT
  x : DINT;
  y : DINT;
END_STRUCT; END_TYPE
VAR_GLOBAL
  gp : Point;
  garr : ARRAY[1..4] OF DINT;
  gs : STRING[8];
END_VAR
PROGRAM Main
VAR
  a : ARRAY[1..3] OF DINT;
  s : STRING[8];
  i : DINT;
END_VAR
  a[1] := 1;
  s := 'hi';
  i := a[1] + LEN(s) + LEN(gs) + gp.x + garr[2];
END_PROGRAM
FUNCTION Addresses : DINT
VAR
  q : Point;
  a : ARRAY[1..4] OF DINT;
  s : STRING[8];
  p : POINTER TO DINT;
  ps : POINTER TO STRING[8];
  r : REF_TO DINT;
  n : UDINT;
  i : DINT;
END_VAR
  n := SIZEOF(Point);
  n := SIZEOF(q);
  n := SIZEOF(a);
  n := SIZEOF(a[2]);
  n := SIZEOF(s);
  n := SIZEOF(STRING[8]);
  p := ADR(q.x);
  i := p^;
  p := ADR(a[2]);
  i := i + p^;
  ps := ADR(s);
  i := i + LEN(ps^);
  r := REF(a[3]);
  i := i + r^;
  p := ADR(gp.y);
  i := i + p^;
  p := ADR(garr[1]);
  i := i + p^;
  Addresses := i + UDINT_TO_DINT(n);
END_FUNCTION
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn sizeof_and_address_of_places_lower() {
    let output = generate(ADDRESSES, Mode::Executable);
    verify_with_clang("addresses", &output.module);
}

const AGGREGATE_CALLS: &str = r#"
TYPE Point : STRUCT
  x : DINT;
  y : DINT;
END_STRUCT; END_TYPE
FUNCTION Sum : DINT
VAR_INPUT
  p : Point;
END_VAR
  Sum := p.x + p.y;
END_FUNCTION
FUNCTION_BLOCK Adder
VAR_INPUT
  p : Point;
END_VAR
VAR_IN_OUT
  q : Point;
END_VAR
VAR_OUTPUT
  r : Point;
END_VAR
  r.x := p.x + q.x;
  r.y := p.y + q.y;
  q := r;
END_FUNCTION_BLOCK
PROGRAM Main
VAR
  a : Point;
  b : Point;
  f : Adder;
  x : DINT;
END_VAR
  a.x := 1;
  a.y := 2;
  b.x := 3;
  b.y := 4;
  x := Sum(a);
  f(p := a, q := b);
  a := b;
  x := x + b.x + a.y;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn aggregate_arguments_and_outputs_materialize_and_split() {
    let output = generate(AGGREGATE_CALLS, Mode::Executable);
    verify_with_clang("aggregate_calls", &output.module);
}

const INITIALIZERS: &str = r#"
TYPE Point : STRUCT
  x : DINT;
  y : DINT;
END_STRUCT; END_TYPE
VAR_GLOBAL
  gp : Point := (x := 1, y := 2);
  ga : ARRAY[1..3] OF DINT := [1, 2, 3];
  gs : STRING[8] := 'hi';
END_VAR
PROGRAM Main
VAR
  p : Point := (x := 3, y := 4);
  a : ARRAY[1..3] OF DINT := [4, 5, 6];
  s : STRING[8] := 'yo';
  i : DINT;
END_VAR
  i := p.x + a[1] + LEN(s) + gp.y + ga[2] + LEN(gs);
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn array_and_struct_initializers_lower() {
    let output = generate(INITIALIZERS, Mode::Executable);
    verify_with_clang("initializers", &output.module);
}

const CLASS_AGGREGATES: &str = r#"
TYPE Pair : STRUCT
  a : DINT;
  b : DINT;
END_STRUCT; END_TYPE
CLASS Holder
VAR PUBLIC
  pairs : ARRAY[1..3] OF Pair;
  nums : ARRAY[1..4] OF DINT;
  text : STRING[8];
END_VAR
END_CLASS
VAR_GLOBAL
  gh : Holder;
END_VAR
PROGRAM Main
VAR
  h : Holder;
  i : DINT;
  x : DINT;
END_VAR
  i := 2;
  h.pairs[i].a := 1;
  h.pairs[i].b := h.pairs[i].a + 1;
  x := h.pairs[i].b + h.pairs[1].a;
  h.nums[i] := 2;
  x := x + h.nums[1];
  h.text[i] := CHAR#'a';
  x := x + LEN(h.text);
  gh.pairs[i].a := 3;
  x := x + gh.pairs[i].a;
  gh.nums[i] := 4;
  gh.text[i] := CHAR#'b';
  x := x + LEN(gh.text);
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn class_scoped_aggregates_lower_dynamic_indices_through_object_cursors() {
    let output = generate(CLASS_AGGREGATES, Mode::Executable);
    let module = &output.module;
    assert!(module.contains("select i1"), "{module}");
    verify_with_clang("class_aggregates", module);
}

const STD_FUNCTIONS: &str = r#"
PROGRAM Main
VAR
  x : DINT;
  b8 : BYTE;
  w : WORD;
  s : STRING[16];
  b : BOOL;
END_VAR
  x := ADD(1, 2, 3);
  x := SUB(5, 2);
  x := MUL(2, 3);
  x := DIV(6, 2);
  x := x MOD 2;
  x := MOVE(x);
  b8 := SHL(BYTE#1, 2);
  b8 := SHR(BYTE#8, 1);
  b8 := ROL(BYTE#1, 1);
  b8 := ROR(BYTE#2, 1);
  w := NOT w;
  x := LIMIT(0, x, 10);
  x := MUX(1, 10, 20);
  x := LEN_MAX(s);
  x := LEN_CODE_UNIT(s);
  s := LEFT('abcdef', 3);
  s := RIGHT('abcdef', 2);
  s := MID('abcdef', 2, 1);
  s := CONCAT('ab', 'cd', 'ef');
  s := INSERT('abcd', 'XY', 2);
  s := DELETE('abcdef', 2, 1);
  s := REPLACE('abcdef', 'XY', 2, 2);
  x := FIND('abcdef', 'cd');
  b := GT(1, 2);
  b := GE(1, 2);
  b := EQ(1, 2);
  b := LE(1, 2);
  b := LT(1, 2);
  b := NE(1, 2);
  b := b = (FIND('a', 'a') = 1);
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn standard_function_places_and_intrinsics_lower() {
    let output = generate(STD_FUNCTIONS, Mode::Executable);
    verify_with_clang("std_functions", &output.module);
}

const OVERLAP: &str = r#"
TYPE View : STRUCT OVERLAP
  i : DINT;
  r : REAL;
END_STRUCT; END_TYPE
PROGRAM Main
VAR
  v : View;
  x : DINT;
END_VAR
  v.i := 1;
  x := v.i;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn overlap_fields_report_unsupported_storage() {
    let hir = common::clean_hir(OVERLAP);
    let sources = vec!["<test>".to_string()];
    let errors = stc_codegen::generate(&hir, &common::options("test", Mode::Executable, &sources))
        .expect_err("overlap field places are unsupported");
    assert!(format!("{errors:?}").contains("no storage"), "{errors:?}");
}

const CLASS_RESULT: &str = r#"
CLASS Shape
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Area : DINT
  Area := id;
END_METHOD
END_CLASS
FUNCTION Use : DINT
VAR_INPUT
  p : Shape;
END_VAR
  p.id := p.id + 1;
  Use := p.Area();
END_FUNCTION
PROGRAM Main
VAR
  s : Shape;
  x : DINT;
END_VAR
  s.id := 1;
  x := Use(s) + s.id;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn class_parameters_lower_as_places() {
    let output = generate(CLASS_RESULT, Mode::Executable);
    verify_with_clang("class_result", &output.module);
}

const SPLIT_STORAGE: &str = r#"
CLASS Shape
VAR PUBLIC
  id : DINT;
END_VAR
END_CLASS
FUNCTION_BLOCK Ticker
VAR PUBLIC
  n : DINT;
END_VAR
METHOD PUBLIC Bump : DINT
  Bump := n;
END_METHOD
END_FUNCTION_BLOCK
TYPE Inner : STRUCT
  sh : Shape;
  n : DINT;
END_STRUCT; END_TYPE
TYPE Pair : STRUCT
  a : DINT;
  b : DINT;
END_STRUCT; END_TYPE
TYPE Holder : STRUCT
  counter : Ticker;
END_STRUCT; END_TYPE
VAR_GLOBAL
  ga : ARRAY[1..4] OF DINT;
  gs : STRING[8];
  gm : ARRAY[1..2] OF Inner;
  gh : Holder;
END_VAR
PROGRAM Main
VAR
  i : DINT;
  x : DINT;
  text : STRING[8];
  pairs : ARRAY[1..3] OF Pair;
  local : ARRAY[1..2] OF Inner;
END_VAR
  i := 2;
  ga[i] := 1;
  x := ga[2];
  gs[i] := CHAR#'a';
  x := x + LEN(gs);
  gm[i].n := 2;
  x := x + gm[i].n;
  text[i] := CHAR#'b';
  x := x + LEN(text);
  pairs[i].a := 3;
  x := x + pairs[2].b;
  local[i].n := 4;
  x := x + local[1].n;
  gh.counter.n := 5;
  x := x + gh.counter.Bump();
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn split_arrays_and_top_level_storage_lower_dynamic_steps() {
    let output = generate(SPLIT_STORAGE, Mode::Executable);
    verify_with_clang("split_storage", &output.module);
}

const STANDARD_EXTRA: &str = r#"
PROGRAM Main
VAR
  x : DINT;
  b : BOOL;
  b8 : BYTE;
  w : WORD;
  ws : WSTRING[8];
  s : STRING[8];
  n : USINT;
END_VAR
  b := TRUE;
  n := 2;
  x := SEL(b, 1, 2);
  x := MIN(3, 1, 2);
  x := MAX(3, 1, 2);
  x := LEN_CODE_UNIT(ws) + LEN_MAX(s);
  b8 := SHL(BYTE#1, n);
  b8 := SHR(BYTE#8, n);
  w := ROL(WORD#1, n);
  w := ROR(WORD#2, n);
  x := x + LEN_CODE_UNIT(s);
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn selector_min_max_unicode_and_shift_standard_functions_lower() {
    let output = generate(STANDARD_EXTRA, Mode::Executable);
    verify_with_clang("standard_extra", &output.module);
}

const SPLIT_FIELDS: &str = r#"
CLASS Shape
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
METHOD PUBLIC Bump : DINT
  Bump := n;
END_METHOD
END_FUNCTION_BLOCK
TYPE Inner : STRUCT
  sh : Shape;
  tick : Ticker;
  name : STRING[8];
  nums : ARRAY[1..3] OF DINT;
  n : DINT;
END_STRUCT; END_TYPE
VAR_GLOBAL
  gm : ARRAY[1..2] OF Inner;
END_VAR
FUNCTION UseFb : DINT
VAR_INPUT
  f : Ticker;
END_VAR
VAR
  l : Ticker;
END_VAR
  f.Bump();
  l.Bump();
  UseFb := l.n + f.n;
END_FUNCTION
FUNCTION LocalPlaces : DINT
VAR
  s : STRING[8];
  i : DINT;
  q : Ticker;
END_VAR
  i := 2;
  s[i] := CHAR#'a';
  q.n := 2;
  LocalPlaces := LEN(s) + q.Bump();
END_FUNCTION
PROGRAM Main
VAR
  i : DINT;
  x : DINT;
  t : Ticker;
END_VAR
  i := 2;
  gm[i].n := 5;
  gm[1].name[i] := CHAR#'a';
  gm[1].nums[i] := 3;
  x := LEN(gm[1].name) + gm[1].nums[1];
  x := x + gm[1].tick.Bump();
  gm[1].sh.id := 4;
  x := x + gm[1].sh.Area();
  x := x + UseFb(t);
  x := x + LocalPlaces();
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn split_aggregate_strings_objects_and_parameters_lower() {
    let output = generate(SPLIT_FIELDS, Mode::Executable);
    verify_with_clang("split_fields", &output.module);
}
