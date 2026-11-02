//! The persistent host runtime for generated ST programs.
//!
//! The crate is a Rust `staticlib` exposing the stable C ABI from
//! Runtime-and-Execution §2.3 (D-077). It owns the process image, the latched
//! cycle clock, trap reporting, and the executable cycle loop. Retained-state
//! file I/O is deliberately a stub until M6: the entry points exist and return
//! "retention disabled" (5).
//!
//! M4.5 adds the host-side online-change substrate:
//!
//! - [`got`] owns the per-generation cell table and binds data, function,
//!   descriptor, direct, and import cells;
//! - [`descriptors`] owns the host's TYPEINFO, class VTABLE, and
//!   per-(object type, interface) VTABLE records, their stable addresses, and
//!   their quarantine;
//! - [`memory`] owns persistent roots, quarantine, and transient module blocks;
//! - [`catalog`] validates descriptor/manifest pairs against host policy;
//! - [`migration`] validates an online-change plan against the active baseline;
//! - [`loader`] maps a native module and copies its descriptor and function
//!   table;
//! - [`host_api`] provides the ABI 2.3 service table;
//! - [`store`] persists the committed manifest baseline and builds downloads;
//! - [`host`] owns the load/prepare/activate/commit/rollback/unload state
//!   machine and the generation-bound root and GOT state;
//! - [`module_info`] reads the process-image and direct-binding metadata
//!   sidecar;
//! - [`image`] validates image layouts and applies direct-variable remaps at
//!   activation, preserving values by `GotId`;
//! - [`scheduler`] owns cooperative task run state keyed by task `GotId` and
//!   computes each scan's guards, deadlines, and `SINGLE` completion;
//! - [`run`] is the shared launcher/embedding run loop (cycle count, cycle
//!   deadline, mock clock, stop conditions, exit codes);
//! - [`control`] is the local named-pipe/Unix-socket deploy/status channel
//!   (protocol version, per-run nonce, user-only access) and queues work for
//!   the cycle thread;
//! - [`compare`] is the manifest-only `stc online diff` entry point;
//! - [`state`] owns the file-backed retained-state format, atomic save, and
//!   the process-wide ABI 2 state services;
//! - [`io`] owns the `null`/`memory`/`file` process-image backends and the
//!   per-scan input latch/output publish;
//! - [`coordinator`] owns the shared process image, per-device backend
//!   dispatch, route application, and per-backend fault isolation
//!   (IO-Routing-and-Backends §3–§4);
//! - [`io_mock`] is the protocol-neutral mock backend used by the coordinator
//!   tests; on Linux `ethercat`/`ethercat-status` are served by `io_ethercat`
//!   instead, and the mock remains the default elsewhere;
//! - `io_ethercat` (Linux only) dynamically loads the project-owned IgH shim
//!   and owns EtherCAT configuration, startup, fault/recovery, and status;
//! - `signal` installs the Ctrl-C/Ctrl-Break stop handlers;
//! - `trap_boundary` provides the non-local trap exit around module calls.
//!
//! Generated ABI 2 modules execute through [`stc_rt_execute`] (the thin
//! launcher path) or the `stc_host_*` embedding exports, including
//! [`stc_host_run`]. Host ABI 2.3 appends the `debug_check` service (ID 13) for
//! debug builds with checkpoint hooks; it is a lock-free no-op without a debug
//! session and is the phase-6 breakpoint hook point. The legacy ABI 1
//! helper symbols remain as host utilities for the process image, clock, and
//! retained-state stubs; generated modules do not import them.
//!
//! The runtime is the only workspace crate that permits unsafe code; every
//! unsafe operation is an FFI pointer access confined to this crate.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) mod backend_defaults;
pub mod catalog;
pub mod compare;
pub mod control;
pub mod coordinator;
pub mod debug;
pub mod debug_server;
pub mod debug_view;
pub mod descriptors;
mod frames;
pub mod got;
pub mod host;
pub mod host_api;
pub mod image;
pub mod io;
#[cfg(target_os = "linux")]
pub mod io_ethercat;
pub mod io_mock;
pub mod loader;
pub mod memory;
pub mod migration;
pub mod module_info;
mod priority;
mod rt;
pub mod run;
pub mod scheduler;
pub mod signal;
pub mod state;
mod stop;
pub mod store;
mod test_host;
pub mod transport;
mod trap_boundary;

#[cfg(test)]
mod test_support;

pub use catalog::{HostPlatform, PROFILE, SUPPORTED_FEATURES, ValidatedPair};
pub use compare::compare_manifests;
pub use control::{
    CONTROL_UNAVAILABLE, ControlClient, ControlOp, ControlRequest, ControlServer, ControlSignal,
    DEPLOY_REJECTED, PROTOCOL_VERSION,
};
pub use coordinator::{
    BackendBinding, BackendChannel, BackendConfig, BackendDeviceConfig, BackendRegistry,
    BackendReport, BackendState, BackendStatus, Coordinator, DeviceBackend, ImageRange,
    ProcessImage,
};
pub use debug::{
    AuditEvent, AuditOutcome, AuditSink, AuditTarget, DebugBatch, DebugEndReason, DebugError,
    DebugEvent, DebugFailure, DebugQueueId, DebugQueueOutcome, DebugQueueResult, DebugRead,
    DebugReadRequest, DebugService, DebugTarget, DebugValue, DebugWarning, DebugWriteKind,
    DebugWriteRequest, MAX_AUDIT_VALUE_BYTES, MAX_DEBUG_QUEUE_BYTES, MAX_DEBUG_QUEUE_ENTRIES,
    MAX_DEBUG_QUEUE_RESULTS, MAX_EVENT_BYTES, MAX_WATCH_BATCHES, MAX_WATCHES,
    MIN_WATCH_INTERVAL_NS, NamedValue, ReadCompletion, ReadRequestId, StopFrame, StoppedLocal,
    StringValue, TypedValue, WatchId, WatchResult, WatchSpec, WatchValue,
};
pub use debug_server::{
    DEBUG_PROTOCOL, DebugClient, DebugClientError, DebugEndpointFile, DebugServer,
    DebugServerConfig, DebugServerInfo, MAX_MESSAGE_BYTES, MAX_REQUEST_BYTES,
};
pub use debug_view::{
    IdentityKind, SplitView, ViewField, ViewIdentity, ViewKind, ViewNode, ViewOrigin, ViewRoot,
    escape_text, node_json, render_text, render_view_json, view_json,
};
pub use got::Got;
pub use host::{
    Candidate, CycleOutcome, GenerationPin, Host, HostStatus, HostWarning, LoadMode, LoadOutcome,
    ManifestDiff,
};
pub use host_api::{ABI_VERSION, HOST_API, HostApi, TrapRecord};
pub use image::{DirectRemap, MAX_IMAGE_BYTES};
pub use io::{IoNotice, IoRuntime};
#[cfg(target_os = "linux")]
pub use io_ethercat::{EthercatBackend, SHIM_ENV, SHIM_LIBRARY, ShimInfo, probe_shim};
pub use io_mock::{MockEthercatBackend, MockWire};
pub use loader::LoadedModule;
pub use memory::{MemoryLimits, RootRegistry, RootStorage, TransientAllocator};
pub use migration::{PlanDecision, validate_deployable, validate_plan};
pub use module_info::{DirectBinding, ImageLayout, ModuleInfo};
pub use run::{IoBackend, RunLoopError, RunOptions, run_loop};
pub use scheduler::{TaskScheduler, TaskStatus};
pub use state::{LoadOutcome as StateLoadOutcome, MismatchPolicy, RetainConfig, StateFailure};
pub use store::{
    Deployment, Download, LayoutState, ManifestStore, RuntimeLayout, RuntimeLayoutEntry,
    StoredManifest,
};
pub use transport::TransportKind;

