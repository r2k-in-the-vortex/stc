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
use stc_got::Manifest;
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
    /// Raw additive `debug` object, when the sidecar carries one.
    ///
    /// The table is validated by a debug session, not by ordinary module
    /// loading: a release or hook-free module without the object loads
    /// normally, and a malformed object cannot prevent the module from
    /// running.
    pub debug: Option<Json>,
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
            debug: None,
            text: String::new(),
        }
    }
}

/// One validated debug checkpoint from the metadata sidecar.
///
/// The record keeps the source binding data needed to resolve source
/// breakpoints and to report a stop: the manifest function identity, the
/// 1-based source coordinate, and the span-table file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DebugCheckpointRecord {
    /// Dense checkpoint identity, starting at 1.
    pub(crate) id: u32,
    /// Sparse checkpoint flags (§16.3).
    pub(crate) flags: u32,
    /// Manifest function that emits the hook.
    pub(crate) function: GotId,
    /// 1-based source line.
    pub(crate) line: u32,
    /// 1-based source column.
    pub(crate) column: u32,
    /// Span-table entry index.
    pub(crate) span: u32,
    /// Relative source file from the span table.
    pub(crate) file: String,
}

/// The validated debug capability of one generation's metadata sidecar.
///
/// The object is validated per build (Debug and Monitoring §16.2/§16.7). A
/// validation failure never prevents module loading; it disables debug
/// features and is reported at `session.start` as `STC-E-DEBUG-016`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DebugMetadataReport {
    /// `debug` or `release`, from the descriptor debug-build bit.
    pub(crate) mode: &'static str,
    /// Descriptor hook capability with a valid checkpoint table.
    pub(crate) hooks: bool,
    /// `dwarf`, `codeview`, or `none`.
    pub(crate) info: &'static str,
    /// Whether a valid non-empty checkpoint table is available.
    pub(crate) checkpoints: bool,
    /// Validated checkpoint records, in ID order.
    pub(crate) records: Vec<DebugCheckpointRecord>,
    /// Validation failure detail, without host paths or token material.
    pub(crate) error: Option<String>,
}

const MAX_CHECKPOINTS: usize = 1_000_000;
const MAX_DEBUG_OBJECT_BYTES: usize = 64 * 1024 * 1024;
const CHECKPOINT_FLAG_MASK: u32 = 0x0F;
const DEBUG_OBJECT_KEYS: [&str; 5] = ["build_id", "checkpoints", "format", "hooks", "info"];
const CHECKPOINT_KEYS: [&str; 6] = ["column", "flags", "function", "id", "line", "span"];

