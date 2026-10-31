//! M6.5 phase-4 write/force queue, boundary application, and audit tests.
//!
//! The tests compile real modules and drive only the public host debug
//! surface. Writes and forces are enqueued on the host thread and applied by
//! the cycle thread at the boundary; storage is never changed at enqueue.

mod common;

use std::sync::{Arc, Mutex};

use stc_got::error::{Error, Result};
use stc_got::{GotId, Retention, RootKind, codes};
use stc_runtime::{
    AuditEvent, AuditOutcome, AuditSink, Candidate, DebugEndReason, DebugQueueOutcome, DebugTarget,
    DebugValue, DebugWriteKind, DebugWriteRequest, DirectBinding, Host, HostPlatform, IoBackend,
    LoadMode, LoadOutcome, MAX_DEBUG_QUEUE_ENTRIES, ManifestStore, ModuleInfo, PROFILE,
    StringValue, TypedValue,
};

use common::{Behavior, Fixture};

const SOURCE: &str = r#"
TYPE
  Color : (Red, Green := 5, Blue);
  Small : INT(0..10);
  Point : STRUCT
    x : DINT;
    y : BOOL;
  END_STRUCT;
  Union : STRUCT OVERLAP
    a : DINT;
    b : REAL;
  END_STRUCT;
END_TYPE

CLASS Shape
VAR PUBLIC
  id : DINT;
END_VAR
END_CLASS

VAR_GLOBAL RETAIN
  g_retained : DINT;
END_VAR

VAR_GLOBAL
  g_dint : DINT;
  g_bool : BOOL;
  g_enum : Color;
  g_small : Small;
  g_string : STRING[8];
  g_array : ARRAY[1..3] OF DINT;
  g_big : ARRAY[1..1024] OF DINT;
  g_real : REAL;
  g_lreal : LREAL;
  g_word : WORD;
  g_point : Point;
  g_union : Union;
  g_object : Shape;
  g_ref : REF_TO DINT;
END_VAR

PROGRAM Main
VAR
  in : DINT;
  out : DINT;
END_VAR
  out := in;
END_PROGRAM
"#;

const DIRECT_SOURCE: &str = r#"
VAR_GLOBAL
  Input AT %IW0 : WORD;
  Out AT %QW0 : WORD;
  Flag AT %IX2.3 : BOOL;
  seen : WORD;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  seen := Input;
  Out := 1;
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
        .find(|root| root.key == key || root.key.ends_with(&format!("::{key}")))
        .unwrap_or_else(|| panic!("root `{key}`"))
}

fn root_type_key(artifacts: &stc_codegen::module::ModuleArtifacts, key: &str) -> String {
    root(artifacts, key).type_key.clone()
}

fn put(host: &mut Host, artifacts: &stc_codegen::module::ModuleArtifacts, key: &str, bytes: &[u8]) {
    let root = root(artifacts, key);
    let storage = host.root_storage_mut(root.got_id).expect("root storage");
    assert_eq!(storage.len(), bytes.len(), "storage size for `{key}`");
    storage.copy_from_slice(bytes);
}

fn read(host: &Host, path: &str) -> stc_got::Result<stc_runtime::DebugRead> {
    host.debug_read(stc_runtime::DebugReadRequest::new(DebugTarget::path(path)))
}

fn decimal(host: &Host, path: &str) -> String {
    let read = read(host, path).expect("read");
    match read.value {
        DebugValue::Decimal(value) => value,
        other => panic!("expected decimal, got {other:?}"),
    }
}

fn bool_value(host: &Host, path: &str) -> bool {
    let read = read(host, path).expect("read");
    match read.value {
        DebugValue::Bool(value) => value,
        other => panic!("expected bool, got {other:?}"),
    }
}

fn decimal_typed(type_key: &str, value: impl Into<String>) -> TypedValue {
    TypedValue::new(type_key, DebugValue::Decimal(value.into()))
}

fn bool_typed(type_key: &str, value: bool) -> TypedValue {
    TypedValue::new(type_key, DebugValue::Bool(value))
}

fn string_typed(type_key: &str, capacity: u32, units: &[u32]) -> TypedValue {
    TypedValue::new(
        type_key,
        DebugValue::String(StringValue {
            capacity,
            length: u32::try_from(units.len()).expect("length"),
            units: units.to_vec(),
            text: units.iter().map(|unit| char::from_u32(*unit)).collect(),
        }),
    )
}

fn write_request(path: &str, value: TypedValue) -> DebugWriteRequest {
    DebugWriteRequest::new(DebugTarget::path(path), value)
}

fn direct_info(artifacts: &stc_codegen::module::ModuleArtifacts, backend: IoBackend) -> ModuleInfo {
    let input = root(artifacts, "INPUT");
    let output = root(artifacts, "OUT");
    let flag = root(artifacts, "FLAG");
    ModuleInfo {
        inputs_bytes: 3,
        outputs_bytes: 2,
        markers_bytes: 0,
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
                got_id: output.got_id,
                area: 2,
                offset_bits: 0,
                width_bits: 16,
                device: None,
                channel: None,
                direction: None,
            },
            DirectBinding {
                got_id: flag.got_id,
                area: 1,
                offset_bits: 19,
                width_bits: 1,
                device: None,
                channel: None,
                direction: None,
            },
        ],
        backend,
        ..ModuleInfo::default()
    }
}

