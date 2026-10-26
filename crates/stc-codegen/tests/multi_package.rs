//! Multi-package code generation: GOT keys and mangled symbols use package
//! identities while dependency aliases only select namespaces (D-153).

use stc_diag::DiagnosticSink;
use stc_lexer::tokenize_bytes;
use stc_parser::{ParseOptions, ParsedFile, parse};
use stc_sema::{PackageSession, ProjectSession, analyze_project};
use stc_span::FileId;

const LIB: &str = "\
VAR_GLOBAL G : DINT; END_VAR
TYPE T : DINT; END_TYPE
FUNCTION Double : DINT
VAR_INPUT x : DINT; END_VAR
  Double := x * 2;
END_FUNCTION
";

const APP: &str = "\
VAR_GLOBAL AppG : DINT; END_VAR
PROGRAM Main
VAR x : drive.T; END_VAR
  drive.G := drive.G + 1;
  x := drive.Double(drive.G);
  AppG := x;
END_PROGRAM
";

struct Parsed {
    lib: Vec<ParsedFile>,
    app: Vec<ParsedFile>,
}

fn parse_sources(sink: &mut DiagnosticSink) -> Parsed {
    let mut next = 0_u32;
    let mut parse_one = |source: &str| {
        let id = FileId::new(next);
        next += 1;
        let lexed = tokenize_bytes(id, source.as_bytes(), sink);
        parse(
            id,
            lexed.text.as_str(),
            &lexed.stream,
            ParseOptions::default(),
            sink,
        )
    };
    Parsed {
        lib: vec![parse_one(LIB)],
        app: vec![parse_one(APP)],
    }
}

fn generate() -> (stc_codegen::Output, stc_sema::Analysis) {
    let mut sink = DiagnosticSink::unlimited();
    let parsed = parse_sources(&mut sink);
    assert!(!sink.has_errors(), "{:#?}", sink.diagnostics());
    let packages = vec![
        PackageSession {
            name: "motor-lib".into(),
            root_namespace: "drive".into(),
            dependencies: Vec::new(),
            files: &parsed.lib,
            is_root: false,
        },
        PackageSession {
            name: "app".into(),
            root_namespace: "app".into(),
            dependencies: vec![0],
            files: &parsed.app,
            is_root: true,
        },
    ];
    let session = ProjectSession {
        packages,
        configuration: None,
        stdlib: true,
        test_build: false,
        test_files: &[],
    };
    let analysis = analyze_project(&session, &mut sink);
    assert!(
        sink.iter()
            .filter(|diagnostic| diagnostic.is_error())
            .count()
            == 0,
        "{:#?}",
        sink.diagnostics()
    );
    let sources = vec!["lib/src/lib.st".to_string(), "app/src/main.st".to_string()];
    let source_paths = vec!["src/lib.st".to_string(), "src/main.st".to_string()];
    let fingerprint = stc_codegen::module::project_source_fingerprint(&[
        (
            "motor-lib".into(),
            "src/lib.st".into(),
            LIB.as_bytes().to_vec(),
        ),
        ("app".into(), "src/main.st".into(), APP.as_bytes().to_vec()),
    ])
    .expect("source fingerprint");
    let options = stc_codegen::Options {
        artifact_name: "two-package",
        target: Some("x86_64-unknown-linux-gnu"),
        debug: false,
        emit_metadata: true,
        mode: stc_codegen::Mode::Library,
        relaxed: false,
        release: false,
        sources: &sources,
        source_paths: Some(&source_paths),
        test_build: false,
        module: stc_codegen::ModuleBuildOptions {
            compiler_version: "stc test".into(),
            source_fingerprint: fingerprint,
            ..stc_codegen::ModuleBuildOptions::default()
        },
        project: stc_codegen::ProjectOptions::default(),
    };
    let output = stc_codegen::generate(&analysis.hir, &options)
        .unwrap_or_else(|errors| panic!("codegen failed: {errors:#?}"));
    (output, analysis)
}

#[test]
fn catalog_keys_use_package_identities_not_aliases() {
    let (output, _) = generate();
    let roots: Vec<&str> = output
        .got
        .catalog
        .roots
        .iter()
        .map(|root| root.key.as_str())
        .collect();
    assert!(roots.contains(&"MOTOR-LIB::G"), "{roots:?}");
    assert!(roots.contains(&"APP::APPG"), "{roots:?}");

    let functions: Vec<&str> = output
        .got
        .slots
        .iter()
        .filter(|slot| !slot.key.is_empty())
        .map(|slot| slot.key.as_str())
        .collect();
    assert!(
        functions
            .iter()
            .any(|key| key.starts_with("MOTOR-LIB::DOUBLE@")),
        "{functions:?}"
    );
    assert!(
        functions
            .iter()
            .any(|key| key.starts_with("APP::MAIN#BODY@")),
        "{functions:?}"
    );
}

#[test]
fn mangled_symbols_include_the_package_identity() {
    let (output, _) = generate();
    assert!(
        output.module.contains("@stc_fn_MOTOR_u002D_LIB_n_DOUBLE_"),
        "{}",
        output.module
    );
    assert!(
        output.module.contains("@stc_prog_APP_n_MAIN_body"),
        "{}",
        output.module
    );
    // The program body calls the dependency through its function slot; the
    // slot's descriptor symbol carries the package identity.
    let function_table = output.got.function_table();
    assert!(
        function_table
            .iter()
            .any(|(_, symbol)| symbol.starts_with("stc_fn_MOTOR_u002D_LIB_n_DOUBLE_")),
        "{function_table:?}"
    );
    assert!(
        output.got.manifest_json.contains("MOTOR-LIB::DOUBLE@"),
        "{}",
        output.got.manifest_json
    );
}

#[test]
fn source_locations_are_package_qualified_and_package_relative() {
    let (output, _) = generate();
    let global = output
        .got
        .catalog
        .roots
        .iter()
        .find(|root| root.key == "MOTOR-LIB::G")
        .expect("library global root");
    let source = global.source.as_ref().expect("source location");
    assert_eq!(source.package, "MOTOR-LIB");
    assert_eq!(source.file, "src/lib.st");

    let app_global = output
        .got
        .catalog
        .roots
        .iter()
        .find(|root| root.key == "APP::APPG")
        .expect("application global root");
    let source = app_global.source.as_ref().expect("source location");
    assert_eq!(source.package, "APP");
    assert_eq!(source.file, "src/main.st");

    assert_eq!(
        output.metadata.spans.files,
        vec!["src/lib.st".to_string(), "src/main.st".to_string()]
    );
}

#[test]
fn project_source_fingerprint_depends_on_package_identity() {
    let bytes = b"PROGRAM P END_PROGRAM".to_vec();
    let first = stc_codegen::module::project_source_fingerprint(&[(
        "motor-lib".into(),
        "src/main.st".into(),
        bytes.clone(),
    )])
    .expect("fingerprint");
    let second = stc_codegen::module::project_source_fingerprint(&[(
        "other-lib".into(),
        "src/main.st".into(),
        bytes,
    )])
    .expect("fingerprint");
    assert_ne!(first, second);
}
