//! Selected machine-profile loading shared by the driver and the IDE (D-253).

mod common;

use common::{TempProject, manifest_with};
use stc_manifest::{load_project_profile, parse_manifest};
use stc_span::FileId;

fn manifest(_project: &TempProject, config: Option<&str>) -> stc_manifest::Manifest {
    let mut text = manifest_with("app", "src/**/*.st");
    if let Some(config) = config {
        text.push_str(&format!("\n[io]\nconfig = \"{config}\"\n"));
    }
    parse_manifest(&text, FileId::new(0))
        .expect("valid manifest")
        .manifest
}

#[test]
fn no_selection_returns_none() {
    let project = TempProject::new("profile-none");
    let manifest = manifest(&project, None);
    assert!(
        load_project_profile(project.root(), &manifest)
            .expect("no profile")
            .is_none()
    );
}

#[test]
fn a_selected_profile_returns_its_canonical_path_and_text() {
    let project = TempProject::new("profile-valid");
    project.write("io/bench.toml", "# profile\n");
    let manifest = manifest(&project, Some("io/bench.toml"));
    let loaded = load_project_profile(project.root(), &manifest)
        .expect("profile loads")
        .expect("profile selected");
    assert_eq!(loaded.selected, "io/bench.toml");
    assert_eq!(loaded.text, "# profile\n");
    assert_eq!(
        loaded.path,
        project
            .path("io/bench.toml")
            .canonicalize()
            .expect("canonical")
    );
}

#[test]
fn a_missing_profile_is_a_project_error() {
    let project = TempProject::new("profile-missing");
    let manifest = manifest(&project, Some("io/missing.toml"));
    let error = load_project_profile(project.root(), &manifest).expect_err("missing profile");
    assert_eq!(error.code, "STC-E-PROJECT-009");
    assert_eq!(error.selected, "io/missing.toml");
    assert!(
        error.message.contains("cannot be read"),
        "{}",
        error.message
    );
}

#[test]
fn an_invalid_path_is_a_project_error() {
    let project = TempProject::new("profile-invalid");
    let mut manifest = manifest(&project, Some("io/bench.toml"));
    manifest.io.config = Some("bad\0path.toml".to_owned());
    let error = load_project_profile(project.root(), &manifest).expect_err("invalid path");
    assert_eq!(error.code, "STC-E-PROJECT-009");
    assert!(
        error.message.contains("cannot be read"),
        "{}",
        error.message
    );
}

#[test]
fn an_escaping_profile_is_rejected() {
    let project = TempProject::new("profile-escape");
    let outside_name = format!(
        "{}-outside.toml",
        project
            .root()
            .file_name()
            .expect("temp name")
            .to_string_lossy()
    );
    let outside = project
        .root()
        .parent()
        .expect("temp parent")
        .join(&outside_name);
    std::fs::write(&outside, "# outside\n").expect("write outside profile");
    let mut manifest = manifest(&project, Some("io/bench.toml"));
    manifest.io.config = Some(format!("../{outside_name}"));
    let error = load_project_profile(project.root(), &manifest).expect_err("escaping path");
    assert_eq!(error.code, "STC-E-PROJECT-009");
    assert!(
        error.message.contains("outside the project root"),
        "{}",
        error.message
    );
    let _ = std::fs::remove_file(outside);
}

#[test]
fn an_unreadable_profile_is_a_project_error() {
    let project = TempProject::new("profile-directory");
    std::fs::create_dir_all(project.path("io/bench.toml")).expect("create directory");
    let manifest = manifest(&project, Some("io/bench.toml"));
    let error = load_project_profile(project.root(), &manifest).expect_err("unreadable profile");
    assert_eq!(error.code, "STC-E-PROJECT-009");
    assert!(
        error.message.contains("cannot be read"),
        "{}",
        error.message
    );
}
