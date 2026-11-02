//! Modbus TCP protocol core, register bank, and `DeviceBackend`.
//!
//! This module implements the wire codec, function-code handling, channel map,
//! register bank, and the socket-serving backend for the `modbus-tcp` device
//! kind. The cycle thread only copies values between the bank and the process
//! image; listeners and connection threads perform all socket I/O.
//!
//! The locked contract is WL-5.1-Plan.md §5 (M-2, M-3, M-6, M-7) and
//! §6.2–§6.8. Addresses are zero-based: address 0 is the first element of every
//! table (vendor reference `00001`/`10001`/`30001`/`40001`), and the configured
//! address is written into the PDU without an off-by-one conversion.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use stc_got::error::Error;
use stc_got::json::{self, Json};

use crate::coordinator::{
    BackendChannel, BackendConfig, BackendDeviceConfig, BackendState, BackendStatus, DeviceBackend,
    ImageRange, ProcessImage,
};
use crate::module_info::ImageLayout;

/// Stable diagnostic codes owned by the Modbus TCP backend.
///
/// Phase 1 uses [`INVALID_CONFIG`](INVALID_CONFIG) for channel-map validation;
/// the remaining codes are declared here so phase 2 and phase 5 use one
/// vocabulary.
pub mod codes {
    /// Invalid device or channel configuration.
    pub const INVALID_CONFIG: &str = "STC-E-IO-MODTCP-001";
    /// A cold-start bind or listen failed.
    pub const BIND_FAILURE: &str = "STC-E-IO-MODTCP-002";
    /// A bound channel does not resolve or disagrees with its image range.
    pub const CHANNEL_RESOLVE: &str = "STC-E-IO-MODTCP-003";
    /// A declared Modbus channel has no image binding.
    pub const UNBOUND_CHANNEL: &str = "STC-W-IO-MODTCP-001";
    /// A runtime listener failed and is retrying.
    pub const LISTENER_FAULT: &str = "STC-W-IO-MODTCP-002";
}

/// Maximum Modbus TCP application data unit, including the 7-byte MBAP header.
pub const MAX_ADU: usize = 260;

/// Maximum protocol data unit length accepted on the wire.
pub const MAX_PDU: usize = 253;

/// MBAP header length: transaction, protocol, length, and unit id.
const MBAP_HEADER_LEN: usize = 7;

/// Smallest MBAP length field: unit id plus one PDU byte.
const MIN_MBAP_LENGTH: u16 = 2;

/// Largest MBAP length field: unit id plus a 253-byte PDU.
const MAX_MBAP_LENGTH: u16 = 254;

/// One Modbus data table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Table {
    /// Coils (`0xxxx`), one bit per address. Writable by FC5/FC15.
    Coil,
    /// Discrete inputs (`1xxxx`), one bit per address. Read-only.
    DiscreteInput,
    /// Holding registers (`4xxxx`), sixteen bits per address. Writable by
    /// FC6/FC16.
    HoldingRegister,
    /// Input registers (`3xxxx`), sixteen bits per address. Read-only.
    InputRegister,
}

impl Table {
    /// Every data table, in protocol order.
    pub const ALL: [Self; 4] = [
        Self::Coil,
        Self::DiscreteInput,
        Self::HoldingRegister,
        Self::InputRegister,
    ];

    /// Parses the profile spelling of a table.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "coil" => Some(Self::Coil),
            "discrete-input" => Some(Self::DiscreteInput),
            "holding-register" => Some(Self::HoldingRegister),
            "input-register" => Some(Self::InputRegister),
            _ => None,
        }
    }

    /// Returns the profile spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Coil => "coil",
            Self::DiscreteInput => "discrete-input",
            Self::HoldingRegister => "holding-register",
            Self::InputRegister => "input-register",
        }
    }

    /// Returns `true` for the one-bit tables.
    #[must_use]
    pub const fn is_bit(self) -> bool {
        matches!(self, Self::Coil | Self::DiscreteInput)
    }

    /// Returns `true` for the register tables.
    #[must_use]
    pub const fn is_register(self) -> bool {
        !self.is_bit()
    }

    /// Returns the vendor reference number of zero-based address 0.
    ///
    /// Vendor `0xxxx`/`1xxxx`/`3xxxx`/`4xxxx` references are one-based; the
    /// runtime and profile are always zero-based. This helper exists so tests
    /// and diagnostics can label both notations explicitly.
    #[must_use]
    pub const fn vendor_base(self) -> u32 {
        match self {
            Self::Coil => 1,
            Self::DiscreteInput => 10_001,
            Self::InputRegister => 30_001,
            Self::HoldingRegister => 40_001,
        }
    }

    /// Returns the read function code for this table.
    #[must_use]
    pub const fn read_function(self) -> u8 {
        match self {
            Self::Coil => 0x01,
            Self::DiscreteInput => 0x02,
            Self::HoldingRegister => 0x03,
            Self::InputRegister => 0x04,
        }
    }
}

/// Channel direction relative to the PLC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Client write → PLC input. The bank value is latched into `%I`.
    Input,
    /// PLC output → client read. The bank value is published from `%Q`.
    Output,
}

impl Direction {
    /// Parses a declared channel direction.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "input" => Some(Self::Input),
            "output" => Some(Self::Output),
            _ => None,
        }
    }

    /// Returns the profile spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
        }
    }

    /// Returns `true` for an input channel.
    #[must_use]
    pub const fn is_input(self) -> bool {
        matches!(self, Self::Input)
    }
}

/// Byte placement of an 8-bit register channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BytePlacement {
    /// Least significant byte of the containing register (the default).
    Low,
    /// Most significant byte of the containing register.
    High,
}

impl BytePlacement {
    /// Parses the profile spelling.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "low" => Some(Self::Low),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    /// Returns the profile spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::High => "high",
        }
    }
}

/// Device-level word-order default for 32- and 64-bit register channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceWordOrder {
    /// Most significant 16-bit word first (the default).
    HighFirst,
    /// Least significant 16-bit word first. At 64 bits this reverses all four
    /// words (GHEFCDAB).
    LowFirst,
}

impl DeviceWordOrder {
    /// Parses the device-level spelling.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "high-first" => Some(Self::HighFirst),
            "low-first" => Some(Self::LowFirst),
            _ => None,
        }
    }

    /// Returns the device-level spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HighFirst => "high-first",
            Self::LowFirst => "low-first",
        }
    }

    /// Returns the equivalent per-channel word order.
    #[must_use]
    pub const fn as_word_order(self) -> WordOrder {
        match self {
            Self::HighFirst => WordOrder::HighFirst,
            Self::LowFirst => WordOrder::LowFirst,
        }
    }
}

/// Byte order of a 32- or 64-bit register channel.
///
/// Letters A..H name value bytes from most significant to least significant.
/// The wire order is from the lowest register address upward. Named forms
/// apply per 32-bit half for a 64-bit value, except that `low-first` reverses
/// all four words (GHEFCDAB) versus per-channel `cdab` (CDABGHEF).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WordOrder {
    /// `A B C D` / `A B C D E F G H`.
    HighFirst,
    /// `C D A B` / `G H E F C D A B`.
    LowFirst,
    /// `A B C D` / `A B C D E F G H`.
    Abcd,
    /// `C D A B` / `C D A B G H E F`.
    Cdab,
    /// `D C B A` / `D C B A H G F E`.
    Dcba,
    /// `B A D C` / `B A D C F E H G`.
    Badc,
    /// An explicit byte permutation: positions from most significant (0) to
    /// least significant (`n-1`) in wire order.
    Explicit(Vec<u8>),
}

impl WordOrder {
    /// Parses a named word-order spelling. Integer lists are constructed as
    /// [`WordOrder::Explicit`] by the profile parser.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "high-first" => Some(Self::HighFirst),
            "low-first" => Some(Self::LowFirst),
            "abcd" => Some(Self::Abcd),
            "cdab" => Some(Self::Cdab),
            "dcba" => Some(Self::Dcba),
            "badc" => Some(Self::Badc),
            _ => None,
        }
    }

    /// Returns the named spelling, or `None` for an explicit list.
    #[must_use]
    pub const fn as_str(&self) -> Option<&'static str> {
        match self {
            Self::HighFirst => Some("high-first"),
            Self::LowFirst => Some("low-first"),
            Self::Abcd => Some("abcd"),
            Self::Cdab => Some("cdab"),
            Self::Dcba => Some("dcba"),
            Self::Badc => Some("badc"),
            Self::Explicit(_) => None,
        }
    }

    /// Returns the wire byte permutation for a 32- or 64-bit value.
    ///
    /// The returned positions index the value's big-endian bytes (0 is the
    /// most significant). Validation of an [`WordOrder::Explicit`] list is the
    /// caller's responsibility; [`ChannelMap::build`] performs it.
    #[must_use]
    pub fn permutation(&self, width_bits: u32) -> Option<Vec<u8>> {
        let byte_count = match width_bits {
            32 => 4,
            64 => 8,
            _ => return None,
        };
        let order = match self {
            Self::HighFirst | Self::Abcd => (0..byte_count as u8).collect(),
            Self::LowFirst if byte_count == 4 => vec![2, 3, 0, 1],
            Self::LowFirst => vec![6, 7, 4, 5, 2, 3, 0, 1],
            Self::Cdab if byte_count == 4 => vec![2, 3, 0, 1],
            Self::Cdab => vec![2, 3, 0, 1, 6, 7, 4, 5],
            Self::Dcba if byte_count == 4 => vec![3, 2, 1, 0],
            Self::Dcba => vec![3, 2, 1, 0, 7, 6, 5, 4],
            Self::Badc if byte_count == 4 => vec![1, 0, 3, 2],
            Self::Badc => vec![1, 0, 3, 2, 5, 4, 7, 6],
            Self::Explicit(list) => list.clone(),
        };
        Some(order)
    }
}

/// A channel-map validation failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    /// Stable diagnostic code (`STC-E-IO-MODTCP-001` in phase 1).
    pub code: &'static str,
    /// Human-readable detail naming the offending channel or address.
    pub message: String,
}

impl ConfigError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: codes::INVALID_CONFIG,
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ConfigError {}

/// One channel declared by `stc.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelDeclaration {
    /// Channel name, unique within the device.
    pub name: String,
    /// Channel direction relative to the PLC.
    pub direction: Direction,
    /// Declared width in bits.
    pub width_bits: u32,
}

impl ChannelDeclaration {
    /// Creates a declaration.
    #[must_use]
    pub fn new(name: impl Into<String>, direction: Direction, width_bits: u32) -> Self {
        Self {
            name: name.into(),
            direction,
            width_bits,
        }
    }
}

/// One profile channel-map entry for a declared channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MappedChannel {
    /// Declared channel name.
    pub name: String,
    /// Modbus data table.
    pub table: Table,
    /// Zero-based first address in the table; address 0 is the first element.
    pub address: u16,
    /// Byte placement for an 8-bit register channel.
    pub byte: Option<BytePlacement>,
    /// Per-channel word order for a 32- or 64-bit register channel.
    pub word_order: Option<WordOrder>,
}

impl MappedChannel {
    /// Creates a mapping with the default byte placement and word order.
    #[must_use]
    pub fn new(name: impl Into<String>, table: Table, address: u16) -> Self {
        Self {
            name: name.into(),
            table,
            address,
            byte: None,
            word_order: None,
        }
    }
}

/// Validated channel metadata exposed for coordinator integration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MappedChannelInfo {
    /// Channel name.
    pub name: String,
    /// Channel direction.
    pub direction: Direction,
    /// Modbus data table.
    pub table: Table,
    /// Zero-based first address.
    pub address: u16,
    /// Declared width in bits.
    pub width_bits: u32,
    /// Number of table addresses occupied: 1 for a bit or 8/16-bit register
    /// channel, 2 for 32 bits, and 4 for 64 bits.
    pub span: u16,
    /// Byte placement for an 8-bit register channel.
    pub byte: Option<BytePlacement>,
    /// Effective word order for a 32/64-bit register channel.
    pub word_order: Option<WordOrder>,
}

/// Immutable conversion data for one mapped channel.
#[derive(Clone, Debug)]
struct ChannelData {
    /// Wire byte permutation for a 32/64-bit channel; empty otherwise.
    permutation: Vec<u8>,
    /// Claimed bytes within one register: `0x00FF`, `0xFF00`, or `0xFFFF`.
    byte_mask: u16,
}

/// Per-address claims in a register table.
#[derive(Clone, Debug, Default)]
struct AddressClaims {
    /// Indices of every channel whose span covers this address.
    channels: Vec<usize>,
    /// Word mask of all claimed bytes (input and output channels).
    claimed_mask: u16,
    /// Word mask of bytes claimed by input channels.
    input_mask: u16,
    /// A 32- or 64-bit input channel covers this address.
    has_input_wide: bool,
}

/// A validated device channel map.
///
/// Builds from the declared `stc.toml` channels and the profile entries,
/// enforcing the table/direction/width/span/byte/order matrix. Names are
/// unique, every declared channel has exactly one entry, and every entry names
/// a declared channel.
#[derive(Clone, Debug)]
pub struct ChannelMap {
    channels: Vec<MappedChannelInfo>,
    data: Vec<ChannelData>,
    indices: BTreeMap<String, usize>,
    coils: BTreeMap<u16, usize>,
    discrete_inputs: BTreeMap<u16, usize>,
    holding_registers: BTreeMap<u16, AddressClaims>,
    input_registers: BTreeMap<u16, AddressClaims>,
}

impl ChannelMap {
    /// Validates and builds a channel map.
    ///
    /// `device_order` is the default for 32/64-bit channels without a
    /// per-channel override.
    pub fn build(
        declared: &[ChannelDeclaration],
        mapped: &[MappedChannel],
        device_order: DeviceWordOrder,
    ) -> Result<Self, ConfigError> {
        let mut declarations = BTreeMap::new();
        for declaration in declared {
            if declaration.name.is_empty() {
                return Err(ConfigError::invalid("channel name must not be empty"));
            }
            if declarations
                .insert(declaration.name.as_str(), declaration)
                .is_some()
            {
                return Err(ConfigError::invalid(format!(
                    "duplicate declared channel `{}`",
                    declaration.name
                )));
            }
        }
        let mut entries = BTreeMap::new();
        for entry in mapped {
            if entry.name.is_empty() {
                return Err(ConfigError::invalid("channel map entry name is empty"));
            }
            if entries.insert(entry.name.as_str(), entry).is_some() {
                return Err(ConfigError::invalid(format!(
                    "duplicate channel map entry `{}`",
                    entry.name
                )));
            }
            if !declarations.contains_key(entry.name.as_str()) {
                return Err(ConfigError::invalid(format!(
                    "channel map entry `{}` does not name a declared channel",
                    entry.name
                )));
            }
        }
        for name in declarations.keys() {
            if !entries.contains_key(name) {
                return Err(ConfigError::invalid(format!(
                    "declared channel `{name}` has no profile mapping"
                )));
            }
        }

        let mut channels = Vec::with_capacity(declared.len());
        let mut data = Vec::with_capacity(declared.len());
        for declaration in declared {
            let entry = entries
                .get(declaration.name.as_str())
                .copied()
                .ok_or_else(|| {
                    ConfigError::invalid(format!(
                        "declared channel `{}` has no profile mapping",
                        declaration.name
                    ))
                })?;
            let (info, channel_data) = resolve_channel(declaration, entry, device_order)?;
            channels.push(info);
            data.push(channel_data);
        }

        let mut map = Self {
            channels,
            data,
            indices: BTreeMap::new(),
            coils: BTreeMap::new(),
            discrete_inputs: BTreeMap::new(),
            holding_registers: BTreeMap::new(),
            input_registers: BTreeMap::new(),
        };
        for (index, info) in map.channels.iter().enumerate() {
            map.indices.insert(info.name.clone(), index);
            if info.table.is_bit() {
                let table = match info.table {
                    Table::Coil => &mut map.coils,
                    Table::DiscreteInput => &mut map.discrete_inputs,
                    _ => unreachable!("bit table checked above"),
                };
                if table.insert(info.address, index).is_some() {
                    return Err(ConfigError::invalid(format!(
                        "channel `{}` overlaps another `{}` channel at address {}",
                        info.name,
                        info.table.as_str(),
                        info.address
                    )));
                }
                continue;
            }

            for previous in &map.channels[..index] {
                if previous.table != info.table || !spans_overlap(previous, info) {
                    continue;
                }
                if !is_low_high_pair(previous, info) {
                    return Err(ConfigError::invalid(format!(
                        "channel `{}` at {} address {} overlaps `{}`",
                        info.name,
                        info.table.as_str(),
                        info.address,
                        previous.name
                    )));
                }
            }

            let byte_mask = map.data[index].byte_mask;
            let claims_table = match info.table {
                Table::HoldingRegister => &mut map.holding_registers,
                Table::InputRegister => &mut map.input_registers,
                _ => unreachable!("register table checked above"),
            };
            for offset in 0..info.span {
                let address = u32::from(info.address) + u32::from(offset);
                let address = u16::try_from(address).map_err(|_| {
                    ConfigError::invalid(format!(
                        "channel `{}` span leaves the 16-bit address space",
                        info.name
                    ))
                })?;
                let claims = claims_table.entry(address).or_default();
                claims.channels.push(index);
                claims.claimed_mask |= byte_mask;
                if info.direction.is_input() {
                    claims.input_mask |= byte_mask;
                    if info.width_bits >= 32 {
                        claims.has_input_wide = true;
                    }
                }
            }
        }
        Ok(map)
    }

    /// Returns every mapped channel in declaration order.
    #[must_use]
    pub fn channels(&self) -> &[MappedChannelInfo] {
        &self.channels
    }

    /// Returns one mapped channel by name.
    #[must_use]
    pub fn channel(&self, name: &str) -> Option<&MappedChannelInfo> {
        self.indices.get(name).map(|&index| &self.channels[index])
    }

    /// Returns the number of mapped channels.
    #[must_use]
    pub fn len(&self) -> usize {
        self.channels.len()
    }

    /// Returns `true` when no channels are mapped.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    fn index_of(&self, name: &str) -> Option<usize> {
        self.indices.get(name).copied()
    }
}

/// Resolves one declaration/entry pair and validates the table rules.
fn resolve_channel(
    declaration: &ChannelDeclaration,
    entry: &MappedChannel,
    device_order: DeviceWordOrder,
) -> Result<(MappedChannelInfo, ChannelData), ConfigError> {
    let name = declaration.name.as_str();
    let table = entry.table;
    let width_bits = declaration.width_bits;
    let direction = declaration.direction;

    if direction.is_input() && matches!(table, Table::DiscreteInput | Table::InputRegister) {
        return Err(ConfigError::invalid(format!(
            "channel `{name}` is an input channel on read-only table `{}`",
            table.as_str()
        )));
    }

    if table.is_bit() {
        if width_bits != 1 {
            return Err(ConfigError::invalid(format!(
                "channel `{name}` must be 1 bit on table `{}`, not {width_bits}",
                table.as_str()
            )));
        }
        if entry.byte.is_some() {
            return Err(ConfigError::invalid(format!(
                "channel `{name}` uses `byte` on one-bit table `{}`",
                table.as_str()
            )));
        }
        if entry.word_order.is_some() {
            return Err(ConfigError::invalid(format!(
                "channel `{name}` uses `word-order` on one-bit table `{}`",
                table.as_str()
            )));
        }
        let info = MappedChannelInfo {
            name: declaration.name.clone(),
            direction,
            table,
            address: entry.address,
            width_bits,
            span: 1,
            byte: None,
            word_order: None,
        };
        return Ok((
            info,
            ChannelData {
                permutation: Vec::new(),
                byte_mask: 0,
            },
        ));
    }

    if !matches!(width_bits, 8 | 16 | 32 | 64) {
        return Err(ConfigError::invalid(format!(
            "channel `{name}` has unsupported width {width_bits} on table `{}`",
            table.as_str()
        )));
    }

    let byte = if width_bits == 8 {
        if entry.word_order.is_some() {
            return Err(ConfigError::invalid(format!(
                "channel `{name}` uses `word-order` on an 8-bit channel"
            )));
        }
        Some(entry.byte.unwrap_or(BytePlacement::Low))
    } else {
        if entry.byte.is_some() {
            return Err(ConfigError::invalid(format!(
                "channel `{name}` uses `byte` on a {width_bits}-bit channel"
            )));
        }
        None
    };

    let word_order = if matches!(width_bits, 32 | 64) {
        Some(
            entry
                .word_order
                .clone()
                .unwrap_or_else(|| device_order.as_word_order()),
        )
    } else {
        if entry.word_order.is_some() {
            return Err(ConfigError::invalid(format!(
                "channel `{name}` uses `word-order` on a {width_bits}-bit channel"
            )));
        }
        None
    };

    let span = u16::try_from(width_bits.div_ceil(16))
        .map_err(|_| ConfigError::invalid(format!("channel `{name}` span is too large")))?;
    let end = u32::from(entry.address) + u32::from(span);
    if end > u32::from(u16::MAX) + 1 {
        return Err(ConfigError::invalid(format!(
            "channel `{name}` span at address {} leaves the 16-bit address space",
            entry.address
        )));
    }

    let mut permutation = Vec::new();
    if let Some(order) = &word_order {
        let list = order.permutation(width_bits).ok_or_else(|| {
            ConfigError::invalid(format!(
                "channel `{name}` word order does not apply to {width_bits} bits"
            ))
        })?;
        let expected = usize::try_from(width_bits / 8).unwrap_or(usize::MAX);
        if list.len() != expected {
            return Err(ConfigError::invalid(format!(
                "channel `{name}` word-order list has {} entries for {width_bits} bits",
                list.len()
            )));
        }
        let mut seen = vec![false; expected];
        for &position in &list {
            let position = usize::from(position);
            if position >= expected || seen[position] {
                return Err(ConfigError::invalid(format!(
                    "channel `{name}` word-order list is not a permutation of 0..{}",
                    expected - 1
                )));
            }
            seen[position] = true;
        }
        permutation = list;
    }

    let byte_mask = match width_bits {
        8 => match byte {
            Some(BytePlacement::Low) => 0x00FF,
            Some(BytePlacement::High) => 0xFF00,
            None => 0,
        },
        _ => 0xFFFF,
    };
    let info = MappedChannelInfo {
        name: declaration.name.clone(),
        direction,
        table,
        address: entry.address,
        width_bits,
        span,
        byte,
        word_order,
    };
    Ok((
        info,
        ChannelData {
            permutation,
            byte_mask,
        },
    ))
}

fn spans_overlap(left: &MappedChannelInfo, right: &MappedChannelInfo) -> bool {
    let left_start = u32::from(left.address);
    let left_end = left_start + u32::from(left.span);
    let right_start = u32::from(right.address);
    let right_end = right_start + u32::from(right.span);
    left_start < right_end && right_start < left_end
}

fn is_low_high_pair(left: &MappedChannelInfo, right: &MappedChannelInfo) -> bool {
    left.width_bits == 8
        && right.width_bits == 8
        && left.address == right.address
        && left.byte != right.byte
}

/// A Modbus exception code used on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ExceptionCode {
    /// Unsupported function code (`0x01`).
    IllegalFunction = 0x01,
    /// Unmapped or write-protected address, or an out-of-space range
    /// (`0x02`).
    IllegalDataAddress = 0x02,
    /// Invalid quantity, byte count, or coil value (`0x03`).
    IllegalDataValue = 0x03,
    /// Internal backend failure while serving a valid request (`0x04`).
    ServerDeviceFailure = 0x04,
    /// The request unit id differs from the configured unit (`0x0B`).
    WrongUnitId = 0x0B,
}

impl ExceptionCode {
    /// Returns the wire code.
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }
}

/// Builds an exception response PDU: `function | 0x80`, exception code.
#[must_use]
pub fn exception_pdu(function: u8, code: ExceptionCode) -> Vec<u8> {
    vec![function | 0x80, code.code()]
}

/// Internal failure from a request target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestError {
    IllegalDataAddress,
    IllegalDataValue,
    ServerDeviceFailure,
}

impl RequestError {
    const fn exception(self) -> ExceptionCode {
        match self {
            Self::IllegalDataAddress => ExceptionCode::IllegalDataAddress,
            Self::IllegalDataValue => ExceptionCode::IllegalDataValue,
            Self::ServerDeviceFailure => ExceptionCode::ServerDeviceFailure,
        }
    }
}

/// Operations used by the function-code handler.
///
/// [`RegisterBank`] is the production implementation. The trait is private; it
/// exists so tests can exercise the internal-failure (0x04) path.
trait Target {
    fn read_bits(&self, table: Table, start: u16, quantity: u16) -> Result<Vec<u8>, RequestError>;
    fn read_registers(
        &self,
        table: Table,
        start: u16,
        quantity: u16,
    ) -> Result<Vec<u16>, RequestError>;
    fn write_coil(&mut self, address: u16, value: bool) -> Result<(), RequestError>;
    fn write_register(&mut self, address: u16, value: u16) -> Result<(), RequestError>;
    fn write_coils(&mut self, start: u16, values: &[bool]) -> Result<(), RequestError>;
    fn write_registers(&mut self, start: u16, values: &[u16]) -> Result<(), RequestError>;
}

/// A mutex-friendly register bank for one Modbus server device.
///
/// The bank stores wire-order register words (big-endian per register) and
/// boolean bits for every mapped address, seeded to zero. Cycle-side code uses
/// [`read_channel`](Self::read_channel) and
/// [`write_channel`](Self::write_channel) with logical values; client requests
/// use [`handle_request`]. All request handling is one bank operation, so a
/// multi-address write is atomic with respect to the cycle boundary.
pub struct RegisterBank {
    map: ChannelMap,
    coils: BTreeMap<u16, bool>,
    discrete_inputs: BTreeMap<u16, bool>,
    holding_registers: BTreeMap<u16, u16>,
    input_registers: BTreeMap<u16, u16>,
}

impl RegisterBank {
    /// Creates a zeroed bank for a validated channel map.
    #[must_use]
    pub fn new(map: ChannelMap) -> Self {
        let coils = map.coils.keys().map(|&address| (address, false)).collect();
        let discrete_inputs = map
            .discrete_inputs
            .keys()
            .map(|&address| (address, false))
            .collect();
        let holding_registers = map
            .holding_registers
            .keys()
            .map(|&address| (address, 0))
            .collect();
        let input_registers = map
            .input_registers
            .keys()
            .map(|&address| (address, 0))
            .collect();
        Self {
            map,
            coils,
            discrete_inputs,
            holding_registers,
            input_registers,
        }
    }

    /// Returns the channel map.
    #[must_use]
    pub fn channel_map(&self) -> &ChannelMap {
        &self.map
    }

    /// Reads one channel's logical value.
    ///
    /// Widths map to `u64` as `BOOL` 0/1, `BYTE` 8 bits, `WORD` 16 bits, and
    /// `DWORD`/`LWORD` 32/64 bits. Used by the phase-2 input latch.
    #[must_use]
    pub fn read_channel(&self, name: &str) -> Option<u64> {
        let index = self.map.index_of(name)?;
        let info = self.map.channels.get(index)?.clone();
        match info.table {
            Table::Coil => Some(u64::from(*self.coils.get(&info.address)?)),
            Table::DiscreteInput => Some(u64::from(*self.discrete_inputs.get(&info.address)?)),
            Table::HoldingRegister | Table::InputRegister => {
                self.read_register_channel(&info, index)
            }
        }
    }

    fn read_register_channel(&self, info: &MappedChannelInfo, index: usize) -> Option<u64> {
        let table = match info.table {
            Table::HoldingRegister => &self.holding_registers,
            Table::InputRegister => &self.input_registers,
            _ => return None,
        };
        match info.width_bits {
            8 => {
                let word = *table.get(&info.address)?;
                let value = match info.byte? {
                    BytePlacement::Low => word & 0x00FF,
                    BytePlacement::High => word >> 8,
                };
                Some(u64::from(value))
            }
            16 => Some(u64::from(*table.get(&info.address)?)),
            32 | 64 => {
                let permutation = &self.map.data.get(index)?.permutation;
                let mut wire = Vec::with_capacity(usize::from(info.span) * 2);
                for offset in 0..info.span {
                    let address = info.address.checked_add(offset)?;
                    let word = *table.get(&address)?;
                    wire.push((word >> 8) as u8);
                    wire.push(word as u8);
                }
                let mut logical = vec![0_u8; permutation.len()];
                for (position, &source) in permutation.iter().enumerate() {
                    let source = usize::from(source);
                    *logical.get_mut(source)? = *wire.get(position)?;
                }
                Some(bytes_to_value(&logical))
            }
            _ => None,
        }
    }

    /// Writes one channel's logical value.
    ///
    /// Returns `false` for an unknown channel or a value that does not fit the
    /// declared width. An 8-bit write updates only its selected byte; the byte
    /// claimed by the paired channel (input or output) is preserved. Used by
    /// the phase-2 output publish.
    pub fn write_channel(&mut self, name: &str, value: u64) -> bool {
        let Some(index) = self.map.index_of(name) else {
            return false;
        };
        let info = self.map.channels[index].clone();
        if info.width_bits < 64 && value >> info.width_bits != 0 {
            return false;
        }
        match info.table {
            Table::Coil => {
                let Some(bit) = self.coils.get_mut(&info.address) else {
                    return false;
                };
                *bit = value != 0;
                true
            }
            Table::DiscreteInput => {
                let Some(bit) = self.discrete_inputs.get_mut(&info.address) else {
                    return false;
                };
                *bit = value != 0;
                true
            }
            Table::HoldingRegister | Table::InputRegister => {
                let permutation = self.map.data[index].permutation.clone();
                self.write_register_channel(&info, value, &permutation)
            }
        }
    }

    fn write_register_channel(
        &mut self,
        info: &MappedChannelInfo,
        value: u64,
        permutation: &[u8],
    ) -> bool {
        let table = match info.table {
            Table::HoldingRegister => &mut self.holding_registers,
            Table::InputRegister => &mut self.input_registers,
            _ => return false,
        };
        match info.width_bits {
            8 => {
                let Some(word) = table.get_mut(&info.address) else {
                    return false;
                };
                let byte = value as u8;
                *word = match info.byte {
                    Some(BytePlacement::Low) => (*word & 0xFF00) | u16::from(byte),
                    Some(BytePlacement::High) => (*word & 0x00FF) | (u16::from(byte) << 8),
                    None => return false,
                };
                true
            }
            16 => {
                let Some(word) = table.get_mut(&info.address) else {
                    return false;
                };
                *word = value as u16;
                true
            }
            32 | 64 => {
                let byte_count = permutation.len();
                let logical = value.to_be_bytes();
                let logical = &logical[logical.len() - byte_count..];
                let mut wire = vec![0_u8; byte_count];
                for (position, &source) in permutation.iter().enumerate() {
                    let Some(&byte) = logical.get(usize::from(source)) else {
                        return false;
                    };
                    wire[position] = byte;
                }
                for offset in 0..info.span {
                    let Some(address) = info.address.checked_add(offset) else {
                        return false;
                    };
                    let low = usize::from(offset) * 2;
                    let word = u16::from_be_bytes([wire[low], wire[low + 1]]);
                    let Some(slot) = table.get_mut(&address) else {
                        return false;
                    };
                    *slot = word;
                }
                true
            }
            _ => false,
        }
    }

