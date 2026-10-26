//! Phase-7 CLI integration: module artifacts, baseline consumption, rename
//! tracking, and launcher/embedding-host coexistence.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use stc_got::{DescriptorKind, GotId, Manifest, PlanVerdict};

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
            let exe = std::env::current_exe().expect("test executable");
            let profile = exe
                .parent()
                .and_then(Path::parent)
                .expect("target profile directory");
            let name = if cfg!(windows) {
                "stc_runtime.lib"
            } else {
                "libstc_runtime.a"
            };
            profile.join(name)
        })
        .clone()
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
        let path = std::env::temp_dir().join(format!("stc-module-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test directory");
        Self { path }
    }

    fn source(&self, name: &str, source: &str) -> PathBuf {
        let path = self.path.join(format!("{name}.st"));
        std::fs::write(&path, source).expect("write source");
        path
    }

    fn manifest(&self, name: &str) -> Manifest {
        let text = std::fs::read_to_string(self.path.join(format!("{name}.stc-got.json")))
            .expect("manifest JSON");
        Manifest::decode_json(&text).expect("parse manifest")
    }

    fn binary_manifest(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.path.join(format!("{name}.stc-got.bin"))).expect("manifest binary")
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn build(directory: &TestDir, source: &Path, name: &str, extra: &[&str]) -> Output {
    let emit = extra
        .iter()
        .find(|argument| argument.starts_with("--emit="))
        .copied()
        .unwrap_or("--emit=module");
    let mut arguments = vec![
        "build".to_owned(),
        source.to_str().unwrap().to_owned(),
        emit.to_owned(),
        "--name".into(),
        name.into(),
        "--out-dir".into(),
        directory.path.to_str().unwrap().to_owned(),
    ];
    for argument in extra {
        if !argument.starts_with("--emit=") {
            arguments.push((*argument).into());
        }
    }
    Command::new(stc())
        .args(&arguments)
        .output()
        .expect("run stc")
}

fn build_ok(directory: &TestDir, source: &Path, name: &str, extra: &[&str]) -> Output {
    let output = build(directory, source, name, extra);
    assert!(
        output.status.success(),
        "build `{name}` failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn module_file(directory: &TestDir, name: &str) -> String {
    let extension = if cfg!(windows) { "dll" } else { "so" };
    let prefix = format!("{name}-");
    let suffix = format!(".{extension}");
    std::fs::read_dir(&directory.path)
        .expect("artifact directory")
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .find(|file| file.starts_with(&prefix) && file.ends_with(&suffix))
        .unwrap_or_else(|| panic!("module for `{name}` was not emitted"))
}

/// Serializes tests that drive a `Host`, because the process image is
/// process-global. Cold installs of compiler modules configure (and clear) it.
static HOST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn host_guard() -> std::sync::MutexGuard<'static, ()> {
    HOST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
    directory: &TestDir,
    name: &str,
    platform: &stc_runtime::HostPlatform,
) -> stc_runtime::Candidate {
    stc_runtime::Candidate::open(
        platform,
        &directory.path.join(module_file(directory, name)),
        &directory.path.join(format!("{name}.stc-got.bin")),
    )
    .unwrap_or_else(|error| panic!("candidate `{name}`: {error}"))
}

fn root_id(manifest: &Manifest, key: &str) -> GotId {
    manifest
        .roots
        .iter()
        .find(|root| root.key == key)
        .unwrap_or_else(|| panic!("root `{key}`"))
        .got_id
}

fn key_with(manifest: &Manifest, needle: &str) -> GotId {
    manifest
        .roots
        .iter()
        .find(|root| root.key.contains(needle))
        .unwrap_or_else(|| panic!("root containing `{needle}`"))
        .got_id
}

fn read_u32(host: &stc_runtime::Host, got_id: GotId) -> u32 {
    let storage = host.root_storage(got_id).expect("root storage");
    u32::from_le_bytes(storage[..4].try_into().expect("four bytes"))
}

fn write_u32(host: &mut stc_runtime::Host, got_id: GotId, value: u32) {
    let storage = host.root_storage_mut(got_id).expect("root storage");
    storage[..4].copy_from_slice(&value.to_le_bytes());
}

fn image_length() -> usize {
    usize::try_from(stc_runtime::image_length()).expect("image length")
}

fn image_base() -> usize {
    let base = stc_runtime::image_base();
    assert_ne!(base, 0, "the process image is configured");
    base
}

fn image_bytes() -> Vec<u8> {
    let len = image_length();
    let mut bytes = vec![0; len];
    assert_eq!(stc_runtime::image_read(0, &mut bytes), len);
    bytes
}

fn write_image(offset: usize, bytes: &[u8]) {
    assert!(
        stc_runtime::image_write(offset, bytes),
        "write inside the process image"
    );
}

const V1: &str = r#"
VAR_GLOBAL
  Counter : DINT;
  Flag : BOOL;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  Counter := ticks;
  Flag := Counter > 3;
END_PROGRAM
"#;

const V2_CODE_ONLY: &str = r#"
VAR_GLOBAL
  Counter : DINT;
  Flag : BOOL;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 2;
  Counter := ticks;
  Flag := Counter > 4;
END_PROGRAM
"#;

#[test]
fn module_build_emits_a_complete_artifact_pair() {
    let Some(_) = clang() else {
        return;
    };
    let directory = TestDir::new("artifacts");
    let source = directory.source("app", V1);
    build_ok(&directory, &source, "app", &[]);

    for file in [
        module_file(&directory, "app"),
        "app.stc-got.bin".into(),
        "app.stc-got.json".into(),
        "app.stc-meta.json".into(),
        "app.ll".into(),
    ] {
        assert!(
            directory.path.join(&file).is_file(),
            "missing artifact `{file}`"
        );
    }
    let metadata = stc_got::json::parse(
        &std::fs::read_to_string(directory.path.join("app.stc-meta.json")).expect("metadata"),
    )
    .expect("metadata JSON");
    let object = metadata.as_object().expect("metadata object");
    assert_eq!(
        object
            .get("module")
            .and_then(stc_got::Json::as_object)
            .and_then(|module| module.get("file"))
            .and_then(stc_got::Json::as_str),
        Some(module_file(&directory, "app").as_str())
    );
    assert_eq!(
        object.get("abi_version").and_then(stc_got::Json::as_number),
        Some(2)
    );
    assert_eq!(
        object.get("target").and_then(stc_got::Json::as_str),
        Some(stc_codegen::host_triple().as_str())
    );
    assert!(
        object
            .get("build_id")
            .and_then(stc_got::Json::as_str)
            .is_some_and(|id| id.starts_with("0x"))
    );

    let manifest = directory.manifest("app");
    assert_eq!(manifest.plan.verdict, PlanVerdict::Cold);
    assert!(manifest.header.parent_build_id.is_none());
    assert!(!manifest.roots.is_empty());
}

fn artifact_bytes(directory: &TestDir, file: &str) -> Vec<u8> {
    std::fs::read(directory.path.join(file))
        .unwrap_or_else(|error| panic!("cannot read artifact `{file}`: {error}"))
}

fn emitted_module(directory: &TestDir, name: &str) -> bool {
    let prefix = format!("{name}-");
    std::fs::read_dir(&directory.path)
        .expect("artifact directory")
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .any(|file| file.starts_with(&prefix) && (file.ends_with(".dll") || file.ends_with(".so")))
}

#[test]
fn double_builds_from_the_same_source_and_baseline_are_byte_identical() {
    let Some(_) = clang() else {
        return;
    };
    let first = TestDir::new("determinism-a");
    let second = TestDir::new("determinism-b");
    for directory in [&first, &second] {
        let source = directory.source("app", V1);
        build_ok(directory, &source, "app", &[]);
        let baseline = directory.path.join("app.stc-got.bin");
        let source = directory.source("app_v2", V2_CODE_ONLY);
        build_ok(
            directory,
            &source,
            "app_v2",
            &["--got-manifest", baseline.to_str().unwrap()],
        );
    }

    for name in ["app", "app_v2"] {
        let module = module_file(&first, name);
        assert_eq!(
            module,
            module_file(&second, name),
            "module name for `{name}`"
        );
        for file in [
            module,
            format!("{name}.stc-got.bin"),
            format!("{name}.stc-got.json"),
            format!("{name}.stc-meta.json"),
            format!("{name}.ll"),
        ] {
            assert_eq!(
                artifact_bytes(&first, &file),
                artifact_bytes(&second, &file),
                "artifact `{file}` differs between build directories"
            );
        }

        let left = first.manifest(name);
        let right = second.manifest(name);
        assert_eq!(left.header.build_id, right.header.build_id);
        assert_eq!(left.plan, right.plan);
        assert_eq!(
            left.digest().expect("digest"),
            right.digest().expect("digest")
        );
        assert_eq!(left, right);
    }
    assert_eq!(
        first.manifest("app_v2").plan.verdict,
        PlanVerdict::OnlineCandidate
    );
}

