//! Configuration selection, program layouts, process-image layout, and
//! scalar retained-state layout (D-083, D-084, D-087).

use std::collections::{BTreeMap, BTreeSet};

use stc_hir::{
    BindingHir, ConstValue, DefId, DefKind, DirectArea, ExprId, ExprKindHir, FieldId, HirCrate,
    IoHint, PlaceHir, RetainPolicy, Symbol, TaskHir, TyId,
};
use stc_span::Span;

use crate::CodegenError;
use crate::codes;
use crate::ir::IrType;
use crate::layout::{self, ScalarKind};
use crate::mangle;
use crate::types::{RootSegment, StorageLayout, TypeCategory, TypeSystem};

/// One task selected for execution/metadata.
#[derive(Clone, Debug)]
pub(crate) struct PlanTask {
    /// Task name.
    pub name: Symbol,
    /// Source spelling.
    pub name_text: String,
    /// Resource name.
    pub resource: String,
    /// Folded `SINGLE`.
    pub single: Option<bool>,
    /// Folded interval.
    pub interval_ns: Option<i128>,
    /// Folded priority.
    pub priority: Option<i128>,
    /// Declaration span.
    pub span: Span,
}

/// One configured program instance.
#[derive(Clone, Debug)]
pub(crate) struct PlanInstance {
    /// Instance name.
    pub name: Symbol,
    /// Source spelling.
    pub name_text: String,
    /// Program type.
    pub program: DefId,
    /// Resource name.
    pub resource: String,
    /// Bound task.
    pub task: Option<Symbol>,
    /// Bindings in source order.
    pub bindings: Vec<BindingHir>,
    /// Instance global symbol.
    pub global: String,
    /// Declaration span.
    pub span: Span,
}

/// The selected execution plan.
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    /// Configuration name, when a configuration was selected.
    pub configuration: Option<String>,
    /// Instances in resource/declaration order.
    pub instances: Vec<PlanInstance>,
    /// Tasks in resource/declaration order.
    pub tasks: Vec<PlanTask>,
    /// Unique program types used by instances.
    pub programs: Vec<DefId>,
    /// Configuration-level `VAR_CONFIG` entries.
    pub config_inits: Vec<stc_hir::ConfigInitHir>,
}

/// One persistent leaf of a program type.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct ProgramLeaf {
    /// Path segments relative to the program type.
    pub segments: Vec<RootSegment>,
    /// Leaf type.
    pub ty: TyId,
    /// Canonical type key.
    pub key: String,
    /// Leaf size/alignment.
    pub layout: StorageLayout,
    /// Leaf layout hash.
    pub layout_hash: stc_got::GotId,
    /// Effective retention policy.
    pub retain: RetainPolicy,
    /// Direct `AT` address for a top-level direct field.
    pub at: Option<stc_hir::DirectAddress>,
    /// Class definition when the leaf is a class handle value.
    pub class_handle: Option<DefId>,
}

/// One persistent leaf of a class/FB object type.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct ObjectLeaf {
    /// Path segments relative to the object type.
    pub segments: Vec<RootSegment>,
    /// Leaf type.
    pub ty: TyId,
    /// Canonical type key.
    pub key: String,
    /// Leaf size/alignment.
    pub layout: StorageLayout,
    /// Leaf layout hash.
    pub layout_hash: stc_got::GotId,
    /// Class definition when the leaf is a class handle value.
    pub class_handle: Option<DefId>,
}

/// Persistent leaf layout for one class/FB type.
#[derive(Clone, Debug, Default)]
pub(crate) struct ObjectLayout {
    /// Leaves in canonical order (base fields first).
    pub leaves: Vec<ObjectLeaf>,
}

/// An LLVM struct layout for one program type.
#[derive(Clone, Debug)]
pub(crate) struct ProgramLayout {
    /// Persistent leaves in canonical order.
    pub leaves: Vec<ProgramLeaf>,
}

/// Which declaration owns a process-image binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IoRootTarget {
    /// A global `AT` variable.
    Global(DefId),
    /// A program-instance `AT` field.
    InstanceField {
        /// Configured instance.
        instance: Symbol,
        /// Field declaration.
        field: FieldId,
    },
}

