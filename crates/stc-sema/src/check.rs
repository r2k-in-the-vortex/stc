//! Body resolution, type checking, and HIR lowering (AST §7–§8).

use std::collections::{BTreeMap, BTreeSet};

use stc_diag::Diagnostic;
use stc_hir::{
    ArgDirection, ArgHir, BinOp, Body, CalleeHir, CaseArmHir, CaseLabelHir, CastKind, ConstValue,
    DefId, DefKind, ExprId, ExprKindHir, FieldId, GenericKind, LocalId, ParamId, PlaceHir,
    SizeOfHir, StmtId, StmtKindHir, Symbol, Ty, TyId, UnOp, Visibility,
};
use stc_lexer::Literal;
use stc_span::Span;
use stc_syntax::ast::AstNode;
use stc_syntax::ast::{
    AddressOfExpr, Arg, CallExpr, CaseStmt, Expr, ForStmt, IfStmt, Name, PathExpr,
    PointerConversionExpr, RefValueExpr, RepeatStmt, SizeOfExpr, Stmt, StmtList, UnaryExpr,
    WhileStmt,
};

use crate::prelude::ty_matches_generic;
use crate::resolve::Lookup;
use crate::types::Conv;
use crate::{Analyzer, ConstBinding};

#[derive(Clone, Debug)]
struct BodyCtx {
    def: DefId,
    scope: DefId,
    owner: Option<DefId>,
    locals: BTreeMap<Symbol, LocalId>,
    params: BTreeMap<Symbol, ParamId>,
    result: Option<Symbol>,
    this_ty: Option<TyId>,
    this_expr: Option<ExprId>,
    super_base: Option<DefId>,
    allow_this: bool,
    loop_depth: usize,
    controlled: Vec<PlaceKey>,
}

