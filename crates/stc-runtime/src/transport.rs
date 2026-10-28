//! Local transport primitives shared by the control channel and the debug
//! service (Debug and Monitoring §13.1, D-173).
//!
//! The platform backends live here: a Windows named-pipe listener created with
//! a user-only DACL, a Unix domain socket created inside a mode-0700 directory
//! with mode 0600, and a loopback-only TCP listener on an ephemeral port. The
//! module also owns the OS primitives the debug service needs and the rest of
//! the runtime does not: a cryptographic random source for the per-run token
//! and endpoint suffix, private (user-only) file creation, an atomic endpoint
//! file replacement, the monotonic clock, and the local-date clock.
//!
//! All unsafe code is confined to the Windows FFI block. The workspace forbids
//! `unsafe_code` except in this crate's documented platform exception; the
//! declarations below mirror the control channel's original FFI and add only
//! `SystemFunction036`, `GetLocalTime`, and `MoveFileExW`.

use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use stc_got::error::{Error, Result, codes};

/// How the debug endpoint is exposed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportKind {
    /// Windows named pipe, or a Unix domain socket on Linux.
    Pipe,
    /// Loopback TCP on `127.0.0.1` with an ephemeral port.
    Socket,
}

impl TransportKind {
    /// Parses the `[debug].protocol` spelling.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "pipe" => Some(Self::Pipe),
            "socket" => Some(Self::Socket),
            _ => None,
        }
    }

    /// Returns the endpoint-file spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pipe => "pipe",
            Self::Socket => "socket",
        }
    }
}

pub(crate) fn transport_error(message: impl Into<String>) -> Error {
    Error::new(codes::DEBUG_TRANSPORT, message.into())
}

// ---------------------------------------------------------------------------
// Cryptographic randomness
// ---------------------------------------------------------------------------

/// Fills a buffer from the OS cryptographic random source.
///
/// # Errors
///
/// Returns an I/O error when the OS source is unavailable.
pub(crate) fn fill_random(buffer: &mut [u8]) -> io::Result<()> {
    platform::fill_random(buffer)
}

/// Returns `count` random bytes encoded as lowercase hex.
///
/// # Errors
///
/// Returns an I/O error when the OS source is unavailable.
pub(crate) fn random_hex(count: usize) -> io::Result<String> {
    let mut bytes = vec![0_u8; count];
    fill_random(&mut bytes)?;
    let mut out = String::with_capacity(count * 2);
    for byte in bytes {
        out.push(char::from_digit(u32::from(byte >> 4), 16).expect("nibble"));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).expect("nibble"));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Clocks
// ---------------------------------------------------------------------------

/// Returns host monotonic nanoseconds since the runtime started.
#[must_use]
pub(crate) fn monotonic_ns() -> u64 {
    static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let base = BASE.get_or_init(Instant::now);
    u64::try_from(base.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// Returns the host local date as Unix-epoch nanoseconds, or `0` when the
/// platform clock is unavailable.
#[must_use]
pub(crate) fn local_date_ns() -> i64 {
    platform::local_date_ns().unwrap_or(0)
}

// ---------------------------------------------------------------------------
// User-only files
// ---------------------------------------------------------------------------

/// Creates a new user-only file, refusing an existing path or symlink.
///
/// On Unix the file is created with mode 0600. On Windows it is created with a
/// DACL that grants access only to the current user.
///
/// # Errors
///
/// Returns an I/O error when the path exists, is a symlink, or cannot be
/// created with user-only access.
pub(crate) fn create_private_file(path: &Path) -> io::Result<std::fs::File> {
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "refusing to follow a symlink for a runtime file",
            ));
        }
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the runtime file already exists",
        ));
    }
    platform::create_private_file(path)
}

/// Creates a new user-private directory (mode 0700 on Unix).
///
/// # Errors
///
/// Returns an I/O error when the path exists or cannot be created.
#[cfg(unix)]
pub(crate) fn create_private_dir(path: &Path) -> io::Result<()> {
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "refusing to follow a symlink for a runtime directory",
            ));
        }
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the runtime directory already exists",
        ));
    }
    platform::create_private_dir(path)
}

/// Verifies that a runtime path is not a symlink.
///
/// # Errors
///
/// Returns a permission error when the path is a symlink.
pub(crate) fn refuse_symlink(path: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing to follow a symlink for a debug endpoint or audit file",
        ));
    }
    Ok(())
}

