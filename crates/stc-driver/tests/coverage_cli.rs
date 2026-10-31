//! `stc test` coverage integration tests (WL-10 phase 3): instrumented builds,
//! raw sidecars, report files, per-test covered indices, overrides, filters,
//! timeouts, JSON output, and cleaning.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

fn stc() -> &'static str {
    env!("CARGO_BIN_EXE_stc")
}

fn clang() -> Option<&'static str> {
    static CLANG: OnceLock<Option<&'static str>> = OnceLock::new();
    CLANG
        .get_or_init(|| {
            let candidate = std::env::var("STC_CLANG").unwrap_or_else(|_| "clang".into());
            match Command::new(&candidate).arg("--version").output() {
                Ok(output) if output.status.success() => {
                    Some(Box::leak(candidate.into_boxed_str()))
                }
                _ => {
                    if std::env::var_os("STC_REQUIRE_CLANG").is_some() {
                        panic!("clang is required (STC_REQUIRE_CLANG=1) but was not found");
                    }
                    None
                }
            }
        })
        .as_ref()
        .copied()
}

fn runtime_artifact() -> PathBuf {
    common::runtime_static()
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stc-coverage-phase3-{}-{name}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test directory");
        Self { path }
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create directory");
        }
        std::fs::write(path, contents).expect("write file");
    }

    fn manifest(&self, test_table: &str) {
        self.write(
            "stc.toml",
            &format!(
                "schema = 1\n\n\
                 [package]\n\
                 name = \"demo\"\n\
                 version = \"0.1.0\"\n\
                 language = \"stc-2013-oop-utf8-v1\"\n\n\
                 [project]\n\
                 sources = [\"src/**/*.st\"]\n\n\
                 [test]\n\
                 sources = [\"tests/**/*.st\"]\n\
                 {test_table}\n"
            ),
        );
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(stc())
            .args(args)
            .current_dir(&self.path)
            .output()
            .expect("run stc")
    }

    fn run_test(&self, args: &[&str]) -> Output {
        let runtime = runtime_artifact();
        let mut all = vec!["test"];
        all.extend_from_slice(args);
        all.push("--runtime");
        let runtime_text = runtime.to_string_lossy().into_owned();
        all.push(&runtime_text);
        self.run(&all)
    }

    fn coverage_dir(&self) -> PathBuf {
        self.path.join("target/stc/coverage")
    }

    fn report(&self) -> Value {
        let path = self.coverage_dir().join("coverage.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read `{}`: {error}", path.display()));
        serde_json::from_str(&text).expect("coverage.json parses")
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

fn find_files(root: &Path, suffix: &str, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_files(&path, suffix, found);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(suffix))
        {
            found.push(path);
        }
    }
}

fn files_with(root: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    find_files(root, suffix, &mut found);
    found.sort();
    found
}

/// Writes a production file with two independent functions and one test per
/// function; each test exercises exactly one of the two statement sites.
fn write_coverage_project(directory: &TestDir) {
    directory.manifest("");
    directory.write(
        "src/stats.st",
        "FUNCTION AddOne : DINT\n\
         VAR_INPUT\n\
           x : DINT;\n\
         END_VAR\n\
           AddOne := x + 1;\n\
         END_FUNCTION\n\
         \n\
         FUNCTION DoubleIt : DINT\n\
         VAR_INPUT\n\
           x : DINT;\n\
         END_VAR\n\
           DoubleIt := x * 2;\n\
         END_FUNCTION\n",
    );
    directory.write(
        "tests/coverage.st",
        "{stc:test}\n\
         FUNCTION UsesAddOne : BOOL\n\
         VAR\n\
           r : DINT;\n\
         END_VAR\n\
           r := AddOne(1);\n\
           UsesAddOne := r = 2;\n\
         END_FUNCTION\n\
         \n\
         {stc:test}\n\
         FUNCTION UsesDoubleIt : BOOL\n\
         VAR\n\
           r : DINT;\n\
         END_VAR\n\
           r := DoubleIt(2);\n\
           UsesDoubleIt := r = 4;\n\
         END_FUNCTION\n",
    );
}

