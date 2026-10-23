use stc_diag::DiagnosticSink;
use stc_span::{FileId, Span, decode_utf8};

use crate::codes;
use crate::literal::{
    CharKind, CharLiteral, DateLiteral, DateTimeLiteral, DirectArea, DirectSize, DirectVariable,
    DurationComponent, DurationLiteral, ElementaryType, Literal, NumericKind, NumericLiteral,
    Radix, Sign, StringEncoding, StringLiteral, TimeUnit, TodLiteral, TypedLiteral, TypedValue,
};
use crate::token::{Token, TokenKind, TokenStream, Trivia, TriviaKind, TriviaRange};
use crate::{Keyword, Punct};

/// Largest identifier length accepted before `STC-E-LEX-003` (STC-LEX-016).
///
/// The specification requires at least 255 bytes; `stc` accepts 1024.
pub const MAX_IDENTIFIER_LEN: usize = 1024;

/// A decoded input together with its token stream.
#[derive(Clone, Debug)]
pub struct LexedSource {
    /// UTF-8 text after invalid sequences were replaced by U+FFFD.
    pub text: String,
    /// Tokens and trivia for `text`.
    pub stream: TokenStream,
}

/// Tokenizes a valid UTF-8 source string.
#[must_use]
pub fn tokenize(file_id: FileId, source: &str, diagnostics: &mut DiagnosticSink) -> TokenStream {
    Lexer::new(file_id, source).tokenize(diagnostics)
}

/// Validates bytes as UTF-8, then tokenizes the result.
///
/// Invalid sequences are reported as `STC-E-LEX-001` and replaced by U+FFFD
/// so lexing can continue (STC-LEX-002).
#[must_use]
pub fn tokenize_bytes(
    file_id: FileId,
    bytes: &[u8],
    diagnostics: &mut DiagnosticSink,
) -> LexedSource {
    let decoded = decode_utf8(bytes);
    for issue in &decoded.issues {
        let start = offset_u32(issue.decoded_offset);
        let end = offset_u32(issue.decoded_offset + '\u{fffd}'.len_utf8());
        let what = match issue.kind {
            stc_span::Utf8IssueKind::Truncated => "truncated UTF-8 sequence",
            stc_span::Utf8IssueKind::InvalidSequence => "invalid UTF-8 sequence",
        };
        diagnostics.error(
            codes::INVALID_UTF8,
            format!(
                "{what} at byte {}: {}",
                issue.byte_offset,
                issue.hex_bytes()
            ),
            Span::new(file_id, start, end),
        );
    }
    let stream = Lexer::new(file_id, &decoded.text).tokenize(diagnostics);
    LexedSource {
        text: decoded.text,
        stream,
    }
}

/// The hand-written lexer (STC-LEX-008, STC-LEX-009).
pub struct Lexer<'src> {
    file_id: FileId,
    source: &'src str,
    bytes: &'src [u8],
    pos: usize,
    unterminated_comment: bool,
}

impl<'src> Lexer<'src> {
    /// Creates a lexer over valid UTF-8 source.
    #[must_use]
    pub fn new(file_id: FileId, source: &'src str) -> Self {
        Self {
            file_id,
            source,
            bytes: source.as_bytes(),
            pos: 0,
            unterminated_comment: false,
        }
    }

    /// Tokenizes the source. The lexer never panics and always terminates.
    pub fn tokenize(mut self, diagnostics: &mut DiagnosticSink) -> TokenStream {
        let mut tokens: Vec<Token> = Vec::new();
        let mut trivia = Vec::new();

        // STC-LEX-001: an initial BOM is trivia so that the CST stays lossless.
        let mut pending = Vec::new();
        if self.source.starts_with('\u{feff}') {
            self.pos = 3;
            pending.push(Trivia {
                kind: TriviaKind::Bom,
                span: Span::new(self.file_id, 0, 3),
            });
        }
        pending.extend(self.scan_trivia(diagnostics));

        let mut previous_token: Option<usize> = None;
        loop {
            // Trivia before the first token is leading trivia. After a token,
            // same-line horizontal whitespace and comments are trailing; the
            // first line break or multi-line comment starts the next token's
            // leading trivia.
            let split = if previous_token.is_some() {
                pending
                    .iter()
                    .position(|item| item.is_multiline(self.source))
                    .unwrap_or(pending.len())
            } else {
                0
            };
            let (trailing, leading) = pending.split_at(split);
            let trailing_range = append_trivia(&mut trivia, trailing);
            if let Some(index) = previous_token {
                tokens[index].trailing_trivia = trailing_range;
            }
            let leading_range = append_trivia(&mut trivia, leading);
            pending.clear();

            let eof_span = Span::empty(self.file_id, offset_u32(self.pos));
            if self.at_end() {
                let empty_trivia = TriviaRange::new(trivia.len(), trivia.len());
                if self.unterminated_comment {
                    // STC-LEX-049: the comment is already trivia; an empty
                    // Error token marks the recovery point for the parser.
                    tokens.push(Token {
                        kind: TokenKind::Error,
                        span: eof_span,
                        leading_trivia: leading_range,
                        trailing_trivia: empty_trivia,
                        literal: None,
                    });
                    tokens.push(Token {
                        kind: TokenKind::Eof,
                        span: eof_span,
                        leading_trivia: empty_trivia,
                        trailing_trivia: empty_trivia,
                        literal: None,
                    });
                } else {
                    tokens.push(Token {
                        kind: TokenKind::Eof,
                        span: eof_span,
                        leading_trivia: leading_range,
                        trailing_trivia: empty_trivia,
                        literal: None,
                    });
                }
                return TokenStream {
                    tokens,
                    trivia,
                    eof_span,
                };
            }

            let mut token = self.lex_token(diagnostics);
            token.leading_trivia = leading_range;
            tokens.push(token);
            previous_token = Some(tokens.len() - 1);
            pending = self.scan_trivia(diagnostics);
        }
    }

    // ------------------------------------------------------------------
    // Cursor helpers
    // ------------------------------------------------------------------

    fn at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn peek_char(&self) -> Option<char> {
        self.source[self.pos..].chars().next()
    }

    fn starts_with(&self, text: &str) -> bool {
        self.bytes[self.pos..].starts_with(text.as_bytes())
    }

    /// Advances one scalar, or one ASCII byte.
    fn bump_any(&mut self) {
        if let Some(byte) = self.peek() {
            if byte.is_ascii() {
                self.pos += 1;
            } else {
                let _ = self.bump_char();
            }
        }
    }

    fn bump_char(&mut self) -> Option<char> {
        let character = self.peek_char()?;
        self.pos += character.len_utf8();
        Some(character)
    }

    fn span_from(&self, start: usize) -> Span {
        Span::new(self.file_id, offset_u32(start), offset_u32(self.pos))
    }

    fn span_bytes(&self, start: usize, end: usize) -> Span {
        Span::new(self.file_id, offset_u32(start), offset_u32(end))
    }