/// Runtime ABI version (Runtime-and-Execution §2.2).
pub const STC_RUNTIME_ABI: i32 = 2;

/// Exit status used when a trap is raised outside a host boundary.
pub const STC_TRAP_EXIT_CODE: i32 = 70;

/// Exit status for a trap during `stc run` (CLI-and-Manifest §1.3).
pub const STC_RUN_TRAP_EXIT_CODE: i32 = 4;

/// Trap codes (Runtime-and-Execution §2.3).
const TRAP_OVERFLOW: i32 = 1;
const TRAP_DIV_ZERO: i32 = 2;
const TRAP_RANGE: i32 = 3;
const TRAP_BOUNDS: i32 = 4;
const TRAP_NULL_REF: i32 = 5;
const TRAP_INVALID_UTF8: i32 = 6;
const TRAP_CONVERSION: i32 = 7;
const TRAP_WATCHDOG: i32 = 8;
const TRAP_ASSERT: i32 = 9;
const TRAP_UNIMPLEMENTED: i32 = 10;

struct State {
    image: Vec<u8>,
    inputs: u64,
    outputs: u64,
    markers: u64,
    cycle_now_ns: i64,
    pending_cycle_ns: Option<i64>,
}

static STATE: Mutex<State> = Mutex::new(State {
    image: Vec::new(),
    inputs: 0,
    outputs: 0,
    markers: 0,
    cycle_now_ns: 0,
    pending_cycle_ns: None,
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn trap_name(code: i32) -> &'static str {
    match code {
        TRAP_OVERFLOW => "OVERFLOW",
        TRAP_DIV_ZERO => "DIV_ZERO",
        TRAP_RANGE => "RANGE",
        TRAP_BOUNDS => "BOUNDS",
        TRAP_NULL_REF => "NULL_REF",
        TRAP_INVALID_UTF8 => "INVALID_UTF8",
        TRAP_CONVERSION => "CONVERSION",
        TRAP_WATCHDOG => "WATCHDOG",
        TRAP_ASSERT => "ASSERT",
        TRAP_UNIMPLEMENTED => "UNIMPLEMENTED",
        _ => "UNKNOWN",
    }
}

/// Reports a fatal runtime trap and exits.
///
/// M4 prints the trap name and span ID only; M6 formats a full source location
/// from the metadata span table.
#[unsafe(no_mangle)]
pub extern "C" fn stc_rt_trap(code: i32, span_id: u32) {
    eprintln!(
        "stc: runtime trap: {:?} (code {code}) at span {span_id}",
        trap_name(code)
    );
    std::process::exit(STC_TRAP_EXIT_CODE);
}

/// Returns the runtime ABI version.
#[unsafe(no_mangle)]
pub extern "C" fn stc_abi_version() -> i32 {
    STC_RUNTIME_ABI
}

/// Runs the persistent host loop for a loadable module.
///
/// `module_path` names the module file; when it has no directory component the
/// launcher's directory is used. The adjacent `<name>.stc-got.bin` manifest and
/// `<name>.stc-meta.json` metadata are loaded automatically.
///
/// Accepted options: `--cycles <n>` (zero runs until interrupted),
/// `--cycle-time <duration>`, `--io null|memory|file`, `--io-file <path>`,
/// `--state-file <path>`, `--reset-retain`, `--no-retain`,
/// `--mock-clock <duration>`, `--status`, and `--module <path>`.
///
/// A relative `--state-file`/`--io-file`, or a relative `retain.file`/
/// `io.file` from the metadata sidecar, resolves against the module directory
/// (D-163). `stc run` passes absolute paths instead. Without a metadata
/// sidecar, retained state stays disabled.
///
/// # Safety
///
/// `argv` must point to `argc` valid, NUL-terminated strings. `module_path`
/// must be null or point to a NUL-terminated string. Both stay valid for the
/// duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_rt_execute(
    argc: i32,
    argv: *const *const c_char,
    module_path: *const c_char,
) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute_module(argc, argv, module_path)
    }));
    match result {
        Ok(code) => code,
        Err(_) => {
            eprintln!("stc: internal runtime panic");
            3
        }
    }
}

/// Parsed launcher arguments.
struct LauncherOptions {
    run: RunOptions,
    module: Option<String>,
    control_endpoint: Option<String>,
    control_nonce: Option<String>,
    manifest_dir: Option<PathBuf>,
    debug_endpoint: Option<PathBuf>,
    debug_protocol: crate::transport::TransportKind,
    debug_allow_write: bool,
    debug_allow_force: bool,
    debug_session_timeout_ns: Option<u64>,
}

