//! Coverage evidence (WL-10 phase 4): committed golden reports for the
//! example project, determinism and host-independence, instrumented versus
//! uninstrumented test semantics, production-build isolation, exclusions,
//! dependency-package labels, and optimization-level stability.
//!
//! The golden files under `tests/coverage/expected/` are host-independent:
//! LF line endings, package-relative forward-slash paths, and no durations,
//! build IDs, absolute paths, timestamps, or host data. Regenerate them with
//! `STC_UPDATE_SNAPSHOTS=1 cargo test -p stc-driver --test coverage_evidence`,
//! following the UI snapshot convention.
//!
//! Missing and malformed raw-sidecar handling is covered by the `stc-test`
//! parser and aggregator unit tests and by the driver's site-attribution unit
//! test. Producing one from an end-to-end run would require a test-only hook
//! in production code, which WL-10 deliberately avoids, so no such test is
//! added here.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

/// The exported counter global; production artifacts must never contain it.
const COUNTER_SYMBOL: &[u8] = b"stc_coverage_counters";

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

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn example_source() -> PathBuf {
    workspace_root().join("examples/coverage")
}

fn expected_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/coverage/expected")
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
            "stc-coverage-evidence-{}-{name}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test directory");
        Self { path }
    }

    fn path(&self, relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(self.path.clone(), |path, part| path.join(part))
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create directory");
        }
        std::fs::write(path, contents).expect("write file");
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(stc())
            .args(args)
            .arg("--color=never")
            .current_dir(&self.path)
            .output()
            .expect("run stc")
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create directory");
    for entry in std::fs::read_dir(from).expect("read directory") {
        let entry = entry.expect("directory entry");
        let name = entry.file_name();
        if matches!(name.to_str(), Some("target" | "state")) {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().expect("entry type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

fn copy_example(directory: &TestDir, name: &str) -> PathBuf {
    let target = directory.path(name);
    copy_tree(&example_source(), &target);
    target
}

/// A `stc test` command rooted at `directory` with the runtime artifact.
fn stc_test(directory: &Path) -> Command {
    let mut command = Command::new(stc());
    command
        .arg("test")
        .arg("--color=never")
        .arg("--runtime")
        .arg(runtime_artifact())
        .current_dir(directory);
    command
}

/// Runs `stc test` on the committed example copy with `--locked`.
fn run_example(example: &Path, args: &[&str]) -> Output {
    let mut command = stc_test(example);
    command.args(args).arg("--locked");
    command.output().expect("run stc test")
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

fn assert_success(output: &Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
}

/// Normalizes CRLF to LF and strips trailing newlines for comparisons.
fn normalized(text: &str) -> String {
    let mut text = text.replace("\r\n", "\n");
    while text.ends_with('\n') {
        text.pop();
    }
    text
}

fn read_text(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read `{}`: {error}", path.display()))
}

fn collect_files(root: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files);
        } else {
            files.push(path);
        }
    }
}

fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_files(root, &mut files);
    files
}

fn contains_symbol(path: &Path) -> bool {
    std::fs::read(path).is_ok_and(|bytes| {
        bytes
            .windows(COUNTER_SYMBOL.len())
            .any(|window| window == COUNTER_SYMBOL)
    })
}

/// The test-host executable statically links the runtime, whose result
/// collection mentions the counter symbol by name even when the tested module
/// is uninstrumented. Coverage isolation is about the module, so the launcher
/// executable is not part of the check.
fn is_test_host_executable(path: &Path) -> bool {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("exe") => true,
        // On Unix the launcher has no extension.
        None => true,
        _ => false,
    }
}

fn assert_no_coverage_outputs(root: &Path) {
    let target = root.join("target");
    for path in files_under(&target) {
        assert!(
            path.extension().and_then(|extension| extension.to_str()) != Some("coverage"),
            "a coverage sidecar was written: {}",
            path.display()
        );
    }
    assert!(
        !root.join("target/stc/coverage").exists(),
        "the coverage report directory exists under {}",
        root.display()
    );
}

