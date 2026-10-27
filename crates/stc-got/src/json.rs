//! Canonical JSON for the manifest projection (GOT Artifacts §2.2).
//!
//! The writer is deterministic: objects are emitted with sorted keys, two-space
//! indentation, LF line endings, and exactly one trailing LF. The parser is
//! strict: integers only, no duplicate object keys, and standard JSON escapes.
//! [`Json::NumberLiteral`] carries a finite non-integer token for audit and
//! protocol envelopes; artifact projections never use it, and the parser
//! rejects it.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::error::{Error, Result};

/// A JSON value with object keys held in sorted order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Json {
    /// `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// An integer number.
    Number(i128),
    /// A finite non-integer JSON number token.
    ///
    /// This is only for audit and protocol envelopes. The manifest projection
    /// never emits it, and [`parse`] rejects a non-integer token. The caller
    /// supplies a valid JSON number spelling (for example `1.5`).
    NumberLiteral(String),
    /// A string.
    String(String),
    /// An array.
    Array(Vec<Json>),
    /// An object with sorted keys.
    Object(BTreeMap<String, Json>),
}

impl Json {
    /// Builds an object from pairs.
    #[must_use]
    pub fn object(pairs: impl IntoIterator<Item = (String, Json)>) -> Self {
        Self::Object(pairs.into_iter().collect())
    }

    /// Builds a string value.
    #[must_use]
    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }

    /// Builds a number value from an unsigned integer.
    #[must_use]
    pub fn unsigned(value: u64) -> Self {
        Self::Number(i128::from(value))
    }

    /// Builds a number value from a signed integer.
    #[must_use]
    pub fn signed(value: i64) -> Self {
        Self::Number(i128::from(value))
    }

    /// Builds an optional value.
    #[must_use]
    pub fn optional(value: Option<Json>) -> Self {
        value.unwrap_or(Self::Null)
    }

    /// Renders the canonical projection.
    ///
    /// The result ends with exactly one LF.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    /// Renders one line with no insignificant whitespace.
    ///
    /// The control protocol frames one request or response per line.
    #[must_use]
    pub fn render_compact(&self) -> String {
        let mut out = String::new();
        self.write_compact(&mut out);
        out
    }

    fn write_compact(&self, out: &mut String) {
        match self {
            Self::Array(values) => {
                out.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    value.write_compact(out);
                }
                out.push(']');
            }
            Self::Object(values) => {
                out.push('{');
                for (index, (key, value)) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write_string(out, key);
                    out.push(':');
                    value.write_compact(out);
                }
                out.push('}');
            }
            other => {
                let mut rendered = String::new();
                if let Self::Null = other {
                    out.push_str("null");
                    return;
                }
                other.write(&mut rendered, 0);
                out.push_str(rendered.trim());
            }
        }
    }

    /// Returns the object entries when this is an object.
    #[must_use]
    pub fn as_object(&self) -> Option<&BTreeMap<String, Json>> {
        match self {
            Self::Object(values) => Some(values),
            _ => None,
        }
    }

    /// Returns the array elements when this is an array.
    #[must_use]
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Self::Array(values) => Some(values),
            _ => None,
        }
    }

    /// Returns the string when this is a string.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    /// Returns the number when this is a number.
    #[must_use]
    pub fn as_number(&self) -> Option<i128> {
        match self {
            Self::Number(value) => Some(*value),
            _ => None,
        }
    }

    /// Returns the boolean when this is a boolean.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// Returns `true` when this is `null`.
    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    fn write(&self, out: &mut String, indent: usize) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Self::Number(value) => {
                let _ = write!(out, "{value}");
            }
            Self::NumberLiteral(value) => out.push_str(value),
            Self::String(value) => write_string(out, value),
            Self::Array(values) => {
                if values.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push('\n');
                    indent_to(out, indent + 1);
                    value.write(out, indent + 1);
                }
                out.push('\n');
                indent_to(out, indent);
                out.push(']');
            }
            Self::Object(values) => {
                if values.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push('{');
                for (index, (key, value)) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push('\n');
                    indent_to(out, indent + 1);
                    write_string(out, key);
                    out.push_str(": ");
                    value.write(out, indent + 1);
                }
                out.push('\n');
                indent_to(out, indent);
                out.push('}');
            }
        }
    }
}