    fn error(
        &self,
        diagnostics: &mut DiagnosticSink,
        code: &'static str,
        message: impl Into<String>,
        start: usize,
        end: usize,
    ) {
        diagnostics.error(code, message, self.span_bytes(start, end));
    }

    /// STC-LEX-003: a raw NUL is invalid everywhere except as an escape.
    fn report_nuls(
        &self,
        diagnostics: &mut DiagnosticSink,
        start: usize,
        end: usize,
        context: &str,
    ) {
        for (offset, byte) in self.bytes[start..end].iter().enumerate() {
            if *byte == 0 {
                self.error(
                    diagnostics,
                    codes::INVALID_CHARACTER,
                    format!("raw NUL is not allowed in {context}; use an escape for NUL data"),
                    start + offset,
                    start + offset + 1,
                );
            }
        }
    }

    fn error_token(&self, start: usize) -> Token {
        Token {
            kind: TokenKind::Error,
            span: self.span_from(start),
            leading_trivia: TriviaRange::default(),
            trailing_trivia: TriviaRange::default(),
            literal: None,
        }
    }

    fn literal_token(&self, kind: TokenKind, literal: Literal, start: usize) -> Token {
        Token {
            kind,
            span: self.span_from(start),
            leading_trivia: TriviaRange::default(),
            trailing_trivia: TriviaRange::default(),
            literal: Some(Box::new(literal)),
        }
    }

    fn plain_token(&self, kind: TokenKind, start: usize) -> Token {
        Token {
            kind,
            span: self.span_from(start),
            leading_trivia: TriviaRange::default(),
            trailing_trivia: TriviaRange::default(),
            literal: None,
        }
    }

    // ------------------------------------------------------------------
    // Trivia (Tokenization §7)
    // ------------------------------------------------------------------

    fn scan_trivia(&mut self, diagnostics: &mut DiagnosticSink) -> Vec<Trivia> {
        let mut items = Vec::new();
        while let Some(byte) = self.peek() {
            let start = self.pos;
            match byte {
                b' ' | b'\t' | 0x0B | 0x0C => {
                    while matches!(self.peek(), Some(b' ' | b'\t' | 0x0B | 0x0C)) {
                        self.pos += 1;
                    }
                    items.push(Trivia {
                        kind: TriviaKind::Whitespace,
                        span: self.span_from(start),
                    });
                }
                b'\n' => {
                    self.pos += 1;
                    items.push(Trivia {
                        kind: TriviaKind::Whitespace,
                        span: self.span_from(start),
                    });
                }
                b'\r' => {
                    self.pos += 1;
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                    items.push(Trivia {
                        kind: TriviaKind::Whitespace,
                        span: self.span_from(start),
                    });
                }
                b'/' if self.peek_at(1) == Some(b'/') => {
                    items.push(self.scan_line_comment(start, diagnostics));
                }
                b'/' if self.peek_at(1) == Some(b'*') => {
                    items.push(self.scan_block_comment(start, *b"/*", *b"*/", diagnostics));
                }
                b'(' if self.peek_at(1) == Some(b'*') => {
                    items.push(self.scan_block_comment(start, *b"(*", *b"*)", diagnostics));
                }
                _ => break,
            }
        }
        items
    }

    fn scan_line_comment(&mut self, start: usize, diagnostics: &mut DiagnosticSink) -> Trivia {
        self.pos += 2;
        let kind = if self.peek() == Some(b'/') {
            self.pos += 1;
            TriviaKind::DocComment
        } else {
            TriviaKind::LineComment
        };
        while let Some(byte) = self.peek() {
            if matches!(byte, b'\n' | b'\r' | 0x0C) {
                break;
            }
            self.bump_any();
        }
        self.report_nuls(diagnostics, start, self.pos, "a line comment");
        Trivia {
            kind,
            span: self.span_from(start),
        }
    }

    fn scan_block_comment(
        &mut self,
        start: usize,
        open: [u8; 2],
        close: [u8; 2],
        diagnostics: &mut DiagnosticSink,
    ) -> Trivia {
        self.pos += 2;
        let mut depth = 1usize;
        while !self.at_end() {
            if self.peek() == Some(open[0]) && self.peek_at(1) == Some(open[1]) {
                self.pos += 2;
                depth += 1;
            } else if self.peek() == Some(close[0]) && self.peek_at(1) == Some(close[1]) {
                self.pos += 2;
                depth -= 1;
                if depth == 0 {
                    self.report_nuls(diagnostics, start, self.pos, "a block comment");
                    return Trivia {
                        kind: TriviaKind::BlockComment,
                        span: self.span_from(start),
                    };
                }
            } else {
                self.bump_any();
            }
        }
        self.report_nuls(diagnostics, start, self.pos, "a block comment");
        self.unterminated_comment = true;
        self.error(
            diagnostics,
            codes::UNTERMINATED_BLOCK_COMMENT,
            "unterminated block comment",
            start,
            self.pos,
        );
        Trivia {
            kind: TriviaKind::BlockComment,
            span: self.span_from(start),
        }
    }

    // ------------------------------------------------------------------
    // Tokens
    // ------------------------------------------------------------------

    fn lex_token(&mut self, diagnostics: &mut DiagnosticSink) -> Token {
        let start = self.pos;
        match self.peek() {
            Some(b'{') => self.lex_pragma(start, diagnostics),
            Some(b'\'' | b'"') => {
                let encoding = if self.peek() == Some(b'\'') {
                    StringEncoding::Sbcs
                } else {
                    StringEncoding::Dbcs
                };
                self.lex_quoted(start, encoding, None, diagnostics)
            }
            Some(b'0'..=b'9') => self.lex_number(start, diagnostics),
            Some(b'%') => self.lex_percent(start, diagnostics),
            Some(b'A'..=b'Z' | b'a'..=b'z' | b'_') => self.lex_word(start, diagnostics),
            Some(_) => self.lex_punct_or_error(start, diagnostics),
            None => self.error_token(start),
        }
    }

    // ------------------------------------------------------------------
    // Words: identifiers, keywords, and literal prefixes (Tokenization §3–§5)
    // ------------------------------------------------------------------

