# Conformance Profile: `stc-2013-oop-utf8-v1`

Status: Draft (target profile; no compiler code implemented yet)  
Applies to: `stc` v1  
Normative reference: IEC 61131-3:2013 (Ed. 3.0), Structured Text portions  
Project extensions: `STC-UTF8`, `STC-PRAGMA`, `STC-TEST`

## 1. What this profile claims

`stc` implements a **documented subset of IEC 61131-3:2013 Structured Text**, extended with:

1. a project-defined UTF-8 string facility (`USTRING`, `UCHAR`, and associated functions), and
2. project pragmas for export and testing.

It does **not** claim conformance to IEC 61131-3:2025 (Ed. 4.0). The Ed. 4 text is not available
to this project. Public descriptions of Ed. 4 were used only as design input for the UTF-8
extension; the extension's semantics are defined by this project, not copied from Ed. 4.

The project also does **not** claim conformance to the complete 2013 edition: only the feature
set below is implemented. Any unimplemented construct in the accepted surface is a bug; any
construct outside the surface must produce an explicit diagnostic.

## 2. Input languages

| Language | Status |
| --- | --- |
| Structured Text (ST) | Implemented |
| Instruction List (IL) | Not accepted as input |
| Ladder Diagram (LD) | Not accepted as input |
| Function Block Diagram (FBD) | Not accepted as input |
| Sequential Function Chart (SFC) | Not accepted as input |

ST bodies are the only POU bodies accepted. The configuration/resource/task declarations are
parsed because they define the execution model, but graphical bodies and SFC actions are not.

## 3. Lexical surface

| Feature | Status | Notes |
| --- | --- | --- |
| UTF-8 source files | Implemented | Optional BOM; invalid UTF-8 is an error |
| Case-insensitive keywords and identifiers | Implemented | Original spelling preserved for diagnostics |
| ASCII identifier alphabet | Implemented (v1) | `[A-Za-z_][A-Za-z0-9_]*`; underscore rules per 2013 |
| Unicode identifiers | Not in v1 | See Q-008 |
| `//` line comments | Implemented | |
| `(* ... *)` nested block comments | Implemented | |
| `/* ... */` nested block comments | Implemented | |
| Pragmas `{ ... }` | Lexed as opaque trivia/tokens | `stc:` pragmas interpreted; others ignored with a warning |
| Numeric literals (decimal, real, exponent, based 2/8/16, underscores) | Implemented | `8#` is accepted from 2013 but flagged deprecated |
| Typed numeric literals (`INT#`, `REAL#`, `WORD#`, ...) | Implemented | Elementary type names only |
| Boolean literals (`TRUE`, `FALSE`, `0`, `1`, `BOOL#...`) | Implemented | |
| Single-byte strings (`'...'`, `STRING#'...'`) | Implemented | 2013 semantics |
| Double-byte strings (`"..."`, `WSTRING#"..."`) | Implemented | 2013 semantics |
| Duration literals (`T#`, `TIME#`, `LTIME#`) | Implemented | |
| Date/time literals (`DATE#`, `D#`, `TOD#`, `DT#`, `LDT#`, ...) | Implemented | |
| Directly represented variables (`%I`, `%Q`, `%M`) | Implemented | Runtime process image; see runtime spec |
| UTF-8 literals (`U#'...'`, `USTRING#'...'`, `UCHAR#...`) | Project extension | See §5 |

## 4. Language surface

### 4.1 Data types

| Feature | Status |
| --- | --- |
| Elementary types: BOOL, SINT/INT/DINT/LINT, USINT/UINT/UDINT/ULINT, BYTE/WORD/DWORD/LWORD, REAL/LREAL, TIME/LTIME, DATE/LDATE, TIME_OF_DAY/TOD/LTOD, DATE_AND_TIME/DT/LDT, STRING/WSTRING/CHAR/WCHAR | Implemented |
| User-defined simple types | Implemented |
| Subrange types | Implemented |
| Enumerated types | Implemented |
| Named-value types | Implemented |
| ARRAY types, static bounds | Implemented |
| ARRAY with variable bounds (`ARRAY [*]`) | Implemented for conformant parameters (`VAR_IN_OUT`, FB inputs/outputs); rejected for locals, globals, and returns |
| STRUCT types, including `OVERLAP` | Implemented |
| Reference types (`REF_TO`, `REF()`, `^`, `NULL`) | Implemented |
| Class types | Implemented |
| Interface types | Implemented |
| Generic types (`ANY_*`) | Implemented for standard function signatures; user POUs may not declare them |
| UTF-8 strings (`USTRING`, `UCHAR`) | Project extension |

