//! The compiler identity engine: baseline matching, tracked rename/move maps,
//! preserve/allocate/drop classification, slot assignment, and manifest
//! emission (M4.5 phase 2).

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::{
    Catalog, DescriptorDecl, ExportDecl, FunctionDecl, Identity, ImportService, InstanceDecl,
    RootDecl, TaskDecl, TypeDecl, root_kind_identity,
};
use crate::digest::{Digest32, GotId};
use crate::error::{BuildError, Diagnostic, Error, Result, codes};
use crate::keys::IdentityKind;
use crate::layout::Target;
use crate::manifest::{FEATURE_HOST_ABI, FEATURE_ONLINE_PLAN, Manifest};
use crate::model::{
    ActionRecord, DescriptorKind, DescriptorRecord, ExportRecord, FieldRecord, FunctionRecord,
    Header, History, InstanceRecord, Plan, PlanAction, PlanDiagnostic, PlanVerdict, RetiredCause,
    RetiredRecord, RootKind, RootRecord, SlotKind, SlotRecord, TaskRecord, TypeRecord,
    compare_plan_diagnostics,
};
use crate::sha256::{domain_hash, sha256};
use crate::wire::Encoder;

/// Generator flag: untracked removal explicitly allowed.
pub const GENERATOR_UNTRACKED_ALLOWED: u64 = 1 << 5;
/// Generator flag: cold-layout build.
pub const GENERATOR_COLD_LAYOUT: u64 = 1 << 6;
/// The host ABI the M4.5 compiler targets.
pub const HOST_ABI_MAJOR: u16 = 2;
/// The host ABI minor the M4.5 compiler targets.
pub const HOST_ABI_MINOR: u16 = 0;

/// Inputs that describe one build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildInput {
    /// Profile identifier.
    pub profile: String,
    /// Compiler package version.
    pub compiler_version: String,
    /// Resolved build target.
    pub target: Target,
    /// Module name.
    pub module_name: String,
    /// Active configuration name (empty when implicit).
    pub active_configuration: String,
    /// Module format.
    pub module_format: u16,
    /// Host ABI major.
    pub host_abi_major: u16,
    /// Host ABI minor.
    pub host_abi_minor: u16,
    /// Generator flags.
    pub generator_flags: u64,
    /// Required feature bits.
    pub required_features: u64,
    /// Paired descriptor digest.
    pub descriptor_digest: Digest32,
    /// Source fingerprint.
    pub source_fingerprint: Digest32,
}

impl BuildInput {
    /// Creates an M4.5-profile build input.
    #[must_use]
    pub fn new(
        profile: impl Into<String>,
        compiler_version: impl Into<String>,
        target: Target,
        module_name: impl Into<String>,
    ) -> Self {
        Self {
            profile: profile.into(),
            compiler_version: compiler_version.into(),
            target,
            module_name: module_name.into(),
            active_configuration: String::new(),
            module_format: 1,
            host_abi_major: HOST_ABI_MAJOR,
            host_abi_minor: HOST_ABI_MINOR,
            generator_flags: 0,
            required_features: FEATURE_HOST_ABI | FEATURE_ONLINE_PLAN,
            descriptor_digest: Digest32::ZERO,
            source_fingerprint: Digest32::ZERO,
        }
    }
}

/// One tracked rename or move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rename {
    /// Baseline key or type field path.
    pub old: String,
    /// New key or type field path.
    pub new: String,
}

impl Rename {
    /// Creates a rename entry.
    #[must_use]
    pub fn new(old: impl Into<String>, new: impl Into<String>) -> Self {
        Self {
            old: old.into(),
            new: new.into(),
        }
    }
}

/// Options that select the baseline and policy.
#[derive(Clone, Debug, Default)]
pub struct BuildOptions<'a> {
    /// Optional baseline manifest.
    pub baseline: Option<&'a Manifest>,
    /// Force a cold-layout build.
    pub cold: bool,
    /// Permit untracked removal of data roots.
    pub allow_untracked_removal: bool,
    /// Tracked rename/move maps.
    pub renames: Vec<Rename>,
}

/// The result of a successful identity build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildOutcome {
    /// The emitted manifest.
    pub manifest: Manifest,
    /// Binary manifest projection.
    pub binary: Vec<u8>,
    /// Canonical JSON manifest projection.
    pub json: String,
    /// Compiler diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

impl BuildOutcome {
    /// Returns the compiler verdict.
    #[must_use]
    pub fn verdict(&self) -> PlanVerdict {
        self.manifest.plan.verdict
    }

    /// Returns `true` when the build may be deployed (online or cold).
    #[must_use]
    pub fn is_deployable(&self) -> bool {
        self.verdict() != PlanVerdict::Rejected
            && !self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// Computes the source fingerprint from `(path, bytes)` pairs.
///
/// Paths are sorted by bytes, must use `/` separators, and must be
/// package-relative.
///
/// # Errors
///
/// Rejects empty, absolute, drive-qualified, backslash, or `..` paths.
pub fn source_fingerprint(files: &[(String, Vec<u8>)]) -> Result<Digest32> {
    let mut ordered: Vec<&(String, Vec<u8>)> = files.iter().collect();
    ordered.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let mut encoded = Encoder::new();
    let count = u32::try_from(ordered.len())
        .map_err(|_| Error::manifest("too many source files for the fingerprint"))?;
    encoded.u32(count);
    for (path, bytes) in ordered {
        validate_source_path(path)?;
        encoded.string(path)?;
        encoded.raw(&sha256(bytes));
    }
    Ok(Digest32::from_bytes(domain_hash(
        "stc:source:v1",
        &[encoded.as_bytes()],
    )))
}

fn validate_source_path(path: &str) -> Result<()> {
    if path.is_empty() {
        return Err(Error::manifest("source path is empty"));
    }
    if path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return Err(Error::manifest(format!(
            "source path `{path}` must be a relative `/`-separated path"
        )));
    }
    if path
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(Error::manifest(format!(
            "source path `{path}` contains an invalid segment"
        )));
    }
    Ok(())
}

#[derive(Clone)]
struct BaselineInfo {
    got_id: GotId,
    key: String,
    previous_keys: Vec<String>,
    kind: u8,
    shape: BaselineShape,
}

#[derive(Clone)]
enum BaselineShape {
    Root {
        root_kind: RootKind,
        type_key: String,
        layout_hash: GotId,
        size: u64,
        align: u32,
        initializer_digest: Option<Digest32>,
    },
    Function {
        signature_key: String,
    },
    Type {
        type_key: String,
        layout_hash: GotId,
        size: u64,
        align: u32,
        vtable_shape: Option<GotId>,
        base: Option<GotId>,
        interfaces: Vec<GotId>,
        fields: Vec<FieldRecord>,
    },
    Descriptor {
        descriptor_kind: DescriptorKind,
        owner_type: Option<GotId>,
        type_key: String,
        layout_hash: GotId,
        size: u64,
        align: u32,
    },
    Instance {
        program_type: Option<GotId>,
        program_type_key: String,
        configuration: String,
        resource: String,
        instance_path: String,
        task_key: Option<String>,
    },
    Task {
        configuration: String,
        resource: String,
        name: String,
        single: bool,
        interval_ns: Option<i64>,
        priority: Option<i64>,
    },
    Export {
        c_name: String,
        function_got_id: GotId,
        signature_key: String,
        no_mangle: bool,
    },
}

struct BaselineIndex {
    records: BTreeMap<(u8, String), BaselineInfo>,
    keys_by_kind: BTreeMap<String, BTreeSet<u8>>,
    used: BTreeSet<(u8, String)>,
    build_id: GotId,
    digest: Digest32,
    source_fingerprint: Digest32,
    history: History,
}

