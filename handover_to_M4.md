# Handover to M4 — LLVM IR Core Codegen

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-24  
Prepared after M3 (HIR, name resolution, type checking) completion

Read this together with the project plan (`Specification/00-Project-Plan.md` §8,
M4–M6), `Specification/Architecture/Architecture.md` §2–§6,
`Specification/Architecture/Runtime-and-Execution.md` §1–§4,
`Specification/Architecture/CLI-and-Manifest.md` §1,
`Specification/Architecture/IO-and-Devices.md` §2–§5,
`Specification/Architecture/State-and-Retain.md` §2–§7,
`Specification/Architecture/Testing-and-Verification.md` §1–§4 and §6,
`Specification/AST/AST.md` §5–§8, and `Specification/Decisions.md` (D-059–D-075).

## 1. State at handover

M3 is implemented, verified, and committed as `4ddd5ea` (`M3`). The working tree is clean as of
2026-10-24. M2 is `24da2f7`; M1 is `51cab11`.

| Item | State |
| --- | --- |
| Workspace | `stc-span`, `stc-diag`, `stc-lexer`, `stc-syntax`, `stc-parser`, `stc-hir`, `stc-sema`, `stc-driver`; `fuzz/` is separate |
| Toolchain | Rust 1.99.1, edition 2024; `unsafe_code = "forbid"` workspace lint |
| Tests | 286 passing (`cargo test --workspace --locked`) |
| Checks | clippy `-D warnings`, `cargo fmt --check`, `check_docs.py`, Markdown lint all pass |
| Fuzzing | `fuzz_lexer`, `fuzz_parser`, `fuzz_analyzer`; analyzer smoke: 6,239 runs in 21 s, no findings |
| Host tools | clang/LLVM 23.1.4, host triple `x86_64-pc-windows-msvc`; `llvm-ar` available |
| Uncommitted | none |
| Codegen | absent: no `stc-codegen`, no `stc-runtime`, no `stc build` |
| Manifest | absent; `stc check` still uses the M3 `_root`/`PackageId(0)` bridge |

M3 delivered a resolved, type-checked HIR with stable arena IDs, explicit casts, resolved call
targets/places, configurations, and deterministic dumps. It did not add codegen, a runtime
library, clang invocation, artifacts, or any manifest/project discovery.

`stc check <file>...` is the current end of the pipeline. `stc parse` remains single-file and
syntax-only. Semantic analysis is skipped when lexing/parsing produced errors (D-070).

## 2. M4 scope and exit criteria

From the project plan:

> Deliverables: typed HIR -> textual IR; scalar types and operators; control flow; functions and
> programs; globals and initialization; process-image metadata for `AT`; runtime ABI stubs; clang
> invocation; `--emit=ir,obj,exe`; execution tests; deterministic IR snapshots.
>
> Exit criteria:
>
> - A set of small ST programs produces IR accepted by `clang` and runs correctly.
> - Both executable and library mode link and execute a no-op cycle.
> - IR is deterministic and free of target-specific offsets.

M4 is the first milestone that produces an artifact that can execute. Its job is to prove the
HIR-to-IR contract and the clang/runtime/ABI path on **elementary scalar code**, not to finish the
language. FBs, OOP, aggregates, strings, references, standard functions, and the standard FB
library are M5. The manifest and full CLI/project system are M6.

Recommended M4 boundary:

| Area | M4 | Deferred |
| --- | --- | --- |
| Types | `BOOL`, `SINT`/`INT`/`DINT`/`LINT` and unsigned counterparts, `BYTE`/`WORD`/`DWORD`/`LWORD`, `REAL`/`LREAL`, `TIME`/`LTIME`, `DATE`/`LDATE`/`TOD`/`LTOD`/`DT`/`LDT`, `CHAR`/`WCHAR`/`UCHAR` | enums, subranges, named-value types, structs, arrays, strings, references, classes, FBs, interfaces, functions as values (M5) |
| Expressions | literals, scalar places, unary/binary scalar operators, explicit/implicit scalar casts, user function calls, explicit conversion calls, direct variables and bit access | `REF`/`NULL`/deref/`?=`, FB/method/interface calls, `SUPER`, standard functions, aggregate initializers (M5) |
| Statements | assignment, call, `RETURN`, `IF`, `CASE`, `FOR`, `WHILE`, `REPEAT`, `EXIT`, `CONTINUE` | `RefAssign`, `Attempt` (M5) |
| Functions | scalar result, scalar `VAR_INPUT` parameters, scalar `VAR_TEMP`/`VAR` locals | output/in-out parameters, methods (M5) |
| Programs | scalar fields and body, one selected configuration, program instances, scalar bindings and `VAR_CONFIG` inits | FB/class instance state, task scheduling, priorities, mock clock, test runner (M5/M6) |
| Globals | scalar globals and scalar constant initializers | aggregates/strings/references (M5), package graph (M6) |
| I/O | `AT` scalar direct variables, process-image layout/metadata, unbound input defaults | device tree, project bindings, `VAR_ACCESS` metadata, hardware backends (M6) |
| ABI | `stc_init`/`stc_cycle`/`stc_shutdown`, executable `main`, static library exports, traps, cycle clock stub, process-image access, retained-state ABI for scalar leaves | vtables/type descriptors, assignment attempt, full retained-state file I/O (M5/M6) |
| Build | `stc build <file>...` bridge, `--emit=ir,obj,exe[,lib,metadata]`, clang discovery/link, IR snapshots, execution tests | manifest, `stc run`, `stc test`, shared libraries, debug info (M6) |

Anything outside the M4 column must fail with an explicit codegen diagnostic. Silent emission of
wrong IR is worse than a deliberate “not implemented until M5” error.

## 3. Contracts M3 provides

### 3.1 Crates and dependency direction

```text
stc-span -> stc-hir -> stc-sema -> stc-parser/stc-syntax
stc-hir -> stc-codegen -> stc-driver
stc-span -> stc-codegen
```

Rules to preserve:

- `stc-hir` does not depend on `stc-syntax` or `stc-parser`.
- `stc-codegen` must depend only on `stc-hir` (and `stc-span` for `Span`) and standard library
  crates. It must not depend on `stc-syntax`, `stc-parser`, or `stc-sema`.
- `stc-runtime` is built independently as a Rust `staticlib` and is not a dependency of
  `stc-codegen`/`stc-sema`. The driver locates and links the artifact.
- `stc-driver` may depend on `stc-codegen`.

### 3.2 HIR entry point

```rust
pub struct HirCrate {
    pub interner: Interner,
    pub types: TyArena,
    pub defs: Vec<Def>,
    pub fields: Vec<FieldHir>,
    pub params: Vec<ParamHir>,
    pub locals: Vec<LocalHir>,
    pub bodies: Vec<Body>,
    pub exprs: Vec<ExprHir>,
    pub stmts: Vec<StmtHir>,
    pub packages: Vec<PackageInfo>,
    pub std_fns: Vec<StdFunction>,
    pub entry_configuration: Option<DefId>,
}

impl HirCrate {
    pub fn def(&self, id: DefId) -> &Def;
    pub fn expr(&self, id: ExprId) -> &ExprHir;
    pub fn stmt(&self, id: StmtId) -> &StmtHir;
    pub fn field(&self, id: FieldId) -> &FieldHir;
    pub fn param(&self, id: ParamId) -> &ParamHir;
    pub fn local(&self, id: LocalId) -> &LocalHir;
    pub fn def_name(&self, id: DefId) -> &str;
    pub fn type_name(&self, ty: TyId) -> String;
    pub fn check_invariants(&self) -> Result<(), String>;
}
```

