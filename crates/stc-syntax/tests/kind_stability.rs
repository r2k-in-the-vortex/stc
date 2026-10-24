use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use stc_syntax::SyntaxKind;

#[test]
fn syntax_kind_table_matches_the_enum() {
    let path = table_path();
    let update = std::env::var_os("STC_UPDATE_KIND_TABLE").is_some();
    if update {
        std::fs::write(&path, render_table()).expect("write syntax-kind table");
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing {}: {error}; regenerate with STC_UPDATE_KIND_TABLE=1 cargo test -p stc-syntax --test kind_stability",
            path.display()
        )
    });
    assert_eq!(
        expected,
        render_table(),
        "SyntaxKind changed; existing discriminants must never be renumbered or reused. \
         Append new kinds and update tests/syntax_kinds.txt."
    );
}

#[test]
fn discriminants_are_unique_and_sorted() {
    let mut seen = BTreeSet::new();
    let mut previous = 0;
    for kind in SyntaxKind::ALL {
        let value = *kind as u16;
        assert!(value > previous, "{} is out of order", kind.as_str());
        previous = value;
        assert!(seen.insert(value), "duplicate discriminant {value}");
    }
}

#[test]
fn node_and_token_kinds_share_no_values_or_names() {
    let mut names = BTreeSet::new();
    for kind in SyntaxKind::ALL {
        assert!(names.insert(kind.as_str()), "duplicate kind name {kind:?}");
    }
    assert!(SyntaxKind::ALL.iter().any(|kind| kind.is_node()));
    assert!(SyntaxKind::ALL.iter().any(|kind| kind.is_token()));
}

fn render_table() -> String {
    let mut out = String::from("# name discriminant\n");
    for kind in SyntaxKind::ALL {
        out.push_str(kind.as_str());
        out.push(' ');
        out.push_str(&(*kind as u16).to_string());
        out.push('\n');
    }
    out
}

fn table_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/syntax_kinds.txt")
}
