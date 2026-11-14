//! Cycle-boundary overhead for the MQTT client backend (WL-15 phase 5).
//!
//! The measured seam is the `DeviceBackend` boundary the coordinator calls on
//! the cycle thread: [`MqttBackend`]'s `latch_inputs` and `publish_outputs`.
//! Sockets, TLS, DNS, keepalive, and reconnect run on the session thread, so
//! the measured cost here is the bank/image copy plus mutex acquisition under
//! three broker conditions: idle, busy, and faulted.
//!
//! The test is ignored by default because wall-clock figures are noisy on
//! shared runners and this repository has no CI timing gate. Run it in release
//! on the host under test and record the output:
//!
//! ```text
//! cargo test -p stc-runtime --release --test mqtt_overhead -- --ignored --nocapture
//! ```
//!
//! `STC_MQTT_OVERHEAD_ITERATIONS` overrides the boundary iterations per
//! scenario (default 20000). Results are host-specific and are not a
//! real-time guarantee.

mod mqtt_sim;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mqtt_sim::{Script, SimBroker};
use stc_runtime::{
    BackendChannel, BackendConfig, BackendDeviceConfig, DeviceBackend, ImageLayout, ImageRange,
    MqttBackend, ProcessImage,
};

/// Bound channels per direction in the measured configuration.
const CHANNELS: usize = 16;

fn channel(name: String, direction: &str, area: u8, offset_bits: u64) -> (String, BackendChannel) {
    let json = format!(
        r#"{{"name":"{name}","direction":"{direction}","topic":"line/{name}","payload-encoding":"uint","qos":1}}"#
    );
    (
        json,
        BackendChannel {
            name,
            direction: direction.to_owned(),
            width_bits: 16,
            image: Some(ImageRange {
                area,
                offset_bits,
                width_bits: 16,
            }),
        },
    )
}

fn backend_config(port: u16) -> BackendConfig {
    let mut entries = Vec::new();
    let mut channels = Vec::new();
    for index in 0..CHANNELS {
        let (json, channel) = channel(format!("in{index}"), "input", 1, 16 * index as u64);
        entries.push(json);
        channels.push(channel);
    }
    for index in 0..CHANNELS {
        let (json, channel) = channel(format!("out{index}"), "output", 2, 16 * index as u64);
        entries.push(json);
        channels.push(channel);
    }
    BackendConfig {
        name: "mqtt".to_owned(),
        devices: vec![BackendDeviceConfig {
            name: "broker0".to_owned(),
            kind: "mqtt".to_owned(),
            config: Some(format!(
                r#"{{"host":"127.0.0.1","port":{port},"client-id":"stc-overhead","keepalive":"30s",
                    "connect-timeout":"200ms","reconnect-backoff":["20ms"],"stale-timeout":"2s",
                    "channels":[{}]}}"#,
                entries.join(",")
            )),
            channels,
        }],
        bindings: Vec::new(),
        layout: ImageLayout {
            inputs_bytes: 2 * CHANNELS as u64,
            outputs_bytes: 2 * CHANNELS as u64,
            markers_bytes: 0,
        },
        profile: None,
        base_dir: PathBuf::from("."),
    }
}

fn percentile(sorted: &[u64], percent: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let last = sorted.len() - 1;
    let index = (last * percent).div_ceil(100);
    sorted[index.min(last)]
}

struct Distribution {
    p50_ns: u64,
    p99_ns: u64,
    max_ns: u64,
    mean_ns: f64,
}

impl Distribution {
    fn from_samples(samples: &mut [u64]) -> Self {
        if samples.is_empty() {
            return Self {
                p50_ns: 0,
                p99_ns: 0,
                max_ns: 0,
                mean_ns: 0.0,
            };
        }
        samples.sort_unstable();
        let sum: u128 = samples.iter().map(|&value| u128::from(value)).sum();
        Self {
            p50_ns: percentile(samples, 50),
            p99_ns: percentile(samples, 99),
            max_ns: *samples.last().expect("non-empty"),
            mean_ns: sum as f64 / samples.len() as f64,
        }
    }
}