impl BaselineIndex {
    fn new(manifest: &Manifest) -> Result<Self> {
        let mut records = BTreeMap::new();
        let mut keys_by_kind: BTreeMap<String, BTreeSet<u8>> = BTreeMap::new();
        for root in &manifest.roots {
            let kind = root_kind_identity(root.root_kind);
            insert_baseline(
                &mut records,
                kind.as_u8(),
                root.key.clone(),
                BaselineInfo {
                    got_id: root.got_id,
                    key: root.key.clone(),
                    previous_keys: root.previous_keys.clone(),
                    kind: kind.as_u8(),
                    shape: BaselineShape::Root {
                        root_kind: root.root_kind,
                        type_key: root.type_key.clone(),
                        layout_hash: root.layout_hash,
                        size: root.size,
                        align: root.align,
                        initializer_digest: root.initializer_digest,
                    },
                },
            )?;
            keys_by_kind
                .entry(root.key.clone())
                .or_default()
                .insert(kind.as_u8());
        }
        for function in &manifest.functions {
            insert_baseline(
                &mut records,
                IdentityKind::Function.as_u8(),
                function.key.clone(),
                BaselineInfo {
                    got_id: function.got_id,
                    key: function.key.clone(),
                    previous_keys: function.previous_keys.clone(),
                    kind: IdentityKind::Function.as_u8(),
                    shape: BaselineShape::Function {
                        signature_key: function.signature_key.clone(),
                    },
                },
            )?;
            keys_by_kind
                .entry(function.key.clone())
                .or_default()
                .insert(IdentityKind::Function.as_u8());
        }
        for ty in &manifest.types {
            insert_baseline(
                &mut records,
                IdentityKind::Type.as_u8(),
                ty.key.clone(),
                BaselineInfo {
                    got_id: ty.got_id,
                    key: ty.key.clone(),
                    previous_keys: ty.previous_keys.clone(),
                    kind: IdentityKind::Type.as_u8(),
                    shape: BaselineShape::Type {
                        type_key: ty.type_key.clone(),
                        layout_hash: ty.layout_hash,
                        size: ty.size,
                        align: ty.align,
                        vtable_shape: ty.vtable_shape,
                        base: ty.base,
                        interfaces: ty.interfaces.clone(),
                        fields: ty.fields.clone(),
                    },
                },
            )?;
            keys_by_kind
                .entry(ty.key.clone())
                .or_default()
                .insert(IdentityKind::Type.as_u8());
        }
        for descriptor in &manifest.descriptors {
            insert_baseline(
                &mut records,
                IdentityKind::Descriptor.as_u8(),
                descriptor.key.clone(),
                BaselineInfo {
                    got_id: descriptor.got_id,
                    key: descriptor.key.clone(),
                    previous_keys: descriptor.previous_keys.clone(),
                    kind: IdentityKind::Descriptor.as_u8(),
                    shape: BaselineShape::Descriptor {
                        descriptor_kind: descriptor.descriptor_kind,
                        owner_type: descriptor.owner_type,
                        type_key: descriptor.type_key.clone(),
                        layout_hash: descriptor.layout_hash,
                        size: descriptor.size,
                        align: descriptor.align,
                    },
                },
            )?;
            keys_by_kind
                .entry(descriptor.key.clone())
                .or_default()
                .insert(IdentityKind::Descriptor.as_u8());
        }
        for instance in &manifest.instances {
            insert_baseline(
                &mut records,
                IdentityKind::Instance.as_u8(),
                instance.key.clone(),
                BaselineInfo {
                    got_id: instance.got_id,
                    key: instance.key.clone(),
                    previous_keys: instance.previous_keys.clone(),
                    kind: IdentityKind::Instance.as_u8(),
                    shape: BaselineShape::Instance {
                        program_type: instance.program_type,
                        program_type_key: instance.program_type_key.clone(),
                        configuration: instance.configuration.clone(),
                        resource: instance.resource.clone(),
                        instance_path: instance.instance_path.clone(),
                        task_key: instance.task_key.clone(),
                    },
                },
            )?;
            keys_by_kind
                .entry(instance.key.clone())
                .or_default()
                .insert(IdentityKind::Instance.as_u8());
        }
        for task in &manifest.tasks {
            insert_baseline(
                &mut records,
                IdentityKind::Task.as_u8(),
                task.key.clone(),
                BaselineInfo {
                    got_id: task.got_id,
                    key: task.key.clone(),
                    previous_keys: task.previous_keys.clone(),
                    kind: IdentityKind::Task.as_u8(),
                    shape: BaselineShape::Task {
                        configuration: task.configuration.clone(),
                        resource: task.resource.clone(),
                        name: task.name.clone(),
                        single: task.single,
                        interval_ns: task.interval_ns,
                        priority: task.priority,
                    },
                },
            )?;
            keys_by_kind
                .entry(task.key.clone())
                .or_default()
                .insert(IdentityKind::Task.as_u8());
        }
        for export in &manifest.exports {
            insert_baseline(
                &mut records,
                IdentityKind::Export.as_u8(),
                export.key.clone(),
                BaselineInfo {
                    got_id: export.got_id,
                    key: export.key.clone(),
                    previous_keys: export.previous_keys.clone(),
                    kind: IdentityKind::Export.as_u8(),
                    shape: BaselineShape::Export {
                        c_name: export.c_name.clone(),
                        function_got_id: export.function_got_id,
                        signature_key: export.signature_key.clone(),
                        no_mangle: export.no_mangle,
                    },
                },
            )?;
            keys_by_kind
                .entry(export.key.clone())
                .or_default()
                .insert(IdentityKind::Export.as_u8());
        }
        Ok(Self {
            records,
            keys_by_kind,
            used: BTreeSet::new(),
            build_id: manifest.header.build_id,
            digest: manifest.digest()?,
            source_fingerprint: manifest.header.source_fingerprint,
            history: manifest.history.clone(),
        })
    }
}

fn insert_baseline(
    records: &mut BTreeMap<(u8, String), BaselineInfo>,
    kind: u8,
    key: String,
    info: BaselineInfo,
) -> Result<()> {
    if records.insert((kind, key.clone()), info).is_some() {
        return Err(Error::manifest(format!(
            "baseline contains a duplicate identity key `{key}`"
        )));
    }
    Ok(())
}

#[derive(Clone)]
struct Assigned {
    got_id: GotId,
    previous_keys: Vec<String>,
    baseline: Option<BaselineInfo>,
    old_key: Option<String>,
}

