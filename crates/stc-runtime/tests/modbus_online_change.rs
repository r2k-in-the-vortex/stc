//! Host-level online-change tests for the Modbus TCP server backend.
//!
//! These exercise the real coordinator, a compiled module, and a real loopback
//! Modbus client. The metadata supplies one direct input bound to one Modbus
//! holding-register channel, so the tests can prove that an online activation
//! keeps the listener, its live connections, and its counters, while any
//! configuration change is rejected with `STC-E-ONLINE-IO-001`.

mod common;

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use stc_got::PlanVerdict;
use stc_runtime::module_info::{BindingChannel, ChannelInfo, DeviceInfo};
use stc_runtime::{
    BackendReport, BackendState, Candidate, CycleOutcome, DirectBinding, Host, HostPlatform,
    LoadMode, LoadOutcome, ManifestStore, ModuleInfo, PROFILE,
};

use common::{Behavior, Fixture};

const SOURCE_V1: &str = r#"
VAR_GLOBAL
  Input AT %IW0 : WORD;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

/// A code-only variant of [`SOURCE_V1`]: the same direct variable, image, and
/// I/O configuration with a different program body.
const SOURCE_V2: &str = r#"
VAR_GLOBAL
  Input AT %IW0 : WORD;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 2;
END_PROGRAM
"#;

const BIND: &str = "127.0.0.1:0";
const UNIT_ID: u8 = 7;
const CHANNEL: &str = r#"{"name":"in0","table":"holding-register","address":0}"#;

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

fn direct_root(artifacts: &stc_codegen::module::ModuleArtifacts, suffix: &str) -> stc_got::GotId {
    artifacts
        .manifest
        .roots
        .iter()
        .find(|root| root.root_kind == stc_got::RootKind::Direct && root.key.ends_with(suffix))
        .unwrap_or_else(|| panic!("direct root ending in `{suffix}`"))
        .got_id
}

fn server_config(bind: &str, extra: &str, channel: &str) -> String {
    format!(r#"{{"bind":["{bind}"]{extra},"channels":[{channel}]}}"#)
}

fn base_config() -> String {
    server_config(BIND, "", CHANNEL)
}

fn modbus_info(artifacts: &stc_codegen::module::ModuleArtifacts) -> ModuleInfo {
    ModuleInfo {
        inputs_bytes: 2,
        bindings: vec![DirectBinding {
            got_id: direct_root(artifacts, "INPUT"),
            area: 1,
            offset_bits: 0,
            width_bits: 16,
            device: Some("mb".into()),
            channel: Some(BindingChannel::Name("in0".into())),
            direction: Some("input".into()),
        }],
        devices: vec![DeviceInfo {
            name: "mb".into(),
            kind: "modbus-tcp".into(),
            config: Some(base_config()),
            channels: vec![ChannelInfo {
                name: "in0".into(),
                direction: "input".into(),
                width_bits: 16,
            }],
        }],
        ..ModuleInfo::default()
    }
}

fn image_bytes() -> Vec<u8> {
    let len = usize::try_from(stc_runtime::image_length()).expect("image length");
    let mut bytes = vec![0; len];
    assert_eq!(stc_runtime::image_read(0, &mut bytes), len);
    bytes
}

fn image_u16(offset: usize) -> u16 {
    let mut bytes = [0u8; 2];
    assert_eq!(stc_runtime::image_read(offset, &mut bytes), bytes.len());
    u16::from_le_bytes(bytes)
}

fn modbus_report(host: &Host) -> BackendReport {
    let mut reports: Vec<BackendReport> = host
        .io_reports()
        .into_iter()
        .filter(|report| report.name == "modbus-tcp")
        .collect();
    assert_eq!(reports.len(), 1, "one Modbus backend instance");
    reports.remove(0)
}

fn listener_bind(host: &Host) -> SocketAddr {
    modbus_report(host)
        .diagnostics
        .get("server0.listener0.bind")
        .expect("listener bind diagnostic")
        .parse()
        .expect("listener bind address")
}

fn counter(report: &BackendReport, key: &str) -> u64 {
    report
        .diagnostics
        .get(key)
        .unwrap_or_else(|| panic!("missing diagnostic `{key}`"))
        .parse()
        .unwrap_or_else(|_| panic!("diagnostic `{key}` is not a number"))
}

fn connect(bind: SocketAddr) -> TcpStream {
    let stream =
        TcpStream::connect_timeout(&bind, Duration::from_secs(2)).expect("connect to the server");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .expect("write timeout");
    stream.set_nodelay(true).expect("nodelay");
    stream
}

/// Sends one ADU and returns the complete response ADU.
fn exchange(stream: &mut TcpStream, request: &[u8]) -> Vec<u8> {
    stream.write_all(request).expect("send request");
    let mut header = [0u8; 7];
    stream.read_exact(&mut header).expect("read MBAP header");
    assert_eq!(&header[0..2], &request[0..2], "transaction id");
    assert_eq!(&header[2..4], &[0u8, 0], "protocol id");
    assert_eq!(header[6], UNIT_ID, "unit id");
    let length = usize::from(u16::from_be_bytes([header[4], header[5]]));
    assert!((2..=254).contains(&length), "response length {length}");
    let mut payload = vec![0u8; length - 1];
    stream.read_exact(&mut payload).expect("read PDU");
    let mut response = header.to_vec();
    response.extend_from_slice(&payload);
    response
}

fn write_register(stream: &mut TcpStream, address: u16, value: u16) -> Vec<u8> {
    let [address_hi, address_lo] = address.to_be_bytes();
    let [value_hi, value_lo] = value.to_be_bytes();
    exchange(
        stream,
        &[
            0x12, 0x34, 0, 0, 0, 6, UNIT_ID, 0x06, address_hi, address_lo, value_hi, value_lo,
        ],
    )
}

fn read_register(stream: &mut TcpStream, address: u16) -> u16 {
    let [address_hi, address_lo] = address.to_be_bytes();
    let response = exchange(
        stream,
        &[
            0x56, 0x78, 0, 0, 0, 6, UNIT_ID, 0x03, address_hi, address_lo, 0, 1,
        ],
    );
    assert_eq!(response[7], 0x03, "read holding registers response");
    assert_eq!(response[8], 2, "one register of data");
    u16::from_be_bytes([response[9], response[10]])
}

fn assert_closed(stream: &mut TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut byte = [0u8; 1];
    match stream.read(&mut byte) {
        Ok(0) => {}
        Ok(read) => panic!("expected the connection to close, read {read} bytes"),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::BrokenPipe
            ) => {}
        Err(error) => panic!("unexpected error while waiting for close: {error}"),
    }
}

