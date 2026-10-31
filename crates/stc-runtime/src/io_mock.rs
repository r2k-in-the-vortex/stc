//! Protocol-neutral mock device backend (IO-Routing-and-Backends §4.2).
//!
//! Phase 1 uses this backend in place of the EtherCAT shim so the coordinator,
//! the route graph, and the process-image mapping are exercised without
//! hardware. It validates the embedded device configuration and PDO/channel
//! map, then moves values between a [`MockWire`] and its bound image ranges.
//!
//! Phase-1 mapping failures use `STC-E-IO-BINDING-002`. The EtherCAT-specific
//! `STC-E-IO-ETH-006`/`-007` codes remain phase-2 work.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use stc_got::error::{Error, Result};
use stc_got::json::{self, Json};

use crate::coordinator::{
    BackendChannel, BackendConfig, BackendDeviceConfig, BackendState, BackendStatus, DeviceBackend,
    ImageRange, ProcessImage, codes,
};

/// The kind served by the phase-1 mock EtherCAT backend.
pub const ETHERCAT_KIND: &str = "ethercat";

/// The read-only status kind served by the same backend instance.
pub const ETHERCAT_STATUS_KIND: &str = "ethercat-status";

/// A shared mock fieldbus wire.
///
/// Tests inject input values before [`DeviceBackend::latch_inputs`] and read
/// published output values after [`DeviceBackend::publish_outputs`]. Clones
/// share one wire.
#[derive(Clone, Default)]
pub struct MockWire {
    state: Arc<Mutex<WireState>>,
    fail_latch: Arc<AtomicBool>,
    fail_publish: Arc<AtomicBool>,
}

#[derive(Default)]
struct WireState {
    inputs: BTreeMap<(String, String), u64>,
    outputs: BTreeMap<(String, String), u64>,
}

impl MockWire {
    /// Creates an empty wire.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets an input channel value as delivered by the fieldbus.
    pub fn set_input(&self, device: &str, channel: &str, value: u64) {
        if let Ok(mut state) = self.state.lock() {
            state
                .inputs
                .insert((device.to_owned(), channel.to_owned()), value);
        }
    }

    /// Returns the current input channel value.
    #[must_use]
    pub fn input(&self, device: &str, channel: &str) -> Option<u64> {
        self.state.lock().ok().and_then(|state| {
            state
                .inputs
                .get(&(device.to_owned(), channel.to_owned()))
                .copied()
        })
    }

    /// Returns the last output value published for a channel.
    #[must_use]
    pub fn output(&self, device: &str, channel: &str) -> Option<u64> {
        self.state.lock().ok().and_then(|state| {
            state
                .outputs
                .get(&(device.to_owned(), channel.to_owned()))
                .copied()
        })
    }

    /// Makes the next and every later latch report a fault until cleared.
    pub fn set_fail_latch(&self, fail: bool) {
        self.fail_latch.store(fail, Ordering::SeqCst);
    }

    /// Makes the next and every later publish report a fault until cleared.
    pub fn set_fail_publish(&self, fail: bool) {
        self.fail_publish.store(fail, Ordering::SeqCst);
    }

    fn failing_latch(&self) -> bool {
        self.fail_latch.load(Ordering::SeqCst)
    }

    fn failing_publish(&self) -> bool {
        self.fail_publish.load(Ordering::SeqCst)
    }

    fn store_output(&self, device: &str, channel: &str, value: u64) {
        if let Ok(mut state) = self.state.lock() {
            state
                .outputs
                .insert((device.to_owned(), channel.to_owned()), value);
        }
    }
}

/// A phase-1 mock of the EtherCAT backend.
///
/// The same implementation serves a second mock instance registered under a
/// different kind; kinds other than [`ETHERCAT_KIND`] skip the EtherCAT PDO
/// validation and act as a simple channel backend.
pub struct MockEthercatBackend {
    wire: MockWire,
    status: BackendStatus,
    inputs: Vec<BoundChannel>,
    outputs: Vec<BoundChannel>,
    status_ranges: Vec<ImageRange>,
    cycle_faulted: bool,
}

#[derive(Clone)]
struct BoundChannel {
    device: String,
    channel: String,
    image: ImageRange,
}

impl Default for MockEthercatBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MockEthercatBackend {
    /// Creates a backend with an isolated wire.
    #[must_use]
    pub fn new() -> Self {
        Self::with_wire(MockWire::new())
    }

