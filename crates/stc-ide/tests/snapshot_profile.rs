//! Snapshot root-manifest and selected-profile retention (D-253).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use stc_ide::Snapshot;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempProject {
    root: PathBuf,
}

impl TempProject {
    fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "stc-ide-snapshot-{}-{name}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp project");
        Self { root }
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(self.root.clone(), |path, part| path.join(part))
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.path(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create source directory");
        }
        std::fs::write(&path, contents).expect("write test file");
        path
    }

    fn manifest(&self, config: Option<&str>) {
        let mut text = String::from(
            "schema = 1\n\n[package]\nname = \"app\"\nversion = \"0.1.0\"\nlanguage = \"stc-2013-oop-utf8-v1\"\n\n[project]\nsources = [\"src/**/*.st\"]\n",
        );
        if let Some(config) = config {
            text.push_str(&format!("\n[io]\nconfig = \"{config}\"\n"));
        }
        self.write("stc.toml", &text);
        self.write(
            "src/main.st",
            "PROGRAM Main\nVAR\n  counter : DINT;\n  out : DINT;\nEND_VAR\n  counter := counter + 1;\nEND_PROGRAM\n",
        );
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

const PROFILE: &str = "[[routes]]\nsource = { kind = \"plc-output\", variable = \"Main.Counter\" }\nsinks = [{ kind = \"plc-input\", variable = \"Main.Out\" }]\n";

#[test]
fn a_project_without_a_profile_has_none() {
    let project = TempProject::new("no-profile");
    project.manifest(None);
    let snapshot = Snapshot::load(project.root()).expect("snapshot loads");
    assert!(snapshot.profile().is_none());
    assert!(snapshot.profile_path().is_none());
    assert!(snapshot.profile_file().is_none());
    assert!(snapshot.profile_diagnostics().is_empty());
}

#[test]
fn root_manifest_accessors_identify_the_root_package() {
    let project = TempProject::new("root");
    project.manifest(None);
    let snapshot = Snapshot::load(project.root()).expect("snapshot loads");
    assert_eq!(snapshot.root_package(), "app");
    assert_eq!(snapshot.root_namespace(), "app");
    assert_eq!(snapshot.root_manifest().package.name, "app");
    assert_eq!(snapshot.manifest_path(), project.path("stc.toml"));
    let file = snapshot.manifest_file();
    let text = snapshot
        .manifest_sources()
        .file(file)
        .expect("root manifest source")
        .text();
    assert!(text.contains("name = \"app\""), "{text}");
}

#[test]
fn a_valid_profile_is_retained_in_its_own_source_map() {
    let project = TempProject::new("valid-profile");
    project.manifest(Some("io/bench.toml"));
    project.write("io/bench.toml", PROFILE);
    let snapshot = Snapshot::load(project.root()).expect("snapshot loads");
    let profile = snapshot.profile().expect("profile parsed");
    assert_eq!(profile.routes.len(), 1);
    assert_eq!(
        snapshot.profile_path(),
        Some(
            project
                .path("io/bench.toml")
                .canonicalize()
                .expect("canonical")
                .as_path()
        )
    );
    let file = snapshot.profile_file().expect("profile file");
    assert_eq!(
        snapshot
            .profile_sources()
            .file(file)
            .expect("profile source")
            .text(),
        PROFILE
    );
    assert_eq!(
        snapshot.profile_sources().file_path(file),
        Some("io/bench.toml")
    );
    assert!(snapshot.profile_diagnostics().is_empty());
}

#[test]
fn a_missing_profile_is_stored_as_an_error() {
    let project = TempProject::new("missing-profile");
    project.manifest(Some("io/missing.toml"));
    let snapshot = Snapshot::load(project.root()).expect("snapshot still loads");
    assert!(snapshot.profile().is_none());
    assert!(snapshot.profile_path().is_none());
    assert_eq!(snapshot.profile_diagnostics().len(), 1);
    assert_eq!(snapshot.profile_diagnostics()[0].code, "STC-E-PROJECT-009");
}

#[test]
fn an_invalid_profile_is_stored_as_an_error() {
    let project = TempProject::new("invalid-profile");
    project.manifest(Some("io/bench.toml"));
    project.write(
        "io/bench.toml",
        "[[routes]]\nsource = { kind = \"plc-output\" }\nsinks = []\n",
    );
    let snapshot = Snapshot::load(project.root()).expect("snapshot still loads");
    assert!(snapshot.profile().is_none());
    assert!(snapshot.profile_path().is_some());
    assert!(snapshot.profile_file().is_some());
    assert!(
        !snapshot.profile_diagnostics().is_empty(),
        "expected profile parse diagnostics"
    );
}

#[test]
fn an_escaping_profile_is_stored_as_an_error() {
    let project = TempProject::new("escaping-profile");
    let outside = project.root().parent().expect("temp parent").join(format!(
        "{}-outside.toml",
        project
            .root()
            .file_name()
            .expect("temp name")
            .to_string_lossy()
    ));
    std::fs::write(&outside, PROFILE).expect("write outside profile");
    project.manifest(Some("io/bench.toml"));
    std::fs::create_dir_all(project.path("io")).expect("create io directory");
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, project.path("io/bench.toml")).is_ok();
    #[cfg(windows)]
    let linked =
        std::os::windows::fs::symlink_file(&outside, project.path("io/bench.toml")).is_ok();
    if !linked {
        // Creating symlinks needs privileges on some hosts; the loader's
        // escape rule is covered directly by the stc-manifest tests.
        let _ = std::fs::remove_file(outside);
        return;
    }
    let snapshot = Snapshot::load(project.root()).expect("snapshot still loads");
    assert!(snapshot.profile().is_none());
    assert!(
        snapshot
            .profile_diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message.contains("outside the project root")),
        "{:?}",
        snapshot.profile_diagnostics()
    );
    let _ = std::fs::remove_file(outside);
}
