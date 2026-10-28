//! M6.5 phase-2 debug/release driver tests: mode precedence, default output
//! directories, opt levels, descriptor hook bits, release stripping, and the
//! host-visible checkpoint contract.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use stc_got::ImportService;
use stc_got::descriptor::{entry_flag, feature};

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
                    eprintln!("note: clang not found; skipping compile-backed checks");
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
            "stc-debug-release-{}-{name}-{unique}",
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

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

fn manifest(name: &str, extra: &str) -> String {
    format!(
        "schema = 1\n\
         \n\
         [package]\n\
         name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         language = \"stc-2013-oop-utf8-v1\"\n\
         \n\
         [project]\n\
         sources = [\"src/**/*.st\"]\n\
         \n\
         {extra}"
    )
}

const PROGRAM: &str = "\
FUNCTION Add : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  Add := a + b;
END_FUNCTION

PROGRAM Main
VAR
  n : DINT;
  total : DINT;
END_VAR
  n := Add(1, 2);
  IF n > 0 THEN
    total := total + n;
  END_IF;
  WHILE total < 10 DO
    total := total + 1;
  END_WHILE;
END_PROGRAM
";

fn scaffold(name: &str, extra: &str) -> TestDir {
    let project = TestDir::new(name);
    project.write("app/stc.toml", &manifest("app", extra));
    project.write("app/src/main.st", PROGRAM);
    project
}

fn read_metadata(project: &TestDir, relative: &str) -> serde_json::Value {
    let text = std::fs::read_to_string(project.path(relative))
        .unwrap_or_else(|error| panic!("read metadata `{relative}`: {error}"));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("parse metadata `{relative}`: {error}\n{text}"))
}

fn find_object<'a>(text: &'a str, key: &str) -> &'a str {
    text.lines()
        .find(|line| line.contains(key))
        .unwrap_or_else(|| panic!("no JSON object containing `{key}` in:\n{text}"))
}

fn host_platform() -> stc_runtime::HostPlatform {
    stc_runtime::HostPlatform::new(
        stc_runtime::PROFILE,
        stc_codegen::resolve_target(Some(&stc_codegen::host_triple())),
        stc_got::HOST_ABI_MAJOR,
        stc_got::HOST_ABI_MINOR,
    )
}

fn open_candidate(
    project: &TestDir,
    out_dir: &str,
    metadata_relative: &str,
) -> stc_runtime::Candidate {
    let metadata = read_metadata(project, metadata_relative);
    let module_file = metadata["module"]["file"]
        .as_str()
        .filter(|file| !file.is_empty())
        .expect("the module file name is recorded");
    let module = project.path(&format!("{out_dir}/{module_file}"));
    let manifest = project.path(&format!("{out_dir}/app.stc-got.bin"));
    stc_runtime::Candidate::open(&host_platform(), &module, &manifest)
        .unwrap_or_else(|error| panic!("candidate: {error}"))
}

fn has_service_13(candidate: &stc_runtime::Candidate) -> bool {
    candidate.descriptor().slots.iter().any(|slot| {
        slot.service == ImportService::DebugCheck.id() && slot.kind == stc_got::SlotKind::Import
    })
}

fn debug_marker() -> &'static [u8] {
    if cfg!(windows) {
        b".debug$S"
    } else {
        b".debug_info"
    }
}

fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn default_is_debug_and_release_is_hook_free_with_distinct_identity() {
    let project = scaffold("forms", "[build]\n");
    let debug = project.run("app", &["build", "--emit", "ir,metadata"]);
    assert_eq!(code(&debug), 0, "{}", stderr(&debug));
    let debug_ir = std::fs::read_to_string(project.path("app/target/stc/debug/app.ll"))
        .expect("default IR is target/stc/debug");
    assert!(debug_ir.contains("!llvm.dbg.cu"), "{debug_ir}");
    assert!(debug_ir.contains("!DISubprogram"), "{debug_ir}");
    let debug_metadata = read_metadata(&project, "app/target/stc/debug/app.stc-meta.json");
    let debug_object = debug_metadata["debug"].as_object().expect("debug object");
    assert_eq!(debug_object["format"], 1);
    assert_eq!(debug_object["hooks"], true);
    assert!(
        debug_object["checkpoints"]
            .as_array()
            .is_some_and(|checkpoints| !checkpoints.is_empty()),
        "default debug builds carry checkpoints: {debug_object:?}"
    );

    let release = project.run("app", &["build", "--release", "--emit", "ir,metadata"]);
    assert_eq!(code(&release), 0, "{}", stderr(&release));
    let release_ir = std::fs::read_to_string(project.path("app/target/stc/release/app.ll"))
        .expect("release IR is target/stc/release");
    assert!(!release_ir.contains("!llvm.dbg"), "{release_ir}");
    assert!(!release_ir.contains("!dbg"), "{release_ir}");
    let release_metadata = read_metadata(&project, "app/target/stc/release/app.stc-meta.json");
    assert!(
        release_metadata.get("debug").is_none(),
        "release sidecars have no debug object: {release_metadata}"
    );
    assert_ne!(
        debug_metadata["build_id"], release_metadata["build_id"],
        "debug and release builds have distinct identities"
    );
    assert_ne!(
        debug_metadata["manifest"]["digest"], release_metadata["manifest"]["digest"],
        "debug and release builds have distinct manifests"
    );

    let summary = project.run("app", &["metadata", "--message-format", "json"]);
    assert_eq!(code(&summary), 0, "{}", stderr(&summary));
    let text = stdout(&summary);
    let line = find_object(&text, "\"build_mode\":");
    assert!(line.contains("\"build_mode\":\"debug\""), "{line}");
    assert!(!line.contains("\"debug_info\":\"none\""), "{line}");
    assert!(line.contains("\"debug_hooks\":true"), "{line}");
    let release_summary = project.run(
        "app",
        &["metadata", "--release", "--message-format", "json"],
    );
    assert_eq!(code(&release_summary), 0, "{}", stderr(&release_summary));
    let text = stdout(&release_summary);
    let line = find_object(&text, "\"build_mode\":");
    assert!(line.contains("\"build_mode\":\"release\""), "{line}");
    assert!(line.contains("\"debug_info\":\"none\""), "{line}");
    assert!(line.contains("\"debug_hooks\":false"), "{line}");
}

