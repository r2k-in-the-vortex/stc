//! Online-change driver tests (M6 phase 8): `stc rename`,
//! `stc online download/diff`, and `stc run --watch`.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use stc_got::{Manifest, PlanVerdict};
use stc_runtime::store::{Download, ManifestStore, StoredManifest};

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
        let path =
            std::env::temp_dir().join(format!("stc-online-{}-{name}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test directory");
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
        std::fs::write(&path, contents).expect("write test file");
        path
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.path(relative)).expect("read test file")
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

fn project_manifest(name: &str, extra: &str) -> String {
    format!(
        r#"schema = 1

[package]
name = "{name}"
version = "0.1.0"
language = "stc-2013-oop-utf8-v1"

[project]
sources = ["src/**/*.st"]
{extra}
"#
    )
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn build_module(project: &TestDir, clang: &str, baseline: Option<&Path>) -> Output {
    let mut args = vec![
        "build",
        "--manifest-path",
        "stc.toml",
        "--emit",
        "module",
        "--clang",
        clang,
        "--out-dir",
        "out",
    ];
    let baseline_text;
    if let Some(baseline) = baseline {
        baseline_text = baseline.to_string_lossy().into_owned();
        args.push("--got-manifest");
        args.push(&baseline_text);
    }
    project.run(".", &args)
}

fn read_manifest(path: &Path) -> Manifest {
    let text = std::fs::read_to_string(path).expect("manifest text");
    Manifest::decode_json(&text).expect("decode manifest")
}

fn root_id(manifest: &Manifest, suffix: &str) -> stc_got::GotId {
    manifest
        .roots
        .iter()
        .find(|root| {
            let path = root.key.split('#').next().unwrap_or(&root.key);
            path.eq_ignore_ascii_case(suffix) || path.ends_with(suffix)
        })
        .unwrap_or_else(|| panic!("no root ending in `{suffix}`: {:?}", keys(manifest)))
        .got_id
}

fn keys(manifest: &Manifest) -> Vec<&str> {
    manifest
        .roots
        .iter()
        .map(|root| root.key.as_str())
        .collect()
}

const RENAME_SOURCE: &str = "\
PROGRAM Main
VAR
  Speed : DINT; // Speed keeps its comment
  text : STRING[32];
END_VAR
  Speed := Speed + 1;
  text := 'Speed';
  // Speed stays in comments
END_PROGRAM
";

#[test]
fn rename_rewrites_identifiers_and_preserves_identity() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping rename test");
        return;
    };
    let project = TestDir::new("rename");
    project.write("stc.toml", &project_manifest("app", ""));
    project.write("src/main.st", RENAME_SOURCE);

    let first = build_module(&project, clang, None);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    let baseline = project.path("out/app.stc-got.json");
    let baseline_copy = project.path("baseline.json");
    std::fs::copy(&baseline, &baseline_copy).expect("copy baseline");
    let first_manifest = read_manifest(&baseline);
    let old_id = root_id(&first_manifest, "MAIN.SPEED");

    let dry = project.run(".", &["rename", "Main.Speed", "Main.Rate", "--dry-run"]);
    assert_eq!(dry.status.code(), Some(0), "{}", stderr(&dry));
    assert!(
        stdout(&dry).contains("Main.Speed -> Main.Rate"),
        "{}",
        stdout(&dry)
    );
    assert_eq!(project.read("src/main.st"), RENAME_SOURCE);
    assert!(!project.path("stc.renames.toml").exists());

    let rename = project.run(".", &["rename", "Main.Speed", "Main.Rate"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    let source = project.read("src/main.st");
    assert!(source.contains("Rate : DINT;"), "{source}");
    assert!(source.contains("Rate := Rate + 1;"), "{source}");
    assert!(source.contains("// Speed keeps its comment"), "{source}");
    assert!(source.contains("// Speed stays in comments"), "{source}");
    assert!(source.contains("text := 'Speed';"), "{source}");
    assert_eq!(source.matches("Speed").count(), 3, "{source}");
    let history = project.read("stc.renames.toml");
    assert!(history.contains("old = \"Main.Speed\""), "{history}");
    assert!(history.contains("new = \"Main.Rate\""), "{history}");

    let cold = project.run(".", &["build", "--emit", "ir", "--out-dir", "out-cold"]);
    assert_eq!(cold.status.code(), Some(0), "{}", stderr(&cold));
    assert!(
        stderr(&cold).contains("STC-W-PROJECT-002"),
        "{}",
        stderr(&cold)
    );

    let second = build_module(&project, clang, Some(&baseline_copy));
    assert_eq!(second.status.code(), Some(0), "{}", stderr(&second));
    let second_manifest = read_manifest(&project.path("out/app.stc-got.json"));
    assert_eq!(second_manifest.plan.verdict, PlanVerdict::OnlineCandidate);
    let new_id = root_id(&second_manifest, "MAIN.RATE");
    assert_eq!(old_id, new_id, "the rename must preserve the GotId");
}

const THIS_SOURCE: &str = "\
FUNCTION_BLOCK Counter
VAR
  Speed : DINT;
END_VAR
  Speed := Speed + 1;
  THIS.Speed := THIS.Speed + 1;
END_FUNCTION_BLOCK

PROGRAM Main
VAR
  counter : Counter;
END_VAR
  counter();
END_PROGRAM
";

#[test]
fn rename_rewrites_this_and_implicit_members() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping THIS rename test");
        return;
    };
    let project = TestDir::new("rename-this");
    project.write("stc.toml", &project_manifest("app", ""));
    project.write("src/main.st", THIS_SOURCE);
    let first = build_module(&project, clang, None);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    let baseline = project.path("out/app.stc-got.json");
    let baseline_copy = project.path("baseline.json");
    std::fs::copy(&baseline, &baseline_copy).expect("copy baseline");
    let old_id = root_id(&read_manifest(&baseline), "COUNTER.SPEED");

    let rename = project.run(".", &["rename", "Counter.Speed", "Counter.Rate"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    let source = project.read("src/main.st");
    assert!(source.contains("Rate := Rate + 1;"), "{source}");
    assert!(source.contains("THIS.Rate := THIS.Rate + 1;"), "{source}");
    assert!(!source.contains("Speed"), "{source}");

    let second = build_module(&project, clang, Some(&baseline_copy));
    assert_eq!(second.status.code(), Some(0), "{}", stderr(&second));
    let new_id = root_id(
        &read_manifest(&project.path("out/app.stc-got.json")),
        "COUNTER.RATE",
    );
    assert_eq!(old_id, new_id);
}

#[test]
fn rename_preserves_bom_and_line_endings() {
    let project = TestDir::new("rename-encoding");
    project.write("stc.toml", &project_manifest("app", ""));
    let source = "\u{feff}PROGRAM Main\r\nVAR\r\n  Speed : DINT;\r\nEND_VAR\r\n  Speed := Speed + 1;\r\nEND_PROGRAM\r\n";
    let path = project.path("src/main.st");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("create src");
    std::fs::write(&path, source).expect("write source");

    let rename = project.run(".", &["rename", "Main.Speed", "Main.Rate"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    let bytes = std::fs::read(&path).expect("read source");
    assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]), "BOM was lost");
    let text = String::from_utf8(bytes).expect("utf8");
    assert!(text.contains("Rate : DINT;"), "{text}");
    assert!(text.contains("Rate := Rate + 1;"), "{text}");
    assert_eq!(text.matches("\r\n").count(), source.matches("\r\n").count());
    assert!(!text.replace("\r\n", "").contains('\n'), "{text}");
}

