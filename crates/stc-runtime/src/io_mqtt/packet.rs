//! MQTT 5.0 control-packet codec (WL-15 phase 1).
//!
//! The decoder accepts the server-to-client packets the client consumes and
//! rejects everything else with [`MqttError`]. The encoder produces the
//! client-to-server packets the session sends. Both directions enforce the
//! fixed-header flags, the variable-integer rules, the property identifier and
//! duplicate rules, and the length limits; a malformed frame is never accepted
//! partially.
//!
//! MQTT 5.0 only: there is no 3.1.1 path and no version negotiation.

use super::topic;
use super::{ErrorKind, MqttError};

/// Largest value representable by the four-byte remaining-length integer.
pub const MAX_REMAINING_LENGTH: usize = 268_435_455;

/// Control-packet type codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketType {
    /// Client to server connection request.
    Connect,
    /// Server connection acknowledgement.
    ConnAck,
    /// Application message.
    Publish,
    /// QoS 1 acknowledgement.
    PubAck,
    /// QoS 2 receipt.
    PubRec,
    /// QoS 2 release.
    PubRel,
    /// QoS 2 completion.
    PubComp,
    /// Subscription request.
    Subscribe,
    /// Subscription acknowledgement.
    SubAck,
    /// Unsubscription request.
    Unsubscribe,
    /// Unsubscription acknowledgement.
    UnsubAck,
    /// Keepalive request.
    PingReq,
    /// Keepalive response.
    PingResp,
    /// Disconnect notification.
    Disconnect,
    /// Enhanced authentication exchange (unsupported).
    Auth,
}

impl PacketType {
    /// Returns the four-bit protocol code.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Connect => 1,
            Self::ConnAck => 2,
            Self::Publish => 3,
            Self::PubAck => 4,
            Self::PubRec => 5,
            Self::PubRel => 6,
            Self::PubComp => 7,
            Self::Subscribe => 8,
            Self::SubAck => 9,
            Self::Unsubscribe => 10,
            Self::UnsubAck => 11,
            Self::PingReq => 12,
            Self::PingResp => 13,
            Self::Disconnect => 14,
            Self::Auth => 15,
        }
    }

    /// Returns the packet type for a four-bit code.
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Connect),
            2 => Some(Self::ConnAck),
            3 => Some(Self::Publish),
            4 => Some(Self::PubAck),
            5 => Some(Self::PubRec),
            6 => Some(Self::PubRel),
            7 => Some(Self::PubComp),
            8 => Some(Self::Subscribe),
            9 => Some(Self::SubAck),
            10 => Some(Self::Unsubscribe),
            11 => Some(Self::UnsubAck),
            12 => Some(Self::PingReq),
            13 => Some(Self::PingResp),
            14 => Some(Self::Disconnect),
            15 => Some(Self::Auth),
            _ => None,
        }
    }

    /// Returns the name used in diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connect => "CONNECT",
            Self::ConnAck => "CONNACK",
            Self::Publish => "PUBLISH",
            Self::PubAck => "PUBACK",
            Self::PubRec => "PUBREC",
            Self::PubRel => "PUBREL",
            Self::PubComp => "PUBCOMP",
            Self::Subscribe => "SUBSCRIBE",
            Self::SubAck => "SUBACK",
            Self::Unsubscribe => "UNSUBSCRIBE",
            Self::UnsubAck => "UNSUBACK",
            Self::PingReq => "PINGREQ",
            Self::PingResp => "PINGRESP",
            Self::Disconnect => "DISCONNECT",
            Self::Auth => "AUTH",
        }
    }
}

/// Quality of service of one message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum QoS {
    /// Fire and forget.
    AtMostOnce,
    /// Acknowledged delivery.
    AtLeastOnce,
    /// Two-phase delivery.
    ExactlyOnce,
}

impl QoS {
    /// Parses the two-bit wire value.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Option<Self> {
        match bits {
            0 => Some(Self::AtMostOnce),
            1 => Some(Self::AtLeastOnce),
            2 => Some(Self::ExactlyOnce),
            _ => None,
        }
    }

    /// Returns the two-bit wire value.
    #[must_use]
    pub const fn bits(self) -> u8 {
        match self {
            Self::AtMostOnce => 0,
            Self::AtLeastOnce => 1,
            Self::ExactlyOnce => 2,
        }
    }

    /// Returns `true` when the packet carries a packet identifier.
    #[must_use]
    pub const fn requires_packet_id(self) -> bool {
        !matches!(self, Self::AtMostOnce)
    }
}

/// One MQTT 5.0 property block.
///
/// Properties are typed by identifier, and a non-repeatable identifier may
/// appear at most once. User properties repeat in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Properties {
    /// Payload format indicator (0 bytes, 1 UTF-8).
    pub payload_format_indicator: Option<u8>,
    /// Message expiry interval in seconds.
    pub message_expiry_interval: Option<u32>,
    /// Content type of the payload.
    pub content_type: Option<String>,
    /// Response topic (not surfaced by this client).
    pub response_topic: Option<String>,
    /// Correlation data (not surfaced by this client).
    pub correlation_data: Option<Vec<u8>>,
    /// Subscription identifier (not surfaced by this client).
    pub subscription_identifier: Option<u32>,
    /// Session expiry interval in seconds.
    pub session_expiry_interval: Option<u32>,
    /// Client identifier assigned by the broker.
    pub assigned_client_identifier: Option<String>,
    /// Keepalive assigned by the broker.
    pub server_keep_alive: Option<u16>,
    /// Authentication method (unsupported).
    pub authentication_method: Option<String>,
    /// Authentication data (unsupported).
    pub authentication_data: Option<Vec<u8>>,
    /// Request problem information flag.
    pub request_problem_information: Option<u8>,
    /// Will delay interval in seconds.
    pub will_delay_interval: Option<u32>,
    /// Request response information flag.
    pub request_response_information: Option<u8>,
    /// Response information.
    pub response_information: Option<String>,
    /// Server reference for redirects.
    pub server_reference: Option<String>,
    /// Human-readable reason string.
    pub reason_string: Option<String>,
    /// Receive maximum the peer accepts.
    pub receive_maximum: Option<u16>,
    /// Topic alias maximum the peer accepts.
    pub topic_alias_maximum: Option<u16>,
    /// Topic alias of this message (rejected by this client).
    pub topic_alias: Option<u16>,
    /// Maximum QoS the broker supports.
    pub maximum_qos: Option<u8>,
    /// Retain-available flag.
    pub retain_available: Option<u8>,
    /// User properties, in wire order.
    pub user_properties: Vec<(String, String)>,
    /// Maximum packet size the broker accepts.
    pub maximum_packet_size: Option<u32>,
    /// Wildcard-subscription-available flag.
    pub wildcard_subscription_available: Option<u8>,
    /// Subscription-identifier-available flag.
    pub subscription_identifier_available: Option<u8>,
    /// Shared-subscription-available flag.
    pub shared_subscription_available: Option<u8>,
}

impl Properties {
    /// Encodes the property block, including its variable-integer length.
    pub fn encode(&self) -> Result<Vec<u8>, MqttError> {
        let mut body = Writer::default();
        self.encode_into(&mut body)?;
        let mut out = Writer::default();
        out.varint(u32::try_from(body.buf.len()).map_err(|_| {
            MqttError::too_large("property block exceeds the remaining-length limit")
        })?)?;
        out.raw(&body.buf);
        Ok(out.buf)
    }

