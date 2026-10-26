//! `stc test` integration tests (M6 phase 6): discovery, test-build mode,
//! process isolation, timeouts, traps, filters, JSON, and production
//! exclusion.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

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
                .ok()
                .as_deref()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .expect("target profile directory")
                .to_path_buf();
            let name = if cfg!(windows) {
                "stc_runtime.lib"
            } else {
                "libstc_runtime.a"
            };
            profile.join(name)
        })
        .clone()
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stc-test-phase6-{}-{name}-{unique}",
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

#[test]
fn the_scaffold_test_passes_end_to_end() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("scaffold");
    let project = directory.path.join("app");
    let created = directory.run(&["new", project.to_str().unwrap()]);
    assert_eq!(code(&created), 0, "{}", stderr(&created));
    let output = Command::new(stc())
        .args(["test", "--runtime"])
        .arg(runtime_artifact())
        .current_dir(&project)
        .output()
        .expect("run stc test");
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("test SmokeTest ... ok"), "{text}");
    assert!(text.contains("1 passed; 0 failed"), "{text}");
}

#[test]
fn function_fb_and_program_results_have_stable_reports() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("results");
    directory.manifest(
        "default-cycles = 3

[[test.case]]
name = \"SlowFb\"
cycles = 5
",
    );
    directory.write("src/main.st", "PROGRAM Main\n  ;\nEND_PROGRAM\n");
    directory.write(
        "tests/all.st",
        r#"
{stc:test}
FUNCTION AddWorks : BOOL
VAR
  r : DINT;
END_VAR
  r := 2 + 3;
  STC_ASSERT(r = 5, "2 + 3 should be 5");
  AddWorks := TRUE;
END_FUNCTION

{stc:test}
FUNCTION AddFails : BOOL
VAR
  r : DINT;
END_VAR
  r := 2 + 3;
  STC_ASSERT(r = 6, "2 + 3 should be 5");
  AddFails := r = 6;
END_FUNCTION

{stc:test}
FUNCTION_BLOCK PassingFb
VAR_OUTPUT
  PASSED : BOOL;
END_VAR
VAR
  n : DINT;
END_VAR
  n := n + 1;
  PASSED := n >= 3;
END_FUNCTION_BLOCK

{stc:test}
FUNCTION_BLOCK FailingFb
VAR_OUTPUT
  PASSED : BOOL;
END_VAR
  PASSED := FALSE;
END_FUNCTION_BLOCK

{stc:test}
FUNCTION_BLOCK SlowFb
VAR_OUTPUT
  PASSED : BOOL;
END_VAR
VAR
  n : DINT;
END_VAR
  n := n + 1;
  PASSED := n >= 5;
END_FUNCTION_BLOCK

{stc:test}
PROGRAM UnexpectedTrap
VAR
  a : DINT;
  b : DINT;
END_VAR
  a := 1;
  b := 0;
  a := a / b;
END_PROGRAM
"#,
    );
    let output = directory.run_test(&["AddWorks", "AddFails", "PassingFb", "SlowFb", "FailingFb"]);
    assert_eq!(code(&output), 1, "{}\n{}", stdout(&output), stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("test AddWorks ... ok"), "{text}");
    assert!(text.contains("test PassingFb ... ok"), "{text}");
    assert!(text.contains("test SlowFb ... ok"), "{text}");
    assert!(text.contains("test AddFails ... FAILED"), "{text}");
    assert!(text.contains("2 + 3 should be 5 (tests/all.st:"), "{text}");
    assert!(
        text.contains("PASSED was not TRUE at the end of the test"),
        "{text}"
    );
    assert!(text.contains("3 passed; 2 failed"), "{text}");

    let trapped = directory.run_test(&["UnexpectedTrap"]);
    assert_eq!(code(&trapped), 4, "{}", stderr(&trapped));
    let text = stdout(&trapped);
    assert!(text.contains("TRAPPED (STC_TRAP_DIV_ZERO)"), "{text}");

    let json = directory.run_test(&["AddFails", "--message-format", "json"]);
    assert_eq!(code(&json), 1);
    let text = stdout(&json);
    let line = text
        .lines()
        .find(|line| line.starts_with('{') && line.contains("\"name\":\"AddFails\""))
        .unwrap_or_else(|| panic!("no JSON result line in {text}"));
    assert!(line.contains("\"status\":\"fail\""), "{line}");
    assert!(line.contains("\"duration_ms\":"), "{line}");
    assert!(line.contains("\"file\":\"tests/all.st\""), "{line}");
    assert!(line.contains("\"message\":\"2 + 3 should be 5\""), "{line}");
    assert!(line.contains("\"trap\":null"), "{line}");
}

