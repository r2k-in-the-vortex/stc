//! Lexer for the `stc-2013-oop-utf8-v1` profile.
//!
//! The lexer is hand-written and byte-oriented over a valid UTF-8 `&str`
//! (STC-LEX-008, STC-LEX-009). It produces a lossless token stream: tokens and
//! trivia cover the input exactly once, so concatenating them reproduces the
//! source (STC-LEX-004).
//!
//! Responsibilities:
//!
//! - keywords, identifiers, and normalization (§3–§4);
//! - numeric, string, character, duration, date/time, and direct-variable
//!   literals (§5–§6), including UTF-8 string literals (STC-UTF8);
//! - comments and documentation comments as trivia (§7);
//! - pragmas as opaque tokens (§8), with balanced JSON braces;
//! - spans and diagnostics; the lexer never panics (STC-LEX-002).
//!
//! The lexer does not parse pragma JSON; that is the parser's job
//! (STC-LEX-056). It also does not range-check literal values; values survive
//! as digit strings for the analyzer (STC-LEX-023).

#![forbid(unsafe_code)]

pub mod codes;
mod keyword;
mod lexer;
mod literal;
mod punct;
mod token;

pub use keyword::Keyword;
pub use lexer::{LexedSource, Lexer, MAX_IDENTIFIER_LEN, tokenize, tokenize_bytes};
pub use literal::{
    CharKind, CharLiteral, DateLiteral, DateTimeLiteral, DirectArea, DirectSize, DirectVariable,
    DurationComponent, DurationLiteral, ElementaryType, Literal, NumericKind, NumericLiteral,
    Radix, Sign, StringEncoding, StringLiteral, TimeUnit, TodLiteral, TypedLiteral, TypedValue,
};
pub use punct::Punct;
pub use token::{Token, TokenKind, TokenStream, Trivia, TriviaKind, TriviaRange};
