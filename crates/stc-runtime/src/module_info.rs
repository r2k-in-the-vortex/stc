//! Adjacent module metadata: process-image sizes and direct-variable bindings.
//!
//! A loadable module carries its identity and code bindings in the descriptor,
//! but the process-image area and address of each direct `AT` variable stay in
//! `<name>.stc-meta.json` (GOT Artifacts §12.1). The host reads that sidecar to
//! configure the image and bind `Direct` cells before prepare runs.
//!
//! Missing metadata is allowed: modules without direct variables and the
//! phase-6 no-op fixtures do not need it. Malformed metadata is a module error.

use std::path::{Path, PathBuf};

use stc_got::GotId;
use stc_got::error::{Error, Result, codes};
use stc_got::json::{self, Json};

/// One directive-variable process-image binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectBinding {
    /// Persistent identity of the direct root.
    pub got_id: GotId,
    /// Image area: 1 inputs, 2 outputs, 3 markers.
    pub area: u8,
    /// Area-relative start bit.
    pub offset_bits: u64,
    /// Declared width in bits.
    pub width_bits: u64,
}

/// Byte sizes of the three process-image areas.
///
/// The process image is contiguous: `%I` inputs, then `%Q` outputs, then `%M`
/// markers (Runtime §2.4). Direct cell addresses are computed from these sizes
/// and the area-relative offsets in [`DirectBinding`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageLayout {
    /// Input image bytes.
    pub inputs_bytes: u64,
    /// Output image bytes.
    pub outputs_bytes: u64,
    /// Marker image bytes.
    pub markers_bytes: u64,
}

impl ImageLayout {
    /// Returns the total image length, or `None` when the sizes overflow.
    #[must_use]
    pub fn total_bytes(&self) -> Option<u64> {
        self.inputs_bytes
            .checked_add(self.outputs_bytes)?
            .checked_add(self.markers_bytes)
    }

    /// Returns the byte offset and length of one image area.
    ///
    /// Areas are `1` inputs, `2` outputs, and `3` markers.
    #[must_use]
    pub fn area(&self, area: u8) -> Option<(u64, u64)> {
        match area {
            1 => Some((0, self.inputs_bytes)),
            2 => Some((self.inputs_bytes, self.outputs_bytes)),
            3 => Some((
                self.inputs_bytes.checked_add(self.outputs_bytes)?,
                self.markers_bytes,
            )),
            _ => None,
        }
    }
}

/// Retained-state policy from the metadata sidecar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetainInfo {
    /// Whether retained state is loaded and saved.
    pub enabled: bool,
    /// State file as written by the compiler (project-relative).
    pub file: Option<String>,
    /// `error` or `reset`.
    pub policy: crate::state::MismatchPolicy,
}

impl Default for RetainInfo {
    fn default() -> Self {
        Self {
            enabled: true,
            file: None,
            policy: crate::state::MismatchPolicy::Error,
        }
    }
}

/// The runtime-relevant fields of the metadata sidecar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleInfo {
    /// Input image bytes.
    pub inputs_bytes: u64,
    /// Output image bytes.
    pub outputs_bytes: u64,
    /// Marker image bytes.
    pub markers_bytes: u64,
    /// Direct-variable bindings in metadata order.
    pub bindings: Vec<DirectBinding>,
    /// I/O backend selected by the project.
    pub backend: crate::run::IoBackend,
    /// File used by the `file` backend, as written in `stc.toml`.
    pub io_file: Option<String>,
    /// Input-image defaults.
    pub input_defaults: Vec<u8>,
    /// Output-image defaults.
    pub output_defaults: Vec<u8>,
    /// Retained-state policy.
    pub retain: RetainInfo,
    /// Raw metadata JSON, retained for the embedding host accessor.
    pub text: String,
}

