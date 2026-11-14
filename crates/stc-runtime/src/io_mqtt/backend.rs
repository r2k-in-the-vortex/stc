//! MQTT `DeviceBackend` and socket session threads (WL-15 phase 3).
//!
//! The backend owns one [`Bank`] per `mqtt` data device. The session thread
//! owns the TCP socket, DNS resolution, connect/backoff, packet codec, keepalive,
//! and acknowledgements. The coordinator cycle thread only copies image values
//! into and out of the bank under one short mutex per device; it never resolves,
//! connects, encodes, decodes, or allocates per message.
//!
//! # Phase 3 boundary
//!
//! A device plan that requests TLS or credential files fails cold start closed
//! with [`codes::MQTT_TLS_MATERIAL`] or [`codes::MQTT_CREDENTIAL_MATERIAL`].
//! Phase 4 completes those transports. Plaintext is never used when TLS or
//! authentication was requested, and no file content is read or logged here.
//!
//! # Status codes
//!
//! `client<N>.state` uses these stable values:
//!
//! | Value | State |
//! | --- | --- |
//! | 0 | idle |
//! | 1 | connecting |
//! | 2 | connected |
//! | 3 | backoff |
//! | 4 | disconnecting |
//! | 5 | closed |
//!
//! `client<N>.last_error` uses the numeric fault codes in [`fault`]:
//! none, resolve, connect, connect timeout, CONNACK rejected, transport closed,
//! transport I/O, protocol, oversize, keepalive, broker DISCONNECT, payload
//! decode, payload encode, and packet-id exhausted. `client<N>.reason` is the
//! last MQTT reason byte seen from the broker (0 when none). Counters saturate;
//! they never wrap.
//!
//! Every connection failure, transport loss, malformed or oversized packet,
//! protocol error, keepalive timeout, and broker DISCONNECT is a non-fatal
//! per-device fault: the client enters the configured backoff ladder, retries
//! forever, and the scan and other backends keep running. A second `cold_start`
//! stops the previous sessions before it validates the new configuration.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{ErrorKind as IoErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use stc_got::error::Error;

use crate::coordinator::{
    BackendConfig, BackendState, BackendStatus, DeviceBackend, ImageRange, ProcessImage,
};

use super::config::{
    self, Direction, MqttBackendPlan, MqttDevicePlan, StatusBinding, StatusField, StatusIndex,
    codes,
};
use super::packet::{
    self, Connect, Packet, Properties, Publish, QoS, Subscribe, Subscription, Will,
};
use super::payload::{self, Encoding};
use super::session::{Session, SessionAction, SessionState};
use super::topic;

/// Numeric fault codes reported in `client<N>.last_error`.
///
/// These are diagnostic field values, not `STC-*` diagnostics; the stable
/// `STC-*` codes remain in [`config::codes`].
pub mod fault {
    /// No fault recorded.
    pub const NONE: u32 = 0;
    /// The broker host name could not be resolved.
    pub const RESOLVE: u32 = 1;
    /// The TCP connection was refused or otherwise failed.
    pub const CONNECT: u32 = 2;
    /// The TCP connection attempt timed out.
    pub const CONNECT_TIMEOUT: u32 = 3;
    /// The broker rejected CONNECT with a non-zero reason code.
    pub const CONNACK_REJECTED: u32 = 4;
    /// The broker closed the transport.
    pub const TRANSPORT_CLOSED: u32 = 5;
    /// A socket read or write failed.
    pub const TRANSPORT_IO: u32 = 6;
    /// The broker sent a malformed, unexpected, or illegal packet.
    pub const PROTOCOL: u32 = 7;
    /// An inbound packet exceeded the configured maximum size.
    pub const OVERSIZE: u32 = 8;
    /// The keepalive response deadline elapsed.
    pub const KEEPALIVE: u32 = 9;
    /// The broker sent DISCONNECT.
    pub const BROKER_DISCONNECT: u32 = 10;
    /// An inbound payload did not decode for its channel.
    pub const PAYLOAD_DECODE: u32 = 11;
    /// An outbound value did not encode for its channel.
    pub const PAYLOAD_ENCODE: u32 = 12;
    /// No outbound packet identifier was available.
    pub const PACKET_ID_EXHAUSTED: u32 = 13;
}

/// Poll interval for socket reads and stop/backoff checks.
const SESSION_POLL: Duration = Duration::from_millis(25);

/// Bound on one inbound MQTT packet and on the stream accumulation buffer.
const MAX_PACKET_SIZE: usize = 256 * 1024;

/// Receive chunk; the accumulation buffer is bounded by the packet limit.
const READ_CHUNK: usize = 8 * 1024;

/// Maximum unacknowledged QoS 1/2 messages this client accepts.
const RECEIVE_MAXIMUM: u16 = u16::MAX;

/// Maps a session state to its stable numeric status value.
fn state_code(state: SessionState) -> u8 {
    match state {
        SessionState::Idle => 0,
        SessionState::Connecting => 1,
        SessionState::Connected => 2,
        SessionState::Backoff => 3,
        SessionState::Disconnecting => 4,
        SessionState::Closed => 5,
    }
}

/// Returns the diagnostic spelling of a numeric session state.
fn state_name(code: u8) -> &'static str {
    match code {
        1 => "connecting",
        2 => "connected",
        3 => "backoff",
        4 => "disconnecting",
        5 => "closed",
        _ => "idle",
    }
}

/// Returns whether an input value is stale at `now`.
///
/// A channel that never received a value is stale. A received value becomes
/// stale only after the configured timeout; the value itself is retained.
fn is_stale(now: Instant, last_received: Option<Instant>, timeout: Duration) -> bool {
    last_received.is_none_or(|last| now.saturating_duration_since(last) > timeout)
}

/// Adds one, saturating at `u64::MAX`.
fn saturating_increment(counter: &AtomicU64) {
    let _ = counter.try_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
        Some(value.saturating_add(1))
    });
}

fn lock_mutex<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(codes::MQTT_INVALID_CONFIG, message)
}

fn channel_resolve(message: impl Into<String>) -> Error {
    Error::new(codes::MQTT_CHANNEL_RESOLVE, message)
}

/// Instance-wide counters shared with the session threads.
#[derive(Default)]
struct CycleCounters {
    exchanges: AtomicU64,
    faults: AtomicU64,
}