`stc_sema::Analysis` exposes `hir` plus the resolution map. The M4 driver only needs `hir`;
the resolution map remains useful for tests and future tooling.

### 3.3 Definition and type surface

```rust
pub enum DefKind {
    Namespace { internal: bool },
    Alias { target: TyId },
    Subrange { base: TyId, lower: i128, upper: i128, default: Option<i128> },
    Enum { base: TyId, values: Vec<EnumValueHir>, default: Option<Symbol> },
    NamedValue { base: TyId, values: Vec<EnumValueHir>, default: Option<Symbol> },
    Struct { fields: Vec<FieldId>, overlap: bool },
    Function { result: Option<TyId>, params: Vec<ParamId>, body: Option<BodyId> },
    FunctionBlock { base, interfaces, fields, methods, body, is_final, is_abstract, vtable },
    Program { fields: Vec<FieldId>, body: Option<BodyId>, var_access: Vec<VarAccessHir> },
    Class { base, interfaces, fields, methods, is_final, is_abstract, vtable },
    Interface { extends, methods, vtable },
    Method { owner, result, params, body, access, is_override, is_final, is_abstract, slot },
    Global { ty, section, retain, constant, at, init },
    Configuration { config: Box<ConfigurationHir> },
}

pub enum Ty {
    Error, Bool,
    Int(IntTy), Real(RealTy), BitString(u8),
    Time(TimeTy), Date(DateTy), Char(CharTy), String(StringTy),
    Array(ArrayTy), Ref(RefTy),
    Named(DefId), Null, Generic(GenericKind),
}
```

`Def` carries `name`, `span`, `parent`, `package`, `visibility`, `test_only`, `export`,
`no_mangle`, and `using`. Field/parameter/local records carry resolved types, sections,
retention, `AT` addresses, and initializer expression IDs.

### 3.4 Bodies and lowering

```rust
pub struct Body { pub owner: DefId, pub params: Vec<ParamId>, pub locals: Vec<LocalId>,
                  pub stmts: Vec<StmtId>, pub span: Span }

pub enum PlaceHir {
    Local(LocalId), Param(ParamId), Global(DefId), Result(DefId), This(DefId),
    Field { base: ExprId, field: FieldId },
    Index { base: ExprId, indices: Vec<ExprId> },
    Deref(ExprId),
    BitField { base: ExprId, bit: u16 },
    Direct(DirectAddress),
    InstanceField { program: DefId, instance: Symbol, fields: Vec<FieldId> },
}

pub enum CalleeHir {
    Function(DefId),
    StdFunction(StdFnId),
    Conversion { from: TyId, to: TyId },
    Method { receiver: ExprId, def: DefId },
    SuperMethod { receiver: ExprId, def: DefId },
    InterfaceMethod { receiver: ExprId, interface: DefId, method: DefId },
    Fb { instance: ExprId, def: DefId },
    SuperBody { instance: ExprId, base: DefId },
}

pub struct ArgHir {
    pub formal: Option<ParamId>,
    pub field: Option<FieldId>,
    pub direction: ArgDirection,
    pub value: Option<ExprId>,
    pub target: Option<ExprId>,
    pub negated: bool,
    pub span: Span,
}
```

Expression kinds are `Literal`, `Place`, `Binary`, `Unary`, `Call`, `Ref`, `Null`, `Cast`,
`ArrayInit`, `StructInit`, and `Error`. Statements are `Assign`, `RefAssign`, `Attempt`, `Call`,
`Return`, `If`, `Case`, `For`, `While`, `Repeat`, `Exit`, `Continue`, and `Empty`.

`CastKind` is `Implicit` or `Explicit`. The analyzer has already checked convertibility and
inserted every implicit conversion; codegen must emit these nodes and must never invent a cast.

### 3.5 Configurations

```rust
pub struct ConfigurationHir { pub name: Symbol, pub resources: Vec<ResourceHir>,
                              pub var_access: Vec<VarAccessHir>,
                              pub config_inits: Vec<ConfigInitHir>, pub span: Span }
pub struct ResourceHir { pub name: Symbol, pub on: Symbol, pub tasks: Vec<TaskHir>,
                         pub programs: Vec<ProgramInstanceHir>,
                         pub var_access: Vec<VarAccessHir> }
pub struct TaskHir { pub name: Symbol, pub single: Option<ExprId>,
                     pub interval: Option<ExprId>, pub priority: Option<ExprId>, pub span: Span }
pub struct ProgramInstanceHir { pub name: Symbol, pub program: DefId,
                                pub task: Option<Symbol>, pub bindings: Vec<BindingHir>,
                                pub span: Span }
pub struct BindingHir { pub target: ExprId, pub source: ExprId,
                        pub output: bool, pub span: Span }
pub struct ConfigInitHir { pub program: DefId, pub instance: Symbol,
                           pub fields: Vec<FieldId>, pub at: Option<DirectAddress>,
                           pub ty: TyId, pub init: Option<ExprId>, pub span: Span }
```

A configuration without an explicit `RESOURCE` gets a synthesized resource named `RESOURCE`
(M3). `HirCrate.entry_configuration` is set only when exactly one configuration exists.

### 3.6 Diagnostics and source information

- HIR owns `stc_span::Span` (`FileId`, byte start/end) for every def/body/expr/stmt. It has no
  file path or line/column.
- The driver owns `SourceMap` and gives `FileId`s in sorted path order (D-060).
- Semantic codes are in `stc_sema::codes`; parser/pragma codes keep their existing values.
- `stc check` reports human or JSON diagnostics and exits 0/1; usage errors exit 2; caught panics
  exit 3 (`STC-I-0001`).
- `HirCrate::check_invariants()` validates every arena/ID reference and is cheap enough for
  debug builds and the fuzzer.

### 3.7 Dump and test helpers

`stc_hir::dump_hir` produces a deterministic, depth-limited HIR dump. Snapshot harnesses use the
`STC_UPDATE_SNAPSHOTS=1` convention already used by the lexer and parser. Reuse that convention
for IR snapshots rather than introducing a new framework.

## 4. M3 HIR gaps M4 must close first

These are known omissions between the AST §5 target and the M3 implementation. Resolve them
before or alongside codegen; do not work around them in the driver with CST access.

### 4.1 `{stc:io}` hints are dropped

The parser validates and stores `PragmaAnnotation` for `{stc:io ...}`, but collection only reads
`test`, `export`, and `no_mangle` on POUs. Variable/field pragmas never reach HIR, so the
process-image metadata cannot include `device`, `channel`, `direction`, or the raw hint payload.

Add to `stc-hir` (names illustrative):

```rust
pub struct IoHint {
    pub device: Option<String>,
    pub channel: Option<u32>,
    pub direction: Option<IoDirection>, // Input | Output | InOut
    pub payload: String,                // canonical raw object for metadata
    pub span: Span,
}
```

