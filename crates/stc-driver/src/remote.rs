//! Remote-debug foundations (WL-1.2).
//!
//! The remote transport is an OpenSSH port forward to the runtime's existing
//! loopback `socket` debug endpoint. This module owns the parts that are
//! independent of the DAP translation: validating the SSH target and remote
//! paths, building and quoting remote commands, reading user-only remote
//! files, validating the `stc-remote/1` session descriptor, and comparing the
//! local project source fingerprint with a remote manifest.

#![allow(dead_code)] // Phase 1 foundations; these are wired by the remote launcher.

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use stc_got::digest::{Digest32, GotId};
use stc_got::error::{Error, codes};
use stc_got::json::{self, Json};

/// Session-descriptor protocol spelling.
pub(crate) const REMOTE_PROTOCOL: &str = "stc-remote/1";
/// Session-descriptor file format.
pub(crate) const SESSION_FORMAT: i64 = 1;
/// Default remote work directory (relative to the remote login home).
pub(crate) const DEFAULT_WORK_DIR: &str = "~/stc-wl12";
/// Default remote `stc` program; the operator installs the package on `PATH`
/// or supplies an absolute path.
pub(crate) const DEFAULT_STC: &str = "stc";
/// Session-descriptor file name under the remote work directory.
pub(crate) const DEFAULT_SESSION_FILE: &str = "stc-remote-session.json";
/// SSH connect/command deadline for non-build commands.
pub(crate) const SSH_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum size of the session descriptor or debug endpoint file.
pub(crate) const PRIVATE_FILE_LIMIT: usize = 64 * 1024;

fn remote_error(code: &'static str, message: impl Into<String>) -> Error {
    Error::new(code, message)
}

/// A validated OpenSSH destination (`[user@]host`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteTarget {
    spelling: String,
    user: Option<String>,
    host: String,
}

impl RemoteTarget {
    /// Parses and validates an SSH destination.
    ///
    /// The accepted form is `[user@]host` with DNS-style user/host names. The
    /// first target is the SSH config alias `r2k@ecat`; option injection,
    /// whitespace, control characters, ports, and IPv6 literals are rejected.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-REMOTE-007` for an unsupported or unsafe destination.
    pub(crate) fn parse(text: &str) -> Result<Self, Error> {
        let text = text.trim();
        if text.is_empty() {
            return Err(remote_error(
                codes::REMOTE_UNSUPPORTED,
                "a remote target is required (`user@host` or an SSH config alias)",
            ));
        }
        if text.chars().any(char::is_whitespace) || text.chars().any(char::is_control) {
            return Err(remote_error(
                codes::REMOTE_UNSUPPORTED,
                "a remote target must not contain whitespace or control characters",
            ));
        }
        let mut parts = text.split('@');
        let (user, host) = match (parts.next(), parts.next(), parts.next()) {
            (Some(host), None, None) => (None, host),
            (Some(user), Some(host), None) => (Some(user), host),
            _ => {
                return Err(remote_error(
                    codes::REMOTE_UNSUPPORTED,
                    "a remote target must be `[user@]host` with at most one `@`",
                ));
            }
        };
        if host.is_empty() || user.is_some_and(str::is_empty) {
            return Err(remote_error(
                codes::REMOTE_UNSUPPORTED,
                "a remote target must have a non-empty user and host",
            ));
        }
        for (label, part) in [("user", user), ("host", Some(host))] {
            let Some(part) = part else {
                continue;
            };
            if part.starts_with('-') {
                return Err(remote_error(
                    codes::REMOTE_UNSUPPORTED,
                    format!("the remote {label} must not start with `-`"),
                ));
            }
            if !part
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
            {
                return Err(remote_error(
                    codes::REMOTE_UNSUPPORTED,
                    format!(
                        "the remote {label} `{part}` must contain only ASCII letters, digits, `.`, `_`, or `-`"
                    ),
                ));
            }
        }
        Ok(Self {
            spelling: text.to_owned(),
            user: user.map(str::to_owned),
            host: host.to_owned(),
        })
    }

    /// Returns the original destination passed to `ssh`.
    #[must_use]
    pub(crate) fn spelling(&self) -> &str {
        &self.spelling
    }

    /// Returns the remote user, when the target names one.
    #[must_use]
    pub(crate) fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    /// Returns the remote host or SSH config alias.
    #[must_use]
    pub(crate) fn host(&self) -> &str {
        &self.host
    }
}

/// A validated remote path.
///
/// Paths are either Unix-absolute (`/home/r2k/...`) or home-relative
/// (`~/stc-wl12/...`). `..`, `.` components, backslashes, and control
/// characters are rejected. The launcher resolves a home-relative path using
/// the remote login home before issuing file commands.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RemotePath(String);

