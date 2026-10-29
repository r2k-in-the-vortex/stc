//! DAP request parsing, launch/attach arguments, and adapter capabilities.
//!
//! The adapter accepts only `request` messages from the client; responses and
//! events are adapter output. Extra properties are tolerated because clients
//! echo launch-configuration keys (for example `name`, `type`, and `request`)
//! in `launch` arguments, but every field this adapter consumes is typed.

use serde::Deserialize;
use serde_json::{Map, Value, json};

/// One client request.
#[derive(Debug, Deserialize)]
pub struct Request {
    /// Client-assigned sequence number (positive).
    pub seq: i64,
    /// Must be `request`.
    #[serde(rename = "type")]
    pub message_type: String,
    /// DAP command name.
    pub command: String,
    /// Command arguments; `null` and absent both mean no arguments.
    #[serde(default)]
    pub arguments: Value,
}

/// Parses one DAP request body.
///
/// # Errors
///
/// Returns a readable message when the body is not a JSON object, is not a
/// `request`, has a missing or mistyped required field, or carries a
/// non-positive sequence number.
pub fn parse_request(body: &[u8]) -> Result<Request, String> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|error| format!("the DAP message is not valid JSON: {error}"))?;
    let Some(object) = value.as_object() else {
        return Err("the DAP message is not a JSON object".into());
    };
    match object.get("type").and_then(Value::as_str) {
        Some("request") => {}
        Some(other) => {
            return Err(format!(
                "the client sent a `{other}` message; only requests are accepted"
            ));
        }
        None => return Err("the DAP message has no string `type`".into()),
    }
    let request: Request = serde_json::from_value(value)
        .map_err(|error| format!("the DAP request is malformed: {error}"))?;
    if request.seq <= 0 {
        return Err("the DAP request `seq` must be a positive integer".into());
    }
    if request.command.is_empty() {
        return Err("the DAP request `command` must not be empty".into());
    }
    Ok(request)
}

/// Launch and attach arguments (WL-1.1 I-10, I-14, and §5.3).
///
/// Kebab-case is not used on the DAP wire: VS Code launch configurations use
/// camelCase keys. Unknown keys are ignored so the whole launch configuration
/// can be passed through.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LaunchArguments {
    /// Manifest path; defaults to `<workspaceFolder>/stc.toml` in the client
    /// and to `stc.toml` in the adapter when the client omits it.
    pub project: Option<String>,
    /// Output directory override, primarily for `noBuild`.
    pub out_dir: Option<String>,
    /// Skip the build and launch the existing artifacts.
    pub no_build: bool,
    /// Build in release mode (monitoring-only).
    pub release: bool,
    /// Request the `write` scope and enable `setVariable`.
    pub allow_write: bool,
    /// Request the `force` scope.
    pub allow_force: bool,
    /// Override the protocol-path root instead of resolving package roots.
    pub source_root: Option<String>,
    /// Extra arguments appended to the launcher command line.
    pub args: Vec<String>,
    /// Working directory for the launched host.
    pub cwd: Option<String>,
    /// Endpoint file for `attach`.
    pub endpoint: Option<String>,
    /// Client stop-on-entry request; this adapter does not support entry stops
    /// and rejects `true` explicitly.
    pub stop_on_entry: bool,
}

impl LaunchArguments {
    /// Returns the project manifest path, applying `fallback` when the client
    /// omitted `project`.
    #[must_use]
    pub fn project_path(&self, fallback: Option<&std::path::Path>) -> std::path::PathBuf {
        match &self.project {
            Some(project) if !project.is_empty() => std::path::PathBuf::from(project),
            _ => fallback.map_or_else(
                || std::path::PathBuf::from("stc.toml"),
                std::path::Path::to_path_buf,
            ),
        }
    }
}

/// Returns the adapter's closed capability set (WL-1.1 I-9).
///
/// `supportsSetVariable` is advertised because the adapter implements the
/// request; the launch configuration's `allowWrite` (and the host's
/// `[debug].allow-write`) gate it at request time. Unsupported breakpoint and
/// restart features are advertised false.
#[must_use]
pub fn capabilities() -> Value {
    json!({
        "supportsConfigurationDoneRequest": true,
        "supportsSetVariable": true,
        "supportsTerminateRequest": true,
        "supportTerminateDebuggee": true,
        "supportsConditionalBreakpoints": false,
        "supportsHitConditionalBreakpoints": false,
        "supportsLogPoints": false,
        "supportsFunctionBreakpoints": false,
        "supportsDataBreakpoints": false,
        "supportsInstructionBreakpoints": false,
        "supportsRestartRequest": false,
        "supportsStepBack": false,
        "supportsSetExpression": false,
        "supportsDelayedStackTraceLoading": false,
        "supportsLoadedSourcesRequest": false,
        "supportsExceptionInfoRequest": false,
        "supportsExceptionOptions": false,
        "supportsEvaluateForHovers": true,
        "exceptionBreakpointFilters": []
    })
}

