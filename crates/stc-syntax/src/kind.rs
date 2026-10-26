//! Stable [`SyntaxKind`] discriminants (AST §3.3).
//!
//! The layout is append-only: token kinds occupy the low range, node kinds
//! start at [`NODE_BASE`]. Existing values are never renumbered or reused.
//! The checked-in table in `tests/syntax_kinds.txt` is enforced by a test.

use stc_lexer::{Keyword, Punct, TokenKind};

/// First discriminant used by non-terminal node kinds.
pub const NODE_BASE: u16 = 0x100;

macro_rules! define_syntax_kinds {
    (
        tokens {
            $($token:ident = $token_value:literal),* $(,)?
        }
        punct {
            $($punct_variant:ident => $punct:ident = $punct_value:literal),* $(,)?
        }
        keywords {
            $($keyword_variant:ident => $keyword:ident = $keyword_value:literal),* $(,)?
        }
        nodes {
            $($node:ident = $node_value:literal),* $(,)?
        }
    ) => {
        /// The syntactic kind of a CST node, token, or error leaf.
        #[repr(u16)]
        #[non_exhaustive]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum SyntaxKind {
            $($token = $token_value,)*
            $($punct_variant = $punct_value,)*
            $($keyword_variant = $keyword_value,)*
            $($node = $node_value,)*
        }

        impl SyntaxKind {
            /// Every kind, in declaration (discriminant) order.
            pub const ALL: &'static [SyntaxKind] = &[
                $(Self::$token,)*
                $(Self::$punct_variant,)*
                $(Self::$keyword_variant,)*
                $(Self::$node,)*
            ];

            /// Returns the stable canonical name.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$token => stringify!($token),)*
                    $(Self::$punct_variant => stringify!($punct_variant),)*
                    $(Self::$keyword_variant => stringify!($keyword_variant),)*
                    $(Self::$node => stringify!($node),)*
                }
            }

            /// Maps a lexer keyword to its distinct token kind.
            #[must_use]
            pub const fn from_keyword(keyword: Keyword) -> Self {
                match keyword {
                    $(Keyword::$keyword => Self::$keyword_variant,)*
                }
            }

            /// Maps lexer punctuation to its distinct token kind.
            #[must_use]
            pub const fn from_punct(punct: Punct) -> Self {
                match punct {
                    $(Punct::$punct => Self::$punct_variant,)*
                }
            }

            /// Returns `true` for a keyword token kind.
            #[must_use]
            pub const fn is_keyword(self) -> bool {
                match self {
                    $(Self::$keyword_variant => true,)*
                    _ => false,
                }
            }
        }
    };
}