/// Atomically writes a user-only endpoint file.
///
/// The contents are written to a private temporary file in the same directory
/// and then moved into place. `replace` selects the activation refresh; the
/// first write must create a new file and fails when the path already exists.
///
/// # Errors
///
/// Returns an I/O error for an existing target (without `replace`), a symlink,
/// a missing parent directory, or a failed atomic move.
pub(crate) fn write_private_atomic(path: &Path, contents: &str, replace: bool) -> io::Result<()> {
    refuse_symlink(path)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent {
        let metadata = std::fs::metadata(parent)?;
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "the endpoint parent is not a directory",
            ));
        }
        refuse_symlink(parent)?;
    } else if !Path::new(".").is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the endpoint parent directory does not exist",
        ));
    }
    if !replace && std::fs::symlink_metadata(path).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the endpoint file already exists; a stale file is never reused",
        ));
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("stc-debug");
    let suffix = random_hex(8)?;
    let temporary = path.with_file_name(format!(".{name}.tmp-{suffix}"));
    let result = (|| {
        let mut file = create_private_file(&temporary)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        drop(file);
        platform::replace_file(&temporary, path, replace)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

// ---------------------------------------------------------------------------
// Listener and connections
// ---------------------------------------------------------------------------

/// A server-side connection with an optional absolute read deadline.
#[allow(private_interfaces)]
pub(crate) enum Connection {
    Platform(platform::Connection),
    Tcp(TcpConnection),
}

impl Connection {
    /// Sets the absolute deadline for blocking reads. A read past the deadline
    /// returns `io::ErrorKind::TimedOut`.
    pub fn set_read_deadline(&mut self, deadline: Option<Instant>) {
        match self {
            Self::Platform(connection) => connection.set_read_deadline(deadline),
            Self::Tcp(connection) => connection.deadline = deadline,
        }
    }

    /// Sets a write timeout, when the transport supports one.
    pub fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            Self::Platform(connection) => connection.set_write_timeout(timeout),
            Self::Tcp(connection) => connection.stream.set_write_timeout(timeout),
        }
    }
}

impl Read for Connection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Platform(connection) => connection.read(buffer),
            Self::Tcp(connection) => connection.read(buffer),
        }
    }
}

impl Write for Connection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Platform(connection) => connection.write(buffer),
            Self::Tcp(connection) => connection.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Platform(connection) => connection.flush(),
            Self::Tcp(connection) => connection.flush(),
        }
    }
}

/// A TCP connection wrapper that observes the server shutdown flag and an
/// optional read deadline.
pub(crate) struct TcpConnection {
    stream: std::net::TcpStream,
    shutdown: Arc<AtomicBool>,
    deadline: Option<Instant>,
}

impl TcpConnection {
    fn new(stream: std::net::TcpStream, shutdown: Arc<AtomicBool>) -> io::Result<Self> {
        stream.set_nonblocking(false)?;
        Ok(Self {
            stream,
            shutdown,
            deadline: None,
        })
    }
}

impl Read for TcpConnection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                return Ok(0);
            }
            let now = Instant::now();
            let timeout = match self.deadline {
                Some(deadline) if deadline <= now => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the read deadline expired",
                    ));
                }
                Some(deadline) => deadline.saturating_duration_since(now),
                None => Duration::from_millis(200),
            };
            self.stream
                .set_read_timeout(Some(timeout.max(Duration::from_millis(1))))?;
            match self.stream.read(buffer) {
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    if self.shutdown.load(Ordering::Acquire) {
                        return Ok(0);
                    }
                    if self
                        .deadline
                        .is_some_and(|deadline| deadline <= Instant::now())
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "the read deadline expired",
                        ));
                    }
                }
                other => return other,
            }
        }
    }
}

impl Write for TcpConnection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.stream.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

/// A bound listener for a debug endpoint.
#[allow(private_interfaces)]
pub(crate) enum Listener {
    Platform(platform::Backend),
    Tcp(TcpListener),
}

struct TcpListener {
    listener: std::net::TcpListener,
}

impl Listener {
    /// Binds the platform pipe/socket primitive for `endpoint`.
    ///
    /// On Unix `endpoint` is the socket path and its private directory has
    /// already been created.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-DEBUG-020` when the primitive cannot be created with
    /// user-only access.
    pub fn bind_pipe(endpoint: &str) -> Result<Self> {
        platform::Backend::bind(endpoint).map(Self::Platform)
    }