Attach it to globals (`DefKind::Global` or `Def`) and to `FieldHir` for program/store variables.
Populate it in `collect_var_sections` from `VarDecl::pragmas()`, preserving the parser-validated
payload. Add a sema test that round-trips a hint into the dump. `IO-004` remains parser-owned.

### 4.2 Export names are not validated or stable

`Def.export`/`Def.no_mangle` are booleans; there is no stable C name list, no verification that a
`no_mangle` name is a C identifier, and no uniqueness check. M4 cannot emit safe wrapper symbols
without this.

Recommended fix (sema, where spans and diagnostics live):

- `{stc:export}` is valid on a `FUNCTION` (M4); on other defs it is either M5 or an explicit
  diagnostic.
- `{stc:no_mangle}` requires `export`; the source name must match the C identifier grammar and be
  unique across exports, globals, and reserved `stc_*` names.
- Record the resolved exported symbol name in HIR (an `Export` side table keyed by `DefId`, or a
  field on `Def`) so codegen never re-spells or re-validates it.

Assign a stable code (for example `STC-E-EXPORT-001`) and add it to the traceability table.

### 4.3 Task attributes are not folded

`TaskHir.single`, `interval`, and `priority` remain `ExprId`s. Sema checks that they are constant
and correctly typed, but does not fold them. Codegen cannot depend on sema to evaluate them.

Choose one:

- preferred: fold them in sema to `Literal`/resolved integers (and nanosecond durations) so HIR
  carries values; or
- add resolved fields (`single_ns: Option<i128>`, `interval_ns`, `priority: Option<i128>`) to
  `TaskHir`.

Do not parse or evaluate syntax in codegen. M4 needs task values at least for metadata and for
rejecting unsupported schedules deterministically.

### 4.4 Retained layout is absent

D-075 deferred `RetainLayout` to M4, but M4 does not have aggregate types yet. Resolve the
milestone mismatch explicitly:

- M4: compute a scalar-leaf `RetainLayout` and emit `stc_retain_size`,
  `stc_retain_layout_hash`, `stc_retain_save`, and `stc_retain_load` plus metadata. Payload
  offsets are cumulative sizes in sorted-path order; memory addresses use GEP at runtime, so no
  target datalayout is needed.
- M5: extend the same layout to arrays/structs/strings/FB state.
- M6: implement the file-backed runtime (`stc_rt_state_*`), atomic save, and restore policy.

Update D-075 or add a new decision that says exactly this. Do not claim "full RetainLayout" in
M4 if aggregates are not lowered.

### 4.5 Configuration selection and implicit programs

`entry_configuration` is `None` when there is no configuration or more than one. M4 has no
manifest to choose one. Define and record:

- exactly one configuration: use it;
- zero configurations: if exactly one `PROGRAM` exists, synthesize an implicit instance (for
  example `_default`) and run it; otherwise fail with a build error listing the programs;
- more than one configuration: fail with a build error listing the names (M6 will select via
  `[project] configuration`).

Do not silently pick the first definition.

### 4.6 Other HIR facts codegen will meet

- Program bodies use `PlaceHir::This(program)` internally for bare field references, even though
  the `THIS` keyword is rejected in program bodies by sema. Codegen must accept `This` for a
  program owner.
- `VAR_EXTERNAL` aliases are already lowered to `PlaceHir::Global(target)`; no special code is
  needed.
- Missing defaulted input arguments are simply absent from `ArgHir`. Scalar parameter defaults are
  constant `Literal` expressions (or `NULL`/reference initializers, which are M5); materialize
  them at the call site or at callee entry, consistently.
- Program-instance fields are reached with `PlaceHir::InstanceField` in configuration
  expressions; ordinary program body fields use `Field { base: This(..) }`.
- `PlaceHir::Direct` and `PlaceHir::BitField` are the only process-image access forms.
- Standard FB definitions are synthesized with empty spans and no bodies. Never emit them; a call
  to one is an M5 feature and must produce a call-site diagnostic, not an empty function.
- `stc_hir::Ty::Named` covers all nominal user types; M4 must reject it except for alias-free
  elementary uses that sema already canonicalized away.
- `stc_hir::ConstValue` includes strings, enum, date, and null variants. M4 supports numeric,
  boolean, char, time, and date constants; the rest are M5.

## 5. Recommended M4 architecture

### 5.1 New crates

```text
crates/stc-codegen/
  src/lib.rs          public generate() API, options, errors
  src/module.rs       textual LLVM module/function/block/value builder
  src/layout.rs       scalar type mapping, sizes, process image, retain layout
  src/mangle.rs       stable names and signature hashes
  src/expr.rs         expression lowering and scalar operators
  src/stmt.rs         statement/control-flow lowering
  src/unit.rs         functions, programs, globals, stc_init/cycle/shutdown/main
  src/config.rs       configuration instances, bindings, metadata
  src/metadata.rs     deterministic metadata JSON model
  src/abi.rs          runtime declarations and ABI constants
  src/codes.rs        codegen/build diagnostic codes (or driver-owned)
crates/stc-runtime/
  Cargo.toml          crate-type = ["staticlib"] (optionally cdylib later)
  src/lib.rs          C ABI, process image, traps, clock, state stubs
```

`stc-codegen` should not emit diagnostics directly. Return structured failures:

```rust
pub struct Options<'a> {
    pub artifact_name: &'a str,
    pub target: Option<&'a str>,
    pub debug: bool,
    pub emit_metadata: bool,
}

pub struct Output {
    pub module: String,
    pub metadata: Metadata,
    pub spans: Vec<SpanEntry>,
}

pub enum CodegenError {
    Unsupported { code: &'static str, message: String, span: Span },
    Internal { message: String },
}

pub fn generate(hir: &HirCrate, options: &Options<'_>) -> Result<Output, Vec<CodegenError>>;
```

The driver maps `Unsupported` to a source diagnostic and `Internal` to `STC-E-CODEGEN-002`/exit 3.
Codegen takes `&HirCrate` so the driver can reuse the same HIR for dumps/tests.

### 5.2 IR builder

- Emit **textual LLVM IR** with opaque pointers (`ptr`), no LLVM library linkage.
- Module header: compiler version, profile ID, and `source_filename = "<artifact>"`; no absolute
  paths, timestamps, or host names.
- Omit `target datalayout` always. Omit `target triple` unless `--target` is given.
- Functions and globals are emitted in deterministic order (sorted by mangled name), runtime
  declarations in sorted order, metadata JSON keys sorted.
- Use a monotonic temporary counter (`%t0`, `%t1`, …) and stable block labels (`bb0`, `bb1`, …).
- Every block has exactly one terminator. A statement after `RETURN`/`EXIT`/`CONTINUE` starts a
  fresh unreachable block so the IR remains valid.
- Represent types explicitly in the builder (i1/i8/i16/i32/i64/float/double/ptr); validate operand
  widths at construction time.
- Emit floating-point constants as hexadecimal (`0x...`) so formatting is exact and deterministic.
- Run a structural verifier in debug/tests; in tests also feed the module to `clang -x ir`.

### 5.3 Type mapping

