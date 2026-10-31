//! Linux EtherCAT backend loaded through the project-owned C shim.
//!
//! IO-Routing-and-Backends §2 and §4 fix the EtherCAT backend as a
//! dynamically loaded shim. The base runtime never links `libethercat`; the
//! shim in `tools/ethercat/` is built separately against IgH 1.6.13 and is
//! loaded with `dlopen` only when an `ethercat` or `ethercat-status` device is
//! configured.
//!
//! The shim owns no policy. It forwards the fixed IgH lifecycle and cycle
//! calls and converts IgH bitfield state structures to plain scalars. This
//! module owns configuration validation, domain mapping, startup timing,
//! fault/recovery policy, and the status block.
//!
//! The shim path follows decision E-8: `STC_ETHERCAT_LIBRARY` when set,
//! otherwise the bare `libstc_io_ethercat.so` loader search path. No
//! EtherCAT master is required to probe the ABI; only cold start opens one.
//!
//! Distributed-clock configuration is only applied when `dc.assign-activate`
//! is the numeric IgH AssignActivate word (E-17, a `u16`). The profile schema
//! accepts `0..=65535`; a boolean or an out-of-range value fails cold start
//! with `STC-E-IO-ETH-006` rather than silently changing the request.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use stc_got::error::{Error, Result};
use stc_got::json::{self, Json};

use crate::coordinator::{
    BackendChannel, BackendConfig, BackendDeviceConfig, BackendState, BackendStatus, DeviceBackend,
    ImageRange, ProcessImage,
};
use crate::io_mock;

/// Stable diagnostics emitted by the EtherCAT backend.
pub mod codes {
    /// The shim is missing, cannot be loaded, or fails the ABI handshake.
    pub const ETH_SHIM: &str = "STC-E-IO-ETH-001";
    /// Requesting the EtherCAT master failed.
    pub const ETH_MASTER: &str = "STC-E-IO-ETH-002";
    /// Slave configuration or slave identity validation failed.
    pub const ETH_SLAVE: &str = "STC-E-IO-ETH-003";
    /// PDO/sync/entry configuration or registration failed.
    pub const ETH_PDO: &str = "STC-E-IO-ETH-004";
    /// Activation, domain data, or release failed.
    pub const ETH_ACTIVATE: &str = "STC-E-IO-ETH-005";
    /// The device or channel configuration is invalid.
    pub const ETH_CONFIG: &str = "STC-E-IO-ETH-006";
    /// A bound channel does not resolve or disagrees with the image.
    pub const ETH_BINDING: &str = "STC-E-IO-ETH-007";
    /// The startup timeout expired before every slave reached OP.
    pub const ETH_STARTUP: &str = "STC-E-IO-ETH-008";
    /// A runtime fieldbus fault was observed; inputs are stale.
    pub const W_ETHERCAT_FAULT: &str = "STC-W-IO-ETH-003";
}

/// Environment variable naming the shim shared object (decision E-8).
pub const SHIM_ENV: &str = "STC_ETHERCAT_LIBRARY";

/// Bare loader name used when [`SHIM_ENV`] is not set.
pub const SHIM_LIBRARY: &str = "libstc_io_ethercat.so";

/// Versioned shim ABI implemented by `tools/ethercat/stc_ec_shim.h`.
const SHIM_ABI_VERSION: u32 = 1;
/// Exact shim ABI name expected by this loader.
const SHIM_ABI_NAME: &str = "stc-io-ethercat/1";

const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_RECOVERY_CYCLES: u32 = 3;

const DIR_INPUT: u8 = 0;
const DIR_OUTPUT: u8 = 1;

const WC_COMPLETE: u8 = 2;

const MASTER_PHASE_OPERATION: u64 = 2;

const ERROR_BUFFER: usize = 256;

const RTLD_NOW: c_int = 2;
const RTLD_LOCAL: c_int = 0;

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *const c_char;
}

/// The shim's version handshake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShimInfo {
    /// Shim ABI version (`1`).
    pub abi_version: u32,
    /// Shim ABI name (`stc-io-ethercat/1`).
    pub name: String,
    /// Human-readable shim build string.
    pub shim_version: String,
}

/// Loads a shim and reports its ABI without opening a master.
///
/// # Errors
///
/// Returns `STC-E-IO-ETH-001` when the library is missing, cannot be loaded,
/// lacks a required symbol, or fails the version/name handshake.
pub fn probe_shim(path: &Path) -> Result<ShimInfo> {
    let shim = LoadedShim::load(path).map_err(|error| {
        Error::new(
            codes::ETH_SHIM,
            format!(
                "cannot load EtherCAT shim `{}`: {}",
                path.display(),
                error.message
            ),
        )
    })?;
    Ok(shim.info())
}

fn shim_path() -> PathBuf {
    match std::env::var_os(SHIM_ENV) {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from(SHIM_LIBRARY),
    }
}

fn load_default_shim() -> Result<Box<dyn EcShim>> {
    let path = shim_path();
    match LoadedShim::load(&path) {
        Ok(shim) => Ok(Box::new(shim)),
        Err(error) => Err(Error::new(
            codes::ETH_SHIM,
            format!(
                "cannot load EtherCAT shim `{}` (set {SHIM_ENV}): {}",
                path.display(),
                error.message
            ),
        )),
    }
}

#[derive(Clone, Debug)]
struct ShimCallError {
    code: i32,
    message: String,
}

type ShimResult<T> = std::result::Result<T, ShimCallError>;

fn call_error(code: i32, buffer: &[u8], operation: &str) -> ShimCallError {
    let text = CStr::from_bytes_until_nul(buffer)
        .ok()
        .map(|text| text.to_string_lossy().into_owned())
        .filter(|text| !text.is_empty());
    let message = text.map_or_else(
        || format!("shim {operation} failed (code {code})"),
        |text| format!("shim {operation} failed: {text} (code {code})"),
    );
    ShimCallError { code, message }
}

fn plain_call_error(code: i32, operation: &str) -> ShimCallError {
    ShimCallError {
        code,
        message: format!("shim {operation} failed (code {code})"),
    }
}

fn loader_error(message: impl Into<String>) -> ShimCallError {
    ShimCallError {
        code: 0,
        message: message.into(),
    }
}

