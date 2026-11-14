//! Scripted MQTT 5.0 broker simulator for tests (WL-15 phase 5).
//!
//! This module is test-only: it is compiled into the `stc-runtime` integration
//! tests, is never part of the runtime crate, and is not a supported product
//! surface. It speaks just enough MQTT 5.0 to stand in for a broker on
//! loopback: CONNECT/CONNACK, SUBSCRIBE/SUBACK, PUBLISH with QoS 0/1/2,
//! PINGREQ/PINGRESP, and DISCONNECT.
//!
//! Everything the client sends is decoded and recorded as an [`Event`], so a
//! test can assert baked-in protocol details (clean start, session expiry,
//! will, credentials, subscription filters, published payloads) instead of
//! re-parsing frames itself. A [`Script`] injects deterministic faults: a
//! delayed, rejected, or absent CONNACK, a dropped transport, a malformed
//! frame, an oversized frame, a duplicate delivery, and retained replay.
//!
//! The simulator keeps accepting connections after a fault, so reconnect and
//! recovery tests observe the retry ladder instead of a closed port. Every
//! socket is loopback with an ephemeral port; no external service is involved.

#![allow(dead_code)]

use std::io::{ErrorKind as IoErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use stc_runtime::io_mqtt::packet::{Packet, Publish, QoS};

/// Longest a test helper waits for a broker event or a frame.
pub const READY: Duration = Duration::from_secs(5);

/// Bound on one inbound client frame the simulator accepts.
const MAX_FRAME: usize = 1024 * 1024;

/// Poll interval used by the simulator's serve loop.
const POLL: Duration = Duration::from_millis(5);

/// One scripted fault or delivery executed after SUBACK.
#[derive(Clone, Debug)]
pub enum Action {
    /// Send one PUBLISH. `duplicate` sends the identical frame twice.
    Publish {
        topic: String,
        payload: Vec<u8>,
        qos: QoS,
        retain: bool,
        duplicate: bool,
    },
    /// Pause the connection.
    Sleep(Duration),
    /// Close the transport without a DISCONNECT.
    Close,
    /// Send a frame with a reserved packet type.
    Malformed,
    /// Send a PUBLISH whose payload exceeds the client's packet limit.
    Oversized,
}

impl Action {
    /// Builds a QoS 0 publish action.
    pub fn publish(topic: &str, payload: Vec<u8>) -> Self {
        Self::Publish {
            topic: topic.to_owned(),
            payload,
            qos: QoS::AtMostOnce,
            retain: false,
            duplicate: false,
        }
    }

    /// Builds a retained QoS 0 publish action.
    pub fn retained(topic: &str, payload: Vec<u8>) -> Self {
        Self::Publish {
            topic: topic.to_owned(),
            payload,
            qos: QoS::AtMostOnce,
            retain: true,
            duplicate: false,
        }
    }
}

/// A deterministic broker script.
#[derive(Clone, Debug, Default)]
pub struct Script {
    /// CONNACK reason code; `None` accepts the connection.
    pub connack_reason: Option<u8>,
    /// Delay before CONNACK is sent.
    pub connack_delay: Option<Duration>,
    /// Drop the transport after CONNECT and before any CONNACK.
    pub drop_before_connack: bool,
    /// Accept CONNECT and never answer.
    pub silent: bool,
    /// Faults executed after the first SUBACK of the first connection.
    ///
    /// Later connections serve normally, so a test observes the reconnect
    /// ladder instead of a broker that faults every session.
    pub after_subscribe: Vec<Action>,
    /// Publish `topic` with an incrementing `u16` payload at this interval.
    pub publish_every: Option<(String, Duration)>,
}

impl Script {
    /// Returns a script that only completes the handshake.
    pub fn quiet() -> Self {
        Self::default()
    }
}

/// One decoded client packet or lifecycle observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The client sent CONNECT.
    Connect {
        client_id: String,
        clean_start: bool,
        keep_alive: u16,
        session_expiry: u32,
        username: Option<String>,
        password: Option<Vec<u8>>,
        will: Option<(String, Vec<u8>)>,
    },
    /// The client sent SUBSCRIBE.
    Subscribe {
        filters: Vec<(String, u8)>,
        retain_handling: u8,
    },
    /// The client published a message.
    Publish {
        topic: String,
        payload: Vec<u8>,
        qos: u8,
        retain: bool,
    },
    /// The client acknowledged a QoS 1 publish.
    PubAck(u16),
    /// The client released a QoS 2 publish.
    PubRel(u16),
    /// The client sent PINGREQ.
    PingReq,
    /// The client sent DISCONNECT with this reason code.
    Disconnect { reason: u8 },
    /// The transport ended without DISCONNECT.
    Closed,
}

