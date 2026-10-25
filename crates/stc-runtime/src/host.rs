//! Host-side module lifecycle: load, validate, prepare, activate, commit,
//! roll back, and unload whole-program module generations.
//!
//! The host owns persistent roots, the GOT, the committed manifest baseline,
//! and the mapping from stable identity to storage. A candidate is loaded and
//! validated without touching the active generation; an online candidate is
//! prepared into a temporary GOT and swapped in at an explicit cycle boundary.
//! The previous generation stays resident for one successful cycle and is
//! restored if that cycle fails.
//!
//! Phase 6 runs the loader, prepare, and swap machinery synchronously on the
//! caller's thread. The background loader/service threads, trap-boundary
//! rollback for non-local traps, process-image reconfiguration, and execution
//! lowering are later phases; the state machine and entry-point contract are
//! already the ones those phases use.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use stc_got::descriptor::entry_flag;
use stc_got::error::{Error, Result, codes};
use stc_got::model::PlanAction;
use stc_got::{
    Descriptor, Digest32, GotId, IdentityKind, Manifest, PlanDiagnostic, PlanVerdict, RetiredCause,
    RootKind, RootRecord, Severity, SlotKind,
};

use crate::catalog::{HostPlatform, ValidatedPair};
use crate::got::Got;
use crate::host_api;
use crate::loader::LoadedModule;
use crate::memory::{MemoryLimits, RootRegistry};
use crate::migration::{self, PlanDecision, validate_plan};
use crate::module_info::ModuleInfo;
use crate::store::{
    Deployment, Download, LayoutState, ManifestStore, RuntimeLayout, RuntimeLayoutEntry,
    StoredManifest,
};

/// Largest manifest file the host accepts.
pub const MAX_MANIFEST_BYTES: u64 = crate::store::MAX_MANIFEST_BYTES;

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

fn allocate_generation() -> u64 {
    NEXT_GENERATION.fetch_add(1, Ordering::Relaxed)
}

/// How a candidate should be installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadMode {
    /// Install cold only when the host has no baseline and the candidate is a
    /// cold build; otherwise prepare online only when the candidate is exactly
    /// current. Anything else reports [`LoadOutcome::ColdRequired`].
    Auto,
    /// Explicitly stop and reinitialize for this module.
    Cold,
    /// Prepare an online change; report cold when exact currentness cannot be
    /// established.
    Online,
}

/// The result of a load request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadOutcome {
    /// A cold install replaced the runtime state and committed the baseline.
    ColdInstalled {
        /// Newly active generation.
        generation: u64,
    },
    /// An online candidate is prepared and waiting for activation.
    Prepared {
        /// Candidate generation, not yet active.
        generation: u64,
    },
    /// The candidate cannot be applied online; the active generation is intact.
    ColdRequired {
        /// Why an online attempt was refused.
        reason: String,
    },
}

impl LoadOutcome {
    /// Returns the generation ID for a completed or prepared load.
    #[must_use]
    pub fn generation(&self) -> Option<u64> {
        match self {
            Self::ColdInstalled { generation } | Self::Prepared { generation } => Some(*generation),
            Self::ColdRequired { .. } => None,
        }
    }

    /// Returns `true` when the result selects the cold-download path.
    #[must_use]
    pub fn is_cold_required(&self) -> bool {
        matches!(self, Self::ColdRequired { .. })
    }
}

/// The result of one cycle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CycleOutcome {
    /// The active generation completed a cycle.
    Ran {
        /// Active generation.
        generation: u64,
    },
    /// The active generation completed its rollback-window cycle and committed.
    Committed {
        /// Newly committed generation.
        generation: u64,
    },
    /// The new generation failed its first cycle and the previous one was
    /// restored.
    RolledBack {
        /// Restored generation.
        restored: u64,
        /// Failure reported by the candidate.
        reason: String,
    },
}

/// A validated module/manifest pair that has not changed host state.
pub struct Candidate {
    module: LoadedModule,
    pair: ValidatedPair,
    info: Option<ModuleInfo>,
}

impl Candidate {
    /// Loads a module and its binary manifest from disk.
    ///
    /// # Errors
    ///
    /// Rejects a missing or oversized artifact, a library load failure, a
    /// malformed or unpaired descriptor/manifest, or an unsupported module
    /// shape.
    pub fn open(platform: &HostPlatform, module_path: &Path, manifest_path: &Path) -> Result<Self> {
        let manifest_binary = crate::store::read_file_bounded(manifest_path, MAX_MANIFEST_BYTES)?;
        let module = LoadedModule::load(module_path)?;
        let info = ModuleInfo::read_beside(module_path)?;
        Self::from_loaded_with_info(platform, module, manifest_binary, info)
    }

    /// Loads a module and its manifest, using an explicit metadata sidecar.
    ///
    /// # Errors
    ///
    /// Applies the same validation as [`Self::open`].
    pub fn open_with_info(
        platform: &HostPlatform,
        module_path: &Path,
        manifest_path: &Path,
        info: Option<ModuleInfo>,
    ) -> Result<Self> {
        let manifest_binary = crate::store::read_file_bounded(manifest_path, MAX_MANIFEST_BYTES)?;
        let module = LoadedModule::load(module_path)?;
        Self::from_loaded_with_info(platform, module, manifest_binary, info)
    }

    /// Validates an already loaded module against binary manifest bytes.
    ///
    /// # Errors
    ///
    /// Rejects a malformed or unpaired descriptor/manifest, a function-table
    /// count mismatch, or a module that cannot accept prepare and cycle calls.
    pub fn from_loaded(
        platform: &HostPlatform,
        module: LoadedModule,
        manifest_binary: Vec<u8>,
    ) -> Result<Self> {
        Self::from_loaded_with_info(platform, module, manifest_binary, None)
    }

