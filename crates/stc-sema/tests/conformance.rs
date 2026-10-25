mod common;

use common::{analyze_sources, assert_clean, assert_error, assert_no_error};

const PROGRAM_SHELL: &str = "PROGRAM P\nVAR\n  x : DINT;\nEND_VAR\n  x := 1;\nEND_PROGRAM\n";

#[test]
fn a_minimal_program_analyzes_cleanly() {
    assert_clean(PROGRAM_SHELL);
}

#[test]
fn functions_calls_and_results_are_resolved() {
    assert_clean(
        "FUNCTION Add : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  Add := a + b;
END_FUNCTION
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := Add(2, 3);
END_PROGRAM
",
    );
}

#[test]
fn overloads_are_selected_by_argument_type() {
    assert_clean(
        "FUNCTION F : DINT
VAR_INPUT a : DINT; END_VAR
  F := a;
END_FUNCTION
FUNCTION F : REAL
VAR_INPUT a : REAL; END_VAR
  F := a;
END_FUNCTION
PROGRAM P
VAR
  x : DINT;
  r : REAL;
END_VAR
  x := F(1);
  r := F(1.5);
END_PROGRAM
",
    );
}

#[test]
fn function_blocks_support_named_and_output_arguments() {
    assert_clean(
        "FUNCTION_BLOCK Counter
VAR_INPUT
  inc : DINT;
END_VAR
VAR_OUTPUT
  q : DINT;
END_VAR
VAR
  cv : DINT;
END_VAR
  cv := cv + inc;
  q := cv;
END_FUNCTION_BLOCK
PROGRAM P
VAR
  c : Counter;
  x : DINT;
END_VAR
  c(inc := 2, q => x);
  c();
END_PROGRAM
",
    );
}

#[test]
fn classes_interfaces_and_dispatch_analyze_cleanly() {
    assert_clean(
        "INTERFACE IShape
METHOD Area : DINT
END_METHOD
END_INTERFACE
CLASS Rect IMPLEMENTS IShape
VAR PUBLIC
  w : DINT;
END_VAR
METHOD PUBLIC Area : DINT
  Area := w * w;
END_METHOD
END_CLASS
FUNCTION_BLOCK Use
VAR
  r : Rect;
  s : IShape;
  x : DINT;
END_VAR
  s := r;
  x := s.Area();
  s ?= r;
END_FUNCTION_BLOCK
",
    );
}

#[test]
fn this_and_super_are_available_in_methods() {
    assert_clean(
        "CLASS Base
METHOD PUBLIC M : DINT
  M := 1;
END_METHOD
END_CLASS
CLASS Derived EXTENDS Base
METHOD PUBLIC OVERRIDE M : DINT
  M := 2;
END_METHOD
METHOD PUBLIC N : DINT
  N := SUPER.M();
END_METHOD
END_CLASS
FUNCTION_BLOCK FB
VAR
  x : DINT;
END_VAR
  x := THIS.x;
END_FUNCTION_BLOCK
",
    );
}

#[test]
fn namespaces_and_using_resolve_across_files() {
    let (analysis, sink) = analyze_sources(&[
        "NAMESPACE ProjectDep
FUNCTION F : DINT
VAR_INPUT a : DINT; END_VAR
  F := a + 1;
END_FUNCTION
END_NAMESPACE
",
        "PROGRAM P
USING ProjectDep;
VAR
  x : DINT;
END_VAR
  x := F(1);
END_PROGRAM
",
    ]);
    assert!(!sink.has_errors(), "{:#?}", sink.diagnostics());
    assert!(analysis.hir.bodies.len() >= 2);
}

#[test]
fn pou_level_using_applies_to_types_and_calls() {
    assert_clean(
        "NAMESPACE Lib
TYPE
  T : DINT;
END_TYPE
FUNCTION F : DINT
  F := 1;
END_FUNCTION
END_NAMESPACE
PROGRAM P
USING Lib;
VAR
  x : T;
END_VAR
  x := F();
END_PROGRAM
",
    );
}

#[test]
fn qualified_names_resolve_without_using() {
    assert_clean(
        "NAMESPACE Lib
FUNCTION F : DINT
  F := 1;
END_FUNCTION
END_NAMESPACE
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := Lib.F();
END_PROGRAM
",
    );
}