fn parse_launcher_args(args: &[String]) -> Result<LauncherOptions, String> {
    let mut run = RunOptions::default();
    let mut module = None;
    let mut control_endpoint = None;
    let mut control_nonce = None;
    let mut manifest_dir = None;
    let mut debug_endpoint = None;
    let mut debug_protocol = crate::transport::TransportKind::Pipe;
    let mut debug_allow_write = false;
    let mut debug_allow_force = false;
    let mut debug_session_timeout_ns = None;
    let mut index = usize::from(!args.is_empty());
    while index < args.len() {
        let argument = args[index].as_str();
        let mut value = |option: &str| -> Result<String, String> {
            index += 1;
            args.get(index)
                .cloned()
                .ok_or_else(|| format!("`{option}` requires a value"))
        };
        match argument {
            "--cycles" => {
                let text = value("--cycles")?;
                run.cycles = text
                    .parse::<u64>()
                    .map_err(|_| format!("invalid cycle count `{text}`"))?;
            }
            "--cycle-time" => {
                let text = value("--cycle-time")?;
                let ns = run::parse_duration_ns(&text)
                    .ok_or_else(|| format!("invalid cycle time `{text}`"))?;
                run.cycle_time_ns = Some(i64::try_from(ns).unwrap_or(i64::MAX));
            }
            "--io" => {
                let text = value("--io")?;
                run.io = Some(IoBackend::parse(&text).ok_or_else(|| {
                    format!(
                        "unknown I/O backend `{text}` (expected null, memory, file, or ethercat)"
                    )
                })?);
            }
            "--io-file" => {
                run.io_file = Some(PathBuf::from(value("--io-file")?));
            }
            "--state-file" => {
                run.state_file = Some(PathBuf::from(value("--state-file")?));
            }
            "--reset-retain" => run.reset_retain = true,
            "--no-retain" => run.no_retain = true,
            "--mock-clock" => {
                let text = value("--mock-clock")?;
                let ns = run::parse_duration_ns(&text)
                    .ok_or_else(|| format!("invalid mock clock `{text}`"))?;
                run.mock_clock_ns = Some(i64::try_from(ns).unwrap_or(i64::MAX));
            }
            "--rt" => run.rt = true,
            "--rt-priority" => {
                let text = value("--rt-priority")?;
                run.rt_priority = Some(parse_rt_priority(&text)?);
            }
            other if other.starts_with("--rt-priority=") => {
                run.rt_priority = Some(parse_rt_priority(&other["--rt-priority=".len()..])?);
            }
            "--rt-cpu" => {
                let text = value("--rt-cpu")?;
                run.rt_cpu = Some(parse_rt_cpu(&text)?);
            }
            other if other.starts_with("--rt-cpu=") => {
                run.rt_cpu = Some(parse_rt_cpu(&other["--rt-cpu=".len()..])?);
            }
            "--rt-metrics" => {
                let path = value("--rt-metrics")?;
                run.rt_metrics = Some(parse_rt_metrics_path(&path)?);
            }
            other if other.starts_with("--rt-metrics=") => {
                run.rt_metrics = Some(parse_rt_metrics_path(&other["--rt-metrics=".len()..])?);
            }
            "--status" => run.status = true,
            "--module" => {
                module = Some(value("--module")?);
            }
            "--control" => {
                control_endpoint = Some(value("--control")?);
            }
            "--control-nonce" => {
                control_nonce = Some(value("--control-nonce")?);
            }
            "--manifest-dir" => {
                manifest_dir = Some(PathBuf::from(value("--manifest-dir")?));
            }
            "--debug-endpoint" => {
                debug_endpoint = Some(PathBuf::from(value("--debug-endpoint")?));
            }
            "--debug-protocol" => {
                let text = value("--debug-protocol")?;
                debug_protocol =
                    crate::transport::TransportKind::parse(&text).ok_or_else(|| {
                        format!("unknown debug transport `{text}` (expected pipe or socket)")
                    })?;
            }
            "--debug-allow-write" => debug_allow_write = true,
            "--debug-allow-force" => debug_allow_force = true,
            "--debug-session-timeout" => {
                let text = value("--debug-session-timeout")?;
                let ns = run::parse_duration_ns(&text)
                    .ok_or_else(|| format!("invalid debug session timeout `{text}`"))?;
                if ns == 0 {
                    return Err("the debug session timeout must be positive".into());
                }
                debug_session_timeout_ns = Some(ns);
            }
            "--trace" => {
                return Err("runtime option `--trace` is not supported in this build".into());
            }
            other => return Err(format!("unknown runtime option `{other}`")),
        }
        index += 1;
    }
    if control_endpoint.is_some() && control_nonce.is_none() {
        return Err("`--control` requires `--control-nonce`".into());
    }
    if debug_allow_write && debug_endpoint.is_none() {
        return Err("`--debug-allow-write` requires `--debug-endpoint`".into());
    }
    if debug_allow_force && debug_endpoint.is_none() {
        return Err("`--debug-allow-force` requires `--debug-endpoint`".into());
    }
    if debug_session_timeout_ns.is_some() && debug_endpoint.is_none() {
        return Err("`--debug-session-timeout` requires `--debug-endpoint`".into());
    }
    if !run.rt {
        if run.rt_priority.is_some() || run.rt_cpu.is_some() || run.rt_metrics.is_some() {
            return Err("`--rt-priority`, `--rt-cpu`, and `--rt-metrics` require `--rt`".into());
        }
    } else {
        if run.cycle_time_ns.is_none_or(|ns| ns <= 0) {
            return Err("`--rt` requires a positive `--cycle-time`".into());
        }
        if run.mock_clock_ns.is_some() {
            return Err("`--rt` cannot be combined with `--mock-clock`".into());
        }
    }
    Ok(LauncherOptions {
        run,
        module,
        control_endpoint,
        control_nonce,
        manifest_dir,
        debug_endpoint,
        debug_protocol,
        debug_allow_write,
        debug_allow_force,
        debug_session_timeout_ns,
    })
}

fn parse_rt_priority(text: &str) -> Result<u8, String> {
    let priority: u8 = text
        .parse()
        .map_err(|_| format!("invalid RT priority `{text}`"))?;
    if !(1..=99).contains(&priority) {
        return Err(format!("RT priority `{text}` must be between 1 and 99"));
    }
    Ok(priority)
}

fn parse_rt_cpu(text: &str) -> Result<u32, String> {
    let cpu: u32 = text
        .parse()
        .map_err(|_| format!("invalid RT CPU `{text}`"))?;
    if cpu > 63 {
        return Err(format!("RT CPU `{text}` must be between 0 and 63"));
    }
    Ok(cpu)
}

fn parse_rt_metrics_path(text: &str) -> Result<PathBuf, String> {
    if text.is_empty() {
        return Err("`--rt-metrics` requires a non-empty path".into());
    }
    Ok(PathBuf::from(text))
}

