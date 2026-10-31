//! Project-mode driver tests: `stc.toml`, package graphs, package-qualified
//! diagnostics, and GOT identities (M6 phase 2, D-153).

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use stc_got::Manifest;

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

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stc-project-{}-{name}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create project directory");
        Self { path }
    }

    fn path(&self, relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(self.path.clone(), |path, part| path.join(part))
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.path(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create source directory");
        }
        std::fs::write(&path, contents).expect("write project file");
        path
    }

    fn run(&self, directory: &str, args: &[&str]) -> Output {
        Command::new(stc())
            .args(args)
            .current_dir(self.path(directory))
            .output()
            .expect("run stc")
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn manifest(name: &str, project: &str, dependencies: &str) -> String {
    format!(
        r#"schema = 1

[package]
name = "{name}"
version = "0.1.0"
language = "stc-2013-oop-utf8-v1"

[project]
sources = ["src/**/*.st"]
{project}

{dependencies}
"#
    )
}

const LIB_SOURCE: &str = "\
VAR_GLOBAL G : DINT; END_VAR
FUNCTION Double : DINT
VAR_INPUT x : DINT; END_VAR
  Double := x * 2;
END_FUNCTION
";

const APP_SOURCE: &str = "\
PROGRAM Main
VAR x : DINT; END_VAR
  drive.G := drive.G + 1;
  x := drive.Double(drive.G);
END_PROGRAM
";

fn write_two_package_project(project: &TestDir, app_project: &str, app_source: &str) {
    project.write("motor-lib/stc.toml", &manifest("motor-lib", "", ""));
    project.write("motor-lib/src/lib.st", LIB_SOURCE);
    project.write(
        "app/stc.toml",
        &manifest(
            "app",
            app_project,
            "[dependencies]\ndrive = { path = \"../motor-lib\", package = \"motor-lib\" }",
        ),
    );
    project.write("app/src/main.st", app_source);
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn check_builds_the_package_graph_in_dependency_order() {
    let project = TestDir::new("check-graph");
    write_two_package_project(&project, "", APP_SOURCE);
    // The default manifest is `stc.toml` in the current directory.
    let output = project.run("app", &["check"]);
    assert!(output.status.success(), "stderr:\n{}", stderr(&output));
    assert!(
        project.path("app/stc.lock").is_file(),
        "stc.lock is written"
    );
}

#[test]
fn check_renders_package_qualified_source_paths() {
    let project = TestDir::new("qualified-diagnostics");
    write_two_package_project(
        &project,
        "",
        "PROGRAM Main\nVAR x : DINT; END_VAR\n  x := drive.Missing();\nEND_PROGRAM\n",
    );
    let output = project.run("app", &["check"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr:\n{}",
        stderr(&output)
    );
    let stderr = stderr(&output);
    assert!(stderr.contains("app/src/main.st"), "{stderr}");
    assert!(stderr.contains("STC-E-NAME-001"), "{stderr}");
}

#[test]
fn check_rejects_internal_definitions_across_packages() {
    let project = TestDir::new("internal");
    project.write("motor-lib/stc.toml", &manifest("motor-lib", "", ""));
    project.write(
        "motor-lib/src/lib.st",
        "VAR_GLOBAL INTERNAL Secret : DINT; END_VAR\n",
    );
    project.write(
        "app/stc.toml",
        &manifest(
            "app",
            "",
            "[dependencies]\ndrive = { path = \"../motor-lib\", package = \"motor-lib\" }",
        ),
    );
    project.write(
        "app/src/main.st",
        "PROGRAM Main\nVAR x : DINT; END_VAR\n  drive.Secret := 1;\n  x := drive.Secret;\nEND_PROGRAM\n",
    );
    let output = project.run("app", &["check"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr:\n{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("STC-E-NAME-004"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn build_uses_package_identities_in_got_keys() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping module build test");
        return;
    };
    let project = TestDir::new("got-keys");
    write_two_package_project(&project, "", APP_SOURCE);
    let output = project.run("app", &["build", "--emit", "module", "--clang", clang]);
    assert!(output.status.success(), "stderr:\n{}", stderr(&output));
    let out_dir = project.path("app/target/stc/debug");
    let manifest_path = std::fs::read_dir(&out_dir)
        .expect("output directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".stc-got.json"))
        })
        .expect("GOT manifest");
    let text = std::fs::read_to_string(&manifest_path).expect("read manifest");
    let manifest = Manifest::decode_json(&text).expect("decode manifest");
    assert!(
        manifest.roots.iter().any(|root| root.key == "MOTOR-LIB::G"),
        "{:?}",
        manifest
            .roots
            .iter()
            .map(|root| &root.key)
            .collect::<Vec<_>>()
    );
    assert!(
        manifest
            .functions
            .iter()
            .any(|function| function.key.starts_with("MOTOR-LIB::DOUBLE@")),
        "{:?}",
        manifest
            .functions
            .iter()
            .map(|function| &function.key)
            .collect::<Vec<_>>()
    );
    let has_module = std::fs::read_dir(&out_dir)
        .expect("output directory")
        .filter_map(Result::ok)
        .any(|entry| {
            let path = entry.path();
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension == "dll" || extension == "so")
        });
    assert!(has_module, "a loadable module is produced");
}

#[test]
fn double_project_build_is_byte_identical() {
    let project = TestDir::new("determinism");
    write_two_package_project(&project, "", APP_SOURCE);
    for directory in ["first", "second"] {
        let output = project.run(
            "app",
            &[
                "build",
                "--emit",
                "ir,metadata",
                "--out-dir",
                &format!("target/{directory}"),
            ],
        );
        assert!(output.status.success(), "stderr:\n{}", stderr(&output));
    }
    for name in ["app.ll", "app.stc-meta.json"] {
        let first = std::fs::read(project.path(&format!("app/target/first/{name}")))
            .unwrap_or_else(|error| panic!("read first {name}: {error}"));
        let second = std::fs::read(project.path(&format!("app/target/second/{name}")))
            .unwrap_or_else(|error| panic!("read second {name}: {error}"));
        assert_eq!(first, second, "{name} is not deterministic");
    }
}

#[test]
fn dependency_configurations_are_not_build_entries() {
    let project = TestDir::new("dependency-configuration");
    project.write("motor-lib/stc.toml", &manifest("motor-lib", "", ""));
    project.write(
        "motor-lib/src/lib.st",
        "\
PROGRAM Dep
VAR x : DINT; END_VAR
  x := 1;
END_PROGRAM
CONFIGURATION DepMain
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms);
  PROGRAM inst WITH t : Dep;
END_RESOURCE
END_CONFIGURATION
",
    );
    project.write(
        "app/stc.toml",
        &manifest(
            "app",
            "",
            "[dependencies]\ndrive = { path = \"../motor-lib\", package = \"motor-lib\" }",
        ),
    );
    project.write(
        "app/src/main.st",
        "PROGRAM Main\nVAR x : DINT; END_VAR\n  x := 1;\nEND_PROGRAM\n",
    );
    let output = project.run("app", &["build", "--emit", "ir,metadata"]);
    assert!(output.status.success(), "stderr:\n{}", stderr(&output));
    let metadata = std::fs::read_to_string(project.path("app/target/stc/debug/app.stc-meta.json"))
        .expect("metadata");
    assert!(metadata.contains("\"configuration\": null"), "{metadata}");
}

#[test]
fn manifest_configuration_selects_a_root_configuration() {
    let project = TestDir::new("configuration-selection");
    write_two_package_project(
        &project,
        "configuration = \"Alternate\"",
        "\
PROGRAM Body
VAR x : DINT; END_VAR
  x := 1;
END_PROGRAM
CONFIGURATION Main
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms);
  PROGRAM inst WITH t : Body;
END_RESOURCE
END_CONFIGURATION
CONFIGURATION Alternate
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms);
  PROGRAM inst WITH t : Body;
END_RESOURCE
END_CONFIGURATION
",
    );
    let output = project.run("app", &["build", "--emit", "ir,metadata"]);
    assert!(output.status.success(), "stderr:\n{}", stderr(&output));
    let metadata = std::fs::read_to_string(project.path("app/target/stc/debug/app.stc-meta.json"))
        .expect("metadata");
    assert!(
        metadata.contains("\"configuration\": \"Alternate\""),
        "{metadata}"
    );
}

#[test]
fn stdlib_false_suppresses_standard_fbs_only() {
    let project = TestDir::new("stdlib");
    project.write("app/stc.toml", &manifest("app", "stdlib = false", ""));
    project.write(
        "app/src/main.st",
        "PROGRAM Main\nVAR t : TON; END_VAR\n  t(IN := TRUE, PT := T#1s);\nEND_PROGRAM\n",
    );
    let output = project.run("app", &["check"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr:\n{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("STC-E-NAME-001"),
        "{}",
        stderr(&output)
    );

    project.write(
        "app/src/main.st",
        "FUNCTION_BLOCK TON\nVAR_INPUT IN : BOOL; END_VAR\nVAR_OUTPUT Q : BOOL; END_VAR\n  Q := IN;\nEND_FUNCTION_BLOCK\nPROGRAM Main\nVAR t : TON; x : DINT; END_VAR\n  t(IN := TRUE);\n  x := ABS(-1);\nEND_PROGRAM\n",
    );
    let output = project.run("app", &["check"]);
    assert!(output.status.success(), "stderr:\n{}", stderr(&output));
}

#[test]
fn separate_test_project_depends_on_the_code_under_test() {
    let project = TestDir::new("separate-test-project");
    project.write("code/stc.toml", &manifest("code", "", ""));
    project.write(
        "code/src/lib.st",
        "\
FUNCTION Clamp : DINT
VAR_INPUT x : DINT; END_VAR
  IF x < 0 THEN
    Clamp := 0;
  ELSE
    Clamp := x;
  END_IF;
END_FUNCTION
",
    );
    project.write(
        "tests/stc.toml",
        &manifest("tests", "", "[dependencies]\ncode = { path = \"../code\" }"),
    );
    project.write(
        "tests/src/tests.st",
        "\
FUNCTION ClampAcceptsPositive : BOOL
  ClampAcceptsPositive := code.Clamp(4) = 4;
END_FUNCTION
",
    );
    let output = project.run("tests", &["check", "--manifest-path", "."]);
    assert!(output.status.success(), "stderr:\n{}", stderr(&output));
}

#[test]
fn manifest_path_accepts_a_directory_from_another_working_directory() {
    let project = TestDir::new("manifest-path");
    write_two_package_project(&project, "", APP_SOURCE);
    let root = project.path("app");
    let output = Command::new(stc())
        .args(["check", "--manifest-path"])
        .arg(&root)
        .output()
        .expect("run stc");
    assert!(output.status.success(), "stderr:\n{}", stderr(&output));
}

#[test]
fn manifest_path_cannot_be_combined_with_input_files() {
    let project = TestDir::new("manifest-mixing");
    write_two_package_project(&project, "", APP_SOURCE);
    let source = project.path("app/src/main.st");
    let output = Command::new(stc())
        .args(["check", "--manifest-path"])
        .arg(project.path("app"))
        .arg(&source)
        .output()
        .expect("run stc");
    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr:\n{}",
        stderr(&output)
    );
}

#[test]
fn project_files_and_manifest_are_reported_when_missing() {
    let project = TestDir::new("missing-manifest");
    let output = project.run(".", &["check"]);
    // A manifest error is exit code 2 (CLI-and-Manifest §1.3).
    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr:\n{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("STC-E-PROJECT-001"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn empty_source_glob_is_an_error() {
    let project = TestDir::new("no-sources");
    project.write("app/stc.toml", &manifest("app", "", ""));
    let output = project.run("app", &["check"]);
    // A manifest error is exit code 2 (CLI-and-Manifest §1.3).
    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr:\n{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("STC-E-PROJECT-004"),
        "{}",
        stderr(&output)
    );
}

const IO_PROFILE: &str = r#"[device.diewu]
vendor = 2

[[routes]]
source = { kind = "physical-input", device = "diewu", channel = "IN0" }
sinks = [
  { kind = "plc-input", variable = "shadow" },
  { kind = "physical-output", device = "diewu", channel = "OUT0" },
]

[[routes]]
source = { kind = "plc-output", variable = "drive" }
sinks = [{ kind = "plc-input", variable = "shadow2" }]
"#;

const IO_SOURCE: &str = "\
VAR_GLOBAL
  shadow AT %IB0 : BYTE;
  shadow2 AT %IB1 : BYTE;
  drive AT %QB0 : BYTE;
END_VAR
PROGRAM Main
VAR value : BYTE; END_VAR
  value := shadow;
  drive := shadow2;
END_PROGRAM
";

fn io_project(project: &TestDir, profile_config: &str, profile_contents: Option<&str>) {
    project.write(
        "app/stc.toml",
        &manifest(
            "app",
            &format!(
                "[io]\nbackend = \"ethercat\"\nconfig = \"{profile_config}\"\n\n\
                 [[io.devices]]\nname = \"diewu\"\nkind = \"ethercat\"\n\
                 config = {{ vendor = 1, revision = 3 }}\n\n\
                 [[io.devices.channels]]\nname = \"IN0\"\ndirection = \"input\"\nwidth = 8\n\n\
                 [[io.devices.channels]]\nname = \"OUT0\"\ndirection = \"output\"\nwidth = 8"
            ),
            "",
        ),
    );
    project.write("app/src/main.st", IO_SOURCE);
    if let Some(profile) = profile_contents {
        project.write(&format!("app/{profile_config}"), profile);
    }
}

#[test]
fn machine_profile_is_embedded_with_routes_and_device_precedence() {
    let project = TestDir::new("io-profile");
    io_project(&project, "profiles/machine.toml", Some(IO_PROFILE));
    let output = project.run("app", &["build", "--emit", "ir,metadata"]);
    assert!(output.status.success(), "stderr:\n{}", stderr(&output));

    let text = std::fs::read_to_string(project.path("app/target/stc/debug/app.stc-meta.json"))
        .expect("metadata");
    let metadata: serde_json::Value = serde_json::from_str(&text).expect("metadata JSON");
    let io = &metadata["io"];
    assert_eq!(io["backend"], "ethercat");

    let profile = &io["profile"];
    assert_eq!(profile["path"], "profiles/machine.toml");
    assert_eq!(profile["text"], IO_PROFILE);
    let digest =
        stc_got::Digest32::from_bytes(stc_got::sha256::sha256(IO_PROFILE.as_bytes())).to_hex();
    assert_eq!(profile["digest"], digest);
    assert_eq!(profile["devices"], serde_json::json!(["diewu"]));

    // Profile device configuration wins field by field over the inline table.
    let device = &io["devices"][0];
    assert_eq!(device["name"], "diewu");
    assert_eq!(device["kind"], "ethercat");
    assert_eq!(device["config"]["vendor"], 2);
    assert_eq!(device["config"]["revision"], 3);

    let routes = io["routes"].as_array().expect("route array");
    assert_eq!(routes.len(), 2);
    assert_eq!(
        routes[0]["source"],
        serde_json::json!({
            "area": null,
            "channel": "IN0",
            "device": "diewu",
            "kind": "physical-input",
            "offset_bits": null,
            "variable": null,
            "width_bits": 8,
        })
    );
    assert_eq!(
        routes[0]["sinks"],
        serde_json::json!([
            {
                "area": "I",
                "channel": null,
                "device": null,
                "kind": "plc-input",
                "offset_bits": 0,
                "variable": "shadow",
                "width_bits": 8,
            },
            {
                "area": null,
                "channel": "OUT0",
                "device": "diewu",
                "kind": "physical-output",
                "offset_bits": null,
                "variable": null,
                "width_bits": 8,
            },
        ])
    );
    assert_eq!(
        routes[1]["source"],
        serde_json::json!({
            "area": "Q",
            "channel": null,
            "device": null,
            "kind": "plc-output",
            "offset_bits": 0,
            "variable": "drive",
            "width_bits": 8,
        })
    );
    assert_eq!(
        routes[1]["sinks"],
        serde_json::json!([{
            "area": "I",
            "channel": null,
            "device": null,
            "kind": "plc-input",
            "offset_bits": 8,
            "variable": "shadow2",
            "width_bits": 8,
        }])
    );
}

#[test]
fn missing_or_invalid_machine_profile_fails_the_build() {
    let missing = TestDir::new("io-profile-missing");
    io_project(&missing, "profiles/machine.toml", None);
    let output = missing.run("app", &["build", "--emit", "ir,metadata"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr:\n{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("STC-E-PROJECT-009"),
        "{}",
        stderr(&output)
    );

    let invalid = TestDir::new("io-profile-invalid");
    io_project(&invalid, "profiles/machine.toml", Some("[device."));
    let output = invalid.run("app", &["build", "--emit", "ir,metadata"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr:\n{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("STC-E-PROJECT-001"),
        "{}",
        stderr(&output)
    );
}