    /// Creates a backend that exchanges through `wire`.
    #[must_use]
    pub fn with_wire(wire: MockWire) -> Self {
        Self {
            wire,
            status: BackendStatus::default(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            status_ranges: Vec::new(),
            cycle_faulted: false,
        }
    }

    fn fail(&mut self, operation: &str) -> Error {
        self.cycle_faulted = true;
        self.status.state = BackendState::Faulted;
        self.status.last_fault = Some(format!("mock fieldbus fault during {operation}"));
        Error::new(
            codes::BINDING_INVALID,
            format!("mock fieldbus fault during {operation}"),
        )
    }

    fn validate(&mut self, config: &BackendConfig) -> Result<()> {
        let data_devices = config
            .devices
            .iter()
            .filter(|device| !is_status_kind(&device.kind))
            .count();
        let mut devices: BTreeMap<&str, &BackendDeviceConfig> = BTreeMap::new();
        for device in &config.devices {
            if devices.insert(device.name.as_str(), device).is_some() {
                return Err(invalid(format!(
                    "backend `{}` owns device `{}` more than once",
                    config.name, device.name
                )));
            }
            if is_status_kind(&device.kind) {
                validate_status_device(device, data_devices)?;
                for channel in &device.channels {
                    if let Some(image) = channel.image {
                        validate_image_range(device, channel, image)?;
                        self.status_ranges.push(image);
                    }
                }
            } else {
                if device.kind == ETHERCAT_KIND {
                    validate_ethercat_device(device)?;
                } else {
                    validate_simple_device(device)?;
                }
                for channel in &device.channels {
                    let Some(image) = channel.image else {
                        continue;
                    };
                    validate_image_range(device, channel, image)?;
                    match channel.direction.as_str() {
                        "input" => self.inputs.push(BoundChannel {
                            device: device.name.clone(),
                            channel: channel.name.clone(),
                            image,
                        }),
                        "output" => self.outputs.push(BoundChannel {
                            device: device.name.clone(),
                            channel: channel.name.clone(),
                            image,
                        }),
                        other => {
                            return Err(invalid(format!(
                                "channel `{}` on device `{}` has direction `{other}`, which the \
                                 mock backend cannot route",
                                channel.name, device.name
                            )));
                        }
                    }
                }
            }
        }
        validate_bindings(config, &devices)?;
        self.status.name = config.name.clone();
        self.status.devices = config.devices.len();
        self.status.state = BackendState::Ready;
        self.status.exchanges = 0;
        self.status.faults = 0;
        self.status.last_fault = None;
        self.cycle_faulted = false;
        Ok(())
    }
}

impl DeviceBackend for MockEthercatBackend {
    fn cold_start(&mut self, config: &BackendConfig) -> Result<()> {
        self.inputs.clear();
        self.outputs.clear();
        self.status_ranges.clear();
        self.validate(config)
    }

    fn latch_inputs(&mut self, image: &mut ProcessImage) -> Result<()> {
        self.cycle_faulted = false;
        if self.wire.failing_latch() {
            return Err(self.fail("input latch"));
        }
        for range in &self.status_ranges {
            if !image.write_bits(range.area, range.offset_bits, range.width_bits, 0) {
                return Err(self.fail("status block write"));
            }
        }
        for input in &self.inputs {
            let value = self.wire.input(&input.device, &input.channel).unwrap_or(0);
            if !image.write_bits(
                input.image.area,
                input.image.offset_bits,
                input.image.width_bits,
                value,
            ) {
                return Err(self.fail("input latch"));
            }
        }
        if !self.cycle_faulted {
            self.status.state = BackendState::Running;
        }
        Ok(())
    }

    fn publish_outputs(&mut self, image: &ProcessImage) -> Result<()> {
        if self.wire.failing_publish() {
            return Err(self.fail("output publish"));
        }
        for output in &self.outputs {
            let Some(value) = image.read_bits(
                output.image.area,
                output.image.offset_bits,
                output.image.width_bits,
            ) else {
                return Err(self.fail("output publish"));
            };
            self.wire
                .store_output(&output.device, &output.channel, value);
        }
        self.status.exchanges = self.status.exchanges.saturating_add(1);
        self.status.state = if self.cycle_faulted {
            BackendState::Faulted
        } else {
            BackendState::Running
        };
        Ok(())
    }

    fn status(&self) -> BackendStatus {
        self.status.clone()
    }