define_syntax_kinds! {
    tokens {
        Eof = 1,
        Ident = 2,
        IntLiteral = 3,
        RealLiteral = 4,
        BasedLiteral = 5,
        TypedLiteral = 6,
        StringLiteral = 7,
        UStringLiteral = 8,
        CharLiteral = 9,
        DurationLiteral = 10,
        DateLiteral = 11,
        TodLiteral = 12,
        DateTimeLiteral = 13,
        DirectVariable = 14,
        PragmaToken = 15,
        ErrorToken = 16,
    }
    punct {
        PunctLParen => LParen = 32,
        PunctRParen => RParen = 33,
        PunctLBracket => LBracket = 34,
        PunctRBracket => RBracket = 35,
        PunctSemicolon => Semicolon = 36,
        PunctComma => Comma = 37,
        PunctColon => Colon = 38,
        PunctDot => Dot = 39,
        PunctHash => Hash = 40,
        PunctAt => At = 41,
        PunctAssign => Assign = 42,
        PunctOutputAssign => OutputAssign = 43,
        PunctAttemptAssign => AttemptAssign = 44,
        PunctPower => Power = 45,
        PunctRange => Range = 46,
        PunctNotEqual => NotEqual = 47,
        PunctLessEqual => LessEqual = 48,
        PunctGreaterEqual => GreaterEqual = 49,
        PunctCaret => Caret = 50,
        PunctPlus => Plus = 51,
        PunctMinus => Minus = 52,
        PunctStar => Star = 53,
        PunctSlash => Slash = 54,
        PunctEqual => Equal = 55,
        PunctLess => Less = 56,
        PunctGreater => Greater = 57,
        PunctAmpersand => Ampersand = 58,
        PunctPercent => Percent = 59,
    }
    keywords {
        KwBool => Bool = 64,
        KwSint => Sint = 65,
        KwInt => Int = 66,
        KwDint => Dint = 67,
        KwLint => Lint = 68,
        KwUsint => Usint = 69,
        KwUint => Uint = 70,
        KwUdint => Udint = 71,
        KwUlint => Ulint = 72,
        KwByte => Byte = 73,
        KwWord => Word = 74,
        KwDword => Dword = 75,
        KwLword => Lword = 76,
        KwReal => Real = 77,
        KwLreal => Lreal = 78,
        KwTime => Time = 79,
        KwLtime => Ltime = 80,
        KwDate => Date = 81,
        KwLdate => Ldate = 82,
        KwLdateAndTime => LdateAndTime = 83,
        KwTimeOfDay => TimeOfDay = 84,
        KwTod => Tod = 85,
        KwLtod => Ltod = 86,
        KwLtimeOfDay => LtimeOfDay = 87,
        KwDateAndTime => DateAndTime = 88,
        KwDt => Dt = 89,
        KwLdt => Ldt = 90,
        KwString => String = 91,
        KwWstring => Wstring = 92,
        KwChar => Char = 93,
        KwWchar => Wchar = 94,
        KwAny => Any = 95,
        KwAnyDerived => AnyDerived = 96,
        KwAnyElementary => AnyElementary = 97,
        KwAnyMagnitude => AnyMagnitude = 98,
        KwAnyNum => AnyNum = 99,
        KwAnyReal => AnyReal = 100,
        KwAnyInt => AnyInt = 101,
        KwAnyUnsigned => AnyUnsigned = 102,
        KwAnySigned => AnySigned = 103,
        KwAnyDuration => AnyDuration = 104,
        KwAnyBit => AnyBit = 105,
        KwAnyChars => AnyChars = 106,
        KwAnyString => AnyString = 107,
        KwAnyChar => AnyChar = 108,
        KwAnyDate => AnyDate = 109,
        KwUstring => Ustring = 110,
        KwUchar => Uchar = 111,
        KwType => Type = 112,
        KwEndType => EndType = 113,
        KwStruct => Struct = 114,
        KwEndStruct => EndStruct = 115,
        KwOverlap => Overlap = 116,
        KwArray => Array = 117,
        KwOf => Of = 118,
        KwAt => At = 119,
        KwVar => Var = 120,
        KwEndVar => EndVar = 121,
        KwVarInput => VarInput = 122,
        KwVarOutput => VarOutput = 123,
        KwVarInOut => VarInOut = 124,
        KwVarTemp => VarTemp = 125,
        KwVarGlobal => VarGlobal = 126,
        KwVarExternal => VarExternal = 127,
        KwVarAccess => VarAccess = 128,
        KwVarConfig => VarConfig = 129,
        KwConstant => Constant = 130,
        KwRetain => Retain = 131,
        KwNonRetain => NonRetain = 132,
        KwREdge => REdge = 133,
        KwFEdge => FEdge = 134,
        KwRefTo => RefTo = 135,
        KwRef => Ref = 136,
        KwNull => Null = 137,
        KwFunction => Function = 138,
        KwEndFunction => EndFunction = 139,
        KwFunctionBlock => FunctionBlock = 140,
        KwEndFunctionBlock => EndFunctionBlock = 141,
        KwProgram => Program = 142,
        KwEndProgram => EndProgram = 143,
        KwClass => Class = 144,
        KwEndClass => EndClass = 145,
        KwInterface => Interface = 146,
        KwEndInterface => EndInterface = 147,
        KwMethod => Method = 148,
        KwEndMethod => EndMethod = 149,
        KwExtends => Extends = 150,
        KwImplements => Implements = 151,
        KwOverride => Override = 152,
        KwFinal => Final = 153,
        KwAbstract => Abstract = 154,
        KwPublic => Public = 155,
        KwPrivate => Private = 156,
        KwProtected => Protected = 157,
        KwInternal => Internal = 158,
        KwThis => This = 159,
        KwSuper => Super = 160,
        KwNamespace => Namespace = 161,
        KwEndNamespace => EndNamespace = 162,
        KwUsing => Using = 163,
        KwIf => If = 164,
        KwThen => Then = 165,
        KwElsif => Elsif = 166,
        KwElse => Else = 167,
        KwEndIf => EndIf = 168,
        KwCase => Case = 169,
        KwEndCase => EndCase = 170,
        KwFor => For = 171,
        KwTo => To = 172,
        KwBy => By = 173,
        KwDo => Do = 174,
        KwEndFor => EndFor = 175,
        KwWhile => While = 176,
        KwEndWhile => EndWhile = 177,
        KwRepeat => Repeat = 178,
        KwUntil => Until = 179,
        KwEndRepeat => EndRepeat = 180,
        KwExit => Exit = 181,
        KwContinue => Continue = 182,
        KwReturn => Return = 183,
        KwTrue => True = 184,
        KwFalse => False = 185,
        KwAnd => And = 186,
        KwOr => Or = 187,
        KwXor => Xor = 188,
        KwNot => Not = 189,
        KwMod => Mod = 190,
        KwConfiguration => Configuration = 191,
        KwEndConfiguration => EndConfiguration = 192,
        KwResource => Resource = 193,
        KwEndResource => EndResource = 194,
        KwOn => On = 195,
        KwTask => Task = 196,
        KwSingle => Single = 197,
        KwInterval => Interval = 198,
        KwPriority => Priority = 199,
        KwWith => With = 200,
        KwReadOnly => ReadOnly = 201,
        KwReadWrite => ReadWrite = 202,
        KwAction => Action = 203,
        KwEndAction => EndAction = 204,
        KwStep => Step = 205,
        KwEndStep => EndStep = 206,
        KwInitialStep => InitialStep = 207,
        KwTransition => Transition = 208,
        KwEndTransition => EndTransition = 209,
        KwFrom => From = 210,
        KwPointer => Pointer = 211,
        KwAdr => Adr = 212,
        KwSizeof => Sizeof = 213,
    }
    nodes {
        SourceFile = 256,
        UsingDirective = 257,
        NamespaceDecl = 258,
        DataTypeDecl = 259,
        SimpleTypeDecl = 260,
        SubrangeTypeDecl = 261,
        EnumTypeDecl = 262,
        EnumValue = 263,
        NamedValueTypeDecl = 264,
        NamedValue = 265,
        ArrayTypeDecl = 266,
        StructTypeDecl = 267,
        StructElem = 268,
        StringTypeDecl = 269,
        RefTypeDecl = 270,
        VarSection = 271,
        VarDecl = 272,
        EdgeDecl = 273,
        AccessDecl = 274,
        FunctionDecl = 275,
        FunctionBlockDecl = 276,
        ProgramDecl = 277,
        ClassDecl = 278,
        InterfaceDecl = 279,
        MethodDecl = 280,
        MethodPrototype = 281,
        ConfigDecl = 282,
        ResourceDecl = 283,
        TaskConfig = 284,
        TaskAttr = 285,
        ProgramConfig = 286,
        ProgConfElem = 287,
        VarAccessSection = 288,
        ConfigInit = 289,
        ConfigInstInit = 290,
        StmtList = 291,
        AssignStmt = 292,
        RefAssignStmt = 293,
        AssignmentAttemptStmt = 294,
        CallStmt = 295,
        ReturnStmt = 296,
        IfStmt = 297,
        CaseStmt = 298,
        CaseSelection = 299,
        CaseElement = 300,
        ForStmt = 301,
        WhileStmt = 302,
        RepeatStmt = 303,
        ExitStmt = 304,
        ContinueStmt = 305,
        EmptyStmt = 306,
        BinaryExpr = 307,
        UnaryExpr = 308,
        CallExpr = 309,
        MemberExpr = 310,
        IndexExpr = 311,
        DerefExpr = 312,
        ParenExpr = 313,
        LiteralExpr = 314,
        PathExpr = 315,
        RefValueExpr = 316,
        Name = 317,
        Path = 318,
        TypeRef = 319,
        ParamList = 320,
        Param = 321,
        Pragma = 322,
        Error = 323,
        ArrayInit = 324,
        StructInit = 325,
        StructFieldInit = 326,
        ArrayBound = 327,
        ElseBranch = 328,
        CaseLabel = 329,
        AddressOfExpr = 330,
        SizeOfExpr = 331,
        PointerConversionExpr = 332,
    }
}

