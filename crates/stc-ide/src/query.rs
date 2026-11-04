//! Definition, references, and hover over the analysis resolutions.
//!
//! The mapping follows `stc rename`: a cursor lands on a non-trivia token, the
//! innermost CST node with a recorded [`Res`] is found by walking parents, and
//! the resolution is normalized to a declaration identity. References invert
//! the same resolution map and pick the identifier token that spells the
//! declared name inside each resolved node, so only real source tokens are
//! reported.

use std::collections::BTreeMap;

use stc_hir::{DefId, DefKind, FieldId, HirCrate, LocalId, ParamId, StdFnId, Ty, TyId};
use stc_parser::ParsedFile;
use stc_sema::Res;
use stc_span::{FileId, Span};
use stc_syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

use crate::snapshot::{Snapshot, SourceLocation};

/// Hover information for one identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoverInfo {
    /// Plain-text contents: a `name : TYPE` line when a type is known, then the
    /// declaration kind and its owner.
    pub contents: String,
    /// The hovered identifier's span.
    pub span: Span,
}

/// The declaration identity a resolution names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Target {
    Def(DefId),
    Field(FieldId),
    Param(ParamId),
    Local(LocalId),
    EnumValue(DefId, i128),
    StdFunction(StdFnId),
    Type(TyId),
}

/// Maps declaration-name tokens to their declaration identity.
///
/// Used when the cursor is on the declaration itself, which carries no
/// resolution entry.
#[derive(Default)]
pub(crate) struct DeclarationIndex {
    by_token: BTreeMap<(FileId, u32, u32), Target>,
}

impl DeclarationIndex {
    pub(crate) fn build(hir: &HirCrate, parsed: &[ParsedFile]) -> Self {
        let mut by_token = BTreeMap::new();
        let mut add = |file: FileId, span: Span, name: &str, target: Target| {
            let Some(parsed) = parsed.get(file.index()) else {
                return;
            };
            if let Some((start, end)) = name_token(parsed, span, name) {
                by_token.insert((file, start, end), target);
            }
        };
        for (index, def) in hir.defs.iter().enumerate() {
            let Some(name) = def.name else {
                continue;
            };
            let name = hir.interner.text(name);
            if !name.is_empty() {
                add(def.span.file, def.span, name, Target::Def(def_id(index)));
            }
        }
        for (index, field) in hir.fields.iter().enumerate() {
            add(
                field.span.file,
                field.span,
                hir.interner.text(field.name),
                Target::Field(field_id(index)),
            );
        }
        for (index, param) in hir.params.iter().enumerate() {
            add(
                param.span.file,
                param.span,
                hir.interner.text(param.name),
                Target::Param(param_id(index)),
            );
        }
        for (index, local) in hir.locals.iter().enumerate() {
            add(
                local.span.file,
                local.span,
                hir.interner.text(local.name),
                Target::Local(local_id(index)),
            );
        }
        for (index, def) in hir.defs.iter().enumerate() {
            let (DefKind::Enum { values, .. } | DefKind::NamedValue { values, .. }) = &def.kind
            else {
                continue;
            };
            for value in values {
                add(
                    value.span.file,
                    value.span,
                    hir.interner.text(value.name),
                    Target::EnumValue(def_id(index), value.value),
                );
            }
        }
        Self { by_token }
    }
}

impl Snapshot {
    /// Returns the declaration of the identifier at `offset`.
    ///
    /// Cursors on either a use or a declaration name resolve; byte offsets are
    /// clamped to the source length. `None` when the position is not on an
    /// identifier with a declaration (for example a keyword, a literal, an
    /// unresolved name, or a standard function).
    #[must_use]
    pub fn definition(&self, file: FileId, offset: usize) -> Option<SourceLocation> {
        self.declaration_location(self.target_at(file, offset)?)
    }

    /// Returns the identifier references to the declaration at `offset`.
    ///
    /// Every resolution with the same identity contributes the identifier token
    /// that spells the declared name, across all packages and test sources.
    /// `include_declaration` adds the declaration's own name. Results are
    /// deduplicated and sorted by file and byte offset.
    #[must_use]
    pub fn references(
        &self,
        file: FileId,
        offset: usize,
        include_declaration: bool,
    ) -> Vec<SourceLocation> {
        let Some(target) = self.target_at(file, offset) else {
            return Vec::new();
        };
        let mut locations = self.collect_references(target);
        if include_declaration && let Some(location) = self.declaration_location(target) {
            locations.push(location);
        }
        locations.sort_by_key(|location| (location.file, location.span.start, location.span.end));
        locations.dedup();
        locations
    }

