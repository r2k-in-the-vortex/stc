//! P0.6: the canonical core-HIR dump is deterministic, versioned, and
//! fails closed on constructs outside the frozen core subset.

use serde_json::Value;
use stc_diag::DiagnosticSink;
use stc_hir::{
    CORE_DUMP_FORMAT, CORE_DUMP_FORMAT_VERSION, CORE_DUMP_PROFILE, CORE_DUMP_SEMANTICS_VERSION,
    CoreDumpSummary, dump_core_hir, dump_hir, verify_core_dump,
};
use stc_lexer::tokenize_bytes;
use stc_parser::{ParseOptions, parse};
use stc_sema::{Analysis, analyze};
use stc_span::{FileId, SourceMap};

fn analyze_with_sources(sources: &[&str]) -> (Analysis, SourceMap) {
    let mut sink = DiagnosticSink::unlimited();
    let mut map = SourceMap::new();
    let mut files = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let file = FileId::new(u32::try_from(index).expect("file limit"));
        let lexed = tokenize_bytes(file, source.as_bytes(), &mut sink);
        let mapped = map.add_file(format!("case{index}.st"), lexed.text.clone());
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
            "unexpected parse errors for:\n{source}\n{:?}",
            sink.diagnostics()
        );
        files.push(parsed);
    }
    let analysis = analyze(&files, &mut sink);
    assert!(
        !sink.has_errors(),
        "unexpected analysis errors:\n{:?}",
        sink.diagnostics()
    );
    analysis
        .hir
        .check_invariants()
        .unwrap_or_else(|message| panic!("HIR invariant: {message}"));
    (analysis, map)
}

fn dump(source: &str) -> (String, Value, CoreDumpSummary) {
    let (analysis, sources) = analyze_with_sources(&[source]);
    let text = dump_core_hir(&analysis.hir, &sources);
    let value: Value = serde_json::from_str(&text).expect("dump is JSON");
    let summary = verify_core_dump(&text).expect("dump validates");
    (text, value, summary)
}

const CORE: &str = "\
VAR_GLOBAL
  g : DINT;
END_VAR
FUNCTION Add : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  Add := a + b;
END_FUNCTION
FUNCTION Twice : DINT
VAR_INPUT
  value : DINT;
END_VAR
  Twice := value * 2;
END_FUNCTION
PROGRAM P
VAR
  x : DINT;
  y : DINT;
END_VAR
  g := g + 1;
  x := Add(b := 3, a := 2);
  y := Twice(x) + g;
END_PROGRAM
";