    fn lex_word(&mut self, start: usize, diagnostics: &mut DiagnosticSink) -> Token {
        while matches!(
            self.peek(),
            Some(b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_')
        ) {
            self.pos += 1;
        }
        let end = self.pos;
        let word = &self.source[start..end];
        let normalized = word.to_ascii_uppercase();

        if self.peek() == Some(b'#')
            && let Some(token) = self.lex_prefixed_literal(start, &normalized, diagnostics)
        {
            return token;
        }

        if let Some(keyword) = Keyword::from_normalized(&normalized) {
            return self.plain_token(TokenKind::Keyword(keyword), start);
        }

        self.validate_identifier(start, end, word, diagnostics);
        self.plain_token(TokenKind::Ident(normalized.into_boxed_str()), start)
    }

    fn validate_identifier(
        &self,
        start: usize,
        end: usize,
        word: &str,
        diagnostics: &mut DiagnosticSink,
    ) {
        let length = end - start;
        if length > MAX_IDENTIFIER_LEN {
            self.error(
                diagnostics,
                codes::INVALID_IDENTIFIER,
                format!(
                    "identifier is {length} bytes; the implementation limit is {MAX_IDENTIFIER_LEN}"
                ),
                start,
                end,
            );
        } else if word.starts_with("__") || word.contains("__") || word.ends_with('_') {
            self.error(
                diagnostics,
                codes::INVALID_IDENTIFIER,
                format!(
                    "invalid underscore placement in identifier `{word}`; only single embedded underscores are allowed"
                ),
                start,
                end,
            );
        }
    }

    fn lex_prefixed_literal(
        &mut self,
        start: usize,
        normalized: &str,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Token> {
        if let Some(long) = duration_long(normalized) {
            self.pos += 1;
            return Some(self.lex_duration_body(start, long, diagnostics));
        }
        if let Some(long) = date_long(normalized) {
            self.pos += 1;
            return Some(self.lex_date(start, long, diagnostics));
        }
        if let Some(long) = tod_long(normalized) {
            self.pos += 1;
            return Some(self.lex_tod(start, long, diagnostics));
        }
        if let Some(long) = datetime_long(normalized) {
            self.pos += 1;
            return Some(self.lex_datetime(start, long, diagnostics));
        }
        let string_encoding = match normalized {
            "STRING" => Some(StringEncoding::Sbcs),
            "WSTRING" => Some(StringEncoding::Dbcs),
            "U" | "USTRING" => Some(StringEncoding::Utf8),
            _ => None,
        };
        if let Some(encoding) = string_encoding {
            self.pos += 1;
            return Some(self.lex_string_prefixed(start, encoding, diagnostics));
        }
        if let Some(kind) = char_kind(normalized) {
            self.pos += 1;
            return Some(self.lex_char_prefixed(start, kind, diagnostics));
        }
        if let Some(ty) = ElementaryType::from_prefix(normalized) {
            self.pos += 1;
            return Some(self.lex_typed_body(start, ty, diagnostics));
        }
        None
    }

    // ------------------------------------------------------------------
    // Numeric literals (Tokenization §5.1)
    // ------------------------------------------------------------------

    fn based_prefix(&self) -> Option<(Radix, usize)> {
        if self.starts_with("16#") {
            Some((Radix::Hexadecimal, 3))
        } else if self.starts_with("8#") {
            Some((Radix::Octal, 2))
        } else if self.starts_with("2#") {
            Some((Radix::Binary, 2))
        } else {
            None
        }
    }

    fn lex_number(&mut self, start: usize, diagnostics: &mut DiagnosticSink) -> Token {
        let result = if self.based_prefix().is_some() {
            self.parse_based_numeric(start, diagnostics)
        } else {
            self.parse_decimal_numeric(start, diagnostics)
        };
        match result {
            Ok(numeric) => {
                let kind = match (numeric.kind, numeric.radix) {
                    (NumericKind::Integer, Radix::Decimal) => TokenKind::IntLiteral,
                    (NumericKind::Real, _) => TokenKind::RealLiteral,
                    (NumericKind::Integer, _) => TokenKind::BasedLiteral,
                };
                self.literal_token(kind, Literal::Numeric(numeric), start)
            }
            Err(()) => self.error_token(start),
        }
    }

    fn parse_decimal_numeric(
        &mut self,
        start: usize,
        diagnostics: &mut DiagnosticSink,
    ) -> Result<NumericLiteral, ()> {
        let mut malformed = false;
        let integer_digits = self.scan_digit_run(Radix::Decimal, &mut malformed);
        if integer_digits.is_empty() {
            self.consume_literal_junk();
            self.error(
                diagnostics,
                codes::MALFORMED_LITERAL,
                "expected digits in numeric literal",
                start,
                self.pos,
            );
            return Err(());
        }
        if malformed {
            self.consume_literal_junk();
            self.error(
                diagnostics,
                codes::MALFORMED_LITERAL,
                "invalid underscore placement in numeric literal",
                start,
                self.pos,
            );
            return Err(());
        }

        let mut text = integer_digits;
        let mut kind = NumericKind::Integer;
        if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
            let mut fraction_malformed = false;
            let fraction = self.scan_digit_run(Radix::Decimal, &mut fraction_malformed);
            if fraction_malformed {
                self.consume_literal_junk();
                self.error(
                    diagnostics,
                    codes::MALFORMED_LITERAL,
                    "invalid underscore placement in real literal",
                    start,
                    self.pos,
                );
                return Err(());
            }
            text.push('.');
            text.push_str(&fraction);
            kind = NumericKind::Real;
        }

        if matches!(self.peek(), Some(b'e' | b'E')) {
            let had_point = kind == NumericKind::Real;
            self.pos += 1;
            let exponent_sign = if matches!(self.peek(), Some(b'+' | b'-')) {
                let sign = self.peek();
                self.pos += 1;
                sign
            } else {
                None
            };
            let mut exponent_malformed = false;
            let exponent = self.scan_digit_run(Radix::Decimal, &mut exponent_malformed);
            if exponent.is_empty() || exponent_malformed {
                self.consume_literal_junk();
                self.error(
                    diagnostics,
                    codes::MALFORMED_LITERAL,
                    "malformed exponent in real literal",
                    start,
                    self.pos,
                );
                return Err(());
            }
            if !had_point {
                self.consume_literal_junk();
                self.error(
                    diagnostics,
                    codes::MALFORMED_LITERAL,
                    "a real literal requires a decimal point before the exponent",
                    start,
                    self.pos,
                );
                return Err(());
            }
            text.push('E');
            if let Some(sign) = exponent_sign {
                text.push(sign as char);
            }
            text.push_str(&exponent);
        }

        Ok(NumericLiteral {
            kind,
            radix: Radix::Decimal,
            text: text.into_boxed_str(),
        })
    }

    fn parse_based_numeric(
        &mut self,
        start: usize,
        diagnostics: &mut DiagnosticSink,
    ) -> Result<NumericLiteral, ()> {
        let Some((radix, prefix_len)) = self.based_prefix() else {
            return Err(());
        };
        self.pos += prefix_len;
        let mut malformed = false;
        let digits = self.scan_digit_run(radix, &mut malformed);
        if self.peek().is_some_and(|b| b.is_ascii_alphanumeric()) {
            while matches!(
                self.peek(),
                Some(b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_')
            ) {
                self.pos += 1;
            }
            malformed = true;
        }
        let invalid = malformed || digits.is_empty();
        if invalid {
            self.consume_literal_junk();
        }
        if radix == Radix::Octal {
            diagnostics.warning(
                codes::DEPRECATED_OCTAL,
                "octal based literals (8#) are accepted for IEC 61131-3:2013 but are deprecated",
                self.span_from(start),
            );
        }
        if invalid {
            self.error(
                diagnostics,
                codes::MALFORMED_LITERAL,
                format!("malformed base-{} literal", radix.base()),
                start,
                self.pos,
            );
            return Err(());
        }
        Ok(NumericLiteral {
            kind: NumericKind::Integer,
            radix,
            text: digits.into_boxed_str(),
        })
    }

    fn scan_digit_run(&mut self, radix: Radix, malformed: &mut bool) -> String {
        let mut digits = String::new();
        let mut previous_was_digit = false;
        while let Some(byte) = self.peek() {
            if digit_value(byte, radix).is_some() {
                let character = if radix == Radix::Hexadecimal {
                    (byte as char).to_ascii_uppercase()
                } else {
                    byte as char
                };
                digits.push(character);
                self.pos += 1;
                previous_was_digit = true;
            } else if byte == b'_' {
                self.pos += 1;
                if previous_was_digit
                    && self
                        .peek()
                        .is_some_and(|next| digit_value(next, radix).is_some())
                {
                    previous_was_digit = false;
                } else {
                    *malformed = true;
                    previous_was_digit = false;
                }
            } else {
                break;
            }
        }
        digits
    }

    fn consume_literal_junk(&mut self) {
        while let Some(byte) = self.peek() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-' | b'+') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn lex_typed_body(
        &mut self,
        start: usize,
        ty: ElementaryType,
        diagnostics: &mut DiagnosticSink,
    ) -> Token {
        let sign = match self.peek() {
            Some(b'+') => {
                self.pos += 1;
                Some(Sign::Plus)
            }
            Some(b'-') => {
                self.pos += 1;
                Some(Sign::Minus)
            }
            _ => None,
        };

        if ty.is_bool() && sign.is_none() && self.peek().is_some_and(|b| b.is_ascii_alphabetic()) {
            let word_start = self.pos;
            while matches!(
                self.peek(),
                Some(b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_')
            ) {
                self.pos += 1;
            }
            let word = &self.source[word_start..self.pos];
            let value = if word.eq_ignore_ascii_case("TRUE") {
                Some(true)
            } else if word.eq_ignore_ascii_case("FALSE") {
                Some(false)
            } else {
                None
            };
            return match value {
                Some(value) => self.literal_token(
                    TokenKind::TypedLiteral,
                    Literal::Typed(TypedLiteral {
                        ty,
                        sign,
                        value: TypedValue::Bool(value),
                    }),
                    start,
                ),
                None => {
                    self.consume_literal_junk();
                    self.error(
                        diagnostics,
                        codes::MALFORMED_LITERAL,
                        "BOOL# requires TRUE, FALSE, or a numeric value",
                        start,
                        self.pos,
                    );
                    self.error_token(start)
                }
            };
        }

        let numeric = if self.based_prefix().is_some() {
            self.parse_based_numeric(start, diagnostics)
        } else if self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.parse_decimal_numeric(start, diagnostics)
        } else {
            self.consume_literal_junk();
            self.error(
                diagnostics,
                codes::MALFORMED_LITERAL,
                format!("expected a value after {}#", ty.as_str()),
                start,
                self.pos,
            );
            return self.error_token(start);
        };

        match numeric {
            Ok(numeric) => self.literal_token(
                TokenKind::TypedLiteral,
                Literal::Typed(TypedLiteral {
                    ty,
                    sign,
                    value: TypedValue::Numeric(numeric),
                }),
                start,
            ),
            Err(()) => self.error_token(start),
        }
    }

    // ------------------------------------------------------------------
    // Duration, date, and time literals (Tokenization §5.4)
    // ------------------------------------------------------------------

    fn lex_duration_body(
        &mut self,
        start: usize,
        long: bool,
        diagnostics: &mut DiagnosticSink,
    ) -> Token {
        let sign = match self.peek() {
            Some(b'+') => {
                self.pos += 1;
                Some(Sign::Plus)
            }
            Some(b'-') => {
                self.pos += 1;
                Some(Sign::Minus)
            }
            _ => None,
        };

        let mut components = Vec::new();
        let mut malformed = false;
        loop {
            if !self.peek().is_some_and(|b| b.is_ascii_digit()) {
                malformed = true;
                break;
            }
            let value_start = self.pos;
            while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                self.pos += 1;
            }
            let value = self.source[value_start..self.pos].to_string();

            let fraction = if self.peek() == Some(b'.') {
                self.pos += 1;
                let fraction_start = self.pos;
                while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                    self.pos += 1;
                }
                if fraction_start == self.pos {
                    malformed = true;
                    break;
                }
                Some(self.source[fraction_start..self.pos].into())
            } else {
                None
            };

            let Some(unit) = self.parse_time_unit() else {
                malformed = true;
                break;
            };
            let has_fraction = fraction.is_some();
            components.push(DurationComponent {
                value: value.into_boxed_str(),
                fraction,
                unit,
            });

            if has_fraction {
                if self.peek() == Some(b'_') || self.peek().is_some_and(|b| b.is_ascii_digit()) {
                    malformed = true;
                }
                break;
            }
            if self.peek() == Some(b'_') {
                self.pos += 1;
                if !self.peek().is_some_and(|b| b.is_ascii_digit()) {
                    malformed = true;
                    break;
                }
            } else if !self.peek().is_some_and(|b| b.is_ascii_digit()) {
                break;
            }
        }

        if malformed || components.is_empty() {
            self.consume_literal_junk();
            self.error(
                diagnostics,
                codes::MALFORMED_LITERAL,
                "malformed duration literal",
                start,
                self.pos,
            );
            return self.error_token(start);
        }

        self.literal_token(
            TokenKind::DurationLiteral,
            Literal::Duration(DurationLiteral {
                long,
                sign,
                components,
            }),
            start,
        )
    }