impl Default for ModuleInfo {
    fn default() -> Self {
        Self {
            inputs_bytes: 0,
            outputs_bytes: 0,
            markers_bytes: 0,
            bindings: Vec::new(),
            backend: crate::run::IoBackend::Null,
            io_file: None,
            input_defaults: Vec::new(),
            output_defaults: Vec::new(),
            retain: RetainInfo::default(),
            text: String::new(),
        }
    }
}

impl ModuleInfo {
    /// Returns the process-image area sizes declared by this sidecar.
    #[must_use]
    pub fn image_layout(&self) -> ImageLayout {
        ImageLayout {
            inputs_bytes: self.inputs_bytes,
            outputs_bytes: self.outputs_bytes,
            markers_bytes: self.markers_bytes,
        }
    }

    /// Returns the binding for one direct-root identity.
    #[must_use]
    pub fn binding(&self, got_id: GotId) -> Option<&DirectBinding> {
        self.bindings
            .iter()
            .find(|binding| binding.got_id == got_id)
    }

    /// Returns `true` when the process-image layout and direct bindings match.
    ///
    /// An equal layout reuses the active image and only rebinds direct cells;
    /// a different layout reconfigures the image at activation (§7.3).
    #[must_use]
    pub fn layout_eq(&self, other: &Self) -> bool {
        self.inputs_bytes == other.inputs_bytes
            && self.outputs_bytes == other.outputs_bytes
            && self.markers_bytes == other.markers_bytes
            && self.bindings == other.bindings
    }

