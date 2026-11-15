//! Host-level online-change tests for the MQTT client backend (WL-15 phase 4).
//!
//! These exercise the real coordinator, a compiled module, and a real loopback
//! MQTT broker. The metadata supplies one direct input bound to one MQTT
//! subscription, so the tests can prove that a code-only online activation
//! keeps the client session, its connection, and its counters, while any
//! mapping change is rejected with `STC-E-ONLINE-IO-001` and a cold download
//! restarts the session.

mod common;

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use stc_got::PlanVerdict;
use stc_runtime::io_mqtt::packet::{self, Packet, Publish, QoS};
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

/// A code-only variant of [`SOURCE_V1`].
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

const TOPIC: &str = "line/online";
const READY_DEADLINE: Duration = Duration::from_secs(5);

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

fn mqtt_config(port: u16, topic: &str, encoding: &str, qos: u8) -> String {
    format!(
        r#"{{"host":"127.0.0.1","port":{port},"client-id":"stc-online","keepalive":"30s",
            "connect-timeout":"2s","reconnect-backoff":["20ms"],"stale-timeout":"5s",
            "channels":[{{"name":"in0","direction":"input","topic":"{topic}",
            "payload-encoding":"{encoding}","qos":{qos}}}]}}"#
    )
}

fn mqtt_info(artifacts: &stc_codegen::module::ModuleArtifacts, config: String) -> ModuleInfo {
    ModuleInfo {
        inputs_bytes: 2,
        bindings: vec![DirectBinding {
            got_id: direct_root(artifacts, "INPUT"),
            area: 1,
            offset_bits: 0,
            width_bits: 16,
            device: Some("broker0".into()),
            channel: Some(BindingChannel::Name("in0".into())),
            direction: Some("input".into()),
        }],
        devices: vec![DeviceInfo {
            name: "broker0".into(),
            kind: "mqtt".into(),
            config: Some(config),
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

fn mqtt_report(host: &Host) -> BackendReport {
    let mut reports: Vec<BackendReport> = host
        .io_reports()
        .into_iter()
        .filter(|report| report.name == "mqtt")
        .collect();
    assert_eq!(reports.len(), 1, "one MQTT backend instance");
    reports.remove(0)
}

fn counter(report: &BackendReport, key: &str) -> u64 {
    report
        .diagnostics
        .get(key)
        .unwrap_or_else(|| panic!("missing diagnostic `{key}`"))
        .parse()
        .unwrap_or_else(|_| panic!("diagnostic `{key}` is not a number"))
}

fn read_frame(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut frame = Vec::new();
    let mut first = [0_u8; 1];
    stream.read_exact(&mut first)?;
    frame.push(first[0]);
    let mut remaining = 0_usize;
    let mut multiplier = 1_usize;
    loop {
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte)?;
        frame.push(byte[0]);
        remaining += usize::from(byte[0] & 0x7f) * multiplier;
        if byte[0] & 0x80 == 0 {
            break;
        }
        multiplier *= 128;
    }
    let mut body = vec![0_u8; remaining];
    stream.read_exact(&mut body)?;
    frame.extend_from_slice(&body);
    Ok(frame)
}

fn frame_id(frame: &[u8]) -> u16 {
    let mut index = 1;
    while frame[index] & 0x80 != 0 {
        index += 1;
    }
    index += 1;
    u16::from_be_bytes([frame[index], frame[index + 1]])
}

fn send_frame(stream: &mut TcpStream, frame: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    stream.write_all(frame)
}

/// A loopback broker that keeps the client session busy with retained-free
/// input publishes and counts every accepted connection.
struct OnlineBroker {
    address: SocketAddr,
    connections: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl OnlineBroker {
    fn connections(&self) -> u32 {
        self.connections.load(Ordering::SeqCst)
    }

    fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.join().expect("broker thread");
    }
}

fn spawn_online_broker() -> OnlineBroker {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind online broker");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let address = listener.local_addr().expect("online broker address");
    let connections = Arc::new(AtomicU32::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let thread_connections = Arc::clone(&connections);
    let thread_stop = Arc::clone(&stop);
    let thread = thread::spawn(move || {
        while !thread_stop.load(Ordering::SeqCst) {
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(_) => return,
            };
            if stream.set_nonblocking(false).is_err() {
                continue;
            }
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
            let connect = match read_frame(&mut stream) {
                Ok(frame) => frame,
                Err(_) => continue,
            };
            assert_eq!(connect[0] >> 4, 1, "first packet is CONNECT");
            thread_connections.fetch_add(1, Ordering::SeqCst);
            if send_frame(&mut stream, &[0x20, 0x03, 0x00, 0x00, 0x00]).is_err() {
                continue;
            }
            let subscribe = match read_frame(&mut stream) {
                Ok(frame) => frame,
                Err(_) => continue,
            };
            let id = frame_id(&subscribe);
            let [high, low] = id.to_be_bytes();
            if send_frame(&mut stream, &[0x90, 0x04, high, low, 0x00, 0x00]).is_err() {
                continue;
            }
            let mut value: u16 = 0x0100;
            while !thread_stop.load(Ordering::SeqCst) {
                let publish = Packet::Publish(Publish {
                    topic: TOPIC.to_owned(),
                    payload: value.to_be_bytes().to_vec(),
                    qos: QoS::AtMostOnce,
                    retain: false,
                    dup: false,
                    packet_id: None,
                    properties: packet::Properties::default(),
                })
                .encode()
                .expect("encode input PUBLISH");
                if send_frame(&mut stream, &publish).is_err() {
                    break;
                }
                value = value.wrapping_add(1);
                thread::sleep(Duration::from_millis(20));
            }
        }
    });
    OnlineBroker {
        address,
        connections,
        stop,
        thread,
    }
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(
            start.elapsed() < READY_DEADLINE,
            "timed out waiting for {what}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn same_configuration_online_change_keeps_the_mqtt_session_and_counters() {
    let _guard = common::image_guard();
    let broker = spawn_online_broker();
    let v1 = common::artifacts(SOURCE_V1, "mqtt-online");
    let Some(v1_fixture) = Fixture::build("mqtt-online-v1", v1.clone(), Behavior::default()) else {
        return;
    };
    let config = mqtt_config(broker.address.port(), TOPIC, "uint", 1);
    let mut host = in_memory_host();
    let candidate = Candidate::open_with_info(
        &platform(),
        &v1_fixture.module_path,
        &v1_fixture.manifest_path,
        Some(mqtt_info(&v1, config.clone())),
    )
    .expect("v1 candidate");
    assert!(matches!(
        host.load(candidate, LoadMode::Cold).expect("cold install"),
        LoadOutcome::ColdInstalled { .. }
    ));

    wait_until("the first input", || {
        let _ = host.cycle();
        image_u16(0) != 0
    });
    let before = mqtt_report(&host);
    assert_eq!(before.state, BackendState::Running);
    assert_eq!(before.diagnostics["client0.connected"], "true");
    assert_eq!(broker.connections(), 1, "one broker connection");
    let messages_before = counter(&before, "client0.messages_in");
    assert!(messages_before >= 1);
    let faults_before = before.faults;

    let v2 = common::artifacts_with(SOURCE_V2, "mqtt-online", v1.manifest.clone(), false);
    assert_eq!(v2.verdict(), PlanVerdict::OnlineCandidate);
    let Some(v2_fixture) = Fixture::build("mqtt-online-v2", v2.clone(), Behavior::default()) else {
        return;
    };
    let candidate = Candidate::open_with_info(
        &platform(),
        &v2_fixture.module_path,
        &v2_fixture.manifest_path,
        Some(mqtt_info(&v2, config)),
    )
    .expect("v2 candidate");
    let generation = match host
        .load(candidate, LoadMode::Online)
        .expect("a code-only change is prepared")
    {
        LoadOutcome::Prepared { generation } => generation,
        other => panic!("unexpected load outcome: {other:?}"),
    };

    // Preparing does not disturb the session: the broker keeps the connection
    // and the counters carry over (IO-Routing-and-Backends §7).
    let prepared = mqtt_report(&host);
    assert_eq!(broker.connections(), 1, "prepare keeps the session");
    assert!(counter(&prepared, "client0.messages_in") >= messages_before);

    let ticks_before = image_u16(0);
    host.activate().expect("activate");
    assert_eq!(host.active_generation(), Some(generation));
    assert!(host.pending_generation().is_none());
    assert_eq!(broker.connections(), 1, "activation keeps the session");

    wait_until("the next input after activation", || {
        let _ = host.cycle();
        image_u16(0) != ticks_before
    });
    let after = mqtt_report(&host);
    assert_eq!(after.diagnostics["client0.connected"], "true");
    assert_eq!(broker.connections(), 1, "the client never reconnected");
    assert!(
        counter(&after, "client0.messages_in") >= messages_before,
        "the input counter carries over the activation"
    );
    assert_eq!(after.faults, faults_before);

    let image_before_shutdown = image_bytes();
    host.shutdown().expect("shutdown");
    assert!(
        host.io_reports().is_empty(),
        "shutdown releases the coordinator"
    );
    assert_eq!(
        image_bytes(),
        image_before_shutdown,
        "shutdown does not rewrite the process image"
    );
    host.shutdown().expect("idempotent shutdown");
    broker.stop();
}

#[test]
fn changed_mqtt_mapping_requires_a_cold_download_and_keeps_the_session() {
    let _guard = common::image_guard();
    let broker = spawn_online_broker();
    let v1 = common::artifacts(SOURCE_V1, "mqtt-online");
    let Some(v1_fixture) = Fixture::build("mqtt-online-v1", v1.clone(), Behavior::default()) else {
        return;
    };
    let config = mqtt_config(broker.address.port(), TOPIC, "uint", 1);
    let mut host = in_memory_host();
    let candidate = Candidate::open_with_info(
        &platform(),
        &v1_fixture.module_path,
        &v1_fixture.manifest_path,
        Some(mqtt_info(&v1, config.clone())),
    )
    .expect("v1 candidate");
    host.load(candidate, LoadMode::Cold).expect("cold install");
    wait_until("the first input", || {
        let _ = host.cycle();
        image_u16(0) != 0
    });

    let active = host.active_generation();
    let build = host.active_build_id();
    let baseline = host.export_manifest().expect("baseline").clone();
    let v2 = common::artifacts_with(SOURCE_V2, "mqtt-online", baseline.clone(), false);
    assert_eq!(v2.verdict(), PlanVerdict::OnlineCandidate);
    let Some(v2_fixture) = Fixture::build("mqtt-cold-required-v2", v2.clone(), Behavior::default())
    else {
        return;
    };
    let info = mqtt_info(&v2, config);
    let port = broker.address.port();

    let mutate = |configure: &dyn Fn(&mut ModuleInfo)| {
        let mut changed = info.clone();
        configure(&mut changed);
        changed
    };
    let variants: Vec<(&str, ModuleInfo)> = vec![
        (
            "topic",
            mutate(&|changed| {
                changed.devices[0].config = Some(mqtt_config(port, "line/changed", "uint", 1));
            }),
        ),
        (
            "encoding",
            mutate(&|changed| {
                changed.devices[0].config = Some(mqtt_config(port, TOPIC, "int", 1));
            }),
        ),
        (
            "qos",
            mutate(&|changed| {
                changed.devices[0].config = Some(mqtt_config(port, TOPIC, "uint", 0));
            }),
        ),
        (
            "client id",
            mutate(&|changed| {
                changed.devices[0].config =
                    Some(mqtt_config(port, TOPIC, "uint", 1).replace("stc-online", "stc-other"));
            }),
        ),
        (
            "stale timeout",
            mutate(&|changed| {
                changed.devices[0].config =
                    Some(mqtt_config(port, TOPIC, "uint", 1).replace("\"5s\"", "\"2s\""));
            }),
        ),
        (
            "binding offset",
            mutate(&|changed| {
                changed.inputs_bytes = 4;
                changed.bindings[0].offset_bits = 16;
            }),
        ),
        (
            "binding channel",
            mutate(&|changed| {
                changed.bindings[0].channel = Some(BindingChannel::Index(0));
            }),
        ),
    ];

    let mut connections = broker.connections();
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
            .expect_err("a changed MQTT mapping needs a cold download");
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
        assert_eq!(
            broker.connections(),
            connections,
            "{label}: the session stays up"
        );
        let report = mqtt_report(&host);
        assert_eq!(report.state, BackendState::Running, "{label}");
        assert_eq!(report.diagnostics["client0.connected"], "true", "{label}");
        assert!(
            matches!(
                host.cycle().expect("the active generation keeps running"),
                CycleOutcome::Ran { .. } | CycleOutcome::Committed { .. }
            ),
            "{label}"
        );
        assert_ne!(image_u16(0), 0, "{label}: the live session keeps latching");
        connections = broker.connections();
    }

    host.shutdown().expect("shutdown");
    broker.stop();
}

#[test]
fn a_cold_download_restarts_the_mqtt_session() {
    let _guard = common::image_guard();
    let broker = spawn_online_broker();
    let v1 = common::artifacts(SOURCE_V1, "mqtt-cold");
    let Some(v1_fixture) = Fixture::build("mqtt-cold-v1", v1.clone(), Behavior::default()) else {
        return;
    };
    let config = mqtt_config(broker.address.port(), TOPIC, "uint", 1);
    let mut host = in_memory_host();
    let candidate = Candidate::open_with_info(
        &platform(),
        &v1_fixture.module_path,
        &v1_fixture.manifest_path,
        Some(mqtt_info(&v1, config.clone())),
    )
    .expect("v1 candidate");
    host.load(candidate, LoadMode::Cold).expect("cold install");
    wait_until("the first input", || {
        let _ = host.cycle();
        image_u16(0) != 0
    });
    wait_until("enough inputs to see a counter reset", || {
        let _ = host.cycle();
        counter(&mqtt_report(&host), "client0.messages_in") >= 20
    });
    let messages_before = counter(&mqtt_report(&host), "client0.messages_in");
    assert_eq!(broker.connections(), 1);

    let v2 = common::artifacts_with(SOURCE_V2, "mqtt-cold", v1.manifest.clone(), false);
    let Some(v2_fixture) = Fixture::build("mqtt-cold-v2", v2.clone(), Behavior::default()) else {
        return;
    };
    let candidate = Candidate::open_with_info(
        &platform(),
        &v2_fixture.module_path,
        &v2_fixture.manifest_path,
        Some(mqtt_info(&v2, config)),
    )
    .expect("v2 candidate");
    host.load(candidate, LoadMode::Cold)
        .expect("a cold download is accepted");
    assert!(
        counter(&mqtt_report(&host), "client0.messages_in") < messages_before,
        "a cold download starts a fresh session with fresh counters"
    );

    wait_until("the restarted session", || broker.connections() == 2);
    wait_until("the first input of the new session", || {
        let _ = host.cycle();
        let report = mqtt_report(&host);
        counter(&report, "client0.messages_in") >= 1
    });
    let restarted = mqtt_report(&host);
    assert_eq!(restarted.diagnostics["client0.connected"], "true");
    assert_eq!(restarted.faults, 0);

    host.shutdown().expect("shutdown");
    broker.stop();
}
