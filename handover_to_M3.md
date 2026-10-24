# Handover to M3 — Name Resolution and Type Checking

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-24  
Prepared after M2 (parser, CST, typed AST, pragmas, driver skeleton) completion

Read this together with the project plan, `Specification/Syntax/Grammar.md`,
`Specification/AST/AST.md` §5–§8, `Specification/Architecture/Architecture.md` §2–§5,
`Specification/Architecture/CLI-and-Manifest.md` §2, and `Specification/Decisions.md`
(D-045–D-058).

## 1. State at handover

M2 is implemented and verified in the working tree. **The M2 changes are not committed yet.**
Commit M2 (including `Cargo.lock`, `fuzz/Cargo.lock`, the new crates, corpus, snapshots, and
`fuzz_parser`) before starting M3 so the M3 diff is clean. M1 is committed as `51cab11`.

| Item | State |
| --- | --- |
| Workspace | `crates/stc-span`, `stc-diag`, `stc-lexer`, `stc-syntax`, `stc-parser`, `stc-driver`; `fuzz/` is separate |
| Toolchain | Rust 1.99.1, edition 2024 |
| Tests | 220 passing (`cargo test --workspace --locked`) |
| Checks | clippy `-D warnings`, `cargo fmt --check`, `check_docs.py`, Markdown lint all pass |
| Fuzzing | `fuzz_lexer` (M1) and `fuzz_parser`; M2 parser campaigns: 21,347 and 21,442 runs, no findings |
| Uncommitted | yes (M2) |

M2 produced no sema, HIR, codegen, or project manifest. `stc-syntax` and `stc-parser` exist;
`stc-hir`, `stc-sema`, `stc-codegen`, `stc-manifest`, and the remaining CLI verbs do not.
`stc check` is parse-only.

## 2. M3 scope and exit criteria

From the project plan:

> Deliverables: symbol tables, scopes, namespaces/USING, class/interface/FB inheritance, method
> resolution, overloads, implicit/explicit conversions, constant expressions, initializers, access
> control, reference/interface rules, assignment attempt, diagnostic codes; `stc check`.
>
> Exit criteria:
>
> - Positive and negative conformance cases for every rule category.
> - No codegen yet; diagnostics are stable and snapshot-tested.
> - Type errors include actionable spans and notes.

M3 is where the compiler stops being syntax-only. It must turn the per-file typed AST into a
project-wide, resolved, type-checked representation. The AST spec places HIR lowering after type
checking (AST §5 and §8); producing HIR in M3 is recommended so M4 has its declared input, but at
minimum M3 must stabilize the resolver, type representation, and diagnostic contracts. Record the
choice (HIR now vs. M4) in `Decisions.md`.

## 3. Contracts M2 provides

### 3.1 Crates and dependency direction

```text
stc-span -> stc-diag
stc-span -> stc-lexer -> stc-syntax -> stc-parser
stc-syntax -> stc-span/stc-diag/stc-lexer
stc-parser -> stc-driver
stc-syntax -> stc-driver
stc-lexer -> stc-driver
```

`stc-syntax` has no dependency on `stc-parser`, and no crate depends on `stc-codegen`. A future
`stc-hir` must not depend on `stc-syntax` or `stc-parser`; `stc-codegen` must consume only HIR
(`Architecture.md` §2).

### 3.2 `stc-syntax`

The public surface M3 should use:

```rust
// Parsed file and source
pub struct ParsedFile { /* private */ }
impl ParsedFile {
    pub fn tree(&self) -> &SyntaxTree;
    pub fn source(&self) -> &str;
    pub fn root(&self) -> SyntaxNode;
    pub fn into_tree(self) -> SyntaxTree;
}

pub struct SyntaxTree { /* Arc<TreeData> */ }
impl SyntaxTree {
    pub fn file(&self) -> FileId;
    pub fn source(&self) -> &str;
    pub fn root(&self) -> SyntaxNode;
    pub fn node(&self, id: NodeId) -> SyntaxNode;
    pub fn token(&self, id: TokenId) -> SyntaxToken;
    pub fn trivia(&self, id: TriviaId) -> SyntaxTrivia;
    pub fn pragma_annotation(&self, token: TokenId) -> Option<&PragmaAnnotation>;
    pub fn check_invariants(&self) -> Result<(), String>;
    pub fn text(&self, span: Span) -> &str;
}

pub struct SyntaxNode { /* tree + NodeId */ }
impl SyntaxNode {
    pub fn kind(&self) -> SyntaxKind;
    pub fn span(&self) -> Span;          // includes attached trivia
    pub fn token_span(&self) -> Span;    // first..last non-trivia token
    pub fn first_token(&self) -> Option<SyntaxToken>;
    pub fn last_token(&self) -> Option<SyntaxToken>;
    pub fn parent(&self) -> Option<SyntaxNode>;
    pub fn children(&self) -> Vec<SyntaxElement>;
    pub fn child_nodes(&self) -> Vec<SyntaxNode>;
    pub fn child_tokens(&self) -> Vec<SyntaxToken>;
    pub fn first_child_of_kind(&self, kind: SyntaxKind) -> Option<SyntaxNode>;
    pub fn first_token_of_kind(&self, kind: SyntaxKind) -> Option<SyntaxToken>;
    pub fn has_direct_token(&self, kind: SyntaxKind) -> bool;
    pub fn text(&self) -> &str;
    pub fn tree(&self) -> &SyntaxTree;
}
```

