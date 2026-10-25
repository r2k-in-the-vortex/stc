# AST, CST, and HIR

Status: Draft  
Profile: `stc-2013-oop-utf8-v1`  
Related: [Tokenization](../Syntax/Tokenization.md), [Grammar](../Syntax/Grammar.md), [Architecture](../Architecture/Architecture.md)

This document defines the compiler's syntax and semantic data structures. The design separates
three layers so that diagnostics, formatting, name resolution, and codegen do not fight over one
tree:

1. **Token stream** — lexical tokens and trivia (defined in the tokenization spec).
2. **CST** — a lossless concrete syntax tree that preserves every byte, including whitespace,
   comments, and pragmas.
3. **Typed AST views** — ergonomic, typed accessors over CST nodes, used by the parser tests,
   formatter, and early analysis.
4. **HIR** — a resolved, typed, desugared representation used by codegen. HIR does not reference
   CST nodes; it owns stable IDs and spans.

The parser produces the CST and typed AST. Name resolution and type checking consume the typed
AST and produce HIR.

## 1. Design goals

- **Lossless**: `cst.text() == source` for every valid or invalid input.
- **Span-complete**: every token, trivia item, CST node, AST node, HIR node, and diagnostic has a
  `Span`.
- **Error-tolerant**: malformed input produces `Error` nodes, not a panic or a discarded tree.
- **Case-insensitive**: symbol lookup uses a normalized key; original spelling is retained.
- **Stable ordering**: HIR declarations and expressions are indexed in source order where
  possible, with deterministic tie-breaking for tests and IR output.
- **No hidden state**: all resolution results are explicit in HIR; codegen never re-resolves a
  name.
- **Cheap to extend**: new syntax (future Ed. 4 features) adds CST kinds and typed wrappers
  without rewriting the analyzer.

## 2. Source map, spans, and symbols

```rust
pub struct FileId(u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Span {
    pub file: FileId,
    pub start: u32, // byte offset, inclusive
    pub end: u32,   // byte offset, exclusive
}

pub struct SourceMap {
    files: Vec<SourceFile>,
    line_starts: Vec<Vec<u32>>, // per file
}

impl SourceMap {
    pub fn location(&self, span: Span) -> Location; // line/column, 1-based
    pub fn text(&self, span: Span) -> &str;
    pub fn line_snippet(&self, span: Span) -> &str;
}

pub struct Location {
    pub file: FileId,
    pub line: u32,
    pub column: u32, // Unicode scalar count, 1-based
}
```

Symbols:

```rust
pub struct Symbol(u32);

pub struct Interner {
    // original text -> Symbol; normalized key handles case-insensitivity
}

impl Interner {
    pub fn intern(&mut self, text: &str) -> Symbol;
    pub fn text(&self, symbol: Symbol) -> &str;
    pub fn normalized(&self, symbol: Symbol) -> &str; // ASCII uppercase
}

pub struct Name {
    pub symbol: Symbol,
    pub span: Span,
}
```

- The lexer stores each identifier's normalized uppercase key; M2 preserves the original spelling
  and the key in `Name` nodes (D-058). The session `Interner` is built during M3 name resolution;
  M2 does not intern identifiers.
- `Name` equality uses `Symbol` (case-insensitive). Diagnostics print `interner.text(symbol)`.
- Namespace-qualified paths are represented as `Vec<Name>` (segments) plus a span covering the
  whole path.

## 3. CST

### 3.1 Shape

The CST is a rowan-style green/red tree or an equivalent arena tree. The implementation may use
`rowan` or a hand-rolled arena; the contract is:

- Nodes have a `SyntaxKind`, a `Span`, ordered children, and an optional parent pointer.
- Tokens and trivia are leaves; no byte is lost.
- `SyntaxNode::text()` concatenates leaves and equals the source slice for the node's span.
- Nodes are immutable after construction.
- The tree can be traversed pre-order, post-order, and by descendant token iteration.

### 3.2 Syntax kinds

Token kinds are defined in the tokenization spec. Node kinds:

