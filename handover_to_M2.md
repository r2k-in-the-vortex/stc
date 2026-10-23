# Handover to M2 — Parser, CST, and Typed AST

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-23  
Prepared after M1 (lexer) completion

This document is the handover from M1 to M2. Read it together with the project plan,
`Specification/Syntax/Grammar.md`, `Specification/AST/AST.md`, the lexer API notes in
`Specification/Syntax/Tokenization.md` §11–§13, and `Specification/Architecture/Architecture.md`
§2–§3.

## 1. State at handover

M1 is implemented and verified in the working tree. **The changes are not committed yet.** Commit
M1 (including `Cargo.lock`, `fuzz/Cargo.lock`, `.gitattributes`, the lexer snapshots, and the fuzz
target) before starting M2 so the M2 diff is clean.

| Item | State |
| --- | --- |
| Workspace | `crates/stc-span`, `crates/stc-diag`, `crates/stc-lexer`; `fuzz/` is a separate workspace |
| Toolchain | Rust 1.99.1, edition 2024; Lean not touched |
| Tests | 96 passing (`cargo test --workspace --locked`) |
| Checks | clippy `-D warnings`, `cargo fmt --check`, `check_docs.py`, Markdown lint all pass |
| Fuzzing | `fuzz_lexer` target; ~95k local executions across ASan campaigns, no failures |
| Uncommitted | yes |

M1 produced no parser, CST, sema, codegen, or CLI. `stc-syntax` and `stc-parser` directories do not
exist yet.

## 2. M2 scope and exit criteria

From the project plan:

> Deliverables: full ST grammar (declarations, statements, expressions, OOP, namespaces,
> configurations); error recovery; CST and typed AST; `stc parse`/`stc check` skeleton; parser
> snapshots and negative tests.
>
> Exit criteria:
>
> - All grammar productions have positive and negative tests.
> - Error recovery produces multiple useful diagnostics and never panics.
> - CST can round-trip source text exactly (including trivia).

This corresponds to Grammar rules `STC-GRAM-001` through `STC-GRAM-016` plus the records in
Grammar §13 (`STC-E-GRAM-101`–`104`). Sema, name resolution, and type checking remain M3.

## 3. Lexer contracts M2 can rely on

### 3.1 Entry points

```rust
// stc-lexer
pub fn tokenize(file_id: FileId, source: &str, diagnostics: &mut DiagnosticSink) -> TokenStream;
pub fn tokenize_bytes(file_id: FileId, bytes: &[u8], diagnostics: &mut DiagnosticSink) -> LexedSource;

pub struct Lexer<'src>;
impl<'src> Lexer<'src> {
    pub fn new(file_id: FileId, source: &'src str) -> Self;
    pub fn tokenize(self, diagnostics: &mut DiagnosticSink) -> TokenStream;
}

pub struct LexedSource {
    pub text: String,       // invalid UTF-8 replaced by U+FFFD
    pub stream: TokenStream,
}
```

Use `tokenize_bytes` in the driver so invalid UTF-8 is reported with `STC-E-LEX-001` and the
decoded text is owned. Keep `LexedSource.text` alive for the CST/parser/`SourceMap`; all spans
refer to it.

### 3.2 Token stream

```rust
pub struct TokenStream {
    pub tokens: Vec<Token>,   // last token is always Eof
    pub trivia: Vec<Trivia>,
    pub eof_span: Span,
}

pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub leading_trivia: TriviaRange,  // indices into TokenStream::trivia
    pub trailing_trivia: TriviaRange,
    pub literal: Option<Box<Literal>>,
}

pub struct Trivia { pub kind: TriviaKind, pub span: Span }
pub struct TriviaRange { pub start: usize, pub end: usize }
```

Useful methods:

- `stream.eof() -> Option<&Token>`
- `token.text(source) -> &str`
- `token.normalized() -> Option<&str>` (identifiers/keywords; uppercase key)
- `stream.leading_trivia(token) -> &[Trivia]`, `stream.trailing_trivia(token) -> &[Trivia]`
- `stream.reconstruct(source) -> String`
- `stream.check_invariants(source) -> Result<(), String>`

`check_invariants` is the executable form of the coverage contract. Call it in parser tests and in
`fuzz_parser` before parsing. `reconstruct` must equal the source, including BOM, comments, and
whitespace.

