mod common;

use common::{MAIN_ST, TempProject, codes, manifest_with};
use stc_manifest::load_project;

fn project(name: &str, sources: &str) -> TempProject {
    let project = TempProject::new(name);
    project.manifest(&manifest_with("app", sources));
    project
}

fn relative(paths: &[stc_manifest::SourcePath]) -> Vec<String> {
    paths.iter().map(|path| path.relative.clone()).collect()
}

#[test]
fn glob_discovery_is_sorted_and_excludes_hidden_and_target() {
    let project = project("sorted", "src/**/*.st");
    project.write("src/main.st", MAIN_ST);
    project.write("src/util/a.st", MAIN_ST);
    project.write("src/util/b.st", MAIN_ST);
    project.write("src/.hidden/ignored.st", MAIN_ST);
    project.write("target/generated.st", MAIN_ST);
    project.write("src/readme.md", "not ST");
    project.manifest(&format!(
        "{}\n[test]\nsources = [\"tests/**/*.st\"]\n",
        manifest_with("app", "src/**/*.st")
    ));
    project.write("tests/smoke.st", MAIN_ST);
    let graph = project.load().expect("valid project");
    let root = graph.root_package();
    assert_eq!(
        relative(&root.sources),
        vec!["src/main.st", "src/util/a.st", "src/util/b.st"]
    );
    assert_eq!(relative(&root.test_sources), vec!["tests/smoke.st"]);
}

#[test]
fn recursive_glob_excludes_hidden_and_target_subtrees() {
    let project = project("hidden", "**/*.st");
    project.write(".hidden/x.st", MAIN_ST);
    project.write("target/gen.st", MAIN_ST);
    project.write("nested/target/gen.st", MAIN_ST);
    let error = project.load().expect_err("nothing matches");
    assert_eq!(codes(&error), vec!["STC-E-PROJECT-004"]);
    assert!(error.diagnostics[0].message.contains("matched no files"));
}

#[test]
fn wildcards_question_marks_and_classes() {
    let question = project("wild", "src/?ain.st");
    question.write("src/main.st", MAIN_ST);
    question.write("src/main2.st", MAIN_ST);
    let graph = question.load().expect("valid project");
    assert_eq!(relative(&graph.root_package().sources), vec!["src/main.st"]);

    let class = project("class", "src/[m-p]ain.st");
    class.write("src/main.st", MAIN_ST);
    class.write("src/xain.st", MAIN_ST);
    let graph = class.load().expect("valid project");
    assert_eq!(relative(&graph.root_package().sources), vec!["src/main.st"]);

    let star = project("star", "src/main*.st");
    star.write("src/main.st", MAIN_ST);
    star.write("src/main2.st", MAIN_ST);
    star.write("src/other.st", MAIN_ST);
    let graph = star.load().expect("valid project");
    assert_eq!(
        relative(&graph.root_package().sources),
        vec!["src/main.st", "src/main2.st"]
    );
}

#[test]
fn a_glob_matching_nothing_is_an_error() {
    let project = project("empty", "src/**/*.st");
    project.write("other/only.st", MAIN_ST);
    let error = project.load().expect_err("no match");
    assert_eq!(codes(&error), vec!["STC-E-PROJECT-004"]);
}

#[test]
fn only_st_files_are_discovered() {
    let project = project("non-st", "**/*.txt");
    project.write("notes.txt", "hello");
    let error = project.load().expect_err("no ST match");
    assert_eq!(codes(&error), vec!["STC-E-PROJECT-004"]);
}

#[test]
fn duplicate_matches_across_globs_are_errors() {
    let project = project("duplicate", "src/**/*.st");
    project.manifest(&manifest_with("app", "src/**/*.st"));
    let text = std::fs::read_to_string(project.path("stc.toml")).unwrap();
    project.manifest(&text.replace(
        "sources = [\"src/**/*.st\"]",
        "sources = [\"src/**/*.st\", \"src/main.st\"]",
    ));
    project.write("src/main.st", MAIN_ST);
    let error = project.load().expect_err("duplicate match");
    assert_eq!(codes(&error), vec!["STC-E-PROJECT-004"]);
    assert!(error.diagnostics[0].message.contains("more than one glob"));
}

