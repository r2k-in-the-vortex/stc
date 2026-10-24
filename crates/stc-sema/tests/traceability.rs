mod common;

use common::analyze_source;
use stc_sema::codes;

/// Every analyzer-owned code maps to at least one negative case here.
/// `STC-E-IO-004` is raised by the pragma parser and is covered by
/// `crates/stc-syntax/tests/pragma.rs`.
fn cases() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            codes::NAME_UNKNOWN,
            "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := missing;
END_PROGRAM
",
        ),
        (
            codes::NAME_DUPLICATE,
            "PROGRAM P
VAR
  x : DINT;
  x : DINT;
END_VAR
  x := 1;
END_PROGRAM
",
        ),
        (
            codes::NAME_INACCESSIBLE,
            "CLASS C
VAR PRIVATE
  secret : DINT;
END_VAR
END_CLASS
FUNCTION_BLOCK FB
VAR
  c : C;
END_VAR
  c.secret := 1;
END_FUNCTION_BLOCK
",
        ),
        (
            codes::NAME_INVALID_CONTEXT,
            "PROGRAM P
VAR
  x : DINT;
END_VAR
  EXIT;
  x := 1;
END_PROGRAM
",
        ),
        (
            codes::NAME_AMBIGUOUS,
            "NAMESPACE A
FUNCTION F : DINT
  F := 1;
END_FUNCTION
END_NAMESPACE
NAMESPACE B
FUNCTION F : DINT
  F := 2;
END_FUNCTION
END_NAMESPACE
PROGRAM P
USING A, B;
VAR
  x : DINT;
END_VAR
  x := F();
END_PROGRAM
",
        ),
        (
            codes::NAME_SHADOWS_STANDARD,
            "FUNCTION ABS : DINT
VAR_INPUT a : DINT; END_VAR
  ABS := a;
END_FUNCTION
",
        ),
        (
            codes::TYPE_MISMATCH,
            "PROGRAM P
VAR
  flag : BOOL;
  x : DINT;
END_VAR
  IF x THEN
    flag := TRUE;
  END_IF;
END_PROGRAM
",
        ),
        (
            codes::TYPE_INVALID_OPERAND,
            "PROGRAM P
VAR
  flag : BOOL;
  x : DINT;
END_VAR
  x := 1 + flag;
END_PROGRAM
",
        ),
        (
            codes::TYPE_INVALID,
            "TYPE
  R : REF_TO REF_TO INT;
END_TYPE
",
        ),
        (
            codes::TYPE_ASSIGN,
            "PROGRAM P
VAR
  flag : BOOL;
  n : DINT;
END_VAR
  flag := n;
END_PROGRAM
",
        ),
        (
            codes::CONST_NOT_CONSTANT,
            "VAR_GLOBAL
  y : DINT;
END_VAR
PROGRAM P
VAR
  x : DINT := y;
END_VAR
  x := 1;
END_PROGRAM
",
        ),
        (
            codes::CONST_FAILURE,
            "TYPE
  Bad : INT(10..0);
END_TYPE
",
        ),
        (
            codes::CONST_DUPLICATE_LABEL,
            "PROGRAM P
VAR
  x : DINT;
END_VAR
  CASE x OF
    1: x := 1;
    1: x := 2;
  END_CASE;
END_PROGRAM
",
        ),
        (
            codes::CONV_NONE,
            "PROGRAM P
VAR
  s : STRING[10];
  x : DINT;
END_VAR
  x := STRING_TO_DINT(s);
END_PROGRAM
",
        ),
        (
            codes::CONV_EXPLICIT,
            "PROGRAM P
VAR
  x : DINT;
  r : REAL;
END_VAR
  x := r;
END_PROGRAM
",
        ),
        (
            codes::CALL_UNKNOWN,
            "PROGRAM P
VAR
  n : DINT;
  x : DINT;
END_VAR
  x := n();
END_PROGRAM
",
        ),
        (
            codes::CALL_ARITY,
            "FUNCTION F : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  F := a + b;
END_FUNCTION
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := F(a := 1);
END_PROGRAM
",
        ),
        (
            codes::CALL_ARGUMENT,
            "FUNCTION F : DINT
VAR_INPUT
  a : DINT;
END_VAR
  F := a;
END_FUNCTION
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := F(c := 1);
END_PROGRAM
",
        ),
        (
            codes::CALL_AMBIGUOUS,
            "FUNCTION F : DINT
VAR_INPUT a : INT; END_VAR
  F := a;
END_FUNCTION
FUNCTION F : DINT
VAR_INPUT a : DINT; END_VAR
  F := a;
END_FUNCTION
PROGRAM P
VAR
  s : SINT;
  x : DINT;
END_VAR
  x := F(s);
END_PROGRAM
",
        ),
        (
            codes::OOP_CYCLE,
            "CLASS A EXTENDS B
END_CLASS
CLASS B EXTENDS A
END_CLASS
",
        ),
        (
            codes::OOP_OVERRIDE,
            "CLASS A
METHOD PUBLIC OVERRIDE M : DINT
  M := 1;
END_METHOD
END_CLASS
",
        ),
        (
            codes::OOP_INTERFACE,
            "INTERFACE I
METHOD M : DINT
END_METHOD
END_INTERFACE
CLASS C IMPLEMENTS I
END_CLASS
",
        ),
        (
            codes::OOP_ABSTRACT,
            "CLASS ABSTRACT A
METHOD PUBLIC ABSTRACT M : DINT
END_METHOD
END_CLASS
CLASS B EXTENDS A
END_CLASS
",
        ),
        (
            codes::OOP_FINAL,
            "CLASS FINAL A
END_CLASS
CLASS B EXTENDS A
END_CLASS
",
        ),
        (
            codes::OOP_ASSIGN_TARGET,
            "CLASS A
END_CLASS
CLASS B
END_CLASS
FUNCTION_BLOCK FB
VAR
  a : A;
  b : B;
END_VAR
  a ?= b;
END_FUNCTION_BLOCK
",
        ),
        (
            codes::ARRAY_BOUND,
            "PROGRAM P
VAR
  a : ARRAY[1..3] OF DINT;
  x : DINT;
END_VAR
  x := a[0];
END_PROGRAM
",
        ),
        (
            codes::ARRAY_CONFORMANT,
            "FUNCTION_BLOCK FB
VAR
  local : ARRAY[*] OF DINT;
END_VAR
END_FUNCTION_BLOCK
",
        ),
        (
            codes::REF_RULE,
            "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := x^;
END_PROGRAM
",
        ),
        (
            codes::REF_NULL,
            "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := NULL^;
END_PROGRAM
",
        ),
        (
            codes::REF_ATTEMPT,
            "PROGRAM P
VAR
  p : REF_TO DINT;
  q : REF_TO BOOL;
END_VAR
  p ?= q;
END_PROGRAM
",
        ),
        (
            codes::INIT_SHAPE,
            "TYPE
  Point : STRUCT
    x : DINT;
  END_STRUCT;
END_TYPE
PROGRAM P
VAR
  p : Point := (z := 1);
END_VAR
  p.x := 1;
END_PROGRAM
",
        ),
        (
            codes::CONFIG_ERROR,
            "CONFIGURATION C
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms);
  PROGRAM inst : Missing;