    /// Binds loopback TCP on an ephemeral port and returns the endpoint
    /// spelling `127.0.0.1:<port>`.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-DEBUG-020` when the loopback listener cannot be created.
    pub fn bind_tcp() -> Result<(Self, String)> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| transport_error(format!("cannot bind loopback TCP: {error}")))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| transport_error(format!("cannot configure loopback TCP: {error}")))?;
        let address = listener
            .local_addr()
            .map_err(|error| transport_error(format!("cannot read the TCP endpoint: {error}")))?;
        Ok((Self::Tcp(TcpListener { listener }), address.to_string()))
    }

    /// Waits for one connection. Returns `None` when shutdown was observed or
    /// the listener failed.
    pub fn accept(&mut self, shutdown: &Arc<AtomicBool>) -> Option<Connection> {
        match self {
            Self::Platform(backend) => backend.accept(shutdown),
            Self::Tcp(listener) => loop {
                if shutdown.load(Ordering::Acquire) {
                    return None;
                }
                match listener.listener.accept() {
                    Ok((stream, _)) => {
                        if let Ok(connection) = TcpConnection::new(stream, Arc::clone(shutdown)) {
                            return Some(Connection::Tcp(connection));
                        }
                        return None;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => return None,
                }
            },
        }
    }

    /// Releases the listener. A Unix socket and its private directory are
    /// removed; a named pipe disappears when its last handle closes.
    pub fn shutdown(&mut self) {
        match self {
            Self::Platform(backend) => backend.shutdown(),
            Self::Tcp(_) => {}
        }
    }
}

/// A client connection to a control or debug endpoint.
#[allow(private_interfaces)]
pub(crate) enum ClientConnection {
    Platform(platform::ClientConnection),
    Tcp(std::net::TcpStream),
}

impl ClientConnection {
    /// Connects to the platform pipe/socket endpoint.
    ///
    /// # Errors
    ///
    /// Returns the transport error when the endpoint is unreachable.
    pub fn connect(endpoint: &str) -> io::Result<Self> {
        platform::ClientConnection::connect(endpoint).map(Self::Platform)
    }

    /// Connects to the endpoint for the given transport spelling.
    ///
    /// # Errors
    ///
    /// Returns the transport error when the endpoint is unreachable.
    pub fn connect_debug(transport: TransportKind, endpoint: &str) -> io::Result<Self> {
        match transport {
            TransportKind::Pipe => Self::connect(endpoint),
            TransportKind::Socket => std::net::TcpStream::connect(endpoint).map(Self::Tcp),
        }
    }

    /// Sets the client read timeout.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the platform cannot set a timeout; the
    /// named-pipe client does not support a per-read timeout.
    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            Self::Platform(connection) => connection.set_read_timeout(timeout),
            Self::Tcp(stream) => stream.set_read_timeout(timeout),
        }
    }

    /// Sets the client write timeout, when supported.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the platform cannot set a timeout.
    pub fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            Self::Platform(connection) => connection.set_write_timeout(timeout),
            Self::Tcp(stream) => stream.set_write_timeout(timeout),
        }
    }

    /// Reports whether the connection already has data or has reached EOF.
    ///
    /// This never blocks, so a client can poll for spontaneous events between
    /// requests even where the platform has no per-read timeout.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the platform probe fails.
    pub fn poll_readable(&mut self) -> io::Result<bool> {
        match self {
            Self::Platform(connection) => connection.poll_readable(),
            Self::Tcp(stream) => poll_tcp(stream),
        }
    }
}

fn poll_tcp(stream: &std::net::TcpStream) -> io::Result<bool> {
    stream.set_nonblocking(true)?;
    let mut byte = [0_u8; 1];
    let result = match stream.peek(&mut byte) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(false),
        Err(error) => Err(error),
    };
    stream.set_nonblocking(false)?;
    result
}

impl Read for ClientConnection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Platform(connection) => connection.read(buffer),
            Self::Tcp(stream) => stream.read(buffer),
        }
    }
}

impl Write for ClientConnection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Platform(connection) => connection.write(buffer),
            Self::Tcp(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Platform(connection) => connection.flush(),
            Self::Tcp(stream) => stream.flush(),
        }
    }
}