Typed AST views (`stc_syntax::ast`) are zero-cost handles over CST nodes:

- `SourceFile::items() -> Vec<Item>`, `SourceFile::item_named(&str)`.
- `Item` includes `Using`, `Namespace`, `DataType`, `Function`, `FunctionBlock`, `Program`,
  `Class`, `Interface`, `Configuration`, `GlobalVars`, `Error`.
- `NamespaceDecl::{name, is_internal, using_directives, elements}`. `elements()` excludes
  `USING` directives; `using_directives()` returns them separately.
- `DataTypeDecl::types() -> Vec<TypeDecl>`; `TypeDecl` distinguishes Simple/Subrange/Enum/
  NamedValue/Array/Struct/String/Ref/Error.
- `VarSection::{kind, qualifiers, declarations}`; `VarDeclLike::{Var, Edge}`;
  `VarDecl::{names, type_ref, has_at, initializer, pragmas}`.
- POU views: `FunctionDecl`, `FunctionBlockDecl`, `ProgramDecl`, `ClassDecl`, `InterfaceDecl`,
  `MethodDecl`, `MethodPrototype`. Common accessors: `name`, `var_sections`, `using_directives`,
  `pragmas`; `body` where a body is allowed; `extends`/`implements` where applicable;
  `MethodDecl::pou_kind() -> Option<PouKind>`.
- Statements: `StmtList::statements() -> Vec<Stmt>`; `Stmt` enum and node views for assignments,
  reference assignment, attempt, call, return, if/case/for/while/repeat, exit/continue, empty,
  and `Error`.
- Expressions: `Expr` enum (`Binary`, `Unary`, `Call`, `Member`, `Index`, `Deref`, `Paren`,
  `Literal`, `Path`, `RefValue`, `Error`); `BinaryExpr::{lhs, rhs, operator}`,
  `UnaryExpr::{operand, operator}`, `CallExpr::{callee, arguments, param_list}`,
  `MemberExpr::{base, member, bit_index}`, `IndexExpr::{base, indices}`,
  `PathExpr::{segments, text}`, `RefValueExpr::{is_null, target}`, `LiteralExpr::{token, text}`.
- `Arg` enum: `Positional`, `Named`, `Output`, `NegatedOutput`, `RefAssign` (currently not
  produced; named `x := y` arguments become `Named`), `Error`.
- `Name::{text, normalized, token}`; `Path::{segments, text}`; `TypeRef::{text, path, bounds}`.
- `Pragma::annotation() -> Option<&PragmaAnnotation>`; `PragmaAnnotation { name, payload, span }`.
- `ErrorNode` for parser-recovered regions.

### 3.3 `stc-parser`

```rust
pub fn parse<S: Into<Arc<str>>>(
    file: FileId,
    source: S,
    stream: &TokenStream,
    options: ParseOptions,
    diagnostics: &mut DiagnosticSink,
) -> ParsedFile;

pub struct ParseOptions {
    pub relaxed_semicolons: bool,
    pub max_depth: usize, // default 256
}
```

Use `tokenize_bytes` in the driver, retain `LexedSource.text`, and parse with the same `FileId` and
text. Do not clear the diagnostic sink between lexing and parsing. Parser codes live in
`stc_parser::codes` and are documented in Grammar §13.1:

| Code | Meaning |
| --- | --- |
| `STC-E-GRAM-101` | IL/LD/FBD body |
| `STC-E-GRAM-102` | SFC construct |
| `STC-E-GRAM-103` | preprocessor directive |
| `STC-E-GRAM-104` | out-of-profile Ed. 4 construct |
| `STC-E-GRAM-105` | unexpected token |
| `STC-E-GRAM-106` | expected token/construct |
| `STC-E-GRAM-107` | missing `;` |
| `STC-E-GRAM-108` | mismatched `END_*` |
| `STC-E-GRAM-109` | depth limit |
| `STC-E-GRAM-110` | pragma placement |
| `STC-E-GRAM-111` | missing `END_*` |

### 3.4 Pragmas

The parser extracts and validates pragma payloads at parse time and stores annotations in the
tree side table keyed by the pragma token. Leading pragmas attach to the following item/
declaration; a trailing pragma before a var-declaration `;` attaches to that declaration.
`{stc:io}` duplicate detection uses `STC-E-IO-003`; `stc:io` without `AT` emits
`STC-W-IO-002`. Unknown keys in known pragmas warn and are preserved. Non-`stc` pragmas warn and
are ignored. Do not re-parse the payload in sema; consume `PragmaAnnotation`.

