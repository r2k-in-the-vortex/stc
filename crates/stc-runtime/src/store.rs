//! Active-manifest storage, export, and download.
//!
//! The host keeps the committed manifest as its online-change baseline. On disk
//! the store writes an immutable versioned pair under `manifests/` and then
//! replaces a small `current` pointer atomically, so a failed or interrupted
//! write leaves the previous baseline selected (Online Change and GOT §7.4).
//!
//! A download combines the active manifest, a value-free runtime layout
//! summary, and deployment identifiers. It is the artifact another developer
//! can use as the next build's baseline. Runtime values never leave the host.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use stc_got::error::{Error, Result, codes};
use stc_got::json::{Json, parse as parse_json};
use stc_got::{Digest32, GotId, Manifest, RetiredCause, RootKind};

/// Download envelope format.
pub const DOWNLOAD_FORMAT: u16 = 1;
/// Download envelope kind tag.
pub const DOWNLOAD_KIND: &str = "stc-host-download";
/// Current-pointer file name.
pub const CURRENT_FILE: &str = "current";
/// Directory holding immutable versioned manifest pairs.
pub const MANIFESTS_DIR: &str = "manifests";

/// Largest manifest file the host reads from disk.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

/// Reads at most `limit` bytes from a file.
///
/// Bounding the read keeps a corrupt or hostile manifest file from forcing an
/// unbounded allocation before validation can reject it.
pub(crate) fn read_file_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = fs::File::open(path).map_err(|error| io_error("read", path, error))?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read", path, error))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(Error::new(
            codes::MANIFEST_INVALID,
            format!(
                "`{}` is larger than the {limit}-byte host limit",
                path.display()
            ),
        ));
    }
    Ok(bytes)
}

/// A manifest with both canonical projections and its digests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredManifest {
    /// The decoded manifest.
    pub manifest: Manifest,
    /// Compact binary projection.
    pub binary: Vec<u8>,
    /// Canonical JSON projection.
    pub json: String,
    /// Build identity from the header.
    pub build_id: GotId,
    /// Manifest digest.
    pub digest: Digest32,
}

impl StoredManifest {
    /// Encodes a manifest and computes its projections.
    ///
    /// # Errors
    ///
    /// Rejects unrepresentable lengths or an invalid manifest.
    pub fn new(manifest: Manifest) -> Result<Self> {
        manifest.validate()?;
        let binary = manifest.encode_binary()?;
        let json = manifest.to_json_text()?;
        let digest = manifest.digest()?;
        Ok(Self {
            build_id: manifest.header.build_id,
            manifest,
            binary,
            json,
            digest,
        })
    }

    /// Validates received projections against a decoded manifest.
    ///
    /// # Errors
    ///
    /// Rejects bytes or text that do not decode to the same manifest, or a
    /// stale build/digest pair.
    pub fn from_parts(
        manifest: Manifest,
        binary: Vec<u8>,
        json: String,
        expected_build_id: GotId,
        expected_digest: Digest32,
    ) -> Result<Self> {
        let from_binary = Manifest::decode_binary(&binary)?;
        let from_json = Manifest::decode_json(&json)?;
        let digest = manifest.digest()?;
        if from_binary != manifest || from_json != manifest {
            return Err(Error::new(
                codes::MANIFEST_PAIRING,
                "manifest projections do not decode to the same record",
            ));
        }
        if manifest.header.build_id != expected_build_id || digest != expected_digest {
            return Err(Error::new(
                codes::MANIFEST_PAIRING,
                "manifest build ID or digest does not match the selected baseline",
            ));
        }
        Ok(Self {
            build_id: expected_build_id,
            manifest,
            binary,
            json,
            digest,
        })
    }
}

/// The state of one runtime-layout entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutState {
    /// The root is active for the committed generation.
    Active,
    /// The root was removed and its storage is quarantined.
    Quarantined,
}

impl LayoutState {
    /// Returns the JSON name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Quarantined => "quarantined",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "active" => Some(Self::Active),
            "quarantined" => Some(Self::Quarantined),
            _ => None,
        }
    }
}

