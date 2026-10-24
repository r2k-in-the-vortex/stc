# Handover to M5 — FBs, OOP, Aggregates, Strings, and Standard Library

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-24  
Prepared after M4 (LLVM IR core codegen) completion

Read this together with the project plan (`Specification/00-Project-Plan.md` §8, M5–M7),
`Specification/Architecture/Architecture.md` §2–§6,
`Specification/Architecture/Runtime-and-Execution.md` §1–§4,
`Specification/Architecture/IO-and-Devices.md` §2–§5,
`Specification/Architecture/State-and-Retain.md` §2–§7,
`Specification/Architecture/Testing-and-Verification.md` §1–§4 and §6,
`Specification/AST/AST.md` §5–§8, `Specification/Decisions.md` D-059–D-108,
`Specification/Architecture/Online-Change-and-GOT.md`,
`Specification/Architecture/Debug-and-Monitoring.md`,
`M4-Plan.md`, `M4.5-Plan.md`, `update_plan_got.md` §13.0, and `handover_to_M4.md` (the M3→M4
input, retained for history).

## 0. Online-change addendum (2026-10-24)

**M4.5 is inserted before M5.** Do not implement M5 directly on M4's artifact model. The
resolved online-change architecture supersedes several M4-era recommendations in this document:

- Persistent globals/fields are **host-owned per-field roots** accessed through GOT cells, not
  module-owned globals or contiguous parent structs. This supersedes the struct layouts and
  direct globals in §3.5, §4.4, and §5.3.
- Every user/standard function call goes through a **host-owned function cell**. OOP type
  descriptors and vtables are runtime-owned; vtable entries are cells, not raw code addresses.
- Class/FB/interface/`REF_TO` values use **stable handles plus descriptors/place descriptors**;
  the fat pointer `{data, vtable}` and raw `this` pointer representations in §4.4/§5.1/§5.3 are
  superseded for persistent-capable values.
- Retained state and the state file are host-rooted; references and pointers are never persisted.
  Online attach does **not** load the state file; in-memory values win.
- `stc build` gains `--emit=module` and the paired GOT manifest. M5 must emit identity records
  for every new root/field, function/method/FB body, type/descriptor, instance, task, and export.
- M5 exit criteria include hot-swap coverage for FB/class state, aggregates/strings, interfaces,
  descriptors, and `POINTER TO`; see §2 and the project plan M5.

Read [Online Change and GOT](Specification/Architecture/Online-Change-and-GOT.md) and
`M4.5-Plan.md` before starting M5. If a recommendation below conflicts with those documents, the
online-change specification wins.

## 1. State at handover

M4 is implemented, verified, and committed as `4c55d71` (`M4: LLVM IR core codegen`). The
working tree is clean as of 2026-10-24. M3 is `4ddd5ea`; M2 is `24da2f7`; M1 is `51cab11`.

| Item | State |
| --- | --- |
| Workspace | `stc-span`, `stc-diag`, `stc-lexer`, `stc-syntax`, `stc-parser`, `stc-hir`, `stc-sema`, `stc-codegen`, `stc-runtime`, `stc-driver`; `fuzz/` is separate |
| Toolchain | Rust 1.99.1, edition 2024; workspace `unsafe_code = "forbid"`; `stc-runtime` is the documented exception |
| Tests | 327 passing (`cargo test --workspace --locked`), including codegen, clang verification, execution, and C-harness tests |
| Checks | clippy `-D warnings`, `cargo fmt --check`, `check_docs.py`, Markdown lint all pass |
| Fuzzing | `fuzz_lexer`, `fuzz_parser`, `fuzz_analyzer`, `fuzz_codegen`; codegen smoke ~27k runs in 11 s, no findings |
| Host tools | clang/LLVM 23.1.4, `llvm-ar`, host triple `x86_64-pc-windows-msvc` |
| Artifacts | `.ll`, `.o`, `.exe`, static `.lib`/`.a`, `.stc-meta.json` produced by `stc build` |
| Uncommitted | none |
| Manifest | still absent; `stc check`/`stc build` use the M3 `_root`/`PackageId(0)` bridge |

M4 delivered the first executable artifacts: deterministic textual LLVM IR for the elementary
scalar subset, a Rust runtime with the C ABI and cycle loop, clang invocation, executable and
static-library modes, process-image layout/metadata, scalar retained-state ABI, execution tests,
a C harness, IR snapshots, and a codegen fuzz target.

`stc check` and `stc build` are the current end of the pipeline. `stc parse` remains single-file
and syntax-only. Semantic analysis is skipped when lexing/parsing produced errors (D-070).
`stc-run`/`stc test`/`stc fmt`/the manifest remain later milestones.

## 2. M5 scope and exit criteria

From the project plan:

> Deliverables: FB instances/state, methods, inheritance, interfaces/vtables, `THIS`/`SUPER`,
> references, `VAR_IN_OUT`; static arrays, conformant `ARRAY [*]` parameters, structs, enums,
> subranges; `STRING`/`WSTRING`/`USTRING`; standard functions; standard FB stdlib; cycle-latched
> clock; single-task cyclic runtime; test runner MVP. All persistent aggregates follow the
> M4.5 per-field/handle model: runtime-owned descriptors, vtable cells, place descriptors for
> `REF_TO`, nullable interfaces, and the transient-only `STC-POINTER` extension.
>
> Exit criteria:
>
> - OOP conformance cases (inheritance, overriding, interface dispatch, assignment attempt) pass.
> - Static arrays and conformant `ARRAY [*]` parameters pass bounds/aliasing tests; dynamic
>   locals/globals/returns are rejected with explicit diagnostics.
> - UTF-8 vectors (valid/invalid sequences, character counts, UTF-8 byte lengths, 4-byte scalar
>   indexing, slicing, emoji/supplementary scalars) pass.
> - Standard timer/counter/edge/bistable FBs pass deterministic tests with a mock clock; all
>   timers in one scan see the same latched cycle time.
> - Online-change tests cover FB/class state, aggregate roots, interfaces, and preserved
>   descriptors; `POINTER TO` tests cover arithmetic, indexing, `SIZEOF`, null traps, and the
>   rejection of persistent pointer fields.

