//! `Content-Length` framing for the Debug Adapter Protocol over stdio.
//!
//! DAP uses the same framing as the Language Server Protocol: a header block
//! of `Name: value` lines terminated by a blank line, followed by exactly
//! `Content-Length` bytes of UTF-8 JSON. The reader here is strict: a header
//! line must be well formed, `Content-Length` is required exactly once, the
//! declared length must fit [`MAX_MESSAGE_BYTES`], and a body that ends early
//! is an error. Standard output stays DAP-only; adapters log to stderr.

use std::fmt;
use std::io::{BufRead, Write};

/// Largest accepted DAP message body, in bytes.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// A framing failure.
#[derive(Debug)]
pub enum FrameError {
    /// The underlying stream failed.
    Io(std::io::Error),
    /// A header line is not `Name: value`, or is not newline-terminated.
    MalformedHeader(String),
    /// The header block has no `Content-Length`.
    MissingContentLength,
    /// The header block has more than one `Content-Length`.
    DuplicateContentLength,
    /// The `Content-Length` value is not a non-negative integer.
    InvalidContentLength(String),
    /// The declared body exceeds [`MAX_MESSAGE_BYTES`].
    BodyTooLarge(usize),
    /// The stream ended before the declared body was complete.
    UnexpectedEof {
        /// Declared body length.
        expected: usize,
        /// Bytes actually read.
        read: usize,
    },
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::MalformedHeader(line) => write!(formatter, "malformed header line `{line}`"),
            Self::MissingContentLength => formatter.write_str("missing Content-Length header"),
            Self::DuplicateContentLength => {
                formatter.write_str("more than one Content-Length header")
            }
            Self::InvalidContentLength(value) => {
                write!(formatter, "invalid Content-Length value `{value}`")
            }
            Self::BodyTooLarge(length) => write!(
                formatter,
                "message body of {length} bytes exceeds the {MAX_MESSAGE_BYTES}-byte limit"
            ),
            Self::UnexpectedEof { expected, read } => write!(
                formatter,
                "stream ended after {read} of {expected} body bytes"
            ),
        }
    }
}

impl std::error::Error for FrameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for FrameError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Reads `Content-Length` framed messages from a buffered stream.
pub struct FrameReader<R: BufRead> {
    reader: R,
}

impl<R: BufRead> FrameReader<R> {
    /// Creates a reader over `reader`.
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    /// Reads the next message body, or `None` at a clean end of stream.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError`] for a malformed header block, a missing or
    /// duplicated `Content-Length`, an oversized body, a truncated body, or an
    /// I/O failure.
    pub fn read_frame(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
        let mut length = None;
        let mut saw_header = false;
        loop {
            let Some(line) = self.read_line()? else {
                if saw_header {
                    return Err(FrameError::UnexpectedEof {
                        expected: 0,
                        read: 0,
                    });
                }
                return Ok(None);
            };
            if line.is_empty() {
                break;
            }
            saw_header = true;
            let Some((name, value)) = line.split_once(':') else {
                return Err(FrameError::MalformedHeader(line));
            };
            if !name.trim().eq_ignore_ascii_case("content-length") {
                continue;
            }
            if length.is_some() {
                return Err(FrameError::DuplicateContentLength);
            }
            let value = value.trim();
            let parsed = value
                .parse::<usize>()
                .map_err(|_| FrameError::InvalidContentLength(value.to_owned()))?;
            if parsed > MAX_MESSAGE_BYTES {
                return Err(FrameError::BodyTooLarge(parsed));
            }
            length = Some(parsed);
        }
        let Some(length) = length else {
            return Err(FrameError::MissingContentLength);
        };
        let mut body = vec![0_u8; length];
        let mut read = 0;
        while read < length {
            match self.reader.read(&mut body[read..]) {
                Ok(0) => {
                    return Err(FrameError::UnexpectedEof {
                        expected: length,
                        read,
                    });
                }
                Ok(count) => read += count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(FrameError::Io(error)),
            }
        }
        Ok(Some(body))
    }

    /// Reads one header line without its line terminator.
    fn read_line(&mut self) -> Result<Option<String>, FrameError> {
        let mut line = String::new();
        let count = self.reader.read_line(&mut line)?;
        if count == 0 {
            return Ok(None);
        }
        let terminated = line.ends_with('\n');
        if terminated {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        if !terminated {
            return Err(FrameError::MalformedHeader(line));
        }
        Ok(Some(line))
    }
}

/// Writes one framed message and flushes the stream.
///
/// # Errors
///
/// Returns an I/O error when the stream cannot be written.
pub fn write_message<W: Write>(writer: &mut W, message: &[u8]) -> std::io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n", message.len())?;
    writer.write_all(message)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn framed(body: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{}", body.len(), body).into_bytes()
    }

    #[test]
    fn round_trips_one_message_and_stops_at_eof() {
        let mut bytes = framed("{\"seq\":1}");
        bytes.extend(framed("{\"seq\":2}"));
        let mut reader = FrameReader::new(Cursor::new(bytes));
        assert_eq!(
            reader.read_frame().expect("first").as_deref(),
            Some(b"{\"seq\":1}".as_slice())
        );
        assert_eq!(
            reader.read_frame().expect("second").as_deref(),
            Some(b"{\"seq\":2}".as_slice())
        );
        assert!(reader.read_frame().expect("eof").is_none());
    }

    #[test]
    fn write_then_read_matches() {
        let mut bytes = Vec::new();
        write_message(&mut bytes, b"{\"type\":\"request\"}").expect("write");
        assert!(bytes.starts_with(b"Content-Length: 18\r\n\r\n"));
        let mut reader = FrameReader::new(Cursor::new(bytes));
        assert_eq!(
            reader.read_frame().expect("frame").as_deref(),
            Some(b"{\"type\":\"request\"}".as_slice())
        );
    }

    #[test]
    fn accepts_lf_only_line_endings_and_case_insensitive_headers() {
        let text = "content-length: 2\nX-Other: 1\n\n{}";
        let mut reader = FrameReader::new(Cursor::new(text.as_bytes().to_vec()));
        assert_eq!(
            reader.read_frame().expect("frame").as_deref(),
            Some(b"{}".as_slice())
        );
    }

    #[test]
    fn rejects_missing_duplicate_and_invalid_lengths() {
        for (text, expected) in [
            ("X-Other: 1\r\n\r\n{}", "missing Content-Length"),
            (
                "Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}",
                "more than one Content-Length",
            ),
            ("Content-Length: two\r\n\r\n{}", "invalid Content-Length"),
            ("Content-Length 2\r\n\r\n{}", "malformed header"),
        ] {
            let mut reader = FrameReader::new(Cursor::new(text.as_bytes().to_vec()));
            let error = reader.read_frame().expect_err(text);
            assert!(
                error.to_string().contains(expected),
                "`{text}`: expected `{expected}`, got `{error}`"
            );
        }
    }

    #[test]
    fn rejects_oversized_and_truncated_bodies() {
        let text = format!("Content-Length: {}\r\n\r\n", MAX_MESSAGE_BYTES + 1);
        let mut reader = FrameReader::new(Cursor::new(text.into_bytes()));
        assert!(matches!(
            reader.read_frame(),
            Err(FrameError::BodyTooLarge(_))
        ));

        let text = "Content-Length: 4\r\n\r\n{}";
        let mut reader = FrameReader::new(Cursor::new(text.as_bytes().to_vec()));
        assert!(matches!(
            reader.read_frame(),
            Err(FrameError::UnexpectedEof {
                expected: 4,
                read: 2
            })
        ));
    }
}