/// Writes (or compares) one golden file; comparisons are CRLF-insensitive and
/// the committed file is always written with LF and one trailing newline.
fn compare_or_update(path: &Path, actual: &str, update: bool) {
    let actual = normalized(actual);
    if update {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create expected directory");
        }
        std::fs::write(path, format!("{actual}\n"))
            .unwrap_or_else(|error| panic!("cannot write `{}`: {error}", path.display()));
        eprintln!("updated {}", path.display());
        return;
    }
    let expected = std::fs::read_to_string(path).unwrap_or_else(|error| {
        panic!(
            "cannot read `{}`: {error}\nrun with STC_UPDATE_SNAPSHOTS=1 to create it",
            path.display()
        )
    });
    assert_eq!(
        actual,
        normalized(&expected),
        "{} does not match the committed golden file; run with STC_UPDATE_SNAPSHOTS=1 to update \
         it and review the diff",
        path.display()
    );
}

fn covered_indices(test: &Value) -> Vec<u64> {
    test["covered"]
        .as_array()
        .unwrap_or_else(|| panic!("no covered array in {test}"))
        .iter()
        .map(|index| index.as_u64().expect("covered index"))
        .collect()
}

fn find_line(source: &str, needle: &str) -> u64 {
    source
        .lines()
        .position(|line| line.contains(needle))
        .map_or_else(
            || panic!("`{needle}` not found in the example source"),
            |index| index as u64 + 1,
        )
}

#[test]
fn golden_reports_match_the_committed_example() {
    if clang().is_none() {
        return;
    }
    let update = std::env::var_os("STC_UPDATE_SNAPSHOTS").is_some();
    let directory = TestDir::new("golden");
    let example = copy_example(&directory, "example");
    let output = run_example(&example, &["--coverage"]);
    assert_eq!(
        code(&output),
        0,
        "coverage run failed:\n{}\n{}",
        stdout(&output),
        stderr(&output)
    );

    let coverage_dir = example.join("target/stc/coverage");
    let json_path = coverage_dir.join("coverage.json");
    let info_path = coverage_dir.join("lcov.info");
    let expected = expected_dir();
    compare_or_update(
        &expected.join("coverage.json"),
        &read_text(&json_path),
        update,
    );
    compare_or_update(&expected.join("lcov.info"), &read_text(&info_path), update);
    compare_or_update(&expected.join("coverage.stdout"), &stdout(&output), update);

    // The golden report carries the suite union and per-test covered indices.
    let report: Value = serde_json::from_str(&read_text(&json_path)).expect("coverage.json parses");
    assert_eq!(report["protocol"], "stc-coverage/1");
    assert_eq!(report["package"], "coverage-demo");
    assert_eq!(report["kind"], "statement");
    assert_eq!(report["totals"]["sites"], 5);
    assert_eq!(report["totals"]["covered"], 3);
    assert_eq!(report["totals"]["percent"], 60.0);

    let sites = report["sites"].as_array().expect("sites array");
    assert_eq!(sites.len(), 5);
    // The never-called function and the untaken ELSE body stay at zero.
    assert_eq!(sites[1]["count"], 0, "the uncovered function: {report}");
    assert_eq!(sites[4]["count"], 0, "the uncovered branch body: {report}");
    assert_eq!(sites[0]["count"], 1);
    assert_eq!(sites[2]["count"], 1);
    assert_eq!(sites[3]["count"], 1);

    let tests = report["tests"].as_array().expect("tests array");
    let names: Vec<&str> = tests
        .iter()
        .map(|test| test["name"].as_str().expect("test name"))
        .collect();
    assert_eq!(names, ["AddWorks", "ClassifyPositive", "ExcludedHelper"]);
    let add = &tests[0];
    let classify = &tests[1];
    let helper = &tests[2];
    assert_eq!(covered_indices(add), [0]);
    assert_eq!(covered_indices(classify), [2, 3]);
    assert_eq!(covered_indices(helper), Vec::<u64>::new());
    assert_eq!(helper["sites_covered"], 0);
    let union: BTreeSet<u64> = covered_indices(add)
        .into_iter()
        .chain(covered_indices(classify))
        .collect();
    assert_eq!(union, BTreeSet::from([0, 2, 3]), "the suite union");
}