impl RemotePath {
    /// Parses a home-relative or absolute remote path.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-REMOTE-005` for an empty, relative, or unsafe path.
    pub(crate) fn parse(text: &str) -> Result<Self, Error> {
        let text = text.trim();
        if text.is_empty() {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                "a remote path must not be empty",
            ));
        }
        if text.chars().any(char::is_control) || text.contains('\\') {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                format!("the remote path `{text}` contains an unsupported character"),
            ));
        }
        if text != "~" && !text.starts_with("~/") && !text.starts_with('/') {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                format!("the remote path `{text}` must be absolute or start with `~/`"),
            ));
        }
        let body = text.strip_prefix("~/").unwrap_or(text);
        let components = body.strip_prefix('/').unwrap_or(body);
        if components
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
        {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                format!("the remote path `{text}` must not contain empty, `.` or `..` components"),
            ));
        }
        Ok(Self(text.to_owned()))
    }

    /// Parses an absolute remote path (as written in the session descriptor).
    ///
    /// # Errors
    ///
    /// Returns `STC-E-REMOTE-005` when the path is not Unix-absolute.
    pub(crate) fn parse_absolute(text: &str) -> Result<Self, Error> {
        let path = Self::parse(text)?;
        if !path.is_absolute() {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                format!("the remote path `{text}` must be absolute in the session descriptor"),
            ));
        }
        Ok(path)
    }

    /// Returns the original path spelling.
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns `true` for a Unix-absolute path.
    #[must_use]
    pub(crate) fn is_absolute(&self) -> bool {
        self.0.starts_with('/')
    }

    /// Returns `true` for a home-relative path.
    #[must_use]
    pub(crate) fn is_home_relative(&self) -> bool {
        self.0 == "~" || self.0.starts_with("~/")
    }

    /// Appends a path component or relative path.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-REMOTE-005` when the joined path is unsafe.
    pub(crate) fn join(&self, child: &str) -> Result<Self, Error> {
        if child.is_empty() || child.starts_with('/') {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                "the appended remote path must be a non-empty relative path",
            ));
        }
        let base = self.0.trim_end_matches('/');
        let joined = format!("{base}/{child}");
        Self::parse(&joined)
    }

    /// Returns `true` when `self` is inside `parent`.
    #[must_use]
    pub(crate) fn is_within(&self, parent: &RemotePath) -> bool {
        if self.0 == parent.0 {
            return true;
        }
        self.0
            .strip_prefix(&parent.0)
            .is_some_and(|rest| rest.starts_with('/'))
    }
}

/// A remote `stc` program: a bare command name or a remote path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteProgram(RemotePath);

impl RemoteProgram {
    /// Parses a bare command name or a remote path.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-REMOTE-007` for an unsafe or empty program.
    pub(crate) fn parse(text: &str) -> Result<Self, Error> {
        let text = text.trim();
        if text.is_empty() {
            return Err(remote_error(
                codes::REMOTE_UNSUPPORTED,
                "the remote `stc` program must not be empty",
            ));
        }
        if text.contains('/') || text.starts_with('~') {
            return RemotePath::parse(text).map(Self);
        }
        if text.starts_with('-')
            || !text
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
        {
            return Err(remote_error(
                codes::REMOTE_UNSUPPORTED,
                format!("the remote `stc` program `{text}` is not a safe command name or path"),
            ));
        }
        // A bare command is represented as a relative path internally; it is
        // never joined with the work directory.
        Ok(Self(RemotePath(text.to_owned())))
    }

    /// Returns the program spelling.
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Attach-time deployment choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RemoteDeploy {
    /// Ask the user when the sources drifted.
    #[default]
    Ask,
    /// Deploy an online change.
    Online,
    /// Stop, install, and restart (cold download).
    Cold,
    /// Never deploy; attach only when the source fingerprint matches.
    Never,
}

impl RemoteDeploy {
    /// Parses the launch configuration spelling.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-REMOTE-004` for an unknown value.
    pub(crate) fn parse(text: Option<&str>) -> Result<Self, Error> {
        match text.unwrap_or("ask").trim() {
            "ask" => Ok(Self::Ask),
            "online" => Ok(Self::Online),
            "cold" => Ok(Self::Cold),
            "never" => Ok(Self::Never),
            other => Err(remote_error(
                codes::REMOTE_DEPLOY,
                format!("`remoteDeploy` must be ask, online, cold, or never; got `{other}`"),
            )),
        }
    }
}

/// Validated remote launch/attach settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteSettings {
    /// SSH destination.
    pub(crate) target: RemoteTarget,
    /// Remote work directory.
    pub(crate) work_dir: RemotePath,
    /// Remote `stc` program.
    pub(crate) stc: RemoteProgram,
    /// Remote session-descriptor path.
    pub(crate) session: RemotePath,
    /// Attach-time deployment choice.
    pub(crate) deploy: RemoteDeploy,
}

