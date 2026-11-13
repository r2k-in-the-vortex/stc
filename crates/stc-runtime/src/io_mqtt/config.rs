//! MQTT device and status profile schema, validation, and channel mapping
//! (WL-15 phase 2).
//!
//! The compiler is protocol-agnostic: it validates the manifest and profile
//! structure, device/channel declarations, bindings, and routes, and embeds
//! the merged device configuration as JSON text. This module is the runtime
//! half: it interprets an `mqtt` or `mqtt-status` device's JSON configuration,
//! rejects keys that belong to another backend, and resolves every declared
//! channel to a topic and payload encoding.
//!
//! Validation is all-or-nothing. A malformed device, an unmapped channel, an
//! unsupported encoding/width pair, a bad topic, or unreadable TLS/credential
//! material fails cold start with a `STC-E-IO-MQTT-*` code and no partial
//! state. File material is resolved against the supplied base directory (the
//! manifest directory at run time) and never echoed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use stc_got::error::Error;
use stc_got::json::{self, Json};

use crate::coordinator::{BackendConfig, BackendDeviceConfig};

use super::packet::QoS;
use super::payload::{self, ByteOrder, Encoding};
use super::session::SessionConfig;
use super::topic;

/// Stable diagnostic codes owned by the MQTT backend.
pub mod codes {
    /// The device or channel configuration is invalid.
    pub const MQTT_INVALID_CONFIG: &str = "STC-E-IO-MQTT-001";
    /// A declared channel cannot be mapped to a topic or image range.
    pub const MQTT_CHANNEL_RESOLVE: &str = "STC-E-IO-MQTT-002";
    /// TLS material is missing, unreadable, or not private.
    pub const MQTT_TLS_MATERIAL: &str = "STC-E-IO-MQTT-003";
    /// Credential material is missing, unreadable, or not private.
    pub const MQTT_CREDENTIAL_MATERIAL: &str = "STC-E-IO-MQTT-004";
    /// A declared channel has no image binding (diagnostic only).
    pub const MQTT_UNBOUND_CHANNEL: &str = "STC-W-IO-MQTT-001";
    /// A client fault: disconnected, retrying, inputs stale (diagnostic only).
    pub const MQTT_CLIENT_FAULT: &str = "STC-W-IO-MQTT-002";
}

/// Device kind of an MQTT data device.
pub const MQTT_KIND: &str = "mqtt";

/// Device kind of the optional read-only status device.
pub const MQTT_STATUS_KIND: &str = "mqtt-status";

/// Default plaintext broker port.
pub const DEFAULT_PORT: u16 = 1883;

/// Default TLS broker port.
pub const DEFAULT_TLS_PORT: u16 = 8883;

/// Default keepalive in seconds (Q5).
pub const DEFAULT_KEEP_ALIVE_SECS: u16 = 30;

/// Default connect/CONNACK budget.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Default stale timeout (Q6).
pub const DEFAULT_STALE_TIMEOUT: Duration = Duration::from_secs(5);

/// Default reconnect ladder (Q5/Q7); the last entry repeats.
pub const DEFAULT_BACKOFF_MS: [u64; 5] = [250, 500, 1_000, 2_000, 5_000];

/// Data direction of one MQTT channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Broker message to process image (`SUBSCRIBE`).
    Input,
    /// Process image to broker message (`PUBLISH`).
    Output,
}

impl Direction {
    /// Parses the manifest spelling.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "input" => Some(Self::Input),
            "output" => Some(Self::Output),
            _ => None,
        }
    }

    /// Returns the manifest spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
        }
    }
}

/// TLS configuration of one broker connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsPlan {
    /// CA bundle used to verify the broker certificate.
    pub ca_file: PathBuf,
    /// Server name for certificate verification; the host when omitted.
    pub server_name: Option<String>,
    /// Whether the certificate chain and hostname are verified.
    pub verify: bool,
    /// Client certificate for mutual TLS.
    pub client_cert_file: Option<PathBuf>,
    /// Client private key for mutual TLS.
    pub client_key_file: Option<PathBuf>,
}

/// Credential files for one broker connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialsPlan {
    /// File holding the user name.
    pub username_file: Option<PathBuf>,
    /// File holding the password.
    pub password_file: Option<PathBuf>,
}

/// Last-will message configured for one broker connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastWillPlan {
    /// Will topic; a topic name, never a filter.
    pub topic: String,
    /// UTF-8 will payload bytes.
    pub payload: Vec<u8>,
    /// Will QoS.
    pub qos: QoS,
    /// Will retain flag.
    pub retain: bool,
}

/// One channel mapped to a topic, payload encoding, and image range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MqttChannelPlan {
    /// Declared channel name.
    pub name: String,
    /// Data direction.
    pub direction: Direction,
    /// Subscription filter (input) or topic name (output).
    pub topic: String,
    /// Payload encoding.
    pub encoding: Encoding,
    /// Byte order of numeric and raw encodings.
    pub byte_order: ByteOrder,
    /// QoS of the message.
    pub qos: QoS,
    /// Retain flag (publish channels).
    pub retain: bool,
    /// Bound image range; `None` when the channel has no binding.
    pub image: Option<crate::coordinator::ImageRange>,
    /// Declared channel width in bits.
    pub width_bits: u32,
}

/// One validated MQTT data device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MqttDevicePlan {
    /// Device name.
    pub name: String,
    /// Broker host name or literal address.
    pub host: String,
    /// Broker port.
    pub port: u16,
    /// Optional TLS configuration.
    pub tls: Option<TlsPlan>,
    /// Configured client identifier; the broker assigns one when omitted.
    pub client_id: Option<String>,
    /// Session timing, keepalive, and reconnect ladder.
    pub session: SessionConfig,
    /// MQTT 5 clean start.
    pub clean_start: bool,
    /// MQTT 5 session expiry interval in seconds.
    pub session_expiry: u32,
    /// Input staleness timeout (Q6).
    pub stale_timeout: Duration,
    /// Optional credential files.
    pub credentials: Option<CredentialsPlan>,
    /// Optional last will.
    pub last_will: Option<LastWillPlan>,
    /// Mapped channels in profile order.
    pub channels: Vec<MqttChannelPlan>,
    /// Declared channels that have no image binding.
    pub unbound: Vec<String>,
}

/// One status field of layout 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusField {
    /// Client connected.
    Connected,
    /// Client lifecycle state.
    State,
    /// Completed reconnect attempts.
    Reconnects,
    /// Accepted inbound messages.
    MessagesIn,
    /// Published outbound messages.
    MessagesOut,
    /// Messages rejected on decode or encode.
    Dropped,
    /// Last backend error code.
    LastError,
    /// Last MQTT reason code.
    Reason,
    /// Inputs are the last received values, not current.
    Stale,
    /// Successful cycle boundaries since start.
    Exchanges,
    /// Fault episodes since start.
    Faults,
}

