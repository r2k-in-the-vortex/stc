//! Pragma payload parsing (Tokenization §8, AST §4.0).
//!
//! The lexer leaves a pragma as one opaque token. This module extracts the
//! body, parses the optional strict JSON payload with duplicate-key and
//! depth/size checks, and validates the known `stc:` names. Unknown keys in a
//! known payload are preserved and warned about.

use std::collections::BTreeSet;
use std::fmt;

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use stc_diag::DiagnosticSink;
use stc_lexer::codes;
use stc_span::Span;

use crate::codes as syntax_codes;

/// Maximum accepted pragma body length in bytes (STC-LEX-056).
pub const MAX_PRAGMA_BODY: usize = 64 * 1024;

/// Maximum accepted JSON nesting depth (STC-LEX-056).
pub const MAX_JSON_DEPTH: usize = 32;

const DUPLICATE_MARKER: &str = "stc duplicate key";
const DEPTH_MARKER: &str = "stc depth limit";

/// A parsed `stc:` pragma annotation.
#[derive(Clone, Debug, PartialEq)]
pub struct PragmaAnnotation {
    /// Name after `stc:`, normalized to lowercase ASCII.
    pub name: String,
    /// Strict JSON payload, when one was present.
    pub payload: Option<Value>,
    /// Span of the whole pragma token, including braces.
    pub span: Span,
}

impl PragmaAnnotation {
    /// Returns the payload as an object, if it is one.
    #[must_use]
    pub fn object(&self) -> Option<&Map<String, Value>> {
        self.payload.as_ref().and_then(Value::as_object)
    }
}

/// Parses one pragma token. `text` is the full token text including `{` and
/// `}`; `terminated` comes from the lexer.
///
/// Returns `None` for an unterminated pragma, a non-`stc:` pragma, or an
/// unknown `stc:` name. Errors and warnings are pushed to `diagnostics`.
pub fn parse_pragma(
    text: &str,
    span: Span,
    terminated: bool,
    diagnostics: &mut DiagnosticSink,
) -> Option<PragmaAnnotation> {
    if !terminated {
        // The lexer already reported STC-E-LEX-007.
        return None;
    }
    let body = text
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .unwrap_or(text);
    if body.len() > MAX_PRAGMA_BODY {
        diagnostics.error(
            codes::PRAGMA_PAYLOAD_LIMIT,
            format!("pragma body exceeds {MAX_PRAGMA_BODY} bytes"),
            span,
        );
        return None;
    }
    let trimmed = body.trim_start();
    let Some(rest) = strip_stc_prefix(trimmed) else {
        diagnostics.warning(
            codes::IGNORED_PRAGMA,
            "only `stc:` pragmas are interpreted; this pragma is ignored",
            span,
        );
        return None;
    };
    let rest = rest.trim_start();
    let name_len = rest
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        .count();
    let starts_valid = rest
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_');
    if name_len == 0 || !starts_valid {
        diagnostics.error(codes::UNKNOWN_PRAGMA, "missing `stc:` pragma name", span);
        return None;
    }
    let name = rest[..name_len].to_ascii_lowercase();
    if !matches!(
        name.as_str(),
        "test" | "export" | "no_mangle" | "inline" | "deprecated" | "io"
    ) {
        diagnostics.error(codes::UNKNOWN_PRAGMA, unknown_name(&name), span);
        return None;
    }
    let payload_text = rest[name_len..].trim();
    let payload = if payload_text.is_empty() {
        None
    } else {
        Some(parse_json(payload_text, span, diagnostics)?)
    };
    if let Err(message) = validate_payload(&name, payload.as_ref()) {
        diagnostics.error(codes::PRAGMA_PAYLOAD, message, span);
        // Keep the annotation: the payload is preserved for tooling even when
        // its shape is wrong.
    }
    if let Some(Value::Object(object)) = &payload {
        for key in object.keys() {
            if !known_key(&name, key) {
                diagnostics.warning(
                    codes::UNKNOWN_PRAGMA_KEY,
                    format!("unknown key `{key}` in `stc:{name}` pragma; it is preserved"),
                    span,
                );
            }
        }
    }
    Some(PragmaAnnotation {
        name,
        payload,
        span,
    })
}

fn strip_stc_prefix(text: &str) -> Option<&str> {
    let (prefix, rest) = text.split_at_checked(4)?;
    prefix.eq_ignore_ascii_case("stc:").then_some(rest)
}

