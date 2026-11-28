//! Object dictionary and typed SDO values (WL-7 phase 4, D-272).
//!
//! The dictionary merges the ESI-declared metadata with the online CoE SDO
//! Information the connected device reports. Authority is field-level
//! (D-272): the ESI is the offline baseline and the sole PDO
//! mapping-eligibility source, while online metadata is the live authority for
//! the connected device's existence, name, type, access and description.
//! Online-only objects stay inspectable and explicitly readable/writable in
//! setup mode, but they are never PDO-mappable and never become PLC channels.
//!
//! Conflicts are surfaced, not silently overwritten. The dictionary records
//! both values and keeps the online value in the entry, because the connected
//! device is authoritative for its own live metadata.

use std::collections::BTreeMap;

use stc_got::json::Json;

use super::abi::{OnlineObject, OperationResult, ResultCategory};
use super::diagnostics::{EngineeringDiagnostic, codes};
use super::esi::model::ObjectEntry as EsiObject;

/// Address of one object/subindex on one configured device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectAddress {
    /// Configured machine slot.
    pub position: u16,
    /// Object index.
    pub index: u16,
    /// Object subindex.
    pub subindex: u8,
}

impl ObjectAddress {
    /// Creates an address.
    #[must_use]
    pub fn new(position: u16, index: u16, subindex: u8) -> Self {
        Self {
            position,
            index,
            subindex,
        }
    }
}

/// Which source supplied an entry's metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataSource {
    /// Declared by the accepted ESI description.
    Esi,
    /// Reported by online CoE SDO Information.
    Online,
}

impl MetadataSource {
    /// Stable spelling used in structured output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Esi => "esi",
            Self::Online => "online",
        }
    }
}

/// One metadata field where the ESI and the device disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldConflict {
    /// Field name (`name`, `type` or `access`).
    pub field: &'static str,
    /// Value the ESI declares.
    pub esi: String,
    /// Value the connected device reports.
    pub online: String,
}

impl FieldConflict {
    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("field".into(), Json::string(self.field)),
            ("esi".into(), Json::string(self.esi.as_str())),
            ("online".into(), Json::string(self.online.as_str())),
        ])
    }
}

/// One merged object/subindex.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectEntry {
    /// Object index.
    pub index: u16,
    /// Object or sub-item subindex.
    pub subindex: u8,
    /// Effective name.
    pub name: Option<String>,
    /// Effective data-type name.
    pub data_type: Option<String>,
    /// Effective access string.
    pub access: Option<String>,
    /// Description reported online, when available.
    pub description: Option<String>,
    /// Source of the effective metadata.
    pub source: MetadataSource,
    /// Whether the object exists only online. Online-only objects are never
    /// PDO-mappable and never auto-exposed as PLC channels (D-272).
    pub online_only: bool,
    /// Field-level disagreements between the ESI and the device.
    pub conflicts: Vec<FieldConflict>,
}

impl ObjectEntry {
    /// Stable object/subindex spelling.
    #[must_use]
    pub fn describe(&self) -> String {
        format!("0x{:04x}:{}", self.index, self.subindex)
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("index".into(), Json::unsigned(u64::from(self.index))),
            (
                "index_hex".into(),
                Json::string(format!("0x{:04x}", self.index)),
            ),
            ("subindex".into(), Json::unsigned(u64::from(self.subindex))),
            (
                "name".into(),
                Json::optional(self.name.as_ref().map(|value| Json::string(value.as_str()))),
            ),
            (
                "type".into(),
                Json::optional(
                    self.data_type
                        .as_ref()
                        .map(|value| Json::string(value.as_str())),
                ),
            ),
            (
                "access".into(),
                Json::optional(
                    self.access
                        .as_ref()
                        .map(|value| Json::string(value.as_str())),
                ),
            ),
            (
                "description".into(),
                Json::optional(
                    self.description
                        .as_ref()
                        .map(|value| Json::string(value.as_str())),
                ),
            ),
            ("source".into(), Json::string(self.source.as_str())),
            ("online_only".into(), Json::Bool(self.online_only)),
            (
                "conflicts".into(),
                Json::Array(self.conflicts.iter().map(FieldConflict::to_json).collect()),
            ),
        ])
    }
}

