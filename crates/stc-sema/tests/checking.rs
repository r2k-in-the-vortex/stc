//! Focused analyzer checks for standard functions, qualified names, calls,
//! configuration validation, sizing, references, and expression rules.
//!
//! These exercise the semantic rules and their diagnostics directly; the
//! broader conformance corpus covers acceptance cases.

mod common;

use common::{assert_clean, assert_error, assert_errors};
use stc_sema::codes;

#[test]
fn standard_functions_unify_numeric_bit_and_character_arguments() {
    assert_clean(
        "PROGRAM P
VAR
  x : DINT;
  b8 : BYTE;
  w : WORD;
  r : REAL;
  lr : LREAL;
  c : CHAR;
END_VAR
  x := ABS(-3);
  x := MAX(1, 2, 3);
  x := MIN(1, 2);
  b8 := NOT b8;
  w := MAX(WORD#16#0010, BYTE#16#0F);
  c := MAX(CHAR#'a', CHAR#'b');
  r := MAX(1, 2.5);
  lr := MAX(1.0, LREAL#2.0);
  x := SEL(TRUE, 1, 2);
END_PROGRAM
",
    );
}

#[test]
fn standard_function_argument_forms_are_validated() {
    assert_clean(
        "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := MIN(IN2 := 2, IN1 := 1);
END_PROGRAM
",
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := ABS(1, 2);
END_PROGRAM
",
        codes::CALL_ARITY,
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := MIN(1, IN1 := 2);
END_PROGRAM
",
        codes::CALL_ARGUMENT,
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := MIN(BOGUS := 1);
END_PROGRAM
",
        codes::CALL_ARGUMENT,
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := MIN(1, 2 => x);
END_PROGRAM
",
        codes::CALL_ARGUMENT,
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := ABS('a');
END_PROGRAM
",
        codes::TYPE_MISMATCH,
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
  s : STRING[8];
END_VAR
  x := GT(s, 1);
END_PROGRAM
",
        codes::TYPE_INVALID_OPERAND,
    );
}

#[test]
fn task_attributes_fold_constant_expressions() {
    assert_clean(
        "PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := 0;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON CPU
  TASK literal (INTERVAL := T#10ms, PRIORITY := 2);
  TASK negative (INTERVAL := T#20ms, PRIORITY := -1);
  PROGRAM inst WITH literal : Main;
END_RESOURCE
END_CONFIGURATION
",
    );
    // Binary expressions still fold for the value but are rejected as
    // non-constant task attributes.
    assert_error(
        "PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := 0;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON CPU
  TASK arith (INTERVAL := T#1s + T#2s - T#500ms, PRIORITY := (2 ** 3) + (5 MOD 2));
  TASK logic (SINGLE := TRUE AND (FALSE OR (TRUE XOR FALSE)));
  TASK compare (SINGLE := (1 < 2) AND (2 >= 2) AND (3 <> 4) AND (4 = 4) AND (3 > 2) AND (2 <= 2));
  PROGRAM inst WITH arith : Main;
END_RESOURCE
END_CONFIGURATION
",
        codes::CONST_NOT_CONSTANT,
    );
}

#[test]
fn qualified_names_resolve_namespaces_enums_and_values() {
    assert_clean(
        "NAMESPACE N
TYPE E : (A, B); END_TYPE
FUNCTION F : DINT
VAR_INPUT
  a : DINT;
END_VAR
  F := a;
END_FUNCTION
FUNCTION_BLOCK FB
VAR
  x : DINT;
END_VAR
END_FUNCTION_BLOCK
END_NAMESPACE
PROGRAM P
VAR
  x : DINT;
  e : N.E;
END_VAR
  x := N.F(1);
  e := N.E.A;
  e := N.E.B;
  e := N.E;
END_PROGRAM
",
    );
}

#[test]
fn qualified_names_report_invalid_value_contexts() {
    const PRELUDE: &str = "NAMESPACE N
TYPE E : (A, B); END_TYPE
FUNCTION F : DINT
VAR_INPUT
  a : DINT;
END_VAR
  F := a;
END_FUNCTION
FUNCTION_BLOCK FB
VAR
  x : DINT;
END_VAR
END_FUNCTION_BLOCK
END_NAMESPACE
";
    let cases: &[(&str, &str)] = &[
        ("x := N;", codes::NAME_INVALID_CONTEXT),
        ("x := N.F;", codes::NAME_INVALID_CONTEXT),
        ("x := N.FB(1);", codes::CALL_UNKNOWN),
        ("x := Missing.x;", codes::NAME_UNKNOWN),
        ("x := N.Missing;", codes::NAME_UNKNOWN),
    ];
    for (statement, code) in cases {
        let source =
            format!("{PRELUDE}PROGRAM P\nVAR\n  x : DINT;\nEND_VAR\n  {statement}\nEND_PROGRAM\n");
        assert_error(&source, code);
    }
}

#[test]
fn member_access_reports_unknown_and_method_as_value() {
    assert_error(
        "CLASS C
VAR PUBLIC
  x : DINT;
END_VAR
METHOD PUBLIC M : DINT
  M := 1;
END_METHOD
END_CLASS
PROGRAM P
VAR
  c : C;
  x : DINT;
END_VAR
  x := c.M;
END_PROGRAM
",
        codes::NAME_INVALID_CONTEXT,
    );
    assert_error(
        "CLASS C
VAR PUBLIC
  x : DINT;
END_VAR
END_CLASS
PROGRAM P
VAR
  c : C;
  x : DINT;
END_VAR
  x := c.nope;
END_PROGRAM
",
        codes::NAME_UNKNOWN,
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := x.nope;
END_PROGRAM
",
        codes::NAME_UNKNOWN,
    );
}

#[test]
fn plain_callees_report_non_callable_names() {
    let cases: &[(&str, &str)] = &[
        (
            "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := x(1);
END_PROGRAM
",
            codes::CALL_UNKNOWN,
        ),
        (
            "FUNCTION_BLOCK FB
VAR
  x : DINT;
END_VAR
END_FUNCTION_BLOCK
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := FB(1);
END_PROGRAM
",
            codes::CALL_UNKNOWN,
        ),
        (
            "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := TON();
END_PROGRAM
",
            codes::CALL_UNKNOWN,
        ),
        (
            "VAR_GLOBAL
  k : DINT := 1;
END_VAR
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := k(1);
END_PROGRAM
",
            codes::CALL_UNKNOWN,
        ),
    ];
    for (source, code) in cases {
        assert_error(source, code);
    }
}

#[test]
fn call_arguments_validate_in_out_output_and_named_forms() {
    const FB: &str = "FUNCTION_BLOCK FB
VAR_INPUT
  i : DINT;
END_VAR
VAR_IN_OUT
  io : DINT;
END_VAR
VAR_OUTPUT
  q : DINT;
END_VAR
  q := i + io;
END_FUNCTION_BLOCK
";
    let cases: &[(&str, &str)] = &[
        (
            "PROGRAM P
VAR
  f : FB;
  x : DINT;
END_VAR
  f(i := 1, i := 2, io := x);
END_PROGRAM
",
            codes::CALL_ARGUMENT,
        ),
        (
            "PROGRAM P
VAR
  f : FB;
  x : DINT;
END_VAR
  f(missing := 1, i := 1, io := x);
END_PROGRAM
",
            codes::CALL_ARGUMENT,
        ),
        (
            "PROGRAM P
VAR
  f : FB;
  x : DINT;
END_VAR
  f(i => x, i := 1, io := x);
END_PROGRAM
",
            codes::CALL_ARGUMENT,
        ),
        (
            "PROGRAM P
VAR
  f : FB;
  x : DINT;
END_VAR
  f(1 => x, i := 1, io := x);
END_PROGRAM
",
            codes::CALL_ARGUMENT,
        ),
        (
            "PROGRAM P
VAR
  f : FB;
  x : DINT;
END_VAR
  f(i := 1, io := x, q := x);
END_PROGRAM
",
            codes::CALL_ARITY,
        ),
        (
            "PROGRAM P
VAR
  f : FB;
END_VAR
  f();
END_PROGRAM
",
            codes::CALL_ARITY,
        ),
        (
            "PROGRAM P
VAR
  f : FB;
  x : DINT;
END_VAR
  f(i := 1, io := x, q => x, 1);
END_PROGRAM
",
            codes::CALL_ARITY,
        ),
    ];
    for (body, code) in cases {
        let source = format!("{FB}{body}");
        assert_error(&source, code);
    }
}

#[test]
fn configuration_resources_tasks_and_programs_are_validated() {
    const MAIN: &str = "PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := 0;
END_PROGRAM
";
    assert_error(
        &format!(
            "{MAIN}CONFIGURATION C\nRESOURCE R ON CPU\n  TASK t (INTERVAL := T#10ms);\n  PROGRAM inst WITH t : Main;\nEND_RESOURCE\nRESOURCE R ON CPU\n  TASK t (INTERVAL := T#10ms);\n  PROGRAM inst2 WITH t : Main;\nEND_RESOURCE\nEND_CONFIGURATION\n"
        ),
        codes::CONFIG_ERROR,
    );
    assert_error(
        &format!(
            "{MAIN}CONFIGURATION C\nRESOURCE R ON CPU\n  PROGRAM inst : Main;\n  PROGRAM inst : Main;\nEND_RESOURCE\nEND_CONFIGURATION\n"
        ),
        codes::CONFIG_ERROR,
    );
    assert_error(
        &format!(
            "{MAIN}CONFIGURATION C\nRESOURCE R ON CPU\n  TASK t (INTERVAL := T#10ms);\n  TASK t (INTERVAL := T#20ms);\n  PROGRAM inst WITH t : Main;\nEND_RESOURCE\nEND_CONFIGURATION\n"
        ),
        codes::CONFIG_ERROR,
    );
    assert_error(
        &format!(
            "{MAIN}CONFIGURATION C\nRESOURCE R ON CPU\n  TASK t (INTERVAL := T#10ms);\n  PROGRAM inst WITH missing : Main;\nEND_RESOURCE\nEND_CONFIGURATION\n"
        ),
        codes::CONFIG_ERROR,
    );
    assert_error(
        &format!(
            "FUNCTION F : DINT\nVAR_INPUT a : DINT; END_VAR\n  F := a;\nEND_FUNCTION\n{MAIN}CONFIGURATION C\nRESOURCE R ON CPU\n  PROGRAM inst : F;\nEND_RESOURCE\nEND_CONFIGURATION\n"
        ),
        codes::CONFIG_ERROR,
    );
    assert_clean(&format!(
        "{MAIN}CONFIGURATION C\nRESOURCE R ON CPU\n  TASK t (INTERVAL := T#10ms);\n  PROGRAM inst WITH t : Main;\nEND_RESOURCE\nEND_CONFIGURATION\n"
    ));
}

#[test]
fn configuration_without_a_resource_still_checks_programs() {
    assert_clean(
        "PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := 0;
END_PROGRAM
CONFIGURATION C
PROGRAM inst : Main;
END_CONFIGURATION
",
    );
    assert_error(
        "CONFIGURATION C
PROGRAM inst : Missing;
END_CONFIGURATION
",
        codes::CONFIG_ERROR,
    );
}

#[test]
fn var_access_paths_are_resolved_and_typed() {
    const BASE: &str = "PROGRAM Main
VAR
  x : DINT;
  p : REF_TO DINT;
END_VAR
  x := 0;
END_PROGRAM
";
    assert_clean(&format!(
        "{BASE}CONFIGURATION C\nVAR_GLOBAL\n  seed : DINT;\nEND_VAR\nRESOURCE R ON CPU\n  TASK t (INTERVAL := T#10ms);\n  PROGRAM inst WITH t : Main;\nEND_RESOURCE\nVAR_ACCESS\n  seed_path : seed : DINT READ_WRITE;\n  inst_x : inst.x : DINT READ_ONLY;\nEND_VAR\nEND_CONFIGURATION\n"
    ));
    let cases: &[(&str, &str)] = &[
        ("  a : missing : DINT READ_WRITE;", codes::NAME_UNKNOWN),
        ("  a : inst.missing : DINT READ_WRITE;", codes::NAME_UNKNOWN),
        ("  a : missing.x : DINT READ_WRITE;", codes::CONFIG_ERROR),
        ("  a : inst.x : STRING[8] READ_WRITE;", codes::TYPE_MISMATCH),
        ("  a : inst.p : DINT READ_WRITE;", codes::TYPE_MISMATCH),
        (
            "  a : seed : POINTER TO DINT READ_WRITE;",
            codes::TYPE_INVALID,
        ),
    ];
    for (access, code) in cases {
        let source = format!(
            "{BASE}CONFIGURATION C\nVAR_GLOBAL\n  seed : DINT;\nEND_VAR\nRESOURCE R ON CPU\n  TASK t (INTERVAL := T#10ms);\n  PROGRAM inst WITH t : Main;\nEND_RESOURCE\nVAR_ACCESS\n{access}\nEND_VAR\nEND_CONFIGURATION\n"
        );
        assert_error(&source, code);
    }
}

fn find_named_def(hir: &stc_hir::HirCrate, name: &str) -> stc_hir::DefId {
    hir.defs
        .iter()
        .enumerate()
        .find(|(_, def)| def.name_text(&hir.interner).eq_ignore_ascii_case(name))
        .map(|(index, _)| stc_hir::DefId::new(u32::try_from(index).expect("def limit")))
        .unwrap_or_else(|| panic!("missing definition `{name}`"))
}

fn span_text(source: &str, span: stc_span::Span) -> &str {
    &source[span.start as usize..span.end as usize]
}

#[test]
fn program_var_access_keeps_each_path_segment_span() {
    let source = "TYPE Inner : STRUCT\n  x : DINT;\nEND_STRUCT; END_TYPE\nPROGRAM Main\nVAR\n  x : DINT;\n  nested : Inner;\nEND_VAR\nVAR_ACCESS\n  visible : x : DINT READ_ONLY;\n  nested_path : nested.x : DINT READ_ONLY;\nEND_VAR\n  x := 1;\nEND_PROGRAM\n";
    let analysis = assert_clean(source);
    let program = find_named_def(&analysis.hir, "Main");
    let stc_hir::DefKind::Program { var_access, .. } = &analysis.hir.defs[program.index()].kind
    else {
        panic!("expected a program");
    };
    assert_eq!(var_access.len(), 2);
    assert_eq!(span_text(source, var_access[0].path_spans[0]), "x");
    let nested: Vec<&str> = var_access[1]
        .path_spans
        .iter()
        .map(|span| span_text(source, *span))
        .collect();
    assert_eq!(nested, ["nested", "x"]);
}

#[test]
fn configuration_var_access_excludes_the_instance_name() {
    let source = "TYPE Inner : STRUCT\n  x : DINT;\nEND_STRUCT; END_TYPE\nPROGRAM Main\nVAR\n  x : DINT;\n  nested : Inner;\nEND_VAR\n  x := 0;\nEND_PROGRAM\nCONFIGURATION C\nVAR_GLOBAL\n  seed : DINT;\nEND_VAR\nRESOURCE R ON CPU\n  TASK t (INTERVAL := T#10ms);\n  PROGRAM inst WITH t : Main;\nEND_RESOURCE\nVAR_ACCESS\n  seed_path : seed : DINT READ_WRITE;\n  inst_x : inst.x : DINT READ_ONLY;\n  inst_nested : inst.nested.x : DINT READ_ONLY;\nEND_VAR\nEND_CONFIGURATION\n";
    let analysis = assert_clean(source);
    let configuration = find_named_def(&analysis.hir, "C");
    let stc_hir::DefKind::Configuration { config } = &analysis.hir.defs[configuration.index()].kind
    else {
        panic!("expected a configuration");
    };
    assert_eq!(config.var_access.len(), 3);
    assert_eq!(
        span_text(source, config.var_access[0].path_spans[0]),
        "seed"
    );
    assert_eq!(
        span_text(source, config.var_access[1].path_spans[0]),
        "x",
        "the instance name is not part of the field path"
    );
    let nested: Vec<&str> = config.var_access[2]
        .path_spans
        .iter()
        .map(|span| span_text(source, *span))
        .collect();
    assert_eq!(nested, ["nested", "x"]);
}

#[test]
fn var_config_field_spans_align_with_the_field_path() {
    let source = "TYPE Inner : STRUCT\n  x : DINT;\nEND_STRUCT; END_TYPE\nPROGRAM Main\nVAR\n  x : DINT;\n  nested : Inner;\nEND_VAR\n  x := 0;\nEND_PROGRAM\nCONFIGURATION C\nRESOURCE R ON CPU\n  TASK t (INTERVAL := T#10ms);\n  PROGRAM inst WITH t : Main;\nEND_RESOURCE\nVAR_CONFIG\n  inst.nested.x AT %MD10 : DINT := 5;\nEND_VAR\nEND_CONFIGURATION\n";
    let analysis = assert_clean(source);
    let configuration = find_named_def(&analysis.hir, "C");
    let stc_hir::DefKind::Configuration { config } = &analysis.hir.defs[configuration.index()].kind
    else {
        panic!("expected a configuration");
    };
    assert_eq!(config.config_inits.len(), 1);
    let entry = &config.config_inits[0];
    assert_eq!(entry.fields.len(), entry.field_spans.len());
    let spans: Vec<&str> = entry
        .field_spans
        .iter()
        .map(|span| span_text(source, *span))
        .collect();
    assert_eq!(spans, ["nested", "x"]);
}

#[test]
fn var_access_rejects_non_variable_paths() {
    assert_error(
        "FUNCTION F : DINT
VAR_INPUT
  a : DINT;
END_VAR
  F := a;
END_FUNCTION
PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := 0;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms);
  PROGRAM inst WITH t : Main;
END_RESOURCE
VAR_ACCESS
  a : F : DINT READ_WRITE;
END_VAR
END_CONFIGURATION
",
        codes::CONFIG_ERROR,
    );
}

#[test]
fn var_config_entries_are_resolved_and_typed() {
    const BASE: &str = "PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := 0;
END_PROGRAM
";
    assert_clean(&format!(
        "{BASE}CONFIGURATION C\nRESOURCE R ON CPU\n  TASK t (INTERVAL := T#10ms);\n  PROGRAM inst WITH t : Main;\nEND_RESOURCE\nVAR_CONFIG\n  inst.x AT %MD10 : DINT := 5;\nEND_VAR\nEND_CONFIGURATION\n"
    ));
    let cases: &[(&str, &str)] = &[
        ("  nope.x AT %MD10 : DINT;", codes::CONFIG_ERROR),
        ("  inst.missing AT %MD10 : DINT;", codes::NAME_UNKNOWN),
        ("  inst.x : STRING[4];", codes::TYPE_MISMATCH),
    ];
    for (entry, code) in cases {
        let source = format!(
            "{BASE}CONFIGURATION C\nRESOURCE R ON CPU\n  TASK t (INTERVAL := T#10ms);\n  PROGRAM inst WITH t : Main;\nEND_RESOURCE\nVAR_CONFIG\n{entry}\nEND_VAR\nEND_CONFIGURATION\n"
        );
        assert_error(&source, code);
    }
}

#[test]
fn sizeof_requires_a_statically_sized_type_or_place() {
    assert_clean(
        "PROGRAM P
VAR
  x : DINT;
  a : ARRAY[1..3] OF DINT;
  n : UDINT;
END_VAR
  n := SIZEOF(DINT);
  n := SIZEOF(x);
  n := SIZEOF(a[1]);
END_PROGRAM
",
    );
    let cases: &[&str] = &[
        "n := SIZEOF('a');",
        "n := SIZEOF(x.0);",
        "n := SIZEOF(ARRAY[*] OF DINT);",
    ];
    for statement in cases {
        let source = format!(
            "PROGRAM P\nVAR\n  x : DINT;\n  n : UDINT;\nEND_VAR\n  {statement}\nEND_PROGRAM\n"
        );
        assert_error(&source, codes::TYPE_INVALID_OPERAND);
    }
}

#[test]
fn ref_and_address_rules_are_enforced() {
    assert_clean(
        "PROGRAM P
VAR
  x : DINT;
  r : REF_TO DINT;
END_VAR
  r := REF(x);
END_PROGRAM
FUNCTION F : DINT
VAR
  x : DINT;
  p : POINTER TO DINT;
END_VAR
  p := ADR(x);
  F := p^;
END_FUNCTION
",
    );
    assert_error(
        "VAR_GLOBAL CONSTANT
  k : DINT := 1;
END_VAR
PROGRAM P
VAR
  r : REF_TO DINT;
END_VAR
  r := REF(k);
END_PROGRAM
",
        codes::REF_RULE,
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
  r : REF_TO DINT;
END_VAR
  r := REF(1);
END_PROGRAM
",
        codes::REF_RULE,
    );
    assert_error(
        "FUNCTION F : DINT
VAR
  x : DINT;
  p : POINTER TO DINT;
  r : REF_TO POINTER TO DINT;
END_VAR
  r := REF(p);
  F := x;
END_FUNCTION
",
        codes::REF_RULE,
    );
    assert_error(
        "FUNCTION F : DINT
VAR
  x : DINT;
  p : POINTER TO DINT;
END_VAR
  p := ADR(1);
  F := x;
END_FUNCTION
",
        codes::REF_RULE,
    );
}

#[test]
fn unary_and_index_operands_are_validated() {
    assert_clean(
        "PROGRAM P
VAR
  x : DINT;
  b : BOOL;
  b8 : BYTE;
  a : ARRAY[1..3] OF DINT;
END_VAR
  b := NOT b;
  b8 := NOT b8;
  x := -x;
  x := +x;
  a[1] := 2;
END_PROGRAM
",
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := NOT missing;
END_PROGRAM
",
        codes::NAME_UNKNOWN,
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := NOT 5;
END_PROGRAM
",
        codes::TYPE_INVALID_OPERAND,
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
  a : ARRAY[1..3] OF DINT;
END_VAR
  x := a[5];
END_PROGRAM
",
        codes::ARRAY_BOUND,
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
  a : ARRAY[1..3] OF DINT;
END_VAR
  x := a[TRUE];
END_PROGRAM
",
        codes::TYPE_INVALID_OPERAND,
    );
}

#[test]
fn case_and_for_control_rules_are_validated() {
    assert_error(
        "PROGRAM P
VAR
  s : STRING[8];
  x : DINT;
END_VAR
  CASE s OF
    1: x := 1;
  END_CASE;
END_PROGRAM
",
        codes::TYPE_INVALID_OPERAND,
    );
    assert_error(
        "PROGRAM P
VAR
  x : DINT;
END_VAR
  CASE x OF
    5..1: x := 1;
  END_CASE;
END_PROGRAM
",
        codes::CONST_FAILURE,
    );
    assert_error(
        "PROGRAM P
VAR
  s : STRING[8];
END_VAR
  FOR s := 1 TO 2 DO
  END_FOR;
END_PROGRAM
",
        codes::TYPE_INVALID_OPERAND,
    );
    assert_error(
        "PROGRAM P
VAR
  i : DINT;
END_VAR
  FOR i := 1 TO 10 BY 0 DO
  END_FOR;
END_PROGRAM
",
        codes::CONST_FAILURE,
    );
    assert_error(
        "PROGRAM P
VAR
  i : DINT;
END_VAR
  FOR i := 1 TO 10 DO
    i := 5;
  END_FOR;
END_PROGRAM",
        codes::TYPE_INVALID_OPERAND,
    );
}

#[test]
fn constant_and_input_places_are_not_writable() {
    assert_error(
        "VAR_GLOBAL CONSTANT
  k : DINT := 1;
END_VAR
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := k;
  k := 2;
END_PROGRAM
",
        codes::TYPE_ASSIGN,
    );
    assert_error(
        "TYPE Point : STRUCT
  x : DINT;
END_STRUCT; END_TYPE
VAR_GLOBAL CONSTANT
  s : Point := (x := 1);
END_VAR
PROGRAM P
  s.x := 2;
END_PROGRAM
",
        codes::TYPE_ASSIGN,
    );
    assert_error(
        "PROGRAM P
VAR
  input AT %IX0.0 : BOOL;
END_VAR
  input := TRUE;
END_PROGRAM
",
        codes::IO_INPUT_WRITE,
    );
}

#[test]
fn program_var_access_entries_are_validated() {
    assert_clean(
        "PROGRAM Main
VAR
  x : INT;
END_VAR
VAR_ACCESS
  visible : x : INT READ_ONLY;
END_VAR
  x := 1;
END_PROGRAM
",
    );
    let cases: &[(&str, &str)] = &[
        ("  bad : nope : INT READ_ONLY;", codes::NAME_UNKNOWN),
        ("  bad : x : STRING[8] READ_ONLY;", codes::TYPE_MISMATCH),
        ("  bad : x : POINTER TO INT READ_ONLY;", codes::TYPE_INVALID),
    ];
    for (access, code) in cases {
        let source = format!(
            "PROGRAM Main\nVAR\n  x : INT;\nEND_VAR\nVAR_ACCESS\n{access}\nEND_VAR\n  x := 1;\nEND_PROGRAM\n"
        );
        assert_error(&source, code);
    }
}

#[test]
fn this_and_super_member_paths_are_checked() {
    assert_clean(
        "CLASS Inner
VAR PUBLIC
  n : DINT;
END_VAR
METHOD PUBLIC Bump : DINT
  Bump := n;
END_METHOD
END_CLASS
CLASS Holder
VAR PUBLIC
  inner : Inner;
END_VAR
METHOD PUBLIC Run : DINT
  Run := THIS.inner.Bump() + THIS.inner.n;
END_METHOD
END_CLASS
FUNCTION_BLOCK Base
VAR PUBLIC
  b : DINT;
END_VAR
END_FUNCTION_BLOCK
FUNCTION_BLOCK Derived EXTENDS Base
METHOD PUBLIC Get : DINT
  Get := SUPER.b;
END_METHOD
END_FUNCTION_BLOCK
PROGRAM P
VAR
  h : Holder;
  d : Derived;
  x : DINT;
END_VAR
  x := h.Run();
  h.inner.n := x;
  x := d.Get();
  d.b := x;
END_PROGRAM
",
    );
    assert_errors(
        "CLASS Inner
VAR PUBLIC
  n : DINT;
END_VAR
METHOD PUBLIC Bump : DINT
  Bump := n;
END_METHOD
END_CLASS
CLASS Holder
VAR PUBLIC
  inner : Inner;
END_VAR
METHOD PUBLIC Bad : DINT
  Bad := THIS.missing.Bump();
END_METHOD
END_CLASS
FUNCTION_BLOCK Base
VAR PUBLIC
  b : DINT;
END_VAR
END_FUNCTION_BLOCK
FUNCTION_BLOCK Derived EXTENDS Base
METHOD PUBLIC Missing : DINT
  Missing := SUPER.nope;
END_METHOD
END_FUNCTION_BLOCK
PROGRAM P
VAR
  h : Holder;
  d : Derived;
  x : DINT;
END_VAR
  x := h.Bad();
  x := d.Missing();
END_PROGRAM
",
        &[codes::NAME_UNKNOWN],
    );
}

#[test]
fn member_access_on_indexed_class_values_is_checked() {
    assert_error(
        "CLASS Box
VAR PUBLIC
  n : DINT;
END_VAR
METHOD PUBLIC M : DINT
  M := n;
END_METHOD
END_CLASS
PROGRAM P
VAR
  a : ARRAY[1..2] OF Box;
  x : DINT;
END_VAR
  x := a[1].M;
END_PROGRAM
",
        codes::NAME_INVALID_CONTEXT,
    );
    assert_error(
        "CLASS Box
VAR PUBLIC
  n : DINT;
END_VAR
END_CLASS
PROGRAM P
VAR
  a : ARRAY[1..2] OF Box;
  x : DINT;
END_VAR
  x := a[1].nope;
END_PROGRAM
",
        codes::NAME_UNKNOWN,
    );
}

#[test]
fn constants_and_overloads_used_as_values_report_errors() {
    assert_error(
        "VAR_GLOBAL CONSTANT
  k : DINT := 1;
END_VAR
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := k.M(1);
END_PROGRAM
",
        codes::NAME_UNKNOWN,
    );
    assert_error(
        "FUNCTION F : DINT
VAR_INPUT
  a : DINT;
END_VAR
  F := a;
END_FUNCTION
FUNCTION F : REAL
VAR_INPUT
  a : REAL;
END_VAR
  F := a;
END_FUNCTION
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := F;
END_PROGRAM
",
        codes::NAME_AMBIGUOUS,
    );
}

#[test]
fn comparisons_use_a_common_operand_type() {
    assert_clean(
        "PROGRAM P
VAR
  b : BOOL;
  r : REAL;
  l : LREAL;
  i : DINT;
  c : CHAR;
  s : STRING[8];
END_VAR
  b := 1 < 2;
  b := r < 2.5;
  b := l < LREAL#2.5;
  b := i <= 2;
  b := r >= 1.5;
  b := c = CHAR#'a';
  b := s = 'abc';
  b := NOT b;
END_PROGRAM
",
    );
    assert_error(
        "PROGRAM P
VAR
  b : BOOL;
  i : DINT;
END_VAR
  b := i < 2.5;
END_PROGRAM
",
        codes::CONV_EXPLICIT,
    );
}

#[test]
fn subrange_assignments_check_constant_bounds() {
    assert_clean(
        "TYPE Small : INT(0..10); END_TYPE
PROGRAM P
VAR
  s : Small;
END_VAR
  s := 5;
END_PROGRAM
",
    );
    assert_error(
        "TYPE Small : INT(0..10); END_TYPE
PROGRAM P
VAR
  s : Small;
END_VAR
  s := 11;
END_PROGRAM
",
        codes::CONST_FAILURE,
    );
}

#[test]
fn for_parameters_and_enum_case_selectors_are_supported() {
    assert_clean(
        "FUNCTION Count : DINT
VAR_INPUT
  i : DINT;
END_VAR
  FOR i := 1 TO 3 BY 2 DO
  END_FOR;
  Count := i;
END_FUNCTION
TYPE Color : (Red, Green, Blue); END_TYPE
PROGRAM P
VAR
  c : Color;
  x : DINT;
END_VAR
  CASE c OF
    Red: x := 1;
    Green, Blue: x := 2;
  END_CASE;
  x := x + Count(1);
END_PROGRAM
",
    );
}

#[test]
fn program_config_binding_type_mismatches_are_reported() {
    assert_error(
        "PROGRAM Main
VAR_INPUT
  i : DINT;
END_VAR
END_PROGRAM
CONFIGURATION C
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms);
  PROGRAM inst WITH t : Main (i := 'text');
END_RESOURCE
END_CONFIGURATION
",
        codes::TYPE_MISMATCH,
    );
}

#[test]
fn task_attribute_casts_fold_and_keep_the_wrong_type_diagnostic() {
    assert_error(
        "PROGRAM Main
END_PROGRAM
CONFIGURATION C
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms, PRIORITY := 1 + 2.0);
  PROGRAM inst WITH t : Main;
END_RESOURCE
END_CONFIGURATION
",
        codes::TYPE_MISMATCH,
    );
}

#[test]
fn qualified_names_report_non_callable_and_non_value_forms() {
    const PRELUDE: &str = "NAMESPACE N
TYPE E : (A, B); END_TYPE
FUNCTION_BLOCK FB
VAR
  n : DINT;
END_VAR
END_FUNCTION_BLOCK
NAMESPACE M
FUNCTION F : DINT
  F := 0;
END_FUNCTION
END_NAMESPACE
END_NAMESPACE
";
    let cases: &[(&str, &str)] = &[
        ("x := N.M(1);", codes::CALL_UNKNOWN),
        ("x := N.E(1);", codes::CALL_UNKNOWN),
        ("x := N.FB(1);", codes::CALL_UNKNOWN),
        ("x := N.E.Missing;", codes::NAME_INVALID_CONTEXT),
        ("x := N.M;", codes::NAME_INVALID_CONTEXT),
    ];
    for (statement, code) in cases {
        let source =
            format!("{PRELUDE}PROGRAM P\nVAR\n  x : DINT;\nEND_VAR\n  {statement}\nEND_PROGRAM\n");
        assert_error(&source, code);
    }
}

#[test]
fn literal_adaptation_covers_scalar_and_null_targets() {
    assert_clean(
        "TYPE E : (A, B); END_TYPE
CLASS Cls
VAR PUBLIC
  id : DINT;
END_VAR
END_CLASS
VAR_GLOBAL
  s8 : SINT;
  u8 : USINT;
  w : WORD;
  flag : BOOL;
  ch : CHAR;
  str : STRING[2];
  t : TIME;
  d : DATE;
END_VAR
PROGRAM P
VAR
  cl : Cls;
  r : REF_TO DINT;
END_VAR
  s8 := 127;
  u8 := 255;
  w := 65535;
  flag := 1;
  flag := TRUE;
  ch := 65;
  ch := CHAR#'a';
  str := 'hi';
  t := T#1s;
  d := D#2024-01-02;
  cl := NULL;
  r := NULL;
END_PROGRAM
",
    );
    assert_error(
        "TYPE E : (A, B); END_TYPE\nPROGRAM P\nVAR\n  e : E;\nEND_VAR\n  e := 0;\nEND_PROGRAM\n",
        codes::TYPE_ASSIGN,
    );
    assert_error(
        "PROGRAM P
VAR
  b : BOOL;
END_VAR
  b := 2;
END_PROGRAM
",
        codes::TYPE_ASSIGN,
    );
}

#[test]
fn task_attributes_fold_typed_constant_expressions() {
    const MAIN: &str = "PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := 0;
END_PROGRAM
";
    assert_clean(&format!(
        "{MAIN}CONFIGURATION C\nRESOURCE R ON CPU\n  TASK a (INTERVAL := TIME#10ms);\n  TASK b (PRIORITY := INT#1);\n  TASK c (SINGLE := BOOL#TRUE);\n  PROGRAM inst WITH a : Main;\nEND_RESOURCE\nEND_CONFIGURATION\n"
    ));
    for (attribute, code) in [
        ("INTERVAL := DINT#10", codes::TYPE_MISMATCH),
        ("PRIORITY := REAL#1.0", codes::TYPE_MISMATCH),
        ("PRIORITY := CHAR#'a'", codes::TYPE_MISMATCH),
        ("INTERVAL := DINT#1 + 2", codes::TYPE_MISMATCH),
    ] {
        let source = format!(
            "{MAIN}CONFIGURATION C\nRESOURCE R ON CPU\n  TASK t ({attribute});\n  PROGRAM inst WITH t : Main;\nEND_RESOURCE\nEND_CONFIGURATION\n"
        );
        assert_error(&source, code);
    }
}

#[test]
fn in_out_and_output_arguments_reject_invalid_bindings() {
    const FB: &str = "FUNCTION_BLOCK FB
VAR_INPUT
  i : DINT;
END_VAR
VAR_IN_OUT
  io : DINT;
END_VAR
VAR_OUTPUT
  q : DINT;
END_VAR
  q := i + io;
END_FUNCTION_BLOCK
";
    for (body, code) in [
        (
            "PROGRAM P\nVAR\n  f : FB;\n  x : DINT;\n  s : STRING[4];\nEND_VAR\n  f(i := 1, io := x, q => s);\nEND_PROGRAM\n",
            codes::CALL_ARITY,
        ),
        (
            "PROGRAM P\nVAR\n  f : FB;\n  x : DINT;\nEND_VAR\n  f(i := 1, io := x, NOT q => x);\nEND_PROGRAM\n",
            codes::CALL_ARITY,
        ),
        (
            "PROGRAM P\nVAR\n  f : FB;\n  x : DINT;\nEND_VAR\n  f(i := 1, io := x, q => missing);\nEND_PROGRAM\n",
            codes::NAME_UNKNOWN,
        ),
    ] {
        let source = format!("{FB}{body}");
        assert_error(&source, code);
    }
}

#[test]
fn writes_through_constant_aggregates_are_rejected() {
    assert_error(
        "TYPE Point : STRUCT
  x : DINT;
  y : DINT;
END_STRUCT; END_TYPE
VAR_GLOBAL CONSTANT
  pt : Point := (x := 1, y := 2);
END_VAR
PROGRAM P
VAR
  x : DINT;
END_VAR
  pt.x := 1;
END_PROGRAM
",
        codes::TYPE_ASSIGN,
    );
}