```text
SourceFile
UsingDirective
NamespaceDecl
DataTypeDecl
TypeDecl
SimpleTypeDecl
SubrangeTypeDecl
EnumTypeDecl
NamedValueTypeDecl
ArrayTypeDecl
StructTypeDecl
StringTypeDecl
RefTypeDecl
VarSection
VarDecl
EdgeDecl
AccessDecl
FunctionDecl
FunctionBlockDecl
ProgramDecl
ClassDecl
InterfaceDecl
MethodDecl
MethodPrototype
ConfigDecl
ResourceDecl
TaskConfig
ProgramConfig
ProgConfElem
VarAccessSection
ConfigInit
ConfigInstInit

StmtList
AssignStmt
RefAssignStmt
AssignmentAttemptStmt
CallStmt
ReturnStmt
IfStmt
CaseStmt
CaseSelection
ForStmt
WhileStmt
RepeatStmt
ExitStmt
ContinueStmt
EmptyStmt

Expr
BinaryExpr
UnaryExpr
CallExpr
MemberExpr
IndexExpr
DerefExpr
ParenExpr
LiteralExpr
PathExpr
RefValueExpr
AddressOfExpr
SizeofExpr
PointerConversionExpr
SizeofOperand

Name
Path
TypeRef
ParamList
Param
Pragma
Error
```

This list is illustrative; the implementation may split nodes further (for example, separate
`StructElem` from `VarDecl`) as long as the AST contract holds.

### 3.3 Stable `SyntaxKind`

- `SyntaxKind` is `#[repr(u16)]` and `#[non_exhaustive]` with explicit discriminants.
- Existing discriminants are never renumbered or reused. New kinds are appended; removed kinds
  remain reserved.
- Kind names and their canonical debug/JSON spellings are stable.
- A checked-in stability table (name, discriminant) is enforced by a test. Additive changes update
  the table and follow semver for `stc-syntax`.
- The rest of the CST (node handles, traversal internals) remains internal in v1. The design keeps
  spans, trivia, error nodes, and stable kinds so a public CST/traversal API can be added in
  v2/v3 for IDE tools.

### 3.4 CST invariants

