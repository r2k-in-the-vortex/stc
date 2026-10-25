//! Module artifact projection for the M4.5 online-change foundation.
//!
//! The compiler builds one catalog from the selected plan, the scalar layouts,
//! and the process-image/retained-state layouts. The identity engine in
//! `stc-got` matches it against the baseline, assigns canonical slots, and
//! produces the manifest and online-change plan; this module adds the module
//! descriptor and the slot bindings the module-shaped code generator and the
//! host consume (GOT Artifacts §6–§7 and §11).
//!
//! Artifacts are projections of one deterministic model. Repeated builds of
//! the same source, target, and baseline produce byte-identical descriptors,
//! binary manifests, canonical JSON manifests, and build IDs.

use std::collections::{BTreeMap, BTreeSet};

use stc_got::catalog::ImportService;
use stc_got::descriptor::{Descriptor, DescriptorFunction, entry_flag, function_flag};
use stc_got::manifest::{FEATURE_HOST_ABI, FEATURE_ONLINE_PLAN};
use stc_got::model::{Retention, RootKind, SourceLocation};
use stc_got::{
    BuildInput, BuildOptions, Catalog, Digest32, ExportDecl, FunctionDecl, FunctionKind, GotId,
    Identity, IdentityKind, InitializerKind, InstanceDecl, Manifest, Rename, RootDecl, Signature,
    Target, TaskDecl, TypeKey, direct_root_key, elementary_layout, elementary_layout_hash,
    export_identity_key, expression_initializer_digest, field_root_key, function_key,
    global_root_key, initializer_digest, instance_identity_key, normalize_ident, normalize_package,
    program_body_key, task_guard_root_key, task_identity_key,
};
use stc_hir::{
    ArgDirection, CalleeHir, ConstValue, DefId, DefKind, ExprId, ExprKindHir, FieldId, HirCrate,
    PlaceHir, RetainPolicy, Symbol, Ty, TyId,
};
use stc_span::{FileId, Span};

use crate::CodegenError;
use crate::codes;
use crate::config::{self, Plan, PlanTask, RetainLayout};
use crate::ir::CellKind;
use crate::layout;
use crate::mangle;

/// The conformance profile embedded in module artifacts.
pub const PROFILE: &str = "stc-2013-oop-utf8-v1";

/// Synthesized configuration/resource/task name for a configuration-less build.
pub const SYNTHETIC_NAME: &str = "_DEFAULT";

/// The package name used by the manifest-free M4 bridge.
pub const M4_PACKAGE: &str = "_ROOT";

/// Placeholder descriptor digest used before the real descriptor exists.
///
/// The build ID, catalog digest, and plan digest do not depend on the
/// descriptor digest, so the manifest is built once and its header field is
/// replaced after the descriptor is encoded (GOT Artifacts §11.2, §11.4).
const PLACEHOLDER_DESCRIPTOR_DIGEST: Digest32 = Digest32::from_bytes([0x5A; 32]);

/// Severity of a module projection diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// A non-fatal condition.
    Warning,
    /// A condition that makes the artifacts undeployable.
    Error,
}

impl Severity {
    /// Returns `true` for errors.
    #[must_use]
    pub const fn is_error(self) -> bool {
        matches!(self, Self::Error)
    }
}

/// One compiler diagnostic produced while projecting module artifacts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleError {
    /// Stable diagnostic code.
    pub code: &'static str,
    /// Warning or error.
    pub severity: Severity,
    /// Human-readable message.
    pub message: String,
    /// Source span, when the diagnostic has a declaration.
    pub span: Option<Span>,
    /// Affected identity, when known.
    pub got_id: Option<GotId>,
    /// Baseline key, when relevant.
    pub old_key: Option<Box<str>>,
    /// New key, when relevant.
    pub new_key: Option<Box<str>>,
}

impl ModuleError {
    fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Error,
            message: message.into(),
            span: None,
            got_id: None,
            old_key: None,
            new_key: None,
        }
    }

    /// Returns `true` for errors.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity.is_error()
    }
}

impl From<CodegenError> for ModuleError {
    fn from(error: CodegenError) -> Self {
        Self {
            code: error.code(),
            severity: Severity::Error,
            message: error.message().to_owned(),
            span: error.span(),
            got_id: None,
            old_key: None,
            new_key: None,
        }
    }
}

impl From<stc_got::Diagnostic> for ModuleError {
    fn from(diagnostic: stc_got::Diagnostic) -> Self {
        Self {
            code: diagnostic.code,
            severity: match diagnostic.severity {
                stc_got::Severity::Warning => Severity::Warning,
                stc_got::Severity::Error => Severity::Error,
            },
            message: diagnostic.message,
            span: None,
            got_id: diagnostic.got_id,
            old_key: diagnostic.old_key.map(Into::into),
            new_key: diagnostic.new_key.map(Into::into),
        }
    }
}

/// Which HIR declaration owns a data or direct root.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RootTarget {
    /// A scalar global.
    Global(DefId),
    /// A program-instance scalar field.
    InstanceField {
        /// Configured instance name. A configuration-less build uses the
        /// reserved `Symbol::new(u32::MAX)` carried by the synthesized plan.
        instance: Symbol,
        /// Field declaration.
        field: FieldId,
    },
    /// A task guard flag.
    TaskGuard {
        /// Configuration name.
        configuration: String,
        /// Resource name.
        resource: String,
        /// Task name.
        task: String,
    },
}

/// Which compiled body owns a function cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FunctionTarget {
    /// A user function body.
    Function(DefId),
    /// A program body.
    ProgramBody(DefId),
}

/// One canonical GOT slot binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotBinding {
    /// Slot number.
    pub slot: u32,
    /// Cell kind.
    pub kind: CellKind,
    /// Identity key, empty for imports.
    pub key: String,
    /// Identity, zero for imports.
    pub got_id: GotId,
    /// Host service for import slots.
    pub service: Option<ImportService>,
    /// Owning root declaration, for data and direct slots.
    pub root: Option<RootTarget>,
    /// Owning callable, for function slots.
    pub function: Option<FunctionTarget>,
    /// M4 code symbol, when the slot has one.
    pub symbol: Option<String>,
}

/// Inputs that select the module target and identity baseline.
#[derive(Clone, Debug)]
pub struct ModuleOptions<'a> {
    /// Resolved target triple, byte order, and pointer width.
    pub target: Target,
    /// Artifact and manifest module name.
    pub module_name: String,
    /// Compiler package version.
    pub compiler_version: String,
    /// Hash of the production source files.
    pub source_fingerprint: Digest32,
    /// Optimised, stripped build (`--release`).
    pub release: bool,
    /// Relaxed syntax was accepted.
    pub relaxed: bool,
    /// Optional identity baseline.
    pub baseline: Option<&'a Manifest>,
    /// Force a cold-layout build and ignore the baseline.
    pub cold: bool,
    /// Permit untracked removal of data roots.
    pub allow_untracked_removal: bool,
    /// Tracked rename/move maps.
    pub renames: Vec<Rename>,
}

impl<'a> ModuleOptions<'a> {
    /// Creates options for a cold-capable build.
    #[must_use]
    pub fn new(
        target: Target,
        module_name: impl Into<String>,
        compiler_version: impl Into<String>,
        source_fingerprint: Digest32,
    ) -> Self {
        Self {
            target,
            module_name: module_name.into(),
            compiler_version: compiler_version.into(),
            source_fingerprint,
            release: false,
            relaxed: false,
            baseline: None,
            cold: false,
            allow_untracked_removal: false,
            renames: Vec::new(),
        }
    }
}

