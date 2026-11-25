//! Explicit setup mode: the managed-master commissioning session (D-260).
//!
//! Setup mode is the only state in which WL-7 changes hardware. It spans
//! several CLI invocations: it is entered and left explicitly, exposes a
//! `status` query, and records a session identity so a second engineering
//! session is rejected rather than silently sharing state.
//!
//! The session descriptor is the claim of record. The master reservation
//! itself belongs to the operating system and lapses when the owning process
//! exits, so an interrupted session (Ctrl-C, SSH drop, crash) leaves a stale
//! record that `setup leave` clears before the next `setup enter`. Nothing here
//! runs a PLC cycle, writes process outputs or touches project files.

use std::path::Path;

use stc_got::json::{self, Json};

use super::diagnostics::{EngineeringDiagnostic, codes};

/// Format version of the setup session descriptor.
pub const SETUP_FORMAT: u32 = 1;

/// Protocol name of the setup session descriptor.
pub const SETUP_PROTOCOL: &str = "stc-ethercat-setup/1";

/// Default session descriptor path, relative to the working directory.
pub const DEFAULT_SESSION_FILE: &str = ".stc/ethercat-setup.json";

/// One setup-mode session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupSession {
    /// Stable session identity, unique per entry.
    pub session: String,
    /// Process that entered the session.
    pub pid: u32,
    /// Managed master index the session holds.
    pub master: u16,
    /// Entry time, in Unix seconds.
    pub started_unix: u64,
}

impl SetupSession {
    /// Creates a session identity for `pid` entering at `started_unix`.
    #[must_use]
    pub fn new(pid: u32, master: u16, started_unix: u64) -> Self {
        Self {
            session: format!("setup-{pid}-{started_unix}"),
            pid,
            master,
            started_unix,
        }
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("format".into(), Json::unsigned(u64::from(SETUP_FORMAT))),
            ("protocol".into(), Json::string(SETUP_PROTOCOL)),
            ("session".into(), Json::string(self.session.as_str())),
            ("pid".into(), Json::unsigned(u64::from(self.pid))),
            ("master".into(), Json::unsigned(u64::from(self.master))),
            ("started_unix".into(), Json::unsigned(self.started_unix)),
        ])
    }

    /// Deterministic descriptor text.
    #[must_use]
    pub fn to_text(&self) -> String {
        self.to_json().render()
    }

    /// Parses a session descriptor.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-045` for a malformed or unsupported descriptor.
    pub fn parse(text: &str) -> Result<Self, EngineeringDiagnostic> {
        let value = json::parse(text).map_err(|error| {
            unreadable(
                "setup.status",
                &format!("not valid JSON: {}", error.message),
            )
        })?;
        let object = value
            .as_object()
            .ok_or_else(|| unreadable("setup.status", "a non-object descriptor"))?;
        if object.get("format").and_then(Json::as_number) != Some(i128::from(SETUP_FORMAT)) {
            return Err(unreadable(
                "setup.status",
                "an unsupported descriptor format",
            ));
        }
        if object.get("protocol").and_then(Json::as_str) != Some(SETUP_PROTOCOL) {
            return Err(unreadable(
                "setup.status",
                "an unsupported descriptor protocol",
            ));
        }
        let number = |key: &str| -> Result<u64, EngineeringDiagnostic> {
            let value = object
                .get(key)
                .and_then(Json::as_number)
                .ok_or_else(|| unreadable("setup.status", &format!("no numeric `{key}`")))?;
            u64::try_from(value)
                .map_err(|_| unreadable("setup.status", &format!("`{key}` out of range")))
        };
        let session = object
            .get("session")
            .and_then(Json::as_str)
            .filter(|session| !session.trim().is_empty())
            .ok_or_else(|| unreadable("setup.status", "no session identity"))?;
        Ok(Self {
            session: session.to_owned(),
            pid: u32::try_from(number("pid")?)
                .map_err(|_| unreadable("setup.status", "`pid` out of range"))?,
            master: u16::try_from(number("master")?)
                .map_err(|_| unreadable("setup.status", "`master` out of range"))?,
            started_unix: number("started_unix")?,
        })
    }

    /// Reads the descriptor at `path`.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-045` when the descriptor exists but cannot be
    /// read or parsed.
    pub fn read(
        path: &Path,
        operation: &'static str,
    ) -> Result<Option<Self>, EngineeringDiagnostic> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map(Some).map_err(|mut error| {
                error.operation = operation;
                error
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(unreadable(
                operation,
                &format!("cannot read the descriptor: {error}"),
            )
            .with_source(path.display().to_string())),
        }
    }

    /// Reads the descriptor, reporting a corrupt record as absent.
    ///
    /// This is the recovery path: `setup leave` must clear a descriptor that a
    /// crashed session left behind even when it is no longer parseable.
    #[must_use]
    pub fn read_lossy(path: &Path) -> Option<Self> {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| Self::parse(&text).ok())
    }

    /// Reports whether a descriptor file exists at `path`.
    #[must_use]
    pub fn exists(path: &Path) -> bool {
        path.exists()
    }

    /// Writes the descriptor atomically.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-045` when the descriptor cannot be written.
    pub fn write(&self, path: &Path, operation: &'static str) -> Result<(), EngineeringDiagnostic> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|error| {
                unreadable(
                    operation,
                    &format!("cannot create the session directory: {error}"),
                )
                .with_source(path.display().to_string())
            })?;
        }
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, self.to_text()).map_err(|error| {
            unreadable(operation, &format!("cannot write the descriptor: {error}"))
                .with_source(path.display().to_string())
        })?;
        std::fs::rename(&temporary, path).map_err(|error| {
            unreadable(
                operation,
                &format!("cannot publish the descriptor: {error}"),
            )
            .with_source(path.display().to_string())
        })
    }

    /// Removes the descriptor at `path`, if any.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-045` when the descriptor cannot be removed.
    pub fn remove(path: &Path, operation: &'static str) -> Result<(), EngineeringDiagnostic> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(unreadable(
                operation,
                &format!("cannot remove the descriptor: {error}"),
            )
            .with_source(path.display().to_string())),
        }
    }
}

