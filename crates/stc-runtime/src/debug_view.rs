//! Manifest-assisted split-object presentation (Debug and Monitoring §6.2,
//! §13.4, and §15.5).
//!
//! A [`SplitView`] is the presentation form of one typed read. It carries the
//! same value as [`crate::DebugRead`] together with the storage split that the
//! manifest and the stopped-frame snapshot describe: per-field runtime roots,
//! class/FB hidden handles, opaque identities, and explicit `manifest_assisted`
//! and `contiguous` labels. The view never contains an address, an absolute
//! path, or a timestamp, and both renderers are deterministic.
//!
//! This module is presentation only. Target resolution, decoding, and the
//! decision about which root backs a field stay in the host read path; phase 8
//! may consume the model or the JSON/text rendering. The wire form is an
//! additive `view` key on `var.read` and `frame.read` results (D-182).

use std::fmt::Write as _;

use stc_got::digest::GotId;
use stc_got::error::codes;
use stc_got::json::Json;
use stc_got::model::RootKind;

use crate::debug::{DebugValue, DebugWarning, debug_value_json};

/// The storage kind a node describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewKind {
    /// A scalar, enum, subrange, time/date, or string value.
    Scalar,
    /// `OVERLAP` storage reported as opaque bytes.
    Overlap,
    /// A split struct or a contiguous struct snapshot.
    Aggregate,
    /// A whole configured-program instance.
    Program,
    /// An array, resolved element by element when split.
    Array,
    /// A class object.
    Class,
    /// A function-block object.
    FunctionBlock,
    /// An interface value.
    Interface,
    /// A `REF_TO` or `PTR_TO` value.
    Reference,
    /// A value the manifest or snapshot cannot reconstruct.
    Unavailable,
}

impl ViewKind {
    /// Returns the stable protocol/text spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Scalar => "scalar",
            Self::Overlap => "overlap",
            Self::Aggregate => "aggregate",
            Self::Program => "program",
            Self::Array => "array",
            Self::Class => "class",
            Self::FunctionBlock => "function_block",
            Self::Interface => "interface",
            Self::Reference => "reference",
            Self::Unavailable => "unavailable",
        }
    }
}

impl ViewKind {
    const fn is_composite(self) -> bool {
        matches!(
            self,
            Self::Aggregate
                | Self::Program
                | Self::Array
                | Self::Class
                | Self::FunctionBlock
                | Self::Interface
        )
    }
}

/// Where a view's values came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewOrigin {
    /// Values copied from the active generation's live storage.
    Live,
    /// Values copied from one immutable stopped-frame snapshot.
    StopSnapshot,
}

impl ViewOrigin {
    /// Returns the stable protocol/text spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::StopSnapshot => "stop_snapshot",
        }
    }
}

/// The identity kind a class/FB/interface/reference node carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityKind {
    /// A class object handle.
    Class,
    /// A function-block object record.
    FunctionBlock,
    /// An interface value.
    Interface,
    /// A `PTR_TO` value.
    Pointer,
    /// A `REF_TO` value.
    Reference,
}

impl IdentityKind {
    /// Returns the stable protocol/text spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::FunctionBlock => "function_block",
            Self::Interface => "interface",
            Self::Pointer => "pointer",
            Self::Reference => "reference",
        }
    }
}

/// One runtime root backing a field or object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewRoot {
    /// Persistent identity of the backing root.
    pub got_id: GotId,
    /// Canonical manifest key.
    pub key: String,
    /// Root storage kind.
    pub kind: RootKind,
    /// `HANDLE` or `OBJREC` for a hidden object root.
    pub marker: Option<String>,
}

/// Opaque object/reference identity metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewIdentity {
    /// Identity kind.
    pub kind: IdentityKind,
    /// Deterministic opaque identity, never an address.
    pub identity: Option<String>,
    /// Whether the handle or reference was non-null.
    pub present: bool,
    /// Class type key, when this is a class object.
    pub class: Option<String>,
    /// Interface type key, when this is an interface value.
    pub interface: Option<String>,
    /// Referenced type key, when this is a reference or pointer.
    pub target_type: Option<String>,
    /// Hidden `HANDLE`/`OBJREC` roots, in that order.
    pub roots: Vec<ViewRoot>,
}

/// One named field in declared order.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewField {
    /// Normalized field name.
    pub name: String,
    /// Normalized path relative to the declaration.
    pub path: String,
    /// Field view.
    pub view: ViewNode,
}

