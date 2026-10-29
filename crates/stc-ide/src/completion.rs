//! Bounded completion: keywords, visible names, and one-level member
//! completion after `.`.
//!
//! General completion offers every keyword plus the names declared in the
//! enclosing POU (fields, methods, parameters, and locals), the members of the
//! enclosing method's owner type, the package-level declarations of the file's
//! own package, and the names made visible by the file's `USING` directives.
//! Member completion resolves the receiver expression
//! through the analysis resolutions and offers the declared fields and methods
//! of its type. Names are deduplicated case-insensitively and sorted.

use std::collections::BTreeSet;

use stc_hir::{Def, DefId, DefKind, HirCrate, Ty, TyId};
use stc_lexer::Keyword;
use stc_parser::ParsedFile;
use stc_sema::Res;
use stc_span::FileId;
use stc_syntax::ast::{AstNode, UsingDirective};
use stc_syntax::{SyntaxElement, SyntaxKind, SyntaxToken};

use crate::snapshot::Snapshot;

/// The kind of a completion item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompletionKind {
    /// A reserved keyword.
    Keyword,
    /// A `FUNCTION`.
    Function,
    /// A `FUNCTION_BLOCK`.
    FunctionBlock,
    /// A `PROGRAM`.
    Program,
    /// A `CLASS`.
    Class,
    /// An `INTERFACE`.
    Interface,
    /// A `METHOD`.
    Method,
    /// A type declaration.
    Type,
    /// A variable, field, parameter, or local.
    Variable,
    /// A field of the receiver type.
    Field,
    /// A parameter of the enclosing POU.
    Parameter,
    /// A local of the enclosing POU.
    Local,
    /// An enumeration value.
    EnumValue,
    /// A namespace.
    Namespace,
    /// A configuration.
    Configuration,
}

/// One completion candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    /// Spelling offered to the editor.
    pub label: String,
    /// Candidate category.
    pub kind: CompletionKind,
    /// Declared type or result type, when known.
    pub detail: Option<String>,
}

/// The type-bearing identity of a member-access receiver.
enum Receiver {
    Def(DefId),
    Type(TyId),
}

impl Snapshot {
    /// Returns completion candidates at the byte `offset`.
    ///
    /// Immediately after a `.` (or while typing its member name) the result is
    /// the declared fields and methods of the receiver's type; an unresolvable
    /// receiver yields an empty list. Everywhere else the result is keywords
    /// plus visible names. Offsets are clamped to the source length.
    #[must_use]
    pub fn completion(&self, file: FileId, offset: usize) -> Vec<CompletionItem> {
        let Some(parsed) = self.parsed(file) else {
            return Vec::new();
        };
        if let Some(dot) = member_context(parsed, offset) {
            let Some(receiver) = self.receiver_type(&dot) else {
                return Vec::new();
            };
            return self.member_completions(receiver);
        }
        self.general_completions(file, parsed, offset)
    }

    fn member_completions(&self, receiver: Receiver) -> Vec<CompletionItem> {
        let hir = self.hir();
        let def = match receiver {
            Receiver::Def(def) => def,
            Receiver::Type(ty) => match hir.types.get(ty) {
                Ty::Named(def) => *def,
                _ => return Vec::new(),
            },
        };
        let data = &hir.defs[def.index()];
        let mut items = Vec::new();
        for field in data.field_ids() {
            let field = hir.field(*field);
            items.push(CompletionItem {
                label: self.spelling(field.span, field.name),
                kind: CompletionKind::Field,
                detail: Some(hir.type_name(field.ty)),
            });
        }
        for method in data.method_ids() {
            let method = &hir.defs[method.index()];
            let Some(name) = method.name else {
                continue;
            };
            items.push(CompletionItem {
                label: self.spelling(method.span, name),
                kind: CompletionKind::Method,
                detail: result_type(hir, &method.kind),
            });
        }
        sort_and_dedup(items)
    }

