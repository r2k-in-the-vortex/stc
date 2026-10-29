//! Minimal JSON-RPC 2.0 message parsing and response construction.
//!
//! Only the shapes the language server needs are modeled. Unknown fields and
//! client responses are tolerated; malformed messages become a
//! [`ProtocolError`] carrying the JSON-RPC error code to answer with.

use serde_json::{Value, json};

/// JSON-RPC parse error.
pub const PARSE_ERROR: i64 = -32700;
/// JSON-RPC invalid request.
pub const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC method not found.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC invalid params.
pub const INVALID_PARAMS: i64 = -32602;
/// JSON-RPC internal error.
pub const INTERNAL_ERROR: i64 = -32603;
/// LSP server not initialized.
pub const SERVER_NOT_INITIALIZED: i64 = -32002;

/// A parsed incoming JSON-RPC message.
#[derive(Clone, Debug)]
pub enum Incoming {
    /// A request that must be answered.
    Request {
        /// Request identifier, echoed in the response.
        id: Value,
        /// Method name.
        method: String,
        /// Parameters, or `null`.
        params: Value,
    },
    /// A notification that must not be answered.
    Notification {
        /// Method name.
        method: String,
        /// Parameters, or `null`.
        params: Value,
    },
    /// A response to a server-initiated request; the server does not issue
    /// requests and ignores it.
    Response,
}

/// A message that could not be parsed.
#[derive(Clone, Debug)]
pub struct ProtocolError {
    /// JSON-RPC error code to answer with.
    pub code: i64,
    /// Human-readable explanation for the log.
    pub message: String,
    /// Request identifier, when one could be recovered.
    pub id: Option<Value>,
}

impl ProtocolError {
    fn parse(message: impl Into<String>) -> Self {
        Self {
            code: PARSE_ERROR,
            message: message.into(),
            id: None,
        }
    }

    fn invalid(message: impl Into<String>, id: Option<Value>) -> Self {
        Self {
            code: INVALID_REQUEST,
            message: message.into(),
            id,
        }
    }
}

/// Parses one JSON-RPC message body.
///
/// # Errors
///
/// Returns a [`ProtocolError`] when the body is not valid JSON or not a
/// JSON-RPC 2.0 request/notification/response.
pub fn parse(body: &[u8]) -> Result<Incoming, ProtocolError> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| ProtocolError::parse(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| ProtocolError::invalid("message is not a JSON object", None))?;
    let identifier = object
        .get("id")
        .filter(|id| id.is_string() || id.is_number() || id.is_null())
        .cloned();
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(ProtocolError::invalid(
            "missing or unsupported `jsonrpc` version",
            identifier,
        ));
    }
    if let Some(method) = object.get("method") {
        let Some(method) = method.as_str() else {
            return Err(ProtocolError::invalid(
                "`method` is not a string",
                identifier,
            ));
        };
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        return Ok(match identifier {
            Some(id) => Incoming::Request {
                id,
                method: method.to_owned(),
                params,
            },
            None => Incoming::Notification {
                method: method.to_owned(),
                params,
            },
        });
    }
    if identifier.is_some() && (object.contains_key("result") || object.contains_key("error")) {
        return Ok(Incoming::Response);
    }
    Err(ProtocolError::invalid(
        "message is neither a request, a notification, nor a response",
        identifier,
    ))
}

/// Builds a successful response.
#[must_use]
pub fn response(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// Builds an error response.
#[must_use]
pub fn error_response(id: Option<&Value>, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.cloned().unwrap_or(Value::Null),
        "error": { "code": code, "message": message.into() },
    })
}

/// Builds a notification.
#[must_use]
pub fn notification(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_requests_and_notifications() {
        let request = parse(br#"{"jsonrpc":"2.0","id":7,"method":"shutdown","params":null}"#)
            .expect("request");
        match request {
            Incoming::Request { id, method, params } => {
                assert_eq!(id, json!(7));
                assert_eq!(method, "shutdown");
                assert_eq!(params, Value::Null);
            }
            other => panic!("unexpected {other:?}"),
        }

        let notification = parse(br#"{"jsonrpc":"2.0","method":"exit"}"#).expect("notification");
        assert!(matches!(
            notification,
            Incoming::Notification { ref method, .. } if method == "exit"
        ));
    }

    #[test]
    fn parses_responses_and_ignores_unknown_fields() {
        let response =
            parse(br#"{"jsonrpc":"2.0","id":1,"result":null,"extra":true}"#).expect("response");
        assert!(matches!(response, Incoming::Response));
    }

    #[test]
    fn rejects_malformed_bodies_without_panicking() {
        let error = parse(b"not json").expect_err("parse error");
        assert_eq!(error.code, PARSE_ERROR);
        assert!(error.id.is_none());

        let error = parse(br#"{"jsonrpc":"2.0","id":"a"}"#).expect_err("invalid request");
        assert_eq!(error.code, INVALID_REQUEST);
        assert_eq!(error.id, Some(json!("a")));

        let error = parse(br#"{"id":1,"method":"shutdown"}"#).expect_err("no version");
        assert_eq!(error.code, INVALID_REQUEST);
        assert_eq!(error.id, Some(json!(1)));
    }
}