    /// Returns hover information for the identifier at `offset`.
    ///
    /// The contents name the declared type when there is one, the declaration
    /// kind, and the owner for fields, parameters, locals, methods, and enum
    /// values. Non-identifier positions return `None`.
    #[must_use]
    pub fn hover(&self, file: FileId, offset: usize) -> Option<HoverInfo> {
        let parsed = self.parsed(file)?;
        let token = cursor_ident(parsed, offset)?;
        let target = self.target_for_token(file, &token)?;
        Some(HoverInfo {
            contents: self.hover_contents(target, token.text())?,
            span: token.span(),
        })
    }

    fn target_at(&self, file: FileId, offset: usize) -> Option<Target> {
        let parsed = self.parsed(file)?;
        let token = cursor_ident(parsed, offset)?;
        self.target_for_token(file, &token)
    }

    fn target_for_token(&self, file: FileId, token: &SyntaxToken) -> Option<Target> {
        // Path resolutions are recorded on the whole path, so a resolution may
        // name a different segment than the token under the cursor (for
        // example the method in `receiver.Method`). Accept it only when the
        // declared name matches the token; otherwise keep walking outward.
        let mut node = token.parent();
        while let Some(current) = node {
            if let Some(resolution) = self.resolutions().get(&(file, current.id()))
                && let Some(target) = target_of_res(self.hir(), resolution)
                && self.target_matches_token(target, token)
            {
                return Some(target);
            }
            node = current.parent();
        }
        self.declarations()
            .by_token
            .get(&(file, token.span().start, token.span().end))
            .copied()
    }

    fn target_matches_token(&self, target: Target, token: &SyntaxToken) -> bool {
        self.target_name(target)
            .is_some_and(|name| token.text().eq_ignore_ascii_case(&name))
    }

    fn collect_references(&self, target: Target) -> Vec<SourceLocation> {
        let Some(name) = self.target_name(target) else {
            return Vec::new();
        };
        let mut locations = Vec::new();
        for ((file, node_id), resolution) in self.resolutions() {
            if !res_matches_target(self.hir(), resolution, target) {
                continue;
            }
            let Some(parsed) = self.parsed(*file) else {
                continue;
            };
            let node = parsed.tree().node(*node_id);
            let mut tokens = Vec::new();
            collect_tokens(&node, &mut tokens);
            let Some(token) = tokens.iter().rev().find(|token| {
                token.kind() == SyntaxKind::Ident && token.text().eq_ignore_ascii_case(&name)
            }) else {
                continue;
            };
            if let Some(location) = self.span_location(token.span()) {
                locations.push(location);
            }
        }
        locations
    }

    fn target_name(&self, target: Target) -> Option<String> {
        self.target_declaration(target).map(|(_, _, name)| name)
    }

    fn declaration_location(&self, target: Target) -> Option<SourceLocation> {
        self.span_location(self.declaration_span(target)?)
    }

    fn declaration_span(&self, target: Target) -> Option<Span> {
        let (file, span, name) = self.target_declaration(target)?;
        if let Some(parsed) = self.parsed(file)
            && let Some((start, end)) = name_token(parsed, span, &name)
        {
            return Some(Span::new(file, start, end));
        }
        (!span.is_empty()).then_some(span)
    }

    fn target_declaration(&self, target: Target) -> Option<(FileId, Span, String)> {
        let hir = self.hir();
        match target {
            Target::Def(def) => {
                let data = &hir.defs[def.index()];
                let name = hir.interner.text(data.name?);
                if name.is_empty() {
                    return None;
                }
                Some((data.span.file, data.span, name.to_owned()))
            }
            Target::Field(field) => {
                let data = hir.field(field);
                Some((
                    data.span.file,
                    data.span,
                    hir.interner.text(data.name).to_owned(),
                ))
            }
            Target::Param(param) => {
                let data = hir.param(param);
                Some((
                    data.span.file,
                    data.span,
                    hir.interner.text(data.name).to_owned(),
                ))
            }
            Target::Local(local) => {
                let data = hir.local(local);
                Some((
                    data.span.file,
                    data.span,
                    hir.interner.text(data.name).to_owned(),
                ))
            }
            Target::EnumValue(def, value) => {
                let data = &hir.defs[def.index()];
                let (DefKind::Enum { values, .. } | DefKind::NamedValue { values, .. }) =
                    &data.kind
                else {
                    return None;
                };
                let entry = values.iter().find(|candidate| candidate.value == value)?;
                Some((
                    entry.span.file,
                    entry.span,
                    hir.interner.text(entry.name).to_owned(),
                ))
            }
            Target::StdFunction(_) | Target::Type(_) => None,
        }
    }

