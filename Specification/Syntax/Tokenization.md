# Tokenization Scheme

Status: Draft  
Profile: `stc-2013-oop-utf8-v1`  
Rule IDs: `STC-LEX-*`  
Related: [Grammar](Grammar.md), [Conformance Profile](../01-Conformance-Profile.md)

This document defines the source model and the token stream consumed by the parser. It is the
normative lexical specification for `stc`. It is written independently from the IEC text; clause
references point at the 2013 edition where useful.

## 1. Source model

### 1.1 Encoding

- `STC-LEX-001`: Source files are UTF-8. An optional UTF-8 BOM (`EF BB BF`) is accepted only at
  the start of a file and is not part of the first token.
- `STC-LEX-002`: Invalid UTF-8 is a lexical error. The driver validates the file bytes before
  constructing the lexer; the lexer API receives a valid `&str`. The loader reports the byte
  offset and the offending sequence and recovers at the next valid scalar boundary.
- `STC-LEX-003`: Byte order marks elsewhere, NUL bytes outside string literals, and unpaired
  UTF-16 surrogates encoded in UTF-8 are errors.
- `STC-LEX-004`: The lexer preserves the original source bytes for every token and trivia item.
  The lossless CST must be able to reproduce the file exactly.

### 1.2 Lines and columns

- `STC-LEX-005`: Line terminators are `\n`, `\r\n`, and `\r`. They are treated as one logical
  newline for line numbering. Column numbers count Unicode scalar values (not bytes) from 1.
- `STC-LEX-006`: Spans are byte offsets `start..end` into the file. Diagnostics expose both byte
  offsets and 1-based `line:column` positions.
- `STC-LEX-007`: A `SourceMap` owns files, line starts, and span-to-location conversion. It is
  shared by the lexer, parser, analyzer, codegen, and runtime trap reporting.

### 1.3 Character classes

- `STC-LEX-008`: ASCII letters are `A-Z` and `a-z`; digits are `0-9`; underscore is `_`.
- `STC-LEX-009`: Language keywords and identifiers are case-insensitive. The lexer stores the
  original spelling and a normalized symbol key (ASCII uppercase) in the token.
- `STC-LEX-010`: Horizontal whitespace is space, tab, form feed, and vertical tab. Newlines are
  handled by §1.2. Whitespace is trivia.
- `STC-LEX-011`: Non-ASCII characters are permitted in comments and in string literals whose
  type supports them. They are not permitted in identifiers or keywords in v1 (A-013).

## 2. Token stream

### 2.1 Token envelope

Every token has:

| Field | Meaning |
| --- | --- |
| `kind` | Lexical kind (see §3) |
| `span` | Byte range in the source file |
| `text` | Original source slice |
| `leading_trivia` | Whitespace/comments before the token |
| `trailing_trivia` | Same-line whitespace/comments after the token (parser convenience) |

Pragmas are tokens, not trivia, because they carry semantics. Comments and whitespace are trivia
and are never discarded from the CST.

### 2.2 Token kinds

| Kind | Examples | Notes |
| --- | --- | --- |
| `Eof` | | Always emitted once at the end |
| `Identifier` | `motor_1`, `_12V7` | Case-insensitive lookup |
| `Keyword` | `IF`, `FUNCTION_BLOCK`, `USTRING` | Reserved, case-insensitive |
| `IntLiteral` | `0`, `123_4`, `+7` (sign handled by parser) | Decimal |
| `RealLiteral` | `0.0`, `3.14159_26`, `1.34E-12` | Decimal point required |
| `BasedLiteral` | `2#1010`, `8#377`, `16#DEAD_BEEF` | Bit-string valued |
| `TypedLiteral` | `INT#16#7FFF`, `REAL#1.0`, `BOOL#TRUE`, `CHAR#16#41` | Elementary type prefix |
| `StringLiteral` | `'abc'`, `"abc"`, `STRING#'abc'`, `WSTRING#"abc"` | Encoding recorded |
| `UStringLiteral` | `U#'abc'`, `USTRING#'abc'` | `STC-UTF8` |
| `CharLiteral` | `CHAR#'A'`, `WCHAR#"A"`, `UCHAR#'A'` | Single scalar |
| `DurationLiteral` | `T#1s`, `TIME#25h_15m`, `LTIME#5m_30s` | |
| `DateLiteral` | `DATE#1984-06-25`, `D#1984-06-25` | |
| `TodLiteral` | `TOD#15:36:55.36` | |
| `DateTimeLiteral` | `DT#1984-06-25-15:36:55.360227400` | |
| `DirectVariable` | `%IX0.0`, `%QW28`, `%I*` | Process-image address |
| `Pragma` | `{stc:test}`, `{stc:io {"device":"d1"}}` | Raw content; balanced JSON payload; no pragma nesting |
| `Punct` | `;`, `,`, `:`, `:=`, `=>`, `?=`, `**`, `..`, `<>`, `<=`, `>=`, `^`, `(` ... | See §3 |
| `Error` | | Malformed input; parser recovery |