### 3.3 Token kinds and payloads

`TokenKind`:

```text
Eof
Ident(Box<str>)                 // normalized uppercase key
Keyword(Keyword)                // Keyword::ALL is the 147-entry list
IntLiteral, RealLiteral, BasedLiteral, TypedLiteral
StringLiteral { encoding, terminated }
UStringLiteral { terminated }
CharLiteral { kind, terminated }
DurationLiteral, DateLiteral, TodLiteral, DateTimeLiteral
DirectVariable
Pragma { terminated }
Punct(Punct)
Error
```

Decoded literal payloads are in `Token.literal`:

| Token | `Literal` payload |
| --- | --- |
| int / real / based | `Numeric { kind: Integer\|Real, radix, text }`; `text` is normalized (underscores removed, hex uppercased) and arbitrary precision |
| typed | `Typed { ty: ElementaryType, sign: Option<Sign>, value: Numeric\|Bool }` |
| string | `String { encoding: Sbcs\|Dbcs\|Utf8, delimiter, units }` — bytes, UTF-16 units, or Unicode scalars respectively |
| char | `Char { kind, value: Option<u32> }` |
| duration/date/TOD/DT | decoded components/fields; date/time ranges are **not** checked |
| direct variable | `Direct { area, size, wildcard, indices }` |
| pragma | none; raw text is `token.text(source)` |

`TokenKind` and `Punct`/`Keyword`/`Literal` are public from `stc-lexer`. `stc_lexer::codes` exports
the stable lexer codes and the parser-owned pragma codes (`STC-E-LEX-010`–`012`,
`STC-E-PRAGMA-001`–`002`, `STC-W-PRAGMA-001`–`002`).

### 3.4 Trivia order and attachment

Global order is:

```text
leading(token0), token0, trailing(token0), leading(token1), token1, trailing(token1), ...
```

A token's leading trivia is everything since the previous token; its trailing trivia is same-line
horizontal whitespace and single-line comments after it. A line break or a multi-line block comment
starts the next token's leading trivia. The initial BOM is `TriviaKind::Bom` trivia.

The CST builder must emit trivia and tokens in this same order if it wants
`cst.text() == source`. A simple rule that works: on each `bump`, emit
`leading_trivia`, the token leaf, then `trailing_trivia`, and finally emit the leading trivia of
`Eof` when finishing the file.

### 3.5 Diagnostics and source maps

`stc-diag` provides `Diagnostic`, `Severity`, `Label`, `Suggestion`, `DiagnosticSink`,
`render_human`, and `render_json`. Notes:

- `DiagnosticSink::new()` retains at most 100 diagnostics; use `unlimited()` in unit tests and
  `with_limit` where needed. Suppressed errors are tracked by `suppressed_errors()` and still make
  `has_errors()` true.
- Codes are `&'static str`; keep parser codes in a `stc_parser::codes` module.
- `render_json` returns one line without a trailing newline.
- `SourceMap` owns files and maps spans to 1-based line/column; use the same `FileId` as the lexer
  and the same decoded text.

## 4. M1 behaviors and gotchas that directly affect M2

1. **Eof is the last token.** A stream may contain a zero-width `Error` token immediately before
   `Eof` when a block comment was unterminated (`STC-LEX-049`). Do not assume token starts are
   strictly increasing; they are non-decreasing.
2. **Invalid identifiers are `Ident` tokens with a diagnostic**, not `Error` tokens. The parser
   should still build the intended CST; the driver fails because the diagnostic is an error.
3. **Malformed literals are `Error` tokens** with a diagnostic and no literal payload. Build an
   `Error`/`ErrorNode` and recover; never unwrap a literal payload without checking.
4. **Typed literal signs are inside the token.** `INT#-123` is one `TypedLiteral` with
   `sign = Some(Minus)`. `-123` is `Minus`, `IntLiteral`; the parser handles that as unary. Do not
   apply unary minus twice.
5. **Pragmas are raw tokens, not trivia.** `token.text(source)` includes `{` and `}`. Extract the
   body only for `terminated == true`; an unterminated pragma is `Pragma { terminated: false }`
   plus `STC-E-LEX-007`. JSON parsing is parser work (see §7).