/// One node in a split-object view.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewNode {
    /// Storage kind.
    pub kind: ViewKind,
    /// Canonical type key.
    pub type_key: String,
    /// Layout hash, or [`GotId::ZERO`] when the manifest has no layout record.
    pub layout_hash: GotId,
    /// Typed value in the §15.5 model.
    pub value: DebugValue,
    /// Whether a value is available.
    pub available: bool,
    /// Whether the split identity came from manifest records.
    pub manifest_assisted: bool,
    /// Whether this node is one contiguous copy.
    pub contiguous: bool,
    /// Whether this node was one atomic copy.
    pub atomic: bool,
    /// Whether this node came from one generation and cycle.
    pub consistent: bool,
    /// Backing runtime root, when this node has its own persistent root.
    pub root: Option<ViewRoot>,
    /// Object/reference identity metadata.
    pub identity: Option<ViewIdentity>,
    /// Fields in declared order.
    pub fields: Vec<ViewField>,
    /// Array elements in declared index order.
    pub elements: Vec<ViewNode>,
}

impl ViewNode {
    /// Returns whether this node is an unavailable value.
    #[must_use]
    pub fn is_unavailable(&self) -> bool {
        matches!(self.value, DebugValue::Unavailable { .. })
    }
}

/// One manifest-assisted split-object view.
#[derive(Clone, Debug, PartialEq)]
pub struct SplitView {
    /// The target as requested.
    pub target: String,
    /// Canonical manifest key, or the stopped-local path.
    pub resolved_key: String,
    /// Generation copied.
    pub generation: u64,
    /// Build copied.
    pub build_id: GotId,
    /// Cycle copied.
    pub cycle: u64,
    /// Whether the copy came from live storage or a stop snapshot.
    pub origin: ViewOrigin,
    /// Whether the whole target was one atomic copy.
    pub atomic: bool,
    /// Whether every leaf came from the reported generation and cycle.
    pub consistent: bool,
    /// Whether the split identity came from manifest records.
    pub manifest_assisted: bool,
    /// Whether the target is one contiguous storage copy.
    pub contiguous: bool,
    /// Root node.
    pub node: ViewNode,
}

impl SplitView {
    /// Returns an approximate encoded size, used by the event queue budget.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.target.len() + self.resolved_key.len() + node_bytes(&self.node) + 192
    }
}

fn node_bytes(node: &ViewNode) -> usize {
    let mut total = node.type_key.len() + 160;
    total = total.saturating_add(match &node.value {
        DebugValue::Decimal(text) => text.len(),
        DebugValue::String(text) => text.text.as_ref().map_or(32, String::len),
        DebugValue::Overlap { base64, .. } => base64.len(),
        DebugValue::Unavailable { warning } => warning.message.len() + 32,
        DebugValue::Reference {
            identity,
            target_type,
            ..
        } => identity.as_ref().map_or(0, String::len) + target_type.as_ref().map_or(0, String::len),
        _ => 0,
    });
    for field in &node.fields {
        total = total
            .saturating_add(field.name.len() + field.path.len())
            .saturating_add(node_bytes(&field.view));
    }
    for element in &node.elements {
        total = total.saturating_add(node_bytes(element));
    }
    if let Some(identity) = &node.identity {
        total = total
            .saturating_add(identity.identity.as_ref().map_or(0, String::len))
            .saturating_add(identity.class.as_ref().map_or(0, String::len))
            .saturating_add(identity.interface.as_ref().map_or(0, String::len))
            .saturating_add(identity.target_type.as_ref().map_or(0, String::len))
            .saturating_add(
                identity
                    .roots
                    .iter()
                    .map(|root| root.key.len() + 32)
                    .sum::<usize>(),
            );
    }
    total
}

