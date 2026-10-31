//! Multi-backend I/O coordinator (IO-Routing-and-Backends §3–§4).
//!
//! The coordinator owns one shared `%I`/`%Q`/`%M` process image. It resolves
//! each declared device `kind` to a registered backend instance, groups
//! bindings and physical route endpoints by device, applies routes at the
//! cycle boundary, and isolates backend faults. Backends never know about each
//! other and no protocol type crosses [`DeviceBackend`].
//!
//! Phase 1 supplies a protocol-neutral mock backend in [`crate::io_mock`];
//! phase 2 registers the same interface with the EtherCAT shim. The public
//! type names here are the phase-1 implementation defaults fixed by
//! IO-Routing-and-Backends §11.

use std::collections::{BTreeMap, BTreeSet};

use stc_got::Digest32;
use stc_got::error::{Error, Result};

use crate::module_info::{
    BindingChannel, ChannelInfo, DeviceInfo, ImageLayout, ModuleInfo, ProfileInfo, RouteEndpoint,
};
use crate::run::IoBackend;

/// Stable diagnostic codes used by the coordinator.
pub mod codes {
    /// A sink has more than one source, or two claims target the same sink.
    pub const BINDING_CONFLICT: &str = "STC-E-IO-BINDING-001";
    /// A route endpoint does not resolve, or its direction/width is invalid.
    pub const BINDING_INVALID: &str = "STC-E-IO-BINDING-002";
    /// A device or route names a `kind` with no registered backend.
    pub const BINDING_NO_BACKEND: &str = "STC-E-IO-BINDING-003";
}

/// One byte range in the process image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageRange {
    /// Image area: 1 inputs, 2 outputs, 3 markers.
    pub area: u8,
    /// Area-relative start bit.
    pub offset_bits: u64,
    /// Range width in bits.
    pub width_bits: u64,
}

/// The shared process image.
///
/// Bits are little-endian: image bit `n` is bit `n % 8` (LSB first) of byte
/// `n / 8`. Areas are contiguous in `%I`, `%Q`, `%M` order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessImage {
    layout: ImageLayout,
    bytes: Vec<u8>,
}

impl ProcessImage {
    /// Creates a zeroed image for `layout`.
    #[must_use]
    pub fn new(layout: ImageLayout) -> Self {
        let length = layout
            .total_bytes()
            .and_then(|bytes| usize::try_from(bytes).ok())
            .unwrap_or(0);
        Self {
            layout,
            bytes: vec![0; length],
        }
    }

    /// Returns the image layout.
    #[must_use]
    pub const fn layout(&self) -> ImageLayout {
        self.layout
    }

    /// Returns the raw image bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the raw image bytes for mutation.
    #[must_use]
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    /// Returns one area's byte slice.
    #[must_use]
    pub fn area_bytes(&self, area: u8) -> Option<&[u8]> {
        let (offset, length) = self.layout.area(area)?;
        let start = usize::try_from(offset).ok()?;
        let end = start.checked_add(usize::try_from(length).ok()?)?;
        self.bytes.get(start..end)
    }

    /// Returns one area's byte slice for mutation.
    #[must_use]
    pub fn area_bytes_mut(&mut self, area: u8) -> Option<&mut [u8]> {
        let (offset, length) = self.layout.area(area)?;
        let start = usize::try_from(offset).ok()?;
        let end = start.checked_add(usize::try_from(length).ok()?)?;
        self.bytes.get_mut(start..end)
    }

    /// Reads a little-endian bit range.
    ///
    /// Returns `None` when the area is unknown, the range leaves the area, or
    /// the width exceeds 64 bits.
    #[must_use]
    pub fn read_bits(&self, area: u8, offset_bits: u64, width_bits: u64) -> Option<u64> {
        let base = self.area_absolute_bits(area)?;
        let limit = self.area_length_bits(area)?;
        let end = offset_bits.checked_add(width_bits)?;
        if width_bits > 64 || end > limit {
            return None;
        }
        let mut value = 0_u64;
        for index in 0..width_bits {
            let absolute = base.checked_add(offset_bits)?.checked_add(index)?;
            let byte = usize::try_from(absolute / 8).ok()?;
            let bit = absolute % 8;
            let set = (*self.bytes.get(byte)? >> bit) & 1;
            value |= u64::from(set) << index;
        }
        Some(value)
    }

    /// Writes a little-endian bit range.
    ///
    /// Returns `false` when the area is unknown, the range leaves the area, or
    /// the width exceeds 64 bits.
    pub fn write_bits(&mut self, area: u8, offset_bits: u64, width_bits: u64, value: u64) -> bool {
        let Some(base) = self.area_absolute_bits(area) else {
            return false;
        };
        let Some(limit) = self.area_length_bits(area) else {
            return false;
        };
        let Some(end) = offset_bits.checked_add(width_bits) else {
            return false;
        };
        if width_bits > 64 || end > limit {
            return false;
        }
        for index in 0..width_bits {
            let Some(absolute) = base
                .checked_add(offset_bits)
                .and_then(|value| value.checked_add(index))
            else {
                return false;
            };
            let byte = usize::try_from(absolute / 8).expect("validated image range");
            let bit = absolute % 8;
            let set = (value >> index) & 1;
            if set == 0 {
                self.bytes[byte] &= !(1 << bit);
            } else {
                self.bytes[byte] |= 1 << bit;
            }
        }
        true
    }

    /// Copies one bit range to another.
    fn copy_bits(&mut self, source: ImageRange, sink: ImageRange) -> bool {
        let Some(value) = self.read_bits(source.area, source.offset_bits, source.width_bits) else {
            return false;
        };
        self.write_bits(sink.area, sink.offset_bits, sink.width_bits, value)
    }

    fn area_absolute_bits(&self, area: u8) -> Option<u64> {
        let (offset, _) = self.layout.area(area)?;
        offset.checked_mul(8)
    }

    fn area_length_bits(&self, area: u8) -> Option<u64> {
        let (_, length) = self.layout.area(area)?;
        length.checked_mul(8)
    }
}

/// Backend lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendState {
    /// Cold start completed and the backend is ready.
    Ready,
    /// The last cycle attempt succeeded.
    Running,
    /// The backend reported a runtime failure; the run continues.
    Faulted,
    /// The backend is retrying after a fault.
    Recovering,
    /// Shutdown completed.
    Stopped,
}

impl BackendState {
    /// Returns the status spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Faulted => "faulted",
            Self::Recovering => "recovering",
            Self::Stopped => "stopped",
        }
    }
}

/// A per-backend status summary (IO-Routing-and-Backends §4.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendStatus {
    /// Backend instance name.
    pub name: String,
    /// Number of devices the instance owns.
    pub devices: usize,
    /// Current state.
    pub state: BackendState,
    /// Successful exchanges since start.
    pub exchanges: u64,
    /// Fault episodes since start.
    pub faults: u64,
    /// Last fault message.
    pub last_fault: Option<String>,
}

impl Default for BackendStatus {
    fn default() -> Self {
        Self {
            name: String::new(),
            devices: 0,
            state: BackendState::Ready,
            exchanges: 0,
            faults: 0,
            last_fault: None,
        }
    }
}

/// One channel's binding to the process image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendBinding {
    /// Device name.
    pub device: String,
    /// Channel name.
    pub channel: String,
    /// Image range.
    pub image: ImageRange,
    /// `input` or `output`.
    pub direction: String,
}

