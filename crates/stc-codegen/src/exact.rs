//! Exact-operation reference models (Testing and Verification §5.1–§5.3).
//!
//! These functions mirror the implementation's exact semantics: checked
//! integer arithmetic and division, logical shifts and rotates on bit
//! patterns, bit-pattern conversions, comparisons, selection, boolean
//! operations, and the fixed string-length operations. `crates/stc-codegen/
//! tests/lean_vectors.rs` checks the committed Lean-generated vectors against
//! this module and requires a vector for every registered operation.

/// The vector format version (§5.3).
pub const FORMAT_VERSION: u32 = 1;

/// The Lean model version recorded in every vector file.
pub const MODEL_VERSION: u32 = 1;

/// A fixed-width integer or bit-string layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntType {
    /// Width in bits (1 for `BOOL`, otherwise 8, 16, 32, or 64).
    pub width: u8,
    /// `true` for `SINT`/`INT`/`DINT`/`LINT`.
    pub signed: bool,
}

impl IntType {
    /// `BOOL`: the one-bit unsigned layout.
    pub const BOOL: Self = Self {
        width: 1,
        signed: false,
    };

    /// Creates a layout.
    #[must_use]
    pub const fn new(signed: bool, width: u8) -> Self {
        Self { signed, width }
    }

    /// Resolves an IEC type spelling used by the vectors.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "BOOL" => Self::BOOL,
            "SINT" => Self::new(true, 8),
            "INT" => Self::new(true, 16),
            "DINT" => Self::new(true, 32),
            "LINT" => Self::new(true, 64),
            "USINT" => Self::new(false, 8),
            "UINT" => Self::new(false, 16),
            "UDINT" => Self::new(false, 32),
            "ULINT" => Self::new(false, 64),
            "BYTE" => Self::new(false, 8),
            "WORD" => Self::new(false, 16),
            "DWORD" => Self::new(false, 32),
            "LWORD" => Self::new(false, 64),
            "CHAR" => Self::new(false, 8),
            "WCHAR" => Self::new(false, 16),
            "UCHAR" => Self::new(false, 32),
            _ => return None,
        })
    }

    /// Returns the number of distinct bit patterns.
    #[must_use]
    pub const fn modulus(self) -> i128 {
        1_i128 << self.width
    }

    /// Returns the smallest representable value.
    #[must_use]
    pub const fn minimum(self) -> i128 {
        if self.signed {
            -(1_i128 << (self.width - 1))
        } else {
            0
        }
    }

    /// Returns the largest representable value.
    #[must_use]
    pub const fn maximum(self) -> i128 {
        if self.signed {
            (1_i128 << (self.width - 1)) - 1
        } else {
            self.modulus() - 1
        }
    }

    /// Returns whether `value` is representable.
    #[must_use]
    pub const fn fits(self, value: i128) -> bool {
        self.minimum() <= value && value <= self.maximum()
    }

    /// Returns the two's-complement bit pattern of `value`.
    #[must_use]
    pub fn to_bits(self, value: i128) -> u128 {
        value.rem_euclid(self.modulus()) as u128
    }

    /// Interprets a bit pattern in the layout, wrapping modulo the width.
    #[must_use]
    pub fn from_bits(self, bits: u128) -> i128 {
        let modulus = self.modulus() as u128;
        let wrapped = bits % modulus;
        let value = wrapped as i128;
        if self.signed && value >= 1_i128 << (self.width - 1) {
            value - self.modulus()
        } else {
            value
        }
    }

    /// Returns the all-ones mask.
    #[must_use]
    pub fn mask(self) -> u128 {
        (1_u128 << self.width) - 1
    }

    /// Returns the masked shift/rotate amount (the low `width` bits of `n`).
    #[must_use]
    pub fn count(self, n: i64) -> u32 {
        n.rem_euclid(i64::from(self.width)) as u32
    }
}

/// A checked exact-operation failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trap {
    /// The exact result is outside the type's range.
    Overflow,
    /// The divisor is zero.
    DivZero,
    /// A conversion result is not representable.
    Range,
}

/// A checked exact result.
pub type Res = Result<i128, Trap>;

fn checked(ty: IntType, value: i128) -> Res {
    if ty.fits(value) {
        Ok(value)
    } else {
        Err(Trap::Overflow)
    }
}

/// Checked addition.
pub fn add(ty: IntType, a: i128, b: i128) -> Res {
    checked(ty, a.checked_add(b).ok_or(Trap::Overflow)?)
}

/// Checked subtraction.
pub fn sub(ty: IntType, a: i128, b: i128) -> Res {
    checked(ty, a.checked_sub(b).ok_or(Trap::Overflow)?)
}

