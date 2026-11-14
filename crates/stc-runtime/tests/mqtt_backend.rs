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
    Disconnected,
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

fn read_frame(stream: &mut TcpStream) -> Vec<u8> {
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
                    let _ = sender.send(BrokerEvent::Disconnected);
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
                    let _ = sender.send(BrokerEvent::Disconnected);
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
        matches!(event, BrokerEvent::Disconnected)
    });
    assert!(matches!(disconnected, BrokerEvent::Disconnected));
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
        matches!(event, BrokerEvent::Disconnected)
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
        matches!(event, BrokerEvent::Disconnected)
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
    fn new(name: &str, contents: &str) -> Self {
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
fn tls_and_credentials_fail_closed_in_phase_three() {
    let tls_file = TempFile::new("ca.pem", "not really a certificate");
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

    let credential_file = TempFile::new("user", "operator");
    let credentials = direct_config(vec![direct_device(
        "broker0",
        format!(
            r#"{{"host":"{LOOPBACK}","port":{},"credentials":{{"username-file":"{}"}},"channels":[]}}"#,
            closed_port(),
            json_path(&credential_file.path)
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