| HIR type | LLVM representation |
| --- | --- |
| `BOOL` | `i8` in memory/global; `i1` in SSA; normalize with `icmp ne 0` |
| `SINT`/`USINT` | `i8` |
| `INT`/`UINT` | `i16` |
| `DINT`/`UDINT` | `i32` |
| `LINT`/`ULINT` | `i64` |
| `BYTE`/`WORD`/`DWORD`/`LWORD` | `i8`/`i16`/`i32`/`i64`, unsigned operations |
| `REAL`/`LREAL` | `float`/`double` |
| `TIME`/`LTIME` | `i64` nanoseconds |
| `DATE` | `i64` days since 1970-01-01 |
| `LDATE`/`TOD`/`LTOD`/`DT`/`LDT` | `i64` nanoseconds |
| `CHAR`/`WCHAR`/`UCHAR` | `i8`/`i16`/`i32` |
| enums/subranges/structs/arrays/strings/refs/classes/FBs/interfaces | M5; report unsupported |

Use declared signedness for `sdiv`/`udiv`, `srem`/`urem`, `icmp s*`/`u*`, and extensions. Never
hard-code pointer size, alignment, or struct offsets. Scalar sizes are known from the type; the
process image and retain payload use explicit byte operations with `align 1`.

### 5.4 Name mangling

Follow Architecture §4.10, and make every input canonical:

- `stc_fn_<ns>_<name>_<sig_hash>`, `stc_g_<ns>_<name>`, `stc_pi_<instance>`,
  `stc_prog_<type>`, `stc_export_<name>`.
- Namespace segments joined with `_n_`; omit the session root `_root`.
- Use normalized (case-insensitive) name text, not `Symbol` IDs or arena indices.
- Signature hash: stable 64-bit FNV-1a of a canonical signature string built from normalized
  parameter/result type names, not from `TyId`s. Add a `canonical_type_name(TyId)` helper or
  equivalent in codegen; do not use the display-oriented `HirCrate::type_name` for hashes.
- Escape characters outside the C identifier subset deterministically (for example `_uXXXX_`).
- `{stc:no_mangle}` uses the validated exact name; collision is an error.
- All emitted symbols are C-identifier-safe and unique. Reserved `stc_*`/`llvm.*` names cannot be
  claimed by users.

### 5.5 Units: functions, programs, globals

Functions:

- Scalar result returns directly (`i32`, `double`, …); no result returns `void`.
- Input parameters pass by value; M4 rejects `VAR_OUTPUT`/`VAR_IN_OUT` parameter directions.
- The function result uses `PlaceHir::Result(def)` (an alloca initialized to zero or the declared
  type default); `RETURN` branches to the return block and returns it.
- Locals are allocas initialized at entry from `LocalHir.init` (scalar literal) or type default.
- `VAR_EXTERNAL` already appears as a global place.
- Calls lower `CalleeHir::Function` directly. Missing defaulted inputs use the parameter's scalar
  default expression at the call site.
- `CalleeHir::Conversion` lowers through the scalar cast machinery.

Programs:

- A program type's fields become an LLVM struct type (M4 scalar fields only).
- Each configured instance is a global of that struct type, named by instance name.
- The program body is `void @stc_prog_<type>_body(ptr %this)`; field accesses are GEPs from
  `This(program)`.
- If there is no configuration and one program is selected implicitly, create the implicit
  instance global in `stc_init`/at module level.
- `stc_cycle` calls each instance body in deterministic configuration/source order. M4 does not
  implement interval scheduling; see §5.8.
- Program input/output bindings (`BindingHir`) are scalar assignments:
  - `output == false`: source evaluated into the target place before the body;
  - `output == true`: target place copied to the source after the body.
- `ConfigInitHir` overrides field initializers in `stc_init`, including scalar `AT` overrides if
  the layout is recomputed per instance; otherwise report them unsupported rather than ignoring.
- `VAR_ACCESS` is parsed/resolved in HIR but its metadata is M6; do not invent code for it.

Globals:

- Emit internal globals with deterministic mangled names and `zeroinitializer`.
- Scalar constant initializers may be embedded directly; all initializers are also applied by
  `stc_init` in deterministic source order (harmless for constants, required for non-constant
  forms). M4 rejects aggregate/reference initializers.
- `VAR_GLOBAL` constants (`CONSTANT`) still need storage if addressed; emit them as globals with
  their initializer.

### 5.6 Scalar operators and checks

- Integer `+`, `-`, `*`: use `llvm.sadd.with.overflow`/`ssub`/`smul` (or unsigned variants) and
  call `stc_rt_trap(STC_TRAP_OVERFLOW, span_id)` on overflow. Branch to a fresh trap block ending
  in `unreachable`.
- Integer `/`/`MOD`: check zero first (`STC_TRAP_DIV_ZERO`). Signed `MIN / -1` and `MIN MOD -1`
  are overflow; trap rather than invoking LLVM UB. `MOD` uses signed/unsigned remainder with the
  dividend's sign, matching M3 constant evaluation.
