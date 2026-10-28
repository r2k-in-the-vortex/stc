//! M6.5 phase-7 manifest-assisted split-object view tests.
//!
//! These tests compile real module fixtures and assert the presentation view
//! for split structs, classes, function blocks, interfaces, references,
//! overlap storage, whole programs, nested aggregates, arrays of aggregates,
//! stopped-frame snapshots, unavailable values, and deterministic rendering.

mod common;

use std::collections::BTreeMap;

use stc_got::codes;
use stc_runtime::{
    Candidate, DebugRead, DebugReadRequest, DebugTarget, DebugValue, DebugWarning, IdentityKind,
    LoadMode, LoadOutcome, ManifestStore, NamedValue, PROFILE, SplitView, StopFrame, StoppedLocal,
    StringValue, TypedValue, ViewKind, ViewOrigin, render_text, view_json,
};

use common::{Behavior, BuildFlags, Fixture};

const SOURCE: &str = r#"
TYPE
  Point : STRUCT
    x : DINT;
    y : BOOL;
  END_STRUCT;
  Color : (Red, Green := 5, Blue);
  Small : INT(0..10);
  Limits : ARRAY[1..3] OF DINT;
  Union : STRUCT OVERLAP
    a : DINT;
    b : REAL;
  END_STRUCT;
END_TYPE

INTERFACE IShape
  METHOD Area : DINT
  END_METHOD
END_INTERFACE

VAR_GLOBAL
  gstruct : Point;
  garr : ARRAY[0..3] OF INT;
  gstr : STRING[8];
  genum : Color;
  gsmall : Small;
  gmyint : DINT;
  galias : Point;
  gunion : Union;
  counter : MyCounter;
  obj : Shape;
  iface : IShape;
  gref : REF_TO DINT;
  gflag : BOOL;
  greal : REAL;
  glreal : LREAL;
  gtime : TIME;
  gbig : LINT;
  gword : WORD;
  gpoints : ARRAY[0..1] OF Point;
END_VAR

FUNCTION_BLOCK MyCounter
VAR_INPUT
  inc : DINT;
END_VAR
VAR
  cv : DINT;
END_VAR
  cv := cv + inc;
END_FUNCTION_BLOCK

CLASS Shape
VAR PUBLIC
  id : DINT;
END_VAR
END_CLASS

PROGRAM Main
VAR
  ticks : DINT;
  inner : Point;
  str : STRING[4];
  arr : ARRAY[1..2] OF DINT;
  points : ARRAY[1..2] OF Point;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

const PROGRAM_QUALIFIER: &str = "_DEFAULT/_DEFAULT/_DEFAULT";

fn platform() -> stc_runtime::HostPlatform {
    stc_runtime::HostPlatform::new(
        PROFILE,
        common::target(),
        stc_got::HOST_ABI_MAJOR,
        stc_got::HOST_ABI_MINOR,
    )
}

fn in_memory_host() -> stc_runtime::Host {
    stc_runtime::Host::new(platform(), ManifestStore::memory())
}

fn fixture(name: &str, artifacts: stc_codegen::module::ModuleArtifacts) -> Option<Fixture> {
    Fixture::build(name, artifacts, Behavior::default())
}

fn open(fixture: &Fixture) -> Candidate {
    Candidate::open(&platform(), &fixture.module_path, &fixture.manifest_path).expect("candidate")
}

fn cold_install(host: &mut stc_runtime::Host, fixture: &Fixture) -> u64 {
    let outcome = host
        .load(open(fixture), LoadMode::Auto)
        .expect("cold install");
    let LoadOutcome::ColdInstalled { generation } = outcome else {
        panic!("expected a cold install, got {outcome:?}");
    };
    generation
}

fn root<'a>(
    artifacts: &'a stc_codegen::module::ModuleArtifacts,
    key: &str,
) -> &'a stc_got::RootRecord {
    artifacts
        .manifest
        .roots
        .iter()
        .find(|root| root.key == key)
        .unwrap_or_else(|| panic!("root `{key}`"))
}

fn put(
    host: &mut stc_runtime::Host,
    artifacts: &stc_codegen::module::ModuleArtifacts,
    key: &str,
    bytes: &[u8],
) {
    let root = root(artifacts, key);
    let storage = host.root_storage_mut(root.got_id).expect("root storage");
    assert_eq!(storage.len(), bytes.len(), "storage size for `{key}`");
    storage.copy_from_slice(bytes);
}