    /// Validates an already loaded module with metadata.
    ///
    /// # Errors
    ///
    /// Applies the same validation as [`Self::from_loaded`].
    pub fn from_loaded_with_info(
        platform: &HostPlatform,
        module: LoadedModule,
        manifest_binary: Vec<u8>,
        info: Option<ModuleInfo>,
    ) -> Result<Self> {
        let pair =
            ValidatedPair::from_bytes(platform, module.descriptor_bytes(), &manifest_binary)?;
        if module.function_addresses().len() != pair.descriptor.functions.len() {
            return Err(Error::new(
                codes::MODULE_INVALID,
                format!(
                    "module exposes {} function addresses but its descriptor lists {}",
                    module.function_addresses().len(),
                    pair.descriptor.functions.len()
                ),
            ));
        }
        let required = entry_flag::PREPARE_INITIALIZES | entry_flag::CYCLE_CALLABLE;
        if pair.descriptor.entry_flags & required != required {
            return Err(Error::new(
                codes::MODULE_INVALID,
                format!(
                    "module entry flags {:#x} do not provide prepare initialization and cycle execution",
                    pair.descriptor.entry_flags
                ),
            ));
        }
        Ok(Self { module, pair, info })
    }

    /// Returns the candidate metadata sidecar, when present.
    #[must_use]
    pub fn info(&self) -> Option<&ModuleInfo> {
        self.info.as_ref()
    }

    /// Returns the candidate manifest.
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.pair.manifest
    }

    /// Returns the candidate descriptor.
    #[must_use]
    pub fn descriptor(&self) -> &Descriptor {
        &self.pair.descriptor
    }

    /// Returns the candidate build identity.
    #[must_use]
    pub fn build_id(&self) -> GotId {
        self.pair.manifest.header.build_id
    }

    /// Returns the candidate manifest digest.
    #[must_use]
    pub fn manifest_digest(&self) -> Digest32 {
        self.pair.manifest_digest
    }
}

/// One active or prepared module generation.
struct Generation {
    id: u64,
    module: LoadedModule,
    got: Got,
    manifest: Manifest,
    manifest_digest: Digest32,
    info: Option<ModuleInfo>,
    pins: Arc<AtomicUsize>,
    attached: bool,
    shutdown_done: bool,
    cycles: u64,
}

impl Generation {
    fn new(
        id: u64,
        module: LoadedModule,
        got: Got,
        manifest: Manifest,
        manifest_digest: Digest32,
        info: Option<ModuleInfo>,
    ) -> Self {
        Self {
            id,
            module,
            got,
            manifest,
            manifest_digest,
            info,
            pins: Arc::new(AtomicUsize::new(0)),
            attached: false,
            shutdown_done: false,
            cycles: 0,
        }
    }

    fn pin(&self) -> GenerationPin {
        self.pins.fetch_add(1, Ordering::AcqRel);
        GenerationPin {
            counter: Arc::clone(&self.pins),
            generation: self.id,
        }
    }

    fn shutdown(&mut self) {
        if self.shutdown_done {
            return;
        }
        self.shutdown_done = true;
        let _guard = host_api::enter_generation(self.id);
        let _ = crate::trap_boundary::with_boundary(|| self.module.shutdown(0));
        let _ = host_api::drain_faults(self.id);
    }

    fn detach(&mut self) {
        if !self.attached {
            return;
        }
        self.attached = false;
        let _guard = host_api::enter_generation(self.id);
        let _ = crate::trap_boundary::with_boundary(|| self.module.detach(self.id));
        let _ = host_api::drain_faults(self.id);
    }
}

impl Drop for Generation {
    fn drop(&mut self) {
        self.detach();
        host_api::withdraw_generation(self.id);
    }
}

/// A pin that delays unloading a retired generation.
///
/// Service readers use pins; the host refuses a cold install while any
/// generation is pinned. Dropping the pin releases the reference.
#[derive(Debug)]
pub struct GenerationPin {
    counter: Arc<AtomicUsize>,
    generation: u64,
}

impl GenerationPin {
    /// Returns the pinned generation.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl Drop for GenerationPin {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::AcqRel);
    }
}

enum RootOp {
    Preserved { previous: Box<RootRecord> },
    Allocated { got_id: GotId },
    Quarantined { got_id: GotId },
}

/// Undo journal for root changes made by a candidate.
#[derive(Default)]
struct RootUndo {
    operations: Vec<RootOp>,
}

impl RootUndo {
    fn push(&mut self, operation: RootOp) {
        self.operations.push(operation);
    }

