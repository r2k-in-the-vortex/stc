//! UI diagnostic snapshots (M6 phase 10).
//!
//! Each `tests/ui/<case>.st` carries rustc-style `//~ ERROR STC-E-...` or
//! `//~ WARNING STC-W-...` annotations on the line that owns the diagnostic's
//! primary span, plus a committed `<case>.stderr` with the full human render.
//! The harness runs `stc check` on the file (relative to the workspace root,
//! so committed paths are stable on both hosts) and fails on a missing or
//! unexpected diagnostic, a changed span, or a changed rendering.
//!
//! Regenerate snapshots with `STC_UPDATE_SNAPSHOTS=1 cargo test -p stc-driver
//! --test ui`; annotations are still checked in update mode.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn stc() -> &'static str {
    env!("CARGO_BIN_EXE_stc")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn ui_dir() -> PathBuf {
    workspace_root().join("tests/ui")
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Annotation {
    line: usize,
    severity: &'static str,
    code: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Actual {
    line: usize,
    severity: &'static str,
    code: String,
}

#[test]
fn ui_snapshots_match() {
    let update = std::env::var_os("STC_UPDATE_SNAPSHOTS").is_some();
    let directory = ui_dir();
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("st"))
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no UI cases in {}", directory.display());

    let mut failures = Vec::new();
    for path in &cases {
        let name = path.file_name().unwrap().to_string_lossy();
        match check_case(path, update) {
            Ok(()) => println!("ok ui {name}"),
            Err(message) => failures.push(format!("{name}:\n{message}")),
        }
    }
    if !failures.is_empty() {
        panic!(
            "{} UI case(s) failed:\n\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}

fn check_case(path: &Path, update: bool) -> Result<(), String> {
    let source =
        std::fs::read_to_string(path).map_err(|error| format!("cannot read source: {error}"))?;
    let annotations = parse_annotations(&source)?;
    let relative = format!("tests/ui/{}", path.file_name().unwrap().to_string_lossy());

    let output = Command::new(stc())
        .current_dir(workspace_root())
        .args(["check", &relative, "--color=never"])
        .output()
        .map_err(|error| format!("cannot run stc: {error}"))?;
    let stderr = normalized(&output);
    let actual = parse_actual(&stderr)?;

    let expected_exit = if annotations
        .iter()
        .any(|annotation| annotation.severity == "ERROR")
    {
        1
    } else {
        0
    };
    let code = output.status.code();
    if code != Some(expected_exit) {
        return Err(format!(
            "expected exit {expected_exit}, got {code:?}\nstderr:\n{stderr}"
        ));
    }
    if !output.stdout.is_empty() {
        return Err(format!(
            "expected an empty stdout, got:\n{}",
            String::from_utf8_lossy(&output.stdout)
        ));
    }

    let expected: Vec<Annotation> = annotations
        .iter()
        .map(|annotation| Annotation {
            line: annotation.line,
            severity: annotation.severity,
            code: annotation.code.clone(),
        })
        .collect();
    let mut expected = expected;
    expected.sort();
    let actual_annotations: Vec<Annotation> = actual
        .iter()
        .map(|actual| Annotation {
            line: actual.line,
            severity: actual.severity,
            code: actual.code.clone(),
        })
        .collect();
    if expected != actual_annotations {
        return Err(format!(
            "annotations do not match diagnostics\n  expected: {expected:?}\n  actual:   {actual_annotations:?}\nstderr:\n{stderr}"
        ));
    }

    let snapshot = path.with_extension("stderr");
    if update {
        std::fs::write(&snapshot, format!("{stderr}\n"))
            .map_err(|error| format!("cannot write {}: {error}", snapshot.display()))?;
        return Ok(());
    }
    let expected_stderr = std::fs::read_to_string(&snapshot).map_err(|error| {
        format!(
            "cannot read {}: {error}\nrun with STC_UPDATE_SNAPSHOTS=1 to create it",
            snapshot.display()
        )
    })?;
    if normalize(&expected_stderr) != stderr {
        return Err(format!(
            "stderr does not match {}\n--- expected ---\n{}\n--- actual ---\n{}",
            snapshot.display(),
            normalize(&expected_stderr),
            stderr
        ));
    }
    Ok(())
}

fn parse_annotations(source: &str) -> Result<Vec<Annotation>, String> {
    let mut annotations = Vec::new();
    for (index, line) in source.lines().enumerate() {
        for (offset, _) in line.match_indices("//~") {
            let rest = line[offset + 3..].trim();
            let mut parts = rest.split_whitespace();
            let severity = parts
                .next()
                .ok_or_else(|| format!("line {}: `//~` without a severity", index + 1))?;
            let severity = match severity {
                "ERROR" => "ERROR",
                "WARNING" => "WARNING",
                other => {
                    return Err(format!(
                        "line {}: unsupported annotation severity `{other}` \
                         (expected ERROR or WARNING)",
                        index + 1
                    ));
                }
            };
            let code = parts.next().ok_or_else(|| {
                format!(
                    "line {}: `//~ {severity}` without a diagnostic code",
                    index + 1
                )
            })?;
            if !code.starts_with("STC-") {
                return Err(format!(
                    "line {}: annotation code `{code}` is not an `STC-*` code",
                    index + 1
                ));
            }
            annotations.push(Annotation {
                line: index + 1,
                severity,
                code: code.to_owned(),
            });
        }
    }
    Ok(annotations)
}

fn parse_actual(stderr: &str) -> Result<Vec<Actual>, String> {
    let mut actual = Vec::new();
    for line in stderr.lines() {
        for (marker, severity) in [(": error[", "ERROR"), (": warning[", "WARNING")] {
            let Some(index) = line.find(marker) else {
                continue;
            };
            let location = &line[..index];
            let rest = &line[index + marker.len()..];
            let Some(end) = rest.find(']') else {
                continue;
            };
            let code = &rest[..end];
            let Some((line_number, _)) = parse_location(location) else {
                return Err(format!("cannot parse diagnostic location in `{line}`"));
            };
            actual.push(Actual {
                line: line_number,
                severity,
                code: code.to_owned(),
            });
            break;
        }
    }
    actual.sort();
    Ok(actual)
}

/// Parses `path:line:column` (the path may itself contain a drive colon).
fn parse_location(location: &str) -> Option<(usize, usize)> {
    let mut parts = location.rsplitn(3, ':');
    let column = parts.next()?.parse().ok()?;
    let line = parts.next()?.parse().ok()?;
    parts.next()?;
    Some((line, column))
}

fn normalized(output: &Output) -> String {
    normalize(&String::from_utf8_lossy(&output.stderr))
}

fn normalize(text: &str) -> String {
    let mut text = text.replace("\r\n", "\n");
    while text.ends_with('\n') {
        text.pop();
    }
    text
}