    fn read_bits_inner(
        &self,
        table: Table,
        start: u16,
        quantity: u16,
    ) -> Result<Vec<u8>, RequestError> {
        if !table.is_bit() || quantity == 0 {
            return Err(RequestError::IllegalDataAddress);
        }
        if leaves_address_space(start, quantity) {
            return Err(RequestError::IllegalDataAddress);
        }
        let bits = match table {
            Table::Coil => &self.coils,
            Table::DiscreteInput => &self.discrete_inputs,
            _ => return Err(RequestError::IllegalDataAddress),
        };
        let mut packed = vec![0_u8; usize::from(quantity.div_ceil(8))];
        for offset in 0..quantity {
            let address = start
                .checked_add(offset)
                .ok_or(RequestError::IllegalDataAddress)?;
            let value = bits.get(&address).ok_or(RequestError::IllegalDataAddress)?;
            if *value {
                let index = usize::from(offset);
                packed[index / 8] |= 1 << (index % 8);
            }
        }
        Ok(packed)
    }

    fn read_registers_inner(
        &self,
        table: Table,
        start: u16,
        quantity: u16,
    ) -> Result<Vec<u16>, RequestError> {
        if !table.is_register() || quantity == 0 {
            return Err(RequestError::IllegalDataAddress);
        }
        if leaves_address_space(start, quantity) {
            return Err(RequestError::IllegalDataAddress);
        }
        let (words, claims) = match table {
            Table::HoldingRegister => (&self.holding_registers, &self.map.holding_registers),
            Table::InputRegister => (&self.input_registers, &self.map.input_registers),
            _ => return Err(RequestError::IllegalDataAddress),
        };
        let mut result = Vec::with_capacity(usize::from(quantity));
        for offset in 0..quantity {
            let address = start
                .checked_add(offset)
                .ok_or(RequestError::IllegalDataAddress)?;
            let claim = claims
                .get(&address)
                .ok_or(RequestError::IllegalDataAddress)?;
            let word = *words
                .get(&address)
                .ok_or(RequestError::ServerDeviceFailure)?;
            result.push(word & claim.claimed_mask);
        }
        Ok(result)
    }

    fn write_coil_inner(&mut self, address: u16, value: bool) -> Result<(), RequestError> {
        let index = *self
            .map
            .coils
            .get(&address)
            .ok_or(RequestError::IllegalDataAddress)?;
        if !self.map.channels[index].direction.is_input() {
            return Err(RequestError::IllegalDataAddress);
        }
        let slot = self
            .coils
            .get_mut(&address)
            .ok_or(RequestError::ServerDeviceFailure)?;
        *slot = value;
        Ok(())
    }

    fn write_coils_inner(&mut self, start: u16, values: &[bool]) -> Result<(), RequestError> {
        if values.is_empty() {
            return Err(RequestError::IllegalDataValue);
        }
        let quantity = u16::try_from(values.len()).map_err(|_| RequestError::IllegalDataValue)?;
        if leaves_address_space(start, quantity) {
            return Err(RequestError::IllegalDataAddress);
        }
        let mut addresses = Vec::with_capacity(values.len());
        for offset in 0..quantity {
            let address = start
                .checked_add(offset)
                .ok_or(RequestError::IllegalDataAddress)?;
            let index = *self
                .map
                .coils
                .get(&address)
                .ok_or(RequestError::IllegalDataAddress)?;
            if !self.map.channels[index].direction.is_input() {
                return Err(RequestError::IllegalDataAddress);
            }
            addresses.push(address);
        }
        for (&address, &value) in addresses.iter().zip(values) {
            let slot = self
                .coils
                .get_mut(&address)
                .ok_or(RequestError::ServerDeviceFailure)?;
            *slot = value;
        }
        Ok(())
    }

    fn write_register_inner(&mut self, address: u16, value: u16) -> Result<(), RequestError> {
        let input_mask = self
            .map
            .holding_registers
            .get(&address)
            .map(|claim| claim.input_mask)
            .ok_or(RequestError::IllegalDataAddress)?;
        let claim = self
            .map
            .holding_registers
            .get(&address)
            .ok_or(RequestError::IllegalDataAddress)?;
        if input_mask == 0 || claim.has_input_wide {
            return Err(RequestError::IllegalDataAddress);
        }
        let current = *self
            .holding_registers
            .get(&address)
            .ok_or(RequestError::ServerDeviceFailure)?;
        let updated = (current & !input_mask) | (value & input_mask);
        let slot = self
            .holding_registers
            .get_mut(&address)
            .ok_or(RequestError::ServerDeviceFailure)?;
        *slot = updated;
        Ok(())
    }

    fn write_registers_inner(&mut self, start: u16, values: &[u16]) -> Result<(), RequestError> {
        if values.is_empty() {
            return Err(RequestError::IllegalDataValue);
        }
        let quantity = u16::try_from(values.len()).map_err(|_| RequestError::IllegalDataValue)?;
        if leaves_address_space(start, quantity) {
            return Err(RequestError::IllegalDataAddress);
        }
        let end = u32::from(start) + u32::from(quantity) - 1;

        let mut touched = BTreeSet::new();
        for offset in 0..quantity {
            let address = start
                .checked_add(offset)
                .ok_or(RequestError::IllegalDataAddress)?;
            let claim = self
                .map
                .holding_registers
                .get(&address)
                .ok_or(RequestError::IllegalDataAddress)?;
            if claim.input_mask == 0 {
                return Err(RequestError::IllegalDataAddress);
            }
            touched.extend(claim.channels.iter().copied());
        }
        for index in touched {
            let info = &self.map.channels[index];
            if info.direction.is_input() && info.width_bits >= 32 {
                let span_end = u32::from(info.address) + u32::from(info.span) - 1;
                if u32::from(start) > u32::from(info.address) || end < span_end {
                    return Err(RequestError::IllegalDataAddress);
                }
            }
        }

        let mut updates = Vec::with_capacity(values.len());
        for (offset, &value) in values.iter().enumerate() {
            let offset = u16::try_from(offset).map_err(|_| RequestError::IllegalDataValue)?;
            let address = start
                .checked_add(offset)
                .ok_or(RequestError::IllegalDataAddress)?;
            let input_mask = self
                .map
                .holding_registers
                .get(&address)
                .map(|claim| claim.input_mask)
                .ok_or(RequestError::IllegalDataAddress)?;
            let current = *self
                .holding_registers
                .get(&address)
                .ok_or(RequestError::ServerDeviceFailure)?;
            updates.push((address, (current & !input_mask) | (value & input_mask)));
        }
        for (address, word) in updates {
            let slot = self
                .holding_registers
                .get_mut(&address)
                .ok_or(RequestError::ServerDeviceFailure)?;
            *slot = word;
        }
        Ok(())
    }
}

impl Target for RegisterBank {
    fn read_bits(&self, table: Table, start: u16, quantity: u16) -> Result<Vec<u8>, RequestError> {
        self.read_bits_inner(table, start, quantity)
    }

    fn read_registers(
        &self,
        table: Table,
        start: u16,
        quantity: u16,
    ) -> Result<Vec<u16>, RequestError> {
        self.read_registers_inner(table, start, quantity)
    }

    fn write_coil(&mut self, address: u16, value: bool) -> Result<(), RequestError> {
        self.write_coil_inner(address, value)
    }

    fn write_register(&mut self, address: u16, value: u16) -> Result<(), RequestError> {
        self.write_register_inner(address, value)
    }

    fn write_coils(&mut self, start: u16, values: &[bool]) -> Result<(), RequestError> {
        self.write_coils_inner(start, values)
    }

    fn write_registers(&mut self, start: u16, values: &[u16]) -> Result<(), RequestError> {
        self.write_registers_inner(start, values)
    }
}

fn bytes_to_value(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0_u64, |value, &byte| (value << 8) | u64::from(byte))
}

fn leaves_address_space(start: u16, quantity: u16) -> bool {
    u32::from(start) + u32::from(quantity) > u32::from(u16::MAX) + 1
}

/// Handles one request PDU and returns the response PDU.
///
/// `unit` is the configured unit id, or `None` to accept and echo any unit.
/// The caller parses the MBAP frame and encodes the response with
/// [`encode_adu`]. Empty PDUs are treated as an unsupported function; the
/// MBAP decoder never produces one because the minimum length is 2.
#[must_use]
pub fn handle_request(
    bank: &mut RegisterBank,
    unit: Option<u8>,
    request_unit: u8,
    pdu: &[u8],
) -> Vec<u8> {
    handle_pdu(bank, unit, request_unit, pdu)
}

fn handle_pdu<T: Target + ?Sized>(
    target: &mut T,
    unit: Option<u8>,
    request_unit: u8,
    pdu: &[u8],
) -> Vec<u8> {
    let function = pdu.first().copied().unwrap_or(0);
    if unit.is_some_and(|configured| configured != request_unit) {
        return exception_pdu(function, ExceptionCode::WrongUnitId);
    }
    let outcome = match function {
        0x01 => read_bits_request(target, Table::Coil, pdu),
        0x02 => read_bits_request(target, Table::DiscreteInput, pdu),
        0x03 => read_registers_request(target, Table::HoldingRegister, pdu),
        0x04 => read_registers_request(target, Table::InputRegister, pdu),
        0x05 => write_single_coil_request(target, pdu),
        0x06 => write_single_register_request(target, pdu),
        0x0F => write_multiple_coils_request(target, pdu),
        0x10 => write_multiple_registers_request(target, pdu),
        _ => return exception_pdu(function, ExceptionCode::IllegalFunction),
    };
    match outcome {
        Ok(response) => response,
        Err(code) => exception_pdu(function, code),
    }
}

fn read_bits_request<T: Target + ?Sized>(
    target: &T,
    table: Table,
    pdu: &[u8],
) -> Result<Vec<u8>, ExceptionCode> {
    if pdu.len() != 5 {
        return Err(ExceptionCode::IllegalDataValue);
    }
    let start = u16::from_be_bytes([pdu[1], pdu[2]]);
    let quantity = u16::from_be_bytes([pdu[3], pdu[4]]);
    if quantity == 0 || quantity > 2000 {
        return Err(ExceptionCode::IllegalDataValue);
    }
    if leaves_address_space(start, quantity) {
        return Err(ExceptionCode::IllegalDataAddress);
    }
    let packed = target
        .read_bits(table, start, quantity)
        .map_err(RequestError::exception)?;
    let byte_count = u8::try_from(packed.len()).map_err(|_| ExceptionCode::ServerDeviceFailure)?;
    let mut response = Vec::with_capacity(2 + packed.len());
    response.push(table.read_function());
    response.push(byte_count);
    response.extend_from_slice(&packed);
    Ok(response)
}

fn read_registers_request<T: Target + ?Sized>(
    target: &T,
    table: Table,
    pdu: &[u8],
) -> Result<Vec<u8>, ExceptionCode> {
    if pdu.len() != 5 {
        return Err(ExceptionCode::IllegalDataValue);
    }
    let start = u16::from_be_bytes([pdu[1], pdu[2]]);
    let quantity = u16::from_be_bytes([pdu[3], pdu[4]]);
    if quantity == 0 || quantity > 125 {
        return Err(ExceptionCode::IllegalDataValue);
    }
    if leaves_address_space(start, quantity) {
        return Err(ExceptionCode::IllegalDataAddress);
    }
    let words = target
        .read_registers(table, start, quantity)
        .map_err(RequestError::exception)?;
    let byte_count =
        u8::try_from(words.len() * 2).map_err(|_| ExceptionCode::ServerDeviceFailure)?;
    let mut response = Vec::with_capacity(2 + words.len() * 2);
    response.push(table.read_function());
    response.push(byte_count);
    for word in words {
        response.extend_from_slice(&word.to_be_bytes());
    }
    Ok(response)
}

fn write_single_coil_request<T: Target + ?Sized>(
    target: &mut T,
    pdu: &[u8],
) -> Result<Vec<u8>, ExceptionCode> {
    if pdu.len() != 5 {
        return Err(ExceptionCode::IllegalDataValue);
    }
    let address = u16::from_be_bytes([pdu[1], pdu[2]]);
    let value = u16::from_be_bytes([pdu[3], pdu[4]]);
    if value != 0xFF00 && value != 0x0000 {
        return Err(ExceptionCode::IllegalDataValue);
    }
    target
        .write_coil(address, value == 0xFF00)
        .map_err(RequestError::exception)?;
    Ok(pdu.to_vec())
}

fn write_single_register_request<T: Target + ?Sized>(
    target: &mut T,
    pdu: &[u8],
) -> Result<Vec<u8>, ExceptionCode> {
    if pdu.len() != 5 {
        return Err(ExceptionCode::IllegalDataValue);
    }
    let address = u16::from_be_bytes([pdu[1], pdu[2]]);
    let value = u16::from_be_bytes([pdu[3], pdu[4]]);
    target
        .write_register(address, value)
        .map_err(RequestError::exception)?;
    Ok(pdu.to_vec())
}

fn write_multiple_coils_request<T: Target + ?Sized>(
    target: &mut T,
    pdu: &[u8],
) -> Result<Vec<u8>, ExceptionCode> {
    if pdu.len() < 6 {
        return Err(ExceptionCode::IllegalDataValue);
    }
    let start = u16::from_be_bytes([pdu[1], pdu[2]]);
    let quantity = u16::from_be_bytes([pdu[3], pdu[4]]);
    let byte_count = pdu[5];
    if quantity == 0 || quantity > 1968 {
        return Err(ExceptionCode::IllegalDataValue);
    }
    if u16::from(byte_count) != quantity.div_ceil(8) {
        return Err(ExceptionCode::IllegalDataValue);
    }
    if pdu.len() != 6 + usize::from(byte_count) {
        return Err(ExceptionCode::IllegalDataValue);
    }
    if leaves_address_space(start, quantity) {
        return Err(ExceptionCode::IllegalDataAddress);
    }
    let mut values = Vec::with_capacity(usize::from(quantity));
    for index in 0..quantity {
        let index = usize::from(index);
        let byte = pdu[6 + index / 8];
        values.push((byte >> (index % 8)) & 1 != 0);
    }
    target
        .write_coils(start, &values)
        .map_err(RequestError::exception)?;
    let mut response = vec![0x0F];
    response.extend_from_slice(&start.to_be_bytes());
    response.extend_from_slice(&quantity.to_be_bytes());
    Ok(response)
}

fn write_multiple_registers_request<T: Target + ?Sized>(
    target: &mut T,
    pdu: &[u8],
) -> Result<Vec<u8>, ExceptionCode> {
    if pdu.len() < 6 {
        return Err(ExceptionCode::IllegalDataValue);
    }
    let start = u16::from_be_bytes([pdu[1], pdu[2]]);
    let quantity = u16::from_be_bytes([pdu[3], pdu[4]]);
    let byte_count = pdu[5];
    if quantity == 0 || quantity > 123 {
        return Err(ExceptionCode::IllegalDataValue);
    }
    let expected = quantity
        .checked_mul(2)
        .ok_or(ExceptionCode::IllegalDataValue)?;
    if u16::from(byte_count) != expected {
        return Err(ExceptionCode::IllegalDataValue);
    }
    if pdu.len() != 6 + usize::from(byte_count) {
        return Err(ExceptionCode::IllegalDataValue);
    }
    if leaves_address_space(start, quantity) {
        return Err(ExceptionCode::IllegalDataAddress);
    }
    let mut values = Vec::with_capacity(usize::from(quantity));
    for index in 0..usize::from(quantity) {
        let offset = 6 + index * 2;
        values.push(u16::from_be_bytes([pdu[offset], pdu[offset + 1]]));
    }
    target
        .write_registers(start, &values)
        .map_err(RequestError::exception)?;
    let mut response = vec![0x10];
    response.extend_from_slice(&start.to_be_bytes());
    response.extend_from_slice(&quantity.to_be_bytes());
    Ok(response)
}

/// One decoded MBAP frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Transaction id echoed from the request.
    pub transaction_id: u16,
    /// Unit id from the request.
    pub unit_id: u8,
    /// Protocol data unit (function code and data).
    pub pdu: Vec<u8>,
}

/// A framing error that closes the connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FramingError {
    /// The protocol id was not zero.
    ProtocolId(u16),
    /// The MBAP length field was outside `2..=254`.
    Length(u16),
    /// A partial ADU remained when the connection closed.
    Truncated,
}

impl fmt::Display for FramingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProtocolId(value) => {
                write!(formatter, "invalid Modbus protocol id {value}")
            }
            Self::Length(value) => {
                write!(formatter, "invalid MBAP length {value}")
            }
            Self::Truncated => formatter.write_str("truncated Modbus TCP frame"),
        }
    }
}

impl std::error::Error for FramingError {}

/// An error encoding an ADU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EncodeError {
    /// An ADU must carry at least one PDU byte.
    EmptyPdu,
    /// The PDU exceeds [`MAX_PDU`].
    PduTooLong(usize),
}

impl fmt::Display for EncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPdu => formatter.write_str("Modbus PDU must not be empty"),
            Self::PduTooLong(length) => {
                write!(formatter, "Modbus PDU length {length} exceeds {MAX_PDU}")
            }
        }
    }
}

impl std::error::Error for EncodeError {}

/// Encodes an MBAP frame around a response PDU.
///
/// The transaction id and unit id are echoed; the protocol id is zero.
pub fn encode_adu(transaction_id: u16, unit_id: u8, pdu: &[u8]) -> Result<Vec<u8>, EncodeError> {
    if pdu.is_empty() {
        return Err(EncodeError::EmptyPdu);
    }
    if pdu.len() > MAX_PDU {
        return Err(EncodeError::PduTooLong(pdu.len()));
    }
    let length = u16::try_from(pdu.len() + 1).expect("PDU length bounded by MAX_PDU");
    let mut adu = Vec::with_capacity(MBAP_HEADER_LEN + pdu.len());
    adu.extend_from_slice(&transaction_id.to_be_bytes());
    adu.extend_from_slice(&0_u16.to_be_bytes());
    adu.extend_from_slice(&length.to_be_bytes());
    adu.push(unit_id);
    adu.extend_from_slice(pdu);
    Ok(adu)
}

/// An incremental Modbus TCP frame decoder.
///
/// Bytes may arrive in arbitrary chunks. Complete ADUs are returned in wire
/// order, supporting pipelined requests on one connection. Framing errors
/// clear the buffer because the connection must close.
#[derive(Clone, Debug, Default)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
}

impl FrameDecoder {
    /// Creates an empty decoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends received bytes.
    pub fn push(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    /// Returns the next complete frame, or `None` when more bytes are needed.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, FramingError> {
        if self.buffer.len() < MBAP_HEADER_LEN {
            return Ok(None);
        }
        let protocol_id = u16::from_be_bytes([self.buffer[2], self.buffer[3]]);
        if protocol_id != 0 {
            self.buffer.clear();
            return Err(FramingError::ProtocolId(protocol_id));
        }
        let length = u16::from_be_bytes([self.buffer[4], self.buffer[5]]);
        if !(MIN_MBAP_LENGTH..=MAX_MBAP_LENGTH).contains(&length) {
            self.buffer.clear();
            return Err(FramingError::Length(length));
        }
        let total = 6 + usize::from(length);
        if self.buffer.len() < total {
            return Ok(None);
        }
        let transaction_id = u16::from_be_bytes([self.buffer[0], self.buffer[1]]);
        let unit_id = self.buffer[6];
        let pdu = self.buffer[MBAP_HEADER_LEN..total].to_vec();
        self.buffer.drain(..total);
        Ok(Some(Frame {
            transaction_id,
            unit_id,
            pdu,
        }))
    }

    /// Appends `data` and returns every complete frame now available.
    pub fn decode(&mut self, data: &[u8]) -> Result<Vec<Frame>, FramingError> {
        self.push(data);
        let mut frames = Vec::new();
        while let Some(frame) = self.next_frame()? {
            frames.push(frame);
        }
        Ok(frames)
    }

    /// Reports a truncated ADU when the connection closes.
    ///
    /// Returns `Ok(())` when the buffer is empty; otherwise clears the partial
    /// frame and returns [`FramingError::Truncated`].
    pub fn finish(&mut self) -> Result<(), FramingError> {
        if self.buffer.is_empty() {
            Ok(())
        } else {
            self.buffer.clear();
            Err(FramingError::Truncated)
        }
    }

    /// Returns the number of buffered, not yet decoded bytes.
    #[must_use]
    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }

    /// Discards buffered bytes.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

/// The data-device kind served by the Modbus TCP backend.
pub const MODBUS_TCP_KIND: &str = "modbus-tcp";

/// The optional read-only status kind served by the same instance.
pub const MODBUS_TCP_STATUS_KIND: &str = "modbus-tcp-status";

/// Default total connection limit for one server device.
const DEFAULT_MAX_CONNECTIONS: usize = 8;

/// Largest accepted `max-connections` value.
const MAX_CONNECTIONS_LIMIT: usize = 128;

/// Default per-connection idle timeout.
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Listener accept poll interval; shutdown observes the stop flag within this.
const ACCEPT_POLL: Duration = Duration::from_millis(10);

/// Connection read poll cap so shutdown does not wait for an idle client.
const CONNECTION_POLL: Duration = Duration::from_millis(100);

/// Per-write socket timeout so a stalled client cannot block shutdown.
const WRITE_TIMEOUT: Duration = Duration::from_millis(500);

/// First retry delay after a runtime listener fault.
const RETRY_BACKOFF_MIN: Duration = Duration::from_millis(100);

/// Upper bound on the listener retry delay.
const RETRY_BACKOFF_MAX: Duration = Duration::from_secs(5);

const LISTENER_LISTENING: u8 = 0;
const LISTENER_FAULTED: u8 = 1;
const LISTENER_STOPPED: u8 = 2;

/// One Modbus data device after configuration validation.
struct ServerPlan {
    name: String,
    unit: Option<u8>,
    max_connections: usize,
    idle_timeout: Duration,
    bind: Vec<SocketAddr>,
    bank: RegisterBank,
    inputs: Vec<BoundChannel>,
    outputs: Vec<BoundChannel>,
    unbound: usize,
}

/// One channel bound to a process-image range.
struct BoundChannel {
    name: String,
    image: ImageRange,
}

/// One channel of a `modbus-tcp-status` device bound to an image range.
#[derive(Clone, Copy)]
struct StatusBinding {
    server: usize,
    field: StatusField,
    image: ImageRange,
}

#[derive(Clone, Copy)]
enum StatusField {
    Listening,
    Listeners,
    BoundListeners,
    Connections,
    Requests,
    Writes,
    Exceptions,
    Malformed,
    LastError,
}

#[derive(Default)]
struct ServerCounters {
    connections: AtomicUsize,
    peak_connections: AtomicUsize,
    requests: AtomicU64,
    reads: AtomicU64,
    writes: AtomicU64,
    exceptions: AtomicU64,
    malformed: AtomicU64,
    last_error: AtomicU32,
}

struct ListenerRuntime {
    state: AtomicU8,
    last_error: AtomicU32,
    connections: AtomicUsize,
    fault_episodes: AtomicU64,
    effective_bind: Mutex<String>,
    #[cfg(test)]
    fault_injection: ListenerFaultInjection,
}

/// Deterministic accept-failure hook used only by lifecycle tests.
///
/// `inject` makes the listener observe one synthetic accept error on its next
/// loop iteration; `hold` keeps it in the faulted/retry state until the test
/// releases it, so the partial-listening state can be observed without racing
/// the retry backoff.
#[cfg(test)]
#[derive(Default)]
struct ListenerFaultInjection {
    inject: AtomicBool,
    hold: AtomicBool,
}

impl ListenerRuntime {
    fn new(effective_bind: String) -> Self {
        Self {
            state: AtomicU8::new(LISTENER_LISTENING),
            last_error: AtomicU32::new(0),
            connections: AtomicUsize::new(0),
            fault_episodes: AtomicU64::new(0),
            effective_bind: Mutex::new(effective_bind),
            #[cfg(test)]
            fault_injection: ListenerFaultInjection::default(),
        }
    }

    fn state_name(&self) -> &'static str {
        match self.state.load(Ordering::SeqCst) {
            LISTENER_LISTENING => "listening",
            LISTENER_FAULTED => "faulted",
            _ => "stopped",
        }
    }

    fn is_listening(&self) -> bool {
        self.state.load(Ordering::SeqCst) == LISTENER_LISTENING
    }
}

struct ServerRuntime {
    counters: ServerCounters,
    listeners: Vec<Arc<ListenerRuntime>>,
    fault: Mutex<Option<String>>,
}

struct ListenerHandle {
    thread: Option<std::thread::JoinHandle<()>>,
}

struct Server {
    name: String,
    bank: Arc<Mutex<RegisterBank>>,
    inputs: Vec<BoundChannel>,
    outputs: Vec<BoundChannel>,
    unbound: usize,
    runtime: Arc<ServerRuntime>,
    listeners: Vec<ListenerHandle>,
}

impl Server {
    fn bound_listeners(&self) -> usize {
        self.runtime
            .listeners
            .iter()
            .filter(|listener| listener.is_listening())
            .count()
    }

    fn is_listening(&self) -> bool {
        !self.runtime.listeners.is_empty() && self.bound_listeners() == self.runtime.listeners.len()
    }

    fn connections(&self) -> usize {
        self.runtime.counters.connections.load(Ordering::SeqCst)
    }