M5 is the milestone where the HIR produced since M3 is fully lowered. It should not change the
language surface or the semantic rules; M5 implements what the M3 analyzer already resolves.
M4's explicit deferrals are the M5 work list.

Recommended M5 boundary:

| Area | M5 | Deferred |
| --- | --- | --- |
| Types | enums, subranges, named-value types, structs, static arrays, conformant arrays, strings (`STRING`/`WSTRING`/`USTRING`), references, classes, FBs, interfaces | — |
| Expressions | standard functions, FB/method/interface calls, `SUPER`, `REF`/`NULL`/deref, `?=`, array/struct/string initializers, aggregate places | — |
| Statements | `RefAssign`, `Attempt`, all existing statements on aggregate places | — |
| Functions | output/in-out parameters, `VAR_IN_OUT`, conformant `ARRAY [*]` parameters, aggregate/string results as scoped by the chosen ABI | dynamic locals/globals/returns (`D-023` already rejects) |
| Programs/FBs | FB instance state, nested FB instances, methods, class instances, vtables/type descriptors, `THIS`/`SUPER` | multi-task scheduling still M6; M5 keeps one task per resource |
| Globals | aggregate/string/reference globals and constant initializers | — |
| I/O | no new image forms; aggregate `AT` remains rejected by sema | device tree, `VAR_ACCESS` metadata, non-null backends (M6) |
| ABI | vtables/type descriptors, attempt helper, string/math/date helpers, aggregate retain leaves | file-backed retained state (M6); `stc_rt_assert` (M6 test runner) |
| Stdlib | standard functions; `SR`, `RS`, `R_TRIG`, `F_TRIG`, `CTU`, `CTD`, `CTUD`, `TP`, `TON`, `TOF`; cycle-latched clock | full task scheduler/mock-clock CLI (M6) |
| Build | unchanged M4 bridge; `--emit` and artifact names stay stable | manifest, `stc run`, `stc test`, `stc fmt` (M6) |
| Test runner | recommend deferring `stc test` execution to M6 (see §9) | test discovery/runner/assert harness (M6) |

Anything outside the M5 column must fail with an explicit codegen diagnostic. Silent emission of
wrong IR remains worse than a deliberate “not implemented until M6” error.

## 3. Contracts M4 provides

### 3.1 Crates and dependency direction

```text
stc-span -> stc-hir -> stc-sema -> stc-parser/stc-syntax
stc-hir -> stc-codegen -> stc-driver
stc-span -> stc-codegen
stc-runtime (independent staticlib, linked by the driver)
```

Rules to preserve:

- `stc-hir` does not depend on `stc-syntax`, `stc-parser`, or `stc-sema`.
- `stc-codegen` depends only on `stc-hir`, `stc-span`, and std. Tests may use parser/sema as
  dev-dependencies; production code must not reach into syntax or semantic internals.
- `stc-runtime` is not a Rust dependency of any workspace crate. The driver discovers and links
  the artifact.
- `stc-driver` is both a binary (`stc`) and a small support library (`stc_driver`) exposing
  `toolchain::system_link_args` for integration tests.
- Only `stc-runtime` may use unsafe code, through a narrow crate-level allowance (D-077).

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

`stc_sema::Analysis` exposes `hir` plus the resolution map. M4 codegen uses only `hir`.

M4 added these HIR fields used by M5:

- `Def.export_name: Option<Symbol>` — validated final C symbol (D-086).
- `FieldHir.io: Option<IoHint>` and `DefKind::Global { io: Option<IoHint>, .. }` — validated
  `{stc:io}` hint with canonical payload text (D-085).
- `TaskHir.single_value`, `interval_ns`, `priority_value` — folded task attributes (D-088).

### 3.3 Definition and type surface

Every M5 construct already exists in HIR. The M5 implementer should read these as the contract:

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
    Global { ty, section, retain, constant, at, io, init },
    Configuration { config: Box<ConfigurationHir> },
}

pub enum Ty {
    Error, Bool, Int(IntTy), Real(RealTy), BitString(u8),
    Time(TimeTy), Date(DateTy), Char(CharTy), String(StringTy),
    Array(ArrayTy), Ref(RefTy),
    Named(DefId), Null, Generic(GenericKind),
}
```

Notes:

- `Ty::Named(DefId)` covers enums, subranges, named-value types, structs, classes, FBs,
  interfaces, functions, and programs. Aliases are resolved away.
- `vtable` is already deterministically ordered by sema. Do not recompute it differently.
- `Method.slot` is the vtable slot where applicable.
- `FieldHir` carries `section`, `visibility`, `retain`, `constant`, `at`, `io`, `init`, `span`.
- Structs have `overlap: bool`; `OVERLAP` layout rules are in Architecture §4.4.

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
    Function(DefId), StdFunction(StdFnId),
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
`ArrayInit`, `StructInit`, `Error`. Statements are `Assign`, `RefAssign`, `Attempt`, `Call`,
`Return`, `If`, `Case`, `For`, `While`, `Repeat`, `Exit`, `Continue`, `Empty`.

`CastKind` is `Implicit` or `Explicit`. Sema has already inserted every implicit conversion;
codegen must emit these nodes and must never invent a cast.

### 3.5 Configurations and instances

`ConfigurationHir`, `ResourceHir`, `TaskHir`, `ProgramInstanceHir`, `BindingHir`, and
`ConfigInitHir` are unchanged from M3 except the folded task fields. Important details:

- A configuration without `RESOURCE` has a synthesized resource named `RESOURCE`.
- `HirCrate.entry_configuration` is set only when exactly one configuration exists.
- Program-instance bindings use `This(program)` for program fields and ordinary places for
  external targets. M4 binds that pseudo-`this` to the current instance global while lowering
  `stc_cycle` (D-083).
- `ConfigInitHir` may carry an `AT` override. M4 rejects overrides explicitly; M5 or M6 may
  implement them.
- `PlaceHir::InstanceField` is used by `VAR_ACCESS` and other configuration paths; M4 supports
  only scalar/final-`AT` paths.

### 3.6 Codegen public API

```rust
pub enum Mode { Executable, Library }