/// One value-free runtime-layout entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeLayoutEntry {
    /// Persistent identity.
    pub got_id: GotId,
    /// Current or retired key.
    pub key: String,
    /// Root kind.
    pub kind: RootKind,
    /// Active or quarantined.
    pub state: LayoutState,
    /// Slot in the committed manifest, when the root is active.
    pub slot: Option<u32>,
    /// Storage size in bytes.
    pub size: u64,
    /// Generation that allocated the storage.
    pub generation: u64,
    /// Storage address, for host diagnostics only.
    pub storage: Option<usize>,
    /// Quarantine cause, for removed roots.
    pub cause: Option<RetiredCause>,
}

/// The value-free runtime layout summary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeLayout {
    /// Entries sorted by identity.
    pub entries: Vec<RuntimeLayoutEntry>,
}

impl RuntimeLayout {
    /// Projects the layout to canonical JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::Array(
            self.entries
                .iter()
                .map(|entry| {
                    Json::object([
                        (
                            "cause".into(),
                            Json::optional(entry.cause.map(|cause| Json::string(cause.name()))),
                        ),
                        ("generation".into(), Json::unsigned(entry.generation)),
                        ("got_id".into(), Json::string(entry.got_id.to_hex())),
                        ("key".into(), Json::string(entry.key.clone())),
                        ("kind".into(), Json::string(entry.kind.name())),
                        ("size".into(), Json::unsigned(entry.size)),
                        (
                            "slot".into(),
                            Json::optional(entry.slot.map(|slot| Json::unsigned(u64::from(slot)))),
                        ),
                        ("state".into(), Json::string(entry.state.name())),
                        (
                            "storage".into(),
                            Json::optional(
                                entry
                                    .storage
                                    .map(|address| Json::string(format!("0x{address:x}"))),
                            ),
                        ),
                    ])
                })
                .collect(),
        )
    }

    /// Parses a layout summary from JSON.
    ///
    /// # Errors
    ///
    /// Rejects malformed entries, unknown fields, or duplicate identities.
    pub fn from_json(value: &Json) -> Result<Self> {
        let Some(array) = value.as_array() else {
            return Err(Error::new(
                codes::MANIFEST_INVALID,
                "download layout must be an array",
            ));
        };
        let mut entries = Vec::with_capacity(array.len());
        for element in array {
            entries.push(parse_layout_entry(element)?);
        }
        let mut seen = std::collections::BTreeSet::new();
        for entry in &entries {
            if !seen.insert(entry.got_id) {
                return Err(Error::new(
                    codes::MANIFEST_INVALID,
                    format!("download layout repeats identity {}", entry.got_id),
                ));
            }
        }
        entries.sort_by_key(|entry| entry.got_id);
        Ok(Self { entries })
    }
}

fn parse_layout_entry(value: &Json) -> Result<RuntimeLayoutEntry> {
    let Some(object) = value.as_object() else {
        return Err(Error::new(
            codes::MANIFEST_INVALID,
            "download layout entry must be an object",
        ));
    };
    check_keys(
        object,
        &[
            "cause",
            "generation",
            "got_id",
            "key",
            "kind",
            "size",
            "slot",
            "state",
            "storage",
        ],
        "download layout entry",
    )?;
    let got_id = GotId::parse(string_field(object, "got_id", "download layout entry")?)?;
    let key = string_field(object, "key", "download layout entry")?.to_owned();
    let kind_name = string_field(object, "kind", "download layout entry")?;
    let kind = RootKind::from_name(kind_name).ok_or_else(|| {
        Error::new(
            codes::MANIFEST_INVALID,
            format!("download layout entry has unknown root kind `{kind_name}`"),
        )
    })?;
    let state_name = string_field(object, "state", "download layout entry")?;
    let state = LayoutState::from_name(state_name).ok_or_else(|| {
        Error::new(
            codes::MANIFEST_INVALID,
            format!("download layout entry has unknown state `{state_name}`"),
        )
    })?;
    let slot = optional_unsigned(object, "slot", "download layout entry")?
        .map(|value| {
            u32::try_from(value).map_err(|_| {
                Error::new(
                    codes::MANIFEST_INVALID,
                    "download layout slot is out of range",
                )
            })
        })
        .transpose()?;
    let size = unsigned_field(object, "size", "download layout entry")?;
    let generation = unsigned_field(object, "generation", "download layout entry")?;
    let storage = optional_string(object, "storage", "download layout entry")?
        .map(|text| parse_hex_usize(&text))
        .transpose()?;
    let cause = optional_string(object, "cause", "download layout entry")?
        .map(|name| {
            RetiredCause::from_name(&name).ok_or_else(|| {
                Error::new(
                    codes::MANIFEST_INVALID,
                    format!("download layout entry has unknown cause `{name}`"),
                )
            })
        })
        .transpose()?;
    Ok(RuntimeLayoutEntry {
        got_id,
        key,
        kind,
        state,
        slot,
        size,
        generation,
        storage,
        cause,
    })
}

