//! Cycle-boundary and request overhead for the Modbus TCP server backend.
//!
//! The public, reproducible boundary measured here is the `DeviceBackend`
//! seam the coordinator calls on the cycle thread: [`ModbusTcpBackend`]'s
//! `latch_inputs` and `publish_outputs`. Socket work never runs on that seam,
//! so the measured cost is the register-bank/image copy plus mutex
//! acquisition. The harness runs 0, 1, 4, and 8 loopback clients that issue
//! read-coil, read-holding-register, read-input-register, and
//! write-multiple-registers requests while the boundary loop runs.
//!
//! The test is ignored by default because wall-clock figures are noisy on
//! shared runners and this repository has no CI timing gate. Run it in release
//! on the host under test and record the output:
//!
//! ```text
//! cargo test -p stc-runtime --release --test modbus_overhead -- --ignored --nocapture
//! ```
//!
//! `STC_MODBUS_OVERHEAD_ITERATIONS` overrides the boundary iterations per
//! client count (default 50000) and `STC_MODBUS_OVERHEAD_CLIENTS` overrides the
//! client list (default `0,1,4,8`). Results are host-specific and are not a
//! real-time guarantee.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::time::{Duration, Instant};

use stc_runtime::io_modbus_tcp::MODBUS_TCP_KIND;
use stc_runtime::{
    BackendBinding, BackendChannel, BackendConfig, BackendDeviceConfig, DeviceBackend, ImageLayout,
    ImageRange, ModbusTcpBackend, ProcessImage,
};

/// One channel in the measured configuration.
struct Spec {
    name: String,
    direction: &'static str,
    width: u32,
    area: u8,
    offset: u64,
    table: &'static str,
    address: u16,
}

/// The measured configuration: 56 bound channels over both bit tables and both
/// register tables, with 1/16/32/64-bit values.
///
/// Input image: 48 bytes (328 used bits). Output image: 64 bytes (464 used
/// bits). The channel count and width mix are deliberately larger than a
/// small project and comparable to the `tests/modbus/` fixture so the copy
/// cost is visible above timer noise.
fn channel_specs() -> Vec<Spec> {
    let mut specs = Vec::new();

    for index in 0..8_u16 {
        specs.push(Spec {
            name: format!("coil_in{index}"),
            direction: "input",
            width: 1,
            area: 1,
            offset: u64::from(index),
            table: "coil",
            address: index,
        });
    }
    for index in 0..8_u16 {
        specs.push(Spec {
            name: format!("hr_in{index}"),
            direction: "input",
            width: 16,
            area: 1,
            offset: 16 + 16 * u64::from(index),
            table: "holding-register",
            address: index,
        });
    }
    for index in 0..2_u16 {
        specs.push(Spec {
            name: format!("u32_in{index}"),
            direction: "input",
            width: 32,
            area: 1,
            offset: 144 + 32 * u64::from(index),
            table: "holding-register",
            address: 20 + 2 * index,
        });
    }
    for index in 0..2_u16 {
        specs.push(Spec {
            name: format!("u64_in{index}"),
            direction: "input",
            width: 64,
            area: 1,
            offset: 208 + 64 * u64::from(index),
            table: "holding-register",
            address: 30 + 8 * index,
        });
    }

    for index in 0..8_u16 {
        specs.push(Spec {
            name: format!("coil_out{index}"),
            direction: "output",
            width: 1,
            area: 2,
            offset: u64::from(index),
            table: "coil",
            address: 1024 + index,
        });
    }
    for index in 0..8_u16 {
        specs.push(Spec {
            name: format!("discrete_out{index}"),
            direction: "output",
            width: 1,
            area: 2,
            offset: 8 + u64::from(index),
            table: "discrete-input",
            address: index,
        });
    }
    for index in 0..8_u16 {
        specs.push(Spec {
            name: format!("hr_out{index}"),
            direction: "output",
            width: 16,
            area: 2,
            offset: 16 + 16 * u64::from(index),
            table: "holding-register",
            address: 1000 + index,
        });
    }
    for index in 0..2_u16 {
        specs.push(Spec {
            name: format!("u32_out{index}"),
            direction: "output",
            width: 32,
            area: 2,
            offset: 144 + 32 * u64::from(index),
            table: "holding-register",
            address: 1010 + 2 * index,
        });
    }
    for index in 0..2_u16 {
        specs.push(Spec {
            name: format!("u64_out{index}"),
            direction: "output",
            width: 64,
            area: 2,
            offset: 208 + 64 * u64::from(index),
            table: "holding-register",
            address: 1020 + 8 * index,
        });
    }
    for index in 0..8_u16 {
        specs.push(Spec {
            name: format!("ir_out{index}"),
            direction: "output",
            width: 16,
            area: 2,
            offset: 336 + 16 * u64::from(index),
            table: "input-register",
            address: index,
        });
    }

    specs
}

