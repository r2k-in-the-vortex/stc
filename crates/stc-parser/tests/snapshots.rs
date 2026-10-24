mod common;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use common::parse_source;
use stc_syntax::{dump_cst, dump_typed_ast};

struct Case {
    name: &'static str,
    source: String,
}

fn case(name: &'static str, source: &str) -> Case {
    Case {
        name,
        source: source.to_string(),
    }
}

fn cases() -> Vec<Case> {
    let function = include_str!("corpus/function.st");
    let statements = include_str!("corpus/statements.st");
    vec![
        case("empty", include_str!("corpus/empty.st")),
        case("function", function),
        case("types", include_str!("corpus/types.st")),
        case("oop", include_str!("corpus/oop.st")),
        case("statements", statements),
        case("configuration", include_str!("corpus/configuration.st")),
        case("pragmas", include_str!("corpus/pragmas.st")),
        case("expressions", include_str!("corpus/expressions.st")),
        case("comments", include_str!("corpus/comments.st")),
        case("errors", include_str!("corpus/errors.st")),
        Case {
            name: "bom",
            source: format!("\u{feff}{function}"),
        },
        Case {
            name: "crlf",
            source: statements.replace('\n', "\r\n"),
        },
    ]
}

fn dump(case: &Case) -> String {
    let (file, sink) = parse_source(&case.source);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# {} ({} bytes, {} diagnostics)",
        case.name,
        case.source.len(),
        sink.len()
    );
    out.push_str("## cst\n");
    out.push_str(&dump_cst(file.tree()));
    out.push_str("## ast\n");
    out.push_str(&dump_typed_ast(file.tree()));
    out.push_str("## diagnostics\n");
    for diagnostic in sink.iter() {
        let _ = writeln!(
            out,
            "{} {} {}..{} {}",
            diagnostic.severity.as_str(),
            diagnostic.code,
            diagnostic.primary.start,
            diagnostic.primary.end,
            diagnostic.message
        );
        for label in &diagnostic.labels {
            let _ = writeln!(
                out,
                "  label {}..{} {}",
                label.span.start, label.span.end, label.message
            );
        }
    }
    out
}

fn snapshot_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

#[test]
fn parser_snapshots_match() {
    let update = std::env::var_os("STC_UPDATE_SNAPSHOTS").is_some();
    let directory = snapshot_dir();
    if update {
        std::fs::create_dir_all(&directory).expect("create snapshot directory");
    }
    let mut failures = Vec::new();
    for case in cases() {
        let actual = dump(&case);
        let path = directory.join(format!("{}.snap", case.name));
        if update {
            std::fs::write(&path, &actual).expect("write snapshot");
            continue;
        }
        let Ok(expected) = std::fs::read_to_string(&path) else {
            failures.push(format!(
                "{}: missing snapshot; run with STC_UPDATE_SNAPSHOTS=1",
                case.name
            ));
            continue;
        };
        if expected != actual {
            failures.push(format!(
                "{}: snapshot mismatch\n--- expected ---\n{expected}\n--- actual ---\n{actual}",
                case.name
            ));
        }
    }
    if !failures.is_empty() {
        panic!(
            "{} snapshot failure(s); regenerate with STC_UPDATE_SNAPSHOTS=1 cargo test -p stc-parser --test snapshots\n\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}

#[test]
fn snapshot_cases_keep_the_cst_contract() {
    for case in cases() {
        let (file, _) = parse_source(&case.source);
        file.tree()
            .check_invariants()
            .unwrap_or_else(|message| panic!("{}: {message}", case.name));
        assert_eq!(file.root().text(), case.source);
    }
}
