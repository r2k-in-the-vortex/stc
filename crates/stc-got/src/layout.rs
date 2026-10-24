//! Type keys, signatures, and target layout hashes (GOT Artifacts §4).

use std::fmt;
use std::str::FromStr;

use crate::digest::{Digest32, GotId};
use crate::error::{Error, Result};
use crate::sha256::{domain_hash, sha256};

/// String encoding category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StringKind {
    /// `STRING`, one byte per code unit.
    Sbcs,
    /// `WSTRING`, two bytes per code unit.
    Dbcs,
    /// `USTRING`, four bytes per Unicode scalar.
    Unicode,
}

impl StringKind {
    /// Returns the keyword spelling.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Sbcs => "STRING",
            Self::Dbcs => "WSTRING",
            Self::Unicode => "USTRING",
        }
    }

    /// Returns the storage width of one code unit.
    #[must_use]
    pub const fn unit_bytes(self) -> u8 {
        match self {
            Self::Sbcs => 1,
            Self::Dbcs => 2,
            Self::Unicode => 4,
        }
    }
}

/// One array bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArrayBound {
    /// A fixed bound.
    Value(i128),
    /// A conformant-array `*` bound.
    Conformant,
}

impl fmt::Display for ArrayBound {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(value) => write!(formatter, "{value}"),
            Self::Conformant => formatter.write_str("*"),
        }
    }
}

/// A canonical structural type description.
///
/// The M4.5 surface uses the elementary variants; the remaining variants are
/// part of format 1 so M5 does not need a format change.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TypeKey {
    /// `BOOL`.
    Bool,
    /// `SINT`.
    Sint,
    /// `INT`.
    Int,
    /// `DINT`.
    Dint,
    /// `LINT`.
    Lint,
    /// `USINT`.
    Usint,
    /// `UINT`.
    Uint,
    /// `UDINT`.
    Udint,
    /// `ULINT`.
    Ulint,
    /// `REAL`.
    Real,
    /// `LREAL`.
    Lreal,
    /// `TIME`.
    Time,
    /// `LTIME`.
    Ltime,
    /// `DATE`.
    Date,
    /// `LDATE`.
    Ldate,
    /// `TOD`.
    Tod,
    /// `LTOD`.
    Ltod,
    /// `DT`.
    Dt,
    /// `LDT`.
    Ldt,
    /// `BYTE`.
    Byte,
    /// `WORD`.
    Word,
    /// `DWORD`.
    Dword,
    /// `LWORD`.
    Lword,
    /// `CHAR`.
    Char,
    /// `WCHAR`.
    Wchar,
    /// `UCHAR`.
    Uchar,
    /// A fixed-capacity string.
    String {
        /// Encoding category.
        kind: StringKind,
        /// Capacity in code units.
        capacity: u32,
    },
    /// A one-dimensional static array.
    ///
    /// Format 1 defines a single bound pair; multidimensional HIR arrays are
    /// represented by nesting, which is an M5 surface question.
    Array {
        /// Inclusive lower bound.
        lower: ArrayBound,
        /// Inclusive upper bound.
        upper: ArrayBound,
        /// Element type.
        element: Box<TypeKey>,
    },
    /// `REF_TO`.
    RefTo(Box<TypeKey>),
    /// `PTR_TO` (transient-only `STC-POINTER`).
    PtrTo(Box<TypeKey>),
    /// A named user-defined type, identified by its type `GotId`.
    Named(GotId),
    /// `VOID`.
    Void,
}

impl TypeKey {
    /// Parses a type key.
    ///
    /// # Errors
    ///
    /// Rejects unknown keywords, malformed bounds, and excessive nesting.
    pub fn parse(text: &str) -> Result<Self> {
        let mut parser = TypeParser {
            text: text.as_bytes(),
            position: 0,
            depth: 0,
        };
        let value = parser.type_key()?;
        if parser.position != parser.text.len() {
            return Err(parser.error("trailing characters after the type key"));
        }
        Ok(value)
    }

