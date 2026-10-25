//! Type resolution, declaration typing, inheritance, and access defaults.

use std::collections::BTreeSet;

use stc_hir::{
    ArgDirection, ArrayBoundTy, ArrayTy, CharTy, ConstValue, DateTy, DefId, DefKind, EnumValueHir,
    FieldHir, FieldId, IntTy, LocalId, RealTy, RetainPolicy, StringKind, StringTy, Symbol, TimeTy,
    Ty, TyId, VarSectionKind, Visibility,
};
use stc_span::Span;
use stc_syntax::SyntaxKind;
use stc_syntax::ast::AstNode;
use stc_syntax::ast::{ArrayBound, NamedValueTypeDecl, StructElem, TypeDecl, TypeRef};

use crate::resolve::Lookup;
use crate::{AliasState, Analyzer, DeclId, TypeSite};

const DEFAULT_STRING_CAPACITY: u32 = 255;
const MAX_STRING_CAPACITY: u32 = 65535;

impl Analyzer<'_> {
    pub(crate) fn resolve_aliases(&mut self) {
        let sites: Vec<TypeSite> = self.type_sites.clone();
        for site in &sites {
            if matches!(&self.hir.defs[site.def.index()].kind, DefKind::Alias { .. }) {
                let _ = self.resolve_alias_def(site.def);
            }
        }
    }

    pub(crate) fn resolve_type_declarations(&mut self) {
        let sites: Vec<TypeSite> = self.type_sites.clone();
        // Enums and named values first so their constants are available to
        // array bounds and subranges.
        for site in &sites {
            if matches!(site.decl, TypeDecl::Enum(_)) {
                self.resolve_enum_site(site);
            }
        }
        for site in &sites {
            if matches!(site.decl, TypeDecl::NamedValue(_)) {
                self.resolve_named_value_site(site);
            }
        }
        for site in &sites {
            if matches!(site.decl, TypeDecl::Simple(_) | TypeDecl::Array(_)) {
                let _ = self.resolve_alias_def(site.def);
            }
        }
        for site in &sites {
            if matches!(site.decl, TypeDecl::Subrange(_)) {
                self.resolve_subrange_site(site);
            }
        }
        for site in &sites {
            if matches!(site.decl, TypeDecl::Struct(_)) {
                self.resolve_struct_site(site);
            }
        }
        for site in &sites {
            if matches!(site.decl, TypeDecl::String(_) | TypeDecl::Ref(_)) {
                let _ = self.resolve_alias_def(site.def);
            }
        }
    }

    pub(crate) fn resolve_alias_def(&mut self, def: DefId) -> TyId {
        match self.alias_state.get(&def).copied() {
            Some(AliasState::Resolved) => {
                return match &self.hir.defs[def.index()].kind {
                    DefKind::Alias { target } => *target,
                    _ => self.hir.types.error(),
                };
            }
            Some(AliasState::Resolving) => {
                let span = self.hir.defs[def.index()].span;
                let name = self.def_name(def);
                self.error(
                    crate::codes::TYPE_INVALID,
                    format!("type alias `{name}` is cyclic"),
                    span,
                );
                return self.hir.types.error();
            }
            _ => {}
        }
        self.alias_state.insert(def, AliasState::Resolving);
        let Some(index) = self.type_site_by_def.get(&def).copied() else {
            self.alias_state.insert(def, AliasState::Resolved);
            return self.hir.types.error();
        };
        let site = self.type_sites[index].clone();
        let ty = self.resolve_alias_site_type(&site);
        if let DefKind::Alias { target } = &mut self.hir.defs[def.index()].kind {
            *target = ty;
        }
        self.alias_state.insert(def, AliasState::Resolved);
        self.resolved_type_defs.insert(def);
        ty
    }

    fn resolve_alias_site_type(&mut self, site: &TypeSite) -> TyId {
        let type_ref = match &site.decl {
            TypeDecl::Simple(node) => node.type_ref(),
            TypeDecl::Array(node) => node.type_ref(),
            TypeDecl::String(node) => node.type_ref(),
            TypeDecl::Ref(node) => node.type_ref(),
            _ => None,
        };
        match type_ref {
            Some(type_ref) => self.resolve_type_ref(&type_ref, site.ns),
            None => {
                let span = crate::type_decl_span(&site.decl);
                self.error(
                    crate::codes::TYPE_INVALID,
                    "type declaration has no resolved shape",
                    span,
                );
                self.hir.types.error()
            }
        }
    }

    /// Resolves a type reference and records the result.
    pub(crate) fn resolve_type_ref(&mut self, type_ref: &TypeRef, ns: DefId) -> TyId {
        let ty = self.resolve_type_ref_inner(type_ref, ns);
        if ty != self.hir.types.error() {
            self.record(type_ref.syntax(), crate::Res::Type(ty));
        }
        ty
    }

    fn resolve_type_ref_inner(&mut self, type_ref: &TypeRef, ns: DefId) -> TyId {
        let Some(first) = type_ref.syntax().first_token() else {
            self.error(
                crate::codes::TYPE_INVALID,
                "missing type",
                type_ref.token_span(),
            );
            return self.hir.types.error();
        };
        let kind = first.kind();
        if let Some(ty) = self.elementary_kind_ty(kind) {
            return ty;
        }
        match kind {
            SyntaxKind::KwString | SyntaxKind::KwWstring | SyntaxKind::KwUstring => {
                self.resolve_string_type(kind, type_ref)
            }
            SyntaxKind::KwChar => self.hir.types.char_ty(CharTy::Char),
            SyntaxKind::KwWchar => self.hir.types.char_ty(CharTy::WChar),
            SyntaxKind::KwUchar => self.hir.types.char_ty(CharTy::UChar),
            SyntaxKind::KwArray => self.resolve_array_type(type_ref, ns),
            SyntaxKind::KwStruct => self.resolve_inline_struct(type_ref, ns),
            SyntaxKind::KwRefTo => {
                let target = match type_ref.element_type() {
                    Some(inner) => self.resolve_type_ref(&inner, ns),
                    None => {
                        self.error(
                            crate::codes::TYPE_INVALID,
                            "`REF_TO` requires a target type",
                            type_ref.token_span(),
                        );
                        self.hir.types.error()
                    }
                };
                self.resolve_reference_target(target, type_ref.token_span())
            }
            SyntaxKind::KwPointer => {
                let target = match type_ref.element_type() {
                    Some(inner) => self.resolve_type_ref(&inner, ns),
                    None => {
                        self.error(
                            crate::codes::TYPE_INVALID,
                            "`POINTER TO` requires a target type",
                            type_ref.token_span(),
                        );
                        self.hir.types.error()
                    }
                };
                self.resolve_pointer_target(target, type_ref.token_span())
            }
            _ => match type_ref.path() {
                Some(path) => match self.resolve_type_path(&path, ns) {
                    Some(def) => self.resolve_named_type_def(def),
                    None => {
                        let names = path.segments();
                        if names.len() == 1 {
                            let symbol = self.hir.intern(names[0].text());
                            if let Some(def) = self.std_fbs.get(&symbol).copied() {
                                return self.hir.named_type(def);
                            }
                        }
                        let text = path.text().to_string();
                        let span = path.token_span();
                        let mut diagnostic = stc_diag::Diagnostic::error(
                            crate::codes::NAME_UNKNOWN,
                            format!("unknown type `{text}`"),
                            span,
                        )
                        .with_note(
                            "type names resolve in the current namespace and its `USING` imports",
                        );
                        if let Some(last) = names.last()
                            && let Some(suggestion) = self.suggest_name(last.text())
                        {
                            diagnostic = diagnostic.with_suggestion(
                                format!("did you mean `{suggestion}`?"),
                                last.token_span(),
                                suggestion,
                            );
                        }
                        self.push_diag(diagnostic);
                        self.hir.types.error()
                    }
                },
                None => {
                    self.error(
                        crate::codes::TYPE_INVALID,
                        "invalid type specification",
                        type_ref.token_span(),
                    );
                    self.hir.types.error()
                }
            },
        }
    }

    pub(crate) fn resolve_named_type_def(&mut self, def: DefId) -> TyId {
        match self.hir.defs[def.index()].kind {
            DefKind::Alias { .. } => self.resolve_alias_def(def),
            DefKind::Namespace { .. } => {
                let span = self.hir.defs[def.index()].span;
                self.error(
                    crate::codes::TYPE_INVALID,
                    "a namespace is not a type",
                    span,
                );
                self.hir.types.error()
            }
            _ => self.hir.named_type(def),
        }
    }

    pub(crate) fn resolve_type_ref_to_def(
        &mut self,
        type_ref: &TypeRef,
        ns: DefId,
    ) -> Option<DefId> {
        let path = type_ref.path()?;
        let def = self.resolve_def_path(&path, ns)?;
        Some(def)
    }

    pub(crate) fn elementary_kind_ty(&self, kind: SyntaxKind) -> Option<TyId> {
        let types = &self.hir.types;
        Some(match kind {
            SyntaxKind::KwBool => types.bool(),
            SyntaxKind::KwSint => types.int(true, 8),
            SyntaxKind::KwInt => types.int(true, 16),
            SyntaxKind::KwDint => types.int(true, 32),
            SyntaxKind::KwLint => types.int(true, 64),
            SyntaxKind::KwUsint => types.int(false, 8),
            SyntaxKind::KwUint => types.int(false, 16),
            SyntaxKind::KwUdint => types.int(false, 32),
            SyntaxKind::KwUlint => types.int(false, 64),
            SyntaxKind::KwByte => types.bit_string(8),
            SyntaxKind::KwWord => types.bit_string(16),
            SyntaxKind::KwDword => types.bit_string(32),
            SyntaxKind::KwLword => types.bit_string(64),
            SyntaxKind::KwReal => types.real(RealTy::Real),
            SyntaxKind::KwLreal => types.real(RealTy::LReal),
            SyntaxKind::KwTime => types.time(TimeTy::Time),
            SyntaxKind::KwLtime => types.time(TimeTy::LTime),
            SyntaxKind::KwDate => types.date(DateTy::Date),
            SyntaxKind::KwLdate => types.date(DateTy::LDate),
            SyntaxKind::KwTimeOfDay | SyntaxKind::KwTod => types.date(DateTy::Tod),
            SyntaxKind::KwLtod | SyntaxKind::KwLtimeOfDay => types.date(DateTy::LTod),
            SyntaxKind::KwDateAndTime | SyntaxKind::KwDt => types.date(DateTy::DateTime),
            SyntaxKind::KwLdt | SyntaxKind::KwLdateAndTime => types.date(DateTy::LDateTime),
            SyntaxKind::KwChar => types.char_ty(CharTy::Char),
            SyntaxKind::KwWchar => types.char_ty(CharTy::WChar),
            SyntaxKind::KwUchar => types.char_ty(CharTy::UChar),
            _ => return None,
        })
    }

    fn resolve_string_type(&mut self, kind: SyntaxKind, type_ref: &TypeRef) -> TyId {
        let string_kind = match kind {
            SyntaxKind::KwString => StringKind::Sbcs,
            SyntaxKind::KwWstring => StringKind::Dbcs,
            SyntaxKind::KwUstring => StringKind::Unicode,
            _ => return self.hir.types.error(),
        };
        let capacity = match type_ref
            .syntax()
            .first_token_of_kind(SyntaxKind::IntLiteral)
        {
            Some(token) => {
                let text = token.text().replace('_', "");
                match text.parse::<u32>() {
                    Ok(0) => {
                        self.error(
                            crate::codes::ARRAY_BOUND,
                            "string capacity must be greater than zero",
                            token.span(),
                        );
                        1
                    }
                    Ok(value) if value > MAX_STRING_CAPACITY => {
                        self.error(
                            crate::codes::ARRAY_BOUND,
                            format!(
                                "string capacity {value} exceeds the profile maximum {MAX_STRING_CAPACITY}"
                            ),
                            token.span(),
                        );
                        MAX_STRING_CAPACITY
                    }
                    Ok(value) => value,
                    Err(_) => {
                        self.error(
                            crate::codes::ARRAY_BOUND,
                            "invalid string capacity",
                            token.span(),
                        );
                        DEFAULT_STRING_CAPACITY
                    }
                }
            }
            None => DEFAULT_STRING_CAPACITY,
        };
        self.hir.types.intern(Ty::String(StringTy {
            kind: string_kind,
            capacity,
        }))
    }

    fn resolve_array_type(&mut self, type_ref: &TypeRef, ns: DefId) -> TyId {
        let element = match type_ref.element_type() {
            Some(element) => self.resolve_type_ref(&element, ns),
            None => {
                self.error(
                    crate::codes::TYPE_INVALID,
                    "`ARRAY` requires an element type",
                    type_ref.token_span(),
                );
                self.hir.types.error()
            }
        };
        let bounds = type_ref.bounds();
        if bounds.is_empty() {
            self.error(
                crate::codes::ARRAY_BOUND,
                "`ARRAY` requires at least one bound",
                type_ref.token_span(),
            );
            return self.hir.types.error();
        }
        let conformant = bounds.iter().any(ArrayBound::is_star);
        let mut resolved = Vec::new();
        for bound in &bounds {
            if bound.is_star() {
                resolved.push(ArrayBoundTy {
                    lower: 0,
                    upper: -1,
                });
                continue;
            }
            let expressions = bound.expressions();
            let (lower, upper) = match expressions.as_slice() {
                [single] => (0, self.eval_const_int(single, ns).unwrap_or(0)),
                [lower, upper] => (
                    self.eval_const_int(lower, ns).unwrap_or(0),
                    self.eval_const_int(upper, ns).unwrap_or(0),
                ),
                _ => {
                    self.error(
                        crate::codes::ARRAY_BOUND,
                        "invalid array bound",
                        bound.token_span(),
                    );
                    (0, 0)
                }
            };
            if lower > upper {
                self.error(
                    crate::codes::ARRAY_BOUND,
                    format!("array lower bound {lower} exceeds upper bound {upper}"),
                    bound.token_span(),
                );
            }
            resolved.push(ArrayBoundTy { lower, upper });
        }
        self.hir.types.intern(Ty::Array(ArrayTy {
            element,
            bounds: resolved,
            conformant,
        }))
    }

    fn resolve_inline_struct(&mut self, type_ref: &TypeRef, ns: DefId) -> TyId {
        let overlap = type_ref.syntax().has_direct_token(SyntaxKind::KwOverlap);
        let def = self.hir.alloc_def(
            DefKind::Struct {
                fields: Vec::new(),
                overlap,
            },
            None,
            type_ref.token_span(),
            Some(ns),
            Visibility::Public,
        );
        let fields = self.resolve_struct_elements(&type_ref.struct_elements(), def, ns);
        if let DefKind::Struct { fields: slot, .. } = &mut self.hir.defs[def.index()].kind {
            *slot = fields.clone();
        }
        self.validate_overlap_fields(def, &fields);
        self.hir.named_type(def)
    }

    fn resolve_struct_elements(
        &mut self,
        elements: &[StructElem],
        owner: DefId,
        ns: DefId,
    ) -> Vec<stc_hir::FieldId> {
        let mut fields: Vec<FieldId> = Vec::new();
        for element in elements {
            let Some(name) = element.name() else {
                continue;
            };
            let span = element.token_span();
            let symbol = self.hir.intern(name.text());
            let ty = match element.type_ref() {
                Some(type_ref) => self.resolve_type_ref(&type_ref, ns),
                None => {
                    self.error(
                        crate::codes::TYPE_INVALID,
                        "struct element has no type",
                        span,
                    );
                    self.hir.types.error()
                }
            };
            if let Some(previous) = fields
                .iter()
                .find(|field| self.hir.fields[field.index()].name == symbol)
            {
                let previous_span = self.hir.fields[previous.index()].span;
                let text = self.hir.interner.text(symbol).to_string();
                self.push_diag(
                    stc_diag::Diagnostic::error(
                        crate::codes::NAME_DUPLICATE,
                        format!("duplicate struct field `{text}`"),
                        span,
                    )
                    .with_label(previous_span, "previous field is here"),
                );
            }
            let at_token = element.at_token();
            let at = at_token.as_ref().and_then(|token| match token.literal() {
                Some(stc_lexer::Literal::Direct(address)) => {
                    self.check_direct_address(address, ty, token.span(), false);
                    Some(crate::collect::direct_address(
                        address.clone(),
                        token.span(),
                    ))
                }
                _ => None,
            });
            let field = self.hir.alloc_field(FieldHir {
                name: symbol,
                ty,
                owner,
                visibility: Visibility::Public,
                section: VarSectionKind::Var,
                retain: RetainPolicy::None,
                constant: false,
                at,
                io: None,
                init: None,
                span,
            });
            if let Some(init) = element.initializer_node() {
                self.field_inits.push((field, ns, init));
            }
            fields.push(field);
        }
        fields
    }

    fn resolve_enum_site(&mut self, site: &TypeSite) {
        if self.resolved_type_defs.contains(&site.def) {
            return;
        }
        let TypeDecl::Enum(node) = &site.decl else {
            return;
        };
        let base = self.hir.types.int(true, 32);
        let named_ty = self.hir.named_type(site.def);
        let span = node.token_span();
        let mut values = Vec::new();
        let mut previous = -1i128;
        for value in node.values() {
            let Some(name) = value.name() else {
                continue;
            };
            let value_span = name.token_span();
            let symbol = self.hir.intern(name.text());
            let number = match value.value() {
                Some(expr) => self.eval_const_int(&expr, site.ns).unwrap_or(previous + 1),
                None => previous.saturating_add(1),
            };
            if !fits_int(number, true, 32) {
                self.error(
                    crate::codes::CONST_FAILURE,
                    format!("enumeration value {number} does not fit `DINT`"),
                    value_span,
                );
            }
            previous = number;
            self.add_constant(
                site.ns,
                symbol,
                crate::ConstBinding {
                    owner: Some(site.def),
                    ty: named_ty,
                    value: ConstValue::Enum {
                        def: site.def,
                        value: number,
                    },
                    span: value_span,
                },
            );
            values.push(EnumValueHir {
                name: symbol,
                value: number,
                span: value_span,
            });
        }
        let default = node.default_value().map(|name| {
            let symbol = self.hir.intern(name.text());
            if values.iter().all(|value| value.name != symbol) {
                self.error(
                    crate::codes::NAME_UNKNOWN,
                    format!("`{}` is not a value of this enumeration", name.text()),
                    name.token_span(),
                );
            }
            symbol
        });
        if let DefKind::Enum {
            base: slot_base,
            values: slot_values,
            default: slot_default,
        } = &mut self.hir.defs[site.def.index()].kind
        {
            *slot_base = base;
            *slot_values = values;
            *slot_default = default;
        }
        let _ = span;
        self.resolved_type_defs.insert(site.def);
    }

    fn resolve_named_value_site(&mut self, site: &TypeSite) {
        if self.resolved_type_defs.contains(&site.def) {
            return;
        }
        let TypeDecl::NamedValue(node) = &site.decl else {
            return;
        };
        let base = self.resolve_named_value_base(node, site.ns);
        let span = node.token_span();
        let mut values = Vec::new();
        let mut previous = -1i128;
        for value in node.values() {
            let Some(name) = value.name() else {
                continue;
            };
            let value_span = name.token_span();
            let symbol = self.hir.intern(name.text());
            let number = match value.value() {
                Some(expr) => self.eval_const_int(&expr, site.ns).unwrap_or(previous + 1),
                None => previous.saturating_add(1),
            };
            previous = number;
            self.add_constant(
                site.ns,
                symbol,
                crate::ConstBinding {
                    owner: Some(site.def),
                    ty: base,
                    value: ConstValue::Int(number),
                    span: value_span,
                },
            );
            values.push(EnumValueHir {
                name: symbol,
                value: number,
                span: value_span,
            });
        }
        let default = node
            .default_value()
            .map(|name| self.hir.intern(name.text()));
        if let DefKind::NamedValue {
            base: slot_base,
            values: slot_values,
            default: slot_default,
        } = &mut self.hir.defs[site.def.index()].kind
        {
            *slot_base = base;
            *slot_values = values;
            *slot_default = default;
        }
        let _ = span;
        self.resolved_type_defs.insert(site.def);
    }

    fn resolve_named_value_base(&mut self, node: &NamedValueTypeDecl, ns: DefId) -> TyId {
        if let Some(token) = node.base_token()
            && let Some(ty) = self.elementary_kind_ty(token.kind())
        {
            if matches!(self.hir.types.get(ty), Ty::Int(_) | Ty::BitString(_)) {
                return ty;
            }
            self.error(
                crate::codes::TYPE_INVALID,
                "named values require an integer or bit-string base type",
                token.span(),
            );
            return self.hir.types.error();
        }
        if let Some(path) = node.base_path() {
            if let Some(def) = self.resolve_type_path(&path, ns) {
                let ty = self.resolve_named_type_def(def);
                if matches!(self.hir.types.get(ty), Ty::Int(_) | Ty::BitString(_)) {
                    return ty;
                }
            }
        }
        self.error(
            crate::codes::TYPE_INVALID,
            "named value type has no integer base type",
            node.token_span(),
        );
        self.hir.types.error()
    }

    fn resolve_subrange_site(&mut self, site: &TypeSite) {
        if self.resolved_type_defs.contains(&site.def) {
            return;
        }
        let TypeDecl::Subrange(node) = &site.decl else {
            return;
        };
        let base = match node
            .base_token()
            .and_then(|token| self.elementary_kind_ty(token.kind()))
        {
            Some(ty) if matches!(self.hir.types.get(ty), Ty::Int(_)) => ty,
            _ => {
                self.error(
                    crate::codes::TYPE_INVALID,
                    "subrange base must be an integer type",
                    node.token_span(),
                );
                self.hir.types.error()
            }
        };
        let (lower_expr, upper_expr) = node.bounds();
        let lower = lower_expr
            .as_ref()
            .and_then(|expr| self.eval_const_int(expr, site.ns));
        let upper = upper_expr
            .as_ref()
            .and_then(|expr| self.eval_const_int(expr, site.ns));
        let (lower, upper) = match (lower, upper) {
            (Some(lower), Some(upper)) => (lower, upper),
            _ => {
                self.error(
                    crate::codes::CONST_NOT_CONSTANT,
                    "subrange bounds must be constant expressions",
                    node.token_span(),
                );
                (0, 0)
            }
        };
        if lower > upper {
            self.error(
                crate::codes::CONST_FAILURE,
                format!("subrange lower bound {lower} exceeds upper bound {upper}"),
                node.token_span(),
            );
        }
        if let Ty::Int(int) = self.hir.types.get(base) {
            if !fits_int(lower, int.signed, int.width) || !fits_int(upper, int.signed, int.width) {
                self.error(
                    crate::codes::TYPE_INVALID,
                    "subrange bounds do not fit the base type",
                    node.token_span(),
                );
            }
        }
        let default = node
            .default_value()
            .and_then(|expr| self.eval_const_int(&expr, site.ns));
        if let DefKind::Subrange {
            base: slot_base,
            lower: slot_lower,
            upper: slot_upper,
            default: slot_default,
        } = &mut self.hir.defs[site.def.index()].kind
        {
            *slot_base = base;
            *slot_lower = lower;
            *slot_upper = upper;
            *slot_default = default;
        }
        self.resolved_type_defs.insert(site.def);
    }

    fn resolve_struct_site(&mut self, site: &TypeSite) {
        if self.resolved_type_defs.contains(&site.def) {
            return;
        }
        let TypeDecl::Struct(node) = &site.decl else {
            return;
        };
        let type_ref = node.type_ref();
        let (elements, overlap) = match type_ref {
            Some(type_ref) => (
                type_ref.struct_elements(),
                type_ref.syntax().has_direct_token(SyntaxKind::KwOverlap),
            ),
            None => (Vec::new(), node.is_overlap()),
        };
        let fields = self.resolve_struct_elements(&elements, site.def, site.ns);
        if let DefKind::Struct {
            fields: slot,
            overlap: slot_overlap,
        } = &mut self.hir.defs[site.def.index()].kind
        {
            *slot = fields.clone();
            *slot_overlap = overlap;
        }
        self.validate_overlap_fields(site.def, &fields);
        self.resolved_type_defs.insert(site.def);
    }

    fn validate_overlap_fields(&mut self, def: DefId, fields: &[FieldId]) {
        let overlap = matches!(
            &self.hir.defs[def.index()].kind,
            DefKind::Struct { overlap: true, .. }
        );
        if !overlap {
            return;
        }
        let mut ranges = Vec::new();
        for field in fields {
            let data = self.hir.field(*field).clone();
            let Some(address) = data.at.as_ref() else {
                continue;
            };
            let Some((area, start, len)) = direct_bit_range(address) else {
                continue;
            };
            ranges.push((*field, area, start, len, data.span));
        }
        for (index, (_, area, start, len, span)) in ranges.iter().enumerate() {
            for (_, other_area, other_start, other_len, other_span) in &ranges[..index] {
                if area != other_area {
                    continue;
                }
                let end = start.saturating_add(*len);
                let other_end = other_start.saturating_add(*other_len);
                let overlaps = *start < other_end && *other_start < end;
                let identical = start == other_start && len == other_len;
                if overlaps && !identical {
                    let message = "overlapping `OVERLAP` fields must share the same `AT` range";
                    self.push_diag(
                        stc_diag::Diagnostic::error(crate::codes::TYPE_INVALID, message, *span)
                            .with_label(*other_span, "conflicting field is here"),
                    );
                }
            }
        }
    }

    pub(crate) fn resolve_pou_declarations(&mut self) {
        self.resolve_decl_types();
        self.resolve_pou_signatures();
        self.check_duplicate_signatures();
        self.check_param_local_duplicates();
        self.check_hierarchy();
    }

    fn resolve_decl_types(&mut self) {
        let sites: Vec<crate::DeclSite> = self.decl_sites.clone();
        for site in &sites {
            let scope = self.decl_scope(site.id, site.ns);
            let ty = if site.edge {
                self.hir.types.bool()
            } else {
                match &site.ty_ref {
                    Some(type_ref) => self.resolve_type_ref(type_ref, scope),
                    None => {
                        self.error(
                            crate::codes::TYPE_INVALID,
                            "variable declaration has no type",
                            site.name_span,
                        );
                        self.hir.types.error()
                    }
                }
            };
            self.set_decl_type(site.id, ty);
            let owner_kind = self.decl_owner_kind(site);
            self.check_decl_rules(site, ty, owner_kind);
        }
    }

    fn decl_scope(&self, id: DeclId, fallback: DefId) -> DefId {
        match id {
            DeclId::Global(_) => fallback,
            DeclId::Field(field) => self.hir.fields[field.index()].owner,
            DeclId::Param(param) => self.hir.params[param.index()].owner,
            DeclId::Local(local) => self.hir.locals[local.index()].owner,
        }
    }

    fn set_decl_type(&mut self, id: DeclId, ty: TyId) {
        match id {
            DeclId::Global(def) => {
                if let DefKind::Global { ty: slot, .. } = &mut self.hir.defs[def.index()].kind {
                    *slot = ty;
                }
            }
            DeclId::Field(field) => self.hir.fields[field.index()].ty = ty,
            DeclId::Param(param) => self.hir.params[param.index()].ty = ty,
            DeclId::Local(local) => self.hir.locals[local.index()].ty = ty,
        }
    }

    pub(crate) fn decl_type(&self, id: DeclId) -> TyId {
        match id {
            DeclId::Global(def) => match &self.hir.defs[def.index()].kind {
                DefKind::Global { ty, .. } => *ty,
                _ => self.hir.types.error(),
            },
            DeclId::Field(field) => self.hir.fields[field.index()].ty,
            DeclId::Param(param) => self.hir.params[param.index()].ty,
            DeclId::Local(local) => self.hir.locals[local.index()].ty,
        }
    }

    fn decl_owner_kind(&self, site: &crate::DeclSite) -> DeclOwner {
        match site.id {
            DeclId::Global(_) => DeclOwner::Global,
            DeclId::Field(field) => {
                match self.hir.defs[self.hir.fields[field.index()].owner.index()].kind {
                    DefKind::Struct { .. } => DeclOwner::Struct,
                    DefKind::FunctionBlock { .. } => DeclOwner::FunctionBlock,
                    DefKind::Class { .. } => DeclOwner::Class,
                    DefKind::Program { .. } => DeclOwner::Program,
                    _ => DeclOwner::Global,
                }
            }
            DeclId::Param(param) => {
                match self.hir.defs[self.hir.params[param.index()].owner.index()].kind {
                    DefKind::Function { .. } => DeclOwner::Function,
                    DefKind::Method { .. } => DeclOwner::Method,
                    _ => DeclOwner::Function,
                }
            }
            DeclId::Local(local) => {
                match self.hir.defs[self.hir.locals[local.index()].owner.index()].kind {
                    DefKind::Function { .. } => DeclOwner::Function,
                    DefKind::Method { .. } => DeclOwner::Method,
                    DefKind::FunctionBlock { .. } => DeclOwner::FunctionBlock,
                    DefKind::Program { .. } => DeclOwner::Program,
                    _ => DeclOwner::Function,
                }
            }
        }
    }

    fn check_decl_rules(&mut self, site: &crate::DeclSite, ty: TyId, owner: DeclOwner) {
        if site.section == VarSectionKind::InOut
            && matches!(site.id, DeclId::Param(_))
            && site.constant
        {
            self.error(
                crate::codes::TYPE_INVALID,
                "`CONSTANT` is not valid on a `VAR_IN_OUT` parameter",
                site.name_span,
            );
        }
        if self.contains_pointer(ty) {
            let persistent = match site.id {
                DeclId::Global(_) => true,
                DeclId::Field(_) => matches!(
                    owner,
                    DeclOwner::Program | DeclOwner::FunctionBlock | DeclOwner::Class
                ),
                DeclId::Param(_) => site.section == VarSectionKind::InOut,
                DeclId::Local(_) => false,
            };
            if persistent {
                self.error(
                    crate::codes::TYPE_INVALID,
                    "pointer values are transient and cannot be stored persistently",
                    site.name_span,
                );
            }
        }
        if site.retain == RetainPolicy::Retain {
            let invalid = matches!(
                site.section,
                VarSectionKind::Temp | VarSectionKind::InOut | VarSectionKind::External
            ) || matches!(owner, DeclOwner::Function | DeclOwner::Method);
            if invalid {
                self.error(
                    crate::codes::RETAIN_INVALID_LOCATION,
                    "`RETAIN` is not valid on this declaration",
                    site.name_span,
                );
            } else if site.at.is_some() {
                self.warning(
                    crate::codes::RETAIN_DIRECT,
                    "`RETAIN` on a direct `AT` variable is ignored",
                    site.name_span,
                );
            } else if !self.is_persistable(ty) {
                self.error(
                    crate::codes::RETAIN_NOT_PERSISTABLE,
                    "retained variables cannot contain references, interfaces, or pointers",
                    site.name_span,
                );
            }
        }
        if let Some(address) = &site.at {
            let span = site.at_span.unwrap_or(site.name_span);
            if site.at.is_some() && !matches!(site.id, DeclId::Global(_) | DeclId::Field(_)) {
                self.error(
                    crate::codes::IO_ADDRESS,
                    "`AT` is only valid on global, field, or instance variables",
                    span,
                );
            } else {
                self.check_direct_address(address, ty, span, false);
            }
        }
        if self.contains_conformant(ty) {
            let allowed = site.section == VarSectionKind::InOut
                || (matches!(site.section, VarSectionKind::Input | VarSectionKind::Output)
                    && owner == DeclOwner::FunctionBlock);
            if !allowed {
                self.error(
                    crate::codes::ARRAY_CONFORMANT,
                    "`ARRAY [*]` is only valid in `VAR_IN_OUT` and FB input/output parameters",
                    site.name_span,
                );
            }
        }
        if site.section == VarSectionKind::External
            && let DeclId::Local(local) = site.id
        {
            let name = self.hir.locals[local.index()].name;
            let scope = self.decl_scope(site.id, site.ns);
            match self.lookup_scope(scope, name) {
                Lookup::Entries(defs) => {
                    let globals: Vec<DefId> = defs
                        .into_iter()
                        .filter(|def| {
                            matches!(&self.hir.defs[def.index()].kind, DefKind::Global { .. })
                        })
                        .collect();
                    if let Some(global) = globals.first() {
                        self.hir.locals[local.index()].external_target = Some(*global);
                        let global_ty = match &self.hir.defs[global.index()].kind {
                            DefKind::Global { ty, .. } => *ty,
                            _ => self.hir.types.error(),
                        };
                        if self.can_assign(ty, global_ty) == Conv::No {
                            let name = self.symbol_text(name).to_string();
                            self.error(
                                crate::codes::TYPE_MISMATCH,
                                format!("`VAR_EXTERNAL {name}` does not match the global's type"),
                                site.name_span,
                            );
                        }
                    } else {
                        self.external_not_found(name, site.name_span);
                    }
                }
                _ => self.external_not_found(name, site.name_span),
            }
        }
    }

    fn external_not_found(&mut self, name: Symbol, span: Span) {
        let text = self.symbol_text(name).to_string();
        self.error(
            crate::codes::NAME_UNKNOWN,
            format!("`VAR_EXTERNAL` name `{text}` has no matching global"),
            span,
        );
    }

    fn resolve_pou_signatures(&mut self) {
        let sites = self.pou_sites.clone();
        for site in &sites {
            self.resolve_pou_signature(site);
        }
        // Inheritance links second, after all signatures exist.
        for site in &sites {
            self.resolve_pou_hierarchy(site);
        }
    }

    fn resolve_pou_signature(&mut self, site: &crate::PouSite) {
        match &site.node {
            crate::PouNode::Function(node) => {
                let result = node.result_type().map(|type_ref| {
                    let ty = self.resolve_type_ref(&type_ref, site.def);
                    if self.contains_conformant(ty) {
                        self.error(
                            crate::codes::ARRAY_CONFORMANT,
                            "a function result cannot be a conformant array",
                            type_ref.token_span(),
                        );
                    }
                    ty
                });
                if let DefKind::Function { result: slot, .. } =
                    &mut self.hir.defs[site.def.index()].kind
                {
                    *slot = result;
                }
            }
            crate::PouNode::Method(node) => {
                let result = node.result_type().map(|type_ref| {
                    let ty = self.resolve_type_ref(&type_ref, site.def);
                    if self.contains_conformant(ty) {
                        self.error(
                            crate::codes::ARRAY_CONFORMANT,
                            "a method result cannot be a conformant array",
                            type_ref.token_span(),
                        );
                    }
                    ty
                });
                if let DefKind::Method { result: slot, .. } =
                    &mut self.hir.defs[site.def.index()].kind
                {
                    *slot = result;
                }
            }
            crate::PouNode::Prototype(node) => {
                let result = node
                    .result_type()
                    .map(|type_ref| self.resolve_type_ref(&type_ref, site.def));
                if let DefKind::Method { result: slot, .. } =
                    &mut self.hir.defs[site.def.index()].kind
                {
                    *slot = result;
                }
            }
            _ => {}
        }
    }

    fn resolve_pou_hierarchy(&mut self, site: &crate::PouSite) {
        match &site.node {
            crate::PouNode::Class(node) => {
                let base = node
                    .extends()
                    .and_then(|type_ref| self.resolve_class_base(&type_ref, site.def, false));
                let interfaces = self.resolve_interface_list(&node.implements(), site.def);
                if let DefKind::Class {
                    base: slot,
                    interfaces: list,
                    ..
                } = &mut self.hir.defs[site.def.index()].kind
                {
                    *slot = base;
                    *list = interfaces;
                }
            }
            crate::PouNode::FunctionBlock(node) => {
                let base = node
                    .extends()
                    .and_then(|type_ref| self.resolve_class_base(&type_ref, site.def, true));
                let interfaces = self.resolve_interface_list(&node.implements(), site.def);
                if let DefKind::FunctionBlock {
                    base: slot,
                    interfaces: list,
                    ..
                } = &mut self.hir.defs[site.def.index()].kind
                {
                    *slot = base;
                    *list = interfaces;
                }
            }
            crate::PouNode::Interface(node) => {
                let extends = self.resolve_interface_list(&node.extends(), site.def);
                if let DefKind::Interface { extends: slot, .. } =
                    &mut self.hir.defs[site.def.index()].kind
                {
                    *slot = extends;
                }
            }
            _ => {}
        }
    }

    fn resolve_class_base(
        &mut self,
        type_ref: &TypeRef,
        ns: DefId,
        allow_fb: bool,
    ) -> Option<DefId> {
        let def = self.resolve_type_ref_to_def(type_ref, ns)?;
        let kind = &self.hir.defs[def.index()].kind;
        let ok = matches!(kind, DefKind::Class { .. })
            || (allow_fb && matches!(kind, DefKind::FunctionBlock { .. }));
        if !ok {
            let span = type_ref.token_span();
            self.error(
                crate::codes::TYPE_INVALID,
                if allow_fb {
                    "`EXTENDS` requires a class or function block"
                } else {
                    "`EXTENDS` requires a class"
                },
                span,
            );
            return None;
        }
        Some(def)
    }

    fn resolve_interface_list(&mut self, paths: &[stc_syntax::ast::Path], ns: DefId) -> Vec<DefId> {
        let mut interfaces = Vec::new();
        for path in paths {
            match self.resolve_def_path(path, ns) {
                Some(def)
                    if matches!(&self.hir.defs[def.index()].kind, DefKind::Interface { .. }) =>
                {
                    if !interfaces.contains(&def) {
                        interfaces.push(def);
                    }
                }
                Some(_) => {
                    let text = path.text().to_string();
                    self.error(
                        crate::codes::TYPE_INVALID,
                        format!("`{text}` is not an interface"),
                        path.token_span(),
                    );
                }
                None => {
                    let text = path.text().to_string();
                    self.error(
                        crate::codes::NAME_UNKNOWN,
                        format!("unknown interface `{text}`"),
                        path.token_span(),
                    );
                }
            }
        }
        interfaces
    }

    fn check_param_local_duplicates(&mut self) {
        let owners: Vec<(DefId, Vec<LocalId>)> = self
            .locals_by_owner
            .iter()
            .map(|(owner, locals)| (*owner, locals.clone()))
            .collect();
        for (owner, locals) in owners {
            let params = match &self.hir.defs[owner.index()].kind {
                DefKind::Function { params, .. } | DefKind::Method { params, .. } => params.clone(),
                _ => continue,
            };
            for local in &locals {
                let name = self.hir.locals[local.index()].name;
                let local_span = self.hir.locals[local.index()].span;
                if let Some(param) = params
                    .iter()
                    .find(|param| self.hir.params[param.index()].name == name)
                {
                    let param_span = self.hir.params[param.index()].span;
                    let text = self.hir.interner.text(name).to_string();
                    self.push_diag(
                        stc_diag::Diagnostic::error(
                            crate::codes::NAME_DUPLICATE,
                            format!("duplicate definition of `{text}`"),
                            local_span,
                        )
                        .with_label(param_span, "parameter declaration is here"),
                    );
                }
            }
        }
    }

    fn check_duplicate_signatures(&mut self) {
        let scopes: Vec<Vec<DefId>> = self
            .scopes
            .values()
            .flat_map(|scope| scope.entries.values().cloned())
            .collect();
        for defs in scopes {
            let functions: Vec<DefId> = defs
                .iter()
                .copied()
                .filter(|def| matches!(&self.hir.defs[def.index()].kind, DefKind::Function { .. }))
                .collect();
            self.check_signature_set(&functions);
        }
        let types: Vec<DefId> = self
            .hir
            .defs
            .iter()
            .enumerate()
            .filter(|(_, def)| {
                matches!(
                    &def.kind,
                    DefKind::Class { .. }
                        | DefKind::FunctionBlock { .. }
                        | DefKind::Interface { .. }
                )
            })
            .map(|(index, _)| DefId::new(index as u32))
            .collect();
        for ty in types {
            let methods = self.hir.defs[ty.index()].method_ids().to_vec();
            self.check_signature_set(&methods);
            let fields = self.hir.defs[ty.index()].field_ids().to_vec();
            for field in &fields {
                let field_name = self.hir.fields[field.index()].name;
                for method in &methods {
                    if self.hir.defs[method.index()].name == Some(field_name) {
                        let method_span = self.hir.defs[method.index()].span;
                        let field_span = self.hir.fields[field.index()].span;
                        let name = self.hir.interner.text(field_name).to_string();
                        self.push_diag(
                            stc_diag::Diagnostic::error(
                                crate::codes::NAME_DUPLICATE,
                                format!("`{name}` is declared as both a field and a method"),
                                method_span,
                            )
                            .with_label(field_span, "field declaration is here"),
                        );
                    }
                }
            }
        }
    }

    fn check_signature_set(&mut self, defs: &[DefId]) {
        for (index, later) in defs.iter().enumerate() {
            for earlier in &defs[..index] {
                if self.hir.defs[earlier.index()].name != self.hir.defs[later.index()].name {
                    continue;
                }
                if self.method_signature(*earlier) == self.method_signature(*later) {
                    let name = self.def_name(*later);
                    let later_span = self.hir.defs[later.index()].span;
                    let earlier_span = self.hir.defs[earlier.index()].span;
                    self.push_diag(
                        stc_diag::Diagnostic::error(
                            crate::codes::NAME_DUPLICATE,
                            format!("duplicate definition of `{name}` with the same signature"),
                            later_span,
                        )
                        .with_label(earlier_span, "previous definition is here"),
                    );
                }
            }
        }
    }

    fn check_hierarchy(&mut self) {
        let defs: Vec<DefId> = self
            .hir
            .defs
            .iter()
            .enumerate()
            .filter(|(_, def)| {
                matches!(
                    &def.kind,
                    DefKind::Class { .. }
                        | DefKind::FunctionBlock { .. }
                        | DefKind::Interface { .. }
                )
            })
            .map(|(index, _)| DefId::new(index as u32))
            .collect();
        for def in &defs {
            self.check_inheritance_cycle(*def);
            self.check_base_final(*def);
        }
        let mut ordered = defs.clone();
        ordered.sort_by_key(|def| self.inheritance_depth(*def));
        for def in &ordered {
            self.assign_vtable(*def);
        }
        for def in &defs {
            self.check_abstract_methods(*def);
            self.check_interface_implementation(*def);
        }
    }

    fn inheritance_depth(&self, def: DefId) -> usize {
        let mut depth = 0;
        let mut current = self.hir.defs[def.index()].base();
        let mut visited = BTreeSet::new();
        while let Some(base) = current {
            if !visited.insert(base) {
                break;
            }
            depth += 1;
            current = self.hir.defs[base.index()].base();
        }
        depth
    }

    fn check_inheritance_cycle(&mut self, def: DefId) {
        let mut visiting = BTreeSet::new();
        let mut done = BTreeSet::new();
        let mut stack = vec![(def, false)];
        while let Some((node, leaving)) = stack.pop() {
            if leaving {
                visiting.remove(&node);
                done.insert(node);
                continue;
            }
            if done.contains(&node) {
                continue;
            }
            if !visiting.insert(node) {
                let span = self.hir.defs[def.index()].span;
                let name = self.def_name(def);
                self.error(
                    crate::codes::OOP_CYCLE,
                    format!("inheritance cycle involving `{name}`"),
                    span,
                );
                return;
            }
            stack.push((node, true));
            let parents: Vec<DefId> = match &self.hir.defs[node.index()].kind {
                DefKind::Class { base, .. } | DefKind::FunctionBlock { base, .. } => {
                    base.iter().copied().collect()
                }
                DefKind::Interface { extends, .. } => extends.clone(),
                _ => Vec::new(),
            };
            for parent in parents {
                if !done.contains(&parent) {
                    stack.push((parent, false));
                }
            }
        }
    }

    fn check_base_final(&mut self, def: DefId) {
        let Some(base) = self.hir.defs[def.index()].base() else {
            return;
        };
        let is_final = match &self.hir.defs[base.index()].kind {
            DefKind::Class { is_final, .. } | DefKind::FunctionBlock { is_final, .. } => *is_final,
            _ => false,
        };
        if is_final {
            let name = self.def_name(def);
            self.error(
                crate::codes::OOP_FINAL,
                format!("`{name}` cannot extend a `FINAL` type"),
                self.hir.defs[def.index()].span,
            );
        }
    }

    fn assign_vtable(&mut self, def: DefId) {
        match &self.hir.defs[def.index()].kind {
            DefKind::Interface { .. } => self.assign_interface_vtable(def),
            DefKind::Class { .. } | DefKind::FunctionBlock { .. } => self.assign_class_vtable(def),
            _ => {}
        }
    }

    fn assign_interface_vtable(&mut self, def: DefId) {
        // Base-first: every interface in the `EXTENDS` closure contributes its
        // own prototypes sorted by (name, signature), and a duplicate is kept
        // only at its earliest occurrence. A derived interface's table is
        // therefore a prefix extension of its base's table (GOT Artifacts
        // §8.5.3).
        let interfaces = self.interface_closure_base_first(&def);
        let mut seen = BTreeSet::new();
        let mut prototypes = Vec::new();
        for interface in interfaces {
            let methods = match &self.hir.defs[interface.index()].kind {
                DefKind::Interface { methods, .. } => methods.clone(),
                _ => continue,
            };
            let mut own = methods;
            own.sort_by_key(|method| {
                let name = self.hir.defs[method.index()]
                    .name_text(&self.hir.interner)
                    .to_owned();
                (name, self.method_signature(*method))
            });
            for method in own {
                let name = self.hir.defs[method.index()].name;
                let key = (name, self.method_signature(method));
                if seen.insert(key) {
                    prototypes.push(method);
                }
            }
        }
        for (index, method) in prototypes.iter().enumerate() {
            if let DefKind::Method { slot, .. } = &mut self.hir.defs[method.index()].kind {
                *slot = Some(index as u32);
            }
        }
        if let DefKind::Interface { vtable, .. } = &mut self.hir.defs[def.index()].kind {
            *vtable = prototypes;
        }
    }

    /// Returns an interface and its `EXTENDS` closure, base interfaces first.
    fn interface_closure_base_first(&self, interface: &DefId) -> Vec<DefId> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        self.visit_interface(*interface, &mut seen, &mut out);
        out
    }

    fn visit_interface(&self, interface: DefId, seen: &mut BTreeSet<DefId>, out: &mut Vec<DefId>) {
        if !seen.insert(interface) {
            return;
        }
        if let DefKind::Interface { extends, .. } = &self.hir.defs[interface.index()].kind {
            for parent in extends {
                self.visit_interface(*parent, seen, out);
            }
        }
        out.push(interface);
    }

    fn assign_class_vtable(&mut self, def: DefId) {
        let base = self.hir.defs[def.index()].base();
        let mut vtable: Vec<DefId> = Vec::new();
        if let Some(base) = base
            && let DefKind::Class {
                vtable: base_vtable,
                ..
            }
            | DefKind::FunctionBlock {
                vtable: base_vtable,
                ..
            } = &self.hir.defs[base.index()].kind
        {
            vtable = base_vtable.clone();
        }
        let methods = match &self.hir.defs[def.index()].kind {
            DefKind::Class { methods, .. } | DefKind::FunctionBlock { methods, .. } => {
                methods.clone()
            }
            _ => return,
        };
        let mut new_methods: Vec<(Option<Symbol>, DefId)> = Vec::new();
        for method in methods {
            let name = self.hir.defs[method.index()].name;
            let signature = self.method_signature(method);
            let is_override = matches!(
                self.hir.defs[method.index()].kind,
                DefKind::Method {
                    is_override: true,
                    ..
                }
            );
            let base_method = name.and_then(|name| {
                vtable.iter().copied().find(|candidate| {
                    self.hir.defs[candidate.index()].name == Some(name)
                        && self.method_signature(*candidate) == signature
                })
            });
            if is_override {
                let Some(base_method) = base_method else {
                    let span = self.hir.defs[method.index()].span;
                    self.error(
                        crate::codes::OOP_OVERRIDE,
                        "`OVERRIDE` has no matching base method",
                        span,
                    );
                    continue;
                };
                let base_final = matches!(
                    self.hir.defs[base_method.index()].kind,
                    DefKind::Method { is_final: true, .. }
                );
                if base_final {
                    let span = self.hir.defs[method.index()].span;
                    self.error(
                        crate::codes::OOP_FINAL,
                        "cannot override a `FINAL` method",
                        span,
                    );
                }
                let slot = match &self.hir.defs[base_method.index()].kind {
                    DefKind::Method { slot, .. } => *slot,
                    _ => None,
                };
                if let Some(index) = vtable.iter().position(|entry| *entry == base_method) {
                    vtable[index] = method;
                }
                if let DefKind::Method {
                    slot: slot_field, ..
                } = &mut self.hir.defs[method.index()].kind
                {
                    *slot_field = slot;
                }
            } else {
                if let Some(existing) = base_method {
                    let span = self.hir.defs[method.index()].span;
                    let base_span = self.hir.defs[existing.index()].span;
                    self.push_diag(
                        stc_diag::Diagnostic::error(
                            crate::codes::OOP_OVERRIDE,
                            "method hides a base method; add `OVERRIDE`",
                            span,
                        )
                        .with_label(base_span, "base method is here"),
                    );
                }
                if base_method.is_none() {
                    new_methods.push((name, method));
                }
            }
        }
        new_methods.sort_by_key(|(_, method)| {
            let name = self.hir.defs[method.index()]
                .name_text(&self.hir.interner)
                .to_owned();
            (name, self.method_signature(*method))
        });
        for (_, method) in new_methods {
            let slot = u32::try_from(vtable.len()).ok();
            vtable.push(method);
            if let DefKind::Method {
                slot: slot_field, ..
            } = &mut self.hir.defs[method.index()].kind
            {
                *slot_field = slot;
            }
        }
        if let DefKind::Class { vtable: slot, .. } | DefKind::FunctionBlock { vtable: slot, .. } =
            &mut self.hir.defs[def.index()].kind
        {
            *slot = vtable;
        }
    }

    fn check_abstract_methods(&mut self, def: DefId) {
        let (is_abstract, methods) = match &self.hir.defs[def.index()].kind {
            DefKind::Class {
                is_abstract,
                methods,
                ..
            }
            | DefKind::FunctionBlock {
                is_abstract,
                methods,
                ..
            } => (*is_abstract, methods.clone()),
            DefKind::Interface { methods, .. } => (true, methods.clone()),
            _ => return,
        };
        for method in methods {
            let (method_abstract, has_body) = match &self.hir.defs[method.index()].kind {
                DefKind::Method {
                    is_abstract, body, ..
                } => (*is_abstract, body.is_some()),
                _ => (false, false),
            };
            if method_abstract && !is_abstract {
                let span = self.hir.defs[method.index()].span;
                self.error(
                    crate::codes::OOP_ABSTRACT,
                    "an `ABSTRACT` method requires an `ABSTRACT` type",
                    span,
                );
            }
            if method_abstract
                && matches!(
                    &self.hir.defs[method.index()].kind,
                    DefKind::Method {
                        is_override: true,
                        ..
                    }
                )
            {
                let span = self.hir.defs[method.index()].span;
                self.error(
                    crate::codes::OOP_ABSTRACT,
                    "a method cannot be both `ABSTRACT` and `OVERRIDE`",
                    span,
                );
            }
            if method_abstract && has_body {
                let span = self.hir.defs[method.index()].span;
                self.error(
                    crate::codes::OOP_ABSTRACT,
                    "an `ABSTRACT` method cannot have a body",
                    span,
                );
            }
        }
        if !is_abstract {
            self.check_unimplemented_abstract(def);
        }
    }

    fn check_unimplemented_abstract(&mut self, def: DefId) {
        let mut pending: Vec<DefId> = Vec::new();
        let mut current = self.hir.defs[def.index()].base();
        let mut visited = BTreeSet::new();
        while let Some(base) = current {
            if !visited.insert(base) {
                break;
            }
            if let DefKind::Class { methods, .. } | DefKind::FunctionBlock { methods, .. } =
                &self.hir.defs[base.index()].kind
            {
                for method in methods {
                    if matches!(
                        self.hir.defs[method.index()].kind,
                        DefKind::Method {
                            is_abstract: true,
                            ..
                        }
                    ) && !self.type_implements_method(def, method)
                    {
                        pending.push(*method);
                    }
                }
            }
            current = self.hir.defs[base.index()].base();
        }
        for method in pending {
            let span = self.hir.defs[def.index()].span;
            self.error(
                crate::codes::OOP_ABSTRACT,
                format!(
                    "concrete type `{}` does not implement abstract method `{}`",
                    self.def_name(def),
                    self.def_name(method)
                ),
                span,
            );
        }
    }

    fn check_interface_implementation(&mut self, def: DefId) {
        let interfaces = match &self.hir.defs[def.index()].kind {
            DefKind::Class { interfaces, .. } | DefKind::FunctionBlock { interfaces, .. } => {
                interfaces.clone()
            }
            _ => return,
        };
        let is_abstract = matches!(
            self.hir.defs[def.index()].kind,
            DefKind::Class {
                is_abstract: true,
                ..
            } | DefKind::FunctionBlock {
                is_abstract: true,
                ..
            }
        );
        if is_abstract {
            return;
        }
        for interface in interfaces {
            let prototypes = self.interface_closure(&interface);
            for prototype in prototypes {
                if !self.type_implements_method(def, &prototype) {
                    let span = self.hir.defs[def.index()].span;
                    self.error(
                        crate::codes::OOP_INTERFACE,
                        format!(
                            "`{}` does not implement interface method `{}`",
                            self.def_name(def),
                            self.def_name(prototype)
                        ),
                        span,
                    );
                }
            }
        }
    }

    fn interface_closure(&self, interface: &DefId) -> Vec<DefId> {
        let mut out = Vec::new();
        let mut stack = vec![*interface];
        let mut visited = BTreeSet::new();
        while let Some(next) = stack.pop() {
            if !visited.insert(next) {
                continue;
            }
            if let DefKind::Interface {
                methods, extends, ..
            } = &self.hir.defs[next.index()].kind
            {
                for method in methods {
                    if !out.contains(method) {
                        out.push(*method);
                    }
                }
                for parent in extends {
                    stack.push(*parent);
                }
            }
        }
        out
    }

    fn type_implements_method(&self, def: DefId, prototype: &DefId) -> bool {
        let Some(name) = self.hir.defs[prototype.index()].name else {
            return false;
        };
        let signature = self.method_signature(*prototype);
        let mut current = Some(def);
        let mut visited = BTreeSet::new();
        while let Some(next) = current {
            if !visited.insert(next) {
                return false;
            }
            if let DefKind::Class { methods, .. } | DefKind::FunctionBlock { methods, .. } =
                &self.hir.defs[next.index()].kind
            {
                for method in methods {
                    if self.hir.defs[method.index()].name == Some(name)
                        && self.method_signature(*method) == signature
                        && matches!(
                            &self.hir.defs[method.index()].kind,
                            DefKind::Method {
                                is_abstract: false,
                                ..
                            }
                        )
                    {
                        return true;
                    }
                }
            }
            current = self.hir.defs[next.index()].base();
        }
        false
    }

    fn method_signature(&self, method: DefId) -> (Option<TyId>, Vec<(ArgDirection, TyId)>) {
        match &self.hir.defs[method.index()].kind {
            DefKind::Method { result, params, .. } | DefKind::Function { result, params, .. } => (
                *result,
                params
                    .iter()
                    .map(|param| {
                        let param = &self.hir.params[param.index()];
                        (param.direction, param.ty)
                    })
                    .collect(),
            ),
            _ => (None, Vec::new()),
        }
    }

    pub(crate) fn resolve_initializers(&mut self) {
        let type_sites = self.type_sites.clone();
        for site in &type_sites {
            if let Some((initializer, ty)) = self.type_default_initializer(site) {
                let _ = self.lower_initializer(&initializer, ty, site.ns);
            }
        }
        let decls = self.decl_sites.clone();
        for site in &decls {
            let Some(init) = &site.init else {
                continue;
            };
            let ty = self.decl_type(site.id);
            if ty == self.hir.types.error() {
                continue;
            }
            let scope = self.decl_scope(site.id, site.ns);
            let expr = self.lower_initializer(init, ty, scope);
            self.set_decl_initializer(site.id, expr);
        }
        let fields = self.field_inits.clone();
        for (field, ns, init) in fields {
            let ty = self.hir.fields[field.index()].ty;
            if ty == self.hir.types.error() {
                continue;
            }
            let expr = self.lower_initializer(&init, ty, ns);
            self.hir.fields[field.index()].init = expr;
        }
    }

    fn type_default_initializer(
        &mut self,
        site: &crate::TypeSite,
    ) -> Option<(stc_syntax::ast::Initializer, TyId)> {
        let alias_ty = match &self.hir.defs[site.def.index()].kind {
            DefKind::Alias { target } => *target,
            _ => self.hir.types.error(),
        };
        let (initializer, ty) = match &site.decl {
            TypeDecl::Simple(node) => (node.initializer()?, alias_ty),
            TypeDecl::Array(node) => (node.initializer()?, alias_ty),
            TypeDecl::String(node) => (node.initializer()?, alias_ty),
            TypeDecl::Ref(node) => (node.initializer()?, alias_ty),
            TypeDecl::Struct(node) => {
                let ty = self.hir.types.intern(Ty::Named(site.def));
                (node.initializer()?, ty)
            }
            _ => return None,
        };
        Some((initializer, ty))
    }

    fn set_decl_initializer(&mut self, id: DeclId, expr: Option<stc_hir::ExprId>) {
        match id {
            DeclId::Global(def) => {
                if let DefKind::Global { init, .. } = &mut self.hir.defs[def.index()].kind {
                    *init = expr;
                }
            }
            DeclId::Field(field) => self.hir.fields[field.index()].init = expr,
            DeclId::Param(param) => self.hir.params[param.index()].default = expr,
            DeclId::Local(local) => self.hir.locals[local.index()].init = expr,
        }
    }

    pub(crate) fn contains_conformant(&self, ty: TyId) -> bool {
        let mut stack = vec![ty];
        let mut visited = BTreeSet::new();
        while let Some(next) = stack.pop() {
            if !visited.insert(next) {
                continue;
            }
            match self.hir.types.get(next) {
                Ty::Error => {}
                Ty::Array(array) => {
                    if array.conformant {
                        return true;
                    }
                    stack.push(array.element);
                }
                Ty::Ref(reference) => stack.push(reference.target),
                Ty::Ptr(pointer) => stack.push(pointer.target),
                Ty::Named(def) => match &self.hir.defs[def.index()].kind {
                    DefKind::Struct { fields, .. }
                    | DefKind::FunctionBlock { fields, .. }
                    | DefKind::Class { fields, .. }
                    | DefKind::Program { fields, .. } => {
                        for field in fields {
                            stack.push(self.hir.fields[field.index()].ty);
                        }
                    }
                    DefKind::Alias { target } => stack.push(*target),
                    _ => {}
                },
                _ => {}
            }
        }
        false
    }

    /// Validates a `REF_TO T` target and interns the reference type.
    pub(crate) fn resolve_reference_target(&mut self, target: TyId, span: Span) -> TyId {
        if target == self.hir.types.error() {
            return target;
        }
        if matches!(self.hir.types.get(target), Ty::Ref(_)) {
            self.error(
                crate::codes::TYPE_INVALID,
                "`REF_TO` of a reference is not allowed; use a named reference type",
                span,
            );
            return self.hir.types.error();
        }
        if matches!(self.hir.types.get(target), Ty::Ptr(_)) {
            self.error(
                crate::codes::TYPE_INVALID,
                "`REF_TO` of a pointer is not allowed; use a named reference type",
                span,
            );
            return self.hir.types.error();
        }
        self.hir.types.intern(Ty::Ref(stc_hir::RefTy { target }))
    }

    /// Validates a `POINTER TO T` target and interns the pointer type
    /// (D-145).
    pub(crate) fn resolve_pointer_target(&mut self, target: TyId, span: Span) -> TyId {
        if target == self.hir.types.error() {
            return target;
        }
        if matches!(self.hir.types.get(target), Ty::Ref(_)) {
            self.error(
                crate::codes::TYPE_INVALID,
                "`POINTER TO` a reference is not allowed",
                span,
            );
            return self.hir.types.error();
        }
        if let Some(problem) = self.pointer_target_problem(target) {
            self.error(
                crate::codes::TYPE_INVALID,
                format!("`POINTER TO` target must be contiguous storage, found {problem}"),
                span,
            );
            return self.hir.types.error();
        }
        self.hir.types.pointer(target)
    }

    /// Returns a description of why `ty` cannot be a pointer target.
    pub(crate) fn pointer_target_problem(&self, ty: TyId) -> Option<&'static str> {
        let mut stack = vec![ty];
        let mut visited = BTreeSet::new();
        let mut conformant = false;
        let mut object = false;
        while let Some(next) = stack.pop() {
            if !visited.insert(next) {
                continue;
            }
            match self.hir.types.get(next) {
                Ty::Error => {}
                Ty::Ref(_) => return Some("a reference"),
                Ty::Null => return Some("NULL"),
                Ty::Generic(_) => return Some("a generic type"),
                Ty::Array(array) => {
                    if array.conformant {
                        conformant = true;
                    }
                    stack.push(array.element);
                }
                Ty::Named(def) => match &self.hir.defs[def.index()].kind {
                    DefKind::Alias { target } => stack.push(*target),
                    DefKind::Class { .. }
                    | DefKind::Interface { .. }
                    | DefKind::FunctionBlock { .. }
                    | DefKind::Program { .. } => object = true,
                    DefKind::Struct { fields, .. } => {
                        for field in fields {
                            stack.push(self.hir.fields[field.index()].ty);
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        if object {
            Some("an object or interface type")
        } else if conformant {
            Some("a conformant array")
        } else {
            None
        }
    }

    /// Returns `true` when `ty` transitively contains a transient pointer.
    pub(crate) fn contains_pointer(&self, ty: TyId) -> bool {
        let mut stack = vec![ty];
        let mut visited = BTreeSet::new();
        while let Some(next) = stack.pop() {
            if !visited.insert(next) {
                continue;
            }
            match self.hir.types.get(next) {
                Ty::Ptr(_) => return true,
                Ty::Array(array) => stack.push(array.element),
                // A reference value is a descriptor, not raw pointer storage.
                Ty::Ref(_) => {}
                Ty::Named(def) => match &self.hir.defs[def.index()].kind {
                    DefKind::Alias { target } => stack.push(*target),
                    DefKind::Struct { fields, .. }
                    | DefKind::FunctionBlock { fields, .. }
                    | DefKind::Class { fields, .. }
                    | DefKind::Program { fields, .. } => {
                        for field in fields {
                            stack.push(self.hir.fields[field.index()].ty);
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        false
    }

    /// Returns `true` when `ty` transitively contains an object or interface
    /// value (not reached through a pointer).
    pub(crate) fn contains_object(&self, ty: TyId) -> bool {
        let mut stack = vec![ty];
        let mut visited = BTreeSet::new();
        while let Some(next) = stack.pop() {
            if !visited.insert(next) {
                continue;
            }
            match self.hir.types.get(next) {
                Ty::Array(array) => stack.push(array.element),
                Ty::Ref(reference) => stack.push(reference.target),
                Ty::Named(def) => match &self.hir.defs[def.index()].kind {
                    DefKind::Alias { target } => stack.push(*target),
                    DefKind::Class { .. }
                    | DefKind::Interface { .. }
                    | DefKind::FunctionBlock { .. }
                    | DefKind::Program { .. } => return true,
                    DefKind::Struct { fields, .. } => {
                        for field in fields {
                            stack.push(self.hir.fields[field.index()].ty);
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        false
    }

    /// Returns `true` when `ty` contains a class handle (directly or inside
    /// an inline function-block value).
    ///
    /// References and pointers are not followed: a reference value is a
    /// descriptor and a pointer target is not part of the value. Interfaces
    /// are handled by the interface-escape rules.
    pub(crate) fn contains_class_handle(&self, ty: TyId) -> bool {
        let mut stack = vec![ty];
        let mut visited = BTreeSet::new();
        while let Some(next) = stack.pop() {
            if !visited.insert(next) {
                continue;
            }
            match self.hir.types.get(next) {
                Ty::Array(array) => stack.push(array.element),
                Ty::Named(def) => match &self.hir.defs[def.index()].kind {
                    DefKind::Alias { target } => stack.push(*target),
                    DefKind::Class { .. } => return true,
                    DefKind::Struct { fields, .. } => {
                        for field in fields {
                            stack.push(self.hir.fields[field.index()].ty);
                        }
                    }
                    DefKind::FunctionBlock { base, fields, .. } => {
                        for field in fields {
                            stack.push(self.hir.fields[field.index()].ty);
                        }
                        if let Some(base) = base {
                            match &self.hir.defs[base.index()].kind {
                                DefKind::Struct { fields, .. }
                                | DefKind::FunctionBlock { fields, .. }
                                | DefKind::Class { fields, .. } => {
                                    for field in fields {
                                        stack.push(self.hir.fields[field.index()].ty);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        false
    }

    /// Returns `true` when `ty` contains an interface value (directly or
    /// inside an inline aggregate/function-block value).
    ///
    /// References and pointers are not followed: a reference value is a
    /// descriptor and a pointer target is not part of the value.
    pub(crate) fn contains_interface(&self, ty: TyId) -> bool {
        let mut stack = vec![ty];
        let mut visited = BTreeSet::new();
        while let Some(next) = stack.pop() {
            if !visited.insert(next) {
                continue;
            }
            match self.hir.types.get(next) {
                Ty::Array(array) => stack.push(array.element),
                Ty::Named(def) => match &self.hir.defs[def.index()].kind {
                    DefKind::Alias { target } => stack.push(*target),
                    DefKind::Interface { .. } => return true,
                    DefKind::Struct { fields, .. } => {
                        for field in fields {
                            stack.push(self.hir.fields[field.index()].ty);
                        }
                    }
                    DefKind::FunctionBlock { base, fields, .. } => {
                        for field in fields {
                            stack.push(self.hir.fields[field.index()].ty);
                        }
                        if let Some(base) = base {
                            match &self.hir.defs[base.index()].kind {
                                DefKind::Struct { fields, .. }
                                | DefKind::FunctionBlock { fields, .. }
                                | DefKind::Class { fields, .. } => {
                                    for field in fields {
                                        stack.push(self.hir.fields[field.index()].ty);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        false
    }

    /// Returns `true` for a raw pointer type.
    pub(crate) fn is_pointer_ty(&self, ty: TyId) -> bool {
        matches!(self.hir.types.get(ty), Ty::Ptr(_))
    }

    /// Returns `true` when a persistent declaration of `ty` is one root.
    ///
    /// Mirrors the codegen root model (D-131): structs without `OVERLAP`
    /// recurse per field; arrays recurse per element; objects are not roots.
    pub(crate) fn is_single_root_ty(&self, ty: TyId) -> bool {
        let mut current = ty;
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(current) {
                return false;
            }
            match self.hir.types.get(current) {
                Ty::Error
                | Ty::Bool
                | Ty::Int(_)
                | Ty::Real(_)
                | Ty::BitString(_)
                | Ty::Time(_)
                | Ty::Date(_)
                | Ty::Char(_)
                | Ty::String(_)
                | Ty::Ref(_)
                | Ty::Ptr(_)
                | Ty::Null => return true,
                Ty::Array(array) => current = array.element,
                Ty::Named(def) => match &self.hir.defs[def.index()].kind {
                    DefKind::Alias { target } => current = *target,
                    DefKind::Struct { overlap, .. } => return *overlap,
                    DefKind::Class { .. }
                    | DefKind::Interface { .. }
                    | DefKind::FunctionBlock { .. }
                    | DefKind::Program { .. } => return false,
                    _ => return true,
                },
                Ty::Generic(_) => return true,
            }
        }
    }

    pub(crate) fn is_persistable(&self, ty: TyId) -> bool {
        let mut stack = vec![ty];
        let mut visited = BTreeSet::new();
        while let Some(next) = stack.pop() {
            if !visited.insert(next) {
                continue;
            }
            match self.hir.types.get(next) {
                Ty::Error => {}
                Ty::Null | Ty::Ref(_) | Ty::Ptr(_) => return false,
                Ty::Named(def) => match &self.hir.defs[def.index()].kind {
                    DefKind::Interface { .. } => return false,
                    // A class value is a handle, but its owned instance's
                    // fields are ordinary persistent leaves (D-152). A
                    // recursive owned-instance cycle has no finite storage and
                    // is rejected by code generation.
                    DefKind::Class { fields, .. }
                    | DefKind::Struct { fields, .. }
                    | DefKind::FunctionBlock { fields, .. }
                    | DefKind::Program { fields, .. } => {
                        for field in fields {
                            stack.push(self.hir.fields[field.index()].ty);
                        }
                    }
                    DefKind::Alias { target } => stack.push(*target),
                    _ => {}
                },
                Ty::Array(array) => stack.push(array.element),
                _ => {}
            }
        }
        true
    }

    /// Validates an `AT` address against a declared type.
    pub(crate) fn check_direct_address(
        &mut self,
        address: &stc_lexer::DirectVariable,
        declared_ty: TyId,
        span: Span,
        executable: bool,
    ) -> bool {
        if address.wildcard {
            if executable {
                self.error(
                    crate::codes::IO_ADDRESS,
                    "a wildcard `AT` address is not valid in an executable expression",
                    span,
                );
                return false;
            }
            return true;
        }
        let expected = self.expected_direct_size(declared_ty);
        let Some(expected) = expected else {
            self.error(
                crate::codes::IO_ADDRESS,
                format!(
                    "`AT` is not allowed for type {}",
                    self.type_name(declared_ty)
                ),
                span,
            );
            return false;
        };
        let actual = address.size;
        let ok = match actual {
            Some(size) => size_matches(size, expected),
            None => expected == stc_lexer::DirectSize::Bit && address.indices.len() == 2,
        };
        if !ok {
            let expected_letter = direct_size_letter(expected);
            self.push_diag(
                stc_diag::Diagnostic::error(
                    crate::codes::IO_ADDRESS,
                    format!(
                        "`AT` address size does not match declared type {}",
                        self.type_name(declared_ty)
                    ),
                    span,
                )
                .with_note(format!("expected a `{expected_letter}` address")),
            );
            return false;
        }
        if expected == stc_lexer::DirectSize::Bit {
            if address.indices.len() != 2 || address.indices[1] > 7 {
                self.error(
                    crate::codes::IO_ADDRESS,
                    "bit addresses require `<byte>.<bit>` with bit in 0..=7",
                    span,
                );
                return false;
            }
        } else if address.indices.len() != 1 {
            self.error(
                crate::codes::IO_ADDRESS,
                "word-sized addresses require exactly one index",
                span,
            );
            return false;
        }
        true
    }

    pub(crate) fn direct_ty(&mut self, address: &stc_lexer::DirectVariable, span: Span) -> TyId {
        let Some(size) = address.size else {
            self.error(
                crate::codes::IO_ADDRESS,
                "direct-variable expression needs an explicit size letter",
                span,
            );
            return self.hir.types.error();
        };
        match size {
            stc_lexer::DirectSize::Bit => self.hir.types.bool(),
            stc_lexer::DirectSize::Byte => self.hir.types.bit_string(8),
            stc_lexer::DirectSize::Word => self.hir.types.bit_string(16),
            stc_lexer::DirectSize::Dword => self.hir.types.bit_string(32),
            stc_lexer::DirectSize::Lword => self.hir.types.bit_string(64),
        }
    }

    pub(crate) fn expected_direct_size(&self, ty: TyId) -> Option<stc_lexer::DirectSize> {
        match self.hir.types.get(ty) {
            Ty::Bool => Some(stc_lexer::DirectSize::Bit),
            Ty::Int(IntTy { width, .. }) => Some(size_for_width(*width)),
            Ty::BitString(width) => Some(size_for_width(*width)),
            Ty::Real(RealTy::Real) => Some(stc_lexer::DirectSize::Dword),
            Ty::Real(RealTy::LReal) => Some(stc_lexer::DirectSize::Lword),
            Ty::Char(CharTy::Char) => Some(stc_lexer::DirectSize::Byte),
            Ty::Char(CharTy::WChar) => Some(stc_lexer::DirectSize::Word),
            Ty::Char(CharTy::UChar) => Some(stc_lexer::DirectSize::Dword),
            Ty::Time(_) | Ty::Date(_) => Some(stc_lexer::DirectSize::Lword),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeclOwner {
    Global,
    Struct,
    Function,
    Method,
    FunctionBlock,
    Class,
    Program,
}

/// Result of an assignability query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Conv {
    /// Identical types.
    Exact,
    /// An implicit conversion is allowed.
    Implicit,
    /// Only an explicit conversion is allowed.
    Explicit,
    /// No conversion exists.
    No,
}

impl Analyzer<'_> {
    /// Returns whether `src` may be assigned to `dst`.
    pub(crate) fn assignability(&self, dst: TyId, src: TyId) -> Conv {
        if dst == src {
            return Conv::Exact;
        }
        if dst == self.hir.types.error() || src == self.hir.types.error() {
            return Conv::Exact;
        }
        let dst_ty = self.hir.types.get(dst);
        let src_ty = self.hir.types.get(src);
        if matches!(src_ty, Ty::Null) {
            return if self.is_reference_like(dst) || matches!(dst_ty, Ty::Ptr(_)) {
                Conv::Implicit
            } else {
                Conv::No
            };
        }
        // A reference to a class/FB object converts to an interface at run
        // time: the referenced object's descriptor is checked for interface
        // membership, and a null or incompatible object yields the null
        // interface (D-150, Q-152, Q-171).
        if let (Ty::Named(dst_def), Ty::Ref(reference)) = (dst_ty, src_ty)
            && matches!(
                self.hir.defs[dst_def.index()].kind,
                DefKind::Interface { .. }
            )
            && self.is_object_type(reference.target)
        {
            return Conv::Implicit;
        }
        if let Ty::Named(def) = src_ty {
            if let DefKind::Subrange {
                base, lower, upper, ..
            } = &self.hir.defs[def.index()].kind
            {
                if let Ty::Named(dst_def) = dst_ty
                    && let DefKind::Subrange { base: dst_base, .. } =
                        &self.hir.defs[dst_def.index()].kind
                {
                    if self.assignability(*dst_base, *base) != Conv::No {
                        return Conv::Implicit;
                    }
                }
                let _ = (lower, upper);
                return self.assignability(dst, *base);
            }
        }
        if let Ty::Named(dst_def) = dst_ty
            && let DefKind::Subrange { base, .. } = &self.hir.defs[dst_def.index()].kind
        {
            if let Ty::Int(_) = src_ty {
                return Conv::Implicit;
            }
            let _ = base;
        }
        if self.is_class_like(dst) || self.is_class_like(src) {
            return if self.is_subtype(dst, src) {
                Conv::Implicit
            } else {
                Conv::No
            };
        }
        match (dst_ty, src_ty) {
            (Ty::Int(dst), Ty::Int(_)) => {
                if dst.width == 0 {
                    Conv::No
                } else {
                    Conv::Implicit
                }
            }
            (Ty::Real(_), Ty::Int(_)) => Conv::Implicit,
            (Ty::Real(RealTy::LReal), Ty::Real(RealTy::Real)) => Conv::Implicit,
            (Ty::Real(RealTy::Real), Ty::Real(RealTy::LReal)) => Conv::Explicit,
            (Ty::Real(_), Ty::Real(_)) => Conv::Exact,
            (Ty::Int(_), Ty::Real(_)) => Conv::Explicit,
            (Ty::BitString(dst), Ty::BitString(src)) => {
                if dst == src {
                    Conv::Exact
                } else if dst > src {
                    Conv::Implicit
                } else {
                    Conv::Explicit
                }
            }
            (Ty::String(dst), Ty::String(src)) => {
                if dst.kind == src.kind {
                    Conv::Implicit
                } else {
                    Conv::Explicit
                }
            }
            (Ty::Char(dst), Ty::Char(src)) => {
                if dst == src {
                    Conv::Exact
                } else {
                    Conv::Explicit
                }
            }
            (Ty::Time(TimeTy::LTime), Ty::Time(TimeTy::Time)) => Conv::Implicit,
            (Ty::Time(TimeTy::Time), Ty::Time(TimeTy::LTime)) => Conv::Explicit,
            (Ty::Time(_), Ty::Time(_)) => Conv::Exact,
            (Ty::Date(dst), Ty::Date(src)) => {
                if dst == src {
                    Conv::Exact
                } else if date_is_long(*dst) && !date_is_long(*src) {
                    Conv::Implicit
                } else {
                    Conv::Explicit
                }
            }
            (Ty::Ref(dst), Ty::Ref(src)) => {
                if self.assignability(dst.target, src.target) == Conv::No {
                    Conv::No
                } else {
                    Conv::Implicit
                }
            }
            (Ty::Ptr(dst), Ty::Ptr(src)) => {
                if dst.target == src.target {
                    Conv::Exact
                } else {
                    Conv::No
                }
            }
            (Ty::Array(dst), Ty::Array(src)) if dst.conformant => {
                if self.assignability(dst.element, src.element) == Conv::No {
                    Conv::No
                } else {
                    Conv::Implicit
                }
            }
            (Ty::Array(dst), Ty::Array(src)) => {
                if dst == src {
                    Conv::Exact
                } else {
                    Conv::No
                }
            }
            _ => Conv::No,
        }
    }

    /// Returns `true` for a conversion the explicit-conversion functions allow.
    pub(crate) fn can_convert(&self, dst: TyId, src: TyId) -> bool {
        !matches!(self.assignability(dst, src), Conv::No)
            || self.numeric_to_numeric(dst, src)
            || self.int_to_bit(dst, src)
            || self.bit_to_int(dst, src)
            || self.enum_to_base(dst, src)
            || self.base_to_enum(dst, src)
            || self.bool_numeric(dst, src)
    }

    fn bool_numeric(&self, dst: TyId, src: TyId) -> bool {
        matches!(
            (
                self.hir.types.get(self.scalar_base(dst)),
                self.hir.types.get(self.scalar_base(src))
            ),
            (Ty::Int(_) | Ty::BitString(_), Ty::Bool) | (Ty::Bool, Ty::Int(_) | Ty::BitString(_))
        )
    }

    fn numeric_to_numeric(&self, dst: TyId, src: TyId) -> bool {
        let dst = self.scalar_base(dst);
        let src = self.scalar_base(src);
        matches!(
            (self.hir.types.get(dst), self.hir.types.get(src)),
            (Ty::Int(_), Ty::Int(_))
                | (Ty::Real(_), Ty::Real(_))
                | (Ty::Int(_), Ty::Real(_))
                | (Ty::Real(_), Ty::Int(_))
        )
    }

    fn int_to_bit(&self, dst: TyId, src: TyId) -> bool {
        matches!(
            (
                self.hir.types.get(self.scalar_base(dst)),
                self.hir.types.get(self.scalar_base(src))
            ),
            (Ty::BitString(_), Ty::Int(_))
        )
    }

    fn bit_to_int(&self, dst: TyId, src: TyId) -> bool {
        matches!(
            (
                self.hir.types.get(self.scalar_base(dst)),
                self.hir.types.get(self.scalar_base(src))
            ),
            (Ty::Int(_), Ty::BitString(_))
        )
    }

    fn enum_to_base(&self, dst: TyId, src: TyId) -> bool {
        let src_base = self.scalar_base(src);
        src_base != src
            && matches!(
                (self.hir.types.get(dst), self.hir.types.get(src_base)),
                (Ty::Int(_) | Ty::BitString(_), Ty::Int(_) | Ty::BitString(_))
            )
    }

    fn base_to_enum(&self, dst: TyId, src: TyId) -> bool {
        let dst_base = self.scalar_base(dst);
        dst_base != dst
            && matches!(
                (self.hir.types.get(dst_base), self.hir.types.get(src)),
                (Ty::Int(_) | Ty::BitString(_), Ty::Int(_) | Ty::BitString(_))
            )
    }

    /// Follows subranges (but not enums/named values) to an arithmetic type.
    pub(crate) fn arithmetic_base(&self, ty: TyId) -> Option<TyId> {
        let mut current = ty;
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(current) {
                return None;
            }
            match self.hir.types.get(current) {
                Ty::Named(def) => match &self.hir.defs[def.index()].kind {
                    DefKind::Subrange { base, .. } => current = *base,
                    _ => return None,
                },
                _ => return Some(current),
            }
        }
    }

    /// Returns `true` for enum and named-value types.
    pub(crate) fn is_enum_like(&self, ty: TyId) -> bool {
        matches!(
            self.hir.types.get(ty),
            Ty::Named(def)
                if matches!(
                    &self.hir.defs[def.index()].kind,
                    DefKind::Enum { .. } | DefKind::NamedValue { .. }
                )
        )
    }

    /// Follows subrange/enum/named-value definitions to their base type.
    pub(crate) fn scalar_base(&self, ty: TyId) -> TyId {
        let mut current = ty;
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(current) {
                return ty;
            }
            let Ty::Named(def) = self.hir.types.get(current) else {
                return current;
            };
            let base = match &self.hir.defs[def.index()].kind {
                DefKind::Subrange { base, .. }
                | DefKind::Enum { base, .. }
                | DefKind::NamedValue { base, .. } => *base,
                _ => return current,
            };
            current = base;
        }
    }

    pub(crate) fn can_assign(&self, dst: TyId, src: TyId) -> Conv {
        self.assignability(dst, src)
    }

    /// Returns `true` for class/interface references, `REF_TO`, and `NULL`.
    pub(crate) fn is_reference_like(&self, ty: TyId) -> bool {
        match self.hir.types.get(ty) {
            Ty::Ref(_) | Ty::Null => true,
            Ty::Named(def) => matches!(
                &self.hir.defs[def.index()].kind,
                DefKind::Class { .. } | DefKind::Interface { .. }
            ),
            _ => false,
        }
    }

    /// Returns `true` for class, FB, and interface types.
    pub(crate) fn is_class_like(&self, ty: TyId) -> bool {
        match self.hir.types.get(ty) {
            Ty::Named(def) => matches!(
                self.hir.defs[def.index()].kind,
                DefKind::Class { .. } | DefKind::Interface { .. } | DefKind::FunctionBlock { .. }
            ),
            _ => false,
        }
    }

    /// Returns `true` for class and function-block object types.
    pub(crate) fn is_object_type(&self, ty: TyId) -> bool {
        matches!(
            self.hir.types.get(ty),
            Ty::Named(def)
                if matches!(
                    self.hir.defs[def.index()].kind,
                    DefKind::Class { .. } | DefKind::FunctionBlock { .. }
                )
        )
    }

    /// Returns `true` for a `REF_TO` value whose target is a class or FB.
    pub(crate) fn is_object_reference(&self, ty: TyId) -> bool {
        match self.hir.types.get(ty) {
            Ty::Ref(reference) => self.is_object_type(reference.target),
            _ => false,
        }
    }

    pub(crate) fn is_subtype(&self, dst: TyId, src: TyId) -> bool {
        if dst == src {
            return true;
        }
        let (Ty::Named(dst_def), Ty::Named(src_def)) =
            (self.hir.types.get(dst), self.hir.types.get(src))
        else {
            return false;
        };
        match &self.hir.defs[dst_def.index()].kind {
            DefKind::Interface { .. } => self.implements_interface(*src_def, *dst_def),
            DefKind::Class { .. } | DefKind::FunctionBlock { .. } => {
                let mut current = Some(*src_def);
                let mut visited = BTreeSet::new();
                while let Some(next) = current {
                    if next == *dst_def {
                        return true;
                    }
                    if !visited.insert(next) {
                        return false;
                    }
                    current = self.hir.defs[next.index()].base();
                }
                false
            }
            _ => false,
        }
    }

    fn implements_interface(&self, def: DefId, interface: DefId) -> bool {
        let mut visited = BTreeSet::new();
        let mut stack = vec![def];
        while let Some(next) = stack.pop() {
            if !visited.insert(next) {
                continue;
            }
            match &self.hir.defs[next.index()].kind {
                DefKind::Class { interfaces, .. } | DefKind::FunctionBlock { interfaces, .. } => {
                    for candidate in interfaces {
                        if *candidate == interface {
                            return true;
                        }
                        stack.push(*candidate);
                    }
                }
                DefKind::Interface { extends, .. } => {
                    for candidate in extends {
                        if *candidate == interface {
                            return true;
                        }
                        stack.push(*candidate);
                    }
                }
                _ => {}
            }
            if let Some(base) = self.hir.defs[next.index()].base() {
                stack.push(base);
            }
        }
        false
    }
}

fn direct_bit_range(address: &stc_hir::DirectAddress) -> Option<(stc_hir::DirectArea, u64, u64)> {
    if address.wildcard {
        return None;
    }
    let area = address.area?;
    let (start, len) = match address.size {
        Some(stc_hir::DirectSize::Bit) => {
            let byte = *address.indices.first()? as u64;
            let bit = *address.indices.get(1)? as u64;
            (byte * 8 + bit, 1)
        }
        Some(stc_hir::DirectSize::Byte) => (*address.indices.first()? as u64 * 8, 8),
        Some(stc_hir::DirectSize::Word) => (*address.indices.first()? as u64 * 16, 16),
        Some(stc_hir::DirectSize::DWord) => (*address.indices.first()? as u64 * 32, 32),
        Some(stc_hir::DirectSize::LWord) => (*address.indices.first()? as u64 * 64, 64),
        None => return None,
    };
    Some((area, start, len))
}

fn size_for_width(width: u8) -> stc_lexer::DirectSize {
    match width {
        8 => stc_lexer::DirectSize::Byte,
        16 => stc_lexer::DirectSize::Word,
        32 => stc_lexer::DirectSize::Dword,
        _ => stc_lexer::DirectSize::Lword,
    }
}

fn size_matches(actual: stc_lexer::DirectSize, expected: stc_lexer::DirectSize) -> bool {
    actual == expected
}

fn direct_size_letter(size: stc_lexer::DirectSize) -> char {
    size.as_char()
}

fn date_is_long(date: DateTy) -> bool {
    matches!(date, DateTy::LDate | DateTy::LTod | DateTy::LDateTime)
}

pub(crate) fn fits_int(value: i128, signed: bool, width: u8) -> bool {
    let bits = u32::from(width);
    if signed {
        let min = -(1i128 << (bits - 1));
        let max = (1i128 << (bits - 1)) - 1;
        (min..=max).contains(&value)
    } else {
        let max = (1i128 << bits) - 1;
        (0..=max).contains(&value)
    }
}