    fn parse_time_unit(&mut self) -> Option<TimeUnit> {
        if let Some(second) = self.peek_at(1) {
            let first = self.peek()?.to_ascii_lowercase();
            let second = second.to_ascii_lowercase();
            let unit = match (first, second) {
                (b'm', b's') => Some(TimeUnit::Millisecond),
                (b'u', b's') => Some(TimeUnit::Microsecond),
                (b'n', b's') => Some(TimeUnit::Nanosecond),
                _ => None,
            };
            if let Some(unit) = unit {
                self.pos += 2;
                return Some(unit);
            }
        }
        let unit = match self.peek()?.to_ascii_lowercase() {
            b'd' => TimeUnit::Day,
            b'h' => TimeUnit::Hour,
            b'm' => TimeUnit::Minute,
            b's' => TimeUnit::Second,
            _ => return None,
        };
        self.pos += 1;
        Some(unit)
    }

    fn lex_date(&mut self, start: usize, long: bool, diagnostics: &mut DiagnosticSink) -> Token {
        match self.parse_date_body() {
            Some((year, month, day)) => self.literal_token(
                TokenKind::DateLiteral,
                Literal::Date(DateLiteral {
                    long,
                    year,
                    month,
                    day,
                }),
                start,
            ),
            None => {
                self.consume_literal_junk();
                self.error(
                    diagnostics,
                    codes::MALFORMED_LITERAL,
                    "malformed date literal; expected YYYY-MM-DD",
                    start,
                    self.pos,
                );
                self.error_token(start)
            }
        }
    }

