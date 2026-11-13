//! Process-image payload encodings for MQTT channels (WL-15 phase 1).
//!
//! WL-15 Q4 fixes six encodings over a scalar channel: `bool`, `uint`, `int`,
//! `float`, `string`, and `raw`. Values travel through the process image as a
//! `u64` bit pattern, so `float` values are the IEEE-754 bit pattern of the
//! channel width and `int` values are two's-complement bits. Numeric and raw
//! encodings are big-endian by default; a per-channel byte order may reverse
//! them. `string` is the UTF-8 decimal (or `true`/`false`) text of the value;
//! a value whose sign bit is set is written as its signed decimal form, which
//! decodes back to the same bit pattern.
//!
//! JSON object payloads and aggregates are not part of WL-15 (Q14, option b);
//! they are recorded as FutureWishlist WL-22.
//!
//! Every decode validates the exact payload length, the channel width, and the
//! value range before it reaches the image.

use super::{ErrorKind, MqttError};

/// One supported payload encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// One byte, `0` or `1`, for a one-bit channel.
    Bool,
    /// Unsigned integer of 8, 16, 32, or 64 bits.
    Uint,
    /// Two's-complement signed integer of 8, 16, 32, or 64 bits.
    Int,
    /// IEEE-754 binary32 or binary64 bit pattern.
    Float,
    /// UTF-8 text of the scalar value.
    String,
    /// Opaque bytes of exactly the channel width.
    Raw,
}

impl Encoding {
    /// Parses the profile spelling.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "bool" => Some(Self::Bool),
            "uint" => Some(Self::Uint),
            "int" => Some(Self::Int),
            "float" => Some(Self::Float),
            "string" => Some(Self::String),
            "raw" => Some(Self::Raw),
            _ => None,
        }
    }

    /// Returns the profile spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Uint => "uint",
            Self::Int => "int",
            Self::Float => "float",
            Self::String => "string",
            Self::Raw => "raw",
        }
    }
}

/// Byte order of a numeric or raw payload.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ByteOrder {
    /// Most significant byte first (the default).
    #[default]
    Big,
    /// Least significant byte first.
    Little,
}

impl ByteOrder {
    /// Parses the profile spelling.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "big" | "big-endian" => Some(Self::Big),
            "little" | "little-endian" => Some(Self::Little),
            _ => None,
        }
    }

    /// Returns the profile spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Big => "big",
            Self::Little => "little",
        }
    }
}

/// Encodes one process-image value into a payload.
///
/// Returns an error for an encoding/width pair that the contract does not
/// allow, and for an unsigned value outside its width.
pub fn encode(
    encoding: Encoding,
    width_bits: u32,
    order: ByteOrder,
    value: u64,
) -> Result<Vec<u8>, MqttError> {
    match encoding {
        Encoding::Bool => {
            require_width(encoding, width_bits, &[1])?;
            Ok(vec![u8::from(value & 1 != 0)])
        }
        Encoding::Uint | Encoding::Int => {
            require_width(encoding, width_bits, &[8, 16, 32, 64])?;
            if encoding == Encoding::Uint && width_bits < 64 && value >> width_bits != 0 {
                return Err(MqttError::new(
                    ErrorKind::Protocol,
                    format!("value does not fit the {width_bits}-bit unsigned channel"),
                ));
            }
            let bytes = width_bits / 8;
            let full = (value & mask(width_bits)).to_be_bytes();
            Ok(ordered(&full[8 - bytes as usize..], order))
        }
        Encoding::Float => {
            require_width(encoding, width_bits, &[32, 64])?;
            if width_bits == 32 {
                let bits = u32::try_from(value & u64::from(u32::MAX))
                    .expect("masked 32-bit value always fits");
                Ok(ordered(&bits.to_be_bytes(), order))
            } else {
                Ok(ordered(&value.to_be_bytes(), order))
            }
        }
        Encoding::String => {
            if width_bits == 0 || width_bits > 64 {
                return Err(width_error(encoding, width_bits));
            }
            Ok(match width_bits {
                1 => {
                    if value > 1 {
                        return Err(MqttError::new(
                            ErrorKind::Protocol,
                            "bool text payload requires a 0 or 1 bit value",
                        ));
                    }
                    if value == 0 { "false".to_owned() } else { "true".to_owned() }
                }
                8 | 16 | 32 | 64 => signed_or_unsigned_text(width_bits, value),
                _ => {
                    return Err(MqttError::new(
                        ErrorKind::Unsupported,
                        format!(
                            "string payload for a {width_bits}-bit channel has no defined text form"
                        ),
                    ));
                }
            }
            .into_bytes())
        }
        Encoding::Raw => {
            if width_bits == 0 || !width_bits.is_multiple_of(8) || width_bits > 64 {
                return Err(width_error(encoding, width_bits));
            }
            let bytes = width_bits / 8;
            let full = value.to_be_bytes();
            Ok(ordered(&full[8 - bytes as usize..], order))
        }
    }
}

