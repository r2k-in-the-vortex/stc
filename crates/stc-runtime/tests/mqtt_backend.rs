//! WL-15 phase 3 MQTT backend tests.
//!
//! Every test uses a loopback socket or a closed loopback port; no external
//! broker, DNS name, or test dependency is involved. The scripted broker is
//! test-only and understands just the MQTT 5.0 packets these cases exchange.

use std::io::{ErrorKind as IoErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use stc_got::GotId;
use stc_runtime::io_mock::{MockEthercatBackend, MockWire};
use stc_runtime::io_mqtt::backend::fault;
use stc_runtime::io_mqtt::config::codes;
use stc_runtime::io_mqtt::packet::{self, Packet, Publish, QoS};
use stc_runtime::module_info::{BindingChannel, ChannelInfo, DeviceInfo, DirectBinding};
use stc_runtime::{
    BackendChannel, BackendConfig, BackendDeviceConfig, BackendRegistry, BackendReport,
    BackendState, Coordinator, DeviceBackend, ImageLayout, ImageRange, ModuleInfo, MqttBackend,
    ProcessImage,
};

const READY_DEADLINE: Duration = Duration::from_secs(5);
const LOOPBACK: &str = "127.0.0.1";

fn closed_port() -> u16 {
    let listener = TcpListener::bind((LOOPBACK, 0)).expect("bind a loopback port");
    let port = listener.local_addr().expect("ephemeral address").port();
    drop(listener);
    port
}

fn sleep_briefly() {
    thread::sleep(Duration::from_millis(10));
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(
            start.elapsed() < READY_DEADLINE,
            "timed out waiting for {what}"
        );
        sleep_briefly();
    }
}

fn mqtt_profile(port: u16, channels: &str, stale: &str) -> String {
    format!(
        r#"{{"host":"{LOOPBACK}","port":{port},"client-id":"stc-test","keepalive":"30s",
            "connect-timeout":"500ms","reconnect-backoff":["20ms"],"stale-timeout":"{stale}",
            "channels":[{channels}]}}"#
    )
}

fn bound_channel(
    name: &str,
    direction: &str,
    width_bits: u32,
    area: u8,
    offset: u64,
) -> BackendChannel {
    BackendChannel {
        name: name.to_owned(),
        direction: direction.to_owned(),
        width_bits,
        image: Some(ImageRange {
            area,
            offset_bits: offset,
            width_bits: u64::from(width_bits),
        }),
    }
}

fn direct_device(name: &str, config: String, channels: Vec<BackendChannel>) -> BackendDeviceConfig {
    BackendDeviceConfig {
        name: name.to_owned(),
        kind: "mqtt".to_owned(),
        config: Some(config),
        channels,
    }
}

fn direct_config(devices: Vec<BackendDeviceConfig>) -> BackendConfig {
    BackendConfig {
        name: "mqtt".to_owned(),
        devices,
        bindings: Vec::new(),
        layout: ImageLayout {
            inputs_bytes: 4,
            outputs_bytes: 4,
            markers_bytes: 0,
        },
        profile: None,
        base_dir: PathBuf::from("."),
    }
}

fn mqtt_input_channel() -> &'static str {
    r#"{"name":"in0","direction":"input","topic":"line/input","payload-encoding":"uint","qos":1}"#
}

fn mqtt_output_channel() -> &'static str {
    r#"{"name":"out0","direction":"output","topic":"line/output","payload-encoding":"float","qos":1,"retain":true}"#
}

fn report<'a>(reports: &'a [BackendReport], name: &str) -> &'a BackendReport {
    reports
        .iter()
        .find(|report| report.name == name)
        .unwrap_or_else(|| panic!("missing `{name}` backend report"))
}

fn mqtt_report(coordinator: &Coordinator) -> BackendReport {
    report(&coordinator.reports(), "mqtt").clone()
}

fn wait_mqtt_faulted(coordinator: &mut Coordinator) {
    let start = Instant::now();
    loop {
        coordinator.input_boundary();
        coordinator.after_scan();
        if mqtt_report(coordinator).state == BackendState::Faulted {
            return;
        }
        assert!(
            start.elapsed() < READY_DEADLINE,
            "timed out waiting for the MQTT backend to fault"
        );
        sleep_briefly();
    }
}

fn binding(
    seed: u8,
    area: u8,
    offset_bits: u64,
    width_bits: u64,
    device: &str,
    channel: &str,
    direction: &str,
) -> DirectBinding {
    DirectBinding {
        got_id: GotId::from_bytes([seed; 16]),
        area,
        offset_bits,
        width_bits,
        device: Some(device.to_owned()),
        channel: Some(BindingChannel::Name(channel.to_owned())),
        direction: Some(direction.to_owned()),
    }
}

fn channel(name: &str, direction: &str, width_bits: u32) -> ChannelInfo {
    ChannelInfo {
        name: name.to_owned(),
        direction: direction.to_owned(),
        width_bits,
    }
}

fn module(
    devices: Vec<DeviceInfo>,
    bindings: Vec<DirectBinding>,
    inputs_bytes: u64,
    outputs_bytes: u64,
) -> ModuleInfo {
    ModuleInfo {
        inputs_bytes,
        outputs_bytes,
        input_defaults: vec![0; usize::try_from(inputs_bytes).unwrap_or(0)],
        bindings,
        devices,
        ..ModuleInfo::default()
    }
}

/// One event observed by the scripted broker.
#[derive(Debug)]
enum BrokerEvent {
    Connected,
    Subscribed,
    InputSent,
    Output {
        topic: String,
        payload: Vec<u8>,
        retain: bool,
        qos: QoS,
    },
    /// The raw CONNECT frame, for tests that inspect its fields.
    ConnectFrame(Vec<u8>),
    Disconnected {
        reason: u8,
    },
}

struct Broker {
    address: SocketAddr,
    events: Receiver<BrokerEvent>,
    thread: JoinHandle<()>,
}

impl Broker {
    fn wait_for(&self, what: &str, predicate: impl Fn(&BrokerEvent) -> bool) -> BrokerEvent {
        let start = Instant::now();
        loop {
            let remaining = READY_DEADLINE.saturating_sub(start.elapsed());
            match self.events.recv_timeout(remaining) {
                Ok(event) => {
                    if predicate(&event) {
                        return event;
                    }
                }
                Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for {what}"),
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("broker ended while waiting for {what}")
                }
            }
        }
    }

    fn join(self) {
        self.thread.join().expect("broker thread");
    }
}