#[test]
fn writes_apply_in_queue_order_at_the_boundary() {
    let artifacts = common::artifacts(SOURCE, "debug-writes");
    let Some(fixture) = fixture("debug-writes", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let _generation = cold_install(&mut host, &fixture);

    let dint = root_type_key(&artifacts, "G_DINT");
    let bool_type = root_type_key(&artifacts, "G_BOOL");
    let array_type = root_type_key(&artifacts, "G_ARRAY");
    let real_type = root_type_key(&artifacts, "G_REAL");
    let lreal_type = root_type_key(&artifacts, "G_LREAL");
    let string_type = root_type_key(&artifacts, "G_STRING");
    let word_type = root_type_key(&artifacts, "G_WORD");

    assert_eq!(decimal(&host, "G_DINT"), "0");
    let first = host
        .debug_enqueue_write(write_request("G_DINT", decimal_typed(&dint, "1")))
        .expect("first write");
    let second = host
        .debug_enqueue_write(write_request("G_DINT", decimal_typed(&dint, "2")))
        .expect("second write");
    assert_eq!(host.debug_queued_count(), 2);
    // Enqueue never changes storage.
    assert_eq!(decimal(&host, "G_DINT"), "0");
    let outcome = host.cycle().expect("cycle");
    assert!(matches!(outcome, stc_runtime::CycleOutcome::Ran { .. }));
    assert_eq!(decimal(&host, "G_DINT"), "2");
    let results = host.debug_take_queue_results();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].queue_id, first);
    assert_eq!(results[0].outcome, DebugQueueOutcome::Applied);
    assert_eq!(results[1].queue_id, second);
    assert_eq!(results[1].outcome, DebugQueueOutcome::Applied);
    assert_eq!(results[1].operation, DebugWriteKind::Write);
    assert_eq!(results[1].cycle, 0);

    host.debug_enqueue_write(write_request("G_BOOL", bool_typed(&bool_type, true)))
        .expect("bool write");
    host.debug_enqueue_write(write_request(
        "G_ARRAY",
        TypedValue::new(
            &array_type,
            DebugValue::Array(vec![
                decimal_typed("DINT", "11"),
                decimal_typed("DINT", "22"),
                decimal_typed("DINT", "33"),
            ]),
        ),
    ))
    .expect("array write");
    host.debug_enqueue_write(write_request(
        "G_REAL",
        TypedValue::new(&real_type, DebugValue::Real(1.5)),
    ))
    .expect("real write");
    host.debug_enqueue_write(write_request(
        "G_LREAL",
        TypedValue::new(&lreal_type, DebugValue::Real(-2.25)),
    ))
    .expect("lreal write");
    host.debug_enqueue_write(write_request(
        "G_STRING",
        string_typed(&string_type, 8, &[b'O' as u32, b'K' as u32, b'!' as u32]),
    ))
    .expect("string write");
    host.debug_enqueue_write(write_request("G_WORD", decimal_typed(&word_type, "65535")))
        .expect("word write");
    host.cycle().expect("cycle");
    assert!(bool_value(&host, "G_BOOL"));
    let array = read(&host, "G_ARRAY").expect("array read");
    match array.value {
        DebugValue::Array(values) => {
            assert_eq!(values.len(), 3);
            assert_eq!(values[0].type_key, "DINT");
            assert_eq!(values[2].value, DebugValue::Decimal("33".into()));
        }
        other => panic!("expected array, got {other:?}"),
    }
    assert!(matches!(
        read(&host, "G_REAL").expect("real").value,
        DebugValue::Real(value) if (value - 1.5).abs() < 1e-6
    ));
    assert!(matches!(
        read(&host, "G_LREAL").expect("lreal").value,
        DebugValue::Real(value) if (value + 2.25).abs() < 1e-9
    ));
    let string = read(&host, "G_STRING").expect("string");
    match string.value {
        DebugValue::String(value) => {
            assert_eq!(value.capacity, 8);
            assert_eq!(value.units, vec![b'O' as u32, b'K' as u32, b'!' as u32]);
            assert_eq!(value.text.as_deref(), Some("OK!"));
        }
        other => panic!("expected string, got {other:?}"),
    }
    assert_eq!(decimal(&host, "G_WORD"), "65535");
    let results = host.debug_take_queue_results();
    assert_eq!(results.len(), 6);
    assert!(
        results
            .iter()
            .all(|result| result.outcome == DebugQueueOutcome::Applied)
    );

    // A program field with its own persistent root is writable like a global.
    let out_type = read(&host, "MAIN.OUT")
        .expect("program field read")
        .type_key;
    host.debug_enqueue_write(write_request("MAIN.OUT", decimal_typed(&out_type, "40")))
        .expect("program field write");
    host.cycle().expect("cycle");
    assert_eq!(decimal(&host, "MAIN.OUT"), "40");
}