### 4.2 Program organization units

| Feature | Status |
| --- | --- |
| FUNCTION (with or without a result) | Implemented |
| FUNCTION_BLOCK | Implemented |
| PROGRAM | Implemented |
| CLASS, methods, access specifiers | Implemented |
| INTERFACE, method prototypes | Implemented |
| EXTENDS / IMPLEMENTS / OVERRIDE / FINAL / ABSTRACT | Implemented |
| THIS / SUPER | Implemented |
| Assignment attempt (`?=`) | Implemented |
| NAMESPACE / USING / qualified names | Implemented |
| Configuration/resource/task/program configuration | Parsed and analysed; single-task execution in v1 |
| VAR_ACCESS | Parsed; runtime binding staged |
| Standard functions | Implemented (see §6) |
| Standard function blocks | Implemented in ST stdlib (TON, TOF, TP, CTU, CTD, CTUD, R_TRIG, F_TRIG, SR, RS) |
| Ed. 4 properties, mutex, semaphore, ASSERT | Not in profile |

### 4.3 Variable declarations

| Feature | Status |
| --- | --- |
| `VAR`, `VAR_INPUT`, `VAR_OUTPUT`, `VAR_IN_OUT`, `VAR_TEMP` | Implemented |
| `VAR_GLOBAL`, `VAR_EXTERNAL`, `VAR_ACCESS`, `VAR_CONFIG` | Implemented |
| `CONSTANT`, `RETAIN`, `NON_RETAIN` | Parsed/analysed; persistence policy in runtime spec |
| `AT` direct-variable declarations | Implemented |
| `R_EDGE` / `F_EDGE` edge variables | Implemented |
| Access specifiers on class/FB variables | Implemented |
| Initial values and constant expressions | Implemented |

### 4.4 Statements and expressions

| Feature | Status |
| --- | --- |
| Assignment `:=`, reference assignment, assignment attempt `?=` | Implemented |
| Function/FB/method calls, named and positional parameters, `=>` outputs | Implemented |
| `RETURN`, `IF`/`ELSIF`/`ELSE`, `CASE`, `FOR`/`BY`, `WHILE`, `REPEAT`/`UNTIL` | Implemented |
| `EXIT`, `CONTINUE`, empty statement | Implemented |
| Operators: `**`, unary `+`/`-`/`NOT`, `*`/`/`/`MOD`, `+`/`-`, comparisons, `&`/`AND`, `XOR`, `OR` | Implemented |
| Parenthesized expressions, array subscripts, struct access, partial bit access, dereference | Implemented |
| `A ** B` equivalent to `EXPT(A, B)` | Implemented |

## 5. UTF-8 extension (`STC-UTF8`)

The extension is deliberately small and explicit. The full definition, including validation,
length, indexing, conversions, and the ABI layout, is in
[UTF-8 Strings](UTF8-Strings.md).

### 5.1 Types

- `USTRING[n]` — Unicode string with a declared capacity of `n` **characters** (Unicode scalar
  values). Each character is stored as a 32-bit scalar, so the data area is exactly `4 * n` bytes;
  storage is static. `USTRING` without a capacity uses the implementation default (255
  characters, 1020 bytes of data). Maximum declared capacity: 65535 characters.
- `UCHAR` — one Unicode scalar value, stored as a 32-bit unsigned value. Invalid scalar values
  (surrogates, values above U+10FFFF) are rejected at compile time for literals and at runtime
  for conversions.