/// The compiler-side module artifacts for one build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleArtifacts {
    /// The compiler catalog projection.
    pub catalog: Catalog,
    /// The validated GOT manifest.
    pub manifest: Manifest,
    /// Compact binary manifest bytes.
    pub manifest_binary: Vec<u8>,
    /// Canonical JSON manifest text.
    pub manifest_json: String,
    /// The validated module descriptor.
    pub descriptor: Descriptor,
    /// Encoded descriptor bytes embedded in the module.
    pub descriptor_bytes: Vec<u8>,
    /// Descriptor digest (computed over the zero-manifest-digest projection).
    pub descriptor_digest: Digest32,
    /// Manifest digest.
    pub manifest_digest: Digest32,
    /// Canonical slot bindings.
    pub slots: Vec<SlotBinding>,
    /// Compiler diagnostics; errors make the artifacts undeployable.
    pub diagnostics: Vec<ModuleError>,
}

impl ModuleArtifacts {
    /// Returns the plan verdict.
    #[must_use]
    pub fn verdict(&self) -> stc_got::PlanVerdict {
        self.manifest.plan.verdict
    }

    /// Returns `true` when the artifacts may be deployed.
    #[must_use]
    pub fn is_deployable(&self) -> bool {
        self.verdict() != stc_got::PlanVerdict::Rejected
            && !self.diagnostics.iter().any(ModuleError::is_error)
    }

    /// Returns the cell kind for every slot, in slot order.
    #[must_use]
    pub fn slot_kinds(&self) -> Vec<CellKind> {
        self.slots.iter().map(|binding| binding.kind).collect()
    }

    /// Returns the slot for a root declaration.
    #[must_use]
    pub fn root_slot(&self, target: &RootTarget) -> Option<u32> {
        self.slots
            .iter()
            .find(|binding| binding.root.as_ref() == Some(target))
            .map(|binding| binding.slot)
    }

    /// Returns the slot for a compiled body.
    #[must_use]
    pub fn function_slot(&self, target: FunctionTarget) -> Option<u32> {
        self.slots
            .iter()
            .find(|binding| binding.function == Some(target))
            .map(|binding| binding.slot)
    }

    /// Returns the slot for a host service.
    #[must_use]
    pub fn import_slot(&self, service: ImportService) -> Option<u32> {
        self.slots
            .iter()
            .find(|binding| binding.service == Some(service))
            .map(|binding| binding.slot)
    }

    /// Returns the slot binding for a stable key.
    #[must_use]
    pub fn slot_for_key(&self, key: &str) -> Option<&SlotBinding> {
        self.slots
            .iter()
            .find(|binding| !binding.key.is_empty() && binding.key == key)
    }
}

/// Projects the M4 scalar surface to module artifacts.
///
/// `sources` is in `FileId` order and is used only for package-relative
/// source locations.
///
/// # Errors
///
/// Returns every catalog, identity, layout, or artifact-validation failure.
/// Non-fatal warnings are returned in [`ModuleArtifacts::diagnostics`].
pub fn project(
    hir: &HirCrate,
    sources: &[String],
    options: &ModuleOptions<'_>,
) -> Result<ModuleArtifacts, Vec<ModuleError>> {
    hir.check_invariants()
        .map_err(|message| vec![ModuleError::error(codes::CODEGEN_INTERNAL, message)])?;
    if options.module_name.is_empty() {
        return Err(vec![ModuleError::error(
            codes::BUILD_OPTIONS,
            "module name is empty",
        )]);
    }
    if options.compiler_version.is_empty() {
        return Err(vec![ModuleError::error(
            codes::BUILD_OPTIONS,
            "compiler version is empty",
        )]);
    }
    let plan = config::select_plan(hir).map_err(map_codegen_errors)?;
    let layouts = config::program_layouts(hir, &plan).map_err(map_codegen_errors)?;
    let _image = config::image_layout(hir, &plan, &layouts).map_err(map_codegen_errors)?;
    let retain = config::retain_layout(hir, &plan, &layouts).map_err(map_codegen_errors)?;
    let source_paths = package_relative_paths(sources);

    let context = Context {
        hir,
        plan: &plan,
        retain: &retain,
        source_paths: &source_paths,
        target: &options.target,
    };
    let built = context.build_catalog().map_err(|error| vec![error])?;

    let mut input = BuildInput::new(
        PROFILE,
        options.compiler_version.clone(),
        options.target.clone(),
        options.module_name.clone(),
    );
    input.active_configuration = plan.configuration.clone().unwrap_or_default();
    input.generator_flags = base_generator_flags(options);
    input.required_features = FEATURE_HOST_ABI | FEATURE_ONLINE_PLAN;
    input.descriptor_digest = PLACEHOLDER_DESCRIPTOR_DIGEST;
    input.source_fingerprint = options.source_fingerprint;

    let build_options = BuildOptions {
        baseline: options.baseline,
        cold: options.cold,
        allow_untracked_removal: options.allow_untracked_removal,
        renames: options.renames.clone(),
    };
    let outcome = stc_got::build(&built.catalog, &input, &build_options).map_err(|error| {
        error
            .diagnostics
            .into_iter()
            .map(ModuleError::from)
            .collect::<Vec<_>>()
    })?;
    let diagnostics = outcome
        .diagnostics
        .iter()
        .cloned()
        .map(ModuleError::from)
        .collect::<Vec<_>>();

    let mut manifest = outcome.manifest;
    let mut descriptor = build_descriptor(&manifest, &options.target, options, &retain);
    let descriptor_digest = descriptor.digest().map_err(map_got_error)?;
    manifest.header.descriptor_digest = descriptor_digest;
    let manifest_digest = manifest.digest().map_err(map_got_error)?;
    descriptor.manifest_digest = manifest_digest;
    descriptor.validate().map_err(map_got_error)?;
    descriptor
        .validate_against_manifest(&manifest)
        .map_err(map_got_error)?;
    if descriptor.digest().map_err(map_got_error)? != descriptor_digest {
        return Err(vec![ModuleError::error(
            codes::CODEGEN_INTERNAL,
            "descriptor digest changed when the manifest digest was sealed",
        )]);
    }
    manifest.validate().map_err(map_got_error)?;
    let descriptor_bytes = descriptor.encode().map_err(map_got_error)?;
    let manifest_binary = manifest.encode_binary().map_err(map_got_error)?;
    let manifest_json = manifest.to_json_text().map_err(map_got_error)?;
    let slots = built.bind_slots(&manifest).map_err(|error| vec![error])?;

    Ok(ModuleArtifacts {
        catalog: built.catalog,
        manifest,
        manifest_binary,
        manifest_json,
        descriptor,
        descriptor_bytes,
        descriptor_digest,
        manifest_digest,
        slots,
        diagnostics,
    })
}

fn map_codegen_errors(errors: Vec<CodegenError>) -> Vec<ModuleError> {
    errors.into_iter().map(ModuleError::from).collect()
}

fn map_got_error(error: stc_got::Error) -> Vec<ModuleError> {
    vec![ModuleError::error(error.code, error.message)]
}

fn base_generator_flags(options: &ModuleOptions<'_>) -> u64 {
    let mut flags = if options.release { 1 << 1 } else { 1 << 0 };
    if options.relaxed {
        flags |= 1 << 2;
    }
    flags
}

struct BuiltCatalog {
    catalog: Catalog,
    root_targets: BTreeMap<String, (RootTarget, Option<String>)>,
    function_targets: BTreeMap<String, (FunctionTarget, Option<String>)>,
}