/// A decoded client packet.
enum ClientPacket {
    Connect {
        client_id: String,
        clean_start: bool,
        keep_alive: u16,
        session_expiry: u32,
        username: Option<String>,
        password: Option<Vec<u8>>,
        will: Option<(String, Vec<u8>)>,
    },
    Subscribe {
        packet_id: u16,
        filters: Vec<(String, u8)>,
        retain_handling: u8,
    },
    Publish {
        topic: String,
        payload: Vec<u8>,
        qos: u8,
        retain: bool,
        packet_id: Option<u16>,
    },
    PubAck(u16),
    PubRec(u16),
    PubRel(u16),
    PubComp(u16),
    PingReq,
    Disconnect {
        reason: u8,
    },
}

struct Recorder {
    events: Mutex<Vec<Event>>,
    changed: Condvar,
}

impl Recorder {
    fn push(&self, event: Event) {
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        events.push(event);
        self.changed.notify_all();
    }

    fn wait_for(&self, what: &str, predicate: impl Fn(&Event) -> bool) -> Event {
        let deadline = Instant::now() + READY;
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        loop {
            if let Some(event) = events.iter().find(|event| predicate(event)) {
                return event.clone();
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                panic!("timed out waiting for {what}; recorded {events:?}");
            }
            let (next, timeout) = self
                .changed
                .wait_timeout(events, remaining)
                .unwrap_or_else(|error| error.into_inner());
            events = next;
            if timeout.timed_out() && !events.iter().any(&predicate) {
                panic!("timed out waiting for {what}; recorded {events:?}");
            }
        }
    }

    fn snapshot(&self) -> Vec<Event> {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

/// A running simulator instance.
pub struct SimBroker {
    address: SocketAddr,
    recorder: Arc<Recorder>,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl SimBroker {
    /// Binds an ephemeral loopback port and runs `script` for every connection.
    pub fn spawn(script: Script) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind MQTT simulator");
        listener
            .set_nonblocking(true)
            .expect("nonblocking MQTT simulator listener");
        let address = listener.local_addr().expect("MQTT simulator address");
        let recorder = Arc::new(Recorder {
            events: Mutex::new(Vec::new()),
            changed: Condvar::new(),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(AtomicU64::new(0));
        let thread_connections = Arc::clone(&connections);
        let thread_recorder = Arc::clone(&recorder);
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                let socket = match listener.accept() {
                    Ok((socket, _)) => socket,
                    Err(error) if error.kind() == IoErrorKind::WouldBlock => {
                        thread::sleep(POLL);
                        continue;
                    }
                    Err(_) => return,
                };
                if socket.set_nonblocking(false).is_err() {
                    continue;
                }
                let _ = socket.set_read_timeout(Some(POLL));
                let _ = socket.set_write_timeout(Some(READY));
                let _ = socket.set_nodelay(true);
                let mut connection = Connection {
                    socket,
                    buffer: Vec::new(),
                };
                let first = thread_connections.fetch_add(1, Ordering::SeqCst) == 0;
                connection.run(&script, &thread_recorder, &thread_stop, first);
            }
        });
        Self {
            address,
            recorder,
            stop,
            thread,
        }
    }

    /// Returns the loopback address of the simulator.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Returns the port of the simulator.
    pub fn port(&self) -> u16 {
        self.address.port()
    }

    /// Waits for a recorded event matching `predicate`.
    pub fn wait_for(&self, what: &str, predicate: impl Fn(&Event) -> bool) -> Event {
        self.recorder.wait_for(what, predicate)
    }

    /// Returns every event recorded so far.
    pub fn events(&self) -> Vec<Event> {
        self.recorder.snapshot()
    }

    /// Stops accepting and joins the simulator thread.
    pub fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.join().expect("MQTT simulator thread");
    }
}

