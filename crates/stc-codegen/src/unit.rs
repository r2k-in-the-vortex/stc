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
use stc_got::model::DescriptorKind;
use stc_hir::{
    ArgDirection, DefId, DefKind, ExprId, ExprKindHir, HirCrate, ParamId, PlaceHir, Symbol, TyId,
    VarSectionKind,
};
use stc_span::Span;

use crate::CodegenError;
use crate::abi::{self, symbols};
use crate::codes;
use crate::config::{
    self, ImageLayout, Plan, PlanInstance, PlanVarAccess, ProgramLayout, RetainLayout, RetainPlace,
};
use crate::expr::{BodyInfo, FieldAccess};
use crate::ir::{Function, GlobalDef, IrType, Module, SymbolAttrs, Value, c_string};
use crate::layout::{self, ScalarKind};
use crate::mangle;
use crate::metadata::{
    DeviceMetadata, ExportMetadata, HintMetadata, IoBindingMetadata, ProgramMetadata,
    ProjectChannel, RetainLeafMetadata as MetaRetainLeaf, SpanMetadata, SpanMetadataEntry,
    TaskMetadata, VarAccessMetadata,
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
    pub types: crate::types::TypeSystem<'a>,
    pub debug: Option<crate::debug::DebugInfo<'a>>,
    pub layouts: BTreeMap<DefId, ProgramLayout>,
    pub objects: BTreeMap<DefId, config::ObjectLayout>,
    /// Leaf-ordinal map per class/FB type, keyed by root segments.
    pub object_ordinal_map: BTreeMap<DefId, BTreeMap<Vec<crate::types::RootSegment>, u32>>,
    pub used_std: BTreeSet<DefId>,
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

/// One callable parameter's implementation ABI.
struct AbiParam {
    id: ParamId,
    by_pointer: bool,
    llvm: IrType,
    storage: IrType,
    /// Function-block values pass a caller-owned record pointer.
    fb: bool,
}

/// Generates IR and metadata for a checked HIR crate.
pub(crate) fn generate(
    hir: &HirCrate,
    options: &Options<'_>,
    target: &Target,
    got: ModuleArtifacts,
) -> Result<Output, Vec<CodegenError>> {
    let plan = config::select_plan(hir)?;
    let types = crate::types::TypeSystem::new(hir, target)
        .map_err(|errors| errors.into_iter().collect::<Vec<_>>())?;
    let layouts = config::program_layouts(hir, &plan, &types)?;
    let objects = config::object_layouts(hir, &types, options.test_build)?;
    let object_ordinal_map = objects
        .iter()
        .map(|(def, layout)| {
            let ordinals = layout
                .leaves
                .iter()
                .enumerate()
                .map(|(index, leaf)| {
                    (
                        leaf.segments.clone(),
                        u32::try_from(index).unwrap_or(u32::MAX),
                    )
                })
                .collect();
            (*def, ordinals)
        })
        .collect();
    let used_std = crate::types::used_standard_fbs(hir);
    let mut image = config::image_layout(hir, &plan, &layouts)?;
    let mut retain = config::retain_layout(hir, &plan, &layouts, &types)?;
    // The retained layout hash is the state-file contract, so it covers the
    // profile/ABI/size header and each leaf's stable root identity (State §3.1).
    retain.hash = retain_layout_hash(&retain, &got);
    let project_io = config::apply_project_bindings(
        hir,
        &mut image,
        &options.project.io.devices,
        &options.project.io.bindings,
    );
    if !project_io.errors.is_empty() {
        return Err(project_io.errors);
    }
    let mut got = got;
    got.diagnostics.extend(project_io.warnings);
    let (span_ids, span_entries) = build_span_table(hir);
    let debug = crate::debug::DebugInfo::new(options, target);
    let mut lowerer = Lowerer {
        hir,
        options,
        module: Module::new(options.artifact_name, options.target),
        target: target.clone(),
        got,
        layouts,
        objects,
        object_ordinal_map,
        used_std,
        types,
        debug,
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
    if !lowerer.errors.is_empty() {
        return Err(lowerer.errors);
    }
    lowerer.emit();
    if !lowerer.errors.is_empty() {
        return Err(lowerer.errors);
    }
    if let Err(error) = lowerer.finalize_debug() {
        lowerer.push_error(error);
    }
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

/// The language profile identifier embedded in metadata and state hashes.
pub(crate) const PROFILE: &str = "stc-2013-oop-utf8-v1";

/// Returns the GOT root target for a retained leaf.
fn retain_root_target(place: &RetainPlace) -> RootTarget {
    match place {
        RetainPlace::Global(def) => RootTarget::Global(*def),
        RetainPlace::GlobalLeaf { def, segments } => RootTarget::GlobalLeaf {
            def: *def,
            segments: segments.clone(),
        },
        RetainPlace::InstanceField { instance, field } => RootTarget::InstanceField {
            instance: *instance,
            field: *field,
        },
        RetainPlace::InstanceLeaf { instance, segments } => RootTarget::InstanceLeaf {
            instance: *instance,
            segments: segments.clone(),
        },
    }
}

/// Computes the documented retained-layout hash (State §3.1).
///
/// The hash covers the profile, host ABI major, payload size, and each leaf's
/// path, stable root identity, type, offset, and size, so a layout change or a
/// different root identity changes the warm-start contract.
fn retain_layout_hash(retain: &RetainLayout, got: &ModuleArtifacts) -> u64 {
    let mut hash = layout::FNV_OFFSET;
    let header = format!(
        "profile={PROFILE}\nabi={}\nsize={}\n",
        abi::ABI_VERSION,
        retain.size
    );
    hash = layout::fnv1a_update(hash, header.as_bytes());
    for leaf in &retain.leaves {
        let got_id = got
            .root_got_id(&retain_root_target(&leaf.place))
            .map_or_else(
                || "0x00000000000000000000000000000000".into(),
                |id| id.to_hex(),
            );
        let line = format!(
            "leaf={}|{}|{}|{}|{}\n",
            leaf.path, got_id, leaf.ty, leaf.offset, leaf.size
        );
        hash = layout::fnv1a_update(hash, line.as_bytes());
    }
    hash
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
            let used_std = self.used_std.contains(&id);
            if (synthetic(self.hir, id) && !used_std) || (def.test_only && !self.options.test_build)
            {
                continue;
            }
            match &def.kind {
                DefKind::Function { body: Some(_), .. } => {
                    match mangle::function_symbol(self.hir, &self.types, id) {
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
                DefKind::FunctionBlock { body, .. } if body.is_some() || used_std => {
                    match mangle::fb_body_symbol(self.hir, id) {
                        Ok(symbol) => {
                            self.function_symbols.insert(id, symbol);
                        }
                        Err(error) => self.push_error(error.with_span(def.span)),
                    }
                }
                DefKind::Method { body: Some(_), .. } => {
                    match mangle::method_symbol(self.hir, &self.types, id) {
                        Ok(symbol) => {
                            self.function_symbols.insert(id, symbol);
                        }
                        Err(error) => self.push_error(error.with_span(def.span)),
                    }
                }
                DefKind::Method {
                    owner, body: None, ..
                } if matches!(self.hir.defs[owner.index()].kind, DefKind::Interface { .. }) => {
                    // Interface prototypes are projected as function records
                    // with a trap-stub body (GOT Artifacts §8.5.7).
                    match mangle::method_symbol(self.hir, &self.types, id) {
                        Ok(symbol) => {
                            self.function_symbols.insert(id, symbol);
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
            if synthetic(self.hir, id) || (def.test_only && !self.options.test_build) {
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
                            "a FUNCTION without a body is not supported".into(),
                            def.span,
                        ));
                    }
                    if let Some(result) = result
                        && let Err(error) = self.types.facts(*result)
                    {
                        self.push_error(error);
                    }
                    for param in params {
                        let data = self.hir.param(*param);
                        if let Err(error) = self.types.facts(data.ty) {
                            self.push_error(error);
                        }
                    }
                }
                DefKind::Global { ty, .. } => {
                    if let Err(error) = self.types.facts(*ty) {
                        self.push_error(error);
                    }
                }
                DefKind::Method { result, params, .. } => {
                    if let Some(result) = result
                        && let Err(error) = self.types.facts(*result)
                    {
                        self.push_error(error);
                    }
                    for param in params {
                        let data = self.hir.param(*param);
                        if let Err(error) = self.types.facts(data.ty) {
                            self.push_error(error);
                        }
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
                                    "program field section `{}` is not supported in M5",
                                    data.section.as_str()
                                ),
                                data.span,
                            ));
                        }
                        if let Err(error) = self.types.facts(data.ty) {
                            self.push_error(error);
                        }
                    }
                }
                DefKind::Namespace { .. }
                | DefKind::Alias { .. }
                | DefKind::Struct { .. }
                | DefKind::Enum { .. }
                | DefKind::NamedValue { .. }
                | DefKind::Subrange { .. }
                | DefKind::Class { .. }
                | DefKind::FunctionBlock { .. }
                | DefKind::Interface { .. }
                | DefKind::Configuration { .. } => {}
            }
        }
    }

    fn emit(&mut self) {
        if self.debug_enabled() {
            self.module.declare(
                "llvm.dbg.declare",
                "void @llvm.dbg.declare(metadata, metadata, metadata)",
            );
        }
        self.emit_module_data();
        self.emit_user_functions();
        self.emit_fb_bodies();
        self.emit_methods();
        self.emit_interface_stubs();
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

    fn abi_params(&self, params: &[ParamId]) -> Result<Vec<AbiParam>, CodegenError> {
        params
            .iter()
            .map(|param| self.abi_param(*param))
            .collect::<Result<Vec<_>, _>>()
    }

    fn abi_param(&self, param: ParamId) -> Result<AbiParam, CodegenError> {
        let data = self.hir.param(param).clone();
        let facts = self.types.facts(data.ty)?.clone();
        if facts.is_class() {
            // A class IN parameter is a handle value; OUT and VAR_IN_OUT
            // parameters receive the caller's handle slot.
            let by_pointer = data.direction != ArgDirection::Input;
            return Ok(AbiParam {
                id: param,
                by_pointer,
                llvm: IrType::Ptr,
                storage: IrType::Ptr,
                fb: false,
            });
        }
        if facts.is_object() {
            // An FB parameter is a caller-owned instance record for every
            // direction; IN receives a copy the caller made.
            return Ok(AbiParam {
                id: param,
                by_pointer: true,
                llvm: IrType::Ptr,
                storage: facts.llvm_storage.clone(),
                fb: true,
            });
        }
        let by_pointer = data.direction != ArgDirection::Input || facts.is_contiguous_aggregate();
        let storage = facts.llvm_storage.clone();
        let llvm = if by_pointer {
            IrType::Ptr
        } else {
            storage.clone()
        };
        Ok(AbiParam {
            id: param,
            by_pointer,
            llvm,
            storage,
            fb: false,
        })
    }

    fn bind_abi_params(
        &self,
        f: &mut Function,
        info: &mut BodyInfo,
        params: &[AbiParam],
        offset: usize,
    ) {
        for (index, param) in params.iter().enumerate() {
            let incoming = f.param_value(offset + index);
            if param.fb || param.by_pointer {
                info.params.insert(param.id, incoming);
            } else {
                let slot = f.alloca(param.storage.clone());
                f.store(&incoming, &slot, None);
                info.params.insert(param.id, slot);
            }
        }
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
            if synthetic(self.hir, def)
                || (self.hir.defs[def.index()].test_only && !self.options.test_build)
            {
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

        let result_facts = result.map(|ty| self.types.facts(ty).cloned()).transpose()?;
        let aggregate_result = result_facts
            .as_ref()
            .is_some_and(|facts| facts.is_contiguous_aggregate());
        let result_abi = result_facts.as_ref().map(|facts| {
            if aggregate_result {
                IrType::Void
            } else {
                facts.llvm_storage.clone()
            }
        });

        let param_data = self.abi_params(&params)?;

        let mut fn_params = param_data
            .iter()
            .enumerate()
            .map(|(index, param)| (param.llvm.clone(), format!("%a{index}")))
            .collect::<Vec<_>>();
        let sret_index = aggregate_result.then_some(param_data.len());
        if aggregate_result {
            fn_params.push((IrType::Ptr, "%sret".into()));
        }
        let slots_index = fn_params.len();
        fn_params.push((IrType::Ptr, "%slots".into()));
        let ret_ty = result_abi.clone().unwrap_or(IrType::Void);
        let mut f = Function::new(symbol, SymbolAttrs::internal(), ret_ty, fn_params);
        f.start_fresh_block();
        let debug_name = self.debug_function_name(def);
        self.begin_debug_function(
            &mut f,
            def,
            debug_name,
            result,
            &params,
            &body.locals,
            body.span,
        )?;
        let slots = f.param_value(slots_index);
        let mut info = BodyInfo {
            params: BTreeMap::new(),
            locals: BTreeMap::new(),
            local_objects: BTreeMap::new(),
            result_ptr: None,
            result_ty: None,
            slots: slots.clone(),
            fields: None,
            handle: None,
            return_label: f.next_block_label(),
        };
        self.bind_abi_params(&mut f, &mut info, &param_data, 0);
        self.declare_debug_params(&mut f, &params, &info);
        if aggregate_result {
            let pointer = f.param_value(sret_index.expect("aggregate result has sret"));
            let storage = self
                .types
                .llvm_storage(result.expect("aggregate result has a type"))?;
            f.store(&Value::zero(storage.clone()), &pointer, None);
            info.result_ptr = Some(pointer);
            info.result_ty = result_abi.clone();
        } else if let Some(facts) = &result_facts {
            let ty = result_abi
                .clone()
                .unwrap_or_else(|| facts.llvm_storage.clone());
            let slot = f.alloca(ty.clone());
            f.store(&Value::zero(ty), &slot, None);
            info.result_ptr = Some(slot);
            info.result_ty = result_abi.clone();
        }
        self.lower_result_objects(&mut f, &mut info, def, result)?;
        let previous = self.current_slots.replace(slots);
        let result = (|| -> Result<(), CodegenError> {
            self.lower_locals(&mut f, &mut info, &body.locals)?;
            let mut loops: Vec<LoopTarget> = Vec::new();
            self.lower_stmts(&mut f, &info, &body.stmts, &mut loops)?;
            if !f.is_terminated() {
                f.branch(&info.return_label);
            }
            f.start_block(&info.return_label);
            if aggregate_result {
                f.ret(None);
            } else if let (Some(pointer), Some(ty)) = (&info.result_ptr, info.result_ty.clone()) {
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

    /// Allocates the owned instance of an FB result.
    fn lower_result_objects(
        &mut self,
        f: &mut Function,
        info: &mut BodyInfo,
        def: DefId,
        result: Option<TyId>,
    ) -> Result<(), CodegenError> {
        let Some(ty) = result else {
            return Ok(());
        };
        let facts = self.types.facts(ty)?.clone();
        if !facts.is_object() || facts.is_class() {
            return Ok(());
        }
        let Some(pointer) = info.result_ptr.clone() else {
            return Ok(());
        };
        self.allocate_object_tree(f, info, crate::expr::ObjectRoot::Result(def), ty, &pointer)
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
            let facts = self.types.facts(data.ty)?.clone();
            let has_class = self.types.type_has_class(data.ty);
            let storage_type = if facts.is_class() {
                IrType::Ptr
            } else if has_class {
                self.types.instance_llvm_storage(data.ty)?
            } else {
                facts.llvm_storage.clone()
            };
            let slot = f.alloca(storage_type.clone());
            info.locals.insert(*local, slot.clone());
            self.declare_debug_local(f, *local, &slot);
            f.store(&Value::zero(storage_type.clone()), &slot, None);
            if has_class {
                self.allocate_object_tree(
                    f,
                    info,
                    crate::expr::ObjectRoot::Local(*local),
                    data.ty,
                    &slot,
                )?;
            }
            if let Some(init) = data.init {
                let value = self.lower_expr(f, info, init)?;
                self.store_value_as_storage(f, &slot, data.ty, value)?;
            }
        }
        Ok(())
    }

    /// Allocates the stack instances and records of every class/FB object
    /// owned by one local declaration or result.
    ///
    /// A class value is a handle slot; its owned instance is a separate alloca.
    /// An FB value is an inline instance; nested function blocks share the
    /// enclosing instance storage. Every object gets a `[N+1 x ptr]` record
    /// whose first entry is the type's TYPEINFO address and whose tail is the
    /// base-first leaf storage table (GOT Artifacts §8.5.1, §8.5.6).
    fn allocate_object_tree(
        &mut self,
        f: &mut Function,
        info: &mut BodyInfo,
        root: crate::expr::ObjectRoot,
        ty: TyId,
        local_storage: &Value,
    ) -> Result<(), CodegenError> {
        let plan = self.types.storage_plan(ty, stc_hir::RetainPolicy::None)?;
        if plan.objects.is_empty() {
            return Ok(());
        }
        let mut allocated: Vec<(Vec<crate::types::RootSegment>, crate::expr::LocalObject)> =
            Vec::with_capacity(plan.objects.len());
        for object in &plan.objects {
            let named_ty = self.types.named_ty(object.def)?;
            let instance_type = self.types.instance_llvm_storage(named_ty)?;
            let location = if let Some((container, container_ty, relative)) =
                self.object_container(&allocated, ty, local_storage, &object.segments)
            {
                Some(self.aggregate_pointer(f, container_ty, &container, &relative)?)
            } else {
                None
            };
            let instance = if object.class {
                let instance = f.alloca(instance_type.clone());
                // Each class owns a fresh instance; nested instances are not
                // part of the declaration's zero-initialized storage.
                f.store(&Value::zero(instance_type), &instance, None);
                instance
            } else if object.segments.is_empty() {
                local_storage.clone()
            } else {
                location.clone().ok_or_else(|| {
                    CodegenError::internal("a nested FB instance has no container storage")
                })?
            };
            let record_type = self.types.object_record_llvm(object.def)?;
            let record = f.alloca_typed(&record_type);
            let handle = if object.class {
                match object.segments.is_empty() {
                    true => local_storage.clone(),
                    false => location.ok_or_else(|| {
                        CodegenError::internal("a nested class value has no handle slot")
                    })?,
                }
            } else {
                record.clone()
            };
            allocated.push((
                object.segments.clone(),
                crate::expr::LocalObject {
                    def: object.def,
                    ty: named_ty,
                    class: object.class,
                    segments: object.segments.clone(),
                    instance,
                    record,
                    handle,
                },
            ));
        }
        for (index, (_, object)) in allocated.iter().enumerate() {
            let typeinfo = self.typeinfo_value(f, info, object.def)?;
            self.write_record_entry(f, &object.record, 0, &typeinfo);
            let leaves = plan.objects[index].leaves.clone();
            for (leaf_index, leaf) in leaves.iter().enumerate() {
                // Storage-plan object leaves carry the instance's declaration
                // path; the record stores paths relative to the object type.
                let relative = leaf.segments.get(object.segments.len()..).unwrap_or(&[]);
                let pointer = self.aggregate_pointer(f, object.ty, &object.instance, relative)?;
                self.write_record_entry(f, &object.record, leaf_index + 1, &pointer);
            }
        }
        for (segments, object) in allocated {
            if object.class {
                f.store(&object.record, &object.handle, Some(8));
            }
            info.local_objects.insert((root.clone(), segments), object);
        }
        Ok(())
    }

    /// Finds the storage of an object's containing instance.
    fn object_container(
        &self,
        allocated: &[(Vec<crate::types::RootSegment>, crate::expr::LocalObject)],
        local_ty: TyId,
        local_storage: &Value,
        segments: &[crate::types::RootSegment],
    ) -> Option<(Value, TyId, Vec<crate::types::RootSegment>)> {
        let parent = allocated
            .iter()
            .filter(|(prefix, _)| prefix.len() < segments.len() && segments.starts_with(prefix))
            .max_by_key(|(prefix, _)| prefix.len());
        match parent {
            Some((prefix, object)) => Some((
                object.instance.clone(),
                object.ty,
                segments[prefix.len()..].to_vec(),
            )),
            None => Some((local_storage.clone(), local_ty, segments.to_vec())),
        }
    }

    /// Emits the record-entry store used by local object initialization.
    fn write_record_entry(&self, f: &mut Function, record: &Value, index: usize, value: &Value) {
        let slot = f.gep("ptr", record, &[format!("i64 {index}")], true);
        f.store(value, &slot, Some(8));
    }

    fn emit_fb_bodies(&mut self) {
        let defs: Vec<DefId> = self
            .hir
            .defs
            .iter()
            .enumerate()
            .filter_map(|(index, def)| {
                let id = DefId::new(u32::try_from(index).ok()?);
                if def.test_only && !self.options.test_build {
                    return None;
                }
                match &def.kind {
                    DefKind::FunctionBlock { body, .. } => {
                        (body.is_some() || self.used_std.contains(&id)).then_some(id)
                    }
                    _ => None,
                }
            })
            .collect();
        for def in defs {
            if let Err(error) = self.emit_fb_body(def) {
                self.push_error(error);
            }
        }
    }

    fn emit_fb_body(&mut self, def: DefId) -> Result<(), CodegenError> {
        let DefKind::FunctionBlock { body, .. } = self.hir.defs[def.index()].kind.clone() else {
            return Ok(());
        };
        let Some(body_id) = body else {
            let Some(name) = crate::types::standard_fb_name(self.hir, def) else {
                return Ok(());
            };
            return self.emit_standard_fb_body(def, name);
        };
        let symbol = self
            .function_symbols
            .get(&def)
            .cloned()
            .ok_or_else(|| CodegenError::internal("FB body has no symbol"))?;
        let layout = self
            .objects
            .get(&def)
            .cloned()
            .ok_or_else(|| CodegenError::internal("FB type has no object layout"))?;
        let mut f = Function::new(
            symbol,
            SymbolAttrs::internal(),
            IrType::Void,
            vec![
                (IrType::Ptr, "%this".into()),
                (IrType::Ptr, "%table".into()),
                (IrType::Ptr, "%slots".into()),
            ],
        );
        f.start_fresh_block();
        let ordinals = self.object_ordinals(&layout);
        let slots = f.param_value(2);
        let mut info = BodyInfo {
            params: BTreeMap::new(),
            locals: BTreeMap::new(),
            local_objects: BTreeMap::new(),
            result_ptr: None,
            result_ty: None,
            slots: slots.clone(),
            fields: Some(FieldAccess {
                pointer: f.param_value(1),
                ordinals,
                direct: BTreeMap::new(),
            }),
            handle: Some(f.param_value(0)),
            return_label: f.next_block_label(),
        };
        let body = self.hir.bodies[body_id.index()].clone();
        let debug_name = self.debug_function_name(def);
        self.begin_debug_function(&mut f, def, debug_name, None, &[], &body.locals, body.span)?;
        let previous = self.current_slots.replace(slots);
        let result = (|| -> Result<(), CodegenError> {
            self.lower_locals(&mut f, &mut info, &body.locals)?;
            let mut loops: Vec<LoopTarget> = Vec::new();
            self.lower_stmts(&mut f, &info, &body.stmts, &mut loops)?;
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

    fn emit_methods(&mut self) {
        let methods: Vec<DefId> = self
            .hir
            .defs
            .iter()
            .enumerate()
            .filter_map(|(index, def)| {
                if def.test_only && !self.options.test_build {
                    return None;
                }
                matches!(def.kind, DefKind::Method { body: Some(_), .. })
                    .then_some(DefId::new(u32::try_from(index).ok()?))
            })
            .collect();
        for def in methods {
            if let Err(error) = self.emit_method(def) {
                self.push_error(error);
            }
        }
    }

    fn emit_method(&mut self, def: DefId) -> Result<(), CodegenError> {
        let DefKind::Method {
            owner,
            result,
            params,
            body: Some(body_id),
            ..
        } = self.hir.defs[def.index()].kind.clone()
        else {
            return Ok(());
        };
        let symbol = self
            .function_symbols
            .get(&def)
            .cloned()
            .ok_or_else(|| CodegenError::internal("method has no symbol"))?;
        let layout = self
            .objects
            .get(&owner)
            .cloned()
            .ok_or_else(|| CodegenError::internal("method owner has no object layout"))?;
        let result_facts = result.map(|ty| self.types.facts(ty).cloned()).transpose()?;
        let aggregate_result = result_facts
            .as_ref()
            .is_some_and(|facts| facts.is_contiguous_aggregate());
        let result_abi = result_facts.as_ref().map(|facts| {
            if aggregate_result {
                IrType::Void
            } else {
                facts.llvm_storage.clone()
            }
        });
        let param_data = self.abi_params(&params)?;
        let mut fn_params = vec![
            (IrType::Ptr, "%this".into()),
            (IrType::Ptr, "%table".into()),
        ];
        fn_params.extend(
            param_data
                .iter()
                .enumerate()
                .map(|(index, param)| (param.llvm.clone(), format!("%a{index}"))),
        );
        let sret_index = if aggregate_result {
            let index = fn_params.len();
            fn_params.push((IrType::Ptr, "%sret".into()));
            Some(index)
        } else {
            None
        };
        let slots_index = fn_params.len();
        fn_params.push((IrType::Ptr, "%slots".into()));
        let ret_ty = result_abi.clone().unwrap_or(IrType::Void);
        let mut f = Function::new(symbol, SymbolAttrs::internal(), ret_ty, fn_params);
        f.start_fresh_block();
        let ordinals = self.object_ordinals(&layout);
        let slots = f.param_value(slots_index);
        let mut info = BodyInfo {
            params: BTreeMap::new(),
            locals: BTreeMap::new(),
            local_objects: BTreeMap::new(),
            result_ptr: None,
            result_ty: None,
            slots: slots.clone(),
            fields: Some(FieldAccess {
                pointer: f.param_value(1),
                ordinals,
                direct: BTreeMap::new(),
            }),
            handle: Some(f.param_value(0)),
            return_label: f.next_block_label(),
        };
        let body = self.hir.bodies[body_id.index()].clone();
        let debug_name = self.debug_function_name(def);
        self.begin_debug_function(
            &mut f,
            def,
            debug_name,
            result,
            &params,
            &body.locals,
            body.span,
        )?;
        self.bind_abi_params(&mut f, &mut info, &param_data, 2);
        self.declare_debug_params(&mut f, &params, &info);
        if aggregate_result {
            let pointer = f.param_value(sret_index.expect("aggregate result has sret"));
            let storage = self
                .types
                .llvm_storage(result.expect("aggregate result has a type"))?;
            f.store(&Value::zero(storage.clone()), &pointer, None);
            info.result_ptr = Some(pointer);
            info.result_ty = result_abi.clone();
        } else if let Some(facts) = &result_facts {
            let ty = result_abi
                .clone()
                .unwrap_or_else(|| facts.llvm_storage.clone());
            let slot = f.alloca(ty.clone());
            f.store(&Value::zero(ty), &slot, None);
            info.result_ptr = Some(slot);
            info.result_ty = result_abi.clone();
        }
        self.lower_result_objects(&mut f, &mut info, def, result)?;
        let previous = self.current_slots.replace(slots);
        let result = (|| -> Result<(), CodegenError> {
            self.lower_locals(&mut f, &mut info, &body.locals)?;
            let mut loops: Vec<LoopTarget> = Vec::new();
            self.lower_stmts(&mut f, &info, &body.stmts, &mut loops)?;
            if !f.is_terminated() {
                f.branch(&info.return_label);
            }
            f.start_block(&info.return_label);
            if aggregate_result {
                f.ret(None);
            } else if let (Some(pointer), Some(ty)) = (&info.result_ptr, info.result_ty.clone()) {
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

    fn emit_interface_stubs(&mut self) {
        let stubs: Vec<DefId> = self
            .hir
            .defs
            .iter()
            .enumerate()
            .filter_map(|(index, def)| {
                if def.test_only && !self.options.test_build {
                    return None;
                }
                let DefKind::Method {
                    owner, body: None, ..
                } = &def.kind
                else {
                    return None;
                };
                matches!(self.hir.defs[owner.index()].kind, DefKind::Interface { .. })
                    .then_some(DefId::new(u32::try_from(index).ok()?))
            })
            .collect();
        for def in stubs {
            if let Err(error) = self.emit_interface_stub(def) {
                self.push_error(error);
            }
        }
    }

    /// Emits the trap-stub body of one interface prototype (GOT Artifacts
    /// §8.5.7). Dispatched interface calls never reach this body; reaching it
    /// means dispatch was bypassed or an implementation is missing.
    fn emit_interface_stub(&mut self, def: DefId) -> Result<(), CodegenError> {
        let DefKind::Method { result, params, .. } = self.hir.defs[def.index()].kind.clone() else {
            return Ok(());
        };
        let symbol = self
            .function_symbols
            .get(&def)
            .cloned()
            .ok_or_else(|| CodegenError::internal("interface prototype has no symbol"))?;
        let result_facts = result.map(|ty| self.types.facts(ty).cloned()).transpose()?;
        let aggregate_result = result_facts
            .as_ref()
            .is_some_and(|facts| facts.is_contiguous_aggregate());
        let result_abi = result_facts.as_ref().map(|facts| {
            if aggregate_result {
                IrType::Void
            } else {
                facts.llvm_storage.clone()
            }
        });
        let param_data = self.abi_params(&params)?;
        let mut fn_params = vec![
            (IrType::Ptr, "%this".into()),
            (IrType::Ptr, "%table".into()),
        ];
        fn_params.extend(
            param_data
                .iter()
                .enumerate()
                .map(|(index, param)| (param.llvm.clone(), format!("%a{index}"))),
        );
        if aggregate_result {
            fn_params.push((IrType::Ptr, "%sret".into()));
        }
        let slots_index = fn_params.len();
        fn_params.push((IrType::Ptr, "%slots".into()));
        let ret_ty = result_abi.unwrap_or(IrType::Void);
        let mut f = Function::new(symbol, SymbolAttrs::internal(), ret_ty, fn_params);
        f.start_fresh_block();
        let slots = f.param_value(slots_index);
        let previous = self.current_slots.replace(slots.clone());
        let result = (|| -> Result<(), CodegenError> {
            let slot = self.import_slot(ImportService::Trap)?;
            let span_id = self.span_id(self.hir.defs[def.index()].span);
            let trap_fn = f.slot_value(&slots, slot);
            f.call_ptr(
                IrType::Void,
                &trap_fn,
                &[
                    Value::int(32, i128::from(abi::trap::UNIMPLEMENTED)),
                    Value::int(32, i128::from(span_id)),
                ],
            );
            f.unreachable();
            f.check().map_err(CodegenError::internal)?;
            self.module.add_function(f);
            Ok(())
        })();
        self.current_slots = previous;
        result
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
            vec![
                (IrType::Ptr, "%fields".into()),
                (IrType::Ptr, "%slots".into()),
            ],
        );
        f.start_fresh_block();
        let layout = self
            .layouts
            .get(&program)
            .cloned()
            .ok_or_else(|| CodegenError::internal("program has no leaf layout"))?;
        let ordinals = self.leaf_ordinals(&layout);
        let direct = layout
            .leaves
            .iter()
            .filter_map(|leaf| leaf.at.clone().map(|at| (leaf.segments.clone(), at)))
            .collect();
        let body = body.map(|body_id| self.hir.bodies[body_id.index()].clone());
        let slots = f.param_value(1);
        let mut info = BodyInfo {
            params: BTreeMap::new(),
            locals: BTreeMap::new(),
            local_objects: BTreeMap::new(),
            result_ptr: None,
            result_ty: None,
            slots: slots.clone(),
            fields: Some(FieldAccess {
                pointer: f.param_value(0),
                ordinals,
                direct,
            }),
            handle: None,
            return_label: f.next_block_label(),
        };
        if let Some(body) = &body {
            let debug_name = self.debug_function_name(program);
            self.begin_debug_function(
                &mut f,
                program,
                debug_name,
                None,
                &[],
                &body.locals,
                body.span,
            )?;
        }
        let previous = self.current_slots.replace(slots);
        let result = (|| -> Result<(), CodegenError> {
            if let Some(body) = &body {
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
            local_objects: BTreeMap::new(),
            result_ptr: None,
            result_ty: None,
            slots,
            fields: None,
            handle: None,
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
        self.emit_state_load(f, info, &cold)?;
        self.emit_object_records(f, info, &cold, &mask)?;
        f.ret(Some(&Value::int(32, 0)));
        Ok(())
    }

    /// Fills fresh hidden object records and class handles from the active
    /// slot table (GOT Artifacts §8.5.1, §8.5.6; Runtime and Execution §2.6).
    fn emit_object_records(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        cold: &Value,
        mask: &Value,
    ) -> Result<(), CodegenError> {
        let objects = self.got.objects.clone();
        for object in &objects {
            let record_slot = self.root_slot(&object.target)?;
            let typeinfo_slot = self
                .got
                .descriptor_slot(object.def, DescriptorKind::TypeInfo)
                .ok_or_else(|| {
                    CodegenError::internal(format!(
                        "object type `{}` has no TYPEINFO descriptor slot",
                        self.hir.def_name(object.def)
                    ))
                })?;
            // The record's storage address is valid whether the root is fresh
            // or preserved; load it before the conditional fill so the handle
            // block below can use the same value.
            let record = Self::slot_storage(f, info, record_slot);
            let (_, next) = Self::begin_init(f, cold, mask, Some(record_slot))?;
            let typeinfo = Self::slot_storage(f, info, typeinfo_slot);
            Self::store_record_entry(f, &record, 0, &typeinfo);
            for (index, leaf) in object.leaves.iter().enumerate() {
                let slot = self.root_slot(leaf)?;
                let pointer = Self::slot_storage(f, info, slot);
                Self::store_record_entry(f, &record, index + 1, &pointer);
            }
            Self::close_init(f, &next);
            let Some(handle) = &object.handle else {
                continue;
            };
            let handle_slot = self.root_slot(&handle.target)?;
            let (_, next) = Self::begin_init(f, cold, mask, Some(handle_slot))?;
            let storage = Self::slot_storage(f, info, handle_slot);
            let pointer = match handle.index {
                Some(index) => f.gep("ptr", &storage, &[format!("i64 {index}")], true),
                None => storage,
            };
            f.store(&record, &pointer, Some(8));
            Self::close_init(f, &next);
        }
        Ok(())
    }

    fn store_record_entry(f: &mut Function, record: &Value, index: usize, value: &Value) {
        let slot = f.gep("ptr", record, &[format!("i64 {index}")], true);
        f.store(value, &slot, Some(8));
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
            stc_hir::RetainPolicy,
            String,
            Span,
        );
        let globals: Vec<GlobalInit> = self
            .hir
            .defs
            .iter()
            .enumerate()
            .filter_map(|(index, def)| {
                let id = DefId::new(u32::try_from(index).ok()?);
                if synthetic(self.hir, id) || (def.test_only && !self.options.test_build) {
                    return None;
                }
                let DefKind::Global {
                    ty,
                    init,
                    at,
                    retain,
                    ..
                } = &def.kind
                else {
                    return None;
                };
                let name = self.hir.def_name(id).to_string();
                Some((id, *ty, *init, at.clone(), *retain, name, def.span))
            })
            .collect();
        for (def, ty, init, at, retain, name, span) in globals {
            let Some(init) = init else {
                continue;
            };
            let storage = self.types.llvm_storage(ty)?;
            let value = self.lower_expr(f, info, init)?;
            let temp = f.alloca(storage.clone());
            f.store(&value, &temp, None);
            let leaves = self
                .types
                .root_leaves(ty, &name, retain)
                .map_err(|error| error.with_span(span))?;
            for leaf in leaves {
                let target =
                    Self::global_leaf_target(def, &leaf.segments, leaf.class_handle.is_some());
                let slot = self.root_slot(&target)?;
                let direct = at.clone();
                let (_, next) = Self::begin_init(
                    f,
                    cold,
                    mask,
                    if direct.is_some() { None } else { Some(slot) },
                )?;
                let source = if leaf.segments.is_empty() {
                    temp.clone()
                } else {
                    self.aggregate_pointer(f, ty, &temp, &leaf.segments)?
                };
                let leaf_value = f.load(self.types.llvm_storage(leaf.ty)?, &source, None);
                let destination = crate::place::ResolvedPointer {
                    pointer: Self::slot_storage(f, info, slot),
                    direct,
                };
                self.write_leaf(f, &destination, leaf.ty, leaf_value)?;
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
                let field_name = mangle::normalize_ident(self.hir.interner.text(data.name));
                let storage = self.types.llvm_storage(data.ty)?;
                let value = self.lower_expr(f, info, init)?;
                let temp = f.alloca(storage.clone());
                f.store(&value, &temp, None);
                let leaves = self
                    .types
                    .root_leaves(data.ty, &field_name, data.retain)
                    .map_err(|error| error.with_span(data.span))?;
                for mut leaf in leaves {
                    let top_level = leaf.segments.is_empty();
                    leaf.segments
                        .insert(0, crate::types::RootSegment::Field(field));
                    let target = self.program_leaf_target(
                        instance.name,
                        &leaf.segments,
                        leaf.class_handle.is_some(),
                    );
                    let slot = self.root_slot(&target)?;
                    let direct = if top_level { data.at.clone() } else { None };
                    let (_, next) = Self::begin_init(
                        f,
                        cold,
                        mask,
                        if direct.is_some() { None } else { Some(slot) },
                    )?;
                    let relative = &leaf.segments[1..];
                    let source = if relative.is_empty() {
                        temp.clone()
                    } else {
                        self.aggregate_pointer(f, data.ty, &temp, relative)?
                    };
                    let leaf_value = f.load(self.types.llvm_storage(leaf.ty)?, &source, None);
                    let destination = crate::place::ResolvedPointer {
                        pointer: Self::slot_storage(f, info, slot),
                        direct,
                    };
                    self.write_leaf(f, &destination, leaf.ty, leaf_value)?;
                    Self::close_init(f, &next);
                }
            }
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
        for index in self.plan.cycle_instances() {
            let instance = &instances[index];
            if let Some(task) = instance.task {
                // The host owns the guard: it sets the root before every scan
                // and the cycle never clears it (D-154, Runtime §1.3).
                let slot = self.task_guard_slot(&instance.resource, task)?;
                let guard_value = Self::slot_storage(f, info, slot);
                let guard = Self::read_scalar(f, &guard_value, ScalarKind::Bool)?;
                let run = f.next_block_label();
                let skip = f.next_block_label();
                f.cond_branch(&guard, &run, &skip);
                f.start_block(&run);
                self.emit_instance_cycle(f, info, instance)?;
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
        let layout = self
            .layouts
            .get(&instance.program)
            .cloned()
            .ok_or_else(|| CodegenError::internal("program has no leaf layout"))?;
        let ordinals = self.leaf_ordinals(&layout);
        let direct = layout
            .leaves
            .iter()
            .filter_map(|leaf| leaf.at.clone().map(|at| (leaf.segments.clone(), at)))
            .collect();
        let mut pointers = Vec::new();
        for leaf in &layout.leaves {
            let target = self.program_leaf_target(
                instance.name,
                &leaf.segments,
                leaf.class_handle.is_some(),
            );
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
            let target = match &leaf.place {
                RetainPlace::Global(def) => RootTarget::Global(*def),
                RetainPlace::GlobalLeaf { def, segments } => RootTarget::GlobalLeaf {
                    def: *def,
                    segments: segments.clone(),
                },
                RetainPlace::InstanceField { instance, field } => RootTarget::InstanceField {
                    instance: *instance,
                    field: *field,
                },
                RetainPlace::InstanceLeaf { instance, segments } => RootTarget::InstanceLeaf {
                    instance: *instance,
                    segments: segments.clone(),
                },
            };
            let slot = self.root_slot(&target)?;
            let source = Self::slot_storage(f, info, slot);
            let offset = f.gep(
                buffer_type,
                &buffer,
                &["i64 0".into(), format!("i64 {}", leaf.offset)],
                true,
            );
            if saving {
                let value = f.load(leaf.storage.clone(), &source, None);
                f.store(&value, &offset, Some(1));
            } else {
                let value = f.load(leaf.storage.clone(), &offset, Some(1));
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
                if def.test_only && !self.options.test_build {
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
        let mut f = Function::new(
            symbol,
            SymbolAttrs::module_export(),
            ret_ty.clone(),
            fn_params,
        );
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
        match &ret_ty {
            IrType::Void => f.ret(None),
            _ => f.ret(Some(&Value::zero(ret_ty.clone()))),
        }
        f.start_block(&call);
        let slot = self.function_slot(FunctionTarget::Function(def))?;
        let callee = Self::slot_storage(f, info, slot);
        let mut args = (0..argc)
            .map(|index| f.param_value(index))
            .collect::<Vec<_>>();
        args.push(info.slots.clone());
        let value = f.call_ptr(ret_ty.clone(), &callee, &args);
        match (&ret_ty, value) {
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
        let required_size = if self.options.test_build {
            abi::host_api::REQUIRED_SIZE_TEST
        } else {
            abi::host_api::REQUIRED_SIZE
        };
        let required_size = Value::int(
            32,
            i128::from(u32::try_from(required_size).unwrap_or(u32::MAX)),
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

    /// Resolves one `VAR_ACCESS` entry to its metadata record.
    fn var_access_metadata(&self, access: &PlanVarAccess) -> VarAccessMetadata {
        let instance = access.instance_name.as_deref();
        let path = self
            .render_place_path(&access.place, instance)
            .unwrap_or_else(|| access.name.clone());
        let mut metadata = VarAccessMetadata {
            name: access.name.clone(),
            path: path.clone(),
            ty: self.hir.type_name(access.ty),
            read_only: access.read_only,
            area: None,
            address: None,
            offset_bits: None,
            width_bits: None,
        };
        if let PlaceHir::Direct(address) = &access.place {
            if let Ok(kind) = layout::scalar_kind_at(self.hir, access.ty, access.span)
                && let Ok(span) = layout::image_span(address, kind)
            {
                metadata.area = Some(span.area.as_char());
                metadata.address = Some(layout::render_address(address));
                metadata.offset_bits = Some(span.start_bit);
                metadata.width_bits = Some(kind.occupancy_bits());
            }
            return metadata;
        }
        if let Some(binding) = self
            .image
            .bindings
            .iter()
            .find(|binding| binding.variable.eq_ignore_ascii_case(&path))
        {
            metadata.area = Some(binding.area.as_char());
            metadata.address = Some(binding.address.clone());
            metadata.offset_bits = Some(binding.start_bit);
            metadata.width_bits = Some(binding.kind.occupancy_bits());
        }
        metadata
    }

    /// Renders a resolved place as a dotted variable path.
    fn render_place_path(&self, place: &PlaceHir, instance: Option<&str>) -> Option<String> {
        match place {
            PlaceHir::Global(def) => Some(self.qualified_path(*def)),
            PlaceHir::This(def) => {
                Some(instance.map_or_else(|| self.hir.def_name(*def).to_string(), str::to_owned))
            }
            PlaceHir::Field { base, field } => {
                let ExprKindHir::Place(base) = &self.hir.expr(*base).kind else {
                    return None;
                };
                let base = self.render_place_path(base, instance)?;
                Some(format!("{base}.{}", self.field_name(*field)))
            }
            PlaceHir::InstanceField {
                instance, fields, ..
            } => {
                let mut path = self.hir.interner.text(*instance).to_string();
                for field in fields {
                    path.push('.');
                    path.push_str(self.field_name(*field));
                }
                Some(path)
            }
            PlaceHir::Direct(address) => Some(format!("%{}", layout::render_address(address))),
            _ => None,
        }
    }

    pub(crate) fn field_name(&self, field: stc_hir::FieldId) -> &str {
        self.hir.interner.text(self.hir.field(field).name)
    }

    /// Returns a definition's namespace-qualified source spelling.
    pub(crate) fn qualified_path(&self, def: DefId) -> String {
        let mut segments = Vec::new();
        let mut current = Some(def);
        while let Some(id) = current {
            let data = &self.hir.defs[id.index()];
            if id == def {
                if let Some(name) = data.name {
                    segments.push(self.hir.interner.text(name).to_string());
                }
            } else if matches!(data.kind, DefKind::Namespace { .. }) {
                // The package root namespace is implicit in source paths.
                if data.parent.is_some()
                    && let Some(name) = data.name
                {
                    segments.push(self.hir.interner.text(name).to_string());
                }
            } else {
                break;
            }
            current = data.parent;
        }
        segments.reverse();
        segments.join(".")
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
                    direct_id: got_id.clone(),
                    got_id,
                    device: binding.device.clone(),
                    channel: binding.channel.as_ref().map(|channel| match channel {
                        ProjectChannel::Index(index) => {
                            crate::metadata::Json::Number(i128::from(*index))
                        }
                        ProjectChannel::Name(name) => crate::metadata::Json::String(name.clone()),
                    }),
                    direction: binding.direction.map(str::to_owned),
                    hint: binding.hint.as_ref().map(|hint| HintMetadata {
                        device: hint.device.clone(),
                        channel: hint.channel,
                        direction: hint.direction.map(|direction| direction.as_str()),
                    }),
                    source: binding.source,
                }
            })
            .collect();
        let devices: Vec<DeviceMetadata> = self.options.project.io.devices.clone();
        let var_access = self
            .plan
            .var_access
            .iter()
            .map(|access| self.var_access_metadata(access))
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
                if def.test_only && !self.options.test_build {
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
            profile: PROFILE,
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
                backend: self.options.project.io.backend.clone(),
                file: self.options.project.io.file.clone().or_else(|| {
                    (self.options.project.io.backend == "file")
                        .then(|| format!("{}.stcio", self.options.artifact_name))
                }),
                inputs_bytes: self.image.inputs_bytes,
                outputs_bytes: self.image.outputs_bytes,
                markers_bytes: self.image.markers_bytes,
                input_defaults_hex: hex(&self.image.input_defaults),
                output_defaults_hex: hex(&self.image.output_defaults),
                devices,
                bindings,
                var_access,
            },
            retain: crate::metadata::RetainMetadata {
                enabled: self.options.project.retain.enabled,
                file: self
                    .options
                    .project
                    .retain
                    .file
                    .clone()
                    .unwrap_or_else(|| format!("{}.stcstate", self.options.artifact_name)),
                on_mismatch: self.options.project.retain.on_mismatch.to_owned(),
                size: self.retain.size,
                layout_hash: self.retain.hash,
                leaves,
            },
            exports,
            spans: SpanMetadata {
                files: match self.options.source_paths {
                    Some(paths) => paths.to_vec(),
                    None => crate::module::package_relative_paths(self.options.sources),
                },
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