impl RemoteSettings {
    /// Builds settings from optional launch-configuration strings.
    ///
    /// # Errors
    ///
    /// Returns a remote diagnostic when a target, path, or deployment value is
    /// invalid, or when the session file is outside the work directory.
    pub(crate) fn from_strings(
        target: &str,
        work_dir: Option<&str>,
        stc: Option<&str>,
        session: Option<&str>,
        deploy: Option<&str>,
    ) -> Result<Self, Error> {
        let target = RemoteTarget::parse(target)?;
        let work_dir = RemotePath::parse(work_dir.unwrap_or(DEFAULT_WORK_DIR))?;
        let stc = RemoteProgram::parse(stc.unwrap_or(DEFAULT_STC))?;
        let session = match session {
            Some(text) => RemotePath::parse(text)?,
            None => work_dir.join(DEFAULT_SESSION_FILE)?,
        };
        if (work_dir.is_absolute() || work_dir.is_home_relative())
            && (session.is_absolute() || session.is_home_relative())
            && !session.is_within(&work_dir)
        {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                format!(
                    "the remote session file `{}` must be under the work directory `{}`",
                    session.as_str(),
                    work_dir.as_str()
                ),
            ));
        }
        Ok(Self {
            target,
            work_dir,
            stc,
            session,
            deploy: RemoteDeploy::parse(deploy)?,
        })
    }
}

/// One `stc-remote/1` session descriptor written by the host-side `stc serve`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteSession {
    /// Remote hostname for display.
    pub(crate) host: String,
    /// Remote work directory (absolute).
    pub(crate) work_dir: RemotePath,
    /// Remote project manifest (absolute).
    pub(crate) project: RemotePath,
    /// Remote loadable module (absolute).
    pub(crate) module: RemotePath,
    /// Remote GOT manifest (absolute).
    pub(crate) manifest: RemotePath,
    /// Remote debug endpoint file (absolute).
    pub(crate) debug_endpoint_file: RemotePath,
    /// Remote control endpoint (absolute).
    pub(crate) control_endpoint: RemotePath,
    /// Remote user-only control-nonce file (absolute).
    pub(crate) control_nonce_file: RemotePath,
    /// Build identity.
    pub(crate) build_id: String,
    /// Active generation.
    pub(crate) generation: u64,
    /// Production source fingerprint.
    pub(crate) source_fingerprint: String,
    /// `debug` or `release`.
    pub(crate) mode: String,
    /// Remote `stc` version.
    pub(crate) stc_version: String,
    /// Remote host process identity.
    pub(crate) pid: u32,
}

