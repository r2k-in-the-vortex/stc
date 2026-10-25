//! Scalar type mapping, sizes, process-image addresses, and stable hashing.

use stc_hir::{
    CharTy, DateTy, DirectAddress, DirectArea, DirectSize, HirCrate, IntTy, RealTy, TimeTy, Ty,
    TyId,
};
use stc_span::Span;

use crate::CodegenError;
use crate::codes;

/// The M4 scalar categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScalarKind {
    /// `BOOL`.
    Bool,
    /// A signed or unsigned integer.
    Int {
        /// Signedness.
        signed: bool,
        /// Width in bits.
        width: u16,
    },
    /// `REAL` or `LREAL`.
    Real {
        /// Width in bits (32 or 64).
        width: u16,
    },
    /// `TIME`/`LTIME` (signed nanoseconds).
    Time,
    /// A date/time category.
    Date(DateTy),
    /// `CHAR`/`WCHAR`/`UCHAR` (unsigned scalar).
    Char {
        /// Width in bits.
        width: u16,
    },
}

impl ScalarKind {
    /// Returns the in-memory LLVM type.
    #[must_use]
    pub fn storage(self) -> crate::ir::IrType {
        use crate::ir::IrType;
        match self {
            Self::Bool => IrType::Int(8),
            Self::Int { width, .. } | Self::Char { width } => IrType::Int(width),
            Self::Real { width: 32 } => IrType::Float,
            Self::Real { .. } => IrType::Double,
            Self::Time | Self::Date(_) => IrType::Int(64),
        }
    }

    /// Returns the storage size in bytes.
    #[must_use]
    pub fn size_bytes(self) -> u64 {
        match self {
            Self::Bool => 1,
            Self::Int { width, .. } | Self::Char { width } => u64::from(width / 8),
            Self::Real { width } => u64::from(width / 8),
            Self::Time | Self::Date(_) => 8,
        }
    }

    /// Returns the process-image occupancy in bits (`BOOL` is one bit).
    #[must_use]
    pub fn occupancy_bits(self) -> u64 {
        match self {
            Self::Bool => 1,
            other => other.size_bytes() * 8,
        }
    }

    /// Returns `true` when arithmetic and comparisons use signed predicates.
    #[must_use]
    pub fn is_signed(self) -> bool {
        match self {
            Self::Int { signed, .. } => signed,
            Self::Time | Self::Date(_) => true,
            Self::Bool | Self::Real { .. } | Self::Char { .. } => false,
        }
    }

    /// Returns the canonical uppercase type name used in signatures and hashes.
    #[must_use]
    pub fn canonical_name(self) -> String {
        match self {
            Self::Bool => "BOOL".into(),
            Self::Int {
                signed: true,
                width: 8,
            } => "SINT".into(),
            Self::Int {
                signed: true,
                width: 16,
            } => "INT".into(),
            Self::Int {
                signed: true,
                width: 32,
            } => "DINT".into(),
            Self::Int {
                signed: true,
                width: 64,
            } => "LINT".into(),
            Self::Int {
                signed: false,
                width: 8,
            } => "USINT".into(),
            Self::Int {
                signed: false,
                width: 16,
            } => "UINT".into(),
            Self::Int {
                signed: false,
                width: 32,
            } => "UDINT".into(),
            Self::Int {
                signed: false,
                width: 64,
            } => "ULINT".into(),
            Self::Int { width, .. } => format!("INT{width}"),
            Self::Real { width: 32 } => "REAL".into(),
            Self::Real { .. } => "LREAL".into(),
            Self::Time => "TIME".into(),
            Self::Date(DateTy::Date) => "DATE".into(),
            Self::Date(DateTy::LDate) => "LDATE".into(),
            Self::Date(DateTy::Tod) => "TOD".into(),
            Self::Date(DateTy::LTod) => "LTOD".into(),
            Self::Date(DateTy::DateTime) => "DT".into(),
            Self::Date(DateTy::LDateTime) => "LDT".into(),
            Self::Char { width: 8 } => "CHAR".into(),
            Self::Char { width: 16 } => "WCHAR".into(),
            Self::Char { .. } => "UCHAR".into(),
        }
    }
}

/// Maps a canonical HIR type to an M4 scalar kind.
///
/// # Errors
///
/// Returns `STC-E-CODEGEN-001` for types deferred to M5.
pub fn scalar_kind(hir: &HirCrate, ty: TyId) -> Result<ScalarKind, CodegenError> {
    match hir.types.get(ty) {
        Ty::Bool => Ok(ScalarKind::Bool),
        Ty::Int(IntTy { signed, width }) => match width {
            8 | 16 | 32 | 64 => Ok(ScalarKind::Int {
                signed: *signed,
                width: u16::from(*width),
            }),
            _ => Err(unsupported(
                ty,
                format!("integer width {width} is not supported in M4"),
            )),
        },
        Ty::BitString(width) => match width {
            8 | 16 | 32 | 64 => Ok(ScalarKind::Int {
                signed: false,
                width: u16::from(*width),
            }),
            _ => Err(unsupported(
                ty,
                format!("bit-string width {width} is not supported in M4"),
            )),
        },
        Ty::Real(RealTy::Real) => Ok(ScalarKind::Real { width: 32 }),
        Ty::Real(RealTy::LReal) => Ok(ScalarKind::Real { width: 64 }),
        Ty::Time(TimeTy::Time | TimeTy::LTime) => Ok(ScalarKind::Time),
        Ty::Date(kind) => Ok(ScalarKind::Date(*kind)),
        Ty::Char(CharTy::Char) => Ok(ScalarKind::Char { width: 8 }),
        Ty::Char(CharTy::WChar) => Ok(ScalarKind::Char { width: 16 }),
        Ty::Char(CharTy::UChar) => Ok(ScalarKind::Char { width: 32 }),
        Ty::Error => Err(CodegenError::Internal {
            message: "error type reached codegen".into(),
        }),
        other => Err(CodegenError::Unsupported {
            code: codes::CODEGEN_UNSUPPORTED,
            message: format!(
                "type `{}` is not supported in M4",
                describe_type(hir, other)
            ),
            span: stc_span::Span::empty(stc_span::FileId::new(0), 0),
        }),
    }
}