    fn general_completions(
        &self,
        file: FileId,
        parsed: &ParsedFile,
        offset: usize,
    ) -> Vec<CompletionItem> {
        let hir = self.hir();
        let mut seen = BTreeSet::new();
        let mut items = Vec::new();
        let mut push = |label: String, kind: CompletionKind, detail: Option<String>| {
            if seen.insert(label.to_ascii_uppercase()) {
                items.push(CompletionItem {
                    label,
                    kind,
                    detail,
                });
            }
        };
        if let Some(def) = self.enclosing_pou(file, offset) {
            let data = &hir.defs[def.index()];
            for field in data.field_ids() {
                let field = hir.field(*field);
                push(
                    self.spelling(field.span, field.name),
                    CompletionKind::Field,
                    Some(hir.type_name(field.ty)),
                );
            }
            for method in data.method_ids() {
                let method = &hir.defs[method.index()];
                if let Some(name) = method.name {
                    push(
                        self.spelling(method.span, name),
                        CompletionKind::Method,
                        result_type(hir, &method.kind),
                    );
                }
            }
            for param in &hir.params {
                if param.owner == def {
                    push(
                        self.spelling(param.span, param.name),
                        CompletionKind::Parameter,
                        Some(hir.type_name(param.ty)),
                    );
                }
            }
            for local in &hir.locals {
                if local.owner == def {
                    push(
                        self.spelling(local.span, local.name),
                        CompletionKind::Local,
                        Some(hir.type_name(local.ty)),
                    );
                }
            }
            if let DefKind::Method { owner, .. } = data.kind {
                let owner_data = &hir.defs[owner.index()];
                for field in owner_data.field_ids() {
                    let field = hir.field(*field);
                    push(
                        self.spelling(field.span, field.name),
                        CompletionKind::Field,
                        Some(hir.type_name(field.ty)),
                    );
                }
                for method in owner_data.method_ids() {
                    let method = &hir.defs[method.index()];
                    if let Some(name) = method.name {
                        push(
                            self.spelling(method.span, name),
                            CompletionKind::Method,
                            result_type(hir, &method.kind),
                        );
                    }
                }
            }
        }
        if let Some(info) = self.file(file) {
            for def in &hir.defs {
                if def.package.index() != info.package_index || !namespace_scoped(hir, def) {
                    continue;
                }
                let Some(name) = def.name else {
                    continue;
                };
                let label = self.spelling(def.span, name);
                if label.is_empty() {
                    continue;
                }
                push(label, completion_kind(&def.kind), def_detail(hir, def));
            }
        }
        // Names imported by `USING` are visible unqualified.
        for segments in using_paths(parsed) {
            let Some(namespace) = resolve_namespace(hir, &segments) else {
                continue;
            };
            let data = &hir.defs[namespace.index()];
            if let Some(name) = data.name {
                push(
                    self.spelling(data.span, name),
                    CompletionKind::Namespace,
                    None,
                );
            }
            for def in &hir.defs {
                if def.parent != Some(namespace) {
                    continue;
                }
                let Some(name) = def.name else {
                    continue;
                };
                let label = self.spelling(def.span, name);
                if !label.is_empty() {
                    push(label, completion_kind(&def.kind), def_detail(hir, def));
                }
            }
        }
        for keyword in Keyword::ALL {
            push(keyword.as_str().to_owned(), CompletionKind::Keyword, None);
        }
        sort_and_dedup(items)
    }

    /// Returns the receiver type of the member expression whose `.` is `dot`.
    fn receiver_type(&self, dot: &SyntaxToken) -> Option<Receiver> {
        let file = dot.span().file;
        let container = dot.parent()?;
        let dot_start = dot.span().start;
        let mut best: Option<(u32, Receiver)> = None;
        let mut elements = container.descendants();
        elements.push(stc_syntax::SyntaxElement::Node(container.clone()));
        for element in elements {
            let stc_syntax::SyntaxElement::Node(node) = element else {
                continue;
            };
            let span = node.token_span();
            if span.is_empty() || span.end > dot_start {
                continue;
            }
            if let Some(resolution) = self.resolutions().get(&(file, node.id()))
                && let Some(receiver) = receiver_of_res(self.hir(), resolution)
                && best.as_ref().is_none_or(|(start, _)| span.start >= *start)
            {
                best = Some((span.start, receiver));
            }
        }
        if best.is_none()
            && let Some(resolution) = self.resolutions().get(&(file, container.id()))
        {
            best = receiver_of_res(self.hir(), resolution).map(|receiver| (0, receiver));
        }
        best.map(|(_, receiver)| receiver)
    }

    /// Returns the innermost POU or method declaration containing `offset`.
    fn enclosing_pou(&self, file: FileId, offset: usize) -> Option<DefId> {
        let mut best: Option<(u32, DefId)> = None;
        for (index, def) in self.hir().defs.iter().enumerate() {
            if def.span.file != file
                || !matches!(
                    def.kind,
                    DefKind::Function { .. }
                        | DefKind::FunctionBlock { .. }
                        | DefKind::Program { .. }
                        | DefKind::Class { .. }
                        | DefKind::Interface { .. }
                        | DefKind::Method { .. }
                )
            {
                continue;
            }
            let start = def.span.start as usize;
            let end = def.span.end as usize;
            if offset < start || offset > end {
                continue;
            }
            if best.is_none_or(|(best_size, _)| def.span.len() < best_size) {
                best = Some((
                    def.span.len(),
                    DefId::new(u32::try_from(index).expect("definition index fits in u32")),
                ));
            }
        }
        best.map(|(_, def)| def)
    }
}