fn covered(test: &Value) -> Vec<u64> {
    test["covered"]
        .as_array()
        .unwrap_or_else(|| panic!("no covered array in {test}"))
        .iter()
        .map(|index| index.as_u64().expect("covered index"))
        .collect()
}

#[test]
fn coverage_writes_reports_sidecars_and_per_test_indices() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("reports");
    write_coverage_project(&directory);
    let output = directory.run_test(&["--coverage"]);
    assert_eq!(code(&output), 0, "{}\n{}", stdout(&output), stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("test UsesAddOne ... ok"), "{text}");
    assert!(text.contains("test UsesDoubleIt ... ok"), "{text}");
    assert!(text.contains("coverage: 2/2 statements (100.0%)"), "{text}");
    assert!(text.contains("  src/stats.st: 2/2 statements"), "{text}");
    assert!(
        text.find("2 passed").unwrap() < text.find("coverage:").unwrap(),
        "the coverage summary follows the test summary: {text}"
    );

    let report = directory.report();
    assert_eq!(report["protocol"], "stc-coverage/1");
    assert_eq!(report["kind"], "statement");
    assert_eq!(report["package"], "demo");
    assert_eq!(report["totals"]["sites"], 2);
    assert_eq!(report["totals"]["covered"], 2);
    assert_eq!(report["totals"]["percent"], 100.0);
    assert_eq!(report["totals"]["lines"], 2);
    assert_eq!(report["totals"]["lines_covered"], 2);
    assert_eq!(report["totals"]["line_percent"], 100.0);
    assert_eq!(report["files"][0]["file"], "src/stats.st");
    assert_eq!(report["files"][0]["sites"], 2);
    assert_eq!(report["files"][0]["covered"], 2);
    let sites = report["sites"].as_array().expect("sites array");
    assert_eq!(sites.len(), 2);
    assert_eq!(sites[0]["line"], 5);
    // The statement span starts at the beginning of its source line, so the
    // reported column follows `SourceMap::location` (1-based).
    assert_eq!(sites[0]["column"], 1);
    assert_eq!(sites[1]["line"], 12);
    assert_eq!(sites[0]["count"], 1);
    assert_eq!(sites[1]["count"], 1);

    let tests = report["tests"].as_array().expect("tests array");
    assert_eq!(tests.len(), 2);
    assert_eq!(tests[0]["name"], "UsesAddOne");
    assert_eq!(tests[0]["status"], "pass");
    assert_eq!(tests[0]["coverage"], "reported");
    assert_eq!(tests[0]["sites_covered"], 1);
    assert_eq!(tests[1]["name"], "UsesDoubleIt");
    assert_eq!(tests[1]["coverage"], "reported");
    let first = covered(&tests[0]);
    let second = covered(&tests[1]);
    assert_eq!(first.len(), 1, "{report}");
    assert_eq!(second.len(), 1, "{report}");
    assert_ne!(first, second, "the tests cover disjoint sites");
    let union: BTreeSet<u64> = first.iter().chain(second.iter()).copied().collect();
    assert_eq!(union, BTreeSet::from([0, 1]));

    let info = std::fs::read_to_string(directory.coverage_dir().join("coverage.info"))
        .expect("coverage.info");
    assert_eq!(
        info,
        "TN:\nSF:src/stats.st\nDA:5,1\nDA:12,1\nLF:2\nLH:2\nend_of_record\n"
    );

    let sidecars = files_with(&directory.path.join("target/stc/test"), ".coverage");
    assert_eq!(sidecars.len(), 2, "{sidecars:?}");
    let mut ids = BTreeSet::new();
    for path in &sidecars {
        let raw = std::fs::read_to_string(path).expect("raw sidecar");
        let parsed = stc_test::parse_raw_coverage(&raw).expect("valid raw sidecar");
        assert_eq!(parsed.sites, 2, "{path:?}");
        assert_eq!(parsed.counts.len(), 1, "{path:?}: {raw}");
        assert_eq!(parsed.counts[0].1, 1, "{path:?}: {raw}");
        ids.insert(parsed.counts[0].0);
    }
    assert_eq!(ids, BTreeSet::from([0, 1]), "one sidecar per test site");
}

