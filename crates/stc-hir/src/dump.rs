//! Deterministic textual HIR dumps for tests and snapshots.

use std::fmt::Write as _;

use crate::hir::{
    ArgHir, Body, CalleeHir, ConstValue, DefKind, ExprHir, ExprKindHir, HirCrate, PlaceHir,
    StmtHir, StmtKindHir,
};
use crate::id::{BodyId, DefId, ExprId, FieldId, StmtId, TyId};

const MAX_DEPTH: usize = 64;

/// Renders a stable, human-readable HIR dump.
#[must_use]
pub fn dump_hir(hir: &HirCrate) -> String {
    let mut dumper = Dumper {
        hir,
        out: String::new(),
    };
    dumper.run();
    dumper.out
}

struct Dumper<'a> {
    hir: &'a HirCrate,
    out: String,
}

impl Dumper<'_> {
    fn run(&mut self) {
        let _ = writeln!(
            self.out,
            "# hir defs={} fields={} params={} locals={} bodies={} exprs={} stmts={} types={} std_fns={}",
            self.hir.defs.len(),
            self.hir.fields.len(),
            self.hir.params.len(),
            self.hir.locals.len(),
            self.hir.bodies.len(),
            self.hir.exprs.len(),
            self.hir.stmts.len(),
            self.hir.types.len(),
            self.hir.std_fns.len(),
        );
        for package in &self.hir.packages {
            let _ = writeln!(
                self.out,
                "package {} name={:?} root={:?}",
                package.id.raw(),
                package.name,
                package.root_namespace
            );
        }
        let _ = writeln!(self.out, "[types]");
        let types: Vec<(TyId, String)> = self
            .hir
            .types
            .iter()
            .map(|(id, _)| (id, self.hir.type_name(id)))
            .collect();
        for (id, name) in types {
            let _ = writeln!(self.out, "ty {} = {}", id.raw(), name);
        }
        let _ = writeln!(self.out, "[defs]");
        for index in 0..self.hir.defs.len() {
            self.dump_def(DefId::new(index as u32));
        }
        let _ = writeln!(self.out, "[fields]");
        for index in 0..self.hir.fields.len() {
            let field = &self.hir.fields[index];
            let _ = writeln!(
                self.out,
                "field {} {}: {} owner={} section={} vis={} retain={:?} at={} io={}",
                index,
                self.hir.interner.text(field.name),
                self.hir.type_name(field.ty),
                field.owner.raw(),
                field.section.as_str(),
                field.visibility.as_str(),
                field.retain,
                dump_direct(&field.at),
                dump_io(&field.io),
            );
        }
        let _ = writeln!(self.out, "[params]");
        for index in 0..self.hir.params.len() {
            let param = &self.hir.params[index];
            let _ = writeln!(
                self.out,
                "param {} {}: {} dir={:?} owner={} default={}",
                index,
                self.hir.interner.text(param.name),
                self.hir.type_name(param.ty),
                param.direction,
                param.owner.raw(),
                param
                    .default
                    .map_or("-".to_string(), |id| id.raw().to_string()),
            );
        }
        let _ = writeln!(self.out, "[locals]");
        for index in 0..self.hir.locals.len() {
            let local = &self.hir.locals[index];
            let _ = writeln!(
                self.out,
                "local {} {}: {} owner={} temp={} retain={:?} init={}",
                index,
                self.hir.interner.text(local.name),
                self.hir.type_name(local.ty),
                local.owner.raw(),
                local.is_temp,
                local.retain,
                local
                    .init
                    .map_or("-".to_string(), |id| id.raw().to_string()),
            );
        }
        let _ = writeln!(self.out, "[bodies]");
        let bodies: Vec<Body> = self.hir.bodies.clone();
        for (index, body) in bodies.iter().enumerate() {
            self.dump_body(BodyId::new(index as u32), body);
        }
    }

    fn dump_def(&mut self, id: DefId) {
        let def = &self.hir.defs[id.index()];
        let name = def.name_text(&self.hir.interner);
        let span = format_span(def.span);
        let _ = write!(
            self.out,
            "def {} {} {:?} @{} parent={} vis={}",
            id.raw(),
            kind_name(&def.kind),
            name,
            span,
            def.parent
                .map_or("-".to_string(), |parent| parent.raw().to_string()),
            def.visibility.as_str(),
        );
        if def.test_only {
            let _ = write!(self.out, " test");
        }
        if def.export {
            let name = def
                .export_name
                .map_or("", |symbol| self.hir.interner.text(symbol));
            let _ = write!(self.out, " export={name}");
        }
        let _ = writeln!(self.out);
        match &def.kind {
            DefKind::Namespace { internal } => {
                let _ = writeln!(self.out, "  internal={internal}");
            }
            DefKind::Alias { target } => {
                let _ = writeln!(self.out, "  target={}", self.hir.type_name(*target));
            }
            DefKind::Subrange {
                base,
                lower,
                upper,
                default,
            } => {
                let _ = writeln!(
                    self.out,
                    "  base={} range={lower}..={upper} default={default:?}",
                    self.hir.type_name(*base)
                );
            }
            DefKind::Enum {
                base,
                values,
                default,
            }
            | DefKind::NamedValue {
                base,
                values,
                default,
            } => {
                let _ = writeln!(self.out, "  base={}", self.hir.type_name(*base));
                for value in values {
                    let _ = writeln!(
                        self.out,
                        "  value {} = {}",
                        self.hir.interner.text(value.name),
                        value.value
                    );
                }
                if let Some(default) = default {
                    let _ = writeln!(self.out, "  default={}", self.hir.interner.text(*default));
                }
            }
            DefKind::Struct { fields, overlap } => {
                let _ = writeln!(self.out, "  overlap={overlap}");
                self.dump_field_ids(fields);
            }
            DefKind::Function {
                result,
                params,
                body,
            } => {
                let _ = writeln!(
                    self.out,
                    "  result={} params={:?} body={}",
                    result.map_or("-".to_string(), |ty| self.hir.type_name(ty)),
                    params.iter().map(|id| id.raw()).collect::<Vec<_>>(),
                    body.map_or("-".to_string(), |id| id.raw().to_string()),
                );
            }
            DefKind::FunctionBlock {
                base,
                interfaces,
                fields,
                methods,
                body,
                is_final,
                is_abstract,
                vtable,
            } => {
                let _ = writeln!(
                    self.out,
                    "  base={} interfaces={:?} final={is_final} abstract={is_abstract} body={}",
                    base.map_or("-".to_string(), |id| id.raw().to_string()),
                    interfaces.iter().map(|id| id.raw()).collect::<Vec<_>>(),
                    body.map_or("-".to_string(), |id| id.raw().to_string()),
                );
                self.dump_field_ids(fields);
                let _ = writeln!(
                    self.out,
                    "  methods={:?} vtable={:?}",
                    methods.iter().map(|id| id.raw()).collect::<Vec<_>>(),
                    vtable.iter().map(|id| id.raw()).collect::<Vec<_>>(),
                );
            }
            DefKind::Class {
                base,
                interfaces,
                fields,
                methods,
                is_final,
                is_abstract,
                vtable,
            } => {
                let _ = writeln!(
                    self.out,
                    "  base={} interfaces={:?} final={is_final} abstract={is_abstract}",
                    base.map_or("-".to_string(), |id| id.raw().to_string()),
                    interfaces.iter().map(|id| id.raw()).collect::<Vec<_>>(),
                );
                self.dump_field_ids(fields);
                let _ = writeln!(
                    self.out,
                    "  methods={:?} vtable={:?}",
                    methods.iter().map(|id| id.raw()).collect::<Vec<_>>(),
                    vtable.iter().map(|id| id.raw()).collect::<Vec<_>>(),
                );
            }
            DefKind::Program {
                fields,
                body,
                var_access,
            } => {
                let _ = writeln!(
                    self.out,
                    "  body={} var_access={}",
                    body.map_or("-".to_string(), |id| id.raw().to_string()),
                    var_access.len()
                );
                self.dump_field_ids(fields);
            }
            DefKind::Interface {
                extends,
                methods,
                vtable,
            } => {
                let _ = writeln!(
                    self.out,
                    "  extends={:?} methods={:?} vtable={:?}",
                    extends.iter().map(|id| id.raw()).collect::<Vec<_>>(),
                    methods.iter().map(|id| id.raw()).collect::<Vec<_>>(),
                    vtable.iter().map(|id| id.raw()).collect::<Vec<_>>(),
                );
            }
            DefKind::Method {
                owner,
                result,
                params,
                body,
                access,
                is_override,
                is_final,
                is_abstract,
                slot,
            } => {
                let _ = writeln!(
                    self.out,
                    "  owner={} result={} params={:?} body={} access={} override={is_override} final={is_final} abstract={is_abstract} slot={slot:?}",
                    owner.raw(),
                    result.map_or("-".to_string(), |ty| self.hir.type_name(ty)),
                    params.iter().map(|id| id.raw()).collect::<Vec<_>>(),
                    body.map_or("-".to_string(), |id| id.raw().to_string()),
                    access.as_str(),
                );
            }
            DefKind::Global {
                ty,
                section,
                retain,
                constant,
                at,
                io,
                init,
            } => {
                let _ = writeln!(
                    self.out,
                    "  ty={} section={} retain={retain:?} constant={constant} at={} io={} init={}",
                    self.hir.type_name(*ty),
                    section.as_str(),
                    dump_direct(at),
                    dump_io(io),
                    init.map_or("-".to_string(), |id| id.raw().to_string()),
                );
            }
            DefKind::Configuration { config } => {
                let _ = writeln!(
                    self.out,
                    "  resources={} programs={} var_access={} config_inits={}",
                    config.resources.len(),
                    config
                        .resources
                        .iter()
                        .map(|resource| resource.programs.len())
                        .sum::<usize>(),
                    config.var_access.len(),
                    config.config_inits.len(),
                );
            }
        }
    }

    fn dump_field_ids(&mut self, fields: &[FieldId]) {
        for &field in fields {
            let field = &self.hir.fields[field.index()];
            let _ = writeln!(
                self.out,
                "  field {}: {} {}",
                self.hir.interner.text(field.name),
                self.hir.type_name(field.ty),
                field.section.as_str(),
            );
        }
    }

    fn dump_body(&mut self, id: BodyId, body: &Body) {
        let _ = writeln!(
            self.out,
            "body {} owner={} params={:?} locals={:?}",
            id.raw(),
            body.owner.raw(),
            body.params
                .iter()
                .map(|param| param.raw())
                .collect::<Vec<_>>(),
            body.locals
                .iter()
                .map(|local| local.raw())
                .collect::<Vec<_>>(),
        );
        self.dump_stmt_list(&body.stmts, 1, 0);
    }

    fn dump_stmt_list(&mut self, stmts: &[StmtId], indent: usize, depth: usize) {
        if depth > MAX_DEPTH {
            self.line(indent, "...");
            return;
        }
        for &id in stmts {
            let stmt = self.hir.stmt(id).clone();
            self.dump_stmt(id, &stmt, indent, depth);
        }
    }

    fn dump_stmt(&mut self, id: StmtId, stmt: &StmtHir, indent: usize, depth: usize) {
        let prefix = format!("stmt {} @{} ", id.raw(), format_span(stmt.span));
        match &stmt.kind {
            StmtKindHir::Assign { place, value } => {
                self.line(indent, &format!("{prefix}assign"));
                self.dump_expr(*place, indent + 1, depth + 1);
                self.dump_expr(*value, indent + 1, depth + 1);
            }
            StmtKindHir::RefAssign { place, value } => {
                self.line(indent, &format!("{prefix}ref-assign"));
                self.dump_expr(*place, indent + 1, depth + 1);
                self.dump_expr(*value, indent + 1, depth + 1);
            }
            StmtKindHir::Attempt { place, value } => {
                self.line(indent, &format!("{prefix}attempt"));
                self.dump_expr(*place, indent + 1, depth + 1);
                self.dump_expr(*value, indent + 1, depth + 1);
            }
            StmtKindHir::Call { call } => {
                self.line(indent, &format!("{prefix}call"));
                self.dump_expr(*call, indent + 1, depth + 1);
            }
            StmtKindHir::Return => self.line(indent, &format!("{prefix}return")),
            StmtKindHir::If { arms, else_body } => {
                self.line(indent, &format!("{prefix}if"));
                for (condition, body) in arms {
                    self.line(indent + 1, "arm");
                    self.dump_expr(*condition, indent + 2, depth + 1);
                    self.dump_stmt_list(body, indent + 2, depth + 1);
                }
                if !else_body.is_empty() {
                    self.line(indent + 1, "else");
                    self.dump_stmt_list(else_body, indent + 2, depth + 1);
                }
            }
            StmtKindHir::Case {
                selector,
                arms,
                else_body,
            } => {
                self.line(indent, &format!("{prefix}case"));
                self.dump_expr(*selector, indent + 1, depth + 1);
                for arm in arms {
                    for label in &arm.labels {
                        let _ = writeln!(
                            self.out,
                            "{}label {:?}..{:?}",
                            "  ".repeat(indent + 1),
                            label.lower,
                            label.upper,
                        );
                    }
                    self.dump_stmt_list(&arm.body, indent + 1, depth + 1);
                }
                if !else_body.is_empty() {
                    self.line(indent + 1, "else");
                    self.dump_stmt_list(else_body, indent + 2, depth + 1);
                }
            }
            StmtKindHir::For {
                var,
                start,
                end,
                step,
                body,
            } => {
                self.line(indent, &format!("{prefix}for"));
                self.dump_expr(*var, indent + 1, depth + 1);
                self.dump_expr(*start, indent + 1, depth + 1);
                self.dump_expr(*end, indent + 1, depth + 1);
                self.dump_expr(*step, indent + 1, depth + 1);
                self.dump_stmt_list(body, indent + 1, depth + 1);
            }
            StmtKindHir::While { cond, body } => {
                self.line(indent, &format!("{prefix}while"));
                self.dump_expr(*cond, indent + 1, depth + 1);
                self.dump_stmt_list(body, indent + 1, depth + 1);
            }
            StmtKindHir::Repeat { body, cond } => {
                self.line(indent, &format!("{prefix}repeat"));
                self.dump_stmt_list(body, indent + 1, depth + 1);
                self.dump_expr(*cond, indent + 1, depth + 1);
            }
            StmtKindHir::Exit => self.line(indent, &format!("{prefix}exit")),
            StmtKindHir::Continue => self.line(indent, &format!("{prefix}continue")),
            StmtKindHir::Empty => self.line(indent, &format!("{prefix}empty")),
        }
    }

    fn dump_expr(&mut self, id: ExprId, indent: usize, depth: usize) {
        if depth > MAX_DEPTH {
            self.line(indent, "...");
            return;
        }
        let expr = self.hir.expr(id).clone();
        self.dump_expr_inner(id, &expr, indent, depth);
    }

    fn dump_expr_inner(&mut self, id: ExprId, expr: &ExprHir, indent: usize, depth: usize) {
        let prefix = format!(
            "expr {} : {} @{} ",
            id.raw(),
            self.hir.type_name(expr.ty),
            format_span(expr.span)
        );
        match &expr.kind {
            ExprKindHir::Literal(literal) => {
                let _ = writeln!(
                    self.out,
                    "{}{prefix}literal {}",
                    "  ".repeat(indent),
                    dump_const(&literal.value)
                );
            }
            ExprKindHir::Place(place) => {
                let _ = writeln!(
                    self.out,
                    "{}{prefix}place {}",
                    "  ".repeat(indent),
                    dump_place(self.hir, place)
                );
            }
            ExprKindHir::Binary { op, lhs, rhs } => {
                self.line(indent, &format!("{prefix}binary {}", op.as_str()));
                self.dump_expr(*lhs, indent + 1, depth + 1);
                self.dump_expr(*rhs, indent + 1, depth + 1);
            }
            ExprKindHir::Unary { op, operand } => {
                self.line(indent, &format!("{prefix}unary {}", op.as_str()));
                self.dump_expr(*operand, indent + 1, depth + 1);
            }
            ExprKindHir::Call { callee, args } => {
                self.line(
                    indent,
                    &format!("{prefix}call {}", dump_callee(self.hir, callee)),
                );
                for arg in args {
                    self.dump_arg(arg, indent + 1, depth + 1);
                }
            }
            ExprKindHir::Ref { target } => {
                self.line(indent, &format!("{prefix}ref"));
                self.dump_expr(*target, indent + 1, depth + 1);
            }
            ExprKindHir::AddressOf { target } => {
                self.line(indent, &format!("{prefix}address-of"));
                self.dump_expr(*target, indent + 1, depth + 1);
            }
            ExprKindHir::SizeOf { operand } => match operand {
                crate::hir::SizeOfHir::Type(ty) => {
                    self.line(
                        indent,
                        &format!("{prefix}sizeof-type {}", self.hir.type_name(*ty)),
                    );
                }
                crate::hir::SizeOfHir::Place(place) => {
                    self.line(indent, &format!("{prefix}sizeof-place"));
                    self.dump_expr(*place, indent + 1, depth + 1);
                }
            },
            ExprKindHir::PointerConversion { operand, to, .. } => {
                self.line(
                    indent,
                    &format!("{prefix}pointer-conversion {}", self.hir.type_name(*to)),
                );
                self.dump_expr(*operand, indent + 1, depth + 1);
            }
            ExprKindHir::Null => self.line(indent, &format!("{prefix}null")),
            ExprKindHir::Cast { operand, to, kind } => {
                self.line(
                    indent,
                    &format!("{prefix}cast {kind:?} {}", self.hir.type_name(*to)),
                );
                self.dump_expr(*operand, indent + 1, depth + 1);
            }
            ExprKindHir::ArrayInit { elements } => {
                self.line(indent, &format!("{prefix}array-init"));
                for element in elements {
                    self.dump_expr(*element, indent + 1, depth + 1);
                }
            }
            ExprKindHir::StructInit { fields } => {
                self.line(indent, &format!("{prefix}struct-init"));
                for (field, value) in fields {
                    let _ = writeln!(
                        self.out,
                        "{}field {}",
                        "  ".repeat(indent + 1),
                        self.hir.interner.text(self.hir.field(*field).name)
                    );
                    self.dump_expr(*value, indent + 2, depth + 1);
                }
            }
            ExprKindHir::Error => self.line(indent, &format!("{prefix}error")),
        }
    }

    fn dump_arg(&mut self, arg: &ArgHir, indent: usize, depth: usize) {
        let formal = arg.formal.map_or("-".to_string(), |param| {
            self.hir
                .interner
                .text(self.hir.param(param).name)
                .to_string()
        });
        let field = arg
            .field
            .map_or("-".to_string(), |field| field.raw().to_string());
        let _ = writeln!(
            self.out,
            "{}arg formal={} field={} dir={:?} negated={} target={}",
            "  ".repeat(indent),
            formal,
            field,
            arg.direction,
            arg.negated,
            arg.target
                .map_or("-".to_string(), |id| id.raw().to_string()),
        );
        if let Some(value) = arg.value {
            self.dump_expr(value, indent + 1, depth + 1);
        }
        if let Some(target) = arg.target {
            self.dump_expr(target, indent + 1, depth + 1);
        }
    }

    fn line(&mut self, indent: usize, text: &str) {
        let _ = writeln!(self.out, "{}{}", "  ".repeat(indent), text);
    }
}