#[test]
fn rename_history_composes_chains_oldest_first() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping rename chain test");
        return;
    };
    let project = TestDir::new("rename-chain");
    project.write("stc.toml", &project_manifest("app", ""));
    project.write(
        "src/main.st",
        "VAR_GLOBAL\n  counter : DINT;\nEND_VAR\nPROGRAM Main\n  counter := counter + 1;\nEND_PROGRAM\n",
    );
    let first = build_module(&project, clang, None);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    let baseline = project.path("out/app.stc-got.json");
    let baseline_copy = project.path("baseline.json");
    std::fs::copy(&baseline, &baseline_copy).expect("copy baseline");
    let old_id = root_id(&read_manifest(&baseline), "COUNTER");

    project.write(
        "src/main.st",
        "VAR_GLOBAL\n  total : DINT;\nEND_VAR\nPROGRAM Main\n  total := total + 1;\nEND_PROGRAM\n",
    );
    project.write(
        "stc.renames.toml",
        "schema = 1\n\n[[rename]]\nold = \"counter\"\nnew = \"counts\"\n\n\
         [[rename]]\nold = \"counts\"\nnew = \"total\"\n",
    );
    let second = build_module(&project, clang, Some(&baseline_copy));
    assert_eq!(second.status.code(), Some(0), "{}", stderr(&second));
    let updated = read_manifest(&project.path("out/app.stc-got.json"));
    assert_eq!(updated.plan.verdict, PlanVerdict::OnlineCandidate);
    assert_eq!(root_id(&updated, "TOTAL"), old_id);
}

