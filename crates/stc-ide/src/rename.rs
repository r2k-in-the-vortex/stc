//! Shared rename engine over the existing project analysis (D-253, WL-18).
//!
//! The engine is pure: it borrows the HIR, resolutions, parsed sources,
//! manifest, selected profile, and a reanalysis callback, and it returns the
//! complete edit set plus the canonical history entries, the online-change
//! classification, and the warnings. It never writes a file and never sends a
//! protocol message.
//!
//! The language server resolves a cursor through [`RenameContext::target_at`];
//! the CLI resolves a dotted key through [`RenameIndex::resolve`]. Both call
//! the same [`RenameSession`], so the two entry points collect the same source,
//! manifest, profile, and access-path edits.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use stc_diag::{Diagnostic, DiagnosticSink};
use stc_hir::{
    CalleeHir, DefId, DefKind, ExprId, ExprKindHir, FieldId, HirCrate, LocalId, ParamId, PlaceHir,
};
use stc_lexer::{is_identifier, tokenize_bytes};
use stc_manifest::{Manifest, Profile, ProfileEndpoint};
use stc_parser::{ParseOptions, ParsedFile, parse};
use stc_sema::{Analysis, Res};
use stc_span::{FileId, SourceMap, Span};
use stc_syntax::ast::{AstNode, Name};
use stc_syntax::{NodeId, SyntaxKind, SyntaxNode};

use crate::query::{
    DeclarationIndex, Target, collect_tokens, cursor_ident, name_token, target_of_res,
};

/// The source map that owns a [`RenameEdit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RenameFile {
    /// A project source file.
    Source(FileId),
    /// The root `stc.toml`.
    Manifest,
    /// The selected machine profile.
    Profile,
}

/// One byte-range replacement.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RenameEdit {
    /// Owning source map.
    pub file: RenameFile,
    /// Inclusive byte offset.
    pub start: u32,
    /// Exclusive byte offset.
    pub end: u32,
    /// Replacement text.
    pub new_text: String,
}

/// One canonical root-package history entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameHistoryEntry {
    /// Old dotted key.
    pub old: String,
    /// New dotted key.
    pub new: String,
}

/// The identifier a rename starts from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenamePlaceholder {
    /// Byte span in its owning source map.
    pub span: Span,
    /// Current declaration spelling.
    pub name: String,
}

/// The public target identity of a rename.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RenameTarget {
    /// A declaration.
    Def(DefId),
    /// A struct/FB/class/program field.
    Field(FieldId),
    /// A parameter.
    Param(ParamId),
    /// A local variable.
    Local(LocalId),
    /// An enumeration or named value.
    EnumValue(DefId, i128),
}

/// Whether a rename has a canonical history entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameTargetKind {
    /// The rename records a canonical root-package entry.
    Tracked,
    /// The rename has no GOT identity and writes no history.
    HistoryFree,
}

/// Why a rename cannot preserve online-change identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColdReason {
    /// The selected configuration's name appears in task-guard identities.
    ConfigurationTaskGuard,
}

/// The online-change classification of a rename.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameOnline {
    /// The tracked history preserves the symbol identity.
    Preserved,
    /// The rename may force the next online download to be cold.
    Cold(ColdReason),
}

/// A non-fatal rename warning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameWarning {
    /// The selected machine profile could not be loaded or parsed.
    ProfileUnavailable(String),
}

/// The complete result of one rename.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameResult {
    /// Non-overlapping edits in every owning source map.
    pub edits: Vec<RenameEdit>,
    /// Canonical root-package history entries; empty when history-free.
    pub history: Vec<RenameHistoryEntry>,
    /// Online-change classification.
    pub online: RenameOnline,
    /// Whether the target is tracked or history-free.
    pub target: RenameTargetKind,
    /// Warnings the caller must show and the client must not treat as failure.
    pub warnings: Vec<RenameWarning>,
}

/// A rename rejection with a stable message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameError {
    /// The cursor is not on an identifier.
    NotAnIdentifier,
    /// The identifier does not resolve to a declaration.
    Unresolved,
    /// The resolved symbol cannot be renamed.
    Unsupported,
    /// The declaration belongs to a dependency package.
    DependencyDeclaration,
    /// The new spelling is not one identifier token or is a keyword.
    InvalidName(String),
    /// The old and new spellings are identical.
    SameName,
    /// The new dotted path already names a different declaration.
    Collision(String),
    /// The rename produced no edits.
    NoOccurrences,
    /// Two edits overlap in one file.
    OverlappingEdits,
    /// A tracked target has no canonical root-package path.
    UntrackedKey,
    /// A matched reference has no editable span.
    MatchedReferenceWithoutSpan,
    /// The post-rename reanalysis produced a new error.
    Analysis(String),
}

impl fmt::Display for RenameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnIdentifier => formatter.write_str("the position is not on an identifier"),
            Self::Unresolved => {
                formatter.write_str("the identifier does not resolve to a declaration")
            }
            Self::Unsupported => formatter.write_str("the symbol cannot be renamed"),
            Self::DependencyDeclaration => formatter.write_str(
                "the declaration belongs to a dependency and cannot be renamed in the current package",
            ),
            Self::InvalidName(name) => write!(
                formatter,
                "`{name}` is not a valid identifier or is a reserved keyword"
            ),
            Self::SameName => formatter.write_str("the old and new names are the same"),
            Self::Collision(path) => write!(
                formatter,
                "`{path}` already names a declaration in the current package"
            ),
            Self::NoOccurrences => formatter.write_str("the rename has no occurrences to rewrite"),
            Self::OverlappingEdits => formatter.write_str("rename produced overlapping edits"),
            Self::UntrackedKey => {
                formatter.write_str("the symbol has no canonical root-package path")
            }
            Self::MatchedReferenceWithoutSpan => {
                formatter.write_str("a matched reference has no editable source span")
            }
            Self::Analysis(message) => write!(formatter, "the renamed project does not analyze: {message}"),
        }
    }
}