    fn lex_tod(&mut self, start: usize, long: bool, diagnostics: &mut DiagnosticSink) -> Token {
        match self.parse_tod_body() {
            Some((hour, minute, second, fraction)) => self.literal_token(
                TokenKind::TodLiteral,
                Literal::Tod(TodLiteral {
                    long,
                    hour,
                    minute,
                    second,
                    fraction,
                }),
                start,
            ),
            None => {
                self.consume_literal_junk();
                self.error(
                    diagnostics,
                    codes::MALFORMED_LITERAL,
                    "malformed time-of-day literal; expected HH:MM:SS[.fraction]",
                    start,
                    self.pos,
                );
                self.error_token(start)
            }
        }
    }

    fn lex_datetime(
        &mut self,
        start: usize,
        long: bool,
        diagnostics: &mut DiagnosticSink,
    ) -> Token {
        let parsed = (|| {
            let (year, month, day) = self.parse_date_body()?;
            if self.peek() != Some(b'-') {
                return None;
            }
            self.pos += 1;
            let (hour, minute, second, fraction) = self.parse_tod_body()?;
            Some((year, month, day, hour, minute, second, fraction))
        })();
        match parsed {
            Some((year, month, day, hour, minute, second, fraction)) => self.literal_token(
                TokenKind::DateTimeLiteral,
                Literal::DateTime(DateTimeLiteral {
                    long,
                    date: DateLiteral {
                        long,
                        year,
                        month,
                        day,
                    },
                    tod: TodLiteral {
                        long,
                        hour,
                        minute,
                        second,
                        fraction,
                    },
                }),
                start,
            ),
            None => {
                self.consume_literal_junk();
                self.error(
                    diagnostics,
                    codes::MALFORMED_LITERAL,
                    "malformed date-and-time literal; expected YYYY-MM-DD-HH:MM:SS[.fraction]",
                    start,
                    self.pos,
                );
                self.error_token(start)
            }
        }
    }

    fn parse_date_body(&mut self) -> Option<(u32, u32, u32)> {
        let year = self.take_digits(4)?;
        if self.peek() != Some(b'-') {
            return None;
        }
        self.pos += 1;
        let month = self.take_digits(2)?;
        if self.peek() != Some(b'-') {
            return None;
        }
        self.pos += 1;
        let day = self.take_digits(2)?;
        Some((year, month, day))
    }

    fn parse_tod_body(&mut self) -> Option<(u32, u32, u32, Option<Box<str>>)> {
        let hour = self.take_digits(2)?;
        if self.peek() != Some(b':') {
            return None;
        }
        self.pos += 1;
        let minute = self.take_digits(2)?;
        if self.peek() != Some(b':') {
            return None;
        }
        self.pos += 1;
        let second = self.take_digits(2)?;
        let fraction = if self.peek() == Some(b'.') {
            self.pos += 1;
            let mut malformed = false;
            let digits = self.scan_digit_run(Radix::Decimal, &mut malformed);
            if digits.is_empty() || malformed || digits.len() > 9 {
                return None;
            }
            Some(digits.into_boxed_str())
        } else {
            None
        };
        Some((hour, minute, second, fraction))
    }

    fn take_digits(&mut self, count: usize) -> Option<u32> {
        let end = self.pos.checked_add(count)?;
        let slice = self.bytes.get(self.pos..end)?;
        if !slice.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let mut value = 0u32;
        for byte in slice {
            value = value * 10 + u32::from(byte - b'0');
        }
        self.pos = end;
        Some(value)
    }

    // ------------------------------------------------------------------
    // Strings and character literals (Tokenization §5.2–§5.3)
    // ------------------------------------------------------------------

    fn lex_string_prefixed(
        &mut self,
        start: usize,
        encoding: StringEncoding,
        diagnostics: &mut DiagnosticSink,
    ) -> Token {
        if matches!(self.peek(), Some(b'\'' | b'"')) {
            self.lex_quoted(start, encoding, None, diagnostics)
        } else {
            self.consume_literal_junk();
            self.error(
                diagnostics,
                codes::MALFORMED_LITERAL,
                "expected a quoted string after the string type prefix",
                start,
                self.pos,
            );
            self.error_token(start)
        }
    }

    fn lex_char_prefixed(
        &mut self,
        start: usize,
        kind: CharKind,
        diagnostics: &mut DiagnosticSink,
    ) -> Token {
        if matches!(self.peek(), Some(b'\'' | b'"')) {
            let encoding = char_encoding(kind);
            return self.lex_quoted(start, encoding, Some(kind), diagnostics);
        }
        if kind == CharKind::UChar && self.peek() == Some(b'$') {
            return self.lex_uchar_unquoted(start, diagnostics);
        }
        if matches!(self.peek(), Some(b'0'..=b'9' | b'+' | b'-')) {
            let ty = match kind {
                CharKind::Char => ElementaryType::Char,
                CharKind::WChar => ElementaryType::Wchar,
                CharKind::UChar => ElementaryType::Uchar,
            };
            return self.lex_typed_body(start, ty, diagnostics);
        }
        self.consume_literal_junk();
        self.error(
            diagnostics,
            codes::MALFORMED_LITERAL,
            format!(
                "expected a quoted character or numeric value after {}#",
                kind.as_str()
            ),
            start,
            self.pos,
        );
        self.error_token(start)
    }

    fn lex_uchar_unquoted(&mut self, start: usize, diagnostics: &mut DiagnosticSink) -> Token {
        let mut units = Vec::new();
        let before = diagnostics.len();
        self.lex_escape(StringEncoding::Utf8, diagnostics, &mut units);
        if units.len() != 1 && diagnostics.len() == before {
            self.error(
                diagnostics,
                codes::MALFORMED_LITERAL,
                "UCHAR# must contain exactly one Unicode scalar value",
                start,
                self.pos,
            );
        }
        let value = if units.len() == 1 {
            Some(units[0])
        } else {
            None
        };
        self.literal_token(
            TokenKind::CharLiteral {
                kind: CharKind::UChar,
                terminated: true,
            },
            Literal::Char(CharLiteral {
                kind: CharKind::UChar,
                value,
            }),
            start,
        )
    }