    fn undo(self, roots: &mut RootRegistry) -> Result<()> {
        let mut first_error = None;
        for operation in self.operations.into_iter().rev() {
            let result = match operation {
                RootOp::Preserved { previous, .. } => roots.preserve(&previous),
                RootOp::Allocated { got_id } => roots.discard(got_id),
                RootOp::Quarantined { got_id } => roots.unquarantine(got_id),
            };
            if let Err(error) = result
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

struct Pending {
    generation: Generation,
    undo: RootUndo,
}

struct Rollback {
    generation: Generation,
    undo: RootUndo,
    successful_cycles: u64,
}

/// A value-free comparison of a candidate against the committed baseline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestDiff {
    /// Committed baseline build, when the store has one.
    pub baseline_build_id: Option<GotId>,
    /// Candidate build.
    pub candidate_build_id: GotId,
    /// Compiler verdict.
    pub verdict: PlanVerdict,
    /// Host decision when the plan validated.
    pub decision: Option<PlanDecision>,
    /// Validation failure when the plan did not validate.
    pub error: Option<Error>,
    /// Candidate plan actions.
    pub actions: Vec<stc_got::ActionRecord>,
    /// Candidate plan diagnostics.
    pub diagnostics: Vec<PlanDiagnostic>,
}

/// The persistent host and swap controller.
pub struct Host {
    platform: HostPlatform,
    roots: RootRegistry,
    store: ManifestStore,
    active: Option<Generation>,
    pending: Option<Pending>,
    rollback: Option<Rollback>,
    retired: Vec<Generation>,
    direct_bindings: BTreeMap<GotId, usize>,
    last_rollback: Option<String>,
    last_candidate: Option<u64>,
}

impl Host {
    /// Creates a host with the default root limits.
    #[must_use]
    pub fn new(platform: HostPlatform, store: ManifestStore) -> Self {
        Self::with_limits(platform, store, MemoryLimits::roots())
    }

    /// Creates a host with explicit root limits.
    #[must_use]
    pub fn with_limits(platform: HostPlatform, store: ManifestStore, limits: MemoryLimits) -> Self {
        Self {
            platform,
            roots: RootRegistry::with_limits(limits),
            store,
            active: None,
            pending: None,
            rollback: None,
            retired: Vec::new(),
            direct_bindings: BTreeMap::new(),
            last_rollback: None,
            last_candidate: None,
        }
    }

    /// Returns the host platform policy.
    #[must_use]
    pub fn platform(&self) -> &HostPlatform {
        &self.platform
    }

    /// Returns the committed manifest store.
    #[must_use]
    pub fn store(&self) -> &ManifestStore {
        &self.store
    }

    /// Returns the active generation ID.
    #[must_use]
    pub fn active_generation(&self) -> Option<u64> {
        self.active.as_ref().map(|generation| generation.id)
    }

    /// Returns the prepared-but-not-active generation ID.
    #[must_use]
    pub fn pending_generation(&self) -> Option<u64> {
        self.pending.as_ref().map(|pending| pending.generation.id)
    }

    /// Returns `true` while a committed change can still be rolled back.
    #[must_use]
    pub fn rollback_pending(&self) -> bool {
        self.rollback.is_some()
    }

    /// Returns the number of retired generations awaiting unload.
    #[must_use]
    pub fn retired_generations(&self) -> usize {
        self.retired.len()
    }

    /// Returns the number of completed cycles for the active generation.
    #[must_use]
    pub fn active_cycles(&self) -> Option<u64> {
        self.active.as_ref().map(|generation| generation.cycles)
    }

    /// Returns the reason recorded by the last rollback.
    #[must_use]
    pub fn last_rollback(&self) -> Option<&str> {
        self.last_rollback.as_deref()
    }

    /// Returns the generation ID most recently created for a load attempt.
    ///
    /// A failed prepare or load withdraws that generation, so its transient
    /// bytes are no longer present.
    #[must_use]
    pub fn last_candidate_generation(&self) -> Option<u64> {
        self.last_candidate
    }

    /// Returns the bytes reserved by one generation's transient module blocks.
    #[must_use]
    pub fn transient_bytes(&self, generation: u64) -> Option<u64> {
        host_api::transient_bytes(generation)
    }

    /// Returns the active generation's manifest.
    #[must_use]
    pub fn active_manifest(&self) -> Option<&Manifest> {
        self.active.as_ref().map(|generation| &generation.manifest)
    }

    /// Returns the active generation's module metadata.
    #[must_use]
    pub fn active_info(&self) -> Option<&ModuleInfo> {
        self.active
            .as_ref()
            .and_then(|generation| generation.info.as_ref())
    }

    /// Resolves an exported symbol in the active module.
    ///
    /// The address is valid until the active generation is retired and
    /// unloaded; embedding hosts call it through [`GenerationPin`] when they
    /// need a stable reference.
    #[must_use]
    pub fn export_symbol(&self, name: &str) -> Option<*const std::ffi::c_void> {
        self.active
            .as_ref()
            .and_then(|generation| generation.module.export_address(name))
    }

    /// Returns the active generation's build identity.
    #[must_use]
    pub fn active_build_id(&self) -> Option<GotId> {
        self.active
            .as_ref()
            .map(|generation| generation.manifest.header.build_id)
    }

    /// Returns the root registry.
    #[must_use]
    pub fn roots(&self) -> &RootRegistry {
        &self.roots
    }

    /// Returns one root's storage bytes.
    #[must_use]
    pub fn root_storage(&self, got_id: GotId) -> Option<&[u8]> {
        self.roots.storage(got_id)
    }

    /// Returns one root's storage bytes mutably.
    ///
    /// This is the control-path accessor used by the host and tests. Generated
    /// code reaches the same storage through a data cell.
    pub fn root_storage_mut(&mut self, got_id: GotId) -> Option<&mut [u8]> {
        self.roots.get_mut(got_id).map(|root| root.storage_mut())
    }

    /// Registers a direct-variable cell address.
    ///
    /// Direct bindings come from module metadata and process-image
    /// reconfiguration; tests and embedding hosts provide them explicitly until
    /// that conversion lands.
    pub fn set_direct_binding(&mut self, got_id: GotId, address: usize) {
        self.direct_bindings.insert(got_id, address);
    }

    /// Removes every direct-variable binding.
    pub fn clear_direct_bindings(&mut self) {
        self.direct_bindings.clear();
    }

    /// Configures the process image and direct bindings from module metadata.
    ///
    /// Cold installation uses this before preparing roots. The image contents
    /// are reset; the module applies input/output defaults in prepare.
    fn apply_info(&mut self, info: &ModuleInfo) -> Result<()> {
        crate::stc_rt_io_configure(info.inputs_bytes, info.outputs_bytes, info.markers_bytes);
        self.direct_bindings = direct_bindings_for(info)?;
        Ok(())
    }

    /// Recomputes the active generation's direct cell addresses after the
    /// process image was restored by a failed cold install.
    fn restore_direct_bindings(&mut self, fallback: BTreeMap<GotId, usize>) {
        let Some(info) = self
            .active
            .as_ref()
            .and_then(|generation| generation.info.clone())
        else {
            self.direct_bindings = fallback;
            return;
        };
        let Ok(bindings) = direct_bindings_for(&info) else {
            self.direct_bindings = fallback;
            return;
        };
        self.direct_bindings = bindings;
        if let Some(active) = self.active.as_mut() {
            let _ = active.got.bind_direct(&self.direct_bindings);
        }
    }

    /// Returns the committed manifest, when the store has a valid baseline.
    #[must_use]
    pub fn export_manifest(&self) -> Option<&Manifest> {
        self.store.active().map(|stored| &stored.manifest)
    }

    /// Returns the canonical JSON baseline.
    #[must_use]
    pub fn export_manifest_json(&self) -> Option<&str> {
        self.store.active().map(|stored| stored.json.as_str())
    }

    /// Returns the binary baseline.
    #[must_use]
    pub fn export_manifest_binary(&self) -> Option<&[u8]> {
        self.store.active().map(|stored| stored.binary.as_slice())
    }

    /// Builds the download artifact for the committed baseline.
    ///
    /// # Errors
    ///
    /// Rejects a request when no valid committed baseline exists.
    pub fn download(&self) -> Result<Download> {
        if self.pending.is_some() || self.rollback.is_some() {
            return Err(prepare_error(
                "cannot export a download while an online change is in progress",
            ));
        }
        let Some(stored) = self.store.active() else {
            return Err(Error::new(
                codes::MANIFEST_INVALID,
                self.store
                    .problem()
                    .map_or_else(|| "the host has no active manifest".into(), str::to_owned),
            ));
        };
        let deployment = Deployment {
            module: stored.manifest.header.module_name.clone(),
            build_id: stored.build_id,
            manifest_digest: stored.digest,
            parent_build_id: stored.manifest.header.parent_build_id,
        };
        self.store.download(self.runtime_layout(), deployment)
    }

    /// Builds the committed runtime layout summary without values.
    #[must_use]
    pub fn runtime_layout(&self) -> RuntimeLayout {
        let Some(stored) = self.store.active() else {
            return RuntimeLayout::default();
        };
        let manifest = &stored.manifest;
        let mut entries = Vec::new();
        for root in manifest
            .roots
            .iter()
            .filter(|root| root.root_kind != RootKind::Direct)
        {
            let Some(active) = self.roots.get(root.got_id) else {
                continue;
            };
            entries.push(RuntimeLayoutEntry {
                got_id: root.got_id,
                key: active.record.key.clone(),
                kind: active.record.root_kind,
                state: LayoutState::Active,
                slot: slot_for(manifest, root.got_id),
                size: active.record.size,
                generation: active.generation,
                storage: Some(active.address()),
                cause: None,
            });
        }
        for root in manifest
            .roots
            .iter()
            .filter(|root| root.root_kind == RootKind::Direct)
        {
            entries.push(RuntimeLayoutEntry {
                got_id: root.got_id,
                key: root.key.clone(),
                kind: root.root_kind,
                state: LayoutState::Active,
                slot: slot_for(manifest, root.got_id),
                size: root.size,
                generation: self.active.as_ref().map_or(0, |generation| generation.id),
                storage: self.direct_bindings.get(&root.got_id).copied(),
                cause: None,
            });
        }
        for root in self.roots.quarantined() {
            entries.push(RuntimeLayoutEntry {
                got_id: root.record.got_id,
                key: root.record.key.clone(),
                kind: root.record.root_kind,
                state: LayoutState::Quarantined,
                slot: None,
                size: root.record.size,
                generation: root.generation,
                storage: Some(root.address()),
                cause: Some(root.cause),
            });
        }
        entries.sort_by_key(|entry| entry.got_id);
        RuntimeLayout { entries }
    }

    /// Validates a candidate plan against the committed baseline.
    ///
    /// # Errors
    ///
    /// Applies the same checks as [`validate_plan`].
    pub fn classify(&self, candidate: &Candidate) -> Result<PlanDecision> {
        validate_plan(
            candidate.manifest(),
            self.store.active().map(|stored| &stored.manifest),
        )
    }

    /// Returns comparison data for a candidate without changing host state.
    ///
    /// # Errors
    ///
    /// Rejects a malformed candidate manifest.
    pub fn diff(&self, candidate: &Candidate) -> Result<ManifestDiff> {
        candidate.manifest().validate()?;
        let baseline = self.store.active().map(|stored| stored.manifest.clone());
        let decision = validate_plan(candidate.manifest(), baseline.as_ref());
        let (decision, error) = match decision {
            Ok(decision) => (Some(decision), None),
            Err(error) => (None, Some(error)),
        };
        Ok(ManifestDiff {
            baseline_build_id: baseline.map(|manifest| manifest.header.build_id),
            candidate_build_id: candidate.build_id(),
            verdict: candidate.manifest().plan.verdict,
            decision,
            error,
            actions: candidate.manifest().plan.actions.clone(),
            diagnostics: candidate.manifest().plan.diagnostics.clone(),
        })
    }

    /// Pins the active generation.
    #[must_use]
    pub fn pin_active(&self) -> Option<GenerationPin> {
        self.active.as_ref().map(Generation::pin)
    }

    /// Pins any live generation by ID.
    #[must_use]
    pub fn pin(&self, generation: u64) -> Option<GenerationPin> {
        if let Some(active) = self.active.as_ref().filter(|g| g.id == generation) {
            return Some(active.pin());
        }
        if let Some(pending) = self
            .pending
            .as_ref()
            .map(|pending| &pending.generation)
            .filter(|g| g.id == generation)
        {
            return Some(pending.pin());
        }
        if let Some(rollback) = self
            .rollback
            .as_ref()
            .map(|rollback| &rollback.generation)
            .filter(|g| g.id == generation)
        {
            return Some(rollback.pin());
        }
        self.retired
            .iter()
            .find(|g| g.id == generation)
            .map(Generation::pin)
    }

    /// Installs a validated candidate and returns the load outcome.
    ///
    /// # Errors
    ///
    /// Rejects an undeployable candidate, a hard plan rejection, a module
    /// preparation failure, or a manifest-store write failure. Every failure
    /// leaves the active generation and committed baseline intact.
    pub fn load(&mut self, candidate: Candidate, mode: LoadMode) -> Result<LoadOutcome> {
        migration::validate_deployable(candidate.manifest())?;
        match mode {
            LoadMode::Cold => self.install_cold(candidate),
            LoadMode::Online => self.load_online(candidate),
            LoadMode::Auto => {
                if self.active.is_none() {
                    if candidate.manifest().plan.verdict == PlanVerdict::Cold {
                        self.install_cold(candidate)
                    } else {
                        Ok(LoadOutcome::ColdRequired {
                            reason: "the host has no active baseline; an online change needs an \
                                     exact-current baseline"
                                .into(),
                        })
                    }
                } else if candidate.manifest().plan.verdict == PlanVerdict::Cold {
                    Ok(LoadOutcome::ColdRequired {
                        reason: "the candidate was built without the active baseline".into(),
                    })
                } else {
                    self.load_online(candidate)
                }
            }
        }
    }

    fn load_online(&mut self, candidate: Candidate) -> Result<LoadOutcome> {
        if self.pending.is_some() || self.rollback.is_some() {
            return Err(prepare_error(
                "another prepared change is still awaiting activation or commit",
            ));
        }
        let Some(active) = self.active.as_ref() else {
            return Ok(LoadOutcome::ColdRequired {
                reason: "the host has no active generation".into(),
            });
        };
        let active_build_id = active.manifest.header.build_id;
        let active_digest = active.manifest_digest;
        let Some(stored) = self.store.active() else {
            return Ok(LoadOutcome::ColdRequired {
                reason: "the host has no valid active manifest baseline".into(),
            });
        };
        if stored.build_id != active_build_id || stored.digest != active_digest {
            return Ok(LoadOutcome::ColdRequired {
                reason: "the committed manifest does not match the active generation".into(),
            });
        }
        if let Some(info) = candidate.info() {
            let Some(active_info) = active.info.as_ref() else {
                return Ok(LoadOutcome::ColdRequired {
                    reason: "the active generation has no process-image metadata".into(),
                });
            };
            if !active_info.layout_eq(info) {
                return Ok(LoadOutcome::ColdRequired {
                    reason: "the candidate changes the process-image layout or direct bindings"
                        .into(),
                });
            }
        }
        let baseline = stored.manifest.clone();
        let decision = validate_plan(candidate.manifest(), Some(&baseline));
        match decision {
            Ok(PlanDecision::Cold) => Ok(LoadOutcome::ColdRequired {
                reason: "the candidate is a cold-layout build".into(),
            }),
            Ok(PlanDecision::Online) => self.prepare_online(candidate),
            Err(error) => {
                if plan_is_terminal(candidate.manifest()) {
                    Err(error)
                } else {
                    Ok(LoadOutcome::ColdRequired {
                        reason: format!("the candidate is not exactly current: {error}"),
                    })
                }
            }
        }
    }

    fn prepare_online(&mut self, candidate: Candidate) -> Result<LoadOutcome> {
        let manifest = candidate.manifest().clone();
        let generation_id = allocate_generation();
        self.last_candidate = Some(generation_id);
        let mut undo = RootUndo::default();
        let mut fresh = BTreeSet::new();

        let applied = (|| -> Result<()> {
            for action in &manifest.plan.actions {
                if action.kind != IdentityKind::DataRoot.as_u8() {
                    continue;
                }
                match action.action {
                    PlanAction::Preserve => {
                        let record = manifest
                            .roots
                            .iter()
                            .find(|root| root.got_id == action.got_id)
                            .ok_or_else(|| {
                                prepare_error(format!(
                                    "preserve action for `{}` has no candidate root",
                                    action.key
                                ))
                            })?;
                        let previous = self
                            .roots
                            .get(action.got_id)
                            .map(|root| root.record.clone())
                            .ok_or_else(|| {
                                prepare_error(format!(
                                    "preserve action for `{}` has no active root",
                                    action.key
                                ))
                            })?;
                        self.roots.preserve(record)?;
                        undo.push(RootOp::Preserved {
                            previous: Box::new(previous),
                        });
                    }
                    PlanAction::Allocate => {
                        let record = manifest
                            .roots
                            .iter()
                            .find(|root| root.got_id == action.got_id)
                            .ok_or_else(|| {
                                prepare_error(format!(
                                    "allocate action for `{}` has no candidate root",
                                    action.key
                                ))
                            })?;
                        self.roots.allocate(record, generation_id)?;
                        undo.push(RootOp::Allocated {
                            got_id: action.got_id,
                        });
                        fresh.insert(action.got_id);
                    }
                    PlanAction::Drop => {
                        self.roots
                            .quarantine(action.got_id, RetiredCause::Dropped)?;
                        undo.push(RootOp::Quarantined {
                            got_id: action.got_id,
                        });
                    }
                    other => {
                        return Err(prepare_error(format!(
                            "plan action `{}` for `{}` is not executable by this host",
                            other.name(),
                            action.key
                        )));
                    }
                }
            }
            for root in manifest
                .roots
                .iter()
                .filter(|root| root.root_kind != RootKind::Direct)
            {
                if !self.roots.contains(root.got_id) {
                    return Err(prepare_error(format!(
                        "candidate root `{}` has no preserve or allocate action",
                        root.key
                    )));
                }
            }
            let expected_roots = manifest
                .roots
                .iter()
                .filter(|root| root.root_kind != RootKind::Direct)
                .count();
            if self.roots.len() != expected_roots {
                return Err(prepare_error(format!(
                    "root registry holds {} roots but the candidate catalog has {expected_roots}",
                    self.roots.len()
                )));
            }
            Ok(())
        })();

        if let Err(error) = applied {
            undo.undo(&mut self.roots)?;
            return Err(error);
        }

        match self.build_generation(candidate, &self.roots, generation_id, &fresh, 1) {
            Ok(generation) => {
                self.pending = Some(Pending { generation, undo });
                Ok(LoadOutcome::Prepared {
                    generation: generation_id,
                })
            }
            Err(error) => {
                undo.undo(&mut self.roots)?;
                Err(error)
            }
        }
    }

    fn install_cold(&mut self, candidate: Candidate) -> Result<LoadOutcome> {
        if self.any_pinned() {
            return Err(prepare_error(
                "cannot cold install while a generation is pinned by a service reader",
            ));
        }
        let manifest = candidate.manifest().clone();
        let previous_bindings = self.direct_bindings.clone();
        let image = candidate.info().map(|_| crate::image_snapshot());
        let generation_id = allocate_generation();
        self.last_candidate = Some(generation_id);
        let prepared = (|| -> Result<(Generation, RootRegistry)> {
            if let Some(info) = candidate.info() {
                self.apply_info(info)?;
            }
            let mut roots = RootRegistry::with_limits(self.roots.limits());
            for root in manifest
                .roots
                .iter()
                .filter(|root| root.root_kind != RootKind::Direct)
            {
                roots.allocate(root, generation_id)?;
            }
            let fresh = BTreeSet::new();
            let generation = self.build_generation(candidate, &roots, generation_id, &fresh, 0)?;
            let stored = StoredManifest::new(generation.manifest.clone())?;
            self.store.install(stored)?;
            Ok((generation, roots))
        })();
        let (generation, roots) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                if let Some(image) = image {
                    crate::restore_image(image);
                    self.restore_direct_bindings(previous_bindings);
                } else {
                    self.direct_bindings = previous_bindings;
                }
                return Err(error);
            }
        };

        // No fallible operation follows; the swap is atomic from here.
        if let Some(pending) = self.pending.take() {
            drop(pending);
        }
        if let Some(rollback) = self.rollback.take() {
            let mut old = rollback.generation;
            old.shutdown();
            self.retired.push(old);
        }
        if let Some(mut old) = self.active.take() {
            old.shutdown();
            self.retired.push(old);
        }
        self.roots = roots;
        self.active = Some(generation);
        self.last_rollback = None;
        self.unload_retired();
        Ok(LoadOutcome::ColdInstalled {
            generation: generation_id,
        })
    }