struct Connection {
    socket: TcpStream,
    buffer: Vec<u8>,
}

impl Connection {
    /// Reads from the socket into the frame buffer.
    fn pump(&mut self) -> std::io::Result<usize> {
        let mut chunk = [0_u8; 4096];
        let read = self.socket.read(&mut chunk)?;
        if read == 0 {
            return Err(std::io::Error::from(IoErrorKind::UnexpectedEof));
        }
        self.buffer.extend_from_slice(&chunk[..read]);
        if self.buffer.len() > MAX_FRAME {
            return Err(std::io::Error::from(IoErrorKind::InvalidData));
        }
        Ok(read)
    }

    /// Removes one complete frame from the buffer, when present.
    fn take_frame(&mut self) -> Option<Vec<u8>> {
        let mut index = 1;
        let mut remaining = 0_usize;
        let mut multiplier = 1_usize;
        loop {
            let byte = *self.buffer.get(index)?;
            remaining += usize::from(byte & 0x7f) * multiplier;
            index += 1;
            if byte & 0x80 == 0 {
                break;
            }
            if index > 4 {
                return None;
            }
            multiplier *= 128;
        }
        let total = index + remaining;
        if self.buffer.len() < total {
            return None;
        }
        Some(self.buffer.drain(..total).collect())
    }