fn put_i32(
    host: &mut stc_runtime::Host,
    artifacts: &stc_codegen::module::ModuleArtifacts,
    key: &str,
    value: i32,
) {
    put(host, artifacts, key, &value.to_le_bytes());
}

fn put_u64(
    host: &mut stc_runtime::Host,
    artifacts: &stc_codegen::module::ModuleArtifacts,
    key: &str,
    value: u64,
) {
    put(host, artifacts, key, &value.to_le_bytes());
}

fn type_key(artifacts: &stc_codegen::module::ModuleArtifacts, key: &str) -> String {
    artifacts
        .manifest
        .types
        .iter()
        .find(|ty| ty.key == key)
        .unwrap_or_else(|| panic!("type `{key}`"))
        .type_key
        .clone()
}

fn read_view(host: &stc_runtime::Host, path: &str) -> DebugRead {
    host.debug_read(DebugReadRequest::new(DebugTarget::path(path)).with_view())
        .expect("read with view")
}

fn view(read: &DebugRead) -> &SplitView {
    read.view.as_deref().expect("split view")
}

fn field<'a>(node: &'a stc_runtime::ViewNode, name: &str) -> &'a stc_runtime::ViewField {
    node.fields
        .iter()
        .find(|field| field.name == name)
        .unwrap_or_else(|| panic!("field `{name}`"))
}

fn decimal(value: &DebugValue) -> &str {
    match value {
        DebugValue::Decimal(text) => text,
        other => panic!("expected decimal, got {other:?}"),
    }
}

fn split_fixture(name: &str) -> Option<(stc_codegen::module::ModuleArtifacts, Fixture)> {
    let artifacts = common::artifacts(SOURCE, name);
    let fixture = fixture(name, artifacts.clone())?;
    Some((artifacts, fixture))
}

fn populate_split(host: &mut stc_runtime::Host, artifacts: &stc_codegen::module::ModuleArtifacts) {
    put_i32(host, artifacts, "_ROOT::GSTRUCT.X", 11);
    put(host, artifacts, "_ROOT::GSTRUCT.Y", &[1]);
    put_i32(host, artifacts, "_ROOT::GALIAS.X", 12);
    put(host, artifacts, "_ROOT::GALIAS.Y", &[0]);
    put_i32(host, artifacts, "_ROOT::OBJ.ID", 77);
    put_u64(host, artifacts, "_ROOT::OBJ#HANDLE", 0x1122_3344_5566_7788);
    let mut iface = [0_u8; 16];
    iface[0] = 1;
    iface[8] = 2;
    put(host, artifacts, "_ROOT::IFACE", &iface);
    let mut reference = [0_u8; 48];
    reference[..4].copy_from_slice(&2_i32.to_le_bytes());
    reference[8..16].copy_from_slice(&0x40_i64.to_le_bytes());
    reference[16..24].copy_from_slice(&7_i64.to_le_bytes());
    reference[24..32].copy_from_slice(&9_i64.to_le_bytes());
    put(host, artifacts, "_ROOT::GREF", &reference);
    put(host, artifacts, "_ROOT::GUNION", &[1, 2, 3, 4]);
    put_i32(host, artifacts, "_ROOT::COUNTER.CV", 3);
    put_i32(host, artifacts, "_ROOT::COUNTER.INC", 4);
    let ticks = format!("_ROOT::MAIN.TICKS#{PROGRAM_QUALIFIER}");
    put_i32(host, artifacts, &ticks, 19);
    let inner_x = format!("_ROOT::MAIN.INNER.X#{PROGRAM_QUALIFIER}");
    let inner_y = format!("_ROOT::MAIN.INNER.Y#{PROGRAM_QUALIFIER}");
    put_i32(host, artifacts, &inner_x, 20);
    put(host, artifacts, &inner_y, &[1]);
    let mut program_array = [0_u8; 8];
    program_array[..4].copy_from_slice(&5_i32.to_le_bytes());
    program_array[4..].copy_from_slice(&6_i32.to_le_bytes());
    let arr = format!("_ROOT::MAIN.ARR#{PROGRAM_QUALIFIER}");
    put(host, artifacts, &arr, &program_array);
}