#[test]
fn modules_expose_2_3_hooks_in_debug_and_2_1_without_hooks_in_release() {
    let Some(_clang) = clang() else {
        return;
    };
    let project = scaffold("hostcontract", "[build]\n");
    let debug = project.run("app", &["build", "--emit", "ir,module,obj,metadata"]);
    assert_eq!(code(&debug), 0, "{}", stderr(&debug));
    let release = project.run(
        "app",
        &["build", "--release", "--emit", "ir,module,obj,metadata"],
    );
    assert_eq!(code(&release), 0, "{}", stderr(&release));

    let debug_candidate = open_candidate(
        &project,
        "app/target/stc/debug",
        "app/target/stc/debug/app.stc-meta.json",
    );
    let descriptor = debug_candidate.descriptor();
    assert_eq!(descriptor.host_abi_minor, 3);
    assert_ne!(descriptor.entry_flags & entry_flag::DEBUG_BUILD, 0);
    assert_ne!(descriptor.entry_flags & entry_flag::DEBUG_HOOKS, 0);
    assert_ne!(descriptor.required_features & feature::DEBUG_HOOKS, 0);
    assert!(has_service_13(&debug_candidate));
    let info = debug_candidate.info().expect("debug metadata sidecar");
    assert!(
        info.debug.is_some(),
        "the host can read the additive debug object"
    );
    let debug_metadata = read_metadata(&project, "app/target/stc/debug/app.stc-meta.json");
    assert_eq!(
        debug_candidate.build_id().to_hex(),
        debug_metadata["build_id"].as_str().expect("build id")
    );
    assert_eq!(
        debug_candidate.manifest_digest().to_hex(),
        debug_metadata["manifest"]["digest"]
            .as_str()
            .expect("manifest digest")
    );

    let release_candidate = open_candidate(
        &project,
        "app/target/stc/release",
        "app/target/stc/release/app.stc-meta.json",
    );
    let descriptor = release_candidate.descriptor();
    assert_eq!(descriptor.host_abi_minor, 1);
    assert_eq!(descriptor.entry_flags & entry_flag::DEBUG_BUILD, 0);
    assert_eq!(descriptor.entry_flags & entry_flag::DEBUG_HOOKS, 0);
    assert_eq!(descriptor.required_features & feature::DEBUG_HOOKS, 0);
    assert!(!has_service_13(&release_candidate));
    assert!(
        release_candidate
            .info()
            .expect("release metadata sidecar")
            .debug
            .is_none(),
        "release sidecars have no debug object"
    );
    let release_metadata = read_metadata(&project, "app/target/stc/release/app.stc-meta.json");
    assert_ne!(
        debug_candidate.build_id(),
        release_candidate.build_id(),
        "the build identity survives stripping and differs from debug"
    );

    let debug_object =
        std::fs::read(project.path("app/target/stc/debug/app.o")).expect("debug object");
    assert!(
        contains(&debug_object, debug_marker()),
        "debug objects keep DWARF or CodeView"
    );
    if cfg!(windows) {
        assert_eq!(
            &debug_object[4..8],
            &[0, 0, 0, 0],
            "COFF objects must not carry a wall-clock timestamp"
        );
    }
    // On ELF the linked module keeps DWARF. On MSVC targets the object carries
    // `.debug$S`/`.debug$T`, but the current lld-link invocation (no `/DEBUG`)
    // neither copies CodeView into the DLL nor emits a PDB; that phase-2
    // limitation is recorded in the M6.5 plan and is not changed here.
    if !cfg!(windows) {
        let debug_module = std::fs::read(project.path(&format!(
            "app/target/stc/debug/{}",
            debug_metadata["module"]["file"]
                .as_str()
                .expect("debug module file")
        )))
        .expect("debug module");
        assert!(
            contains(&debug_module, debug_marker()),
            "ELF debug modules keep DWARF"
        );
    }
    assert!(
        project
            .path("app/target/stc/debug/app.stc-frames.json")
            .is_file(),
        "a hook-bearing debug build writes the stopped-frame sidecar"
    );
    assert!(
        !project
            .path("app/target/stc/release/app.stc-frames.json")
            .exists(),
        "release builds write no stopped-frame sidecar"
    );

    let release_object =
        std::fs::read(project.path("app/target/stc/release/app.o")).expect("release object");
    assert!(
        !contains(&release_object, debug_marker()),
        "release objects are compiled without debug info"
    );
    let release_module = std::fs::read(project.path(&format!(
        "app/target/stc/release/{}",
        release_metadata["module"]["file"]
            .as_str()
            .expect("release module file")
    )))
    .expect("release module");
    for marker in [
        b".debug$S".as_slice(),
        b".debug$T".as_slice(),
        b".debug_info".as_slice(),
        b".debug_line".as_slice(),
        b".debug_abbrev".as_slice(),
        b".zdebug_".as_slice(),
    ] {
        assert!(
            !contains(&release_module, marker),
            "release modules must be stripped of `{}`",
            String::from_utf8_lossy(marker)
        );
    }

    // Hook-free loading must not depend on the sidecar debug object.
    let metadata_path = project.path("app/target/stc/release/app.stc-meta.json");
    std::fs::remove_file(&metadata_path).expect("remove release metadata");
    let module = project.path(&format!(
        "app/target/stc/release/{}",
        release_metadata["module"]["file"]
            .as_str()
            .expect("release module file")
    ));
    let candidate = stc_runtime::Candidate::open(
        &host_platform(),
        &module,
        &project.path("app/target/stc/release/app.stc-got.bin"),
    )
    .expect("release module loads without a debug object or metadata sidecar");
    assert!(candidate.info().is_none());
    assert!(!has_service_13(&candidate));
}