#[test]
fn enums_subranges_arrays_and_structs_analyze_cleanly() {
    assert_clean(
        "TYPE
  Color : (Red, Green := 5, Blue);
  Small : INT(0..10);
  Point : STRUCT
    x : DINT;
    y : DINT;
  END_STRUCT;
END_TYPE
PROGRAM P
VAR
  c : Color;
  s : Small;
  a : ARRAY[1..3] OF DINT;
  p : Point := (x := 1, y := 2);
  q : ARRAY[1..2] OF DINT := [10, 20];
  x : DINT;
END_VAR
  c := Green;
  s := 5;
  a[1] := 3;
  p.x := p.y;
  q[2] := x;
  CASE c OF
    Red: x := 1;
    Green: x := 2;
  ELSE
    x := 0;
  END_CASE;
END_PROGRAM
",
    );
}

#[test]
fn standard_functions_and_strings_analyze_cleanly() {
    assert_clean(
        "PROGRAM P
VAR
  s : STRING[10];
  u : USTRING[10];
  x : DINT;
  r : REAL;
END_VAR
  x := LEN(s);
  s := CONCAT('ab', 'cd');
  r := SQRT(4);
  x := ABS(-3);
  u := USTRING#'hi';
  x := BOOL_TO_INT(TRUE);
END_PROGRAM
",
    );
}

#[test]
fn duration_arithmetic_analyzes_cleanly() {
    assert_clean(
        "PROGRAM P
VAR
  t : TIME;
  lt : LTIME;
  d : DINT;
END_VAR
  t := T#1s + T#2s;
  t := t - T#500ms;
  t := t * 2;
  t := 2 * t;
  t := t / 2;
  lt := t + T#1ms;
END_PROGRAM
",
    );
}

#[test]
fn invalid_duration_operands_are_reported() {
    assert_error(
        "PROGRAM P
VAR
  t : TIME;
END_VAR
  t := t + 1;
END_PROGRAM
",
        stc_sema::codes::TYPE_INVALID_OPERAND,
    );
    assert_error(
        "PROGRAM P
VAR
  t : TIME;
END_VAR
  t := t * t;
END_PROGRAM
",
        stc_sema::codes::TYPE_INVALID_OPERAND,
    );
}

#[test]
fn references_and_assignment_attempts_analyze_cleanly() {
    assert_clean(
        "CLASS A
END_CLASS
CLASS B EXTENDS A
END_CLASS
FUNCTION_BLOCK FB
VAR
  a : A;
  b : B;
  p : REF_TO DINT;
  q : REF_TO DINT;
  x : DINT;
END_VAR
  p := REF(x);
  q := p;
  x := q^;
  a := b;
  a ?= b;
END_FUNCTION_BLOCK
",
    );
}

#[test]
fn configuration_bindings_analyze_cleanly() {
    assert_clean(
        "PROGRAM Main
VAR_INPUT
  input : DINT;
END_VAR
VAR
  x : DINT;
END_VAR
VAR_ACCESS
  ax : x : DINT READ_WRITE;
END_VAR
  x := input;
END_PROGRAM
CONFIGURATION C
VAR_GLOBAL
  seed : DINT;
END_VAR
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms, PRIORITY := 1);
  PROGRAM inst WITH t : Main (input := seed);
END_RESOURCE
VAR_ACCESS
  ainst : inst.x : DINT READ_WRITE;
END_VAR
END_CONFIGURATION
",
    );
}

// ---- negative cases ----------------------------------------------------

#[test]
fn unknown_names_are_reported_with_a_suggestion() {
    let (_, sink) = assert_error(
        "PROGRAM P
VAR
  counter : DINT;
END_VAR
  counter := counte;
END_PROGRAM
",
        stc_sema::codes::NAME_UNKNOWN,
    );
    let diagnostic = sink
        .iter()
        .find(|diagnostic| diagnostic.code == stc_sema::codes::NAME_UNKNOWN)
        .expect("unknown-name diagnostic");
    assert!(!diagnostic.suggestions.is_empty(), "{diagnostic:#?}");
}

#[test]
fn duplicate_definitions_are_reported() {
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
  x : DINT;
END_VAR
  x := 1;
END_PROGRAM
",
        stc_sema::codes::NAME_DUPLICATE,
    );
}

#[test]
fn inaccessible_members_are_reported() {
    assert_error(
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
        stc_sema::codes::NAME_INACCESSIBLE,
    );
}