/// Atomic per-client state and counters.
struct DeviceRuntime {
    connected: AtomicBool,
    state: AtomicU8,
    faulted: AtomicBool,
    reconnects: AtomicU64,
    messages_in: AtomicU64,
    messages_out: AtomicU64,
    dropped: AtomicU64,
    last_error: AtomicU32,
    reason: AtomicU32,
    last_fault: Mutex<Option<String>>,
}

impl DeviceRuntime {
    fn new() -> Self {
        Self {
            connected: AtomicBool::new(false),
            state: AtomicU8::new(state_code(SessionState::Idle)),
            faulted: AtomicBool::new(false),
            reconnects: AtomicU64::new(0),
            messages_in: AtomicU64::new(0),
            messages_out: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            last_error: AtomicU32::new(fault::NONE),
            reason: AtomicU32::new(0),
            last_fault: Mutex::new(None),
        }
    }

    fn set_state(&self, state: SessionState) {
        self.state.store(state_code(state), Ordering::SeqCst);
    }

    fn state(&self) -> u8 {
        self.state.load(Ordering::SeqCst)
    }

    fn connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn faulted(&self) -> bool {
        self.faulted.load(Ordering::SeqCst)
    }

    fn last_fault(&self) -> Option<String> {
        lock_mutex(&self.last_fault).clone()
    }

    fn note_fault(&self, code: u32, reason: u8, message: String) {
        self.connected.store(false, Ordering::SeqCst);
        self.faulted.store(true, Ordering::SeqCst);
        self.last_error.store(code, Ordering::SeqCst);
        self.reason.store(u32::from(reason), Ordering::SeqCst);
        *lock_mutex(&self.last_fault) = Some(message);
    }

    fn note_error(&self, code: u32, message: String) {
        self.last_error.store(code, Ordering::SeqCst);
        *lock_mutex(&self.last_fault) = Some(message);
    }
}

#[derive(Clone, Copy)]
struct InputSlot {
    value: u64,
    last_received: Option<Instant>,
}

#[derive(Clone, Copy, Default)]
struct OutputSlot {
    value: u64,
    dirty: bool,
}

/// The per-device exchange bank.
///
/// Input slots hold the last accepted value and receipt stamp. Output slots
/// hold the last `%Q` value and a dirty flag. Pending packet identifiers map a
/// QoS 1/2 publish to its output slot so the acknowledgement can clear it.
struct Bank {
    inputs: Vec<InputSlot>,
    outputs: Vec<OutputSlot>,
    pending: BTreeMap<u16, usize>,
}

impl Bank {
    fn new(inputs: usize, outputs: usize) -> Self {
        Self {
            inputs: vec![
                InputSlot {
                    value: 0,
                    last_received: None,
                };
                inputs
            ],
            outputs: vec![OutputSlot::default(); outputs],
            pending: BTreeMap::new(),
        }
    }

    fn store_input(&mut self, slot: usize, value: u64, now: Instant) {
        if let Some(input) = self.inputs.get_mut(slot) {
            input.value = value;
            input.last_received = Some(now);
        }
    }

    /// Copies a `%Q` value, marking the output dirty only when it changed.
    fn store_output(&mut self, slot: usize, value: u64) {
        if let Some(output) = self.outputs.get_mut(slot)
            && output.value != value
        {
            output.value = value;
            output.dirty = true;
        }
    }

    /// Takes every dirty output, clearing the flag. The returned pairs are
    /// `(slot, value)` in slot order.
    fn take_dirty(&mut self) -> Vec<(usize, u64)> {
        let mut dirty = Vec::new();
        for (slot, output) in self.outputs.iter_mut().enumerate() {
            if output.dirty {
                output.dirty = false;
                dirty.push((slot, output.value));
            }
        }
        dirty
    }

    fn restore_dirty(&mut self, slots: &[(usize, u64)]) {
        for (slot, _) in slots {
            if let Some(output) = self.outputs.get_mut(*slot) {
                output.dirty = true;
            }
        }
    }

    /// Marks every output dirty and returns the packet identifiers that were
    /// still pending, so the session can release them.
    fn reset_for_connection(&mut self) -> Vec<u16> {
        for output in &mut self.outputs {
            output.dirty = true;
        }
        std::mem::take(&mut self.pending).into_keys().collect()
    }

    fn take_pending(&mut self, packet_id: u16) -> Option<usize> {
        self.pending.remove(&packet_id)
    }
}

/// One bound input channel: bank slot plus plan channel index.
#[derive(Clone, Copy)]
struct BoundInput {
    slot: usize,
    channel: usize,
}

/// One bound output channel: bank slot plus plan channel index.
#[derive(Clone, Copy)]
struct BoundOutput {
    slot: usize,
    channel: usize,
}

fn bind_channels(plan: &MqttDevicePlan) -> (Vec<BoundInput>, Vec<BoundOutput>) {
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut input_slot = 0;
    let mut output_slot = 0;
    for (channel, entry) in plan.channels.iter().enumerate() {
        if entry.image.is_none() {
            continue;
        }
        match entry.direction {
            Direction::Input => {
                inputs.push(BoundInput {
                    slot: input_slot,
                    channel,
                });
                input_slot += 1;
            }
            Direction::Output => {
                outputs.push(BoundOutput {
                    slot: output_slot,
                    channel,
                });
                output_slot += 1;
            }
        }
    }
    (inputs, outputs)
}

struct Device {
    plan: Arc<MqttDevicePlan>,
    bank: Arc<Mutex<Bank>>,
    runtime: Arc<DeviceRuntime>,
    inputs: Arc<Vec<BoundInput>>,
    outputs: Arc<Vec<BoundOutput>>,
    thread: Option<JoinHandle<()>>,
}

impl Device {
    fn image_range(&self, channel: usize) -> Option<ImageRange> {
        self.plan
            .channels
            .get(channel)
            .and_then(|entry| entry.image)
    }

    fn input_is_stale(&self, now: Instant) -> bool {
        let bank = lock_mutex(&self.bank);
        self.inputs.iter().any(|input| {
            bank.inputs
                .get(input.slot)
                .is_some_and(|slot| is_stale(now, slot.last_received, self.plan.stale_timeout))
        })
    }
}

/// The MQTT `DeviceBackend`.
///
/// One instance serves every `mqtt` data device and the optional
/// `mqtt-status` device. Each data device owns a bank and one session thread.
/// The cycle thread only copies the bank at the two scan boundaries.
pub struct MqttBackend {
    name: String,
    devices: Vec<Device>,
    status_bindings: Vec<StatusBinding>,
    counters: Arc<CycleCounters>,
    stop: Arc<AtomicBool>,
    started: bool,
    cycled: bool,
    device_count: usize,
}

