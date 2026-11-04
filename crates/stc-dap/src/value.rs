//! Rendered value text for DAP `variables` and `evaluate` responses.
//!
//! Values come from the runtime's typed envelope (`{"type":...,"value":...}`)
//! and the optional manifest-assisted `view` node. Nothing is re-read or
//! reconstructed here: an unavailable value is rendered as unavailable.

use stc_got::json::Json;

/// Maximum characters shown for one string value.
const MAX_TEXT: usize = 256;

/// Returns the canonical type key from a typed envelope.
#[must_use]
pub(crate) fn typed_value_type(typed: &Json) -> String {
    typed
        .as_object()
        .and_then(|object| object.get("type"))
        .and_then(Json::as_str)
        .unwrap_or("UNKNOWN")
        .to_owned()
}

/// Renders a typed envelope's value as short display text.
#[must_use]
pub(crate) fn typed_value_text(typed: &Json) -> String {
    typed
        .as_object()
        .and_then(|object| object.get("value"))
        .map_or_else(|| "<unavailable>".into(), value_text)
}

/// Renders one debug value (the `value` member of a typed envelope).
#[must_use]
pub(crate) fn value_text(value: &Json) -> String {
    match value {
        Json::Null => "<unavailable>".into(),
        Json::Bool(true) => "TRUE".into(),
        Json::Bool(false) => "FALSE".into(),
        Json::Number(number) => number.to_string(),
        Json::NumberLiteral(text) => text.clone(),
        Json::String(text) => text.clone(),
        Json::Array(elements) => format!("[{}]", elements.len()),
        Json::Object(object) => {
            if let Some(units) = object.get("units") {
                return units.as_array().map_or_else(|| "\"\"".into(), string_text);
            }
            if let Some(elements) = object.get("elements").and_then(Json::as_array) {
                return format!("[{}]", elements.len());
            }
            if let Some(fields) = object.get("fields").and_then(Json::as_array) {
                return format!("{{{} fields}}", fields.len());
            }
            if object.contains_key("aggregate") {
                return "{}".into();
            }
            if let Some(class) = object.get("class").and_then(Json::as_str) {
                return format!("class {class}");
            }
            if let Some(interface) = object.get("interface").and_then(Json::as_str) {
                return format!("interface {interface}");
            }
            if let Some(present) = object.get("present").and_then(Json::as_bool) {
                return if present {
                    object
                        .get("identity")
                        .and_then(Json::as_str)
                        .map_or_else(|| "REF".into(), str::to_owned)
                } else {
                    "NULL".into()
                };
            }
            if let Some(bytes) = object.get("bytes").and_then(Json::as_str) {
                return format!("{{{} encoded bytes}}", bytes.len());
            }
            if object.get("available").and_then(Json::as_bool) == Some(false) {
                let code = object
                    .get("code")
                    .and_then(Json::as_str)
                    .unwrap_or("unavailable");
                return format!("<unavailable: {code}>");
            }
            truncate(&value.render_compact())
        }
    }
}

/// Renders one view node's value without re-reading the target.
#[must_use]
pub(crate) fn view_node_text(node: &Json) -> String {
    let Some(object) = node.as_object() else {
        return "<unavailable>".into();
    };
    if let Some(value) = object.get("value") {
        return value_text(value);
    }
    let kind = object.get("kind").and_then(Json::as_str).unwrap_or("value");
    let fields = object
        .get("fields")
        .and_then(Json::as_array)
        .map_or(0, <[Json]>::len);
    let elements = object
        .get("elements")
        .and_then(Json::as_array)
        .map_or(0, <[Json]>::len);
    match kind {
        "array" => format!("[{elements}]"),
        "aggregate" | "program" | "function_block" | "class" | "interface" => {
            format!("{{{fields} fields}}")
        }
        _ => "<unavailable>".into(),
    }
}

/// Returns whether a view node has children that can be expanded.
#[must_use]
pub(crate) fn view_node_expandable(node: &Json) -> bool {
    node.as_object().is_some_and(|object| {
        object
            .get("fields")
            .and_then(Json::as_array)
            .is_some_and(|fields| !fields.is_empty())
            || object
                .get("elements")
                .and_then(Json::as_array)
                .is_some_and(|elements| !elements.is_empty())
    })
}

/// One expandable child of a view node.
pub(crate) struct ViewChild<'a> {
    pub(crate) name: String,
    pub(crate) node: &'a Json,
}

