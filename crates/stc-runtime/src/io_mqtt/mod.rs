//! MQTT 5.0 client protocol core for the `mqtt` I/O backend (WL-15 phase 1).
//!
//! This module is the protocol layer of the WL-15 MQTT client. It is
//! deliberately transport-free: [`packet`] encodes and decodes MQTT 5.0 control
//! packets, [`payload`] maps scalar process-image values to the supported
//! payload encodings, [`topic`] validates topic names and filters, and
//! [`session`] is the connection state machine and keepalive scheduler, and
//! [`config`] validates the `mqtt`/`mqtt-status` device configuration and maps
//! declared channels to topics and image ranges. Sockets and TLS arrive in
//! phase 3/4; no module here opens a connection or reads the clock, so every
//! transition is deterministic in tests.
//!
//! The wire version is MQTT 5.0 only (WL-15 Q1). There is no MQTT 3.1.1 path
//! and no version negotiation. The client is project-owned with no external
//! MQTT crate (Q2); TLS is the only external dependency and is not part of this
//! phase (Q3).
//!
//! All length fields are bounded. Untrusted input produces
//! [`MqttError`] with a kind, never a panic.

pub mod config;
pub mod packet;
pub mod payload;
pub mod session;
pub mod topic;

use std::fmt;

/// Why a packet or transition was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The input ends before the packet is complete; read more bytes.
    Incomplete,
    /// The input violates the MQTT 5.0 encoding rules.
    Malformed,
    /// The input is valid MQTT but not supported by this client.
    Unsupported,
    /// The packet is not legal in the current session state.
    Protocol,
    /// The packet or payload exceeds a configured limit.
    TooLarge,
    /// A resource limit (for example the packet identifier space) is exhausted.
    Exhausted,
}

impl ErrorKind {
    /// Returns the stable spelling used in diagnostics and logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Incomplete => "incomplete",
            Self::Malformed => "malformed",
            Self::Unsupported => "unsupported",
            Self::Protocol => "protocol",
            Self::TooLarge => "too-large",
            Self::Exhausted => "exhausted",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// An MQTT 5.0 protocol error.
///
/// The message is diagnostic text; the kind is the machine-readable
/// classification. No secret or payload content is included by the codec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MqttError {
    kind: ErrorKind,
    message: String,
}

impl MqttError {
    /// Creates an error with the given kind and message.
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Returns the error kind.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns the diagnostic message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns `true` when the caller should read more bytes and retry.
    #[must_use]
    pub const fn is_incomplete(&self) -> bool {
        matches!(self.kind, ErrorKind::Incomplete)
    }

    pub(super) fn malformed(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Malformed, message)
    }

    pub(super) fn incomplete(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Incomplete, message)
    }

    pub(super) fn protocol(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Protocol, message)
    }

    pub(super) fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported, message)
    }

    pub(super) fn too_large(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::TooLarge, message)
    }

    pub(super) fn exhausted(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Exhausted, message)
    }
}

impl fmt::Display for MqttError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "mqtt {}: {}", self.kind, self.message)
    }
}

impl std::error::Error for MqttError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kinds_have_stable_spellings() {
        for (kind, spelling) in [
            (ErrorKind::Incomplete, "incomplete"),
            (ErrorKind::Malformed, "malformed"),
            (ErrorKind::Unsupported, "unsupported"),
            (ErrorKind::Protocol, "protocol"),
            (ErrorKind::TooLarge, "too-large"),
            (ErrorKind::Exhausted, "exhausted"),
        ] {
            assert_eq!(kind.as_str(), spelling);
            assert_eq!(kind.to_string(), spelling);
        }
    }

    #[test]
    fn incomplete_errors_are_recognizable() {
        let error = MqttError::incomplete("need more bytes");
        assert!(error.is_incomplete());
        assert_eq!(error.kind(), ErrorKind::Incomplete);
        assert_eq!(error.message(), "need more bytes");
        assert_eq!(error.to_string(), "mqtt incomplete: need more bytes");
        assert!(!MqttError::malformed("bad").is_incomplete());
    }
}