fn kind_name(kind: &DefKind) -> &'static str {
    match kind {
        DefKind::Namespace { .. } => "namespace",
        DefKind::Alias { .. } => "alias",
        DefKind::Subrange { .. } => "subrange",
        DefKind::Enum { .. } => "enum",
        DefKind::NamedValue { .. } => "named-value",
        DefKind::Struct { .. } => "struct",
        DefKind::Function { .. } => "function",
        DefKind::FunctionBlock { .. } => "function-block",
        DefKind::Program { .. } => "program",
        DefKind::Class { .. } => "class",
        DefKind::Interface { .. } => "interface",
        DefKind::Method { .. } => "method",
        DefKind::Global { .. } => "global",
        DefKind::Configuration { .. } => "configuration",
    }
}

fn dump_place(hir: &HirCrate, place: &PlaceHir) -> String {
    match place {
        PlaceHir::Local(id) => format!("local#{}", id.raw()),
        PlaceHir::Param(id) => format!("param#{}", id.raw()),
        PlaceHir::Global(id) => format!("global#{} ({})", id.raw(), hir.def_name(*id)),
        PlaceHir::Result(id) => format!("result#{} ({})", id.raw(), hir.def_name(*id)),
        PlaceHir::This(id) => format!("this#{}", id.raw()),
        PlaceHir::Field { base, field } => format!(
            "field#{} ({}) of expr#{}",
            field.raw(),
            hir.interner.text(hir.field(*field).name),
            base.raw()
        ),
        PlaceHir::Index { base, indices } => format!(
            "index of expr#{} with {:?}",
            base.raw(),
            indices.iter().map(|id| id.raw()).collect::<Vec<_>>()
        ),
        PlaceHir::Deref(expr) => format!("deref expr#{}", expr.raw()),
        PlaceHir::BitField { base, bit } => format!("bit {bit} of expr#{}", base.raw()),
        PlaceHir::Direct(address) => format!("direct {}", dump_direct(&Some(address.clone()))),
        PlaceHir::InstanceField {
            program,
            instance,
            fields,
        } => format!(
            "instance-field {}#{} fields={:?}",
            hir.def_name(*program),
            hir.interner.text(*instance),
            fields.iter().map(|field| field.raw()).collect::<Vec<_>>()
        ),
    }
}