### 3.5 Driver

`stc parse <file>` prints a CST dump, plus a typed AST dump with `--ast`; `stc check <file>` is
parse-only. Both support `--relaxed` and `--message-format human|json`. Human diagnostics go to
stderr; JSON diagnostics go to stdout (one object per line). Exit codes: 0 success, 1 compile
error, 2 usage, 3 caught panic. In JSON mode the dump is suppressed (D-051).

## 4. File, namespace, package, and module model

This matters for M3 because name resolution becomes project-wide even though M2 is per-file.

The project does **not** define a source-level module declaration. Distinguish:

| Layer | Scope |
| --- | --- |
| Token stream / CST / typed AST | one per source file (`FileId` distinguishes files) |
| Namespace | a scope; files and nested `NAMESPACE` declarations merge into it |
| Package | a manifest plus its files; has a `PackageId` and a root namespace |
| HIR crate | one per build; all packages merged, globally unique `DefId`s |
| LLVM module | one per artifact (`.ll`); `--emit` derives artifacts from that one module |

Consequences:

- A file with no explicit `NAMESPACE` contributes to the **package root namespace** (the manifest
  name, or dependency alias), not to an anonymous/global module. Until `stc-manifest` exists, M3
  needs a defined session root for multi-file check (see §5.2).
- Two files with `NAMESPACE ProjectDep` contribute to the same `ProjectDep` scope. File order is
  not semantic. Duplicate definitions in that scope are collection errors.
- Namespaces do not create separate semantic modules or LLVM modules. The example “one file with
  no namespace + two files with `NAMESPACE ProjectDep`” is **3 file CSTs, 1 merged HIR crate, 1
  LLVM IR module** (when all files are in one package). The main entry point is an item in the
  package root; the selected `CONFIGURATION`/`PROGRAM` determines `main`/`stc_cycle`.
- Packages are a visibility/namespace-root layer, not a module boundary. Source dependencies
  compile together with the root package in v1; all packages merge into one `HirCrate`
  (`Architecture.md` §5).
- `USING` imports namespaces, not packages. A dependency package is reachable through its root
  namespace (manifest name/alias).
- `INTERNAL` is package-private; `PRIVATE` is POU/class-private; the default top-level
  visibility is public across packages. Access checks run after resolution.

## 5. Recommended M3 architecture

### 5.1 Crates

Add to the workspace:

```text
crates/stc-hir/     arenas, DefId/LocalId/ParamId/TyId, Ty/TyArena, HIR nodes, Interner
crates/stc-sema/    collection, scopes, resolution, type checking, const eval, lowering
```

`stc-hir` must not depend on `stc-syntax`/`stc-parser`; `stc-codegen` must consume only HIR.
`stc-sema` consumes the typed AST from `stc-syntax` and produces HIR (recommended) or a stable
resolution/type map. The driver wires parser + sema.

### 5.2 Minimal multi-file input for M3

M3 needs more than one file to test namespaces and cross-file resolution, but the full project
manifest is M6. Recommended bridge:

- Add `stc check <file>...` (or repeated `--source <file>`) for M3. Lex and parse each file with a
  distinct `FileId`; keep all texts in one `SourceMap`.
- Define a session root namespace for files with no explicit namespace (for example `_root` or
  the package name when a manifest exists). Record the choice in `Decisions.md`; M6 will replace
  it with the manifest package name/alias.
- Keep `stc parse` single-file.
- Do not pull `stc-manifest` forward; a minimal in-memory `SourceSet`/`Session` in `stc-sema` or
  the driver is enough. `stc check` without a manifest is a deliberate M3 bridge.

### 5.3 Collection

Walk `SourceFile::items()` for every file and every nested `NamespaceDecl`, maintaining a
namespace stack. Build:

- namespaces and their `USING` directives;
- user types (simple, subrange, enum, named-value, array, struct, string, ref);
- POUs (function, FB, program, class, interface), methods, method prototypes;
- global variable sections, `VAR_ACCESS`, `VAR_CONFIG`, configurations/resources/tasks/programs;
- package/root namespace entries.

Rules:

- File order is not semantic; collect deterministically (sort by file path/FileId, then source
  order) for stable diagnostics and HIR.
- Duplicate definitions in one scope are errors; record all of them, not just the first.
- Namespace elements cannot be `PROGRAM`, `CONFIGURATION`, or global variable sections. The parser
  builds these nodes with a `105` diagnostic for recovery; collection must reject/ignore them
  rather than registering them.
- Class bodies accept only `VAR` sections; the parser builds other kinds with a `105` diagnostic;
  collection must reject them.
- `INTERNAL` is recorded on the namespace; package visibility is applied after resolution.
- Build a standard prelude scope: elementary types plus standard function/FB signatures. The
  stdlib ST sources are M5; for M3 a signature table is sufficient. Warn when a user POU shadows
  a standard name (`STC-W-NAME-002`).