pub struct Options<'a> {
    pub artifact_name: &'a str,
    pub target: Option<&'a str>,
    pub debug: bool,
    pub emit_metadata: bool,
    pub mode: Mode,
    pub sources: &'a [String],
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

M4's `generate` sequence:

1. `hir.check_invariants()`.
2. `config::select_plan` (configuration/implicit-program rules, task-count check).
3. `config::program_layouts` (scalar program structs, `VAR_CONFIG AT` rejection).
4. `config::image_layout` (declared `AT` bindings plus anonymous direct-expression spans,
   sizes, defaults).
5. `config::retain_layout` (scalar retained leaves, packed offsets, FNV-1a hash).
6. Build span table and symbol maps.
7. Preflight definitions; emit types/globals/declarations.
8. Emit functions, program bodies, `stc_init`/`stc_cycle`/`stc_shutdown`, retain functions,
   export wrappers, `main` or library metadata.
9. Build metadata; add embedded metadata symbol in library mode; structural check; render.

Driver mapping: `Unsupported` with a span becomes a source diagnostic; empty-span errors are
printed without a fake location; `Internal` prints `STC-E-CODEGEN-002` and exits 3.

### 3.7 Internal codegen structure

```text
crates/stc-codegen/src/
  lib.rs        API, Options/Mode, CodegenError, Output, SpanEntry
  module.rs     textual IR: IrType, Value, Function, Module, GlobalDef, c_string/quote
  layout.rs     ScalarKind, scalar_kind(_at), canonical_type_name, image_span,
                render_address, FNV-1a
  mangle.rs     normalize_ident, namespace_segments, function_symbol, signature_text,
                global_symbol, program_type_name, program_instance_symbol,
                program_body_symbol, task_guard_symbol
  config.rs     Plan/PlanTask/PlanInstance/ProgramLayout/IoBinding/ImageLayout/
                RetainPlace/RetainLeaf/RetainLayout; select_plan, program_layouts,
                image_layout, retain_layout
  expr.rs       BodyInfo, iterative expression stack machine, places, operators, casts,
                checked arithmetic, integer power helper, trap_if, storage conversions
  stmt.rs       statements, control flow, loop stack, CASE chains
  unit.rs       Lowerer, generate orchestration, symbols/preflight, functions/program bodies,
                init/cycle/shutdown/main/library metadata/exports/retain, metadata assembly
  metadata.rs   Json value/model and deterministic writers
  abi.rs        runtime symbol names, ABI version, trap codes
  codes.rs      M4 diagnostic/build code constants
```

`BodyInfo` contains parameter/local alloca maps, the result alloca and storage type, an optional
`this` pointer, and the return label. `Lowerer` owns the module, plan, layouts, image, retain,
symbol maps, span IDs, and errors.

### 3.8 Module builder surface

```text
IrType = Void | Int(u16) | Float | Double | Ptr | Overflow(u16)
Value  = { ty: IrType, repr: String }   // repr is a temp (%tN) or literal
Function::new(name, linkage, ret, params)
  start_block / start_fresh_block / next_block_label / ensure_open
  temp / emit_value / emit / terminate
  branch / cond_branch / ret / unreachable
  alloca / load / store / binop / unop / icmp / fcmp / select
  gep(pointee_text, base, indices, inbounds) / gep_bytes
  call(ret, callee, args) / extract_value / phi / param_value
  render / check
Module::new(artifact, target)
  add_type(name, body) / add_global(GlobalDef) / declare(name, signature)
  add_function(Function) / render / check
```

`Value.render()` includes the type for contexts that need it (`store`, `ret`, call arguments).
`binop`/`icmp`/`fcmp`/`phi` emit the type once, as LLVM requires. Names are stable and sorted.

### 3.9 Runtime ABI

`crates/stc-runtime` is a Rust `staticlib` exposing:

```c
int32_t stc_abi_version(void);                    /* runtime-owned, value 1 */
void    stc_rt_trap(int32_t code, uint32_t span_id);   /* exits 70 */
int32_t stc_rt_execute(int32_t argc, char** argv,
                       void (*init)(void), void (*cycle)(void), void (*shutdown)(void));
int64_t stc_rt_cycle_now_ns(void);
uint64_t stc_rt_monotonic_now_ns(void);
void    stc_rt_set_cycle_time_ns(int64_t ns);
int64_t stc_rt_local_date_ns(void);
void    stc_rt_io_configure(uint64_t input_len, uint64_t output_len, uint64_t marker_len);
void    stc_rt_io_image(uint8_t** base, uint64_t* len);
int32_t stc_rt_state_load(void* buffer, int64_t len, int64_t layout_hash);   /* 5 = disabled */
int32_t stc_rt_state_save(const void* buffer, int64_t len, int64_t layout_hash); /* 5 */
void    stc_rt_state_configure(const char* path, int32_t policy, int32_t enabled);
```