fn write_string(out: &mut String, value: &str) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04X}", character as u32);
            }
            character => out.push(character),
        }
    }
    out.push('"');
}

fn indent_to(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push_str("  ");
    }
}

/// Parses strict JSON text.
///
/// # Errors
///
/// Rejects malformed syntax, duplicate object keys, non-integer numbers, and
/// nesting deeper than 64 levels.
pub fn parse(text: &str) -> Result<Json> {
    let mut parser = Parser {
        text,
        position: 0,
        depth: 0,
        finite_reals: false,
    };
    let value = parser.value()?;
    parser.skip_whitespace();
    if parser.position != text.len() {
        return Err(parser.error("trailing characters after the JSON value"));
    }
    Ok(value)
}

/// Parses JSON text for the debug and control protocols.
///
/// This is the strict parser plus finite non-integer numbers, which are
/// returned as [`Json::NumberLiteral`]. Duplicate object keys, leading
/// zeroes, and non-finite numeric tokens are still rejected.
///
/// # Errors
///
/// Rejects malformed syntax, duplicate object keys, non-finite numbers, and
/// nesting deeper than 64 levels.
pub fn parse_protocol(text: &str) -> Result<Json> {
    let mut parser = Parser {
        text,
        position: 0,
        depth: 0,
        finite_reals: true,
    };
    let value = parser.value()?;
    parser.skip_whitespace();
    if parser.position != text.len() {
        return Err(parser.error("trailing characters after the JSON value"));
    }
    Ok(value)
}

struct Parser<'a> {
    text: &'a str,
    position: usize,
    depth: usize,
    finite_reals: bool,
}