impl Default for MqttBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MqttBackend {
    /// Creates an unstarted backend.
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: String::new(),
            devices: Vec::new(),
            status_bindings: Vec::new(),
            counters: Arc::new(CycleCounters::default()),
            stop: Arc::new(AtomicBool::new(true)),
            started: false,
            cycled: false,
            device_count: 0,
        }
    }

    /// Returns the number of declared MQTT channels without an image binding.
    #[must_use]
    pub fn unbound_channels(&self) -> usize {
        self.devices
            .iter()
            .map(|device| device.plan.unbound.len())
            .sum()
    }

    fn compute_state(&self) -> BackendState {
        if !self.started {
            return BackendState::Stopped;
        }
        if self.devices.iter().any(|device| device.runtime.faulted()) {
            return BackendState::Faulted;
        }
        if self.cycled {
            BackendState::Running
        } else {
            BackendState::Ready
        }
    }

    fn last_fault(&self) -> Option<String> {
        self.devices
            .iter()
            .find_map(|device| device.runtime.last_fault())
    }

    fn status_value(&self, binding: &StatusBinding, now: Instant) -> u64 {
        match binding.index {
            StatusIndex::Client(index) => {
                let Some(device) = self.devices.get(index) else {
                    return 0;
                };
                let runtime = &device.runtime;
                match binding.field {
                    StatusField::Connected => u64::from(runtime.connected()),
                    StatusField::State => u64::from(runtime.state()),
                    StatusField::Reconnects => runtime.reconnects.load(Ordering::SeqCst),
                    StatusField::MessagesIn => runtime.messages_in.load(Ordering::SeqCst),
                    StatusField::MessagesOut => runtime.messages_out.load(Ordering::SeqCst),
                    StatusField::Dropped => runtime.dropped.load(Ordering::SeqCst),
                    StatusField::LastError => u64::from(runtime.last_error.load(Ordering::SeqCst)),
                    StatusField::Reason => u64::from(runtime.reason.load(Ordering::SeqCst)),
                    StatusField::Stale | StatusField::Exchanges | StatusField::Faults => 0,
                }
            }
            StatusIndex::Global => match binding.field {
                StatusField::Stale => {
                    u64::from(self.devices.iter().any(|device| device.input_is_stale(now)))
                }
                StatusField::Exchanges => self.counters.exchanges.load(Ordering::SeqCst),
                StatusField::Faults => self.counters.faults.load(Ordering::SeqCst),
                StatusField::Connected
                | StatusField::State
                | StatusField::Reconnects
                | StatusField::MessagesIn
                | StatusField::MessagesOut
                | StatusField::Dropped
                | StatusField::LastError
                | StatusField::Reason => 0,
            },
        }
    }

    fn write_status_bindings(
        &self,
        image: &mut ProcessImage,
        now: Instant,
    ) -> std::result::Result<(), Error> {
        for binding in &self.status_bindings {
            let Some(range) = binding.image else {
                continue;
            };
            let value = self.status_value(binding, now);
            if !image.write_bits(range.area, range.offset_bits, range.width_bits, value) {
                return Err(channel_resolve(format!(
                    "status channel `{}` on device `{}` leaves the process image",
                    binding.field.as_str(),
                    binding.device
                )));
            }
        }
        Ok(())
    }
}

impl Drop for MqttBackend {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

impl DeviceBackend for MqttBackend {
    fn cold_start(&mut self, config: &BackendConfig) -> std::result::Result<(), Error> {
        let _ = self.shutdown();
        self.devices.clear();
        self.status_bindings.clear();
        self.started = false;
        self.cycled = false;
        self.device_count = 0;
        self.counters = Arc::new(CycleCounters::default());

        // Phase 3 resolves TLS and credential paths against the current
        // directory. Phase 4 threads the project/manifest root through here.
        let base_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let MqttBackendPlan {
            devices: plans,
            status: status_bindings,
        } = config::plan(config, &base_dir)?;

        // Phase 3 boundary: a requested transport that this phase cannot
        // provide must fail closed, never fall back to plaintext.
        for plan in &plans {
            if plan.tls.is_some() {
                return Err(Error::new(
                    codes::MQTT_TLS_MATERIAL,
                    format!(
                        "MQTT device `{}` requests TLS; TLS transport is completed in Phase 4, \
                         so the device was not started in plaintext",
                        plan.name
                    ),
                ));
            }
            if plan.credentials.is_some() {
                return Err(Error::new(
                    codes::MQTT_CREDENTIAL_MATERIAL,
                    format!(
                        "MQTT device `{}` requests credentials; credential transport is completed \
                         in Phase 4, so the device was not started without authentication",
                        plan.name
                    ),
                ));
            }
        }

        let stop = Arc::new(AtomicBool::new(false));
        let counters = Arc::new(CycleCounters::default());
        let mut devices: Vec<Device> = Vec::with_capacity(plans.len());
        for plan in plans {
            let plan = Arc::new(plan);
            let (bound_inputs, bound_outputs) = bind_channels(&plan);
            let inputs = Arc::new(bound_inputs);
            let outputs = Arc::new(bound_outputs);
            let bank = Arc::new(Mutex::new(Bank::new(inputs.len(), outputs.len())));
            let runtime = Arc::new(DeviceRuntime::new());
            let runner = SessionRunner {
                plan: Arc::clone(&plan),
                inputs: Arc::clone(&inputs),
                outputs: Arc::clone(&outputs),
                bank: Arc::clone(&bank),
                runtime: Arc::clone(&runtime),
                counters: Arc::clone(&counters),
                stop: Arc::clone(&stop),
            };
            let thread_name = format!("stc-mqtt-{}", plan.name);
            match std::thread::Builder::new()
                .name(thread_name)
                .spawn(move || runner.run())
            {
                Ok(thread) => devices.push(Device {
                    plan,
                    bank,
                    runtime,
                    inputs,
                    outputs,
                    thread: Some(thread),
                }),
                Err(error) => {
                    stop.store(true, Ordering::SeqCst);
                    for device in &mut devices {
                        if let Some(thread) = device.thread.take() {
                            let _ = thread.join();
                        }
                    }
                    return Err(invalid(format!(
                        "MQTT backend `{}` cannot start a session thread: {error}",
                        config.name
                    )));
                }
            }
        }

        self.status_bindings = status_bindings;
        self.devices = devices;
        self.name = config.name.clone();
        self.device_count = config.devices.len();
        self.counters = counters;
        self.stop = stop;
        self.started = true;
        Ok(())
    }