    /// Blocks until one complete frame is available.
    fn next_frame(&mut self, deadline: Instant) -> Option<Vec<u8>> {
        loop {
            if let Some(frame) = self.take_frame() {
                return Some(frame);
            }
            if Instant::now() >= deadline {
                return None;
            }
            match self.pump() {
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        IoErrorKind::WouldBlock | IoErrorKind::TimedOut | IoErrorKind::Interrupted
                    ) => {}
                Err(_) => return None,
            }
        }
    }

    fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.socket.write_all(bytes)?;
        self.socket.flush()
    }

    fn send_publish(
        &mut self,
        topic: &str,
        payload: Vec<u8>,
        qos: QoS,
        retain: bool,
        packet_id: Option<u16>,
    ) -> std::io::Result<()> {
        let frame = Packet::Publish(Publish {
            topic: topic.to_owned(),
            payload,
            qos,
            retain,
            dup: false,
            packet_id,
            properties: stc_runtime::io_mqtt::packet::Properties::default(),
        })
        .encode()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
        self.send(&frame)
    }

    fn run(&mut self, script: &Script, recorder: &Recorder, stop: &AtomicBool, first: bool) {
        let deadline = Instant::now() + READY;
        let Some(connect) = self.next_frame(deadline) else {
            recorder.push(Event::Closed);
            return;
        };
        let Some(ClientPacket::Connect {
            client_id,
            clean_start,
            keep_alive,
            session_expiry,
            username,
            password,
            will,
        }) = decode_client(&connect)
        else {
            recorder.push(Event::Closed);
            return;
        };
        recorder.push(Event::Connect {
            client_id,
            clean_start,
            keep_alive,
            session_expiry,
            username,
            password,
            will,
        });

        if script.drop_before_connack {
            recorder.push(Event::Closed);
            return;
        }
        if let Some(delay) = script.connack_delay {
            let until = Instant::now() + delay;
            while Instant::now() < until && !stop.load(Ordering::SeqCst) {
                thread::sleep(POLL);
            }
        }
        if script.silent {
            while !stop.load(Ordering::SeqCst) {
                match self.pump() {
                    Ok(_) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            IoErrorKind::WouldBlock
                                | IoErrorKind::TimedOut
                                | IoErrorKind::Interrupted
                        ) => {}
                    Err(_) => break,
                }
            }
            recorder.push(Event::Closed);
            return;
        }

        let reason = script.connack_reason.unwrap_or(0);
        if self.send(&[0x20, 0x03, 0x00, reason, 0x00]).is_err() {
            recorder.push(Event::Closed);
            return;
        }

        // Wait for the first SUBSCRIBE so a scripted fault lands after the
        // session is established.
        let deadline = Instant::now() + READY;
        loop {
            let Some(frame) = self.next_frame(deadline) else {
                recorder.push(Event::Closed);
                return;
            };
            match decode_client(&frame) {
                Some(ClientPacket::Subscribe {
                    packet_id,
                    filters,
                    retain_handling,
                }) => {
                    recorder.push(Event::Subscribe {
                        filters: filters
                            .iter()
                            .map(|(topic, qos)| (topic.clone(), *qos))
                            .collect(),
                        retain_handling,
                    });
                    let mut suback = vec![0x90];
                    suback.push(u8::try_from(filters.len() + 3).unwrap_or(u8::MAX));
                    suback.extend_from_slice(&packet_id.to_be_bytes());
                    suback.push(0x00);
                    suback.extend(std::iter::repeat_n(0x00, filters.len()));
                    if self.send(&suback).is_err() {
                        recorder.push(Event::Closed);
                        return;
                    }
                    break;
                }
                Some(ClientPacket::PingReq) => {
                    recorder.push(Event::PingReq);
                    let _ = self.send(&[0xD0, 0x00]);
                }
                Some(other) => self.record(other, recorder),
                None => {}
            }
        }

        for action in script.after_subscribe.iter().filter(|_| first) {
            match action {
                Action::Publish {
                    topic,
                    payload,
                    qos,
                    retain,
                    duplicate,
                } => {
                    let packet_id = if qos.requires_packet_id() {
                        Some(7)
                    } else {
                        None
                    };
                    for _ in 0..if *duplicate { 2 } else { 1 } {
                        if self
                            .send_publish(topic, payload.clone(), *qos, *retain, packet_id)
                            .is_err()
                        {
                            recorder.push(Event::Closed);
                            return;
                        }
                    }
                }
                Action::Sleep(delay) => thread::sleep(*delay),
                Action::Close => {
                    recorder.push(Event::Closed);
                    return;
                }
                Action::Malformed => {
                    // Reserved packet type 15 is never valid MQTT 5.0.
                    if self.send(&[0xF0, 0x00]).is_err() {
                        recorder.push(Event::Closed);
                        return;
                    }
                }
                Action::Oversized => {
                    let oversized = vec![0x5A; 320 * 1024];
                    if self
                        .send_publish("line/oversized", oversized, QoS::AtMostOnce, false, None)
                        .is_err()
                    {
                        recorder.push(Event::Closed);
                        return;
                    }
                }
            }
        }

        let mut next_publish = script
            .publish_every
            .as_ref()
            .map(|(_, interval)| Instant::now() + *interval);
        let mut value = 0_u16;
        loop {
            if stop.load(Ordering::SeqCst) {
                recorder.push(Event::Closed);
                return;
            }
            if let Some((topic, interval)) = &script.publish_every
                && next_publish.is_some_and(|due| Instant::now() >= due)
            {
                if self
                    .send_publish(
                        topic,
                        value.to_be_bytes().to_vec(),
                        QoS::AtMostOnce,
                        false,
                        None,
                    )
                    .is_err()
                {
                    recorder.push(Event::Closed);
                    return;
                }
                value = value.wrapping_add(1);
                next_publish = Some(Instant::now() + *interval);
            }
            match self.pump() {
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        IoErrorKind::WouldBlock | IoErrorKind::TimedOut | IoErrorKind::Interrupted
                    ) => {}
                Err(_) => {
                    recorder.push(Event::Closed);
                    return;
                }
            }
            while let Some(frame) = self.take_frame() {
                let Some(packet) = decode_client(&frame) else {
                    continue;
                };
                let disconnect = matches!(packet, ClientPacket::Disconnect { .. });
                let acknowledged = match &packet {
                    ClientPacket::Publish { qos, packet_id, .. } if *qos == 1 => *packet_id,
                    _ => None,
                };
                if let Some(packet_id) = acknowledged {
                    let [high, low] = packet_id.to_be_bytes();
                    let _ = self.send(&[0x40, 0x02, high, low]);
                }
                self.record(packet, recorder);
                if disconnect {
                    return;
                }
            }
        }
    }

    fn record(&mut self, packet: ClientPacket, recorder: &Recorder) {
        let event = match packet {
            ClientPacket::Connect { .. } => return,
            ClientPacket::Subscribe {
                filters,
                retain_handling,
                ..
            } => Event::Subscribe {
                filters,
                retain_handling,
            },
            ClientPacket::Publish {
                topic,
                payload,
                qos,
                retain,
                ..
            } => Event::Publish {
                topic,
                payload,
                qos,
                retain,
            },
            ClientPacket::PubAck(packet_id) => Event::PubAck(packet_id),
            ClientPacket::PubRel(packet_id) => Event::PubRel(packet_id),
            ClientPacket::PubRec(_) | ClientPacket::PubComp(_) => return,
            ClientPacket::PingReq => Event::PingReq,
            ClientPacket::Disconnect { reason } => Event::Disconnect { reason },
        };
        recorder.push(event);
    }
}