/// Builds a manifest from a catalog and optional baseline.
///
/// # Errors
///
/// Returns [`BuildError`] for malformed catalogs, identity collisions, an
/// untracked data-root removal without the explicit policy, and inconsistent
/// cross-references.
pub fn build(
    catalog: &Catalog,
    input: &BuildInput,
    options: &BuildOptions<'_>,
) -> std::result::Result<BuildOutcome, BuildError> {
    catalog.validate().map_err(BuildError::from_error)?;
    validate_input(input)?;

    let mut diagnostics = Vec::new();
    let mut baseline = establish_baseline(input, options, &mut diagnostics);
    let online = baseline.is_some();

    let mut ids = IdTables::default();
    let mut root_assignments = Vec::new();
    for decl in &catalog.roots {
        let assigned = resolve(
            baseline.as_mut(),
            &options.renames,
            root_kind_identity(decl.root_kind),
            &decl.key,
            &decl.identity,
            true,
            &mut ids,
        )?;
        root_assignments.push(assigned);
    }
    let mut function_assignments = Vec::new();
    for decl in &catalog.functions {
        let assigned = resolve(
            baseline.as_mut(),
            &options.renames,
            IdentityKind::Function,
            &decl.key,
            &decl.identity,
            false,
            &mut ids,
        )?;
        function_assignments.push(assigned);
    }
    let mut type_assignments = Vec::new();
    for decl in &catalog.types {
        let assigned = resolve(
            baseline.as_mut(),
            &options.renames,
            IdentityKind::Type,
            &decl.key,
            &decl.identity,
            false,
            &mut ids,
        )?;
        type_assignments.push(assigned);
    }
    let mut descriptor_assignments = Vec::new();
    for decl in &catalog.descriptors {
        let assigned = resolve(
            baseline.as_mut(),
            &options.renames,
            IdentityKind::Descriptor,
            &decl.key,
            &decl.identity,
            false,
            &mut ids,
        )?;
        descriptor_assignments.push(assigned);
    }
    let mut instance_assignments = Vec::new();
    for decl in &catalog.instances {
        let assigned = resolve(
            baseline.as_mut(),
            &options.renames,
            IdentityKind::Instance,
            &decl.key,
            &decl.identity,
            false,
            &mut ids,
        )?;
        instance_assignments.push(assigned);
    }
    let mut task_assignments = Vec::new();
    for decl in &catalog.tasks {
        let assigned = resolve(
            baseline.as_mut(),
            &options.renames,
            IdentityKind::Task,
            &decl.key,
            &decl.identity,
            false,
            &mut ids,
        )?;
        task_assignments.push(assigned);
    }
    let mut export_assignments = Vec::new();
    for decl in &catalog.exports {
        let assigned = resolve(
            baseline.as_mut(),
            &options.renames,
            IdentityKind::Export,
            &decl.key,
            &decl.identity,
            false,
            &mut ids,
        )?;
        export_assignments.push(assigned);
    }

    // A content-derived identity must not resurrect a retired identity. V1 has
    // no explicit reuse policy, so refuse rather than silently reusing
    // quarantined storage.
    if let Some(index) = baseline.as_ref() {
        for (got_id, (_kind, key)) in &ids.identities {
            if index
                .history
                .retired
                .iter()
                .any(|record| record.got_id == *got_id)
            {
                return Err(BuildError::single(
                    Diagnostic::error(
                        codes::MANIFEST_INVALID,
                        format!(
                            "`{key}` derives a retired GotId {got_id}; \
                             reintroduction requires an explicit reuse policy"
                        ),
                    )
                    .with_got_id(*got_id)
                    .with_new_key(key.clone()),
                ));
            }
        }
    }

    let type_ids: BTreeMap<String, GotId> = catalog
        .types
        .iter()
        .zip(&type_assignments)
        .map(|(decl, assigned)| (decl.key.clone(), assigned.got_id))
        .collect();
    let function_ids: BTreeMap<String, GotId> = catalog
        .functions
        .iter()
        .zip(&function_assignments)
        .map(|(decl, assigned)| (decl.key.clone(), assigned.got_id))
        .collect();
    let task_ids: BTreeMap<String, GotId> = catalog
        .tasks
        .iter()
        .zip(&task_assignments)
        .map(|(decl, assigned)| (decl.key.clone(), assigned.got_id))
        .collect();
    let root_ids: BTreeMap<String, GotId> = catalog
        .roots
        .iter()
        .zip(&root_assignments)
        .map(|(decl, assigned)| (decl.key.clone(), assigned.got_id))
        .collect();

    let mut actions = Vec::new();
    let mut roots = Vec::new();
    for (decl, assigned) in catalog.roots.iter().zip(&root_assignments) {
        let (action, reason, detail, warning) = classify_root(decl, assigned);
        if let Some(diagnostic) = warning {
            diagnostics.push(diagnostic);
        }
        if action == PlanAction::Reject {
            diagnostics.push(
                Diagnostic::error(codes::ONLINE_LAYOUT, detail.clone())
                    .with_got_id(assigned.got_id)
                    .with_old_key(
                        assigned
                            .baseline
                            .as_ref()
                            .map(|info| info.key.clone())
                            .unwrap_or_default(),
                    )
                    .with_new_key(decl.key.clone()),
            );
        }
        actions.push(ActionRecord {
            got_id: assigned.got_id,
            kind: root_kind_identity(decl.root_kind).as_u8(),
            key: decl.key.clone(),
            old_key: assigned.old_key.clone(),
            action,
            reason_code: reason.into(),
            detail,
        });
        roots.push(RootRecord {
            got_id: assigned.got_id,
            key: decl.key.clone(),
            previous_keys: assigned.previous_keys.clone(),
            root_kind: decl.root_kind,
            owner_type: resolve_type(&type_ids, decl.owner_type.as_deref(), &decl.key)?,
            field_path: decl.field_path.clone(),
            type_key: decl.type_key.clone(),
            layout_hash: decl.layout_hash,
            size: decl.size,
            align: decl.align,
            retention: decl.retention,
            initializer: decl.initializer,
            initializer_digest: decl.initializer_digest,
            source: decl.source.clone(),
            slot: 0,
        });
    }

    let mut functions = Vec::new();
    for (decl, assigned) in catalog.functions.iter().zip(&function_assignments) {
        let (action, reason, detail, warning) = classify_function(decl, assigned);
        if let Some(diagnostic) = warning {
            diagnostics.push(diagnostic);
        }
        if action == PlanAction::Reject {
            diagnostics.push(
                Diagnostic::error(codes::ONLINE_EXPORT, detail.clone())
                    .with_got_id(assigned.got_id)
                    .with_new_key(decl.key.clone()),
            );
        }
        actions.push(ActionRecord {
            got_id: assigned.got_id,
            kind: IdentityKind::Function.as_u8(),
            key: decl.key.clone(),
            old_key: assigned.old_key.clone(),
            action,
            reason_code: reason.into(),
            detail,
        });
        functions.push(FunctionRecord {
            got_id: assigned.got_id,
            key: decl.key.clone(),
            previous_keys: assigned.previous_keys.clone(),
            signature_key: decl.signature_key.clone(),
            function_kind: decl.function_kind,
            owner_type: resolve_type(&type_ids, decl.owner_type.as_deref(), &decl.key)?,
            flags: decl.flags,
            slot: 0,
            export_name: decl.export_name.clone(),
            vtable_slot: decl.vtable_slot,
            source: decl.source.clone(),
        });
    }

    let mut types = Vec::new();
    for (decl, assigned) in catalog.types.iter().zip(&type_assignments) {
        let resolved_base = resolve_type(&type_ids, decl.base.as_deref(), &decl.key)?;
        let mut resolved_interfaces = Vec::new();
        for interface in &decl.interfaces {
            resolved_interfaces.push(resolved_type(&type_ids, interface, &decl.key)?);
        }
        let (action, reason, detail, warning) =
            classify_type(decl, assigned, resolved_base, &resolved_interfaces);
        if let Some(diagnostic) = warning {
            diagnostics.push(diagnostic);
        }
        if action == PlanAction::Reject {
            let code = if reason == "reject.vtable" {
                codes::ONLINE_VTABLE
            } else {
                codes::ONLINE_LAYOUT
            };
            diagnostics.push(
                Diagnostic::error(code, detail.clone())
                    .with_got_id(assigned.got_id)
                    .with_new_key(decl.key.clone()),
            );
        }
        actions.push(ActionRecord {
            got_id: assigned.got_id,
            kind: IdentityKind::Type.as_u8(),
            key: decl.key.clone(),
            old_key: assigned.old_key.clone(),
            action,
            reason_code: reason.into(),
            detail,
        });
        let mut fields = Vec::new();
        for field in &decl.fields {
            let carried = assigned.baseline.as_ref().and_then(|info| {
                if let BaselineShape::Type { fields, .. } = &info.shape {
                    fields
                        .iter()
                        .find(|existing| existing.path == field.path)
                        .map(|existing| existing.got_id)
                } else {
                    None
                }
            });
            let (got_id, full) = match carried {
                Some(got_id) => (got_id, None),
                None => {
                    let (got_id, full) = field.identity.derive();
                    (got_id, Some(full))
                }
            };
            let field_key = crate::keys::field_identity_key(
                &field.identity.package,
                &decl.identity.path,
                &field.path,
            )?;
            ids.register(got_id, IdentityKind::Field, &field_key, full)?;
            fields.push(FieldRecord {
                got_id,
                path: field.path.clone(),
                type_key: field.type_key.clone(),
                layout_hash: field.layout_hash,
                offset: field.offset,
                size: field.size,
                align: field.align,
                retention: field.retention,
                source: field.source.clone(),
            });
        }
        fields.sort_by(|left, right| {
            (left.offset, left.path.as_str()).cmp(&(right.offset, right.path.as_str()))
        });
        types.push(TypeRecord {
            got_id: assigned.got_id,
            key: decl.key.clone(),
            previous_keys: assigned.previous_keys.clone(),
            type_key: decl.type_key.clone(),
            layout_hash: decl.layout_hash,
            size: decl.size,
            align: decl.align,
            type_kind: decl.type_kind,
            base: resolved_base,
            interfaces: resolved_interfaces,
            vtable_shape: decl.vtable_shape,
            fields,
            source: decl.source.clone(),
        });
    }

    let mut descriptors = Vec::new();
    for (decl, assigned) in catalog.descriptors.iter().zip(&descriptor_assignments) {
        let resolved_owner = resolve_type(&type_ids, decl.owner_type.as_deref(), &decl.key)?;
        let (action, reason, detail, warning) = classify_descriptor(decl, assigned, resolved_owner);
        if let Some(diagnostic) = warning {
            diagnostics.push(diagnostic);
        }
        if action == PlanAction::Reject {
            let code = if reason == "reject.layout" {
                codes::ONLINE_LAYOUT
            } else {
                codes::ONLINE_VTABLE
            };
            diagnostics.push(
                Diagnostic::error(code, detail.clone())
                    .with_got_id(assigned.got_id)
                    .with_new_key(decl.key.clone()),
            );
        }
        actions.push(ActionRecord {
            got_id: assigned.got_id,
            kind: IdentityKind::Descriptor.as_u8(),
            key: decl.key.clone(),
            old_key: assigned.old_key.clone(),
            action,
            reason_code: reason.into(),
            detail,
        });
        descriptors.push(DescriptorRecord {
            got_id: assigned.got_id,
            key: decl.key.clone(),
            previous_keys: assigned.previous_keys.clone(),
            descriptor_kind: decl.descriptor_kind,
            owner_type: resolved_owner,
            type_key: decl.type_key.clone(),
            layout_hash: decl.layout_hash,
            size: decl.size,
            align: decl.align,
            slot: 0,
            source: decl.source.clone(),
        });
    }

    let mut instances = Vec::new();
    for (decl, assigned) in catalog.instances.iter().zip(&instance_assignments) {
        let resolved_program = resolve_type(&type_ids, decl.program_type.as_deref(), &decl.key)?;
        let (action, reason, detail, warning) = classify_instance(decl, assigned, resolved_program);
        if let Some(diagnostic) = warning {
            diagnostics.push(diagnostic);
        }
        if action == PlanAction::Reject {
            diagnostics.push(
                Diagnostic::error(codes::ONLINE_TOPOLOGY, detail.clone())
                    .with_got_id(assigned.got_id)
                    .with_new_key(decl.key.clone()),
            );
        }
        actions.push(ActionRecord {
            got_id: assigned.got_id,
            kind: IdentityKind::Instance.as_u8(),
            key: decl.key.clone(),
            old_key: assigned.old_key.clone(),
            action,
            reason_code: reason.into(),
            detail,
        });
        if let Some(task_key) = &decl.task_key
            && !task_ids.contains_key(task_key)
        {
            return Err(BuildError::single(
                Diagnostic::error(
                    codes::MANIFEST_INVALID,
                    format!(
                        "instance `{}` references an unknown task `{task_key}`",
                        decl.key
                    ),
                )
                .with_got_id(assigned.got_id),
            ));
        }
        instances.push(InstanceRecord {
            got_id: assigned.got_id,
            key: decl.key.clone(),
            previous_keys: assigned.previous_keys.clone(),
            program_type: resolved_program,
            program_type_key: decl.program_type_key.clone(),
            configuration: decl.configuration.clone(),
            resource: decl.resource.clone(),
            instance_path: decl.instance_path.clone(),
            task_key: decl.task_key.clone(),
            source: decl.source.clone(),
        });
    }

    let mut tasks = Vec::new();
    for (decl, assigned) in catalog.tasks.iter().zip(&task_assignments) {
        let (action, reason, detail, warning) = classify_task(decl, assigned);
        if let Some(diagnostic) = warning {
            diagnostics.push(diagnostic);
        }
        if action == PlanAction::Reject {
            diagnostics.push(
                Diagnostic::error(codes::ONLINE_TOPOLOGY, detail.clone())
                    .with_got_id(assigned.got_id)
                    .with_new_key(decl.key.clone()),
            );
        }
        actions.push(ActionRecord {
            got_id: assigned.got_id,
            kind: IdentityKind::Task.as_u8(),
            key: decl.key.clone(),
            old_key: assigned.old_key.clone(),
            action,
            reason_code: reason.into(),
            detail,
        });
        let guard_got_id = match &decl.guard_key {
            Some(key) => {
                let Some(got_id) = root_ids.get(key) else {
                    return Err(BuildError::single(
                        Diagnostic::error(
                            codes::MANIFEST_INVALID,
                            format!(
                                "task `{}` references an unknown guard root `{key}`",
                                decl.key
                            ),
                        )
                        .with_got_id(assigned.got_id),
                    ));
                };
                Some(*got_id)
            }
            None => None,
        };
        tasks.push(TaskRecord {
            got_id: assigned.got_id,
            key: decl.key.clone(),
            previous_keys: assigned.previous_keys.clone(),
            configuration: decl.configuration.clone(),
            resource: decl.resource.clone(),
            name: decl.name.clone(),
            single: decl.single,
            interval_ns: decl.interval_ns,
            priority: decl.priority,
            guard_got_id,
            source: decl.source.clone(),
        });
    }

    let mut exports = Vec::new();
    for (decl, assigned) in catalog.exports.iter().zip(&export_assignments) {
        let (action, reason, detail, warning) = classify_export(decl, assigned, &function_ids);
        if let Some(diagnostic) = warning {
            diagnostics.push(diagnostic);
        }
        if action == PlanAction::Reject {
            diagnostics.push(
                Diagnostic::error(codes::ONLINE_EXPORT, detail.clone())
                    .with_got_id(assigned.got_id)
                    .with_new_key(decl.key.clone()),
            );
        }
        actions.push(ActionRecord {
            got_id: assigned.got_id,
            kind: IdentityKind::Export.as_u8(),
            key: decl.key.clone(),
            old_key: assigned.old_key.clone(),
            action,
            reason_code: reason.into(),
            detail,
        });
        let function_got_id = *function_ids.get(&decl.function_key).ok_or_else(|| {
            BuildError::single(
                Diagnostic::error(
                    codes::MANIFEST_INVALID,
                    format!(
                        "export `{}` references an unknown function `{}`",
                        decl.key, decl.function_key
                    ),
                )
                .with_got_id(assigned.got_id),
            )
        })?;
        exports.push(ExportRecord {
            got_id: assigned.got_id,
            key: decl.key.clone(),
            previous_keys: assigned.previous_keys.clone(),
            c_name: decl.c_name.clone(),
            function_got_id,
            signature_key: decl.signature_key.clone(),
            no_mangle: decl.no_mangle,
            source: decl.source.clone(),
        });
    }

    let mut fatal = Vec::new();
    let mut history = History::default();
    if let Some(index) = baseline.as_ref() {
        history = index.history.clone();
        if index.source_fingerprint != input.source_fingerprint {
            diagnostics.push(Diagnostic::warning(
                codes::W_MANIFEST_SOURCE,
                "baseline source fingerprint differs from the current source tree",
            ));
        }
        for ((kind_tag, key), info) in &index.records {
            if index.used.contains(&(*kind_tag, key.clone())) {
                continue;
            }
            let kind = IdentityKind::from_tag(*kind_tag).expect("baseline identity kind");
            match kind {
                IdentityKind::Direct => {}
                IdentityKind::Export => {
                    let message = format!("export `{key}` was removed");
                    diagnostics.push(
                        Diagnostic::error(codes::ONLINE_EXPORT, message.clone())
                            .with_got_id(info.got_id)
                            .with_old_key(key.clone()),
                    );
                    actions.push(ActionRecord {
                        got_id: info.got_id,
                        kind: kind_tag.to_owned(),
                        key: key.clone(),
                        old_key: None,
                        action: PlanAction::Reject,
                        reason_code: "reject.export".into(),
                        detail: message,
                    });
                }
                IdentityKind::DataRoot => {
                    let message =
                        format!("removed data root `{key}` is quarantined; its state is discarded");
                    diagnostics.push(
                        Diagnostic::warning(codes::W_ONLINE_REMOVED, message.clone())
                            .with_got_id(info.got_id)
                            .with_old_key(key.clone()),
                    );
                    diagnostics.push(
                        Diagnostic::warning(codes::W_MANIFEST_UNTRACKED, message.clone())
                            .with_got_id(info.got_id)
                            .with_old_key(key.clone()),
                    );
                    actions.push(ActionRecord {
                        got_id: info.got_id,
                        kind: kind_tag.to_owned(),
                        key: key.clone(),
                        old_key: None,
                        action: PlanAction::Drop,
                        reason_code: "drop.removed".into(),
                        detail: message,
                    });
                    if !options.allow_untracked_removal {
                        fatal.push(
                            Diagnostic::error(
                                codes::MANIFEST_UNTRACKED,
                                format!(
                                    "data root `{key}` was removed without a tracked rename; \
                                     pass --allow-untracked-removal or --cold"
                                ),
                            )
                            .with_got_id(info.got_id)
                            .with_old_key(key.clone()),
                        );
                    }
                    history
                        .retired
                        .push(retired(index, info, RetiredCause::Dropped));
                }
                IdentityKind::Function
                | IdentityKind::Type
                | IdentityKind::Descriptor
                | IdentityKind::Instance
                | IdentityKind::Task
                | IdentityKind::Field => {
                    actions.push(ActionRecord {
                        got_id: info.got_id,
                        kind: kind_tag.to_owned(),
                        key: key.clone(),
                        old_key: None,
                        action: PlanAction::Drop,
                        reason_code: "drop.retired".into(),
                        detail: format!("{} `{key}` was removed", kind.name()),
                    });
                    history
                        .retired
                        .push(retired(index, info, RetiredCause::Dropped));
                }
            }
        }
    }
    history.retired.sort_by(|left, right| {
        (left.retired_build_id, left.got_id).cmp(&(right.retired_build_id, right.got_id))
    });

    if !fatal.is_empty() {
        diagnostics.extend(fatal);
        diagnostics.sort_by(compare_diagnostics);
        return Err(BuildError { diagnostics });
    }

    // Slots are assigned after every identity is known.
    let mut slots = Vec::new();
    let mut next_slot = 0_u32;
    assign_root_slots(&mut roots, false, &mut next_slot, &mut slots);
    let mut function_order: Vec<usize> = (0..functions.len()).collect();
    function_order.sort_by_key(|index| functions[*index].got_id);
    for index in function_order {
        functions[index].slot = next_slot;
        slots.push(SlotRecord {
            slot: next_slot,
            kind: SlotKind::Function,
            service: 0,
            got_id: functions[index].got_id,
        });
        next_slot += 1;
    }
    let mut descriptor_order: Vec<usize> = (0..descriptors.len()).collect();
    descriptor_order.sort_by_key(|index| descriptors[*index].got_id);
    for index in descriptor_order {
        descriptors[index].slot = next_slot;
        slots.push(SlotRecord {
            slot: next_slot,
            kind: SlotKind::Descriptor,
            service: 0,
            got_id: descriptors[index].got_id,
        });
        next_slot += 1;
    }
    assign_root_slots(&mut roots, true, &mut next_slot, &mut slots);
    let imports: BTreeSet<ImportService> = catalog.imports.iter().copied().collect();
    for service in imports {
        slots.push(SlotRecord {
            slot: next_slot,
            kind: SlotKind::Import,
            service: service.id(),
            got_id: GotId::ZERO,
        });
        next_slot += 1;
    }

    roots.sort_by(|left, right| {
        (left.key.as_str(), left.got_id).cmp(&(right.key.as_str(), right.got_id))
    });
    functions.sort_by(|left, right| {
        (left.key.as_str(), left.signature_key.as_str(), left.got_id).cmp(&(
            right.key.as_str(),
            right.signature_key.as_str(),
            right.got_id,
        ))
    });
    types.sort_by(|left, right| {
        (left.key.as_str(), left.got_id).cmp(&(right.key.as_str(), right.got_id))
    });
    descriptors.sort_by(|left, right| {
        (left.key.as_str(), left.got_id).cmp(&(right.key.as_str(), right.got_id))
    });
    instances.sort_by(|left, right| {
        (left.key.as_str(), left.got_id).cmp(&(right.key.as_str(), right.got_id))
    });
    tasks.sort_by(|left, right| {
        (left.key.as_str(), left.got_id).cmp(&(right.key.as_str(), right.got_id))
    });
    exports.sort_by(|left, right| {
        (left.c_name.as_str(), left.got_id).cmp(&(right.c_name.as_str(), right.got_id))
    });
    actions.sort_by_key(|action| action.got_id);
    diagnostics.sort_by(compare_diagnostics);

    let rejected = actions
        .iter()
        .any(|action| action.action == PlanAction::Reject)
        || diagnostics.iter().any(Diagnostic::is_error);
    let verdict = if !online {
        PlanVerdict::Cold
    } else if rejected {
        PlanVerdict::Rejected
    } else {
        PlanVerdict::OnlineCandidate
    };

    let plan_diagnostics: Vec<PlanDiagnostic> = diagnostics
        .iter()
        .map(|diagnostic| PlanDiagnostic {
            code: diagnostic.code.to_owned(),
            severity: diagnostic.severity.tag(),
            message: diagnostic.message.clone(),
            got_id: diagnostic.got_id,
            old_key: diagnostic.old_key.clone(),
            new_key: diagnostic.new_key.clone(),
        })
        .collect();
    let plan_diagnostics = {
        let mut values = plan_diagnostics;
        values.sort_by(compare_plan_diagnostics);
        values
    };

    let mut generator_flags = input.generator_flags;
    if options.allow_untracked_removal {
        generator_flags |= GENERATOR_UNTRACKED_ALLOWED;
    }
    if !online {
        generator_flags |= GENERATOR_COLD_LAYOUT;
    }

    let parent_build_id = baseline.as_ref().map(|index| index.build_id);
    let parent_manifest_digest = baseline.as_ref().map(|index| index.digest);
    let plan = Plan {
        verdict,
        parent_build_id,
        parent_manifest_digest,
        actions,
        diagnostics: plan_diagnostics,
    };
    let mut manifest = Manifest {
        header: Header {
            profile: input.profile.clone(),
            compiler_version: input.compiler_version.clone(),
            target_triple: input.target.triple.clone(),
            module_name: input.module_name.clone(),
            active_configuration: input.active_configuration.clone(),
            endianness: input.target.endianness.tag(),
            pointer_width: input.target.pointer_width,
            module_format: input.module_format,
            host_abi_major: input.host_abi_major,
            host_abi_minor: input.host_abi_minor,
            generator_flags,
            required_features: input.required_features | FEATURE_HOST_ABI | FEATURE_ONLINE_PLAN,
            build_id: GotId::ZERO,
            parent_build_id,
            parent_manifest_digest,
            descriptor_digest: input.descriptor_digest,
            source_fingerprint: input.source_fingerprint,
            catalog_digest: Digest32::ZERO,
            plan_digest: Digest32::ZERO,
        },
        roots,
        functions,
        types,
        descriptors,
        instances,
        tasks,
        exports,
        slots,
        plan,
        history,
    };
    let catalog_digest = manifest.catalog_digest().map_err(BuildError::from_error)?;
    let plan_digest = manifest.plan_digest().map_err(BuildError::from_error)?;
    manifest.header.catalog_digest = catalog_digest;
    manifest.header.plan_digest = plan_digest;
    let build_id = compute_build_id(
        input,
        manifest.header.generator_flags,
        manifest.header.parent_build_id,
        manifest.header.parent_manifest_digest,
        catalog_digest,
        plan_digest,
    )?;
    manifest.header.build_id = build_id;
    manifest.validate().map_err(BuildError::from_error)?;
    let binary = manifest.encode_binary().map_err(BuildError::from_error)?;
    let json = manifest.to_json_text().map_err(BuildError::from_error)?;
    Ok(BuildOutcome {
        manifest,
        binary,
        json,
        diagnostics,
    })
}