/// Deployment identifiers accompanying a download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deployment {
    /// Module name from the manifest header.
    pub module: String,
    /// Committed build ID.
    pub build_id: GotId,
    /// Committed manifest digest.
    pub manifest_digest: Digest32,
    /// Parent build ID, when the manifest records one.
    pub parent_build_id: Option<GotId>,
}

/// A self-contained download artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    /// Envelope format.
    pub format: u16,
    /// Deployment identifiers.
    pub deployment: Deployment,
    /// Value-free runtime layout.
    pub layout: RuntimeLayout,
    /// The active manifest and its projections.
    pub manifest: StoredManifest,
}

impl Download {
    /// Returns the active manifest.
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.manifest.manifest
    }

    /// Returns the canonical JSON manifest that can be used as a build baseline.
    #[must_use]
    pub fn manifest_json(&self) -> &str {
        &self.manifest.json
    }

    /// Returns the binary manifest projection.
    #[must_use]
    pub fn manifest_binary(&self) -> &[u8] {
        &self.manifest.binary
    }

    /// Projects the download to canonical JSON text.
    #[must_use]
    pub fn to_json_text(&self) -> String {
        Json::object([
            (
                "deployment".into(),
                Json::object([
                    (
                        "build_id".into(),
                        Json::string(self.deployment.build_id.to_hex()),
                    ),
                    (
                        "manifest_digest".into(),
                        Json::string(self.deployment.manifest_digest.to_hex()),
                    ),
                    (
                        "module".into(),
                        Json::string(self.deployment.module.clone()),
                    ),
                    (
                        "parent_build_id".into(),
                        Json::optional(
                            self.deployment
                                .parent_build_id
                                .map(|build_id| Json::string(build_id.to_hex())),
                        ),
                    ),
                ]),
            ),
            ("format".into(), Json::unsigned(u64::from(self.format))),
            ("kind".into(), Json::string(DOWNLOAD_KIND)),
            ("layout".into(), self.layout.to_json()),
            (
                "manifest_binary".into(),
                Json::string(hex_bytes(&self.manifest.binary)),
            ),
            (
                "manifest_json".into(),
                Json::string(self.manifest.json.clone()),
            ),
        ])
        .render()
    }

    /// Parses and validates a download envelope.
    ///
    /// # Errors
    ///
    /// Rejects malformed JSON, an unknown format or kind, inconsistent manifest
    /// projections, deployment mismatches, or an invalid layout.
    pub fn from_json_text(text: &str) -> Result<Self> {
        let value = parse_json(text)?;
        let Some(object) = value.as_object() else {
            return Err(Error::new(
                codes::MANIFEST_INVALID,
                "download envelope must be an object",
            ));
        };
        check_keys(
            object,
            &[
                "deployment",
                "format",
                "kind",
                "layout",
                "manifest_binary",
                "manifest_json",
            ],
            "download envelope",
        )?;
        let format = unsigned_field(object, "format", "download envelope")?;
        if format != u64::from(DOWNLOAD_FORMAT) {
            return Err(Error::new(
                codes::MANIFEST_INVALID,
                format!("unsupported download format {format}"),
            ));
        }
        let kind = string_field(object, "kind", "download envelope")?;
        if kind != DOWNLOAD_KIND {
            return Err(Error::new(
                codes::MANIFEST_INVALID,
                format!("download envelope kind `{kind}` is not `{DOWNLOAD_KIND}`"),
            ));
        }
        let deployment_value = field(object, "deployment", "download envelope")?;
        let Some(deployment_object) = deployment_value.as_object() else {
            return Err(Error::new(
                codes::MANIFEST_INVALID,
                "download deployment must be an object",
            ));
        };
        check_keys(
            deployment_object,
            &["build_id", "manifest_digest", "module", "parent_build_id"],
            "download deployment",
        )?;
        let deployment = Deployment {
            module: string_field(deployment_object, "module", "download deployment")?.to_owned(),
            build_id: GotId::parse(string_field(
                deployment_object,
                "build_id",
                "download deployment",
            )?)?,
            manifest_digest: Digest32::parse(string_field(
                deployment_object,
                "manifest_digest",
                "download deployment",
            )?)?,
            parent_build_id: optional_string(
                deployment_object,
                "parent_build_id",
                "download deployment",
            )?
            .map(|text| GotId::parse(&text))
            .transpose()?,
        };
        let json = string_field(object, "manifest_json", "download envelope")?.to_owned();
        let binary = decode_hex_bytes(string_field(
            object,
            "manifest_binary",
            "download envelope",
        )?)?;
        let manifest = Manifest::decode_binary(&binary)?;
        let from_json = Manifest::decode_json(&json)?;
        if manifest != from_json {
            return Err(Error::new(
                codes::MANIFEST_PAIRING,
                "download manifest projections disagree",
            ));
        }
        let stored = StoredManifest::from_parts(
            manifest,
            binary,
            json,
            deployment.build_id,
            deployment.manifest_digest,
        )?;
        if stored.manifest.header.module_name != deployment.module {
            return Err(Error::new(
                codes::MANIFEST_INVALID,
                "download deployment module does not match the manifest",
            ));
        }
        let layout_value = field(object, "layout", "download envelope")?;
        Ok(Self {
            format: DOWNLOAD_FORMAT,
            deployment,
            layout: RuntimeLayout::from_json(layout_value)?,
            manifest: stored,
        })
    }
}

