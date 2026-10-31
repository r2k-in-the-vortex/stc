//! Deterministic metadata model and JSON writer (IO §5, State §6, D-091).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use stc_span::Span;

/// A JSON value with sorted object keys.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// An integer number.
    Number(i128),
    /// A floating-point number.
    Float(f64),
    /// A string.
    String(String),
    /// An array.
    Array(Vec<Json>),
    /// An object with sorted keys.
    Object(BTreeMap<String, Json>),
}

impl Json {
    /// Builds an object from pairs.
    #[must_use]
    pub fn object(pairs: impl IntoIterator<Item = (String, Json)>) -> Self {
        Self::Object(pairs.into_iter().collect())
    }

    /// Renders compact JSON (used for the embedded metadata symbol).
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, false, 0);
        out
    }

    /// Renders pretty JSON with sorted keys and a trailing newline.
    #[must_use]
    pub fn render_pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, true, 0);
        out.push('\n');
        out
    }

    fn write(&self, out: &mut String, pretty: bool, indent: usize) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Self::Number(value) => {
                let _ = write!(out, "{value}");
            }
            Self::Float(value) if !value.is_finite() => {
                // JSON has no NaN/infinity.
                out.push_str("null");
            }
            Self::Float(value) => {
                // Rust's shortest round-trip formatting is deterministic and a
                // valid JSON number.
                let rendered = format!("{value}");
                out.push_str(&rendered);
                if !rendered.contains(['.', 'e', 'E']) {
                    out.push_str(".0");
                }
            }
            Self::String(value) => write_string(out, value),
            Self::Array(values) => {
                out.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    if pretty {
                        out.push('\n');
                        indent_to(out, indent + 1);
                    }
                    value.write(out, pretty, indent + 1);
                }
                if pretty && !values.is_empty() {
                    out.push('\n');
                    indent_to(out, indent);
                }
                out.push(']');
            }
            Self::Object(values) => {
                out.push('{');
                for (index, (key, value)) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    if pretty {
                        out.push('\n');
                        indent_to(out, indent + 1);
                    }
                    write_string(out, key);
                    out.push(':');
                    if pretty {
                        out.push(' ');
                    }
                    value.write(out, pretty, indent + 1);
                }
                if pretty && !values.is_empty() {
                    out.push('\n');
                    indent_to(out, indent);
                }
                out.push('}');
            }
        }
    }
}

fn write_string(out: &mut String, value: &str) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04X}", character as u32);
            }
            character => out.push(character),
        }
    }
    out.push('"');
}

fn indent_to(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push_str("  ");
    }
}

/// One configured program instance.
#[derive(Clone, Debug)]
pub struct ProgramMetadata {
    /// Instance name.
    pub name: String,
    /// Program type name.
    pub program: String,
    /// Resource name.
    pub resource: String,
    /// Bound task name, when any.
    pub task: Option<String>,
}

/// One configured task.
#[derive(Clone, Debug)]
pub struct TaskMetadata {
    /// Resource name.
    pub resource: String,
    /// Task name.
    pub name: String,
    /// Folded `SINGLE`, when present.
    pub single: Option<bool>,
    /// Folded interval in nanoseconds, when present.
    pub interval_ns: Option<i128>,
    /// Folded priority, when present.
    pub priority: Option<i128>,
}

/// One process-image binding.
#[derive(Clone, Debug)]
pub struct IoBindingMetadata {
    /// Variable path.
    pub variable: String,
    /// Area letter.
    pub area: char,
    /// Address spelling without `%`.
    pub address: String,
    /// Area-relative start bit.
    pub offset_bits: u64,
    /// Declared width in bits.
    pub width_bits: u64,
    /// Canonical type name.
    pub ty: String,
    /// Persistent identity of the direct root, when the binding is a declared
    /// `AT` variable or field. In v1 this is the same identity as `got_id`.
    pub direct_id: Option<String>,
    /// Per-generation GOT cell identity of the direct root.
    pub got_id: Option<String>,
    /// Effective device, when a pragma or project binding applies.
    pub device: Option<String>,
    /// Effective channel: a number or a declared channel name.
    pub channel: Option<Json>,
    /// Effective direction spelling.
    pub direction: Option<String>,
    /// `{stc:io}` hint fields.
    pub hint: Option<HintMetadata>,
    /// `pragma` or `project`.
    pub source: &'static str,
}