/// The merged object dictionary of one position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectDictionary {
    /// Configured machine slot.
    pub position: u16,
    /// Entries in deterministic index/subindex order.
    pub objects: Vec<ObjectEntry>,
    /// Reported capability gaps, such as unavailable SDO Information.
    pub warnings: Vec<EngineeringDiagnostic>,
}

impl ObjectDictionary {
    /// Merges ESI metadata with online metadata (D-272).
    ///
    /// Field-level authority: online values win where the device reports them,
    /// differing ESI values are retained as conflicts, and ESI values fill the
    /// gaps the device does not report. Online-only entries are marked so no
    /// caller can treat them as PDO-mappable.
    #[must_use]
    pub fn merge(
        position: u16,
        esi: &[EsiObject],
        online: Option<&[OnlineObject]>,
        warning: Option<EngineeringDiagnostic>,
    ) -> Self {
        let mut merged: BTreeMap<(u16, u8), ObjectEntry> = BTreeMap::new();
        for object in esi {
            merged.insert(
                (object.index, object.subindex),
                ObjectEntry {
                    index: object.index,
                    subindex: object.subindex,
                    name: object.name.clone(),
                    data_type: object.data_type.clone(),
                    access: object.access.clone(),
                    description: None,
                    source: MetadataSource::Esi,
                    online_only: false,
                    conflicts: Vec::new(),
                },
            );
        }
        for object in online.unwrap_or_default() {
            match merged.get_mut(&(object.index, object.subindex)) {
                Some(entry) => {
                    entry.source = MetadataSource::Online;
                    merge_field(&mut entry.name, &object.name, "name", &mut entry.conflicts);
                    merge_field(
                        &mut entry.data_type,
                        &object.data_type,
                        "type",
                        &mut entry.conflicts,
                    );
                    merge_field(
                        &mut entry.access,
                        &object.access,
                        "access",
                        &mut entry.conflicts,
                    );
                    if object.description.is_some() {
                        entry.description.clone_from(&object.description);
                    }
                }
                None => {
                    merged.insert(
                        (object.index, object.subindex),
                        ObjectEntry {
                            index: object.index,
                            subindex: object.subindex,
                            name: object.name.clone(),
                            data_type: object.data_type.clone(),
                            access: object.access.clone(),
                            description: object.description.clone(),
                            source: MetadataSource::Online,
                            online_only: true,
                            conflicts: Vec::new(),
                        },
                    );
                }
            }
        }
        Self {
            position,
            objects: merged.into_values().collect(),
            warnings: warning.into_iter().collect(),
        }
    }

    /// Finds one object/subindex.
    #[must_use]
    pub fn object(&self, index: u16, subindex: u8) -> Option<&ObjectEntry> {
        self.objects
            .iter()
            .find(|object| object.index == index && object.subindex == subindex)
    }

    /// All subindices of one object, in subindex order.
    #[must_use]
    pub fn object_at(&self, index: u16) -> Vec<&ObjectEntry> {
        self.objects
            .iter()
            .filter(|object| object.index == index)
            .collect()
    }

    /// Whether the dictionary carries no metadata at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("position".into(), Json::unsigned(u64::from(self.position))),
            (
                "objects".into(),
                Json::Array(self.objects.iter().map(ObjectEntry::to_json).collect()),
            ),
            (
                "warnings".into(),
                Json::Array(
                    self.warnings
                        .iter()
                        .map(EngineeringDiagnostic::to_json)
                        .collect(),
                ),
            ),
        ])
    }
}

fn merge_field(
    target: &mut Option<String>,
    online: &Option<String>,
    field: &'static str,
    conflicts: &mut Vec<FieldConflict>,
) {
    let Some(online) = online else {
        return;
    };
    if let Some(esi) = target
        && esi != online
    {
        conflicts.push(FieldConflict {
            field,
            esi: esi.clone(),
            online: online.clone(),
        });
    }
    *target = Some(online.clone());
}

/// Access rights of one object/subindex.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Read-only.
    ReadOnly,
    /// Write-only.
    WriteOnly,
    /// Readable and writable.
    ReadWrite,
    /// Declared access is absent or not understood.
    Unknown,
}