impl StatusField {
    /// Returns the channel-name suffix.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::State => "state",
            Self::Reconnects => "reconnects",
            Self::MessagesIn => "messages_in",
            Self::MessagesOut => "messages_out",
            Self::Dropped => "dropped",
            Self::LastError => "last_error",
            Self::Reason => "reason",
            Self::Stale => "stale",
            Self::Exchanges => "exchanges",
            Self::Faults => "faults",
        }
    }

    /// Returns the layout-1 width in bits.
    #[must_use]
    pub const fn width_bits(self) -> u32 {
        match self {
            Self::Connected | Self::Stale => 1,
            Self::State => 16,
            Self::Reconnects
            | Self::MessagesIn
            | Self::MessagesOut
            | Self::Dropped
            | Self::LastError
            | Self::Reason
            | Self::Exchanges
            | Self::Faults => 32,
        }
    }
}

/// Which client or cycle a status field reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusIndex {
    /// The instance-wide field (`input.*`, `cycle.*`).
    Global,
    /// One data device, by declaration order.
    Client(usize),
}

/// One validated status-device channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusBinding {
    /// Status device name.
    pub device: String,
    /// Status field.
    pub field: StatusField,
    /// Client index or the instance-wide scope.
    pub index: StatusIndex,
    /// Bound image range; `None` when the channel has no binding.
    pub image: Option<crate::coordinator::ImageRange>,
}

/// The validated backend configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MqttBackendPlan {
    /// Data devices in declaration order.
    pub devices: Vec<MqttDevicePlan>,
    /// Status channels across every status device.
    pub status: Vec<StatusBinding>,
}

/// Validates the backend configuration and maps every declared channel.
///
/// `base_dir` resolves relative TLS and credential file paths; the host passes
/// the manifest directory.
pub fn plan(config: &BackendConfig, base_dir: &Path) -> Result<MqttBackendPlan, Error> {
    let mut names = BTreeSet::new();
    for device in &config.devices {
        if !names.insert(device.name.as_str()) {
            return Err(invalid(format!(
                "backend `{}` owns device `{}` more than once",
                config.name, device.name
            )));
        }
    }
    let data_devices = config
        .devices
        .iter()
        .filter(|device| device.kind != MQTT_STATUS_KIND)
        .count();
    let mut devices = Vec::new();
    let mut status = Vec::new();
    for device in &config.devices {
        if device.kind == MQTT_STATUS_KIND {
            status.extend(parse_status_device(device, data_devices)?);
        } else {
            devices.push(parse_device(device, base_dir)?);
        }
    }
    Ok(MqttBackendPlan { devices, status })
}