/// Returns the platform default endpoint spelling for `pipe` on Windows.
#[cfg(windows)]
#[must_use]
#[cfg(windows)]
pub(crate) fn default_pipe_endpoint(unique: &str) -> String {
    platform::default_pipe_endpoint(unique)
}

/// Removes a Unix socket path and its private directory.
#[cfg(unix)]
pub(crate) fn remove_socket_dir(directory: &Path) {
    let _ = std::fs::remove_dir_all(directory);
}

/// Returns a display-safe path description: only the file name, never an
/// absolute host path (Debug and Monitoring §13.2/§13.3).
#[must_use]
pub(crate) fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || "runtime file".into(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(unix)]
mod platform {
    use std::fs::File;
    use std::io::{self, Read, Write};
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use super::transport_error;

    pub(super) fn fill_random(buffer: &mut [u8]) -> io::Result<()> {
        let mut source = File::open("/dev/urandom")?;
        source.read_exact(buffer)
    }

    pub(super) fn local_date_ns() -> Option<i64> {
        // A portable local-date clock is not available without libc; the
        // protocol and audit format permit zero.
        None
    }

    pub(super) fn create_private_file(path: &Path) -> io::Result<File> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }

    pub(super) fn create_private_dir(path: &Path) -> io::Result<()> {
        std::fs::DirBuilder::new().mode(0o700).create(path)
    }

    pub(super) fn replace_file(from: &Path, to: &Path, _replace: bool) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    pub(super) struct Connection {
        stream: UnixStream,
        shutdown: Arc<AtomicBool>,
        deadline: Option<Instant>,
    }

    impl Connection {
        pub(super) fn set_read_deadline(&mut self, deadline: Option<Instant>) {
            self.deadline = deadline;
        }

        pub(super) fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
            self.stream.set_write_timeout(timeout)
        }
    }

    impl Read for Connection {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            loop {
                if self.shutdown.load(Ordering::Acquire) {
                    return Ok(0);
                }
                let now = Instant::now();
                let timeout = match self.deadline {
                    Some(deadline) if deadline <= now => {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "the read deadline expired",
                        ));
                    }
                    Some(deadline) => deadline.saturating_duration_since(now),
                    None => Duration::from_millis(200),
                };
                self.stream
                    .set_read_timeout(Some(timeout.max(Duration::from_millis(1))))?;
                match self.stream.read(buffer) {
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                        ) =>
                    {
                        if self.shutdown.load(Ordering::Acquire) {
                            return Ok(0);
                        }
                        if self
                            .deadline
                            .is_some_and(|deadline| deadline <= Instant::now())
                        {
                            return Err(io::Error::new(
                                io::ErrorKind::TimedOut,
                                "the read deadline expired",
                            ));
                        }
                    }
                    other => return other,
                }
            }
        }
    }

    impl Write for Connection {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.stream.write(buffer)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.stream.flush()
        }
    }

    pub(super) struct ClientConnection {
        stream: UnixStream,
    }

    impl ClientConnection {
        pub(super) fn connect(endpoint: &str) -> io::Result<Self> {
            let stream = UnixStream::connect(endpoint)?;
            stream.set_read_timeout(Some(Duration::from_secs(30)))?;
            stream.set_write_timeout(Some(Duration::from_secs(5)))?;
            Ok(Self { stream })
        }

        pub(super) fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
            self.stream.set_read_timeout(timeout)
        }

        pub(super) fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
            self.stream.set_write_timeout(timeout)
        }

        pub(super) fn poll_readable(&mut self) -> io::Result<bool> {
            self.stream.set_nonblocking(true)?;
            let mut byte = [0_u8; 1];
            let result = match self.stream.peek(&mut byte) {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(false),
                Err(error) => Err(error),
            };
            self.stream.set_nonblocking(false)?;
            result
        }
    }

    impl Read for ClientConnection {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.stream.read(buffer)
        }
    }

    impl Write for ClientConnection {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.stream.write(buffer)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.stream.flush()
        }
    }

    pub(super) struct Backend {
        listener: UnixListener,
        path: PathBuf,
    }

    impl Backend {
        pub(super) fn bind(endpoint: &str) -> stc_got::error::Result<Self> {
            let path = PathBuf::from(endpoint);
            let listener = UnixListener::bind(&path).map_err(|error| {
                transport_error(format!(
                    "cannot bind the local socket `{}`: {error}",
                    super::display_name(&path)
                ))
            })?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(
                |error| {
                    transport_error(format!(
                        "cannot restrict the local socket permissions: {error}"
                    ))
                },
            )?;
            listener.set_nonblocking(true).map_err(|error| {
                transport_error(format!("cannot configure the local socket: {error}"))
            })?;
            Ok(Self { listener, path })
        }

        pub(super) fn accept(&mut self, shutdown: &Arc<AtomicBool>) -> Option<super::Connection> {
            loop {
                if shutdown.load(Ordering::Acquire) {
                    return None;
                }
                match self.listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                        return Some(super::Connection::Platform(super::platform::Connection {
                            stream,
                            shutdown: Arc::clone(shutdown),
                            deadline: None,
                        }));
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(_) => return None,
                }
            }
        }

        pub(super) fn shutdown(&mut self) {
            let _ = std::fs::remove_file(&self.path);
            if let Some(parent) = self.path.parent()
                && parent
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".sockd"))
            {
                let _ = std::fs::remove_dir(parent);
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::{OsStr, c_void};
    use std::fs::File;
    use std::io::{self, Read, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    type Handle = *mut c_void;

    const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
    const PIPE_TYPE_BYTE: u32 = 0x0000_0000;
    const PIPE_READMODE_BYTE: u32 = 0x0000_0000;
    const PIPE_WAIT: u32 = 0x0000_0000;
    const PIPE_NOWAIT: u32 = 0x0000_0001;
    const PIPE_UNLIMITED_INSTANCES: u32 = 255;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const OPEN_EXISTING: u32 = 3;
    const CREATE_NEW: u32 = 1;
    const FILE_SHARE_READ: u32 = 1;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    const ERROR_FILE_NOT_FOUND: u32 = 2;
    const ERROR_PIPE_BUSY: u32 = 231;
    const ERROR_PIPE_CONNECTED: u32 = 535;
    const ERROR_PIPE_LISTENING: u32 = 536;
    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_USER: u32 = 1;
    const ACL_REVISION: u32 = 2;
    const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
    const GENERIC_ALL: u32 = 0x1000_0000;

    #[repr(C)]
    struct SecurityAttributes {
        n_length: u32,
        security_descriptor: *mut c_void,
        inherit_handle: i32,
    }

    #[repr(C)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }

    #[repr(C, align(8))]
    struct Aligned<const N: usize>([u8; N]);

    impl<const N: usize> Aligned<N> {
        fn new() -> Self {
            Self([0; N])
        }

        fn as_mut(&mut self) -> *mut c_void {
            self.0.as_mut_ptr().cast()
        }
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateNamedPipeW(
            name: *const u16,
            open_mode: u32,
            pipe_mode: u32,
            max_instances: u32,
            out_buffer: u32,
            in_buffer: u32,
            timeout: u32,
            attributes: *const SecurityAttributes,
        ) -> Handle;
        fn ConnectNamedPipe(pipe: Handle, overlapped: *mut c_void) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            attributes: *const SecurityAttributes,
            disposition: u32,
            flags: u32,
            template: Handle,
        ) -> Handle;
        fn WaitNamedPipeW(name: *const u16, timeout: u32) -> i32;
        fn PeekNamedPipe(
            pipe: Handle,
            buffer: *mut c_void,
            buffer_size: u32,
            read: *mut u32,
            available: *mut u32,
            left: *mut u32,
        ) -> i32;
        fn ReadFile(
            file: Handle,
            buffer: *mut u8,
            bytes: u32,
            read: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn WriteFile(
            file: Handle,
            buffer: *const u8,
            bytes: u32,
            written: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn GetCurrentProcess() -> Handle;
        fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
        fn GetTokenInformation(
            token: Handle,
            class: u32,
            info: *mut c_void,
            length: u32,
            needed: *mut u32,
        ) -> i32;
        fn GetLastError() -> u32;
        fn GetLocalTime(system_time: *mut SystemTime);
        fn MoveFileExW(existing: *const u16, new_name: *const u16, flags: u32) -> i32;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn InitializeSecurityDescriptor(descriptor: *mut c_void, revision: u32) -> i32;
        fn SetSecurityDescriptorDacl(
            descriptor: *mut c_void,
            present: i32,
            acl: *mut c_void,
            defaulted: i32,
        ) -> i32;
        fn InitializeAcl(acl: *mut c_void, length: u32, revision: u32) -> i32;
        fn AddAccessAllowedAce(
            acl: *mut c_void,
            revision: u32,
            access: u32,
            sid: *mut c_void,
        ) -> i32;
        #[link_name = "SystemFunction036"]
        fn SystemFunction036(buffer: *mut c_void, length: u32) -> u8;
    }

    fn wide(path: &str) -> Vec<u16> {
        OsStr::new(path).encode_wide().chain([0]).collect()
    }

    fn last_error() -> u32 {
        // SAFETY: GetLastError has no preconditions.
        unsafe { GetLastError() }
    }

    pub(super) fn fill_random(buffer: &mut [u8]) -> io::Result<()> {
        let length = u32::try_from(buffer.len()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "random request is too large")
        })?;
        // SAFETY: the buffer is valid for `length` bytes.
        let ok = unsafe { SystemFunction036(buffer.as_mut_ptr().cast(), length) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn local_date_ns() -> Option<i64> {
        let mut time = SystemTime {
            year: 0,
            month: 0,
            day_of_week: 0,
            day: 0,
            hour: 0,
            minute: 0,
            second: 0,
            milliseconds: 0,
        };
        // SAFETY: the out pointer is valid.
        unsafe { GetLocalTime(&mut time) };
        if time.year == 0 {
            return None;
        }
        let days = civil_days(
            i64::from(time.year),
            i64::from(time.month),
            i64::from(time.day),
        );
        let seconds = days * 86_400
            + i64::from(time.hour) * 3_600
            + i64::from(time.minute) * 60
            + i64::from(time.second);
        Some(seconds * 1_000_000_000 + i64::from(time.milliseconds) * 1_000_000)
    }

    /// Days since 1970-01-01 for a proleptic Gregorian date.
    fn civil_days(year: i64, month: i64, day: i64) -> i64 {
        let year = if month <= 2 { year - 1 } else { year };
        let era = if year >= 0 { year } else { year - 399 } / 400;
        let year_of_era = year - era * 400;
        let month_index = if month > 2 { month - 3 } else { month + 9 };
        let day_of_year = (153 * month_index + 2) / 5 + day - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    /// A user-only security descriptor and its ACL.
    struct UserSecurity {
        _descriptor: Aligned<64>,
        _acl: Aligned<256>,
        _token: Aligned<1024>,
        attributes: SecurityAttributes,
    }

    impl UserSecurity {
        fn new() -> Option<Self> {
            let mut token = Aligned::<1024>::new();
            let mut token_handle: Handle = std::ptr::null_mut();
            // SAFETY: the handles and buffers are valid for these calls; the
            // access mask, class, and lengths are fixed Windows contract values.
            unsafe {
                if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token_handle) == 0 {
                    return None;
                }
                let mut needed = 0_u32;
                let _ = GetTokenInformation(
                    token_handle,
                    TOKEN_USER,
                    token.as_mut(),
                    1024,
                    &mut needed,
                );
                if needed == 0
                    || needed > 1024
                    || GetTokenInformation(
                        token_handle,
                        TOKEN_USER,
                        token.as_mut(),
                        1024,
                        &mut needed,
                    ) == 0
                {
                    CloseHandle(token_handle);
                    return None;
                }
                CloseHandle(token_handle);
            }
            // TOKEN_USER starts with SID_AND_ATTRIBUTES { PSID Sid; DWORD }.
            let sid = unsafe { *token.0.as_ptr().cast::<*mut c_void>() };
            if sid.is_null() {
                return None;
            }
            let mut descriptor = Aligned::<64>::new();
            let mut acl = Aligned::<256>::new();
            // SAFETY: the buffers meet the documented minimum sizes and the
            // SID came from the current process token.
            unsafe {
                if InitializeSecurityDescriptor(descriptor.as_mut(), SECURITY_DESCRIPTOR_REVISION)
                    == 0
                    || InitializeAcl(acl.as_mut(), 256, ACL_REVISION) == 0
                    || AddAccessAllowedAce(acl.as_mut(), ACL_REVISION, GENERIC_ALL, sid) == 0
                    || SetSecurityDescriptorDacl(descriptor.as_mut(), 1, acl.as_mut(), 0) == 0
                {
                    return None;
                }
            }
            Some(Self {
                attributes: SecurityAttributes {
                    n_length: u32::try_from(std::mem::size_of::<SecurityAttributes>()).ok()?,
                    security_descriptor: descriptor.as_mut(),
                    inherit_handle: 0,
                },
                _descriptor: descriptor,
                _acl: acl,
                _token: token,
            })
        }
    }

    pub(super) fn create_private_file(path: &Path) -> io::Result<File> {
        let name = wide(&path.to_string_lossy());
        let security = UserSecurity::new().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "cannot build a user-only security descriptor",
            )
        })?;
        // SAFETY: the name is NUL-terminated and the security attributes point
        // to live buffers for the duration of the call.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                // Read sharing lets a monitor tail the append-only audit log;
                // writes and deletes stay exclusive to the owning process.
                FILE_SHARE_READ,
                &security.attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        if handle.is_null() || handle == (-1_isize as Handle) {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateFileW returned an owned handle.
        Ok(unsafe { File::from_raw_handle(handle.cast::<c_void>() as RawHandle) })
    }

    pub(super) fn replace_file(from: &Path, to: &Path, replace: bool) -> io::Result<()> {
        let existing = wide(&from.to_string_lossy());
        let new_name = wide(&to.to_string_lossy());
        let mut flags = MOVEFILE_WRITE_THROUGH;
        if replace {
            flags |= MOVEFILE_REPLACE_EXISTING;
        }
        // SAFETY: both names are NUL-terminated.
        let ok = unsafe { MoveFileExW(existing.as_ptr(), new_name.as_ptr(), flags) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn default_pipe_endpoint(unique: &str) -> String {
        format!(r"\\.\pipe\stc-{unique}")
    }

    pub(super) struct Connection {
        handle: Handle,
        shutdown: Arc<AtomicBool>,
        deadline: Option<Instant>,
    }

    // The handle is used by one handler thread at a time; Windows pipe
    // handles are process-wide values.
    unsafe impl Send for Connection {}

    impl Connection {
        pub(super) fn set_read_deadline(&mut self, deadline: Option<Instant>) {
            self.deadline = deadline;
        }

        pub(super) fn set_write_timeout(&mut self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
    }

    pub(super) struct ClientConnection {
        file: File,
    }

    impl ClientConnection {
        pub(super) fn connect(endpoint: &str) -> io::Result<Self> {
            let name = wide(endpoint);
            // SAFETY: the name is NUL-terminated and the access mask is valid.
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if !handle.is_null() && handle != (-1_isize as Handle) {
                // SAFETY: CreateFileW returned an owned handle.
                let file = unsafe { File::from_raw_handle(handle.cast::<c_void>() as RawHandle) };
                return Ok(Self { file });
            }
            let error = io::Error::last_os_error();
            if matches!(last_error(), ERROR_FILE_NOT_FOUND | ERROR_PIPE_BUSY) {
                // SAFETY: the name is NUL-terminated.
                unsafe { WaitNamedPipeW(name.as_ptr(), 25) };
            }
            Err(error)
        }

        pub(super) fn set_read_timeout(&mut self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }

        pub(super) fn set_write_timeout(&mut self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }

        pub(super) fn poll_readable(&mut self) -> io::Result<bool> {
            let mut available = 0_u32;
            // SAFETY: the file handle is live and the out pointer is valid.
            let ok = unsafe {
                PeekNamedPipe(
                    self.file.as_raw_handle().cast::<c_void>(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                // A broken pipe is treated as readable so the caller sees EOF.
                return Ok(true);
            }
            Ok(available > 0)
        }
    }

    impl Read for ClientConnection {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.file.read(buffer)
        }
    }

    impl Write for ClientConnection {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.file.write(buffer)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.file.flush()
        }
    }

    pub(super) struct Backend {
        endpoint: String,
        pending: Option<PipeHandle>,
    }

    /// An owned pipe instance created at bind time.
    ///
    /// The raw handle is moved once from the host thread into the accept
    /// thread; it is never shared between threads.
    struct PipeHandle(Handle);

    // SAFETY: the handle is an owned OS primitive used by one thread at a time.
    unsafe impl Send for PipeHandle {}

    impl Backend {
        pub(super) fn bind(endpoint: &str) -> stc_got::error::Result<Self> {
            Ok(Self {
                endpoint: endpoint.to_owned(),
                pending: Some(create_pipe(endpoint)?),
            })
        }

        pub(super) fn accept(&mut self, shutdown: &Arc<AtomicBool>) -> Option<super::Connection> {
            let handle = match self.pending.take() {
                Some(handle) => handle,
                None => match create_pipe(&self.endpoint) {
                    Ok(handle) => handle,
                    Err(_) => {
                        std::thread::sleep(Duration::from_millis(10));
                        return None;
                    }
                },
            };
            let handle = handle.0;
            // Keep the endpoint bound for a reconnect after this connection.
            self.pending = create_pipe(&self.endpoint).ok();
            loop {
                if shutdown.load(Ordering::Acquire) {
                    // SAFETY: the handle is live and owned here.
                    unsafe { CloseHandle(handle) };
                    return None;
                }
                // SAFETY: the handle is live.
                let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
                if connected != 0 {
                    break;
                }
                match last_error() {
                    ERROR_PIPE_CONNECTED => break,
                    ERROR_PIPE_LISTENING => std::thread::sleep(Duration::from_millis(10)),
                    _ => {
                        // SAFETY: the handle is live.
                        unsafe { CloseHandle(handle) };
                        return None;
                    }
                }
            }
            let mut mode = PIPE_READMODE_BYTE | PIPE_WAIT;
            // SAFETY: the connected handle accepts a mode change.
            unsafe {
                SetNamedPipeHandleState(
                    handle,
                    &mut mode,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
            }
            Some(super::Connection::Platform(super::platform::Connection {
                handle,
                shutdown: Arc::clone(shutdown),
                deadline: None,
            }))
        }

        pub(super) fn shutdown(&mut self) {
            if let Some(handle) = self.pending.take() {
                // SAFETY: the handle is owned by this backend.
                unsafe { CloseHandle(handle.0) };
            }
        }
    }

    fn create_pipe(endpoint: &str) -> stc_got::error::Result<PipeHandle> {
        let name = wide(endpoint);
        let security = UserSecurity::new().ok_or_else(|| {
            super::transport_error(
                "cannot establish a user-only access control list for the debug pipe",
            )
        })?;
        // SAFETY: the name is NUL-terminated and the security attributes point
        // to live buffers for the duration of the call.
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                &security.attributes,
            )
        };
        if handle.is_null() || handle == (-1_isize as Handle) {
            return Err(super::transport_error(
                "cannot create the user-only local debug pipe",
            ));
        }
        Ok(PipeHandle(handle))
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "SetNamedPipeHandleState"]
        fn SetNamedPipeHandleState(
            pipe: Handle,
            mode: *mut u32,
            max_collection_count: *mut u32,
            collect_data_timeout: *mut u32,
        ) -> i32;
    }

    impl Drop for Connection {
        fn drop(&mut self) {
            // Closing the server handle disconnects the pipe but leaves data
            // already written in the client's read buffer, so a final
            // `session-ended` event is not discarded by `DisconnectNamedPipe`.
            // SAFETY: the handle is owned by this connection.
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }

    impl Read for Connection {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if buffer.is_empty() {
                return Ok(0);
            }
            loop {
                if self.shutdown.load(Ordering::Acquire) {
                    return Ok(0);
                }
                let mut available = 0_u32;
                // SAFETY: the handle is live and the out pointer is valid.
                let ok = unsafe {
                    PeekNamedPipe(
                        self.handle,
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        &mut available,
                        std::ptr::null_mut(),
                    )
                };
                if ok == 0 {
                    return Ok(0);
                }
                if available == 0 {
                    if self
                        .deadline
                        .is_some_and(|deadline| deadline <= Instant::now())
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "the read deadline expired",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                let wanted = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
                let mut read = 0_u32;
                // SAFETY: the handle is live and the buffer has `wanted` bytes.
                let ok = unsafe {
                    ReadFile(
                        self.handle,
                        buffer.as_mut_ptr(),
                        wanted,
                        &mut read,
                        std::ptr::null_mut(),
                    )
                };
                if ok == 0 {
                    return Ok(0);
                }
                return Ok(usize::try_from(read).unwrap_or(0));
            }
        }
    }

    impl Write for Connection {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            let mut written = 0_u32;
            // SAFETY: the handle is live and the buffer covers `bytes`.
            let ok = unsafe {
                WriteFile(
                    self.handle,
                    buffer.as_ptr(),
                    u32::try_from(buffer.len()).unwrap_or(u32::MAX),
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(usize::try_from(written).unwrap_or(0))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(not(any(unix, windows)))]
compile_error!("the local transport requires a Unix or Windows host");
