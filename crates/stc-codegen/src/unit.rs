//! Module units: functions, programs, roots, init/prepare/cycle/shutdown,
//! exports, retained state, and metadata assembly.
//!
//! Generated code targets the ABI 2 module model. Persistent scalars, program
//! fields, task guards, and direct `AT` variables are reached through GOT cell
//! values; user calls are indirect through function cells; host services are
//! called through import cells. The module never defines persistent storage.

use std::collections::{BTreeMap, BTreeSet};

use stc_got::Target;
use stc_got::catalog::ImportService;
use stc_hir::{ArgDirection, DefId, DefKind, ExprId, HirCrate, Symbol, TyId, VarSectionKind};
use stc_span::Span;

use crate::CodegenError;
use crate::abi::{self, symbols};
use crate::codes;
use crate::config::{
    self, ImageLayout, Plan, PlanInstance, ProgramLayout, RetainLayout, RetainPlace,
};
use crate::expr::{BodyInfo, FieldAccess};
use crate::ir::{Function, GlobalDef, IrType, Module, SymbolAttrs, Value, c_string};
use crate::layout::{self, ScalarKind};
use crate::mangle;
use crate::metadata::{
    ExportMetadata, HintMetadata, IoBindingMetadata, ProgramMetadata,
    RetainLeafMetadata as MetaRetainLeaf, SpanMetadata, SpanMetadataEntry, TaskMetadata,
};
use crate::module::{FunctionTarget, ModuleArtifacts, RootTarget};
use crate::stmt::LoopTarget;
use crate::verify::{self, Contract, DescriptorSpec, FunctionTableSpec};
use crate::{Metadata, Options, Output, SpanEntry};

/// The code generation state for one build.
pub(crate) struct Lowerer<'a> {
    pub hir: &'a HirCrate,
    pub options: &'a Options<'a>,
    pub module: Module,
    pub target: Target,
    pub got: ModuleArtifacts,
    pub layouts: BTreeMap<DefId, ProgramLayout>,
    pub plan: Plan,
    pub image: ImageLayout,
    pub retain: RetainLayout,
    pub function_symbols: BTreeMap<DefId, String>,
    pub global_symbols: BTreeMap<DefId, String>,
    pub instance_globals: BTreeMap<Symbol, String>,
    pub task_guard_symbols: BTreeMap<(String, Symbol), String>,
    /// Slot-table value for the function currently being lowered. Compiler
    /// helpers receive their own slot parameter.
    pub current_slots: Option<Value>,
    pub span_ids: BTreeMap<Span, u32>,
    pub span_entries: Vec<SpanEntry>,
    pub errors: Vec<CodegenError>,
    seen_errors: BTreeSet<String>,
}

