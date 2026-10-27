//! Conformance corpus harness (M6 phase 10).
//!
//! Every `tests/conformance/<area>/<case>.meta.toml` describes one case:
//!
//! ```toml
//! profile = "stc-2013-oop-utf8-v1"
//! kind = "run-pass"      # compile-pass | compile-fail | run-pass | run-fail | trap
//! cycles = 1
//! expected_exit = 0
//! expected_stdout = ""
//! expected_trap = "STC_TRAP_OVERFLOW"
//! expected_stderr_contains = ["STC-E-..."]
//! rules = ["STC-..."]
//! ```
//!
//! A single-file case is `<case>.st` next to the metadata. A directory case
//! has an `stc.toml` next to the metadata and runs through the project CLI.
//! `compile-pass` builds textual IR, `compile-fail` runs `stc check` (or a
//! build when `compile = "build"`), and run kinds build an executable and run
//! it with `--cycles`. Run kinds need clang; without it they are skipped
//! unless `STC_REQUIRE_CLANG=1`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use toml_edit::{DocumentMut, Item};

const PROFILE: &str = "stc-2013-oop-utf8-v1";

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

fn corpus_dir() -> PathBuf {
    workspace_root().join("tests/conformance")
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
                    eprintln!("note: clang not found; skipping conformance run cases");
                    None
                }
            }
        })
        .as_ref()
        .copied()
}

fn runtime_artifact() -> PathBuf {
    static CACHE: OnceLock<PathBuf> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
            let output = Command::new(cargo)
                .args(["build", "-p", "stc-runtime", "--locked"])
                .current_dir(workspace_root())
                .output()
                .expect("run cargo build -p stc-runtime");
            assert!(
                output.status.success(),
                "failed to build stc-runtime:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let profile = std::env::current_exe()
                .expect("test executable")
                .parent()
                .and_then(Path::parent)
                .expect("target profile directory")
                .to_path_buf();
            let name = if cfg!(windows) {
                "stc_runtime.lib"
            } else {
                "libstc_runtime.a"
            };
            let artifact = profile.join(name);
            assert!(
                artifact.is_file(),
                "runtime artifact missing at {}",
                artifact.display()
            );
            artifact
        })
        .clone()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    CompilePass,
    CompileFail,
    RunPass,
    RunFail,
    Trap,
}

impl Kind {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "compile-pass" => Ok(Self::CompilePass),
            "compile-fail" => Ok(Self::CompileFail),
            "run-pass" => Ok(Self::RunPass),
            "run-fail" => Ok(Self::RunFail),
            "trap" => Ok(Self::Trap),
            other => Err(format!(
                "unknown kind `{other}` (expected compile-pass, compile-fail, run-pass, \
                 run-fail, or trap)"
            )),
        }
    }
}

#[derive(Debug)]
struct Case {
    name: String,
    meta_path: PathBuf,
    source: PathBuf,
    project: Option<PathBuf>,
    kind: Kind,
    compile: bool,
    cycles: u64,
    runs: u64,
    state_file: bool,
    corrupt_state: bool,
    cycle_time: Option<String>,
    mock_clock: bool,
    relaxed: bool,
    locked: bool,
    expected_exit: Option<i32>,
    expected_stdout: Option<String>,
    expected_trap: Option<String>,
    expected_stderr_contains: Vec<String>,
    extra_args: Vec<String>,
    rules: Vec<String>,
}

