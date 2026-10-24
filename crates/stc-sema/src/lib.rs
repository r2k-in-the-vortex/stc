//! Name resolution, type checking, constant evaluation, and HIR lowering.
//!
//! The analyzer consumes the typed AST from `stc-syntax` (through parsed
//! files), resolves names and types project-wide, and lowers every body into
//! `stc-hir`. Collection and lowering are deterministic (D-072).

#![forbid(unsafe_code)]
#![allow(
    clippy::cloned_ref_to_slice_refs,
    clippy::collapsible_if,
    clippy::let_and_return,
    clippy::manual_contains,
    clippy::manual_is_multiple_of,
    clippy::question_mark,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::unnecessary_to_owned,
    clippy::while_let_loop
)]

mod check;
pub mod codes;
mod collect;
mod consteval;
mod prelude;
mod resolve;
mod session;
mod types;

use std::collections::{BTreeMap, BTreeSet};

use stc_diag::{Diagnostic, DiagnosticSink};
use stc_hir::{
    ConstValue, DefId, ExprId, FieldId, HirCrate, LocalId, ParamId, StdFnId, Symbol, TyId,
    Visibility,
};
use stc_parser::ParsedFile;
use stc_span::{FileId, Span};
use stc_syntax::ast::AstNode;
use stc_syntax::ast::{
    ClassDecl, ConfigDecl, FunctionBlockDecl, FunctionDecl, Initializer, InterfaceDecl, MethodDecl,
    MethodPrototype, Path, ProgramDecl, TypeDecl, TypeRef, UsingDirective, VarSection,
};
use stc_syntax::{NodeId, SyntaxNode};

pub use session::{Analysis, Res, Session, analyze, analyze_with};

/// Empty span used for synthesized declarations.
pub(crate) const EMPTY_SPAN: Span = Span::new(FileId::new(0), 0, 0);

/// One namespace or configuration scope.
#[derive(Debug, Default)]
pub(crate) struct ScopeData {
    /// Definitions by normalized name (overload sets keep every entry).
    pub entries: BTreeMap<Symbol, Vec<DefId>>,
    /// Named constants (enum values, named values).
    pub constants: BTreeMap<Symbol, ConstBinding>,
    /// `USING` directives in source order.
    pub using_paths: Vec<UsingSite>,
    /// Resolved `USING` namespaces.
    pub using: Vec<DefId>,
}

/// One named constant.
#[derive(Clone, Debug)]
pub(crate) struct ConstBinding {
    pub owner: Option<DefId>,
    pub ty: TyId,
    pub value: ConstValue,
    pub span: Span,
}

/// One `USING` directive.
#[derive(Clone, Debug)]
pub(crate) struct UsingSite {
    pub ns: DefId,
    pub path: Path,
    pub span: Span,
}

/// One named type declaration awaiting resolution.
#[derive(Clone, Debug)]
pub(crate) struct TypeSite {
    pub def: DefId,
    pub ns: DefId,
    pub decl: TypeDecl,
}

/// One declared variable name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeclId {
    Global(DefId),
    Field(FieldId),
    Param(ParamId),
    Local(LocalId),
}

/// One declaration site (one name of a possibly multi-name declaration).
#[derive(Clone, Debug)]
pub(crate) struct DeclSite {
    pub id: DeclId,
    pub ns: DefId,
    pub section: stc_hir::VarSectionKind,
    pub retain: stc_hir::RetainPolicy,
    pub constant: bool,
    pub name_span: Span,
    pub ty_ref: Option<TypeRef>,
    pub at: Option<stc_lexer::DirectVariable>,
    pub at_span: Option<Span>,
    pub init: Option<Initializer>,
    pub edge: bool,
}

/// A POU or method declaration awaiting signature/body resolution.
#[derive(Clone, Debug)]
pub(crate) enum PouNode {
    Function(FunctionDecl),
    FunctionBlock(FunctionBlockDecl),
    Program(ProgramDecl),
    Class(ClassDecl),
    Interface(InterfaceDecl),
    Method(MethodDecl),
    Prototype(MethodPrototype),
}

impl PouNode {
    pub(crate) fn var_sections(&self) -> Vec<VarSection> {
        match self {
            Self::Function(node) => node.var_sections(),
            Self::FunctionBlock(node) => node.var_sections(),
            Self::Program(node) => node.var_sections(),
            Self::Class(node) => node.var_sections(),
            Self::Interface(node) => node.var_sections(),
            Self::Method(node) => node.var_sections(),
            Self::Prototype(node) => node.var_sections(),
        }
    }

