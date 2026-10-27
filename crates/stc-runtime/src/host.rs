//! Host-side module lifecycle: load, validate, prepare, activate, commit,
//! roll back, and unload whole-program module generations.
//!
//! The host owns persistent roots, the GOT, the descriptor registry, the
//! committed manifest baseline, the process image, and the mapping from stable
//! identity to storage. A candidate is loaded and validated without touching
//! the active generation;
//! an online candidate is prepared into a temporary GOT and swapped in at an
//! explicit cycle boundary. The previous generation stays resident for one
//! successful cycle and is restored if that cycle fails.
//!
//! Activation also applies the direct-variable remap policy: a candidate whose
//! process-image layout or direct bindings differ is reconfigured at the
//! boundary, values are preserved by `GotId`, moved variables are warned about,
//! and the old image is snapshotted for rollback (Online Change §6.2, §8.1).
//!
//! Phase 6 runs the loader, prepare, and swap machinery synchronously on the
//! caller's thread. The background loader/service threads are later work; the
//! state machine and entry-point contract are already the ones those phases
//! use.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use stc_got::descriptor::entry_flag;
use stc_got::error::{Error, Result, codes};
use stc_got::model::PlanAction;
use stc_got::{
    Descriptor, Digest32, GotId, IdentityKind, Manifest, PlanDiagnostic, PlanVerdict, RetiredCause,
    RootKind, RootRecord, Severity, SlotKind, TypeKey,
};

use crate::catalog::{HostPlatform, ValidatedPair};
use crate::debug::{DebugEngine, DebugRead, DebugReadRequest, DebugService, WatchId, WatchSpec};
use crate::descriptors::{DescriptorActivation, DescriptorRegistry, PreparedDescriptors};
use crate::got::Got;
use crate::host_api;
use crate::image::{self, ImageSnapshot};
use crate::loader::LoadedModule;
use crate::memory::{MemoryLimits, RootRegistry};
use crate::migration::{self, PlanDecision, validate_plan};
use crate::module_info::ModuleInfo;
use crate::scheduler::{TaskScheduler, TaskStatus};
use crate::state::RetainConfig;
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

/// A non-fatal diagnostic recorded by a host load or activation.
///
/// The compiler's plan diagnostics travel in the manifest; these warnings are
/// observations the runtime makes while applying a change, such as a dropped
/// root being quarantined or a direct variable being remapped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostWarning {
    /// Stable diagnostic code.
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
}

