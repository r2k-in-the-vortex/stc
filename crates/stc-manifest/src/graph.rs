//! Dependency resolution and the ordered project graph.
//!
//! Dependencies are local path packages. Paths are canonicalized, packages are
//! deduplicated by canonical directory, cycles are rejected, and the graph is
//! returned in deterministic topological order (CLI and Manifest §2.2/§3).

use std::collections::{BTreeSet, HashMap};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use stc_diag::{Diagnostic, Severity};
use stc_span::{FileId, SourceMap, Span};

use crate::discover::{self, DiscoveredSource};
use crate::lock::{
    LockPackage, LockSource, Lockfile, parse_lockfile, relative_lock_path, render_lockfile,
};
use crate::model::{BuildSettings, Manifest, SourcePath};
use crate::parse::parse_manifest;

/// Maximum dependency nesting accepted by the loader.
const MAX_DEPTH: usize = 128;

/// Options for [`load_project`].
#[derive(Clone, Copy, Debug, Default)]
pub struct LoadOptions {
    /// Require the existing `stc.lock` to match the graph and fail otherwise.
    pub locked: bool,
}

/// An index into [`ProjectGraph::packages`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageId(pub usize);

/// One dependency edge from a package to another package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageDependency {
    /// The alias used by the depending package; also its root namespace.
    pub alias: String,
    /// The resolved package.
    pub package: PackageId,
}

/// One package in the resolved graph.
#[derive(Clone, Debug)]
pub struct Package {
    /// Position in the topological order.
    pub id: PackageId,
    /// Package identity from `[package].name`.
    pub name: String,
    /// Package version.
    pub version: String,
    /// Absolute path of the package manifest.
    pub manifest_path: PathBuf,
    /// Absolute canonical package directory.
    pub root_dir: PathBuf,
    /// Root namespace: the package name for the root, the alias otherwise.
    pub namespace: String,
    /// Whether this is the root package.
    pub is_root: bool,
    /// Dependencies, sorted by alias.
    pub dependencies: Vec<PackageDependency>,
    /// Production sources, sorted by relative path.
    pub sources: Vec<SourcePath>,
    /// Test-only sources, sorted by relative path.
    pub test_sources: Vec<SourcePath>,
    /// The parsed manifest.
    pub manifest: Manifest,
}

impl Package {
    /// Returns this package's validated `[build]` settings.
    #[must_use]
    pub fn build(&self) -> &BuildSettings {
        &self.manifest.build
    }
}

/// The resolved project graph consumed by the driver.
#[derive(Debug)]
pub struct ProjectGraph {
    /// The root package.
    pub root: PackageId,
    /// Packages in topological order (dependencies before dependents).
    pub packages: Vec<Package>,
    /// The resolved lockfile.
    pub lockfile: Lockfile,
    /// Absolute path of `stc.lock`.
    pub lockfile_path: PathBuf,
    /// Manifest text used by the returned spans.
    pub sources: SourceMap,
    /// Non-fatal manifest warnings.
    pub warnings: Vec<Diagnostic>,
}

impl ProjectGraph {
    /// Returns the root package.
    #[must_use]
    pub fn root_package(&self) -> &Package {
        &self.packages[self.root.0]
    }

    /// Returns a package by id.
    #[must_use]
    pub fn package(&self, id: PackageId) -> &Package {
        &self.packages[id.0]
    }
}

/// Project loading failed.
#[derive(Debug)]
pub struct ProjectError {
    /// Errors and warnings collected while loading.
    pub diagnostics: Vec<Diagnostic>,
    /// Manifest text used by the returned spans.
    pub sources: SourceMap,
}

impl ProjectError {
    /// Returns `true` when at least one diagnostic is an error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    /// Returns the stable diagnostic codes.
    pub fn codes(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.diagnostics.iter().map(|diagnostic| diagnostic.code)
    }
}

/// Loads a project graph from `manifest_path`.
///
/// A directory path is interpreted as `<dir>/stc.toml`. Unless
/// [`LoadOptions::locked`] is set, a missing or stale `stc.lock` is written in
/// canonical form.
///
/// # Errors
///
/// Returns [`ProjectError`] with manifest, source, dependency, or lockfile
/// diagnostics.
pub fn load_project(
    manifest_path: impl AsRef<Path>,
    options: &LoadOptions,
) -> Result<ProjectGraph, ProjectError> {
    load_project_impl(manifest_path.as_ref(), options, true)
}