impl BuiltCatalog {
    fn bind_slots(&self, manifest: &Manifest) -> Result<Vec<SlotBinding>, ModuleError> {
        let roots = manifest
            .roots
            .iter()
            .map(|root| (root.got_id, root))
            .collect::<BTreeMap<_, _>>();
        let functions = manifest
            .functions
            .iter()
            .map(|function| (function.got_id, function))
            .collect::<BTreeMap<_, _>>();
        let mut bindings = Vec::with_capacity(manifest.slots.len());
        for slot in &manifest.slots {
            let (key, root, function, service, symbol) = match slot.kind {
                stc_got::SlotKind::Data | stc_got::SlotKind::Direct => {
                    let record = roots.get(&slot.got_id).ok_or_else(|| {
                        ModuleError::error(
                            codes::CODEGEN_INTERNAL,
                            format!("slot {} references an unknown root", slot.slot),
                        )
                    })?;
                    let (target, symbol) =
                        self.root_targets.get(&record.key).cloned().ok_or_else(|| {
                            ModuleError::error(
                                codes::CODEGEN_INTERNAL,
                                format!("root `{}` has no catalog target", record.key),
                            )
                        })?;
                    (record.key.clone(), Some(target), None, None, symbol)
                }
                stc_got::SlotKind::Function => {
                    let record = functions.get(&slot.got_id).ok_or_else(|| {
                        ModuleError::error(
                            codes::CODEGEN_INTERNAL,
                            format!("slot {} references an unknown function", slot.slot),
                        )
                    })?;
                    let (target, symbol) = self
                        .function_targets
                        .get(&record.key)
                        .cloned()
                        .ok_or_else(|| {
                            ModuleError::error(
                                codes::CODEGEN_INTERNAL,
                                format!("function `{}` has no catalog target", record.key),
                            )
                        })?;
                    (record.key.clone(), None, Some(target), None, symbol)
                }
                stc_got::SlotKind::Descriptor => (String::new(), None, None, None, None),
                stc_got::SlotKind::Import => {
                    let service = ImportService::from_id(slot.service).ok_or_else(|| {
                        ModuleError::error(
                            codes::CODEGEN_INTERNAL,
                            format!("slot {} has unknown service {}", slot.slot, slot.service),
                        )
                    })?;
                    (String::new(), None, None, Some(service), None)
                }
            };
            bindings.push(SlotBinding {
                slot: slot.slot,
                kind: cell_kind(slot.kind),
                key,
                got_id: slot.got_id,
                service,
                root,
                function,
                symbol,
            });
        }
        Ok(bindings)
    }
}

const fn cell_kind(kind: stc_got::SlotKind) -> CellKind {
    match kind {
        stc_got::SlotKind::Data => CellKind::Data,
        stc_got::SlotKind::Function => CellKind::Function,
        stc_got::SlotKind::Descriptor => CellKind::Descriptor,
        stc_got::SlotKind::Direct => CellKind::Direct,
        stc_got::SlotKind::Import => CellKind::Import,
    }
}

fn build_descriptor(
    manifest: &Manifest,
    target: &Target,
    options: &ModuleOptions<'_>,
    retain: &RetainLayout,
) -> Descriptor {
    let mut functions = manifest.functions.iter().collect::<Vec<_>>();
    functions.sort_by_key(|function| function.got_id);
    let functions = functions
        .into_iter()
        .map(|function| DescriptorFunction {
            got_id: function.got_id,
            key: function.key.clone(),
            signature_key: function.signature_key.clone(),
            flags: function.flags,
        })
        .collect();
    let mut entry_flags = entry_flag::PREPARE_INITIALIZES | entry_flag::CYCLE_CALLABLE;
    if retain.size > 0 {
        entry_flags |= entry_flag::SHUTDOWN_RETAINED | entry_flag::RETAINED_SERVICES;
    }
    if !options.release {
        entry_flags |= entry_flag::DEBUG_BUILD;
    }
    let mut descriptor = Descriptor::new(manifest.header.build_id);
    descriptor.required_features = manifest.header.required_features;
    descriptor.host_abi_major = manifest.header.host_abi_major;
    descriptor.host_abi_minor = manifest.header.host_abi_minor;
    descriptor.module_format = manifest.header.module_format;
    descriptor.endianness = target.endianness.tag();
    descriptor.pointer_width = target.pointer_width;
    descriptor.profile = manifest.header.profile.clone();
    descriptor.compiler_version = manifest.header.compiler_version.clone();
    descriptor.target_triple = manifest.header.target_triple.clone();
    descriptor.manifest_format = stc_got::manifest::MANIFEST_FORMAT;
    descriptor.entry_flags = entry_flags;
    descriptor.functions = functions;
    descriptor.slots = manifest.slots.clone();
    descriptor
}

struct Context<'a> {
    hir: &'a HirCrate,
    plan: &'a Plan,
    retain: &'a RetainLayout,
    source_paths: &'a [String],
    target: &'a Target,
}

