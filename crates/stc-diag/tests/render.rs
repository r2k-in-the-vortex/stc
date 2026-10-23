use stc_diag::{Diagnostic, DiagnosticSink, render_human, render_json};
use stc_span::{SourceMap, Span};

fn fixture() -> (SourceMap, Span) {
    let mut map = SourceMap::new();
    let file = map.add_file("src/main.st", "VAR\n  flag := counter;\nEND_VAR\n");
    let start = "VAR\n  flag := ".len() as u32;
    let span = Span::new(file, start, start + "counter".len() as u32);
    (map, span)
}

fn sample_diagnostic(span: Span) -> Diagnostic {
    Diagnostic::error("STC-E-TYPE-012", "cannot assign DINT to BOOL", span)
        .with_label(span, "expected BOOL, found DINT")
        .with_note("implicit BOOL conversion is not defined for DINT")
        .with_suggestion("use (counter <> 0)", span, "(counter <> 0)")
}

#[test]
fn human_render_matches_the_documented_shape() {
    let (map, span) = fixture();
    let rendered = render_human(&sample_diagnostic(span), &map);
    let expected = "\
src/main.st:2:11: error[STC-E-TYPE-012]: cannot assign DINT to BOOL
  |
2 |   flag := counter;
  |           ^^^^^^^ expected BOOL, found DINT
  |
  = note: implicit BOOL conversion is not defined for DINT
  = help: use (counter <> 0): `(counter <> 0)`
";
    assert_eq!(rendered, expected);
}

#[test]
fn human_render_handles_warnings_without_notes() {
    let (map, span) = fixture();
    let diagnostic = Diagnostic::warning("STC-W-LEX-008", "deprecated", span);
    let rendered = render_human(&diagnostic, &map);
    assert!(rendered.starts_with("src/main.st:2:11: warning[STC-W-LEX-008]: deprecated\n"));
    assert!(!rendered.contains("= note:"));
    assert!(!rendered.contains("= help:"));
}

#[test]
fn human_render_prints_secondary_labels() {
    let (map, span) = fixture();
    let first = Span::new(span.file, 0, 3);
    let diagnostic =
        Diagnostic::error("STC-E-X", "problem", span).with_label(first, "declared here");
    let rendered = render_human(&diagnostic, &map);
    assert!(rendered.contains("1 | VAR"));
    assert!(rendered.contains("^^^^^^^")); // primary caret width
    assert!(rendered.contains("^^^ declared here"));
}

#[test]
fn json_render_matches_the_documented_shape() {
    let (map, span) = fixture();
    let rendered = render_json(&sample_diagnostic(span), &map);
    let expected = concat!(
        r#"{"code":"STC-E-TYPE-012","severity":"error","message":"cannot assign DINT to BOOL","#,
        r#""file":"src/main.st","span":{"start":14,"end":21,"line":2,"column":11},"#,
        r#""labels":[{"span":{"start":14,"end":21},"message":"expected BOOL, found DINT"}],"#,
        r#""notes":["implicit BOOL conversion is not defined for DINT"],"#,
        r#""suggestions":[{"message":"use (counter <> 0)","span":{"start":14,"end":21},"replacement":"(counter <> 0)"}]}"#,
    );
    assert_eq!(rendered, expected);
}

#[test]
fn json_render_escapes_strings() {
    let mut map = SourceMap::new();
    let file = map.add_virtual_file("v", "abc");
    let span = Span::new(file, 0, 3);
    let diagnostic =
        Diagnostic::error("STC-E-X", "say \"hi\"\nnext\tend\\", span).with_note("line\nbreak");
    let rendered = render_json(&diagnostic, &map);
    assert!(
        rendered.contains(r#""message":"say \"hi\"\nnext\tend\\""#),
        "{rendered}"
    );
    assert!(
        rendered.contains(r#""notes":["line\nbreak"]"#),
        "{rendered}"
    );
    assert!(rendered.contains(r#""file":"<v>""#), "{rendered}");
}

#[test]
fn sink_counts_errors_and_suppresses_beyond_limit() {
    let mut map = SourceMap::new();
    let file = map.add_virtual_file("v", "abc");
    let span = Span::new(file, 0, 1);

    let mut sink = DiagnosticSink::with_limit(2);
    sink.error("STC-E-A", "first", span);
    sink.warning("STC-W-B", "second", span);
    sink.error("STC-E-C", "third", span);
    assert_eq!(sink.len(), 2);
    assert_eq!(sink.suppressed(), 1);
    assert_eq!(sink.suppressed_errors(), 1);
    assert!(sink.has_errors());
    assert_eq!(sink.error_count(), 1);

    let taken = sink.take();
    assert_eq!(taken.len(), 2);
    assert!(sink.is_empty());
    assert_eq!(sink.suppressed(), 0);
    assert_eq!(sink.suppressed_errors(), 0);
}

#[test]
fn suppressed_errors_still_report_failure() {
    let mut map = SourceMap::new();
    let file = map.add_virtual_file("v", "");
    let span = Span::empty(file, 0);
    let mut sink = DiagnosticSink::with_limit(1);
    sink.warning("STC-W-X", "warning fills the limit", span);
    sink.error("STC-E-X", "error is suppressed", span);
    assert_eq!(sink.len(), 1);
    assert_eq!(sink.error_count(), 0);
    assert_eq!(sink.suppressed_errors(), 1);
    assert!(sink.has_errors());
}

#[test]
fn unlimited_sink_keeps_everything() {
    let mut map = SourceMap::new();
    let file = map.add_virtual_file("v", "");
    let span = Span::empty(file, 0);
    let mut sink = DiagnosticSink::unlimited();
    for _ in 0..150 {
        sink.warning("STC-W-X", "many", span);
    }
    assert_eq!(sink.len(), 150);
    assert_eq!(sink.suppressed(), 0);
    assert!(!sink.has_errors());
}
