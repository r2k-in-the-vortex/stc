//! Cross-host Modbus TCP end-to-end test (WL-5.1 phase 4).
//!
//! Copies the `tests/modbus/` fixture into a temporary project, replaces the
//! two loopback binds with free ports, builds and runs it with `stc run`, then
//! drives it through a real TCP client. The client writes every writable input
//! table and reads every output table using zero-based addresses, covers
//! 1-/8-/16-/32-/64-bit channels, the device-level `low-first` order, `dcba`
//! and explicit integer-list overrides, and proves both binds serve one bank.
//!
//! The run uses a finite cycle count so it stops cleanly and prints the
//! verbose status line. The test then asserts the backend summary and that
//! both ports are closed after exit. It needs clang, like the other
//! project-mode tests, and skips when clang is unavailable unless
//! `STC_REQUIRE_CLANG` is set.

mod common;

use std::fmt::Debug;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const UNIT_ID: u8 = 1;
const RUN_CYCLES: &str = "4000";

fn stc() -> &'static str {
    env!("CARGO_BIN_EXE_stc")
}

fn clang() -> Option<&'static str> {
    static CLANG: OnceLock<Option<&'static str>> = OnceLock::new();
    CLANG
        .get_or_init(|| {
            let candidate = std::env::var("STC_CLANG").unwrap_or_else(|_| "clang".into());
            match Command::new(&candidate).arg("--version").output() {
                Ok(output) if output.status.success() => {
                    Some(Box::leak(candidate.into_boxed_str()))
                }
                _ => {
                    if std::env::var_os("STC_REQUIRE_CLANG").is_some() {
                        panic!("clang is required (STC_REQUIRE_CLANG=1) but was not found");
                    }
                    None
                }
            }
        })
        .as_ref()
        .copied()
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stc-modbus-e2e-{}-{name}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test directory");
        Self { path }
    }

    fn copy(&self, source: &Path, relative: &str) {
        let destination = self.path.join(relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).expect("create fixture subdirectory");
        }
        std::fs::copy(source, &destination).expect("copy fixture file");
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create directory");
        }
        std::fs::write(path, contents).expect("write file");
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("tests/modbus")
}

/// Returns two distinct free loopback ports.
fn free_ports() -> (u16, u16) {
    loop {
        let first = TcpListener::bind("127.0.0.1:0").expect("bind free port");
        let first_port = first.local_addr().expect("first port").port();
        let second = TcpListener::bind("127.0.0.1:0").expect("bind free port");
        let second_port = second.local_addr().expect("second port").port();
        drop(first);
        drop(second);
        if first_port != second_port && first_port != 0 && second_port != 0 {
            return (first_port, second_port);
        }
    }
}

struct Client {
    stream: TcpStream,
    transaction: u16,
    /// Completed requests by class, used to check the backend counters.
    reads: u64,
    writes: u64,
}