/// Validates the additive `debug` object against its descriptor and manifest.
pub(crate) fn debug_metadata(
    info: Option<&ModuleInfo>,
    manifest: &Manifest,
    debug_build: bool,
    hooks_bit: bool,
) -> DebugMetadataReport {
    let mut report = DebugMetadataReport {
        mode: if debug_build { "debug" } else { "release" },
        hooks: false,
        info: "none",
        checkpoints: false,
        records: Vec::new(),
        error: None,
    };
    let Some(info) = info else {
        if hooks_bit {
            report.error = Some(
                "the descriptor requires debug hooks but the metadata sidecar is missing".into(),
            );
        }
        return report;
    };
    let Some(debug) = info.debug.as_ref() else {
        if hooks_bit {
            report.error = Some(
                "the descriptor requires debug hooks but the sidecar has no debug object".into(),
            );
        }
        return report;
    };
    if info.text.len() > MAX_DEBUG_OBJECT_BYTES {
        report.error = Some(format!(
            "the debug object exceeds the {MAX_DEBUG_OBJECT_BYTES}-byte limit"
        ));
        return report;
    }
    let Some(object) = debug.as_object() else {
        report.error = Some("the debug object is not a JSON object".into());
        return report;
    };
    if object.len() != DEBUG_OBJECT_KEYS.len()
        || DEBUG_OBJECT_KEYS
            .iter()
            .any(|key| !object.contains_key(*key))
    {
        return invalid(report, "the debug object does not have the format-1 keys");
    }
    if object.get("format").and_then(Json::as_number) != Some(1) {
        return invalid(report, "the debug object format is not 1");
    }
    let build_id = object
        .get("build_id")
        .and_then(Json::as_str)
        .and_then(|text| GotId::parse(text).ok());
    if build_id != Some(manifest.header.build_id) {
        return invalid(
            report,
            "the debug object build_id does not match the manifest build identity",
        );
    }
    let Some(hooks) = object.get("hooks").and_then(Json::as_bool) else {
        return invalid(report, "the debug object hooks field is not a boolean");
    };
    let Some(info_name) = object.get("info").and_then(Json::as_str) else {
        return invalid(report, "the debug object info field is not a string");
    };
    let info_name = match info_name {
        "dwarf" => "dwarf",
        "codeview" => "codeview",
        "none" => "none",
        _ => return invalid(report, "the debug object info field is invalid"),
    };
    if hooks != hooks_bit {
        return invalid(
            report,
            "the debug object hooks field disagrees with the descriptor",
        );
    }
    if hooks == (info_name == "none") {
        return invalid(report, "the debug object hooks and info fields disagree");
    }
    let Some(checkpoints) = object.get("checkpoints").and_then(Json::as_array) else {
        return invalid(report, "the debug object checkpoints field is not an array");
    };
    if checkpoints.len() > MAX_CHECKPOINTS {
        return invalid(
            report,
            "the debug object checkpoint count exceeds the limit",
        );
    }
    if hooks && checkpoints.is_empty() {
        return invalid(
            report,
            "the debug object declares hooks but has no checkpoints",
        );
    }
    if !hooks && !checkpoints.is_empty() {
        return invalid(
            report,
            "the debug object has checkpoints but does not declare hooks",
        );
    }
    let span_table = if checkpoints.is_empty() {
        None
    } else {
        let Some(table) = span_table(info) else {
            return invalid(report, "the metadata spans table is malformed");
        };
        Some(table)
    };
    let span_count = span_table
        .as_ref()
        .map_or(0, |table| table.entry_files.len());
    let functions = manifest
        .functions
        .iter()
        .map(|function| function.got_id)
        .collect::<std::collections::BTreeSet<_>>();
    let mut records = Vec::with_capacity(checkpoints.len());
    for (index, checkpoint) in checkpoints.iter().enumerate() {
        let Some(checkpoint) = checkpoint.as_object() else {
            return invalid(report, "a checkpoint entry is not a JSON object");
        };
        if checkpoint.len() != CHECKPOINT_KEYS.len()
            || CHECKPOINT_KEYS
                .iter()
                .any(|key| !checkpoint.contains_key(*key))
        {
            return invalid(report, "a checkpoint entry does not have the format-1 keys");
        }
        let number = |name: &str| checkpoint.get(name).and_then(Json::as_number);
        let id = number("id").and_then(|value| u32::try_from(value).ok());
        if id != Some(u32::try_from(index + 1).unwrap_or(u32::MAX)) {
            return invalid(report, "checkpoint IDs are not dense and sorted from 1");
        }
        let flags = number("flags").and_then(|value| u32::try_from(value).ok());
        if flags.is_none_or(|flags| flags & !CHECKPOINT_FLAG_MASK != 0) {
            return invalid(report, "a checkpoint flag has reserved bits set");
        }
        let line = number("line").and_then(|value| u32::try_from(value).ok());
        let column = number("column").and_then(|value| u32::try_from(value).ok());
        if line.is_none_or(|value| value == 0) || column.is_none_or(|value| value == 0) {
            return invalid(
                report,
                "a checkpoint line or column is not a 1-based coordinate",
            );
        }
        let span = number("span").and_then(|value| u32::try_from(value).ok());
        let Some(span) = span.filter(|span| usize::try_from(*span).is_ok_and(|s| s < span_count))
        else {
            return invalid(report, "a checkpoint names an invalid span index");
        };
        let function = checkpoint
            .get("function")
            .and_then(Json::as_str)
            .and_then(|text| GotId::parse(text).ok());
        let Some(function) = function else {
            return invalid(report, "a checkpoint function is not a GotId");
        };
        if !functions.contains(&function) {
            return invalid(report, "a checkpoint names an unknown manifest function");
        }
        let Some(table) = span_table.as_ref() else {
            return invalid(report, "a checkpoint names an invalid span index");
        };
        let file = table.entry_files[usize::try_from(span).unwrap_or(usize::MAX)] as usize;
        records.push(DebugCheckpointRecord {
            id: id.unwrap_or_default(),
            flags: flags.unwrap_or_default(),
            function,
            line: line.unwrap_or_default(),
            column: column.unwrap_or_default(),
            span,
            file: table.files[file].clone(),
        });
    }
    report.hooks = hooks;
    report.info = info_name;
    report.checkpoints = !checkpoints.is_empty();
    report.records = records;
    report
}