impl Access {
    /// Parses the ESI/CoE access spelling.
    #[must_use]
    pub fn parse(text: Option<&str>) -> Self {
        let Some(text) = text else {
            return Self::Unknown;
        };
        let text = text.trim().to_ascii_lowercase();
        if text.starts_with("rw") {
            Self::ReadWrite
        } else if text.starts_with("ro") || text == "r" {
            Self::ReadOnly
        } else if text.starts_with("wo") || text == "w" {
            Self::WriteOnly
        } else {
            Self::Unknown
        }
    }

    /// Whether a read is permitted.
    #[must_use]
    pub fn allows_read(self) -> bool {
        !matches!(self, Self::WriteOnly)
    }

    /// Whether a write is permitted.
    ///
    /// Unknown access fails closed: a commissioning write is never attempted
    /// against metadata that does not state it is writable.
    #[must_use]
    pub fn allows_write(self) -> bool {
        matches!(self, Self::ReadWrite)
    }
}

/// A CoE data type used to encode and decode SDO values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdoType {
    /// Boolean.
    Bool,
    /// Signed 8-bit integer.
    I8,
    /// Signed 16-bit integer.
    I16,
    /// Signed 32-bit integer.
    I32,
    /// Signed 64-bit integer.
    I64,
    /// Unsigned 8-bit integer.
    U8,
    /// Unsigned 16-bit integer.
    U16,
    /// Unsigned 32-bit integer.
    U32,
    /// Unsigned 64-bit integer.
    U64,
    /// 32-bit IEEE-754 float.
    F32,
    /// 64-bit IEEE-754 float.
    F64,
    /// Visible or UTF-8 string.
    String,
    /// Raw octets.
    Octets,
}

impl SdoType {
    /// Maps an ESI/CoE data-type name.
    ///
    /// Returns `None` for record types (such as `IDENTITY`) and for names the
    /// accepted subset does not model; callers keep such values as raw bytes
    /// instead of guessing a width.
    #[must_use]
    pub fn from_declaration(text: Option<&str>) -> Option<Self> {
        let text = text?.trim().to_ascii_uppercase();
        match text.as_str() {
            "BOOL" | "BOOLEAN" => Some(Self::Bool),
            "SINT" | "INT8" => Some(Self::I8),
            "INT" | "INT16" => Some(Self::I16),
            "DINT" | "INT32" => Some(Self::I32),
            "LINT" | "INT64" => Some(Self::I64),
            "USINT" | "UINT8" => Some(Self::U8),
            "UINT" | "UINT16" => Some(Self::U16),
            "UDINT" | "UINT32" => Some(Self::U32),
            "ULINT" | "UINT64" => Some(Self::U64),
            "REAL" | "FLOAT" | "FLOAT32" => Some(Self::F32),
            "LREAL" | "DOUBLE" | "FLOAT64" => Some(Self::F64),
            "STRING" | "VISIBLE_STRING" | "VISIBLESTRING" => Some(Self::String),
            "OCTET_STRING" | "OCTETSTRING" | "BYTES" => Some(Self::Octets),
            _ => None,
        }
    }

    /// Bit width of a fixed-width type.
    #[must_use]
    pub fn bit_len(self) -> Option<u8> {
        match self {
            Self::Bool => Some(1),
            Self::I8 | Self::U8 => Some(8),
            Self::I16 | Self::U16 => Some(16),
            Self::I32 | Self::U32 | Self::F32 => Some(32),
            Self::I64 | Self::U64 | Self::F64 => Some(64),
            Self::String | Self::Octets => None,
        }
    }

    /// Byte width of a fixed-width type.
    #[must_use]
    pub fn width(self) -> Option<usize> {
        match self {
            Self::Bool => Some(1),
            Self::I8 | Self::U8 => Some(1),
            Self::I16 | Self::U16 => Some(2),
            Self::I32 | Self::U32 | Self::F32 => Some(4),
            Self::I64 | Self::U64 | Self::F64 => Some(8),
            Self::String | Self::Octets => None,
        }
    }