    /// Returns the canonical type key string.
    #[must_use]
    pub fn render(&self) -> String {
        self.to_string()
    }

    /// Returns `true` when the type contains a transient pointer.
    #[must_use]
    pub fn contains_pointer(&self) -> bool {
        match self {
            Self::PtrTo(_) => true,
            Self::Array { element, .. } | Self::RefTo(element) => element.contains_pointer(),
            _ => false,
        }
    }

    /// Returns the named type identity, when this is a named user type.
    #[must_use]
    pub fn named(&self) -> Option<GotId> {
        match self {
            Self::Named(id) => Some(*id),
            _ => None,
        }
    }
}

impl fmt::Display for TypeKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool => formatter.write_str("BOOL"),
            Self::Sint => formatter.write_str("SINT"),
            Self::Int => formatter.write_str("INT"),
            Self::Dint => formatter.write_str("DINT"),
            Self::Lint => formatter.write_str("LINT"),
            Self::Usint => formatter.write_str("USINT"),
            Self::Uint => formatter.write_str("UINT"),
            Self::Udint => formatter.write_str("UDINT"),
            Self::Ulint => formatter.write_str("ULINT"),
            Self::Real => formatter.write_str("REAL"),
            Self::Lreal => formatter.write_str("LREAL"),
            Self::Time => formatter.write_str("TIME"),
            Self::Ltime => formatter.write_str("LTIME"),
            Self::Date => formatter.write_str("DATE"),
            Self::Ldate => formatter.write_str("LDATE"),
            Self::Tod => formatter.write_str("TOD"),
            Self::Ltod => formatter.write_str("LTOD"),
            Self::Dt => formatter.write_str("DT"),
            Self::Ldt => formatter.write_str("LDT"),
            Self::Byte => formatter.write_str("BYTE"),
            Self::Word => formatter.write_str("WORD"),
            Self::Dword => formatter.write_str("DWORD"),
            Self::Lword => formatter.write_str("LWORD"),
            Self::Char => formatter.write_str("CHAR"),
            Self::Wchar => formatter.write_str("WCHAR"),
            Self::Uchar => formatter.write_str("UCHAR"),
            Self::String { kind, capacity } => {
                write!(formatter, "{}[{capacity}]", kind.keyword())
            }
            Self::Array {
                lower,
                upper,
                element,
            } => write!(formatter, "ARRAY[{lower}..{upper}]OF({element})"),
            Self::RefTo(target) => write!(formatter, "REF_TO({target})"),
            Self::PtrTo(target) => write!(formatter, "PTR_TO({target})"),
            Self::Named(id) => write!(formatter, "T:{}", id.to_hex_plain()),
            Self::Void => formatter.write_str("VOID"),
        }
    }
}

impl FromStr for TypeKey {
    type Err = Error;

    fn from_str(text: &str) -> std::result::Result<Self, Self::Err> {
        Self::parse(text)
    }
}

struct TypeParser<'a> {
    text: &'a [u8],
    position: usize,
    depth: usize,
}