fn read_frame(stream: &mut impl Read) -> Vec<u8> {
    let mut frame = Vec::new();
    let mut first = [0_u8; 1];
    stream.read_exact(&mut first).expect("read fixed header");
    frame.push(first[0]);
    let mut remaining = 0_usize;
    let mut multiplier = 1_usize;
    let mut length_bytes = 0;
    loop {
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).expect("read remaining length");
        frame.push(byte[0]);
        remaining += usize::from(byte[0] & 0x7f) * multiplier;
        length_bytes += 1;
        if byte[0] & 0x80 == 0 {
            break;
        }
        assert!(length_bytes < 4, "remaining length uses at most four bytes");
        multiplier *= 128;
    }
    let mut body = vec![0_u8; remaining];
    stream.read_exact(&mut body).expect("read packet body");
    frame.extend_from_slice(&body);
    frame
}

fn header_len(frame: &[u8]) -> usize {
    let mut index = 1;
    while frame[index] & 0x80 != 0 {
        index += 1;
    }
    index + 1
}

fn frame_id(frame: &[u8]) -> u16 {
    let offset = header_len(frame);
    u16::from_be_bytes([frame[offset], frame[offset + 1]])
}

fn connack() -> &'static [u8] {
    &[0x20, 0x03, 0x00, 0x00, 0x00]
}

fn suback(packet_id: u16, count: usize) -> Vec<u8> {
    let [high, low] = packet_id.to_be_bytes();
    let mut frame = vec![0x90, u8::try_from(2 + 1 + count).expect("short SUBACK")];
    frame.extend_from_slice(&[high, low, 0x00]);
    frame.extend(std::iter::repeat_n(0x00, count));
    frame
}

fn puback(packet_id: u16) -> [u8; 4] {
    let [high, low] = packet_id.to_be_bytes();
    [0x40, 0x02, high, low]
}

fn spawn_exchange_broker() -> Broker {
    let listener = TcpListener::bind((LOOPBACK, 0)).expect("bind exchange broker");
    let address = listener.local_addr().expect("exchange broker address");
    let (sender, events) = mpsc::channel();
    let thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept MQTT client");
        stream
            .set_read_timeout(Some(READY_DEADLINE))
            .expect("broker read timeout");
        stream
            .set_write_timeout(Some(READY_DEADLINE))
            .expect("broker write timeout");

        let connect = read_frame(&mut stream);
        assert_eq!(connect[0] >> 4, 1, "first packet is CONNECT");
        let _ = sender.send(BrokerEvent::Connected);
        stream.write_all(connack()).expect("send CONNACK");

        let subscribe = read_frame(&mut stream);
        assert_eq!(subscribe[0] >> 4, 8, "client sends SUBSCRIBE");
        let subscribe_id = frame_id(&subscribe);
        stream
            .write_all(&suback(subscribe_id, 1))
            .expect("send SUBACK");
        let _ = sender.send(BrokerEvent::Subscribed);

        let input = Packet::Publish(Publish {
            topic: "line/input".to_owned(),
            payload: vec![0x12, 0x34],
            qos: QoS::AtLeastOnce,
            retain: false,
            dup: false,
            packet_id: Some(42),
            properties: packet::Properties::default(),
        })
        .encode()
        .expect("encode input PUBLISH");
        stream.write_all(&input).expect("send input PUBLISH");
        let _ = sender.send(BrokerEvent::InputSent);

        let expected = 1.5_f32.to_be_bytes();
        let mut output_seen = false;
        loop {
            let frame = read_frame(&mut stream);
            match frame[0] >> 4 {
                3 => {
                    let (packet, _) =
                        packet::decode_packet(&frame, 0).expect("decode client PUBLISH");
                    let Packet::Publish(publish) = packet else {
                        panic!("expected PUBLISH");
                    };
                    if let Some(packet_id) = publish.packet_id {
                        stream
                            .write_all(&puback(packet_id))
                            .expect("acknowledge client PUBLISH");
                    }
                    if !output_seen && publish.topic == "line/output" && publish.payload == expected
                    {
                        output_seen = true;
                        let _ = sender.send(BrokerEvent::Output {
                            topic: publish.topic.clone(),
                            payload: publish.payload.clone(),
                            retain: publish.retain,
                            qos: publish.qos,
                        });
                    }
                }
                4 => {}
                12 => {
                    stream.write_all(&[0xD0, 0x00]).expect("send PINGRESP");
                }
                14 => {
                    let reason = packet_reason(&frame);
                    let _ = sender.send(BrokerEvent::Disconnected { reason });
                    return;
                }
                other => panic!("unexpected client packet type {other}"),
            }
        }
    });
    Broker {
        address,
        events,
        thread,
    }
}

fn spawn_quiet_broker() -> Broker {
    let listener = TcpListener::bind((LOOPBACK, 0)).expect("bind quiet broker");
    let address = listener.local_addr().expect("quiet broker address");
    let (sender, events) = mpsc::channel();
    let thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept MQTT client");
        stream
            .set_read_timeout(Some(READY_DEADLINE))
            .expect("broker read timeout");
        let connect = read_frame(&mut stream);
        assert_eq!(connect[0] >> 4, 1, "first packet is CONNECT");
        stream.write_all(connack()).expect("send CONNACK");
        let _ = sender.send(BrokerEvent::Connected);
        loop {
            let frame = read_frame(&mut stream);
            match frame[0] >> 4 {
                8 => {
                    let id = frame_id(&frame);
                    stream.write_all(&suback(id, 1)).expect("send SUBACK");
                }
                12 => {
                    stream.write_all(&[0xD0, 0x00]).expect("send PINGRESP");
                }
                14 => {
                    let reason = packet_reason(&frame);
                    let _ = sender.send(BrokerEvent::Disconnected { reason });
                    return;
                }
                _ => {}
            }
        }
    });
    Broker {
        address,
        events,
        thread,
    }
}

/// Accepts but never sends CONNACK, and holds the connection until stopped.
fn spawn_silent_broker() -> (SocketAddr, Arc<AtomicBool>, JoinHandle<()>) {
    let listener = TcpListener::bind((LOOPBACK, 0)).expect("bind silent broker");
    let address = listener.local_addr().expect("silent broker address");
    listener
        .set_nonblocking(true)
        .expect("nonblocking silent listener");
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = thread::spawn(move || {
        let mut stream = None;
        while stream.is_none() {
            if thread_stop.load(Ordering::SeqCst) {
                return;
            }
            match listener.accept() {
                Ok((accepted, _)) => stream = Some(accepted),
                Err(error) if error.kind() == IoErrorKind::WouldBlock => sleep_briefly(),
                Err(_) => return,
            }
        }
        let mut stream = stream.expect("accepted stream");
        let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
        let mut buffer = [0_u8; 256];
        loop {
            if thread_stop.load(Ordering::SeqCst) {
                return;
            }
            match stream.read(&mut buffer) {
                Ok(0) => return,
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        IoErrorKind::WouldBlock | IoErrorKind::TimedOut
                    ) => {}
                Err(_) => return,
            }
        }
    });
    (address, stop, thread)
}

