//! M7 manifest and online-change hardening (D-192).
//!
//! These tests lock the documented failure paths: a format-1 baseline is
//! rejected and never reinterpreted; debug and release baselines are treated
//! explicitly rather than as an implied hot-swap; truncated and tampered
//! manifests are rejected before they can influence a build.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use stc_got::{Manifest, PlanVerdict};

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
                    eprintln!("note: clang not found; skipping manifest hardening tests");
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
            "stc-manifest-{}-{name}-{unique}",
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

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.path(relative)).expect("read file")
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(stc())
            .args(args)
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

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn project_manifest() -> &'static str {
    r#"schema = 1

[package]
name = "app"
version = "0.1.0"
language = "stc-2013-oop-utf8-v1"

[project]
sources = ["src/**/*.st"]
"#
}

const SOURCE: &str = "\
PROGRAM Main
VAR
  Count : DINT;
END_VAR
  Count := Count + 1;
END_PROGRAM
";

fn build_mode(
    project: &TestDir,
    clang: &str,
    release: bool,
    out: &str,
    baseline: Option<&Path>,
) -> Output {
    let mut args = vec![
        "build",
        "--manifest-path",
        "stc.toml",
        "--emit",
        "module",
        "--clang",
        clang,
        "--out-dir",
        out,
    ];
    if release {
        args.push("--release");
    }
    let baseline_text;
    if let Some(baseline) = baseline {
        baseline_text = baseline.to_string_lossy().into_owned();
        args.push("--got-manifest");
        args.push(&baseline_text);
    }
    project.run(&args)
}

fn read_manifest(path: &Path) -> Manifest {
    let text = std::fs::read_to_string(path).expect("manifest text");
    Manifest::decode_json(&text).expect("decode manifest")
}

fn project() -> TestDir {
    let project = TestDir::new("app");
    project.write("stc.toml", project_manifest());
    project.write("src/main.st", SOURCE);
    project
}

#[test]
fn format_one_baseline_is_rejected_and_never_upgraded() {
    let Some(clang) = clang() else {
        return;
    };
    let project = project();
    let first = build_mode(&project, clang, false, "out", None);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    let manifest = project.read("out/app.stc-got.json");
    assert!(manifest.contains("\"format\": 2"), "{manifest}");
    let format_one = manifest.replacen("\"format\": 2", "\"format\": 1", 1);
    project.write("format1.json", &format_one);

    let rejected = build_mode(
        &project,
        clang,
        false,
        "out2",
        Some(&project.path("format1.json")),
    );
    assert_eq!(rejected.status.code(), Some(1), "{}", stderr(&rejected));
    let text = stderr(&rejected);
    assert!(
        text.contains("unsupported manifest JSON format 1"),
        "unexpected format-1 diagnostic:\n{text}"
    );

    // `--cold` does not reinterpret the file: an explicit baseline is still
    // read and rejected. A cold build is a build without the baseline.
    let cold_with_baseline = project.run(&[
        "build",
        "--emit",
        "module",
        "--clang",
        clang,
        "--cold",
        "--got-manifest",
        "format1.json",
        "--out-dir",
        "cold",
    ]);
    assert_eq!(
        cold_with_baseline.status.code(),
        Some(1),
        "{}",
        stderr(&cold_with_baseline)
    );
    assert!(
        stderr(&cold_with_baseline).contains("unsupported manifest JSON format 1"),
        "{}",
        stderr(&cold_with_baseline)
    );

    let cold = project.run(&[
        "build",
        "--emit",
        "module",
        "--clang",
        clang,
        "--cold",
        "--out-dir",
        "cold-ok",
    ]);
    assert_eq!(cold.status.code(), Some(0), "{}", stderr(&cold));
    let cold_manifest = read_manifest(&project.path("cold-ok/app.stc-got.json"));
    assert_eq!(cold_manifest.plan.verdict, PlanVerdict::Cold);
}