Generated code declarations in M4 use `i64` for the `uint64_t` lengths; this is ABI-compatible
on the supported hosts but should be normalized when the ABI is revised.

### 3.10 Build bridge and tests

`stc build <file>... [--name] [--emit=ir,obj,exe,lib,metadata] [--out-dir] [--target]
[--clang] [--runtime] [--relaxed] [--message-format] [-v]`.

- Default emit is `ir`; default out-dir is `target/stc/debug`; default name is the sanitized
  first sorted file stem.
- `exe` and `lib` cannot be emitted together in M4.
- Windows MSVC link discovery lives in `crates/stc-driver/src/toolchain.rs`; Linux GNU adds
  `-ldl -lpthread -lm`; `llvm-ar` is used with an `ar` fallback on non-Windows.
- Execution tests live in `crates/stc-driver/tests/execution.rs`; C harnesses in
  `crates/stc-driver/tests/harness/`. The runtime is built once per test process.
- Codegen tests live in `crates/stc-codegen/tests/`; the snapshot harness uses
  `STC_UPDATE_SNAPSHOTS=1`; clang verification is mandatory when `STC_REQUIRE_CLANG=1`.

## 4. M4 behavior and gaps M5 must close

### 4.1 Preflight rejects every M5 definition

`unit.rs::preflight` reports `STC-E-CODEGEN-001` for `Struct`, `Enum`, `NamedValue`, `Subrange`,
`Class`, `FunctionBlock`, and `Interface`, and `layout::scalar_kind` rejects `Ty::Named`,
`Ty::String`, `Ty::Array`, and `Ty::Ref`. Replace this with real support and narrow the
unsupported set to genuinely out-of-profile constructs. Keep the “one definition-level error,
not a per-expression cascade” rule (D-093).

### 4.2 The IR type system is scalar-only

`IrType` has no named, array, struct, string, reference, or interface representation, and
`Module.type_defs` stores raw strings. M5 needs a typed representation that still renders
deterministically. Recommended direction: add a module-owned type table with a small `TypeRef`
ID, keep `Value` cheap, and render named types as `%stc.*` names. Do not embed target offsets.

The builder also lacks indirect calls (needed for vtables), aggregate loads/stores, `memcpy`,
`extractvalue` with multiple indices, `insertvalue`, and array GEPs. Add them with type checks.

### 4.3 Layout is scalar-only

`layout::ScalarKind`, `ProgramLayout`, `ImageLayout`, and `RetainLayout` are scalar. M5 needs:

- `TypeLayout` for enums/subranges (base integer), structs (named LLVM struct), arrays
  (`[N x T]`), strings (`{ i32 len, [N x T] }`), references (`ptr`), interfaces (`{ptr, ptr}`),
  and class/FB instances (named struct with the base prefix).
- Field-path resolution through inheritance, nested structs, and FB instances.
- Aggregate sizes/defaults for initialization and retain, without assuming a target datalayout.
- `OVERLAP` struct handling (byte buffer plus typed accessors, Architecture §4.4).

### 4.4 Programs are scalar structs; FBs/classes have no lowering

`ProgramLayout` keeps only non-`AT` scalar fields and a field-index map. `stc_prog_<type>_body`
is generated for programs only. M5 needs:

- A named struct per class/FB, including inherited base state in a prefix layout.
- FB bodies `void @stc_prog_<type>_body(ptr %this)` (or the FB naming in Architecture §4.10),
  with input fields written before the body and output fields read after it.
- Method functions with an explicit `this` first parameter.
- `stc_init` initialization of FB/class fields and nested instances.
- Class/FB method vtables and type descriptors, with deterministic slot order from HIR.
- Interface values as fat pointers and indirect dispatch through the vtable slot.
- `THIS`/`SUPER` places and calls, including `SUPER()` body invocation.

### 4.5 Calls and places deferred to M5

M4 lowers only `CalleeHir::Function` and `CalleeHir::Conversion`; all other callee kinds are
rejected. M4 also rejects `PlaceHir::Index`, `Deref`, `Direct` as ordinary storage, and
`InstanceField` paths beyond a final `AT` leaf. M5 must implement:

- `StdFunction` (all categories in Runtime §3.1).
- `Method`, `SuperMethod`, `InterfaceMethod`, `Fb`, `SuperBody`.
- `Ref`, `Null`, `Deref`, `RefAssign`, `Attempt`, `?=`.
- `ArrayInit`, `StructInit`, string literals/copies, aggregate assignment and comparison.
- Output arguments (`=>`, `ArgHir.target`/`negated`) and `VAR_IN_OUT` parameters.
- Arrays with declared bounds, 1-based indexing, bounds traps, and conformant descriptors.

M4 has a working trap path (`trap_if`, span IDs) and checked arithmetic; reuse it for bounds,
string capacity, subrange assignment, and conversion failures.

### 4.6 Retain is scalar-only

`RetainLayout` serializes scalar retained globals and program fields as a packed sequence of
leaves sorted by path. M5 must extend it to:

- structs, arrays, strings, enums/subranges (as base integers), and FB/class instance state;
- nested paths and inheritance (`Main.Timer.Elapsed`);
- exclusion of references, interface pointers, and vtable/type-descriptor pointers;
- recursive leaf flattening whose payload offsets are packed sizes, not target struct offsets.

Do **not** compute LLVM struct offsets in the compiler. Save/load should use GEP chains per leaf
and write packed payload bytes; metadata offsets remain payload offsets (D-087). Non-persistable
leaves must keep `STC-E-RETAIN-001` behavior. File-backed state remains M6: keep the
`stc_retain_*` ABI and the disabled `stc_rt_state_*` stubs.