- Decide whether test-only units (`{stc:test}`) are collected/skipped in production and how
  `STC-E-ASSERT-001`/`STC-E-TEST-001` are applied (A-020). This is partly M6, but the declaration
  rules should be decided before collection is frozen.

### 5.4 Types and canonicalization

Implement `Ty` from AST §6 and a `TyArena` that canonicalizes:

- `Error`, `Bool`, `Int`, `Real`, `BitString`, `Time`, `Date`, `Char`, `String`, `Array`,
  `Struct`, `Enum`, `Subrange`, `Ref`, `Class`, `Interface`, `FunctionBlock`, `Function`,
  `Program`, `Generic`.
- Widths and encodings are explicit; `BOOL` is not arithmetic.
- Subranges retain base type and bounds; assignments check bounds.
- Arrays retain element type and bounds; conformant `ARRAY [*]` carries a marker and is allowed
  only in `VAR_IN_OUT`/FB parameters (A-007).
- Structs retain order, `OVERLAP`, and `AT` bindings.
- Class/FB types retain base, interfaces, methods, fields, and vtable slots; interfaces retain
  prototypes in stable name+signature slot order.
- String types retain kind and capacity; `USTRING` capacity is characters, storage 4 bytes per
  scalar.
- `Ty::Error` is the cascade suppressor. Every resolution/type failure returns it.

### 5.5 Name resolution

Follow AST §7:

- Scopes innermost to outermost: POU locals; POU name; class/FB members and inherited members;
  namespace scope and `USING`; package root; globals; standard prelude; built-in types/literals.
- Case-insensitive normalized keys; original spelling preserved for diagnostics.
- Shadowing: local > member > explicit import > wildcard `USING` > outer scope.
- Qualified names resolve segment by segment; a namespace segment may lead to a type, POU, or
  variable.
- `THIS.member` starts at the current class/FB type; `SUPER.member` at the base. `THIS` is not a
  general expression prefix.
- Overload sets are resolved after argument typing; ambiguity is an error.
- Unknown names get `STC-E-NAME-*` with a bounded edit-distance “did you mean” suggestion.
- Access control is checked after resolution: `PRIVATE`, `PROTECTED`, `INTERNAL`, `PUBLIC`.
- Package boundaries: dependency symbols are not implicitly imported; they are reachable through
  the package root namespace; `INTERNAL` is not visible outside the package.
- Record resolutions in a map keyed by `NodeId`/span (or emit them into HIR if lowering runs in
  M3) so tests and later passes do not re-resolve.

### 5.6 Type checking

At minimum:

- Assignments (including reference assignment and `?=`), conditions, `FOR` control variables,
  `CASE` labels, `RETURN`, call arguments, initializers.
- Operators by category: arithmetic, bitwise, comparison, boolean, time/date; `&`/`AND` alias;
  `MOD` sign; `**`/`EXPT`; short-circuit semantics are syntax/semantics already fixed.
- Calls: functions, FBs, methods; positional and named inputs; `=>` outputs; `NOT =>`;
  `REF(...)`; `NULL`; `SUPER()`; overload resolution; output binding targets.
- Conversions: define the implicit/explicit matrix before implementation. Numeric widening/
  narrowing, bit strings, date/time, strings/chars, class/interface up/downcasts, enums/
  subranges. HIR gets explicit `Cast` nodes; no silent conversion in codegen.
- OOP: inheritance graph and cycle detection; override validity; `FINAL`/`ABSTRACT`;
  interface implementation completeness; access defaults (class/FB variables `PROTECTED`,
  methods `PUBLIC`); `THIS`/`SUPER`; assignment attempt (`?=`).
- References/interfaces: `REF_TO`, `REF`/`NULL`, dereference, `VAR_IN_OUT`, interface assignment,
  nullability rules.
- `ARRAY [*]` placement and static/dynamic array rules.
- `RETAIN`/`NON_RETAIN` policy and persistability per AST §8.11 and State §4; at least the
  diagnostics (`STC-E-RETAIN-001/002`) and effective policy in HIR.
- AT/I/O: address/size/type checks (`STC-E-IO-001`), writes to `%I` (`STC-E-IO-002`), wildcard
  executable use. `STC-E-IO-003` (duplicate `stc:io`) is already parser-side; `STC-E-IO-004`
  (non-object/malformed `stc:io` payload) is specified but M2 currently reports payload-shape
  failures as `STC-E-PRAGMA-002`. Reconcile the two codes in M3 (see §8). Manifest device checks
  are M6.
- `stc:test`/`STC_ASSERT` rules (A-020); decide M3 vs M6.
- Configurations/resources/tasks/program configurations: validate structure and bindings;
  `ConfigHir` if HIR is produced.

### 5.7 Constant evaluation

- Array bounds, enum values, initializers, case labels, task intervals, and AT addresses.
- Use the lexer's arbitrary-precision normalized numeric payloads (`Token.literal`); do not
  re-parse token text. Range-check per target type in sema.