fn image_range(spec: &Spec) -> ImageRange {
    ImageRange {
        area: spec.area,
        offset_bits: spec.offset,
        width_bits: u64::from(spec.width),
    }
}

fn backend_config(specs: &[Spec]) -> BackendConfig {
    let mut channels_json = String::from(
        r#"{"bind":["127.0.0.1:0"],"unit":1,"max-connections":16,"idle-timeout":"30s","word-order":"low-first","channels":["#,
    );
    for (index, spec) in specs.iter().enumerate() {
        if index > 0 {
            channels_json.push(',');
        }
        channels_json.push_str(&format!(
            r#"{{"name":"{}","table":"{}","address":{}}}"#,
            spec.name, spec.table, spec.address
        ));
    }
    channels_json.push_str("]}");

    let channels: Vec<BackendChannel> = specs
        .iter()
        .map(|spec| BackendChannel {
            name: spec.name.clone(),
            direction: spec.direction.to_owned(),
            width_bits: spec.width,
            image: Some(image_range(spec)),
        })
        .collect();
    let bindings: Vec<BackendBinding> = specs
        .iter()
        .map(|spec| BackendBinding {
            device: "mb0".to_owned(),
            channel: spec.name.clone(),
            image: image_range(spec),
            direction: spec.direction.to_owned(),
        })
        .collect();

    BackendConfig {
        name: "modbus-tcp".to_owned(),
        devices: vec![BackendDeviceConfig {
            name: "mb0".to_owned(),
            kind: MODBUS_TCP_KIND.to_owned(),
            config: Some(channels_json),
            channels,
        }],
        bindings,
        layout: ImageLayout {
            inputs_bytes: 48,
            outputs_bytes: 64,
            markers_bytes: 0,
        },
        profile: None,
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

/// A request sequence that exercises both directions and both table kinds.
fn request_pdu(kind: usize, sequence: u16) -> Vec<u8> {
    match kind % 4 {
        0 => vec![0x01, 0x04, 0x00, 0x00, 0x08],
        1 => vec![0x03, 0x03, 0xE8, 0x00, 0x08],
        2 => vec![0x04, 0x00, 0x00, 0x00, 0x08],
        _ => {
            let mut pdu = vec![0x10, 0x00, 0x00, 0x00, 0x04, 0x08];
            for step in 0..4_u16 {
                pdu.extend_from_slice(&sequence.wrapping_add(step).to_be_bytes());
            }
            pdu
        }
    }
}

fn read_response(stream: &mut TcpStream, transaction: u16, function: u8) -> io::Result<Vec<u8>> {
    let mut header = [0_u8; 7];
    stream.read_exact(&mut header)?;
    let echoed = u16::from_be_bytes([header[0], header[1]]);
    if echoed != transaction {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("transaction {echoed} != {transaction}"),
        ));
    }
    if u16::from_be_bytes([header[2], header[3]]) != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "non-zero protocol id",
        ));
    }
    let length = usize::from(u16::from_be_bytes([header[4], header[5]]));
    if length < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid MBAP length {length}"),
        ));
    }
    let mut pdu = vec![0_u8; length - 1];
    stream.read_exact(&mut pdu)?;
    if pdu.first().is_some_and(|&code| code & 0x80 != 0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("exception response 0x{:02x}", pdu[1]),
        ));
    }
    if pdu.first() != Some(&function) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("function echoed as {:?}", pdu.first()),
        ));
    }
    Ok(pdu)
}

struct ClientOutcome {
    rtts_ns: Vec<u64>,
    requests: u64,
    errors: Vec<String>,
}