/// Reads a big-endian `u16` from `bytes` at `index`.
fn u16_at(bytes: &[u8], index: usize) -> Option<u16> {
    Some(u16::from_be_bytes([
        *bytes.get(index)?,
        *bytes.get(index + 1)?,
    ]))
}

/// Skips an MQTT variable-byte integer and returns `(value, next index)`.
fn varint(bytes: &[u8], mut index: usize) -> Option<(usize, usize)> {
    let mut value = 0_usize;
    let mut multiplier = 1_usize;
    loop {
        let byte = *bytes.get(index)?;
        index += 1;
        value += usize::from(byte & 0x7f) * multiplier;
        if byte & 0x80 == 0 {
            return Some((value, index));
        }
        if index > 4 {
            return None;
        }
        multiplier *= 128;
    }
}

/// Skips an MQTT property block at `index`, returning the next index.
fn skip_properties(bytes: &[u8], index: usize) -> Option<(usize, usize)> {
    let (length, start) = varint(bytes, index)?;
    let end = start.checked_add(length)?;
    if end > bytes.len() {
        return None;
    }
    Some((start, end))
}

/// Reads an MQTT UTF-8 string and returns `(text, next index)`.
fn string_at(bytes: &[u8], index: usize) -> Option<(String, usize)> {
    let length = usize::from(u16_at(bytes, index)?);
    let start = index + 2;
    let end = start.checked_add(length)?;
    let text = std::str::from_utf8(bytes.get(start..end)?).ok()?;
    Some((text.to_owned(), end))
}

/// Reads an MQTT binary field and returns `(bytes, next index)`.
fn binary_at(bytes: &[u8], index: usize) -> Option<(Vec<u8>, usize)> {
    let length = usize::from(u16_at(bytes, index)?);
    let start = index + 2;
    let end = start.checked_add(length)?;
    Some((bytes.get(start..end)?.to_vec(), end))
}

/// Finds the CONNECT session-expiry property (0x11) in a property block.
fn session_expiry(bytes: &[u8], start: usize, end: usize) -> u32 {
    let mut index = start;
    while index < end {
        let identifier = bytes[index];
        index += 1;
        match identifier {
            0x11 => {
                if end - index < 4 {
                    return 0;
                }
                return u32::from_be_bytes([
                    bytes[index],
                    bytes[index + 1],
                    bytes[index + 2],
                    bytes[index + 3],
                ]);
            }
            0x21 | 0x22 | 0x13 | 0x1F | 0x17 | 0x19 | 0x24 => index += 2,
            0x27 | 0x12 | 0x15 | 0x16 | 0x18 | 0x1A | 0x1C | 0x1D | 0x1E => index += 4,
            _ => return 0,
        }
    }
    0
}