/// One internal process-image binding.
#[derive(Clone, Debug)]
pub(crate) struct IoBinding {
    /// Variable path.
    pub variable: String,
    /// Address spelling without `%`.
    pub address: String,
    /// Area.
    pub area: DirectArea,
    /// Area-relative start bit.
    pub start_bit: u64,
    /// Scalar kind.
    pub kind: ScalarKind,
    /// Hint.
    pub hint: Option<IoHint>,
    /// Source.
    pub source: &'static str,
    /// Initializer expression, when the declared leaf has one.
    pub init: Option<ExprId>,
    /// Owning declaration, when the binding comes from an `AT` declaration.
    pub target: Option<IoRootTarget>,
}

/// Process-image layout.
#[derive(Clone, Debug, Default)]
pub(crate) struct ImageLayout {
    /// Input image bytes.
    pub inputs_bytes: u64,
    /// Output image bytes.
    pub outputs_bytes: u64,
    /// Marker image bytes.
    pub markers_bytes: u64,
    /// Input defaults.
    pub input_defaults: Vec<u8>,
    /// Output defaults.
    pub output_defaults: Vec<u8>,
    /// Bindings sorted by area, offset, and variable.
    pub bindings: Vec<IoBinding>,
}

/// How a retained leaf is reached in generated code.
#[derive(Clone, Debug)]
pub(crate) enum RetainPlace {
    /// A global whose whole value is one root.
    Global(DefId),
    /// One nested root of an aggregate global.
    GlobalLeaf {
        /// Global declaration.
        def: DefId,
        /// Path below the global.
        segments: Vec<RootSegment>,
    },
    /// A program-instance field whose whole value is one root.
    InstanceField {
        /// Instance name.
        instance: Symbol,
        /// Field.
        field: FieldId,
    },
    /// One nested root of a program-instance field.
    InstanceLeaf {
        /// Instance name.
        instance: Symbol,
        /// Path below the program type.
        segments: Vec<RootSegment>,
    },
}

/// One retained persistent leaf.
#[derive(Clone, Debug)]
pub(crate) struct RetainLeaf {
    /// Readable path.
    pub path: String,
    /// Canonical type spelling.
    pub ty: String,
    /// LLVM storage type of the leaf.
    pub storage: IrType,
    /// Payload offset.
    pub offset: u64,
    /// Leaf size.
    pub size: u64,
    /// Storage location.
    pub place: RetainPlace,
}

/// Retained-state layout.
#[derive(Clone, Debug, Default)]
pub(crate) struct RetainLayout {
    /// Total payload size.
    pub size: u64,
    /// FNV-1a layout hash.
    pub hash: u64,
    /// Leaves sorted by uppercase path.
    pub leaves: Vec<RetainLeaf>,
}

fn synthetic(hir: &HirCrate, def: DefId) -> bool {
    let span = hir.defs[def.index()].span;
    span.start == 0 && span.end == 0
}

fn program_defs(hir: &HirCrate) -> Vec<DefId> {
    hir.defs
        .iter()
        .enumerate()
        .filter_map(|(index, def)| {
            let id = DefId::new(u32::try_from(index).ok()?);
            (matches!(def.kind, DefKind::Program { .. }) && !def.test_only && !synthetic(hir, id))
                .then_some(id)
        })
        .collect()
}

fn configuration_defs(hir: &HirCrate) -> Vec<DefId> {
    hir.defs
        .iter()
        .enumerate()
        .filter_map(|(index, def)| {
            matches!(def.kind, DefKind::Configuration { .. })
                .then_some(DefId::new(u32::try_from(index).ok()?))
        })
        .collect()
}

