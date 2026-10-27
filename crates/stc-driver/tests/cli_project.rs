//! `stc new/check/build/clean/metadata/compliance` integration tests
//! (M6 phase 3): project defaults, toolchain precedence, formats, colors,
//! lint levels, exit codes, and shared host libraries.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

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
            "stc-cli-phase3-{}-{name}-{unique}",
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

    fn run_env(&self, directory: &str, args: &[&str], environment: &[(&str, &str)]) -> Output {
        let mut command = Command::new(stc());
        command.args(args).current_dir(self.path(directory));
        for (name, value) in environment {
            command.env(name, value);
        }
        command.output().expect("run stc")
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn scaffold(project: &TestDir, name: &str) {
    let output = project.run(".", &["new", name]);
    assert!(
        output.status.success(),
        "new failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&output),
        stderr(&output)
    );
}

/// Builds the runtime static/shared pair once and returns their paths.
fn runtime_pair() -> Option<(PathBuf, PathBuf)> {
    static CACHE: OnceLock<Option<(PathBuf, PathBuf)>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            clang()?;
            let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()?
                .parent()?
                .to_path_buf();
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
            let output = Command::new(cargo)
                .args(["build", "-p", "stc-runtime", "--locked"])
                .current_dir(&workspace)
                .output()
                .expect("run cargo build -p stc-runtime");
            assert!(
                output.status.success(),
                "failed to build stc-runtime:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let profile = std::env::current_exe()
                .ok()?
                .parent()?
                .parent()?
                .to_path_buf();
            let static_name = if cfg!(windows) {
                "stc_runtime.lib"
            } else {
                "libstc_runtime.a"
            };
            let shared_name = if cfg!(windows) {
                "stc_runtime.dll"
            } else if cfg!(target_os = "macos") {
                "libstc_runtime.dylib"
            } else {
                "libstc_runtime.so"
            };
            let static_library = profile.join(static_name);
            let shared_library = profile.join(shared_name);
            if static_library.is_file() && shared_library.is_file() {
                Some((static_library, shared_library))
            } else {
                None
            }
        })
        .clone()
}

fn host_triple() -> &'static str {
    if cfg!(all(windows, target_env = "msvc")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

fn app_manifest(name: &str, build: &str) -> String {
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
         [build]\n\
         {build}\n"
    )
}

fn find_object<'a>(text: &'a str, key: &str) -> &'a str {
    text.lines()
        .find(|line| line.contains(key))
        .unwrap_or_else(|| panic!("no JSON object containing `{key}` in:\n{text}"))
}

const SCAFFOLD_PROGRAM: &str = "\
PROGRAM Main
VAR
  counter : DINT;
END_VAR
  counter := counter + 1;
END_PROGRAM
";

const RELAXED_PROGRAM: &str = "\
PROGRAM Main
VAR x : DINT; END_VAR
IF TRUE THEN x := 1; END_IF
END_PROGRAM
";

const UNKNOWN_NAME_PROGRAM: &str = "\
PROGRAM Main
VAR x : DINT; END_VAR
  x := Missing();
END_PROGRAM
";