#[test]
fn malformed_baselines_and_options_are_rejected_by_the_build() {
    let Some(_) = clang() else {
        return;
    };
    let directory = TestDir::new("bad-baseline");
    let source = directory.source("app", V1);
    build_ok(&directory, &source, "app", &[]);

    let refused = build(
        &directory,
        &source,
        "app_missing",
        &[
            "--got-manifest",
            directory.path.join("missing.stc-got.bin").to_str().unwrap(),
        ],
    );
    assert_eq!(refused.status.code(), Some(1));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(text.contains("STC-E-BUILD-002"), "{text}");
    assert!(text.contains("cannot read baseline"), "{text}");

    let baseline = std::fs::read(directory.path.join("app.stc-got.bin")).expect("baseline");
    let truncated = directory.path.join("truncated.stc-got.bin");
    std::fs::write(&truncated, &baseline[..baseline.len() - 8]).expect("write truncated baseline");
    let refused = build(
        &directory,
        &source,
        "app_truncated",
        &["--got-manifest", truncated.to_str().unwrap()],
    );
    assert_eq!(refused.status.code(), Some(1));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(text.contains("not a valid GOT manifest"), "{text}");

    let truncated_json = directory.path.join("truncated.stc-got.json");
    std::fs::write(&truncated_json, "{}").expect("write malformed JSON baseline");
    let refused = build(
        &directory,
        &source,
        "app_badjson",
        &["--got-manifest", truncated_json.to_str().unwrap()],
    );
    assert_eq!(refused.status.code(), Some(1));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(text.contains("not a valid GOT manifest"), "{text}");

    assert!(!directory.path.join("app_missing.stc-got.bin").exists());
    for name in ["app_missing", "app_truncated", "app_badjson"] {
        assert!(
            !emitted_module(&directory, name),
            "a refused build emitted a module for `{name}`"
        );
    }

    let refused = build(
        &directory,
        &source,
        "app_badrename",
        &["--rename", "broken"],
    );
    assert_eq!(refused.status.code(), Some(2));
    let text = String::from_utf8_lossy(&refused.stderr);
    assert!(text.contains("`--rename` expects `old=new`"), "{text}");
}

#[test]
fn a_baseline_build_reuses_unchanged_identities() {
    let Some(_) = clang() else {
        return;
    };
    let directory = TestDir::new("baseline");
    let v1_source = directory.source("app_v1", V1);
    build_ok(&directory, &v1_source, "app_v1", &[]);
    let v1 = directory.manifest("app_v1");
    let baseline = directory.binary_manifest("app_v1");
    let baseline_path = directory.path.join("app_v1.stc-got.bin");

    let v2_source = directory.source("app_v2", V2_CODE_ONLY);
    build_ok(
        &directory,
        &v2_source,
        "app_v2",
        &["--got-manifest", baseline_path.to_str().unwrap()],
    );
    let v2 = directory.manifest("app_v2");
    assert_eq!(v2.plan.verdict, PlanVerdict::OnlineCandidate);
    assert_eq!(v2.header.parent_build_id, Some(v1.header.build_id));
    for root in &v1.roots {
        let matching = v2
            .roots
            .iter()
            .find(|candidate| candidate.key == root.key)
            .unwrap_or_else(|| panic!("root `{}`", root.key));
        assert_eq!(matching.got_id, root.got_id, "key `{}`", root.key);
    }
    assert_eq!(baseline, directory.binary_manifest("app_v1"));
}

#[test]
fn tracked_renames_preserve_identity_and_previous_keys() {
    let Some(_) = clang() else {
        return;
    };
    let directory = TestDir::new("rename");
    let v1_source = directory.source("app_v1", V1);
    build_ok(&directory, &v1_source, "app_v1", &[]);
    let v1 = directory.manifest("app_v1");
    let old = v1
        .roots
        .iter()
        .find(|root| root.key == "_ROOT::COUNTER")
        .expect("counter root")
        .got_id;
    let baseline_path = directory.path.join("app_v1.stc-got.bin");

    let renamed = V1
        .replace("Counter : DINT", "Count : DINT")
        .replace("Counter := ticks", "Count := ticks")
        .replace("Counter > 3", "Count > 3");
    let v2_source = directory.source("app_v2", &renamed);
    build_ok(
        &directory,
        &v2_source,
        "app_v2",
        &[
            "--got-manifest",
            baseline_path.to_str().unwrap(),
            "--rename",
            "_ROOT::COUNTER=_ROOT::COUNT",
        ],
    );
    let v2 = directory.manifest("app_v2");
    let renamed_root = v2
        .roots
        .iter()
        .find(|root| root.key == "_ROOT::COUNT")
        .expect("renamed root");
    assert_eq!(renamed_root.got_id, old);
    assert!(
        renamed_root
            .previous_keys
            .iter()
            .any(|key| key == "_ROOT::COUNTER"),
        "{:?}",
        renamed_root.previous_keys
    );
}

#[test]
fn launcher_and_embedding_host_can_be_emitted_together() {
    let Some(_) = clang() else {
        return;
    };
    let directory = TestDir::new("both");
    let source = directory.source("app", V1);
    let runtime = runtime_artifact();
    let mut extra = vec!["--emit=exe,lib"];
    let runtime = runtime.to_str().unwrap();
    extra.push("--runtime");
    extra.push(runtime);
    build_ok(&directory, &source, "both", &extra);

    let exe = if cfg!(windows) {
        "both.exe".to_owned()
    } else {
        "both".to_owned()
    };
    let library = if cfg!(windows) {
        "both.lib".to_owned()
    } else {
        "both.a".to_owned()
    };
    assert!(directory.path.join(exe).is_file(), "launcher missing");
    assert!(
        directory.path.join(library).is_file(),
        "host library missing"
    );
    assert!(
        directory
            .path
            .join(module_file(&directory, "both"))
            .is_file(),
        "module missing"
    );
}

#[test]
fn a_generated_module_runs_and_survives_an_online_change() {
    use stc_runtime::{Candidate, Host, HostPlatform, LoadMode, LoadOutcome, ManifestStore};

    let Some(_) = clang() else {
        return;
    };
    let _guard = host_guard();
    let directory = TestDir::new("online");
    let v1_source = directory.source("app_v1", V1);
    build_ok(&directory, &v1_source, "app_v1", &[]);
    let v1 = directory.manifest("app_v1");
    let baseline_path = directory.path.join("app_v1.stc-got.bin");

    let v2_source = directory.source("app_v2", V2_CODE_ONLY);
    build_ok(
        &directory,
        &v2_source,
        "app_v2",
        &["--got-manifest", baseline_path.to_str().unwrap()],
    );
    let v2 = directory.manifest("app_v2");

    let platform = HostPlatform::new(
        stc_runtime::PROFILE,
        stc_codegen::resolve_target(Some(&stc_codegen::host_triple())),
        stc_got::HOST_ABI_MAJOR,
        stc_got::HOST_ABI_MINOR,
    );
    let mut host = Host::new(platform.clone(), ManifestStore::memory());
    let counter = v1
        .roots
        .iter()
        .find(|root| root.key == "_ROOT::COUNTER")
        .expect("counter root")
        .got_id;
    let ticks = v1
        .roots
        .iter()
        .find(|root| root.key.contains("TICKS"))
        .expect("ticks root")
        .got_id;

    let v1_candidate = Candidate::open(
        &platform,
        &directory.path.join(module_file(&directory, "app_v1")),
        &directory.path.join("app_v1.stc-got.bin"),
    )
    .expect("v1 candidate");
    let outcome = host
        .load(v1_candidate, LoadMode::Auto)
        .expect("cold install");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));

    host.root_storage_mut(counter).expect("counter")[..4].copy_from_slice(&41_u32.to_le_bytes());
    host.root_storage_mut(ticks).expect("ticks")[..4].copy_from_slice(&7_u32.to_le_bytes());

    let v2_candidate = Candidate::open(
        &platform,
        &directory.path.join(module_file(&directory, "app_v2")),
        &directory.path.join("app_v2.stc-got.bin"),
    )
    .expect("v2 candidate");
    let outcome = host.load(v2_candidate, LoadMode::Auto).expect("prepare");
    assert!(matches!(outcome, LoadOutcome::Prepared { .. }));
    assert_eq!(host.active_build_id(), Some(v1.header.build_id));
    assert_eq!(
        u32::from_le_bytes(
            host.root_storage(counter).expect("counter")[..4]
                .try_into()
                .unwrap()
        ),
        41
    );
    assert_eq!(
        u32::from_le_bytes(
            host.root_storage(ticks).expect("ticks")[..4]
                .try_into()
                .unwrap()
        ),
        7
    );

    host.activate().expect("activate");
    assert_eq!(host.active_build_id(), Some(v2.header.build_id));
    assert_eq!(
        u32::from_le_bytes(
            host.root_storage(ticks).expect("ticks")[..4]
                .try_into()
                .unwrap()
        ),
        7,
        "online prepare must not reinitialize a preserved root"
    );
    host.cycle().expect("commit cycle");
    assert_eq!(
        host.export_manifest().expect("baseline").header.build_id,
        v2.header.build_id
    );
    assert_eq!(
        host.retired_generations(),
        0,
        "the committed previous generation is unloaded once unpinned"
    );
    assert_eq!(
        u32::from_le_bytes(
            host.root_storage(ticks).expect("ticks")[..4]
                .try_into()
                .unwrap()
        ),
        9,
        "the new code ran with the preserved value"
    );
}