#[test]
fn rename_preserves_values_across_an_online_swap() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping rename swap test");
        return;
    };
    let project = TestDir::new("rename-swap");
    project.write("stc.toml", &project_manifest("app", ""));
    project.write(
        "src/main.st",
        "VAR_GLOBAL\n  counter : DINT;\nEND_VAR\nPROGRAM Main\n  counter := counter + 1;\nEND_PROGRAM\n",
    );
    let first = project.run(
        ".",
        &[
            "build",
            "--manifest-path",
            "stc.toml",
            "--emit",
            "module",
            "--clang",
            clang,
            "--out-dir",
            "out1",
        ],
    );
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    let baseline = project.path("out1/app.stc-got.json");
    let baseline_copy = project.path("baseline.json");
    std::fs::copy(&baseline, &baseline_copy).expect("copy baseline");
    let v1_manifest = read_manifest(&baseline);
    let counter_id = root_id(&v1_manifest, "COUNTER");

    let v1_module = project
        .path("out1")
        .join(module_name(&project.path("out1/app.stc-meta.json")));
    let v1_candidate = stc_runtime::Candidate::open(
        &stc_runtime::host_platform(),
        &v1_module,
        &project.path("out1/app.stc-got.bin"),
    )
    .expect("v1 candidate");
    let mut host = stc_runtime::Host::new(
        stc_runtime::host_platform(),
        stc_runtime::ManifestStore::memory(),
    );
    match host
        .load(v1_candidate, stc_runtime::LoadMode::Cold)
        .expect("cold load")
    {
        stc_runtime::LoadOutcome::ColdInstalled { .. } => {}
        other => panic!("expected a cold install, got {other:?}"),
    }
    host.root_storage_mut(counter_id).expect("counter storage")[..4]
        .copy_from_slice(&42_i32.to_le_bytes());

    let rename = project.run(".", &["rename", "counter", "total"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    let second = project.run(
        ".",
        &[
            "build",
            "--manifest-path",
            "stc.toml",
            "--emit",
            "module",
            "--clang",
            clang,
            "--out-dir",
            "out2",
            "--got-manifest",
            baseline_copy.to_str().unwrap(),
        ],
    );
    assert_eq!(second.status.code(), Some(0), "{}", stderr(&second));
    let v2_manifest = read_manifest(&project.path("out2/app.stc-got.json"));
    let total_id = root_id(&v2_manifest, "TOTAL");
    assert_eq!(total_id, counter_id);

    let v2_module = project
        .path("out2")
        .join(module_name(&project.path("out2/app.stc-meta.json")));
    let v2_candidate = stc_runtime::Candidate::open(
        &stc_runtime::host_platform(),
        &v2_module,
        &project.path("out2/app.stc-got.bin"),
    )
    .expect("v2 candidate");
    match host
        .load(v2_candidate, stc_runtime::LoadMode::Online)
        .expect("online load")
    {
        stc_runtime::LoadOutcome::Prepared { .. } => {}
        other => panic!("expected a prepared change, got {other:?}"),
    }
    host.activate().expect("activate");
    let value = host.root_storage(total_id).expect("renamed storage");
    assert_eq!(
        i32::from_le_bytes(value[..4].try_into().expect("four bytes")),
        42,
        "the renamed root must keep its value"
    );
    host.cycle().expect("cycle");
    host.shutdown().expect("shutdown");
}

fn module_name(metadata: &Path) -> String {
    let text = std::fs::read_to_string(metadata).expect("metadata");
    let value = stc_got::json::parse(&text).expect("metadata json");
    value
        .as_object()
        .and_then(|object| object.get("module"))
        .and_then(|module| module.as_object())
        .and_then(|module| module.get("file"))
        .and_then(stc_got::Json::as_str)
        .expect("module file")
        .to_owned()
}

#[test]
fn rename_reports_unknown_targets_and_conflicts() {
    let project = TestDir::new("rename-errors");
    project.write("stc.toml", &project_manifest("app", ""));
    project.write("src/main.st", RENAME_SOURCE);

    let unknown = project.run(".", &["rename", "Main.Missing", "Main.Other"]);
    assert_eq!(unknown.status.code(), Some(2), "{}", stderr(&unknown));
    assert!(
        stderr(&unknown).contains("STC-E-PROJECT-013"),
        "{}",
        stderr(&unknown)
    );

    let move_key = project.run(".", &["rename", "Main.Speed", "Other.Rate"]);
    assert_eq!(move_key.status.code(), Some(2), "{}", stderr(&move_key));

    let existing = project.run(".", &["rename", "Main.Speed", "Main.text"]);
    assert_eq!(existing.status.code(), Some(2), "{}", stderr(&existing));

    project.write(
        "stc.renames.toml",
        "schema = 1\n\n[[rename]]\nold = \"Main.Speed\"\nnew = \"Main.Rate\"\n\n\
         [[rename]]\nold = \"Main.Speed\"\nnew = \"Main.Pace\"\n",
    );
    let conflict = project.run(".", &["build", "--emit", "ir"]);
    assert_eq!(conflict.status.code(), Some(2), "{}", stderr(&conflict));
    assert!(
        stderr(&conflict).contains("STC-E-PROJECT-013"),
        "{}",
        stderr(&conflict)
    );
}

const DOWNLOAD_SOURCE_V1: &str = "\
VAR_GLOBAL
  counter : DINT;
END_VAR
PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  counter := ticks;
END_PROGRAM
";

const DOWNLOAD_SOURCE_V2: &str = "\
VAR_GLOBAL
  counter : DINT;
END_VAR
PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 2;
  counter := ticks;
END_PROGRAM
";

const DOWNLOAD_SOURCE_TICKCOUNT: &str = "\
VAR_GLOBAL
  counter : DINT;
END_VAR
PROGRAM Main
VAR
  tickCount : DINT;
END_VAR
  tickCount := tickCount + 2;
  counter := tickCount;
END_PROGRAM
";

#[test]
fn online_download_exports_a_valid_envelope() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping download test");
        return;
    };
    let project = TestDir::new("download");
    project.write("stc.toml", &project_manifest("app", ""));
    project.write("src/main.st", DOWNLOAD_SOURCE_V1);
    let build = build_module(&project, clang, None);
    assert_eq!(build.status.code(), Some(0), "{}", stderr(&build));

    let store_dir = project.path("store");
    let manifest = read_manifest(&project.path("out/app.stc-got.json"));
    let stored = StoredManifest::new(manifest).expect("stored manifest");
    let build_id = stored.build_id;
    let mut store = ManifestStore::open(&store_dir);
    store.install(stored).expect("install store");

    let out = project.path("download.json");
    let binary = project.path("download.bin");
    let download = project.run(
        ".",
        &[
            "online",
            "download",
            "--store",
            store_dir.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--binary",
            binary.to_str().unwrap(),
        ],
    );
    assert_eq!(download.status.code(), Some(0), "{}", stderr(&download));
    let text = std::fs::read_to_string(&out).expect("envelope");
    let parsed = Download::from_json_text(&text).expect("valid envelope");
    assert_eq!(parsed.deployment.build_id, build_id);
    assert_eq!(
        parsed.manifest_json(),
        std::fs::read_to_string(project.path("out/app.stc-got.json")).expect("baseline json")
    );
    assert!(!parsed.layout.entries.is_empty());
    let binary_bytes = std::fs::read(&binary).expect("binary projection");
    assert_eq!(binary_bytes, parsed.manifest_binary());

    let stdout_download = project.run(
        ".",
        &["online", "download", "--store", store_dir.to_str().unwrap()],
    );
    assert_eq!(
        stdout_download.status.code(),
        Some(0),
        "{}",
        stderr(&stdout_download)
    );
    assert!(stdout(&stdout_download).contains("stc-host-download"));
    Download::from_json_text(&stdout(&stdout_download)).expect("stdout envelope");

    let missing = project.run(
        ".",
        &[
            "online",
            "download",
            "--store",
            project.path("missing").to_str().unwrap(),
        ],
    );
    assert_eq!(missing.status.code(), Some(1), "{}", stderr(&missing));
    assert!(
        stderr(&missing).contains("STC-E-ONLINE-009"),
        "{}",
        stderr(&missing)
    );

    let corrupt = project.path("corrupt");
    std::fs::create_dir_all(&corrupt).expect("corrupt store");
    std::fs::write(corrupt.join("current"), "not-a-build-id\n").expect("pointer");
    let corrupt = project.run(
        ".",
        &["online", "download", "--store", corrupt.to_str().unwrap()],
    );
    assert_eq!(corrupt.status.code(), Some(1), "{}", stderr(&corrupt));
    assert!(
        stderr(&corrupt).contains("STC-E-ONLINE-009"),
        "{}",
        stderr(&corrupt)
    );
}