#[test]
fn cross_mode_baselines_are_explicit() {
    let Some(clang) = clang() else {
        return;
    };
    let project = project();

    let debug = build_mode(&project, clang, false, "debug", None);
    assert_eq!(debug.status.code(), Some(0), "{}", stderr(&debug));
    let debug_baseline = project.path("debug-baseline.json");
    std::fs::copy(project.path("debug/app.stc-got.json"), &debug_baseline)
        .expect("copy debug baseline");

    // A release candidate cannot consume a hook-bearing debug baseline: the
    // baseline requires host ABI 2.3 while the release form provides 2.1, so
    // the engine selects a cold layout.
    let release_over_debug = build_mode(
        &project,
        clang,
        true,
        "release-over-debug",
        Some(&debug_baseline),
    );
    assert_eq!(
        release_over_debug.status.code(),
        Some(0),
        "{}",
        stderr(&release_over_debug)
    );
    let text = stderr(&release_over_debug);
    assert!(
        text.contains("STC-W-ONLINE-005") && text.contains("host ABI minor 3"),
        "release-over-debug did not select the documented cold path:\n{text}"
    );
    let release_manifest = read_manifest(&project.path("release-over-debug/app.stc-got.json"));
    assert_eq!(release_manifest.plan.verdict, PlanVerdict::Cold);

    // The reverse direction is classified compatible by the engine. It is
    // documented as an unsupported deployment path, not as a hot-swap
    // promise: keep debug and release stores separate.
    let release = build_mode(&project, clang, true, "release", None);
    assert_eq!(release.status.code(), Some(0), "{}", stderr(&release));
    let release_baseline = project.path("release-baseline.json");
    std::fs::copy(project.path("release/app.stc-got.json"), &release_baseline)
        .expect("copy release baseline");
    let debug_over_release = build_mode(
        &project,
        clang,
        false,
        "debug-over-release",
        Some(&release_baseline),
    );
    assert_eq!(
        debug_over_release.status.code(),
        Some(0),
        "{}",
        stderr(&debug_over_release)
    );
    let debug_manifest = read_manifest(&project.path("debug-over-release/app.stc-got.json"));
    assert_eq!(
        debug_manifest.plan.verdict,
        PlanVerdict::OnlineCandidate,
        "the documented engine classification changed"
    );

    let diff_release = project.run(&[
        "online",
        "diff",
        "release-over-debug/app.stc-got.json",
        "--baseline",
        "debug-baseline.json",
    ]);
    assert_eq!(
        diff_release.status.code(),
        Some(1),
        "{}",
        stderr(&diff_release)
    );
    let diff_debug = project.run(&[
        "online",
        "diff",
        "debug-over-release/app.stc-got.json",
        "--baseline",
        "release-baseline.json",
    ]);
    assert_eq!(diff_debug.status.code(), Some(0), "{}", stderr(&diff_debug));
}

#[test]
fn truncated_and_tampered_manifests_are_rejected() {
    let Some(clang) = clang() else {
        return;
    };
    let project = project();
    let first = build_mode(&project, clang, false, "out", None);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    let manifest = project.read("out/app.stc-got.json");
    std::fs::copy(
        project.path("out/app.stc-got.json"),
        project.path("baseline.json"),
    )
    .expect("copy baseline");

    let bytes = manifest.as_bytes();
    std::fs::write(project.path("truncated.json"), &bytes[..bytes.len() / 2])
        .expect("write truncated manifest");
    let diff = project.run(&[
        "online",
        "diff",
        "baseline.json",
        "--baseline",
        "truncated.json",
    ]);
    assert_eq!(diff.status.code(), Some(2), "{}", stderr(&diff));
    assert!(!stderr(&diff).is_empty(), "missing truncation diagnostic");

    // Flip one hex digit in a layout hash: the JSON stays well formed and the
    // digest check must still reject it.
    let marker = "\"layout_hash\": \"0x";
    let index = manifest.find(marker).expect("layout hash in manifest");
    let digit = index + marker.len();
    let mut tampered = manifest.clone();
    let replacement = if &tampered[digit..digit + 1] == "0" {
        "1"
    } else {
        "0"
    };
    tampered.replace_range(digit..digit + 1, replacement);
    assert_ne!(tampered, manifest);
    project.write("tampered.json", &tampered);
    let diff = project.run(&[
        "online",
        "diff",
        "baseline.json",
        "--baseline",
        "tampered.json",
    ]);
    assert_eq!(diff.status.code(), Some(2), "{}", stderr(&diff));
    let text = stderr(&diff);
    assert!(
        text.contains("digest") || text.contains("invalid") || text.contains("checksum"),
        "unexpected tamper diagnostic:\n{text}"
    );
}
