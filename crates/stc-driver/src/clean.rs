//! `stc clean`: remove the configured output directory.
//!
//! Only the out-dir tree is removed. `stc.lock` and retained-state files are
//! never deleted: a state file that lives inside the out-dir is preserved and
//! the directory is left in place holding it. Paths are made absolute and
//! lexically normalized before the containment checks so `--out-dir .` cannot
//! bypass the project-root guard.

use std::io;
use std::path::{Component, Path, PathBuf};

use stc_manifest::{LoadOptions, load_project};

use crate::report::{Log, Reporter, display_path};

/// Removes the project's configured output directory.
pub(crate) fn run(
    manifest: Option<&str>,
    out_dir_override: Option<&str>,
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
    let configured = match out_dir_override {
        Some(value) => PathBuf::from(value),
        None => root.root_dir.join(&root.manifest.build.out_dir),
    };
    let out_dir = comparable(&absolute(&configured));
    if root_dir.starts_with(&out_dir) {
        eprintln!(
            "stc: refusing to clean `{}`: it contains the project root",
            display_path(&out_dir)
        );
        return 2;
    }
    let state_file = root.manifest.retain.file.as_ref().map_or_else(
        || out_dir.join(format!("{}.stcstate", root.name)),
        |file| comparable(&root.root_dir.join(file)),
    );
    let preserve = state_file.starts_with(&out_dir);
    if !out_dir.exists() {
        log.status(format!("nothing to clean: `{}`", display_path(&out_dir)));
        return 0;
    }
    let preserve_path = if preserve {
        Some(state_file.as_path())
    } else {
        None
    };
    if let Err(error) = remove_tree(&out_dir, preserve_path) {
        eprintln!("stc: cannot remove `{}`: {error}", display_path(&out_dir));
        return 1;
    }
    if out_dir.exists() {
        log.status(format!(
            "removed `{}` (kept state file `{}`)",
            display_path(&out_dir),
            display_path(&state_file)
        ));
        return 0;
    }
    remove_empty_parents(&out_dir, &root_dir);
    log.status(format!("removed {}", display_path(&out_dir)));
    0
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

/// Strips the Windows verbatim prefix so canonical and working-directory paths
/// compare consistently.
fn comparable(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    PathBuf::from(text)
}

fn remove_tree(path: &Path, preserve: Option<&Path>) -> io::Result<()> {
    if preserve == Some(path) {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            remove_tree(&entry.path(), preserve)?;
        }
        match std::fs::remove_dir(path) {
            Ok(()) => Ok(()),
            Err(error)
                if error.kind() == io::ErrorKind::DirectoryNotEmpty
                    && preserve.is_some_and(|preserve| preserve.starts_with(path)) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
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
