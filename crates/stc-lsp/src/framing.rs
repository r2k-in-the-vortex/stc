//! `Content-Length` framing over a byte stream.
//!
//! The reader is deliberately tolerant: a malformed header block is reported
//! as one [`FrameEvent::Malformed`] event and the stream is resynchronized at
//! the next `Content-Length` header so a single bad message cannot stop the
//! server. Framing never panics; every failure is a [`FrameError`].

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, BufRead, Write};

/// Largest accepted message body, in bytes.
///
/// A declared length above this bound is rejected without reading the body.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// A framing failure.
#[derive(Debug)]
pub enum FrameError {
    /// The underlying stream failed.
    Io(io::Error),
    /// A header line is not `Name: value`.
    MalformedHeader(String),
    /// The header block has no `Content-Length`.
    MissingContentLength,
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
            Self::MissingContentLength => write!(formatter, "missing Content-Length header"),
            Self::InvalidContentLength(value) => {
                write!(formatter, "invalid Content-Length value `{value}`")
            }
            Self::BodyTooLarge(length) => {
                write!(
                    formatter,
                    "message body of {length} bytes exceeds the limit"
                )
            }
            Self::UnexpectedEof { expected, read } => {
                write!(
                    formatter,
                    "stream ended after {read} of {expected} body bytes"
                )
            }
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

impl From<io::Error> for FrameError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// One event produced by a [`FrameReader`].
#[derive(Debug)]
pub enum FrameEvent {
    /// A complete message body.
    Message(Vec<u8>),
    /// A framing problem; the reader continues at the next header.
    Malformed(FrameError),
}

/// Reads `Content-Length` framed messages from a buffered stream.
pub struct FrameReader<R: BufRead> {
    reader: R,
    pending: VecDeque<FrameEvent>,
    /// A `Content-Length` recovered by resynchronization whose headers were
    /// already consumed.
    resume: Option<usize>,
    eof: bool,
}

impl<R: BufRead> FrameReader<R> {
    /// Creates a reader over `reader`.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            pending: VecDeque::new(),
            resume: None,
            eof: false,
        }
    }

    /// Returns the next event, or `None` at end of stream.
    ///
    /// After a malformed header event the reader resumes at the next
    /// `Content-Length` header, so callers should keep reading.
    pub fn next_event(&mut self) -> Option<FrameEvent> {
        if let Some(event) = self.pending.pop_front() {
            return Some(event);
        }
        if self.eof {
            return None;
        }
        let resume = self.resume.take();
        match self.read_block(resume) {
            Ok(None) => None,
            Ok(Some(Block::Message(body))) => Some(FrameEvent::Message(body)),
            Ok(Some(Block::MessageAfterError { body, error })) => {
                self.pending.push_back(FrameEvent::Message(body));
                Some(FrameEvent::Malformed(error))
            }
            Ok(Some(Block::Skip { error })) => {
                self.resume = self.resynchronize();
                if self.resume.is_none() {
                    self.eof = true;
                }
                Some(FrameEvent::Malformed(error))
            }
            Err(error) => {
                self.eof = true;
                Some(FrameEvent::Malformed(error))
            }
        }
    }

    fn read_block(&mut self, resume: Option<usize>) -> Result<Option<Block>, FrameError> {
        let length = match resume {
            Some(length) => length,
            None => match self.read_headers()? {
                Headers::End => {
                    self.eof = true;
                    return Ok(None);
                }
                Headers::Block { length, malformed } => {
                    let Some(length) = length else {
                        let error = malformed.unwrap_or(FrameError::MissingContentLength);
                        return Ok(Some(Block::Skip { error }));
                    };
                    if let Some(error) = malformed {
                        // The body is still readable; report the header
                        // problem but keep the message so no data is lost.
                        let body = self.read_body(length)?;
                        return Ok(Some(Block::MessageAfterError { body, error }));
                    }
                    length
                }
            },
        };
        let body = self.read_body(length)?;
        Ok(Some(Block::Message(body)))
    }

    /// Reads one header block.
    ///
    /// Returns [`Headers::End`] on a clean end of stream at a block boundary.
    fn read_headers(&mut self) -> Result<Headers, FrameError> {
        let mut length = None;
        let mut malformed = None;
        let mut saw_line = false;
        loop {
            let Some(line) = self.read_line()? else {
                if saw_line {
                    return Err(FrameError::UnexpectedEof {
                        expected: 0,
                        read: 0,
                    });
                }
                return Ok(Headers::End);
            };
            if line.is_empty() {
                return Ok(Headers::Block { length, malformed });
            }
            saw_line = true;
            let Some((name, value)) = line.split_once(':') else {
                malformed = Some(FrameError::MalformedHeader(line));
                continue;
            };
            if !name.trim().eq_ignore_ascii_case("content-length") {
                continue;
            }
            let value = value.trim();
            match value.parse::<usize>() {
                Ok(parsed) if parsed <= MAX_MESSAGE_BYTES => length = Some(parsed),
                Ok(parsed) => return Err(FrameError::BodyTooLarge(parsed)),
                Err(_) => malformed = Some(FrameError::InvalidContentLength(value.to_owned())),
            }
        }
    }

    /// Scans forward to the next usable `Content-Length` header block.
    ///
    /// Returns the body length when one is found before the end of the stream.
    fn resynchronize(&mut self) -> Option<usize> {
        loop {
            let line = match self.read_line() {
                Ok(Some(line)) => line,
                _ => return None,
            };
            if line.is_empty() {
                continue;
            }
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            if !name.trim().eq_ignore_ascii_case("content-length") {
                continue;
            }
            let Ok(length) = value.trim().parse::<usize>() else {
                continue;
            };
            if length > MAX_MESSAGE_BYTES {
                return None;
            }
            // Consume the rest of the header block.
            loop {
                match self.read_line() {
                    Ok(Some(line)) if !line.is_empty() => {}
                    Ok(Some(_)) | Ok(None) => return Some(length),
                    Err(_) => return None,
                }
            }
        }
    }

    fn read_body(&mut self, length: usize) -> Result<Vec<u8>, FrameError> {
        if length > MAX_MESSAGE_BYTES {
            return Err(FrameError::BodyTooLarge(length));
        }
        let mut body = vec![0_u8; length];
        let mut read = 0_usize;
        while read < length {
            match self.reader.read(&mut body[read..]) {
                Ok(0) => {
                    return Err(FrameError::UnexpectedEof {
                        expected: length,
                        read,
                    });
                }
                Ok(count) => read += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(FrameError::Io(error)),
            }
        }
        Ok(body)
    }

    /// Reads one header line, without its terminator.
    ///
    /// `Ok(None)` means a clean end of stream.
    fn read_line(&mut self) -> Result<Option<String>, FrameError> {
        let mut buffer = Vec::new();
        let count = self.reader.read_until(b'\n', &mut buffer)?;
        if count == 0 {
            return Ok(None);
        }
        if buffer.last() == Some(&b'\n') {
            buffer.pop();
        }
        if buffer.last() == Some(&b'\r') {
            buffer.pop();
        }
        Ok(Some(String::from_utf8_lossy(&buffer).into_owned()))
    }
}

