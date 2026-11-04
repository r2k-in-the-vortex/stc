//! I-13: the VS Code TextMate grammar stays in step with the lexer tables.
//!
//! The test reads `editors/vscode/syntaxes/st.tmLanguage.json` and checks that
//! every [`Keyword`] spelling and every [`Punct`] spelling appears as a whole
//! alternative in one of its regex rules.

use std::path::{Path, PathBuf};

use serde_json::Value;
use stc_lexer::{Keyword, Punct};

fn grammar_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("editors")
        .join("vscode")
        .join("syntaxes")
        .join("st.tmLanguage.json")
}

/// Collects every `match`, `begin`, and `end` regex from the grammar.
fn collect_patterns(value: &Value, patterns: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                match key.as_str() {
                    "match" | "begin" | "end" => {
                        if let Some(pattern) = child.as_str() {
                            patterns.push(pattern.to_owned());
                        }
                    }
                    _ => collect_patterns(child, patterns),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_patterns(item, patterns);
            }
        }
        _ => {}
    }
}

/// Collects `(scope, regex)` pairs for rules that name a whole `match`,
/// `begin`, or `end` pattern.
fn collect_scoped_patterns(value: &Value, patterns: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            if let Some(name) = map.get("name").and_then(Value::as_str) {
                for key in ["match", "begin", "end"] {
                    if let Some(pattern) = map.get(key).and_then(Value::as_str) {
                        patterns.push((name.to_owned(), pattern.to_owned()));
                    }
                }
            }
            for child in map.values() {
                collect_scoped_patterns(child, patterns);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_scoped_patterns(item, patterns);
            }
        }
        _ => {}
    }
}

fn is_word_boundary(character: Option<char>) -> bool {
    character.is_none_or(|character| !(character.is_ascii_alphanumeric() || character == '_'))
}

/// Returns `true` when `word` appears as a whole alternative, so `INT` inside
/// `DINT` or `REF` inside `REF_TO` does not count.
fn contains_word(patterns: &[String], word: &str) -> bool {
    patterns.iter().any(|pattern| {
        pattern.match_indices(word).any(|(start, _)| {
            let before = pattern[..start].chars().next_back();
            let after = pattern[start + word.len()..].chars().next();
            is_word_boundary(before) && is_word_boundary(after)
        })
    })
}

/// Escapes a punctuation spelling as the regex alternative the grammar must
/// contain.
fn escape_regex(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() * 2);
    for character in text.chars() {
        if "\\^$.*+?()[]{}|".contains(character) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

#[test]
fn grammar_covers_lexer_tables() {
    let path = grammar_path();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let grammar: Value = serde_json::from_str(&text).expect("grammar JSON");
    assert_eq!(grammar["scopeName"], "source.st");
    let mut patterns = Vec::new();
    collect_patterns(&grammar, &mut patterns);
    assert!(!patterns.is_empty(), "the grammar has no regex rules");

    let missing: Vec<&str> = Keyword::ALL
        .iter()
        .map(|keyword| keyword.as_str())
        .filter(|spelling| !contains_word(&patterns, spelling))
        .collect();
    assert!(
        missing.is_empty(),
        "the TextMate grammar is missing keywords: {missing:?}"
    );

    let missing: Vec<&str> = Punct::ALL
        .iter()
        .map(|punct| punct.as_str())
        .filter(|spelling| {
            let escaped = escape_regex(spelling);
            !patterns.iter().any(|pattern| pattern.contains(&escaped))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "the TextMate grammar is missing punctuation: {missing:?}"
    );
}

#[test]
fn primitive_type_keywords_use_primitive_scope() {
    // Tokenization §3.1: the elementary and generic primitive type names.
    const PRIMITIVES: &[&str] = &[
        "BOOL",
        "SINT",
        "INT",
        "DINT",
        "LINT",
        "USINT",
        "UINT",
        "UDINT",
        "ULINT",
        "BYTE",
        "WORD",
        "DWORD",
        "LWORD",
        "REAL",
        "LREAL",
        "TIME",
        "LTIME",
        "DATE",
        "LDATE",
        "LDATE_AND_TIME",
        "TIME_OF_DAY",
        "TOD",
        "LTOD",
        "LTIME_OF_DAY",
        "DATE_AND_TIME",
        "DT",
        "LDT",
        "STRING",
        "WSTRING",
        "CHAR",
        "WCHAR",
        "ANY",
        "ANY_DERIVED",
        "ANY_ELEMENTARY",
        "ANY_MAGNITUDE",
        "ANY_NUM",
        "ANY_REAL",
        "ANY_INT",
        "ANY_UNSIGNED",
        "ANY_SIGNED",
        "ANY_DURATION",
        "ANY_BIT",
        "ANY_CHARS",
        "ANY_STRING",
        "ANY_CHAR",
        "ANY_DATE",
        "USTRING",
        "UCHAR",
    ];

    let path = grammar_path();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let grammar: Value = serde_json::from_str(&text).expect("grammar JSON");

    let mut scoped = Vec::new();
    collect_scoped_patterns(&grammar, &mut scoped);
    let primitive_patterns: Vec<String> = scoped
        .iter()
        .filter(|(scope, _)| scope == "support.type.primitives.st")
        .map(|(_, pattern)| pattern.clone())
        .collect();
    assert!(
        !primitive_patterns.is_empty(),
        "the TextMate grammar has no support.type.primitives.st rule"
    );

    for primitive in PRIMITIVES {
        assert!(
            Keyword::ALL
                .iter()
                .any(|keyword| keyword.as_str() == *primitive),
            "{primitive} is not a lexer keyword"
        );
        assert!(
            contains_word(&primitive_patterns, primitive),
            "the TextMate grammar is missing the primitive type scope for {primitive}"
        );
    }
}