#[test]
fn cycles_clock_timers_and_io_prelude_work() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("clock");
    directory.manifest("");
    directory.write("src/main.st", "PROGRAM Main\n  ;\nEND_PROGRAM\n");
    directory.write(
        "tests/clock.st",
        r#"
{stc:test {"cycles": 25}}
PROGRAM TimerRuns
VAR
  t : TON;
  done : BOOL;
  count : DINT;
END_VAR
  count := count + 1;
  t(IN := TRUE, PT := T#10ms);
  done := t.Q;
  IF count = 25 THEN
    STC_ASSERT(done, "timer should complete");
  END_IF;
  STC_CLOCK_ADVANCE(T#1ms);
END_PROGRAM

{stc:test {"cycles": 2}}
PROGRAM IoWorks
VAR
  input AT %IB0 : BYTE;
  output AT %QB0 : BYTE;
END_VAR
  IF STC_IO_GET_OUTPUT(0) = 0 THEN
    STC_IO_SET_INPUT(0, 16#42);
  END_IF;
  output := input;
  STC_ASSERT(STC_IO_GET_OUTPUT(0) = 16#42, "output image");
END_PROGRAM
"#,
    );
    let output = directory.run_test(&[]);
    assert_eq!(code(&output), 0, "{}\n{}", stdout(&output), stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("test IoWorks ... ok"), "{text}");
    assert!(text.contains("test TimerRuns ... ok"), "{text}");
}

#[test]
fn expected_traps_pass_and_timeouts_fail() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("trap-timeout");
    directory.manifest(
        "\n[[test.case]]\nname = \"ExpectedTrap\"\nexpect-trap = \"STC_TRAP_DIV_ZERO\"\n",
    );
    directory.write("src/main.st", "PROGRAM Main\n  ;\nEND_PROGRAM\n");
    directory.write(
        "tests/traps.st",
        r#"
{stc:test}
PROGRAM ExpectedTrap
VAR
  a : DINT;
  b : DINT;
END_VAR
  a := 1;
  b := 0;
  a := a / b;
END_PROGRAM

{stc:test {"timeout": "150ms"}}
PROGRAM Hang
VAR
  i : DINT;
END_VAR
  WHILE TRUE DO
    i := i + 1;
  END_WHILE;
END_PROGRAM

{stc:test}
PROGRAM MismatchTrap
VAR
  a : DINT;
  b : DINT;
END_VAR
  a := 1;
  b := 0;
  a := a / b;
END_PROGRAM

{stc:test}
PROGRAM LoopForeverCli
VAR
  i : DINT;
END_VAR
  WHILE TRUE DO
    i := i + 1;
  END_WHILE;
END_PROGRAM
"#,
    );
    let expected = directory.run_test(&["ExpectedTrap"]);
    assert_eq!(
        code(&expected),
        0,
        "{}\n{}",
        stdout(&expected),
        stderr(&expected)
    );
    assert!(stdout(&expected).contains("test ExpectedTrap ... ok"));

    let timeout = directory.run_test(&["Hang"]);
    assert_eq!(code(&timeout), 1, "{}", stderr(&timeout));
    let text = stdout(&timeout);
    assert!(text.contains("test Hang ... TIMEOUT"), "{text}");
    assert!(text.contains("timed out"), "{text}");

    let cli_timeout = directory.run_test(&["LoopForeverCli", "--timeout", "150ms"]);
    assert_eq!(code(&cli_timeout), 1, "{}", stderr(&cli_timeout));
    assert!(stdout(&cli_timeout).contains("test LoopForeverCli ... TIMEOUT"));

    let mismatch = directory.run_test(&["MismatchTrap"]);
    assert_eq!(code(&mismatch), 4, "{}", stderr(&mismatch));
    assert!(stdout(&mismatch).contains("TRAPPED (STC_TRAP_DIV_ZERO)"));

    // `-- --nocapture` is accepted and keeps the structured result parseable.
    let runtime = runtime_artifact();
    let nocapture = Command::new(stc())
        .args(["test", "ExpectedTrap", "--runtime"])
        .arg(&runtime)
        .args(["--", "--nocapture"])
        .current_dir(&directory.path)
        .output()
        .expect("run stc test -- --nocapture");
    assert_eq!(
        code(&nocapture),
        0,
        "{}
{}",
        stdout(&nocapture),
        stderr(&nocapture)
    );
    assert!(stdout(&nocapture).contains("test ExpectedTrap ... ok"));
}

#[test]
fn list_filters_and_no_run() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("list");
    directory.manifest("");
    directory.write("src/main.st", "PROGRAM Main\n  ;\nEND_PROGRAM\n");
    directory.write(
        "tests/list.st",
        r#"
{stc:test}
FUNCTION AlphaWorks : BOOL
  AlphaWorks := TRUE;
END_FUNCTION

{stc:test}
FUNCTION BetaWorks : BOOL
  BetaWorks := TRUE;
END_FUNCTION
"#,
    );
    let list = directory.run_test(&["--list"]);
    assert_eq!(code(&list), 0, "{}", stderr(&list));
    assert_eq!(stdout(&list), "AlphaWorks\nBetaWorks\n");

    let filtered = directory.run_test(&["--list", "beta"]);
    assert_eq!(stdout(&filtered), "BetaWorks\n");

    let json = directory.run_test(&["--list", "--message-format", "json"]);
    let text = stdout(&json);
    assert!(text.contains("{\"name\":\"AlphaWorks\"}"), "{text}");
    assert!(text.contains("{\"name\":\"BetaWorks\"}"), "{text}");

    let no_run = directory.run_test(&["--no-run"]);
    assert_eq!(code(&no_run), 0, "{}", stderr(&no_run));
    assert!(
        !stdout(&no_run).contains("test AlphaWorks"),
        "{}",
        stdout(&no_run)
    );
    assert!(directory.path.join("target/stc/test").is_dir());
}

#[test]
fn invalid_test_declarations_are_diagnostics() {
    let directory = TestDir::new("invalid");
    directory.manifest("");
    directory.write("src/main.st", "PROGRAM Main\n  ;\nEND_PROGRAM\n");

    directory.write(
        "tests/bad.st",
        r#"
{stc:test}
FUNCTION Same : BOOL
  Same := TRUE;
END_FUNCTION

{stc:test}
FUNCTION same : BOOL
  same := TRUE;
END_FUNCTION
"#,
    );
    let duplicate = directory.run_test(&["--list"]);
    assert_eq!(code(&duplicate), 1);
    assert!(
        stderr(&duplicate).contains("STC-E-TEST-002"),
        "{}",
        stderr(&duplicate)
    );

    directory.write(
        "tests/bad.st",
        r#"
{stc:test}
FUNCTION WithInput : BOOL
VAR_INPUT
  x : DINT;
END_VAR
  WithInput := TRUE;
END_FUNCTION
"#,
    );
    let input = directory.run_test(&["--list"]);
    assert_eq!(code(&input), 1);
    assert!(
        stderr(&input).contains("STC-E-TEST-003"),
        "{}",
        stderr(&input)
    );

    directory.write(
        "tests/bad.st",
        r#"
{stc:test}
FUNCTION_BLOCK WrongPassed
VAR_OUTPUT
  PASSED : DINT;
END_VAR
END_FUNCTION_BLOCK
"#,
    );
    let passed = directory.run_test(&["--list"]);
    assert_eq!(code(&passed), 1);
    assert!(
        stderr(&passed).contains("STC-E-TEST-003"),
        "{}",
        stderr(&passed)
    );

    directory.write(
        "tests/bad.st",
        r#"
{stc:test}
FUNCTION Reserved : BOOL
  Reserved := TRUE;
END_FUNCTION

{stc:test}
FUNCTION STC_TEST_WRAPPER : BOOL
  STC_TEST_WRAPPER := TRUE;
END_FUNCTION
"#,
    );
    let reserved = directory.run_test(&["--list"]);
    assert_eq!(code(&reserved), 1);
    assert!(
        stderr(&reserved).contains("STC-E-TEST-003"),
        "{}",
        stderr(&reserved)
    );

    directory.write(
        "tests/bad.st",
        "{stc:test {\"cycles\": 2}}\nPROGRAM Conflicted\nEND_PROGRAM\n",
    );
    directory.manifest("\n[[test.case]]\nname = \"Conflicted\"\ncycles = 2\n");
    let conflict = directory.run_test(&["--list"]);
    assert_eq!(code(&conflict), 1);
    assert!(
        stderr(&conflict).contains("STC-E-PROJECT-008"),
        "{}",
        stderr(&conflict)
    );

    directory.manifest("\n[[test.case]]\nname = \"Missing\"\n");
    directory.write(
        "tests/bad.st",
        "{stc:test}\nFUNCTION Present : BOOL\n  Present := TRUE;\nEND_FUNCTION\n",
    );
    let unknown = directory.run_test(&["--list"]);
    assert_eq!(code(&unknown), 1);
    assert!(
        stderr(&unknown).contains("STC-E-PROJECT-008"),
        "{}",
        stderr(&unknown)
    );
}

#[test]
fn production_builds_exclude_test_units_and_assert_calls() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("exclusion");
    directory.manifest("");
    directory.write(
        "src/main.st",
        r#"
{stc:test}
FUNCTION HiddenTest : BOOL
  HiddenTest := TRUE;
END_FUNCTION

PROGRAM Main
  ;
END_PROGRAM
"#,
    );
    directory.write(
        "tests/smoke.st",
        "{stc:test}\nFUNCTION Smoke : BOOL\n  Smoke := TRUE;\nEND_FUNCTION\n",
    );
    let built = directory.run(&["build"]);
    assert_eq!(code(&built), 0, "{}", stderr(&built));
    let manifest =
        std::fs::read_to_string(directory.path.join("target/stc/debug/demo.stc-got.json"))
            .expect("manifest");
    assert!(!manifest.contains("HIDDENTEST"), "{manifest}");
    assert!(!manifest.contains("\"service\": 11"), "{manifest}");
    assert!(!manifest.contains("\"service\": 12"), "{manifest}");
    let ir = std::fs::read_to_string(directory.path.join("target/stc/debug/demo.ll")).expect("ir");
    assert!(!ir.contains("stc_rt_test_main"), "{ir}");

    directory.write(
        "src/assertprod.st",
        "PROGRAM AssertProd\n  STC_ASSERT(TRUE, \"no\");\nEND_PROGRAM\n",
    );
    let asserted = directory.run(&["build"]);
    assert_eq!(code(&asserted), 1);
    assert!(
        stderr(&asserted).contains("STC-E-ASSERT-001"),
        "{}",
        stderr(&asserted)
    );
    std::fs::remove_file(directory.path.join("src/assertprod.st")).unwrap();

    directory.write(
        "src/testref.st",
        "PROGRAM TestRef\nVAR\n  r : BOOL;\nEND_VAR\n  r := HiddenTest();\nEND_PROGRAM\n",
    );
    let reference = directory.run(&["check"]);
    assert_eq!(code(&reference), 1);
    assert!(
        stderr(&reference).contains("STC-E-TEST-001"),
        "{}",
        stderr(&reference)
    );
    std::fs::remove_file(directory.path.join("src/testref.st")).unwrap();

    directory.write("tests/broken.st", "FUNCTION broken : BOOL\n");
    let check = directory.run(&["check"]);
    assert_eq!(
        code(&check),
        0,
        "production must ignore test sources: {}",
        stderr(&check)
    );
    let test = directory.run_test(&["--list"]);
    assert_eq!(code(&test), 1);
    assert!(stderr(&test).contains("STC-E-"), "{}", stderr(&test));
}