    /// Stable spelling used in structured output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::String => "string",
            Self::Octets => "octets",
        }
    }
}

/// A typed commissioning value.
#[derive(Clone, Debug, PartialEq)]
pub enum SdoData {
    /// Boolean.
    Bool(bool),
    /// Unsigned integer.
    Unsigned(u64),
    /// Signed integer.
    Signed(i64),
    /// Floating-point value.
    Float(f64),
    /// Text.
    Text(String),
    /// Raw octets.
    Bytes(Vec<u8>),
}

impl SdoData {
    /// Parses a command-line literal for `ty`.
    ///
    /// # Errors
    ///
    /// Returns a human-readable reason the literal does not fit the type; the
    /// caller maps it to `STC-E-IO-ETH-035`.
    pub fn parse(text: &str, ty: Option<SdoType>) -> Result<Self, String> {
        let Some(ty) = ty else {
            return parse_bytes(text).map(Self::Bytes).ok_or_else(|| {
                "the object has no scalar type; pass wire bytes such as `0x0102`".into()
            });
        };
        match ty {
            SdoType::Bool => match text.trim().to_ascii_lowercase().as_str() {
                "true" | "1" => Ok(Self::Bool(true)),
                "false" | "0" => Ok(Self::Bool(false)),
                other => Err(format!("`{other}` is not a boolean (use `true`/`false`)")),
            },
            SdoType::I8 | SdoType::I16 | SdoType::I32 | SdoType::I64 => {
                let value = parse_signed(text)?;
                let width = ty.width().expect("fixed width");
                let min = -(1i128 << (width * 8 - 1));
                let max = (1i128 << (width * 8 - 1)) - 1;
                if i128::from(value) < min || i128::from(value) > max {
                    return Err(format!("`{text}` does not fit {}", ty.as_str()));
                }
                Ok(Self::Signed(value))
            }
            SdoType::U8 | SdoType::U16 | SdoType::U32 | SdoType::U64 => {
                let value = parse_unsigned(text)?;
                let width = ty.width().expect("fixed width");
                let max = (1u128 << (width * 8)) - 1;
                if u128::from(value) > max {
                    return Err(format!("`{text}` does not fit {}", ty.as_str()));
                }
                Ok(Self::Unsigned(value))
            }
            SdoType::F32 | SdoType::F64 => text
                .trim()
                .parse::<f64>()
                .map(Self::Float)
                .map_err(|_| format!("`{text}` is not a number")),
            SdoType::String => Ok(Self::Text(text.to_owned())),
            SdoType::Octets => parse_bytes(text)
                .map(Self::Bytes)
                .ok_or_else(|| format!("`{text}` is not a byte string (use `0x0102` or `1,2`)")),
        }
    }

    /// Encodes the value as CoE wire bytes for `ty`.
    ///
    /// # Errors
    ///
    /// Returns a reason when the value does not match or fit the declared type.
    pub fn encode(&self, ty: Option<SdoType>) -> Result<Vec<u8>, String> {
        let Some(ty) = ty else {
            return match self {
                Self::Bytes(bytes) => Ok(bytes.clone()),
                _ => Err("the object has no scalar type; pass wire bytes".into()),
            };
        };
        match (ty, self) {
            (SdoType::Bool, Self::Bool(value)) => Ok(vec![u8::from(*value)]),
            (SdoType::I8, Self::Signed(value)) => signed_bytes(*value, 1),
            (SdoType::I16, Self::Signed(value)) => signed_bytes(*value, 2),
            (SdoType::I32, Self::Signed(value)) => signed_bytes(*value, 4),
            (SdoType::I64, Self::Signed(value)) => signed_bytes(*value, 8),
            (SdoType::U8, Self::Unsigned(value)) => unsigned_bytes(*value, 1),
            (SdoType::U16, Self::Unsigned(value)) => unsigned_bytes(*value, 2),
            (SdoType::U32, Self::Unsigned(value)) => unsigned_bytes(*value, 4),
            (SdoType::U64, Self::Unsigned(value)) => unsigned_bytes(*value, 8),
            (SdoType::F32, Self::Float(value)) => {
                Ok((*value as f32).to_bits().to_le_bytes().to_vec())
            }
            (SdoType::F64, Self::Float(value)) => Ok(value.to_bits().to_le_bytes().to_vec()),
            (SdoType::String, Self::Text(value)) => Ok(value.as_bytes().to_vec()),
            (SdoType::Octets, Self::Bytes(value)) => Ok(value.clone()),
            (declared, value) => Err(format!(
                "{} value cannot be written to a {} object",
                value.describe(),
                declared.as_str()
            )),
        }
    }