const M5_HOT_V1: &str = r#"
TYPE
  Point : STRUCT
    x : DINT;
    y : DINT;
  END_STRUCT;
END_TYPE

FUNCTION_BLOCK Counter
VAR_INPUT
  inc : DINT;
END_VAR
VAR_OUTPUT
  q : DINT;
END_VAR
VAR
  cv : DINT;
END_VAR
  cv := cv + inc;
  q := cv;
END_FUNCTION_BLOCK

VAR_GLOBAL
  fbc : Counter;
  gp : Point;
  gs : STRING[8];
  ga : ARRAY[1..2] OF Point;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
  q : DINT;
END_VAR
  ticks := ticks + 1;
  fbc(inc := 1, q => q);
  gp.x := ticks;
  ga[2].y := ticks;
  gs := 'v1';
END_PROGRAM
"#;

const M5_HOT_V2_CODE_ONLY: &str = r#"
TYPE
  Point : STRUCT
    x : DINT;
    y : DINT;
  END_STRUCT;
END_TYPE

FUNCTION_BLOCK Counter
VAR_INPUT
  inc : DINT;
END_VAR
VAR_OUTPUT
  q : DINT;
END_VAR
VAR
  cv : DINT;
END_VAR
  cv := cv + inc;
  q := cv;
END_FUNCTION_BLOCK

VAR_GLOBAL
  fbc : Counter;
  gp : Point;
  gs : STRING[8];
  ga : ARRAY[1..2] OF Point;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
  q : DINT;
END_VAR
  ticks := ticks + 2;
  fbc(inc := 2, q => q);
  gp.x := ticks;
  ga[2].y := ticks;
  gs := 'v2';
END_PROGRAM
"#;

#[test]
fn m5_fb_and_aggregate_roots_survive_an_online_change() {
    use stc_runtime::{Candidate, Host, HostPlatform, LoadMode, LoadOutcome, ManifestStore};

    let Some(_) = clang() else {
        return;
    };
    let _guard = host_guard();
    let directory = TestDir::new("online-m5");
    let v1_source = directory.source("m5_v1", M5_HOT_V1);
    build_ok(&directory, &v1_source, "m5_v1", &[]);
    let v1 = directory.manifest("m5_v1");
    let baseline_path = directory.path.join("m5_v1.stc-got.bin");
    let v2_source = directory.source("m5_v2", M5_HOT_V2_CODE_ONLY);
    build_ok(
        &directory,
        &v2_source,
        "m5_v2",
        &["--got-manifest", baseline_path.to_str().unwrap()],
    );
    let v2 = directory.manifest("m5_v2");

    let platform = HostPlatform::new(
        stc_runtime::PROFILE,
        stc_codegen::resolve_target(Some(&stc_codegen::host_triple())),
        stc_got::HOST_ABI_MAJOR,
        stc_got::HOST_ABI_MINOR,
    );
    let mut host = Host::new(platform.clone(), ManifestStore::memory());

    let find_root = |manifest: &stc_got::Manifest, needle: &str| {
        manifest
            .roots
            .iter()
            .find(|root| root.key.contains(needle))
            .unwrap_or_else(|| {
                panic!(
                    "root containing `{needle}` missing; roots: {:?}",
                    manifest
                        .roots
                        .iter()
                        .map(|root| root.key.as_str())
                        .collect::<Vec<_>>()
                )
            })
            .got_id
    };
    let fbc_cv = find_root(&v1, "FBC.CV");
    let gp_x = find_root(&v1, "GP.X");
    let gp_y = find_root(&v1, "GP.Y");
    let ga_y = find_root(&v1, "GA.E0.Y");
    let ga2_y = find_root(&v1, "GA.E1.Y");
    let gs = find_root(&v1, "GS");
    let ticks = find_root(&v1, "TICKS");

    let v1_candidate = Candidate::open(
        &platform,
        &directory.path.join(module_file(&directory, "m5_v1")),
        &directory.path.join("m5_v1.stc-got.bin"),
    )
    .expect("v1 candidate");
    let outcome = host
        .load(v1_candidate, LoadMode::Auto)
        .expect("cold install");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));

    let write_i32 = |host: &mut Host, root: stc_got::GotId, value: i32| {
        host.root_storage_mut(root).expect("root")[..4].copy_from_slice(&value.to_le_bytes());
    };
    write_i32(&mut host, fbc_cv, 41);
    write_i32(&mut host, gp_x, 5);
    write_i32(&mut host, gp_y, 6);
    write_i32(&mut host, ga_y, 7);
    write_i32(&mut host, ga2_y, 11);
    write_i32(&mut host, ticks, 7);
    let string = host.root_storage_mut(gs).expect("string root");
    string[..4].copy_from_slice(&2_i32.to_le_bytes());
    string[4] = b'h';
    string[5] = b'i';

    let read_i32 = |host: &Host, root: stc_got::GotId| {
        i32::from_le_bytes(
            host.root_storage(root).expect("root")[..4]
                .try_into()
                .unwrap(),
        )
    };

    let v2_candidate = Candidate::open(
        &platform,
        &directory.path.join(module_file(&directory, "m5_v2")),
        &directory.path.join("m5_v2.stc-got.bin"),
    )
    .expect("v2 candidate");
    let outcome = host.load(v2_candidate, LoadMode::Auto).expect("prepare");
    assert!(matches!(outcome, LoadOutcome::Prepared { .. }));
    assert_eq!(read_i32(&host, fbc_cv), 41);
    assert_eq!(read_i32(&host, gp_x), 5);
    assert_eq!(read_i32(&host, gp_y), 6);
    assert_eq!(read_i32(&host, ga_y), 7);
    assert_eq!(read_i32(&host, ga2_y), 11);
    assert_eq!(read_i32(&host, ticks), 7);
    assert_eq!(
        &host.root_storage(gs).expect("string root")[..6],
        &[2, 0, 0, 0, b'h', b'i']
    );

    host.activate().expect("activate");
    host.cycle().expect("commit cycle");
    assert_eq!(host.active_build_id(), Some(v2.header.build_id));
    assert_eq!(read_i32(&host, ticks), 9);
    assert_eq!(read_i32(&host, fbc_cv), 43, "FB state continued from 41");
    assert_eq!(read_i32(&host, gp_x), 9);
    assert_eq!(read_i32(&host, gp_y), 6, "untouched struct field preserved");
    assert_eq!(
        read_i32(&host, ga_y),
        7,
        "untouched array element preserved"
    );
    assert_eq!(
        read_i32(&host, ga2_y),
        9,
        "new code updated the second element"
    );
    assert_eq!(
        &host.root_storage(gs).expect("string root")[..6],
        &[2, 0, 0, 0, b'v', b'2']
    );
}

const OOP_HOT_V1: &str = r#"
INTERFACE IBase
METHOD Area : DINT
END_METHOD
END_INTERFACE
INTERFACE IShape EXTENDS IBase
METHOD Perimeter : DINT
END_METHOD
END_INTERFACE