impl TypeParser<'_> {
    fn error(&self, message: impl Into<String>) -> Error {
        Error::manifest(format!(
            "type key error at byte {}: {}",
            self.position,
            message.into()
        ))
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.position).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.position += 1;
        Some(byte)
    }

    fn expect(&mut self, expected: u8) -> Result<()> {
        match self.bump() {
            Some(byte) if byte == expected => Ok(()),
            Some(byte) => Err(self.error(format!(
                "expected `{}`, found `{}`",
                expected as char, byte as char
            ))),
            None => Err(self.error(format!(
                "expected `{}`, found end of input",
                expected as char
            ))),
        }
    }

    fn keyword(&mut self, word: &str) -> bool {
        if self.text[self.position..].starts_with(word.as_bytes()) {
            self.position += word.len();
            true
        } else {
            false
        }
    }

    fn type_key(&mut self) -> Result<TypeKey> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(self.error("type nesting exceeds 64 levels"));
        }
        let value = if self.keyword("BOOL") {
            TypeKey::Bool
        } else if self.keyword("SINT") {
            TypeKey::Sint
        } else if self.keyword("DINT") {
            TypeKey::Dint
        } else if self.keyword("INT") {
            TypeKey::Int
        } else if self.keyword("LINT") {
            TypeKey::Lint
        } else if self.keyword("USINT") {
            TypeKey::Usint
        } else if self.keyword("UDINT") {
            TypeKey::Udint
        } else if self.keyword("UINT") {
            TypeKey::Uint
        } else if self.keyword("ULINT") {
            TypeKey::Ulint
        } else if self.keyword("REAL") {
            TypeKey::Real
        } else if self.keyword("LREAL") {
            TypeKey::Lreal
        } else if self.keyword("LTIME") {
            TypeKey::Ltime
        } else if self.keyword("TIME") {
            TypeKey::Time
        } else if self.keyword("LDATE") {
            TypeKey::Ldate
        } else if self.keyword("DATE") {
            TypeKey::Date
        } else if self.keyword("LTOD") {
            TypeKey::Ltod
        } else if self.keyword("TOD") {
            TypeKey::Tod
        } else if self.keyword("LDT") {
            TypeKey::Ldt
        } else if self.keyword("DT") {
            TypeKey::Dt
        } else if self.keyword("BYTE") {
            TypeKey::Byte
        } else if self.keyword("DWORD") {
            TypeKey::Dword
        } else if self.keyword("WORD") {
            TypeKey::Word
        } else if self.keyword("LWORD") {
            TypeKey::Lword
        } else if self.keyword("WCHAR") {
            TypeKey::Wchar
        } else if self.keyword("CHAR") {
            TypeKey::Char
        } else if self.keyword("UCHAR") {
            TypeKey::Uchar
        } else if self.keyword("STRING[") {
            TypeKey::String {
                kind: StringKind::Sbcs,
                capacity: self.capacity()?,
            }
        } else if self.keyword("WSTRING[") {
            TypeKey::String {
                kind: StringKind::Dbcs,
                capacity: self.capacity()?,
            }
        } else if self.keyword("USTRING[") {
            TypeKey::String {
                kind: StringKind::Unicode,
                capacity: self.capacity()?,
            }
        } else if self.keyword("ARRAY[") {
            let lower = self.bound()?;
            self.expect(b'.')?;
            self.expect(b'.')?;
            let upper = self.bound()?;
            self.expect(b']')?;
            if !self.keyword("OF(") {
                return Err(self.error("expected `OF(` after an array bound"));
            }
            let element = Box::new(self.type_key()?);
            self.expect(b')')?;
            TypeKey::Array {
                lower,
                upper,
                element,
            }
        } else if self.keyword("REF_TO(") {
            let target = Box::new(self.type_key()?);
            self.expect(b')')?;
            TypeKey::RefTo(target)
        } else if self.keyword("PTR_TO(") {
            let target = Box::new(self.type_key()?);
            self.expect(b')')?;
            TypeKey::PtrTo(target)
        } else if self.keyword("T:") {
            let start = self.position;
            let end = start + 32;
            if end > self.text.len() {
                return Err(self.error("truncated named-type identity"));
            }
            let hex = std::str::from_utf8(&self.text[start..end])
                .map_err(|_| self.error("named-type identity is not ASCII"))?;
            let id = GotId::parse(&format!("0x{hex}"))?;
            self.position = end;
            TypeKey::Named(id)
        } else if self.keyword("VOID") {
            TypeKey::Void
        } else {
            return Err(self.error("unknown type key"));
        };
        self.depth -= 1;
        Ok(value)
    }

    fn capacity(&mut self) -> Result<u32> {
        let start = self.position;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.bump();
        }
        if start == self.position {
            return Err(self.error("expected a string capacity"));
        }
        let text = std::str::from_utf8(&self.text[start..self.position])
            .map_err(|_| self.error("capacity is not ASCII"))?;
        let value = text
            .parse::<u32>()
            .map_err(|_| self.error("string capacity is out of range"))?;
        self.expect(b']')?;
        Ok(value)
    }

    fn bound(&mut self) -> Result<ArrayBound> {
        if self.peek() == Some(b'*') {
            self.bump();
            return Ok(ArrayBound::Conformant);
        }
        let start = self.position;
        if self.peek() == Some(b'-') {
            self.bump();
        }
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.bump();
        }
        if start == self.position {
            return Err(self.error("expected an array bound"));
        }
        let text = std::str::from_utf8(&self.text[start..self.position])
            .map_err(|_| self.error("array bound is not ASCII"))?;
        text.parse::<i128>()
            .map(ArrayBound::Value)
            .map_err(|_| self.error("array bound is out of range"))
    }
}