#[test]
fn invalid_contexts_are_reported() {
    assert_error(
        "FUNCTION F : DINT
VAR
  t : DINT;
END_VAR
  t := THIS;
  F := t;
END_FUNCTION
",
        stc_sema::codes::NAME_INVALID_CONTEXT,
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
END_VAR
  EXIT;
  x := 1;
END_PROGRAM
",
        stc_sema::codes::NAME_INVALID_CONTEXT,
    );
}

#[test]
fn ambiguous_imports_are_reported() {
    let (_, sink) = analyze_sources(&[
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
",
        "PROGRAM P
USING A, B;
VAR
  x : DINT;
END_VAR
  x := F();
END_PROGRAM
",
    ]);
    assert_eq!(
        common::error_codes(&sink),
        vec![stc_sema::codes::NAME_AMBIGUOUS]
    );
}

#[test]
fn standard_name_shadowing_warns() {
    let sink = assert_no_error(
        "FUNCTION ABS : DINT
VAR_INPUT a : DINT; END_VAR
  ABS := a;
END_FUNCTION
",
    );
    assert!(
        common::codes(&sink).contains(&stc_sema::codes::NAME_SHADOWS_STANDARD),
        "{:#?}",
        sink.diagnostics()
    );
}

#[test]
fn assignment_type_mismatches_are_reported() {
    assert_error(
        "PROGRAM P
VAR
  flag : BOOL;
  n : DINT;
END_VAR
  flag := n;
END_PROGRAM
",
        stc_sema::codes::TYPE_ASSIGN,
    );
}

#[test]
fn invalid_operands_are_reported() {
    assert_error(
        "PROGRAM P
VAR
  flag : BOOL;
  x : DINT;
END_VAR
  x := 1 + flag;
END_PROGRAM
",
        stc_sema::codes::TYPE_INVALID_OPERAND,
    );
}

#[test]
fn invalid_types_are_reported() {
    assert_error(
        "TYPE
  R : REF_TO REF_TO INT;
END_TYPE
",
        stc_sema::codes::TYPE_INVALID,
    );
}

#[test]
fn non_constant_initializers_are_reported() {
    assert_error(
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
        stc_sema::codes::CONST_NOT_CONSTANT,
    );
}

#[test]
fn constant_evaluation_failures_are_reported() {
    assert_error(
        "TYPE
  Bad : INT(10..0);
END_TYPE
",
        stc_sema::codes::CONST_FAILURE,
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT := 1 / 0;
END_VAR
  x := 1;
END_PROGRAM
",
        stc_sema::codes::CONST_FAILURE,
    );
}

#[test]
fn duplicate_case_labels_are_reported() {
    assert_error(
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
        stc_sema::codes::CONST_DUPLICATE_LABEL,
    );
}

#[test]
fn implicit_lossy_conversions_require_explicit_forms() {
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
  r : REAL;
END_VAR
  x := r;
END_PROGRAM
",
        stc_sema::codes::CONV_EXPLICIT,
    );
}

#[test]
fn impossible_conversions_are_reported() {
    assert_error(
        "PROGRAM P
VAR
  s : STRING[10];
  x : DINT;
END_VAR
  x := STRING_TO_DINT(s);
END_PROGRAM
",
        stc_sema::codes::CONV_NONE,
    );
}

#[test]
fn invalid_callees_are_reported() {
    assert_error(
        "PROGRAM P
VAR
  n : DINT;
  x : DINT;
END_VAR
  x := n();
END_PROGRAM
",
        stc_sema::codes::CALL_UNKNOWN,
    );
}

#[test]
fn missing_and_unknown_arguments_are_reported() {
    assert_error(
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
        stc_sema::codes::CALL_ARITY,
    );
    assert_error(
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
        stc_sema::codes::CALL_ARGUMENT,
    );
}

#[test]
fn ambiguous_overloads_are_reported() {
    assert_error(
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
        stc_sema::codes::CALL_AMBIGUOUS,
    );
}

#[test]
fn inheritance_cycles_are_reported() {
    assert_error(
        "CLASS A EXTENDS B
END_CLASS
CLASS B EXTENDS A
END_CLASS
",
        stc_sema::codes::OOP_CYCLE,
    );
}