CLASS Shape IMPLEMENTS IShape
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Area : DINT
  Area := id;
END_METHOD
METHOD PUBLIC Perimeter : DINT
  Perimeter := id + 1;
END_METHOD
END_CLASS

VAR_GLOBAL
  s : Shape;
  i : IShape;
  result : DINT;
  ticks : DINT;
END_VAR

PROGRAM Main
  s.id := s.id + 1;
  i := s;
  ticks := ticks + 1;
  result := i.Area() * 100 + i.Perimeter() * 10 + s.Area();
END_PROGRAM
"#;

const OOP_HOT_V2_CODE_ONLY: &str = r#"
INTERFACE IBase
METHOD Area : DINT
END_METHOD
END_INTERFACE
INTERFACE IShape EXTENDS IBase
METHOD Perimeter : DINT
END_METHOD
END_INTERFACE

CLASS Shape IMPLEMENTS IShape
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Area : DINT
  Area := id + 10;
END_METHOD
METHOD PUBLIC Perimeter : DINT
  Perimeter := id + 20;
END_METHOD
END_CLASS

VAR_GLOBAL
  s : Shape;
  i : IShape;
  result : DINT;
  ticks : DINT;
END_VAR

PROGRAM Main
  s.id := s.id + 1;
  i := s;
  ticks := ticks + 1;
  result := i.Area() * 100 + i.Perimeter() * 10 + s.Area();
END_PROGRAM
"#;

fn descriptor_id(manifest: &Manifest, owner: GotId, kind: DescriptorKind) -> GotId {
    manifest
        .descriptors
        .iter()
        .find(|descriptor| {
            descriptor.owner_type == Some(owner) && descriptor.descriptor_kind == kind
        })
        .unwrap_or_else(|| panic!("descriptor for type {owner}"))
        .got_id
}

fn type_id(manifest: &Manifest, key: &str) -> GotId {
    manifest
        .types
        .iter()
        .find(|ty| ty.key == key)
        .unwrap_or_else(|| panic!("type `{key}`"))
        .got_id
}

fn read_u64(host: &stc_runtime::Host, root: GotId) -> u64 {
    let storage = host.root_storage(root).expect("root storage");
    u64::from_le_bytes(storage[..8].try_into().expect("eight bytes"))
}

#[test]
fn interface_values_and_class_handles_survive_an_online_change() {
    use stc_runtime::{Host, LoadMode, LoadOutcome, ManifestStore};

    let Some(_) = clang() else {
        return;
    };
    let _guard = host_guard();
    let directory = TestDir::new("online-oop");
    let v1_source = directory.source("oop_v1", OOP_HOT_V1);
    build_ok(&directory, &v1_source, "oop_v1", &[]);
    let v1 = directory.manifest("oop_v1");
    let baseline = directory.path.join("oop_v1.stc-got.bin");
    let v2_source = directory.source("oop_v2", OOP_HOT_V2_CODE_ONLY);
    build_ok(
        &directory,
        &v2_source,
        "oop_v2",
        &["--got-manifest", baseline.to_str().unwrap()],
    );
    let v2 = directory.manifest("oop_v2");

    let shape = type_id(&v1, "_ROOT::SHAPE");
    let ishape = type_id(&v1, "_ROOT::ISHAPE");
    assert_eq!(type_id(&v2, "_ROOT::SHAPE"), shape);
    assert_eq!(type_id(&v2, "_ROOT::ISHAPE"), ishape);
    let typeinfo = descriptor_id(&v1, shape, DescriptorKind::TypeInfo);
    let vtable = descriptor_id(&v1, shape, DescriptorKind::Vtable);
    let handle_root = root_id(&v1, "_ROOT::S#HANDLE");
    let record_root = root_id(&v1, "_ROOT::S#OBJREC");
    let interface_root = root_id(&v1, "_ROOT::I");
    let id_root = root_id(&v1, "_ROOT::S.ID");
    let result = root_id(&v1, "_ROOT::RESULT");
    let ticks = key_with(&v1, "TICKS");

    let platform = host_platform();
    let mut host = Host::new(platform.clone(), ManifestStore::memory());
    let outcome = host
        .load(
            open_candidate(&directory, "oop_v1", &platform),
            LoadMode::Auto,
        )
        .expect("cold install");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));
    host.cycle().expect("cycle v1");
    assert_eq!(read_u32(&host, id_root), 1);
    assert_eq!(read_u32(&host, result), 121);
    assert_eq!(read_u32(&host, ticks), 1);

    // The hidden roots and descriptors describe one live object and one
    // interface value: handle -> record -> TYPEINFO/leaf, interface -> record.
    let record_address = host.roots().address(record_root).expect("record address");
    let typeinfo_address = host.descriptor_address(typeinfo).expect("TYPEINFO address");
    let vtable_address = host.descriptor_address(vtable).expect("VTABLE address");
    let pair_address = host
        .interface_vtable_address(shape, ishape)
        .expect("interface pair");
    assert_eq!(read_u64(&host, handle_root), record_address as u64);
    assert_eq!(host.root_storage(record_root).expect("record").len(), 16);
    assert_eq!(
        u64::from_le_bytes(
            host.root_storage(record_root).expect("record")[..8]
                .try_into()
                .unwrap()
        ),
        typeinfo_address as u64
    );
    let interface_before = host
        .root_storage(interface_root)
        .expect("interface value")
        .to_vec();
    assert_eq!(
        u64::from_le_bytes(interface_before[..8].try_into().unwrap()),
        record_address as u64
    );
    assert_eq!(
        u64::from_le_bytes(interface_before[8..16].try_into().unwrap()),
        pair_address as u64
    );
    let pair_before = host
        .interface_vtable_words(shape, ishape)
        .expect("interface pair")
        .to_vec();

    let outcome = host
        .load(
            open_candidate(&directory, "oop_v2", &platform),
            LoadMode::Auto,
        )
        .expect("prepare");
    assert!(matches!(outcome, LoadOutcome::Prepared { .. }));
    // Descriptors keep their identity and the preserved object and interface
    // roots keep their bytes; the pair still points at v1 cells until activation.
    assert_eq!(host.descriptor_address(typeinfo), Some(typeinfo_address));
    assert_eq!(host.descriptor_address(vtable), Some(vtable_address));
    assert_eq!(
        host.interface_vtable_address(shape, ishape),
        Some(pair_address)
    );
    assert_eq!(host.roots().address(record_root), Some(record_address));
    assert_eq!(read_u64(&host, handle_root), record_address as u64);
    assert_eq!(
        host.root_storage(interface_root).expect("interface value"),
        interface_before.as_slice()
    );
    assert_eq!(
        host.interface_vtable_words(shape, ishape).expect("pair"),
        pair_before.as_slice(),
        "entries stay on the active generation until activation"
    );
    assert_eq!(read_u32(&host, id_root), 1);
    assert_eq!(read_u32(&host, result), 121);

    host.activate().expect("activate");
    assert_eq!(host.descriptor_address(typeinfo), Some(typeinfo_address));
    assert_eq!(host.descriptor_address(vtable), Some(vtable_address));
    assert_eq!(
        host.interface_vtable_address(shape, ishape),
        Some(pair_address),
        "a preserved interface value keeps its pair address"
    );
    assert_ne!(
        host.interface_vtable_words(shape, ishape).expect("pair"),
        pair_before.as_slice(),
        "activation patches the preserved interface pair"
    );
    assert_eq!(
        host.root_storage(interface_root).expect("interface value"),
        interface_before.as_slice(),
        "the preserved interface value itself is untouched"
    );
    assert_eq!(host.roots().address(record_root), Some(record_address));
    assert_eq!(read_u64(&host, handle_root), record_address as u64);
    assert_eq!(read_u32(&host, result), 121, "no code ran before the cycle");

    host.cycle().expect("commit cycle");
    assert_eq!(host.active_build_id(), Some(v2.header.build_id));
    assert_eq!(read_u32(&host, id_root), 2);
    assert_eq!(read_u32(&host, ticks), 2);
    assert_eq!(
        read_u32(&host, result),
        1432,
        "dispatch through the preserved interface value and class handle runs v2"
    );
    assert_eq!(host.descriptor_address(typeinfo), Some(typeinfo_address));
    assert_eq!(
        host.interface_vtable_address(shape, ishape),
        Some(pair_address)
    );
    assert_eq!(host.roots().address(record_root), Some(record_address));
}

