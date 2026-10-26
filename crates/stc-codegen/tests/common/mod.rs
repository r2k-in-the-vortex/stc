#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use stc_diag::DiagnosticSink;
use stc_hir::HirCrate;
use stc_lexer::tokenize_bytes;
use stc_parser::{ParseOptions, parse};
use stc_sema::{Analysis, analyze};
use stc_span::FileId;

pub fn analyze_source(source: &str) -> (Analysis, DiagnosticSink) {
    let mut sink = DiagnosticSink::unlimited();
    let file = FileId::new(0);
    let lexed = tokenize_bytes(file, source.as_bytes(), &mut sink);
    let parsed = parse(
        file,
        lexed.text.as_str(),
        &lexed.stream,
        ParseOptions::default(),
        &mut sink,
    );
    assert!(
        !sink.has_errors(),
        "parse errors for:\n{source}\n{:?}",
        sink.diagnostics()
    );
    let analysis = analyze(&[parsed], &mut sink);
    analysis
        .hir
        .check_invariants()
        .unwrap_or_else(|message| panic!("HIR invariant: {message}"));
    (analysis, sink)
}

pub fn clean_hir(source: &str) -> HirCrate {
    let (analysis, sink) = analyze_source(source);
    let errors: Vec<_> = sink
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .collect();
    assert!(
        errors.is_empty(),
        "semantic errors for:\n{source}\n{errors:#?}"
    );
    analysis.hir
}

pub fn options<'a>(
    name: &'a str,
    mode: stc_codegen::Mode,
    sources: &'a [String],
) -> stc_codegen::Options<'a> {
    stc_codegen::Options {
        artifact_name: name,
        // A fixed target keeps descriptors, manifests, and golden snapshots
        // identical on Windows and Linux CI hosts.
        target: Some("x86_64-unknown-linux-gnu"),
        debug: false,
        emit_metadata: true,
        mode,
        relaxed: false,
        release: false,
        sources,
        source_paths: None,
        module: stc_codegen::ModuleBuildOptions::default(),
        project: stc_codegen::ProjectOptions::default(),
        test_build: false,
    }
}

pub fn generate(source: &str, mode: stc_codegen::Mode) -> stc_codegen::Output {
    let hir = clean_hir(source);
    let sources = vec!["<test>".to_string()];
    stc_codegen::generate(&hir, &options("test", mode, &sources))
        .unwrap_or_else(|errors| panic!("codegen failed: {errors:#?}"))
}

pub fn clang() -> Option<&'static str> {
    static CLANG: OnceLock<Option<&'static str>> = OnceLock::new();
    CLANG
        .get_or_init(|| {
            let candidate = std::env::var("STC_CLANG").unwrap_or_else(|_| "clang".into());
            let command = if cfg!(windows) && candidate == "clang" {
                "clang".to_string()
            } else {
                candidate
            };
            let status = Command::new(&command).arg("--version").output();
            match status {
                Ok(output) if output.status.success() => Some(Box::leak(command.into_boxed_str())),
                _ => {
                    if std::env::var_os("STC_REQUIRE_CLANG").is_some() {
                        panic!("clang is required (STC_REQUIRE_CLANG=1) but was not found");
                    }
                    eprintln!("note: clang not found; skipping IR verification");
                    None
                }
            }
        })
        .as_ref()
        .copied()
}

pub fn verify_with_clang(name: &str, module: &str) {
    let Some(clang) = clang() else {
        return;
    };
    let directory = std::env::temp_dir().join(format!("stc-codegen-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("create temp directory");
    let ir = directory.join(format!("{name}.ll"));
    let object = directory.join(format!("{name}.o"));
    std::fs::write(&ir, module).expect("write IR");
    let output = Command::new(clang)
        .args(["-x", "ir"])
        .arg(&ir)
        .args(["-c", "-o"])
        .arg(&object)
        .output()
        .expect("run clang");
    assert!(
        output.status.success(),
        "clang rejected {name}:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_file(&ir);
    let _ = std::fs::remove_file(&object);
}

pub fn snapshot_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.ll"))
}