#[test]
fn split_struct_view_exposes_per_field_roots_and_flags() {
    let Some((artifacts, fixture)) = split_fixture("debug-view-struct") else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &fixture);
    populate_split(&mut host, &artifacts);

    let structure = read_view(&host, "GSTRUCT");
    let split = view(&structure);
    assert_eq!(split.target, "GSTRUCT");
    assert_eq!(split.origin, ViewOrigin::Live);
    assert_eq!(split.generation, structure.generation);
    assert_eq!(split.build_id, structure.build_id);
    assert_eq!(split.cycle, structure.cycle);
    assert!(split.manifest_assisted);
    assert!(!split.contiguous);
    assert!(!split.atomic);
    assert!(split.consistent);
    assert_eq!(split.node.kind, ViewKind::Aggregate);
    assert_eq!(split.node.type_key, type_key(&artifacts, "_ROOT::POINT"));
    assert_eq!(
        split.node.layout_hash,
        artifacts
            .manifest
            .types
            .iter()
            .find(|ty| ty.key == "_ROOT::POINT")
            .expect("point type")
            .layout_hash
    );
    assert!(split.node.manifest_assisted);
    assert!(!split.node.contiguous);

    assert_eq!(
        split
            .node
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        vec!["X", "Y"]
    );
    let x = field(&split.node, "X");
    assert_eq!(x.path, "GSTRUCT.X");
    assert!(x.view.contiguous);
    assert!(x.view.atomic);
    assert!(!x.view.manifest_assisted);
    assert!(x.view.identity.is_none());
    let x_root = x.view.root.as_ref().expect("X root");
    let manifest_root = root(&artifacts, "_ROOT::GSTRUCT.X");
    assert_eq!(x_root.got_id, manifest_root.got_id);
    assert_eq!(x_root.key, "_ROOT::GSTRUCT.X");
    assert_eq!(x_root.kind, manifest_root.root_kind);
    assert_eq!(x_root.marker, None);
    assert_eq!(decimal(&x.view.value), "11");

    let y = field(&split.node, "Y");
    assert_eq!(y.path, "GSTRUCT.Y");
    assert_eq!(
        y.view.root.as_ref().expect("Y root").key,
        "_ROOT::GSTRUCT.Y"
    );
    assert_eq!(y.view.value, DebugValue::Bool(true));

    let text = render_text(split);
    assert!(
        text.starts_with(
            "manifest-assisted split view (per-field runtime roots/handles, not a raw struct dump)\n"
        ),
        "{text}"
    );
    assert!(text.contains("_ROOT::GSTRUCT.X"), "{text}");
    assert!(text.contains("_ROOT::GSTRUCT.Y"), "{text}");
    assert!(!text.contains("1122334455667788"));

    // The view is opt-in on the host API.
    let plain = host
        .debug_read(DebugReadRequest::new(DebugTarget::path("GSTRUCT")))
        .expect("read without view");
    assert!(plain.view.is_none());
    assert_eq!(plain.value, structure.value);
}

