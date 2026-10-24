//! Module units: functions, programs, globals, init/cycle/shutdown, exports,
//! retained state, and metadata assembly.

use std::collections::{BTreeMap, BTreeSet};

use stc_hir::{ArgDirection, DefId, DefKind, ExprId, HirCrate, Symbol, TyId, VarSectionKind};
use stc_span::Span;

use crate::abi::{self, symbols};
use crate::config::{
    self, ImageLayout, Plan, PlanInstance, ProgramLayout, RetainLayout, RetainPlace,
};
use crate::expr::BodyInfo;
use crate::ir::{Function, GlobalDef, IrType, Module, SymbolAttrs, Value, c_string};
use crate::layout::{self, ScalarKind};
use crate::mangle;
use crate::metadata::{
    ExportMetadata, HintMetadata, IoBindingMetadata, Metadata as MetadataModel, ProgramMetadata,
    RetainLeafMetadata as MetaRetainLeaf, SpanMetadata, SpanMetadataEntry, TaskMetadata,
};
use crate::stmt::LoopTarget;
use crate::{CodegenError, Metadata, Mode, Options, Output, SpanEntry};

/// The code generation state for one build.
pub(crate) struct Lowerer<'a> {
    pub hir: &'a HirCrate,
    pub options: &'a Options<'a>,
    pub module: Module,
    pub layouts: BTreeMap<DefId, ProgramLayout>,
    pub plan: Plan,
    pub image: ImageLayout,
    pub retain: RetainLayout,
    pub function_symbols: BTreeMap<DefId, String>,
    pub global_symbols: BTreeMap<DefId, String>,
    pub instance_globals: BTreeMap<Symbol, String>,
    pub span_ids: BTreeMap<Span, u32>,
    pub span_entries: Vec<SpanEntry>,
    pub errors: Vec<CodegenError>,
    seen_errors: BTreeSet<String>,
}

/// Generates IR and metadata for a checked HIR crate.
pub(crate) fn generate(hir: &HirCrate, options: &Options<'_>) -> Result<Output, Vec<CodegenError>> {
    let plan = config::select_plan(hir)?;
    let layouts = config::program_layouts(hir, &plan)?;
    let image = config::image_layout(hir, &plan, &layouts)?;
    let retain = config::retain_layout(hir, &plan, &layouts)?;
    let (span_ids, span_entries) = build_span_table(hir);
    let mut lowerer = Lowerer {
        hir,
        options,
        module: Module::new(options.artifact_name, options.target),
        layouts,
        plan,
        image,
        retain,
        function_symbols: BTreeMap::new(),
        global_symbols: BTreeMap::new(),
        instance_globals: BTreeMap::new(),
        span_ids,
        span_entries,
        errors: Vec::new(),
        seen_errors: BTreeSet::new(),
    };
    lowerer.build_symbols();
    lowerer.preflight();
    lowerer.emit();
    if !lowerer.errors.is_empty() {
        return Err(lowerer.errors);
    }
    let metadata = lowerer.build_metadata();
    if lowerer.options.mode == Mode::Library {
        lowerer.add_metadata_symbol(&metadata);
    }
    lowerer
        .module
        .check()
        .map_err(|message| vec![CodegenError::internal(message)])?;
    let module = lowerer.module.render();
    Ok(Output {
        module,
        metadata,
        spans: lowerer.span_entries.clone(),
    })
}