/// One declared project I/O device copied from `stc.toml`.
#[derive(Clone, Debug)]
pub struct DeviceMetadata {
    /// Device name.
    pub name: String,
    /// Opaque, runtime-defined device kind.
    pub kind: String,
    /// Opaque canonical device configuration.
    pub config: Option<Json>,
    /// Optional channel descriptors.
    pub channels: Vec<ChannelMetadata>,
}

/// One declared device channel.
#[derive(Clone, Debug)]
pub struct ChannelMetadata {
    /// Channel name.
    pub name: String,
    /// `input`, `output`, or `inout`.
    pub direction: String,
    /// Channel width in bits.
    pub width: u32,
}

/// One project binding channel reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectChannel {
    /// Non-negative channel number.
    Index(u32),
    /// Declared channel name.
    Name(String),
}

/// One machine-profile route to resolve against the program and device tree
/// (IO-Routing-and-Backends §3).
#[derive(Clone, Debug)]
pub struct IoRoute {
    /// The single source endpoint.
    pub source: IoRouteEndpoint,
    /// The non-empty sink list.
    pub sinks: Vec<IoRouteEndpoint>,
}

/// One profile route endpoint before variable resolution.
#[derive(Clone, Debug)]
pub enum IoRouteEndpoint {
    /// A declared physical input channel.
    PhysicalInput {
        /// Device name.
        device: String,
        /// Channel name.
        channel: String,
        /// Profile span of the endpoint.
        span: Span,
    },
    /// A declared physical output channel.
    PhysicalOutput {
        /// Device name.
        device: String,
        /// Channel name.
        channel: String,
        /// Profile span of the endpoint.
        span: Span,
    },
    /// A direct `%I` variable, resolved against the image layout.
    PlcInput {
        /// Variable path as written in the profile.
        variable: String,
        /// Profile span of the endpoint.
        span: Span,
    },
    /// A direct `%Q` variable, resolved against the image layout.
    PlcOutput {
        /// Variable path as written in the profile.
        variable: String,
        /// Profile span of the endpoint.
        span: Span,
    },
}

impl IoRouteEndpoint {
    /// Returns the profile span.
    #[must_use]
    pub const fn span(&self) -> Span {
        match self {
            Self::PhysicalInput { span, .. }
            | Self::PhysicalOutput { span, .. }
            | Self::PlcInput { span, .. }
            | Self::PlcOutput { span, .. } => *span,
        }
    }
}

/// One resolved route endpoint.
#[derive(Clone, Debug)]
pub struct RouteEndpointMetadata {
    /// `physical-input`, `physical-output`, `plc-input`, or `plc-output`.
    pub kind: &'static str,
    /// Physical device name.
    pub device: Option<String>,
    /// Physical channel name.
    pub channel: Option<String>,
    /// PLC variable path, in the image layout's canonical spelling.
    pub variable: Option<String>,
    /// PLC process-image area letter.
    pub area: Option<char>,
    /// PLC area-relative start bit.
    pub offset_bits: Option<u64>,
    /// Endpoint width in bits.
    pub width_bits: u64,
}

/// One resolved route.
#[derive(Clone, Debug)]
pub struct RouteMetadata {
    /// The source endpoint.
    pub source: RouteEndpointMetadata,
    /// The sink endpoints.
    pub sinks: Vec<RouteEndpointMetadata>,
}

/// The embedded machine profile.
#[derive(Clone, Debug)]
pub struct IoProfileMetadata {
    /// Project-relative profile path as selected by `[io].config`.
    pub path: String,
    /// Canonical profile text, embedded so the artifact is self-contained.
    pub text: String,
    /// SHA-256 digest of `text` as `0x…`.
    pub digest: String,
    /// Device names declared by the profile, in declaration order.
    pub devices: Vec<String>,
}

/// One `[[io.bindings]]` entry the compiler resolves against HIR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectBinding {
    /// Fully qualified ST variable path.
    pub variable: String,
    /// Declared device name.
    pub device: String,
    /// Optional channel hint.
    pub channel: Option<ProjectChannel>,
    /// Optional direction (`input`, `output`, or `inout`).
    pub direction: Option<String>,
}

/// One resolved `VAR_ACCESS` declaration.
#[derive(Clone, Debug)]
pub struct VarAccessMetadata {
    /// Access name.
    pub name: String,
    /// Resolved variable path.
    pub path: String,
    /// Declared type spelling.
    pub ty: String,
    /// `READ_ONLY` marker.
    pub read_only: bool,
    /// Process-image area, for a direct access path.
    pub area: Option<char>,
    /// Address spelling without `%`, for a direct access path.
    pub address: Option<String>,
    /// Area-relative start bit, for a direct access path.
    pub offset_bits: Option<u64>,
    /// Access width in bits, for a direct access path.
    pub width_bits: Option<u64>,
}