    fn build_generation(
        &self,
        candidate: Candidate,
        roots: &RootRegistry,
        generation_id: u64,
        fresh: &BTreeSet<GotId>,
        mode: u32,
    ) -> Result<Generation> {
        let Candidate { module, pair, info } = candidate;
        let manifest = pair.manifest;
        let manifest_digest = pair.manifest_digest;
        if manifest
            .slots
            .iter()
            .any(|slot| slot.kind == SlotKind::Descriptor)
        {
            return Err(Error::new(
                codes::ONLINE_BINDING,
                "descriptor cells require the M5 runtime descriptor table",
            ));
        }
        let slot_count = u32::try_from(manifest.slots.len())
            .map_err(|_| Error::new(codes::ONLINE_BINDING, "slot count exceeds u32"))?;

        host_api::register_generation(generation_id)?;
        let result = (|| -> Result<Generation> {
            let mut got = Got::new(&manifest.slots)?;
            got.bind_data_roots(roots)?;
            let mut functions = BTreeMap::new();
            for (function, address) in pair
                .descriptor
                .functions
                .iter()
                .zip(module.function_addresses())
            {
                functions.insert(function.got_id, *address as usize);
            }
            got.bind_functions(&functions)?;
            got.bind_direct(&self.direct_bindings)?;
            got.bind_imports(host_api::service_address)?;
            if !got.unbound_slots().is_empty() {
                return Err(Error::new(
                    codes::ONLINE_BINDING,
                    format!("GOT has unbound slots {:?}", got.unbound_slots()),
                ));
            }

            let mut generation =
                Generation::new(generation_id, module, got, manifest, manifest_digest, info);
            let mask = if mode == 0 {
                Vec::new()
            } else {
                generation.got.fresh_root_mask(fresh)
            };
            let mask_size = u32::try_from(mask.len())
                .map_err(|_| Error::new(codes::ONLINE_BINDING, "fresh-root mask exceeds u32"))?;
            let mask_pointer = if mask.is_empty() {
                std::ptr::null()
            } else {
                mask.as_ptr()
            };
            let attached = {
                let _guard = host_api::enter_generation(generation_id);
                crate::trap_boundary::with_boundary(|| -> Result<()> {
                    // SAFETY: the host API table is a live static and the GOT
                    // exposes a stable slot array for this generation.
                    let attach = unsafe {
                        generation.module.attach(
                            host_api::api_pointer(),
                            generation.got.slot_table_pointer(),
                            slot_count,
                            generation_id,
                        )
                    };
                    check_attach(attach)?;
                    generation.attached = true;
                    // SAFETY: `mask` is alive across the call and its length
                    // is passed with it; null is used only for an empty mask.
                    let prepare = unsafe {
                        generation
                            .module
                            .prepare(mode, generation_id, mask_pointer, mask_size)
                    };
                    check_prepare(prepare)?;
                    Ok(())
                })
            };
            let faults = host_api::drain_faults(generation_id);
            match attached {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return Err(error),
                Err(trap) => {
                    return Err(Error::new(
                        codes::ONLINE_PREPARE,
                        format!(
                            "module trapped during attach or prepare (code {} at span {})",
                            trap.code, trap.span_id
                        ),
                    ));
                }
            }
            if !faults.is_empty() {
                return Err(Error::new(
                    codes::ONLINE_PREPARE,
                    format!("module reported prepare faults: {}", faults.join("; ")),
                ));
            }
            Ok(generation)
        })();
        if result.is_err() {
            host_api::withdraw_generation(generation_id);
        }
        result
    }

