mod common;

use common::{MAIN_ST, TempProject, codes, manifest_with};
use stc_manifest::{LoadOptions, Package, load_project};

fn write_package(project: &TempProject, directory: &str, name: &str, extra: &str) {
    let manifest = format!("{}{extra}", manifest_with(name, "src/**/*.st"),);
    project.write(&format!("{directory}/stc.toml"), &manifest);
    project.write(&format!("{directory}/src/main.st"), MAIN_ST);
}

fn write_app(project: &TempProject, dependencies: &str) {
    let manifest = format!(
        "{}\n[dependencies]\n{dependencies}",
        manifest_with("app", "src/**/*.st")
    );
    project.write("app/stc.toml", &manifest);
    project.write("app/src/main.st", MAIN_ST);
}

fn package_index(graph: &stc_manifest::ProjectGraph, name: &str) -> usize {
    graph
        .packages
        .iter()
        .position(|package| package.name == name)
        .unwrap_or_else(|| panic!("package {name} not found"))
}

fn names(graph: &stc_manifest::ProjectGraph) -> Vec<&str> {
    graph
        .packages
        .iter()
        .map(|package: &Package| package.name.as_str())
        .collect()
}

#[test]
fn dependencies_order_before_dependents_and_write_a_lockfile() {
    let workspace = TempProject::new("two-package");
    write_package(&workspace, "lib", "lib", "");
    write_app(&workspace, "lib = { path = \"../lib\" }");
    let graph = load_project(workspace.path("app"), &LoadOptions::default()).expect("valid graph");
    assert_eq!(names(&graph), vec!["lib", "app"]);
    let root = graph.root_package();
    assert_eq!(root.name, "app");
    assert_eq!(root.namespace, "app");
    assert!(root.is_root);
    assert_eq!(root.dependencies.len(), 1);
    assert_eq!(root.dependencies[0].alias, "lib");
    assert_eq!(graph.package(root.dependencies[0].package).name, "lib");
    assert_eq!(graph.package(root.dependencies[0].package).namespace, "lib");
    assert_eq!(
        root.sources
            .iter()
            .map(|source| source.relative.as_str())
            .collect::<Vec<_>>(),
        vec!["src/main.st"]
    );
    let lockfile = std::fs::read_to_string(&graph.lockfile_path).expect("lockfile written");
    assert!(lockfile.contains("source = \"root\""), "{lockfile}");
    assert!(
        lockfile.contains("source = { path = \"../lib\" }"),
        "{lockfile}"
    );
    assert!(!lockfile.contains('\\'), "{lockfile}");
    let root_entry = graph
        .lockfile
        .packages
        .iter()
        .find(|package| package.name == "app")
        .expect("root entry");
    assert_eq!(root_entry.dependencies, vec!["lib"]);
    let lib_entry = graph
        .lockfile
        .packages
        .iter()
        .find(|package| package.name == "lib")
        .expect("lib entry");
    assert_eq!(lib_entry.dependencies, Vec::<String>::new());
}

#[test]
fn an_explicit_alias_sets_the_namespace_but_not_the_identity() {
    let workspace = TempProject::new("alias");
    write_package(&workspace, "lib", "real-lib", "");
    write_app(
        &workspace,
        "motor-lib = { path = \"../lib\", package = \"real-lib\" }",
    );
    let graph = load_project(workspace.path("app"), &LoadOptions::default()).expect("valid graph");
    let root = graph.root_package();
    let dependency = graph.package(root.dependencies[0].package);
    assert_eq!(dependency.name, "real-lib");
    assert_eq!(dependency.namespace, "motor-lib");
    assert_eq!(root.dependencies[0].alias, "motor-lib");
    let lockfile = std::fs::read_to_string(&graph.lockfile_path).expect("lockfile");
    assert!(lockfile.contains("name = \"real-lib\""), "{lockfile}");
    assert!(
        lockfile.contains("dependencies = [\"motor-lib\"]"),
        "{lockfile}"
    );
}

#[test]
fn alias_and_package_name_mismatches_are_errors() {
    let workspace = TempProject::new("alias-mismatch");
    write_package(&workspace, "lib", "real-lib", "");
    write_app(&workspace, "motor-lib = { path = \"../lib\" }");
    let error = load_project(workspace.path("app"), &LoadOptions::default()).expect_err("mismatch");
    assert_eq!(codes(&error), vec!["STC-E-PROJECT-005"]);
    assert!(error.diagnostics[0].message.contains("does not match"));

    let workspace = TempProject::new("alias-package-mismatch");
    write_package(&workspace, "lib", "real-lib", "");
    write_app(
        &workspace,
        "motor-lib = { path = \"../lib\", package = \"other\" }",
    );
    let error = load_project(workspace.path("app"), &LoadOptions::default()).expect_err("mismatch");
    assert_eq!(codes(&error), vec!["STC-E-PROJECT-005"]);
    assert!(error.diagnostics[0].message.contains("declares package"));
}

#[test]
fn missing_and_nonexistent_dependency_paths_are_errors() {
    let workspace = TempProject::new("missing-path");
    write_app(&workspace, "lib = { version = \"1\" }");
    let error = load_project(workspace.path("app"), &LoadOptions::default()).expect_err("no path");
    assert_eq!(codes(&error), vec!["STC-E-PROJECT-005"]);

    let workspace = TempProject::new("nonexistent-path");
    write_app(&workspace, "lib = { path = \"../missing\" }");
    let error = load_project(workspace.path("app"), &LoadOptions::default()).expect_err("missing");
    assert_eq!(codes(&error), vec!["STC-E-PROJECT-005"]);
}

