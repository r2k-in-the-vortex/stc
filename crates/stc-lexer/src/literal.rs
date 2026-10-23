//! Decoded literal values attached to tokens (Tokenization §5).
//!
//! Decoding is lexical: the lexer records shapes and values but does not
//! perform range checking. Numeric digits are kept as strings so values of any
//! precision survive to the analyzer (STC-LEX-023).

/// An elementary type that may prefix a typed literal (STC-LEX-022).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ElementaryType {
    Bool,
    Sint,
    Int,
    Dint,
    Lint,
    Usint,
    Uint,
    Udint,
    Ulint,
    Byte,
    Word,
    Dword,
    Lword,
    Real,
    Lreal,
    Char,
    Wchar,
    Uchar,
}

impl ElementaryType {
    /// Looks up an elementary type from its uppercase spelling.
    #[must_use]
    pub fn from_prefix(text: &str) -> Option<Self> {
        Some(match text {
            "BOOL" => Self::Bool,
            "SINT" => Self::Sint,
            "INT" => Self::Int,
            "DINT" => Self::Dint,
            "LINT" => Self::Lint,
            "USINT" => Self::Usint,
            "UINT" => Self::Uint,
            "UDINT" => Self::Udint,
            "ULINT" => Self::Ulint,
            "BYTE" => Self::Byte,
            "WORD" => Self::Word,
            "DWORD" => Self::Dword,
            "LWORD" => Self::Lword,
            "REAL" => Self::Real,
            "LREAL" => Self::Lreal,
            "CHAR" => Self::Char,
            "WCHAR" => Self::Wchar,
            "UCHAR" => Self::Uchar,
            _ => return None,
        })
    }

    /// Returns the canonical spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bool => "BOOL",
            Self::Sint => "SINT",
            Self::Int => "INT",
            Self::Dint => "DINT",
            Self::Lint => "LINT",
            Self::Usint => "USINT",
            Self::Uint => "UINT",
            Self::Udint => "UDINT",
            Self::Ulint => "ULINT",
            Self::Byte => "BYTE",
            Self::Word => "WORD",
            Self::Dword => "DWORD",
            Self::Lword => "LWORD",
            Self::Real => "REAL",
            Self::Lreal => "LREAL",
            Self::Char => "CHAR",
            Self::Wchar => "WCHAR",
            Self::Uchar => "UCHAR",
        }
    }

    /// Returns `true` for `BOOL`.
    #[must_use]
    pub const fn is_bool(self) -> bool {
        matches!(self, Self::Bool)
    }

    /// Returns `true` for `CHAR`, `WCHAR`, and `UCHAR`.
    #[must_use]
    pub const fn is_character(self) -> bool {
        matches!(self, Self::Char | Self::Wchar | Self::Uchar)
    }
}

/// Radix of a numeric literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Radix {
    Binary,
    Octal,
    Decimal,
    Hexadecimal,
}

impl Radix {
    /// Returns the radix base.
    #[must_use]
    pub const fn base(self) -> u32 {
        match self {
            Self::Binary => 2,
            Self::Octal => 8,
            Self::Decimal => 10,
            Self::Hexadecimal => 16,
        }
    }
}

/// Whether a numeric literal had a decimal point or exponent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumericKind {
    Integer,
    Real,
}

/// A decoded numeric literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumericLiteral {
    /// Integer or real form.
    pub kind: NumericKind,
    /// Radix; real literals are always decimal.
    pub radix: Radix,
    /// Normalized digits: no underscores, uppercase hexadecimal digits, and an
    /// uppercase `E` for exponents. Leading zeros are preserved.
    pub text: Box<str>,
}

/// An explicit sign on a typed literal or duration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sign {
    Plus,
    Minus,
}

impl Sign {
    /// Returns the source character.
    #[must_use]
    pub const fn as_char(self) -> char {
        match self {
            Self::Plus => '+',
            Self::Minus => '-',
        }
    }
}

/// The decoded value of a typed literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypedValue {
    /// A decimal, based, or real numeric payload.
    Numeric(NumericLiteral),
    /// `BOOL#TRUE` or `BOOL#FALSE`.
    Bool(bool),
}

/// A typed literal such as `INT#-123` or `BOOL#FALSE` (STC-LEX-022).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedLiteral {
    /// The prefix type.
    pub ty: ElementaryType,
    /// A sign immediately after `#`, if present.
    pub sign: Option<Sign>,
    /// The decoded payload.
    pub value: TypedValue,
}

/// Source encoding of a string literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringEncoding {
    /// Single-byte character string (`STRING`, single-quoted untyped literal).
    Sbcs,
    /// Double-byte character string (`WSTRING`, double-quoted untyped literal).
    Dbcs,
    /// UTF-8 source string (`USTRING`, `U#`); units are Unicode scalars.
    Utf8,
}