#[test]
fn invalid_overrides_are_reported() {
    assert_error(
        "CLASS A
METHOD PUBLIC OVERRIDE M : DINT
  M := 1;
END_METHOD
END_CLASS
",
        stc_sema::codes::OOP_OVERRIDE,
    );
}

#[test]
fn incomplete_interfaces_are_reported() {
    assert_error(
        "INTERFACE I
METHOD M : DINT
END_METHOD
END_INTERFACE
CLASS C IMPLEMENTS I
END_CLASS
",
        stc_sema::codes::OOP_INTERFACE,
    );
}

#[test]
fn abstract_rules_are_enforced() {
    assert_error(
        "CLASS ABSTRACT A
METHOD PUBLIC ABSTRACT M : DINT
END_METHOD
END_CLASS
CLASS B EXTENDS A
END_CLASS
",
        stc_sema::codes::OOP_ABSTRACT,
    );
}

#[test]
fn final_rules_are_enforced() {
    assert_error(
        "CLASS FINAL A
END_CLASS
CLASS B EXTENDS A
END_CLASS
",
        stc_sema::codes::OOP_FINAL,
    );
}

#[test]
fn unrelated_assignment_attempts_are_reported() {
    assert_error(
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
        stc_sema::codes::OOP_ASSIGN_TARGET,
    );
}

#[test]
fn array_bounds_and_ranks_are_checked() {
    assert_error(
        "PROGRAM P
VAR
  a : ARRAY[1..3] OF DINT;
  x : DINT;
END_VAR
  x := a[0];
END_PROGRAM
",
        stc_sema::codes::ARRAY_BOUND,
    );
    assert_error(
        "PROGRAM P
VAR
  a : ARRAY[1..3, 1..2] OF DINT;
  x : DINT;
END_VAR
  x := a[1];
END_PROGRAM
",
        stc_sema::codes::ARRAY_BOUND,
    );
}

#[test]
fn conformant_arrays_are_restricted_to_parameters() {
    assert_error(
        "FUNCTION_BLOCK FB
VAR
  local : ARRAY[*] OF DINT;
END_VAR
END_FUNCTION_BLOCK
",
        stc_sema::codes::ARRAY_CONFORMANT,
    );
}

#[test]
fn reference_rules_are_enforced() {
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := x^;
END_PROGRAM
",
        stc_sema::codes::REF_RULE,
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := NULL^;
END_PROGRAM
",
        stc_sema::codes::REF_NULL,
    );
    assert_error(
        "PROGRAM P
VAR
  p : REF_TO DINT;
  q : REF_TO BOOL;
END_VAR
  p ?= q;
END_PROGRAM
",
        stc_sema::codes::REF_ATTEMPT,
    );
}

#[test]
fn initializer_shapes_are_checked() {
    assert_error(
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
        stc_sema::codes::INIT_SHAPE,
    );
    assert_error(
        "PROGRAM P
VAR
  q : ARRAY[1..2] OF DINT := [1, 2, 3];
END_VAR
  q[1] := 1;
END_PROGRAM
",
        stc_sema::codes::INIT_SHAPE,
    );
}

#[test]
fn configuration_errors_are_reported() {
    assert_error(
        "CONFIGURATION C
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms);
  PROGRAM inst : Missing;
END_RESOURCE
END_CONFIGURATION
",
        stc_sema::codes::CONFIG_ERROR,
    );
}

#[test]
fn at_and_input_write_rules_are_enforced() {
    assert_error(
        "PROGRAM P
VAR
  bad AT %IB0 : BOOL;
END_VAR
  bad := TRUE;
END_PROGRAM
",
        stc_sema::codes::IO_ADDRESS,
    );
    assert_error(
        "PROGRAM P
VAR
  input AT %IX0.0 : BOOL;
END_VAR
  input := TRUE;
END_PROGRAM
",
        stc_sema::codes::IO_INPUT_WRITE,
    );
}

#[test]
fn retain_rules_are_enforced() {
    assert_error(
        "PROGRAM P
VAR RETAIN
  r : REF_TO INT;
END_VAR
  r := NULL;
END_PROGRAM
",
        stc_sema::codes::RETAIN_NOT_PERSISTABLE,
    );
    assert_error(
        "PROGRAM P
VAR_TEMP RETAIN
  t : INT;
END_VAR
  t := 1;
END_PROGRAM
",
        stc_sema::codes::RETAIN_INVALID_LOCATION,
    );
    common::assert_warning(
        "PROGRAM P
VAR RETAIN
  t AT %MW0 : INT;
END_VAR
  t := 1;
END_PROGRAM
",
        stc_sema::codes::RETAIN_DIRECT,
    );
}