/// Decodes one payload into a process-image value bit pattern.
///
/// The payload must be exactly the encoded length for the width, and a textual
/// value must be valid UTF-8 in range for the channel.
pub fn decode(
    encoding: Encoding,
    width_bits: u32,
    order: ByteOrder,
    bytes: &[u8],
) -> Result<u64, MqttError> {
    match encoding {
        Encoding::Bool => {
            require_width(encoding, width_bits, &[1])?;
            match bytes {
                [0] => Ok(0),
                [1] => Ok(1),
                [other] => Err(MqttError::new(
                    ErrorKind::Protocol,
                    format!("bool payload byte {other} is not 0 or 1"),
                )),
                _ => Err(length_error(encoding, width_bits, bytes.len())),
            }
        }
        Encoding::Uint | Encoding::Int => {
            require_width(encoding, width_bits, &[8, 16, 32, 64])?;
            let expected = (width_bits / 8) as usize;
            if bytes.len() != expected {
                return Err(length_error(encoding, width_bits, bytes.len()));
            }
            let mut full = [0_u8; 8];
            full[8 - expected..].copy_from_slice(&unorder(bytes, order));
            Ok(u64::from_be_bytes(full))
        }
        Encoding::Float => {
            require_width(encoding, width_bits, &[32, 64])?;
            let expected = (width_bits / 8) as usize;
            if bytes.len() != expected {
                return Err(length_error(encoding, width_bits, bytes.len()));
            }
            let value = unorder(bytes, order);
            let mut full = [0_u8; 8];
            full[8 - expected..].copy_from_slice(&value);
            Ok(u64::from_be_bytes(full))
        }
        Encoding::String => {
            if !matches!(width_bits, 1 | 8 | 16 | 32 | 64) {
                return Err(width_error(encoding, width_bits));
            }
            let text = std::str::from_utf8(bytes).map_err(|_| {
                MqttError::new(ErrorKind::Malformed, "string payload is not valid UTF-8")
            })?;
            decode_text(width_bits, text)
        }
        Encoding::Raw => {
            if width_bits == 0 || !width_bits.is_multiple_of(8) || width_bits > 64 {
                return Err(width_error(encoding, width_bits));
            }
            let expected = (width_bits / 8) as usize;
            if bytes.len() != expected {
                return Err(length_error(encoding, width_bits, bytes.len()));
            }
            let mut full = [0_u8; 8];
            full[8 - expected..].copy_from_slice(&unorder(bytes, order));
            Ok(u64::from_be_bytes(full))
        }
    }
}

fn decode_text(width_bits: u32, text: &str) -> Result<u64, MqttError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(MqttError::new(ErrorKind::Malformed, "empty string payload"));
    }
    if width_bits == 1 {
        return match text {
            "true" | "1" => Ok(1),
            "false" | "0" => Ok(0),
            _ => Err(MqttError::new(
                ErrorKind::Malformed,
                "bool string payload must be true/false or 1/0",
            )),
        };
    }
    let parsed: i128 = text.parse().map_err(|_| {
        MqttError::new(
            ErrorKind::Malformed,
            format!("string payload is not an integer: {text}"),
        )
    })?;
    wrap(width_bits, parsed)
}

fn signed_or_unsigned_text(width_bits: u32, value: u64) -> String {
    // Both integer encodings share the same bit pattern. The caller's channel
    // direction and declared type decide the interpretation; for text we use
    // the signed form when the sign bit is set, because that is the only
    // spelling that round-trips through `decode`.
    let sign_bit = 1_u64 << (width_bits - 1);
    if value & sign_bit == 0 {
        return value.to_string();
    }
    let signed = sign_extend(width_bits, value);
    if signed < 0 {
        signed.to_string()
    } else {
        value.to_string()
    }
}

fn sign_extend(width_bits: u32, value: u64) -> i128 {
    let shift = 64 - width_bits;
    i128::from(((value << shift) as i64) >> shift)
}