/// The host's committed manifest baseline.
#[derive(Debug)]
pub struct ManifestStore {
    root: Option<PathBuf>,
    active: Option<StoredManifest>,
    problem: Option<String>,
}

impl ManifestStore {
    /// Creates an in-memory store.
    #[must_use]
    pub fn memory() -> Self {
        Self {
            root: None,
            active: None,
            problem: None,
        }
    }

    /// Opens a state directory, loading the current baseline when present.
    ///
    /// A missing `current` pointer is an empty store. A present but invalid
    /// pointer or pair is recorded as a problem and leaves the store empty, so
    /// the host requires an explicit cold install.
    #[must_use]
    pub fn open(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let mut store = Self {
            root: Some(root),
            active: None,
            problem: None,
        };
        store.reload();
        store
    }

    /// Reloads the active baseline from disk.
    ///
    /// Returns `true` when a valid baseline is active.
    pub fn reload(&mut self) -> bool {
        self.active = None;
        self.problem = None;
        let Some(root) = &self.root else {
            return false;
        };
        match load_active(root) {
            Ok(active) => {
                self.active = active;
                self.active.is_some()
            }
            Err(error) => {
                self.problem = Some(error.to_string());
                false
            }
        }
    }

    /// Returns the manifest root directory, when the store is file-backed.
    #[must_use]
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Returns the active baseline.
    #[must_use]
    pub fn active(&self) -> Option<&StoredManifest> {
        self.active.as_ref()
    }

    /// Returns the canonical JSON path of the active baseline, when the store
    /// is file-backed and has one.
    #[must_use]
    pub fn active_json_path(&self) -> Option<PathBuf> {
        let root = self.root.as_ref()?;
        let active = self.active.as_ref()?;
        Some(
            root.join(MANIFESTS_DIR)
                .join(format!("{}.stc-got.json", active.build_id.to_hex_plain())),
        )
    }