/// A `{stc:io}` hint as exposed in metadata.
#[derive(Clone, Debug)]
pub struct HintMetadata {
    /// Device name.
    pub device: Option<String>,
    /// Channel number.
    pub channel: Option<u32>,
    /// Direction spelling.
    pub direction: Option<&'static str>,
}

/// Process-image metadata.
#[derive(Clone, Debug, Default)]
pub struct IoMetadata {
    /// Runtime backend selected by `[io].backend`.
    pub backend: String,
    /// File used by the `file` backend, relative to the project root.
    pub file: Option<String>,
    /// Input image size.
    pub inputs_bytes: u64,
    /// Output image size.
    pub outputs_bytes: u64,
    /// Marker image size.
    pub markers_bytes: u64,
    /// Input defaults as lowercase hex.
    pub input_defaults_hex: String,
    /// Output defaults as lowercase hex.
    pub output_defaults_hex: String,
    /// Declared devices in manifest order.
    pub devices: Vec<DeviceMetadata>,
    /// Bindings sorted by area and offset.
    pub bindings: Vec<IoBindingMetadata>,
    /// Embedded machine profile, when `[io].config` selects one.
    pub profile: Option<IoProfileMetadata>,
    /// Resolved route graph from the profile.
    pub routes: Vec<RouteMetadata>,
    /// Resolved `VAR_ACCESS` declarations.
    pub var_access: Vec<VarAccessMetadata>,
}

/// One retained scalar leaf.
#[derive(Clone, Debug)]
pub struct RetainLeafMetadata {
    /// Readable path.
    pub path: String,
    /// Canonical type name.
    pub ty: String,
    /// Payload offset.
    pub offset: u64,
    /// Leaf size.
    pub size: u64,
}

/// Retained-state metadata.
#[derive(Clone, Debug, Default)]
pub struct RetainMetadata {
    /// Whether retained state is loaded and saved.
    pub enabled: bool,
    /// State file relative to the project root (or the module directory for a
    /// standalone launcher).
    pub file: String,
    /// `error` or `reset`.
    pub on_mismatch: String,
    /// Payload size.
    pub size: u64,
    /// FNV-1a layout hash.
    pub layout_hash: u64,
    /// Leaves sorted by path.
    pub leaves: Vec<RetainLeafMetadata>,
}

/// One exported symbol.
#[derive(Clone, Debug)]
pub struct ExportMetadata {
    /// Internal function symbol.
    pub function: String,
    /// C symbol.
    pub symbol: String,
    /// `no_mangle` flag.
    pub no_mangle: bool,
}

/// One span-table entry.
#[derive(Clone, Debug)]
pub struct SpanMetadataEntry {
    /// Span ID used by trap calls.
    pub id: u32,
    /// File index into [`SpanMetadata::files`].
    pub file: u32,
    /// Start byte.
    pub start: u32,
    /// End byte.
    pub end: u32,
}

/// Span metadata.
#[derive(Clone, Debug, Default)]
pub struct SpanMetadata {
    /// Source file paths in `FileId` order.
    pub files: Vec<String>,
    /// Entries sorted by span ID.
    pub entries: Vec<SpanMetadataEntry>,
}

/// One checkpoint-table entry (Debug and Monitoring §16.2).
#[derive(Clone, Debug)]
pub struct DebugCheckpointMetadata {
    /// Dense ID, 1..N.
    pub id: u32,
    /// Span-table index.
    pub span: u32,
    /// Function `GotId` (`0x…`).
    pub function: String,
    /// 1-based source line.
    pub line: u32,
    /// 1-based source column.
    pub column: u32,
    /// Statement/block/call/back-edge flags.
    pub flags: u32,
}

/// The additive sidecar `debug` object, format 1.
#[derive(Clone, Debug)]
pub struct DebugMetadata {
    /// Paired build identity (`0x…`).
    pub build_id: String,
    /// Debug object format.
    pub format: i64,
    /// Whether service 13 is imported and at least one checkpoint exists.
    pub hooks: bool,
    /// `dwarf`, `codeview`, or `none`.
    pub info: &'static str,
    /// Entries sorted by ID.
    pub checkpoints: Vec<DebugCheckpointMetadata>,
}