impl BodyCtx {
    fn namespace(scope: DefId) -> Self {
        Self {
            def: scope,
            scope,
            owner: None,
            locals: BTreeMap::new(),
            params: BTreeMap::new(),
            result: None,
            this_ty: None,
            this_expr: None,
            super_base: None,
            allow_this: false,
            loop_depth: 0,
            controlled: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlaceKey {
    Local(LocalId),
    Param(ParamId),
    Global(DefId),
    Result(DefId),
    Field(FieldId),
}

#[derive(Clone, Debug)]
struct FormalInfo {
    param: Option<ParamId>,
    field: Option<FieldId>,
    name: Symbol,
    ty: TyId,
    direction: ArgDirection,
    required: bool,
}

#[derive(Clone, Debug)]
enum CalleeKind {
    Functions(Vec<DefId>),
    Standard(Vec<stc_hir::StdFnId>),
    Conversion {
        from: TyId,
        to: TyId,
    },
    Fb {
        instance: ExprId,
        def: DefId,
    },
    Method {
        receiver: ExprId,
        methods: Vec<DefId>,
        interface: Option<DefId>,
        is_super: bool,
    },
    SuperBody {
        base: DefId,
    },
    Invalid,
}

impl Analyzer<'_> {
    pub(crate) fn check_bodies(&mut self) {
        let sites = self.pou_sites.clone();
        for site in &sites {
            let stmt_list = match &site.node {
                crate::PouNode::Function(node) => node.body(),
                crate::PouNode::FunctionBlock(node) => node.body(),
                crate::PouNode::Program(node) => node.body(),
                crate::PouNode::Method(node) => node.body(),
                _ => None,
            };
            let Some(stmt_list) = stmt_list else {
                continue;
            };
            let mut params = Vec::new();
            let mut locals = Vec::new();
            let result_name = match &self.hir.defs[site.def.index()].kind {
                DefKind::Function {
                    result,
                    params: ids,
                    ..
                }
                | DefKind::Method {
                    result,
                    params: ids,
                    ..
                } => {
                    params = ids.clone();
                    (result.is_some())
                        .then(|| self.hir.defs[site.def.index()].name)
                        .flatten()
                }
                DefKind::FunctionBlock { .. } | DefKind::Program { .. } => None,
                _ => None,
            };
            if let Some(ids) = self.locals_by_owner.get(&site.def) {
                locals = ids.clone();
            }
            let owner = match &site.node {
                crate::PouNode::Method(_) => site.owner,
                crate::PouNode::FunctionBlock(_) | crate::PouNode::Program(_) => Some(site.def),
                _ => None,
            };
            let this_ty = owner.map(|owner| self.hir.types.intern(Ty::Named(owner)));
            let super_base = owner.and_then(|owner| self.hir.defs[owner.index()].base());
            let allow_this = matches!(
                site.node,
                crate::PouNode::Method(_) | crate::PouNode::FunctionBlock(_)
            );
            let mut ctx = BodyCtx {
                def: site.def,
                scope: site.def,
                owner,
                locals: BTreeMap::new(),
                params: BTreeMap::new(),
                result: result_name,
                this_ty,
                this_expr: None,
                super_base,
                allow_this,
                loop_depth: 0,
                controlled: Vec::new(),
            };
            for id in &params {
                let param = &self.hir.params[id.index()];
                ctx.params.insert(param.name, *id);
            }
            for id in &locals {
                let local = &self.hir.locals[id.index()];
                ctx.locals.insert(local.name, *id);
            }
            let stmts = self.check_stmt_list(&mut ctx, &stmt_list);
            let body = Body {
                owner: site.def,
                params,
                locals,
                stmts,
                span: stmt_list.token_span(),
            };
            let body_id = self.hir.alloc_body(body);
            match &mut self.hir.defs[site.def.index()].kind {
                DefKind::Function { body: slot, .. }
                | DefKind::FunctionBlock { body: slot, .. }
                | DefKind::Program { body: slot, .. }
                | DefKind::Method { body: slot, .. } => *slot = Some(body_id),
                _ => {}
            }
            if let crate::PouNode::Program(decl) = &site.node {
                self.check_program_var_access(site, decl);
            }
            let has_result = matches!(
                &self.hir.defs[site.def.index()].kind,
                DefKind::Function {
                    result: Some(_),
                    ..
                } | DefKind::Method {
                    result: Some(_),
                    ..
                }
            );
            if has_result {
                let body = self.hir.bodies[body_id.index()].clone();
                if !self.body_assigns_result(&body, site.def) {
                    let span = self.hir.defs[site.def.index()].span;
                    let name = self.def_name(site.def);
                    self.push_diag(
                        Diagnostic::error(
                            crate::codes::TYPE_MISMATCH,
                            format!("`{name}` never assigns its result"),
                            span,
                        )
                        .with_note(
                            "a function or method with a result must assign to its own name",
                        ),
                    );
                }
            }
        }
    }

    fn body_assigns_result(&self, body: &Body, def: DefId) -> bool {
        let mut pending: Vec<StmtId> = body.stmts.clone();
        while let Some(id) = pending.pop() {
            let stmt = &self.hir.stmts[id.index()];
            match &stmt.kind {
                StmtKindHir::Assign { place, .. }
                | StmtKindHir::RefAssign { place, .. }
                | StmtKindHir::Attempt { place, .. } => {
                    if self.place_roots_result(*place, def) {
                        return true;
                    }
                }
                StmtKindHir::If { arms, else_body } => {
                    for (_, body) in arms {
                        pending.extend(body.iter().copied());
                    }
                    pending.extend(else_body.iter().copied());
                }
                StmtKindHir::Case {
                    arms, else_body, ..
                } => {
                    for arm in arms {
                        pending.extend(arm.body.iter().copied());
                    }
                    pending.extend(else_body.iter().copied());
                }
                StmtKindHir::For { body, .. }
                | StmtKindHir::While { body, .. }
                | StmtKindHir::Repeat { body, .. } => pending.extend(body.iter().copied()),
                StmtKindHir::Call { call } => {
                    if let ExprKindHir::Call { args, .. } = &self.hir.expr(*call).kind
                        && args.iter().any(|arg| {
                            arg.target
                                .is_some_and(|target| self.place_roots_result(target, def))
                        })
                    {
                        return true;
                    }
                }
                StmtKindHir::Return
                | StmtKindHir::Exit
                | StmtKindHir::Continue
                | StmtKindHir::Empty => {}
            }
        }
        false
    }

    fn place_roots_result(&self, expr: ExprId, def: DefId) -> bool {
        match &self.hir.expr(expr).kind {
            ExprKindHir::Place(place) => self.place_roots_result_place(place, def),
            _ => false,
        }
    }

    fn place_roots_result_place(&self, place: &PlaceHir, def: DefId) -> bool {
        match place {
            PlaceHir::Result(candidate) => *candidate == def,
            PlaceHir::Field { base, .. }
            | PlaceHir::Index { base, .. }
            | PlaceHir::Deref(base)
            | PlaceHir::BitField { base, .. } => self.place_roots_result(*base, def),
            _ => false,
        }
    }

    fn check_program_var_access(
        &mut self,
        site: &crate::PouSite,
        decl: &stc_syntax::ast::ProgramDecl,
    ) {
        let Some(section) = decl.var_access() else {
            return;
        };
        let mut entries = Vec::new();
        for access in section.declarations() {
            let span = access.token_span();
            let Some(name) = access.name() else {
                continue;
            };
            let name_symbol = self.hir.intern(name.text());
            let declared = access
                .type_ref()
                .map(|type_ref| self.resolve_type_ref(&type_ref, site.def))
                .unwrap_or_else(|| self.hir.types.error());
            let (place, actual_ty) = if let Some(token) = access.direct_token() {
                let Some(Literal::Direct(address)) = token.literal() else {
                    continue;
                };
                self.check_direct_address(address, declared, token.span(), false);
                let place = self.hir.alloc_expr(
                    ExprKindHir::Place(PlaceHir::Direct(crate::collect::direct_address(
                        address.clone(),
                        token.span(),
                    ))),
                    declared,
                    token.span(),
                );
                (place, declared)
            } else {
                let names = access.path_names();
                if names.is_empty() {
                    self.error(
                        crate::codes::CONFIG_ERROR,
                        "`VAR_ACCESS` path is empty",
                        span,
                    );
                    continue;
                }
                let program_ty = self.hir.types.intern(Ty::Named(site.def));
                let mut expr = self.hir.alloc_expr(
                    ExprKindHir::Place(PlaceHir::This(site.def)),
                    program_ty,
                    span,
                );
                let mut ty = program_ty;
                let mut ok = true;
                for segment in &names {
                    let symbol = self.hir.intern(segment.text());
                    let Some(field) = self.field_lookup(ty, symbol) else {
                        self.member_error(expr, ty, segment, span);
                        ok = false;
                        break;
                    };
                    let data = self.hir.field(field).clone();
                    self.check_field_access(field, Some(site.def), span);
                    expr = self.hir.alloc_expr(
                        ExprKindHir::Place(PlaceHir::Field { base: expr, field }),
                        data.ty,
                        segment.token_span(),
                    );
                    ty = data.ty;
                }
                if !ok {
                    continue;
                }
                (expr, ty)
            };
            if self.contains_pointer(declared) || self.contains_pointer(actual_ty) {
                self.error(
                    crate::codes::TYPE_INVALID,
                    "pointer values cannot be exposed through `VAR_ACCESS`",
                    span,
                );
                continue;
            }
            if declared != self.hir.types.error()
                && actual_ty != self.hir.types.error()
                && self.can_assign(declared, actual_ty) == Conv::No
            {
                self.error(
                    crate::codes::TYPE_MISMATCH,
                    format!(
                        "`VAR_ACCESS` type `{}` does not match the access path type `{}`",
                        self.type_name(declared),
                        self.type_name(actual_ty)
                    ),
                    span,
                );
            }
            entries.push(stc_hir::VarAccessHir {
                name: name_symbol,
                place,
                ty: declared,
                read_only: access.is_read_only(),
                span,
            });
        }
        if let DefKind::Program { var_access, .. } = &mut self.hir.defs[site.def.index()].kind {
            *var_access = entries;
        }
    }

    pub(crate) fn check_expr_in_namespace(
        &mut self,
        expr: &Expr,
        expected: Option<TyId>,
        ns: DefId,
    ) -> Option<ExprId> {
        let mut ctx = BodyCtx::namespace(ns);
        Some(self.check_expr(&mut ctx, expr, expected))
    }

    fn check_stmt_list(&mut self, ctx: &mut BodyCtx, list: &StmtList) -> Vec<StmtId> {
        let mut out = Vec::new();
        for stmt in list.statements() {
            let id = self.check_stmt(ctx, &stmt);
            out.push(id);
        }
        out
    }

    fn check_stmt(&mut self, ctx: &mut BodyCtx, stmt: &Stmt) -> StmtId {
        let span = stmt.syntax().token_span();
        match stmt {
            Stmt::Assign(node) => {
                self.check_assign_parts(ctx, node.lhs(), node.rhs(), node.token_span(), false)
            }
            Stmt::RefAssign(node) => {
                self.check_assign_parts(ctx, node.lhs(), node.rhs(), node.token_span(), true)
            }
            Stmt::Attempt(node) => self.check_attempt(ctx, node),
            Stmt::Call(node) => {
                let Some(call) = node.call() else {
                    return self.error_stmt(span);
                };
                let id = self.check_call_expr(ctx, &call, true, None);
                if self.hir.expr(id).ty != self.hir.types.error()
                    && !matches!(
                        self.hir.expr(id).kind,
                        ExprKindHir::Call { .. } | ExprKindHir::Error
                    )
                {
                    return self.error_stmt(span);
                }
                self.hir.alloc_stmt(StmtKindHir::Call { call: id }, span)
            }
            Stmt::Return(_) => self.hir.alloc_stmt(StmtKindHir::Return, span),
            Stmt::If(node) => self.check_if(ctx, node),
            Stmt::Case(node) => self.check_case(ctx, node),
            Stmt::For(node) => self.check_for(ctx, node),
            Stmt::While(node) => self.check_while(ctx, node),
            Stmt::Repeat(node) => self.check_repeat(ctx, node),
            Stmt::Exit(_) => {
                if ctx.loop_depth == 0 {
                    self.error(
                        crate::codes::NAME_INVALID_CONTEXT,
                        "`EXIT` is only valid inside a loop",
                        span,
                    );
                }
                self.hir.alloc_stmt(StmtKindHir::Exit, span)
            }
            Stmt::Continue(_) => {
                if ctx.loop_depth == 0 {
                    self.error(
                        crate::codes::NAME_INVALID_CONTEXT,
                        "`CONTINUE` is only valid inside a loop",
                        span,
                    );
                }
                self.hir.alloc_stmt(StmtKindHir::Continue, span)
            }
            Stmt::Empty(_) => self.hir.alloc_stmt(StmtKindHir::Empty, span),
            Stmt::Error(_) => self.error_stmt(span),
        }
    }

    fn error_stmt(&mut self, span: Span) -> StmtId {
        self.hir.alloc_stmt(StmtKindHir::Empty, span)
    }

    fn check_assign_parts(
        &mut self,
        ctx: &mut BodyCtx,
        lhs: Option<Expr>,
        rhs: Option<Expr>,
        span: Span,
        parsed_ref: bool,
    ) -> StmtId {
        let Some(lhs) = lhs else {
            return self.error_stmt(span);
        };
        let Some(rhs) = rhs else {
            return self.error_stmt(span);
        };
        let place = self.check_place(ctx, &lhs, span);
        let place_ty = self.hir.expr(place).ty;
        self.check_place_writable(place, span);
        self.check_forbidden_control(ctx, place, span);
        if place_ty == self.hir.types.error() {
            let value = self.check_expr(ctx, &rhs, None);
            return self
                .hir
                .alloc_stmt(StmtKindHir::Assign { place, value }, span);
        }
        let rhs_id = self.check_expr(ctx, &rhs, None);
        let rhs_ty = self.hir.expr(rhs_id).ty;
        self.check_pointer_escape(place, rhs_id, span);
        self.check_class_handle_escape(place, rhs_id, span);
        let lhs_is_deref = matches!(
            self.hir.expr(place).kind,
            ExprKindHir::Place(PlaceHir::Deref(_))
        );
        let reference_assignment =
            !lhs_is_deref && self.is_reference_assignment_pair(place_ty, rhs_ty);
        if parsed_ref && !reference_assignment && lhs_is_deref {
            // `p^ := value` is an ordinary assignment.
        }
        if reference_assignment {
            let value = self.coerce_expr_to(
                rhs_id,
                place_ty,
                false,
                rhs.token_span(),
                crate::codes::TYPE_ASSIGN,
            );
            self.hir
                .alloc_stmt(StmtKindHir::RefAssign { place, value }, span)
        } else {
            let value = self.coerce_expr_to(
                rhs_id,
                place_ty,
                false,
                rhs.token_span(),
                crate::codes::TYPE_ASSIGN,
            );
            self.hir
                .alloc_stmt(StmtKindHir::Assign { place, value }, span)
        }
    }

    /// Rejects the direct escape of a pointer to callee-owned storage through
    /// a function or method result (Grammar §9.4).
    fn check_pointer_escape(&mut self, place: ExprId, value: ExprId, span: Span) {
        if !matches!(
            self.hir.expr(place).kind,
            ExprKindHir::Place(PlaceHir::Result(_))
        ) {
            return;
        }
        let ExprKindHir::AddressOf { target } = self.hir.expr(value).kind else {
            return;
        };
        if self.addressed_storage_is_callee_owned(target) {
            self.error(
                crate::codes::REF_RULE,
                "a pointer to a local, parameter, or result temporary cannot be returned",
                span,
            );
        }
    }

    /// Rejects class handles and interface values that refer to
    /// activation-owned records escaping into a result, persistent storage, or
    /// an `OUT`/`VAR_IN_OUT` parameter.
    ///
    /// A class/FB local owns its record on the activation stack; a handle or
    /// interface to it cannot outlive the call. Values read from an `IN`
    /// parameter, `THIS`, or persistent storage refer to caller- or
    /// host-owned records and are therefore allowed (Runtime and Execution
    /// §2.6).
    fn check_class_handle_escape(&mut self, place: ExprId, value: ExprId, span: Span) {
        let value_ty = self.hir.expr(value).ty;
        let place_ty = self.hir.expr(place).ty;
        // An interface destination can be reached by an implicit class/FB
        // upcast, so the escape also depends on the destination type.
        if !(self.contains_class_handle(value_ty)
            || self.contains_interface(value_ty)
            || self.contains_interface(place_ty))
            || !self.destination_allows_escape(place)
        {
            return;
        }
        if self.handle_value_is_callee_owned(value) {
            self.error(
                crate::codes::REF_RULE,
                "a class handle or interface to activation-owned storage cannot escape this call",
                span,
            );
        }
    }

    /// Returns `true` when assigning to `place` stores into result, persistent,
    /// or caller storage.
    fn destination_allows_escape(&self, place: ExprId) -> bool {
        let root = self.place_root(place).unwrap_or(place);
        match &self.hir.expr(root).kind {
            ExprKindHir::Place(PlaceHir::Result(_))
            | ExprKindHir::Place(PlaceHir::Global(_))
            | ExprKindHir::Place(PlaceHir::This(_))
            | ExprKindHir::Place(PlaceHir::InstanceField { .. })
            | ExprKindHir::Place(PlaceHir::Deref(_)) => true,
            ExprKindHir::Place(PlaceHir::Param(param)) => {
                self.hir.param(*param).direction != ArgDirection::Input
            }
            _ => false,
        }
    }

    /// Returns `true` when a handle- or interface-typed expression reads from
    /// activation-owned storage.
    ///
    /// Upcast casts are transparent: assigning a local class/FB value to an
    /// interface keeps the same activation-owned record.
    fn handle_value_is_callee_owned(&self, value: ExprId) -> bool {
        if let ExprKindHir::Cast { operand, .. } = &self.hir.expr(value).kind {
            return self.handle_value_is_callee_owned(*operand);
        }
        let root = self.place_root(value).unwrap_or(value);
        matches!(
            &self.hir.expr(root).kind,
            ExprKindHir::Place(PlaceHir::Local(_))
        )
    }

    /// Returns `true` when the addressed place is callee-owned stack storage:
    /// a local, parameter, result, or a field/subscript of one. A pointer or
    /// reference dereference addresses storage owned elsewhere.
    fn addressed_storage_is_callee_owned(&self, id: ExprId) -> bool {
        let ExprKindHir::Place(place) = &self.hir.expr(id).kind else {
            return false;
        };
        match place {
            PlaceHir::Local(_) | PlaceHir::Param(_) | PlaceHir::Result(_) => true,
            PlaceHir::Field { base, .. } => self.addressed_storage_is_callee_owned(*base),
            PlaceHir::Index { base, .. } => {
                if matches!(
                    self.hir.types.get(self.hir.expr(*base).ty),
                    Ty::Ptr(_) | Ty::Ref(_)
                ) {
                    false
                } else {
                    self.addressed_storage_is_callee_owned(*base)
                }
            }
            PlaceHir::This(_)
            | PlaceHir::Global(_)
            | PlaceHir::Deref(_)
            | PlaceHir::BitField { .. }
            | PlaceHir::Direct(_)
            | PlaceHir::InstanceField { .. } => false,
        }
    }

    fn check_attempt(
        &mut self,
        ctx: &mut BodyCtx,
        node: &stc_syntax::ast::AssignmentAttemptStmt,
    ) -> StmtId {
        let span = node.token_span();
        let Some(lhs) = node.lhs() else {
            return self.error_stmt(span);
        };
        let Some(rhs) = node.rhs() else {
            return self.error_stmt(span);
        };
        let place = self.check_place(ctx, &lhs, span);
        self.check_place_writable(place, span);
        let dst_ty = self.hir.expr(place).ty;
        let value = self.check_expr(ctx, &rhs, None);
        let src_ty = self.hir.expr(value).ty;
        if dst_ty != self.hir.types.error() && src_ty != self.hir.types.error() {
            let destination_ok = self.is_reference_like(dst_ty);
            let source_ok = self.is_reference_like(src_ty)
                || self.is_class_like(src_ty)
                || matches!(self.hir.types.get(src_ty), Ty::Null);
            if !destination_ok || !source_ok {
                self.error(
                    crate::codes::REF_ATTEMPT,
                    "`?=` requires a class, interface, or reference destination",
                    span,
                );
            } else if !self.attempt_related(dst_ty, src_ty) {
                let code = if self.is_class_like(dst_ty) && self.is_class_like(src_ty) {
                    crate::codes::OOP_ASSIGN_TARGET
                } else {
                    crate::codes::REF_ATTEMPT
                };
                self.error(
                    code,
                    format!(
                        "`{}` cannot be the target of an assignment attempt from `{}`",
                        self.type_name(dst_ty),
                        self.type_name(src_ty)
                    ),
                    span,
                );
            }
        }
        self.hir
            .alloc_stmt(StmtKindHir::Attempt { place, value }, span)
    }

    fn check_if(&mut self, ctx: &mut BodyCtx, node: &IfStmt) -> StmtId {
        let span = node.token_span();
        let mut arms = Vec::new();
        if let Some(condition) = node.condition() {
            let cond = self.check_condition(ctx, &condition);
            let body = node
                .then_block()
                .map(|list| self.check_stmt_list(ctx, &list))
                .unwrap_or_default();
            arms.push((cond, body));
        }
        for branch in node.elsif_branches() {
            if let Some(condition) = branch.condition() {
                let cond = self.check_condition(ctx, &condition);
                let body = branch
                    .body()
                    .map(|list| self.check_stmt_list(ctx, &list))
                    .unwrap_or_default();
                arms.push((cond, body));
            }
        }
        let else_body = node
            .else_block()
            .map(|list| self.check_stmt_list(ctx, &list))
            .unwrap_or_default();
        self.hir
            .alloc_stmt(StmtKindHir::If { arms, else_body }, span)
    }

    fn check_case(&mut self, ctx: &mut BodyCtx, node: &CaseStmt) -> StmtId {
        let span = node.token_span();
        let selector = match node.selector() {
            Some(selector) => self.check_expr(ctx, &selector, None),
            None => self.alloc_error_expr(span),
        };
        let selector_ty = self.hir.expr(selector).ty;
        if selector_ty != self.hir.types.error()
            && !self.is_int_ty(selector_ty)
            && !self.is_enum_like(selector_ty)
            && !matches!(
                self.hir.types.get(self.scalar_base(selector_ty)),
                Ty::Char(_) | Ty::BitString(_)
            )
        {
            self.error(
                crate::codes::TYPE_INVALID_OPERAND,
                "`CASE` selector must be an integer, enum, character, or bit-string value",
                span,
            );
        }
        let mut arms = Vec::new();
        let mut seen: Vec<(ConstValue, ConstValue)> = Vec::new();
        for selection in node.selections() {
            let mut labels = Vec::new();
            for label in selection.labels() {
                let expressions = label.expressions();
                let lower = expressions
                    .first()
                    .and_then(|expr| self.eval_case_label(expr, selector_ty, ctx.scope, span));
                let upper = expressions
                    .get(1)
                    .and_then(|expr| self.eval_case_label(expr, selector_ty, ctx.scope, span));
                let Some(lower) = lower else {
                    continue;
                };
                if let Some(upper) = &upper {
                    if compare_consts(&lower, upper) == Some(std::cmp::Ordering::Greater) {
                        self.error(
                            crate::codes::CONST_FAILURE,
                            "CASE range lower bound exceeds upper bound",
                            span,
                        );
                    }
                }
                let high = upper.clone().unwrap_or_else(|| lower.clone());
                let duplicate = seen.iter().any(|(existing_low, existing_high)| {
                    labels_overlap(&lower, &high, existing_low, existing_high)
                });
                if duplicate {
                    self.error(
                        crate::codes::CONST_DUPLICATE_LABEL,
                        "duplicate or overlapping CASE label",
                        label.token_span(),
                    );
                }
                seen.push((lower.clone(), high));
                labels.push(CaseLabelHir {
                    lower,
                    upper,
                    span: label.token_span(),
                });
            }
            let body = selection
                .body()
                .map(|list| self.check_stmt_list(ctx, &list))
                .unwrap_or_default();
            arms.push(CaseArmHir { labels, body });
        }
        let else_body = node
            .else_block()
            .map(|list| self.check_stmt_list(ctx, &list))
            .unwrap_or_default();
        self.hir.alloc_stmt(
            StmtKindHir::Case {
                selector,
                arms,
                else_body,
            },
            span,
        )
    }

    fn eval_case_label(
        &mut self,
        expr: &Expr,
        selector_ty: TyId,
        scope: DefId,
        _span: Span,
    ) -> Option<ConstValue> {
        let (_, value) = self.eval_const_expr(expr, Some(selector_ty), scope)?;
        Some(value)
    }

    fn check_for(&mut self, ctx: &mut BodyCtx, node: &ForStmt) -> StmtId {
        let span = node.token_span();
        let Some(variable) = node.variable() else {
            return self.error_stmt(span);
        };
        let Some((var, var_ty)) = self.place_from_name(ctx, &variable) else {
            return self.error_stmt(span);
        };
        if var_ty != self.hir.types.error() && !self.is_int_ty(var_ty) {
            self.error(
                crate::codes::TYPE_INVALID_OPERAND,
                "the `FOR` control variable must be an integer",
                variable.token_span(),
            );
        }
        self.check_forbidden_control(ctx, var, span);
        let expressions = node.expressions();
        let start = expressions
            .first()
            .map(|expr| self.check_expr(ctx, expr, Some(var_ty)))
            .unwrap_or_else(|| self.alloc_error_expr(span));
        let end = expressions
            .get(1)
            .map(|expr| self.check_expr(ctx, expr, Some(var_ty)))
            .unwrap_or_else(|| self.alloc_error_expr(span));
        let step = expressions
            .get(2)
            .map(|expr| self.check_expr(ctx, expr, Some(var_ty)))
            .unwrap_or_else(|| {
                let one = ConstValue::Int(1);
                self.hir.alloc_expr(
                    ExprKindHir::Literal(stc_hir::LiteralHir { value: one }),
                    var_ty,
                    span,
                )
            });
        if let Some(expr) = expressions.get(2)
            && let Some(value) = self.eval_const_int(expr, ctx.scope)
            && value == 0
        {
            self.error(
                crate::codes::CONST_FAILURE,
                "`FOR` step must not be zero",
                expr.token_span(),
            );
        }
        let key = self.place_key(var);
        if let Some(key) = key {
            ctx.controlled.push(key);
        }
        ctx.loop_depth += 1;
        let body = node
            .body()
            .map(|list| self.check_stmt_list(ctx, &list))
            .unwrap_or_default();
        ctx.loop_depth -= 1;
        if key.is_some() {
            ctx.controlled.pop();
        }
        self.hir.alloc_stmt(
            StmtKindHir::For {
                var,
                start,
                end,
                step,
                body,
            },
            span,
        )
    }

    fn check_while(&mut self, ctx: &mut BodyCtx, node: &WhileStmt) -> StmtId {
        let span = node.token_span();
        let cond = node
            .condition()
            .map(|expr| self.check_condition(ctx, &expr))
            .unwrap_or_else(|| self.alloc_error_expr(span));
        ctx.loop_depth += 1;
        let body = node
            .body()
            .map(|list| self.check_stmt_list(ctx, &list))
            .unwrap_or_default();
        ctx.loop_depth -= 1;
        self.hir.alloc_stmt(StmtKindHir::While { cond, body }, span)
    }

    fn check_repeat(&mut self, ctx: &mut BodyCtx, node: &RepeatStmt) -> StmtId {
        let span = node.token_span();
        ctx.loop_depth += 1;
        let body = node
            .body()
            .map(|list| self.check_stmt_list(ctx, &list))
            .unwrap_or_default();
        ctx.loop_depth -= 1;
        let cond = node
            .condition()
            .map(|expr| self.check_condition(ctx, &expr))
            .unwrap_or_else(|| self.alloc_error_expr(span));
        self.hir
            .alloc_stmt(StmtKindHir::Repeat { body, cond }, span)
    }

    fn check_condition(&mut self, ctx: &mut BodyCtx, expr: &Expr) -> ExprId {
        let bool_ty = self.hir.types.bool();
        let id = self.check_expr(ctx, expr, None);
        let ty = self.hir.expr(id).ty;
        if ty == bool_ty || ty == self.hir.types.error() {
            return id;
        }
        if self.literal_can_adapt(id, bool_ty) {
            return self.coerce_expr_to(
                id,
                bool_ty,
                false,
                expr.token_span(),
                crate::codes::TYPE_MISMATCH,
            );
        }
        self.push_diag(
            Diagnostic::error(
                crate::codes::TYPE_MISMATCH,
                format!("condition must be `BOOL`, found `{}`", self.type_name(ty)),
                expr.token_span(),
            )
            .with_note("write an explicit comparison such as `(value <> 0)`"),
        );
        id
    }

    fn check_forbidden_control(&mut self, ctx: &mut BodyCtx, place: ExprId, span: Span) {
        if let Some(key) = self.place_key(place)
            && ctx.controlled.contains(&key)
        {
            self.error(
                crate::codes::TYPE_INVALID_OPERAND,
                "the `FOR` control variable cannot be modified inside the loop body",
                span,
            );
        }
    }

    fn check_place(&mut self, ctx: &mut BodyCtx, expr: &Expr, span: Span) -> ExprId {
        let id = self.check_expr(ctx, expr, None);
        if !self.is_place_expr(id) && self.hir.expr(id).ty != self.hir.types.error() {
            self.error(
                crate::codes::TYPE_ASSIGN,
                "assignment target is not a variable, field, or dereference",
                span,
            );
        }
        id
    }

    fn check_place_writable(&mut self, place: ExprId, span: Span) {
        let ty = self.hir.expr(place).ty;
        let _ = ty;
        if let Some(root) = self.place_root(place)
            && root != place
            && let ExprKindHir::Place(PlaceHir::Global(def)) = &self.hir.expr(root).kind
        {
            let info = match &self.hir.defs[def.index()].kind {
                DefKind::Global { constant, at, .. } => Some((*constant, at.clone())),
                _ => None,
            };
            if let Some((constant, at)) = info {
                if constant {
                    self.error(
                        crate::codes::TYPE_ASSIGN,
                        "cannot assign through a `CONSTANT` variable",
                        span,
                    );
                }
                if at
                    .as_ref()
                    .is_some_and(|address| matches!(address.area, Some(stc_hir::DirectArea::Input)))
                {
                    self.error(
                        crate::codes::IO_INPUT_WRITE,
                        "`%I` inputs are read-only",
                        span,
                    );
                }
            }
        }
        match &self.hir.expr(place).kind {
            ExprKindHir::Place(PlaceHir::Global(def)) => {
                let info = match &self.hir.defs[def.index()].kind {
                    DefKind::Global { constant, at, .. } => Some((*constant, at.clone())),
                    _ => None,
                };
                if let Some((constant, at)) = info {
                    if constant {
                        self.error(
                            crate::codes::TYPE_ASSIGN,
                            "cannot assign to a `CONSTANT` variable",
                            span,
                        );
                    }
                    if at.as_ref().is_some_and(|address| {
                        matches!(address.area, Some(stc_hir::DirectArea::Input))
                    }) {
                        self.error(
                            crate::codes::IO_INPUT_WRITE,
                            "`%I` inputs are read-only",
                            span,
                        );
                    }
                }
            }
            ExprKindHir::Place(PlaceHir::Field { field, .. }) => {
                let data = self.hir.field(*field).clone();
                if data.constant {
                    self.error(
                        crate::codes::TYPE_ASSIGN,
                        "cannot assign to a `CONSTANT` field",
                        span,
                    );
                }
                if data
                    .at
                    .as_ref()
                    .is_some_and(|address| matches!(address.area, Some(stc_hir::DirectArea::Input)))
                {
                    self.error(
                        crate::codes::IO_INPUT_WRITE,
                        "`%I` inputs are read-only",
                        span,
                    );
                }
            }
            ExprKindHir::Place(PlaceHir::Direct(address)) => {
                if matches!(address.area, Some(stc_hir::DirectArea::Input)) {
                    self.error(
                        crate::codes::IO_INPUT_WRITE,
                        "`%I` inputs are read-only",
                        span,
                    );
                }
            }
            _ => {}
        }
    }

    fn is_place_expr(&self, id: ExprId) -> bool {
        matches!(self.hir.expr(id).kind, ExprKindHir::Place(_))
    }

    fn place_root(&self, id: ExprId) -> Option<ExprId> {
        match &self.hir.expr(id).kind {
            ExprKindHir::Place(PlaceHir::Field { base, .. })
            | ExprKindHir::Place(PlaceHir::Index { base, .. })
            | ExprKindHir::Place(PlaceHir::Deref(base))
            | ExprKindHir::Place(PlaceHir::BitField { base, .. }) => self.place_root(*base),
            ExprKindHir::Place(_) => Some(id),
            _ => None,
        }
    }

    fn place_key(&self, id: ExprId) -> Option<PlaceKey> {
        match &self.hir.expr(id).kind {
            ExprKindHir::Place(PlaceHir::Local(local)) => Some(PlaceKey::Local(*local)),
            ExprKindHir::Place(PlaceHir::Param(param)) => Some(PlaceKey::Param(*param)),
            ExprKindHir::Place(PlaceHir::Global(def)) => Some(PlaceKey::Global(*def)),
            ExprKindHir::Place(PlaceHir::Result(def)) => Some(PlaceKey::Result(*def)),
            ExprKindHir::Place(PlaceHir::Field { field, .. }) => Some(PlaceKey::Field(*field)),
            _ => None,
        }
    }

    fn is_reference_assignment_pair(&self, dst: TyId, src: TyId) -> bool {
        if matches!(self.hir.types.get(src), Ty::Null) {
            return self.is_reference_like(dst);
        }
        self.is_reference_like(dst) && (self.is_reference_like(src) || self.is_class_like(src))
    }

    fn attempt_related(&self, dst: TyId, src: TyId) -> bool {
        if matches!(self.hir.types.get(src), Ty::Null) {
            return self.is_reference_like(dst);
        }
        match (self.hir.types.get(dst), self.hir.types.get(src)) {
            (Ty::Ref(dst), Ty::Ref(src)) => {
                self.is_reference_like(dst.target)
                    && self.is_reference_like(src.target)
                    && (self.is_subtype(dst.target, src.target)
                        || self.is_subtype(src.target, dst.target))
            }
            _ => {
                self.is_reference_like(dst)
                    && self.is_class_like(src)
                    && (self.is_subtype(dst, src) || self.is_subtype(src, dst))
            }
        }
    }

    // ---- expressions -----------------------------------------------------

    fn check_expr(&mut self, ctx: &mut BodyCtx, expr: &Expr, expected: Option<TyId>) -> ExprId {
        let id = match expr {
            Expr::Literal(literal) => self.check_literal(literal, expected, ctx.scope),
            Expr::Paren(paren) => match paren.inner() {
                Some(inner) => self.check_expr(ctx, &inner, expected),
                None => self.alloc_error_expr(expr.token_span()),
            },
            Expr::Unary(unary) => self.check_unary(ctx, unary),
            Expr::Binary(binary) => {
                let id = self.check_binary(ctx, binary);
                if let Some(expected) = expected {
                    self.coerce_expr_to(
                        id,
                        expected,
                        false,
                        binary.token_span(),
                        crate::codes::TYPE_MISMATCH,
                    )
                } else {
                    id
                }
            }
            Expr::Path(path) => self.check_path_value(ctx, path, expected),
            Expr::RefValue(reference) => self.check_ref_value(ctx, reference),
            Expr::AddressOf(node) => self.check_address_of(ctx, node),
            Expr::SizeOf(node) => self.check_sizeof(ctx, node),
            Expr::PointerConversion(node) => self.check_pointer_conversion(ctx, node),
            Expr::Call(call) => self.check_call_expr(ctx, call, false, expected),
            Expr::Member(_) | Expr::Index(_) | Expr::Deref(_) => {
                let id = self.check_postfix(ctx, expr);
                if let Some(expected) = expected {
                    self.coerce_expr_to(
                        id,
                        expected,
                        false,
                        expr.token_span(),
                        crate::codes::TYPE_MISMATCH,
                    )
                } else {
                    id
                }
            }
            Expr::Error(_) => self.alloc_error_expr(expr.token_span()),
        };
        id
    }

    fn check_literal(
        &mut self,
        literal: &stc_syntax::ast::LiteralExpr,
        expected: Option<TyId>,
        scope: DefId,
    ) -> ExprId {
        let span = literal.token_span();
        if let Some(token) = literal.token()
            && let Some(Literal::Direct(address)) = token.literal()
        {
            if address.wildcard {
                self.error(
                    crate::codes::IO_ADDRESS,
                    "a wildcard `AT` address is not valid in an executable expression",
                    span,
                );
                return self.alloc_error_expr(span);
            }
            let ty = self.direct_ty(address, span);
            let place = PlaceHir::Direct(crate::collect::direct_address(address.clone(), span));
            return self.hir.alloc_expr(ExprKindHir::Place(place), ty, span);
        }
        let expr = Expr::Literal(literal.clone());
        match self.eval_const_expr(&expr, expected, scope) {
            Some((ty, value)) => self.hir.alloc_expr(
                ExprKindHir::Literal(stc_hir::LiteralHir { value }),
                ty,
                span,
            ),
            None => self.alloc_error_expr(span),
        }
    }

    fn check_unary(&mut self, ctx: &mut BodyCtx, unary: &UnaryExpr) -> ExprId {
        let Some(operand) = unary.operand() else {
            return self.alloc_error_expr(unary.token_span());
        };
        let operand_id = self.check_expr(ctx, &operand, None);
        let operand_ty = self.hir.expr(operand_id).ty;
        let span = unary.token_span();
        if operand_ty == self.hir.types.error() {
            return self.hir.alloc_expr(
                ExprKindHir::Unary {
                    op: UnOp::Not,
                    operand: operand_id,
                },
                operand_ty,
                span,
            );
        }
        let Some(operator) = unary.operator() else {
            return self.alloc_error_expr(span);
        };
        let (op, valid) = match operator.kind() {
            stc_syntax::SyntaxKind::PunctPlus => (UnOp::Plus, self.is_numeric_or_time(operand_ty)),
            stc_syntax::SyntaxKind::PunctMinus => (UnOp::Neg, self.is_numeric_or_time(operand_ty)),
            stc_syntax::SyntaxKind::KwNot => (
                UnOp::Not,
                matches!(
                    self.hir.types.get(self.scalar_base(operand_ty)),
                    Ty::Bool | Ty::BitString(_)
                ),
            ),
            _ => (UnOp::Not, false),
        };
        if !valid {
            self.error(
                crate::codes::TYPE_INVALID_OPERAND,
                format!(
                    "unary `{}` is not defined for `{}`",
                    op.as_str(),
                    self.type_name(operand_ty)
                ),
                span,
            );
        }
        self.hir.alloc_expr(
            ExprKindHir::Unary {
                op,
                operand: operand_id,
            },
            operand_ty,
            span,
        )
    }

    fn check_binary(&mut self, ctx: &mut BodyCtx, binary: &stc_syntax::ast::BinaryExpr) -> ExprId {
        // Flatten the left spine so a 20k-term chain is processed iteratively.
        let mut spine: Vec<(BinOp, Expr, Span)> = Vec::new();
        let mut current = Expr::Binary(binary.clone());
        loop {
            let Expr::Binary(node) = current else {
                break;
            };
            let Some(operator) = node.operator() else {
                return self.alloc_error_expr(node.token_span());
            };
            let Some(op) = bin_op(operator.kind()) else {
                return self.alloc_error_expr(node.token_span());
            };
            let Some(rhs) = node.rhs() else {
                return self.alloc_error_expr(node.token_span());
            };
            spine.push((op, rhs, operator.span()));
            let Some(lhs) = node.lhs() else {
                return self.alloc_error_expr(node.token_span());
            };
            current = lhs;
        }
        let mut acc = self.check_expr(ctx, &current, None);
        for (op, rhs, span) in spine.into_iter().rev() {
            let rhs_id = self.check_expr(ctx, &rhs, None);
            acc = self.build_binary(ctx, op, acc, rhs_id, span);
        }
        acc
    }

    fn build_binary(
        &mut self,
        ctx: &mut BodyCtx,
        op: BinOp,
        lhs: ExprId,
        rhs: ExprId,
        span: Span,
    ) -> ExprId {
        let _ = ctx;
        let lhs_ty = self.hir.expr(lhs).ty;
        let rhs_ty = self.hir.expr(rhs).ty;
        if lhs_ty == self.hir.types.error() || rhs_ty == self.hir.types.error() {
            return self.hir.alloc_expr(
                ExprKindHir::Binary { op, lhs, rhs },
                self.hir.types.error(),
                span,
            );
        }
        if self.is_pointer_ty(lhs_ty) || self.is_pointer_ty(rhs_ty) {
            return self.build_pointer_binary(op, lhs, rhs, span);
        }
        if op.is_comparison() {
            let (lhs, rhs, _) = self.comparable_operands(op, lhs, rhs, span);
            return self.hir.alloc_expr(
                ExprKindHir::Binary { op, lhs, rhs },
                self.hir.types.bool(),
                span,
            );
        }
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Power => {
                let (lhs, rhs, ty) = self.arithmetic_operands(op, lhs, rhs, span);
                self.hir
                    .alloc_expr(ExprKindHir::Binary { op, lhs, rhs }, ty, span)
            }
            BinOp::Mod => {
                let (lhs, rhs, ty) = self.integer_operands(lhs, rhs, span);
                self.hir
                    .alloc_expr(ExprKindHir::Binary { op, lhs, rhs }, ty, span)
            }
            BinOp::And | BinOp::Or | BinOp::Xor => {
                let (lhs, rhs, ty) = self.bit_operands(lhs, rhs, span);
                self.hir
                    .alloc_expr(ExprKindHir::Binary { op, lhs, rhs }, ty, span)
            }
            _ => self.hir.alloc_expr(
                ExprKindHir::Binary { op, lhs, rhs },
                self.hir.types.error(),
                span,
            ),
        }
    }

    /// Types and checks pointer arithmetic and comparisons (D-145).
    fn build_pointer_binary(&mut self, op: BinOp, lhs: ExprId, rhs: ExprId, span: Span) -> ExprId {
        let lhs_ty = self.hir.expr(lhs).ty;
        let rhs_ty = self.hir.expr(rhs).ty;
        let lhs_ptr = self.is_pointer_ty(lhs_ty);
        let rhs_ptr = self.is_pointer_ty(rhs_ty);
        if op.is_comparison() {
            let comparable = (lhs_ptr && rhs_ptr)
                || (lhs_ptr && matches!(self.hir.types.get(rhs_ty), Ty::Null))
                || (rhs_ptr && matches!(self.hir.types.get(lhs_ty), Ty::Null));
            if !comparable {
                self.error(
                    crate::codes::TYPE_INVALID_OPERAND,
                    format!(
                        "cannot compare `{}` with `{}` using `{}`",
                        self.type_name(lhs_ty),
                        self.type_name(rhs_ty),
                        op.as_str()
                    ),
                    span,
                );
                return self.hir.alloc_expr(
                    ExprKindHir::Binary { op, lhs, rhs },
                    self.hir.types.error(),
                    span,
                );
            }
            let (lhs, rhs, _) = self.comparable_operands(op, lhs, rhs, span);
            return self.hir.alloc_expr(
                ExprKindHir::Binary { op, lhs, rhs },
                self.hir.types.bool(),
                span,
            );
        }
        match op {
            BinOp::Add if lhs_ptr && self.is_int_ty(rhs_ty) => {
                let ty = lhs_ty;
                self.hir
                    .alloc_expr(ExprKindHir::Binary { op, lhs, rhs }, ty, span)
            }
            BinOp::Add if rhs_ptr && self.is_int_ty(lhs_ty) => {
                let ty = rhs_ty;
                self.hir
                    .alloc_expr(ExprKindHir::Binary { op, lhs, rhs }, ty, span)
            }
            BinOp::Sub if lhs_ptr && self.is_int_ty(rhs_ty) => {
                let ty = lhs_ty;
                self.hir
                    .alloc_expr(ExprKindHir::Binary { op, lhs, rhs }, ty, span)
            }
            BinOp::Sub if lhs_ptr && rhs_ptr && lhs_ty == rhs_ty => {
                let ty = self.hir.types.int(true, 64);
                self.hir
                    .alloc_expr(ExprKindHir::Binary { op, lhs, rhs }, ty, span)
            }
            _ => {
                self.error(
                    crate::codes::TYPE_INVALID_OPERAND,
                    format!(
                        "operator `{}` is not defined for `{}` and `{}`",
                        op.as_str(),
                        self.type_name(lhs_ty),
                        self.type_name(rhs_ty)
                    ),
                    span,
                );
                self.hir.alloc_expr(
                    ExprKindHir::Binary { op, lhs, rhs },
                    self.hir.types.error(),
                    span,
                )
            }
        }
    }

    fn comparable_operands(
        &mut self,
        op: BinOp,
        lhs: ExprId,
        rhs: ExprId,
        span: Span,
    ) -> (ExprId, ExprId, TyId) {
        let lhs_ty = self.hir.expr(lhs).ty;
        let rhs_ty = self.hir.expr(rhs).ty;
        if self.can_assign(lhs_ty, rhs_ty) != Conv::No {
            let rhs =
                self.coerce_expr_to(rhs, lhs_ty, false, span, crate::codes::TYPE_INVALID_OPERAND);
            return (lhs, rhs, lhs_ty);
        }
        if self.can_assign(rhs_ty, lhs_ty) != Conv::No {
            let lhs =
                self.coerce_expr_to(lhs, rhs_ty, false, span, crate::codes::TYPE_INVALID_OPERAND);
            return (lhs, rhs, rhs_ty);
        }
        self.error(
            crate::codes::TYPE_INVALID_OPERAND,
            format!(
                "cannot compare `{}` with `{}` using `{}`",
                self.type_name(lhs_ty),
                self.type_name(rhs_ty),
                op.as_str()
            ),
            span,
        );
        (lhs, rhs, self.hir.types.error())
    }

    fn arithmetic_operands(
        &mut self,
        op: BinOp,
        lhs: ExprId,
        rhs: ExprId,
        span: Span,
    ) -> (ExprId, ExprId, TyId) {
        let lhs_original = self.hir.expr(lhs).ty;
        let rhs_original = self.hir.expr(rhs).ty;
        let lhs_time = self.is_time_ty(lhs_original);
        let rhs_time = self.is_time_ty(rhs_original);
        if lhs_time || rhs_time {
            let result = match op {
                BinOp::Add | BinOp::Sub if lhs_time && rhs_time => {
                    let long = matches!(
                        self.hir.types.get(self.scalar_base(lhs_original)),
                        Ty::Time(stc_hir::TimeTy::LTime)
                    ) || matches!(
                        self.hir.types.get(self.scalar_base(rhs_original)),
                        Ty::Time(stc_hir::TimeTy::LTime)
                    );
                    if long {
                        self.hir.types.time(stc_hir::TimeTy::LTime)
                    } else {
                        self.hir.types.time(stc_hir::TimeTy::Time)
                    }
                }
                BinOp::Mul | BinOp::Div if lhs_time && self.is_numeric(rhs_original) => {
                    lhs_original
                }
                BinOp::Mul if rhs_time && self.is_numeric(lhs_original) => rhs_original,
                _ => {
                    self.error(
                        crate::codes::TYPE_INVALID_OPERAND,
                        format!(
                            "operator `{}` is not defined for `{}` and `{}`",
                            op.as_str(),
                            self.type_name(lhs_original),
                            self.type_name(rhs_original)
                        ),
                        span,
                    );
                    return (lhs, rhs, self.hir.types.error());
                }
            };
            let lhs = if self.is_time_ty(lhs_original) {
                self.coerce_expr_to(lhs, result, false, span, crate::codes::TYPE_INVALID_OPERAND)
            } else {
                lhs
            };
            let rhs = if self.is_time_ty(rhs_original) {
                self.coerce_expr_to(rhs, result, false, span, crate::codes::TYPE_INVALID_OPERAND)
            } else {
                rhs
            };
            return (lhs, rhs, result);
        }
        if !self.is_numeric(lhs_original) || !self.is_numeric(rhs_original) {
            self.error(
                crate::codes::TYPE_INVALID_OPERAND,
                format!(
                    "operator `{}` requires numeric operands, found `{}` and `{}`",
                    op.as_str(),
                    self.type_name(lhs_original),
                    self.type_name(rhs_original)
                ),
                span,
            );
            return (lhs, rhs, self.hir.types.error());
        }
        let lhs_ty = lhs_original;
        let rhs_ty = rhs_original;
        let result = match (
            self.hir.types.get(self.scalar_base(lhs_ty)),
            self.hir.types.get(self.scalar_base(rhs_ty)),
        ) {
            (Ty::Real(a), Ty::Real(b)) => {
                let ty =
                    if matches!(a, stc_hir::RealTy::LReal) || matches!(b, stc_hir::RealTy::LReal) {
                        self.hir.types.real(stc_hir::RealTy::LReal)
                    } else {
                        self.hir.types.real(stc_hir::RealTy::Real)
                    };
                ty
            }
            (Ty::Real(_), _) | (_, Ty::Real(_)) => self.hir.types.real(stc_hir::RealTy::Real),
            (Ty::Int(a), Ty::Int(b)) => {
                let width = a.width.max(b.width);
                self.hir.types.int(a.signed || b.signed, width)
            }
            _ => self.hir.types.error(),
        };
        let lhs = self.coerce_expr_to(lhs, result, false, span, crate::codes::TYPE_INVALID_OPERAND);
        let rhs = self.coerce_expr_to(rhs, result, false, span, crate::codes::TYPE_INVALID_OPERAND);
        (lhs, rhs, result)
    }

    fn integer_operands(&mut self, lhs: ExprId, rhs: ExprId, span: Span) -> (ExprId, ExprId, TyId) {
        if !self.is_int_ty(self.hir.expr(lhs).ty) || !self.is_int_ty(self.hir.expr(rhs).ty) {
            self.error(
                crate::codes::TYPE_INVALID_OPERAND,
                "`MOD` requires integer operands",
                span,
            );
            return (lhs, rhs, self.hir.types.error());
        }
        let lhs_ty = self.scalar_base(self.hir.expr(lhs).ty);
        let rhs_ty = self.scalar_base(self.hir.expr(rhs).ty);
        if !matches!(
            (self.hir.types.get(lhs_ty), self.hir.types.get(rhs_ty)),
            (Ty::Int(_), Ty::Int(_))
        ) {
            self.error(
                crate::codes::TYPE_INVALID_OPERAND,
                "`MOD` requires integer operands",
                span,
            );
            return (lhs, rhs, self.hir.types.error());
        }
        let result = match (self.hir.types.get(lhs_ty), self.hir.types.get(rhs_ty)) {
            (Ty::Int(a), Ty::Int(b)) => self
                .hir
                .types
                .int(a.signed || b.signed, a.width.max(b.width)),
            _ => self.hir.types.error(),
        };
        let lhs = self.coerce_expr_to(lhs, result, false, span, crate::codes::TYPE_INVALID_OPERAND);
        let rhs = self.coerce_expr_to(rhs, result, false, span, crate::codes::TYPE_INVALID_OPERAND);
        (lhs, rhs, result)
    }

    fn bit_operands(&mut self, lhs: ExprId, rhs: ExprId, span: Span) -> (ExprId, ExprId, TyId) {
        let lhs_ty = self.hir.expr(lhs).ty;
        let rhs_ty = self.hir.expr(rhs).ty;
        let lhs_base = self.scalar_base(lhs_ty);
        let rhs_base = self.scalar_base(rhs_ty);
        if matches!(self.hir.types.get(lhs_base), Ty::Bool)
            && matches!(self.hir.types.get(rhs_base), Ty::Bool)
        {
            return (lhs, rhs, lhs_base);
        }
        if let Ty::BitString(width) = self.hir.types.get(lhs_base)
            && let Ty::BitString(other) = self.hir.types.get(rhs_base)
        {
            let result = if width == other {
                lhs_base
            } else {
                self.hir.types.bit_string((*width).max(*other))
            };
            let lhs =
                self.coerce_expr_to(lhs, result, false, span, crate::codes::TYPE_INVALID_OPERAND);
            let rhs =
                self.coerce_expr_to(rhs, result, false, span, crate::codes::TYPE_INVALID_OPERAND);
            return (lhs, rhs, result);
        }
        self.error(
            crate::codes::TYPE_INVALID_OPERAND,
            format!(
                "bitwise operator requires `BOOL` or matching bit strings, found `{}` and `{}`",
                self.type_name(lhs_ty),
                self.type_name(rhs_ty)
            ),
            span,
        );
        (lhs, rhs, self.hir.types.error())
    }

    fn check_postfix(&mut self, ctx: &mut BodyCtx, expr: &Expr) -> ExprId {
        enum Op {
            Index(Vec<Expr>, Span),
            Member(Option<Name>, Option<u16>, Span),
            Deref(Span),
        }
        let mut ops = Vec::new();
        let mut current = expr.clone();
        loop {
            match current {
                Expr::Index(node) => {
                    let Some(base) = node.base() else {
                        return self.alloc_error_expr(node.token_span());
                    };
                    ops.push(Op::Index(node.indices(), node.token_span()));
                    current = base;
                }
                Expr::Member(node) => {
                    let Some(base) = node.base() else {
                        return self.alloc_error_expr(node.token_span());
                    };
                    let bit = node.bit_index().and_then(|token| match token.kind() {
                        stc_syntax::SyntaxKind::IntLiteral => token.text().parse::<u16>().ok(),
                        stc_syntax::SyntaxKind::DirectVariable => match token.literal() {
                            Some(Literal::Direct(address))
                                if !address.wildcard
                                    && matches!(
                                        address.size,
                                        None | Some(stc_lexer::DirectSize::Bit)
                                    ) =>
                            {
                                address
                                    .indices
                                    .first()
                                    .copied()
                                    .and_then(|value| u16::try_from(value).ok())
                            }
                            _ => None,
                        },
                        _ => None,
                    });
                    ops.push(Op::Member(node.member(), bit, node.token_span()));
                    current = base;
                }
                Expr::Deref(node) => {
                    let Some(base) = node.operand() else {
                        return self.alloc_error_expr(node.token_span());
                    };
                    ops.push(Op::Deref(node.token_span()));
                    current = base;
                }
                other => {
                    let mut acc = self.check_expr(ctx, &other, None);
                    for op in ops.into_iter().rev() {
                        acc = match op {
                            Op::Index(indices, span) => self.apply_index(ctx, acc, &indices, span),
                            Op::Member(name, bit, span) => {
                                self.apply_member(ctx, acc, name, bit, span)
                            }
                            Op::Deref(span) => self.apply_deref(acc, span),
                        };
                    }
                    return acc;
                }
            }
        }
    }

    fn literal_int(expr: &Expr) -> Option<i128> {
        match expr {
            Expr::Literal(literal) => {
                let token = literal.token()?;
                match token.literal()? {
                    Literal::Numeric(numeric)
                        if numeric.kind == stc_lexer::NumericKind::Integer =>
                    {
                        i128::from_str_radix(&numeric.text, numeric.radix.base()).ok()
                    }
                    Literal::Typed(typed) => match &typed.value {
                        stc_lexer::TypedValue::Numeric(numeric)
                            if numeric.kind == stc_lexer::NumericKind::Integer =>
                        {
                            let value =
                                i128::from_str_radix(&numeric.text, numeric.radix.base()).ok()?;
                            if typed.sign == Some(stc_lexer::Sign::Minus) {
                                value.checked_neg()
                            } else {
                                Some(value)
                            }
                        }
                        _ => None,
                    },
                    _ => None,
                }
            }
            Expr::Unary(unary) => {
                let operator = unary.operator()?;
                let value = Self::literal_int(&unary.operand()?)?;
                if operator.kind() == stc_syntax::SyntaxKind::PunctMinus {
                    value.checked_neg()
                } else {
                    Some(value)
                }
            }
            Expr::Paren(paren) => Self::literal_int(&paren.inner()?),
            _ => None,
        }
    }

    fn apply_index(
        &mut self,
        ctx: &mut BodyCtx,
        base: ExprId,
        indices: &[Expr],
        span: Span,
    ) -> ExprId {
        let base_ty = self.hir.expr(base).ty;
        if let Ty::Ptr(pointer) = self.hir.types.get(self.scalar_base(base_ty)).clone() {
            if indices.len() != 1 {
                self.error(
                    crate::codes::TYPE_INVALID_OPERAND,
                    "pointer indexing consumes exactly one subscript; use `p^[i, j]` to index the pointee array",
                    span,
                );
            }
            let dint = self.hir.types.int(true, 32);
            let mut lowered = Vec::new();
            for index in indices {
                let id = self.check_expr(ctx, index, Some(dint));
                if !self.is_int_ty(self.hir.expr(id).ty) {
                    self.error(
                        crate::codes::TYPE_INVALID_OPERAND,
                        "pointer index must be an integer",
                        index.token_span(),
                    );
                }
                lowered.push(id);
            }
            let place = PlaceHir::Index {
                base,
                indices: lowered,
            };
            return self
                .hir
                .alloc_expr(ExprKindHir::Place(place), pointer.target, span);
        }
        let result_ty = match self.hir.types.get(self.scalar_base(base_ty)).clone() {
            Ty::Array(array) => {
                if indices.len() != array.bounds.len() {
                    self.error(
                        crate::codes::ARRAY_BOUND,
                        format!(
                            "array rank is {}, but {} subscript(s) were given",
                            array.bounds.len(),
                            indices.len()
                        ),
                        span,
                    );
                }
                array.element
            }
            Ty::String(string) => match string.kind {
                stc_hir::StringKind::Sbcs => self.hir.types.char_ty(stc_hir::CharTy::Char),
                stc_hir::StringKind::Dbcs => self.hir.types.char_ty(stc_hir::CharTy::WChar),
                stc_hir::StringKind::Unicode => self.hir.types.char_ty(stc_hir::CharTy::UChar),
            },
            _ => {
                self.error(
                    crate::codes::TYPE_INVALID_OPERAND,
                    format!("type `{}` cannot be subscripted", self.type_name(base_ty)),
                    span,
                );
                self.hir.types.error()
            }
        };
        let static_bounds = match self.hir.types.get(self.scalar_base(base_ty)) {
            Ty::Array(array) if !array.conformant => Some(array.bounds.clone()),
            _ => None,
        };
        if let Some(bounds) = static_bounds {
            for (dimension, index) in indices.iter().enumerate() {
                if let (Some(value), Some(bound)) =
                    (Self::literal_int(index), bounds.get(dimension))
                    && (value < bound.lower || value > bound.upper)
                {
                    self.error(
                        crate::codes::ARRAY_BOUND,
                        format!(
                            "index {value} is outside the array bounds {}..{}",
                            bound.lower, bound.upper
                        ),
                        index.token_span(),
                    );
                }
            }
        }
        let dint = self.hir.types.int(true, 32);
        let mut lowered = Vec::new();
        for index in indices {
            let id = self.check_expr(ctx, index, Some(dint));
            if !self.is_int_ty(self.hir.expr(id).ty) {
                self.error(
                    crate::codes::TYPE_INVALID_OPERAND,
                    "array index must be an integer",
                    index.token_span(),
                );
            }
            lowered.push(id);
        }
        let place = PlaceHir::Index {
            base,
            indices: lowered,
        };
        self.hir
            .alloc_expr(ExprKindHir::Place(place), result_ty, span)
    }

    fn apply_member(
        &mut self,
        ctx: &mut BodyCtx,
        base: ExprId,
        name: Option<Name>,
        bit_index: Option<u16>,
        span: Span,
    ) -> ExprId {
        let base_ty = self.hir.expr(base).ty;
        if let Some(bit) = bit_index {
            return self.apply_bit_access(base, bit, span);
        }
        let Some(name) = name else {
            self.error(
                crate::codes::TYPE_INVALID_OPERAND,
                "invalid member access",
                span,
            );
            return self.alloc_error_expr(span);
        };
        let symbol = self.hir.intern(name.text());
        let Some(field) = self.field_lookup(base_ty, symbol) else {
            if !self.methods_named(base_ty, symbol).is_empty() {
                self.error(
                    crate::codes::NAME_INVALID_CONTEXT,
                    "a method must be called, not used as a value",
                    span,
                );
            } else {
                self.error(
                    crate::codes::NAME_UNKNOWN,
                    format!(
                        "type `{}` has no member `{}`",
                        self.type_name(base_ty),
                        name.text()
                    ),
                    span,
                );
            }
            return self.alloc_error_expr(span);
        };
        let field_data = self.hir.field(field).clone();
        self.check_field_access(field, ctx.owner, span);
        let place = PlaceHir::Field { base, field };
        self.record(name.syntax(), crate::Res::Field(field));
        self.hir
            .alloc_expr(ExprKindHir::Place(place), field_data.ty, span)
    }

    fn apply_bit_access(&mut self, base: ExprId, index: u16, span: Span) -> ExprId {
        let base_ty = self.hir.expr(base).ty;
        let width = match self.hir.types.get(self.scalar_base(base_ty)) {
            Ty::Bool => 1,
            Ty::BitString(width) => *width,
            _ => {
                self.error(
                    crate::codes::TYPE_INVALID_OPERAND,
                    "bit access requires `BOOL` or a bit-string value",
                    span,
                );
                return self.alloc_error_expr(span);
            }
        };
        if u32::from(index) >= u32::from(width) {
            self.error(
                crate::codes::ARRAY_BOUND,
                format!("bit index {index} is outside {width}-bit value"),
                span,
            );
        }
        let place = PlaceHir::BitField { base, bit: index };
        self.hir
            .alloc_expr(ExprKindHir::Place(place), self.hir.types.bool(), span)
    }

    fn apply_deref(&mut self, base: ExprId, span: Span) -> ExprId {
        let base_ty = self.hir.expr(base).ty;
        match self.hir.types.get(self.scalar_base(base_ty)).clone() {
            Ty::Ref(reference) => {
                let place = PlaceHir::Deref(base);
                self.hir
                    .alloc_expr(ExprKindHir::Place(place), reference.target, span)
            }
            Ty::Ptr(pointer) => {
                let place = PlaceHir::Deref(base);
                self.hir
                    .alloc_expr(ExprKindHir::Place(place), pointer.target, span)
            }
            Ty::Null => {
                self.error(crate::codes::REF_NULL, "cannot dereference `NULL`", span);
                self.alloc_error_expr(span)
            }
            _ => {
                self.error(
                    crate::codes::REF_RULE,
                    format!(
                        "`^` requires a reference or pointer, found `{}`",
                        self.type_name(base_ty)
                    ),
                    span,
                );
                self.alloc_error_expr(span)
            }
        }
    }

    fn check_ref_value(&mut self, ctx: &mut BodyCtx, reference: &RefValueExpr) -> ExprId {
        let span = reference.token_span();
        if reference.is_null() {
            return self
                .hir
                .alloc_expr(ExprKindHir::Null, self.hir.types.null(), span);
        }
        let Some(target) = reference.target() else {
            self.error(
                crate::codes::REF_RULE,
                "`REF` requires a target variable",
                span,
            );
            return self.alloc_error_expr(span);
        };
        let target_id = self.check_expr(ctx, &target, None);
        if !self.is_place_expr(target_id) {
            self.error(
                crate::codes::REF_RULE,
                "`REF` requires an addressable variable",
                span,
            );
            return self.alloc_error_expr(span);
        }
        let root = self.place_root(target_id).unwrap_or(target_id);
        if let ExprKindHir::Place(PlaceHir::Global(def)) = &self.hir.expr(root).kind
            && matches!(
                &self.hir.defs[def.index()].kind,
                DefKind::Global { constant: true, .. }
            )
        {
            self.error(
                crate::codes::REF_RULE,
                "`REF` cannot target a `CONSTANT` variable",
                span,
            );
        }
        let target_ty = self.hir.expr(target_id).ty;
        if self.is_pointer_ty(target_ty) {
            self.error(
                crate::codes::REF_RULE,
                "`REF` of a pointer is not allowed; use the pointer value directly",
                span,
            );
            return self.alloc_error_expr(span);
        }
        let ty = self
            .hir
            .types
            .intern(Ty::Ref(stc_hir::RefTy { target: target_ty }));
        self.hir
            .alloc_expr(ExprKindHir::Ref { target: target_id }, ty, span)
    }

    // ---- STC-POINTER expressions ------------------------------------------

    fn check_address_of(&mut self, ctx: &mut BodyCtx, node: &AddressOfExpr) -> ExprId {
        let span = node.token_span();
        let Some(operand) = node.operand() else {
            self.error(crate::codes::REF_RULE, "`ADR` requires a place", span);
            return self.alloc_error_expr(span);
        };
        let target = self.check_expr(ctx, &operand, None);
        let target_ty = self.hir.expr(target).ty;
        if target_ty == self.hir.types.error() {
            return self.alloc_error_expr(span);
        }
        if !self.is_place_expr(target) {
            self.error(
                crate::codes::REF_RULE,
                "`ADR` requires a place, not a value",
                span,
            );
            return self.alloc_error_expr(span);
        }
        if let Some(problem) = self.address_of_place_problem(target) {
            self.error(crate::codes::REF_RULE, problem, span);
            return self.alloc_error_expr(span);
        }
        let Some(decayed) = self.decay_pointer_target(target_ty, span) else {
            return self.alloc_error_expr(span);
        };
        if let Some(problem) = self.pointer_target_problem(decayed) {
            self.error(
                crate::codes::REF_RULE,
                format!("`ADR` target must be contiguous storage, found {problem}"),
                span,
            );
            return self.alloc_error_expr(span);
        }
        if self.place_is_persistent(target) && !self.is_single_root_ty(target_ty) {
            self.error(
                crate::codes::REF_RULE,
                "`ADR` of a split persistent aggregate has no stable storage; address a leaf or a transient value",
                span,
            );
            return self.alloc_error_expr(span);
        }
        let ty = self.hir.types.pointer(decayed);
        self.hir
            .alloc_expr(ExprKindHir::AddressOf { target }, ty, span)
    }

    /// Returns a diagnostic message for a place `ADR` may not address.
    fn address_of_place_problem(&self, id: ExprId) -> Option<&'static str> {
        let ExprKindHir::Place(place) = &self.hir.expr(id).kind else {
            return None;
        };
        match place {
            PlaceHir::BitField { .. } => Some("`ADR` cannot target a bit field"),
            PlaceHir::Global(def) => match &self.hir.defs[def.index()].kind {
                DefKind::Global { constant: true, .. } => {
                    Some("`ADR` cannot target a `CONSTANT` variable")
                }
                _ => None,
            },
            PlaceHir::Field { base, field } => {
                if self.hir.field(*field).constant {
                    Some("`ADR` cannot target a `CONSTANT` field")
                } else {
                    self.address_of_place_problem(*base)
                }
            }
            PlaceHir::Index { base, .. } => self.address_of_place_problem(*base),
            PlaceHir::Direct(address) => {
                let bit_address = matches!(address.size, Some(stc_hir::DirectSize::Bit))
                    || (address.size.is_none() && address.indices.len() == 2);
                if address.wildcard {
                    Some("`ADR` cannot target a wildcard `AT` address")
                } else if bit_address {
                    Some("`ADR` cannot target an individual bit")
                } else if matches!(address.area, Some(stc_hir::DirectArea::Input)) {
                    Some("`ADR` cannot target a `%I` input")
                } else {
                    None
                }
            }
            PlaceHir::Deref(base) => {
                if matches!(self.hir.types.get(self.hir.expr(*base).ty), Ty::Ref(_)) {
                    Some(
                        "`ADR` of a `REF_TO` dereference is not allowed; use the `POINTER TO T(reference)` conversion",
                    )
                } else {
                    None
                }
            }
            PlaceHir::Local(local) => self
                .declaration_is_constant(crate::DeclId::Local(*local))
                .then_some("`ADR` cannot target a `CONSTANT` variable"),
            PlaceHir::Param(param) => self
                .declaration_is_constant(crate::DeclId::Param(*param))
                .then_some("`ADR` cannot target a `CONSTANT` variable"),
            PlaceHir::Result(_) | PlaceHir::This(_) | PlaceHir::InstanceField { .. } => None,
        }
    }

