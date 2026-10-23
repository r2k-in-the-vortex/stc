use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use stc_diag::DiagnosticSink;
use stc_lexer::{TokenStream, tokenize};
use stc_span::{FileId, Span};

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
    let basic = include_str!("corpus/basic.st");
    vec![
        case("basic", basic),
        case("literals", include_str!("corpus/literals.st")),
        case("strings", include_str!("corpus/strings.st")),
        case("comments", include_str!("corpus/comments.st")),
        case("direct", include_str!("corpus/direct.st")),
        case("pragmas", include_str!("corpus/pragmas.st")),
        case("errors", include_str!("corpus/errors.st")),
        Case {
            name: "bom",
            source: format!("\u{feff}{basic}"),
        },
        Case {
            name: "crlf",
            source: basic.replace('\n', "\r\n"),
        },
        case("empty", ""),
        case("unterminated_comment", "VAR x : INT; (* never closed"),
        case("unterminated_pragma", "VAR x : INT; {stc:test"),
        case("unterminated_string", "VAR x : INT := 'abc"),
    ]
}

fn dump(case: &Case) -> String {
    let mut diagnostics = DiagnosticSink::unlimited();
    let stream = tokenize(FileId::new(0), &case.source, &mut diagnostics);
    stream
        .check_invariants(&case.source)
        .unwrap_or_else(|message| panic!("{}: {message}", case.name));
    assert_eq!(
        stream.reconstruct(&case.source),
        case.source,
        "{}",
        case.name
    );

    let mut out = String::new();
    let _ = writeln!(
        out,
        "# {} ({} bytes, {} tokens, {} trivia, {} diagnostics)",
        case.name,
        case.source.len(),
        stream.tokens.len(),
        stream.trivia.len(),
        diagnostics.len()
    );

    out.push_str("tokens:\n");
    for (index, token) in stream.tokens.iter().enumerate() {
        let _ = write!(
            out,
            "  [{index}] {:?} {} {:?}",
            token.kind,
            span(token.span),
            token.text(&case.source)
        );
        if let Some(literal) = &token.literal {
            let _ = write!(out, " literal={literal:?}");
        }
        out.push('\n');
        let _ = writeln!(
            out,
            "       leading={} trailing={}",
            range(token.leading_trivia),
            range(token.trailing_trivia)
        );
    }

    out.push_str("trivia:\n");
    for (index, trivia) in stream.trivia.iter().enumerate() {
        let _ = writeln!(
            out,
            "  [{index}] {} {} {:?}",
            trivia.kind.name(),
            span(trivia.span),
            trivia.text(&case.source)
        );
    }

    out.push_str("diagnostics:\n");
    for diagnostic in diagnostics.diagnostics() {
        let _ = writeln!(
            out,
            "  {} {} {} {:?}",
            diagnostic.severity.as_str(),
            diagnostic.code,
            span(diagnostic.primary),
            diagnostic.message
        );
    }
    out
}

fn span(span: Span) -> String {
    format!("{}..{}", span.start, span.end)
}

fn range(range: stc_lexer::TriviaRange) -> String {
    format!("{}..{}", range.start, range.end)
}

fn snapshot_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

#[test]
fn lexer_snapshots_match() {
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
            "{} snapshot failure(s); regenerate with STC_UPDATE_SNAPSHOTS=1 cargo test -p stc-lexer --test snapshots\n\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}

#[test]
fn snapshot_streams_keep_the_invariants() {
    for case in cases() {
        let mut diagnostics = DiagnosticSink::unlimited();
        let stream: TokenStream = tokenize(FileId::new(0), &case.source, &mut diagnostics);
        assert_eq!(stream.reconstruct(&case.source), case.source);
        assert!(stream.eof().is_some());
    }
}