#[test]
fn separate_test_project_discovers_only_its_own_tests() {
    if clang().is_none() {
        return;
    }
    let root = TestDir::new("separate");
    let library = root.path.join("motor");
    root.write(
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
    root.write(
        "motor/src/motor.st",
        "NAMESPACE Motor\n\
         FUNCTION Add : DINT\n\
         VAR_INPUT\n  a : DINT;\n  b : DINT;\nEND_VAR\n\
           Add := a + b;\n\
         END_FUNCTION\n\
         END_NAMESPACE\n",
    );
    // A dependency test that must not be discovered by the test project.
    root.write(
        "motor/tests/ignored.st",
        "{stc:test}\nFUNCTION DependencyTest : BOOL\n  DependencyTest := TRUE;\nEND_FUNCTION\n",
    );
    let tests = root.path.join("tests-project");
    root.write(
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
    root.write(
        "tests-project/tests/motor.st",
        "{stc:test}\n\
         FUNCTION MotorAdds : BOOL\n\
         VAR\n  r : DINT;\nEND_VAR\n\
           r := motor.Motor.Add(2, 3);\n\
           STC_ASSERT(r = 5, \"dependency function\");\n\
           MotorAdds := TRUE;\n\
         END_FUNCTION\n",
    );
    let output = Command::new(stc())
        .args(["test", "--runtime"])
        .arg(runtime_artifact())
        .current_dir(&tests)
        .output()
        .expect("run stc test");
    assert_eq!(code(&output), 0, "{}\n{}", stdout(&output), stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("test MotorAdds ... ok"), "{text}");
    assert!(!text.contains("DependencyTest"), "{text}");
    let _ = library;
}

#[test]
fn a_user_configuration_does_not_steer_test_builds() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("configuration");
    directory.write(
        "stc.toml",
        "schema = 1\n\n\
         [package]\n\
         name = \"demo\"\n\
         version = \"0.1.0\"\n\
         language = \"stc-2013-oop-utf8-v1\"\n\n\
         [project]\n\
         sources = [\"src/**/*.st\"]\n\
         configuration = \"UserCfg\"\n\n\
         [test]\n\
         sources = [\"tests/**/*.st\"]\n",
    );
    directory.write(
        "src/main.st",
        "PROGRAM Main\n  ;\nEND_PROGRAM\n\n\
         CONFIGURATION UserCfg\n\
         RESOURCE R ON Cpu\n\
           TASK Fast (INTERVAL := T#10ms, PRIORITY := 1);\n\
           PROGRAM inst WITH Fast : Main;\n\
         END_RESOURCE\n\
         END_CONFIGURATION\n",
    );
    directory.write(
        "tests/smoke.st",
        "{stc:test}\nFUNCTION Works : BOOL\n  Works := TRUE;\nEND_FUNCTION\n",
    );
    let output = directory.run_test(&["Works"]);
    assert_eq!(code(&output), 0, "{}\n{}", stdout(&output), stderr(&output));
    assert!(stdout(&output).contains("test Works ... ok"));
}

#[test]
fn quiet_and_empty_suites_are_clean() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("quiet");
    directory.manifest("");
    directory.write(
        "src/main.st",
        "PROGRAM Main
  ;
END_PROGRAM
",
    );
    directory.write(
        "tests/only.st",
        "FUNCTION NotATest : BOOL
  NotATest := TRUE;
END_FUNCTION
",
    );
    let empty = directory.run_test(&[]);
    assert_eq!(code(&empty), 0, "{}", stderr(&empty));
    assert!(stdout(&empty).is_empty(), "{}", stdout(&empty));

    directory.write(
        "tests/only.st",
        "{stc:test}
FUNCTION QuietPass : BOOL
  QuietPass := TRUE;
END_FUNCTION
",
    );
    let quiet = directory.run_test(&["QuietPass", "-q"]);
    assert_eq!(code(&quiet), 0, "{}", stderr(&quiet));
    assert!(stdout(&quiet).is_empty(), "{}", stdout(&quiet));
}

#[test]
fn test_option_validation_rejects_inapplicable_flags() {
    let directory = TestDir::new("validation");
    directory.manifest("");
    directory.write(
        "src/main.st",
        "PROGRAM Main
  ;
END_PROGRAM
",
    );
    directory.write(
        "tests/smoke.st",
        "{stc:test}
FUNCTION Smoke : BOOL
  Smoke := TRUE;
END_FUNCTION
",
    );
    assert_eq!(code(&directory.run(&["test", "--emit", "module"])), 2);
    assert_eq!(
        code(&directory.run(&["test", "--got-manifest", "baseline.bin"])),
        2
    );
    assert_eq!(code(&directory.run(&["test", "--cold"])), 2);
    assert_eq!(code(&directory.run(&["build", "--timeout", "1s"])), 2);
    assert_eq!(code(&directory.run(&["check", "--list"])), 2);
    assert_eq!(code(&directory.run(&["test", "--", "--unknown"])), 2);
}