    fn declaration_is_constant(&self, id: crate::DeclId) -> bool {
        self.decl_sites
            .iter()
            .any(|site| site.id == id && site.constant)
    }

    /// Returns `true` when a place's root is persistent host storage.
    fn place_is_persistent(&self, id: ExprId) -> bool {
        self.place_root(id).is_some_and(|root| {
            matches!(
                &self.hir.expr(root).kind,
                ExprKindHir::Place(
                    PlaceHir::Global(_) | PlaceHir::This(_) | PlaceHir::InstanceField { .. }
                )
            )
        })
    }

    /// Applies the `ADR(array)` one-dimension decay.
    fn decay_pointer_target(&mut self, ty: TyId, span: Span) -> Option<TyId> {
        let Ty::Array(array) = self.hir.types.get(ty).clone() else {
            return Some(ty);
        };
        if array.conformant {
            self.error(
                crate::codes::ARRAY_CONFORMANT,
                "a whole `ARRAY [*]` value is not a pointer target",
                span,
            );
            return None;
        }
        if array.bounds.len() <= 1 {
            return Some(array.element);
        }
        let bounds = array.bounds[1..].to_vec();
        Some(self.hir.types.intern(Ty::Array(stc_hir::ArrayTy {
            element: array.element,
            bounds,
            conformant: false,
        })))
    }

    fn check_sizeof(&mut self, ctx: &mut BodyCtx, node: &SizeOfExpr) -> ExprId {
        let span = node.token_span();
        let udint = self.hir.types.int(false, 32);
        if let Some(type_ref) = node.type_ref() {
            let ty = self.resolve_type_ref(&type_ref, ctx.scope);
            if ty == self.hir.types.error() {
                return self.alloc_error_expr(span);
            }
            if let Some(problem) = self.sizeof_type_problem(ty) {
                self.error(
                    crate::codes::TYPE_INVALID_OPERAND,
                    format!("`SIZEOF` requires a statically sized type, found {problem}"),
                    span,
                );
                return self.alloc_error_expr(span);
            }
            return self.hir.alloc_expr(
                ExprKindHir::SizeOf {
                    operand: SizeOfHir::Type(ty),
                },
                udint,
                span,
            );
        }
        let Some(operand) = node.operand() else {
            self.error(
                crate::codes::TYPE_INVALID_OPERAND,
                "`SIZEOF` requires a type or a place",
                span,
            );
            return self.alloc_error_expr(span);
        };
        if let Expr::Path(path) = &operand
            && !self.path_has_value(ctx, path)
            && let Some(def) = self.resolve_type_path_expr(path, ctx.scope)
        {
            let ty = self.resolve_named_type_def(def);
            if ty != self.hir.types.error() {
                if let Some(problem) = self.sizeof_type_problem(ty) {
                    self.error(
                        crate::codes::TYPE_INVALID_OPERAND,
                        format!("`SIZEOF` requires a statically sized type, found {problem}"),
                        span,
                    );
                    return self.alloc_error_expr(span);
                }
                return self.hir.alloc_expr(
                    ExprKindHir::SizeOf {
                        operand: SizeOfHir::Type(ty),
                    },
                    udint,
                    span,
                );
            }
        }
        let place = self.check_expr(ctx, &operand, None);
        let ty = self.hir.expr(place).ty;
        if ty == self.hir.types.error() {
            return self.alloc_error_expr(span);
        }
        if !self.is_place_expr(place) {
            self.error(
                crate::codes::TYPE_INVALID_OPERAND,
                "`SIZEOF` requires a type or a place",
                span,
            );
            return self.alloc_error_expr(span);
        }
        if self.is_bit_field(place) {
            self.error(
                crate::codes::TYPE_INVALID_OPERAND,
                "`SIZEOF` of a bit field is not allowed",
                span,
            );
            return self.alloc_error_expr(span);
        }
        if let Some(problem) = self.sizeof_type_problem(ty) {
            self.error(
                crate::codes::TYPE_INVALID_OPERAND,
                format!("`SIZEOF` requires a statically sized place, found {problem}"),
                span,
            );
            return self.alloc_error_expr(span);
        }
        if self.place_is_persistent(place) && !self.is_single_root_ty(ty) {
            self.error(
                crate::codes::TYPE_INVALID_OPERAND,
                "`SIZEOF` of a split persistent aggregate has no contiguous storage",
                span,
            );
            return self.alloc_error_expr(span);
        }
        self.hir.alloc_expr(
            ExprKindHir::SizeOf {
                operand: SizeOfHir::Place(place),
            },
            udint,
            span,
        )
    }