/// One device channel given to a backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendChannel {
    /// Channel name.
    pub name: String,
    /// `input`, `output`, or `inout`.
    pub direction: String,
    /// Declared width in bits.
    pub width_bits: u32,
    /// Bound image range, when the channel is bound to a direct variable.
    pub image: Option<ImageRange>,
}

/// One device owned by a backend instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendDeviceConfig {
    /// Device name.
    pub name: String,
    /// Declared kind.
    pub kind: String,
    /// Effective device configuration as JSON text (profile merged over
    /// inline configuration).
    pub config: Option<String>,
    /// Declared channels in declaration order.
    pub channels: Vec<BackendChannel>,
}

/// Backend-neutral cold-start configuration.
///
/// No protocol type crosses this interface; device configuration stays opaque
/// JSON text interpreted by the backend that owns the device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendConfig {
    /// Instance name.
    pub name: String,
    /// Owned devices in declaration order.
    pub devices: Vec<BackendDeviceConfig>,
    /// Bindings owned by the instance, in metadata order.
    pub bindings: Vec<BackendBinding>,
    /// Process-image layout.
    pub layout: ImageLayout,
    /// Embedded machine profile summary, when the project selects one.
    pub profile: Option<ProfileInfo>,
}

/// The coordinator-facing backend interface (IO-Routing-and-Backends §4.2).
///
/// No EtherCAT, Modbus, or other protocol type crosses this interface. The
/// coordinator applies routes, never a backend.
pub trait DeviceBackend {
    /// Validates the configuration and enters the ready state.
    fn cold_start(&mut self, config: &BackendConfig) -> Result<()>;
    /// Writes the instance's bound `%I` ranges and status block.
    fn latch_inputs(&mut self, image: &mut ProcessImage) -> Result<()>;
    /// Reads the instance's bound `%Q` ranges.
    fn publish_outputs(&mut self, image: &ProcessImage) -> Result<()>;
    /// Returns the per-backend status summary.
    fn status(&self) -> BackendStatus;
    /// Stops exchanging and releases resources.
    fn shutdown(&mut self) -> Result<()>;
}

#[derive(Clone)]
struct Registration {
    instance: String,
    factory: std::rc::Rc<dyn Fn() -> Box<dyn DeviceBackend>>,
}

/// Maps device `kind` spellings to backend instances.
///
/// Several kinds may map to one instance; for example `ethercat` and
/// `ethercat-status` share the EtherCAT backend.
#[derive(Clone, Default)]
pub struct BackendRegistry {
    registrations: BTreeMap<String, Registration>,
}

impl BackendRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates the phase-1 registry: `ethercat` and `ethercat-status` share
    /// one mock EtherCAT instance. Phase 2 replaces the factory with the
    /// shim-backed backend; the coordinator and kinds do not change.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();
        registry.register("ethercat", "ethercat", || {
            Box::new(crate::io_mock::MockEthercatBackend::new())
        });
        registry.register("ethercat-status", "ethercat", || {
            Box::new(crate::io_mock::MockEthercatBackend::new())
        });
        registry
    }

    /// Registers `kind` as served by `instance`.
    ///
    /// Registering the same kind again replaces its registration. The factory
    /// runs once per instance at cold start.
    pub fn register(
        &mut self,
        kind: impl Into<String>,
        instance: impl Into<String>,
        factory: impl Fn() -> Box<dyn DeviceBackend> + 'static,
    ) {
        self.registrations.insert(
            kind.into(),
            Registration {
                instance: instance.into(),
                factory: std::rc::Rc::new(factory),
            },
        );
    }

    /// Returns the instance serving `kind`.
    #[must_use]
    pub fn instance(&self, kind: &str) -> Option<&str> {
        self.registrations
            .get(kind)
            .map(|registration| registration.instance.as_str())
    }

    fn create(&self, instance: &str) -> Option<Box<dyn DeviceBackend>> {
        self.registrations
            .values()
            .find(|registration| registration.instance == instance)
            .map(|registration| (registration.factory)())
    }
}

/// One route resolved to image work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RoutePlan {
    /// Physical input to a PLC input; applied at the input boundary.
    InputCopy {
        source: ImageRange,
        sink: ImageRange,
    },
    /// PLC output to a PLC input; buffered for one scan.
    PlcToPlc {
        source: ImageRange,
        sink: ImageRange,
    },
}

struct Instance {
    name: String,
    backend: Box<dyn DeviceBackend>,
    devices: usize,
    faults: u64,
    last_fault: Option<String>,
}

/// Per-backend report with the coordinator's aggregate view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendReport {
    /// Backend instance name.
    pub name: String,
    /// Number of devices.
    pub devices: usize,
    /// Current state.
    pub state: BackendState,
    /// Successful exchanges.
    pub exchanges: u64,
    /// Fault episodes, including coordinator-observed failures.
    pub faults: u64,
    /// Last fault message.
    pub last_fault: Option<String>,
    /// Configuration hash carried in the status (IO-Routing-and-Backends §7).
    pub config_hash: String,
}

/// The multi-backend coordinator.
pub struct Coordinator {
    image: ProcessImage,
    input_defaults: Vec<u8>,
    global_backend: IoBackend,
    instances: Vec<Instance>,
    routes: Vec<RoutePlan>,
    staged_plc: Vec<u64>,
    claimed_output: Vec<bool>,
    config_hash: String,
}