#[test]
fn reruns_are_byte_identical_and_free_of_host_data() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("determinism");
    let example = copy_example(&directory, "example");
    let output = run_example(&example, &["--coverage"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let json_path = example.join("target/stc/coverage/coverage.json");
    let info_path = example.join("target/stc/coverage/lcov.info");
    let json_first = std::fs::read(&json_path).expect("read coverage.json");
    let info_first = std::fs::read(&info_path).expect("read lcov.info");

    let output = run_example(&example, &["--coverage"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let json_second = std::fs::read(&json_path).expect("read coverage.json");
    let info_second = std::fs::read(&info_path).expect("read lcov.info");
    assert_eq!(
        json_first, json_second,
        "coverage.json must be byte-identical"
    );
    assert_eq!(info_first, info_second, "lcov.info must be byte-identical");

    let temp = std::env::temp_dir().to_string_lossy().into_owned();
    let example_text = example.to_string_lossy().into_owned();
    let canonical = example
        .canonicalize()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    for (name, bytes) in [("coverage.json", &json_first), ("lcov.info", &info_first)] {
        let text = std::str::from_utf8(bytes).expect("report is UTF-8");
        assert!(!text.contains('\r'), "{name} must use LF line endings");
        for marker in [
            "duration",
            "timestamp",
            "build_id",
            "build-id",
            "elapsed",
            "hostname",
        ] {
            assert!(
                !text.contains(marker),
                "{name} contains host or temporal data `{marker}`:\n{text}"
            );
        }
        assert!(
            !text.contains(&temp),
            "{name} contains the temporary directory"
        );
        assert!(
            !text.contains(&example_text),
            "{name} contains an absolute path"
        );
        assert!(
            canonical.is_empty() || !text.contains(&canonical),
            "{name} contains a canonical absolute path"
        );
    }

    // File labels are package-relative with forward slashes on every host.
    let report: Value =
        serde_json::from_str(std::str::from_utf8(&json_first).expect("coverage.json is UTF-8"))
            .expect("coverage.json parses");
    for site in report["sites"].as_array().expect("sites array") {
        let file = site["file"].as_str().expect("site file");
        assert_eq!(file, "src/coverage.st", "unexpected report label `{file}`");
        assert!(!file.contains('\\') && !file.contains(':'));
        assert!(!file.starts_with('/'));
    }
}

fn without_duration(results: &[Value]) -> Vec<Value> {
    results
        .iter()
        .map(|result| {
            let mut result = result.clone();
            let object = result.as_object_mut().expect("result object");
            object.remove("duration_ms");
            result
        })
        .collect()
}

fn split_json_lines(text: &str) -> (Vec<Value>, Vec<Value>) {
    let mut host_lines = Vec::new();
    let mut results = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("non-JSON output line `{line}`: {error}"));
        if value.get("kind").and_then(Value::as_str) == Some("stc-test-result") {
            host_lines.push(value);
        } else {
            results.push(value);
        }
    }
    (host_lines, results)
}

fn run_json_nocapture(directory: &Path, coverage: bool) -> Output {
    let mut command = stc_test(directory);
    command.args(["--message-format", "json"]);
    if coverage {
        command.arg("--coverage");
    }
    command.args(["--", "--nocapture"]);
    command.output().expect("run stc test")
}

#[test]
fn instrumented_tests_keep_result_semantics() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("equivalence");
    directory.write(
        "stc.toml",
        "schema = 1\n\n\
         [package]\n\
         name = \"equivalence\"\n\
         version = \"0.1.0\"\n\
         language = \"stc-2013-oop-utf8-v1\"\n\n\
         [project]\n\
         sources = [\"src/**/*.st\"]\n\n\
         [test]\n\
         sources = [\"tests/**/*.st\"]\n\n\
         [[test.case]]\n\
         name = \"ExpectedTrap\"\n\
         expect-trap = \"STC_TRAP_DIV_ZERO\"\n",
    );
    directory.write(
        "src/main.st",
        "FUNCTION Divide : DINT\n\
         VAR_INPUT\n\
           dividend : DINT;\n\
           divisor : DINT;\n\
         END_VAR\n\
           Divide := dividend / divisor;\n\
         END_FUNCTION\n",
    );
    directory.write(
        "tests/equiv.st",
        "{stc:test}\n\
         FUNCTION Passing : BOOL\n\
         VAR\n\
           result : DINT;\n\
         END_VAR\n\
           result := Divide(6, 2);\n\
           STC_ASSERT(result = 3, \"6 / 2 should be 3\");\n\
           Passing := TRUE;\n\
         END_FUNCTION\n\
         \n\
         {stc:test}\n\
         FUNCTION Failing : BOOL\n\
         VAR\n\
           result : DINT;\n\
         END_VAR\n\
           result := Divide(1, 1);\n\
           STC_ASSERT(result = 2, \"1 / 1 should be 2\");\n\
           Failing := TRUE;\n\
         END_FUNCTION\n\
         \n\
         {stc:test}\n\
         FUNCTION ExpectedTrap : BOOL\n\
         VAR\n\
           result : DINT;\n\
           zero : DINT;\n\
         END_VAR\n\
           zero := 0;\n\
           result := Divide(1, zero);\n\
           ExpectedTrap := TRUE;\n\
         END_FUNCTION\n\
         \n\
         {stc:test}\n\
         FUNCTION UnexpectedTrap : BOOL\n\
         VAR\n\
           result : DINT;\n\
           zero : DINT;\n\
         END_VAR\n\
           zero := 0;\n\
           result := Divide(1, zero);\n\
           UnexpectedTrap := TRUE;\n\
         END_FUNCTION\n",
    );

    let plain = run_json_nocapture(&directory.path, false);
    let instrumented = run_json_nocapture(&directory.path, true);
    assert_eq!(
        code(&plain),
        4,
        "an unexpected trap exits 4 without coverage:\n{}\n{}",
        stdout(&plain),
        stderr(&plain)
    );
    assert_eq!(
        code(&instrumented),
        4,
        "an unexpected trap exits 4 with coverage:\n{}\n{}",
        stdout(&instrumented),
        stderr(&instrumented)
    );

    let (plain_host, plain_results) = split_json_lines(&stdout(&plain));
    let (instrumented_host, instrumented_results) = split_json_lines(&stdout(&instrumented));
    assert_eq!(plain_results.len(), 4, "{:?}", plain_results);
    assert_eq!(instrumented_results.len(), 4, "{instrumented_results:?}");
    // The host result lines carry statuses, failures, traps, and per-test
    // cycles; `-- --nocapture` exposes them before the CLI result object.
    assert_eq!(
        plain_host, instrumented_host,
        "test-host result lines must match with and without coverage"
    );
    assert_eq!(
        without_duration(&plain_results),
        without_duration(&instrumented_results),
        "CLI result objects must match with and without coverage apart from duration_ms"
    );

    let statuses: Vec<(&str, &str)> = instrumented_results
        .iter()
        .map(|result| {
            (
                result["name"].as_str().expect("result name"),
                result["status"].as_str().expect("result status"),
            )
        })
        .collect();
    assert_eq!(
        statuses,
        [
            ("ExpectedTrap", "pass"),
            ("Failing", "fail"),
            ("Passing", "pass"),
            ("UnexpectedTrap", "trap"),
        ]
    );
    assert!(
        instrumented_results
            .iter()
            .all(|result| result.get("coverage").is_none()),
        "coverage never changes the stable result object: {instrumented_results:?}"
    );

    // Failed and trapped tests still contribute their executed statements.
    let report: Value = serde_json::from_str(&read_text(
        &directory.path.join("target/stc/coverage/coverage.json"),
    ))
    .expect("coverage.json parses");
    let tests = report["tests"].as_array().expect("tests array");
    assert_eq!(tests.len(), 4);
    for test in tests {
        assert_eq!(test["coverage"], "reported", "{test}");
        assert_eq!(covered_indices(test), [0], "{test}");
    }
    let trapped = tests
        .iter()
        .find(|test| test["name"] == "UnexpectedTrap")
        .expect("the trapped test");
    assert_eq!(trapped["status"], "trap");
}