#[test]
fn mode_precedence_cli_over_manifest_and_conflicts_are_errors() {
    let project = scaffold("precedence", "[build]\n");
    let default = project.run("app", &["build", "--emit", "metadata"]);
    assert_eq!(code(&default), 0, "{}", stderr(&default));
    assert!(
        project
            .path("app/target/stc/debug/app.stc-meta.json")
            .is_file()
    );

    project.write("app/stc.toml", &manifest("app", "[build]\ndebug = false\n"));
    let _ = std::fs::remove_dir_all(project.path("app/target"));
    let release = project.run("app", &["build", "--emit", "metadata"]);
    assert_eq!(code(&release), 0, "{}", stderr(&release));
    assert!(
        project
            .path("app/target/stc/release/app.stc-meta.json")
            .is_file(),
        "explicit `[build].debug = false` selects release"
    );

    project.write(
        "app/stc.toml",
        &manifest("app", "[debug]\ndefault = false\n"),
    );
    let _ = std::fs::remove_dir_all(project.path("app/target"));
    let release = project.run("app", &["build", "--emit", "metadata"]);
    assert_eq!(code(&release), 0, "{}", stderr(&release));
    assert!(project.path("app/target/stc/release").is_dir());

    project.write("app/stc.toml", &manifest("app", "[build]\ndebug = true\n"));
    let _ = std::fs::remove_dir_all(project.path("app/target"));
    let cli_release = project.run("app", &["build", "--release", "--emit", "metadata"]);
    assert_eq!(code(&cli_release), 0, "{}", stderr(&cli_release));
    assert!(
        project
            .path("app/target/stc/release/app.stc-meta.json")
            .is_file(),
        "a CLI mode wins over the manifest"
    );

    project.write("app/stc.toml", &manifest("app", "[build]\ndebug = false\n"));
    let _ = std::fs::remove_dir_all(project.path("app/target"));
    let cli_debug = project.run("app", &["build", "--debug", "--emit", "metadata"]);
    assert_eq!(code(&cli_debug), 0, "{}", stderr(&cli_debug));
    assert!(
        project
            .path("app/target/stc/debug/app.stc-meta.json")
            .is_file(),
        "`--debug` wins over an explicit release manifest"
    );

    project.write(
        "app/stc.toml",
        &manifest(
            "precedence",
            "[build]\ndebug = false\n\n[debug]\ndefault = true\n",
        ),
    );
    let conflict = project.run("app", &["build", "--emit", "metadata"]);
    assert_eq!(code(&conflict), 2, "{}", stderr(&conflict));
    assert!(
        stderr(&conflict).contains("STC-E-PROJECT-010"),
        "{}",
        stderr(&conflict)
    );

    let flags = project.run("app", &["build", "--debug", "--release"]);
    assert_eq!(code(&flags), 2, "{}", stderr(&flags));
    assert!(
        stderr(&flags).contains("STC-E-PROJECT-010"),
        "{}",
        stderr(&flags)
    );
}

#[test]
fn opt_levels_default_per_mode_and_explicit_values_win() {
    let Some(_clang) = clang() else {
        return;
    };
    let project = scaffold("opt-levels", "[build]\n");
    let debug = project.run("app", &["build", "--emit", "obj", "-vv"]);
    assert_eq!(code(&debug), 0, "{}", stderr(&debug));
    assert!(stderr(&debug).contains("-O0"), "{}", stderr(&debug));

    let release = project.run("app", &["build", "--release", "--emit", "obj", "-vv"]);
    assert_eq!(code(&release), 0, "{}", stderr(&release));
    assert!(stderr(&release).contains("-O2"), "{}", stderr(&release));

    project.write("app/stc.toml", &manifest("app", "[build]\nopt-level = 1\n"));
    let explicit_release = project.run("app", &["build", "--release", "--emit", "obj", "-vv"]);
    assert_eq!(code(&explicit_release), 0, "{}", stderr(&explicit_release));
    assert!(
        stderr(&explicit_release).contains("-O1"),
        "{}",
        stderr(&explicit_release)
    );

    project.write("app/stc.toml", &manifest("app", "[build]\nopt-level = 3\n"));
    let explicit_debug = project.run("app", &["build", "--emit", "obj", "-vv"]);
    assert_eq!(code(&explicit_debug), 0, "{}", stderr(&explicit_debug));
    assert!(
        stderr(&explicit_debug).contains("-O3"),
        "{}",
        stderr(&explicit_debug)
    );
}

#[test]
fn explicit_out_dirs_are_verbatim_and_reject_mixed_modes() {
    let project = scaffold("outdirs", "[build]\n");
    let debug = project.run(
        "app",
        &["build", "--emit", "metadata", "--out-dir", "custom"],
    );
    assert_eq!(code(&debug), 0, "{}", stderr(&debug));
    assert!(project.path("app/custom/app.stc-meta.json").is_file());
    assert!(read_metadata(&project, "app/custom/app.stc-meta.json")["debug"].is_object());

    let mixed = project.run(
        "app",
        &[
            "build",
            "--release",
            "--emit",
            "metadata",
            "--out-dir",
            "custom",
        ],
    );
    assert_eq!(code(&mixed), 2, "{}", stderr(&mixed));
    assert!(
        stderr(&mixed).contains("STC-E-PROJECT-010")
            && stderr(&mixed).contains("distinct output directories"),
        "{}",
        stderr(&mixed)
    );

    let release = project.run(
        "app",
        &[
            "build",
            "--release",
            "--emit",
            "metadata",
            "--out-dir",
            "release-custom",
        ],
    );
    assert_eq!(code(&release), 0, "{}", stderr(&release));
    assert!(
        read_metadata(&project, "app/release-custom/app.stc-meta.json")
            .get("debug")
            .is_none()
    );

    project.write(
        "app/stc.toml",
        &manifest("app", "[build]\nout-dir = \"dist\"\n"),
    );
    let configured = project.run("app", &["build", "--emit", "metadata"]);
    assert_eq!(code(&configured), 0, "{}", stderr(&configured));
    assert!(project.path("app/dist/app.stc-meta.json").is_file());
}