const FRESH_RECORD_V1: &str = r#"
CLASS Thing
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC M : DINT
  M := id;
END_METHOD
END_CLASS

VAR_GLOBAL
  th : Thing;
  counter : DINT;
END_VAR

PROGRAM Main
  th.id := th.id + 1;
  counter := counter + 1;
END_PROGRAM
"#;

const FRESH_RECORD_V2: &str = r#"
CLASS Thing
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC M : DINT
  M := id;
END_METHOD
END_CLASS

CLASS Extra
VAR PUBLIC
  x : DINT;
END_VAR
METHOD PUBLIC Double : DINT
  Double := x * 2;
END_METHOD
END_CLASS

VAR_GLOBAL
  th : Thing;
  ex : Extra;
  counter : DINT;
END_VAR

PROGRAM Main
  th.id := th.id + 1;
  ex.x := ex.x + 2;
  counter := counter + 1;
END_PROGRAM
"#;

#[test]
fn fresh_object_records_initialize_and_roll_back() {
    use stc_runtime::{Host, LoadMode, LoadOutcome, ManifestStore};

    let Some(_) = clang() else {
        return;
    };
    let _guard = host_guard();
    let directory = TestDir::new("online-fresh-record");
    let v1_source = directory.source("fresh_v1", FRESH_RECORD_V1);
    build_ok(&directory, &v1_source, "fresh_v1", &[]);
    let v1 = directory.manifest("fresh_v1");
    let baseline = directory.path.join("fresh_v1.stc-got.bin");
    let v2_source = directory.source("fresh_v2", FRESH_RECORD_V2);
    build_ok(
        &directory,
        &v2_source,
        "fresh_v2",
        &["--got-manifest", baseline.to_str().unwrap()],
    );
    let v2 = directory.manifest("fresh_v2");

    let platform = host_platform();
    let mut host = Host::new(platform.clone(), ManifestStore::memory());
    let outcome = host
        .load(
            open_candidate(&directory, "fresh_v1", &platform),
            LoadMode::Auto,
        )
        .expect("cold install");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));
    host.cycle().expect("cycle v1");

    let preserved_handle = root_id(&v1, "_ROOT::TH#HANDLE");
    let preserved_record = root_id(&v1, "_ROOT::TH#OBJREC");
    let preserved_leaf = root_id(&v1, "_ROOT::TH.ID");
    let preserved_address = host
        .roots()
        .address(preserved_record)
        .expect("preserved record");
    assert_eq!(read_u64(&host, preserved_handle), preserved_address as u64);
    assert_eq!(read_u32(&host, preserved_leaf), 1);

    let extra_type = type_id(&v2, "_ROOT::EXTRA");
    let extra_typeinfo = descriptor_id(&v2, extra_type, DescriptorKind::TypeInfo);
    let extra_record = root_id(&v2, "_ROOT::EX#OBJREC");
    let extra_handle = root_id(&v2, "_ROOT::EX#HANDLE");
    let extra_leaf = root_id(&v2, "_ROOT::EX.X");

    let outcome = host
        .load(
            open_candidate(&directory, "fresh_v2", &platform),
            LoadMode::Auto,
        )
        .expect("prepare");
    assert!(matches!(outcome, LoadOutcome::Prepared { .. }));

    // Prepare initializes the fresh record and handle once; the record names
    // its TYPEINFO and its leaf root, and the handle points at the record.
    assert!(host.roots().contains(extra_record));
    assert!(host.roots().contains(extra_handle));
    assert!(host.roots().contains(extra_leaf));
    let extra_record_address = host.roots().address(extra_record).expect("fresh record");
    let extra_typeinfo_address = host
        .descriptor_address(extra_typeinfo)
        .expect("fresh TYPEINFO");
    assert_eq!(read_u64(&host, extra_handle), extra_record_address as u64);
    assert_eq!(
        u64::from_le_bytes(
            host.root_storage(extra_record).expect("record")[..8]
                .try_into()
                .unwrap()
        ),
        extra_typeinfo_address as u64
    );
    assert_eq!(
        u64::from_le_bytes(
            host.root_storage(extra_record).expect("record")[8..16]
                .try_into()
                .unwrap()
        ),
        host.roots().address(extra_leaf).expect("leaf") as u64
    );
    assert_eq!(host.root_storage(extra_leaf).expect("leaf"), [0; 4]);
    // The preserved object and its state still sit at their addresses.
    assert_eq!(
        host.roots().address(preserved_record),
        Some(preserved_address)
    );
    assert_eq!(read_u32(&host, preserved_leaf), 1);
    assert_eq!(read_u32(&host, root_id(&v1, "_ROOT::COUNTER")), 1);

    host.activate().expect("activate");
    host.rollback().expect("rollback");
    assert_eq!(host.active_build_id(), Some(v1.header.build_id));
    assert!(!host.roots().contains(extra_record));
    assert!(!host.roots().contains(extra_handle));
    assert!(!host.roots().contains(extra_leaf));
    assert!(host.descriptor_address(extra_typeinfo).is_none());
    assert_eq!(host.quarantined_descriptors(), 0);
    assert_eq!(
        host.roots().address(preserved_record),
        Some(preserved_address)
    );
    assert_eq!(read_u32(&host, preserved_leaf), 1);
}

const VTABLE_SHAPE_V1: &str = r#"
CLASS Thing
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC M : DINT
  M := id;
END_METHOD
END_CLASS

VAR_GLOBAL
  th : Thing;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

const VTABLE_SHAPE_V2: &str = r#"
CLASS Thing
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC M : DINT
  M := id;
END_METHOD
METHOD PUBLIC N : DINT
  N := id + 1;
END_METHOD
END_CLASS

VAR_GLOBAL
  th : Thing;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

#[test]
fn vtable_shape_changes_are_rejected_by_a_baseline_build() {
    let Some(_) = clang() else {
        return;
    };
    let _guard = host_guard();
    let directory = TestDir::new("vtable-shape");
    let v1_source = directory.source("shape_v1", VTABLE_SHAPE_V1);
    build_ok(&directory, &v1_source, "shape_v1", &[]);
    let baseline = directory.path.join("shape_v1.stc-got.bin");

    let v2_source = directory.source("shape_v2", VTABLE_SHAPE_V2);
    let rejected = build(
        &directory,
        &v2_source,
        "shape_v2",
        &["--got-manifest", baseline.to_str().unwrap()],
    );
    assert_eq!(rejected.status.code(), Some(1));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&rejected.stdout),
        String::from_utf8_lossy(&rejected.stderr)
    );
    assert!(text.contains("STC-E-ONLINE-003"), "{text}");
    assert!(text.contains("vtable shape"), "{text}");
}

const SINGLE_TASK: &str = r#"
VAR_GLOBAL
  A : DINT;
  B : DINT;
END_VAR

PROGRAM PA
  A := A + 1;
END_PROGRAM

PROGRAM PB
  B := B + 1;
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  TASK Startup (SINGLE := TRUE);
  PROGRAM a WITH Startup : PA;
  PROGRAM b WITH Startup : PB;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn single_task_guards_execute_once_through_the_got() {
    use stc_runtime::{Candidate, Host, HostPlatform, LoadMode, LoadOutcome, ManifestStore};

    let Some(_) = clang() else {
        return;
    };
    let _guard = host_guard();
    let directory = TestDir::new("single");
    let source = directory.source("single", SINGLE_TASK);
    build_ok(&directory, &source, "single", &[]);
    let manifest = directory.manifest("single");

    let platform = HostPlatform::new(
        stc_runtime::PROFILE,
        stc_codegen::resolve_target(Some(&stc_codegen::host_triple())),
        stc_got::HOST_ABI_MAJOR,
        stc_got::HOST_ABI_MINOR,
    );
    let mut host = Host::new(platform.clone(), ManifestStore::memory());
    let candidate = Candidate::open(
        &platform,
        &directory.path.join(module_file(&directory, "single")),
        &directory.path.join("single.stc-got.bin"),
    )
    .expect("candidate");
    let outcome = host.load(candidate, LoadMode::Auto).expect("cold install");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));

    for _ in 0..3 {
        host.cycle().expect("cycle");
    }
    for key in ["_ROOT::A", "_ROOT::B"] {
        let got_id = manifest
            .roots
            .iter()
            .find(|root| root.key == key)
            .unwrap_or_else(|| panic!("root `{key}`"))
            .got_id;
        let storage = host.root_storage(got_id).expect("root storage");
        assert_eq!(
            u32::from_le_bytes(storage[..4].try_into().expect("four bytes")),
            1,
            "`{key}` runs only in the first cycle"
        );
    }
}

