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
use crate::metadata::{
    ChannelMetadata, DeviceMetadata, IoRoute, IoRouteEndpoint, ProjectBinding, ProjectChannel,
    RouteEndpointMetadata, RouteMetadata,
};
use crate::module::ModuleError;
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
    /// Resource declaration index.
    pub resource_index: usize,
    /// Task declaration index within the resource.
    pub task_index: usize,
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

/// One `VAR_ACCESS` declaration selected for metadata.
#[derive(Clone, Debug)]
pub(crate) struct PlanVarAccess {
    /// Access name.
    pub name: String,
    /// Resolved place.
    pub place: PlaceHir,
    /// Declared type.
    pub ty: TyId,
    /// `READ_ONLY` marker.
    pub read_only: bool,
    /// Declaration span.
    pub span: Span,
    /// Instance spelling for a program-level `VAR_ACCESS`.
    pub instance_name: Option<String>,
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
    /// Configuration-, resource-, and program-level `VAR_ACCESS` entries.
    pub var_access: Vec<PlanVarAccess>,
}

impl Plan {
    /// Returns instance indices in generated-cycle order (D-154, Runtime §1.3).
    ///
    /// Task-less instances run first in declaration order. Task-bound instances
    /// follow in scheduler rank order: explicit priorities ascending, then
    /// tasks without a priority, ties by resource declaration order then task
    /// declaration order, and program declaration order within a task.
    pub(crate) fn cycle_instances(&self) -> Vec<usize> {
        let mut order = Vec::with_capacity(self.instances.len());
        let mut seen = vec![false; self.instances.len()];
        for (index, instance) in self.instances.iter().enumerate() {
            if instance.task.is_none() {
                order.push(index);
                seen[index] = true;
            }
        }
        let mut tasks: Vec<&PlanTask> = self.tasks.iter().collect();
        tasks.sort_by_key(|task| {
            (
                task.priority.is_none(),
                task.priority.unwrap_or(0),
                task.resource_index,
                task.task_index,
            )
        });
        for task in tasks {
            for (index, instance) in self.instances.iter().enumerate() {
                if !seen[index]
                    && instance.task == Some(task.name)
                    && instance.resource == task.resource
                {
                    order.push(index);
                    seen[index] = true;
                }
            }
        }
        // A task-bound instance always names a declared task; keep any
        // unmatched instance reachable rather than silently dropping it.
        for (index, visited) in seen.into_iter().enumerate() {
            if !visited {
                order.push(index);
            }
        }
        order
    }
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
    /// `{stc:io}` hint.
    pub hint: Option<IoHint>,
    /// Effective device name.
    pub device: Option<String>,
    /// Effective channel.
    pub channel: Option<ProjectChannel>,
    /// Effective direction.
    pub direction: Option<&'static str>,
    /// Source.
    pub source: &'static str,
    /// Initializer expression, when the declared leaf has one.
    pub init: Option<ExprId>,
    /// Owning declaration, when the binding comes from an `AT` declaration.
    pub target: Option<IoRootTarget>,
}

impl IoBinding {
    /// Returns the effective binding fields implied by a `{stc:io}` hint.
    fn apply_hint(&mut self) {
        let Some(hint) = &self.hint else {
            return;
        };
        self.device.clone_from(&hint.device);
        self.channel = hint.channel.map(ProjectChannel::Index);
        self.direction = hint.direction.map(stc_hir::IoDirection::as_str);
    }
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
            (matches!(def.kind, DefKind::Program { .. })
                && !def.test_only
                && !synthetic(hir, id)
                && def.package == hir.root_package)
                .then_some(id)
        })
        .collect()
}