    /// Activates a prepared candidate at the current cycle boundary.
    ///
    /// # Errors
    ///
    /// Rejects an activation when no candidate is prepared or a previous change
    /// is still in its rollback window.
    pub fn activate(&mut self) -> Result<u64> {
        if self.rollback.is_some() {
            return Err(prepare_error(
                "the previous change is still in its rollback window",
            ));
        }
        let Some(pending) = self.pending.take() else {
            return Err(prepare_error("no prepared candidate to activate"));
        };
        let Some(previous) = self.active.take() else {
            return Err(prepare_error("no active generation to replace"));
        };
        let generation = pending.generation.id;
        self.rollback = Some(Rollback {
            generation: previous,
            undo: pending.undo,
            successful_cycles: 0,
        });
        self.active = Some(pending.generation);
        Ok(generation)
    }

    /// Runs one cycle of the active generation.
    ///
    /// During the rollback window, a failed cycle restores the previous
    /// generation and returns [`CycleOutcome::RolledBack`]. A successful cycle
    /// commits the new generation and unloads the old one when it is unpinned.
    ///
    /// # Errors
    ///
    /// Rejects a call with no active generation or a cycle failure outside a
    /// rollback window.
    pub fn cycle(&mut self) -> Result<CycleOutcome> {
        let Some(generation) = self.active.as_ref() else {
            return Err(prepare_error("no active generation"));
        };
        let generation_id = generation.id;
        let outcome = {
            let _guard = host_api::enter_generation(generation_id);
            let outcome =
                crate::trap_boundary::with_boundary(|| generation.module.cycle(generation_id));
            let faults = host_api::drain_faults(generation_id);
            (outcome, faults)
        };
        let (outcome, faults) = outcome;
        let code = match outcome {
            Ok(code) => code,
            Err(trap) => {
                let reason = format!(
                    "module trapped during cycle (code {} at span {})",
                    trap.code, trap.span_id
                );
                if self.rollback.is_some() {
                    let restored = self.rollback_internal(&reason)?;
                    return Ok(CycleOutcome::RolledBack { restored, reason });
                }
                return Err(Error::new(codes::ONLINE_PREPARE, reason));
            }
        };
        if code != 0 || !faults.is_empty() {
            let reason = if faults.is_empty() {
                format!("stc_module_cycle returned {code}")
            } else {
                format!("cycle reported module faults: {}", faults.join("; "))
            };
            if self.rollback.is_some() {
                let restored = self.rollback_internal(&reason)?;
                return Ok(CycleOutcome::RolledBack { restored, reason });
            }
            return Err(Error::new(codes::ONLINE_PREPARE, reason));
        }
        if let Some(active) = self.active.as_mut() {
            active.cycles = active.cycles.saturating_add(1);
        }
        if let Some(rollback) = self.rollback.as_mut() {
            rollback.successful_cycles = rollback.successful_cycles.saturating_add(1);
        }
        if self
            .rollback
            .as_ref()
            .is_some_and(|rollback| rollback.successful_cycles >= 1)
        {
            let generation = self.commit_internal()?;
            return Ok(CycleOutcome::Committed { generation });
        }
        Ok(CycleOutcome::Ran {
            generation: generation_id,
        })
    }