const V2_TRAPS: &str = r#"
VAR_GLOBAL
  Counter : DINT;
  Flag : BOOL;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks / 0;
  ticks := ticks + 2;
  Counter := ticks;
  Flag := Counter > 4;
END_PROGRAM
"#;

#[test]
fn a_rollback_window_trap_restores_the_previous_generation() {
    use stc_runtime::{
        Candidate, CycleOutcome, Host, HostPlatform, LoadMode, LoadOutcome, ManifestStore,
    };

    let Some(_) = clang() else {
        return;
    };
    let _guard = host_guard();
    let directory = TestDir::new("trap_rollback");
    let v1_source = directory.source("trap_v1", V1);
    build_ok(&directory, &v1_source, "trap_v1", &[]);
    let v1 = directory.manifest("trap_v1");
    let baseline_path = directory.path.join("trap_v1.stc-got.bin");

    let v2_source = directory.source("trap_v2", V2_TRAPS);
    build_ok(
        &directory,
        &v2_source,
        "trap_v2",
        &["--got-manifest", baseline_path.to_str().unwrap()],
    );
    let v2 = directory.manifest("trap_v2");

    let platform = HostPlatform::new(
        stc_runtime::PROFILE,
        stc_codegen::resolve_target(Some(&stc_codegen::host_triple())),
        stc_got::HOST_ABI_MAJOR,
        stc_got::HOST_ABI_MINOR,
    );
    let mut host = Host::new(platform.clone(), ManifestStore::memory());
    let counter = v1
        .roots
        .iter()
        .find(|root| root.key == "_ROOT::COUNTER")
        .expect("counter root")
        .got_id;

    let v1_candidate = Candidate::open(
        &platform,
        &directory.path.join(module_file(&directory, "trap_v1")),
        &directory.path.join("trap_v1.stc-got.bin"),
    )
    .expect("v1 candidate");
    assert!(matches!(
        host.load(v1_candidate, LoadMode::Auto)
            .expect("cold install"),
        LoadOutcome::ColdInstalled { .. }
    ));
    host.root_storage_mut(counter).expect("counter")[..4].copy_from_slice(&41_u32.to_le_bytes());

    let v2_candidate = Candidate::open(
        &platform,
        &directory.path.join(module_file(&directory, "trap_v2")),
        &directory.path.join("trap_v2.stc-got.bin"),
    )
    .expect("v2 candidate");
    assert!(matches!(
        host.load(v2_candidate, LoadMode::Auto).expect("prepare"),
        LoadOutcome::Prepared { .. }
    ));
    host.activate().expect("activate");
    assert_eq!(host.active_build_id(), Some(v2.header.build_id));

    let outcome = host.cycle().expect("trapping cycle");
    let CycleOutcome::RolledBack { reason, .. } = outcome else {
        panic!("expected rollback, got {outcome:?}");
    };
    assert!(reason.contains("trapped"), "{reason}");
    assert_eq!(host.active_build_id(), Some(v1.header.build_id));
    assert_eq!(
        u32::from_le_bytes(
            host.root_storage(counter).expect("counter")[..4]
                .try_into()
                .unwrap()
        ),
        41
    );
    assert!(host.last_rollback().is_some());
    assert!(matches!(
        host.cycle().expect("cycle after rollback"),
        CycleOutcome::Ran { .. }
    ));
}

// ---- Phase 8: scalar preservation, rename, reject/cold, and direct remap ----

const INITIALIZED_V1: &str = r#"
VAR_GLOBAL RETAIN
  Counter : DINT := 41;
  Flag : BOOL := TRUE;
END_VAR

PROGRAM Main
VAR RETAIN
  ticks : DINT := 7;
END_VAR
  ticks := ticks + 1;
  Counter := ticks;
  Flag := Counter > 100;
END_PROGRAM
"#;

const INITIALIZED_V2: &str = r#"
VAR_GLOBAL RETAIN
  Counter : DINT := 99;
  Flag : BOOL := FALSE;
END_VAR

PROGRAM Main
VAR RETAIN
  ticks : DINT := 9;
END_VAR
  ticks := ticks + 2;
  Counter := ticks;
  Flag := Counter > 100;
END_PROGRAM
"#;

const INITIALIZED_V3: &str = r#"
VAR_GLOBAL RETAIN
  Counter : DINT := 99;
  Flag : BOOL := FALSE;
  Added : DINT := 5;
END_VAR

PROGRAM Main
VAR RETAIN
  ticks : DINT := 9;
END_VAR
  ticks := ticks + 2;
  Counter := ticks;
  Flag := Counter > 100;
  Added := Added + 1;
END_PROGRAM
"#;

#[test]
fn preserved_roots_survive_and_new_roots_initialize_once() {
    use stc_runtime::{Host, LoadMode, LoadOutcome, ManifestStore, PlanDecision};

    let Some(_) = clang() else {
        return;
    };
    let _guard = host_guard();
    let directory = TestDir::new("init");
    let v1_source = directory.source("init_v1", INITIALIZED_V1);
    build_ok(&directory, &v1_source, "init_v1", &[]);
    let v1 = directory.manifest("init_v1");
    let baseline = directory.path.join("init_v1.stc-got.bin");

    let v2_source = directory.source("init_v2", INITIALIZED_V2);
    build_ok(
        &directory,
        &v2_source,
        "init_v2",
        &["--got-manifest", baseline.to_str().unwrap()],
    );
    let v2 = directory.manifest("init_v2");
    assert_eq!(v2.plan.verdict, PlanVerdict::OnlineCandidate);
    assert!(
        v2.plan
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == stc_got::codes::W_ONLINE_INITIALIZER),
        "changed initializers on preserved roots are diagnosed"
    );

    let platform = host_platform();
    let mut host = Host::new(platform.clone(), ManifestStore::memory());
    let outcome = host
        .load(
            open_candidate(&directory, "init_v1", &platform),
            LoadMode::Auto,
        )
        .expect("cold install");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));
    let counter = root_id(&v1, "_ROOT::COUNTER");
    let flag = root_id(&v1, "_ROOT::FLAG");
    let ticks = key_with(&v1, ".TICKS#");
    assert_eq!(
        read_u32(&host, counter),
        41,
        "cold install applies initializers"
    );
    assert_ne!(host.root_storage(flag).expect("flag")[0], 0);
    assert_eq!(read_u32(&host, ticks), 7);
    host.cycle().expect("cycle");
    assert_eq!(read_u32(&host, counter), 8);
    assert_eq!(read_u32(&host, ticks), 8);
    assert_eq!(host.root_storage(flag).expect("flag")[0], 0);

    let candidate = open_candidate(&directory, "init_v2", &platform);
    let diff = host.diff(&candidate).expect("diff");
    assert_eq!(diff.decision, Some(PlanDecision::Online));
    assert!(
        diff.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == stc_got::codes::W_ONLINE_INITIALIZER)
    );
    assert!(matches!(
        host.load(candidate, LoadMode::Auto).expect("prepare"),
        LoadOutcome::Prepared { .. }
    ));
    assert_eq!(
        read_u32(&host, counter),
        8,
        "an online prepare must not reinitialize a preserved root"
    );
    assert_eq!(read_u32(&host, ticks), 8);
    assert_eq!(host.root_storage(flag).expect("flag")[0], 0);
    host.activate().expect("activate");
    assert_eq!(host.active_build_id(), Some(v2.header.build_id));
    host.cycle().expect("commit");
    assert_eq!(read_u32(&host, counter), 10);
    assert_eq!(read_u32(&host, ticks), 10);

    let baseline = directory.path.join("init_v2.stc-got.bin");
    let v3_source = directory.source("init_v3", INITIALIZED_V3);
    build_ok(
        &directory,
        &v3_source,
        "init_v3",
        &["--got-manifest", baseline.to_str().unwrap()],
    );
    let v3 = directory.manifest("init_v3");
    let added = root_id(&v3, "_ROOT::ADDED");
    let outcome = host
        .load(
            open_candidate(&directory, "init_v3", &platform),
            LoadMode::Auto,
        )
        .expect("prepare v3");
    assert!(matches!(outcome, LoadOutcome::Prepared { .. }));
    assert_eq!(
        read_u32(&host, added),
        5,
        "a new root initializes exactly once during prepare"
    );
    host.activate().expect("activate v3");
    host.cycle().expect("commit v3");
    assert_eq!(read_u32(&host, added), 6);
    assert_eq!(read_u32(&host, counter), 12);
    assert_eq!(read_u32(&host, ticks), 12);
}

