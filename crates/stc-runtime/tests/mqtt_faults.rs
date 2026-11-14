//! WL-15 phase-5 MQTT fault-injection tests.
//!
//! These use the scripted loopback broker in `tests/mqtt_sim` to inject a
//! malformed frame, an oversized frame, an undecodable payload, a duplicate
//! QoS 2 delivery, and a slow CONNACK. Every case is a per-device fault: the
//! scan keeps running, the last accepted input stays cached, and the session
//! recovers with the configured backoff. The same suite runs in the existing
//! CI matrix on every supported host, so it is the cross-host end-to-end
//! fixture for the backend.

mod mqtt_sim;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mqtt_sim::{Action, Event, Script, SimBroker};
use stc_runtime::io_mqtt::backend::fault;
use stc_runtime::io_mqtt::packet::QoS;
use stc_runtime::{
    BackendChannel, BackendConfig, BackendDeviceConfig, BackendState, DeviceBackend, ImageLayout,
    ImageRange, MqttBackend, ProcessImage,
};

const READY_DEADLINE: Duration = Duration::from_secs(5);
const INPUT_TOPIC: &str = "line/input";
const OUTPUT_TOPIC: &str = "line/output";

fn sleep_briefly() {
    std::thread::sleep(Duration::from_millis(10));
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

fn profile(port: u16, channels: &str, backoff: &str) -> String {
    format!(
        r#"{{"host":"127.0.0.1","port":{port},"client-id":"stc-sim","keepalive":"30s",
            "connect-timeout":"1s","reconnect-backoff":["{backoff}"],"stale-timeout":"2s",
            "channels":[{channels}]}}"#
    )
}

fn channel(
    name: &str,
    direction: &str,
    width_bits: u32,
    area: u8,
    offset_bits: u64,
) -> BackendChannel {
    BackendChannel {
        name: name.to_owned(),
        direction: direction.to_owned(),
        width_bits,
        image: Some(ImageRange {
            area,
            offset_bits,
            width_bits: u64::from(width_bits),
        }),
    }
}

fn config(port: u16, channels: &str, declared: Vec<BackendChannel>) -> BackendConfig {
    BackendConfig {
        name: "mqtt".to_owned(),
        devices: vec![BackendDeviceConfig {
            name: "broker0".to_owned(),
            kind: "mqtt".to_owned(),
            config: Some(profile(port, channels, "300ms")),
            channels: declared,
        }],
        bindings: Vec::new(),
        layout: ImageLayout {
            inputs_bytes: 8,
            outputs_bytes: 8,
            markers_bytes: 0,
        },
        profile: None,
        base_dir: PathBuf::from("."),
    }
}

fn input_channel() -> &'static str {
    r#"{"name":"in0","direction":"input","topic":"line/input","payload-encoding":"uint","qos":0}"#
}

fn input_plus_output_channels() -> String {
    format!(
        r#"{{"name":"in0","direction":"input","topic":"{INPUT_TOPIC}","payload-encoding":"uint","qos":1}},
           {{"name":"out0","direction":"output","topic":"{OUTPUT_TOPIC}","payload-encoding":"float","qos":1,"retain":true}}"#
    )
}

fn start(config: &BackendConfig) -> (MqttBackend, ProcessImage) {
    let mut backend = MqttBackend::new();
    backend.cold_start(config).expect("cold start");
    let image = ProcessImage::new(config.layout);
    (backend, image)
}

#[test]
fn a_malformed_frame_is_a_protocol_fault_and_the_session_recovers() {
    let broker = SimBroker::spawn(Script {
        after_subscribe: vec![Action::Malformed],
        publish_every: Some((INPUT_TOPIC.to_owned(), Duration::from_millis(10))),
        ..Script::quiet()
    });
    let config = config(
        broker.port(),
        input_channel(),
        vec![channel("in0", "input", 16, 1, 0)],
    );
    let (mut backend, mut image) = start(&config);

    wait_until("the protocol fault", || {
        backend.diagnostics()["client0.last_error"] == fault::PROTOCOL.to_string()
    });
    wait_until("the recovered session", || {
        backend.latch_inputs(&mut image).expect("latch");
        backend.diagnostics()["client0.connected"] == "true"
            && backend.diagnostics()["client0.messages_in"]
                .parse::<u64>()
                .is_ok_and(|messages| messages >= 1)
    });
    let diagnostics = backend.diagnostics();
    assert!(
        diagnostics["client0.reconnects"]
            .parse::<u64>()
            .is_ok_and(|reconnects| reconnects >= 1),
        "the client reconnected: {}",
        diagnostics["client0.reconnects"]
    );
    assert_eq!(backend.status().state, BackendState::Running);

    backend.shutdown().expect("shutdown");
    broker.stop();
}