#[test]
fn retained_class_instances_are_persistable() {
    // A class value is a handle, but its owned instance's fields are ordinary
    // retained leaves (D-152, State and RETAIN §2.3).
    common::assert_no_error(
        "CLASS Shape
VAR PUBLIC
  id : DINT;
END_VAR
END_CLASS
PROGRAM P
VAR RETAIN
  s : Shape;
END_VAR
  s.id := 1;
END_PROGRAM
",
    );
    assert_error(
        "INTERFACE I
METHOD M : DINT
END_METHOD
END_INTERFACE
PROGRAM P
VAR RETAIN
  i : I;
END_VAR
END_PROGRAM
",
        stc_sema::codes::RETAIN_NOT_PERSISTABLE,
    );
}

#[test]
fn test_only_units_are_excluded_and_references_are_errors() {
    assert_error(
        "{stc:test}
FUNCTION T : DINT
  T := undefined_inside_test;
END_FUNCTION
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := T();
END_PROGRAM
",
        stc_sema::codes::TEST_REFERENCE,
    );
}

#[test]
fn partial_bit_access_is_typed_and_bounds_checked() {
    assert_clean(
        "PROGRAM P
VAR
  b : BYTE;
  x : BOOL;
END_VAR
  b.0 := TRUE;
  x := b.5;
END_PROGRAM
",
    );
    assert_error(
        "PROGRAM P
VAR
  b : BYTE;
  x : BOOL;
END_VAR
  x := b.8;
END_PROGRAM
",
        stc_sema::codes::ARRAY_BOUND,
    );
}

#[test]
fn super_is_not_a_standalone_value() {
    assert_error(
        "CLASS A
END_CLASS
CLASS B EXTENDS A
METHOD PUBLIC M : DINT
  M := SUPER;
END_METHOD
END_CLASS
",
        stc_sema::codes::NAME_INVALID_CONTEXT,
    );
}

#[test]
fn configuration_attributes_and_bindings_are_validated() {
    assert_error(
        "PROGRAM Main
VAR_INPUT
  input : DINT;
END_VAR
VAR_OUTPUT
  output : DINT;
END_VAR
VAR
  scratch : DINT;
END_VAR
  output := input;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON CPU
  TASK t (INTERVAL := 5, PRIORITY := TRUE);
  PROGRAM inst : Main (output := 1, input => scratch);
END_RESOURCE
END_CONFIGURATION
",
        stc_sema::codes::TYPE_MISMATCH,
    );
    assert_error(
        "PROGRAM Main
VAR_INPUT
  input : DINT;
END_VAR
VAR_OUTPUT
  output : DINT;
END_VAR
  output := input;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms);
  PROGRAM inst : Main (output := 1, input => input);
END_RESOURCE
END_CONFIGURATION
",
        stc_sema::codes::CONFIG_ERROR,
    );
}

#[test]
fn functions_must_assign_their_result() {
    assert_error(
        "FUNCTION F : DINT
VAR
  x : DINT;
END_VAR
  x := 1;
END_FUNCTION
",
        stc_sema::codes::TYPE_MISMATCH,
    );
}

#[test]
fn class_fb_and_interface_references_follow_the_rules() {
    assert_clean(
        "INTERFACE I
END_INTERFACE
CLASS C IMPLEMENTS I
END_CLASS
FUNCTION_BLOCK FB IMPLEMENTS I
END_FUNCTION_BLOCK
FUNCTION_BLOCK Use
VAR
  c : C;
  i : I;
  f : FB;
  g : FB;
END_VAR
  i := c;
  i := f;
  i ?= c;
  i ?= f;
  f := g;
END_FUNCTION_BLOCK
",
    );
    assert_error(
        "FUNCTION_BLOCK FB
END_FUNCTION_BLOCK
FUNCTION_BLOCK Use
VAR
  f : FB;
  g : FB;
END_VAR
  f ?= g;
END_FUNCTION_BLOCK
",
        stc_sema::codes::REF_ATTEMPT,
    );
}

