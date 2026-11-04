//! M6.5 phase-3 typed debug access tests.
//!
//! These tests compile real module fixtures and drive the public host debug
//! surface: stable path/`GotId`/local targets, typed values for every §15.5
//! class, split aggregates, objects, interfaces, references, direct variables,
//! generation pinning, service-thread handoff, and watch limits/coalescing.

mod common;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};

use stc_got::{GotId, RootKind, codes};
use stc_runtime::{
    Candidate, DebugRead, DebugReadRequest, DebugTarget, DebugValue, DebugWarning, DirectBinding,
    Host, HostPlatform, LoadMode, LoadOutcome, MAX_WATCHES, MIN_WATCH_INTERVAL_NS, ManifestStore,
    ModuleInfo, PROFILE, StopFrame, StoppedLocal, TypedValue, WatchResult, WatchSpec,
};

use common::{Behavior, Fixture};

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

fn platform() -> HostPlatform {
    HostPlatform::new(
        PROFILE,
        common::target(),
        stc_got::HOST_ABI_MAJOR,
        stc_got::HOST_ABI_MINOR,
    )
}

fn in_memory_host() -> Host {
    Host::new(platform(), ManifestStore::memory())
}

fn fixture(name: &str, artifacts: stc_codegen::module::ModuleArtifacts) -> Option<Fixture> {
    Fixture::build(name, artifacts, Behavior::default())
}

fn open(fixture: &Fixture) -> Candidate {
    Candidate::open(&platform(), &fixture.module_path, &fixture.manifest_path).expect("candidate")
}

fn cold_install(host: &mut Host, fixture: &Fixture) -> u64 {
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

fn put(host: &mut Host, artifacts: &stc_codegen::module::ModuleArtifacts, key: &str, bytes: &[u8]) {
    let root = root(artifacts, key);
    let storage = host.root_storage_mut(root.got_id).expect("root storage");
    assert_eq!(storage.len(), bytes.len(), "storage size for `{key}`");
    storage.copy_from_slice(bytes);
}

fn put_i32(
    host: &mut Host,
    artifacts: &stc_codegen::module::ModuleArtifacts,
    key: &str,
    value: i32,
) {
    put(host, artifacts, key, &value.to_le_bytes());
}

fn put_i64(
    host: &mut Host,
    artifacts: &stc_codegen::module::ModuleArtifacts,
    key: &str,
    value: i64,
) {
    put(host, artifacts, key, &value.to_le_bytes());
}

fn put_u16(
    host: &mut Host,
    artifacts: &stc_codegen::module::ModuleArtifacts,
    key: &str,
    value: u16,
) {
    put(host, artifacts, key, &value.to_le_bytes());
}

fn put_u32(
    host: &mut Host,
    artifacts: &stc_codegen::module::ModuleArtifacts,
    key: &str,
    value: u32,
) {
    put(host, artifacts, key, &value.to_le_bytes());
}

fn put_u64(
    host: &mut Host,
    artifacts: &stc_codegen::module::ModuleArtifacts,
    key: &str,
    value: u64,
) {
    put(host, artifacts, key, &value.to_le_bytes());
}

fn read_path(host: &Host, path: &str) -> stc_got::Result<DebugRead> {
    host.debug_read(DebugReadRequest::new(DebugTarget::path(path)))
}

fn decimal(read: &DebugRead) -> &str {
    match &read.value {
        DebugValue::Decimal(value) => value,
        other => panic!("expected decimal, got {other:?}"),
    }
}

fn fields(read: &DebugRead) -> &[stc_runtime::NamedValue] {
    match &read.value {
        DebugValue::Aggregate(fields) => fields,
        other => panic!("expected aggregate, got {other:?}"),
    }
}

fn field<'a>(fields: &'a [stc_runtime::NamedValue], name: &str) -> &'a TypedValue {
    &fields
        .iter()
        .find(|field| field.name == name)
        .unwrap_or_else(|| panic!("field {name}"))
        .value
}