#[test]
fn an_oversized_publish_is_rejected_and_the_session_recovers() {
    let broker = SimBroker::spawn(Script {
        after_subscribe: vec![Action::Oversized],
        publish_every: Some((INPUT_TOPIC.to_owned(), Duration::from_millis(10))),
        ..Script::quiet()
    });
    let config = config(
        broker.port(),
        input_channel(),
        vec![channel("in0", "input", 16, 1, 0)],
    );
    let (mut backend, mut image) = start(&config);

    wait_until("the oversize fault", || {
        backend.diagnostics()["client0.last_error"] == fault::OVERSIZE.to_string()
    });
    wait_until("the recovered session", || {
        backend.latch_inputs(&mut image).expect("latch");
        backend.diagnostics()["client0.connected"] == "true"
    });

    backend.shutdown().expect("shutdown");
    broker.stop();
}

#[test]
fn an_undecodable_payload_is_dropped_and_the_cached_value_kept() {
    let broker = SimBroker::spawn(Script {
        after_subscribe: vec![
            Action::publish(INPUT_TOPIC, vec![0x12, 0x34]),
            Action::publish(INPUT_TOPIC, vec![0x12, 0x34, 0x56]),
        ],
        ..Script::quiet()
    });
    let config = config(
        broker.port(),
        input_channel(),
        vec![channel("in0", "input", 16, 1, 0)],
    );
    let (mut backend, mut image) = start(&config);

    wait_until("the accepted and rejected payloads", || {
        backend.latch_inputs(&mut image).expect("latch");
        backend.diagnostics()["client0.dropped"] == "1"
    });
    assert_eq!(
        image.read_bits(1, 0, 16),
        Some(0x1234),
        "the last decodable value stays cached"
    );
    assert_eq!(
        backend.diagnostics()["client0.last_error"],
        fault::PAYLOAD_DECODE.to_string()
    );
    assert_eq!(backend.diagnostics()["client0.messages_in"], "1");
    assert_eq!(backend.diagnostics()["client0.connected"], "true");

    backend.shutdown().expect("shutdown");
    broker.stop();
}

#[test]
fn a_duplicate_qos2_delivery_is_applied_once() {
    let broker = SimBroker::spawn(Script {
        after_subscribe: vec![Action::Publish {
            topic: INPUT_TOPIC.to_owned(),
            payload: vec![0x00, 0x42],
            qos: QoS::ExactlyOnce,
            retain: false,
            duplicate: true,
        }],
        ..Script::quiet()
    });
    let config = config(
        broker.port(),
        input_channel(),
        vec![channel("in0", "input", 16, 1, 0)],
    );
    let (mut backend, mut image) = start(&config);

    wait_until("the first delivery", || {
        backend.latch_inputs(&mut image).expect("latch");
        backend.diagnostics()["client0.messages_in"] == "1"
    });
    sleep_briefly();
    backend.latch_inputs(&mut image).expect("latch");
    assert_eq!(
        backend.diagnostics()["client0.messages_in"],
        "1",
        "the duplicate QoS 2 delivery is not applied twice"
    );
    assert_eq!(image.read_bits(1, 0, 16), Some(0x0042));

    backend.shutdown().expect("shutdown");
    broker.stop();
}