#[test]
fn direct_variables_are_typed_and_checked() {
    assert_clean(
        "PROGRAM P
VAR
  x : WORD;
  b : BOOL;
END_VAR
  x := %IW0;
  b := %IX0.0;
  %QW0 := x;
  %MW0 := x;
END_PROGRAM
",
    );
    assert_error(
        "PROGRAM P
VAR
  x : WORD;
END_VAR
  x := %I*;
END_PROGRAM
",
        stc_sema::codes::IO_ADDRESS,
    );
    assert_error(
        "PROGRAM P
VAR
  x : WORD;
END_VAR
  %IW0 := x;
END_PROGRAM
",
        stc_sema::codes::IO_INPUT_WRITE,
    );
}

#[test]
fn overlap_structs_must_share_conflicting_ranges() {
    assert_clean(
        "TYPE
  U : STRUCT OVERLAP
    a AT %MW0 : INT;
    b AT %MW0 : INT;
    c AT %MW1 : INT;
  END_STRUCT;
END_TYPE
PROGRAM P
VAR
  u : U;
END_VAR
  u.a := 1;
END_PROGRAM
",
    );
    assert_error(
        "TYPE
  U : STRUCT OVERLAP
    a AT %MW0 : INT;
    b AT %MD0 : DINT;
  END_STRUCT;
END_TYPE
PROGRAM P
VAR
  u : U;
END_VAR
  u.a := 1;
END_PROGRAM
",
        stc_sema::codes::TYPE_INVALID,
    );
}

#[test]
fn deep_left_associative_expressions_do_not_overflow() {
    let terms = vec!["1"; 20_000].join(" + ");
    let source = format!("PROGRAM P\nVAR\n  x : DINT;\nEND_VAR\n  x := {terms};\nEND_PROGRAM\n");
    assert_clean(&source);
}

#[test]
fn pointer_locals_parameters_and_results_are_supported() {
    assert_clean(
        "TYPE
  Node : STRUCT value : DINT; next : POINTER TO Node; END_STRUCT;
END_TYPE

VAR_GLOBAL
  g : DINT;
  arr : ARRAY[1..4] OF DINT;
  gs : STRING[8];
END_VAR

FUNCTION Deref : DINT
VAR_INPUT
  p : POINTER TO DINT;
END_VAR
  Deref := p^;
END_FUNCTION

FUNCTION Next : POINTER TO DINT
VAR_INPUT
  p : POINTER TO DINT;
END_VAR
  Next := p + 1;
END_FUNCTION

FUNCTION PtrOps : DINT
VAR
  p : POINTER TO DINT;
  pn : POINTER TO DINT := NULL;
  q : POINTER TO DINT;
  pp : POINTER TO POINTER TO DINT;
  ps : POINTER TO STRING[8];
  r : REF_TO DINT;
  n : Node;
  buf : ARRAY[1..4] OF DINT;
  ptrs : ARRAY[1..2] OF POINTER TO DINT;
  x : DINT;
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
  x := SIZEOF(DINT) + SIZEOF(p) + SIZEOF(p^) + SIZEOF(n);
  IF p <> NULL THEN x := x + 1; END_IF;
  IF p = ADR(g) THEN x := x + 1; END_IF;
  p := ADR(arr);
  ps := ADR(gs);
  p := ADR(buf);
  n.next := NULL;
  ptrs[1] := p;
  x := Deref(p) + PtrOpsHelper(Next(p));
  PtrOps := x;
END_FUNCTION

FUNCTION PtrOpsHelper : DINT
VAR_INPUT
  p : POINTER TO DINT;
END_VAR
  PtrOpsHelper := p^;
END_FUNCTION

FUNCTION ConfPtr : DINT
VAR_IN_OUT
  a : ARRAY[*] OF DINT;
END_VAR
VAR
  p : POINTER TO DINT;
  n : DINT;
END_VAR
  p := ADR(a[1]);
  n := SIZEOF(a[1]);
  ConfPtr := p^ + n;
END_FUNCTION

PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := PtrOps();
END_PROGRAM
",
    );
}