#[test]
fn new_scaffolds_a_buildable_project() {
    let project = TestDir::new("new-scaffold");
    let output = project.run(".", &["new", "app"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    for file in ["stc.toml", "src/main.st", "tests/smoke.st", ".gitignore"] {
        assert!(
            project.path(&format!("app/{file}")).is_file(),
            "missing scaffold file `{file}`"
        );
    }
    let manifest = std::fs::read_to_string(project.path("app/stc.toml")).expect("manifest");
    assert!(manifest.contains("name = \"app\""), "{manifest}");
    assert!(manifest.contains("[test]"), "{manifest}");
    assert!(
        std::fs::read_to_string(project.path("app/.gitignore"))
            .expect(".gitignore")
            .contains("/target/")
    );

    let output = project.run("app", &["check"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        project.path("app/stc.lock").is_file(),
        "stc.lock is written"
    );

    let output = project.run("app", &["build", "--emit", "ir,metadata"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let ir = project.path("app/target/stc/debug/app.ll");
    let meta = project.path("app/target/stc/debug/app.stc-meta.json");
    assert!(ir.is_file(), "default out-dir is target/stc/debug");
    let metadata = std::fs::read_to_string(&meta).expect("metadata");
    // Production builds never load `[test].sources`.
    assert!(!metadata.contains("smoke.st"), "{metadata}");
}

#[test]
fn new_rejects_existing_invalid_and_mixed_paths() {
    let project = TestDir::new("new-errors");
    scaffold(&project, "app");
    let output = project.run(".", &["new", "app"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    let output = project.run(".", &["new", "9bad"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    let output = project.run(".", &["new", "app", "extra"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    let output = project.run(".", &["new", "other", "--manifest-path", "stc.toml"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
}

#[test]
fn project_build_defaults_and_emit_overrides() {
    let project = TestDir::new("build-defaults");
    scaffold(&project, "app");

    // Default: package name, target/stc/debug, manifest emit (module) unless
    // the CLI selects another kind.
    let output = project.run("app", &["build", "--emit", "ir"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(project.path("app/target/stc/debug/app.ll").is_file());

    let output = project.run("app", &["build", "--emit", "ir", "--name", "other"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(project.path("app/target/stc/debug/other.ll").is_file());

    // `[build].out-dir` is relative to the manifest.
    std::fs::write(
        project.path("app/stc.toml"),
        app_manifest("app", "out-dir = \"out\""),
    )
    .expect("rewrite manifest");
    let output = project.run("app", &["build", "--emit", "ir"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(project.path("app/out/app.ll").is_file());

    // Manifest emit is used when the CLI does not override it.
    std::fs::write(
        project.path("app/stc.toml"),
        app_manifest("app", "emit = [\"metadata\"]\nout-dir = \"out2\""),
    )
    .expect("rewrite manifest");
    let output = project.run("app", &["build"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(project.path("app/out2/app.stc-meta.json").is_file());
    assert!(!project.path("app/out2/app.ll").exists());

    // The CLI emit list overrides the manifest.
    std::fs::write(
        project.path("app/stc.toml"),
        app_manifest("app", "emit = [\"metadata\"]\nout-dir = \"out3\""),
    )
    .expect("rewrite manifest");
    let output = project.run("app", &["build", "--emit", "ir"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(project.path("app/out3/app.ll").is_file());
    assert!(!project.path("app/out3/app.stc-meta.json").exists());
}

#[test]
fn manifest_syntax_controls_relaxed_parsing() {
    let project = TestDir::new("syntax");
    project.write("app/stc.toml", &app_manifest("app", ""));
    project.write("app/src/main.st", RELAXED_PROGRAM);

    let output = project.run("app", &["check"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("STC-E-GRAM-107"),
        "{}",
        stderr(&output)
    );

    project.write("app/stc.toml", &app_manifest("app", "syntax = \"relaxed\""));
    let output = project.run("app", &["check"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));

    // `--relaxed` overrides a strict manifest.
    project.write("app/stc.toml", &app_manifest("app", "syntax = \"strict\""));
    let output = project.run("app", &["check", "--relaxed"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

#[test]
fn locked_requires_a_matching_lockfile() {
    let project = TestDir::new("locked");
    scaffold(&project, "app");

    // `stc new` does not create a lockfile.
    let output = project.run("app", &["check", "--locked"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("STC-E-LOCK-001"),
        "{}",
        stderr(&output)
    );

    let output = project.run("app", &["check"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let output = project.run("app", &["check", "--locked"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));

    let manifest = std::fs::read_to_string(project.path("app/stc.toml"))
        .expect("manifest")
        .replace("version = \"0.1.0\"", "version = \"0.2.0\"");
    std::fs::write(project.path("app/stc.toml"), manifest).expect("rewrite manifest");
    let output = project.run("app", &["check", "--locked"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("STC-E-LOCK-002"),
        "{}",
        stderr(&output)
    );

    let output = project.run("app", &["check"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let output = project.run("app", &["check", "--locked"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

#[test]
fn out_dir_containment_and_cli_override() {
    let project = TestDir::new("out-dir");
    project.write("app/src/main.st", SCAFFOLD_PROGRAM);
    project.write(
        "app/stc.toml",
        &app_manifest("app", "out-dir = \"../escape\""),
    );
    let output = project.run("app", &["check"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("STC-E-PROJECT-010"),
        "{}",
        stderr(&output)
    );

    std::fs::write(
        project.path("app/stc.toml"),
        app_manifest("app", "out-dir = \"/absolute\""),
    )
    .expect("rewrite manifest");
    let output = project.run("app", &["check"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));

    // A CLI out-dir may point outside the project.
    std::fs::write(project.path("app/stc.toml"), app_manifest("app", "")).expect("rewrite");
    let output = project.run("app", &["build", "--emit", "ir", "--out-dir", "../cli-out"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(project.path("cli-out/app.ll").is_file());
}

#[test]
fn toolchain_precedence_cli_env_manifest() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping toolchain precedence test");
        return;
    };
    let project = TestDir::new("toolchain");
    project.write("app/src/main.st", SCAFFOLD_PROGRAM);
    project.write(
        "app/stc.toml",
        &app_manifest("app", "clang = \"no-such-clang\""),
    );

    // Environment beats the manifest.
    let output = project.run_env(
        "app",
        &["build", "--emit", "module"],
        &[("STC_CLANG", clang)],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));

    // ... and an invalid environment value wins over a valid manifest value.
    std::fs::write(
        project.path("app/stc.toml"),
        app_manifest("app", &format!("clang = \"{clang}\"")),
    )
    .expect("rewrite manifest");
    let output = project.run_env(
        "app",
        &["build", "--emit", "module"],
        &[("STC_CLANG", "no-such-clang")],
    );
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("STC-E-BUILD-001"),
        "{}",
        stderr(&output)
    );

    // The CLI beats both.
    let output = project.run_env(
        "app",
        &["build", "--emit", "module", "--clang", clang],
        &[("STC_CLANG", "no-such-clang")],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));

    // Target precedence: CLI > environment > manifest.
    std::fs::write(
        project.path("app/stc.toml"),
        app_manifest("app", "target = \"manifest-triple\""),
    )
    .expect("rewrite manifest");
    let output = project.run_env(
        "app",
        &["metadata", "--message-format", "json"],
        &[("STC_TARGET", "env-triple")],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("\"target\":\"env-triple\""), "{text}");

    let output = project.run_env(
        "app",
        &[
            "metadata",
            "--message-format",
            "json",
            "--target",
            host_triple(),
        ],
        &[("STC_TARGET", "env-triple")],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains(&format!("\"target\":\"{}\"", host_triple())),
        "{text}"
    );

    let output = project.run_env(
        "app",
        &["metadata", "--message-format", "json"],
        &[("STC_TARGET", "")],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("\"target\":\"manifest-triple\""), "{text}");

    // Runtime precedence: CLI > environment > manifest.
    let Some((static_library, _)) = runtime_pair() else {
        eprintln!("note: the runtime host archive is unavailable; skipping runtime precedence");
        return;
    };
    let runtime = static_library.to_str().unwrap();
    std::fs::write(
        project.path("app/stc.toml"),
        app_manifest(
            "app",
            "emit = [\"lib\"]\nlib-type = \"static\"\nruntime = \"no-such-runtime\"",
        ),
    )
    .expect("rewrite manifest");
    let output = project.run("app", &["build", "--clang", clang]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("STC-E-BUILD-003"),
        "{}",
        stderr(&output)
    );

    let output = project.run_env(
        "app",
        &["build", "--clang", clang],
        &[("STC_RUNTIME", runtime)],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));

    let output = project.run_env(
        "app",
        &["build", "--clang", clang, "--runtime", runtime],
        &[("STC_RUNTIME", "no-such-runtime")],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

#[test]
fn shared_and_static_host_libraries() {
    let Some((static_library, shared_library)) = runtime_pair() else {
        eprintln!("note: clang or the runtime host artifacts are unavailable; skipping");
        return;
    };
    let clang = clang().expect("clang checked above");
    let project = TestDir::new("host-libraries");
    project.write("app/src/main.st", SCAFFOLD_PROGRAM);
    project.write(
        "app/stc.toml",
        &app_manifest("app", "emit = [\"lib\"]\nlib-type = \"static\""),
    );
    let output = project.run(
        "app",
        &[
            "build",
            "--clang",
            clang,
            "--runtime",
            static_library.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let static_name = if cfg!(windows) { "app.lib" } else { "app.a" };
    assert!(
        project
            .path(&format!("app/target/stc/debug/{static_name}"))
            .is_file()
    );

    std::fs::write(
        project.path("app/stc.toml"),
        app_manifest("app", "emit = [\"lib\"]\nlib-type = \"shared\""),
    )
    .expect("rewrite manifest");
    let output = project.run(
        "app",
        &[
            "build",
            "--clang",
            clang,
            "--runtime",
            shared_library.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let shared_name = if cfg!(windows) {
        "app.dll"
    } else if cfg!(target_os = "macos") {
        "libapp.dylib"
    } else {
        "libapp.so"
    };
    assert!(
        project
            .path(&format!("app/target/stc/debug/{shared_name}"))
            .is_file()
    );
    // Generated code stays in the separate module artifact.
    let has_module = std::fs::read_dir(project.path("app/target/stc/debug"))
        .expect("out-dir")
        .flatten()
        .any(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with("app-")
                && (name.ends_with(".dll") || name.ends_with(".so"))
                && name.len() > "app-.dll".len()
        });
    assert!(
        has_module,
        "the loadable module is emitted next to the host library"
    );
}

#[test]
fn message_formats_and_colors() {
    let project = TestDir::new("formats");
    project.write("bad.st", UNKNOWN_NAME_PROGRAM);

    let human = project.run(".", &["check", "bad.st", "--color", "never"]);
    assert_eq!(human.status.code(), Some(1));
    assert!(stderr(&human).contains('^'), "{}", stderr(&human));
    assert!(!stderr(&human).contains('\u{1b}'));

    let short = project.run(
        ".",
        &[
            "check",
            "bad.st",
            "--message-format",
            "short",
            "--color",
            "never",
        ],
    );
    assert_eq!(short.status.code(), Some(1));
    let short_text = stderr(&short);
    assert_eq!(short_text.lines().count(), 1, "{short_text}");
    assert!(short_text.contains("bad.st:3:"), "{short_text}");
    assert!(short_text.contains("error[STC-E-NAME-001]"), "{short_text}");
    assert!(!short_text.contains('^'), "{short_text}");

    let color = project.run(
        ".",
        &[
            "check",
            "bad.st",
            "--color",
            "always",
            "--message-format",
            "short",
        ],
    );
    assert!(
        stderr(&color).contains("\u{1b}[1;31m"),
        "{:?}",
        stderr(&color)
    );

    let auto = project.run(".", &["check", "bad.st"]);
    assert!(
        !stderr(&auto).contains('\u{1b}'),
        "piped stderr is not colored"
    );

    let json = project.run(
        ".",
        &[
            "check",
            "bad.st",
            "--message-format",
            "json",
            "--color",
            "always",
        ],
    );
    assert_eq!(json.status.code(), Some(1));
    let json_text = stdout(&json);
    assert_eq!(json_text.lines().count(), 1, "{json_text}");
    assert!(json_text.contains("\"severity\":\"error\""), "{json_text}");
    assert!(!json_text.contains('\u{1b}'), "{json_text}");
    assert!(stderr(&json).is_empty(), "{}", stderr(&json));
}

#[test]
fn quiet_and_verbose_output() {
    let project = TestDir::new("verbosity");
    project.write("app/stc.toml", &app_manifest("app", "debug = false"));
    project.write("app/src/main.st", SCAFFOLD_PROGRAM);

    let default = project.run("app", &["check"]);
    assert_eq!(default.status.code(), Some(0));
    assert!(
        stderr(&default).contains("STC-W-PROJECT-001"),
        "{}",
        stderr(&default)
    );

    let quiet = project.run("app", &["check", "-q"]);
    assert_eq!(quiet.status.code(), Some(0));
    assert!(stderr(&quiet).is_empty(), "{}", stderr(&quiet));

    let verbose = project.run("app", &["check", "-v"]);
    let text = stderr(&verbose);
    for expected in [
        "[stc] profile",
        "[stc] sources",
        "[stc] lex",
        "[stc] analyze",
    ] {
        assert!(text.contains(expected), "missing `{expected}` in:\n{text}");
    }
    assert!(text.contains("STC-W-PROJECT-001"), "{text}");

    let trace = project.run("app", &["check", "-vv"]);
    let text = stderr(&trace);
    assert!(text.contains("app/src/main.st lex"), "{text}");
    assert!(text.contains("parse"), "{text}");

    let quiet_verbose = project.run("app", &["check", "-q", "-vv"]);
    assert!(
        stderr(&quiet_verbose).is_empty(),
        "{}",
        stderr(&quiet_verbose)
    );

    let build = project.run("app", &["build", "--emit", "ir", "-v"]);
    assert_eq!(build.status.code(), Some(0), "{}", stderr(&build));
    let text = stderr(&build);
    assert!(text.contains("[stc] codegen"), "{text}");
    assert!(text.contains("[stc] artifacts"), "{text}");

    let build_trace = project.run("app", &["build", "--emit", "ir", "-vv"]);
    let text = stderr(&build_trace);
    assert!(text.contains("[stc] ir target/stc/debug/app.ll"), "{text}");

    if let Some(clang) = clang() {
        let output = project.run("app", &["build", "--emit", "obj", "-vv", "--clang", clang]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let text = stderr(&output);
        assert!(text.contains("+ "), "{text}");
        assert!(text.contains("clang"), "{text}");

        // A CLI `--debug` is accepted; M6 still builds the debug form.
        let output = project.run(
            "app",
            &["build", "--emit", "obj", "--debug", "--clang", clang],
        );
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    }

    // Errors still print under `-q`.
    project.write("app/src/main.st", UNKNOWN_NAME_PROGRAM);
    let output = project.run("app", &["check", "-q"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("STC-E-NAME-001"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn project_emits_obj_exe_and_opt_levels() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping object/executable test");
        return;
    };
    let project = TestDir::new("artifacts");
    project.write("app/src/main.st", SCAFFOLD_PROGRAM);
    project.write(
        "app/stc.toml",
        &app_manifest("app", "emit = [\"obj\"]\nopt-level = 2"),
    );
    let output = project.run("app", &["build", "-vv", "--clang", clang]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(project.path("app/target/stc/debug/app.o").is_file());
    let text = stderr(&output);
    assert!(text.contains("-O2"), "opt-level reaches clang:\n{text}");

    let Some((static_library, _)) = runtime_pair() else {
        eprintln!("note: the runtime host archive is unavailable; skipping executable");
        return;
    };
    std::fs::write(
        project.path("app/stc.toml"),
        app_manifest("app", "emit = [\"exe\"]"),
    )
    .expect("rewrite manifest");
    let output = project.run(
        "app",
        &[
            "build",
            "--clang",
            clang,
            "--runtime",
            static_library.to_str().unwrap(),
            "-v",
        ],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let exe_name = if cfg!(windows) { "app.exe" } else { "app" };
    assert!(
        project
            .path(&format!("app/target/stc/debug/{exe_name}"))
            .is_file()
    );
    assert!(
        project
            .path("app/target/stc/debug/app.stc-got.json")
            .is_file(),
        "the launcher is paired with the module manifest"
    );
    assert!(
        project
            .path("app/target/stc/debug/app.stc-meta.json")
            .is_file()
    );
    let text = stderr(&output);
    assert!(text.contains("[stc] clang"), "{text}");
}

#[test]
fn lint_levels_and_overrides() {
    let project = TestDir::new("lints");
    project.write("app/stc.toml", &app_manifest("app", "debug = false"));
    project.write("app/src/main.st", SCAFFOLD_PROGRAM);

    let denied = project.run("app", &["check", "-D", "warnings"]);
    assert_eq!(denied.status.code(), Some(1), "{}", stderr(&denied));
    assert!(
        stderr(&denied).contains("error[STC-W-PROJECT-001]"),
        "{}",
        stderr(&denied)
    );

    let deny_flag = project.run("app", &["check", "--deny", "warnings"]);
    assert_eq!(deny_flag.status.code(), Some(1), "{}", stderr(&deny_flag));

    let allowed = project.run("app", &["check", "-A", "warnings"]);
    assert_eq!(allowed.status.code(), Some(0), "{}", stderr(&allowed));
    assert!(!stderr(&allowed).contains("STC-W-PROJECT-001"));

    let code_deny = project.run("app", &["check", "-D", "STC-W-PROJECT-001"]);
    assert_eq!(code_deny.status.code(), Some(1), "{}", stderr(&code_deny));
    assert!(stderr(&code_deny).contains("STC-W-PROJECT-001"));

    let unknown = project.run("app", &["check", "-D", "NOT-A-CODE"]);
    assert_eq!(unknown.status.code(), Some(2), "{}", stderr(&unknown));

    let downgrade = project.run("app", &["check", "-A", "STC-E-NAME-001"]);
    assert_eq!(downgrade.status.code(), Some(2), "{}", stderr(&downgrade));

    // Manifest levels apply below CLI overrides.
    let mut manifest = app_manifest("app", "debug = false");
    manifest.push_str("\n[lints]\n\"STC-W-PROJECT-001\" = \"deny\"\n");
    project.write("app/stc.toml", &manifest);
    let output = project.run("app", &["check"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("error[STC-W-PROJECT-001]"),
        "{}",
        stderr(&output)
    );

    let output = project.run("app", &["check", "-A", "STC-W-PROJECT-001"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(!stderr(&output).contains("STC-W-PROJECT-001"));
}

#[test]
fn project_io_metadata_and_binding_validation() {
    let project = TestDir::new("project-io");
    project.write(
        "app/stc.toml",
        "schema = 1\n\
         \n\
         [package]\n\
         name = \"ioapp\"\n\
         version = \"0.1.0\"\n\
         language = \"stc-2013-oop-utf8-v1\"\n\
         \n\
         [project]\n\
         sources = [\"src/**/*.st\"]\n\
         \n\
         [io]\n\
         backend = \"memory\"\n\
         \n\
         [[io.devices]]\n\
         name = \"drive1\"\n\
         kind = \"ethercat\"\n\
         config = { vendor = \"0x1234\", revision = 3 }\n\
         \n\
         [[io.devices.channels]]\n\
         name = \"DI5\"\n\
         direction = \"input\"\n\
         width = 1\n\
         \n\
         [[io.bindings]]\n\
         variable = \"inst.input\"\n\
         device = \"drive1\"\n\
         channel = \"DI5\"\n\
         direction = \"input\"\n\
         \n\
         [retain]\n\
         file = \"state/app.stcstate\"\n\
         on-mismatch = \"reset\"\n",
    );
    project.write(
        "app/src/main.st",
        "\
PROGRAM Main
VAR
  input AT %IX0.0 : BOOL {stc:io {\"device\":\"hintdev\",\"channel\":1,\"direction\":\"input\"}};
  output AT %QW0 : WORD;
END_VAR
VAR RETAIN
  saved : DINT;
END_VAR
VAR_ACCESS
  visible : input : BOOL READ_ONLY;
END_VAR
  output := 16#1234;
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
",
    );
    let output = project.run("app", &["build", "--emit", "metadata"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("STC-W-IO-003"),
        "the project binding overrides the pragma: {}",
        stderr(&output)
    );
    let metadata =
        std::fs::read_to_string(project.path("app/target/stc/debug/ioapp.stc-meta.json"))
            .expect("metadata sidecar");
    for expected in [
        "\"backend\": \"memory\"",
        "\"source\": \"project\"",
        "\"device\": \"drive1\"",
        "\"channel\": \"DI5\"",
        "\"var_access\": [",
        "\"name\": \"visible\"",
        "\"path\": \"inst.input\"",
        "\"read_only\": true",
        "\"revision\": 3",
        "\"vendor\": \"0x1234\"",
        "\"direct_id\": \"0x",
        "\"got_id\": \"0x",
        "\"enabled\": true",
        "\"file\": \"state/app.stcstate\"",
        "\"on_mismatch\": \"reset\"",
    ] {
        assert!(
            metadata.contains(expected),
            "missing {expected}:\n{metadata}"
        );
    }

    // An unresolved project binding is STC-E-PROJECT-009 and exits 2.
    let manifest = std::fs::read_to_string(project.path("app/stc.toml"))
        .expect("manifest")
        .replace("variable = \"inst.input\"", "variable = \"inst.missing\"");
    std::fs::write(project.path("app/stc.toml"), manifest).expect("rewrite manifest");
    let output = project.run("app", &["build", "--emit", "metadata"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("STC-E-PROJECT-009"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn metadata_summary_shapes() {
    let project = TestDir::new("metadata-shape");
    project.write(
        "app/stc.toml",
        "schema = 1\n\
         \n\
         [package]\n\
         name = \"meta\"\n\
         version = \"0.1.0\"\n\
         language = \"stc-2013-oop-utf8-v1\"\n\
         \n\
         [project]\n\
         sources = [\"src/**/*.st\"]\n\
         \n\
         [io]\n\
         backend = \"file\"\n\
         file = \"io.bin\"\n\
         \n\
         [[io.devices]]\n\
         name = \"drive1\"\n\
         kind = \"ethercat\"\n\
         \n\
         [retain]\n\
         file = \"state/meta.stcstate\"\n\
         on-mismatch = \"reset\"\n",
    );
    project.write(
        "app/src/main.st",
        "\
PROGRAM Main
VAR x : DINT; END_VAR
  x := x + 1;
END_PROGRAM

FUNCTION Plus : DINT
VAR_INPUT a : DINT; b : DINT; END_VAR
  Plus := a + b;
END_FUNCTION

CONFIGURATION Default
RESOURCE Cpu ON CPU
  TASK Fast (INTERVAL := T#1ms, PRIORITY := 1);
  PROGRAM inst WITH Fast : Main;
END_RESOURCE
END_CONFIGURATION
",
    );
    let output = project.run("app", &["metadata", "--message-format", "json"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let text = stdout(&output);
    let summary = find_object(&text, "\"profile\":");
    assert!(summary.contains("\"abi_version\":2"), "{summary}");
    assert!(summary.contains("\"artifact\":\"meta\""), "{summary}");
    assert!(
        summary.contains("\"configuration\":\"Default\""),
        "{summary}"
    );
    assert!(
        summary.contains(&format!("\"target\":\"{}\"", host_triple())),
        "{summary}"
    );
    assert!(summary.contains("\"tasks\":[{"), "{summary}");
    assert!(summary.contains("\"interval_ns\":1000000"), "{summary}");
    assert!(summary.contains("\"priority\":1"), "{summary}");
    assert!(summary.contains("\"single\":false"), "{summary}");
    assert!(
        summary.contains(
            "\"retain\":{\"enabled\":true,\"file\":\"state/meta.stcstate\",\"on_mismatch\":\"reset\"}"
        ),
        "{summary}"
    );
    assert!(
        summary.contains(
            "\"io\":{\"backend\":\"file\",\"bindings\":0,\"devices\":1,\"var_access\":0}"
        ),
        "{summary}"
    );
    assert!(summary.contains("\"artifacts\":[{"), "{summary}");
    assert!(summary.contains("\"kind\":\"module\""), "{summary}");

    let human = project.run("app", &["metadata"]);
    assert_eq!(human.status.code(), Some(0), "{}", stderr(&human));
    let text = stdout(&human);
    assert!(text.contains("stc metadata"), "{text}");
    assert!(text.contains("Fast resource Cpu"), "{text}");
    assert!(text.contains("state/meta.stcstate"), "{text}");
    assert!(text.contains("artifacts"), "{text}");
}

#[test]
fn compliance_summary_shapes() {
    let project = TestDir::new("compliance-shape");
    let output = project.run(".", &["compliance", "--message-format", "json"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let text = stdout(&output);
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(
        text.contains("\"profile\":\"stc-2013-oop-utf8-v1\""),
        "{text}"
    );
    assert!(text.contains("\"edition\":\"IEC 61131-3:2013\""), "{text}");
    assert!(text.contains("\"ed4_claimed\":false"), "{text}");
    assert!(
        text.contains(
            "\"runtime_capabilities\":[\"online-change\",\"got-manifest\",\"debug-monitoring\"]"
        ),
        "{text}"
    );
    assert!(text.contains("\"safety\":\"not-a-safety-plc\""), "{text}");
    assert!(
        text.contains("\"project-system\":\"implemented\""),
        "{text}"
    );
    assert!(text.contains("\"debug-monitoring\":\"planned\""), "{text}");
    assert!(text.contains("\"extensions\":[\"STC-UTF8\""), "{text}");

    let human = project.run(".", &["compliance"]);
    assert_eq!(human.status.code(), Some(0), "{}", stderr(&human));
    let text = stdout(&human);
    assert!(text.contains("stc compliance"), "{text}");
    assert!(
        text.contains("This project is not a safety PLC and assumes no safety responsibility."),
        "{text}"
    );
}

#[test]
fn compatibility_report_for_baseline_builds() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping compatibility report test");
        return;
    };
    let project = TestDir::new("compatibility");
    scaffold(&project, "app");
    let output = project.run("app", &["build", "--emit", "module", "--clang", clang]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let manifest_path = project.path("app/target/stc/debug/app.stc-got.json");
    let baseline = project.path("baseline.json");
    std::fs::copy(&manifest_path, &baseline).expect("copy baseline");

    let output = project.run(
        "app",
        &[
            "build",
            "--emit",
            "module",
            "--clang",
            clang,
            "--got-manifest",
            baseline.to_str().unwrap(),
            "--message-format",
            "json",
        ],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let text = stdout(&output);
    let report = find_object(&text, "\"verdict\":");
    assert!(report.contains("\"verdict\":\"compatible\""), "{report}");
    assert!(report.contains("\"baseline_build_id\":\"0x"), "{report}");
    assert!(report.contains("\"candidate_build_id\":\"0x"), "{report}");
    for key in ["preserved", "allocated", "dropped", "retired", "rejected"] {
        assert!(report.contains(&format!("\"{key}\":")), "{report}");
    }

    let human = project.run(
        "app",
        &[
            "build",
            "--emit",
            "module",
            "--clang",
            clang,
            "--got-manifest",
            baseline.to_str().unwrap(),
            "-v",
        ],
    );
    assert_eq!(human.status.code(), Some(0), "{}", stderr(&human));
    let text = stderr(&human);
    assert!(text.contains("compatibility compatible"), "{text}");
    assert!(text.contains("preserved"), "{text}");
}

#[test]
fn unimplemented_verbs_and_usage_errors() {
    let project = TestDir::new("usage");
    project.write("bad.st", UNKNOWN_NAME_PROGRAM);
    for arguments in [
        vec!["run"],
        vec!["run", "--watch"],
        vec!["test"],
        vec!["fmt", "--check"],
        vec!["online", "download"],
        vec!["online", "diff"],
        vec!["rename", "A", "B"],
        vec!["build", "--release"],
        vec!["unknown-command"],
        vec!["check", "bad.st", "--locked"],
        vec!["metadata", "extra"],
        vec!["check", "bad.st", "--not-an-option"],
    ] {
        let output = project.run(".", &arguments);
        assert_eq!(
            output.status.code(),
            Some(2),
            "`{arguments:?}` should be a usage error:\nstdout:\n{}\nstderr:\n{}",
            stdout(&output),
            stderr(&output)
        );
    }
}

#[test]
fn clean_preserves_lockfile_and_state() {
    let project = TestDir::new("clean");
    scaffold(&project, "app");
    let output = project.run("app", &["build", "--emit", "ir"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let out_dir = project.path("app/target/stc/debug");
    std::fs::write(out_dir.join("junk.txt"), "junk").expect("write junk");
    std::fs::write(out_dir.join("app.stcstate"), "state").expect("write state");

    let output = project.run("app", &["clean"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(!out_dir.join("junk.txt").exists());
    assert!(!out_dir.join("app.ll").exists());
    assert!(
        out_dir.join("app.stcstate").is_file(),
        "state files are kept"
    );
    assert!(project.path("app/stc.lock").is_file(), "stc.lock is kept");

    let output = project.run("app", &["clean"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));

    // `--manifest-path` resolves the out-dir under the project, not the CWD.
    std::fs::write(out_dir.join("junk2.txt"), "junk").expect("write junk");
    let output = project.run(".", &["clean", "--manifest-path", "app"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(!out_dir.join("junk2.txt").exists());
    assert!(out_dir.join("app.stcstate").is_file());

    // A relative CLI out-dir is normalized before preservation checks.
    std::fs::write(out_dir.join("junk3.txt"), "junk").expect("write junk");
    let output = project.run("app", &["clean", "--out-dir", "target/stc/debug"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(!out_dir.join("junk3.txt").exists());
    assert!(out_dir.join("app.stcstate").is_file());

    // A CLI out-dir that contains the project root is refused.
    for out_dir_override in [".", ".."] {
        let output = project.run("app", &["clean", "--out-dir", out_dir_override]);
        assert_eq!(
            output.status.code(),
            Some(2),
            "`--out-dir {out_dir_override}` should be refused:\n{}",
            stderr(&output)
        );
        assert!(project.path("app/stc.toml").is_file());
        assert!(project.path("app/src/main.st").is_file());
    }

    // `--out-dir` selects another tree.
    let other = project.path("app/other");
    std::fs::create_dir_all(&other).expect("create out-dir");
    std::fs::write(other.join("file"), "x").expect("write");
    let output = project.run("app", &["clean", "--out-dir", "other"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(!other.exists());

    // Cleaning the project root is refused.
    std::fs::write(
        project.path("app/stc.toml"),
        app_manifest("app", "out-dir = \".\""),
    )
    .expect("rewrite manifest");
    let output = project.run("app", &["clean"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(project.path("app/stc.toml").is_file());
}
