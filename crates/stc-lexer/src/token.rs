use std::fmt;

use stc_span::Span;

use crate::literal::{CharKind, Literal, StringEncoding};
use crate::{Keyword, Punct};

/// The syntactic kind of a token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenKind {
    /// End of input; always the last token.
    Eof,
    /// Identifier with its normalized (uppercase ASCII) symbol key.
    Ident(Box<str>),
    /// Reserved keyword.
    Keyword(Keyword),
    /// Decimal integer literal.
    IntLiteral,
    /// Decimal real literal with a decimal point.
    RealLiteral,
    /// Based literal (`2#`, `8#`, `16#`).
    BasedLiteral,
    /// Typed literal such as `INT#16#7FFF`.
    TypedLiteral,
    /// `STRING#` or an untyped quoted string.
    StringLiteral {
        /// Source encoding.
        encoding: StringEncoding,
        /// `false` when EOF was reached before the closing quote.
        terminated: bool,
    },
    /// `U#`/`USTRING#` UTF-8 string.
    UStringLiteral {
        /// `false` when EOF was reached before the closing quote.
        terminated: bool,
    },
    /// Quoted `CHAR#`, `WCHAR#`, or `UCHAR#` literal.
    CharLiteral {
        /// The prefix type.
        kind: CharKind,
        /// `false` when EOF was reached before the closing quote.
        terminated: bool,
    },
    /// Duration literal.
    DurationLiteral,
    /// Date literal.
    DateLiteral,
    /// Time-of-day literal.
    TodLiteral,
    /// Date-and-time literal.
    DateTimeLiteral,
    /// Directly represented variable.
    DirectVariable,
    /// `{...}` pragma token.
    Pragma {
        /// `false` when EOF was reached before the closing brace.
        terminated: bool,
    },
    /// Punctuation or compound operator.
    Punct(Punct),
    /// Malformed input. The parser treats this as a recoverable error.
    Error,
}

impl TokenKind {
    /// Returns `true` for [`TokenKind::Eof`].
    #[must_use]
    pub fn is_eof(&self) -> bool {
        matches!(self, Self::Eof)
    }

    /// Returns `true` for [`TokenKind::Error`].
    #[must_use]
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error)
    }

    /// Returns the stable display name used in snapshots and debugging.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Eof => "Eof",
            Self::Ident(_) => "Ident",
            Self::Keyword(_) => "Keyword",
            Self::IntLiteral => "IntLiteral",
            Self::RealLiteral => "RealLiteral",
            Self::BasedLiteral => "BasedLiteral",
            Self::TypedLiteral => "TypedLiteral",
            Self::StringLiteral { .. } => "StringLiteral",
            Self::UStringLiteral { .. } => "UStringLiteral",
            Self::CharLiteral { .. } => "CharLiteral",
            Self::DurationLiteral => "DurationLiteral",
            Self::DateLiteral => "DateLiteral",
            Self::TodLiteral => "TodLiteral",
            Self::DateTimeLiteral => "DateTimeLiteral",
            Self::DirectVariable => "DirectVariable",
            Self::Pragma { .. } => "Pragma",
            Self::Punct(_) => "Punct",
            Self::Error => "Error",
        }
    }
}

/// Kind of whitespace or comment trivia.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TriviaKind {
    /// Horizontal whitespace or one line terminator.
    Whitespace,
    /// `// ...` comment.
    LineComment,
    /// `/// ...` documentation comment (STC-LEX-051).
    DocComment,
    /// `(* ... *)` or `/* ... */` comment.
    BlockComment,
    /// A UTF-8 BOM at the start of the file (STC-LEX-001).
    Bom,
}

impl TriviaKind {
    /// Returns the stable display name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Whitespace => "Whitespace",
            Self::LineComment => "LineComment",
            Self::DocComment => "DocComment",
            Self::BlockComment => "BlockComment",
            Self::Bom => "Bom",
        }
    }
}

/// One trivia item: whitespace, a comment, or the leading BOM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trivia {
    /// Trivia kind.
    pub kind: TriviaKind,
    /// Byte range in the source file.
    pub span: Span,
}

