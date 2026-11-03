//! Expression lowering coverage for pointers, casts, time arithmetic, and
//! call statements.

mod common;

use common::{generate, verify_with_clang};
use stc_codegen::Mode;

const POINTERS: &str = r#"
TYPE Node : STRUCT
  value : DINT;
  next : POINTER TO Node;
END_STRUCT; END_TYPE
VAR_GLOBAL
  g : DINT;
  arr : ARRAY[1..4] OF DINT;
  gs : STRING[8];
END_VAR
FUNCTION PtrOps : DINT
VAR
  p : POINTER TO DINT;
  q : POINTER TO DINT;
  pp : POINTER TO POINTER TO DINT;
  r : REF_TO DINT;
  n : DINT;
  x : DINT;
  node : Node;
  buf : ARRAY[1..4] OF DINT;
  ptrs : ARRAY[1..2] OF POINTER TO DINT;
  i : DINT;
END_VAR
  p := ADR(g);
  p^ := 3;
  q := p + 1;
  q := q - 1;
  x := q - p;
  p[0] := 4;
  i := p[1];
  pp := ADR(p);
  x := pp^^;
  r := REF(g);
  p := POINTER TO DINT(r);
  r := REF_TO DINT(p);
  x := SIZEOF(DINT) + SIZEOF(p) + SIZEOF(p^) + SIZEOF(node);
  IF p <> NULL THEN
    x := x + 1;
  END_IF;
  IF p = ADR(g) THEN
    x := x + 1;
  END_IF;
  p := ADR(arr);
  p := ADR(buf);
  node.next := NULL;
  ptrs[1] := p;
  PtrOps := x + i + n;
END_FUNCTION
PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := PtrOps();
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn pointer_expressions_lower() {
    let output = generate(POINTERS, Mode::Executable);
    verify_with_clang("pointers", &output.module);
}

const CASTS_AND_TIME: &str = r#"
TYPE Color : (Red, Green); END_TYPE
PROGRAM Main
VAR
  s8 : SINT;
  i16 : INT;
  d32 : DINT;
  l64 : LINT;
  u8 : USINT;
  u16 : UINT;
  u32 : UDINT;
  u64 : ULINT;
  r : REAL;
  lr : LREAL;
  b : BOOL;
  c : CHAR;
  t : TIME;
  d : DATE;
  x : DINT;
END_VAR
  x := SINT_TO_DINT(s8);
  x := INT_TO_DINT(i16);
  l64 := DINT_TO_LINT(d32);
  x := LINT_TO_DINT(l64);
  x := USINT_TO_DINT(u8);
  x := UINT_TO_DINT(u16);
  x := UDINT_TO_DINT(u32);
  u64 := DINT_TO_ULINT(x);
  x := ULINT_TO_DINT(u64);
  r := DINT_TO_REAL(x);
  lr := REAL_TO_LREAL(r);
  r := LREAL_TO_REAL(lr);
  x := REAL_TO_DINT(r);
  b := DINT_TO_BOOL(x);
  x := BOOL_TO_DINT(b);
  x := DINT_TO_INT(d32);
  d32 := INT_TO_DINT(i16);
  t := T#1s + T#2s;
  t := t - T#500ms;
  t := t * 2;
  t := t / 2;
  b := t > T#1s;
  b := t <= T#2s;
  b := t = T#3s;
  b := t <> T#4s;
  b := d = D#2024-01-02;
  b := d < D#2025-01-02;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn casts_and_time_arithmetic_lower() {
    let output = generate(CASTS_AND_TIME, Mode::Executable);
    verify_with_clang("casts_and_time", &output.module);
}

const CALL_STATEMENTS: &str = r#"
FUNCTION Add : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  Add := a + b;
END_FUNCTION
FUNCTION_BLOCK Ticker
VAR_INPUT
  i : DINT;
END_VAR
VAR_OUTPUT
  o : DINT;
END_VAR
  o := i + 1;
END_FUNCTION_BLOCK
CLASS Box
VAR PUBLIC
  n : DINT;
END_VAR
METHOD PUBLIC M : DINT
  M := n;
END_METHOD
END_CLASS
PROGRAM Main
VAR
  t : Ticker;
  c : Box;
  x : DINT;
END_VAR
  Add(1, 2);
  t(i := 1);
  t(i := 2, o => x);
  c.n := x;
  x := c.M();
  x := x + Add(3, 4) + c.M();
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn call_statements_lower() {
    let output = generate(CALL_STATEMENTS, Mode::Executable);
    verify_with_clang("call_statements", &output.module);
}
