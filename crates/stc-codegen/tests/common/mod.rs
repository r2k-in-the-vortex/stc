#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use stc_diag::DiagnosticSink;
use stc_hir::HirCrate;
use stc_lexer::tokenize_bytes;
use stc_parser::{ParseOptions, ParsedFile, parse};
use stc_sema::{Analysis, PackageSession, ProjectSession, analyze, analyze_project};
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
        source_map: None,
        module: stc_codegen::ModuleBuildOptions::default(),
        project: stc_codegen::ProjectOptions::default(),
        test_build: false,
        coverage: None,
    }
}

pub fn generate(source: &str, mode: stc_codegen::Mode) -> stc_codegen::Output {
    let hir = clean_hir(source);
    let sources = vec!["<test>".to_string()];
    stc_codegen::generate(&hir, &options("test", mode, &sources))
        .unwrap_or_else(|errors| panic!("codegen failed: {errors:#?}"))
}

/// Parses `sources` in order; source `index` gets `FileId(index)`.
pub fn parse_sources(sources: &[&str]) -> Vec<ParsedFile> {
    let mut sink = DiagnosticSink::unlimited();
    let mut files = Vec::with_capacity(sources.len());
    for (index, source) in sources.iter().enumerate() {
        let file = FileId::new(u32::try_from(index).expect("file limit"));
        let lexed = tokenize_bytes(file, source.as_bytes(), &mut sink);
        let parsed = parse(
            file,
            lexed.text.as_str(),
            &lexed.stream,
            ParseOptions::default(),
            &mut sink,
        );
        files.push(parsed);
    }
    assert!(
        !sink.has_errors(),
        "parse errors for test-build sources: {:?}",
        sink.diagnostics()
    );
    files
}

/// Analyzes one root package as a test build.
///
/// `test_files` indexes entries of `files` that are `[test].sources`; every
/// definition declared in such a file is marked test-only by the front end.
pub fn analyze_test_build(files: &[ParsedFile], test_files: &[usize]) -> Analysis {
    let mut sink = DiagnosticSink::unlimited();
    let test_files: Vec<ParsedFile> = test_files
        .iter()
        .map(|index| {
            files
                .get(*index)
                .unwrap_or_else(|| panic!("test file index {index} out of range"))
                .clone()
        })
        .collect();
    let session = ProjectSession {
        packages: vec![PackageSession::new("_root", "_root", files, true)],
        configuration: None,
        stdlib: true,
        test_build: true,
        test_files: &test_files,
    };
    let analysis = analyze_project(&session, &mut sink);
    analysis
        .hir
        .check_invariants()
        .unwrap_or_else(|message| panic!("HIR invariant: {message}"));
    let errors: Vec<_> = sink
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .collect();
    assert!(
        errors.is_empty(),
        "semantic errors for test-build sources: {errors:#?}"
    );
    analysis
}

/// Options for a test build, optionally enabling statement coverage for the
/// listed source files.
pub fn test_build_options<'a>(
    name: &'a str,
    sources: &'a [String],
    coverage: Option<&[FileId]>,
) -> stc_codegen::Options<'a> {
    stc_codegen::Options {
        artifact_name: name,
        target: Some("x86_64-unknown-linux-gnu"),
        debug: false,
        emit_metadata: true,
        mode: stc_codegen::Mode::Executable,
        relaxed: false,
        release: false,
        sources,
        source_paths: Some(sources),
        source_map: None,
        module: stc_codegen::ModuleBuildOptions::default(),
        project: stc_codegen::ProjectOptions::default(),
        test_build: true,
        coverage: coverage.map(|files| stc_codegen::CoverageOptions {
            files: files.iter().copied().collect(),
        }),
    }
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
