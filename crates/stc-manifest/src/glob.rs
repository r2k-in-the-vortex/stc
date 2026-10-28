//! Source glob compilation and matching.
//!
//! Supported syntax is `*`, `**`, `?`, and character classes. Matching is
//! case-insensitive on Windows and case-sensitive on Linux (CLI and Manifest
//! §2.3). Patterns are always relative and always use `/`.

/// Whether path comparison ignores ASCII case on this host.
pub(crate) use crate::host::CASE_INSENSITIVE_PATHS as CASE_INSENSITIVE;

/// A compiled source glob.
#[derive(Clone, Debug)]
pub(crate) struct Pattern {
    segments: Vec<Segment>,
}

/// One `/`-separated pattern segment.
#[derive(Clone, Debug)]
enum Segment {
    /// `**`: zero or more whole segments.
    Recursive,
    /// A single segment with wildcards.
    Wild(Vec<Token>),
}

/// One wildcard-segment token.
#[derive(Clone, Debug)]
enum Token {
    /// A literal character.
    Literal(char),
    /// `?`.
    AnyOne,
    /// `*`.
    AnyMany,
    /// `[...]`.
    Class(CharClass),
}

/// A parsed character class.
#[derive(Clone, Debug)]
struct CharClass {
    negated: bool,
    singles: Vec<char>,
    ranges: Vec<(char, char)>,
}

impl CharClass {
    fn matches(&self, ch: char, case_insensitive: bool) -> bool {
        let ch = fold(ch, case_insensitive);
        let found = self
            .singles
            .iter()
            .any(|single| fold(*single, case_insensitive) == ch)
            || self.ranges.iter().any(|(start, end)| {
                let start = fold(*start, case_insensitive);
                let end = fold(*end, case_insensitive);
                start <= ch && ch <= end
            });
        found != self.negated
    }
}

fn fold(ch: char, case_insensitive: bool) -> char {
    if case_insensitive {
        ch.to_ascii_lowercase()
    } else {
        ch
    }
}

impl Pattern {
    /// Compiles one glob.
    ///
    /// # Errors
    ///
    /// Returns a human-readable reason for an empty, absolute, backslash, or
    /// syntactically invalid pattern.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        if text.is_empty() {
            return Err("source glob is empty".to_owned());
        }
        if text.contains('\\') {
            return Err("source glob must use `/` separators".to_owned());
        }
        if text.starts_with('/') {
            return Err("source glob must be relative to the manifest".to_owned());
        }
        if is_drive_prefixed(text) {
            return Err("source glob must be relative to the manifest".to_owned());
        }
        let mut segments = Vec::new();
        for raw in text.split('/') {
            match raw {
                "" => return Err("source glob has an empty path segment".to_owned()),
                "." => continue,
                ".." => {
                    return Err("source glob must stay inside the project".to_owned());
                }
                "**" => segments.push(Segment::Recursive),
                other => segments.push(Segment::Wild(parse_tokens(other)?)),
            }
        }
        if segments.is_empty() {
            return Err("source glob must name a file pattern".to_owned());
        }
        Ok(Self { segments })
    }

    /// Returns whether the normalized relative path matches.
    pub(crate) fn matches(&self, path: &str) -> bool {
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        let segments = &self.segments;
        let mut table = vec![vec![false; parts.len() + 1]; segments.len() + 1];
        table[segments.len()][parts.len()] = true;
        for index in (0..segments.len()).rev() {
            for position in (0..=parts.len()).rev() {
                table[index][position] = match &segments[index] {
                    Segment::Recursive => {
                        table[index + 1][position]
                            || (position < parts.len() && table[index][position + 1])
                    }
                    Segment::Wild(tokens) => {
                        position < parts.len()
                            && match_tokens(tokens, parts[position])
                            && table[index + 1][position + 1]
                    }
                };
            }
        }
        table[0][0]
    }
}

fn is_drive_prefixed(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn parse_tokens(segment: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = segment.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '*' => {
                if !matches!(tokens.last(), Some(Token::AnyMany)) {
                    tokens.push(Token::AnyMany);
                }
                index += 1;
            }
            '?' => {
                tokens.push(Token::AnyOne);
                index += 1;
            }
            '[' => {
                let (class, next) = parse_class(&chars, index)?;
                tokens.push(Token::Class(class));
                index = next;
            }
            other => {
                tokens.push(Token::Literal(other));
                index += 1;
            }
        }
    }
    Ok(tokens)
}