#[test]
fn production_builds_and_plain_tests_emit_no_instrumentation() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("isolation");
    directory.write(
        "stc.toml",
        "schema = 1\n\n\
         [package]\n\
         name = \"isolation\"\n\
         version = \"0.1.0\"\n\
         language = \"stc-2013-oop-utf8-v1\"\n\n\
         [project]\n\
         sources = [\"src/**/*.st\"]\n\
         configuration = \"Isolation\"\n\n\
         [build]\n\
         emit = [\"module\", \"metadata\"]\n\n\
         [test]\n\
         sources = [\"tests/**/*.st\"]\n",
    );
    directory.write(
        "src/main.st",
        "PROGRAM Main\n\
         VAR\n\
           count : DINT;\n\
         END_VAR\n\
           count := count + 1;\n\
         END_PROGRAM\n\
         \n\
         CONFIGURATION Isolation\n\
         RESOURCE Cpu ON Cpu\n\
           TASK MainTask(INTERVAL := T#10ms, PRIORITY := 1);\n\
           PROGRAM main WITH MainTask : Main;\n\
         END_RESOURCE\n\
         END_CONFIGURATION\n",
    );
    directory.write(
        "tests/isolation.st",
        "{stc:test}\n\
         FUNCTION CountsOnce : BOOL\n\
         VAR\n\
           c : DINT;\n\
         END_VAR\n\
           c := 1 + 1;\n\
           STC_ASSERT(c = 2, \"arithmetic\");\n\
           CountsOnce := TRUE;\n\
         END_FUNCTION\n",
    );

    for build_args in [&["build"][..], &["build", "--release"][..]] {
        let output = directory.run(build_args);
        assert_success(&output, "stc build for production isolation");
    }
    let mut artifacts = Vec::new();
    for mode in ["debug", "release"] {
        artifacts.extend(files_under(&directory.path.join("target/stc").join(mode)));
    }
    assert!(!artifacts.is_empty(), "stc build wrote no artifacts");
    for path in &artifacts {
        assert!(
            !contains_symbol(path),
            "production artifact {} contains the coverage counter symbol",
            path.display()
        );
    }
    assert_no_coverage_outputs(&directory.path);

    // `--coverage` is a test-only option; production builds reject it.
    let rejected = directory.run(&["build", "--coverage"]);
    assert_eq!(code(&rejected), 2, "{}", stderr(&rejected));
    assert!(
        stderr(&rejected).contains("--coverage") && stderr(&rejected).contains("stc test"),
        "{}",
        stderr(&rejected)
    );
    assert!(
        !directory.path.join("target/stc/coverage").exists(),
        "a rejected build must not write coverage output"
    );

    // Uninstrumented `stc test` builds no counter global into the module and
    // writes no reports or sidecars.
    let output = stc_test(&directory.path)
        .arg("CountsOnce")
        .output()
        .expect("run stc test");
    assert_success(&output, "uninstrumented stc test");
    assert!(
        !stdout(&output).contains("coverage:"),
        "{}",
        stdout(&output)
    );
    let test_files = files_under(&directory.path.join("target/stc/test"));
    assert!(!test_files.is_empty(), "stc test wrote no artifacts");
    for path in &test_files {
        if is_test_host_executable(path) {
            continue;
        }
        assert!(
            !contains_symbol(path),
            "uninstrumented test module artifact {} contains the counter symbol",
            path.display()
        );
    }
    assert_no_coverage_outputs(&directory.path);
}