#[test]
fn a_slow_connack_does_not_stall_the_cycle_boundary() {
    let broker = SimBroker::spawn(Script {
        connack_delay: Some(Duration::from_millis(600)),
        ..Script::quiet()
    });
    let config = config(
        broker.port(),
        input_channel(),
        vec![channel("in0", "input", 16, 1, 0)],
    );
    let (mut backend, mut image) = start(&config);

    let started = Instant::now();
    for _ in 0..200 {
        backend.latch_inputs(&mut image).expect("latch");
        backend.publish_outputs(&image).expect("publish");
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(250),
        "200 cycle boundaries took {elapsed:?} while the broker delayed CONNACK"
    );
    assert_eq!(
        backend.diagnostics()["client0.connected"],
        "false",
        "the delay is observed as a pending connection, not a stall"
    );
    assert_eq!(backend.diagnostics()["input.stale"], "true");

    backend.shutdown().expect("shutdown");
    broker.stop();
}

#[test]
fn the_simulator_captures_connect_subscribe_and_published_outputs() {
    let broker = SimBroker::spawn(Script::quiet());
    let config = BackendConfig {
        name: "mqtt".to_owned(),
        devices: vec![BackendDeviceConfig {
            name: "broker0".to_owned(),
            kind: "mqtt".to_owned(),
            config: Some(format!(
                r#"{{"host":"127.0.0.1","port":{},"client-id":"stc-sim","keepalive":"20s",
                    "clean-start":true,"session-expiry":"60s",
                    "connect-timeout":"1s","reconnect-backoff":["100ms"],"stale-timeout":"2s",
                    "last-will":{{"topic":"line/will","payload":"offline","qos":1,"retain":true}},
                    "channels":[{}]}}"#,
                broker.port(),
                input_plus_output_channels()
            )),
            channels: vec![
                channel("in0", "input", 16, 1, 0),
                channel("out0", "output", 32, 2, 0),
            ],
        }],
        bindings: Vec::new(),
        layout: ImageLayout {
            inputs_bytes: 4,
            outputs_bytes: 4,
            markers_bytes: 0,
        },
        profile: None,
        base_dir: PathBuf::from("."),
    };
    let (mut backend, mut image) = start(&config);

    let Event::Connect {
        client_id,
        clean_start,
        keep_alive,
        session_expiry,
        will,
        ..
    } = broker.wait_for("CONNECT", |event| matches!(event, Event::Connect { .. }))
    else {
        unreachable!();
    };
    assert_eq!(client_id, "stc-sim");
    assert!(clean_start);
    assert_eq!(keep_alive, 20);
    assert_eq!(session_expiry, 60);
    assert_eq!(will, Some(("line/will".to_owned(), b"offline".to_vec())));

    let Event::Subscribe {
        filters,
        retain_handling,
    } = broker.wait_for("SUBSCRIBE", |event| {
        matches!(event, Event::Subscribe { .. })
    })
    else {
        unreachable!();
    };
    assert_eq!(filters, vec![(INPUT_TOPIC.to_owned(), 1)]);
    assert_eq!(retain_handling, 0, "subscriptions request retained replay");

    image
        .write_bits(2, 0, 32, u64::from(1.5_f32.to_bits()))
        .then_some(())
        .expect("write the output image");
    backend.publish_outputs(&image).expect("publish outputs");
    // A session that comes up before the explicit boundary may publish the
    // initial image first, so wait for the value and not merely for a PUBLISH.
    let expected = 1.5_f32.to_be_bytes().to_vec();
    let Event::Publish {
        topic,
        payload,
        qos,
        retain,
    } = broker.wait_for(
        "the published output value",
        |event| matches!(event, Event::Publish { payload, .. } if *payload == expected),
    )
    else {
        unreachable!();
    };
    assert_eq!(topic, OUTPUT_TOPIC);
    assert_eq!(payload, expected);
    assert_eq!(qos, 1);
    assert!(retain, "the channel retain flag reaches the broker");

    backend.shutdown().expect("shutdown");
    // The DISCONNECT is decoded on the broker thread, so it can arrive shortly
    // after shutdown returns.
    let Event::Disconnect { reason } = broker.wait_for("a clean DISCONNECT", |event| {
        matches!(event, Event::Disconnect { .. })
    }) else {
        unreachable!();
    };
    assert_eq!(reason, 0, "shutdown sends the normal disconnection reason");
    broker.stop();
}