/// Lists a view node's children in declared order: fields then elements.
#[must_use]
pub(crate) fn view_children(node: &Json) -> Vec<ViewChild<'_>> {
    let Some(object) = node.as_object() else {
        return Vec::new();
    };
    let mut children = Vec::new();
    if let Some(fields) = object.get("fields").and_then(Json::as_array) {
        for field in fields {
            let Some(field_object) = field.as_object() else {
                continue;
            };
            // The runtime flattens a field's node into the field object
            // (`name`, `path`, then the node keys); older callers can still
            // supply a nested `view`.
            let view = field_object.get("view").unwrap_or(field);
            let name = field_object
                .get("name")
                .and_then(Json::as_str)
                .or_else(|| field_object.get("path").and_then(Json::as_str))
                .unwrap_or("?")
                .to_owned();
            children.push(ViewChild { name, node: view });
        }
    }
    if let Some(elements) = object.get("elements").and_then(Json::as_array) {
        for (index, element) in elements.iter().enumerate() {
            children.push(ViewChild {
                name: format!("[{index}]"),
                node: element,
            });
        }
    }
    children
}

/// Decodes a `units` array into a quoted, escaped string.
fn string_text(units: &[Json]) -> String {
    let mut text = String::new();
    for unit in units.iter().take(MAX_TEXT) {
        let scalar = unit
            .as_number()
            .and_then(|value| u32::try_from(value).ok())
            .and_then(char::from_u32)
            .unwrap_or('\u{fffd}');
        match scalar {
            '"' => text.push_str("\\\""),
            '\\' => text.push_str("\\\\"),
            character if character.is_control() => text.push(' '),
            character => text.push(character),
        }
    }
    if units.len() > MAX_TEXT {
        text.push('…');
    }
    format!("\"{text}\"")
}

fn truncate(text: &str) -> String {
    let mut out: String = text.chars().take(MAX_TEXT).collect();
    if text.chars().count() > MAX_TEXT {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use stc_got::json;

    fn value(text: &str) -> Json {
        json::parse_protocol(text).expect("json")
    }

    #[test]
    fn renders_scalars_and_composites() {
        assert_eq!(
            typed_value_text(&value(r#"{"type":"DINT","value":"42"}"#)),
            "42"
        );
        assert_eq!(
            typed_value_text(&value(r#"{"type":"BOOL","value":true}"#)),
            "TRUE"
        );
        assert_eq!(
            typed_value_text(&value(r#"{"type":"REAL","value":1.5}"#)),
            "1.5"
        );
        assert_eq!(
            typed_value_text(&value(
                r#"{"type":"ARRAY[1..3] OF DINT","value":{"elements":[]}}"#
            )),
            "[0]"
        );
        assert_eq!(
            typed_value_text(&value(
                r#"{"type":"Point","value":{"aggregate":true,"atomic":false,"fields":[]}}"#
            )),
            "{0 fields}"
        );
    }

    #[test]
    fn renders_strings_from_units_and_unavailable_values() {
        assert_eq!(
            value_text(&value(r#"{"length":3,"units":[65,66,34]}"#)),
            "\"AB\\\"\""
        );
        assert_eq!(
            value_text(&value(r#"{"available":false,"code":"STC-W-DEBUG-002"}"#)),
            "<unavailable: STC-W-DEBUG-002>"
        );
        assert_eq!(
            value_text(&value(
                r#"{"present":false,"identity":null,"target_type":"DINT"}"#
            )),
            "NULL"
        );
    }

    #[test]
    fn follows_view_children_without_reads() {
        let node = value(
            r#"{
                "kind": "aggregate",
                "type": "Point",
                "available": true,
                "fields": [
                    {"name": "x", "path": "x", "view": {"kind": "scalar", "type": "DINT", "value": "1"}},
                    {"name": "y", "path": "y", "view": {"kind": "scalar", "type": "BOOL", "value": false}}
                ],
                "elements": []
            }"#,
        );
        assert_eq!(view_node_text(&node), "{2 fields}");
        assert!(view_node_expandable(&node));
        let children = view_children(&node);
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].name, "x");
        assert_eq!(view_node_text(children[0].node), "1");
        assert_eq!(view_node_text(children[1].node), "FALSE");
        assert!(!view_node_expandable(children[0].node));
    }

    #[test]
    fn follows_flattened_view_children() {
        // The runtime serializes each field's node into the field object.
        let node = value(
            r#"{
                "kind": "function_block",
                "type": "T:1",
                "available": true,
                "fields": [
                    {"name": "INCREMENT", "path": "INCREMENT", "kind": "scalar", "type": "DINT", "value": 1},
                    {"name": "TOTAL", "path": "TOTAL", "kind": "scalar", "type": "DINT", "value": 2}
                ],
                "elements": []
            }"#,
        );
        assert_eq!(view_node_text(&node), "{2 fields}");
        assert!(view_node_expandable(&node));
        let children = view_children(&node);
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].name, "INCREMENT");
        assert_eq!(view_node_text(children[0].node), "1");
        assert_eq!(view_node_text(children[1].node), "2");
        assert!(!view_node_expandable(children[0].node));
    }
}