/// Checked multiplication.
pub fn mul(ty: IntType, a: i128, b: i128) -> Res {
    checked(ty, a.checked_mul(b).ok_or(Trap::Overflow)?)
}

/// Checked negation (`0 - a`).
pub fn neg(ty: IntType, a: i128) -> Res {
    checked(ty, a.checked_neg().ok_or(Trap::Overflow)?)
}

/// Checked truncating division.
pub fn div(ty: IntType, a: i128, b: i128) -> Res {
    if b == 0 {
        return Err(Trap::DivZero);
    }
    if ty.signed && a == ty.minimum() && b == -1 {
        return Err(Trap::Overflow);
    }
    Ok(a / b)
}

/// Checked truncating remainder.
pub fn modulo(ty: IntType, a: i128, b: i128) -> Res {
    if b == 0 {
        return Err(Trap::DivZero);
    }
    if ty.signed && a == ty.minimum() && b == -1 {
        return Err(Trap::Overflow);
    }
    Ok(a % b)
}

/// Checked absolute value; the most-negative signed value overflows.
pub fn abs(ty: IntType, a: i128) -> Res {
    if ty.signed && a == ty.minimum() {
        Err(Trap::Overflow)
    } else {
        Ok(a.abs())
    }
}

/// Bitwise AND on the low `width` bits.
#[must_use]
pub fn bit_and(ty: IntType, a: u128, b: u128) -> u128 {
    (a % ty.modulus() as u128) & (b % ty.modulus() as u128)
}

/// Bitwise OR on the low `width` bits.
#[must_use]
pub fn bit_or(ty: IntType, a: u128, b: u128) -> u128 {
    (a % ty.modulus() as u128) | (b % ty.modulus() as u128)
}

/// Bitwise XOR on the low `width` bits.
#[must_use]
pub fn bit_xor(ty: IntType, a: u128, b: u128) -> u128 {
    (a % ty.modulus() as u128) ^ (b % ty.modulus() as u128)
}

/// Bitwise NOT on the low `width` bits.
#[must_use]
pub fn bit_not(ty: IntType, a: u128) -> u128 {
    (a % ty.modulus() as u128) ^ ty.mask()
}

/// Logical left shift by `n mod width`.
#[must_use]
pub fn shl(ty: IntType, a: u128, n: i64) -> u128 {
    ((a % ty.modulus() as u128) << ty.count(n)) % ty.modulus() as u128
}

/// Logical right shift by `n mod width`.
#[must_use]
pub fn shr(ty: IntType, a: u128, n: i64) -> u128 {
    (a % ty.modulus() as u128) >> ty.count(n)
}

/// Rotate left by `n mod width`.
#[must_use]
pub fn rol(ty: IntType, a: u128, n: i64) -> u128 {
    let r = ty.count(n);
    let x = a % ty.modulus() as u128;
    let width = u32::from(ty.width);
    ((x << r) | (x >> ((width - r) % width))) % ty.modulus() as u128
}

/// Rotate right by `n mod width`.
#[must_use]
pub fn ror(ty: IntType, a: u128, n: i64) -> u128 {
    let r = ty.count(n);
    let x = a % ty.modulus() as u128;
    let width = u32::from(ty.width);
    ((x >> r) | (x << ((width - r) % width))) % ty.modulus() as u128
}

/// A comparison operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compare {
    /// `=`.
    Eq,
    /// `<>`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
}

/// Compares two in-range values.
#[must_use]
pub fn compare(op: Compare, a: i128, b: i128) -> bool {
    match op {
        Compare::Eq => a == b,
        Compare::Ne => a != b,
        Compare::Lt => a < b,
        Compare::Le => a <= b,
        Compare::Gt => a > b,
        Compare::Ge => a >= b,
    }
}

/// `SEL(G, IN0, IN1)`: the selector chooses `IN1` when true.
#[must_use]
pub fn sel(g: bool, in0: i128, in1: i128) -> i128 {
    if g { in1 } else { in0 }
}

/// `MIN` over a non-empty value list, keeping the earlier value on ties.
#[must_use]
pub fn min_list(values: &[i128]) -> Option<i128> {
    values.split_first().map(|(first, rest)| {
        rest.iter()
            .fold(*first, |acc, &v| if v < acc { v } else { acc })
    })
}

/// `MAX` over a non-empty value list, keeping the earlier value on ties.
#[must_use]
pub fn max_list(values: &[i128]) -> Option<i128> {
    values.split_first().map(|(first, rest)| {
        rest.iter()
            .fold(*first, |acc, &v| if acc < v { v } else { acc })
    })
}

