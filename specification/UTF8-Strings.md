# UTF-8 String Extension (`STC-UTF8`)

Status: Draft (Q-001 resolved: fixed 4-byte scalar storage)  
Profile: `stc-2013-oop-utf8-v1`  
Related: [Tokenization](Syntax/Tokenization.md), [Grammar](Syntax/Grammar.md), [Runtime](Architecture/Runtime-and-Execution.md), [Conformance](01-Conformance-Profile.md)

## 1. Why this extension exists

The project targets IEC 61131-3:2013. That edition defines `STRING` (single-byte) and `WSTRING`
(double-byte). The 4th edition adds UTF-8 strings, but the 4th edition is not available to this
project and will not be purchased. The user requires UTF-8 strings from the start, so this
document defines a **project extension**. It is not a claim of Ed. 4 conformance and must not be
described as one.

The extension is deliberately small:

- `USTRING[n]` — fixed-capacity Unicode string. Source and interchange encoding are UTF-8; the
  in-memory representation is a static array of 32-bit Unicode scalar values (4 bytes per
  character) so indexing is O(1) and every Unicode scalar, including emoji, fits.
- `UCHAR` — one Unicode scalar value (32-bit).
- `U#'...'`, `USTRING#'...'`, `UCHAR#'x'` literals.
- `LEN`, `LEN_CODE_UNIT`, `LEN_MAX` length operations.
- Explicit conversions to/from `STRING`, `WSTRING`, `BYTE` arrays, `CHAR`, and `WCHAR`.

The 4-byte-per-character layout is a deliberate trade-off: ASCII text uses four times the memory
of a byte string, but storage is static and character indexing is direct. `LEN_CODE_UNIT` reports
the UTF-8 byte length for callers that need the wire/byte size.

## 2. Types

### 2.1 `USTRING[n]`

- A string of Unicode scalar values.
- `n` is the maximum capacity in **characters (Unicode scalar values)**. Each character is stored
  as a 32-bit scalar, so the data area is exactly `4 * n` bytes; the length field adds 4 bytes.
- `USTRING` without `[n]` uses the default capacity (255 characters, 1020 bytes of data).
- Maximum declared capacity in v1: 65535 characters (262140 bytes of data).
- The string may contain any valid Unicode scalar value, including U+0000. There is no NUL
  terminator; the length is stored separately.
- Storage is static: the capacity is fixed at declaration time and never reallocates. Capacity
  is part of the static type. `USTRING` without `[n]` uses the default capacity (255 characters)
  in every context (variables, parameters, fields); declare a larger capacity explicitly.
  Generic-capacity string parameters are not in v1.
- `USTRING` is a value type. Assignment copies the struct. `VAR_IN_OUT` passes a pointer.
- Source literals and interchange with the outside world use UTF-8; the in-memory representation
  is not UTF-8. `LEN_CODE_UNIT` reports the UTF-8 byte length when the value is encoded for
  interchange.
- There is no implicit conversion between `STRING`, `WSTRING`, and `USTRING`; use the explicit
  conversion functions.

### 2.2 `UCHAR`

- One Unicode scalar value, stored as a 32-bit unsigned value.
- Valid range: U+0000 to U+10FFFF excluding U+D800–U+DFFF (surrogates).
- `UCHAR` is not a byte and is not aliased with `CHAR` or `WCHAR`.

## 3. Literals

### 3.1 Forms

```st
u  := U#'hello';
u2 := U#"hello";
u3 := USTRING#'hello';
c  := UCHAR#'A';
c2 := UCHAR#${1F600};
```

- `U#...` and `USTRING#...` are equivalent.
- Single- or double-quoted forms are accepted; the delimiter must match.
- Raw Unicode characters are allowed directly in UTF-8 source.
- A raw NUL byte in source is not allowed; use `$00` or `${0000}`.

### 3.2 Escapes

The 2013 escapes are supported:

| Escape | Meaning |
| --- | --- |
| `$$` | `$` |
| `$'` | `'` (single-quoted form) |
| `$"` | `"` (double-quoted form) |
| `$L`, `$l` | line feed |
| `$N`, `$n` | newline |
| `$P`, `$p` | form feed |
| `$R`, `$r` | carriage return |
| `$T`, `$t` | tab |
| `$HH` | Unicode scalar U+00HH (two hex digits) |
| `${HEX}` | Unicode scalar value from 1–6 hex digits |

- `${...}` accepts uppercase or lowercase hex and 1–6 digits; leading zeros are allowed.
- The value must be a valid scalar (not a surrogate, not above U+10FFFF). Invalid values are
  compile-time errors.
- There is no `\u` escape; `$` is the escape introducer.
- Unknown escapes are errors.

### 3.3 `UCHAR` literals

