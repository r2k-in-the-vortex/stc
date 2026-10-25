//! Scope lookup, `USING` resolution, and qualified-name helpers.

use std::collections::BTreeSet;

use stc_hir::{DefId, DefKind, Symbol};
use stc_span::Span;
use stc_syntax::ast::AstNode;
use stc_syntax::ast::Path;

use crate::{Analyzer, ConstBinding, UsingSite};

/// One lookup result.
#[derive(Clone, Debug)]
pub(crate) enum Lookup {
    /// Definitions with the requested name (overload sets keep all entries).
    Entries(Vec<DefId>),
    /// A named constant.
    Constant(ConstBinding),
    /// More than one import supplies the name.
    Ambiguous(Vec<Span>),
    /// No definition was found.
    NotFound,
}

impl Analyzer<'_> {
    pub(crate) fn resolve_using_directives(&mut self) {
        let sites: Vec<(DefId, UsingSite)> = self
            .scopes
            .iter()
            .flat_map(|(scope, data)| data.using_paths.iter().cloned().map(|site| (*scope, site)))
            .collect();
        for (scope, site) in sites {
            match self.resolve_namespace_path(&site.path, site.ns) {
                Some(namespace) if is_namespace(&self.hir.defs[namespace.index()].kind) => {
                    let using = &mut self.scopes.entry(scope).or_default().using;
                    if !using.contains(&namespace) {
                        using.push(namespace);
                    }
                }
                Some(_) => {
                    self.error(
                        crate::codes::TYPE_INVALID,
                        "`USING` requires a namespace name",
                        site.span,
                    );
                }
                None => {
                    let text = site.path.text().to_string();
                    self.error(
                        crate::codes::NAME_UNKNOWN,
                        format!("unknown namespace `{text}` in `USING`"),
                        site.span,
                    );
                }
            }
        }
    }

    /// Looks a name up through one scope and its enclosing scopes.
    pub(crate) fn lookup_scope(&mut self, ns: DefId, symbol: Symbol) -> Lookup {
        let mut visited = BTreeSet::new();
        self.lookup_scope_inner(ns, symbol, &mut visited)
    }

    fn lookup_scope_inner(
        &mut self,
        ns: DefId,
        symbol: Symbol,
        visited: &mut BTreeSet<DefId>,
    ) -> Lookup {
        if !visited.insert(ns) {
            return Lookup::NotFound;
        }
        if let Some(scope) = self.scopes.get(&ns) {
            if let Some(entries) = scope.entries.get(&symbol) {
                return Lookup::Entries(entries.clone());
            }
            if let Some(constant) = scope.constants.get(&symbol) {
                return Lookup::Constant(constant.clone());
            }
            let mut hits: Vec<DefId> = Vec::new();
            let mut constant_hits: Vec<ConstBinding> = Vec::new();
            for using in &scope.using {
                if let Some(used) = self.scopes.get(using) {
                    if let Some(entries) = used.entries.get(&symbol) {
                        for entry in entries {
                            if !hits.contains(entry) {
                                hits.push(*entry);
                            }
                        }
                    }
                    if let Some(constant) = used.constants.get(&symbol) {
                        if constant_hits
                            .iter()
                            .all(|existing| existing.span != constant.span)
                        {
                            constant_hits.push(constant.clone());
                        }
                    }
                }
            }
            let total = hits.len() + constant_hits.len();
            if total > 1 {
                let mut spans: Vec<Span> = hits
                    .iter()
                    .map(|def| self.hir.defs[def.index()].span)
                    .collect();
                spans.extend(constant_hits.iter().map(|constant| constant.span));
                spans.sort();
                return Lookup::Ambiguous(spans);
            }
            if let Some(def) = hits.first() {
                return Lookup::Entries(vec![*def]);
            }
            if let Some(constant) = constant_hits.first() {
                return Lookup::Constant(constant.clone());
            }
        }
        if let Some(parent) = self.hir.defs.get(ns.index()).and_then(|def| def.parent) {
            return self.lookup_scope_inner(parent, symbol, visited);
        }
        Lookup::NotFound
    }

    /// Reports an ambiguity diagnostic at `span`.
    pub(crate) fn report_ambiguous(&mut self, name: &str, span: Span, candidates: &[Span]) {
        let mut diagnostic = stc_diag::Diagnostic::error(
            crate::codes::NAME_AMBIGUOUS,
            format!("`{name}` is ambiguous"),
            span,
        );
        for (index, candidate) in candidates.iter().enumerate() {
            diagnostic = diagnostic.with_label(
                *candidate,
                if index == 0 {
                    "candidate defined here"
                } else {
                    "another candidate is here"
                },
            );
        }
        self.push_diag(
            diagnostic.with_note("an explicit `USING` or a qualified name resolves the ambiguity"),
        );
    }

    /// Resolves a namespace path to a namespace definition.
    pub(crate) fn resolve_namespace_path(&mut self, path: &Path, from: DefId) -> Option<DefId> {
        let names = path.segments();
        let first = names.first()?;
        let symbol = self.hir.intern(first.text());
        let lookup = self.lookup_scope(self.root_ns, symbol);
        let mut current = match self.single_namespace(lookup) {
            Some(def) => def,
            None => {
                let lookup = self.lookup_scope(from, symbol);
                self.single_namespace(lookup)?
            }
        };
        for name in names.iter().skip(1) {
            let symbol = self.hir.intern(name.text());
            let lookup = self.lookup_scope(current, symbol);
            current = self.single_namespace(lookup)?;
        }
        Some(current)
    }

    fn single_namespace(&self, lookup: Lookup) -> Option<DefId> {
        match lookup {
            Lookup::Entries(defs) => {
                let namespaces: Vec<DefId> = defs
                    .into_iter()
                    .filter(|def| is_namespace(&self.hir.defs[def.index()].kind))
                    .collect();
                (namespaces.len() == 1).then(|| namespaces[0])
            }
            Lookup::Ambiguous(_) => None,
            _ => None,
        }
    }

    /// Resolves a type-name path to a definition.
    pub(crate) fn resolve_type_path(&mut self, path: &Path, ns: DefId) -> Option<DefId> {
        let segments = path.segments();
        self.resolve_type_segments(&segments, ns, path.token_span(), &path.text().to_string())
    }

    /// Resolves a type name written as a `PathExpr` (a `SIZEOF` operand).
    pub(crate) fn resolve_type_path_expr(
        &mut self,
        path: &stc_syntax::ast::PathExpr,
        ns: DefId,
    ) -> Option<DefId> {
        let segments = path.segments();
        if path.prefix().is_some() {
            return None;
        }
        self.resolve_type_segments(&segments, ns, path.token_span(), &path.text().to_string())
    }

    fn resolve_type_segments(
        &mut self,
        names: &[stc_syntax::ast::Name],
        ns: DefId,
        span: Span,
        text: &str,
    ) -> Option<DefId> {
        let first = names.first()?;
        let symbol = self.hir.intern(first.text());
        let mut defs = match self.lookup_scope(ns, symbol) {
            Lookup::Entries(defs) => defs,
            Lookup::Constant(_) => return None,
            Lookup::Ambiguous(spans) => {
                self.report_ambiguous(text, span, &spans);
                return None;
            }
            Lookup::NotFound => return None,
        };
        for name in names.iter().skip(1) {
            let namespace = defs
                .iter()
                .copied()
                .find(|def| is_namespace(&self.hir.defs[def.index()].kind))?;
            let symbol = self.hir.intern(name.text());
            defs = match self.lookup_scope(namespace, symbol) {
                Lookup::Entries(defs) => defs,
                Lookup::Constant(_) => return None,
                Lookup::Ambiguous(spans) => {
                    self.report_ambiguous(text, span, &spans);
                    return None;
                }
                Lookup::NotFound => return None,
            };
        }
        let types: Vec<DefId> = defs
            .into_iter()
            .filter(|def| is_type_like(&self.hir.defs[def.index()].kind))
            .collect();
        if types.len() > 1 {
            let spans: Vec<Span> = types
                .iter()
                .map(|def| self.hir.defs[def.index()].span)
                .collect();
            self.report_ambiguous(text, span, &spans);
            return None;
        }
        types.first().copied()
    }

    /// Resolves a definition path to a POU or type definition.
    pub(crate) fn resolve_def_path(&mut self, path: &Path, ns: DefId) -> Option<DefId> {
        let names = path.segments();
        let first = names.first()?;
        let symbol = self.hir.intern(first.text());
        let mut defs = match self.lookup_scope(ns, symbol) {
            Lookup::Entries(defs) => defs,
            Lookup::Constant(_) => return None,
            Lookup::Ambiguous(spans) => {
                let text = path.text().to_string();
                self.report_ambiguous(&text, path.token_span(), &spans);
                return None;
            }
            Lookup::NotFound => return None,
        };
        for name in names.iter().skip(1) {
            let namespace = defs
                .iter()
                .copied()
                .find(|def| is_namespace(&self.hir.defs[def.index()].kind))?;
            let symbol = self.hir.intern(name.text());
            defs = match self.lookup_scope(namespace, symbol) {
                Lookup::Entries(defs) => defs,
                Lookup::Constant(_) => return None,
                Lookup::Ambiguous(spans) => {
                    let text = path.text().to_string();
                    self.report_ambiguous(&text, path.token_span(), &spans);
                    return None;
                }
                Lookup::NotFound => return None,
            };
        }
        if defs.len() == 1 {
            defs.first().copied()
        } else {
            None
        }
    }
}

/// Returns `true` for namespace definitions.
pub(crate) fn is_namespace(kind: &DefKind) -> bool {
    matches!(kind, DefKind::Namespace { .. })
}

/// Returns `true` for definition kinds that can appear in a type position.
pub(crate) fn is_type_like(kind: &DefKind) -> bool {
    matches!(
        kind,
        DefKind::Alias { .. }
            | DefKind::Subrange { .. }
            | DefKind::Enum { .. }
            | DefKind::NamedValue { .. }
            | DefKind::Struct { .. }
            | DefKind::Class { .. }
            | DefKind::Interface { .. }
            | DefKind::FunctionBlock { .. }
            | DefKind::Program { .. }
    )
}