### 4.7 Exports are function-only

Sema accepts `{stc:export}` only on `FUNCTION` in M4. Decide whether M5 exports methods or
class/FB types (likely no: the C ABI is scalar-function-oriented). If export support expands,
keep the same validation, exact C names, and uniqueness rules. `stc_abi_version` remains
runtime-owned; do not reintroduce a duplicate generated symbol.

### 4.8 Standard functions and standard FBs are signature-only

`HirCrate.std_fns` contains signatures (`StdFnKind`, `StdParam`, `StdResult`). M4 rejects every
`StdFunction` call. M5 must implement all `StdFnKind` variants, including shifts/rotates,
selection, comparisons, math, date/time, and string functions. The standard FB definitions are
synthetic empty-span HIR defs with fields but no bodies; codegen currently skips all empty-span
defs. M5 must provide real FB bodies, not emit empty functions.

Recommended delivery: add `stdlib/*.st` sources for `SR`, `RS`, `R_TRIG`, `F_TRIG`, `CTU`,
`CTD`, `CTUD`, `TP`, `TON`, `TOF` and inject/compile them as production HIR in the driver or
sema. If textual stdlib injection is deferred, synthesize HIR bodies and record the deviation.
Timers must read `stc_rt_cycle_now_ns()`, never the live clock.

### 4.9 Clock and tasks

The runtime already latches the clock per cycle in `stc_rt_execute` and supports library
overrides through `stc_rt_set_cycle_time_ns`. Generated code does not yet read the clock because
there are no time functions or timers. M5 adds those reads. M4 still accepts at most one task
per resource (`STC-E-RUN-001`) and runs all instances each cycle; a `SINGLE` task uses a
first-cycle guard. Full task scheduling, priorities, and a mock-clock CLI remain M6.

### 4.10 Test-only units and the test runner

M4 codegen skips `Def.test_only` defs and treats `{stc:test}` functions as production-absent
(D-071). The M5 plan lists a “test runner MVP”, but test discovery depends on the M6 manifest.
Recommend deferring `stc test` to M6 and continuing to use `cargo test` plus C harnesses in M5.
If a file-list MVP is required, scope it explicitly in an M5 decision and do not pull manifest
rules forward.

### 4.11 Other M4 facts codegen will meet

- `Ty::Named` includes programs; a `PROGRAM` field access in a program body uses
  `PlaceHir::This(program)` internally even though the `THIS` keyword is rejected there.
- `VAR_EXTERNAL` aliases are lowered to `PlaceHir::Global(target)`; do not allocate storage.
- Missing defaulted input arguments are absent from `ArgHir`; M4 materializes scalar defaults
  at the call site. M5 aggregate/reference defaults need a consistent policy.
- Standard FB defs have empty spans and no bodies; never emit them.
- `ExprKindHir::Error` must not reach codegen; report an internal error.
- Binary chains are deep and lowered with an explicit work stack; keep this property for
  aggregate/string expressions so 20k-term chains do not overflow.
- `HirCrate::check_invariants()` is cheap and runs before generation; keep it in tests/fuzzing.
- `type_name` is display-oriented; use `layout::canonical_type_name`/a new canonicalizer for
  mangling, hashes, and metadata.
- Source names are case-insensitive; the interner preserves the first spelling. Normalize for
  symbols, retain paths, and ordering.
- `%QW0.1` lexes as a `WORD` direct address; use `%QX0.1` for a direct bit. Bit access on an
  `AT` word (`word.0`) is supported through `PlaceHir::BitField`.
- `type_name_to_ty` recognizes elementary names plus date/time and char aliases only; user-type
  conversion functions are not resolved. Extend it carefully if M5 needs `MyEnum_TO_...`.
- M4 rejects `VAR_CONFIG AT` overrides; do not silently ignore them.
- `VAR_ACCESS` is resolved in HIR but its metadata is M6; do not invent code for it.

## 5. Recommended M5 architecture

### 5.1 Type and layout layer

Add a `TypeLayout`/`StorageType` abstraction in `layout.rs` (or a new `types.rs`):

```rust
pub enum StorageType {
    Scalar(ScalarKind),
    Named(u32),              // index into a module type table
    Array { element: Box<StorageType>, count: u64 },
    String { kind: StringKind, capacity: u32 },
    Ref,                     // opaque ptr
    Interface,               // { ptr, ptr }
}
```

Rules:

- Enums and subranges lower to their base integer; subrange assignment checks traps.
- Structs/classes/FBs get named LLVM structs. Inheritance uses the prefix layout: base state
  is the first member, and field access walks the prefix chain.
- Arrays are nested LLVM arrays; index computation uses declared lower bounds and traps
  `STC_TRAP_BOUNDS`.
- Strings are `{ i32 len, [N x T] }` with `T = i8/i16/i32` for `STRING`/`WSTRING`/`USTRING`;
  `USTRING` data is `4 * N` bytes.
- Conformant arrays use the descriptor from Architecture §4.3 (`{ptr data, i64 len,
  [rank x {i64 lower, i64 upper}]}`) and never allocate storage.
- `OVERLAP` structs use a byte buffer and typed accessors; the analyzer already validates
  overlap conflicts.
- `REF_TO T` is `ptr`; `NULL` is `null`.
- Interfaces are `{ptr data, ptr vtable}`; `NULL` is `{null, null}`.

No target datalayout, pointer size, alignment, or struct offset may be hard-coded. Use LLVM
types, GEP, and explicit length fields. Retain access is the one exception: it uses packed
payload offsets and `align 1`.

### 5.2 Module/IR builder extensions

- Add named/parameterized type references (prefer a module type table over stringly typed
  `IrType` so operand checking remains possible).
