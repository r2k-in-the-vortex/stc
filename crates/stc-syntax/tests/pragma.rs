use stc_diag::DiagnosticSink;
use stc_span::{FileId, Span};
use stc_syntax::pragma::{MAX_JSON_DEPTH, parse_pragma};

const FILE: FileId = FileId::new(0);
const SPAN: Span = Span::new(FILE, 0, 1);

fn parse(text: &str) -> (Option<stc_syntax::PragmaAnnotation>, DiagnosticSink) {
    let mut sink = DiagnosticSink::unlimited();
    let annotation = parse_pragma(text, SPAN, true, &mut sink);
    (annotation, sink)
}

fn codes(sink: &DiagnosticSink) -> Vec<&'static str> {
    sink.iter().map(|diagnostic| diagnostic.code).collect()
}

#[test]
fn stc_test_accepts_an_optional_object() {
    let (annotation, sink) = parse("{stc:test {\"cycles\": 25, \"timeout\": \"T#5s\"}}");
    assert!(sink.is_empty(), "{:?}", sink.diagnostics());
    let annotation = annotation.expect("annotation");
    assert_eq!(annotation.name, "test");
    assert_eq!(annotation.payload.unwrap()["cycles"], 25);
}

#[test]
fn stc_test_without_payload_is_valid() {
    let (annotation, sink) = parse("{stc:test}");
    assert!(sink.is_empty());
    assert_eq!(annotation.unwrap().name, "test");
}

#[test]
fn no_payload_pragmas_reject_a_payload() {
    for text in ["{stc:export 1}", "{stc:inline []}", "{stc:no_mangle null}"] {
        let (_, sink) = parse(text);
        assert!(codes(&sink).contains(&"STC-E-PRAGMA-002"), "{text}");
    }
}

#[test]
fn unknown_pragma_names_are_rejected() {
    let (annotation, sink) = parse("{stc:mystery {\"x\": 1}}");
    assert!(annotation.is_none());
    assert!(codes(&sink).contains(&"STC-E-PRAGMA-001"));
}

#[test]
fn non_stc_pragmas_warn_and_are_ignored() {
    let (annotation, sink) = parse("{attribute 'noinit'}");
    assert!(annotation.is_none());
    assert_eq!(codes(&sink), vec!["STC-W-PRAGMA-001"]);
}

#[test]
fn invalid_json_is_reported() {
    for text in [
        "{stc:test {nope}}",
        "{stc:test(cycles := 25)}",
        "{stc:test [1,]}",
    ] {
        let (annotation, sink) = parse(text);
        assert!(annotation.is_none(), "{text}");
        assert!(codes(&sink).contains(&"STC-E-LEX-010"), "{text}");
    }
}

#[test]
fn duplicate_keys_are_reported() {
    let (annotation, sink) = parse("{stc:test {\"cycles\": 1, \"cycles\": 2}}");
    assert!(annotation.is_none());
    assert!(
        codes(&sink).contains(&"STC-E-LEX-011"),
        "{:?}",
        sink.diagnostics()
    );
}

#[test]
fn duplicate_keys_are_detected_in_nested_objects() {
    let (_, sink) = parse("{stc:test {\"outer\": {\"x\": 1, \"x\": 2}}}");
    assert!(codes(&sink).contains(&"STC-E-LEX-011"));
}

#[test]
fn too_deep_payloads_are_rejected() {
    let depth = MAX_JSON_DEPTH + 2;
    let text = format!(
        "{{stc:test {}{}{}}}",
        "[".repeat(depth),
        "1",
        "]".repeat(depth)
    );
    let (annotation, sink) = parse(&text);
    assert!(annotation.is_none());
    assert!(
        codes(&sink).contains(&"STC-E-LEX-012"),
        "{:?}",
        sink.diagnostics()
    );
}

#[test]
fn oversized_bodies_are_rejected() {
    let text = format!("{{stc:test \"{}\"}}", "a".repeat(70 * 1024));
    let (annotation, sink) = parse(&text);
    assert!(annotation.is_none());
    assert!(codes(&sink).contains(&"STC-E-LEX-012"));
}

#[test]
fn io_requires_an_object_payload() {
    for text in ["{stc:io}", "{stc:io []}", "{stc:io \"drive\"}"] {
        let (_, sink) = parse(text);
        assert!(codes(&sink).contains(&"STC-E-IO-004"), "{text}");
    }
}

#[test]
fn io_validates_recognized_keys() {
    let (annotation, sink) =
        parse("{stc:io {\"device\": \"drive1\", \"channel\": 5, \"direction\": \"input\"}}");
    assert!(sink.is_empty(), "{:?}", sink.diagnostics());
    assert_eq!(annotation.unwrap().name, "io");

    for text in [
        "{stc:io {\"channel\": -1}}",
        "{stc:io {\"direction\": \"sideways\"}}",
        "{stc:io {\"device\": 5}}",
    ] {
        let (_, sink) = parse(text);
        assert!(codes(&sink).contains(&"STC-E-IO-004"), "{text}");
    }
}

#[test]
fn unknown_keys_are_warned_and_preserved() {
    let (annotation, sink) = parse("{stc:io {\"device\": \"d\", \"future\": 7}}");
    assert!(codes(&sink).contains(&"STC-W-PRAGMA-002"));
    assert_eq!(
        annotation.unwrap().object().unwrap()["future"].to_string(),
        "7"
    );
}

#[test]
fn test_payload_values_are_validated() {
    for text in [
        "{stc:test {\"cycles\": 0}}",
        "{stc:test {\"cycles\": \"many\"}}",
        "{stc:test {\"timeout\": 5}}",
        "{stc:test {\"expect-trap\": 5}}",
        "{stc:test 5}",
    ] {
        let (_, sink) = parse(text);
        assert!(codes(&sink).contains(&"STC-E-PRAGMA-002"), "{text}");
    }
}

#[test]
fn unterminated_pragmas_are_not_parsed() {
    let mut sink = DiagnosticSink::unlimited();
    let annotation = parse_pragma("{stc:test", SPAN, false, &mut sink);
    assert!(annotation.is_none());
    assert!(sink.is_empty());
}