    fn latch_inputs(&mut self, image: &mut ProcessImage) -> std::result::Result<(), Error> {
        for device in &self.devices {
            let bank = lock_mutex(&device.bank);
            for input in device.inputs.iter() {
                let Some(range) = device.image_range(input.channel) else {
                    continue;
                };
                let value = bank.inputs.get(input.slot).map_or(0, |slot| slot.value);
                if !image.write_bits(range.area, range.offset_bits, range.width_bits, value) {
                    return Err(channel_resolve(format!(
                        "input channel `{}` on device `{}` leaves the process image",
                        device.plan.channels[input.channel].name, device.plan.name
                    )));
                }
            }
        }
        let now = Instant::now();
        self.write_status_bindings(image, now)?;
        self.cycled = true;
        Ok(())
    }

    fn publish_outputs(&mut self, image: &ProcessImage) -> std::result::Result<(), Error> {
        for device in &self.devices {
            let mut bank = lock_mutex(&device.bank);
            for output in device.outputs.iter() {
                let Some(range) = device.image_range(output.channel) else {
                    continue;
                };
                let value = image
                    .read_bits(range.area, range.offset_bits, range.width_bits)
                    .ok_or_else(|| {
                        channel_resolve(format!(
                            "output channel `{}` on device `{}` leaves the process image",
                            device.plan.channels[output.channel].name, device.plan.name
                        ))
                    })?;
                bank.store_output(output.slot, value);
            }
        }
        saturating_increment(&self.counters.exchanges);
        // `publish_outputs` receives an immutable image and status bindings are
        // `%I` ranges, so the incremented cycle counters become visible at the
        // next `latch_inputs`, which writes the cycle.* bindings.
        self.cycled = true;
        Ok(())
    }

    fn status(&self) -> BackendStatus {
        BackendStatus {
            name: self.name.clone(),
            devices: self.device_count,
            state: self.compute_state(),
            exchanges: self.counters.exchanges.load(Ordering::SeqCst),
            faults: self.counters.faults.load(Ordering::SeqCst),
            last_fault: self.last_fault(),
        }
    }

    fn diagnostics(&self) -> BTreeMap<String, String> {
        let now = Instant::now();
        let mut values = BTreeMap::new();
        for (index, device) in self.devices.iter().enumerate() {
            let prefix = format!("client{index}");
            let runtime = &device.runtime;
            values.insert(format!("{prefix}.host"), device.plan.host.clone());
            values.insert(format!("{prefix}.port"), device.plan.port.to_string());
            values.insert(
                format!("{prefix}.state"),
                state_name(runtime.state()).to_owned(),
            );
            values.insert(
                format!("{prefix}.connected"),
                runtime.connected().to_string(),
            );
            values.insert(
                format!("{prefix}.reconnects"),
                runtime.reconnects.load(Ordering::SeqCst).to_string(),
            );
            values.insert(
                format!("{prefix}.messages_in"),
                runtime.messages_in.load(Ordering::SeqCst).to_string(),
            );
            values.insert(
                format!("{prefix}.messages_out"),
                runtime.messages_out.load(Ordering::SeqCst).to_string(),
            );
            values.insert(
                format!("{prefix}.dropped"),
                runtime.dropped.load(Ordering::SeqCst).to_string(),
            );
            values.insert(
                format!("{prefix}.last_error"),
                runtime.last_error.load(Ordering::SeqCst).to_string(),
            );
            values.insert(
                format!("{prefix}.reason"),
                runtime.reason.load(Ordering::SeqCst).to_string(),
            );
            values.insert(
                format!("{prefix}.last_fault"),
                runtime.last_fault().unwrap_or_default(),
            );
        }
        values.insert(
            "input.stale".to_owned(),
            self.devices
                .iter()
                .any(|device| device.input_is_stale(now))
                .to_string(),
        );
        values.insert(
            "cycle.exchanges".to_owned(),
            self.counters.exchanges.load(Ordering::SeqCst).to_string(),
        );
        values.insert(
            "cycle.faults".to_owned(),
            self.counters.faults.load(Ordering::SeqCst).to_string(),
        );
        values
    }