#[test]
fn clean_selects_the_release_debug_or_test_directory() {
    let project = scaffold("cleanmode", "[build]\n");
    let debug = project.run("app", &["build", "--emit", "metadata"]);
    assert_eq!(code(&debug), 0, "{}", stderr(&debug));
    let release = project.run("app", &["build", "--release", "--emit", "metadata"]);
    assert_eq!(code(&release), 0, "{}", stderr(&release));
    let test_artifact = project.path("app/target/stc/test/case-A");
    std::fs::create_dir_all(&test_artifact).expect("create test artifact directory");
    std::fs::write(test_artifact.join("junk"), "junk").expect("write test artifact");
    assert!(project.path("app/target/stc/debug").is_dir());
    assert!(project.path("app/target/stc/release").is_dir());

    let clean_test = project.run("app", &["clean", "--test"]);
    assert_eq!(code(&clean_test), 0, "{}", stderr(&clean_test));
    assert!(
        !project.path("app/target/stc/test").exists(),
        "`clean --test` removes only the test directory"
    );
    assert!(project.path("app/target/stc/debug").is_dir());
    assert!(project.path("app/target/stc/release").is_dir());

    let clean_release = project.run("app", &["clean", "--release"]);
    assert_eq!(code(&clean_release), 0, "{}", stderr(&clean_release));
    assert!(
        !project.path("app/target/stc/release").exists(),
        "`clean --release` removes only the release directory"
    );
    assert!(
        project.path("app/target/stc/debug").is_dir(),
        "the debug directory survives"
    );

    // A default clean removes every generated tree, including trees that were
    // not touched by a selector.
    let release = project.run("app", &["build", "--release", "--emit", "metadata"]);
    assert_eq!(code(&release), 0, "{}", stderr(&release));
    std::fs::create_dir_all(&test_artifact).expect("recreate test artifact directory");
    let clean_all = project.run("app", &["clean"]);
    assert_eq!(code(&clean_all), 0, "{}", stderr(&clean_all));
    assert!(
        !project.path("app/target/stc/debug").exists(),
        "the default clean mode removes the debug directory"
    );
    assert!(!project.path("app/target/stc/release").exists());
    assert!(!project.path("app/target/stc/test").exists());
    assert!(
        project.path("app/stc.lock").is_file(),
        "clean preserves stc.lock"
    );
}

#[test]
fn debug_session_flags_are_rejected_by_non_session_commands() {
    let project = scaffold("sessionflags", "[build]\n");
    for arguments in [
        ["build", "--debug-server"].as_slice(),
        ["build", "--debug-protocol", "pipe"].as_slice(),
        ["build", "--allow-write"].as_slice(),
    ] {
        let output = project.run("app", arguments);
        assert_eq!(code(&output), 2, "{arguments:?}\n{}", stderr(&output));
        assert!(
            stderr(&output).contains("only valid with `stc run --debug-server`"),
            "{arguments:?}\n{}",
            stderr(&output)
        );
    }
    // `stc test` always builds the hook-free debug form and never runs a
    // server or a session (CLI-and-Manifest §1.7).
    for arguments in [
        ["test", "--debug-server"].as_slice(),
        ["test", "--allow-force"].as_slice(),
        ["test", "--session-timeout", "1s"].as_slice(),
        ["test", "--break", "src/main.st:3"].as_slice(),
    ] {
        let output = project.run("app", arguments);
        assert_eq!(code(&output), 2, "{arguments:?}\n{}", stderr(&output));
        assert!(
            stderr(&output).contains("STC-E-PROJECT-010"),
            "{arguments:?}\n{}",
            stderr(&output)
        );
    }
}