- Add `call_indirect`, aggregate `load`/`store`, `memcpy`, `extractvalue` with multiple
  indices, `insertvalue`, and typed array/struct GEPs.
- Add `switch` only if it improves deterministic CASE lowering; comparison chains remain valid.
- Keep `Function::check` invariants (one terminator per block, unique labels).
- Keep emission sorted by mangled name and metadata keys sorted.

### 5.3 OOP and FB lowering

- Reuse the sema-assigned `vtable` order. Emit one vtable global per concrete class/FB.
- Type descriptor format (decision required): at minimum `{ptr vtable, i64 type_id}` plus
  interface information sufficient for `?=`. Keep it deterministic and metadata-free if
  runtime `?=` is the only consumer.
- Direct calls for concrete receivers; interface calls load the vtable slot and use
  `call_indirect`.
- `SUPER.method()` calls the base implementation directly.
- `SUPER()` invokes the base FB body on the current instance.
- FB invocation:
  1. evaluate the instance place to a `ptr`;
  2. store input arguments into fields (including nested/defaulted inputs);
  3. call the FB body `void @..._body(ptr %this)`;
  4. copy output fields to `ArgHir.target` places, applying `negated` for `NOT =>`.
- Method signatures: `this` first, scalar/aggregate parameters follow the chosen ABI.
- `stc_init` initializes instance state and declared initializers, then loads retained state.
  Static vtables/descriptors should be constant globals where possible; avoid runtime patching.
- `THIS` is the current `%this`; `SUPER` uses the base pointer (no adjustment under prefix
  layout).

### 5.4 Aggregates and strings

- Aggregate assignment copies the whole value (load/store or `memcpy` for large values).
- `ArrayInit`/`StructInit` zero-initialize then store each element/field; omitted struct fields
  take their declared/type defaults.
- Array indexing: compute `(index - lower)` per dimension, check against bounds, GEP.
- String values copy the struct; length is normalized to destination capacity. Capacity
  overflow traps `STC_TRAP_RANGE`.
- String literals lower to constant/initialized storage in the current function's entry block;
  prefer static constants when possible.
- Implement `LEN`, `LEN_MAX`, `LEN_CODE_UNIT`, concatenation, selection, insert/delete/replace/
  find, and UTF-8 validation in runtime helpers (`stc_rt_str_*`) so bounds/encoding logic is
  shared and testable.
- Invalid UTF-8 must trap `STC_TRAP_INVALID_UTF8`; `USTRING` stores Unicode scalars.
- String comparison follows the profile (unit comparison; no normalization).

### 5.5 Standard functions

Follow Runtime §3.1 ownership:

- Inline IR: `ABS`, `MIN`, `MAX`, `LIMIT`, `SEL`, `MUX`, comparisons, integer arithmetic,
  bitwise/shift/rotate (`llvm.fshl`/`llvm.fshr`), `MOVE`.
- Runtime: string functions, date/time, float math (`stc_rt_math_*` or libm), checked
  conversions.
- `llvm.sqrt`/`llvm.fabs` where semantics match.
- Keep the existing checked arithmetic/trap machinery. `ABS(MIN)` traps overflow.
- Standard functions must never call the live monotonic clock; `NOW()`/`TIME()` use
  `stc_rt_cycle_now_ns`.
- Add Lean-generated vector tests for exact functions, or keep them as test vectors under the
  existing `tests/vectors` convention if present.

### 5.6 Standard FB library and timers

- Compile the standard FBs from `stdlib/*.st` into production HIR, or synthesize equivalent
  HIR bodies. They live in the reserved `STC_STD` namespace and are shadowable.
- Implement `SR`, `RS`, `R_TRIG`, `F_TRIG`, `CTU`, `CTD`, `CTUD`, `TP`, `TON`, `TOF`.
- Timers capture start times in instance fields and compare against the latched
  `stc_rt_cycle_now_ns()`. All timers in one scan see the same value.
- Provide deterministic tests using `stc_rt_set_cycle_time_ns` in library harnesses or
  `--cycle-time` once the runtime CLI supports it (M6).
- Do not implement multi-task scheduling in M5; keep the M4 one-task rule and metadata task
  attributes.

### 5.7 Retained state

- Flatten aggregate/string/FB instance state into scalar/string leaves with readable paths.
- Payload offsets are cumulative packed sizes in sorted-path order (D-087); no LLVM struct
  offsets.
- Exclude pointers, interface/vtable/descriptor pointers, references, and `VAR_IN_OUT`.
- Extend metadata `retain.leaves` with aggregate-derived paths and canonical type names.
- Keep `stc_retain_size/layout_hash/save/load`; save/load use GEP chains + `align 1` stores.
- Update D-087/D-075 to record the M5 aggregate scope; file I/O remains M6.

### 5.8 Metadata and ABI

- Extend `metadata.rs` with new metadata if needed (types/vtables are not required by the
  current specs; prefer not to invent keys).
- If runtime symbols are added, bump the runtime ABI minor version per Runtime §2.2 and record
  it in Decisions. `stc_abi_version` stays runtime-owned.
- Keep `stc_metadata_json` library-only; executable metadata remains the sidecar JSON.
- Span/trap tables stay sorted by `(FileId, start, end)`; trap messages still carry only the
  span ID in M5.

### 5.9 Driver

The M4 build bridge is sufficient. M5 should not add the manifest. Possible additions:

- include `stdlib/*.st` in the analyzed file set (deterministic order; reserved package);
- optionally add `--stdlib <dir>` for experiments, but keep the default self-contained;
- keep `--emit` names/exit codes stable.

If stdlib injection needs a source list, prefer a driver-level convention over a manifest.

### 5.10 Test runner decision