fn execute_module(argc: i32, argv: *const *const c_char, module_path: *const c_char) -> i32 {
    let args = collect_args(argc, argv);
    let options = match parse_launcher_args(&args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("stc: {message}");
            return 2;
        }
    };
    let Some(module_argument) = options.module.or_else(|| {
        if module_path.is_null() {
            None
        } else {
            // SAFETY: the ABI contract requires a NUL-terminated string.
            Some(
                unsafe { CStr::from_ptr(module_path) }
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    }) else {
        eprintln!("stc: no module path was supplied");
        return 2;
    };
    let resolved = resolve_module_path(&module_argument);
    let platform = host_platform();
    let manifest = module_info::manifest_path(&resolved);
    let store = match &options.manifest_dir {
        Some(directory) => ManifestStore::open(directory.clone()),
        None => ManifestStore::memory(),
    };
    let mut host = Host::new(platform.clone(), store);
    host_api::clear_last_trap();
    let candidate = match Candidate::open(&platform, &resolved, &manifest) {
        Ok(candidate) => candidate,
        Err(error) => {
            eprintln!(
                "stc: cannot load module `{}`: {}: {}",
                resolved.display(),
                error.code,
                error.message
            );
            return 1;
        }
    };
    configure_run_services(&mut host, &options.run, &candidate);
    let control = match (&options.control_endpoint, &options.control_nonce) {
        (Some(endpoint), Some(nonce)) => match crate::control::ControlServer::bind(endpoint, nonce)
        {
            Ok(server) => Some(server),
            Err(error) => {
                eprintln!("stc: {}: {}", error.code, error.message);
                return 2;
            }
        },
        _ => None,
    };
    if let Err(error) = host.load(candidate, LoadMode::Cold) {
        eprintln!(
            "stc: cannot install module `{}`: {}: {}",
            resolved.display(),
            error.code,
            error.message
        );
        return if host_api::last_trap().is_some() {
            print_last_trap();
            STC_RUN_TRAP_EXIT_CODE
        } else {
            1
        };
    }
    if let Some(endpoint) = options.debug_endpoint.as_deref() {
        let mut config = DebugServerConfig::new(endpoint.to_path_buf(), options.debug_protocol);
        config.allow_write = options.debug_allow_write;
        config.allow_force = options.debug_allow_force;
        if let Some(ns) = options.debug_session_timeout_ns {
            config.session_timeout = std::time::Duration::from_nanos(ns);
        }
        match host.start_debug_server(&config) {
            Ok(info) => {
                // Only the endpoint file and audit paths are published; the
                // token stays in the endpoint file and is never printed.
                eprintln!("stc: debug endpoint {}", info.endpoint_file.display());
                eprintln!("stc: debug audit {}", info.audit_log.display());
            }
            Err(error) => {
                eprintln!("stc: {}: {}", error.code, error.message);
                if let Err(error) = host.shutdown() {
                    eprintln!("stc: shutdown failed: {}: {}", error.code, error.message);
                }
                return 2;
            }
        }
    }
    let outcome = if control.is_some() {
        run::run_loop_with_control(
            &mut host,
            &options.run,
            run::StopSource::None,
            control.as_ref(),
        )
    } else {
        run::run_loop(&mut host, &options.run)
    };
    match outcome {
        Ok(_) => {
            if options.run.status {
                print_host_status(&host);
            }
            if let Err(error) = host.shutdown() {
                eprintln!("stc: shutdown failed: {}: {}", error.code, error.message);
                return 1;
            }
            0
        }
        Err(error) => {
            if options.run.status {
                print_host_status(&host);
            }
            report_run_error(&error);
            let code = error.exit_code();
            if let Err(error) = host.shutdown() {
                eprintln!("stc: shutdown failed: {}: {}", error.code, error.message);
            }
            code
        }
    }
}

/// Configures retained state and the I/O backend for a launcher run.
///
/// CLI options win over metadata; relative paths resolve against the module
/// directory (D-163). Metadata's `[retain]` policy is the effective fallback.
fn configure_run_services(host: &mut Host, run: &RunOptions, candidate: &Candidate) {
    let info = candidate.info();
    let module_dir = candidate.module_dir();
    let mut state_file = run
        .state_file
        .as_ref()
        .map(|path| resolve_relative(module_dir, path));
    if state_file.is_none() {
        state_file = info
            .and_then(|info| info.retain.file.as_deref())
            .map(|file| module_dir.join(file));
    }
    let enabled =
        info.is_some_and(|info| info.retain.enabled) && !run.no_retain && state_file.is_some();
    let policy = info.map_or(MismatchPolicy::Error, |info| info.retain.policy);
    host.configure_retain(RetainConfig {
        enabled,
        file: state_file.filter(|_| enabled),
        policy,
        reset: run.reset_retain,
    });
    let io_file = run
        .io_file
        .as_ref()
        .map(|path| resolve_relative(module_dir, path));
    host.configure_io(run.io, io_file);
}

fn resolve_relative(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

/// Applies `stc_host_run` options to an embedding host.
///
/// An absent `state_file` leaves a configuration installed through
/// [`stc_host_configure_retain`] alone; `no_retain` disables it.
fn configure_embedding_services(host: &mut Host, run: &RunOptions) {
    if run.no_retain || run.state_file.is_some() {
        let file = run.state_file.as_deref().map(absolute_path);
        host.configure_retain(RetainConfig {
            enabled: !run.no_retain && file.is_some(),
            file,
            policy: MismatchPolicy::Error,
            reset: run.reset_retain,
        });
    }
    host.configure_io(run.io, run.io_file.as_deref().map(absolute_path));
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

fn report_run_error(error: &RunLoopError) {
    if let RunLoopError::Setup { message } = error {
        eprintln!("stc: {message}");
    } else if let RunLoopError::Cycle {
        error: host_error,
        trap,
        ..
    } = error
    {
        if *trap {
            print_last_trap();
        } else {
            eprintln!(
                "stc: cycle failed: {}: {}",
                host_error.code, host_error.message
            );
        }
    }
}

/// Prints the host status surface used by `stc run -v` and the control channel.
fn print_host_status(host: &Host) {
    let status = host.status();
    match status.active_build_id {
        Some(build) => eprintln!("stc: status: active {}", build.to_hex()),
        None => eprintln!("stc: status: active none"),
    }
    if let Some(build) = status.pending_build_id {
        eprintln!("stc: status: pending {}", build.to_hex());
    }
    if !status.retired_build_ids.is_empty() {
        let retired = status
            .retired_build_ids
            .iter()
            .map(|build| build.to_hex())
            .collect::<Vec<_>>()
            .join(",");
        eprintln!("stc: status: retired {retired}");
    }
    eprintln!("stc: status: cycles {}", status.cycles);
    let io = host.io_runtime();
    let io_file = io
        .file()
        .map_or_else(String::new, |file| format!(" file={}", file.display()));
    eprintln!("stc: status: io {}{io_file}", io.backend().name());
    for report in host.io_reports() {
        let details = report
            .diagnostics
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(" ");
        let fault = report
            .last_fault
            .as_deref()
            .map_or_else(String::new, |fault| format!(" last-fault={fault}"));
        eprintln!(
            "stc: status: io-backend {} devices={} state={} exchanges={} faults={} \
             hash={}{fault}{}{details}",
            report.name,
            report.devices,
            report.state.as_str(),
            report.exchanges,
            report.faults,
            report.config_hash,
            if details.is_empty() { "" } else { " " },
        );
    }
    let retain = host.retain_config();
    eprintln!(
        "stc: status: retain {} policy {}{}",
        if retain.enabled {
            "enabled"
        } else {
            "disabled"
        },
        retain.policy.as_str(),
        retain
            .file
            .as_deref()
            .map_or_else(String::new, |file| format!(" file={}", file.display()))
    );
    for task in &status.tasks {
        eprintln!(
            "stc: status: task {} due={} completed={} deadline={}",
            task.key,
            u8::from(task.due),
            u8::from(task.completed),
            task.next_deadline_ns
                .map_or_else(|| "none".into(), |value| value.to_string())
        );
    }
    if let Some(rejection) = &status.last_rejection {
        eprintln!("stc: status: last-rejection {rejection}");
    }
    for warning in &status.warnings {
        eprintln!("stc: status: warning[{}] {}", warning.code, warning.message);
    }
}

fn print_last_trap() {
    if let Some(trap) = host_api::last_trap() {
        eprintln!(
            "stc: runtime trap: {:?} (code {}) at span {}",
            trap_name(trap.code),
            trap.code,
            trap.span_id
        );
    }
}

pub(crate) fn resolve_module_path(argument: &str) -> PathBuf {
    let path = Path::new(argument);
    if path.is_absolute() || path.components().count() > 1 {
        return path.to_path_buf();
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|directory| directory.join(path)))
        .unwrap_or_else(|| path.to_path_buf())
}

/// Returns the host policy for this process.
#[must_use]
pub fn host_platform() -> HostPlatform {
    let triple = if cfg!(all(windows, target_env = "msvc")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    HostPlatform::new(
        PROFILE,
        stc_got::Target::new(triple, stc_got::Endianness::Little, 8),
        stc_got::HOST_ABI_MAJOR,
        stc_got::HOST_ABI_MINOR,
    )
}

/// Captures the process image for rollback of a failed cold install.
pub(crate) fn image_snapshot() -> (Vec<u8>, u64, u64, u64) {
    let state = state();
    (
        state.image.clone(),
        state.inputs,
        state.outputs,
        state.markers,
    )
}

/// Restores a process-image snapshot.
pub(crate) fn restore_image(snapshot: (Vec<u8>, u64, u64, u64)) {
    let mut state = state();
    state.image = snapshot.0;
    state.inputs = snapshot.1;
    state.outputs = snapshot.2;
    state.markers = snapshot.3;
}

/// Runs `f` with mutable access to the configured process image.
///
/// The host calls this on the activation path, after the old generation has
/// completed its cycle and before the new generation starts.
pub(crate) fn image_with<R>(f: impl FnOnce(&mut [u8]) -> R) -> R {
    let mut state = state();
    f(&mut state.image)
}

/// Returns the process-image base address without exposing the buffer.
///
/// Planned direct-cell values use this base before activation; the values are
/// rebound once the image is configured for the candidate layout.
pub(crate) fn image_base_address() -> usize {
    let mut state = state();
    state.image.as_mut_ptr() as usize
}

/// Returns the byte address of a process-image range.
///
/// Areas are `1` inputs, `2` outputs, and `3` markers.
pub(crate) fn image_area_address(area: u8, byte_offset: u64, bytes: u64) -> Option<usize> {
    let mut state = state();
    let (offset, length) = match area {
        1 => (0, state.inputs),
        2 => (state.inputs, state.outputs),
        3 => (state.inputs.saturating_add(state.outputs), state.markers),
        _ => return None,
    };
    if bytes == 0 || byte_offset.saturating_add(bytes) > length {
        return None;
    }
    let offset = usize::try_from(offset).ok()?;
    let byte_offset = usize::try_from(byte_offset).ok()?;
    let total = offset.checked_add(byte_offset)?;
    // SAFETY: the range was checked against the area length and the process
    // image is not resized while a module generation uses it.
    Some(unsafe { state.image.as_mut_ptr().add(total) } as usize)
}

/// Latches the cycle clock for the next scan.
///
/// A pending `set_cycle_time_ns` override wins over the live clock; the
/// override is consumed by this call. Returns the value the module sees.
pub(crate) fn latch_cycle_clock() -> i64 {
    let mut state = state();
    let now = state
        .pending_cycle_ns
        .take()
        .unwrap_or_else(monotonic_now_ns);
    state.cycle_now_ns = now;
    now
}

/// Takes the next cycle's pending clock override, when one was set.
pub(crate) fn take_pending_cycle_ns() -> Option<i64> {
    state().pending_cycle_ns.take()
}

/// Sets the latched cycle time without consuming a pending override.
pub(crate) fn set_cycle_now(ns: i64) {
    state().cycle_now_ns = ns;
}

pub(crate) fn collect_args(argc: i32, argv: *const *const c_char) -> Vec<String> {
    if argv.is_null() || argc <= 0 {
        return Vec::new();
    }
    let mut args = Vec::new();
    for index in 0..argc as usize {
        // SAFETY: the C caller guarantees `argv[0..argc]` are valid,
        // NUL-terminated strings for the duration of the call.
        let pointer = unsafe { *argv.add(index) };
        if pointer.is_null() {
            break;
        }
        let text = unsafe { CStr::from_ptr(pointer) };
        args.push(text.to_string_lossy().into_owned());
    }
    args
}

/// Returns the cycle time latched at the start of the current cycle.
#[unsafe(no_mangle)]
pub extern "C" fn stc_rt_cycle_now_ns() -> i64 {
    state().cycle_now_ns
}

/// Returns the live monotonic clock in nanoseconds.
#[unsafe(no_mangle)]
pub extern "C" fn stc_rt_monotonic_now_ns() -> u64 {
    monotonic_now_ns() as u64
}

fn monotonic_now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX)
        })
}

