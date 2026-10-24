//! Recursive-descent and Pratt-style parser for `stc-2013-oop-utf8-v1`.
//!
//! The parser consumes the lexer's lossless token stream and builds the CST in
//! `stc-syntax`. It never panics on malformed input: errors produce `Error`
//! nodes and recovery synchronizes at the sets in Architecture §3.2.

#![forbid(unsafe_code)]

pub mod codes;
mod config;
mod expr;
mod item;
mod parser;
mod pou;
mod stmt;

pub use parser::{ParseOptions, ParsedFile, parse};
