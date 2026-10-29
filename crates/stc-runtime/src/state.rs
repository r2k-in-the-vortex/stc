//! File-backed retained state (State-and-RETAIN §3–§4).
//!
//! The file is little-endian:
//!
//! ```text
//! 0   8  magic "STCRET01"
//! 8   4  format version (1)
//! 12  4  runtime ABI major (2)
//! 16  8  profile hash (FNV-1a of the profile ID)
//! 24  8  layout hash (FNV-1a of the canonical retained layout)
//! 32  8  payload length
//! 40  N  payload
//! 40+N 4 CRC32 (IEEE) over bytes 0..40+N
//! ```
//!
//! Load returns the ABI 2 `state_load` result codes: 0 OK, 1 not found, 2
//! layout/profile/version mismatch, 3 corrupt, 4 I/O error, 5 disabled. Save
//! writes `<file>.tmp` in the same directory with mode 0600 on Unix, flushes
//! and syncs it, replaces the target, then syncs the parent directory; a
//! failed replace leaves the previous file intact and reports
//! `STC-E-RUN-RETAIN-003`.
//!
//! The service configuration and outcomes are process-wide because the host
//! ABI service table is process-wide (the runtime is a single host process).

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::catalog::PROFILE;

/// State-file magic.
pub const MAGIC: &[u8; 8] = b"STCRET01";
/// State-file format version.
pub const FORMAT_VERSION: u32 = 1;
/// Runtime ABI major recorded in the header.
pub const STATE_ABI: u32 = crate::STC_RUNTIME_ABI as u32;
/// Fixed header length.
pub const HEADER_BYTES: usize = 40;
/// CRC trailer length.
pub const CRC_BYTES: usize = 4;
/// Largest state payload the runtime writes or reads.
pub const MAX_STATE_BYTES: u64 = 64 * 1024 * 1024;

/// `[retain].on-mismatch`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MismatchPolicy {
    /// Trap and do not run (`STC-E-RUN-RETAIN-001/002`).
    #[default]
    Error,
    /// Warn and keep defaults (`STC-W-RUN-RETAIN-002`).
    Reset,
}

impl MismatchPolicy {
    /// Parses the metadata spelling.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "error" => Some(Self::Error),
            "reset" => Some(Self::Reset),
            _ => None,
        }
    }

    /// Returns the metadata spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Reset => "reset",
        }
    }
}

/// How one state-file read ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadOutcome {
    /// The payload was copied into the caller's buffer.
    Loaded,
    /// The file does not exist (cold start).
    NotFound,
    /// Format version, ABI, profile hash, or layout hash differs.
    Mismatch(String),
    /// Magic, length, or CRC is wrong.
    Corrupt(String),
    /// The file could not be read.
    IoError(String),
}

/// A non-fatal notice produced by the state service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateNotice {
    /// Stable diagnostic code.
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
}

/// A fatal state-service failure that must stop the load or run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateFailure {
    /// Stable diagnostic code (`STC-E-RUN-RETAIN-001/002/003`).
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
}

/// Effective retained-state configuration for the process-wide service.
#[derive(Clone, Debug, Default)]
pub struct RetainConfig {
    /// Whether the service loads and saves at all.
    pub enabled: bool,
    /// Absolute state-file path.
    pub file: Option<PathBuf>,
    /// Mismatch policy.
    pub policy: MismatchPolicy,
    /// Cold start: ignore the existing file and overwrite on shutdown.
    pub reset: bool,
}

struct ServiceState {
    config: RetainConfig,
    /// Generation permitted to save, when the host is shutting down.
    save_generation: Option<u64>,
    notices: Vec<StateNotice>,
    failure: Option<StateFailure>,
}

impl ServiceState {
    const fn new() -> Self {
        Self {
            config: RetainConfig {
                enabled: false,
                file: None,
                policy: MismatchPolicy::Error,
                reset: false,
            },
            save_generation: None,
            notices: Vec::new(),
            failure: None,
        }
    }
}

static SERVICE: Mutex<ServiceState> = Mutex::new(ServiceState::new());

fn service() -> std::sync::MutexGuard<'static, ServiceState> {
    SERVICE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Installs the effective configuration and clears prior outcomes.
