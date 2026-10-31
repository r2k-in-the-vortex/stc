//! Protocol source-path mapping (WL-1.1 I-10).
//!
//! Compiler and protocol output names sources with package-relative,
//! `/`-separated paths (`src/main.st`). DAP clients use absolute host paths.
//! This module maps between the two using the project graph: each protocol
//! path resolves against the owning package's root directory, with an optional
//! `sourceRoot` override. Comparison normalizes `\` to `/`, strips Windows
//! verbatim/device prefixes, and folds ASCII case on Windows. A source that is
//! not in the graph is reported as unbound rather than guessed.

use std::path::{Path, PathBuf};

use stc_manifest::{LoadOptions, ProjectGraph};

/// One source known to the project graph.
#[derive(Clone, Debug)]
struct SourceEntry {
    /// Canonical package-relative spelling from the graph.
    relative: String,
    /// Normalized comparison key.
    key: String,
    /// Absolute host path.
    absolute: PathBuf,
}

/// One package's source root and files.
#[derive(Clone, Debug)]
struct PackageSource {
    root_key: String,
    sources: Vec<SourceEntry>,
}

/// Maps protocol source paths to absolute workspace paths and back.
#[derive(Clone, Debug, Default)]
pub struct SourceTable {
    packages: Vec<PackageSource>,
    source_root: Option<PathBuf>,
    source_root_key: Option<String>,
}

impl SourceTable {
    /// Loads the project graph without writing `stc.lock` and builds the table.
    ///
    /// # Errors
    ///
    /// Returns the first project diagnostic when the manifest or project graph
    /// cannot be loaded.
    pub fn from_project(manifest: &Path, source_root: Option<&Path>) -> Result<Self, String> {
        let graph = stc_manifest::load_project_readonly(manifest, &LoadOptions::default())
            .map_err(|error| {
                error.diagnostics.first().map_or_else(
                    || "the project manifest could not be loaded".into(),
                    |diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message),
                )
            })?;
        Ok(Self::from_graph(&graph, source_root))
    }

    /// Builds the table from an already loaded project graph.
    #[must_use]
    pub fn from_graph(graph: &ProjectGraph, source_root: Option<&Path>) -> Self {
        let packages = graph
            .packages
            .iter()
            .map(|package| PackageSource {
                root_key: path_key(&package.root_dir),
                sources: package
                    .sources
                    .iter()
                    .map(|source| SourceEntry {
                        relative: source.relative.clone(),
                        key: protocol_key(&source.relative),
                        absolute: normalize_absolute(&source.path),
                    })
                    .collect(),
            })
            .collect();
        let source_root = source_root.map(Path::to_path_buf);
        let source_root_key = source_root.as_deref().map(path_key);
        Self {
            packages,
            source_root,
            source_root_key,
        }
    }

    /// Returns whether the table has no packages.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// Maps an absolute client path to the protocol's package-relative path.
    ///
    /// Returns `None` when the path is outside every source root or names a
    /// file that is not a source in the project graph.
    #[must_use]
    pub fn protocol_path(&self, absolute: &Path) -> Option<String> {
        let absolute_key = path_key(absolute);
        if let Some(root_key) = &self.source_root_key {
            let relative = strip_component(&absolute_key, root_key)?;
            return self.canonical_relative(relative);
        }
        let mut best: Option<(&PackageSource, &str)> = None;
        for package in &self.packages {
            let Some(relative) = strip_component(&absolute_key, &package.root_key) else {
                continue;
            };
            if best.is_none_or(|(best, _)| package.root_key.len() > best.root_key.len()) {
                best = Some((package, relative));
            }
        }
        let (package, relative) = best?;
        package
            .source_by_key(relative)
            .map(|source| source.relative.clone())
    }

    /// Maps a protocol package-relative path to an absolute client path.
    ///
    /// The root package wins when several packages carry the same relative
    /// path; otherwise the dependency order decides. Returns `None` for a path
    /// that is not a source in the project graph.
    #[must_use]
    pub fn absolute_path(&self, protocol: &str) -> Option<PathBuf> {
        let key = protocol_key(protocol);
        if key.is_empty() {
            return None;
        }
        if let Some(root) = &self.source_root {
            let relative = self.canonical_relative(&key)?;
            return Some(normalize_absolute(&join_relative(root, &relative)));
        }
        // `ProjectGraph` orders dependencies before the root package; iterate
        // in reverse so the root package wins when several packages carry the
        // same relative path.
        self.packages
            .iter()
            .rev()
            .find_map(|package| package.source_by_key(&key))
            .map(|source| normalize_absolute(&source.absolute))
    }

    /// Returns the canonical graph spelling for a normalized relative key.
    fn canonical_relative(&self, key: &str) -> Option<String> {
        self.packages
            .iter()
            .rev()
            .find_map(|package| package.source_by_key(key))
            .map(|source| source.relative.clone())
    }
}