fn parse_json(text: &str, span: Span, diagnostics: &mut DiagnosticSink) -> Option<Value> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let parsed = JsonSeed { depth: 0 }.deserialize(&mut deserializer);
    let result = parsed.and_then(|value| {
        deserializer.end()?;
        Ok(value)
    });
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            let message = error.to_string();
            if message.contains(DUPLICATE_MARKER) {
                diagnostics.error(
                    codes::DUPLICATE_JSON_KEY,
                    format!(
                        "duplicate key in pragma JSON payload: {}",
                        message.split(DUPLICATE_MARKER).nth(1).unwrap_or("").trim()
                    ),
                    span,
                );
            } else if message.contains(DEPTH_MARKER) {
                diagnostics.error(
                    codes::PRAGMA_PAYLOAD_LIMIT,
                    format!("pragma JSON payload nests deeper than {MAX_JSON_DEPTH} levels"),
                    span,
                );
            } else {
                diagnostics.error(
                    codes::INVALID_JSON_PRAGMA,
                    format!("invalid pragma JSON payload: {message}"),
                    span,
                );
            }
            None
        }
    }
}

fn validate_payload(name: &str, payload: Option<&Value>) -> Result<(), String> {
    match name {
        "export" | "no_mangle" | "inline" => {
            if payload.is_some() {
                return Err(format!("`stc:{name}` does not take a payload"));
            }
        }
        "test" => {
            if let Some(value) = payload {
                let object = value
                    .as_object()
                    .ok_or_else(|| "`stc:test` payload must be an object".to_string())?;
                if let Some(cycles) = object.get("cycles") {
                    let Some(cycles) = cycles.as_u64() else {
                        return Err("`cycles` must be a non-negative integer".into());
                    };
                    if cycles == 0 {
                        return Err("`cycles` must be greater than zero".into());
                    }
                }
                if let Some(timeout) = object.get("timeout")
                    && !timeout.is_string()
                {
                    return Err("`timeout` must be a duration string".into());
                }
                if let Some(trap) = object.get("expect-trap")
                    && !trap.is_string()
                {
                    return Err("`expect-trap` must be a string".into());
                }
            }
        }
        "deprecated" => {
            if let Some(value) = payload {
                let object = value
                    .as_object()
                    .ok_or_else(|| "`stc:deprecated` payload must be an object".to_string())?;
                if let Some(message) = object.get("message")
                    && !message.is_string()
                {
                    return Err("`message` must be a string".into());
                }
            }
        }
        "io" => {
            let value = payload.ok_or_else(|| "`stc:io` requires an object payload".to_string())?;
            let object = value
                .as_object()
                .ok_or_else(|| "`stc:io` payload must be an object".to_string())?;
            if let Some(device) = object.get("device")
                && !device.is_string()
            {
                return Err("`device` must be a string".into());
            }
            if let Some(channel) = object.get("channel")
                && channel.as_u64().is_none()
            {
                return Err("`channel` must be a non-negative integer".into());
            }
            if let Some(direction) = object.get("direction") {
                match direction.as_str() {
                    Some("input" | "output" | "inout") => {}
                    _ => {
                        return Err(
                            "`direction` must be \"input\", \"output\", or \"inout\"".into()
                        );
                    }
                }
            }
        }
        _ => {
            return Err(format!("unknown `stc:` pragma name `{name}`"));
        }
    }
    Ok(())
}

fn known_key(name: &str, key: &str) -> bool {
    match name {
        "test" => matches!(key, "cycles" | "timeout" | "expect-trap"),
        "deprecated" => key == "message",
        "io" => matches!(key, "device" | "channel" | "direction"),
        _ => false,
    }
}

/// Report an unknown `stc:` name. Kept separate so `parse_pragma` can return
/// `None` without treating the annotation as usable.
pub fn unknown_name(name: &str) -> String {
    format!("unknown `stc:` pragma name `{name}`")
}

struct JsonSeed {
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for JsonSeed {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(JsonVisitor { depth: self.depth })
    }
}

struct JsonVisitor {
    depth: usize,
}

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a strict JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| de::Error::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        if self.depth >= MAX_JSON_DEPTH {
            return Err(de::Error::custom(DEPTH_MARKER));
        }
        let mut values = Vec::new();
        while let Some(value) = access.next_element_seed(JsonSeed {
            depth: self.depth + 1,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        if self.depth >= MAX_JSON_DEPTH {
            return Err(de::Error::custom(DEPTH_MARKER));
        }
        let mut object = Map::new();
        let mut keys = BTreeSet::new();
        while let Some(key) = access.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(de::Error::custom(format!("{DUPLICATE_MARKER}: `{key}`")));
            }
            let value = access.next_value_seed(JsonSeed {
                depth: self.depth + 1,
            })?;
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
}

/// Reports a duplicate `stc:io` pragma on one declaration (STC-E-IO-003).
pub fn duplicate_io(diagnostics: &mut DiagnosticSink, span: Span, first: Span) {
    diagnostics.push(
        stc_diag::Diagnostic::error(
            syntax_codes::DUPLICATE_IO_PRAGMA,
            "duplicate `stc:io` pragma on one declaration",
            span,
        )
        .with_label(first, "the first `stc:io` pragma is here"),
    );
}