    /// Parses the metadata sidecar.
    ///
    /// # Errors
    ///
    /// Rejects malformed JSON, missing or non-numeric image sizes, and
    /// unparseable direct-binding records.
    pub fn parse(text: &str) -> Result<Self> {
        let root = json::parse(text).map_err(|error| {
            Error::new(
                codes::MODULE_INVALID,
                format!("module metadata is not valid JSON: {}", error.message),
            )
        })?;
        let object = root.as_object().ok_or_else(metadata_error)?;
        let io = object
            .get("io")
            .and_then(Json::as_object)
            .ok_or_else(metadata_error)?;
        let image = io
            .get("image")
            .and_then(Json::as_object)
            .ok_or_else(metadata_error)?;
        let number = |value: Option<&Json>, name: &str| -> Result<u64> {
            let value = value
                .and_then(Json::as_number)
                .and_then(|value| u64::try_from(value).ok())
                .ok_or_else(|| {
                    Error::new(
                        codes::MODULE_INVALID,
                        format!("module metadata is missing `io.image.{name}`"),
                    )
                })?;
            Ok(value)
        };
        let inputs_bytes = number(image.get("inputs_bytes"), "inputs_bytes")?;
        let outputs_bytes = number(image.get("outputs_bytes"), "outputs_bytes")?;
        let markers_bytes = number(image.get("markers_bytes"), "markers_bytes")?;
        let backend = match io.get("backend").and_then(Json::as_str) {
            Some("null") | None => crate::run::IoBackend::Null,
            Some("memory") => crate::run::IoBackend::Memory,
            Some("file") => crate::run::IoBackend::File,
            Some(other) => {
                return Err(Error::new(
                    codes::MODULE_INVALID,
                    format!("module metadata has an unknown I/O backend `{other}`"),
                ));
            }
        };
        let io_file = match io.get("file") {
            None | Some(Json::Null) => None,
            Some(Json::String(value)) => Some(value.clone()),
            Some(_) => return Err(metadata_error()),
        };
        let input_defaults = hex_bytes(
            io.get("input_defaults_hex").and_then(Json::as_str),
            "input_defaults_hex",
        )?;
        let output_defaults = hex_bytes(
            io.get("output_defaults_hex").and_then(Json::as_str),
            "output_defaults_hex",
        )?;
        if !input_defaults.is_empty() && input_defaults.len() as u64 != inputs_bytes {
            return Err(Error::new(
                codes::MODULE_INVALID,
                format!(
                    "module metadata input defaults are {} bytes but the input image is {inputs_bytes}",
                    input_defaults.len()
                ),
            ));
        }
        if !output_defaults.is_empty() && output_defaults.len() as u64 != outputs_bytes {
            return Err(Error::new(
                codes::MODULE_INVALID,
                format!(
                    "module metadata output defaults are {} bytes but the output image is {outputs_bytes}",
                    output_defaults.len()
                ),
            ));
        }
        let retain = match object.get("retain").and_then(Json::as_object) {
            Some(retain) => {
                let enabled = match retain.get("enabled") {
                    None => true,
                    Some(Json::Bool(value)) => *value,
                    Some(_) => return Err(metadata_error()),
                };
                let file = match retain.get("file") {
                    None | Some(Json::Null) => None,
                    Some(Json::String(value)) => Some(value.clone()),
                    Some(_) => return Err(metadata_error()),
                };
                let policy = match retain.get("on_mismatch").and_then(Json::as_str) {
                    None => crate::state::MismatchPolicy::Error,
                    Some(value) => {
                        crate::state::MismatchPolicy::parse(value).ok_or_else(metadata_error)?
                    }
                };
                RetainInfo {
                    enabled,
                    file,
                    policy,
                }
            }
            None if object.contains_key("retain") => return Err(metadata_error()),
            None => RetainInfo::default(),
        };
        let mut bindings = Vec::new();
        if let Some(values) = io.get("bindings").and_then(Json::as_array) {
            for value in values {
                let Some(binding) = value.as_object() else {
                    return Err(metadata_error());
                };
                let Some(got_id) = binding.get("got_id").and_then(Json::as_str) else {
                    // Ad-hoc direct expressions have no root identity.
                    continue;
                };
                let got_id = GotId::parse(got_id).map_err(|error| {
                    Error::new(
                        codes::MODULE_INVALID,
                        format!(
                            "module metadata has an invalid direct `got_id`: {}",
                            error.message
                        ),
                    )
                })?;
                let area = match binding.get("area").and_then(Json::as_str) {
                    Some("I") => 1,
                    Some("Q") => 2,
                    Some("M") => 3,
                    _ => return Err(metadata_error()),
                };
                let offset_bits = binding
                    .get("offset_bits")
                    .and_then(Json::as_number)
                    .and_then(|value| u64::try_from(value).ok())
                    .ok_or_else(metadata_error)?;
                let width_bits = binding
                    .get("width_bits")
                    .and_then(Json::as_number)
                    .and_then(|value| u64::try_from(value).ok())
                    .unwrap_or(8)
                    .max(1);
                bindings.push(DirectBinding {
                    got_id,
                    area,
                    offset_bits,
                    width_bits,
                });
            }
        }
        Ok(Self {
            inputs_bytes,
            outputs_bytes,
            markers_bytes,
            bindings,
            backend,
            io_file,
            input_defaults,
            output_defaults,
            retain,
            text: text.to_owned(),
        })
    }

    /// Reads the metadata sidecar adjacent to a module.
    ///
    /// Returns `Ok(None)` when the file does not exist.
    ///
    /// # Errors
    ///
    /// Rejects an unreadable or malformed sidecar.
    pub fn read_beside(module_path: &Path) -> Result<Option<Self>> {
        let path = metadata_path(module_path);
        if !path.is_file() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path).map_err(|error| {
            Error::new(
                codes::MODULE_INVALID,
                format!("cannot read module metadata `{}`: {error}", path.display()),
            )
        })?;
        Self::parse(&text).map(Some)
    }
}

fn metadata_error() -> Error {
    Error::new(
        codes::MODULE_INVALID,
        "module metadata has an unexpected shape",
    )
}