/// Character literal type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CharKind {
    Char,
    WChar,
    UChar,
}

impl CharKind {
    /// Returns the type spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Char => "CHAR",
            Self::WChar => "WCHAR",
            Self::UChar => "UCHAR",
        }
    }
}

/// A decoded string literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringLiteral {
    /// Source encoding.
    pub encoding: StringEncoding,
    /// The quote character that delimited the literal.
    pub delimiter: char,
    /// Decoded code units: bytes for [`StringEncoding::Sbcs`], UTF-16 code
    /// units for [`StringEncoding::Dbcs`], and Unicode scalar values for
    /// [`StringEncoding::Utf8`].
    pub units: Vec<u32>,
}

/// A decoded character literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharLiteral {
    /// The prefix type.
    pub kind: CharKind,
    /// The single decoded code unit or scalar, or `None` when the literal was
    /// malformed.
    pub value: Option<u32>,
}

/// A duration component unit (STC-LEX-038).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimeUnit {
    Day,
    Hour,
    Minute,
    Second,
    Millisecond,
    Microsecond,
    Nanosecond,
}

impl TimeUnit {
    /// Returns the canonical unit spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Day => "d",
            Self::Hour => "h",
            Self::Minute => "m",
            Self::Second => "s",
            Self::Millisecond => "ms",
            Self::Microsecond => "us",
            Self::Nanosecond => "ns",
        }
    }
}

/// One duration component, for example `3.5ms`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurationComponent {
    /// Integer digits without leading sign or underscores.
    pub value: Box<str>,
    /// Fractional digits without underscores, if present.
    pub fraction: Option<Box<str>>,
    /// The unit.
    pub unit: TimeUnit,
}

/// A duration literal such as `TIME#25h_15m`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurationLiteral {
    /// `true` for `LTIME#`/`LT#`.
    pub long: bool,
    /// An optional leading sign.
    pub sign: Option<Sign>,
    /// Components in source order.
    pub components: Vec<DurationComponent>,
}

/// A date literal such as `D#1984-06-25`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateLiteral {
    /// `true` for `LDATE#`/`LD#`.
    pub long: bool,
    /// Four-digit year.
    pub year: u32,
    /// Two-digit month.
    pub month: u32,
    /// Two-digit day.
    pub day: u32,
}

/// A time-of-day literal such as `TOD#15:36:55.36`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TodLiteral {
    /// `true` for `LTOD#`/`LTIME_OF_DAY#`.
    pub long: bool,
    /// Two-digit hour.
    pub hour: u32,
    /// Two-digit minute.
    pub minute: u32,
    /// Two-digit second.
    pub second: u32,
    /// Fractional-second digits without underscores, if present.
    pub fraction: Option<Box<str>>,
}

/// A date-and-time literal such as `DT#1984-06-25-15:36:55.36`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateTimeLiteral {
    /// `true` for the long forms (`LDT#`, `LDATE_AND_TIME#`).
    pub long: bool,
    /// The date part.
    pub date: DateLiteral,
    /// The time part.
    pub tod: TodLiteral,
}

/// Process-image area of a directly represented variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DirectArea {
    Input,
    Output,
    Memory,
}

impl DirectArea {
    /// Returns the address letter.
    #[must_use]
    pub const fn as_char(self) -> char {
        match self {
            Self::Input => 'I',
            Self::Output => 'Q',
            Self::Memory => 'M',
        }
    }
}

/// Size prefix of a directly represented variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DirectSize {
    Bit,
    Byte,
    Word,
    Dword,
    Lword,
}

impl DirectSize {
    /// Returns the address letter.
    #[must_use]
    pub const fn as_char(self) -> char {
        match self {
            Self::Bit => 'X',
            Self::Byte => 'B',
            Self::Word => 'W',
            Self::Dword => 'D',
            Self::Lword => 'L',
        }
    }
}

/// A directly represented variable such as `%IX0.0` or `%I*` (STC-LEX-043).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectVariable {
    /// Optional area letter.
    pub area: Option<DirectArea>,
    /// Optional size letter.
    pub size: Option<DirectSize>,
    /// `true` for the `%I*`/`%Q*`/`%M*` wildcard forms.
    pub wildcard: bool,
    /// Dot-separated decimal indices. Empty for a wildcard.
    pub indices: Vec<u32>,
}

/// A decoded literal value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Literal {
    Numeric(NumericLiteral),
    Typed(TypedLiteral),
    String(StringLiteral),
    Char(CharLiteral),
    Duration(DurationLiteral),
    Date(DateLiteral),
    Tod(TodLiteral),
    DateTime(DateTimeLiteral),
    Direct(DirectVariable),
}