/// The complete metadata document.
#[derive(Clone, Debug)]
pub struct Metadata {
    /// Profile identifier.
    pub profile: &'static str,
    /// Compiler version.
    pub compiler: String,
    /// Runtime ABI version.
    pub abi_version: i64,
    /// Artifact name.
    pub artifact: String,
    /// `executable` or `library` host artifact mode.
    pub mode: &'static str,
    /// Content-derived build identity (`0x…`).
    pub build_id: String,
    /// Resolved target triple.
    pub target: String,
    /// Loadable module file name; filled in by the driver, which owns the
    /// output-directory naming convention.
    pub module_file: String,
    /// Paired manifest digest (`0x…`).
    pub manifest_digest: String,
    /// Descriptor digest (`0x…`).
    pub descriptor_digest: String,
    /// Selected configuration name, when any.
    pub configuration: Option<String>,
    /// Program instances.
    pub programs: Vec<ProgramMetadata>,
    /// Tasks.
    pub tasks: Vec<TaskMetadata>,
    /// Process image.
    pub io: IoMetadata,
    /// Retained state.
    pub retain: RetainMetadata,
    /// Exports.
    pub exports: Vec<ExportMetadata>,
    /// Span table.
    pub spans: SpanMetadata,
    /// Debug checkpoint table; absent for release builds.
    pub debug: Option<DebugMetadata>,
}

impl Metadata {
    /// Converts the model to sorted JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        let mut root = BTreeMap::new();
        root.insert(
            "abi_version".into(),
            Json::Number(i128::from(self.abi_version)),
        );
        root.insert("artifact".into(), Json::String(self.artifact.clone()));
        root.insert("compiler".into(), Json::String(self.compiler.clone()));
        root.insert("build_id".into(), Json::String(self.build_id.clone()));
        root.insert(
            "descriptor_digest".into(),
            Json::String(self.descriptor_digest.clone()),
        );
        root.insert(
            "module".into(),
            Json::object([
                ("file".into(), Json::String(self.module_file.clone())),
                ("format".into(), Json::Number(1)),
            ]),
        );
        root.insert(
            "manifest".into(),
            Json::object([
                ("digest".into(), Json::String(self.manifest_digest.clone())),
                ("format".into(), Json::Number(1)),
            ]),
        );
        root.insert("target".into(), Json::String(self.target.clone()));
        root.insert(
            "configuration".into(),
            self.configuration.clone().map_or(Json::Null, Json::String),
        );
        root.insert(
            "exports".into(),
            Json::Array(
                self.exports
                    .iter()
                    .map(|export| {
                        Json::object([
                            ("function".into(), Json::String(export.function.clone())),
                            ("no_mangle".into(), Json::Bool(export.no_mangle)),
                            ("symbol".into(), Json::String(export.symbol.clone())),
                        ])
                    })
                    .collect(),
            ),
        );
        root.insert("io".into(), io_json(&self.io));
        root.insert("mode".into(), Json::String(self.mode.into()));
        root.insert("profile".into(), Json::String(self.profile.into()));
        root.insert(
            "programs".into(),
            Json::Array(
                self.programs
                    .iter()
                    .map(|program| {
                        Json::object([
                            ("name".into(), Json::String(program.name.clone())),
                            ("program".into(), Json::String(program.program.clone())),
                            ("resource".into(), Json::String(program.resource.clone())),
                            (
                                "task".into(),
                                program.task.clone().map_or(Json::Null, Json::String),
                            ),
                        ])
                    })
                    .collect(),
            ),
        );
        root.insert("retain".into(), retain_json(&self.retain));
        root.insert("spans".into(), spans_json(&self.spans));
        if let Some(debug) = &self.debug {
            root.insert("debug".into(), debug_json(debug));
        }
        root.insert(
            "tasks".into(),
            Json::Array(
                self.tasks
                    .iter()
                    .map(|task| {
                        Json::object([
                            (
                                "interval_ns".into(),
                                task.interval_ns.map_or(Json::Null, Json::Number),
                            ),
                            ("name".into(), Json::String(task.name.clone())),
                            (
                                "priority".into(),
                                task.priority.map_or(Json::Null, Json::Number),
                            ),
                            ("resource".into(), Json::String(task.resource.clone())),
                            ("single".into(), task.single.map_or(Json::Null, Json::Bool)),
                        ])
                    })
                    .collect(),
            ),
        );
        Json::Object(root)
    }
}