/// Parameter direction in a signature key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ParamMode {
    /// `VAR_INPUT` / plain input.
    In,
    /// `VAR_OUTPUT`.
    Out,
    /// `VAR_IN_OUT`.
    InOut,
}

impl ParamMode {
    /// Returns the signature spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::In => "IN",
            Self::Out => "OUT",
            Self::InOut => "INOUT",
        }
    }
}

/// One signature parameter.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Parameter {
    /// Direction.
    pub mode: ParamMode,
    /// Parameter type.
    pub ty: TypeKey,
}

/// A canonical callable signature key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Signature {
    /// Parameters in declaration order.
    pub parameters: Vec<Parameter>,
    /// Result type; `TypeKey::Void` for no result.
    pub result: TypeKey,
}

impl Signature {
    /// Creates a signature with no parameters and the given result.
    #[must_use]
    pub fn new(result: TypeKey) -> Self {
        Self {
            parameters: Vec::new(),
            result,
        }
    }

    /// Parses a signature key.
    ///
    /// # Errors
    ///
    /// Rejects malformed mode/type lists and trailing input.
    pub fn parse(text: &str) -> Result<Self> {
        let mut parser = SignatureParser {
            text: text.as_bytes(),
            position: 0,
        };
        let value = parser.signature()?;
        if parser.position != parser.text.len() {
            return Err(parser.error("trailing characters after the signature"));
        }
        Ok(value)
    }

    /// Returns the canonical signature string.
    #[must_use]
    pub fn render(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("(")?;
        for (index, parameter) in self.parameters.iter().enumerate() {
            if index > 0 {
                formatter.write_str(",")?;
            }
            write!(formatter, "{}:{}", parameter.mode.name(), parameter.ty)?;
        }
        write!(formatter, ")->{}", self.result)
    }
}

impl FromStr for Signature {
    type Err = Error;

    fn from_str(text: &str) -> std::result::Result<Self, Self::Err> {
        Self::parse(text)
    }
}

struct SignatureParser<'a> {
    text: &'a [u8],
    position: usize,
}