#[test]
fn online_diff_compares_against_a_baseline_or_store() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping diff test");
        return;
    };
    let project = TestDir::new("diff");
    project.write("stc.toml", &project_manifest("app", ""));
    project.write("src/main.st", DOWNLOAD_SOURCE_V1);
    let first = build_module(&project, clang, None);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    let baseline = project.path("out/app.stc-got.json");
    let baseline_copy = project.path("baseline.json");
    std::fs::copy(&baseline, &baseline_copy).expect("copy baseline");

    let store_dir = project.path("store");
    let stored = StoredManifest::new(read_manifest(&baseline)).expect("stored");
    let mut store = ManifestStore::open(&store_dir);
    store.install(stored).expect("install");

    project.write("src/main.st", DOWNLOAD_SOURCE_V2);
    let second = build_module(&project, clang, Some(&baseline_copy));
    assert_eq!(second.status.code(), Some(0), "{}", stderr(&second));
    let candidate = project.path("out/app.stc-got.json");

    let compatible = project.run(
        ".",
        &[
            "online",
            "diff",
            candidate.to_str().unwrap(),
            "--baseline",
            baseline_copy.to_str().unwrap(),
        ],
    );
    assert_eq!(
        compatible.status.code(),
        Some(0),
        "stdout:\n{}\nstderr:\n{}",
        stdout(&compatible),
        stderr(&compatible)
    );
    assert!(
        stderr(&compatible).contains("compatibility compatible"),
        "{}",
        stderr(&compatible)
    );

    let via_store = project.run(
        ".",
        &[
            "online",
            "diff",
            candidate.to_str().unwrap(),
            "--store",
            store_dir.to_str().unwrap(),
            "--message-format",
            "json",
        ],
    );
    assert_eq!(via_store.status.code(), Some(0), "{}", stderr(&via_store));
    assert!(
        stdout(&via_store).contains("\"verdict\":\"compatible\""),
        "{}",
        stdout(&via_store)
    );

    // A cold candidate has no online plan; it is not compatible.
    let cold = project.run(
        ".",
        &[
            "build",
            "--manifest-path",
            "stc.toml",
            "--emit",
            "module",
            "--clang",
            clang,
            "--out-dir",
            "out-cold",
        ],
    );
    assert_eq!(cold.status.code(), Some(0), "{}", stderr(&cold));
    let cold_diff = project.run(
        ".",
        &[
            "online",
            "diff",
            project.path("out-cold/app.stc-got.json").to_str().unwrap(),
            "--baseline",
            baseline_copy.to_str().unwrap(),
        ],
    );
    assert_eq!(
        cold_diff.status.code(),
        Some(1),
        "stdout:\n{}\nstderr:\n{}",
        stdout(&cold_diff),
        stderr(&cold_diff)
    );
    assert!(
        stderr(&cold_diff).contains("compatibility cold"),
        "{}",
        stderr(&cold_diff)
    );

    project.write("bad.json", "{not a manifest");
    let malformed = project.run(
        ".",
        &[
            "online",
            "diff",
            "bad.json",
            "--baseline",
            baseline_copy.to_str().unwrap(),
        ],
    );
    assert_eq!(malformed.status.code(), Some(2), "{}", stderr(&malformed));

    let missing_baseline = project.run(".", &["online", "diff", candidate.to_str().unwrap()]);
    assert_eq!(
        missing_baseline.status.code(),
        Some(2),
        "{}",
        stderr(&missing_baseline)
    );
}