pub fn configure(config: RetainConfig) {
    let mut state = service();
    state.config = config;
    state.save_generation = None;
    state.notices.clear();
    state.failure = None;
}

/// Returns the installed configuration.
#[must_use]
pub fn configured() -> RetainConfig {
    service().config.clone()
}

/// Permits only `generation` to save until the next configuration change.
///
/// The host calls this for its final shutdown; retired generations shutting
/// down during an online swap are not allowed to rewrite the state file
/// (State §4.1.1).
pub fn allow_save(generation: Option<u64>) {
    service().save_generation = generation;
}

/// Takes the notices recorded since the last call.
#[must_use]
pub fn take_notices() -> Vec<StateNotice> {
    std::mem::take(&mut service().notices)
}

/// Takes the fatal failure recorded since the last call.
#[must_use]
pub fn take_failure() -> Option<StateFailure> {
    service().failure.take()
}

fn notice(code: &'static str, message: impl Into<String>) {
    service().notices.push(StateNotice {
        code,
        message: message.into(),
    });
}

fn fail(code: &'static str, message: impl Into<String>) {
    let message = message.into();
    let mut state = service();
    state.failure.get_or_insert(StateFailure {
        code,
        message: message.clone(),
    });
    state.notices.push(StateNotice { code, message });
}

/// Returns the FNV-1a 64 hash of the language profile identifier.
#[must_use]
pub fn profile_hash() -> u64 {
    fnv1a(PROFILE.as_bytes())
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Computes the IEEE CRC32 of `bytes`.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Encodes one complete state file.
#[must_use]
pub fn encode(payload: &[u8], layout_hash: u64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER_BYTES + payload.len() + CRC_BYTES);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&STATE_ABI.to_le_bytes());
    bytes.extend_from_slice(&profile_hash().to_le_bytes());
    bytes.extend_from_slice(&layout_hash.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(payload);
    let crc = crc32(&bytes);
    bytes.extend_from_slice(&crc.to_le_bytes());
    bytes
}

/// Decodes a state file into its payload.
///
/// # Errors
///
/// Returns [`LoadOutcome::Mismatch`] for an unknown version, ABI, profile, or
/// layout hash, and [`LoadOutcome::Corrupt`] for a bad magic, length, or CRC.
pub fn decode(bytes: &[u8], expected_len: u64, layout_hash: u64) -> Result<&[u8], LoadOutcome> {
    let mismatch = |message: String| Err(LoadOutcome::Mismatch(message));
    let corrupt = |message: String| Err(LoadOutcome::Corrupt(message));
    if bytes.len() < HEADER_BYTES + CRC_BYTES {
        return corrupt(format!(
            "state file is {} bytes; at least {} are required",
            bytes.len(),
            HEADER_BYTES + CRC_BYTES
        ));
    }
    if &bytes[0..8] != MAGIC {
        return corrupt("state file has the wrong magic".into());
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().expect("fixed slice"));
    if version != FORMAT_VERSION {
        return mismatch(format!(
            "state file format version {version} is not supported (expected {FORMAT_VERSION})"
        ));
    }
    let abi = u32::from_le_bytes(bytes[12..16].try_into().expect("fixed slice"));
    if abi != STATE_ABI {
        return mismatch(format!(
            "state file ABI major {abi} does not match the runtime ABI {STATE_ABI}"
        ));
    }
    let profile = u64::from_le_bytes(bytes[16..24].try_into().expect("fixed slice"));
    if profile != profile_hash() {
        return mismatch("state file profile does not match this build".into());
    }
    let stored_layout = u64::from_le_bytes(bytes[24..32].try_into().expect("fixed slice"));
    if stored_layout != layout_hash {
        return mismatch(format!(
            "state file layout hash 0x{stored_layout:016x} does not match the build's 0x{layout_hash:016x}"
        ));
    }
    let payload_len = u64::from_le_bytes(bytes[32..40].try_into().expect("fixed slice"));
    if payload_len != expected_len {
        return corrupt(format!(
            "state file payload length {payload_len} does not match the build's {expected_len}"
        ));
    }
    let expected_total = HEADER_BYTES
        .saturating_add(usize::try_from(payload_len).unwrap_or(usize::MAX))
        .saturating_add(CRC_BYTES);
    if bytes.len() != expected_total {
        return corrupt(format!(
            "state file is {} bytes; expected {expected_total}",
            bytes.len()
        ));
    }
    let crc_offset = HEADER_BYTES + usize::try_from(payload_len).unwrap_or(0);
    let stored_crc = u32::from_le_bytes(
        bytes[crc_offset..crc_offset + CRC_BYTES]
            .try_into()
            .expect("fixed slice"),
    );
    let actual_crc = crc32(&bytes[..crc_offset]);
    if stored_crc != actual_crc {
        return corrupt(format!(
            "state file CRC 0x{stored_crc:08x} does not match 0x{actual_crc:08x}"
        ));
    }
    Ok(&bytes[HEADER_BYTES..crc_offset])
}