fn parse_device(device: &BackendDeviceConfig, base_dir: &Path) -> Result<MqttDevicePlan, Error> {
    if device.kind != MQTT_KIND {
        return Err(invalid(format!(
            "device `{}` has kind `{}`; the MQTT backend serves `{MQTT_KIND}` and \
             `{MQTT_STATUS_KIND}`",
            device.name, device.kind
        )));
    }
    let Some(text) = device.config.as_deref() else {
        return Err(invalid(format!(
            "MQTT device `{}` has no profile or inline configuration",
            device.name
        )));
    };
    let root = json::parse(text).map_err(|error| {
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
    reject_foreign_keys(device, object)?;
    const DEVICE_KEYS: &[&str] = &[
        "host",
        "port",
        "client-id",
        "keepalive",
        "clean-start",
        "session-expiry",
        "connect-timeout",
        "reconnect-backoff",
        "stale-timeout",
        "qos",
        "retain",
        "tls",
        "credentials",
        "last-will",
        "channels",
    ];
    reject_unknown_keys(device, object, DEVICE_KEYS, "device")?;

    let host = require_string(object, "host", &device.name)?;
    validate_host(&device.name, &host)?;
    let tls = match object.get("tls") {
        Some(value) => Some(parse_tls(device, value, base_dir)?),
        None => None,
    };
    let port = match object.get("port") {
        Some(value) => {
            let port = number(value).ok_or_else(|| {
                invalid(format!(
                    "MQTT device `{}` `port` must be an integer",
                    device.name
                ))
            })?;
            u16::try_from(port)
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| {
                    invalid(format!(
                        "MQTT device `{}` `port` must be in 1..=65535",
                        device.name
                    ))
                })?
        }
        None if tls.is_some() => DEFAULT_TLS_PORT,
        None => DEFAULT_PORT,
    };

    let keep_alive = match object.get("keepalive") {
        Some(value) => duration_setting(device, "keepalive", value)?,
        None => Duration::from_secs(u64::from(DEFAULT_KEEP_ALIVE_SECS)),
    };
    if keep_alive > Duration::from_secs(u64::from(u16::MAX)) {
        return Err(invalid(format!(
            "MQTT device `{}` `keepalive` must not exceed 65535s",
            device.name
        )));
    }
    let connect_timeout = match object.get("connect-timeout") {
        Some(value) => duration_setting(device, "connect-timeout", value)?,
        None => DEFAULT_CONNECT_TIMEOUT,
    };
    let stale_timeout = match object.get("stale-timeout") {
        Some(value) => duration_setting(device, "stale-timeout", value)?,
        None => DEFAULT_STALE_TIMEOUT,
    };
    let backoff = match object.get("reconnect-backoff") {
        Some(value) => {
            let entries = value.as_array().ok_or_else(|| {
                invalid(format!(
                    "MQTT device `{}` `reconnect-backoff` must be an array of durations",
                    device.name
                ))
            })?;
            if entries.is_empty() || entries.len() > 20 {
                return Err(invalid(format!(
                    "MQTT device `{}` `reconnect-backoff` must have 1..=20 entries",
                    device.name
                )));
            }
            let mut ladder = Vec::with_capacity(entries.len());
            for entry in entries {
                let text = entry.as_str().ok_or_else(|| {
                    invalid(format!(
                        "MQTT device `{}` `reconnect-backoff` entries must be duration strings",
                        device.name
                    ))
                })?;
                let delay = parse_duration(text)
                    .filter(|delay| !delay.is_zero())
                    .ok_or_else(|| {
                        invalid(format!(
                            "MQTT device `{}` `reconnect-backoff` entry `{text}` is not a positive \
                         duration",
                            device.name
                        ))
                    })?;
                ladder.push(delay);
            }
            ladder
        }
        None => DEFAULT_BACKOFF_MS
            .iter()
            .map(|millis| Duration::from_millis(*millis))
            .collect(),
    };
    let session = SessionConfig::new(keep_alive, connect_timeout, backoff)
        .map_err(|error| invalid(format!("MQTT device `{}`: {error}", device.name)))?;

    let clean_start = optional_bool(object, "clean-start", &device.name)?.unwrap_or(true);
    let session_expiry = match object.get("session-expiry") {
        Some(value) => {
            let text = value.as_str().ok_or_else(|| {
                invalid(format!(
                    "MQTT device `{}` `session-expiry` must be a duration string such as \"0s\"",
                    device.name
                ))
            })?;
            let duration = parse_duration(text).ok_or_else(|| {
                invalid(format!(
                    "MQTT device `{}` `session-expiry` must be a duration string such as \"0s\"",
                    device.name
                ))
            })?;
            if duration.subsec_nanos() != 0 {
                return Err(invalid(format!(
                    "MQTT device `{}` `session-expiry` must be a whole number of seconds",
                    device.name
                )));
            }
            u32::try_from(duration.as_secs()).map_err(|_| {
                invalid(format!(
                    "MQTT device `{}` `session-expiry` must fit a 32-bit count of seconds",
                    device.name
                ))
            })?
        }
        None => 0,
    };
    let default_qos = match object.get("qos") {
        Some(value) => parse_qos(value, &device.name, "qos")?,
        None => QoS::AtLeastOnce,
    };
    let default_retain = optional_bool(object, "retain", &device.name)?.unwrap_or(false);
    let credentials = match object.get("credentials") {
        Some(value) => Some(parse_credentials(device, value, base_dir)?),
        None => None,
    };
    let last_will = match object.get("last-will") {
        Some(value) => Some(parse_last_will(device, value)?),
        None => None,
    };

    let channels_value = object.get("channels").ok_or_else(|| {
        invalid(format!(
            "MQTT device `{}` is missing its `channels` array",
            device.name
        ))
    })?;
    let channel_entries = channels_value.as_array().ok_or_else(|| {
        invalid(format!(
            "MQTT device `{}` `channels` must be an array",
            device.name
        ))
    })?;
    let channels = map_channels(device, channel_entries, default_qos, default_retain)?;
    let unbound = channels
        .iter()
        .filter(|channel| channel.image.is_none())
        .map(|channel| channel.name.clone())
        .collect();

    Ok(MqttDevicePlan {
        name: device.name.clone(),
        host,
        port,
        tls,
        client_id: match object.get("client-id") {
            Some(value) => Some(value.as_str().map(str::to_owned).ok_or_else(|| {
                invalid(format!(
                    "MQTT device `{}` `client-id` must be a string",
                    device.name
                ))
            })?),
            None => None,
        },
        session,
        clean_start,
        session_expiry,
        stale_timeout,
        credentials,
        last_will,
        channels,
        unbound,
    })
}

fn map_channels(
    device: &BackendDeviceConfig,
    entries: &[Json],
    default_qos: QoS,
    default_retain: bool,
) -> Result<Vec<MqttChannelPlan>, Error> {
    let mut declared: BTreeMap<&str, &crate::coordinator::BackendChannel> = BTreeMap::new();
    for channel in &device.channels {
        if declared.insert(channel.name.as_str(), channel).is_some() {
            return Err(resolve(format!(
                "device `{}` declares channel `{}` more than once",
                device.name, channel.name
            )));
        }
    }
    let mut seen = BTreeSet::new();
    let mut topics = BTreeSet::new();
    let mut plans = Vec::with_capacity(entries.len());
    for entry in entries {
        let object = entry.as_object().ok_or_else(|| {
            invalid(format!(
                "MQTT device `{}` has a channel entry that is not a table",
                device.name
            ))
        })?;
        const CHANNEL_KEYS: &[&str] = &[
            "name",
            "topic",
            "direction",
            "payload-encoding",
            "byte-order",
            "qos",
            "retain",
        ];
        reject_unknown_keys(device, object, CHANNEL_KEYS, "channel")?;
        let name = require_string(object, "name", &device.name)?;
        let Some(channel) = declared.get(name.as_str()) else {
            return Err(resolve(format!(
                "MQTT device `{}` profile maps undeclared channel `{name}`",
                device.name
            )));
        };
        if !seen.insert(name.clone()) {
            return Err(resolve(format!(
                "MQTT device `{}` channel `{name}` is mapped more than once",
                device.name
            )));
        }
        let direction = match object.get("direction") {
            Some(value) => {
                let text = value.as_str().ok_or_else(|| {
                    invalid(format!(
                        "MQTT device `{}` channel `{name}` `direction` must be a string",
                        device.name
                    ))
                })?;
                Direction::parse(text).ok_or_else(|| {
                    invalid(format!(
                        "MQTT device `{}` channel `{name}` has direction `{text}`; expected \
                         `input` or `output`",
                        device.name
                    ))
                })?
            }
            None => Direction::parse(&channel.direction).ok_or_else(|| {
                resolve(format!(
                    "MQTT device `{}` channel `{name}` has direction `{}`; expected input or \
                     output",
                    device.name, channel.direction
                ))
            })?,
        };
        if direction.as_str() != channel.direction {
            return Err(resolve(format!(
                "MQTT device `{}` channel `{name}` is declared `{}` but mapped as `{}`",
                device.name,
                channel.direction,
                direction.as_str()
            )));
        }
        let topic_text = require_string(object, "topic", &device.name)?;
        match direction {
            Direction::Input if !topic::is_valid_topic_filter(&topic_text) => {
                return Err(resolve(format!(
                    "MQTT device `{}` channel `{name}` subscription `{topic_text}` is not a valid \
                     topic filter",
                    device.name
                )));
            }
            Direction::Output if !topic::is_valid_topic_name(&topic_text) => {
                return Err(resolve(format!(
                    "MQTT device `{}` channel `{name}` topic `{topic_text}` is not a valid topic \
                     name (no wildcards or empty levels)",
                    device.name
                )));
            }
            _ => {}
        }
        if !topics.insert((direction.as_str().to_owned(), topic_text.clone())) {
            return Err(invalid(format!(
                "MQTT device `{}` maps `{topic_text}` more than once in the `{}` direction",
                device.name,
                direction.as_str()
            )));
        }
        let encoding_text = require_string(object, "payload-encoding", &device.name)?;
        let encoding = Encoding::parse(&encoding_text).ok_or_else(|| {
            invalid(format!(
                "MQTT device `{}` channel `{name}` has unknown payload encoding `{encoding_text}`",
                device.name
            ))
        })?;
        let byte_order = match object.get("byte-order") {
            Some(value) => {
                let text = value.as_str().ok_or_else(|| {
                    invalid(format!(
                        "MQTT device `{}` channel `{name}` `byte-order` must be a string",
                        device.name
                    ))
                })?;
                ByteOrder::parse(text).ok_or_else(|| {
                    invalid(format!(
                        "MQTT device `{}` channel `{name}` has byte order `{text}`; expected \
                         `big` or `little`",
                        device.name
                    ))
                })?
            }
            None => ByteOrder::default(),
        };
        payload::encode(encoding, channel.width_bits, byte_order, 0).map_err(|error| {
            invalid(format!(
                "MQTT device `{}` channel `{name}`: {error}",
                device.name
            ))
        })?;
        let qos = match object.get("qos") {
            Some(value) => parse_qos(value, &device.name, &name)?,
            None => default_qos,
        };
        let retain = match (direction, optional_bool(object, "retain", &device.name)?) {
            (Direction::Input, Some(true)) => {
                return Err(invalid(format!(
                    "MQTT device `{}` channel `{name}` is an input; `retain` applies to publishes",
                    device.name
                )));
            }
            (Direction::Input, _) => false,
            (Direction::Output, Some(retain)) => retain,
            (Direction::Output, None) => default_retain,
        };
        plans.push(MqttChannelPlan {
            name,
            direction,
            topic: topic_text,
            encoding,
            byte_order,
            qos,
            retain,
            image: channel.image,
            width_bits: channel.width_bits,
        });
    }
    for channel in &device.channels {
        if !seen.contains(&channel.name) {
            return Err(resolve(format!(
                "MQTT device `{}` declares channel `{}` but the profile does not map it",
                device.name, channel.name
            )));
        }
        if let Some(image) = channel.image
            && image.width_bits != u64::from(channel.width_bits)
        {
            return Err(resolve(format!(
                "MQTT device `{}` channel `{}` is {} bits but its image range is {} bits",
                device.name, channel.name, channel.width_bits, image.width_bits
            )));
        }
        if let Some(image) = channel.image
            && image.area != direction_area(&channel.direction)
        {
            return Err(resolve(format!(
                "MQTT device `{}` channel `{}` has direction `{}` but image area {}",
                device.name, channel.name, channel.direction, image.area
            )));
        }
    }
    Ok(plans)
}

fn direction_area(direction: &str) -> u8 {
    if direction == "input" { 1 } else { 2 }
}

fn parse_status_device(
    device: &BackendDeviceConfig,
    data_devices: usize,
) -> Result<Vec<StatusBinding>, Error> {
    if let Some(text) = device.config.as_deref() {
        let root = json::parse(text).map_err(|error| {
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
        reject_unknown_keys(device, object, &["layout"], "status device")?;
        if let Some(layout) = object.get("layout")
            && number(layout) != Some(1)
        {
            return Err(invalid(format!(
                "status device `{}` has an unknown layout; expected 1",
                device.name
            )));
        }
    }
    let mut names = BTreeSet::new();
    let mut bindings = Vec::new();
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
        let Some((index, field)) = status_field(&channel.name) else {
            return Err(invalid(format!(
                "status device `{}` names unknown status channel `{}`",
                device.name, channel.name
            )));
        };
        if let StatusIndex::Client(client) = index
            && client >= data_devices
        {
            return Err(invalid(format!(
                "status device `{}` channel `{}` names client{client}, but only {data_devices} \
                 MQTT data devices are declared",
                device.name, channel.name
            )));
        }
        if channel.width_bits != field.width_bits() {
            return Err(invalid(format!(
                "status device `{}` channel `{}` is {} bits; layout 1 uses {} bits",
                device.name,
                channel.name,
                channel.width_bits,
                field.width_bits()
            )));
        }
        bindings.push(StatusBinding {
            device: device.name.clone(),
            field,
            index,
            image: channel.image,
        });
    }
    Ok(bindings)
}

fn status_field(name: &str) -> Option<(StatusIndex, StatusField)> {
    if name == "input.stale" {
        return Some((StatusIndex::Global, StatusField::Stale));
    }
    if name == "cycle.exchanges" {
        return Some((StatusIndex::Global, StatusField::Exchanges));
    }
    if name == "cycle.faults" {
        return Some((StatusIndex::Global, StatusField::Faults));
    }
    let rest = name.strip_prefix("client")?;
    let (index, field) = rest.split_once('.')?;
    let index: usize = index.parse().ok()?;
    let field = match field {
        "connected" => StatusField::Connected,
        "state" => StatusField::State,
        "reconnects" => StatusField::Reconnects,
        "messages_in" => StatusField::MessagesIn,
        "messages_out" => StatusField::MessagesOut,
        "dropped" => StatusField::Dropped,
        "last_error" => StatusField::LastError,
        "reason" => StatusField::Reason,
        _ => return None,
    };
    Some((StatusIndex::Client(index), field))
}

fn parse_tls(
    device: &BackendDeviceConfig,
    value: &Json,
    base_dir: &Path,
) -> Result<TlsPlan, Error> {
    let object = value.as_object().ok_or_else(|| {
        invalid(format!(
            "MQTT device `{}` `tls` must be a table",
            device.name
        ))
    })?;
    const TLS_KEYS: &[&str] = &[
        "ca-file",
        "server-name",
        "verify",
        "client-cert-file",
        "client-key-file",
    ];
    reject_unknown_keys(device, object, TLS_KEYS, "tls")?;
    let ca_file = resolve_file(
        device,
        &require_string(object, "ca-file", &device.name)?,
        base_dir,
        codes::MQTT_TLS_MATERIAL,
        "TLS CA file",
    )?;
    let server_name = match object.get("server-name") {
        Some(value) => Some(
            value
                .as_str()
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    invalid(format!(
                        "MQTT device `{}` `tls.server-name` must be a non-empty string",
                        device.name
                    ))
                })?
                .to_owned(),
        ),
        None => None,
    };
    let verify = optional_bool(object, "verify", &device.name)?.unwrap_or(true);
    let client_cert_file = match object.get("client-cert-file") {
        Some(value) => {
            let text = value.as_str().ok_or_else(|| {
                invalid(format!(
                    "MQTT device `{}` `tls.client-cert-file` must be a string",
                    device.name
                ))
            })?;
            Some(resolve_file(
                device,
                text,
                base_dir,
                codes::MQTT_TLS_MATERIAL,
                "TLS client certificate",
            )?)
        }
        None => None,
    };
    let client_key_file = match object.get("client-key-file") {
        Some(value) => {
            let text = value.as_str().ok_or_else(|| {
                invalid(format!(
                    "MQTT device `{}` `tls.client-key-file` must be a string",
                    device.name
                ))
            })?;
            Some(resolve_file(
                device,
                text,
                base_dir,
                codes::MQTT_TLS_MATERIAL,
                "TLS client key",
            )?)
        }
        None => None,
    };
    if client_cert_file.is_some() != client_key_file.is_some() {
        return Err(tls(format!(
            "MQTT device `{}` must configure both `client-cert-file` and `client-key-file` or \
             neither",
            device.name
        )));
    }
    Ok(TlsPlan {
        ca_file,
        server_name,
        verify,
        client_cert_file,
        client_key_file,
    })
}