#[test]
fn project_build_uses_the_active_runtime_store_baseline() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping active-baseline test");
        return;
    };
    let project = TestDir::new("active-baseline");
    project.write(
        "stc.toml",
        &project_manifest("app", "\n[runtime]\nmanifest-dir = \"store\"\n"),
    );
    project.write("src/main.st", DOWNLOAD_SOURCE_V1);
    let first = build_module(&project, clang, None);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    let baseline = read_manifest(&project.path("out/app.stc-got.json"));
    let first_id = root_id(&baseline, "MAIN.TICKS");
    let mut store = ManifestStore::open(project.path("store"));
    store
        .install(StoredManifest::new(baseline).expect("stored"))
        .expect("install");

    project.write("src/main.st", DOWNLOAD_SOURCE_V2);
    let second = project.run(
        ".",
        &[
            "build",
            "--emit",
            "module",
            "--clang",
            clang,
            "--out-dir",
            "out",
        ],
    );
    assert_eq!(second.status.code(), Some(0), "{}", stderr(&second));
    assert!(
        stderr(&second).contains("compatibility compatible"),
        "{}",
        stderr(&second)
    );
    let updated = read_manifest(&project.path("out/app.stc-got.json"));
    assert_eq!(updated.plan.verdict, PlanVerdict::OnlineCandidate);
    assert_eq!(root_id(&updated, "MAIN.TICKS"), first_id);

    // The active store is also the default download source.
    let download = project.run(".", &["online", "download"]);
    assert_eq!(download.status.code(), Some(0), "{}", stderr(&download));
    Download::from_json_text(&stdout(&download)).expect("envelope");
}