    fn shutdown(&mut self) -> Result<()> {
        self.status.state = BackendState::Stopped;
        Ok(())
    }
}

fn is_status_kind(kind: &str) -> bool {
    kind == ETHERCAT_STATUS_KIND
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(codes::BINDING_INVALID, message)
}

fn validate_image_range(
    device: &BackendDeviceConfig,
    channel: &BackendChannel,
    image: ImageRange,
) -> Result<()> {
    if image.width_bits != u64::from(channel.width_bits) {
        return Err(invalid(format!(
            "channel `{}` on device `{}` is bound as {} bits but declares {} bits",
            channel.name, device.name, image.width_bits, channel.width_bits
        )));
    }
    if image.width_bits > 1 && !image.offset_bits.is_multiple_of(8) {
        return Err(invalid(format!(
            "channel `{}` on device `{}` is not byte-aligned in the process image",
            channel.name, device.name
        )));
    }
    let expected_area = match channel.direction.as_str() {
        "input" => 1,
        // A routed physical output reads its route source's image range,
        // which may be an input range for a pass-through route.
        "output" => {
            if image.area != 1 && image.area != 2 {
                return Err(invalid(format!(
                    "output channel `{}` on device `{}` is bound to image area {}",
                    channel.name, device.name, image.area
                )));
            }
            return Ok(());
        }
        other => {
            return Err(invalid(format!(
                "channel `{}` on device `{}` has direction `{other}`",
                channel.name, device.name
            )));
        }
    };
    if image.area != expected_area {
        return Err(invalid(format!(
            "channel `{}` on device `{}` is bound to image area {} but has direction `{}`",
            channel.name, device.name, image.area, channel.direction
        )));
    }
    Ok(())
}

fn validate_bindings(
    config: &BackendConfig,
    devices: &BTreeMap<&str, &BackendDeviceConfig>,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for binding in &config.bindings {
        let Some(device) = devices.get(binding.device.as_str()) else {
            return Err(invalid(format!(
                "binding names device `{}`, which the instance does not own",
                binding.device
            )));
        };
        let Some(channel) = device
            .channels
            .iter()
            .find(|channel| channel.name == binding.channel)
        else {
            return Err(invalid(format!(
                "binding names channel `{}` on device `{}`, which is not declared",
                binding.channel, binding.device
            )));
        };
        let key = (binding.device.clone(), binding.channel.clone());
        if !seen.insert(key) {
            return Err(invalid(format!(
                "channel `{}` on device `{}` is bound more than once",
                binding.channel, binding.device
            )));
        }
        if channel.image != Some(binding.image) {
            return Err(invalid(format!(
                "binding for channel `{}` on device `{}` disagrees with the channel image range",
                binding.channel, binding.device
            )));
        }
        if binding.direction != channel.direction {
            return Err(invalid(format!(
                "binding for channel `{}` on device `{}` has direction `{}`, but the channel has \
                 `{}`",
                binding.channel, binding.device, binding.direction, channel.direction
            )));
        }
    }
    Ok(())
}

fn validate_simple_device(device: &BackendDeviceConfig) -> Result<()> {
    let mut names = BTreeSet::new();
    for channel in &device.channels {
        if !names.insert(channel.name.as_str()) {
            return Err(invalid(format!(
                "device `{}` declares channel `{}` more than once",
                device.name, channel.name
            )));
        }
        if !matches!(channel.direction.as_str(), "input" | "output") {
            return Err(invalid(format!(
                "device `{}` channel `{}` has direction `{}`; expected `input` or `output`",
                device.name, channel.name, channel.direction
            )));
        }
    }
    if let Some(config) = &device.config {
        json::parse(config).map_err(|error| {
            invalid(format!(
                "device `{}` configuration is not valid JSON: {}",
                device.name, error.message
            ))
        })?;
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Input,
    Output,
}

struct EntryInfo {
    bits: u32,
    direction: Direction,
}

struct PdoChannelInfo {
    name: String,
    address: (u32, u8),
    bit: u32,
}

/// Occupied bit spans per PDO entry: `(start bit, end bit, channel name)`.
type OccupiedSpans = BTreeMap<(u32, u8), Vec<(u32, u32, String)>>;

fn validate_ethercat_device(device: &BackendDeviceConfig) -> Result<()> {
    let Some(config) = &device.config else {
        return Err(invalid(format!(
            "EtherCAT device `{}` has no profile or inline configuration",
            device.name
        )));
    };
    let root = json::parse(config).map_err(|error| {
        invalid(format!(
            "device `{}` configuration is not valid JSON: {}",
            device.name, error.message
        ))
    })?;
    let object = root.as_object().ok_or_else(|| {
        invalid(format!(
            "device `{}` configuration is not a JSON object",
            device.name
        ))
    })?;
    let entries = parse_entries(device, object)?;
    let channels = parse_pdo_channels(device, object)?;

    let mut declared = BTreeMap::new();
    for channel in &device.channels {
        if declared.insert(channel.name.as_str(), channel).is_some() {
            return Err(invalid(format!(
                "device `{}` declares channel `{}` more than once",
                device.name, channel.name
            )));
        }
        if !matches!(channel.direction.as_str(), "input" | "output") {
            return Err(invalid(format!(
                "device `{}` channel `{}` has direction `{}`; expected `input` or `output`",
                device.name, channel.name, channel.direction
            )));
        }
        if !matches!(channel.width_bits, 1 | 8 | 16 | 32) {
            return Err(invalid(format!(
                "device `{}` channel `{}` has width {} bits; expected 1, 8, 16, or 32",
                device.name, channel.name, channel.width_bits
            )));
        }
    }

    let mut mapped = BTreeSet::new();
    let mut occupied: OccupiedSpans = BTreeMap::new();
    for channel in &channels {
        if !mapped.insert(channel.name.as_str()) {
            return Err(invalid(format!(
                "device `{}` maps channel `{}` more than once",
                device.name, channel.name
            )));
        }
        let Some(declared_channel) = declared.get(channel.name.as_str()) else {
            return Err(invalid(format!(
                "device `{}` maps channel `{}`, which is not declared in `stc.toml`",
                device.name, channel.name
            )));
        };
        let Some(entry) = entries.get(&channel.address) else {
            return Err(invalid(format!(
                "device `{}` channel `{}` names PDO entry `0x{:04x}:{}`, which is not declared",
                device.name, channel.name, channel.address.0, channel.address.1
            )));
        };
        let width = declared_channel.width_bits;
        if channel.bit >= entry.bits || channel.bit + width > entry.bits {
            return Err(invalid(format!(
                "device `{}` channel `{}` does not fit inside PDO entry `0x{:04x}:{}`",
                device.name, channel.name, channel.address.0, channel.address.1
            )));
        }
        let expected = match declared_channel.direction.as_str() {
            "input" => Direction::Input,
            "output" => Direction::Output,
            other => {
                return Err(invalid(format!(
                    "device `{}` channel `{}` has direction `{other}`",
                    device.name, channel.name
                )));
            }
        };
        if entry.direction != expected {
            return Err(invalid(format!(
                "device `{}` channel `{}` does not match the PDO entry direction",
                device.name, channel.name
            )));
        }
        if width > 1 && channel.bit % 8 != 0 {
            return Err(invalid(format!(
                "device `{}` channel `{}` with width {width} is not byte-aligned in its PDO entry",
                device.name, channel.name
            )));
        }
        let end = channel.bit + width;
        let occupied = occupied.entry(channel.address).or_default();
        for (start, other_end, other_name) in occupied.iter() {
            if channel.bit < *other_end && *start < end {
                return Err(invalid(format!(
                    "device `{}` channels `{}` and `{other_name}` overlap inside PDO entry \
                     `0x{:04x}:{}`",
                    device.name, channel.name, channel.address.0, channel.address.1
                )));
            }
        }
        occupied.push((channel.bit, end, channel.name.clone()));
    }
    for name in declared.keys() {
        if !mapped.contains(name) {
            return Err(invalid(format!(
                "device `{}` channel `{name}` has no `pdo-channels` mapping",
                device.name
            )));
        }
    }
    Ok(())
}

fn parse_entries(
    device: &BackendDeviceConfig,
    object: &BTreeMap<String, Json>,
) -> Result<BTreeMap<(u32, u8), EntryInfo>> {
    let syncs = match object.get("syncs") {
        None => &[][..],
        Some(value) => value
            .as_array()
            .ok_or_else(|| invalid(format!("device `{}` `syncs` is not an array", device.name)))?,
    };
    let mut entries = BTreeMap::new();
    for sync in syncs {
        let sync = sync.as_object().ok_or_else(|| {
            invalid(format!(
                "device `{}` has a sync that is not an object",
                device.name
            ))
        })?;
        let direction = match sync.get("direction").and_then(Json::as_str) {
            Some("input") => Direction::Input,
            Some("output") => Direction::Output,
            _ => {
                return Err(invalid(format!(
                    "device `{}` has a sync without direction `input` or `output`",
                    device.name
                )));
            }
        };
        let pdos = match sync.get("pdos") {
            None => &[][..],
            Some(value) => value.as_array().ok_or_else(|| {
                invalid(format!("device `{}` `pdos` is not an array", device.name))
            })?,
        };
        for pdo in pdos {
            let pdo = pdo.as_object().ok_or_else(|| {
                invalid(format!(
                    "device `{}` has a PDO that is not an object",
                    device.name
                ))
            })?;
            let values = match pdo.get("entries") {
                None => &[][..],
                Some(value) => value.as_array().ok_or_else(|| {
                    invalid(format!(
                        "device `{}` `entries` is not an array",
                        device.name
                    ))
                })?,
            };
            for entry in values {
                let entry = entry.as_object().ok_or_else(|| {
                    invalid(format!(
                        "device `{}` has a PDO entry that is not an object",
                        device.name
                    ))
                })?;
                let index = index_value(entry.get("index"), device, "PDO entry index")?;
                let subindex = number_u8(entry.get("subindex"), device, "PDO entry subindex")?;
                let bits = number_u32(entry.get("bits"), device, "PDO entry bits")?;
                if !(1..=64).contains(&bits) {
                    return Err(invalid(format!(
                        "device `{}` PDO entry `0x{index:04x}:{subindex}` has width {bits}; \
                         expected 1..=64",
                        device.name
                    )));
                }
                if entries
                    .insert((index, subindex), EntryInfo { bits, direction })
                    .is_some()
                {
                    return Err(invalid(format!(
                        "device `{}` declares PDO entry `0x{index:04x}:{subindex}` more than once",
                        device.name
                    )));
                }
            }
        }
    }
    Ok(entries)
}

fn parse_pdo_channels(
    device: &BackendDeviceConfig,
    object: &BTreeMap<String, Json>,
) -> Result<Vec<PdoChannelInfo>> {
    let values = match object.get("pdo-channels") {
        None => &[][..],
        Some(value) => value.as_array().ok_or_else(|| {
            invalid(format!(
                "device `{}` `pdo-channels` is not an array",
                device.name
            ))
        })?,
    };
    let mut channels = Vec::with_capacity(values.len());
    for value in values {
        let value = value.as_object().ok_or_else(|| {
            invalid(format!(
                "device `{}` has a `pdo-channels` entry that is not an object",
                device.name
            ))
        })?;
        let name = value
            .get("name")
            .and_then(Json::as_str)
            .ok_or_else(|| {
                invalid(format!(
                    "device `{}` has an unnamed PDO channel",
                    device.name
                ))
            })?
            .to_owned();
        let entry = value
            .get("entry")
            .and_then(Json::as_object)
            .ok_or_else(|| {
                invalid(format!(
                    "device `{}` channel `{name}` has no `entry` object",
                    device.name
                ))
            })?;
        let index = index_value(entry.get("index"), device, "pdo-channel entry index")?;
        let subindex = number_u8(entry.get("subindex"), device, "pdo-channel entry subindex")?;
        let bit = number_u32(value.get("bit"), device, "pdo-channel bit")?;
        channels.push(PdoChannelInfo {
            name,
            address: (index, subindex),
            bit,
        });
    }
    Ok(channels)
}

fn validate_status_device(device: &BackendDeviceConfig, data_devices: usize) -> Result<()> {
    if let Some(config) = &device.config {
        let root = json::parse(config).map_err(|error| {
            invalid(format!(
                "status device `{}` configuration is not valid JSON: {}",
                device.name, error.message
            ))
        })?;
        let object = root.as_object().ok_or_else(|| {
            invalid(format!(
                "status device `{}` configuration is not a JSON object",
                device.name
            ))
        })?;
        if let Some(layout) = object.get("layout")
            && layout.as_number() != Some(1)
        {
            return Err(invalid(format!(
                "status device `{}` has an unknown layout; expected 1",
                device.name
            )));
        }
    }
    let mut names = BTreeSet::new();
    for channel in &device.channels {
        if !names.insert(channel.name.as_str()) {
            return Err(invalid(format!(
                "status device `{}` declares channel `{}` more than once",
                device.name, channel.name
            )));
        }
        if channel.direction != "input" {
            return Err(invalid(format!(
                "status device `{}` channel `{}` must be read-only (direction `input`)",
                device.name, channel.name
            )));
        }
        let Some(width) = status_channel_width(&channel.name, data_devices) else {
            return Err(invalid(format!(
                "status device `{}` names unknown status channel `{}`",
                device.name, channel.name
            )));
        };
        if channel.width_bits != width {
            return Err(invalid(format!(
                "status device `{}` channel `{}` is {} bits; the status layout uses {width} bits",
                device.name, channel.name, channel.width_bits
            )));
        }
    }
    Ok(())
}

pub(crate) fn status_channel_width(name: &str, data_devices: usize) -> Option<u32> {
    let width = match name {
        "master.phase"
        | "master.slaves_responding"
        | "domain.wc_state"
        | "domain.working_counter" => 16,
        "master.active" | "master.link_up" | "input.stale" => 1,
        "master.last_error" | "master.recoveries" | "cycle.exchanges" | "cycle.faults" => 32,
        _ => {
            let rest = name.strip_prefix("slave")?;
            let (index, field) = rest.split_once('.')?;
            let index = index.parse::<usize>().ok()?;
            if index >= data_devices {
                return None;
            }
            match field {
                "al_state" => 16,
                "online" | "operational" => 1,
                _ => return None,
            }
        }
    };
    Some(width)
}

fn index_value(value: Option<&Json>, device: &BackendDeviceConfig, what: &str) -> Result<u32> {
    let value =
        value.ok_or_else(|| invalid(format!("device `{}` {what} is missing", device.name)))?;
    if let Some(number) = value.as_number() {
        return u32::try_from(number)
            .map_err(|_| invalid(format!("device `{}` {what} is out of range", device.name)));
    }
    let text = value.as_str().ok_or_else(|| {
        invalid(format!(
            "device `{}` {what} is not an integer or hexadecimal string",
            device.name
        ))
    })?;
    parse_identity(text).ok_or_else(|| {
        invalid(format!(
            "device `{}` {what} is not an integer or hexadecimal string",
            device.name
        ))
    })
}

fn parse_identity(text: &str) -> Option<u32> {
    let text = text.trim();
    let (digits, radix) = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(digits) => (digits, 16),
        None => (text, 10),
    };
    if digits.is_empty() {
        return None;
    }
    u32::from_str_radix(digits, radix).ok()
}

fn number_u32(value: Option<&Json>, device: &BackendDeviceConfig, what: &str) -> Result<u32> {
    let value = value
        .and_then(Json::as_number)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            invalid(format!(
                "device `{}` {what} must be a non-negative integer",
                device.name
            ))
        })?;
    Ok(value)
}