    fn shutdown(&mut self) -> std::result::Result<(), Error> {
        self.stop.store(true, Ordering::SeqCst);
        for device in &mut self.devices {
            if let Some(thread) = device.thread.take() {
                let _ = thread.join();
            }
            device.runtime.connected.store(false, Ordering::SeqCst);
            device.runtime.set_state(SessionState::Closed);
        }
        self.started = false;
        Ok(())
    }
}

/// One broker session thread.
struct SessionRunner {
    plan: Arc<MqttDevicePlan>,
    inputs: Arc<Vec<BoundInput>>,
    outputs: Arc<Vec<BoundOutput>>,
    bank: Arc<Mutex<Bank>>,
    runtime: Arc<DeviceRuntime>,
    counters: Arc<CycleCounters>,
    stop: Arc<AtomicBool>,
}

impl SessionRunner {
    fn run(self) {
        let mut session = Session::new(self.plan.session.clone());
        let started = Instant::now();
        let mut queue: VecDeque<SessionAction> = session.start(Duration::ZERO).into();
        let mut stream: Option<TcpStream> = None;
        let mut read_buf: Vec<u8> = Vec::with_capacity(READ_CHUNK);
        let mut chunk = [0_u8; READ_CHUNK];
        let mut attempts: u64 = 0;
        let mut subscribe_id: Option<u16> = None;
        let mut inbound_applied: BTreeSet<u16> = BTreeSet::new();

        'session: loop {
            let now = started.elapsed();
            if self.stop.load(Ordering::SeqCst) {
                self.shutdown_session(&mut session, &mut stream, now);
                break;
            }

            let mut closed = false;
            while let Some(action) = queue.pop_front() {
                match action {
                    SessionAction::StartConnect => {
                        attempts = attempts.saturating_add(1);
                        if attempts > 1 {
                            saturating_increment(&self.runtime.reconnects);
                        }
                        self.runtime.set_state(SessionState::Connecting);
                        self.runtime.connected.store(false, Ordering::SeqCst);
                        read_buf.clear();
                        stream = None;
                        let connect = match build_connect(&self.plan) {
                            Ok(bytes) => bytes,
                            Err(error) => {
                                self.record_fault(
                                    fault::PROTOCOL,
                                    0,
                                    format!("cannot encode CONNECT: {error}"),
                                );
                                queue.extend(session.on_transport_closed(started.elapsed()));
                                continue;
                            }
                        };
                        match open_connection(&self.plan) {
                            Ok(mut connected) => {
                                if let Err(error) = connected.write_all(&connect) {
                                    self.record_fault(
                                        fault::TRANSPORT_IO,
                                        0,
                                        format!("socket write failed while connecting: {error}"),
                                    );
                                    queue.extend(session.on_transport_closed(started.elapsed()));
                                } else {
                                    session.note_sent(started.elapsed());
                                    stream = Some(connected);
                                }
                            }
                            Err((code, message)) => {
                                self.record_fault(code, 0, message);
                                queue.extend(session.on_transport_closed(started.elapsed()));
                            }
                        }
                    }
                    SessionAction::Send(packet) => {
                        let Some(connected) = stream.as_mut() else {
                            continue;
                        };
                        match packet.encode() {
                            Ok(bytes) => {
                                if let Err(error) = connected.write_all(&bytes) {
                                    self.record_fault(
                                        fault::TRANSPORT_IO,
                                        0,
                                        format!("socket write failed: {error}"),
                                    );
                                    queue.extend(session.on_transport_closed(started.elapsed()));
                                } else {
                                    session.note_sent(started.elapsed());
                                }
                            }
                            Err(error) => {
                                self.record_fault(
                                    fault::PROTOCOL,
                                    0,
                                    format!("cannot encode a session packet: {error}"),
                                );
                                queue.extend(session.on_transport_closed(started.elapsed()));
                            }
                        }
                    }
                    SessionAction::AbortConnect => {
                        stream = None;
                        read_buf.clear();
                    }
                    SessionAction::Wait(_) => {}
                    SessionAction::Closed => {
                        closed = true;
                        break;
                    }
                }
            }
            if closed {
                break;
            }
            self.sync_state(&session);

            if let Some(connected) = stream.as_mut() {
                match connected.read(&mut chunk) {
                    Ok(0) => {
                        let now = started.elapsed();
                        if session.state() == SessionState::Disconnecting {
                            queue.extend(session.on_transport_closed(now));
                        } else {
                            self.record_fault(
                                fault::TRANSPORT_CLOSED,
                                0,
                                "broker closed the connection".to_owned(),
                            );
                            queue.extend(session.on_transport_closed(now));
                        }
                        stream = None;
                        read_buf.clear();
                    }
                    Ok(read) => {
                        read_buf.extend_from_slice(&chunk[..read]);
                        let mut drop_transport = false;
                        loop {
                            match packet::decode_packet(&read_buf, MAX_PACKET_SIZE) {
                                Ok((packet, consumed)) => {
                                    read_buf.drain(..consumed);
                                    if self.handle_packet(
                                        &mut session,
                                        packet,
                                        started,
                                        &mut queue,
                                        &mut subscribe_id,
                                        &mut inbound_applied,
                                    ) {
                                        drop_transport = true;
                                        break;
                                    }
                                }
                                Err(error) if error.is_incomplete() => break,
                                Err(error) => {
                                    let code = if matches!(error.kind(), super::ErrorKind::TooLarge)
                                    {
                                        fault::OVERSIZE
                                    } else {
                                        fault::PROTOCOL
                                    };
                                    self.record_fault(
                                        code,
                                        0,
                                        format!("rejected an inbound MQTT packet: {error}"),
                                    );
                                    queue.extend(session.on_transport_closed(started.elapsed()));
                                    drop_transport = true;
                                    break;
                                }
                            }
                        }
                        if drop_transport {
                            stream = None;
                            read_buf.clear();
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            IoErrorKind::WouldBlock | IoErrorKind::TimedOut
                        ) => {}
                    Err(error) if error.kind() == IoErrorKind::Interrupted => {}
                    Err(error) => {
                        self.record_fault(
                            fault::TRANSPORT_IO,
                            0,
                            format!("socket read failed: {error}"),
                        );
                        queue.extend(session.on_transport_closed(started.elapsed()));
                        stream = None;
                        read_buf.clear();
                    }
                }
            }

            let before = session.state();
            let actions = session.on_tick(started.elapsed());
            let after = session.state();
            if before != SessionState::Backoff && after == SessionState::Backoff {
                let (code, message) = match before {
                    SessionState::Connecting => (
                        fault::CONNECT_TIMEOUT,
                        "the CONNECT/CONNACK deadline elapsed".to_owned(),
                    ),
                    SessionState::Connected => (
                        fault::KEEPALIVE,
                        "the keepalive response deadline elapsed".to_owned(),
                    ),
                    _ => (fault::PROTOCOL, "the session entered backoff".to_owned()),
                };
                self.record_fault(code, 0, message);
            }
            queue.extend(actions);
            self.sync_state(&session);

            // Let queued control packets go first: a fresh CONNACK queues the
            // SUBSCRIBE, and outputs must not overtake it.
            if session.state() == SessionState::Connected
                && queue.is_empty()
                && let Some(connected) = stream.as_mut()
                && let Err((code, message)) = self.flush_outputs(connected, &mut session, started)
            {
                self.record_fault(code, 0, message);
                queue.extend(session.on_transport_closed(started.elapsed()));
                stream = None;
                read_buf.clear();
            }

            if stream.is_none() {
                if session.state() == SessionState::Closed {
                    break 'session;
                }
                let now = started.elapsed();
                let wait = session
                    .deadline()
                    .map_or(SESSION_POLL, |deadline| {
                        deadline.saturating_sub(now).min(SESSION_POLL)
                    })
                    .max(Duration::from_millis(1));
                std::thread::sleep(wait);
            }
        }

        self.runtime.connected.store(false, Ordering::SeqCst);
        self.runtime.set_state(SessionState::Closed);
    }

    fn sync_state(&self, session: &Session) {
        self.runtime.set_state(session.state());
        self.runtime
            .connected
            .store(session.state() == SessionState::Connected, Ordering::SeqCst);
    }

    fn record_fault(&self, code: u32, reason: u8, message: String) {
        self.runtime.note_fault(code, reason, message);
        saturating_increment(&self.counters.faults);
    }