    fn encode_into(&self, out: &mut Writer) -> Result<(), MqttError> {
        if let Some(value) = self.payload_format_indicator {
            out.u8(0x01);
            out.u8(value);
        }
        if let Some(value) = self.message_expiry_interval {
            out.u8(0x02);
            out.u32(value);
        }
        if let Some(value) = &self.content_type {
            out.u8(0x03);
            out.string(value)?;
        }
        if let Some(value) = &self.response_topic {
            out.u8(0x08);
            out.string(value)?;
        }
        if let Some(value) = &self.correlation_data {
            out.u8(0x09);
            out.binary(value)?;
        }
        if let Some(value) = self.subscription_identifier {
            out.u8(0x0B);
            out.varint(value)?;
        }
        if let Some(value) = self.session_expiry_interval {
            out.u8(0x11);
            out.u32(value);
        }
        if let Some(value) = &self.assigned_client_identifier {
            out.u8(0x12);
            out.string(value)?;
        }
        if let Some(value) = self.server_keep_alive {
            out.u8(0x13);
            out.u16(value);
        }
        if let Some(value) = &self.authentication_method {
            out.u8(0x15);
            out.string(value)?;
        }
        if let Some(value) = &self.authentication_data {
            out.u8(0x16);
            out.binary(value)?;
        }
        if let Some(value) = self.request_problem_information {
            out.u8(0x17);
            out.u8(value);
        }
        if let Some(value) = self.will_delay_interval {
            out.u8(0x18);
            out.u32(value);
        }
        if let Some(value) = self.request_response_information {
            out.u8(0x19);
            out.u8(value);
        }
        if let Some(value) = &self.response_information {
            out.u8(0x1A);
            out.string(value)?;
        }
        if let Some(value) = &self.server_reference {
            out.u8(0x1C);
            out.string(value)?;
        }
        if let Some(value) = &self.reason_string {
            out.u8(0x1F);
            out.string(value)?;
        }
        if let Some(value) = self.receive_maximum {
            out.u8(0x21);
            out.u16(value);
        }
        if let Some(value) = self.topic_alias_maximum {
            out.u8(0x22);
            out.u16(value);
        }
        if let Some(value) = self.topic_alias {
            out.u8(0x23);
            out.u16(value);
        }
        if let Some(value) = self.maximum_qos {
            out.u8(0x24);
            out.u8(value);
        }
        if let Some(value) = self.retain_available {
            out.u8(0x25);
            out.u8(value);
        }
        for (key, value) in &self.user_properties {
            out.u8(0x26);
            out.string(key)?;
            out.string(value)?;
        }
        if let Some(value) = self.maximum_packet_size {
            out.u8(0x27);
            out.u32(value);
        }
        if let Some(value) = self.wildcard_subscription_available {
            out.u8(0x28);
            out.u8(value);
        }
        if let Some(value) = self.subscription_identifier_available {
            out.u8(0x29);
            out.u8(value);
        }
        if let Some(value) = self.shared_subscription_available {
            out.u8(0x2A);
            out.u8(value);
        }
        Ok(())
    }

    fn decode(reader: &mut Reader<'_>) -> Result<Self, MqttError> {
        let length = usize::try_from(reader.varint()?)
            .map_err(|_| MqttError::malformed("property length is not a valid size"))?;
        let bytes = reader.take(length)?;
        let mut inner = Reader::new(bytes);
        let mut properties = Self::default();
        while inner.remaining() > 0 {
            let id = inner.varint()?;
            match id {
                0x01 => set_once(
                    &mut properties.payload_format_indicator,
                    inner.u8()?,
                    "payload format indicator",
                )?,
                0x02 => set_once(
                    &mut properties.message_expiry_interval,
                    inner.u32()?,
                    "message expiry interval",
                )?,
                0x03 => set_once(
                    &mut properties.content_type,
                    inner.string()?,
                    "content type",
                )?,
                0x08 => set_once(
                    &mut properties.response_topic,
                    inner.string()?,
                    "response topic",
                )?,
                0x09 => set_once(
                    &mut properties.correlation_data,
                    inner.binary()?,
                    "correlation data",
                )?,
                0x0B => set_once(
                    &mut properties.subscription_identifier,
                    inner.varint()?,
                    "subscription identifier",
                )?,
                0x11 => set_once(
                    &mut properties.session_expiry_interval,
                    inner.u32()?,
                    "session expiry interval",
                )?,
                0x12 => set_once(
                    &mut properties.assigned_client_identifier,
                    inner.string()?,
                    "assigned client identifier",
                )?,
                0x13 => set_once(
                    &mut properties.server_keep_alive,
                    inner.u16()?,
                    "server keep alive",
                )?,
                0x15 => set_once(
                    &mut properties.authentication_method,
                    inner.string()?,
                    "authentication method",
                )?,
                0x16 => set_once(
                    &mut properties.authentication_data,
                    inner.binary()?,
                    "authentication data",
                )?,
                0x17 => set_once(
                    &mut properties.request_problem_information,
                    inner.u8()?,
                    "request problem information",
                )?,
                0x18 => set_once(
                    &mut properties.will_delay_interval,
                    inner.u32()?,
                    "will delay interval",
                )?,
                0x19 => set_once(
                    &mut properties.request_response_information,
                    inner.u8()?,
                    "request response information",
                )?,
                0x1A => set_once(
                    &mut properties.response_information,
                    inner.string()?,
                    "response information",
                )?,
                0x1C => set_once(
                    &mut properties.server_reference,
                    inner.string()?,
                    "server reference",
                )?,
                0x1F => set_once(
                    &mut properties.reason_string,
                    inner.string()?,
                    "reason string",
                )?,
                0x21 => set_once(
                    &mut properties.receive_maximum,
                    inner.u16()?,
                    "receive maximum",
                )?,
                0x22 => set_once(
                    &mut properties.topic_alias_maximum,
                    inner.u16()?,
                    "topic alias maximum",
                )?,
                0x23 => set_once(
                    &mut properties.topic_alias,
                    inner.u16()?,
                    "topic alias",
                )?,
                0x24 => set_once(&mut properties.maximum_qos, inner.u8()?, "maximum QoS")?,
                0x25 => set_once(
                    &mut properties.retain_available,
                    inner.u8()?,
                    "retain available",
                )?,
                0x26 => {
                    let key = inner.string()?;
                    let value = inner.string()?;
                    properties.user_properties.push((key, value));
                }
                0x27 => set_once(
                    &mut properties.maximum_packet_size,
                    inner.u32()?,
                    "maximum packet size",
                )?,
                0x28 => set_once(
                    &mut properties.wildcard_subscription_available,
                    inner.u8()?,
                    "wildcard subscription available",
                )?,
                0x29 => set_once(
                    &mut properties.subscription_identifier_available,
                    inner.u8()?,
                    "subscription identifier available",
                )?,
                0x2A => set_once(
                    &mut properties.shared_subscription_available,
                    inner.u8()?,
                    "shared subscription available",
                )?,
                _ => {
                    return Err(MqttError::malformed(format!(
                        "unknown property identifier {id:#04X}"
                    )));
                }
            }
        }
        properties.validate()?;
        Ok(properties)
    }

