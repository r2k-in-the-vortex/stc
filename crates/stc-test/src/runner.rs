//! Process-isolated test runner (Testing and Verification §2.3).
//!
//! Each test runs in its own test-host process. The runner enforces the
//! per-test timeout, kills the process on expiry, and parses the host's
//! structured result line. It never shares a process between tests.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Test outcome status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestStatus {
    /// Passed.
    Pass,
    /// Failed assertions or a runtime error.
    Fail,
    /// The module trapped unexpectedly.
    Trap,
    /// The process exceeded its timeout and was killed.
    Timeout,
}

impl TestStatus {
    /// Returns the CLI report spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Trap => "trap",
            Self::Timeout => "timeout",
        }
    }
}

/// One recorded assertion failure or synthetic test failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assertion {
    /// Failure message.
    pub message: String,
    /// Package-relative source file, when known.
    pub file: Option<String>,
    /// 1-based line, when known.
    pub line: Option<u32>,
    /// 1-based column, when known.
    pub column: Option<u32>,
}

/// The result of running one test process.
#[derive(Clone, Debug)]
pub struct TestOutcome {
    /// Outcome status.
    pub status: TestStatus,
    /// Wall-clock time spent in the test process.
    pub duration_ms: u64,
    /// Recorded failures.
    pub failures: Vec<Assertion>,
    /// Trap name (`STC_TRAP_*`) for an unexpected trap.
    pub trap: Option<String>,
    /// Completed scan count reported by the test host.
    pub cycles: u64,
    /// Captured stdout (test body output plus the result line).
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
    /// Missing or malformed result-line diagnostics.
    pub internal_error: Option<String>,
}

impl TestOutcome {
    fn internal_error(message: String, duration_ms: u64, stdout: String, stderr: String) -> Self {
        Self {
            status: TestStatus::Fail,
            duration_ms,
            failures: vec![Assertion {
                message: message.clone(),
                file: None,
                line: None,
                column: None,
            }],
            trap: None,
            cycles: 0,
            stdout,
            stderr,
            internal_error: Some(message),
        }
    }
}

/// Runs one test host and parses its structured result.
///
/// On timeout the child is killed and the outcome is [`TestStatus::Timeout`].
#[must_use]
pub fn run_process(exe: &Path, args: &[String], timeout: Duration) -> TestOutcome {
    let started = Instant::now();
    let mut child = match Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return TestOutcome::internal_error(
                format!("cannot start test host `{}`: {error}", exe.display()),
                elapsed_ms(started),
                String::new(),
                String::new(),
            );
        }
    };
    let stdout_reader = child.stdout.take().map(spawn_reader);
    let stderr_reader = child.stderr.take().map(spawn_reader);
    let deadline = started + timeout;
    let timed_out;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                timed_out = false;
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => {
                return TestOutcome::internal_error(
                    format!("cannot wait for test host: {error}"),
                    elapsed_ms(started),
                    String::new(),
                    String::new(),
                );
            }
        }
    }
    let duration_ms = elapsed_ms(started);
    let stdout = stdout_reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    let stderr = stderr_reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    if timed_out {
        return TestOutcome {
            status: TestStatus::Timeout,
            duration_ms,
            failures: vec![Assertion {
                message: format!("test timed out after {} ms", timeout.as_millis()),
                file: None,
                line: None,
                column: None,
            }],
            trap: None,
            cycles: 0,
            stdout,
            stderr,
            internal_error: None,
        };
    }
    parse_result(stdout, duration_ms, stderr)
}