    fn shutdown_session(
        &self,
        session: &mut Session,
        stream: &mut Option<TcpStream>,
        now: Duration,
    ) {
        let actions = session.request_disconnect(now);
        for action in actions {
            let SessionAction::Send(packet) = action else {
                continue;
            };
            let Some(connected) = stream.as_mut() else {
                continue;
            };
            if let Ok(bytes) = packet.encode() {
                let _ = connected.write_all(&bytes);
            }
        }
        if let Some(connected) = stream.take() {
            let _ = connected.shutdown(Shutdown::Both);
        }
        self.runtime.connected.store(false, Ordering::SeqCst);
        self.runtime.set_state(SessionState::Closed);
    }

    /// Handles one decoded server packet. Returns `true` when the transport
    /// must be dropped after a protocol or framing error.
    fn handle_packet(
        &self,
        session: &mut Session,
        packet: Packet,
        started: Instant,
        queue: &mut VecDeque<SessionAction>,
        subscribe_id: &mut Option<u16>,
        inbound_applied: &mut BTreeSet<u16>,
    ) -> bool {
        let inbound_publish = match &packet {
            Packet::Publish(publish) => Some(publish.clone()),
            _ => None,
        };
        let connack = match &packet {
            Packet::ConnAck(ack) => Some((ack.is_success(), ack.reason)),
            _ => None,
        };
        let disconnect_reason = match &packet {
            Packet::Disconnect(disconnect) => Some(disconnect.reason),
            _ => None,
        };
        let suback_failure = match &packet {
            Packet::SubAck(ack) => ack.reasons.iter().copied().find(|reason| *reason >= 0x80),
            _ => None,
        };
        let puback_id = match &packet {
            Packet::PubAck(ack) | Packet::PubComp(ack) => Some(ack.packet_id),
            _ => None,
        };
        let pubrel_id = match &packet {
            Packet::PubRel(ack) => Some(ack.packet_id),
            _ => None,
        };
        let suback_id = match &packet {
            Packet::SubAck(ack) => Some(ack.packet_id),
            _ => None,
        };
        if let Some(reason) = packet_reason(&packet) {
            self.runtime
                .reason
                .store(u32::from(reason), Ordering::SeqCst);
        }

        let before = session.state();
        match session.on_packet(started.elapsed(), packet) {
            Ok(actions) => {
                if let Some(publish) = inbound_publish {
                    self.accept_publish(&publish, inbound_applied);
                }
                if let Some(packet_id) = puback_id {
                    let _ = lock_mutex(&self.bank).take_pending(packet_id);
                }
                if let Some(packet_id) = pubrel_id {
                    inbound_applied.remove(&packet_id);
                }
                if let Some(packet_id) = suback_id
                    && *subscribe_id == Some(packet_id)
                {
                    session.release_packet_id(packet_id);
                    *subscribe_id = None;
                }
                if let Some(reason) = suback_failure {
                    self.runtime.note_error(
                        fault::PROTOCOL,
                        format!("broker rejected a subscription with reason 0x{reason:02X}"),
                    );
                }
                if matches!(connack, Some((true, _))) {
                    self.on_connected(session, queue, subscribe_id, inbound_applied);
                }
                if before != SessionState::Backoff && session.state() == SessionState::Backoff {
                    if let Some((false, reason)) = connack {
                        self.record_fault(
                            fault::CONNACK_REJECTED,
                            reason,
                            format!("broker rejected CONNECT with reason 0x{reason:02X}"),
                        );
                    } else if let Some(reason) = disconnect_reason {
                        self.record_fault(
                            fault::BROKER_DISCONNECT,
                            reason,
                            format!("broker sent DISCONNECT with reason 0x{reason:02X}"),
                        );
                    } else {
                        self.record_fault(
                            fault::PROTOCOL,
                            0,
                            "the session entered backoff after a packet".to_owned(),
                        );
                    }
                }
                queue.extend(actions);
                false
            }
            Err(error) => {
                self.record_fault(fault::PROTOCOL, 0, format!("MQTT protocol error: {error}"));
                queue.extend(session.on_transport_closed(started.elapsed()));
                true
            }
        }
    }

    fn on_connected(
        &self,
        session: &mut Session,
        queue: &mut VecDeque<SessionAction>,
        subscribe_id: &mut Option<u16>,
        inbound_applied: &mut BTreeSet<u16>,
    ) {
        let pending = lock_mutex(&self.bank).reset_for_connection();
        for packet_id in pending {
            session.release_packet_id(packet_id);
        }
        for packet_id in std::mem::take(inbound_applied) {
            session.release_packet_id(packet_id);
        }
        if let Some(packet_id) = subscribe_id.take() {
            session.release_packet_id(packet_id);
        }

        self.runtime.connected.store(true, Ordering::SeqCst);
        self.runtime.faulted.store(false, Ordering::SeqCst);
        self.runtime.set_state(SessionState::Connected);
        self.runtime.last_error.store(fault::NONE, Ordering::SeqCst);
        self.runtime.reason.store(0, Ordering::SeqCst);

        let filters: Vec<Subscription> = self
            .inputs
            .iter()
            .map(|input| {
                let channel = &self.plan.channels[input.channel];
                Subscription {
                    filter: channel.topic.clone(),
                    qos: channel.qos,
                    no_local: false,
                    retain_as_published: false,
                    retain_handling: 0,
                }
            })
            .collect();
        if filters.is_empty() {
            return;
        }
        match session.allocate_packet_id() {
            Ok(packet_id) => {
                *subscribe_id = Some(packet_id);
                queue.push_back(SessionAction::Send(Packet::Subscribe(Subscribe {
                    packet_id,
                    properties: Properties::default(),
                    filters,
                })));
            }
            Err(error) => self.runtime.note_error(
                fault::PACKET_ID_EXHAUSTED,
                format!("cannot send SUBSCRIBE: {error}"),
            ),
        }
    }

