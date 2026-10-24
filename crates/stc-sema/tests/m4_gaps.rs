mod common;

use common::{analyze_source, error_codes};
use stc_hir::{ConstValue, DefKind, ExprKindHir};

fn clean(source: &str) -> stc_sema::Analysis {
    let (analysis, sink) = analyze_source(source);
    let errors: Vec<_> = sink
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .collect();
    assert!(
        errors.is_empty(),
        "unexpected errors for:\n{source}\n{errors:#?}"
    );
    analysis
}

#[test]
fn io_hints_are_attached_to_globals_and_fields() {
    let analysis = clean(
        r#"VAR_GLOBAL
  g AT %IW2 : INT {stc:io {"device":"rack","channel":5,"direction":"input"}};
END_VAR
PROGRAM Main
VAR
  f AT %IX0.0 : BOOL {stc:io {"device":"rack","channel":1,"direction":"output"}};
END_VAR
END_PROGRAM
"#,
    );
    let hir = &analysis.hir;
    let global = hir
        .defs
        .iter()
        .find(|def| def.name_text(&hir.interner) == "g")
        .expect("global");
    let DefKind::Global { io: Some(hint), .. } = &global.kind else {
        panic!("global hint missing: {global:?}");
    };
    assert_eq!(hint.device.as_deref(), Some("rack"));
    assert_eq!(hint.channel, Some(5));
    assert_eq!(hint.direction, Some(stc_hir::IoDirection::Input));
    assert!(hint.payload.contains("\"channel\":5"), "{}", hint.payload);

    let field = hir
        .fields
        .iter()
        .find(|field| hir.interner.text(field.name) == "f")
        .expect("field");
    let hint = field.io.as_ref().expect("field hint");
    assert_eq!(hint.device.as_deref(), Some("rack"));
    assert_eq!(hint.channel, Some(1));
    assert_eq!(hint.direction, Some(stc_hir::IoDirection::Output));
}

#[test]
fn export_records_a_stable_c_name() {
    let analysis = clean(
        r#"{stc:export}{stc:no_mangle}
FUNCTION Foo : DINT
VAR_INPUT a : DINT; END_VAR
  Foo := a;
END_FUNCTION
PROGRAM Main
VAR x : DINT; END_VAR
END_PROGRAM
"#,
    );
    let hir = &analysis.hir;
    let def = hir
        .defs
        .iter()
        .find(|def| def.name_text(&hir.interner) == "Foo")
        .expect("function");
    assert!(def.export);
    assert!(def.no_mangle);
    assert_eq!(
        def.export_name.map(|name| hir.interner.text(name)),
        Some("Foo")
    );

    let generated = clean(
        r#"{stc:export}
FUNCTION Bar : DINT
  Bar := 1;
END_FUNCTION
PROGRAM Main
VAR x : DINT; END_VAR
END_PROGRAM
"#,
    );
    let hir = &generated.hir;
    let def = hir
        .defs
        .iter()
        .find(|def| def.name_text(&hir.interner) == "Bar")
        .expect("function");
    assert_eq!(
        def.export_name.map(|name| hir.interner.text(name)),
        Some("stc_export_BAR")
    );
}

#[test]
fn invalid_exports_are_diagnosed() {
    let (_, sink) = analyze_source(
        r#"{stc:no_mangle}
FUNCTION Foo : DINT
  Foo := 1;
END_FUNCTION
PROGRAM Main
VAR x : DINT; END_VAR
END_PROGRAM
"#,
    );
    assert!(
        error_codes(&sink).contains(&"STC-E-EXPORT-001"),
        "{:#?}",
        sink.diagnostics()
    );

    let (_, sink) = analyze_source(
        r#"{stc:export}
PROGRAM Main
VAR x : DINT; END_VAR
END_PROGRAM
"#,
    );
    assert!(
        error_codes(&sink).contains(&"STC-E-EXPORT-001"),
        "{:#?}",
        sink.diagnostics()
    );

    let (_, sink) = analyze_source(
        r#"{stc:export}{stc:no_mangle}
FUNCTION stc_hidden : DINT
  stc_hidden := 1;
END_FUNCTION
PROGRAM Main
VAR x : DINT; END_VAR
END_PROGRAM
"#,
    );
    assert!(
        error_codes(&sink).contains(&"STC-E-EXPORT-001"),
        "{:#?}",
        sink.diagnostics()
    );

    let (_, sink) = analyze_source(
        r#"{stc:export}
FUNCTION F : DINT
VAR_INPUT a : DINT; END_VAR
  F := a;
END_FUNCTION
{stc:export}
FUNCTION F : DINT
VAR_INPUT a : INT; END_VAR
  F := a;
END_FUNCTION
PROGRAM Main
VAR x : DINT; END_VAR
  x := F(1);
END_PROGRAM
"#,
    );
    assert!(
        error_codes(&sink).contains(&"STC-E-EXPORT-001"),
        "{:#?}",
        sink.diagnostics()
    );
}

#[test]
fn task_attributes_are_folded() {
    let analysis = clean(
        r#"PROGRAM Main
VAR x : DINT; END_VAR
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  TASK Slow (SINGLE := TRUE, INTERVAL := T#10ms, PRIORITY := 2);
  PROGRAM p WITH Slow : Main;
END_RESOURCE
END_CONFIGURATION
"#,
    );
    let hir = &analysis.hir;
    let config = hir
        .defs
        .iter()
        .find_map(|def| match &def.kind {
            DefKind::Configuration { config } => Some(config),
            _ => None,
        })
        .expect("configuration");
    let task = &config.resources[0].tasks[0];
    assert_eq!(task.single_value, Some(true));
    assert_eq!(task.interval_ns, Some(10_000_000));
    assert_eq!(task.priority_value, Some(2));
}

#[test]
fn ldate_literals_are_nanoseconds() {
    let analysis = clean(
        r#"PROGRAM Main
VAR
  days : DATE := DATE#1970-01-02;
  long : LDATE := LDATE#1970-01-02;
  converted : LDATE := DATE_TO_LDATE(DATE#1970-01-02);
END_VAR
END_PROGRAM
"#,
    );
    let hir = &analysis.hir;
    let value_of = |name: &str| {
        let field = hir
            .fields
            .iter()
            .find(|field| hir.interner.text(field.name) == name)
            .expect("field");
        let init = field.init.expect("initializer");
        match &hir.expr(init).kind {
            ExprKindHir::Literal(literal) => literal.value.clone(),
            other => panic!("expected literal, got {other:?}"),
        }
    };
    assert_eq!(
        value_of("days"),
        ConstValue::Date {
            kind: stc_hir::DateTy::Date,
            value: 1
        }
    );
    assert_eq!(
        value_of("long"),
        ConstValue::Date {
            kind: stc_hir::DateTy::LDate,
            value: 86_400_000_000_000
        }
    );
    assert_eq!(
        value_of("converted"),
        ConstValue::Date {
            kind: stc_hir::DateTy::LDate,
            value: 86_400_000_000_000
        }
    );
}
