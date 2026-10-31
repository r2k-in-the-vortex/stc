//! `stc clean`: remove the project's generated artifact directories.
//!
//! Without a selector, `clean` removes the debug, release, and test default
//! trees plus an explicit `[build].out-dir`. `--debug`, `--release`, or
//! `--test` narrows it to one tree, and `--out-dir` selects exactly the given
//! directory. `stc.lock`, retained-state (`*.stcstate`) files, and the runtime
//! module/manifest store are never deleted: a preserved entry keeps its parent
//! directories in place. Paths are made absolute and canonicalized (falling
//! back to lexical normalization when they do not exist) before the
//! containment checks, so a relative or aliased `--out-dir` cannot bypass the
//! project-root guard.

use std::ffi::OsStr;
use std::io;
use std::path::{Component, Path, PathBuf};

use stc_manifest::{LoadOptions, Package, load_project};

use crate::build::{self, BuildMode, Request};
use crate::report::{Log, Reporter, display_path};

/// Default test artifact root (`stc test`).
const DEFAULT_TEST_DIR: &str = "target/stc/test";

/// Default coverage report directory (`stc test --coverage`).
const DEFAULT_COVERAGE_DIR: &str = "target/stc/coverage";

/// Default runtime store root used by `stc run --watch` when
/// `[runtime].manifest-dir` is absent.
const DEFAULT_RUNTIME_DIR: &str = "target/stc/runtime";

/// Removes the project's generated artifact directories.
pub(crate) fn run(
    manifest: Option<&str>,
    request: &Request,
    test_only: bool,
    locked: bool,
    reporter: &mut Reporter,
    log: Log,
) -> u8 {
    let graph = match load_project(manifest.unwrap_or("stc.toml"), &LoadOptions { locked }) {
        Ok(graph) => graph,
        Err(error) => {
            reporter.diagnostics(&error.diagnostics, &error.sources);
            return 2;
        }
    };
    reporter.diagnostics(&graph.warnings, &graph.sources);
    let root = graph.root_package();
    let root_dir = comparable(&root.root_dir);
    let targets = targets(request, test_only, root);
    // Validate every target before deleting anything.
    for target in &targets {
        if root_dir.starts_with(target) {
            eprintln!(
                "stc: refusing to clean `{}`: it contains the project root",
                display_path(target)
            );
            return 2;
        }
    }
    let preserve = preserve_paths(root);
    let existing: Vec<&PathBuf> = targets.iter().filter(|target| target.exists()).collect();
    if existing.is_empty() {
        report_nothing(&targets, log);
        return 0;
    }
    let mut failed = false;
    for target in existing {
        if let Err(error) = remove_tree(target, &preserve) {
            eprintln!("stc: cannot remove `{}`: {error}", display_path(target));
            failed = true;
            continue;
        }
        if target.exists() {
            log.status(format!(
                "removed `{}` (kept retained state or runtime files)",
                display_path(target)
            ));
        } else {
            remove_empty_parents(target, &root_dir);
            log.status(format!("removed {}", display_path(target)));
        }
    }
    u8::from(failed)
}

/// Returns the artifact directories selected by the flags.
///
/// `--out-dir` selects exactly the given directory and takes precedence over
/// the mode and test selectors.
fn targets(request: &Request, test_only: bool, root: &Package) -> Vec<PathBuf> {
    let mut targets = Vec::new();
    if let Some(value) = &request.out_dir {
        targets.push(comparable(&absolute(Path::new(value))));
    } else if test_only {
        targets.push(comparable(&absolute(&root.root_dir.join(DEFAULT_TEST_DIR))));
        targets.push(comparable(&absolute(
            &root.root_dir.join(DEFAULT_COVERAGE_DIR),
        )));
    } else if let Some(mode) = request.mode_override {
        let configured = if root.manifest.build.out_dir_explicit {
            root.root_dir.join(&root.manifest.build.out_dir)
        } else {
            root.root_dir.join(build::default_out_dir(mode))
        };
        targets.push(comparable(&absolute(&configured)));
    } else {
        for mode in [BuildMode::Debug, BuildMode::Release] {
            targets.push(comparable(&absolute(
                &root.root_dir.join(build::default_out_dir(mode)),
            )));
        }
        targets.push(comparable(&absolute(&root.root_dir.join(DEFAULT_TEST_DIR))));
        targets.push(comparable(&absolute(
            &root.root_dir.join(DEFAULT_COVERAGE_DIR),
        )));
        if root.manifest.build.out_dir_explicit {
            targets.push(comparable(&absolute(
                &root.root_dir.join(&root.manifest.build.out_dir),
            )));
        }
    }
    // Remove duplicates and trees nested in another selected tree; removing
    // the ancestor covers them.
    targets.sort();
    targets.dedup();
    let mut selected: Vec<PathBuf> = Vec::with_capacity(targets.len());
    for target in targets {
        if selected.iter().any(|kept| target.starts_with(kept)) {
            continue;
        }
        selected.push(target);
    }
    selected
}

/// Returns paths that survive cleaning even when they live under an artifact
/// directory.
fn preserve_paths(root: &Package) -> Vec<PathBuf> {
    let root_dir = comparable(&root.root_dir);
    let mut preserve = Vec::new();
    if let Some(file) = &root.manifest.retain.file {
        preserve.push(comparable(&root_dir.join(file)));
    }
    match &root.manifest.runtime.manifest_dir {
        Some(dir) => preserve.push(comparable(&root_dir.join(dir))),
        None => preserve.push(comparable(&root_dir.join(DEFAULT_RUNTIME_DIR))),
    }
    if let Some(dir) = &root.manifest.runtime.module_dir {
        preserve.push(comparable(&root_dir.join(dir)));
    }
    preserve.sort();
    preserve.dedup();
    preserve
}

/// Makes `path` absolute (against the working directory) and resolves `.` and
/// `..` lexically.
fn absolute(path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

/// Resolves `path` to its canonical form when it exists and strips the Windows
/// verbatim prefix, so aliases such as 8.3 short names or junctions compare
/// consistently with the working directory and the canonical project root.
fn comparable(path: &Path) -> PathBuf {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let text = resolved.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    PathBuf::from(text)
}

fn report_nothing(targets: &[PathBuf], log: Log) {
    if let [only] = targets {
        log.status(format!("nothing to clean: `{}`", display_path(only)));
        return;
    }
    let list = targets
        .iter()
        .map(|target| format!("`{}`", display_path(target)))
        .collect::<Vec<_>>()
        .join(", ");
    log.status(format!("nothing to clean: {list}"));
}

fn remove_tree(path: &Path, preserve: &[PathBuf]) -> io::Result<()> {
    if preserve.iter().any(|entry| entry.as_path() == path) {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            remove_tree(&entry.path(), preserve)?;
        }
        if std::fs::read_dir(path)?.next().is_some() {
            // Only preserved entries (retained state or a runtime store) are
            // left, so the directory stays.
            return Ok(());
        }
        std::fs::remove_dir(path)
    } else if path.extension() == Some(OsStr::new("stcstate")) {
        // Retained state is never generated output; deleting it is an
        // explicit user action (CLI-and-Manifest §3).
        Ok(())
    } else {
        std::fs::remove_file(path)
    }
}

fn remove_empty_parents(out_dir: &Path, root: &Path) {
    let mut current = out_dir.parent();
    while let Some(directory) = current {
        if directory == root || !directory.starts_with(root) {
            break;
        }
        if std::fs::remove_dir(directory).is_err() {
            break;
        }
        current = directory.parent();
    }
}