    fn accept_publish(&self, publish: &Publish, inbound_applied: &mut BTreeSet<u16>) {
        let qos2_id = if publish.qos == QoS::ExactlyOnce {
            publish.packet_id
        } else {
            None
        };
        if qos2_id.is_some_and(|packet_id| inbound_applied.contains(&packet_id)) {
            return;
        }

        let now = Instant::now();
        let mut accepted = 0_u64;
        let mut rejected = 0_u64;
        let mut last_error = None;
        {
            let mut bank = lock_mutex(&self.bank);
            for input in self.inputs.iter() {
                let channel = &self.plan.channels[input.channel];
                if !topic::matches(&channel.topic, &publish.topic) {
                    continue;
                }
                match payload::decode(
                    channel.encoding,
                    channel.width_bits,
                    channel.byte_order,
                    &publish.payload,
                ) {
                    Ok(value) => {
                        bank.store_input(input.slot, value, now);
                        accepted = accepted.saturating_add(1);
                    }
                    Err(error) => {
                        rejected = rejected.saturating_add(1);
                        last_error = Some(format!(
                            "input channel `{}` payload rejected: {error}",
                            channel.name
                        ));
                    }
                }
            }
        }
        if let Some(packet_id) = qos2_id {
            inbound_applied.insert(packet_id);
        }
        if accepted > 0 {
            saturating_increment(&self.runtime.messages_in);
        }
        if rejected > 0 {
            let _ = self
                .runtime
                .dropped
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                    Some(value.saturating_add(rejected))
                });
            if let Some(message) = last_error {
                self.runtime.note_error(fault::PAYLOAD_DECODE, message);
            }
        }
    }

    fn flush_outputs(
        &self,
        stream: &mut TcpStream,
        session: &mut Session,
        started: Instant,
    ) -> std::result::Result<(), (u32, String)> {
        let dirty = lock_mutex(&self.bank).take_dirty();
        for (position, (slot, value)) in dirty.iter().enumerate() {
            let Some(output) = self.outputs.get(*slot) else {
                continue;
            };
            let channel = &self.plan.channels[output.channel];
            let payload = match payload::encode(
                channel.encoding,
                channel.width_bits,
                channel.byte_order,
                *value,
            ) {
                Ok(payload) => payload,
                Err(error) => {
                    saturating_increment(&self.runtime.dropped);
                    self.runtime.note_error(
                        fault::PAYLOAD_ENCODE,
                        format!(
                            "output channel `{}` value cannot be encoded: {error}",
                            channel.name
                        ),
                    );
                    continue;
                }
            };
            let packet_id = if channel.qos.requires_packet_id() {
                match session.allocate_packet_id() {
                    Ok(packet_id) => Some(packet_id),
                    Err(error) => {
                        saturating_increment(&self.runtime.dropped);
                        self.runtime.note_error(
                            fault::PACKET_ID_EXHAUSTED,
                            format!("cannot publish an output: {error}"),
                        );
                        lock_mutex(&self.bank).restore_dirty(&dirty[position..]);
                        break;
                    }
                }
            } else {
                None
            };
            let mut properties = Properties::default();
            if channel.encoding == Encoding::String {
                properties.payload_format_indicator = Some(1);
            }
            let packet = Packet::Publish(Publish {
                topic: channel.topic.clone(),
                payload,
                qos: channel.qos,
                retain: channel.retain,
                dup: false,
                packet_id,
                properties,
            });
            let bytes = match packet.encode() {
                Ok(bytes) => bytes,
                Err(error) => {
                    if let Some(packet_id) = packet_id {
                        session.release_packet_id(packet_id);
                    }
                    saturating_increment(&self.runtime.dropped);
                    self.runtime.note_error(
                        fault::PAYLOAD_ENCODE,
                        format!(
                            "output channel `{}` packet cannot be encoded: {error}",
                            channel.name
                        ),
                    );
                    continue;
                }
            };
            if let Err(error) = stream.write_all(&bytes) {
                if let Some(packet_id) = packet_id {
                    session.release_packet_id(packet_id);
                }
                lock_mutex(&self.bank).restore_dirty(&dirty[position..]);
                return Err((
                    fault::TRANSPORT_IO,
                    format!("socket write failed while publishing: {error}"),
                ));
            }
            session.note_sent(started.elapsed());
            saturating_increment(&self.runtime.messages_out);
            if let Some(packet_id) = packet_id {
                lock_mutex(&self.bank).pending.insert(packet_id, *slot);
            }
        }
        Ok(())
    }
}

/// Builds the CONNECT packet for one device plan.
fn build_connect(plan: &MqttDevicePlan) -> std::result::Result<Vec<u8>, super::MqttError> {
    let keep_alive = u16::try_from(plan.session.keep_alive.as_secs()).unwrap_or(u16::MAX);
    let maximum_packet_size = u32::try_from(MAX_PACKET_SIZE).unwrap_or(u32::MAX);
    Packet::Connect(Connect {
        client_id: plan.client_id.clone().unwrap_or_default(),
        clean_start: plan.clean_start,
        keep_alive,
        session_expiry: plan.session_expiry,
        receive_maximum: RECEIVE_MAXIMUM,
        maximum_packet_size: Some(maximum_packet_size),
        username: None,
        password: None,
        will: plan.last_will.as_ref().map(|will| Will {
            topic: will.topic.clone(),
            payload: will.payload.clone(),
            qos: will.qos,
            retain: will.retain,
            delay_interval: None,
            payload_format_indicator: Some(1),
            content_type: None,
        }),
    })
    .encode()
}

/// Resolves and connects to the broker. Only the session thread calls this.
fn open_connection(plan: &MqttDevicePlan) -> std::result::Result<TcpStream, (u32, String)> {
    let addresses = (plan.host.as_str(), plan.port)
        .to_socket_addrs()
        .map_err(|error| {
            (
                fault::RESOLVE,
                format!("cannot resolve broker host `{}`: {error}", plan.host),
            )
        })?;
    let deadline = Instant::now() + plan.session.connect_timeout;
    let mut last_error = None;
    let mut timed_out = false;
    for address in addresses {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            timed_out = true;
            break;
        }
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(stream) => {
                if let Err(error) = stream.set_nodelay(true) {
                    return Err((
                        fault::TRANSPORT_IO,
                        format!("cannot configure the broker socket: {error}"),
                    ));
                }
                if let Err(error) = stream.set_read_timeout(Some(SESSION_POLL)) {
                    return Err((
                        fault::TRANSPORT_IO,
                        format!("cannot configure the broker read timeout: {error}"),
                    ));
                }
                if let Err(error) = stream.set_write_timeout(Some(plan.session.connect_timeout)) {
                    return Err((
                        fault::TRANSPORT_IO,
                        format!("cannot configure the broker write timeout: {error}"),
                    ));
                }
                return Ok(stream);
            }
            Err(error) => {
                if error.kind() == IoErrorKind::TimedOut {
                    timed_out = true;
                }
                last_error = Some(error);
            }
        }
    }
    let code = if timed_out {
        fault::CONNECT_TIMEOUT
    } else {
        fault::CONNECT
    };
    let detail = last_error.map_or_else(
        || "the host resolved to no addresses".to_owned(),
        |error| error.to_string(),
    );
    Err((
        code,
        format!(
            "cannot connect to broker `{}:{}`: {detail}",
            plan.host, plan.port
        ),
    ))
}