/// Loads a project graph without creating or updating `stc.lock`.
///
/// The graph, source discovery, and manifest diagnostics are identical to
/// [`load_project`]. Without `locked`, a missing or stale lockfile is ignored
/// instead of rewritten, so commands such as `stc fmt --check` have no
/// filesystem side effects.
///
/// # Errors
///
/// Returns [`ProjectError`] with manifest, source, dependency, or lockfile
/// diagnostics. With `locked`, a missing or stale lockfile is still an error.
pub fn load_project_readonly(
    manifest_path: impl AsRef<Path>,
    options: &LoadOptions,
) -> Result<ProjectGraph, ProjectError> {
    load_project_impl(manifest_path.as_ref(), options, false)
}

fn load_project_impl(
    input: &Path,
    options: &LoadOptions,
    write_lock: bool,
) -> Result<ProjectGraph, ProjectError> {
    let manifest_input = if input.is_dir() {
        input.join("stc.toml")
    } else {
        input.to_path_buf()
    };
    let display_root = manifest_input.to_string_lossy().into_owned();
    let canonical_manifest = match manifest_input.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            let mut loader = Loader::new(PathBuf::new(), display_root.clone());
            let file = loader
                .add_source(display_root, String::new())
                .unwrap_or(FileId::new(0));
            loader.error(
                "STC-E-PROJECT-001",
                format!("cannot read project manifest: {error}"),
                Span::new(file, 0, 0),
            );
            return Err(loader.into_error());
        }
    };
    let Some(root_dir) = canonical_manifest.parent().map(Path::to_path_buf) else {
        let mut loader = Loader::new(PathBuf::new(), display_root);
        loader.error(
            "STC-E-PROJECT-001",
            "project manifest has no parent directory",
            Span::new(FileId::new(0), 0, 0),
        );
        return Err(loader.into_error());
    };
    let mut loader = Loader::new(root_dir, display_root);
    let Some(root_node) = loader.load_node(&canonical_manifest, NodeReference::Root, 0) else {
        return Err(loader.into_error());
    };
    loader.check_alias_consistency();
    if loader.has_errors() {
        return Err(loader.into_error());
    }
    let Some(order) = loader.topological_order() else {
        return Err(loader.into_error());
    };
    let mut packages = loader.build_packages(root_node, &order);
    loader.discover_sources(&mut packages);
    if loader.has_errors() {
        return Err(loader.into_error());
    }
    let Some(computed) = loader.compute_lockfile(&packages) else {
        return Err(loader.into_error());
    };
    let lockfile_path = loader.root_dir.join("stc.lock");
    if !loader.check_lockfile(&computed, options.locked, write_lock) {
        return Err(loader.into_error());
    }
    let root = packages
        .iter()
        .position(|package| package.is_root)
        .expect("the root package is in the graph");
    let warnings = std::mem::take(&mut loader.diagnostics);
    debug_assert!(
        warnings
            .iter()
            .all(|diagnostic| diagnostic.severity != Severity::Error),
        "successful load must not retain errors"
    );
    Ok(ProjectGraph {
        root: PackageId(root),
        packages,
        lockfile: computed,
        lockfile_path,
        sources: loader.sources,
        warnings,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NodeState {
    Loading,
    Loaded,
}

#[derive(Clone, Debug)]
enum NodeReference {
    Root,
    Dependency { alias: String, path_span: Span },
}

impl NodeReference {
    fn span(&self) -> Span {
        match self {
            Self::Root => Span::new(FileId::new(0), 0, 0),
            Self::Dependency { path_span, .. } => *path_span,
        }
    }

    fn alias(&self) -> Option<&str> {
        match self {
            Self::Root => None,
            Self::Dependency { alias, .. } => Some(alias),
        }
    }
}

#[derive(Clone, Debug)]
struct Edge {
    alias: String,
    span: Span,
    path_span: Span,
    package: usize,
}

#[derive(Debug)]
struct Node {
    canonical_dir: PathBuf,
    manifest_path: PathBuf,
    manifest: Manifest,
    edges: Vec<Edge>,
    state: NodeState,
}

struct Loader {
    root_dir: PathBuf,
    root_display: String,
    sources: SourceMap,
    diagnostics: Vec<Diagnostic>,
    nodes: Vec<Node>,
    by_dir: HashMap<PathBuf, usize>,
}

impl Loader {
    fn new(root_dir: PathBuf, root_display: String) -> Self {
        Self {
            root_dir,
            root_display,
            sources: SourceMap::new(),
            diagnostics: Vec::new(),
            nodes: Vec::new(),
            by_dir: HashMap::new(),
        }
    }

    fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    fn error(&mut self, code: &'static str, message: impl Into<String>, span: Span) {
        self.diagnostics
            .push(Diagnostic::error(code, message, span));
    }

    fn into_error(self) -> ProjectError {
        ProjectError {
            diagnostics: self.diagnostics,
            sources: self.sources,
        }
    }

    fn add_source(&mut self, display: String, text: String) -> Option<FileId> {
        if text.len() > u32::MAX as usize {
            let file = self.sources.add_file(display, String::new());
            let span = Span::new(file, 0, 0);
            self.error(
                "STC-E-PROJECT-001",
                "project manifest exceeds the 4 GiB source limit",
                span,
            );
            return None;
        }
        Some(self.sources.add_file(display, text))
    }

    fn manifest_display(&self, canonical_dir: &Path) -> String {
        if canonical_dir == self.root_dir {
            return self.root_display.clone();
        }
        match relative_lock_path(&self.root_dir, canonical_dir) {
            Some(relative) if relative != "." => format!("{relative}/stc.toml"),
            _ => canonical_dir
                .join("stc.toml")
                .to_string_lossy()
                .into_owned(),
        }
    }

    fn read_manifest(&mut self, canonical_dir: &Path) -> Option<(String, FileId)> {
        let manifest_path = canonical_dir.join("stc.toml");
        let display = self.manifest_display(canonical_dir);
        let bytes = match std::fs::read(&manifest_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                let file = self.add_source(display.clone(), String::new())?;
                self.error(
                    "STC-E-PROJECT-001",
                    format!("cannot read `{display}`: {error}"),
                    Span::new(file, 0, 0),
                );
                return None;
            }
        };
        let (text, utf8_ok) = match String::from_utf8(bytes) {
            Ok(text) => (text, true),
            Err(error) => (
                String::from_utf8_lossy(error.as_bytes()).into_owned(),
                false,
            ),
        };
        let file = self.add_source(display.clone(), text.clone())?;
        if !utf8_ok {
            self.error(
                "STC-E-PROJECT-001",
                format!("`{display}` is not valid UTF-8"),
                Span::new(file, 0, 0),
            );
            return None;
        }
        Some((text, file))
    }

    fn load_node(
        &mut self,
        manifest_path: &Path,
        reference: NodeReference,
        depth: usize,
    ) -> Option<usize> {
        let canonical_dir = manifest_path.parent()?.to_path_buf();
        if depth > MAX_DEPTH {
            self.error(
                "STC-E-PROJECT-005",
                format!("dependency graph is deeper than {MAX_DEPTH} levels"),
                reference.span(),
            );
            return None;
        }
        if let Some(&id) = self.by_dir.get(&canonical_dir) {
            if self.nodes[id].state == NodeState::Loading {
                let name = self.nodes[id].manifest.package.name.clone();
                let diagnostic = Diagnostic::error(
                    "STC-E-PROJECT-006",
                    format!("dependency cycle detected through `{name}`"),
                    reference.span(),
                )
                .with_note(format!(
                    "package `{name}` is being loaded from `{}`",
                    self.nodes[id].manifest_path.display()
                ));
                self.diagnostics.push(diagnostic);
                return None;
            }
            return Some(id);
        }
        let (text, file) = self.read_manifest(&canonical_dir)?;
        let parsed = match parse_manifest(&text, file) {
            Ok(parsed) => parsed,
            Err(diagnostics) => {
                for mut diagnostic in diagnostics {
                    if let Some(alias) = reference.alias() {
                        diagnostic
                            .notes
                            .push(format!("while loading dependency `{alias}`"));
                    }
                    self.diagnostics.push(diagnostic);
                }
                return None;
            }
        };
        self.diagnostics.extend(parsed.warnings);
        let id = self.nodes.len();
        self.nodes.push(Node {
            canonical_dir: canonical_dir.clone(),
            manifest_path: manifest_path.to_path_buf(),
            manifest: parsed.manifest,
            edges: Vec::new(),
            state: NodeState::Loading,
        });
        self.by_dir.insert(canonical_dir.clone(), id);

        let dependencies = self.nodes[id].manifest.dependencies.clone();
        for declaration in dependencies {
            let raw = Path::new(&declaration.path);
            let candidate = if raw.is_absolute() {
                raw.to_path_buf()
            } else {
                canonical_dir.join(raw)
            };
            let dependency_dir = match candidate.canonicalize() {
                Ok(path) if path.is_dir() => path,
                Ok(_) => {
                    self.error(
                        "STC-E-PROJECT-005",
                        format!(
                            "dependency `{}` path `{}` is not a directory",
                            declaration.alias, declaration.path
                        ),
                        declaration.path_span,
                    );
                    continue;
                }
                Err(error) => {
                    self.error(
                        "STC-E-PROJECT-005",
                        format!(
                            "cannot resolve dependency `{}` path `{}`: {error}",
                            declaration.alias, declaration.path
                        ),
                        declaration.path_span,
                    );
                    continue;
                }
            };
            let child_manifest = dependency_dir.join("stc.toml");
            let child_manifest = match child_manifest.canonicalize() {
                Ok(path) if path.is_file() => path,
                Ok(_) => {
                    self.error(
                        "STC-E-PROJECT-005",
                        format!(
                            "dependency `{}` directory `{}` has no stc.toml",
                            declaration.alias, declaration.path
                        ),
                        declaration.path_span,
                    );
                    continue;
                }
                Err(_) => {
                    self.error(
                        "STC-E-PROJECT-005",
                        format!(
                            "dependency `{}` directory `{}` has no stc.toml",
                            declaration.alias, declaration.path
                        ),
                        declaration.path_span,
                    );
                    continue;
                }
            };
            let child_reference = NodeReference::Dependency {
                alias: declaration.alias.clone(),
                path_span: declaration.path_span,
            };
            let Some(child) = self.load_node(&child_manifest, child_reference, depth + 1) else {
                continue;
            };
            let child_name = self.nodes[child].manifest.package.name.clone();
            match &declaration.package {
                Some(expected) if expected != &child_name => {
                    let span = declaration.package_span.unwrap_or(declaration.path_span);
                    self.error(
                        "STC-E-PROJECT-005",
                        format!(
                            "dependency `{}` declares package `{expected}` but `{}` is `{child_name}`",
                            declaration.alias,
                            child_manifest.display()
                        ),
                        span,
                    );
                }
                None if declaration.alias != child_name => {
                    self.error(
                        "STC-E-PROJECT-005",
                        format!(
                            "dependency alias `{}` does not match package name `{child_name}`; add `package = \"{child_name}\"` to use the alias as its root namespace",
                            declaration.alias
                        ),
                        declaration.key_span,
                    );
                }
                _ => {}
            }
            if let Some(existing) = self.nodes[id]
                .edges
                .iter()
                .find(|edge| edge.package == child)
            {
                let span = existing.path_span;
                let diagnostic = Diagnostic::error(
                    "STC-E-PROJECT-006",
                    format!(
                        "dependency `{}` resolves to the same package as `{}`",
                        declaration.alias, existing.alias
                    ),
                    declaration.path_span,
                )
                .with_label(span, "first dependency here");
                self.diagnostics.push(diagnostic);
                continue;
            }
            self.nodes[id].edges.push(Edge {
                alias: declaration.alias,
                span: declaration.key_span,
                path_span: declaration.path_span,
                package: child,
            });
        }
        self.nodes[id].state = NodeState::Loaded;
        Some(id)
    }

    fn check_alias_consistency(&mut self) {
        let mut first: Vec<Option<(String, Span)>> = vec![None; self.nodes.len()];
        let mut diagnostics = Vec::new();
        for node in &self.nodes {
            for edge in &node.edges {
                match &first[edge.package] {
                    None => first[edge.package] = Some((edge.alias.clone(), edge.span)),
                    Some((alias, span)) if alias != &edge.alias => {
                        let name = self.nodes[edge.package].manifest.package.name.clone();
                        let diagnostic = Diagnostic::error(
                            "STC-E-PROJECT-005",
                            format!(
                                "package `{name}` is referenced by conflicting aliases `{alias}` and `{}`",
                                edge.alias
                            ),
                            edge.span,
                        )
                        .with_label(*span, "first alias declared here");
                        diagnostics.push(diagnostic);
                    }
                    Some(_) => {}
                }
            }
        }
        self.diagnostics.extend(diagnostics);
    }

    fn topological_order(&mut self) -> Option<Vec<usize>> {
        let count = self.nodes.len();
        let mut remaining = vec![0_usize; count];
        let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); count];
        for (parent, node) in self.nodes.iter().enumerate() {
            for edge in &node.edges {
                remaining[parent] += 1;
                dependents[edge.package].push(parent);
            }
        }
        let mut ready: BTreeSet<(String, String, usize)> = BTreeSet::new();
        for (id, pending) in remaining.iter().enumerate() {
            if *pending == 0 {
                ready.insert(self.sort_key(id));
            }
        }
        let mut order = Vec::with_capacity(count);
        while let Some((_, _, id)) = ready.pop_first() {
            order.push(id);
            for &dependent in &dependents[id] {
                remaining[dependent] -= 1;
                if remaining[dependent] == 0 {
                    ready.insert(self.sort_key(dependent));
                }
            }
        }
        if order.len() != count {
            self.error(
                "STC-E-PROJECT-006",
                "dependency cycle detected while ordering packages",
                Span::new(FileId::new(0), 0, 0),
            );
            return None;
        }
        Some(order)
    }

    fn sort_key(&self, id: usize) -> (String, String, usize) {
        let node = &self.nodes[id];
        let relative = relative_lock_path(&self.root_dir, &node.canonical_dir).unwrap_or_default();
        (node.manifest.package.name.clone(), relative, id)
    }

    fn build_packages(&self, root_node: usize, order: &[usize]) -> Vec<Package> {
        let mut id_map = vec![PackageId(0); self.nodes.len()];
        for (position, node) in order.iter().enumerate() {
            id_map[*node] = PackageId(position);
        }
        let mut namespaces: Vec<Option<String>> = vec![None; self.nodes.len()];
        for node in &self.nodes {
            for edge in &node.edges {
                if namespaces[edge.package].is_none() {
                    namespaces[edge.package] = Some(edge.alias.clone());
                }
            }
        }
        order
            .iter()
            .enumerate()
            .map(|(position, node_id)| {
                let node = &self.nodes[*node_id];
                let is_root = *node_id == root_node;
                let namespace = if is_root {
                    node.manifest.package.name.clone()
                } else {
                    namespaces[*node_id]
                        .clone()
                        .unwrap_or_else(|| node.manifest.package.name.clone())
                };
                let mut dependencies: Vec<PackageDependency> = node
                    .edges
                    .iter()
                    .map(|edge| PackageDependency {
                        alias: edge.alias.clone(),
                        package: id_map[edge.package],
                    })
                    .collect();
                dependencies.sort_by(|left, right| left.alias.cmp(&right.alias));
                Package {
                    id: PackageId(position),
                    name: node.manifest.package.name.clone(),
                    version: node.manifest.package.version.clone(),
                    manifest_path: node.manifest_path.clone(),
                    root_dir: node.canonical_dir.clone(),
                    namespace,
                    is_root,
                    dependencies,
                    sources: Vec::new(),
                    test_sources: Vec::new(),
                    manifest: node.manifest.clone(),
                }
            })
            .collect()
    }

    fn discover_sources(&mut self, packages: &mut [Package]) {
        for index in 0..packages.len() {
            let root = packages[index].root_dir.clone();
            let exclusions: Vec<PathBuf> = packages
                .iter()
                .enumerate()
                .filter(|(other, package)| {
                    *other != index
                        && package.root_dir != root
                        && package.root_dir.starts_with(&root)
                })
                .map(|(_, package)| package.root_dir.clone())
                .collect();
            let production_globs = packages[index].manifest.project.sources.clone();
            let test_globs = packages[index].manifest.test.sources.clone();
            let fallback = production_globs
                .first()
                .or_else(|| test_globs.first())
                .map_or(Span::new(FileId::new(0), 0, 0), |glob| glob.span);
            let production =
                match discover::discover(&root, &production_globs, &exclusions, fallback) {
                    Ok(sources) => sources,
                    Err(diagnostics) => {
                        self.diagnostics.extend(diagnostics);
                        Vec::new()
                    }
                };
            let tests = match discover::discover(&root, &test_globs, &exclusions, fallback) {
                Ok(sources) => sources,
                Err(diagnostics) => {
                    self.diagnostics.extend(diagnostics);
                    Vec::new()
                }
            };
            self.check_source_overlap(&production, &tests);
            packages[index].sources = source_paths(production);
            packages[index].test_sources = source_paths(tests);
        }
    }

    fn check_source_overlap(
        &mut self,
        production: &[DiscoveredSource],
        tests: &[DiscoveredSource],
    ) {
        let production: HashMap<&Path, Span> = production
            .iter()
            .map(|source| (source.canonical.as_path(), source.span))
            .collect();
        for test in tests {
            if let Some(span) = production.get(test.canonical.as_path()) {
                let diagnostic = Diagnostic::error(
                    "STC-E-TEST-004",
                    format!(
                        "`{}` is both a project source and a test source",
                        test.source.relative
                    ),
                    test.span,
                )
                .with_label(*span, "project source matched here");
                self.diagnostics.push(diagnostic);
            }
        }
    }

    fn compute_lockfile(&mut self, packages: &[Package]) -> Option<Lockfile> {
        let mut lockfile = Lockfile::new();
        for package in packages {
            let source = if package.is_root {
                LockSource::Root
            } else {
                match relative_lock_path(&self.root_dir, &package.root_dir) {
                    Some(path) => LockSource::Path(path),
                    None => {
                        self.error(
                            "STC-E-PROJECT-005",
                            format!(
                                "dependency `{}` is on a different volume and cannot be recorded in stc.lock",
                                package.name
                            ),
                            Span::new(FileId::new(0), 0, 0),
                        );
                        return None;
                    }
                }
            };
            lockfile.packages.push(LockPackage {
                name: package.name.clone(),
                version: package.version.clone(),
                source,
                dependencies: package
                    .dependencies
                    .iter()
                    .map(|dependency| dependency.alias.clone())
                    .collect(),
            });
        }
        lockfile.canonicalize();
        Some(lockfile)
    }

    fn check_lockfile(&mut self, computed: &Lockfile, locked: bool, write_lock: bool) -> bool {
        if !locked && !write_lock {
            // A read-only load is used by commands that do not consume the
            // lockfile (`stc fmt`); it must not create or update one.
            return true;
        }
        let path = self.root_dir.join("stc.lock");
        let display = self.lock_display();
        let rendered = render_lockfile(computed);
        match std::fs::read(&path) {
            Ok(bytes) => {
                let (text, utf8_ok) = match String::from_utf8(bytes) {
                    Ok(text) => (text, true),
                    Err(error) => (
                        String::from_utf8_lossy(error.as_bytes()).into_owned(),
                        false,
                    ),
                };
                let Some(file) = self.add_source(display.clone(), text.clone()) else {
                    return false;
                };
                if !utf8_ok {
                    self.error(
                        "STC-E-LOCK-003",
                        "stc.lock is not valid UTF-8",
                        Span::new(file, 0, 0),
                    );
                    return false;
                }
                match parse_lockfile(&text, file) {
                    Ok(existing) => {
                        if &existing != computed {
                            if locked {
                                self.error(
                                    "STC-E-LOCK-002",
                                    "stc.lock is out of date; run without --locked to update it",
                                    Span::new(file, 0, 0),
                                );
                                return false;
                            }
                            if let Err(error) = std::fs::write(&path, &rendered) {
                                self.error(
                                    "STC-E-LOCK-003",
                                    format!("cannot write stc.lock: {error}"),
                                    Span::new(file, 0, 0),
                                );
                                return false;
                            }
                        } else if !locked
                            && text != rendered
                            && let Err(error) = std::fs::write(&path, &rendered)
                        {
                            self.error(
                                "STC-E-LOCK-003",
                                format!("cannot write stc.lock: {error}"),
                                Span::new(file, 0, 0),
                            );
                            return false;
                        }
                        true
                    }
                    Err(diagnostics) => {
                        self.diagnostics.extend(diagnostics);
                        false
                    }
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let Some(file) = self.add_source(display, String::new()) else {
                    return false;
                };
                if locked {
                    self.error(
                        "STC-E-LOCK-001",
                        "stc.lock is missing; run without --locked to create it",
                        Span::new(file, 0, 0),
                    );
                    return false;
                }
                if let Err(error) = std::fs::write(&path, &rendered) {
                    self.error(
                        "STC-E-LOCK-003",
                        format!("cannot write stc.lock: {error}"),
                        Span::new(file, 0, 0),
                    );
                    return false;
                }
                true
            }
            Err(error) => {
                let Some(file) = self.add_source(display, String::new()) else {
                    return false;
                };
                self.error(
                    "STC-E-LOCK-003",
                    format!("cannot read stc.lock: {error}"),
                    Span::new(file, 0, 0),
                );
                false
            }
        }
    }

    fn lock_display(&self) -> String {
        match Path::new(&self.root_display).parent() {
            Some(parent) if !parent.as_os_str().is_empty() => {
                parent.join("stc.lock").to_string_lossy().into_owned()
            }
            _ => "stc.lock".to_owned(),
        }
    }
}

fn source_paths(discovered: Vec<DiscoveredSource>) -> Vec<SourcePath> {
    discovered.into_iter().map(|source| source.source).collect()
}