#[test]
fn writes_validate_type_shape_range_enum_subrange_strings_and_writability() {
    let artifacts = common::artifacts(SOURCE, "debug-write-validation");
    let Some(fixture) = fixture("debug-write-validation", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let _generation = cold_install(&mut host, &fixture);

    let dint = root_type_key(&artifacts, "G_DINT");
    let enum_type = root_type_key(&artifacts, "G_ENUM");
    let small = root_type_key(&artifacts, "G_SMALL");
    let word = root_type_key(&artifacts, "G_WORD");
    let string_type = root_type_key(&artifacts, "G_STRING");
    let bool_type = root_type_key(&artifacts, "G_BOOL");

    // Valid enum member and subrange bounds apply.
    host.debug_enqueue_write(write_request("G_ENUM", decimal_typed(&enum_type, "5")))
        .expect("valid enum");
    host.debug_enqueue_write(write_request("G_SMALL", decimal_typed(&small, "10")))
        .expect("subrange upper bound");
    host.cycle().expect("cycle");
    assert_eq!(decimal(&host, "G_ENUM"), "5");
    assert_eq!(decimal(&host, "G_SMALL"), "10");

    // A struct field with its own persistent root is a normal writable target;
    // the non-root parent aggregate path is not.
    host.debug_enqueue_write(write_request("G_POINT.X", decimal_typed(&dint, "77")))
        .expect("field write");
    host.cycle().expect("cycle");
    assert_eq!(decimal(&host, "G_POINT.X"), "77");

    for (path, value, expected) in [
        ("G_ENUM", decimal_typed(&enum_type, "4"), codes::DEBUG_TYPE),
        ("G_SMALL", decimal_typed(&small, "11"), codes::DEBUG_TYPE),
        ("G_SMALL", decimal_typed(&small, "-1"), codes::DEBUG_TYPE),
        (
            "G_DINT",
            decimal_typed(&dint, "2147483648"),
            codes::DEBUG_TYPE,
        ),
        (
            "G_DINT",
            decimal_typed(&dint, "-2147483649"),
            codes::DEBUG_TYPE,
        ),
        ("G_WORD", decimal_typed(&word, "-1"), codes::DEBUG_TYPE),
        ("G_WORD", decimal_typed(&word, "65536"), codes::DEBUG_TYPE),
        ("G_DINT", decimal_typed(&dint, "1.5"), codes::DEBUG_TYPE),
        ("G_BOOL", decimal_typed(&bool_type, "1"), codes::DEBUG_TYPE),
        ("G_DINT", decimal_typed(&enum_type, "5"), codes::DEBUG_TYPE),
        (
            "G_STRING",
            string_typed(&string_type, 8, &[u32::from(b'x'); 9]),
            codes::DEBUG_TYPE,
        ),
        (
            "G_STRING",
            string_typed(&string_type, 8, &[0x100]),
            codes::DEBUG_TYPE,
        ),
        (
            "G_STRING",
            string_typed(&string_type, 7, &[u32::from(b'x')]),
            codes::DEBUG_TYPE,
        ),
        ("G_POINT", decimal_typed(&dint, "1"), codes::DEBUG_UNKNOWN),
        ("G_UNION", decimal_typed(&dint, "1"), codes::DEBUG_TYPE),
        ("G_OBJECT", decimal_typed(&dint, "1"), codes::DEBUG_UNKNOWN),
        ("G_REF", decimal_typed(&dint, "1"), codes::DEBUG_TYPE),
        (
            "NO_SUCH_ROOT",
            decimal_typed(&dint, "1"),
            codes::DEBUG_UNKNOWN,
        ),
    ] {
        let error = match host.debug_enqueue_write(write_request(path, value)) {
            Ok(id) => panic!("write to {path} was accepted as {id:?}"),
            Err(error) => error,
        };
        assert_eq!(error.code, expected, "write to {path}");
    }
    let local = host
        .debug_enqueue_write(write_request("LOCAL", decimal_typed(&dint, "1")))
        .expect_err("unknown local path");
    assert_eq!(local.code, codes::DEBUG_UNKNOWN);
    let local_target =
        DebugWriteRequest::new(DebugTarget::local("STOP-1", "X"), decimal_typed(&dint, "1"));
    assert_eq!(
        host.debug_enqueue_write(local_target)
            .expect_err("local target")
            .code,
        codes::DEBUG_UNKNOWN
    );
    assert_eq!(host.debug_queued_count(), 0);

    // Expected generation/type/layout are checked before enqueue.
    let generation = host.pin_active().expect("active").generation();
    let stale = DebugWriteRequest::new(DebugTarget::path("G_DINT"), decimal_typed(&dint, "1"))
        .with_generation(generation + 1);
    assert_eq!(
        host.debug_enqueue_write(stale)
            .expect_err("stale generation")
            .code,
        codes::DEBUG_STALE
    );
    let stale_type = DebugWriteRequest::new(DebugTarget::path("G_DINT"), decimal_typed(&dint, "1"))
        .with_type_key("INT");
    assert_eq!(
        host.debug_enqueue_write(stale_type)
            .expect_err("stale type")
            .code,
        codes::DEBUG_STALE
    );
    let stale_layout =
        DebugWriteRequest::new(DebugTarget::path("G_DINT"), decimal_typed(&dint, "1"))
            .with_layout_hash(GotId::ZERO);
    assert_eq!(
        host.debug_enqueue_write(stale_layout)
            .expect_err("stale layout")
            .code,
        codes::DEBUG_STALE
    );
}

#[test]
fn forces_apply_every_boundary_win_over_writes_and_release() {
    let artifacts = common::artifacts(SOURCE, "debug-forces");
    let Some(fixture) = fixture("debug-forces", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let _generation = cold_install(&mut host, &fixture);
    let dint = root_type_key(&artifacts, "G_DINT");
    let retained_type = root_type_key(&artifacts, "G_RETAINED");

    let force = host
        .debug_enqueue_force(write_request("G_DINT", decimal_typed(&dint, "9")))
        .expect("force");
    host.cycle().expect("cycle");
    assert_eq!(decimal(&host, "G_DINT"), "9");
    assert_eq!(host.debug_force_count(), 1);
    let results = host.debug_take_queue_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].queue_id, force);
    assert_eq!(results[0].operation, DebugWriteKind::Force);
    assert_eq!(results[0].outcome, DebugQueueOutcome::Applied);

    // A force wins over a write to the same target in the same cycle, in
    // either queue order.
    host.debug_enqueue_force(write_request("G_DINT", decimal_typed(&dint, "10")))
        .expect("replacement force");
    host.debug_enqueue_write(write_request("G_DINT", decimal_typed(&dint, "7")))
        .expect("losing write");
    host.cycle().expect("cycle");
    assert_eq!(decimal(&host, "G_DINT"), "10");
    assert_eq!(host.debug_force_count(), 1);
    let results = host.debug_take_queue_results();
    assert_eq!(results.len(), 2);
    assert!(
        results
            .iter()
            .all(|result| result.outcome == DebugQueueOutcome::Applied)
    );

    // Active force re-applies after the module and after a direct host write.
    put(&mut host, &artifacts, "G_DINT", &3_i32.to_le_bytes());
    host.cycle().expect("cycle");
    assert_eq!(decimal(&host, "G_DINT"), "10");

    assert!(
        host.debug_release_force(DebugTarget::path("G_DINT"))
            .expect("release")
    );
    assert_eq!(host.debug_force_count(), 0);
    assert!(
        !host
            .debug_release_force(DebugTarget::path("G_DINT"))
            .expect("release missing")
    );
    put(&mut host, &artifacts, "G_DINT", &4_i32.to_le_bytes());
    host.cycle().expect("cycle");
    assert_eq!(decimal(&host, "G_DINT"), "4");

    // Release-all covers several active forces.
    host.debug_enqueue_force(write_request("G_DINT", decimal_typed(&dint, "20")))
        .expect("force one");
    host.debug_enqueue_force(write_request(
        "G_RETAINED",
        decimal_typed(&retained_type, "30"),
    ))
    .expect("force two");
    host.cycle().expect("cycle");
    assert_eq!(host.debug_force_count(), 2);
    assert_eq!(host.debug_release_all_forces().expect("release all"), 2);
    assert_eq!(host.debug_force_count(), 0);

    // A force on a RETAIN root reports the documented warning.
    assert_eq!(root(&artifacts, "G_RETAINED").retention, Retention::Retain);
    host.debug_enqueue_force(write_request(
        "G_RETAINED",
        decimal_typed(&retained_type, "40"),
    ))
    .expect("retain force");
    host.cycle().expect("cycle");
    let result = host.debug_take_queue_results().pop().expect("result");
    assert_eq!(result.outcome, DebugQueueOutcome::Applied);
    let warning = result.warning.expect("RETAIN warning");
    assert_eq!(warning.code, codes::W_DEBUG_RETAIN);
    assert!(
        warning.message.contains("G_RETAINED"),
        "{}",
        warning.message
    );
    assert!(!warning.message.is_empty());
}