impl SyntaxKind {
    /// Maps a lexer token kind to its CST leaf kind.
    #[must_use]
    pub fn from_token(kind: &TokenKind) -> Self {
        match kind {
            TokenKind::Eof => Self::Eof,
            TokenKind::Ident(_) => Self::Ident,
            TokenKind::Keyword(keyword) => Self::from_keyword(*keyword),
            TokenKind::IntLiteral => Self::IntLiteral,
            TokenKind::RealLiteral => Self::RealLiteral,
            TokenKind::BasedLiteral => Self::BasedLiteral,
            TokenKind::TypedLiteral => Self::TypedLiteral,
            TokenKind::StringLiteral { .. } => Self::StringLiteral,
            TokenKind::UStringLiteral { .. } => Self::UStringLiteral,
            TokenKind::CharLiteral { .. } => Self::CharLiteral,
            TokenKind::DurationLiteral => Self::DurationLiteral,
            TokenKind::DateLiteral => Self::DateLiteral,
            TokenKind::TodLiteral => Self::TodLiteral,
            TokenKind::DateTimeLiteral => Self::DateTimeLiteral,
            TokenKind::DirectVariable => Self::DirectVariable,
            TokenKind::Pragma { .. } => Self::PragmaToken,
            TokenKind::Punct(punct) => Self::from_punct(*punct),
            TokenKind::Error => Self::ErrorToken,
        }
    }

