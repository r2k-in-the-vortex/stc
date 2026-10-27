//! Primitive byte-packed encoding from GOT Artifacts §2.1.

use crate::digest::{Digest32, GotId};
use crate::error::{Error, Result};

/// A little-endian, byte-packed artifact encoder.
#[derive(Clone, Debug, Default)]
pub struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    /// Creates an empty encoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an encoder with pre-allocated capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
        }
    }

    /// Returns the encoded bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the encoder and returns the bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Returns the number of bytes written.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns `true` when nothing has been written.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Writes one byte.
    pub fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    /// Writes a little-endian `u16`.
    pub fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Writes a little-endian `u32`.
    pub fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Writes a little-endian `u64`.
    pub fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Writes a little-endian `i64`.
    pub fn i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Writes a little-endian `i128`.
    pub fn i128(&mut self, value: i128) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Writes the strict boolean encoding.
    pub fn bool(&mut self, value: bool) {
        self.bytes.push(u8::from(value));
    }

    /// Writes an enum tag.
    pub fn tag(&mut self, value: u8) {
        self.bytes.push(value);
    }

    /// Writes a 128-bit identifier or digest-like value.
    pub fn id128(&mut self, value: GotId) {
        self.bytes.extend_from_slice(value.as_bytes());
    }

    /// Writes a 256-bit digest.
    pub fn digest32(&mut self, value: Digest32) {
        self.bytes.extend_from_slice(value.as_bytes());
    }

    /// Writes a length-prefixed UTF-8 string.
    ///
    /// # Errors
    ///
    /// Rejects a string longer than `u32::MAX` bytes.
    pub fn string(&mut self, value: &str) -> Result<()> {
        let length = u32::try_from(value.len())
            .map_err(|_| Error::manifest("string exceeds the u32 length limit"))?;
        self.u32(length);
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }

    /// Writes a list header.
    ///
    /// # Errors
    ///
    /// Rejects more than `u32::MAX` elements.
    pub fn list_header(&mut self, count: usize) -> Result<()> {
        let count = u32::try_from(count)
            .map_err(|_| Error::manifest("list exceeds the u32 element limit"))?;
        self.u32(count);
        Ok(())
    }

    /// Writes raw bytes.
    pub fn raw(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }
}