/// Maps a type at a source location, attaching `span` to unsupported errors.
///
/// # Errors
///
/// Returns `STC-E-CODEGEN-001` for types deferred to M5.
pub fn scalar_kind_at(hir: &HirCrate, ty: TyId, span: Span) -> Result<ScalarKind, CodegenError> {
    scalar_kind(hir, ty).map_err(|error| error.with_span(span))
}

fn describe_type(hir: &HirCrate, ty: &Ty) -> String {
    match ty {
        Ty::String(string) => format!("{:?}[{}]", string.kind, string.capacity),
        Ty::Array(_) => "ARRAY".into(),
        Ty::Ref(_) => "REF_TO".into(),
        Ty::Named(def) => hir.def_name(*def).to_string(),
        Ty::Null => "NULL".into(),
        Ty::Generic(generic) => generic.as_str().into(),
        other => format!("{other:?}"),
    }
}

fn unsupported(ty: TyId, message: String) -> CodegenError {
    let _ = ty;
    CodegenError::Unsupported {
        code: codes::CODEGEN_UNSUPPORTED,
        message,
        span: Span::empty(stc_span::FileId::new(0), 0),
    }
}

/// One resolved process-image placement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageSpan {
    /// Area.
    pub area: DirectArea,
    /// Absolute start bit within the area.
    pub start_bit: u64,
    /// Occupied bits.
    pub width_bits: u64,
}

/// Resolves a direct address to an area-relative bit range.
///
/// # Errors
///
/// Returns `STC-E-CODEGEN-001` for wildcard or area-less addresses.
pub fn image_span(address: &DirectAddress, kind: ScalarKind) -> Result<ImageSpan, CodegenError> {
    let Some(area) = address.area else {
        return Err(CodegenError::Unsupported {
            code: codes::CODEGEN_UNSUPPORTED,
            message: "a direct variable without an area cannot be laid out".into(),
            span: address.span,
        });
    };
    if address.wildcard {
        return Err(CodegenError::Unsupported {
            code: codes::CODEGEN_UNSUPPORTED,
            message: "a wildcard direct-variable address cannot be laid out".into(),
            span: address.span,
        });
    }
    let start_bit = address_start_bit(address)?;
    Ok(ImageSpan {
        area,
        start_bit,
        width_bits: kind.occupancy_bits(),
    })
}

fn address_start_bit(address: &DirectAddress) -> Result<u64, CodegenError> {
    let indices = &address.indices;
    let (byte, bit) = match (address.size, indices.as_slice()) {
        (Some(DirectSize::Bit), &[byte, bit]) => (u64::from(byte), u64::from(bit)),
        (Some(DirectSize::Byte), &[byte]) => (u64::from(byte), 0),
        (Some(DirectSize::Word), &[word]) => (u64::from(word) * 2, 0),
        (Some(DirectSize::DWord), &[word]) => (u64::from(word) * 4, 0),
        (Some(DirectSize::LWord), &[word]) => (u64::from(word) * 8, 0),
        (None, &[byte, bit]) => (u64::from(byte), u64::from(bit)),
        (None, &[byte]) => (u64::from(byte), 0),
        _ => {
            return Err(CodegenError::Unsupported {
                code: codes::CODEGEN_UNSUPPORTED,
                message: "malformed direct-variable address".into(),
                span: address.span,
            });
        }
    };
    Ok(byte * 8 + bit)
}

/// Renders the metadata spelling of a direct address (without `%`).
#[must_use]
pub fn render_address(address: &DirectAddress) -> String {
    let mut out = String::new();
    if let Some(area) = address.area {
        out.push(area.as_char());
    }
    if let Some(size) = address.size {
        out.push(size.as_char());
    }
    for (index, value) in address.indices.iter().enumerate() {
        if index > 0 {
            out.push('.');
        }
        out.push_str(&value.to_string());
    }
    out
}

/// FNV-1a 64-bit offset basis.
pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64-bit prime.
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Continues an FNV-1a 64 hash with `bytes`.
#[must_use]
pub fn fnv1a_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Hashes `bytes` with FNV-1a 64.
#[must_use]
pub fn fnv1a(bytes: &[u8]) -> u64 {
    fnv1a_update(FNV_OFFSET, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_matches_the_reference_vector() {
        assert_eq!(fnv1a(b""), FNV_OFFSET);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn bit_addresses_resolve_to_absolute_bits() {
        let address = DirectAddress {
            area: Some(DirectArea::Input),
            size: Some(DirectSize::Bit),
            wildcard: false,
            indices: vec![2, 3],
            span: Span::empty(stc_span::FileId::new(0), 0),
        };
        assert_eq!(address_start_bit(&address).unwrap(), 19);
    }
}
