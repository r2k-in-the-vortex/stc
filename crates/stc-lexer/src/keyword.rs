use std::fmt;

macro_rules! keywords {
    ($($variant:ident => $text:literal),* $(,)?) => {
        /// A reserved IEC 61131-3 keyword (STC-LEX-009).
        ///
        /// Keywords are case-insensitive. The variant name and canonical
        /// spelling are stable; new keywords are appended.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Keyword {
            $($variant),*
        }

        impl Keyword {
            /// Every keyword in declaration order.
            pub const ALL: &'static [Keyword] = &[$(Keyword::$variant),*];

            /// Returns the canonical uppercase spelling.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Keyword::$variant => $text),*
                }
            }

            /// Looks up a normalized (uppercase ASCII) spelling.
            #[must_use]
            pub fn from_normalized(text: &str) -> Option<Self> {
                match text {
                    $($text => Some(Keyword::$variant)),*,
                    _ => None,
                }
            }
        }
    };
}

keywords! {
    // Elementary and generic types (Tokenization §3.1).
    Bool => "BOOL",
    Sint => "SINT",
    Int => "INT",
    Dint => "DINT",
    Lint => "LINT",
    Usint => "USINT",
    Uint => "UINT",
    Udint => "UDINT",
    Ulint => "ULINT",
    Byte => "BYTE",
    Word => "WORD",
    Dword => "DWORD",
    Lword => "LWORD",
    Real => "REAL",
    Lreal => "LREAL",
    Time => "TIME",
    Ltime => "LTIME",
    Date => "DATE",
    Ldate => "LDATE",
    LdateAndTime => "LDATE_AND_TIME",
    TimeOfDay => "TIME_OF_DAY",
    Tod => "TOD",
    Ltod => "LTOD",
    LtimeOfDay => "LTIME_OF_DAY",
    DateAndTime => "DATE_AND_TIME",
    Dt => "DT",
    Ldt => "LDT",
    String => "STRING",
    Wstring => "WSTRING",
    Char => "CHAR",
    Wchar => "WCHAR",
    Any => "ANY",
    AnyDerived => "ANY_DERIVED",
    AnyElementary => "ANY_ELEMENTARY",
    AnyMagnitude => "ANY_MAGNITUDE",
    AnyNum => "ANY_NUM",
    AnyReal => "ANY_REAL",
    AnyInt => "ANY_INT",
    AnyUnsigned => "ANY_UNSIGNED",
    AnySigned => "ANY_SIGNED",
    AnyDuration => "ANY_DURATION",
    AnyBit => "ANY_BIT",
    AnyChars => "ANY_CHARS",
    AnyString => "ANY_STRING",
    AnyChar => "ANY_CHAR",
    AnyDate => "ANY_DATE",
    Ustring => "USTRING",
    Uchar => "UCHAR",

    // Declarations and variables (Tokenization §3.2).
    Type => "TYPE",
    EndType => "END_TYPE",
    Struct => "STRUCT",
    EndStruct => "END_STRUCT",
    Overlap => "OVERLAP",
    Array => "ARRAY",
    Of => "OF",
    At => "AT",
    Var => "VAR",
    EndVar => "END_VAR",
    VarInput => "VAR_INPUT",
    VarOutput => "VAR_OUTPUT",
    VarInOut => "VAR_IN_OUT",
    VarTemp => "VAR_TEMP",
    VarGlobal => "VAR_GLOBAL",
    VarExternal => "VAR_EXTERNAL",
    VarAccess => "VAR_ACCESS",
    VarConfig => "VAR_CONFIG",
    Constant => "CONSTANT",
    Retain => "RETAIN",
    NonRetain => "NON_RETAIN",
    REdge => "R_EDGE",
    FEdge => "F_EDGE",
    RefTo => "REF_TO",
    Ref => "REF",
    Null => "NULL",

    // POUs, OOP, and namespaces (Tokenization §3.3).
    Function => "FUNCTION",
    EndFunction => "END_FUNCTION",
    FunctionBlock => "FUNCTION_BLOCK",
    EndFunctionBlock => "END_FUNCTION_BLOCK",
    Program => "PROGRAM",
    EndProgram => "END_PROGRAM",
    Class => "CLASS",
    EndClass => "END_CLASS",
    Interface => "INTERFACE",
    EndInterface => "END_INTERFACE",
    Method => "METHOD",
    EndMethod => "END_METHOD",
    Extends => "EXTENDS",
    Implements => "IMPLEMENTS",
    Override => "OVERRIDE",
    Final => "FINAL",
    Abstract => "ABSTRACT",
    Public => "PUBLIC",
    Private => "PRIVATE",
    Protected => "PROTECTED",
    Internal => "INTERNAL",
    This => "THIS",
    Super => "SUPER",
    Namespace => "NAMESPACE",
    EndNamespace => "END_NAMESPACE",
    Using => "USING",

    // Statements and operators (Tokenization §3.4).
    If => "IF",
    Then => "THEN",
    Elsif => "ELSIF",
    Else => "ELSE",
    EndIf => "END_IF",
    Case => "CASE",
    EndCase => "END_CASE",
    For => "FOR",
    To => "TO",
    By => "BY",
    Do => "DO",
    EndFor => "END_FOR",
    While => "WHILE",
    EndWhile => "END_WHILE",
    Repeat => "REPEAT",
    Until => "UNTIL",
    EndRepeat => "END_REPEAT",
    Exit => "EXIT",
    Continue => "CONTINUE",
    Return => "RETURN",
    True => "TRUE",
    False => "FALSE",
    And => "AND",
    Or => "OR",
    Xor => "XOR",
    Not => "NOT",
    Mod => "MOD",

    // Configuration (Tokenization §3.5).
    Configuration => "CONFIGURATION",
    EndConfiguration => "END_CONFIGURATION",
    Resource => "RESOURCE",
    EndResource => "END_RESOURCE",
    On => "ON",
    Task => "TASK",
    Single => "SINGLE",
    Interval => "INTERVAL",
    Priority => "PRIORITY",
    With => "WITH",
    ReadOnly => "READ_ONLY",
    ReadWrite => "READ_WRITE",

    // Reserved SFC keywords (Tokenization §3.6).
    Action => "ACTION",
    EndAction => "END_ACTION",
    Step => "STEP",
    EndStep => "END_STEP",
    InitialStep => "INITIAL_STEP",
    Transition => "TRANSITION",
    EndTransition => "END_TRANSITION",
    From => "FROM",
}

impl fmt::Display for Keyword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