/// Decodes one client packet. Returns `None` for an unsupported or malformed
/// frame, which the simulator ignores.
fn decode_client(frame: &[u8]) -> Option<ClientPacket> {
    let header = *frame.first()?;
    let packet_type = header >> 4;
    let flags = header & 0x0f;
    let mut index = 1;
    let (_, length_bytes) = varint(frame, index)?;
    index = length_bytes;
    match packet_type {
        1 => {
            let (_protocol, next) = string_at(frame, index)?;
            index = next;
            let _level = *frame.get(index)?;
            index += 1;
            let connect_flags = *frame.get(index)?;
            index += 1;
            let keep_alive = u16_at(frame, index)?;
            index += 2;
            let (start, end) = skip_properties(frame, index)?;
            let session_expiry = session_expiry(frame, start, end);
            index = end;
            let (client_id, next) = string_at(frame, index)?;
            index = next;
            let mut will = None;
            if connect_flags & 0x04 != 0 {
                let (start, end) = skip_properties(frame, index)?;
                let _ = (start, end);
                index = end;
                let (topic, next) = string_at(frame, index)?;
                index = next;
                let (payload, next) = binary_at(frame, index)?;
                index = next;
                will = Some((topic, payload));
            }
            let username = if connect_flags & 0x80 != 0 {
                let (username, next) = string_at(frame, index)?;
                index = next;
                Some(username)
            } else {
                None
            };
            let password = if connect_flags & 0x40 != 0 {
                let (password, next) = binary_at(frame, index)?;
                index = next;
                Some(password)
            } else {
                None
            };
            let _ = index;
            Some(ClientPacket::Connect {
                client_id,
                clean_start: connect_flags & 0x02 != 0,
                keep_alive,
                session_expiry,
                username,
                password,
                will,
            })
        }
        3 => {
            let qos = (flags >> 1) & 0x03;
            let retain = flags & 0x01 != 0;
            let (topic, next) = string_at(frame, index)?;
            index = next;
            let packet_id = if qos > 0 {
                let id = u16_at(frame, index)?;
                index += 2;
                Some(id)
            } else {
                None
            };
            let (_, end) = skip_properties(frame, index)?;
            let payload = frame.get(end..)?.to_vec();
            Some(ClientPacket::Publish {
                topic,
                payload,
                qos,
                retain,
                packet_id,
            })
        }
        4..=7 => {
            let packet_id = u16_at(frame, index)?;
            Some(match packet_type {
                4 => ClientPacket::PubAck(packet_id),
                5 => ClientPacket::PubRec(packet_id),
                6 => ClientPacket::PubRel(packet_id),
                _ => ClientPacket::PubComp(packet_id),
            })
        }
        8 => {
            let packet_id = u16_at(frame, index)?;
            index += 2;
            let (_, end) = skip_properties(frame, index)?;
            index = end;
            let mut filters = Vec::new();
            let mut retain_handling = 0;
            while index < frame.len() {
                let (filter, next) = string_at(frame, index)?;
                let options = *frame.get(next)?;
                index = next + 1;
                filters.push((filter, options & 0x03));
                retain_handling = (options >> 4) & 0x03;
            }
            Some(ClientPacket::Subscribe {
                packet_id,
                filters,
                retain_handling,
            })
        }
        12 => Some(ClientPacket::PingReq),
        14 => Some(ClientPacket::Disconnect {
            reason: frame.get(index).copied().unwrap_or(0),
        }),
        _ => None,
    }
}

/// Returns a monotonically increasing identifier for test topics.
pub fn unique_topic(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let value = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{prefix}/{value}")
}