#[test]
fn queue_limits_are_enforced_at_enqueue() {
    let artifacts = common::artifacts(SOURCE, "debug-write-limit");
    let Some(fixture) = fixture("debug-write-limit", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let _generation = cold_install(&mut host, &fixture);
    let dint = root_type_key(&artifacts, "G_DINT");
    for _ in 0..MAX_DEBUG_QUEUE_ENTRIES {
        host.debug_enqueue_write(write_request("G_DINT", decimal_typed(&dint, "1")))
            .expect("queue below limit");
    }
    assert_eq!(host.debug_queued_count(), MAX_DEBUG_QUEUE_ENTRIES);
    let error = host
        .debug_enqueue_write(write_request("G_DINT", decimal_typed(&dint, "1")))
        .expect_err("queue limit");
    assert_eq!(error.code, codes::DEBUG_LIMIT);
    assert_eq!(host.debug_queued_count(), MAX_DEBUG_QUEUE_ENTRIES);

    // A force shares the same entry budget.
    let error = host
        .debug_enqueue_force(write_request("G_DINT", decimal_typed(&dint, "2")))
        .expect_err("force queue limit");
    assert_eq!(error.code, codes::DEBUG_LIMIT);
}

#[test]
fn direct_input_output_and_bit_writes_follow_boundary_rules() {
    let _guard = common::image_guard();
    let artifacts = common::artifacts(DIRECT_SOURCE, "debug-write-direct");
    let Some(fixture) = fixture("debug-write-direct", artifacts.clone()) else {
        return;
    };
    let platform = platform();
    let info = direct_info(&artifacts, IoBackend::Memory);
    let candidate = Candidate::open_with_info(
        &platform,
        &fixture.module_path,
        &fixture.manifest_path,
        Some(info),
    )
    .expect("candidate");
    let mut host = in_memory_host();
    assert!(matches!(
        host.load(candidate, LoadMode::Cold).expect("cold install"),
        LoadOutcome::ColdInstalled { .. }
    ));
    let input_type = root_type_key(&artifacts, "INPUT");
    let output_type = root_type_key(&artifacts, "OUT");
    let flag_type = root_type_key(&artifacts, "FLAG");
    assert_eq!(root(&artifacts, "INPUT").root_kind, RootKind::Direct);

    // A direct input write is applied after the scan and is overwritten by
    // the next latch when no force holds it.
    host.debug_enqueue_write(write_request("INPUT", decimal_typed(&input_type, "5")))
        .expect("input write");
    host.cycle().expect("cycle");
    assert_eq!(decimal(&host, "INPUT"), "5");
    // The next cycle's backend input is zero; the write is not re-applied.
    assert!(stc_runtime::image_write(0, &0_u16.to_le_bytes()));
    host.cycle().expect("cycle");
    assert_eq!(decimal(&host, "INPUT"), "0");

    // An active input force is substituted at the next scan's latch, so the
    // module sees the forced value even after the backend overwrote it.
    host.debug_enqueue_force(write_request("INPUT", decimal_typed(&input_type, "1234")))
        .expect("input force");
    host.cycle().expect("force activation");
    assert_eq!(decimal(&host, "INPUT"), "1234");
    assert!(stc_runtime::image_write(0, &0_u16.to_le_bytes()));
    host.cycle().expect("forced latch");
    // The latch replaced the image from the backend, then the input force was
    // substituted before the scan.
    assert_eq!(decimal(&host, "INPUT"), "1234");
    assert!(
        host.debug_release_force(DebugTarget::path("INPUT"))
            .expect("release")
    );
    assert!(stc_runtime::image_write(0, &0_u16.to_le_bytes()));
    host.cycle().expect("released latch");
    assert_eq!(decimal(&host, "INPUT"), "0");

    // A direct output write and force are applied before publication.
    host.debug_enqueue_write(write_request("OUT", decimal_typed(&output_type, "40")))
        .expect("output write");
    host.cycle().expect("cycle");
    assert_eq!(decimal(&host, "OUT"), "40");
    host.debug_enqueue_force(write_request("OUT", decimal_typed(&output_type, "7")))
        .expect("output force");
    host.cycle().expect("cycle");
    assert_eq!(decimal(&host, "OUT"), "7");
    assert!(
        host.debug_release_force(DebugTarget::path("OUT"))
            .expect("release")
    );
    assert!(stc_runtime::image_write(3, &42_u16.to_le_bytes()));
    host.cycle().expect("cycle");
    assert_eq!(decimal(&host, "OUT"), "42");

    // A one-bit direct write changes only its bound bit.
    assert!(stc_runtime::image_write(2, &[0x30]));
    host.debug_enqueue_write(write_request("FLAG", bool_typed(&flag_type, true)))
        .expect("bit write");
    host.cycle().expect("cycle");
    let mut byte = [0_u8; 1];
    assert_eq!(stc_runtime::image_read(2, &mut byte), 1);
    assert_eq!(byte[0], 0x38);
    host.debug_enqueue_write(write_request("FLAG", bool_typed(&flag_type, false)))
        .expect("bit write");
    host.cycle().expect("cycle");
    assert_eq!(stc_runtime::image_read(2, &mut byte), 1);
    assert_eq!(byte[0], 0x30);
}

#[test]
fn swap_and_shutdown_clear_queued_writes_and_active_forces() {
    let artifacts = common::artifacts(SOURCE, "debug-write-swap");
    let Some(fixture) = fixture("debug-write-swap", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let old_generation = cold_install(&mut host, &fixture);
    let dint = root_type_key(&artifacts, "G_DINT");
    host.debug_enqueue_write(write_request("G_DINT", decimal_typed(&dint, "1")))
        .expect("queued write");
    host.debug_enqueue_force(write_request("G_DINT", decimal_typed(&dint, "2")))
        .expect("queued force");
    host.cycle().expect("cycle");
    assert_eq!(host.debug_force_count(), 1);
    let _ = host.debug_take_queue_results();

    host.debug_enqueue_write(write_request("G_DINT", decimal_typed(&dint, "3")))
        .expect("write across swap");
    host.debug_enqueue_force(write_request("G_DINT", decimal_typed(&dint, "4")))
        .expect("force across swap");
    let outcome = host
        .load(open(&fixture), LoadMode::Cold)
        .expect("cold install");
    assert!(matches!(outcome, LoadOutcome::ColdInstalled { .. }));
    assert_eq!(host.debug_force_count(), 0);
    assert_eq!(host.debug_queued_count(), 0);
    let results = host.debug_take_queue_results();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|result| {
        result.outcome == DebugQueueOutcome::Rejected
            && result.code == Some(codes::DEBUG_STALE)
            && result.generation == Some(old_generation)
    }));
    assert_eq!(decimal(&host, "G_DINT"), "0");

    host.debug_enqueue_write(write_request("G_DINT", decimal_typed(&dint, "5")))
        .expect("write before shutdown");
    host.debug_enqueue_force(write_request("G_DINT", decimal_typed(&dint, "6")))
        .expect("force before shutdown");
    host.cycle().expect("cycle");
    assert_eq!(host.debug_force_count(), 1);
    let _ = host.debug_take_queue_results();
    host.debug_enqueue_write(write_request("G_DINT", decimal_typed(&dint, "7")))
        .expect("write at shutdown");
    host.shutdown().expect("shutdown");
    assert_eq!(host.debug_force_count(), 0);
    assert_eq!(host.debug_queued_count(), 0);
    let results = host.debug_take_queue_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].outcome, DebugQueueOutcome::Rejected);
    assert_eq!(results[0].code, Some(codes::DEBUG_STALE));
}