- The decoded value must contain exactly one Unicode scalar value.
- `UCHAR#''`, `UCHAR#'ab'`, `UCHAR#${D800}`, and `UCHAR#${110000}` are errors.
- `UCHAR#${0}` and `UCHAR#'\0'`-style values are allowed via `${0000}`/`$00`.

## 4. Validation

A `USTRING` value is always valid UTF-8. The compiler/runtime enforces:

- no overlong encodings,
- no surrogate code points,
- no values above U+10FFFF,
- no truncated sequences,
- no invalid continuation bytes.

Construction paths are checked:

- literals are validated at compile time;
- `BYTES_TO_USTRING` validates at runtime and traps `STC_TRAP_INVALID_UTF8` on failure;
- conversions from `WSTRING` validate surrogate pairing;
- memory is never exposed for arbitrary byte mutation; `USTRING_TO_BYTES` produces a copy.

## 5. Length and indexing

| Operation | Result |
| --- | --- |
| `LEN(s)` | number of Unicode scalar values (characters) |
| `LEN_CODE_UNIT(s)` | number of UTF-8 code units if encoded for interchange |
| `LEN_MAX(s)` | declared capacity `n` in characters |
| `s[i]` | `UCHAR` at 1-based scalar index `i`; O(1), bounds-checked |
| `s[i..j]` | slice of scalars (function form below) |

- `s[i]` counts Unicode scalar values, not bytes and not grapheme clusters. It indexes directly
  into the `[n x i32]` storage in O(1).
- A combining sequence (`e` + U+0301) has length 2; it is not normalized.
- Supplementary-plane characters count as one scalar and occupy exactly 4 bytes in storage.
- In-memory size is `4 + 4 * n` bytes. `LEN_CODE_UNIT` does **not** report this size; it reports
  the UTF-8 byte length, which is computed by encoding/scalar scanning.
- Out-of-range index traps `STC_TRAP_BOUNDS`.
- An empty string has length 0; `LEN_CODE_UNIT('')` is 0; `LEN_MAX` still returns the capacity.
- A string that stores `n` ASCII characters still occupies `4 * n` data bytes.

## 6. String functions

The 2013 string functions are overloaded for `USTRING` with scalar-based positions:

| Function | `USTRING` behavior |
| --- | --- |
| `LEN` | scalar count |
| `LEN_CODE_UNIT` | UTF-8 code-unit count (extension) |
| `LEN_MAX` | capacity in characters (extension) |
| `LEFT(IN, L)` | first `L` scalars |
| `RIGHT(IN, L)` | last `L` scalars |
| `MID(IN, L, P)` | `L` scalars starting at 1-based `P` |
| `CONCAT(IN1, IN2, ...)` | concatenation; capacity checked |
| `INSERT(IN1, IN2, P)` | insert `IN2` at scalar position `P` |
| `DELETE(IN, L, P)` | delete `L` scalars at `P` |
| `REPLACE(IN1, IN2, L, P)` | replace `L` scalars at `P` |
| `FIND(IN1, IN2)` | 1-based scalar position of first occurrence, or 0 |

- All positions are 1-based; positions and lengths are in characters (Unicode scalar values).
- Capacity overflow traps `STC_TRAP_RANGE`; capacity is counted in characters, not bytes.
- Invalid ranges are trapped, not clamped, unless the individual function's 2013 definition says
  otherwise. Every function has a rule ID and tests.
- `FIND` matches scalar sequences, not bytes; the returned position is a scalar index.
- The underlying storage is a 32-bit scalar array, so slicing/copying moves 4-byte elements; the
  algorithms do not need UTF-8 decoding for indexing.

## 7. Conversions

| Function | Behavior |
| --- | --- |
| `STRING_TO_USTRING` | ASCII/single-byte -> UTF-8; always succeeds for values the profile permits |
| `USTRING_TO_STRING` | succeeds only if every scalar is representable in the single-byte profile; otherwise trap |
| `WSTRING_TO_USTRING` | validates UTF-16; converts surrogate pairs; traps on lone surrogates |
| `USTRING_TO_WSTRING` | encodes UTF-16; capacity checked in 16-bit units; traps on overflow |
| `CHAR_TO_UCHAR` | byte -> scalar U+00xx |
| `UCHAR_TO_CHAR` | succeeds only for U+0000–U+00FF in the profile; otherwise trap |
| `WCHAR_TO_UCHAR` | succeeds for non-surrogate UTF-16 units; traps on lone surrogates |
| `UCHAR_TO_WCHAR` | succeeds for BMP values; traps for supplementary-plane values |
| `USTRING_TO_BYTES` | encodes the scalar sequence as UTF-8 and returns a byte array copy |
| `BYTES_TO_USTRING` | decodes UTF-8 into 32-bit scalars; traps on invalid UTF-8 or capacity overflow |

- Conversion failure is a runtime trap, not a silent truncation. A future checked/unchecked
  variant split can be added without changing the default.