impl Context<'_> {
    fn build_catalog(&self) -> Result<BuiltCatalog, ModuleError> {
        let mut catalog = Catalog::default();
        let mut root_targets = BTreeMap::new();
        let mut function_targets = BTreeMap::new();
        let package = self.default_package()?;

        self.add_global_roots(&mut catalog, &mut root_targets)?;
        self.add_instance_roots(&mut catalog, &mut root_targets)?;
        self.add_task_guards(&package, &mut catalog, &mut root_targets)?;
        self.add_functions(&mut catalog, &mut function_targets)?;
        self.add_program_bodies(&mut catalog, &mut function_targets)?;
        self.add_instances(&package, &mut catalog)?;
        self.add_tasks(&package, &mut catalog)?;
        self.add_exports(&mut catalog)?;
        catalog.imports = default_imports(self.retain);
        Ok(BuiltCatalog {
            catalog,
            root_targets,
            function_targets,
        })
    }

    fn default_package(&self) -> Result<String, ModuleError> {
        let def = self.hir.entry_configuration.or_else(|| {
            self.hir.defs.iter().enumerate().find_map(|(index, def)| {
                matches!(def.kind, DefKind::Configuration { .. })
                    .then_some(DefId::new(u32::try_from(index).ok()?))
            })
        });
        match def {
            Some(def) => self.def_package(def),
            None => {
                let package = self
                    .hir
                    .packages
                    .first()
                    .ok_or_else(|| ModuleError::error(codes::CODEGEN_INTERNAL, "no package"))?;
                normalize_package(&package.name).map_err(|error| map_error(error, None))
            }
        }
    }

    fn def_package(&self, def: DefId) -> Result<String, ModuleError> {
        let package = self.hir.defs[def.index()].package;
        let package = self.hir.packages.get(package.index()).ok_or_else(|| {
            ModuleError::error(codes::CODEGEN_INTERNAL, "definition has an unknown package")
        })?;
        normalize_package(&package.name).map_err(|error| map_error(error, None))
    }

    fn add_global_roots(
        &self,
        catalog: &mut Catalog,
        root_targets: &mut BTreeMap<String, (RootTarget, Option<String>)>,
    ) -> Result<(), ModuleError> {
        for (index, def) in self.hir.defs.iter().enumerate() {
            let id = DefId::new(u32::try_from(index).unwrap_or(u32::MAX));
            if synthetic(self.hir, id) || def.test_only {
                continue;
            }
            let DefKind::Global {
                ty,
                retain,
                at,
                init,
                ..
            } = &def.kind
            else {
                continue;
            };
            let def_package = self.def_package(id)?;
            let path = self.qualified_path(id)?;
            let type_key = self.got_type_key(*ty, def.span)?;
            let (layout_hash, size, align) = self.layout(&type_key, def.span)?;
            let source = self.source_location(&def_package, def.span);
            let (root_kind, identity_kind, key, field_path, qualifier) = match at {
                None => (
                    RootKind::Global,
                    IdentityKind::DataRoot,
                    global_root_key(&def_package, &path)
                        .map_err(|error| map_error(error, Some(def.span)))?,
                    String::new(),
                    String::new(),
                ),
                Some(_) => (
                    RootKind::Direct,
                    IdentityKind::Direct,
                    direct_root_key(&def_package, &path, None)
                        .map_err(|error| map_error(error, Some(def.span)))?,
                    String::new(),
                    String::new(),
                ),
            };
            let (initializer, init_digest, retention) = match at {
                None => {
                    let (initializer, digest) = self.initializer(*ty, *init, def.span)?;
                    (
                        initializer,
                        digest,
                        match retain {
                            RetainPolicy::Retain => Retention::Retain,
                            RetainPolicy::None | RetainPolicy::NonRetain => Retention::None,
                        },
                    )
                }
                Some(_) => (InitializerKind::None, None, Retention::None),
            };
            let symbol =
                (root_kind != RootKind::Direct).then(|| mangle::global_symbol(self.hir, id));
            let key_for_target = key.clone();
            catalog.roots.push(RootDecl {
                identity: Identity::new(identity_kind, &def_package, &path, qualifier),
                key,
                root_kind,
                owner_type: None,
                field_path,
                type_key,
                layout_hash,
                size,
                align,
                retention,
                initializer,
                initializer_digest: init_digest,
                source,
            });
            root_targets.insert(key_for_target, (RootTarget::Global(id), symbol));
        }
        Ok(())
    }

    fn add_instance_roots(
        &self,
        catalog: &mut Catalog,
        root_targets: &mut BTreeMap<String, (RootTarget, Option<String>)>,
    ) -> Result<(), ModuleError> {
        let configuration = self.configuration_name();
        let overrides = config_overrides(self.plan);
        for instance in &self.plan.instances {
            let DefKind::Program { fields, .. } = &self.hir.defs[instance.program.index()].kind
            else {
                continue;
            };
            let package = self.def_package(instance.program)?;
            let program_path = self.qualified_path(instance.program)?;
            let instance_path =
                instance_path(&configuration, &instance.resource, &instance.name_text);
            for field in fields {
                let data = self.hir.field(*field);
                if matches!(
                    data.section,
                    stc_hir::VarSectionKind::InOut
                        | stc_hir::VarSectionKind::Temp
                        | stc_hir::VarSectionKind::External
                ) {
                    return Err(ModuleError::from(CodegenError::unsupported(
                        codes::CODEGEN_UNSUPPORTED,
                        format!(
                            "program field section `{}` is not supported in M4",
                            data.section.as_str()
                        ),
                        data.span,
                    )));
                }
                let field_name = normalize_ident(self.hir.interner.text(data.name))
                    .map_err(|error| map_error(error, Some(data.span)))?;
                let path = format!("{program_path}.{field_name}");
                let type_key = self.got_type_key(data.ty, data.span)?;
                let (layout_hash, size, align) = self.layout(&type_key, data.span)?;
                let source = self.source_location(&package, data.span);
                let init = overrides
                    .get(&(instance.name, *field))
                    .copied()
                    .or(data.init);
                let (root_kind, identity_kind, key, initializer, init_digest, retention) =
                    match data.at {
                        None => {
                            let (initializer, digest) =
                                self.initializer(data.ty, init, data.span)?;
                            (
                                RootKind::ProgramField,
                                IdentityKind::DataRoot,
                                field_root_key(&package, &path, &instance_path)
                                    .map_err(|error| map_error(error, Some(data.span)))?,
                                initializer,
                                digest,
                                match data.retain {
                                    RetainPolicy::Retain => Retention::Retain,
                                    RetainPolicy::None | RetainPolicy::NonRetain => Retention::None,
                                },
                            )
                        }
                        Some(_) => (
                            RootKind::Direct,
                            IdentityKind::Direct,
                            direct_root_key(&package, &path, Some(&instance_path))
                                .map_err(|error| map_error(error, Some(data.span)))?,
                            InitializerKind::None,
                            None,
                            Retention::None,
                        ),
                    };
                catalog.roots.push(RootDecl {
                    identity: Identity::new(identity_kind, &package, &path, &instance_path),
                    key: key.clone(),
                    root_kind,
                    owner_type: None,
                    field_path: field_name,
                    type_key,
                    layout_hash,
                    size,
                    align,
                    retention,
                    initializer,
                    initializer_digest: init_digest,
                    source,
                });
                root_targets.insert(
                    key,
                    (
                        RootTarget::InstanceField {
                            instance: instance.name,
                            field: *field,
                        },
                        None,
                    ),
                );
            }
        }
        Ok(())
    }

    fn add_task_guards(
        &self,
        package: &str,
        catalog: &mut Catalog,
        root_targets: &mut BTreeMap<String, (RootTarget, Option<String>)>,
    ) -> Result<(), ModuleError> {
        let configuration = self.configuration_name();
        for task in self.tasks() {
            let resource = if task.resource.is_empty() {
                SYNTHETIC_NAME.to_owned()
            } else {
                task.resource.to_ascii_uppercase()
            };
            let name = task.name_text.to_ascii_uppercase();
            let key = task_guard_root_key(package, &configuration, &resource, &name)
                .map_err(|error| map_error(error, Some(task.span)))?;
            let type_key = "BOOL".to_owned();
            let (layout_hash, size, align) = self.layout(&type_key, task.span)?;
            let identity_path = format!("{configuration}/{resource}#{name}.GUARD");
            let initializer_digest = initializer_digest(&type_key, &[1]);
            let symbol = mangle::task_guard_symbol_text(&resource, &name);
            catalog.roots.push(RootDecl {
                identity: Identity::new(IdentityKind::DataRoot, package, &identity_path, ""),
                key: key.clone(),
                root_kind: RootKind::TaskGuard,
                owner_type: None,
                field_path: String::new(),
                type_key,
                layout_hash,
                size,
                align,
                retention: Retention::None,
                initializer: InitializerKind::Constant,
                initializer_digest: Some(initializer_digest),
                source: self.source_location(package, task.span),
            });
            root_targets.insert(
                key,
                (
                    RootTarget::TaskGuard {
                        configuration: configuration.clone(),
                        resource,
                        task: name,
                    },
                    Some(symbol),
                ),
            );
        }
        Ok(())
    }

    fn add_functions(
        &self,
        catalog: &mut Catalog,
        function_targets: &mut BTreeMap<String, (FunctionTarget, Option<String>)>,
    ) -> Result<(), ModuleError> {
        for (index, def) in self.hir.defs.iter().enumerate() {
            let id = DefId::new(u32::try_from(index).unwrap_or(u32::MAX));
            if synthetic(self.hir, id) || def.test_only {
                continue;
            }
            let DefKind::Function { body, .. } = &def.kind else {
                continue;
            };
            if body.is_none() {
                continue;
            }
            let package = self.def_package(id)?;
            let path = self.qualified_path(id)?;
            let signature = self.signature(id)?;
            let key = function_key(&package, &path, &signature)
                .map_err(|error| map_error(error, Some(def.span)))?;
            let signature_key = signature.to_string();
            let flags = function_flag::USER | function_flag::INDIRECT_CALLABLE;
            let export_name = def
                .export_name
                .map(|name| self.hir.interner.text(name).to_owned());
            catalog.functions.push(FunctionDecl {
                identity: Identity::new(IdentityKind::Function, &package, &path, &signature_key),
                key: key.clone(),
                signature_key,
                function_kind: FunctionKind::Function,
                owner_type: None,
                flags,
                export_name,
                vtable_slot: None,
                source: self.source_location(&package, def.span),
            });
            let symbol = mangle::function_symbol(self.hir, id).map_err(ModuleError::from)?;
            function_targets.insert(key, (FunctionTarget::Function(id), Some(symbol)));
        }
        Ok(())
    }

    fn add_program_bodies(
        &self,
        catalog: &mut Catalog,
        function_targets: &mut BTreeMap<String, (FunctionTarget, Option<String>)>,
    ) -> Result<(), ModuleError> {
        for program in &self.plan.programs {
            let package = self.def_package(*program)?;
            let type_path = self.qualified_path(*program)?;
            let signature = Signature::new(TypeKey::Void);
            let signature_key = signature.to_string();
            let key = program_body_key(&package, &type_path)
                .map_err(|error| map_error(error, Some(self.hir.defs[program.index()].span)))?;
            let path = format!("{type_path}#BODY");
            catalog.functions.push(FunctionDecl {
                identity: Identity::new(IdentityKind::Function, &package, &path, &signature_key),
                key: key.clone(),
                signature_key,
                function_kind: FunctionKind::ProgramBody,
                owner_type: None,
                flags: function_flag::PROGRAM_BODY | function_flag::INDIRECT_CALLABLE,
                export_name: None,
                vtable_slot: None,
                source: self.source_location(&package, self.hir.defs[program.index()].span),
            });
            function_targets.insert(
                key,
                (
                    FunctionTarget::ProgramBody(*program),
                    Some(mangle::program_body_symbol(self.hir, *program)),
                ),
            );
        }
        Ok(())
    }

    fn add_instances(&self, package: &str, catalog: &mut Catalog) -> Result<(), ModuleError> {
        let configuration = self.configuration_name();
        let tasks = self.task_keys(package)?;
        for instance in &self.plan.instances {
            let program_package = self.def_package(instance.program)?;
            let program_path = self.qualified_path(instance.program)?;
            let resource = if instance.resource.is_empty() {
                SYNTHETIC_NAME.to_owned()
            } else {
                instance.resource.to_ascii_uppercase()
            };
            let instance_name = instance.name_text.to_ascii_uppercase();
            let instance_path = instance_path(&configuration, &resource, &instance_name);
            let key =
                instance_identity_key(&program_package, &configuration, &resource, &instance_name)
                    .map_err(|error| map_error(error, Some(instance.span)))?;
            let task_key = match instance.task {
                Some(task) => tasks.get(&(resource.clone(), task)).cloned(),
                None if self.plan.tasks.is_empty() => tasks
                    .get(&(SYNTHETIC_NAME.to_owned(), Symbol::new(u32::MAX)))
                    .cloned(),
                None => None,
            };
            catalog.instances.push(InstanceDecl {
                identity: Identity::new(
                    IdentityKind::Instance,
                    &program_package,
                    &instance_path,
                    &program_path,
                ),
                key,
                program_type: None,
                program_type_key: program_path,
                configuration: configuration.clone(),
                resource,
                instance_path,
                task_key,
                source: self.source_location(&program_package, instance.span),
            });
        }
        Ok(())
    }

    fn add_tasks(&self, package: &str, catalog: &mut Catalog) -> Result<(), ModuleError> {
        let configuration = self.configuration_name();
        for task in self.tasks() {
            let resource = if task.resource.is_empty() {
                SYNTHETIC_NAME.to_owned()
            } else {
                task.resource.to_ascii_uppercase()
            };
            let name = task.name_text.to_ascii_uppercase();
            let key = task_identity_key(package, &configuration, &resource, &name)
                .map_err(|error| map_error(error, Some(task.span)))?;
            let guard_key = task_guard_root_key(package, &configuration, &resource, &name)
                .map_err(|error| map_error(error, Some(task.span)))?;
            catalog.tasks.push(TaskDecl {
                identity: Identity::new(
                    IdentityKind::Task,
                    package,
                    format!("{configuration}/{resource}"),
                    &name,
                ),
                key,
                configuration: configuration.clone(),
                resource,
                name,
                single: task.single.unwrap_or(false),
                interval_ns: task
                    .interval_ns
                    .map(i64::try_from)
                    .transpose()
                    .map_err(|_| {
                        ModuleError::error(
                            codes::RUN_TASK,
                            format!("task `{}` interval is out of range", task.name_text),
                        )
                    })?,
                priority: task.priority.map(i64::try_from).transpose().map_err(|_| {
                    ModuleError::error(
                        codes::RUN_TASK,
                        format!("task `{}` priority is out of range", task.name_text),
                    )
                })?,
                guard_key: Some(guard_key),
                source: self.source_location(package, task.span),
            });
        }
        Ok(())
    }

    fn add_exports(&self, catalog: &mut Catalog) -> Result<(), ModuleError> {
        for (index, def) in self.hir.defs.iter().enumerate() {
            let id = DefId::new(u32::try_from(index).unwrap_or(u32::MAX));
            if synthetic(self.hir, id) || def.test_only {
                continue;
            }
            let Some(c_name) = def.export_name else {
                continue;
            };
            let DefKind::Function { body, .. } = &def.kind else {
                continue;
            };
            if body.is_none() {
                continue;
            }
            let package = self.def_package(id)?;
            let path = self.qualified_path(id)?;
            let signature = self.signature(id)?;
            let signature_key = signature.to_string();
            let c_name = self.hir.interner.text(c_name).to_owned();
            let function_key = function_key(&package, &path, &signature)
                .map_err(|error| map_error(error, Some(def.span)))?;
            let qualifier = format!("{signature_key}#C:{c_name}");
            let key = export_identity_key(&package, &path, &signature, &c_name)
                .map_err(|error| map_error(error, Some(def.span)))?;
            catalog.exports.push(ExportDecl {
                identity: Identity::new(IdentityKind::Export, &package, &path, qualifier),
                key,
                c_name,
                function_key,
                signature_key,
                no_mangle: def.no_mangle,
                source: self.source_location(&package, def.span),
            });
        }
        Ok(())
    }

    fn tasks(&self) -> Vec<PlanTask> {
        if self.plan.tasks.is_empty() {
            vec![PlanTask {
                name: Symbol::new(u32::MAX),
                name_text: SYNTHETIC_NAME.to_owned(),
                resource: SYNTHETIC_NAME.to_owned(),
                single: Some(false),
                interval_ns: None,
                priority: None,
                span: Span::empty(FileId::new(0), 0),
            }]
        } else {
            self.plan.tasks.clone()
        }
    }

    fn task_keys(&self, package: &str) -> Result<BTreeMap<(String, Symbol), String>, ModuleError> {
        let configuration = self.configuration_name();
        let mut keys = BTreeMap::new();
        for task in self.tasks() {
            let resource = if task.resource.is_empty() {
                SYNTHETIC_NAME.to_owned()
            } else {
                task.resource.to_ascii_uppercase()
            };
            let name = task.name_text.to_ascii_uppercase();
            let key = task_identity_key(package, &configuration, &resource, &name)
                .map_err(|error| map_error(error, Some(task.span)))?;
            keys.insert((resource, task.name), key);
        }
        Ok(keys)
    }

    fn configuration_name(&self) -> String {
        self.plan
            .configuration
            .as_deref()
            .map(str::to_ascii_uppercase)
            .unwrap_or_else(|| SYNTHETIC_NAME.to_owned())
    }

    fn qualified_path(&self, def: DefId) -> Result<String, ModuleError> {
        let mut segments = Vec::new();
        let mut current = Some(def);
        while let Some(id) = current {
            let data = &self.hir.defs[id.index()];
            if id == def {
                segments.push(
                    normalize_ident(data.name_text(&self.hir.interner))
                        .map_err(|error| map_error(error, Some(data.span)))?,
                );
            } else if matches!(data.kind, DefKind::Namespace { .. }) {
                if data.parent.is_some() {
                    segments.push(
                        normalize_ident(data.name_text(&self.hir.interner))
                            .map_err(|error| map_error(error, Some(data.span)))?,
                    );
                }
            } else {
                break;
            }
            current = data.parent;
        }
        segments.reverse();
        Ok(segments.join("."))
    }

    fn signature(&self, def: DefId) -> Result<Signature, ModuleError> {
        let DefKind::Function { result, params, .. } = &self.hir.defs[def.index()].kind else {
            return Err(ModuleError::error(
                codes::CODEGEN_INTERNAL,
                "signature requested for a non-function",
            ));
        };
        let mut parameters = Vec::new();
        for param in params {
            let data = self.hir.param(*param);
            let mode = match data.direction {
                ArgDirection::Input => stc_got::ParamMode::In,
                ArgDirection::Output | ArgDirection::InOut => {
                    return Err(ModuleError::from(CodegenError::unsupported(
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
                    )));
                }
            };
            let type_key = self.got_type_key(data.ty, data.span)?;
            parameters.push(stc_got::Parameter {
                mode,
                ty: TypeKey::parse(&type_key).map_err(|error| map_error(error, Some(data.span)))?,
            });
        }
        let result = match result {
            Some(ty) => {
                let type_key = self.got_type_key(*ty, self.hir.defs[def.index()].span)?;
                TypeKey::parse(&type_key)
                    .map_err(|error| map_error(error, Some(self.hir.defs[def.index()].span)))?
            }
            None => TypeKey::Void,
        };
        Ok(Signature { parameters, result })
    }

    fn got_type_key(&self, ty: TyId, span: Span) -> Result<String, ModuleError> {
        let value = match self.hir.types.get(ty) {
            Ty::Bool => "BOOL".to_owned(),
            Ty::Int(int) => match (int.signed, int.width) {
                (true, 8) => "SINT".to_owned(),
                (true, 16) => "INT".to_owned(),
                (true, 32) => "DINT".to_owned(),
                (true, 64) => "LINT".to_owned(),
                (false, 8) => "USINT".to_owned(),
                (false, 16) => "UINT".to_owned(),
                (false, 32) => "UDINT".to_owned(),
                (false, 64) => "ULINT".to_owned(),
                _ => {
                    return Err(ModuleError::from(CodegenError::unsupported(
                        codes::CODEGEN_UNSUPPORTED,
                        format!("integer width {} is not supported in M4", int.width),
                        span,
                    )));
                }
            },
            Ty::BitString(width) => match width {
                8 => "BYTE".to_owned(),
                16 => "WORD".to_owned(),
                32 => "DWORD".to_owned(),
                64 => "LWORD".to_owned(),
                _ => {
                    return Err(ModuleError::from(CodegenError::unsupported(
                        codes::CODEGEN_UNSUPPORTED,
                        format!("bit-string width {width} is not supported in M4"),
                        span,
                    )));
                }
            },
            Ty::Real(stc_hir::RealTy::Real) => "REAL".to_owned(),
            Ty::Real(stc_hir::RealTy::LReal) => "LREAL".to_owned(),
            Ty::Time(stc_hir::TimeTy::Time) => "TIME".to_owned(),
            Ty::Time(stc_hir::TimeTy::LTime) => "LTIME".to_owned(),
            Ty::Date(kind) => match kind {
                stc_hir::DateTy::Date => "DATE",
                stc_hir::DateTy::LDate => "LDATE",
                stc_hir::DateTy::Tod => "TOD",
                stc_hir::DateTy::LTod => "LTOD",
                stc_hir::DateTy::DateTime => "DT",
                stc_hir::DateTy::LDateTime => "LDT",
            }
            .to_owned(),
            Ty::Char(kind) => match kind {
                stc_hir::CharTy::Char => "CHAR",
                stc_hir::CharTy::WChar => "WCHAR",
                stc_hir::CharTy::UChar => "UCHAR",
            }
            .to_owned(),
            other => {
                layout::scalar_kind_at(self.hir, ty, span).map_err(|_| {
                    CodegenError::unsupported(
                        codes::CODEGEN_UNSUPPORTED,
                        format!("type `{other:?}` is not supported in M4"),
                        span,
                    )
                })?;
                return Err(ModuleError::error(
                    codes::CODEGEN_INTERNAL,
                    format!("type `{other:?}` has no module type key"),
                ));
            }
        };
        Ok(value)
    }

    fn layout(&self, type_key: &str, span: Span) -> Result<(GotId, u64, u32), ModuleError> {
        let ty = TypeKey::parse(type_key).map_err(|error| map_error(error, Some(span)))?;
        let layout = elementary_layout(&ty).ok_or_else(|| {
            ModuleError::error(
                codes::CODEGEN_INTERNAL,
                format!("no elementary layout for `{type_key}`"),
            )
        })?;
        let hash = elementary_layout_hash(self.target, &ty).ok_or_else(|| {
            ModuleError::error(
                codes::CODEGEN_INTERNAL,
                format!("no layout hash for `{type_key}`"),
            )
        })?;
        Ok((hash, layout.size, layout.align))
    }

    fn initializer(
        &self,
        ty: TyId,
        init: Option<ExprId>,
        span: Span,
    ) -> Result<(InitializerKind, Option<Digest32>), ModuleError> {
        let Some(init) = init else {
            return Ok((InitializerKind::Zero, None));
        };
        let type_key = self.got_type_key(ty, span)?;
        if let ExprKindHir::Literal(literal) = &self.hir.expr(init).kind {
            let bytes = encode_value(self.hir, ty, &literal.value, span)?;
            return Ok((
                InitializerKind::Constant,
                Some(initializer_digest(&type_key, &bytes)),
            ));
        }
        let fingerprint = expression_fingerprint(self.hir, init)?;
        Ok((
            InitializerKind::Expression,
            Some(expression_initializer_digest(&fingerprint)),
        ))
    }

    fn source_location(&self, package: &str, span: Span) -> Option<SourceLocation> {
        if span.start == 0 && span.end == 0 {
            return None;
        }
        let file = self.source_paths.get(span.file.index())?.clone();
        Some(SourceLocation {
            package: package.to_owned(),
            file,
            start: span.start,
            end: span.end,
        })
    }
}