#[test]
fn watch_rebuilds_and_deploys_a_code_change() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping watch test");
        return;
    };
    let runtime = runtime_artifact();
    let project = TestDir::new("watch");
    project.write(
        "stc.toml",
        &project_manifest("app", "\n[runtime]\nmanifest-dir = \"runtime\"\n"),
    );
    project.write("src/main.st", DOWNLOAD_SOURCE_V1);
    let store_dir = project.path("runtime");

    let mut child = Command::new(stc())
        .args([
            "run",
            "--watch",
            "--watch-timeout",
            "20s",
            "--clang",
            clang,
            "--runtime",
            runtime.to_str().unwrap(),
            "--store",
            store_dir.to_str().unwrap(),
        ])
        .current_dir(&project.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn watch");

    // Wait for the initial launch and store install, then change the source.
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && !store_dir.join("current").is_file() {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        store_dir.join("current").is_file(),
        "watch did not install the initial baseline"
    );
    std::thread::sleep(Duration::from_millis(500));
    let old_manifest = ManifestStore::open(&store_dir)
        .active()
        .expect("initial baseline")
        .manifest
        .clone();
    let old_id = root_id(&old_manifest, "MAIN.TICKS");

    // A source rename is recorded in the history; both files change and the
    // watcher must deploy a preserved identity.
    let rename = project.run(".", &["rename", "Main.ticks", "Main.tickCount"]);
    assert_eq!(rename.status.code(), Some(0), "{}", stderr(&rename));
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut renamed = None;
    while Instant::now() < deadline {
        if let Some(active) = ManifestStore::open(&store_dir).active()
            && active
                .manifest
                .roots
                .iter()
                .any(|root| root.key.contains("MAIN.TICKCOUNT"))
        {
            renamed = Some(active.manifest.clone());
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let renamed = renamed.expect("watch did not deploy the rename");
    assert_eq!(root_id(&renamed, "MAIN.TICKCOUNT"), old_id);

    // A new source file discovered by the glob must trigger another build.
    project.write("src/extra.st", "VAR_GLOBAL\n  extra : DINT;\nEND_VAR\n");
    project.write("src/main.st", DOWNLOAD_SOURCE_TICKCOUNT);

    let output = wait_with_timeout(&mut child, Duration::from_secs(40));
    let output = output.expect("watch exited");
    let text = format!("{}\n{}", stdout(&output), stderr(&output));
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.matches("deployed build").count() >= 2, "{text}");
    assert!(text.contains("watch timeout reached"), "{text}");

    let download = project.run(
        ".",
        &["online", "download", "--store", store_dir.to_str().unwrap()],
    );
    assert_eq!(download.status.code(), Some(0), "{}", stderr(&download));
    let envelope = Download::from_json_text(&stdout(&download)).expect("download envelope");
    assert_ne!(
        envelope.deployment.build_id,
        stc_got::GotId::from_bytes([0; 16])
    );
    assert_eq!(root_id(envelope.manifest(), "MAIN.TICKCOUNT"), old_id);
    assert!(
        envelope
            .manifest()
            .roots
            .iter()
            .any(|root| root.key.contains("EXTRA")),
        "the new source file was not deployed"
    );
}

#[test]
fn watch_stops_on_an_interrupt_with_exit_code_130() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping watch interrupt test");
        return;
    };
    let runtime = runtime_artifact();
    let project = TestDir::new("watch-interrupt");
    project.write(
        "stc.toml",
        &project_manifest("app", "\n[runtime]\nmanifest-dir = \"runtime\"\n"),
    );
    project.write("src/main.st", DOWNLOAD_SOURCE_V1);
    let store_dir = project.path("runtime");
    let mut command = Command::new(stc());
    command
        .args([
            "run",
            "--watch",
            "--watch-timeout",
            "30s",
            "--clang",
            clang,
            "--runtime",
            runtime.to_str().unwrap(),
            "--store",
            store_dir.to_str().unwrap(),
        ])
        .current_dir(&project.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
    let mut child = command.spawn().expect("spawn watch");
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && !store_dir.join("current").is_file() {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(store_dir.join("current").is_file(), "no initial baseline");
    std::thread::sleep(Duration::from_millis(500));
    interrupt(child.id());
    let output = wait_with_timeout(&mut child, Duration::from_secs(30)).expect("watch exited");
    let text = format!("{}\n{}", stdout(&output), stderr(&output));
    assert_eq!(output.status.code(), Some(130), "{text}");
    assert!(text.contains("watch interrupted"), "{text}");
}

/// Sends a console interrupt to one child process.
fn interrupt(pid: u32) {
    assert!(
        stc_runtime::signal::interrupt_process(pid),
        "the platform rejected the interrupt request"
    );
}

#[test]
fn watch_stops_when_stdin_closes() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping watch stdin test");
        return;
    };
    let runtime = runtime_artifact();
    let project = TestDir::new("watch-stdin");
    project.write(
        "stc.toml",
        &project_manifest("app", "\n[runtime]\nmanifest-dir = \"runtime\"\n"),
    );
    project.write("src/main.st", DOWNLOAD_SOURCE_V1);
    let store_dir = project.path("runtime");
    let mut child = Command::new(stc())
        .args([
            "run",
            "--watch",
            "--watch-timeout",
            "30s",
            "--clang",
            clang,
            "--runtime",
            runtime.to_str().unwrap(),
            "--store",
            store_dir.to_str().unwrap(),
        ])
        .current_dir(&project.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn watch");

    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && !store_dir.join("current").is_file() {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(store_dir.join("current").is_file(), "no initial baseline");
    std::thread::sleep(Duration::from_millis(500));

    // Closing the pipe is the IDE's graceful stop signal: the watch stops the
    // host through its control channel and exits 0, not 130.
    drop(child.stdin.take().expect("watch stdin"));
    let output = wait_with_timeout(&mut child, Duration::from_secs(20)).expect("watch exited");
    let text = format!("{}\n{}", stdout(&output), stderr(&output));
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("watching for changes"), "{text}");
    assert!(text.contains("watch stopped"), "{text}");
    assert!(!text.contains("watch interrupted"), "{text}");
}