    fn hover_contents(&self, target: Target, display_name: &str) -> Option<String> {
        let hir = self.hir();
        match target {
            Target::Def(def) => {
                let data = &hir.defs[def.index()];
                data.name?;
                if display_name.is_empty() {
                    return None;
                }
                let (declared, kind) = def_declared_type(hir, &data.kind);
                let mut text = match declared {
                    Some(ty) => format!("{display_name} : {ty}"),
                    None => display_name.to_owned(),
                };
                text.push('\n');
                match &data.kind {
                    DefKind::Method { owner, .. } => {
                        let owner_data = &hir.defs[owner.index()];
                        text.push_str(&format!(
                            "{kind} of {} {}",
                            owner_kind(&owner_data.kind),
                            self.definition_spelling(*owner)
                        ));
                    }
                    _ => text.push_str(kind),
                }
                Some(text)
            }
            Target::Field(field) => {
                let data = hir.field(field);
                let owner = &hir.defs[data.owner.index()];
                Some(format!(
                    "{display_name} : {}\nfield of {} {}",
                    hir.type_name(data.ty),
                    owner_kind(&owner.kind),
                    self.definition_spelling(data.owner)
                ))
            }
            Target::Param(param) => {
                let data = hir.param(param);
                let owner = &hir.defs[data.owner.index()];
                Some(format!(
                    "{display_name} : {}\nparameter of {} {}",
                    hir.type_name(data.ty),
                    owner_kind(&owner.kind),
                    self.definition_spelling(data.owner)
                ))
            }
            Target::Local(local) => {
                let data = hir.local(local);
                let owner = &hir.defs[data.owner.index()];
                Some(format!(
                    "{display_name} : {}\nlocal of {} {}",
                    hir.type_name(data.ty),
                    owner_kind(&owner.kind),
                    self.definition_spelling(data.owner)
                ))
            }
            Target::EnumValue(def, value) => {
                let data = &hir.defs[def.index()];
                let (DefKind::Enum { values, .. } | DefKind::NamedValue { values, .. }) =
                    &data.kind
                else {
                    return None;
                };
                values.iter().find(|candidate| candidate.value == value)?;
                let enum_name = self.definition_spelling(def);
                Some(format!(
                    "{display_name} : {enum_name}\nenum value of {enum_name}"
                ))
            }
            Target::StdFunction(id) => {
                let data = &hir.std_fns[id.index()];
                Some(format!(
                    "{}\nstandard function",
                    hir.interner.text(data.name)
                ))
            }
            Target::Type(ty) => Some(format!("{}\ntype", hir.type_name(ty))),
        }
    }
}

/// Returns the identifier token a cursor at `offset` selects.
///
/// The cursor may be inside the identifier or at its end (the position after
/// the last character); a position in trivia or further away selects nothing.
fn cursor_ident(parsed: &ParsedFile, offset: usize) -> Option<SyntaxToken> {
    let tree = parsed.tree();
    let offset = offset.min(tree.source().len());
    if let Some(token) = tree.token_at(offset)
        && token.kind() == SyntaxKind::Ident
    {
        return Some(token);
    }
    // A caret immediately after an identifier (for example between a call's
    // callee and its `(`) sits at the next token's start. F12 there must still
    // name the identifier the caret is at the end of.
    let token = tree.token_before(offset)?;
    (token.kind() == SyntaxKind::Ident && token.span().end as usize == offset).then_some(token)
}

/// Normalizes a resolution to a declaration identity.
fn target_of_res(hir: &HirCrate, resolution: &Res) -> Option<Target> {
    Some(match resolution {
        Res::Def(def) | Res::Namespace(def) => Target::Def(*def),
        Res::Local(local) => Target::Local(*local),
        Res::Param(param) => Target::Param(*param),
        Res::Field(field) => Target::Field(*field),
        Res::EnumValue(def, value) => Target::EnumValue(*def, *value),
        Res::StdFunction(id) => Target::StdFunction(*id),
        Res::Type(ty) => match hir.types.get(*ty) {
            Ty::Named(def) => Target::Def(*def),
            _ => Target::Type(*ty),
        },
        Res::Error => return None,
    })
}