/// `LIMIT(MN, IN, MX)` clamps to the bounds in declaration order.
#[must_use]
pub fn limit(mn: i128, x: i128, mx: i128) -> i128 {
    let low = if x < mn { mn } else { x };
    if mx < x { mx } else { low }
}

/// `MUX(K, IN0, ..., DEFAULT)`: returns the `K`th value or the default.
#[must_use]
pub fn mux(k: i128, values: &[i128], default: i128) -> i128 {
    if k < 0 {
        return default;
    }
    usize::try_from(k)
        .ok()
        .and_then(|index| values.get(index))
        .copied()
        .unwrap_or(default)
}

/// Converts `value` from `src` to `to`.
///
/// `BOOL` targets test against zero (the generator's `Int -> BOOL` rule).
/// Widening sign- or zero-extends, equal widths reinterpret the bit pattern,
/// and narrowing reports [`Trap::Range`] when the value does not fit.
pub fn convert(src: IntType, to: IntType, value: i128) -> Res {
    if to.width == 1 {
        return Ok(i128::from(value != 0));
    }
    if src.width < to.width {
        if src.signed && value < 0 {
            if to.signed {
                Ok(value)
            } else {
                Ok(value + to.modulus())
            }
        } else {
            Ok(value)
        }
    } else if src.width > to.width {
        if to.fits(value) {
            Ok(value)
        } else {
            Err(Trap::Range)
        }
    } else if src.signed == to.signed {
        Ok(value)
    } else if to.signed {
        Ok(if value >= 1_i128 << (to.width - 1) {
            value - to.modulus()
        } else {
            value
        })
    } else {
        Ok(if value < 0 {
            value + to.modulus()
        } else {
            value
        })
    }
}

/// A string encoding used by `LEN` and `LEN_CODE_UNIT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringKind {
    /// `STRING` (single-byte units).
    Sbcs,
    /// `WSTRING` (UTF-16 code units).
    Dbcs,
    /// `USTRING` (Unicode scalars).
    Unicode,
}

impl StringKind {
    /// Resolves a vector kind spelling.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "SBCS" => Self::Sbcs,
            "DBCS" => Self::Dbcs,
            "UNICODE" => Self::Unicode,
            _ => return None,
        })
    }
}

/// `LEN`: the stored character/unit count.
///
/// # Panics
///
/// Panics when an SBCS string contains a non-ASCII character; the vectors keep
/// SBCS input to ASCII.
#[must_use]
pub fn len(kind: StringKind, text: &str) -> usize {
    match kind {
        StringKind::Sbcs => {
            assert!(text.is_ascii(), "SBCS vector `{text}` is not ASCII");
            text.len()
        }
        StringKind::Dbcs => text.encode_utf16().count(),
        StringKind::Unicode => text.chars().count(),
    }
}

/// `LEN_MAX`: the declared capacity.
#[must_use]
pub const fn len_max(capacity: usize) -> usize {
    capacity
}

/// `LEN_CODE_UNIT`: UTF-8 code units for `USTRING`; the stored unit count for
/// the 2013 encodings.
///
/// # Panics
///
/// Panics when an SBCS string contains a non-ASCII character.
#[must_use]
pub fn len_code_unit(kind: StringKind, text: &str) -> usize {
    match kind {
        StringKind::Sbcs => {
            assert!(text.is_ascii(), "SBCS vector `{text}` is not ASCII");
            text.len()
        }
        StringKind::Dbcs => text.encode_utf16().count(),
        StringKind::Unicode => text.len(),
    }
}

/// The type lists referenced by several operations.
pub const INT_TYPES: &[&str] = &[
    "SINT", "INT", "DINT", "LINT", "USINT", "UINT", "UDINT", "ULINT",
];
/// The bit-string type list.
pub const BIT_TYPES: &[&str] = &["BYTE", "WORD", "DWORD", "LWORD"];
/// The types accepted by exact comparisons.
pub const COMPARE_TYPES: &[&str] = &[
    "SINT", "INT", "DINT", "LINT", "USINT", "UINT", "UDINT", "ULINT", "BYTE", "WORD", "DWORD",
    "LWORD", "BOOL",
];
/// The types accepted by `SEL`.
pub const SELECT_TYPES: &[&str] = &["DINT", "LINT", "UDINT", "ULINT", "DWORD", "LWORD"];
/// The types accepted by `MUX`.
pub const MUX_TYPES: &[&str] = &["DINT", "UDINT", "LWORD"];
/// The types accepted by exact conversions.
pub const CONV_TYPES: &[&str] = &[
    "BOOL", "SINT", "INT", "DINT", "LINT", "USINT", "UINT", "UDINT", "ULINT", "BYTE", "WORD",
    "DWORD", "LWORD", "CHAR", "WCHAR", "UCHAR",
];
/// The string kinds accepted by the length operations.
pub const STRING_KINDS: &[&str] = &["SBCS", "DBCS", "UNICODE"];