#[test]
fn unreachable_broker_is_nonfatal_and_keeps_the_cached_input() {
    let port = closed_port();
    let mut backend = MqttBackend::new();
    let config = direct_config(vec![direct_device(
        "broker0",
        mqtt_profile(port, mqtt_input_channel(), "50ms"),
        vec![bound_channel("in0", "input", 16, 1, 0)],
    )]);
    backend.cold_start(&config).expect("cold start succeeds");
    let mut image = ProcessImage::new(config.layout);

    let start = Instant::now();
    loop {
        backend.latch_inputs(&mut image).expect("latch");
        backend.publish_outputs(&image).expect("publish");
        if backend.status().state == BackendState::Faulted {
            break;
        }
        assert!(
            start.elapsed() < READY_DEADLINE,
            "timed out waiting for the unreachable broker fault"
        );
        sleep_briefly();
    }
    wait_until("stale input", || {
        backend.latch_inputs(&mut image).expect("latch");
        backend.diagnostics()["input.stale"] == "true"
    });
    assert_eq!(image.read_bits(1, 0, 16), Some(0), "default retained");

    backend.shutdown().expect("shutdown");
    assert_eq!(backend.status().state, BackendState::Stopped);
}

#[test]
fn broker_that_never_answers_leaves_the_run_alive_and_inputs_stale() {
    let (address, stop, broker) = spawn_silent_broker();
    let mut backend = MqttBackend::new();
    let mut profile = mqtt_profile(address.port(), mqtt_input_channel(), "50ms");
    profile = profile.replace("\"500ms\"", "\"150ms\"");
    let config = direct_config(vec![direct_device(
        "broker0",
        profile,
        vec![bound_channel("in0", "input", 16, 1, 0)],
    )]);
    backend
        .cold_start(&config)
        .expect("cold start is immediate");
    let mut image = ProcessImage::new(config.layout);

    let start = Instant::now();
    loop {
        backend.latch_inputs(&mut image).expect("latch");
        backend.publish_outputs(&image).expect("publish");
        if backend.status().state == BackendState::Faulted {
            break;
        }
        assert!(
            start.elapsed() < READY_DEADLINE,
            "timed out waiting for the CONNACK deadline"
        );
        sleep_briefly();
    }
    assert_eq!(image.read_bits(1, 0, 16), Some(0));
    assert_eq!(backend.diagnostics()["input.stale"], "true");

    backend.shutdown().expect("shutdown");
    stop.store(true, Ordering::SeqCst);
    broker.join().expect("broker thread");
}

#[test]
fn loopback_exchange_decodes_inputs_and_publishes_outputs() {
    let broker = spawn_exchange_broker();
    let data = DeviceInfo {
        name: "broker0".to_owned(),
        kind: "mqtt".to_owned(),
        config: Some(mqtt_profile(
            broker.address.port(),
            &format!("{},{}", mqtt_input_channel(), mqtt_output_channel()),
            "2s",
        )),
        channels: vec![channel("in0", "input", 16), channel("out0", "output", 32)],
    };
    let status = DeviceInfo {
        name: "mqtt-status".to_owned(),
        kind: "mqtt-status".to_owned(),
        config: None,
        channels: vec![
            channel("client0.connected", "input", 1),
            channel("client0.state", "input", 16),
            channel("client0.messages_in", "input", 32),
            channel("client0.messages_out", "input", 32),
            channel("input.stale", "input", 1),
            channel("cycle.exchanges", "input", 32),
            channel("cycle.faults", "input", 32),
        ],
    };
    let info = module(
        vec![data, status],
        vec![
            binding(1, 1, 0, 16, "broker0", "in0", "input"),
            binding(2, 2, 0, 32, "broker0", "out0", "output"),
            binding(3, 1, 16, 1, "mqtt-status", "client0.connected", "input"),
            binding(4, 1, 32, 16, "mqtt-status", "client0.state", "input"),
            binding(5, 1, 48, 32, "mqtt-status", "client0.messages_in", "input"),
            binding(6, 1, 80, 32, "mqtt-status", "client0.messages_out", "input"),
            binding(7, 1, 112, 1, "mqtt-status", "input.stale", "input"),
            binding(8, 1, 128, 32, "mqtt-status", "cycle.exchanges", "input"),
            binding(9, 1, 160, 32, "mqtt-status", "cycle.faults", "input"),
        ],
        24,
        4,
    );
    let mut coordinator =
        Coordinator::cold_start(&info, &BackendRegistry::with_defaults()).expect("cold start");

    coordinator
        .image_mut()
        .write_bits(2, 0, 32, u64::from(1.5_f32.to_bits()));

    let start = Instant::now();
    loop {
        coordinator.input_boundary();
        coordinator.after_scan();
        if coordinator.image().read_bits(1, 0, 16) == Some(0x1234) {
            break;
        }
        assert!(
            start.elapsed() < READY_DEADLINE,
            "timed out waiting for the decoded input"
        );
        sleep_briefly();
    }

    let output = broker.wait_for("the encoded output", |event| {
        matches!(event, BrokerEvent::Output { .. })
    });
    let BrokerEvent::Output {
        topic,
        payload,
        retain,
        qos,
    } = output
    else {
        unreachable!();
    };
    assert_eq!(topic, "line/output");
    assert_eq!(payload, 1.5_f32.to_be_bytes().to_vec());
    assert!(retain, "the publish keeps the channel retain flag");
    assert_eq!(qos, QoS::AtLeastOnce);

    wait_until("connected status", || {
        coordinator.input_boundary();
        coordinator.after_scan();
        coordinator.image().read_bits(1, 16, 1) == Some(1)
    });
    assert_eq!(coordinator.image().read_bits(1, 32, 16), Some(2));
    assert!(
        coordinator
            .image()
            .read_bits(1, 48, 32)
            .is_some_and(|value| value >= 1)
    );
    assert!(
        coordinator
            .image()
            .read_bits(1, 80, 32)
            .is_some_and(|value| value >= 1)
    );
    assert!(
        coordinator
            .image()
            .read_bits(1, 128, 32)
            .is_some_and(|value| value >= 1)
    );

    coordinator.shutdown().expect("shutdown");
    let disconnected = broker.wait_for("DISCONNECT", |event| {
        matches!(event, BrokerEvent::Disconnected { .. })
    });
    assert!(
        matches!(disconnected, BrokerEvent::Disconnected { reason: 0 }),
        "shutdown sends DISCONNECT with the normal reason code"
    );
    broker.join();
}

fn write_register(stream: &mut TcpStream, unit: u8, address: u16, value: u16) {
    let [address_high, address_low] = address.to_be_bytes();
    let [value_high, value_low] = value.to_be_bytes();
    stream
        .write_all(&[
            0x12,
            0x34,
            0,
            0,
            0,
            6,
            unit,
            0x06,
            address_high,
            address_low,
            value_high,
            value_low,
        ])
        .expect("write register request");
    let mut response = [0_u8; 12];
    stream
        .read_exact(&mut response)
        .expect("write register response");
    assert_eq!(response[7], 0x06);
}