fn parse_credentials(
    device: &BackendDeviceConfig,
    value: &Json,
    base_dir: &Path,
) -> Result<CredentialsPlan, Error> {
    let object = value.as_object().ok_or_else(|| {
        invalid(format!(
            "MQTT device `{}` `credentials` must be a table",
            device.name
        ))
    })?;
    reject_unknown_keys(
        device,
        object,
        &["username-file", "password-file"],
        "credentials",
    )?;
    let username_file = match object.get("username-file") {
        Some(value) => {
            let text = value.as_str().ok_or_else(|| {
                invalid(format!(
                    "MQTT device `{}` `credentials.username-file` must be a string",
                    device.name
                ))
            })?;
            Some(resolve_file(
                device,
                text,
                base_dir,
                codes::MQTT_CREDENTIAL_MATERIAL,
                "user name file",
            )?)
        }
        None => None,
    };
    let password_file = match object.get("password-file") {
        Some(value) => {
            let text = value.as_str().ok_or_else(|| {
                invalid(format!(
                    "MQTT device `{}` `credentials.password-file` must be a string",
                    device.name
                ))
            })?;
            Some(resolve_file(
                device,
                text,
                base_dir,
                codes::MQTT_CREDENTIAL_MATERIAL,
                "password file",
            )?)
        }
        None => None,
    };
    if username_file.is_none() && password_file.is_none() {
        return Err(invalid(format!(
            "MQTT device `{}` `credentials` configures neither a user name nor a password file",
            device.name
        )));
    }
    if password_file.is_some() && username_file.is_none() {
        return Err(invalid(format!(
            "MQTT device `{}` configures a password file without a user name file",
            device.name
        )));
    }
    Ok(CredentialsPlan {
        username_file,
        password_file,
    })
}