#[test]
fn test_sources_may_not_overlap_project_sources() {
    let project = project("overlap", "src/**/*.st");
    project.write("src/main.st", MAIN_ST);
    let text = std::fs::read_to_string(project.path("stc.toml")).unwrap();
    project.manifest(&format!("{text}\n[test]\nsources = [\"src/**/*.st\"]\n"));
    let error = project.load().expect_err("overlap");
    assert_eq!(codes(&error), vec!["STC-E-TEST-004"]);
}

#[test]
fn matching_is_case_insensitive_on_windows_only() {
    let project = project("case", "SRC/**/*.st");
    project.write("src/main.st", MAIN_ST);
    let result = project.load();
    if cfg!(windows) {
        let graph = result.expect("case-insensitive match");
        assert_eq!(relative(&graph.root_package().sources), vec!["src/main.st"]);
    } else {
        let error = result.expect_err("case-sensitive host");
        assert_eq!(codes(&error), vec!["STC-E-PROJECT-004"]);
    }
}

#[cfg(windows)]
fn symlink_file(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(windows)]
fn symlink_dir(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(unix)]
fn symlink_file(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(unix)]
fn symlink_dir(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[test]
fn a_symlinked_file_inside_the_project_is_accepted() {
    let project = project("symlink-in", "alias.st");
    project.write("src/main.st", MAIN_ST);
    if symlink_file(&project.path("src/main.st"), &project.path("alias.st")).is_err() {
        eprintln!("skipping symlink test: host does not allow symlink creation");
        return;
    }
    let graph = project.load().expect("accepted symlink");
    assert_eq!(relative(&graph.root_package().sources), vec!["alias.st"]);
}

#[test]
fn a_symlinked_file_outside_the_project_is_rejected() {
    let project = project("symlink-out", "outside.st");
    let outside = TempProject::new("symlink-target");
    outside.write("target.st", MAIN_ST);
    if symlink_file(&outside.path("target.st"), &project.path("outside.st")).is_err() {
        eprintln!("skipping symlink test: host does not allow symlink creation");
        return;
    }
    let error = project.load().expect_err("escaping symlink");
    assert_eq!(codes(&error), vec!["STC-E-PROJECT-004"]);
    assert!(error.diagnostics[0].message.contains("outside the project"));
}

#[test]
fn symlinked_directories_are_not_followed() {
    let project = project("symlink-dir", "link/**/*.st");
    project.write("src/main.st", MAIN_ST);
    if symlink_dir(&project.path("src"), &project.path("link")).is_err() {
        eprintln!("skipping symlink test: host does not allow symlink creation");
        return;
    }
    let error = project
        .load()
        .expect_err("directory symlink is not followed");
    assert_eq!(codes(&error), vec!["STC-E-PROJECT-004"]);
}

#[test]
fn an_invalid_utf8_manifest_is_reported() {
    let project = TempProject::new("utf8");
    std::fs::write(project.path("stc.toml"), [0xff, 0xfe, 0x00, 0x41]).expect("write bytes");
    let error = project.load().expect_err("invalid UTF-8");
    assert_eq!(codes(&error), vec!["STC-E-PROJECT-001"]);
    assert!(error.diagnostics[0].message.contains("not valid UTF-8"));
}

#[test]
fn a_missing_manifest_is_reported() {
    let project = TempProject::new("missing-manifest");
    let error = load_project(
        project.path("nope/stc.toml"),
        &stc_manifest::LoadOptions::default(),
    )
    .expect_err("missing manifest");
    assert_eq!(codes(&error), vec!["STC-E-PROJECT-001"]);
}

#[test]
fn a_directory_manifest_path_is_accepted() {
    let project = project("directory", "src/**/*.st");
    project.write("src/main.st", MAIN_ST);
    let graph = load_project(project.root(), &stc_manifest::LoadOptions::default())
        .expect("directory manifest path");
    assert_eq!(
        graph
            .root_package()
            .manifest_path
            .file_name()
            .unwrap()
            .to_string_lossy(),
        "stc.toml"
    );
}