#[test]
fn dependency_cycles_are_rejected() {
    let workspace = TempProject::new("cycle");
    write_package(
        &workspace,
        "a",
        "a",
        "\n[dependencies]\nb = { path = \"../b\" }\n",
    );
    write_package(
        &workspace,
        "b",
        "b",
        "\n[dependencies]\na = { path = \"../a\" }\n",
    );
    write_app(&workspace, "a = { path = \"../a\" }");
    let error = load_project(workspace.path("app"), &LoadOptions::default()).expect_err("cycle");
    assert!(
        codes(&error).contains(&"STC-E-PROJECT-006"),
        "{:?}",
        codes(&error)
    );
}

#[test]
fn self_dependencies_are_cycles() {
    let workspace = TempProject::new("self-cycle");
    write_app(&workspace, "self = { path = \".\", package = \"app\" }");
    let error = load_project(workspace.path("app"), &LoadOptions::default()).expect_err("cycle");
    assert!(
        codes(&error).contains(&"STC-E-PROJECT-006"),
        "{:?}",
        codes(&error)
    );
}

#[test]
fn duplicate_canonical_dependency_paths_are_rejected() {
    let workspace = TempProject::new("duplicate-path");
    write_package(&workspace, "lib", "lib", "");
    write_app(
        &workspace,
        "first = { path = \"../lib\", package = \"lib\" }\nsecond = { path = \"../lib\", package = \"lib\" }",
    );
    let error =
        load_project(workspace.path("app"), &LoadOptions::default()).expect_err("duplicate");
    assert!(
        codes(&error).contains(&"STC-E-PROJECT-006"),
        "{:?}",
        codes(&error)
    );
}

#[test]
fn a_shared_dependency_is_loaded_once() {
    let workspace = TempProject::new("diamond");
    write_package(&workspace, "c", "c", "");
    write_package(
        &workspace,
        "a",
        "a",
        "\n[dependencies]\nc = { path = \"../c\" }\n",
    );
    write_package(
        &workspace,
        "b",
        "b",
        "\n[dependencies]\nc = { path = \"../c\" }\n",
    );
    write_app(
        &workspace,
        "a = { path = \"../a\" }\nb = { path = \"../b\" }",
    );
    let graph = load_project(workspace.path("app"), &LoadOptions::default()).expect("diamond");
    assert_eq!(graph.packages.len(), 4);
    let c = package_index(&graph, "c");
    let a = package_index(&graph, "a");
    let b = package_index(&graph, "b");
    let app = package_index(&graph, "app");
    assert!(c < a && c < b && a < app && b < app, "{:?}", names(&graph));
    assert_eq!(graph.lockfile.packages.len(), 4);
}

#[test]
fn topological_ties_are_broken_by_package_name() {
    let workspace = TempProject::new("tie-break");
    write_package(&workspace, "a", "a", "");
    write_package(&workspace, "b", "b", "");
    write_app(
        &workspace,
        "b = { path = \"../b\" }\na = { path = \"../a\" }",
    );
    let graph = load_project(workspace.path("app"), &LoadOptions::default()).expect("graph");
    assert_eq!(names(&graph), vec!["a", "b", "app"]);
}

#[test]
fn nested_dependency_sources_are_excluded_from_the_parent_glob() {
    let project = TempProject::new("nested");
    project.manifest(&format!(
        "{}\n[dependencies]\nlib = {{ path = \"deps/lib\" }}\n",
        manifest_with("app", "**/*.st")
    ));
    project.write("src/main.st", MAIN_ST);
    project.write("deps/lib/stc.toml", &manifest_with("lib", "src/**/*.st"));
    project.write("deps/lib/src/lib.st", MAIN_ST);
    let graph = project.load().expect("nested dependency");
    let root = graph.root_package();
    assert_eq!(
        root.sources
            .iter()
            .map(|source| source.relative.as_str())
            .collect::<Vec<_>>(),
        vec!["src/main.st"]
    );
    let lib = graph.package(root.dependencies[0].package);
    assert_eq!(
        lib.sources
            .iter()
            .map(|source| source.relative.as_str())
            .collect::<Vec<_>>(),
        vec!["src/lib.st"]
    );
}

#[test]
fn conflicting_aliases_for_one_package_are_rejected() {
    let workspace = TempProject::new("alias-conflict");
    write_package(&workspace, "lib", "lib", "");
    write_package(
        &workspace,
        "a",
        "a",
        "\n[dependencies]\nleft = { path = \"../lib\", package = \"lib\" }\n",
    );
    write_package(
        &workspace,
        "b",
        "b",
        "\n[dependencies]\nright = { path = \"../lib\", package = \"lib\" }\n",
    );
    write_app(
        &workspace,
        "a = { path = \"../a\" }\nb = { path = \"../b\" }",
    );
    let error =
        load_project(workspace.path("app"), &LoadOptions::default()).expect_err("alias conflict");
    assert!(
        codes(&error).contains(&"STC-E-PROJECT-005"),
        "{:?}",
        codes(&error)
    );
}

#[test]
fn dependency_manifest_errors_are_reported_with_context() {
    let workspace = TempProject::new("bad-dependency");
    workspace.write("lib/stc.toml", "schema = 1\n[package]\nname = \"lib\"\n");
    workspace.write("lib/src/main.st", MAIN_ST);
    write_app(&workspace, "lib = { path = \"../lib\" }");
    let error = load_project(workspace.path("app"), &LoadOptions::default()).expect_err("bad dep");
    assert!(
        codes(&error).contains(&"STC-E-PROJECT-003"),
        "{:?}",
        codes(&error)
    );
    assert!(error.diagnostics.iter().any(|diagnostic| {
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("while loading dependency `lib`"))
    }));
}