- `USTRING_TO_BYTES` and `BYTES_TO_USTRING` are project extensions; names follow the 2013
  conversion naming convention. `USTRING_TO_BYTES` output length equals `LEN_CODE_UNIT`.
- `BYTES_TO_USTRING` capacity is checked in characters; a valid UTF-8 sequence is decoded to
  scalars before storage.
- No implicit conversion is inserted between the three string encodings. `STRING`/`WSTRING`
  overload resolution does not silently select `USTRING`.

## 8. Comparison and assignment

- `=` and `<>` compare scalar sequences. For valid UTF-8, byte equality and scalar-sequence
  equality coincide; the implementation may use byte comparison after validation.
- No case-insensitive comparison in v1; use an explicit function when added.
- `USTRING` assignment copies the length and scalar array. Assignment between `USTRING[m]` and
  `USTRING[n]` is statically safe when `m <= n`; when `m > n`, the compiler inserts a runtime
  capacity check and an overlong value traps `STC_TRAP_RANGE`. There is no silent truncation.
- Function arguments follow the same rule. A value parameter of `USTRING[n]` copies into an
  `n`-character buffer and traps if the actual length exceeds it. Use `VAR_IN_OUT` with a
  matching capacity when passing large strings without copying.
- Struct/array assignment of `USTRING` fields follows the same rule per field.
- `VAR_IN_OUT USTRING` passes a pointer; the callee must respect the actual capacity and traps on
  overflow.

## 9. Memory layout and ABI

```llvm
%stc.ustring.N = type { i32 len, [N x i32] }
%stc.uchar = type i32
```

- `len` counts characters (Unicode scalars), not bytes.
- Each element is a 32-bit Unicode scalar value; the data area is exactly `4 * N` bytes. Elements
  at indices `[len, N)` are unspecified.
- In-memory size is `4 + 4 * N` bytes. `LEN_CODE_UNIT` is computed by UTF-8 encoding and is not
  the storage size.
- `USTRING` is passed by value as the struct for value parameters; `VAR_IN_OUT`/`REF_TO` pass a
  pointer.
- Runtime helpers take `(ptr data, i32 len, i32 capacity)`; capacity is in characters.
- `UCHAR` is a 32-bit integer in memory and SSA; comparisons are unsigned scalar comparisons.

## 10. Interoperability and limits

- `USTRING` is not layout-compatible with `STRING` or `WSTRING`; no pointer casts are allowed in ST.
- `USTRING[n]` capacity is a compile-time constant. Dynamic-capacity strings are not in v1.
- The maximum capacity (65535 characters, 262140 bytes of scalar data) is a v1 implementation
  limit; exceeding it is a compile-time error.
- The in-memory representation is UTF-32-like (one 32-bit scalar per character). UTF-8 is the
  source/interchange encoding, not the storage encoding. This is intentional: it gives static
  storage and O(1) character indexing for every Unicode scalar.
- Unicode normalization, grapheme clusters, locale-aware case mapping, and bidirectional text are
  out of scope for v1.
- BOM (U+FEFF) inside a string is an ordinary scalar, not stripped.
- Noncharacters (e.g., U+FFFE) are permitted as scalar values; they are not used for encoding
  errors.

## 11. Rule IDs

| ID | Rule |
| --- | --- |
| `STC-UTF8-001` | `USTRING[n]` capacity is characters; each scalar uses 4 bytes; default 255; max 65535 |
| `STC-UTF8-002` | `UCHAR` is one Unicode scalar value (32-bit) |
| `STC-UTF8-003` | Literal forms and escapes |
| `STC-UTF8-004` | Valid UTF-8 only; invalid sequences rejected |
| `STC-UTF8-005` | `LEN` counts scalars; `LEN_CODE_UNIT` counts UTF-8 bytes; `LEN_MAX` returns capacity in characters |
| `STC-UTF8-006` | 1-based O(1) scalar indexing and slicing |
| `STC-UTF8-007` | String functions operate on scalars |
| `STC-UTF8-008` | Explicit conversions only; failure traps |
| `STC-UTF8-009` | `{ i32 len, [N x i32] }` fixed-capacity layout; static storage |
| `STC-UTF8-010` | No Unicode normalization |
| `STC-UTF8-011` | Capacity is part of the type; cross-capacity assignment/argument passing checks and traps on overflow |

## 12. Resolved decision (Q-001 / A-005)

**Answer:** `USTRING[n]` has a static capacity of `n` characters; each character occupies 4 bytes
(32-bit Unicode scalar). `LEN` counts characters; `LEN_CODE_UNIT` counts UTF-8 code units for
interchange; `LEN_MAX` returns `n`. This guarantees O(1) indexing and room for any Unicode scalar,
including emoji, at the cost of 4 bytes per ASCII character. Storage never reallocates.