fn parse_last_will(device: &BackendDeviceConfig, value: &Json) -> Result<LastWillPlan, Error> {
    let object = value.as_object().ok_or_else(|| {
        invalid(format!(
            "MQTT device `{}` `last-will` must be a table",
            device.name
        ))
    })?;
    reject_unknown_keys(
        device,
        object,
        &["topic", "payload-encoding", "payload", "qos", "retain"],
        "last-will",
    )?;
    if let Some(value) = object.get("payload-encoding")
        && value.as_str() != Some("utf8")
    {
        return Err(invalid(format!(
            "MQTT device `{}` `last-will.payload-encoding` must be `utf8`",
            device.name
        )));
    }
    let topic_name = require_string(object, "topic", &device.name)?;
    if !topic::is_valid_topic_name(&topic_name) {
        return Err(invalid(format!(
            "MQTT device `{}` last-will topic `{topic_name}` is not a valid topic name",
            device.name
        )));
    }
    let payload = require_string(object, "payload", &device.name)?;
    let qos = match object.get("qos") {
        Some(value) => parse_qos(value, &device.name, "last-will.qos")?,
        None => QoS::AtLeastOnce,
    };
    let retain = optional_bool(object, "retain", &device.name)?.unwrap_or(false);
    Ok(LastWillPlan {
        topic: topic_name,
        payload: payload.into_bytes(),
        qos,
        retain,
    })
}

fn reject_foreign_keys(
    device: &BackendDeviceConfig,
    object: &BTreeMap<String, Json>,
) -> Result<(), Error> {
    const ETHERCAT_KEYS: &[&str] = &[
        "master",
        "alias",
        "position",
        "vendor",
        "product",
        "revision",
        "startup-timeout",
        "recovery-cycles",
        "syncs",
        "pdo-channels",
        "dc",
    ];
    const MODBUS_KEYS: &[&str] = &[
        "bind",
        "unit",
        "max-connections",
        "idle-timeout",
        "word-order",
    ];
    for key in object.keys() {
        if ETHERCAT_KEYS.contains(&key.as_str()) {
            return Err(invalid(format!(
                "MQTT device `{}` uses the EtherCAT key `{key}`",
                device.name
            )));
        }
        if MODBUS_KEYS.contains(&key.as_str()) {
            return Err(invalid(format!(
                "MQTT device `{}` uses the Modbus key `{key}`",
                device.name
            )));
        }
    }
    Ok(())
}

fn reject_unknown_keys(
    device: &BackendDeviceConfig,
    object: &BTreeMap<String, Json>,
    allowed: &[&str],
    scope: &str,
) -> Result<(), Error> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(invalid(format!(
                "MQTT device `{}` has unknown {scope} key `{key}`",
                device.name
            )));
        }
    }
    Ok(())
}

fn validate_host(device: &str, host: &str) -> Result<(), Error> {
    let valid = !host.is_empty()
        && host.len() <= 253
        && !host.contains('/')
        && !host.contains(char::is_whitespace)
        && !host.contains('\0');
    if valid {
        Ok(())
    } else {
        Err(invalid(format!(
            "MQTT device `{device}` has an invalid host name `{host}`"
        )))
    }
}

fn require_string(
    object: &BTreeMap<String, Json>,
    key: &str,
    device: &str,
) -> Result<String, Error> {
    match object.get(key) {
        Some(value) => value
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                invalid(format!(
                    "MQTT device `{device}` `{key}` must be a non-empty string"
                ))
            }),
        None => Err(invalid(format!(
            "MQTT device `{device}` is missing the required `{key}` key"
        ))),
    }
}

fn optional_bool(
    object: &BTreeMap<String, Json>,
    key: &str,
    device: &str,
) -> Result<Option<bool>, Error> {
    match object.get(key) {
        Some(value) => value.as_bool().map(Some).ok_or_else(|| {
            invalid(format!(
                "MQTT device `{device}` `{key}` must be true or false"
            ))
        }),
        None => Ok(None),
    }
}

fn number(value: &Json) -> Option<u64> {
    let value = value.as_number()?;
    u64::try_from(value).ok()
}

fn parse_qos(value: &Json, device: &str, scope: &str) -> Result<QoS, Error> {
    let bits = number(value).ok_or_else(|| {
        invalid(format!(
            "MQTT device `{device}` `{scope}` must be 0, 1, or 2"
        ))
    })?;
    u8::try_from(bits)
        .ok()
        .and_then(QoS::from_bits)
        .ok_or_else(|| {
            invalid(format!(
                "MQTT device `{device}` `{scope}` must be 0, 1, or 2"
            ))
        })
}

fn duration_setting(
    device: &BackendDeviceConfig,
    key: &str,
    value: &Json,
) -> Result<Duration, Error> {
    let text = value.as_str().ok_or_else(|| {
        invalid(format!(
            "MQTT device `{}` `{key}` must be a duration string",
            device.name
        ))
    })?;
    parse_duration(text)
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| {
            invalid(format!(
                "MQTT device `{}` `{key}` must be a positive duration such as \"5s\"",
                device.name
            ))
        })
}

/// Parses a duration string in the manifest spelling (`ms`, `s`, `m`, `h`).
#[must_use]
pub fn parse_duration(text: &str) -> Option<Duration> {
    let split = text
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(text.len());
    if split == 0 {
        return None;
    }
    let number: u64 = text.get(..split)?.parse().ok()?;
    let factor = match text.get(split..)? {
        "ns" => 1,
        "us" | "µs" => 1_000,
        "ms" => 1_000_000,
        "s" => 1_000_000_000,
        "m" => 60_000_000_000,
        "h" => 3_600_000_000_000,
        _ => return None,
    };
    number.checked_mul(factor).map(Duration::from_nanos)
}

fn resolve_file(
    device: &BackendDeviceConfig,
    value: &str,
    base_dir: &Path,
    code: &'static str,
    description: &str,
) -> Result<PathBuf, Error> {
    if value.is_empty() || value.contains('\0') {
        return Err(Error::new(
            code,
            format!(
                "MQTT device `{}` has an invalid {description} path",
                device.name
            ),
        ));
    }
    let path = Path::new(value);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    };
    let metadata = std::fs::metadata(&resolved).map_err(|error| {
        Error::new(
            code,
            format!(
                "MQTT device `{}` cannot read {description} `{}`: {error}",
                device.name,
                resolved.display()
            ),
        )
    })?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(Error::new(
            code,
            format!(
                "MQTT device `{}` {description} `{}` is not a non-empty file",
                device.name,
                resolved.display()
            ),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(Error::new(
                code,
                format!(
                    "MQTT device `{}` {description} `{}` is readable by other users; chmod 600",
                    device.name,
                    resolved.display()
                ),
            ));
        }
    }
    Ok(resolved)
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(codes::MQTT_INVALID_CONFIG, message)
}