fn read_register(stream: &mut TcpStream, unit: u8, address: u16) -> u16 {
    let [address_high, address_low] = address.to_be_bytes();
    stream
        .write_all(&[
            0x56,
            0x78,
            0,
            0,
            0,
            6,
            unit,
            0x03,
            address_high,
            address_low,
            0,
            1,
        ])
        .expect("read register request");
    let mut response = [0_u8; 11];
    stream
        .read_exact(&mut response)
        .expect("read register response");
    assert_eq!(response[7], 0x03);
    assert_eq!(response[8], 2);
    u16::from_be_bytes([response[9], response[10]])
}

fn connect_modbus(address: SocketAddr) -> TcpStream {
    let stream = TcpStream::connect_timeout(&address, READY_DEADLINE).expect("Modbus connect");
    stream
        .set_read_timeout(Some(READY_DEADLINE))
        .expect("Modbus read timeout");
    stream
        .set_write_timeout(Some(READY_DEADLINE))
        .expect("Modbus write timeout");
    stream
}

#[test]
fn mixed_modbus_and_faulted_mqtt_keep_their_own_data() {
    let port = closed_port();
    let modbus = DeviceInfo {
        name: "mb".to_owned(),
        kind: "modbus-tcp".to_owned(),
        config: Some(
            r#"{"bind":["127.0.0.1:0"],"channels":[
                {"name":"in0","table":"holding-register","address":0},
                {"name":"out0","table":"holding-register","address":1}
            ]}"#
            .to_owned(),
        ),
        channels: vec![channel("in0", "input", 16), channel("out0", "output", 16)],
    };
    let mqtt = DeviceInfo {
        name: "broker0".to_owned(),
        kind: "mqtt".to_owned(),
        config: Some(mqtt_profile(
            port,
            &format!("{},{}", mqtt_input_channel(), mqtt_output_channel()),
            "50ms",
        )),
        channels: vec![channel("in0", "input", 16), channel("out0", "output", 32)],
    };
    let info = module(
        vec![modbus, mqtt],
        vec![
            binding(1, 1, 0, 16, "mb", "in0", "input"),
            binding(2, 2, 0, 16, "mb", "out0", "output"),
            binding(3, 1, 16, 16, "broker0", "in0", "input"),
            binding(4, 2, 16, 32, "broker0", "out0", "output"),
        ],
        4,
        8,
    );
    let mut coordinator =
        Coordinator::cold_start(&info, &BackendRegistry::with_defaults()).expect("cold start");

    wait_mqtt_faulted(&mut coordinator);
    let modbus_address: SocketAddr = report(&coordinator.reports(), "modbus-tcp")
        .diagnostics
        .get("server0.listener0.bind")
        .expect("Modbus listener address")
        .parse()
        .expect("Modbus listener address");
    let mut client = connect_modbus(modbus_address);

    write_register(&mut client, 7, 0, 0x1234);
    let start = Instant::now();
    loop {
        coordinator.input_boundary();
        coordinator.after_scan();
        if coordinator.image().read_bits(1, 0, 16) == Some(0x1234) {
            break;
        }
        assert!(
            start.elapsed() < READY_DEADLINE,
            "timed out waiting for the Modbus input"
        );
        sleep_briefly();
    }
    assert_eq!(coordinator.image().read_bits(1, 16, 16), Some(0));
    assert_eq!(mqtt_report(&coordinator).state, BackendState::Faulted);

    coordinator.image_mut().write_bits(2, 0, 16, 0x5678);
    coordinator.image_mut().write_bits(2, 16, 16, 0x1111);
    coordinator.after_scan();
    assert_eq!(read_register(&mut client, 7, 1), 0x5678);
    assert_eq!(read_register(&mut client, 7, 0), 0x1234);

    coordinator.shutdown().expect("shutdown");
}

const MOCK_ETHERCAT: &str = r#"{
    "syncs": [
        {
            "index": 1,
            "direction": "input",
            "watchdog": "disable",
            "pdos": [{
                "index": "0x1a00",
                "entries": [{"index": "0x3001", "subindex": 1, "bits": 8}]
            }]
        },
        {
            "index": 2,
            "direction": "output",
            "watchdog": "enable",
            "pdos": [{
                "index": "0x1600",
                "entries": [{"index": "0x3101", "subindex": 1, "bits": 8}]
            }]
        }
    ],
    "pdo-channels": [
        {"name": "IN0", "entry": {"index": "0x3001", "subindex": 1}, "bit": 0},
        {"name": "OUT0", "entry": {"index": "0x3101", "subindex": 1}, "bit": 0}
    ]
}"#;

fn mock_ethercat_registry(wire: &MockWire) -> BackendRegistry {
    let mut registry = BackendRegistry::with_defaults();
    let wire = wire.clone();
    let status_wire = wire.clone();
    registry.register("ethercat", "ethercat", move || {
        Box::new(MockEthercatBackend::with_wire(wire.clone()))
    });
    registry.register("ethercat-status", "ethercat", move || {
        Box::new(MockEthercatBackend::with_wire(status_wire.clone()))
    });
    registry
}

#[test]
fn mixed_mock_ethercat_and_faulted_mqtt_keep_their_own_data() {
    let port = closed_port();
    let ethercat = DeviceInfo {
        name: "ec".to_owned(),
        kind: "ethercat".to_owned(),
        config: Some(MOCK_ETHERCAT.to_owned()),
        channels: vec![channel("IN0", "input", 8), channel("OUT0", "output", 8)],
    };
    let mqtt = DeviceInfo {
        name: "broker0".to_owned(),
        kind: "mqtt".to_owned(),
        config: Some(mqtt_profile(
            port,
            &format!("{},{}", mqtt_input_channel(), mqtt_output_channel()),
            "50ms",
        )),
        channels: vec![channel("in0", "input", 16), channel("out0", "output", 32)],
    };
    let info = module(
        vec![ethercat, mqtt],
        vec![
            binding(1, 1, 0, 8, "ec", "IN0", "input"),
            binding(2, 2, 0, 8, "ec", "OUT0", "output"),
            binding(3, 1, 8, 16, "broker0", "in0", "input"),
            binding(4, 2, 8, 32, "broker0", "out0", "output"),
        ],
        4,
        8,
    );
    let wire = MockWire::new();
    wire.set_input("ec", "IN0", 0xA5);
    let registry = mock_ethercat_registry(&wire);
    let mut coordinator = Coordinator::cold_start(&info, &registry).expect("cold start");

    wait_mqtt_faulted(&mut coordinator);
    assert_eq!(coordinator.image().read_bits(1, 0, 8), Some(0xA5));
    assert_eq!(coordinator.image().read_bits(1, 8, 16), Some(0));

    coordinator.image_mut().write_bits(2, 0, 8, 0x3C);
    coordinator.image_mut().write_bits(2, 8, 16, 0x2222);
    coordinator.after_scan();
    assert_eq!(wire.output("ec", "OUT0"), Some(0x3C));
    assert_eq!(mqtt_report(&coordinator).state, BackendState::Faulted);

    coordinator.shutdown().expect("shutdown");
}