6. **`a.%X0` is lexed as `Ident`, `Dot`, `DirectVariable`.** `%X0` is a direct-variable token, not
   `Percent` + identifiers. `a.0` is `Ident`, `Dot`, `IntLiteral`. The parser must accept these
   forms for partial bit access. The grammar's `. '%' ...` spelling is a grammar-notation
   abstraction; the lexer normalizes the address.
7. **Non-keyword constructs are identifiers.** `PROPERTY`, `MUTEX`, `SEMA`, `ASSERT`, and typical
   IL mnemonics (`LD`, `ST`, `CAL`, …) are valid identifiers. Grammar §13 requires explicit
   `STC-E-GRAM-104`/`STC-E-GRAM-101` diagnostics; the parser needs a deliberate lookahead policy
   for them (see §10).
8. **Preprocessor input is `#` + tokens.** `#if` is `Hash`, `Keyword(If)`; `#define` is `Hash`,
   `Ident(DEFINE)`. Reject at item/statement start with `STC-E-GRAM-103`.
9. **Unsupported SFC keywords are reserved but not parsed.** `ACTION`, `STEP`, `TRANSITION`, and
   the other SFC keywords are `Keyword` tokens. Diagnose them with `STC-E-GRAM-102`.
10. **BOM is trivia.** `CST.text()` must include it; a BOM elsewhere outside comments/strings/
    pragmas is already a lexer error.
11. **NUL is rejected everywhere**, including comments and pragmas; diagnostics may point into
    trivia.
12. **Trivia attachment is same-line only.** A multi-line block comment is leading trivia of the
    next token; do not assume a comment is trailing.
13. **Literal ranges are not checked.** `D#2024-99-99`, `TOD#99:99:99`, and huge typed integers
    reach the parser unchanged. Sema checks ranges (M3).
14. **String/char units are encoding-dependent.** `StringLiteral.units` contains bytes (SBCS),
    UTF-16 units (DBCS), or scalars (UTF-8). Do not reinterpret them.
15. **Direct-variable indices are `u32`.** A wildcard token carries `wildcard = true` and no
    indices; the analyzer rejects it in executable expressions.
16. **The lexer's character literal arity is already checked.** `Char { value: None }` means the
    source literal was malformed; a diagnostic exists.
17. **`TokenStream` is single-file.** Project manifests/source globs are M6; M2's driver can take
    a path on the command line and lex it directly.

## 5. Recommended M2 architecture

### 5.1 Crates

Add to the root workspace:

```text
crates/stc-syntax/     SyntaxKind, CST tree/builder, typed AST wrappers, pragma JSON
crates/stc-parser/     recursive descent + Pratt parser, recovery, parse entry points
crates/stc-driver/     minimal `stc` binary with `parse` and `check` skeletons
fuzz/fuzz_targets/fuzz_parser.rs
```

Dependency direction must stay acyclic:

```text
stc-span -> stc-diag
stc-span -> stc-lexer -> stc-syntax -> stc-parser
stc-parser -> stc-driver
```

`stc-syntax` may depend on `stc-lexer` for `TokenKind`/`Keyword`/`Punct`/`Literal`; `stc-parser`
must not depend on `stc-codegen`, and `stc-codegen` must never depend on the parser.

### 5.2 CST backing store

`AST.md` allows a rowan-style tree or an equivalent arena. Recommendation: use a small immutable
arena tree and keep the workspace dependency-free. If you choose `rowan`, record it in
`Specification/Decisions.md` and adapt the `SyntaxKind` mapping; both are allowed by the spec.

A simple arena shape:

```rust
struct SyntaxTree {
    source: Arc<str>,            // or share with the driver; decide and document
    tokens: Vec<Token>,          // or references/indices into the lexer stream
    trivia: Vec<Trivia>,
    nodes: Vec<SyntaxNodeData>,
    root: NodeId,
}

struct SyntaxNodeData {
    kind: SyntaxKind,
    span: Span,
    parent: Option<NodeId>,
    children: Vec<Element>,      // node, token, or trivia
}
```

Requirements:

- Nodes are immutable after construction, have ordered children, a span, and optionally a parent.
- Tokens and trivia are leaves; every byte is represented.
- `SyntaxNode::text()` concatenates leaves and equals the source slice for the node span.
- A node's span is the union of its children; missing tokens are zero-width.
- Error nodes may contain tokens/nested nodes and are reachable to the parser's recovery code.