#[test]
fn activation_and_rollback_clear_queued_writes_and_active_forces() {
    let v1 = common::artifacts(common::SIMPLE, "debug-write-activate");
    let Some(v1_fixture) = fixture("debug-write-activate-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let old_generation = cold_install(&mut host, &v1_fixture);
    let dint = root_type_key(&v1, "COUNTER");
    host.debug_enqueue_force(write_request("COUNTER", decimal_typed(&dint, "5")))
        .expect("active force");
    host.cycle().expect("force boundary");
    assert_eq!(host.debug_force_count(), 1);
    let _ = host.debug_take_queue_results();
    host.debug_enqueue_write(write_request("COUNTER", decimal_typed(&dint, "6")))
        .expect("write across activation");
    assert_eq!(host.debug_queued_count(), 1);

    let v2 = common::artifacts_with(
        common::SIMPLE_ALTERNATE,
        "debug-write-activate",
        v1.manifest.clone(),
        false,
    );
    let Some(v2_fixture) = fixture("debug-write-activate-v2", v2.clone()) else {
        return;
    };
    let outcome = host
        .load(open(&v2_fixture), LoadMode::Auto)
        .expect("prepare");
    assert!(matches!(outcome, LoadOutcome::Prepared { .. }));
    // Preparing a candidate does not end debug state; the old generation is
    // still active until activation.
    assert_eq!(host.debug_force_count(), 1);
    assert_eq!(host.debug_queued_count(), 1);

    let new_generation = host.activate().expect("activate");
    assert_eq!(host.debug_force_count(), 0);
    assert_eq!(host.debug_queued_count(), 0);
    let results = host.debug_take_queue_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].outcome, DebugQueueOutcome::Rejected);
    assert_eq!(results[0].code, Some(codes::DEBUG_STALE));
    assert_eq!(results[0].generation, Some(old_generation));

    // A queued force created under the new generation is rejected, not
    // migrated, when rollback restores the previous generation.
    let new_type = root_type_key(&v2, "COUNTER");
    host.debug_enqueue_force(write_request("COUNTER", decimal_typed(&new_type, "7")))
        .expect("force across rollback");
    host.rollback().expect("rollback");
    assert_eq!(host.debug_force_count(), 0);
    assert_eq!(host.debug_queued_count(), 0);
    let results = host.debug_take_queue_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].operation, DebugWriteKind::Force);
    assert_eq!(results[0].outcome, DebugQueueOutcome::Rejected);
    assert_eq!(results[0].code, Some(codes::DEBUG_STALE));
    assert_eq!(results[0].generation, Some(new_generation));
}