- Every non-terminal node has at least one child unless it is an `Error` node or an empty
  `StmtList` (the grammar's `(stmt? ';')*` permits an empty body; D-054).
- `Error` nodes may contain tokens and nested nodes; they never contain unreachable siblings.
- A node's span is the union of its children's spans. Nodes with zero-width content (for example
  a missing `;`) use an empty span at the insertion point.
- Comments and whitespace are leaves attached to the CST, not discarded. The parser may attach
  leading trivia to the next token.

## 4. Typed AST views

The typed AST is a set of zero-cost wrappers around CST nodes. It is not a second tree.

```rust
pub trait AstNode: Sized {
    fn cast(node: &SyntaxNode) -> Option<Self>;
    fn syntax(&self) -> &SyntaxNode;
    fn span(&self) -> Span { self.syntax().span() }
}

pub struct SourceFile(SyntaxNode);
pub struct FunctionDecl(SyntaxNode);
pub struct VarSection(SyntaxNode);
pub struct VarDecl(SyntaxNode);
pub struct Stmt(SyntaxNode);
pub struct Expr(SyntaxNode);

pub enum Item {
    Using(UsingDirective),
    Namespace(NamespaceDecl),
    DataType(DataTypeDecl),
    Function(FunctionDecl),
    FunctionBlock(FunctionBlockDecl),
    Program(ProgramDecl),
    Class(ClassDecl),
    Interface(InterfaceDecl),
    Configuration(ConfigDecl),
    GlobalVars(VarSection),
}

pub enum StmtKind {
    Assign(AssignStmt),
    RefAssign(RefAssignStmt),
    Attempt(AssignmentAttemptStmt),
    Call(CallStmt),
    Return(ReturnStmt),
    If(IfStmt),
    Case(CaseStmt),
    For(ForStmt),
    While(WhileStmt),
    Repeat(RepeatStmt),
    Exit(ExitStmt),
    Continue(ContinueStmt),
    Empty(EmptyStmt),
    Error(ErrorNode),
}

pub enum ExprKind {
    Binary(BinaryExpr),
    Unary(UnaryExpr),
    Call(CallExpr),
    Member(MemberExpr),
    Index(IndexExpr),
    Deref(DerefExpr),
    Paren(ParenExpr),
    Literal(LiteralExpr),
    Path(PathExpr),
    RefValue(RefValueExpr),
    AddrOf(AddressOfExpr),
    Sizeof(SizeofExpr),
    PointerConversion(PointerConversionExpr),
    Error(ErrorNode),
}
```

Typed accessors return `Option` for optional children and `Result` only when a structural
invariant is broken. The parser guarantees the shape; AST accessors do not re-validate grammar.

Node spans include attached trivia (rowan-style), so diagnostics that point into a typed AST node
use `token_span()`, the first-to-last non-trivia token extent (D-052). Simple dotted access such
as `a.b` is a `PathExpr` with multiple `Name` segments; `MemberExpr` represents access on a
computed base such as `f().x` or `arr[0].x` (D-052).

### 4.0 Pragmas

```rust
pub struct PragmaAnnotation {
    pub name: String,                     // after "stc:"
    pub payload: Option<serde_json::Value>,
    pub span: Span,
}
```

- Leading pragmas attach to the following item; a trailing pragma on a variable declaration
  attaches to that declaration.
- `stc:` pragmas are parsed as strict JSON (objects, arrays, strings, numbers, booleans, null).
  Non-`stc` pragmas remain raw and are ignored/warned.
- The annotation keeps the raw JSON for metadata and future I/O/device keys. Unknown keys are
  preserved where the pragma schema permits.

### 4.1 Typed AST invariants

- A `FunctionDecl` has at most one result type and one body.
- `IfStmt` has one or more branches and an optional `ELSE` block.
- `CaseStmt` has one or more selections; at most one `ELSE`.
- `CallExpr` has a callee path and an argument list; arguments preserve source order and whether
  they are named/positional/output.
- `AddressOfExpr` has exactly one place operand; `SizeofExpr` has exactly one type or place
  operand; `PointerConversionExpr` has a target type and one expression operand.
- `MethodDecl` records its enclosing POU kind (class, FB, interface) and access specifier.
- Every node has a span; missing delimiters are represented by zero-width spans and an `Error`
  node/flag.

## 5. HIR

HIR is where names and types become explicit. It is produced after name resolution and type
checking. Codegen consumes only HIR.

```rust
pub struct PackageId(u32);
pub struct DefId(u32);

pub struct PackageInfo {
    pub id: PackageId,
    pub name: String,       // manifest name
    pub root_namespace: String,
}

pub struct HirCrate {
    pub defs: Arena<Def>,
    pub bodies: Arena<Body>,
    pub types: TypeArena,
    pub globals: Vec<Global>,
    pub packages: Vec<PackageInfo>,
    pub dependency_order: Vec<PackageId>,
    pub config: Option<ConfigHir>,
    pub exports: Vec<Export>,
    pub retain_layout: Option<RetainLayout>,
    pub identity: Option<IdentityCatalog>,
}

/// Compiler/runtime identity metadata for the GOT manifest. This is not language semantics:
/// no ST source construct can observe a `GotId`, and type checking does not depend on it.
pub struct IdentityCatalog {
    pub roots: Vec<RootIdentity>,      // per persistent field/leaf
    pub functions: Vec<FunctionIdentity>,
    pub types: Vec<TypeIdentity>,
    pub instances: Vec<InstanceIdentity>,
    pub tasks: Vec<TaskIdentity>,
    pub exports: Vec<ExportIdentity>,
}

pub struct RootIdentity {
    pub got_id: [u8; 16],              // content-derived 128-bit identity
    pub path: String,                  // current normalized qualified path
    pub owner: Option<DefId>,
    pub ty: TyId,
    pub kind: RootKind,                // field, array element/handle, string, overlap, direct AT
}

pub struct RetainLayout {
    pub leaves: Vec<RetainLeaf>, // sorted by path, then offset
    pub size: u64,
    pub layout_hash: u64,
}

pub struct RetainLeaf {
    pub path: String, // e.g. Main.Timer.State
    pub got_id: [u8; 16],
    pub ty: TyId,
    pub offset: u64,
    pub size: u64,
}

pub struct Body {
    pub owner: DefId,
    pub params: Vec<ParamHir>,
    pub locals: Vec<LocalHir>,
    pub stmts: Vec<StmtHir>,
    pub span: Span,
}

pub struct StmtHir {
    pub kind: StmtKindHir,
    pub span: Span,
}

pub enum StmtKindHir {
    Assign { place: PlaceHir, value: ExprHir },
    RefAssign { place: PlaceHir, value: ExprHir },
    Attempt { place: PlaceHir, value: ExprHir },
    Call { callee: CalleeHir, args: Vec<ArgHir>, result: Option<PlaceHir> },
    Return { value: Option<ExprHir> },
    If { arms: Vec<(ExprHir, Vec<StmtHir>)>, else_body: Vec<StmtHir> },
    Case { selector: ExprHir, arms: Vec<CaseArmHir>, else_body: Vec<StmtHir> },
    For { var: PlaceHir, start: ExprHir, end: ExprHir, step: ExprHir, body: Vec<StmtHir> },
    While { cond: ExprHir, body: Vec<StmtHir> },
    Repeat { body: Vec<StmtHir>, cond: ExprHir },
    Exit,
    Continue,
}

pub enum ExprKindHir {
    Literal(LiteralHir),
    Local(LocalId),
    Global(DefId),
    Param(ParamId),
    Field { base: Box<ExprHir>, field: FieldId },
    Index { base: Box<ExprHir>, indices: Vec<ExprHir> },
    Deref(Box<ExprHir>),
    Binary { op: BinOp, lhs: Box<ExprHir>, rhs: Box<ExprHir>, ty: TyId },
    Unary { op: UnOp, operand: Box<ExprHir>, ty: TyId },
    Call { callee: CalleeHir, args: Vec<ArgHir>, ty: TyId },
    Ref { target: ExprId },
    Null,
    AddressOf { target: ExprId },
    SizeOf { operand: SizeOfHir },
    PointerConversion { operand: ExprId, to: TyId, kind: CastKind },
    Cast { operand: ExprId, to: TyId, kind: CastKind },
}

pub enum SizeOfHir {
    Type(TyId),
    Place(ExprId),
}
```

Notes:

- `PlaceHir` is a resolved lvalue: local, parameter, global, field, index, or deref. A deref of a
  `POINTER TO` place and a deref of a `REF_TO` place share the same place node; the operand type
  distinguishes them.
- `SizeofHir` records whether `SIZEOF` named a type or a place. A bare/qualified name is resolved
  by the analyzer: a visible place wins, otherwise the name is resolved as a type.
- `CalleeHir` is `Function(DefId)`, `FunctionBlock(DefId)`, `Method { receiver, def }`, or
  `InterfaceMethod { receiver, slot }`. The analyzer resolves overloads and stores the chosen
  definition.
- `ArgHir` records the formal parameter ID, direction (input/output/in-out), and expression.
- `Cast` nodes are explicit. Codegen never inserts implicit conversions on its own.
- `PointerConversion` is reserved for the explicit `POINTER TO T`↔`REF_TO T` forms; scalar and
  other conversions remain `Cast` nodes.
- `TyId` indexes the type arena; types are canonicalized (same type = same ID).
- `ConfigHir` contains the selected configuration, resources, tasks, program instances, and
  variable bindings needed by the runtime generator.
- `Export` records `{stc:export}` symbols and their stable C names.
- `IdentityCatalog` is produced after semantic analysis and consumed by codegen to emit the GOT
  manifest. `GotId`s are content-derived identities; a tracked rename/move changes the current
  path while retaining the identity. The catalog is a compiler/runtime contract, never a source
  feature: ST code cannot spell or compare a `GotId`, and identity does not affect typing.
- Per-field roots mean a persistent aggregate contributes one identity record per field
  recursively; arrays/strings/`OVERLAP` follow the resolved hybrid/one-root/atomic rules.

## 6. Type representation

```rust
pub enum Ty {
    Error,
    Bool,
    Int(IntTy),       // signed/unsigned, width
    Real(RealTy),     // f32/f64
    BitString(u16),   // BYTE/WORD/DWORD/LWORD width
    Time(TimeTy),     // TIME/LTIME
    Date(DateTy),     // DATE/LDATE/TOD/LTOD/DT/LDT
    Char(CharTy),     // CHAR/WCHAR/UCHAR
    String(StringTy), // STRING/WSTRING/USTRING + capacity
    Array(ArrayTy),
    Struct(StructTy),
    Enum(EnumTy),
    Subrange(SubrangeTy),
    Ref(RefTy),
    Ptr(PtrTy), // raw pointer; STC-POINTER
    Class(ClassTy),
    Interface(InterfaceTy),
    FunctionBlock(FbTy),
    Function(FunctionTy),
    Program(ProgramTy),
    Generic(GenericTy), // only in standard function signatures
}
```

Canonicalization rules:

- Integer and bit-string widths are explicit; `BOOL` is not an integer type for arithmetic.
- Subrange types retain their base integer type and bounds; assignment checks the bounds.
- Arrays retain element type and bounds; variable-length arrays carry a conformant marker and are
  only valid as `VAR_IN_OUT`/FB parameters in v1 (A-007). Dynamic locals/globals/returns are
  rejected.
- Pointer types retain their target type and are canonicalized structurally. `POINTER TO T` is
  distinct from `REF_TO T`; pointer storage transience and target restrictions are checked at
  declarations and places, not in the type identity. A persistent root rejects any type that
  transitively contains a pointer.
- Structs retain field order, `OVERLAP` flags, and `AT` bindings.
- Class/FB types retain a base type, implemented interfaces, methods, fields, and vtable slots.
- Interface types retain method prototypes and a stable slot order (sorted by name + signature).
- String types retain kind (`SBCS`, `DBCS`, `Unicode`) and capacity. For `Unicode`, capacity is
  characters and storage is 4 bytes per character (32-bit scalar); UTF-8 is the
  source/interchange encoding, not the storage encoding.
- `USTRING` is `String(StringTy { kind: Unicode, capacity: CharCount, scalar_width: 32 })`;
  `UCHAR` is `Char(CharTy::UnicodeScalar32)`.

## 7. Name resolution

Resolution produces a `Resolution` map keyed by syntax node IDs or spans:

```rust
pub enum Res {
    Def(DefId),
    Local(LocalId),
    Param(ParamId),
    Field(FieldId),
    Namespace(NamespaceId),
    Package(PackageId),
    StdFunction(StdFnId),
    StdFb(StdFbId),
    Err,
}
```

Scopes, from innermost to outermost:

1. POU local variables (`VAR`, `VAR_TEMP`, `VAR_INPUT`, `VAR_OUTPUT`, `VAR_IN_OUT`).
2. POU name (function result / FB instance self).
3. Class/FB members and inherited members (with access checks).
4. Namespace scope, including `USING` imports.
5. Package root namespace (for cross-package references).
6. Global variables.
7. Standard library prelude (functions and FBs).
8. Built-in elementary types and literals.

Resolution rules:

- Case-insensitive, exact normalized-key matching.
- A local variable shadows a member; a member shadows a namespace import; an explicit import
  shadows a wildcard `USING`.
- `THIS.member` resolves starting at the current class/FB type; `SUPER.member` starts at the
  base type.
- Qualified names resolve segment by segment; a namespace segment may be followed by a type,
  POU, or variable.
- Overload sets are resolved after argument typing; ambiguity is an error.
- Unknown names produce `STC-E-NAME-*` diagnostics with a "did you mean" suggestion using a
  bounded edit-distance search.
- Access control is checked after resolution: `PRIVATE` (declaring POU only), `PROTECTED`
  (declaring POU and derived types), `INTERNAL` (same package/namespace subtree), `PUBLIC`
  (visible across packages).
- Package boundaries participate in resolution: a dependency's package name (or alias) is a root
  namespace; symbols from a dependency are not implicitly imported. `INTERNAL` symbols are not
  visible outside their package.

## 8. Lowering from AST to HIR

Lowering is a single pass after type checking, with these transformations:

1. **Implicit conversions** become explicit `Cast` nodes.
2. **Overloaded standard functions** become concrete `CalleeHir::Function` entries; operator
   nodes become calls to the corresponding standard function where the standard defines one.
3. **FB calls** become `CalleeHir::FunctionBlock` with an instance receiver and parameter
   assignments. Output bindings become assignment targets.
4. **Method calls** become `CalleeHir::Method` with an explicit `this` receiver. Calls through an
   interface or base-class reference become `InterfaceMethod`/virtual slots.
5. **`VAR_IN_OUT`** parameters, `REF_TO` values, and `POINTER TO` values become
   pointer/reference operations. `ADR` lowers to the address of a resolved place, `SIZEOF` folds
   to an integer constant, and pointer↔reference conversions lower to the host resolver calls
   (or `NULL` on failure).
6. **Class/interface references** become explicit reference values; assignment attempts become
   runtime type checks.
7. **`FOR`** retains its structure; codegen emits the initialization, condition, body, and
   increment explicitly. The loop variable is a local with checked bounds.
8. **Initializers** are lowered to constant data when possible, otherwise to `stc_init` code.
9. **Configuration** is lowered to `ConfigHir` with selected resources/tasks/programs and I/O
   bindings.
10. **Pragmas** are resolved to HIR annotations (`test`, `export`, `no_mangle`, `inline`, `io`).
11. **Retention**: the effective `RETAIN`/`NON_RETAIN` policy is computed per storage leaf
    (explicit qualifier, section qualifier, instance inheritance, aggregate inheritance). The
    result is a `RetainLayout` of persistable leaves; non-persistable retained fields
    (references/interfaces/pointers) are diagnosed. Vtable/type pointers are excluded and
    reinitialized before state load.

Lowering never re-resolves a name and never inserts a conversion silently; every semantic choice
is visible in HIR.

## 9. Diagnostics and error nodes

```rust
pub struct Diagnostic {
    pub code: &'static str,        // STC-E-..., STC-W-...
    pub severity: Severity,
    pub message: String,
    pub primary: Span,
    pub labels: Vec<(Span, String)>,
    pub notes: Vec<String>,
    pub suggestions: Vec<Suggestion>,
}
```

- The parser reports syntax errors and produces `Error` nodes; it does not stop at the first
  error.
- The analyzer reports name/type errors and continues with `Ty::Error` to avoid cascades.
- Codegen refuses to run if any error-severity diagnostic exists.
- Error codes are stable and documented. Tests snapshot the code, message, span, and labels.
- Suggestions include machine-applicable replacements where safe (`=`, `:=`, missing `;`,
  misspelled keyword/identifier).

## 10. Example

Source:

```st
FUNCTION Add : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  Add := a + b;
END_FUNCTION
```

CST (abridged):

```text
FunctionDecl
  FUNCTION
  Name "Add"
  Colon
  TypeRef Name "DINT"
  VarSection VAR_INPUT
    VarDecl Name "a" Colon TypeRef Name "DINT" Semicolon
    VarDecl Name "b" Colon TypeRef Name "DINT" Semicolon
  END_VAR
  StmtList
    AssignStmt
      PathExpr Name "Add"
      Assign
      BinaryExpr
        PathExpr Name "a"
        Plus
        PathExpr Name "b"
      Semicolon
  END_FUNCTION
```

HIR (abridged):

```text
Def Function(Add) -> DINT
Body(Add):
  params: a: DINT, b: DINT
  stmts:
    Assign(place=Result(Add), value=Binary(Add, Local(a), Local(b)))
```

The lowering of `a + b` inserts the standard arithmetic function resolution and any implicit
conversions. In this case both operands are `DINT`, so the HIR is a `Binary(Add)` with type
`DINT`.

## 11. Memory and performance

- CST nodes and tokens are immutable and arena/slab allocated.
- AST wrappers are `Copy`-able handles; no tree duplication.
- HIR uses dense arenas (`Id<T>`) for defs, bodies, types, expressions, and statements.
- Symbols are interned once per compiler session.
- Name resolution uses `HashMap` keyed by `Symbol` in each scope; expected scope sizes are small.
- No global mutable state; the driver owns the compiler session.
- Parallelism is not required in v1; the data structures do not prevent per-POU checking later.

## 12. Testing contract

- **CST round-trip**: for every corpus file, `cst.text() == source`.
- **SyntaxKind stability**: the checked-in name/discriminant table matches the enum exactly;
  renumbering or reuse fails, additive changes update the table and follow semver.
- **AST snapshots**: a compact debug dump of typed AST nodes and spans.
- **HIR snapshots**: resolved names, types, and explicit casts.
- **Error recovery**: malformed snippets produce expected diagnostics and no panic.
- **Property tests**: random token insertions/deletions do not panic and preserve the
  token/trivia coverage invariant.
- **Fuzzing**: the lexer/parser entry point is fuzzed with arbitrary bytes.

## 13. Resolved AST questions

No open AST questions currently.

- **Q-016 resolved (A-021):** `SyntaxKind` is stable and append-only; the full CST/traversal API
  is planned for v2/v3 for IDE tooling.
- **Online-change identity resolved (A-023/A-026):** HIR may carry an `IdentityCatalog` for the
  GOT manifest; it is optional for compiler-only uses. `GotId`s, manifest history, and catalog
  records are compiler/runtime contracts, not language semantics, and do not change the CST or
  typed AST.