END_RESOURCE
END_CONFIGURATION
",
        ),
        (
            codes::IO_ADDRESS,
            "PROGRAM P
VAR
  bad AT %IB0 : BOOL;
END_VAR
  bad := TRUE;
END_PROGRAM
",
        ),
        (
            codes::IO_INPUT_WRITE,
            "PROGRAM P
VAR
  input AT %IX0.0 : BOOL;
END_VAR
  input := TRUE;
END_PROGRAM
",
        ),
        (
            codes::RETAIN_NOT_PERSISTABLE,
            "PROGRAM P
VAR RETAIN
  r : REF_TO INT;
END_VAR
  r := NULL;
END_PROGRAM
",
        ),
        (
            codes::RETAIN_INVALID_LOCATION,
            "PROGRAM P
VAR_TEMP RETAIN
  t : INT;
END_VAR
  t := 1;
END_PROGRAM
",
        ),
        (
            codes::RETAIN_DIRECT,
            "PROGRAM P
VAR RETAIN
  t AT %MW0 : INT;
END_VAR
  t := 1;
END_PROGRAM
",
        ),
        (
            codes::EXPORT_INVALID,
            "{stc:no_mangle}
FUNCTION F : DINT
  F := 1;
END_FUNCTION
PROGRAM P
VAR x : DINT; END_VAR
END_PROGRAM
",
        ),
        (
            codes::TEST_REFERENCE,
            "{stc:test}
FUNCTION T : DINT
  T := 1;
END_FUNCTION
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := T();
END_PROGRAM
",
        ),
    ]
}

#[test]
fn every_analyzer_code_has_a_negative_case() {
    let mut missing = Vec::new();
    for (code, source) in cases() {
        let (_, sink) = analyze_source(source);
        let found = sink.iter().any(|diagnostic| diagnostic.code == code);
        if !found {
            missing.push(format!(
                "{code}: not produced; got {:?}",
                common::codes(&sink)
            ));
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn the_table_covers_every_exported_semantic_code() {
    let table: Vec<&str> = cases().iter().map(|(code, _)| *code).collect();
    let exported = [
        codes::NAME_UNKNOWN,
        codes::NAME_SHADOWS_STANDARD,
        codes::NAME_DUPLICATE,
        codes::NAME_INACCESSIBLE,
        codes::NAME_INVALID_CONTEXT,
        codes::NAME_AMBIGUOUS,
        codes::TYPE_MISMATCH,
        codes::TYPE_INVALID_OPERAND,
        codes::TYPE_INVALID,
        codes::TYPE_ASSIGN,
        codes::CONST_NOT_CONSTANT,
        codes::CONST_FAILURE,
        codes::CONST_DUPLICATE_LABEL,
        codes::CONV_NONE,
        codes::CONV_EXPLICIT,
        codes::CALL_UNKNOWN,
        codes::CALL_ARITY,
        codes::CALL_ARGUMENT,
        codes::CALL_AMBIGUOUS,
        codes::OOP_CYCLE,
        codes::OOP_OVERRIDE,
        codes::OOP_INTERFACE,
        codes::OOP_ABSTRACT,
        codes::OOP_FINAL,
        codes::OOP_ASSIGN_TARGET,
        codes::ARRAY_BOUND,
        codes::ARRAY_CONFORMANT,
        codes::REF_RULE,
        codes::REF_NULL,
        codes::REF_ATTEMPT,
        codes::INIT_SHAPE,
        codes::CONFIG_ERROR,
        codes::IO_ADDRESS,
        codes::IO_INPUT_WRITE,
        codes::RETAIN_NOT_PERSISTABLE,
        codes::RETAIN_INVALID_LOCATION,
        codes::RETAIN_DIRECT,
        codes::EXPORT_INVALID,
        codes::TEST_REFERENCE,
    ];
    for code in exported {
        assert!(
            table.contains(&code),
            "missing traceability case for {code}"
        );
    }
}