/// Encodes one view as the additive `stc-debug/1` `view` object.
///
/// Composite nodes carry `fields`/`elements` and no duplicate `value` key;
/// scalar/reference/overlap/unavailable nodes carry `value`. Objects sort keys
/// through [`Json`], so the encoding is deterministic.
#[must_use]
pub fn view_json(view: &SplitView) -> Json {
    let mut object = std::collections::BTreeMap::new();
    object.insert("target".into(), Json::string(view.target.clone()));
    object.insert(
        "resolved_key".into(),
        Json::string(view.resolved_key.clone()),
    );
    object.insert("generation".into(), Json::unsigned(view.generation));
    object.insert("build_id".into(), Json::string(view.build_id.to_hex()));
    object.insert("cycle".into(), Json::unsigned(view.cycle));
    object.insert("origin".into(), Json::string(view.origin.name()));
    object.insert("atomic".into(), Json::Bool(view.atomic));
    object.insert("consistent".into(), Json::Bool(view.consistent));
    object.insert(
        "manifest_assisted".into(),
        Json::Bool(view.manifest_assisted),
    );
    object.insert("contiguous".into(), Json::Bool(view.contiguous));
    object.insert("view".into(), node_json(&view.node));
    Json::Object(object)
}

/// Encodes one node. Exposed for phase 8 presentation code.
#[must_use]
pub fn node_json(node: &ViewNode) -> Json {
    let mut object = std::collections::BTreeMap::new();
    object.insert("kind".into(), Json::string(node.kind.name()));
    object.insert("type".into(), Json::string(node.type_key.clone()));
    object.insert(
        "layout_hash".into(),
        if node.layout_hash == GotId::ZERO {
            Json::Null
        } else {
            Json::string(node.layout_hash.to_hex())
        },
    );
    object.insert("available".into(), Json::Bool(node.available));
    object.insert(
        "manifest_assisted".into(),
        Json::Bool(node.manifest_assisted),
    );
    object.insert("contiguous".into(), Json::Bool(node.contiguous));
    object.insert("atomic".into(), Json::Bool(node.atomic));
    object.insert("consistent".into(), Json::Bool(node.consistent));
    object.insert(
        "root".into(),
        node.root.as_ref().map_or(Json::Null, root_json),
    );
    object.insert(
        "identity".into(),
        node.identity.as_ref().map_or(Json::Null, identity_json),
    );
    object.insert(
        "fields".into(),
        Json::Array(
            node.fields
                .iter()
                .map(|field| {
                    let mut field_object = std::collections::BTreeMap::new();
                    field_object.insert("name".into(), Json::string(field.name.clone()));
                    field_object.insert("path".into(), Json::string(field.path.clone()));
                    if let Json::Object(view) = node_json(&field.view) {
                        field_object.extend(view);
                    }
                    Json::Object(field_object)
                })
                .collect(),
        ),
    );
    object.insert(
        "elements".into(),
        Json::Array(node.elements.iter().map(node_json).collect()),
    );
    if !node.kind.is_composite() {
        object.insert("value".into(), debug_value_json(&node.value));
    }
    Json::Object(object)
}

fn root_json(root: &ViewRoot) -> Json {
    Json::object([
        ("got_id".into(), Json::string(root.got_id.to_hex())),
        ("key".into(), Json::string(root.key.clone())),
        ("kind".into(), Json::string(root.kind.name())),
        (
            "marker".into(),
            root.marker
                .as_ref()
                .map_or(Json::Null, |marker| Json::string(marker.clone())),
        ),
    ])
}

fn identity_json(identity: &ViewIdentity) -> Json {
    Json::object([
        ("kind".into(), Json::string(identity.kind.name())),
        (
            "identity".into(),
            identity
                .identity
                .as_ref()
                .map_or(Json::Null, |text| Json::string(text.clone())),
        ),
        ("present".into(), Json::Bool(identity.present)),
        (
            "class".into(),
            identity
                .class
                .as_ref()
                .map_or(Json::Null, |text| Json::string(text.clone())),
        ),
        (
            "interface".into(),
            identity
                .interface
                .as_ref()
                .map_or(Json::Null, |text| Json::string(text.clone())),
        ),
        (
            "target_type".into(),
            identity
                .target_type
                .as_ref()
                .map_or(Json::Null, |text| Json::string(text.clone())),
        ),
        (
            "roots".into(),
            Json::Array(identity.roots.iter().map(root_json).collect()),
        ),
    ])
}