## 3. Keywords

Keywords are reserved in all contexts. Standard function names such as `ABS`, `LEN`, and `TON`
are **predeclared identifiers**, not keywords, except where they are also operators (`AND`,
`OR`, `XOR`, `NOT`, `MOD`).

### 3.1 Elementary and generic type keywords

```text
BOOL SINT INT DINT LINT USINT UINT UDINT ULINT
BYTE WORD DWORD LWORD REAL LREAL
TIME LTIME DATE LDATE LDATE_AND_TIME
TIME_OF_DAY TOD LTOD LTIME_OF_DAY
DATE_AND_TIME DT LDT
STRING WSTRING CHAR WCHAR
ANY ANY_DERIVED ANY_ELEMENTARY ANY_MAGNITUDE ANY_NUM ANY_REAL
ANY_INT ANY_UNSIGNED ANY_SIGNED ANY_DURATION ANY_BIT ANY_CHARS
ANY_STRING ANY_CHAR ANY_DATE
USTRING UCHAR                                  (* STC-UTF8 extension *)
```

### 3.2 Declaration and variable keywords

```text
TYPE END_TYPE STRUCT END_STRUCT OVERLAP ARRAY OF AT
VAR END_VAR VAR_INPUT VAR_OUTPUT VAR_IN_OUT VAR_TEMP
VAR_GLOBAL VAR_EXTERNAL VAR_ACCESS VAR_CONFIG
CONSTANT RETAIN NON_RETAIN R_EDGE F_EDGE
REF_TO REF NULL
```

### 3.3 POU, OOP, and namespace keywords

```text
FUNCTION END_FUNCTION FUNCTION_BLOCK END_FUNCTION_BLOCK
PROGRAM END_PROGRAM CLASS END_CLASS INTERFACE END_INTERFACE
METHOD END_METHOD EXTENDS IMPLEMENTS OVERRIDE FINAL ABSTRACT
PUBLIC PRIVATE PROTECTED INTERNAL THIS SUPER
NAMESPACE END_NAMESPACE USING
```

### 3.4 Statement and operator keywords

```text
IF THEN ELSIF ELSE END_IF
CASE END_CASE FOR TO BY DO END_FOR
WHILE END_WHILE REPEAT UNTIL END_REPEAT
EXIT CONTINUE RETURN
TRUE FALSE
AND OR XOR NOT MOD
```

### 3.5 Configuration keywords

```text
CONFIGURATION END_CONFIGURATION RESOURCE END_RESOURCE ON
TASK SINGLE INTERVAL PRIORITY WITH
READ_ONLY READ_WRITE
```

### 3.6 SFC keywords reserved but not parsed

The compiler does not accept SFC bodies, but these 2013 keywords remain reserved so that a
future SFC front end can be added without breaking identifiers:

```text
ACTION END_ACTION STEP END_STEP INITIAL_STEP TRANSITION END_TRANSITION FROM
```

### 3.7 Keyword notes

- `END_*` keywords are single tokens; they are not `END` followed by an identifier.
- `TIME_OF_DAY`, `TOD`, `LTOD`, `LTIME_OF_DAY`, `DATE_AND_TIME`, `DT`, `LDT`, and
  `LDATE_AND_TIME` may be followed immediately by `#` to form a time literal. The lexer handles
  the literal prefix before keyword classification (see §5.4).
- `T#` and `LT#` are literal prefixes, not general identifiers.