/// Returns the last reason byte carried by a server packet, when it has one.
fn packet_reason(packet: &Packet) -> Option<u8> {
    match packet {
        Packet::ConnAck(ack) => Some(ack.reason),
        Packet::PubAck(ack) | Packet::PubRec(ack) | Packet::PubRel(ack) | Packet::PubComp(ack) => {
            Some(ack.reason)
        }
        Packet::SubAck(ack) => ack
            .reasons
            .iter()
            .copied()
            .find(|reason| *reason != 0)
            .or_else(|| ack.reasons.first().copied()),
        Packet::Disconnect(disconnect) => Some(disconnect.reason),
        Packet::Connect(_)
        | Packet::Publish(_)
        | Packet::Subscribe(_)
        | Packet::Unsubscribe(_)
        | Packet::UnsubAck(_)
        | Packet::PingReq
        | Packet::PingResp => None,
    }
}

#[cfg(test)]
mod tests {
    use super::payload::ByteOrder;
    use super::*;

    #[test]
    fn session_states_have_stable_codes_and_names() {
        let cases = [
            (SessionState::Idle, 0, "idle"),
            (SessionState::Connecting, 1, "connecting"),
            (SessionState::Connected, 2, "connected"),
            (SessionState::Backoff, 3, "backoff"),
            (SessionState::Disconnecting, 4, "disconnecting"),
            (SessionState::Closed, 5, "closed"),
        ];
        for (state, code, name) in cases {
            assert_eq!(state_code(state), code);
            assert_eq!(state_name(code), name);
        }
        assert_eq!(state_name(u8::MAX), "idle");
    }

    #[test]
    fn staleness_reports_missing_and_expired_values_without_clearing_them() {
        let now = Instant::now();
        let timeout = Duration::from_millis(100);
        assert!(is_stale(now, None, timeout));
        assert!(!is_stale(now, Some(now), timeout));
        assert!(!is_stale(
            now,
            Some(now.checked_sub(timeout).expect("recent instant")),
            timeout
        ));
        assert!(is_stale(
            now,
            Some(
                now.checked_sub(timeout + Duration::from_millis(1))
                    .expect("recent instant")
            ),
            timeout
        ));
    }

    #[test]
    fn counters_saturate_instead_of_wrapping() {
        let counter = AtomicU64::new(u64::MAX - 1);
        saturating_increment(&counter);
        assert_eq!(counter.load(Ordering::SeqCst), u64::MAX);
        saturating_increment(&counter);
        assert_eq!(counter.load(Ordering::SeqCst), u64::MAX);
    }

    #[test]
    fn bank_marks_only_changed_outputs_dirty_and_releases_pending_ids() {
        let mut bank = Bank::new(1, 2);
        bank.store_output(0, 7);
        assert_eq!(bank.take_dirty(), vec![(0, 7)]);
        bank.store_output(0, 7);
        assert!(bank.take_dirty().is_empty());

        bank.store_output(1, 9);
        let dirty = bank.take_dirty();
        assert_eq!(dirty, vec![(1, 9)]);
        bank.restore_dirty(&dirty);
        assert_eq!(bank.take_dirty(), vec![(1, 9)]);

        bank.pending.insert(11, 0);
        bank.pending.insert(12, 1);
        assert_eq!(bank.reset_for_connection(), vec![11, 12]);
        assert!(bank.pending.is_empty());
        assert_eq!(bank.take_dirty().len(), 2);
    }

    #[test]
    fn status_values_select_the_requested_client_or_cycle_counter() {
        let backend = MqttBackend::new();
        let now = Instant::now();
        let binding = |index, field| StatusBinding {
            device: "status".to_owned(),
            field,
            index,
            image: None,
        };

        assert_eq!(
            backend.status_value(&binding(StatusIndex::Global, StatusField::Exchanges), now),
            0
        );
        assert_eq!(
            backend.status_value(&binding(StatusIndex::Global, StatusField::Faults), now),
            0
        );
        assert_eq!(
            backend.status_value(&binding(StatusIndex::Global, StatusField::Stale), now),
            0
        );
        assert_eq!(
            backend.status_value(
                &binding(StatusIndex::Client(3), StatusField::Connected),
                now
            ),
            0
        );
        assert_eq!(
            backend.status_value(&binding(StatusIndex::Global, StatusField::Connected), now),
            0
        );

        backend.counters.exchanges.store(4, Ordering::SeqCst);
        backend.counters.faults.store(5, Ordering::SeqCst);
        assert_eq!(
            backend.status_value(&binding(StatusIndex::Global, StatusField::Exchanges), now),
            4
        );
        assert_eq!(
            backend.status_value(&binding(StatusIndex::Global, StatusField::Faults), now),
            5
        );
    }

    #[test]
    fn backend_state_is_stopped_ready_then_running() {
        let mut backend = MqttBackend::new();
        assert_eq!(backend.compute_state(), BackendState::Stopped);
        backend.started = true;
        assert_eq!(backend.compute_state(), BackendState::Ready);
        backend.cycled = true;
        assert_eq!(backend.compute_state(), BackendState::Running);
    }

    #[test]
    fn payload_mapping_uses_the_channel_encoding_width_and_byte_order() {
        let bits = u64::from(1.5_f32.to_bits());
        let encoded = payload::encode(Encoding::Float, 32, ByteOrder::Big, bits)
            .expect("encode a 32-bit float output");
        assert_eq!(encoded, 1.5_f32.to_be_bytes());
        assert_eq!(
            payload::decode(Encoding::Float, 32, ByteOrder::Big, &encoded)
                .expect("decode the same float"),
            bits
        );

        let little = payload::encode(Encoding::Uint, 16, ByteOrder::Little, 0xBEEF)
            .expect("encode a little-endian uint input");
        assert_eq!(little, 0xBEEF_u16.to_le_bytes());
        assert!(
            payload::decode(Encoding::Uint, 16, ByteOrder::Big, &[0xBE]).is_err(),
            "a truncated payload is rejected instead of replacing the cached value"
        );
    }
}