/// The kind of case set a vector file must contain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FunctionKind {
    /// Binary arithmetic (checked).
    Binary {
        /// Required operand types.
        types: &'static [&'static str],
    },
    /// Unary arithmetic (`NEG`, `ABS`).
    Unary {
        /// Required operand types.
        types: &'static [&'static str],
    },
    /// Bitwise binary operations (`AND`, `OR`, `XOR`) including `BOOL`.
    Bitwise {
        /// Required operand types.
        types: &'static [&'static str],
    },
    /// Bitwise `NOT` including `BOOL`.
    Not {
        /// Required operand types.
        types: &'static [&'static str],
    },
    /// Shifts and rotates.
    Shift {
        /// Required operand types.
        types: &'static [&'static str],
    },
    /// Comparisons.
    Compare {
        /// Required operand types.
        types: &'static [&'static str],
    },
    /// `SEL`.
    Select {
        /// Required operand types.
        types: &'static [&'static str],
    },
    /// `MIN`/`MAX`.
    MinMax {
        /// Required operand types.
        types: &'static [&'static str],
    },
    /// `LIMIT`.
    Limit {
        /// Required operand types.
        types: &'static [&'static str],
    },
    /// `MUX`.
    Mux {
        /// Required operand types.
        types: &'static [&'static str],
    },
    /// `X_TO_Y` conversions; every ordered pair is required.
    Conv {
        /// Required source/target types.
        types: &'static [&'static str],
    },
    /// `LEN`/`LEN_CODE_UNIT`.
    Len {
        /// Required string kinds.
        kinds: &'static [&'static str],
    },
    /// `LEN_MAX`.
    LenMax {
        /// Required string kinds.
        kinds: &'static [&'static str],
    },
}

/// One generated vector file.
#[derive(Clone, Copy, Debug)]
pub struct FunctionSpec {
    /// The function name recorded in the file.
    pub name: &'static str,
    /// The committed file name (function name in lower case plus `.json`).
    pub file: &'static str,
    /// The required case coverage.
    pub kind: FunctionKind,
}

const BITWISE_TYPES: &[&str] = &["BYTE", "WORD", "DWORD", "LWORD", "BOOL"];
const NOT_TYPES: &[&str] = &["BYTE", "WORD", "DWORD", "LWORD", "BOOL"];
const SHIFT_TYPES: &[&str] = &["BYTE", "WORD", "DWORD", "LWORD"];
const MINMAX_TYPES: &[&str] = COMPARE_TYPES;

const fn binary(name: &'static str, file: &'static str) -> FunctionSpec {
    FunctionSpec {
        name,
        file,
        kind: FunctionKind::Binary { types: INT_TYPES },
    }
}

const fn unary(name: &'static str, file: &'static str) -> FunctionSpec {
    FunctionSpec {
        name,
        file,
        kind: FunctionKind::Unary { types: INT_TYPES },
    }
}

const fn bits(name: &'static str, file: &'static str) -> FunctionSpec {
    FunctionSpec {
        name,
        file,
        kind: FunctionKind::Bitwise {
            types: BITWISE_TYPES,
        },
    }
}

const fn shift(name: &'static str, file: &'static str) -> FunctionSpec {
    FunctionSpec {
        name,
        file,
        kind: FunctionKind::Shift { types: SHIFT_TYPES },
    }
}

const fn comparison(name: &'static str, file: &'static str) -> FunctionSpec {
    FunctionSpec {
        name,
        file,
        kind: FunctionKind::Compare {
            types: COMPARE_TYPES,
        },
    }
}