    fn validate(&self) -> Result<(), MqttError> {
        for (value, name) in [
            (self.payload_format_indicator, "payload format indicator"),
            (self.request_problem_information, "request problem information"),
            (self.request_response_information, "request response information"),
            (self.maximum_qos, "maximum QoS"),
            (self.retain_available, "retain available"),
            (self.wildcard_subscription_available, "wildcard subscription available"),
            (self.subscription_identifier_available, "subscription identifier available"),
            (self.shared_subscription_available, "shared subscription available"),
        ] {
            if let Some(value) = value
                && value > 1
            {
                return Err(MqttError::malformed(format!("{name} must be 0 or 1")));
            }
        }
        if self.receive_maximum == Some(0) {
            return Err(MqttError::malformed("receive maximum must not be zero"));
        }
        if self.maximum_packet_size == Some(0) {
            return Err(MqttError::malformed("maximum packet size must not be zero"));
        }
        if self.topic_alias == Some(0) {
            return Err(MqttError::malformed("topic alias must not be zero"));
        }
        Ok(())
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, name: &str) -> Result<(), MqttError> {
    if slot.replace(value).is_some() {
        return Err(MqttError::malformed(format!(
            "duplicate {name} property"
        )));
    }
    Ok(())
}

/// Last-will message carried by CONNECT.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Will {
    /// Will topic; a topic name, never a filter.
    pub topic: String,
    /// Will payload bytes.
    pub payload: Vec<u8>,
    /// Will QoS.
    pub qos: QoS,
    /// Will retain flag.
    pub retain: bool,
    /// Will delay interval in seconds.
    pub delay_interval: Option<u32>,
    /// Payload format indicator, when set.
    pub payload_format_indicator: Option<u8>,
    /// Content type, when set.
    pub content_type: Option<String>,
}

/// Client connection request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connect {
    /// Client identifier; empty lets the broker assign one.
    pub client_id: String,
    /// MQTT 5 clean start.
    pub clean_start: bool,
    /// Keepalive interval in seconds; zero disables it.
    pub keep_alive: u16,
    /// Session expiry interval in seconds.
    pub session_expiry: u32,
    /// Maximum number of unacknowledged QoS 1/2 messages the client accepts.
    pub receive_maximum: u16,
    /// Maximum packet size the client accepts.
    pub maximum_packet_size: Option<u32>,
    /// User name, when configured.
    pub username: Option<String>,
    /// Password bytes, when configured.
    pub password: Option<Vec<u8>>,
    /// Last will, when configured.
    pub will: Option<Will>,
}

impl Connect {
    /// Encodes the CONNECT packet.
    pub fn encode(&self) -> Result<Vec<u8>, MqttError> {
        if self.receive_maximum == 0 {
            return Err(MqttError::new(
                ErrorKind::Protocol,
                "receive maximum must not be zero",
            ));
        }
        if self.password.is_some() && self.username.is_none() {
            return Err(MqttError::new(
                ErrorKind::Protocol,
                "a password requires a user name",
            ));
        }
        let mut body = Writer::default();
        body.string("MQTT")?;
        body.u8(5);
        let mut flags = 0_u8;
        if self.clean_start {
            flags |= 0x02;
        }
        if let Some(will) = &self.will {
            if !topic::is_valid_topic_name(&will.topic) {
                return Err(MqttError::new(
                    ErrorKind::Protocol,
                    "will topic is not a valid topic name",
                ));
            }
            flags |= 0x04;
            flags |= will.qos.bits() << 3;
            if will.retain {
                flags |= 0x20;
            }
        }
        if self.username.is_some() {
            flags |= 0x80;
        }
        if self.password.is_some() {
            flags |= 0x40;
        }
        body.u8(flags);
        body.u16(self.keep_alive);
        let properties = Properties {
            session_expiry_interval: Some(self.session_expiry),
            receive_maximum: Some(self.receive_maximum),
            maximum_packet_size: self.maximum_packet_size,
            ..Properties::default()
        };
        body.raw(&properties.encode()?);
        body.string(&self.client_id)?;
        if let Some(will) = &self.will {
            let properties = Properties {
                will_delay_interval: will.delay_interval,
                payload_format_indicator: will.payload_format_indicator,
                content_type: will.content_type.clone(),
                ..Properties::default()
            };
            body.raw(&properties.encode()?);
            body.string(&will.topic)?;
            body.binary(&will.payload)?;
        }
        if let Some(username) = &self.username {
            body.string(username)?;
        }
        if let Some(password) = &self.password {
            body.binary(password)?;
        }
        finish(PacketType::Connect.code(), 0, body)
    }
}

impl Properties {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Server connection acknowledgement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnAck {
    /// Broker continued an existing session.
    pub session_present: bool,
    /// Reason code; zero is success.
    pub reason: u8,
    /// Property block.
    pub properties: Properties,
}

impl ConnAck {
    /// Returns `true` when the connection succeeded.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.reason == 0
    }
}

/// One application message in either direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Publish {
    /// Topic name.
    pub topic: String,
    /// Payload bytes.
    pub payload: Vec<u8>,
    /// QoS of the message.
    pub qos: QoS,
    /// Retain flag.
    pub retain: bool,
    /// Duplicate delivery flag.
    pub dup: bool,
    /// Packet identifier; required for QoS 1/2.
    pub packet_id: Option<u16>,
    /// Property block.
    pub properties: Properties,
}

impl Publish {
    /// Encodes the PUBLISH packet.
    pub fn encode(&self) -> Result<Vec<u8>, MqttError> {
        if !topic::is_valid_topic_name(&self.topic) {
            return Err(MqttError::new(
                ErrorKind::Protocol,
                "publish topic is not a valid topic name",
            ));
        }
        if let Some(id) = self.packet_id
            && id == 0
        {
            return Err(MqttError::new(
                ErrorKind::Protocol,
                "packet identifier must not be zero",
            ));
        }
        let mut flags = 0_u8;
        if self.dup {
            flags |= 0x08;
        }
        flags |= self.qos.bits() << 1;
        if self.retain {
            flags |= 0x01;
        }
        let mut body = Writer::default();
        body.string(&self.topic)?;
        if self.qos.requires_packet_id() {
            let id = self.packet_id.ok_or_else(|| {
                MqttError::new(ErrorKind::Protocol, "QoS 1/2 publish requires a packet id")
            })?;
            body.u16(id);
        }
        body.raw(&self.properties.encode()?);
        body.raw(&self.payload);
        finish(PacketType::Publish.code(), flags, body)
    }
}

/// Acknowledgement kinds that share the packet-identifier encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AckKind {
    /// PUBACK.
    PubAck,
    /// PUBREC.
    PubRec,
    /// PUBREL.
    PubRel,
    /// PUBCOMP.
    PubComp,
}

impl AckKind {
    const fn packet_type(self) -> PacketType {
        match self {
            Self::PubAck => PacketType::PubAck,
            Self::PubRec => PacketType::PubRec,
            Self::PubRel => PacketType::PubRel,
            Self::PubComp => PacketType::PubComp,
        }
    }

    const fn flags(self) -> u8 {
        match self {
            Self::PubRel => 0x02,
            _ => 0,
        }
    }
}

/// A PUBACK, PUBREC, PUBREL, or PUBCOMP packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ack {
    /// Packet identifier.
    pub packet_id: u16,
    /// Reason code; zero is success.
    pub reason: u8,
}