fn hex_bytes(value: Option<&str>, field: &str) -> Result<Vec<u8>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_empty() {
        return Ok(Vec::new());
    }
    if value.len() % 2 != 0 {
        return Err(Error::new(
            codes::MODULE_INVALID,
            format!("module metadata `io.{field}` has an odd number of hex digits"),
        ));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().as_chunks::<2>().0 {
        let text = std::str::from_utf8(pair).map_err(|_| metadata_error())?;
        bytes.push(u8::from_str_radix(text, 16).map_err(|_| metadata_error())?);
    }
    Ok(bytes)
}

/// Returns the metadata sidecar path for a module.
#[must_use]
pub fn metadata_path(module_path: &Path) -> PathBuf {
    let Some(name) = module_path.file_name().and_then(|name| name.to_str()) else {
        return module_path.with_extension("stc-meta.json");
    };
    let stem = module_stem(name);
    module_path.with_file_name(format!("{stem}.stc-meta.json"))
}

/// Returns the binary manifest path for a module.
#[must_use]
pub fn manifest_path(module_path: &Path) -> PathBuf {
    let Some(name) = module_path.file_name().and_then(|name| name.to_str()) else {
        return module_path.with_extension("stc-got.bin");
    };
    let stem = module_stem(name);
    module_path.with_file_name(format!("{stem}.stc-got.bin"))
}

