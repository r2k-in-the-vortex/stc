//! Stable diagnostic codes used by the lexer and by pragma tooling.

// Lexical errors (Tokenization §10.1).
pub const INVALID_UTF8: &str = "STC-E-LEX-001";
pub const INVALID_CHARACTER: &str = "STC-E-LEX-002";
pub const INVALID_IDENTIFIER: &str = "STC-E-LEX-003";
pub const UNTERMINATED_STRING: &str = "STC-E-LEX-004";
pub const INVALID_ESCAPE: &str = "STC-E-LEX-005";
pub const UNTERMINATED_BLOCK_COMMENT: &str = "STC-E-LEX-006";
pub const UNTERMINATED_PRAGMA: &str = "STC-E-LEX-007";
pub const MALFORMED_LITERAL: &str = "STC-E-LEX-008";
pub const INVALID_SCALAR_ESCAPE: &str = "STC-E-LEX-009";
pub const INVALID_JSON_PRAGMA: &str = "STC-E-LEX-010";
pub const DUPLICATE_JSON_KEY: &str = "STC-E-LEX-011";
pub const PRAGMA_PAYLOAD_LIMIT: &str = "STC-E-LEX-012";

// Pragma errors and warnings (Tokenization §8).
pub const UNKNOWN_PRAGMA: &str = "STC-E-PRAGMA-001";
pub const PRAGMA_PAYLOAD: &str = "STC-E-PRAGMA-002";
pub const IGNORED_PRAGMA: &str = "STC-W-PRAGMA-001";
pub const UNKNOWN_PRAGMA_KEY: &str = "STC-W-PRAGMA-002";

/// Deprecated octal based literal (`8#`, STC-LEX-021).
pub const DEPRECATED_OCTAL: &str = "STC-W-LEX-008";