    /// Returns why the stored baseline is unavailable, when it is invalid.
    #[must_use]
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// Makes a manifest the active baseline.
    ///
    /// A file-backed store writes the versioned pair and then replaces the
    /// `current` pointer. If any write fails, the previous baseline stays
    /// active and the error is returned.
    ///
    /// # Errors
    ///
    /// Rejects an inconsistent manifest projection or a filesystem failure.
    pub fn install(&mut self, stored: StoredManifest) -> Result<()> {
        // Guard against a caller mutating the manifest after encoding.
        let verified = StoredManifest::from_parts(
            stored.manifest.clone(),
            stored.binary.clone(),
            stored.json.clone(),
            stored.build_id,
            stored.digest,
        )?;
        if let Some(root) = &self.root {
            write_baseline(root, &verified)?;
        }
        self.active = Some(verified);
        self.problem = None;
        Ok(())
    }

    /// Builds a download artifact from the committed baseline.
    ///
    /// # Errors
    ///
    /// Rejects a request when no valid baseline is active.
    pub fn download(&self, layout: RuntimeLayout, deployment: Deployment) -> Result<Download> {
        let Some(manifest) = self.active.clone() else {
            return Err(Error::new(
                codes::MANIFEST_INVALID,
                self.problem
                    .clone()
                    .unwrap_or_else(|| "the host has no active manifest".into()),
            ));
        };
        if manifest.build_id != deployment.build_id
            || manifest.digest != deployment.manifest_digest
            || manifest.manifest.header.module_name != deployment.module
        {
            return Err(Error::new(
                codes::MANIFEST_PAIRING,
                "download deployment identifiers do not match the active manifest",
            ));
        }
        Ok(Download {
            format: DOWNLOAD_FORMAT,
            deployment,
            layout,
            manifest,
        })
    }
}

