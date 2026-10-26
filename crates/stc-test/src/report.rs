//! Stable human and JSON test reports (CLI and Manifest §1.6).

use serde_json::{Value, json};

use crate::runner::{Assertion, TestOutcome};

/// Returns the human line for one test ("test NAME ... ok").
#[must_use]
pub fn human_result_line(name: &str, outcome: &TestOutcome) -> String {
    match outcome.status {
        crate::TestStatus::Pass => format!("test {name} ... ok"),
        crate::TestStatus::Fail => format!("test {name} ... FAILED"),
        crate::TestStatus::Timeout => format!("test {name} ... TIMEOUT"),
        crate::TestStatus::Trap => match &outcome.trap {
            Some(trap) => format!("test {name} ... TRAPPED ({trap})"),
            None => format!("test {name} ... TRAPPED"),
        },
    }
}

/// Returns one human line per failure, indented under the test line.
#[must_use]
pub fn human_failure_lines(outcome: &TestOutcome) -> Vec<String> {
    outcome
        .failures
        .iter()
        .map(|failure| {
            let location = match (&failure.file, failure.line, failure.column) {
                (Some(file), Some(line), Some(column)) => {
                    format!(" ({file}:{line}:{column})")
                }
                (Some(file), Some(line), None) => format!(" ({file}:{line})"),
                (Some(file), None, _) => format!(" ({file})"),
                _ => String::new(),
            };
            format!("  {}{location}", failure.message)
        })
        .collect()
}

/// Returns the stable JSON result object for one test.
#[must_use]
pub fn json_result(name: &str, outcome: &TestOutcome) -> String {
    let failures: Vec<Value> = outcome.failures.iter().map(assertion_json).collect();
    json!({
        "name": name,
        "status": outcome.status.as_str(),
        "duration_ms": outcome.duration_ms,
        "failures": failures,
        "trap": outcome.trap,
    })
    .to_string()
}

fn assertion_json(failure: &Assertion) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("message".into(), Value::String(failure.message.clone()));
    if let Some(file) = &failure.file {
        object.insert("file".into(), Value::String(file.clone()));
    }
    if let Some(line) = failure.line {
        object.insert("line".into(), Value::from(line));
    }
    if let Some(column) = failure.column {
        object.insert("column".into(), Value::from(column));
    }
    Value::Object(object)
}

/// Returns the final summary line.
#[must_use]
pub fn summary_line(passed: usize, failed: usize, timed_out: usize, trapped: usize) -> String {
    let verdict = if failed == 0 && timed_out == 0 && trapped == 0 {
        "ok"
    } else {
        "FAILED"
    };
    format!(
        "test result: {verdict}. {passed} passed; {failed} failed; {timed_out} timed out; {trapped} trapped"
    )
}
