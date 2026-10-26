mod common;

use common::{MAIN_ST, TempProject, codes, manifest_with};

fn write_package(project: &TempProject, directory: &str, name: &str) {
    project.write(
        &format!("{directory}/stc.toml"),
        &manifest_with(name, "src/**/*.st"),
    );
    project.write(&format!("{directory}/src/main.st"), MAIN_ST);
}

fn write_app(project: &TempProject, dependencies: &str) {
    project.manifest(&format!(
        "{}\n[dependencies]\n{dependencies}",
        manifest_with("app", "src/**/*.st")
    ));
    project.write("src/main.st", MAIN_ST);
}

#[test]
fn lockfile_rendering_is_deterministic() {
    let project = TempProject::new("deterministic");
    write_package(&project, "lib", "lib");
    write_app(&project, "lib = { path = \"lib\" }");
    let first = project.load().expect("first load");
    let first_text = std::fs::read_to_string(&first.lockfile_path).expect("lockfile");
    std::fs::remove_file(&first.lockfile_path).expect("remove lockfile");
    let second = project.load().expect("second load");
    let second_text = std::fs::read_to_string(&second.lockfile_path).expect("lockfile");
    assert_eq!(first_text, second_text);
    assert!(first_text.ends_with('\n'));
}

#[test]
fn lockfile_sorts_packages_and_dependency_lists() {
    let project = TempProject::new("sorting");
    write_package(&project, "a", "a");
    write_package(&project, "z", "z");
    write_app(&project, "z = { path = \"z\" }\na = { path = \"a\" }");
    let graph = project.load().expect("load");
    let names: Vec<&str> = graph
        .lockfile
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect();
    assert_eq!(names, vec!["a", "app", "z"]);
    let root = graph
        .lockfile
        .packages
        .iter()
        .find(|package| package.name == "app")
        .expect("root entry");
    assert_eq!(root.dependencies, vec!["a", "z"]);
}

#[test]
fn locked_requires_an_existing_lockfile() {
    let project = TempProject::new("locked-missing");
    write_app(&project, "");
    let error = project.load_locked().expect_err("missing lockfile");
    assert_eq!(codes(&error), vec!["STC-E-LOCK-001"]);
    assert!(error.diagnostics[0].message.contains("--locked"));
    assert!(!project.path("stc.lock").exists());
}

#[test]
fn locked_accepts_a_matching_lockfile() {
    let project = TempProject::new("locked-ok");
    write_package(&project, "lib", "lib");
    write_app(&project, "lib = { path = \"lib\" }");
    project.load().expect("write lockfile");
    let graph = project.load_locked().expect("matching lockfile");
    assert_eq!(graph.packages.len(), 2);
}

#[test]
fn locked_rejects_changed_versions_and_paths() {
    let project = TempProject::new("locked-version");
    write_package(&project, "lib", "lib");
    write_app(&project, "lib = { path = \"lib\" }");
    project.load().expect("write lockfile");
    let lib_manifest = std::fs::read_to_string(project.path("lib/stc.toml")).unwrap();
    project.write(
        "lib/stc.toml",
        &lib_manifest.replace("version = \"0.1.0\"", "version = \"0.2.0\""),
    );
    let error = project.load_locked().expect_err("changed version");
    assert_eq!(codes(&error), vec!["STC-E-LOCK-002"]);

    let project = TempProject::new("locked-path");
    write_package(&project, "lib", "lib");
    write_package(&project, "lib2", "lib");
    write_app(&project, "lib = { path = \"lib\" }");
    project.load().expect("write lockfile");
    let manifest = std::fs::read_to_string(project.path("stc.toml")).unwrap();
    project.manifest(&manifest.replace("path = \"lib\"", "path = \"lib2\""));
    let error = project.load_locked().expect_err("changed path");
    assert_eq!(codes(&error), vec!["STC-E-LOCK-002"]);
}

#[test]
fn locked_rejects_a_new_package() {
    let project = TempProject::new("locked-new-package");
    write_package(&project, "lib", "lib");
    write_package(&project, "extra", "extra");
    write_app(&project, "lib = { path = \"lib\" }");
    project.load().expect("write lockfile");
    let manifest = std::fs::read_to_string(project.path("stc.toml")).unwrap();
    project.manifest(&format!("{manifest}\nextra = {{ path = \"extra\" }}\n"));
    let error = project.load_locked().expect_err("new package");
    assert_eq!(codes(&error), vec!["STC-E-LOCK-002"]);
}

#[test]
fn a_normal_build_rewrites_a_stale_lockfile() {
    let project = TempProject::new("update");
    write_package(&project, "lib", "lib");
    write_app(&project, "lib = { path = \"lib\" }");
    project.load().expect("write lockfile");
    let lib_manifest = std::fs::read_to_string(project.path("lib/stc.toml")).unwrap();
    project.write(
        "lib/stc.toml",
        &lib_manifest.replace("version = \"0.1.0\"", "version = \"0.2.0\""),
    );
    project.load().expect("update lockfile");
    let text = std::fs::read_to_string(project.path("stc.lock")).unwrap();
    assert!(text.contains("version = \"0.2.0\""), "{text}");
}

#[test]
fn a_malformed_lockfile_is_an_error_even_without_locked() {
    let project = TempProject::new("malformed");
    write_app(&project, "");
    project.write("stc.lock", "schema = 2\n");
    let error = project.load().expect_err("bad schema");
    assert_eq!(codes(&error), vec!["STC-E-LOCK-003"]);

    let project = TempProject::new("malformed-toml");
    write_app(&project, "");
    project.write("stc.lock", "[[package]\n");
    let error = project.load().expect_err("bad toml");
    assert_eq!(codes(&error), vec!["STC-E-LOCK-003"]);
}

#[test]
fn an_unsorted_lockfile_is_rewritten_in_canonical_form() {
    let project = TempProject::new("unsorted");
    write_package(&project, "a", "a");
    write_package(&project, "z", "z");
    write_app(&project, "a = { path = \"a\" }\nz = { path = \"z\" }");
    project.write(
        "stc.lock",
        r#"schema = 1

[[package]]
name = "z"
version = "0.1.0"
source = { path = "z" }
dependencies = []

[[package]]
name = "app"
version = "0.1.0"
source = "root"
dependencies = ["z", "a"]

[[package]]
name = "a"
version = "0.1.0"
source = { path = "a" }
dependencies = []
"#,
    );
    project.load().expect("normalize lockfile");
    let text = std::fs::read_to_string(project.path("stc.lock")).unwrap();
    let a = text.find("name = \"a\"").expect("a entry");
    let app = text.find("name = \"app\"").expect("app entry");
    let z = text.find("name = \"z\"").expect("z entry");
    assert!(a < app && app < z, "{text}");
    assert!(text.contains("dependencies = [\"a\", \"z\"]"), "{text}");
}

#[test]
fn locked_does_not_modify_the_lockfile() {
    let project = TempProject::new("locked-readonly");
    write_app(&project, "");
    project.load().expect("write lockfile");
    let before = std::fs::read_to_string(project.path("stc.lock")).unwrap();
    project.load_locked().expect("matching");
    let after = std::fs::read_to_string(project.path("stc.lock")).unwrap();
    assert_eq!(before, after);
}