fn parse_case(meta_path: &Path) -> Result<Case, String> {
    let text = std::fs::read_to_string(meta_path)
        .map_err(|error| format!("cannot read metadata: {error}"))?;
    let document: DocumentMut = text
        .parse()
        .map_err(|error| format!("cannot parse metadata: {error}"))?;
    let profile = string(&document, "profile")?;
    if profile != PROFILE {
        return Err(format!("profile `{profile}` does not match `{PROFILE}`"));
    }
    let kind = Kind::parse(&string(&document, "kind")?)?;
    let directory = meta_path.parent().expect("metadata parent");
    let stem = meta_path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".meta.toml"))
        .ok_or_else(|| "metadata file name must end in `.meta.toml`".to_owned())?;
    let project_manifest = directory.join("stc.toml");
    let (source, project) = if project_manifest.is_file() {
        (project_manifest.clone(), Some(project_manifest.clone()))
    } else {
        (directory.join(format!("{stem}.st")), None)
    };
    if !source.is_file() {
        return Err(format!(
            "source `{}` does not exist (and neither does `{}`)",
            source.display(),
            project_manifest.display()
        ));
    }
    let rules = array(&document, "rules")?;
    if rules.is_empty() {
        return Err("`rules` must list at least one rule ID".into());
    }
    for rule in &rules {
        if !rule.starts_with("STC-")
            || !rule
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(format!("`{rule}` is not an `STC-*` rule ID"));
        }
    }
    let compile = match document.get("compile").and_then(Item::as_str) {
        None => false,
        Some("check") => false,
        Some("build") => true,
        Some(other) => {
            return Err(format!(
                "unknown `compile` value `{other}` (check or build)"
            ));
        }
    };
    Ok(Case {
        name: stem.to_owned(),
        meta_path: meta_path.to_path_buf(),
        source,
        project,
        kind,
        compile,
        cycles: integer(&document, "cycles")?.unwrap_or(1),
        runs: integer(&document, "runs")?.unwrap_or(1),
        state_file: boolean(&document, "state_file")?.unwrap_or(false),
        corrupt_state: boolean(&document, "corrupt_state")?.unwrap_or(false),
        cycle_time: document
            .get("cycle_time")
            .and_then(Item::as_str)
            .map(str::to_owned),
        mock_clock: boolean(&document, "mock_clock")?.unwrap_or(false),
        relaxed: boolean(&document, "relaxed")?.unwrap_or(false),
        locked: boolean(&document, "locked")?.unwrap_or(false),
        expected_exit: integer(&document, "expected_exit")?
            .map(|value| i32::try_from(value).unwrap_or(i32::MAX)),
        expected_stdout: document
            .get("expected_stdout")
            .and_then(Item::as_str)
            .map(str::to_owned),
        expected_trap: document
            .get("expected_trap")
            .and_then(Item::as_str)
            .map(str::to_owned),
        expected_stderr_contains: array(&document, "expected_stderr_contains")?,
        extra_args: array(&document, "args")?,
        rules,
    })
}

fn string(document: &DocumentMut, key: &str) -> Result<String, String> {
    document
        .get(key)
        .and_then(Item::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("missing string field `{key}`"))
}

fn integer(document: &DocumentMut, key: &str) -> Result<Option<u64>, String> {
    document
        .get(key)
        .map(|item| {
            item.as_integer()
                .and_then(|value| u64::try_from(value).ok())
                .ok_or_else(|| format!("`{key}` must be a non-negative integer"))
        })
        .transpose()
}

fn boolean(document: &DocumentMut, key: &str) -> Result<Option<bool>, String> {
    document
        .get(key)
        .map(|item| {
            item.as_bool()
                .ok_or_else(|| format!("`{key}` must be a boolean"))
        })
        .transpose()
}