fn wrap(width_bits: u32, value: i128) -> Result<u64, MqttError> {
    let (minimum, maximum) = match width_bits {
        8 => (-128, 127),
        16 => (-32_768, 32_767),
        32 => (i128::from(i32::MIN), i128::from(i32::MAX)),
        64 => (i128::from(i64::MIN), i128::from(i64::MAX)),
        _ => {
            return Err(MqttError::new(
                ErrorKind::Unsupported,
                format!("no integer range for a {width_bits}-bit channel"),
            ));
        }
    };
    if value < minimum || value > maximum {
        return Err(MqttError::new(
            ErrorKind::Protocol,
            format!("string payload value {value} does not fit a {width_bits}-bit channel"),
        ));
    }
    Ok((value as i64 as u64) & mask(width_bits))
}

fn mask(width_bits: u32) -> u64 {
    if width_bits >= 64 {
        u64::MAX
    } else {
        (1_u64 << width_bits) - 1
    }
}

fn ordered(bytes: &[u8], order: ByteOrder) -> Vec<u8> {
    match order {
        ByteOrder::Big => bytes.to_vec(),
        ByteOrder::Little => bytes.iter().rev().copied().collect(),
    }
}

fn unorder(bytes: &[u8], order: ByteOrder) -> Vec<u8> {
    ordered(bytes, order)
}

fn require_width(encoding: Encoding, width_bits: u32, allowed: &[u32]) -> Result<(), MqttError> {
    if allowed.contains(&width_bits) {
        Ok(())
    } else {
        Err(width_error(encoding, width_bits))
    }
}

fn width_error(encoding: Encoding, width_bits: u32) -> MqttError {
    MqttError::new(
        ErrorKind::Unsupported,
        format!(
            "encoding `{}` is not defined for a {width_bits}-bit channel",
            encoding.as_str()
        ),
    )
}

