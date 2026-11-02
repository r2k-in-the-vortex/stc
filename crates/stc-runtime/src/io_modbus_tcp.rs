//! Modbus TCP protocol core and register bank (WL-5.1 phase 1).
//!
//! This module implements the wire codec, function-code handling, channel map,
//! and register bank for the `modbus-tcp` device kind. It deliberately owns no
//! sockets, threads, or coordinator types: phase 2 wraps [`RegisterBank`] in a
//! mutex and adapts [`ChannelMap`] channels to `BackendChannel`/`ImageRange`.
//!
//! The locked contract is WL-5.1-Plan.md §5 (M-2, M-3, M-6, M-7) and
//! §6.2–§6.5. Addresses are zero-based: address 0 is the first element of every
//! table (vendor reference `00001`/`10001`/`30001`/`40001`), and the configured
//! address is written into the PDU without an off-by-one conversion.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

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
}