fn number_u8(value: Option<&Json>, device: &BackendDeviceConfig, what: &str) -> Result<u8> {
    let value = value
        .and_then(Json::as_number)
        .and_then(|value| u8::try_from(value).ok())
        .ok_or_else(|| {
            invalid(format!(
                "device `{}` {what} must be an integer in 0..=255",
                device.name
            ))
        })?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use crate::coordinator::{BackendBinding, BackendChannel, BackendConfig, BackendDeviceConfig};
    use crate::module_info::ImageLayout;

    use super::*;

    const SYNC_CONFIG: &str = r#"{
        "syncs": [
            {
                "direction": "input",
                "pdos": [{
                    "entries": [
                        {"index": "0x3001", "subindex": 1, "bits": 16}
                    ]
                }]
            },
            {
                "direction": "output",
                "pdos": [{
                    "entries": [
                        {"index": "0x3101", "subindex": 1, "bits": 16}
                    ]
                }]
            }
        ],
        "pdo-channels": [
            {"name": "IN_A", "entry": {"index": "0x3001", "subindex": 1}, "bit": 0},
            {"name": "IN_B", "entry": {"index": "0x3001", "subindex": 1}, "bit": 8},
            {"name": "OUT_A", "entry": {"index": "0x3101", "subindex": 1}, "bit": 0},
            {"name": "OUT_B", "entry": {"index": "0x3101", "subindex": 1}, "bit": 8}
        ]
    }"#;

    const SIMPLE_CONFIG: &str = r#"{
        "syncs": [{
            "direction": "input",
            "pdos": [{"entries": [{"index": "0x3001", "subindex": 1, "bits": 8}]}]
        }],
        "pdo-channels": [
            {"name": "IN", "entry": {"index": "0x3001", "subindex": 1}, "bit": 0}
        ]
    }"#;

    fn chan(name: &str, direction: &str, width_bits: u32) -> BackendChannel {
        BackendChannel {
            name: name.into(),
            direction: direction.into(),
            width_bits,
            image: None,
        }
    }

    fn bound(
        name: &str,
        direction: &str,
        width_bits: u32,
        area: u8,
        offset_bits: u64,
    ) -> BackendChannel {
        BackendChannel {
            name: name.into(),
            direction: direction.into(),
            width_bits,
            image: Some(ImageRange {
                area,
                offset_bits,
                width_bits: u64::from(width_bits),
            }),
        }
    }

    fn bind(
        device: &str,
        channel: &str,
        direction: &str,
        area: u8,
        offset_bits: u64,
        width_bits: u64,
    ) -> BackendBinding {
        BackendBinding {
            device: device.into(),
            channel: channel.into(),
            image: ImageRange {
                area,
                offset_bits,
                width_bits,
            },
            direction: direction.into(),
        }
    }

    fn device(
        name: &str,
        kind: &str,
        config: Option<&str>,
        channels: Vec<BackendChannel>,
    ) -> BackendDeviceConfig {
        BackendDeviceConfig {
            name: name.into(),
            kind: kind.into(),
            config: config.map(str::to_owned),
            channels,
        }
    }

    fn backend_config(
        devices: Vec<BackendDeviceConfig>,
        bindings: Vec<BackendBinding>,
    ) -> BackendConfig {
        BackendConfig {
            name: "ethercat".into(),
            devices,
            bindings,
            layout: ImageLayout {
                inputs_bytes: 8,
                outputs_bytes: 8,
                markers_bytes: 0,
            },
            profile: None,
        }
    }

    fn assert_invalid(config: BackendConfig, needle: &str) {
        let error = MockEthercatBackend::new()
            .cold_start(&config)
            .expect_err("invalid configuration");
        assert_eq!(error.code, codes::BINDING_INVALID);
        assert!(
            error.message.contains(needle),
            "unexpected message: {}",
            error.message
        );
    }

    #[test]
    fn valid_pdo_mapping_moves_values_through_the_wire() {
        let config = backend_config(
            vec![device(
                "diewu",
                "ethercat",
                Some(SYNC_CONFIG),
                vec![
                    bound("IN_A", "input", 8, 1, 0),
                    chan("IN_B", "input", 8),
                    bound("OUT_A", "output", 8, 2, 0),
                    chan("OUT_B", "output", 8),
                ],
            )],
            vec![
                bind("diewu", "IN_A", "input", 1, 0, 8),
                bind("diewu", "OUT_A", "output", 2, 0, 8),
            ],
        );
        let wire = MockWire::new();
        wire.set_input("diewu", "IN_A", 0x5A);
        wire.set_input("diewu", "IN_B", 0x99);
        let mut backend = MockEthercatBackend::with_wire(wire.clone());
        backend.cold_start(&config).expect("valid mapping");
        let mut image = ProcessImage::new(config.layout);
        assert!(image.write_bits(1, 0, 8, 0));
        assert!(image.write_bits(1, 8, 8, 0xEE));
        backend.latch_inputs(&mut image).expect("latch");
        assert_eq!(image.read_bits(1, 0, 8), Some(0x5A));
        assert_eq!(image.read_bits(1, 8, 8), Some(0xEE));
        assert!(image.write_bits(2, 0, 8, 0x77));
        backend.publish_outputs(&image).expect("publish");
        assert_eq!(wire.output("diewu", "OUT_A"), Some(0x77));
        assert_eq!(wire.output("diewu", "OUT_B"), None);
        let status = backend.status();
        assert_eq!(status.name, "ethercat");
        assert_eq!(status.devices, 1);
        assert_eq!(status.state, BackendState::Running);
        assert_eq!(status.exchanges, 1);
        backend.shutdown().expect("shutdown");
        assert_eq!(backend.status().state, BackendState::Stopped);
    }

    fn one_channel_config(entry_bits: u32, bit: u32, sync: &str) -> String {
        format!(
            r#"{{
                "syncs": [{{
                    "direction": "{sync}",
                    "pdos": [{{"entries": [{{"index": "0x3001", "subindex": 1, "bits": {entry_bits}}}]}}]
                }}],
                "pdo-channels": [
                    {{"name": "IN", "entry": {{"index": "0x3001", "subindex": 1}}, "bit": {bit}}}
                ]
            }}"#
        )
    }

    #[test]
    fn pdo_mapping_rejects_invalid_entries() {
        let fit = one_channel_config(8, 0, "input");
        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(&fit),
                    vec![chan("IN", "input", 16)],
                )],
                Vec::new(),
            ),
            "does not fit inside PDO entry",
        );

        let aligned = one_channel_config(32, 4, "input");
        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(&aligned),
                    vec![chan("IN", "input", 16)],
                )],
                Vec::new(),
            ),
            "is not byte-aligned in its PDO entry",
        );

        let direction = one_channel_config(8, 0, "output");
        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(&direction),
                    vec![chan("IN", "input", 8)],
                )],
                Vec::new(),
            ),
            "does not match the PDO entry direction",
        );

        let width = one_channel_config(16, 0, "input");
        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(&width),
                    vec![chan("IN", "input", 12)],
                )],
                Vec::new(),
            ),
            "width 12 bits",
        );

        let duplicate = one_channel_config(8, 0, "input");
        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(&duplicate),
                    vec![chan("IN", "input", 8), chan("IN", "input", 8)],
                )],
                Vec::new(),
            ),
            "declares channel `IN` more than once",
        );
    }

    #[test]
    fn pdo_mapping_rejects_unknown_duplicate_and_missing_channels() {
        let undeclared_entry = r#"{
            "syncs": [{"direction": "input", "pdos": [{"entries": [
                {"index": "0x3001", "subindex": 1, "bits": 8}
            ]}]}],
            "pdo-channels": [
                {"name": "IN", "entry": {"index": "0x4001", "subindex": 1}, "bit": 0}
            ]
        }"#;
        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(undeclared_entry),
                    vec![chan("IN", "input", 8)],
                )],
                Vec::new(),
            ),
            "which is not declared",
        );

        let duplicate_mapping = r#"{
            "syncs": [{"direction": "input", "pdos": [{"entries": [
                {"index": "0x3001", "subindex": 1, "bits": 8}
            ]}]}],
            "pdo-channels": [
                {"name": "IN", "entry": {"index": "0x3001", "subindex": 1}, "bit": 0},
                {"name": "IN", "entry": {"index": "0x3001", "subindex": 1}, "bit": 0}
            ]
        }"#;
        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(duplicate_mapping),
                    vec![chan("IN", "input", 8)],
                )],
                Vec::new(),
            ),
            "maps channel `IN` more than once",
        );

        let missing_mapping = r#"{
            "syncs": [{"direction": "input", "pdos": [{"entries": [
                {"index": "0x3001", "subindex": 1, "bits": 8}
            ]}]}],
            "pdo-channels": []
        }"#;
        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(missing_mapping),
                    vec![chan("IN", "input", 8)],
                )],
                Vec::new(),
            ),
            "has no `pdo-channels` mapping",
        );

        let ghost_mapping = r#"{
            "syncs": [{"direction": "input", "pdos": [{"entries": [
                {"index": "0x3001", "subindex": 1, "bits": 8}
            ]}]}],
            "pdo-channels": [
                {"name": "GHOST", "entry": {"index": "0x3001", "subindex": 1}, "bit": 0}
            ]
        }"#;
        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(ghost_mapping),
                    vec![chan("IN", "input", 8)],
                )],
                Vec::new(),
            ),
            "which is not declared in `stc.toml`",
        );

        let overlapping = r#"{
            "syncs": [{"direction": "input", "pdos": [{"entries": [
                {"index": "0x3001", "subindex": 1, "bits": 16}
            ]}]}],
            "pdo-channels": [
                {"name": "A", "entry": {"index": "0x3001", "subindex": 1}, "bit": 0},
                {"name": "B", "entry": {"index": "0x3001", "subindex": 1}, "bit": 0}
            ]
        }"#;
        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(overlapping),
                    vec![chan("A", "input", 1), chan("B", "input", 1)],
                )],
                Vec::new(),
            ),
            "overlap inside PDO entry",
        );
    }

    #[test]
    fn image_ranges_must_match_the_declared_channel() {
        let mut wrong_width = bound("IN", "input", 8, 1, 0);
        wrong_width.image = Some(ImageRange {
            area: 1,
            offset_bits: 0,
            width_bits: 16,
        });
        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(SIMPLE_CONFIG),
                    vec![wrong_width],
                )],
                Vec::new(),
            ),
            "is bound as 16 bits but declares 8 bits",
        );

        let mut unaligned = bound("IN", "input", 16, 1, 4);
        unaligned.image = Some(ImageRange {
            area: 1,
            offset_bits: 4,
            width_bits: 16,
        });
        let wide = one_channel_config(16, 0, "input");
        assert_invalid(
            backend_config(
                vec![device("diewu", "ethercat", Some(&wide), vec![unaligned])],
                Vec::new(),
            ),
            "is not byte-aligned in the process image",
        );

        let output_config = one_channel_config(8, 0, "output");
        let mut output_area = bound("IN", "output", 8, 3, 0);
        output_area.image = Some(ImageRange {
            area: 3,
            offset_bits: 0,
            width_bits: 8,
        });
        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(&output_config),
                    vec![output_area],
                )],
                Vec::new(),
            ),
            "is bound to image area 3",
        );

        let mut input_area = bound("IN", "input", 8, 2, 0);
        input_area.image = Some(ImageRange {
            area: 2,
            offset_bits: 0,
            width_bits: 8,
        });
        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(SIMPLE_CONFIG),
                    vec![input_area],
                )],
                Vec::new(),
            ),
            "is bound to image area 2 but has direction `input`",
        );
    }

    #[test]
    fn bindings_must_agree_with_declared_channels() {
        let channel = bound("IN", "input", 8, 1, 0);
        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(SIMPLE_CONFIG),
                    vec![channel.clone()],
                )],
                vec![bind("ghost", "IN", "input", 1, 0, 8)],
            ),
            "which the instance does not own",
        );
        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(SIMPLE_CONFIG),
                    vec![channel.clone()],
                )],
                vec![bind("diewu", "GHOST", "input", 1, 0, 8)],
            ),
            "which is not declared",
        );

        let duplicate = bind("diewu", "IN", "input", 1, 0, 8);
        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(SIMPLE_CONFIG),
                    vec![channel.clone()],
                )],
                vec![duplicate.clone(), duplicate],
            ),
            "is bound more than once",
        );

        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(SIMPLE_CONFIG),
                    vec![channel.clone()],
                )],
                vec![bind("diewu", "IN", "input", 1, 8, 8)],
            ),
            "disagrees with the channel image range",
        );

        assert_invalid(
            backend_config(
                vec![device(
                    "diewu",
                    "ethercat",
                    Some(SIMPLE_CONFIG),
                    vec![channel],
                )],
                vec![bind("diewu", "IN", "output", 1, 0, 8)],
            ),
            "but the channel has `input`",
        );
    }

    #[test]
    fn status_channels_use_the_documented_layout() {
        let config = backend_config(
            vec![
                device(
                    "diewu",
                    "ethercat",
                    Some(SYNC_CONFIG),
                    vec![
                        chan("IN_A", "input", 8),
                        chan("IN_B", "input", 8),
                        chan("OUT_A", "output", 8),
                        chan("OUT_B", "output", 8),
                    ],
                ),
                device(
                    "status",
                    "ethercat-status",
                    None,
                    vec![
                        bound("master.phase", "input", 16, 1, 0),
                        chan("master.active", "input", 1),
                        chan("cycle.faults", "input", 32),
                        chan("slave0.al_state", "input", 16),
                    ],
                ),
            ],
            vec![bind("status", "master.phase", "input", 1, 0, 16)],
        );
        let mut backend = MockEthercatBackend::new();
        backend.cold_start(&config).expect("valid status device");
        let mut image = ProcessImage::new(config.layout);
        assert!(image.write_bits(1, 0, 16, 0xFFFF));
        backend.latch_inputs(&mut image).expect("latch");
        assert_eq!(image.read_bits(1, 0, 16), Some(0));
    }

    #[test]
    fn status_devices_reject_unknown_and_mismatched_channels() {
        let status = |channels: Vec<BackendChannel>, config: Option<&str>| {
            backend_config(
                vec![device("status", "ethercat-status", config, channels)],
                Vec::new(),
            )
        };
        assert_invalid(
            status(vec![chan("master.nonsense", "input", 16)], None),
            "unknown status channel",
        );
        assert_invalid(
            status(vec![chan("master.phase", "input", 8)], None),
            "the status layout uses 16 bits",
        );
        assert_invalid(
            status(vec![chan("master.phase", "output", 16)], None),
            "must be read-only",
        );
        assert_invalid(
            status(
                vec![chan("master.phase", "input", 16)],
                Some(r#"{"layout": 2}"#),
            ),
            "unknown layout",
        );
        assert_invalid(
            status(vec![chan("slave0.online", "input", 1)], None),
            "unknown status channel",
        );
        for (name, width) in [
            ("master.lost_frames", 32),
            ("master.tx_errors", 32),
            ("master.frame_loss_permille", 16),
        ] {
            assert_invalid(
                status(vec![chan(name, "input", width)], None),
                "unknown status channel",
            );
        }
        assert_invalid(
            status(
                vec![
                    chan("master.phase", "input", 16),
                    chan("master.phase", "input", 16),
                ],
                None,
            ),
            "declares channel `master.phase` more than once",
        );
    }

    #[test]
    fn fault_injection_reports_and_clears_a_faulted_backend() {
        let config = backend_config(
            vec![device(
                "diewu",
                "ethercat",
                Some(SYNC_CONFIG),
                vec![
                    bound("IN_A", "input", 8, 1, 0),
                    chan("IN_B", "input", 8),
                    bound("OUT_A", "output", 8, 2, 0),
                    chan("OUT_B", "output", 8),
                ],
            )],
            vec![
                bind("diewu", "IN_A", "input", 1, 0, 8),
                bind("diewu", "OUT_A", "output", 2, 0, 8),
            ],
        );
        let wire = MockWire::new();
        let mut backend = MockEthercatBackend::with_wire(wire.clone());
        backend.cold_start(&config).expect("valid");
        let mut image = ProcessImage::new(config.layout);

        wire.set_fail_latch(true);
        let error = backend.latch_inputs(&mut image).expect_err("latch fault");
        assert_eq!(error.code, codes::BINDING_INVALID);
        assert_eq!(backend.status().state, BackendState::Faulted);
        assert!(backend.status().last_fault.is_some());
        wire.set_fail_latch(false);
        backend.latch_inputs(&mut image).expect("recovered latch");
        assert_eq!(backend.status().state, BackendState::Running);

        wire.set_fail_publish(true);
        let error = backend.publish_outputs(&image).expect_err("publish fault");
        assert_eq!(error.code, codes::BINDING_INVALID);
        assert_eq!(backend.status().state, BackendState::Faulted);
        wire.set_fail_publish(false);
        backend.latch_inputs(&mut image).expect("latch");
        backend.publish_outputs(&image).expect("recovered publish");
        assert_eq!(backend.status().state, BackendState::Running);
        assert_eq!(backend.status().exchanges, 1);
    }
}