impl RemoteSession {
    /// Parses and validates a session descriptor.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-REMOTE-005` for malformed or inconsistent descriptors.
    pub(crate) fn parse(text: &str) -> Result<Self, Error> {
        let value = json::parse_protocol(text).map_err(|error| {
            remote_error(
                codes::REMOTE_SESSION,
                format!(
                    "the remote session descriptor is not valid JSON: {}",
                    error.message
                ),
            )
        })?;
        let object = value.as_object().ok_or_else(|| {
            remote_error(
                codes::REMOTE_SESSION,
                "the remote session descriptor is not an object",
            )
        })?;
        let string = |key: &str| -> Result<String, Error> {
            object
                .get(key)
                .and_then(Json::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    remote_error(
                        codes::REMOTE_SESSION,
                        format!("the remote session descriptor has no string `{key}`"),
                    )
                })
        };
        let number = |key: &str| -> Result<i128, Error> {
            object.get(key).and_then(Json::as_number).ok_or_else(|| {
                remote_error(
                    codes::REMOTE_SESSION,
                    format!("the remote session descriptor has no number `{key}`"),
                )
            })
        };
        let unsigned = |key: &str| -> Result<u64, Error> {
            u64::try_from(number(key)?).map_err(|_| {
                remote_error(
                    codes::REMOTE_SESSION,
                    format!("the remote session descriptor `{key}` is out of range"),
                )
            })
        };
        if number("format")? != i128::from(SESSION_FORMAT) {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                "unsupported remote session descriptor format",
            ));
        }
        if string("protocol")? != REMOTE_PROTOCOL {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                "unsupported remote session descriptor protocol",
            ));
        }
        let session = Self {
            host: string("host")?,
            work_dir: RemotePath::parse_absolute(&string("work_dir")?)?,
            project: RemotePath::parse_absolute(&string("project")?)?,
            module: RemotePath::parse_absolute(&string("module")?)?,
            manifest: RemotePath::parse_absolute(&string("manifest")?)?,
            debug_endpoint_file: RemotePath::parse_absolute(&string("debug_endpoint_file")?)?,
            control_endpoint: RemotePath::parse_absolute(&string("control_endpoint")?)?,
            control_nonce_file: RemotePath::parse_absolute(&string("control_nonce_file")?)?,
            build_id: string("build_id")?,
            generation: unsigned("generation")?,
            source_fingerprint: string("source_fingerprint")?,
            mode: string("mode")?,
            stc_version: string("stc_version")?,
            pid: u32::try_from(unsigned("pid")?).map_err(|_| {
                remote_error(codes::REMOTE_SESSION, "the remote `pid` is out of range")
            })?,
        };
        session.validate()?;
        Ok(session)
    }

    /// Serializes the descriptor as compact, key-sorted JSON with one newline.
    #[must_use]
    pub(crate) fn to_json_text(&self) -> String {
        format!("{}\n", self.to_json().render_compact())
    }

    /// Returns the descriptor as a sorted JSON object.
    #[must_use]
    pub(crate) fn to_json(&self) -> Json {
        Json::object([
            ("build_id".into(), Json::string(self.build_id.clone())),
            (
                "control_endpoint".into(),
                Json::string(self.control_endpoint.as_str()),
            ),
            (
                "control_nonce_file".into(),
                Json::string(self.control_nonce_file.as_str()),
            ),
            (
                "debug_endpoint_file".into(),
                Json::string(self.debug_endpoint_file.as_str()),
            ),
            (
                "format".into(),
                Json::unsigned(u64::try_from(SESSION_FORMAT).unwrap_or(1)),
            ),
            ("generation".into(), Json::unsigned(self.generation)),
            ("host".into(), Json::string(self.host.clone())),
            ("manifest".into(), Json::string(self.manifest.as_str())),
            ("mode".into(), Json::string(self.mode.clone())),
            ("module".into(), Json::string(self.module.as_str())),
            ("pid".into(), Json::unsigned(u64::from(self.pid))),
            ("project".into(), Json::string(self.project.as_str())),
            ("protocol".into(), Json::string(REMOTE_PROTOCOL)),
            (
                "source_fingerprint".into(),
                Json::string(self.source_fingerprint.clone()),
            ),
            ("stc_version".into(), Json::string(self.stc_version.clone())),
            ("work_dir".into(), Json::string(self.work_dir.as_str())),
        ])
    }

    /// Validates cross-field invariants.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-REMOTE-005` for an empty host/version, a bad identity,
    /// an unsupported mode, a zero pid, or a path outside the work directory.
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if !self.work_dir.is_absolute() {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                "the remote session descriptor work_dir must be absolute",
            ));
        }
        if self.host.trim().is_empty() {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                "the remote session descriptor has an empty host",
            ));
        }
        if self.stc_version.trim().is_empty() {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                "the remote session descriptor has an empty stc_version",
            ));
        }
        if self.pid == 0 {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                "the remote session descriptor has a zero pid",
            ));
        }
        if self.mode != "debug" && self.mode != "release" {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                format!(
                    "the remote session descriptor mode `{}` is not debug or release",
                    self.mode
                ),
            ));
        }
        GotId::parse(&self.build_id).map_err(|error| {
            remote_error(
                codes::REMOTE_SESSION,
                format!("the remote session build_id is invalid: {}", error.message),
            )
        })?;
        Digest32::parse(&self.source_fingerprint).map_err(|error| {
            remote_error(
                codes::REMOTE_SESSION,
                format!(
                    "the remote session source_fingerprint is invalid: {}",
                    error.message
                ),
            )
        })?;
        for (label, path) in [
            ("project", &self.project),
            ("module", &self.module),
            ("manifest", &self.manifest),
            ("debug_endpoint_file", &self.debug_endpoint_file),
            ("control_endpoint", &self.control_endpoint),
            ("control_nonce_file", &self.control_nonce_file),
        ] {
            if !path.is_absolute() {
                return Err(remote_error(
                    codes::REMOTE_SESSION,
                    format!("the remote session `{label}` path must be absolute"),
                ));
            }
            if !path.is_within(&self.work_dir) {
                return Err(remote_error(
                    codes::REMOTE_SESSION,
                    format!(
                        "the remote session `{label}` path `{}` is outside the work directory `{}`",
                        path.as_str(),
                        self.work_dir.as_str()
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// Compares the local and remote source fingerprints.
///
/// # Errors
///
/// Returns `STC-E-REMOTE-003` when either fingerprint is invalid or they
/// differ.
pub(crate) fn compare_source_fingerprints(local: &str, remote: &str) -> Result<(), Error> {
    let local = Digest32::parse(local).map_err(|error| {
        remote_error(
            codes::REMOTE_SOURCE_MISMATCH,
            format!("the local source fingerprint is invalid: {}", error.message),
        )
    })?;
    let remote = Digest32::parse(remote).map_err(|error| {
        remote_error(
            codes::REMOTE_SOURCE_MISMATCH,
            format!(
                "the remote source fingerprint is invalid: {}",
                error.message
            ),
        )
    })?;
    if local != remote {
        return Err(remote_error(
            codes::REMOTE_SOURCE_MISMATCH,
            format!(
                "the remote build source fingerprint {} does not match the local project {}",
                remote.to_hex(),
                local.to_hex()
            ),
        ));
    }
    Ok(())
}

/// Computes the canonical production source fingerprint for a local project.
///
/// # Errors
///
/// Returns `STC-E-REMOTE-003` when the project cannot be loaded or a source
/// file cannot be read.
pub(crate) fn local_source_fingerprint(project: &Path) -> Result<String, Error> {
    let graph = stc_manifest::load_project_readonly(project, &stc_manifest::LoadOptions::default())
        .map_err(|error| {
            let detail = error.diagnostics.first().map_or_else(
                || "the project manifest could not be loaded".to_owned(),
                |diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message),
            );
            remote_error(codes::REMOTE_SOURCE_MISMATCH, detail)
        })?;
    let mut sources = Vec::new();
    for package in &graph.packages {
        for source in &package.sources {
            let bytes = std::fs::read(&source.path).map_err(|error| {
                remote_error(
                    codes::REMOTE_SOURCE_MISMATCH,
                    format!(
                        "cannot read the project source `{}`: {error}",
                        source.path.display()
                    ),
                )
            })?;
            sources.push((package.name.clone(), source.relative.clone(), bytes));
        }
    }
    let fingerprint =
        stc_codegen::module::project_source_fingerprint(&sources).map_err(|error| {
            remote_error(
                codes::REMOTE_SOURCE_MISMATCH,
                format!(
                    "cannot compute the project source fingerprint: {}",
                    error.message
                ),
            )
        })?;
    Ok(fingerprint.to_hex())
}

/// Quotes one remote command argument for a POSIX shell.
#[must_use]
pub(crate) fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-./:@%+=,".contains(character))
    {
        return value.to_owned();
    }
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for character in value.chars() {
        if character == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(character);
        }
    }
    quoted.push('\'');
    quoted
}

/// Builds one safely quoted remote shell command from an argument vector.
///
/// # Errors
///
/// Returns `STC-E-REMOTE-002` for an empty vector or an argument containing a
/// control character.
pub(crate) fn remote_command(argv: &[String]) -> Result<String, Error> {
    if argv.is_empty() {
        return Err(remote_error(
            codes::REMOTE_COMMAND,
            "a remote command must have at least one argument",
        ));
    }
    if argv
        .iter()
        .any(|argument| argument.chars().any(|character| character == '\0'))
    {
        return Err(remote_error(
            codes::REMOTE_COMMAND,
            "a remote command argument must not contain a NUL character",
        ));
    }
    Ok(argv
        .iter()
        .map(|argument| shell_quote(argument))
        .collect::<Vec<_>>()
        .join(" "))
}

/// Returns the `ssh` argument vector for one non-interactive remote command.
#[must_use]
pub(crate) fn ssh_arguments(target: &RemoteTarget, command: &str) -> Vec<String> {
    vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=yes".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
        "-o".into(),
        "LogLevel=ERROR".into(),
        "-T".into(),
        "--".into(),
        target.spelling().to_owned(),
        command.to_owned(),
    ]
}

/// Output of one non-interactive SSH command.
#[derive(Clone, Debug)]
pub(crate) struct SshOutput {
    /// Process exit code, or `None` when terminated by a signal.
    pub(crate) status: Option<i32>,
    /// Captured standard output (lossy UTF-8).
    pub(crate) stdout: String,
    /// Captured standard error (lossy UTF-8).
    pub(crate) stderr: String,
}

impl SshOutput {
    /// Returns `true` for exit code 0.
    #[must_use]
    pub(crate) fn success(&self) -> bool {
        self.status == Some(0)
    }
}

/// Runs remote commands through the system OpenSSH client.
#[derive(Clone, Debug)]
pub(crate) struct SshRunner {
    program: String,
}

impl Default for SshRunner {
    fn default() -> Self {
        Self::new("ssh")
    }
}

impl SshRunner {
    /// Creates a runner for an alternate `ssh` program (tests use a fake).
    #[must_use]
    pub(crate) fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// Runs one remote command and captures its output.
    ///
    /// The command is not required to succeed; callers inspect
    /// [`SshOutput::status`] for expected failures.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-REMOTE-001` when SSH cannot start or the command does
    /// not finish within `timeout`, or `STC-E-REMOTE-002` for an invalid
    /// command.
    pub(crate) fn run(
        &self,
        target: &RemoteTarget,
        argv: &[String],
        timeout: Duration,
    ) -> Result<SshOutput, Error> {
        let command = remote_command(argv)?;
        let mut child = Command::new(&self.program)
            .args(ssh_arguments(target, &command))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                remote_error(
                    codes::REMOTE_TRANSPORT,
                    format!(
                        "cannot start the SSH client `{}` for `{}`: {error}",
                        self.program,
                        target.spelling()
                    ),
                )
            })?;
        let captured = wait_with_timeout(&mut child, timeout)?;
        Ok(SshOutput {
            status: captured.status,
            stdout: String::from_utf8_lossy(&captured.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&captured.stderr).into_owned(),
        })
    }

    /// Returns the remote login home directory as an absolute path.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-REMOTE-001` when the home directory cannot be read.
    pub(crate) fn remote_home(&self, target: &RemoteTarget) -> Result<RemotePath, Error> {
        let argv = vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "printf '%s' \"$HOME\"".to_owned(),
        ];
        let output = self.run(target, &argv, SSH_COMMAND_TIMEOUT)?;
        if !output.success() {
            return Err(remote_error(
                codes::REMOTE_TRANSPORT,
                format!(
                    "cannot read the remote home directory through `{}`{}",
                    target.spelling(),
                    command_detail(&output.stderr)
                ),
            ));
        }
        RemotePath::parse_absolute(output.stdout.trim())
    }

    /// Reads a user-only remote file, rejecting non-private files.
    ///
    /// The file must be a regular file, owned by the remote login user, with no
    /// group or other permission bits, and no larger than
    /// [`PRIVATE_FILE_LIMIT`].
    ///
    /// # Errors
    ///
    /// Returns `STC-E-REMOTE-005` when the file is not private or the content
    /// is not UTF-8; `STC-E-REMOTE-001` for an SSH failure.
    pub(crate) fn read_private_file(
        &self,
        target: &RemoteTarget,
        path: &RemotePath,
    ) -> Result<String, Error> {
        if !path.is_absolute() {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                "a private remote file must have an absolute path",
            ));
        }
        let script = private_file_script(path.as_str());
        let argv = vec!["sh".to_owned(), "-c".to_owned(), script];
        let output = self.run(target, &argv, SSH_COMMAND_TIMEOUT)?;
        match output.status {
            Some(0) => String::from_utf8(output.stdout.into_bytes()).map_err(|_| {
                remote_error(
                    codes::REMOTE_SESSION,
                    format!("the remote file `{}` is not valid UTF-8", path.as_str()),
                )
            }),
            Some(3) => Err(remote_error(
                codes::REMOTE_SESSION,
                format!("the remote file `{}` is not user-only", path.as_str()),
            )),
            Some(4) => Err(remote_error(
                codes::REMOTE_SESSION,
                format!(
                    "the remote file `{}` exceeds the {} byte limit",
                    path.as_str(),
                    PRIVATE_FILE_LIMIT
                ),
            )),
            _ => Err(remote_error(
                codes::REMOTE_SESSION,
                format!(
                    "cannot read the remote file `{}`{}",
                    path.as_str(),
                    command_detail(&output.stderr)
                ),
            )),
        }
    }
}