impl Ack {
    /// Encodes the acknowledgement.
    pub fn encode(&self, kind: AckKind) -> Result<Vec<u8>, MqttError> {
        if self.packet_id == 0 {
            return Err(MqttError::new(
                ErrorKind::Protocol,
                "packet identifier must not be zero",
            ));
        }
        let mut body = Writer::default();
        body.u16(self.packet_id);
        if self.reason != 0 {
            body.u8(self.reason);
            body.raw(&Properties::default().encode()?);
        }
        finish(kind.packet_type().code(), kind.flags(), body)
    }
}

/// One subscription entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subscription {
    /// Topic filter, including an optional `$share/<group>/` prefix.
    pub filter: String,
    /// Requested QoS.
    pub qos: QoS,
    /// Do not send the client its own publications.
    pub no_local: bool,
    /// Keep the retain flag of forwarded messages.
    pub retain_as_published: bool,
    /// Retain handling: 0 send retained, 1 only for new subscriptions,
    /// 2 never send retained.
    pub retain_handling: u8,
}

/// Subscription request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subscribe {
    /// Packet identifier.
    pub packet_id: u16,
    /// Property block (subscription identifiers are not used).
    pub properties: Properties,
    /// Filters in request order.
    pub filters: Vec<Subscription>,
}

impl Subscribe {
    /// Encodes the SUBSCRIBE packet.
    pub fn encode(&self) -> Result<Vec<u8>, MqttError> {
        if self.packet_id == 0 {
            return Err(MqttError::new(
                ErrorKind::Protocol,
                "packet identifier must not be zero",
            ));
        }
        if self.filters.is_empty() {
            return Err(MqttError::new(
                ErrorKind::Protocol,
                "a subscription request requires at least one filter",
            ));
        }
        let mut body = Writer::default();
        body.u16(self.packet_id);
        body.raw(&self.properties.encode()?);
        for subscription in &self.filters {
            if !topic::is_valid_topic_filter(&subscription.filter) {
                return Err(MqttError::new(
                    ErrorKind::Protocol,
                    format!("invalid topic filter `{}`", subscription.filter),
                ));
            }
            if subscription.retain_handling > 2 {
                return Err(MqttError::new(
                    ErrorKind::Protocol,
                    "retain handling must be 0, 1, or 2",
                ));
            }
            body.string(&subscription.filter)?;
            let mut options = subscription.qos.bits();
            if subscription.no_local {
                options |= 0x04;
            }
            if subscription.retain_as_published {
                options |= 0x08;
            }
            options |= subscription.retain_handling << 4;
            body.u8(options);
        }
        finish(PacketType::Subscribe.code(), 0x02, body)
    }
}

/// Subscription acknowledgement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubAck {
    /// Packet identifier.
    pub packet_id: u16,
    /// Property block.
    pub properties: Properties,
    /// One reason code per requested filter.
    pub reasons: Vec<u8>,
}

/// Unsubscription request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsubscribe {
    /// Packet identifier.
    pub packet_id: u16,
    /// Property block.
    pub properties: Properties,
    /// Filters to remove.
    pub filters: Vec<String>,
}

impl Unsubscribe {
    /// Encodes the UNSUBSCRIBE packet.
    pub fn encode(&self) -> Result<Vec<u8>, MqttError> {
        if self.packet_id == 0 {
            return Err(MqttError::new(
                ErrorKind::Protocol,
                "packet identifier must not be zero",
            ));
        }
        if self.filters.is_empty() {
            return Err(MqttError::new(
                ErrorKind::Protocol,
                "an unsubscribe request requires at least one filter",
            ));
        }
        let mut body = Writer::default();
        body.u16(self.packet_id);
        body.raw(&self.properties.encode()?);
        for filter in &self.filters {
            if !topic::is_valid_topic_filter(filter) {
                return Err(MqttError::new(
                    ErrorKind::Protocol,
                    format!("invalid topic filter `{filter}`"),
                ));
            }
            body.string(filter)?;
        }
        finish(PacketType::Unsubscribe.code(), 0x02, body)
    }
}

/// Unsubscription acknowledgement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsubAck {
    /// Packet identifier.
    pub packet_id: u16,
    /// Property block.
    pub properties: Properties,
    /// One reason code per removed filter.
    pub reasons: Vec<u8>,
}

/// Disconnect notification in either direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disconnect {
    /// Reason code; zero is normal disconnection.
    pub reason: u8,
    /// Property block.
    pub properties: Properties,
}

impl Disconnect {
    /// Encodes the DISCONNECT packet.
    pub fn encode(&self) -> Result<Vec<u8>, MqttError> {
        let mut body = Writer::default();
        if self.reason != 0 || !self.properties.is_empty() {
            body.u8(self.reason);
            body.raw(&self.properties.encode()?);
        }
        finish(PacketType::Disconnect.code(), 0, body)
    }
}

/// One MQTT 5.0 control packet.
///
/// The decoder produces server-to-client packets only. The encoder accepts the
/// client-to-server variants; encoding a server-only packet is an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Packet {
    /// CONNECT (client to server).
    Connect(Connect),
    /// CONNACK (server to client).
    ConnAck(ConnAck),
    /// PUBLISH in either direction.
    Publish(Publish),
    /// PUBACK.
    PubAck(Ack),
    /// PUBREC.
    PubRec(Ack),
    /// PUBREL.
    PubRel(Ack),
    /// PUBCOMP.
    PubComp(Ack),
    /// SUBSCRIBE (client to server).
    Subscribe(Subscribe),
    /// SUBACK (server to client).
    SubAck(SubAck),
    /// UNSUBSCRIBE (client to server).
    Unsubscribe(Unsubscribe),
    /// UNSUBACK (server to client).
    UnsubAck(UnsubAck),
    /// PINGREQ (client to server).
    PingReq,
    /// PINGRESP (server to client).
    PingResp,
    /// DISCONNECT in either direction.
    Disconnect(Disconnect),
}

impl Packet {
    /// Returns the packet type.
    #[must_use]
    pub const fn packet_type(&self) -> PacketType {
        match self {
            Self::Connect(_) => PacketType::Connect,
            Self::ConnAck(_) => PacketType::ConnAck,
            Self::Publish(_) => PacketType::Publish,
            Self::PubAck(_) => PacketType::PubAck,
            Self::PubRec(_) => PacketType::PubRec,
            Self::PubRel(_) => PacketType::PubRel,
            Self::PubComp(_) => PacketType::PubComp,
            Self::Subscribe(_) => PacketType::Subscribe,
            Self::SubAck(_) => PacketType::SubAck,
            Self::Unsubscribe(_) => PacketType::Unsubscribe,
            Self::UnsubAck(_) => PacketType::UnsubAck,
            Self::PingReq => PacketType::PingReq,
            Self::PingResp => PacketType::PingResp,
            Self::Disconnect(_) => PacketType::Disconnect,
        }
    }

    /// Encodes a client-to-server packet.
    pub fn encode(&self) -> Result<Vec<u8>, MqttError> {
        match self {
            Self::Connect(connect) => connect.encode(),
            Self::Publish(publish) => publish.encode(),
            Self::PubAck(ack) => ack.encode(AckKind::PubAck),
            Self::PubRec(ack) => ack.encode(AckKind::PubRec),
            Self::PubRel(ack) => ack.encode(AckKind::PubRel),
            Self::PubComp(ack) => ack.encode(AckKind::PubComp),
            Self::Subscribe(subscribe) => subscribe.encode(),
            Self::Unsubscribe(unsubscribe) => unsubscribe.encode(),
            Self::PingReq => Ok(vec![
                PacketType::PingReq.code() << 4,
                0,
            ]),
            Self::Disconnect(disconnect) => disconnect.encode(),
            Self::ConnAck(_) | Self::SubAck(_) | Self::UnsubAck(_) | Self::PingResp => Err(
                MqttError::unsupported("server packets cannot be encoded by the client"),
            ),
        }
    }
}

