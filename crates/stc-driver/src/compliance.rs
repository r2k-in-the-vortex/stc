//! `stc compliance`: profile, feature matrix, extensions, runtime capabilities,
//! and the non-safety statement (CLI-and-Manifest §1.6, Conformance §11).
//!
//! The matrix reports the status of the implementation at this milestone; it
//! does not claim Ed. 4 conformance or any safety function.

use stc_codegen::Json;

use crate::report::Format;

const EDITION: &str = "IEC 61131-3:2013";
const SAFETY: &str = "This project is not a safety PLC and assumes no safety responsibility.";
const EXTENSIONS: [&str; 4] = ["STC-UTF8", "STC-PRAGMA", "STC-TEST", "STC-POINTER"];
const RUNTIME_CAPABILITIES: [&str; 2] = ["online-change", "got-manifest"];

/// Feature categories and their implementation status at M6 phase 3.
const FEATURES: [(&str, &str); 19] = [
    ("structured-text", "implemented"),
    ("lexical", "implemented"),
    ("data-types", "implemented"),
    ("pous", "implemented"),
    ("variables", "implemented"),
    ("statements", "implemented"),
    ("oop", "implemented"),
    ("utf8-strings", "implemented"),
    ("standard-functions", "implemented"),
    ("standard-fbs", "implemented"),
    ("configurations", "partial"),
    ("io-devices", "planned"),
    ("retained-state", "partial"),
    ("project-system", "partial"),
    ("test-runner", "planned"),
    ("formatter", "planned"),
    ("online-change", "partial"),
    ("debug-monitoring", "planned"),
    ("ed4-features", "not-in-profile"),
];

/// Prints the compliance summary.
pub(crate) fn run(format: Format) -> u8 {
    if format.is_json() {
        println!("{}", summary().render());
    } else {
        human();
    }
    0
}

fn summary() -> Json {
    let features = FEATURES
        .iter()
        .map(|(name, status)| ((*name).to_owned(), Json::String((*status).into())))
        .collect::<std::collections::BTreeMap<_, _>>();
    Json::object([
        (
            "profile".into(),
            Json::String(stc_manifest::PROFILE_ID.into()),
        ),
        ("edition".into(), Json::String(EDITION.into())),
        ("ed4_claimed".into(), Json::Bool(false)),
        (
            "extensions".into(),
            Json::Array(
                EXTENSIONS
                    .iter()
                    .map(|extension| Json::String((*extension).into()))
                    .collect(),
            ),
        ),
        (
            "runtime_capabilities".into(),
            Json::Array(
                RUNTIME_CAPABILITIES
                    .iter()
                    .map(|capability| Json::String((*capability).into()))
                    .collect(),
            ),
        ),
        ("features".into(), Json::Object(features)),
        ("safety".into(), Json::String("not-a-safety-plc".into())),
    ])
}

fn human() {
    println!("stc compliance");
    println!("  profile              {}", stc_manifest::PROFILE_ID);
    println!("  edition              {EDITION}");
    println!("  Ed. 4 claimed        no");
    println!("  extensions           {}", EXTENSIONS.join(", "));
    println!("  runtime capabilities {}", RUNTIME_CAPABILITIES.join(", "));
    println!("  features");
    for (name, status) in FEATURES {
        println!("    {name:<20} {status}");
    }
    println!("  safety               {SAFETY}");
}