#[test]
fn class_fb_interface_reference_and_overlap_views_use_handles_not_addresses() {
    let Some((artifacts, fixture)) = split_fixture("debug-view-objects") else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &fixture);
    populate_split(&mut host, &artifacts);

    let object = read_view(&host, "OBJ");
    let split = view(&object);
    assert_eq!(split.node.kind, ViewKind::Class);
    assert!(split.manifest_assisted);
    assert!(!split.contiguous);
    let identity = split.node.identity.as_ref().expect("class identity");
    assert_eq!(identity.kind, IdentityKind::Class);
    assert!(identity.present);
    assert!(identity.identity.is_some());
    assert_eq!(
        identity.class.as_deref(),
        Some(type_key(&artifacts, "_ROOT::SHAPE").as_str())
    );
    assert!(identity.interface.is_none());
    let handle = identity
        .roots
        .iter()
        .find(|root| root.marker.as_deref() == Some("HANDLE"))
        .expect("HANDLE root");
    assert_eq!(handle.key, "_ROOT::OBJ#HANDLE");
    assert_eq!(handle.got_id, root(&artifacts, "_ROOT::OBJ#HANDLE").got_id);
    let id_field = field(&split.node, "ID");
    assert_eq!(
        id_field.view.root.as_ref().expect("ID root").key,
        "_ROOT::OBJ.ID"
    );
    assert_eq!(decimal(&id_field.view.value), "77");
    let rendered = render_text(split);
    assert!(!rendered.contains("1122334455667788"), "{rendered}");
    assert!(!rendered.contains("address"), "{rendered}");

    let fb = read_view(&host, "COUNTER");
    let split = view(&fb);
    assert_eq!(split.node.kind, ViewKind::FunctionBlock);
    assert!(!split.contiguous);
    let identity = split.node.identity.as_ref().expect("FB identity");
    assert_eq!(identity.kind, IdentityKind::FunctionBlock);
    let record = identity
        .roots
        .iter()
        .find(|root| root.marker.as_deref() == Some("OBJREC"))
        .expect("OBJREC root");
    assert_eq!(
        record.got_id,
        root(&artifacts, "_ROOT::COUNTER#OBJREC").got_id
    );
    for name in ["CV", "INC"] {
        let field = field(&split.node, name);
        let expected = format!("_ROOT::COUNTER.{name}");
        assert_eq!(
            field.view.root.as_ref().expect("field root").key,
            expected,
            "field {name}"
        );
    }

    let iface = read_view(&host, "IFACE");
    let split = view(&iface);
    assert_eq!(split.node.kind, ViewKind::Interface);
    assert!(split.node.contiguous);
    assert!(!split.node.manifest_assisted);
    assert_eq!(
        split.node.root.as_ref().expect("interface root").key,
        "_ROOT::IFACE"
    );
    let identity = split.node.identity.as_ref().expect("interface identity");
    assert_eq!(identity.kind, IdentityKind::Interface);
    assert!(identity.present);
    assert_eq!(
        identity.interface.as_deref(),
        Some(type_key(&artifacts, "_ROOT::ISHAPE").as_str())
    );
    assert!(identity.class.is_none());

    let reference = read_view(&host, "GREF");
    let split = view(&reference);
    assert_eq!(split.node.kind, ViewKind::Reference);
    assert!(split.node.contiguous);
    assert!(!split.node.atomic);
    assert_eq!(
        split.node.root.as_ref().expect("reference root").key,
        "_ROOT::GREF"
    );
    let identity = split.node.identity.as_ref().expect("reference identity");
    assert_eq!(identity.kind, IdentityKind::Reference);
    assert!(identity.present);
    assert_eq!(identity.target_type.as_deref(), Some("DINT"));
    let rendered = render_text(split);
    assert!(!rendered.contains("0000000000000040"), "{rendered}");
    assert!(!rendered.contains("address"), "{rendered}");

    let overlap = read_view(&host, "GUNION");
    let split = view(&overlap);
    assert_eq!(split.node.kind, ViewKind::Overlap);
    assert!(split.node.contiguous);
    assert!(split.node.atomic);
    assert_eq!(
        split.node.root.as_ref().expect("overlap root").key,
        "_ROOT::GUNION"
    );
    assert_eq!(
        split.node.value,
        DebugValue::Overlap {
            bytes: vec![1, 2, 3, 4],
            base64: "AQIDBA==".into(),
        }
    );
}