impl Client {
    fn connect(port: u16, timeout: Duration) -> Self {
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let deadline = Instant::now() + timeout;
        loop {
            match TcpStream::connect_timeout(&address, Duration::from_millis(500)) {
                Ok(stream) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .expect("read timeout");
                    stream
                        .set_write_timeout(Some(Duration::from_secs(5)))
                        .expect("write timeout");
                    stream.set_nodelay(true).expect("nodelay");
                    return Self {
                        stream,
                        transaction: 0,
                        reads: 0,
                        writes: 0,
                    };
                }
                Err(error) => {
                    if Instant::now() >= deadline {
                        panic!("cannot connect to 127.0.0.1:{port} within {timeout:?}: {error}");
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }

    fn exchange(&mut self, pdu: &[u8]) -> Vec<u8> {
        self.transaction = self.transaction.wrapping_add(1);
        if self.transaction == 0 {
            self.transaction = 1;
        }
        let length = u16::try_from(pdu.len() + 1).expect("PDU length");
        let mut adu = Vec::with_capacity(7 + pdu.len());
        adu.extend_from_slice(&self.transaction.to_be_bytes());
        adu.extend_from_slice(&0_u16.to_be_bytes());
        adu.extend_from_slice(&length.to_be_bytes());
        adu.push(UNIT_ID);
        adu.extend_from_slice(pdu);
        self.stream.write_all(&adu).expect("write request");

        let mut header = [0_u8; 7];
        self.stream
            .read_exact(&mut header)
            .expect("read MBAP header");
        let transaction = u16::from_be_bytes([header[0], header[1]]);
        let protocol = u16::from_be_bytes([header[2], header[3]]);
        let length = u16::from_be_bytes([header[4], header[5]]);
        assert_eq!(transaction, self.transaction, "transaction id echo");
        assert_eq!(protocol, 0, "protocol id");
        assert_eq!(header[6], UNIT_ID, "unit id echo");
        assert!((2..=254).contains(&length), "MBAP length {length}");
        let mut payload = vec![0_u8; usize::from(length - 1)];
        self.stream
            .read_exact(&mut payload)
            .expect("read response PDU");
        // The server increments its counters before writing the response, so
        // a fully received response means this request is accounted for. The
        // same function-code split as the backend's `count_response` keeps the
        // two totals directly comparable, poll retries included.
        match pdu.first().copied().unwrap_or(0) {
            0x01..=0x04 => self.reads = self.reads.saturating_add(1),
            0x05 | 0x06 | 0x0F | 0x10 => self.writes = self.writes.saturating_add(1),
            _ => {}
        }
        payload
    }

    fn check_exception(&self, pdu: &[u8], function: u8) {
        if pdu.first() == Some(&(function | 0x80)) {
            panic!(
                "FC{function} returned exception {:?}",
                pdu.get(1).copied().unwrap_or(0)
            );
        }
    }

    fn read_bits(&mut self, function: u8, address: u16, quantity: u16) -> Vec<bool> {
        let mut pdu = vec![function];
        pdu.extend_from_slice(&address.to_be_bytes());
        pdu.extend_from_slice(&quantity.to_be_bytes());
        let response = self.exchange(&pdu);
        self.check_exception(&response, function);
        assert_eq!(response.first(), Some(&function), "FC{function} function");
        let byte_count = *response.get(1).expect("byte count") as usize;
        assert_eq!(
            byte_count,
            (usize::from(quantity)).div_ceil(8),
            "FC{function} byte count"
        );
        assert_eq!(
            response.len(),
            2 + byte_count,
            "FC{function} response length"
        );
        (0..usize::from(quantity))
            .map(|index| response[2 + index / 8] & (1 << (index % 8)) != 0)
            .collect()
    }

    fn read_registers(&mut self, function: u8, address: u16, quantity: u16) -> Vec<u16> {
        let mut pdu = vec![function];
        pdu.extend_from_slice(&address.to_be_bytes());
        pdu.extend_from_slice(&quantity.to_be_bytes());
        let response = self.exchange(&pdu);
        self.check_exception(&response, function);
        assert_eq!(response.first(), Some(&function), "FC{function} function");
        let byte_count = *response.get(1).expect("byte count") as usize;
        assert_eq!(byte_count, usize::from(quantity) * 2, "FC{function} bytes");
        assert_eq!(
            response.len(),
            2 + byte_count,
            "FC{function} response length"
        );
        response[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_be_bytes(*pair))
            .collect()
    }

    fn write_coil(&mut self, address: u16, value: bool) {
        let wire = if value { 0xFF00_u16 } else { 0x0000 };
        let mut pdu = vec![5];
        pdu.extend_from_slice(&address.to_be_bytes());
        pdu.extend_from_slice(&wire.to_be_bytes());
        let response = self.exchange(&pdu);
        self.check_exception(&response, 5);
        assert_eq!(response, pdu, "FC5 echo");
    }

    fn write_coils(&mut self, address: u16, values: &[bool]) {
        let quantity = u16::try_from(values.len()).expect("coil count");
        let byte_count = values.len().div_ceil(8);
        let mut pdu = vec![15];
        pdu.extend_from_slice(&address.to_be_bytes());
        pdu.extend_from_slice(&quantity.to_be_bytes());
        pdu.push(u8::try_from(byte_count).expect("coil byte count"));
        let mut packed = vec![0_u8; byte_count];
        for (index, &value) in values.iter().enumerate() {
            if value {
                packed[index / 8] |= 1 << (index % 8);
            }
        }
        pdu.extend_from_slice(&packed);
        let response = self.exchange(&pdu);
        self.check_exception(&response, 15);
        let mut expected = vec![15];
        expected.extend_from_slice(&address.to_be_bytes());
        expected.extend_from_slice(&quantity.to_be_bytes());
        assert_eq!(response, expected, "FC15 echo");
    }

    fn write_register(&mut self, address: u16, value: u16) {
        let mut pdu = vec![6];
        pdu.extend_from_slice(&address.to_be_bytes());
        pdu.extend_from_slice(&value.to_be_bytes());
        let response = self.exchange(&pdu);
        self.check_exception(&response, 6);
        assert_eq!(response, pdu, "FC6 echo");
    }

    fn write_registers(&mut self, address: u16, values: &[u16]) {
        let quantity = u16::try_from(values.len()).expect("register count");
        let mut pdu = vec![16];
        pdu.extend_from_slice(&address.to_be_bytes());
        pdu.extend_from_slice(&quantity.to_be_bytes());
        pdu.push(u8::try_from(values.len() * 2).expect("register byte count"));
        for value in values {
            pdu.extend_from_slice(&value.to_be_bytes());
        }
        let response = self.exchange(&pdu);
        self.check_exception(&response, 16);
        let mut expected = vec![16];
        expected.extend_from_slice(&address.to_be_bytes());
        expected.extend_from_slice(&quantity.to_be_bytes());
        assert_eq!(response, expected, "FC16 echo");
    }
}

fn expect<T: Debug + PartialEq>(label: &str, actual: T, expected: T) {
    assert_eq!(actual, expected, "{label}");
}

/// Repeats a read until the PLC scan has copied the value or the deadline
/// passes. The PLC copies input to output in its 1 ms task.
fn poll_until<T, F>(label: &str, mut read: F, expected: T) -> T
where
    T: Debug + PartialEq,
    F: FnMut() -> T,
{
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let actual = read();
        if actual == expected {
            return actual;
        }
        if Instant::now() >= deadline {
            panic!("{label}: got {actual:?}, expected {expected:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn diagnostic<'a>(text: &'a str, key: &str) -> &'a str {
    let prefix = format!("{key}=");
    text.split_whitespace()
        .find_map(|token| token.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("status line has no `{key}`: {text}"))
}

fn diagnostic_number(text: &str, key: &str) -> u64 {
    diagnostic(text, key)
        .parse()
        .unwrap_or_else(|_| panic!("`{key}` is not a number in: {text}"))
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn fixture_serves_all_tables_and_word_orders_through_two_binds() {
    let Some(clang) = clang() else {
        eprintln!("note: clang not found; skipping the Modbus end-to-end test");
        return;
    };
    let source_root = fixture_root();
    assert!(
        source_root.join("stc.toml").is_file(),
        "fixture not found at {}",
        source_root.display()
    );
    let project = TestDir::new("two-binds");
    for relative in ["stc.toml", "stc.lock", "src/main.st", "io/bench.toml"] {
        project.copy(&source_root.join(relative), relative);
    }

    // Replace only the loopback ports. Addressing and channel order stay
    // exactly as documented in tests/modbus/README.md.
    let profile =
        std::fs::read_to_string(source_root.join("io/bench.toml")).expect("read fixture profile");
    let (port_a, port_b) = free_ports();
    let original_bind = r#"bind = ["127.0.0.1:15020", "127.0.0.1:15021"]"#;
    assert_eq!(
        profile.matches(original_bind).count(),
        1,
        "fixture profile bind line changed"
    );
    let replacement = format!(r#"bind = ["127.0.0.1:{port_a}", "127.0.0.1:{port_b}"]"#);
    project.write(
        "io/bench.toml",
        &profile.replace(original_bind, &replacement),
    );

    let runtime = common::runtime_static();
    let mut command = Command::new(stc());
    command
        .arg("run")
        .arg("--manifest-path")
        .arg(project.path.join("stc.toml"))
        .args(["--cycles", RUN_CYCLES, "--cycle-time", "1ms", "-v"])
        .arg("--runtime")
        .arg(&runtime)
        .arg("--clang")
        .arg(clang)
        .current_dir(&project.path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command.spawn().expect("spawn stc run");

    let mut first = Client::connect(port_a, Duration::from_secs(30));
    let mut second = Client::connect(port_b, Duration::from_secs(30));

    // ---- Write every writable input table through bind A ------------------
    first.write_coil(0, true);
    first.write_coils(0, &[true, true, false]);
    // 8-bit low/high pair sharing holding register 0.
    first.write_register(0, 0x1234);
    first.write_register(2, 0xA1B2);
    // Device-level low-first: AABBCCDD -> CC DD AA BB.
    first.write_registers(4, &[0xCCDD, 0xAABB]);
    // Per-channel "dcba": 11223344 -> 44 33 22 11.
    first.write_registers(6, &[0x4433, 0x2211]);
    // Explicit [4,5,6,7,0,1,2,3]: 0102030405060708 -> 05 06 07 08 01 02 03 04.
    first.write_registers(12, &[0x0506, 0x0708, 0x0102, 0x0304]);

    // ---- Read every output table through bind B --------------------------
    let coil = poll_until(
        "coil output 1024",
        || second.read_bits(1, 1024, 1),
        vec![true],
    );
    expect("FC1 coil output 1024", coil, vec![true]);
    // FC5 and FC15 may straddle an input boundary: FC5 updates CoilIn0
    // first, so CoilOut can be readable before CoilIn1 (DiOut) has been
    // latched. Poll the discrete-input table like the register tables below.
    let discrete = poll_until(
        "discrete input output 0",
        || second.read_bits(2, 0, 1),
        vec![true],
    );
    expect("FC2 discrete input 0", discrete, vec![true]);

    let expected_outputs = vec![
        0x1234, 0xA1B2, // holding registers 8 and 10 (8- and 16-bit outputs)
        0xCCDD, 0xAABB, // input registers 0..1 (device low-first, 32-bit)
        0x4433, 0x2211, // input registers 2..3 ("dcba", 32-bit)
        0x0506, 0x0708, 0x0102, 0x0304, // input registers 4..7 (list, 64-bit)
    ];
    let outputs = poll_until(
        "output register tables",
        || {
            let mut values = second.read_registers(3, 8, 1);
            values.extend(second.read_registers(3, 10, 1));
            values.extend(second.read_registers(4, 0, 2));
            values.extend(second.read_registers(4, 2, 2));
            values.extend(second.read_registers(4, 4, 4));
            values
        },
        expected_outputs.clone(),
    );
    expect(
        "holding output register 8 (8-bit low/high pair)",
        outputs[0..1].to_vec(),
        vec![0x1234],
    );
    expect(
        "holding output register 10 (16-bit)",
        outputs[1..2].to_vec(),
        vec![0xA1B2],
    );
    expect(
        "input registers 0..1 (device low-first 32-bit)",
        outputs[2..4].to_vec(),
        vec![0xCCDD, 0xAABB],
    );
    expect(
        "input registers 2..3 (dcba 32-bit)",
        outputs[4..6].to_vec(),
        vec![0x4433, 0x2211],
    );
    expect(
        "input registers 4..7 (explicit list 64-bit)",
        outputs[6..10].to_vec(),
        vec![0x0506, 0x0708, 0x0102, 0x0304],
    );

    // The writable input tables also read back the last written bank value.
    expect(
        "coil inputs 0..2",
        second.read_bits(1, 0, 3),
        vec![true, true, false],
    );
    expect(
        "holding input register 0 (8-bit pair)",
        second.read_registers(3, 0, 1),
        vec![0x1234],
    );
    expect(
        "holding input registers 4..5 (device low-first)",
        second.read_registers(3, 4, 2),
        vec![0xCCDD, 0xAABB],
    );
    expect(
        "holding input registers 12..15 (explicit list)",
        second.read_registers(3, 12, 4),
        vec![0x0506, 0x0708, 0x0102, 0x0304],
    );

    // ---- Both binds serve one register bank ------------------------------
    second.write_register(2, 0xBEEF);
    let via_first = poll_until(
        "bind B write visible on bind A",
        || first.read_registers(3, 2, 1),
        vec![0xBEEF],
    );
    expect("bind B write visible on bind A", via_first, vec![0xBEEF]);
    let published = poll_until(
        "bind B write published to the output table",
        || first.read_registers(3, 10, 1),
        vec![0xBEEF],
    );
    expect(
        "bind B write published on bind A output register",
        published,
        vec![0xBEEF],
    );
    expect(
        "bind B reads its own write",
        second.read_registers(3, 2, 1),
        vec![0xBEEF],
    );

    // Both clients are the only traffic source. Snapshot their completed
    // request counts before closing them so the final backend counters can be
    // checked exactly, independent of how many poll attempts the PLC scan
    // timing required.
    let client_reads = first.reads + second.reads;
    let client_writes = first.writes + second.writes;

    drop(first);
    drop(second);

    let output = child.wait_with_output().expect("wait for stc run");
    let status = text(&output);
    assert_eq!(output.status.code(), Some(0), "{status}");

    // ---- Clean shutdown and backend summary ------------------------------
    assert!(
        status.contains("stc: status: io-backend modbus-tcp devices=2 state=running"),
        "{status}"
    );
    assert_eq!(diagnostic(&status, "faults"), "0", "{status}");
    assert_eq!(
        diagnostic(&status, "server0.bind"),
        format!("127.0.0.1:{port_a},127.0.0.1:{port_b}"),
        "{status}"
    );
    assert_eq!(diagnostic_number(&status, "server0.listeners"), 2);
    assert_eq!(diagnostic_number(&status, "server0.bound_listeners"), 2);
    assert_eq!(
        diagnostic(&status, "server0.listener0.state"),
        "listening",
        "{status}"
    );
    assert_eq!(
        diagnostic(&status, "server0.listener1.state"),
        "listening",
        "{status}"
    );
    assert_eq!(diagnostic_number(&status, "server0.peak_connections"), 2);
    assert_eq!(diagnostic_number(&status, "server0.connections"), 0);
    let requests = diagnostic_number(&status, "server0.requests");
    let reads = diagnostic_number(&status, "server0.reads");
    let writes = diagnostic_number(&status, "server0.writes");
    // Every write is issued exactly once. Polls may retry until the 1 ms PLC
    // scan latches the input and publishes the output, so their number is
    // timing-dependent; both the client and the server count every attempt,
    // making the exact equality below deterministic and stronger than a
    // minimum bound: the backend must account for every request the test
    // issued, in the correct read/write class, with no extra traffic.
    assert_eq!(writes, 8, "{status}");
    assert_eq!(client_writes, 8, "{status}");
    assert_eq!(writes, client_writes, "{status}");
    assert_eq!(reads, client_reads, "{status}");
    assert_eq!(requests, client_reads + client_writes, "{status}");
    assert_eq!(requests, reads + writes, "{status}");
    for key in [
        "server0.exceptions",
        "server0.malformed",
        "server0.unbound",
        "server0.last_error",
    ] {
        assert_eq!(diagnostic_number(&status, key), 0, "{key}: {status}");
    }

    // Both listeners are gone after the clean stop.
    for port in [port_a, port_b] {
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        assert!(
            TcpStream::connect_timeout(&address, Duration::from_millis(500)).is_err(),
            "listener on port {port} outlived the run"
        );
    }
}