/// Every implemented exact function and its committed vector file.
pub const REGISTRY: &[FunctionSpec] = &[
    binary("ADD", "add.json"),
    binary("SUB", "sub.json"),
    binary("MUL", "mul.json"),
    binary("DIV", "div.json"),
    binary("MOD", "mod.json"),
    unary("NEG", "neg.json"),
    unary("ABS", "abs.json"),
    bits("AND", "and.json"),
    bits("OR", "or.json"),
    bits("XOR", "xor.json"),
    FunctionSpec {
        name: "NOT",
        file: "not.json",
        kind: FunctionKind::Not { types: NOT_TYPES },
    },
    shift("SHL", "shl.json"),
    shift("SHR", "shr.json"),
    shift("ROL", "rol.json"),
    shift("ROR", "ror.json"),
    comparison("EQ", "eq.json"),
    comparison("NE", "ne.json"),
    comparison("LT", "lt.json"),
    comparison("LE", "le.json"),
    comparison("GT", "gt.json"),
    comparison("GE", "ge.json"),
    FunctionSpec {
        name: "SEL",
        file: "sel.json",
        kind: FunctionKind::Select {
            types: SELECT_TYPES,
        },
    },
    FunctionSpec {
        name: "MIN",
        file: "min.json",
        kind: FunctionKind::MinMax {
            types: MINMAX_TYPES,
        },
    },
    FunctionSpec {
        name: "MAX",
        file: "max.json",
        kind: FunctionKind::MinMax {
            types: MINMAX_TYPES,
        },
    },
    FunctionSpec {
        name: "LIMIT",
        file: "limit.json",
        kind: FunctionKind::Limit {
            types: COMPARE_TYPES,
        },
    },
    FunctionSpec {
        name: "MUX",
        file: "mux.json",
        kind: FunctionKind::Mux { types: MUX_TYPES },
    },
    FunctionSpec {
        name: "CONV",
        file: "conv.json",
        kind: FunctionKind::Conv { types: CONV_TYPES },
    },
    FunctionSpec {
        name: "LEN",
        file: "len.json",
        kind: FunctionKind::Len {
            kinds: STRING_KINDS,
        },
    },
    FunctionSpec {
        name: "LEN_MAX",
        file: "len_max.json",
        kind: FunctionKind::LenMax {
            kinds: STRING_KINDS,
        },
    },
    FunctionSpec {
        name: "LEN_CODE_UNIT",
        file: "len_code_unit.json",
        kind: FunctionKind::Len {
            kinds: STRING_KINDS,
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_ranges_and_bits_round_trip() {
        for ty in [
            IntType::new(true, 8),
            IntType::new(false, 32),
            IntType::new(true, 64),
            IntType::new(false, 64),
        ] {
            assert_eq!(ty.from_bits(ty.to_bits(ty.minimum())), ty.minimum());
            assert_eq!(ty.from_bits(ty.to_bits(ty.maximum())), ty.maximum());
            assert_eq!(
                ty.from_bits(ty.to_bits(-1)),
                if ty.signed { -1 } else { ty.maximum() }
            );
        }
    }

    #[test]
    fn division_is_truncating_and_checks_zero() {
        let ty = IntType::new(true, 32);
        assert_eq!(div(ty, -7, 3), Ok(-2));
        assert_eq!(modulo(ty, -7, 3), Ok(-1));
        assert_eq!(div(ty, 7, 0), Err(Trap::DivZero));
        assert_eq!(div(ty, ty.minimum(), -1), Err(Trap::Overflow));
    }

    #[test]
    fn shifts_and_rotates_mask_their_counts() {
        let ty = IntType::new(false, 8);
        assert_eq!(shl(ty, 0x55, -1), 0x80);
        assert_eq!(shr(ty, 0x80, -1), 0x01);
        assert_eq!(rol(ty, 0x81, 1), 0x03);
        assert_eq!(ror(ty, 0x81, 1), 0xC0);
        assert_eq!(rol(ty, 0x5A, 8), 0x5A);
    }

    #[test]
    fn conversions_follow_the_codegen_rules() {
        let dint = IntType::new(true, 32);
        let udint = IntType::new(false, 32);
        let sint = IntType::new(true, 8);
        let uint = IntType::new(false, 16);
        assert_eq!(convert(dint, udint, -1), Ok(4_294_967_295));
        assert_eq!(convert(udint, dint, 4_294_967_295), Ok(-1));
        assert_eq!(convert(sint, uint, -1), Ok(65_535));
        assert_eq!(convert(dint, sint, 300), Err(Trap::Range));
        assert_eq!(convert(dint, IntType::BOOL, -5), Ok(1));
        assert_eq!(convert(dint, IntType::BOOL, 0), Ok(0));
    }

    #[test]
    fn string_lengths_use_the_documented_units() {
        assert_eq!(len(StringKind::Unicode, "a€😀z"), 4);
        assert_eq!(len_code_unit(StringKind::Unicode, "a€😀z"), 1 + 3 + 4 + 1);
        assert_eq!(len(StringKind::Dbcs, "😀"), 2);
        assert_eq!(len_code_unit(StringKind::Dbcs, "😀"), 2);
        assert_eq!(len(StringKind::Sbcs, "hello"), 5);
    }
}