#[test]
fn watch_keeps_the_old_generation_on_a_build_error() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping watch failure test");
        return;
    };
    let runtime = runtime_artifact();
    let project = TestDir::new("watch-failure");
    project.write(
        "stc.toml",
        &project_manifest("app", "\n[runtime]\nmanifest-dir = \"runtime\"\n"),
    );
    project.write("src/main.st", DOWNLOAD_SOURCE_V1);
    let store_dir = project.path("runtime");
    let mut child = Command::new(stc())
        .args([
            "run",
            "--watch",
            "--watch-timeout",
            "18s",
            "--clang",
            clang,
            "--runtime",
            runtime.to_str().unwrap(),
            "--store",
            store_dir.to_str().unwrap(),
        ])
        .current_dir(&project.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn watch");
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && !store_dir.join("current").is_file() {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(store_dir.join("current").is_file(), "no initial baseline");
    let initial = std::fs::read_to_string(store_dir.join("current")).expect("current");
    std::thread::sleep(Duration::from_millis(400));

    project.write(
        "src/main.st",
        "PROGRAM Main\n  THIS IS NOT VALID\nEND_PROGRAM\n",
    );
    std::thread::sleep(Duration::from_secs(3));
    let still_initial = std::fs::read_to_string(store_dir.join("current")).expect("current");
    assert_eq!(
        initial, still_initial,
        "a failed build must keep the old generation"
    );

    project.write("src/main.st", DOWNLOAD_SOURCE_V2);
    let output = wait_with_timeout(&mut child, Duration::from_secs(40)).expect("watch exited");
    let text = format!("{}\n{}", stdout(&output), stderr(&output));
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("build failed; keeping generation"), "{text}");
    assert!(text.contains("deployed build"), "{text}");
    let final_current = std::fs::read_to_string(store_dir.join("current")).expect("current");
    assert_ne!(
        final_current, initial,
        "the recovery build must be deployed"
    );

    let download = project.run(
        ".",
        &["online", "download", "--store", store_dir.to_str().unwrap()],
    );
    assert_eq!(download.status.code(), Some(0), "{}", stderr(&download));
    let envelope = Download::from_json_text(&stdout(&download)).expect("envelope");
    assert!(
        final_current.contains(&envelope.deployment.build_id.to_hex_plain()),
        "{final_current}"
    );
}

