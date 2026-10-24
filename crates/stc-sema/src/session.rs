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
    let mut analyzer = Analyzer::new(session, files, diagnostics);
    analyzer.run();
    Analysis {
        hir: analyzer.hir,
        resolutions: analyzer.resolutions,
    }
}
