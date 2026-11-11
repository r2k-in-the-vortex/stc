//! `stc rename` parity tests for the shared `stc-ide` engine (WL-18 Phase 3).

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn stc() -> &'static str {
    env!("CARGO_BIN_EXE_stc")
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stc-rename-cli-{}-{name}-{unique}",
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
            std::fs::create_dir_all(parent).expect("create source directory");
        }
        std::fs::write(&path, contents).expect("write test file");
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.path(relative)).expect("read test file")
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(stc())
            .args(args)
            .current_dir(&self.path)
            .output()
            .expect("run stc")
    }

    fn build_ir(&self) -> Output {
        self.run(&["build", "--emit", "ir"])
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

fn manifest(name: &str, extra: &str) -> String {
    format!(
        "schema = 1\n\n[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\
         language = \"stc-2013-oop-utf8-v1\"\n\n[project]\nsources = [\"src/**/*.st\"]\n{extra}\n"
    )
}

const BASIC_SOURCE: &str = "\
PROGRAM Main
VAR
  Speed : DINT;
  text : STRING[32];
END_VAR
  Speed := Speed + 1;
  text := 'Speed';
END_PROGRAM
FUNCTION AddStep : DINT
VAR_INPUT
  value : DINT;
END_VAR
VAR
  local_value : DINT;
END_VAR
  AddStep := value + 1;
END_FUNCTION
";

const EXTERNAL_SOURCE: &str = "\
VAR_GLOBAL
  Short AT %QX0.0 : BOOL;
  Ns AT %IX0.1 : BOOL;
  Pkg AT %IX0.2 : BOOL;
  Other AT %IX0.3 : BOOL;
END_VAR

PROGRAM Main
VAR
  Field AT %QX0.1 : BOOL;
END_VAR
  Field := TRUE;
END_PROGRAM

CONFIGURATION MainConfig
RESOURCE Cpu ON Cpu
  TASK MainTask(INTERVAL := T#10ms, PRIORITY := 1);
  PROGRAM main WITH MainTask : Main;
END_RESOURCE
END_CONFIGURATION
";

const EXTERNAL_PROFILE: &str = "\
[[routes]]
source = { kind = \"plc-output\", variable = \"Short\" }
sinks = [{ kind = \"plc-input\", variable = \"Other\" }]
";

fn external_manifest() -> String {
    manifest(
        "app",
        "\
configuration = \"MainConfig\"

[io]
config = \"io/bench.toml\"

[[io.devices]]
name = \"bench\"
kind = \"demo\"

[[io.bindings]]
variable = \"Short\"
device = \"bench\"
channel = 0

[[io.bindings]]
variable = \"Ns\"
device = \"bench\"
channel = 1

[[io.bindings]]
variable = \"app.Pkg\"
device = \"bench\"
channel = 2

[[io.bindings]]
variable = \"main.Field\"
device = \"bench\"
channel = 3

[test]
sources = [\"tests/**/*.st\"]

[[test.case]]
name = \"SampleTest\"",
    )
}

fn write_external_project(project: &TestDir) {
    project.write("stc.toml", &external_manifest());
    project.write("io/bench.toml", EXTERNAL_PROFILE);
    project.write("src/main.st", EXTERNAL_SOURCE);
    project.write(
        "tests/sample_test.st",
        "{stc:test}\nFUNCTION SampleTest : BOOL\nVAR\n  result : BOOL;\nEND_VAR\n  result := TRUE;\n  SampleTest := result;\nEND_FUNCTION\n",
    );
}

#[test]
fn external_references_are_edited_with_canonical_history() {
    let project = TestDir::new("external");
    write_external_project(&project);
    let original_manifest = project.read("stc.toml");
    let original_source = project.read("src/main.st");
    let original_profile = project.read("io/bench.toml");

    let dry = project.run(&["rename", "Short", "Brief", "--dry-run"]);
    assert_eq!(dry.status.code(), Some(0), "{}", stderr(&dry));
    assert!(stdout(&dry).contains("stc.toml"), "{}", stdout(&dry));
    assert!(stdout(&dry).contains("bench.toml"), "{}", stdout(&dry));
    assert_eq!(project.read("src/main.st"), original_source);
    assert_eq!(project.read("stc.toml"), original_manifest);
    assert_eq!(project.read("io/bench.toml"), original_profile);
    assert!(!project.path("stc.renames.toml").exists());

    let rename = project.run(&["rename", "Short", "Brief"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    assert!(project.read("src/main.st").contains("Brief AT %QX0.0"));
    assert!(project.read("stc.toml").contains("variable = \"Brief\""));
    assert!(
        project
            .read("io/bench.toml")
            .contains("variable = \"Brief\"")
    );
    let history = project.read("stc.renames.toml");
    assert!(history.contains("old = \"Short\""), "{history}");
    assert!(history.contains("new = \"Brief\""), "{history}");

    let rename = project.run(&["rename", "Pkg", "Bundle"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    assert!(project.read("src/main.st").contains("Bundle AT %IX0.2"));
    let manifest = project.read("stc.toml");
    assert!(manifest.contains("variable = \"app.Bundle\""), "{manifest}");
    assert!(!manifest.contains("app.Pkg"), "{manifest}");
    let history = project.read("stc.renames.toml");
    assert!(history.contains("old = \"Pkg\""), "{history}");

    let rename = project.run(&["rename", "Ns", "Node"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    assert!(project.read("src/main.st").contains("Node AT %IX0.1"));
    assert!(project.read("stc.toml").contains("variable = \"Node\""));

    let rename = project.run(&["rename", "Other", "OtherNode"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    assert!(project.read("src/main.st").contains("OtherNode AT %IX0.3"));
    assert!(
        project
            .read("io/bench.toml")
            .contains("variable = \"OtherNode\"")
    );

    let rename = project.run(&["rename", "Main.Field", "Main.Out"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    assert!(project.read("src/main.st").contains("Out AT %QX0.1"));
    let manifest = project.read("stc.toml");
    assert!(manifest.contains("variable = \"main.Out\""), "{manifest}");
    assert!(!manifest.contains("main.Field"), "{manifest}");
    let history = project.read("stc.renames.toml");
    assert!(history.contains("old = \"Main.Field\""), "{history}");

    let rename = project.run(&["rename", "SampleTest", "SmokeTest"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    assert!(
        project
            .read("tests/sample_test.st")
            .contains("FUNCTION SmokeTest")
    );
    assert!(project.read("stc.toml").contains("name = \"SmokeTest\""));
    let history = project.read("stc.renames.toml");
    assert!(history.contains("old = \"SampleTest\""), "{history}");

    let rename = project.run(&["rename", "MainConfig", "MainConfigX"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    assert!(
        project
            .read("src/main.st")
            .contains("CONFIGURATION MainConfigX")
    );
    assert!(
        project
            .read("stc.toml")
            .contains("configuration = \"MainConfigX\"")
    );
    assert!(
        stderr(&rename)
            .contains("no history entry (configuration; the next online download may be cold)"),
        "{}",
        stderr(&rename)
    );
    assert!(!project.read("stc.renames.toml").contains("MainConfig"));

    let build = project.build_ir();
    assert_eq!(build.status.code(), Some(0), "{}", stderr(&build));
}

#[test]
fn case_only_rename_writes_no_history_and_a_bad_self_mapping_is_ignored() {
    let project = TestDir::new("case-only");
    project.write("stc.toml", &manifest("app", ""));
    project.write("src/main.st", BASIC_SOURCE);

    let rename = project.run(&["rename", "Main.Speed", "Main.speed"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    assert!(project.read("src/main.st").contains("speed : DINT;"));
    assert!(
        stderr(&rename).contains("no history entry (case-only)"),
        "{}",
        stderr(&rename)
    );
    assert!(!project.path("stc.renames.toml").exists());

    project.write(
        "stc.renames.toml",
        "schema = 1\n\n[[rename]]\nold = \"Main.Speed\"\nnew = \"Main.speed\"\n",
    );
    let build = project.build_ir();
    assert_eq!(build.status.code(), Some(0), "{}", stderr(&build));
}

#[test]
fn canonical_keys_and_history_free_targets_are_reported() {
    let project = TestDir::new("canonical");
    project.write("stc.toml", &manifest("app", ""));
    project.write("src/main.st", BASIC_SOURCE);

    let rename = project.run(&["rename", "app.Main.Speed", "app.Main.Rate"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    let history = project.read("stc.renames.toml");
    assert!(history.contains("old = \"Main.Speed\""), "{history}");
    assert!(history.contains("new = \"Main.Rate\""), "{history}");
    assert!(!history.contains("app.Main"), "{history}");

    let parameter = project.run(&["rename", "AddStep.value", "AddStep.input"]);
    assert_eq!(parameter.status.code(), Some(0), "{}", stderr(&parameter));
    assert!(
        stderr(&parameter).contains("no history entry (no GOT identity)"),
        "{}",
        stderr(&parameter)
    );
    assert!(!project.read("stc.renames.toml").contains("AddStep.value"));

    let local = project.run(&["rename", "AddStep.local_value", "AddStep.scratch"]);
    assert_eq!(local.status.code(), Some(0), "{}", stderr(&local));
    assert!(
        stderr(&local).contains("no history entry (no GOT identity)"),
        "{}",
        stderr(&local)
    );
    assert!(
        !project
            .read("stc.renames.toml")
            .contains("AddStep.local_value")
    );
}

#[test]
fn collisions_and_locked_projects_leave_files_unchanged() {
    let project = TestDir::new("collision-locked");
    project.write("stc.toml", &manifest("app", ""));
    project.write("src/main.st", BASIC_SOURCE);
    let before = project.read("src/main.st");

    let collision = project.run(&["rename", "Main.Speed", "Main.TEXT"]);
    assert_eq!(collision.status.code(), Some(2), "{}", stderr(&collision));
    assert!(
        stderr(&collision).contains("STC-E-PROJECT-013"),
        "{}",
        stderr(&collision)
    );
    assert_eq!(project.read("src/main.st"), before);
    assert!(!project.path("stc.renames.toml").exists());

    let missing = project.run(&["rename", "Main.Speed", "Main.Rate", "--locked"]);
    assert_eq!(missing.status.code(), Some(2), "{}", stderr(&missing));
    assert_eq!(project.read("src/main.st"), before);
    assert!(!project.path("stc.renames.toml").exists());

    let check = project.run(&["check"]);
    assert_eq!(check.status.code(), Some(0), "{}", stderr(&check));
    let stale = project
        .read("stc.toml")
        .replace("version = \"0.1.0\"", "version = \"0.2.0\"");
    project.write("stc.toml", &stale);
    let locked = project.run(&["rename", "Main.Speed", "Main.Rate", "--locked"]);
    assert_eq!(locked.status.code(), Some(2), "{}", stderr(&locked));
    assert!(
        stderr(&locked).contains("STC-E-LOCK-002"),
        "{}",
        stderr(&locked)
    );
    assert_eq!(project.read("src/main.st"), before);
    assert!(!project.path("stc.renames.toml").exists());
}

#[test]
fn manifest_path_selects_a_project_from_another_directory() {
    let project = TestDir::new("manifest-path");
    project.write("app/stc.toml", &manifest("app", ""));
    project.write("app/src/main.st", BASIC_SOURCE);

    let output = Command::new(stc())
        .args([
            "rename",
            "Main.Speed",
            "Main.Rate",
            "--manifest-path",
            "app",
        ])
        .current_dir(&project.path)
        .output()
        .expect("run stc");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(project.read("app/src/main.st").contains("Rate := Rate + 1"));
    assert!(project.path("app/stc.renames.toml").exists());
    assert!(!project.path("stc.renames.toml").exists());
}

#[test]
fn relaxed_syntax_is_used_for_analysis_and_reanalysis() {
    let project = TestDir::new("relaxed");
    project.write("stc.toml", &manifest("app", ""));
    project.write(
        "src/main.st",
        "PROGRAM Main\nVAR\n  Speed : DINT;\nEND_VAR\n  IF Speed = 0 THEN\n    Speed := Speed + 1;\n  END_IF\nEND_PROGRAM\n",
    );

    let rename = project.run(&["rename", "Main.Speed", "Main.Rate", "--relaxed"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    assert!(project.read("src/main.st").contains("Rate := Rate + 1"));
}

#[test]
fn var_access_and_var_config_paths_are_edited() {
    let project = TestDir::new("access");
    project.write("stc.toml", &manifest("app", "configuration = \"C\""));
    project.write(
        "src/main.st",
        "TYPE Inner : STRUCT\n  x : DINT;\nEND_STRUCT; END_TYPE\nPROGRAM Main\nVAR\n  x : DINT;\n  nested : Inner;\nEND_VAR\nVAR_ACCESS\n  visible : x : DINT READ_ONLY;\n  nested_path : nested.x : DINT READ_ONLY;\nEND_VAR\n  x := 1;\nEND_PROGRAM\nCONFIGURATION C\nVAR_GLOBAL\n  seed : DINT;\nEND_VAR\nRESOURCE R ON CPU\n  TASK t (INTERVAL := T#10ms);\n  PROGRAM inst WITH t : Main;\nEND_RESOURCE\nVAR_ACCESS\n  seed_path : seed : DINT READ_WRITE;\n  inst_x : inst.x : DINT READ_ONLY;\n  inst_nested : inst.nested.x : DINT READ_ONLY;\nEND_VAR\nVAR_CONFIG\n  inst.nested.x AT %MD10 : DINT := 5;\nEND_VAR\nEND_CONFIGURATION\n",
    );

    let rename = project.run(&["rename", "Main.x", "Main.value"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    let source = project.read("src/main.st");
    assert!(source.contains("visible : value"), "{source}");
    assert!(source.contains("inst_x : inst.value"), "{source}");
    assert!(source.contains("nested_path : nested.x"), "{source}");
    let history = project.read("stc.renames.toml");
    assert!(history.contains("old = \"Main.x\""), "{history}");
    assert!(history.contains("new = \"Main.value\""), "{history}");

    let rename = project.run(&["rename", "Inner.x", "Inner.y"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    let source = project.read("src/main.st");
    assert!(source.contains("nested_path : nested.y"), "{source}");
    assert!(source.contains("inst_nested : inst.nested.y"), "{source}");
    assert!(source.contains("inst.nested.y AT %MD10"), "{source}");
    let history = project.read("stc.renames.toml");
    assert!(history.contains("old = \"Inner.x\""), "{history}");

    let rename = project.run(&["rename", "C.seed", "C.start"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    assert!(project.read("src/main.st").contains("seed_path : start"));
    let history = project.read("stc.renames.toml");
    assert!(history.contains("old = \"C.seed\""), "{history}");
}

#[test]
fn a_missing_profile_warns_and_still_renames_source() {
    let project = TestDir::new("profile-missing");
    project.write(
        "stc.toml",
        &manifest("app", "[io]\nconfig = \"io/missing.toml\"\n"),
    );
    project.write(
        "src/main.st",
        "PROGRAM Main\nVAR\n  counter : DINT;\nEND_VAR\n  counter := counter + 1;\nEND_PROGRAM\n",
    );

    let rename = project.run(&["rename", "Main.counter", "Main.count"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    assert!(project.read("src/main.st").contains("count := count + 1"));
    let message = stderr(&rename);
    assert!(
        message.contains("selected machine profile unavailable"),
        "{message}"
    );
    assert!(message.contains("missing.toml"), "{message}");
}

#[test]
fn dry_run_prints_access_and_history_decisions_without_writing() {
    let project = TestDir::new("access-dry");
    project.write("stc.toml", &manifest("app", "configuration = \"C\""));
    project.write(
        "src/main.st",
        "PROGRAM Main\nVAR\n  x : DINT;\nEND_VAR\nVAR_ACCESS\n  visible : x : DINT READ_ONLY;\nEND_VAR\n  x := 1;\nEND_PROGRAM\nCONFIGURATION C\nRESOURCE R ON CPU\n  TASK t (INTERVAL := T#10ms);\n  PROGRAM inst WITH t : Main;\nEND_RESOURCE\nEND_CONFIGURATION\n",
    );
    let original = project.read("src/main.st");

    let dry = project.run(&["rename", "Main.x", "Main.value", "--dry-run"]);
    assert_eq!(dry.status.code(), Some(0), "{}", stderr(&dry));
    assert!(
        stdout(&dry).contains("Main.x -> Main.value"),
        "{}",
        stdout(&dry)
    );
    assert!(stdout(&dry).contains("history:"), "{}", stdout(&dry));
    assert_eq!(project.read("src/main.st"), original);
    assert!(!project.path("stc.renames.toml").exists());
}

#[test]
fn a_declaration_named_like_the_package_namespace_is_renameable() {
    let project = TestDir::new("def-path");
    project.write("stc.toml", &manifest("Sample", ""));
    project.write(
        "src/main.st",
        "FUNCTION Sample : DINT\n  Sample := 1;\nEND_FUNCTION\nPROGRAM Main\nVAR\n  value : DINT;\nEND_VAR\n  value := Sample();\nEND_PROGRAM\n",
    );

    let rename = project.run(&["rename", "Sample", "SampleX"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    assert!(project.read("src/main.st").contains("FUNCTION SampleX"));
    let history = project.read("stc.renames.toml");
    assert!(history.contains("old = \"Sample\""), "{history}");
    assert!(history.contains("new = \"SampleX\""), "{history}");
}