/// Decodes one packet from a byte stream.
///
/// Returns the packet and the number of bytes it consumed. An incomplete frame
/// is an `Incomplete` error, so a stream reader can wait for more bytes without
/// treating the input as malformed. The frame is rejected when its total size
/// exceeds `max_packet_size`; a value of zero means [`MAX_REMAINING_LENGTH`]
/// plus the fixed header.
pub fn decode_packet(bytes: &[u8], max_packet_size: usize) -> Result<(Packet, usize), MqttError> {
    let first = *bytes
        .first()
        .ok_or_else(|| MqttError::incomplete("no packet header yet"))?;
    let packet_type = PacketType::from_code(first >> 4)
        .ok_or_else(|| MqttError::malformed(format!("reserved packet type {}", first >> 4)))?;
    let flags = first & 0x0f;
    let (remaining, header) = decode_remaining_length(bytes)?;
    let total = header
        .checked_add(remaining)
        .ok_or_else(|| MqttError::malformed("packet length overflows"))?;
    let limit = if max_packet_size == 0 {
        MAX_REMAINING_LENGTH + 5
    } else {
        max_packet_size
    };
    if total > limit {
        return Err(MqttError::too_large(format!(
            "{total}-byte packet exceeds the {limit}-byte maximum"
        )));
    }
    if bytes.len() < total {
        return Err(MqttError::incomplete(format!(
            "{remaining}-byte body needs more bytes"
        )));
    }
    let body = &bytes[header..total];
    let packet = decode_body(packet_type, flags, body)?;
    Ok((packet, total))
}

fn decode_remaining_length(bytes: &[u8]) -> Result<(usize, usize), MqttError> {
    let mut value = 0_usize;
    let mut multiplier = 1_usize;
    let mut index = 1;
    loop {
        let Some(&byte) = bytes.get(index) else {
            return Err(MqttError::incomplete("remaining length is incomplete"));
        };
        index += 1;
        value += usize::from(byte & 0x7f) * multiplier;
        if byte & 0x80 == 0 {
            if multiplier > 1 && byte == 0 {
                return Err(MqttError::malformed(
                    "remaining length is not minimally encoded",
                ));
            }
            return Ok((value, index));
        }
        multiplier *= 128;
        if multiplier > 128 * 128 * 128 {
            return Err(MqttError::malformed(
                "remaining length exceeds four bytes",
            ));
        }
    }
}

fn decode_body(packet_type: PacketType, flags: u8, body: &[u8]) -> Result<Packet, MqttError> {
    let mut reader = Reader::new(body);
    let packet = match packet_type {
        PacketType::ConnAck => {
            require_flags(packet_type, flags, 0)?;
            let ack_flags = reader.u8()?;
            if ack_flags & !0x01 != 0 {
                return Err(MqttError::malformed(
                    "CONNACK reserved flag bits must be zero",
                ));
            }
            let session_present = ack_flags & 0x01 != 0;
            let reason = reader.u8()?;
            if reason != 0 && session_present {
                return Err(MqttError::malformed(
                    "CONNACK cannot report a session present on failure",
                ));
            }
            let properties = Properties::decode(&mut reader)?;
            Packet::ConnAck(ConnAck {
                session_present,
                reason,
                properties,
            })
        }
        PacketType::Publish => {
            let qos = QoS::from_bits((flags >> 1) & 0x03).ok_or_else(|| {
                MqttError::malformed("PUBLISH carries the reserved QoS value 3")
            })?;
            let dup = flags & 0x08 != 0;
            if dup && qos == QoS::AtMostOnce {
                return Err(MqttError::malformed(
                    "PUBLISH with QoS 0 must not set the duplicate flag",
                ));
            }
            let retain = flags & 0x01 != 0;
            let topic = reader.string()?;
            if !topic::is_valid_topic_name(&topic) {
                return Err(MqttError::malformed(
                    "PUBLISH topic is not a valid topic name",
                ));
            }
            let packet_id = if qos.requires_packet_id() {
                let id = reader.u16()?;
                if id == 0 {
                    return Err(MqttError::malformed(
                        "PUBLISH packet identifier must not be zero",
                    ));
                }
                Some(id)
            } else {
                None
            };
            let properties = Properties::decode(&mut reader)?;
            if properties.topic_alias.is_some() {
                return Err(MqttError::unsupported(
                    "topic aliases are not supported by this client",
                ));
            }
            Packet::Publish(Publish {
                topic,
                payload: reader.rest().to_vec(),
                qos,
                retain,
                dup,
                packet_id,
                properties,
            })
        }
        PacketType::PubAck | PacketType::PubRec | PacketType::PubRel | PacketType::PubComp => {
            let ack_kind = match packet_type {
                PacketType::PubAck => AckKind::PubAck,
                PacketType::PubRec => AckKind::PubRec,
                PacketType::PubRel => AckKind::PubRel,
                _ => AckKind::PubComp,
            };
            require_flags(packet_type, flags, ack_kind.flags())?;
            let ack = decode_ack(&mut reader)?;
            match packet_type {
                PacketType::PubAck => Packet::PubAck(ack),
                PacketType::PubRec => Packet::PubRec(ack),
                PacketType::PubRel => Packet::PubRel(ack),
                _ => Packet::PubComp(ack),
            }
        }
        PacketType::SubAck => {
            require_flags(packet_type, flags, 0)?;
            let packet_id = decode_packet_id(&mut reader)?;
            let properties = Properties::decode(&mut reader)?;
            let reasons = reader.rest().to_vec();
            if reasons.is_empty() {
                return Err(MqttError::malformed("SUBACK has no reason codes"));
            }
            if let Some(bad) = reasons.iter().find(|code| !is_suback_reason(**code)) {
                return Err(MqttError::malformed(format!(
                    "SUBACK reason code {bad:#04X} is not valid"
                )));
            }
            Packet::SubAck(SubAck {
                packet_id,
                properties,
                reasons,
            })
        }
        PacketType::UnsubAck => {
            require_flags(packet_type, flags, 0)?;
            let packet_id = decode_packet_id(&mut reader)?;
            let properties = Properties::decode(&mut reader)?;
            Packet::UnsubAck(UnsubAck {
                packet_id,
                properties,
                reasons: reader.rest().to_vec(),
            })
        }
        PacketType::PingResp => {
            require_flags(packet_type, flags, 0)?;
            if !body.is_empty() {
                return Err(MqttError::malformed("PINGRESP must not carry a payload"));
            }
            Packet::PingResp
        }
        PacketType::Disconnect => {
            require_flags(packet_type, flags, 0)?;
            if body.is_empty() {
                Packet::Disconnect(Disconnect {
                    reason: 0,
                    properties: Properties::default(),
                })
            } else {
                let reason = reader.u8()?;
                let properties = Properties::decode(&mut reader)?;
                Packet::Disconnect(Disconnect { reason, properties })
            }
        }
        PacketType::Auth => {
            return Err(MqttError::unsupported(
                "enhanced authentication (AUTH) is not supported",
            ));
        }
        PacketType::Connect
        | PacketType::Subscribe
        | PacketType::Unsubscribe
        | PacketType::PingReq => {
            return Err(MqttError::protocol(format!(
                "client received server-forbidden {}",
                packet_type.as_str()
            )));
        }
    };
    if packet_type != PacketType::Publish && reader.remaining() > 0 {
        return Err(MqttError::malformed(format!(
            "{} has {} trailing bytes",
            packet_type.as_str(),
            reader.remaining()
        )));
    }
    Ok(packet)
}