fn io_json(io: &IoMetadata) -> Json {
    let image = Json::object([
        (
            "inputs_bytes".into(),
            Json::Number(i128::from(io.inputs_bytes)),
        ),
        (
            "markers_bytes".into(),
            Json::Number(i128::from(io.markers_bytes)),
        ),
        (
            "outputs_bytes".into(),
            Json::Number(i128::from(io.outputs_bytes)),
        ),
    ]);
    let devices = io
        .devices
        .iter()
        .map(|device| {
            let channels = device
                .channels
                .iter()
                .map(|channel| {
                    Json::object([
                        ("direction".into(), Json::String(channel.direction.clone())),
                        ("name".into(), Json::String(channel.name.clone())),
                        ("width".into(), Json::Number(i128::from(channel.width))),
                    ])
                })
                .collect();
            Json::object([
                ("config".into(), device.config.clone().unwrap_or(Json::Null)),
                ("kind".into(), Json::String(device.kind.clone())),
                ("name".into(), Json::String(device.name.clone())),
                ("channels".into(), Json::Array(channels)),
            ])
        })
        .collect();
    let bindings = io
        .bindings
        .iter()
        .map(|binding| {
            let hint = binding.hint.as_ref().map_or(Json::Null, |hint| {
                let mut object = BTreeMap::new();
                if let Some(device) = &hint.device {
                    object.insert("device".into(), Json::String(device.clone()));
                }
                if let Some(channel) = hint.channel {
                    object.insert("channel".into(), Json::Number(i128::from(channel)));
                }
                if let Some(direction) = hint.direction {
                    object.insert("direction".into(), Json::String(direction.into()));
                }
                Json::Object(object)
            });
            Json::object([
                ("address".into(), Json::String(binding.address.clone())),
                ("area".into(), Json::String(binding.area.to_string())),
                (
                    "channel".into(),
                    binding.channel.clone().unwrap_or(Json::Null),
                ),
                (
                    "device".into(),
                    binding.device.clone().map_or(Json::Null, Json::String),
                ),
                (
                    "direct_id".into(),
                    binding.direct_id.clone().map_or(Json::Null, Json::String),
                ),
                (
                    "direction".into(),
                    binding.direction.clone().map_or(Json::Null, Json::String),
                ),
                (
                    "got_id".into(),
                    binding.got_id.clone().map_or(Json::Null, Json::String),
                ),
                ("hint".into(), hint),
                (
                    "offset_bits".into(),
                    Json::Number(i128::from(binding.offset_bits)),
                ),
                ("source".into(), Json::String(binding.source.into())),
                ("type".into(), Json::String(binding.ty.clone())),
                (
                    "width_bits".into(),
                    Json::Number(i128::from(binding.width_bits)),
                ),
                ("variable".into(), Json::String(binding.variable.clone())),
            ])
        })
        .collect();
    let var_access = io
        .var_access
        .iter()
        .map(|access| {
            Json::object([
                (
                    "address".into(),
                    access.address.clone().map_or(Json::Null, Json::String),
                ),
                (
                    "area".into(),
                    access
                        .area
                        .map_or(Json::Null, |area| Json::String(area.to_string())),
                ),
                ("name".into(), Json::String(access.name.clone())),
                (
                    "offset_bits".into(),
                    access
                        .offset_bits
                        .map_or(Json::Null, |value| Json::Number(i128::from(value))),
                ),
                ("path".into(), Json::String(access.path.clone())),
                ("read_only".into(), Json::Bool(access.read_only)),
                ("type".into(), Json::String(access.ty.clone())),
                (
                    "width_bits".into(),
                    access
                        .width_bits
                        .map_or(Json::Null, |value| Json::Number(i128::from(value))),
                ),
            ])
        })
        .collect();
    let routes = io
        .routes
        .iter()
        .map(|route| {
            Json::object([
                (
                    "sinks".into(),
                    Json::Array(route.sinks.iter().map(route_endpoint_json).collect()),
                ),
                ("source".into(), route_endpoint_json(&route.source)),
            ])
        })
        .collect();
    let profile = io.profile.as_ref().map_or(Json::Null, |profile| {
        Json::object([
            (
                "devices".into(),
                Json::Array(
                    profile
                        .devices
                        .iter()
                        .map(|device| Json::String(device.clone()))
                        .collect(),
                ),
            ),
            ("digest".into(), Json::String(profile.digest.clone())),
            ("path".into(), Json::String(profile.path.clone())),
            ("text".into(), Json::String(profile.text.clone())),
        ])
    });
    Json::object([
        ("backend".into(), Json::String(io.backend.clone())),
        ("bindings".into(), Json::Array(bindings)),
        ("devices".into(), Json::Array(devices)),
        (
            "file".into(),
            io.file.clone().map_or(Json::Null, Json::String),
        ),
        ("image".into(), image),
        (
            "input_defaults_hex".into(),
            Json::String(io.input_defaults_hex.clone()),
        ),
        (
            "output_defaults_hex".into(),
            Json::String(io.output_defaults_hex.clone()),
        ),
        ("profile".into(), profile),
        ("routes".into(), Json::Array(routes)),
        ("var_access".into(), Json::Array(var_access)),
    ])
}