- Define compile-time overflow/division-by-zero behavior; the runtime policy traps, but constant
  evaluation should produce a diagnostic rather than an unchecked value.
- Keep constant evaluation deterministic and snapshot-testable.

### 5.8 HIR lowering (recommended in M3)

If HIR is produced, follow AST §8:

- explicit `Cast` nodes; resolved `CalleeHir`; resolved `PlaceHir`; FB/method/interface calls;
  `VAR_IN_OUT`/`REF_TO` as pointers/references; assignment attempt as a runtime type check;
  `FOR` structure retained with a checked loop variable; initializers as constant data or
  `stc_init`; `ConfigHir`; pragma annotations; retention leaves.
- HIR owns stable IDs and spans and never references CST nodes. Codegen consumes only HIR.
- If HIR is deferred to M4, keep the resolution/type maps and type arena stable and document the
  deferral in `Decisions.md`.

### 5.9 Diagnostics

Define the stable M3 code table **before** implementing diagnostics. Existing codes to preserve:

- Parser: `STC-E-GRAM-101`–`111`.
- I/O: `STC-E-IO-001`–`004`, `STC-W-IO-001`–`003`.
- Retention: `STC-E-RETAIN-001/002`, `STC-W-RETAIN-001`.
- Tests: `STC-E-ASSERT-001`, `STC-E-TEST-001`.
- Runtime: `STC-E-RUN-001`, `STC-E-RUN-RETAIN-001`–`003`, warnings.
- Examples already used in specs: `STC-E-TYPE-012` (cannot assign DINT to BOOL) and
  `STC-W-NAME-002` (user POU shadows a standard name). Do not renumber either.

Proposed new families (final numbers in `Decisions.md`):

| Prefix | Examples |
| --- | --- |
| `STC-E-NAME-*` | 001 unknown name, 003 duplicate definition, 004 inaccessible, 005 invalid context, 006 ambiguous |
| `STC-E-TYPE-*` | 001 type mismatch, 002 invalid operand, 012 assignment mismatch (existing example) |
| `STC-E-CONST-*` | 001 non-constant expression, 002 evaluation failure |
| `STC-E-CONV-*` | 001 no conversion, 002 lossy/explicit conversion required |
| `STC-E-CALL-*` | 001 unknown callee, 002 arity, 003 named/output argument, 004 overload ambiguity |
| `STC-E-ACCESS-*` | 001 access violation |
| `STC-E-OOP-*` | 001 inheritance cycle, 002 override, 003 interface implementation, 004 abstract, 005 final, 006 assignment attempt |
| `STC-E-ARRAY-*` | 001 bound/index, 002 conformant placement |
| `STC-E-REF-*` | 001 reference rules, 002 null, 003 assignment attempt |
| `STC-E-INIT-*` | 001 initializer shape |
| `STC-E-CONFIG-*` | 001 configuration/task/program binding |

Every diagnostic needs code, severity, primary span, labels, notes, and suggestions. Use
`token_span()` for typed AST nodes (node spans include trivia). Use `Ty::Error` to suppress
cascades, and report all independent errors. Snapshot the code, message, primary span, labels,
notes, and suggestions.

Decide and document whether `stc check` runs sema after parser errors. Recommendation: run
collection/resolution on recovered trees with `Ty::Error` and suppress follow-on diagnostics, or
skip sema if parser errors exist; do not emit a cascade of type errors from a missing token.

### 5.10 `stc check` integration

Extend the driver to:

- accept multiple files for `stc check` (M3 bridge);
- lex/parse every file, retaining one `SourceMap` and distinct `FileId`s;
- run collection, resolution, type checking, and (recommended) lowering;
- report lexical + syntax + semantic diagnostics through the same sink;
- keep `stc parse` single-file and unchanged;
- preserve exit codes 0/1/2/3 and the `STC-I-0001` panic boundary.

## 6. M2 behaviors and gotchas that directly affect M3

1. **Per-file trees.** `ParsedFile`/`SyntaxTree` is one file. M3 must combine files itself; there
   is no project AST. `SourceFile::items()` is per file.
2. **No namespace means package root.** A file without `NAMESPACE` contributes to the session/
   package root namespace, not an anonymous module. There is no manifest yet; define the M3
   session root explicitly.
3. **Namespaces merge by name.** Two files with `NAMESPACE ProjectDep` form one scope. File order
   is not semantic. Duplicate definitions in that scope are errors.
4. **No interner yet.** `Name::text()` returns original spelling; `Name::normalized()` allocates
   an uppercase `String`. Build the session `Interner` in M3 (D-058) and use `Symbol` keys. Do not
   assume identifiers were interned by the lexer.
5. **Node spans include trivia.** `SyntaxNode::span()` includes leading/trailing trivia (rowan
   style). Use `token_span()` for diagnostics and typed AST text accessors. `dump_cst` prints
   trivia-inclusive spans; `dump_typed_ast` labels use token extents.