Recommend recording a decision: **M5 does not implement `stc test`**. Test functions continue to
be excluded from production HIR (D-071); M5 verification uses the existing execution/C-harness
suites plus new OOP/aggregate/stdlib harnesses. M6 implements test discovery, the runner,
`STC_ASSERT`, the mock-clock CLI, and process isolation.

## 6. Diagnostics for M5

Prefer existing sema codes and traps over new compiler codes. Proposed policy:

| Code | Meaning |
| --- | --- |
| `STC-E-CODEGEN-001` | out-of-profile construct or an M5 feature deliberately deferred to M6 |
| `STC-E-CODEGEN-002` | internal codegen failure / IR invariant violation |
| `STC-E-RUN-001` | multiple tasks/unsupported scheduling (still M6 for real scheduling) |
| `STC-E-RUN-002` | no runnable program/configuration or ambiguous configuration (unchanged) |
| `STC-E-RETAIN-001/002`, `STC-W-RETAIN-001` | existing retain rules |
| `STC-TRAP_BOUNDS`, `STC-TRAP_RANGE`, `STC-TRAP_NULL_REF`, `STC-TRAP_INVALID_UTF8`, `STC-TRAP_CONVERSION` | runtime traps via `stc_rt_trap` |

Do not add a code per aggregate type or per standard function. One definition/call-site error
explains the feature. Keep every existing diagnostic code.

If a new code is unavoidable (for example an aggregate ABI restriction), assign the next stable
`STC-E-CODEGEN-00N` and record it in Decisions and the traceability table.

## 7. Tests and fuzzing

### 7.1 Codegen tests

Extend `crates/stc-codegen/tests/`:

- type/layout mapping and sizes for enums, subranges, structs, arrays, strings, references,
  interfaces, classes, FBs;
- struct/array/string initializers and copies;
- OOP: inheritance, overrides, interface dispatch, `SUPER`, `THIS`, `?=`;
- FB invocation: inputs, outputs, defaults, nested instances;
- standard functions (all categories) and standard FBs with mock clock;
- retain aggregate layout/hash and save/load structure;
- determinism twice-byte equality; unsupported M6 constructs still explicit;
- deep chains through aggregate/string expressions;
- golden `.ll` snapshots and clang verification (`STC_REQUIRE_CLANG=1`).

Add negative cases for dynamic arrays/locals/returns (`D-023`), non-persistable retain leaves,
invalid conformant placement, and cross-kind/invalid string conversions.

### 7.2 Execution tests

Extend `crates/stc-driver/tests/execution.rs` and `tests/harness/`:

- OOP conformance programs linked in library mode and driven from C harnesses;
- interface dispatch and `?=` success/failure;
- FB state across cycles, timers with explicit `stc_rt_set_cycle_time_ns` steps, edge/counter
  behavior;
- arrays/structs/strings, bounds traps, string capacity/UTF-8 traps;
- conformant `ARRAY [*]` aliasing and bounds;
- retained-state round-trip for aggregate/string/FB leaves;
- no-op cycles in both modes remain green.

### 7.3 Fuzzing

Extend `fuzz_codegen` to exercise M5 HIR. Keep it clang-free. Assert bounded output/time,
determinism, HIR invariants, and no panic. Add M5 constructs to the corpus/dictionary where
useful. Keep the 60 s CI smoke.

### 7.4 CI

- `STC_REQUIRE_CLANG=1` remains mandatory in the test job.
- Runtime build and C-harness linking stay in the test helper.
- Keep the cross-target object smoke (`tests/cross-target/smoke.ll`) and add aggregate/string
  IR to it if useful.
- Keep Windows MSVC and Ubuntu 24.04 test jobs.

## 8. Suggested order

1. Record M5 decisions D-094+ and write `M5-Plan.md`; confirm M4 is committed (it is).
2. Extend the IR builder type system and layout layer with aggregates/strings/references.
3. Lower structs, enums, subranges, arrays, and strings; add bounds/string traps and tests.
4. Lower references, `NULL`, deref, `RefAssign`, `Attempt`/`?=`, and output/`VAR_IN_OUT`
   parameters.
5. Lower classes/FBs/methods, inheritance, vtables/type descriptors, interface fat pointers,
   `THIS`/`SUPER`, and FB invocation.
6. Implement standard functions; add the standard FB stdlib and cycle-latched timers.
7. Extend retained state to aggregates/strings/FB state; update metadata and D-075/D-087.
8. Add OOP/aggregate/stdlib execution harnesses, clang verification, snapshots, fuzz coverage.
9. Update decisions, README/status, conformance/profile docs, and the M5 plan; run all checks.
10. Commit M5.

## 9. Decisions/open items to resolve during M5

Record each in `Specification/Decisions.md` before or as it is implemented:

1. **Aggregate ABI.** Which aggregates pass by value, by pointer, or result via hidden pointer;
   how arrays/strings/FB instances are passed; whether aggregate function results are allowed.
2. **Module type system.** Type table/ID design; how named types render; how operand types are
   checked without `Copy`-hostile recursive enums.
3. **Aggregate layout.** Prefix inheritance, `OVERLAP`, arrays, strings, conformant descriptors;
   no target datalayout.
4. **Bounds and assignment checks.** Subrange assignment, array bounds, string capacity,
   conversion ranges; which trap codes.
5. **References and assignment attempt.** `REF`/`NULL`/deref semantics, fat-pointer conversion,
   `?=` helper/descriptor format, null traps.
6. **OOP representation.** Vtable/descriptor shape, slot order source (HIR `vtable`), direct vs
   indirect call rules, interface upcast/downcast.
7. **FB invocation.** Input/output binding order, defaults, negation, nested instances,
   reentrancy/aliasing.