impl Coordinator {
    /// Cold-starts every backend instance in device declaration order.
    ///
    /// A partial start shuts down the instances already started in reverse
    /// order and fails the run (IO-Routing-and-Backends §4.3–§4.4).
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-BINDING-003` for an unregistered `kind`,
    /// `STC-E-IO-BINDING-002` for an unresolved or invalid endpoint, and
    /// `STC-E-IO-BINDING-001` for a duplicate sink claim or overlapping
    /// binding.
    pub fn cold_start(info: &ModuleInfo, registry: &BackendRegistry) -> Result<Self> {
        let image = ProcessImage::new(info.image_layout());
        let mut profile_devices = BTreeSet::new();
        if let Some(profile) = &info.profile {
            for name in &profile.devices {
                let Some(device) = info.devices.iter().find(|device| device.name == *name) else {
                    return Err(binding_invalid(format!(
                        "machine profile configures undeclared device `{name}`"
                    )));
                };
                if is_status_kind(&device.kind) {
                    return Err(binding_invalid(format!(
                        "machine profile configures status device `{name}`; status devices have no \
                         profile entry"
                    )));
                }
                profile_devices.insert(name.as_str());
            }
        }
        for device in &info.devices {
            if !is_status_kind(&device.kind)
                && device.config.is_none()
                && !profile_devices.contains(device.name.as_str())
            {
                return Err(binding_invalid(format!(
                    "declared data device `{}` has neither a profile entry nor inline configuration",
                    device.name
                )));
            }
        }
        let mut claims: BTreeMap<String, String> = BTreeMap::new();
        // (device, channel) -> image range.
        let mut channel_images: BTreeMap<(String, String), ImageRange> = BTreeMap::new();
        let mut ranges: BTreeMap<u8, Vec<ImageRange>> = BTreeMap::new();
        for binding in &info.bindings {
            let (Some(device_name), Some(channel_ref)) = (&binding.device, &binding.channel) else {
                continue;
            };
            let Some(device) = info
                .devices
                .iter()
                .find(|device| device.name == *device_name)
            else {
                return Err(binding_invalid(format!(
                    "binding for `{}` names undeclared device `{device_name}`",
                    binding.got_id.to_hex()
                )));
            };
            let channel = resolve_channel(device, channel_ref).ok_or_else(|| {
                binding_invalid(format!(
                    "binding for `{}` names a channel that device `{device_name}` does not declare",
                    binding.got_id.to_hex()
                ))
            })?;
            let direction = binding
                .direction
                .as_deref()
                .unwrap_or(channel.direction.as_str());
            if direction == "inout" {
                return Err(binding_invalid(format!(
                    "binding of channel `{}` on device `{device_name}` has direction `inout`, \
                     which cannot be routed",
                    channel.name
                )));
            }
            let expected_area = if direction == "input" { 1 } else { 2 };
            if binding.area != expected_area {
                return Err(binding_invalid(format!(
                    "binding of channel `{}` on device `{device_name}` has direction `{direction}` \
                     but image area {}",
                    channel.name, binding.area
                )));
            }
            if binding.width_bits != u64::from(channel.width_bits) {
                return Err(binding_invalid(format!(
                    "binding of channel `{}` on device `{device_name}` is {} bits but the channel \
                     is {} bits",
                    channel.name, binding.width_bits, channel.width_bits
                )));
            }
            if !image_fits(
                &image,
                binding.area,
                binding.offset_bits,
                binding.width_bits,
            ) {
                return Err(binding_invalid(format!(
                    "binding of channel `{}` on device `{device_name}` leaves the process image",
                    channel.name
                )));
            }
            let range = ImageRange {
                area: binding.area,
                offset_bits: binding.offset_bits,
                width_bits: binding.width_bits,
            };
            ensure_disjoint(
                &mut ranges,
                range,
                &format!(
                    "binding of channel `{}` on device `{device_name}`",
                    channel.name
                ),
            )?;
            if channel_images
                .insert((device_name.clone(), channel.name.clone()), range)
                .is_some()
            {
                return Err(binding_conflict(format!(
                    "channel `{}` on device `{device_name}` is bound more than once",
                    channel.name
                )));
            }
            let (key, description) = if binding.area == 1 {
                (
                    format!("plc-input:{}", binding.offset_bits),
                    format!("binding of `%I` at bit {}", binding.offset_bits),
                )
            } else {
                (
                    format!("physical-output:{device_name}.{}", channel.name),
                    format!(
                        "binding of channel `{}` on device `{device_name}`",
                        channel.name
                    ),
                )
            };
            claim(&mut claims, key, description)?;
        }

        let mut routes = Vec::with_capacity(info.routes.len());
        let mut claimed_output = vec![false; image_output_bits(&image)];
        for range in ranges.values().flatten().filter(|range| range.area == 2) {
            claim_bit_range(&mut claimed_output, *range);
        }
        for route in &info.routes {
            let (source, source_kind) = resolve_route_endpoint(
                &info.devices,
                &channel_images,
                &route.source,
                true,
                &image,
            )?;
            if source.width_bits != route.source.width_bits() {
                return Err(binding_invalid(format!(
                    "route source `{source_kind}` is {} bits wide but the declared width is {}",
                    source.width_bits,
                    route.source.width_bits()
                )));
            }
            for sink in &route.sinks {
                if let RouteEndpoint::PhysicalOutput {
                    device,
                    channel,
                    width_bits,
                } = sink
                {
                    let (device, channel, declared_width) =
                        resolve_physical_target(&info.devices, device, channel, "output")?;
                    if declared_width != *width_bits {
                        return Err(binding_invalid(format!(
                            "route sink `{device}.{channel}` is {declared_width} bits but the \
                             routed width is {width_bits}"
                        )));
                    }
                    if declared_width != source.width_bits {
                        return Err(binding_invalid(format!(
                            "route source `{source_kind}` is {} bits wide but sink \
                             `{device}.{channel}` is {declared_width} bits wide",
                            source.width_bits
                        )));
                    }
                    let (key, description) = sink_claim(sink, &source);
                    claim(&mut claims, key, description)?;
                    // A routed physical output has no PLC variable of its own.
                    // Its PDO entry reads the route source's image range, so
                    // the backend publishes the source value unchanged.
                    channel_images.insert((device, channel), source);
                    if source.area == 2 {
                        claim_bit_range(&mut claimed_output, source);
                    }
                    continue;
                }
                let (sink_range, sink_kind) =
                    resolve_route_endpoint(&info.devices, &channel_images, sink, false, &image)?;
                if sink.width_bits() != source.width_bits {
                    return Err(binding_invalid(format!(
                        "route source `{source_kind}` is {} bits wide but sink `{sink_kind}` is \
                         {} bits wide",
                        source.width_bits,
                        sink.width_bits()
                    )));
                }
                let plan = match (&route.source, sink) {
                    (RouteEndpoint::PhysicalInput { .. }, RouteEndpoint::PlcInput { .. }) => {
                        RoutePlan::InputCopy {
                            source,
                            sink: sink_range,
                        }
                    }
                    (RouteEndpoint::PlcOutput { .. }, RouteEndpoint::PlcInput { .. }) => {
                        RoutePlan::PlcToPlc {
                            source,
                            sink: sink_range,
                        }
                    }
                    _ => {
                        return Err(binding_invalid(format!(
                            "route source `{source_kind}` cannot drive sink `{sink_kind}`"
                        )));
                    }
                };
                let (key, description) = sink_claim(sink, &sink_range);
                claim(&mut claims, key, description)?;
                routes.push(plan);
            }
        }

        // Group devices by registered instance, preserving declaration order.
        let mut order: Vec<String> = Vec::new();
        let mut groups: BTreeMap<String, Vec<&DeviceInfo>> = BTreeMap::new();
        for device in &info.devices {
            let Some(instance) = registry.instance(&device.kind) else {
                return Err(no_backend(format!(
                    "device `{}` has kind `{}` with no registered backend",
                    device.name, device.kind
                )));
            };
            let instance = instance.to_owned();
            if !groups.contains_key(&instance) {
                order.push(instance.clone());
            }
            groups.entry(instance).or_default().push(device);
        }

        let config_hash = configuration_hash(info);
        let mut instances = Vec::new();
        for name in &order {
            let Some(mut backend) = registry.create(name) else {
                shutdown_started(&mut instances);
                return Err(no_backend(format!(
                    "backend instance `{name}` is not registered"
                )));
            };
            let devices = groups.get(name).cloned().unwrap_or_default();
            let mut bindings = Vec::new();
            let mut device_configs = Vec::with_capacity(devices.len());
            for device in &devices {
                let mut channels = Vec::with_capacity(device.channels.len());
                for channel in &device.channels {
                    let image_range = channel_images
                        .get(&(device.name.clone(), channel.name.clone()))
                        .copied();
                    channels.push(BackendChannel {
                        name: channel.name.clone(),
                        direction: channel.direction.clone(),
                        width_bits: channel.width_bits,
                        image: image_range,
                    });
                    if let Some(range) = image_range {
                        bindings.push(BackendBinding {
                            device: device.name.clone(),
                            channel: channel.name.clone(),
                            image: range,
                            direction: channel.direction.clone(),
                        });
                    }
                }
                device_configs.push(BackendDeviceConfig {
                    name: device.name.clone(),
                    kind: device.kind.clone(),
                    config: device.config.clone(),
                    channels,
                });
            }
            let config = BackendConfig {
                name: name.clone(),
                devices: device_configs,
                bindings,
                layout: info.image_layout(),
                profile: info.profile.clone(),
            };
            if let Err(error) = backend.cold_start(&config) {
                shutdown_started(&mut instances);
                return Err(error);
            }
            instances.push(Instance {
                name: name.clone(),
                backend,
                devices: devices.len(),
                faults: 0,
                last_fault: None,
            });
        }

        Ok(Self {
            image,
            input_defaults: sized(&info.input_defaults, info.inputs_bytes),
            global_backend: info.backend,
            instances,
            routes,
            staged_plc: Vec::new(),
            claimed_output,
            config_hash,
        })
    }

    /// Applies the input boundary (IO-Routing-and-Backends §4.3 step 2).
    ///
    /// Global input defaults are applied first, then every instance latches
    /// in declaration order, then physical-input routes run, then the
    /// buffered PLC→PLC sources from the previous scan are applied.
    pub fn input_boundary(&mut self) {
        if self.global_backend.is_null_behavior()
            && let Some(area) = self.image.area_bytes_mut(1)
        {
            let count = area.len().min(self.input_defaults.len());
            area[..count].copy_from_slice(&self.input_defaults[..count]);
        }
        for index in 0..self.instances.len() {
            let result = self.instances[index].backend.latch_inputs(&mut self.image);
            self.note(index, result);
        }
        for route in self.routes.clone() {
            if let RoutePlan::InputCopy { source, sink } = route {
                self.image.copy_bits(source, sink);
            }
        }
        // Apply the PLC->PLC values staged after the previous scan. The
        // buffer starts empty, so the first scan sees no staged values.
        if self.staged_plc.len() == self.plc_to_plc().len() {
            let plans = self.plc_to_plc();
            for (value, plan) in self.staged_plc.iter().zip(plans) {
                if let RoutePlan::PlcToPlc { sink, .. } = plan {
                    self.image
                        .write_bits(sink.area, sink.offset_bits, sink.width_bits, *value);
                }
            }
        }
    }

    /// Applies the after-scan boundary (IO-Routing-and-Backends §4.3 step 3).
    ///
    /// PLC→physical routes run first, PLC→PLC sources are staged for the next
    /// input boundary, then every instance publishes in declaration order.
    /// Output publication runs on every cycle attempt, including a faulted
    /// one, so a fieldbus watchdog is not tripped by a missing frame.
    pub fn after_scan(&mut self) {
        let staged = self
            .plc_to_plc()
            .into_iter()
            .map(|plan| {
                let RoutePlan::PlcToPlc { source, .. } = plan else {
                    return 0;
                };
                self.image
                    .read_bits(source.area, source.offset_bits, source.width_bits)
                    .unwrap_or(0)
            })
            .collect();
        self.staged_plc = staged;
        for index in 0..self.instances.len() {
            let result = self.instances[index].backend.publish_outputs(&self.image);
            self.note(index, result);
        }
        if self.global_backend.is_null_behavior() {
            self.clear_unbound_outputs();
        }
    }

    /// Returns the process image.
    #[must_use]
    pub fn image(&self) -> &ProcessImage {
        &self.image
    }

    /// Returns the process image for mutation (test and embedding access).
    #[must_use]
    pub fn image_mut(&mut self) -> &mut ProcessImage {
        &mut self.image
    }

    /// Returns the configuration hash carried in backend status.
    #[must_use]
    pub fn config_hash(&self) -> &str {
        &self.config_hash
    }

    /// Returns every backend's status summary in declaration order.
    #[must_use]
    pub fn reports(&self) -> Vec<BackendReport> {
        self.instances
            .iter()
            .map(|instance| {
                let mut status = instance.backend.status();
                status.name = instance.name.clone();
                status.devices = instance.devices;
                BackendReport {
                    name: status.name,
                    devices: status.devices,
                    state: status.state,
                    exchanges: status.exchanges,
                    faults: status.faults.saturating_add(instance.faults),
                    last_fault: instance.last_fault.clone().or(status.last_fault),
                    config_hash: self.config_hash.clone(),
                }
            })
            .collect()
    }

    /// Shuts down every instance in reverse declaration order.
    ///
    /// Every instance is shut down even when an earlier shutdown fails; the
    /// first error is returned.
    ///
    /// # Errors
    ///
    /// Returns the first shutdown failure.
    pub fn shutdown(&mut self) -> Result<()> {
        let mut first = None;
        for instance in self.instances.iter_mut().rev() {
            if let Err(error) = instance.backend.shutdown() {
                first.get_or_insert(error);
            }
        }
        first.map_or(Ok(()), Err)
    }

    fn plc_to_plc(&self) -> Vec<RoutePlan> {
        self.routes
            .iter()
            .copied()
            .filter(|route| matches!(route, RoutePlan::PlcToPlc { .. }))
            .collect()
    }

    fn note(&mut self, index: usize, result: Result<()>) {
        let instance = &mut self.instances[index];
        match result {
            Ok(()) => {}
            Err(error) => {
                instance.faults = instance.faults.saturating_add(1);
                instance
                    .last_fault
                    .get_or_insert(format!("{}: {}", error.code, error.message));
            }
        }
    }

    fn clear_unbound_outputs(&mut self) {
        let Some(area) = self.image.area_bytes_mut(2) else {
            return;
        };
        for (index, byte) in area.iter_mut().enumerate() {
            let mut mask = 0_u8;
            for bit in 0..8 {
                let absolute = index * 8 + bit;
                if !self.claimed_output.get(absolute).copied().unwrap_or(false) {
                    mask |= 1 << bit;
                }
            }
            *byte &= !mask;
        }
    }
}

impl IoBackend {
    /// Returns `true` when the backend uses the `null` behavior for unbound
    /// image areas.
    #[must_use]
    pub const fn is_null_behavior(self) -> bool {
        matches!(self, Self::Null | Self::Ethercat)
    }
}

fn shutdown_started(instances: &mut [Instance]) {
    for instance in instances.iter_mut().rev() {
        let _ = instance.backend.shutdown();
    }
}

fn sized(bytes: &[u8], length: u64) -> Vec<u8> {
    let mut value = vec![0; usize::try_from(length).unwrap_or(0)];
    let count = value.len().min(bytes.len());
    value[..count].copy_from_slice(&bytes[..count]);
    value
}

fn image_output_bits(image: &ProcessImage) -> usize {
    usize::try_from(image.layout().outputs_bytes)
        .unwrap_or(0)
        .saturating_mul(8)
}

fn image_fits(image: &ProcessImage, area: u8, offset_bits: u64, width_bits: u64) -> bool {
    image.layout().area(area).is_some_and(|(_, length)| {
        offset_bits
            .checked_add(width_bits)
            .is_some_and(|end| end <= length.saturating_mul(8))
    })
}

fn resolve_channel<'a>(
    device: &'a DeviceInfo,
    channel: &BindingChannel,
) -> Option<&'a ChannelInfo> {
    match channel {
        BindingChannel::Name(name) => device.channels.iter().find(|channel| channel.name == *name),
        BindingChannel::Index(index) => usize::try_from(*index)
            .ok()
            .and_then(|index| device.channels.get(index)),
    }
}

fn is_status_kind(kind: &str) -> bool {
    kind.ends_with("-status")
}

/// Resolves a declared physical device/channel to its name and declared width
/// without requiring a bound image range.
fn resolve_physical_target(
    devices: &[DeviceInfo],
    device: &str,
    channel: &str,
    expected: &str,
) -> Result<(String, String, u64)> {
    let declared = devices
        .iter()
        .find(|candidate| candidate.name == device)
        .ok_or_else(|| {
            binding_invalid(format!("route endpoint names undeclared device `{device}`"))
        })?;
    if is_status_kind(&declared.kind) {
        return Err(binding_invalid(format!(
            "route endpoint names status device `{device}`; status channels are not physical \
             route endpoints"
        )));
    }
    let declared_channel = declared
        .channels
        .iter()
        .find(|candidate| candidate.name == channel)
        .ok_or_else(|| {
            binding_invalid(format!(
                "route endpoint names undeclared channel `{channel}` on device `{device}`"
            ))
        })?;
    if declared_channel.direction != expected && declared_channel.direction != "inout" {
        return Err(binding_invalid(format!(
            "route endpoint channel `{channel}` on device `{device}` has direction `{}`, expected \
             `{expected}`",
            declared_channel.direction
        )));
    }
    if declared_channel.width_bits == 0 {
        return Err(binding_invalid(format!(
            "route endpoint channel `{channel}` on device `{device}` has zero width"
        )));
    }
    Ok((
        declared.name.clone(),
        declared_channel.name.clone(),
        u64::from(declared_channel.width_bits),
    ))
}

fn resolve_route_endpoint(
    devices: &[DeviceInfo],
    channel_images: &BTreeMap<(String, String), ImageRange>,
    endpoint: &RouteEndpoint,
    source: bool,
    image: &ProcessImage,
) -> Result<(ImageRange, String)> {
    match endpoint {
        RouteEndpoint::PhysicalInput {
            device,
            channel,
            width_bits,
        }
        | RouteEndpoint::PhysicalOutput {
            device,
            channel,
            width_bits,
        } => {
            let expected = if matches!(endpoint, RouteEndpoint::PhysicalInput { .. }) {
                "input"
            } else {
                "output"
            };
            let declared = devices
                .iter()
                .find(|candidate| candidate.name == *device)
                .ok_or_else(|| {
                    binding_invalid(format!("route endpoint names undeclared device `{device}`"))
                })?;
            if is_status_kind(&declared.kind) {
                return Err(binding_invalid(format!(
                    "route endpoint names status device `{device}`; status channels are not \
                     physical route endpoints"
                )));
            }
            let declared_channel = declared
                .channels
                .iter()
                .find(|candidate| candidate.name == *channel)
                .ok_or_else(|| {
                    binding_invalid(format!(
                        "route endpoint names undeclared channel `{channel}` on device `{device}`"
                    ))
                })?;
            if declared_channel.direction != expected && declared_channel.direction != "inout" {
                return Err(binding_invalid(format!(
                    "route endpoint channel `{channel}` on device `{device}` has direction `{}`, \
                     expected `{expected}`",
                    declared_channel.direction
                )));
            }
            if u64::from(declared_channel.width_bits) != *width_bits {
                return Err(binding_invalid(format!(
                    "route endpoint channel `{channel}` on device `{device}` is {} bits but the \
                     routed width is {width_bits}",
                    declared_channel.width_bits
                )));
            }
            let range = channel_images
                .get(&(device.clone(), channel.clone()))
                .copied()
                .ok_or_else(|| {
                    binding_invalid(format!(
                        "route endpoint channel `{channel}` on device `{device}` is not bound to an \
                         image location"
                    ))
                })?;
            Ok((range, format!("{device}.{channel}")))
        }
        RouteEndpoint::PlcInput {
            area,
            offset_bits,
            width_bits,
        }
        | RouteEndpoint::PlcOutput {
            area,
            offset_bits,
            width_bits,
        } => {
            let expected = if source { 2 } else { 1 };
            if *area != expected {
                return Err(binding_invalid(format!(
                    "route endpoint `%{}` at bit {offset_bits} has area {area}, expected {}",
                    area_char(*area),
                    area_char(expected)
                )));
            }
            if !image_fits(image, *area, *offset_bits, *width_bits) {
                return Err(binding_invalid(format!(
                    "route endpoint `%{}` at bit {offset_bits} leaves the process image",
                    area_char(*area)
                )));
            }
            Ok((
                ImageRange {
                    area: *area,
                    offset_bits: *offset_bits,
                    width_bits: *width_bits,
                },
                format!("%{}@{offset_bits}", area_char(*area)),
            ))
        }
    }
}

fn area_char(area: u8) -> char {
    match area {
        1 => 'I',
        2 => 'Q',
        _ => 'M',
    }
}

fn sink_claim(endpoint: &RouteEndpoint, range: &ImageRange) -> (String, String) {
    match endpoint {
        RouteEndpoint::PhysicalOutput {
            device, channel, ..
        } => (
            format!("physical-output:{device}.{channel}"),
            format!("route sink `{device}.{channel}`"),
        ),
        RouteEndpoint::PlcInput { .. } => (
            format!("plc-input:{}", range.offset_bits),
            format!("route sink `%I` at bit {}", range.offset_bits),
        ),
        RouteEndpoint::PhysicalInput { .. } => {
            ("physical-input".into(), "physical input source".into())
        }
        RouteEndpoint::PlcOutput { .. } => ("plc-output".into(), "PLC output source".into()),
    }
}

fn claim(claims: &mut BTreeMap<String, String>, key: String, description: String) -> Result<()> {
    if let Some(previous) = claims.get(&key) {
        return Err(binding_conflict(format!(
            "{description} and {previous} claim the same sink"
        )));
    }
    claims.insert(key, description);
    Ok(())
}

fn ensure_disjoint(
    ranges: &mut BTreeMap<u8, Vec<ImageRange>>,
    range: ImageRange,
    description: &str,
) -> Result<()> {
    let existing = ranges.entry(range.area).or_default();
    for other in existing.iter() {
        let end = range.offset_bits.saturating_add(range.width_bits);
        let other_end = other.offset_bits.saturating_add(other.width_bits);
        if range.offset_bits < other_end && other.offset_bits < end {
            return Err(binding_conflict(format!(
                "{description} overlaps another bound image range"
            )));
        }
    }
    existing.push(range);
    Ok(())
}

fn claim_bit_range(claimed: &mut [bool], range: ImageRange) {
    for bit in 0..range.width_bits {
        let Some(absolute) = range
            .offset_bits
            .checked_add(bit)
            .and_then(|value| usize::try_from(value).ok())
        else {
            continue;
        };
        if let Some(slot) = claimed.get_mut(absolute) {
            *slot = true;
        }
    }
}

fn binding_invalid(message: impl Into<String>) -> Error {
    Error::new(codes::BINDING_INVALID, message)
}

fn binding_conflict(message: impl Into<String>) -> Error {
    Error::new(codes::BINDING_CONFLICT, message)
}

fn no_backend(message: impl Into<String>) -> Error {
    Error::new(codes::BINDING_NO_BACKEND, message)
}

/// Returns the stable configuration hash over the profile, device tree,
/// PDO/channel map, bindings, and routes (IO-Routing-and-Backends §7).
fn configuration_hash(info: &ModuleInfo) -> String {
    let mut text = String::new();
    if let Some(profile) = &info.profile {
        text.push_str("profile\0");
        text.push_str(&profile.digest);
        text.push('\0');
    }
    for route in &info.routes {
        text.push_str("route\0");
        push_endpoint(&mut text, &route.source);
        for sink in &route.sinks {
            text.push_str("sink\0");
            push_endpoint(&mut text, sink);
        }
    }
    for device in &info.devices {
        text.push_str("device\0");
        text.push_str(&device.name);
        text.push('\0');
        text.push_str(&device.kind);
        text.push('\0');
        text.push_str(device.config.as_deref().unwrap_or(""));
        text.push('\0');
        for channel in &device.channels {
            text.push_str(&channel.name);
            text.push('\0');
            text.push_str(&channel.direction);
            text.push('\0');
            text.push_str(&channel.width_bits.to_string());
            text.push('\0');
        }
    }
    for binding in &info.bindings {
        text.push_str("binding\0");
        text.push_str(&binding.got_id.to_hex());
        text.push('\0');
        text.push_str(&binding.area.to_string());
        text.push('\0');
        text.push_str(&binding.offset_bits.to_string());
        text.push('\0');
        text.push_str(&binding.width_bits.to_string());
        text.push('\0');
        text.push_str(binding.device.as_deref().unwrap_or(""));
        text.push('\0');
        text.push('\n');
    }
    Digest32::from_bytes(stc_got::sha256::sha256(text.as_bytes())).to_hex()
}

fn push_endpoint(text: &mut String, endpoint: &RouteEndpoint) {
    match endpoint {
        RouteEndpoint::PhysicalInput {
            device,
            channel,
            width_bits,
        } => {
            text.push_str("physical-input\0");
            text.push_str(device);
            text.push('\0');
            text.push_str(channel);
            text.push('\0');
            text.push_str(&width_bits.to_string());
        }
        RouteEndpoint::PhysicalOutput {
            device,
            channel,
            width_bits,
        } => {
            text.push_str("physical-output\0");
            text.push_str(device);
            text.push('\0');
            text.push_str(channel);
            text.push('\0');
            text.push_str(&width_bits.to_string());
        }
        RouteEndpoint::PlcInput {
            area,
            offset_bits,
            width_bits,
        }
        | RouteEndpoint::PlcOutput {
            area,
            offset_bits,
            width_bits,
        } => {
            text.push_str("plc\0");
            text.push(area_char(*area));
            text.push('\0');
            text.push_str(&offset_bits.to_string());
            text.push('\0');
            text.push_str(&width_bits.to_string());
        }
    }
    text.push('\0');
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use stc_got::GotId;

    use super::*;
    use crate::io_mock::{MockEthercatBackend, MockWire};
    use crate::module_info::{DirectBinding, RouteInfo};

    const MOCK_CONFIG: &str = r#"{
        "syncs": [
            {
                "index": 1,
                "direction": "input",
                "watchdog": "disable",
                "pdos": [{
                    "index": "0x1a00",
                    "entries": [
                        {"index": "0x3001", "subindex": 1, "bits": 8},
                        {"index": "0x3001", "subindex": 2, "bits": 8}
                    ]
                }]
            },
            {
                "index": 2,
                "direction": "output",
                "watchdog": "enable",
                "pdos": [{
                    "index": "0x1600",
                    "entries": [
                        {"index": "0x3101", "subindex": 1, "bits": 8},
                        {"index": "0x3101", "subindex": 2, "bits": 8}
                    ]
                }]
            }
        ],
        "pdo-channels": [
            {"name": "IN0", "entry": {"index": "0x3001", "subindex": 1}, "bit": 0},
            {"name": "IN1", "entry": {"index": "0x3001", "subindex": 2}, "bit": 0},
            {"name": "OUT0", "entry": {"index": "0x3101", "subindex": 1}, "bit": 0},
            {"name": "OUT1", "entry": {"index": "0x3101", "subindex": 2}, "bit": 0}
        ]
    }"#;

    fn channel(name: &str, direction: &str, width_bits: u32) -> ChannelInfo {
        ChannelInfo {
            name: name.into(),
            direction: direction.into(),
            width_bits,
        }
    }

    fn diewu(kind: &str, config: Option<&str>, channels: Vec<ChannelInfo>) -> DeviceInfo {
        DeviceInfo {
            name: "diewu".into(),
            kind: kind.into(),
            config: config.map(str::to_owned),
            channels,
        }
    }

    fn all_channels() -> Vec<ChannelInfo> {
        vec![
            channel("IN0", "input", 8),
            channel("IN1", "input", 8),
            channel("OUT0", "output", 8),
            channel("OUT1", "output", 8),
        ]
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
            device: Some(device.into()),
            channel: Some(BindingChannel::Name(channel.into())),
            direction: Some(direction.into()),
        }
    }

    fn module(
        devices: Vec<DeviceInfo>,
        bindings: Vec<DirectBinding>,
        routes: Vec<RouteInfo>,
        inputs_bytes: u64,
        outputs_bytes: u64,
    ) -> ModuleInfo {
        ModuleInfo {
            inputs_bytes,
            outputs_bytes,
            input_defaults: vec![0; usize::try_from(inputs_bytes).unwrap_or(0)],
            bindings,
            devices,
            routes,
            ..ModuleInfo::default()
        }
    }

    fn physical(device: &str, channel: &str, width_bits: u64, input: bool) -> RouteEndpoint {
        if input {
            RouteEndpoint::PhysicalInput {
                device: device.into(),
                channel: channel.into(),
                width_bits,
            }
        } else {
            RouteEndpoint::PhysicalOutput {
                device: device.into(),
                channel: channel.into(),
                width_bits,
            }
        }
    }

    fn plc(area: u8, offset_bits: u64, width_bits: u64) -> RouteEndpoint {
        if area == 1 {
            RouteEndpoint::PlcInput {
                area,
                offset_bits,
                width_bits,
            }
        } else {
            RouteEndpoint::PlcOutput {
                area,
                offset_bits,
                width_bits,
            }
        }
    }

    fn route(source: RouteEndpoint, sinks: Vec<RouteEndpoint>) -> RouteInfo {
        RouteInfo { source, sinks }
    }

    fn ethercat_registry(wire: &MockWire) -> BackendRegistry {
        let mut registry = BackendRegistry::new();
        for kind in ["ethercat", "ethercat-status"] {
            let wire = wire.clone();
            registry.register(kind, "ethercat", move || {
                Box::new(MockEthercatBackend::with_wire(wire.clone()))
            });
        }
        registry
    }

    fn two_backend_registry(ethercat: &MockWire, other: &MockWire) -> BackendRegistry {
        let mut registry = BackendRegistry::new();
        let wire = ethercat.clone();
        registry.register("ethercat", "ethercat", move || {
            Box::new(MockEthercatBackend::with_wire(wire.clone()))
        });
        let wire = other.clone();
        registry.register("mock2", "mock2", move || {
            Box::new(MockEthercatBackend::with_wire(wire.clone()))
        });
        registry
    }

    #[test]
    fn process_image_is_little_endian_and_area_relative() {
        let mut image = ProcessImage::new(ImageLayout {
            inputs_bytes: 2,
            outputs_bytes: 1,
            markers_bytes: 1,
        });
        assert!(image.write_bits(1, 3, 5, 0b1_0110));
        assert_eq!(image.bytes()[0], 0b1011_0000);
        assert_eq!(image.read_bits(1, 3, 5), Some(0b1_0110));
        assert_eq!(image.read_bits(1, 13, 5), None);
        assert!(image.write_bits(2, 0, 8, 0x5A));
        assert_eq!(image.area_bytes(2), Some(&[0x5A][..]));
        assert!(image.write_bits(3, 7, 1, 1));
        assert_eq!(image.bytes()[3], 0x80);
        assert_eq!(image.read_bits(2, 0, 52), None);
    }

    #[test]
    fn physical_routes_fan_out_and_pass_through_in_one_scan() {
        let wire = MockWire::new();
        let registry = ethercat_registry(&wire);
        let devices = vec![diewu("ethercat", Some(MOCK_CONFIG), all_channels())];
        let bindings = vec![
            binding(1, 1, 0, 8, "diewu", "IN0", "input"),
            binding(2, 1, 8, 8, "diewu", "IN1", "input"),
            binding(3, 2, 0, 8, "diewu", "OUT0", "output"),
        ];
        let routes = vec![route(
            physical("diewu", "IN0", 8, true),
            vec![plc(1, 16, 8), physical("diewu", "OUT1", 8, false)],
        )];
        let info = module(devices, bindings, routes, 3, 1);
        let mut coordinator = Coordinator::cold_start(&info, &registry).expect("cold start");

        wire.set_input("diewu", "IN0", 0x5A);
        coordinator.input_boundary();
        assert_eq!(coordinator.image().read_bits(1, 0, 8), Some(0x5A));
        assert_eq!(coordinator.image().read_bits(1, 16, 8), Some(0x5A));
        coordinator.after_scan();
        assert_eq!(wire.output("diewu", "OUT1"), Some(0x5A));
        assert_eq!(wire.output("diewu", "OUT0"), Some(0));

        coordinator.image_mut().write_bits(2, 0, 8, 0x33);
        coordinator.after_scan();
        assert_eq!(wire.output("diewu", "OUT0"), Some(0x33));

        let reports = coordinator.reports();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].name, "ethercat");
        assert_eq!(reports[0].devices, 1);
        assert_eq!(reports[0].state, BackendState::Running);
        assert_eq!(reports[0].faults, 0);
        assert!(!reports[0].config_hash.is_empty());
    }

    #[test]
    fn plc_to_physical_route_publishes_the_source_range() {
        let wire = MockWire::new();
        let registry = ethercat_registry(&wire);
        let devices = vec![diewu("ethercat", Some(MOCK_CONFIG), all_channels())];
        let routes = vec![route(
            plc(2, 0, 8),
            vec![physical("diewu", "OUT1", 8, false)],
        )];
        let info = module(devices, Vec::new(), routes, 1, 1);
        let mut coordinator = Coordinator::cold_start(&info, &registry).expect("cold start");
        coordinator.image_mut().write_bits(2, 0, 8, 0x77);
        coordinator.after_scan();
        assert_eq!(wire.output("diewu", "OUT1"), Some(0x77));
    }

    #[test]
    fn plc_to_plc_route_has_exactly_one_scan_of_delay() {
        let mut info = module(
            Vec::new(),
            Vec::new(),
            vec![route(plc(2, 0, 8), vec![plc(1, 8, 8)])],
            2,
            1,
        );
        let mut coordinator =
            Coordinator::cold_start(&info, &BackendRegistry::new()).expect("cold start");
        coordinator.image_mut().write_bits(2, 0, 8, 0x44);
        coordinator.input_boundary();
        assert_eq!(coordinator.image().read_bits(1, 8, 8), Some(0));
        coordinator.after_scan();
        assert_eq!(coordinator.image().read_bits(1, 8, 8), Some(0));
        coordinator.input_boundary();
        assert_eq!(coordinator.image().read_bits(1, 8, 8), Some(0x44));

        info.routes = vec![route(plc(2, 0, 8), vec![plc(1, 16, 8)])];
        info.inputs_bytes = 3;
        let changed = Coordinator::cold_start(&info, &BackendRegistry::new()).expect("cold start");
        assert_ne!(changed.config_hash(), coordinator.config_hash());
    }

    #[test]
    fn fan_in_is_rejected_at_cold_start() {
        let info = module(
            Vec::new(),
            Vec::new(),
            vec![
                route(plc(2, 0, 8), vec![plc(1, 8, 8)]),
                route(plc(2, 16, 8), vec![plc(1, 8, 8)]),
            ],
            3,
            3,
        );
        let error = Coordinator::cold_start(&info, &BackendRegistry::new())
            .err()
            .expect("fan-in is rejected");
        assert_eq!(error.code, codes::BINDING_CONFLICT);
    }

    #[test]
    fn binding_and_route_cannot_claim_the_same_physical_output() {
        let devices = vec![diewu("ethercat", Some(MOCK_CONFIG), all_channels())];
        let bindings = vec![
            binding(1, 1, 0, 8, "diewu", "IN0", "input"),
            binding(2, 2, 0, 8, "diewu", "OUT0", "output"),
        ];
        let routes = vec![route(
            physical("diewu", "IN0", 8, true),
            vec![physical("diewu", "OUT0", 8, false)],
        )];
        let info = module(devices, bindings, routes, 1, 1);
        let error = Coordinator::cold_start(&info, &ethercat_registry(&MockWire::new()))
            .err()
            .expect("conflict");
        assert_eq!(error.code, codes::BINDING_CONFLICT);
    }

    #[test]
    fn unregistered_kind_and_missing_configuration_fail_closed() {
        let mut info = module(
            vec![diewu("modbus", Some("{}"), Vec::new())],
            Vec::new(),
            Vec::new(),
            0,
            0,
        );
        let error = Coordinator::cold_start(&info, &BackendRegistry::new())
            .err()
            .expect("unregistered kind");
        assert_eq!(error.code, codes::BINDING_NO_BACKEND);

        info.devices = vec![diewu("ethercat", None, Vec::new())];
        let error = Coordinator::cold_start(&info, &BackendRegistry::with_defaults())
            .err()
            .expect("missing configuration");
        assert_eq!(error.code, codes::BINDING_INVALID);
    }

    #[test]
    fn route_endpoint_must_resolve_and_match_width() {
        let devices = vec![diewu("ethercat", Some(MOCK_CONFIG), all_channels())];
        let bindings = vec![binding(1, 1, 0, 8, "diewu", "IN0", "input")];
        let cases = [
            route(physical("diewu", "NOPE", 8, true), vec![plc(1, 8, 8)]),
            route(physical("diewu", "IN0", 8, true), vec![plc(1, 8, 16)]),
            route(physical("diewu", "OUT0", 8, false), vec![plc(1, 8, 8)]),
        ];
        for routes in cases {
            let info = module(devices.clone(), bindings.clone(), vec![routes], 2, 1);
            let error = Coordinator::cold_start(&info, &ethercat_registry(&MockWire::new()))
                .err()
                .expect("invalid route endpoint");
            assert_eq!(error.code, codes::BINDING_INVALID, "{}", error.message);
        }
    }

    #[derive(Clone, Default)]
    struct Calls(Arc<Mutex<Vec<String>>>);

    impl Calls {
        fn push(&self, call: String) {
            self.0.lock().expect("calls").push(call);
        }

        fn take(&self) -> Vec<String> {
            std::mem::take(&mut *self.0.lock().expect("calls"))
        }
    }

    struct RecordingBackend {
        label: String,
        calls: Calls,
        fail_start: bool,
    }

    impl DeviceBackend for RecordingBackend {
        fn cold_start(&mut self, _config: &BackendConfig) -> Result<()> {
            self.calls.push(format!("start:{}", self.label));
            if self.fail_start {
                return Err(Error::new(codes::BINDING_INVALID, "cold start failed"));
            }
            Ok(())
        }

        fn latch_inputs(&mut self, _image: &mut ProcessImage) -> Result<()> {
            self.calls.push(format!("latch:{}", self.label));
            Ok(())
        }

        fn publish_outputs(&mut self, _image: &ProcessImage) -> Result<()> {
            self.calls.push(format!("publish:{}", self.label));
            Ok(())
        }

        fn status(&self) -> BackendStatus {
            BackendStatus::default()
        }

        fn shutdown(&mut self) -> Result<()> {
            self.calls.push(format!("stop:{}", self.label));
            Ok(())
        }
    }

    fn recording_registry(calls: &Calls, entries: &[(&str, &str, bool)]) -> BackendRegistry {
        let mut registry = BackendRegistry::new();
        for (kind, instance, fail_start) in entries {
            let calls = calls.clone();
            let label = (*instance).to_owned();
            let fail_start = *fail_start;
            registry.register(*kind, *instance, move || {
                Box::new(RecordingBackend {
                    label: label.clone(),
                    calls: calls.clone(),
                    fail_start,
                })
            });
        }
        registry
    }

    fn config_device(name: &str, kind: &str) -> DeviceInfo {
        DeviceInfo {
            name: name.into(),
            kind: kind.into(),
            config: Some("{}".into()),
            channels: Vec::new(),
        }
    }

    #[test]
    fn cold_start_latch_publish_follow_declaration_order_and_shutdown_reverses() {
        let calls = Calls::default();
        let registry = recording_registry(
            &calls,
            &[("kindb", "instb", false), ("kinda", "insta", false)],
        );
        let info = module(
            vec![config_device("one", "kindb"), config_device("two", "kinda")],
            Vec::new(),
            Vec::new(),
            0,
            0,
        );
        let mut coordinator = Coordinator::cold_start(&info, &registry).expect("cold start");
        assert_eq!(calls.take(), ["start:instb", "start:insta"]);
        coordinator.input_boundary();
        assert_eq!(calls.take(), ["latch:instb", "latch:insta"]);
        coordinator.after_scan();
        assert_eq!(calls.take(), ["publish:instb", "publish:insta"]);
        coordinator.shutdown().expect("shutdown");
        assert_eq!(calls.take(), ["stop:insta", "stop:instb"]);
        let reports = coordinator.reports();
        assert_eq!(reports[0].name, "instb");
        assert_eq!(reports[1].name, "insta");
    }

    #[test]
    fn a_partial_cold_start_shuts_down_started_instances_in_reverse() {
        let calls = Calls::default();
        let registry = recording_registry(&calls, &[("good", "good", false), ("bad", "bad", true)]);
        let info = module(
            vec![config_device("one", "good"), config_device("two", "bad")],
            Vec::new(),
            Vec::new(),
            0,
            0,
        );
        let error = Coordinator::cold_start(&info, &registry)
            .err()
            .expect("cold start fails");
        assert_eq!(error.code, codes::BINDING_INVALID);
        assert_eq!(
            calls.take(),
            ["start:good", "start:bad", "stop:good"],
            "the failed instance is never started or stopped"
        );
    }

    #[test]
    fn one_backend_fault_does_not_touch_another_backends_data() {
        let ethercat = MockWire::new();
        let other = MockWire::new();
        let registry = two_backend_registry(&ethercat, &other);
        let devices = vec![
            diewu("ethercat", Some(MOCK_CONFIG), all_channels()),
            DeviceInfo {
                name: "other".into(),
                kind: "mock2".into(),
                config: Some("{}".into()),
                channels: vec![channel("IN0", "input", 8)],
            },
        ];
        let bindings = vec![
            binding(1, 1, 0, 8, "diewu", "IN0", "input"),
            binding(2, 1, 8, 8, "other", "IN0", "input"),
        ];
        let info = module(devices, bindings, Vec::new(), 2, 0);
        let mut coordinator = Coordinator::cold_start(&info, &registry).expect("cold start");
        ethercat.set_input("diewu", "IN0", 0x11);
        other.set_input("other", "IN0", 0x22);

        ethercat.set_fail_latch(true);
        coordinator.input_boundary();
        assert_eq!(coordinator.image().read_bits(1, 0, 8), Some(0));
        assert_eq!(coordinator.image().read_bits(1, 8, 8), Some(0x22));
        let reports = coordinator.reports();
        assert_eq!(reports[0].state, BackendState::Faulted);
        assert_eq!(reports[0].faults, 1);
        assert_eq!(reports[1].state, BackendState::Running);
        assert_eq!(reports[1].faults, 0);

        ethercat.set_fail_latch(false);
        coordinator.input_boundary();
        assert_eq!(coordinator.image().read_bits(1, 0, 8), Some(0x11));
        assert_eq!(coordinator.image().read_bits(1, 8, 8), Some(0x22));
        let reports = coordinator.reports();
        assert_eq!(reports[0].state, BackendState::Running);
        assert_eq!(reports[0].faults, 1);
        assert_eq!(reports[1].faults, 0);
    }

    #[test]
    fn cross_backend_route_reads_one_shared_image() {
        let ethercat = MockWire::new();
        let other = MockWire::new();
        let registry = two_backend_registry(&ethercat, &other);
        let devices = vec![
            diewu("ethercat", Some(MOCK_CONFIG), all_channels()),
            DeviceInfo {
                name: "other".into(),
                kind: "mock2".into(),
                config: Some("{}".into()),
                channels: vec![channel("IN0", "input", 8)],
            },
        ];
        let bindings = vec![binding(1, 1, 0, 8, "other", "IN0", "input")];
        let routes = vec![route(
            physical("other", "IN0", 8, true),
            vec![physical("diewu", "OUT1", 8, false)],
        )];
        let info = module(devices, bindings, routes, 1, 1);
        let mut coordinator = Coordinator::cold_start(&info, &registry).expect("cold start");
        other.set_input("other", "IN0", 0x9C);
        coordinator.input_boundary();
        coordinator.after_scan();
        assert_eq!(ethercat.output("diewu", "OUT1"), Some(0x9C));
    }

    #[test]
    fn unbound_inputs_receive_global_defaults_and_unbound_outputs_are_cleared() {
        let devices = vec![diewu("ethercat", Some(MOCK_CONFIG), all_channels())];
        let mut info = module(devices, Vec::new(), Vec::new(), 2, 1);
        info.backend = IoBackend::Ethercat;
        info.input_defaults = vec![0xAA, 0xBB];
        let mut coordinator = Coordinator::cold_start(&info, &ethercat_registry(&MockWire::new()))
            .expect("cold start");
        coordinator.input_boundary();
        assert_eq!(coordinator.image().read_bits(1, 0, 16), Some(0xBBAA));
        coordinator.image_mut().write_bits(2, 0, 8, 0xFF);
        coordinator.after_scan();
        assert_eq!(coordinator.image().read_bits(2, 0, 8), Some(0));
    }
}