fn array(document: &DocumentMut, key: &str) -> Result<Vec<String>, String> {
    let Some(item) = document.get(key) else {
        return Ok(Vec::new());
    };
    let array = item
        .as_array()
        .ok_or_else(|| format!("`{key}` must be an array of strings"))?;
    array
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("`{key}` must contain only strings"))
        })
        .collect()
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(case: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let safe: String = case
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                    character
                } else {
                    '-'
                }
            })
            .collect();
        let path = std::env::temp_dir().join(format!(
            "stc-conformance-{}-{safe}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp directory");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn conformance_corpus_passes() {
    let directory = corpus_dir();
    let mut cases = Vec::new();
    collect_meta(&directory, &mut cases);
    cases.sort();
    assert!(
        !cases.is_empty(),
        "no conformance cases in {}",
        directory.display()
    );

    // Fail on metadata errors before running anything.
    let parsed: Vec<Case> = cases
        .iter()
        .map(|path| parse_case(path).unwrap_or_else(|error| panic!("{}: {error}", path.display())))
        .collect();

    let mut failures = Vec::new();
    for case in &parsed {
        match run_case(case) {
            Ok(Some(note)) => println!("skip {} ({note})", case.name),
            Ok(None) => println!("ok {}", case.name),
            Err(message) => failures.push(format!(
                "{} (rules: {}):\n{message}",
                case.name,
                case.rules.join(", ")
            )),
        }
    }
    if !failures.is_empty() {
        panic!(
            "{} conformance case(s) failed:\n\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}

fn collect_meta(directory: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_meta(&path, out);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".meta.toml"))
        {
            out.push(path);
        }
    }
}

/// Returns `Ok(Some(note))` when a case was skipped (no clang).
fn run_case(case: &Case) -> Result<Option<&'static str>, String> {
    match case.kind {
        Kind::CompilePass => {
            let output = compile(case)?;
            expect_success(case, &output, "compile-pass")?;
            Ok(None)
        }
        Kind::CompileFail => {
            let output = if case.compile {
                compile(case)?
            } else {
                check(case)?
            };
            let expected = case.expected_exit.unwrap_or(1);
            expect_exit_contains(case, &output, expected)?;
            Ok(None)
        }
        Kind::RunPass | Kind::RunFail | Kind::Trap => {
            if clang().is_none() {
                return Ok(Some("clang unavailable"));
            }
            let directory = TempDir::new(&case.name);
            let executable = build_executable(case, &directory)?;
            if case.corrupt_state {
                std::fs::write(directory.path.join("state.bin"), b"garbage")
                    .map_err(|error| format!("cannot write corrupt state: {error}"))?;
            }
            let expected = match case.kind {
                Kind::RunPass => case.expected_exit.unwrap_or(0),
                Kind::RunFail => case
                    .expected_exit
                    .ok_or_else(|| "run-fail requires `expected_exit`".to_owned())?,
                Kind::Trap => case.expected_exit.unwrap_or(4),
                _ => unreachable!(),
            };
            for run in 0..case.runs {
                let output = run_executable(case, &directory, &executable)?;
                expect_exit_contains(case, &output, expected)?;
                if let Some(stdout) = &case.expected_stdout {
                    let actual = normalize(&String::from_utf8_lossy(&output.stdout));
                    if &actual != stdout {
                        return Err(format!(
                            "run {}: stdout mismatch\n--- expected ---\n{stdout}\n--- actual ---\n{actual}",
                            run + 1
                        ));
                    }
                }
                if let Some(trap) = &case.expected_trap {
                    let name = trap.strip_prefix("STC_TRAP_").unwrap_or(trap);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if !stderr.contains(name) {
                        return Err(format!(
                            "run {}: expected trap `{trap}` in stderr:\n{stderr}",
                            run + 1
                        ));
                    }
                }
            }
            Ok(None)
        }
    }
}

fn compile(case: &Case) -> Result<Output, String> {
    if let Some(manifest) = &case.project {
        let directory = copy_project(case, manifest)?;
        let manifest = directory.path.join("stc.toml");
        Command::new(stc())
            .args(["build", "--manifest-path"])
            .arg(&manifest)
            .args(["--emit", "ir", "--color", "never"])
            .args(case_args(case))
            .output()
            .map_err(|error| format!("cannot run stc: {error}"))
    } else {
        let directory = TempDir::new(&case.name);
        Command::new(stc())
            .arg("build")
            .arg(&case.source)
            .args(["--emit", "ir", "--name", &case.name, "--out-dir"])
            .arg(&directory.path)
            .args(["--color", "never"])
            .args(case_args(case))
            .output()
            .map_err(|error| format!("cannot run stc: {error}"))
    }
}