fn decode_ack(reader: &mut Reader<'_>) -> Result<Ack, MqttError> {
    let packet_id = decode_packet_id(reader)?;
    if reader.remaining() == 0 {
        return Ok(Ack {
            packet_id,
            reason: 0,
        });
    }
    let reason = reader.u8()?;
    let _properties = Properties::decode(reader)?;
    Ok(Ack { packet_id, reason })
}

fn decode_packet_id(reader: &mut Reader<'_>) -> Result<u16, MqttError> {
    let id = reader.u16()?;
    if id == 0 {
        return Err(MqttError::malformed("packet identifier must not be zero"));
    }
    Ok(id)
}

fn is_suback_reason(code: u8) -> bool {
    matches!(code, 0x00 | 0x01 | 0x02 | 0x80 | 0x83 | 0x87 | 0x8F | 0x91 | 0x97 | 0x9E | 0xA1 | 0xA2)
}

fn require_flags(packet_type: PacketType, flags: u8, expected: u8) -> Result<(), MqttError> {
    if flags == expected {
        Ok(())
    } else {
        Err(MqttError::malformed(format!(
            "{} fixed-header flags {flags:#06b} are not {expected:#06b}",
            packet_type.as_str()
        )))
    }
}

fn finish(packet_type: u8, flags: u8, body: Writer) -> Result<Vec<u8>, MqttError> {
    if body.buf.len() > MAX_REMAINING_LENGTH {
        return Err(MqttError::too_large(
            "packet body exceeds the maximum remaining length",
        ));
    }
    let mut out = Writer::default();
    out.u8((packet_type << 4) | flags);
    out.varint(u32::try_from(body.buf.len()).expect("length checked above"))?;
    out.raw(&body.buf);
    Ok(out.buf)
}