    fn effective_binds(&self) -> String {
        self.runtime
            .listeners
            .iter()
            .map(|listener| lock_mutex(&listener.effective_bind).clone())
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// The Modbus TCP `DeviceBackend`.
///
/// One instance serves every `modbus-tcp` data device and every optional
/// `modbus-tcp-status` device. Each data device owns a register bank, one
/// listener per configured bind, and connection threads bounded by its total
/// `max-connections`. The cycle thread only copies bank values at the two
/// boundaries; no socket operation runs there.
pub struct ModbusTcpBackend {
    servers: Vec<Server>,
    status_bindings: Vec<StatusBinding>,
    status: BackendStatus,
    stop: Arc<AtomicBool>,
    started: bool,
    cycled: bool,
}

impl Default for ModbusTcpBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ModbusTcpBackend {
    /// Creates an unstarted backend.
    #[must_use]
    pub fn new() -> Self {
        Self {
            servers: Vec::new(),
            status_bindings: Vec::new(),
            status: BackendStatus::default(),
            stop: Arc::new(AtomicBool::new(false)),
            started: false,
            cycled: false,
        }
    }

    /// Returns the number of declared Modbus channels without an image
    /// binding, summed across data devices.
    #[must_use]
    pub fn unbound_channels(&self) -> usize {
        self.servers.iter().map(|server| server.unbound).sum()
    }

    /// Forces one listener to observe a synthetic accept failure and holds it
    /// in the faulted/retry state until [`release_listener_fault`] is called.
    #[cfg(test)]
    fn inject_listener_fault(&self, server_index: usize, listener_index: usize) {
        let listener = &self.servers[server_index].runtime.listeners[listener_index];
        listener.fault_injection.hold.store(true, Ordering::SeqCst);
        listener
            .fault_injection
            .inject
            .store(true, Ordering::SeqCst);
    }

    /// Lets an injected listener leave the retry hold and bind again.
    #[cfg(test)]
    fn release_listener_fault(&self, server_index: usize, listener_index: usize) {
        self.servers[server_index].runtime.listeners[listener_index]
            .fault_injection
            .hold
            .store(false, Ordering::SeqCst);
    }

    fn fault_episodes(&self) -> u64 {
        self.servers
            .iter()
            .flat_map(|server| &server.runtime.listeners)
            .map(|listener| listener.fault_episodes.load(Ordering::SeqCst))
            .fold(0, u64::saturating_add)
    }

    fn last_fault(&self) -> Option<String> {
        self.servers
            .iter()
            .find_map(|server| lock_mutex(&server.runtime.fault).clone())
    }

    fn compute_state(&self) -> BackendState {
        if !self.started {
            return BackendState::Stopped;
        }
        if self
            .servers
            .iter()
            .flat_map(|server| &server.runtime.listeners)
            .any(|listener| listener.state.load(Ordering::SeqCst) == LISTENER_FAULTED)
        {
            return BackendState::Faulted;
        }
        if self.cycled {
            BackendState::Running
        } else {
            BackendState::Ready
        }
    }

    fn refresh_state(&mut self) {
        self.status.state = self.compute_state();
    }

    fn status_value(&self, server_index: usize, field: StatusField) -> u64 {
        let Some(server) = self.servers.get(server_index) else {
            return 0;
        };
        match field {
            StatusField::Listening => u64::from(server.is_listening()),
            StatusField::Listeners => server.runtime.listeners.len() as u64,
            StatusField::BoundListeners => server.bound_listeners() as u64,
            StatusField::Connections => server.connections() as u64,
            StatusField::Requests => server.runtime.counters.requests.load(Ordering::SeqCst),
            StatusField::Writes => server.runtime.counters.writes.load(Ordering::SeqCst),
            StatusField::Exceptions => server.runtime.counters.exceptions.load(Ordering::SeqCst),
            StatusField::Malformed => server.runtime.counters.malformed.load(Ordering::SeqCst),
            StatusField::LastError => {
                u64::from(server.runtime.counters.last_error.load(Ordering::SeqCst))
            }
        }
    }
}

impl Drop for ModbusTcpBackend {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

impl DeviceBackend for ModbusTcpBackend {
    fn cold_start(&mut self, config: &BackendConfig) -> std::result::Result<(), Error> {
        let _ = self.shutdown();
        self.servers.clear();
        self.status_bindings.clear();
        self.cycled = false;
        self.status = BackendStatus {
            name: config.name.clone(),
            devices: config.devices.len(),
            state: BackendState::Ready,
            exchanges: 0,
            faults: 0,
            last_fault: None,
        };

        let (plans, status_bindings) = parse_backend_config(config)?;

        // All-or-nothing: bind every configured address before any listener
        // thread is spawned. Dropping a partial set closes it.
        let mut bound: Vec<Vec<(SocketAddr, TcpListener, String)>> = Vec::new();
        for plan in &plans {
            let mut listeners = Vec::with_capacity(plan.bind.len());
            for &address in &plan.bind {
                let listener = TcpListener::bind(address).map_err(|error| {
                    Error::new(
                        codes::BIND_FAILURE,
                        format!(
                            "Modbus device `{}` cannot bind {address}: {error}",
                            plan.name
                        ),
                    )
                })?;
                listener.set_nonblocking(true).map_err(|error| {
                    Error::new(
                        codes::BIND_FAILURE,
                        format!(
                            "Modbus device `{}` cannot configure listener {address}: {error}",
                            plan.name
                        ),
                    )
                })?;
                let effective = listener
                    .local_addr()
                    .map_or_else(|_| address.to_string(), |bound| bound.to_string());
                listeners.push((address, listener, effective));
            }
            bound.push(listeners);
        }

        let stop = Arc::new(AtomicBool::new(false));
        let mut servers: Vec<Server> = Vec::with_capacity(plans.len());
        for (plan, bound_listeners) in plans.into_iter().zip(bound) {
            let listener_runtimes: Vec<Arc<ListenerRuntime>> = bound_listeners
                .iter()
                .map(|(_, _, effective)| Arc::new(ListenerRuntime::new(effective.clone())))
                .collect();
            let runtime = Arc::new(ServerRuntime {
                counters: ServerCounters::default(),
                listeners: listener_runtimes.clone(),
                fault: Mutex::new(None),
            });
            let bank = Arc::new(Mutex::new(plan.bank));
            let mut handles = Vec::with_capacity(bound_listeners.len());
            for (listener_index, ((configured, listener, _), listener_runtime)) in bound_listeners
                .into_iter()
                .zip(listener_runtimes)
                .enumerate()
            {
                let task = ListenerTask {
                    server_index: servers.len(),
                    listener_index,
                    configured,
                    listener,
                    runtime: listener_runtime,
                    server: runtime.clone(),
                    bank: bank.clone(),
                    unit: plan.unit,
                    max_connections: plan.max_connections,
                    idle_timeout: plan.idle_timeout,
                    stop: stop.clone(),
                };
                let name = format!("stc-modbus-{}-{listener_index}", plan.name);
                match std::thread::Builder::new().name(name).spawn(move || {
                    run_listener(task);
                }) {
                    Ok(handle) => handles.push(ListenerHandle {
                        thread: Some(handle),
                    }),
                    Err(error) => {
                        stop.store(true, Ordering::SeqCst);
                        for handle in handles {
                            if let Some(thread) = handle.thread {
                                let _ = thread.join();
                            }
                        }
                        for mut server in servers {
                            for listener in &mut server.listeners {
                                if let Some(thread) = listener.thread.take() {
                                    let _ = thread.join();
                                }
                            }
                        }
                        return Err(Error::new(
                            codes::BIND_FAILURE,
                            format!(
                                "Modbus device `{}` cannot start a listener thread: {error}",
                                plan.name
                            ),
                        ));
                    }
                }
            }
            servers.push(Server {
                name: plan.name,
                bank,
                inputs: plan.inputs,
                outputs: plan.outputs,
                unbound: plan.unbound,
                runtime,
                listeners: handles,
            });
        }

        self.stop = stop;
        self.status_bindings = status_bindings;
        self.servers = servers;
        self.started = true;
        Ok(())
    }

    fn latch_inputs(&mut self, image: &mut ProcessImage) -> std::result::Result<(), Error> {
        for server in &self.servers {
            let bank = lock_mutex(&server.bank);
            for channel in &server.inputs {
                let value = bank.read_channel(&channel.name).ok_or_else(|| {
                    channel_resolve(format!(
                        "input channel `{}` on device `{}` has no bank value",
                        channel.name, server.name
                    ))
                })?;
                if !image.write_bits(
                    channel.image.area,
                    channel.image.offset_bits,
                    channel.image.width_bits,
                    value,
                ) {
                    return Err(channel_resolve(format!(
                        "input channel `{}` on device `{}` leaves the process image",
                        channel.name, server.name
                    )));
                }
            }
        }
        for binding in &self.status_bindings {
            let value = self.status_value(binding.server, binding.field);
            if !image.write_bits(
                binding.image.area,
                binding.image.offset_bits,
                binding.image.width_bits,
                value,
            ) {
                return Err(channel_resolve(format!(
                    "status channel for server{} leaves the process image",
                    binding.server
                )));
            }
        }
        self.cycled = true;
        self.refresh_state();
        Ok(())
    }

    fn publish_outputs(&mut self, image: &ProcessImage) -> std::result::Result<(), Error> {
        for server in &self.servers {
            let mut bank = lock_mutex(&server.bank);
            for channel in &server.outputs {
                let value = image
                    .read_bits(
                        channel.image.area,
                        channel.image.offset_bits,
                        channel.image.width_bits,
                    )
                    .ok_or_else(|| {
                        channel_resolve(format!(
                            "output channel `{}` on device `{}` leaves the process image",
                            channel.name, server.name
                        ))
                    })?;
                if !bank.write_channel(&channel.name, value) {
                    return Err(channel_resolve(format!(
                        "output channel `{}` on device `{}` does not fit the register bank",
                        channel.name, server.name
                    )));
                }
            }
        }
        self.status.exchanges = self.status.exchanges.saturating_add(1);
        self.refresh_state();
        Ok(())
    }

    fn status(&self) -> BackendStatus {
        let mut status = self.status.clone();
        status.state = self.compute_state();
        status.faults = self.fault_episodes();
        status.last_fault = self.last_fault();
        status
    }

    fn diagnostics(&self) -> BTreeMap<String, String> {
        let mut values = BTreeMap::new();
        for (index, server) in self.servers.iter().enumerate() {
            let prefix = format!("server{index}");
            values.insert(format!("{prefix}.bind"), server.effective_binds());
            values.insert(
                format!("{prefix}.listeners"),
                server.runtime.listeners.len().to_string(),
            );
            values.insert(
                format!("{prefix}.bound_listeners"),
                server.bound_listeners().to_string(),
            );
            values.insert(
                format!("{prefix}.connections"),
                server.connections().to_string(),
            );
            values.insert(
                format!("{prefix}.peak_connections"),
                server
                    .runtime
                    .counters
                    .peak_connections
                    .load(Ordering::SeqCst)
                    .to_string(),
            );
            values.insert(
                format!("{prefix}.requests"),
                server
                    .runtime
                    .counters
                    .requests
                    .load(Ordering::SeqCst)
                    .to_string(),
            );
            values.insert(
                format!("{prefix}.reads"),
                server
                    .runtime
                    .counters
                    .reads
                    .load(Ordering::SeqCst)
                    .to_string(),
            );
            values.insert(
                format!("{prefix}.writes"),
                server
                    .runtime
                    .counters
                    .writes
                    .load(Ordering::SeqCst)
                    .to_string(),
            );
            values.insert(
                format!("{prefix}.exceptions"),
                server
                    .runtime
                    .counters
                    .exceptions
                    .load(Ordering::SeqCst)
                    .to_string(),
            );
            values.insert(
                format!("{prefix}.malformed"),
                server
                    .runtime
                    .counters
                    .malformed
                    .load(Ordering::SeqCst)
                    .to_string(),
            );
            values.insert(format!("{prefix}.unbound"), server.unbound.to_string());
            values.insert(
                format!("{prefix}.last_error"),
                server
                    .runtime
                    .counters
                    .last_error
                    .load(Ordering::SeqCst)
                    .to_string(),
            );
            for (listener_index, listener) in server.runtime.listeners.iter().enumerate() {
                let listener_prefix = format!("{prefix}.listener{listener_index}");
                values.insert(
                    format!("{listener_prefix}.bind"),
                    lock_mutex(&listener.effective_bind).clone(),
                );
                values.insert(
                    format!("{listener_prefix}.state"),
                    listener.state_name().to_owned(),
                );
                values.insert(
                    format!("{listener_prefix}.connections"),
                    listener.connections.load(Ordering::SeqCst).to_string(),
                );
                values.insert(
                    format!("{listener_prefix}.last_error"),
                    listener.last_error.load(Ordering::SeqCst).to_string(),
                );
            }
        }
        values
    }

    fn shutdown(&mut self) -> std::result::Result<(), Error> {
        if self.started {
            self.stop.store(true, Ordering::SeqCst);
            for server in &mut self.servers {
                for listener in &mut server.listeners {
                    if let Some(thread) = listener.thread.take() {
                        let _ = thread.join();
                    }
                }
            }
            self.started = false;
        }
        self.status.state = BackendState::Stopped;
        Ok(())
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(codes::INVALID_CONFIG, message)
}

fn channel_resolve(message: impl Into<String>) -> Error {
    Error::new(codes::CHANNEL_RESOLVE, message)
}

fn lock_mutex<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn parse_backend_config(
    config: &BackendConfig,
) -> std::result::Result<(Vec<ServerPlan>, Vec<StatusBinding>), Error> {
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
        .filter(|device| device.kind != MODBUS_TCP_STATUS_KIND)
        .count();

    let mut plans = Vec::new();
    let mut status_bindings = Vec::new();
    for device in &config.devices {
        if device.kind == MODBUS_TCP_STATUS_KIND {
            status_bindings.extend(parse_status_device(device, data_devices, &config.layout)?);
        } else {
            plans.push(parse_server_device(device, &config.layout)?);
        }
    }

    let mut concrete = BTreeSet::new();
    for plan in &plans {
        for &address in &plan.bind {
            if address.port() != 0 && !concrete.insert(address) {
                return Err(invalid(format!(
                    "Modbus bind address {address} is configured more than once"
                )));
            }
        }
    }

    validate_bindings(config)?;
    Ok((plans, status_bindings))
}

fn parse_server_device(
    device: &BackendDeviceConfig,
    layout: &ImageLayout,
) -> std::result::Result<ServerPlan, Error> {
    let Some(config) = device.config.as_deref() else {
        return Err(invalid(format!(
            "Modbus device `{}` has no profile or inline configuration",
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
    for key in ["syncs", "pdo-channels", "dc"] {
        if object.contains_key(key) {
            return Err(invalid(format!(
                "Modbus device `{}` uses the EtherCAT key `{key}`",
                device.name
            )));
        }
    }

    let bind_value = object.get("bind").ok_or_else(|| {
        invalid(format!(
            "Modbus device `{}` is missing its `bind` array",
            device.name
        ))
    })?;
    let entries = bind_value.as_array().ok_or_else(|| {
        invalid(format!(
            "Modbus device `{}` `bind` must be an array of address:port strings",
            device.name
        ))
    })?;
    if entries.is_empty() {
        return Err(invalid(format!(
            "Modbus device `{}` `bind` must contain at least one address",
            device.name
        )));
    }
    let mut bind = Vec::with_capacity(entries.len());
    for entry in entries {
        let text = entry.as_str().ok_or_else(|| {
            invalid(format!(
                "Modbus device `{}` `bind` entries must be strings",
                device.name
            ))
        })?;
        let text = text.trim();
        if text.is_empty() {
            return Err(invalid(format!(
                "Modbus device `{}` has an empty `bind` entry",
                device.name
            )));
        }
        let address = text.parse::<SocketAddr>().map_err(|_| {
            invalid(format!(
                "Modbus device `{}` `bind` entry `{text}` is not a literal IP address:port",
                device.name
            ))
        })?;
        bind.push(address);
    }

    let unit = match object.get("unit") {
        None => None,
        Some(Json::Number(value)) => match u8::try_from(*value) {
            Ok(unit) if (1..=247).contains(&unit) => Some(unit),
            _ => {
                return Err(invalid(format!(
                    "Modbus device `{}` `unit` must be an integer in 1..=247",
                    device.name
                )));
            }
        },
        Some(_) => {
            return Err(invalid(format!(
                "Modbus device `{}` `unit` must be an integer in 1..=247",
                device.name
            )));
        }
    };

    let max_connections = match object.get("max-connections") {
        None => DEFAULT_MAX_CONNECTIONS,
        Some(Json::Number(value)) => match usize::try_from(*value) {
            Ok(limit) if (1..=MAX_CONNECTIONS_LIMIT).contains(&limit) => limit,
            _ => {
                return Err(invalid(format!(
                    "Modbus device `{}` `max-connections` must be an integer in 1..=128",
                    device.name
                )));
            }
        },
        Some(_) => {
            return Err(invalid(format!(
                "Modbus device `{}` `max-connections` must be an integer in 1..=128",
                device.name
            )));
        }
    };

    let idle_timeout = match object.get("idle-timeout") {
        None => DEFAULT_IDLE_TIMEOUT,
        Some(Json::String(text)) => parse_duration(text).ok_or_else(|| {
            invalid(format!(
                "Modbus device `{}` `idle-timeout` must be a positive duration such as `60s`",
                device.name
            ))
        })?,
        Some(_) => {
            return Err(invalid(format!(
                "Modbus device `{}` `idle-timeout` must be a positive duration such as `60s`",
                device.name
            )));
        }
    };

    let device_order = match object.get("word-order") {
        None => DeviceWordOrder::HighFirst,
        Some(Json::String(text)) => DeviceWordOrder::parse(text).ok_or_else(|| {
            invalid(format!(
                "Modbus device `{}` `word-order` must be `high-first` or `low-first`",
                device.name
            ))
        })?,
        Some(_) => {
            return Err(invalid(format!(
                "Modbus device `{}` `word-order` must be `high-first` or `low-first`",
                device.name
            )));
        }
    };

    let mapped = match object.get("channels") {
        None => Vec::new(),
        Some(value) => parse_mapped_channels(device, value)?,
    };
    let declarations = device
        .channels
        .iter()
        .map(|channel| {
            let direction = Direction::parse(&channel.direction).ok_or_else(|| {
                invalid(format!(
                    "channel `{}` on device `{}` has direction `{}`; expected `input` or `output`",
                    channel.name, device.name, channel.direction
                ))
            })?;
            Ok(ChannelDeclaration::new(
                channel.name.clone(),
                direction,
                channel.width_bits,
            ))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let map = ChannelMap::build(&declarations, &mapped, device_order).map_err(|error| {
        Error::new(
            error.code,
            format!("device `{}`: {}", device.name, error.message),
        )
    })?;

    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut unbound = 0;
    for channel in &device.channels {
        let info = map.channel(&channel.name).ok_or_else(|| {
            channel_resolve(format!(
                "channel `{}` on device `{}` has no channel-map entry",
                channel.name, device.name
            ))
        })?;
        if info.width_bits != channel.width_bits {
            return Err(channel_resolve(format!(
                "channel `{}` on device `{}` is declared as {} bits but mapped as {} bits",
                channel.name, device.name, channel.width_bits, info.width_bits
            )));
        }
        let Some(image) = channel.image else {
            unbound += 1;
            continue;
        };
        validate_image(device, channel, image, layout, info.direction)?;
        let bound = BoundChannel {
            name: channel.name.clone(),
            image,
        };
        if info.direction.is_input() {
            inputs.push(bound);
        } else {
            outputs.push(bound);
        }
    }

    Ok(ServerPlan {
        name: device.name.clone(),
        unit,
        max_connections,
        idle_timeout,
        bind,
        bank: RegisterBank::new(map),
        inputs,
        outputs,
        unbound,
    })
}

fn parse_mapped_channels(
    device: &BackendDeviceConfig,
    value: &Json,
) -> std::result::Result<Vec<MappedChannel>, Error> {
    let entries = value.as_array().ok_or_else(|| {
        invalid(format!(
            "Modbus device `{}` `channels` must be an array of tables",
            device.name
        ))
    })?;
    let mut mapped = Vec::with_capacity(entries.len());
    for entry in entries {
        let entry = entry.as_object().ok_or_else(|| {
            invalid(format!(
                "Modbus device `{}` has a `channels` entry that is not a table",
                device.name
            ))
        })?;
        let name = entry
            .get("name")
            .and_then(Json::as_str)
            .ok_or_else(|| {
                invalid(format!(
                    "Modbus device `{}` has a channel map entry without a `name`",
                    device.name
                ))
            })?
            .to_owned();
        let table = entry
            .get("table")
            .and_then(Json::as_str)
            .and_then(Table::parse)
            .ok_or_else(|| {
                invalid(format!(
                    "channel `{name}` on device `{}` has an unknown `table`",
                    device.name
                ))
            })?;
        let address = match entry.get("address") {
            Some(Json::Number(value)) => u16::try_from(*value).map_err(|_| {
                invalid(format!(
                    "channel `{name}` on device `{}` `address` must be in 0..=65535",
                    device.name
                ))
            })?,
            _ => {
                return Err(invalid(format!(
                    "channel `{name}` on device `{}` `address` must be an integer in 0..=65535",
                    device.name
                )));
            }
        };
        let byte = match entry.get("byte") {
            None => None,
            Some(Json::String(text)) => match BytePlacement::parse(text) {
                Some(byte) => Some(byte),
                None => {
                    return Err(invalid(format!(
                        "channel `{name}` on device `{}` `byte` must be `low` or `high`",
                        device.name
                    )));
                }
            },
            Some(_) => {
                return Err(invalid(format!(
                    "channel `{name}` on device `{}` `byte` must be `low` or `high`",
                    device.name
                )));
            }
        };
        let word_order = match entry.get("word-order") {
            None => None,
            Some(Json::String(text)) => match WordOrder::parse(text) {
                Some(order) => Some(order),
                None => {
                    return Err(invalid(format!(
                        "channel `{name}` on device `{}` has an unknown `word-order`",
                        device.name
                    )));
                }
            },
            Some(Json::Array(values)) => {
                let mut order = Vec::with_capacity(values.len());
                for value in values {
                    let Some(position) = value
                        .as_number()
                        .and_then(|number| u8::try_from(number).ok())
                    else {
                        return Err(invalid(format!(
                            "channel `{name}` on device `{}` `word-order` list must contain \
                             non-negative byte positions",
                            device.name
                        )));
                    };
                    order.push(position);
                }
                Some(WordOrder::Explicit(order))
            }
            Some(_) => {
                return Err(invalid(format!(
                    "channel `{name}` on device `{}` `word-order` must be a spelling or an \
                     integer list",
                    device.name
                )));
            }
        };
        mapped.push(MappedChannel {
            name,
            table,
            address,
            byte,
            word_order,
        });
    }
    Ok(mapped)
}

fn validate_image(
    device: &BackendDeviceConfig,
    channel: &BackendChannel,
    image: ImageRange,
    layout: &ImageLayout,
    direction: Direction,
) -> std::result::Result<(), Error> {
    if image.width_bits != u64::from(channel.width_bits) {
        return Err(channel_resolve(format!(
            "channel `{}` on device `{}` is bound as {} bits but declares {} bits",
            channel.name, device.name, image.width_bits, channel.width_bits
        )));
    }
    if direction.is_input() {
        if image.area != 1 {
            return Err(channel_resolve(format!(
                "channel `{}` on device `{}` has direction `input` but image area {}",
                channel.name, device.name, image.area
            )));
        }
    } else if !matches!(image.area, 1 | 2) {
        return Err(channel_resolve(format!(
            "channel `{}` on device `{}` has direction `output` but image area {}",
            channel.name, device.name, image.area
        )));
    }
    if image.width_bits > 1 && !image.offset_bits.is_multiple_of(8) {
        return Err(channel_resolve(format!(
            "channel `{}` on device `{}` is not byte-aligned in the process image",
            channel.name, device.name
        )));
    }
    if !image_fits(layout, image) {
        return Err(channel_resolve(format!(
            "channel `{}` on device `{}` leaves the process image",
            channel.name, device.name
        )));
    }
    Ok(())
}

fn image_fits(layout: &ImageLayout, image: ImageRange) -> bool {
    layout.area(image.area).is_some_and(|(_, bytes)| {
        bytes
            .checked_mul(8)
            .and_then(|bits| {
                image
                    .offset_bits
                    .checked_add(image.width_bits)
                    .map(|end| end <= bits)
            })
            .unwrap_or(false)
    })
}

fn parse_status_device(
    device: &BackendDeviceConfig,
    data_devices: usize,
    layout: &ImageLayout,
) -> std::result::Result<Vec<StatusBinding>, Error> {
    if let Some(config) = device.config.as_deref() {
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
        let Some((server, field, width)) = status_field(&channel.name) else {
            return Err(invalid(format!(
                "status device `{}` names unknown status channel `{}`",
                device.name, channel.name
            )));
        };
        if server >= data_devices {
            return Err(invalid(format!(
                "status device `{}` channel `{}` names server{server}, but only {data_devices} \
                 Modbus data devices are declared",
                device.name, channel.name
            )));
        }
        if channel.width_bits != width {
            return Err(invalid(format!(
                "status device `{}` channel `{}` is {} bits; the status layout uses {width} bits",
                device.name, channel.name, channel.width_bits
            )));
        }
        let Some(image) = channel.image else {
            continue;
        };
        if image.area != 1 {
            return Err(invalid(format!(
                "status device `{}` channel `{}` must be bound to an `%I` range",
                device.name, channel.name
            )));
        }
        if image.width_bits != u64::from(width) {
            return Err(channel_resolve(format!(
                "status device `{}` channel `{}` is bound as {} bits but the status layout uses \
                 {width} bits",
                device.name, channel.name, image.width_bits
            )));
        }
        if image.width_bits > 1 && !image.offset_bits.is_multiple_of(8) {
            return Err(channel_resolve(format!(
                "status device `{}` channel `{}` is not byte-aligned in the process image",
                device.name, channel.name
            )));
        }
        if !image_fits(layout, image) {
            return Err(channel_resolve(format!(
                "status device `{}` channel `{}` leaves the process image",
                device.name, channel.name
            )));
        }
        bindings.push(StatusBinding {
            server,
            field,
            image,
        });
    }
    Ok(bindings)
}

fn status_field(name: &str) -> Option<(usize, StatusField, u32)> {
    let (prefix, field) = name.rsplit_once('.')?;
    let server = prefix.strip_prefix("server")?.parse::<usize>().ok()?;
    let (field, width) = match field {
        "listening" => (StatusField::Listening, 1),
        "listeners" => (StatusField::Listeners, 16),
        "bound_listeners" => (StatusField::BoundListeners, 16),
        "connections" => (StatusField::Connections, 16),
        "requests" => (StatusField::Requests, 32),
        "writes" => (StatusField::Writes, 32),
        "exceptions" => (StatusField::Exceptions, 32),
        "malformed" => (StatusField::Malformed, 32),
        "last_error" => (StatusField::LastError, 32),
        _ => return None,
    };
    Some((server, field, width))
}

fn validate_bindings(config: &BackendConfig) -> std::result::Result<(), Error> {
    let mut channels = BTreeMap::new();
    for device in &config.devices {
        for channel in &device.channels {
            channels.insert(
                (device.name.clone(), channel.name.clone()),
                (
                    channel.image,
                    channel.direction.as_str(),
                    channel.width_bits,
                ),
            );
        }
    }
    let mut seen = BTreeSet::new();
    for binding in &config.bindings {
        let key = (binding.device.clone(), binding.channel.clone());
        let Some((image, direction, width_bits)) = channels.get(&key) else {
            return Err(channel_resolve(format!(
                "binding names channel `{}` on device `{}`, which is not declared",
                binding.channel, binding.device
            )));
        };
        if *image != Some(binding.image) {
            return Err(channel_resolve(format!(
                "binding for channel `{}` on device `{}` disagrees with the channel image range",
                binding.channel, binding.device
            )));
        }
        if *direction != binding.direction {
            return Err(channel_resolve(format!(
                "binding for channel `{}` on device `{}` has direction `{}`, but the channel has \
                 `{direction}`",
                binding.channel, binding.device, binding.direction
            )));
        }
        if binding.image.width_bits != u64::from(*width_bits) {
            return Err(channel_resolve(format!(
                "binding for channel `{}` on device `{}` is {} bits but the channel is {} bits",
                binding.channel, binding.device, binding.image.width_bits, width_bits
            )));
        }
        if !seen.insert(key) {
            return Err(channel_resolve(format!(
                "channel `{}` on device `{}` is bound more than once",
                binding.channel, binding.device
            )));
        }
    }
    Ok(())
}

fn parse_duration(text: &str) -> Option<Duration> {
    let split = text.find(|ch: char| !ch.is_ascii_digit())?;
    if split == 0 {
        return None;
    }
    let number: u64 = text.get(..split)?.parse().ok()?;
    let factor = match text.get(split..)? {
        "ns" => 1_u64,
        "us" | "µs" => 1_000,
        "ms" => 1_000_000,
        "s" => 1_000_000_000,
        "m" => 60_000_000_000,
        "h" => 3_600_000_000_000,
        _ => return None,
    };
    let nanos = number.checked_mul(factor)?;
    if nanos == 0 {
        return None;
    }
    Some(Duration::from_nanos(nanos))
}

struct ListenerTask {
    server_index: usize,
    listener_index: usize,
    configured: SocketAddr,
    listener: TcpListener,
    runtime: Arc<ListenerRuntime>,
    server: Arc<ServerRuntime>,
    bank: Arc<Mutex<RegisterBank>>,
    unit: Option<u8>,
    max_connections: usize,
    idle_timeout: Duration,
    stop: Arc<AtomicBool>,
}

fn run_listener(task: ListenerTask) {
    let mut current = Some(task.listener);
    let mut children: Vec<std::thread::JoinHandle<()>> = Vec::new();
    'serve: loop {
        if task.stop.load(Ordering::SeqCst) {
            break;
        }
        #[cfg(test)]
        if task
            .runtime
            .fault_injection
            .inject
            .swap(false, Ordering::SeqCst)
        {
            task.runtime.state.store(LISTENER_FAULTED, Ordering::SeqCst);
            task.runtime.last_error.store(2, Ordering::SeqCst);
            task.server.counters.last_error.store(2, Ordering::SeqCst);
            task.runtime.fault_episodes.fetch_add(1, Ordering::SeqCst);
            note_fault(
                &task.server,
                format!(
                    "{}: listener {} accept failed: injected test fault",
                    codes::LISTENER_FAULT,
                    task.configured
                ),
            );
            current = None;
        }
        if current.is_none() {
            let mut backoff = RETRY_BACKOFF_MIN;
            while current.is_none() {
                sleep_interruptible(backoff, &task.stop);
                if task.stop.load(Ordering::SeqCst) {
                    break 'serve;
                }
                #[cfg(test)]
                {
                    while task.runtime.fault_injection.hold.load(Ordering::SeqCst)
                        && !task.stop.load(Ordering::SeqCst)
                    {
                        std::thread::sleep(ACCEPT_POLL);
                    }
                    if task.stop.load(Ordering::SeqCst) {
                        break 'serve;
                    }
                }
                match TcpListener::bind(task.configured) {
                    Ok(listener) => {
                        if let Err(error) = listener.set_nonblocking(true) {
                            task.runtime.state.store(LISTENER_FAULTED, Ordering::SeqCst);
                            task.runtime.last_error.store(3, Ordering::SeqCst);
                            task.server.counters.last_error.store(3, Ordering::SeqCst);
                            note_fault(
                                &task.server,
                                format!(
                                    "{}: listener {} cannot be configured: {error}",
                                    codes::LISTENER_FAULT,
                                    task.configured
                                ),
                            );
                            backoff = backoff.saturating_mul(2).min(RETRY_BACKOFF_MAX);
                            continue;
                        }
                        let effective = listener.local_addr().map_or_else(
                            |_| task.configured.to_string(),
                            |bound| bound.to_string(),
                        );
                        *lock_mutex(&task.runtime.effective_bind) = effective;
                        task.runtime
                            .state
                            .store(LISTENER_LISTENING, Ordering::SeqCst);
                        current = Some(listener);
                    }
                    Err(error) => {
                        task.runtime.state.store(LISTENER_FAULTED, Ordering::SeqCst);
                        task.runtime.last_error.store(1, Ordering::SeqCst);
                        task.server.counters.last_error.store(1, Ordering::SeqCst);
                        note_fault(
                            &task.server,
                            format!(
                                "{}: listener {} rebind failed: {error}",
                                codes::LISTENER_FAULT,
                                task.configured
                            ),
                        );
                        backoff = backoff.saturating_mul(2).min(RETRY_BACKOFF_MAX);
                    }
                }
            }
        }
        let Some(listener) = current.as_ref() else {
            continue;
        };
        match listener.accept() {
            Ok((stream, _peer)) => {
                children.retain(|handle| !handle.is_finished());
                if let Some(guard) = acquire_connection(&task.server, task.max_connections) {
                    task.runtime.connections.fetch_add(1, Ordering::SeqCst);
                    let guard = ConnectionGuard {
                        listener: task.runtime.clone(),
                        _slot: guard,
                    };
                    let server = task.server.clone();
                    let bank = task.bank.clone();
                    let stop = task.stop.clone();
                    let idle_timeout = task.idle_timeout;
                    let unit = task.unit;
                    let name = format!(
                        "stc-modbus-conn-{}-{}",
                        task.server_index, task.listener_index
                    );
                    let spawned = std::thread::Builder::new().name(name).spawn(move || {
                        let _guard = guard;
                        serve_connection(stream, &server, &bank, unit, idle_timeout, &stop);
                    });
                    if let Err(_error) = spawned {
                        // The closure and its connection guard were dropped.
                    }
                }
                // At the connection limit the stream is closed immediately.
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                children.retain(|handle| !handle.is_finished());
                std::thread::sleep(ACCEPT_POLL);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => {
                children.retain(|handle| !handle.is_finished());
                task.runtime.state.store(LISTENER_FAULTED, Ordering::SeqCst);
                task.runtime.last_error.store(2, Ordering::SeqCst);
                task.server.counters.last_error.store(2, Ordering::SeqCst);
                task.runtime.fault_episodes.fetch_add(1, Ordering::SeqCst);
                note_fault(
                    &task.server,
                    format!(
                        "{}: listener {} accept failed: {error}",
                        codes::LISTENER_FAULT,
                        task.configured
                    ),
                );
                current = None;
            }
        }
    }
    task.runtime.state.store(LISTENER_STOPPED, Ordering::SeqCst);
    for child in children {
        let _ = child.join();
    }
}

fn sleep_interruptible(duration: Duration, stop: &AtomicBool) {
    let deadline = Instant::now() + duration;
    while !stop.load(Ordering::SeqCst) {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        std::thread::sleep((deadline - now).min(ACCEPT_POLL));
    }
}

fn note_fault(server: &ServerRuntime, message: String) {
    *lock_mutex(&server.fault) = Some(message);
}

struct ConnectionGuard {
    listener: Arc<ListenerRuntime>,
    _slot: SlotGuard,
}

struct SlotGuard(Arc<ServerRuntime>);

impl Drop for SlotGuard {
    fn drop(&mut self) {
        self.0.counters.connections.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.listener.connections.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Reserves one connection slot for the server; `None` at the limit.
fn acquire_connection(server: &Arc<ServerRuntime>, max: usize) -> Option<SlotGuard> {
    let counters = &server.counters;
    loop {
        let current = counters.connections.load(Ordering::SeqCst);
        if current >= max {
            return None;
        }
        if counters
            .connections
            .compare_exchange(current, current + 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            counters
                .peak_connections
                .fetch_max(current + 1, Ordering::SeqCst);
            return Some(SlotGuard(server.clone()));
        }
    }
}

fn serve_connection(
    mut stream: TcpStream,
    server: &ServerRuntime,
    bank: &Mutex<RegisterBank>,
    unit: Option<u8>,
    idle_timeout: Duration,
    stop: &AtomicBool,
) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_nodelay(true);
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let mut decoder = FrameDecoder::new();
    let mut buffer = [0_u8; 1024];
    let mut last_request = Instant::now();
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let idle = last_request.elapsed();
        if idle >= idle_timeout {
            finish_connection(&mut decoder, server);
            return;
        }
        let timeout = (idle_timeout - idle).min(CONNECTION_POLL);
        if stream.set_read_timeout(Some(timeout)).is_err() {
            return;
        }
        match stream.read(&mut buffer) {
            Ok(0) => {
                finish_connection(&mut decoder, server);
                return;
            }
            Ok(count) => {
                decoder.push(&buffer[..count]);
                loop {
                    match decoder.next_frame() {
                        Ok(Some(frame)) => {
                            last_request = Instant::now();
                            server.counters.requests.fetch_add(1, Ordering::SeqCst);
                            let response = {
                                let mut bank = lock_mutex(bank);
                                handle_request(&mut bank, unit, frame.unit_id, &frame.pdu)
                            };
                            count_response(&server.counters, &frame.pdu, &response);
                            let Ok(adu) =
                                encode_adu(frame.transaction_id, frame.unit_id, &response)
                            else {
                                server.counters.last_error.store(3, Ordering::SeqCst);
                                return;
                            };
                            if stream.write_all(&adu).is_err() {
                                return;
                            }
                        }
                        Ok(None) => break,
                        Err(_) => {
                            server.counters.malformed.fetch_add(1, Ordering::SeqCst);
                            return;
                        }
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => {
                finish_connection(&mut decoder, server);
                return;
            }
        }
    }
}

fn finish_connection(decoder: &mut FrameDecoder, server: &ServerRuntime) {
    if decoder.finish().is_err() {
        server.counters.malformed.fetch_add(1, Ordering::SeqCst);
    }
}

fn count_response(counters: &ServerCounters, request: &[u8], response: &[u8]) {
    let function = request.first().copied().unwrap_or(0);
    if response.first().is_some_and(|first| first & 0x80 != 0) {
        counters.exceptions.fetch_add(1, Ordering::SeqCst);
        return;
    }
    match function {
        0x01..=0x04 => {
            counters.reads.fetch_add(1, Ordering::SeqCst);
        }
        0x05 | 0x06 | 0x0F | 0x10 => {
            counters.writes.fetch_add(1, Ordering::SeqCst);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration(name: &str, direction: Direction, width_bits: u32) -> ChannelDeclaration {
        ChannelDeclaration::new(name, direction, width_bits)
    }

    fn entry(name: &str, table: Table, address: u16) -> MappedChannel {
        MappedChannel::new(name, table, address)
    }

    fn mapped(
        name: &str,
        table: Table,
        address: u16,
        byte: Option<BytePlacement>,
        word_order: Option<WordOrder>,
    ) -> MappedChannel {
        MappedChannel {
            name: name.to_owned(),
            table,
            address,
            byte,
            word_order,
        }
    }

    fn build(
        declared: &[ChannelDeclaration],
        entries: &[MappedChannel],
        default: DeviceWordOrder,
    ) -> RegisterBank {
        RegisterBank::new(
            ChannelMap::build(declared, entries, default)
                .unwrap_or_else(|error| panic!("channel map rejected: {error}")),
        )
    }

    fn high_first_bank(declared: &[ChannelDeclaration], entries: &[MappedChannel]) -> RegisterBank {
        build(declared, entries, DeviceWordOrder::HighFirst)
    }

    fn map_error(
        declared: &[ChannelDeclaration],
        entries: &[MappedChannel],
        default: DeviceWordOrder,
    ) -> ConfigError {
        match ChannelMap::build(declared, entries, default) {
            Ok(_) => panic!("channel map unexpectedly accepted"),
            Err(error) => {
                assert_eq!(error.code, codes::INVALID_CONFIG);
                error
            }
        }
    }

    fn single(
        name: &str,
        direction: Direction,
        width_bits: u32,
        table: Table,
        address: u16,
        byte: Option<BytePlacement>,
        word_order: Option<WordOrder>,
    ) -> RegisterBank {
        build(
            &[declaration(name, direction, width_bits)],
            &[mapped(name, table, address, byte, word_order)],
            DeviceWordOrder::HighFirst,
        )
    }

    fn simple(
        name: &str,
        direction: Direction,
        width_bits: u32,
        table: Table,
        address: u16,
    ) -> RegisterBank {
        single(name, direction, width_bits, table, address, None, None)
    }

    fn placed(
        name: &str,
        direction: Direction,
        width_bits: u32,
        table: Table,
        address: u16,
        byte: BytePlacement,
    ) -> RegisterBank {
        single(
            name,
            direction,
            width_bits,
            table,
            address,
            Some(byte),
            None,
        )
    }

    fn ordered(
        name: &str,
        direction: Direction,
        width_bits: u32,
        table: Table,
        address: u16,
        order: WordOrder,
    ) -> RegisterBank {
        single(
            name,
            direction,
            width_bits,
            table,
            address,
            None,
            Some(order),
        )
    }

    fn read_pdu(function: u8, start: u16, quantity: u16) -> Vec<u8> {
        vec![
            function,
            (start >> 8) as u8,
            start as u8,
            (quantity >> 8) as u8,
            quantity as u8,
        ]
    }

    fn write_single_pdu(function: u8, address: u16, value: u16) -> Vec<u8> {
        vec![
            function,
            (address >> 8) as u8,
            address as u8,
            (value >> 8) as u8,
            value as u8,
        ]
    }

    fn write_coils_pdu(start: u16, values: &[bool]) -> Vec<u8> {
        let byte_count = values.len().div_ceil(8);
        let mut data = vec![0_u8; byte_count];
        for (index, &value) in values.iter().enumerate() {
            if value {
                data[index / 8] |= 1 << (index % 8);
            }
        }
        let quantity = u16::try_from(values.len()).expect("test quantity");
        let mut pdu = vec![
            0x0F,
            (start >> 8) as u8,
            start as u8,
            (quantity >> 8) as u8,
            quantity as u8,
            u8::try_from(byte_count).expect("test byte count"),
        ];
        pdu.extend_from_slice(&data);
        pdu
    }

    fn write_registers_pdu(start: u16, values: &[u16]) -> Vec<u8> {
        let quantity = u16::try_from(values.len()).expect("test quantity");
        let mut pdu = vec![
            0x10,
            (start >> 8) as u8,
            start as u8,
            (quantity >> 8) as u8,
            quantity as u8,
            u8::try_from(values.len() * 2).expect("test byte count"),
        ];
        for &value in values {
            pdu.extend_from_slice(&value.to_be_bytes());
        }
        pdu
    }

    fn handle(bank: &mut RegisterBank, pdu: &[u8]) -> Vec<u8> {
        handle_request(bank, None, 1, pdu)
    }

    fn assert_exception(response: &[u8], function: u8, code: ExceptionCode) {
        assert_eq!(
            response,
            exception_pdu(function, code),
            "response {response:02X?}"
        );
    }

    fn response_words(response: &[u8]) -> Vec<u16> {
        assert_eq!(
            usize::from(response[1]) + 2,
            response.len(),
            "response {response:02X?}"
        );
        response[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_be_bytes(*pair))
            .collect()
    }

    fn response_bits(response: &[u8], quantity: usize) -> Vec<bool> {
        assert_eq!(usize::from(response[1]), quantity.div_ceil(8));
        (0..quantity)
            .map(|index| (response[2 + index / 8] >> (index % 8)) & 1 != 0)
            .collect()
    }

    fn vendor_reference(table: Table, address: u16) -> u32 {
        table.vendor_base() + u32::from(address)
    }

    fn read_words_for_order(width: u32, order: WordOrder, value: u64) -> Vec<u16> {
        let mut bank = ordered(
            "value",
            Direction::Output,
            width,
            Table::HoldingRegister,
            0,
            order,
        );
        assert!(bank.write_channel("value", value));
        let span = u16::try_from(width.div_ceil(16)).expect("test span");
        response_words(&handle(&mut bank, &read_pdu(0x03, 0, span)))
    }

    fn read_words_for_default(width: u32, default: DeviceWordOrder, value: u64) -> Vec<u16> {
        let mut bank = build(
            &[declaration("value", Direction::Output, width)],
            &[entry("value", Table::HoldingRegister, 0)],
            default,
        );
        assert!(bank.write_channel("value", value));
        let span = u16::try_from(width.div_ceil(16)).expect("test span");
        response_words(&handle(&mut bank, &read_pdu(0x03, 0, span)))
    }

    // ---------------------------------------------------------------- codec

    #[test]
    fn mbap_round_trip_echoes_transaction_and_unit() {
        let pdu = vec![0x03, 0x00, 0x00, 0x00, 0x01];
        let adu = encode_adu(0x1234, 7, &pdu).expect("encode");
        assert_eq!(
            adu,
            vec![
                0x12, 0x34, 0x00, 0x00, 0x00, 0x06, 0x07, 0x03, 0x00, 0x00, 0x00, 0x01
            ]
        );
        let mut decoder = FrameDecoder::new();
        let frames = decoder.decode(&adu).expect("decode");
        assert_eq!(
            frames,
            vec![Frame {
                transaction_id: 0x1234,
                unit_id: 7,
                pdu,
            }]
        );
        decoder.finish().expect("complete frame");
    }

    #[test]
    fn mbap_rejects_nonzero_protocol_id() {
        let mut decoder = FrameDecoder::new();
        decoder.push(&[0x00, 0x01, 0x00, 0x01, 0x00, 0x02, 0x01, 0x03]);
        assert_eq!(decoder.next_frame(), Err(FramingError::ProtocolId(1)));
        assert_eq!(decoder.buffered(), 0);
    }

    #[test]
    fn mbap_length_bounds_and_maximum_adu() {
        for length in [0_u16, 1, 255, 0xFFFF] {
            let mut decoder = FrameDecoder::new();
            let mut header = vec![0x00, 0x01, 0x00, 0x00];
            header.extend_from_slice(&length.to_be_bytes());
            header.push(0x01);
            decoder.push(&header);
            assert_eq!(decoder.next_frame(), Err(FramingError::Length(length)));
        }

        // length 254 is unit id plus a 253-byte PDU: the 260-byte maximum.
        let pdu = vec![0x03; MAX_PDU];
        let adu = encode_adu(1, 2, &pdu).expect("maximum PDU");
        assert_eq!(adu.len(), MAX_ADU);
        let mut decoder = FrameDecoder::new();
        let frames = decoder.decode(&adu).expect("decode maximum");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].pdu.len(), MAX_PDU);
        decoder.finish().expect("complete frame");

        // A 254-byte PDU would need a 261-byte ADU.
        assert_eq!(
            encode_adu(1, 2, &vec![0; MAX_PDU + 1]),
            Err(EncodeError::PduTooLong(MAX_PDU + 1))
        );
    }

    #[test]
    fn decoder_pipelines_frames_in_order() {
        let first = encode_adu(1, 1, &read_pdu(0x03, 0, 1)).expect("first");
        let second = encode_adu(2, 1, &read_pdu(0x04, 0, 1)).expect("second");
        let mut combined = first;
        combined.extend_from_slice(&second);

        let mut decoder = FrameDecoder::new();
        let frames = decoder.decode(&combined).expect("frames");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].transaction_id, 1);
        assert_eq!(frames[1].transaction_id, 2);
        assert_eq!(decoder.buffered(), 0);

        // Byte-at-a-time delivery yields the same frames in the same order.
        let mut decoder = FrameDecoder::new();
        let mut seen = Vec::new();
        for byte in combined {
            decoder.push(&[byte]);
            while let Some(frame) = decoder.next_frame().expect("frame") {
                seen.push(frame.transaction_id);
            }
        }
        assert_eq!(seen, vec![1, 2]);
    }

    #[test]
    fn truncated_frames_do_not_panic_and_finish_reports_them() {
        let adu = encode_adu(0x0102, 3, &read_pdu(0x03, 0, 1)).expect("adu");
        for length in 0..adu.len() {
            let mut decoder = FrameDecoder::new();
            decoder.push(&adu[..length]);
            assert_eq!(decoder.next_frame(), Ok(None), "prefix {length}");
            assert_eq!(decoder.buffered(), length);
            let finished = decoder.finish();
            if length == 0 {
                assert_eq!(finished, Ok(()));
            } else {
                assert_eq!(finished, Err(FramingError::Truncated));
            }
        }

        let complete = encode_adu(1, 1, &read_pdu(0x03, 0, 1)).expect("complete");
        let second = encode_adu(2, 1, &read_pdu(0x03, 0, 1)).expect("second");
        let mut data = complete;
        data.extend_from_slice(&second[..3]);
        let mut decoder = FrameDecoder::new();
        let frames = decoder.decode(&data).expect("first frame");
        assert_eq!(frames.len(), 1);
        assert_eq!(decoder.finish(), Err(FramingError::Truncated));
    }

    #[test]
    fn arbitrary_byte_streams_do_not_panic() {
        let mut state = 0x1234_5678_9ABC_DEF0_u64;
        let mut decoder = FrameDecoder::new();
        for _ in 0..500 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let length = usize::try_from(state >> 60).expect("small length");
            let mut chunk = vec![0_u8; length];
            for byte in &mut chunk {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                *byte = (state >> 32) as u8;
            }
            if decoder.decode(&chunk).is_err() {
                decoder = FrameDecoder::new();
            }
            assert!(decoder.buffered() < MAX_ADU);
            if decoder.buffered() >= MAX_ADU {
                decoder = FrameDecoder::new();
            }
        }
    }

    #[test]
    fn random_pdus_do_not_panic() {
        let declared = [
            declaration("coil", Direction::Input, 1),
            declaration("holding", Direction::Input, 16),
            declaration("dword", Direction::Input, 32),
            declaration("out", Direction::Output, 16),
        ];
        let entries = [
            entry("coil", Table::Coil, 0),
            entry("holding", Table::HoldingRegister, 0),
            entry("dword", Table::HoldingRegister, 2),
            entry("out", Table::HoldingRegister, 10),
        ];
        let mut bank = high_first_bank(&declared, &entries);
        let mut state = 0xDEAD_BEEF_0123_4567_u64;
        for _ in 0..2000 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let length = usize::try_from((state >> 56) % 40).expect("small length");
            let mut pdu = vec![0_u8; length];
            for byte in &mut pdu {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                *byte = (state >> 40) as u8;
            }
            let _ = handle(&mut bank, &pdu);
        }
    }

    #[test]
    fn encode_rejects_empty_and_oversized_pdus() {
        assert_eq!(encode_adu(0, 0, &[]), Err(EncodeError::EmptyPdu));
        assert_eq!(
            encode_adu(0, 0, &[0; MAX_PDU + 1]),
            Err(EncodeError::PduTooLong(MAX_PDU + 1))
        );
    }

    // ------------------------------------------------- mapping and addresses

    #[test]
    fn table_spellings_vendor_bases_and_function_codes() {
        for (text, table, base, function) in [
            ("coil", Table::Coil, 1, 0x01),
            ("discrete-input", Table::DiscreteInput, 10_001, 0x02),
            ("holding-register", Table::HoldingRegister, 40_001, 0x03),
            ("input-register", Table::InputRegister, 30_001, 0x04),
        ] {
            assert_eq!(Table::parse(text), Some(table));
            assert_eq!(table.as_str(), text);
            assert_eq!(table.vendor_base(), base);
            assert_eq!(table.read_function(), function);
        }
        assert_eq!(Table::parse("register"), None);
        assert_eq!(Direction::parse("input"), Some(Direction::Input));
        assert_eq!(Direction::parse("output"), Some(Direction::Output));
        assert_eq!(Direction::parse("inout"), None);
        assert_eq!(BytePlacement::parse("low"), Some(BytePlacement::Low));
        assert_eq!(BytePlacement::parse("high"), Some(BytePlacement::High));
        assert_eq!(
            DeviceWordOrder::parse("high-first"),
            Some(DeviceWordOrder::HighFirst)
        );
        assert_eq!(
            DeviceWordOrder::parse("low-first"),
            Some(DeviceWordOrder::LowFirst)
        );
        for name in ["high-first", "low-first", "abcd", "cdab", "dcba", "badc"] {
            assert!(WordOrder::parse(name).is_some(), "{name}");
        }
        assert_eq!(WordOrder::parse("ABCD"), None);
    }

    #[test]
    fn zero_based_addresses_match_vendor_reference_numbers() {
        for (table, address, reference) in [
            (Table::Coil, 0, 1),
            (Table::Coil, 1, 2),
            (Table::Coil, 1024, 1025),
            (Table::DiscreteInput, 0, 10_001),
            (Table::DiscreteInput, 1, 10_002),
            (Table::DiscreteInput, 1024, 11_025),
            (Table::InputRegister, 0, 30_001),
            (Table::InputRegister, 1, 30_002),
            (Table::InputRegister, 1024, 31_025),
            (Table::HoldingRegister, 0, 40_001),
            (Table::HoldingRegister, 1, 40_002),
            (Table::HoldingRegister, 1024, 41_025),
        ] {
            assert_eq!(
                vendor_reference(table, address),
                reference,
                "{} address {address}",
                table.as_str()
            );
        }
    }

    #[test]
    fn protocol_addresses_are_zero_based_for_every_table() {
        for address in [0_u16, 1, 1024] {
            let problem = format!("address {address}");

            // Coil: FC5 writes and FC1 reads the zero-based PDU address.
            let mut bank = simple("coil", Direction::Input, 1, Table::Coil, address);
            let request = write_single_pdu(0x05, address, 0xFF00);
            assert_eq!(handle(&mut bank, &request), request, "{problem}");
            assert_eq!(
                handle(&mut bank, &read_pdu(0x01, address, 1)),
                vec![0x01, 0x01, 0x01],
                "{problem}"
            );
            if address > 0 {
                assert_exception(
                    &handle(&mut bank, &read_pdu(0x01, address - 1, 1)),
                    0x01,
                    ExceptionCode::IllegalDataAddress,
                );
            }

            // Discrete input: published by the PLC, read with FC2.
            let mut bank = simple("di", Direction::Output, 1, Table::DiscreteInput, address);
            assert!(bank.write_channel("di", 1));
            assert_eq!(
                handle(&mut bank, &read_pdu(0x02, address, 1)),
                vec![0x02, 0x01, 0x01],
                "{problem}"
            );
            if address > 0 {
                assert_exception(
                    &handle(&mut bank, &read_pdu(0x02, address - 1, 1)),
                    0x02,
                    ExceptionCode::IllegalDataAddress,
                );
            }

            // Holding register: FC6 writes and FC3 reads.
            let mut bank = simple(
                "holding",
                Direction::Input,
                16,
                Table::HoldingRegister,
                address,
            );
            let request = write_single_pdu(0x06, address, 0x1234);
            assert_eq!(handle(&mut bank, &request), request, "{problem}");
            assert_eq!(
                response_words(&handle(&mut bank, &read_pdu(0x03, address, 1))),
                vec![0x1234],
                "{problem}"
            );
            if address > 0 {
                assert_exception(
                    &handle(&mut bank, &read_pdu(0x03, address - 1, 1)),
                    0x03,
                    ExceptionCode::IllegalDataAddress,
                );
            }

            // Input register: published by the PLC, read with FC4.
            let mut bank = simple("ir", Direction::Output, 16, Table::InputRegister, address);
            assert!(bank.write_channel("ir", 0xBEEF));
            assert_eq!(
                response_words(&handle(&mut bank, &read_pdu(0x04, address, 1))),
                vec![0xBEEF],
                "{problem}"
            );
            if address > 0 {
                assert_exception(
                    &handle(&mut bank, &read_pdu(0x04, address - 1, 1)),
                    0x04,
                    ExceptionCode::IllegalDataAddress,
                );
            }
        }
    }

    #[test]
    fn the_first_register_is_address_zero_not_one() {
        let mut bank = simple("h", Direction::Input, 16, Table::HoldingRegister, 1);
        let request = write_single_pdu(0x06, 1, 0x1234);
        assert_eq!(handle(&mut bank, &request), request);
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 1, 1))),
            vec![0x1234]
        );
        assert_exception(
            &handle(&mut bank, &read_pdu(0x03, 0, 1)),
            0x03,
            ExceptionCode::IllegalDataAddress,
        );
        assert_exception(
            &handle(&mut bank, &write_single_pdu(0x06, 0, 1)),
            0x06,
            ExceptionCode::IllegalDataAddress,
        );
    }

    #[test]
    fn wide_channels_span_consecutive_zero_based_registers() {
        let mut bank = ordered(
            "dword",
            Direction::Output,
            32,
            Table::HoldingRegister,
            20,
            WordOrder::HighFirst,
        );
        assert!(bank.write_channel("dword", 0xAABB_CCDD));
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 20, 2))),
            vec![0xAABB, 0xCCDD]
        );
        assert_exception(
            &handle(&mut bank, &read_pdu(0x03, 19, 1)),
            0x03,
            ExceptionCode::IllegalDataAddress,
        );
        assert_eq!(vendor_reference(Table::HoldingRegister, 20), 40_021);
        assert_eq!(vendor_reference(Table::HoldingRegister, 21), 40_022);

        let mut bank = ordered(
            "lword",
            Direction::Output,
            64,
            Table::InputRegister,
            30,
            WordOrder::HighFirst,
        );
        assert!(bank.write_channel("lword", 0x0102_0304_0506_0708));
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x04, 30, 4))),
            vec![0x0102, 0x0304, 0x0506, 0x0708]
        );
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x04, 31, 2))),
            vec![0x0304, 0x0506]
        );
        assert_exception(
            &handle(&mut bank, &read_pdu(0x04, 29, 4)),
            0x04,
            ExceptionCode::IllegalDataAddress,
        );
    }

    // ------------------------------------------------------- function codes

    #[test]
    fn fc1_and_fc2_pack_bits_lsb_first() {
        let values = [true, false, true, true, false, false, false, false, true];
        let declarations: Vec<_> = (0..values.len())
            .map(|index| declaration(&format!("c{index}"), Direction::Input, 1))
            .collect();
        let entries: Vec<_> = (0..values.len())
            .map(|index| entry(&format!("c{index}"), Table::Coil, index as u16))
            .collect();
        let mut bank = high_first_bank(&declarations, &entries);
        for (index, &value) in values.iter().enumerate() {
            assert!(bank.write_channel(&format!("c{index}"), u64::from(value)));
        }
        let response = handle(&mut bank, &read_pdu(0x01, 0, 9));
        assert_eq!(response, vec![0x01, 0x02, 0x0D, 0x01]);
        assert_eq!(response_bits(&response, 9), values.to_vec());

        let declarations: Vec<_> = (0..values.len())
            .map(|index| declaration(&format!("d{index}"), Direction::Output, 1))
            .collect();
        let entries: Vec<_> = (0..values.len())
            .map(|index| entry(&format!("d{index}"), Table::DiscreteInput, index as u16))
            .collect();
        let mut bank = high_first_bank(&declarations, &entries);
        for (index, &value) in values.iter().enumerate() {
            assert!(bank.write_channel(&format!("d{index}"), u64::from(value)));
        }
        let response = handle(&mut bank, &read_pdu(0x02, 0, 9));
        assert_eq!(response, vec![0x02, 0x02, 0x0D, 0x01]);
        assert_eq!(response_bits(&response, 9), values.to_vec());

        // Three bits occupy one response byte with zero padding.
        let response = handle(&mut bank, &read_pdu(0x02, 0, 3));
        assert_eq!(response, vec![0x02, 0x01, 0x05]);
    }

    #[test]
    fn fc3_and_fc4_read_registers_big_endian() {
        let declarations = [
            declaration("h0", Direction::Input, 16),
            declaration("h1", Direction::Input, 16),
        ];
        let entries = [
            entry("h0", Table::HoldingRegister, 0),
            entry("h1", Table::HoldingRegister, 1),
        ];
        let mut bank = high_first_bank(&declarations, &entries);
        assert!(bank.write_channel("h0", 0x1234));
        assert!(bank.write_channel("h1", 0xABCD));
        assert_eq!(
            handle(&mut bank, &read_pdu(0x03, 0, 2)),
            vec![0x03, 0x04, 0x12, 0x34, 0xAB, 0xCD]
        );

        let declarations = [
            declaration("ir0", Direction::Output, 16),
            declaration("ir1", Direction::Output, 16),
        ];
        let entries = [
            entry("ir0", Table::InputRegister, 0),
            entry("ir1", Table::InputRegister, 1),
        ];
        let mut bank = high_first_bank(&declarations, &entries);
        assert!(bank.write_channel("ir0", 0x0001));
        assert!(bank.write_channel("ir1", 0x0100));
        assert_eq!(
            handle(&mut bank, &read_pdu(0x04, 0, 2)),
            vec![0x04, 0x04, 0x00, 0x01, 0x01, 0x00]
        );
    }

    #[test]
    fn quantity_limits_are_enforced() {
        // FC1/FC2: 1..=2000 bits.
        let declarations: Vec<_> = (0..2000_u16)
            .map(|index| declaration(&format!("c{index}"), Direction::Input, 1))
            .collect();
        let entries: Vec<_> = (0..2000_u16)
            .map(|index| entry(&format!("c{index}"), Table::Coil, index))
            .collect();
        let mut bank = high_first_bank(&declarations, &entries);
        let response = handle(&mut bank, &read_pdu(0x01, 0, 2000));
        assert_eq!(response[0], 0x01);
        assert_eq!(usize::from(response[1]), 250);
        assert_eq!(response.len(), 252);
        assert_exception(
            &handle(&mut bank, &read_pdu(0x01, 0, 2001)),
            0x01,
            ExceptionCode::IllegalDataValue,
        );
        assert_exception(
            &handle(&mut bank, &read_pdu(0x01, 0, 0)),
            0x01,
            ExceptionCode::IllegalDataValue,
        );
        assert_exception(
            &handle(&mut bank, &read_pdu(0x01, 65535, 2)),
            0x01,
            ExceptionCode::IllegalDataAddress,
        );

        // FC3/FC4: 1..=125 registers.
        let declarations: Vec<_> = (0..125_u16)
            .map(|index| declaration(&format!("h{index}"), Direction::Input, 16))
            .collect();
        let entries: Vec<_> = (0..125_u16)
            .map(|index| entry(&format!("h{index}"), Table::HoldingRegister, index))
            .collect();
        let mut bank = high_first_bank(&declarations, &entries);
        let response = handle(&mut bank, &read_pdu(0x03, 0, 125));
        assert_eq!(response[0], 0x03);
        assert_eq!(usize::from(response[1]), 250);
        assert_eq!(response.len(), 252);
        assert_exception(
            &handle(&mut bank, &read_pdu(0x03, 0, 126)),
            0x03,
            ExceptionCode::IllegalDataValue,
        );
        assert_exception(
            &handle(&mut bank, &read_pdu(0x03, 0, 0)),
            0x03,
            ExceptionCode::IllegalDataValue,
        );

        // FC15: 1..=1968 bits.
        let declarations: Vec<_> = (0..1968_u16)
            .map(|index| declaration(&format!("w{index}"), Direction::Input, 1))
            .collect();
        let entries: Vec<_> = (0..1968_u16)
            .map(|index| entry(&format!("w{index}"), Table::Coil, index))
            .collect();
        let mut bank = high_first_bank(&declarations, &entries);
        let request = write_coils_pdu(0, &vec![true; 1968]);
        assert_eq!(
            handle(&mut bank, &request),
            vec![0x0F, 0x00, 0x00, 0x07, 0xB0]
        );
        assert_exception(
            &handle(&mut bank, &write_coils_pdu(0, &vec![true; 1969])),
            0x0F,
            ExceptionCode::IllegalDataValue,
        );
        assert_exception(
            &handle(&mut bank, &[0x0F, 0, 0, 0, 0, 0]),
            0x0F,
            ExceptionCode::IllegalDataValue,
        );

        // FC16: 1..=123 registers.
        let declarations: Vec<_> = (0..123_u16)
            .map(|index| declaration(&format!("r{index}"), Direction::Input, 16))
            .collect();
        let entries: Vec<_> = (0..123_u16)
            .map(|index| entry(&format!("r{index}"), Table::HoldingRegister, index))
            .collect();
        let mut bank = high_first_bank(&declarations, &entries);
        let request = write_registers_pdu(0, &[0x0001; 123]);
        assert_eq!(
            handle(&mut bank, &request),
            vec![0x10, 0x00, 0x00, 0x00, 0x7B]
        );
        assert_exception(
            &handle(&mut bank, &write_registers_pdu(0, &[0x0001; 124])),
            0x10,
            ExceptionCode::IllegalDataValue,
        );
        assert_exception(
            &handle(&mut bank, &write_registers_pdu(0, &[])),
            0x10,
            ExceptionCode::IllegalDataValue,
        );
    }

    #[test]
    fn fc5_and_fc6_rules() {
        let mut bank = simple("c", Direction::Input, 1, Table::Coil, 3);
        let request = write_single_pdu(0x05, 3, 0xFF00);
        assert_eq!(handle(&mut bank, &request), request);
        assert_eq!(bank.read_channel("c"), Some(1));
        let request = write_single_pdu(0x05, 3, 0x0000);
        assert_eq!(handle(&mut bank, &request), request);
        assert_eq!(bank.read_channel("c"), Some(0));
        assert_exception(
            &handle(&mut bank, &write_single_pdu(0x05, 3, 0x1234)),
            0x05,
            ExceptionCode::IllegalDataValue,
        );
        assert_exception(
            &handle(&mut bank, &write_single_pdu(0x05, 4, 0xFF00)),
            0x05,
            ExceptionCode::IllegalDataAddress,
        );

        let mut bank = simple("h", Direction::Input, 16, Table::HoldingRegister, 5);
        let request = write_single_pdu(0x06, 5, 0xBEEF);
        assert_eq!(handle(&mut bank, &request), request);
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 5, 1))),
            vec![0xBEEF]
        );
        assert_exception(
            &handle(&mut bank, &write_single_pdu(0x06, 6, 1)),
            0x06,
            ExceptionCode::IllegalDataAddress,
        );
        assert_exception(
            &handle(&mut bank, &[0x06, 0, 0]),
            0x06,
            ExceptionCode::IllegalDataValue,
        );
    }

    #[test]
    fn writes_to_output_channels_are_rejected() {
        let mut bank = simple("q", Direction::Output, 1, Table::Coil, 0);
        assert!(bank.write_channel("q", 1));
        assert_exception(
            &handle(&mut bank, &write_single_pdu(0x05, 0, 0x0000)),
            0x05,
            ExceptionCode::IllegalDataAddress,
        );
        assert_exception(
            &handle(&mut bank, &write_coils_pdu(0, &[false])),
            0x0F,
            ExceptionCode::IllegalDataAddress,
        );
        assert_eq!(bank.read_channel("q"), Some(1));

        let mut bank = simple("h", Direction::Output, 16, Table::HoldingRegister, 0);
        assert!(bank.write_channel("h", 0x1234));
        assert_exception(
            &handle(&mut bank, &write_single_pdu(0x06, 0, 0x0000)),
            0x06,
            ExceptionCode::IllegalDataAddress,
        );
        assert_exception(
            &handle(&mut bank, &write_registers_pdu(0, &[0x0000])),
            0x10,
            ExceptionCode::IllegalDataAddress,
        );
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 0, 1))),
            vec![0x1234]
        );
    }

    #[test]
    fn fc15_and_fc16_apply_writes_and_echo_ranges() {
        let declarations = [
            declaration("a", Direction::Input, 1),
            declaration("b", Direction::Input, 1),
            declaration("c", Direction::Input, 1),
        ];
        let entries = [
            entry("a", Table::Coil, 0),
            entry("b", Table::Coil, 1),
            entry("c", Table::Coil, 2),
        ];
        let mut bank = high_first_bank(&declarations, &entries);
        let request = write_coils_pdu(0, &[true, false, true]);
        assert_eq!(
            handle(&mut bank, &request),
            vec![0x0F, 0x00, 0x00, 0x00, 0x03]
        );
        assert_eq!(
            response_bits(&handle(&mut bank, &read_pdu(0x01, 0, 3)), 3),
            vec![true, false, true]
        );

        let declarations = [
            declaration("a", Direction::Input, 16),
            declaration("b", Direction::Input, 16),
        ];
        let entries = [
            entry("a", Table::HoldingRegister, 4),
            entry("b", Table::HoldingRegister, 5),
        ];
        let mut bank = high_first_bank(&declarations, &entries);
        let request = write_registers_pdu(4, &[0x1111, 0x2222]);
        assert_eq!(
            handle(&mut bank, &request),
            vec![0x10, 0x00, 0x04, 0x00, 0x02]
        );
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 4, 2))),
            vec![0x1111, 0x2222]
        );
    }

    #[test]
    fn fc15_byte_count_and_length_are_validated() {
        let declarations: Vec<_> = (0..10_u16)
            .map(|index| declaration(&format!("c{index}"), Direction::Input, 1))
            .collect();
        let entries: Vec<_> = (0..10_u16)
            .map(|index| entry(&format!("c{index}"), Table::Coil, index))
            .collect();
        let mut bank = high_first_bank(&declarations, &entries);

        // Byte count says two bytes but only one follows.
        assert_exception(
            &handle(&mut bank, &[0x0F, 0, 0, 0, 10, 2, 0xFF]),
            0x0F,
            ExceptionCode::IllegalDataValue,
        );
        // Byte count says one byte but two follow.
        assert_exception(
            &handle(&mut bank, &[0x0F, 0, 0, 0, 10, 1, 0xFF, 0x00]),
            0x0F,
            ExceptionCode::IllegalDataValue,
        );
        // Byte count disagrees with quantity.
        assert_exception(
            &handle(&mut bank, &[0x0F, 0, 0, 0, 10, 1, 0xFF]),
            0x0F,
            ExceptionCode::IllegalDataValue,
        );
        // Quantity 0 is invalid.
        assert_exception(
            &handle(&mut bank, &[0x0F, 0, 0, 0, 0, 0]),
            0x0F,
            ExceptionCode::IllegalDataValue,
        );
    }

    #[test]
    fn fc16_byte_count_and_length_are_validated() {
        let declarations = [declaration("h", Direction::Input, 16)];
        let entries = [entry("h", Table::HoldingRegister, 0)];
        let mut bank = high_first_bank(&declarations, &entries);

        assert_exception(
            &handle(&mut bank, &[0x10, 0, 0, 0, 1, 4, 0x00, 0x01]),
            0x10,
            ExceptionCode::IllegalDataValue,
        );
        assert_exception(
            &handle(&mut bank, &[0x10, 0, 0, 0, 1, 2, 0x00]),
            0x10,
            ExceptionCode::IllegalDataValue,
        );
        assert_exception(
            &handle(&mut bank, &[0x10, 0, 0, 0, 1, 2, 0x00, 0x01, 0x02]),
            0x10,
            ExceptionCode::IllegalDataValue,
        );
        assert_exception(
            &handle(&mut bank, &[0x10, 0, 0, 0, 0, 0]),
            0x10,
            ExceptionCode::IllegalDataValue,
        );
    }

    #[test]
    fn malformed_supported_function_pdus_return_0x03() {
        let mut bank = simple("c", Direction::Input, 1, Table::Coil, 0);
        for function in [0x01_u8, 0x02, 0x03, 0x04, 0x05, 0x06] {
            for length in [1_usize, 2, 3, 4, 6] {
                let pdu = vec![function; length];
                assert_exception(
                    &handle(&mut bank, &pdu),
                    function,
                    ExceptionCode::IllegalDataValue,
                );
            }
        }
        for pdu in [
            vec![0x0F],
            vec![0x0F, 0, 0, 0, 1],
            vec![0x10],
            vec![0x10, 0, 0, 0, 1],
        ] {
            let function = pdu[0];
            assert_exception(
                &handle(&mut bank, &pdu),
                function,
                ExceptionCode::IllegalDataValue,
            );
        }
    }

    #[test]
    fn unsupported_functions_return_0x01() {
        let mut bank = simple("c", Direction::Input, 1, Table::Coil, 0);
        for function in [0x00_u8, 0x07, 0x08, 0x0B, 0x16, 0x17, 0x2B, 0x80, 0xFF] {
            let response = handle(&mut bank, &[function, 0, 0, 0, 1]);
            assert_exception(&response, function, ExceptionCode::IllegalFunction);
        }
        assert_exception(
            &handle(&mut bank, &[]),
            0x00,
            ExceptionCode::IllegalFunction,
        );
    }

    #[test]
    fn unit_id_policy() {
        let mut bank = simple("c", Direction::Input, 1, Table::Coil, 0);
        assert_eq!(
            handle_request(&mut bank, None, 0, &read_pdu(0x01, 0, 1)),
            vec![0x01, 0x01, 0x00]
        );
        assert_eq!(
            handle_request(&mut bank, None, 255, &read_pdu(0x01, 0, 1)),
            vec![0x01, 0x01, 0x00]
        );
        assert_exception(
            &handle_request(&mut bank, Some(7), 2, &read_pdu(0x01, 0, 1)),
            0x01,
            ExceptionCode::WrongUnitId,
        );
        assert_eq!(
            handle_request(&mut bank, Some(7), 7, &read_pdu(0x01, 0, 1)),
            vec![0x01, 0x01, 0x00]
        );
        // The unit check precedes function dispatch.
        assert_exception(
            &handle_request(&mut bank, Some(7), 2, &[0x2B]),
            0x2B,
            ExceptionCode::WrongUnitId,
        );
    }

    struct FailingTarget;

    impl Target for FailingTarget {
        fn read_bits(
            &self,
            _table: Table,
            _start: u16,
            _quantity: u16,
        ) -> Result<Vec<u8>, RequestError> {
            Err(RequestError::ServerDeviceFailure)
        }

        fn read_registers(
            &self,
            _table: Table,
            _start: u16,
            _quantity: u16,
        ) -> Result<Vec<u16>, RequestError> {
            Err(RequestError::ServerDeviceFailure)
        }

        fn write_coil(&mut self, _address: u16, _value: bool) -> Result<(), RequestError> {
            Err(RequestError::ServerDeviceFailure)
        }

        fn write_register(&mut self, _address: u16, _value: u16) -> Result<(), RequestError> {
            Err(RequestError::ServerDeviceFailure)
        }

        fn write_coils(&mut self, _start: u16, _values: &[bool]) -> Result<(), RequestError> {
            Err(RequestError::ServerDeviceFailure)
        }

        fn write_registers(&mut self, _start: u16, _values: &[u16]) -> Result<(), RequestError> {
            Err(RequestError::ServerDeviceFailure)
        }
    }

    #[test]
    fn internal_failures_return_exception_0x04() {
        let mut target = FailingTarget;
        assert_exception(
            &handle_pdu(&mut target, None, 1, &read_pdu(0x03, 0, 1)),
            0x03,
            ExceptionCode::ServerDeviceFailure,
        );
        assert_exception(
            &handle_pdu(&mut target, None, 1, &write_single_pdu(0x06, 0, 1)),
            0x06,
            ExceptionCode::ServerDeviceFailure,
        );
        assert_exception(
            &handle_pdu(&mut target, None, 1, &write_coils_pdu(0, &[true])),
            0x0F,
            ExceptionCode::ServerDeviceFailure,
        );
    }

    // ------------------------------------------------------------ byte order

    #[test]
    fn eight_bit_low_high_placement() {
        let mut bank = placed(
            "low",
            Direction::Output,
            8,
            Table::HoldingRegister,
            0,
            BytePlacement::Low,
        );
        assert!(bank.write_channel("low", 0x34));
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 0, 1))),
            vec![0x0034]
        );

        let mut bank = placed(
            "high",
            Direction::Output,
            8,
            Table::HoldingRegister,
            0,
            BytePlacement::High,
        );
        assert!(bank.write_channel("high", 0x12));
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 0, 1))),
            vec![0x1200]
        );

        let declarations = [
            declaration("low", Direction::Output, 8),
            declaration("high", Direction::Output, 8),
        ];
        let entries = [
            mapped(
                "low",
                Table::HoldingRegister,
                0,
                Some(BytePlacement::Low),
                None,
            ),
            mapped(
                "high",
                Table::HoldingRegister,
                0,
                Some(BytePlacement::High),
                None,
            ),
        ];
        let mut bank = high_first_bank(&declarations, &entries);
        assert!(bank.write_channel("low", 0x34));
        assert!(bank.write_channel("high", 0x12));
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 0, 1))),
            vec![0x1234]
        );
        assert_eq!(bank.read_channel("low"), Some(0x34));
        assert_eq!(bank.read_channel("high"), Some(0x12));

        // An unclaimed byte reads as zero and ignores writes.
        let mut bank = placed(
            "low",
            Direction::Input,
            8,
            Table::HoldingRegister,
            0,
            BytePlacement::Low,
        );
        assert_eq!(
            handle(&mut bank, &write_single_pdu(0x06, 0, 0xABCD)),
            vec![0x06, 0, 0, 0xAB, 0xCD]
        );
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 0, 1))),
            vec![0x00CD]
        );
    }

    #[test]
    fn mixed_byte_pair_updates_only_the_input_byte() {
        let declarations = [
            declaration("in_low", Direction::Input, 8),
            declaration("out_high", Direction::Output, 8),
        ];
        let entries = [
            mapped(
                "in_low",
                Table::HoldingRegister,
                0,
                Some(BytePlacement::Low),
                None,
            ),
            mapped(
                "out_high",
                Table::HoldingRegister,
                0,
                Some(BytePlacement::High),
                None,
            ),
        ];
        let mut bank = high_first_bank(&declarations, &entries);
        assert!(bank.write_channel("out_high", 0x12));
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 0, 1))),
            vec![0x1200]
        );

        // FC6 updates the input low byte and preserves the output high byte.
        let request = write_single_pdu(0x06, 0, 0xFF34);
        assert_eq!(handle(&mut bank, &request), request);
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 0, 1))),
            vec![0x1234]
        );
        assert_eq!(bank.read_channel("in_low"), Some(0x34));
        assert_eq!(bank.read_channel("out_high"), Some(0x12));

        // FC16 has the same partial-byte behavior for this mixed register.
        let request = write_registers_pdu(0, &[0x5678]);
        assert_eq!(handle(&mut bank, &request), vec![0x10, 0, 0, 0, 1]);
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 0, 1))),
            vec![0x1278]
        );
        assert_eq!(bank.read_channel("in_low"), Some(0x78));
        assert_eq!(bank.read_channel("out_high"), Some(0x12));
    }

    #[test]
    fn named_word_orders_32_bit() {
        let value = 0xAABB_CCDD_u64;
        assert_eq!(
            read_words_for_order(32, WordOrder::HighFirst, value),
            vec![0xAABB, 0xCCDD]
        );
        assert_eq!(
            read_words_for_order(32, WordOrder::Abcd, value),
            vec![0xAABB, 0xCCDD]
        );
        assert_eq!(
            read_words_for_order(32, WordOrder::LowFirst, value),
            vec![0xCCDD, 0xAABB]
        );
        assert_eq!(
            read_words_for_order(32, WordOrder::Cdab, value),
            vec![0xCCDD, 0xAABB]
        );
        assert_eq!(
            read_words_for_order(32, WordOrder::Dcba, value),
            vec![0xDDCC, 0xBBAA]
        );
        assert_eq!(
            read_words_for_order(32, WordOrder::Badc, value),
            vec![0xBBAA, 0xDDCC]
        );
    }

    #[test]
    fn named_word_orders_64_bit() {
        let value = 0xAABB_CCDD_EEFF_0011_u64;
        assert_eq!(
            read_words_for_order(64, WordOrder::HighFirst, value),
            vec![0xAABB, 0xCCDD, 0xEEFF, 0x0011]
        );
        assert_eq!(
            read_words_for_order(64, WordOrder::Abcd, value),
            vec![0xAABB, 0xCCDD, 0xEEFF, 0x0011]
        );
        assert_eq!(
            read_words_for_order(64, WordOrder::LowFirst, value),
            vec![0x0011, 0xEEFF, 0xCCDD, 0xAABB],
            "GHEFCDAB"
        );
        assert_eq!(
            read_words_for_order(64, WordOrder::Cdab, value),
            vec![0xCCDD, 0xAABB, 0x0011, 0xEEFF],
            "CDABGHEF"
        );
        assert_eq!(
            read_words_for_order(64, WordOrder::Dcba, value),
            vec![0xDDCC, 0xBBAA, 0x1100, 0xFFEE]
        );
        assert_eq!(
            read_words_for_order(64, WordOrder::Badc, value),
            vec![0xBBAA, 0xDDCC, 0xFFEE, 0x1100]
        );
    }

    #[test]
    fn device_default_word_order_and_per_channel_override() {
        let value = 0xAABB_CCDD_EEFF_0011_u64;
        assert_eq!(
            read_words_for_default(64, DeviceWordOrder::HighFirst, value),
            read_words_for_order(64, WordOrder::HighFirst, value)
        );
        assert_eq!(
            read_words_for_default(64, DeviceWordOrder::LowFirst, value),
            vec![0x0011, 0xEEFF, 0xCCDD, 0xAABB]
        );
        assert_eq!(
            read_words_for_default(32, DeviceWordOrder::LowFirst, 0xAABB_CCDD),
            vec![0xCCDD, 0xAABB]
        );
        // Device low-first at 64 bits (GHEFCDAB) differs from per-channel
        // cdab (CDABGHEF).
        assert_ne!(
            read_words_for_default(64, DeviceWordOrder::LowFirst, value),
            read_words_for_order(64, WordOrder::Cdab, value)
        );
    }

    #[test]
    fn explicit_permutations_match_named_orders_and_round_trip() {
        let value = 0xAABB_CCDD_u64;
        assert_eq!(
            read_words_for_order(32, WordOrder::Explicit(vec![3, 2, 1, 0]), value),
            read_words_for_order(32, WordOrder::Dcba, value)
        );
        assert_eq!(
            read_words_for_order(32, WordOrder::Explicit(vec![1, 0, 3, 2]), value),
            read_words_for_order(32, WordOrder::Badc, value)
        );
        let value = 0xAABB_CCDD_EEFF_0011_u64;
        assert_eq!(
            read_words_for_order(64, WordOrder::Explicit(vec![6, 7, 4, 5, 2, 3, 0, 1]), value),
            read_words_for_order(64, WordOrder::LowFirst, value)
        );

        // A non-involutive explicit permutation exercises the true inverse.
        let mut bank = ordered(
            "value",
            Direction::Output,
            32,
            Table::HoldingRegister,
            0,
            WordOrder::Explicit(vec![1, 2, 3, 0]),
        );
        assert!(bank.write_channel("value", 0xAABB_CCDD));
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 0, 2))),
            vec![0xBBCC, 0xDDAA]
        );
        assert_eq!(bank.read_channel("value"), Some(0xAABB_CCDD));
    }

    #[test]
    fn sixteen_bit_registers_are_big_endian() {
        for value in [0x0000_u64, 0x0001, 0x00FF, 0x0100, 0x1234, 0xFFFF] {
            let mut bank = simple("word", Direction::Output, 16, Table::HoldingRegister, 0);
            assert!(bank.write_channel("word", value));
            let response = handle(&mut bank, &read_pdu(0x03, 0, 1));
            assert_eq!(response[0], 0x03);
            assert_eq!(response[1], 0x02);
            assert_eq!(&response[2..], &(value as u16).to_be_bytes());
            assert_eq!(bank.read_channel("word"), Some(value));
        }
    }

    // ----------------------------------------------- validation and atomicity

    #[test]
    fn table_direction_and_width_matrix() {
        for (table, direction, width) in [
            (Table::Coil, Direction::Input, 1),
            (Table::Coil, Direction::Output, 1),
            (Table::DiscreteInput, Direction::Output, 1),
            (Table::HoldingRegister, Direction::Input, 8),
            (Table::HoldingRegister, Direction::Input, 16),
            (Table::HoldingRegister, Direction::Input, 32),
            (Table::HoldingRegister, Direction::Input, 64),
            (Table::HoldingRegister, Direction::Output, 8),
            (Table::HoldingRegister, Direction::Output, 16),
            (Table::HoldingRegister, Direction::Output, 32),
            (Table::HoldingRegister, Direction::Output, 64),
            (Table::InputRegister, Direction::Output, 8),
            (Table::InputRegister, Direction::Output, 16),
            (Table::InputRegister, Direction::Output, 32),
            (Table::InputRegister, Direction::Output, 64),
        ] {
            self::build(
                &[declaration("channel", direction, width)],
                &[entry("channel", table, 0)],
                DeviceWordOrder::HighFirst,
            );
        }

        for table in [Table::DiscreteInput, Table::InputRegister] {
            map_error(
                &[declaration("channel", Direction::Input, 1)],
                &[entry("channel", table, 0)],
                DeviceWordOrder::HighFirst,
            );
        }
        for (data_table, direction, width) in [
            (Table::Coil, Direction::Input, 8),
            (Table::Coil, Direction::Output, 16),
            (Table::DiscreteInput, Direction::Output, 8),
            (Table::HoldingRegister, Direction::Input, 1),
            (Table::HoldingRegister, Direction::Input, 2),
            (Table::HoldingRegister, Direction::Input, 128),
            (Table::InputRegister, Direction::Output, 1),
        ] {
            map_error(
                &[declaration("channel", direction, width)],
                &[entry("channel", data_table, 0)],
                DeviceWordOrder::HighFirst,
            );
        }
    }

    #[test]
    fn byte_and_word_order_validation() {
        let error = map_error(
            &[declaration("v", Direction::Output, 16)],
            &[mapped(
                "v",
                Table::HoldingRegister,
                0,
                Some(BytePlacement::Low),
                None,
            )],
            DeviceWordOrder::HighFirst,
        );
        assert!(error.message.contains("byte"));

        map_error(
            &[declaration("v", Direction::Output, 16)],
            &[mapped(
                "v",
                Table::HoldingRegister,
                0,
                None,
                Some(WordOrder::LowFirst),
            )],
            DeviceWordOrder::HighFirst,
        );
        map_error(
            &[declaration("v", Direction::Output, 8)],
            &[mapped(
                "v",
                Table::HoldingRegister,
                0,
                None,
                Some(WordOrder::HighFirst),
            )],
            DeviceWordOrder::HighFirst,
        );
        map_error(
            &[declaration("v", Direction::Input, 1)],
            &[mapped("v", Table::Coil, 0, Some(BytePlacement::Low), None)],
            DeviceWordOrder::HighFirst,
        );
        map_error(
            &[declaration("v", Direction::Input, 1)],
            &[mapped(
                "v",
                Table::Coil,
                0,
                None,
                Some(WordOrder::HighFirst),
            )],
            DeviceWordOrder::HighFirst,
        );

        for list in [
            vec![0, 1, 2],
            vec![0, 1, 2, 3, 4],
            vec![0, 0, 1, 2],
            vec![0, 1, 2, 4],
        ] {
            map_error(
                &[declaration("v", Direction::Output, 32)],
                &[mapped(
                    "v",
                    Table::HoldingRegister,
                    0,
                    None,
                    Some(WordOrder::Explicit(list)),
                )],
                DeviceWordOrder::HighFirst,
            );
        }
        build(
            &[declaration("v", Direction::Output, 32)],
            &[mapped(
                "v",
                Table::HoldingRegister,
                0,
                None,
                Some(WordOrder::Explicit(vec![3, 2, 1, 0])),
            )],
            DeviceWordOrder::HighFirst,
        );
    }

    #[test]
    fn overlapping_register_spans_are_rejected_unless_low_high() {
        map_error(
            &[
                declaration("a", Direction::Input, 16),
                declaration("b", Direction::Input, 16),
            ],
            &[
                entry("a", Table::HoldingRegister, 0),
                entry("b", Table::HoldingRegister, 0),
            ],
            DeviceWordOrder::HighFirst,
        );
        self::build(
            &[
                declaration("a", Direction::Input, 16),
                declaration("b", Direction::Input, 16),
            ],
            &[
                entry("a", Table::HoldingRegister, 0),
                entry("b", Table::HoldingRegister, 1),
            ],
            DeviceWordOrder::HighFirst,
        );
        map_error(
            &[
                declaration("a", Direction::Input, 32),
                declaration("b", Direction::Input, 16),
            ],
            &[
                entry("a", Table::HoldingRegister, 0),
                entry("b", Table::HoldingRegister, 1),
            ],
            DeviceWordOrder::HighFirst,
        );
        self::build(
            &[
                declaration("a", Direction::Input, 32),
                declaration("b", Direction::Input, 32),
            ],
            &[
                entry("a", Table::HoldingRegister, 0),
                entry("b", Table::HoldingRegister, 2),
            ],
            DeviceWordOrder::HighFirst,
        );
        self::build(
            &[
                declaration("low", Direction::Input, 8),
                declaration("high", Direction::Output, 8),
            ],
            &[
                mapped(
                    "low",
                    Table::HoldingRegister,
                    0,
                    Some(BytePlacement::Low),
                    None,
                ),
                mapped(
                    "high",
                    Table::HoldingRegister,
                    0,
                    Some(BytePlacement::High),
                    None,
                ),
            ],
            DeviceWordOrder::HighFirst,
        );
        map_error(
            &[
                declaration("a", Direction::Input, 8),
                declaration("b", Direction::Input, 8),
            ],
            &[
                mapped(
                    "a",
                    Table::HoldingRegister,
                    0,
                    Some(BytePlacement::Low),
                    None,
                ),
                mapped(
                    "b",
                    Table::HoldingRegister,
                    0,
                    Some(BytePlacement::Low),
                    None,
                ),
            ],
            DeviceWordOrder::HighFirst,
        );
        map_error(
            &[
                declaration("a", Direction::Input, 8),
                declaration("b", Direction::Input, 16),
            ],
            &[
                mapped(
                    "a",
                    Table::HoldingRegister,
                    0,
                    Some(BytePlacement::Low),
                    None,
                ),
                entry("b", Table::HoldingRegister, 0),
            ],
            DeviceWordOrder::HighFirst,
        );
        map_error(
            &[
                declaration("a", Direction::Input, 1),
                declaration("b", Direction::Input, 1),
            ],
            &[entry("a", Table::Coil, 0), entry("b", Table::Coil, 0)],
            DeviceWordOrder::HighFirst,
        );
    }

    #[test]
    fn register_spans_stay_inside_the_address_space() {
        self::build(
            &[declaration("w", Direction::Input, 16)],
            &[entry("w", Table::HoldingRegister, 65_535)],
            DeviceWordOrder::HighFirst,
        );
        self::build(
            &[declaration("d", Direction::Input, 32)],
            &[entry("d", Table::HoldingRegister, 65_534)],
            DeviceWordOrder::HighFirst,
        );
        map_error(
            &[declaration("d", Direction::Input, 32)],
            &[entry("d", Table::HoldingRegister, 65_535)],
            DeviceWordOrder::HighFirst,
        );
        self::build(
            &[declaration("l", Direction::Input, 64)],
            &[entry("l", Table::HoldingRegister, 65_532)],
            DeviceWordOrder::HighFirst,
        );
        map_error(
            &[declaration("l", Direction::Input, 64)],
            &[entry("l", Table::HoldingRegister, 65_533)],
            DeviceWordOrder::HighFirst,
        );
        self::build(
            &[declaration("b", Direction::Input, 1)],
            &[entry("b", Table::Coil, 65_535)],
            DeviceWordOrder::HighFirst,
        );
    }

    #[test]
    fn channel_map_requires_one_entry_per_declared_channel() {
        map_error(
            &[declaration("a", Direction::Input, 1)],
            &[],
            DeviceWordOrder::HighFirst,
        );
        map_error(
            &[],
            &[entry("a", Table::Coil, 0)],
            DeviceWordOrder::HighFirst,
        );
        map_error(
            &[
                declaration("a", Direction::Input, 1),
                declaration("a", Direction::Output, 1),
            ],
            &[entry("a", Table::Coil, 0)],
            DeviceWordOrder::HighFirst,
        );
        map_error(
            &[declaration("a", Direction::Input, 1)],
            &[entry("a", Table::Coil, 0), entry("a", Table::Coil, 1)],
            DeviceWordOrder::HighFirst,
        );
        map_error(
            &[declaration("", Direction::Input, 1)],
            &[entry("", Table::Coil, 0)],
            DeviceWordOrder::HighFirst,
        );
    }

    #[test]
    fn partial_wide_writes_are_rejected_atomically() {
        let mut bank = ordered(
            "dword",
            Direction::Input,
            32,
            Table::HoldingRegister,
            0,
            WordOrder::HighFirst,
        );
        let request = write_registers_pdu(0, &[0x1122, 0x3344]);
        assert_eq!(handle(&mut bank, &request), vec![0x10, 0, 0, 0, 2]);
        assert_eq!(bank.read_channel("dword"), Some(0x1122_3344));

        assert_exception(
            &handle(&mut bank, &write_single_pdu(0x06, 0, 0xFFFF)),
            0x06,
            ExceptionCode::IllegalDataAddress,
        );
        assert_exception(
            &handle(&mut bank, &write_single_pdu(0x06, 1, 0xFFFF)),
            0x06,
            ExceptionCode::IllegalDataAddress,
        );
        assert_exception(
            &handle(&mut bank, &write_registers_pdu(0, &[0xFFFF])),
            0x10,
            ExceptionCode::IllegalDataAddress,
        );
        assert_exception(
            &handle(&mut bank, &write_registers_pdu(1, &[0xFFFF])),
            0x10,
            ExceptionCode::IllegalDataAddress,
        );
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 0, 2))),
            vec![0x1122, 0x3344],
            "rejected writes apply nothing"
        );

        let mut bank = ordered(
            "lword",
            Direction::Input,
            64,
            Table::HoldingRegister,
            0,
            WordOrder::HighFirst,
        );
        let request = write_registers_pdu(0, &[1, 2, 3, 4]);
        assert_eq!(handle(&mut bank, &request), vec![0x10, 0, 0, 0, 4]);
        assert_exception(
            &handle(&mut bank, &write_single_pdu(0x06, 0, 0xFFFF)),
            0x06,
            ExceptionCode::IllegalDataAddress,
        );
        assert_exception(
            &handle(&mut bank, &write_registers_pdu(0, &[9, 9, 9])),
            0x10,
            ExceptionCode::IllegalDataAddress,
        );
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 0, 4))),
            vec![1, 2, 3, 4],
            "rejected writes apply nothing"
        );
    }

    #[test]
    fn unmapped_write_ranges_apply_nothing_atomically() {
        let declarations = [
            declaration("a", Direction::Input, 16),
            declaration("b", Direction::Input, 16),
        ];
        let entries = [
            entry("a", Table::HoldingRegister, 10),
            entry("b", Table::HoldingRegister, 12),
        ];
        let mut bank = high_first_bank(&declarations, &entries);
        // Address 11 is unmapped: reject the whole request.
        assert_exception(
            &handle(&mut bank, &write_registers_pdu(10, &[0x1111, 0, 0x2222])),
            0x10,
            ExceptionCode::IllegalDataAddress,
        );
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 10, 1))),
            vec![0x0000]
        );
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 12, 1))),
            vec![0x0000]
        );
        // Writing a mapped address still works; a contiguous range cannot
        // skip the unmapped address 11.
        assert_eq!(
            handle(&mut bank, &write_registers_pdu(10, &[0x1111])),
            vec![0x10, 0, 10, 0, 1]
        );
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 10, 1))),
            vec![0x1111]
        );

        let declarations = [
            declaration("a", Direction::Input, 1),
            declaration("b", Direction::Input, 1),
        ];
        let entries = [entry("a", Table::Coil, 0), entry("b", Table::Coil, 2)];
        let mut bank = high_first_bank(&declarations, &entries);
        assert_exception(
            &handle(&mut bank, &write_coils_pdu(0, &[true, true, true])),
            0x0F,
            ExceptionCode::IllegalDataAddress,
        );
        assert_eq!(
            response_bits(&handle(&mut bank, &read_pdu(0x01, 0, 1)), 1),
            vec![false]
        );
        assert_eq!(
            response_bits(&handle(&mut bank, &read_pdu(0x01, 2, 1)), 1),
            vec![false]
        );
    }

    #[test]
    fn last_writer_wins_and_never_written_inputs_read_zero() {
        let mut bank = simple("h", Direction::Input, 16, Table::HoldingRegister, 0);
        assert_eq!(bank.read_channel("h"), Some(0));
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 0, 1))),
            vec![0x0000]
        );
        for value in [0x1111_u16, 0x2222] {
            assert_eq!(
                handle(&mut bank, &write_single_pdu(0x06, 0, value)),
                write_single_pdu(0x06, 0, value)
            );
        }
        assert_eq!(bank.read_channel("h"), Some(0x2222));

        let mut bank = simple("c", Direction::Input, 1, Table::Coil, 0);
        assert_eq!(
            handle(&mut bank, &read_pdu(0x01, 0, 1)),
            vec![0x01, 0x01, 0x00]
        );
        assert!(bank.write_channel("c", 1));
        // Coil inputs accept client writes; the last writer wins.
        let request = write_single_pdu(0x05, 0, 0x0000);
        assert_eq!(handle(&mut bank, &request), request);
        assert_eq!(
            handle(&mut bank, &read_pdu(0x01, 0, 1)),
            vec![0x01, 0x01, 0x00]
        );
        assert_eq!(bank.read_channel("c"), Some(0));

        let mut bank = simple("d", Direction::Input, 32, Table::HoldingRegister, 0);
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 0, 2))),
            vec![0, 0]
        );
    }

    #[test]
    fn reads_of_partial_wide_channels_return_raw_order_words() {
        let value = 0x0102_0304_0506_0708_u64;
        let mut bank = ordered(
            "lword",
            Direction::Output,
            64,
            Table::HoldingRegister,
            0,
            WordOrder::Cdab,
        );
        assert!(bank.write_channel("lword", value));
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 0, 4))),
            vec![0x0304, 0x0102, 0x0708, 0x0506]
        );
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 1, 2))),
            vec![0x0102, 0x0708]
        );
        assert_eq!(
            response_words(&handle(&mut bank, &read_pdu(0x03, 0, 1))),
            vec![0x0304]
        );
        assert_eq!(bank.read_channel("lword"), Some(value));
    }

    #[test]
    fn read_ranges_with_unmapped_addresses_return_only_an_exception() {
        let declarations = [
            declaration("a", Direction::Input, 16),
            declaration("b", Direction::Input, 16),
        ];
        let entries = [
            entry("a", Table::HoldingRegister, 0),
            entry("b", Table::HoldingRegister, 2),
        ];
        let mut bank = high_first_bank(&declarations, &entries);
        let response = handle(&mut bank, &read_pdu(0x03, 0, 3));
        assert_exception(&response, 0x03, ExceptionCode::IllegalDataAddress);
        assert_eq!(response.len(), 2, "no partial read data");

        let declarations = [
            declaration("a", Direction::Input, 1),
            declaration("b", Direction::Input, 1),
        ];
        let entries = [entry("a", Table::Coil, 0), entry("b", Table::Coil, 2)];
        let mut bank = high_first_bank(&declarations, &entries);
        let response = handle(&mut bank, &read_pdu(0x01, 0, 3));
        assert_exception(&response, 0x01, ExceptionCode::IllegalDataAddress);
        assert_eq!(response.len(), 2, "no partial read data");
    }

    #[test]
    fn channel_api_round_trips_all_widths_and_orders() {
        let mut bank = simple("bit", Direction::Output, 1, Table::Coil, 0);
        for value in [0_u64, 1] {
            assert!(bank.write_channel("bit", value));
            assert_eq!(bank.read_channel("bit"), Some(value));
        }

        for placement in [BytePlacement::Low, BytePlacement::High] {
            let mut bank = placed(
                "byte",
                Direction::Output,
                8,
                Table::HoldingRegister,
                0,
                placement,
            );
            for value in [0_u64, 1, 0x7F, 0x80, 0xFF] {
                assert!(bank.write_channel("byte", value));
                assert_eq!(bank.read_channel("byte"), Some(value));
            }
        }

        let mut bank = simple("word", Direction::Output, 16, Table::HoldingRegister, 0);
        for value in [0_u64, 1, 0x00FF, 0x0100, 0x1234, 0xFFFF] {
            assert!(bank.write_channel("word", value));
            assert_eq!(bank.read_channel("word"), Some(value));
        }

        for (width, values) in [
            (
                32_u32,
                &[0_u64, 1, 0x00FF, 0x0100, 0x1234, 0xFFFF, 0xAABB_CCDD][..],
            ),
            (
                64_u32,
                &[0_u64, 1, 0x1234, 0xFFFF, 0xAABB_CCDD_EEFF_0011][..],
            ),
        ] {
            for order in [
                WordOrder::HighFirst,
                WordOrder::LowFirst,
                WordOrder::Abcd,
                WordOrder::Cdab,
                WordOrder::Dcba,
                WordOrder::Badc,
            ] {
                let mut bank = ordered(
                    "wide",
                    Direction::Output,
                    width,
                    Table::HoldingRegister,
                    0,
                    order.clone(),
                );
                for &value in values {
                    assert!(
                        bank.write_channel("wide", value),
                        "width {width} order {order:?}"
                    );
                    assert_eq!(
                        bank.read_channel("wide"),
                        Some(value),
                        "width {width} order {order:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn channel_api_rejects_unknown_channels_and_oversized_values() {
        let mut bank = simple("bit", Direction::Output, 1, Table::Coil, 0);
        assert!(!bank.write_channel("bit", 2));
        assert_eq!(bank.read_channel("bit"), Some(0));
        assert!(!bank.write_channel("missing", 0));
        assert_eq!(bank.read_channel("missing"), None);

        let mut bank = simple("byte", Direction::Output, 8, Table::HoldingRegister, 0);
        assert!(!bank.write_channel("byte", 0x100));
        assert_eq!(bank.read_channel("byte"), Some(0));

        let mut bank = simple("word", Direction::Output, 16, Table::HoldingRegister, 0);
        assert!(!bank.write_channel("word", 0x1_0000));
        assert_eq!(bank.read_channel("word"), Some(0));
    }

    // ---- Phase 2 backend tests -------------------------------------------

    use crate::coordinator::{BackendBinding, BackendRegistry, Coordinator};
    use crate::io_mock::{MockEthercatBackend, MockWire};
    use crate::module_info::{
        ChannelInfo, DeviceInfo, DirectBinding, ModuleInfo, RouteEndpoint, RouteInfo,
    };
    use stc_got::GotId;

    const UNIT: u8 = 7;

    const LOOPBACK_CONFIG: &str = r#"{
        "bind": ["127.0.0.1:0", "127.0.0.1:0"],
        "unit": 7,
        "max-connections": 4,
        "idle-timeout": "5s",
        "word-order": "low-first",
        "channels": [
            {"name": "coil_in", "table": "coil", "address": 0},
            {"name": "coil_out", "table": "coil", "address": 10},
            {"name": "discrete_out", "table": "discrete-input", "address": 0},
            {"name": "hr_u8_low", "table": "holding-register", "address": 0, "byte": "low"},
            {"name": "hr_u8_high", "table": "holding-register", "address": 0, "byte": "high"},
            {"name": "hr_u16_in", "table": "holding-register", "address": 1},
            {"name": "hr_u32_in", "table": "holding-register", "address": 2, "word-order": "abcd"},
            {"name": "hr_u64_in", "table": "holding-register", "address": 4},
            {"name": "hr_unbound", "table": "holding-register", "address": 30},
            {"name": "hr_u16_out", "table": "holding-register", "address": 20},
            {"name": "hr_u32_out", "table": "holding-register", "address": 22, "word-order": "cdab"},
            {"name": "ir_u16_out", "table": "input-register", "address": 0},
            {"name": "ir_u64_out", "table": "input-register", "address": 2,
             "word-order": [4, 5, 6, 7, 0, 1, 2, 3]}
        ]
    }"#;

    fn at(area: u8, offset_bits: u64, width_bits: u64) -> ImageRange {
        ImageRange {
            area,
            offset_bits,
            width_bits,
        }
    }

    fn backend_channel(
        name: &str,
        direction: &str,
        width_bits: u32,
        image: Option<ImageRange>,
    ) -> BackendChannel {
        BackendChannel {
            name: name.into(),
            direction: direction.into(),
            width_bits,
            image,
        }
    }

    fn status_channel(name: &str, width_bits: u32, offset_bits: u64) -> BackendChannel {
        backend_channel(
            name,
            "input",
            width_bits,
            Some(at(1, offset_bits, u64::from(width_bits))),
        )
    }

    fn test_backend_config() -> BackendConfig {
        let data_channels = vec![
            backend_channel("coil_in", "input", 1, Some(at(1, 0, 1))),
            backend_channel("hr_u8_low", "input", 8, Some(at(1, 8, 8))),
            backend_channel("hr_u8_high", "input", 8, Some(at(1, 16, 8))),
            backend_channel("hr_u16_in", "input", 16, Some(at(1, 24, 16))),
            backend_channel("hr_u32_in", "input", 32, Some(at(1, 40, 32))),
            backend_channel("hr_u64_in", "input", 64, Some(at(1, 72, 64))),
            backend_channel("hr_unbound", "input", 16, None),
            backend_channel("coil_out", "output", 1, Some(at(2, 0, 1))),
            backend_channel("discrete_out", "output", 1, Some(at(2, 1, 1))),
            backend_channel("hr_u16_out", "output", 16, Some(at(2, 8, 16))),
            backend_channel("hr_u32_out", "output", 32, Some(at(2, 32, 32))),
            backend_channel("ir_u16_out", "output", 16, Some(at(2, 64, 16))),
            backend_channel("ir_u64_out", "output", 64, Some(at(2, 80, 64))),
        ];
        let status_channels = vec![
            status_channel("server0.listening", 1, 136),
            status_channel("server0.listeners", 16, 144),
            status_channel("server0.bound_listeners", 16, 160),
            status_channel("server0.connections", 16, 176),
            status_channel("server0.requests", 32, 192),
            status_channel("server0.writes", 32, 224),
            status_channel("server0.exceptions", 32, 256),
            status_channel("server0.malformed", 32, 288),
            status_channel("server0.last_error", 32, 320),
        ];
        let mut bindings = Vec::new();
        for (device, channels) in [("mb", &data_channels), ("mb-status", &status_channels)] {
            for channel in channels {
                if let Some(image) = channel.image {
                    bindings.push(BackendBinding {
                        device: device.into(),
                        channel: channel.name.clone(),
                        image,
                        direction: channel.direction.clone(),
                    });
                }
            }
        }
        BackendConfig {
            name: "modbus-tcp".into(),
            devices: vec![
                BackendDeviceConfig {
                    name: "mb".into(),
                    kind: MODBUS_TCP_KIND.into(),
                    config: Some(LOOPBACK_CONFIG.into()),
                    channels: data_channels,
                },
                BackendDeviceConfig {
                    name: "mb-status".into(),
                    kind: MODBUS_TCP_STATUS_KIND.into(),
                    config: None,
                    channels: status_channels,
                },
            ],
            bindings,
            layout: ImageLayout {
                inputs_bytes: 44,
                outputs_bytes: 18,
                markers_bytes: 0,
            },
            profile: None,
        }
    }

    fn diagnostic_bind(backend: &ModbusTcpBackend, listener: usize) -> String {
        backend
            .diagnostics()
            .get(&format!("server0.listener{listener}.bind"))
            .cloned()
            .expect("listener bind diagnostic")
    }

    fn test_client(bind: &str) -> TcpStream {
        let address: SocketAddr = bind.parse().expect("effective bind");
        let stream = TcpStream::connect(address).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .expect("write timeout");
        stream
    }

    fn read_response(stream: &mut TcpStream) -> Vec<u8> {
        let mut header = [0_u8; 7];
        stream.read_exact(&mut header).expect("MBAP header");
        assert_eq!(u16::from_be_bytes([header[2], header[3]]), 0);
        let length = usize::from(u16::from_be_bytes([header[4], header[5]]));
        assert!(length >= 2, "MBAP length {length}");
        let mut pdu = vec![0_u8; length - 1];
        stream.read_exact(&mut pdu).expect("PDU");
        pdu
    }

    fn exchange(stream: &mut TcpStream, transaction: u16, unit: u8, pdu: &[u8]) -> Vec<u8> {
        let adu = encode_adu(transaction, unit, pdu).expect("adu");
        stream.write_all(&adu).expect("request");
        read_response(stream)
    }

    fn image_value(image: &ProcessImage, area: u8, offset: u64, width: u64) -> u64 {
        image.read_bits(area, offset, width).unwrap_or_else(|| {
            panic!(
                "%{} range {offset}+{width} missing",
                if area == 1 { "I" } else { "Q" }
            )
        })
    }

    #[test]
    fn loopback_server_serves_tables_widths_status_and_shutdown() {
        let config = test_backend_config();
        let mut backend = ModbusTcpBackend::new();
        backend.cold_start(&config).expect("cold start");
        assert_eq!(backend.status().state, BackendState::Ready);
        assert_eq!(backend.status().devices, 2);
        assert_eq!(backend.unbound_channels(), 1);
        assert_eq!(backend.diagnostics()["server0.unbound"], "1");
        assert_eq!(backend.diagnostics()["server0.listeners"], "2");
        assert_eq!(backend.diagnostics()["server0.bound_listeners"], "2");
        assert_eq!(
            backend.diagnostics()["server0.listener0.state"],
            "listening"
        );
        assert_eq!(
            backend.diagnostics()["server0.listener1.state"],
            "listening"
        );

        let bind_a = diagnostic_bind(&backend, 0);
        let bind_b = diagnostic_bind(&backend, 1);
        assert_ne!(bind_a, bind_b);
        let mut client = test_client(&bind_a);

        // Client writes are visible at the next latch for every input width.
        assert_eq!(
            exchange(&mut client, 1, UNIT, &write_single_pdu(0x05, 0, 0xFF00)),
            write_single_pdu(0x05, 0, 0xFF00)
        );
        assert_eq!(
            exchange(&mut client, 2, UNIT, &write_single_pdu(0x06, 0, 0xAB12)),
            write_single_pdu(0x06, 0, 0xAB12)
        );
        exchange(
            &mut client,
            3,
            UNIT,
            &write_registers_pdu(1, &[0x1234, 0x1122, 0x3344]),
        );
        exchange(
            &mut client,
            4,
            UNIT,
            &write_registers_pdu(4, &[0xCDEF, 0x89AB, 0x4567, 0x0123]),
        );

        let mut image = ProcessImage::new(config.layout);
        backend.latch_inputs(&mut image).expect("latch");
        assert_eq!(backend.status().state, BackendState::Running);
        assert_eq!(image_value(&image, 1, 0, 1), 1);
        assert_eq!(image_value(&image, 1, 8, 8), 0x12);
        assert_eq!(image_value(&image, 1, 16, 8), 0xAB);
        assert_eq!(image_value(&image, 1, 24, 16), 0x1234);
        assert_eq!(image_value(&image, 1, 40, 32), 0x1122_3344);
        assert_eq!(
            image_value(&image, 1, 72, 64),
            0x0123_4567_89AB_CDEF,
            "device low-first 64-bit order"
        );
        assert_eq!(image_value(&image, 1, 136, 1), 1);
        assert_eq!(image_value(&image, 1, 144, 16), 2);
        assert_eq!(image_value(&image, 1, 160, 16), 2);
        assert_eq!(image_value(&image, 1, 176, 16), 1);
        assert_eq!(image_value(&image, 1, 192, 32), 4);
        assert_eq!(image_value(&image, 1, 224, 32), 4);
        assert_eq!(image_value(&image, 1, 256, 32), 0);
        assert_eq!(image_value(&image, 1, 288, 32), 0);
        assert_eq!(image_value(&image, 1, 320, 32), 0);

        // PLC outputs reach clients on all four tables.
        assert!(image.write_bits(2, 0, 1, 1));
        assert!(image.write_bits(2, 1, 1, 1));
        assert!(image.write_bits(2, 8, 16, 0xBEEF));
        assert!(image.write_bits(2, 32, 32, 0x1122_3344));
        assert!(image.write_bits(2, 64, 16, 0xCAFE));
        assert!(image.write_bits(2, 80, 64, 0x0102_0304_0506_0708));
        backend.publish_outputs(&image).expect("publish");

        let response = exchange(&mut client, 5, UNIT, &read_pdu(0x01, 10, 1));
        assert_eq!(response_bits(&response, 1), vec![true]);
        let response = exchange(&mut client, 6, UNIT, &read_pdu(0x02, 0, 1));
        assert_eq!(response_bits(&response, 1), vec![true]);
        let response = exchange(&mut client, 7, UNIT, &read_pdu(0x03, 20, 1));
        assert_eq!(response_words(&response), vec![0xBEEF]);
        let response = exchange(&mut client, 8, UNIT, &read_pdu(0x03, 22, 2));
        assert_eq!(response_words(&response), vec![0x3344, 0x1122]);
        let response = exchange(&mut client, 9, UNIT, &read_pdu(0x04, 0, 1));
        assert_eq!(response_words(&response), vec![0xCAFE]);
        let response = exchange(&mut client, 10, UNIT, &read_pdu(0x04, 2, 4));
        assert_eq!(
            response_words(&response),
            vec![0x0506, 0x0708, 0x0102, 0x0304],
            "explicit per-channel permutation"
        );

        // Both listeners serve the same bank.
        let mut second = test_client(&bind_b);
        let response = exchange(&mut second, 11, UNIT, &read_pdu(0x01, 0, 1));
        assert_eq!(response_bits(&response, 1), vec![true]);
        let response = exchange(&mut second, 12, 8, &read_pdu(0x03, 20, 1));
        assert_exception(&response, 0x03, ExceptionCode::WrongUnitId);

        // Pipelined ADUs are answered in order on one connection.
        let first = encode_adu(13, UNIT, &read_pdu(0x03, 20, 1)).expect("first");
        let second = encode_adu(14, UNIT, &read_pdu(0x04, 0, 1)).expect("second");
        let mut pipelined = first;
        pipelined.extend_from_slice(&second);
        client.write_all(&pipelined).expect("pipelined requests");
        assert_eq!(response_words(&read_response(&mut client)), vec![0xBEEF]);
        assert_eq!(response_words(&read_response(&mut client)), vec![0xCAFE]);

        // Refresh the status block and assert the documented counters.
        backend.latch_inputs(&mut image).expect("latch status");
        assert_eq!(image_value(&image, 1, 176, 16), 2);
        assert_eq!(image_value(&image, 1, 192, 32), 14);
        assert_eq!(image_value(&image, 1, 224, 32), 4);
        assert_eq!(image_value(&image, 1, 256, 32), 1);
        assert_eq!(image_value(&image, 1, 288, 32), 0);
        assert_eq!(image_value(&image, 1, 320, 32), 0);
        let diagnostics = backend.diagnostics();
        assert_eq!(diagnostics["server0.requests"], "14");
        assert_eq!(diagnostics["server0.reads"], "9");
        assert_eq!(diagnostics["server0.writes"], "4");
        assert_eq!(diagnostics["server0.exceptions"], "1");
        assert_eq!(diagnostics["server0.malformed"], "0");
        assert_eq!(diagnostics["server0.last_error"], "0");
        assert_eq!(diagnostics["server0.peak_connections"], "2");
        assert_eq!(diagnostics["server0.listener0.connections"], "1");
        assert_eq!(diagnostics["server0.listener1.connections"], "1");

        drop(client);
        drop(second);
        backend.shutdown().expect("shutdown");
        backend.shutdown().expect("idempotent shutdown");
        assert_eq!(backend.status().state, BackendState::Stopped);
        let diagnostics = backend.diagnostics();
        assert_eq!(diagnostics["server0.listener0.state"], "stopped");
        assert_eq!(diagnostics["server0.listener1.state"], "stopped");
        assert_eq!(diagnostics["server0.bound_listeners"], "0");
    }

    const MIXED_CONFIG: &str = r#"{
        "bind": ["127.0.0.1:0"],
        "channels": [
            {"name": "mb_in", "table": "holding-register", "address": 0, "byte": "low"},
            {"name": "mb_out", "table": "holding-register", "address": 1, "byte": "low"}
        ]
    }"#;

    const ECAT_CONFIG: &str = r#"{
        "syncs": [
            {"index": 1, "direction": "input", "watchdog": "disable", "pdos": [{
                "index": "0x1a00",
                "entries": [{"index": "0x3001", "subindex": 1, "bits": 8}]
            }]},
            {"index": 2, "direction": "output", "watchdog": "enable", "pdos": [{
                "index": "0x1600",
                "entries": [{"index": "0x3101", "subindex": 1, "bits": 8}]
            }]}
        ],
        "pdo-channels": [
            {"name": "IN0", "entry": {"index": "0x3001", "subindex": 1}, "bit": 0},
            {"name": "OUT0", "entry": {"index": "0x3101", "subindex": 1}, "bit": 0}
        ]
    }"#;

    fn channel_info(name: &str, direction: &str, width_bits: u32) -> ChannelInfo {
        ChannelInfo {
            name: name.into(),
            direction: direction.into(),
            width_bits,
        }
    }

    fn direct_binding(
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
            channel: Some(crate::module_info::BindingChannel::Name(channel.into())),
            direction: Some(direction.into()),
        }
    }

    fn route_endpoint(device: &str, channel: &str, input: bool) -> RouteEndpoint {
        if input {
            RouteEndpoint::PhysicalInput {
                device: device.into(),
                channel: channel.into(),
                width_bits: 8,
            }
        } else {
            RouteEndpoint::PhysicalOutput {
                device: device.into(),
                channel: channel.into(),
                width_bits: 8,
            }
        }
    }

    fn mixed_registry(wire: &MockWire) -> BackendRegistry {
        let mut registry = BackendRegistry::new();
        for kind in ["ethercat", "ethercat-status"] {
            let wire = wire.clone();
            registry.register(kind, "ethercat", move || {
                Box::new(MockEthercatBackend::with_wire(wire.clone()))
            });
        }
        for kind in [MODBUS_TCP_KIND, MODBUS_TCP_STATUS_KIND] {
            registry.register(kind, "modbus-tcp", || Box::new(ModbusTcpBackend::new()));
        }
        registry
    }

    #[test]
    fn coordinator_serves_modbus_and_ethercat_over_one_image() {
        let wire = MockWire::new();
        let registry = mixed_registry(&wire);
        let devices = vec![
            DeviceInfo {
                name: "ec".into(),
                kind: "ethercat".into(),
                config: Some(ECAT_CONFIG.into()),
                channels: vec![
                    channel_info("IN0", "input", 8),
                    channel_info("OUT0", "output", 8),
                ],
            },
            DeviceInfo {
                name: "mb".into(),
                kind: MODBUS_TCP_KIND.into(),
                config: Some(MIXED_CONFIG.into()),
                channels: vec![
                    channel_info("mb_in", "input", 8),
                    channel_info("mb_out", "output", 8),
                ],
            },
        ];
        let bindings = vec![
            direct_binding(1, 1, 0, 8, "mb", "mb_in", "input"),
            direct_binding(3, 1, 16, 8, "ec", "IN0", "input"),
        ];
        let routes = vec![
            RouteInfo {
                source: route_endpoint("mb", "mb_in", true),
                sinks: vec![route_endpoint("ec", "OUT0", false)],
            },
            RouteInfo {
                source: route_endpoint("ec", "IN0", true),
                sinks: vec![route_endpoint("mb", "mb_out", false)],
            },
        ];
        let module = ModuleInfo {
            inputs_bytes: 3,
            outputs_bytes: 3,
            input_defaults: vec![0; 3],
            bindings,
            devices,
            routes,
            ..ModuleInfo::default()
        };
        let mut coordinator = Coordinator::cold_start(&module, &registry).expect("cold start");
        let reports = coordinator.reports();
        let modbus_report = reports
            .iter()
            .find(|report| report.name == "modbus-tcp")
            .expect("Modbus report");
        let bind = modbus_report.diagnostics["server0.listener0.bind"].clone();
        let mut client = test_client(&bind);

        // Modbus client write -> EtherCAT output over the shared image.
        exchange(&mut client, 1, 1, &write_single_pdu(0x06, 0, 0x00A1));
        coordinator.input_boundary();
        coordinator.after_scan();
        assert_eq!(wire.output("ec", "OUT0"), Some(0xA1));
        assert_eq!(coordinator.image().read_bits(1, 0, 8), Some(0xA1));

        // EtherCAT input -> Modbus client read over the shared image.
        wire.set_input("ec", "IN0", 0xB2);
        coordinator.input_boundary();
        coordinator.after_scan();
        assert_eq!(coordinator.image().read_bits(1, 16, 8), Some(0xB2));
        let response = exchange(&mut client, 2, 1, &read_pdu(0x03, 1, 1));
        assert_eq!(response_words(&response), vec![0x00B2]);

        // An EtherCAT fault is isolated: Modbus keeps serving and reporting.
        wire.set_fail_latch(true);
        coordinator.input_boundary();
        coordinator.after_scan();
        let reports = coordinator.reports();
        let ethercat = reports
            .iter()
            .find(|report| report.name == "ethercat")
            .expect("EtherCAT report");
        assert_eq!(ethercat.state, BackendState::Faulted);
        assert!(ethercat.faults >= 1);
        let modbus = reports
            .iter()
            .find(|report| report.name == "modbus-tcp")
            .expect("Modbus report");
        assert_eq!(modbus.state, BackendState::Running);
        assert_eq!(modbus.faults, 0);

        exchange(&mut client, 3, 1, &write_single_pdu(0x06, 0, 0x00A3));
        let response = exchange(&mut client, 4, 1, &read_pdu(0x03, 0, 1));
        assert_eq!(response_words(&response), vec![0x00A3]);
        coordinator.input_boundary();
        coordinator.after_scan();
        assert_eq!(
            coordinator
                .reports()
                .iter()
                .find(|report| report.name == "modbus-tcp")
                .expect("Modbus report")
                .state,
            BackendState::Running
        );
        coordinator.shutdown().expect("shutdown");
    }

    // ---- Phase 3 error, lifecycle, and online-change tests ----------------

    use crate::coordinator::configuration_hash;
    use crate::module_info::{BindingChannel, ProfileInfo};

    const DEFAULT_MAPPING: &str = r#"{"name":"in0","table":"holding-register","address":0}"#;

    fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if condition() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn diagnostic(backend: &ModbusTcpBackend, key: &str) -> String {
        backend
            .diagnostics()
            .get(key)
            .cloned()
            .unwrap_or_else(|| panic!("missing diagnostic `{key}`"))
    }

    fn diagnostic_number(backend: &ModbusTcpBackend, key: &str) -> u64 {
        diagnostic(backend, key)
            .parse()
            .unwrap_or_else(|_| panic!("diagnostic `{key}` is not a number"))
    }

    fn listener_bind_at(backend: &ModbusTcpBackend, server: usize, listener: usize) -> String {
        diagnostic(backend, &format!("server{server}.listener{listener}.bind"))
    }

    fn assert_connection_closed(stream: &mut TcpStream) {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("close read timeout");
        let mut byte = [0_u8; 1];
        match stream.read(&mut byte) {
            Ok(0) => {}
            Ok(count) => panic!("expected a closed connection, read {count} byte(s)"),
            Err(error) => assert!(
                !matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ),
                "the connection stayed open: {error}"
            ),
        }
    }

    fn bindings_for(device: &str, channels: &[BackendChannel]) -> Vec<BackendBinding> {
        channels
            .iter()
            .filter_map(|channel| {
                channel.image.map(|image| BackendBinding {
                    device: device.into(),
                    channel: channel.name.clone(),
                    image,
                    direction: channel.direction.clone(),
                })
            })
            .collect()
    }

    fn server_config_json(binds: &[&str], extra: &str, mappings: &[&str]) -> String {
        let binds = binds
            .iter()
            .map(|bind| format!("\"{bind}\""))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"bind\":[{binds}]{extra},\"channels\":[{}]}}",
            mappings.join(",")
        )
    }

    fn default_layout() -> ImageLayout {
        ImageLayout {
            inputs_bytes: 2,
            outputs_bytes: 0,
            markers_bytes: 0,
        }
    }

    fn input_channel(name: &str, offset_bits: u64, width_bits: u32) -> BackendChannel {
        backend_channel(
            name,
            "input",
            width_bits,
            Some(at(1, offset_bits, u64::from(width_bits))),
        )
    }

    fn output_channel(name: &str, offset_bits: u64, width_bits: u32) -> BackendChannel {
        backend_channel(
            name,
            "output",
            width_bits,
            Some(at(2, offset_bits, u64::from(width_bits))),
        )
    }

    fn single_modbus_config(
        binds: &[&str],
        extra: &str,
        mappings: &[&str],
        channels: Vec<BackendChannel>,
        layout: ImageLayout,
    ) -> BackendConfig {
        let config = server_config_json(binds, extra, mappings);
        let bindings = bindings_for("mb", &channels);
        BackendConfig {
            name: "modbus-tcp".into(),
            devices: vec![BackendDeviceConfig {
                name: "mb".into(),
                kind: MODBUS_TCP_KIND.into(),
                config: Some(config),
                channels,
            }],
            bindings,
            layout,
            profile: None,
        }
    }

    fn default_input_config(binds: &[&str]) -> BackendConfig {
        single_modbus_config(
            binds,
            "",
            &[DEFAULT_MAPPING],
            vec![input_channel("in0", 0, 16)],
            default_layout(),
        )
    }

    fn raw_config(config: &str, channels: Vec<BackendChannel>) -> BackendConfig {
        let bindings = bindings_for("mb", &channels);
        BackendConfig {
            name: "modbus-tcp".into(),
            devices: vec![BackendDeviceConfig {
                name: "mb".into(),
                kind: MODBUS_TCP_KIND.into(),
                config: Some(config.into()),
                channels,
            }],
            bindings,
            layout: default_layout(),
            profile: None,
        }
    }

    fn cold_start_error(config: &BackendConfig) -> Error {
        let mut backend = ModbusTcpBackend::new();
        let error = backend
            .cold_start(config)
            .expect_err("cold start must fail closed");
        assert_eq!(backend.status().state, BackendState::Stopped);
        assert!(
            backend.diagnostics().is_empty(),
            "a failed cold start leaves no server diagnostics"
        );
        error
    }

    #[test]
    fn malformed_framing_closes_connections_and_counts_once() {
        let config = default_input_config(&["127.0.0.1:0"]);
        let mut backend = ModbusTcpBackend::new();
        backend.cold_start(&config).expect("cold start");
        let bind = listener_bind_at(&backend, 0, 0);
        let mut malformed = 0_u64;
        let mut requests = 0_u64;

        // A non-zero protocol id is a framing error.
        let mut client = test_client(&bind);
        client
            .write_all(&[0, 1, 0, 1, 0, 6, UNIT, 0x03, 0, 0, 0, 1])
            .expect("protocol id request");
        assert!(wait_until(|| diagnostic_number(
            &backend,
            "server0.malformed"
        ) > malformed));
        malformed += 1;
        assert_eq!(diagnostic_number(&backend, "server0.malformed"), malformed);
        assert_connection_closed(&mut client);

        // The MBAP length must be 2..=254.
        let mut client = test_client(&bind);
        client
            .write_all(&[0, 2, 0, 0, 0, 1, UNIT, 0x03])
            .expect("short length request");
        assert!(wait_until(|| diagnostic_number(
            &backend,
            "server0.malformed"
        ) > malformed));
        malformed += 1;
        assert_connection_closed(&mut client);

        // A length above 254 is an oversized ADU.
        let mut client = test_client(&bind);
        client
            .write_all(&[0, 3, 0, 0, 0, 255, UNIT, 0x03])
            .expect("oversized request");
        assert!(wait_until(|| diagnostic_number(
            &backend,
            "server0.malformed"
        ) > malformed));
        malformed += 1;
        assert_connection_closed(&mut client);

        // Closing with a partial ADU is a truncated frame.
        let mut client = test_client(&bind);
        client
            .write_all(&[0, 4, 0, 0, 0, 6, UNIT, 0x03, 0])
            .expect("partial request");
        client
            .shutdown(std::net::Shutdown::Write)
            .expect("half close");
        assert!(wait_until(|| diagnostic_number(
            &backend,
            "server0.malformed"
        ) > malformed));
        malformed += 1;
        assert_connection_closed(&mut client);

        // A valid pipelined ADU is answered before the invalid frame closes.
        let mut client = test_client(&bind);
        let mut pipelined = encode_adu(5, UNIT, &read_pdu(0x03, 0, 1)).expect("valid adu");
        pipelined.extend_from_slice(&[0, 6, 0, 7, 0, 6, UNIT, 0x03, 0, 0, 0, 1]);
        client.write_all(&pipelined).expect("pipelined request");
        assert_eq!(response_words(&read_response(&mut client)), vec![0]);
        requests += 1;
        assert!(wait_until(|| diagnostic_number(
            &backend,
            "server0.malformed"
        ) > malformed));
        malformed += 1;
        assert_eq!(diagnostic_number(&backend, "server0.requests"), requests);
        assert_connection_closed(&mut client);

        // The largest legal length is incomplete when the peer closes.
        let mut client = test_client(&bind);
        client
            .write_all(&[0, 7, 0, 0, 0, 254, UNIT])
            .expect("max length partial request");
        client
            .shutdown(std::net::Shutdown::Write)
            .expect("half close");
        assert!(wait_until(|| diagnostic_number(
            &backend,
            "server0.malformed"
        ) > malformed));
        malformed += 1;
        assert_connection_closed(&mut client);

        assert_eq!(diagnostic_number(&backend, "server0.malformed"), malformed);
        assert_eq!(diagnostic_number(&backend, "server0.last_error"), 0);
        assert_eq!(backend.status().faults, 0);
        assert_eq!(backend.status().state, BackendState::Ready);

        // Framing errors are client-level events: the listener keeps serving.
        let mut client = test_client(&bind);
        assert_eq!(
            response_words(&exchange(&mut client, 8, UNIT, &read_pdu(0x03, 0, 1))),
            vec![0]
        );
        backend.shutdown().expect("shutdown");
    }

    #[test]
    fn semantic_errors_return_exceptions_and_preserve_values() {
        let channels = vec![
            input_channel("in16", 0, 16),
            input_channel("in32", 16, 32),
            backend_channel("coil_in", "input", 1, Some(at(1, 48, 1))),
            input_channel("in8_low", 56, 8),
            input_channel("in8_high", 64, 8),
            output_channel("out16", 0, 16),
            backend_channel("coil_out", "output", 1, Some(at(2, 16, 1))),
        ];
        let mappings = [
            r#"{"name":"in16","table":"holding-register","address":0}"#,
            r#"{"name":"in32","table":"holding-register","address":2}"#,
            r#"{"name":"coil_in","table":"coil","address":0}"#,
            r#"{"name":"in8_low","table":"holding-register","address":6,"byte":"low"}"#,
            r#"{"name":"in8_high","table":"holding-register","address":6,"byte":"high"}"#,
            r#"{"name":"out16","table":"holding-register","address":10}"#,
            r#"{"name":"coil_out","table":"coil","address":5}"#,
        ];
        let layout = ImageLayout {
            inputs_bytes: 9,
            outputs_bytes: 3,
            markers_bytes: 0,
        };
        let config = single_modbus_config(
            &["127.0.0.1:0"],
            ",\"unit\":7,\"max-connections\":4",
            &mappings,
            channels,
            layout,
        );
        let mut backend = ModbusTcpBackend::new();
        backend.cold_start(&config).expect("cold start");
        let bind = listener_bind_at(&backend, 0, 0);
        let mut image = ProcessImage::new(layout);
        let mut client = test_client(&bind);

        // Seed one value in every writable table, then publish an output.
        assert_eq!(
            exchange(&mut client, 1, UNIT, &write_single_pdu(0x06, 0, 0x1234)),
            write_single_pdu(0x06, 0, 0x1234)
        );
        exchange(
            &mut client,
            2,
            UNIT,
            &write_registers_pdu(2, &[0xAAAA, 0xBBBB]),
        );
        exchange(&mut client, 3, UNIT, &write_single_pdu(0x06, 6, 0xAABB));
        exchange(&mut client, 4, UNIT, &write_single_pdu(0x05, 0, 0xFF00));
        assert!(image.write_bits(2, 0, 16, 0x5678));
        backend.publish_outputs(&image).expect("publish");
        assert_eq!(
            response_words(&exchange(&mut client, 5, UNIT, &read_pdu(0x03, 10, 1))),
            vec![0x5678]
        );
        let reads_before = diagnostic_number(&backend, "server0.reads");
        let writes_before = diagnostic_number(&backend, "server0.writes");
        let requests_before = diagnostic_number(&backend, "server0.requests");
        let mut exceptions = 0_u64;
        let mut expect_exception =
            |client: &mut TcpStream, transaction: u16, pdu: &[u8], function: u8, code| {
                let response = exchange(client, transaction, UNIT, pdu);
                assert_exception(&response, function, code);
                exceptions += 1;
            };

        // Unsupported function codes are 0x01 and keep the connection open.
        expect_exception(
            &mut client,
            10,
            &[0x07],
            0x07,
            ExceptionCode::IllegalFunction,
        );
        expect_exception(
            &mut client,
            11,
            &[0x2B, 0x0E, 0x01, 0x00],
            0x2B,
            ExceptionCode::IllegalFunction,
        );
        // Quantity limits and PDU shapes are 0x03.
        expect_exception(
            &mut client,
            12,
            &read_pdu(0x01, 0, 0),
            0x01,
            ExceptionCode::IllegalDataValue,
        );
        expect_exception(
            &mut client,
            13,
            &read_pdu(0x01, 0, 2001),
            0x01,
            ExceptionCode::IllegalDataValue,
        );
        expect_exception(
            &mut client,
            14,
            &read_pdu(0x03, 0, 0),
            0x03,
            ExceptionCode::IllegalDataValue,
        );
        expect_exception(
            &mut client,
            15,
            &read_pdu(0x03, 0, 126),
            0x03,
            ExceptionCode::IllegalDataValue,
        );
        expect_exception(
            &mut client,
            16,
            &write_single_pdu(0x05, 0, 0x1234),
            0x05,
            ExceptionCode::IllegalDataValue,
        );
        expect_exception(
            &mut client,
            17,
            &[0x0F, 0, 0, 0, 8, 2, 0, 0],
            0x0F,
            ExceptionCode::IllegalDataValue,
        );
        expect_exception(
            &mut client,
            18,
            &[0x10, 0, 0, 0, 1, 4, 0, 1, 0, 2],
            0x10,
            ExceptionCode::IllegalDataValue,
        );
        // Unmapped, read-only-table, and output-only writes are 0x02.
        expect_exception(
            &mut client,
            19,
            &write_single_pdu(0x06, 100, 1),
            0x06,
            ExceptionCode::IllegalDataAddress,
        );
        expect_exception(
            &mut client,
            20,
            &write_single_pdu(0x06, 10, 1),
            0x06,
            ExceptionCode::IllegalDataAddress,
        );
        expect_exception(
            &mut client,
            21,
            &write_single_pdu(0x06, 2, 0xFFFF),
            0x06,
            ExceptionCode::IllegalDataAddress,
        );
        expect_exception(
            &mut client,
            22,
            &write_registers_pdu(2, &[0xFFFF]),
            0x10,
            ExceptionCode::IllegalDataAddress,
        );
        expect_exception(
            &mut client,
            23,
            &write_registers_pdu(0, &[0x9999, 0x8888, 0x7777]),
            0x10,
            ExceptionCode::IllegalDataAddress,
        );
        expect_exception(
            &mut client,
            24,
            &[0x0F, 0, 5, 0, 1, 1, 1],
            0x0F,
            ExceptionCode::IllegalDataAddress,
        );
        expect_exception(
            &mut client,
            25,
            &read_pdu(0x04, 0, 1),
            0x04,
            ExceptionCode::IllegalDataAddress,
        );
        expect_exception(
            &mut client,
            26,
            &read_pdu(0x01, 0, 2),
            0x01,
            ExceptionCode::IllegalDataAddress,
        );
        expect_exception(
            &mut client,
            27,
            &write_registers_pdu(65535, &[1, 2]),
            0x10,
            ExceptionCode::IllegalDataAddress,
        );
        expect_exception(
            &mut client,
            28,
            &write_single_pdu(0x06, 65535, 1),
            0x06,
            ExceptionCode::IllegalDataAddress,
        );
        // A configured unit id rejects every other unit with 0x0B.
        let response = exchange(&mut client, 29, 8, &read_pdu(0x03, 0, 1));
        assert_exception(&response, 0x03, ExceptionCode::WrongUnitId);
        exceptions += 1;

        // The connection is still healthy and no failed write applied.
        assert_eq!(
            response_words(&exchange(&mut client, 30, UNIT, &read_pdu(0x03, 0, 1))),
            vec![0x1234]
        );
        assert_eq!(
            response_words(&exchange(&mut client, 31, UNIT, &read_pdu(0x03, 2, 2))),
            vec![0xAAAA, 0xBBBB]
        );
        assert_eq!(
            response_words(&exchange(&mut client, 32, UNIT, &read_pdu(0x03, 6, 1))),
            vec![0xAABB]
        );
        assert_eq!(
            response_bits(&exchange(&mut client, 33, UNIT, &read_pdu(0x01, 0, 1)), 1),
            vec![true]
        );
        assert_eq!(
            response_words(&exchange(&mut client, 34, UNIT, &read_pdu(0x03, 10, 1))),
            vec![0x5678]
        );

        assert_eq!(
            diagnostic_number(&backend, "server0.exceptions"),
            exceptions
        );
        assert_eq!(
            diagnostic_number(&backend, "server0.reads"),
            reads_before + 5
        );
        assert_eq!(diagnostic_number(&backend, "server0.writes"), writes_before);
        assert_eq!(
            diagnostic_number(&backend, "server0.requests"),
            requests_before + exceptions + 5
        );
        assert_eq!(diagnostic_number(&backend, "server0.malformed"), 0);
        assert_eq!(diagnostic_number(&backend, "server0.last_error"), 0);
        assert_eq!(backend.status().faults, 0);
        backend.shutdown().expect("shutdown");
    }

    #[test]
    fn connection_limit_is_shared_across_binds() {
        let config = single_modbus_config(
            &["127.0.0.1:0", "127.0.0.1:0"],
            ",\"max-connections\":2",
            &[DEFAULT_MAPPING],
            vec![input_channel("in0", 0, 16)],
            default_layout(),
        );
        let mut backend = ModbusTcpBackend::new();
        backend.cold_start(&config).expect("cold start");
        let first_bind = listener_bind_at(&backend, 0, 0);
        let second_bind = listener_bind_at(&backend, 0, 1);
        assert_ne!(first_bind, second_bind, "port-0 listeners are distinct");

        let mut first = test_client(&first_bind);
        let mut second = test_client(&second_bind);
        assert!(wait_until(|| {
            diagnostic_number(&backend, "server0.connections") == 2
        }));
        // An absent unit accepts any request unit.
        exchange(&mut first, 1, 200, &read_pdu(0x03, 0, 1));
        exchange(&mut second, 2, 0, &read_pdu(0x03, 0, 1));
        assert_eq!(diagnostic_number(&backend, "server0.peak_connections"), 2);
        assert_eq!(
            diagnostic_number(&backend, "server0.listener0.connections"),
            1
        );
        assert_eq!(
            diagnostic_number(&backend, "server0.listener1.connections"),
            1
        );

        // The third connection is accepted and closed immediately.
        let mut third = test_client(&first_bind);
        assert_connection_closed(&mut third);
        assert_eq!(diagnostic_number(&backend, "server0.connections"), 2);
        assert_eq!(diagnostic_number(&backend, "server0.peak_connections"), 2);

        // Releasing one slot admits a new connection on either bind.
        drop(first);
        assert!(wait_until(|| {
            diagnostic_number(&backend, "server0.connections") == 1
        }));
        let mut fourth = test_client(&second_bind);
        assert!(wait_until(|| {
            diagnostic_number(&backend, "server0.connections") == 2
        }));
        exchange(&mut fourth, 3, UNIT, &read_pdu(0x03, 0, 1));
        assert_eq!(diagnostic_number(&backend, "server0.peak_connections"), 2);
        backend.shutdown().expect("shutdown");
        drop(second);
        drop(fourth);
    }

    #[test]
    fn idle_timeout_closes_silent_slow_and_partial_clients() {
        let config = single_modbus_config(
            &["127.0.0.1:0"],
            ",\"idle-timeout\":\"500ms\",\"max-connections\":4",
            &[DEFAULT_MAPPING],
            vec![input_channel("in0", 0, 16)],
            default_layout(),
        );
        let mut backend = ModbusTcpBackend::new();
        backend.cold_start(&config).expect("cold start");
        let bind = listener_bind_at(&backend, 0, 0);
        let malformed_before = diagnostic_number(&backend, "server0.malformed");

        // A silent connection is closed after the idle interval.
        let mut silent = test_client(&bind);
        assert_connection_closed(&mut silent);
        assert_eq!(
            diagnostic_number(&backend, "server0.malformed"),
            malformed_before
        );

        // A partial ADU that stops mid-frame closes under the same timeout and
        // is counted as malformed.
        let mut partial = test_client(&bind);
        let adu = encode_adu(1, UNIT, &read_pdu(0x03, 0, 1)).expect("adu");
        partial.write_all(&adu[..4]).expect("partial request");
        assert!(wait_until(|| {
            diagnostic_number(&backend, "server0.malformed") > malformed_before
        }));
        assert_eq!(
            diagnostic_number(&backend, "server0.malformed"),
            malformed_before + 1
        );
        assert_connection_closed(&mut partial);

        // A slow but complete request is served, and traffic below the idle
        // interval keeps one connection alive across the timeout.
        let mut slow = test_client(&bind);
        for chunk in adu.chunks(3) {
            slow.write_all(chunk).expect("slow chunk");
            std::thread::sleep(Duration::from_millis(40));
        }
        assert_eq!(response_words(&read_response(&mut slow)), vec![0]);
        for transaction in 2..8 {
            assert_eq!(
                response_words(&exchange(
                    &mut slow,
                    transaction,
                    UNIT,
                    &read_pdu(0x03, 0, 1)
                )),
                vec![0]
            );
            std::thread::sleep(Duration::from_millis(150));
        }
        assert_connection_closed(&mut slow);
        assert_eq!(
            diagnostic_number(&backend, "server0.malformed"),
            malformed_before + 1
        );

        // Reconnecting after an idle close works.
        let mut reconnected = test_client(&bind);
        assert_eq!(
            response_words(&exchange(&mut reconnected, 9, UNIT, &read_pdu(0x03, 0, 1))),
            vec![0]
        );
        assert_eq!(backend.status().faults, 0);
        backend.shutdown().expect("shutdown");
    }

    #[test]
    fn abrupt_disconnects_do_not_fault_or_leak_connections() {
        let config = default_input_config(&["127.0.0.1:0"]);
        let mut backend = ModbusTcpBackend::new();
        backend.cold_start(&config).expect("cold start");
        let bind = listener_bind_at(&backend, 0, 0);
        for transaction in 0..4 {
            let mut client = test_client(&bind);
            assert!(wait_until(|| {
                diagnostic_number(&backend, "server0.connections") == 1
            }));
            assert_eq!(
                response_words(&exchange(
                    &mut client,
                    transaction,
                    UNIT,
                    &read_pdu(0x03, 0, 1)
                )),
                vec![0]
            );
            // Drop without an explicit shutdown; the slot must be released.
            drop(client);
            assert!(wait_until(|| {
                diagnostic_number(&backend, "server0.connections") == 0
            }));
        }
        // A client that vanishes mid-frame is counted once as malformed and
        // releases its slot.
        let mut partial = test_client(&bind);
        let adu = encode_adu(9, UNIT, &read_pdu(0x03, 0, 1)).expect("adu");
        partial.write_all(&adu[..5]).expect("partial request");
        drop(partial);
        assert!(wait_until(|| {
            diagnostic_number(&backend, "server0.connections") == 0
                && diagnostic_number(&backend, "server0.malformed") == 1
        }));
        assert_eq!(backend.status().state, BackendState::Ready);
        assert_eq!(backend.status().faults, 0);
        assert_eq!(diagnostic_number(&backend, "server0.last_error"), 0);
        assert_eq!(diagnostic_number(&backend, "server0.malformed"), 1);
        assert_eq!(diagnostic_number(&backend, "server0.connections"), 0);
        let mut client = test_client(&bind);
        assert_eq!(
            response_words(&exchange(&mut client, 9, UNIT, &read_pdu(0x03, 0, 1))),
            vec![0]
        );
        backend.shutdown().expect("shutdown");
    }

    #[test]
    fn multi_writes_are_atomic_and_last_writer_wins_under_a_racing_latch() {
        let channels = vec![
            input_channel("pair_a", 0, 16),
            input_channel("pair_b", 16, 16),
            input_channel("wide", 32, 32),
        ];
        let mappings = [
            r#"{"name":"pair_a","table":"holding-register","address":0}"#,
            r#"{"name":"pair_b","table":"holding-register","address":1}"#,
            r#"{"name":"wide","table":"holding-register","address":2}"#,
        ];
        let layout = ImageLayout {
            inputs_bytes: 8,
            outputs_bytes: 0,
            markers_bytes: 0,
        };
        let config = single_modbus_config(
            &["127.0.0.1:0"],
            ",\"max-connections\":4",
            &mappings,
            channels,
            layout,
        );
        let mut backend = ModbusTcpBackend::new();
        backend.cold_start(&config).expect("cold start");
        let bind = listener_bind_at(&backend, 0, 0);
        let mut first = test_client(&bind);
        let mut second = test_client(&bind);

        // Last writer wins across independent connections.
        exchange(
            &mut first,
            1,
            UNIT,
            &write_registers_pdu(0, &[0x1111, 0x2222]),
        );
        assert_eq!(
            response_words(&exchange(&mut second, 2, UNIT, &read_pdu(0x03, 0, 2))),
            vec![0x1111, 0x2222]
        );
        exchange(
            &mut second,
            3,
            UNIT,
            &write_registers_pdu(0, &[0x3333, 0x4444]),
        );
        assert_eq!(
            response_words(&exchange(&mut first, 4, UNIT, &read_pdu(0x03, 0, 2))),
            vec![0x3333, 0x4444]
        );

        // A whole 32-bit span round-trips through the device default order.
        exchange(
            &mut first,
            5,
            UNIT,
            &write_registers_pdu(2, &[0xAAAA, 0xBBBB]),
        );
        assert_eq!(
            response_words(&exchange(&mut second, 6, UNIT, &read_pdu(0x03, 2, 2))),
            vec![0xAAAA, 0xBBBB]
        );

        // A multi-address write that partially covers the 32-bit channel is
        // rejected without applying any word.
        assert_exception(
            &exchange(
                &mut second,
                7,
                UNIT,
                &write_registers_pdu(1, &[0x9999, 0x8888]),
            ),
            0x10,
            ExceptionCode::IllegalDataAddress,
        );
        assert_eq!(
            response_words(&exchange(&mut first, 8, UNIT, &read_pdu(0x03, 0, 2))),
            vec![0x3333, 0x4444]
        );
        assert_eq!(
            response_words(&exchange(&mut first, 9, UNIT, &read_pdu(0x03, 2, 2))),
            vec![0xAAAA, 0xBBBB]
        );

        // A racing latch must see whole FC16 requests only.
        let stop = Arc::new(AtomicBool::new(false));
        let writer_stop = stop.clone();
        let writer_bind = bind.clone();
        let writer = std::thread::spawn(move || {
            let mut client = test_client(&writer_bind);
            let patterns = [
                ([0x1111_u16, 0x2222], [0xAAAA_u16, 0xBBBB]),
                ([0x3333, 0x4444], [0xCCCC, 0xDDDD]),
            ];
            let mut transaction = 100_u16;
            while !writer_stop.load(Ordering::SeqCst) {
                for &(pair, wide) in &patterns {
                    let _ = exchange(
                        &mut client,
                        transaction,
                        UNIT,
                        &write_registers_pdu(0, &pair),
                    );
                    transaction = transaction.wrapping_add(1);
                    let _ = exchange(
                        &mut client,
                        transaction,
                        UNIT,
                        &write_registers_pdu(2, &wide),
                    );
                    transaction = transaction.wrapping_add(1);
                }
            }
        });
        let mut image = ProcessImage::new(layout);
        for _ in 0..100 {
            backend.latch_inputs(&mut image).expect("latch");
            let pair_a = image_value(&image, 1, 0, 16);
            let pair_b = image_value(&image, 1, 16, 16);
            assert!(
                matches!(
                    (pair_a, pair_b),
                    (0, 0) | (0x1111, 0x2222) | (0x3333, 0x4444)
                ),
                "torn multi-address write {pair_a:#06x}/{pair_b:#06x}"
            );
            let wide = image_value(&image, 1, 32, 32);
            assert!(
                matches!(wide, 0 | 0xAAAA_BBBB | 0xCCCC_DDDD),
                "torn 32-bit write {wide:#010x}"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        stop.store(true, Ordering::SeqCst);
        writer.join().expect("writer thread");
        assert_eq!(diagnostic_number(&backend, "server0.malformed"), 0);
        assert_eq!(backend.status().faults, 0);
        backend.shutdown().expect("shutdown");
    }

    fn two_server_config() -> BackendConfig {
        let mb0_channels = vec![input_channel("in0", 0, 16)];
        let mb1_channels = vec![input_channel("in1", 16, 16)];
        let status_channels = vec![
            status_channel("server0.listening", 1, 32),
            status_channel("server0.listeners", 16, 40),
            status_channel("server0.bound_listeners", 16, 56),
            status_channel("server1.listening", 1, 72),
            status_channel("server0.requests", 32, 80),
            status_channel("server1.requests", 32, 112),
            status_channel("server0.last_error", 32, 144),
        ];
        let mb0_config = server_config_json(
            &["127.0.0.1:0", "127.0.0.1:0"],
            "",
            &[r#"{"name":"in0","table":"holding-register","address":0}"#],
        );
        let mb1_config = server_config_json(
            &["127.0.0.1:0"],
            "",
            &[r#"{"name":"in1","table":"holding-register","address":0}"#],
        );
        let mut bindings = bindings_for("mb0", &mb0_channels);
        bindings.extend(bindings_for("mb1", &mb1_channels));
        bindings.extend(bindings_for("status", &status_channels));
        BackendConfig {
            name: "modbus-tcp".into(),
            devices: vec![
                BackendDeviceConfig {
                    name: "mb0".into(),
                    kind: MODBUS_TCP_KIND.into(),
                    config: Some(mb0_config),
                    channels: mb0_channels,
                },
                BackendDeviceConfig {
                    name: "mb1".into(),
                    kind: MODBUS_TCP_KIND.into(),
                    config: Some(mb1_config),
                    channels: mb1_channels,
                },
                BackendDeviceConfig {
                    name: "status".into(),
                    kind: MODBUS_TCP_STATUS_KIND.into(),
                    config: Some("{\"layout\":1}".into()),
                    channels: status_channels,
                },
            ],
            bindings,
            layout: ImageLayout {
                inputs_bytes: 22,
                outputs_bytes: 0,
                markers_bytes: 0,
            },
            profile: None,
        }
    }

    #[test]
    fn listener_faults_are_isolated_and_partial_listening_is_observable() {
        let config = two_server_config();
        let mut backend = ModbusTcpBackend::new();
        backend.cold_start(&config).expect("cold start");
        assert_eq!(diagnostic_number(&backend, "server0.listeners"), 2);
        assert_eq!(diagnostic_number(&backend, "server1.listeners"), 1);
        let s0_first = listener_bind_at(&backend, 0, 0);
        let s0_second = listener_bind_at(&backend, 0, 1);
        let s1_bind = listener_bind_at(&backend, 1, 0);
        let mut client0 = test_client(&s0_first);
        let mut client0_second = test_client(&s0_second);
        let mut client1 = test_client(&s1_bind);
        exchange(&mut client0, 1, UNIT, &write_single_pdu(0x06, 0, 0x1111));
        exchange(
            &mut client0_second,
            2,
            UNIT,
            &write_single_pdu(0x06, 0, 0x2222),
        );
        exchange(&mut client1, 3, UNIT, &write_single_pdu(0x06, 0, 0x3333));
        let mut image = ProcessImage::new(config.layout);
        backend.latch_inputs(&mut image).expect("latch");
        assert_eq!(image_value(&image, 1, 0, 16), 0x2222);
        assert_eq!(image_value(&image, 1, 16, 16), 0x3333);
        assert_eq!(image_value(&image, 1, 32, 1), 1);
        assert_eq!(image_value(&image, 1, 40, 16), 2);
        assert_eq!(image_value(&image, 1, 56, 16), 2);
        assert_eq!(image_value(&image, 1, 72, 1), 1);
        assert_eq!(image_value(&image, 1, 80, 32), 2);
        assert_eq!(image_value(&image, 1, 112, 32), 1);
        assert_eq!(backend.status().state, BackendState::Running);

        // Fault both listeners of server0; server1 must keep serving.
        backend.inject_listener_fault(0, 0);
        backend.inject_listener_fault(0, 1);
        assert!(wait_until(|| {
            diagnostic(&backend, "server0.listener0.state") == "faulted"
                && diagnostic(&backend, "server0.listener1.state") == "faulted"
        }));
        assert_eq!(diagnostic_number(&backend, "server0.bound_listeners"), 0);
        assert_eq!(diagnostic_number(&backend, "server1.bound_listeners"), 1);
        assert_eq!(diagnostic(&backend, "server1.listener0.state"), "listening");
        assert_eq!(
            diagnostic_number(&backend, "server0.listener0.last_error"),
            2
        );
        assert_eq!(
            diagnostic_number(&backend, "server0.listener1.last_error"),
            2
        );
        assert_eq!(backend.status().state, BackendState::Faulted);
        assert_eq!(backend.status().faults, 2);
        let last_fault = backend.status().last_fault.expect("fault message");
        assert!(last_fault.contains(codes::LISTENER_FAULT), "{last_fault}");

        // The other server and the already accepted connections keep serving.
        assert_eq!(
            response_words(&exchange(&mut client1, 4, UNIT, &read_pdu(0x03, 0, 1))),
            vec![0x3333]
        );
        assert_eq!(
            response_words(&exchange(&mut client0, 5, UNIT, &read_pdu(0x03, 0, 1))),
            vec![0x2222]
        );
        backend.latch_inputs(&mut image).expect("latch status");
        assert_eq!(image_value(&image, 1, 32, 1), 0);
        assert_eq!(image_value(&image, 1, 56, 16), 0);
        assert_eq!(image_value(&image, 1, 72, 1), 1);
        assert_eq!(image_value(&image, 1, 144, 32), 2);

        // One listener recovers while the other is still faulted: the server is
        // not listening yet, and the aggregate state stays Faulted.
        backend.release_listener_fault(0, 0);
        assert!(wait_until(|| {
            diagnostic(&backend, "server0.listener0.state") == "listening"
        }));
        assert_eq!(diagnostic_number(&backend, "server0.bound_listeners"), 1);
        assert_eq!(backend.status().state, BackendState::Faulted);
        backend.latch_inputs(&mut image).expect("latch partial");
        assert_eq!(image_value(&image, 1, 32, 1), 0);
        assert_eq!(image_value(&image, 1, 56, 16), 1);

        // Releasing the second listener restores Running and full listening.
        backend.release_listener_fault(0, 1);
        assert!(wait_until(|| {
            diagnostic_number(&backend, "server0.bound_listeners") == 2
                && backend.status().state == BackendState::Running
        }));
        assert_eq!(
            diagnostic_number(&backend, "server0.listener0.last_error"),
            2
        );
        backend.latch_inputs(&mut image).expect("latch recovered");
        assert_eq!(image_value(&image, 1, 32, 1), 1);
        assert_eq!(image_value(&image, 1, 56, 16), 2);
        assert_eq!(backend.status().faults, 2);

        // A new connection is accepted on the rebound listener.
        let rebind = listener_bind_at(&backend, 0, 0);
        let mut recovered = test_client(&rebind);
        assert_eq!(
            response_words(&exchange(&mut recovered, 6, UNIT, &read_pdu(0x03, 0, 1))),
            vec![0x2222]
        );
        backend.shutdown().expect("shutdown");
    }

    #[test]
    fn socket_write_failure_ends_only_the_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("pair listener");
        let address = listener.local_addr().expect("pair address");
        let mut client = TcpStream::connect(address).expect("pair client");
        let (server_stream, _) = listener.accept().expect("pair server");
        client
            .write_all(&encode_adu(1, UNIT, &read_pdu(0x03, 0, 1)).expect("adu"))
            .expect("request");
        // Make every server write fail without involving a real peer failure.
        server_stream
            .shutdown(std::net::Shutdown::Write)
            .expect("shut down server write half");
        let runtime = Arc::new(ServerRuntime {
            counters: ServerCounters::default(),
            listeners: vec![Arc::new(ListenerRuntime::new(address.to_string()))],
            fault: Mutex::new(None),
        });
        let bank = Mutex::new(simple(
            "in0",
            Direction::Input,
            16,
            Table::HoldingRegister,
            0,
        ));
        serve_connection(
            server_stream,
            &runtime,
            &bank,
            None,
            Duration::from_secs(60),
            &AtomicBool::new(false),
        );
        // The request was served and counted; only the reply write failed.
        assert_eq!(runtime.counters.requests.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.counters.reads.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.counters.exceptions.load(Ordering::SeqCst), 0);
        assert_eq!(runtime.counters.last_error.load(Ordering::SeqCst), 0);
        assert_eq!(
            runtime.listeners[0].state.load(Ordering::SeqCst),
            LISTENER_LISTENING
        );
        assert_eq!(runtime.listeners[0].last_error.load(Ordering::SeqCst), 0);
        assert_connection_closed(&mut client);
    }

    #[test]
    fn cold_start_bind_failure_is_all_or_nothing() {
        // Occupy the second address; the first address is free and concrete.
        let occupied = TcpListener::bind("127.0.0.1:0").expect("occupy a port");
        let occupied_address = occupied.local_addr().expect("occupied address");
        let free = TcpListener::bind("127.0.0.1:0").expect("reserve a port");
        let free_address = free.local_addr().expect("free address");
        drop(free);
        let first = free_address.to_string();
        let second = occupied_address.to_string();
        let config = default_input_config(&[&first, &second]);

        let error = cold_start_error(&config);
        assert_eq!(error.code, codes::BIND_FAILURE);
        assert!(error.message.contains("mb"), "{}", error.message);
        assert!(
            error.message.contains(&second),
            "the error names the failing address: {}",
            error.message
        );
        // The listener bound before the failure was closed again.
        TcpListener::bind(free_address).expect("the partial listener set must be released");

        // A failure on the very first bind leaves no listener either.
        let config = default_input_config(&[&second, &first]);
        let error = cold_start_error(&config);
        assert_eq!(error.code, codes::BIND_FAILURE);
        TcpListener::bind(free_address).expect("no listener was left behind");

        // Once the conflict clears, the same configuration starts normally.
        drop(occupied);
        let config = default_input_config(&[&first, &second]);
        let mut backend = ModbusTcpBackend::new();
        backend
            .cold_start(&config)
            .expect("cold start after the conflict");
        assert_eq!(backend.status().state, BackendState::Ready);
        assert_eq!(diagnostic_number(&backend, "server0.bound_listeners"), 2);
        let first_bind = listener_bind_at(&backend, 0, 0);
        let second_bind = listener_bind_at(&backend, 0, 1);
        assert_eq!(
            first_bind.parse::<SocketAddr>().expect("first bind"),
            free_address
        );
        assert_eq!(
            second_bind.parse::<SocketAddr>().expect("second bind"),
            occupied_address
        );
        let mut client = test_client(&first_bind);
        assert_eq!(
            response_words(&exchange(&mut client, 1, UNIT, &read_pdu(0x03, 0, 1))),
            vec![0]
        );
        backend.shutdown().expect("shutdown");
    }

    #[test]
    fn shutdown_and_drop_stop_listeners_without_rewriting_the_image() {
        let config = default_input_config(&["127.0.0.1:0"]);
        let mut backend = ModbusTcpBackend::new();
        backend.cold_start(&config).expect("cold start");
        let bind = listener_bind_at(&backend, 0, 0);
        let address: SocketAddr = bind.parse().expect("effective bind");
        let mut client = test_client(&bind);
        exchange(&mut client, 1, UNIT, &write_single_pdu(0x06, 0, 0x1357));
        let mut image = ProcessImage::new(config.layout);
        backend.latch_inputs(&mut image).expect("latch");
        assert_eq!(image_value(&image, 1, 0, 16), 0x1357);

        backend.shutdown().expect("shutdown");
        assert_eq!(backend.status().state, BackendState::Stopped);
        // Shutdown itself must not rewrite the image.
        assert_eq!(image_value(&image, 1, 0, 16), 0x1357);
        assert_eq!(diagnostic(&backend, "server0.listener0.state"), "stopped");
        assert_eq!(diagnostic_number(&backend, "server0.bound_listeners"), 0);
        assert_connection_closed(&mut client);
        assert!(
            TcpStream::connect(address).is_err(),
            "a stopped listener refuses new connections"
        );

        // Shutdown is idempotent, and a later latch still sees the last value.
        backend.shutdown().expect("second shutdown");
        backend.latch_inputs(&mut image).expect("latch after stop");
        assert_eq!(image_value(&image, 1, 0, 16), 0x1357);
        assert_eq!(backend.status().state, BackendState::Stopped);

        // The same instance can cold start again with a fresh bank.
        backend.cold_start(&config).expect("restart");
        assert_eq!(backend.status().state, BackendState::Ready);
        let rebind = listener_bind_at(&backend, 0, 0);
        let mut restarted = test_client(&rebind);
        assert_eq!(
            response_words(&exchange(&mut restarted, 2, UNIT, &read_pdu(0x03, 0, 1))),
            vec![0]
        );
        backend.shutdown().expect("shutdown");

        // Drop without an explicit shutdown stops the threads and keeps the
        // process image unchanged.
        let mut dropped_image = ProcessImage::new(config.layout);
        let drop_bind = {
            let mut other = ModbusTcpBackend::new();
            other.cold_start(&config).expect("cold start for drop");
            let bind = listener_bind_at(&other, 0, 0);
            let mut other_client = test_client(&bind);
            exchange(
                &mut other_client,
                1,
                UNIT,
                &write_single_pdu(0x06, 0, 0x2468),
            );
            other.latch_inputs(&mut dropped_image).expect("latch");
            bind
        };
        assert_eq!(image_value(&dropped_image, 1, 0, 16), 0x2468);
        let drop_address: SocketAddr = drop_bind.parse().expect("drop bind");
        assert!(
            TcpStream::connect(drop_address).is_err(),
            "drop stops the listener"
        );
        TcpListener::bind(drop_address).expect("drop releases the listener address");
    }

    fn status_backend(
        data_devices: usize,
        status_channels: Vec<BackendChannel>,
        config: Option<&str>,
        layout: ImageLayout,
    ) -> BackendConfig {
        let mut devices = Vec::new();
        let mut bindings = Vec::new();
        for index in 0..data_devices {
            let name = format!("mb{index}");
            let channel_name = format!("in{index}");
            let mapping = format!(
                "{{\"name\":\"{channel_name}\",\"table\":\"holding-register\",\"address\":0}}"
            );
            let channel = input_channel(&channel_name, 0, 16);
            bindings.extend(bindings_for(&name, std::slice::from_ref(&channel)));
            devices.push(BackendDeviceConfig {
                name,
                kind: MODBUS_TCP_KIND.into(),
                config: Some(server_config_json(&["127.0.0.1:0"], "", &[&mapping])),
                channels: vec![channel],
            });
        }
        let status = BackendDeviceConfig {
            name: "status".into(),
            kind: MODBUS_TCP_STATUS_KIND.into(),
            config: config.map(str::to_owned),
            channels: status_channels.clone(),
        };
        bindings.extend(bindings_for("status", &status_channels));
        devices.push(status);
        BackendConfig {
            name: "modbus-tcp".into(),
            devices,
            bindings,
            layout,
            profile: None,
        }
    }

    fn status_layout(bits: u64) -> ImageLayout {
        ImageLayout {
            inputs_bytes: bits.div_ceil(8),
            outputs_bytes: 0,
            markers_bytes: 0,
        }
    }

    fn assert_config_failure(config: &BackendConfig, code: &str, label: &str) {
        let error = cold_start_error(config);
        assert_eq!(error.code, code, "{label}: {}", error.message);
    }

    #[test]
    fn status_fields_reflect_live_counters_and_listener_state() {
        let status_channels = vec![
            status_channel("server0.listening", 1, 32),
            status_channel("server0.listeners", 16, 40),
            status_channel("server0.bound_listeners", 16, 56),
            status_channel("server0.connections", 16, 72),
            status_channel("server0.requests", 32, 88),
            status_channel("server0.writes", 32, 120),
            status_channel("server0.exceptions", 32, 152),
            status_channel("server0.malformed", 32, 184),
            status_channel("server0.last_error", 32, 216),
        ];
        let layout = status_layout(248);
        let config = status_backend(1, status_channels, Some("{\"layout\":1}"), layout);
        let mut backend = ModbusTcpBackend::new();
        backend.cold_start(&config).expect("cold start");
        let bind = listener_bind_at(&backend, 0, 0);
        let mut client = test_client(&bind);
        assert_eq!(
            exchange(&mut client, 1, UNIT, &write_single_pdu(0x06, 0, 0x0102)),
            write_single_pdu(0x06, 0, 0x0102)
        );
        assert_eq!(
            response_words(&exchange(&mut client, 2, UNIT, &read_pdu(0x03, 0, 1))),
            vec![0x0102]
        );
        assert_exception(
            &exchange(&mut client, 3, UNIT, &[0x07]),
            0x07,
            ExceptionCode::IllegalFunction,
        );
        let mut malformed = test_client(&bind);
        malformed
            .write_all(&[0, 4, 0, 1, 0, 6, UNIT, 0x03, 0, 0, 0, 1])
            .expect("bad protocol id");
        assert!(wait_until(|| {
            diagnostic_number(&backend, "server0.malformed") == 1
        }));
        assert_connection_closed(&mut malformed);
        drop(malformed);
        assert!(wait_until(|| {
            diagnostic_number(&backend, "server0.connections") == 1
        }));

        let mut image = ProcessImage::new(layout);
        backend.latch_inputs(&mut image).expect("latch");
        assert_eq!(image_value(&image, 1, 32, 1), 1);
        assert_eq!(image_value(&image, 1, 40, 16), 1);
        assert_eq!(image_value(&image, 1, 56, 16), 1);
        assert_eq!(image_value(&image, 1, 72, 16), 1);
        assert_eq!(image_value(&image, 1, 88, 32), 3);
        assert_eq!(image_value(&image, 1, 120, 32), 1);
        assert_eq!(image_value(&image, 1, 152, 32), 1);
        assert_eq!(image_value(&image, 1, 184, 32), 1);
        assert_eq!(image_value(&image, 1, 216, 32), 0);
        assert_eq!(diagnostic_number(&backend, "server0.requests"), 3);
        assert_eq!(diagnostic_number(&backend, "server0.reads"), 1);
        assert_eq!(diagnostic_number(&backend, "server0.writes"), 1);
        assert_eq!(diagnostic_number(&backend, "server0.exceptions"), 1);
        assert_eq!(diagnostic_number(&backend, "server0.malformed"), 1);
        assert_eq!(diagnostic_number(&backend, "server0.last_error"), 0);

        drop(client);
        assert!(wait_until(|| {
            diagnostic_number(&backend, "server0.connections") == 0
        }));
        backend.latch_inputs(&mut image).expect("latch after close");
        assert_eq!(image_value(&image, 1, 72, 16), 0);
        assert_eq!(image_value(&image, 1, 88, 32), 3);
        backend.shutdown().expect("shutdown");
    }

    #[test]
    fn status_device_accepts_optional_unbound_fields() {
        let status_channels = vec![
            status_channel("server0.listening", 1, 16),
            backend_channel("server0.requests", "input", 32, None),
        ];
        let layout = status_layout(24);
        let config = status_backend(1, status_channels, None, layout);
        let mut backend = ModbusTcpBackend::new();
        backend
            .cold_start(&config)
            .expect("status device without config");
        assert_eq!(backend.unbound_channels(), 0);
        assert_eq!(backend.status().devices, 2);
        let mut image = ProcessImage::new(layout);
        backend.latch_inputs(&mut image).expect("latch");
        assert_eq!(image_value(&image, 1, 0, 16), 0);
        assert_eq!(image_value(&image, 1, 16, 1), 1);
        backend.shutdown().expect("shutdown");
    }

    #[test]
    fn status_device_validation_fails_closed() {
        let layout = status_layout(64);
        let listening = || status_channel("server0.listening", 1, 0);
        assert_config_failure(
            &status_backend(1, vec![listening()], Some("{\"layout\":2}"), layout),
            codes::INVALID_CONFIG,
            "unknown status layout",
        );
        assert_config_failure(
            &status_backend(1, vec![listening()], Some("{\"layout\":\"1\"}"), layout),
            codes::INVALID_CONFIG,
            "status layout must be a number",
        );
        assert_config_failure(
            &status_backend(1, vec![listening()], Some("[]"), layout),
            codes::INVALID_CONFIG,
            "status config must be an object",
        );
        assert_config_failure(
            &status_backend(1, vec![listening()], Some("{"), layout),
            codes::INVALID_CONFIG,
            "status config must be JSON",
        );
        assert_config_failure(
            &status_backend(
                1,
                vec![backend_channel(
                    "server0.bogus",
                    "input",
                    16,
                    Some(at(1, 0, 16)),
                )],
                None,
                layout,
            ),
            codes::INVALID_CONFIG,
            "unknown status channel",
        );
        assert_config_failure(
            &status_backend(
                1,
                vec![backend_channel(
                    "server.listening",
                    "input",
                    1,
                    Some(at(1, 0, 1)),
                )],
                None,
                layout,
            ),
            codes::INVALID_CONFIG,
            "malformed status channel name",
        );
        assert_config_failure(
            &status_backend(
                1,
                vec![status_channel("server0.listening", 16, 0)],
                None,
                layout,
            ),
            codes::INVALID_CONFIG,
            "status channel width differs from layout 1",
        );
        assert_config_failure(
            &status_backend(
                1,
                vec![backend_channel(
                    "server0.listening",
                    "output",
                    1,
                    Some(at(1, 0, 1)),
                )],
                None,
                layout,
            ),
            codes::INVALID_CONFIG,
            "status channels are read-only",
        );
        assert_config_failure(
            &status_backend(
                1,
                vec![status_channel("server1.listening", 1, 0)],
                None,
                layout,
            ),
            codes::INVALID_CONFIG,
            "status server index out of range",
        );
        assert_config_failure(
            &status_backend(0, vec![listening()], None, layout),
            codes::INVALID_CONFIG,
            "status device without a data device",
        );
        assert_config_failure(
            &status_backend(1, vec![listening(), listening()], None, layout),
            codes::INVALID_CONFIG,
            "duplicate status channel",
        );
        assert_config_failure(
            &status_backend(
                1,
                vec![backend_channel(
                    "server0.listening",
                    "input",
                    1,
                    Some(at(2, 0, 1)),
                )],
                None,
                layout,
            ),
            codes::INVALID_CONFIG,
            "status channel must bind %I",
        );
        assert_config_failure(
            &status_backend(
                1,
                vec![backend_channel(
                    "server0.listening",
                    "input",
                    1,
                    Some(at(1, 0, 2)),
                )],
                None,
                layout,
            ),
            codes::CHANNEL_RESOLVE,
            "status image width differs from the channel",
        );
        assert_config_failure(
            &status_backend(
                1,
                vec![backend_channel(
                    "server0.listeners",
                    "input",
                    16,
                    Some(at(1, 4, 16)),
                )],
                None,
                layout,
            ),
            codes::CHANNEL_RESOLVE,
            "status image range is not byte aligned",
        );
        assert_config_failure(
            &status_backend(
                1,
                vec![status_channel("server0.listeners", 16, 56)],
                None,
                layout,
            ),
            codes::CHANNEL_RESOLVE,
            "status image range leaves the image",
        );

        let channels = vec![listening()];
        let mut config = status_backend(1, channels.clone(), None, layout);
        config.bindings[1].image.width_bits = 2;
        assert_config_failure(
            &config,
            codes::CHANNEL_RESOLVE,
            "binding image disagrees with the status channel",
        );

        let mut config = status_backend(1, channels.clone(), None, layout);
        config.bindings[1].direction = "output".into();
        assert_config_failure(
            &config,
            codes::CHANNEL_RESOLVE,
            "binding direction disagrees with the status channel",
        );

        let mut config = status_backend(1, channels.clone(), None, layout);
        config.bindings.push(BackendBinding {
            device: "status".into(),
            channel: "server0.requests".into(),
            image: at(1, 8, 32),
            direction: "input".into(),
        });
        assert_config_failure(
            &config,
            codes::CHANNEL_RESOLVE,
            "binding names an undeclared status channel",
        );

        let mut config = status_backend(1, channels, None, layout);
        let duplicate = config.bindings[1].clone();
        config.bindings.push(duplicate);
        assert_config_failure(&config, codes::CHANNEL_RESOLVE, "duplicate status binding");
    }

    fn server_config_text(binds: &str, extra: &str, channels: &str) -> String {
        format!("{{\"bind\":{binds}{extra},\"channels\":{channels}}}")
    }

    fn server_validation_channels() -> Vec<BackendChannel> {
        vec![input_channel("in0", 0, 16)]
    }

    fn server_config_failure(config: &str, channels: Vec<BackendChannel>, label: &str) {
        let config = raw_config(config, channels);
        assert_config_failure(&config, codes::INVALID_CONFIG, label);
    }

    #[test]
    fn server_configuration_validation_fails_closed() {
        let channels = || server_validation_channels();
        let mapping = r#"[{"name":"in0","table":"holding-register","address":0}]"#;
        server_config_failure("{}", channels(), "missing bind");
        server_config_failure("[]", channels(), "config must be an object");
        server_config_failure("{", channels(), "config must be JSON");
        server_config_failure(
            &server_config_text("[]", "", mapping),
            channels(),
            "empty bind array",
        );
        server_config_failure(
            &server_config_text("\"127.0.0.1:0\"", "", mapping),
            channels(),
            "bind must be an array",
        );
        server_config_failure(
            &server_config_text("[1]", "", mapping),
            channels(),
            "bind entries must be strings",
        );
        server_config_failure(
            &server_config_text("[\"\"]", "", mapping),
            channels(),
            "empty bind entry",
        );
        server_config_failure(
            &server_config_text("[\"localhost:502\"]", "", mapping),
            channels(),
            "bind must be a literal address",
        );
        for key in ["syncs", "pdo-channels", "dc"] {
            server_config_failure(
                &server_config_text("[\"127.0.0.1:0\"]", &format!(",\"{key}\":[]"), mapping),
                channels(),
                "EtherCAT key on a Modbus device",
            );
        }
        for unit in ["0", "248", "\"7\""] {
            server_config_failure(
                &server_config_text("[\"127.0.0.1:0\"]", &format!(",\"unit\":{unit}"), mapping),
                channels(),
                "invalid unit",
            );
        }
        for limit in ["0", "129", "\"4\""] {
            server_config_failure(
                &server_config_text(
                    "[\"127.0.0.1:0\"]",
                    &format!(",\"max-connections\":{limit}"),
                    mapping,
                ),
                channels(),
                "invalid max-connections",
            );
        }
        for timeout in ["\"0s\"", "\"soon\"", "5"] {
            server_config_failure(
                &server_config_text(
                    "[\"127.0.0.1:0\"]",
                    &format!(",\"idle-timeout\":{timeout}"),
                    mapping,
                ),
                channels(),
                "invalid idle-timeout",
            );
        }
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                ",\"word-order\":\"middle-first\"",
                mapping,
            ),
            channels(),
            "invalid device word order",
        );
        server_config_failure(
            &server_config_text("[\"127.0.0.1:0\"]", "", "{}"),
            channels(),
            "channels must be an array",
        );
        server_config_failure(
            &server_config_text("[\"127.0.0.1:0\"]", "", "[1]"),
            channels(),
            "channel entries must be tables",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"table":"holding-register","address":0}]"#,
            ),
            channels(),
            "missing channel name",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"in0","table":"register","address":0}]"#,
            ),
            channels(),
            "unknown table",
        );
        for address in ["65536", "\"0\"", "-1"] {
            server_config_failure(
                &server_config_text(
                    "[\"127.0.0.1:0\"]",
                    "",
                    &format!(
                        "[{{\"name\":\"in0\",\"table\":\"holding-register\",\"address\":{address}}}]"
                    ),
                ),
                channels(),
                "invalid address",
            );
        }
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"in0","table":"holding-register","address":0,"byte":"middle"}]"#,
            ),
            channels(),
            "invalid byte placement",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"in0","table":"holding-register","address":0,"word-order":"sideways"}]"#,
            ),
            channels(),
            "invalid channel word order",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"in0","table":"holding-register","address":0,"word-order":5}]"#,
            ),
            channels(),
            "channel word order must be a spelling or list",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"in0","table":"holding-register","address":0},
                    {"name":"in0","table":"holding-register","address":1}]"#,
            ),
            channels(),
            "duplicate channel map entry",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"ghost","table":"holding-register","address":0}]"#,
            ),
            channels(),
            "channel map entry must be declared",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"in0","table":"holding-register","address":0}]"#,
            ),
            vec![input_channel("in0", 0, 16), input_channel("in1", 16, 16)],
            "declared channel without a mapping",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"in0","table":"holding-register","address":0},
                    {"name":"in1","table":"holding-register","address":0}]"#,
            ),
            vec![input_channel("in0", 0, 16), input_channel("in1", 16, 16)],
            "overlapping register spans",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"c0","table":"coil","address":0},
                    {"name":"c1","table":"coil","address":0}]"#,
            ),
            vec![
                backend_channel("c0", "input", 1, Some(at(1, 0, 1))),
                backend_channel("c1", "input", 1, Some(at(1, 1, 1))),
            ],
            "overlapping coil channels",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"in0","table":"input-register","address":0}]"#,
            ),
            channels(),
            "input channel on a read-only table",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"c0","table":"coil","address":0}]"#,
            ),
            vec![backend_channel("c0", "input", 16, Some(at(1, 0, 16)))],
            "coil channel must be one bit",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"c0","table":"coil","address":0,"byte":"low"}]"#,
            ),
            vec![backend_channel("c0", "input", 1, Some(at(1, 0, 1)))],
            "coil channel cannot use byte placement",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"in0","table":"holding-register","address":0,"byte":"low"}]"#,
            ),
            channels(),
            "byte placement is only for 8-bit channels",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"in0","table":"holding-register","address":0,"word-order":"cdab"}]"#,
            ),
            channels(),
            "word order is only for 32/64-bit channels",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"in0","table":"holding-register","address":0}]"#,
            ),
            vec![input_channel("in0", 0, 12)],
            "unsupported channel width",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"in32","table":"holding-register","address":65535}]"#,
            ),
            vec![input_channel("in32", 0, 32)],
            "span leaves the 16-bit address space",
        );
        server_config_failure(
            &server_config_text(
                "[\"127.0.0.1:0\"]",
                "",
                r#"[{"name":"in0","table":"holding-register","address":0}]"#,
            ),
            vec![backend_channel("in0", "sideways", 16, Some(at(1, 0, 16)))],
            "declared direction must be input or output",
        );

        // Duplicate concrete bind entries are rejected; port 0 entries are
        // allowed and bind distinct ports.
        assert_config_failure(
            &raw_config(
                &server_config_text("[\"127.0.0.1:1502\",\"127.0.0.1:1502\"]", "", mapping),
                channels(),
            ),
            codes::INVALID_CONFIG,
            "duplicate concrete bind address",
        );

        // Image-resolution failures use the channel/image code.
        let mut config = raw_config(
            &server_config_text("[\"127.0.0.1:0\"]", "", mapping),
            channels(),
        );
        config.devices[0].channels[0].image = Some(at(2, 0, 16));
        assert_config_failure(
            &config,
            codes::CHANNEL_RESOLVE,
            "input channel bound to an output image range",
        );
        let mut config = raw_config(
            &server_config_text("[\"127.0.0.1:0\"]", "", mapping),
            channels(),
        );
        config.devices[0].channels[0].image = Some(at(1, 0, 8));
        assert_config_failure(
            &config,
            codes::CHANNEL_RESOLVE,
            "image width differs from the channel",
        );
        let mut config = raw_config(
            &server_config_text("[\"127.0.0.1:0\"]", "", mapping),
            channels(),
        );
        config.devices[0].channels[0].image = Some(at(1, 24, 16));
        assert_config_failure(
            &config,
            codes::CHANNEL_RESOLVE,
            "image range leaves the image",
        );
    }

    const HASH_CONFIG: &str = concat!(
        r#"{"bind":["127.0.0.1:1502"],"unit":7,"word-order":"high-first","#,
        r#""max-connections":8,"idle-timeout":"60s","channels":["#,
        r#"{"name":"in0","table":"holding-register","address":0},"#,
        r#"{"name":"in8","table":"holding-register","address":1,"byte":"low"},"#,
        r#"{"name":"in32","table":"holding-register","address":2},"#,
        r#"{"name":"coil_in","table":"coil","address":0}"#,
        r#"]}"#
    );

    fn hash_info(config: Option<&str>) -> ModuleInfo {
        ModuleInfo {
            devices: vec![DeviceInfo {
                name: "mb".into(),
                kind: "modbus-tcp".into(),
                config: config.map(str::to_owned),
                channels: vec![
                    ChannelInfo {
                        name: "in0".into(),
                        direction: "input".into(),
                        width_bits: 16,
                    },
                    ChannelInfo {
                        name: "in8".into(),
                        direction: "input".into(),
                        width_bits: 8,
                    },
                    ChannelInfo {
                        name: "in32".into(),
                        direction: "input".into(),
                        width_bits: 32,
                    },
                    ChannelInfo {
                        name: "coil_in".into(),
                        direction: "input".into(),
                        width_bits: 1,
                    },
                ],
            }],
            bindings: vec![DirectBinding {
                got_id: GotId::from_bytes([0x11; 16]),
                area: 1,
                offset_bits: 0,
                width_bits: 16,
                device: Some("mb".into()),
                channel: Some(BindingChannel::Name("in0".into())),
                direction: Some("input".into()),
            }],
            ..ModuleInfo::default()
        }
    }

    fn hash_with(config: &str) -> String {
        configuration_hash(Some(&hash_info(Some(config))))
    }

    #[test]
    fn configuration_hash_covers_the_modbus_configuration() {
        let info = hash_info(Some(HASH_CONFIG));
        let base = configuration_hash(Some(&info));
        assert_eq!(
            base,
            configuration_hash(Some(&hash_info(Some(HASH_CONFIG)))),
            "the hash is deterministic"
        );
        assert_eq!(
            configuration_hash(None),
            configuration_hash(None),
            "an absent module has a stable hash"
        );

        // Every detail of the device configuration text takes part.
        let config_mutations = [
            (
                "bind port",
                HASH_CONFIG.replace("127.0.0.1:1502", "127.0.0.1:1503"),
            ),
            (
                "bind array",
                HASH_CONFIG.replace(
                    "\"127.0.0.1:1502\"",
                    "\"127.0.0.1:1502\",\"127.0.0.1:1504\"",
                ),
            ),
            ("unit", HASH_CONFIG.replace("\"unit\":7", "\"unit\":8")),
            (
                "device word order",
                HASH_CONFIG.replace("high-first", "low-first"),
            ),
            (
                "byte placement",
                HASH_CONFIG.replace("\"byte\":\"low\"", "\"byte\":\"high\""),
            ),
            (
                "table",
                HASH_CONFIG.replace(
                    "{\"name\":\"in0\",\"table\":\"holding-register\"",
                    "{\"name\":\"in0\",\"table\":\"input-register\"",
                ),
            ),
            (
                "address",
                HASH_CONFIG.replace(
                    "{\"name\":\"in32\",\"table\":\"holding-register\",\"address\":2}",
                    "{\"name\":\"in32\",\"table\":\"holding-register\",\"address\":4}",
                ),
            ),
            (
                "per-channel word order",
                HASH_CONFIG.replace(
                    "{\"name\":\"in32\",\"table\":\"holding-register\",\"address\":2}",
                    "{\"name\":\"in32\",\"table\":\"holding-register\",\"address\":2,\
                     \"word-order\":\"cdab\"}",
                ),
            ),
            (
                "max connections",
                HASH_CONFIG.replace("\"max-connections\":8", "\"max-connections\":16"),
            ),
            (
                "idle timeout",
                HASH_CONFIG.replace("\"idle-timeout\":\"60s\"", "\"idle-timeout\":\"5s\""),
            ),
        ];
        for (label, text) in config_mutations {
            assert_ne!(
                hash_with(&text),
                base,
                "changing `{label}` must change the configuration hash"
            );
        }

        let assert_changed = |label: &str, changed: ModuleInfo| {
            assert_ne!(
                configuration_hash(Some(&changed)),
                base,
                "changing `{label}` must change the configuration hash"
            );
        };

        let mut changed = info.clone();
        changed.devices[0].name = "mb-renamed".into();
        assert_changed("device name", changed);
        let mut changed = info.clone();
        changed.devices[0].kind = "modbus-tcp-status".into();
        assert_changed("device kind", changed);
        let mut changed = info.clone();
        changed.devices[0].config = None;
        assert_changed("device configuration presence", changed);
        let mut changed = info.clone();
        changed.devices[0].channels[0].name = "renamed".into();
        assert_changed("channel name", changed);
        let mut changed = info.clone();
        changed.devices[0].channels[0].direction = "output".into();
        assert_changed("channel direction", changed);
        let mut changed = info.clone();
        changed.devices[0].channels[2].width_bits = 64;
        assert_changed("channel width and register span", changed);

        let mut changed = info.clone();
        changed.profile = Some(ProfileInfo {
            path: "profiles/machine.toml".into(),
            text: "[[io.devices]]".into(),
            digest: "0x1234".into(),
            devices: vec!["mb".into()],
        });
        assert_changed("profile digest", changed);

        let mut changed = info.clone();
        changed.bindings[0].got_id = GotId::from_bytes([0x22; 16]);
        assert_changed("binding identity", changed);
        let mut changed = info.clone();
        changed.bindings[0].area = 2;
        assert_changed("binding area", changed);
        let mut changed = info.clone();
        changed.bindings[0].offset_bits = 8;
        assert_changed("binding offset", changed);
        let mut changed = info.clone();
        changed.bindings[0].width_bits = 8;
        assert_changed("binding width", changed);
        let mut changed = info.clone();
        changed.bindings[0].device = Some("mb-other".into());
        assert_changed("binding device", changed);
        let mut changed = info.clone();
        changed.bindings[0].channel = Some(BindingChannel::Name("in8".into()));
        assert_changed("binding channel name", changed);
        let mut changed = info.clone();
        changed.bindings[0].channel = Some(BindingChannel::Index(0));
        assert_changed("binding channel reference form", changed);
        let mut changed = info.clone();
        changed.bindings[0].direction = Some("output".into());
        assert_changed("binding direction", changed);

        // Metadata text that does not describe the I/O configuration is not
        // part of the hash; this is what lets a code-only swap stay online.
        let mut unrelated = info.clone();
        unrelated.text = "{\"code\":\"new build\"}".into();
        unrelated.debug = Some(Json::Null);
        assert_eq!(configuration_hash(Some(&unrelated)), base);
    }
}