/// Builds one response message body.
#[must_use]
pub fn response(
    request_seq: i64,
    command: &str,
    success: bool,
    body: Option<Value>,
    message: Option<String>,
) -> Value {
    let mut object = Map::new();
    object.insert("type".into(), Value::String("response".into()));
    object.insert("request_seq".into(), json!(request_seq));
    object.insert("success".into(), json!(success));
    object.insert("command".into(), Value::String(command.into()));
    if let Some(message) = message {
        object.insert("message".into(), Value::String(message));
    }
    if let Some(body) = body {
        object.insert("body".into(), body);
    }
    Value::Object(object)
}

/// Builds one event message body.
#[must_use]
pub fn event(name: &str, body: Value) -> Value {
    json!({
        "type": "event",
        "event": name,
        "body": body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_request_and_rejects_other_messages() {
        let request = parse_request(br#"{"seq":3,"type":"request","command":"initialize"}"#)
            .expect("request");
        assert_eq!(request.seq, 3);
        assert_eq!(request.command, "initialize");
        assert!(request.arguments.is_null());

        for (body, expected) in [
            (br#"[]"#.as_slice(), "not a JSON object"),
            (
                br#"{"seq":1,"type":"event","event":"stopped"}"#.as_slice(),
                "only requests are accepted",
            ),
            (br#"{"seq":1,"command":"x"}"#.as_slice(), "no string `type`"),
            (
                br#"{"seq":0,"type":"request","command":"x"}"#.as_slice(),
                "must be a positive integer",
            ),
            (br#"{"seq":1,"type":"request"}"#.as_slice(), "malformed"),
        ] {
            let error = parse_request(body).expect_err("must reject");
            assert!(
                error.contains(expected),
                "expected `{expected}` in `{error}`"
            );
        }
    }

    #[test]
    fn launch_arguments_default_and_tolerate_client_keys() {
        let arguments: LaunchArguments = serde_json::from_value(json!({
            "name": "stc: Launch",
            "type": "stc",
            "request": "launch",
            "project": "demo/stc.toml",
            "noBuild": true,
            "allowWrite": true,
            "args": ["--cycles", "10"],
        }))
        .expect("arguments");
        assert!(arguments.no_build);
        assert!(arguments.allow_write);
        assert!(!arguments.allow_force);
        assert_eq!(arguments.args, vec!["--cycles", "10"]);
        assert_eq!(
            arguments.project_path(None),
            std::path::PathBuf::from("demo/stc.toml")
        );
        let defaulted: LaunchArguments = serde_json::from_value(json!({})).expect("empty");
        assert_eq!(
            defaulted.project_path(None),
            std::path::PathBuf::from("stc.toml")
        );
        let stopping: LaunchArguments =
            serde_json::from_value(json!({ "stopOnEntry": true })).expect("stopOnEntry");
        assert!(stopping.stop_on_entry);
    }

    #[test]
    fn capabilities_advertise_the_closed_set() {
        let capabilities = capabilities();
        assert_eq!(capabilities["supportsConfigurationDoneRequest"], true);
        assert_eq!(capabilities["supportsSetVariable"], true);
        assert_eq!(capabilities["supportsConditionalBreakpoints"], false);
        assert_eq!(capabilities["supportsHitConditionalBreakpoints"], false);
        assert_eq!(capabilities["supportsLogPoints"], false);
        assert_eq!(capabilities["supportsFunctionBreakpoints"], false);
        assert_eq!(capabilities["supportsDataBreakpoints"], false);
        assert_eq!(capabilities["supportsInstructionBreakpoints"], false);
        assert_eq!(capabilities["supportsRestartRequest"], false);
    }

    #[test]
    fn responses_and_events_have_the_dap_shape() {
        let response = response(7, "threads", true, Some(json!({"threads": []})), None);
        assert_eq!(response["type"], "response");
        assert_eq!(response["request_seq"], 7);
        assert_eq!(response["command"], "threads");
        assert_eq!(response["success"], true);
        let event = event("stopped", json!({"threadId": 1}));
        assert_eq!(event["type"], "event");
        assert_eq!(event["event"], "stopped");
        assert_eq!(event["body"]["threadId"], 1);
    }
}
