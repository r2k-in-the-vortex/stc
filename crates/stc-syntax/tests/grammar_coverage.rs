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