6. **Dotted access representation.** Simple `a.b` is a `PathExpr` with multiple `Name` segments;
   `MemberExpr` is used for access on a computed base (`f().x`, `arr[0].x`). Resolve both.
7. **Assignment classification is syntactic.** `AssignStmt` vs `RefAssignStmt` is chosen by a
   dereference on the left or `REF(...)`/`NULL` on the right (D-055). A bare `x := y` is an
   `AssignStmt`; sema must decide whether it is a reference assignment from types.
8. **Call argument classification.** `Arg::Named` covers `name := expr` (the parser cannot tell an
   input from a reference assignment); `Arg::Output`/`NegatedOutput` cover `=>`. `Arg::RefAssign`
   is currently unused. Sema must validate directions and targets.
9. **Missing/error nodes.** Missing tokens are zero-width leaves; parser errors produce `Error`
   nodes while the CST remains lossless. `Item::Error`, `Stmt::Error`, `Expr::Error`, and
   `VarSection::declarations()` (which skips errors) are reachable. Classify and skip errors;
   do not re-diagnose them as semantic errors without cause.
10. **Empty bodies.** `StmtList` may be empty; `FunctionDecl`/`ProgramDecl` always have a
    `StmtList` (possibly empty), while `FunctionBlockDecl`/`MethodDecl` have `Option<StmtList>`.
    Empty `StmtList` is allowed by the CST invariant (D-054).
11. **Pragmas are side-table annotations.** `Pragma::annotation()` returns the parsed payload;
    do not re-parse. Duplicate `stc:io` is `STC-E-IO-003`; `stc:io` without `AT` is
    `STC-W-IO-002`; unknown keys are preserved with `STC-W-PRAGMA-002`. End-of-block pragmas are
    diagnosed `STC-E-GRAM-110` and remain in the `StmtList`, not on the terminator.
12. **Unsupported constructs are parser-diagnosed.** SFC keywords (`102`), `#` (`103`), IL
    mnemonics followed by operands (`101`), Ed. 4 declaration names (`104`), and `ASSERT(...)`
    call statements (`104`). Ordinary identifiers with those spellings are allowed elsewhere.
    Do not silently accept them in sema.
13. **Parser recovery is deliberate.** A mismatched `END_*` reports both spans and leaves the
    token for the enclosing construct; stray statement terminators at POU level are consumed and
    parsing continues. Sema should not assume every node is structurally perfect.
14. **Deep expressions.** The parser accepts left-associative chains of tens of thousands of
    terms without recursion (a 20k-term test exists). Type checking, resolution, lowering, and
    any tree traversal in M3 must be iterative or depth-limited; do not recursively walk
    `BinaryExpr` trees. The CST dumps are iterative for this reason.
15. **Literal payloads are decoded by the lexer.** Numeric strings are normalized arbitrary
    precision; string payloads are bytes/UTF-16 units/scalars by encoding; typed literal signs are
    inside the token. Do not re-lex or apply unary minus twice. Range checks are sema.
16. **Direct variables.** Indices are `u32`; wildcards carry `wildcard = true` and no indices.
    Wildcards are allowed in declarations/`VAR_ACCESS`; sema rejects them in executable
    expressions. AT type/address checks are `STC-E-IO-001`; `%I` is read-only (`STC-E-IO-002`).
17. **`ARRAY [*]`.** The parser accepts the shape anywhere. Sema enforces conformant placement
    (`VAR_IN_OUT`, FB inputs/outputs) and rejects dynamic locals/globals/returns (A-007).
18. **`THIS`/`SUPER` are syntactically accepted.** The parser builds `PathExpr`s for
    `THIS.member`, `SUPER.method()`, and `SUPER()`. Sema restricts them to method/FB bodies and
    enforces `THIS` is not a standalone variable.
19. **`METHOD` prototypes vs definitions.** Interfaces use `MethodPrototype` (no body); classes/
    FBs use `MethodDecl` (optional body). `MethodDecl::pou_kind()` uses parent pointers.
    `InterfaceDecl::extends()` returns `Path`s; FB/Class `extends()` returns a `TypeRef` and
    `implements()` returns `Path`s.
20. **Invalid nesting is parsed for recovery.** Namespace `PROGRAM`/`CONFIGURATION`/
    `VAR_GLOBAL` and class non-`VAR` sections get a parser `105` but are still built. Collection
    must reject them rather than registering invalid definitions.
21. **`VAR_ACCESS`/`VAR_CONFIG`.** Access paths and configuration instance initializers are
    parsed but not resolved. Sema validates paths, types, `AT`, and bindings.
22. **`RETAIN` policy is a lowering concern.** Parser accepts qualifiers; sema computes the
    effective per-leaf policy and persistability (AST §8.11, State §4).
23. **`stc:test` semantics.** Test-only units are excluded from production analysis/codegen
    (A-020). Decide whether M3 enforces `STC-E-ASSERT-001`/`STC-E-TEST-001` or defers to M6.