#[test]
fn audit_records_writes_forces_releases_and_policy_denials_fail_closed() {
    #[derive(Clone)]
    struct Sink {
        events: Arc<Mutex<Vec<AuditEvent>>>,
    }
    impl AuditSink for Sink {
        fn record(&mut self, event: &AuditEvent) -> Result<()> {
            self.events.lock().expect("events").push(event.clone());
            Ok(())
        }
    }
    struct FailingSink;
    impl AuditSink for FailingSink {
        fn record(&mut self, _event: &AuditEvent) -> Result<()> {
            Err(Error::new(codes::DEBUG_AUDIT, "disk full"))
        }
    }

    let artifacts = common::artifacts(SOURCE, "debug-write-audit");
    let Some(fixture) = fixture("debug-write-audit", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &fixture);
    let dint = root_type_key(&artifacts, "G_DINT");
    let events = Arc::new(Mutex::new(Vec::new()));
    host.debug_set_audit_sink(Some(Box::new(Sink {
        events: Arc::clone(&events),
    })));

    let queue_id = host
        .debug_enqueue_write(write_request("G_DINT", decimal_typed(&dint, "8")))
        .expect("audited write");
    host.debug_enqueue_force(write_request("G_DINT", decimal_typed(&dint, "9")))
        .expect("audited force");
    // A replacement force is a new audited force, not a silent update.
    host.debug_enqueue_force(write_request("G_DINT", decimal_typed(&dint, "10")))
        .expect("replacement audited force");
    host.cycle().expect("cycle");
    assert_eq!(decimal(&host, "G_DINT"), "10");
    assert!(
        host.debug_release_force(DebugTarget::path("G_DINT"))
            .expect("audited release")
    );

    let event_log = Arc::clone(&events);
    let events = events_lock(&event_log);
    let names: Vec<_> = events.iter().map(|event| event.event).collect();
    assert_eq!(names, vec!["write", "force", "force", "force-release"]);
    assert_eq!(events[0].seq, 1);
    assert_eq!(events[1].seq, 2);
    assert_eq!(events[2].seq, 3);
    assert_eq!(events[3].seq, 4);
    assert!(events.iter().all(|event| event.format == 1));
    assert!(events.iter().all(|event| event.outcome == AuditOutcome::Ok));
    let target = events[0].target.as_ref().expect("target");
    assert_eq!(target.kind, "root");
    assert!(target.key.ends_with("G_DINT"), "{}", target.key);
    assert_eq!(events[0].session, None);
    assert_eq!(events[0].scope, None);
    assert_eq!(events[0].local_date_ns, 0);
    assert_eq!(events[0].monotonic_ns, 0);
    assert!(events[0].detail.as_object().is_some());
    let json = events[0].to_json();
    assert_eq!(
        json.as_object()
            .and_then(|object| object.get("event"))
            .and_then(stc_got::json::Json::as_str),
        Some("write")
    );
    let _ = queue_id;

    // A policy rejection is audited and the original error is returned.
    let error = host
        .debug_enqueue_write(write_request("G_POINT", decimal_typed(&dint, "1")))
        .expect_err("aggregate rejection");
    assert_eq!(error.code, codes::DEBUG_UNKNOWN);
    let events_after_error = events_lock(&event_log);
    assert_eq!(
        events_after_error.last().map(|event| event.event),
        Some("write")
    );
    assert_eq!(
        events_after_error.last().map(|event| event.outcome),
        Some(AuditOutcome::Error)
    );

    // During the six-action test above the replacement force is audited as a
    // new force, and a failed sink refuses the operation without enqueueing.
    let mut failing = in_memory_host();
    cold_install(&mut failing, &fixture);
    failing.debug_set_audit_sink(Some(Box::new(FailingSink)));
    let error = failing
        .debug_enqueue_write(write_request("G_DINT", decimal_typed(&dint, "11")))
        .expect_err("audit failure");
    assert_eq!(error.code, codes::DEBUG_AUDIT);
    assert_eq!(failing.debug_queued_count(), 0);
    let error = failing
        .debug_enqueue_force(write_request("G_DINT", decimal_typed(&dint, "12")))
        .expect_err("audit failure");
    assert_eq!(error.code, codes::DEBUG_AUDIT);
    assert_eq!(failing.debug_force_count(), 0);
    // With no active forces, release-all has nothing to audit and succeeds.
    assert_eq!(
        failing
            .debug_release_all_forces()
            .expect("empty release-all"),
        0
    );
}

