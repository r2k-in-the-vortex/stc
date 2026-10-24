//! 128-bit persistent identities and 256-bit artifact digests.

use std::fmt;
use std::str::FromStr;

use crate::error::Error;
use crate::sha256::domain_hash;

/// The domain separator for `GotId` derivation.
pub const GOTID_DOMAIN: &str = "stc:gotid:v1";

/// A persistent 128-bit GOT identity.
///
/// The byte order is the digest byte order. `Ord` therefore matches the
/// ascending-bytes order used by slot assignment and manifest tables, which is
/// also the ascending `0x`-hex order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GotId([u8; 16]);

impl GotId {
    /// The all-zero identity, used only for import slots and absent fields.
    pub const ZERO: Self = Self([0; 16]);

    /// Creates an identity from raw bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Returns `true` for the all-zero identity.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0 == [0; 16]
    }

    /// Renders the identity as `0x` followed by 32 lowercase hex digits.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(34);
        out.push_str("0x");
        for byte in self.0 {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }

    /// Renders the identity without the `0x` prefix (file-name form).
    #[must_use]
    pub fn to_hex_plain(self) -> String {
        let mut out = String::with_capacity(32);
        for byte in self.0 {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }

    /// Parses the strict `0x`-prefixed form.
    ///
    /// # Errors
    ///
    /// Rejects a missing prefix, wrong length, or non-hex characters.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let bytes = parse_hex(text, 16, "GotId")?;
        let mut array = [0_u8; 16];
        array.copy_from_slice(&bytes);
        Ok(Self(array))
    }

    /// Derives an identity from its kind tag and tuple.
    ///
    /// Returns both the truncated identity and the full SHA-256 digest so a
    /// build can detect truncation collisions.
    #[must_use]
    pub fn derive(kind: u8, package: &str, path: &str, qualifier: &str) -> (Self, Digest32) {
        let mut input = Vec::new();
        input.push(kind);
        encode_string(&mut input, package);
        encode_string(&mut input, path);
        encode_string(&mut input, qualifier);
        let digest = domain_hash(GOTID_DOMAIN, &[&input]);
        let mut truncated = [0_u8; 16];
        truncated.copy_from_slice(&digest[..16]);
        (Self(truncated), Digest32(digest))
    }
}

impl fmt::Debug for GotId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl fmt::Display for GotId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl FromStr for GotId {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

/// A 256-bit SHA-256 digest.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest32([u8; 32]);

impl Digest32 {
    /// The all-zero digest.
    pub const ZERO: Self = Self([0; 32]);

    /// Creates a digest from raw bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns `true` for the all-zero digest.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0 == [0; 32]
    }

    /// Renders the digest as `0x` followed by 64 lowercase hex digits.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(66);
        out.push_str("0x");
        for byte in self.0 {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }

    /// Parses the strict `0x`-prefixed form.
    ///
    /// # Errors
    ///
    /// Rejects a missing prefix, wrong length, or non-hex characters.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let bytes = parse_hex(text, 32, "digest")?;
        let mut array = [0_u8; 32];
        array.copy_from_slice(&bytes);
        Ok(Self(array))
    }
}

impl fmt::Debug for Digest32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl fmt::Display for Digest32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl FromStr for Digest32 {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

fn parse_hex(text: &str, length: usize, what: &str) -> Result<Vec<u8>, Error> {
    let Some(hex) = text.strip_prefix("0x") else {
        return Err(Error::manifest(format!(
            "{what} `{text}` is missing the `0x` prefix"
        )));
    };
    if hex.len() != length * 2 {
        return Err(Error::manifest(format!(
            "{what} `{text}` must contain exactly {} hexadecimal digits",
            length * 2
        )));
    }
    let mut bytes = Vec::with_capacity(length);
    let raw = hex.as_bytes();
    for index in (0..raw.len()).step_by(2) {
        let high = hex_value(raw[index])?;
        let low = hex_value(raw[index + 1])?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn hex_value(byte: u8) -> Result<u8, Error> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(Error::manifest(
            "hexadecimal identifiers use lowercase `0-9a-f` only",
        )),
    }
}

/// Appends the primitive `string` encoding (u32 length plus UTF-8 bytes).
fn encode_string(out: &mut Vec<u8>, text: &str) {
    let Ok(length) = u32::try_from(text.len()) else {
        // A string longer than 4 GiB cannot occur in a compiler artifact; the
        // caller validates the same bound during wire encoding.
        out.extend_from_slice(&u32::MAX.to_le_bytes());
        return;
    };
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_and_parses_the_strict_hex_form() {
        let id = GotId::from_bytes([
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ]);
        let text = "0x00112233445566778899aabbccddeeff";
        assert_eq!(id.to_hex(), text);
        assert_eq!(GotId::parse(text).unwrap(), id);
        assert!(GotId::parse("00112233445566778899aabbccddeeff").is_err());
        assert!(GotId::parse("0x00112233445566778899AABBCCDDEEFF").is_err());
        assert!(GotId::parse("0x0011").is_err());
    }

    #[test]
    fn derivation_is_domain_separated_and_stable() {
        let (first, full) = GotId::derive(1, "_ROOT", "COUNTER", "");
        let (second, full_again) = GotId::derive(1, "_ROOT", "COUNTER", "");
        assert_eq!(first, second);
        assert_eq!(full, full_again);
        assert_eq!(&full.as_bytes()[..16], first.as_bytes());
        let (other, _) = GotId::derive(3, "_ROOT", "COUNTER", "");
        assert_ne!(first, other);
    }
}