## 4. Identifiers

- `STC-LEX-012`: An identifier starts with an ASCII letter or `_`, followed by zero or more
  ASCII letters, digits, or `_`.
- `STC-LEX-013`: Multiple leading underscores, multiple consecutive embedded underscores, and a
  trailing underscore are invalid. Examples: `_MAIN` and `LIM_SW_5` are valid; `__LIM`,
  `LIM__SW5`, and `LIM_SW5_` are invalid.
- `STC-LEX-014`: Identifiers are case-insensitive. `abc`, `ABC`, and `aBc` denote the same
  symbol. The original spelling is kept for diagnostics and for emitted debug names.
- `STC-LEX-015`: A Unicode letter is not part of an identifier in v1; it is an invalid character
  outside strings/comments. This is an explicit profile restriction (A-013).
- `STC-LEX-016`: The maximum identifier length is implementation-defined; `stc` accepts at least
  255 bytes and reports a diagnostic beyond its internal limit.

## 5. Literals

### 5.1 Numeric literals

- `STC-LEX-017`: Decimal integer literals are `digit ( '_'? digit )*`. Underscores are allowed
  only between digits. Leading zeros are accepted.
- `STC-LEX-018`: Real literals require a decimal point with digits on both sides:
  `digit+ '.' digit+` and an optional exponent `[Ee][+-]?digit+`. Underscores are allowed between
  digits. Examples: `0.0`, `3.14159_26`, `-1.34E-12`.
- `STC-LEX-019`: The sign of an ordinary numeric literal is parsed as a unary operator. A sign
  immediately after a typed-literal prefix (`INT#-123`) is part of the typed literal.
- `STC-LEX-020`: Based literals are `2#`, `8#`, or `16#` followed by one or more digits of the
  base, with single underscores between digits. Letters `A-F`/`a-f` are allowed only for base 16.
  No sign is allowed. Based literals are bit-string values.
- `STC-LEX-021`: `8#` is accepted because the profile is based on 2013, but it is marked
  deprecated and produces the warning `STC-W-LEX-008`.
- `STC-LEX-022`: Typed numeric literals are `<elementary_numeric_type>'#'<numeric_literal>`.
  The prefix is case-insensitive and must name an elementary numeric, bit-string, boolean, or
  character type. Examples: `INT#-123`, `INT#16#7FFF`, `WORD#1234`, `BOOL#FALSE`, `REAL#1.0e3`.
- `STC-LEX-023`: A numeric literal whose value does not fit its declared type is a semantic error,
  not a lexical error. The lexer records the raw digits and an arbitrary-precision value where
  needed; the analyzer performs range checking.
- `STC-LEX-024`: `TRUE`, `FALSE`, `BOOL#TRUE`, `BOOL#FALSE`, `BOOL#0`, and `BOOL#1` are boolean
  literals. Other integer literals are not booleans; the analyzer applies the normal conversion
  rules.

### 5.2 Single-byte and double-byte strings

- `STC-LEX-025`: A single-byte string literal is delimited by `'...'`. A double-byte string
  literal is delimited by `"..."`.
- `STC-LEX-026`: Raw characters in a single-byte literal are restricted to printable ASCII
  (`0x20..=0x7E`) excluding `'` and `$`. Other bytes must use escapes. Raw characters in a
  double-byte literal may be any Unicode scalar value except `"` and `$`.
- `STC-LEX-027`: The escapes are:

| Escape | Meaning |
| --- | --- |
| `$$` | Dollar sign |
| `$'` | Single quote (single-quoted literals) |
| `$"` | Double quote (double-quoted literals) |
| `$L`, `$l` | Line feed |
| `$N`, `$n` | Newline |
| `$P`, `$p` | Form feed |
| `$R`, `$r` | Carriage return |
| `$T`, `$t` | Tab |
| `$HH` | One 8-bit character with hex code `HH` (single-byte literals) |
| `$HHHH` | One 16-bit character with hex code `HHHH` (double-byte literals) |

- `STC-LEX-028`: In a single-byte literal, `$` followed by four hex digits is an error. In a
  double-byte literal, `$` followed by two hex digits is an error. A lone `$` or an escape with
  the wrong number of hex digits is an error.