fn micros(nanoseconds: u64) -> f64 {
    nanoseconds as f64 / 1000.0
}

fn iterations() -> usize {
    std::env::var("STC_MQTT_OVERHEAD_ITERATIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|&value| value > 0)
        .unwrap_or(20_000)
}

/// Runs one scenario and prints one result row.
fn measure(label: &str, port: u16, iterations: usize) {
    let config = backend_config(port);
    let mut backend = MqttBackend::new();
    backend.cold_start(&config).expect("cold start");
    let mut image = ProcessImage::new(config.layout);

    // Warm up the boundary and let the session settle into its state.
    for _ in 0..2_000 {
        backend.latch_inputs(&mut image).expect("warm-up latch");
        backend.publish_outputs(&image).expect("warm-up publish");
    }
    let settle = Instant::now();
    while settle.elapsed() < Duration::from_secs(1) {
        backend.latch_inputs(&mut image).expect("settle latch");
        backend.publish_outputs(&image).expect("settle publish");
        std::thread::sleep(Duration::from_millis(5));
    }

    let mut latch_samples = Vec::with_capacity(iterations);
    let mut publish_samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        backend.latch_inputs(&mut image).expect("latch");
        latch_samples.push(started.elapsed().as_nanos() as u64);

        let started = Instant::now();
        backend.publish_outputs(&image).expect("publish");
        publish_samples.push(started.elapsed().as_nanos() as u64);
    }

    let latch = Distribution::from_samples(&mut latch_samples);
    let publish = Distribution::from_samples(&mut publish_samples);
    let diagnostics = backend.diagnostics();
    let status = backend.status();
    println!(
        "mqtt-overhead: {label} {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} {} {} {} {}",
        micros(latch.p50_ns),
        micros(latch.p99_ns),
        micros(latch.max_ns),
        micros(publish.p50_ns),
        micros(publish.p99_ns),
        micros(publish.max_ns),
        (latch.mean_ns + publish.mean_ns) / 1000.0,
        status.state.as_str(),
        diagnostics["client0.state"].clone(),
        diagnostics["client0.messages_in"].clone(),
        diagnostics["client0.reconnects"].clone(),
    );

    backend.shutdown().expect("shutdown");
}

#[test]
#[ignore = "timing evidence; run explicitly in release with --ignored --nocapture"]
fn cycle_boundary_overhead_with_idle_busy_and_faulted_brokers() {
    #[allow(clippy::assertions_on_constants)] // compile-time release guard is intentional
    {
        assert!(
            !cfg!(debug_assertions),
            "run the overhead harness in release (cargo test --release)"
        );
    }
    let iterations = iterations();
    println!(
        "mqtt-overhead: host={} arch={} release={} iterations={iterations} channels={} \
         inputs={}B outputs={}B",
        std::env::consts::OS,
        std::env::consts::ARCH,
        !cfg!(debug_assertions),
        2 * CHANNELS,
        2 * CHANNELS,
        2 * CHANNELS,
    );
    println!(
        "mqtt-overhead: columns: scenario latch_p50_us latch_p99_us latch_max_us publish_p50_us \
         publish_p99_us publish_max_us cycle_mean_us backend_state client_state messages_in \
         reconnects"
    );

    // Idle broker: connected, no traffic.
    let idle = SimBroker::spawn(Script::quiet());
    measure("idle", idle.port(), iterations);
    idle.stop();

    // Busy broker: one input every millisecond.
    let busy = SimBroker::spawn(Script {
        publish_every: Some(("line/in0".to_owned(), Duration::from_millis(1))),
        ..Script::quiet()
    });
    measure("busy", busy.port(), iterations);
    busy.stop();

    // Faulted broker: nothing is listening, the client retries with backoff.
    let port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind closed port");
        listener.local_addr().expect("closed port address").port()
    };
    measure("faulted", port, iterations);
}