/// Selects the M4 execution plan.
///
/// # Errors
///
/// Returns `STC-E-RUN-001`/`STC-E-RUN-002` for ambiguous or unsupported
/// configurations.
pub(crate) fn select_plan(hir: &HirCrate) -> Result<Plan, Vec<CodegenError>> {
    let configurations = configuration_defs(hir);
    let chosen = if let Some(entry) = hir.entry_configuration {
        Some(entry)
    } else {
        match configurations.as_slice() {
            [] => None,
            [only] => Some(*only),
            _ => {
                let names = configurations
                    .iter()
                    .map(|def| hir.def_name(*def).to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let span = hir.defs[configurations[0].index()].span;
                return Err(vec![CodegenError::unsupported(
                    codes::RUN_PLAN,
                    format!(
                        "multiple configurations are present ({names}); select one in a manifest (M6)"
                    ),
                    span,
                )]);
            }
        }
    };

    let Some(configuration) = chosen else {
        let programs = program_defs(hir);
        return match programs.as_slice() {
            [program] => {
                // There is no interner mutation path in codegen; the synthetic
                // instance name is carried as text and a reserved symbol ID.
                let name = Symbol::new(u32::MAX);
                Ok(Plan {
                    configuration: None,
                    instances: vec![PlanInstance {
                        name,
                        name_text: "_default".into(),
                        program: *program,
                        resource: String::new(),
                        task: None,
                        bindings: Vec::new(),
                        global: mangle::program_instance_symbol("_default"),
                        span: hir.defs[program.index()].span,
                    }],
                    tasks: Vec::new(),
                    programs: vec![*program],
                    config_inits: Vec::new(),
                })
            }
            _ => {
                let span = programs
                    .first()
                    .map(|program| hir.defs[program.index()].span)
                    .unwrap_or_else(|| Span::empty(stc_span::FileId::new(0), 0));
                let names = programs
                    .iter()
                    .map(|program| hir.def_name(*program).to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(vec![CodegenError::unsupported(
                    codes::RUN_PLAN,
                    if names.is_empty() {
                        "no PROGRAM or CONFIGURATION is available to run".into()
                    } else {
                        format!(
                            "multiple programs are present ({names}) and no configuration selects one"
                        )
                    },
                    span,
                )])
            }
        };
    };

    let config = match &hir.defs[configuration.index()].kind {
        DefKind::Configuration { config } => config.as_ref().clone(),
        _ => {
            return Err(vec![CodegenError::internal(
                "entry configuration is not a configuration definition",
            )]);
        }
    };
    let configuration_name = Some(hir.def_name(configuration).to_string());
    let mut errors = Vec::new();
    let mut instances = Vec::new();
    let mut tasks = Vec::new();
    let mut programs = BTreeSet::new();
    for resource in &config.resources {
        let resource_name = hir.interner.text(resource.name).to_string();
        if resource.tasks.len() > 1 {
            let span = resource.tasks[1].span;
            errors.push(CodegenError::unsupported(
                codes::RUN_TASK,
                "M4 executes at most one task per resource; task scheduling is M5".into(),
                span,
            ));
        }
        for task in &resource.tasks {
            tasks.push(plan_task(hir, &resource_name, task));
        }
        for program in &resource.programs {
            if !matches!(
                hir.defs[program.program.index()].kind,
                DefKind::Program { .. }
            ) {
                errors.push(CodegenError::unsupported(
                    codes::RUN_PLAN,
                    format!(
                        "`{}` does not name a PROGRAM",
                        hir.def_name(program.program)
                    ),
                    program.span,
                ));
                continue;
            }
            programs.insert(program.program);
            let name_text = hir.interner.text(program.name).to_string();
            let global = mangle::program_instance_symbol(&name_text);
            instances.push(PlanInstance {
                name: program.name,
                name_text,
                program: program.program,
                resource: resource_name.clone(),
                task: program.task,
                bindings: program.bindings.clone(),
                global,
                span: program.span,
            });
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Plan {
        configuration: configuration_name,
        instances,
        tasks,
        programs: programs.into_iter().collect(),
        config_inits: config.config_inits.clone(),
    })
}

fn plan_task(hir: &HirCrate, resource: &str, task: &TaskHir) -> PlanTask {
    PlanTask {
        name: task.name,
        name_text: hir.interner.text(task.name).to_string(),
        resource: resource.into(),
        single: task.single_value,
        interval_ns: task.interval_ns,
        priority: task.priority_value,
        span: task.span,
    }
}

/// Builds struct layouts for every selected program type.
///
/// # Errors
///
/// Reports non-scalar program fields and `VAR_CONFIG AT` overrides.
pub(crate) fn program_layouts(
    hir: &HirCrate,
    plan: &Plan,
    types: &TypeSystem<'_>,
) -> Result<BTreeMap<DefId, ProgramLayout>, Vec<CodegenError>> {
    let mut layouts = BTreeMap::new();
    let mut errors = Vec::new();
    for program in &plan.programs {
        let DefKind::Program { fields, .. } = &hir.defs[program.index()].kind else {
            continue;
        };
        let mut leaves = Vec::new();
        for field in fields {
            let data = hir.field(*field);
            match types.storage_plan(data.ty, data.retain) {
                Ok(plan) => {
                    for mut leaf in plan.leaves {
                        leaf.segments.insert(0, RootSegment::Field(*field));
                        let at = if data.at.is_some() && leaf.segments.len() == 1 {
                            data.at.clone()
                        } else {
                            None
                        };
                        leaves.push(ProgramLeaf {
                            segments: leaf.segments,
                            ty: leaf.ty,
                            key: leaf.key,
                            layout: leaf.layout,
                            layout_hash: leaf.layout_hash,
                            retain: leaf.retain,
                            at,
                            class_handle: leaf.class_handle,
                        });
                    }
                }
                Err(error) => errors.push(error.with_span(data.span)),
            }
        }
        layouts.insert(*program, ProgramLayout { leaves });
    }
    for init in &plan.config_inits {
        if let Some(address) = &init.at {
            errors.push(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "`VAR_CONFIG` AT overrides are not supported in M4".into(),
                address.span,
            ));
        }
    }
    if errors.is_empty() {
        Ok(layouts)
    } else {
        errors.sort_by_key(|error| error.span());
        errors.dedup();
        Err(errors)
    }
}

/// Builds the persistent leaf layout of every class/FB type.
///
/// # Errors
///
/// Returns unsupported-construct errors for malformed object layouts.
pub(crate) fn object_layouts(
    hir: &HirCrate,
    types: &TypeSystem<'_>,
) -> Result<BTreeMap<DefId, ObjectLayout>, Vec<CodegenError>> {
    let mut layouts = BTreeMap::new();
    let mut errors = Vec::new();
    for (index, def) in hir.defs.iter().enumerate() {
        let id = DefId::new(u32::try_from(index).unwrap_or(u32::MAX));
        if def.test_only {
            continue;
        }
        if !matches!(
            def.kind,
            DefKind::FunctionBlock { .. } | DefKind::Class { .. }
        ) {
            continue;
        }
        if types.named_ty(id).is_err() {
            // Synthetic prelude types that no source references have no
            // interned value type and therefore no object layout.
            continue;
        }
        match types.instance_leaves_for_def(id, RetainPolicy::None) {
            Ok(leaves) => {
                layouts.insert(
                    id,
                    ObjectLayout {
                        leaves: leaves
                            .into_iter()
                            .map(|leaf| ObjectLeaf {
                                segments: leaf.segments,
                                ty: leaf.ty,
                                key: leaf.key,
                                layout: leaf.layout,
                                layout_hash: leaf.layout_hash,
                                class_handle: leaf.class_handle,
                            })
                            .collect(),
                    },
                );
            }
            Err(error) => errors.push(error.with_span(def.span)),
        }
    }
    if errors.is_empty() {
        Ok(layouts)
    } else {
        errors.sort_by_key(|error| error.span());
        errors.dedup();
        Err(errors)
    }
}

/// Builds the process-image layout and defaults.
///
/// # Errors
///
/// Reports wildcard/area-less `AT` addresses and non-scalar leaves.
pub(crate) fn image_layout(
    hir: &HirCrate,
    plan: &Plan,
    _layouts: &BTreeMap<DefId, ProgramLayout>,
) -> Result<ImageLayout, Vec<CodegenError>> {
    let mut bindings = Vec::new();
    let mut errors = Vec::new();
    let mut overrides: BTreeMap<(Symbol, FieldId), ExprId> = BTreeMap::new();
    for init in &plan.config_inits {
        if init.fields.len() == 1
            && let Some(value) = init.init
        {
            overrides.insert((init.instance, init.fields[0]), value);
        }
    }

    for (index, def) in hir.defs.iter().enumerate() {
        let DefKind::Global {
            ty,
            at: Some(address),
            io,
            init,
            ..
        } = &def.kind
        else {
            continue;
        };
        match layout::scalar_kind_at(hir, *ty, address.span) {
            Ok(kind) => match layout::image_span(address, kind) {
                Ok(span) => bindings.push(IoBinding {
                    variable: def.name_text(&hir.interner).to_string(),
                    address: layout::render_address(address),
                    area: span.area,
                    start_bit: span.start_bit,
                    kind,
                    hint: io.clone(),
                    source: "pragma",
                    init: *init,
                    target: Some(IoRootTarget::Global(DefId::new(
                        u32::try_from(index).unwrap_or(u32::MAX),
                    ))),
                }),
                Err(error) => errors.push(error),
            },
            Err(error) => errors.push(error),
        }
    }

    for instance in &plan.instances {
        let DefKind::Program { fields, .. } = &hir.defs[instance.program.index()].kind else {
            continue;
        };
        for field in fields {
            let data = hir.field(*field);
            let Some(address) = &data.at else {
                continue;
            };
            match layout::scalar_kind_at(hir, data.ty, data.span) {
                Ok(kind) => match layout::image_span(address, kind) {
                    Ok(span) => bindings.push(IoBinding {
                        variable: format!(
                            "{}.{}",
                            instance.name_text,
                            hir.interner.text(data.name)
                        ),
                        address: layout::render_address(address),
                        area: span.area,
                        start_bit: span.start_bit,
                        kind,
                        hint: data.io.clone(),
                        source: "pragma",
                        init: overrides
                            .get(&(instance.name, *field))
                            .copied()
                            .or(data.init),
                        target: Some(IoRootTarget::InstanceField {
                            instance: instance.name,
                            field: *field,
                        }),
                    }),
                    Err(error) => errors.push(error),
                },
                Err(error) => errors.push(error),
            }
        }
    }

    let mut max = [0_u64; 3];
    let mut grow = |area: DirectArea, start_bit: u64, width_bits: u64| {
        let slot = match area {
            DirectArea::Input => 0,
            DirectArea::Output => 1,
            DirectArea::Memory => 2,
        };
        max[slot] = max[slot].max(start_bit + width_bits);
    };
    for binding in &bindings {
        grow(
            binding.area,
            binding.start_bit,
            binding.kind.occupancy_bits(),
        );
    }
    // Direct-variable expressions have no declaration; still size the image
    // for them (metadata bindings remain declaration-driven, §5.9).
    for expr in &hir.exprs {
        let ExprKindHir::Place(PlaceHir::Direct(address)) = &expr.kind else {
            continue;
        };
        match layout::scalar_kind_at(hir, expr.ty, expr.span)
            .and_then(|kind| layout::image_span(address, kind))
        {
            Ok(span) => grow(span.area, span.start_bit, span.width_bits),
            Err(error) => errors.push(error),
        }
    }
    drop(grow);
    let inputs_bytes = max[0].div_ceil(8);
    let outputs_bytes = max[1].div_ceil(8);
    let markers_bytes = max[2].div_ceil(8);
    let mut image = ImageLayout {
        inputs_bytes,
        outputs_bytes,
        markers_bytes,
        input_defaults: vec![0; usize::try_from(inputs_bytes).unwrap_or(0)],
        output_defaults: vec![0; usize::try_from(outputs_bytes).unwrap_or(0)],
        bindings,
    };
    let mut ordered = image.bindings.clone();
    ordered.sort_by_key(|binding| (binding.area, binding.start_bit, binding.variable.clone()));
    image.bindings = ordered;
    apply_defaults(hir, &mut image);
    if errors.is_empty() {
        Ok(image)
    } else {
        Err(errors)
    }
}

fn apply_defaults(hir: &HirCrate, image: &mut ImageLayout) {
    for binding in &image.bindings {
        let Some(init) = binding.init else {
            continue;
        };
        let Some(bytes) = const_bytes(hir, init, binding.kind) else {
            continue;
        };
        let target = match binding.area {
            DirectArea::Input => &mut image.input_defaults,
            DirectArea::Output => &mut image.output_defaults,
            DirectArea::Memory => continue,
        };
        let byte = usize::try_from(binding.start_bit / 8).unwrap_or(usize::MAX);
        if binding.kind == ScalarKind::Bool {
            let bit = u8::try_from(binding.start_bit % 8).unwrap_or(0);
            let Some(slot) = target.get_mut(byte) else {
                continue;
            };
            if bytes.first().is_some_and(|value| *value != 0) {
                *slot |= 1 << bit;
            } else {
                *slot &= !(1 << bit);
            }
        } else {
            for (index, value) in bytes.iter().enumerate() {
                if let Some(slot) = target.get_mut(byte + index) {
                    *slot = *value;
                }
            }
        }
    }
}

fn const_bytes(hir: &HirCrate, id: ExprId, kind: ScalarKind) -> Option<Vec<u8>> {
    let value = match &hir.expr(id).kind {
        ExprKindHir::Literal(literal) => &literal.value,
        _ => return None,
    };
    Some(encode_const(value, kind))
}

fn encode_const(value: &ConstValue, kind: ScalarKind) -> Vec<u8> {
    match kind {
        ScalarKind::Bool => vec![u8::from(matches!(value, ConstValue::Bool(true)))],
        ScalarKind::Int { width, .. } | ScalarKind::Char { width } => {
            let number = match value {
                ConstValue::Int(value)
                | ConstValue::Time(value)
                | ConstValue::Enum { value, .. } => *value,
                ConstValue::Char(value) => i128::from(*value),
                ConstValue::Bool(value) => i128::from(u8::from(*value)),
                ConstValue::Date { value, .. } => *value,
                _ => 0,
            };
            let bytes = (width / 8) as usize;
            let number = number as u128;
            (0..bytes)
                .map(|index| ((number >> (index * 8)) & 0xFF) as u8)
                .collect()
        }
        ScalarKind::Real { width: 32 } => {
            let number = match value {
                ConstValue::Real(value) => *value as f32,
                ConstValue::Int(value) => *value as f32,
                _ => 0.0,
            };
            number.to_bits().to_le_bytes().to_vec()
        }
        ScalarKind::Real { .. } => {
            let number = match value {
                ConstValue::Real(value) => *value,
                ConstValue::Int(value) => *value as f64,
                _ => 0.0,
            };
            number.to_bits().to_le_bytes().to_vec()
        }
        ScalarKind::Time | ScalarKind::Date(_) => {
            let number = match value {
                ConstValue::Time(value) | ConstValue::Date { value, .. } => *value,
                ConstValue::Int(value) => *value,
                _ => 0,
            };
            (number as u64).to_le_bytes().to_vec()
        }
    }
}

/// Builds the retained layout from effective per-leaf retention (D-135).
///
/// Structs, arrays, and class/FB objects recurse through their persistent
/// roots; strings and scalar arrays stay single leaves. References,
/// interfaces, and pointers are rejected by sema before code generation.
///
/// # Errors
///
/// Reports malformed types and unsupported leaf shapes.
pub(crate) fn retain_layout(
    hir: &HirCrate,
    plan: &Plan,
    layouts: &BTreeMap<DefId, ProgramLayout>,
    types: &TypeSystem<'_>,
) -> Result<RetainLayout, Vec<CodegenError>> {
    let mut leaves = Vec::new();
    let mut errors = Vec::new();
    for (index, def) in hir.defs.iter().enumerate() {
        let DefKind::Global { ty, retain, at, .. } = &def.kind else {
            continue;
        };
        if at.is_some() {
            continue;
        }
        let id = DefId::new(u32::try_from(index).unwrap_or(0));
        let prefix = def.name_text(&hir.interner).to_string();
        let declared = match types.storage_plan(*ty, *retain) {
            Ok(plan) => plan.leaves,
            Err(error) => {
                errors.push(error.with_span(def.span));
                continue;
            }
        };
        for leaf in declared {
            if leaf.retain != RetainPolicy::Retain || leaf.is_class_handle() {
                continue;
            }
            let path = match retain_path(hir, types, *ty, &prefix, &leaf.segments) {
                Ok(path) => path,
                Err(error) => {
                    errors.push(error.with_span(def.span));
                    continue;
                }
            };
            let storage = match types.llvm_storage(leaf.ty) {
                Ok(storage) => storage,
                Err(error) => {
                    errors.push(error.with_span(def.span));
                    continue;
                }
            };
            let place = if leaf.segments.is_empty() {
                RetainPlace::Global(id)
            } else {
                RetainPlace::GlobalLeaf {
                    def: id,
                    segments: leaf.segments,
                }
            };
            leaves.push(RetainLeaf {
                path,
                ty: hir.type_name(leaf.ty),
                storage,
                offset: 0,
                size: leaf.layout.size,
                place,
            });
        }
    }

    for instance in &plan.instances {
        let Some(layout) = layouts.get(&instance.program) else {
            continue;
        };
        let program_ty = match types.named_ty(instance.program) {
            Ok(ty) => ty,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        for leaf in &layout.leaves {
            if leaf.retain != RetainPolicy::Retain
                || leaf.at.is_some()
                || leaf.class_handle.is_some()
            {
                continue;
            }
            let path =
                match retain_path(hir, types, program_ty, &instance.name_text, &leaf.segments) {
                    Ok(path) => path,
                    Err(error) => {
                        errors.push(error);
                        continue;
                    }
                };
            let storage = match types.llvm_storage(leaf.ty) {
                Ok(storage) => storage,
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            };
            let place = match leaf.segments.as_slice() {
                [RootSegment::Field(field)] => RetainPlace::InstanceField {
                    instance: instance.name,
                    field: *field,
                },
                _ => RetainPlace::InstanceLeaf {
                    instance: instance.name,
                    segments: leaf.segments.clone(),
                },
            };
            leaves.push(RetainLeaf {
                path,
                ty: hir.type_name(leaf.ty),
                storage,
                offset: 0,
                size: leaf.layout.size,
                place,
            });
        }
    }

    if !errors.is_empty() {
        errors.sort_by_key(|error| error.span());
        errors.dedup();
        return Err(errors);
    }
    leaves.sort_by(|left, right| {
        let left_key = left.path.to_ascii_uppercase();
        let right_key = right.path.to_ascii_uppercase();
        left_key
            .cmp(&right_key)
            .then_with(|| left.path.cmp(&right.path))
    });
    let mut offset = 0;
    for leaf in &mut leaves {
        leaf.offset = offset;
        offset += leaf.size;
    }
    let mut hash = layout::FNV_OFFSET;
    for (index, leaf) in leaves.iter().enumerate() {
        if index > 0 {
            hash = layout::fnv1a_update(hash, b"\n");
        }
        let line = format!("{}|{}|{}|{}", leaf.path, leaf.ty, leaf.offset, leaf.size);
        hash = layout::fnv1a_update(hash, line.as_bytes());
    }
    Ok(RetainLayout {
        size: offset,
        hash,
        leaves,
    })
}

/// Formats a retained path with source field names and source array indices.
fn retain_path(
    hir: &HirCrate,
    types: &TypeSystem<'_>,
    ty: TyId,
    prefix: &str,
    segments: &[RootSegment],
) -> Result<String, CodegenError> {
    let mut out = String::from(prefix);
    let mut current = ty;
    for segment in segments {
        match segment {
            RootSegment::Field(field) => {
                let data = hir.field(*field);
                out.push('.');
                out.push_str(hir.interner.text(data.name));
                current = data.ty;
            }
            RootSegment::Element { index, dim } => {
                let facts = types.facts(current)?;
                let TypeCategory::Array { dims, element, .. } = &facts.category else {
                    return Err(CodegenError::internal(
                        "retained leaf has an array index outside an array",
                    ));
                };
                let Some(dimension) = dims.get(usize::try_from(*dim).unwrap_or(usize::MAX)) else {
                    return Err(CodegenError::internal(
                        "retained leaf has an out-of-range array dimension",
                    ));
                };
                out.push('[');
                out.push_str(&(dimension.lower + i128::from(*index)).to_string());
                out.push(']');
                if usize::try_from(*dim).unwrap_or(usize::MAX) + 1 == dims.len() {
                    current = *element;
                }
            }
        }
    }
    Ok(out)
}