/// Returns the `.` token when `offset` is in a member-access suffix.
fn member_context(parsed: &ParsedFile, offset: usize) -> Option<SyntaxToken> {
    let tree = parsed.tree();
    let offset = offset.min(tree.source().len());
    let token = match tree.token_at(offset) {
        Some(token) if token.kind() != SyntaxKind::Eof => token,
        _ => {
            let token = tree.token_before(offset)?;
            (token.span().end as usize == offset).then_some(token)?
        }
    };
    if token.kind() == SyntaxKind::PunctDot {
        return Some(token);
    }
    if token.kind() != SyntaxKind::Ident {
        return None;
    }
    // `drive.St` parses as one path, and the name token sits inside a `Name`
    // node, so the dot is the preceding token rather than a direct sibling.
    let before = tree.token_before(token.span().start as usize)?;
    (before.kind() == SyntaxKind::PunctDot).then_some(before)
}

/// Returns the namespace paths named by `USING` directives in `parsed`.
fn using_paths(parsed: &ParsedFile) -> Vec<Vec<String>> {
    let mut paths = Vec::new();
    for element in parsed.tree().root().descendants() {
        let SyntaxElement::Node(node) = element else {
            continue;
        };
        if node.kind() != SyntaxKind::UsingDirective {
            continue;
        }
        let Some(directive) = UsingDirective::cast(&node) else {
            continue;
        };
        for path in directive.namespaces() {
            let segments: Vec<String> = path
                .segments()
                .iter()
                .map(|name| name.text().to_owned())
                .collect();
            if !segments.is_empty() {
                paths.push(segments);
            }
        }
    }
    paths
}

/// Resolves a `USING` path to its namespace definition.
fn resolve_namespace(hir: &HirCrate, segments: &[String]) -> Option<DefId> {
    let mut current = None;
    for (depth, segment) in segments.iter().enumerate() {
        let mut found = None;
        for (index, def) in hir.defs.iter().enumerate() {
            if !matches!(def.kind, DefKind::Namespace { .. }) {
                continue;
            }
            let parent_matches = if depth == 0 {
                def.parent.is_none()
            } else {
                def.parent == current
            };
            if parent_matches
                && def
                    .name
                    .is_some_and(|name| hir.interner.text(name).eq_ignore_ascii_case(segment))
            {
                found = Some(DefId::new(
                    u32::try_from(index).expect("definition index fits in u32"),
                ));
                break;
            }
        }
        current = Some(found?);
    }
    current
}

fn receiver_of_res(hir: &HirCrate, resolution: &Res) -> Option<Receiver> {
    Some(match resolution {
        Res::Local(local) => Receiver::Type(hir.local(*local).ty),
        Res::Param(param) => Receiver::Type(hir.param(*param).ty),
        Res::Field(field) => Receiver::Type(hir.field(*field).ty),
        Res::Def(def) => match &hir.defs[def.index()].kind {
            DefKind::Global { ty, .. } => Receiver::Type(*ty),
            _ => return None,
        },
        Res::Type(ty) => Receiver::Type(*ty),
        Res::EnumValue(def, _) => Receiver::Def(*def),
        _ => return None,
    })
}

fn completion_kind(kind: &DefKind) -> CompletionKind {
    match kind {
        DefKind::Namespace { .. } => CompletionKind::Namespace,
        DefKind::Alias { .. }
        | DefKind::Subrange { .. }
        | DefKind::Enum { .. }
        | DefKind::NamedValue { .. }
        | DefKind::Struct { .. } => CompletionKind::Type,
        DefKind::Function { .. } => CompletionKind::Function,
        DefKind::FunctionBlock { .. } => CompletionKind::FunctionBlock,
        DefKind::Program { .. } => CompletionKind::Program,
        DefKind::Class { .. } => CompletionKind::Class,
        DefKind::Interface { .. } => CompletionKind::Interface,
        DefKind::Method { .. } => CompletionKind::Method,
        DefKind::Global { .. } => CompletionKind::Variable,
        DefKind::Configuration { .. } => CompletionKind::Configuration,
    }
}

fn def_detail(hir: &HirCrate, def: &Def) -> Option<String> {
    match &def.kind {
        DefKind::Global { ty, .. } => Some(hir.type_name(*ty)),
        DefKind::Function { result, .. } | DefKind::Method { result, .. } => {
            result.map(|ty| hir.type_name(ty))
        }
        _ => None,
    }
}

fn result_type(hir: &HirCrate, kind: &DefKind) -> Option<String> {
    match kind {
        DefKind::Function { result, .. } | DefKind::Method { result, .. } => {
            result.map(|ty| hir.type_name(ty))
        }
        _ => None,
    }
}

/// Returns whether every ancestor of `def` is a namespace.
fn namespace_scoped(hir: &HirCrate, def: &Def) -> bool {
    let mut parent = def.parent;
    while let Some(current) = parent {
        let data = &hir.defs[current.index()];
        if !matches!(data.kind, DefKind::Namespace { .. }) {
            return false;
        }
        parent = data.parent;
    }
    true
}

fn sort_and_dedup(mut items: Vec<CompletionItem>) -> Vec<CompletionItem> {
    items.sort_by(|left, right| {
        left.label
            .to_ascii_uppercase()
            .cmp(&right.label.to_ascii_uppercase())
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.kind.cmp(&right.kind))
    });
    items.dedup();
    items
}