#[test]
fn audit_records_swap_end_and_force_clearing_with_the_ending_generation() {
    #[derive(Clone)]
    struct Sink {
        events: Arc<Mutex<Vec<AuditEvent>>>,
    }
    impl AuditSink for Sink {
        fn record(&mut self, event: &AuditEvent) -> Result<()> {
            self.events.lock().expect("events").push(event.clone());
            Ok(())
        }
    }

    let artifacts = common::artifacts(SOURCE, "debug-write-swap-audit");
    let Some(fixture) = fixture("debug-write-swap-audit", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let old_generation = cold_install(&mut host, &fixture);
    let dint = root_type_key(&artifacts, "G_DINT");
    let events = Arc::new(Mutex::new(Vec::new()));
    host.debug_set_audit_sink(Some(Box::new(Sink {
        events: Arc::clone(&events),
    })));

    host.debug_enqueue_force(write_request("G_DINT", decimal_typed(&dint, "2")))
        .expect("force");
    host.cycle().expect("force boundary");
    host.debug_enqueue_write(write_request("G_DINT", decimal_typed(&dint, "3")))
        .expect("queued write");
    host.debug_enqueue_force(write_request("G_DINT", decimal_typed(&dint, "4")))
        .expect("queued replacement force");
    let _ = host
        .load(open(&fixture), LoadMode::Cold)
        .expect("cold install");

    let event_log = Arc::clone(&events);
    let events = events_lock(&event_log);
    let swap_end = events
        .iter()
        .find(|event| event.event == "swap-end")
        .expect("swap-end event");
    assert_eq!(swap_end.outcome, AuditOutcome::Ok);
    assert_eq!(swap_end.generation, Some(old_generation));
    assert!(swap_end.build.is_some());
    assert!(swap_end.cycle.is_some());
    assert!(events.iter().any(|event| {
        event.event == "force-release"
            && event.generation == Some(old_generation)
            && event
                .detail
                .as_object()
                .and_then(|detail| detail.get("reason"))
                .and_then(stc_got::json::Json::as_str)
                == Some("cold_install")
    }));
    // The queued force was rejected rather than migrated to the new generation.
    assert!(events.iter().any(|event| {
        event.event == "force"
            && event.outcome == AuditOutcome::Error
            && event.generation == Some(old_generation)
    }));

    host.shutdown().expect("shutdown");
    let shutdown_events = events_lock(&event_log);
    assert!(shutdown_events.iter().any(|event| {
        event.event == "session-end"
            && event
                .detail
                .as_object()
                .and_then(|detail| detail.get("reason"))
                .and_then(stc_got::json::Json::as_str)
                == Some("shutdown")
    }));
}

fn events_lock(events: &Arc<Mutex<Vec<AuditEvent>>>) -> Vec<AuditEvent> {
    events.lock().expect("events").clone()
}

#[test]
fn audit_values_over_four_kib_are_truncated() {
    let mut event = AuditEvent::new("write", AuditOutcome::Ok);
    event.value = Some(stc_got::json::Json::object([(
        "value".into(),
        stc_got::json::Json::string("x".repeat(5000)),
    )]));
    let json = event.to_json();
    let value = json
        .as_object()
        .and_then(|object| object.get("value"))
        .expect("value");
    let object = value.as_object().expect("truncation object");
    assert_eq!(
        object.get("truncated"),
        Some(&stc_got::json::Json::Bool(true))
    );
    assert!(object.get("size").is_some());
    let line = event.to_json_line();
    assert!(!line.contains("xxxxx"));
    assert!(line.contains("\"truncated\":true"));
    assert!(line.contains("\"event\":\"write\""));
}

#[test]
fn queue_byte_limit_is_enforced_independently_of_the_entry_limit() {
    let artifacts = common::artifacts(SOURCE, "debug-write-byte-limit");
    let Some(fixture) = fixture("debug-write-byte-limit", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    let _generation = cold_install(&mut host, &fixture);
    let big_type = root_type_key(&artifacts, "G_BIG");
    let element = decimal_typed("DINT", "1");
    let value = TypedValue::new(&big_type, DebugValue::Array(vec![element; 1024]));
    let mut accepted = 0_usize;
    loop {
        let request = write_request("G_BIG", value.clone());
        match host.debug_enqueue_write(request) {
            Ok(_) => accepted += 1,
            Err(error) => {
                assert_eq!(error.code, codes::DEBUG_LIMIT);
                break;
            }
        }
        assert!(
            accepted < MAX_DEBUG_QUEUE_ENTRIES,
            "the byte limit must be reached before the entry limit"
        );
    }
    assert!(accepted > 0);
    assert!(accepted < MAX_DEBUG_QUEUE_ENTRIES);
    assert_eq!(host.debug_queued_count(), accepted);
    let first_big = artifacts
        .manifest
        .roots
        .iter()
        .find(|root| root.key.contains("G_BIG"))
        .expect("G_BIG storage root");
    assert!(
        host.root_storage(first_big.got_id)
            .expect("storage")
            .iter()
            .all(|byte| *byte == 0)
    );
}

#[test]
fn audit_values_use_the_section_15_5_typed_envelope() {
    #[derive(Clone)]
    struct Sink {
        events: Arc<Mutex<Vec<AuditEvent>>>,
    }
    impl AuditSink for Sink {
        fn record(&mut self, event: &AuditEvent) -> Result<()> {
            self.events.lock().expect("events").push(event.clone());
            Ok(())
        }
    }

    let artifacts = common::artifacts(SOURCE, "debug-write-audit-values");
    let Some(fixture) = fixture("debug-write-audit-values", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &fixture);
    let string_type = root_type_key(&artifacts, "G_STRING");
    let array_type = root_type_key(&artifacts, "G_ARRAY");
    let events = Arc::new(Mutex::new(Vec::new()));
    host.debug_set_audit_sink(Some(Box::new(Sink {
        events: Arc::clone(&events),
    })));
    host.debug_enqueue_write(write_request(
        "G_STRING",
        string_typed(&string_type, 8, &[u32::from(b'O'), u32::from(b'K')]),
    ))
    .expect("string write");
    host.debug_enqueue_write(write_request(
        "G_ARRAY",
        TypedValue::new(
            &array_type,
            DebugValue::Array(vec![
                decimal_typed("DINT", "1"),
                decimal_typed("DINT", "2"),
                decimal_typed("DINT", "3"),
            ]),
        ),
    ))
    .expect("array write");
    let real_type = root_type_key(&artifacts, "G_REAL");
    host.debug_enqueue_write(write_request(
        "G_REAL",
        TypedValue::new(&real_type, DebugValue::Real(1.5)),
    ))
    .expect("real write");
    host.cycle().expect("cycle");
    let events = events_lock(&events);
    let value = events[0].value.as_ref().expect("value");
    let value = value.as_object().expect("typed envelope");
    assert_eq!(
        value.get("type").and_then(stc_got::json::Json::as_str),
        Some(string_type.as_str())
    );
    let payload = value
        .get("value")
        .and_then(stc_got::json::Json::as_object)
        .expect("string payload");
    assert_eq!(
        payload
            .get("length")
            .and_then(stc_got::json::Json::as_number),
        Some(2)
    );
    assert!(payload.get("units").is_some());
    let array = events[1].value.as_ref().expect("array value");
    let array = array.as_object().expect("typed envelope");
    let elements = array
        .get("value")
        .and_then(stc_got::json::Json::as_object)
        .and_then(|payload| payload.get("elements"))
        .and_then(stc_got::json::Json::as_array)
        .expect("elements array");
    assert_eq!(elements.len(), 3);
    let first = elements[0].as_object().expect("element envelope");
    assert_eq!(
        first.get("type").and_then(stc_got::json::Json::as_str),
        Some("DINT")
    );
    // A finite REAL is a JSON number, not a string, in audit values.
    let real_line = events[2].to_json_line();
    assert!(real_line.contains("\"value\":1.5"), "{real_line}");
    assert!(!real_line.contains("\"value\":\"1.5\""), "{real_line}");
}

#[test]
fn debug_clear_session_state_is_explicit_and_releases_forces() {
    let artifacts = common::artifacts(SOURCE, "debug-write-clear");
    let Some(fixture) = fixture("debug-write-clear", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &fixture);
    let dint = root_type_key(&artifacts, "G_DINT");
    host.debug_enqueue_force(write_request("G_DINT", decimal_typed(&dint, "1")))
        .expect("force");
    host.cycle().expect("cycle");
    assert_eq!(host.debug_force_count(), 1);
    host.debug_enqueue_write(write_request("G_DINT", decimal_typed(&dint, "2")))
        .expect("write");
    host.debug_clear_session_state(DebugEndReason::SessionEnd)
        .expect("clear");
    assert_eq!(host.debug_force_count(), 0);
    assert_eq!(host.debug_queued_count(), 0);
}