    fn lex_quoted(
        &mut self,
        start: usize,
        encoding: StringEncoding,
        char_kind: Option<CharKind>,
        diagnostics: &mut DiagnosticSink,
    ) -> Token {
        let Some(delimiter) = self.peek_char() else {
            self.error(
                diagnostics,
                codes::MALFORMED_LITERAL,
                "expected a string literal",
                start,
                self.pos,
            );
            return self.error_token(start);
        };
        self.pos += delimiter.len_utf8();

        let mut units = Vec::new();
        let mut terminated = false;
        while let Some(character) = self.peek_char() {
            if character == delimiter {
                self.pos += character.len_utf8();
                terminated = true;
                break;
            }
            if character == '\0' {
                self.error(
                    diagnostics,
                    codes::INVALID_CHARACTER,
                    "raw NUL is not allowed in a string literal; use $00 or ${0000}",
                    self.pos,
                    self.pos + 1,
                );
                self.pos += 1;
                continue;
            }
            if character == '\n' || character == '\r' {
                let mut end = self.pos + character.len_utf8();
                if character == '\r' && self.peek_at(1) == Some(b'\n') {
                    end += 1;
                }
                self.error(
                    diagnostics,
                    codes::INVALID_CHARACTER,
                    "raw line break is not allowed in a string literal; use $L, $N, or $R",
                    self.pos,
                    end,
                );
                match encoding {
                    StringEncoding::Sbcs => {}
                    StringEncoding::Dbcs => push_scalar(encoding, &mut units, character as u32),
                    StringEncoding::Utf8 => units.push(character as u32),
                }
                self.pos = end;
                continue;
            }
            if character == '$' {
                self.lex_escape(encoding, diagnostics, &mut units);
                continue;
            }

            match encoding {
                StringEncoding::Sbcs => {
                    let code = character as u32;
                    if (0x20..=0x7E).contains(&code) {
                        units.push(code);
                    } else {
                        self.error(
                            diagnostics,
                            codes::INVALID_CHARACTER,
                            format!(
                                "character {character:?} (U+{code:04X}) is not allowed in a single-byte string literal"
                            ),
                            self.pos,
                            self.pos + character.len_utf8(),
                        );
                    }
                }
                StringEncoding::Dbcs => push_scalar(encoding, &mut units, character as u32),
                StringEncoding::Utf8 => units.push(character as u32),
            }
            self.pos += character.len_utf8();
        }

        if !terminated {
            self.error(
                diagnostics,
                codes::UNTERMINATED_STRING,
                "unterminated string literal",
                start,
                self.pos,
            );
        }

        if let Some(kind) = char_kind {
            let value = if terminated && units.len() == 1 {
                Some(units[0])
            } else {
                if terminated {
                    self.error(
                        diagnostics,
                        codes::MALFORMED_LITERAL,
                        format!(
                            "{} literal must contain exactly one character, found {}",
                            kind.as_str(),
                            units.len()
                        ),
                        start,
                        self.pos,
                    );
                }
                None
            };
            return self.literal_token(
                TokenKind::CharLiteral { kind, terminated },
                Literal::Char(CharLiteral { kind, value }),
                start,
            );
        }

        let is_utf8 = encoding == StringEncoding::Utf8;
        let kind = if is_utf8 {
            TokenKind::UStringLiteral { terminated }
        } else {
            TokenKind::StringLiteral {
                encoding,
                terminated,
            }
        };
        self.literal_token(
            kind,
            Literal::String(StringLiteral {
                encoding,
                delimiter,
                units,
            }),
            start,
        )
    }

    fn lex_escape(
        &mut self,
        encoding: StringEncoding,
        diagnostics: &mut DiagnosticSink,
        units: &mut Vec<u32>,
    ) {
        let escape_start = self.pos;
        self.pos += 1;
        let Some(byte) = self.peek() else {
            self.error(
                diagnostics,
                codes::INVALID_ESCAPE,
                "unterminated escape sequence",
                escape_start,
                self.pos,
            );
            return;
        };

        if byte == b'{' {
            if encoding != StringEncoding::Utf8 {
                self.pos += 1;
                self.error(
                    diagnostics,
                    codes::INVALID_ESCAPE,
                    "`${...}` escapes are only valid in UTF-8 string literals",
                    escape_start,
                    self.pos,
                );
                return;
            }
            self.lex_braced_scalar(escape_start, diagnostics, units);
            return;
        }

        if let Some(scalar) = named_escape(byte) {
            self.pos += 1;
            push_scalar(encoding, units, scalar);
            return;
        }

        if byte.is_ascii_hexdigit() {
            self.lex_hex_escape(encoding, escape_start, diagnostics, units);
            return;
        }

        self.bump_any();
        let text = &self.source[escape_start..self.pos];
        self.error(
            diagnostics,
            codes::INVALID_ESCAPE,
            format!("unknown escape sequence `{text}`"),
            escape_start,
            self.pos,
        );
    }

    fn lex_hex_escape(
        &mut self,
        encoding: StringEncoding,
        escape_start: usize,
        diagnostics: &mut DiagnosticSink,
        units: &mut Vec<u32>,
    ) {
        let mut count = 0;
        while self
            .peek_at(count)
            .is_some_and(|byte| byte.is_ascii_hexdigit())
        {
            count += 1;
        }

        match encoding {
            StringEncoding::Sbcs => {
                if count >= 4 {
                    let value = self.take_hex(4);
                    let _ = value;
                    self.error(
                        diagnostics,
                        codes::INVALID_ESCAPE,
                        "a single-byte literal escape uses two hexadecimal digits ($HH), not four",
                        escape_start,
                        self.pos,
                    );
                } else if count == 2 {
                    units.push(self.take_hex(2));
                } else {
                    self.pos += count;
                    self.error(
                        diagnostics,
                        codes::INVALID_ESCAPE,
                        format!("expected two hexadecimal digits after $, found {count}"),
                        escape_start,
                        self.pos,
                    );
                }
            }
            StringEncoding::Dbcs => {
                if count >= 4 {
                    units.push(self.take_hex(4));
                } else {
                    self.pos += count;
                    self.error(
                        diagnostics,
                        codes::INVALID_ESCAPE,
                        format!(
                            "a double-byte literal escape uses four hexadecimal digits ($HHHH), found {count}"
                        ),
                        escape_start,
                        self.pos,
                    );
                }
            }
            StringEncoding::Utf8 => {
                if count >= 4 {
                    let value = self.take_hex(4);
                    if is_unicode_scalar(value) {
                        units.push(value);
                    } else {
                        self.error(
                            diagnostics,
                            codes::INVALID_SCALAR_ESCAPE,
                            format!("escape denotes invalid Unicode scalar U+{value:04X}"),
                            escape_start,
                            self.pos,
                        );
                    }
                } else if count == 2 {
                    units.push(self.take_hex(2));
                } else {
                    self.pos += count;
                    self.error(
                        diagnostics,
                        codes::INVALID_ESCAPE,
                        format!(
                            "expected two or four hexadecimal digits after $, or a ${{...}} escape, found {count}"
                        ),
                        escape_start,
                        self.pos,
                    );
                }
            }
        }
    }