#[derive(Default)]
struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.buf.extend_from_slice(&value.to_be_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.buf.extend_from_slice(&value.to_be_bytes());
    }

    fn varint(&mut self, mut value: u32) -> Result<(), MqttError> {
        if usize::try_from(value).unwrap_or(usize::MAX) > MAX_REMAINING_LENGTH {
            return Err(MqttError::too_large(
                "variable integer exceeds the four-byte maximum",
            ));
        }
        loop {
            let mut byte = u8::try_from(value % 128).expect("masked value fits a byte");
            value /= 128;
            if value > 0 {
                byte |= 0x80;
            }
            self.buf.push(byte);
            if value == 0 {
                return Ok(());
            }
        }
    }

    fn string(&mut self, value: &str) -> Result<(), MqttError> {
        if value.len() > usize::from(u16::MAX) {
            return Err(MqttError::too_large("string exceeds 65 535 bytes"));
        }
        self.u16(u16::try_from(value.len()).expect("length checked above"));
        self.buf.extend_from_slice(value.as_bytes());
        Ok(())
    }

    fn binary(&mut self, value: &[u8]) -> Result<(), MqttError> {
        if value.len() > usize::from(u16::MAX) {
            return Err(MqttError::too_large("binary data exceeds 65 535 bytes"));
        }
        self.u16(u16::try_from(value.len()).expect("length checked above"));
        self.buf.extend_from_slice(value);
        Ok(())
    }

    fn raw(&mut self, value: &[u8]) {
        self.buf.extend_from_slice(value);
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    const fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn u8(&mut self) -> Result<u8, MqttError> {
        let byte = *self
            .bytes
            .get(self.pos)
            .ok_or_else(|| MqttError::malformed("packet body is truncated"))?;
        self.pos += 1;
        Ok(byte)
    }

    fn u16(&mut self) -> Result<u16, MqttError> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, MqttError> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn varint(&mut self) -> Result<u32, MqttError> {
        let mut value = 0_u32;
        let mut multiplier = 1_u32;
        for index in 0..4 {
            let byte = self.u8()?;
            value += u32::from(byte & 0x7f) * multiplier;
            if byte & 0x80 == 0 {
                if index > 0 && byte == 0 {
                    return Err(MqttError::malformed(
                        "variable integer is not minimally encoded",
                    ));
                }
                return Ok(value);
            }
            multiplier *= 128;
        }
        Err(MqttError::malformed(
            "variable integer exceeds four bytes",
        ))
    }

    fn string(&mut self) -> Result<String, MqttError> {
        let length = usize::from(self.u16()?);
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| MqttError::malformed("string is not valid UTF-8"))
    }

    fn binary(&mut self) -> Result<Vec<u8>, MqttError> {
        let length = usize::from(self.u16()?);
        Ok(self.take(length)?.to_vec())
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], MqttError> {
        let end = self
            .pos
            .checked_add(length)
            .ok_or_else(|| MqttError::malformed("length overflows"))?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| MqttError::malformed("packet body is truncated"))?;
        self.pos = end;
        Ok(slice)
    }

    fn rest(&mut self) -> &'a [u8] {
        let slice = &self.bytes[self.pos..];
        self.pos = self.bytes.len();
        slice
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connect() -> Connect {
        Connect {
            client_id: "c".to_owned(),
            clean_start: true,
            keep_alive: 30,
            session_expiry: 0,
            receive_maximum: 10,
            maximum_packet_size: None,
            username: None,
            password: None,
            will: None,
        }
    }

    #[test]
    fn packet_type_codes_round_trip() {
        for code in 1..=15_u8 {
            let packet_type = PacketType::from_code(code).unwrap();
            assert_eq!(packet_type.code(), code);
            assert!(!packet_type.as_str().is_empty());
        }
        assert_eq!(PacketType::from_code(0), None);
        assert_eq!(PacketType::from_code(16), None);
    }

    #[test]
    fn qos_values_are_bounded() {
        assert_eq!(QoS::from_bits(0), Some(QoS::AtMostOnce));
        assert_eq!(QoS::from_bits(1), Some(QoS::AtLeastOnce));
        assert_eq!(QoS::from_bits(2), Some(QoS::ExactlyOnce));
        assert_eq!(QoS::from_bits(3), None);
        assert_eq!(QoS::ExactlyOnce.bits(), 2);
        assert!(QoS::AtLeastOnce.requires_packet_id());
        assert!(!QoS::AtMostOnce.requires_packet_id());
    }

    #[test]
    fn connect_encodes_the_mqtt5_shape() {
        let bytes = connect().encode().unwrap();
        assert_eq!(
            bytes,
            vec![
                0x10, 0x16, // fixed header, remaining length 22
                0x00, 0x04, b'M', b'Q', b'T', b'T', // protocol name
                0x05, // MQTT 5.0
                0x02, // clean start
                0x00, 0x1E, // keepalive 30
                0x08, // property length 8
                0x11, 0x00, 0x00, 0x00, 0x00, // session expiry 0
                0x21, 0x00, 0x0A, // receive maximum 10
                0x00, 0x01, b'c', // client id "c"
            ]
        );
    }

    #[test]
    fn connect_encodes_will_credentials_and_properties() {
        let mut connect = connect();
        connect.client_id = "line1-plc".to_owned();
        connect.maximum_packet_size = Some(4096);
        connect.username = Some("plc".to_owned());
        connect.password = Some(b"secret".to_vec());
        connect.will = Some(Will {
            topic: "line1/plc/status".to_owned(),
            payload: b"offline".to_vec(),
            qos: QoS::AtLeastOnce,
            retain: true,
            delay_interval: Some(5),
            payload_format_indicator: Some(1),
            content_type: Some("text/plain".to_owned()),
        });
        let bytes = connect.encode().unwrap();
        assert_eq!(bytes[0], 0x10);
        assert_eq!(bytes[9] & 0x02, 0x02, "clean start");
        assert_eq!(bytes[9] & 0x04, 0x04, "will flag");
        assert_eq!(bytes[9] & 0x20, 0x20, "will retain");
        assert_eq!(bytes[9] & 0x18, 0x08, "will QoS 1");
        assert_eq!(bytes[9] & 0x80, 0x80, "username flag");
        assert_eq!(bytes[9] & 0x40, 0x40, "password flag");
        assert!(bytes.windows(6).any(|window| window == b"secret"));
        assert!(bytes.windows(7).any(|window| window == b"offline"));
    }

    #[test]
    fn connect_rejects_a_password_without_a_username() {
        let mut connect = connect();
        connect.password = Some(b"secret".to_vec());
        assert!(connect.encode().is_err());
    }

    #[test]
    fn connack_decodes_session_reason_and_properties() {
        let bytes = [0x20, 0x09, 0x01, 0x00, 0x06, 0x13, 0x00, 0x0A, 0x1F, 0x00, 0x00];
        let (packet, used) = decode_packet(&bytes, 0).unwrap();
        assert_eq!(used, bytes.len());
        let Packet::ConnAck(ack) = packet else {
            panic!("expected CONNACK");
        };
        assert!(ack.session_present);
        assert!(ack.is_success());
        assert_eq!(ack.properties.server_keep_alive, Some(10));
    }

    #[test]
    fn connack_rejects_session_present_on_failure() {
        let bytes = [0x20, 0x03, 0x01, 0x80, 0x00];
        assert!(decode_packet(&bytes, 0).is_err());
    }

    #[test]
    fn publish_decodes_topic_payload_and_qos() {
        let bytes = [0x30, 0x08, 0x00, 0x03, b'a', b'/', b'b', 0x00, 0x01, 0x02];
        let (packet, used) = decode_packet(&bytes, 0).unwrap();
        assert_eq!(used, bytes.len());
        let Packet::Publish(publish) = packet else {
            panic!("expected PUBLISH");
        };
        assert_eq!(publish.topic, "a/b");
        assert_eq!(publish.payload, vec![1, 2]);
        assert_eq!(publish.qos, QoS::AtMostOnce);
        assert_eq!(publish.packet_id, None);
        assert!(!publish.retain);
    }

    #[test]
    fn publish_decodes_qos1_round_trip() {
        let publish = Publish {
            topic: "line1/speed".to_owned(),
            payload: b"12.5".to_vec(),
            qos: QoS::AtLeastOnce,
            retain: true,
            dup: false,
            packet_id: Some(7),
            properties: Properties {
                payload_format_indicator: Some(1),
                content_type: Some("text/plain".to_owned()),
                user_properties: vec![("unit".to_owned(), "rpm".to_owned())],
                ..Properties::default()
            },
        };
        let bytes = publish.encode().unwrap();
        let (decoded, used) = decode_packet(&bytes, 0).unwrap();
        assert_eq!(used, bytes.len());
        assert_eq!(decoded, Packet::Publish(publish));
    }

    #[test]
    fn publish_rejects_bad_topics_without_a_packet_id() {
        let mut publish = Publish {
            topic: "line1/#".to_owned(),
            payload: Vec::new(),
            qos: QoS::AtLeastOnce,
            retain: false,
            dup: false,
            packet_id: Some(1),
            properties: Properties::default(),
        };
        assert!(publish.encode().is_err());
        publish.topic = "line1/speed".to_owned();
        publish.packet_id = None;
        assert!(publish.encode().is_err());
        publish.qos = QoS::AtMostOnce;
        assert!(publish.encode().is_ok());
    }

    #[test]
    fn acks_round_trip_with_and_without_a_reason() {
        for kind in [
            AckKind::PubAck,
            AckKind::PubRec,
            AckKind::PubRel,
            AckKind::PubComp,
        ] {
            let ack = Ack {
                packet_id: 42,
                reason: 0,
            };
            let bytes = ack.encode(kind).unwrap();
            assert_eq!(bytes[0] >> 4, kind.packet_type().code());
            assert_eq!(bytes[0] & 0x0f, kind.flags());
            assert_eq!(&bytes[2..], &[0x00, 42]);
            let with_reason = Ack {
                packet_id: 42,
                reason: 0x10,
            };
            let bytes = with_reason.encode(kind).unwrap();
            assert_eq!(bytes.len(), 6);
            assert_eq!(bytes[4], 0x10);
        }
        assert!(
            Ack {
                packet_id: 0,
                reason: 0
            }
            .encode(AckKind::PubAck)
            .is_err()
        );
    }

    #[test]
    fn suback_and_unsuback_decode() {
        let bytes = [0x90, 0x04, 0x00, 0x07, 0x00, 0x01];
        let (packet, _) = decode_packet(&bytes, 0).unwrap();
        let Packet::SubAck(suback) = packet else {
            panic!("expected SUBACK");
        };
        assert_eq!(suback.packet_id, 7);
        assert_eq!(suback.reasons, vec![0x01]);

        let bytes = [0xB0, 0x03, 0x00, 0x07, 0x00];
        let (packet, _) = decode_packet(&bytes, 0).unwrap();
        let Packet::UnsubAck(unsuback) = packet else {
            panic!("expected UNSUBACK");
        };
        assert_eq!(unsuback.packet_id, 7);
        assert!(unsuback.reasons.is_empty());
    }

    #[test]
    fn subscribe_encodes_flags_and_options() {
        let subscribe = Subscribe {
            packet_id: 3,
            properties: Properties::default(),
            filters: vec![
                Subscription {
                    filter: "line1/+/speed".to_owned(),
                    qos: QoS::AtLeastOnce,
                    no_local: false,
                    retain_as_published: true,
                    retain_handling: 1,
                },
                Subscription {
                    filter: "$share/plc/line1/#".to_owned(),
                    qos: QoS::AtMostOnce,
                    no_local: true,
                    retain_as_published: false,
                    retain_handling: 2,
                },
            ],
        };
        let bytes = subscribe.encode().unwrap();
        assert_eq!(bytes[0], 0x82);
        assert!(bytes.windows(11).any(|w| w == b"line1/+/spe"));
        assert!(bytes.iter().any(|byte| byte & 0xF0 == 0x10));
        assert!(bytes.iter().any(|byte| byte & 0xF0 == 0x20));
        let mut empty = subscribe.clone();
        empty.filters.clear();
        assert!(empty.encode().is_err());
    }

    #[test]
    fn unsubscribe_and_disconnect_encode() {
        let unsubscribe = Unsubscribe {
            packet_id: 9,
            properties: Properties::default(),
            filters: vec!["line1/speed".to_owned()],
        };
        let bytes = unsubscribe.encode().unwrap();
        assert_eq!(bytes[0], 0xA2);
        let disconnect = Disconnect {
            reason: 0,
            properties: Properties::default(),
        };
        assert_eq!(disconnect.encode().unwrap(), vec![0xE0, 0x00]);
        let disconnect = Disconnect {
            reason: 0x04,
            properties: Properties::default(),
        };
        assert_eq!(disconnect.encode().unwrap(), vec![0xE0, 0x02, 0x04, 0x00]);
    }

    #[test]
    fn pingreq_and_pingresp() {
        assert_eq!(Packet::PingReq.encode().unwrap(), vec![0xC0, 0x00]);
        let (packet, used) = decode_packet(&[0xD0, 0x00], 0).unwrap();
        assert_eq!(packet, Packet::PingResp);
        assert_eq!(used, 2);
    }

    #[test]
    fn disconnect_decodes_with_and_without_reason() {
        let (packet, _) = decode_packet(&[0xE0, 0x00], 0).unwrap();
        assert_eq!(
            packet,
            Packet::Disconnect(Disconnect {
                reason: 0,
                properties: Properties::default()
            })
        );
        let (packet, _) = decode_packet(&[0xE0, 0x02, 0x8E, 0x00], 0).unwrap();
        let Packet::Disconnect(disconnect) = packet else {
            panic!("expected DISCONNECT");
        };
        assert_eq!(disconnect.reason, 0x8E);
    }

    #[test]
    fn incomplete_frames_report_incomplete_not_malformed() {
        assert!(decode_packet(&[], 0).unwrap_err().is_incomplete());
        assert!(decode_packet(&[0x30], 0).unwrap_err().is_incomplete());
        assert!(decode_packet(&[0x30, 0x05, 0x00], 0).unwrap_err().is_incomplete());
        // A complete frame with an invalid topic is malformed, not incomplete.
        assert!(!decode_packet(&[0x30, 0x03, 0x00, 0x01, b'#'], 0)
            .unwrap_err()
            .is_incomplete());
    }

    #[test]
    fn malformed_encodings_are_rejected() {
        // Overlong remaining length.
        assert!(decode_packet(&[0x30, 0x80, 0x00], 0).is_err());
        // Five-byte remaining length.
        assert!(decode_packet(&[0x30, 0x80, 0x80, 0x80, 0x80, 0x01], 0).is_err());
        // PUBLISH with QoS 3.
        assert!(decode_packet(&[0x36, 0x03, 0x00, 0x01, b'a'], 0).is_err());
        // PUBLISH with DUP and QoS 0.
        assert!(decode_packet(&[0x38, 0x03, 0x00, 0x01, b'a'], 0).is_err());
        // PUBLISH with an empty topic.
        assert!(decode_packet(&[0x30, 0x02, 0x00, 0x00], 0).is_err());
        // PUBLISH with a wildcard topic.
        assert!(decode_packet(&[0x30, 0x03, 0x00, 0x01, b'#'], 0).is_err());
        // CONNACK with reserved flags set.
        assert!(decode_packet(&[0x21, 0x03, 0x00, 0x00, 0x00], 0).is_err());
        // PINGRESP with a body.
        assert!(decode_packet(&[0xD0, 0x01, 0x00], 0).is_err());
        // Reserved packet type zero.
        assert!(decode_packet(&[0x00, 0x00], 0).is_err());
    }

    #[test]
    fn unknown_and_duplicate_properties_are_rejected() {
        // PUBLISH with an unknown property id 0x7F.
        let bytes = [0x30, 0x06, 0x00, 0x01, b'a', 0x02, 0x7F, 0x00];
        assert!(decode_packet(&bytes, 0).is_err());
        // PUBLISH with two payload format indicators.
        let bytes = [0x30, 0x08, 0x00, 0x01, b'a', 0x04, 0x01, 0x01, 0x01, 0x00];
        assert!(decode_packet(&bytes, 0).is_err());
        // CONNACK with a zero receive maximum.
        let bytes = [0x20, 0x06, 0x00, 0x00, 0x03, 0x21, 0x00, 0x00];
        assert!(decode_packet(&bytes, 0).is_err());
    }

    #[test]
    fn topic_alias_is_rejected_as_unsupported() {
        let bytes = [0x30, 0x08, 0x00, 0x01, b'a', 0x03, 0x23, 0x00, 0x01, 0x00];
        let error = decode_packet(&bytes, 0).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported);
    }

    #[test]
    fn maximum_packet_size_is_enforced() {
        let bytes = [0x30, 0x08, 0x00, 0x03, b'a', b'/', b'b', 0x00, 0x01, 0x02];
        assert!(decode_packet(&bytes, 4).is_err());
        assert!(decode_packet(&bytes, bytes.len()).is_ok());
    }

    #[test]
    fn server_packets_cannot_be_encoded_by_the_client() {
        assert!(
            Packet::ConnAck(ConnAck {
                session_present: false,
                reason: 0,
                properties: Properties::default()
            })
            .encode()
            .is_err()
        );
        assert!(Packet::PingResp.encode().is_err());
    }

    #[test]
    fn client_forbidden_packets_are_rejected() {
        let (error, _) = (
            decode_packet(&[0x10, 0x00], 0).unwrap_err(),
            decode_packet(&[0x80, 0x00], 0).unwrap_err(),
        );
        assert_eq!(error.kind(), ErrorKind::Protocol);
        assert_eq!(
            decode_packet(&[0xF0, 0x00], 0).unwrap_err().kind(),
            ErrorKind::Unsupported
        );
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let bytes = [0x40, 0x05, 0x00, 0x07, 0x00, 0x00, 0x00];
        assert!(decode_packet(&bytes, 0).is_err());
    }

    #[test]
    fn oversized_strings_and_binaries_are_rejected() {
        let mut connect = connect();
        connect.client_id = "x".repeat(usize::from(u16::MAX) + 1);
        assert!(connect.encode().is_err());
        let mut publish = Publish {
            topic: "a".to_owned(),
            payload: Vec::new(),
            qos: QoS::AtMostOnce,
            retain: false,
            dup: false,
            packet_id: None,
            properties: Properties::default(),
        };
        publish.topic = "t".repeat(usize::from(u16::MAX) + 1);
        assert!(publish.encode().is_err());
    }

    #[test]
    fn properties_round_trip_all_supported_fields() {
        let properties = Properties {
            payload_format_indicator: Some(1),
            message_expiry_interval: Some(60),
            content_type: Some("text/plain".to_owned()),
            session_expiry_interval: Some(30),
            assigned_client_identifier: Some("assigned".to_owned()),
            server_keep_alive: Some(20),
            request_problem_information: Some(0),
            will_delay_interval: Some(5),
            request_response_information: Some(1),
            response_information: Some("resp".to_owned()),
            server_reference: Some("other".to_owned()),
            reason_string: Some("ok".to_owned()),
            receive_maximum: Some(10),
            topic_alias_maximum: Some(4),
            maximum_qos: Some(1),
            retain_available: Some(1),
            user_properties: vec![
                ("a".to_owned(), "1".to_owned()),
                ("b".to_owned(), "2".to_owned()),
            ],
            maximum_packet_size: Some(1024),
            wildcard_subscription_available: Some(1),
            subscription_identifier_available: Some(1),
            shared_subscription_available: Some(1),
            ..Properties::default()
        };
        let bytes = properties.encode().unwrap();
        let mut reader = Reader::new(&bytes);
        assert_eq!(Properties::decode(&mut reader).unwrap(), properties);
        assert_eq!(reader.remaining(), 0);
    }
}