impl Trivia {
    /// Returns the source text for this trivia item.
    #[must_use]
    pub fn text<'source>(&self, source: &'source str) -> &'source str {
        source
            .get(self.span.start as usize..self.span.end as usize)
            .unwrap_or("")
    }

    /// Returns `true` if this trivia item contains a line terminator.
    #[must_use]
    pub fn is_multiline(&self, source: &str) -> bool {
        matches!(self.kind, TriviaKind::Whitespace | TriviaKind::BlockComment)
            && self.text(source).contains(['\n', '\r'])
    }
}

/// A half-open range of indices into [`TokenStream::trivia`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TriviaRange {
    /// First trivia index.
    pub start: usize,
    /// One past the last trivia index.
    pub end: usize,
}

impl TriviaRange {
    /// Creates a trivia range.
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Returns the number of trivia items.
    #[must_use]
    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Returns `true` for an empty range.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

/// One token and its attached trivia.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// Token kind.
    pub kind: TokenKind,
    /// Byte range of the token text (excluding trivia).
    pub span: Span,
    /// Trivia before the token.
    pub leading_trivia: TriviaRange,
    /// Same-line trivia after the token (see the tokenization spec §2.1).
    pub trailing_trivia: TriviaRange,
    /// Decoded literal payload, when the token is a literal.
    pub literal: Option<Box<Literal>>,
}

impl Token {
    /// Returns the token's source text.
    #[must_use]
    pub fn text<'source>(&self, source: &'source str) -> &'source str {
        source
            .get(self.span.start as usize..self.span.end as usize)
            .unwrap_or("")
    }

    /// Returns the normalized symbol key for identifiers.
    #[must_use]
    pub fn normalized(&self) -> Option<&str> {
        match &self.kind {
            TokenKind::Ident(normalized) => Some(normalized),
            TokenKind::Keyword(keyword) => Some(keyword.as_str()),
            _ => None,
        }
    }

    /// Returns the decoded numeric literal, if this token has one.
    #[must_use]
    pub fn numeric(&self) -> Option<&crate::NumericLiteral> {
        match self.literal.as_deref() {
            Some(Literal::Numeric(numeric)) => Some(numeric),
            Some(Literal::Typed(typed)) => match &typed.value {
                crate::TypedValue::Numeric(numeric) => Some(numeric),
                crate::TypedValue::Bool(_) => None,
            },
            _ => None,
        }
    }

    /// Returns the decoded string literal, if this token has one.
    #[must_use]
    pub fn string(&self) -> Option<&crate::StringLiteral> {
        match self.literal.as_deref() {
            Some(Literal::String(string)) => Some(string),
            _ => None,
        }
    }
}

/// The complete token stream for one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenStream {
    /// Tokens in source order; the last token is always [`TokenKind::Eof`].
    pub tokens: Vec<Token>,
    /// All trivia in source order.
    pub trivia: Vec<Trivia>,
    /// The zero-width span of the end of input.
    pub eof_span: Span,
}

impl TokenStream {
    /// Returns the end-of-file token.
    #[must_use]
    pub fn eof(&self) -> Option<&Token> {
        self.tokens.last().filter(|token| token.kind.is_eof())
    }

    /// Returns the trivia slice for a range.
    #[must_use]
    pub fn trivia_range(&self, range: TriviaRange) -> &[Trivia] {
        self.trivia.get(range.start..range.end).unwrap_or(&[])
    }

    /// Returns a token's leading trivia.
    #[must_use]
    pub fn leading_trivia(&self, token: &Token) -> &[Trivia] {
        self.trivia_range(token.leading_trivia)
    }

    /// Returns a token's trailing trivia.
    #[must_use]
    pub fn trailing_trivia(&self, token: &Token) -> &[Trivia] {
        self.trivia_range(token.trailing_trivia)
    }

    /// Reconstructs the source text from tokens and trivia.
    ///
    /// For a valid stream this equals the input exactly (STC-LEX-004).
    #[must_use]
    pub fn reconstruct(&self, source: &str) -> String {
        let mut out = String::with_capacity(source.len());
        for token in &self.tokens {
            for trivia in self.leading_trivia(token) {
                push_span(&mut out, source, trivia.span);
            }
            push_span(&mut out, source, token.span);
            for trivia in self.trailing_trivia(token) {
                push_span(&mut out, source, trivia.span);
            }
        }
        out
    }