/// Overrides the clock value used by the next cycle.
#[unsafe(no_mangle)]
pub extern "C" fn stc_rt_set_cycle_time_ns(ns: i64) {
    state().pending_cycle_ns = Some(ns);
}

/// Returns the local date/time in nanoseconds (M4: same as the latched cycle).
#[unsafe(no_mangle)]
pub extern "C" fn stc_rt_local_date_ns() -> i64 {
    state().cycle_now_ns
}

/// Allocates the process image with the given area sizes (D-084).
#[unsafe(no_mangle)]
pub extern "C" fn stc_rt_io_configure(input_len: u64, output_len: u64, marker_len: u64) {
    let total = input_len
        .checked_add(output_len)
        .and_then(|value| value.checked_add(marker_len))
        .unwrap_or(0);
    let total = usize::try_from(total).unwrap_or(0);
    let mut state = state();
    state.inputs = input_len;
    state.outputs = output_len;
    state.markers = marker_len;
    // Reuse the allocation when it already has capacity. Direct cells point
    // into this buffer, so moving it invalidates the active generation's
    // address bindings until they are recomputed.
    if total <= state.image.capacity() {
        state.image.clear();
        state.image.resize(total, 0);
    } else {
        state.image = vec![0; total];
    }
}

/// Returns the process-image base pointer and total length.
///
/// # Safety
///
/// `base` and `len` must be valid, writable pointers to a `uint8_t*` and a
/// `uint64_t` respectively.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_rt_io_image(base: *mut *mut u8, len: *mut u64) {
    let mut state = state();
    if !base.is_null() {
        // SAFETY: the caller contract above guarantees writable pointers.
        unsafe { *base = state.image.as_mut_ptr() };
    }
    if !len.is_null() {
        let total = state.image.len() as u64;
        // SAFETY: the caller contract above guarantees writable pointers.
        unsafe { *len = total };
    }
}

/// Returns the configured process-image length in bytes.
///
/// This is the safe Rust accessor used by embedding hosts and tests; generated
/// code reaches the image through the host I/O service.
#[must_use]
pub fn image_length() -> u64 {
    state().image.len() as u64
}

/// Returns the process-image base address, or `0` when no image is configured.
///
/// The address is only meaningful while the image layout is unchanged; a
/// reconfiguration may move the buffer.
#[must_use]
pub fn image_base() -> usize {
    let mut state = state();
    if state.image.is_empty() {
        0
    } else {
        state.image.as_mut_ptr() as usize
    }
}

/// Copies up to `buffer.len()` process-image bytes starting at `offset`.
///
/// Returns the number of bytes copied. A range outside the image copies
/// nothing.
pub fn image_read(offset: usize, buffer: &mut [u8]) -> usize {
    let state = state();
    let Some(slice) = state.image.get(offset..) else {
        return 0;
    };
    let count = slice.len().min(buffer.len());
    buffer[..count].copy_from_slice(&slice[..count]);
    count
}

/// Writes `bytes` into the process image at `offset`.
///
/// Returns `false` when the range lies outside the configured image.
pub fn image_write(offset: usize, bytes: &[u8]) -> bool {
    let mut state = state();
    let Some(slice) = state.image.get_mut(offset..) else {
        return false;
    };
    if bytes.len() > slice.len() {
        return false;
    }
    slice[..bytes.len()].copy_from_slice(bytes);
    true
}

/// Retained-state load using the configured state file (State §5).
///
/// Returns the ABI 2 `state_load` codes: 0 OK, 1 not found, 2 mismatch,
/// 3 corrupt, 4 I/O error, 5 retention disabled.
///
/// # Safety
///
/// `buffer` must be null for `len == 0`, or point to `len` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_rt_state_load(buffer: *mut u8, len: i64, layout_hash: i64) -> i32 {
    // SAFETY: the caller follows the ABI buffer contract.
    unsafe { state::service_load(buffer, len, layout_hash) }
}

/// Retained-state save using the configured state file (State §5).
///
/// # Safety
///
/// `buffer` must be null for `len == 0`, or point to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_rt_state_save(buffer: *const u8, len: i64, layout_hash: i64) -> i32 {
    // SAFETY: the caller follows the ABI buffer contract.
    unsafe { state::service_save(buffer, len, layout_hash) }
}

/// Configures the legacy retained-state helpers.
///
/// `path` is an absolute file path (null disables the helpers), `policy` is
/// 0 for `error` and 1 for `reset`, and `enabled` is nonzero to enable load
/// and save. The ABI 2 host services are configured through [`Host`] instead.
///
/// # Safety
///
/// `path` must be null or point to a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_rt_state_configure(path: *const c_char, policy: i32, enabled: i32) {
    let file = if path.is_null() {
        None
    } else {
        // SAFETY: the caller guarantees a NUL-terminated string.
        Some(PathBuf::from(
            unsafe { CStr::from_ptr(path) }
                .to_string_lossy()
                .into_owned(),
        ))
    };
    state::configure(RetainConfig {
        enabled: enabled != 0 && file.is_some(),
        file,
        policy: if policy == 1 {
            MismatchPolicy::Reset
        } else {
            MismatchPolicy::Error
        },
        reset: false,
    });
    // The legacy helpers are called outside a host generation.
    state::allow_save(Some(0));
}

/// Load mode values for [`stc_host_load_module`].
pub mod load_mode {
    /// Stop and reinitialize the program for this module.
    pub const COLD: i32 = 0;
    /// Prepare an online change; the old generation stays active until the
    /// host activates the candidate.
    pub const ONLINE: i32 = 1;
    /// Install cold when the host is empty, otherwise prepare online.
    pub const AUTO: i32 = 2;
}