/// A bounds-checked artifact decoder.
#[derive(Clone, Debug)]
pub struct Decoder<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Decoder<'a> {
    /// Creates a decoder over `data`.
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    /// Returns the current byte position.
    #[must_use]
    pub fn position(&self) -> usize {
        self.position
    }

    /// Returns the number of unread bytes.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.data.len() - self.position
    }

    /// Returns `true` when every byte has been consumed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// Reads one byte.
    ///
    /// # Errors
    ///
    /// Rejects truncated input.
    pub fn u8(&mut self) -> Result<u8> {
        let bytes = self.take(1)?;
        Ok(bytes[0])
    }

    /// Reads a little-endian `u16`.
    ///
    /// # Errors
    ///
    /// Rejects truncated input.
    pub fn u16(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    /// Reads a little-endian `u32`.
    ///
    /// # Errors
    ///
    /// Rejects truncated input.
    pub fn u32(&mut self) -> Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Reads a little-endian `u64`.
    ///
    /// # Errors
    ///
    /// Rejects truncated input.
    pub fn u64(&mut self) -> Result<u64> {
        let bytes = self.take(8)?;
        let mut array = [0_u8; 8];
        array.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(array))
    }

    /// Reads a little-endian `i64`.
    ///
    /// # Errors
    ///
    /// Rejects truncated input.
    pub fn i64(&mut self) -> Result<i64> {
        let bytes = self.take(8)?;
        let mut array = [0_u8; 8];
        array.copy_from_slice(bytes);
        Ok(i64::from_le_bytes(array))
    }

    /// Reads a little-endian `i128`.
    ///
    /// # Errors
    ///
    /// Rejects truncated input.
    pub fn i128(&mut self) -> Result<i128> {
        let bytes = self.take(16)?;
        let mut array = [0_u8; 16];
        array.copy_from_slice(bytes);
        Ok(i128::from_le_bytes(array))
    }

    /// Reads the strict boolean encoding.
    ///
    /// # Errors
    ///
    /// Rejects values other than 0 or 1.
    pub fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(Error::manifest(format!(
                "invalid boolean byte {value} at offset {}",
                self.position - 1
            ))),
        }
    }

    /// Reads an enum tag.
    ///
    /// # Errors
    ///
    /// Rejects truncated input.
    pub fn tag(&mut self) -> Result<u8> {
        self.u8()
    }

    /// Reads a 128-bit identifier.
    ///
    /// # Errors
    ///
    /// Rejects truncated input.
    pub fn id128(&mut self) -> Result<GotId> {
        let bytes = self.take(16)?;
        let mut array = [0_u8; 16];
        array.copy_from_slice(bytes);
        Ok(GotId::from_bytes(array))
    }

    /// Reads a 256-bit digest.
    ///
    /// # Errors
    ///
    /// Rejects truncated input.
    pub fn digest32(&mut self) -> Result<Digest32> {
        let bytes = self.take(32)?;
        let mut array = [0_u8; 32];
        array.copy_from_slice(bytes);
        Ok(Digest32::from_bytes(array))
    }

    /// Reads a length-prefixed UTF-8 string.
    ///
    /// # Errors
    ///
    /// Rejects truncated input or invalid UTF-8.
    pub fn string(&mut self) -> Result<String> {
        let start = self.position;
        let length = self.u32()? as usize;
        let bytes = self.take(length)?;
        let text = std::str::from_utf8(bytes)
            .map_err(|_| Error::manifest(format!("invalid UTF-8 string at offset {start}")))?;
        Ok(text.to_owned())
    }

    /// Reads a list count after checking it against the remaining input.
    ///
    /// `minimum_element_size` is the smallest number of bytes one element can
    /// occupy. The check prevents allocating from an untrusted count.
    ///
    /// # Errors
    ///
    /// Rejects a count that cannot fit in the remaining bytes.
    pub fn count(&mut self, minimum_element_size: usize) -> Result<usize> {
        let count = self.u32()? as u64;
        let required = count.saturating_mul(minimum_element_size as u64);
        if required > self.remaining() as u64 {
            return Err(Error::manifest(format!(
                "list of {count} elements cannot fit in {} remaining bytes",
                self.remaining()
            )));
        }
        usize::try_from(count).map_err(|_| Error::manifest("list count exceeds the platform limit"))
    }

    /// Reads exactly `length` bytes.
    ///
    /// # Errors
    ///
    /// Rejects truncated input.
    pub fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| Error::manifest("artifact length overflow"))?;
        if end > self.data.len() {
            return Err(Error::manifest(format!(
                "truncated artifact: need {length} bytes at offset {}, have {}",
                self.position,
                self.remaining()
            )));
        }
        let bytes = &self.data[self.position..end];
        self.position = end;
        Ok(bytes)
    }

    /// Requires that every byte has been consumed.
    ///
    /// # Errors
    ///
    /// Rejects trailing bytes.
    pub fn finish(&self) -> Result<()> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(Error::manifest(format!(
                "{} trailing bytes after the artifact",
                self.remaining()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_primitives() {
        let mut encoder = Encoder::new();
        encoder.u8(7);
        encoder.u16(0x1234);
        encoder.u32(0x1234_5678);
        encoder.u64(0x1234_5678_9abc_def0);
        encoder.i64(-5);
        encoder.bool(true);
        encoder.tag(3);
        encoder.id128(GotId::from_bytes([0xab; 16]));
        encoder.digest32(Digest32::from_bytes([0xcd; 32]));
        encoder.string("héllo").unwrap();
        let bytes = encoder.into_bytes();

        let mut decoder = Decoder::new(&bytes);
        assert_eq!(decoder.u8().unwrap(), 7);
        assert_eq!(decoder.u16().unwrap(), 0x1234);
        assert_eq!(decoder.u32().unwrap(), 0x1234_5678);
        assert_eq!(decoder.u64().unwrap(), 0x1234_5678_9abc_def0);
        assert_eq!(decoder.i64().unwrap(), -5);
        assert!(decoder.bool().unwrap());
        assert_eq!(decoder.tag().unwrap(), 3);
        assert_eq!(decoder.id128().unwrap(), GotId::from_bytes([0xab; 16]));
        assert_eq!(
            decoder.digest32().unwrap(),
            Digest32::from_bytes([0xcd; 32])
        );
        assert_eq!(decoder.string().unwrap(), "héllo");
        decoder.finish().unwrap();
    }

    #[test]
    fn rejects_truncation_and_trailing_bytes() {
        let bytes = [1_u8, 0, 0, 0, 9];
        let mut decoder = Decoder::new(&bytes);
        assert_eq!(decoder.u32().unwrap(), 1);
        assert!(decoder.u32().is_err());

        let mut decoder = Decoder::new(&[0]);
        assert!(!decoder.bool().unwrap());
        assert!(Decoder::new(&[2]).bool().is_err());

        let mut decoder = Decoder::new(&[1, 2]);
        assert_eq!(decoder.u8().unwrap(), 1);
        assert!(decoder.finish().is_err());
    }

    #[test]
    fn rejects_untrusted_list_counts() {
        let mut decoder = Decoder::new(&[0xff, 0xff, 0xff, 0xff, 0]);
        assert!(decoder.count(8).is_err());
    }
}