Decide whether node spans include trivia. If they do (rowan-style), typed AST spans may include
comments; consider keeping a separate token-extent span for diagnostics if needed.

### 5.3 SyntaxKind

`AST.md` requires `SyntaxKind` to be `#[repr(u16)]`, `#[non_exhaustive]`, explicit discriminants,
append-only, and enforced by a checked-in name/discriminant table test.

Recommended layout:

- Distinct token kinds for every keyword, punctuation, and token category (IDENT, literals,
  pragma, error, Eof) so the CST is useful to the formatter and future IDE.
- Distinct node kinds for the items/statements/expressions/types listed in `AST.md` §3.2.
- Reserve numeric ranges for organization (for example tokens in one range, nodes in another), but
  never renumber or reuse existing values.
- `SyntaxKind::from_token(&TokenKind) -> SyntaxKind` plus mapping tests for all
  `Keyword::ALL`, all `Punct` variants, and every `TokenKind` variant.
- Canonical `as_str`/`Debug` spellings are stable; the table test covers both name and value.

### 5.4 Parser shape

- Recursive descent for declarations and statements; Pratt/precedence climbing for expressions.
- A `Parser` over `(source, stream, diagnostics, mode)` with:
  - token index, lookahead helpers, `at(kind)`, `bump()`, `expect(kind)`;
  - a builder that emits CST nodes and leaves;
  - current context: strict/relaxed semicolons, top-level vs configuration, namespace, POU kind,
    method vs prototype, expression depth;
  - a recursion depth limit (propose 256) producing a diagnostic instead of a stack overflow.
- Do not re-lex tokens or re-parse literal text; consume `Token.literal`.
- Parse pragmas as tokens at the grammar positions where they are allowed; the lexer accepts them
  anywhere whitespace is allowed.
- Build typed AST wrappers as zero-cost handles over CST nodes, not a second tree (`AST.md` §4).

### 5.5 Recovery

Use the sync sets from `Architecture.md` §3.2:

- declarations: `;`, `END_VAR`, `END_FUNCTION`, `END_FUNCTION_BLOCK`, `END_PROGRAM`,
  `END_CLASS`, `END_INTERFACE`, `END_TYPE`, `END_NAMESPACE`;
- statements: `;`, `END_IF`, `END_CASE`, `END_FOR`, `END_WHILE`, `END_REPEAT`, `ELSE`, `ELSIF`;
- expressions: `)`, `]`, `,`, `;`, and block delimiters.

On a mismatch, emit one diagnostic, create an `Error`/missing node, and synchronise. Prefer a
small number of precise diagnostics over a cascade. Track mismatched opening constructs so
`END_*` errors can point at both spans (Grammar §11.9).

## 6. Grammar implementation notes

Read `Grammar.md` in full; these are the high-risk points:

- **Precedence** (Grammar §9.1): relational (`< > <= >=`) binds tighter than equality (`= <>`);
  `NOT` is unary at level 9, above `**` at level 8; `**` is left-associative; unary `-` binds
  tighter than `**`, so `-2 ** 2` is `(-2) ** 2`. `AND`/`&` are aliases; `A = B < C` is
  `A = (B < C)`. Encode this as a table-driven Pratt parser and test every level.
- **Strict semicolons** by default (`STC-GRAM-016`). `--relaxed` is a parser option that accepts
  omitted semicolons only after block statements when the next token is a block delimiter or a
  nested statement. Simple statements always require `;`. `;;` is two empty statements.
- **`stmt_list ::= (stmt? ';')*`**: empty statements are valid; block statements are followed by a
  terminator in strict mode.
- **`PROGRAM` ambiguity**: a top-level POU at file scope, a program-configuration entry inside
  `CONFIGURATION`/`RESOURCE`. Track the configuration context.
- **`TASK` ambiguity**: a declaration at the start of a resource item; a name reference elsewhere.
- **`METHOD` in interfaces** is a prototype (no body); in classes/FBs it is a definition with a
  body.
- **`THIS`/`SUPER`** are valid only in method/FB bodies and as call/path prefixes. `THIS` is not a
  standalone variable. `SUPER()` with no arguments is a special FB-body call.
- **Calls**: a path followed by `(` is a call; `REF(x)` is a reference value, not a normal call;
  `F` alone is a variable/type name. `NOT a => x` is a negated output binding.