    pub(crate) fn using_directives(&self) -> Vec<UsingDirective> {
        match self {
            Self::Function(node) => node.using_directives(),
            Self::FunctionBlock(node) => node.using_directives(),
            Self::Program(node) => node.using_directives(),
            Self::Class(node) => node.using_directives(),
            Self::Interface(node) => node.using_directives(),
            Self::Method(node) => node.using_directives(),
            Self::Prototype(_) => Vec::new(),
        }
    }

    pub(crate) fn pragmas(&self) -> Vec<stc_syntax::ast::Pragma> {
        match self {
            Self::Function(node) => node.pragmas(),
            Self::FunctionBlock(node) => node.pragmas(),
            Self::Program(node) => node.pragmas(),
            Self::Class(node) => node.pragmas(),
            Self::Interface(node) => node.pragmas(),
            Self::Method(node) => node.pragmas(),
            Self::Prototype(_) => Vec::new(),
        }
    }
}

/// One POU/method collected from a file.
#[derive(Clone, Debug)]
pub(crate) struct PouSite {
    pub def: DefId,
    pub node: PouNode,
    pub owner: Option<DefId>,
}

/// One configuration declaration.
#[derive(Clone, Debug)]
pub(crate) struct ConfigSite {
    pub def: DefId,
    pub ns: DefId,
    pub decl: ConfigDecl,
}

/// Alias resolution state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AliasState {
    Unresolved,
    Resolving,
    Resolved,
}

/// The semantic analyzer for one build.
pub(crate) struct Analyzer<'a> {
    pub files: &'a [ParsedFile],
    pub hir: HirCrate,
    pub diags: &'a mut DiagnosticSink,
    pub resolutions: BTreeMap<(FileId, NodeId), Res>,
    pub root_ns: DefId,
    pub namespaces: BTreeMap<Vec<Symbol>, DefId>,
    pub scopes: BTreeMap<DefId, ScopeData>,
    pub type_sites: Vec<TypeSite>,
    pub field_inits: Vec<(FieldId, DefId, Initializer)>,
    pub type_site_by_def: BTreeMap<DefId, usize>,
    pub resolved_type_defs: BTreeSet<DefId>,
    pub alias_state: BTreeMap<DefId, AliasState>,
    pub decl_sites: Vec<DeclSite>,
    pub pou_sites: Vec<PouSite>,
    pub config_sites: Vec<ConfigSite>,
    pub locals_by_owner: BTreeMap<DefId, Vec<LocalId>>,
    pub test_only: BTreeMap<Symbol, Span>,
    pub std_fns: BTreeMap<Symbol, Vec<StdFnId>>,
    pub std_fbs: BTreeMap<Symbol, DefId>,
    pub entry_configuration: Option<DefId>,
    pub const_globals: BTreeMap<DefId, ConstValue>,
    pub const_in_progress: BTreeSet<DefId>,
}

impl<'a> Analyzer<'a> {
    pub(crate) fn new(
        session: &Session,
        files: &'a [ParsedFile],
        diags: &'a mut DiagnosticSink,
    ) -> Self {
        let mut hir = HirCrate::new(session.package_name.clone(), session.root_namespace.clone());
        let root_symbol = hir.intern(&session.root_namespace);
        let root_ns = hir.alloc_def(
            stc_hir::DefKind::Namespace { internal: false },
            Some(root_symbol),
            EMPTY_SPAN,
            None,
            Visibility::Public,
        );
        let mut analyzer = Self {
            files,
            hir,
            diags,
            resolutions: BTreeMap::new(),
            root_ns,
            namespaces: BTreeMap::new(),
            scopes: BTreeMap::new(),
            type_sites: Vec::new(),
            field_inits: Vec::new(),
            type_site_by_def: BTreeMap::new(),
            resolved_type_defs: BTreeSet::new(),
            alias_state: BTreeMap::new(),
            decl_sites: Vec::new(),
            pou_sites: Vec::new(),
            config_sites: Vec::new(),
            locals_by_owner: BTreeMap::new(),
            test_only: BTreeMap::new(),
            std_fns: BTreeMap::new(),
            std_fbs: BTreeMap::new(),
            entry_configuration: None,
            const_globals: BTreeMap::new(),
            const_in_progress: BTreeSet::new(),
        };
        analyzer.namespaces.insert(vec![root_symbol], root_ns);
        analyzer.scopes.insert(root_ns, ScopeData::default());
        analyzer.install_prelude();
        analyzer
    }

    pub(crate) fn run(&mut self) {
        self.collect_files();
        self.resolve_using_directives();
        self.resolve_aliases();
        self.resolve_type_declarations();
        self.resolve_pou_declarations();
        self.resolve_initializers();
        self.check_bodies();
        self.check_configurations();
        self.hir.entry_configuration = self.entry_configuration;
    }

    pub(crate) fn error(&mut self, code: &'static str, message: impl Into<String>, span: Span) {
        self.diags.error(code, message, span);
    }