fn client_worker(
    bind: SocketAddr,
    stop: Arc<AtomicBool>,
    barrier: Arc<Barrier>,
    ready: mpsc::Sender<Result<(), String>>,
) -> ClientOutcome {
    let mut outcome = ClientOutcome {
        rtts_ns: Vec::new(),
        requests: 0,
        errors: Vec::new(),
    };
    let mut stream = match TcpStream::connect(bind) {
        Ok(stream) => stream,
        Err(error) => {
            let _ = ready.send(Err(format!("connect {bind}: {error}")));
            return outcome;
        }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_nodelay(true);
    let _ = ready.send(Ok(()));
    barrier.wait();

    let mut sequence = 0_u16;
    while !stop.load(Ordering::Relaxed) {
        let kind = usize::from(sequence % 4);
        let function = match kind {
            0 => 0x01,
            1 => 0x03,
            2 => 0x04,
            _ => 0x10,
        };
        let transaction = sequence;
        let mut adu = vec![(transaction >> 8) as u8, transaction as u8, 0, 0, 0, 0, 1];
        let pdu = request_pdu(kind, sequence);
        adu[4] = ((pdu.len() + 1) >> 8) as u8;
        adu[5] = (pdu.len() + 1) as u8;
        adu.extend_from_slice(&pdu);

        let started = Instant::now();
        let result = stream
            .write_all(&adu)
            .and_then(|()| read_response(&mut stream, transaction, function));
        let elapsed = started.elapsed();
        match result {
            Ok(_) => {
                outcome.rtts_ns.push(elapsed.as_nanos() as u64);
                outcome.requests += 1;
            }
            Err(error) => {
                outcome.errors.push(format!("request {sequence}: {error}"));
                break;
            }
        }
        sequence = sequence.wrapping_add(1);
    }
    outcome
}

/// Returns process CPU time when the host exposes it without a dependency.
#[cfg(windows)]
mod process_cpu {
    use std::ffi::c_void;
    use std::time::Duration;

    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn GetProcessTimes(
            process: *mut c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }

    pub fn now() -> Option<Duration> {
        let mut creation = FileTime::default();
        let mut exit = FileTime::default();
        let mut kernel = FileTime::default();
        let mut user = FileTime::default();
        // SAFETY: the process pseudo-handle is always valid and every output
        // pointer refers to a live, writable `FileTime`.
        let ok = unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        };
        if ok == 0 {
            return None;
        }
        let kernel_100ns = (u64::from(kernel.high) << 32) | u64::from(kernel.low);
        let user_100ns = (u64::from(user.high) << 32) | u64::from(user.low);
        Some(Duration::from_nanos(
            kernel_100ns.saturating_add(user_100ns) * 100,
        ))
    }
}

#[cfg(target_os = "linux")]
mod process_cpu {
    use std::time::Duration;

    #[repr(C)]
    struct Timespec {
        seconds: i64,
        nanoseconds: i64,
    }

    unsafe extern "C" {
        fn clock_gettime(clock_id: i32, time: *mut Timespec) -> i32;
    }

    const CLOCK_PROCESS_CPUTIME_ID: i32 = 2;