const RENAME_V1: &str = r#"
VAR_GLOBAL
  Counter : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  Counter := ticks;
END_PROGRAM
"#;

const RENAME_V2: &str = r#"
VAR_GLOBAL
  Count : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  Count := ticks;
END_PROGRAM
"#;

#[test]
fn tracked_rename_is_executed_by_the_host() {
    use stc_runtime::{Host, LoadMode, LoadOutcome, ManifestStore};

    let Some(_) = clang() else {
        return;
    };
    let _guard = host_guard();
    let directory = TestDir::new("rename_host");
    let v1_source = directory.source("rename_v1", RENAME_V1);
    build_ok(&directory, &v1_source, "rename_v1", &[]);
    let v1 = directory.manifest("rename_v1");
    let baseline = directory.path.join("rename_v1.stc-got.bin");
    let v2_source = directory.source("rename_v2", RENAME_V2);
    build_ok(
        &directory,
        &v2_source,
        "rename_v2",
        &[
            "--got-manifest",
            baseline.to_str().unwrap(),
            "--rename",
            "_ROOT::COUNTER=_ROOT::COUNT",
        ],
    );
    let v2 = directory.manifest("rename_v2");
    let counter = root_id(&v1, "_ROOT::COUNTER");
    let renamed = root_id(&v2, "_ROOT::COUNT");
    assert_eq!(renamed, counter, "the tracked rename keeps the identity");
    assert_eq!(
        v2.roots
            .iter()
            .find(|root| root.got_id == renamed)
            .expect("renamed root")
            .previous_keys,
        vec!["_ROOT::COUNTER".to_owned()]
    );

    let platform = host_platform();
    let mut host = Host::new(platform.clone(), ManifestStore::memory());
    host.load(
        open_candidate(&directory, "rename_v1", &platform),
        LoadMode::Auto,
    )
    .expect("cold install");
    let ticks = key_with(&v1, ".TICKS#");
    write_u32(&mut host, counter, 41);
    write_u32(&mut host, ticks, 7);

    let outcome = host
        .load(
            open_candidate(&directory, "rename_v2", &platform),
            LoadMode::Auto,
        )
        .expect("prepare");
    assert!(matches!(outcome, LoadOutcome::Prepared { .. }));
    host.activate().expect("activate");
    assert_eq!(
        host.roots().get(counter).expect("renamed root").record.key,
        "_ROOT::COUNT"
    );
    assert_eq!(read_u32(&host, counter), 41);
    host.cycle().expect("commit");
    assert_eq!(read_u32(&host, counter), 8);
    assert_eq!(read_u32(&host, ticks), 8);
}

const REJECT_V1: &str = r#"
VAR_GLOBAL
  Counter : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  Counter := ticks;
END_PROGRAM
"#;

const REJECT_V2: &str = r#"
VAR_GLOBAL
  Counter : LINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  Counter := ticks;
END_PROGRAM
"#;

#[test]
fn incompatible_layout_is_rejected_and_a_cold_build_resets() {
    use stc_runtime::{Host, LoadMode, LoadOutcome, ManifestStore};

    let Some(_) = clang() else {
        return;
    };
    let _guard = host_guard();
    let directory = TestDir::new("reject");
    let v1_source = directory.source("reject_v1", REJECT_V1);
    build_ok(&directory, &v1_source, "reject_v1", &[]);
    let v1 = directory.manifest("reject_v1");
    let baseline = directory.path.join("reject_v1.stc-got.bin");

    let v2_source = directory.source("reject_v2", REJECT_V2);
    let rejected = build(
        &directory,
        &v2_source,
        "reject_v2",
        &["--got-manifest", baseline.to_str().unwrap()],
    );
    assert!(
        !rejected.status.success(),
        "a layout change against the exact baseline is rejected"
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&rejected.stdout),
        String::from_utf8_lossy(&rejected.stderr)
    );
    assert!(text.contains("STC-E-ONLINE-001"), "{text}");

    let cold_source = directory.source("reject_v2_cold", REJECT_V2);
    build_ok(&directory, &cold_source, "reject_v2_cold", &["--cold"]);
    let cold = directory.manifest("reject_v2_cold");
    assert_eq!(cold.plan.verdict, PlanVerdict::Cold);

    let platform = host_platform();
    let mut host = Host::new(platform.clone(), ManifestStore::memory());
    host.load(
        open_candidate(&directory, "reject_v1", &platform),
        LoadMode::Auto,
    )
    .expect("cold install");
    let counter = root_id(&v1, "_ROOT::COUNTER");
    write_u32(&mut host, counter, 41);
    let outcome = host
        .load(
            open_candidate(&directory, "reject_v2_cold", &platform),
            LoadMode::Auto,
        )
        .expect("stale auto");
    assert!(outcome.is_cold_required());
    assert_eq!(
        read_u32(&host, counter),
        41,
        "the old version keeps running"
    );
    let outcome = host
        .load(
            open_candidate(&directory, "reject_v2_cold", &platform),
            LoadMode::Cold,
        )
        .expect("cold reset");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));
    assert_eq!(read_u32(&host, counter), 0, "a cold install resets state");
}

const REMOVAL_V1: &str = r#"
VAR_GLOBAL
  Counter : DINT;
  Flag : BOOL;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  Counter := ticks;
  Flag := Counter > 3;
END_PROGRAM
"#;

const REMOVAL_V2: &str = r#"
VAR_GLOBAL
  Counter : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  Counter := ticks;
END_PROGRAM
"#;

#[test]
fn untracked_removal_requires_an_explicit_policy() {
    use stc_runtime::{Host, LoadMode, ManifestStore};

    let Some(_) = clang() else {
        return;
    };
    let _guard = host_guard();
    let directory = TestDir::new("removal");
    let v1_source = directory.source("removal_v1", REMOVAL_V1);
    build_ok(&directory, &v1_source, "removal_v1", &[]);
    let v1 = directory.manifest("removal_v1");
    let baseline = directory.path.join("removal_v1.stc-got.bin");

    let v2_source = directory.source("removal_v2", REMOVAL_V2);
    let refused = build(
        &directory,
        &v2_source,
        "removal_v2",
        &["--got-manifest", baseline.to_str().unwrap()],
    );
    assert!(!refused.status.success());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(text.contains("STC-E-MANIFEST-003"), "{text}");
    assert!(text.contains("--allow-untracked-removal"), "{text}");

    build_ok(
        &directory,
        &v2_source,
        "removal_v2",
        &[
            "--got-manifest",
            baseline.to_str().unwrap(),
            "--allow-untracked-removal",
        ],
    );
    let v2 = directory.manifest("removal_v2");
    let flag = root_id(&v1, "_ROOT::FLAG");
    assert!(
        v2.plan
            .actions
            .iter()
            .any(|action| { action.got_id == flag && action.action == stc_got::PlanAction::Drop })
    );

    let platform = host_platform();
    let mut host = Host::new(platform.clone(), ManifestStore::memory());
    host.load(
        open_candidate(&directory, "removal_v1", &platform),
        LoadMode::Auto,
    )
    .expect("cold install");
    let counter = root_id(&v1, "_ROOT::COUNTER");
    write_u32(&mut host, counter, 7);
    host.root_storage_mut(flag).expect("flag")[0] = 1;
    host.load(
        open_candidate(&directory, "removal_v2", &platform),
        LoadMode::Auto,
    )
    .expect("prepare");
    host.activate().expect("activate");
    assert!(
        host.warnings().iter().any(|warning| {
            warning.code == stc_got::codes::W_ONLINE_REMOVED
                && warning.message.contains("_ROOT::FLAG")
        }),
        "the removed root is reported: {:?}",
        host.warnings()
    );
    assert!(
        host.roots()
            .quarantined()
            .iter()
            .any(|root| root.record.got_id == flag)
    );
    assert_eq!(read_u32(&host, counter), 7);
    host.cycle().expect("commit");
    assert_eq!(host.reset_quarantine().expect("reset"), 1);

    let cold_source = directory.source("removal_v2_cold", REMOVAL_V2);
    build_ok(&directory, &cold_source, "removal_v2_cold", &["--cold"]);
    let outcome = host
        .load(
            open_candidate(&directory, "removal_v2_cold", &platform),
            LoadMode::Auto,
        )
        .expect("stale auto");
    assert!(outcome.is_cold_required());
    assert!(!host.roots().contains(flag));
}