    pub(crate) fn warning(&mut self, code: &'static str, message: impl Into<String>, span: Span) {
        self.diags.warning(code, message, span);
    }

    pub(crate) fn push_diag(&mut self, diagnostic: Diagnostic) {
        self.diags.push(diagnostic);
    }

    pub(crate) fn record(&mut self, node: &SyntaxNode, res: Res) {
        let file = node.tree().file();
        self.resolutions.insert((file, node.id()), res);
    }

    pub(crate) fn symbol_text(&self, symbol: Symbol) -> &str {
        self.hir.interner.text(symbol)
    }

    pub(crate) fn type_name(&self, ty: TyId) -> String {
        self.hir.type_name(ty)
    }

    pub(crate) fn def_name(&self, def: DefId) -> String {
        self.hir.def_name(def).to_string()
    }

    pub(crate) fn add_entry(
        &mut self,
        ns: DefId,
        symbol: Symbol,
        def: DefId,
        span: Span,
        allow_overload: bool,
    ) {
        let existing = self
            .scopes
            .entry(ns)
            .or_default()
            .entries
            .get(&symbol)
            .cloned()
            .unwrap_or_default();
        if !existing.is_empty() && !allow_overload {
            let original = existing[0];
            let original_span = self.hir.defs[original.index()].span;
            let name = self.symbol_text(symbol).to_string();
            self.push_diag(
                Diagnostic::error(
                    codes::NAME_DUPLICATE,
                    format!("duplicate definition of `{name}`"),
                    span,
                )
                .with_label(original_span, "previous definition is here")
                .with_note("names in one scope must be unique (case-insensitive)"),
            );
        }
        self.scopes
            .entry(ns)
            .or_default()
            .entries
            .entry(symbol)
            .or_default()
            .push(def);
    }

    pub(crate) fn add_constant(&mut self, ns: DefId, symbol: Symbol, binding: ConstBinding) {
        let existing_span = self
            .scopes
            .get(&ns)
            .and_then(|scope| scope.constants.get(&symbol))
            .map(|existing| existing.span);
        if let Some(previous) = existing_span {
            let message = format!("duplicate named value `{}`", self.hir.interner.text(symbol));
            self.push_diag(
                Diagnostic::error(codes::NAME_DUPLICATE, message, binding.span)
                    .with_label(previous, "previous definition is here"),
            );
            return;
        }
        let conflicts = self
            .scopes
            .get(&ns)
            .is_some_and(|scope| scope.entries.contains_key(&symbol));
        if conflicts {
            let message = format!(
                "named value `{}` conflicts with another declaration",
                self.hir.interner.text(symbol)
            );
            self.push_diag(Diagnostic::error(
                codes::NAME_DUPLICATE,
                message,
                binding.span,
            ));
            return;
        }
        self.scopes
            .entry(ns)
            .or_default()
            .constants
            .insert(symbol, binding);
    }

    pub(crate) fn alloc_error_expr(&mut self, span: Span) -> ExprId {
        let ty = self.hir.types.error();
        self.hir.alloc_expr(stc_hir::ExprKindHir::Error, ty, span)
    }
}

/// Convenience helper used by tests and the driver.
#[must_use]
pub fn analyze_files(files: &[ParsedFile], diagnostics: &mut DiagnosticSink) -> Analysis {
    analyze(files, diagnostics)
}

/// Returns the declaration name of a type declaration.
pub(crate) fn type_decl_name(decl: &TypeDecl) -> Option<stc_syntax::ast::Name> {
    match decl {
        TypeDecl::Simple(node) => node.name(),
        TypeDecl::Subrange(node) => node.name(),
        TypeDecl::Enum(node) => node.name(),
        TypeDecl::NamedValue(node) => node.name(),
        TypeDecl::Array(node) => node.name(),
        TypeDecl::Struct(node) => node.name(),
        TypeDecl::String(node) => node.name(),
        TypeDecl::Ref(node) => node.name(),
        TypeDecl::Error(_) => None,
    }
}

/// Returns the token extent of a type declaration.
pub(crate) fn type_decl_span(decl: &TypeDecl) -> Span {
    match decl {
        TypeDecl::Simple(node) => node.token_span(),
        TypeDecl::Subrange(node) => node.token_span(),
        TypeDecl::Enum(node) => node.token_span(),
        TypeDecl::NamedValue(node) => node.token_span(),
        TypeDecl::Array(node) => node.token_span(),
        TypeDecl::Struct(node) => node.token_span(),
        TypeDecl::String(node) => node.token_span(),
        TypeDecl::Ref(node) => node.token_span(),
        TypeDecl::Error(node) => node.token_span(),
    }
}