#[test]
fn online_download_can_request_a_live_host() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping live download test");
        return;
    };
    let project = TestDir::new("live-download");
    project.write("stc.toml", &project_manifest("app", ""));
    project.write("src/main.st", DOWNLOAD_SOURCE_V1);
    let build = build_module(&project, clang, None);
    assert_eq!(build.status.code(), Some(0), "{}", stderr(&build));
    let stored =
        StoredManifest::new(read_manifest(&project.path("out/app.stc-got.json"))).expect("stored");
    let build_id = stored.build_id;

    project.write("src/main.st", DOWNLOAD_SOURCE_V2);
    let candidate = project.run(
        ".",
        &[
            "build",
            "--manifest-path",
            "stc.toml",
            "--emit",
            "module",
            "--clang",
            clang,
            "--out-dir",
            "out2",
            "--got-manifest",
            project.path("out/app.stc-got.json").to_str().unwrap(),
        ],
    );
    assert_eq!(candidate.status.code(), Some(0), "{}", stderr(&candidate));
    let candidate_path = project.path("out2/app.stc-got.json");

    let endpoint = {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let token = format!("{}-{id}", std::process::id());
        if cfg!(windows) {
            format!(r"\\.\pipe\stc-online-test-{token}")
        } else {
            std::env::temp_dir()
                .join(format!("stc-online-test-{token}.sock"))
                .to_string_lossy()
                .into_owned()
        }
    };
    let server = stc_runtime::control::ControlServer::bind(&endpoint, "nonce").expect("bind");
    let mut store = ManifestStore::memory();
    store.install(stored).expect("install");
    let mut host = stc_runtime::Host::new(stc_runtime::host_platform(), store);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let output = std::thread::scope(|scope| {
        let stop_worker = std::sync::Arc::clone(&stop);
        let worker = scope.spawn(move || {
            while !stop_worker.load(Ordering::Relaxed) {
                server.process(&mut host);
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let output = project.run(
            ".",
            &[
                "online", "download", "--host", &endpoint, "--nonce", "nonce",
            ],
        );
        let diff = project.run(
            ".",
            &[
                "online",
                "diff",
                candidate_path.to_str().unwrap(),
                "--host",
                &endpoint,
                "--nonce",
                "nonce",
            ],
        );
        let wrong = project.run(
            ".",
            &[
                "online", "download", "--host", &endpoint, "--nonce", "wrong",
            ],
        );
        stop.store(true, Ordering::Relaxed);
        worker.join().expect("worker");
        (output, diff, wrong)
    });
    let (output, diff, wrong) = output;
    assert_eq!(
        diff.status.code(),
        Some(0),
        "stdout:\n{}\nstderr:\n{}",
        stdout(&diff),
        stderr(&diff)
    );
    assert!(
        stderr(&diff).contains("compatibility compatible"),
        "{}",
        stderr(&diff)
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let envelope = Download::from_json_text(&stdout(&output)).expect("live envelope");
    assert_eq!(envelope.deployment.build_id, build_id);
    assert_eq!(wrong.status.code(), Some(1), "{}", stderr(&wrong));
    assert!(
        stderr(&wrong).contains("STC-E-ONLINE-007"),
        "{}",
        stderr(&wrong)
    );
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Option<Output> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let mut stdout_pipe = child.stdout.take().expect("stdout");
                let mut stderr_pipe = child.stderr.take().expect("stderr");
                use std::io::Read;
                let mut out = String::new();
                let mut err = String::new();
                let _ = stdout_pipe.read_to_string(&mut out);
                let _ = stderr_pipe.read_to_string(&mut err);
                let status = child.wait().ok()?;
                return Some(Output {
                    status,
                    stdout: out.into_bytes(),
                    stderr: err.into_bytes(),
                });
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}