impl Lowerer<'_> {
    fn push_error(&mut self, error: CodegenError) {
        let key = format!("{}|{}|{:?}", error.code(), error.message(), error.span());
        if self.seen_errors.insert(key) {
            self.errors.push(error);
        }
    }

    pub(crate) fn span_id(&self, span: Span) -> u32 {
        self.span_ids.get(&span).copied().unwrap_or(0)
    }

    fn build_symbols(&mut self) {
        for (index, def) in self.hir.defs.iter().enumerate() {
            let id = DefId::new(u32::try_from(index).unwrap_or(0));
            if synthetic(self.hir, id) || def.test_only {
                continue;
            }
            match &def.kind {
                DefKind::Function { body: Some(_), .. } => {
                    match mangle::function_symbol(self.hir, id) {
                        Ok(symbol) => {
                            if self
                                .function_symbols
                                .values()
                                .any(|existing| *existing == symbol)
                            {
                                self.push_error(CodegenError::unsupported(
                                    crate::codes::CODEGEN_UNSUPPORTED,
                                    format!(
                                        "two functions have the same mangled signature `{symbol}`"
                                    ),
                                    def.span,
                                ));
                            } else {
                                self.function_symbols.insert(id, symbol);
                            }
                        }
                        Err(error) => self.push_error(error.with_span(def.span)),
                    }
                }
                DefKind::Global { at: None, .. } => {
                    self.global_symbols
                        .insert(id, mangle::global_symbol(self.hir, id));
                }
                _ => {}
            }
        }
        for instance in &self.plan.instances {
            self.instance_globals
                .insert(instance.name, instance.global.clone());
        }
    }

    fn preflight(&mut self) {
        for (index, def) in self.hir.defs.iter().enumerate() {
            let id = DefId::new(u32::try_from(index).unwrap_or(0));
            if synthetic(self.hir, id) || def.test_only {
                continue;
            }
            match &def.kind {
                DefKind::Function {
                    result,
                    params,
                    body,
                } => {
                    if body.is_none() {
                        self.push_error(CodegenError::unsupported(
                            crate::codes::CODEGEN_UNSUPPORTED,
                            "a FUNCTION without a body is not supported in M4".into(),
                            def.span,
                        ));
                    }
                    if let Some(result) = result
                        && let Err(error) = layout::scalar_kind_at(self.hir, *result, def.span)
                    {
                        self.push_error(error);
                    }
                    for param in params {
                        let data = self.hir.param(*param);
                        if data.direction != ArgDirection::Input {
                            self.push_error(CodegenError::unsupported(
                                crate::codes::CODEGEN_UNSUPPORTED,
                                format!(
                                    "`{}` parameters are M5",
                                    match data.direction {
                                        ArgDirection::Output => "VAR_OUTPUT",
                                        ArgDirection::InOut => "VAR_IN_OUT",
                                        ArgDirection::Input => "VAR_INPUT",
                                    }
                                ),
                                data.span,
                            ));
                        }
                        if let Err(error) = layout::scalar_kind_at(self.hir, data.ty, data.span) {
                            self.push_error(error);
                        }
                    }
                }
                DefKind::Global { ty, .. } => {
                    if let Err(error) = layout::scalar_kind_at(self.hir, *ty, def.span) {
                        self.push_error(error);
                    }
                }
                DefKind::Program { fields, .. } => {
                    for field in fields {
                        let data = self.hir.field(*field);
                        if matches!(
                            data.section,
                            VarSectionKind::InOut | VarSectionKind::Temp | VarSectionKind::External
                        ) {
                            self.push_error(CodegenError::unsupported(
                                crate::codes::CODEGEN_UNSUPPORTED,
                                format!(
                                    "program field section `{}` is not supported in M4",
                                    data.section.as_str()
                                ),
                                data.span,
                            ));
                        }
                        if let Err(error) = layout::scalar_kind_at(self.hir, data.ty, data.span) {
                            self.push_error(error);
                        }
                    }
                }
                DefKind::Struct { .. } => self.push_error(CodegenError::unsupported(
                    crate::codes::CODEGEN_UNSUPPORTED,
                    "STRUCT declarations are M5".into(),
                    def.span,
                )),
                DefKind::Enum { .. } => self.push_error(CodegenError::unsupported(
                    crate::codes::CODEGEN_UNSUPPORTED,
                    "ENUM declarations are M5".into(),
                    def.span,
                )),
                DefKind::NamedValue { .. } => self.push_error(CodegenError::unsupported(
                    crate::codes::CODEGEN_UNSUPPORTED,
                    "named-value type declarations are M5".into(),
                    def.span,
                )),
                DefKind::Subrange { .. } => self.push_error(CodegenError::unsupported(
                    crate::codes::CODEGEN_UNSUPPORTED,
                    "subrange type declarations are M5".into(),
                    def.span,
                )),
                DefKind::Class { .. } => self.push_error(CodegenError::unsupported(
                    crate::codes::CODEGEN_UNSUPPORTED,
                    "CLASS declarations are M5".into(),
                    def.span,
                )),
                DefKind::FunctionBlock { .. } => self.push_error(CodegenError::unsupported(
                    crate::codes::CODEGEN_UNSUPPORTED,
                    "FUNCTION_BLOCK declarations are M5".into(),
                    def.span,
                )),
                DefKind::Interface { .. } => self.push_error(CodegenError::unsupported(
                    crate::codes::CODEGEN_UNSUPPORTED,
                    "INTERFACE declarations are M5".into(),
                    def.span,
                )),
                DefKind::Namespace { .. }
                | DefKind::Alias { .. }
                | DefKind::Method { .. }
                | DefKind::Configuration { .. } => {}
            }
        }
    }

    fn emit(&mut self) {
        self.emit_types_and_globals();
        self.emit_declarations();
        self.emit_user_functions();
        self.emit_program_bodies();
        self.emit_init();
        self.emit_cycle();
        self.emit_shutdown();
        self.emit_retain_functions();
        self.emit_export_wrappers();
        match self.options.mode {
            Mode::Executable => self.emit_main(),
            // The runtime owns `stc_abi_version` (Runtime §2.2); the generated
            // library must not define a duplicate symbol (D-086).
            Mode::Library => {}
        }
    }

    fn emit_types_and_globals(&mut self) {
        for layout in self.layouts.values() {
            let fields = layout
                .fields
                .iter()
                .filter_map(|field| {
                    let data = self.hir.field(*field);
                    layout::scalar_kind_at(self.hir, data.ty, data.span)
                        .ok()
                        .map(|kind| kind.storage().render())
                })
                .collect::<Vec<_>>();
            self.module
                .add_type(&layout.name, &format!("{{ {} }}", fields.join(", ")));
        }
        let globals: Vec<(DefId, TyId, Span)> = self
            .hir
            .defs
            .iter()
            .enumerate()
            .filter_map(|(index, def)| {
                let id = DefId::new(u32::try_from(index).ok()?);
                let DefKind::Global { ty, at: None, .. } = &def.kind else {
                    return None;
                };
                Some((id, *ty, def.span))
            })
            .collect();
        for (def, ty, span) in globals {
            let Ok(kind) = layout::scalar_kind_at(self.hir, ty, span) else {
                continue;
            };
            let Some(symbol) = self.global_symbols.get(&def).cloned() else {
                continue;
            };
            self.module.add_global(GlobalDef {
                name: symbol,
                attrs: SymbolAttrs::internal(),
                ty: kind.storage().render(),
                init: "zeroinitializer".into(),
                constant: false,
                align: None,
            });
        }
        for instance in &self.plan.instances {
            let Some(layout) = self.layouts.get(&instance.program) else {
                continue;
            };
            self.module.add_global(GlobalDef {
                name: instance.global.clone(),
                attrs: SymbolAttrs::internal(),
                ty: format!("%{}", layout.name),
                init: "zeroinitializer".into(),
                constant: false,
                align: None,
            });
        }
        self.module.add_global(GlobalDef {
            name: symbols::IO_BASE.into(),
            attrs: SymbolAttrs::internal(),
            ty: "ptr".into(),
            init: "null".into(),
            constant: false,
            align: None,
        });
        self.module.add_global(GlobalDef {
            name: symbols::IO_LEN.into(),
            attrs: SymbolAttrs::internal(),
            ty: "i64".into(),
            init: "0".into(),
            constant: false,
            align: None,
        });
        let buffer_len = self.retain.size.max(1);
        self.module.add_global(GlobalDef {
            name: symbols::RETAIN_BUFFER.into(),
            attrs: SymbolAttrs::internal(),
            ty: format!("[{buffer_len} x i8]"),
            init: "zeroinitializer".into(),
            constant: false,
            align: None,
        });
        for task in &self.plan.tasks {
            if task.single != Some(true) {
                continue;
            }
            let guard = mangle::task_guard_symbol(&task.resource, task.name, self.hir);
            self.module.add_global(GlobalDef {
                name: guard,
                attrs: SymbolAttrs::internal(),
                ty: "i1".into(),
                init: "true".into(),
                constant: false,
                align: None,
            });
        }
    }

    fn emit_declarations(&mut self) {
        self.module
            .declare(symbols::TRAP, "void @stc_rt_trap(i32, i32)");
        self.module.declare(
            symbols::EXECUTE,
            "i32 @stc_rt_execute(i32, ptr, ptr, ptr, ptr)",
        );
        self.module
            .declare(symbols::CYCLE_NOW, "i64 @stc_rt_cycle_now_ns()");
        self.module
            .declare(symbols::MONOTONIC_NOW, "i64 @stc_rt_monotonic_now_ns()");
        self.module.declare(
            symbols::SET_CYCLE_TIME,
            "void @stc_rt_set_cycle_time_ns(i64)",
        );
        self.module.declare(
            symbols::IO_CONFIGURE,
            "void @stc_rt_io_configure(i64, i64, i64)",
        );
        self.module
            .declare(symbols::IO_IMAGE, "void @stc_rt_io_image(ptr, ptr)");
        self.module
            .declare(symbols::STATE_LOAD, "i32 @stc_rt_state_load(ptr, i64, i64)");
        self.module
            .declare(symbols::STATE_SAVE, "i32 @stc_rt_state_save(ptr, i64, i64)");
    }

    fn emit_user_functions(&mut self) {
        let functions: Vec<DefId> = self
            .hir
            .defs
            .iter()
            .enumerate()
            .filter_map(|(index, def)| {
                let id = DefId::new(u32::try_from(index).ok()?);
                matches!(def.kind, DefKind::Function { body: Some(_), .. }).then_some(id)
            })
            .collect();
        for def in functions {
            if synthetic(self.hir, def) || self.hir.defs[def.index()].test_only {
                continue;
            }
            if let Err(error) = self.emit_function(def) {
                self.push_error(error);
            }
        }
    }

    fn emit_function(&mut self, def: DefId) -> Result<(), CodegenError> {
        let DefKind::Function {
            result,
            params,
            body: Some(body_id),
        } = self.hir.defs[def.index()].kind.clone()
        else {
            return Ok(());
        };
        let symbol = self
            .function_symbols
            .get(&def)
            .cloned()
            .ok_or_else(|| CodegenError::internal("function has no symbol"))?;
        let body = self.hir.bodies[body_id.index()].clone();
        let result_kind = result
            .map(|ty| layout::scalar_kind_at(self.hir, ty, self.hir.defs[def.index()].span))
            .transpose()?;
        let mut param_data = Vec::new();
        for param in &params {
            let data = self.hir.param(*param).clone();
            let kind = layout::scalar_kind_at(self.hir, data.ty, data.span)?;
            param_data.push((*param, kind));
        }
        let ret_ty = result_kind.map_or(IrType::Void, ScalarKind::storage);
        let fn_params = param_data
            .iter()
            .enumerate()
            .map(|(index, (_, kind))| (kind.storage(), format!("%a{index}")))
            .collect();
        let mut f = Function::new(symbol, SymbolAttrs::internal(), ret_ty, fn_params);
        f.start_fresh_block();
        let mut info = BodyInfo {
            params: BTreeMap::new(),
            locals: BTreeMap::new(),
            result_ptr: None,
            result_ty: None,
            this_ptr: None,
            return_label: f.next_block_label(),
        };
        for (index, (param, kind)) in param_data.iter().enumerate() {
            let slot = f.alloca(kind.storage());
            let incoming = f.param_value(index);
            f.store(&incoming, &slot, None);
            info.params.insert(*param, slot);
        }
        if let Some(kind) = result_kind {
            let slot = f.alloca(kind.storage());
            f.store(&Value::zero(kind.storage()), &slot, None);
            info.result_ptr = Some(slot);
            info.result_ty = Some(kind.storage());
        }
        self.lower_locals(&mut f, &mut info, &body.locals)?;
        let mut loops: Vec<LoopTarget> = Vec::new();
        self.lower_stmts(&mut f, &info, &body.stmts, &mut loops)?;
        if !f.is_terminated() {
            f.branch(&info.return_label);
        }
        f.start_block(&info.return_label);
        if let (Some(pointer), Some(ty)) = (&info.result_ptr, info.result_ty) {
            let value = f.load(ty, pointer, None);
            f.ret(Some(&value));
        } else {
            f.ret(None);
        }
        f.check().map_err(CodegenError::internal)?;
        self.module.add_function(f);
        Ok(())
    }

    fn lower_locals(
        &mut self,
        f: &mut Function,
        info: &mut BodyInfo,
        locals: &[stc_hir::LocalId],
    ) -> Result<(), CodegenError> {
        for local in locals {
            let data = self.hir.local(*local).clone();
            if data.external_target.is_some() {
                continue;
            }
            let kind = layout::scalar_kind_at(self.hir, data.ty, data.span)?;
            let slot = f.alloca(kind.storage());
            info.locals.insert(*local, slot.clone());
            if let Some(init) = data.init {
                let value = self.lower_expr(f, info, init)?;
                Self::write_scalar(f, &slot, kind, &value);
            } else {
                f.store(&Value::zero(kind.storage()), &slot, None);
            }
        }
        Ok(())
    }

    fn emit_program_bodies(&mut self) {
        let programs = self.plan.programs.clone();
        for program in programs {
            if let Err(error) = self.emit_program_body(program) {
                self.push_error(error);
            }
        }
    }

    fn emit_program_body(&mut self, program: DefId) -> Result<(), CodegenError> {
        let DefKind::Program { body, .. } = self.hir.defs[program.index()].kind.clone() else {
            return Ok(());
        };
        let symbol = mangle::program_body_symbol(self.hir, program);
        let mut f = Function::new(
            symbol,
            SymbolAttrs::internal(),
            IrType::Void,
            vec![(IrType::Ptr, "%this".into())],
        );
        f.start_fresh_block();
        let mut info = BodyInfo {
            params: BTreeMap::new(),
            locals: BTreeMap::new(),
            result_ptr: None,
            result_ty: None,
            this_ptr: Some(f.param_value(0)),
            return_label: f.next_block_label(),
        };
        if let Some(body_id) = body {
            let body = self.hir.bodies[body_id.index()].clone();
            self.lower_locals(&mut f, &mut info, &body.locals)?;
            let mut loops: Vec<LoopTarget> = Vec::new();
            self.lower_stmts(&mut f, &info, &body.stmts, &mut loops)?;
        }
        if !f.is_terminated() {
            f.branch(&info.return_label);
        }
        f.start_block(&info.return_label);
        f.ret(None);
        f.check().map_err(CodegenError::internal)?;
        self.module.add_function(f);
        Ok(())
    }

    fn unit_info(f: &mut Function) -> BodyInfo {
        BodyInfo {
            params: BTreeMap::new(),
            locals: BTreeMap::new(),
            result_ptr: None,
            result_ty: None,
            this_ptr: None,
            return_label: f.next_block_label(),
        }
    }

    fn linkage(&self) -> SymbolAttrs {
        match self.options.mode {
            Mode::Executable => SymbolAttrs::internal(),
            Mode::Library => SymbolAttrs::external(),
        }
    }

    fn emit_init(&mut self) {
        let linkage = self.linkage();
        let mut f = Function::new(symbols::INIT, linkage, IrType::Void, Vec::new());
        f.start_fresh_block();
        let info = Self::unit_info(&mut f);
        if let Err(error) = self.emit_init_body(&mut f, &info) {
            self.push_error(error);
            return;
        }
        f.check()
            .map_err(CodegenError::internal)
            .unwrap_or_else(|error| {
                self.push_error(error);
            });
        self.module.add_function(f);
    }

    fn emit_init_body(&mut self, f: &mut Function, info: &BodyInfo) -> Result<(), CodegenError> {
        f.call(
            IrType::Void,
            symbols::IO_CONFIGURE,
            &[
                Value::int(64, i128::from(self.image.inputs_bytes)),
                Value::int(64, i128::from(self.image.outputs_bytes)),
                Value::int(64, i128::from(self.image.markers_bytes)),
            ],
        );
        f.call(
            IrType::Void,
            symbols::IO_IMAGE,
            &[
                Value::symbol(symbols::IO_BASE),
                Value::symbol(symbols::IO_LEN),
            ],
        );

        type GlobalInit = (
            DefId,
            TyId,
            Option<ExprId>,
            Option<stc_hir::DirectAddress>,
            Span,
        );
        let globals: Vec<GlobalInit> = self
            .hir
            .defs
            .iter()
            .enumerate()
            .filter_map(|(index, def)| {
                let DefKind::Global { ty, init, at, .. } = &def.kind else {
                    return None;
                };
                Some((
                    DefId::new(u32::try_from(index).ok()?),
                    *ty,
                    *init,
                    at.clone(),
                    def.span,
                ))
            })
            .collect();
        for (def, ty, init, at, span) in globals {
            let Some(init) = init else {
                continue;
            };
            let kind = layout::scalar_kind_at(self.hir, ty, span)?;
            let value = self.lower_expr(f, info, init)?;
            if let Some(address) = at {
                self.write_direct_initial(f, &address, kind, value, span)?;
            } else if let Some(symbol) = self.global_symbols.get(&def) {
                let pointer = Value::symbol(symbol);
                Self::write_scalar(f, &pointer, kind, &value);
            }
        }

        let instances = self.plan.instances.clone();
        for instance in &instances {
            let DefKind::Program { fields, .. } = &self.hir.defs[instance.program.index()].kind
            else {
                continue;
            };
            let fields = fields.clone();
            for field in fields {
                let data = self.hir.field(field).clone();
                let Some(init) = data.init else {
                    continue;
                };
                let kind = layout::scalar_kind_at(self.hir, data.ty, data.span)?;
                let value = self.lower_expr(f, info, init)?;
                if let Some(address) = data.at.clone() {
                    self.write_direct_initial(f, &address, kind, value, data.span)?;
                } else {
                    let pointer = self.instance_field_pointer(f, instance, field, data.span)?;
                    Self::write_scalar(f, &pointer, kind, &value);
                }
            }
        }

        let instances_by_name: BTreeMap<Symbol, PlanInstance> = self
            .plan
            .instances
            .iter()
            .map(|instance| (instance.name, instance.clone()))
            .collect();
        for init in self.plan.config_inits.clone() {
            if init.at.is_some() {
                return Err(CodegenError::unsupported(
                    crate::codes::CODEGEN_UNSUPPORTED,
                    "`VAR_CONFIG` AT overrides are not supported in M4".into(),
                    init.span,
                ));
            }
            let Some(value_expr) = init.init else {
                continue;
            };
            let Some(instance) = instances_by_name.get(&init.instance) else {
                return Err(CodegenError::internal(
                    "`VAR_CONFIG` names an unknown instance",
                ));
            };
            if init.fields.len() != 1 {
                return Err(CodegenError::unsupported(
                    crate::codes::CODEGEN_UNSUPPORTED,
                    "nested `VAR_CONFIG` field paths are M5".into(),
                    init.span,
                ));
            }
            let field = init.fields[0];
            let data = self.hir.field(field).clone();
            if data.at.is_some() {
                return Err(CodegenError::unsupported(
                    crate::codes::CODEGEN_UNSUPPORTED,
                    "`VAR_CONFIG` on a direct variable is not supported in M4".into(),
                    init.span,
                ));
            }
            let kind = layout::scalar_kind_at(self.hir, data.ty, data.span)?;
            let value = self.lower_expr(f, info, value_expr)?;
            let pointer = self.instance_field_pointer(f, instance, field, init.span)?;
            Self::write_scalar(f, &pointer, kind, &value);
        }

        let load = f
            .call(
                IrType::Int(32),
                symbols::STATE_LOAD,
                &[
                    Value::symbol(symbols::RETAIN_BUFFER),
                    Value::int(64, i128::from(self.retain.size)),
                    Value::int(64, i128::from(self.retain.hash as i64)),
                ],
            )
            .ok_or_else(|| CodegenError::internal("state load returned void"))?;
        let ok = f.icmp("eq", &load, &Value::int(32, 0));
        let restore = f.next_block_label();
        let done = f.next_block_label();
        f.cond_branch(&ok, &restore, &done);
        f.start_block(&restore);
        f.call(
            IrType::Int(32),
            symbols::RETAIN_LOAD,
            &[Value::symbol(symbols::RETAIN_BUFFER)],
        );
        f.branch(&done);
        f.start_block(&done);
        f.ret(None);
        Ok(())
    }

    fn instance_field_pointer(
        &mut self,
        f: &mut Function,
        instance: &PlanInstance,
        field: stc_hir::FieldId,
        span: Span,
    ) -> Result<Value, CodegenError> {
        let Some(layout) = self.layouts.get(&instance.program) else {
            return Err(CodegenError::internal("missing program layout"));
        };
        let Some(index) = layout.field_indices.get(&field) else {
            return Err(CodegenError::unsupported(
                crate::codes::CODEGEN_UNSUPPORTED,
                "a direct-variable field has no ordinary storage".into(),
                span,
            ));
        };
        let base = Value::symbol(&instance.global);
        Ok(f.gep(
            &format!("%{}", layout.name),
            &base,
            &["i32 0".into(), format!("i32 {index}")],
            true,
        ))
    }

    fn emit_cycle(&mut self) {
        let mut f = Function::new(symbols::CYCLE, self.linkage(), IrType::Void, Vec::new());
        f.start_fresh_block();
        let info = Self::unit_info(&mut f);
        if let Err(error) = self.emit_cycle_body(&mut f, &info) {
            self.push_error(error);
            return;
        }
        f.check()
            .map_err(CodegenError::internal)
            .unwrap_or_else(|error| self.push_error(error));
        self.module.add_function(f);
    }

    fn emit_cycle_body(&mut self, f: &mut Function, info: &BodyInfo) -> Result<(), CodegenError> {
        let instances = self.plan.instances.clone();
        let single_tasks: BTreeSet<(String, Symbol)> = self
            .plan
            .tasks
            .iter()
            .filter(|task| task.single == Some(true))
            .map(|task| (task.resource.clone(), task.name))
            .collect();
        let last_for_task: Vec<bool> = instances
            .iter()
            .enumerate()
            .map(|(index, instance)| {
                !instances
                    .iter()
                    .skip(index + 1)
                    .any(|other| other.resource == instance.resource && other.task == instance.task)
            })
            .collect();
        for (index, instance) in instances.iter().enumerate() {
            let guarded = instance
                .task
                .is_some_and(|task| single_tasks.contains(&(instance.resource.clone(), task)));
            if guarded {
                let task = instance.task.expect("guarded instance has a task");
                let guard = mangle::task_guard_symbol(&instance.resource, task, self.hir);
                let guard_value = Value::symbol(&guard);
                let loaded = f.load(IrType::Int(1), &guard_value, None);
                let run = f.next_block_label();
                let skip = f.next_block_label();
                f.cond_branch(&loaded, &run, &skip);
                f.start_block(&run);
                self.emit_instance_cycle(f, info, instance)?;
                if last_for_task[index] {
                    f.store(&Value::bool(false), &guard_value, None);
                }
                f.branch(&skip);
                f.start_block(&skip);
            } else {
                self.emit_instance_cycle(f, info, instance)?;
            }
        }
        f.ret(None);
        Ok(())
    }

    fn emit_instance_cycle(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        instance: &PlanInstance,
    ) -> Result<(), CodegenError> {
        // Configuration bindings lower program fields through `This(program)`
        // even though the cycle function has no `%this` parameter; bind them
        // to this instance's global (D-083).
        let mut info = info.clone();
        info.this_ptr = Some(Value::symbol(&instance.global));
        for binding in instance.bindings.iter().filter(|binding| !binding.output) {
            let value = self.lower_expr(f, &info, binding.source)?;
            self.write_place(f, &info, binding.target, value)?;
        }
        let body = mangle::program_body_symbol(self.hir, instance.program);
        f.call(IrType::Void, &body, &[Value::symbol(&instance.global)]);
        for binding in instance.bindings.iter().filter(|binding| binding.output) {
            let value = self.lower_expr(f, &info, binding.target)?;
            self.write_place(f, &info, binding.source, value)?;
        }
        Ok(())
    }

    fn emit_shutdown(&mut self) {
        let mut f = Function::new(symbols::SHUTDOWN, self.linkage(), IrType::Void, Vec::new());
        f.start_fresh_block();
        f.call(
            IrType::Int(32),
            symbols::RETAIN_SAVE,
            &[Value::symbol(symbols::RETAIN_BUFFER)],
        );
        f.call(
            IrType::Int(32),
            symbols::STATE_SAVE,
            &[
                Value::symbol(symbols::RETAIN_BUFFER),
                Value::int(64, i128::from(self.retain.size)),
                Value::int(64, i128::from(self.retain.hash as i64)),
            ],
        );
        f.ret(None);
        if let Err(error) = f.check() {
            self.push_error(CodegenError::internal(error));
            return;
        }
        self.module.add_function(f);
    }

    fn emit_retain_functions(&mut self) {
        if let Err(error) = self.emit_retain_functions_inner() {
            self.push_error(error);
        }
    }

    fn emit_retain_functions_inner(&mut self) -> Result<(), CodegenError> {
        let mut size_fn = Function::new(
            symbols::RETAIN_SIZE,
            SymbolAttrs::external(),
            IrType::Int(64),
            Vec::new(),
        );
        size_fn.start_fresh_block();
        size_fn.ret(Some(&Value::int(64, i128::from(self.retain.size))));
        size_fn.check().map_err(CodegenError::internal)?;
        self.module.add_function(size_fn);

        let mut hash_fn = Function::new(
            symbols::RETAIN_HASH,
            SymbolAttrs::external(),
            IrType::Int(64),
            Vec::new(),
        );
        hash_fn.start_fresh_block();
        hash_fn.ret(Some(&Value::int(64, i128::from(self.retain.hash as i64))));
        hash_fn.check().map_err(CodegenError::internal)?;
        self.module.add_function(hash_fn);

        let buffer_len = self.retain.size.max(1);
        let buffer_type = format!("[{buffer_len} x i8]");
        let leaves = self.retain.leaves.clone();
        for saving in [true, false] {
            let name = if saving {
                symbols::RETAIN_SAVE
            } else {
                symbols::RETAIN_LOAD
            };
            let mut f = Function::new(
                name,
                SymbolAttrs::external(),
                IrType::Int(32),
                vec![(IrType::Ptr, "%a0".into())],
            );
            f.start_fresh_block();
            let buffer = f.param_value(0);
            for leaf in &leaves {
                let source = match &leaf.place {
                    RetainPlace::Global(def) => {
                        let symbol = self.global_symbols.get(def).cloned().ok_or_else(|| {
                            CodegenError::internal("retained global has no symbol")
                        })?;
                        Value::symbol(&symbol)
                    }
                    RetainPlace::InstanceField { instance, field } => {
                        let plan_instance = self
                            .plan
                            .instances
                            .iter()
                            .find(|candidate| candidate.name == *instance)
                            .cloned()
                            .ok_or_else(|| {
                                CodegenError::internal("retained instance is missing")
                            })?;
                        self.instance_field_pointer(
                            &mut f,
                            &plan_instance,
                            *field,
                            Span::empty(stc_span::FileId::new(0), 0),
                        )?
                    }
                };
                let offset = f.gep(
                    &buffer_type,
                    &buffer,
                    &["i64 0".into(), format!("i64 {}", leaf.offset)],
                    true,
                );
                if saving {
                    let value = f.load(leaf.kind.storage(), &source, None);
                    f.store(&value, &offset, Some(1));
                } else {
                    let value = f.load(leaf.kind.storage(), &offset, Some(1));
                    f.store(&value, &source, None);
                }
            }
            f.ret(Some(&Value::int(32, 0)));
            f.check().map_err(CodegenError::internal)?;
            self.module.add_function(f);
        }
        Ok(())
    }

    fn emit_export_wrappers(&mut self) {
        let exports: Vec<(DefId, String, bool)> = self
            .hir
            .defs
            .iter()
            .enumerate()
            .filter_map(|(index, def)| {
                let name = def.export_name?;
                if def.test_only {
                    return None;
                }
                let symbol = self.hir.interner.text(name).to_string();
                Some((
                    DefId::new(u32::try_from(index).ok()?),
                    symbol,
                    def.no_mangle,
                ))
            })
            .collect();
        for (def, symbol, _no_mangle) in exports {
            if let Err(error) = self.emit_export_wrapper(def, &symbol) {
                self.push_error(error);
            }
        }
    }

    fn emit_export_wrapper(&mut self, def: DefId, symbol: &str) -> Result<(), CodegenError> {
        let DefKind::Function { result, params, .. } = self.hir.defs[def.index()].kind.clone()
        else {
            return Ok(());
        };
        let internal = self
            .function_symbols
            .get(&def)
            .cloned()
            .ok_or_else(|| CodegenError::internal("export has no internal function"))?;
        let mut param_data = Vec::new();
        for param in &params {
            let data = self.hir.param(*param).clone();
            let kind = layout::scalar_kind_at(self.hir, data.ty, data.span)?;
            param_data.push(kind);
        }
        let result_kind = result
            .map(|ty| layout::scalar_kind_at(self.hir, ty, self.hir.defs[def.index()].span))
            .transpose()?;
        let ret_ty = result_kind.map_or(IrType::Void, ScalarKind::storage);
        let fn_params = param_data
            .iter()
            .enumerate()
            .map(|(index, kind)| (kind.storage(), format!("%a{index}")))
            .collect();
        let mut f = Function::new(symbol, SymbolAttrs::external(), ret_ty, fn_params);
        f.start_fresh_block();
        let args = (0..param_data.len())
            .map(|index| f.param_value(index))
            .collect::<Vec<_>>();
        let value = f.call(ret_ty, &internal, &args);
        match (ret_ty, value) {
            (IrType::Void, _) => f.ret(None),
            (_, Some(value)) => f.ret(Some(&value)),
            (_, None) => {
                return Err(CodegenError::internal(
                    "export wrapper call did not return a value",
                ));
            }
        }
        f.check().map_err(CodegenError::internal)?;
        self.module.add_function(f);
        Ok(())
    }

    fn emit_main(&mut self) {
        let mut f = Function::new(
            "main",
            SymbolAttrs::external(),
            IrType::Int(32),
            vec![
                (IrType::Int(32), "%argc".into()),
                (IrType::Ptr, "%argv".into()),
            ],
        );
        f.start_fresh_block();
        let argc = f.param_value(0);
        let argv = f.param_value(1);
        let code = f.call(
            IrType::Int(32),
            symbols::EXECUTE,
            &[
                argc,
                argv,
                Value::symbol(symbols::INIT),
                Value::symbol(symbols::CYCLE),
                Value::symbol(symbols::SHUTDOWN),
            ],
        );
        match code {
            Some(code) => f.ret(Some(&code)),
            None => f.ret(Some(&Value::int(32, 0))),
        }
        if let Err(error) = f.check() {
            self.push_error(CodegenError::internal(error));
            return;
        }
        self.module.add_function(f);
    }

    fn add_metadata_symbol(&mut self, metadata: &MetadataModel) {
        let json = metadata.to_json().render();
        let bytes = json.as_bytes();
        let length = bytes.len() + 1;
        self.module.add_global(GlobalDef {
            name: symbols::METADATA_DATA.into(),
            attrs: SymbolAttrs::private(),
            ty: format!("[{length} x i8]"),
            init: c_string(bytes),
            constant: true,
            align: Some(1),
        });
        let mut f = Function::new(
            symbols::METADATA_JSON,
            SymbolAttrs::external(),
            IrType::Ptr,
            Vec::new(),
        );
        f.start_fresh_block();
        f.ret(Some(&Value::symbol(symbols::METADATA_DATA)));
        if let Err(error) = f.check() {
            self.push_error(CodegenError::internal(error));
            return;
        }
        self.module.add_function(f);
    }

    fn build_metadata(&self) -> Metadata {
        let programs = self
            .plan
            .instances
            .iter()
            .map(|instance| ProgramMetadata {
                name: instance.name_text.clone(),
                program: self.hir.def_name(instance.program).to_string(),
                resource: instance.resource.clone(),
                task: instance
                    .task
                    .map(|task| self.hir.interner.text(task).to_string()),
            })
            .collect();
        let tasks = self
            .plan
            .tasks
            .iter()
            .map(|task| TaskMetadata {
                resource: task.resource.clone(),
                name: task.name_text.clone(),
                single: task.single,
                interval_ns: task.interval_ns,
                priority: task.priority,
            })
            .collect();
        let bindings = self
            .image
            .bindings
            .iter()
            .map(|binding| IoBindingMetadata {
                variable: binding.variable.clone(),
                area: binding.area.as_char(),
                address: binding.address.clone(),
                offset_bits: binding.start_bit,
                ty: binding.kind.canonical_name(),
                hint: binding.hint.as_ref().map(|hint| HintMetadata {
                    device: hint.device.clone(),
                    channel: hint.channel,
                    direction: hint.direction.map(|direction| direction.as_str()),
                }),
                source: binding.source,
            })
            .collect();
        let leaves = self
            .retain
            .leaves
            .iter()
            .map(|leaf| MetaRetainLeaf {
                path: leaf.path.clone(),
                ty: leaf.ty.clone(),
                offset: leaf.offset,
                size: leaf.size,
            })
            .collect();
        let exports = self
            .hir
            .defs
            .iter()
            .enumerate()
            .filter_map(|(index, def)| {
                let name = def.export_name?;
                if def.test_only {
                    return None;
                }
                let id = DefId::new(u32::try_from(index).ok()?);
                Some(ExportMetadata {
                    function: self.function_symbols.get(&id).cloned().unwrap_or_default(),
                    symbol: self.hir.interner.text(name).to_string(),
                    no_mangle: def.no_mangle,
                })
            })
            .collect();
        let span_entries = self
            .span_entries
            .iter()
            .map(|entry| SpanMetadataEntry {
                id: entry.id,
                file: entry.file.raw(),
                start: entry.start,
                end: entry.end,
            })
            .collect();
        Metadata {
            profile: "stc-2013-oop-utf8-v1",
            compiler: format!("stc {}", env!("CARGO_PKG_VERSION")),
            abi_version: i64::from(abi::ABI_VERSION),
            artifact: self.options.artifact_name.to_string(),
            mode: self.options.mode.as_str(),
            configuration: self.plan.configuration.clone(),
            programs,
            tasks,
            io: crate::metadata::IoMetadata {
                inputs_bytes: self.image.inputs_bytes,
                outputs_bytes: self.image.outputs_bytes,
                markers_bytes: self.image.markers_bytes,
                input_defaults_hex: hex(&self.image.input_defaults),
                output_defaults_hex: hex(&self.image.output_defaults),
                bindings,
            },
            retain: crate::metadata::RetainMetadata {
                size: self.retain.size,
                layout_hash: self.retain.hash,
                leaves,
            },
            exports,
            spans: SpanMetadata {
                files: self.options.sources.to_vec(),
                entries: span_entries,
            },
        }
    }
}

