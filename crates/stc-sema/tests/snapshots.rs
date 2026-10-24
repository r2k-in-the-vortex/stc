mod common;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use common::analyze_sources;
use stc_hir::dump_hir;

struct Case {
    name: &'static str,
    sources: Vec<&'static str>,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "basic",
            sources: vec![
                "PROGRAM P
VAR
  x : DINT;
END_VAR
  x := 1 + 2;
END_PROGRAM
",
            ],
        },
        Case {
            name: "functions",
            sources: vec![
                "FUNCTION Add : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  Add := a + b;
END_FUNCTION
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := Add(2, 3);
END_PROGRAM
",
            ],
        },
        Case {
            name: "oop",
            sources: vec![
                "INTERFACE IShape
METHOD Area : DINT
END_METHOD
END_INTERFACE
CLASS Rect IMPLEMENTS IShape
VAR PUBLIC
  w : DINT;
END_VAR
METHOD PUBLIC Area : DINT
  Area := w * w;
END_METHOD
END_CLASS
FUNCTION_BLOCK Use
VAR
  r : Rect;
  s : IShape;
  x : DINT;
END_VAR
  s := r;
  x := s.Area();
END_FUNCTION_BLOCK
",
            ],
        },
        Case {
            name: "types",
            sources: vec![
                "TYPE
  Color : (Red, Green := 5, Blue);
  Small : INT(0..10);
  Point : STRUCT
    x : DINT;
    y : DINT;
  END_STRUCT;
END_TYPE
PROGRAM P
VAR
  c : Color := Green;
  s : Small := 5;
  p : Point := (x := 1, y := 2);
  q : ARRAY[1..2] OF DINT := [10, 20];
END_VAR
  p.x := p.y;
END_PROGRAM
",
            ],
        },
        Case {
            name: "diagnostics",
            sources: vec![
                "PROGRAM P
VAR
  flag : BOOL;
  n : DINT;
  a : ARRAY[1..3] OF DINT;
END_VAR
  flag := n;
  n := a[0];
  n := missing;
END_PROGRAM
",
            ],
        },
        Case {
            name: "namespace",
            sources: vec![
                "NAMESPACE ProjectDep
FUNCTION F : DINT
VAR_INPUT a : DINT; END_VAR
  F := a + 1;
END_FUNCTION
END_NAMESPACE
",
                "PROGRAM P
USING ProjectDep;
VAR
  x : DINT;
END_VAR
  x := F(1);
END_PROGRAM
",
            ],
        },
    ]
}

fn dump(case: &Case) -> String {
    let (analysis, sink) = analyze_sources(&case.sources);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# {} ({} file(s), {} diagnostics)",
        case.name,
        case.sources.len(),
        sink.len()
    );
    out.push_str("## diagnostics\n");
    for diagnostic in sink.iter() {
        let _ = writeln!(
            out,
            "{} {} file{}:{}..{} {}",
            diagnostic.severity.as_str(),
            diagnostic.code,
            diagnostic.primary.file.raw(),
            diagnostic.primary.start,
            diagnostic.primary.end,
            diagnostic.message
        );
        for label in &diagnostic.labels {
            let _ = writeln!(
                out,
                "  label file{}:{}..{} {}",
                label.span.file.raw(),
                label.span.start,
                label.span.end,
                label.message
            );
        }
        for note in &diagnostic.notes {
            let _ = writeln!(out, "  note {note}");
        }
        for suggestion in &diagnostic.suggestions {
            let _ = writeln!(
                out,
                "  help file{}:{}..{} {} => {}",
                suggestion.span.file.raw(),
                suggestion.span.start,
                suggestion.span.end,
                suggestion.message,
                suggestion.replacement
            );
        }
    }
    out.push_str("## hir\n");
    out.push_str(&dump_hir(&analysis.hir));
    out
}

fn snapshot_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

#[test]
fn semantic_snapshots_match() {
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
            "{} snapshot failure(s); regenerate with STC_UPDATE_SNAPSHOTS=1 cargo test -p stc-sema --test snapshots\n\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}