impl PackageSource {
    fn source_by_key(&self, key: &str) -> Option<&SourceEntry> {
        self.sources.iter().find(|source| source.key == key)
    }
}

/// Removes the Windows verbatim prefix from an absolute path.
///
/// The project graph may carry `\\?\C:\...` paths after canonicalization, and
/// DAP clients treat that as a different file from the normal spelling, which
/// opens a second editor tab on every stop. The DAP `Source.path` is emitted
/// without the prefix; comparison keys are normalized separately.
#[cfg(windows)]
fn normalize_absolute(path: &Path) -> PathBuf {
    let mut components = path.components();
    match components.next() {
        Some(std::path::Component::Prefix(prefix)) => match prefix.kind() {
            std::path::Prefix::VerbatimDisk(disk) => {
                let mut base = std::ffi::OsString::from(format!("{}:", disk as char));
                base.push(components.as_path());
                PathBuf::from(base)
            }
            std::path::Prefix::VerbatimUNC(server, share) => {
                let mut base = std::ffi::OsString::from(r"\\");
                base.push(server);
                base.push(r"\");
                base.push(share);
                base.push(components.as_path());
                PathBuf::from(base)
            }
            _ => path.to_path_buf(),
        },
        _ => path.to_path_buf(),
    }
}

#[cfg(not(windows))]
fn normalize_absolute(path: &Path) -> PathBuf {
    path.to_path_buf()
}

/// Normalizes `\` to `/`, strips Windows verbatim/device prefixes, and removes
/// a trailing separator.
fn normalize_slashes(text: &str) -> String {
    let mut text = text.replace('\\', "/");
    if let Some(rest) = strip_prefix_ci(&text, "//?/UNC/") {
        text = format!("//{rest}");
    } else if let Some(rest) = strip_prefix_ci(&text, "//?/") {
        text = rest.to_owned();
    } else if let Some(rest) = strip_prefix_ci(&text, "//./") {
        text = rest.to_owned();
    }
    while text.len() > 1 && text.ends_with('/') {
        text.pop();
    }
    text
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// Returns the normalized comparison key for an absolute path.
///
/// `\` becomes `/`, Windows verbatim/device prefixes are stripped, a trailing
/// separator is removed, and ASCII case is folded on Windows.
#[must_use]
pub(crate) fn path_key(path: &Path) -> String {
    let text = normalize_slashes(&path.to_string_lossy());
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text
    }
}

/// Returns the comparison key for a protocol path.
fn protocol_key(path: &str) -> String {
    let mut text = normalize_slashes(path);
    while let Some(rest) = text.strip_prefix("./") {
        text = rest.to_owned();
    }
    let text = text.trim_start_matches('/');
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text.to_owned()
    }
}

/// Strips `root` plus one separator from a normalized path.
fn strip_component<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    if root.is_empty() {
        return Some(path);
    }
    if path == root {
        return Some("");
    }
    let rest = path.strip_prefix(root)?;
    let rest = rest.strip_prefix('/')?;
    Some(rest)
}