- `STRING`/`WSTRING`/`CHAR`/`WCHAR` keep their 2013 meanings and encodings.
- UTF-8 is the source/interchange encoding for `USTRING`; the in-memory representation is
  32-bit scalars for O(1) indexing. `LEN_CODE_UNIT` reports the UTF-8 byte length.

### 5.2 Literals

- `U#'...'`, `U#"..."`, `USTRING#'...'`, `USTRING#"..."`.
- `UCHAR#'x'`, `UCHAR#"x"`, and `UCHAR#${HEX}`.
- Raw Unicode scalars are permitted directly in UTF-8 source.
- Escapes: the 2013 escapes (`$$`, `$'`, `$"`, `$L`, `$N`, `$P`, `$R`, `$T`) plus
  `${HEX}` where `HEX` is 1–6 hexadecimal digits denoting a Unicode scalar value.
- Undecodable byte escapes are not allowed; use `${...}`.

### 5.3 Length and indexing

- `LEN(s)` returns the number of Unicode scalar values (characters).
- `LEN_CODE_UNIT(s)` returns the number of UTF-8 code units when encoded for interchange.
- `LEN_MAX(s)` returns the declared capacity in characters.
- Character access `s[i]` is 1-based, counts Unicode scalar values, and is O(1) because the
  storage is a 32-bit scalar array. Out-of-range access is a runtime error (checked).
- In-memory size is `4 + 4 * n` bytes; `LEN_CODE_UNIT` is not the storage size.
- String slicing functions (`LEFT`, `RIGHT`, `MID`, `INSERT`, `DELETE`, `REPLACE`, `FIND`, and
  `CONCAT`) operate on Unicode scalar values for `USTRING` overloads.

### 5.4 Conversions

Defined extension conversions (other conversions follow the 2013 naming rules):

- `USTRING_TO_STRING`, `STRING_TO_USTRING`
- `USTRING_TO_WSTRING`, `WSTRING_TO_USTRING`
- `UCHAR_TO_WCHAR`, `WCHAR_TO_UCHAR`
- `USTRING_TO_BYTES`, `BYTES_TO_USTRING` (UTF-8 code-unit view)
- `CHAR_TO_UCHAR`, `UCHAR_TO_CHAR` (only for scalar values representable in the target type)

Conversion failure policy: invalid UTF-8 or unrepresentable scalar values produce a runtime
error; the compiler inserts checks. A future checked/unchecked conversion split is possible.

## 6. Standard library surface

### 6.1 Functions

The 2013 standard function categories implemented:

- type conversions (numeric, bit, date/time, character),
- numerical/arithmetic (`ABS`, `SQRT`, `LN`, `LOG`, `EXP`, `SIN`, `COS`, `TAN`, `ASIN`, `ACOS`,
  `ATAN`, `ATAN2`, `EXPT`, `TRUNC`),
- arithmetic and bitwise (`ADD`, `SUB`, `MUL`, `DIV`, `MOD`, `MOVE`, `SHL`, `SHR`, `ROL`, `ROR`,
  `AND`, `OR`, `XOR`, `NOT`),
- selection (`SEL`, `MAX`, `MIN`, `LIMIT`, `MUX`),
- comparisons (`GT`, `GE`, `EQ`, `LE`, `LT`, `NE`),
- string functions (`LEN`, `LEFT`, `RIGHT`, `MID`, `CONCAT`, `INSERT`, `DELETE`, `REPLACE`,
  `FIND`), with `USTRING` overloads,
- date/time functions defined by the profile.

### 6.2 Function blocks

`SR`, `RS`, `R_TRIG`, `F_TRIG`, `CTU`, `CTD`, `CTUD`, `TP`, `TON`, `TOF`.

These are implemented in ST under `stdlib/` and compiled with the user project. Their timing
behavior uses a single cycle time latched at `stc_cycle` entry, so all timers in one scan see the
same time; tests use a mock clock with the same semantics.

### 6.3 Exact vs. approximate math

- **Exact functions** (integer arithmetic, bitwise, shifts/rotates, conversions, comparisons,
  selection, fixed-width string length) are specified in Lean; proofs and generated vectors gate
  implementation.