const DIRECT_REMAP_V1: &str = r#"
VAR_GLOBAL
  Input AT %ID0 : DINT;
  Marker AT %MD0 : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  Marker := Marker + Input;
END_PROGRAM
"#;

const DIRECT_REMAP_V2: &str = r#"
VAR_GLOBAL
  Input AT %ID1 : DINT;
  Marker AT %MD0 : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  Marker := Marker + Input;
END_PROGRAM
"#;

#[test]
fn direct_variables_remap_across_a_real_online_change() {
    use stc_runtime::{Host, LoadMode, LoadOutcome, ManifestStore};

    let Some(_) = clang() else {
        return;
    };
    let _guard = host_guard();
    let directory = TestDir::new("direct_remap");
    let v1_source = directory.source("direct_v1", DIRECT_REMAP_V1);
    build_ok(&directory, &v1_source, "direct_v1", &[]);
    let v1 = directory.manifest("direct_v1");
    let baseline = directory.path.join("direct_v1.stc-got.bin");
    let v2_source = directory.source("direct_v2", DIRECT_REMAP_V2);
    build_ok(
        &directory,
        &v2_source,
        "direct_v2",
        &["--got-manifest", baseline.to_str().unwrap()],
    );
    let v2 = directory.manifest("direct_v2");

    let platform = host_platform();
    let mut host = Host::new(platform.clone(), ManifestStore::memory());
    host.load(
        open_candidate(&directory, "direct_v1", &platform),
        LoadMode::Auto,
    )
    .expect("cold install");
    assert_eq!(image_bytes().len(), 8, "4 input bytes and 4 marker bytes");
    write_image(0, &1000_u32.to_le_bytes());
    write_image(4, &42_u32.to_le_bytes());
    let input = root_id(&v1, "_ROOT::INPUT");
    let marker = root_id(&v1, "_ROOT::MARKER");
    let old_base = image_base();
    assert_eq!(
        host.runtime_layout()
            .entries
            .iter()
            .find(|entry| entry.got_id == input)
            .and_then(|entry| entry.storage),
        Some(old_base)
    );

    assert!(matches!(
        host.load(
            open_candidate(&directory, "direct_v2", &platform),
            LoadMode::Auto,
        )
        .expect("prepare"),
        LoadOutcome::Prepared { .. }
    ));
    assert_eq!(
        image_bytes().len(),
        8,
        "prepare leaves the active image alone"
    );
    host.activate().expect("activate");
    let bytes = image_bytes();
    assert_eq!(bytes.len(), 12, "the input area grew from 4 to 8 bytes");
    assert_eq!(&bytes[4..8], &1000_u32.to_le_bytes());
    assert_eq!(&bytes[8..12], &42_u32.to_le_bytes());
    assert!(
        host.warnings().iter().any(|warning| {
            warning.code == stc_got::codes::W_ONLINE_IO && warning.message.contains("INPUT")
        }),
        "the direct remap is reported: {:?}",
        host.warnings()
    );
    let base = image_base();
    let layout = host.runtime_layout();
    assert_eq!(
        layout
            .entries
            .iter()
            .find(|entry| entry.got_id == input)
            .and_then(|entry| entry.storage),
        Some(base + 4)
    );
    assert_eq!(
        layout
            .entries
            .iter()
            .find(|entry| entry.got_id == marker)
            .and_then(|entry| entry.storage),
        Some(base + 8)
    );

    host.cycle().expect("cycle");
    assert_eq!(
        &image_bytes()[8..12],
        &(42_u32 + 1000).to_le_bytes(),
        "the new code reads the remapped input cell"
    );
    assert_eq!(host.active_build_id(), Some(v2.header.build_id));

    let cold_source = directory.source("direct_v2_cold", DIRECT_REMAP_V2);
    build_ok(&directory, &cold_source, "direct_v2_cold", &["--cold"]);
    let outcome = host
        .load(
            open_candidate(&directory, "direct_v2_cold", &platform),
            LoadMode::Auto,
        )
        .expect("stale auto");
    assert!(outcome.is_cold_required());
}

const MULTI_TASK: &str = r#"
VAR_GLOBAL
  ORDER : DINT;
  ET_HIGH : TIME;
  ET_LOW : TIME;
END_VAR

PROGRAM Free
  ORDER := ORDER * 10 + 1;
END_PROGRAM
PROGRAM HighA
VAR
  ton : TON;
END_VAR
  ORDER := ORDER * 10 + 2;
  ton(IN := TRUE, PT := T#100ms);
  ET_HIGH := ton.ET;
END_PROGRAM
PROGRAM LowB
VAR
  ton : TON;
END_VAR
  ORDER := ORDER * 10 + 3;
  ton(IN := TRUE, PT := T#100ms);
  ET_LOW := ton.ET;
END_PROGRAM
PROGRAM Timed
  ORDER := ORDER * 10 + 4;
END_PROGRAM
PROGRAM Once
  ORDER := ORDER * 10 + 5;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  TASK Low (PRIORITY := 10);
  TASK High (PRIORITY := 1);
  TASK Tick (INTERVAL := T#10ms);
  TASK OnceTask (SINGLE := TRUE);
  PROGRAM low WITH Low : LowB;
  PROGRAM high WITH High : HighA;
  PROGRAM tick WITH Tick : Timed;
  PROGRAM once WITH OnceTask : Once;
  PROGRAM free : Free;
END_RESOURCE
END_CONFIGURATION
"#;

fn root_got_id(manifest: &Manifest, key: &str) -> GotId {
    manifest
        .roots
        .iter()
        .find(|root| root.key == key)
        .unwrap_or_else(|| panic!("root `{key}`"))
        .got_id
}

fn read_i32_host(host: &stc_runtime::Host, manifest: &Manifest, key: &str) -> i32 {
    let storage = host
        .root_storage(root_got_id(manifest, key))
        .expect("root storage");
    i32::from_le_bytes(storage[..4].try_into().expect("four bytes"))
}

fn read_i64_host(host: &stc_runtime::Host, manifest: &Manifest, key: &str) -> i64 {
    let storage = host
        .root_storage(root_got_id(manifest, key))
        .expect("root storage");
    i64::from_le_bytes(storage[..8].try_into().expect("eight bytes"))
}

#[test]
fn multi_task_scheduling_orders_due_work_and_latches_one_clock() {
    use stc_runtime::{Candidate, Host, HostPlatform, LoadMode, LoadOutcome, ManifestStore};

    let Some(_) = clang() else {
        return;
    };
    let _guard = host_guard();
    let directory = TestDir::new("multi-task");
    let source = directory.source("multi", MULTI_TASK);
    build_ok(&directory, &source, "multi", &[]);
    let manifest = directory.manifest("multi");

    let platform = HostPlatform::new(
        stc_runtime::PROFILE,
        stc_codegen::resolve_target(Some(&stc_codegen::host_triple())),
        stc_got::HOST_ABI_MAJOR,
        stc_got::HOST_ABI_MINOR,
    );
    let mut host = Host::new(platform.clone(), ManifestStore::memory());
    let candidate = Candidate::open(
        &platform,
        &directory.path.join(module_file(&directory, "multi")),
        &directory.path.join("multi.stc-got.bin"),
    )
    .expect("candidate");
    assert!(matches!(
        host.load(candidate, LoadMode::Auto).expect("cold install"),
        LoadOutcome::ColdInstalled { .. }
    ));

    // Unbound `free` runs first, then priority order High, Low, and SINGLE
    // `Once`; the interval task is not due on the first scan.
    host.cycle_at(0).expect("first scan");
    assert_eq!(read_i32_host(&host, &manifest, "_ROOT::ORDER"), 1235);
    assert_eq!(read_i64_host(&host, &manifest, "_ROOT::ET_HIGH"), 0);
    assert_eq!(read_i64_host(&host, &manifest, "_ROOT::ET_LOW"), 0);

    // At the interval deadline the timed task joins the same scan; both
    // timers observe one latched cycle time, and `Once` never runs again.
    host.cycle_at(10_000_000).expect("second scan");
    assert_eq!(read_i32_host(&host, &manifest, "_ROOT::ORDER"), 12_351_234);
    assert_eq!(
        read_i64_host(&host, &manifest, "_ROOT::ET_HIGH"),
        10_000_000
    );
    assert_eq!(read_i64_host(&host, &manifest, "_ROOT::ET_LOW"), 10_000_000);
}