    fn is_bit_field(&self, id: ExprId) -> bool {
        matches!(
            &self.hir.expr(id).kind,
            ExprKindHir::Place(PlaceHir::BitField { .. })
        )
    }

    fn sizeof_type_problem(&self, ty: TyId) -> Option<&'static str> {
        match self.hir.types.get(ty) {
            Ty::Generic(_) => Some("a generic type"),
            Ty::Null | Ty::Ref(_) => Some("a reference"),
            Ty::Named(def)
                if matches!(
                    &self.hir.defs[def.index()].kind,
                    DefKind::Class { .. }
                        | DefKind::Interface { .. }
                        | DefKind::FunctionBlock { .. }
                        | DefKind::Program { .. }
                ) =>
            {
                Some("an object or interface type")
            }
            Ty::Array(array) if array.conformant => Some("a conformant array"),
            _ => {
                if self.contains_conformant(ty) {
                    Some("a conformant array")
                } else if self.contains_object(ty) {
                    Some("an object or interface type")
                } else {
                    None
                }
            }
        }
    }

    /// Returns `true` when a bare or qualified name denotes a value or place
    /// in the current context. A visible value wins over a type of the same
    /// name (Grammar §11.11).
    fn path_has_value(&mut self, ctx: &BodyCtx, path: &PathExpr) -> bool {
        if path.prefix().is_some() {
            return true;
        }
        let segments = path.segments();
        let Some(first) = segments.first() else {
            return false;
        };
        let symbol = self.hir.intern(first.text());
        if ctx.locals.contains_key(&symbol)
            || ctx.params.contains_key(&symbol)
            || ctx.result == Some(symbol)
        {
            return true;
        }
        if let Some(owner) = ctx.owner {
            let ty = self.hir.types.intern(Ty::Named(owner));
            if self.field_lookup(ty, symbol).is_some() {
                return true;
            }
        }
        match self.lookup_scope(ctx.scope, symbol) {
            Lookup::Entries(defs) => {
                if defs
                    .iter()
                    .any(|def| matches!(self.hir.defs[def.index()].kind, DefKind::Namespace { .. }))
                {
                    return false;
                }
                defs.iter()
                    .any(|def| !crate::resolve::is_type_like(&self.hir.defs[def.index()].kind))
            }
            Lookup::Constant(_) => true,
            Lookup::Ambiguous(_) | Lookup::NotFound => false,
        }
    }

    fn check_pointer_conversion(
        &mut self,
        ctx: &mut BodyCtx,
        node: &PointerConversionExpr,
    ) -> ExprId {
        let span = node.token_span();
        let Some(type_ref) = node.type_ref() else {
            self.error(
                crate::codes::REF_RULE,
                "the conversion requires a target type",
                span,
            );
            return self.alloc_error_expr(span);
        };
        let target = self.resolve_type_ref(&type_ref, ctx.scope);
        let to = if node.is_pointer_form() {
            self.resolve_pointer_target(target, span)
        } else {
            self.resolve_reference_target(target, span)
        };
        let Some(operand) = node.operand() else {
            self.error(
                crate::codes::REF_RULE,
                "the conversion requires an operand",
                span,
            );
            return self.alloc_error_expr(span);
        };
        let value = self.check_expr(ctx, &operand, None);
        let from = self.hir.expr(value).ty;
        if from == self.hir.types.error() || to == self.hir.types.error() {
            return self.alloc_error_expr(span);
        }
        let ok = match (self.hir.types.get(from), self.hir.types.get(to)) {
            (Ty::Ptr(source), Ty::Ref(destination)) if source.target == destination.target => true,
            (Ty::Ref(source), Ty::Ptr(destination)) if source.target == destination.target => {
                if let Some(problem) = self.pointer_target_problem(source.target) {
                    self.error(
                        crate::codes::REF_RULE,
                        format!(
                            "`POINTER TO` conversion requires contiguous target storage, found {problem}"
                        ),
                        span,
                    );
                    false
                } else {
                    true
                }
            }
            _ => {
                self.error(
                    crate::codes::REF_RULE,
                    format!(
                        "the conversion requires `POINTER TO T` and `REF_TO T` with the same target type; found `{}` and `{}`",
                        self.type_name(from),
                        self.type_name(to)
                    ),
                    span,
                );
                false
            }
        };
        if !ok {
            return self.alloc_error_expr(span);
        }
        self.hir.alloc_expr(
            ExprKindHir::PointerConversion {
                operand: value,
                to,
                kind: CastKind::Explicit,
            },
            to,
            span,
        )
    }

    // ---- paths and names -------------------------------------------------

    fn check_path_value(
        &mut self,
        ctx: &mut BodyCtx,
        path: &PathExpr,
        expected: Option<TyId>,
    ) -> ExprId {
        let id = self.check_path_value_inner(ctx, path);
        if let Some(expected) = expected {
            if self.hir.expr(id).ty != self.hir.types.error() {
                return self.coerce_expr_to(
                    id,
                    expected,
                    false,
                    path.token_span(),
                    crate::codes::TYPE_MISMATCH,
                );
            }
        }
        id
    }

    fn check_path_value_inner(&mut self, ctx: &mut BodyCtx, path: &PathExpr) -> ExprId {
        let span = path.token_span();
        let segments = path.segments();
        let prefix = path.prefix();
        match prefix {
            Some(token) if token.kind() == stc_syntax::SyntaxKind::KwThis => {
                if !ctx.allow_this {
                    self.error(
                        crate::codes::NAME_INVALID_CONTEXT,
                        "`THIS` is only valid in a method or FB body",
                        span,
                    );
                    return self.alloc_error_expr(span);
                }
                let Some(mut expr) = self.make_this(ctx, span) else {
                    return self.alloc_error_expr(span);
                };
                let mut ty = ctx.this_ty.unwrap_or_else(|| self.hir.types.error());
                if segments.is_empty() {
                    return expr;
                }
                for segment in &segments {
                    let symbol = self.hir.intern(segment.text());
                    let Some(field) = self.field_lookup(ty, symbol) else {
                        return self.member_error(expr, ty, segment, span);
                    };
                    let field_data = self.hir.field(field).clone();
                    self.check_field_access(field, ctx.owner, span);
                    expr = self.hir.alloc_expr(
                        ExprKindHir::Place(PlaceHir::Field { base: expr, field }),
                        field_data.ty,
                        span,
                    );
                    ty = field_data.ty;
                }
                if let Some(owner) = ctx.owner {
                    self.record(path.syntax(), crate::Res::Def(owner));
                }
                return expr;
            }
            Some(token) if token.kind() == stc_syntax::SyntaxKind::KwSuper => {
                if segments.is_empty() {
                    self.error(
                        crate::codes::NAME_INVALID_CONTEXT,
                        "`SUPER` is only valid as `SUPER()` or `SUPER.member`",
                        span,
                    );
                    return self.alloc_error_expr(span);
                }
                let Some(base) = ctx.super_base else {
                    self.error(
                        crate::codes::NAME_INVALID_CONTEXT,
                        "`SUPER` requires a base type",
                        span,
                    );
                    return self.alloc_error_expr(span);
                };
                let Some(mut expr) = self.make_this(ctx, span) else {
                    return self.alloc_error_expr(span);
                };
                let mut ty = self.hir.types.intern(Ty::Named(base));
                for segment in &segments {
                    let symbol = self.hir.intern(segment.text());
                    let Some(field) = self.field_lookup(ty, symbol) else {
                        return self.member_error(expr, ty, segment, span);
                    };
                    let field_data = self.hir.field(field).clone();
                    self.check_field_access(field, ctx.owner, span);
                    expr = self.hir.alloc_expr(
                        ExprKindHir::Place(PlaceHir::Field { base: expr, field }),
                        field_data.ty,
                        span,
                    );
                    ty = field_data.ty;
                }
                let _ = ty;
                return expr;
            }
            _ => {}
        }
        let Some(first) = segments.first() else {
            self.error(crate::codes::TYPE_INVALID_OPERAND, "empty name", span);
            return self.alloc_error_expr(span);
        };
        let symbol = self.hir.intern(first.text());
        let rest = &segments[1..];
        match self.resolve_first(ctx, symbol, first.token_span()) {
            FirstName::Local(local) => {
                let local_data = self.hir.local(local).clone();
                let place = local_data
                    .external_target
                    .map_or(PlaceHir::Local(local), PlaceHir::Global);
                let ty = local_data.external_target.map_or(local_data.ty, |global| {
                    match &self.hir.defs[global.index()].kind {
                        DefKind::Global { ty, .. } => *ty,
                        _ => local_data.ty,
                    }
                });
                let expr = self
                    .hir
                    .alloc_expr(ExprKindHir::Place(place), ty, first.token_span());
                self.record(path.syntax(), crate::Res::Local(local));
                self.project_value(ctx, expr, ty, rest, span)
            }
            FirstName::Param(param) => {
                let ty = self.hir.params[param.index()].ty;
                let expr =
                    self.hir
                        .alloc_expr(ExprKindHir::Place(PlaceHir::Param(param)), ty, span);
                self.record(path.syntax(), crate::Res::Param(param));
                self.project_value(ctx, expr, ty, rest, span)
            }
            FirstName::Result(def) => {
                let ty = match &self.hir.defs[def.index()].kind {
                    DefKind::Function {
                        result: Some(ty), ..
                    }
                    | DefKind::Method {
                        result: Some(ty), ..
                    } => *ty,
                    _ => self.hir.types.error(),
                };
                let expr = self
                    .hir
                    .alloc_expr(ExprKindHir::Place(PlaceHir::Result(def)), ty, span);
                self.record(path.syntax(), crate::Res::Def(def));
                self.project_value(ctx, expr, ty, rest, span)
            }
            FirstName::Field(field, base) => {
                let field_data = self.hir.field(field).clone();
                let expr = self.hir.alloc_expr(
                    ExprKindHir::Place(PlaceHir::Field { base, field }),
                    field_data.ty,
                    span,
                );
                self.record(path.syntax(), crate::Res::Field(field));
                self.project_value(ctx, expr, field_data.ty, rest, span)
            }
            FirstName::Global(def) => {
                let ty = match &self.hir.defs[def.index()].kind {
                    DefKind::Global { ty, .. } => *ty,
                    _ => self.hir.types.error(),
                };
                let expr = self
                    .hir
                    .alloc_expr(ExprKindHir::Place(PlaceHir::Global(def)), ty, span);
                self.record(path.syntax(), crate::Res::Def(def));
                self.project_value(ctx, expr, ty, rest, span)
            }
            FirstName::Constant(binding) => {
                if !rest.is_empty() {
                    self.error(
                        crate::codes::NAME_INVALID_CONTEXT,
                        "a constant has no members",
                        span,
                    );
                    return self.alloc_error_expr(span);
                }
                if let (Some(owner), ConstValue::Enum { value, .. }) =
                    (binding.owner, &binding.value)
                {
                    self.record(path.syntax(), crate::Res::EnumValue(owner, *value));
                } else if let Some(owner) = binding.owner {
                    self.record(path.syntax(), crate::Res::Def(owner));
                }
                self.hir.alloc_expr(
                    ExprKindHir::Literal(stc_hir::LiteralHir {
                        value: binding.value,
                    }),
                    binding.ty,
                    span,
                )
            }
            FirstName::Namespace(namespace) => self.qualified_value(ctx, namespace, rest, path),
            FirstName::Function | FirstName::Method | FirstName::Standard => {
                self.error(
                    crate::codes::NAME_INVALID_CONTEXT,
                    "a callable name must be called",
                    span,
                );
                self.alloc_error_expr(span)
            }
            FirstName::Type | FirstName::StandardFb => {
                self.error(
                    crate::codes::NAME_INVALID_CONTEXT,
                    "a type name is not a value",
                    span,
                );
                self.alloc_error_expr(span)
            }
            FirstName::Error => self.alloc_error_expr(span),
        }
    }

    fn project_value(
        &mut self,
        ctx: &mut BodyCtx,
        mut expr: ExprId,
        mut ty: TyId,
        segments: &[Name],
        span: Span,
    ) -> ExprId {
        for segment in segments {
            let symbol = self.hir.intern(segment.text());
            let Some(field) = self.field_lookup(ty, symbol) else {
                return self.member_error(expr, ty, segment, span);
            };
            let field_data = self.hir.field(field).clone();
            self.check_field_access(field, ctx.owner, span);
            expr = self.hir.alloc_expr(
                ExprKindHir::Place(PlaceHir::Field { base: expr, field }),
                field_data.ty,
                span,
            );
            ty = field_data.ty;
            self.record(segment.syntax(), crate::Res::Field(field));
        }
        expr
    }

    fn member_error(&mut self, _base: ExprId, ty: TyId, name: &Name, span: Span) -> ExprId {
        let symbol = self.hir.intern(name.text());
        if !self.methods_named(ty, symbol).is_empty() {
            self.error(
                crate::codes::NAME_INVALID_CONTEXT,
                "a method must be called, not used as a value",
                span,
            );
        } else {
            self.error(
                crate::codes::NAME_UNKNOWN,
                format!(
                    "type `{}` has no member `{}`",
                    self.type_name(ty),
                    name.text()
                ),
                span,
            );
        }
        self.alloc_error_expr(span)
    }

    fn qualified_value(
        &mut self,
        ctx: &mut BodyCtx,
        mut namespace: DefId,
        segments: &[Name],
        path: &PathExpr,
    ) -> ExprId {
        if segments.is_empty() {
            self.error(
                crate::codes::NAME_INVALID_CONTEXT,
                "a namespace is not a value",
                path.token_span(),
            );
            return self.alloc_error_expr(path.token_span());
        }
        for (index, segment) in segments.iter().enumerate() {
            let symbol = self.hir.intern(segment.text());
            let last = index + 1 == segments.len();
            let lookup = self.lookup_scope(namespace, symbol);
            match lookup {
                Lookup::Entries(defs) => {
                    if defs.len() != 1 {
                        self.error(
                            crate::codes::TYPE_INVALID_OPERAND,
                            "ambiguous name",
                            segment.token_span(),
                        );
                        return self.alloc_error_expr(path.token_span());
                    }
                    let def = defs[0];
                    if last {
                        let kind = self.hir.defs[def.index()].kind.clone();
                        return match kind {
                            DefKind::Global { ty, .. } => {
                                self.record(path.syntax(), crate::Res::Def(def));
                                self.hir.alloc_expr(
                                    ExprKindHir::Place(PlaceHir::Global(def)),
                                    ty,
                                    path.token_span(),
                                )
                            }
                            DefKind::Namespace { .. } => {
                                self.error(
                                    crate::codes::NAME_INVALID_CONTEXT,
                                    "a namespace is not a value",
                                    segment.token_span(),
                                );
                                self.alloc_error_expr(path.token_span())
                            }
                            DefKind::Enum { values, .. } => {
                                if let Some(value) = values.first() {
                                    let ty = self.hir.named_type(def);
                                    self.record(
                                        path.syntax(),
                                        crate::Res::EnumValue(def, value.value),
                                    );
                                    self.hir.alloc_expr(
                                        ExprKindHir::Literal(stc_hir::LiteralHir {
                                            value: ConstValue::Enum {
                                                def,
                                                value: value.value,
                                            },
                                        }),
                                        ty,
                                        path.token_span(),
                                    )
                                } else {
                                    self.alloc_error_expr(path.token_span())
                                }
                            }
                            _ => {
                                self.error(
                                    crate::codes::NAME_INVALID_CONTEXT,
                                    "name is not a value",
                                    segment.token_span(),
                                );
                                self.alloc_error_expr(path.token_span())
                            }
                        };
                    }
                    if crate::resolve::is_namespace(&self.hir.defs[def.index()].kind) {
                        namespace = def;
                        continue;
                    }
                    let enum_values = match &self.hir.defs[def.index()].kind {
                        DefKind::Enum { values, .. } => Some(values.clone()),
                        _ => None,
                    };
                    if let Some(values) = enum_values {
                        let next = &segments[index + 1];
                        let next_symbol = self.hir.intern(next.text());
                        if let Some(value) = values.iter().find(|value| value.name == next_symbol) {
                            let ty = self.hir.named_type(def);
                            let value = value.value;
                            self.record(path.syntax(), crate::Res::EnumValue(def, value));
                            return self.hir.alloc_expr(
                                ExprKindHir::Literal(stc_hir::LiteralHir {
                                    value: ConstValue::Enum { def, value },
                                }),
                                ty,
                                path.token_span(),
                            );
                        }
                    }
                    self.error(
                        crate::codes::NAME_INVALID_CONTEXT,
                        "qualified name does not denote a value",
                        segment.token_span(),
                    );
                    return self.alloc_error_expr(path.token_span());
                }
                Lookup::Constant(binding) => {
                    if !last {
                        self.error(
                            crate::codes::NAME_INVALID_CONTEXT,
                            "a constant has no members",
                            segment.token_span(),
                        );
                        return self.alloc_error_expr(path.token_span());
                    }
                    if let (Some(owner), ConstValue::Enum { value, .. }) =
                        (binding.owner, &binding.value)
                    {
                        self.record(path.syntax(), crate::Res::EnumValue(owner, *value));
                    }
                    return self.hir.alloc_expr(
                        ExprKindHir::Literal(stc_hir::LiteralHir {
                            value: binding.value,
                        }),
                        binding.ty,
                        path.token_span(),
                    );
                }
                Lookup::Ambiguous(spans) => {
                    let text = path.text().to_string();
                    self.report_ambiguous(&text, path.token_span(), &spans);
                    return self.alloc_error_expr(path.token_span());
                }
                Lookup::NotFound => {
                    self.unknown_name(&text_of(segment), segment.token_span());
                    return self.alloc_error_expr(path.token_span());
                }
            }
        }
        let _ = ctx;
        self.alloc_error_expr(path.token_span())
    }

    fn resolve_first(&mut self, ctx: &mut BodyCtx, symbol: Symbol, span: Span) -> FirstName {
        if let Some(local) = ctx.locals.get(&symbol) {
            return FirstName::Local(*local);
        }
        if let Some(param) = ctx.params.get(&symbol) {
            return FirstName::Param(*param);
        }
        if ctx.result == Some(symbol) {
            return FirstName::Result(ctx.def);
        }
        // Bare method name inside a method body resolves to a call on THIS.
        if let Some(owner) = ctx.owner {
            let owner_ty = self.hir.types.intern(Ty::Named(owner));
            let methods = self.methods_named(owner_ty, symbol);
            if !methods.is_empty() {
                return FirstName::Method;
            }
        }
        if let Some(owner) = ctx.owner {
            let ty = self.hir.types.intern(Ty::Named(owner));
            if let Some(field) = self.field_lookup(ty, symbol) {
                self.check_field_access(field, ctx.owner, span);
                if let Some(base) = self.make_this(ctx, span) {
                    return FirstName::Field(field, base);
                }
            }
        }
        match self.lookup_scope(ctx.scope, symbol) {
            Lookup::Entries(defs) => {
                if defs.len() > 1 {
                    let spans: Vec<Span> = defs
                        .iter()
                        .map(|def| self.hir.defs[def.index()].span)
                        .collect();
                    let name = self.symbol_text(symbol).to_string();
                    self.report_ambiguous(&name, span, &spans);
                    return FirstName::Error;
                }
                let def = defs[0];
                match &self.hir.defs[def.index()].kind {
                    DefKind::Function { .. } => FirstName::Function,
                    DefKind::Global { .. } => FirstName::Global(def),
                    DefKind::Namespace { .. } => FirstName::Namespace(def),
                    DefKind::Enum { values, .. } => {
                        // A single-segment enum value reference is resolved by
                        // name in the scope's constant table; this arm only
                        // remains for an unqualified enum type name.
                        let _ = values;
                        FirstName::Type
                    }
                    DefKind::NamedValue { values, .. } => {
                        let _ = values;
                        FirstName::Type
                    }
                    DefKind::Alias { .. }
                    | DefKind::Subrange { .. }
                    | DefKind::Struct { .. }
                    | DefKind::Class { .. }
                    | DefKind::Interface { .. }
                    | DefKind::FunctionBlock { .. }
                    | DefKind::Program { .. }
                    | DefKind::Method { .. }
                    | DefKind::Configuration { .. } => FirstName::Type,
                }
            }
            Lookup::Constant(binding) => FirstName::Constant(binding),
            Lookup::Ambiguous(spans) => {
                let name = self.symbol_text(symbol).to_string();
                self.report_ambiguous(&name, span, &spans);
                FirstName::Error
            }
            Lookup::NotFound => {
                if self.std_fns.contains_key(&symbol) {
                    return FirstName::Standard;
                }
                if self.std_fbs.contains_key(&symbol) {
                    return FirstName::StandardFb;
                }
                self.unknown_name(&self.symbol_text(symbol).to_string(), span);
                FirstName::Error
            }
        }
    }

    fn unknown_name(&mut self, name: &str, span: Span) {
        if let Some(decl_span) = self
            .test_only
            .iter()
            .find(|(symbol, _)| self.hir.interner.normalized(**symbol) == name.to_ascii_uppercase())
            .map(|(_, span)| *span)
        {
            self.push_diag(
                Diagnostic::error(
                    crate::codes::TEST_REFERENCE,
                    format!("`{name}` is a test-only unit and is not available in production"),
                    span,
                )
                .with_label(decl_span, "test-only declaration is here"),
            );
            return;
        }
        let mut diagnostic = Diagnostic::error(
            crate::codes::NAME_UNKNOWN,
            format!("unknown name `{name}`"),
            span,
        );
        if let Some(suggestion) = self.suggest_name(name) {
            diagnostic = diagnostic.with_suggestion(
                format!("did you mean `{suggestion}`?"),
                span,
                suggestion,
            );
        }
        self.push_diag(diagnostic);
    }

    pub(crate) fn suggest_name(&self, name: &str) -> Option<String> {
        let mut best: Option<(usize, String)> = None;
        let mut consider = |candidate: &str| {
            let distance =
                edit_distance(&name.to_ascii_uppercase(), &candidate.to_ascii_uppercase());
            if distance <= 2
                && distance > 0
                && best.as_ref().is_none_or(|(best, _)| distance < *best)
            {
                best = Some((distance, candidate.to_string()));
            }
        };
        for scope in self.scopes.values() {
            for symbol in scope.entries.keys().chain(scope.constants.keys()) {
                consider(self.hir.interner.text(*symbol));
            }
        }
        for field in &self.hir.fields {
            consider(self.hir.interner.text(field.name));
        }
        for param in &self.hir.params {
            consider(self.hir.interner.text(param.name));
        }
        for local in &self.hir.locals {
            consider(self.hir.interner.text(local.name));
        }
        for def in &self.hir.defs {
            if let Some(name) = def.name {
                consider(self.hir.interner.text(name));
            }
        }
        for symbol in self.std_fns.keys() {
            consider(self.hir.interner.text(*symbol));
        }
        best.map(|(_, name)| name)
    }

    fn make_this(&mut self, ctx: &mut BodyCtx, span: Span) -> Option<ExprId> {
        if let Some(expr) = ctx.this_expr {
            return Some(expr);
        }
        let Some(owner) = ctx.owner else {
            self.error(
                crate::codes::NAME_INVALID_CONTEXT,
                "no current instance",
                span,
            );
            return None;
        };
        let ty = ctx.this_ty.unwrap_or_else(|| self.hir.types.error());
        let expr = self
            .hir
            .alloc_expr(ExprKindHir::Place(PlaceHir::This(owner)), ty, span);
        ctx.this_expr = Some(expr);
        Some(expr)
    }

    fn place_from_name(&mut self, ctx: &mut BodyCtx, name: &Name) -> Option<(ExprId, TyId)> {
        let symbol = self.hir.intern(name.text());
        let span = name.token_span();
        match self.resolve_first(ctx, symbol, span) {
            FirstName::Local(local) => {
                let data = self.hir.local(local).clone();
                Some((
                    self.hir
                        .alloc_expr(ExprKindHir::Place(PlaceHir::Local(local)), data.ty, span),
                    data.ty,
                ))
            }
            FirstName::Param(param) => {
                let ty = self.hir.params[param.index()].ty;
                Some((
                    self.hir
                        .alloc_expr(ExprKindHir::Place(PlaceHir::Param(param)), ty, span),
                    ty,
                ))
            }
            FirstName::Global(def) => {
                let ty = match &self.hir.defs[def.index()].kind {
                    DefKind::Global { ty, .. } => *ty,
                    _ => self.hir.types.error(),
                };
                Some((
                    self.hir
                        .alloc_expr(ExprKindHir::Place(PlaceHir::Global(def)), ty, span),
                    ty,
                ))
            }
            FirstName::Result(def) => {
                let ty = match &self.hir.defs[def.index()].kind {
                    DefKind::Function {
                        result: Some(ty), ..
                    }
                    | DefKind::Method {
                        result: Some(ty), ..
                    } => *ty,
                    _ => self.hir.types.error(),
                };
                Some((
                    self.hir
                        .alloc_expr(ExprKindHir::Place(PlaceHir::Result(def)), ty, span),
                    ty,
                ))
            }
            FirstName::Field(field, base) => {
                let data = self.hir.field(field).clone();
                Some((
                    self.hir.alloc_expr(
                        ExprKindHir::Place(PlaceHir::Field { base, field }),
                        data.ty,
                        span,
                    ),
                    data.ty,
                ))
            }
            _ => None,
        }
    }

    // ---- calls -----------------------------------------------------------

    fn check_call_expr(
        &mut self,
        ctx: &mut BodyCtx,
        call: &CallExpr,
        as_statement: bool,
        expected: Option<TyId>,
    ) -> ExprId {
        let span = call.token_span();
        let Some(callee) = call.callee() else {
            return self.alloc_error_expr(span);
        };
        let kind = self.resolve_callee(ctx, &callee);
        let arguments = call.arguments();
        let id = match kind {
            CalleeKind::Functions(defs) => {
                self.check_function_call(ctx, &callee, defs, &arguments, span, as_statement)
            }
            CalleeKind::Standard(ids) => {
                self.check_standard_call(ctx, &callee, ids, &arguments, span, as_statement)
            }
            CalleeKind::Conversion { from, to } => {
                self.check_conversion_call(ctx, &callee, from, to, &arguments, span)
            }
            CalleeKind::Fb { instance, def } => {
                self.check_fb_call(ctx, &callee, instance, def, &arguments, span, as_statement)
            }
            CalleeKind::Method {
                receiver,
                methods,
                interface,
                is_super,
            } => self.check_method_call(
                ctx,
                &callee,
                receiver,
                methods,
                interface,
                is_super,
                &arguments,
                span,
                as_statement,
            ),
            CalleeKind::SuperBody { base } => {
                if !arguments.is_empty() {
                    self.error(
                        crate::codes::CALL_ARITY,
                        "`SUPER()` does not take arguments",
                        span,
                    );
                }
                if !as_statement {
                    self.error(
                        crate::codes::CALL_UNKNOWN,
                        "`SUPER()` is only valid as a statement",
                        span,
                    );
                }
                let Some(instance) = self.make_this(ctx, span) else {
                    return self.alloc_error_expr(span);
                };
                self.hir.alloc_expr(
                    ExprKindHir::Call {
                        callee: CalleeHir::SuperBody { instance, base },
                        args: Vec::new(),
                    },
                    self.hir.types.error(),
                    span,
                )
            }
            CalleeKind::Invalid => return self.alloc_error_expr(span),
        };
        if let Some(expected) = expected
            && self.hir.expr(id).ty != self.hir.types.error()
        {
            return self.coerce_expr_to(id, expected, false, span, crate::codes::TYPE_MISMATCH);
        }
        id
    }

    fn check_function_call(
        &mut self,
        ctx: &mut BodyCtx,
        callee: &Expr,
        defs: Vec<DefId>,
        arguments: &[Arg],
        span: Span,
        as_statement: bool,
    ) -> ExprId {
        let prepared = self.prepare_args(ctx, arguments);
        let mut candidates = Vec::new();
        for def in &defs {
            candidates.push((*def, self.function_formals(*def)));
        }
        let Some((def, formals, bindings)) = self.select_overload(&candidates, &prepared, span)
        else {
            return self.alloc_error_expr(span);
        };
        let result = match &self.hir.defs[def.index()].kind {
            DefKind::Function { result, .. } | DefKind::Method { result, .. } => *result,
            _ => None,
        };
        if result.is_none() && !as_statement {
            self.error(
                crate::codes::TYPE_INVALID_OPERAND,
                "a function without a result cannot be used as an expression",
                span,
            );
        }
        self.record(callee.syntax(), crate::Res::Def(def));
        let ty = result.unwrap_or_else(|| self.hir.types.error());
        self.finish_call(
            ctx,
            CalleeHir::Function(def),
            &formals,
            &bindings,
            &prepared,
            ty,
            span,
        )
    }

    fn check_method_call(
        &mut self,
        ctx: &mut BodyCtx,
        callee: &Expr,
        receiver: ExprId,
        methods: Vec<DefId>,
        interface: Option<DefId>,
        is_super: bool,
        arguments: &[Arg],
        span: Span,
        as_statement: bool,
    ) -> ExprId {
        let prepared = self.prepare_args(ctx, arguments);
        let mut candidates = Vec::new();
        for def in &methods {
            candidates.push((*def, self.function_formals(*def)));
        }
        let Some((def, formals, bindings)) = self.select_overload(&candidates, &prepared, span)
        else {
            return self.alloc_error_expr(span);
        };
        self.check_method_access(def, ctx.owner, span);
        let result = match &self.hir.defs[def.index()].kind {
            DefKind::Method { result, .. } => *result,
            _ => None,
        };
        if result.is_none() && !as_statement {
            self.error(
                crate::codes::TYPE_INVALID_OPERAND,
                "a method without a result cannot be used as an expression",
                span,
            );
        }
        self.record(callee.syntax(), crate::Res::Def(def));
        let callee_hir = if is_super {
            CalleeHir::SuperMethod { receiver, def }
        } else if let Some(interface) = interface {
            CalleeHir::InterfaceMethod {
                receiver,
                interface,
                method: def,
            }
        } else {
            CalleeHir::Method { receiver, def }
        };
        let ty = result.unwrap_or_else(|| self.hir.types.error());
        self.finish_call(ctx, callee_hir, &formals, &bindings, &prepared, ty, span)
    }

    fn check_fb_call(
        &mut self,
        ctx: &mut BodyCtx,
        callee: &Expr,
        instance: ExprId,
        def: DefId,
        arguments: &[Arg],
        span: Span,
        as_statement: bool,
    ) -> ExprId {
        if !as_statement {
            self.error(
                crate::codes::CALL_UNKNOWN,
                "a function block invocation is only valid as a statement",
                span,
            );
        }
        let prepared = self.prepare_args(ctx, arguments);
        let formals = self.fb_formals(def);
        let candidates = vec![(def, formals)];
        let Some((_def, formals, bindings)) = self.select_overload(&candidates, &prepared, span)
        else {
            return self.alloc_error_expr(span);
        };
        self.record(callee.syntax(), crate::Res::Def(def));
        self.finish_call(
            ctx,
            CalleeHir::Fb { instance, def },
            &formals,
            &bindings,
            &prepared,
            self.hir.types.error(),
            span,
        )
    }

    fn check_conversion_call(
        &mut self,
        ctx: &mut BodyCtx,
        callee: &Expr,
        from: TyId,
        to: TyId,
        arguments: &[Arg],
        span: Span,
    ) -> ExprId {
        if arguments.len() != 1 {
            self.error(
                crate::codes::CALL_ARITY,
                "a conversion function takes exactly one argument",
                span,
            );
            return self.alloc_error_expr(span);
        }
        let expr = match &arguments[0] {
            Arg::Positional(expr) | Arg::Named(_, expr) => expr.clone(),
            _ => {
                self.error(
                    crate::codes::CALL_ARGUMENT,
                    "a conversion argument must be an input expression",
                    span,
                );
                return self.alloc_error_expr(span);
            }
        };
        let operand = self.check_expr(ctx, &expr, Some(from));
        let operand_ty = self.hir.expr(operand).ty;
        if !self.can_convert(to, operand_ty) {
            self.error(
                crate::codes::CONV_NONE,
                format!(
                    "no conversion from `{}` to `{}`",
                    self.type_name(operand_ty),
                    self.type_name(to)
                ),
                span,
            );
        }
        let operand = self.coerce_expr_to(operand, from, false, span, crate::codes::CONV_NONE);
        self.record(callee.syntax(), crate::Res::Type(to));
        self.hir.alloc_expr(
            ExprKindHir::Cast {
                operand,
                to,
                kind: CastKind::Explicit,
            },
            to,
            span,
        )
    }

    fn check_standard_call(
        &mut self,
        ctx: &mut BodyCtx,
        _callee: &Expr,
        ids: Vec<stc_hir::StdFnId>,
        arguments: &[Arg],
        span: Span,
        _as_statement: bool,
    ) -> ExprId {
        let std_fn = self.hir.std_fns[ids[0].index()].clone();
        let name = std_fn.kind.as_str().to_string();
        let params = std_fn.params.clone();
        let variadic = params.last().is_some_and(|param| param.variadic);
        if arguments.len() < params.len() || (!variadic && arguments.len() > params.len()) {
            self.error(
                crate::codes::CALL_ARITY,
                format!(
                    "`{name}` expects {}{} argument(s), found {}",
                    params.len(),
                    if variadic { " or more" } else { "" },
                    arguments.len()
                ),
                span,
            );
        }
        let mut slots: Vec<Option<(ExprId, TyId, Span)>> = vec![None; params.len()];
        let mut extras: Vec<(ExprId, TyId, Span)> = Vec::new();
        let mut positional = 0;
        for arg in arguments {
            match arg {
                Arg::Positional(expr) | Arg::RefAssign(_, expr) => {
                    let id = self.check_expr(ctx, expr, None);
                    let ty = self.hir.expr(id).ty;
                    let arg_span = expr.token_span();
                    if positional < params.len() {
                        if slots[positional].is_some() {
                            self.error(
                                crate::codes::CALL_ARGUMENT,
                                "argument supplied twice",
                                arg_span,
                            );
                        } else {
                            slots[positional] = Some((id, ty, arg_span));
                        }
                    } else if variadic {
                        extras.push((id, ty, arg_span));
                    } else {
                        self.error(
                            crate::codes::CALL_ARITY,
                            format!("`{name}` has too many arguments"),
                            arg_span,
                        );
                    }
                    positional += 1;
                }
                Arg::Named(name_node, expr) => {
                    let symbol = self.hir.intern(name_node.text());
                    let Some(index) = params.iter().position(|param| {
                        self.hir
                            .interner
                            .get(param.name)
                            .is_some_and(|candidate| candidate == symbol)
                    }) else {
                        self.error(
                            crate::codes::CALL_ARGUMENT,
                            format!("`{name}` has no parameter `{}`", name_node.text()),
                            name_node.token_span(),
                        );
                        continue;
                    };
                    let id = self.check_expr(ctx, expr, None);
                    let ty = self.hir.expr(id).ty;
                    if slots[index].is_some() {
                        self.error(
                            crate::codes::CALL_ARGUMENT,
                            "argument supplied twice",
                            name_node.token_span(),
                        );
                    } else {
                        slots[index] = Some((id, ty, expr.token_span()));
                    }
                }
                Arg::Output(_, _) | Arg::NegatedOutput(_, _) => {
                    self.error(
                        crate::codes::CALL_ARGUMENT,
                        "standard functions have no output parameters",
                        span,
                    );
                }
                Arg::Error(_) => {}
            }
        }
        let mut arg_ids = Vec::new();
        let mut arg_tys = Vec::new();
        let mut arg_spans = Vec::new();
        for (index, param) in params.iter().enumerate() {
            let Some((id, ty, arg_span)) = slots[index] else {
                continue;
            };
            let Some((id, ty)) = self.adapt_generic(id, ty, param.generic, arg_span, &name) else {
                continue;
            };
            arg_ids.push(id);
            arg_tys.push(ty);
            arg_spans.push(arg_span);
        }
        for (id, ty, arg_span) in extras {
            let generic = params
                .last()
                .map_or(GenericKind::Any, |param| param.generic);
            let Some((id, ty)) = self.adapt_generic(id, ty, generic, arg_span, &name) else {
                continue;
            };
            arg_ids.push(id);
            arg_tys.push(ty);
            arg_spans.push(arg_span);
        }
        let Some((result_ty, arg_ids)) =
            self.std_unify(std_fn.kind, arg_ids, arg_tys, arg_spans, span)
        else {
            return self.alloc_error_expr(span);
        };
        let args = arg_ids
            .into_iter()
            .map(|id| ArgHir {
                formal: None,
                field: None,
                direction: ArgDirection::Input,
                value: Some(id),
                target: None,
                negated: false,
                span,
            })
            .collect();
        self.hir.alloc_expr(
            ExprKindHir::Call {
                callee: CalleeHir::StdFunction(ids[0]),
                args,
            },
            result_ty,
            span,
        )
    }

    fn finish_call(
        &mut self,
        _ctx: &mut BodyCtx,
        callee: CalleeHir,
        formals: &[FormalInfo],
        bindings: &[Option<usize>],
        prepared: &[PreparedArg],
        result_ty: TyId,
        span: Span,
    ) -> ExprId {
        let mut args = Vec::new();
        for (index, formal_index) in bindings.iter().enumerate() {
            let Some(formal_index) = formal_index else {
                continue;
            };
            let formal = &formals[*formal_index];
            let prepared = &prepared[index];
            match prepared.kind {
                PreparedKind::Positional | PreparedKind::Named(_) => {
                    let Some(value_id) = prepared.value else {
                        continue;
                    };
                    if formal.direction == ArgDirection::InOut {
                        if !self.is_place_expr(value_id) {
                            self.error(
                                crate::codes::CALL_ARGUMENT,
                                "a `VAR_IN_OUT` argument must be a variable",
                                prepared.span,
                            );
                        }
                        if let Some(value_ty) = prepared.value_ty
                            && self.can_assign(formal.ty, value_ty) == Conv::No
                        {
                            self.error(
                                crate::codes::TYPE_MISMATCH,
                                format!(
                                    "cannot bind `{}` to `VAR_IN_OUT` parameter of type `{}`",
                                    self.type_name(value_ty),
                                    self.type_name(formal.ty)
                                ),
                                prepared.span,
                            );
                        }
                        args.push(ArgHir {
                            formal: formal.param,
                            field: formal.field,
                            direction: ArgDirection::InOut,
                            value: Some(value_id),
                            target: None,
                            negated: false,
                            span: prepared.span,
                        });
                    } else {
                        let value = self.coerce_expr_to(
                            value_id,
                            formal.ty,
                            false,
                            prepared.span,
                            crate::codes::TYPE_MISMATCH,
                        );
                        args.push(ArgHir {
                            formal: formal.param,
                            field: formal.field,
                            direction: ArgDirection::Input,
                            value: Some(value),
                            target: None,
                            negated: false,
                            span: prepared.span,
                        });
                    }
                }
                PreparedKind::Output(_) => {
                    let Some(target) = prepared.target else {
                        continue;
                    };
                    if let Some(target_ty) = prepared.target_ty
                        && self.can_assign(target_ty, formal.ty) == Conv::No
                    {
                        self.error(
                            crate::codes::TYPE_MISMATCH,
                            format!(
                                "output of type `{}` cannot be written to `{}`",
                                self.type_name(formal.ty),
                                self.type_name(target_ty)
                            ),
                            prepared.span,
                        );
                    }
                    if prepared.negated && formal.ty != self.hir.types.bool() {
                        self.error(
                            crate::codes::TYPE_MISMATCH,
                            "`NOT =>` requires a `BOOL` output",
                            prepared.span,
                        );
                    }
                    args.push(ArgHir {
                        formal: formal.param,
                        field: formal.field,
                        direction: ArgDirection::Output,
                        value: None,
                        target: Some(target),
                        negated: prepared.negated,
                        span: prepared.span,
                    });
                }
                PreparedKind::Error => {}
            }
        }
        self.hir
            .alloc_expr(ExprKindHir::Call { callee, args }, result_ty, span)
    }

    fn prepare_args(&mut self, ctx: &mut BodyCtx, arguments: &[Arg]) -> Vec<PreparedArg> {
        let mut prepared = Vec::new();
        for arg in arguments {
            let span = arg.syntax().token_span();
            let prepared_arg = match arg {
                Arg::Positional(expr) => {
                    let id = self.check_expr(ctx, expr, None);
                    PreparedArg {
                        kind: PreparedKind::Positional,
                        value: Some(id),
                        value_ty: Some(self.hir.expr(id).ty),
                        target: None,
                        target_ty: None,
                        negated: false,
                        span,
                    }
                }
                Arg::Named(name, expr) | Arg::RefAssign(name, expr) => {
                    let symbol = self.hir.intern(name.text());
                    let id = self.check_expr(ctx, expr, None);
                    PreparedArg {
                        kind: PreparedKind::Named(symbol),
                        value: Some(id),
                        value_ty: Some(self.hir.expr(id).ty),
                        target: None,
                        target_ty: None,
                        negated: false,
                        span,
                    }
                }
                Arg::Output(source, target) => {
                    self.prepare_output(ctx, source, target, false, span)
                }
                Arg::NegatedOutput(source, target) => {
                    self.prepare_output(ctx, source, target, true, span)
                }
                Arg::Error(_) => PreparedArg {
                    kind: PreparedKind::Error,
                    value: None,
                    value_ty: None,
                    target: None,
                    target_ty: None,
                    negated: false,
                    span,
                },
            };
            prepared.push(prepared_arg);
        }
        prepared
    }

    fn prepare_output(
        &mut self,
        ctx: &mut BodyCtx,
        source: &Expr,
        target: &Expr,
        negated: bool,
        span: Span,
    ) -> PreparedArg {
        let Some(name) = self.output_name(source) else {
            self.error(
                crate::codes::CALL_ARGUMENT,
                "output binding requires a parameter name on the left of `=>`",
                span,
            );
            return PreparedArg {
                kind: PreparedKind::Error,
                value: None,
                value_ty: None,
                target: None,
                target_ty: None,
                negated,
                span,
            };
        };
        let target_id = self.check_place(ctx, target, span);
        let target_ty = self.hir.expr(target_id).ty;
        PreparedArg {
            kind: PreparedKind::Output(name),
            value: None,
            value_ty: None,
            target: Some(target_id),
            target_ty: Some(target_ty),
            negated,
            span,
        }
    }

    fn output_name(&mut self, expr: &Expr) -> Option<Symbol> {
        match expr {
            Expr::Path(path) if path.prefix().is_none() && path.segments().len() == 1 => {
                let name = path.segments().first()?.clone();
                Some(self.hir.intern(name.text()))
            }
            Expr::Unary(unary)
                if unary
                    .operator()
                    .is_some_and(|token| token.kind() == stc_syntax::SyntaxKind::KwNot) =>
            {
                let operand = unary.operand()?;
                self.output_name(&operand)
            }
            _ => None,
        }
    }

    fn select_overload(
        &mut self,
        candidates: &[(DefId, Vec<FormalInfo>)],
        prepared: &[PreparedArg],
        span: Span,
    ) -> Option<(DefId, Vec<FormalInfo>, Vec<Option<usize>>)> {
        let mut best: Option<(usize, DefId, Vec<FormalInfo>, Vec<Option<usize>>)> = None;
        let mut ambiguous = false;
        for (def, formals) in candidates {
            let Some((bindings, score)) = self.bind_args(formals, prepared) else {
                continue;
            };
            match &best {
                None => best = Some((score, *def, formals.clone(), bindings)),
                Some((best_score, best_def, _, _)) => {
                    if score < *best_score {
                        best = Some((score, *def, formals.clone(), bindings));
                        ambiguous = false;
                    } else if score == *best_score && *best_def != *def {
                        ambiguous = true;
                    }
                }
            }
        }
        if ambiguous {
            self.error(
                crate::codes::CALL_AMBIGUOUS,
                "the call is ambiguous between overloads",
                span,
            );
            return None;
        }
        if best.is_none() {
            self.report_bind_failure(candidates, prepared, span);
            return None;
        }
        best.map(|(_, def, formals, bindings)| (def, formals, bindings))
    }

    fn report_bind_failure(
        &mut self,
        candidates: &[(DefId, Vec<FormalInfo>)],
        prepared: &[PreparedArg],
        span: Span,
    ) {
        let mut reported = false;
        let mut seen: Vec<Symbol> = Vec::new();
        for arg in prepared {
            match &arg.kind {
                PreparedKind::Named(symbol) | PreparedKind::Output(symbol) => {
                    if seen.contains(symbol) {
                        self.error(
                            crate::codes::CALL_ARGUMENT,
                            "argument supplied twice",
                            arg.span,
                        );
                        reported = true;
                        continue;
                    }
                    seen.push(*symbol);
                    let known = candidates
                        .iter()
                        .any(|(_, formals)| formals.iter().any(|formal| formal.name == *symbol));
                    if !known {
                        self.error(
                            crate::codes::CALL_ARGUMENT,
                            format!("no parameter named `{}`", self.symbol_text(*symbol)),
                            arg.span,
                        );
                        reported = true;
                    } else if matches!(arg.kind, PreparedKind::Output(_))
                        && !candidates.iter().any(|(_, formals)| {
                            formals.iter().any(|formal| {
                                formal.name == *symbol && formal.direction == ArgDirection::Output
                            })
                        })
                    {
                        self.error(
                            crate::codes::CALL_ARGUMENT,
                            format!(
                                "`{}` is not an output parameter; use `:=`",
                                self.symbol_text(*symbol)
                            ),
                            arg.span,
                        );
                        reported = true;
                    }
                }
                PreparedKind::Error => reported = true,
                PreparedKind::Positional => {}
            }
        }
        if !reported
            && prepared
                .iter()
                .all(|arg| matches!(arg.kind, PreparedKind::Named(_)))
            && let Some((_, formals)) = candidates.first()
        {
            for formal in formals {
                if formal.required && !seen.contains(&formal.name) {
                    self.error(
                        crate::codes::CALL_ARITY,
                        format!(
                            "missing required argument `{}`",
                            self.symbol_text(formal.name)
                        ),
                        span,
                    );
                    reported = true;
                }
            }
        }
        if !reported {
            self.error(
                crate::codes::CALL_ARITY,
                "no overload matches these arguments",
                span,
            );
        }
    }

    fn bind_args(
        &mut self,
        formals: &[FormalInfo],
        prepared: &[PreparedArg],
    ) -> Option<(Vec<Option<usize>>, usize)> {
        let mut bindings = vec![None; prepared.len()];
        let mut used = vec![false; formals.len()];
        let mut next_positional = 0;
        let mut score = 0;
        for (index, arg) in prepared.iter().enumerate() {
            match &arg.kind {
                PreparedKind::Positional => {
                    let mut found = None;
                    while next_positional < formals.len() {
                        let formal = &formals[next_positional];
                        if !used[next_positional]
                            && matches!(formal.direction, ArgDirection::Input | ArgDirection::InOut)
                        {
                            found = Some(next_positional);
                            break;
                        }
                        next_positional += 1;
                    }
                    let Some(formal_index) = found else {
                        return None;
                    };
                    used[formal_index] = true;
                    bindings[index] = Some(formal_index);
                    next_positional = formal_index + 1;
                    score += self.score_input(&formals[formal_index], arg)?;
                }
                PreparedKind::Named(symbol) => {
                    let formal_index = formals.iter().position(|formal| formal.name == *symbol)?;
                    if used[formal_index]
                        || !matches!(
                            formals[formal_index].direction,
                            ArgDirection::Input | ArgDirection::InOut
                        )
                    {
                        return None;
                    }
                    used[formal_index] = true;
                    bindings[index] = Some(formal_index);
                    score += self.score_input(&formals[formal_index], arg)?;
                }
                PreparedKind::Output(symbol) => {
                    let formal_index = formals.iter().position(|formal| formal.name == *symbol)?;
                    if used[formal_index] || formals[formal_index].direction != ArgDirection::Output
                    {
                        return None;
                    }
                    used[formal_index] = true;
                    bindings[index] = Some(formal_index);
                    let target_ty = arg.target_ty?;
                    if self.can_assign(target_ty, formals[formal_index].ty) == Conv::No {
                        return None;
                    }
                    if arg.negated && formals[formal_index].ty != self.hir.types.bool() {
                        return None;
                    }
                }
                PreparedKind::Error => return None,
            }
        }
        for (index, formal) in formals.iter().enumerate() {
            if formal.required && !used[index] {
                return None;
            }
        }
        Some((bindings, score))
    }

    fn score_input(&self, formal: &FormalInfo, arg: &PreparedArg) -> Option<usize> {
        let ty = arg.value_ty?;
        if formal.direction == ArgDirection::InOut
            && !arg.value.is_some_and(|id| self.is_place_expr(id))
        {
            return None;
        }
        match self.can_assign(formal.ty, ty) {
            Conv::Exact => Some(0),
            Conv::Implicit => Some(1),
            Conv::Explicit | Conv::No => {
                if arg
                    .value
                    .is_some_and(|id| self.literal_can_adapt(id, formal.ty))
                {
                    Some(1)
                } else {
                    None
                }
            }
        }
    }

    fn function_formals(&self, def: DefId) -> Vec<FormalInfo> {
        let params = match &self.hir.defs[def.index()].kind {
            DefKind::Function { params, .. } | DefKind::Method { params, .. } => params,
            _ => return Vec::new(),
        };
        params
            .iter()
            .map(|id| {
                let param = &self.hir.params[id.index()];
                let required = param.direction != ArgDirection::Output
                    && !(param.direction == ArgDirection::Input && param.default.is_some());
                FormalInfo {
                    param: Some(*id),
                    field: None,
                    name: param.name,
                    ty: param.ty,
                    direction: param.direction,
                    required,
                }
            })
            .collect()
    }

    fn fb_formals(&self, def: DefId) -> Vec<FormalInfo> {
        let fields = match &self.hir.defs[def.index()].kind {
            DefKind::FunctionBlock { fields, .. } => fields,
            _ => return Vec::new(),
        };
        fields
            .iter()
            .filter_map(|id| {
                let field = &self.hir.fields[id.index()];
                let direction = field.direction()?;
                Some(FormalInfo {
                    param: None,
                    field: Some(*id),
                    name: field.name,
                    ty: field.ty,
                    direction,
                    required: direction == ArgDirection::InOut,
                })
            })
            .collect()
    }

    fn adapt_generic(
        &mut self,
        id: ExprId,
        ty: TyId,
        generic: GenericKind,
        span: Span,
        function: &str,
    ) -> Option<(ExprId, TyId)> {
        if ty_matches_generic(self.hir.types.get(ty), generic) {
            return Some((id, ty));
        }
        if generic == GenericKind::AnyReal && self.is_numeric(ty) {
            let lreal = matches!(
                self.hir.types.get(self.scalar_base(ty)),
                Ty::Real(stc_hir::RealTy::LReal)
            );
            let target = if lreal {
                self.hir.types.real(stc_hir::RealTy::LReal)
            } else {
                self.hir.types.real(stc_hir::RealTy::Real)
            };
            let id = self.coerce_expr_to(id, target, false, span, crate::codes::TYPE_MISMATCH);
            return Some((id, target));
        }
        if generic == GenericKind::AnyNum && self.is_numeric(ty) {
            return Some((id, ty));
        }
        if generic == GenericKind::AnyMagnitude && self.is_numeric_or_time(ty) {
            return Some((id, ty));
        }
        self.error(
            crate::codes::TYPE_MISMATCH,
            format!(
                "argument of `{function}` must be `{}`, found `{}`",
                generic.as_str(),
                self.type_name(ty)
            ),
            span,
        );
        None
    }

    fn std_unify(
        &mut self,
        kind: stc_hir::StdFnKind,
        ids: Vec<ExprId>,
        tys: Vec<TyId>,
        spans: Vec<Span>,
        span: Span,
    ) -> Option<(TyId, Vec<ExprId>)> {
        if ids.is_empty() {
            return Some((self.hir.types.error(), ids));
        }
        let mut ids = ids;
        let result;
        match kind {
            stc_hir::StdFnKind::Abs
            | stc_hir::StdFnKind::Move
            | stc_hir::StdFnKind::Not
            | stc_hir::StdFnKind::Shl
            | stc_hir::StdFnKind::Shr
            | stc_hir::StdFnKind::Rol
            | stc_hir::StdFnKind::Ror => {
                result = tys[0];
                if matches!(
                    kind,
                    stc_hir::StdFnKind::Shl
                        | stc_hir::StdFnKind::Shr
                        | stc_hir::StdFnKind::Rol
                        | stc_hir::StdFnKind::Ror
                ) && ids.len() > 1
                {
                    let dint = self.hir.types.int(true, 32);
                    ids[1] = self.coerce_expr_to(
                        ids[1],
                        dint,
                        false,
                        spans[1],
                        crate::codes::TYPE_MISMATCH,
                    );
                }
            }
            stc_hir::StdFnKind::Sqrt
            | stc_hir::StdFnKind::Ln
            | stc_hir::StdFnKind::Log
            | stc_hir::StdFnKind::Exp
            | stc_hir::StdFnKind::Sin
            | stc_hir::StdFnKind::Cos
            | stc_hir::StdFnKind::Tan
            | stc_hir::StdFnKind::Asin
            | stc_hir::StdFnKind::Acos
            | stc_hir::StdFnKind::Atan
            | stc_hir::StdFnKind::Atan2
            | stc_hir::StdFnKind::Expt => {
                let lreal = tys.iter().any(|ty| {
                    matches!(
                        self.hir.types.get(self.scalar_base(*ty)),
                        Ty::Real(stc_hir::RealTy::LReal)
                    )
                });
                result = if lreal {
                    self.hir.types.real(stc_hir::RealTy::LReal)
                } else {
                    self.hir.types.real(stc_hir::RealTy::Real)
                };
                for index in 0..ids.len() {
                    ids[index] = self.coerce_expr_to(
                        ids[index],
                        result,
                        false,
                        spans[index],
                        crate::codes::TYPE_MISMATCH,
                    );
                }
            }
            stc_hir::StdFnKind::Trunc => {
                result = self.hir.types.int(true, 32);
            }
            stc_hir::StdFnKind::Add
            | stc_hir::StdFnKind::Sub
            | stc_hir::StdFnKind::Mul
            | stc_hir::StdFnKind::Div
            | stc_hir::StdFnKind::Mod
            | stc_hir::StdFnKind::Max
            | stc_hir::StdFnKind::Min => {
                let any_real = tys
                    .iter()
                    .any(|ty| matches!(self.hir.types.get(self.scalar_base(*ty)), Ty::Real(_)));
                if any_real {
                    let lreal = tys.iter().any(|ty| {
                        matches!(
                            self.hir.types.get(self.scalar_base(*ty)),
                            Ty::Real(stc_hir::RealTy::LReal)
                        )
                    });
                    result = if lreal {
                        self.hir.types.real(stc_hir::RealTy::LReal)
                    } else {
                        self.hir.types.real(stc_hir::RealTy::Real)
                    };
                } else {
                    result = self.promoted_int(&tys);
                }
                for index in 0..ids.len() {
                    ids[index] = self.coerce_expr_to(
                        ids[index],
                        result,
                        false,
                        spans[index],
                        crate::codes::TYPE_MISMATCH,
                    );
                }
            }
            stc_hir::StdFnKind::And | stc_hir::StdFnKind::Or | stc_hir::StdFnKind::Xor => {
                let all_bool = tys
                    .iter()
                    .all(|ty| matches!(self.hir.types.get(self.scalar_base(*ty)), Ty::Bool));
                if all_bool {
                    result = self.hir.types.bool();
                } else {
                    let width = tys
                        .iter()
                        .filter_map(|ty| match self.hir.types.get(self.scalar_base(*ty)) {
                            Ty::BitString(width) => Some(*width),
                            _ => None,
                        })
                        .max()
                        .unwrap_or(8);
                    result = self.hir.types.bit_string(width);
                    for index in 0..ids.len() {
                        ids[index] = self.coerce_expr_to(
                            ids[index],
                            result,
                            false,
                            spans[index],
                            crate::codes::TYPE_MISMATCH,
                        );
                    }
                }
            }
            stc_hir::StdFnKind::Sel => {
                let bool_ty = self.hir.types.bool();
                if ids.len() == 3 {
                    ids[0] = self.coerce_expr_to(
                        ids[0],
                        bool_ty,
                        false,
                        spans[0],
                        crate::codes::TYPE_MISMATCH,
                    );
                    result = tys[1];
                    ids[2] = self.coerce_expr_to(
                        ids[2],
                        result,
                        false,
                        spans[2],
                        crate::codes::TYPE_MISMATCH,
                    );
                } else {
                    result = self.hir.types.error();
                }
            }
            stc_hir::StdFnKind::Limit => {
                if ids.len() == 3 {
                    result = tys[1];
                    ids[0] = self.coerce_expr_to(
                        ids[0],
                        result,
                        false,
                        spans[0],
                        crate::codes::TYPE_MISMATCH,
                    );
                    ids[2] = self.coerce_expr_to(
                        ids[2],
                        result,
                        false,
                        spans[2],
                        crate::codes::TYPE_MISMATCH,
                    );
                } else {
                    result = self.hir.types.error();
                }
            }
            stc_hir::StdFnKind::Mux => {
                if ids.len() >= 2 {
                    let dint = self.hir.types.int(true, 32);
                    ids[0] = self.coerce_expr_to(
                        ids[0],
                        dint,
                        false,
                        spans[0],
                        crate::codes::TYPE_MISMATCH,
                    );
                    result = tys[1];
                    for index in 2..ids.len() {
                        ids[index] = self.coerce_expr_to(
                            ids[index],
                            result,
                            false,
                            spans[index],
                            crate::codes::TYPE_MISMATCH,
                        );
                    }
                } else {
                    result = self.hir.types.error();
                }
            }
            stc_hir::StdFnKind::Gt
            | stc_hir::StdFnKind::Ge
            | stc_hir::StdFnKind::Eq
            | stc_hir::StdFnKind::Le
            | stc_hir::StdFnKind::Lt
            | stc_hir::StdFnKind::Ne => {
                if ids.len() == 2 {
                    if self.can_assign(tys[0], tys[1]) != Conv::No {
                        ids[1] = self.coerce_expr_to(
                            ids[1],
                            tys[0],
                            false,
                            spans[1],
                            crate::codes::TYPE_MISMATCH,
                        );
                    } else if self.can_assign(tys[1], tys[0]) != Conv::No {
                        ids[0] = self.coerce_expr_to(
                            ids[0],
                            tys[1],
                            false,
                            spans[0],
                            crate::codes::TYPE_MISMATCH,
                        );
                    } else {
                        self.error(
                            crate::codes::TYPE_INVALID_OPERAND,
                            "comparison arguments are not comparable",
                            span,
                        );
                    }
                }
                result = self.hir.types.bool();
            }
            stc_hir::StdFnKind::Len
            | stc_hir::StdFnKind::LenMax
            | stc_hir::StdFnKind::LenCodeUnit
            | stc_hir::StdFnKind::Find => {
                result = self.hir.types.int(true, 32);
                if matches!(kind, stc_hir::StdFnKind::Find) && ids.len() == 2 {
                    ids[1] = self.coerce_expr_to(
                        ids[1],
                        tys[0],
                        false,
                        spans[1],
                        crate::codes::TYPE_MISMATCH,
                    );
                }
            }
            stc_hir::StdFnKind::Left | stc_hir::StdFnKind::Right => {
                result = tys[0];
                if ids.len() > 1 {
                    let dint = self.hir.types.int(true, 32);
                    ids[1] = self.coerce_expr_to(
                        ids[1],
                        dint,
                        false,
                        spans[1],
                        crate::codes::TYPE_MISMATCH,
                    );
                }
            }
            stc_hir::StdFnKind::Mid | stc_hir::StdFnKind::Delete => {
                result = tys[0];
                let dint = self.hir.types.int(true, 32);
                for index in 1..ids.len() {
                    ids[index] = self.coerce_expr_to(
                        ids[index],
                        dint,
                        false,
                        spans[index],
                        crate::codes::TYPE_MISMATCH,
                    );
                }
            }
            stc_hir::StdFnKind::Concat => {
                result = tys[0];
                for index in 1..ids.len() {
                    ids[index] = self.coerce_expr_to(
                        ids[index],
                        result,
                        false,
                        spans[index],
                        crate::codes::TYPE_MISMATCH,
                    );
                }
            }
            stc_hir::StdFnKind::Insert => {
                result = tys[0];
                if ids.len() > 1 {
                    ids[1] = self.coerce_expr_to(
                        ids[1],
                        result,
                        false,
                        spans[1],
                        crate::codes::TYPE_MISMATCH,
                    );
                }
                if ids.len() > 2 {
                    let dint = self.hir.types.int(true, 32);
                    ids[2] = self.coerce_expr_to(
                        ids[2],
                        dint,
                        false,
                        spans[2],
                        crate::codes::TYPE_MISMATCH,
                    );
                }
            }
            stc_hir::StdFnKind::Replace => {
                result = tys[0];
                if ids.len() > 1 {
                    ids[1] = self.coerce_expr_to(
                        ids[1],
                        result,
                        false,
                        spans[1],
                        crate::codes::TYPE_MISMATCH,
                    );
                }
                let dint = self.hir.types.int(true, 32);
                for index in 2..ids.len() {
                    ids[index] = self.coerce_expr_to(
                        ids[index],
                        dint,
                        false,
                        spans[index],
                        crate::codes::TYPE_MISMATCH,
                    );
                }
            }
        }
        Some((result, ids))
    }

    fn promoted_int(&self, tys: &[TyId]) -> TyId {
        let mut width = 8u8;
        let mut signed = false;
        for ty in tys {
            if let Ty::Int(int) = self.hir.types.get(self.scalar_base(*ty)) {
                width = width.max(int.width);
                signed |= int.signed;
            }
        }
        self.hir.types.int(signed, width)
    }

    fn literal_can_adapt(&self, id: ExprId, target: TyId) -> bool {
        let ExprKindHir::Literal(literal) = &self.hir.expr(id).kind else {
            return false;
        };
        self.literal_as(&literal.value, target).is_some()
    }

    fn resolve_callee(&mut self, ctx: &mut BodyCtx, callee: &Expr) -> CalleeKind {
        match callee {
            Expr::Path(path) => self.resolve_path_callee(ctx, path),
            Expr::Member(member) => {
                let Some(base) = member.base() else {
                    self.error(
                        crate::codes::CALL_UNKNOWN,
                        "invalid method receiver",
                        member.token_span(),
                    );
                    return CalleeKind::Invalid;
                };
                let receiver = self.check_expr(ctx, &base, None);
                let ty = self.hir.expr(receiver).ty;
                let Some(name) = member.member() else {
                    self.error(
                        crate::codes::CALL_UNKNOWN,
                        "invalid method name",
                        member.token_span(),
                    );
                    return CalleeKind::Invalid;
                };
                self.call_method_on_value(ctx, receiver, ty, &name, member.token_span(), false)
            }
            _ => {
                self.error(
                    crate::codes::CALL_UNKNOWN,
                    "this expression is not callable",
                    callee.token_span(),
                );
                CalleeKind::Invalid
            }
        }
    }

    fn resolve_path_callee(&mut self, ctx: &mut BodyCtx, path: &PathExpr) -> CalleeKind {
        let span = path.token_span();
        let segments = path.segments();
        match path.prefix() {
            Some(token) if token.kind() == stc_syntax::SyntaxKind::KwSuper => {
                if segments.is_empty() {
                    return match ctx.super_base {
                        Some(base)
                            if matches!(
                                &self.hir.defs[base.index()].kind,
                                DefKind::FunctionBlock { .. }
                            ) =>
                        {
                            CalleeKind::SuperBody { base }
                        }
                        Some(_) => {
                            self.error(
                                crate::codes::NAME_INVALID_CONTEXT,
                                "`SUPER()` requires a base function block",
                                span,
                            );
                            CalleeKind::Invalid
                        }
                        None => {
                            self.error(
                                crate::codes::NAME_INVALID_CONTEXT,
                                "`SUPER()` requires a base function block",
                                span,
                            );
                            CalleeKind::Invalid
                        }
                    };
                }
                let Some(base) = ctx.super_base else {
                    self.error(
                        crate::codes::NAME_INVALID_CONTEXT,
                        "`SUPER` requires a base type",
                        span,
                    );
                    return CalleeKind::Invalid;
                };
                let Some(receiver) = self.make_this(ctx, span) else {
                    return CalleeKind::Invalid;
                };
                let base_ty = self.hir.types.intern(Ty::Named(base));
                let Some((receiver, _ty)) = self.project_receiver(
                    ctx,
                    receiver,
                    base_ty,
                    &segments[..segments.len() - 1],
                    span,
                ) else {
                    return CalleeKind::Invalid;
                };
                let name = &segments[segments.len() - 1];
                self.call_method_on_value(ctx, receiver, base_ty, name, span, true)
            }
            Some(token) if token.kind() == stc_syntax::SyntaxKind::KwThis => {
                if segments.is_empty() {
                    self.error(
                        crate::codes::NAME_INVALID_CONTEXT,
                        "`THIS` is not callable",
                        span,
                    );
                    return CalleeKind::Invalid;
                }
                if ctx.this_ty.is_none() {
                    self.error(
                        crate::codes::NAME_INVALID_CONTEXT,
                        "`THIS` is only valid in a method or FB body",
                        span,
                    );
                    return CalleeKind::Invalid;
                }
                let Some(receiver) = self.make_this(ctx, span) else {
                    return CalleeKind::Invalid;
                };
                let owner_ty = ctx.this_ty.unwrap();
                let Some((receiver, ty)) = self.project_receiver(
                    ctx,
                    receiver,
                    owner_ty,
                    &segments[..segments.len() - 1],
                    span,
                ) else {
                    return CalleeKind::Invalid;
                };
                let name = &segments[segments.len() - 1];
                self.call_method_on_value(ctx, receiver, ty, name, span, false)
            }
            _ => self.resolve_plain_callee(ctx, path, &segments),
        }
    }

    fn resolve_plain_callee(
        &mut self,
        ctx: &mut BodyCtx,
        path: &PathExpr,
        segments: &[Name],
    ) -> CalleeKind {
        let span = path.token_span();
        if segments.is_empty() {
            self.error(crate::codes::CALL_UNKNOWN, "empty callee", span);
            return CalleeKind::Invalid;
        }
        if segments.len() == 1 {
            let name = &segments[0];
            let symbol = self.hir.intern(name.text());
            if let Some((value, ty)) = self.silent_value(ctx, symbol, name.token_span()) {
                if let Some(def) = self.fb_def_of_ty(ty) {
                    return CalleeKind::Fb {
                        instance: value,
                        def,
                    };
                }
                self.error(
                    crate::codes::CALL_UNKNOWN,
                    format!("value of type `{}` is not callable", self.type_name(ty)),
                    span,
                );
                return CalleeKind::Invalid;
            }
            if let Some(owner) = ctx.owner {
                let owner_ty = self.hir.types.intern(Ty::Named(owner));
                let methods = self.methods_named(owner_ty, symbol);
                if !methods.is_empty() {
                    let Some(receiver) = self.make_this(ctx, span) else {
                        return CalleeKind::Invalid;
                    };
                    return CalleeKind::Method {
                        receiver,
                        methods,
                        interface: None,
                        is_super: false,
                    };
                }
            }
            match self.lookup_scope(ctx.scope, symbol) {
                Lookup::Entries(defs) => {
                    let functions: Vec<DefId> = defs
                        .iter()
                        .copied()
                        .filter(|def| {
                            matches!(&self.hir.defs[def.index()].kind, DefKind::Function { .. })
                        })
                        .collect();
                    if !functions.is_empty() {
                        return CalleeKind::Functions(functions);
                    }
                    if defs.len() == 1 {
                        let def = defs[0];
                        match &self.hir.defs[def.index()].kind {
                            DefKind::Global { ty, .. } => {
                                let ty = *ty;
                                let value = self.hir.alloc_expr(
                                    ExprKindHir::Place(PlaceHir::Global(def)),
                                    ty,
                                    span,
                                );
                                if let Some(fb) = self.fb_def_of_ty(ty) {
                                    return CalleeKind::Fb {
                                        instance: value,
                                        def: fb,
                                    };
                                }
                                self.error(
                                    crate::codes::CALL_UNKNOWN,
                                    "a variable is not callable",
                                    span,
                                );
                                return CalleeKind::Invalid;
                            }
                            DefKind::FunctionBlock { .. } => {
                                self.error(
                                    crate::codes::CALL_UNKNOWN,
                                    "a function block type is not an instance",
                                    span,
                                );
                                return CalleeKind::Invalid;
                            }
                            DefKind::Namespace { .. } => {
                                self.error(
                                    crate::codes::CALL_UNKNOWN,
                                    "a namespace is not callable",
                                    span,
                                );
                                return CalleeKind::Invalid;
                            }
                            _ => {
                                self.error(
                                    crate::codes::CALL_UNKNOWN,
                                    "this name is not callable",
                                    span,
                                );
                                return CalleeKind::Invalid;
                            }
                        }
                    }
                    self.error(crate::codes::CALL_UNKNOWN, "ambiguous callee", span);
                    CalleeKind::Invalid
                }
                Lookup::Constant(_) => {
                    self.error(
                        crate::codes::CALL_UNKNOWN,
                        "a constant is not callable",
                        span,
                    );
                    CalleeKind::Invalid
                }
                Lookup::Ambiguous(spans) => {
                    let text = path.text().to_string();
                    self.report_ambiguous(&text, span, &spans);
                    CalleeKind::Invalid
                }
                Lookup::NotFound => {
                    if let Some((from, to)) = self.conversion_from_symbol(symbol) {
                        return CalleeKind::Conversion { from, to };
                    }
                    if let Some(std) = self.std_fns.get(&symbol) {
                        return CalleeKind::Standard(std.clone());
                    }
                    if self.std_fbs.contains_key(&symbol) {
                        self.error(
                            crate::codes::CALL_UNKNOWN,
                            "a standard function block type is not an instance",
                            span,
                        );
                        return CalleeKind::Invalid;
                    }
                    self.unknown_name(name.text(), span);
                    CalleeKind::Invalid
                }
            }
        } else {
            if let Some(kind) = self.qualified_callee(ctx, path, segments) {
                return kind;
            }
            let first = &segments[0];
            let symbol = self.hir.intern(first.text());
            let Some((mut receiver, mut ty)) = self.silent_value(ctx, symbol, first.token_span())
            else {
                self.unknown_name(first.text(), span);
                return CalleeKind::Invalid;
            };
            let last = segments.len() - 1;
            for segment in &segments[1..last] {
                let symbol = self.hir.intern(segment.text());
                let Some(field) = self.field_lookup(ty, symbol) else {
                    self.member_error(receiver, ty, segment, span);
                    return CalleeKind::Invalid;
                };
                let field_data = self.hir.field(field).clone();
                self.check_field_access(field, ctx.owner, span);
                receiver = self.hir.alloc_expr(
                    ExprKindHir::Place(PlaceHir::Field {
                        base: receiver,
                        field,
                    }),
                    field_data.ty,
                    span,
                );
                ty = field_data.ty;
            }
            self.call_method_on_value(ctx, receiver, ty, &segments[last], span, false)
        }
    }

    fn qualified_callee(
        &mut self,
        ctx: &mut BodyCtx,
        path: &PathExpr,
        segments: &[Name],
    ) -> Option<CalleeKind> {
        let span = path.token_span();
        let first = &segments[0];
        let symbol = self.hir.intern(first.text());
        let defs = match self.lookup_scope(ctx.scope, symbol) {
            Lookup::Entries(defs) => defs,
            Lookup::Ambiguous(spans) => {
                let text = path.text().to_string();
                self.report_ambiguous(&text, span, &spans);
                return Some(CalleeKind::Invalid);
            }
            _ => return None,
        };
        let mut namespace = defs
            .iter()
            .copied()
            .find(|def| crate::resolve::is_namespace(&self.hir.defs[def.index()].kind))?;
        for index in 1..segments.len() {
            let segment = &segments[index];
            let symbol = self.hir.intern(segment.text());
            let last = index + 1 == segments.len();
            match self.lookup_scope(namespace, symbol) {
                Lookup::Entries(defs) => {
                    if last {
                        let functions: Vec<DefId> = defs
                            .iter()
                            .copied()
                            .filter(|def| {
                                matches!(&self.hir.defs[def.index()].kind, DefKind::Function { .. })
                            })
                            .collect();
                        if !functions.is_empty() {
                            return Some(CalleeKind::Functions(functions));
                        }
                        if defs.len() == 1 {
                            let def = defs[0];
                            match &self.hir.defs[def.index()].kind {
                                DefKind::Global { ty, .. } => {
                                    let ty = *ty;
                                    let value = self.hir.alloc_expr(
                                        ExprKindHir::Place(PlaceHir::Global(def)),
                                        ty,
                                        span,
                                    );
                                    if let Some(fb) = self.fb_def_of_ty(ty) {
                                        return Some(CalleeKind::Fb {
                                            instance: value,
                                            def: fb,
                                        });
                                    }
                                    self.error(
                                        crate::codes::CALL_UNKNOWN,
                                        "a variable is not callable",
                                        span,
                                    );
                                    return Some(CalleeKind::Invalid);
                                }
                                DefKind::FunctionBlock { .. } => {
                                    self.error(
                                        crate::codes::CALL_UNKNOWN,
                                        "a function block type is not an instance",
                                        span,
                                    );
                                    return Some(CalleeKind::Invalid);
                                }
                                _ => {
                                    self.error(
                                        crate::codes::CALL_UNKNOWN,
                                        "qualified name is not callable",
                                        span,
                                    );
                                    return Some(CalleeKind::Invalid);
                                }
                            }
                        }
                        self.error(crate::codes::CALL_UNKNOWN, "ambiguous callee", span);
                        return Some(CalleeKind::Invalid);
                    }
                    if defs.len() == 1
                        && crate::resolve::is_namespace(&self.hir.defs[defs[0].index()].kind)
                    {
                        namespace = defs[0];
                        continue;
                    }
                    if defs.len() == 1 {
                        let def = defs[0];
                        let DefKind::Global { ty, .. } = &self.hir.defs[def.index()].kind else {
                            return Some(CalleeKind::Invalid);
                        };
                        let ty = *ty;
                        let mut receiver = self.hir.alloc_expr(
                            ExprKindHir::Place(PlaceHir::Global(def)),
                            ty,
                            span,
                        );
                        let mut current_ty = ty;
                        let last_index = segments.len() - 1;
                        for field_name in &segments[index + 1..last_index] {
                            let field_symbol = self.hir.intern(field_name.text());
                            let Some(field) = self.field_lookup(current_ty, field_symbol) else {
                                self.member_error(receiver, current_ty, field_name, span);
                                return Some(CalleeKind::Invalid);
                            };
                            let field_data = self.hir.field(field).clone();
                            self.check_field_access(field, ctx.owner, span);
                            receiver = self.hir.alloc_expr(
                                ExprKindHir::Place(PlaceHir::Field {
                                    base: receiver,
                                    field,
                                }),
                                field_data.ty,
                                span,
                            );
                            current_ty = field_data.ty;
                        }
                        return Some(self.call_method_on_value(
                            ctx,
                            receiver,
                            current_ty,
                            &segments[last_index],
                            span,
                            false,
                        ));
                    }
                    return Some(CalleeKind::Invalid);
                }
                Lookup::Constant(_) => {
                    if last {
                        self.error(
                            crate::codes::CALL_UNKNOWN,
                            "a constant is not callable",
                            span,
                        );
                        return Some(CalleeKind::Invalid);
                    }
                    return None;
                }
                Lookup::Ambiguous(spans) => {
                    let text = path.text().to_string();
                    self.report_ambiguous(&text, span, &spans);
                    return Some(CalleeKind::Invalid);
                }
                Lookup::NotFound => return None,
            }
        }
        Some(CalleeKind::Invalid)
    }

    fn call_method_on_value(
        &mut self,
        _ctx: &mut BodyCtx,
        receiver: ExprId,
        ty: TyId,
        name: &Name,
        span: Span,
        is_super: bool,
    ) -> CalleeKind {
        let symbol = self.hir.intern(name.text());
        if let Some(field) = self.field_lookup(ty, symbol) {
            let field_data = self.hir.field(field).clone();
            if let Some(def) = self.fb_def_of_ty(field_data.ty) {
                let place = self.hir.alloc_expr(
                    ExprKindHir::Place(PlaceHir::Field {
                        base: receiver,
                        field,
                    }),
                    field_data.ty,
                    span,
                );
                return CalleeKind::Fb {
                    instance: place,
                    def,
                };
            }
            self.error(crate::codes::CALL_UNKNOWN, "a field is not callable", span);
            return CalleeKind::Invalid;
        }
        let methods = self.methods_named(ty, symbol);
        if methods.is_empty() {
            self.error(
                crate::codes::NAME_UNKNOWN,
                format!(
                    "type `{}` has no method `{}`",
                    self.type_name(ty),
                    name.text()
                ),
                span,
            );
            return CalleeKind::Invalid;
        }
        let interface = match self.hir.types.get(ty) {
            Ty::Named(def)
                if matches!(&self.hir.defs[def.index()].kind, DefKind::Interface { .. }) =>
            {
                Some(*def)
            }
            _ => None,
        };
        CalleeKind::Method {
            receiver,
            methods,
            interface,
            is_super,
        }
    }

    fn silent_value(
        &mut self,
        ctx: &mut BodyCtx,
        symbol: Symbol,
        span: Span,
    ) -> Option<(ExprId, TyId)> {
        if let Some(local) = ctx.locals.get(&symbol) {
            let data = self.hir.local(*local).clone();
            let ty = data.external_target.map_or(data.ty, |global| {
                match &self.hir.defs[global.index()].kind {
                    DefKind::Global { ty, .. } => *ty,
                    _ => data.ty,
                }
            });
            let place = data
                .external_target
                .map_or(PlaceHir::Local(*local), PlaceHir::Global);
            return Some((self.hir.alloc_expr(ExprKindHir::Place(place), ty, span), ty));
        }
        if let Some(param) = ctx.params.get(&symbol) {
            let ty = self.hir.params[param.index()].ty;
            return Some((
                self.hir
                    .alloc_expr(ExprKindHir::Place(PlaceHir::Param(*param)), ty, span),
                ty,
            ));
        }
        if ctx.result == Some(symbol) {
            let def = ctx.def;
            let ty = match &self.hir.defs[def.index()].kind {
                DefKind::Function {
                    result: Some(ty), ..
                }
                | DefKind::Method {
                    result: Some(ty), ..
                } => *ty,
                _ => self.hir.types.error(),
            };
            return Some((
                self.hir
                    .alloc_expr(ExprKindHir::Place(PlaceHir::Result(def)), ty, span),
                ty,
            ));
        }
        if let Some(owner) = ctx.owner {
            let owner_ty = self.hir.types.intern(Ty::Named(owner));
            if let Some(field) = self.field_lookup(owner_ty, symbol) {
                let field_data = self.hir.field(field).clone();
                if let Some(base) = self.make_this(ctx, span) {
                    return Some((
                        self.hir.alloc_expr(
                            ExprKindHir::Place(PlaceHir::Field { base, field }),
                            field_data.ty,
                            span,
                        ),
                        field_data.ty,
                    ));
                }
            }
        }
        match self.lookup_scope(ctx.scope, symbol) {
            Lookup::Entries(defs) if defs.len() == 1 => {
                let def = defs[0];
                if let DefKind::Global { ty, .. } = &self.hir.defs[def.index()].kind {
                    let ty = *ty;
                    return Some((
                        self.hir
                            .alloc_expr(ExprKindHir::Place(PlaceHir::Global(def)), ty, span),
                        ty,
                    ));
                }
                None
            }
            Lookup::Constant(binding) => Some((
                self.hir.alloc_expr(
                    ExprKindHir::Literal(stc_hir::LiteralHir {
                        value: binding.value,
                    }),
                    binding.ty,
                    span,
                ),
                binding.ty,
            )),
            _ => None,
        }
    }

    fn project_receiver(
        &mut self,
        ctx: &mut BodyCtx,
        mut receiver: ExprId,
        mut ty: TyId,
        segments: &[Name],
        span: Span,
    ) -> Option<(ExprId, TyId)> {
        for segment in segments {
            let symbol = self.hir.intern(segment.text());
            let Some(field) = self.field_lookup(ty, symbol) else {
                self.member_error(receiver, ty, segment, span);
                return None;
            };
            let field_data = self.hir.field(field).clone();
            self.check_field_access(field, ctx.owner, span);
            receiver = self.hir.alloc_expr(
                ExprKindHir::Place(PlaceHir::Field {
                    base: receiver,
                    field,
                }),
                field_data.ty,
                span,
            );
            ty = field_data.ty;
        }
        Some((receiver, ty))
    }

    fn fb_def_of_ty(&self, ty: TyId) -> Option<DefId> {
        match self.hir.types.get(ty) {
            Ty::Named(def)
                if matches!(
                    &self.hir.defs[def.index()].kind,
                    DefKind::FunctionBlock { .. }
                ) =>
            {
                Some(*def)
            }
            _ => None,
        }
    }

    fn conversion_from_symbol(&mut self, symbol: Symbol) -> Option<(TyId, TyId)> {
        let text = self.symbol_text(symbol).to_string();
        let (from, to) = text.split_once("_TO_")?;
        let from = self.type_name_to_ty(from)?;
        let to = self.type_name_to_ty(to)?;
        Some((from, to))
    }
    fn methods_named(&self, ty: TyId, symbol: Symbol) -> Vec<DefId> {
        self.methods_named_from(ty, symbol, true)
    }

    /// Returns same-named methods, preferring the most-derived declaration for
    /// an identical parameter/result signature.
    fn methods_named_from(&self, ty: TyId, symbol: Symbol, include_self: bool) -> Vec<DefId> {
        let Ty::Named(def) = self.hir.types.get(ty) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut signatures = BTreeSet::new();
        let mut stack = vec![*def];
        let mut visited = BTreeSet::new();
        let mut first = true;
        while let Some(next) = stack.pop() {
            if !visited.insert(next) {
                continue;
            }
            let skip = first && !include_self;
            first = false;
            if !skip {
                match &self.hir.defs[next.index()].kind {
                    DefKind::Class { methods, .. }
                    | DefKind::FunctionBlock { methods, .. }
                    | DefKind::Interface { methods, .. } => {
                        for method in methods {
                            if self.hir.defs[method.index()].name != Some(symbol) {
                                continue;
                            }
                            let signature = self.method_signature_types(*method);
                            if signatures.insert(signature) {
                                out.push(*method);
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let Some(base) = self.hir.defs[next.index()].base() {
                stack.push(base);
            }
            if let DefKind::Interface { extends, .. } = &self.hir.defs[next.index()].kind {
                for parent in extends {
                    stack.push(*parent);
                }
            }
        }
        out
    }

    fn method_signature_types(&self, def: DefId) -> Vec<TyId> {
        let DefKind::Method { params, result, .. } = &self.hir.defs[def.index()].kind else {
            return Vec::new();
        };
        let mut signature: Vec<TyId> = params
            .iter()
            .map(|param| self.hir.param(*param).ty)
            .collect();
        signature.push(result.unwrap_or_else(|| self.hir.types.error()));
        signature
    }

    // ---- coercion and type helpers --------------------------------------

    fn coerce_expr_to(
        &mut self,
        id: ExprId,
        target: TyId,
        explicit: bool,
        span: Span,
        code: &'static str,
    ) -> ExprId {
        let ty = self.hir.expr(id).ty;
        if ty == target || ty == self.hir.types.error() || target == self.hir.types.error() {
            return id;
        }
        if let ExprKindHir::Literal(literal) = &self.hir.expr(id).kind
            && let ConstValue::Int(value) = literal.value
            && let Ty::Named(def) = self.hir.types.get(target)
            && let DefKind::Subrange { lower, upper, .. } = &self.hir.defs[def.index()].kind
            && !(*lower..=*upper).contains(&value)
        {
            self.error(
                crate::codes::CONST_FAILURE,
                format!("constant {value} is outside the subrange bounds {lower}..{upper}"),
                span,
            );
            return id;
        }
        if self.adapt_literal(id, target) {
            return id;
        }
        match self.assignability(target, ty) {
            Conv::Exact => id,
            Conv::Implicit => self.hir.alloc_expr(
                ExprKindHir::Cast {
                    operand: id,
                    to: target,
                    kind: CastKind::Implicit,
                },
                target,
                span,
            ),
            Conv::Explicit if explicit => self.hir.alloc_expr(
                ExprKindHir::Cast {
                    operand: id,
                    to: target,
                    kind: CastKind::Explicit,
                },
                target,
                span,
            ),
            Conv::Explicit => {
                self.push_diag(
                    Diagnostic::error(
                        crate::codes::CONV_EXPLICIT,
                        format!(
                            "cannot implicitly convert `{}` to `{}`",
                            self.type_name(ty),
                            self.type_name(target)
                        ),
                        span,
                    )
                    .with_note("an explicit conversion function is required"),
                );
                id
            }
            Conv::No => {
                self.error(
                    code,
                    format!(
                        "cannot convert `{}` to `{}`",
                        self.type_name(ty),
                        self.type_name(target)
                    ),
                    span,
                );
                id
            }
        }
    }

    fn adapt_literal(&mut self, id: ExprId, target: TyId) -> bool {
        let ExprKindHir::Literal(literal) = self.hir.expr(id).kind.clone() else {
            return false;
        };
        let Some(value) = self.literal_as(&literal.value, target) else {
            return false;
        };
        self.hir.exprs[id.index()].kind = ExprKindHir::Literal(stc_hir::LiteralHir { value });
        self.hir.exprs[id.index()].ty = target;
        true
    }

    fn literal_as(&self, value: &ConstValue, target: TyId) -> Option<ConstValue> {
        if self.is_enum_like(target) {
            return None;
        }
        let target_base = self.scalar_base(target);
        if let Ty::Named(def) = self.hir.types.get(target)
            && let DefKind::Subrange { lower, upper, .. } = &self.hir.defs[def.index()].kind
        {
            match value {
                ConstValue::Int(value) if (*lower..=*upper).contains(value) => {
                    return Some(ConstValue::Int(*value));
                }
                ConstValue::Int(_) => return None,
                _ => {}
            }
        }
        let target_kind = self.hir.types.get(target_base).clone();
        Some(match (value, target_kind) {
            (ConstValue::Int(value), Ty::Int(int)) => {
                if fits_int(*value, int.signed, int.width) {
                    ConstValue::Int(*value)
                } else {
                    return None;
                }
            }
            (ConstValue::Int(value), Ty::BitString(width)) => {
                if fits_int(*value, false, width) {
                    ConstValue::Int(*value)
                } else {
                    return None;
                }
            }
            (ConstValue::Int(value), Ty::Bool) => {
                if *value == 0 || *value == 1 {
                    ConstValue::Bool(*value == 1)
                } else {
                    return None;
                }
            }
            (ConstValue::Int(value), Ty::Real(_)) => ConstValue::Real(*value as f64),
            (ConstValue::Real(value), Ty::Real(_)) => ConstValue::Real(*value),
            (ConstValue::Int(value), Ty::Char(_)) => ConstValue::Char(u32::try_from(*value).ok()?),
            (ConstValue::Char(value), Ty::Char(_)) => ConstValue::Char(*value),
            (ConstValue::String(units), Ty::String(string)) => {
                if units.len() as u64 <= u64::from(string.capacity) {
                    ConstValue::String(units.clone())
                } else {
                    return None;
                }
            }
            (ConstValue::Bool(value), Ty::Bool) => ConstValue::Bool(*value),
            (ConstValue::Time(value), Ty::Time(_)) => ConstValue::Time(*value),
            (ConstValue::Date { value, .. }, Ty::Date(kind)) => ConstValue::Date {
                kind,
                value: *value,
            },
            (ConstValue::Null, _)
                if self.is_class_like(target)
                    || matches!(self.hir.types.get(target), Ty::Ref(_)) =>
            {
                ConstValue::Null
            }
            _ => return None,
        })
    }

    fn is_numeric(&self, ty: TyId) -> bool {
        self.arithmetic_base(ty)
            .is_some_and(|base| matches!(self.hir.types.get(base), Ty::Int(_) | Ty::Real(_)))
    }

    fn is_int_ty(&self, ty: TyId) -> bool {
        self.arithmetic_base(ty)
            .is_some_and(|base| matches!(self.hir.types.get(base), Ty::Int(_)))
    }

    fn is_numeric_or_time(&self, ty: TyId) -> bool {
        self.arithmetic_base(ty)
            .is_some_and(|base| matches!(self.hir.types.get(base), Ty::Int(_) | Ty::Real(_)))
            || self.is_time_ty(ty)
    }

    fn is_time_ty(&self, ty: TyId) -> bool {
        matches!(self.hir.types.get(self.scalar_base(ty)), Ty::Time(_))
    }

    fn check_field_access(
        &mut self,
        field: FieldId,
        from_owner: Option<DefId>,
        span: Span,
    ) -> bool {
        let data = self.hir.field(field).clone();
        self.check_visibility(data.visibility, data.owner, from_owner, data.span, span)
    }

    fn check_method_access(
        &mut self,
        method: DefId,
        from_owner: Option<DefId>,
        span: Span,
    ) -> bool {
        let (visibility, owner, decl) = match &self.hir.defs[method.index()].kind {
            DefKind::Method { access, owner, .. } => {
                (*access, *owner, self.hir.defs[method.index()].span)
            }
            _ => (
                Visibility::Public,
                method,
                self.hir.defs[method.index()].span,
            ),
        };
        self.check_visibility(visibility, owner, from_owner, decl, span)
    }

    fn check_visibility(
        &mut self,
        visibility: Visibility,
        owner: DefId,
        from_owner: Option<DefId>,
        decl: Span,
        span: Span,
    ) -> bool {
        let ok = match visibility {
            Visibility::Public | Visibility::Internal => true,
            Visibility::Private => from_owner == Some(owner),
            Visibility::Protected => match from_owner {
                Some(current) if current == owner => true,
                Some(current) => {
                    let current_ty = self.hir.types.intern(Ty::Named(current));
                    let owner_ty = self.hir.types.intern(Ty::Named(owner));
                    self.is_subtype(owner_ty, current_ty)
                }
                None => false,
            },
        };
        if !ok {
            let name = self.def_name(owner);
            self.push_diag(
                Diagnostic::error(
                    crate::codes::NAME_INACCESSIBLE,
                    format!("`{name}` is not accessible here"),
                    span,
                )
                .with_label(decl, "declaration is here")
                .with_note(format!("its visibility is `{}`", visibility.as_str())),
            );
        }
        ok
    }

    // ---- exports ---------------------------------------------------------

    /// Validates `{stc:export}`/`{stc:no_mangle}` and records stable C names (D-086).
    pub(crate) fn check_exports(&mut self) {
        let mut candidates = Vec::new();
        for (index, def) in self.hir.defs.iter().enumerate() {
            if (!def.export && !def.no_mangle) || def.test_only {
                continue;
            }
            let id = DefId::new(u32::try_from(index).expect("definition limit"));
            let is_function = matches!(def.kind, DefKind::Function { .. });
            let name = def.name_text(&self.hir.interner).to_string();
            let normalized = def.name.map_or(String::new(), |symbol| {
                self.hir.interner.normalized(symbol).to_string()
            });
            candidates.push((
                id,
                def.span,
                def.export,
                def.no_mangle,
                is_function,
                name,
                normalized,
            ));
        }
        let mut emitted: BTreeMap<String, Span> = BTreeMap::new();
        let mut updates = Vec::new();
        for (def, span, export, no_mangle, is_function, name, normalized) in candidates {
            if no_mangle && !export {
                self.error(
                    crate::codes::EXPORT_INVALID,
                    "`{stc:no_mangle}` requires `{stc:export}`",
                    span,
                );
                continue;
            }
            if !is_function {
                self.error(
                    crate::codes::EXPORT_INVALID,
                    "`{stc:export}` is only supported on `FUNCTION` in M4",
                    span,
                );
                continue;
            }
            let symbol_text = if no_mangle {
                if !is_c_identifier(&name) {
                    self.error(
                        crate::codes::EXPORT_INVALID,
                        format!("`no_mangle` name `{name}` is not a C identifier"),
                        span,
                    );
                    continue;
                }
                let lowered = name.to_ascii_lowercase();
                if lowered.starts_with("stc_") || lowered.starts_with("llvm.") {
                    self.error(
                        crate::codes::EXPORT_INVALID,
                        format!("export name `{name}` is reserved"),
                        span,
                    );
                    continue;
                }
                name
            } else {
                format!("stc_export_{normalized}")
            };
            if let Some(previous) = emitted.get(&symbol_text) {
                let previous = *previous;
                self.push_diag(
                    Diagnostic::error(
                        crate::codes::EXPORT_INVALID,
                        format!("duplicate export symbol `{symbol_text}`"),
                        span,
                    )
                    .with_label(previous, "the first export with this name is here"),
                );
                continue;
            }
            emitted.insert(symbol_text.clone(), span);
            let symbol = self.hir.intern(&symbol_text);
            updates.push((def, symbol));
        }
        for (def, symbol) in updates {
            self.hir.defs[def.index()].export_name = Some(symbol);
        }
    }

    // ---- configuration ---------------------------------------------------

    pub(crate) fn check_configurations(&mut self) {
        let sites = self.config_sites.clone();
        for site in &sites {
            self.check_configuration(site);
        }
        let configs: Vec<DefId> = sites.iter().map(|site| site.def).collect();
        if configs.len() == 1 {
            self.entry_configuration = Some(configs[0]);
        }
    }

    fn check_task_value(&mut self, id: ExprId, kind: TaskKind, span: Span) {
        let ty = self.hir.expr(id).ty;
        if !self.is_constant_expr(id) {
            self.error(
                crate::codes::CONST_NOT_CONSTANT,
                "task attributes must be constant expressions",
                span,
            );
        }
        let ok = match kind {
            TaskKind::Single => ty == self.hir.types.bool(),
            TaskKind::Interval => self.is_time_ty(ty),
            TaskKind::Priority => self.is_int_ty(ty),
        };
        if !ok {
            let expected = match kind {
                TaskKind::Single => "BOOL",
                TaskKind::Interval => "TIME",
                TaskKind::Priority => "an integer",
            };
            self.error(
                crate::codes::TYPE_MISMATCH,
                format!(
                    "task attribute must be {expected}, found `{}`",
                    self.type_name(ty)
                ),
                span,
            );
        }
    }

    /// Folds a checked task attribute into a scalar HIR constant (D-088).
    fn fold_task_const(&self, id: ExprId) -> Option<ConstValue> {
        match &self.hir.expr(id).kind {
            ExprKindHir::Literal(literal) => Some(literal.value.clone()),
            ExprKindHir::Place(PlaceHir::Global(def)) => self.const_globals.get(def).cloned(),
            ExprKindHir::Cast { operand, to, .. } => {
                let value = self.fold_task_const(*operand)?;
                fold_cast_const(&value, self.hir.types.get(*to))
            }
            ExprKindHir::Unary { op, operand } => {
                let value = self.fold_task_const(*operand)?;
                match (op, value) {
                    (UnOp::Plus, value) => Some(value),
                    (UnOp::Neg, ConstValue::Int(value)) => {
                        Some(ConstValue::Int(value.checked_neg()?))
                    }
                    (UnOp::Neg, ConstValue::Real(value)) => Some(ConstValue::Real(-value)),
                    (UnOp::Neg, ConstValue::Time(value)) => {
                        Some(ConstValue::Time(value.checked_neg()?))
                    }
                    (UnOp::Not, ConstValue::Bool(value)) => Some(ConstValue::Bool(!value)),
                    _ => None,
                }
            }
            ExprKindHir::Binary { op, lhs, rhs } => {
                let lhs = self.fold_task_const(*lhs)?;
                let rhs = self.fold_task_const(*rhs)?;
                fold_binary_const(*op, &lhs, &rhs)
            }
            _ => None,
        }
    }

    fn is_constant_expr(&self, id: ExprId) -> bool {
        match &self.hir.expr(id).kind {
            ExprKindHir::Literal(_) | ExprKindHir::Null | ExprKindHir::Error => true,
            ExprKindHir::Cast { operand, .. } | ExprKindHir::Unary { operand, .. } => {
                self.is_constant_expr(*operand)
            }
            ExprKindHir::Place(PlaceHir::Global(def)) => matches!(
                &self.hir.defs[def.index()].kind,
                DefKind::Global { constant: true, .. }
            ),
            _ => false,
        }
    }

    fn check_configuration(&mut self, site: &crate::ConfigSite) {
        let mut resources = Vec::new();
        for resource in site.decl.resources() {
            let name = resource
                .name()
                .map_or_else(String::new, |name| name.text().to_string());
            let on = resource
                .on()
                .map_or_else(String::new, |name| name.text().to_string());
            let name = self.hir.intern(&name);
            let on = self.hir.intern(&on);
            let mut tasks: Vec<stc_hir::TaskHir> = Vec::new();
            for task in resource.tasks() {
                let task_name = task
                    .name()
                    .map_or_else(String::new, |name| name.text().to_string());
                let mut single = None;
                let mut interval = None;
                let mut priority = None;
                for attribute in task.attributes() {
                    let value = attribute
                        .value()
                        .and_then(|expr| self.check_expr_in_namespace(&expr, None, site.def));
                    match attribute.kind().map(|token| token.kind()) {
                        Some(stc_syntax::SyntaxKind::KwSingle) => single = value,
                        Some(stc_syntax::SyntaxKind::KwInterval) => interval = value,
                        Some(stc_syntax::SyntaxKind::KwPriority) => priority = value,
                        _ => {}
                    }
                }
                let task_symbol = self.hir.intern(&task_name);
                if tasks.iter().any(|candidate| candidate.name == task_symbol) {
                    self.error(
                        crate::codes::CONFIG_ERROR,
                        format!("duplicate task `{task_name}`"),
                        task.token_span(),
                    );
                }
                if let Some(value) = single {
                    self.check_task_value(value, TaskKind::Single, task.token_span());
                }
                if let Some(value) = interval {
                    self.check_task_value(value, TaskKind::Interval, task.token_span());
                }
                if let Some(value) = priority {
                    self.check_task_value(value, TaskKind::Priority, task.token_span());
                }
                let single_value = single.and_then(|id| match self.fold_task_const(id) {
                    Some(ConstValue::Bool(value)) => Some(value),
                    _ => None,
                });
                let interval_ns = interval.and_then(|id| match self.fold_task_const(id) {
                    Some(ConstValue::Time(value) | ConstValue::Int(value)) => Some(value),
                    _ => None,
                });
                let priority_value = priority.and_then(|id| match self.fold_task_const(id) {
                    Some(
                        ConstValue::Int(value)
                        | ConstValue::Time(value)
                        | ConstValue::Enum { value, .. },
                    ) => Some(value),
                    Some(ConstValue::Char(value)) => Some(i128::from(value)),
                    Some(ConstValue::Bool(value)) => Some(i128::from(u8::from(value))),
                    _ => None,
                });
                tasks.push(stc_hir::TaskHir {
                    name: task_symbol,
                    single,
                    interval,
                    priority,
                    single_value,
                    interval_ns,
                    priority_value,
                    span: task.token_span(),
                });
            }
            let mut programs: Vec<stc_hir::ProgramInstanceHir> = Vec::new();
            for program in resource.program_configs() {
                if let Some(instance) = self.check_program_config(&program, site, &tasks) {
                    if programs
                        .iter()
                        .any(|candidate| candidate.name == instance.name)
                    {
                        self.error(
                            crate::codes::CONFIG_ERROR,
                            "duplicate program instance name",
                            instance.span,
                        );
                    } else {
                        programs.push(instance);
                    }
                }
            }
            if resources
                .iter()
                .any(|candidate: &stc_hir::ResourceHir| candidate.name == name)
            {
                self.error(
                    crate::codes::CONFIG_ERROR,
                    "duplicate resource name",
                    resource.token_span(),
                );
            }
            resources.push(stc_hir::ResourceHir {
                name,
                on,
                tasks,
                programs,
                var_access: Vec::new(),
            });
        }
        // A configuration without an explicit RESOURCE still has program
        // configurations; validate them too.
        if site.decl.resources().is_empty() {
            let tasks = Vec::new();
            let mut programs = Vec::new();
            for program in site.decl.program_configs() {
                if let Some(instance) = self.check_program_config(&program, site, &tasks) {
                    programs.push(instance);
                }
            }
            resources.push(stc_hir::ResourceHir {
                name: self.hir.intern("RESOURCE"),
                on: self.hir.intern(""),
                tasks,
                programs,
                var_access: Vec::new(),
            });
        }
        let mut instance_names = std::collections::BTreeSet::new();
        for resource in &resources {
            for program in &resource.programs {
                if !instance_names.insert(program.name) {
                    self.error(
                        crate::codes::CONFIG_ERROR,
                        "program instance name is duplicated across resources",
                        program.span,
                    );
                }
            }
        }
        let instance_map: BTreeMap<Symbol, DefId> = resources
            .iter()
            .flat_map(|resource| resource.programs.iter())
            .map(|program| (program.name, program.program))
            .collect();
        let var_access = self.check_config_var_access(site, &instance_map);
        let config_inits = self.check_config_inits(site, &instance_map);
        if let DefKind::Configuration { config } = &mut self.hir.defs[site.def.index()].kind {
            config.resources = resources;
            config.var_access = var_access;
            config.config_inits = config_inits;
        }
    }

    fn check_config_var_access(
        &mut self,
        site: &crate::ConfigSite,
        instances: &BTreeMap<Symbol, DefId>,
    ) -> Vec<stc_hir::VarAccessHir> {
        let Some(section) = site.decl.var_access() else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        for access in section.declarations() {
            let span = access.token_span();
            let Some(name) = access.name() else {
                continue;
            };
            let name_symbol = self.hir.intern(name.text());
            let declared = access
                .type_ref()
                .map(|type_ref| self.resolve_type_ref(&type_ref, site.ns))
                .unwrap_or_else(|| self.hir.types.error());
            let (place, actual_ty) = if let Some(token) = access.direct_token() {
                let Some(Literal::Direct(address)) = token.literal() else {
                    continue;
                };
                self.check_direct_address(address, declared, token.span(), false);
                let place = self.hir.alloc_expr(
                    ExprKindHir::Place(PlaceHir::Direct(crate::collect::direct_address(
                        address.clone(),
                        token.span(),
                    ))),
                    declared,
                    token.span(),
                );
                (place, declared)
            } else {
                let names = access.path_names();
                if names.is_empty() {
                    self.error(
                        crate::codes::CONFIG_ERROR,
                        "`VAR_ACCESS` path is empty",
                        span,
                    );
                    continue;
                }
                if names.len() == 1 {
                    let symbol = self.hir.intern(names[0].text());
                    match self.lookup_scope(site.def, symbol) {
                        Lookup::Entries(defs) if defs.len() == 1 => {
                            let def = defs[0];
                            match &self.hir.defs[def.index()].kind {
                                DefKind::Global { ty, .. } => {
                                    let ty = *ty;
                                    let place = self.hir.alloc_expr(
                                        ExprKindHir::Place(PlaceHir::Global(def)),
                                        ty,
                                        span,
                                    );
                                    (place, ty)
                                }
                                _ => {
                                    self.error(
                                        crate::codes::CONFIG_ERROR,
                                        "`VAR_ACCESS` path does not denote a variable",
                                        span,
                                    );
                                    continue;
                                }
                            }
                        }
                        _ => {
                            self.error(
                                crate::codes::NAME_UNKNOWN,
                                format!("unknown `VAR_ACCESS` path `{}`", names[0].text()),
                                span,
                            );
                            continue;
                        }
                    }
                } else {
                    let instance = self.hir.intern(names[0].text());
                    let Some(program) = instances.get(&instance).copied() else {
                        self.error(
                            crate::codes::CONFIG_ERROR,
                            format!("unknown program instance `{}`", names[0].text()),
                            names[0].token_span(),
                        );
                        continue;
                    };
                    let mut ty = self.hir.types.intern(Ty::Named(program));
                    let mut fields = Vec::new();
                    let mut ok = true;
                    for segment in &names[1..] {
                        let symbol = self.hir.intern(segment.text());
                        let Some(field) = self.field_lookup(ty, symbol) else {
                            self.error(
                                crate::codes::NAME_UNKNOWN,
                                format!(
                                    "type `{}` has no field `{}`",
                                    self.type_name(ty),
                                    segment.text()
                                ),
                                segment.token_span(),
                            );
                            ok = false;
                            break;
                        };
                        let data = self.hir.field(field).clone();
                        self.check_field_access(field, None, span);
                        ty = data.ty;
                        fields.push(field);
                    }
                    if !ok {
                        continue;
                    }
                    let place = self.hir.alloc_expr(
                        ExprKindHir::Place(PlaceHir::InstanceField {
                            program,
                            instance,
                            fields,
                        }),
                        ty,
                        span,
                    );
                    (place, ty)
                }
            };
            if self.contains_pointer(declared) || self.contains_pointer(actual_ty) {
                self.error(
                    crate::codes::TYPE_INVALID,
                    "pointer values cannot be exposed through `VAR_ACCESS`",
                    span,
                );
                continue;
            }
            if declared != self.hir.types.error()
                && actual_ty != self.hir.types.error()
                && self.can_assign(declared, actual_ty) == Conv::No
            {
                self.error(
                    crate::codes::TYPE_MISMATCH,
                    format!(
                        "`VAR_ACCESS` type `{}` does not match the access path type `{}`",
                        self.type_name(declared),
                        self.type_name(actual_ty)
                    ),
                    span,
                );
            }
            entries.push(stc_hir::VarAccessHir {
                name: name_symbol,
                place,
                ty: declared,
                read_only: access.is_read_only(),
                span,
            });
        }
        entries
    }

    fn check_config_inits(
        &mut self,
        site: &crate::ConfigSite,
        instances: &BTreeMap<Symbol, DefId>,
    ) -> Vec<stc_hir::ConfigInitHir> {
        let Some(init) = site.decl.config_init() else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        for instance_init in init.instances() {
            let span = instance_init.token_span();
            let names = instance_init.names();
            if names.len() < 2 {
                self.error(
                    crate::codes::CONFIG_ERROR,
                    "`VAR_CONFIG` requires an instance and a field path",
                    span,
                );
                continue;
            }
            let instance = self.hir.intern(names[0].text());
            let Some(program) = instances.get(&instance).copied() else {
                self.error(
                    crate::codes::CONFIG_ERROR,
                    format!("unknown program instance `{}`", names[0].text()),
                    names[0].token_span(),
                );
                continue;
            };
            let mut ty = self.hir.types.intern(Ty::Named(program));
            let mut fields = Vec::new();
            let mut ok = true;
            for segment in &names[1..] {
                let symbol = self.hir.intern(segment.text());
                let Some(field) = self.field_lookup(ty, symbol) else {
                    self.error(
                        crate::codes::NAME_UNKNOWN,
                        format!(
                            "type `{}` has no field `{}`",
                            self.type_name(ty),
                            segment.text()
                        ),
                        segment.token_span(),
                    );
                    ok = false;
                    break;
                };
                let data = self.hir.field(field).clone();
                self.check_field_access(field, None, segment.token_span());
                ty = data.ty;
                fields.push(field);
            }
            if !ok {
                continue;
            }
            let declared = instance_init
                .type_ref()
                .map(|type_ref| self.resolve_type_ref(&type_ref, site.ns))
                .unwrap_or(ty);
            if self.can_assign(declared, ty) == Conv::No {
                self.error(
                    crate::codes::TYPE_MISMATCH,
                    format!(
                        "`VAR_CONFIG` type `{}` does not match the configured field type `{}`",
                        self.type_name(declared),
                        self.type_name(ty)
                    ),
                    span,
                );
            }
            let at = instance_init
                .at_token()
                .and_then(|token| match token.literal() {
                    Some(Literal::Direct(address)) => {
                        self.check_direct_address(address, declared, token.span(), false);
                        Some(crate::collect::direct_address(
                            address.clone(),
                            token.span(),
                        ))
                    }
                    _ => None,
                });
            let init_expr = instance_init
                .initializer_node()
                .and_then(|initializer| self.lower_initializer(&initializer, declared, site.def));
            entries.push(stc_hir::ConfigInitHir {
                program,
                instance,
                fields,
                at,
                ty: declared,
                init: init_expr,
                span,
            });
        }
        entries
    }

    fn check_program_config(
        &mut self,
        program: &stc_syntax::ast::ProgramConfig,
        site: &crate::ConfigSite,
        tasks: &[stc_hir::TaskHir],
    ) -> Option<stc_hir::ProgramInstanceHir> {
        let span = program.token_span();
        let instance = program.name()?;
        let instance_symbol = self.hir.intern(instance.text());
        let path = program.program_path()?;
        let Some(def) = self.resolve_def_path(&path, site.ns) else {
            let text = path.text().to_string();
            self.error(
                crate::codes::CONFIG_ERROR,
                format!("unknown program `{text}`"),
                path.token_span(),
            );
            return None;
        };
        if !matches!(self.hir.defs[def.index()].kind, DefKind::Program { .. }) {
            self.error(
                crate::codes::CONFIG_ERROR,
                "configuration entry must name a `PROGRAM`",
                path.token_span(),
            );
            return None;
        }
        let task = program.with().map(|name| self.hir.intern(name.text()));
        if let Some(task) = task
            && !tasks.iter().any(|candidate| candidate.name == task)
        {
            self.error(
                crate::codes::CONFIG_ERROR,
                "program configuration references an unknown task",
                span,
            );
        }
        let mut bindings = Vec::new();
        for element in program.elements() {
            if element.operator().is_none() {
                // `fb WITH task` form: both names must resolve in context.
                let names = element.names();
                if names.len() != 2 {
                    self.error(
                        crate::codes::CONFIG_ERROR,
                        "`WITH` binding requires a field and a task name",
                        element.token_span(),
                    );
                    continue;
                }
                let field_symbol = self.hir.intern(names[0].text());
                let program_ty = self.hir.types.intern(Ty::Named(def));
                if self.field_lookup(program_ty, field_symbol).is_none() {
                    self.error(
                        crate::codes::CONFIG_ERROR,
                        format!("`{}` is not a field of the program", names[0].text()),
                        names[0].token_span(),
                    );
                }
                let task_symbol = self.hir.intern(names[1].text());
                if !tasks.iter().any(|candidate| candidate.name == task_symbol) {
                    self.error(
                        crate::codes::CONFIG_ERROR,
                        format!("unknown task `{}`", names[1].text()),
                        names[1].token_span(),
                    );
                }
                continue;
            }
            let expressions = element.expressions();
            let Some(target) = expressions.first() else {
                continue;
            };
            let Some(source) = expressions.get(1) else {
                continue;
            };
            let output = element
                .operator()
                .is_some_and(|token| token.kind() == stc_syntax::SyntaxKind::PunctOutputAssign);
            let target_id = self.check_config_value(target, site.def, Some(def), true);
            let source_id = self.check_config_value(source, site.def, Some(def), false);
            if let (Some(target_id), Some(source_id)) = (target_id, source_id) {
                let target_ty = self.hir.expr(target_id).ty;
                let source_ty = self.hir.expr(source_id).ty;
                let direction = match &self.hir.expr(target_id).kind {
                    ExprKindHir::Place(PlaceHir::Field { field, .. }) => {
                        self.hir.field(*field).direction()
                    }
                    _ => None,
                };
                if output && direction == Some(ArgDirection::Input) {
                    self.error(
                        crate::codes::CONFIG_ERROR,
                        "`=>` cannot bind a program input",
                        element.token_span(),
                    );
                }
                if !output && direction == Some(ArgDirection::Output) {
                    self.error(
                        crate::codes::CONFIG_ERROR,
                        "`:=` cannot bind a program output; use `=>`",
                        element.token_span(),
                    );
                }
                let compatible = if output {
                    self.can_assign(source_ty, target_ty)
                } else {
                    self.can_assign(target_ty, source_ty)
                };
                if compatible == Conv::No {
                    self.error(
                        crate::codes::TYPE_MISMATCH,
                        format!(
                            "binding type `{}` does not match `{}`",
                            self.type_name(source_ty),
                            self.type_name(target_ty)
                        ),
                        element.token_span(),
                    );
                }
                bindings.push(stc_hir::BindingHir {
                    target: target_id,
                    source: source_id,
                    output,
                    span: element.token_span(),
                });
            }
        }
        Some(stc_hir::ProgramInstanceHir {
            name: instance_symbol,
            program: def,
            task,
            bindings,
            span,
        })
    }

    fn check_config_value(
        &mut self,
        expr: &Expr,
        ns: DefId,
        program: Option<DefId>,
        _target: bool,
    ) -> Option<ExprId> {
        let mut ctx = BodyCtx::namespace(ns);
        if let Some(program) = program {
            ctx.owner = Some(program);
            ctx.this_ty = Some(self.hir.types.intern(Ty::Named(program)));
        }
        Some(self.check_expr(&mut ctx, expr, None))
    }
}

#[derive(Clone, Debug)]
enum FirstName {
    Local(LocalId),
    Param(ParamId),
    Result(DefId),
    Field(FieldId, ExprId),
    Global(DefId),
    Constant(ConstBinding),
    Function,
    Method,
    Standard,
    StandardFb,
    Type,
    Namespace(DefId),
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskKind {
    Single,
    Interval,
    Priority,
}

#[derive(Clone, Debug)]
struct PreparedArg {
    kind: PreparedKind,
    value: Option<ExprId>,
    value_ty: Option<TyId>,
    target: Option<ExprId>,
    target_ty: Option<TyId>,
    negated: bool,
    span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PreparedKind {
    Positional,
    Named(Symbol),
    Output(Symbol),
    Error,
}

fn bin_op(kind: stc_syntax::SyntaxKind) -> Option<BinOp> {
    Some(match kind {
        stc_syntax::SyntaxKind::PunctPlus => BinOp::Add,
        stc_syntax::SyntaxKind::PunctMinus => BinOp::Sub,
        stc_syntax::SyntaxKind::PunctStar => BinOp::Mul,
        stc_syntax::SyntaxKind::PunctSlash => BinOp::Div,
        stc_syntax::SyntaxKind::KwMod => BinOp::Mod,
        stc_syntax::SyntaxKind::PunctPower => BinOp::Power,
        stc_syntax::SyntaxKind::PunctEqual => BinOp::Eq,
        stc_syntax::SyntaxKind::PunctNotEqual => BinOp::Ne,
        stc_syntax::SyntaxKind::PunctLess => BinOp::Lt,
        stc_syntax::SyntaxKind::PunctLessEqual => BinOp::Le,
        stc_syntax::SyntaxKind::PunctGreater => BinOp::Gt,
        stc_syntax::SyntaxKind::PunctGreaterEqual => BinOp::Ge,
        stc_syntax::SyntaxKind::KwAnd | stc_syntax::SyntaxKind::PunctAmpersand => BinOp::And,
        stc_syntax::SyntaxKind::KwOr => BinOp::Or,
        stc_syntax::SyntaxKind::KwXor => BinOp::Xor,
        _ => return None,
    })
}

fn fits_int(value: i128, signed: bool, width: u8) -> bool {
    crate::types::fits_int(value, signed, width)
}

fn labels_overlap(
    left_low: &ConstValue,
    left_high: &ConstValue,
    right_low: &ConstValue,
    right_high: &ConstValue,
) -> bool {
    match (
        compare_consts(left_low, right_high),
        compare_consts(right_low, left_high),
    ) {
        (Some(left_start), Some(right_start)) => {
            left_start != std::cmp::Ordering::Greater && right_start != std::cmp::Ordering::Greater
        }
        _ => false,
    }
}

fn compare_consts(lhs: &ConstValue, rhs: &ConstValue) -> Option<std::cmp::Ordering> {
    match (lhs, rhs) {
        (ConstValue::Int(a), ConstValue::Int(b)) => Some(a.cmp(b)),
        (ConstValue::Enum { value: a, .. }, ConstValue::Enum { value: b, .. }) => Some(a.cmp(b)),
        (ConstValue::Char(a), ConstValue::Char(b)) => Some(a.cmp(b)),
        (ConstValue::Time(a), ConstValue::Time(b)) => Some(a.cmp(b)),
        (ConstValue::Date { value: a, .. }, ConstValue::Date { value: b, .. }) => Some(a.cmp(b)),
        _ => None,
    }
}

fn text_of(name: &Name) -> String {
    name.text().to_string()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

fn is_c_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn fold_cast_const(value: &ConstValue, to: &Ty) -> Option<ConstValue> {
    match (value, to) {
        (ConstValue::Int(value), Ty::Int(_) | Ty::BitString(_)) => Some(ConstValue::Int(*value)),
        (ConstValue::Int(value), Ty::Time(_)) => Some(ConstValue::Time(*value)),
        (ConstValue::Int(value), Ty::Real(_)) => Some(ConstValue::Real(*value as f64)),
        (ConstValue::Int(value), Ty::Bool) => Some(ConstValue::Bool(*value != 0)),
        (ConstValue::Int(value), Ty::Char(_)) => {
            Some(ConstValue::Char(u32::try_from(*value).ok()?))
        }
        (ConstValue::Bool(value), Ty::Bool) => Some(ConstValue::Bool(*value)),
        (ConstValue::Bool(value), Ty::Int(_) | Ty::BitString(_)) => {
            Some(ConstValue::Int(i128::from(*value)))
        }
        (ConstValue::Real(value), Ty::Real(_)) => Some(ConstValue::Real(*value)),
        (ConstValue::Time(value), Ty::Time(_)) => Some(ConstValue::Time(*value)),
        (ConstValue::Char(value), Ty::Char(_)) => Some(ConstValue::Char(*value)),
        (ConstValue::Char(value), Ty::Int(_) | Ty::BitString(_)) => {
            Some(ConstValue::Int(i128::from(*value)))
        }
        (ConstValue::Date { kind, value }, Ty::Date(destination)) => {
            let value = match (date_is_days(*kind), date_is_days(*destination)) {
                (false, true) => value.checked_mul(86_400_000_000_000)?,
                (true, false) => value.checked_div(86_400_000_000_000)?,
                _ => *value,
            };
            Some(ConstValue::Date {
                kind: *destination,
                value,
            })
        }
        _ => None,
    }
}

fn date_is_days(kind: stc_hir::DateTy) -> bool {
    matches!(kind, stc_hir::DateTy::Date)
}

fn fold_binary_const(op: BinOp, lhs: &ConstValue, rhs: &ConstValue) -> Option<ConstValue> {
    if op.is_comparison() {
        let ordering = compare_consts(lhs, rhs)?;
        let value = match op {
            BinOp::Eq => ordering == std::cmp::Ordering::Equal,
            BinOp::Ne => ordering != std::cmp::Ordering::Equal,
            BinOp::Lt => ordering == std::cmp::Ordering::Less,
            BinOp::Le => ordering != std::cmp::Ordering::Greater,
            BinOp::Gt => ordering == std::cmp::Ordering::Greater,
            BinOp::Ge => ordering != std::cmp::Ordering::Less,
            _ => return None,
        };
        return Some(ConstValue::Bool(value));
    }
    match (lhs, rhs) {
        (ConstValue::Int(a), ConstValue::Int(b)) => {
            let value = match op {
                BinOp::Add => a.checked_add(*b)?,
                BinOp::Sub => a.checked_sub(*b)?,
                BinOp::Mul => a.checked_mul(*b)?,
                BinOp::Div => a.checked_div(*b)?,
                BinOp::Mod => a.checked_rem(*b)?,
                BinOp::And => a & b,
                BinOp::Or => a | b,
                BinOp::Xor => a ^ b,
                BinOp::Power => {
                    let exponent = u32::try_from(*b).ok()?;
                    a.checked_pow(exponent)?
                }
                _ => return None,
            };
            Some(ConstValue::Int(value))
        }
        (ConstValue::Time(a), ConstValue::Time(b)) => {
            let value = match op {
                BinOp::Add => a.checked_add(*b)?,
                BinOp::Sub => a.checked_sub(*b)?,
                _ => return None,
            };
            Some(ConstValue::Time(value))
        }
        (ConstValue::Real(a), ConstValue::Real(b)) => {
            let value = match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::Mul => a * b,
                BinOp::Div => a / b,
                _ => return None,
            };
            Some(ConstValue::Real(value))
        }
        (ConstValue::Bool(a), ConstValue::Bool(b)) => {
            let value = match op {
                BinOp::And => *a && *b,
                BinOp::Or => *a || *b,
                BinOp::Xor => *a ^ *b,
                _ => return None,
            };
            Some(ConstValue::Bool(value))
        }
        _ => None,
    }
}