- **`ARRAY [*]`** is valid only as a conformant parameter type; the parser accepts the shape and
  the analyzer enforces placement.
- **Struct/section contexts**: `VAR_CONFIG` is only inside `CONFIGURATION`; `VAR_ACCESS` is its own
  section; class bodies accept only `VAR` sections; global variable sections are top-level.
- **`AT`** is always the address-binding keyword; wildcard direct variables are allowed only in
  declarations/`VAR_ACCESS` (analyzer enforces executable contexts).
- **Partial bit access**: `a.0`, `a.%X0`, `a.%B0`, etc. See the lexer gotcha in §4.6.
- **Unsupported constructs**: SFC keywords (`ACTION`, `STEP`, `TRANSITION`, …) → `STC-E-GRAM-102`;
  preprocessor (`#if`, `#define`, …) → `STC-E-GRAM-103`; IL/LD/FBD markers → `STC-E-GRAM-101`;
  `PROPERTY`/`MUTEX`/`SEMA`/`ASSERT` → `STC-E-GRAM-104`. Since several of these are ordinary
  identifiers, define the detection policy and add positive/negative tests.
- **Pragmas**: leading pragmas attach to the following item; a trailing pragma before `;` attaches
  to the variable declaration; a pragma at the end of a block attaches to the terminator and is
  reported unused if meaningless.

## 7. Pragma JSON and attachment

`Architecture.md` says `stc-syntax` parses pragma JSON with `serde_json`. The lexer only scans
balanced braces and strings.

Rules to implement:

- Extract the body of a terminated `Pragma` token. The token span includes `{`/`}`; the body is
  between them. Do not parse an unterminated pragma.
- A non-`stc:` pragma is ignored with `STC-W-PRAGMA-001`; `{attribute '...'}` follows the same
  path for now.
- A `stc:` pragma has the shape `{stc:<name> <json-value>?}`. `name` is
  `[A-Za-z_][A-Za-z0-9_]*`.
- Known names and payload expectations are in Tokenization §8: `test` (optional object),
  `export`/`no_mangle`/`inline` (no payload), `deprecated` (optional object), `io` (required
  object). Unknown names → `STC-E-PRAGMA-001`; missing/unexpected payload → `STC-E-PRAGMA-002`;
  unknown keys in known objects → `STC-W-PRAGMA-002` and preserved.
- Strict JSON: invalid JSON → `STC-E-LEX-010`; duplicate object keys → `STC-E-LEX-011`; payload
  larger than 64 KiB or nested deeper than 32 → `STC-E-LEX-012`.
- The legacy `{stc:test(cycles := 25)}` form is not JSON and must be rejected with
  `STC-E-LEX-010`, not accepted (STC-LEX-058).
- `serde_json::Value`'s default map silently overwrites duplicate keys. Use a custom
  `Deserialize`/visitor or a pre-parse key check to produce `STC-E-LEX-011`.
- Preserve the raw JSON/`serde_json::Value` in `PragmaAnnotation` (`AST.md` §4.0) so metadata and
  unknown keys survive.
- Policy decisions still needed: the diagnostic code for duplicate `{stc:io}` pragmas (Grammar
  §5 says it is an error; no code is assigned), and whether any other duplicate `stc:` pragma is
  an error. Record the choice in `Decisions.md` and add a test.

## 8. `stc parse` and `stc check` skeleton

Add `crates/stc-driver` with a `stc` binary and the minimum needed for M2:

- `stc parse <file>`: lex + parse, print a stable CST dump and/or typed AST dump to stdout.
  Human diagnostics go to stderr; `--message-format=json` emits one JSON object per diagnostic on
  stdout. Exit 0 if no errors, 1 if errors, 2 for usage, 3 for a caught panic (catch at the driver
  boundary).
- `stc check <file>`: lex + parse only for now; full name resolution/type checking arrives in M3.
- `--relaxed` switches the semicolon mode.
- Keep the CLI argument handling small; either parse `std::env::args` manually or introduce
  `clap` and record the dependency decision. `stc.toml`, manifests, and source globs are M6.
- The driver should use `tokenize_bytes`, retain `LexedSource.text`, build a `SourceMap` with the
  same text, and render diagnostics through `stc-diag`.