fn decimal_value(value: &TypedValue) -> &str {
    match &value.value {
        DebugValue::Decimal(value) => value,
        other => panic!("expected decimal, got {other:?}"),
    }
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

#[test]
fn reads_scalars_strings_enums_subranges_and_arrays() {
    let artifacts = common::artifacts(SOURCE, "debug-scalars");
    let Some(fixture) = fixture("debug-scalars", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let generation = cold_install(&mut host, &fixture);

    put_i32(&mut host, &artifacts, "_ROOT::GMYINT", 42);
    put(&mut host, &artifacts, "_ROOT::GFLAG", &[1]);
    put_u32(&mut host, &artifacts, "_ROOT::GREAL", 1.5_f32.to_bits());
    put(
        &mut host,
        &artifacts,
        "_ROOT::GLREAL",
        &f64::NAN.to_le_bytes(),
    );
    put_i64(&mut host, &artifacts, "_ROOT::GTIME", 1_234_567);
    put_i64(&mut host, &artifacts, "_ROOT::GBIG", -9_007_199_254_740_993);
    put_u16(&mut host, &artifacts, "_ROOT::GWORD", 65_535);
    let array = [7_i16, 8, 9, 10];
    put(
        &mut host,
        &artifacts,
        "_ROOT::GARR",
        &array
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    let mut string = [0_u8; 12];
    string[..4].copy_from_slice(&3_i32.to_le_bytes());
    string[4..7].copy_from_slice(b"hey");
    put(&mut host, &artifacts, "_ROOT::GSTR", &string);
    put_u32(&mut host, &artifacts, "_ROOT::GENUM", 5);
    put_u16(&mut host, &artifacts, "_ROOT::GSMALL", 10);

    let counter = read_path(&host, "gmyint").expect("global scalar");
    assert_eq!(counter.resolved_key, "_ROOT::GMYINT");
    assert_eq!(counter.generation, generation);
    assert_eq!(counter.build_id, artifacts.manifest.header.build_id);
    assert_eq!(counter.cycle, 0);
    assert!(counter.atomic);
    assert!(counter.consistent);
    assert_eq!(counter.type_key, "DINT");
    assert_eq!(decimal(&counter), "42");

    let by_id = host
        .debug_read(DebugReadRequest::new(DebugTarget::got_id(
            root(&artifacts, "_ROOT::GMYINT").got_id,
        )))
        .expect("by GotId");
    assert_eq!(decimal(&by_id), "42");

    let flag = read_path(&host, "GFLAG").expect("bool");
    assert_eq!(flag.value, DebugValue::Bool(true));
    assert!(flag.atomic);

    let real = read_path(&host, "GREAL").expect("real");
    assert_eq!(real.value, DebugValue::Real(1.5));
    let lreal = read_path(&host, "GLREAL").expect("lreal");
    assert!(matches!(lreal.value, DebugValue::Real(value) if value.is_nan()));

    let time = read_path(&host, "GTIME").expect("time");
    assert_eq!(decimal(&time), "1234567");
    let big = read_path(&host, "GBIG").expect("lint");
    assert_eq!(decimal(&big), "-9007199254740993");
    let word = read_path(&host, "GWORD").expect("word");
    assert_eq!(decimal(&word), "65535");

    let array_read = read_path(&host, "GARR").expect("array");
    let DebugValue::Array(elements) = &array_read.value else {
        panic!("array");
    };
    assert_eq!(
        elements.iter().map(decimal_value).collect::<Vec<_>>(),
        vec!["7", "8", "9", "10"]
    );
    assert!(array_read.atomic);
    assert_eq!(array_read.type_key, "ARRAY[0..3]OF(INT)");

    let string_read = read_path(&host, "GSTR").expect("string");
    let DebugValue::String(string) = &string_read.value else {
        panic!("string");
    };
    assert_eq!(string.capacity, 8);
    assert_eq!(string.length, 3);
    assert_eq!(string.units, vec![104, 101, 121]);
    assert_eq!(string.text.as_deref(), Some("hey"));
    assert!(string_read.atomic);

    let enum_read = read_path(&host, "GENUM").expect("enum");
    assert_eq!(decimal(&enum_read), "5");
    assert_eq!(enum_read.type_key, type_key(&artifacts, "_ROOT::COLOR"));
    let subrange = read_path(&host, "GSMALL").expect("subrange");
    assert_eq!(decimal(&subrange), "10");
    assert!(subrange.atomic);

    let root_record = root(&artifacts, "_ROOT::GMYINT");
    let exact = host
        .debug_read(
            DebugReadRequest::new(DebugTarget::path("GMYINT"))
                .with_generation(generation)
                .with_build(artifacts.manifest.header.build_id)
                .with_type_key("DINT")
                .with_layout_hash(root_record.layout_hash),
        )
        .expect("matching expectations");
    assert_eq!(decimal(&exact), "42");
    let wrong_type = host
        .debug_read(DebugReadRequest::new(DebugTarget::path("GMYINT")).with_type_key("LINT"))
        .expect_err("wrong type");
    assert_eq!(wrong_type.code, codes::DEBUG_STALE);
    let wrong_generation = host
        .debug_read(
            DebugReadRequest::new(DebugTarget::path("GMYINT")).with_generation(generation + 1),
        )
        .expect_err("wrong generation");
    assert_eq!(wrong_generation.code, codes::DEBUG_STALE);
    let wrong_build = host
        .debug_read(DebugReadRequest::new(DebugTarget::path("GMYINT")).with_build(GotId::ZERO))
        .expect_err("wrong build");
    assert_eq!(wrong_build.code, codes::DEBUG_STALE);
    let unknown = read_path(&host, "NO_SUCH_FIELD").expect_err("unknown");
    assert_eq!(unknown.code, codes::DEBUG_UNKNOWN);
    let unknown_with_expectation = host
        .debug_read(
            DebugReadRequest::new(DebugTarget::path("NO_SUCH_FIELD")).with_generation(generation),
        )
        .expect_err("unknown before generation");
    assert_eq!(unknown_with_expectation.code, codes::DEBUG_UNKNOWN);
    let address = read_path(&host, "0x1234").expect_err("address");
    assert_eq!(address.code, codes::DEBUG_UNKNOWN);

    // An over-capacity string is malformed storage, not a truncated value.
    let mut over = [0_u8; 12];
    over[..4].copy_from_slice(&9_i32.to_le_bytes());
    put(&mut host, &artifacts, "_ROOT::GSTR", &over);
    let error = read_path(&host, "GSTR").expect_err("capacity");
    assert_eq!(error.code, codes::DEBUG_TYPE);

    assert!(matches!(
        host.cycle_at(1_000_000).expect("cycle"),
        stc_runtime::CycleOutcome::Ran { .. }
    ));
    let after = read_path(&host, "GMYINT").expect("after cycle");
    assert_eq!(after.cycle, 1);
}

#[test]
fn reads_split_aggregates_objects_interfaces_references_and_overlap() {
    let artifacts = common::artifacts(SOURCE, "debug-split");
    let Some(fixture) = fixture("debug-split", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &fixture);

    put_i32(&mut host, &artifacts, "_ROOT::GSTRUCT.X", 11);
    put(&mut host, &artifacts, "_ROOT::GSTRUCT.Y", &[1]);
    put_i32(&mut host, &artifacts, "_ROOT::GALIAS.X", 12);
    put(&mut host, &artifacts, "_ROOT::GALIAS.Y", &[0]);
    put_i32(&mut host, &artifacts, "_ROOT::OBJ.ID", 77);
    put_u64(
        &mut host,
        &artifacts,
        "_ROOT::OBJ#HANDLE",
        0x1122_3344_5566_7788,
    );
    let mut iface = [0_u8; 16];
    iface[0] = 1;
    iface[8] = 2;
    put(&mut host, &artifacts, "_ROOT::IFACE", &iface);
    let mut reference = [0_u8; 48];
    reference[..4].copy_from_slice(&2_i32.to_le_bytes());
    reference[8..16].copy_from_slice(&0x40_i64.to_le_bytes());
    reference[16..24].copy_from_slice(&7_i64.to_le_bytes());
    reference[24..32].copy_from_slice(&9_i64.to_le_bytes());
    put(&mut host, &artifacts, "_ROOT::GREF", &reference);
    put(&mut host, &artifacts, "_ROOT::GUNION", &[1, 2, 3, 4]);
    put_i32(&mut host, &artifacts, "_ROOT::COUNTER.CV", 3);
    put_i32(&mut host, &artifacts, "_ROOT::COUNTER.INC", 4);
    put_i32(
        &mut host,
        &artifacts,
        "_ROOT::MAIN.TICKS#_DEFAULT/_DEFAULT/_DEFAULT",
        19,
    );
    put_i32(
        &mut host,
        &artifacts,
        "_ROOT::MAIN.INNER.X#_DEFAULT/_DEFAULT/_DEFAULT",
        20,
    );
    put(
        &mut host,
        &artifacts,
        "_ROOT::MAIN.INNER.Y#_DEFAULT/_DEFAULT/_DEFAULT",
        &[1],
    );
    let mut program_array = [0_u8; 8];
    program_array[..4].copy_from_slice(&5_i32.to_le_bytes());
    program_array[4..].copy_from_slice(&6_i32.to_le_bytes());
    put(
        &mut host,
        &artifacts,
        "_ROOT::MAIN.ARR#_DEFAULT/_DEFAULT/_DEFAULT",
        &program_array,
    );

    let point_type = type_key(&artifacts, "_ROOT::POINT");
    let structure = read_path(&host, "GSTRUCT").expect("split struct");
    assert_eq!(structure.type_key, point_type);
    assert!(!structure.atomic);
    assert!(structure.consistent);
    assert_eq!(decimal_value(field(fields(&structure), "X")), "11");
    assert_eq!(field(fields(&structure), "Y").value, DebugValue::Bool(true));

    let alias = read_path(&host, "GALIAS").expect("alias aggregate");
    assert!(!alias.atomic);
    assert_eq!(decimal_value(field(fields(&alias), "X")), "12");

    let object_type = type_key(&artifacts, "_ROOT::SHAPE");
    let object = read_path(&host, "OBJ").expect("class object");
    assert!(!object.atomic);
    let DebugValue::Object {
        identity,
        present,
        class,
        interface,
        fields: object_fields,
    } = &object.value
    else {
        panic!("object got {:?}", object.value);
    };
    assert!(*present);
    assert_eq!(class.as_deref(), Some(object_type.as_str()));
    assert!(interface.is_none());
    assert_eq!(
        identity
            .as_deref()
            .unwrap_or_else(|| panic!("identity"))
            .len(),
        18
    );
    assert_eq!(decimal_value(field(object_fields, "ID")), "77");

    let object_field = read_path(&host, "OBJ.ID").expect("object scalar");
    assert!(object_field.atomic);
    assert_eq!(decimal(&object_field), "77");

    let fb = read_path(&host, "COUNTER").expect("FB object record");
    assert!(!fb.atomic);
    assert_eq!(decimal_value(field(fields(&fb), "CV")), "3");
    assert_eq!(decimal_value(field(fields(&fb), "INC")), "4");

    let interface_type = type_key(&artifacts, "_ROOT::ISHAPE");
    let iface_read = read_path(&host, "IFACE").expect("interface");
    let DebugValue::Object {
        identity,
        present,
        class,
        interface,
        ..
    } = &iface_read.value
    else {
        panic!("interface got {:?}", iface_read.value);
    };
    assert!(*present);
    assert!(identity.is_some());
    assert!(class.is_none());
    assert_eq!(interface.as_deref(), Some(interface_type.as_str()));

    let reference = read_path(&host, "GREF").expect("reference");
    let DebugValue::Reference {
        identity,
        present,
        target_type,
    } = &reference.value
    else {
        panic!("reference got {:?}", reference.value);
    };
    assert!(*present);
    assert_eq!(target_type.as_deref(), Some("DINT"));
    assert_eq!(
        identity
            .as_deref()
            .unwrap_or_else(|| panic!("identity"))
            .len(),
        18
    );
    assert!(
        !identity
            .as_deref()
            .expect("identity")
            .contains("0000000000000040")
    );

    let overlap = read_path(&host, "GUNION").expect("overlap");
    assert_eq!(
        overlap.value,
        DebugValue::Overlap {
            bytes: vec![1, 2, 3, 4],
            base64: "AQIDBA==".into(),
        }
    );
    assert!(overlap.atomic);

    let program_scalar = read_path(&host, "MAIN.TICKS").expect("program field");
    assert_eq!(decimal(&program_scalar), "19");
    assert_eq!(
        program_scalar.resolved_key,
        "_ROOT::MAIN.TICKS#_DEFAULT/_DEFAULT/_DEFAULT"
    );
    let program_nested = read_path(&host, "MAIN.INNER.X").expect("nested program field");
    assert_eq!(decimal(&program_nested), "20");
    let program_array = read_path(&host, "MAIN.ARR").expect("program array");
    let DebugValue::Array(values) = &program_array.value else {
        panic!("array");
    };
    assert_eq!(
        values.iter().map(decimal_value).collect::<Vec<_>>(),
        vec!["5", "6"]
    );

    let parent = read_path(&host, "MAIN.INNER").expect("typed parent aggregate");
    let DebugValue::Aggregate(fields) = &parent.value else {
        panic!("parent got {:?}", parent.value);
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name, "X");
    assert_eq!(decimal_value(&fields[0].value), "20");
    assert_eq!(fields[1].name, "Y");
    assert_eq!(fields[1].value.value, DebugValue::Bool(true));

    let whole = read_path(&host, "MAIN").expect("whole-program aggregate");
    assert_eq!(whole.type_key, "MAIN");
    let DebugValue::Aggregate(fields) = &whole.value else {
        panic!("program got {:?}", whole.value);
    };
    assert!(fields.iter().any(|field| field.name == "INNER"));
    assert!(fields.iter().any(|field| field.name == "ARR"));
    let points = read_path(&host, "MAIN.POINTS").expect("program array of aggregates");
    let DebugValue::Array(values) = &points.value else {
        panic!("program points");
    };
    assert_eq!(values.len(), 2);
    assert!(matches!(values[0].value, DebugValue::Aggregate(_)));

    let global_points = read_path(&host, "GPOINTS").expect("global array of aggregates");
    let DebugValue::Array(values) = &global_points.value else {
        panic!("global points");
    };
    assert_eq!(values.len(), 2);
    assert!(matches!(values[0].value, DebugValue::Aggregate(_)));
    let element = read_path(&host, "GPOINTS.E1").expect("aggregate array element");
    assert!(matches!(element.value, DebugValue::Aggregate(_)));
}

#[test]
fn locals_require_a_current_stop_frame_snapshot() {
    let artifacts = common::artifacts(SOURCE, "debug-locals");
    let Some(fixture) = fixture("debug-locals", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let generation = cold_install(&mut host, &fixture);
    let build = artifacts.manifest.header.build_id;

    let mut locals = BTreeMap::new();
    locals.insert(
        "X".into(),
        StoppedLocal::new(
            TypedValue::new("DINT", DebugValue::Decimal("9".into())),
            true,
        ),
    );
    locals.insert(
        "OPTIMIZED".into(),
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
    let frame = StopFrame {
        token: "stop-1".into(),
        generation,
        build_id: build,
        cycle: 0,
        locals,
    };
    let request =
        DebugReadRequest::new(DebugTarget::local("stop-1", "x")).with_stop_frame(frame.clone());
    let read = host.debug_read(request).expect("local");
    assert_eq!(decimal(&read), "9");
    assert!(read.atomic);
    assert!(read.consistent);
    assert_eq!(read.cycle, 0);
    assert_eq!(read.resolved_key, "X");

    let missing = host
        .debug_read(DebugReadRequest::new(DebugTarget::local("stop-1", "x")))
        .expect_err("missing frame");
    assert_eq!(missing.code, codes::DEBUG_STOP);
    let wrong_frame = host
        .debug_read(
            DebugReadRequest::new(DebugTarget::local("stop-2", "x")).with_stop_frame(frame.clone()),
        )
        .expect_err("wrong frame");
    assert_eq!(wrong_frame.code, codes::DEBUG_STOP);
    let mut stale = frame.clone();
    stale.generation += 1;
    let stale_error = host
        .debug_read(DebugReadRequest::new(DebugTarget::local("stop-1", "x")).with_stop_frame(stale))
        .expect_err("stale frame");
    assert_eq!(stale_error.code, codes::DEBUG_STOP);
    let mut future = frame.clone();
    future.cycle = 1;
    let future_error = host
        .debug_read(
            DebugReadRequest::new(DebugTarget::local("stop-1", "x")).with_stop_frame(future),
        )
        .expect_err("future frame");
    assert_eq!(future_error.code, codes::DEBUG_STOP);
    let unknown = host
        .debug_read(
            DebugReadRequest::new(DebugTarget::local("stop-1", "nope"))
                .with_stop_frame(frame.clone()),
        )
        .expect_err("unknown local");
    assert_eq!(unknown.code, codes::DEBUG_UNKNOWN);
    let optimized = host
        .debug_read(
            DebugReadRequest::new(DebugTarget::local("stop-1", "optimized"))
                .with_stop_frame(frame.clone()),
        )
        .expect("optimized local");
    match optimized.value {
        DebugValue::Unavailable { warning } => {
            assert_eq!(warning.code, codes::W_DEBUG_UNAVAILABLE);
        }
        other => panic!("expected unavailable, got {other:?}"),
    }
    let wrong_type = host
        .debug_read(
            DebugReadRequest::new(DebugTarget::local("stop-1", "x"))
                .with_stop_frame(frame)
                .with_type_key("LINT"),
        )
        .expect_err("wrong local type");
    assert_eq!(wrong_type.code, codes::DEBUG_STALE);

    let local_watch = WatchSpec::new(DebugTarget::local("stop-1", "x"), MIN_WATCH_INTERVAL_NS)
        .expect_err("local watch spec");
    assert_eq!(local_watch.code, codes::DEBUG_STOP);
}

#[test]
fn service_reads_do_not_block_cycles_and_watch_limits_hold() {
    let artifacts = common::artifacts(SOURCE, "debug-service");
    let Some(fixture) = fixture("debug-service", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let generation = cold_install(&mut host, &fixture);
    put_i32(&mut host, &artifacts, "_ROOT::GMYINT", 31);

    // Submit from a second thread while the cycle thread runs. The facade is
    // Send + Sync and only touches its own queue; the cycle thread applies the
    // reads at its boundary.
    let service = host.debug_service();
    let worker_service = service.clone();
    let barrier = Arc::new(Barrier::new(2));
    let worker_barrier = Arc::clone(&barrier);
    let worker = std::thread::spawn(move || {
        worker_barrier.wait();
        for _ in 0..8 {
            worker_service
                .submit_read(DebugReadRequest::new(DebugTarget::path("GMYINT")))
                .expect("enqueue read");
        }
        worker_service
    });
    barrier.wait();
    for index in 0..8 {
        host.cycle_at(1_000_000 * (index + 1))
            .expect("cycle while servicing");
    }
    let worker_service = worker.join().expect("service thread");
    host.cycle_at(9_000_000).expect("final boundary");
    let events = worker_service.drain_events();
    let reads: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            stc_runtime::DebugEvent::Read(completion) => Some(completion),
            _ => None,
        })
        .collect();
    assert_eq!(reads.len(), 8);
    assert!(reads.iter().all(|completion| {
        completion
            .result
            .as_ref()
            .is_ok_and(|read| decimal(read) == "31")
    }));
    let _ = service;

    // Direct watch path: first pump is due immediately, then 50 ms periods.
    let watch = host
        .debug_add_watch(
            WatchSpec::new(DebugTarget::path("GMYINT"), MIN_WATCH_INTERVAL_NS).expect("watch spec"),
        )
        .expect("watch");
    let events = host.debug_drain_events();
    assert!(events.is_empty());
    assert_eq!(host.debug_pump_at(0), 1);
    let first = host.debug_drain_events();
    let first_batch = batch_event(&first);
    assert_eq!(first_batch.generation, generation);
    assert_eq!(first_batch.build_id, artifacts.manifest.header.build_id);
    assert_eq!(first_batch.cycle, 9);
    assert_eq!(first_batch.values.len(), 1);
    assert!(first_batch.values[0].consistent);
    assert!(first_batch.values[0].atomic);
    assert_eq!(
        first_batch.values[0].result,
        WatchResult::Available(TypedValue::new("DINT", DebugValue::Decimal("31".into())))
    );

    put_i32(&mut host, &artifacts, "_ROOT::GMYINT", 32);
    assert_eq!(host.debug_pump_at(MIN_WATCH_INTERVAL_NS - 1), 0);
    assert!(host.debug_drain_events().is_empty());
    assert_eq!(host.debug_pump_at(MIN_WATCH_INTERVAL_NS), 1);
    let second = host.debug_drain_events();
    let second_batch = batch_event(&second);
    assert_eq!(
        second_batch.values[0].result,
        WatchResult::Available(TypedValue::new("DINT", DebugValue::Decimal("32".into())))
    );
    // Missed periods coalesce into one batch and do not create a backlog.
    put_i32(&mut host, &artifacts, "_ROOT::GMYINT", 33);
    assert_eq!(host.debug_pump_at(10 * MIN_WATCH_INTERVAL_NS), 1);
    let coalesced = host.debug_drain_events();
    assert_eq!(coalesced.len(), 1);
    assert_eq!(host.debug_pump_at(10 * MIN_WATCH_INTERVAL_NS), 0);

    // A queue of more than 64 batches keeps the newest 64 and warns the next
    // delivered batch.
    let batch_limit = i64::try_from(stc_runtime::MAX_WATCH_BATCHES).expect("batch limit");
    for tick in 11..(11 + batch_limit + 4) {
        assert_eq!(host.debug_pump_at(tick * MIN_WATCH_INTERVAL_NS), 1);
    }
    let queued = host.debug_drain_events();
    let batches: Vec<_> = queued
        .iter()
        .filter_map(|event| match event {
            stc_runtime::DebugEvent::Batch(batch) => Some(batch),
            _ => None,
        })
        .collect();
    assert_eq!(batches.len(), stc_runtime::MAX_WATCH_BATCHES);
    assert!(
        batches
            .last()
            .expect("last batch")
            .warnings
            .iter()
            .any(|warning| warning.code == codes::W_DEBUG_WATCH_DROPPED)
    );
    assert!(host.debug_remove_watch(watch));

    // Unknown targets stay watchable but yield a per-value warning.
    let unknown = host
        .debug_add_watch(
            WatchSpec::new(DebugTarget::path("NO_SUCH"), MIN_WATCH_INTERVAL_NS).expect("spec"),
        )
        .expect("unknown watch");
    host.debug_pump_at(0);
    let events = host.debug_drain_events();
    let batch = batch_event(&events);
    match &batch.values[0].result {
        WatchResult::Unavailable { warning, error, .. } => {
            assert_eq!(warning.code, codes::W_DEBUG_UNAVAILABLE);
            assert_eq!(error.as_ref().expect("error").code, codes::DEBUG_UNKNOWN);
        }
        other => panic!("expected unavailable, got {other:?}"),
    }
    assert!(host.debug_remove_watch(unknown));

    let short = WatchSpec::new(DebugTarget::path("GMYINT"), MIN_WATCH_INTERVAL_NS - 1)
        .expect_err("minimum interval");
    assert_eq!(short.code, codes::DEBUG_LIMIT);

    // The 256-watch limit is enforced before taking any cycle lock.
    let mut watches = Vec::new();
    for _ in 0..MAX_WATCHES {
        watches.push(
            host.debug_add_watch(
                WatchSpec::new(DebugTarget::path("GMYINT"), MIN_WATCH_INTERVAL_NS).expect("spec"),
            )
            .expect("watch below limit"),
        );
    }
    let limit = host
        .debug_add_watch(
            WatchSpec::new(DebugTarget::path("GMYINT"), MIN_WATCH_INTERVAL_NS).expect("spec"),
        )
        .expect_err("watch limit");
    assert_eq!(limit.code, codes::DEBUG_LIMIT);
    for watch in watches {
        assert!(host.debug_remove_watch(watch));
    }
}

#[test]
fn pinned_debug_read_keeps_the_generation_alive_and_swap_ends_watches() {
    let artifacts = common::artifacts(SOURCE, "debug-pins");
    let Some(fixture) = fixture("debug-pins", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &fixture);
    put_i32(&mut host, &artifacts, "_ROOT::GMYINT", 5);

    let watch = host
        .debug_add_watch(
            WatchSpec::new(DebugTarget::path("GMYINT"), MIN_WATCH_INTERVAL_NS).expect("watch spec"),
        )
        .expect("watch");
    let pin = host.pin_active().expect("pin active generation");
    let read = host
        .debug_read(DebugReadRequest::new(DebugTarget::path("GMYINT")))
        .expect("pinned typed read");
    assert_eq!(decimal(&read), "5");
    let error = host
        .load(open(&fixture), LoadMode::Cold)
        .expect_err("cold install while pinned");
    assert_eq!(error.code, codes::ONLINE_PREPARE);
    assert!(host.debug_remove_watch(watch));
    assert_eq!(host.debug_watch_count(), 0);

    // A watch added just before the swap is ended by the generation change.
    let watch = host
        .debug_add_watch(
            WatchSpec::new(DebugTarget::path("GMYINT"), MIN_WATCH_INTERVAL_NS).expect("watch spec"),
        )
        .expect("watch");
    drop(pin);
    let outcome = host
        .load(open(&fixture), LoadMode::Cold)
        .expect("cold install after unpin");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));
    assert_eq!(host.debug_watch_count(), 0);
    assert!(!host.debug_remove_watch(watch));

    // Reads still work after the cold install.
    let read = read_path(&host, "GMYINT").expect("post-swap read");
    assert_eq!(decimal(&read), "0");
}

fn batch_event(events: &[stc_runtime::DebugEvent]) -> &stc_runtime::DebugBatch {
    events
        .iter()
        .find_map(|event| match event {
            stc_runtime::DebugEvent::Batch(batch) => Some(batch),
            _ => None,
        })
        .expect("watch batch")
}

const DIRECT_SOURCE: &str = r#"
VAR_GLOBAL
  Input AT %IW0 : WORD;
  Flag AT %IX0.3 : BOOL;
  Marker AT %MD0 : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  Marker := Marker + 1;
END_PROGRAM
"#;

#[test]
fn direct_variables_are_read_through_the_process_image() {
    let _guard = common::image_guard();
    let artifacts = common::artifacts(DIRECT_SOURCE, "debug-direct");
    let Some(fixture) = fixture("debug-direct", artifacts.clone()) else {
        return;
    };
    let platform = platform();
    let input = artifacts
        .manifest
        .roots
        .iter()
        .find(|root| root.root_kind == RootKind::Direct && root.key.ends_with("INPUT"))
        .expect("direct input root");
    let marker = artifacts
        .manifest
        .roots
        .iter()
        .find(|root| root.root_kind == RootKind::Direct && root.key.ends_with("MARKER"))
        .expect("direct marker root");
    let flag = artifacts
        .manifest
        .roots
        .iter()
        .find(|root| root.root_kind == RootKind::Direct && root.key.ends_with("FLAG"))
        .expect("direct flag root");
    let info = ModuleInfo {
        inputs_bytes: 2,
        outputs_bytes: 0,
        markers_bytes: 4,
        bindings: vec![
            DirectBinding {
                got_id: input.got_id,
                area: 1,
                offset_bits: 0,
                width_bits: 16,
                device: None,
                channel: None,
                direction: None,
            },
            DirectBinding {
                got_id: flag.got_id,
                area: 1,
                offset_bits: 3,
                width_bits: 1,
                device: None,
                channel: None,
                direction: None,
            },
            DirectBinding {
                got_id: marker.got_id,
                area: 3,
                offset_bits: 0,
                width_bits: 32,
                device: None,
                channel: None,
                direction: None,
            },
        ],
        ..ModuleInfo::default()
    };
    let candidate = Candidate::open_with_info(
        &platform,
        &fixture.module_path,
        &fixture.manifest_path,
        Some(info),
    )
    .expect("candidate with direct info");
    let mut host = in_memory_host();
    assert!(matches!(
        host.load(candidate, LoadMode::Cold).expect("cold install"),
        LoadOutcome::ColdInstalled { .. }
    ));
    assert!(stc_runtime::image_write(0, &0x1234_u16.to_le_bytes()));
    let read_path_result = read_path(&host, "INPUT").expect("direct input");
    assert_eq!(read_path_result.type_key, input.type_key);
    assert_eq!(decimal(&read_path_result), "4660");
    assert!(read_path_result.atomic);
    assert_eq!(read_path_result.resolved_key, input.key);
    let by_id = host
        .debug_read(DebugReadRequest::new(DebugTarget::got_id(input.got_id)))
        .expect("direct by GotId");
    assert_eq!(decimal(&by_id), "4660");

    // A bit-addressed BOOL shares its byte with other direct variables; only
    // the bound bit is read.
    assert!(stc_runtime::image_write(0, &[0x3c]));
    let flag_read = read_path(&host, "FLAG").expect("direct flag");
    assert_eq!(flag_read.value, DebugValue::Bool(true));
    assert!(flag_read.atomic);
    assert!(stc_runtime::image_write(0, &[0x34]));
    let flag_read = read_path(&host, "FLAG").expect("direct flag");
    assert_eq!(flag_read.value, DebugValue::Bool(false));

    assert!(stc_runtime::image_write(2, &7_i32.to_le_bytes()));
    let marker = host
        .debug_read(DebugReadRequest::new(DebugTarget::got_id(marker.got_id)))
        .expect("direct marker");
    assert_eq!(decimal(&marker), "7");
}

#[test]
fn service_handoff_is_bounded_and_never_exposes_addresses() {
    let artifacts = common::artifacts(SOURCE, "debug-handoff");
    let Some(fixture) = fixture("debug-handoff", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &fixture);
    put_u64(
        &mut host,
        &artifacts,
        "_ROOT::OBJ#HANDLE",
        0xdead_beef_cafe_babe,
    );
    let read = read_path(&host, "OBJ").expect("object");
    let DebugValue::Object { identity, .. } = read.value else {
        panic!("object");
    };
    let identity = identity.expect("identity");
    assert!(identity.starts_with("0x"));
    assert_eq!(identity.len(), 18);
    assert!(!identity.contains("dead"));
    assert!(!identity.contains("cafe"));

    // The service facade is safe to move to a monitor thread.
    let service = host.debug_service();
    let worker = service.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let handle = std::thread::spawn(move || {
        while !worker_stop.load(Ordering::Relaxed) {
            assert!(
                worker
                    .submit_read(DebugReadRequest::new(DebugTarget::path("OBJ")))
                    .is_ok()
            );
            std::thread::yield_now();
        }
    });
    stop.store(true, Ordering::Relaxed);
    handle.join().expect("worker");
    host.cycle_at(1_000_000).expect("boundary");
    let events = service.drain_events();
    assert!(
        events
            .iter()
            .all(|event| !format!("{event:?}").contains("deadbeef"))
    );
}