impl std::error::Error for RenameError {}

/// The root-package dotted-key index used by the CLI and the collision check.
pub struct RenameIndex {
    paths: Vec<(Vec<String>, RenameTarget)>,
    by_key: BTreeMap<String, Vec<RenameTarget>>,
    package_name: String,
    namespace: String,
}

impl RenameIndex {
    /// Builds the index from a merged project HIR and the root package id.
    ///
    /// Only declarations owned by `root_package` are included. Paths are
    /// namespace-relative and case-insensitive; the package/namespace prefix
    /// is accepted by [`RenameIndex::resolve`].
    #[must_use]
    pub fn build(hir: &HirCrate, root_package: usize, namespace: &str) -> Self {
        let mut index = Self {
            paths: Vec::new(),
            by_key: BTreeMap::new(),
            package_name: hir
                .packages
                .get(root_package)
                .map_or_else(|| namespace.to_owned(), |package| package.name.clone()),
            namespace: namespace.to_owned(),
        };
        for (def_index, def) in hir.defs.iter().enumerate() {
            if def.package.index() != root_package {
                continue;
            }
            let Ok(def_id) = u32::try_from(def_index).map(DefId::new) else {
                continue;
            };
            let Some(path) = hir.def_path(def_id) else {
                continue;
            };
            index.add(path.clone(), RenameTarget::Def(def_id));
            for field in def.field_ids() {
                let data = hir.field(*field);
                let mut field_path = path.clone();
                field_path.push(hir.interner.text(data.name).to_owned());
                index.add(field_path, RenameTarget::Field(*field));
            }
            for method in def.method_ids() {
                let data = &hir.defs[method.index()];
                let Some(name) = data.name else {
                    continue;
                };
                let mut method_path = path.clone();
                method_path.push(hir.interner.text(name).to_owned());
                index.add(method_path, RenameTarget::Def(*method));
            }
            if let DefKind::Enum { values, .. } | DefKind::NamedValue { values, .. } = &def.kind {
                for value in values {
                    let mut value_path = path.clone();
                    value_path.push(hir.interner.text(value.name).to_owned());
                    index.add(value_path, RenameTarget::EnumValue(def_id, value.value));
                }
            }
        }
        for (param_index, param) in hir.params.iter().enumerate() {
            if param.owner.index() >= hir.defs.len()
                || hir.defs[param.owner.index()].package.index() != root_package
            {
                continue;
            }
            let Ok(param_id) = u32::try_from(param_index).map(ParamId::new) else {
                continue;
            };
            if let Some(mut path) = hir.def_path(param.owner) {
                path.push(hir.interner.text(param.name).to_owned());
                index.add(path, RenameTarget::Param(param_id));
            }
        }
        for (local_index, local) in hir.locals.iter().enumerate() {
            if local.owner.index() >= hir.defs.len()
                || hir.defs[local.owner.index()].package.index() != root_package
            {
                continue;
            }
            let Ok(local_id) = u32::try_from(local_index).map(LocalId::new) else {
                continue;
            };
            if let Some(mut path) = hir.def_path(local.owner) {
                path.push(hir.interner.text(local.name).to_owned());
                index.add(path, RenameTarget::Local(local_id));
            }
        }
        index
    }

    /// Resolves a dotted key, accepting an optional package/namespace prefix.
    #[must_use]
    pub fn resolve(&self, segments: &[String]) -> Option<RenameTarget> {
        let mut candidates = Vec::new();
        if let Some(target) = self.lookup(segments) {
            candidates.push(target);
        }
        if let Some(first) = segments.first()
            && (first.eq_ignore_ascii_case(&self.package_name)
                || first.eq_ignore_ascii_case(&self.namespace))
            && segments.len() > 1
            && let Some(target) = self.lookup(&segments[1..])
        {
            candidates.push(target);
        }
        candidates.sort();
        candidates.dedup();
        match candidates.as_slice() {
            [] => None,
            [target] => Some(*target),
            _ => None,
        }
    }

    /// Returns a target's canonical namespace-relative path.
    #[must_use]
    pub fn path_of(&self, target: RenameTarget) -> Option<Vec<String>> {
        self.paths
            .iter()
            .find(|(_, candidate)| *candidate == target)
            .map(|(path, _)| path.clone())
    }

    /// Returns whether a definition kind can be used as a path prefix.
    #[must_use]
    pub fn is_path_prefix_kind(&self, kind: &DefKind) -> bool {
        matches!(
            kind,
            DefKind::Namespace { .. }
                | DefKind::Function { .. }
                | DefKind::FunctionBlock { .. }
                | DefKind::Program { .. }
                | DefKind::Class { .. }
                | DefKind::Interface { .. }
                | DefKind::Method { .. }
                | DefKind::Struct { .. }
                | DefKind::Enum { .. }
                | DefKind::NamedValue { .. }
                | DefKind::Subrange { .. }
                | DefKind::Alias { .. }
        )
    }

    fn add(&mut self, path: Vec<String>, target: RenameTarget) {
        let key = normalize_path(&path);
        let targets = self.by_key.entry(key).or_default();
        if !targets.contains(&target) {
            targets.push(target);
        }
        let duplicate = self.paths.iter().any(|(existing, candidate)| {
            *candidate == target && normalize_path(existing) == normalize_path(&path)
        });
        if !duplicate {
            self.paths.push((path, target));
        }
    }

    fn lookup(&self, segments: &[String]) -> Option<RenameTarget> {
        let key = normalize_path(segments);
        let targets = self.by_key.get(&key)?;
        (targets.len() == 1).then_some(targets[0])
    }
}