#[test]
fn coverage_is_off_by_default_and_no_coverage_overrides_manifest() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("toggle");
    write_coverage_project(&directory);

    let plain = directory.run_test(&["UsesAddOne"]);
    assert_eq!(code(&plain), 0, "{}", stderr(&plain));
    assert!(!directory.coverage_dir().exists());
    assert!(files_with(&directory.path.join("target/stc/test"), ".coverage").is_empty());

    directory.manifest("coverage = true");
    let enabled = directory.run_test(&["UsesAddOne"]);
    assert_eq!(
        code(&enabled),
        0,
        "{}\n{}",
        stdout(&enabled),
        stderr(&enabled)
    );
    assert!(directory.coverage_dir().join("coverage.json").is_file());
    assert!(directory.coverage_dir().join("coverage.info").is_file());

    std::fs::remove_dir_all(directory.coverage_dir()).expect("remove reports");
    let disabled = directory.run_test(&["UsesAddOne", "--no-coverage"]);
    assert_eq!(code(&disabled), 0, "{}", stderr(&disabled));
    assert!(
        !directory.coverage_dir().exists(),
        "--no-coverage overrides `[test] coverage = true`"
    );
}

#[test]
fn coverage_out_requires_effective_coverage() {
    let directory = TestDir::new("out-requires");
    write_coverage_project(&directory);
    let output = directory.run_test(&["--coverage-out", "reports"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("requires coverage"),
        "{}",
        stderr(&output)
    );
    assert!(!directory.path.join("reports").exists());
}

#[test]
fn no_run_builds_instrumented_artifacts_without_reports() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("no-run");
    write_coverage_project(&directory);
    let output = directory.run_test(&["--coverage", "--no-run"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("no coverage report"),
        "{}",
        stderr(&output)
    );
    assert!(!directory.coverage_dir().exists());
    assert!(files_with(&directory.path.join("target/stc/test"), ".coverage").is_empty());

    let artifacts = files_with(&directory.path.join("target/stc/test"), ".stc-meta.json");
    assert_eq!(artifacts.len(), 2, "one artifact per test");
    let ir = files_with(&directory.path.join("target/stc/test"), ".ll");
    assert!(
        ir.iter().any(|path| {
            std::fs::read_to_string(path).is_ok_and(|text| text.contains("stc_coverage_counters"))
        }),
        "instrumentation is compiled into the artifact: {ir:?}"
    );
}

#[test]
fn filters_limit_tests_included_in_the_report() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("filters");
    write_coverage_project(&directory);
    let output = directory.run_test(&["--coverage", "UsesAddOne"]);
    assert_eq!(code(&output), 0, "{}\n{}", stdout(&output), stderr(&output));
    assert!(
        !stdout(&output).contains("UsesDoubleIt"),
        "{}",
        stdout(&output)
    );
    let report = directory.report();
    let tests = report["tests"].as_array().expect("tests array");
    assert_eq!(tests.len(), 1, "{report}");
    assert_eq!(tests[0]["name"], "UsesAddOne");
    assert_eq!(report["totals"]["sites"], 2);
    assert_eq!(report["totals"]["covered"], 1);
}

#[test]
fn timeouts_are_marked_without_coverage_data() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("timeout");
    directory.manifest("");
    directory.write(
        "src/stats.st",
        "FUNCTION AddOne : DINT\n\
         VAR_INPUT\n\
           x : DINT;\n\
         END_VAR\n\
           AddOne := x + 1;\n\
         END_FUNCTION\n",
    );
    directory.write(
        "tests/hang.st",
        "{stc:test {\"timeout\": \"150ms\"}}\n\
         PROGRAM Hang\n\
         VAR\n\
           i : DINT;\n\
         END_VAR\n\
           WHILE TRUE DO\n\
             i := i + 1;\n\
           END_WHILE;\n\
         END_PROGRAM\n",
    );
    let output = directory.run_test(&["--coverage", "Hang"]);
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    assert!(stdout(&output).contains("test Hang ... TIMEOUT"));
    let report = directory.report();
    let tests = report["tests"].as_array().expect("tests array");
    assert_eq!(tests.len(), 1);
    assert_eq!(tests[0]["name"], "Hang");
    assert_eq!(tests[0]["status"], "timeout");
    assert_eq!(tests[0]["coverage"], "timeout");
    assert!(
        tests[0].get("covered").is_none(),
        "timeouts have no covered indices: {report}"
    );
    assert_eq!(report["totals"]["covered"], 0);
    assert!(
        !stderr(&output).contains("STC-W-COVERAGE-001"),
        "a timeout never reads a sidecar: {}",
        stderr(&output)
    );
}