    /// Commits the active generation after its rollback window.
    ///
    /// # Errors
    ///
    /// Rejects a commit with no rollback window or a manifest-store failure. A
    /// store failure leaves the previous generation active and the new change
    /// rollback-capable.
    pub fn commit(&mut self) -> Result<u64> {
        if self.rollback.is_none() {
            return Err(prepare_error("no rollback window is open"));
        }
        self.commit_internal()
    }

    fn commit_internal(&mut self) -> Result<u64> {
        let Some(active) = self.active.as_ref() else {
            return Err(prepare_error("no active generation to commit"));
        };
        let generation_id = active.id;
        let stored = StoredManifest::new(active.manifest.clone())?;
        let Some(rollback) = self.rollback.as_ref() else {
            return Err(prepare_error("no rollback window is open"));
        };
        if rollback.successful_cycles == 0 {
            return Err(prepare_error(
                "commit requires one successful cycle of the new generation",
            ));
        }
        self.store.install(stored)?;
        let Some(mut rollback) = self.rollback.take() else {
            return Err(prepare_error("no rollback window is open"));
        };
        rollback.generation.shutdown();
        self.retired.push(rollback.generation);
        self.unload_retired();
        Ok(generation_id)
    }

    /// Restores the previous generation during the rollback window.
    ///
    /// # Errors
    ///
    /// Rejects a rollback with no window open, or reports a root-journal undo
    /// failure.
    pub fn rollback(&mut self) -> Result<u64> {
        self.rollback_internal("explicit rollback")
    }