fn normalize_path(path: &[String]) -> String {
    path.iter()
        .map(|segment| segment.to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(".")
}

/// The selected machine profile available to a rename.
pub struct RenameProfile<'a> {
    /// Absolute profile path, empty when the loader could not resolve it.
    pub path: PathBuf,
    /// Profile text.
    pub text: &'a str,
    /// Source map that owns the profile text and spans.
    pub sources: &'a SourceMap,
    /// Profile file id in [`RenameProfile::sources`].
    pub file: FileId,
    /// Parsed profile, when it loaded and parsed.
    pub profile: Option<&'a Profile>,
    /// Load or parse error when [`RenameProfile::profile`] is `None`.
    pub error: Option<String>,
}

/// The reanalysis callback: parsed post-rename sources and manifest model.
pub type RenameAnalyzer<'a> = dyn Fn(&[ParsedFile], &Manifest) -> (Analysis, Vec<Diagnostic>) + 'a;

/// Borrowed inputs for one rename operation.
pub struct RenameContext<'a> {
    /// Merged project HIR.
    pub hir: &'a HirCrate,
    /// Name resolutions keyed by source node.
    pub resolutions: &'a BTreeMap<(FileId, NodeId), Res>,
    /// Parsed project sources.
    pub parsed: &'a [ParsedFile],
    /// Project source map.
    pub sources: &'a SourceMap,
    /// Cursor declaration-name index.
    pub declarations: &'a DeclarationIndex,
    /// Whether the project parses with relaxed semicolons.
    pub relaxed: bool,
    /// Root package index in [`HirCrate::packages`].
    pub root_package: usize,
    /// Root package namespace.
    pub root_namespace: &'a str,
    /// Root manifest.
    pub manifest: &'a Manifest,
    /// Source map that owns the root manifest text.
    pub manifest_sources: &'a SourceMap,
    /// Root manifest file id in [`RenameContext::manifest_sources`].
    pub manifest_file: FileId,
    /// Selected machine profile, when one is configured.
    pub profile: Option<RenameProfile<'a>>,
    /// Semantic diagnostics of the original analysis.
    pub baseline_diagnostics: &'a [Diagnostic],
    /// Reanalysis callback for the post-edit source and manifest.
    pub analyzer: Box<RenameAnalyzer<'a>>,
}

impl<'a> RenameContext<'a> {
    /// Builds the root-package dotted-key index.
    #[must_use]
    pub fn index(&self) -> RenameIndex {
        RenameIndex::build(self.hir, self.root_package, self.root_namespace)
    }

    /// Resolves a cursor to a public rename target.
    ///
    /// The resolution walk matches definition/references, then the cursor
    /// declaration index, then the `VAR_ACCESS`/`VAR_CONFIG` path segments.
    ///
    /// # Errors
    ///
    /// Returns [`RenameError::NotAnIdentifier`] when the cursor is not on an
    /// identifier, [`RenameError::Unresolved`] when no declaration resolves,
    /// and [`RenameError::Unsupported`] for standard functions and built-in
    /// types.
    pub fn target_at(&self, file: FileId, offset: usize) -> Result<RenameTarget, RenameError> {
        let Some(parsed) = self.parsed.get(file.index()) else {
            return Err(RenameError::Unresolved);
        };
        let token = cursor_ident(parsed, offset).ok_or(RenameError::NotAnIdentifier)?;
        let mut node = token.parent();
        while let Some(current) = node {
            if let Some(resolution) = self.resolutions.get(&(file, current.id()))
                && let Some(target) = target_of_res(self.hir, resolution)
                && target_name(self.hir, target)
                    .is_some_and(|name| token.text().eq_ignore_ascii_case(&name))
            {
                return public_target(target);
            }
            node = current.parent();
        }
        if let Some(target) = self.declarations.target_at_token(file, token.span()) {
            return public_target(target);
        }
        if let Some(target) = self.access_target_at(file, offset) {
            return Ok(target);
        }
        if std_function_at(self.hir, file, offset, token.text()) {
            return Err(RenameError::Unsupported);
        }
        Err(RenameError::Unresolved)
    }

    /// Resolves a dotted key through the shared root-package index.
    #[must_use]
    pub fn resolve_path(&self, segments: &[String]) -> Option<RenameTarget> {
        self.index().resolve(segments)
    }

    /// Starts a session for a resolved target.
    #[must_use]
    pub fn session(&self, target: RenameTarget) -> RenameSession<'_, 'a> {
        RenameSession {
            context: self,
            target,
        }
    }