fn dump_callee(hir: &HirCrate, callee: &CalleeHir) -> String {
    match callee {
        CalleeHir::Function(id) => format!("function#{} ({})", id.raw(), hir.def_name(*id)),
        CalleeHir::StdFunction(id) => format!(
            "std#{} ({})",
            id.raw(),
            hir.std_fns[id.index()].kind.as_str()
        ),
        CalleeHir::Conversion { from, to } => {
            format!("convert {} -> {}", hir.type_name(*from), hir.type_name(*to))
        }
        CalleeHir::Method { receiver, def } => format!(
            "method#{} ({}) on expr#{}",
            def.raw(),
            hir.def_name(*def),
            receiver.raw()
        ),
        CalleeHir::SuperMethod { receiver, def } => format!(
            "super-method#{} ({}) on expr#{}",
            def.raw(),
            hir.def_name(*def),
            receiver.raw()
        ),
        CalleeHir::InterfaceMethod {
            receiver,
            interface,
            method,
        } => format!(
            "interface-method#{} ({}) of iface#{} on expr#{}",
            method.raw(),
            hir.def_name(*method),
            interface.raw(),
            receiver.raw()
        ),
        CalleeHir::Fb { instance, def } => format!(
            "fb#{} ({}) on expr#{}",
            def.raw(),
            hir.def_name(*def),
            instance.raw()
        ),
        CalleeHir::SuperBody { instance, base } => {
            format!("super-body#{} on expr#{}", base.raw(), instance.raw())
        }
    }
}