struct Embedded {
    host: Option<Host>,
    metadata: Option<CString>,
}

static EMBEDDED: Mutex<Embedded> = Mutex::new(Embedded {
    host: None,
    metadata: None,
});

fn embedded() -> std::sync::MutexGuard<'static, Embedded> {
    EMBEDDED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Loads a module and its adjacent manifest into the embedding host.
///
/// Returns `0` when the candidate is installed or prepared, `1` for an invalid
/// argument or load failure, and `2` when an online request requires a cold
/// download.
///
/// # Safety
///
/// `module_path` must be null or point to a NUL-terminated string that stays
/// valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_host_load_module(module_path: *const c_char, mode: i32) -> i32 {
    if module_path.is_null() {
        return 1;
    }
    // SAFETY: the ABI contract requires a NUL-terminated path.
    let argument = unsafe { CStr::from_ptr(module_path) }
        .to_string_lossy()
        .into_owned();
    let resolved = resolve_module_path(&argument);
    let platform = host_platform();
    let manifest = module_info::manifest_path(&resolved);
    let candidate = match Candidate::open(&platform, &resolved, &manifest) {
        Ok(candidate) => candidate,
        Err(error) => {
            eprintln!(
                "stc: cannot load module `{}`: {}: {}",
                resolved.display(),
                error.code,
                error.message
            );
            return 1;
        }
    };
    let metadata_text = candidate.info().map(|info| info.text.clone());
    let mode = match mode {
        load_mode::COLD => LoadMode::Cold,
        load_mode::ONLINE => LoadMode::Online,
        load_mode::AUTO => LoadMode::Auto,
        _ => return 1,
    };
    let mut state = embedded();
    if state.host.is_none() {
        state.host = Some(Host::new(platform, ManifestStore::memory()));
    }
    let host = state.host.as_mut().expect("host created above");
    host_api::clear_last_trap();
    match host.load(candidate, mode) {
        Ok(LoadOutcome::ColdRequired { .. }) => 2,
        Ok(_) => {
            state.metadata = metadata_text.and_then(|text| CString::new(text).ok());
            0
        }
        Err(error) => {
            eprintln!(
                "stc: cannot install module: {}: {}",
                error.code, error.message
            );
            1
        }
    }
}

/// Selects the embedding host's process-image backend.
///
/// `backend` is `-1` (keep the module metadata's backend), `0` `null`, `1`
/// `memory`, `2` `file`, or `3` `ethercat`. `file` is null to keep the
/// metadata/default file.
/// Returns `0` on success and `2` for an unknown backend. The setting applies
/// to subsequent loads and the current host's next cycles.
///
/// # Safety
///
/// `file` must be null or point to a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_host_configure_io(backend: i32, file: *const c_char) -> i32 {
    let backend = match backend {
        -1 => None,
        0 => Some(IoBackend::Null),
        1 => Some(IoBackend::Memory),
        2 => Some(IoBackend::File),
        3 => Some(IoBackend::Ethercat),
        _ => return 2,
    };
    // SAFETY: the caller contract requires a NUL-terminated string.
    let file = unsafe { c_text(file) }.map(|text| absolute_path(Path::new(&text)));
    let mut state = embedded();
    let host = state
        .host
        .get_or_insert_with(|| Host::new(host_platform(), ManifestStore::memory()));
    host.configure_io(backend, file);
    0
}

/// Configures the embedding host's retained state.
///
/// `path` is an absolute file path; null disables retention. `policy` is `0`
/// for `error` and `1` for `reset`; `reset` is nonzero for a cold start.
/// Returns `0` on success and `2` for an invalid policy.
///
/// # Safety
///
/// `path` must be null or point to a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_host_configure_retain(
    path: *const c_char,
    policy: i32,
    reset: i32,
) -> i32 {
    let policy = match policy {
        0 => MismatchPolicy::Error,
        1 => MismatchPolicy::Reset,
        _ => return 2,
    };
    // SAFETY: the caller contract requires a NUL-terminated string.
    let file = unsafe { c_text(path) }.map(|text| absolute_path(Path::new(&text)));
    let mut state = embedded();
    let host = state
        .host
        .get_or_insert_with(|| Host::new(host_platform(), ManifestStore::memory()));
    host.configure_retain(RetainConfig {
        enabled: file.is_some(),
        file,
        policy,
        reset: reset != 0,
    });
    0
}

/// Runs one cycle of the active embedding-host generation.
///
/// Returns `0` on success, `1` on a cycle failure or trap, and `2` when no
/// module is loaded.
#[unsafe(no_mangle)]
pub extern "C" fn stc_host_cycle() -> i32 {
    let mut state = embedded();
    let Some(host) = state.host.as_mut() else {
        return 2;
    };
    match host.cycle() {
        Ok(_) => 0,
        Err(error) => {
            eprintln!("stc: cycle failed: {}: {}", error.code, error.message);
            1
        }
    }
}

/// Shuts down and unloads the embedding-host module.
#[unsafe(no_mangle)]
pub extern "C" fn stc_host_shutdown() -> i32 {
    let mut state = embedded();
    state.metadata = None;
    let Some(mut host) = state.host.take() else {
        return 0;
    };
    match host.shutdown() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("stc: shutdown failed: {}: {}", error.code, error.message);
            1
        }
    }
}

/// C-facing run options for [`stc_host_run`] (Runtime §1.2).
///
/// `struct_size` should be `sizeof(stc_host_options)`; zero accepts the current
/// layout. A negative `cycle_time_ns` or `mock_clock_ns` selects the live clock
/// and no mock clock. A non-empty `control_endpoint` requires the matching
/// `control_nonce` and serves the local deploy/status channel.
#[repr(C)]
pub struct StcHostOptions {
    /// `sizeof(stc_host_options)`, or zero.
    pub struct_size: u32,
    /// Host ABI major version (2), or zero.
    pub abi_version: u32,
    /// Cycle count; zero runs until interrupted or `stop`.
    pub cycles: u64,
    /// Fixed cycle time in nanoseconds; negative selects none.
    pub cycle_time_ns: i64,
    /// I/O backend: -1 uses module metadata, 0 `null`, 1 `memory`, 2 `file`.
    pub io_backend: i32,
    /// Cold start: ignore existing retained state.
    pub reset_retain: i32,
    /// Disable retained-state load and save.
    pub no_retain: i32,
    /// Print the status summary at exit.
    pub status: i32,
    /// Initial mock-clock time in nanoseconds; negative selects the live clock.
    pub mock_clock_ns: i64,
    /// Module path, or null to run the already-loaded embedding host.
    pub module_path: *const c_char,
    /// File backend image path (M6 phase 5).
    pub io_file: *const c_char,
    /// Retained-state file (M6 phase 5).
    pub state_file: *const c_char,
    /// Control endpoint (named pipe or Unix socket), or null/empty.
    pub control_endpoint: *const c_char,
    /// Optional external stop flag; nonzero stops before the next scan.
    pub stop: *const c_int,
    /// Control-channel nonce required by a non-empty endpoint.
    pub control_nonce: *const c_char,
    /// Local debug endpoint file, or null. Appended in M6.5 (ABI 2.3).
    pub debug_endpoint: *const c_char,
    /// Grant the `write` scope to debug sessions.
    pub debug_allow_write: i32,
    /// Grant the `force` scope to debug sessions.
    pub debug_allow_force: i32,
    /// Idle session bound in nanoseconds; zero selects five minutes.
    pub debug_session_timeout_ns: i64,
}