    fn access_target_at(&self, file: FileId, offset: usize) -> Option<RenameTarget> {
        for def in &self.hir.defs {
            if def.package.index() != self.root_package {
                continue;
            }
            match &def.kind {
                DefKind::Program { var_access, .. } => {
                    for access in var_access {
                        let Some(index) = access
                            .path_spans
                            .iter()
                            .position(|span| cursor_in_span(*span, file, offset))
                        else {
                            continue;
                        };
                        let fields = program_access_fields(self.hir, access.place);
                        if let Some(field) = fields.get(index) {
                            return Some(RenameTarget::Field(*field));
                        }
                    }
                }
                DefKind::Configuration { config } => {
                    for access in &config.var_access {
                        if let Some(target) = self.config_access_target(access, file, offset) {
                            return Some(target);
                        }
                    }
                    for resource in &config.resources {
                        for access in &resource.var_access {
                            if let Some(target) = self.config_access_target(access, file, offset) {
                                return Some(target);
                            }
                        }
                    }
                    for init in &config.config_inits {
                        let Some(index) = init
                            .field_spans
                            .iter()
                            .position(|span| cursor_in_span(*span, file, offset))
                        else {
                            continue;
                        };
                        if let Some(field) = init.fields.get(index) {
                            return Some(RenameTarget::Field(*field));
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn config_access_target(
        &self,
        access: &stc_hir::VarAccessHir,
        file: FileId,
        offset: usize,
    ) -> Option<RenameTarget> {
        let index = access
            .path_spans
            .iter()
            .position(|span| cursor_in_span(*span, file, offset))?;
        match &self.hir.expr(access.place).kind {
            ExprKindHir::Place(PlaceHir::Global(def)) => Some(RenameTarget::Def(*def)),
            ExprKindHir::Place(PlaceHir::InstanceField { fields, .. }) => {
                fields.get(index).map(|field| RenameTarget::Field(*field))
            }
            _ => None,
        }
    }
}

/// One rename over a resolved target.
pub struct RenameSession<'s, 'a> {
    context: &'s RenameContext<'a>,
    target: RenameTarget,
}

impl RenameSession<'_, '_> {
    /// Validates the target and returns the declaration placeholder.
    ///
    /// # Errors
    ///
    /// Returns a stable [`RenameError`] for a dependency declaration, a root
    /// namespace, or a target without a declaration name.
    pub fn prepare(&self) -> Result<RenamePlaceholder, RenameError> {
        self.validate_target()?;
        let (span, name) = self.declaration()?;
        Ok(RenamePlaceholder { span, name })
    }

    /// Computes the complete edit set, history entries, and online status.
    ///
    /// # Errors
    ///
    /// Returns a stable [`RenameError`] for an invalid name, a collision, an
    /// unsupported or dependency target, a missing span, overlapping edits, or
    /// a new error introduced by the post-rename reanalysis.
    pub fn rename(&self, new_name: &str) -> Result<RenameResult, RenameError> {
        if !is_identifier(new_name) {
            return Err(RenameError::InvalidName(new_name.to_owned()));
        }
        self.validate_target()?;
        let (_, old_name) = self.declaration()?;
        if old_name == new_name {
            return Err(RenameError::SameName);
        }
        let index = self.context.index();
        let old_path = self
            .canonical_path(self.target)
            .ok_or(RenameError::UntrackedKey)?;
        let mut new_path = old_path.clone();
        if let Some(last) = new_path.last_mut() {
            *last = new_name.to_owned();
        }
        if let Some(existing) = index.resolve(&new_path)
            && existing != self.target
        {
            return Err(RenameError::Collision(new_path.join(".")));
        }
        let case_only = old_name.eq_ignore_ascii_case(new_name);

        let mut edits = Vec::new();
        edits.push(self.declaration_edit(new_name)?);
        self.collect_reference_edits(&old_name, new_name, &mut edits)?;
        if let RenameTarget::Def(def) = self.target {
            let kind = &self.context.hir.defs[def.index()].kind;
            if index.is_path_prefix_kind(kind) {
                self.collect_prefix_edits(&old_path, new_name, &mut edits);
            }
        }
        self.collect_access_edits(new_name, &mut edits);
        self.collect_manifest_edits(&old_name, new_name, &mut edits)?;
        let mut warnings = Vec::new();
        self.collect_profile_edits(new_name, &mut edits, &mut warnings)?;

        if edits.is_empty() {
            return Err(RenameError::NoOccurrences);
        }
        edits.sort();
        edits.dedup();
        if edits_overlap(&edits) {
            return Err(RenameError::OverlappingEdits);
        }

        self.reanalyze(&edits, &old_name, new_name)?;
        let (history, target, online) = self.classify(new_name, case_only)?;
        Ok(RenameResult {
            edits,
            history,
            online,
            target,
            warnings,
        })
    }

    fn validate_target(&self) -> Result<(), RenameError> {
        let hir = self.context.hir;
        let package = match self.target {
            RenameTarget::Def(def) => {
                let data = &hir.defs[def.index()];
                if is_root_namespace(data) {
                    return Err(RenameError::UntrackedKey);
                }
                data.package.index()
            }
            RenameTarget::Field(field) => hir.defs[hir.field(field).owner.index()].package.index(),
            RenameTarget::Param(param) => hir.defs[hir.param(param).owner.index()].package.index(),
            RenameTarget::Local(local) => hir.defs[hir.local(local).owner.index()].package.index(),
            RenameTarget::EnumValue(def, _) => hir.defs[def.index()].package.index(),
        };
        if package != self.context.root_package {
            return Err(RenameError::DependencyDeclaration);
        }
        Ok(())
    }

    fn canonical_path(&self, target: RenameTarget) -> Option<Vec<String>> {
        let hir = self.context.hir;
        match target {
            RenameTarget::Def(def) => self.definition_path(def),
            RenameTarget::Field(field) => {
                let data = hir.field(field);
                let mut path = self.definition_path(data.owner)?;
                path.push(self.spelling(data.span, data.name));
                Some(path)
            }
            RenameTarget::EnumValue(def, value) => {
                let data = &hir.defs[def.index()];
                let (DefKind::Enum { values, .. } | DefKind::NamedValue { values, .. }) =
                    &data.kind
                else {
                    return None;
                };
                let entry = values.iter().find(|candidate| candidate.value == value)?;
                let mut path = self.definition_path(def)?;
                path.push(self.spelling(entry.span, entry.name));
                Some(path)
            }
            RenameTarget::Param(param) => {
                let data = hir.param(param);
                let mut path = self.definition_path(data.owner)?;
                path.push(self.spelling(data.span, data.name));
                Some(path)
            }
            RenameTarget::Local(local) => {
                let data = hir.local(local);
                let mut path = self.definition_path(data.owner)?;
                path.push(self.spelling(data.span, data.name));
                Some(path)
            }
        }
    }

    fn definition_path(&self, def: DefId) -> Option<Vec<String>> {
        let hir = self.context.hir;
        let mut path = Vec::new();
        let mut current = Some(def);
        while let Some(id) = current {
            let data = &hir.defs[id.index()];
            let is_root_namespace =
                data.parent.is_none() && matches!(data.kind, DefKind::Namespace { .. });
            if is_root_namespace {
                break;
            }
            if let Some(name) = data.name {
                path.push(self.spelling(data.span, name));
            }
            current = data.parent;
        }
        path.reverse();
        (!path.is_empty()).then_some(path)
    }

    fn spelling(&self, span: Span, symbol: stc_hir::Symbol) -> String {
        let canonical = self.context.hir.interner.text(symbol);
        self.context
            .parsed
            .get(span.file.index())
            .and_then(|parsed| {
                name_token(parsed, span, canonical)
                    .map(|(start, end)| parsed.source()[start as usize..end as usize].to_owned())
            })
            .unwrap_or_else(|| canonical.to_owned())
    }

    fn declaration(&self) -> Result<(Span, String), RenameError> {
        let hir = self.context.hir;
        match self.target {
            RenameTarget::Def(def) => {
                let data = &hir.defs[def.index()];
                let name = data.name.ok_or(RenameError::Unresolved)?;
                Ok((data.span, hir.interner.text(name).to_owned()))
            }
            RenameTarget::Field(field) => {
                let data = hir.field(field);
                Ok((data.span, hir.interner.text(data.name).to_owned()))
            }
            RenameTarget::Param(param) => {
                let data = hir.param(param);
                Ok((data.span, hir.interner.text(data.name).to_owned()))
            }
            RenameTarget::Local(local) => {
                let data = hir.local(local);
                Ok((data.span, hir.interner.text(data.name).to_owned()))
            }
            RenameTarget::EnumValue(def, value) => {
                let data = &hir.defs[def.index()];
                let (DefKind::Enum { values, .. } | DefKind::NamedValue { values, .. }) =
                    &data.kind
                else {
                    return Err(RenameError::Unresolved);
                };
                let entry = values
                    .iter()
                    .find(|candidate| candidate.value == value)
                    .ok_or(RenameError::Unresolved)?;
                Ok((entry.span, hir.interner.text(entry.name).to_owned()))
            }
        }
    }

    fn declaration_edit(&self, new_name: &str) -> Result<RenameEdit, RenameError> {
        let (span, name) = self.declaration()?;
        let parsed = self
            .context
            .parsed
            .get(span.file.index())
            .ok_or(RenameError::MatchedReferenceWithoutSpan)?;
        let (start, end) =
            name_token(parsed, span, &name).ok_or(RenameError::MatchedReferenceWithoutSpan)?;
        Ok(RenameEdit {
            file: RenameFile::Source(span.file),
            start,
            end,
            new_text: new_name.to_owned(),
        })
    }

    fn collect_reference_edits(
        &self,
        old_name: &str,
        new_name: &str,
        edits: &mut Vec<RenameEdit>,
    ) -> Result<(), RenameError> {
        for ((file, node_id), resolution) in self.context.resolutions {
            if !res_matches_target(self.context.hir, resolution, self.target) {
                continue;
            }
            let parsed = self
                .context
                .parsed
                .get(file.index())
                .ok_or(RenameError::MatchedReferenceWithoutSpan)?;
            let node = parsed.tree().node(*node_id);
            let mut tokens = Vec::new();
            collect_tokens(&node, &mut tokens);
            let token = tokens
                .iter()
                .rev()
                .find(|token| {
                    token.kind() == SyntaxKind::Ident && token.text().eq_ignore_ascii_case(old_name)
                })
                .ok_or(RenameError::MatchedReferenceWithoutSpan)?;
            edits.push(RenameEdit {
                file: RenameFile::Source(*file),
                start: token.span().start,
                end: token.span().end,
                new_text: new_name.to_owned(),
            });
        }
        Ok(())
    }

    fn collect_prefix_edits(
        &self,
        target_path: &[String],
        new_name: &str,
        edits: &mut Vec<RenameEdit>,
    ) {
        for (index, parsed) in self.context.parsed.iter().enumerate() {
            let file = FileId::new(u32::try_from(index).expect("source index fits in u32"));
            let mut nodes = Vec::new();
            collect_nodes(&parsed.tree().root(), &mut nodes);
            for node in nodes {
                if node.kind() != SyntaxKind::PathExpr && node.kind() != SyntaxKind::Path {
                    continue;
                }
                let Some(resolved) = self.path_resolution(file, &node) else {
                    continue;
                };
                let Some(resolved_path) = self.canonical_path(resolved) else {
                    continue;
                };
                if !path_starts_with(&resolved_path, target_path) {
                    continue;
                }
                let names: Vec<Name> = node.child_nodes().iter().filter_map(Name::cast).collect();
                if names.len() < target_path.len() {
                    continue;
                }
                for start in 0..=names.len() - target_path.len() {
                    let window = &names[start..start + target_path.len()];
                    if !window
                        .iter()
                        .zip(target_path)
                        .all(|(name, expected)| name.text().eq_ignore_ascii_case(expected))
                    {
                        continue;
                    }
                    let name = &window[target_path.len() - 1];
                    let Some(token) = name.token() else {
                        continue;
                    };
                    edits.push(RenameEdit {
                        file: RenameFile::Source(file),
                        start: token.span().start,
                        end: token.span().end,
                        new_text: new_name.to_owned(),
                    });
                }
            }
        }
    }

    fn collect_access_edits(&self, new_name: &str, edits: &mut Vec<RenameEdit>) {
        let hir = self.context.hir;
        for def in &hir.defs {
            if def.package.index() != self.context.root_package {
                continue;
            }
            match &def.kind {
                DefKind::Program { var_access, .. } => {
                    for access in var_access {
                        let fields = program_access_fields(hir, access.place);
                        for (field, span) in fields.iter().zip(&access.path_spans) {
                            if self.target == RenameTarget::Field(*field) {
                                push_span_edit(edits, *span, new_name);
                            }
                        }
                    }
                }
                DefKind::Configuration { config } => {
                    for access in &config.var_access {
                        self.push_config_access_edits(access, new_name, edits);
                    }
                    for resource in &config.resources {
                        for access in &resource.var_access {
                            self.push_config_access_edits(access, new_name, edits);
                        }
                    }
                    for init in &config.config_inits {
                        for (field, span) in init.fields.iter().zip(&init.field_spans) {
                            if self.target == RenameTarget::Field(*field) {
                                push_span_edit(edits, *span, new_name);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn push_config_access_edits(
        &self,
        access: &stc_hir::VarAccessHir,
        new_name: &str,
        edits: &mut Vec<RenameEdit>,
    ) {
        match &self.context.hir.expr(access.place).kind {
            ExprKindHir::Place(PlaceHir::Global(def)) => {
                if self.target == RenameTarget::Def(*def)
                    && let Some(span) = access.path_spans.first()
                {
                    push_span_edit(edits, *span, new_name);
                }
            }
            ExprKindHir::Place(PlaceHir::InstanceField { fields, .. }) => {
                for (field, span) in fields.iter().zip(&access.path_spans) {
                    if self.target == RenameTarget::Field(*field) {
                        push_span_edit(edits, *span, new_name);
                    }
                }
            }
            _ => {}
        }
    }

    fn collect_manifest_edits(
        &self,
        old_name: &str,
        new_name: &str,
        edits: &mut Vec<RenameEdit>,
    ) -> Result<(), RenameError> {
        let hir = self.context.hir;
        if let RenameTarget::Def(def) = self.target {
            let data = &hir.defs[def.index()];
            if self.is_selected_configuration() {
                let span = self
                    .context
                    .manifest
                    .project
                    .configuration_span
                    .ok_or(RenameError::MatchedReferenceWithoutSpan)?;
                push_whole_edit(edits, RenameFile::Manifest, span, new_name);
            }
            if data.test_only && data.package.index() == self.context.root_package {
                for case in &self.context.manifest.test.cases {
                    if case.name.eq_ignore_ascii_case(old_name) {
                        push_whole_edit(edits, RenameFile::Manifest, case.name_span, new_name);
                    }
                }
            }
        }
        let Some(candidates) = self.binding_candidates() else {
            return Ok(());
        };
        for binding in &self.context.manifest.io.bindings {
            if !candidates
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(&binding.variable))
            {
                continue;
            }
            let text = self.manifest_text(binding.variable_span.file)?;
            edits.push(segment_edit(
                RenameFile::Manifest,
                text,
                binding.variable_span,
                &binding.variable,
                new_name,
            )?);
        }
        Ok(())
    }

    fn collect_profile_edits(
        &self,
        new_name: &str,
        edits: &mut Vec<RenameEdit>,
        warnings: &mut Vec<RenameWarning>,
    ) -> Result<(), RenameError> {
        let Some(profile) = &self.context.profile else {
            return Ok(());
        };
        let Some(parsed) = profile.profile else {
            warnings.push(RenameWarning::ProfileUnavailable(
                profile
                    .error
                    .clone()
                    .unwrap_or_else(|| "the selected machine profile is unavailable".to_owned()),
            ));
            return Ok(());
        };
        let Some(candidates) = self.binding_candidates() else {
            return Ok(());
        };
        for route in &parsed.routes {
            for endpoint in std::iter::once(&route.source).chain(route.sinks.iter()) {
                let (variable, span) = match endpoint {
                    ProfileEndpoint::PlcInput {
                        variable,
                        variable_span,
                        ..
                    }
                    | ProfileEndpoint::PlcOutput {
                        variable,
                        variable_span,
                        ..
                    } => (variable, *variable_span),
                    _ => continue,
                };
                if !candidates
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(variable))
                {
                    continue;
                }
                let text = profile
                    .sources
                    .file(span.file)
                    .map(stc_span::SourceFile::text)
                    .ok_or(RenameError::MatchedReferenceWithoutSpan)?;
                edits.push(segment_edit(
                    RenameFile::Profile,
                    text,
                    span,
                    variable,
                    new_name,
                )?);
            }
        }
        Ok(())
    }

    fn binding_candidates(&self) -> Option<Vec<String>> {
        let hir = self.context.hir;
        match self.target {
            RenameTarget::Def(def) => {
                let DefKind::Global { at: Some(_), .. } = &hir.defs[def.index()].kind else {
                    return None;
                };
                Some(hir.binding_name_candidates(def))
            }
            RenameTarget::Field(field) => {
                let data = hir.field(field);
                data.at.as_ref()?;
                let DefKind::Program { .. } = hir.defs[data.owner.index()].kind else {
                    return None;
                };
                Some(self.program_field_binding_names(data.owner, hir.interner.text(data.name)))
            }
            _ => None,
        }
    }

    fn program_field_binding_names(&self, program: DefId, field_name: &str) -> Vec<String> {
        let hir = self.context.hir;
        let mut names = Vec::new();
        for def in &hir.defs {
            if def.package.index() != self.context.root_package {
                continue;
            }
            let DefKind::Configuration { config } = &def.kind else {
                continue;
            };
            for resource in &config.resources {
                for instance in &resource.programs {
                    if instance.program == program {
                        names.push(format!("{}.{field_name}", hir.interner.text(instance.name)));
                    }
                }
            }
        }
        names.sort();
        names.dedup();
        names
    }

    fn path_resolution(&self, file: FileId, node: &SyntaxNode) -> Option<RenameTarget> {
        let mut current = Some(node.clone());
        while let Some(node) = current {
            if let Some(resolution) = self.context.resolutions.get(&(file, node.id()))
                && let Some(target) = target_of_res(self.context.hir, resolution)
                && let Ok(target) = public_target(target)
            {
                return Some(target);
            }
            current = node.parent();
        }
        None
    }

    fn manifest_text(&self, file: FileId) -> Result<&str, RenameError> {
        self.context
            .manifest_sources
            .file(file)
            .map(stc_span::SourceFile::text)
            .ok_or(RenameError::MatchedReferenceWithoutSpan)
    }

    fn is_selected_configuration(&self) -> bool {
        let RenameTarget::Def(def) = self.target else {
            return false;
        };
        let data = &self.context.hir.defs[def.index()];
        matches!(data.kind, DefKind::Configuration { .. })
            && self.context.hir.entry_configuration == Some(def)
    }

    fn post_rename_manifest(&self, old_name: &str, new_name: &str) -> Manifest {
        let mut manifest = self.context.manifest.clone();
        if self.is_selected_configuration() {
            manifest.project.configuration = Some(new_name.to_owned());
        }
        if let RenameTarget::Def(def) = self.target {
            let data = &self.context.hir.defs[def.index()];
            if data.test_only && data.package.index() == self.context.root_package {
                for case in &mut manifest.test.cases {
                    if case.name.eq_ignore_ascii_case(old_name) {
                        case.name = new_name.to_owned();
                    }
                }
            }
        }
        manifest
    }

    fn reanalyze(
        &self,
        edits: &[RenameEdit],
        old_name: &str,
        new_name: &str,
    ) -> Result<(), RenameError> {
        let mut parsed = self.context.parsed.to_vec();
        let mut by_file: BTreeMap<FileId, Vec<&RenameEdit>> = BTreeMap::new();
        for edit in edits {
            if let RenameFile::Source(file) = edit.file {
                by_file.entry(file).or_default().push(edit);
            }
        }
        for (file, mut file_edits) in by_file {
            let Some(index) = parsed.get(file.index()).map(|_| file.index()) else {
                return Err(RenameError::MatchedReferenceWithoutSpan);
            };
            let mut text = parsed[index].source().to_owned();
            file_edits.sort_by_key(|edit| std::cmp::Reverse(edit.start));
            for edit in file_edits {
                let start = edit.start as usize;
                let end = edit.end as usize;
                if start >= end || end > text.len() {
                    return Err(RenameError::OverlappingEdits);
                }
                text.replace_range(start..end, &edit.new_text);
            }
            parsed[index] = parse_edited(file, &text, self.context.relaxed);
        }
        let manifest = self.post_rename_manifest(old_name, new_name);
        let (_, diagnostics) = (self.context.analyzer)(&parsed, &manifest);
        let baseline: std::collections::BTreeSet<(&'static str, String, u32)> = self
            .context
            .baseline_diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .map(|diagnostic| {
                (
                    diagnostic.code,
                    diagnostic.message.clone(),
                    diagnostic.primary.file.raw(),
                )
            })
            .collect();
        for diagnostic in diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
        {
            let key = (
                diagnostic.code,
                diagnostic.message.clone(),
                diagnostic.primary.file.raw(),
            );
            if !baseline.contains(&key) {
                return Err(RenameError::Analysis(format!(
                    "{}: {}",
                    diagnostic.code, diagnostic.message
                )));
            }
        }
        Ok(())
    }

    fn classify(
        &self,
        new_name: &str,
        case_only: bool,
    ) -> Result<(Vec<RenameHistoryEntry>, RenameTargetKind, RenameOnline), RenameError> {
        if case_only {
            return Ok((
                Vec::new(),
                RenameTargetKind::HistoryFree,
                RenameOnline::Preserved,
            ));
        }
        let online = if self.is_selected_configuration() {
            RenameOnline::Cold(ColdReason::ConfigurationTaskGuard)
        } else {
            RenameOnline::Preserved
        };
        let hir = self.context.hir;
        match self.target {
            RenameTarget::Param(_) | RenameTarget::Local(_) => {
                Ok((Vec::new(), RenameTargetKind::HistoryFree, online))
            }
            RenameTarget::Def(def) => {
                let data = &hir.defs[def.index()];
                if matches!(data.kind, DefKind::Configuration { .. }) {
                    return Ok((Vec::new(), RenameTargetKind::HistoryFree, online));
                }
                let path = self
                    .canonical_path(RenameTarget::Def(def))
                    .ok_or(RenameError::UntrackedKey)?;
                let entry = history_entry(path, new_name)?;
                Ok((vec![entry], RenameTargetKind::Tracked, online))
            }
            RenameTarget::Field(_) | RenameTarget::EnumValue(_, _) => {
                let path = self
                    .canonical_path(self.target)
                    .ok_or(RenameError::UntrackedKey)?;
                let entry = history_entry(path, new_name)?;
                Ok((vec![entry], RenameTargetKind::Tracked, online))
            }
        }
    }
}

fn history_entry(mut path: Vec<String>, new_name: &str) -> Result<RenameHistoryEntry, RenameError> {
    if path.is_empty() {
        return Err(RenameError::UntrackedKey);
    }
    let old = path.join(".");
    if let Some(last) = path.last_mut() {
        *last = new_name.to_owned();
    }
    Ok(RenameHistoryEntry {
        old,
        new: path.join("."),
    })
}

fn is_root_namespace(data: &stc_hir::Def) -> bool {
    data.parent.is_none() && matches!(data.kind, DefKind::Namespace { .. })
}

fn target_name(hir: &HirCrate, target: Target) -> Option<String> {
    match target {
        Target::Def(def) => {
            let name = hir.defs[def.index()].name?;
            let text = hir.interner.text(name);
            (!text.is_empty()).then(|| text.to_owned())
        }
        Target::Field(field) => Some(hir.interner.text(hir.field(field).name).to_owned()),
        Target::Param(param) => Some(hir.interner.text(hir.param(param).name).to_owned()),
        Target::Local(local) => Some(hir.interner.text(hir.local(local).name).to_owned()),
        Target::EnumValue(def, value) => {
            let data = &hir.defs[def.index()];
            let (DefKind::Enum { values, .. } | DefKind::NamedValue { values, .. }) = &data.kind
            else {
                return None;
            };
            values
                .iter()
                .find(|candidate| candidate.value == value)
                .map(|entry| hir.interner.text(entry.name).to_owned())
        }
        Target::StdFunction(id) => Some(hir.interner.text(hir.std_fns[id.index()].name).to_owned()),
        Target::Type(ty) => Some(hir.type_name(ty)),
    }
}

fn public_target(target: Target) -> Result<RenameTarget, RenameError> {
    match target {
        Target::Def(def) => Ok(RenameTarget::Def(def)),
        Target::Field(field) => Ok(RenameTarget::Field(field)),
        Target::Param(param) => Ok(RenameTarget::Param(param)),
        Target::Local(local) => Ok(RenameTarget::Local(local)),
        Target::EnumValue(def, value) => Ok(RenameTarget::EnumValue(def, value)),
        Target::StdFunction(_) | Target::Type(_) => Err(RenameError::Unsupported),
    }
}

fn std_function_at(hir: &HirCrate, file: FileId, offset: usize, name: &str) -> bool {
    hir.exprs.iter().any(|expr| {
        expr.span.file == file
            && offset >= expr.span.start as usize
            && offset <= expr.span.end as usize
            && matches!(
                &expr.kind,
                ExprKindHir::Call {
                    callee: CalleeHir::StdFunction(id),
                    ..
                } if hir
                    .interner
                    .text(hir.std_fns[id.index()].name)
                    .eq_ignore_ascii_case(name)
            )
    })
}

fn res_matches_target(hir: &HirCrate, resolution: &Res, target: RenameTarget) -> bool {
    match (resolution, target) {
        (Res::Def(def) | Res::Namespace(def), RenameTarget::Def(expected)) => *def == expected,
        (Res::Field(field), RenameTarget::Field(expected)) => *field == expected,
        (Res::Param(param), RenameTarget::Param(expected)) => *param == expected,
        (Res::Local(local), RenameTarget::Local(expected)) => *local == expected,
        (Res::EnumValue(def, value), RenameTarget::EnumValue(expected_def, expected_value)) => {
            *def == expected_def && *value == expected_value
        }
        (Res::Type(ty), RenameTarget::Def(expected)) => {
            matches!(hir.types.get(*ty), stc_hir::Ty::Named(def) if *def == expected)
        }
        _ => false,
    }
}

fn cursor_in_span(span: Span, file: FileId, offset: usize) -> bool {
    span.file == file && offset >= span.start as usize && offset <= span.end as usize
}

fn program_access_fields(hir: &HirCrate, place: ExprId) -> Vec<FieldId> {
    let mut fields = Vec::new();
    let mut current = Some(place);
    while let Some(expr) = current {
        match &hir.expr(expr).kind {
            ExprKindHir::Place(PlaceHir::Field { base, field }) => {
                fields.push(*field);
                current = Some(*base);
            }
            _ => break,
        }
    }
    fields.reverse();
    fields
}

fn push_span_edit(edits: &mut Vec<RenameEdit>, span: Span, new_name: &str) {
    if span.start < span.end {
        edits.push(RenameEdit {
            file: RenameFile::Source(span.file),
            start: span.start,
            end: span.end,
            new_text: new_name.to_owned(),
        });
    }
}

fn push_whole_edit(edits: &mut Vec<RenameEdit>, file: RenameFile, span: Span, new_name: &str) {
    if span.start < span.end {
        edits.push(RenameEdit {
            file,
            start: span.start,
            end: span.end,
            new_text: new_name.to_owned(),
        });
    }
}

fn segment_edit(
    file: RenameFile,
    text: &str,
    span: Span,
    value: &str,
    new_name: &str,
) -> Result<RenameEdit, RenameError> {
    let raw = text
        .get(span.start as usize..span.end as usize)
        .ok_or(RenameError::MatchedReferenceWithoutSpan)?;
    let last = value.rsplit('.').next().unwrap_or(value);
    let raw_last = raw.rsplit('.').next().unwrap_or(raw);
    if !raw_last.eq_ignore_ascii_case(last) {
        return Err(RenameError::MatchedReferenceWithoutSpan);
    }
    let length =
        u32::try_from(raw_last.len()).map_err(|_| RenameError::MatchedReferenceWithoutSpan)?;
    Ok(RenameEdit {
        file,
        start: span.end - length,
        end: span.end,
        new_text: new_name.to_owned(),
    })
}

fn path_starts_with(path: &[String], prefix: &[String]) -> bool {
    path.len() >= prefix.len()
        && path
            .iter()
            .zip(prefix)
            .all(|(segment, expected)| segment.eq_ignore_ascii_case(expected))
}

fn edits_overlap(edits: &[RenameEdit]) -> bool {
    for pair in edits.windows(2) {
        let (left, right) = (&pair[0], &pair[1]);
        if left.file == right.file && right.start < left.end {
            return true;
        }
    }
    false
}

fn collect_nodes(node: &SyntaxNode, out: &mut Vec<SyntaxNode>) {
    out.push(node.clone());
    for child in node.child_nodes() {
        collect_nodes(&child, out);
    }
}

fn parse_edited(file: FileId, text: &str, relaxed: bool) -> ParsedFile {
    let mut sink = DiagnosticSink::new();
    let lexed = tokenize_bytes(file, text.as_bytes(), &mut sink);
    parse(
        file,
        lexed.text.as_str(),
        &lexed.stream,
        ParseOptions {
            relaxed_semicolons: relaxed,
            ..ParseOptions::default()
        },
        &mut sink,
    )
}