fn config_overrides(plan: &Plan) -> BTreeMap<(Symbol, FieldId), ExprId> {
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

fn instance_path(configuration: &str, resource: &str, instance: &str) -> String {
    let configuration = if configuration.is_empty() {
        SYNTHETIC_NAME
    } else {
        configuration
    };
    let resource = if resource.is_empty() {
        SYNTHETIC_NAME
    } else {
        resource
    };
    format!(
        "{}/{}/{}",
        configuration.to_ascii_uppercase(),
        resource.to_ascii_uppercase(),
        instance.to_ascii_uppercase()
    )
}

fn default_imports(_retain: &RetainLayout) -> Vec<ImportService> {
    vec![
        ImportService::Trap,
        ImportService::IoImage,
        ImportService::IoSize,
        ImportService::StateLoad,
        ImportService::StateSave,
    ]
}

fn map_error(error: stc_got::Error, span: Option<Span>) -> ModuleError {
    ModuleError {
        code: error.code,
        severity: Severity::Error,
        message: error.message,
        span,
        got_id: None,
        old_key: None,
        new_key: None,
    }
}

fn synthetic(hir: &HirCrate, def: DefId) -> bool {
    let span = hir.defs[def.index()].span;
    span.start == 0 && span.end == 0
}

/// Computes the source fingerprint for the M4 bridge package.
///
/// Input paths may be absolute; they are reduced to package-relative paths
/// with the same rule used for manifest source locations, then prefixed with
/// [`M4_PACKAGE`] (GOT Artifacts §11.1).
///
/// # Errors
///
/// Rejects unrepresentable counts or invalid paths.
pub fn source_fingerprint(sources: &[(String, Vec<u8>)]) -> Result<Digest32, ModuleError> {
    let paths = sources
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    let relative = package_relative_paths(&paths);
    let files = sources
        .iter()
        .zip(relative)
        .map(|((_, bytes), path)| (format!("{M4_PACKAGE}/{path}"), bytes.clone()))
        .collect::<Vec<_>>();
    stc_got::source_fingerprint(&files).map_err(|error| map_error(error, None))
}

/// Computes package-relative `/`-separated source paths in `FileId` order.
///
/// Absolute or drive-qualified inputs are reduced to a path relative to the
/// longest common parent directory. When no common parent exists, the file
/// name is used; duplicate names are disambiguated by input ordinal.
#[must_use]
pub fn package_relative_paths(sources: &[String]) -> Vec<String> {
    let components = sources
        .iter()
        .map(|source| {
            source
                .replace('\\', "/")
                .split('/')
                .filter(|segment| !segment.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut common = components
        .first()
        .map(|first| first[..first.len().saturating_sub(1)].to_vec())
        .unwrap_or_default();
    for path in components.iter().skip(1) {
        let parent = &path[..path.len().saturating_sub(1)];
        let length = common
            .iter()
            .zip(parent)
            .take_while(|(left, right)| left == right)
            .count();
        common.truncate(length);
    }
    let mut seen = BTreeSet::new();
    components
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let mut parts = if path.len() > common.len() {
                path[common.len()..].to_vec()
            } else {
                Vec::new()
            };
            // Drop a drive prefix that survived prefix stripping: `C:` as a
            // whole component, or `C:name` inside the final file name.
            if parts.first().is_some_and(|part| part.ends_with(':')) {
                parts.remove(0);
            }
            if parts.len() == 1
                && let Some((_, rest)) = parts[0].split_once(':')
            {
                parts[0] = rest.to_owned();
            }
            let mut relative = parts.join("/");
            if relative.is_empty() {
                relative = path.last().cloned().unwrap_or_default();
            }
            if !seen.insert(relative.clone()) {
                let name = path.last().cloned().unwrap_or_default();
                relative = format!("{index}_{name}");
            }
            relative
        })
        .collect()
}

/// Encodes one scalar constant in the canonical little-endian form used by
/// initializer digests.
fn encode_value(
    hir: &HirCrate,
    ty: TyId,
    value: &ConstValue,
    span: Span,
) -> Result<Vec<u8>, ModuleError> {
    match hir.types.get(ty) {
        Ty::Bool => Ok(vec![u8::from(matches!(value, ConstValue::Bool(true)))]),
        Ty::Int(int) => Ok(int_bytes(const_value_int(value), int.width)),
        Ty::BitString(width) => Ok(int_bytes(const_value_int(value), *width)),
        Ty::Char(kind) => {
            let width = match kind {
                stc_hir::CharTy::Char => 8,
                stc_hir::CharTy::WChar => 16,
                stc_hir::CharTy::UChar => 32,
            };
            Ok(int_bytes(const_value_int(value), width))
        }
        Ty::Real(stc_hir::RealTy::Real) => {
            let number = match value {
                ConstValue::Real(value) => *value as f32,
                ConstValue::Int(value) => *value as f32,
                _ => 0.0,
            };
            Ok(number.to_bits().to_le_bytes().to_vec())
        }
        Ty::Real(stc_hir::RealTy::LReal) => {
            let number = match value {
                ConstValue::Real(value) => *value,
                ConstValue::Int(value) => *value as f64,
                _ => 0.0,
            };
            Ok(number.to_bits().to_le_bytes().to_vec())
        }
        Ty::Time(_) | Ty::Date(_) => {
            let number = match value {
                ConstValue::Time(value) | ConstValue::Date { value, .. } => *value,
                ConstValue::Int(value) => *value,
                _ => 0,
            };
            Ok((number as u64).to_le_bytes().to_vec())
        }
        _ => Err(ModuleError::from(CodegenError::unsupported(
            codes::CODEGEN_UNSUPPORTED,
            "a non-scalar initializer is not supported in M4".into(),
            span,
        ))),
    }
}

fn const_value_int(value: &ConstValue) -> i128 {
    match value {
        ConstValue::Int(value)
        | ConstValue::Time(value)
        | ConstValue::Enum { value, .. }
        | ConstValue::Date { value, .. } => *value,
        ConstValue::Char(value) => i128::from(*value),
        ConstValue::Bool(value) => i128::from(u8::from(*value)),
        _ => 0,
    }
}

fn int_bytes(value: i128, width: u8) -> Vec<u8> {
    let bytes = usize::from(width / 8);
    let value = value as u128;
    (0..bytes)
        .map(|index| ((value >> (index * 8)) & 0xFF) as u8)
        .collect()
}

/// Builds a deterministic structural fingerprint for an initializer
/// expression.
fn expression_fingerprint(hir: &HirCrate, expr: ExprId) -> Result<Vec<u8>, ModuleError> {
    let mut out = Vec::new();
    write_expr(hir, expr, &mut out, 0)?;
    Ok(out)
}

fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_tag(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

fn type_key_text(hir: &HirCrate, ty: TyId) -> String {
    match hir.types.get(ty) {
        Ty::Bool => "BOOL".to_owned(),
        Ty::Int(int) => match (int.signed, int.width) {
            (true, 8) => "SINT".to_owned(),
            (true, 16) => "INT".to_owned(),
            (true, 32) => "DINT".to_owned(),
            (true, 64) => "LINT".to_owned(),
            (false, 8) => "USINT".to_owned(),
            (false, 16) => "UINT".to_owned(),
            (false, 32) => "UDINT".to_owned(),
            (false, 64) => "ULINT".to_owned(),
            _ => format!("INT{}", int.width),
        },
        Ty::BitString(width) => match width {
            8 => "BYTE".to_owned(),
            16 => "WORD".to_owned(),
            32 => "DWORD".to_owned(),
            64 => "LWORD".to_owned(),
            _ => format!("BITS{width}"),
        },
        Ty::Real(stc_hir::RealTy::Real) => "REAL".to_owned(),
        Ty::Real(stc_hir::RealTy::LReal) => "LREAL".to_owned(),
        Ty::Time(stc_hir::TimeTy::Time) => "TIME".to_owned(),
        Ty::Time(stc_hir::TimeTy::LTime) => "LTIME".to_owned(),
        Ty::Date(kind) => format!("{kind:?}"),
        Ty::Char(kind) => format!("{kind:?}"),
        other => format!("{other:?}"),
    }
}

fn write_text(out: &mut Vec<u8>, text: &str) {
    write_u32(out, u32::try_from(text.len()).unwrap_or(u32::MAX));
    out.extend_from_slice(text.as_bytes());
}

fn write_type_key(hir: &HirCrate, ty: TyId, out: &mut Vec<u8>) {
    write_text(out, &type_key_text(hir, ty));
}

fn signature_text_raw(hir: &HirCrate, def: DefId) -> String {
    let DefKind::Function { result, params, .. } = &hir.defs[def.index()].kind else {
        return String::new();
    };
    let mut text = String::from("(");
    for (index, param) in params.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        let data = hir.param(*param);
        text.push_str(match data.direction {
            ArgDirection::Input => "IN:",
            ArgDirection::Output => "OUT:",
            ArgDirection::InOut => "INOUT:",
        });
        text.push_str(&type_key_text(hir, data.ty));
    }
    text.push_str(")->");
    match result {
        Some(ty) => text.push_str(&type_key_text(hir, *ty)),
        None => text.push_str("VOID"),
    }
    text
}

fn write_const(hir: &HirCrate, value: &ConstValue, out: &mut Vec<u8>) {
    match value {
        ConstValue::Int(value) => {
            write_tag(out, 1);
            out.extend_from_slice(&value.to_le_bytes());
        }
        ConstValue::Real(value) => {
            write_tag(out, 2);
            out.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        ConstValue::Bool(value) => {
            write_tag(out, 3);
            out.push(u8::from(*value));
        }
        ConstValue::Char(value) => {
            write_tag(out, 4);
            write_u32(out, *value);
        }
        ConstValue::String(values) => {
            write_tag(out, 5);
            write_u32(out, u32::try_from(values.len()).unwrap_or(u32::MAX));
            for value in values {
                write_u32(out, *value);
            }
        }
        ConstValue::Time(value) => {
            write_tag(out, 6);
            out.extend_from_slice(&value.to_le_bytes());
        }
        ConstValue::Date { kind, value } => {
            write_tag(out, 7);
            write_u32(out, *kind as u32);
            out.extend_from_slice(&value.to_le_bytes());
        }
        ConstValue::Enum { def, value } => {
            write_tag(out, 8);
            write_text(out, &qualified_path_raw(hir, *def));
            out.extend_from_slice(&value.to_le_bytes());
        }
        ConstValue::Null => write_tag(out, 9),
    }
}

fn write_place(
    hir: &HirCrate,
    place: &PlaceHir,
    out: &mut Vec<u8>,
    depth: usize,
) -> Result<(), ModuleError> {
    match place {
        PlaceHir::Local(id) => {
            write_tag(out, 1);
            write_u32(out, id.raw());
        }
        PlaceHir::Param(id) => {
            write_tag(out, 2);
            write_u32(out, id.raw());
        }
        PlaceHir::Global(id) => {
            write_tag(out, 3);
            write_text(out, &qualified_path_raw(hir, *id));
        }
        PlaceHir::Result(id) => {
            write_tag(out, 4);
            write_text(out, &qualified_path_raw(hir, *id));
        }
        PlaceHir::This(id) => {
            write_tag(out, 5);
            write_text(out, &qualified_path_raw(hir, *id));
        }
        PlaceHir::Field { base, field } => {
            write_tag(out, 6);
            write_expr(hir, *base, out, depth + 1)?;
            let data = hir.field(*field);
            let owner = qualified_path_raw(hir, data.owner);
            let name = hir.interner.text(data.name).to_ascii_uppercase();
            write_text(out, &format!("{owner}.{name}"));
        }
        PlaceHir::Index { base, indices } => {
            write_tag(out, 7);
            write_expr(hir, *base, out, depth + 1)?;
            write_u32(out, u32::try_from(indices.len()).unwrap_or(u32::MAX));
            for index in indices {
                write_expr(hir, *index, out, depth + 1)?;
            }
        }
        PlaceHir::Deref(inner) => {
            write_tag(out, 8);
            write_expr(hir, *inner, out, depth + 1)?;
        }
        PlaceHir::BitField { base, bit } => {
            write_tag(out, 9);
            write_expr(hir, *base, out, depth + 1)?;
            write_u32(out, u32::from(*bit));
        }
        PlaceHir::Direct(address) => {
            write_tag(out, 10);
            write_text(out, &layout::render_address(address));
        }
        PlaceHir::InstanceField {
            program,
            instance,
            fields,
        } => {
            write_tag(out, 11);
            write_text(out, &qualified_path_raw(hir, *program));
            write_text(out, &symbol_text(hir, *instance));
            write_u32(out, u32::try_from(fields.len()).unwrap_or(u32::MAX));
            for field in fields {
                write_text(out, hir.interner.text(hir.field(*field).name));
            }
        }
    }
    Ok(())
}

fn write_callee(
    hir: &HirCrate,
    callee: &CalleeHir,
    out: &mut Vec<u8>,
    depth: usize,
) -> Result<(), ModuleError> {
    match callee {
        CalleeHir::Function(def) => {
            write_tag(out, 1);
            write_text(out, &qualified_path_raw(hir, *def));
            write_text(out, &signature_text_raw(hir, *def));
        }
        CalleeHir::StdFunction(id) => {
            write_tag(out, 2);
            write_u32(out, id.raw());
        }
        CalleeHir::Conversion { from, to } => {
            write_tag(out, 3);
            write_type_key(hir, *from, out);
            write_type_key(hir, *to, out);
        }
        CalleeHir::Method { receiver, def } => {
            write_tag(out, 4);
            write_expr(hir, *receiver, out, depth + 1)?;
            write_text(out, &qualified_path_raw(hir, *def));
        }
        CalleeHir::SuperMethod { receiver, def } => {
            write_tag(out, 5);
            write_expr(hir, *receiver, out, depth + 1)?;
            write_text(out, &qualified_path_raw(hir, *def));
        }
        CalleeHir::InterfaceMethod {
            receiver,
            interface,
            method,
        } => {
            write_tag(out, 6);
            write_expr(hir, *receiver, out, depth + 1)?;
            write_text(out, &qualified_path_raw(hir, *interface));
            write_text(out, &qualified_path_raw(hir, *method));
        }
        CalleeHir::Fb { instance, def } => {
            write_tag(out, 7);
            write_expr(hir, *instance, out, depth + 1)?;
            write_text(out, &qualified_path_raw(hir, *def));
        }
        CalleeHir::SuperBody { instance, base } => {
            write_tag(out, 8);
            write_expr(hir, *instance, out, depth + 1)?;
            write_text(out, &qualified_path_raw(hir, *base));
        }
    }
    Ok(())
}

fn write_expr(
    hir: &HirCrate,
    id: ExprId,
    out: &mut Vec<u8>,
    depth: usize,
) -> Result<(), ModuleError> {
    if depth > 256 {
        return Err(ModuleError::error(
            codes::CODEGEN_INTERNAL,
            "initializer expression nesting exceeds the fingerprint limit",
        ));
    }
    let expr = hir.expr(id);
    match &expr.kind {
        ExprKindHir::Literal(literal) => {
            write_tag(out, 1);
            write_type_key(hir, expr.ty, out);
            write_const(hir, &literal.value, out);
        }
        ExprKindHir::Place(place) => {
            write_tag(out, 2);
            write_place(hir, place, out, depth + 1)?;
        }
        ExprKindHir::Binary { op, lhs, rhs } => {
            write_tag(out, 3);
            write_tag(out, *op as u8);
            write_expr(hir, *lhs, out, depth + 1)?;
            write_expr(hir, *rhs, out, depth + 1)?;
        }
        ExprKindHir::Unary { op, operand } => {
            write_tag(out, 4);
            write_tag(out, *op as u8);
            write_expr(hir, *operand, out, depth + 1)?;
        }
        ExprKindHir::Call { callee, args } => {
            write_tag(out, 5);
            write_callee(hir, callee, out, depth + 1)?;
            write_u32(out, u32::try_from(args.len()).unwrap_or(u32::MAX));
            for arg in args {
                write_u32(out, arg.formal.map_or(u32::MAX, |id| id.raw()));
                write_u32(out, arg.field.map_or(u32::MAX, |id| id.raw()));
                write_tag(
                    out,
                    match arg.direction {
                        ArgDirection::Input => 1,
                        ArgDirection::Output => 2,
                        ArgDirection::InOut => 3,
                    },
                );
                match arg.value {
                    Some(value) => {
                        write_tag(out, 1);
                        write_expr(hir, value, out, depth + 1)?;
                    }
                    None => write_tag(out, 0),
                }
                match arg.target {
                    Some(target) => {
                        write_tag(out, 1);
                        write_expr(hir, target, out, depth + 1)?;
                    }
                    None => write_tag(out, 0),
                }
                write_tag(out, u8::from(arg.negated));
            }
        }
        ExprKindHir::Ref { target } => {
            write_tag(out, 6);
            write_expr(hir, *target, out, depth + 1)?;
        }
        ExprKindHir::Null => write_tag(out, 7),
        ExprKindHir::Cast { operand, to, kind } => {
            write_tag(out, 8);
            write_type_key(hir, *to, out);
            write_tag(
                out,
                match kind {
                    stc_hir::CastKind::Implicit => 1,
                    stc_hir::CastKind::Explicit => 2,
                },
            );
            write_expr(hir, *operand, out, depth + 1)?;
        }
        ExprKindHir::ArrayInit { elements } => {
            write_tag(out, 9);
            write_u32(out, u32::try_from(elements.len()).unwrap_or(u32::MAX));
            for element in elements {
                write_expr(hir, *element, out, depth + 1)?;
            }
        }
        ExprKindHir::StructInit { fields } => {
            write_tag(out, 10);
            write_u32(out, u32::try_from(fields.len()).unwrap_or(u32::MAX));
            for (field, value) in fields {
                write_u32(out, field.raw());
                write_expr(hir, *value, out, depth + 1)?;
            }
        }
        ExprKindHir::Error => write_tag(out, 11),
    }
    Ok(())
}

/// Returns the source text of a symbol, resolving the reserved synthesized
/// instance/task symbol to its canonical name.
fn symbol_text(hir: &HirCrate, symbol: Symbol) -> String {
    if symbol == Symbol::new(u32::MAX) {
        SYNTHETIC_NAME.to_owned()
    } else {
        hir.interner.text(symbol).to_owned()
    }
}

fn qualified_path_raw(hir: &HirCrate, def: DefId) -> String {
    let mut segments = Vec::new();
    let mut current = Some(def);
    while let Some(id) = current {
        let data = &hir.defs[id.index()];
        if id == def || (matches!(data.kind, DefKind::Namespace { .. }) && data.parent.is_some()) {
            segments.push(data.name_text(&hir.interner).to_ascii_uppercase());
        } else if !matches!(data.kind, DefKind::Namespace { .. }) {
            break;
        }
        current = data.parent;
    }
    segments.reverse();
    segments.join(".")
}