24. **No UI harness.** `tests/ui` + `xtask` remain deferred to M6. Put M3 diagnostics in
    `crates/stc-sema/tests` and `crates/stc-driver/tests` unless you add xtask.
25. **Stable `SyntaxKind`.** Appending kinds requires updating `crates/stc-syntax/tests/
    syntax_kinds.txt` and `SyntaxKind::ALL` through the generated macro table. Do not renumber.

## 7. Tests and fuzzing

### 7.1 Semantics tests

- Positive and negative cases per rule category, named for traceability:
  collection/duplicates, namespaces/`USING`, name resolution/shadowing, access control,
  type representation/canonicalization, conversions, operators, calls/overloads, initializers,
  constant expressions, arrays/subranges/structs/strings, references/interfaces/assignment
  attempt, OOP (inheritance/override/abstract/final/interfaces), `THIS`/`SUPER`, configurations/
  tasks/program bindings, retention/AT.
- Multi-file cases: root file without namespace plus two files sharing `NAMESPACE ProjectDep`;
  duplicate definitions; `USING` across files; package-root assumptions.
- Diagnostic snapshots assert code, severity, primary span, labels, notes, and suggestions.
- Every diagnostic code maps to at least one negative test; rule IDs map to tests.
- Type errors include notes/suggestions and actionable spans (`token_span`).
- Deep-chain tests for resolution/type checking (20k-term left-associative expression) must not
  overflow the stack.

### 7.2 Fuzzing

Add `fuzz/fuzz_targets/fuzz_analyzer.rs`:

```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut diagnostics = stc_diag::DiagnosticSink::new();
    let lexed = stc_lexer::tokenize_bytes(stc_span::FileId::new(0), data, &mut diagnostics);
    lexed.stream.check_invariants(&lexed.text).unwrap();
    let parsed = stc_parser::parse(/* ... */);
    // analyze a single-file session; no panic, bounded diagnostics, HIR invariants
});
```

CI's `fuzz-smoke` picks up any new `fuzz_targets/*.rs`. Keep `fuzz/Cargo.toml` and
`fuzz/Cargo.lock` updated.

### 7.3 UI harness

Per the M2 handover, `tests/ui` requires `xtask`; both are deferred to M6. Keep M3 snapshots in
`crates/stc-sema/tests` and `crates/stc-driver/tests` until then.

## 8. Decisions/open items to resolve during M3

Record each in `Specification/Decisions.md` before or as it is implemented:

1. **Interner/session model.** Where `Interner`/`Symbol` lives, how the session owns files,
   package roots, and the `SourceMap`.
2. **Multi-file bridge.** `stc check <file>...` vs `--source`; the M3 session root namespace for
   files without a manifest; how M6 replaces it.
3. **Diagnostic code table.** Final `STC-E-NAME/TYPE/CONST/CONV/CALL/ACCESS/OOP/ARRAY/REF/INIT/
   CONFIG-*` numbers and messages; preserve `STC-E-TYPE-012` and `STC-W-NAME-002`.
4. **HIR in M3 vs M4.** Produce `HirCrate` in M3 (recommended) or stabilize the resolver/type
   maps and defer lowering.
5. **Type canonicalization/alias rules.** How aliases, subranges, enums, strings, arrays, and
   class/FB types canonicalize; type equality across packages.
6. **Conversion matrix.** Implicit vs explicit conversions for every category; literal typing
   (untyped integer/real literals); `BOOL` arithmetic restrictions; bit-string rules.
7. **Overload resolution.** Ordering, ambiguity, named arguments, output bindings, standard
   function overloads, user overloads.
8. **Standard prelude.** Signature table for standard functions/FBs; where it lives; how
   `{stc:no_mangle}`/`export` interact; shadowing warning `STC-W-NAME-002`.
9. **Constant evaluation.** Compile-time overflow/division-by-zero; which expressions are
   constant; enum/array/subrange/case/task/AT evaluation.
10. **Access defaults and package visibility.** Class/FB variable `PROTECTED`, method `PUBLIC`;
    `INTERNAL` package subtree; cross-package resolution without a manifest in M3.
11. **Reference/interface/assignment-attempt rules.** Nullability, dereference counts, interface
    assignments, downcast runtime checks, `REF_TO REF_TO` restrictions.
12. **Conformant arrays and retention.** `ARRAY [*]` placement; `RETAIN` diagnostics/policy and
    whether the full retain layout is M3 or M4.
13. **Test-only semantics.** `{stc:test}` validation, `STC_ASSERT`/`STC-E-TEST-001` timing,
    production exclusion (A-020); M3 vs M6.
14. **`VAR_ACCESS`/`VAR_CONFIG`/I/O validation.** What can be checked without `stc.toml` and
    project devices; which IO diagnostics run in M3 vs M6. Also reconcile `STC-E-IO-004`
    (malformed/non-object `stc:io` payload, per IO §4 and the IO code table) with the parser's
    current `STC-E-PRAGMA-002` for payload-shape failures; update Tokenization, IO, Grammar, and
    tests consistently.