fn configuration_defs(hir: &HirCrate) -> Vec<DefId> {
    hir.defs
        .iter()
        .enumerate()
        .filter_map(|(index, def)| {
            (matches!(def.kind, DefKind::Configuration { .. }) && def.package == hir.root_package)
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
                let var_access = program_var_access(hir, *program, Some((name, "_default")));
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
                    var_access,
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
    for (resource_index, resource) in config.resources.iter().enumerate() {
        let resource_name = hir.interner.text(resource.name).to_string();
        for (task_index, task) in resource.tasks.iter().enumerate() {
            tasks.push(plan_task(
                hir,
                &resource_name,
                resource_index,
                task_index,
                task,
            ));
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
    let mut var_access = Vec::new();
    for access in &config.var_access {
        var_access.extend(plan_var_access(hir, access, None));
    }
    for resource in &config.resources {
        for access in &resource.var_access {
            var_access.extend(plan_var_access(hir, access, None));
        }
    }
    for instance in &instances {
        for access in program_var_access(
            hir,
            instance.program,
            Some((instance.name, &instance.name_text)),
        ) {
            var_access.push(access);
        }
    }
    Ok(Plan {
        configuration: configuration_name,
        instances,
        tasks,
        programs: programs.into_iter().collect(),
        config_inits: config.config_inits.clone(),
        var_access,
    })
}

/// Returns a program type's `VAR_ACCESS` entries with their instance context.
fn program_var_access(
    hir: &HirCrate,
    program: DefId,
    instance: Option<(Symbol, &str)>,
) -> Vec<PlanVarAccess> {
    let DefKind::Program { var_access, .. } = &hir.defs[program.index()].kind else {
        return Vec::new();
    };
    var_access
        .iter()
        .filter_map(|access| plan_var_access(hir, access, instance))
        .collect()
}

fn plan_var_access(
    hir: &HirCrate,
    access: &stc_hir::VarAccessHir,
    instance: Option<(Symbol, &str)>,
) -> Option<PlanVarAccess> {
    let ExprKindHir::Place(place) = &hir.expr(access.place).kind else {
        return None;
    };
    Some(PlanVarAccess {
        name: hir.interner.text(access.name).to_string(),
        place: place.clone(),
        ty: access.ty,
        read_only: access.read_only,
        span: access.span,
        instance_name: instance.map(|(_, name)| name.to_owned()),
    })
}

fn plan_task(
    hir: &HirCrate,
    resource: &str,
    resource_index: usize,
    task_index: usize,
    task: &TaskHir,
) -> PlanTask {
    PlanTask {
        name: task.name,
        name_text: hir.interner.text(task.name).to_string(),
        resource: resource.into(),
        resource_index,
        task_index,
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
    test_build: bool,
) -> Result<BTreeMap<DefId, ObjectLayout>, Vec<CodegenError>> {
    let mut layouts = BTreeMap::new();
    let mut errors = Vec::new();
    for (index, def) in hir.defs.iter().enumerate() {
        let id = DefId::new(u32::try_from(index).unwrap_or(u32::MAX));
        if def.test_only && !test_build {
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
                Ok(span) => {
                    let mut binding = IoBinding {
                        variable: def.name_text(&hir.interner).to_string(),
                        address: layout::render_address(address),
                        area: span.area,
                        start_bit: span.start_bit,
                        kind,
                        hint: io.clone(),
                        device: None,
                        channel: None,
                        direction: None,
                        source: "pragma",
                        init: *init,
                        target: Some(IoRootTarget::Global(DefId::new(
                            u32::try_from(index).unwrap_or(u32::MAX),
                        ))),
                    };
                    binding.apply_hint();
                    bindings.push(binding);
                }
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
                    Ok(span) => {
                        let mut binding = IoBinding {
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
                            device: None,
                            channel: None,
                            direction: None,
                            source: "pragma",
                            init: overrides
                                .get(&(instance.name, *field))
                                .copied()
                                .or(data.init),
                            target: Some(IoRootTarget::InstanceField {
                                instance: instance.name,
                                field: *field,
                            }),
                        };
                        binding.apply_hint();
                        bindings.push(binding);
                    }
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

/// Result of resolving the machine-profile route graph.
#[derive(Default)]
pub(crate) struct IoRouteResult {
    /// Resolved routes in profile order.
    pub routes: Vec<RouteMetadata>,
    /// Route resolution errors (`STC-E-IO-BINDING-001`/`002`).
    pub errors: Vec<CodegenError>,
    /// Non-fatal diagnostics.
    pub warnings: Vec<ModuleError>,
}

/// Resolves profile routes against the program and the device tree
/// (IO-Routing-and-Backends §3).
///
/// The profile parser has already checked endpoint spelling and source/sink
/// pairing. This step resolves PLC endpoints against the image layout,
/// resolves physical endpoints against the declared device/channel tree,
/// checks width equality, and rejects fan-in and sink conflicts across routes
/// and project/pragma bindings.
pub(crate) fn resolve_routes(
    hir: &HirCrate,
    image: &ImageLayout,
    devices: &[DeviceMetadata],
    routes: &[IoRoute],
) -> IoRouteResult {
    let mut result = IoRouteResult::default();
    if routes.is_empty() {
        // A project without a selected profile keeps the v1 binding behavior
        // unchanged; there is no route graph to validate.
        return result;
    }
    // Sink claims are one namespace across project bindings and routes
    // (IO-Routing-and-Backends §3.3). Only a binding with a device/channel is
    // a single-sink shorthand; a raw `AT` variable on its own claims nothing.
    let empty = Span::empty(stc_span::FileId::new(0), 0);
    let mut claims: BTreeMap<String, String> = BTreeMap::new();
    for binding in &image.bindings {
        let Some(device_name) = &binding.device else {
            continue;
        };
        let Some(channel_ref) = binding.channel.as_ref() else {
            continue;
        };
        let description = format!("binding `{}`", binding.variable);
        let key = match binding.area {
            DirectArea::Input => format!("plc-input:{}", binding.start_bit),
            DirectArea::Output => {
                let Some(device) = devices.iter().find(|device| device.name == *device_name) else {
                    // An undeclared pragma device is already a warning, and the
                    // runtime may add it; it claims no known physical sink.
                    continue;
                };
                let Ok(channel) = resolve_channel(device, channel_ref) else {
                    // Existing binding validation owns this error.
                    continue;
                };
                format!("physical-output:{}.{}", device.name, channel.name)
            }
            DirectArea::Memory => continue,
        };
        if let Some(previous) = claims.get(&key) {
            result.errors.push(CodegenError::unsupported(
                codes::IO_BINDING_CONFLICT,
                format!("{description} and {previous} claim the same sink"),
                empty,
            ));
        } else {
            claims.insert(key, description);
        }
    }

    for route in routes {
        let Some(source) = resolve_endpoint(hir, image, devices, &route.source, true, &mut result)
        else {
            continue;
        };
        let mut sinks = Vec::with_capacity(route.sinks.len());
        for sink in &route.sinks {
            let Some(resolved) = resolve_endpoint(hir, image, devices, sink, false, &mut result)
            else {
                continue;
            };
            if source.width_bits != resolved.width_bits {
                result.errors.push(CodegenError::unsupported(
                    codes::IO_BINDING_INVALID,
                    format!(
                        "route source is {} bits wide but sink `{}` is {} bits wide",
                        source.width_bits,
                        describe_endpoint(&resolved),
                        resolved.width_bits
                    ),
                    sink.span(),
                ));
                continue;
            }
            let key = sink_claim(&resolved);
            if let Some(previous) = claims.get(&key) {
                result.push_route_error(
                    codes::IO_BINDING_CONFLICT,
                    format!(
                        "route sink `{}` is fan-in: it is already written by {previous}",
                        describe_endpoint(&resolved)
                    ),
                    sink.span(),
                );
                continue;
            }
            claims.insert(
                key,
                format!("route source `{}`", describe_endpoint(&source)),
            );
            sinks.push(resolved);
        }
        result.routes.push(RouteMetadata { source, sinks });
    }
    result
}

impl IoRouteResult {
    fn push_route_error(&mut self, code: &'static str, message: String, span: Span) {
        self.errors
            .push(CodegenError::unsupported(code, message, span));
    }
}

fn resolve_endpoint(
    hir: &HirCrate,
    image: &ImageLayout,
    devices: &[DeviceMetadata],
    endpoint: &IoRouteEndpoint,
    source: bool,
    result: &mut IoRouteResult,
) -> Option<RouteEndpointMetadata> {
    match endpoint {
        IoRouteEndpoint::PlcInput { variable, span } => {
            if source {
                result.push_route_error(
                    codes::IO_BINDING_INVALID,
                    "a `plc-input` endpoint cannot be a route source".into(),
                    *span,
                );
                return None;
            }
            let Some(binding) = resolve_plc_variable(hir, image, variable, DirectArea::Input)
            else {
                result.push_route_error(
                    codes::IO_BINDING_INVALID,
                    format!("route endpoint `{variable}` does not name a direct `%I` variable"),
                    *span,
                );
                return None;
            };
            Some(plc_endpoint("plc-input", binding))
        }
        IoRouteEndpoint::PlcOutput { variable, span } => {
            if !source {
                result.push_route_error(
                    codes::IO_BINDING_INVALID,
                    "a `plc-output` endpoint cannot be a route sink".into(),
                    *span,
                );
                return None;
            }
            let Some(binding) = resolve_plc_variable(hir, image, variable, DirectArea::Output)
            else {
                result.push_route_error(
                    codes::IO_BINDING_INVALID,
                    format!("route endpoint `{variable}` does not name a direct `%Q` variable"),
                    *span,
                );
                return None;
            };
            Some(plc_endpoint("plc-output", binding))
        }
        IoRouteEndpoint::PhysicalInput {
            device,
            channel,
            span,
        } => resolve_physical_endpoint(
            devices,
            device,
            channel,
            "input",
            "physical-input",
            *span,
            result,
        ),
        IoRouteEndpoint::PhysicalOutput {
            device,
            channel,
            span,
        } => resolve_physical_endpoint(
            devices,
            device,
            channel,
            "output",
            "physical-output",
            *span,
            result,
        ),
    }
}

fn resolve_plc_variable<'a>(
    hir: &HirCrate,
    image: &'a ImageLayout,
    variable: &str,
    area: DirectArea,
) -> Option<&'a IoBinding> {
    // A PLC route endpoint must name a direct variable. The lookup is total;
    // `resolve_endpoint` reports the failure at the profile span.
    image.bindings.iter().find(|binding| {
        binding.area == area
            && binding_names(hir, binding)
                .iter()
                .any(|name| name.eq_ignore_ascii_case(variable))
    })
}

fn plc_endpoint(kind: &'static str, binding: &IoBinding) -> RouteEndpointMetadata {
    RouteEndpointMetadata {
        kind,
        device: None,
        channel: None,
        variable: Some(binding.variable.clone()),
        area: Some(binding.area.as_char()),
        offset_bits: Some(binding.start_bit),
        width_bits: binding.kind.occupancy_bits(),
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_physical_endpoint(
    devices: &[DeviceMetadata],
    device_name: &str,
    channel_name: &str,
    expected: &str,
    kind: &'static str,
    span: Span,
    result: &mut IoRouteResult,
) -> Option<RouteEndpointMetadata> {
    let Some(device) = devices.iter().find(|device| device.name == device_name) else {
        result.push_route_error(
            codes::IO_BINDING_INVALID,
            format!("route endpoint names undeclared device `{device_name}`"),
            span,
        );
        return None;
    };
    let Some(channel) = device
        .channels
        .iter()
        .find(|channel| channel.name == channel_name)
    else {
        result.push_route_error(
            codes::IO_BINDING_INVALID,
            format!(
                "route endpoint names undeclared channel `{channel_name}` on device `{device_name}`"
            ),
            span,
        );
        return None;
    };
    if channel.direction != expected && channel.direction != "inout" {
        result.push_route_error(
            codes::IO_BINDING_INVALID,
            format!(
                "route endpoint channel `{channel_name}` on device `{device_name}` has direction \
                 `{}`, expected `{expected}`",
                channel.direction
            ),
            span,
        );
        return None;
    }
    if channel.width == 0 {
        result.push_route_error(
            codes::IO_BINDING_INVALID,
            format!(
                "route endpoint channel `{channel_name}` on device `{device_name}` has zero width"
            ),
            span,
        );
        return None;
    }
    Some(RouteEndpointMetadata {
        kind,
        device: Some(device.name.clone()),
        channel: Some(channel.name.clone()),
        variable: None,
        area: None,
        offset_bits: None,
        width_bits: u64::from(channel.width),
    })
}

fn resolve_channel<'a>(
    device: &'a DeviceMetadata,
    channel: &ProjectChannel,
) -> Result<&'a ChannelMetadata, String> {
    match channel {
        ProjectChannel::Name(name) => device
            .channels
            .iter()
            .find(|channel| channel.name == *name)
            .ok_or_else(|| {
                format!(
                    "I/O binding names undeclared channel `{name}` on device `{}`",
                    device.name
                )
            }),
        ProjectChannel::Index(index) => usize::try_from(*index)
            .ok()
            .and_then(|index| device.channels.get(index))
            .ok_or_else(|| {
                format!(
                    "I/O binding names channel {index}, which device `{}` does not declare",
                    device.name
                )
            }),
    }
}

/// Returns the sink identity for one resolved route endpoint.
fn sink_claim(endpoint: &RouteEndpointMetadata) -> String {
    match endpoint.kind {
        "physical-output" => format!(
            "physical-output:{}.{}",
            endpoint.device.as_deref().unwrap_or_default(),
            endpoint.channel.as_deref().unwrap_or_default()
        ),
        "plc-input" => format!("plc-input:{}", endpoint.offset_bits.unwrap_or_default()),
        _ => describe_endpoint(endpoint),
    }
}

fn describe_endpoint(endpoint: &RouteEndpointMetadata) -> String {
    match endpoint.kind {
        "physical-input" | "physical-output" => format!(
            "{} {}.{}",
            endpoint.kind,
            endpoint.device.as_deref().unwrap_or_default(),
            endpoint.channel.as_deref().unwrap_or_default()
        ),
        _ => format!(
            "{} {}",
            endpoint.kind,
            endpoint.variable.as_deref().unwrap_or_default()
        ),
    }
}

/// Result of resolving project I/O bindings and validating pragma hints.
#[derive(Default)]
pub(crate) struct IoProjectResult {
    /// Non-fatal diagnostics (`STC-W-IO-001`, `STC-W-IO-003`).
    pub warnings: Vec<ModuleError>,
    /// Binding errors (`STC-E-PROJECT-009`).
    pub errors: Vec<CodegenError>,
}

/// Applies `[[io.bindings]]` over pragma hints and validates declared devices.
///
/// Precedence is runtime override (not compiler metadata) > project binding >
/// `{stc:io}` pragma > raw address (IO §4). A project binding rewrites the
/// binding's effective device/channel/direction and its metadata `source`.
pub(crate) fn apply_project_bindings(
    hir: &HirCrate,
    image: &mut ImageLayout,
    devices: &[DeviceMetadata],
    bindings: &[ProjectBinding],
) -> IoProjectResult {
    let mut result = IoProjectResult::default();

    // A pragma naming an undeclared device stays a warning: the runtime may
    // add devices (IO §4).
    for binding in &image.bindings {
        let Some(hint) = &binding.hint else {
            continue;
        };
        let Some(device) = &hint.device else {
            continue;
        };
        if !devices.iter().any(|declared| declared.name == *device) {
            result.warnings.push(ModuleError {
                code: codes::W_IO_UNKNOWN_DEVICE,
                severity: crate::module::Severity::Warning,
                message: format!(
                    "`stc:io` on `{}` names device `{device}`, which is not declared in stc.toml",
                    binding.variable
                ),
                span: Some(hint.span),
                got_id: None,
                old_key: None,
                new_key: None,
            });
        }
    }

    let span = Span::empty(stc_span::FileId::new(0), 0);
    let mut bound = BTreeSet::new();
    for project in bindings {
        let Some(index) = image.bindings.iter().position(|candidate| {
            binding_names(hir, candidate)
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&project.variable))
        }) else {
            result.errors.push(CodegenError::unsupported(
                codes::PROJECT_IO,
                format!(
                    "I/O binding variable `{}` does not name a declared `AT` variable or field",
                    project.variable
                ),
                span,
            ));
            continue;
        };
        if !bound.insert(index) {
            result.errors.push(CodegenError::unsupported(
                codes::PROJECT_IO,
                format!(
                    "I/O binding variable `{}` is bound more than once (paths are \
                     case-insensitive)",
                    project.variable
                ),
                span,
            ));
            continue;
        }
        let device = devices.iter().find(|device| device.name == project.device);
        let Some(device) = device else {
            result.errors.push(CodegenError::unsupported(
                codes::PROJECT_IO,
                format!(
                    "I/O binding `{}` names undeclared device `{}`",
                    project.variable, project.device
                ),
                span,
            ));
            continue;
        };
        if let Some(ProjectChannel::Name(name)) = &project.channel
            && !device.channels.is_empty()
            && !device.channels.iter().any(|channel| channel.name == *name)
        {
            result.errors.push(CodegenError::unsupported(
                codes::PROJECT_IO,
                format!(
                    "I/O binding `{}` names undeclared channel `{name}` on device `{}`",
                    project.variable, project.device
                ),
                span,
            ));
            continue;
        }
        if let (Some(ProjectChannel::Name(name)), Some(direction)) =
            (&project.channel, project.direction.as_deref())
            && let Some(channel) = device.channels.iter().find(|channel| channel.name == *name)
            && channel.direction != "inout"
            && direction != "inout"
            && channel.direction != direction
        {
            result.errors.push(CodegenError::unsupported(
                codes::PROJECT_IO,
                format!(
                    "I/O binding `{}` direction `{direction}` conflicts with channel `{name}` \
                     direction `{}`",
                    project.variable, channel.direction
                ),
                span,
            ));
            continue;
        }

        let target = &mut image.bindings[index];
        if target.hint.is_some() {
            result.warnings.push(ModuleError {
                code: codes::W_PROJECT_OVERRIDES_HINT,
                severity: crate::module::Severity::Warning,
                message: format!(
                    "project I/O binding for `{}` overrides its `stc:io` hint",
                    target.variable
                ),
                span: target.hint.as_ref().map(|hint| hint.span).or(Some(span)),
                got_id: None,
                old_key: None,
                new_key: None,
            });
        }
        target.device = Some(project.device.clone());
        target.channel.clone_from(&project.channel);
        // A project binding that omits the direction keeps the pragma's
        // direction; every explicitly supplied field wins.
        if let Some(direction) = project.direction.as_deref() {
            target.direction = Some(match direction {
                "input" => "input",
                "output" => "output",
                _ => "inout",
            });
        }
        target.source = "project";
    }

    result
}

/// Returns every source spelling a project binding may use for one variable.
fn binding_names(hir: &HirCrate, binding: &IoBinding) -> Vec<String> {
    let Some(target) = &binding.target else {
        return vec![binding.variable.clone()];
    };
    let IoRootTarget::Global(def) = target else {
        return vec![binding.variable.clone()];
    };
    let mut names = vec![binding.variable.clone()];
    let path = namespace_path(hir, *def);
    if path != binding.variable {
        names.push(path.clone());
    }
    let package = &hir.defs[def.index()].package;
    if let Some(info) = hir.packages.get(package.index()) {
        for prefix in [&info.root_namespace, &info.name] {
            if !prefix.is_empty()
                && !path.starts_with(&format!("{prefix}."))
                && !path.eq_ignore_ascii_case(prefix)
            {
                names.push(format!("{prefix}.{path}"));
            }
        }
    }
    names
}

/// Returns a definition's namespace-qualified source spelling.
fn namespace_path(hir: &HirCrate, def: DefId) -> String {
    let mut segments = Vec::new();
    let mut current = Some(def);
    while let Some(id) = current {
        let data = &hir.defs[id.index()];
        if id == def {
            if let Some(name) = data.name {
                segments.push(hir.interner.text(name).to_string());
            }
        } else if matches!(data.kind, DefKind::Namespace { .. }) {
            // The package root namespace is implicit in source paths.
            if data.parent.is_some()
                && let Some(name) = data.name
            {
                segments.push(hir.interner.text(name).to_string());
            }
        } else {
            break;
        }
        current = data.parent;
    }
    segments.reverse();
    segments.join(".")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn task(
        name: u32,
        resource: &str,
        resource_index: usize,
        task_index: usize,
        priority: Option<i128>,
    ) -> PlanTask {
        PlanTask {
            name: Symbol::new(name),
            name_text: format!("T{name}"),
            resource: resource.into(),
            resource_index,
            task_index,
            single: None,
            interval_ns: None,
            priority,
            span: Span::empty(stc_span::FileId::new(0), 0),
        }
    }

    fn instance(name: u32, resource: &str, task: Option<u32>) -> PlanInstance {
        PlanInstance {
            name: Symbol::new(name),
            name_text: format!("P{name}"),
            program: DefId::new(0),
            resource: resource.into(),
            task: task.map(Symbol::new),
            bindings: Vec::new(),
            global: format!("p{name}"),
            span: Span::empty(stc_span::FileId::new(0), 0),
        }
    }

    #[test]
    fn cycle_order_is_unguarded_then_priority_rank() {
        let plan = Plan {
            configuration: Some("C".into()),
            instances: vec![
                instance(1, "R", Some(10)),
                instance(2, "R", None),
                instance(3, "R", Some(11)),
                instance(4, "R", Some(12)),
                instance(5, "R", Some(10)),
            ],
            tasks: vec![
                // Declaration order is Low, None, High; priority rank is High,
                // Low, None.
                task(10, "R", 0, 0, Some(10)),
                task(11, "R", 0, 1, None),
                task(12, "R", 0, 2, Some(1)),
            ],
            programs: Vec::new(),
            config_inits: Vec::new(),
            var_access: Vec::new(),
        };
        assert_eq!(plan.cycle_instances(), vec![1, 3, 0, 4, 2]);
    }

    #[test]
    fn cycle_order_ties_follow_resource_then_task_declaration() {
        let plan = Plan {
            configuration: Some("C".into()),
            instances: vec![
                instance(1, "R2", Some(20)),
                instance(2, "R1", Some(10)),
                instance(3, "R2", Some(21)),
            ],
            tasks: vec![
                task(10, "R1", 0, 0, Some(1)),
                task(20, "R2", 1, 0, Some(1)),
                task(21, "R2", 1, 1, Some(1)),
            ],
            programs: Vec::new(),
            config_inits: Vec::new(),
            var_access: Vec::new(),
        };
        assert_eq!(plan.cycle_instances(), vec![1, 0, 2]);
    }

    fn route_span() -> Span {
        Span::empty(stc_span::FileId::new(0), 0)
    }

    fn int8() -> ScalarKind {
        ScalarKind::Int {
            signed: false,
            width: 8,
        }
    }

    fn int16() -> ScalarKind {
        ScalarKind::Int {
            signed: false,
            width: 16,
        }
    }

    fn image_binding(
        variable: &str,
        area: DirectArea,
        start_bit: u64,
        kind: ScalarKind,
    ) -> IoBinding {
        IoBinding {
            variable: variable.into(),
            address: String::new(),
            area,
            start_bit,
            kind,
            hint: None,
            device: None,
            channel: None,
            direction: None,
            source: "project",
            init: None,
            target: None,
        }
    }

    fn physical_binding(
        variable: &str,
        area: DirectArea,
        start_bit: u64,
        kind: ScalarKind,
        device: &str,
        channel: &str,
        direction: &'static str,
    ) -> IoBinding {
        let mut binding = image_binding(variable, area, start_bit, kind);
        binding.device = Some(device.into());
        binding.channel = Some(ProjectChannel::Name(channel.into()));
        binding.direction = Some(direction);
        binding
    }

    fn route_image(bindings: Vec<IoBinding>) -> ImageLayout {
        ImageLayout {
            inputs_bytes: 4,
            outputs_bytes: 4,
            markers_bytes: 0,
            input_defaults: Vec::new(),
            output_defaults: Vec::new(),
            bindings,
        }
    }

    fn device_tree() -> Vec<DeviceMetadata> {
        vec![DeviceMetadata {
            name: "diewu".into(),
            kind: "ethercat".into(),
            config: None,
            channels: vec![
                ChannelMetadata {
                    name: "IN0".into(),
                    direction: "input".into(),
                    width: 8,
                },
                ChannelMetadata {
                    name: "IN1".into(),
                    direction: "input".into(),
                    width: 8,
                },
                ChannelMetadata {
                    name: "OUT0".into(),
                    direction: "output".into(),
                    width: 8,
                },
                ChannelMetadata {
                    name: "OUT1".into(),
                    direction: "output".into(),
                    width: 8,
                },
                ChannelMetadata {
                    name: "OUT16".into(),
                    direction: "output".into(),
                    width: 16,
                },
                ChannelMetadata {
                    name: "ZERO".into(),
                    direction: "input".into(),
                    width: 0,
                },
            ],
        }]
    }

    fn phys_input(device: &str, channel: &str) -> IoRouteEndpoint {
        IoRouteEndpoint::PhysicalInput {
            device: device.into(),
            channel: channel.into(),
            span: route_span(),
        }
    }

    fn phys_output(device: &str, channel: &str) -> IoRouteEndpoint {
        IoRouteEndpoint::PhysicalOutput {
            device: device.into(),
            channel: channel.into(),
            span: route_span(),
        }
    }

    fn plc_in(variable: &str) -> IoRouteEndpoint {
        IoRouteEndpoint::PlcInput {
            variable: variable.into(),
            span: route_span(),
        }
    }

    fn plc_out(variable: &str) -> IoRouteEndpoint {
        IoRouteEndpoint::PlcOutput {
            variable: variable.into(),
            span: route_span(),
        }
    }

    fn route(source: IoRouteEndpoint, sinks: Vec<IoRouteEndpoint>) -> IoRoute {
        IoRoute { source, sinks }
    }

    fn assert_route_error(result: &IoRouteResult, code: &'static str, needle: &str) {
        let found = result
            .errors
            .iter()
            .find(|error| error.code() == code && error.message().contains(needle));
        assert!(
            found.is_some(),
            "expected {code} containing `{needle}`, got {:?}",
            result
                .errors
                .iter()
                .map(|error| (error.code(), error.message()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn routes_resolve_fan_out_into_metadata() {
        let hir = HirCrate::new("app", "app");
        let image = route_image(vec![
            image_binding("shadow", DirectArea::Input, 8, int8()),
            image_binding("shadow2", DirectArea::Input, 16, int8()),
            image_binding("drive", DirectArea::Output, 0, int8()),
        ]);
        let routes = vec![
            route(
                phys_input("diewu", "IN0"),
                vec![plc_in("shadow"), phys_output("diewu", "OUT0")],
            ),
            route(
                plc_out("drive"),
                vec![plc_in("shadow2"), phys_output("diewu", "OUT1")],
            ),
        ];
        let result = resolve_routes(&hir, &image, &device_tree(), &routes);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(result.warnings.is_empty());
        assert_eq!(result.routes.len(), 2);

        let first = &result.routes[0];
        assert_eq!(first.source.kind, "physical-input");
        assert_eq!(first.source.device.as_deref(), Some("diewu"));
        assert_eq!(first.source.channel.as_deref(), Some("IN0"));
        assert_eq!(first.source.width_bits, 8);
        assert_eq!(first.sinks.len(), 2);
        assert_eq!(first.sinks[0].kind, "plc-input");
        assert_eq!(first.sinks[0].variable.as_deref(), Some("shadow"));
        assert_eq!(first.sinks[0].area, Some('I'));
        assert_eq!(first.sinks[0].offset_bits, Some(8));
        assert_eq!(first.sinks[0].width_bits, 8);
        assert_eq!(first.sinks[1].kind, "physical-output");
        assert_eq!(first.sinks[1].device.as_deref(), Some("diewu"));
        assert_eq!(first.sinks[1].channel.as_deref(), Some("OUT0"));

        let second = &result.routes[1];
        assert_eq!(second.source.kind, "plc-output");
        assert_eq!(second.source.variable.as_deref(), Some("drive"));
        assert_eq!(second.source.area, Some('Q'));
        assert_eq!(second.source.offset_bits, Some(0));
        assert_eq!(second.source.width_bits, 8);
        assert_eq!(second.sinks[0].kind, "plc-input");
        assert_eq!(second.sinks[0].offset_bits, Some(16));
        assert_eq!(second.sinks[1].kind, "physical-output");
        assert_eq!(second.sinks[1].channel.as_deref(), Some("OUT1"));
    }

    #[test]
    fn routes_reject_fan_in_and_binding_sink_conflicts() {
        let hir = HirCrate::new("app", "app");
        let image = route_image(vec![image_binding("shadow", DirectArea::Input, 8, int8())]);
        let fan_in = vec![
            route(phys_input("diewu", "IN0"), vec![plc_in("shadow")]),
            route(phys_input("diewu", "IN1"), vec![plc_in("shadow")]),
        ];
        let result = resolve_routes(&hir, &image, &device_tree(), &fan_in);
        assert_route_error(&result, codes::IO_BINDING_CONFLICT, "fan-in");

        let image = route_image(vec![
            physical_binding(
                "out",
                DirectArea::Output,
                0,
                int8(),
                "diewu",
                "OUT0",
                "output",
            ),
            physical_binding(
                "shadow",
                DirectArea::Input,
                8,
                int8(),
                "diewu",
                "IN0",
                "input",
            ),
        ]);
        let routes = vec![
            route(
                phys_input("diewu", "IN0"),
                vec![phys_output("diewu", "OUT0")],
            ),
            route(phys_input("diewu", "IN0"), vec![plc_in("shadow")]),
        ];
        let result = resolve_routes(&hir, &image, &device_tree(), &routes);
        assert_route_error(&result, codes::IO_BINDING_CONFLICT, "is fan-in");
        assert_route_error(&result, codes::IO_BINDING_CONFLICT, "binding `out`");
        assert_route_error(&result, codes::IO_BINDING_CONFLICT, "binding `shadow`");
        assert_eq!(
            result
                .errors
                .iter()
                .filter(|error| error.code() == codes::IO_BINDING_CONFLICT)
                .count(),
            2,
            "both the physical output and the PLC input are claimed"
        );
    }

    #[test]
    fn routes_reject_invalid_endpoints_and_widths() {
        let hir = HirCrate::new("app", "app");
        let image = route_image(vec![
            image_binding("shadow", DirectArea::Input, 8, int8()),
            image_binding("wide_in", DirectArea::Input, 16, int16()),
            image_binding("drive", DirectArea::Output, 0, int8()),
        ]);
        let devices = device_tree();

        let cases = [
            (
                vec![route(plc_in("shadow"), vec![plc_in("shadow")])],
                "cannot be a route source",
            ),
            (
                vec![route(plc_out("drive"), vec![plc_out("drive")])],
                "cannot be a route sink",
            ),
            (
                vec![route(plc_out("missing"), vec![plc_in("shadow")])],
                "does not name a direct `%Q` variable",
            ),
            (
                vec![route(plc_out("shadow"), vec![plc_in("shadow")])],
                "does not name a direct `%Q` variable",
            ),
            (
                vec![route(phys_input("ghost", "IN0"), vec![plc_in("shadow")])],
                "undeclared device",
            ),
            (
                vec![route(phys_input("diewu", "GHOST"), vec![plc_in("shadow")])],
                "undeclared channel",
            ),
            (
                vec![route(phys_input("diewu", "OUT0"), vec![plc_in("shadow")])],
                "has direction `output`, expected `input`",
            ),
            (
                vec![route(phys_input("diewu", "ZERO"), vec![plc_in("shadow")])],
                "has zero width",
            ),
            (
                vec![route(
                    phys_input("diewu", "IN0"),
                    vec![phys_output("diewu", "OUT16")],
                )],
                "route source is 8 bits wide",
            ),
            (
                vec![route(phys_input("diewu", "IN0"), vec![plc_in("wide_in")])],
                "route source is 8 bits wide",
            ),
            (
                vec![route(
                    phys_input("diewu", "IN0"),
                    vec![phys_output("ghost", "OUT0")],
                )],
                "undeclared device",
            ),
            (
                vec![route(
                    phys_input("diewu", "IN0"),
                    vec![phys_output("diewu", "IN0")],
                )],
                "has direction `input`, expected `output`",
            ),
        ];
        for (routes, needle) in cases {
            let result = resolve_routes(&hir, &image, &devices, &routes);
            assert_route_error(&result, codes::IO_BINDING_INVALID, needle);
        }
    }
}