impl SignatureParser<'_> {
    fn error(&self, message: impl Into<String>) -> Error {
        Error::manifest(format!(
            "signature error at byte {}: {}",
            self.position,
            message.into()
        ))
    }

    fn expect(&mut self, expected: u8) -> Result<()> {
        match self.text.get(self.position).copied() {
            Some(byte) if byte == expected => {
                self.position += 1;
                Ok(())
            }
            Some(byte) => Err(self.error(format!(
                "expected `{}`, found `{}`",
                expected as char, byte as char
            ))),
            None => Err(self.error(format!(
                "expected `{}`, found end of input",
                expected as char
            ))),
        }
    }

    fn keyword(&mut self, word: &str) -> bool {
        if self.text[self.position..].starts_with(word.as_bytes()) {
            self.position += word.len();
            true
        } else {
            false
        }
    }

    fn mode(&mut self) -> Result<ParamMode> {
        if self.keyword("INOUT") {
            Ok(ParamMode::InOut)
        } else if self.keyword("OUT") {
            Ok(ParamMode::Out)
        } else if self.keyword("IN") {
            Ok(ParamMode::In)
        } else {
            Err(self.error("expected a parameter mode"))
        }
    }

    fn signature(&mut self) -> Result<Signature> {
        self.expect(b'(')?;
        let mut parameters = Vec::new();
        if self.text.get(self.position) != Some(&b')') {
            loop {
                let mode = self.mode()?;
                self.expect(b':')?;
                let ty = self.type_key()?;
                parameters.push(Parameter { mode, ty });
                match self.text.get(self.position) {
                    Some(b',') => {
                        self.position += 1;
                    }
                    Some(b')') => break,
                    _ => return Err(self.error("expected `,` or `)` after a parameter")),
                }
            }
        }
        self.expect(b')')?;
        self.expect(b'-')?;
        self.expect(b'>')?;
        let result = self.type_key()?;
        Ok(Signature { parameters, result })
    }

    fn type_key(&mut self) -> Result<TypeKey> {
        let mut parser = TypeParser {
            text: self.text,
            position: self.position,
            depth: 0,
        };
        let value = parser.type_key()?;
        self.position = parser.position;
        Ok(value)
    }
}

/// Byte order of the build target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Endianness {
    /// Little-endian.
    Little,
    /// Big-endian.
    Big,
}

impl Endianness {
    /// Returns the artifact tag.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Little => 0,
            Self::Big => 1,
        }
    }
}

/// A resolved build target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// Resolved LLVM target triple.
    pub triple: String,
    /// Byte order.
    pub endianness: Endianness,
    /// Pointer width in bytes (8 for the supported targets).
    pub pointer_width: u8,
}

impl Target {
    /// Creates a target description.
    #[must_use]
    pub fn new(triple: impl Into<String>, endianness: Endianness, pointer_width: u8) -> Self {
        Self {
            triple: triple.into(),
            endianness,
            pointer_width,
        }
    }
}

/// A target storage layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Layout {
    /// Size in bytes.
    pub size: u64,
    /// Alignment in bytes (a power of two).
    pub align: u32,
}

/// Elementary encoding tags used by the layout byte string.
///
/// The values are fixed by this implementation and recorded in GOT Artifacts
/// §4.3; the type key is also hashed, so they are a stable descriptive tag,
/// not the sole layout discriminator.
pub mod encoding {
    /// `BOOL`.
    pub const BOOL: u8 = 1;
    /// Signed integer.
    pub const SIGNED_INT: u8 = 2;
    /// Unsigned integer.
    pub const UNSIGNED_INT: u8 = 3;
    /// Bit string.
    pub const BIT_STRING: u8 = 4;
    /// `REAL`.
    pub const REAL32: u8 = 5;
    /// `LREAL`.
    pub const REAL64: u8 = 6;
    /// `TIME`/`LTIME`.
    pub const DURATION: u8 = 7;
    /// `DATE`/`LDATE`.
    pub const DATE: u8 = 8;
    /// `TOD`/`LTOD`.
    pub const TIME_OF_DAY: u8 = 9;
    /// `DT`/`LDT`.
    pub const DATE_AND_TIME: u8 = 10;
    /// `CHAR`.
    pub const CHAR: u8 = 11;
    /// `WCHAR`.
    pub const WCHAR: u8 = 12;
    /// `UCHAR`.
    pub const UCHAR: u8 = 13;
}