fn parse_class(chars: &[char], open: usize) -> Result<(CharClass, usize), String> {
    let mut index = open + 1;
    let negated = matches!(chars.get(index), Some('!' | '^'));
    if negated {
        index += 1;
    }
    let mut content = Vec::new();
    while index < chars.len() && chars[index] != ']' {
        content.push(chars[index]);
        index += 1;
    }
    if index == chars.len() {
        return Err("source glob has an unterminated character class".to_owned());
    }
    if content.is_empty() {
        return Err("source glob has an empty character class".to_owned());
    }
    // Skip the closing `]`.
    index += 1;
    let mut singles = Vec::new();
    let mut ranges = Vec::new();
    let mut cursor = 0;
    while cursor < content.len() {
        if cursor + 2 < content.len() && content[cursor + 1] == '-' {
            let start = content[cursor];
            let end = content[cursor + 2];
            if start > end {
                return Err(format!(
                    "source glob character range `{start}-{end}` is reversed"
                ));
            }
            ranges.push((start, end));
            cursor += 3;
        } else {
            singles.push(content[cursor]);
            cursor += 1;
        }
    }
    Ok((
        CharClass {
            negated,
            singles,
            ranges,
        },
        index,
    ))
}

fn match_tokens(tokens: &[Token], text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let mut table = vec![vec![false; chars.len() + 1]; tokens.len() + 1];
    table[tokens.len()][chars.len()] = true;
    for index in (0..tokens.len()).rev() {
        for position in (0..=chars.len()).rev() {
            table[index][position] = match &tokens[index] {
                Token::AnyMany => {
                    table[index + 1][position]
                        || (position < chars.len() && table[index][position + 1])
                }
                Token::AnyOne => position < chars.len() && table[index + 1][position + 1],
                Token::Literal(expected) => {
                    position < chars.len()
                        && fold(chars[position], CASE_INSENSITIVE)
                            == fold(*expected, CASE_INSENSITIVE)
                        && table[index + 1][position + 1]
                }
                Token::Class(class) => {
                    position < chars.len()
                        && class.matches(chars[position], CASE_INSENSITIVE)
                        && table[index + 1][position + 1]
                }
            };
        }
    }
    table[0][0]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, path: &str) -> bool {
        Pattern::parse(pattern).unwrap().matches(path)
    }

    #[test]
    fn star_stays_within_a_segment() {
        assert!(matches("src/*.st", "src/main.st"));
        assert!(!matches("src/*.st", "src/nested/main.st"));
    }

    #[test]
    fn recursive_glob_matches_zero_or_more_segments() {
        assert!(matches("src/**/*.st", "src/main.st"));
        assert!(matches("src/**/*.st", "src/a/b/main.st"));
        assert!(matches("**/*.st", "main.st"));
        assert!(matches("**", "a/b/c"));
    }

    #[test]
    fn question_mark_and_classes() {
        assert!(matches("src/?ain.st", "src/main.st"));
        assert!(matches("src/[mn]ain.st", "src/main.st"));
        assert!(matches("src/[a-z]ain.st", "src/main.st"));
        assert!(matches("src/[!x]ain.st", "src/main.st"));
        assert!(!matches("src/[!m]ain.st", "src/main.st"));
    }

    #[test]
    fn rejects_escaping_and_absolute_patterns() {
        assert!(Pattern::parse("../src/*.st").is_err());
        assert!(Pattern::parse("/src/*.st").is_err());
        assert!(Pattern::parse("C:/src/*.st").is_err());
        assert!(Pattern::parse("src\\*.st").is_err());
        assert!(Pattern::parse("src//*.st").is_err());
        assert!(Pattern::parse("src/[!.st").is_err());
        assert!(Pattern::parse("src/[]").is_err());
    }

    #[test]
    fn leading_current_directory_is_ignored() {
        assert!(matches("./src/*.st", "src/main.st"));
    }
}