enum Block {
    Message(Vec<u8>),
    MessageAfterError { body: Vec<u8>, error: FrameError },
    Skip { error: FrameError },
}

enum Headers {
    End,
    Block {
        length: Option<usize>,
        malformed: Option<FrameError>,
    },
}

/// Writes one framed message and flushes the stream.
///
/// # Errors
///
/// Returns the underlying I/O error.
pub fn write_message<W: Write>(writer: &mut W, body: &[u8]) -> io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use super::*;

    fn encode(body: &str) -> Vec<u8> {
        let mut output = Vec::new();
        write_message(&mut output, body.as_bytes()).expect("write frame");
        output
    }

    /// A reader that yields at most one byte per `read`, exercising partial
    /// reads through `BufReader`.
    struct Trickle<'a> {
        bytes: &'a [u8],
        position: usize,
    }

    impl std::io::Read for Trickle<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.position >= self.bytes.len() || buffer.is_empty() {
                return Ok(0);
            }
            buffer[0] = self.bytes[self.position];
            self.position += 1;
            Ok(1)
        }
    }

    fn messages(input: &[u8]) -> (Vec<String>, Vec<String>) {
        let mut reader = FrameReader::new(BufReader::new(Cursor::new(input)));
        let mut bodies = Vec::new();
        let mut errors = Vec::new();
        while let Some(event) = reader.next_event() {
            match event {
                FrameEvent::Message(body) => {
                    bodies.push(String::from_utf8(body).expect("utf-8 body"));
                }
                FrameEvent::Malformed(error) => errors.push(error.to_string()),
            }
        }
        (bodies, errors)
    }

    #[test]
    fn writes_content_length_and_crlf_headers() {
        let encoded = encode("{}");
        assert_eq!(encoded, b"Content-Length: 2\r\n\r\n{}");
    }

    #[test]
    fn reads_frames_across_partial_reads() {
        let mut input = encode("{\"a\":1}");
        input.extend(encode("{\"b\":2}"));
        let trickle = BufReader::new(Trickle {
            bytes: &input,
            position: 0,
        });
        let mut reader = FrameReader::new(trickle);
        let mut bodies = Vec::new();
        while let Some(event) = reader.next_event() {
            match event {
                FrameEvent::Message(body) => bodies.push(body),
                FrameEvent::Malformed(error) => panic!("unexpected {error}"),
            }
        }
        assert_eq!(bodies, vec![b"{\"a\":1}".to_vec(), b"{\"b\":2}".to_vec()]);
    }

    #[test]
    fn accepts_a_valid_header_block_after_a_malformed_length() {
        let mut input = b"Content-Length: nope\r\n\r\n".to_vec();
        input.extend(encode("{\"ok\":true}"));
        let (bodies, errors) = messages(&input);
        assert_eq!(bodies, vec!["{\"ok\":true}"]);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("invalid Content-Length"));
    }

    #[test]
    fn reports_a_malformed_line_and_keeps_the_message_body() {
        let input = b"garbage\r\nContent-Length: 2\r\n\r\n{}";
        let (bodies, errors) = messages(input);
        assert_eq!(bodies, vec!["{}"]);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("garbage"));
    }

    #[test]
    fn reports_a_missing_content_length_and_recovers() {
        let mut input = b"Content-Type: application/json\r\n\r\n".to_vec();
        input.extend(encode("{}"));
        let (bodies, errors) = messages(&input);
        assert_eq!(bodies, vec!["{}"]);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("missing Content-Length"));
    }

    #[test]
    fn reports_a_truncated_body_without_panicking() {
        let input = b"Content-Length: 10\r\n\r\n{}";
        let (bodies, errors) = messages(input);
        assert!(bodies.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("stream ended"));
    }
}