/// Returns the elementary encoding tag for a type key.
#[must_use]
pub fn elementary_encoding(ty: &TypeKey) -> Option<u8> {
    Some(match ty {
        TypeKey::Bool => encoding::BOOL,
        TypeKey::Sint | TypeKey::Int | TypeKey::Dint | TypeKey::Lint => encoding::SIGNED_INT,
        TypeKey::Usint | TypeKey::Uint | TypeKey::Udint | TypeKey::Ulint => encoding::UNSIGNED_INT,
        TypeKey::Byte | TypeKey::Word | TypeKey::Dword | TypeKey::Lword => encoding::BIT_STRING,
        TypeKey::Real => encoding::REAL32,
        TypeKey::Lreal => encoding::REAL64,
        TypeKey::Time | TypeKey::Ltime => encoding::DURATION,
        TypeKey::Date | TypeKey::Ldate => encoding::DATE,
        TypeKey::Tod | TypeKey::Ltod => encoding::TIME_OF_DAY,
        TypeKey::Dt | TypeKey::Ldt => encoding::DATE_AND_TIME,
        TypeKey::Char => encoding::CHAR,
        TypeKey::Wchar => encoding::WCHAR,
        TypeKey::Uchar => encoding::UCHAR,
        _ => return None,
    })
}

/// Returns the target layout of an elementary type.
///
/// The M4.5-compatible targets all use the same elementary sizes and natural
/// alignments. Strings, arrays, references, and named types require a layout
/// supplied by the compiler.
#[must_use]
pub fn elementary_layout(ty: &TypeKey) -> Option<Layout> {
    let (size, align) = match ty {
        TypeKey::Bool => (1, 1),
        TypeKey::Sint | TypeKey::Usint | TypeKey::Byte | TypeKey::Char => (1, 1),
        TypeKey::Int | TypeKey::Uint | TypeKey::Word | TypeKey::Wchar => (2, 2),
        TypeKey::Dint | TypeKey::Udint | TypeKey::Dword | TypeKey::Real | TypeKey::Uchar => (4, 4),
        TypeKey::Lint
        | TypeKey::Ulint
        | TypeKey::Lword
        | TypeKey::Lreal
        | TypeKey::Time
        | TypeKey::Ltime
        | TypeKey::Date
        | TypeKey::Ldate
        | TypeKey::Tod
        | TypeKey::Ltod
        | TypeKey::Dt
        | TypeKey::Ldt => (8, 8),
        _ => return None,
    };
    Some(Layout { size, align })
}

/// Encodes the elementary shape row.
#[must_use]
pub fn elementary_layout_bytes(layout: Layout, encoding: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(10);
    out.push(0x01);
    out.extend_from_slice(&u32::try_from(layout.size).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(&layout.align.to_le_bytes());
    out.push(encoding);
    out
}

/// Encodes the string shape row.
#[must_use]
pub fn string_layout_bytes(capacity: u32, unit_bytes: u8, layout: Layout) -> Vec<u8> {
    let mut out = Vec::with_capacity(14);
    out.push(0x02);
    out.extend_from_slice(&capacity.to_le_bytes());
    out.push(unit_bytes);
    out.extend_from_slice(&u32::try_from(layout.size).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(&layout.align.to_le_bytes());
    out
}

/// Encodes the array shape row.
#[must_use]
pub fn array_layout_bytes(
    element_count: u64,
    stride: u64,
    element_key: &TypeKey,
    element_layout: GotId,
) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(0x03);
    out.extend_from_slice(&element_count.to_le_bytes());
    out.extend_from_slice(&stride.to_le_bytes());
    out.extend_from_slice(element_key.to_string().as_bytes());
    out.push(0);
    out.extend_from_slice(element_layout.as_bytes());
    out
}

/// Encodes the struct/FB/class shape row.
#[must_use]
pub fn struct_layout_bytes(size: u32, align: u32, fields_hash: GotId) -> Vec<u8> {
    let mut out = Vec::with_capacity(25);
    out.push(0x04);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&align.to_le_bytes());
    out.extend_from_slice(fields_hash.as_bytes());
    out
}