    /// Returns `true` for a token or error leaf.
    #[must_use]
    pub const fn is_token(self) -> bool {
        (self as u16) < NODE_BASE
    }

    /// Returns `true` for a non-terminal node.
    #[must_use]
    pub const fn is_node(self) -> bool {
        (self as u16) >= NODE_BASE
    }
}

#[cfg(test)]
mod tests {
    use super::{NODE_BASE, SyntaxKind};

    #[test]
    fn token_and_node_ranges_do_not_overlap() {
        for kind in SyntaxKind::ALL {
            assert_eq!(kind.is_token(), !kind.is_node(), "{kind:?}");
            assert_eq!(kind.is_node(), (*kind as u16) >= NODE_BASE);
        }
    }

    #[test]
    fn every_keyword_maps_to_a_distinct_kind() {
        let mut kinds = std::collections::BTreeSet::new();
        for keyword in stc_lexer::Keyword::ALL {
            let kind = SyntaxKind::from_keyword(*keyword);
            assert!(kind.is_token());
            assert!(kinds.insert(kind), "duplicate kind for {keyword}");
        }
        assert_eq!(kinds.len(), stc_lexer::Keyword::ALL.len());
    }

    #[test]
    fn every_punct_maps_to_a_distinct_kind() {
        let mut kinds = std::collections::BTreeSet::new();
        for punct in stc_lexer::Punct::ALL {
            let kind = SyntaxKind::from_punct(*punct);
            assert!(kind.is_token());
            assert!(kinds.insert(kind), "duplicate kind for {punct:?}");
        }
        assert_eq!(kinds.len(), stc_lexer::Punct::ALL.len());
    }

    #[test]
    fn every_token_category_maps_to_a_token_kind() {
        let cases = [
            stc_lexer::TokenKind::Eof,
            stc_lexer::TokenKind::Ident("X".into()),
            stc_lexer::TokenKind::IntLiteral,
            stc_lexer::TokenKind::RealLiteral,
            stc_lexer::TokenKind::BasedLiteral,
            stc_lexer::TokenKind::TypedLiteral,
            stc_lexer::TokenKind::StringLiteral {
                encoding: stc_lexer::StringEncoding::Sbcs,
                terminated: true,
            },
            stc_lexer::TokenKind::UStringLiteral { terminated: true },
            stc_lexer::TokenKind::CharLiteral {
                kind: stc_lexer::CharKind::Char,
                terminated: true,
            },
            stc_lexer::TokenKind::DurationLiteral,
            stc_lexer::TokenKind::DateLiteral,
            stc_lexer::TokenKind::TodLiteral,
            stc_lexer::TokenKind::DateTimeLiteral,
            stc_lexer::TokenKind::DirectVariable,
            stc_lexer::TokenKind::Pragma { terminated: true },
            stc_lexer::TokenKind::Error,
        ];
        for kind in cases {
            assert!(SyntaxKind::from_token(&kind).is_token(), "{kind:?}");
        }
    }
}
