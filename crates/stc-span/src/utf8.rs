use std::fmt;

/// Why a byte sequence is not valid UTF-8.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Utf8IssueKind {
    /// An invalid start byte or an invalid continuation sequence.
    InvalidSequence,
    /// The file ended in the middle of a code point.
    Truncated,
}

/// One invalid UTF-8 sequence found by [`decode_utf8`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Utf8Issue {
    /// Byte offset of the offending sequence in the original input.
    pub byte_offset: usize,
    /// Length of the offending sequence in bytes.
    pub byte_len: usize,
    /// Byte offset of the replacement scalar in the decoded text.
    pub decoded_offset: usize,
    /// The offending bytes.
    pub bytes: Vec<u8>,
    /// The kind of failure.
    pub kind: Utf8IssueKind,
}

impl Utf8Issue {
    /// Formats the offending bytes as uppercase hexadecimal.
    #[must_use]
    pub fn hex_bytes(&self) -> String {
        let mut out = String::with_capacity(self.bytes.len() * 3);
        for (index, byte) in self.bytes.iter().enumerate() {
            if index > 0 {
                out.push(' ');
            }
            out.push_str(&format!("{byte:02X}"));
        }
        out
    }
}

impl fmt::Display for Utf8Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid UTF-8 at byte {} ({} byte(s): {})",
            self.byte_offset,
            self.byte_len,
            self.hex_bytes()
        )
    }
}

/// The result of decoding source bytes.
#[derive(Clone, Debug, Default)]
pub struct DecodedUtf8 {
    /// Valid UTF-8 text. Every invalid sequence is replaced by U+FFFD.
    pub text: String,
    /// Invalid sequences in input order.
    pub issues: Vec<Utf8Issue>,
}

/// Decodes source bytes as UTF-8, recovering at the next valid scalar
/// boundary (STC-LEX-002).
///
/// Valid input is returned unchanged. Each maximal invalid subsequence is
/// replaced by one U+FFFD so that lexing can continue; the original byte
/// offset and bytes are recorded in [`DecodedUtf8::issues`].
#[must_use]
pub fn decode_utf8(bytes: &[u8]) -> DecodedUtf8 {
    let mut decoded = DecodedUtf8 {
        text: String::with_capacity(bytes.len()),
        issues: Vec::new(),
    };
    let mut offset = 0;
    while offset < bytes.len() {
        match std::str::from_utf8(&bytes[offset..]) {
            Ok(rest) => {
                decoded.text.push_str(rest);
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if valid > 0 {
                    let valid_end = offset + valid;
                    let valid_text = std::str::from_utf8(&bytes[offset..valid_end])
                        .expect("valid_up_to prefix must be valid UTF-8");
                    decoded.text.push_str(valid_text);
                }

                let bad_start = offset + valid;
                let mut bad_len = error
                    .error_len()
                    .unwrap_or_else(|| bytes.len().saturating_sub(bad_start));
                // `std` reports the shortest invalid prefix. Extend over
                // trailing continuation bytes so an overlong or surrogate
                // sequence is reported once, as one offending sequence,
                // rather than one diagnostic per byte.
                while let Some(byte) = bytes.get(bad_start + bad_len) {
                    if !(0x80..=0xBF).contains(byte) {
                        break;
                    }
                    bad_len += 1;
                }
                let bad_end = (bad_start + bad_len).min(bytes.len());
                let decoded_offset = decoded.text.len();
                decoded.text.push('\u{fffd}');
                decoded.issues.push(Utf8Issue {
                    byte_offset: bad_start,
                    byte_len: bad_end - bad_start,
                    decoded_offset,
                    bytes: bytes[bad_start..bad_end].to_vec(),
                    kind: if error.error_len().is_some() {
                        Utf8IssueKind::InvalidSequence
                    } else {
                        Utf8IssueKind::Truncated
                    },
                });

                if bad_end == bad_start {
                    // `Utf8Error` always reports either an error length or a
                    // truncated tail; this guard keeps the loop total if that
                    // ever changes.
                    break;
                }
                offset = bad_end;
            }
        }
    }
    decoded
}