#[test]
fn test_code_and_test_only_pous_are_excluded() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("exclusions");
    let example = copy_example(&directory, "example");
    let output = run_example(&example, &["--coverage"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let report: Value = serde_json::from_str(&read_text(
        &example.join("target/stc/coverage/coverage.json"),
    ))
    .expect("coverage.json parses");
    let files: Vec<&str> = report["files"]
        .as_array()
        .expect("files array")
        .iter()
        .map(|file| file["file"].as_str().expect("file label"))
        .collect();
    assert_eq!(
        files,
        ["src/coverage.st"],
        "only production statements are reportable: {report}"
    );
    for site in report["sites"].as_array().expect("sites array") {
        let file = site["file"].as_str().expect("site file");
        assert!(
            !file.starts_with("tests/"),
            "a [test].sources statement was reported: {site}"
        );
        assert!(
            !file.contains("stc-test-") && !file.contains("wrapper"),
            "a synthetic wrapper statement was reported: {site}"
        );
    }

    // The `{stc:test}` POU lives in the production file, is discovered and
    // run as a test, and is still excluded by definition, not by file.
    let source = read_text(&example.join("src/coverage.st"));
    let production_line = find_line(&source, "AddValues := a + b;");
    let helper_line = find_line(&source, "doubled := 21 * 2;");
    let helper_result_line = find_line(&source, "ExcludedHelper := doubled = 42;");
    let reported_lines: BTreeSet<u64> = report["sites"]
        .as_array()
        .expect("sites array")
        .iter()
        .map(|site| site["line"].as_u64().expect("site line"))
        .collect();
    assert!(
        reported_lines.contains(&production_line),
        "the neighboring production statement at line {production_line} is missing: {report}"
    );
    assert!(
        !reported_lines.contains(&helper_line),
        "the test-only helper statement at line {helper_line} was reported: {report}"
    );
    assert!(
        !reported_lines.contains(&helper_result_line),
        "the test-only helper result at line {helper_result_line} was reported: {report}"
    );
    let helper = report["tests"]
        .as_array()
        .expect("tests array")
        .iter()
        .find(|test| test["name"] == "ExcludedHelper")
        .expect("the test-only helper runs as a test");
    assert_eq!(helper["status"], "pass");
    assert_eq!(covered_indices(helper), Vec::<u64>::new());
    assert_eq!(helper["sites_covered"], 0);
}

#[test]
fn dependency_packages_use_package_qualified_labels() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("dependency");
    directory.write(
        "motor/stc.toml",
        "schema = 1\n\n\
         [package]\n\
         name = \"motor-control\"\n\
         version = \"0.1.0\"\n\
         language = \"stc-2013-oop-utf8-v1\"\n\n\
         [project]\n\
         sources = [\"src/**/*.st\"]\n\n\
         [test]\n\
         sources = [\"tests/**/*.st\"]\n",
    );
    directory.write(
        "motor/src/motor.st",
        "NAMESPACE Motor\n\
         FUNCTION AddNumbers : DINT\n\
         VAR_INPUT\n\
           a : DINT;\n\
           b : DINT;\n\
         END_VAR\n\
           AddNumbers := a + b;\n\
         END_FUNCTION\n\
         END_NAMESPACE\n",
    );
    // The dependency has its own test source set, which the test project must
    // not discover.
    directory.write(
        "motor/tests/ignored.st",
        "{stc:test}\n\
         FUNCTION DependencyTest : BOOL\n\
           DependencyTest := TRUE;\n\
         END_FUNCTION\n",
    );
    directory.write(
        "tests-project/stc.toml",
        "schema = 1\n\n\
         [package]\n\
         name = \"motor-tests\"\n\
         version = \"0.1.0\"\n\
         language = \"stc-2013-oop-utf8-v1\"\n\n\
         [project]\n\
         sources = [\"tests/**/*.st\"]\n\n\
         [dependencies]\n\
         motor = { path = \"../motor\", package = \"motor-control\" }\n",
    );
    directory.write(
        "tests-project/tests/motor.st",
        "{stc:test}\n\
         FUNCTION MotorAdds : BOOL\n\
         VAR\n\
           r : DINT;\n\
         END_VAR\n\
           r := motor.Motor.AddNumbers(2, 3);\n\
           STC_ASSERT(r = 5, \"dependency function\");\n\
           MotorAdds := TRUE;\n\
         END_FUNCTION\n",
    );

    let project = directory.path("tests-project");
    let output = stc_test(&project)
        .arg("--coverage")
        .output()
        .expect("run stc test");
    assert_success(&output, "dependency coverage run");
    assert!(
        !stdout(&output).contains("DependencyTest"),
        "the dependency's test source was discovered: {}",
        stdout(&output)
    );

    let report: Value = serde_json::from_str(&read_text(
        &project.join("target/stc/coverage/coverage.json"),
    ))
    .expect("coverage.json parses");
    assert_eq!(report["package"], "motor-tests");
    let files = report["files"].as_array().expect("files array");
    assert_eq!(files.len(), 1, "{report}");
    assert_eq!(
        files[0]["file"], "motor-control/src/motor.st",
        "dependency files are package-qualified: {report}"
    );
    assert_eq!(files[0]["sites"], 1);
    assert_eq!(files[0]["covered"], 1);
    assert_eq!(files[0]["percent"], 100.0);
    let sites = report["sites"].as_array().expect("sites array");
    assert_eq!(sites.len(), 1);
    assert_eq!(
        sites[0]["line"],
        find_line(
            &read_text(&directory.path("motor/src/motor.st")),
            "AddNumbers := a + b;"
        )
    );
    assert_eq!(sites[0]["count"], 1);
    let label = sites[0]["file"].as_str().expect("site file");
    assert!(!label.contains('\\') && !label.contains(':'), "{label}");
}