#[test]
fn shutdown_sends_disconnect_is_idempotent_and_drop_joins() {
    let broker = spawn_quiet_broker();
    let config = direct_config(vec![direct_device(
        "broker0",
        mqtt_profile(broker.address.port(), "", "1s"),
        Vec::new(),
    )]);
    let mut backend = MqttBackend::new();
    backend.cold_start(&config).expect("cold start");
    broker.wait_for("connected", |event| matches!(event, BrokerEvent::Connected));
    wait_until("connected diagnostic", || {
        backend.diagnostics()["client0.connected"] == "true"
    });
    backend.shutdown().expect("first shutdown");
    assert_eq!(backend.status().state, BackendState::Stopped);
    backend.shutdown().expect("second shutdown");
    broker.wait_for("DISCONNECT", |event| {
        matches!(event, BrokerEvent::Disconnected { reason: 0 })
    });
    broker.join();

    let broker = spawn_quiet_broker();
    let config = direct_config(vec![direct_device(
        "broker0",
        mqtt_profile(broker.address.port(), "", "1s"),
        Vec::new(),
    )]);
    let mut backend = MqttBackend::new();
    backend.cold_start(&config).expect("cold start");
    broker.wait_for("connected", |event| matches!(event, BrokerEvent::Connected));
    wait_until("connected diagnostic", || {
        backend.diagnostics()["client0.connected"] == "true"
    });
    drop(backend);
    broker.wait_for("DISCONNECT after drop", |event| {
        matches!(event, BrokerEvent::Disconnected { .. })
    });
    broker.join();
}

#[test]
fn invalid_configuration_fails_closed_before_any_connection() {
    let listener = TcpListener::bind((LOOPBACK, 0)).expect("bind unconnected listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let address = listener.local_addr().expect("listener address");
    let valid = direct_device(
        "broker0",
        mqtt_profile(address.port(), mqtt_input_channel(), "1s"),
        vec![bound_channel("in0", "input", 16, 1, 0)],
    );
    let invalid = direct_device(
        "broker1",
        mqtt_profile(address.port(), mqtt_input_channel(), "1s"),
        vec![bound_channel("missing", "input", 16, 1, 16)],
    );
    let config = direct_config(vec![valid, invalid]);
    let mut backend = MqttBackend::new();
    let error = backend
        .cold_start(&config)
        .expect_err("invalid config is fatal");
    assert_eq!(error.code, codes::MQTT_CHANNEL_RESOLVE);
    assert_eq!(backend.status().state, BackendState::Stopped);
    assert_eq!(backend.status().devices, 0);
    match listener.accept() {
        Err(error) if error.kind() == IoErrorKind::WouldBlock => {}
        Ok(_) => panic!("a partial backend connected before validation finished"),
        Err(error) => panic!("unexpected accept error: {error}"),
    }
    backend.shutdown().expect("shutdown after failure");
}

struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn new(name: &str, contents: impl AsRef<[u8]>) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "stc-mqtt-test-{}-{unique}-{name}",
            std::process::id()
        ));
        std::fs::write(&path, contents).expect("write temp file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .expect("private temp file");
        }
        Self { path }
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn json_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[test]
fn invalid_tls_and_credential_material_fails_closed() {
    let tls_file = TempFile::new("invalid-ca.pem", "not really a certificate");
    let tls = direct_config(vec![direct_device(
        "broker0",
        format!(
            r#"{{"host":"{LOOPBACK}","port":{},"tls":{{"ca-file":"{}"}},"channels":[]}}"#,
            closed_port(),
            json_path(&tls_file.path)
        ),
        Vec::new(),
    )]);
    let mut backend = MqttBackend::new();
    let error = backend.cold_start(&tls).expect_err("TLS fails closed");
    assert_eq!(error.code, codes::MQTT_TLS_MATERIAL);
    assert_eq!(backend.status().state, BackendState::Stopped);

    let credentials = direct_config(vec![direct_device(
        "broker0",
        format!(
            r#"{{"host":"{LOOPBACK}","port":{},"credentials":{{"username-file":"missing-user-file"}},"channels":[]}}"#,
            closed_port()
        ),
        Vec::new(),
    )]);
    let error = backend
        .cold_start(&credentials)
        .expect_err("credentials fail closed");
    assert_eq!(error.code, codes::MQTT_CREDENTIAL_MATERIAL);
    assert_eq!(backend.status().state, BackendState::Stopped);
    backend.shutdown().expect("shutdown after failure");
}

/// Returns the reason byte of a client DISCONNECT frame.
fn packet_reason(frame: &[u8]) -> u8 {
    match packet::decode_packet(frame, 0) {
        Ok((Packet::Disconnect(disconnect), _)) => disconnect.reason,
        other => panic!("expected a DISCONNECT frame, got {other:?}"),
    }
}

/// Returns the CONNECT flags byte.
fn connect_flags(frame: &[u8]) -> u8 {
    assert_eq!(frame[0] >> 4, 1, "frame is CONNECT");
    let mut index = 1;
    while frame[index] & 0x80 != 0 {
        index += 1;
    }
    frame[index + 1 + 2 + 4 + 1]
}

/// Reads one committed test-only PEM fixture.
fn fixture_bytes(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mqtt")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("read fixture {name}: {error}"))
}

/// Copies a fixture into a private temporary file, as a real deployment would.
fn private_fixture(name: &str) -> TempFile {
    TempFile::new(name, fixture_bytes(name))
}

fn tls_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn tls_server_config(require_client_cert: bool) -> Arc<rustls::ServerConfig> {
    let certs: Vec<_> = rustls_pemfile::certs(&mut &fixture_bytes("server.pem")[..])
        .collect::<Result<_, _>>()
        .expect("server certificate PEM");
    let key = rustls_pemfile::private_key(&mut &fixture_bytes("server-key.pem")[..])
        .expect("server key PEM")
        .expect("server private key");
    let builder = rustls::ServerConfig::builder_with_provider(tls_provider())
        .with_safe_default_protocol_versions()
        .expect("server protocol versions");
    let builder = if require_client_cert {
        let mut roots = rustls::RootCertStore::empty();
        for certificate in rustls_pemfile::certs(&mut &fixture_bytes("ca.pem")[..]) {
            roots
                .add(certificate.expect("client CA certificate"))
                .expect("client CA root");
        }
        builder.with_client_cert_verifier(
            rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
                .build()
                .expect("client certificate verifier"),
        )
    } else {
        builder.with_no_client_auth()
    };
    Arc::new(
        builder
            .with_single_cert(certs, key)
            .expect("server certificate"),
    )
}