    fn rollback_internal(&mut self, reason: &str) -> Result<u64> {
        if self.rollback.is_none() {
            return Err(prepare_error("no rollback window is open"));
        }
        if self.active.is_none() {
            return Err(prepare_error("no active generation to roll back"));
        }
        let Some(rollback) = self.rollback.take() else {
            return Err(prepare_error("no rollback window is open"));
        };
        let Some(mut candidate) = self.active.take() else {
            return Err(prepare_error("no active generation to roll back"));
        };
        candidate.detach();
        drop(candidate);
        let restored = rollback.generation.id;
        let undo = rollback.undo;
        self.active = Some(rollback.generation);
        self.last_rollback = Some(reason.to_owned());
        undo.undo(&mut self.roots)?;
        Ok(restored)
    }

    /// Discards a prepared candidate without activating it.
    ///
    /// # Errors
    ///
    /// Reports a root-journal undo failure.
    pub fn abort_pending(&mut self) -> Result<()> {
        let Some(pending) = self.pending.take() else {
            return Ok(());
        };
        drop(pending.generation);
        pending.undo.undo(&mut self.roots)
    }

    /// Unloads retired generations that no pin references.
    ///
    /// Returns the number of modules unloaded.
    pub fn unload_retired(&mut self) -> usize {
        let mut unloaded = 0;
        let mut index = 0;
        while index < self.retired.len() {
            if self.retired[index].pins.load(Ordering::Acquire) == 0 {
                let mut generation = self.retired.remove(index);
                generation.detach();
                drop(generation);
                unloaded += 1;
            } else {
                index += 1;
            }
        }
        unloaded
    }