    /// Decodes CoE wire bytes for `ty`.
    ///
    /// # Errors
    ///
    /// Returns a reason when the byte length does not match the declared type.
    pub fn decode(bytes: &[u8], ty: Option<SdoType>) -> Result<Self, String> {
        let Some(ty) = ty else {
            return Ok(Self::Bytes(bytes.to_vec()));
        };
        if let Some(width) = ty.width()
            && bytes.len() != width
        {
            return Err(format!(
                "a {} value is {width} bytes, the device returned {}",
                ty.as_str(),
                bytes.len()
            ));
        }
        match ty {
            SdoType::Bool => Ok(Self::Bool(bytes.first().copied().unwrap_or(0) != 0)),
            SdoType::I8 => Ok(Self::Signed(i64::from(bytes[0] as i8))),
            SdoType::I16 => Ok(Self::Signed(i64::from(i16::from_le_bytes([
                bytes[0], bytes[1],
            ])))),
            SdoType::I32 => Ok(Self::Signed(i64::from(i32::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3],
            ])))),
            SdoType::I64 => Ok(Self::Signed(i64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]))),
            SdoType::U8 => Ok(Self::Unsigned(u64::from(bytes[0]))),
            SdoType::U16 => Ok(Self::Unsigned(u64::from(u16::from_le_bytes([
                bytes[0], bytes[1],
            ])))),
            SdoType::U32 => Ok(Self::Unsigned(u64::from(u32::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3],
            ])))),
            SdoType::U64 => Ok(Self::Unsigned(u64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]))),
            SdoType::F32 => Ok(Self::Float(f64::from(f32::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3],
            ])))),
            SdoType::F64 => Ok(Self::Float(f64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]))),
            SdoType::String => {
                let end = bytes
                    .iter()
                    .rposition(|byte| *byte != 0)
                    .map_or(0, |position| position + 1);
                Ok(Self::Text(
                    String::from_utf8_lossy(&bytes[..end]).into_owned(),
                ))
            }
            SdoType::Octets => Ok(Self::Bytes(bytes.to_vec())),
        }
    }

    /// Stable value-kind spelling for diagnostics.
    #[must_use]
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Bool(_) => "bool",
            Self::Unsigned(_) => "unsigned",
            Self::Signed(_) => "signed",
            Self::Float(_) => "float",
            Self::Text(_) => "string",
            Self::Bytes(_) => "bytes",
        }
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        let value = match self {
            Self::Bool(value) => Json::Bool(*value),
            Self::Unsigned(value) => Json::unsigned(*value),
            Self::Signed(value) => Json::signed(*value),
            Self::Float(value) => Json::string(format!("{value}")),
            Self::Text(value) => Json::string(value.as_str()),
            Self::Bytes(bytes) => Json::Array(
                bytes
                    .iter()
                    .map(|byte| Json::unsigned(u64::from(*byte)))
                    .collect(),
            ),
        };
        Json::object([
            ("type".into(), Json::string(self.describe())),
            ("value".into(), value),
        ])
    }
}

fn signed_bytes(value: i64, width: usize) -> Result<Vec<u8>, String> {
    let bits = width * 8;
    let min = -(1i128 << (bits - 1));
    let max = (1i128 << (bits - 1)) - 1;
    if i128::from(value) < min || i128::from(value) > max {
        return Err(format!("{value} does not fit a signed {bits}-bit value"));
    }
    Ok(value.to_le_bytes()[..width].to_vec())
}

fn unsigned_bytes(value: u64, width: usize) -> Result<Vec<u8>, String> {
    let bits = width * 8;
    if u128::from(value) > (1u128 << bits) - 1 {
        return Err(format!("{value} does not fit an unsigned {bits}-bit value"));
    }
    Ok(value.to_le_bytes()[..width].to_vec())
}