- **Approximate functions** (`SQRT`, `LN`, `LOG`, `EXP`, trigonometric, `EXPT`) have written
  edge-case/accuracy rules and are tested against high-precision reference vectors. No formal
  proof of the platform libm is claimed.

## 7. Explicitly out of scope

- IEC 61131-3:2025 Ed. 4 features not listed above (properties, mutex/semaphore, ASSERT, USTRING
  as standardized by Ed. 4, IL removal changes).
- Communication model, remote variables, and `VAR_ACCESS` network binding.
- Graphics languages and SFC.
- Vendor pragmas and the AX preprocessor.
- Multi-threaded task scheduling in v1.
- Human-machine interface and online debugging.
- A network package registry and version-solving resolver in v1. Local path dependencies are in
  v1; a registry/version solver is a later extension (A-009).
- Full RETAIN/PERSISTENT persistence in executable mode.

## 8. Implementation-defined choices

The following are deliberately chosen and must be documented and tested:

| Topic | Choice |
| --- | --- |
| Evaluation order | Left-to-right for operands; `AND`/`&` and `OR` short-circuit; see `STC-SEM-*` |
| Signed overflow | Checked; runtime trap (no silent wraparound) |
| Division/modulo by zero | Runtime trap |
| `MOD` sign | Result has the sign of the dividend (truncating division) |
| Real behavior | IEC 60559 binary32/binary64; NaN/infinity behavior explicitly documented |
| Date/time representation | `DATE` = days since 1970-01-01; `LDATE`/`TOD`/`LTOD`/`DT`/`LDT` = nanoseconds; `TIME`/`LTIME` = nanoseconds |
| Time source | One cycle time latched at `stc_cycle` entry; all timers in a scan see the same value |
| BOOL representation | `i8` in memory, `i1` in SSA; normalized on conversion |
| STRING representation | Length-prefixed fixed-capacity buffer; see runtime spec |
| USTRING representation | Length in characters + fixed `[n x i32]` scalar buffer; 4 bytes/character; static |
| USTRING assignment | Capacity is part of the type; cross-capacity assignment/argument passing inserts a runtime check and traps on overflow |
| Array bounds | Checked by default in hosted builds; release policy documented |
| Variable-length arrays | Conformant parameters only; descriptor points at caller storage; no dynamic allocation |
| Subrange assignment | Checked |
| Identifier alphabet | ASCII letters/digits/underscore in v1 |
| Default STRING/WSTRING/USTRING capacities | STRING/WSTRING 255 units; USTRING 255 characters (1020 bytes of data) unless declared |
| Case normalization | Uppercase ASCII for symbol keys; original spelling retained |
| Comparison associativity | All comparison operators share one precedence level, left-associative |
| Semicolon handling | Semicolons terminate simple statements; block terminators may omit a trailing semicolon before a block delimiter |
| Task scheduling | Single task in v1; multiple tasks diagnosed as unsupported |

## 9. Project model (project feature, not an IEC claim)

- A package is an `stc.toml` manifest plus sources. Dependencies are local path packages in v1,
  pinned by a committed `stc.lock` with `--locked` enforcement.
- The dependency graph must be acyclic; packages compile topologically; duplicate canonical paths
  are deduplicated.
- Package-aware namespaces: a package's root namespace is its manifest name (or dependency alias).
  `INTERNAL` is package-private; `PRIVATE` is POU/class-private; top-level declarations are
  public across packages by default.
- A separate test project is a normal package that depends on the code under test; `stc test`
  runs only the current package's test POUs.
- No network access, registry, or version solving in v1; the lockfile pins the local path graph
  (A-009).

## 10. Claim format

Any compliance statement produced by the project shall state:

- profile ID and version,
- the exact supported feature list (this document),
- the project extensions (`STC-UTF8`, `STC-PRAGMA`, `STC-TEST`),
- the implementation-defined choices in §8,
- that Ed. 4 conformance is not claimed.

The compiler should be able to emit this statement (or a machine-readable equivalent) from
`stc --compliance`.