fn write_baseline(root: &Path, stored: &StoredManifest) -> Result<()> {
    let directory = root.join(MANIFESTS_DIR);
    fs::create_dir_all(&directory).map_err(|error| io_error("create", &directory, error))?;
    let build = stored.build_id.to_hex_plain();
    let binary_path = directory.join(format!("{build}.stc-got.bin"));
    let json_path = directory.join(format!("{build}.stc-got.json"));
    write_atomic(&binary_path, &stored.binary)?;
    write_atomic(&json_path, stored.json.as_bytes())?;
    let pointer = format!(
        "{} {}\n",
        stored.build_id.to_hex_plain(),
        stored.digest.to_hex()
    );
    write_atomic(&root.join(CURRENT_FILE), pointer.as_bytes())?;
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::new(codes::MANIFEST_INVALID, "manifest path has no parent"))?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "manifest".into());
    let temporary = parent.join(format!(".{name}.tmp-{}", std::process::id()));
    let result = (|| {
        let mut file =
            fs::File::create(&temporary).map_err(|error| io_error("create", &temporary, error))?;
        file.write_all(bytes)
            .map_err(|error| io_error("write", &temporary, error))?;
        file.sync_all()
            .map_err(|error| io_error("flush", &temporary, error))?;
        drop(file);
        fs::rename(&temporary, path).map_err(|error| io_error("replace", path, error))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn load_active(root: &Path) -> Result<Option<StoredManifest>> {
    let pointer_path = root.join(CURRENT_FILE);
    let text = match fs::read_to_string(&pointer_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("read", &pointer_path, error)),
    };
    let mut fields = text.split_whitespace();
    let build_token = fields.next().ok_or_else(|| {
        Error::new(
            codes::MANIFEST_INVALID,
            format!("`{}` has no build ID", pointer_path.display()),
        )
    })?;
    let digest_token = fields.next().ok_or_else(|| {
        Error::new(
            codes::MANIFEST_INVALID,
            format!("`{}` has no manifest digest", pointer_path.display()),
        )
    })?;
    if fields.next().is_some() {
        return Err(Error::new(
            codes::MANIFEST_INVALID,
            format!("`{}` has trailing fields", pointer_path.display()),
        ));
    }
    if build_token.len() != 32 {
        return Err(Error::new(
            codes::MANIFEST_INVALID,
            format!("`{}` has a malformed build ID", pointer_path.display()),
        ));
    }
    let build_id = GotId::parse(&format!("0x{build_token}"))?;
    let digest = Digest32::parse(digest_token)?;

    let directory = root.join(MANIFESTS_DIR);
    let binary_path = directory.join(format!("{build_token}.stc-got.bin"));
    let json_path = directory.join(format!("{build_token}.stc-got.json"));
    let binary = read_file_bounded(&binary_path, MAX_MANIFEST_BYTES)?;
    let json_bytes = read_file_bounded(&json_path, MAX_MANIFEST_BYTES)?;
    let json = String::from_utf8(json_bytes).map_err(|_| {
        Error::new(
            codes::MANIFEST_INVALID,
            format!("`{}` is not UTF-8", json_path.display()),
        )
    })?;
    let manifest = Manifest::decode_binary(&binary)?;
    StoredManifest::from_parts(manifest, binary, json, build_id, digest).map(Some)
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> Error {
    Error::new(
        codes::MANIFEST_INVALID,
        format!("cannot {action} `{}`: {error}", path.display()),
    )
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("0x");
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn decode_hex_bytes(text: &str) -> Result<Vec<u8>> {
    let Some(digits) = text.strip_prefix("0x") else {
        return Err(Error::new(
            codes::MANIFEST_INVALID,
            "hex byte string must start with `0x`",
        ));
    };
    if digits.len() % 2 != 0 {
        return Err(Error::new(
            codes::MANIFEST_INVALID,
            "hex byte string has an odd number of digits",
        ));
    }
    let mut bytes = Vec::with_capacity(digits.len() / 2);
    for pair in digits.as_bytes().as_chunks::<2>().0 {
        let text = std::str::from_utf8(pair)
            .map_err(|_| Error::new(codes::MANIFEST_INVALID, "hex byte string is not ASCII"))?;
        let byte = u8::from_str_radix(text, 16)
            .map_err(|_| Error::new(codes::MANIFEST_INVALID, "hex byte string is malformed"))?;
        bytes.push(byte);
    }
    Ok(bytes)
}

fn parse_hex_usize(text: &str) -> Result<usize> {
    let Some(digits) = text.strip_prefix("0x") else {
        return Err(Error::new(
            codes::MANIFEST_INVALID,
            "address must start with `0x`",
        ));
    };
    usize::from_str_radix(digits, 16)
        .map_err(|_| Error::new(codes::MANIFEST_INVALID, "address is malformed"))
}

fn field<'a>(object: &'a BTreeMap<String, Json>, name: &str, what: &str) -> Result<&'a Json> {
    object.get(name).ok_or_else(|| {
        Error::new(
            codes::MANIFEST_INVALID,
            format!("{what} is missing `{name}`"),
        )
    })
}

fn string_field<'a>(object: &'a BTreeMap<String, Json>, name: &str, what: &str) -> Result<&'a str> {
    field(object, name, what)?.as_str().ok_or_else(|| {
        Error::new(
            codes::MANIFEST_INVALID,
            format!("{what}.{name} must be a string"),
        )
    })
}

fn optional_string(
    object: &BTreeMap<String, Json>,
    name: &str,
    what: &str,
) -> Result<Option<String>> {
    let value = field(object, name, what)?;
    if value.is_null() {
        Ok(None)
    } else {
        value
            .as_str()
            .map(|text| Some(text.to_owned()))
            .ok_or_else(|| {
                Error::new(
                    codes::MANIFEST_INVALID,
                    format!("{what}.{name} must be a string or null"),
                )
            })
    }
}