#[test]
fn optimized_test_builds_report_identical_coverage() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("opt-level");
    let baseline = copy_example(&directory, "o0");
    let optimized = copy_example(&directory, "o3");
    let manifest = read_text(&optimized.join("stc.toml"));
    std::fs::write(
        optimized.join("stc.toml"),
        format!("{manifest}\n[build]\nopt-level = 3\n"),
    )
    .expect("write optimized manifest");

    let baseline_output = run_example(&baseline, &["--coverage"]);
    let optimized_output = run_example(&optimized, &["--coverage"]);
    assert_eq!(code(&baseline_output), 0, "{}", stderr(&baseline_output));
    assert_eq!(code(&optimized_output), 0, "{}", stderr(&optimized_output));

    let baseline_json = read_text(&baseline.join("target/stc/coverage/coverage.json"));
    let optimized_json = read_text(&optimized.join("target/stc/coverage/coverage.json"));
    assert_eq!(
        normalized(&baseline_json),
        normalized(&optimized_json),
        "the instrumentation must survive optimization without changing the report"
    );
    let baseline_info = read_text(&baseline.join("target/stc/coverage/lcov.info"));
    let optimized_info = read_text(&optimized.join("target/stc/coverage/lcov.info"));
    assert_eq!(normalized(&baseline_info), normalized(&optimized_info));
    assert_eq!(
        normalized(&stdout(&baseline_output)),
        normalized(&stdout(&optimized_output))
    );
}