- `STC-LEX-029`: Raw newlines are not allowed inside string literals; use `$L`, `$N`, or `$R`.
  The lexer continues to the closing delimiter and reports an unterminated-literal error if none
  is found before EOF.
- `STC-LEX-030`: Typed string/character literals are `STRING#'...'`, `WSTRING#"..."`,
  `WSTRING#'...'`, `CHAR#'X'`, and `WCHAR#"X"`. `CHAR` and `WCHAR` literals contain exactly one
  character after escape decoding.
- `STC-LEX-031`: The empty literals `''` and `""` are valid empty strings. Empty `CHAR`/`WCHAR`
  literals are errors.

### 5.3 UTF-8 string literals (`STC-UTF8`)

The full type/length/conversion semantics are in [UTF-8 Strings](../UTF8-Strings.md).

- `STC-LEX-032`: UTF-8 string literals are `U#'...'`, `U#"..."`, `USTRING#'...'`, or
  `USTRING#"..."`. The source text is UTF-8; the literal decodes to a Unicode scalar sequence.
  `USTRING` stores that sequence as 32-bit scalars (4 bytes per character) for O(1) indexing;
  UTF-8 is the source/interchange encoding, not the in-memory encoding.
- `STC-LEX-033`: Raw characters in a UTF-8 literal may be any Unicode scalar value except the
  delimiter and `$`. The source file itself must be valid UTF-8.
- `STC-LEX-034`: The 2013 escapes are supported. In addition, `${HEX}` denotes one Unicode
  scalar value, where `HEX` is 1–6 hexadecimal digits. The value must be a valid scalar
  (not a surrogate, not above U+10FFFF). `$HH` is also accepted as U+00HH for compatibility with
  the 2013 escape table.
- `STC-LEX-035`: `UCHAR#` literals use the same quoted forms and must decode to exactly one
  Unicode scalar value. `UCHAR#${1F600}` is valid; `UCHAR#''` and `UCHAR#'ab'` are errors.
- `STC-LEX-036`: A UTF-8 literal that contains an unpaired surrogate escape or an invalid
  `${...}` value is a lexical error with the offending scalar in the message.
- `STC-LEX-037`: String equality and length operate on Unicode scalar values for `USTRING`, not
  on bytes. Normalization is **not** performed: `U+00E9` and `U+0065 U+0301` remain distinct.
  This is recorded as an implementation-defined choice.

### 5.4 Duration, date, and time literals

- `STC-LEX-038`: Duration prefixes are `T#`, `TIME#`, and `LTIME#` (case-insensitive). The body
  is an optional sign followed by one or more components:
  `d`, `h`, `m`, `s`, `ms`, `us`, `ns`, with optional fractional values on the last component.
  Components may be separated by single underscores. Examples: `T#14ms`, `TIME#25h_15m`,
  `LTIME#5d_14h_12m_18s_3.5ms`.
- `STC-LEX-039`: The prefix must be immediately followed by `#`. No whitespace is allowed between
  the type name and `#` or within the body.
- `STC-LEX-040`: Date/time prefixes are `DATE#`, `D#`, `LDATE#`, `LD#`, `TIME_OF_DAY#`, `TOD#`,
  `LTOD#`, `LTIME_OF_DAY#`, `DATE_AND_TIME#`, `DT#`, `LDT#`, and `LDATE_AND_TIME#`.
- `STC-LEX-041`: Date body: `YYYY-MM-DD`. Time-of-day body: `HH:MM:SS` with an optional
  fractional part. Date-and-time body: date, `-`, time. Underscores are allowed between digits
  in the fractional part.
- `STC-LEX-042`: The lexer validates the shape and digit counts; range validation (month, day,
  hour, minute, second, time zone-free date) is performed by the analyzer/constant evaluator.
  Out-of-range values produce an error, not a silent wrap.

## 6. Directly represented variables

- `STC-LEX-043`: A direct variable starts with `%`, followed by an optional area
  (`I`, `Q`, or `M`), an optional size (`X`, `B`, `W`, `D`, or `L`), and one or more
  non-negative decimal integers separated by `.`.
- `STC-LEX-044`: `%I*`, `%Q*`, and `%M*` wildcard forms are accepted in declarations and
  `VAR_ACCESS`. The wildcard is not allowed in an executable expression.