#[test]
fn accepted_core_dump_is_deterministic_and_versioned() {
    let (first, value, summary) = dump(CORE);
    let (second, _, summary_again) = dump(CORE);

    assert_eq!(first, second, "the dump must be deterministic");
    assert_eq!(summary, summary_again);
    assert!(summary.accepted(), "{:?}", summary.features);
    assert_eq!(summary.format, CORE_DUMP_FORMAT);
    assert_eq!(summary.format_version, CORE_DUMP_FORMAT_VERSION);
    assert_eq!(summary.semantics_version, CORE_DUMP_SEMANTICS_VERSION);
    assert_eq!(summary.profile, CORE_DUMP_PROFILE);
    assert_eq!(summary.package, "_ROOT");
    assert_eq!(summary.status, "ok");

    assert_eq!(value["format"], "stc-core-hir");
    assert_eq!(value["formatVersion"], "0.1.0");
    assert_eq!(value["semanticsVersion"], "1.0");
    assert_eq!(value["profile"], "stc-2013-oop-utf8-v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["features"], Value::Array(Vec::new()));
    assert_eq!(value["unit"]["program"]["name"], "P");

    let functions = value["unit"]["functions"]
        .as_array()
        .expect("function array");
    let names: Vec<&str> = functions
        .iter()
        .map(|function| function["name"].as_str().expect("function name"))
        .collect();
    assert_eq!(names, ["ADD", "TWICE"]);
    assert_eq!(functions[0]["params"][0]["name"], "A");
    assert_eq!(functions[0]["params"][1]["name"], "B");

    let globals = value["unit"]["globals"].as_array().expect("global array");
    assert_eq!(globals.len(), 1);
    assert_eq!(globals[0]["name"], "G");
    assert_eq!(globals[0]["gotKey"], "_ROOT::G");

    let locals = value["unit"]["program"]["locals"]
        .as_array()
        .expect("program locals");
    assert_eq!(locals.len(), 2);
    assert_eq!(locals[0]["name"], "X");
    assert_eq!(locals[1]["name"], "Y");

    let first_assign = &value["unit"]["program"]["body"][0];
    assert_eq!(first_assign["kind"], "assign");
    assert_eq!(first_assign["place"]["varKind"], "global");
    assert_eq!(first_assign["place"]["name"], "G");
    assert_eq!(first_assign["place"]["gotKey"], "_ROOT::G");
    assert_eq!(first_assign["value"]["kind"], "binary");

    let second_assign = &value["unit"]["program"]["body"][1];
    assert_eq!(second_assign["kind"], "assign");
    assert_eq!(second_assign["place"]["varKind"], "local");
    assert_eq!(second_assign["place"]["position"], 0);
    assert_eq!(second_assign["place"]["name"], "X");
}

#[test]
fn function_arguments_are_emitted_in_formal_order() {
    let (_, value, summary) = dump(CORE);
    assert!(summary.accepted());
    let calls = collect_calls(&value);
    let add = calls
        .iter()
        .find(|call| call["callee"]["name"] == "ADD")
        .expect("the ADD call is dumped");
    let args = add["args"].as_array().expect("call arguments");
    assert_eq!(args.len(), 2);
    // Source order is `b := 3, a := 2`; the dump is formal order A, B.
    assert_eq!(args[0]["value"], 2);
    assert_eq!(args[1]["value"], 3);
}

#[test]
fn core_dump_does_not_expose_rust_identity_or_debug_text() {
    let (text, _, summary) = dump(CORE);
    assert!(summary.accepted());
    for forbidden in ["DefId", "ExprId", "StmtId", "LocalId", "ParamId", "expr#"] {
        assert!(
            !text.contains(forbidden),
            "the canonical dump must not contain `{forbidden}`:\n{text}"
        );
    }
    // The human-readable debug dump remains available and separate.
    let (analysis, _) = analyze_with_sources(&[CORE]);
    let debug = dump_hir(&analysis.hir);
    assert!(debug.contains("def 0"), "{debug}");
}

#[test]
fn non_core_types_are_rejected_not_interpreted() {
    let (text, value, summary) = dump(
        "PROGRAM P
VAR
  r : REAL;
  x : DINT;
END_VAR
  x := 1;
END_PROGRAM
",
    );
    assert!(!summary.accepted());
    assert_eq!(value["status"], "rejected");
    assert!(
        summary.features.iter().any(|tag| tag == "type.real"),
        "{:?}",
        summary.features
    );
    let features = value["features"].as_array().expect("features");
    assert!(!features.is_empty());
    assert!(features[0]["tag"].is_string());
    assert!(features[0]["detail"].is_string());
    // The rejected dump is still structurally valid.
    assert!(verify_core_dump(&text).is_ok());
}

#[test]
fn non_core_standard_functions_are_rejected() {
    let (_, value, summary) = dump(
        "PROGRAM P
VAR
  x : BYTE;
  y : BYTE;
END_VAR
  y := SHL(x, 1);
END_PROGRAM
",
    );
    assert!(!summary.accepted());
    assert_eq!(value["status"], "rejected");
    assert!(
        summary
            .features
            .iter()
            .any(|tag| tag == "call.std-function-shl"),
        "{:?}",
        summary.features
    );
}

#[test]
fn unknown_versions_and_malformed_dumps_are_rejected() {
    let (text, _, summary) = dump(CORE);
    assert!(summary.accepted());

    let wrong_version = text.replace("\"formatVersion\":\"0.1.0\"", "\"formatVersion\":\"9.9.9\"");
    assert_ne!(text, wrong_version);
    let error = verify_core_dump(&wrong_version).expect_err("unknown versions must fail");
    assert!(error.contains("format version"), "{error}");

    let wrong_semantics = text.replace(
        "\"semanticsVersion\":\"1.0\"",
        "\"semanticsVersion\":\"2.0\"",
    );
    let error = verify_core_dump(&wrong_semantics).expect_err("unknown semantics must fail");
    assert!(error.contains("semantics version"), "{error}");

    let error = verify_core_dump("{}").expect_err("a bare object is not a dump");
    assert!(error.contains("format"), "{error}");

    let wrong_status = text.replace("\"status\":\"ok\"", "\"status\":\"rejected\"");
    let error = verify_core_dump(&wrong_status).expect_err("status must match the features");
    assert!(error.contains("does not match"), "{error}");
}

fn collect_calls(value: &Value) -> Vec<&Value> {
    let mut calls = Vec::new();
    walk(value, &mut calls);
    calls
}

fn walk<'a>(value: &'a Value, calls: &mut Vec<&'a Value>) {
    match value {
        Value::Object(object) => {
            if object.get("kind").and_then(Value::as_str) == Some("call")
                && object.get("callee").is_some()
            {
                calls.push(value);
            }
            for child in object.values() {
                walk(child, calls);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(item, calls);
            }
        }
        _ => {}
    }
}