fn verify_handshake(version: u32, name: &str) -> ShimResult<()> {
    if version != SHIM_ABI_VERSION {
        return Err(loader_error(format!(
            "unsupported shim ABI version {version}; expected {SHIM_ABI_VERSION}"
        )));
    }
    if name != SHIM_ABI_NAME {
        return Err(loader_error(format!(
            "shim ABI name `{name}` does not match `{SHIM_ABI_NAME}`"
        )));
    }
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CShimSlaveDesc {
    alias: u16,
    position: u16,
    vendor_id: u32,
    product_code: u32,
    revision: u32,
    check_revision: u8,
    reserved: [u8; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CShimSyncDesc {
    index: u8,
    direction: u8,
    watchdog: u8,
    reserved: u8,
    pdo_offset: u32,
    pdo_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CShimPdoDesc {
    index: u16,
    reserved: u16,
    entry_offset: u32,
    entry_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CShimEntryDesc {
    index: u16,
    subindex: u8,
    bits: u8,
    direction: u8,
    reserved: [u8; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CShimMasterStatus {
    slaves_responding: u32,
    al_states: u32,
    link_up: u8,
    reserved: [u8; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CShimDomainStatus {
    working_counter: u16,
    wc_state: u8,
    redundancy: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CShimSlaveStatus {
    al_state: u16,
    online: u8,
    operational: u8,
}

type MasterOpenFn = unsafe extern "C" fn(u16, *mut *mut c_void, *mut c_char, usize) -> c_int;
type MasterCloseFn = unsafe extern "C" fn(*mut c_void) -> c_int;
type MasterOpFn = unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> c_int;
type SlaveConfigureFn =
    unsafe extern "C" fn(*mut c_void, u32, *const CShimSlaveDesc, *mut c_char, usize) -> c_int;
type SlaveConfigurePdosFn = unsafe extern "C" fn(
    *mut c_void,
    u32,
    *const CShimSyncDesc,
    usize,
    *const CShimPdoDesc,
    usize,
    *const CShimEntryDesc,
    usize,
    *mut c_char,
    usize,
) -> c_int;
type SlaveConfigureDcFn =
    unsafe extern "C" fn(*mut c_void, u32, u16, u32, i32, u32, i32, *mut c_char, usize) -> c_int;
type EntryRegisterFn = unsafe extern "C" fn(
    *mut c_void,
    u32,
    u16,
    u8,
    *mut i32,
    *mut u32,
    *mut c_char,
    usize,
) -> c_int;
type ActivateFn = unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> c_int;
type DomainDataFn =
    unsafe extern "C" fn(*mut c_void, *mut *mut u8, *mut usize, *mut c_char, usize) -> c_int;
type MasterStatusFn = unsafe extern "C" fn(*mut c_void, *mut CShimMasterStatus) -> c_int;
type DomainStatusFn = unsafe extern "C" fn(*mut c_void, *mut CShimDomainStatus) -> c_int;
type SlaveStatusFn = unsafe extern "C" fn(*mut c_void, u32, *mut CShimSlaveStatus) -> c_int;
type CycleFn = unsafe extern "C" fn(*mut c_void) -> c_int;
type ApplicationTimeFn = unsafe extern "C" fn(*mut c_void, u64) -> c_int;

struct Api {
    master_open: MasterOpenFn,
    master_close: MasterCloseFn,
    domain_create: MasterOpFn,
    slave_configure: SlaveConfigureFn,
    slave_configure_pdos: SlaveConfigurePdosFn,
    slave_configure_dc: SlaveConfigureDcFn,
    entry_register: EntryRegisterFn,
    master_activate: ActivateFn,
    domain_data: DomainDataFn,
    master_status: MasterStatusFn,
    domain_status: DomainStatusFn,
    slave_status: SlaveStatusFn,
    receive: CycleFn,
    domain_process: CycleFn,
    domain_queue: CycleFn,
    send: CycleFn,
    application_time: ApplicationTimeFn,
    sync_clocks: CycleFn,
}

/// A loaded shim library and its resolved symbol table.
struct LoadedShim {
    handle: *mut c_void,
    info: ShimInfo,
    api: Api,
}

// SAFETY: `LoadedShim` is used only through `&mut self`; moving it between
// threads is safe because the dynamic library handle and resolved function
// pointers are process-wide, and the shim never exposes Rust thread-local
// state.
unsafe impl Send for LoadedShim {}

macro_rules! resolve_symbol {
    ($handle:expr, $name:literal, $type:ty) => {{
        let symbol = unsafe { dlsym($handle, concat!($name, "\0").as_ptr().cast::<c_char>()) };
        if symbol.is_null() {
            Err(loader_error(format!(
                "shim is missing required symbol `{}`",
                $name
            )))
        } else {
            // SAFETY: the shim ABI declares these symbols with exactly these
            // signatures; the loader validates the ABI name before using them.
            Ok(unsafe { std::mem::transmute_copy::<*mut c_void, $type>(&symbol) })
        }
    }};
}

impl LoadedShim {
    fn load(path: &Path) -> ShimResult<Self> {
        let path_text = path.to_string_lossy();
        let text = std::ffi::CString::new(path_text.as_bytes()).map_err(|_| {
            loader_error(format!(
                "shim path `{path_text}` contains an interior NUL byte"
            ))
        })?;
        let handle = unsafe { dlopen(text.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        if handle.is_null() {
            let details = unsafe { dlerror() };
            let message = if details.is_null() {
                "dlopen failed".to_owned()
            } else {
                unsafe { CStr::from_ptr(details) }
                    .to_string_lossy()
                    .into_owned()
            };
            return Err(loader_error(message));
        }
        let loaded = Self::resolve(handle);
        if loaded.is_err() {
            unsafe { dlclose(handle) };
        }
        loaded
    }

    fn resolve(handle: *mut c_void) -> ShimResult<Self> {
        let abi_version: unsafe extern "C" fn() -> u32 =
            resolve_symbol!(handle, "stc_ec_abi_version", _)?;
        let abi_name: unsafe extern "C" fn() -> *const c_char =
            resolve_symbol!(handle, "stc_ec_abi_name", _)?;
        let shim_version: unsafe extern "C" fn() -> *const c_char =
            resolve_symbol!(handle, "stc_ec_shim_version", _)?;
        let version = unsafe { abi_version() };
        let name = unsafe { symbol_text(abi_name()) }
            .ok_or_else(|| loader_error("shim returned a null ABI name"))?;
        let version_text = unsafe { symbol_text(shim_version()) }
            .ok_or_else(|| loader_error("shim returned a null version string"))?;
        verify_handshake(version, &name)?;

        let api = Api {
            master_open: resolve_symbol!(handle, "stc_ec_master_open", _)?,
            master_close: resolve_symbol!(handle, "stc_ec_master_close", _)?,
            domain_create: resolve_symbol!(handle, "stc_ec_domain_create", _)?,
            slave_configure: resolve_symbol!(handle, "stc_ec_slave_configure", _)?,
            slave_configure_pdos: resolve_symbol!(handle, "stc_ec_slave_configure_pdos", _)?,
            slave_configure_dc: resolve_symbol!(handle, "stc_ec_slave_configure_dc", _)?,
            entry_register: resolve_symbol!(handle, "stc_ec_entry_register", _)?,
            master_activate: resolve_symbol!(handle, "stc_ec_master_activate", _)?,
            domain_data: resolve_symbol!(handle, "stc_ec_domain_data", _)?,
            master_status: resolve_symbol!(handle, "stc_ec_master_status", _)?,
            domain_status: resolve_symbol!(handle, "stc_ec_domain_status", _)?,
            slave_status: resolve_symbol!(handle, "stc_ec_slave_status", _)?,
            receive: resolve_symbol!(handle, "stc_ec_receive", _)?,
            domain_process: resolve_symbol!(handle, "stc_ec_domain_process", _)?,
            domain_queue: resolve_symbol!(handle, "stc_ec_domain_queue", _)?,
            send: resolve_symbol!(handle, "stc_ec_send", _)?,
            application_time: resolve_symbol!(handle, "stc_ec_application_time", _)?,
            sync_clocks: resolve_symbol!(handle, "stc_ec_sync_clocks", _)?,
        };
        Ok(Self {
            handle,
            info: ShimInfo {
                abi_version: version,
                name,
                shim_version: version_text,
            },
            api,
        })
    }

    fn info(&self) -> ShimInfo {
        self.info.clone()
    }
}

/// Reads a shim-owned C string.
///
/// # Safety
///
/// `pointer` must be null or point to a valid NUL-terminated string that stays
/// alive for the duration of the call.
unsafe fn symbol_text(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    Some(
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned(),
    )
}

impl Drop for LoadedShim {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { dlclose(self.handle) };
            self.handle = std::ptr::null_mut();
        }
    }
}

impl EcShim for LoadedShim {
    fn master_open(&mut self, master_index: u16) -> ShimResult<*mut c_void> {
        let mut master: *mut c_void = std::ptr::null_mut();
        let mut error = [0_u8; ERROR_BUFFER];
        let rc = unsafe {
            (self.api.master_open)(
                master_index,
                &mut master,
                error.as_mut_ptr().cast::<c_char>(),
                error.len(),
            )
        };
        if rc != 0 || master.is_null() {
            return Err(call_error(rc, &error, "master open"));
        }
        Ok(master)
    }

    fn master_close(&mut self, master: *mut c_void) -> ShimResult<()> {
        let rc = unsafe { (self.api.master_close)(master) };
        if rc != 0 {
            return Err(plain_call_error(rc, "master close"));
        }
        Ok(())
    }

    fn domain_create(&mut self, master: *mut c_void) -> ShimResult<()> {
        let mut error = [0_u8; ERROR_BUFFER];
        let rc = unsafe {
            (self.api.domain_create)(master, error.as_mut_ptr().cast::<c_char>(), error.len())
        };
        if rc != 0 {
            return Err(call_error(rc, &error, "domain create"));
        }
        Ok(())
    }

    fn slave_configure(
        &mut self,
        master: *mut c_void,
        slave: u32,
        desc: &SlaveDesc,
    ) -> ShimResult<()> {
        let desc = CShimSlaveDesc {
            alias: desc.alias,
            position: desc.position,
            vendor_id: desc.vendor_id,
            product_code: desc.product_code,
            revision: desc.revision,
            check_revision: u8::from(desc.check_revision),
            reserved: [0; 3],
        };
        let mut error = [0_u8; ERROR_BUFFER];
        let rc = unsafe {
            (self.api.slave_configure)(
                master,
                slave,
                &desc,
                error.as_mut_ptr().cast::<c_char>(),
                error.len(),
            )
        };
        if rc != 0 {
            return Err(call_error(rc, &error, "slave configure"));
        }
        Ok(())
    }

    fn slave_configure_pdos(
        &mut self,
        master: *mut c_void,
        slave: u32,
        syncs: &[SyncDesc],
        pdos: &[PdoDesc],
        entries: &[EntryDesc],
    ) -> ShimResult<()> {
        let c_syncs: Vec<CShimSyncDesc> = syncs
            .iter()
            .map(|sync| CShimSyncDesc {
                index: sync.index,
                direction: sync.direction,
                watchdog: sync.watchdog,
                reserved: 0,
                pdo_offset: sync.pdo_offset,
                pdo_count: sync.pdo_count,
            })
            .collect();
        let c_pdos: Vec<CShimPdoDesc> = pdos
            .iter()
            .map(|pdo| CShimPdoDesc {
                index: pdo.index,
                reserved: 0,
                entry_offset: pdo.entry_offset,
                entry_count: pdo.entry_count,
            })
            .collect();
        let c_entries: Vec<CShimEntryDesc> = entries
            .iter()
            .map(|entry| CShimEntryDesc {
                index: entry.index,
                subindex: entry.subindex,
                bits: entry.bits,
                direction: entry.direction,
                reserved: [0; 3],
            })
            .collect();
        let mut error = [0_u8; ERROR_BUFFER];
        let rc = unsafe {
            (self.api.slave_configure_pdos)(
                master,
                slave,
                slice_ptr(&c_syncs),
                c_syncs.len(),
                slice_ptr(&c_pdos),
                c_pdos.len(),
                slice_ptr(&c_entries),
                c_entries.len(),
                error.as_mut_ptr().cast::<c_char>(),
                error.len(),
            )
        };
        if rc != 0 {
            return Err(call_error(rc, &error, "slave PDO configure"));
        }
        Ok(())
    }

    fn slave_configure_dc(
        &mut self,
        master: *mut c_void,
        slave: u32,
        dc: &DcConfig,
    ) -> ShimResult<()> {
        let mut error = [0_u8; ERROR_BUFFER];
        let rc = unsafe {
            (self.api.slave_configure_dc)(
                master,
                slave,
                dc.assign_activate,
                dc.sync0_cycle,
                dc.sync0_shift,
                dc.sync1_cycle,
                dc.sync1_shift,
                error.as_mut_ptr().cast::<c_char>(),
                error.len(),
            )
        };
        if rc != 0 {
            return Err(call_error(rc, &error, "slave DC configure"));
        }
        Ok(())
    }

    fn entry_register(
        &mut self,
        master: *mut c_void,
        slave: u32,
        index: u16,
        subindex: u8,
    ) -> ShimResult<(i32, u32)> {
        let mut offset = 0_i32;
        let mut bit_position = 0_u32;
        let mut error = [0_u8; ERROR_BUFFER];
        let rc = unsafe {
            (self.api.entry_register)(
                master,
                slave,
                index,
                subindex,
                &mut offset,
                &mut bit_position,
                error.as_mut_ptr().cast::<c_char>(),
                error.len(),
            )
        };
        if rc != 0 {
            return Err(call_error(rc, &error, "PDO entry registration"));
        }
        Ok((offset, bit_position))
    }

    fn master_activate(&mut self, master: *mut c_void) -> ShimResult<()> {
        let mut error = [0_u8; ERROR_BUFFER];
        let rc = unsafe {
            (self.api.master_activate)(master, error.as_mut_ptr().cast::<c_char>(), error.len())
        };
        if rc != 0 {
            return Err(call_error(rc, &error, "master activation"));
        }
        Ok(())
    }

    fn domain_data(&mut self, master: *mut c_void) -> ShimResult<(*mut u8, usize)> {
        let mut data: *mut u8 = std::ptr::null_mut();
        let mut size = 0_usize;
        let mut error = [0_u8; ERROR_BUFFER];
        let rc = unsafe {
            (self.api.domain_data)(
                master,
                &mut data,
                &mut size,
                error.as_mut_ptr().cast::<c_char>(),
                error.len(),
            )
        };
        if rc != 0 {
            return Err(call_error(rc, &error, "domain data"));
        }
        Ok((data, size))
    }

    fn master_status(&mut self, master: *mut c_void) -> ShimResult<MasterStatus> {
        let mut out = CShimMasterStatus::default();
        let rc = unsafe { (self.api.master_status)(master, &mut out) };
        if rc != 0 {
            return Err(plain_call_error(rc, "master status"));
        }
        Ok(MasterStatus {
            slaves_responding: out.slaves_responding,
            link_up: out.link_up != 0,
        })
    }

    fn domain_status(&mut self, master: *mut c_void) -> ShimResult<DomainStatus> {
        let mut out = CShimDomainStatus::default();
        let rc = unsafe { (self.api.domain_status)(master, &mut out) };
        if rc != 0 {
            return Err(plain_call_error(rc, "domain status"));
        }
        Ok(DomainStatus {
            working_counter: out.working_counter,
            wc_state: out.wc_state,
        })
    }

    fn slave_status(&mut self, master: *mut c_void, slave: u32) -> ShimResult<SlaveStatus> {
        let mut out = CShimSlaveStatus::default();
        let rc = unsafe { (self.api.slave_status)(master, slave, &mut out) };
        if rc != 0 {
            return Err(plain_call_error(rc, "slave status"));
        }
        Ok(SlaveStatus {
            al_state: out.al_state,
            online: out.online != 0,
            operational: out.operational != 0,
        })
    }

    fn receive(&mut self, master: *mut c_void) -> ShimResult<()> {
        self.cycle(master, "receive")
    }

    fn domain_process(&mut self, master: *mut c_void) -> ShimResult<()> {
        self.cycle(master, "domain process")
    }

    fn domain_queue(&mut self, master: *mut c_void) -> ShimResult<()> {
        self.cycle(master, "domain queue")
    }

    fn send(&mut self, master: *mut c_void) -> ShimResult<()> {
        self.cycle(master, "send")
    }

    fn application_time(&mut self, master: *mut c_void, app_time: u64) -> ShimResult<()> {
        let rc = unsafe { (self.api.application_time)(master, app_time) };
        if rc != 0 {
            return Err(plain_call_error(rc, "application time"));
        }
        Ok(())
    }

    fn sync_clocks(&mut self, master: *mut c_void) -> ShimResult<()> {
        self.cycle(master, "clock sync")
    }
}

impl LoadedShim {
    fn cycle(&mut self, master: *mut c_void, operation: &str) -> ShimResult<()> {
        let rc = match operation {
            "receive" => unsafe { (self.api.receive)(master) },
            "domain process" => unsafe { (self.api.domain_process)(master) },
            "domain queue" => unsafe { (self.api.domain_queue)(master) },
            "send" => unsafe { (self.api.send)(master) },
            _ => unsafe { (self.api.sync_clocks)(master) },
        };
        if rc != 0 {
            return Err(plain_call_error(rc, operation));
        }
        Ok(())
    }
}

fn slice_ptr<T>(slice: &[T]) -> *const T {
    if slice.is_empty() {
        std::ptr::null()
    } else {
        slice.as_ptr()
    }
}

/// Plain shim-facing slave identity.
#[derive(Clone, Copy, Debug, Default)]
struct SlaveDesc {
    alias: u16,
    position: u16,
    vendor_id: u32,
    product_code: u32,
    revision: u32,
    check_revision: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct SyncDesc {
    index: u8,
    direction: u8,
    watchdog: u8,
    pdo_offset: u32,
    pdo_count: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct PdoDesc {
    index: u16,
    entry_offset: u32,
    entry_count: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct EntryDesc {
    index: u16,
    subindex: u8,
    bits: u8,
    direction: u8,
}

#[derive(Clone, Copy, Debug, Default)]
struct MasterStatus {
    slaves_responding: u32,
    link_up: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct DomainStatus {
    working_counter: u16,
    wc_state: u8,
}

#[derive(Clone, Copy, Debug, Default)]
struct SlaveStatus {
    al_state: u16,
    online: bool,
    operational: bool,
}

/// The shim operations the backend uses.
///
/// The trait exists so lifecycle and policy can be tested without a master or
/// a built shim; [`LoadedShim`] is the production implementation.
trait EcShim: Send {
    fn master_open(&mut self, master_index: u16) -> ShimResult<*mut c_void>;
    fn master_close(&mut self, master: *mut c_void) -> ShimResult<()>;
    fn domain_create(&mut self, master: *mut c_void) -> ShimResult<()>;
    fn slave_configure(
        &mut self,
        master: *mut c_void,
        slave: u32,
        desc: &SlaveDesc,
    ) -> ShimResult<()>;
    fn slave_configure_pdos(
        &mut self,
        master: *mut c_void,
        slave: u32,
        syncs: &[SyncDesc],
        pdos: &[PdoDesc],
        entries: &[EntryDesc],
    ) -> ShimResult<()>;
    fn slave_configure_dc(
        &mut self,
        master: *mut c_void,
        slave: u32,
        dc: &DcConfig,
    ) -> ShimResult<()>;
    fn entry_register(
        &mut self,
        master: *mut c_void,
        slave: u32,
        index: u16,
        subindex: u8,
    ) -> ShimResult<(i32, u32)>;
    fn master_activate(&mut self, master: *mut c_void) -> ShimResult<()>;
    fn domain_data(&mut self, master: *mut c_void) -> ShimResult<(*mut u8, usize)>;
    fn master_status(&mut self, master: *mut c_void) -> ShimResult<MasterStatus>;
    fn domain_status(&mut self, master: *mut c_void) -> ShimResult<DomainStatus>;
    fn slave_status(&mut self, master: *mut c_void, slave: u32) -> ShimResult<SlaveStatus>;
    fn receive(&mut self, master: *mut c_void) -> ShimResult<()>;
    fn domain_process(&mut self, master: *mut c_void) -> ShimResult<()>;
    fn domain_queue(&mut self, master: *mut c_void) -> ShimResult<()>;
    fn send(&mut self, master: *mut c_void) -> ShimResult<()>;
    fn application_time(&mut self, master: *mut c_void, app_time: u64) -> ShimResult<()>;
    fn sync_clocks(&mut self, master: *mut c_void) -> ShimResult<()>;
}

/// A distributed-clock configuration in IgH units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DcConfig {
    assign_activate: u16,
    sync0_cycle: u32,
    sync0_shift: i32,
    sync1_cycle: u32,
    sync1_shift: i32,
}

#[derive(Clone, Copy, Debug)]
struct EntryInfo {
    bits: u8,
    direction: u8,
}

type EntryKey = (u16, u8);
type EntryInfoMap = BTreeMap<EntryKey, EntryInfo>;
type OccupiedEntries = BTreeMap<EntryKey, Vec<(u32, u32, String)>>;
type ParsedSyncTree = (Vec<SyncDesc>, Vec<PdoDesc>, Vec<EntryDesc>, EntryInfoMap);

struct ChannelConfig {
    name: String,
    entry_index: u16,
    entry_subindex: u8,
    bit: u32,
    width_bits: u32,
    direction: u8,
    image: Option<ImageRange>,
}

struct ParsedDevice {
    name: String,
    master: u16,
    desc: SlaveDesc,
    startup_timeout: Duration,
    recovery_cycles: u32,
    syncs: Vec<SyncDesc>,
    pdos: Vec<PdoDesc>,
    entries: Vec<EntryDesc>,
    channels: Vec<ChannelConfig>,
    dc: Option<DcConfig>,
}

#[derive(Clone, Copy, Debug)]
enum StatusField {
    MasterPhase,
    MasterActive,
    MasterSlavesResponding,
    MasterLinkUp,
    MasterLastError,
    MasterRecoveries,
    DomainWcState,
    DomainWorkingCounter,
    InputStale,
    CycleExchanges,
    CycleFaults,
    SlaveAlState(usize),
    SlaveOnline(usize),
    SlaveOperational(usize),
}

#[derive(Clone)]
struct StatusChannel {
    field: StatusField,
    range: ImageRange,
}

struct ParsedConfig {
    master: u16,
    devices: Vec<ParsedDevice>,
    status_channels: Vec<StatusChannel>,
    startup_timeout: Duration,
    recovery_cycles: u32,
}

struct RuntimeChannel {
    image: Option<ImageRange>,
    domain_bit: u64,
    width_bits: u32,
    direction: u8,
}

struct Active {
    master: *mut c_void,
    domain_data: *mut u8,
    domain_size: usize,
    channels: Vec<RuntimeChannel>,
    status_channels: Vec<StatusChannel>,
    device_count: usize,
    dc_active: bool,
}

#[derive(Clone, Default)]
struct Snapshot {
    master: MasterStatus,
    domain: DomainStatus,
    slaves: Vec<SlaveStatus>,
}

/// The EtherCAT `DeviceBackend`.
///
/// One instance serves both the `ethercat` and `ethercat-status` kinds; the
/// coordinator groups them under one instance name.
pub struct EthercatBackend {
    injected: Option<Box<dyn EcShim>>,
    shim: Option<Box<dyn EcShim>>,
    active: Option<Active>,
    status: BackendStatus,
    snapshot: Snapshot,
    faulted: bool,
    stale: bool,
    recovery_streak: u32,
    recovery_cycles: u32,
    recoveries: u64,
    last_error_code: u32,
}

// SAFETY: the backend is used only through `&mut self`; the raw master and
// domain pointers are owned by the shim for the active lifetime and are
// process memory. Moving the backend between threads is safe as long as access
// is not concurrent, which the coordinator guarantees.
unsafe impl Send for EthercatBackend {}

impl Default for EthercatBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl EthercatBackend {
    /// Creates an unstarted backend. The shim is loaded at cold start.
    #[must_use]
    pub fn new() -> Self {
        Self {
            injected: None,
            shim: None,
            active: None,
            status: BackendStatus::default(),
            snapshot: Snapshot::default(),
            faulted: false,
            stale: false,
            recovery_streak: 0,
            recovery_cycles: DEFAULT_RECOVERY_CYCLES,
            recoveries: 0,
            last_error_code: 0,
        }
    }

    #[cfg(test)]
    fn with_shim(shim: Box<dyn EcShim>) -> Self {
        let mut backend = Self::new();
        backend.injected = Some(shim);
        backend
    }

    fn release_active(&mut self) -> Option<ShimCallError> {
        let mut error = None;
        if let Some(active) = self.active.take()
            && let Some(shim) = self.shim.as_mut()
            && let Err(close_error) = shim.master_close(active.master)
        {
            error = Some(close_error);
        }
        self.shim = None;
        error
    }

    fn enter_fault(&mut self, message: &str, code: Option<i32>) {
        if !self.faulted {
            self.faulted = true;
            self.status.faults = self.status.faults.saturating_add(1);
            self.status.last_fault = Some(format!("{}: {message}", codes::W_ETHERCAT_FAULT));
        }
        if let Some(code) = code {
            self.last_error_code = u32::try_from(code).unwrap_or(0);
        }
        self.stale = true;
        self.recovery_streak = 0;
        self.status.state = BackendState::Faulted;
    }

    fn note_health(&mut self, healthy: bool, error: Option<&ShimCallError>) {
        if healthy {
            if self.faulted {
                self.recovery_streak = self.recovery_streak.saturating_add(1);
                if self.recovery_streak >= self.recovery_cycles {
                    self.faulted = false;
                    self.stale = false;
                    self.recovery_streak = 0;
                    self.recoveries = self.recoveries.saturating_add(1);
                    self.status.state = BackendState::Running;
                } else {
                    self.status.state = BackendState::Recovering;
                }
            } else {
                self.stale = false;
                self.status.state = BackendState::Running;
            }
            return;
        }
        let message = error.map_or_else(
            || "fieldbus is not operational".to_owned(),
            |error| error.message.clone(),
        );
        self.enter_fault(&message, error.map(|error| error.code));
    }

    fn copy_inputs(&self, image: &mut ProcessImage) -> std::result::Result<(), String> {
        let Some(active) = self.active.as_ref() else {
            return Err("backend is not started".into());
        };
        if active.domain_size == 0 {
            return Ok(());
        }
        if active.domain_data.is_null() {
            return Err("domain data is null".into());
        }
        // SAFETY: the shim owns the domain buffer for the lifetime of the
        // active master; `domain_size` was reported with the same pointer and
        // is not resized while the master is active.
        let data = unsafe { std::slice::from_raw_parts(active.domain_data, active.domain_size) };
        for channel in &active.channels {
            let (Some(range), DIR_INPUT) = (channel.image, channel.direction) else {
                continue;
            };
            let value =
                domain_read(data, channel.domain_bit, channel.width_bits).ok_or_else(|| {
                    format!(
                        "input channel image range at bit {} leaves the process image",
                        range.offset_bits
                    )
                })?;
            if !image.write_bits(range.area, range.offset_bits, range.width_bits, value) {
                return Err(format!(
                    "input channel image range at bit {} leaves the process image",
                    range.offset_bits
                ));
            }
        }
        Ok(())
    }

    fn write_outputs(
        &mut self,
        image: &ProcessImage,
    ) -> std::result::Result<(), (String, Option<i32>)> {
        let Some(active) = self.active.as_mut() else {
            return Err(("backend is not started".into(), None));
        };
        if active.domain_size == 0 {
            return Ok(());
        }
        if active.domain_data.is_null() {
            return Err(("domain data is null".into(), None));
        }
        // SAFETY: the shim owns the domain buffer for the lifetime of the
        // active master; access is exclusive here because the backend holds
        // `&mut self`.
        let data =
            unsafe { std::slice::from_raw_parts_mut(active.domain_data, active.domain_size) };
        for channel in &active.channels {
            let (Some(range), DIR_OUTPUT) = (channel.image, channel.direction) else {
                continue;
            };
            let Some(value) = image.read_bits(range.area, range.offset_bits, range.width_bits)
            else {
                return Err((
                    format!(
                        "output channel image range at bit {} leaves the process image",
                        range.offset_bits
                    ),
                    None,
                ));
            };
            if !domain_write(data, channel.domain_bit, channel.width_bits, value) {
                return Err((
                    format!(
                        "output channel image range at bit {} leaves the domain",
                        range.offset_bits
                    ),
                    None,
                ));
            }
        }
        Ok(())
    }

    fn fill_status(&self, image: &mut ProcessImage) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        for channel in &active.status_channels {
            let value = match channel.field {
                StatusField::MasterPhase => MASTER_PHASE_OPERATION,
                StatusField::MasterActive => 1,
                StatusField::MasterSlavesResponding => {
                    u64::from(self.snapshot.master.slaves_responding)
                }
                StatusField::MasterLinkUp => u64::from(self.snapshot.master.link_up),
                StatusField::MasterLastError => u64::from(self.last_error_code),
                StatusField::MasterRecoveries => self.recoveries,
                StatusField::DomainWcState => u64::from(self.snapshot.domain.wc_state),
                StatusField::DomainWorkingCounter => {
                    u64::from(self.snapshot.domain.working_counter)
                }
                StatusField::InputStale => u64::from(self.stale),
                StatusField::CycleExchanges => self.status.exchanges,
                StatusField::CycleFaults => self.status.faults,
                StatusField::SlaveAlState(index) => self
                    .snapshot
                    .slaves
                    .get(index)
                    .map_or(0, |slave| u64::from(slave.al_state)),
                StatusField::SlaveOnline(index) => self
                    .snapshot
                    .slaves
                    .get(index)
                    .map_or(0, |slave| u64::from(slave.online)),
                StatusField::SlaveOperational(index) => self
                    .snapshot
                    .slaves
                    .get(index)
                    .map_or(0, |slave| u64::from(slave.operational)),
            };
            let _ = image.write_bits(
                channel.range.area,
                channel.range.offset_bits,
                channel.range.width_bits,
                value,
            );
        }
    }
}

impl Drop for EthercatBackend {
    fn drop(&mut self) {
        let _ = self.release_active();
    }
}

impl DeviceBackend for EthercatBackend {
    fn cold_start(&mut self, config: &BackendConfig) -> Result<()> {
        let _ = self.release_active();
        self.status = BackendStatus {
            name: config.name.clone(),
            devices: config.devices.len(),
            state: BackendState::Ready,
            exchanges: 0,
            faults: 0,
            last_fault: None,
        };
        self.snapshot = Snapshot::default();
        self.faulted = false;
        self.stale = false;
        self.recovery_streak = 0;
        self.recoveries = 0;
        self.last_error_code = 0;

        let parsed = parse_config(config)?;
        let mut shim = match self.injected.take() {
            Some(shim) => shim,
            None => load_default_shim()?,
        };
        let active = match start_active(&mut *shim, &parsed) {
            Ok(active) => active,
            Err(error) => {
                self.injected = Some(shim);
                return Err(error);
            }
        };
        self.recovery_cycles = parsed.recovery_cycles;
        self.snapshot = Snapshot {
            slaves: vec![SlaveStatus::default(); active.device_count],
            ..Snapshot::default()
        };
        self.shim = Some(shim);
        self.active = Some(active);
        self.status.state = BackendState::Ready;
        Ok(())
    }

    fn latch_inputs(&mut self, image: &mut ProcessImage) -> Result<()> {
        let Some(active) = self.active.as_ref() else {
            return Err(Error::new(
                codes::ETH_ACTIVATE,
                "EtherCAT backend has not been started",
            ));
        };
        let Some(shim) = self.shim.as_mut() else {
            return Err(Error::new(
                codes::ETH_ACTIVATE,
                "EtherCAT shim is not loaded",
            ));
        };
        let exchange = exchange_cycle(&mut **shim, active);
        let healthy = exchange
            .as_ref()
            .is_ok_and(|snapshot| is_healthy(snapshot, active.device_size()));
        let error = exchange.as_ref().err();
        self.note_health(healthy, error);
        if let Ok(snapshot) = &exchange {
            self.snapshot = snapshot.clone();
        }
        if !self.faulted
            && !self.stale
            && let Err(message) = self.copy_inputs(image)
        {
            self.enter_fault(&message, None);
        }
        self.fill_status(image);
        Ok(())
    }

    fn publish_outputs(&mut self, image: &ProcessImage) -> Result<()> {
        if let Err((message, code)) = self.write_outputs(image) {
            self.enter_fault(&message, code);
            return Ok(());
        }
        let Some(active) = self.active.as_ref() else {
            return Ok(());
        };
        let Some(shim) = self.shim.as_mut() else {
            return Ok(());
        };
        let result = shim
            .domain_queue(active.master)
            .and_then(|()| shim.send(active.master));
        match result {
            Ok(()) => {
                self.status.exchanges = self.status.exchanges.saturating_add(1);
                if !self.faulted {
                    self.status.state = BackendState::Running;
                }
            }
            Err(error) => {
                let message = error.message.clone();
                self.enter_fault(&message, Some(error.code));
            }
        }
        Ok(())
    }

    fn status(&self) -> BackendStatus {
        self.status.clone()
    }

    fn diagnostics(&self) -> BTreeMap<String, String> {
        let mut values = BTreeMap::new();
        values.insert(
            "master.slaves_responding".into(),
            self.snapshot.master.slaves_responding.to_string(),
        );
        values.insert(
            "master.link_up".into(),
            u8::from(self.snapshot.master.link_up).to_string(),
        );
        values.insert(
            "domain.wc_state".into(),
            self.snapshot.domain.wc_state.to_string(),
        );
        values.insert(
            "domain.working_counter".into(),
            self.snapshot.domain.working_counter.to_string(),
        );
        values.insert("input.stale".into(), u8::from(self.stale).to_string());
        values.insert("master.recoveries".into(), self.recoveries.to_string());
        values.insert("master.last_error".into(), self.last_error_code.to_string());
        values
    }

    fn shutdown(&mut self) -> Result<()> {
        let error = self.release_active();
        self.status.state = BackendState::Stopped;
        if let Some(error) = error {
            return Err(Error::new(
                codes::ETH_ACTIVATE,
                format!("cannot release EtherCAT master: {}", error.message),
            ));
        }
        Ok(())
    }
}

impl Active {
    fn device_size(&self) -> usize {
        self.device_count
    }
}

fn exchange_cycle(shim: &mut dyn EcShim, active: &Active) -> ShimResult<Snapshot> {
    shim.receive(active.master)?;
    shim.domain_process(active.master)?;
    if active.dc_active {
        shim.application_time(active.master, application_time_ns())?;
        shim.sync_clocks(active.master)?;
    }
    read_snapshot(shim, active.master, active.device_count)
}

fn application_time_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
        })
}

fn read_snapshot(
    shim: &mut dyn EcShim,
    master: *mut c_void,
    device_count: usize,
) -> ShimResult<Snapshot> {
    let master_status = shim.master_status(master)?;
    let domain_status = shim.domain_status(master)?;
    let mut slaves = Vec::with_capacity(device_count);
    for slave in 0..device_count {
        slaves.push(shim.slave_status(master, u32::try_from(slave).unwrap_or(u32::MAX))?);
    }
    Ok(Snapshot {
        master: master_status,
        domain: domain_status,
        slaves,
    })
}

fn is_healthy(snapshot: &Snapshot, domain_size: usize) -> bool {
    snapshot.master.link_up
        && usize::try_from(snapshot.master.slaves_responding)
            .is_ok_and(|responding| responding >= snapshot.slaves.len())
        && snapshot
            .slaves
            .iter()
            .all(|slave| slave.online && slave.operational)
        && (domain_size == 0 || snapshot.domain.wc_state == WC_COMPLETE)
}

fn start_active(shim: &mut dyn EcShim, parsed: &ParsedConfig) -> Result<Active> {
    let master = shim
        .master_open(parsed.master)
        .map_err(|error| Error::new(codes::ETH_MASTER, error.message))?;
    let result = build_active(shim, master, parsed);
    if result.is_err() {
        let _ = shim.master_close(master);
    }
    result
}

fn build_active(
    shim: &mut dyn EcShim,
    master: *mut c_void,
    parsed: &ParsedConfig,
) -> Result<Active> {
    shim.domain_create(master)
        .map_err(|error| Error::new(codes::ETH_ACTIVATE, error.message))?;

    let mut channels = Vec::new();
    for (index, device) in parsed.devices.iter().enumerate() {
        let slave = u32::try_from(index).unwrap_or(u32::MAX);
        shim.slave_configure(master, slave, &device.desc)
            .map_err(|error| Error::new(codes::ETH_SLAVE, error.message))?;
        if !device.syncs.is_empty() {
            shim.slave_configure_pdos(master, slave, &device.syncs, &device.pdos, &device.entries)
                .map_err(|error| Error::new(codes::ETH_PDO, error.message))?;
        }
        if let Some(dc) = &device.dc {
            shim.slave_configure_dc(master, slave, dc)
                .map_err(|error| Error::new(codes::ETH_PDO, error.message))?;
        }
    }

    let mut registered: BTreeMap<(u32, u16, u8), (i32, u32)> = BTreeMap::new();
    for (index, device) in parsed.devices.iter().enumerate() {
        let slave = u32::try_from(index).unwrap_or(u32::MAX);
        for channel in &device.channels {
            let key = (slave, channel.entry_index, channel.entry_subindex);
            let (offset, bit_position) = match registered.get(&key) {
                Some(value) => *value,
                None => {
                    let value = shim
                        .entry_register(master, slave, channel.entry_index, channel.entry_subindex)
                        .map_err(|error| Error::new(codes::ETH_PDO, error.message))?;
                    registered.insert(key, value);
                    value
                }
            };
            if offset < 0 {
                return Err(Error::new(
                    codes::ETH_PDO,
                    format!(
                        "slave `{}` entry 0x{:04x}:{} has a negative domain offset",
                        device.name, channel.entry_index, channel.entry_subindex
                    ),
                ));
            }
            let base = u64::try_from(offset)
                .unwrap_or(0)
                .checked_mul(8)
                .and_then(|value| value.checked_add(u64::from(bit_position)))
                .and_then(|value| value.checked_add(u64::from(channel.bit)));
            let Some(domain_bit) = base else {
                return Err(Error::new(
                    codes::ETH_PDO,
                    format!(
                        "slave `{}` channel `{}` domain offset overflows",
                        device.name, channel.name
                    ),
                ));
            };
            channels.push(RuntimeChannel {
                image: channel.image,
                domain_bit,
                width_bits: channel.width_bits,
                direction: channel.direction,
            });
        }
    }

    shim.master_activate(master)
        .map_err(|error| Error::new(codes::ETH_ACTIVATE, error.message))?;
    let (domain_data, domain_size) = shim
        .domain_data(master)
        .map_err(|error| Error::new(codes::ETH_ACTIVATE, error.message))?;
    if domain_size > 0 && domain_data.is_null() {
        return Err(Error::new(
            codes::ETH_ACTIVATE,
            "shim returned a null domain buffer",
        ));
    }
    let domain_bits = u64::try_from(domain_size)
        .unwrap_or(u64::MAX)
        .saturating_mul(8);
    for channel in &channels {
        if channel
            .domain_bit
            .checked_add(u64::from(channel.width_bits))
            .is_none_or(|end| end > domain_bits)
        {
            return Err(Error::new(
                codes::ETH_PDO,
                format!(
                    "channel domain range at bit {} leaves the {}-byte domain",
                    channel.domain_bit, domain_size
                ),
            ));
        }
    }

    let active = Active {
        master,
        domain_data,
        domain_size,
        channels,
        status_channels: parsed.status_channels.clone(),
        device_count: parsed.devices.len(),
        dc_active: parsed.devices.iter().any(|device| device.dc.is_some()),
    };
    wait_for_operational(shim, &active, parsed.startup_timeout)?;
    Ok(active)
}

fn wait_for_operational(shim: &mut dyn EcShim, active: &Active, timeout: Duration) -> Result<()> {
    let started = Instant::now();
    loop {
        let received = shim
            .receive(active.master)
            .and_then(|()| shim.domain_process(active.master))
            .and_then(|()| {
                if active.dc_active {
                    shim.application_time(active.master, application_time_ns())?;
                    shim.sync_clocks(active.master)?;
                }
                Ok(())
            })
            .and_then(|()| read_snapshot(shim, active.master, active.device_count))
            .and_then(|snapshot| {
                if is_healthy(&snapshot, active.domain_size) {
                    Ok(())
                } else {
                    Err(ShimCallError {
                        code: 0,
                        message: "fieldbus is not operational".into(),
                    })
                }
            });
        // Queue and send the current domain on every attempt. The slave only
        // advances PREOP -> SAFEOP -> OP when it receives process data; waiting
        // without transmitting would never observe a healthy startup.
        let sent = shim
            .domain_queue(active.master)
            .and_then(|()| shim.send(active.master));
        let result = received.and(sent);
        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                if started.elapsed() >= timeout {
                    return Err(Error::new(
                        codes::ETH_STARTUP,
                        format!(
                            "EtherCAT startup timed out after {} ms: {}",
                            timeout.as_millis(),
                            error.message
                        ),
                    ));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn domain_read(data: &[u8], bit_offset: u64, width_bits: u32) -> Option<u64> {
    if width_bits > 64 {
        return None;
    }
    let end = bit_offset.checked_add(u64::from(width_bits))?;
    if end > u64::try_from(data.len()).ok()?.checked_mul(8)? {
        return None;
    }
    let mut value = 0_u64;
    for index in 0..width_bits {
        let absolute = bit_offset.checked_add(u64::from(index))?;
        let byte = usize::try_from(absolute / 8).ok()?;
        let bit = absolute % 8;
        let set = (data.get(byte)? >> bit) & 1;
        value |= u64::from(set) << index;
    }
    Some(value)
}

fn domain_write(data: &mut [u8], bit_offset: u64, width_bits: u32, value: u64) -> bool {
    if width_bits > 64 {
        return false;
    }
    let Some(end) = bit_offset.checked_add(u64::from(width_bits)) else {
        return false;
    };
    let Some(limit) = u64::try_from(data.len())
        .ok()
        .and_then(|len| len.checked_mul(8))
    else {
        return false;
    };
    if end > limit {
        return false;
    }
    for index in 0..width_bits {
        let Some(absolute) = bit_offset.checked_add(u64::from(index)) else {
            return false;
        };
        let Ok(byte) = usize::try_from(absolute / 8) else {
            return false;
        };
        let bit = absolute % 8;
        let set = (value >> index) & 1;
        if set == 0 {
            data[byte] &= !(1 << bit);
        } else {
            data[byte] |= 1 << bit;
        }
    }
    true
}

fn cfg_error(message: impl Into<String>) -> Error {
    Error::new(codes::ETH_CONFIG, message)
}

fn binding_error(message: impl Into<String>) -> Error {
    Error::new(codes::ETH_BINDING, message)
}

fn is_status_device(device: &BackendDeviceConfig) -> bool {
    device.kind == io_mock::ETHERCAT_STATUS_KIND
}

fn parse_config(config: &BackendConfig) -> Result<ParsedConfig> {
    let mut data_devices = Vec::new();
    let mut status_devices = Vec::new();
    let mut names = BTreeSet::new();
    for device in &config.devices {
        if !names.insert(device.name.as_str()) {
            return Err(cfg_error(format!(
                "EtherCAT backend owns device `{}` more than once",
                device.name
            )));
        }
        if is_status_device(device) {
            status_devices.push(device);
        } else if device.kind == io_mock::ETHERCAT_KIND {
            data_devices.push(device);
        } else {
            return Err(cfg_error(format!(
                "device `{}` has kind `{}`, which the EtherCAT backend does not serve",
                device.name, device.kind
            )));
        }
    }
    if data_devices.is_empty() {
        return Err(cfg_error(
            "an EtherCAT backend instance requires at least one data device",
        ));
    }

    let mut devices = Vec::with_capacity(data_devices.len());
    let mut master = None;
    let mut startup_timeout = Duration::ZERO;
    let mut recovery_cycles = 0_u32;
    for device in &data_devices {
        let parsed = parse_data_device(device)?;
        if let Some(previous) = master
            && previous != parsed.master
        {
            return Err(cfg_error(format!(
                "device `{}` uses EtherCAT master {}, but the instance already uses master \
                 {previous}",
                device.name, parsed.master
            )));
        }
        master = Some(parsed.master);
        startup_timeout = startup_timeout.max(parsed.startup_timeout);
        recovery_cycles = recovery_cycles.max(parsed.recovery_cycles);
        devices.push(parsed);
    }
    let mut positions = BTreeMap::new();
    for device in &devices {
        if let Some(previous) = positions.insert(device.desc.position, device.name.as_str()) {
            return Err(cfg_error(format!(
                "devices `{previous}` and `{}` both use slave position {} on EtherCAT master {}",
                device.name, device.desc.position, device.master
            )));
        }
    }

    let data_count = devices.len();
    let mut status_channels = Vec::new();
    for device in &status_devices {
        parse_status_device(device, data_count, &mut status_channels)?;
    }
    validate_bindings(config)?;
    validate_layout_ranges(config)?;

    Ok(ParsedConfig {
        master: master.unwrap_or(0),
        devices,
        status_channels,
        startup_timeout,
        recovery_cycles,
    })
}

fn parse_data_device(device: &BackendDeviceConfig) -> Result<ParsedDevice> {
    let Some(text) = &device.config else {
        return Err(cfg_error(format!(
            "EtherCAT device `{}` has no profile or inline configuration",
            device.name
        )));
    };
    let root = json::parse(text).map_err(|error| {
        cfg_error(format!(
            "device `{}` configuration is not valid JSON: {}",
            device.name, error.message
        ))
    })?;
    let object = root.as_object().ok_or_else(|| {
        cfg_error(format!(
            "device `{}` configuration is not a JSON object",
            device.name
        ))
    })?;

    let master = optional_u16(object, "master", device)?.unwrap_or(0);
    let alias = optional_u16(object, "alias", device)?.unwrap_or(0);
    let position = required_u16(object, "position", device)?;
    let vendor_id = required_identity(object, "vendor", device)?;
    let product_code = required_identity(object, "product", device)?;
    let check_revision = object.contains_key("revision");
    let revision = optional_identity(object, "revision", device)?.unwrap_or(0);
    let startup_timeout = match object.get("startup-timeout") {
        None => DEFAULT_STARTUP_TIMEOUT,
        Some(value) => {
            let text = value.as_str().ok_or_else(|| {
                cfg_error(format!(
                    "device `{}` `startup-timeout` must be a duration string such as `5s`",
                    device.name
                ))
            })?;
            let nanos = crate::run::parse_duration_ns(text).ok_or_else(|| {
                cfg_error(format!(
                    "device `{}` `startup-timeout` must be a duration string such as `5s`",
                    device.name
                ))
            })?;
            Duration::from_nanos(nanos)
        }
    };
    let recovery_cycles = match object.get("recovery-cycles") {
        None => DEFAULT_RECOVERY_CYCLES,
        Some(value) => value
            .as_number()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                cfg_error(format!(
                    "device `{}` `recovery-cycles` must be a non-negative integer",
                    device.name
                ))
            })?,
    };
    let dc = parse_dc(device, object)?;

    let (syncs, pdos, entries, entry_info) = parse_syncs(device, object)?;
    let channels = parse_pdo_channels(device, object, &entry_info)?;

    let mut declared = BTreeSet::new();
    for channel in &device.channels {
        if !declared.insert(channel.name.as_str()) {
            return Err(cfg_error(format!(
                "device `{}` declares channel `{}` more than once",
                device.name, channel.name
            )));
        }
        let direction = parse_direction(&channel.direction).ok_or_else(|| {
            cfg_error(format!(
                "device `{}` channel `{}` has direction `{}`; expected `input` or `output`",
                device.name, channel.name, channel.direction
            ))
        })?;
        if !matches!(channel.width_bits, 1 | 8 | 16 | 32) {
            return Err(cfg_error(format!(
                "device `{}` channel `{}` has width {} bits; expected 1, 8, 16, or 32",
                device.name, channel.name, channel.width_bits
            )));
        }
        if let Some(image) = channel.image {
            validate_channel_image(device, channel, direction, image)?;
        }
    }

    let mut mapped = BTreeSet::new();
    let mut occupied: OccupiedEntries = BTreeMap::new();
    let mut channel_configs = Vec::with_capacity(channels.len());
    for channel in &channels {
        if !mapped.insert(channel.name.as_str()) {
            return Err(cfg_error(format!(
                "device `{}` maps channel `{}` more than once",
                device.name, channel.name
            )));
        }
        let Some(declared_channel) = device
            .channels
            .iter()
            .find(|declared| declared.name == channel.name)
        else {
            return Err(cfg_error(format!(
                "device `{}` maps channel `{}`, which is not declared in `stc.toml`",
                device.name, channel.name
            )));
        };
        let Some(entry) = entry_info.get(&(channel.entry_index, channel.entry_subindex)) else {
            return Err(cfg_error(format!(
                "device `{}` channel `{}` names PDO entry `0x{:04x}:{}`, which is not declared",
                device.name, channel.name, channel.entry_index, channel.entry_subindex
            )));
        };
        let width = declared_channel.width_bits;
        let Some(end) = channel.bit.checked_add(width) else {
            return Err(cfg_error(format!(
                "device `{}` channel `{}` bit range overflows",
                device.name, channel.name
            )));
        };
        if channel.bit >= u32::from(entry.bits) || end > u32::from(entry.bits) {
            return Err(cfg_error(format!(
                "device `{}` channel `{}` does not fit inside PDO entry `0x{:04x}:{}`",
                device.name, channel.name, channel.entry_index, channel.entry_subindex
            )));
        }
        let direction = parse_direction(&declared_channel.direction)
            .expect("declared channel direction validated above");
        if entry.direction != direction {
            return Err(cfg_error(format!(
                "device `{}` channel `{}` does not match the PDO entry direction",
                device.name, channel.name
            )));
        }
        if width > 1 && !channel.bit.is_multiple_of(8) {
            return Err(cfg_error(format!(
                "device `{}` channel `{}` with width {width} is not byte-aligned in its PDO entry",
                device.name, channel.name
            )));
        }
        let spans = occupied
            .entry((channel.entry_index, channel.entry_subindex))
            .or_default();
        for (start, other_end, other_name) in spans.iter() {
            if channel.bit < *other_end && *start < end {
                return Err(cfg_error(format!(
                    "device `{}` channels `{}` and `{other_name}` overlap inside PDO entry \
                     `0x{:04x}:{}`",
                    device.name, channel.name, channel.entry_index, channel.entry_subindex
                )));
            }
        }
        spans.push((channel.bit, end, channel.name.clone()));
        channel_configs.push(ChannelConfig {
            name: channel.name.clone(),
            entry_index: channel.entry_index,
            entry_subindex: channel.entry_subindex,
            bit: channel.bit,
            width_bits: width,
            direction,
            image: declared_channel.image,
        });
    }
    for name in declared {
        if !mapped.contains(name) {
            return Err(cfg_error(format!(
                "device `{}` channel `{name}` has no `pdo-channels` mapping",
                device.name
            )));
        }
    }

    Ok(ParsedDevice {
        name: device.name.clone(),
        master,
        desc: SlaveDesc {
            alias,
            position,
            vendor_id,
            product_code,
            revision,
            check_revision,
        },
        startup_timeout,
        recovery_cycles,
        syncs,
        pdos,
        entries,
        channels: channel_configs,
        dc,
    })
}

fn parse_syncs(
    device: &BackendDeviceConfig,
    object: &BTreeMap<String, Json>,
) -> Result<ParsedSyncTree> {
    let syncs = match object.get("syncs") {
        None => &[][..],
        Some(value) => value.as_array().ok_or_else(|| {
            cfg_error(format!("device `{}` `syncs` is not an array", device.name))
        })?,
    };
    let mut sync_descs = Vec::with_capacity(syncs.len());
    let mut pdo_descs = Vec::new();
    let mut entry_descs = Vec::new();
    let mut entry_info = BTreeMap::new();
    let mut sync_indexes = BTreeSet::new();
    for sync in syncs {
        let sync = sync.as_object().ok_or_else(|| {
            cfg_error(format!(
                "device `{}` has a sync that is not an object",
                device.name
            ))
        })?;
        let index = required_u8(sync, "index", device, "sync index")?;
        if !sync_indexes.insert(index) {
            return Err(cfg_error(format!(
                "device `{}` declares sync index {index} more than once",
                device.name
            )));
        }
        let direction = sync
            .get("direction")
            .and_then(Json::as_str)
            .and_then(parse_direction)
            .ok_or_else(|| {
                cfg_error(format!(
                    "device `{}` sync {index} has no direction `input` or `output`",
                    device.name
                ))
            })?;
        let watchdog = match sync.get("watchdog").and_then(Json::as_str) {
            None | Some("default") => 0,
            Some("enable") => 1,
            Some("disable") => 2,
            Some(other) => {
                return Err(cfg_error(format!(
                    "device `{}` sync {index} has watchdog `{other}`; expected `enable` or \
                     `disable`",
                    device.name
                )));
            }
        };
        let pdos = match sync.get("pdos") {
            None => &[][..],
            Some(value) => value.as_array().ok_or_else(|| {
                cfg_error(format!("device `{}` `pdos` is not an array", device.name))
            })?,
        };
        let pdo_offset = u32::try_from(pdo_descs.len()).unwrap_or(u32::MAX);
        for pdo in pdos {
            let pdo = pdo.as_object().ok_or_else(|| {
                cfg_error(format!(
                    "device `{}` has a PDO that is not an object",
                    device.name
                ))
            })?;
            let pdo_index = required_u16(pdo, "index", device)?;
            let values = match pdo.get("entries") {
                None => &[][..],
                Some(value) => value.as_array().ok_or_else(|| {
                    cfg_error(format!(
                        "device `{}` `entries` is not an array",
                        device.name
                    ))
                })?,
            };
            let entry_offset = u32::try_from(entry_descs.len()).unwrap_or(u32::MAX);
            for entry in values {
                let entry = entry.as_object().ok_or_else(|| {
                    cfg_error(format!(
                        "device `{}` has a PDO entry that is not an object",
                        device.name
                    ))
                })?;
                let entry_index = required_u16(entry, "index", device)?;
                let subindex = required_u8(entry, "subindex", device, "PDO entry subindex")?;
                let bits = required_u8(entry, "bits", device, "PDO entry bits")?;
                if !(1..=64).contains(&bits) {
                    return Err(cfg_error(format!(
                        "device `{}` PDO entry `0x{entry_index:04x}:{subindex}` has width {bits}; \
                         expected 1..=64",
                        device.name
                    )));
                }
                if entry_info
                    .insert((entry_index, subindex), EntryInfo { bits, direction })
                    .is_some()
                {
                    return Err(cfg_error(format!(
                        "device `{}` declares PDO entry `0x{entry_index:04x}:{subindex}` more \
                         than once",
                        device.name
                    )));
                }
                entry_descs.push(EntryDesc {
                    index: entry_index,
                    subindex,
                    bits,
                    direction,
                });
            }
            pdo_descs.push(PdoDesc {
                index: pdo_index,
                entry_offset,
                entry_count: u32::try_from(values.len()).unwrap_or(u32::MAX),
            });
        }
        sync_descs.push(SyncDesc {
            index,
            direction,
            watchdog,
            pdo_offset,
            pdo_count: u32::try_from(pdos.len()).unwrap_or(u32::MAX),
        });
    }
    Ok((sync_descs, pdo_descs, entry_descs, entry_info))
}

struct PdoChannelInfo {
    name: String,
    entry_index: u16,
    entry_subindex: u8,
    bit: u32,
}

fn parse_pdo_channels(
    device: &BackendDeviceConfig,
    object: &BTreeMap<String, Json>,
    entry_info: &EntryInfoMap,
) -> Result<Vec<PdoChannelInfo>> {
    let values = match object.get("pdo-channels") {
        None => &[][..],
        Some(value) => value.as_array().ok_or_else(|| {
            cfg_error(format!(
                "device `{}` `pdo-channels` is not an array",
                device.name
            ))
        })?,
    };
    let mut channels = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values {
        let value = value.as_object().ok_or_else(|| {
            cfg_error(format!(
                "device `{}` has a `pdo-channels` entry that is not an object",
                device.name
            ))
        })?;
        let name = value
            .get("name")
            .and_then(Json::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                cfg_error(format!(
                    "device `{}` has an unnamed PDO channel",
                    device.name
                ))
            })?
            .to_owned();
        if !seen.insert(name.clone()) {
            return Err(cfg_error(format!(
                "device `{}` maps channel `{name}` more than once",
                device.name
            )));
        }
        let entry = value
            .get("entry")
            .and_then(Json::as_object)
            .ok_or_else(|| {
                cfg_error(format!(
                    "device `{}` channel `{name}` has no `entry` object",
                    device.name
                ))
            })?;
        let entry_index = required_u16(entry, "index", device)?;
        let entry_subindex = required_u8(entry, "subindex", device, "pdo-channel entry subindex")?;
        if !entry_info.contains_key(&(entry_index, entry_subindex)) {
            return Err(cfg_error(format!(
                "device `{}` channel `{name}` names PDO entry `0x{entry_index:04x}:{entry_subindex}`, \
                 which is not declared",
                device.name
            )));
        }
        let bit = value
            .get("bit")
            .and_then(Json::as_number)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                cfg_error(format!(
                    "device `{}` channel `{name}` `bit` must be a non-negative integer",
                    device.name
                ))
            })?;
        channels.push(PdoChannelInfo {
            name,
            entry_index,
            entry_subindex,
            bit,
        });
    }
    Ok(channels)
}

fn parse_status_device(
    device: &BackendDeviceConfig,
    data_count: usize,
    output: &mut Vec<StatusChannel>,
) -> Result<()> {
    if let Some(text) = &device.config {
        let root = json::parse(text).map_err(|error| {
            cfg_error(format!(
                "status device `{}` configuration is not valid JSON: {}",
                device.name, error.message
            ))
        })?;
        let object = root.as_object().ok_or_else(|| {
            cfg_error(format!(
                "status device `{}` configuration is not a JSON object",
                device.name
            ))
        })?;
        if let Some(layout) = object.get("layout")
            && layout.as_number() != Some(1)
        {
            return Err(cfg_error(format!(
                "status device `{}` has an unknown layout; expected 1",
                device.name
            )));
        }
    }
    let mut names = BTreeSet::new();
    for channel in &device.channels {
        if !names.insert(channel.name.as_str()) {
            return Err(cfg_error(format!(
                "status device `{}` declares channel `{}` more than once",
                device.name, channel.name
            )));
        }
        if channel.direction != "input" {
            return Err(cfg_error(format!(
                "status device `{}` channel `{}` must be read-only (direction `input`)",
                device.name, channel.name
            )));
        }
        let Some(width) = io_mock::status_channel_width(&channel.name, data_count) else {
            return Err(cfg_error(format!(
                "status device `{}` names unknown status channel `{}`",
                device.name, channel.name
            )));
        };
        if channel.width_bits != width {
            return Err(cfg_error(format!(
                "status device `{}` channel `{}` is {} bits; the status layout uses {width} bits",
                device.name, channel.name, channel.width_bits
            )));
        }
        let field = parse_status_field(&channel.name, data_count).ok_or_else(|| {
            cfg_error(format!(
                "status device `{}` names unknown status channel `{}`",
                device.name, channel.name
            ))
        })?;
        if let Some(range) = channel.image {
            if range.width_bits != u64::from(channel.width_bits) {
                return Err(binding_error(format!(
                    "status channel `{}` on device `{}` is bound as {} bits but declares {} bits",
                    channel.name, device.name, range.width_bits, channel.width_bits
                )));
            }
            if range.area != 1 {
                return Err(binding_error(format!(
                    "status channel `{}` on device `{}` is bound to image area {}; status channels \
                     are read-only inputs",
                    channel.name, device.name, range.area
                )));
            }
            output.push(StatusChannel { field, range });
        }
    }
    Ok(())
}

fn parse_status_field(name: &str, data_count: usize) -> Option<StatusField> {
    let field = match name {
        "master.phase" => StatusField::MasterPhase,
        "master.active" => StatusField::MasterActive,
        "master.slaves_responding" => StatusField::MasterSlavesResponding,
        "master.link_up" => StatusField::MasterLinkUp,
        "master.last_error" => StatusField::MasterLastError,
        "master.recoveries" => StatusField::MasterRecoveries,
        "domain.wc_state" => StatusField::DomainWcState,
        "domain.working_counter" => StatusField::DomainWorkingCounter,
        "input.stale" => StatusField::InputStale,
        "cycle.exchanges" => StatusField::CycleExchanges,
        "cycle.faults" => StatusField::CycleFaults,
        _ => {
            let rest = name.strip_prefix("slave")?;
            let (index, field) = rest.split_once('.')?;
            let index = index.parse::<usize>().ok()?;
            if index >= data_count {
                return None;
            }
            match field {
                "al_state" => StatusField::SlaveAlState(index),
                "online" => StatusField::SlaveOnline(index),
                "operational" => StatusField::SlaveOperational(index),
                _ => return None,
            }
        }
    };
    Some(field)
}

fn validate_bindings(config: &BackendConfig) -> Result<()> {
    let mut devices: BTreeMap<&str, &BackendDeviceConfig> = BTreeMap::new();
    for device in &config.devices {
        if devices.insert(device.name.as_str(), device).is_some() {
            return Err(cfg_error(format!(
                "EtherCAT backend owns device `{}` more than once",
                device.name
            )));
        }
    }
    let mut seen = BTreeSet::new();
    for binding in &config.bindings {
        let Some(device) = devices.get(binding.device.as_str()) else {
            return Err(binding_error(format!(
                "binding names device `{}`, which the instance does not own",
                binding.device
            )));
        };
        let Some(channel) = device
            .channels
            .iter()
            .find(|channel| channel.name == binding.channel)
        else {
            return Err(binding_error(format!(
                "binding names channel `{}` on device `{}`, which is not declared",
                binding.channel, binding.device
            )));
        };
        if !seen.insert((binding.device.clone(), binding.channel.clone())) {
            return Err(binding_error(format!(
                "channel `{}` on device `{}` is bound more than once",
                binding.channel, binding.device
            )));
        }
        if channel.image != Some(binding.image) {
            return Err(binding_error(format!(
                "binding for channel `{}` on device `{}` disagrees with the channel image range",
                binding.channel, binding.device
            )));
        }
        if binding.direction != channel.direction {
            return Err(binding_error(format!(
                "binding for channel `{}` on device `{}` has direction `{}`, but the channel has \
                 `{}`",
                binding.channel, binding.device, binding.direction, channel.direction
            )));
        }
    }
    Ok(())
}

fn validate_layout_ranges(config: &BackendConfig) -> Result<()> {
    let check = |label: &str, image: ImageRange| -> Result<()> {
        let Some((_, length)) = config.layout.area(image.area) else {
            return Err(binding_error(format!(
                "{label} is bound to unknown image area {}",
                image.area
            )));
        };
        let limit = length
            .checked_mul(8)
            .ok_or_else(|| binding_error(format!("{label} image area {length} bytes overflows")))?;
        let end = image
            .offset_bits
            .checked_add(image.width_bits)
            .ok_or_else(|| binding_error(format!("{label} image range overflows")))?;
        if end > limit {
            return Err(binding_error(format!(
                "{label} image range {}+{} bits leaves its {length}-byte area",
                image.offset_bits, image.width_bits
            )));
        }
        Ok(())
    };
    for device in &config.devices {
        for channel in &device.channels {
            if let Some(image) = channel.image {
                check(
                    &format!("channel `{}` on device `{}`", channel.name, device.name),
                    image,
                )?;
            }
        }
    }
    for binding in &config.bindings {
        check(
            &format!(
                "binding for channel `{}` on device `{}`",
                binding.channel, binding.device
            ),
            binding.image,
        )?;
    }
    Ok(())
}

fn validate_channel_image(
    device: &BackendDeviceConfig,
    channel: &BackendChannel,
    direction: u8,
    image: ImageRange,
) -> Result<()> {
    if image.width_bits != u64::from(channel.width_bits) {
        return Err(binding_error(format!(
            "channel `{}` on device `{}` is bound as {} bits but declares {} bits",
            channel.name, device.name, image.width_bits, channel.width_bits
        )));
    }
    if image.width_bits > 1 && !image.offset_bits.is_multiple_of(8) {
        return Err(binding_error(format!(
            "channel `{}` on device `{}` is not byte-aligned in the process image",
            channel.name, device.name
        )));
    }
    if direction == DIR_INPUT && image.area != 1 {
        return Err(binding_error(format!(
            "input channel `{}` on device `{}` is bound to image area {}",
            channel.name, device.name, image.area
        )));
    }
    if direction == DIR_OUTPUT && image.area != 1 && image.area != 2 {
        return Err(binding_error(format!(
            "output channel `{}` on device `{}` is bound to image area {}",
            channel.name, device.name, image.area
        )));
    }
    Ok(())
}

fn parse_dc(
    device: &BackendDeviceConfig,
    object: &BTreeMap<String, Json>,
) -> Result<Option<DcConfig>> {
    let Some(value) = object.get("dc") else {
        return Ok(None);
    };
    let dc = value.as_object().ok_or_else(|| {
        cfg_error(format!(
            "device `{}` `dc` must be a table/object",
            device.name
        ))
    })?;
    let Some(assign) = dc.get("assign-activate") else {
        // The profile contract says a DC table without `assign-activate`
        // leaves the device in free-run and warns (W-ETH-002). The runtime
        // has no warning channel for backend configuration, so it behaves as
        // free-run here.
        return Ok(None);
    };
    let assign_activate = match assign {
        Json::Number(number) => u16::try_from(*number).map_err(|_| {
            cfg_error(format!(
                "device `{}` `dc.assign-activate` must be an integer in 0..=65535",
                device.name
            ))
        })?,
        _ => {
            return Err(cfg_error(format!(
                "device `{}` `dc.assign-activate` must be an integer in 0..=65535 (the vendor \
                 AssignActivate word); boolean shorthand is not accepted",
                device.name
            )));
        }
    };
    let sync0_cycle = optional_u32(dc, "sync0-cycle", device)?.unwrap_or(0);
    let sync0_shift = optional_i32(dc, "sync0-shift", device)?.unwrap_or(0);
    let sync1_cycle = optional_u32(dc, "sync1-cycle", device)?.unwrap_or(0);
    let sync1_shift = optional_i32(dc, "sync1-shift", device)?.unwrap_or(0);
    Ok(Some(DcConfig {
        assign_activate,
        sync0_cycle,
        sync0_shift,
        sync1_cycle,
        sync1_shift,
    }))
}

fn parse_direction(text: &str) -> Option<u8> {
    match text {
        "input" => Some(DIR_INPUT),
        "output" => Some(DIR_OUTPUT),
        _ => None,
    }
}

fn required_identity(
    object: &BTreeMap<String, Json>,
    key: &str,
    device: &BackendDeviceConfig,
) -> Result<u32> {
    object.get(key).and_then(identity_value).ok_or_else(|| {
        cfg_error(format!(
            "device `{}` `{key}` must be an integer or a hexadecimal string",
            device.name
        ))
    })
}

fn optional_identity(
    object: &BTreeMap<String, Json>,
    key: &str,
    device: &BackendDeviceConfig,
) -> Result<Option<u32>> {
    match object.get(key) {
        None => Ok(None),
        Some(value) => identity_value(value).map(Some).ok_or_else(|| {
            cfg_error(format!(
                "device `{}` `{key}` must be an integer or a hexadecimal string",
                device.name
            ))
        }),
    }
}

fn identity_value(value: &Json) -> Option<u32> {
    if let Some(number) = value.as_number() {
        return u32::try_from(number).ok();
    }
    let text = value.as_str()?.trim();
    let (digits, radix) = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(digits) => (digits, 16),
        None => (text, 10),
    };
    if digits.is_empty() {
        return None;
    }
    u32::from_str_radix(digits, radix).ok()
}

fn optional_u16(
    object: &BTreeMap<String, Json>,
    key: &str,
    device: &BackendDeviceConfig,
) -> Result<Option<u16>> {
    match object.get(key) {
        None => Ok(None),
        Some(value) => identity_value(value)
            .and_then(|value| u16::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| {
                cfg_error(format!(
                    "device `{}` `{key}` must be an integer in 0..=65535",
                    device.name
                ))
            }),
    }
}

fn required_u16(
    object: &BTreeMap<String, Json>,
    key: &str,
    device: &BackendDeviceConfig,
) -> Result<u16> {
    optional_u16(object, key, device)?.ok_or_else(|| {
        cfg_error(format!(
            "device `{}` is missing required key `{key}`",
            device.name
        ))
    })
}

fn required_u8(
    object: &BTreeMap<String, Json>,
    key: &str,
    device: &BackendDeviceConfig,
    what: &str,
) -> Result<u8> {
    object
        .get(key)
        .and_then(Json::as_number)
        .and_then(|value| u8::try_from(value).ok())
        .ok_or_else(|| {
            cfg_error(format!(
                "device `{}` {what} `{key}` must be an integer in 0..=255",
                device.name
            ))
        })
}

fn optional_u32(
    object: &BTreeMap<String, Json>,
    key: &str,
    device: &BackendDeviceConfig,
) -> Result<Option<u32>> {
    match object.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_number()
            .and_then(|value| u32::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| {
                cfg_error(format!(
                    "device `{}` `{key}` must be a non-negative integer",
                    device.name
                ))
            }),
    }
}

fn optional_i32(
    object: &BTreeMap<String, Json>,
    key: &str,
    device: &BackendDeviceConfig,
) -> Result<Option<i32>> {
    match object.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_number()
            .and_then(|value| i32::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| {
                cfg_error(format!(
                    "device `{}` `{key}` must be a 32-bit integer",
                    device.name
                ))
            }),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    const DATA_CONFIG: &str = r#"{
        "master": 0,
        "alias": 0,
        "position": 1,
        "vendor": "0x2",
        "product": 3,
        "revision": 4,
        "startup-timeout": "0ns",
        "recovery-cycles": 2,
        "syncs": [
            {
                "index": 1,
                "direction": "input",
                "watchdog": "disable",
                "pdos": [{
                    "index": "0x1a00",
                    "entries": [
                        {"index": "0x3001", "subindex": 1, "bits": 16}
                    ]
                }]
            },
            {
                "index": 2,
                "direction": "output",
                "watchdog": "enable",
                "pdos": [{
                    "index": "0x1600",
                    "entries": [
                        {"index": "0x3101", "subindex": 1, "bits": 8}
                    ]
                }]
            }
        ],
        "pdo-channels": [
            {"name": "IN", "entry": {"index": "0x3001", "subindex": 1}, "bit": 0},
            {"name": "OUT", "entry": {"index": "0x3101", "subindex": 1}, "bit": 0}
        ]
    }"#;

    #[derive(Default)]
    struct FakeShared {
        calls: Vec<String>,
        healthy: bool,
        input_bytes: Vec<u8>,
        outputs: Vec<u8>,
        entry_offsets: BTreeMap<(u16, u8), (i32, u32)>,
        failures: BTreeMap<String, i32>,
        master: MasterStatus,
        domain: DomainStatus,
        slaves: Vec<SlaveStatus>,
        closed: bool,
        application_time: Option<u64>,
        synced: bool,
    }

    struct FakeShim {
        domain: Vec<u8>,
        shared: Arc<Mutex<FakeShared>>,
    }

    impl FakeShim {
        fn new(domain_size: usize, data_devices: usize) -> Self {
            let shared = FakeShared {
                healthy: true,
                input_bytes: vec![0; domain_size],
                outputs: vec![0; domain_size],
                master: MasterStatus {
                    slaves_responding: u32::try_from(data_devices).unwrap_or(0),
                    link_up: true,
                },
                domain: DomainStatus {
                    working_counter: 1,
                    wc_state: WC_COMPLETE,
                },
                slaves: vec![
                    SlaveStatus {
                        al_state: 8,
                        online: true,
                        operational: true,
                    };
                    data_devices
                ],
                ..FakeShared::default()
            };
            Self {
                domain: vec![0; domain_size],
                shared: Arc::new(Mutex::new(shared)),
            }
        }

        fn shared(&self) -> Arc<Mutex<FakeShared>> {
            Arc::clone(&self.shared)
        }

        fn record(&self, call: &str) {
            if let Ok(mut shared) = self.shared.lock() {
                shared.calls.push(call.to_owned());
            }
        }

        fn failure(&self, call: &str) -> Option<ShimCallError> {
            let shared = self.shared.lock().ok()?;
            shared
                .failures
                .get(call)
                .copied()
                .map(|code| ShimCallError {
                    code,
                    message: format!("injected {call} failure"),
                })
        }

        fn healthy(&self) -> bool {
            self.shared
                .lock()
                .map(|shared| shared.healthy)
                .unwrap_or(false)
        }
    }

    impl EcShim for FakeShim {
        fn master_open(&mut self, _master_index: u16) -> ShimResult<*mut c_void> {
            self.record("master_open");
            if let Some(error) = self.failure("master_open") {
                return Err(error);
            }
            Ok(std::ptr::dangling_mut::<c_void>())
        }

        fn master_close(&mut self, _master: *mut c_void) -> ShimResult<()> {
            self.record("master_close");
            if let Ok(mut shared) = self.shared.lock() {
                shared.closed = true;
            }
            Ok(())
        }

        fn domain_create(&mut self, _master: *mut c_void) -> ShimResult<()> {
            self.record("domain_create");
            self.failure("domain_create").map_or(Ok(()), Err)
        }

        fn slave_configure(
            &mut self,
            _master: *mut c_void,
            slave: u32,
            desc: &SlaveDesc,
        ) -> ShimResult<()> {
            self.record(&format!(
                "slave_configure:{slave}:{}:{}:{}:{}:{}",
                desc.alias, desc.position, desc.vendor_id, desc.product_code, desc.check_revision
            ));
            self.failure("slave_configure").map_or(Ok(()), Err)
        }

        fn slave_configure_pdos(
            &mut self,
            _master: *mut c_void,
            slave: u32,
            syncs: &[SyncDesc],
            pdos: &[PdoDesc],
            entries: &[EntryDesc],
        ) -> ShimResult<()> {
            self.record(&format!(
                "slave_configure_pdos:{slave}:{}:{}:{}",
                syncs.len(),
                pdos.len(),
                entries.len()
            ));
            self.failure("slave_configure_pdos").map_or(Ok(()), Err)
        }

        fn slave_configure_dc(
            &mut self,
            _master: *mut c_void,
            slave: u32,
            dc: &DcConfig,
        ) -> ShimResult<()> {
            self.record(&format!(
                "slave_configure_dc:{slave}:{}:{}:{}",
                dc.assign_activate, dc.sync0_cycle, dc.sync0_shift
            ));
            self.failure("slave_configure_dc").map_or(Ok(()), Err)
        }

        fn entry_register(
            &mut self,
            _master: *mut c_void,
            slave: u32,
            index: u16,
            subindex: u8,
        ) -> ShimResult<(i32, u32)> {
            self.record(&format!("entry_register:{slave}:0x{index:04x}:{subindex}"));
            if let Some(error) = self.failure("entry_register") {
                return Err(error);
            }
            self.shared
                .lock()
                .ok()
                .and_then(|shared| shared.entry_offsets.get(&(index, subindex)).copied())
                .ok_or_else(|| ShimCallError {
                    code: 4,
                    message: format!("no fake offset for entry 0x{index:04x}:{subindex}"),
                })
        }

        fn master_activate(&mut self, _master: *mut c_void) -> ShimResult<()> {
            self.record("master_activate");
            self.failure("master_activate").map_or(Ok(()), Err)
        }

        fn domain_data(&mut self, _master: *mut c_void) -> ShimResult<(*mut u8, usize)> {
            self.record("domain_data");
            if let Some(error) = self.failure("domain_data") {
                return Err(error);
            }
            Ok((self.domain.as_mut_ptr(), self.domain.len()))
        }

        fn master_status(&mut self, _master: *mut c_void) -> ShimResult<MasterStatus> {
            self.record("master_status");
            if let Some(error) = self.failure("master_status") {
                return Err(error);
            }
            Ok(self.shared.lock().expect("fake status").master)
        }

        fn domain_status(&mut self, _master: *mut c_void) -> ShimResult<DomainStatus> {
            self.record("domain_status");
            if let Some(error) = self.failure("domain_status") {
                return Err(error);
            }
            Ok(self.shared.lock().expect("fake status").domain)
        }

        fn slave_status(&mut self, _master: *mut c_void, slave: u32) -> ShimResult<SlaveStatus> {
            self.record(&format!("slave_status:{slave}"));
            if let Some(error) = self.failure("slave_status") {
                return Err(error);
            }
            let shared = self.shared.lock().expect("fake status");
            shared
                .slaves
                .get(usize::try_from(slave).unwrap_or(usize::MAX))
                .copied()
                .ok_or_else(|| ShimCallError {
                    code: 8,
                    message: "fake slave out of range".into(),
                })
        }

        fn receive(&mut self, _master: *mut c_void) -> ShimResult<()> {
            self.record("receive");
            if let Some(error) = self.failure("receive") {
                return Err(error);
            }
            if self.healthy() {
                let shared = self.shared.lock().expect("fake input");
                let count = self.domain.len().min(shared.input_bytes.len());
                self.domain[..count].copy_from_slice(&shared.input_bytes[..count]);
            }
            Ok(())
        }

        fn domain_process(&mut self, _master: *mut c_void) -> ShimResult<()> {
            self.record("domain_process");
            self.failure("domain_process").map_or(Ok(()), Err)
        }

        fn domain_queue(&mut self, _master: *mut c_void) -> ShimResult<()> {
            self.record("domain_queue");
            if let Some(error) = self.failure("domain_queue") {
                return Err(error);
            }
            if let Ok(mut shared) = self.shared.lock() {
                shared.outputs = self.domain.clone();
            }
            Ok(())
        }

        fn send(&mut self, _master: *mut c_void) -> ShimResult<()> {
            self.record("send");
            self.failure("send").map_or(Ok(()), Err)
        }

        fn application_time(&mut self, _master: *mut c_void, app_time: u64) -> ShimResult<()> {
            self.record("application_time");
            if let Ok(mut shared) = self.shared.lock() {
                shared.application_time = Some(app_time);
            }
            self.failure("application_time").map_or(Ok(()), Err)
        }

        fn sync_clocks(&mut self, _master: *mut c_void) -> ShimResult<()> {
            self.record("sync_clocks");
            if let Ok(mut shared) = self.shared.lock() {
                shared.synced = true;
            }
            self.failure("sync_clocks").map_or(Ok(()), Err)
        }
    }

    fn bound(
        name: &str,
        direction: &str,
        width_bits: u32,
        area: u8,
        offset_bits: u64,
    ) -> BackendChannel {
        BackendChannel {
            name: name.into(),
            direction: direction.into(),
            width_bits,
            image: Some(ImageRange {
                area,
                offset_bits,
                width_bits: u64::from(width_bits),
            }),
        }
    }

    fn unbound(name: &str, direction: &str, width_bits: u32) -> BackendChannel {
        BackendChannel {
            name: name.into(),
            direction: direction.into(),
            width_bits,
            image: None,
        }
    }

    fn device(
        name: &str,
        kind: &str,
        config: Option<&str>,
        channels: Vec<BackendChannel>,
    ) -> BackendDeviceConfig {
        BackendDeviceConfig {
            name: name.into(),
            kind: kind.into(),
            config: config.map(str::to_owned),
            channels,
        }
    }

    fn binding(
        device: &str,
        channel: &str,
        direction: &str,
        area: u8,
        offset_bits: u64,
        width_bits: u64,
    ) -> crate::coordinator::BackendBinding {
        crate::coordinator::BackendBinding {
            device: device.into(),
            channel: channel.into(),
            image: ImageRange {
                area,
                offset_bits,
                width_bits,
            },
            direction: direction.into(),
        }
    }

    fn config(
        devices: Vec<BackendDeviceConfig>,
        bindings: Vec<crate::coordinator::BackendBinding>,
        inputs_bytes: u64,
        outputs_bytes: u64,
    ) -> BackendConfig {
        BackendConfig {
            name: "ethercat".into(),
            devices,
            bindings,
            layout: crate::module_info::ImageLayout {
                inputs_bytes,
                outputs_bytes,
                markers_bytes: 0,
            },
            profile: None,
        }
    }

    fn data_channels() -> Vec<BackendChannel> {
        vec![
            bound("IN", "input", 8, 1, 0),
            bound("OUT", "output", 8, 2, 0),
        ]
    }

    fn data_bindings() -> Vec<crate::coordinator::BackendBinding> {
        vec![
            binding("diewu", "IN", "input", 1, 0, 8),
            binding("diewu", "OUT", "output", 2, 0, 8),
        ]
    }

    fn fake_backend(domain_size: usize) -> (EthercatBackend, Arc<Mutex<FakeShared>>) {
        let shim = FakeShim::new(domain_size, 1);
        if let Ok(mut shared) = shim.shared.lock() {
            shared.entry_offsets.insert((0x3001, 1), (0, 0));
            shared.entry_offsets.insert((0x3101, 1), (1, 0));
        }
        let shared = shim.shared();
        (EthercatBackend::with_shim(Box::new(shim)), shared)
    }

    fn basic_config() -> BackendConfig {
        config(
            vec![device(
                "diewu",
                "ethercat",
                Some(DATA_CONFIG),
                data_channels(),
            )],
            data_bindings(),
            1,
            1,
        )
    }

    fn start_basic() -> (EthercatBackend, Arc<Mutex<FakeShared>>) {
        let (mut backend, shared) = fake_backend(2);
        backend.cold_start(&basic_config()).expect("cold start");
        (backend, shared)
    }

    #[test]
    fn shim_path_defaults_to_the_bare_loader_name() {
        // Do not mutate the process environment here; the default is checked
        // through the constant and the resolver in an isolated helper.
        assert_eq!(SHIM_LIBRARY, "libstc_io_ethercat.so");
        assert_eq!(SHIM_ENV, "STC_ETHERCAT_LIBRARY");
    }

    #[test]
    fn abi_handshake_rejects_version_and_name_mismatches() {
        assert!(verify_handshake(SHIM_ABI_VERSION, SHIM_ABI_NAME).is_ok());
        let version = verify_handshake(SHIM_ABI_VERSION + 1, SHIM_ABI_NAME)
            .expect_err("wrong version is rejected");
        assert!(version.message.contains("ABI version"));
        let name =
            verify_handshake(SHIM_ABI_VERSION, "other/1").expect_err("wrong name is rejected");
        assert!(name.message.contains("ABI name"));
    }

    #[test]
    fn missing_shim_reports_eth_001() {
        let error = probe_shim(Path::new("/nonexistent/stc/libstc_io_ethercat.so"))
            .expect_err("missing shim");
        assert_eq!(error.code, codes::ETH_SHIM);
        assert!(error.message.contains("cannot load EtherCAT shim"));
    }

    #[test]
    fn cold_start_configures_the_shim_in_ig_h_order() {
        let (mut backend, shared) = fake_backend(2);
        backend.cold_start(&basic_config()).expect("cold start");
        let calls = shared.lock().expect("calls").calls.clone();
        let prefix = &calls[..9];
        assert_eq!(prefix[0], "master_open");
        assert_eq!(prefix[1], "domain_create");
        assert!(
            prefix[2].starts_with("slave_configure:0:0:1:2:3:true"),
            "unexpected slave descriptor: {}",
            prefix[2]
        );
        assert_eq!(prefix[3], "slave_configure_pdos:0:2:2:2");
        assert_eq!(prefix[4], "entry_register:0:0x3001:1");
        assert_eq!(prefix[5], "entry_register:0:0x3101:1");
        assert_eq!(prefix[6], "master_activate");
        assert_eq!(prefix[7], "domain_data");
        assert_eq!(prefix[8], "receive");
        assert_eq!(backend.status().state, BackendState::Ready);
        assert_eq!(backend.status().devices, 1);
        backend.shutdown().expect("shutdown");
        assert!(shared.lock().expect("closed").closed);
        assert_eq!(backend.status().state, BackendState::Stopped);
    }

    #[test]
    fn latch_moves_inputs_and_publish_moves_outputs() {
        let (mut backend, shared) = start_basic();
        shared.lock().expect("input").input_bytes[0] = 0x5A;
        let mut image = ProcessImage::new(basic_config().layout);
        backend.latch_inputs(&mut image).expect("latch");
        assert_eq!(image.read_bits(1, 0, 8), Some(0x5A));
        assert_eq!(backend.status().state, BackendState::Running);

        assert!(image.write_bits(2, 0, 8, 0x77));
        backend.publish_outputs(&image).expect("publish");
        assert_eq!(shared.lock().expect("outputs").outputs[1], 0x77);
        assert_eq!(backend.status().exchanges, 1);
    }

    #[test]
    fn status_device_is_filled_from_master_domain_and_slave_state() {
        let status = device(
            "status",
            "ethercat-status",
            Some(r#"{"layout": 1}"#),
            vec![
                bound("master.phase", "input", 16, 1, 64),
                bound("master.active", "input", 1, 1, 80),
                bound("master.slaves_responding", "input", 16, 1, 96),
                bound("master.link_up", "input", 1, 1, 112),
                bound("master.last_error", "input", 32, 1, 128),
                bound("domain.wc_state", "input", 16, 1, 160),
                bound("domain.working_counter", "input", 16, 1, 176),
                bound("slave0.al_state", "input", 16, 1, 192),
                bound("slave0.online", "input", 1, 1, 208),
                bound("slave0.operational", "input", 1, 1, 224),
                bound("input.stale", "input", 1, 1, 240),
                bound("cycle.exchanges", "input", 32, 1, 256),
                bound("cycle.faults", "input", 32, 1, 288),
            ],
        );
        let config = config(
            vec![
                device("diewu", "ethercat", Some(DATA_CONFIG), data_channels()),
                status,
            ],
            data_bindings()
                .into_iter()
                .chain([
                    binding("status", "master.phase", "input", 1, 64, 16),
                    binding("status", "master.active", "input", 1, 80, 1),
                    binding("status", "master.slaves_responding", "input", 1, 96, 16),
                    binding("status", "master.link_up", "input", 1, 112, 1),
                    binding("status", "master.last_error", "input", 1, 128, 32),
                    binding("status", "domain.wc_state", "input", 1, 160, 16),
                    binding("status", "domain.working_counter", "input", 1, 176, 16),
                    binding("status", "slave0.al_state", "input", 1, 192, 16),
                    binding("status", "slave0.online", "input", 1, 208, 1),
                    binding("status", "slave0.operational", "input", 1, 224, 1),
                    binding("status", "input.stale", "input", 1, 240, 1),
                    binding("status", "cycle.exchanges", "input", 1, 256, 32),
                    binding("status", "cycle.faults", "input", 1, 288, 32),
                ])
                .collect(),
            40,
            1,
        );
        let shim = FakeShim::new(2, 1);
        if let Ok(mut shared) = shim.shared.lock() {
            shared.entry_offsets.insert((0x3001, 1), (0, 0));
            shared.entry_offsets.insert((0x3101, 1), (1, 0));
            shared.master.slaves_responding = 1;
            shared.master.link_up = true;
            shared.domain.wc_state = WC_COMPLETE;
            shared.domain.working_counter = 7;
            shared.slaves[0].al_state = 8;
        }
        let mut backend = EthercatBackend::with_shim(Box::new(shim));
        backend.cold_start(&config).expect("cold start");
        let mut image = ProcessImage::new(config.layout);
        backend.latch_inputs(&mut image).expect("latch");
        assert_eq!(image.read_bits(1, 64, 16), Some(MASTER_PHASE_OPERATION));
        assert_eq!(image.read_bits(1, 80, 1), Some(1));
        assert_eq!(image.read_bits(1, 96, 16), Some(1));
        assert_eq!(image.read_bits(1, 112, 1), Some(1));
        assert_eq!(image.read_bits(1, 160, 16), Some(u64::from(WC_COMPLETE)));
        assert_eq!(image.read_bits(1, 176, 16), Some(7));
        assert_eq!(image.read_bits(1, 192, 16), Some(8));
        assert_eq!(image.read_bits(1, 208, 1), Some(1));
        assert_eq!(image.read_bits(1, 224, 1), Some(1));
        assert_eq!(image.read_bits(1, 240, 1), Some(0));
        assert_eq!(image.read_bits(1, 256, 32), Some(0));
        assert_eq!(image.read_bits(1, 288, 32), Some(0));
    }

    #[test]
    fn fault_keeps_last_inputs_stale_and_recovers_after_the_window() {
        let (mut backend, shared) = start_basic();
        let mut image = ProcessImage::new(basic_config().layout);
        shared.lock().expect("input").input_bytes[0] = 0x11;
        backend.latch_inputs(&mut image).expect("healthy latch");
        assert_eq!(image.read_bits(1, 0, 8), Some(0x11));

        shared.lock().expect("input").input_bytes[0] = 0x22;
        backend_shim_set_healthy(&shared, false);
        backend
            .latch_inputs(&mut image)
            .expect("faulted latch is non-fatal");
        assert_eq!(image.read_bits(1, 0, 8), Some(0x11));
        assert_eq!(backend.status().state, BackendState::Faulted);
        assert_eq!(backend.status().faults, 1);
        assert!(
            backend
                .status()
                .last_fault
                .as_deref()
                .is_some_and(|fault| fault.starts_with(codes::W_ETHERCAT_FAULT))
        );

        backend_shim_set_healthy(&shared, true);
        shared.lock().expect("input").input_bytes[0] = 0x33;
        backend.latch_inputs(&mut image).expect("recovering latch");
        assert_eq!(image.read_bits(1, 0, 8), Some(0x11));
        assert_eq!(backend.status().state, BackendState::Recovering);
        backend.latch_inputs(&mut image).expect("recovered latch");
        assert_eq!(image.read_bits(1, 0, 8), Some(0x33));
        assert_eq!(backend.status().state, BackendState::Running);
        assert_eq!(backend.status().faults, 1);
    }

    fn backend_shim_set_healthy(shared: &Arc<Mutex<FakeShared>>, healthy: bool) {
        if let Ok(mut shared) = shared.lock() {
            shared.healthy = healthy;
            shared.domain.wc_state = if healthy { WC_COMPLETE } else { 1 };
            for slave in &mut shared.slaves {
                slave.online = healthy;
                slave.operational = healthy;
            }
        }
    }

    #[test]
    fn faulted_publish_still_sends_plc_outputs_unchanged() {
        let (mut backend, shared) = start_basic();
        let mut image = ProcessImage::new(basic_config().layout);
        backend_shim_set_healthy(&shared, false);
        backend.latch_inputs(&mut image).expect("faulted latch");
        assert!(image.write_bits(2, 0, 8, 0x44));
        backend
            .publish_outputs(&image)
            .expect("publish during fault");
        assert_eq!(shared.lock().expect("outputs").outputs[1], 0x44);
        assert_eq!(backend.status().exchanges, 1);
        assert_eq!(backend.status().state, BackendState::Faulted);
    }

    #[test]
    fn startup_timeout_is_eth_008_and_opens_no_run() {
        let (mut backend, shared) = fake_backend(2);
        backend_shim_set_healthy(&shared, false);
        let error = backend
            .cold_start(&basic_config())
            .expect_err("unhealthy startup");
        assert_eq!(error.code, codes::ETH_STARTUP);
        assert!(shared.lock().expect("close").closed);
    }

    #[test]
    fn shim_operation_failures_map_to_distinct_codes() {
        let cases = [
            ("master_open", codes::ETH_MASTER),
            ("domain_create", codes::ETH_ACTIVATE),
            ("slave_configure", codes::ETH_SLAVE),
            ("slave_configure_pdos", codes::ETH_PDO),
            ("entry_register", codes::ETH_PDO),
            ("master_activate", codes::ETH_ACTIVATE),
            ("domain_data", codes::ETH_ACTIVATE),
        ];
        for (operation, expected) in cases {
            let (mut backend, shared) = fake_backend(2);
            shared
                .lock()
                .expect("failures")
                .failures
                .insert(operation.to_owned(), 8);
            let error = backend
                .cold_start(&basic_config())
                .expect_err("injected shim failure");
            assert_eq!(
                error.code, expected,
                "operation {operation}: {}",
                error.message
            );
        }
    }

    #[test]
    fn invalid_configuration_uses_eth_006() {
        let (mut backend, _shared) = fake_backend(2);
        let error = backend
            .cold_start(&config(
                vec![device("diewu", "ethercat", None, data_channels())],
                Vec::new(),
                1,
                1,
            ))
            .expect_err("missing configuration");
        assert_eq!(error.code, codes::ETH_CONFIG);

        let status_only = config(
            vec![device(
                "status",
                "ethercat-status",
                Some(r#"{"layout": 1}"#),
                vec![bound("master.phase", "input", 16, 1, 0)],
            )],
            Vec::new(),
            2,
            0,
        );
        let error = backend.cold_start(&status_only).expect_err("status only");
        assert_eq!(error.code, codes::ETH_CONFIG);

        let mismatched_master = DATA_CONFIG.replace("\"master\": 0", "\"master\": 1");
        let error = backend
            .cold_start(&config(
                vec![
                    device("one", "ethercat", Some(DATA_CONFIG), data_channels()),
                    device("two", "ethercat", Some(&mismatched_master), Vec::new()),
                ],
                Vec::new(),
                1,
                1,
            ))
            .expect_err("master mismatch");
        assert_eq!(error.code, codes::ETH_CONFIG);

        let error = backend
            .cold_start(&config(
                vec![
                    device("one", "ethercat", Some(DATA_CONFIG), data_channels()),
                    device("two", "ethercat", Some(DATA_CONFIG), data_channels()),
                ],
                Vec::new(),
                1,
                1,
            ))
            .expect_err("duplicate slave position");
        assert_eq!(error.code, codes::ETH_CONFIG);
        assert!(
            error.message.contains("slave position"),
            "{}",
            error.message
        );
    }

    #[test]
    fn removed_frame_counters_are_rejected_as_status_channels() {
        let (mut backend, _shared) = fake_backend(2);
        for (name, width) in [
            ("master.lost_frames", 32),
            ("master.tx_errors", 32),
            ("master.frame_loss_permille", 16),
        ] {
            let error = backend
                .cold_start(&config(
                    vec![
                        device("diewu", "ethercat", Some(DATA_CONFIG), data_channels()),
                        device(
                            "status",
                            "ethercat-status",
                            Some(r#"{"layout": 1}"#),
                            vec![bound(name, "input", width, 1, 0)],
                        ),
                    ],
                    Vec::new(),
                    4,
                    1,
                ))
                .expect_err("removed frame counter");
            assert_eq!(error.code, codes::ETH_CONFIG);
            assert!(
                error.message.contains("unknown status channel"),
                "{name}: {}",
                error.message
            );
        }
    }

    #[test]
    fn binding_disagreement_uses_eth_007() {
        let (mut backend, _shared) = fake_backend(2);
        let mut config = basic_config();
        config.bindings[0] = binding("diewu", "IN", "input", 1, 8, 8);
        let error = backend.cold_start(&config).expect_err("binding mismatch");
        assert_eq!(error.code, codes::ETH_BINDING);
    }

    #[test]
    fn unbound_channels_are_still_configured_and_register_entries() {
        let (mut backend, shared) = fake_backend(2);
        let config = config(
            vec![device(
                "diewu",
                "ethercat",
                Some(DATA_CONFIG),
                vec![unbound("IN", "input", 8), unbound("OUT", "output", 8)],
            )],
            Vec::new(),
            1,
            1,
        );
        backend.cold_start(&config).expect("unbound cold start");
        let calls = shared.lock().expect("calls").calls.clone();
        assert!(calls.iter().any(|call| call == "entry_register:0:0x3001:1"));
        assert!(calls.iter().any(|call| call == "entry_register:0:0x3101:1"));
        let mut image = ProcessImage::new(config.layout);
        backend.latch_inputs(&mut image).expect("latch");
        assert_eq!(image.read_bits(1, 0, 8), Some(0));
    }

    #[test]
    fn dc_boolean_is_rejected_but_numeric_word_is_configured() {
        let (mut backend, _shared) = fake_backend(2);
        let boolean = DATA_CONFIG.replace(
            "\"recovery-cycles\": 2,",
            "\"recovery-cycles\": 2, \"dc\": {\"assign-activate\": true, \"sync0-cycle\": 1000000},",
        );
        let error = backend
            .cold_start(&config(
                vec![device("diewu", "ethercat", Some(&boolean), data_channels())],
                data_bindings(),
                1,
                1,
            ))
            .expect_err("boolean DC is ambiguous");
        assert_eq!(error.code, codes::ETH_CONFIG);
        assert!(error.message.contains("AssignActivate"));

        let out_of_range = DATA_CONFIG.replace(
            "\"recovery-cycles\": 2,",
            "\"recovery-cycles\": 2, \"dc\": {\"assign-activate\": 65536, \"sync0-cycle\": 1000000},",
        );
        let error = backend
            .cold_start(&config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(&out_of_range),
                    data_channels(),
                )],
                data_bindings(),
                1,
                1,
            ))
            .expect_err("out-of-range DC");
        assert_eq!(error.code, codes::ETH_CONFIG);
        assert!(error.message.contains("0..=65535"), "{}", error.message);

        let numeric = DATA_CONFIG.replace(
            "\"recovery-cycles\": 2,",
            "\"recovery-cycles\": 2, \"dc\": {\"assign-activate\": 768, \"sync0-cycle\": 1000000},",
        );
        let shim = FakeShim::new(2, 1);
        if let Ok(mut shared) = shim.shared.lock() {
            shared.entry_offsets.insert((0x3001, 1), (0, 0));
            shared.entry_offsets.insert((0x3101, 1), (1, 0));
        }
        let shared = shim.shared();
        let mut backend = EthercatBackend::with_shim(Box::new(shim));
        backend
            .cold_start(&config(
                vec![device("diewu", "ethercat", Some(&numeric), data_channels())],
                data_bindings(),
                1,
                1,
            ))
            .expect("numeric DC");
        let calls = shared.lock().expect("calls").calls.clone();
        assert!(
            calls
                .iter()
                .any(|call| call == "slave_configure_dc:0:768:1000000:0"),
            "DC configuration was not forwarded: {calls:?}"
        );
        assert!(
            calls.iter().any(|call| call == "application_time"),
            "DC cycles must supply the application time: {calls:?}"
        );
        assert!(
            calls.iter().any(|call| call == "sync_clocks"),
            "DC cycles must synchronize reference and slave clocks: {calls:?}"
        );
        assert!(shared.lock().expect("shared").application_time.is_some());
        assert!(shared.lock().expect("shared").synced);
    }

    #[test]
    fn domain_buffer_encoding_is_little_endian_bits() {
        let mut data = [0_u8; 2];
        assert!(domain_write(&mut data, 0, 8, 0xA5));
        assert_eq!(data[0], 0xA5);
        assert!(domain_write(&mut data, 12, 4, 0b1010));
        assert_eq!(domain_read(&data, 12, 4), Some(0b1010));
        assert_eq!(domain_read(&data, 0, 8), Some(0xA5));
        assert_eq!(domain_read(&data, 12, 5), None);
    }
}