/// Encodes the `OVERLAP` shape row.
#[must_use]
pub fn overlap_layout_bytes(size: u32, align: u32, union_hash: GotId) -> Vec<u8> {
    let mut out = Vec::with_capacity(25);
    out.push(0x05);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&align.to_le_bytes());
    out.extend_from_slice(union_hash.as_bytes());
    out
}

/// Encodes the reference/pointer shape row.
#[must_use]
pub fn reference_layout_bytes(kind: u8, target: &TypeKey) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(0x06);
    out.push(kind);
    out.extend_from_slice(target.to_string().as_bytes());
    out.push(0);
    out
}

/// One field entry for a struct/overlap field-list hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldLayout {
    /// Field path.
    pub path: String,
    /// Byte offset.
    pub offset: u64,
    /// Size in bytes.
    pub size: u64,
    /// Alignment in bytes.
    pub align: u32,
    /// Field type key.
    pub type_key: TypeKey,
    /// Field layout hash.
    pub layout_hash: GotId,
}

/// Computes the SHA-256 field-list hash used by struct and overlap rows.
///
/// Entries are sorted by `(offset, path)` and concatenated as defined in GOT
/// Artifacts §4.3. The 16-byte prefix is returned.
#[must_use]
pub fn field_list_hash(entries: &[FieldLayout]) -> GotId {
    let mut ordered = entries.to_vec();
    ordered.sort_by(|left, right| {
        (left.offset, left.path.as_str()).cmp(&(right.offset, right.path.as_str()))
    });
    let mut input = Vec::new();
    for entry in &ordered {
        input.extend_from_slice(entry.path.as_bytes());
        input.push(0);
        input.extend_from_slice(&entry.offset.to_le_bytes());
        input.extend_from_slice(&entry.size.to_le_bytes());
        input.extend_from_slice(&entry.align.to_le_bytes());
        input.extend_from_slice(entry.type_key.to_string().as_bytes());
        input.push(0);
        input.extend_from_slice(entry.layout_hash.as_bytes());
    }
    let digest = sha256(&input);
    let mut truncated = [0_u8; 16];
    truncated.copy_from_slice(&digest[..16]);
    GotId::from_bytes(truncated)
}

/// Computes a target-specific layout hash.
#[must_use]
pub fn layout_hash(target: &Target, ty: &TypeKey, layout_bytes: &[u8]) -> GotId {
    let endian = [target.endianness.tag()];
    let width = [target.pointer_width];
    let separator = [0_u8];
    let text = ty.to_string();
    let digest = domain_hash(
        "stc:layout:v1",
        &[
            target.triple.as_bytes(),
            &separator,
            &endian,
            &width,
            text.as_bytes(),
            &separator,
            layout_bytes,
        ],
    );
    let mut truncated = [0_u8; 16];
    truncated.copy_from_slice(&digest[..16]);
    GotId::from_bytes(truncated)
}

/// Computes the layout hash of an elementary type.
///
/// Returns `None` for types that require a compiler-supplied layout.
#[must_use]
pub fn elementary_layout_hash(target: &Target, ty: &TypeKey) -> Option<GotId> {
    let layout = elementary_layout(ty)?;
    let encoding = elementary_encoding(ty)?;
    Some(layout_hash(
        target,
        ty,
        &elementary_layout_bytes(layout, encoding),
    ))
}