/// Returns whether a resolution names the same declaration as `target`.
fn res_matches_target(hir: &HirCrate, resolution: &Res, target: Target) -> bool {
    match (target, resolution) {
        (Target::Def(a), Res::Def(b) | Res::Namespace(b)) => a == *b,
        (Target::Def(a), Res::Type(ty)) => {
            matches!(hir.types.get(*ty), Ty::Named(def) if *def == a)
        }
        (Target::Field(a), Res::Field(b)) => a == *b,
        (Target::Param(a), Res::Param(b)) => a == *b,
        (Target::Local(a), Res::Local(b)) => a == *b,
        (Target::EnumValue(def, value), Res::EnumValue(other_def, other_value)) => {
            def == *other_def && value == *other_value
        }
        (Target::StdFunction(a), Res::StdFunction(b)) => a == *b,
        (Target::Type(a), Res::Type(b)) => a == *b,
        _ => false,
    }
}

/// Returns the declared type and kind label of a definition.
fn def_declared_type(hir: &HirCrate, kind: &DefKind) -> (Option<String>, &'static str) {
    match kind {
        DefKind::Alias { target } => (Some(hir.type_name(*target)), "type alias"),
        DefKind::Subrange { base, .. } => (Some(hir.type_name(*base)), "subrange type"),
        DefKind::Enum { base, .. } => (Some(hir.type_name(*base)), "enumeration type"),
        DefKind::NamedValue { base, .. } => (Some(hir.type_name(*base)), "named-value type"),
        DefKind::Struct { .. } => (None, "structure type"),
        DefKind::Function { result, .. } => (result.map(|ty| hir.type_name(ty)), "function"),
        DefKind::FunctionBlock { .. } => (None, "function block"),
        DefKind::Program { .. } => (None, "program"),
        DefKind::Class { .. } => (None, "class"),
        DefKind::Interface { .. } => (None, "interface"),
        DefKind::Method { result, .. } => (result.map(|ty| hir.type_name(ty)), "method"),
        DefKind::Global { ty, .. } => (Some(hir.type_name(*ty)), "global variable"),
        DefKind::Namespace { .. } => (None, "namespace"),
        DefKind::Configuration { .. } => (None, "configuration"),
    }
}

/// Returns the owner noun used in hover and completion lines.
fn owner_kind(kind: &DefKind) -> &'static str {
    match kind {
        DefKind::Struct { .. } => "struct",
        DefKind::FunctionBlock { .. } => "function block",
        DefKind::Program { .. } => "program",
        DefKind::Class { .. } => "class",
        DefKind::Interface { .. } => "interface",
        DefKind::Function { .. } => "function",
        DefKind::Method { .. } => "method",
        _ => "declaration",
    }
}

pub(crate) fn collect_tokens(node: &SyntaxNode, out: &mut Vec<SyntaxToken>) {
    for child in node.children() {
        match child {
            SyntaxElement::Node(node) => collect_tokens(&node, out),
            SyntaxElement::Token(token) => out.push(token),
            SyntaxElement::Trivia(_) => {}
        }
    }
}

/// Finds the first identifier token spelling `name` inside `span`.
pub(crate) fn name_token(parsed: &ParsedFile, span: Span, name: &str) -> Option<(u32, u32)> {
    if span.file != parsed.tree().file() {
        return None;
    }
    let mut tokens = Vec::new();
    collect_tokens(&parsed.tree().root(), &mut tokens);
    tokens
        .into_iter()
        .filter(|token| {
            token.kind() == SyntaxKind::Ident
                && token.span().start >= span.start
                && token.span().end <= span.end
                && token.text().eq_ignore_ascii_case(name)
        })
        .map(|token| (token.span().start, token.span().end))
        .next()
}

fn def_id(index: usize) -> DefId {
    DefId::new(u32::try_from(index).expect("definition index fits in u32"))
}

fn field_id(index: usize) -> FieldId {
    FieldId::new(u32::try_from(index).expect("field index fits in u32"))
}

fn param_id(index: usize) -> ParamId {
    ParamId::new(u32::try_from(index).expect("parameter index fits in u32"))
}

fn local_id(index: usize) -> LocalId {
    LocalId::new(u32::try_from(index).expect("local index fits in u32"))
}