fn route_endpoint_json(endpoint: &RouteEndpointMetadata) -> Json {
    Json::object([
        (
            "area".into(),
            endpoint
                .area
                .map_or(Json::Null, |area| Json::String(area.to_string())),
        ),
        (
            "channel".into(),
            endpoint.channel.clone().map_or(Json::Null, Json::String),
        ),
        (
            "device".into(),
            endpoint.device.clone().map_or(Json::Null, Json::String),
        ),
        ("kind".into(), Json::String(endpoint.kind.into())),
        (
            "offset_bits".into(),
            endpoint
                .offset_bits
                .map_or(Json::Null, |value| Json::Number(i128::from(value))),
        ),
        (
            "variable".into(),
            endpoint.variable.clone().map_or(Json::Null, Json::String),
        ),
        (
            "width_bits".into(),
            Json::Number(i128::from(endpoint.width_bits)),
        ),
    ])
}

fn retain_json(retain: &RetainMetadata) -> Json {
    Json::object([
        ("enabled".into(), Json::Bool(retain.enabled)),
        ("file".into(), Json::String(retain.file.clone())),
        (
            "on_mismatch".into(),
            Json::String(retain.on_mismatch.clone()),
        ),
        (
            "layout_hash".into(),
            Json::String(format!("0x{:016x}", retain.layout_hash)),
        ),
        (
            "leaves".into(),
            Json::Array(
                retain
                    .leaves
                    .iter()
                    .map(|leaf| {
                        Json::object([
                            ("offset".into(), Json::Number(i128::from(leaf.offset))),
                            ("path".into(), Json::String(leaf.path.clone())),
                            ("size".into(), Json::Number(i128::from(leaf.size))),
                            ("type".into(), Json::String(leaf.ty.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("size".into(), Json::Number(i128::from(retain.size))),
    ])
}

/// Maximum serialized size of the additive `debug` object.
pub(crate) const DEBUG_OBJECT_LIMIT: usize = 64 * 1024 * 1024;

/// Returns the compact serialized size of a debug object.
pub(crate) fn debug_object_len(debug: &DebugMetadata) -> usize {
    debug_json(debug).render().len()
}

fn debug_json(debug: &DebugMetadata) -> Json {
    Json::object([
        ("build_id".into(), Json::String(debug.build_id.clone())),
        (
            "checkpoints".into(),
            Json::Array(
                debug
                    .checkpoints
                    .iter()
                    .map(|checkpoint| {
                        Json::object([
                            ("column".into(), Json::Number(i128::from(checkpoint.column))),
                            ("flags".into(), Json::Number(i128::from(checkpoint.flags))),
                            ("function".into(), Json::String(checkpoint.function.clone())),
                            ("id".into(), Json::Number(i128::from(checkpoint.id))),
                            ("line".into(), Json::Number(i128::from(checkpoint.line))),
                            ("span".into(), Json::Number(i128::from(checkpoint.span))),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("format".into(), Json::Number(i128::from(debug.format))),
        ("hooks".into(), Json::Bool(debug.hooks)),
        ("info".into(), Json::String(debug.info.into())),
    ])
}

fn spans_json(spans: &SpanMetadata) -> Json {
    Json::object([
        (
            "entries".into(),
            Json::Array(
                spans
                    .entries
                    .iter()
                    .map(|entry| {
                        Json::object([
                            ("end".into(), Json::Number(i128::from(entry.end))),
                            ("file".into(), Json::Number(i128::from(entry.file))),
                            ("id".into(), Json::Number(i128::from(entry.id))),
                            ("start".into(), Json::Number(i128::from(entry.start))),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "files".into(),
            Json::Array(
                spans
                    .files
                    .iter()
                    .map(|file| Json::String(file.clone()))
                    .collect(),
            ),
        ),
    ])
}