fn parse_unsigned(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let (digits, radix) = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .or_else(|| text.strip_prefix("#x"))
        .map_or((text, 10), |digits| (digits, 16));
    u64::from_str_radix(digits, radix).map_err(|_| format!("`{text}` is not an unsigned integer"))
}

fn parse_signed(text: &str) -> Result<i64, String> {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix('-') {
        let value = parse_unsigned(rest)?;
        return i64::try_from(value)
            .map(|value| -value)
            .map_err(|_| format!("`{text}` is out of range"));
    }
    i64::try_from(parse_unsigned(text).map_err(|_| format!("`{text}` is not an integer"))?)
        .map_err(|_| format!("`{text}` is out of range"))
}

fn parse_bytes(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if let Some(rest) = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .or_else(|| text.strip_prefix("#x"))
    {
        let digits: String = rest
            .chars()
            .filter(|character| !character.is_whitespace() && *character != '_')
            .collect();
        if digits.is_empty() || !digits.len().is_multiple_of(2) {
            return None;
        }
        let mut bytes = Vec::with_capacity(digits.len() / 2);
        for pair in digits.as_bytes().chunks(2) {
            let pair = std::str::from_utf8(pair).ok()?;
            bytes.push(u8::from_str_radix(pair, 16).ok()?);
        }
        return Some(bytes);
    }
    let mut bytes = Vec::new();
    for item in text
        .split([',', ' '])
        .filter(|item| !item.trim().is_empty())
    {
        bytes.push(
            parse_unsigned(item)
                .ok()
                .and_then(|value| u8::try_from(value).ok())?,
        );
    }
    if bytes.is_empty() { None } else { Some(bytes) }
}

/// `STC-E-IO-ETH-030`: neither the ESI nor the device knows the object.
#[must_use]
pub fn object_unavailable(
    operation: &'static str,
    device: &str,
    position: u16,
    index: u16,
    subindex: u8,
) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::SDO_OBJECT_UNAVAILABLE,
        operation,
        "list the dictionary and use a declared or online-reported object",
    )
    .with_device(device)
    .with_position(position)
    .with_object(index, subindex)
    .with_expected("an ESI-declared or online-reported object")
    .with_observed(format!("no metadata for 0x{index:04x}:{subindex}"))
}

/// `STC-E-IO-ETH-034`: the declared access rights deny the operation.
#[must_use]
pub fn access_denied(
    operation: &'static str,
    device: &str,
    position: u16,
    index: u16,
    subindex: u8,
    access: Option<&str>,
) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::SDO_ACCESS_DENIED,
        operation,
        "inspect the object's access rights; a commissioning write never overrides them",
    )
    .with_device(device)
    .with_position(position)
    .with_object(index, subindex)
    .with_expected("read/write access")
    .with_observed(access.map_or_else(|| "no declared access".to_owned(), str::to_owned))
}

/// `STC-E-IO-ETH-035`: a typed value or read-back disagrees.
#[must_use]
pub fn value_error(
    operation: &'static str,
    device: &str,
    position: u16,
    index: u16,
    subindex: u8,
    expected: &str,
    observed: &str,
) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::SDO_VALUE,
        operation,
        "check the declared data type and the value; no partial write is claimed",
    )
    .with_device(device)
    .with_position(position)
    .with_object(index, subindex)
    .with_expected(expected)
    .with_observed(observed)
}

/// `STC-E-IO-ETH-036`: a read or required read-back is unavailable.
#[must_use]
pub fn read_back_unavailable(
    operation: &'static str,
    device: &str,
    position: u16,
    index: u16,
    subindex: u8,
) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::SDO_READ_BACK_UNAVAILABLE,
        operation,
        "the device or host does not provide the read-back; the check stays unverified",
    )
    .with_device(device)
    .with_position(position)
    .with_object(index, subindex)
    .with_expected("an SDO upload")
    .with_observed("the transfer returned no data")
}