#[test]
fn pointer_storage_is_rejected_in_persistent_declarations() {
    assert_error(
        "VAR_GLOBAL
  gp : POINTER TO DINT;
END_VAR
",
        stc_sema::codes::TYPE_INVALID,
    );
    assert_error(
        "PROGRAM P
VAR
  p : POINTER TO DINT;
END_VAR
END_PROGRAM
",
        stc_sema::codes::TYPE_INVALID,
    );
    assert_error(
        "FUNCTION_BLOCK FB
VAR_INPUT
  p : POINTER TO DINT;
END_VAR
END_FUNCTION_BLOCK
",
        stc_sema::codes::TYPE_INVALID,
    );
    assert_error(
        "CLASS C
VAR
  p : POINTER TO DINT;
END_VAR
END_CLASS
",
        stc_sema::codes::TYPE_INVALID,
    );
    assert_error(
        "FUNCTION F : DINT
VAR_IN_OUT
  p : POINTER TO DINT;
END_VAR
  F := 0;
END_FUNCTION
",
        stc_sema::codes::TYPE_INVALID,
    );
    assert_error(
        "TYPE
  Holder : STRUCT p : POINTER TO DINT; END_STRUCT;
END_TYPE
VAR_GLOBAL
  h : Holder;
END_VAR
",
        stc_sema::codes::TYPE_INVALID,
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
END_VAR
VAR_ACCESS
  pa : x : POINTER TO DINT;
END_VAR
END_PROGRAM
",
        stc_sema::codes::TYPE_INVALID,
    );
}

#[test]
fn pointer_type_forms_are_restricted() {
    assert_error(
        "TYPE P : REF_TO POINTER TO INT; END_TYPE
",
        stc_sema::codes::TYPE_INVALID,
    );
    assert_error(
        "TYPE P : POINTER TO REF_TO INT; END_TYPE
",
        stc_sema::codes::TYPE_INVALID,
    );
    assert_error(
        "CLASS C
END_CLASS
TYPE P : POINTER TO C; END_TYPE
",
        stc_sema::codes::TYPE_INVALID,
    );
    assert_error(
        "FUNCTION F : DINT
VAR_IN_OUT
  a : ARRAY[*] OF DINT;
END_VAR
  F := 0;
END_FUNCTION
TYPE P : POINTER TO ARRAY[*] OF DINT; END_TYPE
",
        stc_sema::codes::TYPE_INVALID,
    );
    assert_clean(
        "TYPE P : POINTER TO POINTER TO POINTER TO INT; END_TYPE
PROGRAM Q
VAR x : DINT; END_VAR
END_PROGRAM
",
    );
}

#[test]
fn adr_and_sizeof_target_rules_are_checked() {
    assert_error(
        "FUNCTION F : DINT
VAR
  x : DINT;
END_VAR
  x := ADR(Abs(x));
  F := 0;
END_FUNCTION
",
        stc_sema::codes::REF_RULE,
    );
    assert_error(
        "VAR_GLOBAL CONSTANT
  c : DINT := 1;
END_VAR
FUNCTION F : DINT
  F := ADR(c);
END_FUNCTION
",
        stc_sema::codes::REF_RULE,
    );
    assert_error(
        "FUNCTION F : DINT
VAR
  x : DINT;
  r : REF_TO DINT;
  p : POINTER TO DINT;
END_VAR
  r := REF(x);
  p := ADR(r^);
  F := 0;
END_FUNCTION
",
        stc_sema::codes::REF_RULE,
    );
    assert_error(
        "TYPE
  Point : STRUCT x : DINT; y : DINT; END_STRUCT;
END_TYPE
VAR_GLOBAL
  gp : Point;
END_VAR
FUNCTION F : DINT
VAR
  p : POINTER TO Point;
END_VAR
  p := ADR(gp);
  F := 0;
END_FUNCTION
",
        stc_sema::codes::REF_RULE,
    );
    assert_error(
        "FUNCTION F : DINT
VAR
  p : POINTER TO DINT;
  x : DINT;
  r : REF_TO DINT;
END_VAR
  x := SIZEOF(r);
  F := 0;
END_FUNCTION
",
        stc_sema::codes::TYPE_INVALID_OPERAND,
    );
    assert_error(
        "TYPE
  Point : STRUCT x : DINT; y : DINT; END_STRUCT;
END_TYPE
VAR_GLOBAL
  gp : Point;
END_VAR
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := SIZEOF(gp);
END_PROGRAM
",
        stc_sema::codes::TYPE_INVALID_OPERAND,
    );
    assert_error(
        "FUNCTION F : DINT
VAR
  x : DINT;
  p : POINTER TO DINT;
END_VAR
  x := SIZEOF(x.0);
  F := 0;
END_FUNCTION
",
        stc_sema::codes::TYPE_INVALID_OPERAND,
    );
    assert_error(
        "FUNCTION F : DINT
VAR
  x : DINT;
END_VAR
  x := ADR(%IB0);
  F := 0;
END_FUNCTION
",
        stc_sema::codes::REF_RULE,
    );
    assert_clean(
        "TYPE
  Color : (Red, Green);
  View : STRUCT OVERLAP
    a AT %MW0 : INT;
    b AT %MW1 : INT;
  END_STRUCT;
END_TYPE
VAR_GLOBAL
  c : Color;
  u : View;
END_VAR
FUNCTION F : DINT
VAR
  p : POINTER TO Color;
  q : POINTER TO View;
END_VAR
  p := ADR(c);
  q := ADR(u);
  F := SIZEOF(Color) + SIZEOF(View);
END_FUNCTION
",
    );
}