    /// Checks the stream invariants from the tokenization spec §11.
    ///
    /// # Errors
    ///
    /// Returns a description of the first violated invariant. This is used by
    /// tests and the fuzz target.
    pub fn check_invariants(&self, source: &str) -> Result<(), String> {
        if self.tokens.is_empty() {
            return Err("token stream is empty".into());
        }
        let source_len = u32::try_from(source.len())
            .map_err(|_| "source exceeds the 4 GiB span limit".to_string())?;
        let eof_count = self
            .tokens
            .iter()
            .filter(|token| token.kind.is_eof())
            .count();
        if eof_count != 1 {
            return Err(format!("expected exactly one Eof token, found {eof_count}"));
        }
        if !self.tokens.last().is_some_and(|token| token.kind.is_eof()) {
            return Err("the last token is not Eof".into());
        }

        let mut checker = StreamChecker {
            stream: self,
            source,
            source_len,
            expected: 0,
            next_trivia: 0,
        };
        checker.run()?;

        if checker.expected != source_len {
            return Err(format!(
                "stream covers {} of {source_len} bytes",
                checker.expected
            ));
        }
        if checker.next_trivia != self.trivia.len() {
            return Err(format!(
                "{} trivia item(s) are not attached to a token",
                self.trivia.len() - checker.next_trivia
            ));
        }
        if self.eof_span.start != source_len {
            return Err("eof_span does not point at the end of the source".into());
        }
        if self.tokens.last().map(|token| token.span) != Some(self.eof_span) {
            return Err("eof_span does not match the Eof token".into());
        }
        Ok(())
    }
}

struct StreamChecker<'a> {
    stream: &'a TokenStream,
    source: &'a str,
    source_len: u32,
    expected: u32,
    next_trivia: usize,
}

impl StreamChecker<'_> {
    fn run(&mut self) -> Result<(), String> {
        for index in 0..self.stream.tokens.len() {
            let token = &self.stream.tokens[index];
            let leading = token.leading_trivia;
            let span = token.span;
            let trailing = token.trailing_trivia;
            self.check_trivia(leading, index, "leading")?;
            self.check_span(span, index, "token")?;
            self.check_trivia(trailing, index, "trailing")?;
        }
        Ok(())
    }

    fn check_trivia(
        &mut self,
        range: TriviaRange,
        token_index: usize,
        side: &str,
    ) -> Result<(), String> {
        if range.start != self.next_trivia {
            return Err(format!(
                "token {token_index} {side} trivia starts at {} instead of {}",
                range.start, self.next_trivia
            ));
        }
        if range.end < range.start || range.end > self.stream.trivia.len() {
            return Err(format!(
                "token {token_index} {side} trivia range {}..{} is out of bounds",
                range.start, range.end
            ));
        }
        for trivia_index in range.start..range.end {
            let span = self.stream.trivia[trivia_index].span;
            self.check_span(span, token_index, "trivia")?;
        }
        self.next_trivia = range.end;
        Ok(())
    }

    fn check_span(&mut self, span: Span, token_index: usize, what: &str) -> Result<(), String> {
        if span.file != self.stream.eof_span.file {
            return Err(format!(
                "token {token_index} {what} uses file {} instead of {}",
                span.file.raw(),
                self.stream.eof_span.file.raw()
            ));
        }
        if span.start != self.expected {
            return Err(format!(
                "token {token_index} {what} starts at {} instead of {}",
                span.start, self.expected
            ));
        }
        if span.end < span.start || span.end > self.source_len {
            return Err(format!(
                "token {token_index} {what} span {}..{} is out of bounds",
                span.start, span.end
            ));
        }
        if !self.source.is_char_boundary(span.start as usize)
            || !self.source.is_char_boundary(span.end as usize)
        {
            return Err(format!(
                "token {token_index} {what} span {}..{} is not on UTF-8 boundaries",
                span.start, span.end
            ));
        }
        self.expected = span.end;
        Ok(())
    }
}

fn push_span(out: &mut String, source: &str, span: Span) {
    if let Some(text) = source.get(span.start as usize..span.end as usize) {
        out.push_str(text);
    }
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ident(normalized) => write!(f, "Ident({normalized})"),
            Self::Keyword(keyword) => write!(f, "Keyword({keyword})"),
            Self::Punct(punct) => write!(f, "Punct({punct})"),
            Self::StringLiteral { encoding, .. } => {
                write!(f, "StringLiteral({encoding:?})")
            }
            Self::CharLiteral { kind, .. } => write!(f, "CharLiteral({})", kind.as_str()),
            other => f.write_str(other.name()),
        }
    }
}