#[test]
fn json_mode_keeps_one_result_object_per_test() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("json");
    write_coverage_project(&directory);
    let output = directory.run_test(&["--coverage", "--message-format", "json"]);
    assert_eq!(code(&output), 0, "{}\n{}", stdout(&output), stderr(&output));
    let text = stdout(&output);
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(lines.len(), 2, "one result object per test: {text}");
    for line in lines {
        let value: Value = serde_json::from_str(line).expect("result line is JSON");
        assert!(value["name"].is_string(), "{line}");
        assert!(value.get("coverage").is_none(), "{line}");
    }
    assert!(!text.contains("coverage:"), "no human summary in JSON mode");
    assert!(
        stderr(&output).contains("coverage report"),
        "report paths are logged on stderr: {}",
        stderr(&output)
    );
    assert!(directory.coverage_dir().join("coverage.json").is_file());
}

#[test]
fn list_rejects_explicit_coverage_and_ignores_the_manifest_default() {
    let directory = TestDir::new("list");
    write_coverage_project(&directory);
    directory.manifest("coverage = true");

    let listed = directory.run_test(&["--list"]);
    assert_eq!(code(&listed), 0, "{}", stderr(&listed));
    assert_eq!(stdout(&listed), "UsesAddOne\nUsesDoubleIt\n");
    assert!(
        !directory.coverage_dir().exists(),
        "`--list` never writes a coverage report"
    );

    let explicit = directory.run_test(&["--list", "--coverage"]);
    assert_eq!(code(&explicit), 2, "{}", stderr(&explicit));
    assert!(
        stderr(&explicit).contains("cannot be combined with `--list`"),
        "{}",
        stderr(&explicit)
    );

    let out = directory.run_test(&["--list", "--coverage-out", "reports"]);
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    assert!(
        stderr(&out).contains("cannot be combined with `--list`"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn quiet_suppresses_the_human_summary_but_still_writes_reports() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("quiet");
    write_coverage_project(&directory);
    let output = directory.run_test(&["--coverage", "UsesAddOne", "-q"]);
    assert_eq!(code(&output), 0, "{}\n{}", stdout(&output), stderr(&output));
    assert!(stdout(&output).is_empty(), "{}", stdout(&output));
    assert!(directory.coverage_dir().join("coverage.json").is_file());
    assert!(directory.coverage_dir().join("coverage.info").is_file());
}

#[test]
fn a_report_write_failure_is_a_build_error() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("write-failure");
    write_coverage_project(&directory);
    std::fs::create_dir_all(directory.path.join("target/stc")).expect("create target");
    std::fs::write(directory.coverage_dir(), "not a directory").expect("block coverage dir");
    let output = directory.run_test(&["--coverage", "UsesAddOne"]);
    assert_eq!(code(&output), 1, "{}\n{}", stdout(&output), stderr(&output));
    assert!(
        stderr(&output).contains("STC-E-BUILD-002"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn clean_removes_default_coverage_but_preserves_a_custom_out_dir() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("clean");
    write_coverage_project(&directory);
    let output = directory.run_test(&["--coverage"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(directory.coverage_dir().join("coverage.json").is_file());

    let clean = directory.run(&["clean", "--test"]);
    assert_eq!(code(&clean), 0, "{}", stderr(&clean));
    assert!(!directory.coverage_dir().exists());
    assert!(!directory.path.join("target/stc/test").exists());

    let custom = directory.path.join("reports");
    let output = directory.run_test(&["--coverage", "--coverage-out", "reports"]);
    assert_eq!(code(&output), 0, "{}\n{}", stdout(&output), stderr(&output));
    assert!(custom.join("coverage.json").is_file());
    let clean = directory.run(&["clean"]);
    assert_eq!(code(&clean), 0, "{}", stderr(&clean));
    assert!(
        custom.join("coverage.json").is_file(),
        "a custom --coverage-out directory is not a clean target"
    );
    assert!(!directory.coverage_dir().exists());
}