/// The observed state of setup mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupStatus {
    /// Recorded session, when the descriptor exists and parses.
    pub session: Option<SetupSession>,
    /// Owner the host reports for the master, when it reports one.
    pub master_owner: Option<String>,
    /// `Some(true)` when the recorded session holds the master,
    /// `Some(false)` when another owner holds it, `None` when the host cannot
    /// report an owner.
    pub master_held: Option<bool>,
    /// The other owner that holds the master, when one is reported.
    pub conflict: Option<String>,
    /// Whether an unreadable descriptor was discarded during recovery.
    pub recovered: bool,
}

impl SetupStatus {
    /// Reports whether a session record is active.
    #[must_use]
    pub fn active(&self) -> bool {
        self.session.is_some()
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("active".into(), Json::Bool(self.active())),
            (
                "session".into(),
                Json::optional(self.session.as_ref().map(SetupSession::to_json)),
            ),
            (
                "master_owner".into(),
                Json::optional(
                    self.master_owner
                        .as_ref()
                        .map(|owner| Json::string(owner.as_str())),
                ),
            ),
            (
                "master_held".into(),
                Json::optional(self.master_held.map(Json::Bool)),
            ),
            (
                "conflict".into(),
                Json::optional(
                    self.conflict
                        .as_ref()
                        .map(|owner| Json::string(owner.as_str())),
                ),
            ),
            ("recovered".into(), Json::Bool(self.recovered)),
        ])
    }
}

fn unreadable(operation: &'static str, observed: &str) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::SETUP_SESSION_UNREADABLE,
        operation,
        "run `stc ethercat setup leave` to clear the session record, then enter again",
    )
    .with_expected("a readable setup session descriptor")
    .with_observed(observed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary(name: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("stc-ethercat-setup-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp directory");
        directory.join("session.json")
    }

    #[test]
    fn a_session_round_trips_through_its_descriptor() {
        let session = SetupSession::new(4321, 0, 1_700_000_000);
        assert_eq!(session.session, "setup-4321-1700000000");
        let parsed = SetupSession::parse(&session.to_text()).expect("parse");
        assert_eq!(parsed, session);
        let json = session.to_json().render();
        assert!(json.contains("\"protocol\": \"stc-ethercat-setup/1\""));
        assert!(json.contains("\"master\": 0"));
    }

    #[test]
    fn a_corrupt_descriptor_is_reported_and_recoverable() {
        let path = temporary("corrupt");
        std::fs::write(&path, "{not json").expect("write");
        let error = SetupSession::read(&path, "setup.status").expect_err("corrupt");
        assert_eq!(error.code, "STC-E-IO-ETH-045");
        assert_eq!(error.operation, "setup.status");
        assert!(SetupSession::exists(&path));
        assert!(SetupSession::read_lossy(&path).is_none());
        SetupSession::remove(&path, "setup.leave").expect("remove");
        assert!(!SetupSession::exists(&path));
        assert!(
            SetupSession::read(&path, "setup.status")
                .expect("absent")
                .is_none()
        );
    }

    #[test]
    fn unsupported_descriptors_are_rejected() {
        for text in [
            "[]",
            r#"{"format":2,"protocol":"stc-ethercat-setup/1","session":"a","pid":1,"master":0,"started_unix":0}"#,
            r#"{"format":1,"protocol":"other","session":"a","pid":1,"master":0,"started_unix":0}"#,
            r#"{"format":1,"protocol":"stc-ethercat-setup/1","session":"","pid":1,"master":0,"started_unix":0}"#,
            r#"{"format":1,"protocol":"stc-ethercat-setup/1","session":"a","master":0,"started_unix":0}"#,
        ] {
            let error = SetupSession::parse(text).expect_err(text);
            assert_eq!(error.code, "STC-E-IO-ETH-045", "{text}");
        }
    }

    #[test]
    fn writing_creates_the_session_directory() {
        let base = temporary("write");
        let path = base
            .parent()
            .expect("parent")
            .join("nested")
            .join("session.json");
        let session = SetupSession::new(1, 0, 5);
        session.write(&path, "setup.enter").expect("write");
        assert_eq!(
            SetupSession::read(&path, "setup.status").expect("read"),
            Some(session.clone())
        );
        session.write(&path, "setup.enter").expect("rewrite");
        SetupSession::remove(&path, "setup.leave").expect("remove");
        SetupSession::remove(&path, "setup.leave").expect("idempotent remove");
    }

    #[test]
    fn status_projects_every_documented_field() {
        let status = SetupStatus {
            session: Some(SetupSession::new(1, 0, 5)),
            master_owner: Some("setup-1-5".into()),
            master_held: Some(true),
            conflict: None,
            recovered: false,
        };
        assert!(status.active());
        let json = status.to_json().render();
        assert!(json.contains("\"active\": true"));
        assert!(json.contains("\"master_held\": true"));
        assert!(json.contains("\"conflict\": null"));
        assert!(json.contains("\"recovered\": false"));
        let inactive = SetupStatus {
            session: None,
            master_owner: None,
            master_held: None,
            conflict: None,
            recovered: false,
        };
        assert!(!inactive.active());
        assert!(inactive.to_json().render().contains("\"active\": false"));
    }
}