#[test]
fn program_nested_and_array_aggregate_views_follow_manifest_shape() {
    let Some((artifacts, fixture)) = split_fixture("debug-view-program") else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &fixture);
    populate_split(&mut host, &artifacts);

    let whole = read_view(&host, "MAIN");
    let split = view(&whole);
    assert_eq!(split.node.kind, ViewKind::Program);
    assert!(split.manifest_assisted);
    assert!(!split.contiguous);
    assert!(!split.atomic);
    let shape = artifacts
        .manifest
        .aggregates
        .iter()
        .find(|shape| shape.path == "MAIN")
        .expect("program shape");
    assert_eq!(split.node.type_key, shape.shape_key);
    assert_eq!(split.node.layout_hash, shape.layout_hash);
    assert_eq!(
        split
            .node
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        shape
            .fields
            .iter()
            .map(|field| field.path.as_str())
            .collect::<Vec<_>>()
    );

    let ticks = field(&split.node, "TICKS");
    let ticks_root = format!("_ROOT::MAIN.TICKS#{PROGRAM_QUALIFIER}");
    assert_eq!(
        ticks.view.root.as_ref().expect("TICKS root").key,
        ticks_root
    );
    assert_eq!(decimal(&ticks.view.value), "19");

    let inner = field(&split.node, "INNER");
    assert_eq!(inner.view.kind, ViewKind::Aggregate);
    assert!(!inner.view.contiguous);
    assert!(inner.view.manifest_assisted);
    assert_eq!(decimal(&field(&inner.view, "X").view.value), "20");
    let inner_x_root = format!("_ROOT::MAIN.INNER.X#{PROGRAM_QUALIFIER}");
    assert_eq!(
        field(&inner.view, "X")
            .view
            .root
            .as_ref()
            .expect("INNER.X root")
            .key,
        inner_x_root
    );

    let arr = field(&split.node, "ARR");
    assert_eq!(arr.view.kind, ViewKind::Array);
    assert!(arr.view.contiguous);
    assert!(arr.view.atomic);
    let arr_root = format!("_ROOT::MAIN.ARR#{PROGRAM_QUALIFIER}");
    assert_eq!(arr.view.root.as_ref().expect("ARR root").key, arr_root);
    assert_eq!(arr.view.elements.len(), 2);
    assert_eq!(decimal(&arr.view.elements[0].value), "5");
    assert_eq!(decimal(&arr.view.elements[1].value), "6");
    assert!(arr.view.elements[0].contiguous);

    let points = field(&split.node, "POINTS");
    assert_eq!(points.view.kind, ViewKind::Array);
    assert!(!points.view.contiguous);
    assert!(points.view.manifest_assisted);
    assert_eq!(points.view.elements.len(), 2);
    let element = &points.view.elements[0];
    assert_eq!(element.kind, ViewKind::Aggregate);
    let element_x = field(element, "X");
    assert_eq!(
        element_x.path, "MAIN.POINTS.E0.X",
        "unexpected element field path"
    );
    assert_eq!(
        element_x
            .view
            .root
            .as_ref()
            .expect("POINTS element root")
            .key,
        format!("_ROOT::MAIN.POINTS.E0.X#{PROGRAM_QUALIFIER}")
    );

    let global_points = read_view(&host, "GPOINTS");
    let split = view(&global_points);
    assert_eq!(split.node.kind, ViewKind::Array);
    assert!(split.node.manifest_assisted);
    assert!(!split.node.contiguous);
    let global_shape = artifacts
        .manifest
        .aggregates
        .iter()
        .find(|shape| shape.path == "GPOINTS")
        .expect("GPOINTS shape");
    assert_eq!(split.node.layout_hash, global_shape.layout_hash);
    assert_eq!(split.node.elements.len(), 2);
    let element = &split.node.elements[0];
    assert_eq!(element.kind, ViewKind::Aggregate);
    assert_eq!(
        field(element, "X")
            .view
            .root
            .as_ref()
            .expect("GPOINTS element root")
            .key,
        "_ROOT::GPOINTS.E0.X"
    );
}

#[test]
fn stopped_locals_render_contiguous_snapshot_views_and_w002() {
    let Some((artifacts, fixture)) = split_fixture("debug-view-locals") else {
        return;
    };
    let mut host = in_memory_host();
    let generation = cold_install(&mut host, &fixture);
    let build = artifacts.manifest.header.build_id;
    let point_type = type_key(&artifacts, "_ROOT::POINT");

    let mut locals = BTreeMap::new();
    locals.insert(
        "P".into(),
        StoppedLocal::new(
            TypedValue::new(
                point_type.clone(),
                DebugValue::Aggregate(vec![
                    NamedValue {
                        name: "X".into(),
                        value: TypedValue::new("DINT", DebugValue::Decimal("5".into())),
                    },
                    NamedValue {
                        name: "Y".into(),
                        value: TypedValue::new("BOOL", DebugValue::Bool(true)),
                    },
                ]),
            ),
            false,
        ),
    );
    locals.insert(
        "BAD".into(),
        StoppedLocal::new(
            TypedValue::new(
                "DINT",
                DebugValue::Unavailable {
                    warning: DebugWarning {
                        code: codes::W_DEBUG_UNAVAILABLE,
                        message: "optimized out".into(),
                    },
                },
            ),
            false,
        ),
    );
    locals.insert(
        "S".into(),
        StoppedLocal::new(
            TypedValue::new(
                "STRING[8]",
                DebugValue::String(StringValue {
                    capacity: 8,
                    length: 3,
                    units: vec![0x1b, u32::from(b'['), u32::from(b'A')],
                    text: Some("\u{1b}[A".into()),
                }),
            ),
            false,
        ),
    );
    let frame = StopFrame {
        token: "stop-7".into(),
        generation,
        build_id: build,
        cycle: 0,
        locals,
    };

    let aggregate = host
        .debug_read(
            DebugReadRequest::new(DebugTarget::local("stop-7", "p"))
                .with_stop_frame(frame.clone())
                .with_view(),
        )
        .expect("aggregate local");
    let split = view(&aggregate);
    assert_eq!(split.origin, ViewOrigin::StopSnapshot);
    assert!(!split.node.manifest_assisted);
    assert!(split.node.contiguous);
    assert!(!split.atomic);
    assert_eq!(split.node.kind, ViewKind::Aggregate);
    assert_eq!(split.node.type_key, point_type);
    let x = field(&split.node, "X");
    assert_eq!(x.path, "P.X");
    assert!(x.view.contiguous);
    assert!(!x.view.manifest_assisted);
    assert!(x.view.root.is_none());
    assert_eq!(decimal(&x.view.value), "5");
    let rendered = render_text(split);
    assert!(
        rendered
            .starts_with("stop-snapshot object view (contiguous snapshot copy, no live roots)\n"),
        "{rendered}"
    );
    assert!(!rendered.contains("manifest-assisted view"), "{rendered}");

    let unavailable = host
        .debug_read(
            DebugReadRequest::new(DebugTarget::local("stop-7", "bad"))
                .with_stop_frame(frame.clone())
                .with_view(),
        )
        .expect("unavailable local");
    let split = view(&unavailable);
    assert_eq!(split.node.kind, ViewKind::Unavailable);
    assert!(!split.node.available);
    assert!(split.node.fields.is_empty());
    match &split.node.value {
        DebugValue::Unavailable { warning } => {
            assert_eq!(warning.code, codes::W_DEBUG_UNAVAILABLE);
            assert_eq!(warning.message, "optimized out");
        }
        other => panic!("expected unavailable, got {other:?}"),
    }
    let rendered = render_text(split);
    assert!(
        rendered.contains("unavailable STC-W-DEBUG-002"),
        "{rendered}"
    );

    let string = host
        .debug_read(
            DebugReadRequest::new(DebugTarget::local("stop-7", "s"))
                .with_stop_frame(frame)
                .with_view(),
        )
        .expect("string local");
    let split = view(&string);
    let rendered = render_text(split);
    assert!(rendered.contains("\\u{1b}"), "{rendered}");
    assert!(
        !rendered.contains('\u{1b}'),
        "control character leaked: {rendered:?}"
    );
}