fn module_stem(name: &str) -> String {
    for suffix in [".stc-module.dll", ".stc-module.so", ".dll", ".so"] {
        if let Some(stem) = name.strip_suffix(suffix) {
            return strip_build_id(stem);
        }
    }
    let stem = Path::new(name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(name);
    strip_build_id(stem)
}

/// Removes a trailing `-<32 hex digits>` build-ID suffix from a module stem.
fn strip_build_id(stem: &str) -> String {
    if let Some((base, id)) = stem.rsplit_once('-')
        && id.len() == 32
        && id.chars().all(|character| character.is_ascii_hexdigit())
    {
        return base.to_owned();
    }
    stem.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"{
        "io": {
            "image": {
                "inputs_bytes": 4,
                "outputs_bytes": 8,
                "markers_bytes": 2
            },
            "bindings": [
                {
                    "got_id": "0x82daffe66a71a484b7cc7e234bed4347",
                    "area": "I",
                    "offset_bits": 3,
                    "width_bits": 8
                },
                {
                    "got_id": "0x987a8ef0813b2598ec043ebdd6744b08",
                    "area": "M",
                    "offset_bits": 0
                }
            ]
        }
    }"#;

    #[test]
    fn parses_the_image_layout_and_direct_bindings() {
        let info = ModuleInfo::parse(VALID).expect("valid metadata");
        assert_eq!(info.inputs_bytes, 4);
        assert_eq!(info.outputs_bytes, 8);
        assert_eq!(info.markers_bytes, 2);
        assert_eq!(info.image_layout().total_bytes(), Some(14));
        assert_eq!(info.bindings.len(), 2);

        let first = GotId::parse("0x82daffe66a71a484b7cc7e234bed4347").expect("identity");
        let binding = info.binding(first).expect("binding");
        assert_eq!(binding.area, 1);
        assert_eq!(binding.offset_bits, 3);
        assert_eq!(binding.width_bits, 8);

        let marker = GotId::parse("0x987a8ef0813b2598ec043ebdd6744b08").expect("identity");
        let binding = info.binding(marker).expect("binding");
        assert_eq!(binding.area, 3);
        assert_eq!(binding.offset_bits, 0);
        assert_eq!(
            binding.width_bits, 8,
            "a missing width defaults to one byte"
        );
    }

    #[test]
    fn parses_io_backend_defaults_and_retain_policy() {
        let info = ModuleInfo::parse(
            r#"{
                "io": {
                    "backend": "file",
                    "file": "io.bin",
                    "image": {
                        "inputs_bytes": 2,
                        "outputs_bytes": 2,
                        "markers_bytes": 0
                    },
                    "input_defaults_hex": "0102",
                    "output_defaults_hex": "0000",
                    "bindings": []
                },
                "retain": {
                    "enabled": false,
                    "file": "state/app.stcstate",
                    "on_mismatch": "reset",
                    "size": 4,
                    "layout_hash": "0x0000000000000001",
                    "leaves": []
                }
            }"#,
        )
        .expect("metadata");
        assert_eq!(info.backend, crate::run::IoBackend::File);
        assert_eq!(info.io_file.as_deref(), Some("io.bin"));
        assert_eq!(info.input_defaults, vec![1, 2]);
        assert_eq!(info.output_defaults, vec![0, 0]);
        assert!(!info.retain.enabled);
        assert_eq!(info.retain.file.as_deref(), Some("state/app.stcstate"));
        assert_eq!(info.retain.policy, crate::state::MismatchPolicy::Reset);
    }

    #[test]
    fn rejects_invalid_io_and_retain_metadata() {
        let cases = [
            r#"{"io":{"backend":"disk","image":{"inputs_bytes":0,"outputs_bytes":0,"markers_bytes":0}}}"#,
            r#"{"io":{"image":{"inputs_bytes":1,"outputs_bytes":0,"markers_bytes":0},
                "input_defaults_hex":"0000"}}"#,
            r#"{"io":{"image":{"inputs_bytes":1,"outputs_bytes":0,"markers_bytes":0},
                "input_defaults_hex":"0"}}"#,
            r#"{"io":{"image":{"inputs_bytes":0,"outputs_bytes":0,"markers_bytes":0}},
                "retain":{"on_mismatch":"reuse"}}"#,
            r#"{"io":{"image":{"inputs_bytes":0,"outputs_bytes":0,"markers_bytes":0}},
                "retain":{"enabled":"yes"}}"#,
        ];
        for text in cases {
            assert!(
                ModuleInfo::parse(text).is_err(),
                "accepted malformed metadata: {text}"
            );
        }
    }

    #[test]
    fn ignores_ad_hoc_direct_expressions_without_an_identity() {
        let info = ModuleInfo::parse(
            r#"{"io":{"image":{"inputs_bytes":0,"outputs_bytes":0,"markers_bytes":0},
                "bindings":[{"area":"Q","offset_bits":0,"width_bits":1}]}}"#,
        )
        .expect("metadata without a persistent binding");
        assert!(info.bindings.is_empty());
    }

    #[test]
    fn rejects_malformed_metadata() {
        let cases = [
            "not json",
            "{}",
            r#"{"io":{}}"#,
            r#"{"io":{"image":{}}}"#,
            r#"{"io":{"image":{"inputs_bytes":"four","outputs_bytes":0,"markers_bytes":0}}}"#,
            r#"{"io":{"image":{"inputs_bytes":0,"outputs_bytes":0,"markers_bytes":0},
                "bindings":[{"got_id":"0x82daffe66a71a484b7cc7e234bed4347","area":"X"}]}}"#,
            r#"{"io":{"image":{"inputs_bytes":0,"outputs_bytes":0,"markers_bytes":0},
                "bindings":[{"got_id":"not-an-identity","area":"I"}]}}"#,
            r#"{"io":{"image":{"inputs_bytes":0,"outputs_bytes":0,"markers_bytes":0},
                "bindings":[42]}}"#,
        ];
        for text in cases {
            assert!(
                ModuleInfo::parse(text).is_err(),
                "accepted malformed metadata: {text}"
            );
        }
    }

    #[test]
    fn layout_equality_covers_sizes_and_bindings() {
        let info = ModuleInfo::parse(VALID).expect("valid metadata");
        assert!(info.layout_eq(&info.clone()));

        let mut resized = info.clone();
        resized.outputs_bytes += 1;
        assert!(!info.layout_eq(&resized));

        let mut rebound = info;
        let before = rebound.clone();
        rebound.bindings[0].offset_bits += 8;
        assert!(!before.layout_eq(&rebound));
    }
}