impl Default for StcHostOptions {
    fn default() -> Self {
        Self {
            struct_size: u32::try_from(std::mem::size_of::<Self>()).unwrap_or(u32::MAX),
            abi_version: u32::try_from(STC_RUNTIME_ABI).unwrap_or(2),
            cycles: 0,
            cycle_time_ns: -1,
            io_backend: -1,
            reset_retain: 0,
            no_retain: 0,
            status: 0,
            mock_clock_ns: -1,
            module_path: std::ptr::null(),
            io_file: std::ptr::null(),
            state_file: std::ptr::null(),
            control_endpoint: std::ptr::null(),
            stop: std::ptr::null(),
            control_nonce: std::ptr::null(),
            debug_endpoint: std::ptr::null(),
            debug_allow_write: 0,
            debug_allow_force: 0,
            debug_session_timeout_ns: 0,
        }
    }
}

/// Runs the embedding host loop with an options struct.
///
/// Loads `module_path` cold when it is non-null; otherwise runs the module
/// already loaded through [`stc_host_load_module`]. The module stays loaded
/// afterwards. Returns the run exit code (0, 1, 2, 3, 4, or 130).
///
/// # Safety
///
/// `options` must be null or point to a readable [`StcHostOptions`] whose
/// pointer fields are null or NUL-terminated strings. The `stop` flag must stay
/// readable for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_host_run(options: *const StcHostOptions) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: the caller contract is stated above.
        unsafe { run_embedded(options) }
    }));
    match result {
        Ok(code) => code,
        Err(_) => {
            eprintln!("stc: internal runtime panic");
            3
        }
    }
}

/// # Safety
///
/// `options` must satisfy the [`stc_host_run`] contract.
unsafe fn run_embedded(options: *const StcHostOptions) -> i32 {
    if options.is_null() {
        eprintln!("stc: `stc_host_run` requires an options struct");
        return 2;
    }
    // SAFETY: the caller guarantees a readable options struct.
    let options = unsafe { &*options };
    let expected_size = u32::try_from(std::mem::size_of::<StcHostOptions>()).unwrap_or(u32::MAX);
    let legacy_size =
        u32::try_from(std::mem::offset_of!(StcHostOptions, debug_endpoint)).unwrap_or(u32::MAX);
    let current_layout = options.struct_size == 0 || options.struct_size == expected_size;
    if options.struct_size != 0
        && options.struct_size != expected_size
        && options.struct_size != legacy_size
    {
        eprintln!(
            "stc: `stc_host_run` options size {} does not match {expected_size}",
            options.struct_size
        );
        return 2;
    }
    if options.abi_version != 0 && options.abi_version != STC_RUNTIME_ABI as u32 {
        eprintln!(
            "stc: `stc_host_run` requires host ABI {STC_RUNTIME_ABI}, found {}",
            options.abi_version
        );
        return 2;
    }
    let io = match options.io_backend {
        -1 => None,
        0 => Some(IoBackend::Null),
        1 => Some(IoBackend::Memory),
        2 => Some(IoBackend::File),
        3 => Some(IoBackend::Ethercat),
        other => {
            eprintln!("stc: unknown I/O backend {other}");
            return 2;
        }
    };
    // SAFETY: the caller contract requires readable NUL-terminated strings.
    let control_endpoint =
        unsafe { c_text(options.control_endpoint) }.filter(|text| !text.is_empty());
    let control_nonce = unsafe { c_text(options.control_nonce) }.filter(|text| !text.is_empty());
    if control_endpoint.is_some() && control_nonce.is_none() {
        eprintln!("stc: a control endpoint requires a nonce");
        return 2;
    }
    let debug_endpoint = if current_layout {
        unsafe { c_text(options.debug_endpoint) }.filter(|text| !text.is_empty())
    } else {
        None
    };
    let debug_allow_write = current_layout && options.debug_allow_write != 0;
    let debug_allow_force = current_layout && options.debug_allow_force != 0;
    if current_layout && options.debug_session_timeout_ns < 0 {
        eprintln!("stc: a negative debug session timeout is invalid");
        return 2;
    }
    let debug_session_timeout_ns = if current_layout {
        options.debug_session_timeout_ns
    } else {
        0
    };
    let run = RunOptions {
        cycles: options.cycles,
        cycle_time_ns: (options.cycle_time_ns >= 0).then_some(options.cycle_time_ns),
        io,
        io_file: unsafe { c_text(options.io_file) }.map(PathBuf::from),
        state_file: unsafe { c_text(options.state_file) }.map(PathBuf::from),
        reset_retain: options.reset_retain != 0,
        no_retain: options.no_retain != 0,
        mock_clock_ns: (options.mock_clock_ns >= 0).then_some(options.mock_clock_ns),
        status: options.status != 0,
        ..RunOptions::default()
    };
    let module_path = unsafe { c_text(options.module_path) };
    let loads_module = module_path.is_some();
    let mut state = embedded();
    if let Some(module_path) = module_path {
        let platform = host_platform();
        if state.host.is_none() {
            state.host = Some(Host::new(platform.clone(), ManifestStore::memory()));
        }
        configure_embedding_services(state.host.as_mut().expect("host created above"), &run);
        let resolved = resolve_module_path(&module_path);
        let manifest = module_info::manifest_path(&resolved);
        let candidate = match Candidate::open(&platform, &resolved, &manifest) {
            Ok(candidate) => candidate,
            Err(error) => {
                eprintln!(
                    "stc: cannot load module `{}`: {}: {}",
                    resolved.display(),
                    error.code,
                    error.message
                );
                return 1;
            }
        };
        let metadata = candidate.info().map(|info| info.text.clone());
        host_api::clear_last_trap();
        let loaded = state
            .host
            .as_mut()
            .expect("host created above")
            .load(candidate, LoadMode::Cold);
        if let Err(error) = loaded {
            eprintln!(
                "stc: cannot install module `{}`: {}: {}",
                resolved.display(),
                error.code,
                error.message
            );
            return if host_api::last_trap().is_some() {
                print_last_trap();
                STC_RUN_TRAP_EXIT_CODE
            } else {
                1
            };
        }
        state.metadata = metadata.and_then(|text| CString::new(text).ok());
    }
    let Some(host) = state.host.as_mut() else {
        eprintln!("stc: `stc_host_run` has no loaded module");
        return 2;
    };
    if !loads_module {
        configure_embedding_services(host, &run);
    }
    if let Some(endpoint) = debug_endpoint {
        // A previous run of the same embedding host may still own a server;
        // its endpoint file describes the previous generation and must not be
        // reused. Stopping it ends any session without touching the module.
        if host.debug_server_info().is_some() {
            host.stop_debug_server();
        }
        let mut config = DebugServerConfig::new(
            PathBuf::from(endpoint),
            crate::transport::TransportKind::Pipe,
        );
        config.allow_write = debug_allow_write;
        config.allow_force = debug_allow_force;
        if debug_session_timeout_ns > 0 {
            config.session_timeout =
                std::time::Duration::from_nanos(debug_session_timeout_ns.unsigned_abs());
        }
        if let Err(error) = host.start_debug_server(&config) {
            eprintln!("stc: {}: {}", error.code, error.message);
            return 2;
        }
    }
    let control = match (&control_endpoint, &control_nonce) {
        (Some(endpoint), Some(nonce)) => match crate::control::ControlServer::bind(endpoint, nonce)
        {
            Ok(server) => Some(server),
            Err(error) => {
                eprintln!("stc: {}: {}", error.code, error.message);
                return 2;
            }
        },
        _ => None,
    };
    let outcome = run::run_loop_with_control(
        host,
        &run,
        run::StopSource::External(options.stop),
        control.as_ref(),
    );
    if run.status {
        print_host_status(host);
    }
    match outcome {
        Ok(_) => 0,
        Err(error) => {
            report_run_error(&error);
            error.exit_code()
        }
    }
}