- `STC-LEX-045`: Examples: `%IX0.0`, `%QW28`, `%MD100`, `%I*`, `%B0` (struct-relative address).
  The analyzer checks that the address fits the declared type and target layout.
- `STC-LEX-046`: A `%` not followed by a valid direct-variable shape is a punctuation token used
  in partial bit access (`a.%X0`) or an error, depending on context.

## 7. Comments and trivia

- `STC-LEX-047`: A line comment starts with `//` and ends at the next line feed, carriage return,
  or form feed. `(*`, `*)`, `/*`, and `*/` have no special meaning inside it.
- `STC-LEX-048`: Block comments start with `(*` and end with `*)`, or start with `/*` and end
  with `*/`. They do not nest across the two styles. Within one style, nested pairs are
  supported: `(* a (* b *) c *)` and `/* a /* b */ c */`.
- `STC-LEX-049`: An unterminated block comment is an error at EOF. The lexer consumes to EOF and
  emits one `Error` token plus trivia so the CST remains complete.
- `STC-LEX-050`: Comments are trivia. They are attached to the nearest token in the CST and are
  preserved for formatting. They have no semantic effect.
- `STC-LEX-051`: `///` starts a documentation line comment. It is trivia in v1 but is tagged
  `Doc` so a future documentation/IDE feature can use it.

## 8. Pragmas

- `STC-LEX-052`: A pragma starts with `{` and ends at the `}` that returns the outer brace depth
  to zero. Pragmas do **not** nest as pragmas, but the body may contain balanced JSON braces.
  The lexer scans with a small state machine:
  - outer `{`: depth = 1;
  - outside strings, `{` increments depth and `}` decrements it; depth 0 ends the pragma;
  - inside a double-quoted JSON string, `\"` and `\\` are escapes and braces are ignored;
  - inside a single-quoted payload string, `\'` and `\\` are escapes and braces are ignored;
  - braces in comments are not special because pragma content is not lexed as ST.
  This allows `{stc:io {"device":"drive1","channel":5}}` while keeping pragmas non-nested.
- `STC-LEX-053`: Pragmas may appear anywhere whitespace is allowed, except inside a string
  literal. They are tokens so the parser can attach them to declarations/statements. A variable
  declaration may have leading pragmas and one trailing pragma before `;`; see the grammar.
- `STC-LEX-054`: A `stc:` pragma has the shape `{stc:<name> <json-value>?}` where `<name>` matches
  `[A-Za-z_][A-Za-z0-9_]*` and the optional payload is a JSON value (object, array, string,
  number, boolean, or null). Whitespace is allowed around the payload. Known names:
  - `{stc:test {"cycles": 25}}` — test declaration; optional `cycles` (positive integer),
    `timeout` (duration string), `expect-trap` (trap name string);
  - `{stc:export}` / `{stc:no_mangle}` / `{stc:inline}` — no payload;
  - `{stc:deprecated {"message": "..."}}` — deprecation note;
  - `{stc:io {...}}` — I/O binding hint; object payload; only meaningful on an `AT` declaration.
  Unknown `stc:` names are `STC-E-PRAGMA-001`. Unknown keys inside a known stc pragma are
  `STC-W-PRAGMA-002` and preserved where the schema permits. Pragmas without the `stc:` prefix
  are ignored with `STC-W-PRAGMA-001`; `{attribute '...'}` is reserved for future AX-compatible
  metadata and currently follows the same ignore path.
- `STC-LEX-055`: An unterminated pragma is `STC-E-LEX-007` at EOF.
- `STC-LEX-056`: The JSON payload is parsed by the parser, not the lexer, using a strict JSON
  parser. Invalid JSON is `STC-E-LEX-010`; duplicate object keys are `STC-E-LEX-011`; payloads
  larger than 64 KiB or nested deeper than 32 levels are `STC-E-LEX-012`.
- `STC-LEX-057`: A `stc:` pragma with a payload on a name that takes no payload, or without a
  required payload, is `STC-E-PRAGMA-002`.
- `STC-LEX-058`: The legacy parenthesized form `{stc:test(cycles := 25)}` is **not** accepted.
  Use JSON: `{stc:test {"cycles": 25}}`.

