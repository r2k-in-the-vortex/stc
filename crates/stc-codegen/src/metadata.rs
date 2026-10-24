//! Deterministic metadata model and JSON writer (IO §5, State §6, D-091).

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// A JSON value with sorted object keys.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// An integer number.
    Number(i128),
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
    /// Canonical type name.
    pub ty: String,
    /// `{stc:io}` hint fields.
    pub hint: Option<HintMetadata>,
    /// `pragma` or `project`.
    pub source: &'static str,
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
    /// Bindings sorted by area and offset.
    pub bindings: Vec<IoBindingMetadata>,
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
    /// `executable` or `library`.
    pub mode: &'static str,
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
                ("hint".into(), hint),
                (
                    "offset_bits".into(),
                    Json::Number(i128::from(binding.offset_bits)),
                ),
                ("source".into(), Json::String(binding.source.into())),
                ("type".into(), Json::String(binding.ty.clone())),
                ("variable".into(), Json::String(binding.variable.clone())),
            ])
        })
        .collect();
    Json::object([
        ("backend".into(), Json::String("null".into())),
        ("bindings".into(), Json::Array(bindings)),
        ("image".into(), image),
        (
            "input_defaults_hex".into(),
            Json::String(io.input_defaults_hex.clone()),
        ),
        (
            "output_defaults_hex".into(),
            Json::String(io.output_defaults_hex.clone()),
        ),
    ])
}

fn retain_json(retain: &RetainMetadata) -> Json {
    Json::object([
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