/// Reads and validates a state file.
#[must_use]
pub fn load(path: &Path, expected_len: u64, layout_hash: u64, buffer: &mut [u8]) -> LoadOutcome {
    if expected_len > MAX_STATE_BYTES || expected_len != buffer.len() as u64 {
        return LoadOutcome::IoError(format!(
            "state buffer is {} bytes but the build retains {expected_len}",
            buffer.len()
        ));
    }
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return LoadOutcome::NotFound;
        }
        Err(error) => {
            return LoadOutcome::IoError(format!(
                "cannot read state file `{}`: {error}",
                path.display()
            ));
        }
    };
    let limit = u64::try_from(HEADER_BYTES + CRC_BYTES)
        .unwrap_or(0)
        .saturating_add(expected_len)
        .saturating_add(1);
    let mut bytes = Vec::new();
    if let Err(error) = std::io::Read::by_ref(&mut file)
        .take(limit)
        .read_to_end(&mut bytes)
    {
        return LoadOutcome::IoError(format!(
            "cannot read state file `{}`: {error}",
            path.display()
        ));
    }
    match decode(&bytes, expected_len, layout_hash) {
        Ok(payload) => {
            buffer.copy_from_slice(payload);
            LoadOutcome::Loaded
        }
        Err(outcome) => outcome,
    }
}

/// Writes a state file atomically.
///
/// # Errors
///
/// Reports the I/O or replace failure; the previous file is left untouched.
pub fn save(path: &Path, payload: &[u8], layout_hash: u64) -> Result<(), String> {
    if payload.len() as u64 > MAX_STATE_BYTES {
        return Err(format!(
            "state payload of {} bytes exceeds the {MAX_STATE_BYTES}-byte limit",
            payload.len()
        ));
    }
    let bytes = encode(payload, layout_hash);
    write_private_atomically(path, &bytes)
        .map_err(|error| format!("cannot write state file `{}`: {error}", path.display()))
}

/// Writes `bytes` to `<path>.tmp`, syncs, and atomically replaces `path`.
///
/// A failed replace leaves the previous file intact. The caller owns the
/// diagnostic message and code.
///
/// # Errors
///
/// Reports the create/write/sync/flush or replace failure.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    write_file_atomically(path, bytes, false)
}

/// Writes a runtime-private file atomically.
///
/// On Unix the temporary file is mode 0600 and the parent directory is synced
/// after the rename, so a power loss cannot resurrect the old file with the
/// new bytes missing (D-203). On Windows the replace is write-through.
///
/// # Errors
///
/// Reports the create/write/sync/flush or replace failure.
pub(crate) fn write_private_atomically(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    write_file_atomically(path, bytes, true)
}