/// Renders a deterministic, terminal-safe text presentation.
///
/// The result ends with one LF. Dynamic names, type keys, identities, and
/// string values are escaped so a value cannot inject terminal control
/// sequences. No address, timestamp, or absolute filesystem path is included.
#[must_use]
pub fn render_text(view: &SplitView) -> String {
    let mut out = String::new();
    if view.manifest_assisted {
        out.push_str(
            "manifest-assisted split view (per-field runtime roots/handles, not a raw struct dump)\n",
        );
    } else if view.origin == ViewOrigin::StopSnapshot {
        out.push_str("stop-snapshot object view (contiguous snapshot copy, no live roots)\n");
    } else {
        out.push_str("contiguous single-root object view\n");
    }
    let _ = writeln!(out, "target: {}", escape_text(&view.target));
    let _ = writeln!(out, "resolved: {}", escape_text(&view.resolved_key));
    let _ = writeln!(out, "origin: {}", view.origin.name());
    let _ = writeln!(out, "generation: {}", view.generation);
    let _ = writeln!(out, "build: {}", view.build_id.to_hex());
    let _ = writeln!(out, "cycle: {}", view.cycle);
    let _ = writeln!(
        out,
        "atomic: {}  consistent: {}  manifest_assisted: {}  contiguous: {}",
        view.atomic, view.consistent, view.manifest_assisted, view.contiguous
    );
    render_node(&mut out, &view.node, None, 0);
    out
}

/// Decodes one additive wire `view` object and renders it with
/// [`render_text`].
///
/// Returns `None` when the value is not a view object, so a caller can fall
/// back to the typed `value`. The decoded model is presentation-only: an
/// unavailable or overlap node is reconstructed as far as the wire form
/// carries it, and the text renderer never invents a value.
#[must_use]
pub fn render_view_json(value: &Json) -> Option<String> {
    decode_view(value).map(|view| render_text(&view))
}

fn decode_view(value: &Json) -> Option<SplitView> {
    let object = value.as_object()?;
    let target = object.get("target").and_then(Json::as_str)?.to_owned();
    let resolved_key = object
        .get("resolved_key")
        .and_then(Json::as_str)?
        .to_owned();
    let generation = u64::try_from(object.get("generation").and_then(Json::as_number)?).ok()?;
    let build_id = GotId::parse(object.get("build_id").and_then(Json::as_str)?).ok()?;
    let cycle = u64::try_from(object.get("cycle").and_then(Json::as_number)?).ok()?;
    let origin = view_origin_from_name(object.get("origin").and_then(Json::as_str)?)?;
    let boolean = |key: &str| object.get(key).and_then(Json::as_bool);
    Some(SplitView {
        target,
        resolved_key,
        generation,
        build_id,
        cycle,
        origin,
        atomic: boolean("atomic")?,
        consistent: boolean("consistent")?,
        manifest_assisted: boolean("manifest_assisted")?,
        contiguous: boolean("contiguous")?,
        node: decode_node(object.get("view")?)?,
    })
}