fn join_relative(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(source_root: Option<&str>) -> SourceTable {
        let packages = vec![
            PackageSource {
                root_key: path_key(Path::new("C:/work/app")),
                sources: vec![SourceEntry {
                    relative: "src/main.st".into(),
                    key: protocol_key("src/main.st"),
                    absolute: PathBuf::from("C:/work/app/src/main.st"),
                }],
            },
            PackageSource {
                root_key: path_key(Path::new("C:/work/lib")),
                sources: vec![SourceEntry {
                    relative: "src/lib.st".into(),
                    key: protocol_key("src/lib.st"),
                    absolute: PathBuf::from("C:/work/lib/src/lib.st"),
                }],
            },
        ];
        SourceTable {
            packages,
            source_root: source_root.map(PathBuf::from),
            source_root_key: source_root.map(|root| path_key(Path::new(root))),
        }
    }

    #[test]
    fn maps_package_relative_sources_both_ways() {
        let table = table(None);
        assert_eq!(
            table.protocol_path(Path::new("C:/work/app/src/main.st")),
            Some("src/main.st".into())
        );
        assert_eq!(
            table.protocol_path(Path::new("C:/work/lib/src/lib.st")),
            Some("src/lib.st".into())
        );
        assert_eq!(
            table.absolute_path("src/main.st"),
            Some(PathBuf::from("C:/work/app/src/main.st"))
        );
        assert_eq!(
            table.absolute_path("src/lib.st"),
            Some(PathBuf::from("C:/work/lib/src/lib.st"))
        );
    }

    #[test]
    fn normalizes_windows_separators_and_verbatim_prefixes() {
        let table = table(None);
        assert_eq!(
            table.protocol_path(Path::new(r"C:\work\app\src\main.st")),
            Some("src/main.st".into())
        );
        assert_eq!(
            table.protocol_path(Path::new(r"\\?\C:\work\app\src\main.st")),
            Some("src/main.st".into())
        );
        assert_eq!(
            table.protocol_path(Path::new(r"\\?\UNC\server\share\src\main.st")),
            None
        );
        assert_eq!(
            table.absolute_path(r"src\main.st"),
            Some(PathBuf::from("C:/work/app/src/main.st"))
        );
    }

    #[test]
    fn path_keys_normalize_one_source_spelling() {
        let normal = path_key(Path::new("C:/work/app/src/main.st"));
        assert_eq!(path_key(Path::new(r"C:\work\app\src\main.st")), normal);
        assert_eq!(path_key(Path::new(r"\\?\C:\work\app\src\main.st")), normal);
        assert_eq!(path_key(Path::new(r"\\?\C:\work\app\src\main.st\")), normal);
        assert_eq!(path_key(Path::new("C:/work/app/src/main.st/")), normal);
        if cfg!(windows) {
            assert_eq!(path_key(Path::new("C:/WORK/App/SRC/Main.ST")), normal);
        }
    }

    #[cfg(windows)]
    #[test]
    fn absolute_paths_strip_windows_verbatim_prefixes() {
        let mut sources = table(None);
        sources.packages[0].sources[0].absolute = PathBuf::from(r"\\?\C:\work\app\src\main.st");
        assert_eq!(
            sources.absolute_path("src/main.st"),
            Some(PathBuf::from(r"C:\work\app\src\main.st"))
        );

        let mut overridden = table(None);
        overridden.source_root = Some(PathBuf::from(r"\\?\C:\checkout"));
        overridden.source_root_key = Some(path_key(Path::new(r"\\?\C:\checkout")));
        assert_eq!(
            overridden.absolute_path("src/main.st"),
            Some(PathBuf::from(r"C:\checkout\src\main.st"))
        );
    }

    #[test]
    fn unbound_sources_are_not_guessed() {
        let table = table(None);
        assert_eq!(
            table.protocol_path(Path::new("C:/work/app/src/other.st")),
            None
        );
        assert_eq!(
            table.protocol_path(Path::new("C:/elsewhere/src/main.st")),
            None
        );
        assert_eq!(table.absolute_path("src/other.st"), None);
        assert_eq!(table.absolute_path(""), None);
    }

    #[test]
    fn source_root_overrides_package_roots() {
        let table = table(Some("C:/checkout"));
        assert_eq!(
            table.protocol_path(Path::new("C:/checkout/src/main.st")),
            Some("src/main.st".into())
        );
        assert_eq!(
            table.protocol_path(Path::new("C:/work/app/src/main.st")),
            None
        );
        assert_eq!(
            table.absolute_path("src/main.st"),
            Some(PathBuf::from("C:/checkout").join("src").join("main.st"))
        );
        assert_eq!(table.absolute_path("src/other.st"), None);
    }

    #[test]
    fn comparison_folds_ascii_case_on_windows() {
        let table = table(None);
        let result = table.protocol_path(Path::new("C:/WORK/App/SRC/Main.ST"));
        if cfg!(windows) {
            assert_eq!(result, Some("src/main.st".into()));
        } else {
            assert_eq!(result, None);
        }
    }
}