    fn lex_braced_scalar(
        &mut self,
        escape_start: usize,
        diagnostics: &mut DiagnosticSink,
        units: &mut Vec<u32>,
    ) {
        self.pos += 1; // '{'
        let digits_start = self.pos;
        let mut digits = 0usize;
        let mut too_many = false;
        while let Some(byte) = self.peek() {
            if !byte.is_ascii_hexdigit() {
                break;
            }
            if digits < 6 {
                digits += 1;
            } else {
                too_many = true;
            }
            self.pos += 1;
        }
        let closed = self.peek() == Some(b'}');
        if closed {
            self.pos += 1;
        }

        if !closed {
            self.error(
                diagnostics,
                codes::INVALID_SCALAR_ESCAPE,
                "unterminated `${...}` escape; expected `}`",
                escape_start,
                self.pos,
            );
            return;
        }
        if digits == 0 {
            self.error(
                diagnostics,
                codes::INVALID_SCALAR_ESCAPE,
                "`${...}` escape requires 1 to 6 hexadecimal digits",
                escape_start,
                self.pos,
            );
            return;
        }
        if too_many {
            self.error(
                diagnostics,
                codes::INVALID_SCALAR_ESCAPE,
                "`${...}` escape accepts at most 6 hexadecimal digits",
                escape_start,
                self.pos,
            );
            return;
        }

        let mut value = 0u32;
        for byte in &self.bytes[digits_start..self.pos - 1] {
            value = value * 16 + u32::from(hex_value(*byte).unwrap_or(0));
        }
        if is_unicode_scalar(value) {
            units.push(value);
        } else {
            self.error(
                diagnostics,
                codes::INVALID_SCALAR_ESCAPE,
                format!("escape denotes invalid Unicode scalar U+{value:04X}"),
                escape_start,
                self.pos,
            );
        }
    }

    fn take_hex(&mut self, count: usize) -> u32 {
        let mut value = 0u32;
        for _ in 0..count {
            if let Some(byte) = self.peek() {
                value = value * 16 + u32::from(hex_value(byte).unwrap_or(0));
                self.pos += 1;
            }
        }
        value
    }

    // ------------------------------------------------------------------
    // Direct variables (Tokenization §6)
    // ------------------------------------------------------------------

    fn lex_percent(&mut self, start: usize, diagnostics: &mut DiagnosticSink) -> Token {
        if let Some(token) = self.try_direct_variable(start, diagnostics) {
            token
        } else {
            self.pos = start + 1;
            self.plain_token(TokenKind::Punct(Punct::Percent), start)
        }
    }

    fn try_direct_variable(
        &mut self,
        start: usize,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Token> {
        let saved = self.pos;
        self.pos += 1; // '%'

        let area = match self.peek().map(|byte| byte.to_ascii_uppercase()) {
            Some(b'I') => {
                self.pos += 1;
                Some(DirectArea::Input)
            }
            Some(b'Q') => {
                self.pos += 1;
                Some(DirectArea::Output)
            }
            Some(b'M') => {
                self.pos += 1;
                Some(DirectArea::Memory)
            }
            _ => None,
        };

        let size = match self.peek().map(|byte| byte.to_ascii_uppercase()) {
            Some(b'X') => {
                self.pos += 1;
                Some(DirectSize::Bit)
            }
            Some(b'B') => {
                self.pos += 1;
                Some(DirectSize::Byte)
            }
            Some(b'W') => {
                self.pos += 1;
                Some(DirectSize::Word)
            }
            Some(b'D') => {
                self.pos += 1;
                Some(DirectSize::Dword)
            }
            Some(b'L') => {
                self.pos += 1;
                Some(DirectSize::Lword)
            }
            _ => None,
        };

        if self.peek() == Some(b'*') {
            if area.is_some() && size.is_none() {
                self.pos += 1;
                return Some(self.literal_token(
                    TokenKind::DirectVariable,
                    Literal::Direct(DirectVariable {
                        area,
                        size: None,
                        wildcard: true,
                        indices: Vec::new(),
                    }),
                    start,
                ));
            }
            self.pos = saved;
            return None;
        }

        let mut indices = Vec::new();
        loop {
            if !self.peek().is_some_and(|b| b.is_ascii_digit()) {
                if indices.is_empty() {
                    self.pos = saved;
                    return None;
                }
                break;
            }
            let digits_start = self.pos;
            while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                self.pos += 1;
            }
            let digits = &self.source[digits_start..self.pos];
            match digits.parse::<u32>() {
                Ok(index) => indices.push(index),
                Err(_) => {
                    while self.peek().is_some_and(|b| b.is_ascii_digit() || b == b'.') {
                        self.pos += 1;
                    }
                    self.error(
                        diagnostics,
                        codes::MALFORMED_LITERAL,
                        "direct variable index exceeds the 32-bit address range",
                        start,
                        self.pos,
                    );
                    return Some(self.error_token(start));
                }
            }
            if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
                self.pos += 1;
            } else {
                break;
            }
        }