    /// Shuts down and unloads every generation.
    ///
    /// # Errors
    ///
    /// Reports a root-journal undo failure from a pending candidate.
    pub fn shutdown(&mut self) -> Result<()> {
        let mut first_error = self.abort_pending().err();
        if self.rollback.is_some()
            && let Err(error) = self.rollback_internal("host shutdown")
        {
            first_error.get_or_insert(error);
        }
        if let Some(mut active) = self.active.take() {
            active.shutdown();
            self.retired.push(active);
        }
        for mut generation in self.retired.drain(..) {
            generation.shutdown();
            generation.detach();
        }
        first_error.map_or(Ok(()), Err)
    }

    fn any_pinned(&self) -> bool {
        let pinned = |generation: &Generation| generation.pins.load(Ordering::Acquire) > 0;
        self.active.iter().any(pinned)
            || self
                .pending
                .as_ref()
                .is_some_and(|pending| pinned(&pending.generation))
            || self
                .rollback
                .as_ref()
                .is_some_and(|rollback| pinned(&rollback.generation))
            || self.retired.iter().any(pinned)
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn direct_bindings_for(info: &ModuleInfo) -> Result<BTreeMap<GotId, usize>> {
    let mut bindings = BTreeMap::new();
    for binding in &info.bindings {
        let offset = binding.offset_bits / 8;
        let bytes = binding.width_bits.div_ceil(8).max(1);
        let Some(address) = crate::image_area_address(binding.area, offset, bytes) else {
            return Err(Error::new(
                codes::ONLINE_BINDING,
                format!(
                    "direct binding {} lies outside the configured process image",
                    binding.got_id
                ),
            ));
        };
        bindings.insert(binding.got_id, address);
    }
    Ok(bindings)
}

fn slot_for(manifest: &Manifest, got_id: GotId) -> Option<u32> {
    manifest
        .slots
        .iter()
        .find(|slot| {
            slot.got_id == got_id && matches!(slot.kind, SlotKind::Data | SlotKind::Direct)
        })
        .map(|slot| slot.slot)
}

fn plan_is_terminal(manifest: &Manifest) -> bool {
    manifest.plan.verdict == PlanVerdict::Rejected
        || manifest
            .plan
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error.tag())
}

fn prepare_error(message: impl Into<String>) -> Error {
    Error::new(codes::ONLINE_PREPARE, message)
}

fn check_attach(code: i32) -> Result<()> {
    match code {
        0 => Ok(()),
        1 => Err(prepare_error(
            "stc_module_attach rejected an invalid argument or state",
        )),
        2 => Err(Error::new(
            codes::ABI_MISMATCH,
            "stc_module_attach rejected the host ABI or feature set",
        )),
        3 => Err(Error::new(
            codes::MODULE_INVALID,
            "stc_module_attach reported a slot-count or descriptor mismatch",
        )),
        other => Err(Error::new(
            codes::MODULE_INVALID,
            format!("stc_module_attach returned undocumented status {other}"),
        )),
    }
}

fn check_prepare(code: i32) -> Result<()> {
    match code {
        0 => Ok(()),
        1 => Err(prepare_error(
            "stc_module_prepare rejected the mode or state",
        )),
        2 => Err(prepare_error(
            "stc_module_prepare failed to initialize roots",
        )),
        other => Err(prepare_error(format!(
            "stc_module_prepare returned undocumented status {other}"
        ))),
    }
}