/// Builds and cold-installs the baseline, returning the fixture, artifacts,
/// host, and the address of its loopback listener.
fn install_baseline() -> Option<(
    Host,
    Fixture,
    stc_codegen::module::ModuleArtifacts,
    SocketAddr,
)> {
    let v1 = common::artifacts(SOURCE_V1, "modbus-online");
    let v1_fixture = Fixture::build("modbus-online-v1", v1.clone(), Behavior::default())?;
    let platform = platform();
    let mut host = in_memory_host();
    let candidate = Candidate::open_with_info(
        &platform,
        &v1_fixture.module_path,
        &v1_fixture.manifest_path,
        Some(modbus_info(&v1)),
    )
    .expect("v1 candidate");
    assert!(matches!(
        host.load(candidate, LoadMode::Cold).expect("cold install"),
        LoadOutcome::ColdInstalled { .. }
    ));
    let bind = listener_bind(&host);
    assert_ne!(bind.port(), 0, "the ephemeral port was resolved");
    Some((host, v1_fixture, v1, bind))
}

#[test]
fn same_configuration_online_change_preserves_modbus_transport_and_counters() {
    let _guard = common::image_guard();
    let Some((mut host, _v1_fixture, v1, bind)) = install_baseline() else {
        return;
    };
    let mut stream = connect(bind);
    write_register(&mut stream, 0, 0x1234);
    assert_eq!(read_register(&mut stream, 0), 0x1234);
    assert!(matches!(
        host.cycle().expect("first cycle"),
        CycleOutcome::Ran { .. }
    ));
    assert_eq!(
        image_u16(0),
        0x1234,
        "the bound Modbus channel latches into the image"
    );

    let before = modbus_report(&host);
    assert_eq!(before.state, BackendState::Running);
    assert_eq!(counter(&before, "server0.listener0.connections"), 1);
    let requests_before = counter(&before, "server0.requests");
    let reads_before = counter(&before, "server0.reads");
    let writes_before = counter(&before, "server0.writes");
    assert!(requests_before >= 2);

    let v2 = common::artifacts_with(SOURCE_V2, "modbus-online", v1.manifest.clone(), false);
    assert_eq!(v2.verdict(), PlanVerdict::OnlineCandidate);
    let Some(v2_fixture) = Fixture::build("modbus-online-v2", v2.clone(), Behavior::default())
    else {
        return;
    };
    let candidate = Candidate::open_with_info(
        &platform(),
        &v2_fixture.module_path,
        &v2_fixture.manifest_path,
        Some(modbus_info(&v2)),
    )
    .expect("v2 candidate");
    let generation = match host
        .load(candidate, LoadMode::Online)
        .expect("a code-only change is prepared")
    {
        LoadOutcome::Prepared { generation } => generation,
        other => panic!("unexpected load outcome: {other:?}"),
    };
    assert_eq!(host.pending_generation(), Some(generation));

    // Preparing does not disturb the listener, its live connection, or its
    // counters (IO-Routing-and-Backends §7).
    let prepared = modbus_report(&host);
    assert_eq!(listener_bind(&host), bind, "prepare keeps the listener");
    assert_eq!(counter(&prepared, "server0.requests"), requests_before);
    assert_eq!(counter(&prepared, "server0.listener0.connections"), 1);
    assert_eq!(
        read_register(&mut stream, 0),
        0x1234,
        "the pre-existing connection survives prepare"
    );

    host.activate().expect("activate");
    assert_eq!(host.active_generation(), Some(generation));
    assert_eq!(
        host.active_build_id(),
        Some(v2.manifest.header.build_id),
        "the new build is active"
    );
    assert!(host.pending_generation().is_none());
    let activated = modbus_report(&host);
    assert_eq!(listener_bind(&host), bind, "activation keeps the listener");
    assert_eq!(
        counter(&activated, "server0.requests"),
        requests_before + 1,
        "the request counter carries over the activation"
    );
    assert_eq!(
        counter(&activated, "server0.listener0.connections"),
        1,
        "the accepted connection carries over the activation"
    );

    // The same TCP stream keeps serving after activation: the connection was
    // never closed or re-accepted.
    write_register(&mut stream, 0, 0xBEEF);
    assert_eq!(read_register(&mut stream, 0), 0xBEEF);
    assert!(matches!(
        host.cycle().expect("commit cycle"),
        CycleOutcome::Committed { .. }
    ));
    assert_eq!(
        image_u16(0),
        0xBEEF,
        "the unchanged channel binding keeps latching after activation"
    );

    let after = modbus_report(&host);
    assert_eq!(counter(&after, "server0.requests"), requests_before + 3);
    assert_eq!(counter(&after, "server0.writes"), writes_before + 1);
    assert_eq!(counter(&after, "server0.reads"), reads_before + 2);
    assert_eq!(counter(&after, "server0.listener0.connections"), 1);
    assert_eq!(after.faults, 0);

    let image_before_shutdown = image_bytes();
    host.shutdown().expect("shutdown");
    assert!(
        host.io_reports().is_empty(),
        "shutdown releases the coordinator"
    );
    assert_closed(&mut stream);
    assert!(
        TcpStream::connect_timeout(&bind, Duration::from_millis(500)).is_err(),
        "shutdown stops the listener"
    );
    assert_eq!(
        image_bytes(),
        image_before_shutdown,
        "shutdown does not rewrite the process image"
    );
    host.shutdown().expect("idempotent shutdown");
}