- `**`: float uses `llvm.pow.f32`/`llvm.pow.f64`. Integer exponentiation uses a checked
  multiplication loop; negative exponent traps `STC_TRAP_RANGE` (matching M3's constant error).
- Bitwise `AND`/`OR`/`XOR` on `BOOL` is logical i1; on bit strings it is unsigned integer
  bitwise of the same width. `NOT` is `xor i1, true` for bool and `xor -1` for bit strings.
- Comparisons: signed/unsigned according to type; floats use ordered predicates (`oeq`, `olt`,
  …); `<>` must be true for NaN (`une`), per IEC 60559. `BOOL` comparison uses i1.
- Unary `+` is a no-op; unary `-` on integers traps on `MIN` overflow (use checked subtraction
  from zero); on floats it is `fneg`; on TIME/DATE it is checked i64 negation.
- Float `/` by zero follows IEEE (no trap); no fast-math flags.
- Casts:
  - int→int, bitstring→bitstring: extend/truncate with explicit range checks when narrowing;
  - int↔real: `sitofp`/`uitofp`, `fptosi`/`fptoui` with range/NaN checks;
  - real→real: `fpext`/`fptrunc`;
  - bool↔integer/bitstring: zero-extend/comparison to zero, as defined by D-064;
  - time/date/char conversions: through i64/i32 plus range checks;
  - unsupported pairs: codegen error, not a wrong bitcast.
  Sema guarantees convertibility; codegen still must trap when the runtime value does not fit
  (`STC_TRAP_RANGE`/`STC_TRAP_CONVERSION`).
- Span IDs are assigned deterministically from HIR spans, deduplicated, sorted by
  `(FileId, start, end)`. Traps carry the ID.

### 5.7 Control flow

- `IF`/`ELSIF`/`ELSE`: one block per arm, `br`/`condbr`, merge block.
- `CASE`: comparison chain (or `switch` only for dense integer labels). Labels are `ConstValue`;
  ranges expand to two comparisons. Non-integer selectors are M5.
- `FOR`: store start into the control place, header compare against end using the constant step
  sign, body, checked increment, back edge, exit block. `step` is constant and nonzero (sema
  enforces); if it is a non-literal expression, evaluate it once to a temporary.
- `WHILE`: condition block, body, back edge.
- `REPEAT`: body first, then condition, back edge.
- `EXIT`/`CONTINUE`: codegen maintains a loop stack. `CONTINUE` targets the FOR increment block,
  the WHILE condition block, or the REPEAT condition block respectively.
- `RETURN`: branch to the function return block. Program `RETURN` branches to the body exit.
- Empty statements emit nothing.

### 5.8 Configuration selection and scheduling in M4

- Select one configuration (`entry_configuration`), or one implicit program (see §4.5).
- Flatten resources in HIR order and program instances in HIR order. M4 executes every program
  instance each cycle; `stc_cycle` is the outer loop body.
- Accept at most one task in M4. More than one task is `STC-E-RUN-001` at build time. Task
  `INTERVAL`/`PRIORITY` scheduling and real deadlines are M5; task attributes must still be
  reflected in metadata.
- `SINGLE` tasks may run only on the first cycle using a generated boolean guard, or be rejected
  if that complicates the cycle shape. Record the choice; do not claim scheduling support.
- Unbound programs (no `WITH` task) run every cycle in declaration order.
- Program instance names are unique across resources (sema enforces); use them in global names.
- Multiple configurations without a manifest are a build error (M6 selects).

### 5.9 I/O and process image

- Areas are contiguous: `%I`, then `%Q`, then `%M`. Offsets are absolute within each area; no
  packing of unrelated variables.
- Normalize `DirectAddress` deterministically:
  - wildcard (`%I*` etc.) is not executable; emit an explicit error (it may exist in declarations
    but cannot be laid out or accessed);
  - `size == Bit`: bit offset from the index pair (byte/index or bit index);
  - `size == None` with one index: byte; with two indices: byte.bit;
  - byte/word/dword/lword: index × size.
- Image sizes are `ceil(max bit end / 8)` per area. Metadata records `inputs_bytes`,
  `outputs_bytes`, `markers_bytes`.
- `AT` variables have no ordinary global storage. Accesses become unaligned (`align 1`) loads or
  stores into the image. `%I` is read-only (sema already diagnoses writes); `%Q`/`%M` read/write.
- Bit access (including `PlaceHir::BitField`) uses read-modify-write for writable areas and a
  masked load for `%I`.
- The runtime allocates the image, or the compiler emits the buffer and registers it. Pick one
  additive ABI and record it:
  - recommended: add `stc_rt_io_configure(input_len, output_len, marker_len)` before
    `stc_rt_io_image(&base, &len)`; generated `stc_init` calls both and caches the base pointer
    in a module global. Do not hard-code a maximum image size.
- `input_defaults_hex`/`output_defaults_hex` are computed from scalar type defaults and explicit
  initializers. For the M4 null backend, either the runtime applies defaults each cycle from
  metadata or generated `stc_init` writes them once; document the choice. M6 owns bound-device
  precedence.
- Metadata follows IO §5: image sizes, bindings (`variable`, `area`, `address`, `offset_bits`,
  `type`, `hint`, `source`), defaults. `source` is `pragma` in M4; `project` needs M6.
- `{stc:io}` hints come from the HIR extension in §4.1; unknown devices are warnings (M6 device
  tree), not build failures.
- `VAR_ACCESS` metadata and device trees are M6.

### 5.10 Runtime crate and ABI

`stc-runtime` is a Rust `staticlib` exposing the fixed C ABI from Runtime §2.3. M4 needs:

```c
void    stc_rt_trap(int32_t code, uint32_t span_id);
int32_t stc_rt_execute(int32_t argc, char** argv,
                       void (*init)(void), void (*cycle)(void), void (*shutdown)(void));
int64_t stc_rt_cycle_now_ns(void);
int64_t stc_rt_monotonic_now_ns(void);
void    stc_rt_set_cycle_time_ns(int64_t ns);
void    stc_rt_io_image(uint8_t** base, uint64_t* len);   /* plus the configure entry chosen in §5.9 */
int32_t stc_rt_state_load(void* buffer, int64_t len, int64_t layout_hash);
int32_t stc_rt_state_save(const void* buffer, int64_t len, int64_t layout_hash);
void    stc_rt_state_configure(const char* path, int32_t policy, int32_t enabled);
```

- `stc_rt_execute` parses at least `--cycles N` (tests always pass it), calls `init`, loops, calls
  `shutdown`, and returns 0. It owns the process-image buffers and the latched cycle clock.
  `--cycle-time`, `--io`, and `--trace` may be accepted or rejected explicitly; do not silently
  ignore them.
- `stc_rt_trap` prints a deterministic message (trap name and span ID) to stderr and exits with a
  documented nonzero status; no unwinding across the C boundary.
- `stc_rt_cycle_now_ns` returns the latched value; all code in one cycle sees the same value.
  `stc_rt_monotonic_now_ns` is host/runtime-only.
- `stc_rt_state_*` are stubs in M4 (return “disabled/not found”); the generated retained-state
  functions exist and are tested without file I/O. M6 implements the file format/atomicity.
- `stc_rt_assert` is not emitted in M4.
- The runtime crate must opt out of the workspace `unsafe_code = "forbid"` lint (edition 2024
  requires `#[unsafe(no_mangle)]`/unsafe extern blocks for FFI). Use the narrowest possible
  `allow(unsafe_code)` with a crate-level justification and keep all unsafe in this crate. Record
  this exception in `Decisions.md`.

**Do the runtime/link spike first.** Add a trivial staticlib exporting `stc_rt_execute`, emit a
hand-written `.ll` `main` that calls it, and link it with clang on Windows MSVC. Resolve the
Microsoft CRT/system-library flags before building the real codegen; Rust staticlibs often need
extra system libraries (`userenv`, `ws2_32`, `ntdll`, `bcrypt`, … depending on what the runtime
uses). Keep the runtime's `std` surface small, or use `no_std` + explicit OS FFI, but decide
early.

### 5.11 Module units and exports

Emit, as applicable:

```llvm
define internal void @stc_init()
define internal void @stc_cycle()
define internal void @stc_shutdown()
define i32 @main(i32 %argc, ptr %argv)
```

`main` calls `stc_rt_execute(argc, argv, @stc_init, @stc_cycle, @stc_shutdown)` and returns its
result. Library mode omits `main` and exports:

```llvm
define void @stc_init()
define void @stc_cycle()
define void @stc_shutdown()
define i32 @stc_abi_version()          ; 1
define ptr @stc_metadata_json()        ; embedded deterministic JSON string
```

`{stc:export}` functions get C wrappers (`stc_export_<name>` unless `no_mangle`); M4 supports
scalar functions. Wrappers must have ABI-compatible scalar signatures and call the internal
mangled function. `stc_metadata_json` may be deferred if library metadata is not tested, but it
is part of the documented library ABI; prefer implementing it now with the same metadata JSON
written to disk.

### 5.12 Retained-state ABI (scalar scope)

- Enumerate retained scalar leaves: retained globals and retained fields of program instances.
  Names/paths use instance/field names (`Main.Counter`, `Timer.Elapsed`), normalized for stable
  ordering but preserving a readable spelling.
- Sort by path, then emit payload offsets as cumulative sizes in sorted order. Sizes are the
  scalar widths from §5.3.
- `stc_retain_size()` returns the total; `stc_retain_layout_hash()` returns FNV-1a over the
  documented `leaf=path|type|offset|size` lines; `stc_retain_save(ptr)` copies each leaf's value
  into the buffer; `stc_retain_load(ptr)` restores.
- Use GEP/store/load with `align 1`; do not depend on target layout.
- Metadata includes `retain.size`, `layout_hash`, `leaves`, and the configured file/policy when
  available (file policy is M6; M4 can omit path/policy or use defaults).
- M5 extends leaves to aggregates/strings/FB state; M6 implements file-backed load/save.

### 5.13 Driver: `stc build` bridge

Keep `parse`/`check` unchanged. Add a deliberately minimal build bridge until M6:

```text
stc build <file>... [--name <name>] [--emit ir,obj,exe,lib,metadata]
           [--out-dir <dir>] [--target <triple>] [--clang <path>]
           [--runtime <path>] [--relaxed] [--message-format human|json]
```

- Lex/parse/analyze exactly as `stc check`; if any diagnostic is an error, do not codegen and
  exit 1.
- `--name` defaults to a sanitized stem of the first sorted input file. M6 replaces it with the
  manifest package name.
- Default emit in M4: `ir` (no clang/runtime needed). `exe`/`obj`/`lib` require clang; `exe`/`lib`
  also require the runtime staticlib. M6 loads defaults from `stc.toml`.
- Default out-dir: `target/stc/debug` (or `release` when added). Keep artifact filenames stable:
  `<name>.ll`, `<name>.o`, `<name>.exe`, `<name>.lib`/`<name>.a`,
  `<name>.stc-meta.json`.
- Runtime discovery: `--runtime`, then `STC_RUNTIME`, then a sibling of the `stc` executable or the
  workspace target directory. If missing, exit 2 with an actionable message.
- Clang discovery: `--clang`, `STC_CLANG`, then `clang` on `PATH`. Check `--version` and report a
  missing/unsupported toolchain as exit 2. Clang diagnostics/link failures are exit 1 and keep
  the `.ll` for inspection; `-v` prints the exact command.
- IR: write the module; no clang invocation.
- Object: `clang --target=<triple> -x ir <name>.ll -c -o <name>.o -O<level>`.
- Executable: link the object/IR with the runtime staticlib; Windows MSVC may need explicit system
  libraries discovered during the spike.
- Static library: compile the module and archive with `llvm-ar rcs` (or target-equivalent).
  Shared libraries (`lib-type = shared`) are M6; diagnose if requested.
- Metadata: write deterministic JSON whenever `metadata` is emitted and alongside `exe`/`lib`
  where the runtime needs it. Use sorted keys and normalized paths.
- Preserve exit codes 0/1/2/3. In JSON mode, diagnostics are JSON lines on stdout; clang output
  stays on stderr. Use the existing `STC-I-0001` panic boundary.
- Call `hir.check_invariants()` in debug builds before codegen; on failure report an internal
  compiler error, not a panic.

### 5.14 Determinism and reproducibility

- No hash-map iteration in emission paths; use `BTreeMap`/sorted vectors.
- Stable sort keys: mangled symbol, then source order where required. HIR arena order is
  deterministic for a fixed input (D-072).
- Never emit `Symbol` IDs, `TyId` numbers, absolute paths, timestamps, host names, or temporary
  directory names.
- Use a monotonic temp/block counter per function (or module) and reset it per emission.
- Hex float constants; integer constants with explicit LLVM types.
- Generate the same module twice in tests and require byte equality.
- Snapshots normalize only the documented compiler version line if needed; do not normalize away
  real IR differences.

## 6. M3 behaviors and gotchas that affect M4

1. **HIR is the only input.** Do not reach back into CST/AST. Codegen diagnostics get spans from
   HIR; if a span is `EMPTY_SPAN` (synthetic prelude defs), point at the use site.
2. **Spans are byte offsets, not lines.** The driver's `SourceMap` is the only line/column/path
   source. Build a deterministic span table from HIR and let the driver render it.
3. **Stable-looking IDs are not stable names.** `Symbol` and arena IDs depend on collection
   order. Mangle from normalized source text and canonical type strings.
4. **`HirCrate.entry_configuration` is optional and only unique-case.** Implement the fallback
   rules in §4.5.
5. **Program bodies use `This(program)` internally.** `THIS` syntax is rejected in programs, but
   bare fields lower through `PlaceHir::This`. Support this place in `This`/`Field` chains.
6. **Function results are places.** Initialize the result local to zero/type default; `RETURN`
   returns it. Missing assignment is already a sema error for functions.
7. **Defaulted arguments are omitted from `ArgHir`.** Check `ParamHir.default` at call sites.
   Scalar defaults are literals; aggregate/reference defaults are M5.
8. **`VAR_EXTERNAL` is already a global place.** Do not allocate local storage for it.
9. **`InstanceField` is configuration-only.** Do not expect it in ordinary program bodies; map it
   to the configured instance global and GEP its field path.
10. **Direct variables have no storage.** All accesses go through the process image; wildcard
    addresses cannot be emitted.
11. **Partial bit access is a read-modify-write.** `PlaceHir::BitField` applies to integer/bitstring
    places; preserve the other bits.
12. **Standard FBs are synthetic empty defs.** Their spans are empty and their bodies are `None`.
    A call is M5; report at the call span.
13. **Standard functions are signatures only.** `CalleeHir::StdFunction` is M5; conversions are
    `CalleeHir::Conversion` and are M4.
14. **M5 HIR is already produced.** Enums, subranges, structs, arrays, strings, references, OOP,
    etc. can appear in HIR. M4 must reject them explicitly (preflight or on demand), never emit
    placeholder IR.
15. **Only two callee kinds are M4.** `Function` and `Conversion`. All others are unsupported in
    M4.
16. **Casts are already explicit.** Emit both implicit and explicit casts; do not re-run the
    conversion matrix. Do trap on runtime range failures.
17. **Checked arithmetic is required.** Sema checked constant overflow only; runtime integer
    overflow/division by zero/range violations must trap with a span ID.
18. **Binary chains are deep.** M3 lowers a 20k-term left-associative chain iteratively but HIR
    still stores it as a left-deep arena chain. Codegen must traverse binary spines iteratively
    (or with an explicit stack); recursive expression lowering will overflow. Add a codegen
    deep-chain test.
19. **`CASE` labels are `ConstValue`s.** Overlaps/ranges are already validated. Emit comparison
    chains; do not assume dense integer labels.
20. **Task attributes are not folded.** Resolve them in HIR first (§4.3).
21. **Test-only units are absent from production HIR.** Build only production code; `stc test`
    and assert prelude remain M6.
22. **Parse errors gate sema and codegen.** `stc build` must not run codegen when the diagnostic
    sink has errors (D-070). Semantic errors also stop before codegen.
23. **`ExprKindHir::Error` should not reach codegen.** If it does, report an internal error; do not
    emit invalid IR.
24. **HIR invariants are checked in tests/fuzz.** Reuse `check_invariants` in codegen tests and the
    new codegen fuzzer; keep it cheap.
25. **`type_name` is for display.** It preserves original spelling for named types and may be
    recursive/depth-limited. Use a canonical formatter for mangling/retain hashes.
26. **Names are case-insensitive, original spelling preserved.** Metadata and mangled names need
    one canonical normalization; diagnostics can use original spelling.
27. **Configuration without `RESOURCE` has a synthetic resource.** Iterate resources normally;
    don't assume an `ON` value is non-empty.
28. **`ConfigInitHir.at` may override a field `AT`.** Either recompute layout/access per instance
    or reject explicitly; never ignore the override.
29. **`VAR_ACCESS` is resolved but not codegen.** Its metadata is M6. Do not confuse it with
    `{stc:io}` bindings.
30. **No manifest yet.** File paths are sorted lexicographically; `_root` is the package and root
    namespace. `stc build --name` is a bridge, not the final naming rule.
31. **The workspace forbids unsafe.** Only `stc-runtime` opts out, with a documented reason, for
    FFI/C ABI.
32. **Edition 2024 unsafe attributes.** Use `#[unsafe(no_mangle)]` and `unsafe extern` as required;
    the runtime's lint exception must permit them.
33. **Clang minimum is LLVM 15.** Emit opaque pointers (`ptr`) and avoid version-specific syntax.
    The local toolchain is LLVM 23.1.4; do not require it in emitted IR.
34. **Do not hard-code target layout.** No datalayout, pointer size, struct offsets, or alignment
    guesses. Use GEP and LLVM types; process-image/retain access uses explicit byte offsets with
    `align 1`.
35. **Windows MSVC linking needs a spike.** Rust staticlib + clang + MSVC CRT can require extra
    system libraries; prove the path before implementing codegen. Linux GNU follows later (M8),
    but keep the IR host-independent.
36. **Warnings are denied.** All new crates inherit `-D warnings`; keep codegen/runtime clippy
    clean. If the runtime needs crate-level allows, keep them narrow and documented.

## 7. Diagnostics for M4

Finalize the table in `Decisions.md` before implementation. Proposed additions:

| Code | Meaning |
| --- | --- |
| `STC-E-CODEGEN-001` | construct is valid HIR but not implemented in M4 (explicit M5 deferral) |
| `STC-E-CODEGEN-002` | internal codegen failure / IR invariant violation |
| `STC-E-BUILD-001` | clang missing, unsupported, or unusable |
| `STC-E-BUILD-002` | clang invocation or link failed |
| `STC-E-BUILD-003` | runtime staticlib missing or ABI mismatch |
| `STC-E-BUILD-004` | invalid build option/artifact combination (for example shared lib in M4) |
| `STC-E-EXPORT-001` | invalid/duplicate `{stc:export}`/`{stc:no_mangle}` symbol |
| `STC-E-RUN-001` | multiple tasks/unsupported scheduling (existing code; reuse) |
| `STC-E-RUN-002` | no runnable program/configuration or ambiguous configuration |

Preserve every existing diagnostic code. Codegen/build errors include primary spans where a HIR
span exists; toolchain errors use a command/context note instead of a fake span. Do not emit a
cascade of per-expression unsupported errors when one definition-level feature explains them.

Runtime traps use the `STC_TRAP_*` names from Runtime §2.3 and exit with a documented nonzero
status; they are not compiler diagnostic codes. The M4 runtime may print only the trap name and
span ID; M6 formats full source locations from metadata.

## 8. Tests and fuzzing

### 8.1 Codegen tests

Put unit/snapshot tests in `crates/stc-codegen/tests`:

- scalar type mapping and sizes;
- mangling and signature-hash stability (same source → same names; overloading distinct);
- casts and checked arithmetic structure;
- control-flow shape for each statement;
- program/global/config emission;
- process-image layout and metadata;
- retained layout and hash for scalar leaves;
- deterministic emission (generate twice, byte equality);
- unsupported M5 constructs produce `STC-E-CODEGEN-001` with spans;
- deep 20k-term binary chain completes without stack overflow;
- golden `.ll` snapshots using the existing `STC_UPDATE_SNAPSHOTS=1` convention.

### 8.2 clang verification

- In tests, if clang is available, write the module to a temp file and run
  `clang -x ir -c -o <temp> <file>`; fail on nonzero status and include clang stderr.
- CI must have clang; set `STC_REQUIRE_CLANG=1` in CI so a missing clang is a failure, while local
  test runs may skip with a clear message. Never silently pass a verification test that did not
  run.
- Add a `--target <triple>` object-emission smoke when cheap; execution remains host-only.

### 8.3 Execution tests

Keep these runnable by plain `cargo test --workspace` (no xtask yet; xtask/tests are M6). Suggested
location: `crates/stc-driver/tests/execution.rs` plus a checked-in C harness under
`crates/stc-driver/tests/harness/`.

Executable mode:

- build a program with `--emit=exe`, run it with `--cycles N`, check exit status/stdout;
- empty/no-op program runs finite cycles and exits 0;
- arithmetic, comparisons, `IF`/`CASE`/`FOR`/`WHILE`/`REPEAT`, function calls, globals;
- integer overflow, division by zero, and narrowing violations exit nonzero with the expected
  trap;
- program globals persist across cycles;
- direct `%I`/`%Q`/`%M` access through a process-image image (library harness can write/read the
  image between cycles).

Library mode:

- build `--emit=lib` (static), compile the C harness with clang, link the generated library and
  runtime staticlib, and execute `stc_init`/`stc_cycle`/`stc_shutdown`;
- call an exported scalar ST function through its wrapper and check the return value;
- test `stc_abi_version`/`stc_metadata_json` if implemented;
- ensure a no-op cycle links and returns 0.

Runtime artifact discovery:

- integration tests must build or locate `stc-runtime` deterministically. `cargo test` does not
  necessarily build a staticlib artifact for a staticlib-only crate. Add an explicit helper that
  runs `cargo build -p stc-runtime --locked` once per test process (or a small build script in the
  test crate) and passes the artifact via `--runtime`/`STC_RUNTIME`. CI should build it explicitly
  before tests if needed.

### 8.4 Fuzzing

Add `fuzz/fuzz_targets/fuzz_codegen.rs`:

1. tokenize/parse arbitrary bytes;
2. if there are no errors, run `stc_sema::analyze`;
3. if there are no semantic errors, call `stc_codegen::generate` with bounded options;
4. assert no panic and bounded output/time; optionally call `check_invariants` before codegen.

Keep `fuzz_lexer`, `fuzz_parser`, and `fuzz_analyzer`. Update `fuzz/Cargo.toml`/`fuzz/Cargo.lock`,
format the target explicitly, and run a 60 s CI smoke campaign. Do not require clang in the fuzzer.

### 8.5 Deep chains and limits

M2 accepts a 20k-term left-associative expression. Sema lowers it iteratively. Codegen must do the
same; add a test that emits IR for that expression and feeds it to clang (or at least the
structural verifier) without stack overflow. Keep codegen output bounded enough for tests.

## 9. Decisions/open items to resolve during M4

Record each in `Specification/Decisions.md` before or as it is implemented:

1. **Codegen crate/API.** `stc-codegen` consumes only HIR; `generate(&HirCrate, Options) ->
   Result<Output, _>`; no syntax/parser dependencies.
2. **Runtime crate and lint exception.** `stc-runtime` staticlib, C ABI, `unsafe_code` allow,
   edition-2024 unsafe attributes, panic/unwind policy.
3. **Runtime/link strategy.** Staticlib vs alternatives, MSVC system libraries, runtime
   discovery, `--runtime`/`STC_RUNTIME`, ABI version check.
4. **Build bridge.** `stc build <file>...`, `--name`, default emit `ir`, out-dir, artifact names;
   how M6 replaces each with manifest rules.
5. **M4 scalar subset.** Exactly which types/operators/calls are supported, and the explicit
   diagnostic for everything else.
6. **Diagnostic table.** `CODEGEN-*`, `BUILD-*`, `EXPORT-*`, `RUN-*` codes and messages.
7. **IR conventions.** Opaque pointers, no datalayout, optional triple, module header,
   `source_filename`, function/global ordering, temp/block naming, hex floats.
8. **Mangling.** Namespace joining, root omission, normalized/escaped identifiers, canonical
   signature strings, FNV-1a hashing, no_mangle validation/uniqueness.
9. **Configuration fallback/scheduling.** Unique configuration; implicit single program; multiple
   configurations error; one task; taskless programs; SINGLE handling; metadata task list.
10. **Process-image ABI.** Image allocation/configure entry point, image ownership, defaults
    application, unaligned/bit access, wildcard rejection, metadata shape.
11. **I/O hint HIR extension.** `IoHint` fields/payload preservation, attachment points, binding
    source/precedence without a manifest.
12. **Retain scope.** Scalar-leaf `RetainLayout`/ABI in M4; aggregate extension in M5; file-backed
    runtime in M6; update D-075 accordingly.
13. **Export ABI.** Supported export targets in M4, wrapper names, no_mangle rules, metadata/
    `stc_metadata_json`.
14. **Span/trap reporting.** Span table ordering, trap status/exit code, metadata span entries,
    debug info deferral.
15. **Type canonicalization for names.** Canonical type string for signature/retain hashes, not
    display names or arena IDs.
16. **Tests/CI.** Execution tests under `cargo test`, clang requirement/skip policy, runtime
    artifact helper, fuzz target, IR snapshot update convention, cross-target object smoke.
17. **Determinism.** Sorted iteration, counters, no paths/timestamps, reproducibility test.

## 10. Suggested order

1. Record M4 decisions and codes; write `M4-Plan.md`. Confirm M3 is committed (it is).
2. Spike `stc-runtime` staticlib + clang link on Windows MSVC: stub `stc_rt_execute`, hand-written
   IR `main`, link and run. Fix system-library/CRT issues now.
3. Close the blocking HIR gaps: `{stc:io}` hints, export names/validation, task values, implicit
   configuration/program selection. Add tests/snapshots.
4. Add `stc-codegen` skeletons: module builder, scalar types/layout, mangling, deterministic
   emission and snapshots.
5. Lower scalar functions, expressions, casts, checked arithmetic, and control flow; verify IR with
   clang.
6. Add globals, program instances/bodies, `stc_init`/`stc_cycle`/`stc_shutdown`, executable
   `main`, and library exports.
7. Implement `stc build` with `--emit=ir`; then clang `obj`/`exe`/static `lib`; execution and C
   harness tests.
8. Implement process-image layout/metadata/access and retain ABI; wire metadata.
9. Add unsupported-feature preflight, build/clang error handling, JSON diagnostics, artifact
   layout, and runtime discovery.
10. Add `fuzz_codegen`, deep-chain test, determinism test, cross-target object smoke if cheap.
11. Update README/spec status, run the full CI-equivalent checks, and commit M4.

## 11. Definition of done for M4

- [ ] HIR-to-IR codegen exists in `stc-codegen` and consumes only `stc-hir`.
- [ ] Elementary scalar types, operators, casts, checked arithmetic, control flow, functions,
      programs, globals, and initialization are emitted.
- [ ] Unsupported M5 features produce explicit diagnostics with spans; no silent wrong IR.
- [ ] `stc build` supports multi-file input and `--emit=ir,obj,exe` (static `lib` if scoped in),
      with deterministic artifact names and exit codes.
- [ ] `clang` accepts and links generated IR; executable and static-library modes run a no-op
      cycle and scalar functional tests.
- [ ] IR is deterministic, target-independent, free of target offsets/paths/timestamps, and
      snapshot-tested.
- [ ] Process-image layout, `AT` access, defaults, and metadata are emitted and tested.
- [ ] Scalar retained-state ABI/layout is emitted; file-backed state remains explicitly M6.
- [ ] `{stc:io}`/export/task/configuration HIR gaps are closed and tested.
- [ ] Deep 20k-term codegen does not overflow; `fuzz_codegen` is clean in smoke.
- [ ] `cargo test --workspace --locked`, clippy `-D warnings`, `cargo fmt --check`,
      `check_docs.py`, and Markdown lint pass.
- [ ] README, Decisions, project plan/conformance status, CLI/runtime/IO/state specs reflect M4.
- [ ] No manifest, `stc run`, `stc test`, OOP, aggregates, strings, or standard library silently
      claimed as implemented.

## 12. Commands

```sh
cargo test --workspace --locked
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all --check
python tools/ci/check_docs.py
npx --yes markdownlint-cli2@0.24.0

# Runtime staticlib and a direct build
cargo build -p stc-runtime --locked
cargo run -p stc-driver -- build examples/hello.st --emit=ir
cargo run -p stc-driver -- build examples/hello.st --emit=ir,obj,exe --runtime target/debug/stc_runtime.lib

# IR verification
clang -x ir -c -o /tmp/hello.o target/stc/debug/hello.ll
# Windows: clang -x ir -c -o %TEMP%\hello.obj target\stc\debug\hello.ll

# Fuzz workspace
cargo check --manifest-path fuzz/Cargo.toml --locked --offline
cargo +nightly fuzz run fuzz_codegen -- -max_total_time=60 -rss_limit_mb=2048 -timeout=25
rustfmt --edition 2024 --check fuzz/fuzz_targets/*.rs
```

Runtime artifact names differ by host: `stc_runtime.lib` on Windows MSVC, `libstc_runtime.a` on
Linux GNU. Use the driver's discovery helper in tests rather than hard-coding paths.

In PowerShell, set environment variables first, for example
`$env:RUSTFLAGS = "-D warnings"; cargo clippy --workspace --all-targets --locked -- -D warnings`.

## 13. Key files from M3

- `crates/stc-hir/src/{hir,ty,id,interner,dump}.rs` — resolved HIR, canonical types, stable IDs,
  interner, deterministic dump, `check_invariants`.
- `crates/stc-sema/src/{session,collect,types,resolve,consteval,check,prelude,codes}.rs` — analysis
  pipeline, diagnostics, configuration/body lowering.
- `crates/stc-sema/tests/{conformance,traceability,snapshots,hir_lowering}.rs` and
  `crates/stc-sema/tests/snapshots/*.snap` — semantic contract.
- `crates/stc-driver/src/main.rs` — `parse`/`check`, exit codes, human/JSON reporting, panic
  boundary.
- `crates/stc-driver/tests/cli.rs` — CLI contract.
- `fuzz/fuzz_targets/fuzz_analyzer.rs` — analyzer robustness pattern to copy for `fuzz_codegen`.
- `Specification/Decisions.md` D-059–D-075; `M3-Plan.md`; AST §5–§8; Architecture §4 and §6;
  Runtime §1–§4; IO §2/§5; State §2/§5–§6.