fn unsigned_field(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<u64> {
    let value = field(object, name, what)?.as_number().ok_or_else(|| {
        Error::new(
            codes::MANIFEST_INVALID,
            format!("{what}.{name} must be an integer"),
        )
    })?;
    u64::try_from(value).map_err(|_| {
        Error::new(
            codes::MANIFEST_INVALID,
            format!("{what}.{name} is out of range"),
        )
    })
}

fn optional_unsigned(
    object: &BTreeMap<String, Json>,
    name: &str,
    what: &str,
) -> Result<Option<u64>> {
    let value = field(object, name, what)?;
    if value.is_null() {
        return Ok(None);
    }
    unsigned_field(object, name, what).map(Some)
}

fn check_keys(object: &BTreeMap<String, Json>, allowed: &[&str], what: &str) -> Result<()> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(Error::new(
                codes::MANIFEST_INVALID,
                format!("{what} contains unknown field `{key}`"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU64, Ordering};

    use stc_got::SlotKind;

    use crate::test_support;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temporary_directory(label: &str) -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stc-runtime-store-{}-{id}-{label}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn simple() -> StoredManifest {
        let artifacts = test_support::artifacts(test_support::SIMPLE, "simple");
        StoredManifest::new(artifacts.manifest).expect("stored")
    }

    fn deployment(stored: &StoredManifest) -> Deployment {
        Deployment {
            module: stored.manifest.header.module_name.clone(),
            build_id: stored.build_id,
            manifest_digest: stored.digest,
            parent_build_id: stored.manifest.header.parent_build_id,
        }
    }

    fn sample_layout(stored: &StoredManifest) -> RuntimeLayout {
        let mut entries: Vec<RuntimeLayoutEntry> = stored
            .manifest
            .roots
            .iter()
            .map(|root| RuntimeLayoutEntry {
                got_id: root.got_id,
                key: root.key.clone(),
                kind: root.root_kind,
                state: LayoutState::Active,
                slot: stored
                    .manifest
                    .slots
                    .iter()
                    .find(|slot| slot.got_id == root.got_id && slot.kind == SlotKind::Data)
                    .map(|slot| slot.slot),
                size: root.size,
                generation: 1,
                storage: Some(0x1000),
                cause: None,
            })
            .collect();
        if let Some(entry) = entries.first_mut() {
            entry.state = LayoutState::Quarantined;
            entry.slot = None;
            entry.storage = Some(0x2000);
            entry.cause = Some(RetiredCause::Dropped);
        }
        entries.sort_by_key(|entry| entry.got_id);
        RuntimeLayout { entries }
    }

    #[test]
    fn memory_store_installs_and_exports() {
        let stored = simple();
        let mut store = ManifestStore::memory();
        assert!(store.active().is_none());
        store.install(stored.clone()).expect("install");
        let active = store.active().expect("active");
        assert_eq!(active.manifest, stored.manifest);
        assert_eq!(active.binary, stored.binary);
        assert_eq!(active.json, stored.json);
        assert_eq!(active.digest, stored.digest);
    }

    #[test]
    fn directory_store_round_trips_the_baseline() {
        let directory = temporary_directory("round-trip");
        let stored = simple();
        let mut store = ManifestStore::open(&directory);
        assert!(store.active().is_none());
        assert!(store.problem().is_none());
        store.install(stored.clone()).expect("install");

        assert!(directory.join(CURRENT_FILE).is_file());
        assert!(
            directory
                .join(MANIFESTS_DIR)
                .join(format!("{}.stc-got.bin", stored.build_id.to_hex_plain()))
                .is_file()
        );

        let reopened = ManifestStore::open(&directory);
        let active = reopened.active().expect("active after reopen");
        assert_eq!(active.manifest, stored.manifest);
        assert_eq!(active.binary, stored.binary);
        assert_eq!(active.json, stored.json);
        assert_eq!(active.digest, stored.digest);
        assert!(reopened.problem().is_none());
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn directory_store_replaces_the_current_pointer() {
        let directory = temporary_directory("replace");
        let first = simple();
        let mut store = ManifestStore::open(&directory);
        store.install(first.clone()).expect("first install");

        let alternate = test_support::artifacts(test_support::SIMPLE_ALTERNATE, "simple");
        let second = StoredManifest::new(alternate.manifest).expect("second");
        assert_ne!(first.build_id, second.build_id);
        store.install(second.clone()).expect("second install");

        let reopened = ManifestStore::open(&directory);
        let active = reopened.active().expect("active after replace");
        assert_eq!(active.build_id, second.build_id);
        assert_eq!(active.digest, second.digest);
        assert_eq!(active.manifest, second.manifest);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn missing_current_is_an_empty_store() {
        let directory = temporary_directory("missing");
        fs::create_dir_all(&directory).expect("create");
        let store = ManifestStore::open(&directory);
        assert!(store.active().is_none());
        assert!(store.problem().is_none());
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn corrupt_current_reports_a_problem() {
        let directory = temporary_directory("corrupt");
        fs::create_dir_all(&directory).expect("create");
        fs::write(directory.join(CURRENT_FILE), "not-a-build-id\n").expect("write");
        let store = ManifestStore::open(&directory);
        assert!(store.active().is_none());
        assert!(store.problem().is_some());
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn mismatched_projections_are_rejected() {
        let stored = simple();
        let mut changed = stored.manifest.clone();
        changed.header.module_name = "other".into();
        let error = StoredManifest::from_parts(
            changed,
            stored.binary.clone(),
            stored.json.clone(),
            stored.build_id,
            stored.digest,
        )
        .unwrap_err();
        assert_eq!(error.code, codes::MANIFEST_PAIRING);

        let error = StoredManifest::from_parts(
            stored.manifest.clone(),
            stored.binary.clone(),
            stored.json.clone(),
            GotId::from_bytes([7; 16]),
            stored.digest,
        )
        .unwrap_err();
        assert_eq!(error.code, codes::MANIFEST_PAIRING);
    }

    #[test]
    fn failed_install_keeps_the_previous_baseline() {
        let directory = temporary_directory("failure");
        fs::create_dir_all(&directory).expect("create");
        let root_file = directory.join("root-is-a-file");
        fs::write(&root_file, b"x").expect("write");
        let mut store = ManifestStore {
            root: Some(root_file),
            active: None,
            problem: None,
        };
        let first = simple();
        assert!(store.install(first.clone()).is_err());
        assert!(store.active().is_none());

        store.root = None;
        store.install(first.clone()).expect("memory install");
        store.root = Some(directory.join("root-is-a-file"));
        assert!(store.install(simple()).is_err());
        assert_eq!(
            store.active().map(|stored| stored.digest),
            Some(first.digest)
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn download_round_trips_through_canonical_json() {
        let stored = simple();
        let layout = sample_layout(&stored);
        let deployment = deployment(&stored);
        let download = Download {
            format: DOWNLOAD_FORMAT,
            deployment: deployment.clone(),
            layout: layout.clone(),
            manifest: stored.clone(),
        };
        let text = download.to_json_text();
        let parsed = Download::from_json_text(&text).expect("parse download");
        assert_eq!(parsed, download);
        assert_eq!(parsed.manifest_json(), stored.json);
        assert_eq!(parsed.manifest_binary(), stored.binary);
        assert_eq!(
            Manifest::decode_json(parsed.manifest_json()).expect("decode"),
            stored.manifest
        );

        let tampered = text.replace("\"format\": 1", "\"format\": 2");
        let error = Download::from_json_text(&tampered).unwrap_err();
        assert_eq!(error.code, codes::MANIFEST_INVALID);
    }

    #[test]
    fn store_download_requires_an_active_baseline() {
        let store = ManifestStore::memory();
        let error = store
            .download(
                RuntimeLayout::default(),
                Deployment {
                    module: "simple".into(),
                    build_id: GotId::from_bytes([1; 16]),
                    manifest_digest: Digest32::ZERO,
                    parent_build_id: None,
                },
            )
            .unwrap_err();
        assert_eq!(error.code, codes::MANIFEST_INVALID);
    }

    #[test]
    fn layout_json_round_trips_and_is_sorted() {
        let stored = simple();
        let mut layout = sample_layout(&stored);
        layout.entries.reverse();
        layout.entries.sort_by_key(|entry| entry.got_id);
        let json = layout.to_json();
        let parsed = RuntimeLayout::from_json(&json).expect("parse layout");
        assert_eq!(parsed, layout);
        let mut entries = parsed.entries.clone();
        entries.sort_by_key(|entry| entry.got_id);
        assert_eq!(entries, parsed.entries);
    }

    #[test]
    fn hex_helpers_reject_malformed_input() {
        assert_eq!(decode_hex_bytes("0x00ff").unwrap(), vec![0, 0xFF]);
        assert!(decode_hex_bytes("00ff").is_err());
        assert!(decode_hex_bytes("0x0").is_err());
        assert!(decode_hex_bytes("0xzz").is_err());
        assert_eq!(parse_hex_usize("0x10").unwrap(), 16);
        assert!(parse_hex_usize("10").is_err());
        assert_eq!(hex_bytes(&[0, 0xFF]), "0x00ff");
    }
}