fn write_file_atomically(path: &Path, bytes: &[u8], private: bool) -> Result<(), std::io::Error> {
    let temporary = temporary_path(path);
    let result = (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::PermissionsExt;
            // The create mode is masked by the process umask; set the exact
            // user-only mode before writing any state.
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        let _ = private;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        replace_file(&temporary, path)?;
        sync_parent(path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Syncs the directory that holds `path` after a rename (Unix).
///
/// Windows' `MoveFileEx` with `WRITE_THROUGH` already covers the rename, and
/// directories cannot be opened as ordinary files there.
#[cfg(unix)]
fn sync_parent(path: &Path) -> std::io::Result<()> {
    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Ok(());
    };
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Returns `<path>.tmp` in the same directory.
#[must_use]
pub fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

#[cfg(not(windows))]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    unsafe extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    let from: Vec<u16> = from
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let to: Vec<u16> = to
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: both paths are NUL-terminated UTF-16 buffers that live across the
    // call; MoveFileExW copies the names before returning.
    let result = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Implements the ABI 2 `state_load` service.
///
/// # Safety
///
/// `buffer` must be null for `len == 0`, or point to `len` writable bytes.
pub(crate) unsafe fn service_load(buffer: *mut u8, len: i64, layout_hash: i64) -> i32 {
    let config = service().config.clone();
    if !config.enabled {
        return 5;
    }
    let Ok(len) = u64::try_from(len) else {
        return 4;
    };
    let Ok(len_usize) = usize::try_from(len) else {
        return 4;
    };
    if len > MAX_STATE_BYTES {
        return 4;
    }
    if len == 0 {
        // Nothing is retained; do not create or validate a file.
        return 5;
    }
    if buffer.is_null() {
        return 4;
    }
    if config.reset {
        return 1;
    }
    let Some(path) = config.file else {
        return 4;
    };
    // SAFETY: the caller guarantees `len` writable bytes.
    let slice = unsafe { std::slice::from_raw_parts_mut(buffer, len_usize) };
    match load(&path, len, layout_hash as u64, slice) {
        LoadOutcome::Loaded => 0,
        LoadOutcome::NotFound => {
            notice(
                "STC-W-RUN-RETAIN-001",
                format!(
                    "state file `{}` was not found; starting cold with defaults",
                    path.display()
                ),
            );
            1
        }
        LoadOutcome::Mismatch(message) => {
            match config.policy {
                MismatchPolicy::Error => fail("STC-E-RUN-RETAIN-001", message),
                MismatchPolicy::Reset => notice(
                    "STC-W-RUN-RETAIN-002",
                    format!("{message}; resetting retained state to defaults"),
                ),
            }
            2
        }
        LoadOutcome::Corrupt(message) => {
            match config.policy {
                MismatchPolicy::Error => fail("STC-E-RUN-RETAIN-002", message),
                MismatchPolicy::Reset => notice(
                    "STC-W-RUN-RETAIN-002",
                    format!("{message}; resetting retained state to defaults"),
                ),
            }
            3
        }
        LoadOutcome::IoError(message) => {
            notice("STC-W-RUN-RETAIN-001", message);
            4
        }
    }
}

/// Implements the ABI 2 `state_save` service.
///
/// # Safety
///
/// `buffer` must be null for `len == 0`, or point to `len` readable bytes.
pub(crate) unsafe fn service_save(buffer: *const u8, len: i64, layout_hash: i64) -> i32 {
    let (config, allowed) = {
        let state = service();
        (
            state.config.clone(),
            state.save_generation == Some(crate::host_api::current_generation()),
        )
    };
    if !config.enabled || !allowed {
        return 5;
    }
    let Ok(len) = u64::try_from(len) else {
        return 4;
    };
    let Ok(len_usize) = usize::try_from(len) else {
        return 4;
    };
    if len > MAX_STATE_BYTES {
        return 4;
    }
    if len == 0 {
        return 5;
    }
    if buffer.is_null() {
        return 4;
    }
    let Some(path) = config.file else {
        return 4;
    };
    // SAFETY: the caller guarantees `len` readable bytes.
    let payload = unsafe { std::slice::from_raw_parts(buffer, len_usize) };
    match save(&path, payload, layout_hash as u64) {
        Ok(()) => 0,
        Err(message) => {
            fail("STC-E-RUN-RETAIN-003", message);
            4
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> Vec<u8> {
        vec![1, 0, 0, 0, 2, 0, 0, 0, 0xAA]
    }

    #[test]
    fn crc32_matches_the_reference_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn round_trips_the_documented_format() {
        let payload = payload();
        let hash = 0x0123_4567_89AB_CDEF;
        let bytes = encode(&payload, hash);
        assert_eq!(&bytes[0..8], MAGIC);
        assert_eq!(&bytes[8..12], &1_u32.to_le_bytes());
        assert_eq!(&bytes[12..16], &2_u32.to_le_bytes());
        assert_eq!(&bytes[16..24], &profile_hash().to_le_bytes());
        assert_eq!(&bytes[24..32], &hash.to_le_bytes());
        assert_eq!(&bytes[32..40], &(payload.len() as u64).to_le_bytes());
        assert_eq!(
            u32::from_le_bytes(bytes[bytes.len() - 4..].try_into().unwrap()),
            crc32(&bytes[..bytes.len() - 4])
        );
        assert_eq!(
            decode(&bytes, payload.len() as u64, hash),
            Ok(payload.as_slice())
        );
    }

    #[test]
    fn canonical_encoding_is_host_independent() {
        // A fixed payload and layout hash must encode to these exact bytes on
        // every architecture. The sequence pins the magic, version, ABI,
        // profile hash, layout hash, little-endian payload length, payload,
        // and CRC32, so a host-dependent layout or hash would fail here.
        let payload = [0x10_u8, 0x20, 0x30, 0x40, 0x50];
        let layout_hash = 0x0011_2233_4455_6677_u64;
        #[rustfmt::skip]
        let expected: [u8; 49] = [
            // magic "STCRET01"
            0x53, 0x54, 0x43, 0x52, 0x45, 0x54, 0x30, 0x31,
            // format version 1, little-endian u32
            0x01, 0x00, 0x00, 0x00,
            // runtime ABI major 2, little-endian u32
            0x02, 0x00, 0x00, 0x00,
            // FNV-1a profile hash of "stc-2013-oop-utf8-v1", little-endian u64
            0x3d, 0xc9, 0x12, 0xd8, 0x68, 0x53, 0x20, 0xce,
            // layout hash 0x0011223344556677, little-endian u64
            0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x00,
            // payload length 5, little-endian u64
            0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // payload
            0x10, 0x20, 0x30, 0x40, 0x50,
            // CRC32 (IEEE) 0x91350493 over the preceding 45 bytes
            0x93, 0x04, 0x35, 0x91,
        ];
        assert_eq!(encode(&payload, layout_hash), expected);
        assert_eq!(
            decode(&expected, payload.len() as u64, layout_hash),
            Ok(payload.as_slice())
        );
    }

    #[test]
    fn rejects_corrupt_and_mismatched_headers() {
        let payload = payload();
        let hash = 7;
        let bytes = encode(&payload, hash);

        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'X';
        assert!(matches!(
            decode(&bad_magic, payload.len() as u64, hash),
            Err(LoadOutcome::Corrupt(_))
        ));

        let mut bad_version = bytes.clone();
        bad_version[8..12].copy_from_slice(&2_u32.to_le_bytes());
        assert!(matches!(
            decode(&bad_version, payload.len() as u64, hash),
            Err(LoadOutcome::Mismatch(_))
        ));

        let mut bad_abi = bytes.clone();
        bad_abi[12..16].copy_from_slice(&3_u32.to_le_bytes());
        assert!(matches!(
            decode(&bad_abi, payload.len() as u64, hash),
            Err(LoadOutcome::Mismatch(_))
        ));

        let mut bad_profile = bytes.clone();
        bad_profile[16..24].copy_from_slice(&0_u64.to_le_bytes());
        assert!(matches!(
            decode(&bad_profile, payload.len() as u64, hash),
            Err(LoadOutcome::Mismatch(_))
        ));

        assert!(matches!(
            decode(&bytes, payload.len() as u64, hash + 1),
            Err(LoadOutcome::Mismatch(_))
        ));
        assert!(matches!(
            decode(&bytes, payload.len() as u64 + 1, hash),
            Err(LoadOutcome::Corrupt(_))
        ));
        assert!(matches!(
            decode(&bytes[..bytes.len() - 1], payload.len() as u64, hash),
            Err(LoadOutcome::Corrupt(_))
        ));

        let mut bad_crc = bytes.clone();
        let last = bad_crc.len() - 1;
        bad_crc[last] ^= 0xFF;
        assert!(matches!(
            decode(&bad_crc, payload.len() as u64, hash),
            Err(LoadOutcome::Corrupt(_))
        ));

        let mut bad_payload = bytes;
        bad_payload[HEADER_BYTES] ^= 0xFF;
        assert!(matches!(
            decode(&bad_payload, payload.len() as u64, hash),
            Err(LoadOutcome::Corrupt(_))
        ));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "stc-state-{}-{name}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        ));
        std::fs::create_dir_all(&path).expect("create temp directory");
        path
    }

    #[cfg(unix)]
    #[test]
    fn saved_state_is_user_only() {
        use std::os::unix::fs::PermissionsExt;

        let directory = temp_dir("private");
        let path = directory.join("app.stcstate");
        save(&path, &payload(), 7).expect("save");
        let mode = std::fs::metadata(&path)
            .expect("state metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "state files are user-only (D-203)");
        assert!(
            !temporary_path(&path).exists(),
            "the temporary file is renamed, not left behind"
        );
        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    #[cfg(unix)]
    #[test]
    fn failed_replace_keeps_the_target_and_removes_the_temporary() {
        let directory = temp_dir("failed-replace");
        let path = directory.join("app.stcstate");
        // A non-empty directory cannot be replaced by a file; the rename
        // fails after the temporary file has been written and synced.
        std::fs::create_dir_all(path.join("child")).expect("target directory");
        let error = save(&path, &payload(), 7).expect_err("replace must fail");
        assert!(
            error.contains("cannot write state file"),
            "unexpected error: {error}"
        );
        assert!(path.is_dir(), "the previous target is intact");
        assert!(
            !temporary_path(&path).exists(),
            "the failed temporary file is removed"
        );
        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    #[test]
    fn service_load_reports_the_abi_codes() {
        let _lock = crate::test_support::state_guard();
        let directory = temp_dir("service");
        let path = directory.join("state.bin");
        let mut buffer = [0_u8; 4];
        let payload = [1_u8, 2, 3, 4];

        // Missing file: cold start.
        configure(RetainConfig {
            enabled: true,
            file: Some(path.clone()),
            policy: MismatchPolicy::Error,
            reset: false,
        });
        // SAFETY: the buffer is four writable bytes.
        assert_eq!(
            unsafe { service_load(buffer.as_mut_ptr(), 4, 7) },
            1,
            "missing file is a cold start"
        );
        assert!(
            take_notices()
                .iter()
                .any(|notice| notice.code == "STC-W-RUN-RETAIN-001")
        );
        assert!(take_failure().is_none());

        // Valid file: restore.
        save(&path, &payload, 7).expect("save");
        assert_eq!(unsafe { service_load(buffer.as_mut_ptr(), 4, 7) }, 0);
        assert_eq!(buffer, payload);

        // Reset mode ignores the file and returns 1.
        configure(RetainConfig {
            enabled: true,
            file: Some(path.clone()),
            policy: MismatchPolicy::Error,
            reset: true,
        });
        assert_eq!(unsafe { service_load(buffer.as_mut_ptr(), 4, 7) }, 1);
        let _ = take_notices();

        // Corrupt file under the error policy.
        let mut bytes = std::fs::read(&path).expect("state");
        bytes[HEADER_BYTES] ^= 0xFF;
        std::fs::write(&path, &bytes).expect("corrupt state");
        configure(RetainConfig {
            enabled: true,
            file: Some(path.clone()),
            policy: MismatchPolicy::Error,
            reset: false,
        });
        assert_eq!(unsafe { service_load(buffer.as_mut_ptr(), 4, 7) }, 3);
        assert_eq!(take_failure().expect("fatal").code, "STC-E-RUN-RETAIN-002");

        // Corrupt file under the reset policy.
        configure(RetainConfig {
            enabled: true,
            file: Some(path.clone()),
            policy: MismatchPolicy::Reset,
            reset: false,
        });
        assert_eq!(unsafe { service_load(buffer.as_mut_ptr(), 4, 7) }, 3);
        assert!(take_failure().is_none());
        assert!(
            take_notices()
                .iter()
                .any(|notice| notice.code == "STC-W-RUN-RETAIN-002")
        );

        // Layout mismatch under the error policy.
        save(&path, &payload, 7).expect("save");
        configure(RetainConfig {
            enabled: true,
            file: Some(path.clone()),
            policy: MismatchPolicy::Error,
            reset: false,
        });
        assert_eq!(unsafe { service_load(buffer.as_mut_ptr(), 4, 8) }, 2);
        assert_eq!(take_failure().expect("fatal").code, "STC-E-RUN-RETAIN-001");

        // Disabled and empty payloads report "retention disabled".
        configure(RetainConfig::default());
        assert_eq!(unsafe { service_load(buffer.as_mut_ptr(), 4, 7) }, 5);
        configure(RetainConfig {
            enabled: true,
            file: Some(path.clone()),
            policy: MismatchPolicy::Error,
            reset: false,
        });
        assert_eq!(unsafe { service_load(std::ptr::null_mut(), 0, 7) }, 5);

        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    #[test]
    fn service_save_requires_the_active_generation() {
        let _lock = crate::test_support::state_guard();
        let directory = temp_dir("save-generation");
        let path = directory.join("state.bin");
        let payload = [9_u8, 8, 7, 6];
        configure(RetainConfig {
            enabled: true,
            file: Some(path.clone()),
            policy: MismatchPolicy::Error,
            reset: false,
        });

        // No generation is permitted to save yet.
        allow_save(None);
        // SAFETY: the payload is four readable bytes.
        assert_eq!(
            unsafe { service_save(payload.as_ptr(), 4, 7) },
            5,
            "saves are refused outside the normal shutdown"
        );
        assert!(!path.exists());

        // A retired generation cannot save either; the active one can.
        allow_save(Some(42));
        assert_eq!(unsafe { service_save(payload.as_ptr(), 4, 7) }, 5);
        allow_save(Some(0));
        assert_eq!(unsafe { service_save(payload.as_ptr(), 4, 7) }, 0);
        let mut buffer = [0_u8; 4];
        assert_eq!(load(&path, 4, 7, &mut buffer), LoadOutcome::Loaded);
        assert_eq!(buffer, payload);
        assert!(take_failure().is_none());

        // A save failure records STC-E-RUN-RETAIN-003.
        std::fs::create_dir(temporary_path(&path)).expect("block temporary path");
        assert_eq!(unsafe { service_save(payload.as_ptr(), 4, 7) }, 4);
        assert_eq!(take_failure().expect("fatal").code, "STC-E-RUN-RETAIN-003");

        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    #[test]
    fn save_is_atomic_and_leaves_the_previous_file_on_failure() {
        let directory = std::env::temp_dir().join(format!(
            "stc-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        ));
        std::fs::create_dir_all(&directory).expect("create temp directory");
        let path = directory.join("state.bin");
        std::fs::write(&path, b"previous").expect("write previous state");

        // A directory at the temporary path makes the write fail before the
        // replace; the previous file must stay intact.
        std::fs::create_dir(temporary_path(&path)).expect("block temporary path");
        let error = save(&path, &payload(), 5).expect_err("save must fail");
        assert!(error.contains("cannot write state file"), "{error}");
        assert_eq!(std::fs::read(&path).expect("previous state"), b"previous");
        std::fs::remove_dir(temporary_path(&path)).expect("remove blocker");

        save(&path, &payload(), 5).expect("atomic save");
        let mut buffer = vec![0; payload().len()];
        assert_eq!(
            load(&path, payload().len() as u64, 5, &mut buffer),
            LoadOutcome::Loaded
        );
        assert_eq!(buffer, payload());

        // A wrong layout hash is a mismatch and leaves the file unchanged.
        let before = std::fs::read(&path).expect("state");
        let mut buffer = vec![0; payload().len()];
        assert!(matches!(
            load(&path, payload().len() as u64, 6, &mut buffer),
            LoadOutcome::Mismatch(_)
        ));
        assert_eq!(std::fs::read(&path).expect("state"), before);

        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    #[test]
    fn save_replaces_an_existing_file() {
        let directory = std::env::temp_dir().join(format!(
            "stc-state-replace-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        ));
        std::fs::create_dir_all(&directory).expect("create temp directory");
        let path = directory.join("state.bin");
        save(&path, &[1, 2, 3], 9).expect("first save");
        save(&path, &[4, 5], 9).expect("replace");
        let mut buffer = vec![0; 2];
        assert_eq!(load(&path, 2, 9, &mut buffer), LoadOutcome::Loaded);
        assert_eq!(buffer, [4, 5]);
        std::fs::remove_dir_all(&directory).expect("clean up");
    }
}
