//! Source discovery from `[project].sources` and `[test].sources`.
//!
//! Hidden directories and `target/` are excluded, symlinked directories are
//! not followed, and a symlinked file is accepted only when its real path is
//! inside the package (CLI and Manifest §2.3). Results are sorted by their
//! forward-slash relative path.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use stc_diag::{Diagnostic, Severity};
use stc_span::Span;

use crate::glob::Pattern;
use crate::model::{GlobSpec, SourcePath};

/// One source file matched by a glob.
#[derive(Clone, Debug)]
pub(crate) struct DiscoveredSource {
    /// Absolute path and package-relative spelling.
    pub source: SourcePath,
    /// Canonical path used for duplicate/overlap checks.
    pub canonical: PathBuf,
    /// The glob that matched this file.
    pub span: Span,
}

/// Resolves all globs for one package.
///
/// Returns every discovered file on success. On failure, returns one or more
/// diagnostics and no files.
pub(crate) fn discover(
    root: &Path,
    globs: &[GlobSpec],
    exclusions: &[PathBuf],
    fallback: Span,
) -> Result<Vec<DiscoveredSource>, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let mut candidates = match walk(root, exclusions) {
        Ok(candidates) => candidates,
        Err(error) => {
            diagnostics.push(Diagnostic::error(
                "STC-E-PROJECT-004",
                format!("cannot read source directory `{}`: {error}", root.display()),
                globs.first().map_or(fallback, |glob| glob.span),
            ));
            return Err(diagnostics);
        }
    };
    candidates.sort_by(|left, right| left.relative.cmp(&right.relative));
    let mut matched: BTreeMap<String, DiscoveredSource> = BTreeMap::new();
    for glob in globs {
        let pattern = match Pattern::parse(&glob.pattern) {
            Ok(pattern) => pattern,
            Err(reason) => {
                diagnostics.push(Diagnostic::error(
                    "STC-E-PROJECT-004",
                    format!("invalid source glob `{}`: {reason}", glob.pattern),
                    glob.span,
                ));
                continue;
            }
        };
        let mut hits = 0_usize;
        for candidate in &candidates {
            if !pattern.matches(&candidate.relative) {
                continue;
            }
            hits += 1;
            let canonical = match canonical_path(candidate) {
                Ok(path) => path,
                Err(error) => {
                    diagnostics.push(Diagnostic::error(
                        "STC-E-PROJECT-004",
                        format!("cannot resolve source `{}`: {error}", candidate.relative),
                        glob.span,
                    ));
                    continue;
                }
            };
            if !canonical.starts_with(root) {
                diagnostics.push(Diagnostic::error(
                    "STC-E-PROJECT-004",
                    format!(
                        "symlinked source `{}` resolves outside the project",
                        candidate.relative
                    ),
                    glob.span,
                ));
                continue;
            }
            if let Some(previous) = matched.get(&candidate.relative) {
                let diagnostic = Diagnostic::error(
                    "STC-E-PROJECT-004",
                    format!(
                        "source file `{}` is matched by more than one glob",
                        candidate.relative
                    ),
                    glob.span,
                )
                .with_label(previous.span, "first matched here");
                diagnostics.push(diagnostic);
                continue;
            }
            matched.insert(
                candidate.relative.clone(),
                DiscoveredSource {
                    source: SourcePath {
                        path: candidate.path.clone(),
                        relative: candidate.relative.clone(),
                    },
                    canonical,
                    span: glob.span,
                },
            );
        }
        if hits == 0 {
            diagnostics.push(Diagnostic::error(
                "STC-E-PROJECT-004",
                format!("source glob `{}` matched no files", glob.pattern),
                glob.span,
            ));
        }
    }
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        Err(diagnostics)
    } else {
        Ok(matched.into_values().collect())
    }
}

fn canonical_path(candidate: &Candidate) -> std::io::Result<PathBuf> {
    if candidate.is_symlink {
        candidate.path.canonicalize()
    } else {
        Ok(candidate.path.clone())
    }
}

#[derive(Debug)]
struct Candidate {
    path: PathBuf,
    relative: String,
    is_symlink: bool,
}

fn walk(root: &Path, exclusions: &[PathBuf]) -> std::io::Result<Vec<Candidate>> {
    let mut candidates = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&directory)? {
            entries.push(entry?);
        }
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)?;
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                let target = match std::fs::metadata(&path) {
                    Ok(target) => target,
                    Err(_) => continue,
                };
                if target.is_file() && has_st_extension(&path) {
                    candidates.push(Candidate {
                        relative: relative_spelling(root, &path),
                        path,
                        is_symlink: true,
                    });
                }
                // Symlinked directories are not followed.
                continue;
            }
            if file_type.is_dir() {
                let name = entry.file_name();
                if is_hidden(&name) || is_target(&name) || is_excluded(&path, exclusions) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if file_type.is_file() && has_st_extension(&path) {
                candidates.push(Candidate {
                    relative: relative_spelling(root, &path),
                    path,
                    is_symlink: false,
                });
            }
        }
    }
    Ok(candidates)
}

fn is_hidden(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
}

fn is_target(name: &OsStr) -> bool {
    crate::host::is_target_dir(name)
}

fn is_excluded(path: &Path, exclusions: &[PathBuf]) -> bool {
    crate::host::is_excluded(path, exclusions)
}

fn has_st_extension(path: &Path) -> bool {
    crate::host::has_st_extension(path)
}

fn relative_spelling(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
