//! Span retention for renameable manifest and profile values (D-253).

use stc_manifest::{ProfileEndpoint, parse_manifest, parse_profile};
use stc_span::{FileId, Span};

fn slice(text: &str, span: Span) -> &str {
    &text[span.start as usize..span.end as usize]
}

const BASIC: &str = r#"schema = 1

[package]
name = "app"
version = "0.1.0"
language = "stc-2013-oop-utf8-v1"

[project]
sources = ["src/**/*.st"]
configuration = "Main"

[io]

[[io.devices]]
name = "drive1"
kind = "ethercat"

[[io.bindings]]
variable = "Main.MyBool"
device = "drive1"

[test]

[[test.case]]
name = "TimerBlink"
"#;

#[test]
fn basic_strings_and_arrays_of_tables_keep_content_spans() {
    let parsed = parse_manifest(BASIC, FileId::new(0)).expect("valid manifest");
    let manifest = parsed.manifest;

    let configuration_span = manifest
        .project
        .configuration_span
        .expect("configuration span");
    assert_eq!(slice(BASIC, configuration_span), "Main");

    assert_eq!(manifest.io.bindings.len(), 1);
    assert_eq!(
        slice(BASIC, manifest.io.bindings[0].variable_span),
        "Main.MyBool"
    );

    assert_eq!(manifest.test.cases.len(), 1);
    assert_eq!(slice(BASIC, manifest.test.cases[0].name_span), "TimerBlink");
}

const BOM_LITERAL_MULTILINE: &str = "\u{feff}schema = 1

[package]
name = \"app\"
version = \"0.1.0\"
language = \"stc-2013-oop-utf8-v1\"

[project]
sources = [\"src/**/*.st\"]
configuration = '''Main'''

[io]

[[io.devices]]
name = \"drive1\"
kind = \"ethercat\"

[[io.bindings]]
variable = 'Main.MyBool'
device = \"drive1\"

[test]

[[test.case]]
name = \"\"\"TimerBlink\"\"\"
";

#[test]
fn literal_multiline_and_bom_offsets_are_original_text_offsets() {
    let parsed = parse_manifest(BOM_LITERAL_MULTILINE, FileId::new(0)).expect("valid manifest");
    let manifest = parsed.manifest;

    let configuration_span = manifest
        .project
        .configuration_span
        .expect("configuration span");
    assert_eq!(slice(BOM_LITERAL_MULTILINE, configuration_span), "Main");
    assert_eq!(
        configuration_span.start as usize,
        BOM_LITERAL_MULTILINE
            .find("'''Main'''")
            .expect("configuration")
            + 3
    );

    assert_eq!(
        slice(BOM_LITERAL_MULTILINE, manifest.io.bindings[0].variable_span),
        "Main.MyBool"
    );
    assert_eq!(
        slice(BOM_LITERAL_MULTILINE, manifest.test.cases[0].name_span),
        "TimerBlink"
    );
}

#[test]
fn missing_configuration_has_no_span() {
    let text = BASIC.replace("configuration = \"Main\"\n", "");
    let parsed = parse_manifest(&text, FileId::new(0)).expect("valid manifest");
    assert!(parsed.manifest.project.configuration.is_none());
    assert!(parsed.manifest.project.configuration_span.is_none());
}

const PROFILE: &str = r#"[[routes]]
sinks = [
  { kind = "plc-input", variable = "Main.In0" },
]

[routes.source]
kind = "plc-output"
variable = 'Main.Out'
"#;

#[test]
fn profile_endpoints_keep_variable_content_spans() {
    let profile = parse_profile(PROFILE, FileId::new(0)).expect("valid profile");
    assert_eq!(profile.routes.len(), 1);
    let route = &profile.routes[0];

    let ProfileEndpoint::PlcOutput {
        variable,
        variable_span,
        ..
    } = &route.source
    else {
        panic!("expected a plc-output source");
    };
    assert_eq!(variable, "Main.Out");
    assert_eq!(slice(PROFILE, *variable_span), "Main.Out");

    let ProfileEndpoint::PlcInput {
        variable,
        variable_span,
        ..
    } = &route.sinks[0]
    else {
        panic!("expected a plc-input sink");
    };
    assert_eq!(variable, "Main.In0");
    assert_eq!(slice(PROFILE, *variable_span), "Main.In0");
}

#[test]
fn profile_multiline_and_bom_offsets_are_original_text_offsets() {
    let text = format!("\u{feff}{PROFILE}");
    let text = text.replace("\"Main.In0\"", "\"\"\"Main.In0\"\"\"");
    let profile = parse_profile(&text, FileId::new(0)).expect("valid profile");
    let ProfileEndpoint::PlcInput { variable_span, .. } = &profile.routes[0].sinks[0] else {
        panic!("expected a plc-input sink");
    };
    assert_eq!(slice(&text, *variable_span), "Main.In0");
    assert_eq!(
        slice(
            &text,
            match &profile.routes[0].source {
                ProfileEndpoint::PlcOutput { variable_span, .. } => *variable_span,
                _ => panic!("expected a plc-output source"),
            }
        ),
        "Main.Out"
    );
}