fn resolve(message: impl Into<String>) -> Error {
    Error::new(codes::MQTT_CHANNEL_RESOLVE, message)
}

fn tls(message: impl Into<String>) -> Error {
    Error::new(codes::MQTT_TLS_MATERIAL, message)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::coordinator::{BackendChannel, ImageRange};
    use crate::module_info::ImageLayout;

    use super::*;

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir()
                .join(format!("stc-mqtt-config-{}-{unique}", std::process::id()));
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }

        fn file(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.path.join(name);
            let mut file = std::fs::File::create(&path).expect("create file");
            file.write_all(contents.as_bytes()).expect("write file");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                    .expect("private file");
            }
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn channel(name: &str, direction: &str, width_bits: u32, offset: u64) -> BackendChannel {
        BackendChannel {
            name: name.to_owned(),
            direction: direction.to_owned(),
            width_bits,
            image: Some(ImageRange {
                area: direction_area(direction),
                offset_bits: offset,
                width_bits: u64::from(width_bits),
            }),
        }
    }

    fn device(
        name: &str,
        kind: &str,
        config: &str,
        channels: Vec<BackendChannel>,
    ) -> BackendDeviceConfig {
        BackendDeviceConfig {
            name: name.to_owned(),
            kind: kind.to_owned(),
            config: Some(config.to_owned()),
            channels,
        }
    }

    fn backend(devices: Vec<BackendDeviceConfig>) -> BackendConfig {
        BackendConfig {
            name: "mqtt".to_owned(),
            devices,
            bindings: Vec::new(),
            layout: ImageLayout {
                inputs_bytes: 16,
                outputs_bytes: 16,
                markers_bytes: 0,
            },
            profile: None,
        }
    }

    fn minimal_profile() -> &'static str {
        r#"{
            "host": "broker.example",
            "channels": [
                {"name": "Speed", "topic": "line1/speed", "payload-encoding": "uint", "qos": 1}
            ]
        }"#
    }

    fn minimal() -> BackendConfig {
        backend(vec![device(
            "broker0",
            MQTT_KIND,
            minimal_profile(),
            vec![channel("Speed", "input", 16, 0)],
        )])
    }

    #[test]
    fn minimal_device_plans_defaults() {
        let dir = TempDir::new();
        let plan = plan(&minimal(), &dir.path).expect("valid plan");
        assert_eq!(plan.devices.len(), 1);
        let device = &plan.devices[0];
        assert_eq!(device.host, "broker.example");
        assert_eq!(device.port, DEFAULT_PORT);
        assert!(device.tls.is_none());
        assert!(device.clean_start);
        assert_eq!(device.session_expiry, 0);
        assert_eq!(device.session.keep_alive, Duration::from_secs(30));
        assert_eq!(device.session.connect_timeout, DEFAULT_CONNECT_TIMEOUT);
        assert_eq!(device.session.backoff.len(), DEFAULT_BACKOFF_MS.len());
        assert_eq!(device.stale_timeout, DEFAULT_STALE_TIMEOUT);
        assert_eq!(device.channels.len(), 1);
        assert_eq!(device.channels[0].qos, QoS::AtLeastOnce);
        assert!(!device.channels[0].retain);
        assert_eq!(device.channels[0].encoding, Encoding::Uint);
        assert_eq!(device.channels[0].byte_order, ByteOrder::Big);
        assert_eq!(device.channels[0].image.unwrap().offset_bits, 0);
        assert!(device.unbound.is_empty());
        assert!(plan.status.is_empty());
    }

    #[test]
    fn channels_map_by_direction_with_wildcard_subscriptions() {
        let dir = TempDir::new();
        let config = backend(vec![device(
            "broker0",
            MQTT_KIND,
            r#"{
                "host": "127.0.0.1",
                "port": 18830,
                "qos": 2,
                "retain": true,
                "channels": [
                    {"name": "Speed", "topic": "line1/+/speed", "payload-encoding": "int"},
                    {"name": "Start", "topic": "line1/start", "payload-encoding": "bool", "qos": 0}
                ]
            }"#,
            vec![
                channel("Speed", "input", 16, 0),
                channel("Start", "output", 1, 0),
            ],
        )]);
        let plan = plan(&config, &dir.path).expect("valid plan");
        let channels = &plan.devices[0].channels;
        assert_eq!(channels[0].direction, Direction::Input);
        assert_eq!(channels[0].topic, "line1/+/speed");
        assert_eq!(channels[0].qos, QoS::ExactlyOnce);
        assert_eq!(channels[1].direction, Direction::Output);
        assert_eq!(channels[1].qos, QoS::AtMostOnce);
        assert!(
            channels[1].retain,
            "device default retain applies to publishes"
        );
    }

    #[test]
    fn unmapped_and_unknown_channels_are_rejected() {
        let dir = TempDir::new();
        let missing = backend(vec![device(
            "broker0",
            MQTT_KIND,
            minimal_profile(),
            vec![
                channel("Speed", "input", 16, 0),
                channel("Extra", "input", 8, 16),
            ],
        )]);
        let error = plan(&missing, &dir.path).unwrap_err();
        assert_eq!(error.code, codes::MQTT_CHANNEL_RESOLVE);
        assert!(error.message.contains("Extra"));

        let unknown = backend(vec![device(
            "broker0",
            MQTT_KIND,
            r#"{"host":"h","channels":[{"name":"Nope","topic":"t","payload-encoding":"bool"}]}"#,
            vec![],
        )]);
        let error = plan(&unknown, &dir.path).unwrap_err();
        assert_eq!(error.code, codes::MQTT_CHANNEL_RESOLVE);
        assert!(error.message.contains("Nope"));
    }

    #[test]
    fn direction_mismatch_is_rejected() {
        let dir = TempDir::new();
        let config = backend(vec![device(
            "broker0",
            MQTT_KIND,
            r#"{"host":"h","channels":[
                {"name":"Speed","topic":"t","payload-encoding":"uint","direction":"output"}
            ]}"#,
            vec![channel("Speed", "input", 16, 0)],
        )]);
        let error = plan(&config, &dir.path).unwrap_err();
        assert_eq!(error.code, codes::MQTT_CHANNEL_RESOLVE);
        assert!(error.message.contains("declared"));
    }

    #[test]
    fn publish_topics_reject_wildcards_and_inputs_reject_retain() {
        let dir = TempDir::new();
        let wildcard = backend(vec![device(
            "broker0",
            MQTT_KIND,
            r#"{"host":"h","channels":[
                {"name":"Start","topic":"line1/+","payload-encoding":"bool"}
            ]}"#,
            vec![channel("Start", "output", 1, 0)],
        )]);
        let error = plan(&wildcard, &dir.path).unwrap_err();
        assert_eq!(error.code, codes::MQTT_CHANNEL_RESOLVE);
        assert!(error.message.contains("topic name"));

        let retain = backend(vec![device(
            "broker0",
            MQTT_KIND,
            r#"{"host":"h","channels":[
                {"name":"Speed","topic":"line1/speed","payload-encoding":"uint","retain":true}
            ]}"#,
            vec![channel("Speed", "input", 16, 0)],
        )]);
        let error = plan(&retain, &dir.path).unwrap_err();
        assert_eq!(error.code, codes::MQTT_INVALID_CONFIG);
        assert!(error.message.contains("retain"));
    }

    #[test]
    fn duplicate_topics_in_one_direction_are_rejected() {
        let dir = TempDir::new();
        let config = backend(vec![device(
            "broker0",
            MQTT_KIND,
            r#"{"host":"h","channels":[
                {"name":"A","topic":"line1/x","payload-encoding":"uint"},
                {"name":"B","topic":"line1/x","payload-encoding":"uint"}
            ]}"#,
            vec![channel("A", "input", 16, 0), channel("B", "input", 16, 16)],
        )]);
        let error = plan(&config, &dir.path).unwrap_err();
        assert_eq!(error.code, codes::MQTT_INVALID_CONFIG);
        assert!(error.message.contains("more than once"));
    }

    #[test]
    fn unknown_and_foreign_keys_are_rejected() {
        let dir = TempDir::new();
        for (snippet, expected) in [
            (r#""speed": 1"#, "unknown device key"),
            (r#""master": 0"#, "EtherCAT key"),
            (r#""bind": ["127.0.0.1:502"]"#, "Modbus key"),
        ] {
            let text = format!(r#"{{"host":"h",{snippet},"channels":[]}}"#);
            let config = backend(vec![device("broker0", MQTT_KIND, &text, vec![])]);
            let error = plan(&config, &dir.path).unwrap_err();
            assert_eq!(error.code, codes::MQTT_INVALID_CONFIG, "{snippet}");
            assert!(error.message.contains(expected), "{}", error.message);
        }
    }

    #[test]
    fn bad_timing_and_ports_are_rejected() {
        let dir = TempDir::new();
        for snippet in [
            r#""port": 0"#,
            r#""port": 70000"#,
            r#""keepalive": "0s""#,
            r#""keepalive": "10x""#,
            r#""connect-timeout": "0s""#,
            r#""stale-timeout": "0ms""#,
            r#""reconnect-backoff": []"#,
            r#""reconnect-backoff": ["0s"]"#,
            r#""clean-start": "yes""#,
            r#""session-expiry": -1"#,
            r#""session-expiry": "1.5s""#,
            r#""qos": 3"#,
        ] {
            let text = format!(r#"{{"host":"h",{snippet},"channels":[]}}"#);
            let config = backend(vec![device("broker0", MQTT_KIND, &text, vec![])]);
            let error = plan(&config, &dir.path).unwrap_err();
            assert_eq!(error.code, codes::MQTT_INVALID_CONFIG, "{snippet}");
        }
        let text = r#"{"host":"bad host","channels":[]}"#;
        let config = backend(vec![device("broker0", MQTT_KIND, text, vec![])]);
        assert_eq!(
            plan(&config, &dir.path).unwrap_err().code,
            codes::MQTT_INVALID_CONFIG
        );
    }

    #[test]
    fn encoding_widths_are_checked_against_the_declared_channel() {
        let dir = TempDir::new();
        for (encoding, width) in [("float", 16), ("bool", 8), ("raw", 12), ("uint", 24)] {
            let text = format!(
                r#"{{"host":"h","channels":[{{"name":"C","topic":"t","payload-encoding":"{encoding}"}}]}}"#
            );
            let config = backend(vec![device(
                "broker0",
                MQTT_KIND,
                &text,
                vec![channel("C", "input", width, 0)],
            )]);
            let error = plan(&config, &dir.path).unwrap_err();
            assert_eq!(error.code, codes::MQTT_INVALID_CONFIG, "{encoding} {width}");
        }
    }

    #[test]
    fn tls_material_is_resolved_and_validated() {
        let dir = TempDir::new();
        dir.file("ca.pem", "ca");
        let text = r#"{
            "host":"broker.example",
            "tls":{"ca-file":"ca.pem","server-name":"broker.example","verify":false},
            "channels":[]
        }"#;
        let config = backend(vec![device("broker0", MQTT_KIND, text, vec![])]);
        let planned = plan(&config, &dir.path).expect("valid tls plan");
        let tls = planned.devices[0].tls.as_ref().unwrap();
        assert_eq!(tls.ca_file, dir.path.join("ca.pem"));
        assert_eq!(tls.server_name.as_deref(), Some("broker.example"));
        assert!(!tls.verify);

        let missing = r#"{"host":"h","tls":{"ca-file":"absent.pem"},"channels":[]}"#;
        let config = backend(vec![device("broker0", MQTT_KIND, missing, vec![])]);
        let error = plan(&config, &dir.path).unwrap_err();
        assert_eq!(error.code, codes::MQTT_TLS_MATERIAL);

        let half = r#"{
            "host":"h",
            "tls":{"ca-file":"ca.pem","client-cert-file":"ca.pem"},
            "channels":[]
        }"#;
        let config = backend(vec![device("broker0", MQTT_KIND, half, vec![])]);
        let error = plan(&config, &dir.path).unwrap_err();
        assert_eq!(error.code, codes::MQTT_TLS_MATERIAL);
        assert!(error.message.contains("client-key-file"));
    }

    #[test]
    fn ports_default_to_the_tls_port_with_tls() {
        let dir = TempDir::new();
        dir.file("ca.pem", "ca");
        let text = r#"{"host":"h","tls":{"ca-file":"ca.pem"},"channels":[]}"#;
        let config = backend(vec![device("broker0", MQTT_KIND, text, vec![])]);
        let plan = plan(&config, &dir.path).expect("valid plan");
        assert_eq!(plan.devices[0].port, DEFAULT_TLS_PORT);
    }

    #[test]
    fn credentials_require_files_and_a_username_for_a_password() {
        let dir = TempDir::new();
        dir.file("user", "plc\n");
        dir.file("pass", "secret\n");
        let text = r#"{
            "host":"h",
            "credentials":{"username-file":"user","password-file":"pass"},
            "channels":[]
        }"#;
        let config = backend(vec![device("broker0", MQTT_KIND, text, vec![])]);
        let planned = plan(&config, &dir.path).expect("valid credentials plan");
        let credentials = planned.devices[0].credentials.as_ref().unwrap();
        assert_eq!(
            credentials.username_file.as_deref(),
            Some(dir.path.join("user").as_path())
        );
        assert!(credentials.password_file.is_some());

        let password_only = r#"{"host":"h","credentials":{"password-file":"pass"},"channels":[]}"#;
        let config = backend(vec![device("broker0", MQTT_KIND, password_only, vec![])]);
        let error = plan(&config, &dir.path).unwrap_err();
        assert_eq!(error.code, codes::MQTT_INVALID_CONFIG);
        assert!(error.message.contains("user name"));

        let empty = r#"{"host":"h","credentials":{},"channels":[]}"#;
        let config = backend(vec![device("broker0", MQTT_KIND, empty, vec![])]);
        assert_eq!(
            plan(&config, &dir.path).unwrap_err().code,
            codes::MQTT_INVALID_CONFIG
        );
    }

    #[cfg(unix)]
    #[test]
    fn credential_files_must_not_be_world_readable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        let path = dir.file("user", "plc\n");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let text = r#"{"host":"h","credentials":{"username-file":"user"},"channels":[]}"#;
        let config = backend(vec![device("broker0", MQTT_KIND, text, vec![])]);
        let error = plan(&config, &dir.path).unwrap_err();
        assert_eq!(error.code, codes::MQTT_CREDENTIAL_MATERIAL);
        assert!(error.message.contains("chmod 600"));
    }

    #[test]
    fn last_will_is_validated() {
        let dir = TempDir::new();
        let text = r#"{
            "host":"h",
            "last-will":{"topic":"line1/status","payload-encoding":"utf8",
                         "payload":"offline","qos":1,"retain":true},
            "channels":[]
        }"#;
        let config = backend(vec![device("broker0", MQTT_KIND, text, vec![])]);
        let planned = plan(&config, &dir.path).expect("valid will");
        let will = planned.devices[0].last_will.as_ref().unwrap();
        assert_eq!(will.topic, "line1/status");
        assert_eq!(will.payload, b"offline".to_vec());
        assert_eq!(will.qos, QoS::AtLeastOnce);
        assert!(will.retain);

        let bad = r#"{"host":"h","last-will":{"topic":"line1/#","payload":"x"},"channels":[]}"#;
        let config = backend(vec![device("broker0", MQTT_KIND, bad, vec![])]);
        assert_eq!(
            plan(&config, &dir.path).unwrap_err().code,
            codes::MQTT_INVALID_CONFIG
        );
    }

    #[test]
    fn status_layout_maps_channels_and_validates_index_and_width() {
        let dir = TempDir::new();
        let data = device(
            "broker0",
            MQTT_KIND,
            minimal_profile(),
            vec![channel("Speed", "input", 16, 0)],
        );
        let status = device(
            "mqtt-status",
            MQTT_STATUS_KIND,
            r#"{"layout":1}"#,
            vec![
                channel("client0.connected", "input", 1, 0),
                channel("client0.reason", "input", 32, 8),
                channel("input.stale", "input", 1, 40),
                channel("cycle.exchanges", "input", 32, 48),
            ],
        );
        let planned = plan(&backend(vec![data, status]), &dir.path).expect("valid status");
        assert_eq!(planned.status.len(), 4);
        assert_eq!(planned.status[0].field, StatusField::Connected);
        assert_eq!(planned.status[0].index, StatusIndex::Client(0));
        assert_eq!(planned.status[2].field, StatusField::Stale);
        assert_eq!(planned.status[2].index, StatusIndex::Global);

        for (name, width, expected) in [
            ("client1.connected", 1, "client1"),
            ("client0.nope", 1, "unknown status"),
            ("client0.state", 8, "layout 1"),
        ] {
            let status = device(
                "mqtt-status",
                MQTT_STATUS_KIND,
                r#"{"layout":1}"#,
                vec![channel(name, "input", width, 0)],
            );
            let error = plan(
                &backend(vec![
                    device(
                        "broker0",
                        MQTT_KIND,
                        minimal_profile(),
                        vec![channel("Speed", "input", 16, 0)],
                    ),
                    status,
                ]),
                &dir.path,
            )
            .unwrap_err();
            assert_eq!(error.code, codes::MQTT_INVALID_CONFIG, "{name}");
            assert!(error.message.contains(expected), "{}", error.message);
        }

        let writable = device(
            "mqtt-status",
            MQTT_STATUS_KIND,
            r#"{"layout":1}"#,
            vec![channel("input.stale", "output", 1, 0)],
        );
        let error = plan(
            &backend(vec![
                device(
                    "broker0",
                    MQTT_KIND,
                    minimal_profile(),
                    vec![channel("Speed", "input", 16, 0)],
                ),
                writable,
            ]),
            &dir.path,
        )
        .unwrap_err();
        assert!(error.message.contains("read-only"));
    }

    #[test]
    fn duplicate_devices_and_wrong_kinds_are_rejected() {
        let dir = TempDir::new();
        let duplicate = backend(vec![
            device(
                "broker0",
                MQTT_KIND,
                minimal_profile(),
                vec![channel("Speed", "input", 16, 0)],
            ),
            device(
                "broker0",
                MQTT_KIND,
                minimal_profile(),
                vec![channel("Speed", "input", 16, 0)],
            ),
        ]);
        assert_eq!(
            plan(&duplicate, &dir.path).unwrap_err().code,
            codes::MQTT_INVALID_CONFIG
        );

        let wrong = backend(vec![device(
            "broker0",
            "ethercat",
            minimal_profile(),
            vec![channel("Speed", "input", 16, 0)],
        )]);
        assert_eq!(
            plan(&wrong, &dir.path).unwrap_err().code,
            codes::MQTT_INVALID_CONFIG
        );
    }

    #[test]
    fn unbound_channels_are_planned_not_rejected() {
        let dir = TempDir::new();
        let mut speed = channel("Speed", "input", 16, 0);
        speed.image = None;
        let config = backend(vec![device(
            "broker0",
            MQTT_KIND,
            minimal_profile(),
            vec![speed],
        )]);
        let plan = plan(&config, &dir.path).expect("unbound channel is diagnostic only");
        assert_eq!(plan.devices[0].unbound, vec!["Speed".to_owned()]);
        assert!(plan.devices[0].channels[0].image.is_none());
    }

    #[test]
    fn session_expiry_is_a_whole_second_duration() {
        let dir = TempDir::new();
        let text = r#"{"host":"h","session-expiry":"1h","channels":[]}"#;
        let config = backend(vec![device("broker0", MQTT_KIND, text, vec![])]);
        let planned = plan(&config, &dir.path).expect("valid session expiry");
        assert_eq!(planned.devices[0].session_expiry, 3_600);
    }

    #[test]
    fn durations_parse_in_the_manifest_spelling() {
        assert_eq!(parse_duration("250ms"), Some(Duration::from_millis(250)));
        assert_eq!(parse_duration("5s"), Some(Duration::from_secs(5)));
        assert_eq!(parse_duration("2m"), Some(Duration::from_secs(120)));
        assert_eq!(parse_duration("1h"), Some(Duration::from_secs(3_600)));
        assert_eq!(parse_duration("0s"), Some(Duration::ZERO));
        for bad in ["", "s", "5", "5d", "5.5s", "-5s"] {
            assert_eq!(parse_duration(bad), None, "{bad}");
        }
    }
}