fn dump_const(value: &ConstValue) -> String {
    match value {
        ConstValue::Int(value) => format!("Int({value})"),
        ConstValue::Real(value) => format!("Real({value})"),
        ConstValue::Bool(value) => format!("Bool({value})"),
        ConstValue::Char(value) => format!("Char({value})"),
        ConstValue::String(value) => format!("String({value:?})"),
        ConstValue::Time(value) => format!("Time({value})"),
        ConstValue::Date { kind, value } => format!("Date({kind:?}, {value})"),
        ConstValue::Enum { def, value } => format!("Enum(#{}, {value})", def.raw()),
        ConstValue::Null => "Null".into(),
    }
}

fn dump_io(hint: &Option<crate::hir::IoHint>) -> String {
    let Some(hint) = hint else {
        return "-".into();
    };
    format!(
        "device={} channel={} direction={}",
        hint.device.as_deref().unwrap_or("-"),
        hint.channel
            .map_or_else(|| "-".to_string(), |channel| channel.to_string()),
        hint.direction.map_or("-", |direction| direction.as_str()),
    )
}

fn dump_direct(address: &Option<crate::DirectAddress>) -> String {
    let Some(address) = address else {
        return "-".into();
    };
    if address.wildcard {
        return "%*".into();
    }
    let mut out = String::from("%");
    if let Some(area) = address.area {
        out.push(area.as_char());
    }
    if let Some(size) = address.size {
        out.push(size.as_char());
    }
    for (index, value) in address.indices.iter().enumerate() {
        if index > 0 {
            out.push('.');
        }
        out.push_str(&value.to_string());
    }
    out
}

fn format_span(span: stc_span::Span) -> String {
    format!("{}:{}..{}", span.file.raw(), span.start, span.end)
}