fn check(case: &Case) -> Result<Output, String> {
    if let Some(manifest) = &case.project {
        let directory = copy_project(case, manifest)?;
        let manifest = directory.path.join("stc.toml");
        Command::new(stc())
            .args(["check", "--manifest-path"])
            .arg(&manifest)
            .args(["--color", "never"])
            .args(case_args(case))
            .output()
            .map_err(|error| format!("cannot run stc: {error}"))
    } else {
        Command::new(stc())
            .arg("check")
            .arg(&case.source)
            .args(["--color", "never"])
            .args(case_args(case))
            .output()
            .map_err(|error| format!("cannot run stc: {error}"))
    }
}

fn build_executable(case: &Case, directory: &TempDir) -> Result<PathBuf, String> {
    if let Some(manifest) = &case.project {
        let source = manifest.parent().expect("project root");
        let project = directory.path.join("project");
        copy_tree(source, &project)?;
        return Ok(project.join("stc.toml"));
    }
    let output = Command::new(stc())
        .arg("build")
        .arg(&case.source)
        .args(["--emit", "exe", "--name", &case.name, "--out-dir"])
        .arg(&directory.path)
        .args(["--runtime"])
        .arg(runtime_artifact())
        .args(["--color", "never"])
        .args(case_args(case))
        .output()
        .map_err(|error| format!("cannot run stc: {error}"))?;
    expect_success(case, &output, "build")?;
    Ok(directory.path.join(executable_name(&case.name)))
}

fn run_executable(case: &Case, directory: &TempDir, executable: &Path) -> Result<Output, String> {
    let project_directory = executable.parent().expect("manifest parent");
    let mut command = if case.project.is_some() {
        let mut command = Command::new(stc());
        command
            .args(["run", "--manifest-path"])
            .arg(executable)
            .arg("--runtime")
            .arg(runtime_artifact());
        command
    } else {
        Command::new(executable)
    };
    command
        .current_dir(if case.project.is_some() {
            project_directory
        } else {
            &directory.path
        })
        .arg("--cycles")
        .arg(case.cycles.to_string());
    if let Some(cycle_time) = &case.cycle_time {
        command.arg("--cycle-time").arg(cycle_time);
    }
    if case.mock_clock && case.project.is_none() {
        command.arg("--mock-clock").arg("0");
    }
    if case.state_file {
        command
            .arg("--state-file")
            .arg(directory.path.join("state.bin"));
    }
    command.args(&case.extra_args);
    command
        .output()
        .map_err(|error| format!("cannot run the case: {error}"))
}

fn case_args(case: &Case) -> Vec<String> {
    let mut args = Vec::new();
    if case.relaxed {
        args.push("--relaxed".into());
    }
    if case.locked {
        args.push("--locked".into());
    }
    args.extend(case.extra_args.iter().cloned());
    args
}

fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

fn copy_project(case: &Case, manifest: &Path) -> Result<TempDir, String> {
    let directory = TempDir::new(&case.name);
    let source = manifest.parent().expect("project root");
    copy_tree(source, &directory.path)?;
    Ok(directory)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in std::fs::read_dir(source)
        .map_err(|error| format!("cannot read `{}`: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        if name == "target" {
            continue;
        }
        let from = entry.path();
        let to = destination.join(&name);
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)
                .map_err(|error| format!("cannot copy `{}`: {error}", from.display()))?;
        }
    }
    Ok(())
}

fn expect_success(case: &Case, output: &Output, phase: &str) -> Result<(), String> {
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "{phase} failed for {} ({})\nstdout:\n{}\nstderr:\n{}",
        case.meta_path.display(),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn expect_exit_contains(case: &Case, output: &Output, expected: i32) -> Result<(), String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.code() != Some(expected) {
        return Err(format!(
            "expected exit {expected}, got {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status.code()
        ));
    }
    for needle in &case.expected_stderr_contains {
        if !stderr.contains(needle) {
            return Err(format!(
                "expected stderr to contain `{needle}`\nstdout:\n{stdout}\nstderr:\n{stderr}"
            ));
        }
    }
    Ok(())
}

fn normalize(text: &str) -> String {
    let mut text = text.replace("\r\n", "\n");
    while text.ends_with('\n') {
        text.pop();
    }
    text
}