fn synthetic(hir: &HirCrate, def: DefId) -> bool {
    let span = hir.defs[def.index()].span;
    span.start == 0 && span.end == 0
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn build_span_table(hir: &HirCrate) -> (BTreeMap<Span, u32>, Vec<SpanEntry>) {
    let mut spans = BTreeSet::new();
    let mut add = |span: Span| {
        if span.start != span.end {
            spans.insert(span);
        }
    };
    for def in &hir.defs {
        add(def.span);
    }
    for field in &hir.fields {
        add(field.span);
    }
    for param in &hir.params {
        add(param.span);
    }
    for local in &hir.locals {
        add(local.span);
    }
    for body in &hir.bodies {
        add(body.span);
    }
    for expr in &hir.exprs {
        add(expr.span);
    }
    for stmt in &hir.stmts {
        add(stmt.span);
        if let stc_hir::StmtKindHir::Case { arms, .. } = &stmt.kind {
            for arm in arms {
                for label in &arm.labels {
                    add(label.span);
                }
            }
        }
    }
    for def in &hir.defs {
        let stc_hir::DefKind::Configuration { config } = &def.kind else {
            continue;
        };
        add(config.span);
        for resource in &config.resources {
            for task in &resource.tasks {
                add(task.span);
            }
            for program in &resource.programs {
                add(program.span);
                for binding in &program.bindings {
                    add(binding.span);
                }
            }
        }
        for init in &config.config_inits {
            add(init.span);
        }
    }
    drop(add);
    let mut ids = BTreeMap::new();
    let mut entries = Vec::new();
    for (index, span) in spans.into_iter().enumerate() {
        let id = u32::try_from(index).unwrap_or(u32::MAX);
        ids.insert(span, id);
        entries.push(SpanEntry {
            id,
            file: span.file,
            start: span.start,
            end: span.end,
        });
    }
    (ids, entries)
}
