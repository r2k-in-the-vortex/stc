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
    /// Raw metadata JSON, retained for the embedding host accessor.
    pub text: String,
}

impl ModuleInfo {
    /// Returns `true` when the process-image layout and direct bindings match.
    ///
    /// An online change may reuse the active image only when this holds; a
    /// layout or binding change requires a cold download until phase 8's
    /// reconfiguration and remap policy lands.
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