fn decode_node(value: &Json) -> Option<ViewNode> {
    let object = value.as_object()?;
    let kind = view_kind_from_name(object.get("kind").and_then(Json::as_str)?)?;
    let type_key = object.get("type").and_then(Json::as_str)?.to_owned();
    let layout_hash = match object.get("layout_hash")? {
        Json::Null => GotId::ZERO,
        Json::String(text) => GotId::parse(text).ok()?,
        _ => return None,
    };
    let boolean = |key: &str| object.get(key).and_then(Json::as_bool);
    let fields = object
        .get("fields")?
        .as_array()?
        .iter()
        .map(|field| {
            let object = field.as_object()?;
            Some(ViewField {
                name: object.get("name").and_then(Json::as_str)?.to_owned(),
                path: object.get("path").and_then(Json::as_str)?.to_owned(),
                view: decode_node(field)?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(ViewNode {
        kind,
        type_key,
        layout_hash,
        value: match object.get("value") {
            Some(value) => decode_node_value(value)?,
            // Composite nodes carry no `value`; the renderer uses the kind.
            None => DebugValue::Aggregate(Vec::new()),
        },
        available: boolean("available")?,
        manifest_assisted: boolean("manifest_assisted")?,
        contiguous: boolean("contiguous")?,
        atomic: boolean("atomic")?,
        consistent: boolean("consistent")?,
        root: match object.get("root")? {
            Json::Null => None,
            other => Some(decode_root(other)?),
        },
        identity: match object.get("identity")? {
            Json::Null => None,
            other => Some(decode_identity(other)?),
        },
        elements: object
            .get("elements")?
            .as_array()?
            .iter()
            .map(decode_node)
            .collect::<Option<Vec<_>>>()?,
        fields,
    })
}

fn decode_node_value(value: &Json) -> Option<DebugValue> {
    if let Json::Object(object) = value {
        if object.get("available").and_then(Json::as_bool) == Some(false) {
            let code = object.get("code").and_then(Json::as_str)?;
            return Some(DebugValue::Unavailable {
                warning: DebugWarning {
                    code: warning_code(code),
                    message: "the value is unavailable".into(),
                },
            });
        }
        if let Some(base64) = object.get("bytes").and_then(Json::as_str) {
            // The text renderer prints the encoded bytes; decoding the payload
            // would add nothing to the presentation surface.
            let padding = base64
                .bytes()
                .rev()
                .take_while(|byte| *byte == b'=')
                .count();
            let length = base64.len().saturating_div(4).saturating_mul(3) - padding;
            return Some(DebugValue::Overlap {
                bytes: vec![0; length],
                base64: base64.to_owned(),
            });
        }
    }
    crate::debug::debug_value_from_json(value, 0).ok()
}

fn warning_code(code: &str) -> &'static str {
    if code == codes::W_DEBUG_UNBOUND {
        codes::W_DEBUG_UNBOUND
    } else if code == codes::W_DEBUG_WATCH_DROPPED {
        codes::W_DEBUG_WATCH_DROPPED
    } else if code == codes::W_DEBUG_MONITORING_ONLY {
        codes::W_DEBUG_MONITORING_ONLY
    } else if code == codes::W_DEBUG_RETAIN {
        codes::W_DEBUG_RETAIN
    } else {
        codes::W_DEBUG_UNAVAILABLE
    }
}

fn decode_root(value: &Json) -> Option<ViewRoot> {
    let object = value.as_object()?;
    Some(ViewRoot {
        got_id: GotId::parse(object.get("got_id").and_then(Json::as_str)?).ok()?,
        key: object.get("key").and_then(Json::as_str)?.to_owned(),
        kind: RootKind::from_name(object.get("kind").and_then(Json::as_str)?)?,
        marker: match object.get("marker")? {
            Json::Null => None,
            Json::String(text) => Some(text.clone()),
            _ => return None,
        },
    })
}

fn decode_identity(value: &Json) -> Option<ViewIdentity> {
    let object = value.as_object()?;
    let optional = |key: &str| match object.get(key) {
        None | Some(Json::Null) => Some(None),
        Some(Json::String(text)) => Some(Some(text.clone())),
        Some(_) => None,
    };
    Some(ViewIdentity {
        kind: identity_kind_from_name(object.get("kind").and_then(Json::as_str)?)?,
        identity: optional("identity")?,
        present: object.get("present").and_then(Json::as_bool)?,
        class: optional("class")?,
        interface: optional("interface")?,
        target_type: optional("target_type")?,
        roots: object
            .get("roots")?
            .as_array()?
            .iter()
            .map(decode_root)
            .collect::<Option<Vec<_>>>()?,
    })
}

fn view_kind_from_name(name: &str) -> Option<ViewKind> {
    Some(match name {
        "scalar" => ViewKind::Scalar,
        "overlap" => ViewKind::Overlap,
        "aggregate" => ViewKind::Aggregate,
        "program" => ViewKind::Program,
        "array" => ViewKind::Array,
        "class" => ViewKind::Class,
        "function_block" => ViewKind::FunctionBlock,
        "interface" => ViewKind::Interface,
        "reference" => ViewKind::Reference,
        "unavailable" => ViewKind::Unavailable,
        _ => return None,
    })
}

fn view_origin_from_name(name: &str) -> Option<ViewOrigin> {
    Some(match name {
        "live" => ViewOrigin::Live,
        "stop_snapshot" => ViewOrigin::StopSnapshot,
        _ => return None,
    })
}

fn identity_kind_from_name(name: &str) -> Option<IdentityKind> {
    Some(match name {
        "class" => IdentityKind::Class,
        "function_block" => IdentityKind::FunctionBlock,
        "interface" => IdentityKind::Interface,
        "pointer" => IdentityKind::Pointer,
        "reference" => IdentityKind::Reference,
        _ => return None,
    })
}

fn render_node(out: &mut String, node: &ViewNode, label: Option<(&str, &str)>, depth: usize) {
    let indent = "  ".repeat(depth);
    match label {
        Some((name, path)) => {
            let _ = write!(
                out,
                "{indent}{} ({}) : {}",
                escape_text(name),
                escape_text(path),
                escape_text(&node.type_key)
            );
        }
        None => {
            let _ = write!(out, "{indent}{}", escape_text(&node.type_key));
        }
    }
    let _ = write!(out, " [{}", node.kind.name());
    if node.manifest_assisted {
        out.push_str(", manifest-assisted");
    }
    if node.kind != ViewKind::Unavailable {
        out.push_str(if node.contiguous {
            ", contiguous"
        } else {
            ", split"
        });
    }
    if node.atomic {
        out.push_str(", atomic");
    }
    if !node.consistent {
        out.push_str(", inconsistent");
    }
    if let Some(root) = &node.root {
        let _ = write!(
            out,
            ", root {} {} {}",
            root.got_id.to_hex(),
            root.kind.name(),
            escape_text(&root.key)
        );
        if let Some(marker) = &root.marker {
            let _ = write!(out, " ({})", escape_text(marker));
        }
    }
    out.push(']');
    if let Some(identity) = &node.identity {
        let _ = write!(
            out,
            " identity {} present {}",
            identity
                .identity
                .as_deref()
                .map_or_else(|| "none".to_owned(), escape_text),
            identity.present
        );
        if let Some(class) = &identity.class {
            let _ = write!(out, " class {}", escape_text(class));
        }
        if let Some(interface) = &identity.interface {
            let _ = write!(out, " interface {}", escape_text(interface));
        }
        if let Some(target_type) = &identity.target_type {
            let _ = write!(out, " target {}", escape_text(target_type));
        }
        if !identity.roots.is_empty() {
            out.push_str(" roots:");
            for root in &identity.roots {
                let _ = write!(
                    out,
                    " {} {}{}",
                    root.got_id.to_hex(),
                    root.kind.name(),
                    root.marker
                        .as_ref()
                        .map_or_else(String::new, |marker| format!("/{}", escape_text(marker)))
                );
            }
        }
    }
    if let DebugValue::Unavailable { warning } = &node.value {
        let _ = write!(
            out,
            " = unavailable {}: {}",
            warning.code,
            escape_text(&warning.message)
        );
    } else if node.available {
        match &node.value {
            DebugValue::Bool(value) => {
                let _ = write!(out, " = {value}");
            }
            DebugValue::Decimal(text) => {
                let _ = write!(out, " = {}", escape_text(text));
            }
            DebugValue::Real(value) => {
                let _ = write!(out, " = {}", real_text(*value));
            }
            DebugValue::String(string) => {
                let text = string
                    .text
                    .as_deref()
                    .map_or_else(|| "<invalid units>".to_owned(), quote_text);
                let _ = write!(out, " = {text}");
                let _ = write!(
                    out,
                    " (units {}, capacity {})",
                    string.length, string.capacity
                );
            }
            DebugValue::Overlap { bytes, base64 } => {
                let _ = write!(out, " = bytes[{}] {}", bytes.len(), escape_text(base64));
            }
            DebugValue::Reference {
                identity,
                present,
                target_type,
            } => {
                let _ = write!(
                    out,
                    " = ref identity {} present {present} target {}",
                    identity
                        .as_deref()
                        .map_or_else(|| "none".to_owned(), escape_text),
                    target_type
                        .as_deref()
                        .map_or_else(|| "none".to_owned(), escape_text)
                );
            }
            DebugValue::Array(_)
            | DebugValue::Aggregate(_)
            | DebugValue::Object { .. }
            | DebugValue::Unavailable { .. } => {}
        }
    }
    out.push('\n');
    for field in &node.fields {
        render_node(
            out,
            &field.view,
            Some((&field.name, &field.path)),
            depth + 1,
        );
    }
    for (index, element) in node.elements.iter().enumerate() {
        let name = format!("[{index}]");
        render_node(out, element, Some((&name, &name)), depth + 1);
    }
}

fn real_text(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value == f64::INFINITY {
        return "INF".into();
    }
    if value == f64::NEG_INFINITY {
        return "-INF".into();
    }
    if value == 0.0 {
        return "0".into();
    }
    value.to_string()
}

/// Escapes control characters and Unicode line/paragraph separators so a
/// value cannot inject terminal control sequences.
#[must_use]
pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_control() || character == '\u{2028}' || character == '\u{2029}' {
            let _ = write!(out, "\\u{{{:x}}}", character as u32);
        } else {
            out.push(character);
        }
    }
    out
}

fn quote_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            other if other.is_control() || other == '\u{2028}' || other == '\u{2029}' => {
                let _ = write!(out, "\\u{{{:x}}}", other as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}