#[test]
fn changed_modbus_configuration_requires_a_cold_download_and_keeps_serving() {
    let _guard = common::image_guard();
    let Some((mut host, _v1_fixture, _v1, bind)) = install_baseline() else {
        return;
    };
    let mut stream = connect(bind);
    write_register(&mut stream, 0, 0x2222);
    assert_eq!(read_register(&mut stream, 0), 0x2222);
    assert!(matches!(
        host.cycle().expect("first cycle"),
        CycleOutcome::Ran { .. }
    ));

    let active = host.active_generation();
    let build = host.active_build_id();
    let baseline = host.export_manifest().expect("baseline").clone();
    let v2 = common::artifacts_with(SOURCE_V2, "modbus-online", baseline.clone(), false);
    assert_eq!(v2.verdict(), PlanVerdict::OnlineCandidate);
    let Some(v2_fixture) =
        Fixture::build("modbus-cold-required-v2", v2.clone(), Behavior::default())
    else {
        return;
    };
    let info = modbus_info(&v2);

    let mutate = |configure: &dyn Fn(&mut ModuleInfo)| {
        let mut changed = info.clone();
        configure(&mut changed);
        changed
    };
    let mut variants: Vec<(&str, ModuleInfo)> = Vec::new();
    variants.push((
        "bind",
        mutate(&|changed| {
            changed.devices[0].config = Some(server_config("127.0.0.1:65000", "", CHANNEL));
        }),
    ));
    variants.push((
        "unit",
        mutate(&|changed| {
            changed.devices[0].config = Some(server_config(BIND, r#","unit":7"#, CHANNEL));
        }),
    ));
    variants.push((
        "device word order",
        mutate(&|changed| {
            changed.devices[0].config =
                Some(server_config(BIND, r#","word-order":"low-first""#, CHANNEL));
        }),
    ));
    variants.push((
        "table",
        mutate(&|changed| {
            changed.devices[0].config = Some(server_config(
                BIND,
                "",
                r#"{"name":"in0","table":"input-register","address":0}"#,
            ));
        }),
    ));
    variants.push((
        "address",
        mutate(&|changed| {
            changed.devices[0].config = Some(server_config(
                BIND,
                "",
                r#"{"name":"in0","table":"holding-register","address":4}"#,
            ));
        }),
    ));
    variants.push((
        "byte placement and width",
        mutate(&|changed| {
            changed.inputs_bytes = 1;
            changed.bindings[0].width_bits = 8;
            changed.devices[0].channels[0].width_bits = 8;
            changed.devices[0].config = Some(server_config(
                BIND,
                "",
                r#"{"name":"in0","table":"holding-register","address":0,"byte":"high"}"#,
            ));
        }),
    ));
    variants.push((
        "channel direction",
        mutate(&|changed| {
            changed.inputs_bytes = 0;
            changed.outputs_bytes = 2;
            changed.bindings[0].area = 2;
            changed.bindings[0].direction = Some("output".into());
            changed.devices[0].channels[0].direction = "output".into();
        }),
    ));
    variants.push((
        "binding channel",
        mutate(&|changed| {
            changed.bindings[0].channel = Some(BindingChannel::Index(0));
        }),
    ));
    variants.push((
        "binding offset",
        mutate(&|changed| {
            changed.inputs_bytes = 4;
            changed.bindings[0].offset_bits = 16;
        }),
    ));

    let mut requests = counter(&modbus_report(&host), "server0.requests");
    for (label, changed) in variants {
        let candidate = Candidate::open_with_info(
            &platform(),
            &v2_fixture.module_path,
            &v2_fixture.manifest_path,
            Some(changed),
        )
        .expect("changed candidate");
        let error = host
            .load(candidate, LoadMode::Online)
            .expect_err("a changed Modbus configuration needs a cold download");
        assert_eq!(error.code, stc_got::codes::ONLINE_IO, "{label}");
        assert!(
            error.message.contains("cold download"),
            "{label}: {}",
            error.message
        );
        assert_eq!(host.active_generation(), active, "{label}");
        assert_eq!(host.active_build_id(), build, "{label}");
        assert!(host.pending_generation().is_none(), "{label}");
        assert_eq!(host.export_manifest(), Some(&baseline), "{label}");
        assert_eq!(listener_bind(&host), bind, "{label}: the listener stays up");
        assert_eq!(
            read_register(&mut stream, 0),
            0x2222,
            "{label}: the live connection keeps serving"
        );
        let report = modbus_report(&host);
        requests += 1;
        assert_eq!(counter(&report, "server0.requests"), requests, "{label}");
        assert_eq!(report.faults, 0, "{label}");
        assert!(matches!(
            host.cycle().expect("active generation continues"),
            CycleOutcome::Ran { .. }
        ));
    }

    host.shutdown().expect("shutdown");
    assert_closed(&mut stream);
}

#[test]
fn a_failed_cycle_keeps_modbus_serving_until_the_host_stops() {
    let _guard = common::image_guard();
    let v1 = common::artifacts(SOURCE_V1, "modbus-stop");
    let Some(fixture) = Fixture::build(
        "modbus-stop-v1",
        v1.clone(),
        Behavior {
            cycle_failures: 1,
            ..Behavior::default()
        },
    ) else {
        return;
    };
    let platform = platform();
    let mut host = in_memory_host();
    let candidate = Candidate::open_with_info(
        &platform,
        &fixture.module_path,
        &fixture.manifest_path,
        Some(modbus_info(&v1)),
    )
    .expect("v1 candidate");
    host.load(candidate, LoadMode::Cold).expect("cold install");
    let bind = listener_bind(&host);
    let mut stream = connect(bind);
    write_register(&mut stream, 0, 0x4444);

    // A module failure stops the scan but not the server: the failed cycle
    // still runs the input boundary and publishes the last outputs.
    let error = host.cycle().expect_err("the module cycle fails");
    assert!(
        error.message.contains("returned 1"),
        "the failed cycle is reported: {}",
        error.message
    );
    assert_eq!(
        read_register(&mut stream, 0),
        0x4444,
        "the server keeps serving after a failed cycle"
    );
    let report = modbus_report(&host);
    assert_eq!(report.state, BackendState::Running);
    assert_eq!(counter(&report, "server0.listener0.connections"), 1);
    assert_eq!(report.faults, 0);

    // The embedding host decides to stop; shutdown closes the listener and
    // never rewrites the image it latched.
    assert!(matches!(
        host.cycle().expect("the next cycle succeeds"),
        CycleOutcome::Ran { .. }
    ));
    assert_eq!(image_u16(0), 0x4444);
    let image_before_shutdown = image_bytes();
    host.shutdown().expect("stop");
    assert_closed(&mut stream);
    assert!(
        TcpStream::connect_timeout(&bind, Duration::from_millis(500)).is_err(),
        "shutdown stops accepting"
    );
    assert_eq!(
        image_bytes(),
        image_before_shutdown,
        "stopping does not rewrite the image"
    );
}