fn private_file_script(path: &str) -> String {
    format!(
        "set -eu\n\
         p={path}\n\
         test -f \"$p\"\n\
         size=$(wc -c < \"$p\")\n\
         if [ \"$size\" -gt {limit} ]; then\n\
         \x20 echo 'remote file is too large' >&2\n\
         \x20 exit 4\n\
         fi\n\
         mode=$(stat -c '%a' -- \"$p\")\n\
         uid=$(stat -c '%u' -- \"$p\")\n\
         test \"$uid\" = \"$(id -u)\"\n\
         case \"$mode\" in\n\
         \x20 400|500|600|700) ;;\n\
         \x20 *) echo 'remote file is not user-only' >&2; exit 3 ;;\n\
         esac\n\
         cat -- \"$p\"",
        path = shell_quote(path),
        limit = PRIVATE_FILE_LIMIT
    )
}

fn command_detail(stderr: &str) -> String {
    let detail = stderr.trim();
    if detail.is_empty() {
        String::new()
    } else {
        format!(": {detail}")
    }
}

struct CapturedOutput {
    status: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Result<CapturedOutput, Error> {
    let mut stdout = child.stdout.take().ok_or_else(|| {
        remote_error(
            codes::REMOTE_TRANSPORT,
            "the SSH stdout pipe is unavailable",
        )
    })?;
    let mut stderr = child.stderr.take().ok_or_else(|| {
        remote_error(
            codes::REMOTE_TRANSPORT,
            "the SSH stderr pipe is unavailable",
        )
    })?;
    let stdout_handle = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stdout.read_to_end(&mut buffer);
        buffer
    });
    let stderr_handle = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stderr.read_to_end(&mut buffer);
        buffer
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_handle.join();
                    let _ = stderr_handle.join();
                    return Err(remote_error(
                        codes::REMOTE_TRANSPORT,
                        format!(
                            "the remote command did not finish within {} ms",
                            timeout.as_millis()
                        ),
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_handle.join();
                let _ = stderr_handle.join();
                return Err(remote_error(
                    codes::REMOTE_TRANSPORT,
                    format!("cannot wait for the remote command: {error}"),
                ));
            }
        }
    };
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    Ok(CapturedOutput {
        status,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn target() -> RemoteTarget {
        RemoteTarget::parse("r2k@ecat").expect("target")
    }

    fn path(text: &str) -> RemotePath {
        RemotePath::parse(text).expect("path")
    }

    fn sample_session() -> RemoteSession {
        RemoteSession {
            host: "ecat-host".into(),
            work_dir: path("/home/r2k/stc-wl12"),
            project: path("/home/r2k/stc-wl12/stc.toml"),
            module: path("/home/r2k/stc-wl12/target/stc/debug/demo.so"),
            manifest: path("/home/r2k/stc-wl12/target/stc/debug/demo.stc-got.json"),
            debug_endpoint_file: path("/home/r2k/stc-wl12/stc-debug.endpoint"),
            control_endpoint: path("/home/r2k/stc-wl12/stc-control.sock"),
            control_nonce_file: path("/home/r2k/stc-wl12/stc-control.nonce"),
            build_id: format!("0x{}", "11".repeat(16)),
            generation: 1,
            source_fingerprint: format!("0x{}", "22".repeat(32)),
            mode: "debug".into(),
            stc_version: "3.0.0".into(),
            pid: 1234,
        }
    }

    fn temp_project(name: &str) -> std::path::PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "stc-remote-{}-{name}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(directory.join("src")).expect("create project");
        std::fs::write(
            directory.join("stc.toml"),
            "schema = 1\n\n[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\
             language = \"stc-2013-oop-utf8-v1\"\n\n[project]\nsources = [\"src/**/*.st\"]\n",
        )
        .expect("write manifest");
        std::fs::write(
            directory.join("src/main.st"),
            "PROGRAM Main\n  ;\nEND_PROGRAM\n",
        )
        .expect("write source");
        directory
    }

    #[test]
    fn parses_safe_ssh_targets_and_rejects_option_injection() {
        assert_eq!(target().spelling(), "r2k@ecat");
        assert_eq!(target().user(), Some("r2k"));
        assert_eq!(target().host(), "ecat");
        assert_eq!(
            RemoteTarget::parse("ecat").expect("alias").spelling(),
            "ecat"
        );
        for text in [
            "",
            "-oProxyCommand=calc",
            "r2k@ecat;reboot",
            "r2k@@ecat",
            "r2k@ecat extra",
            "r2k@ecat\nrm",
            "r2k@ecat:22",
        ] {
            let error = RemoteTarget::parse(text).expect_err(text);
            assert_eq!(error.code, codes::REMOTE_UNSUPPORTED, "{text}: {error}");
        }
    }

    #[test]
    fn remote_paths_are_absolute_or_home_relative_and_stay_safe() {
        assert!(path("/home/r2k/stc-wl12").is_absolute());
        assert!(path("~/stc-wl12").is_home_relative());
        assert_eq!(
            path("~/stc-wl12")
                .join("stc-remote-session.json")
                .expect("join")
                .as_str(),
            "~/stc-wl12/stc-remote-session.json"
        );
        for text in [
            "",
            "stc-wl12",
            "~/stc-wl12/../etc",
            "~/stc-wl12/./x",
            "/home//r2k",
            "~/stc-wl12\\x",
        ] {
            assert!(RemotePath::parse(text).is_err(), "{text}");
        }
        assert!(path("/home/r2k/stc-wl12/x").is_within(&path("/home/r2k/stc-wl12")));
        assert!(!path("/home/r2k/stc-wl12x").is_within(&path("/home/r2k/stc-wl12")));
    }

    #[test]
    fn settings_default_to_the_documented_remote_workspace() {
        let settings =
            RemoteSettings::from_strings("r2k@ecat", None, None, None, None).expect("settings");
        assert_eq!(settings.work_dir.as_str(), "~/stc-wl12");
        assert_eq!(settings.stc.as_str(), "stc");
        assert_eq!(
            settings.session.as_str(),
            "~/stc-wl12/stc-remote-session.json"
        );
        assert_eq!(settings.deploy, RemoteDeploy::Ask);
        assert_eq!(
            RemoteDeploy::parse(Some("online")).expect("online"),
            RemoteDeploy::Online
        );
        assert_eq!(
            RemoteDeploy::parse(Some("cold")).expect("cold"),
            RemoteDeploy::Cold
        );
        assert_eq!(
            RemoteDeploy::parse(Some("never")).expect("never"),
            RemoteDeploy::Never
        );
        assert!(RemoteDeploy::parse(Some("always")).is_err());
    }

    #[test]
    fn session_settings_must_stay_under_the_work_directory() {
        let error = RemoteSettings::from_strings(
            "r2k@ecat",
            Some("/home/r2k/stc-wl12"),
            None,
            Some("/tmp/session.json"),
            None,
        )
        .expect_err("outside session");
        assert_eq!(error.code, codes::REMOTE_SESSION);
    }

    #[test]
    fn remote_commands_quote_every_argument() {
        assert_eq!(shell_quote("stc"), "stc");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(
            remote_command(&[
                "stc".into(),
                "serve".into(),
                "--work-dir".into(),
                "/home/r2k/a b".into()
            ])
            .expect("command"),
            "stc serve --work-dir '/home/r2k/a b'"
        );
        assert!(remote_command(&["a\nb".into()]).is_ok());
        assert!(remote_command(&["a\0b".into()]).is_err());
        assert!(remote_command(&[]).is_err());
    }

    #[test]
    fn ssh_arguments_enforce_batch_mode_and_known_hosts() {
        let arguments = ssh_arguments(&target(), "stc serve");
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["-o", "BatchMode=yes"])
        );
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["-o", "StrictHostKeyChecking=yes"])
        );
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["-o", "ConnectTimeout=10"])
        );
        assert!(arguments.iter().any(|argument| argument == "r2k@ecat"));
        assert!(arguments.iter().any(|argument| argument == "stc serve"));
    }

    #[test]
    fn private_file_script_checks_owner_and_mode() {
        let script = private_file_script("/home/r2k/stc-wl12/stc-debug.endpoint");
        assert!(script.contains("test \"$uid\" = \"$(id -u)\""), "{script}");
        assert!(script.contains("400|500|600|700"), "{script}");
        assert!(script.contains("exit 3"), "{script}");
        assert!(script.contains("wc -c"), "{script}");
    }

    #[test]
    fn session_descriptor_round_trips_and_validates() {
        let session = sample_session();
        let text = session.to_json_text();
        assert!(text.ends_with('\n'));
        assert_eq!(RemoteSession::parse(&text).expect("parse"), session);
    }

    #[test]
    fn session_descriptor_rejects_paths_outside_the_work_directory() {
        let mut session = sample_session();
        session.module = path("/tmp/demo.so");
        let error = session.validate().expect_err("outside path");
        assert_eq!(error.code, codes::REMOTE_SESSION);
    }

    #[test]
    fn session_descriptor_rejects_bad_identity_and_mode() {
        let mut session = sample_session();
        session.build_id = "0x12".into();
        assert_eq!(
            session.validate().expect_err("bad build id").code,
            codes::REMOTE_SESSION
        );
        session = sample_session();
        session.mode = "release-debug".into();
        assert_eq!(
            session.validate().expect_err("bad mode").code,
            codes::REMOTE_SESSION
        );
    }

    #[test]
    fn source_fingerprints_must_match() {
        let project = temp_project("fingerprint");
        let first = local_source_fingerprint(&project.join("stc.toml")).expect("fingerprint");
        compare_source_fingerprints(&first, &first).expect("equal fingerprints");
        std::fs::write(
            project.join("src/main.st"),
            "PROGRAM Main\n  ;\n  ;\nEND_PROGRAM\n",
        )
        .expect("edit source");
        let second = local_source_fingerprint(&project.join("stc.toml")).expect("fingerprint");
        assert_ne!(first, second);
        let error = compare_source_fingerprints(&first, &second).expect_err("mismatch");
        assert_eq!(error.code, codes::REMOTE_SOURCE_MISMATCH);
        std::fs::remove_dir_all(&project).expect("clean up");
    }

    #[test]
    fn remote_program_accepts_bare_names_and_paths() {
        assert_eq!(RemoteProgram::parse("stc").expect("bare").as_str(), "stc");
        assert_eq!(
            RemoteProgram::parse("/home/r2k/stc-wl12/bin/stc")
                .expect("absolute")
                .as_str(),
            "/home/r2k/stc-wl12/bin/stc"
        );
        assert_eq!(
            RemoteProgram::parse("~/stc-wl12/bin/stc")
                .expect("home-relative")
                .as_str(),
            "~/stc-wl12/bin/stc"
        );
        for text in ["", "-stc", "a b", "stc;reboot"] {
            assert!(RemoteProgram::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn session_descriptor_rejects_malformed_json_and_wrong_protocol() {
        assert!(RemoteSession::parse("not json").is_err());
        let text = sample_session()
            .to_json_text()
            .replace("stc-remote/1", "stc-remote/2");
        let error = RemoteSession::parse(&text).expect_err("protocol");
        assert_eq!(error.code, codes::REMOTE_SESSION);
        let text = sample_session()
            .to_json_text()
            .replace("\"format\":1", "\"format\":2");
        assert!(RemoteSession::parse(&text).is_err());
    }

    #[cfg(target_os = "linux")]
    fn fake_ssh(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!(
            "stc-remote-ssh-{}-{name}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        ));
        std::fs::create_dir_all(&directory).expect("create fake ssh directory");
        let script = directory.join("ssh");
        std::fs::write(
            &script,
            "#!/bin/sh\nlast=\nfor arg in \"$@\"; do last=\"$arg\"; done\nexec /bin/sh -c \"$last\"\n",
        )
        .expect("write fake ssh");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake ssh");
        (directory, script)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ssh_runner_executes_remote_commands_and_reads_private_files() {
        use std::os::unix::fs::PermissionsExt;
        let (directory, script) = fake_ssh("runner");
        let runner = SshRunner::new(script.to_string_lossy().into_owned());
        let output = runner
            .run(
                &target(),
                &["printf".into(), "%s".into(), "hello".into()],
                Duration::from_secs(5),
            )
            .expect("run");
        assert_eq!(output.status, Some(0));
        assert_eq!(output.stdout, "hello");
        let home = runner.remote_home(&target()).expect("home");
        assert!(home.is_absolute(), "{home:?}");

        let nonce = directory.join("nonce");
        std::fs::write(&nonce, b"secret\n").expect("write nonce");
        std::fs::set_permissions(&nonce, std::fs::Permissions::from_mode(0o600)).expect("mode");
        let path = RemotePath::parse_absolute(&nonce.to_string_lossy()).expect("path");
        assert_eq!(
            runner.read_private_file(&target(), &path).expect("read"),
            "secret"
        );
        std::fs::set_permissions(&nonce, std::fs::Permissions::from_mode(0o644)).expect("mode");
        let error = runner
            .read_private_file(&target(), &path)
            .expect_err("group access");
        assert_eq!(error.code, codes::REMOTE_SESSION);
        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ssh_runner_reports_timeouts_and_missing_programs() {
        let (directory, script) = fake_ssh("timeout");
        let runner = SshRunner::new(script.to_string_lossy().into_owned());
        let error = runner
            .run(
                &target(),
                &["sleep".into(), "1".into()],
                Duration::from_millis(20),
            )
            .expect_err("timeout");
        assert_eq!(error.code, codes::REMOTE_TRANSPORT);
        let missing = SshRunner::new("/nonexistent/stc-ssh");
        let error = missing
            .run(&target(), &["true".into()], Duration::from_secs(1))
            .expect_err("missing ssh");
        assert_eq!(error.code, codes::REMOTE_TRANSPORT);
        std::fs::remove_dir_all(&directory).expect("clean up");
    }
}