    pub fn now() -> Option<Duration> {
        let mut time = Timespec {
            seconds: 0,
            nanoseconds: 0,
        };
        // SAFETY: `time` points to a live, writable `Timespec`.
        let ok = unsafe { clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &mut time) };
        if ok != 0 || time.seconds < 0 || time.nanoseconds < 0 {
            return None;
        }
        Some(Duration::new(time.seconds as u64, time.nanoseconds as u32))
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod process_cpu {
    use std::time::Duration;

    pub fn now() -> Option<Duration> {
        None
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|&value| value > 0)
        .unwrap_or(default)
}

fn client_counts() -> Vec<usize> {
    let Ok(value) = std::env::var("STC_MODBUS_OVERHEAD_CLIENTS") else {
        return vec![0, 1, 4, 8];
    };
    let counts: Vec<usize> = value
        .split(',')
        .filter_map(|entry| entry.trim().parse().ok())
        .collect();
    if counts.is_empty() {
        vec![0, 1, 4, 8]
    } else {
        counts
    }
}

#[test]
#[ignore = "timing evidence; run explicitly in release with --ignored --nocapture"]
fn cycle_boundary_and_request_overhead_with_0_1_4_8_clients() {
    #[allow(clippy::assertions_on_constants)] // compile-time release guard is intentional
    {
        assert!(
            !cfg!(debug_assertions),
            "run the overhead harness in release (cargo test --release)"
        );
    }
    let iterations = env_usize("STC_MODBUS_OVERHEAD_ITERATIONS", 50_000);
    let counts = client_counts();
    let specs = channel_specs();
    let config = backend_config(&specs);
    let mut backend = ModbusTcpBackend::new();
    backend.cold_start(&config).expect("cold start");
    let bind: SocketAddr = backend.diagnostics()["server0.bind"]
        .parse()
        .expect("effective bind");
    let mut image = ProcessImage::new(config.layout);

    println!(
        "modbus-overhead: host={} arch={} release={} iterations={iterations} \
         channels={} inputs={}B outputs={}B bind={bind}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        !cfg!(debug_assertions),
        specs.len(),
        config.layout.inputs_bytes,
        config.layout.outputs_bytes,
    );
    println!(
        "modbus-overhead: columns: clients latch_p50_us latch_p99_us latch_max_us \
         publish_p50_us publish_p99_us publish_max_us cycle_mean_us rtt_p50_us \
         rtt_p99_us rtt_max_us requests_per_s process_cpu_s cpu_percent"
    );

    for client_count in counts {
        let stop = Arc::new(AtomicBool::new(false));
        let barrier = Arc::new(Barrier::new(client_count + 1));
        let (ready_tx, ready_rx) = mpsc::channel();
        let mut handles = Vec::with_capacity(client_count);
        for _ in 0..client_count {
            let stop = stop.clone();
            let barrier = barrier.clone();
            let ready_tx = ready_tx.clone();
            handles.push(std::thread::spawn(move || {
                client_worker(bind, stop, barrier, ready_tx)
            }));
        }
        drop(ready_tx);
        for _ in 0..client_count {
            match ready_rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => panic!("client failed to connect: {error}"),
                Err(error) => panic!("client readiness channel closed: {error}"),
            }
        }

        // Warm up the boundary and the sockets before measuring.
        for _ in 0..2_000 {
            backend.latch_inputs(&mut image).expect("warm-up latch");
            backend.publish_outputs(&image).expect("warm-up publish");
        }
        barrier.wait();

        let mut latch_samples = Vec::with_capacity(iterations);
        let mut publish_samples = Vec::with_capacity(iterations);
        let wall_start = Instant::now();
        let cpu_start = process_cpu::now();
        for _ in 0..iterations {
            let started = Instant::now();
            backend.latch_inputs(&mut image).expect("latch");
            latch_samples.push(started.elapsed().as_nanos() as u64);

            let started = Instant::now();
            backend.publish_outputs(&image).expect("publish");
            publish_samples.push(started.elapsed().as_nanos() as u64);
        }
        let wall = wall_start.elapsed();
        stop.store(true, Ordering::Relaxed);

        let mut outcomes = Vec::with_capacity(client_count);
        for handle in handles {
            outcomes.push(handle.join().expect("client thread"));
        }
        let cpu_wall = wall_start.elapsed();
        let cpu_end = process_cpu::now();

        let mut rtts: Vec<u64> = Vec::new();
        let mut requests = 0_u64;
        for outcome in &outcomes {
            assert!(
                outcome.errors.is_empty(),
                "client error with {client_count} clients: {:?}",
                outcome.errors
            );
            rtts.extend_from_slice(&outcome.rtts_ns);
            requests += outcome.requests;
        }
        if client_count > 0 {
            assert!(
                requests > 0,
                "no requests completed with {client_count} clients"
            );
        }

        let latch = Distribution::from_samples(&mut latch_samples);
        let publish = Distribution::from_samples(&mut publish_samples);
        let rtt = Distribution::from_samples(&mut rtts);
        let cycle_mean = latch.mean_ns + publish.mean_ns;
        let throughput = requests as f64 / wall.as_secs_f64();
        let (cpu_seconds, cpu_percent) = match (cpu_start, cpu_end) {
            (Some(start), Some(end)) if end >= start => {
                let seconds = (end - start).as_secs_f64();
                (seconds, seconds / cpu_wall.as_secs_f64() * 100.0)
            }
            _ => (f64::NAN, f64::NAN),
        };

        println!(
            "modbus-overhead: {client_count} {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} \
             {:.3} {:.3} {:.3} {throughput:.0} {cpu_seconds:.3} {cpu_percent:.1}",
            micros(latch.p50_ns),
            micros(latch.p99_ns),
            micros(latch.max_ns),
            micros(publish.p50_ns),
            micros(publish.p99_ns),
            micros(publish.max_ns),
            cycle_mean / 1000.0,
            micros(rtt.p50_ns),
            micros(rtt.p99_ns),
            micros(rtt.max_ns),
        );
    }

    backend.shutdown().expect("shutdown");
}
