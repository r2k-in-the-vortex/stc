//! Session configuration and the public analysis entry points.

use std::collections::BTreeMap;

use stc_diag::DiagnosticSink;
use stc_hir::{DefId, FieldId, HirCrate, LocalId, ParamId, StdFnId, TyId};
use stc_parser::ParsedFile;
use stc_span::FileId;
use stc_syntax::NodeId;

use crate::Analyzer;

/// Session-level names used by the M3 multi-file bridge (D-060).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// In-memory package name.
    pub package_name: String,
    /// Root namespace for files without an explicit `NAMESPACE`.
    pub root_namespace: String,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            package_name: "_root".into(),
            root_namespace: "_root".into(),
        }
    }
}

impl Session {
    /// Creates a session with the given package and root namespace names.
    #[must_use]
    pub fn new(package_name: impl Into<String>, root_namespace: impl Into<String>) -> Self {
        Self {
            package_name: package_name.into(),
            root_namespace: root_namespace.into(),
        }
    }
}

/// One package in a merged multi-package analysis (D-153).
#[derive(Clone, Debug)]
pub struct PackageSession<'a> {
    /// Package identity from `[package].name`; used for `INTERNAL` access
    /// checks and recorded on every definition.
    pub name: String,
    /// Root namespace: the package name for the root, the dependency alias
    /// otherwise. This is the name dependents use to qualify the package.
    pub root_namespace: String,
    /// Indices of the direct dependencies whose root namespaces are visible
    /// from this package. Transitive dependencies are not visible directly.
    pub dependencies: Vec<usize>,
    /// Parsed production source files belonging to this package.
    pub files: &'a [ParsedFile],
    /// Whether this is the root package. Exactly one package should set it.
    pub is_root: bool,
}

impl<'a> PackageSession<'a> {
    /// Creates a package session with no dependencies.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        root_namespace: impl Into<String>,
        files: &'a [ParsedFile],
        is_root: bool,
    ) -> Self {
        Self {
            name: name.into(),
            root_namespace: root_namespace.into(),
            dependencies: Vec::new(),
            files,
            is_root,
        }
    }
}

/// A merged multi-package analysis session.
#[derive(Clone, Debug)]
pub struct ProjectSession<'a> {
    /// Packages in dependency order (dependencies before dependents).
    pub packages: Vec<PackageSession<'a>>,
    /// `[project].configuration` from the root package.
    pub configuration: Option<String>,
    /// `[project].stdlib` from the root package. When false, standard
    /// function blocks are not synthesized; standard functions remain.
    pub stdlib: bool,
}

impl Default for ProjectSession<'_> {
    fn default() -> Self {
        Self {
            packages: Vec::new(),
            configuration: None,
            stdlib: true,
        }
    }
}

/// One recorded name resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Res {
    /// A definition.
    Def(DefId),
    /// A local variable.
    Local(LocalId),
    /// A parameter.
    Param(ParamId),
    /// A field.
    Field(FieldId),
    /// A namespace.
    Namespace(DefId),
    /// A standard function signature.
    StdFunction(StdFnId),
    /// A nominal type.
    Type(TyId),
    /// An enumeration value.
    EnumValue(DefId, i128),
    /// An unresolved name; a diagnostic has been reported.
    Error,
}

/// The result of analyzing a set of parsed files.
#[derive(Debug)]
pub struct Analysis {
    /// Resolved HIR.
    pub hir: HirCrate,
    /// Resolutions keyed by source node.
    pub resolutions: BTreeMap<(FileId, NodeId), Res>,
}

/// Analyzes parsed files with the default M3 session.
#[must_use]
pub fn analyze(files: &[ParsedFile], diagnostics: &mut DiagnosticSink) -> Analysis {
    analyze_with(&Session::default(), files, diagnostics)
}

/// Analyzes parsed files with explicit session names.
#[must_use]
pub fn analyze_with(
    session: &Session,
    files: &[ParsedFile],
    diagnostics: &mut DiagnosticSink,
) -> Analysis {
    let project = ProjectSession {
        packages: vec![PackageSession {
            name: session.package_name.clone(),
            root_namespace: session.root_namespace.clone(),
            dependencies: Vec::new(),
            files,
            is_root: true,
        }],
        configuration: None,
        stdlib: true,
    };
    analyze_project(&project, diagnostics)
}

/// Analyzes an ordered multi-package project into one merged HIR crate.
///
/// Packages are collected in the order given (dependencies first) and share
/// one `DefId` arena. Definitions record their owning package; dependency root
/// namespaces are visible to dependents, and `INTERNAL` definitions are
/// rejected across package boundaries.
#[must_use]
pub fn analyze_project<'a>(
    session: &'a ProjectSession<'a>,
    diagnostics: &'a mut DiagnosticSink,
) -> Analysis {
    let mut analyzer = Analyzer::new_project(session, diagnostics);
    analyzer.run();
    Analysis {
        hir: analyzer.hir,
        resolutions: analyzer.resolutions,
    }
}