impl Parser<'_> {
    fn error(&self, message: impl Into<String>) -> Error {
        Error::manifest(format!(
            "JSON error at byte {}: {}",
            self.position,
            message.into()
        ))
    }

    fn peek(&self) -> Option<char> {
        self.text[self.position..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.position += character.len_utf8();
        Some(character)
    }

    fn skip_whitespace(&mut self) {
        while let Some(character) = self.peek() {
            if matches!(character, ' ' | '\t' | '\n' | '\r') {
                self.position += character.len_utf8();
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, expected: char) -> Result<()> {
        match self.bump() {
            Some(character) if character == expected => Ok(()),
            Some(character) => {
                Err(self.error(format!("expected `{expected}`, found `{character}`")))
            }
            None => Err(self.error(format!("expected `{expected}`, found end of input"))),
        }
    }

    fn value(&mut self) -> Result<Json> {
        self.skip_whitespace();
        let Some(character) = self.peek() else {
            return Err(self.error("expected a JSON value"));
        };
        match character {
            '{' => self.object(),
            '[' => self.array(),
            '"' => Ok(Json::String(self.string()?)),
            't' => {
                self.expect_word("true")?;
                Ok(Json::Bool(true))
            }
            'f' => {
                self.expect_word("false")?;
                Ok(Json::Bool(false))
            }
            'n' => {
                self.expect_word("null")?;
                Ok(Json::Null)
            }
            '-' | '0'..='9' => self.number(),
            _ => Err(self.error(format!("unexpected character `{character}`"))),
        }
    }

    fn expect_word(&mut self, word: &str) -> Result<()> {
        if self.text[self.position..].starts_with(word) {
            self.position += word.len();
            Ok(())
        } else {
            Err(self.error(format!("expected `{word}`")))
        }
    }

    fn object(&mut self) -> Result<Json> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(self.error("JSON nesting exceeds 64 levels"));
        }
        self.expect('{')?;
        let mut values = BTreeMap::new();
        self.skip_whitespace();
        if self.peek() == Some('}') {
            self.bump();
            self.depth -= 1;
            return Ok(Json::Object(values));
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(':')?;
            let value = self.value()?;
            if values.insert(key.clone(), value).is_some() {
                return Err(self.error(format!("duplicate object key `{key}`")));
            }
            self.skip_whitespace();
            match self.bump() {
                Some(',') => continue,
                Some('}') => break,
                Some(character) => {
                    return Err(self.error(format!(
                        "expected `,` or `}}` after an object member, found `{character}`"
                    )));
                }
                None => return Err(self.error("unterminated object")),
            }
        }
        self.depth -= 1;
        Ok(Json::Object(values))
    }

    fn array(&mut self) -> Result<Json> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(self.error("JSON nesting exceeds 64 levels"));
        }
        self.expect('[')?;
        let mut values = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(']') {
            self.bump();
            self.depth -= 1;
            return Ok(Json::Array(values));
        }
        loop {
            values.push(self.value()?);
            self.skip_whitespace();
            match self.bump() {
                Some(',') => continue,
                Some(']') => break,
                Some(character) => {
                    return Err(self.error(format!(
                        "expected `,` or `]` after an array element, found `{character}`"
                    )));
                }
                None => return Err(self.error("unterminated array")),
            }
        }
        self.depth -= 1;
        Ok(Json::Array(values))
    }

    fn string(&mut self) -> Result<String> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            let Some(character) = self.bump() else {
                return Err(self.error("unterminated string"));
            };
            match character {
                '"' => return Ok(out),
                '\\' => {
                    let Some(escape) = self.bump() else {
                        return Err(self.error("unterminated escape sequence"));
                    };
                    match escape {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{0008}'),
                        'f' => out.push('\u{000C}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let first = self.hex_quad()?;
                            if (0xD800..=0xDBFF).contains(&first) {
                                if self.bump() != Some('\\') || self.bump() != Some('u') {
                                    return Err(self.error(
                                        "a high surrogate must be followed by a low surrogate",
                                    ));
                                }
                                let second = self.hex_quad()?;
                                if !(0xDC00..=0xDFFF).contains(&second) {
                                    return Err(self.error("invalid low surrogate"));
                                }
                                let scalar = 0x1_0000
                                    + ((u32::from(first) - 0xD800) << 10)
                                    + (u32::from(second) - 0xDC00);
                                let Some(character) = char::from_u32(scalar) else {
                                    return Err(self.error("invalid Unicode scalar value"));
                                };
                                out.push(character);
                            } else if (0xDC00..=0xDFFF).contains(&first) {
                                return Err(self.error("unexpected low surrogate"));
                            } else {
                                let Some(character) = char::from_u32(u32::from(first)) else {
                                    return Err(self.error("invalid Unicode scalar value"));
                                };
                                out.push(character);
                            }
                        }
                        other => {
                            return Err(self.error(format!("invalid escape `\\{other}`")));
                        }
                    }
                }
                character if (character as u32) < 0x20 => {
                    return Err(
                        self.error("raw control characters are not allowed in JSON strings")
                    );
                }
                character => out.push(character),
            }
        }
    }

    fn hex_quad(&mut self) -> Result<u16> {
        let mut value = 0_u16;
        for _ in 0..4 {
            let Some(character) = self.bump() else {
                return Err(self.error("truncated Unicode escape"));
            };
            let digit = match character {
                '0'..='9' => character as u16 - b'0' as u16,
                'a'..='f' => character as u16 - b'a' as u16 + 10,
                'A'..='F' => character as u16 - b'A' as u16 + 10,
                _ => return Err(self.error("invalid Unicode escape digit")),
            };
            value = value * 16 + digit;
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<Json> {
        let start = self.position;
        if self.peek() == Some('-') {
            self.bump();
        }
        let Some(first) = self.peek() else {
            return Err(self.error("truncated number"));
        };
        if !first.is_ascii_digit() {
            return Err(self.error("a number must contain a digit"));
        }
        if first == '0' {
            self.bump();
            if self
                .peek()
                .is_some_and(|character| character.is_ascii_digit())
            {
                return Err(self.error("numbers may not have leading zeroes"));
            }
        } else {
            while self
                .peek()
                .is_some_and(|character| character.is_ascii_digit())
            {
                self.bump();
            }
        }
        let mut fractional = false;
        if self.peek() == Some('.') {
            fractional = true;
            self.bump();
            let mut digits = 0;
            while self
                .peek()
                .is_some_and(|character| character.is_ascii_digit())
            {
                self.bump();
                digits += 1;
            }
            if digits == 0 {
                return Err(self.error("a fraction must contain a digit"));
            }
        }
        if self
            .peek()
            .is_some_and(|character| matches!(character, 'e' | 'E'))
        {
            fractional = true;
            self.bump();
            if self
                .peek()
                .is_some_and(|character| matches!(character, '+' | '-'))
            {
                self.bump();
            }
            let mut digits = 0;
            while self
                .peek()
                .is_some_and(|character| character.is_ascii_digit())
            {
                self.bump();
                digits += 1;
            }
            if digits == 0 {
                return Err(self.error("an exponent must contain a digit"));
            }
        }
        let text = &self.text[start..self.position];
        if fractional {
            if !self.finite_reals {
                return Err(self.error("only integer numbers are permitted"));
            }
            let Ok(value) = text.parse::<f64>() else {
                return Err(self.error("invalid number"));
            };
            if !value.is_finite() {
                return Err(self.error("numbers must be finite"));
            }
            return Ok(Json::NumberLiteral(text.to_owned()));
        }
        text.parse::<i128>()
            .map(Json::Number)
            .map_err(|_| self.error("integer does not fit in 128 bits"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(pairs: &[(&str, Json)]) -> Json {
        Json::object(
            pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone())),
        )
    }

    #[test]
    fn canonical_rendering_is_sorted_and_indented() {
        let value = object(&[
            ("z", Json::Array(vec![])),
            (
                "a",
                Json::Array(vec![Json::Number(1), Json::string("x\ny")]),
            ),
            ("m", Json::Null),
        ]);
        assert_eq!(
            value.render(),
            "{\n  \"a\": [\n    1,\n    \"x\\ny\"\n  ],\n  \"m\": null,\n  \"z\": []\n}\n"
        );
    }

    #[test]
    fn escaping_matches_the_canonical_rules() {
        let value = Json::String("\"\\\n\r\t\u{1}é".into());
        assert_eq!(value.render(), "\"\\\"\\\\\\n\\r\\t\\u0001é\"\n");
    }

    #[test]
    fn parser_round_trips_canonical_output() {
        let value = object(&[
            ("a", Json::Number(-12)),
            ("b", Json::string("héllo\n")),
            ("c", Json::Bool(true)),
            (
                "d",
                Json::Array(vec![Json::Null, Json::Number(0), Json::Array(vec![])]),
            ),
        ]);
        let text = value.render();
        assert_eq!(parse(&text).unwrap(), value);
    }

    #[test]
    fn parser_rejects_non_canonical_or_invalid_input() {
        assert!(parse("{\"a\":1,\"a\":2}").is_err());
        assert!(parse("{\"a\":1.5}").is_err());
        assert!(parse("{\"a\":01}").is_err());
        assert!(parse("{\"a\":1e3}").is_err());
        assert!(parse("{\"a\":\"\\q\"}").is_err());
        assert!(parse("{\"a\":\"\\uD800\"}").is_err());
        assert!(parse("[1,]").is_err());
        assert!(parse("[1] x").is_err());
        assert!(parse("{\"a\":\"raw\ncontrol\"}").is_err());
    }

    #[test]
    fn parser_handles_surrogate_pairs() {
        assert_eq!(
            parse("\"\\uD83D\\uDE00\"").unwrap(),
            Json::String("😀".into())
        );
    }

    #[test]
    fn protocol_parser_accepts_finite_real_numbers() {
        assert_eq!(
            parse_protocol("{\"a\":1.5,\"b\":-2e3,\"c\":0.0}").unwrap(),
            object(&[
                ("a", Json::NumberLiteral("1.5".into())),
                ("b", Json::NumberLiteral("-2e3".into())),
                ("c", Json::NumberLiteral("0.0".into())),
            ])
        );
        assert_eq!(parse_protocol("1").unwrap(), Json::Number(1));
        assert!(parse_protocol("1e999").is_err());
        assert!(parse_protocol("01.5").is_err());
        assert!(parse_protocol("1.").is_err());
        assert!(parse_protocol("1e").is_err());
        assert!(parse("1.5").is_err());
    }
}