/// `STC-W-IO-ETH-009`: SDO Information is unavailable; the ESI view remains.
#[must_use]
pub fn sdo_information_unavailable(
    operation: &'static str,
    position: u16,
) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::SDO_INFO_UNAVAILABLE,
        operation,
        "the ESI-only dictionary remains usable; online-only objects are not visible",
    )
    .with_position(position)
    .with_expected("online CoE SDO Information")
    .with_observed("the device or host reports none")
}

/// Maps a structured transfer result to the SDO diagnostic family.
#[must_use]
pub fn transfer_error(
    operation: &'static str,
    device: &str,
    position: u16,
    index: u16,
    subindex: u8,
    result: &OperationResult,
) -> EngineeringDiagnostic {
    let diagnostic = match result.category {
        ResultCategory::Abort => EngineeringDiagnostic::new(
            codes::SDO_ABORT,
            operation,
            "inspect the abort code and the object's state/precondition requirements",
        ),
        ResultCategory::Timeout => EngineeringDiagnostic::new(
            codes::SDO_TIMEOUT,
            operation,
            "check the mailbox and repeat the transfer; a timeout is never a success",
        ),
        ResultCategory::StateRefused | ResultCategory::NotActive => EngineeringDiagnostic::new(
            codes::SDO_STATE,
            operation,
            "enter setup mode and confirm the device's AL state before the transfer",
        ),
        ResultCategory::OwnershipConflict => EngineeringDiagnostic::new(
            codes::OWNERSHIP_CONFLICT,
            operation,
            "stop the owning runtime or leave the other engineering session",
        ),
        ResultCategory::IdentityMismatch => EngineeringDiagnostic::new(
            codes::ETH_SLAVE,
            operation,
            "inspect the live identity before configuring the device",
        ),
        ResultCategory::Unavailable | ResultCategory::RemapRefused | ResultCategory::Ok => {
            EngineeringDiagnostic::new(
                codes::SDO_READ_BACK_UNAVAILABLE,
                operation,
                "the device or host does not provide the transfer; the result stays unverified",
            )
        }
    };
    let mut diagnostic = diagnostic
        .with_device(device)
        .with_position(position)
        .with_object(index, subindex)
        .with_expected("a completed SDO transfer")
        .with_observed(format!("the shim reported `{}`", result.category.as_str()));
    if let Some(abort_code) = result.abort_code {
        diagnostic = diagnostic.with_abort(abort_code);
    }
    if let Some(al_state) = result.al_state {
        diagnostic = diagnostic.with_al_state(al_state);
    }
    diagnostic
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_rights_fail_closed_for_writes() {
        assert_eq!(Access::parse(Some("ro")), Access::ReadOnly);
        assert_eq!(Access::parse(Some("rw")), Access::ReadWrite);
        assert_eq!(Access::parse(Some("wo")), Access::WriteOnly);
        assert_eq!(Access::parse(Some("vendor")), Access::Unknown);
        assert!(!Access::Unknown.allows_write());
        assert!(Access::Unknown.allows_read());
        assert!(!Access::ReadOnly.allows_write());
    }

    #[test]
    fn values_round_trip_at_their_declared_width() {
        for (ty, text) in [
            (SdoType::Bool, "true"),
            (SdoType::I8, "-7"),
            (SdoType::I16, "-300"),
            (SdoType::I32, "-70000"),
            (SdoType::U8, "255"),
            (SdoType::U16, "65535"),
            (SdoType::U32, "0x1234"),
            (SdoType::U64, "9000000000"),
            (SdoType::F32, "1.5"),
            (SdoType::F64, "-2.25"),
            (SdoType::String, "hello"),
            (SdoType::Octets, "0x0102"),
        ] {
            let value = SdoData::parse(text, Some(ty)).expect("parse");
            let bytes = value.encode(Some(ty)).expect("encode");
            let decoded = SdoData::decode(&bytes, Some(ty)).expect("decode");
            assert_eq!(decoded, value, "{ty:?}");
        }
        assert_eq!(SdoType::from_declaration(Some("UDINT")), Some(SdoType::U32));
        assert_eq!(SdoType::from_declaration(Some("IDENTITY")), None);
        assert_eq!(SdoType::U32.bit_len(), Some(32));
    }
}