/// A loopback TLS broker that keeps accepting so retries can be observed.
struct TlsBroker {
    address: SocketAddr,
    events: Receiver<BrokerEvent>,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl TlsBroker {
    fn wait_for(&self, what: &str, predicate: impl Fn(&BrokerEvent) -> bool) -> BrokerEvent {
        Broker::wait_for_events(&self.events, what, predicate)
    }

    fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.join().expect("TLS broker thread");
    }
}

fn tls_handshake(
    stream: &mut rustls::StreamOwned<rustls::ServerConnection, TcpStream>,
) -> std::io::Result<()> {
    let deadline = Instant::now() + READY_DEADLINE;
    loop {
        match stream.conn.complete_io(&mut stream.sock) {
            Ok(_) if !stream.conn.is_handshaking() => return Ok(()),
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    IoErrorKind::WouldBlock | IoErrorKind::TimedOut | IoErrorKind::Interrupted
                ) =>
            {
                if Instant::now() >= deadline {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

/// Serves the exchange script over an established stream until DISCONNECT.
fn serve_tls(
    stream: &mut rustls::StreamOwned<rustls::ServerConnection, TcpStream>,
    sender: &mpsc::Sender<BrokerEvent>,
) {
    let connect = read_frame(&mut *stream);
    assert_eq!(connect[0] >> 4, 1, "first packet is CONNECT");
    let _ = sender.send(BrokerEvent::Connected);
    stream.write_all(connack()).expect("send CONNACK");

    let subscribe = read_frame(&mut *stream);
    assert_eq!(subscribe[0] >> 4, 8, "client sends SUBSCRIBE");
    stream
        .write_all(&suback(frame_id(&subscribe), 1))
        .expect("send SUBACK");

    let input = Packet::Publish(Publish {
        topic: "line/input".to_owned(),
        payload: vec![0x12, 0x34],
        qos: QoS::AtLeastOnce,
        retain: false,
        dup: false,
        packet_id: Some(7),
        properties: packet::Properties::default(),
    })
    .encode()
    .expect("encode input PUBLISH");
    stream.write_all(&input).expect("send input PUBLISH");

    loop {
        let frame = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            read_frame(&mut *stream)
        })) {
            Ok(frame) => frame,
            Err(_) => return,
        };
        match frame[0] >> 4 {
            4 => {}
            12 => stream.write_all(&[0xD0, 0x00]).expect("send PINGRESP"),
            14 => {
                let _ = sender.send(BrokerEvent::Disconnected {
                    reason: packet_reason(&frame),
                });
                return;
            }
            _ => {}
        }
    }
}

fn spawn_tls_broker(require_client_cert: bool) -> TlsBroker {
    let listener = TcpListener::bind((LOOPBACK, 0)).expect("bind TLS broker");
    listener
        .set_nonblocking(true)
        .expect("nonblocking TLS listener");
    let address = listener.local_addr().expect("TLS broker address");
    let (sender, events) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let config = tls_server_config(require_client_cert);
    let thread = thread::spawn(move || {
        while !thread_stop.load(Ordering::SeqCst) {
            let socket = match listener.accept() {
                Ok((accepted, _)) => accepted,
                Err(error) if error.kind() == IoErrorKind::WouldBlock => {
                    sleep_briefly();
                    continue;
                }
                Err(_) => return,
            };
            // An accepted socket inherits the listener's non-blocking mode on
            // Windows; the scripted broker blocks on framed reads instead.
            if socket.set_nonblocking(false).is_err() {
                continue;
            }
            if socket
                .set_read_timeout(Some(Duration::from_millis(100)))
                .is_err()
            {
                continue;
            }
            if socket.set_write_timeout(Some(READY_DEADLINE)).is_err() {
                continue;
            }
            let Ok(connection) = rustls::ServerConnection::new(Arc::clone(&config)) else {
                return;
            };
            let mut stream = rustls::StreamOwned::new(connection, socket);
            if tls_handshake(&mut stream).is_err() {
                continue;
            }
            if stream.sock.set_read_timeout(Some(READY_DEADLINE)).is_err() {
                continue;
            }
            serve_tls(&mut stream, &sender);
        }
    });
    TlsBroker {
        address,
        events,
        stop,
        thread,
    }
}

impl Broker {
    fn wait_for_events(
        events: &Receiver<BrokerEvent>,
        what: &str,
        predicate: impl Fn(&BrokerEvent) -> bool,
    ) -> BrokerEvent {
        let start = Instant::now();
        loop {
            let remaining = READY_DEADLINE.saturating_sub(start.elapsed());
            match events.recv_timeout(remaining) {
                Ok(event) => {
                    if predicate(&event) {
                        return event;
                    }
                }
                Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for {what}"),
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("broker ended while waiting for {what}")
                }
            }
        }
    }
}

fn tls_profile(port: u16, ca: &Path, verify: bool, material: Option<(&Path, &Path)>) -> String {
    let client = match material {
        Some((cert, key)) => format!(
            r#","client-cert-file":"{}","client-key-file":"{}""#,
            json_path(cert),
            json_path(key)
        ),
        None => String::new(),
    };
    format!(
        r#"{{"host":"{LOOPBACK}","port":{port},"keepalive":"30s","connect-timeout":"2s",
            "reconnect-backoff":["20ms"],"stale-timeout":"2s",
            "tls":{{"ca-file":"{}","server-name":"localhost","verify":{verify}{client}}},
            "channels":[{}]}}"#,
        json_path(ca),
        mqtt_input_channel()
    )
}

#[test]
fn tls_session_verifies_the_broker_and_reports_verified() {
    let broker = spawn_tls_broker(false);
    let ca = private_fixture("ca.pem");
    let config = direct_config(vec![direct_device(
        "broker0",
        tls_profile(broker.address.port(), &ca.path, true, None),
        vec![bound_channel("in0", "input", 16, 1, 0)],
    )]);
    let mut backend = MqttBackend::new();
    backend.cold_start(&config).expect("cold start");
    assert_eq!(backend.diagnostics()["client0.tls"], "verified");
    assert_eq!(backend.diagnostics()["client0.last_warning"], "");

    let mut image = ProcessImage::new(config.layout);
    wait_until("the decoded TLS input", || {
        backend.latch_inputs(&mut image).expect("latch");
        image.read_bits(1, 0, 16) == Some(0x1234)
    });
    assert_eq!(backend.diagnostics()["client0.connected"], "true");

    backend.shutdown().expect("shutdown");
    assert!(matches!(
        broker.wait_for("DISCONNECT over TLS", |event| matches!(
            event,
            BrokerEvent::Disconnected { .. }
        )),
        BrokerEvent::Disconnected { reason: 0 }
    ));
    broker.stop();
}