fn length_error(encoding: Encoding, width_bits: u32, got: usize) -> MqttError {
    MqttError::new(
        ErrorKind::Protocol,
        format!(
            "encoding `{}` on a {width_bits}-bit channel expects {} bytes, got {got}",
            encoding.as_str(),
            width_bits / 8
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings_round_trip_with_stable_spellings() {
        for (encoding, spelling) in [
            (Encoding::Bool, "bool"),
            (Encoding::Uint, "uint"),
            (Encoding::Int, "int"),
            (Encoding::Float, "float"),
            (Encoding::String, "string"),
            (Encoding::Raw, "raw"),
        ] {
            assert_eq!(encoding.as_str(), spelling);
            assert_eq!(Encoding::parse(spelling), Some(encoding));
        }
        assert_eq!(Encoding::parse("json"), None);
        assert_eq!(ByteOrder::parse("big"), Some(ByteOrder::Big));
        assert_eq!(ByteOrder::parse("little-endian"), Some(ByteOrder::Little));
        assert_eq!(ByteOrder::parse("middle"), None);
        assert_eq!(ByteOrder::default(), ByteOrder::Big);
    }

    #[test]
    fn bool_payloads_are_one_byte() {
        assert_eq!(
            encode(Encoding::Bool, 1, ByteOrder::Big, 1).unwrap(),
            vec![1]
        );
        assert_eq!(
            encode(Encoding::Bool, 1, ByteOrder::Big, 0).unwrap(),
            vec![0]
        );
        assert_eq!(decode(Encoding::Bool, 1, ByteOrder::Big, &[1]).unwrap(), 1);
        assert_eq!(decode(Encoding::Bool, 1, ByteOrder::Big, &[0]).unwrap(), 0);
        assert!(decode(Encoding::Bool, 1, ByteOrder::Big, &[2]).is_err());
        assert!(decode(Encoding::Bool, 1, ByteOrder::Big, &[]).is_err());
        assert!(encode(Encoding::Bool, 8, ByteOrder::Big, 1).is_err());
    }

    #[test]
    fn unsigned_integers_respect_width_and_byte_order() {
        assert_eq!(
            encode(Encoding::Uint, 16, ByteOrder::Big, 0xBEEF).unwrap(),
            vec![0xBE, 0xEF]
        );
        assert_eq!(
            encode(Encoding::Uint, 16, ByteOrder::Little, 0xBEEF).unwrap(),
            vec![0xEF, 0xBE]
        );
        assert_eq!(
            decode(Encoding::Uint, 16, ByteOrder::Big, &[0xBE, 0xEF]).unwrap(),
            0xBEEF
        );
        assert_eq!(
            decode(Encoding::Uint, 16, ByteOrder::Little, &[0xEF, 0xBE]).unwrap(),
            0xBEEF
        );
        assert!(encode(Encoding::Uint, 8, ByteOrder::Big, 256).is_err());
        assert!(decode(Encoding::Uint, 16, ByteOrder::Big, &[1]).is_err());
    }

    #[test]
    fn signed_integers_keep_twos_complement_bits() {
        assert_eq!(
            encode(Encoding::Int, 8, ByteOrder::Big, (-1_i64) as u64).unwrap(),
            vec![0xFF]
        );
        assert_eq!(
            decode(Encoding::Int, 8, ByteOrder::Big, &[0xFF]).unwrap(),
            0xFF
        );
        assert_eq!(
            encode(Encoding::Int, 32, ByteOrder::Big, 0x8000_0000).unwrap(),
            vec![0x80, 0x00, 0x00, 0x00]
        );
        assert_eq!(
            decode(Encoding::Int, 32, ByteOrder::Little, &[0x00, 0x00, 0x00, 0x80]).unwrap(),
            0x8000_0000
        );
    }

    #[test]
    fn floats_are_ieee_bit_patterns() {
        let bits = u64::from(1.5_f32.to_bits());
        let payload = encode(Encoding::Float, 32, ByteOrder::Big, bits).unwrap();
        assert_eq!(payload, 1.5_f32.to_be_bytes().to_vec());
        assert_eq!(
            decode(Encoding::Float, 32, ByteOrder::Big, &payload).unwrap(),
            bits
        );
        let bits64 = 2.5_f64.to_bits();
        let payload64 = encode(Encoding::Float, 64, ByteOrder::Big, bits64).unwrap();
        assert_eq!(payload64, 2.5_f64.to_be_bytes().to_vec());
        assert_eq!(
            decode(Encoding::Float, 64, ByteOrder::Big, &payload64).unwrap(),
            bits64
        );
        assert!(encode(Encoding::Float, 16, ByteOrder::Big, 0).is_err());
    }

    #[test]
    fn string_payloads_round_trip_scalars() {
        let payload = encode(Encoding::String, 16, ByteOrder::Big, 1234).unwrap();
        assert_eq!(payload, b"1234".to_vec());
        assert_eq!(
            decode(Encoding::String, 16, ByteOrder::Big, &payload).unwrap(),
            1234
        );
        let negative = encode(Encoding::String, 16, ByteOrder::Big, (-2_i64) as u64).unwrap();
        assert_eq!(negative, b"-2".to_vec());
        assert_eq!(
            decode(Encoding::String, 16, ByteOrder::Big, &negative).unwrap(),
            0xFFFE
        );
        assert_eq!(
            decode(Encoding::String, 1, ByteOrder::Big, b"true").unwrap(),
            1
        );
        assert_eq!(
            decode(Encoding::String, 1, ByteOrder::Big, b"0").unwrap(),
            0
        );
        let float = encode(Encoding::String, 32, ByteOrder::Big, u64::from(1.5_f32.to_bits()))
            .unwrap();
        assert_eq!(
            decode(Encoding::String, 32, ByteOrder::Big, &float).unwrap(),
            u64::from(1.5_f32.to_bits())
        );
    }

    #[test]
    fn string_payloads_reject_bad_text_and_ranges() {
        assert!(decode(Encoding::String, 16, ByteOrder::Big, b"abc").is_err());
        assert!(decode(Encoding::String, 16, ByteOrder::Big, b"").is_err());
        assert!(decode(Encoding::String, 16, ByteOrder::Big, &[0xFF, 0xFE]).is_err());
        assert!(decode(Encoding::String, 8, ByteOrder::Big, b"256").is_err());
        assert!(decode(Encoding::String, 1, ByteOrder::Big, b"yes").is_err());
        assert!(decode(Encoding::String, 32, ByteOrder::Big, b"nan").is_err());
        assert!(decode(Encoding::String, 24, ByteOrder::Big, b"1").is_err());
    }

    #[test]
    fn raw_payloads_are_exactly_the_channel_width() {
        assert_eq!(
            encode(Encoding::Raw, 16, ByteOrder::Big, 0x1234).unwrap(),
            vec![0x12, 0x34]
        );
        assert_eq!(
            decode(Encoding::Raw, 16, ByteOrder::Big, &[0x12, 0x34]).unwrap(),
            0x1234
        );
        assert!(encode(Encoding::Raw, 12, ByteOrder::Big, 0).is_err());
        assert!(decode(Encoding::Raw, 16, ByteOrder::Big, &[0x12]).is_err());
    }

    #[test]
    fn unsupported_widths_are_rejected_not_truncated() {
        for encoding in [Encoding::Uint, Encoding::Int, Encoding::Float] {
            assert!(encode(encoding, 24, ByteOrder::Big, 0).is_err());
            assert!(decode(encoding, 24, ByteOrder::Big, &[0, 0, 0]).is_err());
        }
    }
}
