use std::fmt;

/// Punctuation and compound operators (Tokenization §9).
///
/// Lexing is maximal-munch: `:=` wins over `:`, `**` over `*`, `..` over `.`,
/// `<>` over `<`, and `<=`/`>=` over `<`/`>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Punct {
    LParen,
    RParen,
    LBracket,
    RBracket,
    Semicolon,
    Comma,
    Colon,
    Dot,
    Hash,
    At,
    Assign,
    OutputAssign,
    AttemptAssign,
    Power,
    Range,
    NotEqual,
    LessEqual,
    GreaterEqual,
    Caret,
    Plus,
    Minus,
    Star,
    Slash,
    Equal,
    Less,
    Greater,
    Ampersand,
    Percent,
}

impl Punct {
    /// Returns the canonical source spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LParen => "(",
            Self::RParen => ")",
            Self::LBracket => "[",
            Self::RBracket => "]",
            Self::Semicolon => ";",
            Self::Comma => ",",
            Self::Colon => ":",
            Self::Dot => ".",
            Self::Hash => "#",
            Self::At => "@",
            Self::Assign => ":=",
            Self::OutputAssign => "=>",
            Self::AttemptAssign => "?=",
            Self::Power => "**",
            Self::Range => "..",
            Self::NotEqual => "<>",
            Self::LessEqual => "<=",
            Self::GreaterEqual => ">=",
            Self::Caret => "^",
            Self::Plus => "+",
            Self::Minus => "-",
            Self::Star => "*",
            Self::Slash => "/",
            Self::Equal => "=",
            Self::Less => "<",
            Self::Greater => ">",
            Self::Ampersand => "&",
            Self::Percent => "%",
        }
    }
}

impl fmt::Display for Punct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