/// Computes the full SHA-256 of a layout byte string, for tests and tooling.
#[must_use]
pub fn layout_digest(target: &Target, ty: &TypeKey, layout_bytes: &[u8]) -> Digest32 {
    let endian = [target.endianness.tag()];
    let width = [target.pointer_width];
    let separator = [0_u8];
    let text = ty.to_string();
    Digest32::from_bytes(domain_hash(
        "stc:layout:v1",
        &[
            target.triple.as_bytes(),
            &separator,
            &endian,
            &width,
            text.as_bytes(),
            &separator,
            layout_bytes,
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_keys_round_trip() {
        let cases = [
            "BOOL",
            "DINT",
            "UCHAR",
            "STRING[255]",
            "WSTRING[80]",
            "USTRING[10]",
            "ARRAY[1..10]OF(DINT)",
            "ARRAY[*..*]OF(BYTE)",
            "REF_TO(LREAL)",
            "PTR_TO(INT)",
            "T:00112233445566778899aabbccddeeff",
            "VOID",
        ];
        for text in cases {
            let parsed = TypeKey::parse(text).unwrap();
            assert_eq!(parsed.to_string(), text);
        }
        assert_eq!(
            TypeKey::parse("ARRAY[-5..5]OF(REF_TO(DINT))")
                .unwrap()
                .to_string(),
            "ARRAY[-5..5]OF(REF_TO(DINT))"
        );
    }

    #[test]
    fn rejects_malformed_type_keys() {
        for text in [
            "",
            "dint",
            "STRING[]",
            "STRING[999999999999]",
            "ARRAY[1..2]OF DINT",
            "T:xyz",
            "REF_TO()",
            "VOIDX",
        ] {
            assert!(TypeKey::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn signatures_round_trip() {
        let signature = Signature::parse("(IN:DINT,IN:DINT)->DINT").unwrap();
        assert_eq!(signature.parameters.len(), 2);
        assert_eq!(signature.to_string(), "(IN:DINT,IN:DINT)->DINT");
        let empty = Signature::parse("()->VOID").unwrap();
        assert!(empty.parameters.is_empty());
        assert_eq!(empty.to_string(), "()->VOID");
        let mixed = Signature::parse("(IN:BOOL,OUT:STRING[10],INOUT:REAL)->VOID").unwrap();
        assert_eq!(
            mixed.to_string(),
            "(IN:BOOL,OUT:STRING[10],INOUT:REAL)->VOID"
        );
        assert!(Signature::parse("(DINT)->VOID").is_err());
        assert!(Signature::parse("(IN:DINT)").is_err());
    }

    #[test]
    fn layout_hashes_are_target_and_type_specific() {
        let target = Target::new("x86_64-pc-windows-msvc", Endianness::Little, 8);
        let dint = elementary_layout_hash(&target, &TypeKey::Dint).unwrap();
        let int = elementary_layout_hash(&target, &TypeKey::Int).unwrap();
        assert_ne!(dint, int);
        let other = Target::new("aarch64-unknown-linux-gnu", Endianness::Little, 8);
        assert_ne!(
            dint,
            elementary_layout_hash(&other, &TypeKey::Dint).unwrap()
        );
        assert!(elementary_layout_hash(&target, &TypeKey::Void).is_none());
    }

    #[test]
    fn elementary_layouts_match_the_m4_storage_model() {
        assert_eq!(
            elementary_layout(&TypeKey::Bool),
            Some(Layout { size: 1, align: 1 })
        );
        assert_eq!(
            elementary_layout(&TypeKey::Dint),
            Some(Layout { size: 4, align: 4 })
        );
        assert_eq!(
            elementary_layout(&TypeKey::Lreal),
            Some(Layout { size: 8, align: 8 })
        );
        assert_eq!(
            elementary_layout(&TypeKey::Wchar),
            Some(Layout { size: 2, align: 2 })
        );
    }

    #[test]
    fn field_list_hash_is_order_independent() {
        let first = FieldLayout {
            path: "A".into(),
            offset: 0,
            size: 4,
            align: 4,
            type_key: TypeKey::Dint,
            layout_hash: GotId::from_bytes([1; 16]),
        };
        let second = FieldLayout {
            path: "B".into(),
            offset: 4,
            size: 4,
            align: 4,
            type_key: TypeKey::Dint,
            layout_hash: GotId::from_bytes([2; 16]),
        };
        assert_eq!(
            field_list_hash(&[first.clone(), second.clone()]),
            field_list_hash(&[second, first])
        );
    }
}