/// Reads an optional NUL-terminated string argument.
///
/// # Safety
///
/// `pointer` must be null or point to a NUL-terminated string.
unsafe fn c_text(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees a NUL-terminated string.
    Some(
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// Resolves an exported symbol in the active embedding-host module.
///
/// Returns a null pointer when the host has no active module or the module does
/// not export the name. The address stays valid until the generation retires.
///
/// # Safety
///
/// `name` must be null or point to a NUL-terminated string that stays valid
/// for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_host_symbol(name: *const c_char) -> *mut c_void {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: the ABI contract requires a NUL-terminated symbol name.
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    let state = embedded();
    state
        .host
        .as_ref()
        .and_then(|host| host.export_symbol(&name))
        .map_or(std::ptr::null_mut(), |address| address.cast_mut())
}

/// Copies the committed manifest JSON into a caller buffer.
///
/// With a null buffer, writes the required length (including the terminating
/// NUL) and returns `0`. With a buffer shorter than required, writes the
/// required length and returns `1`.
///
/// # Safety
///
/// `len` must point to a writable `i64`. When `buffer` is non-null it must
/// point to at least the capacity reported in `*len` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_host_export_manifest(buffer: *mut c_char, len: *mut i64) -> i32 {
    if len.is_null() {
        return 1;
    }
    let state = embedded();
    let Some(host) = state.host.as_ref() else {
        return 1;
    };
    let Some(json) = host.export_manifest_json() else {
        return 1;
    };
    let required = i64::try_from(json.len())
        .unwrap_or(i64::MAX)
        .saturating_add(1);
    // SAFETY: the caller supplies a readable capacity through `*len`.
    let capacity = if buffer.is_null() { 0 } else { unsafe { *len } };
    // SAFETY: the caller supplies a writable `i64` pointer.
    unsafe { *len = required };
    if buffer.is_null() {
        return 0;
    }
    if capacity < required {
        return 1;
    }
    let required_usize = usize::try_from(required).unwrap_or(usize::MAX);
    // SAFETY: the caller's buffer has at least `required` bytes; the source
    // has `required - 1` bytes plus the terminating NUL written below.
    unsafe {
        std::ptr::copy_nonoverlapping(json.as_ptr().cast::<c_char>(), buffer, required_usize - 1);
        *buffer.add(required_usize - 1) = 0;
    }
    0
}

/// Returns the loaded module's metadata JSON, or null before a load.
#[unsafe(no_mangle)]
pub extern "C" fn stc_metadata_json() -> *const c_char {
    let state = embedded();
    state
        .metadata
        .as_ref()
        .map_or(std::ptr::null(), |text| text.as_ptr())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_is_contiguous_and_exposed() {
        stc_rt_io_configure(4, 2, 3);
        let mut base: *mut u8 = std::ptr::null_mut();
        let mut len: u64 = 0;
        // SAFETY: the test passes valid local pointers.
        unsafe { stc_rt_io_image(&mut base, &mut len) };
        assert_eq!(len, 9);
        assert!(!base.is_null());
        // SAFETY: the image is 9 bytes and owned by the runtime.
        unsafe { *base.add(8) = 7 };
        let state = state();
        assert_eq!(state.image[8], 7);
        assert_eq!(state.inputs, 4);
        assert_eq!(state.outputs, 2);
        assert_eq!(state.markers, 3);
    }

    #[test]
    fn cycle_time_is_latched() {
        stc_rt_set_cycle_time_ns(1234);
        latch_cycle_clock();
        assert_eq!(stc_rt_cycle_now_ns(), 1234);
        stc_rt_set_cycle_time_ns(4321);
        assert_eq!(stc_rt_cycle_now_ns(), 1234);
        latch_cycle_clock();
        assert_eq!(stc_rt_cycle_now_ns(), 4321);
    }

    #[test]
    fn legacy_state_helpers_use_the_configured_file() {
        let _lock = test_support::state_guard();
        let directory = std::env::temp_dir().join(format!(
            "stc-legacy-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        ));
        std::fs::create_dir_all(&directory).expect("temp directory");
        let path = directory.join("legacy.stcstate");
        let path_text = CString::new(path.to_string_lossy().as_bytes()).expect("path");
        // SAFETY: the path is a live NUL-terminated string.
        unsafe { stc_rt_state_configure(path_text.as_ptr(), 0, 1) };

        let first = [1_u8, 2, 3, 4];
        // SAFETY: the buffer is four readable bytes.
        assert_eq!(unsafe { stc_rt_state_save(first.as_ptr(), 4, 9) }, 0);
        let mut second = [0_u8; 4];
        // SAFETY: the buffer is four writable bytes.
        assert_eq!(unsafe { stc_rt_state_load(second.as_mut_ptr(), 4, 9) }, 0);
        assert_eq!(second, first);

        // A mismatched layout hash is rejected under the error policy.
        assert_eq!(unsafe { stc_rt_state_load(second.as_mut_ptr(), 4, 10) }, 2);
        // SAFETY: null disables the helpers.
        unsafe { stc_rt_state_configure(std::ptr::null(), 0, 0) };
        assert_eq!(unsafe { stc_rt_state_load(second.as_mut_ptr(), 4, 9) }, 5);
        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    #[test]
    fn launcher_rt_options_validate_combinations() {
        let args = |items: &[&str]| {
            items
                .iter()
                .map(|item| (*item).to_owned())
                .collect::<Vec<_>>()
        };
        assert!(
            parse_launcher_args(&args(&["stc", "--rt", "--cycle-time", "1ms"])).is_ok(),
            "a positive cycle time is accepted"
        );
        assert!(
            parse_launcher_args(&args(&[
                "stc",
                "--rt",
                "--cycle-time",
                "1ms",
                "--rt-priority",
                "90",
                "--rt-cpu",
                "3",
                "--rt-metrics",
                "metrics.jsonl",
            ]))
            .is_ok(),
            "the full RT option set is accepted"
        );
        assert!(
            parse_launcher_args(&args(&["stc", "--rt"])).is_err(),
            "RT requires a cycle time"
        );
        assert!(
            parse_launcher_args(&args(&["stc", "--rt-priority", "90"])).is_err(),
            "RT sub-options require --rt"
        );
        assert!(
            parse_launcher_args(&args(&[
                "stc",
                "--rt",
                "--cycle-time",
                "1ms",
                "--mock-clock",
                "0ns",
            ]))
            .is_err(),
            "RT and the mock clock are mutually exclusive"
        );
        assert!(
            parse_launcher_args(&args(&[
                "stc",
                "--rt",
                "--cycle-time",
                "1ms",
                "--rt-priority",
                "0",
            ]))
            .is_err(),
            "RT priority zero is rejected"
        );
        assert!(
            parse_launcher_args(&args(&[
                "stc",
                "--rt",
                "--cycle-time",
                "1ms",
                "--rt-priority",
                "100",
            ]))
            .is_err(),
            "RT priority above 99 is rejected"
        );
        assert!(
            parse_launcher_args(&args(&[
                "stc",
                "--rt",
                "--cycle-time",
                "1ms",
                "--rt-cpu",
                "64",
            ]))
            .is_err(),
            "RT CPU 64 is outside the supported range"
        );
    }
}