## 9. Punctuation and compound tokens

| Token | Text |
| --- | --- |
| `LParen` / `RParen` | `(` `)` |
| `LBracket` / `RBracket` | `[` `]` |
| `Semicolon` | `;` |
| `Comma` | `,` |
| `Colon` | `:` |
| `Dot` | `.` |
| `Hash` | `#` |
| `At` | `@` |
| `Assign` | `:=` |
| `OutputAssign` | `=>` |
| `AttemptAssign` | `?=` |
| `Power` | `**` |
| `Range` | `..` |
| `NotEqual` | `<>` |
| `LessEqual` / `GreaterEqual` | `<=` `>=` |
| `Caret` | `^` |
| `Plus` / `Minus` | `+` `-` |
| `Star` / `Slash` | `*` `/` |
| `Equal` / `Less` / `Greater` | `=` `<` `>` |
| `Ampersand` | `&` |
| `Percent` | `%` |

Lexing is maximal-munch: `:=` wins over `:`, `**` over `*`, `..` over `.`, `<>` over `<`, and
`<=`/`>=` over `<`/`>`.

## 10. Errors and recovery

### 10.1 Error codes

| Code | Condition |
| --- | --- |
| `STC-E-LEX-001` | Invalid UTF-8 |
| `STC-E-LEX-002` | Invalid character |
| `STC-E-LEX-003` | Invalid identifier (underscore rule) |
| `STC-E-LEX-004` | Unterminated string literal |
| `STC-E-LEX-005` | Invalid escape sequence |
| `STC-E-LEX-006` | Unterminated block comment |
| `STC-E-LEX-007` | Unterminated pragma |
| `STC-E-LEX-008` | Malformed numeric/time/date literal |
| `STC-E-LEX-009` | Invalid UTF-8 scalar escape |
| `STC-E-LEX-010` | Invalid JSON pragma payload |
| `STC-E-LEX-011` | Duplicate key in a JSON pragma payload |
| `STC-E-LEX-012` | Pragma payload exceeds size/depth limits |
| `STC-E-PRAGMA-001` | Unknown `stc:` pragma name |
| `STC-E-PRAGMA-002` | Missing or unexpected pragma payload |
| `STC-W-PRAGMA-001` | Non-`stc:` pragma ignored |
| `STC-W-PRAGMA-002` | Unknown key in a known `stc:` pragma |
| `STC-W-LEX-008` | Deprecated octal (`8#`) literal |

### 10.2 Recovery rules

- On an invalid character, emit an `Error` token for the offending scalar and continue.
- On an unterminated string/comment/pragma, emit the token/trivia through EOF, report once, and
  emit `Eof`.
- On a malformed literal, consume the longest plausible prefix, emit `Error`, and continue.
- The lexer never panics and always terminates; it is fuzz-tested with arbitrary bytes.

## 11. Lexer API contract

```rust
pub struct Lexer<'src> {
    source: &'src str,
    cursor: usize,
    file_id: FileId,
    diagnostics: &'d mut DiagnosticSink,
}

impl Lexer<'_> {
    pub fn tokenize(&mut self) -> TokenStream;
}

pub struct TokenStream {
    pub tokens: Vec<Token>,
    pub trivia: Vec<Trivia>,
    pub eof_span: Span,
}
```

- `tokenize` is total for any `&str`.
- Token and trivia spans are monotonically non-decreasing and cover the entire input exactly
  once (whitespace/comments as trivia, pragmas/tokens as tokens).
- The parser may re-scan a token's raw text for a pragma payload; the lexer does not interpret
  pragma contents.
- The lexer is independent of the parser; it can be tested and fuzzed alone.

## 12. Resolved lexical questions

No open lexical questions currently.

- **Q-008 resolved (A-013):** identifiers are ASCII-only; UTF-8 is supported in comments and
  string literals. Unicode letters outside those contexts are invalid characters.
- **Q-001 resolved (A-005):** `USTRING[n]` capacity is `n` characters; each scalar occupies
  4 bytes; storage is static; `LEN` counts characters; `LEN_CODE_UNIT` counts UTF-8 code units for
  interchange. This affects type layout, not tokenization.