#[test]
fn verify_false_accepts_an_untrusted_broker_and_reports_the_warning() {
    let broker = spawn_tls_broker(false);
    let ca = private_fixture("wrong-ca.pem");
    let config = direct_config(vec![direct_device(
        "broker0",
        tls_profile(broker.address.port(), &ca.path, false, None),
        vec![bound_channel("in0", "input", 16, 1, 0)],
    )]);
    let mut backend = MqttBackend::new();
    backend.cold_start(&config).expect("cold start");
    assert_eq!(backend.diagnostics()["client0.tls"], "insecure");
    assert_eq!(
        backend.diagnostics()["client0.last_warning"],
        codes::MQTT_TLS_VERIFICATION_DISABLED
    );

    let mut image = ProcessImage::new(config.layout);
    wait_until("the decoded insecure TLS input", || {
        backend.latch_inputs(&mut image).expect("latch");
        image.read_bits(1, 0, 16) == Some(0x1234)
    });

    backend.shutdown().expect("shutdown");
    broker.stop();
}

#[test]
fn a_wrong_ca_fails_the_handshake_without_a_plaintext_fallback() {
    let broker = spawn_tls_broker(false);
    let ca = private_fixture("wrong-ca.pem");
    let config = direct_config(vec![direct_device(
        "broker0",
        tls_profile(broker.address.port(), &ca.path, true, None),
        vec![bound_channel("in0", "input", 16, 1, 0)],
    )]);
    let mut backend = MqttBackend::new();
    backend.cold_start(&config).expect("cold start");
    wait_until("the TLS handshake fault", || {
        backend.diagnostics()["client0.last_error"] == fault::TLS_HANDSHAKE.to_string()
    });
    assert_eq!(backend.diagnostics()["client0.tls"], "verified");
    assert_eq!(backend.diagnostics()["client0.connected"], "false");
    assert_eq!(backend.status().state, BackendState::Faulted);

    let mut image = ProcessImage::new(config.layout);
    backend.latch_inputs(&mut image).expect("latch");
    assert_eq!(image.read_bits(1, 0, 16), Some(0), "no plaintext fallback");

    backend.shutdown().expect("shutdown");
    broker.stop();
}

#[test]
fn client_certificate_authentication_completes_when_both_files_match() {
    let broker = spawn_tls_broker(true);
    let ca = private_fixture("ca.pem");
    let cert = private_fixture("client.pem");
    let key = private_fixture("client-key.pem");
    let config = direct_config(vec![direct_device(
        "broker0",
        tls_profile(
            broker.address.port(),
            &ca.path,
            true,
            Some((&cert.path, &key.path)),
        ),
        vec![bound_channel("in0", "input", 16, 1, 0)],
    )]);
    let mut backend = MqttBackend::new();
    backend.cold_start(&config).expect("cold start");
    let mut image = ProcessImage::new(config.layout);
    wait_until("the decoded input over mutual TLS", || {
        backend.latch_inputs(&mut image).expect("latch");
        image.read_bits(1, 0, 16) == Some(0x1234)
    });
    backend.shutdown().expect("shutdown");
    broker.stop();
}

#[test]
fn a_broker_that_requires_a_client_certificate_rejects_one_without_it() {
    let broker = spawn_tls_broker(true);
    let ca = private_fixture("ca.pem");
    let config = direct_config(vec![direct_device(
        "broker0",
        tls_profile(broker.address.port(), &ca.path, true, None),
        vec![bound_channel("in0", "input", 16, 1, 0)],
    )]);
    let mut backend = MqttBackend::new();
    backend.cold_start(&config).expect("cold start");
    wait_until("the client-certificate rejection", || {
        backend.status().state == BackendState::Faulted
    });
    assert_eq!(backend.diagnostics()["client0.connected"], "false");
    // TLS 1.3 completes the client side of the handshake before the broker's
    // certificate_required alert arrives, so the rejection surfaces as a
    // handshake fault, an I/O fault, or a CONNECT deadline. What matters is that
    // the session never becomes connected.
    let code = backend.diagnostics()["client0.last_error"].clone();
    assert!(
        [
            fault::TLS_HANDSHAKE.to_string(),
            fault::TRANSPORT_IO.to_string(),
            fault::CONNECT_TIMEOUT.to_string(),
        ]
        .contains(&code),
        "a missing client certificate never connects (last_error {code})"
    );
    backend.shutdown().expect("shutdown");
    broker.stop();
}
fn spawn_connect_capture_broker() -> Broker {
    let listener = TcpListener::bind((LOOPBACK, 0)).expect("bind capture broker");
    let address = listener.local_addr().expect("capture broker address");
    let (sender, events) = mpsc::channel();
    let thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept MQTT client");
        stream
            .set_read_timeout(Some(READY_DEADLINE))
            .expect("broker read timeout");
        let connect = read_frame(&mut stream);
        let _ = sender.send(BrokerEvent::ConnectFrame(connect));
        stream.write_all(connack()).expect("send CONNACK");
        loop {
            let frame = read_frame(&mut stream);
            match frame[0] >> 4 {
                8 => {
                    let id = frame_id(&frame);
                    stream.write_all(&suback(id, 1)).expect("send SUBACK");
                }
                12 => stream.write_all(&[0xD0, 0x00]).expect("send PINGRESP"),
                14 => {
                    let _ = sender.send(BrokerEvent::Disconnected {
                        reason: packet_reason(&frame),
                    });
                    return;
                }
                _ => {}
            }
        }
    });
    Broker {
        address,
        events,
        thread,
    }
}

#[test]
fn connect_carries_file_credentials_without_leaking_them() {
    let broker = spawn_connect_capture_broker();
    let user = TempFile::new("mqtt-user", "operator\n");
    let password = TempFile::new("mqtt-password", b"p@ss\x00word\r\n".as_slice());
    let profile = format!(
        r#"{{"host":"{LOOPBACK}","port":{},"keepalive":"30s","connect-timeout":"2s",
            "reconnect-backoff":["20ms"],"stale-timeout":"2s",
            "credentials":{{"username-file":"{}","password-file":"{}"}},"channels":[]}}"#,
        broker.address.port(),
        json_path(&user.path),
        json_path(&password.path)
    );
    let config = direct_config(vec![direct_device("broker0", profile, Vec::new())]);
    let mut backend = MqttBackend::new();
    backend.cold_start(&config).expect("cold start");

    let BrokerEvent::ConnectFrame(frame) = broker.wait_for("CONNECT", |event| {
        matches!(event, BrokerEvent::ConnectFrame(_))
    }) else {
        unreachable!();
    };
    let flags = connect_flags(&frame);
    assert_eq!(flags & 0x80, 0x80, "user name flag");
    assert_eq!(flags & 0x40, 0x40, "password flag");
    assert!(
        frame.windows(8).any(|window| window == b"operator"),
        "CONNECT carries the user name"
    );
    assert!(
        frame.windows(9).any(|window| window == b"p@ss\x00word"),
        "CONNECT carries the raw password bytes"
    );

    for (key, value) in backend.diagnostics() {
        assert!(
            !value.contains("p@ss") && !value.contains("operator"),
            "diagnostic `{key}` must not leak credentials"
        );
    }
    assert!(
        !backend
            .status()
            .last_fault
            .unwrap_or_default()
            .contains("p@ss"),
        "no fault message may leak credentials"
    );

    backend.shutdown().expect("shutdown");
    broker.join();
}