fn validate_input(input: &BuildInput) -> std::result::Result<(), BuildError> {
    if input.profile.is_empty() {
        return Err(BuildError::single(Diagnostic::error(
            codes::MANIFEST_INVALID,
            "build profile is empty",
        )));
    }
    if input.target.triple.is_empty() {
        return Err(BuildError::single(Diagnostic::error(
            codes::MANIFEST_INVALID,
            "build target triple is empty",
        )));
    }
    if input.target.pointer_width != 8 {
        return Err(BuildError::single(Diagnostic::error(
            codes::MANIFEST_INVALID,
            "format 1 supports 8-byte pointers only",
        )));
    }
    if input.descriptor_digest.is_zero() {
        return Err(BuildError::single(Diagnostic::error(
            codes::MANIFEST_INVALID,
            "descriptor digest is zero; build the descriptor before the manifest",
        )));
    }
    if input.source_fingerprint.is_zero() {
        return Err(BuildError::single(Diagnostic::error(
            codes::MANIFEST_INVALID,
            "source fingerprint is zero",
        )));
    }
    Ok(())
}

fn establish_baseline(
    input: &BuildInput,
    options: &BuildOptions<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<BaselineIndex> {
    let Some(baseline) = options.baseline else {
        diagnostics.push(Diagnostic::warning(
            codes::W_MANIFEST_COLD,
            "no baseline manifest; the build uses a cold layout",
        ));
        return None;
    };
    if options.cold {
        diagnostics.push(Diagnostic::warning(
            codes::W_MANIFEST_COLD,
            "cold layout forced; the supplied baseline was ignored",
        ));
        return None;
    }
    if let Err(error) = baseline.validate() {
        diagnostics.push(Diagnostic::warning(
            codes::W_ONLINE_STALE_BASELINE,
            format!("baseline manifest is invalid; cold layout selected: {error}"),
        ));
        return None;
    }
    if let Err(error) = baseline.validate_for(
        &input.profile,
        &input.target,
        input.host_abi_major,
        input.host_abi_minor,
    ) {
        diagnostics.push(Diagnostic::warning(
            codes::W_ONLINE_STALE_BASELINE,
            format!("baseline manifest is not usable; cold layout selected: {error}"),
        ));
        return None;
    }
    match BaselineIndex::new(baseline) {
        Ok(index) => Some(index),
        Err(error) => {
            diagnostics.push(Diagnostic::warning(
                codes::W_ONLINE_STALE_BASELINE,
                format!("baseline manifest is invalid; cold layout selected: {error}"),
            ));
            None
        }
    }
}

fn resolve(
    baseline: Option<&mut BaselineIndex>,
    renames: &[Rename],
    kind: IdentityKind,
    key: &str,
    identity: &Identity,
    allow_field_rename: bool,
    ids: &mut IdTables,
) -> std::result::Result<Assigned, BuildError> {
    if let Some(index) = baseline {
        if let Some(old_key) = rename_candidate(key, renames, allow_field_rename)
            && old_key != key
            && let Some(info) = index.records.get(&(kind.as_u8(), old_key.clone()))
        {
            if !index.used.insert((kind.as_u8(), old_key.clone())) {
                return Err(BuildError::single(Diagnostic::error(
                    codes::MANIFEST_PLAN,
                    format!(
                        "baseline identity `{old_key}` is matched by more than one declaration"
                    ),
                )));
            }
            let mut previous_keys = info.previous_keys.clone();
            previous_keys.push(info.key.clone());
            ids.register(info.got_id, kind, key, None)?;
            return Ok(Assigned {
                got_id: info.got_id,
                previous_keys,
                old_key: Some(info.key.clone()),
                baseline: Some(info.clone()),
            });
        }
        if let Some(info) = index.records.get(&(kind.as_u8(), key.to_owned())) {
            if !index.used.insert((kind.as_u8(), key.to_owned())) {
                return Err(BuildError::single(Diagnostic::error(
                    codes::MANIFEST_PLAN,
                    format!("baseline identity `{key}` is matched by more than one declaration"),
                )));
            }
            ids.register(info.got_id, kind, key, None)?;
            return Ok(Assigned {
                got_id: info.got_id,
                previous_keys: info.previous_keys.clone(),
                old_key: None,
                baseline: Some(info.clone()),
            });
        }
        if let Some(kinds) = index.keys_by_kind.get(key)
            && !kinds.contains(&kind.as_u8())
        {
            return Err(BuildError::single(Diagnostic::error(
                codes::MANIFEST_INVALID,
                format!("key `{key}` matches a baseline identity of a different kind"),
            )));
        }
        if let Some(old_key) = rename_candidate(key, renames, allow_field_rename)
            && let Some(kinds) = index.keys_by_kind.get(&old_key)
            && !kinds.contains(&kind.as_u8())
        {
            return Err(BuildError::single(Diagnostic::error(
                codes::MANIFEST_INVALID,
                format!("rename source `{old_key}` names a baseline identity of a different kind"),
            )));
        }
    }
    let (got_id, full) = identity.derive();
    ids.register(got_id, kind, key, Some(full))?;
    Ok(Assigned {
        got_id,
        previous_keys: Vec::new(),
        baseline: None,
        old_key: None,
    })
}

#[derive(Default)]
struct IdTables {
    identities: BTreeMap<GotId, (u8, String)>,
    full_hashes: BTreeMap<GotId, Digest32>,
}

impl IdTables {
    fn register(
        &mut self,
        got_id: GotId,
        kind: IdentityKind,
        key: &str,
        full: Option<Digest32>,
    ) -> std::result::Result<(), BuildError> {
        if let Some((existing_kind, existing_key)) = self.identities.get(&got_id) {
            if *existing_kind != kind.as_u8() || existing_key != key {
                return Err(BuildError::single(Diagnostic::error(
                    codes::MANIFEST_INVALID,
                    format!("GotId {got_id} is shared by two different identities"),
                )));
            }
        } else {
            self.identities
                .insert(got_id, (kind.as_u8(), key.to_owned()));
        }
        if let Some(full) = full {
            if let Some(existing) = self.full_hashes.get(&got_id) {
                if existing != &full {
                    return Err(BuildError::single(Diagnostic::error(
                        codes::MANIFEST_INVALID,
                        format!("two identity hashes truncate to the same GotId {got_id}"),
                    )));
                }
            } else {
                self.full_hashes.insert(got_id, full);
            }
        }
        Ok(())
    }
}

fn rename_candidate(key: &str, renames: &[Rename], allow_field_rename: bool) -> Option<String> {
    for rename in renames {
        if rename.new == key {
            return Some(rename.old.clone());
        }
    }
    let mut best: Option<(&Rename, usize)> = None;
    for rename in renames {
        if let Some(suffix) = key.strip_prefix(&rename.new)
            && !suffix.is_empty()
            && is_move_boundary(suffix)
            && best.is_none_or(|(_, length)| rename.new.len() > length)
        {
            best = Some((rename, rename.new.len()));
        }
    }
    if let Some((rename, _)) = best {
        return Some(format!("{}{}", rename.old, &key[rename.new.len()..]));
    }
    if !allow_field_rename {
        return None;
    }
    let (package, rest) = key.split_once("::")?;
    let split = rest.find(['#', '@']).unwrap_or(rest.len());
    let path = &rest[..split];
    let suffix = &rest[split..];
    for rename in renames {
        if rename.new.contains("::") {
            continue;
        }
        if path == rename.new {
            return Some(format!("{package}::{}{suffix}", rename.old));
        }
        if let Some(prefix) = path.strip_suffix(&rename.new)
            && prefix.ends_with('.')
        {
            return Some(format!("{package}::{}{}{suffix}", prefix, rename.old));
        }
    }
    None
}

fn is_move_boundary(suffix: &str) -> bool {
    matches!(suffix.as_bytes()[0], b'.' | b'#' | b'@' | b'/' | b':')
}

fn compare_diagnostics(left: &Diagnostic, right: &Diagnostic) -> std::cmp::Ordering {
    (
        left.code,
        left.got_id.is_some(),
        left.got_id.unwrap_or(GotId::ZERO),
        left.old_key.as_deref().unwrap_or(""),
        left.new_key.as_deref().unwrap_or(""),
        left.message.as_str(),
    )
        .cmp(&(
            right.code,
            right.got_id.is_some(),
            right.got_id.unwrap_or(GotId::ZERO),
            right.old_key.as_deref().unwrap_or(""),
            right.new_key.as_deref().unwrap_or(""),
            right.message.as_str(),
        ))
}

type Classification = (PlanAction, &'static str, String, Option<Diagnostic>);

fn classify_root(decl: &RootDecl, assigned: &Assigned) -> Classification {
    if decl.root_kind == RootKind::Direct {
        let warning = match &assigned.baseline {
            Some(BaselineInfo {
                shape:
                    BaselineShape::Root {
                        type_key,
                        layout_hash,
                        size,
                        align,
                        ..
                    },
                ..
            }) if type_key != &decl.type_key
                || layout_hash != &decl.layout_hash
                || *size != decl.size
                || *align != decl.align =>
            {
                Some(
                    Diagnostic::warning(
                        codes::W_ONLINE_DIRECT,
                        format!("direct variable `{}` was remapped", decl.key),
                    )
                    .with_got_id(assigned.got_id)
                    .with_new_key(decl.key.clone()),
                )
            }
            _ => None,
        };
        return (
            PlanAction::Allocate,
            "allocate.direct",
            "direct-variable cell is per generation".into(),
            warning,
        );
    }
    let Some(BaselineInfo {
        shape:
            BaselineShape::Root {
                root_kind,
                type_key,
                layout_hash,
                size,
                align,
                initializer_digest,
            },
        ..
    }) = &assigned.baseline
    else {
        return (
            PlanAction::Allocate,
            "allocate.new",
            "new persistent root".into(),
            None,
        );
    };
    if root_kind == &decl.root_kind
        && type_key == &decl.type_key
        && layout_hash == &decl.layout_hash
        && *size == decl.size
        && *align == decl.align
    {
        let warning = if initializer_digest != &decl.initializer_digest {
            Some(
                Diagnostic::warning(
                    codes::W_ONLINE_INITIALIZER,
                    format!(
                        "initializer change for preserved root `{}` is ignored",
                        decl.key
                    ),
                )
                .with_got_id(assigned.got_id)
                .with_new_key(decl.key.clone()),
            )
        } else {
            None
        };
        (
            PlanAction::Preserve,
            "preserve.exact",
            "type key, layout hash, size, and alignment match".into(),
            warning,
        )
    } else {
        (
            PlanAction::Reject,
            "reject.layout",
            format!(
                "root `{}` layout changed (type `{}` -> `{}`)",
                decl.key, type_key, decl.type_key
            ),
            None,
        )
    }
}

fn classify_function(decl: &FunctionDecl, assigned: &Assigned) -> Classification {
    let Some(BaselineInfo {
        shape: BaselineShape::Function { signature_key },
        ..
    }) = &assigned.baseline
    else {
        return (
            PlanAction::Allocate,
            "allocate.new",
            "new callable".into(),
            None,
        );
    };
    if signature_key == &decl.signature_key {
        (
            PlanAction::Preserve,
            "preserve.callable",
            "callable signature matches".into(),
            None,
        )
    } else {
        (
            PlanAction::Reject,
            "reject.export",
            format!("signature changed for callable `{}`", decl.key),
            None,
        )
    }
}

fn classify_type(
    decl: &TypeDecl,
    assigned: &Assigned,
    resolved_base: Option<GotId>,
    resolved_interfaces: &[GotId],
) -> Classification {
    let Some(BaselineInfo {
        shape:
            BaselineShape::Type {
                type_key,
                layout_hash,
                size,
                align,
                vtable_shape,
                base,
                interfaces,
                ..
            },
        ..
    }) = &assigned.baseline
    else {
        return (
            PlanAction::Allocate,
            "allocate.new",
            "new type".into(),
            None,
        );
    };
    if vtable_shape != &decl.vtable_shape {
        return (
            PlanAction::Reject,
            "reject.vtable",
            format!("vtable shape changed for type `{}`", decl.key),
            None,
        );
    }
    if type_key == &decl.type_key
        && layout_hash == &decl.layout_hash
        && *size == decl.size
        && *align == decl.align
        && base == &resolved_base
        && interfaces.as_slice() == resolved_interfaces
    {
        (
            PlanAction::Preserve,
            "preserve.exact",
            "type layout matches".into(),
            None,
        )
    } else {
        (
            PlanAction::Reject,
            "reject.layout",
            format!("type layout changed for `{}`", decl.key),
            None,
        )
    }
}

fn classify_descriptor(
    decl: &DescriptorDecl,
    assigned: &Assigned,
    resolved_owner: Option<GotId>,
) -> Classification {
    let Some(BaselineInfo {
        shape:
            BaselineShape::Descriptor {
                descriptor_kind,
                owner_type,
                type_key,
                layout_hash,
                size,
                align,
            },
        ..
    }) = &assigned.baseline
    else {
        return (
            PlanAction::Allocate,
            "allocate.new",
            "new descriptor".into(),
            None,
        );
    };
    if descriptor_kind != &decl.descriptor_kind
        || type_key != &decl.type_key
        || owner_type != &resolved_owner
    {
        return (
            PlanAction::Reject,
            "reject.vtable",
            format!("descriptor shape changed for `{}`", decl.key),
            None,
        );
    }
    if layout_hash == &decl.layout_hash && *size == decl.size && *align == decl.align {
        (
            PlanAction::Preserve,
            "preserve.exact",
            "descriptor layout matches".into(),
            None,
        )
    } else {
        (
            PlanAction::Reject,
            "reject.layout",
            format!("descriptor layout changed for `{}`", decl.key),
            None,
        )
    }
}

fn classify_instance(
    decl: &InstanceDecl,
    assigned: &Assigned,
    resolved_program: Option<GotId>,
) -> Classification {
    let Some(BaselineInfo {
        shape:
            BaselineShape::Instance {
                program_type,
                program_type_key,
                configuration,
                resource,
                instance_path,
                task_key,
            },
        ..
    }) = &assigned.baseline
    else {
        return (
            PlanAction::Allocate,
            "allocate.new",
            "new instance".into(),
            None,
        );
    };
    if program_type_key == &decl.program_type_key
        && configuration == &decl.configuration
        && resource == &decl.resource
        && instance_path == &decl.instance_path
        && task_key == &decl.task_key
        && program_type == &resolved_program
    {
        (
            PlanAction::Preserve,
            "preserve.instance",
            "instance binding matches".into(),
            None,
        )
    } else {
        (
            PlanAction::Reject,
            "reject.topology",
            format!("instance topology changed for `{}`", decl.key),
            None,
        )
    }
}

fn classify_task(decl: &TaskDecl, assigned: &Assigned) -> Classification {
    let Some(BaselineInfo {
        shape:
            BaselineShape::Task {
                configuration,
                resource,
                name,
                single,
                interval_ns,
                priority,
            },
        ..
    }) = &assigned.baseline
    else {
        return (
            PlanAction::Allocate,
            "allocate.new",
            "new task".into(),
            None,
        );
    };
    if configuration == &decl.configuration
        && resource == &decl.resource
        && name == &decl.name
        && *single == decl.single
        && *interval_ns == decl.interval_ns
        && *priority == decl.priority
    {
        (
            PlanAction::Preserve,
            "preserve.task",
            "task attributes match".into(),
            None,
        )
    } else {
        (
            PlanAction::Reject,
            "reject.topology",
            format!("task topology changed for `{}`", decl.key),
            None,
        )
    }
}

fn classify_export(
    decl: &ExportDecl,
    assigned: &Assigned,
    function_ids: &BTreeMap<String, GotId>,
) -> Classification {
    let Some(BaselineInfo {
        shape:
            BaselineShape::Export {
                c_name,
                function_got_id,
                signature_key,
                no_mangle,
            },
        ..
    }) = &assigned.baseline
    else {
        return (
            PlanAction::Allocate,
            "allocate.new",
            "new export".into(),
            None,
        );
    };
    let current_function = function_ids.get(&decl.function_key);
    if c_name == &decl.c_name
        && signature_key == &decl.signature_key
        && no_mangle == &decl.no_mangle
        && current_function == Some(function_got_id)
    {
        (
            PlanAction::Preserve,
            "preserve.export",
            "export signature matches".into(),
            None,
        )
    } else {
        (
            PlanAction::Reject,
            "reject.export",
            format!("export `{}` changed its ABI", decl.key),
            None,
        )
    }
}

fn resolved_type(
    types: &BTreeMap<String, GotId>,
    key: &str,
    owner: &str,
) -> std::result::Result<GotId, BuildError> {
    types.get(key).copied().ok_or_else(|| {
        BuildError::single(Diagnostic::error(
            codes::MANIFEST_INVALID,
            format!("`{owner}` references an unknown type `{key}`"),
        ))
    })
}

fn resolve_type(
    types: &BTreeMap<String, GotId>,
    key: Option<&str>,
    owner: &str,
) -> std::result::Result<Option<GotId>, BuildError> {
    match key {
        Some(key) => Ok(Some(resolved_type(types, key, owner)?)),
        None => Ok(None),
    }
}

fn assign_root_slots(
    roots: &mut [RootRecord],
    direct: bool,
    next_slot: &mut u32,
    slots: &mut Vec<SlotRecord>,
) {
    let mut order: Vec<usize> = roots
        .iter()
        .enumerate()
        .filter(|(_, root)| (root.root_kind == RootKind::Direct) == direct)
        .map(|(index, _)| index)
        .collect();
    order.sort_by_key(|index| roots[*index].got_id);
    for index in order {
        roots[index].slot = *next_slot;
        slots.push(SlotRecord {
            slot: *next_slot,
            kind: if direct {
                SlotKind::Direct
            } else {
                SlotKind::Data
            },
            service: 0,
            got_id: roots[index].got_id,
        });
        *next_slot += 1;
    }
}

fn retired(index: &BaselineIndex, info: &BaselineInfo, cause: RetiredCause) -> RetiredRecord {
    RetiredRecord {
        got_id: info.got_id,
        kind: info.kind,
        key: info.key.clone(),
        previous_keys: info.previous_keys.clone(),
        retired_build_id: index.build_id,
        retired_manifest_digest: index.digest,
        cause,
    }
}

fn compute_build_id(
    input: &BuildInput,
    generator_flags: u64,
    parent_build_id: Option<GotId>,
    parent_manifest_digest: Option<Digest32>,
    catalog_digest: Digest32,
    plan_digest: Digest32,
) -> std::result::Result<GotId, BuildError> {
    let mut encoded = Encoder::new();
    encoded
        .string(&input.profile)
        .map_err(BuildError::from_error)?;
    encoded
        .string(&input.target.triple)
        .map_err(BuildError::from_error)?;
    encoded
        .string(&input.compiler_version)
        .map_err(BuildError::from_error)?;
    encoded
        .string(&input.active_configuration)
        .map_err(BuildError::from_error)?;
    encoded.u8(input.target.endianness.tag());
    encoded.u8(input.target.pointer_width);
    encoded.u16(input.module_format);
    encoded.u16(input.host_abi_major);
    encoded.u16(input.host_abi_minor);
    encoded.u64(generator_flags);
    encoded.digest32(input.source_fingerprint);
    match (parent_build_id, parent_manifest_digest) {
        (Some(build), Some(digest)) => {
            encoded.u8(1);
            encoded.id128(build);
            encoded.digest32(digest);
        }
        _ => encoded.u8(0),
    }
    encoded.digest32(catalog_digest);
    encoded.digest32(plan_digest);
    let digest = domain_hash("stc:build:v1", &[encoded.as_bytes()]);
    let mut truncated = [0_u8; 16];
    truncated.copy_from_slice(&digest[..16]);
    let build_id = GotId::from_bytes(truncated);
    if build_id.is_zero() {
        return Err(BuildError::single(Diagnostic::error(
            codes::MANIFEST_INVALID,
            "derived build ID is zero",
        )));
    }
    Ok(build_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_paths_are_validated() {
        assert!(validate_source_path("_ROOT/main.st").is_ok());
        assert!(validate_source_path("/main.st").is_err());
        assert!(validate_source_path("C:/main.st").is_err());
        assert!(validate_source_path("a/../b.st").is_err());
        assert!(validate_source_path("a\\b.st").is_err());
    }

    #[test]
    fn rename_candidates_cover_exact_prefix_and_field_forms() {
        let renames = vec![
            Rename::new("_ROOT::OLD", "_ROOT::NEW"),
            Rename::new("_ROOT::MAIN.SPEED", "_ROOT::MAIN.RATE"),
        ];
        assert_eq!(
            rename_candidate("_ROOT::NEW", &renames, false).as_deref(),
            Some("_ROOT::OLD")
        );
        assert_eq!(
            rename_candidate("_ROOT::NEW.SUB", &renames, false).as_deref(),
            Some("_ROOT::OLD.SUB")
        );
        assert_eq!(
            rename_candidate("_ROOT::MAIN.RATE#MAIN/R1/INST", &renames, true).as_deref(),
            Some("_ROOT::MAIN.SPEED#MAIN/R1/INST")
        );
        assert!(rename_candidate("_ROOT::NEWER", &renames, false).is_none());
    }

    #[test]
    fn source_fingerprint_is_order_independent() {
        let first = source_fingerprint(&[
            ("_ROOT/b.st".into(), b"b".to_vec()),
            ("_ROOT/a.st".into(), b"a".to_vec()),
        ])
        .unwrap();
        let second = source_fingerprint(&[
            ("_ROOT/a.st".into(), b"a".to_vec()),
            ("_ROOT/b.st".into(), b"b".to_vec()),
        ])
        .unwrap();
        assert_eq!(first, second);
        let changed = source_fingerprint(&[("_ROOT/a.st".into(), b"c".to_vec())]).unwrap();
        assert_ne!(first, changed);
    }
}