/// A validated module/manifest pair that has not changed host state.
pub struct Candidate {
    module: LoadedModule,
    pair: ValidatedPair,
    info: Option<ModuleInfo>,
    module_dir: PathBuf,
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
        let module_dir = module_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map_or_else(
                || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                Path::to_path_buf,
            );
        Self::from_loaded_with_info_in(platform, module, manifest_binary, info, module_dir)
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
        let module_dir = module_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map_or_else(
                || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                Path::to_path_buf,
            );
        Self::from_loaded_with_info_in(platform, module, manifest_binary, info, module_dir)
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
        let module_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::from_loaded_with_info_in(platform, module, manifest_binary, info, module_dir)
    }

    /// Validates a loaded module with metadata and an explicit base directory
    /// for relative I/O paths.
    ///
    /// # Errors
    ///
    /// Applies the same validation as [`Self::from_loaded`].
    pub fn from_loaded_with_info_in(
        platform: &HostPlatform,
        module: LoadedModule,
        manifest_binary: Vec<u8>,
        info: Option<ModuleInfo>,
        module_dir: PathBuf,
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
        Ok(Self {
            module,
            pair,
            info,
            module_dir,
        })
    }

    /// Returns the candidate metadata sidecar, when present.
    #[must_use]
    pub fn info(&self) -> Option<&ModuleInfo> {
        self.info.as_ref()
    }

    /// Returns the directory holding the module, used for relative I/O paths.
    #[must_use]
    pub fn module_dir(&self) -> &Path {
        &self.module_dir
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

/// Process-image work a prepared candidate needs at activation.
struct PendingImage {
    /// Candidate metadata: image layout and direct bindings.
    info: ModuleInfo,
    /// Whether the image itself must be reconfigured (not just rebound).
    reconfigure: bool,
}

/// Process-image state captured when activation applied a direct remap.
struct ImageRollback {
    /// Image at the moment of activation.
    snapshot: ImageSnapshot,
    /// Direct bindings before the swap, used to rebind the restored GOT.
    bindings: BTreeMap<GotId, usize>,
}

struct Pending {
    generation: Generation,
    undo: RootUndo,
    descriptors: PreparedDescriptors,
    image: Option<PendingImage>,
    /// Metadata applied to the I/O backend at activation.
    io_info: Option<(ModuleInfo, PathBuf)>,
    warnings: Vec<HostWarning>,
    scheduler: TaskScheduler,
}

struct Rollback {
    generation: Generation,
    undo: RootUndo,
    descriptors: DescriptorActivation,
    successful_cycles: u64,
    image: Option<ImageRollback>,
    scheduler: TaskScheduler,
}

/// A host status snapshot for verbose runs and the control channel
/// (M6-Plan §14.5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostStatus {
    /// Active generation build, when one is installed.
    pub active_build_id: Option<GotId>,
    /// Prepared candidate build awaiting activation.
    pub pending_build_id: Option<GotId>,
    /// Retired generation builds still resident.
    pub retired_build_ids: Vec<GotId>,
    /// Cycles completed by the active generation.
    pub cycles: u64,
    /// Why the latest load request was rejected or required a cold download.
    pub last_rejection: Option<String>,
    /// Non-fatal diagnostics from the latest activation.
    pub warnings: Vec<HostWarning>,
    /// Per-task schedule state.
    pub tasks: Vec<TaskStatus>,
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
    descriptors: DescriptorRegistry,
    store: ManifestStore,
    active: Option<Generation>,
    pending: Option<Pending>,
    rollback: Option<Rollback>,
    retired: Vec<Generation>,
    direct_bindings: BTreeMap<GotId, usize>,
    last_rollback: Option<String>,
    last_candidate: Option<u64>,
    last_warnings: Vec<HostWarning>,
    last_rejection: Option<String>,
    scheduler: TaskScheduler,
    retain: RetainConfig,
    io: crate::io::IoRuntime,
    debug: DebugEngine,
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
            descriptors: DescriptorRegistry::new(),
            store,
            active: None,
            pending: None,
            rollback: None,
            retired: Vec::new(),
            direct_bindings: BTreeMap::new(),
            last_rollback: None,
            last_candidate: None,
            last_warnings: Vec::new(),
            last_rejection: None,
            scheduler: TaskScheduler::default(),
            retain: RetainConfig::default(),
            io: crate::io::IoRuntime::default(),
            debug: DebugEngine::new(),
        }
    }

    /// Overrides the process-image backend selected by metadata.
    ///
    /// CLI `--io`/`--io-file` values are explicit and survive online changes;
    /// embedding hosts call this before loading a module.
    pub fn configure_io(&mut self, backend: Option<crate::run::IoBackend>, file: Option<PathBuf>) {
        self.io.configure(backend, file);
    }

    /// Returns the process-image backend state.
    #[must_use]
    pub fn io_runtime(&self) -> &crate::io::IoRuntime {
        &self.io
    }

    /// Configures file-backed retained state for subsequent loads and the
    /// final shutdown.
    ///
    /// The runtime's state service is process-wide; the most recently
    /// configured host owns it. Embedding hosts supply absolute paths (D-163).
    pub fn configure_retain(&mut self, config: RetainConfig) {
        self.retain = config;
        crate::state::configure(self.retain.clone());
    }

    /// Returns the effective retained-state configuration.
    #[must_use]
    pub fn retain_config(&self) -> &RetainConfig {
        &self.retain
    }

    /// Installs the current configuration and permits (or forbids) saves.
    fn install_state_service(&self, save_generation: Option<u64>) {
        crate::state::configure(self.retain.clone());
        crate::state::allow_save(save_generation);
    }

    /// Merges state-service notices into the host warning list.
    fn collect_state_notices(&mut self) {
        for notice in crate::state::take_notices() {
            let warning = HostWarning {
                code: notice.code,
                message: notice.message,
            };
            if !self.last_warnings.contains(&warning) {
                self.last_warnings.push(warning);
            }
        }
    }

    /// Merges I/O-backend notices into the host warning list.
    fn collect_io_notices(&mut self) {
        for notice in self.io.take_notices() {
            let warning = HostWarning {
                code: notice.code,
                message: notice.message,
            };
            if !self.last_warnings.contains(&warning) {
                self.last_warnings.push(warning);
            }
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

    /// Returns the non-fatal diagnostics recorded by the latest activation.
    ///
    /// These are runtime observations such as a dropped root being quarantined
    /// or a direct variable being remapped. Compiler plan diagnostics remain in
    /// the manifest and in [`Self::diff`].
    #[must_use]
    pub fn warnings(&self) -> &[HostWarning] {
        &self.last_warnings
    }

    /// Returns a status snapshot for verbose runs and the control channel.
    #[must_use]
    pub fn status(&self) -> HostStatus {
        HostStatus {
            active_build_id: self
                .active
                .as_ref()
                .map(|generation| generation.manifest.header.build_id),
            pending_build_id: self
                .pending
                .as_ref()
                .map(|pending| pending.generation.manifest.header.build_id),
            retired_build_ids: self
                .retired
                .iter()
                .map(|generation| generation.manifest.header.build_id)
                .collect(),
            cycles: self
                .active
                .as_ref()
                .map_or(0, |generation| generation.cycles),
            last_rejection: self.last_rejection.clone(),
            warnings: self.last_warnings.clone(),
            tasks: self.scheduler.status(),
        }
    }

    /// Returns the per-task schedule state.
    #[must_use]
    pub fn task_status(&self) -> Vec<TaskStatus> {
        self.scheduler.status()
    }

    /// Releases every quarantined root's storage.
    ///
    /// Removed roots stay addressable until this explicit reset policy is
    /// applied; a host restart has the same effect. Any stale reference into
    /// released storage becomes invalid.
    ///
    /// # Errors
    ///
    /// Rejects a reset while a prepared candidate or rollback window still
    /// needs the quarantined roots to undo a change.
    pub fn reset_quarantine(&mut self) -> Result<usize> {
        if self.pending.is_some() || self.rollback.is_some() {
            return Err(prepare_error(
                "cannot reset quarantine while an online change is in progress",
            ));
        }
        let _ = self.descriptors.clear_quarantine();
        Ok(self.roots.clear_quarantine())
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

    /// Reads one typed debug target from the active generation.
    ///
    /// The read pins the active generation for the duration of the copy,
    /// resolves the target against the active manifest, and never takes a lock
    /// held by the module cycle. It is the synchronous embedding/test path;
    /// service threads submit requests through [`DebugService`] and drain the
    /// results after [`Host::debug_pump_at`].
    ///
    /// # Errors
    ///
    /// Returns the stable debug codes from Debug and Monitoring §15.8:
    /// unknown target, wrong type/shape, stale generation/build/type/layout,
    /// stop-frame failure, or a read limit.
    pub fn debug_read(&self, request: DebugReadRequest) -> Result<DebugRead> {
        crate::debug::validate_request(&request)?;
        let pin = self.pin_active().ok_or_else(|| {
            Error::new(
                codes::DEBUG_STALE,
                "no active generation is available for a typed read",
            )
        })?;
        let active = self.active.as_ref().ok_or_else(|| {
            Error::new(
                codes::DEBUG_STALE,
                "no active generation is available for a typed read",
            )
        })?;
        let source = crate::debug::DebugSource {
            target: &self.platform.target,
            manifest: &active.manifest,
            roots: &self.roots,
            info: active.info.as_ref(),
            generation: active.id,
            build_id: active.manifest.header.build_id,
            cycle: active.cycles,
        };
        let read = crate::debug::read_request(&source, &request);
        drop(pin);
        read
    }

    /// Returns a `Send + Sync` service facade.
    ///
    /// Phase 5 drives this facade from its protocol service thread. The facade
    /// only queues commands and drains events; the cycle thread applies them at
    /// [`Host::debug_pump_at`]. This is the documented handoff in M6.5.
    #[must_use]
    pub fn debug_service(&self) -> DebugService {
        self.debug.service()
    }

    /// Adds a watch applied immediately on this host's thread.
    ///
    /// Tests and embedding hosts use this path. Service clients use
    /// [`DebugService::add_watch`] so the command is applied by the cycle
    /// thread.
    ///
    /// # Errors
    ///
    /// Rejects a local target, an interval below 50 ms, or the 256-watch
    /// limit.
    pub fn debug_add_watch(&mut self, spec: WatchSpec) -> Result<WatchId> {
        self.debug.add_watch(spec)
    }

    /// Removes a watch.
    pub fn debug_remove_watch(&mut self, watch: WatchId) -> bool {
        self.debug.remove_watch(watch)
    }

    /// Returns the number of active watches.
    #[must_use]
    pub fn debug_watch_count(&self) -> usize {
        self.debug.watch_count()
    }

    /// Applies queued debug commands and emits due watch batches at a cycle
    /// boundary.
    ///
    /// The host holds a generation pin across the copy. A generation change
    /// ends every watch before commands for the new generation are applied.
    /// Returns the number of events emitted.
    pub fn debug_pump_at(&mut self, now_ns: i64) -> usize {
        let pin = self.pin_active();
        let target = &self.platform.target;
        let active = self.active.as_ref();
        let manifest = active.map(|generation| &generation.manifest);
        let info = active.and_then(|generation| generation.info.as_ref());
        let generation = active.map(|generation| generation.id);
        let build_id = active.map(|generation| generation.manifest.header.build_id);
        let cycle = active.map(|generation| generation.cycles);
        let source = match (manifest, generation, build_id, cycle) {
            (Some(manifest), Some(generation), Some(build_id), Some(cycle)) => {
                Some(crate::debug::DebugSource {
                    target,
                    manifest,
                    roots: &self.roots,
                    info,
                    generation,
                    build_id,
                    cycle,
                })
            }
            _ => None,
        };
        let count = self.debug.pump(now_ns, source);
        drop(pin);
        count
    }

    /// Drains queued debug events in order.
    #[must_use]
    pub fn debug_drain_events(&mut self) -> Vec<crate::debug::DebugEvent> {
        self.debug.drain_events()
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

    /// Returns one TYPEINFO or class VTABLE record's stable address.
    ///
    /// Descriptor records are host-owned and keep their address while their
    /// shape stays compatible across an online change.
    #[must_use]
    pub fn descriptor_address(&self, got_id: GotId) -> Option<usize> {
        self.descriptors.address(got_id)
    }

    /// Returns one TYPEINFO or class VTABLE record's words.
    ///
    /// The words are the ABI 2.1 runtime layout fixed in GOT Artifacts §8.5;
    /// callers must not write them.
    #[must_use]
    pub fn descriptor_words(&self, got_id: GotId) -> Option<&[u64]> {
        self.descriptors.words(got_id)
    }

    /// Returns one per-(object type, interface) VTABLE's stable address.
    #[must_use]
    pub fn interface_vtable_address(&self, owner: GotId, interface: GotId) -> Option<usize> {
        self.descriptors.interface_address(owner, interface)
    }

    /// Returns one per-(object type, interface) VTABLE's words.
    #[must_use]
    pub fn interface_vtable_words(&self, owner: GotId, interface: GotId) -> Option<&[u64]> {
        self.descriptors.interface_words(owner, interface)
    }

    /// Returns the number of quarantined descriptor records.
    #[must_use]
    pub fn quarantined_descriptors(&self) -> usize {
        self.descriptors.quarantined_len()
    }

    /// Returns the active generation's value for one GOT cell.
    #[must_use]
    pub fn active_cell_value(&self, slot: u32) -> Option<usize> {
        self.active
            .as_ref()
            .and_then(|generation| generation.got.cell_value(slot).ok())
    }

    /// Returns the active generation's address for one GOT cell.
    #[must_use]
    pub fn active_cell_address(&self, slot: u32) -> Option<usize> {
        self.active
            .as_ref()
            .and_then(|generation| generation.got.cell_address(slot).ok())
            .map(|address| address as usize)
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
    fn apply_info(&mut self, info: &ModuleInfo, module_dir: &Path) -> Result<()> {
        crate::stc_rt_io_configure(info.inputs_bytes, info.outputs_bytes, info.markers_bytes);
        self.direct_bindings = direct_bindings_for(info)?;
        self.io.apply_metadata(info, module_dir);
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
        let result = self.load_inner(candidate, mode);
        match &result {
            Ok(LoadOutcome::ColdRequired { reason }) => {
                self.last_rejection = Some(reason.clone());
            }
            Ok(_) => self.last_rejection = None,
            Err(error) => {
                self.last_rejection = Some(format!("{}: {}", error.code, error.message));
            }
        }
        result
    }

    fn load_inner(&mut self, candidate: Candidate, mode: LoadMode) -> Result<LoadOutcome> {
        // Reject a scheduling configuration the host cannot run before any
        // host state changes: every task needs a host-owned guard root.
        TaskScheduler::from_manifest(candidate.manifest())?;
        migration::validate_deployable(candidate.manifest())?;
        validate_direct_metadata(
            candidate.manifest(),
            candidate.info(),
            &self.direct_bindings,
        )?;
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

    /// Returns the process-image work the candidate needs at activation.
    ///
    /// `None` means the candidate's bindings are already the active ones (or
    /// the candidate declares no direct variables), so activation only swaps
    /// the GOT.
    fn pending_image(&self, candidate: &Candidate) -> Option<PendingImage> {
        let info = candidate.info()?.clone();
        let active_info = self
            .active
            .as_ref()
            .and_then(|generation| generation.info.as_ref());
        let layout = info.image_layout();
        if active_info.is_none() && layout.total_bytes() == Some(0) && info.bindings.is_empty() {
            return None;
        }
        let reconfigure = active_info.is_none_or(|active| !active.image_layout().eq(&layout));
        let changed =
            reconfigure || active_info.is_none_or(|active| active.bindings != info.bindings);
        changed.then_some(PendingImage { info, reconfigure })
    }

    /// Returns the runtime warnings for an online candidate.
    fn change_warnings(&self, candidate: &Candidate) -> Vec<HostWarning> {
        let mut warnings = Vec::new();
        for action in &candidate.manifest().plan.actions {
            if action.action == PlanAction::Drop && action.kind == IdentityKind::DataRoot.as_u8() {
                warnings.push(HostWarning {
                    code: codes::W_ONLINE_REMOVED,
                    message: format!("root `{}` was removed and quarantined", action.key),
                });
            }
        }
        if let Some(info) = candidate.info()
            && let Some(active_info) = self
                .active
                .as_ref()
                .and_then(|generation| generation.info.as_ref())
        {
            for remap in image::remaps(active_info, info) {
                let key = candidate
                    .manifest()
                    .roots
                    .iter()
                    .find(|root| root.got_id == remap.got_id)
                    .map_or("direct variable", |root| root.key.as_str());
                warnings.push(HostWarning {
                    code: codes::W_ONLINE_IO,
                    message: format!(
                        "direct variable `{key}` was remapped from area {} offset {} to area {} offset {}",
                        remap.previous.area,
                        remap.previous.offset_bits,
                        remap.current.area,
                        remap.current.offset_bits
                    ),
                });
            }
        }
        warnings.sort_by(|left, right| {
            (left.code, left.message.as_str()).cmp(&(right.code, right.message.as_str()))
        });
        warnings
    }

    fn prepare_online(&mut self, candidate: Candidate) -> Result<LoadOutcome> {
        // An online attach never reads the state file (State §4.1.1); the
        // service is installed so a shutdown during the swap cannot save.
        self.install_state_service(None);
        let io_info = candidate
            .info()
            .map(|info| (info.clone(), candidate.module_dir().to_path_buf()));
        let image_plan = self.pending_image(&candidate);
        let warnings = self.change_warnings(&candidate);
        let direct = match &image_plan {
            Some(plan) => {
                let layout = plan.info.image_layout();
                image::planned_bindings(layout, &plan.info.bindings)?
            }
            None => self.direct_bindings.clone(),
        };
        let manifest = candidate.manifest().clone();
        let scheduler = self.scheduler.reconcile(&manifest)?;
        let live = live_descriptor_types(&manifest)?;
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

        match Self::build_generation(
            candidate,
            &self.roots,
            generation_id,
            &fresh,
            1,
            &direct,
            &mut self.descriptors,
            &live,
        ) {
            Ok((generation, descriptors)) => {
                if let Some(failure) = crate::state::take_failure() {
                    undo.undo(&mut self.roots)?;
                    return Err(Error::new(failure.code, failure.message));
                }
                self.pending = Some(Pending {
                    generation,
                    undo,
                    descriptors,
                    image: image_plan,
                    io_info,
                    warnings,
                    scheduler,
                });
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
        let previous_io = self.io.clone();
        let image = candidate.info().map(|_| crate::image_snapshot());
        let generation_id = allocate_generation();
        self.last_candidate = Some(generation_id);
        self.install_state_service(None);
        let prepared = (|| -> Result<(Generation, RootRegistry, DescriptorRegistry, PreparedDescriptors, TaskScheduler)> {
            if let Some(info) = candidate.info() {
                let module_dir = candidate.module_dir().to_path_buf();
                self.apply_info(info, &module_dir)?;
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
            let direct = self.direct_bindings.clone();
            let mut descriptors = DescriptorRegistry::new();
            let (generation, prepared) = Self::build_generation(
                candidate,
                &roots,
                generation_id,
                &fresh,
                0,
                &direct,
                &mut descriptors,
                &BTreeSet::new(),
            )?;
            if let Some(failure) = crate::state::take_failure() {
                return Err(Error::new(failure.code, failure.message));
            }
            self.io.cold_start();
            let scheduler = TaskScheduler::from_manifest(&generation.manifest)?;
            let stored = StoredManifest::new(generation.manifest.clone())?;
            self.store.install(stored)?;
            Ok((generation, roots, descriptors, prepared, scheduler))
        })();
        let (generation, roots, mut descriptors, prepared, scheduler) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                if let Some(image) = image {
                    crate::restore_image(image);
                    self.restore_direct_bindings(previous_bindings);
                } else {
                    self.direct_bindings = previous_bindings;
                }
                self.io = previous_io;
                return Err(error);
            }
        };
        let activation = descriptors.activate(prepared);
        descriptors.commit(activation);

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
        self.debug.end_watches();
        self.scheduler = scheduler;
        self.last_rollback = None;
        self.last_warnings.clear();
        self.collect_state_notices();
        self.collect_io_notices();
        // Retired generations still hold descriptor cell addresses from the
        // old registry; unload them before dropping its records.
        self.unload_retired();
        self.descriptors = descriptors;
        Ok(LoadOutcome::ColdInstalled {
            generation: generation_id,
        })
    }

    /// Builds a generation's GOT and descriptor records and runs prepare.
    ///
    /// Descriptor records are prepared before `stc_module_prepare` so fresh
    /// object records can store their TYPEINFO addresses. Activation patches
    /// the shared records afterwards; a prepare failure reverts the registry
    /// changes before returning.
    #[allow(clippy::too_many_arguments)]
    fn build_generation(
        candidate: Candidate,
        roots: &RootRegistry,
        generation_id: u64,
        fresh: &BTreeSet<GotId>,
        mode: u32,
        direct: &BTreeMap<GotId, usize>,
        descriptors: &mut DescriptorRegistry,
        live: &BTreeSet<GotId>,
    ) -> Result<(Generation, PreparedDescriptors)> {
        let Candidate {
            module,
            pair,
            info,
            module_dir: _,
        } = candidate;
        let manifest = pair.manifest;
        let manifest_digest = pair.manifest_digest;
        let slot_count = u32::try_from(manifest.slots.len())
            .map_err(|_| Error::new(codes::ONLINE_BINDING, "slot count exceeds u32"))?;

        host_api::register_generation(generation_id)?;
        let result = (|| -> Result<(Generation, PreparedDescriptors)> {
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
            let function_cells = function_cell_addresses(&manifest, &got)?;
            let prepared = descriptors.prepare(&manifest, live, &function_cells)?;

            let run = (|| -> Result<Generation> {
                got.bind_descriptors(prepared.bindings())?;
                got.bind_direct(direct)?;
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
                let mask_size = u32::try_from(mask.len()).map_err(|_| {
                    Error::new(codes::ONLINE_BINDING, "fresh-root mask exceeds u32")
                })?;
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
            match run {
                Ok(generation) => Ok((generation, prepared)),
                Err(error) => {
                    descriptors.revert_prepare(prepared);
                    Err(error)
                }
            }
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
        let Some(mut pending) = self.pending.take() else {
            return Err(prepare_error("no prepared candidate to activate"));
        };
        if self.active.is_none() {
            self.pending = Some(pending);
            return Err(prepare_error("no active generation to replace"));
        }
        let mut image_rollback = None;
        if let Some(image) = pending.image.as_ref() {
            let previous_info = self
                .active
                .as_ref()
                .and_then(|generation| generation.info.clone());
            let snapshot = image::snapshot();
            let applied = if image.reconfigure {
                image::reconfigure(&snapshot, previous_info.as_ref(), &image.info)
            } else {
                image::remap_in_place(&snapshot, previous_info.as_ref(), &image.info)
            };
            match applied {
                Ok(bindings) => {
                    if let Err(error) = pending.generation.got.bind_direct(&bindings) {
                        self.restore_image_state(&snapshot);
                        self.pending = Some(pending);
                        return Err(error);
                    }
                    let bindings_before = std::mem::replace(&mut self.direct_bindings, bindings);
                    image_rollback = Some(ImageRollback {
                        snapshot,
                        bindings: bindings_before,
                    });
                }
                Err(error) => {
                    self.restore_image_state(&snapshot);
                    self.pending = Some(pending);
                    return Err(error);
                }
            }
        }
        let Some(previous) = self.active.take() else {
            self.pending = Some(pending);
            return Err(prepare_error("no active generation to replace"));
        };
        let Pending {
            generation: new_generation,
            undo,
            descriptors,
            image: _,
            io_info,
            warnings,
            scheduler,
        } = pending;
        if let Some((info, module_dir)) = &io_info {
            self.io.apply_metadata(info, module_dir);
        }
        let generation = new_generation.id;
        let descriptor_activation = self.descriptors.activate(descriptors);
        self.rollback = Some(Rollback {
            generation: previous,
            undo,
            descriptors: descriptor_activation,
            successful_cycles: 0,
            image: image_rollback,
            scheduler: std::mem::replace(&mut self.scheduler, scheduler),
        });
        self.active = Some(new_generation);
        self.debug.end_watches();
        self.last_warnings = warnings;
        Ok(generation)
    }

    /// Restores the process image after a failed activation and rebinds the
    /// active generation's direct cells, whose addresses may have moved.
    fn restore_image_state(&mut self, snapshot: &ImageSnapshot) {
        image::restore(snapshot);
        self.rebind_active_direct();
    }

    /// Recomputes and rebinds the active generation's direct cells.
    fn rebind_active_direct(&mut self) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let Some(info) = active.info.as_ref() else {
            return;
        };
        if let Ok(bindings) = image::current_bindings(info) {
            let _ = active.got.bind_direct(&bindings);
            self.direct_bindings = bindings;
        }
    }

    /// Runs one cycle of the active generation at the latched clock time.
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
        let now = crate::latch_cycle_clock();
        self.cycle_at(now)
    }

    /// Runs one cycle with an explicit scan time.
    ///
    /// Embedding hosts that own the loop use this to supply the mock clock's
    /// next value. The value becomes the latched `cycle_now_ns` the module
    /// reads, and the task scheduler uses the same value.
    ///
    /// # Errors
    ///
    /// Rejects a call with no active generation or a cycle failure outside a
    /// rollback window.
    pub fn cycle_at(&mut self, now_ns: i64) -> Result<CycleOutcome> {
        crate::set_cycle_now(now_ns);
        let Some(generation_id) = self.active.as_ref().map(|generation| generation.id) else {
            return Err(prepare_error("no active generation"));
        };
        // Debug service work is applied at the boundary, before inputs are
        // latched. The engine pins the active generation, copies only from
        // immutable root storage, and never holds a cycle lock.
        self.debug_pump_at(now_ns);
        // Inputs are latched once, then every due task runs against the same
        // image; outputs publish after the cycle (IO §6.3).
        self.io.latch_inputs();
        self.scheduler.begin_scan(now_ns);
        if let Err(error) = self.write_task_guards() {
            self.scheduler.finish_scan(false);
            return Err(error);
        }
        let outcome = {
            let generation = self.active.as_ref().expect("checked above");
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
                self.scheduler.finish_scan(false);
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
            self.scheduler.finish_scan(false);
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
        self.io.publish_outputs();
        self.scheduler.finish_scan(true);
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

    /// Writes every task's guard root for the scan prepared by the scheduler.
    fn write_task_guards(&mut self) -> Result<()> {
        let guards: Vec<(GotId, bool)> = self.scheduler.guards().collect();
        for (got_id, due) in guards {
            let Some(root) = self.roots.get_mut(got_id) else {
                return Err(Error::new(
                    codes::RUN_OPTION,
                    format!("task-guard root {} is not active", got_id.to_hex()),
                ));
            };
            let Some(slot) = root.storage_mut().first_mut() else {
                return Err(Error::new(
                    codes::RUN_OPTION,
                    format!("task-guard root {} has no storage", got_id.to_hex()),
                ));
            };
            *slot = u8::from(due);
        }
        Ok(())
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
        self.descriptors.commit(rollback.descriptors);
        // A retired generation shutting down during a swap must not rewrite
        // the state file (State §4.1.1).
        self.install_state_service(None);
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
        let Rollback {
            generation: restored_generation,
            undo,
            descriptors,
            image,
            scheduler,
            ..
        } = rollback;
        let restored = restored_generation.id;
        self.active = Some(restored_generation);
        self.debug.end_watches();
        self.scheduler = scheduler;
        if let Some(image) = image {
            image::restore(&image.snapshot);
            self.direct_bindings = image.bindings;
            self.rebind_active_direct();
        }
        self.descriptors.rollback(descriptors);
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
        let Pending {
            generation,
            undo,
            descriptors,
            ..
        } = pending;
        drop(generation);
        self.descriptors.revert_prepare(descriptors);
        undo.undo(&mut self.roots)
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
        // Only the active generation's normal shutdown may write the state
        // file; retired generations shut down here without saving.
        let save_generation = self.active.as_ref().map(|generation| generation.id);
        self.install_state_service(save_generation);
        let io_error = self.io.flush_output();
        self.collect_io_notices();
        let mut first_error = io_error.map(|message| Error::new("STC-E-RUN-IO-001", message));
        if let Some(error) = self.abort_pending().err() {
            first_error.get_or_insert(error);
        }
        if self.rollback.is_some()
            && let Err(error) = self.rollback_internal("host shutdown")
        {
            first_error.get_or_insert(error);
        }
        if let Some(mut active) = self.active.take() {
            active.shutdown();
            self.retired.push(active);
        }
        self.debug.end_watches();
        for mut generation in self.retired.drain(..) {
            generation.shutdown();
            generation.detach();
        }
        let failure = crate::state::take_failure();
        self.collect_state_notices();
        if let Some(failure) = failure {
            // The failure also appears as a notice; report it once as an error.
            self.last_warnings
                .retain(|warning| warning.code != failure.code);
            first_error.get_or_insert(Error::new(failure.code, failure.message));
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

/// Validates a candidate's direct roots against its metadata or host bindings.
///
/// Compiler-produced modules carry one metadata binding per `Direct` root.
/// Modules without a sidecar may still be installed by an embedding host that
/// supplies the bindings explicitly; those must cover every direct root.
fn validate_direct_metadata(
    manifest: &Manifest,
    info: Option<&ModuleInfo>,
    fallback: &BTreeMap<GotId, usize>,
) -> Result<()> {
    let direct: Vec<&RootRecord> = manifest
        .roots
        .iter()
        .filter(|root| root.root_kind == RootKind::Direct)
        .collect();
    match info {
        Some(info) => {
            let layout = info.image_layout();
            image::validate_layout(layout, &info.bindings)?;
            let direct_ids: BTreeSet<GotId> = direct.iter().map(|root| root.got_id).collect();
            let bound: BTreeSet<GotId> =
                info.bindings.iter().map(|binding| binding.got_id).collect();
            for binding in &info.bindings {
                if !direct_ids.contains(&binding.got_id) {
                    return Err(Error::new(
                        codes::ONLINE_IO,
                        format!(
                            "process-image metadata binds {} but the manifest has no such direct root",
                            binding.got_id
                        ),
                    ));
                }
            }
            for root in direct {
                if !bound.contains(&root.got_id) {
                    return Err(Error::new(
                        codes::ONLINE_BINDING,
                        format!(
                            "direct variable `{}` has no process-image binding",
                            root.key
                        ),
                    ));
                }
            }
        }
        None => {
            for root in direct {
                if !fallback.contains_key(&root.got_id) {
                    return Err(Error::new(
                        codes::ONLINE_BINDING,
                        format!(
                            "direct variable `{}` has no process-image binding",
                            root.key
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
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

/// Returns the named type identities that preserved roots can depend on.
///
/// A preserved class handle or object record names its class type; a preserved
/// interface value names its interface type. The registry expands interface
/// dependencies to the object types that implement them and class
/// dependencies to their derived types.
fn live_descriptor_types(manifest: &Manifest) -> Result<BTreeSet<GotId>> {
    let preserved: BTreeSet<GotId> = manifest
        .plan
        .actions
        .iter()
        .filter(|action| {
            action.kind == IdentityKind::DataRoot.as_u8() && action.action == PlanAction::Preserve
        })
        .map(|action| action.got_id)
        .collect();
    let mut live = BTreeSet::new();
    for root in &manifest.roots {
        if !preserved.contains(&root.got_id) {
            continue;
        }
        let key = TypeKey::parse(&root.type_key).map_err(|error| {
            Error::new(
                codes::ONLINE_VTABLE,
                format!("preserved root `{}` type key: {}", root.key, error.message),
            )
        })?;
        collect_named_types(&key, &mut live);
    }
    Ok(live)
}

fn collect_named_types(key: &TypeKey, out: &mut BTreeSet<GotId>) {
    match key {
        TypeKey::Named(got_id) => {
            out.insert(*got_id);
        }
        TypeKey::Array { element, .. } | TypeKey::RefTo(element) | TypeKey::PtrTo(element) => {
            collect_named_types(element, out);
        }
        _ => {}
    }
}

fn function_cell_addresses(manifest: &Manifest, got: &Got) -> Result<BTreeMap<GotId, usize>> {
    let mut cells = BTreeMap::new();
    for slot in &manifest.slots {
        if slot.kind != SlotKind::Function {
            continue;
        }
        cells.insert(slot.got_id, got.cell_address(slot.slot)? as usize);
    }
    Ok(cells)
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