        Some(self.literal_token(
            TokenKind::DirectVariable,
            Literal::Direct(DirectVariable {
                area,
                size,
                wildcard: false,
                indices,
            }),
            start,
        ))
    }

    // ------------------------------------------------------------------
    // Pragmas (Tokenization §8)
    // ------------------------------------------------------------------

    fn lex_pragma(&mut self, start: usize, diagnostics: &mut DiagnosticSink) -> Token {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Mode {
            Outside,
            Double,
            Single,
        }

        let mut mode = Mode::Outside;
        let mut depth = 1usize;
        self.pos += 1;
        let mut terminated = false;
        while !self.at_end() {
            match mode {
                Mode::Outside => match self.peek() {
                    Some(b'{') => {
                        self.pos += 1;
                        depth += 1;
                    }
                    Some(b'}') => {
                        self.pos += 1;
                        depth -= 1;
                        if depth == 0 {
                            terminated = true;
                            break;
                        }
                    }
                    Some(b'"') => {
                        self.pos += 1;
                        mode = Mode::Double;
                    }
                    Some(b'\'') => {
                        self.pos += 1;
                        mode = Mode::Single;
                    }
                    _ => self.bump_any(),
                },
                Mode::Double => match self.peek() {
                    Some(b'\\') => {
                        self.pos += 1;
                        if !self.at_end() {
                            self.bump_any();
                        }
                    }
                    Some(b'"') => {
                        self.pos += 1;
                        mode = Mode::Outside;
                    }
                    _ => self.bump_any(),
                },
                Mode::Single => match self.peek() {
                    Some(b'\\') => {
                        self.pos += 1;
                        if !self.at_end() {
                            self.bump_any();
                        }
                    }
                    Some(b'\'') => {
                        self.pos += 1;
                        mode = Mode::Outside;
                    }
                    _ => self.bump_any(),
                },
            }
        }

        self.report_nuls(diagnostics, start, self.pos, "a pragma");
        if !terminated {
            self.error(
                diagnostics,
                codes::UNTERMINATED_PRAGMA,
                "unterminated pragma",
                start,
                self.pos,
            );
        }
        self.plain_token(TokenKind::Pragma { terminated }, start)
    }

    // ------------------------------------------------------------------
    // Punctuation (Tokenization §9)
    // ------------------------------------------------------------------

    fn lex_punct_or_error(&mut self, start: usize, diagnostics: &mut DiagnosticSink) -> Token {
        let Some(byte) = self.peek() else {
            return self.error_token(start);
        };
        if !byte.is_ascii() {
            let character = self.bump_char().unwrap_or('\u{fffd}');
            self.error(
                diagnostics,
                codes::INVALID_CHARACTER,
                format!(
                    "invalid character {character:?} (U+{:04X}) outside a comment or string literal",
                    character as u32
                ),
                start,
                self.pos,
            );
            return self.error_token(start);
        }

        let next = self.peek_at(1);
        let punct = match (byte, next) {
            (b':', Some(b'=')) => Some((Punct::Assign, 2)),
            (b'=', Some(b'>')) => Some((Punct::OutputAssign, 2)),
            (b'?', Some(b'=')) => Some((Punct::AttemptAssign, 2)),
            (b'*', Some(b'*')) => Some((Punct::Power, 2)),
            (b'.', Some(b'.')) => Some((Punct::Range, 2)),
            (b'<', Some(b'>')) => Some((Punct::NotEqual, 2)),
            (b'<', Some(b'=')) => Some((Punct::LessEqual, 2)),
            (b'>', Some(b'=')) => Some((Punct::GreaterEqual, 2)),
            (b'(', _) => Some((Punct::LParen, 1)),
            (b')', _) => Some((Punct::RParen, 1)),
            (b'[', _) => Some((Punct::LBracket, 1)),
            (b']', _) => Some((Punct::RBracket, 1)),
            (b';', _) => Some((Punct::Semicolon, 1)),
            (b',', _) => Some((Punct::Comma, 1)),
            (b':', _) => Some((Punct::Colon, 1)),
            (b'.', _) => Some((Punct::Dot, 1)),
            (b'#', _) => Some((Punct::Hash, 1)),
            (b'@', _) => Some((Punct::At, 1)),
            (b'^', _) => Some((Punct::Caret, 1)),
            (b'+', _) => Some((Punct::Plus, 1)),
            (b'-', _) => Some((Punct::Minus, 1)),
            (b'*', _) => Some((Punct::Star, 1)),
            (b'/', _) => Some((Punct::Slash, 1)),
            (b'=', _) => Some((Punct::Equal, 1)),
            (b'<', _) => Some((Punct::Less, 1)),
            (b'>', _) => Some((Punct::Greater, 1)),
            (b'&', _) => Some((Punct::Ampersand, 1)),
            _ => None,
        };

        if let Some((punct, length)) = punct {
            self.pos += length;
            return self.plain_token(TokenKind::Punct(punct), start);
        }

        self.bump_any();
        let character = self.source[start..self.pos]
            .chars()
            .next()
            .unwrap_or('\u{fffd}');
        self.error(
            diagnostics,
            codes::INVALID_CHARACTER,
            format!(
                "invalid character {character:?} (U+{:04X})",
                character as u32
            ),
            start,
            self.pos,
        );
        self.error_token(start)
    }
}

fn append_trivia(trivia: &mut Vec<Trivia>, items: &[Trivia]) -> TriviaRange {
    let start = trivia.len();
    trivia.extend_from_slice(items);
    TriviaRange::new(start, trivia.len())
}

fn duration_long(normalized: &str) -> Option<bool> {
    match normalized {
        "T" | "TIME" => Some(false),
        "LT" | "LTIME" => Some(true),
        _ => None,
    }
}

fn date_long(normalized: &str) -> Option<bool> {
    match normalized {
        "DATE" | "D" => Some(false),
        "LDATE" | "LD" => Some(true),
        _ => None,
    }
}

fn tod_long(normalized: &str) -> Option<bool> {
    match normalized {
        "TIME_OF_DAY" | "TOD" => Some(false),
        "LTOD" | "LTIME_OF_DAY" => Some(true),
        _ => None,
    }
}

fn datetime_long(normalized: &str) -> Option<bool> {
    match normalized {
        "DATE_AND_TIME" | "DT" => Some(false),
        "LDT" | "LDATE_AND_TIME" => Some(true),
        _ => None,
    }
}

fn char_kind(normalized: &str) -> Option<CharKind> {
    match normalized {
        "CHAR" => Some(CharKind::Char),
        "WCHAR" => Some(CharKind::WChar),
        "UCHAR" => Some(CharKind::UChar),
        _ => None,
    }
}

fn char_encoding(kind: CharKind) -> StringEncoding {
    match kind {
        CharKind::Char => StringEncoding::Sbcs,
        CharKind::WChar => StringEncoding::Dbcs,
        CharKind::UChar => StringEncoding::Utf8,
    }
}

fn named_escape(byte: u8) -> Option<u32> {
    Some(match byte {
        b'$' => u32::from(b'$'),
        b'\'' => u32::from(b'\''),
        b'"' => u32::from(b'"'),
        b'L' | b'l' | b'N' | b'n' => 0x0A,
        b'P' | b'p' => 0x0C,
        b'R' | b'r' => 0x0D,
        b'T' | b't' => 0x09,
        _ => return None,
    })
}

fn push_scalar(encoding: StringEncoding, units: &mut Vec<u32>, scalar: u32) {
    match encoding {
        StringEncoding::Sbcs => {
            if scalar <= 0xFF {
                units.push(scalar);
            }
        }
        StringEncoding::Dbcs => {
            if scalar <= 0xFFFF {
                units.push(scalar);
            } else {
                let value = scalar - 0x1_0000;
                units.push(0xD800 + (value >> 10));
                units.push(0xDC00 + (value & 0x3FF));
            }
        }
        StringEncoding::Utf8 => units.push(scalar),
    }
}

fn digit_value(byte: u8, radix: Radix) -> Option<u8> {
    match radix {
        Radix::Binary => match byte {
            b'0'..=b'1' => Some(byte - b'0'),
            _ => None,
        },
        Radix::Octal => match byte {
            b'0'..=b'7' => Some(byte - b'0'),
            _ => None,
        },
        Radix::Decimal => match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            _ => None,
        },
        Radix::Hexadecimal => hex_value(byte),
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn is_unicode_scalar(value: u32) -> bool {
    value <= 0x10_FFFF && !(0xD800..=0xDFFF).contains(&value)
}

fn offset_u32(offset: usize) -> u32 {
    u32::try_from(offset).expect("source file exceeds the 4 GiB span limit")
}
