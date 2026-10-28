//! Acceptance: formatting the examples and parser corpus is always idempotent,
//! and malformed corpus files are refused with `STC-E-FMT-001`.

use std::path::{Path, PathBuf};

use stc_diag::DiagnosticSink;
use stc_fmt::{SYNTAX_ERROR, format_bytes};
use stc_span::FileId;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn st_files(directory: &Path) -> Vec<PathBuf> {
    fn visit(directory: &Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(directory)
            .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                // Example projects may contain a local build directory.
                if path.file_name().and_then(|name| name.to_str()) != Some("target") {
                    visit(&path, files);
                }
            } else if path.extension() == Some("st".as_ref()) {
                files.push(path);
            }
        }
    }

    let mut files = Vec::new();
    visit(directory, &mut files);
    files.sort();
    files
}

#[test]
fn examples_and_parser_corpus_format_idempotently() {
    let root = workspace_root();
    let mut files = st_files(&root.join("examples"));
    files.extend(st_files(&root.join("crates/stc-parser/tests/corpus")));
    assert!(files.len() >= 10, "found only {} source files", files.len());

    let mut formatted = 0_usize;
    let mut refused = 0_usize;
    for path in files {
        let bytes = std::fs::read(&path).expect("read source file");
        let mut diagnostics = DiagnosticSink::new();
        let outcome = format_bytes(FileId::new(0), &bytes, &mut diagnostics);
        if diagnostics.has_errors() {
            // The corpus deliberately includes recovery cases; they must be
            // refused rather than rewritten.
            assert!(
                outcome.formatted.is_none(),
                "{} was formatted despite syntax errors",
                path.display()
            );
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == SYNTAX_ERROR),
                "{} did not report {SYNTAX_ERROR}",
                path.display()
            );
            refused += 1;
            continue;
        }
        let once = outcome.formatted.expect("clean file is formatted");
        let mut second = DiagnosticSink::new();
        let twice = format_bytes(FileId::new(0), once.as_bytes(), &mut second);
        assert!(
            !second.has_errors(),
            "{}: formatted output did not parse: {:?}",
            path.display(),
            second.diagnostics()
        );
        assert_eq!(
            once,
            twice.formatted.expect("second pass formats"),
            "{}: formatting is not idempotent",
            path.display()
        );
        formatted += 1;
    }
    assert!(formatted >= 8, "formatted only {formatted} files");
    assert!(refused >= 1, "no malformed corpus file was refused");
}

#[test]
fn the_hello_example_is_already_formatted() {
    let example = workspace_root().join("examples/hello");
    let mut files = st_files(&example.join("src"));
    files.extend(st_files(&example.join("tests")));
    assert!(!files.is_empty(), "the hello example has no sources");
    for path in files {
        let source = std::fs::read_to_string(&path).expect("read example");
        let mut diagnostics = DiagnosticSink::new();
        let outcome = format_bytes(FileId::new(0), source.as_bytes(), &mut diagnostics);
        assert!(
            !diagnostics.has_errors(),
            "{}: {:?}",
            path.display(),
            diagnostics.diagnostics()
        );
        assert_eq!(
            outcome.formatted.expect("formatted"),
            source,
            "{} is not formatted",
            path.display()
        );
    }
}
