//! Deterministic collection of namespaces, types, POUs, methods, globals, and
//! configurations (Architecture §3.3, D-072).

use stc_hir::{
    DefId, DefKind, FieldHir, IoDirection, IoHint, LocalHir, ParamHir, RetainPolicy, Symbol,
    VarSectionKind, Visibility,
};
use stc_lexer::{DirectArea, DirectSize, DirectVariable, Literal};
use stc_span::Span;
use stc_syntax::ast::AstNode;
use stc_syntax::ast::{
    ClassDecl, ConfigDecl, DataTypeDecl, EdgeDecl, FunctionBlockDecl, FunctionDecl, InterfaceDecl,
    Item, NamespaceDecl, ProgramDecl, SourceFile, TypeDecl, UsingDirective, VarDecl, VarSection,
};
use stc_syntax::{SyntaxKind, SyntaxToken};

use crate::{Analyzer, ConfigSite, DeclId, DeclSite, PouNode, PouSite, TypeSite, UsingSite};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OwnerKind {
    Function,
    FunctionBlock,
    Program,
    Class,
    Method,
    Interface,
    Configuration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SectionTarget {
    Param,
    Field,
    Local,
    Global,
    Skip,
}

impl Analyzer<'_> {
    pub(crate) fn collect_files(&mut self) {
        for index in 0..self.packages.len() {
            self.current_package =
                stc_hir::PackageId::new(u32::try_from(index).expect("package limit"));
            let files = self.packages[index].files;
            let root_ns = self.root_namespaces[index];
            for file in files {
                self.current_file_test =
                    self.test_build && self.test_file_ids.contains(&file.tree().file());
                let root = file.tree().root();
                if let Some(source) = SourceFile::cast(&root) {
                    let items = source.items();
                    self.collect_items(&items, root_ns);
                }
            }
        }
        self.current_file_test = false;
        self.current_package = self.root_package;
        if self.test_build {
            let test_files = self.test_file_ids.clone();
            for def in &mut self.hir.defs {
                if def.span.start != def.span.end
                    && test_files.contains(&def.span.file)
                    && !matches!(def.kind, DefKind::Namespace { .. })
                {
                    def.test_only = true;
                }
            }
        }
    }

    fn collect_items(&mut self, items: &[Item], ns: DefId) {
        for item in items {
            match item {
                Item::Using(directive) => self.add_using_site(ns, directive),
                Item::Namespace(decl) => self.collect_namespace(decl, ns, &[]),
                Item::DataType(decl) => self.collect_data_type(decl, ns),
                Item::Function(decl) => self.collect_function(decl, ns),
                Item::FunctionBlock(decl) => self.collect_function_block(decl, ns),
                Item::Program(decl) => self.collect_program(decl, ns),
                Item::Class(decl) => self.collect_class(decl, ns),
                Item::Interface(decl) => self.collect_interface(decl, ns),
                Item::Configuration(decl) => self.collect_configuration(decl, ns),
                Item::GlobalVars(section) => {
                    self.collect_var_sections(
                        None,
                        OwnerKind::Configuration,
                        ns,
                        &[section.clone()],
                    );
                }
                Item::Error(_) => {}
            }
        }
    }

    fn collect_namespace(&mut self, decl: &NamespaceDecl, parent: DefId, parent_path: &[Symbol]) {
        let Some(path) = decl.name() else {
            self.error(
                crate::codes::TYPE_INVALID,
                "namespace declaration has no name",
                decl.token_span(),
            );
            return;
        };
        let span = path.token_span();
        let mut current = parent;
        let mut full = parent_path.to_vec();
        let mut segments = Vec::new();
        for name in path.segments() {
            segments.push(self.hir.intern(name.text()));
        }
        for segment in segments {
            full.push(segment);
            current = self.get_or_create_namespace(segment, current, &full, span);
        }
        if decl.is_internal()
            && let DefKind::Namespace { internal } = &mut self.hir.defs[current.index()].kind
        {
            *internal = true;
            self.hir.defs[current.index()].visibility = Visibility::Internal;
        }
        for directive in decl.using_directives() {
            self.add_using_site(current, &directive);
        }
        let elements = decl.elements();
        self.collect_items(&elements, current);
    }

    fn get_or_create_namespace(
        &mut self,
        segment: Symbol,
        parent: DefId,
        full: &[Symbol],
        span: Span,
    ) -> DefId {
        let package = self.current_package.index();
        if let Some(def) = self.namespaces[package].get(full) {
            return *def;
        }
        let def = self.alloc_def(
            DefKind::Namespace { internal: false },
            Some(segment),
            span,
            Some(parent),
            Visibility::Public,
        );
        self.namespaces[package].insert(full.to_vec(), def);
        self.scopes.entry(def).or_default();
        self.add_entry(parent, segment, def, span, false);
        def
    }

    fn add_using_site(&mut self, ns: DefId, directive: &UsingDirective) {
        for path in directive.namespaces() {
            let span = path.token_span();
            self.scopes
                .entry(ns)
                .or_default()
                .using_paths
                .push(UsingSite { ns, path, span });
        }
    }

    fn collect_data_type(&mut self, decl: &DataTypeDecl, ns: DefId) {
        for type_decl in decl.types() {
            self.collect_type_decl(&type_decl, ns);
        }
    }

    fn collect_type_decl(&mut self, decl: &TypeDecl, ns: DefId) {
        let Some(name) = crate::type_decl_name(decl) else {
            return;
        };
        let symbol = self.hir.intern(name.text());
        let span = crate::type_decl_span(decl);
        let kind = match decl {
            TypeDecl::Simple(_) | TypeDecl::Array(_) | TypeDecl::String(_) | TypeDecl::Ref(_) => {
                DefKind::Alias {
                    target: self.hir.types.error(),
                }
            }
            TypeDecl::Subrange(_) => DefKind::Subrange {
                base: self.hir.types.error(),
                lower: 0,
                upper: 0,
                default: None,
            },
            TypeDecl::Enum(_) => DefKind::Enum {
                base: self.hir.types.error(),
                values: Vec::new(),
                default: None,
            },
            TypeDecl::NamedValue(_) => DefKind::NamedValue {
                base: self.hir.types.error(),
                values: Vec::new(),
                default: None,
            },
            TypeDecl::Struct(node) => DefKind::Struct {
                fields: Vec::new(),
                overlap: node.is_overlap(),
            },
            TypeDecl::Error(_) => return,
        };
        let def = self.alloc_def(kind, Some(symbol), span, Some(ns), Visibility::Public);
        self.add_entry(ns, symbol, def, span, false);
        self.type_site_by_def.insert(def, self.type_sites.len());
        self.type_sites.push(TypeSite {
            def,
            ns,
            decl: decl.clone(),
        });
        self.alias_state.insert(def, crate::AliasState::Unresolved);
    }

    fn collect_function(&mut self, decl: &FunctionDecl, ns: DefId) {
        let Some(name) = decl.name() else {
            return;
        };
        let symbol = self.hir.intern(name.text());
        let span = decl.token_span();
        let def = self.alloc_def(
            DefKind::Function {
                result: None,
                params: Vec::new(),
                body: None,
            },
            Some(symbol),
            span,
            Some(ns),
            Visibility::Public,
        );
        self.finish_pou_def(
            def,
            symbol,
            span,
            &PouNode::Function(decl.clone()),
            ns,
            false,
        );
    }

    fn collect_function_block(&mut self, decl: &FunctionBlockDecl, ns: DefId) {
        let Some(name) = decl.name() else {
            return;
        };
        let symbol = self.hir.intern(name.text());
        let span = decl.token_span();
        let def = self.alloc_def(
            DefKind::FunctionBlock {
                base: None,
                interfaces: Vec::new(),
                fields: Vec::new(),
                methods: Vec::new(),
                body: None,
                is_final: decl.is_final(),
                is_abstract: decl.is_abstract(),
                vtable: Vec::new(),
            },
            Some(symbol),
            span,
            Some(ns),
            Visibility::Public,
        );
        self.finish_pou_def(
            def,
            symbol,
            span,
            &PouNode::FunctionBlock(decl.clone()),
            ns,
            false,
        );
    }

    fn collect_program(&mut self, decl: &ProgramDecl, ns: DefId) {
        let Some(name) = decl.name() else {
            return;
        };
        let symbol = self.hir.intern(name.text());
        let span = decl.token_span();
        let def = self.alloc_def(
            DefKind::Program {
                fields: Vec::new(),
                body: None,
                var_access: Vec::new(),
            },
            Some(symbol),
            span,
            Some(ns),
            Visibility::Public,
        );
        self.finish_pou_def(
            def,
            symbol,
            span,
            &PouNode::Program(decl.clone()),
            ns,
            false,
        );
    }

    fn collect_class(&mut self, decl: &ClassDecl, ns: DefId) {
        let Some(name) = decl.name() else {
            return;
        };
        let symbol = self.hir.intern(name.text());
        let span = decl.token_span();
        let def = self.alloc_def(
            DefKind::Class {
                base: None,
                interfaces: Vec::new(),
                fields: Vec::new(),
                methods: Vec::new(),
                is_final: decl.is_final(),
                is_abstract: decl.is_abstract(),
                vtable: Vec::new(),
            },
            Some(symbol),
            span,
            Some(ns),
            Visibility::Public,
        );
        self.finish_pou_def(def, symbol, span, &PouNode::Class(decl.clone()), ns, false);
    }

    fn collect_interface(&mut self, decl: &InterfaceDecl, ns: DefId) {
        let Some(name) = decl.name() else {
            return;
        };
        let symbol = self.hir.intern(name.text());
        let span = decl.token_span();
        let def = self.alloc_def(
            DefKind::Interface {
                extends: Vec::new(),
                methods: Vec::new(),
                vtable: Vec::new(),
            },
            Some(symbol),
            span,
            Some(ns),
            Visibility::Public,
        );
        self.finish_pou_def(
            def,
            symbol,
            span,
            &PouNode::Interface(decl.clone()),
            ns,
            false,
        );
    }

    fn finish_pou_def(
        &mut self,
        def: DefId,
        symbol: Symbol,
        span: Span,
        node: &PouNode,
        ns: DefId,
        is_method: bool,
    ) {
        let mut test_only = self.current_file_test;
        let mut export = false;
        let mut no_mangle = false;
        for pragma in node.pragmas() {
            let Some(annotation) = pragma.annotation() else {
                continue;
            };
            match annotation.name.as_str() {
                "test" => test_only = true,
                "export" => export = true,
                "no_mangle" => no_mangle = true,
                _ => {}
            }
        }
        if crate::prelude::is_standard_function_name(self.hir.interner.text(symbol))
            || (self.stdlib && crate::prelude::is_standard_fb_name(self.hir.interner.text(symbol)))
        {
            let name = self.hir.interner.text(symbol).to_string();
            self.push_diag(
                stc_diag::Diagnostic::warning(
                    crate::codes::NAME_SHADOWS_STANDARD,
                    format!("`{name}` shadows a standard name"),
                    span,
                )
                .with_note("the user declaration takes precedence in this scope"),
            );
        }
        self.hir.defs[def.index()].test_only = test_only;
        self.hir.defs[def.index()].export = export;
        self.hir.defs[def.index()].no_mangle = no_mangle;
        if test_only {
            if !is_method {
                self.test_only.insert((self.current_package, symbol), span);
            }
            // Test-only units are excluded from production analysis (D-071).
            // Dependencies compile as production packages even in a test build;
            // only the root package's test units are analyzed.
            if !(self.test_build && self.current_package == self.root_package) {
                return;
            }
        }
        let allow_overload = matches!(&self.hir.defs[def.index()].kind, DefKind::Function { .. });
        self.add_entry(ns, symbol, def, span, allow_overload);
        for directive in node.using_directives() {
            self.add_using_site(def, &directive);
        }
        let owner_kind = match node {
            PouNode::Function(_) => OwnerKind::Function,
            PouNode::FunctionBlock(_) => OwnerKind::FunctionBlock,
            PouNode::Program(_) => OwnerKind::Program,
            PouNode::Class(_) => OwnerKind::Class,
            PouNode::Interface(_) => OwnerKind::Interface,
            PouNode::Method(_) | PouNode::Prototype(_) => OwnerKind::Method,
        };
        let sections = node.var_sections();
        self.collect_var_sections(Some(def), owner_kind, ns, &sections);
        self.pou_sites.push(PouSite {
            def,
            node: node.clone(),
            owner: None,
        });
        match node {
            PouNode::FunctionBlock(decl) => self.collect_methods(def, ns, &decl.methods()),
            PouNode::Class(decl) => self.collect_methods(def, ns, &decl.methods()),
            PouNode::Interface(decl) => self.collect_prototypes(def, ns, &decl.prototypes()),
            _ => {}
        }
    }

    fn collect_methods(
        &mut self,
        owner: DefId,
        ns: DefId,
        methods: &[stc_syntax::ast::MethodDecl],
    ) {
        for method in methods {
            let Some(name) = method.name() else {
                continue;
            };
            let symbol = self.hir.intern(name.text());
            let span = method.token_span();
            let access = visibility_from_token(method.access().map(|token| token.kind()))
                .unwrap_or(Visibility::Public);
            let def = self.alloc_def(
                DefKind::Method {
                    owner,
                    result: None,
                    params: Vec::new(),
                    body: None,
                    access,
                    is_override: method.is_override(),
                    is_final: method.is_final(),
                    is_abstract: method.is_abstract(),
                    slot: None,
                },
                Some(symbol),
                span,
                Some(owner),
                access,
            );
            self.push_method(owner, def);
            let node = PouNode::Method(method.clone());
            for directive in node.using_directives() {
                self.add_using_site(def, &directive);
            }
            let sections = node.var_sections();
            self.collect_var_sections(Some(def), OwnerKind::Method, ns, &sections);
            self.pou_sites.push(PouSite {
                def,
                node,
                owner: Some(owner),
            });
        }
    }

    fn collect_prototypes(
        &mut self,
        owner: DefId,
        ns: DefId,
        methods: &[stc_syntax::ast::MethodPrototype],
    ) {
        for method in methods {
            let Some(name) = method.name() else {
                continue;
            };
            let symbol = self.hir.intern(name.text());
            let span = method.token_span();
            let access = visibility_from_token(method.access().map(|token| token.kind()))
                .unwrap_or(Visibility::Public);
            let def = self.alloc_def(
                DefKind::Method {
                    owner,
                    result: None,
                    params: Vec::new(),
                    body: None,
                    access,
                    is_override: false,
                    is_final: false,
                    is_abstract: true,
                    slot: None,
                },
                Some(symbol),
                span,
                Some(owner),
                access,
            );
            self.push_method(owner, def);
            let node = PouNode::Prototype(method.clone());
            let sections = node.var_sections();
            self.collect_var_sections(Some(def), OwnerKind::Method, ns, &sections);
            self.pou_sites.push(PouSite {
                def,
                node,
                owner: Some(owner),
            });
        }
    }

    fn push_method(&mut self, owner: DefId, method: DefId) {
        match &mut self.hir.defs[owner.index()].kind {
            DefKind::FunctionBlock { methods, .. }
            | DefKind::Class { methods, .. }
            | DefKind::Interface { methods, .. } => methods.push(method),
            _ => {}
        }
    }

    fn collect_var_sections(
        &mut self,
        owner: Option<DefId>,
        owner_kind: OwnerKind,
        ns: DefId,
        sections: &[VarSection],
    ) {
        for section in sections {
            let kind = section_kind(section);
            let qualifiers = section.qualifiers();
            let constant = qualifiers
                .iter()
                .any(|token| token.kind() == SyntaxKind::KwConstant);
            let retain = retain_policy(&qualifiers);
            let default_visibility = match owner_kind {
                // The FB interface is the callable contract: inputs, outputs,
                // and in-out variables are externally visible like the call
                // bindings that carry them. Internal `VAR` state stays
                // protected. Classes keep the protected default for all
                // variables.
                OwnerKind::FunctionBlock
                    if matches!(
                        kind,
                        VarSectionKind::Input | VarSectionKind::Output | VarSectionKind::InOut
                    ) =>
                {
                    Visibility::Public
                }
                OwnerKind::Class | OwnerKind::FunctionBlock => Visibility::Protected,
                _ => Visibility::Public,
            };
            let visibility = visibility_from_section(&qualifiers).unwrap_or(default_visibility);
            let target = section_target(owner_kind, kind);
            if target == SectionTarget::Skip {
                continue;
            }
            for declaration in section.declarations() {
                match declaration {
                    stc_syntax::ast::VarDeclLike::Var(decl) => self.collect_var_decl(
                        owner, ns, kind, target, visibility, retain, constant, &decl,
                    ),
                    stc_syntax::ast::VarDeclLike::Edge(decl) => self.collect_edge_decl(
                        owner, ns, kind, target, visibility, retain, constant, &decl,
                    ),
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_var_decl(
        &mut self,
        owner: Option<DefId>,
        ns: DefId,
        section: VarSectionKind,
        target: SectionTarget,
        visibility: Visibility,
        retain: RetainPolicy,
        constant: bool,
        decl: &VarDecl,
    ) {
        let ty_ref = decl.type_ref();
        let at_token = decl.at_token();
        let at = at_token.as_ref().and_then(direct_variable);
        let at_span = at_token.as_ref().map(SyntaxToken::span);
        let init = decl.initializer_node();
        let io = io_hint(decl);
        for name in decl.names() {
            self.push_decl(
                owner,
                ns,
                section,
                target,
                visibility,
                retain,
                constant,
                name.text(),
                name.token_span(),
                ty_ref.clone(),
                at.clone(),
                at_span,
                io.clone(),
                init.clone(),
                false,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_edge_decl(
        &mut self,
        owner: Option<DefId>,
        ns: DefId,
        section: VarSectionKind,
        target: SectionTarget,
        visibility: Visibility,
        retain: RetainPolicy,
        constant: bool,
        decl: &EdgeDecl,
    ) {
        for name in decl.names() {
            self.push_decl(
                owner,
                ns,
                section,
                target,
                visibility,
                retain,
                constant,
                name.text(),
                name.token_span(),
                None,
                None,
                None,
                None,
                None,
                true,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_decl(
        &mut self,
        owner: Option<DefId>,
        ns: DefId,
        section: VarSectionKind,
        target: SectionTarget,
        visibility: Visibility,
        retain: RetainPolicy,
        constant: bool,
        name: &str,
        span: Span,
        ty_ref: Option<stc_syntax::ast::TypeRef>,
        at: Option<DirectVariable>,
        at_span: Option<Span>,
        io: Option<IoHint>,
        init: Option<stc_syntax::ast::Initializer>,
        edge: bool,
    ) {
        let symbol = self.hir.intern(name);
        let id = match target {
            SectionTarget::Field => {
                let owner_def = owner.expect("field without owner");
                if let Some(previous) = self.member_span(owner_def, symbol, SectionTarget::Field) {
                    self.duplicate_member(symbol, span, previous);
                }
                let field = self.hir.alloc_field(FieldHir {
                    name: symbol,
                    ty: self.hir.types.error(),
                    owner: owner_def,
                    visibility,
                    section,
                    retain,
                    constant,
                    at: at
                        .clone()
                        .map(|address| crate::collect::direct_address(address, span)),
                    io: io.clone(),
                    init: None,
                    span,
                });
                match &mut self.hir.defs[owner_def.index()].kind {
                    DefKind::Struct { fields, .. }
                    | DefKind::FunctionBlock { fields, .. }
                    | DefKind::Class { fields, .. }
                    | DefKind::Program { fields, .. } => fields.push(field),
                    _ => {}
                }
                DeclId::Field(field)
            }
            SectionTarget::Param => {
                let owner_def = owner.expect("parameter without owner");
                if let Some(previous) = self.member_span(owner_def, symbol, SectionTarget::Param) {
                    self.duplicate_member(symbol, span, previous);
                }
                let direction = section
                    .direction()
                    .expect("parameter section must have a direction");
                let param = self.hir.alloc_param(ParamHir {
                    name: symbol,
                    ty: self.hir.types.error(),
                    direction,
                    owner: owner_def,
                    span,
                    default: None,
                });
                match &mut self.hir.defs[owner_def.index()].kind {
                    DefKind::Function { params, .. } | DefKind::Method { params, .. } => {
                        params.push(param);
                    }
                    _ => {}
                }
                DeclId::Param(param)
            }
            SectionTarget::Local => {
                let owner_def = owner.expect("local without owner");
                if let Some(previous) = self.member_span(owner_def, symbol, SectionTarget::Local) {
                    self.duplicate_member(symbol, span, previous);
                }
                let local = self.hir.alloc_local(LocalHir {
                    name: symbol,
                    ty: self.hir.types.error(),
                    owner: owner_def,
                    is_temp: section == VarSectionKind::Temp,
                    retain,
                    span,
                    init: None,
                    external_target: None,
                });
                self.locals_by_owner
                    .entry(owner_def)
                    .or_default()
                    .push(local);
                DeclId::Local(local)
            }
            SectionTarget::Global => {
                let def = self.alloc_def(
                    DefKind::Global {
                        ty: self.hir.types.error(),
                        section: VarSectionKind::Global,
                        retain,
                        constant,
                        at: at
                            .clone()
                            .map(|address| crate::collect::direct_address(address, span)),
                        io,
                        init: None,
                    },
                    Some(symbol),
                    span,
                    Some(ns),
                    visibility,
                );
                self.add_entry(ns, symbol, def, span, false);
                DeclId::Global(def)
            }
            SectionTarget::Skip => return,
        };
        self.decl_sites.push(DeclSite {
            id,
            ns,
            section,
            retain,
            constant,
            name_span: span,
            ty_ref,
            at,
            at_span,
            init,
            edge,
        });
    }

    fn member_span(&self, owner: DefId, symbol: Symbol, target: SectionTarget) -> Option<Span> {
        match target {
            SectionTarget::Field => self.hir.defs[owner.index()]
                .field_ids()
                .iter()
                .find(|field| self.hir.fields[field.index()].name == symbol)
                .map(|field| self.hir.fields[field.index()].span),
            SectionTarget::Param => match &self.hir.defs[owner.index()].kind {
                DefKind::Function { params, .. } | DefKind::Method { params, .. } => params
                    .iter()
                    .find(|param| self.hir.params[param.index()].name == symbol)
                    .map(|param| self.hir.params[param.index()].span),
                _ => None,
            },
            SectionTarget::Local => self
                .locals_by_owner
                .get(&owner)
                .and_then(|locals| {
                    locals
                        .iter()
                        .find(|local| self.hir.locals[local.index()].name == symbol)
                })
                .map(|local| self.hir.locals[local.index()].span),
            _ => None,
        }
    }

    fn duplicate_member(&mut self, symbol: Symbol, span: Span, previous: Span) {
        let name = self.hir.interner.text(symbol).to_string();
        self.push_diag(
            stc_diag::Diagnostic::error(
                crate::codes::NAME_DUPLICATE,
                format!("duplicate definition of `{name}`"),
                span,
            )
            .with_label(previous, "previous definition is here"),
        );
    }

    fn collect_configuration(&mut self, decl: &ConfigDecl, ns: DefId) {
        let Some(name) = decl.name() else {
            return;
        };
        let symbol = self.hir.intern(name.text());
        let span = decl.token_span();
        let def = self.alloc_def(
            DefKind::Configuration {
                config: Box::new(stc_hir::ConfigurationHir {
                    name: symbol,
                    resources: Vec::new(),
                    var_access: Vec::new(),
                    config_inits: Vec::new(),
                    span,
                }),
            },
            Some(symbol),
            span,
            Some(ns),
            Visibility::Public,
        );
        self.add_entry(ns, symbol, def, span, false);
        self.scopes.entry(def).or_default();
        let package_root = self.root_namespaces[self.current_package.index()];
        if ns != package_root {
            self.error(
                crate::codes::NAME_INVALID_CONTEXT,
                "CONFIGURATION declarations are only allowed at the top level",
                span,
            );
        }
        let sections = decl.var_sections();
        self.collect_var_sections(Some(def), OwnerKind::Configuration, def, &sections);
        for resource in decl.resources() {
            let sections = resource.var_sections();
            self.collect_var_sections(Some(def), OwnerKind::Configuration, def, &sections);
        }
        self.config_sites.push(ConfigSite {
            def,
            ns,
            decl: decl.clone(),
        });
    }
}

/// Extracts the parser-validated `{stc:io}` hint from a variable declaration.
fn io_hint(decl: &VarDecl) -> Option<IoHint> {
    let mut hint = None;
    for pragma in decl.pragmas() {
        let Some(annotation) = pragma.annotation() else {
            continue;
        };
        if annotation.name != "io" {
            continue;
        }
        let object = annotation.object();
        let device = object
            .and_then(|object| object.get("device"))
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        let channel = object
            .and_then(|object| object.get("channel"))
            .and_then(|value| value.as_u64())
            .and_then(|value| u32::try_from(value).ok());
        let direction = object
            .and_then(|object| object.get("direction"))
            .and_then(|value| value.as_str())
            .and_then(|value| match value {
                "input" => Some(IoDirection::Input),
                "output" => Some(IoDirection::Output),
                "inout" => Some(IoDirection::InOut),
                _ => None,
            });
        let payload = annotation
            .payload
            .as_ref()
            .map(|value| value.to_string())
            .unwrap_or_default();
        hint = Some(IoHint {
            device,
            channel,
            direction,
            payload,
            span: annotation.span,
        });
    }
    hint
}

pub(crate) fn direct_address(address: DirectVariable, span: Span) -> stc_hir::DirectAddress {
    stc_hir::DirectAddress {
        area: address.area.map(|area| match area {
            DirectArea::Input => stc_hir::DirectArea::Input,
            DirectArea::Output => stc_hir::DirectArea::Output,
            DirectArea::Memory => stc_hir::DirectArea::Memory,
        }),
        size: address.size.map(|size| match size {
            DirectSize::Bit => stc_hir::DirectSize::Bit,
            DirectSize::Byte => stc_hir::DirectSize::Byte,
            DirectSize::Word => stc_hir::DirectSize::Word,
            DirectSize::Dword => stc_hir::DirectSize::DWord,
            DirectSize::Lword => stc_hir::DirectSize::LWord,
        }),
        wildcard: address.wildcard,
        indices: address.indices,
        span,
    }
}

fn section_kind(section: &VarSection) -> VarSectionKind {
    match section.kind().map(|token| token.kind()) {
        Some(SyntaxKind::KwVarInput) => VarSectionKind::Input,
        Some(SyntaxKind::KwVarOutput) => VarSectionKind::Output,
        Some(SyntaxKind::KwVarInOut) => VarSectionKind::InOut,
        Some(SyntaxKind::KwVarTemp) => VarSectionKind::Temp,
        Some(SyntaxKind::KwVarExternal) => VarSectionKind::External,
        Some(SyntaxKind::KwVarGlobal) => VarSectionKind::Global,
        _ => VarSectionKind::Var,
    }
}

fn section_target(owner: OwnerKind, section: VarSectionKind) -> SectionTarget {
    match owner {
        OwnerKind::Function | OwnerKind::Method => match section {
            VarSectionKind::Input | VarSectionKind::Output | VarSectionKind::InOut => {
                SectionTarget::Param
            }
            VarSectionKind::Temp | VarSectionKind::Var | VarSectionKind::External => {
                SectionTarget::Local
            }
            _ => SectionTarget::Skip,
        },
        OwnerKind::FunctionBlock => match section {
            VarSectionKind::Input
            | VarSectionKind::Output
            | VarSectionKind::InOut
            | VarSectionKind::Var => SectionTarget::Field,
            VarSectionKind::Temp | VarSectionKind::External => SectionTarget::Local,
            _ => SectionTarget::Skip,
        },
        OwnerKind::Class => match section {
            VarSectionKind::Var => SectionTarget::Field,
            _ => SectionTarget::Skip,
        },
        OwnerKind::Program => match section {
            VarSectionKind::Temp | VarSectionKind::External => SectionTarget::Local,
            VarSectionKind::Global => SectionTarget::Skip,
            _ => SectionTarget::Field,
        },
        OwnerKind::Interface => match section {
            VarSectionKind::Input | VarSectionKind::Output | VarSectionKind::InOut => {
                SectionTarget::Param
            }
            _ => SectionTarget::Skip,
        },
        OwnerKind::Configuration => {
            if section == VarSectionKind::Global {
                SectionTarget::Global
            } else {
                SectionTarget::Skip
            }
        }
    }
}

fn visibility_from_section(qualifiers: &[SyntaxToken]) -> Option<Visibility> {
    qualifiers
        .iter()
        .rev()
        .find_map(|token| visibility_from_token(Some(token.kind())))
}

fn visibility_from_token(kind: Option<SyntaxKind>) -> Option<Visibility> {
    match kind? {
        SyntaxKind::KwPublic => Some(Visibility::Public),
        SyntaxKind::KwProtected => Some(Visibility::Protected),
        SyntaxKind::KwPrivate => Some(Visibility::Private),
        SyntaxKind::KwInternal => Some(Visibility::Internal),
        _ => None,
    }
}

fn retain_policy(qualifiers: &[SyntaxToken]) -> RetainPolicy {
    if qualifiers
        .iter()
        .any(|token| token.kind() == SyntaxKind::KwRetain)
    {
        RetainPolicy::Retain
    } else if qualifiers
        .iter()
        .any(|token| token.kind() == SyntaxKind::KwNonRetain)
    {
        RetainPolicy::NonRetain
    } else {
        RetainPolicy::None
    }
}

fn direct_variable(token: &SyntaxToken) -> Option<DirectVariable> {
    match token.literal()? {
        Literal::Direct(address) => Some(address.clone()),
        _ => None,
    }
}