struct SpanTable {
    files: Vec<String>,
    entry_files: Vec<u32>,
}

/// Parses and validates the span table used to resolve checkpoint source files.
fn span_table(info: &ModuleInfo) -> Option<SpanTable> {
    let root = json::parse(&info.text).ok()?;
    let spans = root.as_object()?.get("spans")?.as_object()?;
    let files = spans.get("files")?.as_array()?;
    let entries = spans.get("entries")?.as_array()?;
    let mut paths = Vec::with_capacity(files.len());
    for file in files {
        let file = file.as_str()?;
        if !crate::frames::valid_source_file(file) {
            return None;
        }
        paths.push(file.to_owned());
    }
    let mut entry_files = Vec::with_capacity(entries.len());
    for entry in entries {
        let file = entry.as_object()?.get("file")?.as_number()?;
        let file = u32::try_from(file).ok()?;
        if usize::try_from(file).ok()? >= paths.len() {
            return None;
        }
        entry_files.push(file);
    }
    Some(SpanTable {
        files: paths,
        entry_files,
    })
}

fn invalid(mut report: DebugMetadataReport, message: &str) -> DebugMetadataReport {
    report.error = Some(message.to_owned());
    report.hooks = false;
    report.checkpoints = false;
    report.records.clear();
    report
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
        let debug = object.get("debug").cloned();
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
            debug,
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

/// Returns the stopped-frame sidecar path for a module.
#[must_use]
pub fn frames_path(module_path: &Path) -> PathBuf {
    let Some(name) = module_path.file_name().and_then(|name| name.to_str()) else {
        return module_path.with_extension("stc-frames.json");
    };
    let stem = module_stem(name);
    module_path.with_file_name(format!("{stem}.stc-frames.json"))
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
    fn reads_the_additive_debug_object_without_requiring_it() {
        let text = r#"{
            "io": {"image": {"inputs_bytes": 0, "outputs_bytes": 0, "markers_bytes": 0}},
            "debug": {
                "build_id": "0x11111111111111111111111111111111",
                "checkpoints": [{
                    "column": 3,
                    "flags": 1,
                    "function": "0x22222222222222222222222222222222",
                    "id": 1,
                    "line": 6,
                    "span": 12
                }],
                "format": 1,
                "hooks": true,
                "info": "dwarf"
            }
        }"#;
        let info = ModuleInfo::parse(text).expect("metadata with a debug object");
        let debug = info
            .debug
            .as_ref()
            .and_then(Json::as_object)
            .expect("debug");
        assert_eq!(debug.get("hooks"), Some(&Json::Bool(true)));
        let checkpoints = debug
            .get("checkpoints")
            .and_then(Json::as_array)
            .expect("checkpoints");
        assert_eq!(checkpoints.len(), 1);

        let without = ModuleInfo::parse(
            r#"{"io":{"image":{"inputs_bytes":0,"outputs_bytes":0,"markers_bytes":0}}}"#,
        )
        .expect("metadata without debug");
        assert!(without.debug.is_none());

        // The object is opaque to ordinary loading; session.start validates it.
        let opaque = ModuleInfo::parse(
            r#"{"io":{"image":{"inputs_bytes":0,"outputs_bytes":0,"markers_bytes":0}},
                "debug":"not-an-object"}"#,
        )
        .expect("opaque debug object");
        assert_eq!(opaque.debug, Some(Json::String("not-an-object".into())));
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