8. **Standard functions.** Inline vs runtime split, math library calls, Lean vector coverage,
   date/time conversion rules.
9. **Standard FB delivery.** `stdlib/*.st` injection vs synthesized HIR; reserved namespace;
   shadowing and diagnostics.
10. **Clock and timers.** Library mock-clock usage, duration arithmetic, timer state fields,
    `STC_CLOCK_ADVANCE` scope.
11. **Retained scope.** Aggregate/string/FB leaf flattening, packed offsets, non-persistable
    exclusions, hash-format compatibility.
12. **Runtime ABI revision.** New `stc_rt_str_*`/`stc_rt_math_*`/attempt helpers; minor version
    bump; whether `i64` vs `uint64_t` length declarations are normalized.
13. **Export scope.** Whether methods/classes can be exported; wrapper naming; metadata.
14. **Test runner.** Confirm deferral to M6 or define a file-list MVP with `{stc:test}`.
15. **Metadata additions.** Only add keys required by a spec or tested consumer; keep sorted.
16. **Determinism.** Type/vtable ordering, string constants, aggregate metadata, reproducible
    snapshots.
17. **Tests/CI.** Harness structure for OOP/stdlib, mock-clock helpers, fuzz coverage.

## 10. Definition of done for M5

- [ ] Enums, subranges, named-value types, structs, static and conformant arrays, strings,
      references, classes, FBs, and interfaces are lowered to valid IR.
- [ ] OOP conformance cases pass: inheritance, overriding, interface dispatch, `SUPER`, `THIS`,
      `?=`.
- [ ] FB instances preserve state across cycles; inputs/outputs bind correctly; nested instances
      work.
- [ ] Standard functions are implemented and tested; standard FB stdlib (SR/RS/edge/counter/
      timer) runs deterministically with a latched mock clock.
- [ ] Array bounds, subrange assignment, string capacity, conversion, and UTF-8 failures trap
      with span IDs.
- [ ] Conformant `ARRAY [*]` parameters use the documented descriptor and bounds checks;
      dynamic locals/globals/returns remain explicit diagnostics.
- [ ] Aggregate/string/FB retained leaves are laid out, hashed, saved, and loaded through
      `stc_retain_*`; file-backed state remains explicitly M6.
- [ ] `stc build` still emits deterministic IR/objects/executables/static libraries/metadata;
      `clang` accepts and runs them on the host.
- [ ] Unsupported M6 constructs (manifest, `stc run/test`, device tree, file state, full task
      scheduling) produce explicit diagnostics or remain absent as documented.
- [ ] IR/metadata are deterministic, target-independent, path/timestamp-free, and snapshotted.
- [ ] `cargo test --workspace --locked`, clippy `-D warnings`, `cargo fmt --check`,
      `check_docs.py`, Markdown lint, and the fuzz smoke pass.
- [ ] README, Decisions, project plan/conformance status, and the architecture/runtime/IO/state/
      testing specs reflect M5.
- [ ] No FBs, OOP, aggregates, strings, standard functions, or standard FB library are silently
      claimed as unimplemented when they are present, or implemented when they are not.

## 11. Commands

```sh
cargo test --workspace --locked
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all --check
python tools/ci/check_docs.py
npx --yes markdownlint-cli2@0.24.0

# Runtime staticlib and direct builds
cargo build -p stc-runtime --locked
cargo run -p stc-driver -- build examples/hello.st --emit=ir
cargo run -p stc-driver -- build examples/hello.st --emit=ir,obj,exe \
    --runtime target/debug/stc_runtime.lib
cargo run -p stc-driver -- build examples/hello.st --emit=lib,metadata \
    --runtime target/debug/stc_runtime.lib

# IR verification
clang -x ir -c -o /tmp/hello.o target/stc/debug/hello.ll

# Fuzz workspace
cargo check --manifest-path fuzz/Cargo.toml --locked --offline
cargo +nightly fuzz run fuzz_codegen -- -max_total_time=60 -rss_limit_mb=2048 -timeout=25
rustfmt --edition 2024 --check fuzz/fuzz_targets/*.rs
```

Runtime artifact names differ by host: `stc_runtime.lib` on Windows MSVC,
`libstc_runtime.a` on Linux GNU. Use the test/driver discovery helpers rather than hard-coding
paths.

In PowerShell, set environment variables first, for example
`$env:RUSTFLAGS = "-D warnings"; cargo clippy --workspace --all-targets --locked -- -D warnings`.
Set `$env:STC_REQUIRE_CLANG = "1"` to make clang verification mandatory locally.

## 12. Key files from M4

- `crates/stc-codegen/src/{lib,module,layout,mangle,config,expr,stmt,unit,metadata,abi,codes}.rs`
- `crates/stc-codegen/tests/{codegen.rs,common/mod.rs,snapshots/library.ll}`
- `crates/stc-runtime/src/lib.rs` — C ABI, image, clock, traps, state stubs
- `crates/stc-driver/src/{main,build,toolchain,lib}.rs` — CLI build bridge and link discovery
- `crates/stc-driver/tests/{execution.rs,harness/*.c}` — host/library execution tests
- `crates/stc-hir/src/{hir,ty,id,interner,dump}.rs` — resolved HIR and invariants
- `crates/stc-sema/src/{session,collect,types,resolve,consteval,check,prelude,codes}.rs`
- `crates/stc-sema/tests/{conformance,traceability,snapshots,hir_lowering,m4_gaps}.rs`
- `Specification/Decisions.md` D-059–D-093; `M4-Plan.md`; `handover_to_M4.md`
- Architecture §4 (especially §4.3–§4.10), Runtime §2–§4, IO §2/§5, State §2/§5–§6,
  Testing §1–§4/§6, AST §5–§8