#[test]
fn pointer_conversions_and_arithmetic_are_typed() {
    assert_clean(
        "FUNCTION F : DINT
VAR
  x : DINT;
  p : POINTER TO DINT;
  r : REF_TO DINT;
  b : BOOL;
END_VAR
  r := REF(x);
  p := POINTER TO DINT(r);
  r := REF_TO DINT(p);
  p := p + 1;
  p := 1 + p;
  p := p - 1;
  b := p = NULL;
  b := p <> NULL;
  b := p < p;
  F := 0;
END_FUNCTION
",
    );
    assert_error(
        "FUNCTION F : DINT
VAR
  x : DINT;
  p : POINTER TO DINT;
  q : POINTER TO BOOL;
END_VAR
  p := q;
  F := 0;
END_FUNCTION
",
        stc_sema::codes::TYPE_ASSIGN,
    );
    assert_error(
        "FUNCTION F : DINT
VAR
  x : DINT;
  p : POINTER TO DINT;
END_VAR
  x := p * 2;
  F := 0;
END_FUNCTION
",
        stc_sema::codes::TYPE_INVALID_OPERAND,
    );
    assert_error(
        "FUNCTION F : DINT
VAR
  p : POINTER TO DINT;
  q : POINTER TO DINT;
END_VAR
  p := p + q;
  F := 0;
END_FUNCTION
",
        stc_sema::codes::TYPE_INVALID_OPERAND,
    );
    assert_error(
        "FUNCTION F : DINT
VAR
  x : DINT;
  p : POINTER TO DINT;
  q : POINTER TO BOOL;
END_VAR
  x := p - q;
  F := 0;
END_FUNCTION
",
        stc_sema::codes::TYPE_INVALID_OPERAND,
    );
    assert_error(
        "FUNCTION F : DINT
VAR
  x : DINT;
  r : REF_TO DINT;
  p : POINTER TO BOOL;
END_VAR
  p := POINTER TO BOOL(r);
  F := 0;
END_FUNCTION
",
        stc_sema::codes::REF_RULE,
    );
    assert_error(
        "FUNCTION F : DINT
VAR
  x : DINT;
  r : REF_TO DINT;
  p : POINTER TO DINT;
END_VAR
  r ?= p;
  F := 0;
END_FUNCTION",
        stc_sema::codes::REF_ATTEMPT,
    );
    assert_error(
        "FUNCTION F : DINT
VAR
  p : POINTER TO DINT;
END_VAR
  p := POINTER TO DINT(1);
  F := 0;
END_FUNCTION
",
        stc_sema::codes::REF_RULE,
    );
    assert_error(
        "FUNCTION F : DINT
VAR
  p : POINTER TO DINT;
  r : REF_TO DINT;
END_VAR
  r := REF(p);
  F := 0;
END_FUNCTION
",
        stc_sema::codes::REF_RULE,
    );
}

#[test]
fn pointers_to_callee_storage_cannot_escape() {
    assert_error(
        "FUNCTION F : POINTER TO DINT
VAR
  x : DINT;
END_VAR
  F := ADR(x);
END_FUNCTION
",
        stc_sema::codes::REF_RULE,
    );
    assert_clean(
        "VAR_GLOBAL
  gv : DINT;
END_VAR
FUNCTION G : POINTER TO DINT
  G := ADR(gv);
END_FUNCTION
FUNCTION H : POINTER TO DINT
VAR_INPUT
  p : POINTER TO DINT;
END_VAR
  H := ADR(p^);
END_FUNCTION
",
    );
}