fn spawn_retained_broker() -> Broker {
    let listener = TcpListener::bind((LOOPBACK, 0)).expect("bind retained broker");
    let address = listener.local_addr().expect("retained broker address");
    let (sender, events) = mpsc::channel();
    let thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept MQTT client");
        stream
            .set_read_timeout(Some(READY_DEADLINE))
            .expect("broker read timeout");
        let connect = read_frame(&mut stream);
        assert_eq!(connect[0] >> 4, 1, "first packet is CONNECT");
        stream.write_all(connack()).expect("send CONNACK");

        let subscribe = read_frame(&mut stream);
        assert_eq!(
            subscribe_retain_handling(&subscribe),
            0,
            "subscribing keeps the broker's retained message available"
        );
        stream
            .write_all(&suback(frame_id(&subscribe), 1))
            .expect("send SUBACK");

        let retained = Packet::Publish(Publish {
            topic: "line/retained".to_owned(),
            payload: vec![0x00, 0x42],
            qos: QoS::AtMostOnce,
            retain: true,
            dup: false,
            packet_id: None,
            properties: packet::Properties::default(),
        })
        .encode()
        .expect("encode retained PUBLISH");
        stream.write_all(&retained).expect("send retained PUBLISH");

        loop {
            let frame = read_frame(&mut stream);
            match frame[0] >> 4 {
                12 => stream.write_all(&[0xD0, 0x00]).expect("send PINGRESP"),
                14 => {
                    let _ = sender.send(BrokerEvent::Disconnected {
                        reason: packet_reason(&frame),
                    });
                    return;
                }
                _ => {}
            }
        }
    });
    Broker {
        address,
        events,
        thread,
    }
}

/// Returns the retain-handling bits of the first SUBSCRIBE filter.
fn subscribe_retain_handling(frame: &[u8]) -> u8 {
    let mut index = header_len(frame) + 2;
    let mut length = 0_usize;
    let mut multiplier = 1_usize;
    loop {
        let byte = frame[index];
        index += 1;
        length += usize::from(byte & 0x7f) * multiplier;
        if byte & 0x80 == 0 {
            break;
        }
        multiplier *= 128;
    }
    index += length;
    let topic_len = usize::from(u16::from_be_bytes([frame[index], frame[index + 1]]));
    frame[index + 2 + topic_len] & 0x30
}

#[test]
fn retained_messages_are_replayed_on_subscribe() {
    let broker = spawn_retained_broker();
    let channel = r#"{"name":"in0","direction":"input","topic":"line/retained","payload-encoding":"uint","qos":0}"#;
    let config = direct_config(vec![direct_device(
        "broker0",
        mqtt_profile(broker.address.port(), channel, "2s"),
        vec![bound_channel("in0", "input", 16, 1, 0)],
    )]);
    let mut backend = MqttBackend::new();
    backend.cold_start(&config).expect("cold start");
    let mut image = ProcessImage::new(config.layout);
    wait_until("the retained input", || {
        backend.latch_inputs(&mut image).expect("latch");
        image.read_bits(1, 0, 16) == Some(0x0042)
    });
    assert_eq!(backend.diagnostics()["client0.messages_in"], "1");
    backend.shutdown().expect("shutdown");
    broker.join();
}

fn spawn_reconnect_broker() -> Broker {
    let listener = TcpListener::bind((LOOPBACK, 0)).expect("bind reconnect broker");
    let address = listener.local_addr().expect("reconnect broker address");
    let (sender, events) = mpsc::channel();
    let thread = thread::spawn(move || {
        for attempt in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept MQTT client");
            stream
                .set_read_timeout(Some(READY_DEADLINE))
                .expect("broker read timeout");
            let connect = read_frame(&mut stream);
            assert_eq!(connect[0] >> 4, 1, "first packet is CONNECT");
            stream.write_all(connack()).expect("send CONNACK");
            let subscribe = read_frame(&mut stream);
            stream
                .write_all(&suback(frame_id(&subscribe), 1))
                .expect("send SUBACK");
            let _ = sender.send(BrokerEvent::Connected);
            if attempt == 0 {
                // Drop the first connection once the session is established;
                // the client must reconnect with the configured backoff.
                stream
                    .set_read_timeout(Some(Duration::from_millis(50)))
                    .expect("short read timeout");
                let mut buffer = [0_u8; 64];
                let _ = stream.read(&mut buffer);
                drop(stream);
                continue;
            }
            let input = Packet::Publish(Publish {
                topic: "line/input".to_owned(),
                payload: vec![0x00, 0x77],
                qos: QoS::AtMostOnce,
                retain: false,
                dup: false,
                packet_id: None,
                properties: packet::Properties::default(),
            })
            .encode()
            .expect("encode input PUBLISH");
            stream.write_all(&input).expect("send input PUBLISH");
            loop {
                let frame = read_frame(&mut stream);
                match frame[0] >> 4 {
                    12 => stream.write_all(&[0xD0, 0x00]).expect("send PINGRESP"),
                    14 => {
                        let _ = sender.send(BrokerEvent::Disconnected {
                            reason: packet_reason(&frame),
                        });
                        return;
                    }
                    _ => {}
                }
            }
        }
    });
    Broker {
        address,
        events,
        thread,
    }
}

#[test]
fn a_dropped_transport_reconnects_with_the_backoff_ladder() {
    let broker = spawn_reconnect_broker();
    let config = direct_config(vec![direct_device(
        "broker0",
        mqtt_profile(broker.address.port(), mqtt_input_channel(), "2s"),
        vec![bound_channel("in0", "input", 16, 1, 0)],
    )]);
    let mut backend = MqttBackend::new();
    backend.cold_start(&config).expect("cold start");
    let mut image = ProcessImage::new(config.layout);
    wait_until("the input after reconnecting", || {
        backend.latch_inputs(&mut image).expect("latch");
        image.read_bits(1, 0, 16) == Some(0x0077)
    });
    let diagnostics = backend.diagnostics();
    assert_eq!(diagnostics["client0.connected"], "true");
    assert!(
        diagnostics["client0.reconnects"]
            .parse::<u64>()
            .is_ok_and(|reconnects| reconnects >= 1),
        "the reconnect counter advanced: {}",
        diagnostics["client0.reconnects"]
    );
    backend.shutdown().expect("shutdown");
    broker.join();
}