fn spawn_reader(mut stream: impl Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stream.read_to_string(&mut text);
        text
    })
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn parse_result(captured_stdout: String, duration_ms: u64, stderr: String) -> TestOutcome {
    let Some(line) = captured_stdout
        .lines()
        .rev()
        .find(|line| line.starts_with("{\"kind\":\"stc-test-result\""))
    else {
        return TestOutcome::internal_error(
            "test host did not print a structured result line".into(),
            duration_ms,
            captured_stdout,
            stderr,
        );
    };
    let parsed: serde_json::Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(error) => {
            return TestOutcome::internal_error(
                format!("malformed test-host result line: {error}"),
                duration_ms,
                captured_stdout,
                stderr,
            );
        }
    };
    let status = match parsed.get("status").and_then(serde_json::Value::as_str) {
        Some("pass") => TestStatus::Pass,
        Some("fail") => TestStatus::Fail,
        Some("trap") => TestStatus::Trap,
        other => {
            return TestOutcome::internal_error(
                format!("unknown test status {other:?}"),
                duration_ms,
                captured_stdout,
                stderr,
            );
        }
    };
    let failures = parsed
        .get("assertions")
        .and_then(serde_json::Value::as_array)
        .map(|entries| entries.iter().map(parse_assertion).collect())
        .unwrap_or_default();
    let trap = parsed
        .get("trap")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let cycles = parsed
        .get("cycles")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    TestOutcome {
        status,
        duration_ms,
        failures,
        trap,
        cycles,
        stdout: captured_stdout,
        stderr,
        internal_error: None,
    }
}

fn parse_assertion(value: &serde_json::Value) -> Assertion {
    Assertion {
        message: value
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("assertion failed")
            .to_owned(),
        file: value
            .get("file")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        line: value
            .get("line")
            .and_then(serde_json::Value::as_u64)
            .and_then(|line| u32::try_from(line).ok()),
        column: value
            .get("column")
            .and_then(serde_json::Value::as_u64)
            .and_then(|column| u32::try_from(column).ok()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASS_LINE: &str = "{\"kind\":\"stc-test-result\",\"name\":\"T\",\"status\":\"pass\",\"assertions\":[],\"trap\":null,\"cycles\":3}";

    #[test]
    fn parses_the_structured_result_line() {
        let outcome = parse_result(format!("body output\n{PASS_LINE}\n"), 12, String::new());
        assert_eq!(outcome.status, TestStatus::Pass);
        assert_eq!(outcome.duration_ms, 12);
        assert_eq!(outcome.cycles, 3);
        assert!(outcome.failures.is_empty());
        assert_eq!(outcome.trap, None);
    }

    #[test]
    fn maps_failures_and_traps() {
        let line = "{\"kind\":\"stc-test-result\",\"name\":\"T\",\"status\":\"fail\",\"assertions\":[{\"message\":\"boom\",\"file\":\"t.st\",\"line\":2,\"column\":4}],\"trap\":null,\"cycles\":1}";
        let outcome = parse_result(line.to_owned(), 1, String::new());
        assert_eq!(outcome.status, TestStatus::Fail);
        assert_eq!(outcome.failures.len(), 1);
        assert_eq!(outcome.failures[0].message, "boom");
        assert_eq!(outcome.failures[0].file.as_deref(), Some("t.st"));
        assert_eq!(outcome.failures[0].line, Some(2));
        assert_eq!(outcome.failures[0].column, Some(4));

        let line = "{\"kind\":\"stc-test-result\",\"name\":\"T\",\"status\":\"trap\",\"assertions\":[],\"trap\":\"STC_TRAP_DIV_ZERO\",\"cycles\":0}";
        let outcome = parse_result(line.to_owned(), 1, String::new());
        assert_eq!(outcome.status, TestStatus::Trap);
        assert_eq!(outcome.trap.as_deref(), Some("STC_TRAP_DIV_ZERO"));
    }

    #[test]
    fn missing_or_malformed_results_are_internal_errors() {
        let missing = parse_result("no result here\n".into(), 1, String::new());
        assert_eq!(missing.status, TestStatus::Fail);
        assert!(missing.internal_error.is_some());

        let malformed = parse_result(
            "{\"kind\":\"stc-test-result\",\"name\":\"T\",\"status\":\"bogus\"}".into(),
            1,
            String::new(),
        );
        assert_eq!(malformed.status, TestStatus::Fail);
        assert!(malformed.internal_error.is_some());
    }
}