#[test]
fn views_render_deterministically_and_never_emit_addresses() {
    let Some((artifacts, fixture)) = split_fixture("debug-view-determinism") else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &fixture);
    populate_split(&mut host, &artifacts);

    let read = read_view(&host, "OBJ");
    let split = view(&read);
    let first_text = render_text(split);
    let second_text = render_text(split);
    assert_eq!(first_text, second_text);
    let first_json = view_json(split).render_compact();
    let second_json = view_json(split).render_compact();
    assert_eq!(first_json, second_json);
    assert!(first_json.contains("\"manifest_assisted\":true"));
    assert!(first_json.contains("\"origin\":\"live\""));
    assert!(first_json.contains("\"_ROOT::OBJ#HANDLE\""));
    assert!(!first_json.contains("1122334455667788"));
    assert!(!first_json.contains("address"));
    assert!(!first_json.contains("timestamp"));
    assert!(!first_json.contains("C:\\"));

    // A `GotId` target produces the same view shape and root metadata.
    let by_id = host
        .debug_read(
            DebugReadRequest::new(DebugTarget::got_id(
                root(&artifacts, "_ROOT::GSTRUCT.X").got_id,
            ))
            .with_view(),
        )
        .expect("GotId target");
    let split = view(&by_id);
    assert_eq!(
        split.target,
        root(&artifacts, "_ROOT::GSTRUCT.X").got_id.to_hex()
    );
    assert_eq!(split.node.kind, ViewKind::Scalar);
    assert!(split.contiguous);
    assert_eq!(
        split.node.root.as_ref().expect("GotId root").key,
        "_ROOT::GSTRUCT.X"
    );
}

#[test]
fn release_builds_keep_manifest_assisted_views() {
    let artifacts = common::artifacts_with_flags(
        SOURCE,
        "debug-view-release",
        BuildFlags {
            release: true,
            ..BuildFlags::default()
        },
    );
    let Some(fixture) = fixture("debug-view-release", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &fixture);
    put_i32(&mut host, &artifacts, "_ROOT::GSTRUCT.X", 21);
    put(&mut host, &artifacts, "_ROOT::GSTRUCT.Y", &[1]);

    let read = read_view(&host, "GSTRUCT");
    let split = view(&read);
    assert_eq!(split.origin, ViewOrigin::Live);
    assert!(split.manifest_assisted);
    assert!(!split.contiguous);
    assert_eq!(decimal(&field(&split.node, "X").view.value), "21");
    assert_eq!(
        field(&split.node, "X")
            .view
            .root
            .as_ref()
            .expect("release X root")
            .kind,
        root(&artifacts, "_ROOT::GSTRUCT.X").root_kind
    );
}