/// Generates IR and metadata for a checked HIR crate.
pub(crate) fn generate(
    hir: &HirCrate,
    options: &Options<'_>,
    target: &Target,
    got: ModuleArtifacts,
) -> Result<Output, Vec<CodegenError>> {
    let plan = config::select_plan(hir)?;
    let layouts = config::program_layouts(hir, &plan)?;
    let image = config::image_layout(hir, &plan, &layouts)?;
    let retain = config::retain_layout(hir, &plan, &layouts)?;
    let (span_ids, span_entries) = build_span_table(hir);
    let mut lowerer = Lowerer {
        hir,
        options,
        module: Module::new(options.artifact_name, options.target),
        target: target.clone(),
        got,
        layouts,
        plan,
        image,
        retain,
        function_symbols: BTreeMap::new(),
        global_symbols: BTreeMap::new(),
        instance_globals: BTreeMap::new(),
        task_guard_symbols: BTreeMap::new(),
        current_slots: None,
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
    if let Err(message) = lowerer.module.check() {
        return Err(vec![CodegenError::internal(message)]);
    }
    if let Err(errors) = verify::verify(&lowerer.module, &lowerer.contract()) {
        return Err(errors
            .into_iter()
            .map(|error| {
                CodegenError::internal(format!(
                    "module verification failed [{}]: {}",
                    error.rule, error.message
                ))
            })
            .collect());
    }
    let rendered = lowerer.module.render();
    Ok(Output {
        module: rendered,
        metadata,
        spans: lowerer.span_entries.clone(),
        got: lowerer.got.clone(),
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
                                    codes::CODEGEN_UNSUPPORTED,
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
                DefKind::Global { .. } => {
                    // Direct globals have no module storage, but their symbols
                    // stay in the verification contract so a regression that
                    // emits one is rejected.
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
        for task in &self.plan.tasks {
            let resource = if task.resource.is_empty() {
                crate::module::SYNTHETIC_NAME.to_owned()
            } else {
                task.resource.to_ascii_uppercase()
            };
            let name = task.name_text.to_ascii_uppercase();
            self.task_guard_symbols.insert(
                (task.resource.clone(), task.name),
                mangle::task_guard_symbol_text(&resource, &name),
            );
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
                            codes::CODEGEN_UNSUPPORTED,
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
                                codes::CODEGEN_UNSUPPORTED,
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
                                codes::CODEGEN_UNSUPPORTED,
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
                    codes::CODEGEN_UNSUPPORTED,
                    "STRUCT declarations are M5".into(),
                    def.span,
                )),
                DefKind::Enum { .. } => self.push_error(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "ENUM declarations are M5".into(),
                    def.span,
                )),
                DefKind::NamedValue { .. } => self.push_error(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "named-value type declarations are M5".into(),
                    def.span,
                )),
                DefKind::Subrange { .. } => self.push_error(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "subrange type declarations are M5".into(),
                    def.span,
                )),
                DefKind::Class { .. } => self.push_error(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "CLASS declarations are M5".into(),
                    def.span,
                )),
                DefKind::FunctionBlock { .. } => self.push_error(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "FUNCTION_BLOCK declarations are M5".into(),
                    def.span,
                )),
                DefKind::Interface { .. } => self.push_error(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
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
        self.emit_module_data();
        self.emit_user_functions();
        self.emit_program_bodies();
        self.emit_prepare();
        self.emit_cycle();
        self.emit_shutdown();
        self.emit_retain_functions();
        self.emit_export_wrappers();
        self.emit_module_accessors();
        self.emit_attach();
        self.emit_detach();
        self.add_metadata_symbol();
    }

    // ---- module data ------------------------------------------------------

    fn emit_module_data(&mut self) {
        for name in [abi::module::API_GLOBAL, abi::module::SLOTS_GLOBAL] {
            self.module.add_global(GlobalDef {
                name: name.into(),
                attrs: SymbolAttrs::internal(),
                ty: "ptr".into(),
                init: "null".into(),
                constant: false,
                align: None,
            });
        }
        let buffer_len = self.retain.size.max(1);
        self.module.add_global(GlobalDef {
            name: symbols::RETAIN_BUFFER.into(),
            attrs: SymbolAttrs::internal(),
            ty: format!("[{buffer_len} x i8]"),
            init: "zeroinitializer".into(),
            constant: false,
            align: None,
        });
        self.module
            .add_descriptor(abi::module::DESCRIPTOR_DATA, &self.got.descriptor_bytes);
        let functions = self.got.function_table();
        if functions.len() != self.got.descriptor.functions.len() {
            self.push_error(CodegenError::internal(format!(
                "descriptor lists {} functions but the slot table binds {}",
                self.got.descriptor.functions.len(),
                functions.len()
            )));
            return;
        }
        let symbols: Vec<&str> = functions
            .iter()
            .map(|(_, symbol)| symbol.as_str())
            .collect();
        self.module
            .add_function_table(abi::module::FUNCTION_TABLE, &symbols);
    }

    fn slots_global(&mut self, f: &mut Function) -> Value {
        f.load(
            IrType::Ptr,
            &Value::symbol(abi::module::SLOTS_GLOBAL),
            Some(8),
        )
    }

    // ---- functions --------------------------------------------------------

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
        let mut fn_params = param_data
            .iter()
            .enumerate()
            .map(|(index, (_, kind))| (kind.storage(), format!("%a{index}")))
            .collect::<Vec<_>>();
        fn_params.push((IrType::Ptr, "%slots".into()));
        let mut f = Function::new(symbol, SymbolAttrs::internal(), ret_ty, fn_params);
        f.start_fresh_block();
        let slots = f.param_value(param_data.len());
        let mut info = BodyInfo {
            params: BTreeMap::new(),
            locals: BTreeMap::new(),
            result_ptr: None,
            result_ty: None,
            slots: slots.clone(),
            fields: None,
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
        let previous = self.current_slots.replace(slots);
        let result = (|| -> Result<(), CodegenError> {
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
        })();
        self.current_slots = previous;
        result
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
        let DefKind::Program { body, fields, .. } = self.hir.defs[program.index()].kind.clone()
        else {
            return Ok(());
        };
        let symbol = mangle::program_body_symbol(self.hir, program);
        let mut f = Function::new(
            symbol,
            SymbolAttrs::internal(),
            IrType::Void,
            vec![
                (IrType::Ptr, "%fields".into()),
                (IrType::Ptr, "%slots".into()),
            ],
        );
        f.start_fresh_block();
        let mut ordinals = BTreeMap::new();
        let mut direct = BTreeSet::new();
        for (index, field) in fields.iter().enumerate() {
            ordinals.insert(*field, u32::try_from(index).unwrap_or(u32::MAX));
            if self.hir.field(*field).at.is_some() {
                direct.insert(*field);
            }
        }
        let slots = f.param_value(1);
        let mut info = BodyInfo {
            params: BTreeMap::new(),
            locals: BTreeMap::new(),
            result_ptr: None,
            result_ty: None,
            slots: slots.clone(),
            fields: Some(FieldAccess {
                pointer: f.param_value(0),
                ordinals,
                direct,
            }),
            return_label: f.next_block_label(),
        };
        let previous = self.current_slots.replace(slots);
        let result = (|| -> Result<(), CodegenError> {
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
        })();
        self.current_slots = previous;
        result
    }

    // ---- prepare ----------------------------------------------------------

    fn unit_info(f: &mut Function, slots: Value) -> BodyInfo {
        BodyInfo {
            params: BTreeMap::new(),
            locals: BTreeMap::new(),
            result_ptr: None,
            result_ty: None,
            slots,
            fields: None,
            return_label: f.next_block_label(),
        }
    }

    fn emit_prepare(&mut self) {
        let mut f = Function::new(
            abi::module::PREPARE,
            SymbolAttrs::module_export(),
            IrType::Int(32),
            vec![
                (IrType::Int(32), "%mode".into()),
                (IrType::Int(64), "%generation".into()),
                (IrType::Ptr, "%mask".into()),
                (IrType::Int(32), "%mask_size".into()),
            ],
        );
        f.start_fresh_block();
        let slots = self.slots_global(&mut f);
        let mut info = Self::unit_info(&mut f, slots.clone());
        let previous = self.current_slots.replace(slots);
        let result = self.emit_prepare_body(&mut f, &mut info);
        self.current_slots = previous;
        match result {
            Ok(()) => {
                if let Err(error) = f.check() {
                    self.push_error(CodegenError::internal(error));
                } else {
                    self.module.add_function(f);
                }
            }
            Err(error) => self.push_error(error),
        }
    }

    fn emit_prepare_body(
        &mut self,
        f: &mut Function,
        info: &mut BodyInfo,
    ) -> Result<(), CodegenError> {
        let slots = info.slots.clone();
        let mode = f.param_value(0);
        let generation = f.param_value(1);
        let mask = f.param_value(2);
        let mask_size = f.param_value(3);
        let zero_generation = f.icmp("eq", &generation, &Value::int(64, 0));
        Self::return_if(f, &zero_generation, 1);
        let slots_null = f.icmp("eq", &slots, &Value::null_ptr());
        Self::return_if(f, &slots_null, 1);
        let bad_mode = f.icmp("ugt", &mode, &Value::int(32, 1));
        Self::return_if(f, &bad_mode, 1);
        let expected_size = i32::try_from(self.got.slots.len().div_ceil(8)).unwrap_or(i32::MAX);
        let online = f.icmp("eq", &mode, &Value::int(32, 1));
        let wrong_size = f.icmp("ne", &mask_size, &Value::int(32, i128::from(expected_size)));
        let bad_mask = f.binop("and", &online, &wrong_size);
        Self::return_if(f, &bad_mask, 1);
        let cold = f.icmp("eq", &mode, &Value::int(32, 0));

        self.emit_global_initializers(f, info, &cold, &mask)?;
        self.emit_instance_initializers(f, info, &cold, &mask)?;
        self.emit_task_guard_initializers(f, info, &cold, &mask)?;
        self.emit_state_load(f, info, &cold)?;
        f.ret(Some(&Value::int(32, 0)));
        Ok(())
    }

    fn return_if(f: &mut Function, condition: &Value, code: i32) {
        let fail = f.next_block_label();
        let cont = f.next_block_label();
        f.cond_branch(condition, &fail, &cont);
        f.start_block(&fail);
        f.ret(Some(&Value::int(32, i128::from(code))));
        f.start_block(&cont);
    }

    /// Opens a conditional initialization block.
    ///
    /// For a data root the block runs when the build is cold or the fresh-root
    /// mask names the root's slot. Direct roots run only on a cold build. The
    /// returned label is the block the caller must terminate with
    /// [`Self::close_init`].
    fn begin_init(
        f: &mut Function,
        cold: &Value,
        mask: &Value,
        slot: Option<u32>,
    ) -> Result<(String, String), CodegenError> {
        let run = f.next_block_label();
        let next = f.next_block_label();
        match slot {
            Some(slot) => {
                let check = f.next_block_label();
                let cold_predecessor = f.current_label().to_string();
                f.cond_branch(cold, &run, &check);
                f.start_block(&check);
                let byte = f.gep("i8", mask, &[format!("i64 {}", u64::from(slot) / 8)], false);
                let raw = f.load(IrType::Int(8), &byte, Some(1));
                let shifted = Value::int(8, 1_i128 << (slot % 8));
                let masked = f.binop("and", &raw, &shifted);
                let set = f.icmp("ne", &masked, &Value::int(8, 0));
                let check_predecessor = f.current_label().to_string();
                f.branch(&run);
                f.start_block(&run);
                let should = f.phi(
                    IrType::Int(1),
                    &[
                        (Value::bool(true), cold_predecessor),
                        (set, check_predecessor),
                    ],
                );
                let do_init = f.next_block_label();
                f.cond_branch(&should, &do_init, &next);
                f.start_block(&do_init);
            }
            None => {
                f.cond_branch(cold, &run, &next);
                f.start_block(&run);
            }
        }
        Ok((run, next))
    }

    fn close_init(f: &mut Function, next: &str) {
        f.branch(next);
        f.start_block(next);
    }

    fn emit_global_initializers(
        &mut self,
        f: &mut Function,
        info: &mut BodyInfo,
        cold: &Value,
        mask: &Value,
    ) -> Result<(), CodegenError> {
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
                let id = DefId::new(u32::try_from(index).ok()?);
                if synthetic(self.hir, id) || def.test_only {
                    return None;
                }
                let DefKind::Global { ty, init, at, .. } = &def.kind else {
                    return None;
                };
                Some((id, *ty, *init, at.clone(), def.span))
            })
            .collect();
        for (def, ty, init, at, span) in globals {
            let Some(init) = init else {
                continue;
            };
            let kind = layout::scalar_kind_at(self.hir, ty, span)?;
            let target = RootTarget::Global(def);
            let slot = self.root_slot(&target)?;
            if let Some(address) = at {
                let (_, next) = Self::begin_init(f, cold, mask, None)?;
                let value = self.lower_expr(f, info, init)?;
                self.write_direct_initial(f, info, slot, &address, kind, value)?;
                Self::close_init(f, &next);
            } else {
                let (_, next) = Self::begin_init(f, cold, mask, Some(slot))?;
                let value = self.lower_expr(f, info, init)?;
                let pointer = Self::slot_storage(f, info, slot);
                Self::write_scalar(f, &pointer, kind, &value);
                Self::close_init(f, &next);
            }
        }
        Ok(())
    }

    fn emit_instance_initializers(
        &mut self,
        f: &mut Function,
        info: &mut BodyInfo,
        cold: &Value,
        mask: &Value,
    ) -> Result<(), CodegenError> {
        let overrides = config_overrides(&self.plan);
        let instances = self.plan.instances.clone();
        for instance in &instances {
            let DefKind::Program { fields, .. } = &self.hir.defs[instance.program.index()].kind
            else {
                continue;
            };
            let fields = fields.clone();
            for field in fields {
                let data = self.hir.field(field).clone();
                let init = overrides
                    .get(&(instance.name, field))
                    .copied()
                    .or(data.init);
                let Some(init) = init else {
                    continue;
                };
                let kind = layout::scalar_kind_at(self.hir, data.ty, data.span)?;
                let target = RootTarget::InstanceField {
                    instance: instance.name,
                    field,
                };
                let slot = self.root_slot(&target)?;
                if let Some(address) = data.at {
                    let (_, next) = Self::begin_init(f, cold, mask, None)?;
                    let value = self.lower_expr(f, info, init)?;
                    self.write_direct_initial(f, info, slot, &address, kind, value)?;
                    Self::close_init(f, &next);
                } else {
                    let (_, next) = Self::begin_init(f, cold, mask, Some(slot))?;
                    let value = self.lower_expr(f, info, init)?;
                    let pointer = Self::slot_storage(f, info, slot);
                    Self::write_scalar(f, &pointer, kind, &value);
                    Self::close_init(f, &next);
                }
            }
        }
        Ok(())
    }

    fn emit_task_guard_initializers(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        cold: &Value,
        mask: &Value,
    ) -> Result<(), CodegenError> {
        let tasks = self.plan.tasks.clone();
        for task in &tasks {
            if task.single != Some(true) {
                continue;
            }
            let slot = self.task_guard_slot(&task.resource, task.name)?;
            let (_, next) = Self::begin_init(f, cold, mask, Some(slot))?;
            let pointer = Self::slot_storage(f, info, slot);
            Self::write_scalar(f, &pointer, ScalarKind::Bool, &Value::bool(true));
            Self::close_init(f, &next);
        }
        Ok(())
    }

    fn emit_state_load(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        cold: &Value,
    ) -> Result<(), CodegenError> {
        let load = f.next_block_label();
        let skip = f.next_block_label();
        f.cond_branch(cold, &load, &skip);
        f.start_block(&load);
        let slot = self.import_slot(ImportService::StateLoad)?;
        let service = Self::slot_storage(f, info, slot);
        let rc = f
            .call_ptr(
                IrType::Int(32),
                &service,
                &[
                    Value::symbol(symbols::RETAIN_BUFFER),
                    Value::int(64, i128::from(self.retain.size)),
                    Value::int(64, i128::from(self.retain.hash as i64)),
                ],
            )
            .ok_or_else(|| CodegenError::internal("state_load returned void"))?;
        let ok = f.icmp("eq", &rc, &Value::int(32, 0));
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
        f.branch(&skip);
        f.start_block(&skip);
        Ok(())
    }

    // ---- cycle ------------------------------------------------------------

    fn emit_cycle(&mut self) {
        let mut f = Function::new(
            abi::module::CYCLE,
            SymbolAttrs::module_export(),
            IrType::Int(32),
            vec![(IrType::Int(64), "%generation".into())],
        );
        f.start_fresh_block();
        let slots = self.slots_global(&mut f);
        let info = Self::unit_info(&mut f, slots.clone());
        let previous = self.current_slots.replace(slots);
        let result = self.emit_cycle_entry_body(&mut f, &info);
        self.current_slots = previous;
        match result {
            Ok(()) => {
                if let Err(error) = f.check() {
                    self.push_error(CodegenError::internal(error));
                } else {
                    self.module.add_function(f);
                }
            }
            Err(error) => self.push_error(error),
        }
    }

    fn emit_cycle_entry_body(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
    ) -> Result<(), CodegenError> {
        let generation = f.param_value(0);
        let zero_generation = f.icmp("eq", &generation, &Value::int(64, 0));
        Self::return_if(f, &zero_generation, 1);
        let slots_null = f.icmp("eq", &info.slots, &Value::null_ptr());
        Self::return_if(f, &slots_null, 1);
        self.emit_cycle_body(f, info)?;
        f.ret(Some(&Value::int(32, 0)));
        Ok(())
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
                let slot = self.task_guard_slot(&instance.resource, task)?;
                let guard_value = Self::slot_storage(f, info, slot);
                let guard = Self::read_scalar(f, &guard_value, ScalarKind::Bool)?;
                let run = f.next_block_label();
                let skip = f.next_block_label();
                f.cond_branch(&guard, &run, &skip);
                f.start_block(&run);
                self.emit_instance_cycle(f, info, instance)?;
                if last_for_task[index] {
                    Self::write_scalar(f, &guard_value, ScalarKind::Bool, &Value::bool(false));
                }
                f.branch(&skip);
                f.start_block(&skip);
            } else {
                self.emit_instance_cycle(f, info, instance)?;
            }
        }
        Ok(())
    }

    fn emit_instance_cycle(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        instance: &PlanInstance,
    ) -> Result<(), CodegenError> {
        let DefKind::Program { fields, .. } = &self.hir.defs[instance.program.index()].kind else {
            return Ok(());
        };
        let fields = fields.clone();
        let mut ordinals = BTreeMap::new();
        let mut direct = BTreeSet::new();
        let mut pointers = Vec::new();
        for (index, field) in fields.iter().enumerate() {
            ordinals.insert(*field, u32::try_from(index).unwrap_or(u32::MAX));
            if self.hir.field(*field).at.is_some() {
                direct.insert(*field);
            }
            let target = RootTarget::InstanceField {
                instance: instance.name,
                field: *field,
            };
            let slot = self.root_slot(&target)?;
            pointers.push(Self::slot_storage(f, info, slot));
        }
        let array_type = format!("[{} x ptr]", pointers.len());
        let field_pointer = if pointers.is_empty() {
            Value::null_ptr()
        } else {
            let array = f.alloca_typed(&array_type);
            for (index, pointer) in pointers.iter().enumerate() {
                let entry = f.gep(
                    &array_type,
                    &array,
                    &["i64 0".into(), format!("i64 {index}")],
                    true,
                );
                f.store(pointer, &entry, Some(8));
            }
            array
        };
        let mut local = info.clone();
        local.fields = Some(FieldAccess {
            pointer: field_pointer.clone(),
            ordinals,
            direct,
        });
        for binding in instance.bindings.iter().filter(|binding| !binding.output) {
            let value = self.lower_expr(f, &local, binding.source)?;
            self.write_place(f, &local, binding.target, value)?;
        }
        let slot = self.function_slot(FunctionTarget::ProgramBody(instance.program))?;
        let body = Self::slot_storage(f, &local, slot);
        f.call_ptr(IrType::Void, &body, &[field_pointer, local.slots.clone()]);
        for binding in instance.bindings.iter().filter(|binding| binding.output) {
            let value = self.lower_expr(f, &local, binding.target)?;
            self.write_place(f, &local, binding.source, value)?;
        }
        Ok(())
    }

    // ---- shutdown ---------------------------------------------------------

    fn emit_shutdown(&mut self) {
        let mut f = Function::new(
            abi::module::SHUTDOWN,
            SymbolAttrs::module_export(),
            IrType::Int(32),
            vec![(IrType::Int(32), "%mode".into())],
        );
        f.start_fresh_block();
        let slots = self.slots_global(&mut f);
        let info = Self::unit_info(&mut f, slots.clone());
        let previous = self.current_slots.replace(slots);
        let result = self.emit_shutdown_body(&mut f, &info);
        self.current_slots = previous;
        match result {
            Ok(()) => {
                if let Err(error) = f.check() {
                    self.push_error(CodegenError::internal(error));
                } else {
                    self.module.add_function(f);
                }
            }
            Err(error) => self.push_error(error),
        }
    }

    fn emit_shutdown_body(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
    ) -> Result<(), CodegenError> {
        let mode = f.param_value(0);
        let bad_mode = f.icmp("ugt", &mode, &Value::int(32, 1));
        Self::return_if(f, &bad_mode, 1);
        let slots_null = f.icmp("eq", &info.slots, &Value::null_ptr());
        Self::return_if(f, &slots_null, 1);
        f.call(
            IrType::Int(32),
            symbols::RETAIN_SAVE,
            &[Value::symbol(symbols::RETAIN_BUFFER)],
        );
        let slot = self.import_slot(ImportService::StateSave)?;
        let service = Self::slot_storage(f, info, slot);
        f.call_ptr(
            IrType::Int(32),
            &service,
            &[
                Value::symbol(symbols::RETAIN_BUFFER),
                Value::int(64, i128::from(self.retain.size)),
                Value::int(64, i128::from(self.retain.hash as i64)),
            ],
        );
        f.ret(Some(&Value::int(32, 0)));
        Ok(())
    }

    // ---- retained state ---------------------------------------------------

    fn emit_retain_functions(&mut self) {
        if let Err(error) = self.emit_retain_functions_inner() {
            self.push_error(error);
        }
    }

    fn emit_retain_functions_inner(&mut self) -> Result<(), CodegenError> {
        let mut size_fn = Function::new(
            symbols::RETAIN_SIZE,
            SymbolAttrs::module_export(),
            IrType::Int(64),
            Vec::new(),
        );
        size_fn.start_fresh_block();
        size_fn.ret(Some(&Value::int(64, i128::from(self.retain.size))));
        size_fn.check().map_err(CodegenError::internal)?;
        self.module.add_function(size_fn);

        let mut hash_fn = Function::new(
            symbols::RETAIN_HASH,
            SymbolAttrs::module_export(),
            IrType::Int(64),
            Vec::new(),
        );
        hash_fn.start_fresh_block();
        hash_fn.ret(Some(&Value::int(64, i128::from(self.retain.hash as i64))));
        hash_fn.check().map_err(CodegenError::internal)?;
        self.module.add_function(hash_fn);

        let buffer_len = self.retain.size.max(1);
        let buffer_type = format!("[{buffer_len} x i8]");
        for saving in [true, false] {
            let name = if saving {
                symbols::RETAIN_SAVE
            } else {
                symbols::RETAIN_LOAD
            };
            let mut f = Function::new(
                name,
                SymbolAttrs::module_export(),
                IrType::Int(32),
                vec![(IrType::Ptr, "%a0".into())],
            );
            f.start_fresh_block();
            let slots = self.slots_global(&mut f);
            let info = Self::unit_info(&mut f, slots);
            let previous = self.current_slots.replace(info.slots.clone());
            let result = self.emit_retain_mover(&mut f, &info, &buffer_type, saving);
            self.current_slots = previous;
            result?;
            f.check().map_err(CodegenError::internal)?;
            self.module.add_function(f);
        }
        Ok(())
    }

    fn emit_retain_mover(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        buffer_type: &str,
        saving: bool,
    ) -> Result<(), CodegenError> {
        let buffer = f.param_value(0);
        let slots_null = f.icmp("eq", &info.slots, &Value::null_ptr());
        let invalid = f.next_block_label();
        let valid = f.next_block_label();
        f.cond_branch(&slots_null, &invalid, &valid);
        f.start_block(&invalid);
        f.ret(Some(&Value::int(32, 1)));
        f.start_block(&valid);
        let leaves = self.retain.leaves.clone();
        for leaf in &leaves {
            let slot = match &leaf.place {
                RetainPlace::Global(def) => self.root_slot(&RootTarget::Global(*def))?,
                RetainPlace::InstanceField { instance, field } => {
                    self.root_slot(&RootTarget::InstanceField {
                        instance: *instance,
                        field: *field,
                    })?
                }
            };
            let source = Self::slot_storage(f, info, slot);
            let offset = f.gep(
                buffer_type,
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
        Ok(())
    }

    // ---- exports ----------------------------------------------------------

    fn emit_export_wrappers(&mut self) {
        let exports: Vec<(DefId, String)> = self
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
                Some((DefId::new(u32::try_from(index).ok()?), symbol))
            })
            .collect();
        for (def, symbol) in exports {
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
        let mut f = Function::new(symbol, SymbolAttrs::module_export(), ret_ty, fn_params);
        f.start_fresh_block();
        let slots = self.slots_global(&mut f);
        let info = Self::unit_info(&mut f, slots.clone());
        let previous = self.current_slots.replace(slots);
        let result = self.emit_export_body(&mut f, &info, def, ret_ty, param_data.len());
        self.current_slots = previous;
        match result {
            Ok(()) => {
                if let Err(error) = f.check() {
                    self.push_error(CodegenError::internal(error));
                } else {
                    self.module.add_function(f);
                }
            }
            Err(error) => self.push_error(error),
        }
        Ok(())
    }

    fn emit_export_body(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        def: DefId,
        ret_ty: IrType,
        argc: usize,
    ) -> Result<(), CodegenError> {
        let slots_null = f.icmp("eq", &info.slots, &Value::null_ptr());
        let call = f.next_block_label();
        let missing = f.next_block_label();
        f.cond_branch(&slots_null, &missing, &call);
        f.start_block(&missing);
        match ret_ty {
            IrType::Void => f.ret(None),
            _ => f.ret(Some(&Value::zero(ret_ty))),
        }
        f.start_block(&call);
        let slot = self.function_slot(FunctionTarget::Function(def))?;
        let callee = Self::slot_storage(f, info, slot);
        let mut args = (0..argc)
            .map(|index| f.param_value(index))
            .collect::<Vec<_>>();
        args.push(info.slots.clone());
        let value = f.call_ptr(ret_ty, &callee, &args);
        match (ret_ty, value) {
            (IrType::Void, _) => f.ret(None),
            (_, Some(value)) => f.ret(Some(&value)),
            (_, None) => {
                return Err(CodegenError::internal(
                    "export wrapper call did not return a value",
                ));
            }
        }
        Ok(())
    }

    // ---- module entry points ---------------------------------------------

    fn emit_module_accessors(&mut self) {
        let descriptor_len =
            i128::from(u64::try_from(self.got.descriptor_bytes.len()).unwrap_or(u64::MAX));
        let mut descriptor = Function::new(
            abi::module::DESCRIPTOR,
            SymbolAttrs::module_export(),
            IrType::Ptr,
            vec![(IrType::Ptr, "%out_size".into())],
        );
        descriptor.start_fresh_block();
        let out_size = descriptor.param_value(0);
        descriptor.store(&Value::int(64, descriptor_len), &out_size, Some(8));
        descriptor.ret(Some(&Value::symbol(abi::module::DESCRIPTOR_DATA)));
        if let Err(error) = descriptor.check() {
            self.push_error(CodegenError::internal(error));
        } else {
            self.module.add_function(descriptor);
        }

        let function_count =
            i128::from(u32::try_from(self.got.descriptor.functions.len()).unwrap_or(u32::MAX));
        let mut functions = Function::new(
            abi::module::FUNCTIONS,
            SymbolAttrs::module_export(),
            IrType::Ptr,
            vec![(IrType::Ptr, "%out_count".into())],
        );
        functions.start_fresh_block();
        let out_count = functions.param_value(0);
        functions.store(&Value::int(32, function_count), &out_count, Some(4));
        functions.ret(Some(&Value::symbol(abi::module::FUNCTION_TABLE)));
        if let Err(error) = functions.check() {
            self.push_error(CodegenError::internal(error));
        } else {
            self.module.add_function(functions);
        }
    }

    fn emit_attach(&mut self) {
        let mut f = Function::new(
            abi::module::ATTACH,
            SymbolAttrs::module_export(),
            IrType::Int(32),
            vec![
                (IrType::Ptr, "%api".into()),
                (IrType::Ptr, "%slots".into()),
                (IrType::Int(32), "%slot_count".into()),
                (IrType::Int(64), "%generation".into()),
            ],
        );
        f.start_fresh_block();
        let api = f.param_value(0);
        let slots = f.param_value(1);
        let slot_count = f.param_value(2);
        let generation = f.param_value(3);
        let zero_generation = f.icmp("eq", &generation, &Value::int(64, 0));
        Self::return_if(&mut f, &zero_generation, 1);
        let null_api = f.icmp("eq", &api, &Value::null_ptr());
        let null_slots = f.icmp("eq", &slots, &Value::null_ptr());
        let invalid = f.binop("or", &null_api, &null_slots);
        Self::return_if(&mut f, &invalid, 1);

        let version_pointer = f.gep_bytes(&api, 0);
        let version = f.load(IrType::Int(32), &version_pointer, Some(4));
        let major = f.binop("lshr", &version, &Value::int(32, 16));
        let required_major = Value::int(32, i128::from(abi::host_api::ABI_VERSION >> 16));
        let bad_major = f.icmp("ne", &major, &required_major);
        Self::return_if(&mut f, &bad_major, 2);
        let size_pointer = f.gep_bytes(
            &api,
            i64::try_from(abi::host_api::STRUCT_SIZE_OFFSET).unwrap_or(4),
        );
        let size = f.load(IrType::Int(32), &size_pointer, Some(4));
        let required_size = Value::int(
            32,
            i128::from(u32::try_from(abi::host_api::REQUIRED_SIZE).unwrap_or(u32::MAX)),
        );
        let short_table = f.icmp("ult", &size, &required_size);
        Self::return_if(&mut f, &short_table, 2);
        let expected = i128::from(u32::try_from(self.got.slots.len()).unwrap_or(u32::MAX));
        let wrong_count = f.icmp("ne", &slot_count, &Value::int(32, expected));
        Self::return_if(&mut f, &wrong_count, 3);

        f.store(&api, &Value::symbol(abi::module::API_GLOBAL), Some(8));
        f.store(&slots, &Value::symbol(abi::module::SLOTS_GLOBAL), Some(8));
        f.ret(Some(&Value::int(32, 0)));
        if let Err(error) = f.check() {
            self.push_error(CodegenError::internal(error));
        } else {
            self.module.add_function(f);
        }
    }

    fn emit_detach(&mut self) {
        let mut f = Function::new(
            abi::module::DETACH,
            SymbolAttrs::module_export(),
            IrType::Void,
            vec![(IrType::Int(64), "%generation".into())],
        );
        f.start_fresh_block();
        f.store(
            &Value::null_ptr(),
            &Value::symbol(abi::module::API_GLOBAL),
            Some(8),
        );
        f.store(
            &Value::null_ptr(),
            &Value::symbol(abi::module::SLOTS_GLOBAL),
            Some(8),
        );
        f.ret(None);
        if let Err(error) = f.check() {
            self.push_error(CodegenError::internal(error));
        } else {
            self.module.add_function(f);
        }
    }

    fn add_metadata_symbol(&mut self) {
        let metadata = self.build_metadata();
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
            SymbolAttrs::module_export(),
            IrType::Ptr,
            Vec::new(),
        );
        f.start_fresh_block();
        f.ret(Some(&Value::symbol(symbols::METADATA_DATA)));
        if let Err(error) = f.check() {
            self.push_error(CodegenError::internal(error));
        } else {
            self.module.add_function(f);
        }
    }

    fn contract(&self) -> Contract {
        let mut contract =
            Contract::for_module(self.got.slots.iter().map(|binding| binding.kind).collect());
        contract.roots.extend(self.global_symbols.values().cloned());
        contract
            .roots
            .extend(self.instance_globals.values().cloned());
        contract
            .roots
            .extend(self.task_guard_symbols.values().cloned());
        contract.roots.insert(symbols::IO_BASE.into());
        contract.roots.insert(symbols::IO_LEN.into());
        contract
            .functions
            .extend(self.function_symbols.values().cloned());
        for program in &self.plan.programs {
            contract
                .functions
                .insert(mangle::program_body_symbol(self.hir, *program));
        }
        contract
            .local_globals
            .insert(abi::module::API_GLOBAL.into());
        contract
            .local_globals
            .insert(abi::module::SLOTS_GLOBAL.into());
        contract.local_globals.insert(symbols::RETAIN_BUFFER.into());
        contract.local_globals.insert(symbols::METADATA_DATA.into());
        contract.descriptor = Some(DescriptorSpec {
            symbol: abi::module::DESCRIPTOR_DATA.into(),
            size: self.got.descriptor_bytes.len(),
        });
        contract.function_table = Some(FunctionTableSpec {
            symbol: abi::module::FUNCTION_TABLE.into(),
            count: self.got.descriptor.functions.len(),
        });
        contract
    }

    /// Returns the guard slot for a configured task.
    fn task_guard_slot(&self, resource: &str, task: Symbol) -> Result<u32, CodegenError> {
        let plan_task = self
            .plan
            .tasks
            .iter()
            .find(|candidate| candidate.resource == resource && candidate.name == task);
        let Some(plan_task) = plan_task else {
            return Err(CodegenError::internal("task guard has no planned task"));
        };
        let configuration = self
            .plan
            .configuration
            .as_deref()
            .map(str::to_ascii_uppercase)
            .unwrap_or_else(|| crate::module::SYNTHETIC_NAME.to_owned());
        let resource = if plan_task.resource.is_empty() {
            crate::module::SYNTHETIC_NAME.to_owned()
        } else {
            plan_task.resource.to_ascii_uppercase()
        };
        let target = RootTarget::TaskGuard {
            configuration,
            resource,
            task: plan_task.name_text.to_ascii_uppercase(),
        };
        self.root_slot(&target)
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
            .map(|binding| {
                let target = binding.target.map(|target| match target {
                    config::IoRootTarget::Global(def) => RootTarget::Global(def),
                    config::IoRootTarget::InstanceField { instance, field } => {
                        RootTarget::InstanceField { instance, field }
                    }
                });
                let got_id = target
                    .as_ref()
                    .and_then(|target| self.got.root_got_id(target))
                    .map(stc_got::GotId::to_hex);
                IoBindingMetadata {
                    variable: binding.variable.clone(),
                    area: binding.area.as_char(),
                    address: binding.address.clone(),
                    offset_bits: binding.start_bit,
                    width_bits: binding.kind.occupancy_bits(),
                    ty: binding.kind.canonical_name(),
                    got_id,
                    hint: binding.hint.as_ref().map(|hint| HintMetadata {
                        device: hint.device.clone(),
                        channel: hint.channel,
                        direction: hint.direction.map(|direction| direction.as_str()),
                    }),
                    source: binding.source,
                }
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
            build_id: self.got.manifest.header.build_id.to_hex(),
            target: self.target.triple.clone(),
            module_file: String::new(),
            manifest_digest: self.got.manifest_digest.to_hex(),
            descriptor_digest: self.got.descriptor_digest.to_hex(),
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
                files: crate::module::package_relative_paths(self.options.sources),
                entries: span_entries,
            },
        }
    }
}

fn config_overrides(plan: &Plan) -> BTreeMap<(Symbol, stc_hir::FieldId), ExprId> {
    let mut overrides = BTreeMap::new();
    for init in &plan.config_inits {
        if init.fields.len() == 1
            && let Some(value) = init.init
        {
            overrides.insert((init.instance, init.fields[0]), value);
        }
    }
    overrides
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