15. **`stc check` on parser errors.** Run sema with `Ty::Error`, or skip sema; how to avoid
    cascades.
16. **UI harness.** Confirm deferral to M6 unless xtask is added.
17. **Deterministic ordering.** Definition collection and diagnostic order across files;
    stable `DefId`/`Symbol` allocation for snapshots.

## 9. Suggested order

1. Commit M2; record M3 decisions and the diagnostic code table.
2. Add `stc-hir`: arenas, IDs, `Ty`/`TyArena`, `Interner`, HIR node shapes from AST §5.
3. Add the minimal multi-file session and extend `stc check` to multiple files.
4. Implement collection: namespaces, `USING`, POUs, types, globals, configurations; duplicate/
   nesting diagnostics.
5. Implement type representation/canonicalization and the standard signature prelude.
6. Implement name resolution (scopes, shadowing, qualified names, `THIS`/`SUPER`, access).
7. Implement type checking, conversions, and constant evaluation.
8. Lower to HIR (recommended) with explicit casts/resolved callees/places.
9. Wire `stc check` reporting and diagnostic snapshots.
10. Add semantic conformance/negative tests, traceability, multi-file tests, deep-chain tests, and
    `fuzz_analyzer`.
11. Update docs/status; run the full CI-equivalent checks; commit M3.

## 10. Definition of done for M3

- [ ] Positive and negative tests exist for every semantic rule category; rule IDs and diagnostic
      codes are traceable.
- [ ] Name resolution covers scopes, shadowing, namespaces, `USING`, qualified names, package
      roots, `THIS`/`SUPER`, access control, and overloads.
- [ ] Type checking covers assignments, calls (positional/named/output), operators, conditions,
      loops, arrays/subranges/structs/strings, references/interfaces/assignment attempt, OOP
      inheritance/override/abstract/final/interfaces, initializers, and constants.
- [ ] Constant expressions are evaluated for bounds, enums, initializers, case labels, task
      intervals, and `AT` addresses.
- [ ] Diagnostics are stable, snapshot-tested, include actionable spans/notes/suggestions, and
      suppress cascades with `Ty::Error`.
- [ ] `stc check` handles multiple files, reports lexical + syntax + semantic diagnostics, and
      keeps the exit-code/JSON contract. `stc parse` remains parse-only.
- [ ] HIR is produced in M3, or the deferral is recorded and the resolver/type maps are frozen for
      M4.
- [ ] No codegen is added; `stc-codegen` still does not exist or does not depend on syntax/parser.
- [ ] Multi-file namespace/package-root behavior is tested (the `ProjectDep` case).
- [ ] `fuzz_analyzer` is clean in CI smoke; deep-chain traversals do not overflow.
- [ ] `cargo test --workspace --locked`, clippy `-D warnings`, `cargo fmt --check`,
      `check_docs.py`, and Markdown lint pass.
- [ ] README, Decisions, conformance/profile status, and relevant specs reflect M3.

## 11. Commands

```sh
cargo test --workspace --locked
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all --check
python tools/ci/check_docs.py
npx --yes markdownlint-cli2@0.24.0

# Fuzz workspace
cargo check --manifest-path fuzz/Cargo.toml --locked
cargo +nightly fuzz run fuzz_parser -- -max_total_time=60 -rss_limit_mb=2048 -timeout=25
cargo +nightly fuzz run fuzz_analyzer -- -max_total_time=60 -rss_limit_mb=2048 -timeout=25

# Fuzz targets are outside the workspace, so format/check them explicitly
rustfmt --edition 2024 --check fuzz/fuzz_targets/fuzz_lexer.rs \
  fuzz/fuzz_targets/fuzz_parser.rs fuzz/fuzz_targets/fuzz_analyzer.rs
```

In PowerShell, set environment variables first, for example
`$env:RUSTFLAGS = "-D warnings"; cargo clippy --workspace --all-targets --locked -- -D warnings`.

## 12. Key files from M2

- `crates/stc-syntax/src/kind.rs` — stable `SyntaxKind` + mapping.
- `crates/stc-syntax/src/tree.rs` — arena CST, spans, tokens, trivia, invariants.
- `crates/stc-syntax/src/ast.rs` — typed AST views and accessors.
- `crates/stc-syntax/src/pragma.rs` — pragma JSON and annotation rules.
- `crates/stc-parser/src/{expr,stmt,item,pou,config}.rs` — grammar surface.
- `crates/stc-parser/src/parser.rs` — cursor, recovery, depth, pragmas.
- `crates/stc-driver/src/main.rs` — `stc parse`/`check`, diagnostics, exit codes.
- `crates/stc-parser/tests/traceability.rs` — production/rule/code traceability.
- `crates/stc-parser/tests/{corpus,snapshots,grammar,recovery,pragmas,typed_ast}.rs`.
- `Specification/Decisions.md` D-045–D-058; `Grammar.md` §13.1; `AST.md` §5–§8.