- Do not clear the diagnostic sink between lexing and parsing; `stc check` should report both
  lexical and syntax errors when parsing can continue.
- If the driver catches a panic (exit 3), report `STC-I-0001` as the CLI spec requires.

## 9. Tests and fuzzing

### 9.1 Parser tests

- Positive and negative tests for every grammar production, not just every rule ID. The exit
  criterion is per production.
- A traceability test (similar to the lexer's `rule_ids_have_named_tests`) mapping:
  - `STC-GRAM-001`–`016` to named tests;
  - every production in Grammar §2–§10 to at least one positive and one negative test;
  - `STC-E-GRAM-101`–`104` to negative tests.
- CST round-trip: `cst.text() == source` for every corpus file and every negative test.
- Snapshot tests: compact CST dump (kinds, spans, text), typed AST dump, and diagnostic output.
  The lexer uses a hand-rolled snapshot harness with `STC_UPDATE_SNAPSHOTS=1`; reuse that pattern
  or introduce `insta` and record the dependency decision.
- Error-recovery tests: multiple independent errors in one file; missing `;`, missing `END_*`,
  mismatched `END_*`, bad expression delimiters, malformed declarations; assert several
  diagnostics and recoverable CST.
- Diagnostic snapshots assert code, severity, primary span, labels, and notes, not only counts.
- Depth tests: deeply nested parentheses/expressions produce a diagnostic rather than a stack
  overflow.
- Property tests: random token insertion/deletion; no panic; CST round-trip; coverage invariants.

### 9.2 UI and harness timing

`tests/ui/*.st` plus `.stderr` is the long-term diagnostic harness (`Testing-and-Verification.md`
§3.2), but CI's `ui` job only runs when **both** `tests/ui` and `crates/xtask` or `tools/xtask`
exist. If you add `tests/ui` in M2, also add the xtask entry point; otherwise keep parser
snapshots/negative tests in `crates/stc-parser/tests` and `crates/stc-driver/tests` until M6.

### 9.3 Fuzzing

Add `fuzz/fuzz_targets/fuzz_parser.rs`:

```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut diagnostics = stc_diag::DiagnosticSink::new();
    let lexed = stc_lexer::tokenize_bytes(stc_span::FileId::new(0), data, &mut diagnostics);
    lexed.stream.check_invariants(&lexed.text).unwrap();
    let parsed = stc_parser::parse(/* file id, text, stream, options */);
    // assert CST round-trip, no panic, bounded depth
});
```

CI's `fuzz-smoke` detection picks up any new `fuzz_targets/*.rs` automatically. Keep
`fuzz/Cargo.toml` and `fuzz/Cargo.lock` updated. The Windows ASan runtime issue is documented in
`fuzz/README.md`.

## 10. Decisions/open items to resolve during M2

Record each in `Specification/Decisions.md` and, where it changes the surface, in the profile or
grammar spec:

1. **CST backing store**: hand-rolled arena (recommended) or `rowan`.
2. **CST source ownership**: tree owns `Arc<str>`/`String`, or the driver keeps a shared source and
   nodes expose text through a trait. The AST spec's `SyntaxNode::text()` favors tree ownership.
3. **SyntaxKind layout**: numeric ranges, token-vs-node mapping, and whether each keyword/punct is
   a distinct kind (recommended for tooling).
4. **Parser diagnostic codes**: Grammar has `STC-E-GRAM-101`–`104` for unsupported constructs but
   no codes for ordinary syntax errors. Define a stable table (expected token, unexpected token,
   missing terminator, mismatched `END_*`, expression nesting too deep, invalid pragma placement)
   before implementing the diagnostics; extend `Grammar.md` and add tests.
5. **Unsupported-construct detection**: decide how to recognize IL/LD/FBD markers and Ed. 4
   identifiers (`PROPERTY`, `MUTEX`, `SEMA`, `ASSERT`) without rejecting legitimate identifiers.
   Propose a small lookahead/keyword set and test both directions.
6. **Pragma codes**: duplicate `{stc:io}` (and possibly duplicate `stc:` pragmas) needs a code
   (for example a new `STC-E-PRAGMA-003`) and a spec entry.
7. **JSON dependency and duplicate-key handling**: `serde_json` is named by the architecture;
   decide the custom visitor/pre-scan for `STC-E-LEX-011` and the 64 KiB/32-depth checks.
8. **CLI**: manual argument parsing vs `clap`; output format of `stc parse`; whether `stc check`
   accepts directories or only files in M2.
9. **UI harness**: add xtask now if `tests/ui` is introduced, or defer UI to M6.
10. **Trivia and spans in typed AST**: whether typed AST node spans include leading/trailing trivia
    and how diagnostics choose a token-only span.
11. **Missing-token policy**: zero-width token insertion versus zero-width `Error` node, and how
    `END_*` mismatch diagnostics report both spans.

## 11. Definition of done for M2

- [ ] `crates/stc-syntax` exists with a stable, append-only `SyntaxKind` and a checked-in
      name/discriminant table test.
- [ ] The CST is lossless for valid, invalid, and partially recovered inputs; `cst.text() ==
      source` including BOM, comments, whitespace, and pragmas.
- [ ] Every grammar production has at least one positive and one negative test; `STC-GRAM-001`–
      `016` and `STC-E-GRAM-101`–`104` are traceable to tests.
- [ ] Recursive descent and Pratt expression parsing implement the complete Grammar §2–§10
      surface, including OOP, namespaces, configurations, strict/relaxed semicolons, and the
      Annex A precedence split.
- [ ] Pragmas are attached to the intended items; strict JSON payloads, duplicate keys, size/depth
      limits, and known-name/payload rules produce the specified diagnostics.
- [ ] Error recovery emits multiple useful diagnostics, never panics, and respects a recursion
      limit.
- [ ] `stc parse` and `stc check` skeletons build and respect the CLI exit-code and
      human/JSON-diagnostic contract; `stc check` is parse-only until M3.
- [ ] Parser snapshots and negative tests are committed; `fuzz_parser` is clean in CI smoke.
- [ ] `cargo test --workspace --locked`, clippy `-D warnings`, `cargo fmt --check`,
      `check_docs.py`, and Markdown lint pass.
- [ ] `README.md`, `Decisions.md`, and the conformance/profile status reflect M2.

## 12. Commands

```sh
cargo test --workspace --locked
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all --check
python tools/ci/check_docs.py
npx --yes markdownlint-cli2@0.24.0

# Regenerate lexer snapshots (only when the lexer output changes)
STC_UPDATE_SNAPSHOTS=1 cargo test -p stc-lexer --test snapshots

# Lexer fuzz target
cargo +nightly fuzz run fuzz_lexer -- -max_total_time=60 -rss_limit_mb=2048 -timeout=25

# Fuzz workspace check
cargo check --manifest-path fuzz/Cargo.toml --locked

# Fuzz targets are outside the workspace, so format/check them explicitly
rustfmt --edition 2024 --check fuzz/fuzz_targets/fuzz_lexer.rs
```

The block above is POSIX `sh`. In PowerShell, set environment variables first, for example
`$env:RUSTFLAGS = "-D warnings"; cargo clippy --workspace --all-targets --locked -- -D warnings`
and `$env:STC_UPDATE_SNAPSHOTS = "1"; cargo test -p stc-lexer --test snapshots`. Remove the
variables afterwards if desired (`Remove-Item Env:RUSTFLAGS`).

On Windows, the fuzzer needs the MSVC ASan runtime; see `fuzz/README.md`.

## 13. Suggested order

1. Resolve the M2 spec decisions in §10, especially parser error codes and pragma codes.
2. Add `stc-syntax`: `SyntaxKind` + table test, CST arena/builder, typed AST wrappers, pragma JSON.
3. Add CST construction from a `TokenStream` and prove round-trip on the lexer corpus before
   writing grammar rules.
4. Add `stc-parser` declarations and types; then POU/method/namespace/OOP; then statements;
   then the Pratt expression parser; then configurations.
5. Add recovery and the unsupported-construct diagnostics; add the depth limit.
6. Add `stc-driver` `parse`/`check` skeleton and CLI tests.
7. Add parser snapshots, negative tests, traceability, property tests, and `fuzz_parser`.
8. Update docs/status; run the full CI-equivalent checks; commit M2.

The most expensive correctness risks are the Annex A precedence split, semicolon strictness,
configuration/POU `PROGRAM` ambiguity, pragma JSON strictness, and recovery cascades. Put those
under tests early.
